flake: { config, lib, pkgs, ... }:
let
  cfg = config.services.nix-p2p-cache;
  defaultPkg = flake.packages.${pkgs.stdenv.hostPlatform.system}.default;
  hostName = cfg.hostName;
  pubKeyDrv = pkgs.runCommand "nix-p2p-cache-pubkey-${hostName}" { } ''
    ${cfg.package}/bin/nix-p2p-cache derive-pubkey --hostname ${lib.escapeShellArg hostName} > $out
  '';
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
    hostName = lib.mkOption {
      type = lib.types.str;
      default = config.networking.hostName;
      description = "Hostname used to derive the deterministic signing key.";
    };
    keyDir = lib.mkOption {
      type = lib.types.path;
      default = "/var/lib/nix-p2p-cache";
      description = "Where to materialize the derived keypair.";
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
      [ (lib.removeSuffix "\n" (builtins.readFile pubKeyDrv)) ]
      ++ cfg.extraTrustedPublicKeys;
  };
}
