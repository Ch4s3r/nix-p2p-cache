use crate::keys::LocalKey;
use serde::{Deserialize, Serialize};

pub const STORE_DIR: &str = "/nix/store";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NarInfo {
    pub store_path: String,
    pub url: String,
    pub compression: String,
    pub file_hash: String,
    pub file_size: u64,
    pub nar_hash: String,
    pub nar_size: u64,
    pub references: Vec<String>,
    pub deriver: Option<String>,
    pub ca: Option<String>,
    pub sig: Option<String>,
}

impl NarInfo {
    pub fn fingerprint(&self) -> String {
        let refs_full: Vec<String> = self
            .references
            .iter()
            .map(|r| {
                if r.starts_with('/') {
                    r.clone()
                } else {
                    format!("{STORE_DIR}/{r}")
                }
            })
            .collect();
        format!(
            "1;{};{};{};{}",
            self.store_path,
            self.nar_hash,
            self.nar_size,
            refs_full.join(",")
        )
    }

    pub fn sign_with(&mut self, key: &LocalKey) {
        let fp = self.fingerprint();
        self.sig = Some(key.sign(fp.as_bytes()));
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("StorePath: {}\n", self.store_path));
        out.push_str(&format!("URL: {}\n", self.url));
        out.push_str(&format!("Compression: {}\n", self.compression));
        out.push_str(&format!("FileHash: {}\n", self.file_hash));
        out.push_str(&format!("FileSize: {}\n", self.file_size));
        out.push_str(&format!("NarHash: {}\n", self.nar_hash));
        out.push_str(&format!("NarSize: {}\n", self.nar_size));
        let refs_basenames: Vec<&str> = self
            .references
            .iter()
            .map(|r| r.rsplit('/').next().unwrap_or(r))
            .collect();
        out.push_str(&format!("References: {}\n", refs_basenames.join(" ")));
        if let Some(d) = &self.deriver {
            let base = d.rsplit('/').next().unwrap_or(d);
            out.push_str(&format!("Deriver: {base}\n"));
        }
        if let Some(c) = &self.ca {
            out.push_str(&format!("CA: {c}\n"));
        }
        if let Some(s) = &self.sig {
            out.push_str(&format!("Sig: {s}\n"));
        }
        out
    }
}

pub fn sha256_to_nix_hash(digest: &[u8; 32]) -> String {
    format!("sha256:{}", nix_base32::to_nix_base32(digest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::LocalKey;

    fn sample() -> NarInfo {
        NarInfo {
            store_path: "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-foo".into(),
            url: "nar/abcd.nar".into(),
            compression: "none".into(),
            file_hash: "sha256:00000000000000000000000000000000000000000000000000".into(),
            file_size: 42,
            nar_hash: "sha256:00000000000000000000000000000000000000000000000000".into(),
            nar_size: 42,
            references: vec!["/nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-bar".into()],
            deriver: None,
            ca: None,
            sig: None,
        }
    }

    #[test]
    fn narinfo_signing_roundtrip_matches_nix_format() {
        let key = LocalKey::shared();
        let mut info = sample();
        info.sign_with(&key);
        let fp = info.fingerprint();
        assert!(fp.starts_with("1;/nix/store/"));
        let sig = info.sig.as_deref().unwrap();
        let (name, b64) = sig.split_once(':').unwrap();
        assert_eq!(name, crate::keys::KEY_NAME);
        let sig_bytes = data_encoding::BASE64.decode(b64.as_bytes()).unwrap();
        let sig = ed25519_dalek::Signature::from_slice(&sig_bytes).unwrap();
        ed25519_dalek::Verifier::verify(&key.signing.verifying_key(), fp.as_bytes(), &sig).unwrap();
    }

    #[test]
    fn render_emits_basenames_for_references() {
        let info = sample();
        let rendered = info.render();
        assert!(rendered.contains("References: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-bar\n"));
        assert!(rendered.contains("StorePath: /nix/store/"));
    }
}
