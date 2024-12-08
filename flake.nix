{
  description = "mdfried";
  nixConfig.bash-prompt = "\[mdfried\]$ ";

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
      in
      with pkgs;
      {
        packages.default = rustPlatform.buildRustPackage {
          pname = "mdfried";
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
            rust-bin.stable.latest.default
            cargo-tarpaulin
            cargo-watch
          ];
        };
      });
}
