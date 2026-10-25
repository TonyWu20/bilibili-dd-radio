{
  description = "Rust environment";
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    devshell.url = "github:numtide/devshell";
  };
  outputs =
    {
      nixpkgs,
      devshell,
      fenix,
      ...
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-darwin"
      ];

      pkgsFor =
        system:
        import nixpkgs {
          config.allowUnfree = true;
          inherit system;
          overlays = [
            devshell.overlays.default
            fenix.overlays.default
          ];
        };
      forAllSystems = nixpkgs.lib.genAttrs systems;

      # Patched mpv: mpv 0.41.0 plus the tmux in-band passthrough
      # fix from scripts/mpv-passthrough/. The patch moves the
      # chunked base64 image data inside the tmux DCS envelope in
      # flip_page(). Stock mpv writes that payload outside the
      # envelope, so tmux renders it as pane text.
      #
      # The base is nixpkgs mpv-unwrapped, not pkgs.mpv. pkgs.mpv
      # is the with-scripts wrapper and ignores these overrides.
      # nixpkgs fetches the mpv source itself (fetchFromGitHub,
      # tag v0.41.0, hash-checked). That matches the tag the patch
      # is written against, so no src or version override is
      # needed. No mpv source is kept in this repository.
      mkMpvPatched =
        pkgs:
        (pkgs."mpv-unwrapped").overrideAttrs (old: {
          pname = "mpv-patched";
          patches = (old.patches or [ ])
            ++ [ ./scripts/mpv-passthrough/vo_kitty-tmux-passthrough.patch ];
          patchFlags = [ "-p1" ];
        });

      # One overlay: add the patched mpv to a nixpkgs package set.
      # Exposed as the flake overlays output and applied to this
      # flake packages and devShells with lib.fix.
      mpvPatchedOverlay =
        final: prev:
        {
          "mpv-patched" = mkMpvPatched prev;
          # Pass the base through so the overlay composes onto a
          # set that already went through it.
          "mpv-unwrapped" = prev."mpv-unwrapped";
        };
    in
    {
      # A Nix user installing the flake packages gets the patched
      # mpv:  nix profile install github:bilibili-dd-radio#mpv-patched
      #
      # This flake applies its own overlay: lib.fix computes
      # x = mpvPatchedOverlay x. The overlay output below is the
      # same function for consumers, for example a NixOS system
      # config.
      packages = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          overlaid = (nixpkgs.lib.fix mpvPatchedOverlay) pkgs;
        in
        {
          # Dashed names need quoting in a Nix attrset literal.
          "mpv-patched" = overlaid."mpv-patched";
          mpvPatched = overlaid."mpv-patched";
          "mpv-unwrapped" = overlaid."mpv-unwrapped";
          default = overlaid."mpv-patched";
        }
      );

      # Expose the overlay so consumers can add the patched mpv
      # to their own package sets.
      overlays = forAllSystems (
        system:
        {
          default = mpvPatchedOverlay;
        }
      );

      devShells = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          rustPackages = (
            fenix.packages.${system}.stable.withComponents [
              "cargo"
              "clippy"
              "rust-src"
              "rustc"
              "rustfmt"
              "rust-analyzer"
            ]
          );
        in
        {
          default = pkgs.devshell.mkShell {
            packages =
              with pkgs;
              [
                rustPackages
                # The patched mpv ships the tmux passthrough fix
                # that the per-room video overlay needs.
                ((nixpkgs.lib.fix mpvPatchedOverlay) pkgs)."mpv-patched"
              ]
              ++ lib.optionals stdenv.hostPlatform.isDarwin ([
                apple-sdk_26
                libiconv
              ]);
            env = with pkgs; lib.optionals stdenv.hostPlatform.isDarwin [
              {
                name = "RUSTFLAGS";
                value = "-C link-arg=-L${libiconv}/lib";
              }
              # The nix devshell shadows `cc` with gcc. The C code of
              # aws-lc-sys and rustc's final link need Apple clang on
              # aarch64-apple-darwin.
              {
                name = "CC";
                value = "/usr/bin/clang";
              }
              {
                name = "CXX";
                value = "/usr/bin/clang++";
              }
              {
                name = "CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER";
                value = "/usr/bin/clang";
              }
            ];
          };
        }
      );
    };
}
