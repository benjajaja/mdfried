{
  description = "mdcooked";
  nixConfig.bash-prompt = "\[mdcooked\]$ ";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        # We only need the nightly overlay in the devShell because .rs files are formatted with nightly.
        overlays = [];
        pkgs = import nixpkgs { inherit system overlays; };
        rustNightly = pkgs.rust-bin.nightly."2024-08-27".default;
      in
      with pkgs;
      {
        packages.default = rustPlatform.buildRustPackage {
          pname = "mdcooked";
          version = self.shortRev or self.dirtyShortRev;
          src = ./.;
          cargoLock = {
            lockFile = ./Cargo.lock;
          };
            (with darwin.apple_sdk.frameworks; [ AppKit Security Cocoa]);
        };

        devShell = mkShell {
          buildInputs = [
            (rustNightly.override {
              extensions = [ "rust-src" "rust-analyzer-preview" "rustfmt" "clippy" ];
            })
            cargo-tarpaulin
            cargo-watch
          ];
        };
      });
}
