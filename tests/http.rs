use assert_cmd::cargo::CommandCargoExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn pick_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

fn wait_ready(port: u16) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Ok(resp) = reqwest::blocking::get(format!("http://127.0.0.1:{port}/nix-cache-info"))
        {
            if resp.status().is_success() {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

#[test]
fn serves_nix_cache_info_over_http() {
    let port = pick_port();
    let mut child = Command::cargo_bin("nix-p2p-cache")
        .unwrap()
        .args([
            "run",
            "--port",
            &port.to_string(),
            "--bind",
            "127.0.0.1",
            "--db",
            "/nonexistent.sqlite",
            "--store-dir",
            "/nix/store",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let ok = wait_ready(port);
    let body = if ok {
        reqwest::blocking::get(format!("http://127.0.0.1:{port}/nix-cache-info"))
            .ok()
            .and_then(|r| r.text().ok())
            .unwrap_or_default()
    } else {
        String::new()
    };
    let _ = child.kill();
    let _ = child.wait();
    assert!(ok, "server did not become ready");
    assert!(body.contains("StoreDir: /nix/store"));
    assert!(body.contains("WantMassQuery: 1"));
}

#[test]
fn head_request_for_nix_cache_info_succeeds() {
    let port = pick_port();
    let mut child = Command::cargo_bin("nix-p2p-cache")
        .unwrap()
        .args([
            "run",
            "--port",
            &port.to_string(),
            "--bind",
            "127.0.0.1",
            "--db",
            "/nonexistent.sqlite",
            "--store-dir",
            "/nix/store",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let ok = wait_ready(port);
    let status = if ok {
        let client = reqwest::blocking::Client::new();
        client
            .head(format!("http://127.0.0.1:{port}/nix-cache-info"))
            .send()
            .map(|r| r.status().as_u16())
            .unwrap_or(0)
    } else {
        0
    };
    let _ = child.kill();
    let _ = child.wait();
    assert!(ok);
    assert_eq!(status, 200);
}

#[test]
fn cache_miss_returns_404_when_no_peer_has_path() {
    let port = pick_port();
    let mut child = Command::cargo_bin("nix-p2p-cache")
        .unwrap()
        .args([
            "run",
            "--port",
            &port.to_string(),
            "--bind",
            "127.0.0.1",
            "--db",
            "/nonexistent.sqlite",
            "--store-dir",
            "/nix/store",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let ok = wait_ready(port);
    let status = if ok {
        reqwest::blocking::get(format!(
            "http://127.0.0.1:{port}/{}.narinfo",
            "z".repeat(32)
        ))
        .map(|r| r.status().as_u16())
        .unwrap_or(0)
    } else {
        0
    };
    let _ = child.kill();
    let _ = child.wait();
    assert!(ok);
    assert_eq!(status, 404);
}
