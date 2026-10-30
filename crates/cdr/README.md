# Native CDR import

`vectorcraft-cdr` reads drawing records into editable VectorCraft paths, artboards,
layers, groups and embedded images. It does not extract a preview or run CorelDRAW, a converter,
native C/C++ code or a network service. The same reader builds for native and WASM.

The accepted format range is 6 through 27 (including minor format numbers within
that range). This is a **partial importer**, not a claim of complete compatibility
with every file or feature in those releases. A CDR 12 grammar fixture is generated
in Rust; original drawings saved locally by CorelDRAW 27.2 in formats 15, 16 and 27
were checked against that application's SVG exports. Those local CDR/SVG binaries
are not distributed in this repository. One supplied format-24.3 drawing with
embedded CMYK images, alpha and complex crops was also checked locally. Other
accepted layouts remain experimental; no historical CorelDRAW application
release was executed for these checks.

## Implemented

- RIFF records and `LIST cmpr` zlib streams with their indexed length pool.
- ZIP containers: `content/riffData.cdr` (14/15), `content/root.dat`,
  `dataFileList.dat` and external data streams (16+); inline redirect records.
  ZIP storage and deflate are supported, with CRC and local/directory validation.
- Version detection from `vrsn`/`fver`, which takes precedence over the RIFF letter
  (recent releases use extra letters for minor revisions).
- Per-page dimensions, y-up coordinates, multiple pages, names, layers, ordinary
  groups, linked-group transforms and front-to-back native paint order.
- Drawing-only files with zero-by-zero page dimensions: infer a warned artboard
  from the imported vector/image bounds, preserving scale and the first page's
  drawing origin. Valid explicit page dimensions still take precedence.
- Rectangles, full ellipses, ellipse arcs/pies, compound straight/cubic paths,
  affine object transforms and numeric RGB, CMYK, Gray and Lab colours.
- Modern rectangle round, scallop and chamfer corners, with independent corner
  values and absolute/relative scaling, retained as editable straight/cubic paths.
- Modern polygon/star single-side seeds, including their intermediate anchors
  and cubic handles, expanded with the native repetition step and axis scales.
- Modern native B-spline controls and clamped points, converted into editable
  cubic paths with open endpoints and curved closed seams.
- Referenced PowerClip contents, nested frames, frame fills/outlines and native
  alternate/winding fill rules. Frames and contents remain editable vector nodes.
- Solid fills; editable linear and radial fountain ramps, stops and midpoints;
  solid/dashed outlines with width, caps and joins; basic object opacity.
- Native RI bitmap resources: BGR/palette/grayscale and CMYK samples, separate
  continuous 8-bit alpha, object transforms and straight/cubic crop outlines.
  Images remain embedded image objects inside editable native clip groups.

## Current losses

Text, master-page artwork, inherited styles, JSON appearance/transparency
overrides, patterns, textures, arrowheads and embedded colour profiles are not
imported. Unsupported colour models use black with a warning. Legacy rectangle
corner layouts remain square with a warning; overlapping modern radii are clamped
with a warning. Alternate polygon seed modes, non-coprime compound-star steps,
displaced seed centres and unsupported PowerClip reference layouts fail explicitly.
PowerClip fitting/locking UI settings and complex transparency are not reproduced.
Conical/square fountains become radial; advanced
fountain placement and colour interpolation can be approximate. Spot identities,
overprint and exact calligraphic/scaled outline behaviour are not preserved.
Warnings describe omitted/approximated features. A file containing objects but no
supported editable objects fails instead of opening a misleading empty drawing.
A genuinely blank native drawing can open.

The numeric and rendering checks cover simple geometry, RGB fills, outline
widths, two pages, rotated/nonuniformly scaled groups, a horizontal fountain,
an elliptical radial fountain, CMYK cyan and Gray. They do not establish fidelity
for arbitrary production CorelDRAW files. CMYK and Gray preserve their numbers;
their displayed colour can differ when the source document's profile is omitted.
CMYK bitmap samples are approximated as RGB PNG pixels without an ICC profile,
with a warning. This conversion does not preserve the image's original ink values.

## Bounds

