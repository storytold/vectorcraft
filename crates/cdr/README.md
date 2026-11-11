# vectorcraft-cdr

A bounded Rust reader for CorelDRAW drawings of versions 5 to 13 (`.cdr`, and `.cdt` templates).
It reads the RIFF chunk tree with its compressed lists and the records listed below, and hands the
engine a drawing in points, with each object's transform. It does not write CorelDRAW files.

## What is read

| CorelDRAW | Read as |
|---|---|
| Pages, the page size in `mcfg` or a page's own size | pages, laid out by the engine as artboards side by side |
| Layers (drawing, desktop, guides and grid layers), the master page's among them | layers by name and kind |
| Guidelines (object type 0x16, and the version 7 `guid` chunk) | guidelines at their point and angle |
| Groups (`LIST grp `) | groups |
| Curves (point lists of moves, lines and cubic curves) | paths |
| Rectangles (one corner radius, four from version 9) | rectangles with each corner's radius |
| Ellipses, arcs and pies | ellipses, and paths for arcs and pies |
| Polygons and stars (a base side copied for each side) | paths |
| Uniform fills: CMYK, CMY, RGB, gray, registration, library colors | colors, spot colors with their tints |
| Outlines: width, caps, joins, dashes, the no-outline flag | outlines |

Text, bitmaps, linked groups (blends, contours, brush strokes), B-spline curves, fountain,
pattern, texture and PostScript fills and outline arrowheads are not read; the drawing's
`warnings` name each kind it held. Drawings of versions before 5 and from 14 on are refused.

## Provenance

`PROVENANCE.md` lists every record layout the reader relies on, with its public source and status.
Code cites the rows by id (`R1` ...), and `tests/provenance.rs` fails when a cited id has no row.
The test inputs are synthetic drawings built by `src/synth.rs` (the `synth` feature); no
third-party file is committed.

## Limits

`Limits` bounds the input size, the bytes inflated from compressed lists, the nesting depth, the
chunk count, the objects and the coordinates of one read. Past a limit the reader stops with an
error or leaves the rest out with a warning; it does not recurse and does not panic.
