<p align="center">
  <img src="assets/app-icon/hicolor/128x128/apps/ai.storyteller.vectorcraft.png" alt="Vector W3K2 app icon: PT, a black P and a cyan T in an orange frame" width="96">
</p>

<h1 align="center">Vector W3K2</h1>

<p align="center">
  <b>Offline vector illustration by Print That 204.</b><br>
  An Illustrator-style editor in pure Rust for signs, decals, vinyl, print-and-cut and CAD-ready artwork.
</p>

<p align="center">
  <a href="https://printthat.ca"><b>printthat.ca</b></a> ·
  <a href="#download">Download</a> ·
  <a href="#whats-new-in-vector-w3k2">What's new</a> ·
  <a href="#about-print-that-204">About Print That 204</a>
</p>

## About Print That 204

**Print That 204** is a Canadian 3D-printing and custom manufacturing studio in Winnipeg, Manitoba.
We make vendor booth signs, custom signs and displays, stickers and decals, printed vinyl, laminated
graphics, scale-model accessories, prototypes and small-batch custom products, with FDM and resin 3D
printing, custom CAD, 3D scanning and print-and-cut.

Vector W3K2 is the vector editor we build and use for that work: it runs fully offline, opens and
writes the files a print shop deals with (PDF, SVG, EPS, DXF, the Illustrator-compatible `.ai` PDF
part), and handles spot colours and colour books for production.

