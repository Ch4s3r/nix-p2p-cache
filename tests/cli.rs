use assert_cmd::Command;

#[test]
fn derive_pubkey_is_deterministic_for_hostname() {
    let a = Command::cargo_bin("nix-p2p-cache")
        .unwrap()
        .args(["derive-pubkey", "--hostname", "alpha"])
        .output()
        .unwrap();
    assert!(a.status.success());
    let out_a = String::from_utf8(a.stdout).unwrap();
    let b = Command::cargo_bin("nix-p2p-cache")
        .unwrap()
        .args(["derive-pubkey", "--hostname", "alpha"])
        .output()
        .unwrap();
    let out_b = String::from_utf8(b.stdout).unwrap();
    assert_eq!(out_a, out_b);
    assert!(out_a.starts_with("nix-p2p-cache-alpha:"));
}

#[test]
fn derive_pubkey_differs_per_hostname() {
    let a = Command::cargo_bin("nix-p2p-cache")
        .unwrap()
        .args(["derive-pubkey", "--hostname", "one"])
        .output()
        .unwrap();
    let b = Command::cargo_bin("nix-p2p-cache")
        .unwrap()
        .args(["derive-pubkey", "--hostname", "two"])
        .output()
        .unwrap();
    assert_ne!(a.stdout, b.stdout);
}

#[test]
fn keygen_writes_idempotent_files() {
    let dir = tempfile::tempdir().unwrap();
    Command::cargo_bin("nix-p2p-cache")
        .unwrap()
        .args(["keygen", "--hostname", "foo", "--out"])
        .arg(dir.path())
        .assert()
        .success();
    let pub1 = std::fs::read_to_string(dir.path().join("key.pub")).unwrap();
    Command::cargo_bin("nix-p2p-cache")
        .unwrap()
        .args(["keygen", "--hostname", "foo", "--out"])
        .arg(dir.path())
        .assert()
        .success();
    let pub2 = std::fs::read_to_string(dir.path().join("key.pub")).unwrap();
    assert_eq!(pub1, pub2);
    assert!(pub1.starts_with("nix-p2p-cache-foo:"));
}
