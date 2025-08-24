{

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };
  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem
      (system:
        let
          overlays = [ (import rust-overlay) ];
          pkgs = import nixpkgs {
            inherit system overlays;
          };
        in
        with pkgs;
        {
          devShells.default = mkShell {
            buildInputs = [
                (rust-bin.stable.latest.default.override {
                    extensions = [ "rust-src" "rustfmt" ];
                })
                rust-analyzer
            ] ++
              pkgs.lib.optionals pkgs.stdenv.isDarwin [
                darwin.apple_sdk.frameworks.Security # Should only be for darwin
                darwin.apple_sdk.frameworks.SystemConfiguration
            ]
            ;
            shellHook = ''
              export DATABASE_URL=sqlite://metadata.db
            '';
          };

        packages.default = nixpkgs.legacyPackages.${system}.callPackage ./default.nix {};
        }
      );
}
