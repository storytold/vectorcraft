# Effects parity

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-16 · **Change:** minor (G2: Effect › Sketch › Halftone Pattern; Photoshop filters 30/57, 27 remaining (22–38 h); Round Corners also rounds corners where a curve meets a line or another curve, with Live Corners' cut; straight corners unchanged; score unchanged; Revolve: automatic smooth shading and default-on Cap with continuous rims; wide shadows and glows at high zoom, #1706; stacked Transform effects, #1705; 3D on nonzero compound paths, #1724; pattern tiles redraw, #1723; scores unchanged; Roughen's Smooth turn-backs, #1643; Scribble per region, #1648; score unchanged; Warp Arc bends in the object's proportions, #1578; Zig Zag ridges evenly spaced, #1558; effect parameters checked, #1592; score unchanged; raster effects after a pattern, radial or freeform gradient fill export at their place and size, #1536; score unchanged; 3D and Materials on outlined type and other shapes of many small curves, #1481; Twist 0° to 360°, #1524; twisted walls keep their faces, #1525; score unchanged; 3D and Materials: Inflate takes Twist, Taper and Cap; score unchanged; 3D and Materials: Perspective as the documented 0°–160° angle; 3D and Materials: the documented light settings) · **Target:** Adobe Illustrator 2026 (30.x)

Every entry of Illustrator's Effect menu, by its public feature name, against VectorCraft's effects catalogue
(`effect_catalog()` in `crates/effects/src/lib.rs`) and the Effect menu built from it
(`crates/ui-egui/src/menus.rs`). Counts are **measured** from the catalogue; "depth" notes are estimated.
Part of [target-app-parity.md](target-app-parity.md); open work is in [gaps.md](gaps.md).

## Summary

| Group | Illustrator | VectorCraft | % | Remaining (h) |
|---|---:|---:|---:|---:|
| Illustrator effects (vector and stylize) | 50 | 45 | 90% | 6–10 |
| Photoshop effects (raster, Effect Gallery) | 57 | 30 | 53% | 22–38 |
| 3D and Materials | 4 (+3 Classic) | 4 (initial: Extrude & Bevel, Revolve, Inflate, Rotate) | ~25% | 25–45 |
| Beyond Illustrator: Effect › Color Adjustments (6), effect plug-ins (WebAssembly) | — | 6 + plug-ins | — | — |

## Illustrator effects

| Submenu | Items | Ours | Missing |
|---|---|---:|---|
| Convert to Shape | Rectangle, Rounded Rectangle, Ellipse | 3/3 | — |
| Crop Marks | Crop Marks | 1/1 | — |
| Distort & Transform | Free Distort, Pucker & Bloat, Roughen, Transform, Tweak, Twist, Zig Zag | 7/7 | Free Distort's dialog has a handle on each corner of the object's bounds, moved by dragging or with the arrow keys; Transform's Transform Objects/Patterns and Scale Strokes & Effects options, invisible Reflect checkboxes (#885); Zig Zag's ridges are evenly spaced along each segment (#1558); Roughen with Smooth keeps turn-backs smooth instead of cusps that draw as spikes (#1646) |
| Path | Offset Path, Outline Object, Outline Stroke | 3/3 | With Use Preview Bounds on, Align, Distribute and the Transform panel still measure type that carries Outline Object by its layout box (preview bounds leave out effects) |
| Pathfinder | Add, Intersect, Exclude, Subtract, Minus Back, Divide, Trim, Merge, Crop, Outline, Hard Mix, Soft Mix, Trap | 10/13 | Hard Mix, Soft Mix, Trap |
| Rasterize | Rasterize… (as a live effect) | 0/1 | the live effect (Object › Rasterize exists) |
| Stylize | Drop Shadow, Feather, Inner Glow, Outer Glow, Round Corners, Scribble | 6/6 | Scribble Options' Settings presets |
| SVG Filters | Apply SVG Filter… and the filter list | 0/1 | the whole submenu (SVG filters on import round-trip already) |
| Warp | Arc, Arc Lower, Arc Upper, Arch, Bulge, Shell Lower, Shell Upper, Flag, Wave, Fish, Rise, Fisheye, Inflate, Squeeze, Twist | 15/15 | warp point editing in envelopes; Arc is a circular arc in the object's own proportions (#1578) |

Document Raster Effects Settings exists. When the 64 million pixel cap on an effect image lowers
its resolution, PDF export warns once per object with the requested and the lowest achieved ppi
(`crates/engine/src/tests_pdfraster.rs`).

## Photoshop effects

| Submenu | Items | Ours | Missing |
|---|---|---:|---|
| Artistic | Colored Pencil, Cutout, Dry Brush, Film Grain, Fresco, Neon Glow, Paint Daubs, Palette Knife, Plastic Wrap, Poster Edges, Rough Pastels, Smudge Stick, Sponge, Underpainting, Watercolor | 1/15 (approximate Film Grain) | Colored Pencil, Cutout, Dry Brush, Fresco, Neon Glow, Paint Daubs, Palette Knife, Plastic Wrap, Poster Edges, Rough Pastels, Smudge Stick, Sponge, Underpainting, Watercolor |
| Blur | Gaussian Blur, Radial Blur, Smart Blur | 3/3 | — |
| Brush Strokes | Accented Edges, Angled Strokes, Crosshatch, Dark Strokes, Ink Outlines, Spatter, Sprayed Strokes, Sumi-e | 8/8 | — |
| Distort | Diffuse Glow, Glass, Ocean Ripple | 3/3 | — |
| Pixelate | Color Halftone, Crystallize, Mezzotint, Pointillize | 4/4 | — |
| Sharpen | Unsharp Mask | 1/1 | — |
| Sketch | Bas Relief, Chalk & Charcoal, Charcoal, Chrome, Conté Crayon, Graphic Pen, Halftone Pattern, Note Paper, Photocopy, Plaster, Reticulation, Stamp, Torn Edges, Water Paper | 1/14 | Bas Relief, Chalk & Charcoal, Charcoal, Chrome, Conté Crayon, Graphic Pen, Note Paper, Photocopy, Plaster, Reticulation, Stamp, Torn Edges, Water Paper |
| Stylize | Glowing Edges | 1/1 | — |
| Texture | Craquelure, Grain, Mosaic Tiles, Patchwork, Stained Glass, Texturizer | 6/6 | Texturizer's Load Texture |
| Video | De-Interlace, NTSC Colors | 2/2 | — |
| Effect Gallery | the combined browser and stack dialog | 0/1 | all |

The pixel-filter pipeline (`vectorcraft_effects::pixel`) is in place: each new filter is one function, a
catalogue row, a menu entry and translations. Measured throughput: 0.7–1.2 agent hours per filter
(see [calibration](target-app-parity.md#calibration)), so the 27 missing filters are ~18–33 h and the Effect
Gallery dialog another 4–6 h. These parallelize well across agents.

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
`tests_rasterfilters`, `tests_adjust::film_grain_dialog_*` and `tests_labels::effect_menu_lists_film_grain_*`.

## 3D and Materials

| Item | Ours |
|---|---|
| Revolve | initial: live, editable profile, rotation and light gizmos, default-on Cap with continuous curved rims, smooth vector shading and round seam joins, SVG/PDF output, Expand Appearance ([revolve.md](revolve.md), #846) |
| Extrude & Bevel, Inflate, Rotate | initial: live, depth, twist, taper, solid or hollow caps, five original bevel shapes, Inflate volume, both sides, twist, taper and cap, Rotate of every fill and stroke, Position presets, rotation and light gizmos, SVG/PDF output, Expand Appearance ([three-d.md](three-d.md)) |
| Materials panel, lighting, ray-traced render | one directional light with the documented Intensity, Rotation, Height, Softness and Ambient Light, Revolve uses smooth vector shading, the other types use flat facets; no materials, several lights, light colour or presets, shadows or ray tracing |
| 3D (Classic): Extrude & Bevel, Revolve, Rotate | none |
| Turntable (30.0 beta) | none |
| Export 3D objects (OBJ, USDA, glTF) | none |

The `three-d` crate (depends on `geom` and `pathops`, with no UI or GPU dependencies) builds, rotates, projects, shades and depth-sorts the surfaces;
the effects crate turns them into art. What's left (materials, several lights and shadows, ray-traced rendering,
angular cut caps, exact visibility, 3D Classic, Turntable, 3D export) is 25–45 h.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-16 | minor | G2: Effect › Sketch › Halftone Pattern (Size, Contrast, Pattern Type Circle, Dot or Line; black ink on white paper, Size in points at any Document Raster Effects resolution); measured Photoshop filters 30/57, 27 remaining (22–38 h). |
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
