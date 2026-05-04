flake: { config, lib, pkgs, ... }:
let cfg = config.services.nix-p2p-cache; in
{
  imports = [ (import ./common.nix flake) ];

  config = lib.mkIf cfg.enable {
    launchd.daemons.nix-p2p-cache = {
      script = "exec ${cfg.package}/bin/nix-p2p-cache run --port ${toString cfg.port} --bind ${cfg.bind}";
      serviceConfig = {
        RunAtLoad = true;
        KeepAlive = true;
        StandardOutPath = "/var/log/nix-p2p-cache.log";
        StandardErrorPath = "/var/log/nix-p2p-cache.log";
      };
    };
  };
}
