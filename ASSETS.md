# Asset attribution

Every non-code asset in this repository (icons, images, fonts, example art, colour profiles, presets) is listed here with its author, source and licence. `cargo xtask assets` (part of `cargo xtask ci`) fails if an asset file is missing from this table.

**Policy (mandatory):** VectorCraft contains **no Adobe iconography, images, artwork, presets, swatch/brush/symbol/pattern libraries or ICC profiles**. Every asset is either original work by VectorCraft contributors, or third-party material under an open licence (OSI open source, public domain / CC0, or Creative Commons that allows redistribution), and it is attributed below. Screenshots of Adobe software are never committed. The one exception is the ArtCraft name, wordmark and logos in `docs/brand/`: ArtCraft trademarks, not open source, usable only under [`docs/brand/LICENSE-brand.txt`](docs/brand/LICENSE-brand.txt).

Generated-in-code art is original and has no file to list. This covers the default swatches, brushes, symbols, patterns, graphic styles, image-trace presets and the vector tool cursors (`crates/ui-egui/src/cursors.rs`).

**Font files are never committed here.** Fonts shared by the Crafting Apps live in [storytold/craft-fonts](https://github.com/storytold/craft-fonts) (rules: craftrules [`standards/fonts.md`](https://github.com/storytold/craftrules/blob/main/standards/fonts.md)). The small Latin UI/document fonts below predate that rule and stay.

**Optional build input: craft-fonts.** Builds made with `CRAFT_FONTS_DIR=<craft-fonts checkout>` (all official releases) embed the Japanese fonts listed in its [`ATTRIBUTION.md`](https://github.com/storytold/craft-fonts/blob/main/ATTRIBUTION.md): BIZ UDPGothic Regular and Bold, Shippori Mincho Regular and BIZ UDMincho Regular (web builds: BIZ UDPGothic Regular only), all under the SIL Open Font License 1.1. They are not files in this repository; release packages carry each one's `OFL-<family>.txt`. Builds without `CRAFT_FONTS_DIR` embed none of them.

| Asset | Author | Source | Licence | Notes |
|---|---|---|---|---|
| `docs/brand/artcraft-logo-white.png` | ArtCraft team (project owner) | https://getartcraft.com/ | ArtCraft trademark, see docs/brand/LICENSE-brand.txt (not open source) | Used in README only; not shipped in the app |
| `docs/brand/artcraft-logo-white.svg` | ArtCraft team (project owner) | https://getartcraft.com/ | ArtCraft trademark, see docs/brand/LICENSE-brand.txt (not open source) | Used in README only; not shipped in the app |
| `docs/brand/artcraft-logo.png` | ArtCraft team (project owner) | https://getartcraft.com/ | ArtCraft trademark, see docs/brand/LICENSE-brand.txt (not open source) | Used in README only; not shipped in the app |
| `docs/brand/artcraft-logo.svg` | ArtCraft team (project owner) | https://getartcraft.com/ | ArtCraft trademark, see docs/brand/LICENSE-brand.txt (not open source) | Used in README only; not shipped in the app |
| `docs/brand/artcraft-mark-black.png` | ArtCraft team (project owner) | https://getartcraft.com/ | ArtCraft trademark, see docs/brand/LICENSE-brand.txt (not open source) | Used in README only; not shipped in the app |
| `docs/brand/artcraft-mark-black.svg` | ArtCraft team (project owner) | https://getartcraft.com/ | ArtCraft trademark, see docs/brand/LICENSE-brand.txt (not open source) | Used in README only; not shipped in the app |
| `docs/brand/artcraft-mark.png` | ArtCraft team (project owner) | https://getartcraft.com/ | ArtCraft trademark, see docs/brand/LICENSE-brand.txt (not open source) | Used in README only; not shipped in the app |
| `docs/brand/artcraft-mark.svg` | ArtCraft team (project owner) | https://getartcraft.com/ | ArtCraft trademark, see docs/brand/LICENSE-brand.txt (not open source) | Used in README only; not shipped in the app |
| `assets/app-icon/LICENSE.txt` | (licence/readme text) | VectorCraft | MIT OR Apache-2.0 |  |
| `assets/app-icon/README.md` | (licence/readme text) | VectorCraft | MIT OR Apache-2.0 |  |
| `assets/app-icon/hicolor/128x128/apps/ai.storyteller.vectorcraft.png` | Project owner (ArtCraft team) | Original artwork drawn in ArtCraft for VectorCraft, vectorised and rendered by `packaging/icons.sh` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (engraved dragon) |
| `assets/app-icon/hicolor/16x16/apps/ai.storyteller.vectorcraft.png` | Project owner (ArtCraft team) | Original artwork drawn in ArtCraft for VectorCraft, vectorised and rendered by `packaging/icons.sh` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (engraved dragon) |
| `assets/app-icon/hicolor/24x24/apps/ai.storyteller.vectorcraft.png` | Project owner (ArtCraft team) | Original artwork drawn in ArtCraft for VectorCraft, vectorised and rendered by `packaging/icons.sh` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (engraved dragon) |
| `assets/app-icon/hicolor/256x256/apps/ai.storyteller.vectorcraft.png` | Project owner (ArtCraft team) | Original artwork drawn in ArtCraft for VectorCraft, vectorised and rendered by `packaging/icons.sh` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (engraved dragon) |
| `assets/app-icon/hicolor/32x32/apps/ai.storyteller.vectorcraft.png` | Project owner (ArtCraft team) | Original artwork drawn in ArtCraft for VectorCraft, vectorised and rendered by `packaging/icons.sh` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (engraved dragon) |
| `assets/app-icon/hicolor/48x48/apps/ai.storyteller.vectorcraft.png` | Project owner (ArtCraft team) | Original artwork drawn in ArtCraft for VectorCraft, vectorised and rendered by `packaging/icons.sh` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (engraved dragon) |
| `assets/app-icon/hicolor/512x512/apps/ai.storyteller.vectorcraft.png` | Project owner (ArtCraft team) | Original artwork drawn in ArtCraft for VectorCraft, vectorised and rendered by `packaging/icons.sh` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (engraved dragon) |
| `assets/app-icon/hicolor/64x64/apps/ai.storyteller.vectorcraft.png` | Project owner (ArtCraft team) | Original artwork drawn in ArtCraft for VectorCraft, vectorised and rendered by `packaging/icons.sh` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (engraved dragon) |
| `assets/app-icon/hicolor/scalable/apps/ai.storyteller.vectorcraft.svg` | Project owner (ArtCraft team) | Original artwork drawn in ArtCraft for VectorCraft, vectorised and rendered by `packaging/icons.sh` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (engraved dragon) |
| `assets/app-icon/vectorcraft-1024.png` | Project owner (ArtCraft team) | Original artwork drawn in ArtCraft for VectorCraft, vectorised and rendered by `packaging/icons.sh` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (engraved dragon) |
| `assets/app-icon/vectorcraft-macos-512.png` | Project owner (ArtCraft team) | Original artwork drawn in ArtCraft for VectorCraft, vectorised and rendered by `packaging/icons.sh` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (engraved dragon) |
| `assets/app-icon/vectorcraft-small.svg` | Project owner (ArtCraft team) | Original artwork drawn in ArtCraft for VectorCraft, vectorised and rendered by `packaging/icons.sh` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (engraved dragon) |
| `assets/app-icon/vectorcraft.icns` | Project owner (ArtCraft team) | Original artwork drawn in ArtCraft for VectorCraft, vectorised and rendered by `packaging/icons.sh` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (engraved dragon) |
| `assets/app-icon/vectorcraft.ico` | Project owner (ArtCraft team) | Original artwork drawn in ArtCraft for VectorCraft, vectorised and rendered by `packaging/icons.sh` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (engraved dragon) |
| `assets/app-icon/vectorcraft.svg` | Project owner (ArtCraft team) | Original artwork drawn in ArtCraft for VectorCraft, vectorised and rendered by `packaging/icons.sh` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (engraved dragon) |
| `packaging/macos/dmg/background.svg` | Xus Badia (contributed in #493) | Original artwork for VectorCraft's DMG window; links the app icon `assets/app-icon/vectorcraft-small.svg` | MIT OR Apache-2.0 (licensed by the contributor under the project licence) | DMG background source. Text in Inter and JetBrains Mono (OFL-1.1, `assets/fonts`), not embedded; paper grain generated by an SVG `feTurbulence` filter |
| `packaging/macos/dmg/background.tiff` | Xus Badia (contributed in #493) | Rendered from `packaging/macos/dmg/background.svg` by `packaging/macos/dmg/generate.py` | MIT OR Apache-2.0 | 1x + 2x DMG background; includes the app icon (see its rows), text rendered from OFL-1.1 fonts and LittleCMS's built-in sRGB profile ("No copyright, use freely") |
| `packaging/macos/dmg/DS_Store` | Xus Badia (layout, #493) | Written by `packaging/macos/dmg/generate.py` | MIT OR Apache-2.0 | Finder view settings for the DMG (window, icon size and positions, background alias); no machine data |
| `assets/fonts/Inter-Medium.ttf` | Rasmus Andersson / The Inter Project Authors | https://github.com/rsms/inter | OFL-1.1 (`assets/fonts/OFL-Inter.txt`) | Open-source typeface, not product iconography or artwork |
| `assets/fonts/Inter-Regular.ttf` | Rasmus Andersson / The Inter Project Authors | https://github.com/rsms/inter | OFL-1.1 (`assets/fonts/OFL-Inter.txt`) | Open-source typeface, not product iconography or artwork |
| `assets/fonts/Inter-SemiBold.ttf` | Rasmus Andersson / The Inter Project Authors | https://github.com/rsms/inter | OFL-1.1 (`assets/fonts/OFL-Inter.txt`) | Open-source typeface, not product iconography or artwork |
| `assets/fonts/JetBrainsMono-Regular.ttf` | The JetBrains Mono Project Authors | https://github.com/JetBrains/JetBrainsMono | OFL-1.1 (`assets/fonts/OFL-JetBrainsMono.txt`) | Open-source typeface, not product iconography or artwork |
| `assets/fonts/SourceSans3-Bold.ttf` | Paul D. Hunt / Adobe (released as open source) | https://github.com/adobe-fonts/source-sans | OFL-1.1 (`assets/fonts/OFL-SourceSans3.txt`) | Open-source typeface, not product iconography or artwork |
| `assets/fonts/SourceSans3-It.ttf` | Paul D. Hunt / Adobe (released as open source) | https://github.com/adobe-fonts/source-sans | OFL-1.1 (`assets/fonts/OFL-SourceSans3.txt`) | Open-source typeface, not product iconography or artwork |
| `assets/fonts/SourceSans3-Regular.ttf` | Paul D. Hunt / Adobe (released as open source) | https://github.com/adobe-fonts/source-sans | OFL-1.1 (`assets/fonts/OFL-SourceSans3.txt`) | Open-source typeface, not product iconography or artwork |
| `assets/fonts/SourceSans3-Semibold.ttf` | Paul D. Hunt / Adobe (released as open source) | https://github.com/adobe-fonts/source-sans | OFL-1.1 (`assets/fonts/OFL-SourceSans3.txt`) | Open-source typeface, not product iconography or artwork |
| `assets/fonts/SourceSerif4-Regular.ttf` | Frank Grießhammer / Adobe (released as open source) | https://github.com/adobe-fonts/source-serif | OFL-1.1 (`assets/fonts/OFL-SourceSerif4.txt`) | Open-source typeface, not product iconography or artwork |
| `assets/fonts/OFL-Inter.txt` | (licence text) | upstream project | — |  |
| `assets/fonts/OFL-JetBrainsMono.txt` | (licence text) | upstream project | — |  |
| `assets/fonts/OFL-SourceSans3.txt` | (licence text) | upstream project | — |  |
| `assets/fonts/OFL-SourceSerif4.txt` | (licence text) | upstream project | — |  |
| `assets/icons/LICENSE-lucide.txt` | (licence text) | upstream project | — |  |
| `assets/icons/align-center-horizontal.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/align-center-vertical.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/align-end-horizontal.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/align-end-vertical.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/align-horizontal-justify-center.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/align-horizontal-justify-end.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/align-horizontal-justify-start.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/align-start-horizontal.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/align-start-vertical.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/align-vertical-justify-center.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/align-vertical-justify-end.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/align-vertical-justify-start.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/arrow-left-right.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/arrow-up-down.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/blend.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/brush.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/chart-column.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/chevron-down.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC; derived from Feather, also MIT (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/chevron-left.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC; derived from Feather, also MIT (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/chevron-right.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC; derived from Feather, also MIT (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/chevrons-left.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC; derived from Feather, also MIT (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/chevrons-right.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC; derived from Feather, also MIT (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/circle.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC; derived from Feather, also MIT (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/clipboard-paste.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/cloud.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/combine.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/copy.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/dc-actions.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-al-bottom.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-al-center.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-al-hcenter.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-al-left.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-al-right.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-al-top.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-al-vcenter.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-align.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-alignto-artboard.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-alignto-key.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-alignto-selection.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-anchor.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-anchor-smooth.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-appearance.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-arc.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-arrow-down.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-arrow-extend.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-arrow-fill.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 | The Selection and Direct Selection arrows' fill, painted black or white under their outline |
| `assets/icons/dc-arrow-tip.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-arrow-up.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-artboard-options.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-artboards.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-blend.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-bloat.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-cap-butt.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-cap-round.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-cap-square.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-center-hide.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-center-show.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-clear.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-color-guide.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-crystallize.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-cube.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-dash-align.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-dash-exact.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-dimension.svg` | Osama Amin | Original work, drawn for VectorCraft: a dimension line with arrowheads between two extension lines | MIT OR Apache-2.0 |  |
| `assets/icons/dc-dir-off.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-dir-on.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-direct.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-dist-bottom.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-dist-hcenter.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-dist-hspace.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-dist-left.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-dist-right.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-dist-top.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-dist-vcenter.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-dist-vspace.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-draw-behind.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-draw-inside.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-draw-normal.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-ellipse.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-fill-none.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-filter.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-flare.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-folder.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-free-transform.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-fx.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-gamut.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-grad-freeform.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-grad-linear.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-grad-radial.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-grad-stroke-across.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-grad-stroke-along.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-grad-stroke-within.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-gradient.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-graphic-styles.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-grid-view.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-group-select.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-join-bevel.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-join-miter.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-join-round.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-join.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-knife.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-line-cut.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-line.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-list-view.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-live-bucket.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-live-select.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-mask-none.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-measure.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-mesh.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-mirror-cut.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-new-fill.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-new-item.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-new-stroke.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-objects-on-path.svg` | VectorCraft contributors | Original work, drawn for VectorCraft: three small squares along a curve | MIT OR Apache-2.0 |  |
| `assets/icons/dc-options.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-para-bullets.svg` | VectorCraft contributors | Original work, drawn for VectorCraft: three dots, each before a line | MIT OR Apache-2.0 |  |
| `assets/icons/dc-para-center.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-para-justify-all.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-para-justify-center.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-para-justify-left.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-para-justify-right.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-para-left.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-para-ltr.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-para-numbers.svg` | VectorCraft contributors | Original work, drawn for VectorCraft: the digits 1, 2 and 3, each before a line | MIT OR Apache-2.0 |  |
| `assets/icons/dc-para-right.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-para-rtl.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-path-eraser.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-pathfinder.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-perspective.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-pf-crop.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-pf-divide.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-pf-exclude.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-pf-intersect.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-pf-merge.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-pf-minus-back.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-pf-minus-front.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-pf-outline.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-pf-trim.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-pf-unite.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-place-symbol.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-polar-grid.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-polygon.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-pucker.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-puppet.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-rearrange.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-rect-cut.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-rect-grid.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-reference-point.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-remove-brush.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-reshape.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-reverse.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-rotate-view.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-rounded-rect.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-rule-evenodd.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-rule-nonzero.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-scallop.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-screen-mode.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-selection.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-shape-builder.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-shear.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-smooth.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-stroke-center.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-stroke-inside.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-stroke-outside.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-stroke.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-swap.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-swatch-kinds.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-symbol-sprayer.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-touch-type.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-transform-panel.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-transparency.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-twirl.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-type-area.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-type-path.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-type-vertical.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-width-profile.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-width.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-wrinkle.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-zoom-large.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/dc-zoom-small.svg` | VectorCraft contributors | Original work, drawn for VectorCraft | MIT OR Apache-2.0 |  |
| `assets/icons/ellipsis.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/eraser.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/eye-off.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/eye.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/file-plus.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/flip-horizontal-2.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/flip-vertical-2.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/folder-open.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/frame.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/grid-3x3.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/group.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/hand.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/hexagon.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/history.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/house.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/image.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/info.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC; derived from Feather, also MIT (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/lasso.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/layers.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/layout-grid.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/library.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/link-2-off.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/link.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC; derived from Feather, also MIT (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/external-link.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC; derived from Feather, also MIT (`assets/icons/LICENSE-lucide.txt`) | Help → app page link |
| `assets/icons/git-branch.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) | Help → GitHub link |
| `assets/icons/globe.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) | Help → website link |
| `assets/icons/message-circle.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) | Discord button and Help → Discord link |
| `assets/icons/lock-open.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/lock.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC; derived from Feather, also MIT (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/map.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/maximize-2.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/menu.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/minus.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC; derived from Feather, also MIT (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/monitor.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC; derived from Feather, also MIT (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/move-diagonal-2.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/move.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC; derived from Feather, also MIT (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/paint-bucket.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/paintbrush.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/palette.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/pen-tool.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/pen-tool-add.svg` | Lucide Icons and Contributors; badge by VectorCraft contributors | https://lucide.dev (lucide-static v1.49.0 `pen-tool`; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) | Lucide's pen-tool with a plus badge |
| `assets/icons/pen-tool-delete.svg` | Lucide Icons and Contributors; badge by VectorCraft contributors | https://lucide.dev (lucide-static v1.49.0 `pen-tool`; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) | Lucide's pen-tool with a minus badge |
| `assets/icons/pencil.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/pilcrow.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/pin.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/pipette.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/plus.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC; derived from Feather, also MIT (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/printer.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/redo-2.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/rotate-3d.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/rotate-ccw.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/rotate-cw.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/ruler.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/save.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/scaling.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/scissors.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/search.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC; derived from Feather, also MIT (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/settings.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/shapes.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/share-2.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/slice.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/sparkles.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/spline-pointer.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/spline.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/spray-can.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/square-dashed.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/square.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC; derived from Feather, also MIT (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/squares-exclude.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/squares-intersect.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/squares-subtract.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/squares-unite.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/squircle.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/star.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/sun.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/swatch-book.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/text-cursor-input.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/tornado.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/trash-2.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC; derived from Feather, also MIT (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/triangle.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC; derived from Feather, also MIT (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/type.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC; derived from Feather, also MIT (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/undo-2.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/ungroup.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/wand-sparkles.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/waves.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/x.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC; derived from Feather, also MIT (`assets/icons/LICENSE-lucide.txt`) |  |
| `assets/icons/zoom-in.svg` | Lucide Icons and Contributors | https://lucide.dev (lucide-static v1.49.0; header kept in file) | ISC; derived from Feather, also MIT (`assets/icons/LICENSE-lucide.txt`) |  |
| `docs/images/dusk-poster.png` | VectorCraft contributors | Rendered by VectorCraft from `examples/dusk-poster.vectorcraft` | MIT OR Apache-2.0 |  |
| `examples/dusk-poster.vectorcraft` | VectorCraft contributors | Original artwork built through the VectorCraft command API | MIT OR Apache-2.0 |  |
| `examples/dusk-poster.svg` | VectorCraft contributors | SVG export of the above | MIT OR Apache-2.0 |  |
| `docs/images/shot-1-neon.png` | VectorCraft contributors | Screenshot of VectorCraft itself (VectorCraft/Lucide UI icons only), editing `examples/neon-drive.vectorcraft` | MIT OR Apache-2.0 |  |
| `docs/images/shot-2-ribbons.png` | VectorCraft contributors | Screenshot of VectorCraft itself, editing `examples/ribbons.vectorcraft` | MIT OR Apache-2.0 |  |
| `docs/images/shot-3-sheet.png` | VectorCraft contributors | Screenshot of VectorCraft itself, editing `examples/feature-sheet.vectorcraft` | MIT OR Apache-2.0 |  |
| `docs/images/shot-4-bezier.png` | VectorCraft contributors | Screenshot of VectorCraft itself, editing `examples/feature-sheet.vectorcraft` | MIT OR Apache-2.0 |  |
| `docs/images/shot-5-perspective.png` | VectorCraft contributors | Screenshot of VectorCraft itself, editing `examples/perspective-city.vectorcraft` | MIT OR Apache-2.0 |  |
| `docs/images/shot-6-revolve.png` | VectorCraft contributors | Screenshot of VectorCraft itself, editing `examples/still-life.vectorcraft` | MIT OR Apache-2.0 |  |
| `docs/images/shot-7-dimensions.png` | VectorCraft contributors | Screenshot of VectorCraft itself, editing `examples/bracket-drawing.vectorcraft` | MIT OR Apache-2.0 |  |
| `docs/images/art-still-life.png` | VectorCraft contributors | Rendered by VectorCraft from `examples/still-life.vectorcraft` | MIT OR Apache-2.0 |  |
| `examples/still-life.vectorcraft` | VectorCraft contributors | Original artwork built through the VectorCraft command API | MIT OR Apache-2.0 |  |
| `examples/bracket-drawing.vectorcraft` | VectorCraft contributors | Original drawing built through the VectorCraft command API | MIT OR Apache-2.0 |  |
| `docs/images/art-neon-drive.png` | VectorCraft contributors | Rendered by VectorCraft from `examples/neon-drive.vectorcraft` | MIT OR Apache-2.0 |  |
| `docs/images/art-ribbons.png` | VectorCraft contributors | Rendered by VectorCraft from `examples/ribbons.vectorcraft` | MIT OR Apache-2.0 |  |
| `docs/images/art-pathfinder.png` | VectorCraft contributors | Rendered by VectorCraft from `examples/feature-sheet.vectorcraft` (artboard 1) | MIT OR Apache-2.0 |  |
| `docs/images/art-mesh.png` | VectorCraft contributors | Rendered by VectorCraft from `examples/feature-sheet.vectorcraft` (artboard 2) | MIT OR Apache-2.0 |  |
| `docs/images/art-repeat.png` | VectorCraft contributors | Rendered by VectorCraft from `examples/feature-sheet.vectorcraft` (artboard 3) | MIT OR Apache-2.0 |  |
| `docs/images/art-envelope.png` | VectorCraft contributors | Rendered by VectorCraft from `examples/feature-sheet.vectorcraft` (artboard 4) | MIT OR Apache-2.0 |  |
| `examples/neon-drive.vectorcraft` | VectorCraft contributors | Original artwork built through the VectorCraft command API | MIT OR Apache-2.0 |  |
| `examples/ribbons.vectorcraft` | VectorCraft contributors | Original artwork built through the VectorCraft command API | MIT OR Apache-2.0 |  |
| `examples/feature-sheet.vectorcraft` | VectorCraft contributors | Original artwork built through the VectorCraft command API | MIT OR Apache-2.0 |  |
| `examples/perspective-city.vectorcraft` | VectorCraft contributors | Original artwork built through the VectorCraft command API | MIT OR Apache-2.0 |  |

## Assets embedded by the PDF backend

The shared PDF backend keeps the existing standard-font and CMap features. These files remain in dependency sources; no font files are copied into this repository. Hashes identify the pinned dependency contents.

| Crate / file | Author | Source | Licence | SHA-256 | Use |
| --- | --- | --- | --- | --- | --- |
| `hayro-interpret 0.7.0 / assets/FoxitDingbats.pfb` | Foxit Software Inc. / PDFium Authors | https://github.com/storytold/craft-libs/blob/2eca9b3a53a98ac52efed2a27353c6a6927ead10/vendor/hayro-interpret/assets/FoxitDingbats.pfb | BSD-3-Clause | `845c752392b6c914fb989c75a08b7792b88f542d2499042ef2889f8c814a16ed` | Renders non-embedded standard-14 fonts (pinned hayro-interpret) |
| `hayro-interpret 0.7.0 / assets/FoxitFixed.pfb` | Foxit Software Inc. / PDFium Authors | https://github.com/storytold/craft-libs/blob/2eca9b3a53a98ac52efed2a27353c6a6927ead10/vendor/hayro-interpret/assets/FoxitFixed.pfb | BSD-3-Clause | `b6c8fe53f134b8b6d4578cd2d544df4cee9624c4efa8d51a560fb40ea296101b` | Renders non-embedded standard-14 fonts (pinned hayro-interpret) |
| `hayro-interpret 0.7.0 / assets/FoxitFixedBold.pfb` | Foxit Software Inc. / PDFium Authors | https://github.com/storytold/craft-libs/blob/2eca9b3a53a98ac52efed2a27353c6a6927ead10/vendor/hayro-interpret/assets/FoxitFixedBold.pfb | BSD-3-Clause | `f1b7159702973f54fc86254ea38bcf3712b2a736eb3a0995751e9f4bb45ad603` | Renders non-embedded standard-14 fonts (pinned hayro-interpret) |
| `hayro-interpret 0.7.0 / assets/FoxitFixedBoldItalic.pfb` | Foxit Software Inc. / PDFium Authors | https://github.com/storytold/craft-libs/blob/2eca9b3a53a98ac52efed2a27353c6a6927ead10/vendor/hayro-interpret/assets/FoxitFixedBoldItalic.pfb | BSD-3-Clause | `8a000945843bd31add06aee63bf9fd41b7578b4f3242a4b7cd46349ad9d24d4d` | Renders non-embedded standard-14 fonts (pinned hayro-interpret) |
| `hayro-interpret 0.7.0 / assets/FoxitFixedItalic.pfb` | Foxit Software Inc. / PDFium Authors | https://github.com/storytold/craft-libs/blob/2eca9b3a53a98ac52efed2a27353c6a6927ead10/vendor/hayro-interpret/assets/FoxitFixedItalic.pfb | BSD-3-Clause | `5007faf8320fa1fcc08b8894b356ad976f60b992ba29ee5272578bbdebaf3876` | Renders non-embedded standard-14 fonts (pinned hayro-interpret) |
| `hayro-interpret 0.7.0 / assets/FoxitSans.pfb` | Foxit Software Inc. / PDFium Authors | https://github.com/storytold/craft-libs/blob/2eca9b3a53a98ac52efed2a27353c6a6927ead10/vendor/hayro-interpret/assets/FoxitSans.pfb | BSD-3-Clause | `a02339e95d20db14cbdfc1c6baf359cd36a091d5434925c3a4040f60e1a1af9e` | Renders non-embedded standard-14 fonts (pinned hayro-interpret) |
| `hayro-interpret 0.7.0 / assets/FoxitSansBold.pfb` | Foxit Software Inc. / PDFium Authors | https://github.com/storytold/craft-libs/blob/2eca9b3a53a98ac52efed2a27353c6a6927ead10/vendor/hayro-interpret/assets/FoxitSansBold.pfb | BSD-3-Clause | `7bdfa36de1d5757dcfaf8b8a4be95c6e41c40fa70dbed9d78bf979a7b9959054` | Renders non-embedded standard-14 fonts (pinned hayro-interpret) |
| `hayro-interpret 0.7.0 / assets/FoxitSansBoldItalic.pfb` | Foxit Software Inc. / PDFium Authors | https://github.com/storytold/craft-libs/blob/2eca9b3a53a98ac52efed2a27353c6a6927ead10/vendor/hayro-interpret/assets/FoxitSansBoldItalic.pfb | BSD-3-Clause | `555ce45924338e2e4b32821910221ce85da691fd59a9723c4c971a7c5627fc8b` | Renders non-embedded standard-14 fonts (pinned hayro-interpret) |
| `hayro-interpret 0.7.0 / assets/FoxitSansItalic.pfb` | Foxit Software Inc. / PDFium Authors | https://github.com/storytold/craft-libs/blob/2eca9b3a53a98ac52efed2a27353c6a6927ead10/vendor/hayro-interpret/assets/FoxitSansItalic.pfb | BSD-3-Clause | `7751cbefeda351708dae8c4a88db043411bc39ab2ae2097685da845d4d16f307` | Renders non-embedded standard-14 fonts (pinned hayro-interpret) |
| `hayro-interpret 0.7.0 / assets/FoxitSerif.pfb` | Foxit Software Inc. / PDFium Authors | https://github.com/storytold/craft-libs/blob/2eca9b3a53a98ac52efed2a27353c6a6927ead10/vendor/hayro-interpret/assets/FoxitSerif.pfb | BSD-3-Clause | `4f57d2b9d884af8f907bf22df6019b52d86cbf6214fdbd02b1ae05472a543f35` | Renders non-embedded standard-14 fonts (pinned hayro-interpret) |
| `hayro-interpret 0.7.0 / assets/FoxitSerifBold.pfb` | Foxit Software Inc. / PDFium Authors | https://github.com/storytold/craft-libs/blob/2eca9b3a53a98ac52efed2a27353c6a6927ead10/vendor/hayro-interpret/assets/FoxitSerifBold.pfb | BSD-3-Clause | `0bdf4b04e964139818d51eda03d566ba999fc3ba2421b1c6c51f9dc969022e80` | Renders non-embedded standard-14 fonts (pinned hayro-interpret) |
| `hayro-interpret 0.7.0 / assets/FoxitSerifBoldItalic.pfb` | Foxit Software Inc. / PDFium Authors | https://github.com/storytold/craft-libs/blob/2eca9b3a53a98ac52efed2a27353c6a6927ead10/vendor/hayro-interpret/assets/FoxitSerifBoldItalic.pfb | BSD-3-Clause | `a406cac82583bf98175cb62c87ed5e95c45fbb34eece63a10b0a13e793cb2e10` | Renders non-embedded standard-14 fonts (pinned hayro-interpret) |
| `hayro-interpret 0.7.0 / assets/FoxitSerifItalic.pfb` | Foxit Software Inc. / PDFium Authors | https://github.com/storytold/craft-libs/blob/2eca9b3a53a98ac52efed2a27353c6a6927ead10/vendor/hayro-interpret/assets/FoxitSerifItalic.pfb | BSD-3-Clause | `610ae0687198045c4db5d1a6650fd5e92536631706382eb8b72e38df578d0ae9` | Renders non-embedded standard-14 fonts (pinned hayro-interpret) |
| `hayro-interpret 0.7.0 / assets/FoxitSymbol.pfb` | Foxit Software Inc. / PDFium Authors | https://github.com/storytold/craft-libs/blob/2eca9b3a53a98ac52efed2a27353c6a6927ead10/vendor/hayro-interpret/assets/FoxitSymbol.pfb | BSD-3-Clause | `47967d055530e7357088a08403115425643ec2cdfd6201ba8af0fbd7116c1539` | Renders non-embedded standard-14 fonts (pinned hayro-interpret) |
| `hayro-interpret 0.7.0 / assets/CGATS001Compat-v2-micro.icc` | Clinton Ingram (saucecontrol/Compact-ICC-Profiles) | https://github.com/storytold/craft-libs/blob/2eca9b3a53a98ac52efed2a27353c6a6927ead10/vendor/hayro-interpret/assets/CGATS001Compat-v2-micro.icc | CC0-1.0 | `73e1ba37d2bad5bab2a964f40a9eed96209666efc067c3322626214bbef234a0` | Default CMYK colour conversion (pinned hayro-interpret) |
| `hayro-cmap 0.1.0 / assets/cmaps.brotli` | Adobe | https://github.com/storytold/craft-libs/blob/2eca9b3a53a98ac52efed2a27353c6a6927ead10/vendor/hayro-cmap/assets/cmaps.brotli | BSD-3-Clause | `ddf37af521d904d5646c2fc37b2b7e912dc1b56f0d210190b61f598154758e46` | Character-code mapping tables required to read CJK PDFs. Non-visual dependency data; existing embedding unchanged |
| `hayro-interpret 0.7.0 / src/font/generated/metrics.rs` | Adobe (Core 14 AFM metrics, PDF specification encodings); tables by The Hayro Authors | https://github.com/storytold/craft-libs/blob/2eca9b3a53a98ac52efed2a27353c6a6927ead10/vendor/hayro-interpret/src/font/generated/metrics.rs | MIT OR Apache-2.0 | `2ec9e608573eda459de1e1d9bda3bb494315e36bee885acc88912a11fe0caa41` | Glyph widths and encodings for non-embedded standard fonts. Non-visual dependency data; existing embedding unchanged |
