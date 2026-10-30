# Native CorelDRAW import

> **Last updated:** 2026-10-22 · CDR.8: editable PowerClip, modern corners,
> polygon/star seeds, B-splines and compound-path fill rules. Compatibility remains partial.

CDR.1 adds read-only `.cdr` import to the shared engine. File > Open, File > Place,
drag and drop, the CLI, MCP and the browser use the same Rust reader. Open creates
editable paths, artboards, layers and groups. Save asks for a new VectorCraft
destination; the CDR source is never overwritten. There is no CDR exporter.
Embedded native bitmap resources also retain their image data, continuous alpha,
object transforms and editable crop outlines. CMYK bitmap colours are converted
to RGB without the source ICC profile, with an approximation warning.
PowerClip contents now become native clipping groups with editable frames and
children. Modern rectangle corners and supported polygon/star seeds become
straight/cubic paths; compound holes retain their alternate or winding rule.
Drawing-only CDRs whose page metadata is zero-by-zero recover an artboard from
the imported artwork, with a warning. The first page retains its native origin
and physical curve scale. A valid per-page size overrides a missing document default;
partial-zero/negative dimensions still fail. Empty unsized pages use a warned default.

The macOS bundle and Linux desktop entry also advertise CDR as a readable format.
Linux uses the existing `application/vnd.corel-draw` type from
[shared-mime-info](https://cgit.freedesktop.org/xdg/shared-mime-info/tree/data/freedesktop.org.xml.in)
rather than declaring a competing MIME type in the app's package.

This is partial interoperability. Unsupported art is reported in the import
warnings. The reader does not substitute an embedded thumbnail or a rendered
preview for editable objects.

## Containers and version coverage

CDR is a family of binary formats. The reader handles little-endian RIFF records,
their `LIST` hierarchy, compressed `cmpr` lists, and ZIP packages. X4/X5 packages
carry `content/riffData.cdr`; newer packages carry `content/root.dat` with records
that reference the streams named by `content/dataFileList.dat`. Compressed RIFF
lists have two zlib streams: the record data and a table of record lengths.

The object argument tables refer to native geometry, transforms and fill/outline
styles. Those records are decoded into the document model, without launching a
converter or requiring CorelDRAW on the importing machine. The implementation
and its public source references are in [the CDR crate](../crates/cdr/README.md).

| File layout | Evidence |
|---|---|
| CDR 12 RIFF | Original, generated record fixture in unit and engine tests |
| CDR 15 ZIP | Original rectangle and two-page geometry files saved by CorelDRAW 27.2 |
| CDR 16 ZIP with external streams | The same original scenes saved by CorelDRAW 27.2 |
| CDR 24.3 ZIP with external streams | One locally supplied drawing: two CMYK bitmaps, exact continuous alpha and native crop outlines; checked with CorelDRAW 27.2, kept private |
| CDR 27 ZIP with external streams | Original scenes saved by CorelDRAW 27.2, including its extended version and fill records |
| Other accepted versions between 6 and 27 | Experimental record-layout coverage; not a claim of full fidelity or a verified historical application version |
| Waldo (CDR 1/2), CMX, encrypted packages, ZIP64, newer unknown versions | Unsupported; returned as errors |

The installed CorelDRAW 27.2 did not produce files for requested save versions
6, 10, 12, 13 or 14, despite reporting no COM error. These attempts do not verify
older versions. No claim of universal CDR support is made.

## Verification and remaining work

The local test artwork is original: a red rectangle with a black outline, an
ellipse, a cubic curve, grouped objects, a rotated rectangle and two pages.
CorelDRAW independently exports each page to SVG. The engine test renders both
the native CDR import and that SVG and compares their dimensions and pixels.
The fixtures and application resources remain local; no Corel fonts, profiles,
vendor artwork or binaries are redistributed.

Corel-specific effects, text and other unsupported records
remain outside the initial import subset. Warnings state what was omitted or
approximated; see the crate README for the exact implemented paint and geometry
coverage. Export SVG or PDF from CorelDRAW when a file depends on those features.
Editable linear and radial fountain fills are also imported; advanced placement and
colour interpolation can be approximate. The feature and format parity scores
remain unchanged.

The parser enforces limits on input bytes, expanded data, copied record payloads,
record and point counts, nesting, pages and coordinates. ZIP CRCs and declared
sizes are checked. Unit/property tests cover truncated, mutated and hostile
records; the engine fuzz tests also open, place, render and export successful
imports.

CDR.10 distinguishes up to two million coordinate records (including cubic control
points) from at most one million editable anchors. Native curve points are streamed
from borrowed arrays rather than copied into another full point vector. Both quotas
remain cumulative across objects, crops and referenced occurrences. Generated
polygon/B-spline paths also respect the unspent anchor quota before expansion.
This permits control-heavy production drawings without allowing two million
straight anchors or unbounded repeated geometry.

Run the synthetic checks with `cargo test -p vectorcraft-cdr` and
`cargo test -p vectorcraft-engine cdr`. To run the independent application-oracle
checks, set `CDR_FIXTURE_DIR` to a local folder containing
`rectangle-v15.cdr`, `rectangle-v16.cdr`, `rectangle-v27.cdr`,
`geometry-v15.cdr`, `geometry-v16.cdr`, `geometry-v27.cdr`,
`rectangle-page1.svg`, `geometry-page1.svg` and `geometry-page2.svg`, then run
`cargo test -p vectorcraft-engine --test cdr_files -- --nocapture`.
`CDR_RENDER_DIR` optionally saves the compared PNGs outside the repository.
When also present, `gradient-v27.cdr` / `gradient-page1.svg` and
`group-transform-v27.cdr` / `group-transform-page1.svg` extend the render checks
to fountains and nonuniformly transformed, rotated groups.
`CDR_ZERO_PAGE_FIXTURE_DIR` can separately point to a private `input.cdr` and
`corel-artwork.svg`: a supplied format-24.3 zero-page drawing checked through Corel27.2
has 180 curves,424 subpaths and no images. Its original and derived artwork stay local.
The engine checks its recovered size, object counts, original paint, and a second
distinct-colour geometry/stacking comparison against the independent SVG.
Optional `bitmap-v15.cdr`, `bitmap-v16.cdr`, `bitmap-v27.cdr` and
`bitmap-page1.svg` plus the original `bitmap-original-checker.png` cover original
RGB/CMYK images with 0/64/128/255 alpha.
Their rendered alpha/placement and RGB samples are checked independently;
CMYK-to-RGB profile differences are reported separately from geometry fidelity.
Optional `powerclip`, `corners`, `corner-types`, `polygons`, `fill-rules` and `bspline`
scenes use `NAME-v15.cdr`, `NAME-v16.cdr`, `NAME-v27.cdr` and `NAME-page1.svg`.
These cover nested/rotated clipping, multiple ordered contents, painted frames,
same-winding holes, explicit nonzero winding, asymmetric round/scallop/chamfer
corners, both corner scaling modes, concave polygons and a complex seven-point star.
The B-spline scene covers open endpoints, cyclic controls and a clamped interior
point; imported geometry retains its cubic handles rather than flattened segments.
Every vector-only imported scene must have zero image resources. A separate engine
regression edits a clipped child, undoes, saves/reopens and places/undoes the drawing,
and requires a vector `<clipPath>` export. See [record notes](cdr-vector-records.md)
for observed fields and the remaining boundaries.

When all visible artwork lies outside every page and no saved camera exists,
the desktop's initial view fits that artwork after the canvas has a size. The
original page and artwork coordinates stay unchanged. Explicit Fit Artboard and
saved native viewports keep their existing behavior.

Use a 100 by 100 mm page, with the rectangle at drawing coordinates
(10, 60) mm, 40 by 20 mm, red RGB fill and a 1 mm black outline. The geometry
scene adds a grouped blue ellipse, a green cubic curve and a rotated rectangle on
page 2. Save each scene in the named versions and export each page as SVG 1.1,
millimetres, embedded images and presentation attributes. The initial fixtures
inherit Corel's drawing origin: its SVG and native records both include a
roughly 0.038 mm origin offset. Compare against the exported page, rather than
assuming the UI's drawing coordinates are the page coordinates.

## Revision history

| Date | Change |
|---|---|
| 2026-10-22 | CDR.10: separate coordinate-work and editable-anchor budgets and stream native curve coordinates; optional control-heavy private-file oracle and boundary/alias regressions. CDR.9 history entries completed in the progress documents. |
| 2026-10-22 | CDR.9: accept exact PowerClip aliases with bounded name materialization; recover drawing-only zero-page files as editable vectors with a fitted artboard. External-reference, branching-graph, synthetic page and local independent-oracle regressions cover both fixes. |
| 2026-10-22 | CDR.8: native PowerClip references, modern corner treatments, polygon/star seeds, B-spline cubics and alternate/winding fills; original 15/16/27 SVG oracles and bounded reference expansion. |
| 2026-10-19 | CDR.4: native embedded images, continuous alpha and editable crop outlines; initial framing of off-page artwork. |
| 2026-10-19 | Initial bounded native import and original CorelDRAW application-oracle validation. |
