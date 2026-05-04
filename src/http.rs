use crate::keys::LocalKey;
use crate::narinfo::STORE_DIR;
use crate::p2p::P2pHandle;
use crate::store::{LocalStore, normalize_nar_hash_pub};
use anyhow::Result;
use axum::Router;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use std::io::Read;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{debug, info};

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<LocalStore>,
    pub key: Arc<LocalKey>,
    pub p2p: P2pHandle,
}

pub async fn serve(state: AppState, addr: SocketAddr) -> Result<()> {
    let app = Router::new()
        .route("/nix-cache-info", get(nix_cache_info))
        .route("/{file}", get(get_narinfo))
        .route("/nar/{file}", get(get_nar))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, "http listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn nix_cache_info() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain")],
        format!("StoreDir: {STORE_DIR}\nWantMassQuery: 1\nPriority: 30\n"),
    )
}

async fn get_narinfo(State(state): State<AppState>, Path(file): Path<String>) -> Response {
    let Some(hash_part) = file.strip_suffix(".narinfo") else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let hash_part = hash_part.to_string();
    if hash_part.len() != 32 {
        return (StatusCode::NOT_FOUND, "bad hash").into_response();
    }
    let nar_url = format!("nar/{hash_part}.nar");
    match state.store.narinfo_for(&hash_part, &nar_url) {
        Ok(Some(mut info)) => {
            info.sign_with(&state.key);
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/x-nix-narinfo")],
                info.render(),
            )
                .into_response()
        }
        Ok(None) => {
            // Cache miss locally — query peers.
            if let Some((peer, meta)) = state.p2p.find_path(&hash_part).await {
                debug!(%peer, hash_part, "found path on peer");
                let nar_hash = match normalize_nar_hash_pub(&meta.nar_hash) {
                    Ok(v) => v,
                    Err(_) => return (StatusCode::BAD_GATEWAY, "bad peer narHash").into_response(),
                };
                let mut info = crate::narinfo::NarInfo {
                    store_path: meta.store_path,
                    url: nar_url,
                    compression: "none".into(),
                    file_hash: nar_hash.clone(),
                    file_size: meta.nar_size,
                    nar_hash,
                    nar_size: meta.nar_size,
                    references: meta.references,
                    deriver: meta.deriver,
                    ca: meta.ca.clone(),
                    sig: None,
                };
                info.sign_with(&state.key);
                let mut headers = HeaderMap::new();
                headers.insert(header::CONTENT_TYPE, "text/x-nix-narinfo".parse().unwrap());
                headers.insert(
                    "X-Nix-P2p-Peer",
                    peer.to_string()
                        .parse()
                        .unwrap_or_else(|_| "unknown".parse().unwrap()),
                );
                (StatusCode::OK, headers, info.render()).into_response()
            } else {
                StatusCode::NOT_FOUND.into_response()
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "narinfo handler error");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn get_nar(State(state): State<AppState>, Path(file): Path<String>) -> Response {
    let hash_part = file.strip_suffix(".nar").unwrap_or(&file).to_string();
    let local = state.store.clone();
    let h = hash_part.clone();
    let local_path = tokio::task::spawn_blocking(move || local.lookup_by_hash(&h))
        .await
        .ok()
        .and_then(|r| r.ok())
        .flatten();

    if let Some(row) = local_path {
        let store = state.store.clone();
        let path = row.path.clone();
        let stream_res = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
            let mut enc = store.open_nar_stream(&path)?;
            let mut buf = Vec::with_capacity(row.nar_size as usize);
            std::io::copy(&mut enc, &mut buf)?;
            Ok(buf)
        })
        .await;
        match stream_res {
            Ok(Ok(buf)) => {
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/x-nix-archive")],
                    buf,
                )
                    .into_response();
            }
            _ => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    }

    let candidates = state.p2p.find_paths(&hash_part).await;
    if !candidates.is_empty() {
        if let Some(bytes) = state
            .p2p
            .fetch_nar_with_failover(&candidates, &hash_part)
            .await
        {
            return (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/x-nix-archive")],
                bytes,
            )
                .into_response();
        }
    }
    StatusCode::NOT_FOUND.into_response()
}

pub fn _used(_b: Body, _r: &dyn Read) {}
