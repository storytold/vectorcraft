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
      # Nixpkgs removed support for x86_64-darwin in 26.11 upstream so sadly but we can't provide nixpkgs to that platform.
      # badPlatforms = [ "x86_64-darwin" ]; will be depricated so keep in mind that packages won't support x86_64-darwin
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

      packages = forAllSystems ({ pkgs, commonDeps, system, ... }:
        let
          common = {
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
            doCheck = false;
            env = pkgs.lib.optionalAttrs (craft-fonts != null) {
              CRAFT_FONTS_DIR = "${craft-fonts}";
            };
            postInstall = if craft-fonts != null then ''
              mkdir -p "$out/share/doc/vectorcraft"
              shopt -s nullglob
              for ofl in ${craft-fonts}/fonts/*/OFL.txt; do
                [[ -f "$ofl" ]] || continue
                family=$(basename "$(dirname "$ofl")")
                install -Dm644 "$ofl" "$out/share/doc/vectorcraft/OFL-$family.txt"
              done
              shopt -u nullglob
            '' else "";
          };
          commonMeta = {
            license = with pkgs.lib.licenses; [ mit asl20 ofl ];
            maintainers = [ { name = "SaB131X"; email = "sea1024wm@gmail.com"; } ]; # Temporary set myself as a maintainer of nixpkg
            homepage = "https://github.com/storytold/vectorcraft";
            platforms = systems;
          };
        in {
          vectorcraft = pkgs.rustPlatform.buildRustPackage (common // {
            pname = "vectorcraft";
            cargoBuildFlags = [ "-p" "vectorcraft" ];
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
            meta = commonMeta // { description = "Vector illustration; an open-source, modern vector art application, built in pure Rust."; };
          });
          vectorcraft-cli = pkgs.rustPlatform.buildRustPackage (common // {
            pname = "vectorcraft-cli";
            cargoBuildFlags = [ "-p" "vectorcraft-cli" ];
            buildInputs = with pkgs; [
              fontconfig
              freetype
              zlib
            ];
            meta = commonMeta // { description = "Headless command runner and MCP server for VectorCraft."; };
          });
          default = self.packages.${system}.vectorcraft;
      });

      apps = forAllSystems ({ pkgs, system, ... }: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/vectorcraft";
        };
      });
    };
}
