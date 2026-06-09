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

  outputs =
    {
      self,
      nixpkgs,
      crane,
      flake-utils,
      rust-overlay,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };
        inherit (pkgs) lib;

        rustToolchainFile = builtins.fromTOML (builtins.readFile ./rust-toolchain.toml);
        rustVersion = rustToolchainFile.toolchain.channel;

        rustBase = p: p.rust-bin.stable.${rustVersion}.default;
        withTargets = targets: p:
          (rustBase p).override { inherit targets; };
        withExtensions = extensions: p:
          (rustBase p).override { inherit extensions; };

        craneLib = (crane.mkLib pkgs).overrideToolchain rustBase;

        unfilteredRoot = ./.;
        src = lib.fileset.toSource {
          root = unfilteredRoot;
          fileset = lib.fileset.unions [
            (craneLib.fileset.commonCargoSources unfilteredRoot)
            (lib.fileset.maybeMissing ./assets)
            (lib.fileset.maybeMissing ./src/fonts)
            (lib.fileset.maybeMissing ./src/snapshots)
            (lib.fileset.maybeMissing ./src/worker/snapshots)
            (lib.fileset.maybeMissing ./mdfrier/src/snapshots)
          ];
        };

        # Common args for default builds using chafa-dyn (dynamic linking)
        commonArgs = {
          inherit src;
          strictDeps = true;
          # Prevent updateAutotoolsGnuConfigScripts from modifying mupdf's vendored
          # autotools files — doing so invalidates cargo's fingerprint for mupdf-sys
          # and causes a rebuild that fails on read-only cargoArtifacts files.
          updateAutotoolsGnuConfigScriptsPhase = "true";

          nativeBuildInputs = with pkgs; [
            makeWrapper
            pkg-config
            rustPlatform.bindgenHook # for mupdf-sys bindgen
            gperf # for mupdf vendored Makefile
            python3 # for mupdf vendored Makefile
            unzip # for mupdf vendored docx_template build
            # mupdf-sys cp_r copies files from the read-only Nix store, preserving
            # mode 444. make then fails to regenerate headers. Wrap make to chmod first.
            (writeShellScriptBin "make" ''
              chmod -R u+w . 2>/dev/null || true
              exec ${gnumake}/bin/make "$@"
            '')
          ];

          buildInputs = [
            pkgs.chafa
            pkgs.glib.dev # for glib-2.0.pc (chafa dependency)
            pkgs.fontconfig.dev # for font-kit (mupdf dep)
          ]
          ++ lib.optionals pkgs.stdenv.isDarwin [
            pkgs.libiconv
          ];
          # mupdf's vendored zlib defines fdopen(fd,mode) as NULL when TARGET_OS_MAC
          # is set. macOS stdio.h then fails to declare fdopen as a function (parse
          # errors). Undefine TARGET_OS_MAC so zlib skips that macro.
          CFLAGS_aarch64_apple_darwin = "-UTARGET_OS_MAC";
          CXXFLAGS_aarch64_apple_darwin = "-UTARGET_OS_MAC";
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        mdfried = craneLib.buildPackage (
          commonArgs
          // {
            inherit cargoArtifacts;
          }
        );

        # Fully static musl build for portable Linux binaries
        mdfriedStatic =
          let
            craneLibMusl = (crane.mkLib pkgs).overrideToolchain 
            (withTargets [ "x86_64-unknown-linux-musl" ]);
            muslPkgs = pkgs.pkgsCross.musl64.pkgsStatic;
            chafaMuslStatic =
              (muslPkgs.chafa.override {
                libavif = null;
                libjxl = null;
                librsvg = null;
              }).overrideAttrs
                (old: {
                  configureFlags = (old.configureFlags or [ ]) ++ [
                    "--enable-static"
                    "--disable-shared"
                    "--without-avif"
                    "--without-jxl"
                    "--without-svg"
                    "--without-tools"
                  ];
                });
            glibMuslStatic = muslPkgs.glib;
            staticArgs = {
              inherit src;
              strictDeps = true;
              doCheck = false;
              cargoExtraArgs = "--no-default-features --features chafa-static,svg,mermaid,pdf";
              CARGO_BUILD_TARGET = "x86_64-unknown-linux-musl";
              CARGO_BUILD_RUSTFLAGS = "-C target-feature=+crt-static -C link-arg=-lgcc -C link-arg=-Wl,--start-group -C link-arg=-lbrotlicommon -C link-arg=-lexpat -C link-arg=-lc -C link-arg=-Wl,--end-group";
              CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER = "${pkgs.pkgsCross.musl64.stdenv.cc}/bin/x86_64-unknown-linux-musl-cc";
              nativeBuildInputs = with pkgs; [
                pkgsCross.musl64.stdenv.cc
                pkg-config
                llvmPackages.libclang
                gperf # for mupdf vendored Makefile
                python3 # for mupdf vendored Makefile
                unzip # for mupdf vendored docx_template build
              ];
              buildInputs = [
                chafaMuslStatic
                glibMuslStatic
                muslPkgs.pcre2
                muslPkgs.libffi
                muslPkgs.zlib
                muslPkgs.fontconfig # for font-kit (mupdf dep)
                muslPkgs.expat # fontconfig dependency
              ];
              PKG_CONFIG_PATH = lib.makeSearchPath "lib/pkgconfig" [
                chafaMuslStatic
                glibMuslStatic
                muslPkgs.pcre2
                muslPkgs.libffi
                muslPkgs.zlib
                muslPkgs.fontconfig
                muslPkgs.expat
              ];
              LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
              BINDGEN_EXTRA_CLANG_ARGS = "-isystem ${pkgs.pkgsCross.musl64.musl.dev}/include";
              # Disable fortify source to avoid __snprintf_chk/__memmove_chk (glibc-specific) in tree-sitter and mupdf/harfbuzz
              CC_x86_64_unknown_linux_musl = "${pkgs.pkgsCross.musl64.stdenv.cc}/bin/x86_64-unknown-linux-musl-cc";
              CXX_x86_64_unknown_linux_musl = "${pkgs.pkgsCross.musl64.stdenv.cc}/bin/x86_64-unknown-linux-musl-c++";
              CFLAGS_x86_64_unknown_linux_musl = "-U_FORTIFY_SOURCE -D_FORTIFY_SOURCE=0";
              CXXFLAGS_x86_64_unknown_linux_musl = "-U_FORTIFY_SOURCE -D_FORTIFY_SOURCE=0";
            };
            cargoArtifactsStatic = craneLibMusl.buildDepsOnly staticArgs;
          in
          craneLibMusl.buildPackage (
            staticArgs
            // {
              cargoArtifacts = cargoArtifactsStatic;
            }
          );

        # Windows cross-compilation (only on Linux)
        mdfriedWindows =
          let
            pkgsWindows = import nixpkgs {
              overlays = [ (import rust-overlay) ];
              localSystem = system;
              crossSystem = {
                config = "x86_64-w64-mingw32";
              };
            };
            craneLibWindows = (crane.mkLib pkgsWindows).overrideToolchain 
            (withTargets [ "x86_64-pc-windows-gnu" ]);
          in
          craneLibWindows.buildPackage {
            inherit src;
            strictDeps = true;
            doCheck = false;
            cargoExtraArgs = "--no-default-features";

            nativeBuildInputs = with pkgs; [
              makeWrapper
            ];
          };

        # LLVM coverage toolchain
        craneLibLLvmTools = (crane.mkLib pkgs).overrideToolchain
          (withExtensions [ "llvm-tools" ]);

        # Screenshot tests (only on Linux)
        screenshotTests = if pkgs.stdenv.isLinux then
          import ./nix/screenshot-tests.nix {
            inherit pkgs src;
            mdfriedStatic = mdfriedStatic;
          }
        else {};
      in
      {
        checks = {
          inherit mdfried;

          mdfried-clippy = craneLib.cargoClippy (
            commonArgs
            // {
              inherit cargoArtifacts;
              cargoClippyExtraArgs = "--all-targets -- --deny warnings";
            }
          );

          mdfried-doc = craneLib.cargoDoc (
            commonArgs
            // {
              inherit cargoArtifacts;
            }
          );

          mdfried-fmt = craneLib.cargoFmt {
            inherit src;
          };

          mdfried-nextest = craneLib.cargoNextest (
            commonArgs
            // {
              inherit cargoArtifacts;
              partitions = 1;
              partitionType = "count";
              cargoNextestCommand = "RUST_LOG=debug cargo nextest";
              cargoNextestExtraArgs = "--workspace";
              env = {
                RUST_LOG = "debug";
              };
            }
          );
        };

        packages = {
          default = mdfried;
        }
        // lib.optionalAttrs pkgs.stdenv.isLinux {
          static = mdfriedStatic;
          windows = mdfriedWindows;
          mdfried-llvm-coverage = craneLibLLvmTools.cargoLlvmCov (
            commonArgs
            // {
              inherit cargoArtifacts;
            }
          );
        }
        // screenshotTests
        // { screen-recording = (import ./nix/screen-recording.nix { inherit pkgs src mdfriedStatic; }).driver; }
        // { screenshot = (import ./nix/screenshot.nix { inherit pkgs src mdfriedStatic; }).driver; };

        apps.default = flake-utils.lib.mkApp {
          drv = mdfried;
        };

        devShells.default = craneLib.devShell {
          checks = self.checks.${system};

          packages =
            let
              screenshotDiffsScript = pkgs.writeShellScriptBin "screenshotDiffs" ''
                set -e

                echo "Building current screenshots sequentially..."
                for terminal in foot kitty wezterm alacritty; do
                  echo "  Building screenshot-test-$terminal..."
                  nix build ".#screenshot-test-$terminal" --print-build-logs
                done

                echo "Building diffs (impure, against benjajaja.github.io/mdfried-screenshots/images/screenshot-<terminal>.png)..."
                nix build .#screenshotDiffs --impure --print-build-logs

                URL="file://$(readlink -f result)/index.html"
                echo ""
                printf '\e]8;;%s\e\\%s\e]8;;\e\\\n' "$URL" "Click here to open in browser"
              '';
            in
            with pkgs;
            [
              nixfmt
              cargo-release
              cargo-semver-checks
              cargo-flamegraph
              chafa
              glib.dev # for glib-2.0.pc (chafa dependency)
              cargo-insta
              nodePackages."@mermaid-js/mermaid-cli"
              screenshotDiffsScript
              gperf # for mupdf-sys vendored build
              python3 # for mupdf-sys vendored build
            ]
            ++ lib.optionals pkgs.stdenv.isLinux [
              perf
            ];
          buildInputs = [ pkgs.fontconfig.dev ];
          LD_LIBRARY_PATH = lib.makeLibraryPath [ pkgs.chafa ];
          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
          BINDGEN_EXTRA_CLANG_ARGS = lib.optionalString pkgs.stdenv.isLinux "-isystem ${pkgs.glibc.dev}/include";
        };

        screenshotDiffs = import ./nix/screenshot-diffs.nix { inherit pkgs src mdfriedStatic; };
      }
    );
}
