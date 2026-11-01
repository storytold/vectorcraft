{
  description = "VectorCraft — vector illustration, a free alternative to Illustrator, in pure Rust";

  inputs = {
    # VectorCraft needs edition 2024 and rustc ≥ 1.95, so the flake tracks unstable. Consumers on a
    # release channel can retarget it: `inputs.vectorcraft.inputs.nixpkgs.follows = "nixpkgs";`
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    # Optional build input of the repository: the fonts that the build embeds when `CRAFT_FONTS_DIR`
    # is set. Official releases always build with them; without them everything works but some
    # scripts have no glyphs. Pinned to the revision the release workflow uses — bump deliberately
    # (`nix flake update craft-fonts`).
    craft-fonts = {
      url = "github:storytold/craft-fonts/3d9ab3a132ed0eff457e424226a435a901da30d3";
      flake = false;
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      craft-fonts,
    }:
    let
      inherit (nixpkgs) lib;

      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];

      # The AppStream metadata carries a release date. The flake source's own commit date is
      # reproducible, unlike "now"; a tarball or dirty tree without one falls back to the epoch.
      buildDate =
        let
          d = self.lastModifiedDate or "";
        in
        if builtins.stringLength d >= 8 then
          "${builtins.substring 0 4 d}-${builtins.substring 4 2 d}-${builtins.substring 6 2 d}"
        else
          "1970-01-01";

      packageArgs = {
        inherit craft-fonts buildDate;
      };

      forAllSystems = f: lib.genAttrs systems (system: f (import nixpkgs { inherit system; }) system);
    in
    {
      packages = forAllSystems (
        pkgs: _:
        let
          vectorcraft = pkgs.callPackage ./nix/package.nix packageArgs;
        in
        {
          inherit vectorcraft;
          default = vectorcraft;
        }
      );

      # `nixpkgs.overlays = [ inputs.vectorcraft.overlays.default ]` → `pkgs.vectorcraft` everywhere.
      overlays.default = final: _prev: {
        vectorcraft = final.callPackage ./nix/package.nix packageArgs;
      };

      apps = forAllSystems (
        _: system: {
          default = {
            type = "app";
            program = lib.getExe self.packages.${system}.vectorcraft;
          };
        }
      );

      devShells = forAllSystems (
        pkgs: system:
        let
          vectorcraft = self.packages.${system}.vectorcraft;
        in
        {
          default = pkgs.mkShell (
            {
              # The package's own native inputs, so `cargo build` in the shell needs no extra setup.
              inputsFrom = [ vectorcraft ];
              packages = with pkgs; [
                cargo
                rustc
                clippy
                rustfmt
                rust-analyzer
              ];
              # `cargo run` dlopens the Vulkan loader, libwayland and libxkbcommon like the packaged binary.
              LD_LIBRARY_PATH = lib.makeLibraryPath vectorcraft.passthru.runtimeLibs;
              RUST_BACKTRACE = "1";
            }
            // lib.optionalAttrs (craft-fonts != null) {
              # Embedded fonts in `cargo run` too; build.rs reads this as CRAFT_FONTS_DIR.
              CRAFT_FONTS_DIR = toString craft-fonts;
            }
          );
        }
      );

      checks = forAllSystems (
        pkgs: system: {
          vectorcraft = self.packages.${system}.vectorcraft;

          # `nix fmt` formats the flake; this keeps it formatted.
          formatting =
            pkgs.runCommand "vectorcraft-nix-formatting" { nativeBuildInputs = [ pkgs.nixfmt ]; }
              ''
                nixfmt --check ${./flake.nix} ${./nix}
                touch $out
              '';
        }
      );

      formatter = forAllSystems (pkgs: _: pkgs.nixfmt);
    };
}
