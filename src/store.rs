use crate::narinfo::{NarInfo, sha256_to_nix_hash};
use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use std::path::PathBuf;

pub const DEFAULT_DB_PATH: &str = "/nix/var/nix/db/db.sqlite";

#[derive(Debug, Clone)]
pub struct StorePathRow {
    pub path: String,
    pub nar_hash_db: String,
    pub nar_size: u64,
    pub deriver: Option<String>,
    pub references: Vec<String>,
    pub ca: Option<String>,
}

pub struct LocalStore {
    db_path: PathBuf,
    store_dir: PathBuf,
}

impl LocalStore {
    pub fn new(db_path: impl Into<PathBuf>, store_dir: impl Into<PathBuf>) -> Self {
        Self {
            db_path: db_path.into(),
            store_dir: store_dir.into(),
        }
    }

    fn open_ro(&self) -> Result<Connection> {
        let conn = Connection::open_with_flags(
            &self.db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .with_context(|| format!("open nix db read-only at {}", self.db_path.display()))?;
        conn.busy_timeout(std::time::Duration::from_secs(2))?;
        Ok(conn)
    }

    pub fn lookup_by_hash(&self, hash_part: &str) -> Result<Option<StorePathRow>> {
        if !is_valid_hash_part(hash_part) {
            return Ok(None);
        }
        if !self.db_path.exists() {
            return Ok(None);
        }
        let conn = self.open_ro()?;
        let prefix = format!("{}/{}-", self.store_dir.display(), hash_part);
        let pattern = format!("{}%", escape_like(&prefix));
        let mut stmt = conn.prepare(
            "SELECT id, path, hash, narSize, deriver, ca FROM ValidPaths WHERE path LIKE ?1 ESCAPE '\\' LIMIT 1",
        )?;
        let row: Option<(i64, String, Option<String>, Option<i64>, Option<String>, Option<String>)> = stmt
            .query_row([&pattern], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                ))
            })
            .ok();
        let Some((id, path, hash_opt, nar_size_opt, deriver, ca)) = row else {
            return Ok(None);
        };
        let (Some(hash), Some(nar_size)) = (hash_opt, nar_size_opt) else {
            return Ok(None);
        };
        if nar_size < 0 {
            return Ok(None);
        }
        let mut refs_stmt = conn.prepare(
            "SELECT v.path FROM Refs r JOIN ValidPaths v ON v.id = r.reference WHERE r.referrer = ?1 ORDER BY v.path",
        )?;
        let references: Vec<String> = refs_stmt
            .query_map([id], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(StorePathRow {
            path,
            nar_hash_db: hash,
            nar_size: nar_size as u64,
            deriver,
            references,
            ca: ca.filter(|s| !s.is_empty()),
        }))
    }

    pub fn narinfo_for(&self, hash_part: &str, nar_url: &str) -> Result<Option<NarInfo>> {
        let Some(row) = self.lookup_by_hash(hash_part)? else {
            return Ok(None);
        };
        let nar_hash = match normalize_nar_hash(&row.nar_hash_db) {
            Ok(v) => v,
            Err(_) => return Ok(None),
        };
        Ok(Some(NarInfo {
            store_path: row.path,
            url: nar_url.to_string(),
            compression: "none".into(),
            file_hash: nar_hash.clone(),
            file_size: row.nar_size,
            nar_hash,
            nar_size: row.nar_size,
            references: row.references,
            deriver: row.deriver,
            ca: row.ca,
            sig: None,
        }))
    }

    pub fn open_nar_stream(&self, store_path: &str) -> Result<nix_nar::Encoder> {
        let owned = store_path.to_string();
        nix_nar::Encoder::new(owned)
            .map_err(|e| anyhow::anyhow!("nix-nar encoder: {e}"))
    }
}

pub fn normalize_nar_hash_pub(raw: &str) -> Result<String> {
    normalize_nar_hash(raw)
}

pub fn is_valid_hash_part(hash_part: &str) -> bool {
    hash_part.len() == 32 && hash_part.chars().all(|c| c.is_ascii_alphanumeric())
}

fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' | '%' | '_' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

fn normalize_nar_hash(raw: &str) -> Result<String> {
    let (algo, value) = raw
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("malformed narHash {raw}"))?;
    if algo != "sha256" {
        anyhow::bail!("unsupported hash algorithm {algo}");
    }
    if value.len() == 52 {
        return Ok(format!("sha256:{value}"));
    }
    if value.len() == 64 {
        let bytes = hex::decode(value).context("decode hex narHash")?;
        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("hex narHash wrong length"))?;
        return Ok(sha256_to_nix_hash(&arr));
    }
    anyhow::bail!("unexpected narHash length {}", value.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nar_encoder_handles_mixed_entries_without_panic() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("payload");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("regular.txt"), b"hello").unwrap();
        let exe_path = root.join("script.sh");
        std::fs::write(&exe_path, b"#!/bin/sh\necho hi\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&exe_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::create_dir(root.join("empty_dir")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("regular.txt", root.join("link")).unwrap();
        let mut encoder = nix_nar::Encoder::new(&root).unwrap();
        let mut buf = Vec::new();
        std::io::copy(&mut encoder, &mut buf).unwrap();
        assert!(buf.len() > 100);
        let s = String::from_utf8_lossy(&buf);
        assert!(s.contains("nix-archive-1"));
        assert!(s.contains("regular.txt"));
        assert!(s.contains("empty_dir"));
        #[cfg(unix)]
        {
            assert!(s.contains("script.sh"));
            assert!(s.contains("link"));
        }
    }

    #[test]
    fn rejects_invalid_hash_part() {
        assert!(!is_valid_hash_part(""));
        assert!(!is_valid_hash_part("short"));
        assert!(!is_valid_hash_part(&"x".repeat(33)));
        assert!(!is_valid_hash_part(&"a/".repeat(16)));
        assert!(is_valid_hash_part(&"a".repeat(32)));
    }

    #[test]
    fn escape_like_quotes_metachars() {
        assert_eq!(escape_like("ab%c_d\\e"), "ab\\%c\\_d\\\\e");
    }

    #[test]
    fn unsupported_hash_algo_returns_none_not_500() {
        let raw = format!("md5:{}", "a".repeat(32));
        assert!(normalize_nar_hash(&raw).is_err());
    }

    #[test]
    fn normalize_hex_to_nix_base32() {
        let hex = "0".repeat(64);
        let raw = format!("sha256:{hex}");
        let n = normalize_nar_hash(&raw).unwrap();
        assert!(n.starts_with("sha256:"));
        assert_eq!(n.len(), "sha256:".len() + 52);
    }

    #[test]
    fn normalize_passthrough_base32() {
        let raw = format!("sha256:{}", "a".repeat(52));
        let n = normalize_nar_hash(&raw).unwrap();
        assert_eq!(n, raw);
    }
}
