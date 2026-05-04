use crate::peer_protocol::{MAX_MESSAGE_BYTES, PROTOCOL_NAME, PathMeta, PeerRequest, PeerResponse};
use crate::store::LocalStore;
use anyhow::Result;
use futures::StreamExt;
use libp2p::request_response::{
    self, ProtocolSupport, ResponseChannel, cbor::Behaviour as RrBehaviour,
    cbor::codec::Codec as CborCodec,
};
use libp2p::swarm::{NetworkBehaviour, SwarmEvent};
use libp2p::{PeerId, StreamProtocol, mdns};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};

#[derive(NetworkBehaviour)]
pub struct CacheBehaviour {
    pub mdns: mdns::tokio::Behaviour,
    pub rr: RrBehaviour<PeerRequest, PeerResponse>,
}

#[derive(Debug)]
pub enum Command {
    FindPath {
        hash_part: String,
        wait_all: bool,
        reply: oneshot::Sender<Vec<(PeerId, PathMeta)>>,
    },
    FetchNar {
        peer: PeerId,
        hash_part: String,
        reply: oneshot::Sender<Option<Vec<u8>>>,
    },
}

#[derive(Clone)]
pub struct P2pHandle {
    tx: mpsc::Sender<Command>,
}

impl P2pHandle {
    async fn find_inner(&self, hash_part: &str, wait_all: bool) -> Vec<(PeerId, PathMeta)> {
        let (tx, rx) = oneshot::channel();
        if self
            .tx
            .send(Command::FindPath {
                hash_part: hash_part.to_string(),
                wait_all,
                reply: tx,
            })
            .await
            .is_err()
        {
            return Vec::new();
        }
        rx.await.unwrap_or_default()
    }

    pub async fn find_paths(&self, hash_part: &str) -> Vec<(PeerId, PathMeta)> {
        self.find_inner(hash_part, true).await
    }

    pub async fn find_path(&self, hash_part: &str) -> Option<(PeerId, PathMeta)> {
        self.find_inner(hash_part, false).await.into_iter().next()
    }

    pub async fn fetch_nar(&self, peer: PeerId, hash_part: &str) -> Option<Vec<u8>> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(Command::FetchNar {
                peer,
                hash_part: hash_part.to_string(),
                reply: tx,
            })
            .await
            .ok()?;
        rx.await.ok().flatten()
    }

    pub async fn fetch_nar_with_failover(
        &self,
        peers: &[(PeerId, PathMeta)],
        hash_part: &str,
    ) -> Option<Vec<u8>> {
        for (peer, _meta) in peers {
            if let Some(bytes) = self.fetch_nar(*peer, hash_part).await {
                return Some(bytes);
            }
            tracing::warn!(%peer, hash_part, "peer failed to deliver NAR; trying next");
        }
        None
    }
}

pub fn start(
    store: Arc<LocalStore>,
    port: u16,
) -> (P2pHandle, impl std::future::Future<Output = Result<()>>) {
    let (tx, rx) = mpsc::channel(256);
    let fut = async move {
        match run(store, rx, port).await {
            Ok(()) => Ok(()),
            Err(err) => {
                warn!(error = %err, "p2p task exited");
                Err(err)
            }
        }
    };
    (P2pHandle { tx }, fut)
}

struct PendingHas {
    expected: usize,
    received: usize,
    found: Vec<(PeerId, PathMeta)>,
    wait_all: bool,
    reply: Option<oneshot::Sender<Vec<(PeerId, PathMeta)>>>,
}

struct PendingNar {
    reply: Option<oneshot::Sender<Option<Vec<u8>>>>,
}

