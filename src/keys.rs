use anyhow::{Context, Result};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use std::fs;
use std::path::{Path, PathBuf};

const KEY_DOMAIN: &str = "nix-p2p-cache.v1";

pub fn key_name_for_host(hostname: &str) -> String {
    format!("nix-p2p-cache-{hostname}")
}

pub fn validate_hostname(hostname: &str) -> Result<()> {
    if hostname.is_empty() {
        anyhow::bail!("hostname is empty");
    }
    if hostname.len() > 253 {
        anyhow::bail!("hostname too long");
    }
    for c in hostname.chars() {
        let ok = c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_';
        if !ok {
            anyhow::bail!("hostname contains invalid character {c:?}; allowed: a-z A-Z 0-9 . - _");
        }
    }
    Ok(())
}

pub fn signing_key_from_hostname(hostname: &str) -> SigningKey {
    let mut hasher = blake3::Hasher::new();
    hasher.update(KEY_DOMAIN.as_bytes());
    hasher.update(b"|");
    hasher.update(hostname.as_bytes());
    let seed: [u8; 32] = hasher.finalize().into();
    SigningKey::from_bytes(&seed)
}

pub fn public_key_line(hostname: &str) -> String {
    let signing = signing_key_from_hostname(hostname);
    let verifying: VerifyingKey = signing.verifying_key();
    let b64 = data_encoding::BASE64.encode(verifying.as_bytes());
    format!("{}:{}", key_name_for_host(hostname), b64)
}

pub struct LocalKey {
    pub hostname: String,
    pub signing: SigningKey,
}

impl LocalKey {
    pub fn from_hostname(hostname: impl Into<String>) -> Self {
        let hostname = hostname.into();
        let signing = signing_key_from_hostname(&hostname);
        Self { hostname, signing }
    }

    pub fn name(&self) -> String {
        key_name_for_host(&self.hostname)
    }

    pub fn public_key_line(&self) -> String {
        public_key_line(&self.hostname)
    }

    pub fn secret_key_line(&self) -> String {
        let bytes = self.signing.to_keypair_bytes();
        let b64 = data_encoding::BASE64.encode(&bytes);
        format!("{}:{}", self.name(), b64)
    }

    pub fn sign(&self, data: &[u8]) -> String {
        let sig = self.signing.sign(data);
        let b64 = data_encoding::BASE64.encode(&sig.to_bytes());
        format!("{}:{}", self.name(), b64)
    }
}

pub fn write_key_files(dir: &Path, hostname: &str) -> Result<(PathBuf, PathBuf)> {
    fs::create_dir_all(dir).with_context(|| format!("create key dir {}", dir.display()))?;
    let key = LocalKey::from_hostname(hostname);
    let secret_path = dir.join("key");
    let public_path = dir.join("key.pub");
    fs::write(&secret_path, key.secret_key_line() + "\n")?;
    fs::write(&public_path, key.public_key_line() + "\n")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&secret_path, fs::Permissions::from_mode(0o600))?;
    }
    Ok((secret_path, public_path))
}

pub fn detect_hostname() -> Result<String> {
    if let Ok(h) = std::env::var("NIX_P2P_CACHE_HOSTNAME") {
        return Ok(h);
    }
    let out = std::process::Command::new("hostname").output()?;
    let raw = String::from_utf8(out.stdout)?;
    let trimmed = raw.trim().to_string();
    if trimmed.is_empty() {
        anyhow::bail!("hostname returned empty string")
    }
    Ok(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_hostname_rejects_empty_and_special_chars() {
        assert!(validate_hostname("").is_err());
        assert!(validate_hostname("foo:bar").is_err());
        assert!(validate_hostname("foo|bar").is_err());
        assert!(validate_hostname("foo bar").is_err());
        assert!(validate_hostname(&"x".repeat(254)).is_err());
        assert!(validate_hostname("ok-host_1.example").is_ok());
    }

    #[test]
    fn deterministic_pubkey_line_for_same_hostname() {
        let a = public_key_line("alpha");
        let b = public_key_line("alpha");
        assert_eq!(a, b);
        assert!(a.starts_with("nix-p2p-cache-alpha:"));
    }

    #[test]
    fn different_hostnames_produce_different_keys() {
        assert_ne!(public_key_line("alpha"), public_key_line("beta"));
    }

    #[test]
    fn signs_and_verifies_with_derived_key() {
        let key = LocalKey::from_hostname("host");
        let sig_line = key.sign(b"hello");
        let (name, b64) = sig_line.split_once(':').unwrap();
        assert_eq!(name, "nix-p2p-cache-host");
        let sig_bytes = data_encoding::BASE64.decode(b64.as_bytes()).unwrap();
        let sig = ed25519_dalek::Signature::from_slice(&sig_bytes).unwrap();
        ed25519_dalek::Verifier::verify(&key.signing.verifying_key(), b"hello", &sig).unwrap();
    }
}
