{
  inputs = {
    # a better way of using the latest stable version of nixpkgs
    # without specifying specific release
    nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/*.tar.gz";
  };

  outputs = { self, nixpkgs, ... }:
    let
      # helpers for producing system-specific outputs
      supportedSystems = [
        "aarch64-linux"
        "riscv64-linux"
        "x86_64-linux"
      ];
      forEachSupportedSystem = f: nixpkgs.lib.genAttrs supportedSystems (system: f {
        pkgs = import nixpkgs { inherit system; };
      });
    in
    {
      devShells = forEachSupportedSystem ({ pkgs, ... }: {
        default = pkgs.mkShell {
          packages = with pkgs; [
            rustup
            gdb
            pkg-config

            # formatting this flake
            nixpkgs-fmt
          ];
        };
      });

      packages = forEachSupportedSystem ({ pkgs, ... }: let
        manifest = (pkgs.lib.importTOML ./Cargo.toml).package;
      in {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = manifest.name;
          version = manifest.version;
          src = ./.;
          cargoLock = {
            lockFile = ./Cargo.lock;
            outputHashes."wheel-0.15.0" = "sha256-nNn6Rwnc/6ZLzJdgR2to2xPpgo2mKPMs6dtDHzIbPMc=";
          };
        };
      });
    };
}
