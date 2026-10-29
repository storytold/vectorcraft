# Effects parity

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-28 · **Change:** minor (Drop Shadow Darkness uses fixed RGB subtraction and CMYK black addition; scores unchanged; Pathfinder › Trap: the Pathfinder panel's Trap… and Effect › Pathfinder › Trap… add an overprinting strip of the lighter colour where two solid colours meet; recounted with Hard Mix (#3061) and Soft Mix already in the catalogue: Pathfinder 13/13, Illustrator effects 49/50; previously: Transform effect: Transform Objects, Transform Patterns and Scale Strokes & Effects, #885; G2: the Effect Gallery shipped (#2883): Effect › Effect Gallery… browses the Artistic, Brush Strokes, Distort, Sketch, Stylize and Texture filters, edits each filter's settings, builds an ordered stack (add, reorder, hide, remove) with a live canvas preview, and OK applies it as separate appearance effects in one undo step; remaining: filter thumbnails and a preview area inside the dialog; G2 estimate 4–6 → 1–2 h; Raster effects 82% → 85% (power-user view 49% → 52%), estimated, a modest move because the named missing dialog landed without thumbnails or an in-dialog preview; previously:; stacked geometry effects make at most 2 million anchors of a path and about 8 million lines to draw; Roughen, Zig Zag, Free Distort, Twist and the warps build within it, #3578; scores unchanged; G2: Effect › Sketch › Conté Crayon (Foreground Level, Background Level, Texture, Scaling, Relief, Light Direction, Invert; a black crayon in the dark areas and a white one in the light areas on textured mid-grey paper; ranges and defaults unverified VectorCraft choices); measured Photoshop-style filters 57/57, the Effect Gallery remains (4–6 h); 3D: original Standard, Diffuse, Top Left and Right lighting presets; command access, effective-setting recognition and preview/history; scores and estimates unchanged; G2: Effect › Sketch › Note Paper; Halftone Pattern and Note Paper defaults documented as unverified VectorCraft choices; G2: Effect › Artistic › Smudge Stick, Sponge and Watercolor; Sketch › Bas Relief, Charcoal, Photocopy and Stamp, #2533; Photoshop filters 46/57, 11 remaining (8–14 h); G2: Sketch › Chrome; Sketch › Chalk & Charcoal, Graphic Pen and Reticulation and Artistic › Plastic Wrap, #2972, #2967, #2976, #3034: 37/57 Photoshop effects; 3D lighting: one to ten colored lights, smooth colored Inflate, legacy white-light appearance preserved; surface batching and restored seam minimum; scores and estimates unchanged; Zig Zag's Smooth points make a continuous wave, #2343; G2: Effect › Sketch › Halftone Pattern; Photoshop filters 46/57, 11 remaining (8–14 h); Round Corners also rounds corners where a curve meets a line or another curve, with Live Corners' cut; straight corners unchanged; score unchanged; Revolve: automatic smooth shading and default-on Cap with continuous rims; wide shadows and glows at high zoom, #1706; stacked Transform effects, #1705; 3D on nonzero compound paths, #1724; pattern tiles redraw, #1723; scores unchanged; Roughen's Smooth turn-backs, #1643; Scribble per region, #1648; score unchanged; Warp Arc bends in the object's proportions, #1578; Zig Zag ridges evenly spaced, #1558; effect parameters checked, #1592; score unchanged; raster effects after a pattern, radial or freeform gradient fill export at their place and size, #1536; score unchanged; 3D and Materials on outlined type and other shapes of many small curves, #1481; Twist 0° to 360°, #1524; twisted walls keep their faces, #1525; score unchanged; 3D and Materials: Inflate takes Twist, Taper and Cap; score unchanged; 3D and Materials: Perspective as the documented 0°–160° angle; 3D and Materials: the documented light settings) · **Target:** Adobe Illustrator 2026 (30.x)

Every entry of Illustrator's Effect menu, by its public feature name, against VectorCraft's effects catalogue
(`effect_catalog()` in `crates/effects/src/lib.rs`) and the Effect menu built from it
(`crates/ui-egui/src/menus.rs`). Counts are **measured** from the catalogue; "depth" notes are estimated.
Part of [target-app-parity.md](target-app-parity.md); open work is in [gaps.md](gaps.md).

## Summary

| Group | Illustrator | VectorCraft | % | Remaining (h) |
|---|---:|---:|---:|---:|
| Illustrator effects (vector and stylize) | 50 | 49 | 98% | 4–8 |
| Photoshop effects (raster, Effect Gallery) | 57 | 57 | 100% | 1–2 (Effect Gallery thumbnails and in-dialog preview) |
| 3D and Materials | 4 (+3 Classic) | 4 (initial: Extrude & Bevel, Revolve, Inflate, Rotate) | ~25% | 25–45 |
| Beyond Illustrator: Effect › Color Adjustments (6), effect plug-ins (WebAssembly) | — | 6 + plug-ins | — | — |

## Illustrator effects

| Submenu | Items | Ours | Missing |
|---|---|---:|---|
| Convert to Shape | Rectangle, Rounded Rectangle, Ellipse | 3/3 | — |
| Crop Marks | Crop Marks | 1/1 | — |
| Distort & Transform | Free Distort, Pucker & Bloat, Roughen, Transform, Tweak, Twist, Zig Zag | 7/7 | Free Distort's dialog has a handle on each corner of the object's bounds, moved by dragging or with the arrow keys; Transform Patterns and Scale Strokes & Effects on a fill's or stroke's own Transform effect (on the object, Transform has all three options and visible checkboxes, #885); Zig Zag's ridges are evenly spaced along each segment (#1558), and Smooth points make a continuous wave whose peaks and valleys have handles along the path (#2343); Roughen with Smooth keeps turn-backs smooth instead of cusps that draw as spikes (#1646) |
| Path | Offset Path, Outline Object, Outline Stroke | 3/3 | With Use Preview Bounds on, Align, Distribute and the Transform panel still measure type that carries Outline Object by its layout box (preview bounds leave out effects) |
| Pathfinder | Add, Intersect, Exclude, Subtract, Minus Back, Divide, Trim, Merge, Crop, Outline, Hard Mix, Soft Mix, Trap | 13/13 | Trap traps only solid fills (gradients and patterns hide what is below them but don't trap) and spreads each colour's visible area as a whole; the public page gives no default values, so Trap's 0.25 pt, 100% and 40% are VectorCraft's own |
| Rasterize | Rasterize… (as a live effect) | 1/1 | — |
| Stylize | Drop Shadow, Feather, Inner Glow, Outer Glow, Round Corners, Scribble | 6/6 | Scribble Options' Settings presets |
| SVG Filters | Apply SVG Filter… and the filter list | 0/1 | the whole submenu (SVG filters on import round-trip already) |
| Warp | Arc, Arc Lower, Arc Upper, Arch, Bulge, Shell Lower, Shell Upper, Flag, Wave, Fish, Rise, Fisheye, Inflate, Squeeze, Twist | 15/15 | warp point editing in envelopes; Arc is a circular arc in the object's own proportions (#1578) |

Document Raster Effects Settings exists. When the 64 million pixel cap on an effect image lowers
its resolution, PDF export warns once per object with the requested and the lowest achieved ppi
(`crates/engine/src/tests/pdfraster.rs`).

## Photoshop effects

| Submenu | Items | Ours | Missing |
|---|---|---:|---|
| Artistic | Colored Pencil, Cutout, Dry Brush, Film Grain, Fresco, Neon Glow, Paint Daubs, Palette Knife, Plastic Wrap, Poster Edges, Rough Pastels, Smudge Stick, Sponge, Underpainting, Watercolor | 15/15 (approximate Colored Pencil, Cutout, Dry Brush, Film Grain, Fresco, Neon Glow, Paint Daubs, Palette Knife, Plastic Wrap, Poster Edges, Rough Pastels, Smudge Stick, Sponge, Underpainting, Watercolor) | — |
| Blur | Gaussian Blur, Radial Blur, Smart Blur | 3/3 | — |
| Brush Strokes | Accented Edges, Angled Strokes, Crosshatch, Dark Strokes, Ink Outlines, Spatter, Sprayed Strokes, Sumi-e | 8/8 | — |
| Distort | Diffuse Glow, Glass, Ocean Ripple | 3/3 | — |
| Pixelate | Color Halftone, Crystallize, Mezzotint, Pointillize | 4/4 | — |
| Sharpen | Unsharp Mask | 1/1 | — |
| Sketch | Bas Relief, Chalk & Charcoal, Charcoal, Chrome, Conté Crayon, Graphic Pen, Halftone Pattern, Note Paper, Photocopy, Plaster, Reticulation, Stamp, Torn Edges, Water Paper | 14/14 (approximate Bas Relief, Chalk & Charcoal, Charcoal, Chrome, Conté Crayon, Graphic Pen, Halftone Pattern, Note Paper, Photocopy, Plaster, Reticulation, Stamp, Torn Edges, Water Paper; Chalk & Charcoal, Conté Crayon, Graphic Pen, Note Paper and Water Paper defaults unverified) | — |
| Stylize | Glowing Edges | 1/1 | — |
| Texture | Craquelure, Grain, Mosaic Tiles, Patchwork, Stained Glass, Texturizer | 6/6 | — |
| Video | De-Interlace, NTSC Colors | 2/2 | — |
| Effect Gallery | the combined browser and stack dialog | 1/1 (Effect › Effect Gallery…, #2883: six categories, per-filter settings, ordered stack with add/reorder/hide/remove, live canvas preview, one undo step; `dialogs::effect_gallery::tests::stack_ok_is_one_undo_step`, `stack_preview_is_one_interaction`) | filter thumbnails; a preview area inside the dialog ([Filter Gallery overview](https://helpx.adobe.com/photoshop/desktop/effects-filters/get-started-with-filters/filter-gallery.html)) |

The pixel-filter pipeline (`vectorcraft_effects::pixel`) is in place: each new filter is one function, a
catalogue row, a menu entry and translations. Measured throughput: 0.7–1.2 agent hours per filter
(see [calibration](target-app-parity.md#calibration)). Every Photoshop-style filter is in the catalogue, and the Effect
Gallery dialog shipped (#2883); its filter thumbnails and in-dialog preview area are the 1–2 h that remain.

Film Grain (`artistic.filmGrain`, Effect › Artistic) adds stable monochrome noise to shadows and
midtones, with Grain, Highlight Area and Intensity sliders, live preview and appearance editing.
Its smooth grain field follows translations, preserves transparency and stays in document units;
zoomed-out rendering uses bounded approximate footprint filtering. Changing the bounds shifts
the pattern. Command exports bake it to an image in SVG and PDF, while the native document keeps the effect editable. Behaviour is an original visual
approximation based on [Illustrator's public effects reference](https://helpx.adobe.com/illustrator/using/summary-effects.html)
and the [public Artistic filters guide](https://helpx.adobe.com/photoshop-elements/using/artistic-filters.html),
not a pixel-identical proprietary implementation. Intensity is interpreted as highlight strength and
quieter highlight grain. Grain strength stays even across shadows and midtones and rolls off above
mid-grey to a quarter-strength floor at full Intensity before highlight headroom; cream retains
soft grain, black takes light speckles and white stays clean. The tonal curve and endpoint behaviour
are interpretations, not verified proprietary behaviour. Saturated colour channels may clip. The parameter
ranges and defaults are chosen for VectorCraft, not asserted as measured proprietary defaults.

Cutout, Neon Glow and Poster Edges (`artistic.cutout`, `artistic.neonGlow`, `artistic.posterEdges`)
follow the same two public references, with original algorithms. Cutout softens the object's colours (Edge Simplicity, less with Edge
Fidelity), splits each colour channel into Number of Levels steps, and paints each resulting piece
in the average colour that falls into it, so hues survive. Poster Edges
posterizes each colour channel to Posterization + 2 levels and draws black lines on the edges of
any channel and of the object's outline, wider with Edge Thickness and darker, on fainter edges
too, with Edge Intensity. Neon Glow redraws the object in dimmed greys screened with a glow colour
from its light areas (positive Glow Size) or its dark ones (negative), spread by the size and as
strong as Glow Brightness; with no foreground and background colours in Illustrator's model, the
glow colour is the effect's own parameter. All three keep coverage, work in document units at any
Document Raster Effects resolution, and use the generic effect dialog with live preview. Ranges and
defaults are chosen for VectorCraft. Evidence: `crates/effects/src/pixel/artistic.rs`, the
`artistic_*`, `cutout_*`, `poster_edges_*` and `neon_glow_*` tests in `crates/effects/src/pixel/tests.rs`,
and `artistic_settings_survive_a_save_and_open` in `crates/engine/src/tests/rasterfilters.rs`.
Smudge Stick (`artistic.smudgeStick`), Sponge (`artistic.sponge`) and Watercolor (`artistic.watercolor`),
Effect › Artistic, are original visual approximations from the public descriptions in
[Illustrator's effects reference](https://helpx.adobe.com/illustrator/using/summary-effects.html) and the
[public Artistic filters guide](https://helpx.adobe.com/photoshop-elements/using/artistic-filters.html), not
pixel-identical copies. Each has live preview, appearance editing, document-unit patterns that follow the
object, bounded work per pixel and kept coverage (they paint nothing outside the object), and bakes to an
image in SVG and PDF. Ranges and defaults are chosen for VectorCraft, not asserted as measured.
- Smudge Stick (Stroke Length 0–10, Highlight Area 0–20, Intensity 0–10): dark areas smudged along short
  diagonal strokes; light areas smudged with strokes twice as long and lightened towards white.
- Sponge (Brush Size 0–10, Definition 0–25, Smoothness 1–15): the object softened, then lighter and darker
  blobs of the same hue, their contrast set by Definition, their edges by Smoothness.
- Watercolor (Brush Detail 1–14, Shadow Intensity 0–10, Texture 1–3): detail simplified by an edge-keeping
  average whose brush widens as Brush Detail falls, colour saturated and pooled darker where tones change at
  edges, shadows deepened, a faint paper grain. Edges are found from tone only, so a change of hue at equal
  brightness is not saturated.

Tests: `pixel::tests_paint::*`, `tests_rasterfilters`, `dialogs::tests::raster_filter_dialogs_*`,
`tests_labels::effect_menu_lists_the_artistic_filters_*` and the `command_sweep` structured junk.

Near white, highlight headroom can limit grain equally at every Intensity; highlight lift still
depends on Intensity when Highlight Area is nonzero.
The two rotated, variance-normalised noise layers can leave a faint diagonal weave at high
magnification. Removing that remaining direction would require a more expensive field; the
current implementation keeps bounded per-pixel work. Film Grain is still slower than Texture
Grain (Regular), particularly when its wider footprint needs four samples.
Calibration on uniform grey at Grain 20, using 32×32 fine samples per coarse pixel, measured
grain standard deviation within about 15% of the dense area average at pixel widths of 2, 4
and 8 document units (ratios 1.13, 0.89, 0.85). This is evidence for the bounded approximation,
not an accuracy guarantee for arbitrary artwork. Tests: `pixel::tests::film_grain_*`,
`tests_rasterfilters`, `tests_adjust::film_grain_dialog_*` and `tests_labels::effect_menu_lists_the_artistic_filters_*`.

Smudge Stick (`artistic.smudgeStick`), Sponge (`artistic.sponge`) and Watercolor (`artistic.watercolor`),
Effect › Artistic, are original visual approximations from the public descriptions in
[Illustrator's effects reference](https://helpx.adobe.com/illustrator/using/summary-effects.html) and the
[public Artistic filters guide](https://helpx.adobe.com/photoshop-elements/using/artistic-filters.html), not
pixel-identical copies. Each has live preview, appearance editing, document-unit patterns that follow the
object, bounded work per pixel and kept coverage (they paint nothing outside the object), and bakes to an
image in SVG and PDF. Ranges and defaults are chosen for VectorCraft, not asserted as measured.
- Smudge Stick (Stroke Length 0–10, Highlight Area 0–20, Intensity 0–10): dark areas smudged along short
  diagonal strokes; light areas smudged with strokes twice as long and lightened towards white.
- Sponge (Brush Size 0–10, Definition 0–25, Smoothness 1–15): the object softened, then lighter and darker
  blobs of the same hue, their contrast set by Definition, their edges by Smoothness.
- Watercolor (Brush Detail 1–14, Shadow Intensity 0–10, Texture 1–3): detail simplified by an edge-keeping
  average whose brush widens as Brush Detail falls, colour saturated and pooled darker where tones change at
  edges, shadows deepened, a faint paper grain. Edges are found from tone only, so a change of hue at equal
  brightness is not saturated.

Tests: `pixel::tests_paint::*`, `tests_rasterfilters`, `dialogs::tests::raster_filter_dialogs_*`,
`tests_labels::effect_menu_lists_the_artistic_filters_*` and the `command_sweep` structured junk.

Bas Relief, Charcoal, Photocopy and Stamp (`sketch.basRelief`, `sketch.charcoal`, `sketch.photocopy`,
`sketch.stamp`, Effect › Sketch; `pixel/sketch_ink.rs`) redraw the object in black ink on white paper
and keep its coverage. Bas Relief carves low relief lit from one of eight directions (Detail, Smoothness,
Light Direction); Charcoal draws bold edges and sketches the midtones in diagonal strokes (Thickness,
Detail, Light/Dark Balance); Photocopy prints only where a pixel is darker than its surroundings, so large
dark areas copy around their edges (Detail, Darkness); Stamp reduces the object to solid black and
white (Light/Dark Balance, Smoothness). Sizes are in document points at any Document Raster Effects
resolution, strokes and noise lie in document space around the object's centre, and all four preview
live. Behaviour is an original visual approximation based on
[Illustrator's public effects reference](https://helpx.adobe.com/illustrator/using/summary-effects.html)
and the [public Sketch filters guide](https://helpx.adobe.com/photoshop-elements/using/sketch-filters.html),
not a pixel-identical proprietary implementation; the parameter ranges and defaults are chosen for
VectorCraft, not asserted as measured proprietary defaults. Tests: `pixel::tests_sketch::*`,
`tests_rasterfilters`, `command_sweep`, `dialogs::tests::raster_filter_dialogs_preview_and_apply` and
`tests_labels::effect_menu_lists_the_sketch_filters_*`.

Effect › Sketch › Halftone Pattern and Note Paper (`sketch.halftonePattern`, `sketch.notePaper`) are
original black-and-white filters guided by [Illustrator's public Sketch effects summary](https://helpx.adobe.com/illustrator/using/summary-effects.html)
and [effects overview](https://helpx.adobe.com/illustrator/using/effects.html) (Halftone Size in points at any Document Raster Effects resolution).
The public help pages name each control but publish no numeric defaults or slider limits; VectorCraft's ranges
follow those control names and the emboss/halftone behaviour described there, while dialog defaults are chosen
for VectorCraft and are not verified against proprietary defaults. Evidence: `crates/effects/src/pixel/sketch.rs`,
`halftone_*` and `note_paper_*` in `crates/effects/src/pixel/tests.rs`, `SKETCH` and
`halftone_pattern_settings_survive_a_save_and_open` in `crates/engine/src/tests/rasterfilters.rs`, and
`tests_labels::effect_menu_lists_the_sketch_filters_between_sharpen_and_stylize`.

Effect › Sketch › Conté Crayon (`sketch.conteCrayon`; `pixel/sketch.rs`) redraws the object in a dense
black crayon where it is dark and a pure white crayon where it is light, on mid-grey paper, as
[Illustrator's public effects summary](https://helpx.adobe.com/illustrator/using/summary-effects.html) describes
(black for dark areas, white for light areas). Its controls follow the public
[Photoshop filter reference](https://helpx.adobe.com/photoshop/using/filter-effects-reference.html) and the
[Photoshop Elements Sketch filters guide](https://helpx.adobe.com/photoshop-elements/using/sketch-filters.html)
(foreground and background levels, texture options): Foreground Level and Background Level (0–20) set how far
each crayon reaches into the midtones; Texture (Brick, Burlap, Canvas, Sandstone, the surfaces Texturizer
draws), Scaling, Relief, Light Direction and Invert make the paper's tooth, which breaks the diagonal crayon
strokes, and its relief. The public pages publish no slider ranges or defaults, so those are unverified
VectorCraft choices; an original approximation, never compared with another application's output. Tests:
`pixel::tests_sketch::conte_crayon_*`, `tests_rasterfilters`, `command_sweep`,
`dialogs::tests::raster_filter_dialogs_preview_and_apply` and
`tests_labels::effect_menu_lists_the_sketch_filters_between_sharpen_and_stylize`.

## 3D and Materials

| Item | Ours |
|---|---|
| Revolve | initial: live, editable profile, rotation and light gizmos, default-on Cap with continuous curved rims, smooth vector shading and round seam joins, SVG/PDF output, Expand Appearance ([revolve.md](revolve.md), #846) |
| Extrude & Bevel, Inflate, Rotate | initial: live, depth, twist, taper, solid or hollow caps, five original bevel shapes, Inflate volume, both sides, twist, taper and cap, Rotate of every fill and stroke, Position presets, rotation and light gizmos, SVG/PDF output, Expand Appearance ([three-d.md](three-d.md)) |
| Materials panel, lighting, ray-traced render | one to ten directional lights with per-light Color (opaque RGB, white by default), Intensity, Rotation, Height, Softness and neutral Ambient Light, Revolve and colored Inflate use smooth vector shading; Extrude & Bevel and Rotate retain planar shading; original Standard, Diffuse, Top Left and Right lighting rigs; no materials, shadows or ray tracing |
| 3D (Classic): Extrude & Bevel, Revolve, Rotate | none |
| Turntable (30.0 beta) | none |
| Export 3D objects (OBJ, USDA, glTF) | none |

The `three-d` crate (depends on `geom` and `pathops`, with no UI or GPU dependencies) builds, rotates, projects, shades and depth-sorts the surfaces;
the effects crate turns them into art. What's left (materials and shadows, ray-traced rendering,
angular cut caps, exact visibility, 3D Classic, Turntable, 3D export) is 25–45 h.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-23 | minor | Pathfinder › Trap ([trapping](https://helpx.adobe.com/illustrator/using/trapping.html)): the Pathfinder panel's Trap… (`object.pathfinder.trap`, one undo step, the traps grouped above the topmost selected object and selected) and Effect › Pathfinder › Trap… (`pathfinder.trap`, live on groups) add an overprinting strip of the lighter colour inside the darker one where two solid colours meet (Reverse Traps: the darker into the lighter, but not a rich black), with Thickness, Height/Width, Tint Reduction and Traps with Process Color; the effect also has Precision and Remove Redundant Points. Recounted: Hard Mix (#3061) and Soft Mix had shipped, so Pathfinder 11/13 → 13/13 and Illustrator effects 46/50 → 49/50 (SVG Filters remains); remaining 6–10 → 4–8 h. Tests: `vectorcraft_pathops` `trap::tests`, `vectorcraft_effects` `trap::tests` and `group::tests::trap_effect_adds_an_overprinting_strip_above_the_members`, `vectorcraft_engine` `tests_trap`, `panels::pathfinder::tests::trap_opens_its_options_and_ok_traps_the_selection`. |
| 2026-10-22 | minor | The Transform effect has Transform Objects, Transform Patterns and Scale Strokes & Effects, with visible checkboxes (#885); on a fill's or stroke's own Transform effect the last two are still missing. Counts unchanged. |
| 2026-10-22 | minor | G2: the Effect Gallery shipped (#2883): Effect › Effect Gallery… browses the Artistic, Brush Strokes, Distort, Sketch, Stylize and Texture filters, edits each filter's settings, builds an ordered stack (add, reorder, hide, remove) with a live canvas preview, and OK applies it as separate appearance effects in one undo step; remaining: filter thumbnails and a preview area inside the dialog; G2 estimate 4–6 → 1–2 h; Raster effects 82% → 85% (power-user view 49% → 52%), estimated, a modest move because the named missing dialog landed without thumbnails or an in-dialog preview. |
| 2026-10-22 | minor | Stacked geometry effects make at most 2 million anchors of a path (of a type, symbol or image art's paths together) and about 8 million lines to draw: Roughen spaces its points further apart, Zig Zag makes fewer ridges a segment, Free Distort, Twist and the warps split into longer pieces, a result drawn with too many lines is built again with fewer points, and an effect that would still pass either is left out (#3578). Scores unchanged (estimated) |
| 2026-10-22 | minor | G2: Effect › Sketch › Conté Crayon (Foreground Level, Background Level, Texture, Scaling, Relief, Light Direction, Invert; a black crayon in the dark areas and a white one in the light areas on textured mid-grey paper; ranges and defaults unverified VectorCraft choices); measured Photoshop-style filters 57/57, the Effect Gallery remains (4–6 h); Sketch 14/14. |
| 2026-10-21 | minor | 3D: original Standard, Diffuse, Top Left and Right lighting presets; command access, effective-setting recognition and preview/history; scores and estimates unchanged |
| 2026-10-20 | minor | G2: Effect › Sketch › Water Paper (Fiber Length, Brightness, Contrast; colours flow on damp fibrous paper, defaults unverified, #2955); measured Photoshop filters 47/57, Sketch 12/14. |
| 2026-10-20 | minor | G2: Effect › Sketch › Note Paper (Image Balance, Graininess, Relief; embossed tones on fibrous paper); Halftone Pattern and Note Paper parameter ranges follow public control names, numeric defaults are unverified VectorCraft choices (public help gives no numbers); measured Photoshop filters 46/57, Sketch 11/14. |
| 2026-10-19 | minor | Sketch › Chalk & Charcoal, Graphic Pen and Reticulation and Artistic › Plastic Wrap landed (#2972, #2967, #2976, #3034): 36/57 Photoshop effects (estimated; defaults unverified) |
| 2026-10-16 | minor | Zig Zag with Smooth points draws a continuous wave: every peak and valley is a smooth point with handles along the path, a third of the way to the next ridge, so each span follows a half cosine through the path instead of a straight diagonal with rounded tips, also at the ends of open paths and for ridges much taller than they are apart (#2343); score unchanged (estimated) |
| 2026-10-16 | minor | G2: Effect › Artistic › Smudge Stick, Sponge and Watercolor (live, with their documented options; original approximations); measured Photoshop filters 36/57 with Cutout, Neon Glow and Poster Edges, 21 remaining (19–31 h). |
| 2026-10-16 | minor | G2: Effect › Sketch › Bas Relief, Charcoal, Photocopy and Stamp (black ink on white paper, sizes in points at any Document Raster Effects resolution, live preview); measured Photoshop filters 37/57 with Cutout, Neon Glow and Poster Edges, 20 remaining (18–30 h). |
| 2026-10-16 | minor | G2: Effect › Sketch › Halftone Pattern (Size, Contrast, Pattern Type Circle, Dot or Line; black ink on white paper, Size in points at any Document Raster Effects resolution); measured Photoshop filters 30/57, 27 remaining (22–38 h). |
| 2026-10-17 | minor | Colored Inflate: shared membrane normals and smooth RGB vector ramps replace flat polygon bands; channel groups are batched per solid-fill surface. White-light Inflate retains its original polygons, paint composition and seam joins; the 0.05 seam minimum is restored. Scores and hour estimates unchanged. |
| 2026-10-16 | minor | 3D lighting: one to ten directional lights, a selectable light list, Add Light and Delete Light, independent settings and artboard gestures; ambient applied once and smooth RGB vector export; scores and hour estimates unchanged |
| 2026-10-16 | minor | 3D lighting: opaque RGB Color for the shared directional light, with neutral ambient, preview/reset controls and smooth vector Revolve shading; scores and hour estimates unchanged |
| 2026-10-15 | minor | Round Corners rounds corners whose anchor carries a handle (where a curve meets a line, or two curves meet) with the round cut Live Corners uses (#1144), reaching `radius` along each side; before, they were left sharp. Corners between handle-less anchors are rounded as before; score unchanged (estimated) |
| 2026-10-14 | minor | Revolve: automatic smooth vector shading, default-on Cap with continuous curved rims, and bounded seam joins; angular cut sides remain open. Scores and hour estimates unchanged at this rounding |
| 2026-10-14 | minor | Wide drop shadows and glows at high zoom render within the raster budget (#1706); stacked Transform effects are bounded (#1705); 3D effects keep the area a nonzero compound path fills (#1724); pattern tiles redraw when a symbol, pattern or image their art shows changes (#1723) (all #1864). Scores unchanged (estimated) |
| 2026-10-14 | minor | Roughen with Smooth keeps turn-backs smooth instead of making cusps that draw as spikes (#1646, #1643); Scribble draws one zig-zag per region, with no lines across holes and notches; convex shapes are unchanged (#1651, #1648); scores unchanged (estimated) |
| 2026-10-13 | minor | Zig Zag spaces its ridges evenly along each segment, so lines and curves get a regular wave in Corner and Smooth (#1558); Warp › Arc (the effect and Envelope › Make with Warp) bends in the object's real proportions: a circular arc with concentric edges and no fold-over at 100%, so Arc on art that isn't square draws differently from before (#1581, fixes #1578); `effect.apply` and `effect.setParams` give an error for a misspelt or wrongly typed effect parameter instead of running the effect with its default (#1623, #1592); scores unchanged (estimated) |
| 2026-10-13 | minor | 3D lighting takes the documented Intensity, Rotation, Height, Softness and Ambient Light (to 200 %), and a light behind the object; earlier lights read as the same ([three-d.md](three-d.md#lighting)); 3D stays ~25%, 25–45 h |
| 2026-10-13 | minor | 3D Perspective is the documented 0°–160° angle, earlier strengths read as the same view ([three-d.md](three-d.md#perspective)); 3D stays ~25%, 25–45 h |
| 2026-10-13 | minor | Raster effects (blur, shadow, glow, grain) drawn after a pattern, radial or freeform gradient fill export at their place and size (#1541, fixes #1536); scores unchanged (estimated) |
| 2026-10-13 | minor | 3D and Materials: outlined words and other shapes of many small curves take Rotate, Extrude & Bevel and Inflate (curves are sampled by a tolerance; Rotate takes up to 65,536 outline points; #1481); Extrude & Bevel's Twist runs 0° to 360° as the public panel reference gives it, a negative value drawing untwisted (#1524); a twisted extrusion keeps its walls, which were dropped from about 45° (#1525); details in [three-d.md](three-d.md); 3D stays ~25%, 25–45 h (estimated) |
| 2026-10-13 | minor | Inflate takes Twist, Taper and Cap ([three-d.md](three-d.md)); 3D stays ~25%, 25–45 h: materials, lights and shadows are the bulk of what remains |
| 2026-10-12 | minor | Extrude & Bevel, Inflate and Rotate (initial, [three-d.md](three-d.md)); 3D ~8% → ~25%, 45–75 → 25–45 h |
| 2026-10-12 | minor | G2: Artistic › Film Grain added with live controls and raster exports; measured Photoshop filters 29/57, 28 remaining (23–39 h). |
| 2026-10-11 | minor | PDF cap warnings identify original objects after appearance splitting and report the lowest achieved ppi; cap/rendering and scores unchanged |
| 2026-10-11 | minor | Zig Zag's dialog: Size and Ridges per segment sliders, Points as Smooth / Corner radio buttons (#1133) |
| 2026-10-11 | minor | Brush Strokes landed (8 filters): 28/57 Photoshop effects |
| 2026-10-10 | minor | Initial Revolve landed (#846) |
| 2026-10-10 | major | First checklist, counted from the effects catalogue (20/57 Photoshop effects, 44/50 Illustrator effects) |
| 2026-10-28 | minor | Drop Shadow Darkness subtracts the normalized amount from RGB source channels or adds it to CMYK/Gray black before profile conversion and blur; near-100% becomes gray black, with shadow opacity separate. SVG keeps live filters for uniform sRGB sources and renders shadows for other RGB profiles, source models and gradients. Gradient-stop adjustment is an application rule; spot/custom colors and composite-source parity remain open. Scores and estimates unchanged. |
