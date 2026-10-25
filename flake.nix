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

      # Patched mpv: mpv v0.41.0 plus the tmux in-band passthrough
      # fix from scripts/mpv-passthrough/. The patch moves the
      # chunked base64 image data inside the tmux DCS envelope in
      # flip_page(). Stock mpv writes that payload outside the
      # envelope, so tmux renders it as pane text.
      #
      # Override mpv-unwrapped, the base derivation. pkgs.mpv is
      # the with-scripts wrapper and ignores these overrides.
      # The mpv v0.41.0 source is vendored: github.com is not
      # reliably reachable from the build hosts (codeload 404s,
      # in-sandbox git fetch fails). The vendored tree is the
      # exact tag 41f6a645068483470267271e1d09966ca3b9f413.
      # Its version matches the nixpkgs base, 0.41.0.
      mkMpvPatched =
        pkgs:
        (pkgs."mpv-unwrapped").overrideAttrs (old: {
          src = ./vendor/mpv-0.41.0;
          patches = [ ./scripts/mpv-passthrough/vo_kitty-tmux-passthrough.patch ];
          patchFlags = [ "-p1" ];
        });
    in
    {
      # A Nix user installing the flake packages gets the patched
      # mpv:  nix profile install github:bilibili-dd-radio#mpv-patched
      packages = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          mpvPatched = mkMpvPatched pkgs;
        in
        {
          # Dashed names need quoting in a Nix attrset literal.
          "mpv-patched" = mpvPatched;
          inherit mpvPatched;
          default = mpvPatched;
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
                (mkMpvPatched pkgs)
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
