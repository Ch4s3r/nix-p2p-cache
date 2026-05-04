flake: { config, lib, pkgs, ... }:
let cfg = config.services.nix-p2p-cache; in
{
  imports = [ (import ./common.nix flake) ];

  config = lib.mkIf cfg.enable {
    networking.firewall = lib.mkIf cfg.openFirewall {
      allowedTCPPorts = [ cfg.port ];
      allowedUDPPorts = [ cfg.port ];
    };

    systemd.tmpfiles.rules = [ "d ${cfg.keyDir} 0750 nix-p2p-cache nix-p2p-cache -" ];

    users.groups.nix-p2p-cache = { };
    users.users.nix-p2p-cache = {
      isSystemUser = true;
      group = "nix-p2p-cache";
      description = "nix-p2p-cache daemon";
    };

    systemd.services.nix-p2p-cache = {
      description = "nix-p2p-cache LAN substituter";
      wantedBy = [ "multi-user.target" ];
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];
      preStart = ''
        ${cfg.package}/bin/nix-p2p-cache keygen --hostname ${lib.escapeShellArg cfg.hostName} --out ${cfg.keyDir}
      '';
      serviceConfig = {
        ExecStart = "${cfg.package}/bin/nix-p2p-cache run --port ${toString cfg.port} --bind ${cfg.bind} --hostname ${lib.escapeShellArg cfg.hostName}";
        User = "nix-p2p-cache";
        Group = "nix-p2p-cache";
        Restart = "on-failure";
        RestartSec = "5s";
        ReadOnlyPaths = [ "/nix/store" "/nix/var/nix/db" ];
      };
    };
  };
}
