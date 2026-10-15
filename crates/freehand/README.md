# vectorcraft-freehand

A bounded Rust reader for Macromedia / Aldus FreeHand drawings (`.fh*`) and FreeHand templates
(`.ft*`), product versions 8 through 11. It reads the AGD container, the record list and the part
of the document model listed below, and hands the engine pages and paths in y-down points at 72
per inch, with every transform composed. It does not write FreeHand files.

## What is imported

| FreeHand | VectorCraft |
|---|---|
| Pages (`VMpObj` on the document `pages` list), or the trailer page size when no page is stored | artboards, keeping their relative positions |
| Layers, back to front. Visibility 3 is shown | layers. Any other visibility is hidden, and still holds its objects. The file has no documented lock bit, so layers are not locked |
| Groups, including a group whose child list is empty, and their transforms | groups. Transforms are baked into absolute coordinates |
| Clip groups whose first child is a path | clip groups |
| Composite paths | compound paths (even-odd when a child says so) |
| Paths (cubic nodes, closed, even-odd) | paths |
| Rectangles, including a corner radius, ovals (full ellipses; version 11 arcs as polylines), polygons and stars | closed paths |
| Solid fills and strokes, dash patterns | solid paints and dash arrays. Caps stay butt and joins stay miter: the file stores a miter-like number and a 4-byte tail, and neither is a confirmed cap or join |
| Linear gradients (`TaperedFillX` with a stop list) and radial gradients (`RadialFillX`) | linear and radial gradients, when a shown object uses them |
| Text blocks (`TextColumn`, `TextInPath`, `TFOnPath`) | text, best effort: string, font name, size, box and colour |
| Symbol instances | the symbol's artwork, with the instance matrix |

Y-up page space is flipped so each page's top-left is the artboard's origin.

## Not imported

These are skipped or approximated. Each kind is named once in the warnings, with a count.

Placed images (`ImageImport`, `SwfImport`) are left out. Tile fills are left unfilled. Pattern
fills and pattern strokes become their solid colour. Procedure (custom) paints and PostScript
paints become a solid colour. Lens fills become a flat colour (or nothing, for magnify and
invert). Conic gradients become linear. Contour gradients become radial. Tapered gradients
without a stop list become linear. A content fill (the `contents` paint, a drawing used as a
fill) becomes a clip group: the shape clips that drawing, and a stroke on the shape is kept.
When the referenced art is missing or empty, the shape stays and one warning is recorded. A
zero id is no paint. Blends are drawn from the objects already stored, not regenerated.
Arrowheads are left off: `ArrowPath` has a known size and is a closed cubic, and the description
does not say how that art is placed, scaled or rotated onto the stroke. Brushes, envelopes,
perspective, extrusion, shadows, glows and other filters are not applied. Guides are not drawn.
Records that no layer reaches (unused swatches, a symbol master that is never instanced) are
not drawn. Sizes that the description does not fix refuse the file instead of guessing.

## Versions

VectorCraft reads FreeHand 8, 9, 10 and 11 (MX). Those are the versions whose record sizes the
format description fixes: `AGD3` is 8, `AGD4` is 9, `AGD5` is 10 and `AGD6` is 11. FreeHand 10
and 11 also put `FreeHand10` or `FreeHand11` in the first 24 bytes; when that text and an `AGD`
digit disagree, the digit chooses the layout. Version 8 stores the record stream raw and uses
the version-8 sizes (dictionary tail, transforms, block, list padding, colour tails). Versions
9–11 zlib-compress that stream.

Anything else that is still FreeHand is refused. That covers
FreeHand 1 (`acf3`), 2 (`FHD2`), 3 (`FH31`), 4 and 5 (`AGD1`), 7 (`AGD2`), the `AGD0` digit, and
files that begin `ALDUS LZW`. The error names the version and says VectorCraft opens FreeHand 8
to MX (11). A `FreeHand10` or `FreeHand11` text with no `AGD` header is recognised and reported
as cut off in the header. The optional `0x1C` chunk wrapper is followed until the `AGD` header.

## Errors

A file that cannot be opened returns one of these sentences, with no record index and no
decompressor diagnostic:

