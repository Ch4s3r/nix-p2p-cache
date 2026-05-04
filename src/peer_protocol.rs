use serde::{Deserialize, Serialize};

pub const PROTOCOL_NAME: &str = "/nix-p2p-cache/1.0.0";
pub const MAX_MESSAGE_BYTES: usize = 512 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PeerRequest {
    Has { hash_part: String },
    GetNar { hash_part: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathMeta {
    pub store_path: String,
    pub nar_hash: String,
    pub nar_size: u64,
    pub references: Vec<String>,
    pub deriver: Option<String>,
    #[serde(default)]
    pub ca: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PeerResponse {
    Has(Option<PathMeta>),
    Nar(Option<Vec<u8>>),
    Error(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_protocol_encodes_has_request_roundtrip() {
        let req = PeerRequest::Has {
            hash_part: "a".repeat(32),
        };
        let bytes = serde_json::to_vec(&req).unwrap();
        let back: PeerRequest = serde_json::from_slice(&bytes).unwrap();
        match back {
            PeerRequest::Has { hash_part } => assert_eq!(hash_part.len(), 32),
            _ => panic!("wrong variant"),
        }
    }
}
