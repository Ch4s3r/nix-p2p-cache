flake: { config, lib, pkgs, ... }:
let
  cfg = config.services.nix-p2p-cache;
  defaultPkg = flake.packages.${pkgs.stdenv.hostPlatform.system}.default;
  # Shared pubkey baked in. Must match keys::public_key_line() in src/keys.rs
  # (derived from blake3("nix-p2p-cache.shared.v1")). Update both together.
  sharedPublicKey = "nix-p2p-cache-shared:zfv4gOrH/QCQjKhPybhUvhgzM5vj/2zq/F3iplvDbxE=";
in
{
  options.services.nix-p2p-cache = {
    enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Whether to enable the nix-p2p-cache LAN substituter. Importing the module enables it by default; set to false to opt out.";
    };
    package = lib.mkOption {
      type = lib.types.package;
      default = defaultPkg;
      description = "The nix-p2p-cache package to use.";
    };
    port = lib.mkOption {
      type = lib.types.port;
      default = 5555;
      description = "TCP port for HTTP and UDP port for libp2p QUIC.";
    };
    bind = lib.mkOption {
      type = lib.types.str;
      default = "::";
      description = "HTTP bind address (dual-stack v6 by default).";
    };
    openFirewall = lib.mkOption {
      type = lib.types.bool;
      default = true;
    };
    extraSubstituters = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ "https://cache.nixos.org" ];
    };
    extraTrustedPublicKeys = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ "cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY=" ];
    };
  };

  config = lib.mkIf cfg.enable {
    nix.settings.substituters =
      [ "http://127.0.0.1:${toString cfg.port}" ] ++ cfg.extraSubstituters;
    nix.settings.trusted-public-keys =
      [ sharedPublicKey ] ++ cfg.extraTrustedPublicKeys;
  };
}