* `the FreeHand file is empty`
* `the FreeHand file is cut off in the header`
* `not a FreeHand file`
* `the FreeHand file is cut off in its compressed data`
* `the FreeHand file's compressed data is damaged`
* `the FreeHand file is cut off in the dictionary`
* `the FreeHand file is cut off in the record list`
* `the FreeHand file is cut off in a record`
* `the FreeHand file claims more records than can be read`
* `the FreeHand file claims a dictionary larger than can be read`
* `the FreeHand file claims more path nodes than can be read`
* `the FreeHand file claims more list entries than can be read`
* `a FreeHand record id points past the end of the file`
* `the FreeHand file decompresses past the safety limit`
* `the FreeHand file's data block runs past the end of the file`
* `the FreeHand file is larger than can be read`
* `the FreeHand file claims more entries than can be read`
* `the FreeHand file has a record whose size is not known`
* `unsupported FreeHand version {n}: VectorCraft opens FreeHand 8 to MX (11); open it in a newer FreeHand and save, or export it as EPS, PDF or AI`

The dictionary count and the list and node counts are 16-bit, so the largest claim they can
make is 65535. A record count is 32-bit. An escaped record id (`0xFFFF` plus a u16) that lands
past the record list is refused. A plain 16-bit slot that is not an id, such as a block field,
is not.

## Provenance and clean-room

FreeHand has no file-format specification we can use, so this reader is built only from a written
description of the format. FreeHand has been owned by Adobe since 2005, and no fixture was made
with it.

* **No FreeHand or Adobe software.** FreeHand and Adobe applications were never downloaded,
  installed, run, scripted or disassembled for this work. No file was made with FreeHand. The
  tests are synthetic files built by the `synth` feature.
* **A written description, produced in a separate step.** The container and record layout was
  learned from a written description produced in a separate step from libfreehand (MPL-2.0,
  LibreOffice/DLP, <https://git.libreoffice.org/libfreehand> at
  `85ac573dae4dd4569dc4ad2e12b3a4b80fb4a5c2`) and re-described in our own words before this Rust
  was written. No code was translated or copied.
* **Checked locally, not committed.** The reader was checked locally against a FreeHand 9 file
  its owner shared. That file is not in this repository.

## Validation

* Unit tests on the synthetic files, including an empty group, a content fill, a hidden layer,
  each supported version, each refused version, a symbol that contains itself, and groups nested
  past the depth cap.
* Property tests of random bytes and of mutations of a synthetic file.
* `engine/tests/import_fuzz.rs` mutates a synthetic file and checks that whatever opens also
  renders and exports. A file with groups nested 10_000 deep, and a file with a record cycle,
  are opened on a 1 MB stack: they open within the depth cap, or the reader refuses them, and
  neither overflows.
* `fuzz/` has `cargo-fuzz` targets. `read` feeds arbitrary bytes to `is_freehand` and `read`.
  `synth` builds a FreeHand 8–11 file from the input (counts, ids, matrices, truncation) and
  reads it. Build and smoke them from the harness crate (libFuzzer, nightly):

  ```sh
  cd crates/freehand/fuzz
  # cargo-fuzz 0.13 does not accept -j. Cap the inner cargo build with CARGO_BUILD_JOBS.
  CARGO_BUILD_JOBS=6 cargo +nightly fuzz build
  cargo +nightly fuzz run read -- -max_total_time=60 -rss_limit_mb=2048 -timeout=10
  cargo +nightly fuzz run synth -- -max_total_time=60 -rss_limit_mb=2048 -timeout=10
  ```

  Seeds are not stored in git. `FREEHAND_SEED_DIR=/path cargo test -p vectorcraft-freehand -- --ignored write_fuzz_seeds`
  writes the synthetic fixtures into that directory. Copy them into `fuzz/corpus/read` or
  `fuzz/corpus/synth` locally.

## Safety limits

Every size is checked before it is allocated (`Limits`): 64 MiB of input, 256 MiB decompressed,
250_000 records, 8_192 dictionary names, 2_000_000 list entries and path nodes together, strings
of at most 1_048_576 bytes. The inflated stream is capped before it grows. Record-graph cycles,
a symbol whose art contains that symbol, style chains and group nesting stop at 64 levels in
this crate; the engine then merges anything past its own document depth, the same way it does
for the other importers. A non-finite coordinate or matrix is left out or replaced with the
identity, with a warning. Malformed input returns an `Error`. The crate has no panics outside
tests.