- Web: [printthat.ca](https://printthat.ca)
- Email: [printthat204@gmail.com](mailto:printthat204@gmail.com)
- Winnipeg, Manitoba, Canada

## Download

| Platform | Get it | Notes |
|---|---|---|
| Windows 10/11 (x64) | `vector-w3k2-<version>-windows-x64-setup.exe` | Installs for you only (no admin needed) or for all users; Start Menu shortcut, optional desktop shortcut and `.vectorcraft` file association |
| Windows, portable | `vector-w3k2-<version>-windows-x64-portable.zip` | Unzip anywhere (USB stick too) and run `Vector W3K2.exe`; nothing to install |
| macOS 11+ (Apple silicon and Intel) | `vector-w3k2-<version>-macos-universal.dmg` | Drag Vector W3K2 to Applications. Not notarized: the first time, right-click the app and choose Open |

The builds come from the [Vector W3K2 build](.github/workflows/vector-w3k2-build.yml) workflow: open the
repository's **Actions** tab, pick the latest run, and download the files at the bottom. To make new ones,
push a tag that starts with `vw3k2-v` (for example `vw3k2-v0.3.2`). To build the Windows installer on a
PC, see [Building](#building).

## What's new in Vector W3K2

Vector W3K2 adds the non-AI features
from Illustrator's 2025–2026 releases and the fixes Print That asked for (the full list, release by
release, is in [`docs/illustrator-2026-updates.md`](docs/illustrator-2026-updates.md)):

- **Gradients:** dithering against banding and perceptual (OKLab) blending.
- **Artboards:** background colours, a clear active artboard, renaming on the canvas, a right-click
  menu (Duplicate, Rename, Lock, Export, Delete) and locking an artboard with its art.
- **Snapping:** segment midpoints, tangent and perpendicular snapping with a 90° corner mark, 0°/45°/90°
  angle guides for lines and the Pen, magenta labels and a marker while hovering, and a Snapping popover.
- **Selection:** Selection (V) is the black arrow and Direct Selection (A) the white arrow, as in
  Illustrator. The white arrow picks single points and single edges by click or by dragging a box,
  shows the line or point under the pointer bolder, and pulses a picked line gently so it reads as
  selected. Delete removes just the picked edges: a box opens there and an open line splits.
- **Editing lines:** dragging an end of a picked line moves only that end, so the line swings around
  its other end. Dragging an open end onto another snaps on with a magenta "join" mark and joins
  them, closing the shape when both ends belong to one line. Only closed shapes are filled: lines
  from the Line and Pen tools have no fill, and deleting an edge of a filled box leaves the lines
  with their stroke while the old fill stays as its own shape.
- **Corners:** a corner radius field in the Essentials, Essentials Classic and Print and Proofing
  workspaces, four corner fields for rounding each corner on its own, and the white arrow rounds just
  the corners you pick.
- **Panels:** drag any panel by its name tab to float it anywhere, lock it beside the toolbar
  (padlock or drop it there) or put it back in the dock (× or drop it there). Positions are kept
  between sessions.
- **Blends:** a Blend panel with step easing and separate colour easing.
- **Colour:** swatch libraries from `.ase`, `.acb` colour books and `.aco` files (spot colour books
  load as spot colours); Recent Colors keep their spot swatch and show its name.
- **Cutter registration marks:** File › Registration › Summa (OPOS marks, OPOS XY with a bar along the bottom, OPOS XY 2 with bars top and bottom, and OPOS Random XY
  for older Summas: a round mark in each corner and an arrow pointing left) and File › Registration › Zünd (five registration dots, the fifth above the bottom-left one to
  show the registration corner), built in with no plug-in. The marks go around the selection
  or all the art, on a layer of their own, with a live preview and sizes kept inside each system's range.
- **Dialogs:** Move, Scale and the other dialogs open next to the pointer at a compact size and can be
  dragged anywhere, where they stay.
- **Smaller things:** Relative/Absolute scaling, relinking every copy of an image and the missing files
  beside it.

Generative AI tools are intentionally left out: Vector W3K2 never needs an account or a connection.

> [!NOTE]
> Internal names (the crates, the `.vectorcraft` file format and the preferences folder) stay
> `vectorcraft`, so files and settings from earlier versions keep working in Vector W3K2.

<p align="center">
  <img src="docs/images/shot-1-neon.png" alt="The editor with the Neon Drive poster open: the title is selected, the Appearance panel shows its live Outer Glow, and the Properties panel shows its character settings" width="100%">
</p>

## A look around

<table>
<tr>
<td width="50%" valign="top">
  <img src="docs/images/shot-2-ribbons.png" alt="Three live blend ribbons of 55 to 70 steps with smooth colour, clipped to the artboard, with the Layers panel open" width="100%">
  <p align="center"><sub><b>Live Blends</b>: editable key paths and smooth colour, clipped to the artboard</sub></p>
</td>
<td width="50%" valign="top">
  <img src="docs/images/shot-4-bezier.png" alt="Direct Selection tool showing anchor points and Bézier handles on a crescent built with Pathfinder, with the contextual task bar below it" width="100%">
  <p align="center"><sub><b>Pen and Direct Selection</b>: real Bézier anchors and handles, plus a contextual task bar</sub></p>
</td>
</tr>
<tr>
<td colspan="2" valign="top">
  <img src="docs/images/shot-3-sheet.png" alt="Four artboards in the light UI theme: Pathfinder, Gradient Mesh, radial Repeat and Envelope Distort" width="100%">
  <p align="center"><sub><b>Multiple artboards, light theme</b>: Pathfinder · Gradient Mesh · live radial Repeat · Envelope Distort · <code>examples/feature-sheet.vectorcraft</code></sub></p>
</td>
</tr>
</table>

## Made in Vector W3K2

Every piece below was built entirely through Vector W3K2's command API, the same one the MCP server
exposes to agents, and exported by Vector W3K2's own renderer. The source files are in
[`examples/`](examples).

<table>
<tr>
<td width="50%" valign="top">
  <img src="docs/images/art-neon-drive.png" alt="Neon Drive synthwave poster: a striped orange sun setting between purple mountains over a glowing pink grid" width="100%">
  <p align="center"><sub><b>Neon Drive</b>: synthwave poster with glowing type and grid</sub></p>
</td>
<td width="50%" valign="top">
  <img src="docs/images/dusk-poster.png" alt="Dusk poster: a gradient sky with stars and birds, a glowing sun behind layered purple mountains and pine trees" width="100%">
  <p align="center"><sub><b>Dusk</b>: gradient sky, glowing sun, layered mountains</sub></p>
</td>
</tr>
<tr>
<td colspan="2" valign="top">
  <img src="docs/images/art-ribbons.png" alt="Live Blends: three wide ribbons blending yellow to pink and teal to purple, crossing over a dark background" width="100%">
  <p align="center"><sub><b>Live Blends</b>: 70 steps, smooth colour, editable spines</sub></p>
</td>
</tr>
<tr>
<td width="50%" valign="top">
  <img src="docs/images/art-repeat.png" alt="Radial Repeat mandala: teal petals around a coral center, ringed by yellow dots" width="100%">
  <p align="center"><sub><b>Radial Repeat</b>: a live mandala</sub></p>
</td>
<td width="50%" valign="top">
  <img src="docs/images/art-envelope.png" alt="Envelope Distort: rainbow stripes and the word WARP bent into a waving flag" width="100%">
  <p align="center"><sub><b>Envelope Distort</b>: striped type warped into a flag</sub></p>
</td>
</tr>
<tr>
<td width="50%" valign="top">
  <img src="docs/images/art-mesh.png" alt="Gradient Mesh: three softly shaded purple, pink and cyan spheres on a dark background" width="100%">
  <p align="center"><sub><b>Gradient Mesh</b>: shaded spheres</sub></p>
</td>
<td width="50%" valign="top">
  <img src="docs/images/art-pathfinder.png" alt="Pathfinder: a purple gradient crescent moon and three orange stars on a peach background" width="100%">
  <p align="center"><sub><b>Pathfinder</b>: a crescent and stars from exact booleans</sub></p>
</td>
</tr>
</table>

## Why Vector W3K2

- **Familiar.** Illustrator's layout, tools, menus, panels and shortcuts: the Pen, Direct Selection,
  Pathfinder, Smart Guides, Appearance, Swatches, Layers and more. You already know how to use it.
- **Fast.** Multithreaded SIMD rendering off the UI thread. 20,000 shapes render in about 27 ms at
  full retina resolution while the interface stays at 120 fps.
- **Robust.** Exact curve booleans (no "cannot perform operation"), unlimited undo via structural
  sharing, and property-tested file round trips.
- **Open.** A documented native format (`.vectorcraft`, JSON), first-class SVG, PDF (and
  PDF-compatible `.ai`) import and export, PNG/JPEG/WebP export, Export for Screens, and SVGZ,
  templates, GIF, TIFF and BMP on open.
- **Agent-native.** Every menu item, tool gesture, panel and dialog can be driven over a JSON
  control channel and an **MCP server**, so Claude and other agents can draw, edit and export the
  way a person does.
- **Everywhere.** One codebase for the desktop apps and the same UI in the browser.

## Building

You need [Rust](https://rustup.rs) 1.95 or newer.

```sh
cargo run --release -p vectorcraft                          # desktop app
cargo run --release -p vectorcraft -- examples/dusk-poster.vectorcraft
cargo run --release -p vectorcraft -- --control 7979        # + JSON control channel
cargo run --release -p vectorcraft-cli -- mcp               # MCP server (stdio)
cargo run --release -p vectorcraft-cli -- run --in examples/ribbons.vectorcraft --export out.pdf   # headless batch
cargo run --release -p vectorcraft-cli -- bench examples/neon-drive.vectorcraft                   # render timing
cargo xtask bundle                                        # dist/Vector W3K2.app (macOS)
pwsh packaging/windows/build-installer.ps1                # Windows installer + portable zip (needs Inno Setup 6)
cd apps/vectorcraft-web && trunk build --release            # web build → dist/web
cargo xtask ci                                            # fmt, clippy, tests, layering, wasm, vendor names
```

On Windows the official builds use the MSVC toolchain. The GNU toolchain works too
(`rustup default stable-x86_64-pc-windows-gnu`) with a full MinGW-w64 install on the `PATH` (for example
WinLibs) and `CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-C link-self-contained=no"`.

Japanese fonts come from [storytold/craft-fonts](https://github.com/storytold/craft-fonts), an optional
build input (releases always use it): `CRAFT_FONTS_DIR="$PWD/../craft-fonts" cargo run --release -p vectorcraft` (an absolute path).
Without it, Japanese text uses the installed system fonts. See [`docs/development.md`](docs/development.md#fonts-craft-fonts-optional-build-input).

### Use it from Claude Code and other agents

Register the MCP server with Claude Code:

```sh
claude mcp add vectorcraft -- /path/to/vectorcraft-cli mcp
```

The details are in [`docs/mcp.md`](docs/mcp.md) and [`docs/control-protocol.md`](docs/control-protocol.md).

## Status

Vector W3K2 is under active development. [**ROADMAP.md**](ROADMAP.md) covers what ships today, the
milestones, and honest time-to-parity estimates.

**Where we are (2026-10-06):** roughly 69–75% of Illustrator's features exist and work, and about 40–55% of
"a power user can't tell the difference". Everyday vector illustration is close to usable: drawing and path tools,
Pathfinder and Shape Builder, paint, gradients, appearance and transparency, type with styles and threading, and
files (SVG, PDF and PDF-compatible `.ai` with PDF/X, EPS, DXF, EMF/WMF, raster formats and PSD, Print, Package). The scores are
self-assessed, so the [honest assessment](ROADMAP.md#honest-assessment-2026-10-05) explains how far to trust them.

**What's missing:**
- 3D and Materials;
- the Photoshop-style raster effects (Effect Gallery);
- CJK composition for vertical type (vertical type itself has initial support, with a Japanese interface);
- Variables and scripting;
- an interaction-fidelity pass covering every tool's modifiers and small behaviours;
- packaging for Windows and Linux.

**Where we're going:** next is the interaction-fidelity pass alongside the raster-effects package, then 3D and
advanced type, then hardening and packaging for 1.0. The prioritized list is in
[Where we're lacking](ROADMAP.md#where-were-lacking-in-priority-order).

**Workspace:** `crates/{geom, color, doc, pathops, text, effects, trace, brush, render, svg, pdf, eps, cad, metafile, format, tools, engine, ui-egui, mcp, testkit}`
and `apps/{vectorcraft, vectorcraft-cli, vectorcraft-web}`. The egui frontend is its own crate, so
the UI can be swapped without touching the engine.

Agent and contributor rules (clean-room, the asset policy, no panics in shipped code, quality gates) are in
[`AGENTS.md`](AGENTS.md); [`docs/development.md`](docs/development.md#robustness-vector-w3k2-never-crashes)
explains how Vector W3K2 avoids crashing, and [`docs/releasing.md`](docs/releasing.md) how releases are built,
signed and published. Every bundled asset is listed with its licence in [`ASSETS.md`](ASSETS.md).

## License and credits

Vector W3K2 is licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
Copyright (c) 2026 Print That 204. Notices are in [NOTICE](NOTICE).

Bundled fonts, icons, images and other assets keep their own open licenses; each one is listed
with its author, source and license in [ASSETS.md](ASSETS.md). Release builds also embed the
Japanese fonts of [craft-fonts](https://github.com/storytold/craft-fonts/blob/main/ATTRIBUTION.md)
(SIL Open Font License 1.1).

The Vector W3K2 app icon ("PT": a black P and a cyan T) is Print That 204's original
artwork; its palette and files are in [`assets/app-icon/`](assets/app-icon/README.md).

"Print That 204", "Print That" and Vector W3K2 are Print That 204's names.

<sub>Adobe, Photoshop, Illustrator, Premiere Pro, Lightroom, Acrobat, After Effects and InDesign are trademarks or registered trademarks of Adobe Inc. in the United States and/or other countries. Vector W3K2 is an independent project and is not affiliated with, sponsored by or endorsed by Adobe Inc.; these names are used only to describe the workflows it is compatible with.</sub>


