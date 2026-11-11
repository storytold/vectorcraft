# vectorcraft-cdr

A bounded Rust reader for `.cdr` drawings of versions 5 to 27 (and `.cdt` templates).
It reads the RIFF chunk tree with its compressed lists, the ZIP packages that versions 14 and later
save (with the data streams of versions 16 and later) and the records listed below, and hands the
engine a drawing in points, with each object's transform. It does not write `.cdr` files.

## What is read

| In the `.cdr` file | Read as |
|---|---|
| ZIP packages: `content/riffData.cdr` (versions 14 and 15), `content/root.dat` with the data streams `content/dataFileList.dat` names (16 on) | the RIFF chunk tree; a data stream is inflated only when a chunk the reader reads, or a list it splits, points into it |
| Pages, the page size in `mcfg` or a page's own size | pages, laid out by the engine as artboards side by side |
| Layers (drawing, desktop, guides and grid layers), the master page's among them | layers by name and kind |
| Guidelines (object type 0x16, and the version 7 `guid` chunk) | guidelines at their point and angle |
| Groups (`LIST grp `) | groups |
| Curves (point lists of moves, lines and cubic curves) and paths (object type 0x25, version 16 on) | paths |
| Rectangles (one corner radius, four from version 9) | rectangles with each corner's radius |
| Ellipses, arcs and pies | ellipses, and paths for arcs and pies |
| Polygons and stars (a base side copied for each side) | paths |
| Uniform fills: CMYK, CMY, RGB, gray, registration, library colors | colors, spot colors with their tints |
| Outlines: width, caps, joins, dashes, the no-outline flag | outlines |

Text, bitmaps, linked groups (blends, contours, brush strokes), B-spline curves, fountain,
pattern, texture and PostScript fills and outline arrowheads are not read; the drawing's
`warnings` name each kind it held. Drawings of versions before 5 and after 27 are refused, and so
are packages that are encrypted, ZIP64 archives or split across files.

## Provenance

`PROVENANCE.md` lists every record layout the reader relies on, with its public source and status.
Code cites the rows by id (`R1` ...), and `tests/provenance.rs` fails when a cited id has no row.
The test inputs are synthetic drawings built by `src/synth.rs` (the `synth` feature); no
third-party file is committed.

## Limits

`Limits` bounds the input size, the bytes inflated from compressed lists and package members, the
size of one package member, the number of members, the nesting depth, the chunk count, the
objects, the coordinates and the outline dashes of one read. Past a limit the reader stops with an
error or leaves the rest out with a warning; it does not recurse and does not panic. A package's
data stream is read once however many lines of `content/dataFileList.dat` name it. A redirected
list whose bytes overlap those of an earlier redirected list is left out with the damage warning.
Other records may share bytes, and the bytes read again may add up to the member limit; past it
the records are left out with the limit warning.
