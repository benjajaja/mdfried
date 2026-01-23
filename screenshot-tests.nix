{ pkgs, src, mdfriedStatic, }:

let
  inherit (pkgs) lib;

  makeScreenshotTest = { terminal, terminalCommand, terminalPackages, setup ? null, xwayland ? false }: pkgs.testers.nixosTest {
    name = "mdfried-test-wayland-${terminal}";

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

  screenshots = pkgs.runCommand "mdfried-screenshots" {
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

in
screenshotTests // { inherit screenshots; }
