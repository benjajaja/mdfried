{ pkgs, src, mdfriedStatic }:

let
  inherit (pkgs) lib;
  screenshotTests = import ./screenshot-tests.nix { inherit pkgs src mdfriedStatic; };
  terminals = [ "foot" "kitty" "wezterm" "alacritty" ];
  screenshots = screenshotTests.screenshots;

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

  baseUrl = "https://benjajaja.github.io/mdfried-screenshots/images";
  referenceScreenshots = pkgs.runCommand "reference-screenshots" {} ''
    mkdir -p $out/images
    ${lib.concatMapStringsSep "\n" (terminal: ''
      cp ${builtins.fetchurl "${baseUrl}/screenshot-${terminal}.png"} \
        $out/images/screenshot-${terminal}.png
    '') terminals}
  '';

in pkgs.runCommand "mdfried-screenshot-diffs" {
  nativeBuildInputs = [ difyBin ];
} ''
  mkdir -p $out/images $out/diffs

  ${lib.concatMapStringsSep "\n" (terminal: ''
    cp ${screenshots}/images/screenshot-${terminal}.png $out/images/screenshot-${terminal}.png
  '') terminals}

  ${lib.concatMapStringsSep "\n" (terminal: ''
    if [ -f "${referenceScreenshots}/images/screenshot-${terminal}.png" ]; then
      cp "${referenceScreenshots}/images/screenshot-${terminal}.png" "$out/images/reference-${terminal}.png"
    fi
  '') terminals}

  ${lib.concatMapStringsSep "\n" (terminal: ''
    if [ -f "${referenceScreenshots}/images/screenshot-${terminal}.png" ]; then
      echo "Creating diff for ${terminal}..."
      dify "${referenceScreenshots}/images/screenshot-${terminal}.png" "$out/images/screenshot-${terminal}.png" || true
      if [ -f "diff.png" ]; then
        mv diff.png "$out/diffs/diff-${terminal}.png"
      fi
    else
      echo "No reference screenshot for ${terminal}, skipping diff"
    fi
  '') terminals}

  cat > $out/body.html << 'HTMLEOF'
<h2>Screenshots, Master References, and Diffs</h2>
HTMLEOF

  ${lib.concatMapStringsSep "\n" (terminal: ''
    cat >> $out/body.html << TERMEOF
<h3>${terminal}</h3>
<p><strong>New:</strong></p>
<img src="IMAGE_BASE_URL/images/screenshot-${terminal}.png" alt="${terminal} current">
TERMEOF

    if [ -f "$out/images/reference-${terminal}.png" ]; then
      cat >> $out/body.html << TERMEOF
<p><strong>Master:</strong></p>
<img src="IMAGE_BASE_URL/images/reference-${terminal}.png" alt="${terminal} reference">
TERMEOF
    fi

    if [ -f "$out/diffs/diff-${terminal}.png" ]; then
      cat >> $out/body.html << TERMEOF
<p><strong>Diff:</strong></p>
<img src="IMAGE_BASE_URL/diffs/diff-${terminal}.png" alt="${terminal} diff">
TERMEOF
    fi
  '') terminals}

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

  ${lib.concatMapStringsSep "\n" (terminal: ''
    cat >> $out/index.html << TERMEOF
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
''
