use anyhow::{Context, Result};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const KEY_DOMAIN: &str = "nix-p2p-cache.shared.v1";
pub const KEY_NAME: &str = "nix-p2p-cache-shared";

fn shared_seed() -> [u8; 32] {
    blake3::hash(KEY_DOMAIN.as_bytes()).into()
}

pub fn shared_signing_key() -> SigningKey {
    SigningKey::from_bytes(&shared_seed())
}

pub fn shared_verifying_key() -> VerifyingKey {
    shared_signing_key().verifying_key()
}

pub fn public_key_line() -> String {
    let b64 = data_encoding::BASE64.encode(shared_verifying_key().as_bytes());
    format!("{KEY_NAME}:{b64}")
}

pub struct LocalKey {
    pub signing: SigningKey,
}

static GLOBAL: OnceLock<()> = OnceLock::new();

impl Default for LocalKey {
    fn default() -> Self {
        let _ = GLOBAL.set(());
        Self {
            signing: shared_signing_key(),
        }
    }
}

impl LocalKey {
    pub fn shared() -> Self {
        Self::default()
    }

    pub fn name(&self) -> &'static str {
        KEY_NAME
    }

    pub fn public_key_line(&self) -> String {
        public_key_line()
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

pub fn write_key_files(dir: &Path) -> Result<(PathBuf, PathBuf)> {
    fs::create_dir_all(dir).with_context(|| format!("create key dir {}", dir.display()))?;
    let key = LocalKey::shared();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_pubkey_line_is_stable_and_well_formed() {
        let a = public_key_line();
        let b = public_key_line();
        assert_eq!(a, b);
        assert!(a.starts_with("nix-p2p-cache-shared:"));
        let (_, b64) = a.split_once(':').unwrap();
        let bytes = data_encoding::BASE64.decode(b64.as_bytes()).unwrap();
        assert_eq!(bytes.len(), 32);
    }

    #[test]
    fn signs_and_verifies_with_shared_key() {
        let key = LocalKey::shared();
        let sig_line = key.sign(b"hello");
        let (name, b64) = sig_line.split_once(':').unwrap();
        assert_eq!(name, KEY_NAME);
        let sig_bytes = data_encoding::BASE64.decode(b64.as_bytes()).unwrap();
        let sig = ed25519_dalek::Signature::from_slice(&sig_bytes).unwrap();
        ed25519_dalek::Verifier::verify(&shared_verifying_key(), b"hello", &sig).unwrap();
    }
}
