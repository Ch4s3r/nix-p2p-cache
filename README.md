# nix-p2p-cache

> ⚠️ **Experimental.** Not end-to-end tested yet on real two-host LAN setups. Use at your own risk.

> Peer-to-peer LAN substituter for `/nix/store`. Stop hammering `cache.nixos.org` when the machine next to you already built the path.

A single Rust daemon you run on every nix machine on your LAN. Each node auto-discovers its peers via mDNS and shares its read-only `/nix/store` over libp2p. Configure once via the included nix-darwin / NixOS modules — no per-peer key management, no manual substituter registration as machines come and go.

Inspired by [`cid-chan/peerix`](https://github.com/cid-chan/peerix); rewritten in Rust with libp2p for the peer-to-peer transport and a deterministic-from-hostname signing key so the trust setup collapses into one `services.nix-p2p-cache.enable = true;`.

## What it does

```
┌─────────┐   http://127.0.0.1:5555    ┌──────────────┐    libp2p QUIC + mDNS     ┌──────────────┐
│   nix   │ ─────────────────────────► │ nix-p2p-cache│◄─────────────────────────►│ nix-p2p-cache│
│ (build) │   /nix-cache-info          │  (this host) │      LAN peers            │ (other host) │
└─────────┘   /<hash>.narinfo          └──────┬───────┘                           └──────────────┘
              /nar/<hash>.nar                 │
                                              ▼
                                         /nix/store (RO)
                                         /nix/var/nix/db (RO)
```

Two async servers in one binary process:

- **HTTP** (axum, TCP) — implements the standard nix binary cache protocol so `nix` treats us as just another substituter.
- **libp2p** (QUIC over UDP) — mDNS discovery + `request_response` for `Has` queries and `GetNar` transfers between peers.

Both bind dual-stack (IPv4 + IPv6) on the same port number. Driven concurrently with `tokio::try_join!`.

## Why

- No more redundant `cache.nixos.org` downloads when 5 machines on the same LAN need the same closure.
- Faster CI: workers fetch from the developer who just built the same derivation, not a cold upstream cache.
- No central infrastructure: no `nix-serve` to babysit, no S3 bucket, no signing keys to distribute.
- Drop-in: the included nix module wires substituters and trusted public keys for you. Joining the cluster = one line in `flake.nix`.

## Quick start

### nix-darwin

```nix
{
  inputs.nix-p2p-cache.url = "github:Ch4s3r/nix-p2p-cache";

  outputs = { self, nix-darwin, nix-p2p-cache, ... }: {
    darwinConfigurations."mybox" = nix-darwin.lib.darwinSystem {
      modules = [
        nix-p2p-cache.darwinModules.default
      ];
    };
  };
}
```

`darwin-rebuild switch`, done. Importing the module enables the service by default — it installs a `launchd` daemon and wires `http://127.0.0.1:5555` + a fixed shared public key into `nix.conf`. No per-host key generation, no IFD. To opt out, set `services.nix-p2p-cache.enable = false;`.

### NixOS

```nix
{
  imports = [ nix-p2p-cache.nixosModules.default ];
}
```

Installs a `systemd` service running as a dedicated `nix-p2p-cache` user, with `/nix/store` and `/nix/var/nix/db` mounted read-only, and the firewall opened for the configured port.

### Manual (no nix module)

```sh
cargo build --release
./target/release/nix-p2p-cache run --port 5555 --bind ::
```

Then in `/etc/nix/nix.conf`:

```
substituters = http://127.0.0.1:5555 https://cache.nixos.org
trusted-public-keys = nix-p2p-cache-shared:zfv4gOrH/QCQjKhPybhUvhgzM5vj/2zq/F3iplvDbxE= cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY=
```

`nix-p2p-cache derive-pubkey` prints the public key line. It's the same on every host — the keypair is deterministically derived from a constant in the source.

## How a fetch flows

1. `nix build` asks the local HTTP shim for `<hash>.narinfo`.
2. The shim queries the local nix DB (`/nix/var/nix/db/db.sqlite`, read-only via `rusqlite`).
3. **Local hit:** narinfo is built from DB metadata, NAR is encoded in-process via the `nix-nar` crate, both signed with the local key, and returned. No shellouts to `nix-store`.
4. **Local miss:** the shim fans out a libp2p `Has` request to every mDNS-discovered peer. First Some-responder wins for narinfo; for the NAR we keep all responders and try the next on failure.
5. The shim re-signs the narinfo with the local key before handing it to nix — so nix only ever validates signatures from `127.0.0.1`, never directly from peers.

## CLI

```
nix-p2p-cache run            --port 5555 --bind ::
nix-p2p-cache keygen         --out <dir>     # write key + key.pub
nix-p2p-cache derive-pubkey                  # print the shared trusted-public-keys line
nix-p2p-cache setup                          # print nix.conf snippet
```

## Module options

| Option | Default | Meaning |
|---|---|---|
| `services.nix-p2p-cache.enable` | `true` | Set to `false` to opt out. Enabled automatically when the module is imported. |
| `services.nix-p2p-cache.port` | `5555` | TCP (HTTP) and UDP (QUIC) port. |
| `services.nix-p2p-cache.bind` | `"::"` | HTTP bind address. |
| `services.nix-p2p-cache.openFirewall` | `true` | Open the port (NixOS only). |
| `services.nix-p2p-cache.extraSubstituters` | `[ cache.nixos.org ]` | Additional upstream substituters. |
| `services.nix-p2p-cache.extraTrustedPublicKeys` | `[ cache.nixos.org-1 ]` | Additional trusted public keys. |
| `services.nix-p2p-cache.package` | flake's `packages.default` | Override the package. |

## Trust model

LAN-trusted. The shim signs narinfo with a fixed shared keypair derived deterministically from a constant in the source:

```
seed   = blake3("nix-p2p-cache.shared.v1")
key    = ed25519 from seed
pubkey = nix-p2p-cache-shared:zfv4gOrH/QCQjKhPybhUvhgzM5vj/2zq/F3iplvDbxE=
```

**Why this is OK in practice:** NARs are always re-signed locally on `127.0.0.1` before nix sees them. The trust boundary is the loopback shim, not the peers. The libp2p layer authenticates peers separately via its own noise keypair.

**The trade-off:** anyone with the binary holds the private half. Because nix's `trusted-public-keys` is global across all substituters, an attacker who can MITM your HTTPS connection to `cache.nixos.org` (e.g., compromised CA, hostile Wi-Fi, DNS hijack) could in theory feed nix a malicious narinfo signed with the shared key. Mitigations: use HTTPS pinning where possible, drop `cache.nixos.org` from `extraSubstituters` if you only want LAN content, and don't run this on hostile networks.

**Do not expose the libp2p port to the internet.** This is a LAN tool. No rate limiting, no auth — any peer can ask any node for any path in its store.

## Verify it actually works

End-to-end on two machines (or [in tests on one](#development)):

```sh
# Host A
nix build nixpkgs#hello

# Host B (must NOT have hello)
nix store delete /nix/store/<hello-hash>-hello-* 2>/dev/null || true
nix copy --from http://127.0.0.1:5555 /nix/store/<hello-hash>-hello-*
```

B should fetch the closure from A. Check logs on B for `mdns discovered peer` and on the daemon for incoming `Has` requests.

## Inspecting daemon logs (who served what)

The daemon emits one structured **JSON audit line per peer-served event** under the tracing target `nix_p2p_cache::audit`. Two events:

- `narinfo_from_peer` — a peer answered the metadata lookup
- `nar_pulled_from_peer` — a peer delivered the actual NAR bytes

Each line includes the libp2p `peer_id`, the resolved `peer_host` (reverse-DNS lookup of the peer's first multiaddr), all known `peer_addrs`, the `hash_part`, the `store_path`, and (for NAR events) `bytes`:

```json
{"ts":1714838400,"event":"narinfo_from_peer","peer_id":"12D3KooW…","peer_host":"alice.local","peer_addrs":["/ip4/192.168.1.42/udp/5555/quic-v1"],"hash_part":"abc…","store_path":"/nix/store/abc…-hello-2.12"}
{"ts":1714838401,"event":"nar_pulled_from_peer","peer_id":"12D3KooW…","peer_host":"alice.local","peer_addrs":["/ip4/192.168.1.42/udp/5555/quic-v1"],"hash_part":"abc…","store_path":"/nix/store/abc…-hello-2.12","bytes":204800}
```

`peer_host` is `null` if reverse DNS fails. To suppress non-audit noise, set `RUST_LOG=nix_p2p_cache::audit=info,warn`.

### macOS (nix-darwin / launchd)

```sh
# live tail
log stream --predicate 'process == "nix-p2p-cache"' --info \
  | rg '"event":"' \
  | jq .

# last hour
log show --predicate 'process == "nix-p2p-cache"' --info --last 1h \
  | rg '"event":"nar_pulled_from_peer"' | jq .
```

If you set `StandardOutPath` / `StandardErrorPath` in the launchd plist, tail those files instead.

### NixOS (systemd)

```sh
journalctl -u nix-p2p-cache -f \
  | rg '"event":"' | jq .

# only what got pulled in the last hour
journalctl -u nix-p2p-cache --since '1 hour ago' \
  | rg '"event":"nar_pulled_from_peer"' | jq .
```

### Manual / foreground

The daemon writes to stderr via `tracing`. Set `RUST_LOG=info` (default) or `RUST_LOG=nix_p2p_cache=debug` for the mDNS-level events too.

## Development

```sh
devenv shell      # enter rust dev shell
devenv up         # run daemon as managed process
devenv test       # cargo build --locked && cargo test --all
cargo test        # 23 tests: unit + CLI + HTTP + mDNS smoke + two-node end-to-end
```

The two-node end-to-end test (`tests/two_node.rs`) builds two synthetic nix DBs, plants a real NAR-encoded path on one node, then asserts that the other node fetches both the narinfo and the actual NAR bytes through libp2p — without any external nix install.

## Limitations

- **NARs are buffered in memory.** Multi-GB closures will OOM. Streaming refactor is on the roadmap.
- **No request deduplication.** Concurrent fetches for the same hash hit the network independently.
- **mDNS-only discovery.** No WAN, no NAT traversal, no static peer lists yet.
- **No compression negotiation.** NARs are served identity-encoded; nix accepts this.

## Project layout

```
src/
  main.rs           CLI (clap): run | keygen | derive-pubkey | setup
  keys.rs           hostname → ed25519
  narinfo.rs        narinfo render + fingerprint + signing
  store.rs          rusqlite read-only DB + nix-nar encoder
  peer_protocol.rs  PeerRequest / PeerResponse types
  p2p.rs            libp2p Swarm: mDNS + request_response over QUIC, peer failover
  http.rs           axum: /nix-cache-info, /{hash}.narinfo, /nar/{hash}.nar
modules/
  common.nix        shared options + nix.settings wiring (IFD pubkey)
  nixos.nix         systemd service + user/group + firewall
  darwin.nix        launchd daemon + activation script
flake.nix           packages, apps, modules
devenv.nix          dev shell, processes, enterTest
```

## Acknowledgements

[`cid-chan/peerix`](https://github.com/cid-chan/peerix) — original concept and module shape.
