{
  description = "fluxter - a TUI chat client for the Fluxer messaging platform";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = {
    self,
    nixpkgs,
  }: let
    systems = ["x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin"];
    forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    cargoToml = nixpkgs.lib.importTOML ./Cargo.toml;
  in {
    packages = forAllSystems (pkgs: rec {
      fluxer-tui = pkgs.rustPlatform.buildRustPackage {
        pname = "fluxer-tui";
        version = cargoToml.package.version + (
          if self ? shortRev
          then "-${self.shortRev}"
          else "-dirty"
        );

        src = self;

        cargoLock.lockFile = ./Cargo.lock;

        nativeBuildInputs = [pkgs.makeWrapper];

        # chafa is the text-art fallback when the terminal answers no
        # graphics-protocol query; sixel/kitty/iTerm2 rendering is built in.
        # fluxter-phone carries a voice call's sound (see phone/). ffmpeg,
        # ffplay, mpv, wl-clipboard and xclip are looked up on PATH at
        # runtime, so whatever the desktop already has gets used.
        postInstall = ''
          wrapProgram $out/bin/fluxter --suffix PATH : ${pkgs.lib.makeBinPath [pkgs.chafa phone]}
        '';

        meta = {
          description = "TUI chat client for the Fluxer messaging platform";
          homepage = "https://github.com/AIVirtuoso/fluxter";
          license = pkgs.lib.licenses.gpl3Plus;
          mainProgram = "fluxter";
        };
      };
      # the sound of a voice call, handed the LiveKit grant by the client;
      # a Go program because LiveKit's Go SDK is pure Go (pion), with no
      # libwebrtc and no C++ in the build
      phone = pkgs.buildGoModule {
        pname = "fluxter-phone";
        version = cargoToml.package.version;
        src = ./phone;
        vendorHash = "sha256-iNa8rFksHlxRe2QihkaSW3XmwOlYaKXcyhah+y59wOQ=";
        env.CGO_ENABLED = 0;
        nativeBuildInputs = [pkgs.makeWrapper];
        # go names the binary after the directory. Sharing a screen encodes
        # the portal's PipeWire stream with GStreamer: its launcher, the
        # PipeWire source and the x264 encoder ride along, since nothing
        # else on a system is likely to have exactly those.
        postInstall = let
          gstPlugins = with pkgs; [gst_all_1.gstreamer gst_all_1.gst-plugins-base gst_all_1.gst-plugins-ugly pipewire];
        in ''
          mv $out/bin/phone $out/bin/fluxter-phone
          wrapProgram $out/bin/fluxter-phone \
            --suffix PATH : ${pkgs.lib.makeBinPath [pkgs.gst_all_1.gstreamer]} \
            --suffix GST_PLUGIN_SYSTEM_PATH_1_0 : ${pkgs.lib.makeSearchPathOutput "out" "lib/gstreamer-1.0" gstPlugins}
        '';
        meta = {
          description = "Carries the sound of a Fluxer voice call for fluxter";
          license = pkgs.lib.licenses.gpl3Plus;
          mainProgram = "fluxter-phone";
        };
      };
      default = fluxer-tui;
    });

    # `nix run .#dev` or `nix run github:AIVirtuoso/fluxter/<branch>#dev`:
    # build with cargo in a target directory under the cache directory, so
    # trying a branch recompiles only what changed instead of every
    # dependency, as the sandboxed package build must.
    apps = forAllSystems (pkgs: {
      dev = {
        type = "app";
        program = toString (pkgs.writeShellScript "fluxer-tui-dev" ''
          set -eu
          cache="''${XDG_CACHE_HOME:-$HOME/.cache}/fluxer-tui"
          export CARGO_TARGET_DIR="''${CARGO_TARGET_DIR:-$cache/target}"
          # cargo tells fresh from stale by file dates, and every file in
          # the store is dated 1970, so a new snapshot would never be
          # rebuilt: work from a copy with real dates, one per snapshot
          snap="$cache/src-$(basename ${self} | cut -c1-32)"
          if [ ! -e "$snap/.complete" ]; then
            rm -rf "$cache"/src-*
            mkdir -p "$snap"
            cp -r --no-preserve=mode,timestamps ${self}/. "$snap"/
            touch "$snap/.complete"
          fi
          export PATH="${pkgs.lib.makeBinPath [pkgs.cargo pkgs.rustc pkgs.chafa self.packages.${pkgs.system}.phone]}:$PATH"
          exec cargo run --release --locked --manifest-path "$snap/Cargo.toml" -- "$@"
        '');
      };
    });

    devShells = forAllSystems (pkgs: {
      default = pkgs.mkShell {
        packages = with pkgs; [cargo rustc rustfmt clippy chafa wl-clipboard go];
      };
    });
  };
}
