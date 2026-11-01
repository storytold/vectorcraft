{
  description = "Experimental Nix flake for VectorCraft";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    craft-fonts = {
      url = "github:storytold/craft-fonts";
      flake = false;
    };
  };

  outputs = { self, nixpkgs, rust-overlay, craft-fonts }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ];

      version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;

      forAllSystems = f: nixpkgs.lib.genAttrs systems (system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ rust-overlay.overlays.default ];
          };
          commonDeps = with pkgs; [
            fontconfig
            freetype
            zlib
          ] ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [
            vulkan-loader
            wayland
            libxkbcommon
            libx11
            libxcursor
            libxrandr
            libxi
            libxcb
            libgcc
            libGL
            alsa-lib
          ];
        in
        f { inherit system pkgs commonDeps; }
      );
    in
    {
      devShells = forAllSystems ({ pkgs, commonDeps, ... }:
        let
          rust = pkgs.rust-bin.stable.latest.default.override {
            extensions = [ "rustfmt" "clippy" "rust-src" ];
          };
        in
        {
          default = pkgs.mkShell {
            nativeBuildInputs = with pkgs; [
              rust
              libclang
            ];
            buildInputs = commonDeps;
  
            shellHook = ''
              export LIBCLANG_PATH="${pkgs.libclang.lib}/lib"
            '';
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (with pkgs; [
              # No path links possible for non Linux
            ] ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [
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
        }
      );

      packages = forAllSystems ({ pkgs, commonDeps, ... }: {
        default = pkgs.rustPlatform.buildRustPackage {
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
          ] ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ autoPatchelfHook ]; # Hooks runtime dependencies to RPATH
          buildInputs = commonDeps;
  
          runtimeDependencies = with pkgs; [
            # No dependencies for non Linux possible
          ] ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [
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
          env = pkgs.lib.optionalAttrs (craft-fonts != null) {
            CRAFT_FONTS_DIR = "${craft-fonts}";
          };
          doCheck = false;
          meta = {
            license = with pkgs.lib.licenses; [ mit asl20 ofl ];
            maintainers = [ { name = "SaB131X"; email = "sea1024wm@gmail.com"; } ];
            description = "Vector illustration; an open-source, modern vector art application, built in pure Rust.";
            homepage = "https://github.com/storytold/vectorcraft";
            platforms = systems;
            badPlatforms = [ "x86_64-darwin" ]; # Nixpkgs removed support for x86_64-darwin in 26.11 upstream so sadly but we can't provide nixpkgs to that platform.
          };
        };
      });

      apps = forAllSystems ({ pkgs, system, ... }: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/vectorcraft";
        };
      });
    };
}
