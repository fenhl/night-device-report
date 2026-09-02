{
    inputs.flake.url = "github:fenhl/flake";
    outputs = attrs: attrs.flake.lib {
        packages.default = { pkgs, ... }: let
            manifest = (pkgs.lib.importTOML ./Cargo.toml).package;
        in pkgs.rustPlatform.buildRustPackage {
            pname = "night-device-report";
            inherit (manifest) version;
            cargoLock = {
                allowBuiltinFetchGit = true; # allows omitting cargoLock.outputHashes
                lockFile = ./Cargo.lock;
            };
            src = ./.;
        };
    };
}
