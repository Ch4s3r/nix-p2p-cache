{
  description = "P2P LAN substituter for /nix/store";

  inputs = {
    nixpkgs.url = "github:cachix/devenv-nixpkgs/rolling";
    systems.url = "github:nix-systems/default";
    devenv.url = "github:cachix/devenv";
    devenv.inputs.nixpkgs.follows = "nixpkgs";
  };

  nixConfig = {
    extra-trusted-public-keys = "devenv.cachix.org-1:w1cLUi8dv3hnoSPGAuibQv+f9TZLr6cv/Hm9XgU50cw=";
    extra-substituters = "https://devenv.cachix.org";
  };

  outputs = { self, nixpkgs, devenv, systems, ... } @ inputs:
    let
      forEachSystem = nixpkgs.lib.genAttrs (import systems);
      mkPackage = pkgs:
        pkgs.rustPlatform.buildRustPackage {
          pname = "nix-p2p-cache";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = [ pkgs.pkg-config ];
          doCheck = false;
        };
    in
    {
      packages = forEachSystem (system:
        let pkgs = nixpkgs.legacyPackages.${system}; in {
          default = mkPackage pkgs;
          nix-p2p-cache = mkPackage pkgs;
        });

      apps = forEachSystem (system: {
        default = {
          type = "app";
          program = "${mkPackage nixpkgs.legacyPackages.${system}}/bin/nix-p2p-cache";
        };
      });

      overlays.default = final: prev: { nix-p2p-cache = mkPackage final; };

      nixosModules.default = import ./modules/nixos.nix self;
      darwinModules.default = import ./modules/darwin.nix self;

      devShells = forEachSystem (system:
        let pkgs = nixpkgs.legacyPackages.${system}; in {
          default = devenv.lib.mkShell {
            inherit inputs pkgs;
            modules = [ ./devenv.nix ];
          };
        });
    };
}
