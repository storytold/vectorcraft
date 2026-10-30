# CDR vector records: observed subset

**Last updated:** 2026-10-22. These notes describe CDR.8's recovered subset, not a
complete CDR specification. The reader stays independent of CorelDRAW at runtime.

## Evidence and method

Original geometric scenes were created with CorelDRAW 27.2.0.135, saved in formats
15, 16 and 27, and independently exported to SVG with presentation attributes,
millimetres, precision 5 and text-as-curves. Binary files, fonts, vendor resources
and user artwork are not distributed here. The optional `CDR_FIXTURE_DIR` engine
suite compares the resulting native documents against those independent SVGs.
The existing MIT Kaitai grammar supplies the container, argument-table and curve
field descriptions; its pinned URL and attribution remain in the CDR README.

Public automation documentation used to construct the original scenes:

- [Shape.AddToPowerClip](https://community.coreldraw.com/sdk/api/draw/27/m/Shape.AddToPowerClip)
- [Layer.CreateRectangle2](https://community.coreldraw.com/sdk/api/draw/27/m/Layer.CreateRectangle2)
- [Layer.CreatePolygon2](https://community.coreldraw.com/sdk/api/draw/27/m/Layer.CreatePolygon2)
- [cdrFillMode](https://community.coreldraw.com/sdk/api/draw/27/e/cdrFillMode)

Offsets below are within decoded argument payloads, after container indirection.
Native coordinates use 254,000 units per inch; object placement is a separate
affine transform. Static byte observations establish fields; independent exported
geometry corroborates their interpretation.

## PowerClip

`LIST doc / LIST clpt / LIST grp ` stores clipping contents outside the page tree.
The group's `spnd` word is its resource identifier. A frame object's argument
8005 contains four little-endian words: the contents identifier, `1`, `0`, `0` in
the checked files. Group argument 3500 refers back to the frame's `spnd`.
Both directions are checked, and missing/ambiguous/cyclic references are errors.
Each owner/resource identifier must have one unambiguous full definition. Exact
aliases share indexing while every occurrence is still drawn under its own parent;
distinct definitions are rejected. Actual expansion, including names, is charged:
a 16 MiB cumulative UTF-8 name quota prevents a compact branching alias graph from
materializing unbounded name data. Name source decoding is also bounded.

The contents already have complete transforms, including a rotated frame's
rotation. Applying the resource group's matrix again would transform them twice.
Native front-first records are reversed into paint order. The native clip group
contains the editable frame first, then its editable contents; its fill paints
below the contents and its outline above them. Nested references are resolved
recursively under cumulative visit, point and depth limits.

Evidence covers rectangular, elliptical and compound-path frames, painted and
unpainted frames, multiple contents, rotation and nested clips. Other reference
layouts, fitting/locking settings and complex transparency remain unverified.
The normal document depth limiter can warn about excessively deep artwork.

## Drawing-only zero pages

A supplied format-24.3 drawing has zero width/height in both `mcfg` and package
metadata, and no per-page dimension override. CorelDRAW27.2 also opens it as a
zero-by-zero page at drawing origin0,0, while exposing 180 curves,424 subpaths and
7386 segments. Its artwork spans200 by191.3496mm and exports as180 SVG paths,
without images. Those are observations of one local file, not a new universal
interpretation of all missing metadata.

The recovery policy treats a complete zero-by-zero size as unavailable. Valid
explicit dimensions or document defaults are used where available; otherwise the
imported visual bounds define the artboard, with a warning. The first page keeps
its native zero origin and scale; later inferred boards are translated into the
usual page row. A degenerate span gets a one-point extent, and an empty unsized
page gets the existing default board. Partial zeros, negative sizes and oversized
inferred bounds still fail. Embedded previews are never used as content or sizing
authority. The local oracle checks full colours and independently recoloured
geometry/stacking, retaining the existing0.1/255 mean-byte threshold.

## Modern rectangle corners

Argument 30 has a 32-byte header: width, height, x scale and y scale as four
little-endian doubles. Four 24-byte slots follow in lower-right, upper-right,
upper-left, lower-left order. Each observed slot contains:

| Offset | Value |
|---|---|
| 0 | u32 radius mode: 0 relative, 1 absolute |
| 4 | reserved u32, zero in checked records |
| 8 | f64 radius |
| 16 | u32 treatment: 0 round, 1 inward scallop, 2 chamfer |
| 20 | reserved u32, zero in checked records |

Relative radii multiply half the shorter absolute local side. Absolute radii
are native lengths divided by the smaller absolute stored scale. The object's
affine transform is then applied once. Round and scallop handles use `0.55` of
their local radius in the observed SVGs; chamfers are straight segments.
All-zero slots are valid square corners. Signed dimensions remain signed.
Unknown metadata falls back square with a warning; radii beyond half the shorter
side are clamped with a warning because overlap resolution is not established.
Legacy corner layouts retain their previous warned fallback.

## Polygon and star seeds

Object kind 20 uses ordinary point/flag geometry in argument 30, describing one
side rather than the full shape. Argument 11000 in the checked files has 48 bytes:
u32 side count, mode `1`, repetition step, reserved `0`; f64 x/y scales; two zero
centre words; an editing hint; a zero tail word. The hint's effect is already
present in the seed geometry and is not applied again.

Repeat the actual seed clockwise by `2π * step / side_count`, preserving its
intermediate anchors and cubic handles, then apply its axis scales. Join adjacent
sides within integer coordinate quantization and close only the full outline.
The original probes include a hexagon, concave five-point polygon, seven-point
complex star with step 3, and a stretched/rotated eight-sided polygon. Their
placed SVG anchors agree within 0.000115 mm. The v15/v16 geometry arguments agree
byte-for-byte; v27 omits the older eight-byte zero seed trailer.

Repeated output is bounded before allocation. Non-coprime compound-star steps,
displaced centres, alternate modes and unsupported flags return explicit errors.
Cubic seeds and unequal seed scales have synthetic tests but no application
oracle yet; no regular-polygon substitute is used for unknown geometry.

## Compound-path coverage

Original same-direction outer/inner contours establish alternate (even-odd)
coverage by default. Setting the public shape fill mode to winding adds argument
14001 with u32 value `1`; the independent SVG then has `fill-rule="nonzero"`.
The same distinction is present in PowerClip's `clip-rule`. Preserve that rule
on the frame instead of forcing all masks to even-odd or all paths to nonzero.

## Native B-splines

Kind 38 (`0x26`) has no ordinary coordinate argument. Its direct `LIST npps`
contains a `ppid` identifying this geometry with bytes
`ef e9 0e 66 4c a5 bc 40 91 0b 8a 16 16 c1 5e 4a`, and a `ppdt` payload:
u16 control count, u16 closed flag, u16 reserved zero, signed i32 x/y pairs,
then one u32 clamped flag per control. Checked payloads are byte-identical in
formats 15/16/27. Other property identifiers are not guessed as geometry.

For controls P(i), an unclamped interior anchor is
`(P(i-1) + 4*P(i) + P(i+1))/6`; a clamped control and an open endpoint are
interpolated. The cubic from anchor i to i+1 has handles
`(2*P(i)+P(i+1))/3` and `(P(i)+2*P(i+1))/3`. Closed curves wrap neighbours
and retain the last cubic, rather than closing with a straight line.
The original open/closed probes check 11 cubics against independent SVGs;
maximum control/position discrepancy is 0.000067 mm. Other layouts and
unobserved endpoint conventions remain experimental.
The output quota includes every handle and is checked before allocation.

## Remaining work

Text/font/style reconstruction, master-page artwork, patterns,
colour profiles and Corel-specific effects remain outside this subset. Existing
embedded bitmaps remain images, while genuine vector objects remain paths. No
thumbnail, screenshot or tracing pass is substituted for native geometry.

## Control-heavy curve capacity (CDR.10)

A second supplied format-24.3 file has 1,068,736 encoded point records, including
596,188 cubic controls. Independent Corel27.2 observations count 1,897 curves,
4,343 subpaths and 468,205 editable nodes/segments, with no bitmaps or text. Its
page is A4 portrait and the artwork is off-page. The local SVG oracle has the same
path-object count but normalizes some subpaths; its move count is not a topology
oracle. The source and all derived artwork remain private.

The importer separates coordinate-work capacity (two million records) from native
editable storage (one million anchors), rather than removing its geometry bounds.
Each native curve pre-counts a conservative anchor bound, including implicit starts
after close, and borrows the source arrays instead of building a temporary full point
vector. Rectangles/ellipses charge conservative bounds and generated geometry
receives the remaining coordinate and anchor quota before expansion. Duplicate
occurrences still consume both quotas. Unit checks exercise the old million-record
boundary, cubic-handle preservation, both independent ceilings and repeated curves.

The local native import retains all Corel-observed paths, subpaths and anchors;
three closed single-anchor subpaths have no drawable segment in PathData, rather
than being lost. The original SVG is demonstrably incomplete: one 91,621-coordinate
curve is serialized only through coordinate 26,085 (the native count modulo 65,536).
Its command roles/coordinates match that native prefix; the omitted 840 moves
explain the entire subpath-count discrepancy and missing horizontal extrema.
This is consistent with an export-count truncation; the exporter implementation
was not inspected. No scaling correction or relaxed threshold is inferred from
that incomplete oracle. A separate Corel-rendered 1600 by 1595 RGBA PNG is the
coverage oracle; native object/node counts are checked independently. Since it
uses a different rasterizer, half-covered binary masks must agree away from an
explicit one-pixel boundary: all 2,924 differing mask pixels are within that band,
with zero disagreements outside it. Raw alpha MAE is 0.296852/255; premultiplied
RGB MAE is 4.001351/255. These are not claims of pixel equality or of passing the
existing shared-renderer SVG byte threshold, which remains unchanged for the
other scenes. CMYK numbers remain native and a warning states that embedded
profiles are not imported.
