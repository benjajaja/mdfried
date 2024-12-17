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
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };
      in
      with pkgs;
      {
        packages.defaultPackage = rustPlatform.buildRustPackage {
          pname = "mdfried";
          version = self.shortRev or self.dirtyShortRev;
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = [
            cmake
            file
            freetype
            pkg-config
          ];
          buildInputs = [
            freetype
            expat
            rust-bin.stable.latest.default
          ];
          doCheck = true;
        };

        devShells.default = mkShell {
          nativeBuildInputs = [
            cmake
            file
            freetype
            expat
            pkg-config
          ];
          buildInputs = [
            rust-bin.stable.latest.default
            cargo-tarpaulin
            cargo-watch
          ];
        };

        checks.test = self.packages.${system}.defaultPackage;
      });
}
