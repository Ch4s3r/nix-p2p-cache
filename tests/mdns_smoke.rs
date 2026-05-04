use assert_cmd::cargo::CommandCargoExt;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn pick_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

fn spawn_node(name: &str, port: u16) -> std::process::Child {
    Command::cargo_bin("nix-p2p-cache")
        .unwrap()
        .env("RUST_LOG", "info,nix_p2p_cache::p2p=debug,libp2p_mdns=info")
        .args([
            "run",
            "--port",
            &port.to_string(),
            "--bind",
            "127.0.0.1",
            "--hostname",
            name,
            "--db",
            "/nonexistent.sqlite",
            "--store-dir",
            "/nix/store",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

#[test]
fn two_local_instances_discover_each_other_via_mdns() {
    let port_a = pick_port();
    let port_b = pick_port();
    let mut a = spawn_node("smoke-a", port_a);
    let mut b = spawn_node("smoke-b", port_b);

    let (tx, rx) = mpsc::channel();
    for child in [&mut a, &mut b] {
        let stdout = child.stdout.take().unwrap();
        let tx = tx.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if line.contains("mdns discovered peer") {
                    let _ = tx.send(());
                    break;
                }
            }
        });
    }

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut discoveries = 0;
    while Instant::now() < deadline && discoveries < 2 {
        if rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .is_ok()
        {
            discoveries += 1;
        }
    }
    let _ = a.kill();
    let _ = b.kill();
    let _ = a.wait();
    let _ = b.wait();

    assert!(
        discoveries >= 2,
        "expected both nodes to log mdns discovery, got {discoveries}"
    );
}
