{
  description = "VectorCraft — native app development shell and package for Linux";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      # Why this shell exists. winit opens the window by dlopen'ing the Wayland client libraries,
      # wgpu renders through the EGL loader (libEGL.so.1), and rfd's file dialogs reach
      # xdg-desktop-portal through libdbus-1; a plain `cargo run` under NixOS finds none of them,
      # because there is no /usr/lib and nothing puts them on LD_LIBRARY_PATH:
      #   winit EventLoopError: The wayland library could not be loaded
      #   eframe: WGPU error: Failed to create surface for any enabled backend: {}
      #   rfd::backend::xdg_desktop_portal::portal::libdbus: Can't connect to a portal: libdbus-1.so not found
      # The Mesa drivers themselves — and the EGL vendor / Vulkan ICD descriptions that name them —
      # come from the system in /run/opengl-driver (hardware.graphics.enable), not from here, since
      # that is the driver stack the compositor runs on. Verified on an Intel LNL iGPU under
      # Wayland: wgpu picks the GL backend through this loader and draws.
      # The X11 libraries are for `WAYLAND_DISPLAY= vectorcraft`, the XWayland run the development
      # notes recommend where drag and drop or a pen display has to work (winit 0.30 has no Wayland
      # drag and drop and doesn't bind the tablet protocol).
      runtimeLibs =
        pkgs:
        (with pkgs; [
          wayland
          libxkbcommon
          libglvnd
          dbus
        ])
        ++ (with pkgs; [
          # xorg.libX11 and friends are deprecated aliases in current nixpkgs.
          libx11
          libxcursor
          libxi
          libxrandr
          libxrender
          libxext
          libxcb
        ]);
      driverLibDir = "/run/opengl-driver/lib";
      libraryPath = pkgs: "${pkgs.lib.makeLibraryPath (runtimeLibs pkgs)}:${driverLibDir}";

      shellHook = ''
        if [ ! -d ${driverLibDir} ]; then
          echo "vectorcraft: ${driverLibDir} is missing — set hardware.graphics.enable = true so the Mesa drivers and their EGL vendor descriptions exist." >&2
        fi
        command -v cc >/dev/null || echo "vectorcraft: no C linker on PATH, cargo cannot link — use nix develop .#rust" >&2
        echo "vectorcraft: cargo run --release -p vectorcraft [file.svg]"
      '';
    in
    {
      devShells = forAllSystems (pkgs: {
        # The shell to work in. It brings no toolchain of its own, Rust or C: the ones already on
        # PATH (a system package, rustup, nixpkgs) keep their build cache and their linker, where a
        # second rustc — or mkShell's second gcc, with another glibc behind it — would make every
        # `cargo build` produce binaries from somewhere else.
        default = pkgs.mkShellNoCC {
          name = "vectorcraft";
          LD_LIBRARY_PATH = libraryPath pkgs;
          inherit shellHook;
        };

        # For a machine with no Rust at all: `nix develop .#rust`. Its own target directory, so
        # switching between this toolchain and the system one does not thrash the cache.
        rust = pkgs.mkShell {
          name = "vectorcraft-rust";
          packages = [
            pkgs.cargo
            pkgs.rustc
            pkgs.rustfmt
            pkgs.clippy
          ];
          LD_LIBRARY_PATH = libraryPath pkgs;
          # Under target/, which .gitignore already covers.
          CARGO_TARGET_DIR = "target/nix";
          shellHook = ''
            ${shellHook}
            echo "vectorcraft: $(rustc --version), building into target/nix"
          '';
        };
      });

      packages = forAllSystems (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "vectorcraft";
          version = (pkgs.lib.importTOML ./Cargo.toml).workspace.package.version;
          src = self;

          cargoLock.lockFile = ./Cargo.lock;
          # Build and install the app alone: the workspace also holds the CLI, xtask and the
          # crates only the test suites need.
          cargoBuildFlags = [
            "-p"
            "vectorcraft"
          ];
          cargoInstallFlags = [
            "-p"
            "vectorcraft"
          ];
          # `cargo xtask ci` is the test gate (it needs the wasm target and the corpus); the
          # sandbox has neither the network nor a GPU.
          doCheck = false;

          nativeBuildInputs = [ pkgs.makeWrapper ];
          buildInputs = runtimeLibs pkgs;

          # The app reaches the window through dlopen'd libraries, so the packaged binary needs the
          # same LD_LIBRARY_PATH the shell exports.
          postFixup = ''
            wrapProgram $out/bin/vectorcraft --prefix LD_LIBRARY_PATH : "${libraryPath pkgs}"
          '';

          meta = {
            description = "Rust-native vector illustration app";
            homepage = "https://github.com/storytold/vectorcraft";
            license = with pkgs.lib.licenses; [
              mit
              asl20
            ];
            mainProgram = "vectorcraft";
          };
        };
      });
    };
}
