# Asset attribution

Every non-code asset in this repository (icons, images, fonts, example art, colour profiles, presets) is listed here with its author, source and licence. `cargo xtask assets` (part of `cargo xtask ci`) fails if an asset file is missing from this table.

**Policy (mandatory):** Vector W3K2 contains **no Adobe iconography, images, artwork, presets, swatch/brush/symbol/pattern libraries or ICC profiles**. Every asset is either original work by Vector W3K2 contributors, or third-party material under an open licence (OSI open source, public domain / CC0, or Creative Commons that allows redistribution), and it is attributed below. Screenshots of Adobe software are never committed.

Generated-in-code art is original and has no file to list. This covers the default swatches, brushes, symbols, patterns, graphic styles, image-trace presets and the vector tool cursors (`crates/ui-egui/src/cursors.rs`).

**Font files are never committed here.** Fonts shared by the Crafting Apps live in [storytold/craft-fonts](https://github.com/storytold/craft-fonts) (rules: craftrules [`standards/fonts.md`](https://github.com/storytold/craftrules/blob/main/standards/fonts.md)). The small Latin UI/document fonts below predate that rule and stay.

**Optional build input: craft-fonts.** Builds made with `CRAFT_FONTS_DIR=<craft-fonts checkout>` (all official releases) embed the Japanese fonts listed in its [`ATTRIBUTION.md`](https://github.com/storytold/craft-fonts/blob/main/ATTRIBUTION.md): BIZ UDPGothic Regular and Bold, Shippori Mincho Regular and BIZ UDMincho Regular (web builds: BIZ UDPGothic Regular only), all under the SIL Open Font License 1.1. They are not files in this repository; release packages carry each one's `OFL-<family>.txt`. Builds without `CRAFT_FONTS_DIR` embed none of them.

| Asset | Author | Source | Licence | Notes |
|---|---|---|---|---|
| `assets/app-icon/LICENSE.txt` | (licence/readme text) | Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/app-icon/README.md` | (licence/readme text) | Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/app-icon/hicolor/128x128/apps/ai.storyteller.vectorcraft.png` | Print That 204 | Original artwork for Vector W3K2 ("PT": a black P and a Print That cyan T on a white tile with an orange frame), drawn as SVG and rendered by `vectorcraft-cli convert` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (Vector W3K2) |
| `assets/app-icon/hicolor/16x16/apps/ai.storyteller.vectorcraft.png` | Print That 204 | Original artwork for Vector W3K2 ("PT": a black P and a Print That cyan T on a white tile with an orange frame), drawn as SVG and rendered by `vectorcraft-cli convert` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (Vector W3K2) |
| `assets/app-icon/hicolor/24x24/apps/ai.storyteller.vectorcraft.png` | Print That 204 | Original artwork for Vector W3K2 ("PT": a black P and a Print That cyan T on a white tile with an orange frame), drawn as SVG and rendered by `vectorcraft-cli convert` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (Vector W3K2) |
| `assets/app-icon/hicolor/256x256/apps/ai.storyteller.vectorcraft.png` | Print That 204 | Original artwork for Vector W3K2 ("PT": a black P and a Print That cyan T on a white tile with an orange frame), drawn as SVG and rendered by `vectorcraft-cli convert` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (Vector W3K2) |
| `assets/app-icon/hicolor/32x32/apps/ai.storyteller.vectorcraft.png` | Print That 204 | Original artwork for Vector W3K2 ("PT": a black P and a Print That cyan T on a white tile with an orange frame), drawn as SVG and rendered by `vectorcraft-cli convert` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (Vector W3K2) |
| `assets/app-icon/hicolor/48x48/apps/ai.storyteller.vectorcraft.png` | Print That 204 | Original artwork for Vector W3K2 ("PT": a black P and a Print That cyan T on a white tile with an orange frame), drawn as SVG and rendered by `vectorcraft-cli convert` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (Vector W3K2) |
| `assets/app-icon/hicolor/512x512/apps/ai.storyteller.vectorcraft.png` | Print That 204 | Original artwork for Vector W3K2 ("PT": a black P and a Print That cyan T on a white tile with an orange frame), drawn as SVG and rendered by `vectorcraft-cli convert` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (Vector W3K2) |
| `assets/app-icon/hicolor/64x64/apps/ai.storyteller.vectorcraft.png` | Print That 204 | Original artwork for Vector W3K2 ("PT": a black P and a Print That cyan T on a white tile with an orange frame), drawn as SVG and rendered by `vectorcraft-cli convert` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (Vector W3K2) |
| `assets/app-icon/hicolor/scalable/apps/ai.storyteller.vectorcraft.svg` | Print That 204 | Original artwork for Vector W3K2 ("PT": a black P and a Print That cyan T on a white tile with an orange frame), drawn as SVG and rendered by `vectorcraft-cli convert` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (Vector W3K2) |
| `assets/app-icon/vectorcraft-1024.png` | Print That 204 | Original artwork for Vector W3K2 ("PT": a black P and a Print That cyan T on a white tile with an orange frame), drawn as SVG and rendered by `vectorcraft-cli convert` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (Vector W3K2) |
| `assets/app-icon/vectorcraft-macos-512.png` | Print That 204 | Original artwork for Vector W3K2 ("PT": a black P and a Print That cyan T on a white tile with an orange frame), drawn as SVG and rendered by `vectorcraft-cli convert` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (Vector W3K2) |
| `assets/app-icon/vectorcraft-small.svg` | Print That 204 | Original artwork for Vector W3K2 ("PT": a black P and a Print That cyan T on a white tile with an orange frame), drawn as SVG and rendered by `vectorcraft-cli convert` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (Vector W3K2) |
| `assets/app-icon/vectorcraft.icns` | Print That 204 | Original artwork for Vector W3K2 ("PT": a black P and a Print That cyan T on a white tile with an orange frame), drawn as SVG and rendered by `vectorcraft-cli convert` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (Vector W3K2) |
| `assets/app-icon/vectorcraft.ico` | Print That 204 | Original artwork for Vector W3K2 ("PT": a black P and a Print That cyan T on a white tile with an orange frame), drawn as SVG and rendered by `vectorcraft-cli convert` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (Vector W3K2) |
| `assets/app-icon/vectorcraft.svg` | Print That 204 | Original artwork for Vector W3K2 ("PT": a black P and a Print That cyan T on a white tile with an orange frame), drawn as SVG and rendered by `vectorcraft-cli convert` | MIT OR Apache-2.0 (`assets/app-icon/LICENSE.txt`) | App icon (Vector W3K2) |
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
| `assets/icons/dc-actions.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-al-bottom.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-al-hcenter.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-al-left.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-al-right.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-al-top.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-al-vcenter.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-align.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-alignto-artboard.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-alignto-key.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-alignto-selection.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-anchor.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-appearance.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-arc.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-arrow-down.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-arrow-extend.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-arrow-tip.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-arrow-up.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-artboard-options.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-artboards.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-blend.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-bloat.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-cap-butt.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-cap-round.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-cap-square.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-center-hide.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-center-show.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-clear.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-color-guide.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-crystallize.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-cube.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-dash-align.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-dash-exact.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-dir-off.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-dir-on.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-direct.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-dist-bottom.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-dist-hcenter.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-dist-hspace.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-dist-left.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-dist-right.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-dist-top.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-dist-vcenter.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-dist-vspace.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-draw-behind.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-draw-inside.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-draw-normal.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-ellipse.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-fill-none.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-flare.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-folder.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-free-transform.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-fx.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-gamut.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-grad-freeform.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-grad-linear.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-grad-radial.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-grad-stroke-across.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-grad-stroke-along.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-grad-stroke-within.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-gradient.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-graphic-styles.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-grid-view.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-group-select.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-join-bevel.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-join-miter.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-join-round.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-join.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-knife.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-line-cut.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-line.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-list-view.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-live-bucket.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-live-select.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-mask-none.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-measure.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-mesh.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-mirror-cut.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-new-fill.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-new-item.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-new-stroke.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-options.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-para-center.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-para-justify-all.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-para-justify-center.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-para-justify-left.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-para-justify-right.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-para-left.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-para-right.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-path-eraser.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-pathfinder.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-pen-add.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-pen-delete.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-perspective.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-pf-crop.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-pf-divide.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-pf-exclude.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-pf-intersect.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-pf-merge.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-pf-minus-back.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-pf-minus-front.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-pf-outline.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-pf-trim.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-pf-unite.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-place-symbol.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-polar-grid.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-polygon.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-pucker.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-puppet.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-rearrange.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-rect-cut.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-rect-grid.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-reference-point.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-remove-brush.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-reshape.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-reverse.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-rotate-view.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-rounded-rect.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-rule-evenodd.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-rule-nonzero.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-scallop.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-screen-mode.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-selection.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-shape-builder.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-shear.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-smooth.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-stroke-center.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-stroke-inside.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-stroke-outside.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-stroke.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-swap.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-swatch-kinds.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-symbol-sprayer.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-touch-type.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-transform-panel.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-transparency.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-twirl.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-type-area.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-type-path.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-type-vertical.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-width-profile.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-width.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-wrinkle.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-zoom-large.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
| `assets/icons/dc-zoom-small.svg` | Vector W3K2 contributors | Original work, drawn for Vector W3K2 | MIT OR Apache-2.0 |  |
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
| `docs/images/dusk-poster.png` | Vector W3K2 contributors | Rendered by Vector W3K2 from `examples/dusk-poster.vectorcraft` | MIT OR Apache-2.0 |  |
| `examples/dusk-poster.vectorcraft` | Vector W3K2 contributors | Original artwork built through the Vector W3K2 command API | MIT OR Apache-2.0 |  |
| `examples/dusk-poster.svg` | Vector W3K2 contributors | SVG export of the above | MIT OR Apache-2.0 |  |
| `docs/images/shot-1-neon.png` | Vector W3K2 contributors | Screenshot of Vector W3K2 itself (Vector W3K2/Lucide UI icons only), editing `examples/neon-drive.vectorcraft` | MIT OR Apache-2.0 |  |
| `docs/images/shot-2-ribbons.png` | Vector W3K2 contributors | Screenshot of Vector W3K2 itself, editing `examples/ribbons.vectorcraft` | MIT OR Apache-2.0 |  |
| `docs/images/shot-3-sheet.png` | Vector W3K2 contributors | Screenshot of Vector W3K2 itself, editing `examples/feature-sheet.vectorcraft` | MIT OR Apache-2.0 |  |
| `docs/images/shot-4-bezier.png` | Vector W3K2 contributors | Screenshot of Vector W3K2 itself, editing `examples/feature-sheet.vectorcraft` | MIT OR Apache-2.0 |  |
| `docs/images/art-neon-drive.png` | Vector W3K2 contributors | Rendered by Vector W3K2 from `examples/neon-drive.vectorcraft` | MIT OR Apache-2.0 |  |
| `docs/images/art-ribbons.png` | Vector W3K2 contributors | Rendered by Vector W3K2 from `examples/ribbons.vectorcraft` | MIT OR Apache-2.0 |  |
| `docs/images/art-pathfinder.png` | Vector W3K2 contributors | Rendered by Vector W3K2 from `examples/feature-sheet.vectorcraft` (artboard 1) | MIT OR Apache-2.0 |  |
| `docs/images/art-mesh.png` | Vector W3K2 contributors | Rendered by Vector W3K2 from `examples/feature-sheet.vectorcraft` (artboard 2) | MIT OR Apache-2.0 |  |
| `docs/images/art-repeat.png` | Vector W3K2 contributors | Rendered by Vector W3K2 from `examples/feature-sheet.vectorcraft` (artboard 3) | MIT OR Apache-2.0 |  |
| `docs/images/art-envelope.png` | Vector W3K2 contributors | Rendered by Vector W3K2 from `examples/feature-sheet.vectorcraft` (artboard 4) | MIT OR Apache-2.0 |  |
| `examples/neon-drive.vectorcraft` | Vector W3K2 contributors | Original artwork built through the Vector W3K2 command API | MIT OR Apache-2.0 |  |
| `examples/ribbons.vectorcraft` | Vector W3K2 contributors | Original artwork built through the Vector W3K2 command API | MIT OR Apache-2.0 |  |
| `examples/feature-sheet.vectorcraft` | Vector W3K2 contributors | Original artwork built through the Vector W3K2 command API | MIT OR Apache-2.0 |  |
