# VectorCraft app icon

**Creature:** a regal engraved dragon, VectorCraft's mascot.

**Style:** an engraving-style portrait in the Crafting Apps owl template (the FilmCraft owl): a tight
head-and-shoulders portrait on a full-bleed field, no frame or roundel, with the body running off the tile.

**Palette** (exactly three colours):

| Colour | Hex | Used for |
|---|---|---|
| Ink | `#0b0b0c` | line work and contour |
| Paper | `#efe9dc` | the figure |
| VectorCraft colour | `#e8573f` | the full-bleed field |

**Tile:** `viewBox="0 0 512 512"`, a rounded square with `rx=112`, clipped. The macOS files add Apple's
transparent margin (tile = 824/1024 of the canvas); Windows and Linux use the tile edge to edge.

**Provenance:** the owner's original ArtCraft drawing, vectorised. `vectorcraft.svg` is the canonical
artwork. Licence: see `LICENSE.txt`.

## Files

| File | What | Used by |
|---|---|---|
| `vectorcraft.svg` | master vector (traced at 2048 px, about 700 KB) | source for everything below |
| `vectorcraft-small.svg` | lighter vector (traced at 1024 px, about 370 KB) | `hicolor/scalable` |
| `vectorcraft-1024.png` | full tile, 1024 px | docs, store listings |
| `vectorcraft-macos-512.png` | tile with Apple margin, 512 px | runtime Dock icon (`apps/vectorcraft/src/main.rs`) |
| `vectorcraft.icns` | macOS icon set, 16 to 1024 px | `cargo xtask bundle` (`CFBundleIconFile`) |
| `vectorcraft.ico` | Windows icon, 16 to 256 px | `apps/vectorcraft/build.rs` (embedded in the `.exe`) |
| `hicolor/<size>/apps/ai.storyteller.vectorcraft.png` | Linux theme icons, 16 to 512 px; the 256 one is also the runtime icon on Windows and Linux, the 128 one the brand mark in the app bar and About box (`crates/ui-egui/src/brand.rs`) | `packaging/linux/ai.storyteller.vectorcraft.desktop` |
| `hicolor/scalable/apps/ai.storyteller.vectorcraft.svg` | Linux scalable icon | as above |
| `vectorcraft-nightly.png` | the full tile with a deeper field, 1024 px | the Nightly feed's art (`packaging/nightly-icons.py`) |
| `vectorcraft-nightly-1024.png` | the same tile with Apple's margin, 1024 px | the Nightly `.app` master |
| `vectorcraft-nightly.ico` | Windows icons for the Nightly executable, 16 to 256 px | `apps/vectorcraft/build.rs` when built for `nightly-v1` |
| `vectorcraft-nightly.icns` | macOS icon set for the Nightly `.app`, 16 to 1024 px | `packaging/macos/nightly.sh` |

## How it reaches each OS

- **macOS:** the app sets the Dock / app-switcher icon at runtime (`ViewportBuilder::with_icon`); the
  `.app` from `cargo xtask bundle` carries `vectorcraft.icns`.
- **Windows:** `build.rs` embeds `vectorcraft.ico` and VERSIONINFO with `winresource` (taskbar, Start
  menu, Explorer, Alt-Tab); the runtime icon covers the title bar.
- **Linux:** install `packaging/linux/ai.storyteller.vectorcraft.desktop` to `share/applications/` and
  `hicolor/` to `share/icons/`. The app sets its Wayland app ID / X11 class to
  `ai.storyteller.vectorcraft`, so the dock matches the window to the launcher.

## Regenerate

Edit or replace `vectorcraft.svg` (and `vectorcraft-small.svg`), then run `packaging/icons.sh`
(needs `resvg`; `iconutil` on macOS for the `.icns`). It writes every PNG, the `.ico` (via
`cargo xtask ico`) and the `.icns`.

The Nightly art comes from the committed stable art, not from a new drawing: `python3
packaging/nightly-icons.py` solves each pixel's field contribution from the three-colour palette
(`#e8573f`, `#0b0b0c`, `#efe9dc`), adds the deeper field (`#a83a24`) and leaves the figure (ink and
paper) exactly as it was, then scales every frame natively with ffmpeg and packs the `.ico` (32bpp
BGRA frames with AND masks up to 128, a PNG frame for 256) and the `.icns` (PNG frames). No badge,
no crescent and no letter: the field alone says Nightly.
