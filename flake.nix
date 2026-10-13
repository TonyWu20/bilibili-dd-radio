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
    in
    {
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
                mpv
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
