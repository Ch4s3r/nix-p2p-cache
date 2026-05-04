use assert_cmd::cargo::CommandCargoExt;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const SCHEMA: &str = r#"
CREATE TABLE ValidPaths (
  id integer primary key autoincrement not null,
  path text unique not null,
  hash text not null,
  registrationTime integer not null,
  deriver text,
  narSize integer,
  ultimate integer default 0,
  sigs text default null,
  ca text default null
);
CREATE TABLE Refs (
  referrer integer not null,
  reference integer not null,
  primary key (referrer, reference)
);
"#;

fn pick_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

struct NodeFixture {
    store: PathBuf,
    db: PathBuf,
    nar_bytes: Option<Vec<u8>>,
}

fn make_node(root: &Path, hash_part: Option<&str>) -> NodeFixture {
    let store = root.join("store");
    let db = root.join("db.sqlite");
    fs::create_dir_all(&store).unwrap();
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(SCHEMA).unwrap();

    let nar_bytes = if let Some(hp) = hash_part {
        let dir = store.join(format!("{hp}-test"));
        fs::create_dir_all(&dir).unwrap();
        let mut f = fs::File::create(dir.join("data.txt")).unwrap();
        f.write_all(b"hello-from-node-a\n").unwrap();

        let mut encoder = nix_nar::Encoder::new(&dir).unwrap();
        let mut buf = Vec::new();
        std::io::copy(&mut encoder, &mut buf).unwrap();
        let mut hasher = Sha256::new();
        hasher.update(&buf);
        let digest: [u8; 32] = hasher.finalize().into();
        let hash_str = format!("sha256:{}", hex::encode(digest));

        conn.execute(
            "INSERT INTO ValidPaths (path, hash, registrationTime, narSize) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![dir.to_str().unwrap(), hash_str, 0i64, buf.len() as i64],
        )
        .unwrap();
        Some(buf)
    } else {
        None
    };
    NodeFixture {
        store,
        db,
        nar_bytes,
    }
}

fn spawn(name: &str, port: u16, db: &Path, store: &Path) -> Child {
    Command::cargo_bin("nix-p2p-cache")
        .unwrap()
        .env("RUST_LOG", "info,nix_p2p_cache=debug")
        .args([
            "run",
            "--port",
            &port.to_string(),
            "--bind",
            "127.0.0.1",
            "--hostname",
            name,
            "--db",
        ])
        .arg(db)
        .arg("--store-dir")
        .arg(store)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

fn wait_http(port: u16) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if reqwest::blocking::get(format!("http://127.0.0.1:{port}/nix-cache-info"))
            .map(|r| r.status().is_success())
            .unwrap_or(false)
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    false
}

