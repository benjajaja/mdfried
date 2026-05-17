{ pkgs, src, mdfriedStatic }:

let
  inherit (pkgs) lib;

  mdfriedCmd = "${mdfriedStatic}/bin/mdfried README.md";

in
pkgs.testers.nixosTest {
  name = "mdfried-screen-recording";

  nodes.machine = { pkgs, ... }: {
    virtualisation.memorySize = 4096;

    programs.sway = {
      enable = true;
      wrapperFeatures.gtk = true;
    };
    environment.etc."sway/config.d/fullscreen.conf".text = ''
      bar { mode invisible }
      default_border none
      default_floating_border none
      gaps inner 0
      gaps outer 0
      for_window [app_id="kitty"] fullscreen enable
    '';

    services.xserver.enable = true;
    services.displayManager.sddm.enable = true;
    services.displayManager.sddm.wayland.enable = true;

    services.displayManager.autoLogin = {
      enable = true;
      user = "test";
    };

    services.displayManager.defaultSession = "sway";

    users.users.test = {
      isNormalUser = true;
      extraGroups = [ "wheel" "video" ];
    };

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

    environment.systemPackages = with pkgs; [
      kitty
      chafa
      wf-recorder
    ];
  };

  testScript = ''
    machine.wait_for_unit("graphical.target")
    machine.wait_until_succeeds("pgrep -f sway")

    # Copy assets
    machine.succeed("mkdir -p /tmp/test-assets")
    machine.copy_from_host("${builtins.path { path = ./README.md; name = "README.md"; }}", "/tmp/test-assets/README.md")

    # Set up mdfried config to skip font setup wizard
    machine.succeed("mkdir -p /home/test/.config/mdfried")
    machine.succeed("echo 'font_family = \"Noto Sans Mono\"' > /home/test/.config/mdfried/config.toml")
    machine.succeed("chown -R test:users /home/test/.config")

    # Wait for Wayland compositor to be ready
    machine.wait_until_succeeds("systemd-run --uid=test --setenv=XDG_RUNTIME_DIR=/run/user/1000 --setenv=WAYLAND_DISPLAY=wayland-1 -- swaymsg -t get_version")

    # Launch kitty fullscreen running mdfried on README.md
    machine.succeed("""
      systemd-run --uid=test \
        --setenv=XDG_RUNTIME_DIR=/run/user/1000 \
        --setenv=WAYLAND_DISPLAY=wayland-1 \
        --setenv=LIBGL_ALWAYS_SOFTWARE=1 \
        --working-directory=/tmp/test-assets \
        -- kitty ${mdfriedCmd} &
    """)

    # Give kitty and mdfried time to start and render
    machine.succeed("sleep 3")

    machine.succeed("sleep 1")

    # Start wf-recorder for 5 seconds, output to /tmp/recording.mp4
    machine.succeed("""
      systemd-run --uid=test \
        --setenv=XDG_RUNTIME_DIR=/run/user/1000 \
        --setenv=WAYLAND_DISPLAY=wayland-1 \
        -- wf-recorder -f /tmp/recording.mp4 --codec libx264 &
    """)

    machine.succeed("sleep 5")

    # Stop the recorder gracefully
    machine.succeed("pkill -SIGINT wf-recorder || true")
    machine.succeed("sleep 1")

    # Copy recording to test output
    machine.copy_from_vm("/tmp/recording.mp4", "recording")
    print("Screen recording saved as result/recording.mp4")
  '';
}

