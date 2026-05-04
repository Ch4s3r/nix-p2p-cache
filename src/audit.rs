use libp2p::{Multiaddr, PeerId, multiaddr::Protocol};
use serde::Serialize;
use std::net::IpAddr;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize)]
pub struct AuditEvent<'a> {
    pub ts: u64,
    pub event: &'a str,
    pub peer_id: String,
    pub peer_host: Option<String>,
    pub peer_addrs: Vec<String>,
    pub hash_part: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_path: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<usize>,
}

fn first_ip(addrs: &[Multiaddr]) -> Option<IpAddr> {
    for a in addrs {
        for c in a.iter() {
            match c {
                Protocol::Ip4(ip) => return Some(IpAddr::V4(ip)),
                Protocol::Ip6(ip) => return Some(IpAddr::V6(ip)),
                _ => {}
            }
        }
    }
    None
}

pub async fn resolve_hostname(addrs: Vec<Multiaddr>) -> (Option<String>, Vec<String>) {
    let addr_strs: Vec<String> = addrs.iter().map(|a| a.to_string()).collect();
    let ip = first_ip(&addrs);
    let host = match ip {
        Some(ip) => tokio::task::spawn_blocking(move || dns_lookup::lookup_addr(&ip).ok())
            .await
            .ok()
            .flatten(),
        None => None,
    };
    (host, addr_strs)
}

pub fn emit(
    event: &str,
    peer: PeerId,
    peer_host: Option<String>,
    peer_addrs: Vec<String>,
    hash_part: &str,
    store_path: Option<&str>,
    bytes: Option<usize>,
) {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let ev = AuditEvent {
        ts,
        event,
        peer_id: peer.to_string(),
        peer_host,
        peer_addrs,
        hash_part,
        store_path,
        bytes,
    };
    match serde_json::to_string(&ev) {
        Ok(line) => tracing::info!(target: "nix_p2p_cache::audit", "{line}"),
        Err(e) => tracing::warn!(error = %e, "audit serialize failed"),
    }
}
