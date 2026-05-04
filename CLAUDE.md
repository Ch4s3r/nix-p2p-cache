# nix-p2p-cache — agent notes

## What this is
Rust binary that runs **two async servers in one process** so a LAN of nix machines can share `/nix/store` peer-to-peer:
- HTTP (axum, TCP) — speaks the standard nix binary cache protocol on `127.0.0.1:<port>`. Configure as a substituter.
- libp2p (QUIC over UDP) — discovers peers via mDNS and serves NAR data peer-to-peer.

Both bind dual-stack (IPv4 + IPv6). Driven concurrently with `tokio::try_join!` in `src/main.rs`.

## Hard constraints
- **Pure Rust where possible.** No shelling out to `nix-store`, `nix path-info`, etc. Local store reads use `rusqlite` against `/nix/var/nix/db/db.sqlite` (read-only). NAR encoding uses the `nix-nar` crate.
- **Shared deterministic signing key.** `blake3("nix-p2p-cache.shared.v1")` → ed25519 seed. Same key on every host. The pubkey is hardcoded in `modules/common.nix` (`sharedPublicKey`); it must match the output of `nix-p2p-cache derive-pubkey`. If you change the domain string in `src/keys.rs`, regenerate the pubkey and update the module — both must move together.
- **LAN trust model.** Anyone with the binary holds the private half. Acceptable because the signing boundary is the loopback shim; nix only ever validates signatures from `127.0.0.1`. Don't propose per-peer key registration. Don't propose moving the constant key to runtime generation — that re-introduces the activation-script chicken-and-egg.
- **One process, two servers.** Don't merge them, don't split into two binaries. They share a port number (TCP for HTTP, UDP for QUIC).

## Layout
```
src/
  main.rs           clap CLI: run | keygen | derive-pubkey | setup
  keys.rs           shared ed25519 derived from constant seed; key file I/O
  narinfo.rs        narinfo render + fingerprint + sign
  store.rs          rusqlite read-only DB + nix-nar encoder
  peer_protocol.rs  PeerRequest / PeerResponse (serde)
  p2p.rs            libp2p Swarm: mdns + identify + request_response over QUIC
  http.rs           axum routes: /nix-cache-info, /{hash}.narinfo, /nar/{hash}.nar
modules/
  common.nix        shared options + nix.settings wiring (hardcoded sharedPublicKey)
  nixos.nix         systemd service + user/group + firewall
  darwin.nix        launchd daemon
flake.nix           packages.default, apps.default, {nixos,darwin}Modules.default
devenv.nix          dev shell, processes.nix-p2p-cache, enterTest
```

## CLI subcommands
- `run --port <p> --bind <addr> [--db <path>] [--store-dir <path>]` — main daemon.
- `keygen --out <dir>` — write `key` + `key.pub` (the shared keypair, idempotent).
- `derive-pubkey` — prints `nix-p2p-cache-shared:<b64>`. Used to verify the constant in modules/common.nix matches the binary.
- `setup [--port <p>]` — print nix.conf snippet for non-nix users.

## Adding deps
Project rule (from `~/CLAUDE.md`): after adding a dep, run `cargo upgrade --incompatible`. **Avoid `bincode = "3"`** — that crate is sabotaged with `compile_error!`. Use serde_json or a different format if you need wire serialization beyond what `libp2p::request_response::cbor` provides.

## Building / testing
```sh
cargo check
cargo test            # 9 unit + 3 CLI + 2 HTTP integration
devenv test           # canonical (cargo build --locked && cargo test --all)
devenv up             # run daemon
```
HTTP integration tests spawn the binary on an ephemeral port and use `reqwest::blocking`. They pass `--db /nonexistent.sqlite`; `store::lookup_by_hash` short-circuits to `Ok(None)` when the DB file is missing — keep that behavior or the tests break.

## Axum 0.8 path syntax
Routes use `{name}` not `:name`. Only one capture per segment, so the file routes capture the whole filename and strip suffixes inside the handler (`.narinfo` / `.nar`).

## Known dead-code warnings
`StorePathRow.id`, `default_for_system`, `store_dir`, `hash_nar_stream`, `PendingHas.hash_part`, `narinfo::store_path_hash_part` — kept on purpose as part of the public-ish surface. Don't aggressively delete them without checking callers added since.

## Known limitations / future work
- **Whole-NAR-in-memory.** Both libp2p and HTTP responses load the full NAR into a `Vec<u8>`. Multi-GB NARs will OOM. A real fix needs streaming through libp2p (single request_response message → switch to a stream protocol or chunked frames) plus `axum::body::Body::from_stream` on the HTTP side.
- **No request deduplication.** N concurrent fetches for the same hash → N parallel fan-outs. Add an in-flight registry keyed by `hash_part` whose entries return shared receivers.
- **No retries on transient libp2p errors.** A `Has` request that races mDNS dial setup is recorded as a "no" once. Retry once on `OutboundFailure::DialFailure`.
- **NAR-vs-`nix-store --dump` byte-equivalence is unverified.** Tests cover B == A round-trip, not parity with stock nix. If `nix-nar` ever drifts from the reference encoder, narHash mismatches at the client. Add a test gated on `nix` being on PATH.
- **mDNS service name is fixed by libp2p (`_p2p._udp.local.`).** Cannot run isolated test domains without forking libp2p-mdns.

## When verifying end-to-end
Two LAN hosts. On A: `nix build nixpkgs#hello` then start the daemon. On B: `nix store delete <path>` first, then `nix copy --from http://127.0.0.1:5555 <path>`. B's logs should show mDNS discovery → fan-out → fetch from A. Without two real hosts you can't fully verify the p2p path; say so explicitly rather than claiming success.