async fn run(store: Arc<LocalStore>, mut rx: mpsc::Receiver<Command>, port: u16) -> Result<()> {
    let mut swarm = libp2p::SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_quic()
        .with_behaviour(|key| {
            let mdns =
                mdns::tokio::Behaviour::new(mdns::Config::default(), key.public().to_peer_id())?;
            let rr_cfg =
                request_response::Config::default().with_request_timeout(Duration::from_secs(120));
            let codec = CborCodec::<PeerRequest, PeerResponse>::default()
                .set_request_size_maximum(1024 * 1024)
                .set_response_size_maximum(MAX_MESSAGE_BYTES as u64);
            let rr = RrBehaviour::<PeerRequest, PeerResponse>::with_codec(
                codec,
                std::iter::once((StreamProtocol::new(PROTOCOL_NAME), ProtocolSupport::Full)),
                rr_cfg,
            );
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(CacheBehaviour { mdns, rr })
        })?
        .build();

    swarm.listen_on(format!("/ip4/0.0.0.0/udp/{port}/quic-v1").parse()?)?;
    swarm.listen_on(format!("/ip6/::/udp/{port}/quic-v1").parse()?)?;
    info!(port, "p2p listening");

    let mut peers: HashSet<PeerId> = HashSet::new();
    let mut pending_has: HashMap<request_response::OutboundRequestId, String> = HashMap::new();
    let mut pending_has_state: HashMap<String, PendingHas> = HashMap::new();
    let mut pending_nar: HashMap<request_response::OutboundRequestId, PendingNar> = HashMap::new();

    loop {
        tokio::select! {
            cmd = rx.recv() => {
                let Some(cmd) = cmd else { break };
                match cmd {
                    Command::FindPath { hash_part, wait_all, reply } => {
                        if peers.is_empty() {
                            let _ = reply.send(Vec::new());
                            continue;
                        }
                        let req = PeerRequest::Has { hash_part: hash_part.clone() };
                        let mut expected = 0;
                        for p in peers.iter() {
                            let id = swarm.behaviour_mut().rr.send_request(p, req.clone());
                            pending_has.insert(id, hash_part.clone());
                            expected += 1;
                        }
                        pending_has_state.insert(hash_part.clone(), PendingHas {
                            expected,
                            received: 0,
                            found: Vec::new(),
                            wait_all,
                            reply: Some(reply),
                        });
                    }
                    Command::FetchNar { peer, hash_part, reply } => {
                        let req = PeerRequest::GetNar { hash_part };
                        let id = swarm.behaviour_mut().rr.send_request(&peer, req);
                        pending_nar.insert(id, PendingNar { reply: Some(reply) });
                    }
                }
            }
            event = swarm.select_next_some() => {
                match event {
                    SwarmEvent::Behaviour(CacheBehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                        for (peer, addr) in list {
                            debug!(%peer, %addr, "mdns discovered peer");
                            swarm.add_peer_address(peer, addr.clone());
                            peers.insert(peer);
                        }
                    }
                    SwarmEvent::Behaviour(CacheBehaviourEvent::Mdns(mdns::Event::Expired(list))) => {
                        for (peer, _) in list { peers.remove(&peer); }
                    }
                    SwarmEvent::Behaviour(CacheBehaviourEvent::Rr(request_response::Event::Message { peer, message, .. })) => {
                        match message {
                            request_response::Message::Request { request, channel, .. } => {
                                handle_incoming(&store, request, channel, &mut swarm).await;
                            }
                            request_response::Message::Response { request_id, response } => {
                                if let Some(hash_part) = pending_has.remove(&request_id) {
                                    if let Some(state) = pending_has_state.get_mut(&hash_part) {
                                        state.received += 1;
                                        if let PeerResponse::Has(Some(meta)) = &response {
                                            state.found.push((peer, meta.clone()));
                                        }
                                        let early =
                                            !state.wait_all && !state.found.is_empty();
                                        if early || state.received >= state.expected {
                                            if let Some(reply) = state.reply.take() {
                                                let _ = reply.send(state.found.clone());
                                            }
                                        }
                                        if state.received >= state.expected {
                                            pending_has_state.remove(&hash_part);
                                        }
                                    }
                                } else if let Some(mut p) = pending_nar.remove(&request_id) {
                                    if let Some(reply) = p.reply.take() {
                                        let bytes = match response {
                                            PeerResponse::Nar(b) => b,
                                            _ => None,
                                        };
                                        let _ = reply.send(bytes);
                                    }
                                }
                            }
                        }
                    }
                    SwarmEvent::Behaviour(CacheBehaviourEvent::Rr(request_response::Event::OutboundFailure { request_id, .. })) => {
                        if let Some(hash_part) = pending_has.remove(&request_id) {
                            if let Some(state) = pending_has_state.get_mut(&hash_part) {
                                state.received += 1;
                                if state.received >= state.expected {
                                    if let Some(reply) = state.reply.take() {
                                        let _ = reply.send(state.found.clone());
                                    }
                                    pending_has_state.remove(&hash_part);
                                }
                            }
                        }
                        if let Some(mut p) = pending_nar.remove(&request_id) {
                            if let Some(reply) = p.reply.take() {
                                let _ = reply.send(None);
                            }
                        }
                    }
                    SwarmEvent::NewListenAddr { address, .. } => info!(%address, "p2p listening on"),
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

async fn handle_incoming(
    store: &Arc<LocalStore>,
    request: PeerRequest,
    channel: ResponseChannel<PeerResponse>,
    swarm: &mut libp2p::Swarm<CacheBehaviour>,
) {
    let response = match request {
        PeerRequest::Has { hash_part } => {
            let store = store.clone();
            let r = tokio::task::spawn_blocking(move || store.lookup_by_hash(&hash_part)).await;
            match r {
                Ok(Ok(Some(row))) => PeerResponse::Has(Some(PathMeta {
                    store_path: row.path,
                    nar_hash: row.nar_hash_db,
                    nar_size: row.nar_size,
                    references: row.references,
                    deriver: row.deriver,
                    ca: row.ca,
                })),
                Ok(Ok(None)) => PeerResponse::Has(None),
                Ok(Err(e)) => PeerResponse::Error(e.to_string()),
                Err(e) => PeerResponse::Error(format!("join: {e}")),
            }
        }
        PeerRequest::GetNar { hash_part } => {
            let store = store.clone();
            let r: Result<Option<Vec<u8>>, anyhow::Error> =
                tokio::task::spawn_blocking(move || {
                    let Some(row) = store.lookup_by_hash(&hash_part)? else {
                        return Ok::<Option<Vec<u8>>, anyhow::Error>(None);
                    };
                    let mut enc = store.open_nar_stream(&row.path)?;
                    let mut buf = Vec::with_capacity(row.nar_size as usize);
                    std::io::copy(&mut enc, &mut buf)?;
                    if buf.len() > MAX_MESSAGE_BYTES {
                        anyhow::bail!("NAR larger than max message size");
                    }
                    Ok(Some(buf))
                })
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("join: {e}")));
            match r {
                Ok(b) => PeerResponse::Nar(b),
                Err(e) => PeerResponse::Error(e.to_string()),
            }
        }
    };
    let _ = swarm.behaviour_mut().rr.send_response(channel, response);
}
