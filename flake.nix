{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
    crate2nix.url = "github:nix-community/crate2nix";
    crate2nix.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay, crate2nix }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        package_version = pkgs.lib.removeSuffix "\n" (builtins.readFile ./VERSION);
        package_name = "calibre-metadata-api";

        cargoNix = pkgs.callPackage ./Cargo.nix {};

        package = cargoNix.workspaceMembers.${package_name}.build.override {
          crateOverrides = pkgs.defaultCrateOverrides // {
            ${package_name} = attrs: {
              nativeBuildInputs = (attrs.nativeBuildInputs or []) ++ [ pkgs.tailwindcss ];
              SQLX_OFFLINE = true;
              SQLX_OFFLINE_DIR = ".sqlx";
              # https://github.com/launchbadge/sqlx/issues/1021
              CARGO = "${pkgs.cargo}/bin/cargo";
              CARGO_MANIFEST_DIR = attrs.src;
            };
            calibreweb = attrs: {
              SQLX_OFFLINE = true;
              SQLX_OFFLINE_DIR = "${self}/.sqlx";
              CARGO = "${pkgs.cargo}/bin/cargo";
              CARGO_MANIFEST_DIR = attrs.src;
            };
          };
        };

      in with pkgs; {
        devShells.default = mkShell {
          buildInputs = [
            rust-bin.stable.latest.default
            rust-analyzer
            pkgs.postgresql_17
            pkgs.foreman
            pkgs.tailwindcss
            pkgs.opentelemetry-collector
            pkgs.crate2nix
          ];
        };

        packages.default = package;
        packages.docker = pkgs.dockerTools.buildLayeredImage {
          name = package_name;
          tag = package_version;
          contents = [ package pkgs.cacert ];
          config = {
            Cmd = [ "/bin/cma" ];
          };
        };
      }
    );
}
