flake: { config, lib, pkgs, ... }:
let cfg = config.services.nix-p2p-cache; in
{
  imports = [ (import ./common.nix flake) ];

  config = lib.mkIf cfg.enable {
    networking.firewall = lib.mkIf cfg.openFirewall {
      allowedTCPPorts = [ cfg.port ];
      allowedUDPPorts = [ cfg.port ];
    };

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
      serviceConfig = {
        ExecStart = "${cfg.package}/bin/nix-p2p-cache run --port ${toString cfg.port} --bind ${cfg.bind}";
        User = "nix-p2p-cache";
        Group = "nix-p2p-cache";
        Restart = "on-failure";
        RestartSec = "5s";
        ReadOnlyPaths = [ "/nix/store" "/nix/var/nix/db" ];
      };
    };
  };
}
