{
  description = "Experimental Nix flake for VectorCraft";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

        rust = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rustfmt" "clippy" "rust-src" ];
        };

        version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;

        commonDeps = with pkgs; [
          pkg-config
          libxkbcommon
          libx11
          libxcursor
          libxrandr
          libxi
          libxcb
          libgcc
          libGL
          vulkan-loader
          wayland
          fontconfig
          freetype
          alsa-lib
          zlib
        ];
      in
      {
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            rust
            libclang
          ];
          buildInputs = commonDeps;

          shellHook = ''
            export LIBCLANG_PATH="${pkgs.libclang.lib}/lib"
          '';
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (with pkgs; [
            # Graphic APIs
            libGL
            vulkan-loader
            # Wayland support
            wayland
            libxkbcommon
            # X11 support
            libx11
            libxcursor
            libxrandr
            libxi
            libxcb
          ]);
        };

        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "vectorcraft";
          version = version;
          src = ./.;
          cargoLock = {
            lockFile = ./Cargo.lock;
            # Used purely because some libs don't have a hash in Cargo.lock
            # Better remove it later and add hash to all custom libs
            allowBuiltinFetchGit = true;
          };
          nativeBuildInputs = with pkgs; [
            pkg-config
            libclang
            # Hooks runtime dependencies to RPATH 
            autoPatchelfHook
          ];
          buildInputs = commonDeps;

          runtimeDependencies = with pkgs; [
            # Graphic APIs
            libGL
            vulkan-loader
            # Wayland support
            wayland
            libxkbcommon
            # X11 support
            libx11
            libxcursor
            libxrandr
            libxi
            libxcb
          ];

          doCheck = false;
        };

        apps.default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/vectorcraft";
        };
      });
}
