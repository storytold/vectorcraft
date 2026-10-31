# CorelDRAW (.cdr) compatibility

Status: open, place and export implemented (`crates/cdr`, `vectorcraft-cdr`). The feature matrix
below is generated from `crates/cdr/src/matrix.rs` (the `compatibility_doc_matches_the_matrix` test
keeps them in step).

## Where it is

- **File › Open…**: the dialog's filters include `*.cdr`, and the format is also recognised by its
  content. Shows *CorelDRAW Import Options*.
- **File › Place…**: places a `.cdr` in the open document through the same dialog.
- **File › Export › Export As CDR…** and **File › Export › Export As… › CorelDRAW**: show
  *CorelDRAW Export Options*.

Import options: keep the original document scale (off when placing: the first page fits the first
artboard); keep layers and groups; keep type editable or convert it to outlines; warn about missing
fonts; report unsupported effects.

Export options: the target version (each with its status); type editable or as outlines; vector
paths always kept (never rasterized); explicit permission to rasterize raster effects; post-export
validation (on by default).

Commands for automation and MCP: `document.cdrInfo`, `document.cdrCapabilities`,
`document.open {cdr: {…}}`, `file.place {cdr: {…}}`,
`document.export {format: "cdr", version, textToOutlines, validate, rasterizeEffects}`.

## Versions

| Version | Product | Container | Read | Write |
|---|---|---|---|---|
| 3–6 | CorelDRAW 3–6 | RIFF (16-bit before 6) | partial | — |
| 7–13 | CorelDRAW 7–12, X3 | RIFF + `cmpr` (zlib) | yes | experimental |
| 14–15 | CorelDRAW X4, X5 | ZIP (`content/riffData.cdr`) | yes | experimental |
| 16–22 | CorelDRAW X6 … 2020 | ZIP (`content/root.dat`) | yes | **verified** (default: 16) |
| 23–26 | CorelDRAW 2021 … 2024 | ZIP (`content/root.dat`) | yes | — (write 22, which these releases open) |

"Verified" means files of that version written by the upstream writer (TraceDraw) opened in
CorelDRAW in that project's tests. VectorCraft's own validation reads the written file back with the
reader and compares it with what was written; it doesn't run CorelDRAW.

## Feature matrix

<!-- matrix:start -->
| Feature | Import | Export |
|---|---|---|
| Vector paths and Bézier curves | **Fully supported**: Nodes, cubic curves and closed subpaths kept as editable paths (even-odd fill, as CorelDRAW fills them) | **Fully supported**: Written as CDR curves with their nodes; node types (cusp, smooth, symmetrical) come from the handles |
| Shapes and geometric primitives | **Converted with limitations**: Rectangles (with corners), ellipses, arcs, polygons and stars become editable paths of the same geometry; the primitive's parameters are lost | **Converted with limitations**: VectorCraft live shapes are written as curves |
| Positioning and transformations | **Fully supported**: Each object's matrix is applied to its geometry; type and images keep their matrix | **Fully supported**: Coordinates converted from points to CDR units relative to the page; type and images keep their own matrix |
| Groups and hierarchy | **Fully supported**: Nested groups kept (up to 64 levels); tables and symbols become groups | **Partially supported**: Groups kept; sublayers become groups; symbols, blends and repeats become the art they draw |
| Layers and layer order | **Partially supported**: Name and order; master layers under the first page. The reader doesn't read visibility, lock or print settings: layers come in visible and unlocked | **Partially supported**: Name and order; hidden layers and objects are left out (with a warning), as the writer keeps no hidden state; lock isn't written; layer opacity moves to the objects; template layers are left out |
| Text content and font attributes | **Partially supported**: Editable artistic and paragraph text: family, bold/italic, size, colour, underline, strikethrough, alignment, tracking. Type on a path loses its path; leading and paragraph indents use the defaults | **Partially supported**: Point type written as editable artistic text (verified in CorelDRAW from version 16) or as outlines on request; area, vertical and path type, and type inside clipping groups, become outlines |
| Fill colours and stroke properties | **Fully supported**: RGB, CMYK, greyscale and Lab colours; strokes with width, caps, joins and dashes | **Partially supported**: One fill and one stroke per object (the first of each); inside/outside alignment, arrowheads and width profiles are lost; Lab becomes RGB |
| Gradients | **Converted with limitations**: Linear and radial with their colour stops; conical, square and shaped become radial; repeats and mirroring are expanded into stops | **Converted with limitations**: Linear and radial with angle and centre; stop opacity and midpoints are lost; freeform becomes linear |
| Transparency | **Partially supported**: An object's uniform transparency becomes its opacity; gradient transparencies don't | **Partially supported**: Object, fill and group opacity combined into one uniform transparency per object |
| Clipping paths and masks | **Partially supported**: PowerClip becomes an editable clipping group; opacity masks are not imported | **Converted with limitations**: The clip is applied to the geometry (a boolean intersection, editable; clipped strokes become filled areas); clipped bitmaps get a native crop path; opacity masks and blending modes are not written |
| Page dimensions and document units | **Fully supported**: Exact physical size (mm → pt); the document in millimetres | **Fully supported**: Each artboard becomes a page of the same size; checked by the validation |
| Multiple pages | **Fully supported**: One artboard per page, side by side, with each page's layers | **Fully supported**: One page per artboard; each object goes to the artboard holding its centre |
| Embedded bitmaps | **Fully supported**: Pixels kept (RGB, CMYK converted, transparency) with position and transformation | **Partially supported**: Images embedded at 24 bits; linked images are embedded; bitmap alpha may be lost |
| Colour profiles and CMYK | **Partially supported**: CMYK values kept; the file's ICC profile is not applied (the active one is); spot colours become CMYK | **Partially supported**: CMYK and RGB values kept; ICC profiles and spot colours are not written |
| Effects (shadows, blends, extrusions, lenses, envelopes) | **Not supported**: Live effects are not recreated; the shapes stored in the file are; shadows are left out (with a warning) | **Converted with limitations**: Live vector effects are applied and written as shapes; raster effects are left out unless rasterizing them is allowed |
| Pattern, texture and mesh fills | **Converted with limitations**: Replaced by an approximate solid colour (with a warning) | **Converted with limitations**: Patterns written in their first solid colour; gradient meshes written as the art they draw |
<!-- matrix:end -->

## Known writer limitations

- Clipping groups around vectors are not written as PowerClip: the clip is applied to the geometry
  (a boolean intersection; clipped strokes become filled areas). Clipped bitmaps use the native crop
  path.
- Area, vertical and path type is written as outlines.
- One fill and one stroke per object; one uniform transparency per object.
- ICC profiles, spot colours, opacity masks and blending modes are not written.
