{ pkgs, src, mdfriedStatic }:

let
  inherit (pkgs) lib;

  mdfriedCmd = "${mdfriedStatic}/bin/mdfried README.md --animate";

in
pkgs.testers.nixosTest {
  name = "mdfried-screen-recording";

  nodes.machine = { pkgs, ... }: {
    virtualisation.memorySize = 4096;
    virtualisation.qemu.options = [ "-device virtio-vga" ];
    virtualisation.graphics = true;

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
      seat * hide_cursor 1
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
    machine.copy_from_host("${builtins.path { path = ./assets/logo.png; name = "logo.png"; }}", "/tmp/test-assets/assets/logo.png")
    machine.copy_from_host("${builtins.path { path = ./assets/screenshot_1.png; name = "screenshot_1.png"; }}", "/tmp/test-assets/assets/screenshot_1.png")

    # Wait for Wayland compositor to be ready
    machine.wait_until_succeeds("systemd-run --uid=test --setenv=XDG_RUNTIME_DIR=/run/user/1000 --setenv=WAYLAND_DISPLAY=wayland-1 -- swaymsg -t get_version")

    machine.succeed("""
      systemd-run --uid=test \
        --setenv=XDG_RUNTIME_DIR=/run/user/1000 \
        --setenv=WAYLAND_DISPLAY=wayland-1 \
        --setenv=LIBGL_ALWAYS_SOFTWARE=1 \
        --working-directory=/tmp/test-assets \
        -- kitty ${mdfriedCmd} &
    """)

    # Give kitty and mdfried time to start and render
    machine.succeed("sleep 5")

    machine.succeed("""
      systemd-run --uid=test \
        --setenv=XDG_RUNTIME_DIR=/run/user/1000 \
        --setenv=WAYLAND_DISPLAY=wayland-1 \
        -- wf-recorder -f /tmp/recording.mp4 --codec libx264 --pixel-format yuv420p -r 30 -p crf=20 -p preset=medium &
    """)
    machine.succeed("sleep 1")

    machine.succeed("kill -USR1 $(pgrep mdfried)")

    machine.succeed("sleep 4")

    # Stop the recorder gracefully
    machine.succeed("pkill -SIGINT wf-recorder || true")
    machine.succeed("sleep 1")

    machine.copy_from_vm("/tmp/recording.mp4", "recording")
    print("Screen recording saved as result/recording.mp4")
  '';
}

