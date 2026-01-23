{ pkgs, src, mdfriedStatic }:

let
  inherit (pkgs) lib;

  referenceScreenshotsPath = builtins.getEnv "REFERENCE";
  # Optional cache key - if set, appends to derivation names to bust cache
  # e.g. SCREENSHOT_CACHE_KEY=${{ github.run_id }} for per-run fresh builds
  cacheKey = builtins.getEnv "SCREENSHOT_CACHE_KEY";
  cacheSuffix = lib.optionalString (cacheKey != "") "-${cacheKey}";

  makeScreenshotTest = { terminal, terminalCommand, terminalPackages, setup ? null, xwayland ? false }: pkgs.testers.nixosTest {
    name = "mdfried-test-wayland-${terminal}${cacheSuffix}";

    nodes.machine = { pkgs, ... }: {
      virtualisation.memorySize = 4096;

      programs.sway = {
        enable = true;
        wrapperFeatures.gtk = true;
      };

      programs.xwayland.enable = xwayland;

      services.xserver.enable = true;
      services.displayManager.sddm.enable = true;
      services.displayManager.sddm.wayland.enable = true;

      services.displayManager.autoLogin = {
        enable = true;
        user = "test";
      };

      services.displayManager.defaultSession = "sway";

      # Create test user
      users.users.test = {
        isNormalUser = true;
        extraGroups = [ "wheel" "video" ];
        packages = [ ];
      };

      # Fonts for proper Unicode rendering
      fonts.packages = with pkgs; [
        unifont
        noto-fonts
        noto-fonts-lgc-plus
        noto-fonts-cjk-sans
        noto-fonts-color-emoji
        dejavu_fonts
        freefont_ttf
        fira-code
        fira-mono
      ];

      # Ensure required packages are available
      environment.systemPackages = with pkgs;
        terminalPackages ++ [ chafa ];
    };

    testScript = ''
      machine.wait_for_unit("graphical.target")

      machine.wait_until_succeeds("pgrep -f sway")

      machine.succeed("mkdir -p /tmp/test-assets")
      machine.copy_from_host("${src}/assets/screenshot-test.md", "/tmp/test-assets/screenshot-test.md")
      machine.copy_from_host("${src}/assets/NixOS.png", "/tmp/test-assets/NixOS.png")

      # Create mdfried config to skip font setup wizard
      machine.succeed("mkdir -p /home/test/.config/mdfried")
      machine.succeed("echo 'font_family = \"Noto Sans Mono\"' > /home/test/.config/mdfried/config.toml")
      machine.succeed("chown -R test:users /home/test/.config")

      machine.wait_until_succeeds("systemd-run --uid=test --setenv=XDG_RUNTIME_DIR=/run/user/1000 --setenv=WAYLAND_DISPLAY=wayland-1 -- swaymsg -t get_version")

      machine.succeed("${if setup != null then setup else "true"}")

      # Use systemd-run to ensure proper environment
      machine.succeed("""
        systemd-run --uid=test --setenv=XDG_RUNTIME_DIR=/run/user/1000 \
          --setenv=WAYLAND_DISPLAY=wayland-1 \
          --setenv=LIBGL_ALWAYS_SOFTWARE=1 \
          --setenv=QT_QPA_PLATFORM="wayland" \
          --setenv=RUST_BACKTRACE=1 \
          ${if xwayland then "--setenv=DISPLAY=:0" else ""} \
          --working-directory=/tmp/test-assets \
          -- ${terminalCommand}
      """)

      # Wait for mdfried to render (images, headers, etc.)
      machine.succeed("sleep 10")
      machine.screenshot("screenshot-${terminal}")
      print("Screenshot saved to test output directory as screenshot-${terminal}.png")
    '';
  };

  # mdfried command to view the test markdown file
  mdfriedCmd = "${mdfriedStatic}/bin/mdfried screenshot-test.md";

  screenshotTests = {
    screenshot-test-foot = makeScreenshotTest {
      terminal = "foot";
      terminalCommand = "foot ${mdfriedCmd}";
      terminalPackages = [ pkgs.foot ];
    };

    screenshot-test-kitty = makeScreenshotTest {
      terminal = "kitty";
      terminalCommand = "kitty ${mdfriedCmd}";
      terminalPackages = [ pkgs.kitty ];
    };

    screenshot-test-wezterm = makeScreenshotTest {
      terminal = "wezterm";
      terminalCommand = "wezterm start --always-new-process --cwd /tmp/test-assets -- ${mdfriedCmd}";
      terminalPackages = [ pkgs.wezterm ];
    };

    screenshot-test-alacritty = makeScreenshotTest {
      terminal = "alacritty";
      terminalCommand = "alacritty -e ${mdfriedCmd}";
      terminalPackages = [ pkgs.alacritty ];
    };
  };

  terminals = map (name: lib.removePrefix "screenshot-test-" name) (builtins.attrNames screenshotTests);

  screenshots = pkgs.runCommand "mdfried-screenshots${cacheSuffix}" {
    buildInputs = builtins.attrValues screenshotTests;
  } ''
    mkdir -p $out/images

    # Copy all screenshots from individual test results
    ${lib.concatMapStringsSep "\n" (terminal: ''
      cp ${screenshotTests."screenshot-test-${terminal}"}/screenshot-${terminal}.png $out/images/screenshot-${terminal}.png
    '') terminals}

    # Generate index.html
    cat > $out/index.html << 'HTMLEOF'
    <!DOCTYPE html>
    <html>
    <head>
      <title>Screenshots</title>
      <style>
        body { font-family: -apple-system, BlinkMacSystemFont, sans-serif; margin: 2rem; }
        .terminal { margin: 2rem 0; }
        img { max-width: 100%; border: 1px solid #ddd; border-radius: 8px; }
        h1 { text-align: center; }
        .container { max-width: 1200px; margin: 0 auto; }
      </style>
    </head>
    <body>
      <div class="container">
        <h1>mdfried Terminal Screenshots</h1>
        <p style="text-align: center; color: #666;">Screenshots from various terminal emulators</p>
    HTMLEOF

    # Add each terminal section
    ${lib.concatMapStringsSep "\n" (terminal: ''
      cat >> $out/index.html << 'TERMEOF'
        <div class="terminal">
          <h2 id="${terminal}">${terminal}</h2>
          <img src="images/screenshot-${terminal}.png" alt="${terminal} screenshot">
        </div>
    TERMEOF
    '') terminals}

    cat >> $out/index.html << 'HTMLEOF'
      </div>
    </body>
    </html>
    HTMLEOF
  '';

  # Dify binary for image diffing
  dify = pkgs.fetchurl {
    url = "https://github.com/jihchi/dify/releases/download/v0.7.4/dify-x86_64-unknown-linux-gnu-v0.7.4.tar.gz";
    sha256 = "1cc3pzrn1bn88r72957jwznkzjlkblpjylvsv44vnqszragk1f8c";
  };

  difyBin = pkgs.stdenv.mkDerivation {
    name = "dify";
    src = dify;
    nativeBuildInputs = [ pkgs.autoPatchelfHook pkgs.gnutar pkgs.gzip ];
    buildInputs = [ pkgs.stdenv.cc.cc.lib ];
    sourceRoot = ".";
    unpackPhase = ''
      tar -xzf $src
    '';
    installPhase = ''
      mkdir -p $out/bin
      cp dify $out/bin/
      chmod +x $out/bin/dify
    '';
  };

  # Reference screenshots from master branch
  # Pass via environment variable: REFERENCE=$(nix build "git+file://$PWD?ref=master#screenshots" --print-out-paths --impure)
  # Then: nix build .#screenshotDiffs --impure
  hasReference = referenceScreenshotsPath != "";
  referenceScreenshots = if hasReference then /. + referenceScreenshotsPath else null;

  # Screenshot diffs comparing current screenshots against reference (master)
  screenshotDiffs = pkgs.runCommand "mdfried-screenshot-diffs" {
    nativeBuildInputs = [ difyBin ];
  } ''
    ${lib.optionalString (!hasReference) ''
      echo "ERROR: REFERENCE environment variable not set."
      echo "Run: REFERENCE=\$(nix build \"git+file://\$PWD?ref=master#screenshots\" --print-out-paths --impure) nix build .#screenshotDiffs --impure"
      exit 1
    ''}

    mkdir -p $out/images $out/diffs

    # Copy current screenshots
    ${lib.concatMapStringsSep "\n" (terminal: ''
      cp ${screenshots}/images/screenshot-${terminal}.png $out/images/screenshot-${terminal}.png
    '') terminals}

    # Copy reference screenshots
    ${lib.optionalString hasReference (lib.concatMapStringsSep "\n" (terminal: ''
      if [ -f "${referenceScreenshots}/images/screenshot-${terminal}.png" ]; then
        cp "${referenceScreenshots}/images/screenshot-${terminal}.png" "$out/images/reference-${terminal}.png"
      fi
    '') terminals)}

    # Create diffs against reference screenshots
    ${lib.optionalString hasReference (lib.concatMapStringsSep "\n" (terminal: ''
      if [ -f "${referenceScreenshots}/images/screenshot-${terminal}.png" ]; then
        echo "Creating diff for ${terminal}..."
        dify "${referenceScreenshots}/images/screenshot-${terminal}.png" "$out/images/screenshot-${terminal}.png" || true
        if [ -f "diff.png" ]; then
          mv diff.png "$out/diffs/diff-${terminal}.png"
        fi
      else
        echo "No reference screenshot for ${terminal}, skipping diff"
      fi
    '') terminals)}

    # Generate body.html (shared content, uses IMAGE_BASE_URL placeholder)
    cat > $out/body.html << 'HTMLEOF'
<h2>Screenshot Diffs vs Master</h2>
<p><a href="IMAGE_BASE_URL/index.html">View full comparison</a></p>
HTMLEOF

    ${lib.concatMapStringsSep "\n" (terminal: ''
      has_diff=""
      if [ -f "$out/diffs/diff-${terminal}.png" ]; then
        has_diff="yes"
      fi

      cat >> $out/body.html << TERMEOF
<h3>${terminal}</h3>
<p><strong>Current:</strong></p>
<img src="IMAGE_BASE_URL/images/screenshot-${terminal}.png" alt="${terminal} current">
TERMEOF

      if [ -f "$out/images/reference-${terminal}.png" ]; then
        cat >> $out/body.html << TERMEOF
<p><strong>Reference (master):</strong></p>
<img src="IMAGE_BASE_URL/images/reference-${terminal}.png" alt="${terminal} reference">
TERMEOF
      fi

      if [ -n "$has_diff" ]; then
        cat >> $out/body.html << TERMEOF
<p><strong>Diff:</strong></p>
<img src="IMAGE_BASE_URL/diffs/diff-${terminal}.png" alt="${terminal} diff">
TERMEOF
      fi
    '') terminals}

    # Generate index.html (body.html with relative URLs wrapped in html structure)
    cat > $out/index.html << 'HTMLEOF'
<!DOCTYPE html>
<html>
<head><title>Screenshot Diffs</title></head>
<body>
HTMLEOF
    sed 's|IMAGE_BASE_URL/||g' $out/body.html >> $out/index.html
    cat >> $out/index.html << 'HTMLEOF'
</body>
</html>
HTMLEOF
  '';

in
screenshotTests // { inherit screenshots screenshotDiffs; }
