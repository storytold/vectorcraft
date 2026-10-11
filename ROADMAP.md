# VectorCraft Roadmap

- UI.5: Shared docking adds tab reordering, docking into other groups, dock splits and resizable floating panel groups. Panel positions and return locations persist with workspaces; the native icon rail and Tools panel remain available.

- UI.4: Dock and floating-panel headers use shared accessible tabs with bounded overflow. Widget-state theme application uses `craft-ui`; all four palettes, font stacks and preferences remain app-owned.

**Stage: alpha** · next: beta, ~13 points (ready for real work ~62% → ~75%, and reliable `.ai` exchange) and ~170–270 h away

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-11 · **Change:** minor (Effect › Brush Strokes landed: Photoshop-style filters 28 of 57, raster effects 40% → 48%) · **Target:** Adobe Illustrator 2026 (30.x)

VectorCraft is a clean-room, open-source, pure-Rust reimplementation of the Adobe Illustrator workflow. It runs
on macOS, Windows, Linux, FreeBSD and the web (WASM), and agents can drive all of it over MCP, the CLI and a JSON
control channel. This page is the summary; the evidence is in [`docs/target-app-parity.md`](docs/target-app-parity.md)
and the work list in [`docs/gaps.md`](docs/gaps.md).

**Why alpha:** the core workflows exist end to end (drawing and path editing, Pathfinder, paint and appearance,
type, SVG/PDF/EPS, print, export) and feature depth is ~75%, but a professional can't yet switch: Illustrator's
own `.ai` files open with a wrong layer structure in real cases and can't be saved back with Illustrator's
editing data, the interaction details have never been checked side by side, and some machines fail to start the
app. Beta needs those closed (the beta list in [gaps.md](docs/gaps.md)). It passes the core-workflow
gate: all six core workflows (path drawing and editing, paint and style, type, layers/artboards/export, print
production, file exchange) work end to end on macOS with save and reopen, and only the last is partial, without
blocking it ([Alpha gate](docs/roadmap.md#alpha-gate)).

## Headline numbers

| | Value | Kind |
|---|---|---|
| **Feature breadth** (Illustrator's menu items, tools, effects, formats exist) | **~88%** | measured in part: menu items 348/375 wired (92.8%), tools 89/92, Illustrator effects 44/50, Photoshop effects 28/57, 3D 1/4 (initial Revolve) |
| **Feature depth** (weighted by use, scored by behaviour) | **~75%** | estimated |
| **To beta** | **~170–270 h** one agent · ~50–80 h with 4–6 agents | estimated |
| **To full parity** | **~360–590 h** one agent · ~95–165 h with 4–6 agents | estimated |

**Readiness by audience** (all estimated; method in [target-app-parity.md](docs/target-app-parity.md#ready-for-real-work)):

| Audience | Ready % | Opus 5.5 agent hours to ~95% (one agent) | With 4–6 agents | Work that dominates |
|---|---:|---:|---:|---|
| **Full Illustrator** (ready for real work; decides the stage) | **~62%** (57–67%) | **~340–560 h** | ~95–160 h | the mainstream work below, plus 3D (50–85 h), raster effects (24–40 h), advanced type (20–28 h), generative AI (30–60 h), localization (70–110 h) and ecosystem |
| **Mainstream illustrator** | **~65%** (60–70%) | **~230–380 h** | ~65–110 h | the interaction-fidelity pass (60–90 h), hardening and QA (30–50 h), stability (20–35 h), performance budgets (15–25 h), `.ai` exchange (10–20 h), the remaining depth of the everyday areas |
| **Essentials user** | **~75%** (70–80%) | **~60–105 h** | ~25–45 h | launch and stability on Windows (20–35 h), in-app help and onboarding (8–15 h), basic QA (10–15 h), Shape Builder regions (4–8 h), UI polish |

Hours are Opus 5.5 agent wall-clock hours, calibrated from this repo's merged-PR timestamps (e.g. 0.7–1.2 h per
Photoshop-style filter, ~0.8 h per medium feature); see [calibration](docs/target-app-parity.md#calibration).
The installed Illustrator was **not** inspected: the clean-room rule in [`AGENTS.md`](AGENTS.md) forbids it, so
everything is measured from our code against Illustrator's public documentation.

## By dimension

| Dimension | % | One agent (h) | Doc |
|---|---:|---:|---|
| Features (depth) | 75% | 220–360 | [target-app-parity.md](docs/target-app-parity.md) |
| UI/UX fidelity | 55% | 70–110 | [ui-parity.md](docs/ui-parity.md) |
| File formats | 75% | 15–30 | [file-format-parity.md](docs/file-format-parity.md) |
| Hardware (GPU, pen, touch, displays) | 35% | 25–45 | [hardware-parity.md](docs/hardware-parity.md) |
| Localization (12 key languages) | 54% (measured) | 70–110 | [localization-parity.md](docs/localization-parity.md) |
| Performance | ~60% | 15–25 | [gaps.md › G15](docs/gaps.md#g15-performance-budgets-and-a-gpu-renderer) |
| Stability | ~60% | 30–50 | [gaps.md › G10](docs/gaps.md#g10-launch-and-stability-on-real-machines) |
| Platforms | 80% | 15–25 | [gaps.md › G8](docs/gaps.md#g8-hardening-at-scale) |
| Ecosystem and plug-ins | 15% | 30–60 | [gaps.md › G17](docs/gaps.md#g17-ecosystem-third-party-plug-ins) |
| AI features (generative) | 0% | 30–60 + owner decision | [gaps.md › G18](docs/gaps.md#g18-generative-ai-features) |
| Agent automation | beyond Illustrator | — | [mcp.md](docs/mcp.md) |

The dimension hours overlap (feature rows hold some UI, format and AI work), so they don't add up to the total.

## Features

| Area | % | One agent (h) |
|---|---:|---:|
| Selection, transform & align | 85% | 4–6 |
| Drawing tools | 85% | 8–13 |
| Path operations, Pathfinder, Shape Builder, Live Paint | 80% | 4–8 |
| Colour, swatches, gradients, patterns, mesh, recolor | 92% | 4–7 |
| Strokes, brushes, width profiles | 88% | 8–15 |
| Appearance, transparency, graphic styles, masks | 97% | 1–2 |
| Live vector effects | 85% | 6–10 |
| Raster effects (Effect Gallery) — [effects-parity.md](docs/effects-parity.md) | 48% | 24–40 |
| 3D and Materials | ~8% | 50–85 |
| Type core — [type-parity.md](docs/type-parity.md) | 80% | 14–19 |
| Type advanced (CJK, spelling, Touch Type) | 50% | 20–28 |
| Symbols, blends, envelopes, Repeat, perspective | 85% | 6–10 |
| Image Trace, graphs, image tools | 75% | 5–8 |
| Layers, artboards, document setup | 78% | 5–8 |
| View & navigation | 70% | 9–14 |
| Guides, grids, smart guides, snapping | 80% | 4–7 |
| File formats — [file-format-parity.md](docs/file-format-parity.md) | 75% | 10–20 |
| Export for Screens, Asset Export, slices | 85% | 2–4 |
| Print, colour management, separations | 80% | 2–4 |
| Automation (Actions, Variables, scripting) | 60% | 8–14 |
| UI chrome (panels, Properties, preferences) | 80% | 13–20 |
| Libraries, Links, Package | 82% | 2–3 |
| Generative AI | 0% | 30–60 |

Weights, evidence and the missing items per area: [target-app-parity.md](docs/target-app-parity.md#feature-areas).

## Languages

| Language | Code | Status | Translated |
|---|---|---|---:|
| English | `en` | full | 100% |
| Simplified Chinese | `zh-hans` | partial (messages in English) | 85% |
| Spanish | `es` | full | 100% |
| Hindi | `hi` | none | 0% |
| Arabic | `ar` | none (no right-to-left interface) | 0% |
| French | `fr` | full | 100% |
| Portuguese (Brazil) | `pt-br` | partial | 68% |
| Indonesian | `id` | none | 0% |
| Japanese | `ja` | full | 100% |
| German | `de` | full | 98% |
| Korean | `ko` | none | 0% |
| Vietnamese | `vi` | none | 0% |

Also shipped: Traditional Chinese (partial, 80%), Italian, Russian and Ukrainian (full), Czech (menus only).
No catalog has had a native speaker's review. Details: [localization-parity.md](docs/localization-parity.md).

## Upcoming

Ranked; detail in [docs/roadmap.md](docs/roadmap.md) and [docs/gaps.md](docs/gaps.md).

1. **Interaction-fidelity pass** (G1), tool by tool from [ui-parity.md](docs/ui-parity.md): 60–90 h.
2. **`.ai` files from real users** (G9): layer structure, guides, speed: 10–20 h.
3. **Launch and stability on real machines** (G10): 20–35 h.
4. **Core-tool correctness** (G11: Shape Builder regions, Asset Export borders): 8–15 h.
5. **Raster effects** (G2) alongside: 29 filters and the Effect Gallery, 24–40 h.
6. **Then:** type core and advanced type (G4), the Illustrator 2026 additions (G13), 3D (G3).

## Progress log

| Date | What landed |
|---|---|
| 2026-10-11 | Effect › Brush Strokes: Accented Edges, Angled Strokes, Crosshatch, Dark Strokes, Ink Outlines, Spatter, Sprayed Strokes, Sumi-e (G2). Command parameters from agents and files checked before anything changes (paint, place, cuts, dashes, crop, asset ids, anchors, paths, distort; #984–#999); command dialogs keep whole numbers whole, so Create Object Mosaic takes the tiles typed (#1000); stepper arrows take Shift (×10) and Ctrl/Cmd (÷10) like the arrow keys (#991). Exports and Rasterize supersample Art Optimized edges, so shapes meeting edge to edge leave no seam (#983). Character and paragraph styles take leading, kerning, mojikumi and the other attributes left out at their defaults (#1001). Drawing, typing and pasting with no layer shown and unlocked is refused instead of adding art to a hidden or locked layer (#1002). MCP `add_text` gives type on or in a path the colour asked for (#1003). MCP `screenshot` of a connected app renders the artboard asked for (#1004). `vectorcraft-cli run` ends quietly when its reader closes early (#1005). Select › All on Active Artboard selects on the artboard made active, not always the first (#1006). Reverse Gradient keeps off-centre midpoints, mirrored (#1007). A polar grid with no radial dividers draws none (#1008). Use Precise Cursors also covers the Pen's join pointer (#1009). SVGs with one absolute and one unitless root side import at their size, unstretched (#1010). DXF US survey inches, yards and miles open at their size and name (#1011). A file placed into a document with no other links follows its first change even while the app sits idle (#1031). PDF import keeps the style a font name says when the file doesn't embed the font (#1023). Shift selects ruler guides and art together, and Delete removes both (#1022). Open filled paths are hit by their fill on every side, the rotate cursor bows around the corner it is at, the flip icons match their flips, the Color Picker previews on the selection, and Properties › Transform reaches Scale Corners and Scale Strokes & Effects (#1036). Offset Path and the other booleans finish on outlines where the sweep used to loop until memory ran out (linesweeper 0.5, #1028). Floating panel groups stack into sets that move together, collapse to their tabs and dock as extra dock columns that scroll (#1020). Ruler guides live on layers: listed in the Layers panel, shown, hidden, locked, duplicated, merged and deleted with their layer, and moved between layers (#1021). A `.ai` whose editing data uses global process colours (`Xk`/`XK`) or pattern fills keeps its sublayers: the colours come in as global swatches at their tint, the patterns as pattern swatches (#1025). A `.ai` in PostScript form opens through its layers, read from the program itself, also when it names the prolog defining its operators without including it, as Rhino and other CAD apps write it (#1027). A fill or stroke chosen while the Type tool has characters selected colours just those characters (#1063). Scaled type shows its scaled font size, leading, baseline shift and horizontal scale in the Character panel, and a size typed there is the size you get (#1034). Character panel menu › Proportional Metrics sets Japanese and Chinese full-width glyphs on the font's proportional widths in horizontal type (#966). Align, Distribute, Move, Arrange, Lock, Hide, Ungroup and other Object commands that would change nothing no longer record an undo step or mark the document changed (#1060). Spot colours defined in Lab open with their Lab values from `.ai` and PDF files, instead of as wrong RGB colours or their CMYK equivalents (#1032). Build scripts leave a generated source they wouldn't change untouched, so the next build doesn't compile most of the workspace again where file times lose their sub-second part (#1051). A fill colour recolours the gradient mesh point last clicked with the Mesh tool or Direct Selection, and a Mesh tool click on a mesh's point selects that mesh (#1052). The All Tools drawer stays two tools wide instead of filling a wide window (#1054). Text Wrap's Make, Release and Options refuse a list of object ids with a malformed or unknown id instead of skipping it (#1055). A freeform gradient line with a malformed point index is refused instead of being shortened to the indices that are valid (#1056). The Corners dialog (`ui.corners`) refuses a malformed object id or a listed anchor that isn't a corner instead of editing the selection or the other corners (#1057). New View refuses a centre that isn't exactly [x, y] and a rotation that isn't a number, instead of trimming the one and saving the other as 0° (#1058). The command sweeps try junk scale, spacing and artboard values too, which they had skipped since before those bugs were fixed (#1059). EPS and PostScript `arc` and `arcn` with huge angles no longer hang the import, and an arc whose angles are whole turns apart is empty, as the PostScript reference defines it (#1062). Align and Distribute Spacing keep the key object still also when it is inside a selected group or compound path (#1074). A `.ai` whose layers can't be read yet still opens with its artboards where they were (in a grid, say), not in a row (#1068). Layer Options (`ui.layerOptions`) refuses a malformed or unknown row id instead of skipping it or opening on the highlighted rows (#1077). Shaper scribbles given a list of object ids refuse a malformed or unknown id instead of skipping it and editing the shapes that remain (#1078). Delete Width Point (`stroke.widthPoint.remove`) refuses a list of indices with a malformed entry instead of deleting the points that remain, and no longer wraps a huge index round on the web (#1079). Preferences' checkboxes have an outline when the pointer isn't over them, in every interface brightness, as the app's other dialogs' checkboxes do (#1080). The last plain checkboxes (Properties panel snapping and Scale Strokes & Effects, Revolve, Save options, Export for Screens, Variables, plug-in forms) keep a visible box at rest in every theme too (#885). |
| 2026-10-10 | Initial live Revolve (Effect › 3D and Materials, #846, #605), in its own `three-d` crate. Progress docs re-measured and restructured to the craftrules standard (this page, `docs/target-app-parity.md`, `gaps.md`, `ui-parity.md`, the format, hardware, localization, effects and type checklists). Same day: v0.8.0; Effect › Distort, Pixelate and Texture filters (13); German interface; Variables (data merge); Graph Type value axes and tick marks; restart when the first frame never reaches the screen (#964) |
| 2026-10-09 | Radial Blur, Smart Blur, Unsharp Mask (gap 2 begins); 340 commits, the busiest day |
| 2026-10-08 | v0.5.0–v0.7.0; community contributions: Pen and shape-tool modifiers, Layers, Artboards, Japanese composition, Hebrew/Arabic type, interface languages, saved selections |
| 2026-10-07 | v0.4.0; issue fixes; Layers panel rework |
| 2026-10-06 | M8 transform and distort fidelity pass (M8.1–M8.20) |
| 2026-10-05 | M4 files done (M4.14–M4.98); M3 paint and appearance done (92 tasks over two days) |
| 2026-10-02 | First weighted parity estimate (22 areas); look and feel measured against public screenshots |
| 2026-10-01 | Renamed from DrawCraft to VectorCraft |
| 2026-09-30 | Repository history begins |

The full pre-standard record of what landed is kept in [docs/roadmap.md](docs/roadmap.md#progress-log-before-the-progress-docs-standard).

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-10 | minor | Method aligned with the standard, no new evidence: full ready for real work is the additive weighted sum, ~62%; hours per audience; beta ~13 points away |
| 2026-10-10 | minor | Added the essentials-user readiness score (~75%) |
| 2026-10-10 | minor | Ready for real work re-examined against the issue tracker and user feedback: 50% → 55% (full), 65% mainstream added; see target-app-parity.md |
| 2026-10-10 | minor | Applied the core-workflow gate (docs/roadmap.md › Alpha gate): passes, stage stays alpha |
| 2026-10-10 | major | Re-measured against Illustrator 2026 (30.x) from code counts, 71 open issues and public docs; stage set to alpha; moved the parity estimate, honest assessment and "Shipped so far" to `docs/target-app-parity.md`, the gap list to `docs/gaps.md`, milestones to `docs/roadmap.md`, the `.ai` scope to `docs/file-format-parity.md` |
| 2026-10-05 | major | Honest assessment by dimension and the prioritized gap list |
| 2026-10-02 | major | First 22-area weighted parity estimate |
