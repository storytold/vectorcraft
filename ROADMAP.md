# Vector W3K2 Roadmap

Vector W3K2 is a clean-room, open-source, pure-Rust reimplementation of the Adobe Illustrator workflow. It runs on macOS, Windows, Linux and the web (WASM), and agents can drive it fully over a JSON control channel and MCP.

This file tracks **how far we are and what's left**. Time estimates are wall-clock hours of continuous Claude Opus 5.5 agent work (including builds and the CI gate), given both for **one agent** and for **4–6 parallel agents** on disjoint crates. They are counted from the remaining work (see [Parity estimate](#parity-estimate)), calibrated against measured throughput, and updated as work lands.

_Last updated: 2026-10-07 (after M4.14–M4.98, M8.1–M8.20, M14.4–M14.7, the MCP prompts, resource templates, completions and logging, and the 2026-10-07 issue fixes; see [Honest assessment](#honest-assessment-2026-10-05))._

## Where we are

| Dimension | Status |
|---|---|
| Infrastructure (engine, command registry, history, render, formats, MCP, web, packaging, tests) | **~90%** |
| Look & feel vs Illustrator 2026 default workspace (measured) | **~75–80%** |
| Feature surface vs full Illustrator (weighted, see below) | **~75%** on the rubric · **69–75%** honest range (self-graded, see [Honest assessment](#honest-assessment-2026-10-05)) |
| Parity including interaction fidelity and hardening ("a power user can't tell the difference, but faster") | **~40–55%** |
| Time to **feature parity** (every menu item, tool, panel, effect and dialog functional) | **~210–330 h** one agent · **~55–95 h** with 4–6 agents |
| Time to **full parity** (feature parity + interaction-fidelity pass + hardening) | **~320–500 h** one agent · **~80–145 h** with 4–6 agents |

## Honest assessment (2026-10-05)

**In one line:** about two-thirds to three-quarters of Illustrator's features exist and work, and for everyday
vector illustration (drawing, paths, paint and appearance, SVG/PDF/print) Vector W3K2 is close to usable. Measured
against "a power user can't tell the difference", it is closer to 40–55%. What stands between here and "anyone can
switch" is a handful of large subsystems plus a fidelity pass, not a long tail of small fixes.

**How much to trust the numbers:** area scores are graded by the agents that built the features, from behaviour docs
(`plan/illustrator/`) and tests, not by side-by-side use of Illustrator. Treat them as upper bounds. Look & feel is
the only dimension measured against the reference app. Performance budgets haven't been re-run on an idle machine
since 2026-10-01.

### By dimension

| Dimension | Estimate | Evidence and what's missing |
|---|---|---|
| **Breadth:** menus, tools and panels exist | ~90% | 16 menu items still stubbed; every tool implemented except Touch Type; 51 panel modules |
| **Depth:** each feature behaves like Illustrator | ~69–75% | Strong: paint, appearance and colour (M3), Pathfinder and booleans, selection, drawing, files (M4). Weak: advanced type (~45%), brushes and symbols (in progress), raster effects (~20%) |
| **Large missing subsystems** | 0–20% | 3D & Materials (0%), Photoshop-style raster effects and the Effect Gallery (~1 of ~56 filters), SVG Filters, CJK composition (vertical type with kinsoku and tate-chu-yoko; ruby, mojikumi and vertical metrics open), Variables (data merge), scripting |
| **Interaction fidelity:** modifiers, cursors, small behaviours | ~30–40% | The dedicated pass hasn't started, and there has been no side-by-side session with Illustrator yet. A power user notices this first |
| **Look & feel** | ~75–80% | Measured against Illustrator 2026 screenshots (2026-10-02); the panels added since haven't been re-measured |
| **File interop** | ~85% | SVG/SVGZ, PDF and PDF-compatible `.ai`, EPS, DXF, EMF/WMF, raster formats, PSD export, Place and Links, Package, Print, clipboard flavours. Native `.ai` private data is out of scope by design; DWG has no open spec |
| **Bundled content:** brush, symbol, style and swatch libraries | ~30% | Ours are original and generated in code, and far fewer than Illustrator ships; brush and symbol libraries are still missing |
| **Performance** | unverified | Multithreaded, off-thread rendering and caches are in place. The last budget run (2026-10-01, loaded machine) measured 290 ms for a 50k-path fit against a 16 ms budget. Re-run `vectorcraft-cli perf` on an idle machine |
| **Robustness** | good, new | ~2,840 tests, property tests, no panics in shipped code (lints, the `guard` safety net, import fuzzing), Data Recovery. Missing: a corpus of real-world files, Windows/Linux/browser QA |
| **Platforms and 1.0 polish** | ~60% | The macOS app and the web build work; Windows/Linux packaging and accessibility are pending |
| **Agent automation** | beyond Illustrator | Every command, gesture and dialog is drivable over MCP, the CLI and the control channel |

### Where we're lacking (in priority order)

Ordered by how much each gap blocks someone from switching. Sizes are one-agent hours from the parity table.

1. **Interaction fidelity:** go tool by tool and panel by panel against `plan/illustrator/05-tools.md`,
   `06-panels.md` and `09-shortcuts.md`, covering modifier keys, cursors, the Properties panel per context and
   isolation mode. Do it side by side with Illustrator where the owner allows. 60–90 h.
2. **Photoshop-style raster effects and the Effect Gallery:** about 55 filters (Artistic, Brush Strokes, Distort,
   Pixelate, Sketch, Stylize, Texture, Video) on the raster pipeline that drop shadows already use, applied at
   Document Raster Effects Settings resolution. Parallelizes well across agents. 28–42 h.
3. **3D and Materials:** Extrude & Bevel, Revolve, Inflate and Rotate with lighting and materials, using a software
   renderer in its own crate (layering allows it below L6), with output in SVG/PDF as rasters or projected vectors.
   The largest single gap. 50–80 h.
4. **Advanced type:** CJK composition for vertical type (ruby, mojikumi, vertical font metrics, manual
   tate-chu-yoko), Optical Margin Alignment, the composer/hyphenation options, tab leaders, a
   spell-check dictionary (open licence), Touch Type and Snap to Glyph. Vertical point, area and path type have
   initial support. 35–48 h across type core and advanced.
5. **Brushes, symbols and libraries:** brush options depth; original brush, symbol and graphic-style libraries
   generated in code (never Adobe's); dynamic symbols; Start Global Edit. 14–22 h.
6. **Automation:** Variables (data merge), a scripting surface over the command registry, batch. 8–12 h.
7. **Views and windows:** multiple windows and arrange, Consolidate All Windows, video
   rulers. 15–25 h.
8. **Hardening at scale:** a corpus of real-world SVG/PDF/EPS files, idle-machine perf budgets, Windows, Linux and
   browser QA, accessibility, packaging. 50–80 h.

### Where we're going

- **Next:** start the interaction-fidelity pass (1) in parallel with the raster-effects package (2). Both are broad,
  so they split well across agents, and they move the "can't tell the difference" number most.
- **Then:** 3D (3) and advanced type (4), each a focused milestone with its own plan in `plan/` before coding.
- **Alongside:** keep closing the stubbed menu items. Run `vectorcraft-cli perf` on an idle machine, and fix any
  budget it misses before 1.0.
- **1.0:** feature parity plus the fidelity pass, hardening (8) and packaging for all three desktop OSes and the web.

**For agents:** pick work from the list above (or the milestone rows below), and write a task plan before coding.
When a task lands, update this section, the parity table and "Shipped so far" in the same PR. Keep the scores honest:
grade by behaviour against `plan/illustrator/`, not by whether a menu item exists.

## Shipped so far
- **Illustrator 2026 catch-up (29.8–30.8, see [`docs/illustrator-2026-updates.md`](docs/illustrator-2026-updates.md)):** gradient dithering and perceptual (OKLab) interpolation; artboard background colours, an active artboard with a heavier border, in-place renaming on the canvas, a right-click artboard menu (Duplicate, Rename, Lock, Export, Delete) and Lock Artboard (its art too); Smart Guides snap to segment midpoints and, drawing lines or with the Pen, tangent and perpendicular to paths, with a marker and guide shown while hovering; a Snapping popover in the Control bar; a Blend panel with step easing (Ease In / Out / In and Out, strength) and independent colour easing; Relative / Absolute in the Scale dialog; Relink relinks every instance and the other missing files found in the same folder; swatch libraries read from `.ase`, `.acb` colour books and `.aco` files; form dialogs open by the pointer at a compact width and can be dragged; panels (dock tabs and icon panels) drag out of the dock by their heading to float anywhere or lock in a column beside the toolbar, and go back when dropped on the dock (positions kept between sessions).
- **Architecture:** 19+ crates with enforced layering (`cargo xtask layers`). Every action is a command (~400 engine + ~50 UI). Undo is unlimited via structural sharing. `command.batch` runs several commands as one transaction.
- **Automation:**
  - Actions panel (record/playback, persisted), generic parameter dialogs for every "…" command.
  - JSON-lines control channel with real egui pointer and keyboard injection.
  - MCP server with 25 tools (drawing, text, effects, Pathfinder, transforms, graphs, text wrap, export, screenshots, any command), attached to the running app or headless.
  - MCP protocol: five prompts (`prompts/list` / `prompts/get`: poster, icon set, recolour, trace-and-style, export set), `completion/complete` for prompt arguments and template variables read from the live catalogues (formats, effect ids, command ids, trace presets, swatch and object ids), four resource templates (`vectorcraft://object/{id}`, `//command/{id}`, `//effect/{id}`, `//swatch/{name}`) so an agent reads one thing instead of the whole document, and `logging/setLevel` with `notifications/message`. `docs/mcp.md` has a Protocol section covering the capability table and what is deliberately absent (subscriptions, progress, cancellation, sampling, elicitation, Streamable HTTP — all of which need the transport to interleave reads).
  - Headless CLI (`vectorcraft-cli run`, `convert`, `info`, `bench`, `perf`, `mcp`).
  - Actions panel that records and plays back commands.
- **UI:** Illustrator 2026 layout restyled to measured values:
  - Medium Dark theme, categorized and Advanced toolbars, 35 pt document tabs, 33 pt panel tabs.
  - Hint bar, contextual task bar, 19 dock panels with ≡ menus.
  - Localised UI: menus (in-window and native), panels, dialogs, toolbar and Preferences read from per-language catalogs (`crates/ui-egui/src/i18n`, see `docs/development.md`); Traditional Chinese (Taiwan) ships complete, Czech covers every menu label and Japanese the main menus, with system-locale detection on macOS, Windows and Linux, a Vector W3K2 ▸ Language menu and a Language preference. Names are never translated (fonts and their styles, layers, artboards, saved presets, user libraries, custom workspaces, recent files, plug-ins).
  - Native macOS menu bar, vector tool cursors, a ⌘K command palette, middle-button panning with any tool.
  - A canvas context menu: right-click selects the object under the pointer and lists what applies to the selection (Undo/Redo, clipboard, group, isolation, join, masks, compound paths, guides, Transform, Arrange, Select, Export Selection), or the view commands on empty canvas.
  - Save / Don't Save / Cancel before closing or quitting with unsaved documents (tabs, Close, Close All, Quit, window close).
  - Four brightness themes, persistent preferences.
  - On Windows and Linux the app bar is the window's title bar (its own minimize, maximize and close buttons, drag to move, edges to resize), with the dragon app icon as the brand mark; dialogs and menus size to their content.
- **Tools:**
  - **Selection:** Selection, Direct/Group Selection, Magic Wand, Lasso.
  - **Drawing:** Pen, Curvature, anchor tools, Pencil, Paintbrush, Blob Brush, Smooth, Path Eraser, Join.
  - **Shapes:** all shape tools (including Flare) and the line, arc, spiral and grid tools.
  - **Cutting:** Eraser, Scissors, Knife, Mirror & Cut, Line Cut and Rectangle Cut (real geometry, compound paths keep their holes).
  - **Transform:** Rotate, Reflect, Scale, Shear (click or Alt-click snaps the reference point to anchors and centres), Reshape, Free Transform (distort/perspective).
  - **Live Corners:** drag a live rectangle's corner widgets (Selection or Direct Selection) to round all corners, with a radius readout.
  - **Graphs:** Column, Stacked Column, Bar, Stacked Bar, Line, Area, Scatter, Pie and Radar graph tools with Graph Data and Graph Type.
  - **Other:** Eyedropper, Gradient annotator, Artboard, Measure, Type, Hand, Zoom, Rotate View.
- **Drawing aids:** Smart Guides and snapping, and Draw Normal / Behind / Inside modes.
- **Geometry and effects:**
  - Pathfinder (10 exact curve booleans), Offset, Outline Stroke, Simplify, Clean Up, Split Into Grid, Divide Objects Below.
  - Live effects with previewing dialogs: Distort & Transform, Path, Convert to Shape, 15 Warp styles, Round Corners, Scribble, Effect → Pathfinder (all 10 operations, live on groups; several loose objects are grouped first, in one undo step), Color Adjustments (Brightness/Contrast, Curves, Levels, Hue/Saturation, Shift to Color, Temperature/Tint, on vectors, live type and embedded images), and raster drop shadow, glows and feather. SVG and PDF export keep live effects (geometry baked; SVG raster effects as filters).
- **Type:** Text Wrap, Type on a Path effects (Rainbow/Skew/3D Ribbon/Stair Step/Gravity), Character and Paragraph Styles (override-preserving redefine), Area Type Options (rows/columns/inset/first baseline), threaded text across any closed shapes, Fit Headline.
- **Interface languages, vertical type:** Vector W3K2 ▸ Language (or Preferences ▸ User Interface) picks Automatic (the system locale) or a registered language, persisted as `interfaceLanguage`. Translations come from per-language catalogs (`i18n/*.tsv`): Traditional Chinese covers the whole interface, Czech every menu label (tested) and Japanese the main menus; untranslated text stays English. Japanese and Chinese glyphs come from the craft-fonts build input (release builds) or the installed fonts, Czech ones from the bundled UI fonts. Vertical Type, Vertical Area Type and Vertical Type on a Path create vertical text (columns right to left, upright CJK glyphs and marks with `vert` alternates, Latin and longer numbers on their side, two- and three-digit numbers set as tate-chu-yoko, upright glyphs centred on the column, point type's anchor on the first column's centre line); kinsoku (the strict set) applies to horizontal and vertical Japanese; Type ▸ Type Orientation switches existing text; caret, selection, hit testing and arrow keys follow the writing direction. Vertical text exports to PDF as real text and to SVG as outlines.
- **Transparency:** opacity masks (clip/invert/disable/link), exported as SVG `<mask>` and PDF soft masks.
- **Advanced art:** live Blends (steps/distance/smooth colour, editable spine, anchor-targeted Blend tool, faithful interpolation, knockout), Envelope Distort (warp/mesh/top object, Reset, full Envelope Options, mesh handles, Edit Contents; type, images, symbols, gradients, patterns and appearance distorted everywhere), Liquify tools with options and pen pressure, Puppet Warp with rotating pins, Perspective Grid with presets, movable planes and projected type, Gradient Mesh, Shape Builder, Live Paint (every path an edge, open lines included), Image Trace (12 presets), pattern swatches with pattern editing mode, live Repeat (radial/grid/mirror).
- **Paint and appearance (M3):**
  - **Swatches:** names follow the colour model and are unique; Swatch Options (process/spot, Global, Gray/RGB/HSB/CMYK/Web, live preview); editing a global or spot swatch recolours every linked fill, stroke and text run in one undo step; multi-select and delete with confirmation; New Swatch and New Color Group dialogs (a group from the selected artwork's colours); swatches in colour groups convert, separate and count like any other. The Swatches panel selects ranges and whole groups, has a find field, and drags swatches to reorder, regroup or paint art; Add Used Colors, Select All Unused, Merge, Ungroup and Sort by Kind; a CMYK document keeps applied colours in CMYK. Swatch libraries: nine generated colour libraries and five gradient libraries in a library panel, saved and loaded as `.vcswatches`, `.gpl` or CSS. Tints of global and spot colours ("Name 40%") stay linked and separate as a percentage of their plate. Spot colours can be defined in Lab (Spot Colors options). A built-in [Registration] swatch prints on every plate.
  - **Colour:** a Color Picker (colour field, H/S/B/R/G/B channel slider, HSB/RGB/Lab/CMYK fields, hex, web-only snapping, out-of-gamut correction); the Color panel follows each colour's own model and Alt-click paints the other proxy; proxies look through groups and show "?" for mixed paint; Invert, Complement, Apply Last Color (`,`) and Apply Last Gradient (`.`) keep the colour model; every paint command feeds the recent colours. The Color Guide has 21 harmony rules, Steps and Variation options and Limit to Library. Adjust Color Balance (its Global mode shifts tints) and Saturate preview live; Edit Colors and Recolor also reach meshes, images and patterns. Recolor Artwork has colour reduction, five recolour methods, presets, Limit to Library, a colour-wheel Edit tab and Edit Color Group; a local Color Themes panel saves five-colour themes. Fill/Stroke chips open swatch and mixer popovers, panels have their shortcuts, and Eyedropper Options pick up and apply chosen attributes (Alt and Shift+Alt, image pixels). Fills and strokes can overprint (Attributes panel, Overprint Black options), and CMYK documents blend in inks on screen, in PDF and when flattening.
  - **Gradients:** lossless, validated gradient params; gradients follow rotate, reflect, shear, non-uniform scale and distortions; the Gradient tool edits the fill or stroke of the active proxy, type included; an interactive on-canvas annotator (move, length/angle, rotate, add/move/delete/duplicate stops, midpoints, a stop popover, Delete and arrow keys). The Gradient panel has the Fill/Stroke proxy, a gradient-swatch dropdown with Save to Swatches, midpoint locations, Alt-drag to copy or swap stops, swatch drops on the ramp and a stop eyedropper; copied appearances and graphic styles place gradients on each target's own bounds; gradient handles snap and honour Constrain Angle. Gradient stops stay linked to global and spot swatches. Freeform gradients (points and lines) are edited on the canvas; radial gradients have an extent ellipse, an aspect handle and an off-centre focal point (SVG `fx`/`fy`, PDF two-point radials). A stroke's gradient runs within, along or across it. Object ▸ Expand turns gradients into N objects or a gradient mesh.
  - **Strokes:** one stroke-geometry module shared by the canvas, SVG, PDF and Outline Stroke; dotted lines (zero-length dashes draw dots or squares); arrowheads with hollow outlines, tip-on-end or extend-past-end alignment and one opacity for line and head. SVG and PDF export arrowheads, width profiles, brushes and aligned strokes exactly as the canvas draws them; Outline Stroke and the live Outline Stroke effect outline every stroke item as drawn (dashes, heads, profiles, alignment); dashes can be fitted to corners and path ends; width profiles carry through dashes, and their corners take the join. The Stroke panel edits type's character strokes, follows the stroke units, shows mixed values blank and opens from the Control bar. A width profile library, thirty more arrowheads, and Width Point Edit with multi-select and discontinuous points. Visual bounds and clicks cover the whole painted stroke. Scale Strokes & Effects, Scale Corners and Use Preview Bounds work everywhere, and new art inherits stroke options, a graphic style or the whole last appearance.
  - **Appearance:** the Appearance panel's selected row drives every paint, stroke, gradient and transparency edit (`item` on the commands); effects apply to one fill or stroke; per-item Opacity popups; Mixed Appearances, Layers target dots and basic-appearance rules. Effects can be reordered, moved between items and copied by dragging; Show All Hidden Attributes; clicking an effect edits it in place; the object thumbnail drags onto art; effects render on type, images, symbols, blends, envelopes, meshes and repeats, and per-item raster effects export. Groups, layers and type have their own fills, strokes and effects (Contents and Characters rows); the Layers panel's target circles target layers, groups and objects; Expand Appearance turns fills, strokes, effects and brushes into objects.
  - **Masks and clipping:** opacity-mask controls work while the mask is being edited; transparency commands take ids and work without a selection; clipping sets clip by compound paths, even-odd paths, text and groups on screen and in raster export, matching SVG/PDF. Alt-click shows only the mask; SVG masks keep Clip and Invert; the transparency grid is per-document state; a clipping path's own fill and stroke paint behind and over the clipped art; layers can be clipping masks.
  - **Transparency:** three-state Knockout Group (on, neutral, off), Opacity & Mask Define Knockout Shape, Page Isolated Blending and Page Knockout Group, on screen and in SVG/PDF. Blend modes match the standard formulas on screen, in SVG and in PDF, and non-isolated groups blend with the art below. Flatten Transparency with presets, a dialog and a Flattener Preview panel.
  - **Graphic styles:** styles keep opacity and blend mode, capture groups and type, apply on top with Alt, and link to the objects using them; Redefine, Break Link, Graphic Style Options, Select All Unused, Sort by Name, and Select > Same Graphic Style / Appearance Attribute. The panel has list and thumbnail views with rendered previews, merge, reorder and drag and drop, plus a Control bar Style picker; six generated style libraries save and load as `.vcstyles`.
  - **Neutral wording:** labels, MCP tool text, docs and packaging use neutral names, and `cargo xtask brands` (part of `cargo xtask ci`) fails on vendor names.
- **Colour, type and file workflows:** Recolor Artwork (dialog with harmonies), Edit Colors, Find & Replace, Change Case, Smart Punctuation, Guides, Lock/Hide Above, Transform Each, Rasterize.
- **Fonts:** a font is found by any of its names (family and style in every language of its name table, legacy family/style pairs, PostScript name), so documents from other apps open with the font they name; Find Font and `text.fonts` say for each font whether it was found exactly, by another name, substituted or missing, and how many glyphs are missing.
- **Files (M4):**
  - `.vectorcraft` (lossless JSON, compressed, atomic saves, save down to v1), Save As with a format chooser, Save a Copy, Revert, templates, Data Recovery after a crash, background save and export.
  - SVG/SVGZ in and out with SVG Options, Preserve Editing, symbols, filters, rich text and physical units; PDF and PDF-compatible `.ai` in and out (layers, masks, editable text, spot colours, presets, security, marks and bleed, ICC output intents, raster effects, subset fonts, PDF/X, PDF layers, overprint, page thumbnails, fast web view, PDF 1.3).
  - EPS and DXF in and out, EMF/WMF in and out; PNG (with PNG-8), JPEG, WebP, GIF, TIFF, BMP, Targa and layered PSD export. EPS import runs Illustrator's AGM/CoolType prologs (resource categories, `resourceforall`, `clipsave`/`cliprestore`, subarrays and substrings that share storage) and falls back to its palette TIFF preview. DXF TEXT justified Fit or Aligned spans its two points (stretched, or scaled as a whole).
  - Place and the Links panel, Package, File Info, Print with print presets, PostScript output and print tiling, slices and Save for Web, Export for Screens and Asset Export, CSS Properties, PNG/PDF/SVG/text clipboard flavours.
- **Performance:** 20k shapes + 1k texts render in 27 ms per full-retina frame (7.8 ms zoomed), 7× faster than the first version. The UI thread never blocks. The web build is 7.1 MB gzipped.
- **Tests and robustness:** ~2,840 automated tests: model-based property tests, a junk-parameter sweep over every command, import fuzzing (SVG, PDF, libraries), golden renders, and MCP end-to-end tests over stdio. Shipped code never panics (workspace lints and a rollback safety net; see `docs/development.md`).

## Milestones and estimates

| # | Milestone | Status | Est. remaining, one agent (h) |
|---|---|---|---|
| M0 | Skeleton + vertical slice | ✅ done | — |
| M1 | Selection, transform, layers, MCP | ✅ mostly done (transform reference point snaps to anchors/centres; rotated objects keep a rotated bounding box and their angle; Free Transform, Scale and Reflect along a rotated box pending) | 4–6 |
| M2 | Drawing tools + smart guides | ✅ mostly done (Flare, Reshape, Live Corners widget dragging landed; Shaper, Pen modifier nuances) | 15–20 |
| M3 | Paint & appearance (swatches, color, gradient, stroke, appearance, transparency, styles) | ✅ done (M3.7–M3.98): swatches, groups, libraries, tints, Lab spots and Registration; Color Picker, Color panel, Color Guide, Color Themes, Edit Colors and Recolor Artwork; gradients that follow every transform, the annotator, linked stops, freeform gradients, focal points, gradients on strokes and Expand; stroke geometry, arrowheads, dashes fitted to corners, stroke on type, width profiles and the Width tool, Scale Strokes and preview bounds; Appearance targeting, container appearance, target circles, Expand Appearance; knockout, isolation, blend accuracy, CMYK blending, overprint, Attributes, masks and clipping, Flatten Transparency; graphic styles, links and libraries. Left: freeform and mixed spot/process gradients export as stops or process colours to SVG/PDF; confirm the Scale Strokes & Effects default | 4–8 |
| M4 | Files (native, SVG, PDF, raster, Export for Screens, clipboard interop) | ✅ done (M4.14–M4.98 on 2026-10-05): Save As/Save a Copy/Revert/templates, Data Recovery and background save, SVG import and export fidelity (units, text, symbols, filters, SVGZ, Preserve Editing, SVG Options), PDF import (colours, layers, masks, editable text, security) and export (presets, marks, bleed, ICC and output intent, raster effects, subset fonts, PDF/X-1a, X-3 and X-4, PDF layers, overprint, page thumbnails, fast web view, PDF 1.3), Place and Links, Package, EPS and DXF in and out, EMF/WMF, TIFF/BMP/Targa/PSD/GIF/PNG-8 export, Print and print presets, slices, Save for Web, Asset Export, clipboard flavours, File Info. Left: DWG (use DXF), PSD placement as layers, overprint read back from PDF, polish | 3–6 |
| M5 | Performance | 🟡 background render + caches + MT done; `vectorcraft-cli bench` and `vectorcraft-cli perf` (budget suite); file format v2 opens 3× faster (50k paths: 722 → 244 ms); raster effects (glows, shadows, blur, feather) no longer force the whole frame single-threaded (filtered offscreen per effect, verified equal to the single-threaded reference); effect-heavy demos need a clean-machine benchmark; dirty-region rendering, GPU backend spike pending | 15–25 |
| M6 | Path operations (Pathfinder, Shape Builder, offset…) | ✅ mostly done (Boolean precision on almost-horizontal edges fixed and the property tests made deterministic; Shape Builder edge erase, large-offset bug open) | 3–6 |
| M7 | Type (point/area/path, editing, styles, OpenType, threading, glyphs) | 🟡 Character/Paragraph Styles, Area Type Options, threaded text, Fit Headline, Glyphs, OpenType panel, Find Font, installed system fonts in every font menu (searchable) and found by name on open/import, CJK names in the UI, input methods (marked text in place, candidate window at the caret; Japanese IME), Text Wrap (offset, invert, both sides of an object; follows edits), Type on a Path effects, tab stops + Tabs panel done; tab leaders, spell check, vertical type pending | 45–60 |
| M8 | Transform & distort (Puppet Warp, Liquify tools, Envelopes, Blends, Perspective Grid) | ✅ done (M8.1–M8.20 on 2026-10-06, fidelity pass): Liquify Tool Options with one shared brush, pen pressure, hold-to-apply, a safe scope and incremental strokes; tool options that last; Puppet Warp rotation, multi-select, Control bar and a rest-shape session that follows Undo; Width tool on compound paths; Envelope dialogs with preview, Reset with Warp/Mesh, full Envelope Options, type/images/symbols/appearance/gradients/patterns distorted on canvas and in every export, warps that keep their frame, Edit Contents, mesh handles and Control bar; Blend Options, anchor-targeted Blend tool, editable spine, keys picked on the canvas, faithful interpolation and knockout; Perspective Grid with Define Grid, presets, lock/snap/rulers, widgets, the 1–4 plane switch, movable planes, Perspective Selection scale/copy/perpendicular/Transform Again, and type and symbols truly projected with Edit Text. Left: Anti-Alias and Preserve Shape output, warp-envelope point editing, brush interpolation in blends, Free Transform/Scale/Reflect along a rotated box | 2–4 |
| M9 | Live effects (+ 3D & Materials) | 🟡 2D effects done incl. Effect → Pathfinder and Color Adjustments; SVG Filters, Document Raster Effects Settings, 3D pending | 86–135 |
| M10 | Brushes, symbols, patterns, Repeat | 🟡 pattern swatches (5 tile types, Pattern Options, editing mode, SVG `<pattern>`/PDF export) and live Repeat (radial/grid/mirror) done; brushes/symbols in progress | 23–37 |
| M11 | Artboards & views (artboard panel/tool, Trim View, middle-button pan and print tiling done; multiple windows, presentation polish) | 🟡 | 15–25 |
| M12 | Advanced color & art (CMYK/ICC, separations, Gradient Mesh, Live Paint, Image Trace, Graphs) | 🟡 Gradient Mesh, Live Paint, Image Trace (12 presets, 18 ms/1k² image), Recolor Artwork, colour management (ICC, soft proofing, separations preview), Graphs (all 9 tools, Graph Data/Type, regenerate in place) done; graph Design/Column/Marker designs pending | 6–10 |
| M13 | Automation (Actions ✅ record/playback, persisted; variables, scripting, batch) | 🟡 | 18–27 |
| M14 | 1.0 polish (preferences, shortcut editor, workspaces, accessibility, packaging for all OSes) | 🟡 Preferences, shortcut editor, workspaces, a custom title bar on Windows/Linux with a Home button, content-sized dialogs and menus that scroll when longer than the window done; accessibility, Windows/Linux packaging pending | 18–28 |
| — | Interaction fidelity pass (every tool's modifiers, Properties panel per context, isolation, nuance) | ⬜ | 60–90 |
| — | Hardening at scale (big-file corpus, fuzzing, cross-platform + browser QA) | 🟡 | 50–80 |
| | **Total to full parity** (one agent; re-derived from the [Parity estimate](#parity-estimate) table) | | **~320–500** |

## Parity estimate

_Method (2026-10-02; M3 rows re-derived 2026-10-04, file, export, print, links and UI-chrome rows after M4 on 2026-10-05):_ Illustrator's feature surface is split into 22 areas, weighted by how much of the app (and of
real users' work) each represents. Each area is scored by depth of behaviour, not by presence of a menu item: an area
is 100% only when every feature in it behaves like Illustrator. Feature parity = Σ weight × score / Σ weight. Remaining
time is counted per area from what is missing, calibrated on measured throughput: in the last session one agent landed
about 20 medium features (Text Wrap, Graphs, Effect → Pathfinder, export baking…) in ~16 h wall clock including builds
and CI on a heavily loaded machine — about 0.8 h per medium feature; large subsystems (3D, raster filters, vertical type)
are counted bottom-up. The M3 rows (colour, strokes, appearance) come from a detailed 92-task M3 plan whose review
found more missing depth than the first estimate; all 92 tasks landed on 2026-10-04 and 2026-10-05 with 6–8 agents in parallel; its packages, planned at 5–11 h each, took
about 1–1.5 agent-hours each, so the other rows (estimated on the older scale) are likely high.

| Area | Weight | Done | Missing (main items) | One agent (h) |
|---|---:|---:|---|---:|
| Selection, transform & align tools | 6 | 85% | Free Transform/Scale/Reflect along a rotated box, Start Global Edit, transform nuances (the rotated persistent bounding box is done) | 4–6 |
| Drawing tools | 7 | 85% | Shaper Groups (merge/punch overlapping shapes), pen/pencil modifier nuances, Touch Type | 10–15 |
| Path operations, Pathfinder, Shape Builder, Live Paint | 5 | 86% | Live Paint gap options, Shape Builder edge cases | 3–6 |
| Colour, swatches, gradients, patterns, mesh, recolor | 7 | 96% | freeform and mixed spot/process gradients in SVG/PDF (exported as stops or process colours), pattern fills in Expand | 2–4 |
| Strokes, brushes, width profiles | 5 | 88% | brush options depth, brush libraries (generated in code) | 6–10 |
| Appearance, transparency, graphic styles, masks | 5 | 98% | raster effect reach in preview bounds (flattener presets apply in PDF, EPS and print) | 1–2 |
| Live vector effects | 5 | 85% | Outline Object, Pathfinder Hard/Soft Mix and Trap, SVG Filters | 6–10 |
| Raster effects (Effect Gallery, Document Raster Effects Settings) | 4 | 20% | ~55 Photoshop-style filters (Artistic, Brush Strokes, Distort, Pixelate, Sketch, Stylize, Texture, Video) and the Effect Gallery; Document Raster Effects Settings and raster effects in PDF are done | 28–42 |
| 3D and Materials | 4 | 0% | Extrude & Bevel, Revolve, Inflate, Rotate, lighting, materials (software renderer) | 50–80 |
| Type core | 9 | 78% | composer/hyphenation options, Optical Margin Alignment, hidden characters | 15–20 |
| Type advanced | 4 | 48% | CJK composition for vertical type (ruby, mojikumi, vertical metrics, manual tate-chu-yoko), tab leaders, spell check (open dictionary), Touch Type, Retype; vertical point/area/path type and Type Orientation have initial support | 20–28 |
| Symbols, blends, envelopes, Repeat, perspective | 5 | 88% | symbol libraries (original), dynamic symbols, envelope Anti-Alias/Preserve Shape output and warp point editing, brush interpolation in blends (M8 fidelity pass done) | 5–8 |
| Image Trace, graphs, image tools | 3 | 70% | graph Design/Column/Marker, Create Object Mosaic, Crop Image polish (Vector Halftone is done) | 6–10 |
| Layers, artboards, document setup | 5 | 80% | Layers panel options depth, artboard presets/rearrange polish (Document Setup and New Document are done) | 5–8 |
| View & navigation | 3 | 70% | New View/Edit Views, multiple windows/arrange, Snap to Pixel/Glyph (print tiling is done) | 9–14 |
| Guides, grids, smart guides, snapping, rulers | 3 | 75% | global/video rulers, smart-guide preference depth | 4–8 |
| File formats | 6 | 85% | DWG (no open spec: DXF instead), PSD placement as layers, Illustrator EPS checked against real files (only reduced repros so far), the remaining fidelity polish; EPS, DXF, EMF/WMF, TIFF, BMP, Targa, PSD export, SVGZ, PDF security, PDF/X, PDF layers and presets are done | 6–10 |
| Export for Screens, Asset Export, slices, Save for Web | 3 | 85% | polish only (all four are done) | 2–4 |
| Print, colour management, separations, flattener | 3 | 80% | print preview fidelity (flattener presets in PDF and print, overprint in PDF and composite print, Print dialog, presets, PostScript, marks, separations, Print Tiling, print overprint/flattener/bitmap options and printer profile are done) | 1–2 |
| Automation | 3 | 60% | Variables (data merge), scripting surface, batch | 8–12 |
| UI chrome (panels, contextual Properties, workspaces, prefs) | 6 | 78% | Variables, SVG Interactivity, Properties per context, Consolidate All Windows, context menus beyond the canvas (Links, Asset Export, CSS Properties, Attributes and the canvas context menu are done) | 15–25 |
| Libraries, Links, Package | 2 | 50% | a local Libraries panel (no cloud by design); Links and Package are done | 5–8 |
| **Feature parity** | **103** | **~75%** | | **~210–330** |
| Interaction-fidelity pass (side by side with Illustrator: every tool modifier, cursor, dialog, Properties context) | | | | 60–90 |
| Hardening (big-file corpus, fuzzing, cross-platform and browser QA, accessibility, packaging) | | | | 50–80 |
| **Full parity** | | **~56%** | | **~320–500** |

With 4–6 agents working on disjoint crates (as the layering allows) the wall-clock time divides by roughly 3.5–4
(integration, review and shared files such as `menus.rs` serialize some work): **~55–95 h** to feature parity,
**~80–145 h** to full parity.

_Inventories (2026-10-05):_ 16 menu items still stubbed (`todo(…)` in `crates/ui-egui/src/menus.rs`); every tool
implemented except Touch Type; 51 panel modules; Illustrator-style live
effects ~44/54, Photoshop-style raster effects ~1/56, 3D 0/5; ~2,840 tests; ~221k lines of Rust.

_Where we already beat Illustrator:_ exact curve booleans, off-thread multithreaded rendering, undo that never runs out,
lossless SVG/PDF export of live effects with SVG filters, a documented JSON format, the same app on the web, and every
command, gesture and dialog drivable by agents (MCP, CLI, control channel).

## Out of scope (by design or by law)
- **Native `.ai` private data:** it's undocumented. We read the PDF-compatible part, so Illustrator-only live objects arrive as appearance.
- **Adobe cloud services** (Libraries sync, Adobe Fonts, Firefly/generative): these are pluggable provider APIs, not built in.
- **Adobe's bundled assets** (swatch/brush/symbol libraries, presets, icons): ours are original.

## Where we aim to be better than Illustrator
- **Speed:** off-thread multithreaded rendering, instant startup, a responsive UI on huge files.
- **Robustness:** exact curve booleans (no "cannot perform operation"), property-tested undo/redo and file round trips.
- **Openness:** a documented native format, first-class SVG, and the same app in the browser.
- **Automation:** every command, gesture, dialog and widget is drivable by agents (JSON channel + MCP), with headless batch mode.

## How to update this file
After each milestone task lands, update the status column and remaining estimates, and move items into "Shipped so far". Keep estimates honest: re-derive them from what remains, never from wishful velocity.
