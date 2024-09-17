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
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };
        #rustNightly = pkgs.rust-bin.stable;
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
        };

        devShell = mkShell {
          nativeBuildInputs = [
            freetype expat pkg-config
          ];
          buildInputs = [
            #(rustNightly.override {
              #extensions = [ "rust-src" "rust-analyzer-preview" "rustfmt" "clippy" ];
            #})
            rust-bin.stable.latest.default
            cargo-tarpaulin
            cargo-watch
            #freetype
            #expat
            #fontconfig
          ];
        };
      });
}
