flake: { config, lib, pkgs, ... }:
let cfg = config.services.nix-p2p-cache; in
{
  imports = [ (import ./common.nix flake) ];

  config = lib.mkIf cfg.enable {
    system.activationScripts.nix-p2p-cache-keygen.text = ''
      mkdir -p ${cfg.keyDir}
      ${cfg.package}/bin/nix-p2p-cache keygen --hostname ${lib.escapeShellArg cfg.hostName} --out ${cfg.keyDir}
    '';

    launchd.daemons.nix-p2p-cache = {
      script = "exec ${cfg.package}/bin/nix-p2p-cache run --port ${toString cfg.port} --bind ${cfg.bind} --hostname ${lib.escapeShellArg cfg.hostName}";
      serviceConfig = {
        RunAtLoad = true;
        KeepAlive = true;
        StandardOutPath = "/var/log/nix-p2p-cache.log";
        StandardErrorPath = "/var/log/nix-p2p-cache.log";
      };
    };
  };
}
