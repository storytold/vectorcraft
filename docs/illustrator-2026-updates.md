# Illustrator 2026 updates vs Vector W3K2

Illustrator's release notes for 29.8 (August 2025) through 30.8 (August 2026), checked against Vector W3K2.
Generative AI features (Text to Vector Graphic, Turntable, Concept to Vector, Rewrite, Generative Expand,
Remove Background, Generative Shape Fill, Firefly Boards and the rest) are out of scope and not listed, as are
cloud-only features (Projects, cloud storage export, account settings, credits).

Status: ✅ in Vector W3K2 (already there, or added on the `illustrator-2026-updates` branch) · ⬜ not yet.

| Release | Feature | Status | Notes |
|---|---|---|---|
| 30.0 | Gradient dithering to reduce banding | ✅ | Gradient panel ▸ Dither; ordered dither on screen and in raster export (`paint.editGradient {dither}`) |
| 30.0 | Perceptual gradient interpolation | ✅ | Gradient panel ▸ Interpolation; mixes stops in OKLab, exported to SVG/PDF as extra stops (SVG round-trips) |
| 30.0 | Ready-to-use gradient presets | ✅ | five generated gradient libraries |
| 30.1 | Convert a solid fill to a colour-aware gradient | ⬜ | a solid still becomes the default White, Black gradient |
| 30.0 | Snap tangent to arcs and circles | ✅ | Line Segment and Pen tools, from the line's start or the last anchor |
| 30.0 | Snap perpendicular to lines and curves | ✅ | same tools |
| 29.8 | Smart Guides snap to endpoint, midpoint and centre | ✅ | straight-segment midpoints added (anchors and centres existed) |
| 29.8 | Snap only within the active artboard | ⬜ | |
| 29.8 | Snapping quick-access popover in the Control bar | ✅ | Smart Guides, Snap to Point/Grid/Pixel and a link to the Smart Guides preferences |
| 29.8 | Snap to Grid / Snap to Pixel / Snap to Point | ✅ | |
| 30.0 | Artboard background colours | ✅ | Artboard Options ▸ Background; on screen, raster, SVG and PDF export |
| 30.0 | Lock all objects on an artboard | ✅ | `artboard.lock`: the artboard and its art; unlocking unlocks only what it locked |
| 30.0 | Artboard labels on canvas, rename in place | ✅ | double-click an artboard's name on the canvas |
| 30.0 | Clearer border on the active artboard | ✅ | the active artboard is now real state (`artboard.setActive`), shared by the Artboards panel and tool |
| 30.0 | Right-click artboard menu (Duplicate, Rename, Lock, Export, Delete) | ✅ | right-click empty canvas on an artboard, or its name |
| 30.0 | Export selected artboards from the canvas | ✅ | Export Artboard… opens Export for Screens with that artboard checked |
| 30.0 | Hide Grid widget on the perspective grid | ⬜ | View ▸ Perspective Grid ▸ Hide Grid works; no on-canvas widget yet |
| 30.0 | Save in Background, crash recovery | ✅ | |
| 30.0 | Enhanced font browser (find, favourites, filters) | ⬜ | font menus are searchable; favourites and filters are not there yet |
| 30.0 | Variable font axis sliders in the Character panel | ⬜ | needs variation support in the text crate |
| 30.2 | Export for Screens to TIFF | ✅ | shares the export format registry |
| 30.2 | Relative and Absolute scaling | ✅ | Scale dialog ▸ Relative / Absolute; `object.scale {width, height}` |
| 30.3 | Pencil live curve fitting on by default | ⬜ | to check against the Pencil tool |
| 30.4 | Mojikumi and Kinsoku presets | ⬜ | follows the CJK composition work in the roadmap |
| 30.5 | Numeric fields ignore the scroll wheel unless focused | ✅ | the wheel never changes a field |
| 30.6 | Relink all instances of a linked image at once | ✅ | `links.relink {allInstances}` (on by default) |
| 30.6 | Shortcuts and actions for Align and Distribute | ✅ | every command takes shortcuts and records as an action |
| 30.6 | Preview distribute spacing before applying | ⬜ | |
| 30.7 | Blend panel (Make, Expand, Release, Reverse Spine, Replace Spine, live preview) | ✅ | Window ▸ Blend; edits apply to the selected blend as you change them |
| 30.7 | Blend easing (Ease In / Ease Out presets with strength) | ✅ | Ease In, Ease Out, Ease In and Out with a strength |
| 30.7 | Colour acceleration separate from spacing | ✅ | Blend panel ▸ Color ease |
| 30.8 | Relink other missing files from the same folder | ✅ | `links.relink {sameFolder}` (on by default) |
| 30.8 | Clicking only a drop shadow or effect area doesn't select | ✅ | hit testing already uses the geometry, not the effect bounds |
| 29.8 | Copy colour values from the Color panel | ✅ | Copy Color Value (Hex) |