The input is limited to 128 MiB. Decompression has a cumulative 128 MiB budget;
materialized leaf records have a separate 128 MiB budget. Every recursive visit,
including an alias of an external LIST, consumes a cumulative 512 MiB traversal
budget. Other bounds: 64 nested lists, 200,000 records, 4,096 ZIP entries/streams,
1,000 pages, 20,000 styles, 2,000,000 geometry coordinate records (including cubic
controls), 1,000,000 editable anchors and 250,000 materialized
gradient stops/dash values (both style definitions and object copies count).
Referenced drawing expansion additionally limits interpretation to 200,000 record
visits and 64 recursive levels; cyclic or mismatched PowerClip references fail.
Exact repeated PowerClip frame/resource definitions from external LIST aliases
share indexing; distinct definitions reusing an identifier fail. Each occurrence
remains editable under its own page/parent transform. Materialized names have a
cumulative 16 MiB UTF-8 budget, including aliases; source decoding is bounded too.
Polygon expansion charges its generated point count before allocating the outline.
Native curves also charge their anchor bound before allocating their path, including
implicit starts after closed segments. They read borrowed coordinate/flag arrays
without another full point-vector allocation. Straight-heavy drawings and repeated
aliases cannot bypass the anchor cap; procedural expansion also takes the remaining
anchor quota before allocation. These quotas are independent of the input-byte limit.
Bitmap decoding additionally limits each side to 32,768 pixels, the cumulative
pixel count to 32 million and generated encoded PNGs to 128 MiB. Exact raw bitmap
aliases are skipped before decoding and consume no additional image quota. A
different encoding with the same identifier is decoded and accepted only when
its dimensions and complete PNG bytes agree; this actual decoding work remains
charged even when the final image is shared. Conflicting images fail. Row strides,
palette indices, RI bounds and separate alpha dimensions/overlaps are checked
before the RGBA allocation.
Malformed, encrypted, split and ZIP64 archives fail with an error. These are
resource bounds, not an assertion that arbitrary input can never exhaust a host
whose available memory is already smaller than the bounds.

Format recognition checks only the 12-byte RIFF header, including for stored or
deflated ZIP roots. Loading performs the complete size and CRC validation once.
The weak legacy `WL` prefix is not used for sniffing; a `.cdr` extension still
routes Waldo input to its explicit unsupported-version diagnostic.

## Verification

`cargo test -p vectorcraft-cdr` covers the generated editable drawing, equivalent
compressed RIFF, stored/deflated ZIP prefix recognition, header/CRC mismatch,
legacy-extension detection, every truncated prefix,
decompression/copy/traversal limits and property-based random/mutated input.
Original generated bitmap records check resource aliases, conflicting identifiers,
crop geometry, bottom-up rows, BGR,
CMYK, palettes, continuous alpha, image quotas and native save/reopen/place.
`CDR_FIXTURE_DIR` optionally points to the locally generated CorelDRAW fixture
directory. It runs the native-file assertions without committing binary fixtures.
The engine oracle suite also accepts `powerclip`, `corners`, `corner-types`,
`polygons`, `fill-rules` and `bspline` scenes saved in formats 15/16/27, each with its independent
page SVG. Those checks assert zero image resources for every vector-only scene.
Record observations and unresolved cases are in [the vector record notes](../../docs/cdr-vector-records.md).
`CDR_ZERO_PAGE_FIXTURE_DIR` optionally points to a local supplied `input.cdr` and
its independently exported `corel-artwork.svg`. The checked zero-page scene keeps
180 editable curves/424 subpaths and matches the separate SVG's geometry and paint.
`CDR_LARGE_CURVE_FIXTURE_DIR` optionally supplies a local control-heavy `input.cdr`
and independent `corel-artwork.svg`. The native example contains 1,068,736 coordinate
records but 468,205 editable anchors: 1,897 curves, 4,343 native subpaths, no images.
Corel's SVG may normalize subpaths, so its move-command count is not used to assert
native topology. Original curve counts are checked separately from rendering.
For the checked large file, Corel's SVG truncates one curve's coordinates, so
`corel-artwork-rgba-20261022.png` is the complete native-rendered coverage oracle.
The two rasterizers' half-coverage masks must agree outside a one-pixel edge band;
raw alpha and profile-dependent RGB differences are reported separately. This
does not change the existing original-scene SVG comparison limits.
`cargo clippy -p vectorcraft-cdr --all-targets --all-features -- -D warnings` checks
the shipped no-unsafe/no-panic policy as well as the fixture feature.

## Provenance

The reader is original Rust written from public format descriptions and checked
with original, locally generated drawings. No implementation from the existing
VectorCraft/TraceDraw import PR, Inkscape, UniConvertor or libcdr is vendored or
linked. Public descriptions used:

- [Kaitai CDR specification](https://github.com/kaitai-io/coreldraw_cdr.ksy/tree/9d8e2704b946961f5114115f1a488750a3c86ed0),
  MIT, copyright 2020 Kaitai team. Its grammar documents container/record layouts,
  reversed argument types, coordinates, curves, colours, fills and outlines.
  The attribution and license are retained in `LICENSE-kaitai.txt`.
- [Photopea format notes](https://github.com/photopea/CDR-specification): RIFF and
  compressed length-pool explanation, consulted as documentation only.
- [TraceDraw format notes](https://github.com/igtoth/tracedraw/blob/a834cfccdb912d573acb3f3fcaadfdc280bb44aa/docs/cdr-format.md),
  MIT OR Apache-2.0: cross-checks of paint order, group transform scope and newer
  record conventions. The documented RI alpha layout was cross-checked against
  its [reader](https://github.com/igtoth/tracedraw/blob/a834cfccdb912d573acb3f3fcaadfdc280bb44aa/crates/cdr/src/parse.rs#L3609);
  no implementation code was copied.

CorelDRAW's documented automation was used to create the local original fixture
drawings and SVG oracles. It is not required by the importer.
