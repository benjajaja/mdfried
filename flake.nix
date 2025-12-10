{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, crane, flake-utils, rust-overlay, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };
        inherit (pkgs) lib;

        craneLib = (crane.mkLib pkgs).overrideToolchain (p:
          p.rust-bin.stable.latest.default
        );

        unfilteredRoot = ./.;
        src = lib.fileset.toSource {
          root = unfilteredRoot;
          fileset = lib.fileset.unions [
            (craneLib.fileset.commonCargoSources unfilteredRoot)
            (lib.fileset.maybeMissing ./assets)
            (lib.fileset.maybeMissing ./src/snapshots)
          ];
        };

        commonArgs = {
          inherit src;
          strictDeps = true;

          buildInputs = lib.optionals pkgs.stdenv.isDarwin [
            pkgs.libiconv
          ];
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        mdfried = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
        });

        # Windows cross-compilation (only on Linux)
        pkgsWindows = import nixpkgs {
          overlays = [ (import rust-overlay) ];
          localSystem = system;
          crossSystem = {
            config = "x86_64-w64-mingw32";
          };
        };

        craneLibWindows = (crane.mkLib pkgsWindows).overrideToolchain (p:
          p.rust-bin.stable.latest.default.override {
            targets = [ "x86_64-pc-windows-gnu" ];
          }
        );

        mdfriedWindows = craneLibWindows.buildPackage {
          inherit src;
          strictDeps = true;
          doCheck = false;
        };

        # LLVM coverage toolchain
        craneLibLLvmTools = (crane.mkLib pkgs).overrideToolchain (p:
          p.rust-bin.stable.latest.default.override {
            extensions = [ "llvm-tools" ];
          }
        );
      in
      {
        checks = {
          inherit mdfried;

          mdfried-clippy = craneLib.cargoClippy (commonArgs // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "--all-targets -- --deny warnings";
          });

          mdfried-doc = craneLib.cargoDoc (commonArgs // {
            inherit cargoArtifacts;
          });

          mdfried-fmt = craneLib.cargoFmt {
            inherit src;
          };

          mdfried-nextest = craneLib.cargoNextest (commonArgs // {
            inherit cargoArtifacts;
            partitions = 1;
            partitionType = "count";
          });
        };

        packages = {
          default = mdfried;
        } // lib.optionalAttrs pkgs.stdenv.isLinux {
          windows = mdfriedWindows;
          mdfried-llvm-coverage = craneLibLLvmTools.cargoLlvmCov (commonArgs // {
            inherit cargoArtifacts;
          });
        };

        apps.default = flake-utils.lib.mkApp {
          drv = mdfried;
        };

        devShells.default = craneLib.devShell {
          checks = self.checks.${system};

          packages = with pkgs; [
            cargo-release
            cargo-flamegraph
          ] ++ lib.optionals pkgs.stdenv.isLinux [
            perf
          ];
        };
      });
}
