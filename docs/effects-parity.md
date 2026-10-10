# Effects parity

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-10 · **Change:** major (first checklist, counted from the effects catalogue) · **Target:** Adobe Illustrator 2026 (30.x)

Every entry of Illustrator's Effect menu, by its public feature name, against VectorCraft's effects catalogue
(`effect_catalog()` in `crates/effects/src/lib.rs`) and the Effect menu built from it
(`crates/ui-egui/src/menus.rs`). Counts are **measured** from the catalogue; "depth" notes are estimated.
Part of [target-app-parity.md](target-app-parity.md); open work is in [gaps.md](gaps.md).

## Summary

| Group | Illustrator | VectorCraft | % | Remaining (h) |
|---|---:|---:|---:|---:|
| Illustrator effects (vector and stylize) | 50 | 44 | 88% | 6–10 |
| Photoshop effects (raster, Effect Gallery) | 57 | 20 | 35% | 30–50 |
| 3D and Materials | 4 (+3 Classic) | 0 | 0% | 50–80 |
| Beyond Illustrator: Effect › Color Adjustments (6), effect plug-ins (WebAssembly) | — | 6 + plug-ins | — | — |

## Illustrator effects

| Submenu | Items | Ours | Missing |
|---|---|---:|---|
| Convert to Shape | Rectangle, Rounded Rectangle, Ellipse | 3/3 | — |
| Crop Marks | Crop Marks | 1/1 | — |
| Distort & Transform | Free Distort, Pucker & Bloat, Roughen, Transform, Tweak, Twist, Zig Zag | 7/7 | Transform's Transform Objects/Patterns and Scale Strokes & Effects options, invisible Reflect checkboxes (#885) |
| Path | Offset Path, Outline Object, Outline Stroke | 2/3 | Outline Object |
| Pathfinder | Add, Intersect, Exclude, Subtract, Minus Back, Divide, Trim, Merge, Crop, Outline, Hard Mix, Soft Mix, Trap | 10/13 | Hard Mix, Soft Mix, Trap |
| Rasterize | Rasterize… (as a live effect) | 0/1 | the live effect (Object › Rasterize exists) |
| Stylize | Drop Shadow, Feather, Inner Glow, Outer Glow, Round Corners, Scribble | 6/6 | — |
| SVG Filters | Apply SVG Filter… and the filter list | 0/1 | the whole submenu (SVG filters on import round-trip already) |
| Warp | Arc, Arc Lower, Arc Upper, Arch, Bulge, Shell Lower, Shell Upper, Flag, Wave, Fish, Rise, Fisheye, Inflate, Squeeze, Twist | 15/15 | warp point editing in envelopes |

Document Raster Effects Settings exists.

## Photoshop effects

| Submenu | Items | Ours | Missing |
|---|---|---:|---|
| Artistic | Colored Pencil, Cutout, Dry Brush, Film Grain, Fresco, Neon Glow, Paint Daubs, Palette Knife, Plastic Wrap, Poster Edges, Rough Pastels, Smudge Stick, Sponge, Underpainting, Watercolor | 0/15 | all |
| Blur | Gaussian Blur, Radial Blur, Smart Blur | 3/3 | Smart Blur's Edge Only and Overlay Edge modes |
| Brush Strokes | Accented Edges, Angled Strokes, Crosshatch, Dark Strokes, Ink Outlines, Spatter, Sprayed Strokes, Sumi-e | 0/8 | all |
| Distort | Diffuse Glow, Glass, Ocean Ripple | 3/3 | Glass's Load Texture |
| Pixelate | Color Halftone, Crystallize, Mezzotint, Pointillize | 4/4 | — |
| Sharpen | Unsharp Mask | 1/1 | — |
| Sketch | Bas Relief, Chalk & Charcoal, Charcoal, Chrome, Conté Crayon, Graphic Pen, Halftone Pattern, Note Paper, Photocopy, Plaster, Reticulation, Stamp, Torn Edges, Water Paper | 0/14 | all |
| Stylize | Glowing Edges | 1/1 | — |
| Texture | Craquelure, Grain, Mosaic Tiles, Patchwork, Stained Glass, Texturizer | 6/6 | Texturizer's Load Texture |
| Video | De-Interlace, NTSC Colors | 2/2 | — |
| Effect Gallery | the combined browser and stack dialog | 0/1 | all |

The pixel-filter pipeline (`vectorcraft_effects::pixel`) is in place: each new filter is one function, a
catalogue row, a menu entry and translations. Measured throughput: 0.7–1.2 agent hours per filter
(see [calibration](target-app-parity.md#calibration)), so the 37 missing filters are ~26–44 h and the Effect
Gallery dialog another 4–6 h. These parallelize well across agents.

## 3D and Materials

| Item | Ours |
|---|---|
| Extrude & Bevel, Revolve, Inflate, Rotate | none (menu stubs) |
| Materials panel, lighting, ray-traced render | none |
| 3D (Classic): Extrude & Bevel, Revolve, Rotate | none |
| Turntable (30.0 beta) | none |
| Export 3D objects (OBJ, USDA, glTF) | none |

Plan: a software renderer in its own crate (the layering allows it below L6), output to SVG/PDF as images or
projected vectors. 50–80 h, the largest single gap.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-10 | major | First checklist, counted from the effects catalogue (20/57 Photoshop effects, 44/50 Illustrator effects) |