#[test]
fn node_b_fetches_narinfo_from_node_a_over_libp2p() {
    let hash_part = "abcd0123456789012345678901234567";

    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let node_a = make_node(dir_a.path(), Some(hash_part));
    let node_b = make_node(dir_b.path(), None);
    let expected_nar = node_a.nar_bytes.clone().unwrap();

    let port_a = pick_port();
    let port_b = pick_port();
    let mut a = spawn("two-node-a", port_a, &node_a.db, &node_a.store);
    let mut b = spawn("two-node-b", port_b, &node_b.db, &node_b.store);

    let ready = wait_http(port_a) && wait_http(port_b);

    let mut narinfo_status = 0;
    let mut narinfo_body = String::new();
    if ready {
        let deadline = Instant::now() + Duration::from_secs(25);
        while Instant::now() < deadline {
            match reqwest::blocking::get(format!("http://127.0.0.1:{port_b}/{hash_part}.narinfo")) {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    if status == 200 {
                        narinfo_status = status;
                        narinfo_body = resp.text().unwrap_or_default();
                        break;
                    }
                    narinfo_status = status;
                }
                Err(_) => {}
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    let mut nar_status = 0;
    let mut nar_bytes: Vec<u8> = Vec::new();
    if narinfo_status == 200 {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            match reqwest::blocking::get(format!("http://127.0.0.1:{port_b}/nar/{hash_part}.nar")) {
                Ok(resp) => {
                    nar_status = resp.status().as_u16();
                    if nar_status == 200 {
                        nar_bytes = resp.bytes().unwrap_or_default().to_vec();
                        break;
                    }
                }
                Err(_) => {}
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    }

    let _ = a.kill();
    let _ = b.kill();
    let _ = a.wait();
    let _ = b.wait();

    assert!(ready, "nodes did not become ready");
    assert_eq!(
        narinfo_status, 200,
        "B never resolved {hash_part} via peer A (last status {narinfo_status})"
    );
    assert!(
        narinfo_body.contains(&format!(
            "StorePath: {}/{hash_part}-test",
            node_a.store.display()
        )),
        "narinfo body missing StorePath: {narinfo_body}"
    );
    assert!(narinfo_body.contains("Sig: nix-p2p-cache-two-node-b:"));
    assert_eq!(
        nar_status, 200,
        "B never fetched NAR bytes from A (last status {nar_status})"
    );
    assert_eq!(
        nar_bytes.len(),
        expected_nar.len(),
        "NAR length mismatch: got {} expected {}",
        nar_bytes.len(),
        expected_nar.len()
    );
    assert_eq!(
        nar_bytes, expected_nar,
        "NAR bytes returned by B differ from A's source NAR"
    );
}

fn derive_pubkey(hostname: &str) -> VerifyingKey {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"nix-p2p-cache.v1|");
    hasher.update(hostname.as_bytes());
    let seed: [u8; 32] = hasher.finalize().into();
    ed25519_dalek::SigningKey::from_bytes(&seed).verifying_key()
}

fn parse_narinfo(body: &str) -> std::collections::HashMap<&str, &str> {
    body.lines().filter_map(|l| l.split_once(": ")).collect()
}

fn fingerprint(fields: &std::collections::HashMap<&str, &str>) -> String {
    let refs: Vec<String> = fields["References"]
        .split_whitespace()
        .filter(|s| !s.is_empty())
        .map(|b| format!("/nix/store/{b}"))
        .collect();
    format!(
        "1;{};{};{};{}",
        fields["StorePath"],
        fields["NarHash"],
        fields["NarSize"],
        refs.join(",")
    )
}

#[test]
fn local_hit_returns_signed_narinfo_with_valid_signature() {
    let hash_part = "abcd0123456789012345678901234567";
    let dir_a = tempfile::tempdir().unwrap();
    let node_a = make_node(dir_a.path(), Some(hash_part));
    let port = pick_port();
    let host = "siglocal";
    let mut child = spawn(host, port, &node_a.db, &node_a.store);
    let ready = wait_http(port);
    let body = if ready {
        reqwest::blocking::get(format!("http://127.0.0.1:{port}/{hash_part}.narinfo"))
            .ok()
            .and_then(|r| {
                if r.status().is_success() {
                    r.text().ok()
                } else {
                    None
                }
            })
            .unwrap_or_default()
    } else {
        String::new()
    };
    let _ = child.kill();
    let _ = child.wait();

    assert!(ready);
    assert!(!body.is_empty(), "no narinfo body for local hit");

    let fields = parse_narinfo(&body);
    let sig_line = fields["Sig"];
    let (name, b64) = sig_line.split_once(':').unwrap();
    assert_eq!(name, format!("nix-p2p-cache-{host}"));
    let sig_bytes = data_encoding::BASE64.decode(b64.as_bytes()).unwrap();
    let sig = Signature::from_slice(&sig_bytes).unwrap();
    let pubkey = derive_pubkey(host);
    let fp = fingerprint(&fields);
    pubkey
        .verify(fp.as_bytes(), &sig)
        .expect("signature must verify against derived pubkey for local hit");
    let expected_path = format!("{}/{hash_part}-test", node_a.store.display());
    assert_eq!(fields["StorePath"], expected_path);
    assert_eq!(fields["URL"], format!("nar/{hash_part}.nar"));
    assert_eq!(fields["Compression"], "none");
}

#[test]
fn references_are_emitted_in_lexicographic_order_in_narinfo() {
    let hash_part = "abcd0123456789012345678901234567";
    let dir = tempfile::tempdir().unwrap();
    let node = make_node(dir.path(), Some(hash_part));
    // Insert two reference rows (different hash prefix), associate with referrer id.
    let conn = Connection::open(&node.db).unwrap();
    let referrer_id: i64 = conn
        .query_row(
            "SELECT id FROM ValidPaths WHERE path LIKE ?1",
            rusqlite::params![format!("{}/{hash_part}-%", node.store.display())],
            |r| r.get(0),
        )
        .unwrap();
    let p_y = format!(
        "{}/yyyy0123456789012345678901234567-y",
        node.store.display()
    );
    let p_a = format!(
        "{}/aaaa0123456789012345678901234567-a",
        node.store.display()
    );
    for p in [&p_y, &p_a] {
        std::fs::create_dir_all(p).unwrap();
        conn.execute(
            "INSERT INTO ValidPaths (path, hash, registrationTime, narSize) VALUES (?1, ?2, 0, 0)",
            rusqlite::params![p, "sha256:00"],
        )
        .unwrap();
    }
    let id_y: i64 = conn
        .query_row("SELECT id FROM ValidPaths WHERE path = ?1", [&p_y], |r| {
            r.get(0)
        })
        .unwrap();
    let id_a: i64 = conn
        .query_row("SELECT id FROM ValidPaths WHERE path = ?1", [&p_a], |r| {
            r.get(0)
        })
        .unwrap();
    // Insert in reverse order to prove ORDER BY in our query, not insertion order.
    conn.execute(
        "INSERT INTO Refs (referrer, reference) VALUES (?1, ?2), (?1, ?3)",
        rusqlite::params![referrer_id, id_y, id_a],
    )
    .unwrap();
    drop(conn);

    let port = pick_port();
    let mut child = spawn("ordhost", port, &node.db, &node.store);
    let ready = wait_http(port);
    let body = if ready {
        reqwest::blocking::get(format!("http://127.0.0.1:{port}/{hash_part}.narinfo"))
            .ok()
            .and_then(|r| r.text().ok())
            .unwrap_or_default()
    } else {
        String::new()
    };
    let _ = child.kill();
    let _ = child.wait();

    assert!(ready);
    let refs_line = body
        .lines()
        .find(|l| l.starts_with("References: "))
        .expect("references line");
    let names: Vec<&str> = refs_line["References: ".len()..]
        .split_whitespace()
        .collect();
    assert_eq!(
        names,
        vec![
            "aaaa0123456789012345678901234567-a",
            "yyyy0123456789012345678901234567-y",
        ],
        "expected lexicographic order"
    );
}
