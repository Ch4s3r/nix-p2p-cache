{ pkgs, lib, config, inputs, ... }:

{
  packages = [ pkgs.git pkgs.pkg-config ];

  languages.rust.enable = true;

  scripts.run.exec = "cargo run --release -- run";
  scripts.keygen.exec = "cargo run --release -- keygen";
  scripts.derive-pubkey.exec = "cargo run --release -- derive-pubkey";

  processes.nix-p2p-cache.exec = "cargo run --release -- run";

  enterShell = ''
    echo "nix-p2p-cache dev shell. Try: devenv up | cargo test | devenv test"
  '';

  enterTest = ''
    cargo build --locked
    cargo test --all
  '';
}
