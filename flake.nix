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

        # Build chafa with static library support (uses autotools)
        chafaStatic = pkgs.chafa.overrideAttrs (old: {
          configureFlags = (old.configureFlags or []) ++ [
            "--enable-static"
            "--enable-shared"
          ];
        });

        # We also need static glib for full static linking (uses meson)
        glibStatic = pkgs.glib.overrideAttrs (old: {
          mesonFlags = (old.mesonFlags or []) ++ [
            "-Ddefault_library=both"
          ];
        });


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
          nativeBuildInputs = with pkgs; [
            makeWrapper
            pkg-config
            llvmPackages.libclang
          ];
          buildInputs = with pkgs; [
            chafaStatic
            chafaStatic.dev
            glibStatic.dev
            libsysprof-capture
            pcre2.dev
            libffi.dev
            zlib.dev
          ];
          cargoExtraArgs = "--no-default-features --features chafa-static";
          # Environment for pkg-config and bindgen
          PKG_CONFIG_PATH = "${chafaStatic.dev}/lib/pkgconfig:${glibStatic.dev}/lib/pkgconfig:${pkgs.libsysprof-capture}/lib/pkgconfig:${pkgs.pcre2.dev}/lib/pkgconfig:${pkgs.libffi.dev}/lib/pkgconfig:${pkgs.zlib.dev}/lib/pkgconfig";
          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
          BINDGEN_EXTRA_CLANG_ARGS = "-isystem ${pkgs.glibc.dev}/include";
        });

        # Fully static musl build for portable Linux binaries
        craneLibMusl = (crane.mkLib pkgs).overrideToolchain (p:
          p.rust-bin.stable.latest.default.override {
            targets = [ "x86_64-unknown-linux-musl" ];
          }
        );

        muslPkgs = pkgs.pkgsCross.musl64.pkgsStatic;

        # Build chafa for musl without problematic deps
        chafaMuslStatic = (muslPkgs.chafa.override {
          libavif = null;
          libjxl = null;
          librsvg = null;
        }).overrideAttrs (old: {
          configureFlags = (old.configureFlags or []) ++ [
            "--enable-static"
            "--disable-shared"
            "--without-avif"
            "--without-jxl"
            "--without-svg"
            "--without-tools"
          ];
        });

        glibMuslStatic = muslPkgs.glib;

        mdfriedStatic = craneLibMusl.buildPackage {
          inherit src;
          strictDeps = true;
          doCheck = false;
          CARGO_BUILD_TARGET = "x86_64-unknown-linux-musl";
          CARGO_BUILD_RUSTFLAGS = "-C target-feature=+crt-static -C link-arg=-lgcc";
          CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER = "${pkgs.pkgsCross.musl64.stdenv.cc}/bin/x86_64-unknown-linux-musl-cc";
          nativeBuildInputs = with pkgs; [
            pkgsCross.musl64.stdenv.cc
            pkg-config
            llvmPackages.libclang
          ];
          buildInputs = [
            chafaMuslStatic
            glibMuslStatic
            muslPkgs.pcre2
            muslPkgs.libffi
            muslPkgs.zlib
          ];
          cargoExtraArgs = "--no-default-features --features chafa-static";
          PKG_CONFIG_PATH = lib.makeSearchPath "lib/pkgconfig" [
            chafaMuslStatic
            glibMuslStatic
            muslPkgs.pcre2
            muslPkgs.libffi
            muslPkgs.zlib
          ];
          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
          BINDGEN_EXTRA_CLANG_ARGS = "-isystem ${pkgs.pkgsCross.musl64.musl.dev}/include";
        };

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
          static = mdfriedStatic;
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
            chafa
          ] ++ lib.optionals pkgs.stdenv.isLinux [
            perf
          ];
          LD_LIBRARY_PATH = lib.makeLibraryPath [ pkgs.chafa ];
        };
      });
}
