# Image Trace quality

> **Last reviewed:** 2026-10-14 · **Last updated:** 2026-10-17 · **Change:** minor (Image Trace: Auto Color keeps rare but distinct colours; earlier: Gradients: grown over noise and soft rims)
> **Last reviewed:** 2026-10-14 · **Last updated:** 2026-10-17 · **Change:** minor (Image Trace: large images trace with a warning; anchor limit raised to 8,000,000; earlier: Gradients: grown over noise and soft rims)
> **Last reviewed:** 2026-10-14 · **Last updated:** 2026-10-17 · **Change:** minor (Image Trace: Raster::new returns an error for a buffer of the wrong length; earlier: Gradients: grown over noise and soft rims)
> **Last reviewed:** 2026-10-17 · **Last updated:** 2026-10-17 · **Change:** minor (Create Strokes: maximal-disc pruning and junction merging; centre-lines rerun)
> **Last reviewed:** 2026-10-17 · **Last updated:** 2026-10-17 · **Change:** minor (Create Strokes: exact crop distances and Guo–Hall thinning; centre-lines rerun)
> **Last reviewed:** 2026-10-17 · **Last updated:** 2026-10-17 · **Change:** minor (centre-lines baseline; no tracing behaviour change)
> **Last reviewed:** 2026-10-14 · **Last updated:** 2026-10-16 · **Change:** minor (Image Trace: a higher Paths never gives fewer segments on collapsed loops)
> **Last reviewed:** 2026-10-14 · **Last updated:** 2026-10-16 · **Change:** minor (Image Trace: the notch rule reads only its own piece)
> **Last reviewed:** 2026-10-14 · **Last updated:** 2026-10-16 · **Change:** minor (Image Trace: an edge's sub-pixel shift is the same from either side)
> **Last reviewed:** 2026-10-14 · **Last updated:** 2026-10-16 · **Change:** minor (Image Trace: a simpler contour walk with no precomputed direction table)

What Image Trace (`crates/trace`) makes of images whose right answer is known, measured by a harness and
compared with the numbers before each change. Nothing here is a judgement by eye, and nothing is compared
with any other product except the open tracer named under [Yardstick](#4-yardstick).


## Auto Color keeps rare colours

The Automatic palette (Auto Color) counts flat art's colours by area: a colour needs about 1% of the
picture (0.5% at Colors 100) to be counted, and the count is then fitted by median cut (Heckbert 1982)
and k-means (Lloyd 1982), weighted by area. A stem, a highlight or a cream close to a pale background
was averaged into a large neighbour, and Noise could remove what was left of it. Now, on the flat route
only, after the count is fitted, the colours too rare or too close for it are found and kept when they
are:

- distinct: at least 2.5 (Oklab × 100; Ottosson 2020) from every palette colour and from the main
  colour each one stands for;
- flat: mostly one histogram bin;
- drawn: a 4-connected region of their own (not blended) pixels of at least 8 pixels, or three of at
  least 4 (repeated highlights); a colour lying between two others, as an edge's anti-aliasing mixes
  them, must also have an inside (pixels whose four neighbours are all of it).

At most six are kept, largest region first. They are fixed while the other colours are refitted around
them, mixed-colour merging leaves them alone, and Noise removal keeps their regions of 4 pixels or more
(smaller specks of them still go). This follows the idea of salient-colour preservation in palette
reduction (Fukuda et al. 2019, doi:10.1117/12.2521607); the rule and its numbers are VectorCraft's own.
Full Tone, Limited and the continuous-tone route of Automatic are unchanged.

Tests (code-drawn): `automatic_keeps_a_rare_distinct_stem_or_patch` (2, 3, 5 and 8 flat colours and a
contrasting stem 1–4 px wide or a patch, about 0.05% to 0.5% of the picture, hard and anti-aliased at
1× and 2×: the stem's colour is kept and no other is added), `automatic_keeps_near_white_highlights_on_a_pale_background`
(ten white islands of about 12 px on #FFF4E0: kept, each still a path at Noise 16),
`automatic_keeps_a_thin_low_contrast_stem` (#909090 on #808080, 3 px), `scattered_noise_adds_no_colour`
(the negative control: the same areas as single scattered pixels add no colour) and
`noise_keeps_a_kept_rare_colours_regions_but_not_its_specks` (Noise 0 to 32).

Measured on 22 synthetic flat-colour logos drawn from known palettes, each hard 1×, anti-aliased 1×
and 2× (66 inputs, used locally and not committed; a key colour is missing when no traced colour is
within ΔE00 5): every key colour and no spurious one in **66 of 66** (was 39); missing key colours 0 (was 43),
spurious 0 (was 2), key colours sharing one traced colour 0 pairs (was 46); mean rendered ΔE00 against
the input 0.138 (was 0.348); paths 2547 → 2586, anchors 27,204 → 24,896, trace time 2.85 → 3.02 s in
all. On four public-domain scan crops (1024 × 1024) Auto Color takes the continuous-tone route and
nothing changes: 14, 72, 199 and 98 colours, the same paths, anchors and rendered error (2.11, 5.66,
9.15 and 1.00 ΔE00), time within run-to-run spread.


## Large images
Images over 1500 px on a side are traced with a warning that they trace more slowly (Flat Logo names
the size it traced at instead; see
[image-trace-adjustments.md](image-trace-adjustments.md)), and the anchor limit that guards memory is
8,000,000 (2,000,000 on the web) instead of 2,000,000. Measured through `imageTrace.make` (release
build, 4 cores, one trace at a time; memory is the peak allocated above the placed image, and what the
document keeps after the trace) on code-drawn photo-like pictures (40 soft-edged shaded discs on a
gradient, texture, sensor noise):
| size | preset | time | anchors | paths | peak | kept |
|:--|:--|--:|--:|--:|--:|--:|
| 2000 × 2000 | High Fidelity Photo | 4.1 s | 737,478 | 16,064 | 152 MB | 52 MB |
| 2000 × 2000 | Low Fidelity Photo | 1.6 s | 114,008 | 1,486 | 142 MB | 7 MB |
| 2000 × 2000 | Default | 0.9 s | 103,084 | 1,001 | 117 MB | 6 MB |
| 3000 × 2000 | High Fidelity Photo | 6.4 s | 1,013,114 | 25,256 | 236 MB | 75 MB |
| 3000 × 2000 | Default | 1.2 s | 145,756 | 1,574 | 162 MB | 9 MB |
| 4000 × 3000 | High Fidelity Photo | 12.8 s | 1,914,018 | 58,638 | 471 MB | 150 MB |
| 4000 × 3000 | Low Fidelity Photo | 5.1 s | 258,826 | 4,833 | 428 MB | 18 MB |
| 4000 × 3000 | Default | 2.4 s | 273,392 | 3,568 | 317 MB | 17 MB |
| 4000 × 3000 | 16 Colors | 3.0 s | 103,222 | 2,434 | 364 MB | 7 MB |
| 4000 × 3000 | Full Tone, Colors 85, Paths 90, Noise 4 | 29.6 s | 5,230,847 | 164,687 | 684 MB | 412 MB |
| 4000 × 3000 | Full Tone, Colors 100, Paths 100, Noise 1 | 37.2 s | 7,885,709 | 338,561 | 876 MB | 685 MB |
| 6000 × 4000 | Full Tone, Colors 85, Paths 90, Noise 4 | 106.1 s | 10,445,409 | 398,772 | 1169 MB | 872 MB |
| 6000 × 4000 | Full Tone, Colors 100, Paths 100, Noise 1 | 116.3 s | 13,508,871 | 658,133 | 1456 MB | 1230 MB |
(The last two rows were measured without a limit; they are refused now.) The 4000 × 3000 High Fidelity
Photo trace was 86,000 anchors under the old limit; a noisier scan of the same size would have been
refused. At 8,000,000 anchors a trace keeps about 0.7 GB, with a peak of about 0.9 GB while tracing.
## Collapsed loops keep few segments
A small closed loop (or a piece that returns to its own junction) whose fit collapsed to a single
cubic cannot stay as one, so it kept every positioned point as a straight segment. A looser Paths
collapses more loops, so it could give more segments than a tighter one. Such a loop now tries the
fewest segments that still hold it, each only if the result keeps an area (more than 0.25 px²) and
stays within 1.5 × max(tolerance, 0.5 px) of the points both ways: the fit at up to three somewhat
tighter tolerances (what a slightly higher Paths gives); for a closed loop then two cubics split at
its start and its farthest point, then the outline simplified at the tolerance and finer; the
positioned points last, as before.
Test on code-drawn images: `a_higher_paths_never_gives_fewer_segments` (32 seeded 48 px images of
soft blobs in four colours plus pixel noise, Color and Black and White, Noise 1 and 3, segments at
Paths 0, 25, 50, 75 and 100 must not fall). Before, six runs fell, e.g. 125 → 79 segments from
Paths 0 to 25 (seed 21, Color, Noise 1) and 71 → 63 → 52 from Paths 0 to 50 (seed 3, Black and
White, Noise 3); now they are 75 → 79 and 41 → 48 → 52. One image (seed 16 in Black and White) still
falls from 140 to 136 segments between Paths 0 and 25, before and after this change; that fall has
another cause, still open, and the image is left out of the test. The crate's other tests pass unchanged.
## Notches read only their own piece
Where the pixel at a sharp corner falls to the other colour, the outline cuts the corner in two
one-pixel steps, and between two straight edges of three cracks or more those two steps become one
point, where the two edges meet (a notch). Whether the edges were long enough was measured on the
whole loop. An edge can run on past a junction in one region's loop while the region across it,
whose loop turns at that junction, sees a shorter edge: one side made the notch and the other did
not, the shared piece got different points on each side, and the abutting curves left a gap. The
rule in `subpixel::outline` now also requires the three cracks of each edge next to the notch to lie
in the piece (no junction among them), as the corner-pixel correction above it already did.
Test on code-drawn images: `noisy_colour_images_leave_no_sample_uncovered` (20 seeded 36 px images
of soft overlapping blobs in four colours plus pixel noise; Color, 5 colours, Noise 1 and 2, Paths
0, 50 and 100; five sample points per pixel must each lie in some path). Without the condition 93
sample points were uncovered (seeds 9, 11 and 13); with it none are. The crate's other tests pass
## Same sub-pixel shift from either side of an edge
Each crack between two regions is moved by a sub-pixel shift read from the image. The reading
looked for a thin run (one to three pixels) on the crack's inside before one on its outside, so where
both sides were thin (a one- or two-pixel sliver beside another, as in soft gradients and noisy
scans) the two regions of the crack got different shifts, their shared piece different points, and
the abutting curves left slivers that no path covers. `Layers::shift` now averages the reading from
either side, `(s(c) - s(c flipped)) / 2`, which is exactly antisymmetric in floating point, so both
regions get the same points.
Tests on code-drawn images: `a_crack_between_thin_slivers_has_the_same_shift_from_either_side` (a
noisy ripple of 1–2 px columns at three thresholds: every crack's shift is the exact negative of its
flipped crack's, and the image does have cracks the one-sided reading gets wrong) and, in
`tests/shared_boundaries.rs`, `thin_slivers_on_both_sides_of_an_edge_leave_nothing_uncovered` (a
48 px noisy ripple in Black and White at thresholds 140, 157, 170 and Paths 0, 50, 100, sampled on
the 4 × 4 grid per pixel the shared-boundary check uses). With the one-sided reading 482–1,228
sample points per trace were uncovered; now none are. The crate's other tests pass unchanged.
## A simpler contour walk
`contour::trace_mask` follows each region's outline along the pixel cracks with the inside on the
right. It used to build a table of outgoing directions for every pixel corner in a raster pass and
then follow it. It now works out each turn during the walk from the two pixels ahead of the current
corner (right if the one ahead on the right is outside, straight if the one ahead on the left is,
else left; so at a saddle it turns right and diagonal pixels stay apart, as before). Loop starts are
found by scanning the pixels for a top edge on the outline, with one flag per pixel for a top edge
already walked. A mask of the wrong length now gives no loops instead of failing an assertion.
The output is identical: `the_walk_equals_the_stored_direction_walk` compares it with a test-only
copy of the previous walk on all 65,536 masks of 4 × 4 pixels and on 400 seeded 23 × 17 masks.
Time (best of 7, release build, 4 cores): the walk alone on 2000 × 2000 masks 19.1 ms vs 26.6 ms
(a smooth pattern), 81.3 vs 93.2 ms (12% random pixels), 223.1 vs 238.1 ms (50% random pixels);
whole traces of 1000 × 1000 images 73.1 vs 77.1 ms (Black and White), 249.6 vs 251.7 ms (Color, 6
colours) and 214.1 vs 218.6 ms (Color, 6 colours, Noise 1), with the same segment counts.
## Noise threshold stability (local development, 2026-10-14)

Noise must merge whole 4-connected regions, including pixels absorbed by an earlier merge.
A count of regions alone cannot establish this: one old region may split while others disappear.
The repeated-pass implementation at `a0671777` fails a partition regression at case 0, Noise 3.

The replacement discovers the initial regions, processes them in deterministic increasing-area
order, and keeps their membership current. All adjacent regions of the chosen colour join the
merge, with ties resolved by label and region id. Increasing Noise continues the same merge
sequence: every lower-threshold region remains wholly inside one higher-threshold region.
Regions exactly at the threshold stay; an opaque island surrounded by transparency stays;
transparent pixels are never recoloured. A merged region is tested at its current area, so a
1-pixel speck joining a 2-pixel region meets Noise 3 and is retained as one 3-pixel region.

Tests generate 64 multicolour 7 × 7 grids with transparent barriers and check every threshold
from 0 through 24 with an independent component traversal. They assert nested partitions,
non-increasing region counts and unchanged transparency. Separate cases check the strict
threshold, isolated opaque islands and invalid or overflowing grid dimensions. The existing
no-small-region and traced-path regressions remain active.

This uses region and heap bookkeeping instead of repeated full-image passes, so memory and
photo quality must also be checked. Local photo/scan validation uses fixed hashed inputs and
identical settings on unchanged current source and the candidate, including separate portrait
crops and white-composited PNG comparisons. These local fixtures are not repository assets.
The parity score is unchanged.

## Pieces returning to a junction
An outline piece can return to its starting junction while being fitted as an open chain. At loose
Corners/Paths settings, a nonzero positioned boundary could collapse to one cubic and lose a small
corner-touching mark. Fixed-end pieces with identical endpoints and fewer than two fitted segments
now retain their positioned boundary when its area is finite and nonzero. Fitting the shared piece
this way also gives an abutting fill exactly the reversed boundary, including within a compound path.
Six source-generated regressions check 1- and 2-pixel regions through four rotations, both windings
and large translations; fixed junctions; zero positioned area; free ends; Noise controls; and shared
edges through four rotations and black/white inversion. Noise 2 still removes the 1-pixel mark.
The white-hole Noise check is a control, not a reproduced returning-hole case. Defaults are unchanged;
these tests establish survival and edge consistency, not a new artistic-quality or parity score.
This is separate from the closed-contour recovery in #2012 and straightening guard in #2025.
## Thin closed outlines with Snap Curves To Lines

A four-pixel stair-step mark can fit as two straight cubics along the same diagonal, leaving a closed path with zero area. The outline-distance checks accept this because both sides are close to the chord. Closed fitting now checks analytic curve area after straightening and upright levelling; if a nondegenerate source loop has lost its interior, it retains the positioned boundary as an unsimplified polygon. The area integral is translated to a local origin to reduce cancellation. This numerical degeneracy check is not a new Noise or minimum-feature-area setting.

`fit::snap_area_tests` covers the mark at two positions and five Corners settings, a white stair-step hole, reversed/rotated closed boundaries with exactly matching opposite curves, and a narrow filled rectangle. Noise still removes the four-pixel mark or hole when set to 5. It does not change open-piece fitting, global Snap tolerances or defaults, and it does not address every possible zero-area compound outline. The preset quality table below is the previous measurement, not a rerun of that full report.

## Tiny closed contours
A coarse fit can reduce a small closed outline to one cubic, which the editable-path builder cannot
represent as a closed subpath with two anchors. Previously that dropped the component or hole after
it survived Noise. The fitter now retains the positioned contour without further simplification when
that minimum is not met. Corners and Noise defaults are unchanged.
Three source-generated regressions cover a one-pixel island, a one-pixel hole with opposite winding,
and a stepped seven-pixel region across Corners values. They also check that Noise still removes the
island and fills the hole when its threshold requests that. These are survival and winding checks,
not a new quality-harness baseline or a general claim about small-feature fidelity.
## How it is measured

`crates/trace/tests/quality` draws its own inputs from vector art (no image files), traces each with every
built-in preset, and measures the result:

```text
TRACE_QUALITY_REPORT=report.md cargo test --release -p vectorcraft-trace --test quality -- --ignored
```

**Inputs** (drawn anti-aliased at 128, 256 and 512 px, and at 256 px also blurred, with compression-like
noise, as a scan of ink on cream paper with uneven light and grain, and aliased: one hard sample per pixel, no
anti-aliasing):

| kind | inputs |
|:--|:--|
| 2026-10-15 | Fixed-end pieces returning to their junction retain their positioned nonzero boundary when fitted to fewer than two cubics; source-generated tests cover survival, winding, shared-edge equality, translations and Noise controls. Defaults and parity scores unchanged. |
| curved shapes | a circle; two ellipses; a rounded badge with a round hole; a letter D with its counter |
| polygons | a triangle, a turned square and a hexagon; a five-pointed star; a letter A with its counter |
| thin lines | five lines 0.5, 1, 2, 3 and 5 units wide (of a 256-unit square), a few degrees off the pixel grid |
| askew shapes | a rectangle, a plus sign and two lines, each 1 to 2 degrees off level and upright |
| plain shapes | a circle, a rectangle, a square, a rounded rectangle, an ellipse and a hexagon, each alone on white (measured from the Shapes change on, not at the baseline) |
| linear gradients | a two-stop gradient over the whole image at 25 degrees, and a three-stop gradient at 110 degrees in a disc on white: their lines and stops are known (measured from the Gradients change on, not at the baseline) |
| flat colour art | a disc of two halves in different colours that touch; a badge of four colours on white |
| gradient, photo | a smooth two-way gradient with a glow; a photo-like image (soft-edged shaded objects, texture, sensor noise) |

The letters are drawn by hand from lines and curves: the repository has no font to set them in.

**Columns**

| column | meaning |
|:--|:--|
| ΔE | the trace drawn again against the source: mean colour difference in Oklab × 100 (black to white is 100) |
| edge ΔE | the same, weighted by how strong an edge the source has at each pixel |
| off mean, off max | distance (px) from the traced outlines to the art's true outlines: wobble and staircase |
| missed max | largest distance (px) from a true outline to the nearest traced one: what was lost |
| paths, anchors | counts, the background's frame aside |
| × art | anchors per anchor point of the art itself (1.00 is as economical as the original) |
| lines % | share of the outline length made of straight segments (100 is right for polygons, 0 for a circle) |
| upright % | share of the outline length that is exactly level or upright |
| corners % | share of the art's sharp corners with a traced corner within 1.5 px |
| kinks | anchors where the outline turns by 10° or more away from any true corner |
| odd colours, odd px | paths (and their area) in a colour the art doesn't have: slivers along anti-aliased edges |
| gap px, overlap px | area no path covers, and area covered twice (abutting traces with a white background only) |
| specks | paths under 6 px of area |
| centre off, width off, anchors a stroke | stroked centre lines (Create: Strokes) against the thin lines they trace: mean distance (px) from the true line's centre line, mean error (px) of the stroke width, anchors per stroked path |
| shapes % | with Shapes on, the share of the plain shapes that came out as that shape (a shape, not a path, whose outline lies within 0.6 px of the true one) |
| false shapes | with Shapes on, paths made a shape whose outline strays more than 0.6 px from the art (a shape the art does not have, or one put where it is not) |
| grouped % | with Auto Grouping on, the share of the paths that were put inside another (the picture is painted as the app groups it for every other measure of that row: each path together with those inside it, in the order they were traced in; a ΔE equal to the row without the option says grouping changed nothing that shows) |
| gradient paths | paths filled with a linear gradient (on inputs that are flat colour, every one would be a gradient that is not there) |
| found %, angle off, ramp off | on the linear-gradient inputs: the share of them in which a gradient was found; and for those found, the largest angle (degrees) between the true gradient line and the traced one, and the largest difference (levels of a channel, 0 to 255) between the true colours and the traced gradient's at 17 points along the true line |
| outside px | how far the trace reaches outside the image: a curve that strays |

Rows named "…, enhanced" are that preset's enhanced version (Auto Grouping on; Shapes on in all but High
Fidelity Photo) on the same inputs as the row of the preset itself. Flat Logo has no enhanced version.

Black-and-white presets trace black: on the scanned variant their ΔE includes the paper's and ink's own
colours. Silhouettes has its threshold at 200, not 128: its outline lies where the image
crosses that tone, about 0.3 px outside the true edge on anti-aliased art and 0.8 px on blurred art; with
the threshold at 128 the same preset is 0.110 px off on its inputs (1.00 px at most), as the other
black-and-white presets are. `–` means the measure does not apply; `all` that nothing was traced.

## Baseline and now

Each preset on the inputs it is meant for, all sizes and variants.

### Baseline (before any change, main at `baf9693f`)

| preset | inputs | ΔE | edge ΔE | off mean | off max | missed max | paths | anchors | × art | lines % | upright % | corners % | kinks | odd colours | odd px | gap px | overlap px | specks | outside px | centre off | width off | anchors a stroke |
|:--|:--|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| Default | circle, ellipses, badge with a hole, letter D, polygons, star, letter A | 2.79 | 17.15 | 0.313 | 1.51 | 2.52 | 1.9 | 14 | 1.45 | 34 | 5 | 76 | 1.4 | – | – | 63.5 | 67.0 | 0.0 | 0.0 | – | – | – |
| High Fidelity Photo | gradient, photo-like | 2.30 | 2.63 | – | – | – | 3652.6 | 46866 | – | 84 | 29 | – | – | – | – | 0.0 | – | 1169.4 | 0.3 | – | – | – |
| Low Fidelity Photo | gradient, photo-like | 2.61 | 2.94 | – | – | – | 128.3 | 3867 | – | 42 | 22 | – | – | – | – | 0.0 | – | 0.0 | 0.7 | – | – | – |
| 3 Colors | two touching halves | 4.93 | 11.38 | 0.482 | 2.35 | 89.09 | 2.1 | 14 | 2.31 | 12 | 8 | 29 | 2.1 | 1.6 | 15418 | 118.0 | 87.3 | 0.0 | 0.0 | – | – | – |
| 6 Colors | two touching halves, four-colour badge | 0.78 | 8.82 | 4.383 | 83.10 | 30.11 | 21.2 | 270 | 30.18 | 51 | 40 | 98 | 213.8 | 2.4 | 272 | 198.8 | 194.6 | 0.0 | 0.3 | – | – | – |
| 16 Colors | two touching halves, four-colour badge | 0.74 | 9.02 | 5.115 | 86.21 | 4.32 | 69.9 | 923 | 102.80 | 58 | 47 | 100 | 765.9 | 5.9 | 670 | 293.7 | 329.9 | 0.0 | 0.5 | – | – | – |
| Shades of Gray | gradient, photo-like | 9.29 | 9.16 | – | – | – | 90.8 | 1921 | – | 42 | 27 | – | – | – | – | 0.0 | – | 0.2 | 0.5 | – | – | – |
| Black and White Logo | circle, ellipses, badge with a hole, letter D, polygons, star, letter A | 2.76 | 15.96 | 0.270 | 1.76 | 2.53 | 1.4 | 31 | 3.78 | 90 | 17 | 81 | 15.3 | – | – | – | – | 0.0 | 0.0 | – | – | – |
| Sketched Art | circle, ellipses, badge with a hole, letter D, lines | 2.85 | 17.04 | 0.302 | 1.63 | 105.63 | 1.8 | 14 | 1.34 | 28 | 8 | 18 | 1.4 | – | – | – | – | 0.0 | 0.0 | – | – | – |
| Silhouettes | circle, ellipses, badge with a hole, letter D | 3.04 | 21.41 | 0.573 | 2.88 | 2.89 | 1.2 | 10 | 1.16 | 9 | 7 | 86 | 1.4 | – | – | – | – | 0.0 | 0.0 | – | – | – |
| Line Art | lines | 2.77 | 12.51 | 0.158 | 1.13 | 105.76 | 5.4 | 25 | 1.27 | 90 | 6 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – |
| Technical Drawing | polygons, star, letter A, lines, askew shapes and lines | 2.73 | 13.67 | 0.210 | 1.43 | 105.76 | 4.7 | 31 | 2.01 | 100 | 9 | 85 | 5.9 | – | – | – | – | 0.1 | 0.0 | – | – | – |
| Flat Logo | two touching halves, four-colour badge | 0.43 | 3.53 | 3.106 | 88.50 | 0.78 | 69.9 | 352 | 38.92 | 38 | 10 | 80 | 237.0 | 0.0 | 0 | 13.8 | – | 59.1 | 0.2 | – | – | – |

### Now (with gradients grown over noise and soft rims)

| preset | inputs | ΔE | edge ΔE | off mean | off max | missed max | paths | anchors | × art | lines % | upright % | corners % | kinks | odd colours | odd px | gap px | overlap px | specks | outside px | centre off | width off | anchors a stroke | shapes % | false shapes | grouped % | gradient paths | found % | angle off | ramp off |
|:--|:--|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| Default | circle, ellipses, badge with a hole, letter D, polygons, star, letter A | 2.60 | 10.05 | 0.091 | 1.02 | 2.18 | 1.9 | 17 | 1.73 | 30 | 6 | 76 | 0.4 | – | – | 0.3 | 0.5 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Auto Color | two touching halves, four-colour badge | 0.55 | 4.99 | 0.141 | 1.33 | 1.21 | 4.0 | 51 | 3.19 | 39 | 16 | 91 | 6.0 | 0.0 | 0 | 1.5 | 1.2 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| High Fidelity Photo | gradient, photo-like | 1.75 | 1.98 | – | – | – | 120.7 | 9929 | – | 38 | 36 | – | – | – | – | 0.0 | – | 10.1 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| Low Fidelity Photo | gradient, photo-like | 2.45 | 2.69 | – | – | – | 37.1 | 2134 | – | 43 | 37 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| 3 Colors | two touching halves | 0.42 | 3.98 | 0.104 | 0.91 | 0.91 | 2.0 | 11 | 1.81 | 39 | 6 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 6 Colors | two touching halves, four-colour badge | 0.55 | 4.99 | 0.140 | 1.34 | 1.19 | 4.0 | 49 | 3.07 | 39 | 16 | 91 | 6.1 | 0.0 | 0 | 0.3 | 0.3 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 16 Colors | two touching halves, four-colour badge | 0.55 | 4.91 | 0.142 | 1.31 | 1.19 | 4.2 | 63 | 3.87 | 40 | 16 | 91 | 7.7 | 0.0 | 0 | 0.5 | 0.6 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Shades of Gray | gradient, photo-like | 9.26 | 9.06 | – | – | – | 16.6 | 700 | – | 51 | 40 | – | – | – | – | 0.0 | – | 0.5 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| Black and White Logo | circle, ellipses, badge with a hole, letter D, polygons, star, letter A | 2.59 | 9.46 | 0.064 | 1.02 | 2.17 | 1.4 | 27 | 2.68 | 42 | 11 | 76 | 0.3 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Sketched Art | circle, ellipses, badge with a hole, letter D, lines | 2.71 | 12.02 | 0.185 | 1.00 | 105.61 | 1.8 | 16 | 1.42 | 19 | 8 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Silhouettes | circle, ellipses, badge with a hole, letter D | 2.98 | 19.30 | 0.524 | 2.19 | 2.19 | 1.2 | 14 | 1.64 | 12 | 9 | 93 | 0.4 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Line Art | lines | 2.62 | 9.49 | 0.056 | 0.99 | 105.72 | 5.4 | 43 | 2.14 | 40 | 0 | – | – | – | – | – | – | 0.1 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Technical Drawing | polygons, star, letter A, lines, askew shapes and lines | 2.65 | 10.88 | 0.126 | 1.46 | 105.64 | 4.7 | 38 | 2.55 | 81 | 19 | 78 | 0.8 | – | – | – | – | 0.5 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Outline | lines | 2.61 | 9.39 | 0.050 | 0.88 | 105.74 | 5.0 | 10 | 0.52 | 99 | 4 | – | – | – | – | – | – | 0.0 | 0.0 | 0.039 | 0.02 | 2.0 | – | 0.00 | – | 0.0 | – | – | – |
| Flat Logo | two touching halves, four-colour badge | 0.48 | 3.47 | 0.090 | 0.82 | 0.78 | 3.9 | 49 | 3.85 | 40 | 10 | 80 | 2.6 | 0.0 | 0 | 10.6 | – | 0.9 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Default with Shapes | plain shapes | 3.16 | 9.22 | 0.085 | 0.90 | 0.90 | 6.0 | 30 | 1.01 | 65 | 40 | – | – | – | – | 0.5 | 0.0 | 0.0 | 0.0 | – | – | – | 98 | 0.00 | – | 0.0 | – | – | – |
| 6 Colors with Shapes | plain shapes | 0.90 | 6.64 | 0.077 | 0.60 | 0.60 | 6.0 | 30 | 1.00 | 67 | 39 | – | – | 0.0 | 0 | 2.1 | 1.3 | 0.0 | 0.0 | – | – | – | 100 | 0.00 | – | 0.0 | – | – | – |
| 6 Colors with Auto Grouping | two touching halves, four-colour badge | 0.55 | 4.99 | 0.140 | 1.34 | 1.19 | 4.0 | 49 | 3.07 | 39 | 16 | 91 | 6.1 | 0.0 | 0 | 0.3 | 0.3 | 0.0 | 0.0 | – | – | – | – | 0.00 | 27 | 0.0 | – | – | – |
| Shades of Gray with Auto Grouping | gradient, photo-like | 9.26 | 9.06 | – | – | – | 16.6 | 700 | – | 51 | 40 | – | – | – | – | 0.0 | – | 0.5 | 0.0 | – | – | – | – | – | 47 | 0.0 | – | – | – |
| High Fidelity Photo with Auto Grouping | gradient, photo-like | 1.75 | 1.98 | – | – | – | 120.7 | 9929 | – | 38 | 36 | – | – | – | – | 0.0 | – | 10.1 | 0.0 | – | – | – | – | – | 69 | 0.0 | – | – | – |
| 6 Colors, no Gradients | two-stop gradient, three-stop gradient in a disc | 2.12 | 3.04 | – | – | – | 6.4 | 469 | – | 21 | 14 | – | – | – | – | 3.9 | 12.7 | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| 6 Colors with Gradients | two-stop gradient, three-stop gradient in a disc | 0.62 | 1.34 | – | – | – | 1.2 | 60 | – | 0 | 0 | – | – | – | – | 1.6 | 1.2 | 0.0 | 0.0 | – | – | – | – | – | – | 0.9 | 86 | 1.5 | 21 |
| High Fidelity Photo, no Gradients | two-stop gradient, three-stop gradient in a disc | 0.79 | 2.98 | – | – | – | 38.1 | 3827 | – | 31 | 30 | – | – | – | – | 0.0 | – | 1.1 | 0.0 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| High Fidelity Photo with Gradients | two-stop gradient, three-stop gradient in a disc | 0.46 | 1.29 | – | – | – | 8.2 | 545 | – | 1 | 0 | – | – | – | – | 0.0 | – | 0.8 | 0.0 | – | – | – | – | – | – | 0.9 | 86 | 1.5 | 21 |
| Default, enhanced | circle, ellipses, badge with a hole, letter D, polygons, star, letter A | 2.60 | 9.88 | 0.083 | 1.00 | 2.18 | 1.9 | 16 | 1.59 | 36 | 6 | 83 | 0.4 | – | – | 0.3 | 0.5 | 0.0 | 0.0 | – | – | – | – | 0.02 | 14 | 0.0 | – | – | – |
| Auto Color, enhanced | two touching halves, four-colour badge | 0.55 | 5.00 | 0.139 | 1.33 | 1.00 | 4.0 | 48 | 3.03 | 41 | 16 | 100 | 6.0 | 0.0 | 0 | 1.3 | 1.1 | 0.0 | 0.0 | – | – | – | – | 0.00 | 27 | 0.0 | – | – | – |
| High Fidelity Photo, enhanced | gradient, photo-like | 1.75 | 1.98 | – | – | – | 120.7 | 9929 | – | 38 | 36 | – | – | – | – | 0.0 | – | 10.1 | 0.0 | – | – | – | – | – | 69 | 0.0 | – | – | – |
| Low Fidelity Photo, enhanced | gradient, photo-like | 2.45 | 2.69 | – | – | – | 37.1 | 2134 | – | 43 | 37 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 63 | 0.0 | – | – | – |
| 3 Colors, enhanced | two touching halves | 0.42 | 3.98 | 0.104 | 0.91 | 0.91 | 2.0 | 11 | 1.81 | 39 | 6 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| 6 Colors, enhanced | two touching halves, four-colour badge | 0.55 | 4.99 | 0.139 | 1.34 | 1.00 | 4.0 | 45 | 2.91 | 41 | 16 | 100 | 6.1 | 0.0 | 0 | 0.2 | 0.3 | 0.0 | 0.0 | – | – | – | – | 0.00 | 27 | 0.0 | – | – | – |
| 16 Colors, enhanced | two touching halves, four-colour badge | 0.55 | 4.91 | 0.141 | 1.31 | 1.00 | 4.2 | 59 | 3.68 | 42 | 17 | 100 | 7.7 | 0.0 | 0 | 0.4 | 0.5 | 0.0 | 0.0 | – | – | – | – | 0.00 | 28 | 0.0 | – | – | – |
| Shades of Gray, enhanced | gradient, photo-like | 9.26 | 9.06 | – | – | – | 16.6 | 700 | – | 51 | 40 | – | – | – | – | 0.0 | – | 0.5 | 0.0 | – | – | – | – | 0.00 | 47 | 0.0 | – | – | – |
| Black and White Logo, enhanced | circle, ellipses, badge with a hole, letter D, polygons, star, letter A | 2.59 | 9.49 | 0.065 | 1.00 | 2.17 | 1.4 | 21 | 2.14 | 46 | 11 | 83 | 0.3 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.02 | 0 | 0.0 | – | – | – |
| Sketched Art, enhanced | circle, ellipses, badge with a hole, letter D, lines | 2.71 | 11.80 | 0.178 | 1.00 | 105.61 | 1.8 | 15 | 1.32 | 19 | 8 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.03 | 0 | 0.0 | – | – | – |
| Silhouettes, enhanced | circle, ellipses, badge with a hole, letter D | 2.97 | 18.98 | 0.515 | 2.19 | 2.19 | 1.2 | 13 | 1.57 | 12 | 10 | 93 | 0.4 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.11 | 0 | 0.0 | – | – | – |
| Line Art, enhanced | lines | 2.62 | 9.50 | 0.055 | 0.99 | 105.72 | 5.4 | 42 | 2.11 | 42 | 0 | – | – | – | – | – | – | 0.1 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Technical Drawing, enhanced | polygons, star, letter A, lines, askew shapes and lines | 2.65 | 10.93 | 0.128 | 1.46 | 105.64 | 4.7 | 34 | 2.26 | 86 | 19 | 86 | 0.8 | – | – | – | – | 0.5 | 0.0 | – | – | – | – | 0.09 | 0 | 0.0 | – | – | – |
| Outline, enhanced | lines | 2.61 | 9.39 | 0.050 | 0.88 | 105.74 | 5.0 | 10 | 0.52 | 99 | 4 | – | – | – | – | – | – | 0.0 | 0.0 | 0.039 | 0.02 | 2.0 | – | 0.00 | 0 | 0.0 | – | – | – |

The sections below are the numbers now (with gradients grown over noise and soft rims).

## 1. Each preset on the inputs it is meant for

All sizes and variants (clean, aliased, blurred, noisy, scanned) of those inputs.

| preset | inputs | ΔE | edge ΔE | off mean | off max | missed max | paths | anchors | × art | lines % | upright % | corners % | kinks | odd colours | odd px | gap px | overlap px | specks | outside px | centre off | width off | anchors a stroke | shapes % | false shapes | grouped % | gradient paths | found % | angle off | ramp off |
|:--|:--|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| Default | circle, ellipses, badge with a hole, letter D, polygons, star, letter A | 2.60 | 10.05 | 0.091 | 1.02 | 2.18 | 1.9 | 17 | 1.73 | 30 | 6 | 76 | 0.4 | – | – | 0.3 | 0.5 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Auto Color | two touching halves, four-colour badge | 0.55 | 4.99 | 0.141 | 1.33 | 1.21 | 4.0 | 51 | 3.19 | 39 | 16 | 91 | 6.0 | 0.0 | 0 | 1.5 | 1.2 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| High Fidelity Photo | gradient, photo-like | 1.75 | 1.98 | – | – | – | 120.7 | 9929 | – | 38 | 36 | – | – | – | – | 0.0 | – | 10.1 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| Low Fidelity Photo | gradient, photo-like | 2.45 | 2.69 | – | – | – | 37.1 | 2134 | – | 43 | 37 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| 3 Colors | two touching halves | 0.42 | 3.98 | 0.104 | 0.91 | 0.91 | 2.0 | 11 | 1.81 | 39 | 6 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 6 Colors | two touching halves, four-colour badge | 0.55 | 4.99 | 0.140 | 1.34 | 1.19 | 4.0 | 49 | 3.07 | 39 | 16 | 91 | 6.1 | 0.0 | 0 | 0.3 | 0.3 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 16 Colors | two touching halves, four-colour badge | 0.55 | 4.91 | 0.142 | 1.31 | 1.19 | 4.2 | 63 | 3.87 | 40 | 16 | 91 | 7.7 | 0.0 | 0 | 0.5 | 0.6 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Shades of Gray | gradient, photo-like | 9.26 | 9.06 | – | – | – | 16.6 | 700 | – | 51 | 40 | – | – | – | – | 0.0 | – | 0.5 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| Black and White Logo | circle, ellipses, badge with a hole, letter D, polygons, star, letter A | 2.59 | 9.46 | 0.064 | 1.02 | 2.17 | 1.4 | 27 | 2.68 | 42 | 11 | 76 | 0.3 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Sketched Art | circle, ellipses, badge with a hole, letter D, lines | 2.71 | 12.02 | 0.185 | 1.00 | 105.61 | 1.8 | 16 | 1.42 | 19 | 8 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Silhouettes | circle, ellipses, badge with a hole, letter D | 2.98 | 19.30 | 0.524 | 2.19 | 2.19 | 1.2 | 14 | 1.64 | 12 | 9 | 93 | 0.4 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Line Art | lines | 2.62 | 9.49 | 0.056 | 0.99 | 105.72 | 5.4 | 43 | 2.14 | 40 | 0 | – | – | – | – | – | – | 0.1 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Technical Drawing | polygons, star, letter A, lines, askew shapes and lines | 2.65 | 10.88 | 0.126 | 1.46 | 105.64 | 4.7 | 38 | 2.55 | 81 | 19 | 78 | 0.8 | – | – | – | – | 0.5 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Outline | lines | 2.61 | 9.39 | 0.050 | 0.88 | 105.74 | 5.0 | 10 | 0.52 | 99 | 4 | – | – | – | – | – | – | 0.0 | 0.0 | 0.039 | 0.02 | 2.0 | – | 0.00 | – | 0.0 | – | – | – |
| Flat Logo | two touching halves, four-colour badge | 0.48 | 3.47 | 0.090 | 0.82 | 0.78 | 3.9 | 49 | 3.85 | 40 | 10 | 80 | 2.6 | 0.0 | 0 | 10.6 | – | 0.9 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Default with Shapes | plain shapes | 3.16 | 9.22 | 0.085 | 0.90 | 0.90 | 6.0 | 30 | 1.01 | 65 | 40 | – | – | – | – | 0.5 | 0.0 | 0.0 | 0.0 | – | – | – | 98 | 0.00 | – | 0.0 | – | – | – |
| 6 Colors with Shapes | plain shapes | 0.90 | 6.64 | 0.077 | 0.60 | 0.60 | 6.0 | 30 | 1.00 | 67 | 39 | – | – | 0.0 | 0 | 2.1 | 1.3 | 0.0 | 0.0 | – | – | – | 100 | 0.00 | – | 0.0 | – | – | – |
| 6 Colors with Auto Grouping | two touching halves, four-colour badge | 0.55 | 4.99 | 0.140 | 1.34 | 1.19 | 4.0 | 49 | 3.07 | 39 | 16 | 91 | 6.1 | 0.0 | 0 | 0.3 | 0.3 | 0.0 | 0.0 | – | – | – | – | 0.00 | 27 | 0.0 | – | – | – |
| Shades of Gray with Auto Grouping | gradient, photo-like | 9.26 | 9.06 | – | – | – | 16.6 | 700 | – | 51 | 40 | – | – | – | – | 0.0 | – | 0.5 | 0.0 | – | – | – | – | – | 47 | 0.0 | – | – | – |
| High Fidelity Photo with Auto Grouping | gradient, photo-like | 1.75 | 1.98 | – | – | – | 120.7 | 9929 | – | 38 | 36 | – | – | – | – | 0.0 | – | 10.1 | 0.0 | – | – | – | – | – | 69 | 0.0 | – | – | – |
| 6 Colors, no Gradients | two-stop gradient, three-stop gradient in a disc | 2.12 | 3.04 | – | – | – | 6.4 | 469 | – | 21 | 14 | – | – | – | – | 3.9 | 12.7 | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| 6 Colors with Gradients | two-stop gradient, three-stop gradient in a disc | 0.62 | 1.34 | – | – | – | 1.2 | 60 | – | 0 | 0 | – | – | – | – | 1.6 | 1.2 | 0.0 | 0.0 | – | – | – | – | – | – | 0.9 | 86 | 1.5 | 21 |
| High Fidelity Photo, no Gradients | two-stop gradient, three-stop gradient in a disc | 0.79 | 2.98 | – | – | – | 38.1 | 3827 | – | 31 | 30 | – | – | – | – | 0.0 | – | 1.1 | 0.0 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| High Fidelity Photo with Gradients | two-stop gradient, three-stop gradient in a disc | 0.46 | 1.29 | – | – | – | 8.2 | 545 | – | 1 | 0 | – | – | – | – | 0.0 | – | 0.8 | 0.0 | – | – | – | – | – | – | 0.9 | 86 | 1.5 | 21 |
| Default, enhanced | circle, ellipses, badge with a hole, letter D, polygons, star, letter A | 2.60 | 9.88 | 0.083 | 1.00 | 2.18 | 1.9 | 16 | 1.59 | 36 | 6 | 83 | 0.4 | – | – | 0.3 | 0.5 | 0.0 | 0.0 | – | – | – | – | 0.02 | 14 | 0.0 | – | – | – |
| Auto Color, enhanced | two touching halves, four-colour badge | 0.55 | 5.00 | 0.139 | 1.33 | 1.00 | 4.0 | 48 | 3.03 | 41 | 16 | 100 | 6.0 | 0.0 | 0 | 1.3 | 1.1 | 0.0 | 0.0 | – | – | – | – | 0.00 | 27 | 0.0 | – | – | – |
| High Fidelity Photo, enhanced | gradient, photo-like | 1.75 | 1.98 | – | – | – | 120.7 | 9929 | – | 38 | 36 | – | – | – | – | 0.0 | – | 10.1 | 0.0 | – | – | – | – | – | 69 | 0.0 | – | – | – |
| Low Fidelity Photo, enhanced | gradient, photo-like | 2.45 | 2.69 | – | – | – | 37.1 | 2134 | – | 43 | 37 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 63 | 0.0 | – | – | – |
| 3 Colors, enhanced | two touching halves | 0.42 | 3.98 | 0.104 | 0.91 | 0.91 | 2.0 | 11 | 1.81 | 39 | 6 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| 6 Colors, enhanced | two touching halves, four-colour badge | 0.55 | 4.99 | 0.139 | 1.34 | 1.00 | 4.0 | 45 | 2.91 | 41 | 16 | 100 | 6.1 | 0.0 | 0 | 0.2 | 0.3 | 0.0 | 0.0 | – | – | – | – | 0.00 | 27 | 0.0 | – | – | – |
| 16 Colors, enhanced | two touching halves, four-colour badge | 0.55 | 4.91 | 0.141 | 1.31 | 1.00 | 4.2 | 59 | 3.68 | 42 | 17 | 100 | 7.7 | 0.0 | 0 | 0.4 | 0.5 | 0.0 | 0.0 | – | – | – | – | 0.00 | 28 | 0.0 | – | – | – |
| Shades of Gray, enhanced | gradient, photo-like | 9.26 | 9.06 | – | – | – | 16.6 | 700 | – | 51 | 40 | – | – | – | – | 0.0 | – | 0.5 | 0.0 | – | – | – | – | 0.00 | 47 | 0.0 | – | – | – |
| Black and White Logo, enhanced | circle, ellipses, badge with a hole, letter D, polygons, star, letter A | 2.59 | 9.49 | 0.065 | 1.00 | 2.17 | 1.4 | 21 | 2.14 | 46 | 11 | 83 | 0.3 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.02 | 0 | 0.0 | – | – | – |
| Sketched Art, enhanced | circle, ellipses, badge with a hole, letter D, lines | 2.71 | 11.80 | 0.178 | 1.00 | 105.61 | 1.8 | 15 | 1.32 | 19 | 8 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.03 | 0 | 0.0 | – | – | – |
| Silhouettes, enhanced | circle, ellipses, badge with a hole, letter D | 2.97 | 18.98 | 0.515 | 2.19 | 2.19 | 1.2 | 13 | 1.57 | 12 | 10 | 93 | 0.4 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.11 | 0 | 0.0 | – | – | – |
| Line Art, enhanced | lines | 2.62 | 9.50 | 0.055 | 0.99 | 105.72 | 5.4 | 42 | 2.11 | 42 | 0 | – | – | – | – | – | – | 0.1 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Technical Drawing, enhanced | polygons, star, letter A, lines, askew shapes and lines | 2.65 | 10.93 | 0.128 | 1.46 | 105.64 | 4.7 | 34 | 2.26 | 86 | 19 | 86 | 0.8 | – | – | – | – | 0.5 | 0.0 | – | – | – | – | 0.09 | 0 | 0.0 | – | – | – |
| Outline, enhanced | lines | 2.61 | 9.39 | 0.050 | 0.88 | 105.74 | 5.0 | 10 | 0.52 | 99 | 4 | – | – | – | – | – | – | 0.0 | 0.0 | 0.039 | 0.02 | 2.0 | – | 0.00 | 0 | 0.0 | – | – | – |

## 2. Each preset on each kind of input (clean, three sizes)

| preset | inputs | ΔE | edge ΔE | off mean | off max | missed max | paths | anchors | × art | lines % | upright % | corners % | kinks | odd colours | odd px | gap px | overlap px | specks | outside px | centre off | width off | anchors a stroke | shapes % | false shapes | grouped % | gradient paths | found % | angle off | ramp off |
|:--|:--|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| Default | curved shapes | 0.09 | 3.40 | 0.060 | 0.38 | 0.38 | 1.8 | 14 | 1.57 | 14 | 14 | 100 | 0.0 | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Default | polygons | 0.06 | 2.37 | 0.041 | 0.47 | 1.26 | 2.0 | 14 | 1.26 | 88 | 5 | 95 | 0.0 | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Default | thin lines | 0.54 | 4.46 | 0.049 | 0.47 | 105.72 | 3.3 | 25 | 1.27 | 40 | 0 | – | – | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Default | askew shapes | 0.16 | 3.29 | 0.044 | 0.34 | 36.06 | 3.7 | 23 | 0.94 | 100 | 3 | – | – | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Default | plain shapes | 0.13 | 2.63 | 0.055 | 0.35 | 0.35 | 6.0 | 40 | 1.34 | 53 | 48 | – | – | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | 0 | 0.00 | – | 0.0 | – | – | – |
| Default | flat colour art | 19.91 | 17.97 | 0.172 | 0.51 | 179.34 | 1.5 | 16 | 1.17 | 14 | 14 | 0 | 0.0 | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Default | gradient, photo | 39.03 | 38.64 | – | – | – | 3.2 | 595 | – | 55 | 41 | – | – | – | – | 18.8 | 35.5 | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| Default | linear gradients | 25.64 | 29.86 | – | – | – | 2.0 | 43 | – | 33 | 32 | – | – | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| Auto Color | curved shapes | 0.06 | 2.43 | 0.043 | 0.27 | 0.28 | 1.8 | 16 | 1.77 | 16 | 16 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Auto Color | polygons | 0.06 | 2.42 | 0.041 | 0.42 | 1.39 | 2.0 | 16 | 1.43 | 87 | 7 | 93 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Auto Color | thin lines | 0.70 | 5.57 | 0.066 | 0.50 | 105.72 | 5.0 | 38 | 1.88 | 43 | 3 | – | – | 4.0 | 3225 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Auto Color | askew shapes | 0.64 | 3.76 | 0.047 | 0.36 | 9.49 | 4.0 | 24 | 1.00 | 100 | 3 | – | – | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Auto Color | plain shapes | 0.10 | 2.07 | 0.043 | 0.30 | 0.26 | 6.0 | 42 | 1.41 | 59 | 49 | – | – | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | 0 | 0.00 | – | 0.0 | – | – | – |
| Auto Color | flat colour art | 0.13 | 2.72 | 0.057 | 0.31 | 0.30 | 3.0 | 28 | 2.00 | 43 | 19 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Auto Color | gradient, photo | 2.07 | 2.39 | – | – | – | 159.3 | 4441 | – | 21 | 8 | – | – | – | – | 37.6 | 46.7 | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| Auto Color | linear gradients | 1.02 | 1.74 | – | – | – | 15.7 | 1272 | – | 12 | 8 | – | – | – | – | 9.1 | 7.4 | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| High Fidelity Photo | curved shapes | 0.04 | 1.63 | 0.026 | 0.15 | 0.15 | 1.2 | 19 | 2.11 | 21 | 21 | 100 | 0.0 | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| High Fidelity Photo | polygons | 0.06 | 2.33 | 0.039 | 0.50 | 1.32 | 1.7 | 17 | 1.47 | 88 | 7 | 93 | 0.0 | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| High Fidelity Photo | thin lines | 0.35 | 6.05 | 0.085 | 0.90 | 4.35 | 40.3 | 348 | 17.42 | 35 | 9 | – | – | 27.3 | 4505 | 0.0 | – | 19.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| High Fidelity Photo | askew shapes | 0.17 | 3.97 | 0.100 | 0.94 | 1.73 | 8.7 | 157 | 6.54 | 71 | 5 | – | – | 4.3 | 3998 | 0.0 | – | 1.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| High Fidelity Photo | plain shapes | 0.08 | 1.66 | 0.035 | 0.25 | 0.28 | 6.0 | 50 | 1.68 | 62 | 51 | – | – | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | 0 | 0.00 | – | 0.0 | – | – | – |
| High Fidelity Photo | flat colour art | 0.14 | 4.16 | 0.029 | 0.20 | 0.20 | 3.5 | 73 | 4.59 | 35 | 34 | 100 | 0.0 | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| High Fidelity Photo | gradient, photo | 1.80 | 2.05 | – | – | – | 134.7 | 11212 | – | 39 | 37 | – | – | – | – | 0.0 | – | 7.3 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| High Fidelity Photo | linear gradients | 0.64 | 3.29 | – | – | – | 34.5 | 4318 | – | 32 | 31 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| Low Fidelity Photo | curved shapes | 0.07 | 2.74 | 0.048 | 0.31 | 0.31 | 1.2 | 13 | 1.44 | 18 | 18 | 100 | 0.0 | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Low Fidelity Photo | polygons | 0.06 | 2.49 | 0.042 | 0.42 | 1.39 | 1.7 | 14 | 1.28 | 86 | 7 | 93 | 0.0 | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Low Fidelity Photo | thin lines | 0.49 | 5.70 | 0.082 | 0.70 | 45.31 | 7.0 | 56 | 2.80 | 49 | 11 | – | – | 4.0 | 3985 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Low Fidelity Photo | askew shapes | 0.23 | 4.65 | 0.059 | 0.70 | 21.55 | 6.0 | 42 | 1.76 | 85 | 3 | – | – | 2.3 | 2630 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Low Fidelity Photo | plain shapes | 0.11 | 2.25 | 0.047 | 0.30 | 0.28 | 6.0 | 41 | 1.38 | 58 | 48 | – | – | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | 0 | 0.00 | – | 0.0 | – | – | – |
| Low Fidelity Photo | flat colour art | 0.15 | 4.25 | 0.044 | 0.28 | 0.28 | 3.5 | 54 | 3.38 | 31 | 29 | 100 | 0.0 | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Low Fidelity Photo | gradient, photo | 2.53 | 2.80 | – | – | – | 42.7 | 2307 | – | 43 | 37 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| Low Fidelity Photo | linear gradients | 0.69 | 3.09 | – | – | – | 24.3 | 1061 | – | 31 | 31 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| 3 Colors | curved shapes | 0.07 | 2.74 | 0.048 | 0.31 | 0.31 | 1.8 | 16 | 1.67 | 15 | 15 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 3 Colors | polygons | 0.06 | 2.49 | 0.042 | 0.42 | 1.39 | 2.0 | 16 | 1.42 | 86 | 7 | 93 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 3 Colors | thin lines | 0.43 | 4.48 | 0.117 | 1.04 | 45.31 | 5.0 | 43 | 2.13 | 54 | 13 | – | – | 2.0 | 308 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 3 Colors | askew shapes | 0.18 | 3.58 | 0.054 | 0.70 | 14.59 | 4.0 | 25 | 1.04 | 97 | 3 | – | – | 0.7 | 23 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 3 Colors | plain shapes | 0.11 | 2.25 | 0.047 | 0.30 | 0.28 | 6.0 | 41 | 1.38 | 58 | 48 | – | – | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | 0 | 0.00 | – | 0.0 | – | – | – |
| 3 Colors | flat colour art | 1.31 | 3.61 | 0.125 | 0.90 | 60.25 | 2.5 | 25 | 1.86 | 45 | 21 | 100 | 0.0 | 1.0 | 9914 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 3 Colors | gradient, photo | 7.49 | 7.60 | – | – | – | 5.7 | 524 | – | 48 | 40 | – | – | – | – | 6.9 | 8.7 | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| 3 Colors | linear gradients | 4.45 | 5.79 | – | – | – | 3.0 | 78 | – | 28 | 24 | – | – | – | – | 10.3 | 1.9 | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| 6 Colors | curved shapes | 0.06 | 2.43 | 0.043 | 0.27 | 0.28 | 1.8 | 16 | 1.77 | 16 | 16 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 6 Colors | polygons | 0.06 | 2.40 | 0.041 | 0.42 | 1.39 | 2.0 | 16 | 1.45 | 86 | 7 | 93 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 6 Colors | thin lines | 0.49 | 4.76 | 0.108 | 0.70 | 45.31 | 4.0 | 30 | 1.50 | 63 | 19 | – | – | 1.7 | 413 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 6 Colors | askew shapes | 0.19 | 3.72 | 0.058 | 0.73 | 36.06 | 4.0 | 25 | 1.06 | 90 | 3 | – | – | 0.3 | 11 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 6 Colors | plain shapes | 0.10 | 2.07 | 0.043 | 0.30 | 0.26 | 6.0 | 42 | 1.41 | 59 | 49 | – | – | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | 0 | 0.00 | – | 0.0 | – | – | – |
| 6 Colors | flat colour art | 0.13 | 2.72 | 0.057 | 0.31 | 0.30 | 3.0 | 28 | 2.00 | 43 | 19 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 6 Colors | gradient, photo | 5.00 | 5.28 | – | – | – | 13.0 | 829 | – | 37 | 28 | – | – | – | – | 6.9 | 8.0 | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| 6 Colors | linear gradients | 2.07 | 3.07 | – | – | – | 6.2 | 266 | – | 20 | 14 | – | – | – | – | 2.4 | 9.3 | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| 16 Colors | curved shapes | 0.05 | 2.21 | 0.038 | 0.24 | 0.25 | 1.8 | 17 | 1.83 | 16 | 16 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 16 Colors | polygons | 0.06 | 2.34 | 0.039 | 0.42 | 1.39 | 2.0 | 16 | 1.48 | 86 | 7 | 93 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 16 Colors | thin lines | 0.39 | 3.90 | 0.113 | 0.70 | 23.53 | 8.7 | 52 | 2.58 | 56 | 17 | – | – | 5.7 | 322 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 16 Colors | askew shapes | 0.18 | 3.65 | 0.062 | 0.73 | 21.55 | 4.3 | 28 | 1.17 | 87 | 3 | – | – | 0.7 | 17 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 16 Colors | plain shapes | 0.09 | 1.92 | 0.041 | 0.30 | 0.26 | 6.0 | 44 | 1.46 | 60 | 49 | – | – | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | 0 | 0.00 | – | 0.0 | – | – | – |
| 16 Colors | flat colour art | 0.12 | 2.53 | 0.051 | 0.31 | 0.30 | 3.0 | 28 | 2.11 | 43 | 19 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 16 Colors | gradient, photo | 3.00 | 3.30 | – | – | – | 47.2 | 2126 | – | 30 | 16 | – | – | – | – | 15.2 | 19.6 | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| 16 Colors | linear gradients | 0.98 | 1.67 | – | – | – | 15.8 | 1340 | – | 8 | 6 | – | – | – | – | 13.9 | 15.5 | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| Shades of Gray | curved shapes | 0.07 | 2.70 | 0.047 | 0.31 | 0.31 | 1.2 | 13 | 1.45 | 18 | 18 | 100 | 0.0 | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Shades of Gray | polygons | 0.06 | 2.28 | 0.039 | 0.41 | 1.26 | 1.7 | 13 | 1.18 | 89 | 5 | 95 | 0.0 | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Shades of Gray | thin lines | 0.56 | 7.08 | 0.088 | 0.68 | 45.31 | 10.3 | 73 | 3.65 | 55 | 11 | – | – | 8.0 | 4319 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Shades of Gray | askew shapes | 0.25 | 4.95 | 0.049 | 0.73 | 21.52 | 6.0 | 39 | 1.61 | 95 | 3 | – | – | 2.3 | 2638 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Shades of Gray | plain shapes | 0.12 | 2.31 | 0.048 | 0.31 | 0.28 | 6.0 | 42 | 1.39 | 54 | 48 | – | – | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | 0 | 0.00 | – | 0.0 | – | – | – |
| Shades of Gray | flat colour art | 7.33 | 11.24 | 0.257 | 1.47 | 179.39 | 4.7 | 199 | 18.39 | 22 | 22 | 50 | 0.3 | 3.7 | 173367 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Shades of Gray | gradient, photo | 9.71 | 9.55 | – | – | – | 18.3 | 753 | – | 51 | 41 | – | – | – | – | 0.0 | – | 0.5 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| Shades of Gray | linear gradients | 5.25 | 8.40 | – | – | – | 9.0 | 477 | – | 31 | 31 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| Black and White Logo | curved shapes | 0.03 | 1.48 | 0.024 | 0.16 | 0.16 | 1.2 | 20 | 2.12 | 21 | 21 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Black and White Logo | polygons | 0.06 | 2.36 | 0.040 | 0.47 | 1.26 | 1.7 | 14 | 1.24 | 93 | 8 | 95 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Black and White Logo | thin lines | 0.59 | 6.00 | 0.105 | 0.50 | 77.21 | 10.3 | 44 | 2.18 | 90 | 10 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Black and White Logo | askew shapes | 0.49 | 11.14 | 0.234 | 1.46 | 9.63 | 4.0 | 25 | 1.03 | 98 | 70 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Black and White Logo | plain shapes | 0.08 | 1.56 | 0.033 | 0.31 | 0.30 | 6.0 | 55 | 1.84 | 58 | 52 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | 0 | 0.00 | – | 0.0 | – | – | – |
| Black and White Logo | flat colour art | 19.90 | 17.76 | 0.157 | 0.30 | 179.51 | 1.0 | 18 | 1.12 | 23 | 23 | 0 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Black and White Logo | gradient, photo | 38.97 | 38.53 | – | – | – | 9.0 | 634 | – | 73 | 54 | – | – | – | – | – | – | 1.2 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| Black and White Logo | linear gradients | 25.62 | 29.78 | – | – | – | 1.5 | 127 | – | 46 | 33 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| Sketched Art | curved shapes | 0.14 | 5.71 | 0.110 | 0.46 | 0.46 | 1.2 | 12 | 1.37 | 16 | 16 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Sketched Art | polygons | 0.12 | 4.71 | 0.085 | 0.37 | 1.08 | 1.7 | 13 | 1.16 | 86 | 5 | 93 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Sketched Art | thin lines | 0.47 | 5.70 | 0.113 | 0.96 | 52.59 | 4.3 | 29 | 1.45 | 64 | 3 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Sketched Art | askew shapes | 0.32 | 6.03 | 0.091 | 0.45 | 35.94 | 3.3 | 23 | 0.94 | 96 | 4 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Sketched Art | plain shapes | 0.30 | 5.84 | 0.107 | 0.47 | 0.47 | 6.0 | 40 | 1.32 | 53 | 47 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | 0 | 0.00 | – | 0.0 | – | – | – |
| Sketched Art | flat colour art | 20.25 | 19.55 | 0.146 | 1.15 | 179.63 | 1.5 | 26 | 1.53 | 16 | 16 | 0 | 0.3 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Sketched Art | gradient, photo | 45.42 | 45.21 | – | – | – | 1.0 | 244 | – | 46 | 35 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| Sketched Art | linear gradients | 29.20 | 33.47 | – | – | – | 1.5 | 54 | – | 35 | 34 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| Silhouettes | curved shapes | 0.35 | 14.53 | 0.281 | 0.69 | 0.69 | 1.2 | 12 | 1.33 | 16 | 16 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Silhouettes | polygons | 0.34 | 13.44 | 0.251 | 0.64 | 1.67 | 1.7 | 14 | 1.28 | 74 | 6 | 86 | 0.1 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Silhouettes | thin lines | 0.99 | 13.74 | 0.266 | 0.77 | 18.26 | 5.0 | 30 | 1.50 | 67 | 12 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Silhouettes | askew shapes | 0.58 | 14.93 | 0.297 | 0.55 | 0.55 | 4.0 | 24 | 1.01 | 95 | 3 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Silhouettes | plain shapes | 0.76 | 15.17 | 0.291 | 0.69 | 0.69 | 6.0 | 39 | 1.29 | 54 | 46 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | 0 | 0.00 | – | 0.0 | – | – | – |
| Silhouettes | flat colour art | 23.83 | 33.06 | 0.200 | 0.69 | 188.27 | 1.0 | 11 | 0.77 | 25 | 25 | 0 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Silhouettes | gradient, photo | 61.57 | 61.32 | – | – | – | 0.2 | 2 | – | 47 | 21 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| Silhouettes | linear gradients | 39.85 | 46.48 | – | – | – | 1.0 | 37 | – | 1 | 0 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| Line Art | curved shapes | 0.04 | 1.83 | 0.031 | 0.22 | 0.22 | 1.2 | 16 | 1.70 | 20 | 20 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Line Art | polygons | 0.05 | 2.11 | 0.036 | 0.47 | 1.26 | 1.7 | 14 | 1.26 | 89 | 5 | 95 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Line Art | thin lines | 0.52 | 4.09 | 0.040 | 0.47 | 105.72 | 3.3 | 29 | 1.45 | 40 | 0 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Line Art | askew shapes | 0.12 | 3.13 | 0.044 | 0.48 | 9.60 | 4.0 | 24 | 1.01 | 98 | 3 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Line Art | plain shapes | 0.09 | 1.76 | 0.038 | 0.30 | 0.29 | 6.0 | 47 | 1.56 | 56 | 51 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | 0 | 0.00 | – | 0.0 | – | – | – |
| Line Art | flat colour art | 19.90 | 17.83 | 0.162 | 0.42 | 179.43 | 1.0 | 18 | 1.30 | 21 | 21 | 0 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Line Art | gradient, photo | 38.98 | 38.54 | – | – | – | 7.7 | 472 | – | 58 | 44 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| Line Art | linear gradients | 25.63 | 29.80 | – | – | – | 1.5 | 95 | – | 33 | 32 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| Technical Drawing | curved shapes | 0.03 | 1.48 | 0.024 | 0.16 | 0.16 | 1.2 | 20 | 2.12 | 21 | 21 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Technical Drawing | polygons | 0.06 | 2.36 | 0.040 | 0.47 | 1.26 | 1.7 | 14 | 1.23 | 94 | 8 | 95 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Technical Drawing | thin lines | 0.50 | 5.46 | 0.109 | 0.50 | 55.93 | 15.3 | 62 | 3.12 | 86 | 14 | – | – | – | – | – | – | 3.3 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Technical Drawing | askew shapes | 0.48 | 11.07 | 0.233 | 1.46 | 2.11 | 4.3 | 26 | 1.08 | 98 | 70 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Technical Drawing | plain shapes | 0.08 | 1.63 | 0.035 | 0.33 | 0.33 | 6.0 | 55 | 1.82 | 60 | 52 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | 0 | 0.00 | – | 0.0 | – | – | – |
| Technical Drawing | flat colour art | 19.90 | 17.76 | 0.157 | 0.30 | 179.51 | 1.0 | 18 | 1.12 | 23 | 23 | 0 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Technical Drawing | gradient, photo | 38.94 | 38.51 | – | – | – | 20.0 | 659 | – | 73 | 51 | – | – | – | – | – | – | 11.7 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| Technical Drawing | linear gradients | 25.62 | 29.80 | – | – | – | 1.5 | 118 | – | 49 | 33 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| Outline | curved shapes | 23.38 | 44.76 | 0.499 | 0.71 | 0.52 | 1.2 | 16 | 1.70 | 20 | 20 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Outline | polygons | 17.94 | 44.26 | 0.494 | 0.94 | 0.76 | 1.7 | 14 | 1.26 | 89 | 5 | 95 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Outline | thin lines | 0.51 | 3.57 | 0.028 | 0.41 | 105.74 | 3.3 | 7 | 0.33 | 100 | 7 | – | – | – | – | – | – | 0.0 | 0.0 | 0.020 | 0.02 | 2.0 | – | 0.00 | – | 0.0 | – | – | – |
| Outline | askew shapes | 23.68 | 36.53 | 0.422 | 0.62 | 6.62 | 4.0 | 20 | 0.83 | 100 | 3 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Outline | plain shapes | 34.32 | 43.23 | 0.495 | 0.73 | 0.51 | 6.0 | 47 | 1.56 | 56 | 51 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | 0 | 0.00 | – | 0.0 | – | – | – |
| Outline | flat colour art | 26.69 | 32.18 | 0.494 | 0.92 | 178.93 | 1.0 | 18 | 1.30 | 21 | 21 | 0 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Outline | gradient, photo | 39.38 | 39.18 | – | – | – | 7.5 | 445 | – | 59 | 43 | – | – | – | – | – | – | 3.3 | 0.5 | – | – | – | – | – | – | 0.0 | – | – | – |
| Outline | linear gradients | 25.08 | 30.80 | – | – | – | 1.5 | 92 | – | 33 | 32 | – | – | – | – | – | – | 0.0 | 0.5 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| Flat Logo | curved shapes | 0.06 | 2.14 | 0.036 | 0.25 | 0.25 | 1.8 | 31 | 3.38 | 19 | 6 | 100 | 0.0 | 0.0 | 0 | 6.8 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Flat Logo | polygons | 0.09 | 3.44 | 0.060 | 0.35 | 1.03 | 2.0 | 35 | 3.17 | 67 | 0 | 94 | 0.0 | 0.0 | 0 | 6.8 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Flat Logo | thin lines | 1.17 | 11.78 | 0.050 | 0.49 | all | 3.0 | 189 | 9.45 | 28 | 9 | – | – | 0.0 | 0 | 5467.1 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Flat Logo | askew shapes | 0.15 | 3.78 | 0.058 | 0.43 | 0.50 | 4.0 | 114 | 4.76 | 39 | 0 | – | – | 0.0 | 0 | 6.8 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Flat Logo | plain shapes | 0.16 | 3.09 | 0.061 | 0.30 | 0.27 | 6.0 | 70 | 2.32 | 57 | 4 | – | – | 0.0 | 0 | 6.8 | – | 0.0 | 0.0 | – | – | – | 0 | 0.00 | – | 0.0 | – | – | – |
| Flat Logo | flat colour art | 0.07 | 1.53 | 0.065 | 0.31 | 0.27 | 3.0 | 36 | 3.51 | 47 | 11 | 100 | 0.0 | 0.0 | 0 | 6.8 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Flat Logo | gradient, photo | refused | | | | | | | | | | | | | | | | | | | | | | | | | | | |
| Flat Logo | linear gradients | 6.49 | 6.47 | – | – | – | 484.0 | 4773 | – | 58 | 0 | – | – | – | – | 146.0 | – | 446.7 | 0.1 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| Default with Shapes | curved shapes | 0.08 | 3.21 | 0.056 | 0.38 | 0.38 | 1.8 | 13 | 1.44 | 14 | 14 | 100 | 0.0 | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Default with Shapes | polygons | 0.07 | 2.58 | 0.045 | 0.50 | 1.26 | 2.0 | 13 | 1.16 | 97 | 5 | 100 | 0.0 | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Default with Shapes | thin lines | 0.54 | 4.50 | 0.049 | 0.47 | 105.72 | 3.3 | 25 | 1.27 | 40 | 0 | – | – | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Default with Shapes | askew shapes | 0.16 | 3.24 | 0.041 | 0.37 | 36.06 | 3.7 | 23 | 0.94 | 100 | 3 | – | – | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Default with Shapes | plain shapes | 0.11 | 2.30 | 0.049 | 0.28 | 0.28 | 6.0 | 30 | 1.00 | 67 | 54 | – | – | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | 100 | 0.00 | – | 0.0 | – | – | – |
| Default with Shapes | flat colour art | 19.91 | 17.94 | 0.180 | 0.39 | 179.45 | 1.5 | 16 | 1.01 | 14 | 14 | 0 | 0.0 | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Default with Shapes | gradient, photo | 39.03 | 38.64 | – | – | – | 3.2 | 595 | – | 55 | 41 | – | – | – | – | 18.8 | 35.5 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Default with Shapes | linear gradients | 25.64 | 29.86 | – | – | – | 2.0 | 43 | – | 33 | 32 | – | – | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | 0 | – | – |
| 6 Colors with Shapes | curved shapes | 0.05 | 2.25 | 0.039 | 0.27 | 0.28 | 1.8 | 15 | 1.59 | 16 | 16 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 6 Colors with Shapes | polygons | 0.06 | 2.28 | 0.042 | 0.63 | 1.39 | 2.0 | 14 | 1.27 | 94 | 7 | 99 | 0.0 | 0.0 | 0 | 2.0 | 2.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 6 Colors with Shapes | thin lines | 0.49 | 4.79 | 0.108 | 0.70 | 45.31 | 4.0 | 30 | 1.50 | 63 | 19 | – | – | 1.7 | 413 | 8.1 | 2.3 | 0.0 | 0.0 | – | – | – | – | 0.33 | – | 0.0 | – | – | – |
| 6 Colors with Shapes | askew shapes | 0.18 | 3.59 | 0.053 | 0.73 | 36.06 | 4.0 | 25 | 1.03 | 99 | 3 | – | – | 0.3 | 11 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 6 Colors with Shapes | plain shapes | 0.09 | 1.88 | 0.039 | 0.29 | 0.29 | 6.0 | 30 | 1.00 | 67 | 54 | – | – | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | 100 | 0.00 | – | 0.0 | – | – | – |
| 6 Colors with Shapes | flat colour art | 0.13 | 2.72 | 0.057 | 0.31 | 0.30 | 3.0 | 28 | 2.00 | 43 | 19 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 6 Colors with Shapes | gradient, photo | 5.00 | 5.28 | – | – | – | 13.0 | 828 | – | 37 | 28 | – | – | – | – | 7.5 | 10.6 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 6 Colors with Shapes | linear gradients | 2.07 | 3.07 | – | – | – | 6.2 | 266 | – | 20 | 14 | – | – | – | – | 2.4 | 9.3 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | 0 | – | – |
| 6 Colors with Auto Grouping | curved shapes | 0.06 | 2.43 | 0.043 | 0.27 | 0.28 | 1.8 | 16 | 1.77 | 16 | 16 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 17 | 0.0 | – | – | – |
| 6 Colors with Auto Grouping | polygons | 0.06 | 2.40 | 0.041 | 0.42 | 1.39 | 2.0 | 16 | 1.45 | 86 | 7 | 93 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 11 | 0.0 | – | – | – |
| 6 Colors with Auto Grouping | thin lines | 0.49 | 4.76 | 0.108 | 0.70 | 45.31 | 4.0 | 30 | 1.50 | 63 | 19 | – | – | 1.7 | 413 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| 6 Colors with Auto Grouping | askew shapes | 0.19 | 3.72 | 0.058 | 0.73 | 36.06 | 4.0 | 25 | 1.06 | 90 | 3 | – | – | 0.3 | 11 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| 6 Colors with Auto Grouping | plain shapes | 0.10 | 2.07 | 0.043 | 0.30 | 0.26 | 6.0 | 42 | 1.41 | 59 | 49 | – | – | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | 0 | 0.00 | 0 | 0.0 | – | – | – |
| 6 Colors with Auto Grouping | flat colour art | 0.13 | 2.72 | 0.057 | 0.31 | 0.30 | 3.0 | 28 | 2.00 | 43 | 19 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 30 | 0.0 | – | – | – |
| 6 Colors with Auto Grouping | gradient, photo | 5.00 | 5.28 | – | – | – | 13.0 | 829 | – | 37 | 28 | – | – | – | – | 6.9 | 8.0 | 0.0 | 0.0 | – | – | – | – | – | 15 | 0.0 | – | – | – |
| 6 Colors with Auto Grouping | linear gradients | 2.07 | 3.07 | – | – | – | 6.2 | 266 | – | 20 | 14 | – | – | – | – | 2.4 | 9.3 | 0.0 | 0.0 | – | – | – | – | – | 0 | 0.0 | 0 | – | – |
| Shades of Gray with Auto Grouping | curved shapes | 0.07 | 2.70 | 0.047 | 0.31 | 0.31 | 1.2 | 13 | 1.45 | 18 | 18 | 100 | 0.0 | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Shades of Gray with Auto Grouping | polygons | 0.06 | 2.28 | 0.039 | 0.41 | 1.26 | 1.7 | 13 | 1.18 | 89 | 5 | 95 | 0.0 | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Shades of Gray with Auto Grouping | thin lines | 0.56 | 7.08 | 0.088 | 0.68 | 45.31 | 10.3 | 73 | 3.65 | 55 | 11 | – | – | 8.0 | 4319 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Shades of Gray with Auto Grouping | askew shapes | 0.25 | 4.95 | 0.049 | 0.73 | 21.52 | 6.0 | 39 | 1.61 | 95 | 3 | – | – | 2.3 | 2638 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 3 | 0.0 | – | – | – |
| Shades of Gray with Auto Grouping | plain shapes | 0.12 | 2.31 | 0.048 | 0.31 | 0.28 | 6.0 | 42 | 1.39 | 54 | 48 | – | – | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | 0 | 0.00 | 0 | 0.0 | – | – | – |
| Shades of Gray with Auto Grouping | flat colour art | 7.33 | 11.24 | 0.257 | 1.47 | 179.39 | 4.7 | 199 | 18.39 | 22 | 22 | 50 | 0.3 | 3.7 | 173367 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 52 | 0.0 | – | – | – |
| Shades of Gray with Auto Grouping | gradient, photo | 9.71 | 9.55 | – | – | – | 18.3 | 753 | – | 51 | 41 | – | – | – | – | 0.0 | – | 0.5 | 0.0 | – | – | – | – | – | 53 | 0.0 | – | – | – |
| Shades of Gray with Auto Grouping | linear gradients | 5.25 | 8.40 | – | – | – | 9.0 | 477 | – | 31 | 31 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | – | 68 | 0.0 | 0 | – | – |
| High Fidelity Photo with Auto Grouping | curved shapes | 0.04 | 1.63 | 0.026 | 0.15 | 0.15 | 1.2 | 19 | 2.11 | 21 | 21 | 100 | 0.0 | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| High Fidelity Photo with Auto Grouping | polygons | 0.06 | 2.33 | 0.039 | 0.50 | 1.32 | 1.7 | 17 | 1.47 | 88 | 7 | 93 | 0.0 | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| High Fidelity Photo with Auto Grouping | thin lines | 0.35 | 6.05 | 0.085 | 0.90 | 4.35 | 40.3 | 348 | 17.42 | 35 | 9 | – | – | 27.3 | 4505 | 0.0 | – | 19.0 | 0.0 | – | – | – | – | 0.00 | 23 | 0.0 | – | – | – |
| High Fidelity Photo with Auto Grouping | askew shapes | 0.17 | 3.97 | 0.100 | 0.94 | 1.73 | 8.7 | 157 | 6.54 | 71 | 5 | – | – | 4.3 | 3998 | 0.0 | – | 1.0 | 0.0 | – | – | – | – | 0.00 | 18 | 0.0 | – | – | – |
| High Fidelity Photo with Auto Grouping | plain shapes | 0.08 | 1.66 | 0.035 | 0.25 | 0.28 | 6.0 | 50 | 1.68 | 62 | 51 | – | – | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | 0 | 0.00 | 0 | 0.0 | – | – | – |
| High Fidelity Photo with Auto Grouping | flat colour art | 0.14 | 4.16 | 0.029 | 0.20 | 0.20 | 3.5 | 73 | 4.59 | 35 | 34 | 100 | 0.0 | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 25 | 0.0 | – | – | – |
| High Fidelity Photo with Auto Grouping | gradient, photo | 1.80 | 2.05 | – | – | – | 134.7 | 11212 | – | 39 | 37 | – | – | – | – | 0.0 | – | 7.3 | 0.0 | – | – | – | – | – | 69 | 0.0 | – | – | – |
| High Fidelity Photo with Auto Grouping | linear gradients | 0.64 | 3.29 | – | – | – | 34.5 | 4318 | – | 32 | 31 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | – | 72 | 0.0 | 0 | – | – |
| 6 Colors, no Gradients | curved shapes | 0.06 | 2.43 | 0.043 | 0.27 | 0.28 | 1.8 | 16 | 1.77 | 16 | 16 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 6 Colors, no Gradients | polygons | 0.06 | 2.40 | 0.041 | 0.42 | 1.39 | 2.0 | 16 | 1.45 | 86 | 7 | 93 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 6 Colors, no Gradients | thin lines | 0.49 | 4.76 | 0.108 | 0.70 | 45.31 | 4.0 | 30 | 1.50 | 63 | 19 | – | – | 1.7 | 413 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 6 Colors, no Gradients | askew shapes | 0.19 | 3.72 | 0.058 | 0.73 | 36.06 | 4.0 | 25 | 1.06 | 90 | 3 | – | – | 0.3 | 11 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 6 Colors, no Gradients | plain shapes | 0.10 | 2.07 | 0.043 | 0.30 | 0.26 | 6.0 | 42 | 1.41 | 59 | 49 | – | – | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | 0 | 0.00 | – | 0.0 | – | – | – |
| 6 Colors, no Gradients | flat colour art | 0.13 | 2.72 | 0.057 | 0.31 | 0.30 | 3.0 | 28 | 2.00 | 43 | 19 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 6 Colors, no Gradients | gradient, photo | 5.00 | 5.28 | – | – | – | 13.0 | 829 | – | 37 | 28 | – | – | – | – | 6.9 | 8.0 | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| 6 Colors, no Gradients | linear gradients | 2.07 | 3.07 | – | – | – | 6.2 | 266 | – | 20 | 14 | – | – | – | – | 2.4 | 9.3 | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| 6 Colors with Gradients | curved shapes | 0.06 | 2.43 | 0.043 | 0.27 | 0.28 | 1.8 | 16 | 1.77 | 16 | 16 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 6 Colors with Gradients | polygons | 0.06 | 2.40 | 0.041 | 0.42 | 1.39 | 2.0 | 16 | 1.45 | 86 | 7 | 93 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 6 Colors with Gradients | thin lines | 0.49 | 4.76 | 0.108 | 0.70 | 45.31 | 4.0 | 30 | 1.50 | 63 | 19 | – | – | 1.7 | 413 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 6 Colors with Gradients | askew shapes | 0.19 | 3.72 | 0.058 | 0.73 | 36.06 | 4.0 | 25 | 1.06 | 90 | 3 | – | – | 0.3 | 11 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 6 Colors with Gradients | plain shapes | 0.10 | 2.07 | 0.043 | 0.30 | 0.26 | 6.0 | 42 | 1.41 | 59 | 49 | – | – | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | 0 | 0.00 | – | 0.0 | – | – | – |
| 6 Colors with Gradients | flat colour art | 0.13 | 2.72 | 0.057 | 0.31 | 0.30 | 3.0 | 28 | 2.00 | 43 | 19 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 6 Colors with Gradients | gradient, photo | 4.49 | 4.84 | – | – | – | 15.0 | 909 | – | 36 | 27 | – | – | – | – | 9.7 | 12.0 | 0.0 | 0.0 | – | – | – | – | – | – | 0.7 | – | – | – |
| 6 Colors with Gradients | linear gradients | 0.40 | 1.09 | – | – | – | 1.3 | 39 | – | 0 | 0 | – | – | – | – | 0.5 | 1.1 | 0.0 | 0.0 | – | – | – | – | – | – | 0.8 | 83 | 0.2 | 5 |
| High Fidelity Photo, no Gradients | curved shapes | 0.04 | 1.63 | 0.026 | 0.15 | 0.15 | 1.2 | 19 | 2.11 | 21 | 21 | 100 | 0.0 | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| High Fidelity Photo, no Gradients | polygons | 0.06 | 2.33 | 0.039 | 0.50 | 1.32 | 1.7 | 17 | 1.47 | 88 | 7 | 93 | 0.0 | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| High Fidelity Photo, no Gradients | thin lines | 0.35 | 6.05 | 0.085 | 0.90 | 4.35 | 40.3 | 348 | 17.42 | 35 | 9 | – | – | 27.3 | 4505 | 0.0 | – | 19.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| High Fidelity Photo, no Gradients | askew shapes | 0.17 | 3.97 | 0.100 | 0.94 | 1.73 | 8.7 | 157 | 6.54 | 71 | 5 | – | – | 4.3 | 3998 | 0.0 | – | 1.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| High Fidelity Photo, no Gradients | plain shapes | 0.08 | 1.66 | 0.035 | 0.25 | 0.28 | 6.0 | 50 | 1.68 | 62 | 51 | – | – | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | 0 | 0.00 | – | 0.0 | – | – | – |
| High Fidelity Photo, no Gradients | flat colour art | 0.14 | 4.16 | 0.029 | 0.20 | 0.20 | 3.5 | 73 | 4.59 | 35 | 34 | 100 | 0.0 | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| High Fidelity Photo, no Gradients | gradient, photo | 1.80 | 2.05 | – | – | – | 134.7 | 11212 | – | 39 | 37 | – | – | – | – | 0.0 | – | 7.3 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| High Fidelity Photo, no Gradients | linear gradients | 0.64 | 3.29 | – | – | – | 34.5 | 4318 | – | 32 | 31 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| High Fidelity Photo with Gradients | curved shapes | 0.04 | 1.63 | 0.026 | 0.15 | 0.15 | 1.2 | 19 | 2.11 | 21 | 21 | 100 | 0.0 | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| High Fidelity Photo with Gradients | polygons | 0.06 | 2.33 | 0.039 | 0.50 | 1.32 | 1.7 | 17 | 1.47 | 88 | 7 | 93 | 0.0 | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| High Fidelity Photo with Gradients | thin lines | 0.35 | 6.05 | 0.085 | 0.90 | 4.35 | 40.3 | 348 | 17.42 | 35 | 9 | – | – | 27.3 | 4505 | 0.0 | – | 19.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| High Fidelity Photo with Gradients | askew shapes | 0.17 | 3.97 | 0.100 | 0.94 | 1.73 | 8.7 | 157 | 6.54 | 71 | 5 | – | – | 4.3 | 3998 | 0.0 | – | 1.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| High Fidelity Photo with Gradients | plain shapes | 0.08 | 1.66 | 0.035 | 0.25 | 0.28 | 6.0 | 50 | 1.68 | 62 | 51 | – | – | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | 0 | 0.00 | – | 0.0 | – | – | – |
| High Fidelity Photo with Gradients | flat colour art | 0.14 | 4.16 | 0.029 | 0.20 | 0.20 | 3.5 | 73 | 4.59 | 35 | 34 | 100 | 0.0 | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| High Fidelity Photo with Gradients | gradient, photo | 1.79 | 2.04 | – | – | – | 138.0 | 11156 | – | 40 | 39 | – | – | – | – | 0.0 | – | 9.2 | 0.0 | – | – | – | – | – | – | 0.8 | – | – | – |
| High Fidelity Photo with Gradients | linear gradients | 0.23 | 1.19 | – | – | – | 6.0 | 252 | – | 0 | 0 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | – | – | 0.8 | 83 | 0.2 | 5 |
| Default, enhanced | curved shapes | 0.08 | 3.21 | 0.056 | 0.38 | 0.38 | 1.8 | 13 | 1.44 | 14 | 14 | 100 | 0.0 | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 17 | 0.0 | – | – | – |
| Default, enhanced | polygons | 0.07 | 2.58 | 0.045 | 0.50 | 1.26 | 2.0 | 13 | 1.16 | 97 | 5 | 100 | 0.0 | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 11 | 0.0 | – | – | – |
| Default, enhanced | thin lines | 0.54 | 4.50 | 0.049 | 0.47 | 105.72 | 3.3 | 25 | 1.27 | 40 | 0 | – | – | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Default, enhanced | askew shapes | 0.16 | 3.24 | 0.041 | 0.37 | 36.06 | 3.7 | 23 | 0.94 | 100 | 3 | – | – | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Default, enhanced | plain shapes | 0.11 | 2.30 | 0.049 | 0.28 | 0.28 | 6.0 | 30 | 1.00 | 67 | 54 | – | – | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | 100 | 0.00 | 0 | 0.0 | – | – | – |
| Default, enhanced | flat colour art | 19.91 | 17.94 | 0.180 | 0.39 | 179.45 | 1.5 | 16 | 1.01 | 14 | 14 | 0 | 0.0 | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 17 | 0.0 | – | – | – |
| Default, enhanced | gradient, photo | 39.03 | 38.64 | – | – | – | 3.2 | 595 | – | 55 | 41 | – | – | – | – | 18.8 | 35.5 | 0.0 | 0.0 | – | – | – | – | 0.00 | 15 | 0.0 | – | – | – |
| Default, enhanced | linear gradients | 25.64 | 29.86 | – | – | – | 2.0 | 43 | – | 33 | 32 | – | – | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | 0 | – | – |
| Auto Color, enhanced | curved shapes | 0.05 | 2.25 | 0.039 | 0.27 | 0.28 | 1.8 | 15 | 1.59 | 16 | 16 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 17 | 0.0 | – | – | – |
| Auto Color, enhanced | polygons | 0.06 | 2.30 | 0.042 | 0.63 | 1.39 | 2.0 | 14 | 1.25 | 95 | 7 | 99 | 0.0 | 0.0 | 0 | 2.0 | 2.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 11 | 0.0 | – | – | – |
| Auto Color, enhanced | thin lines | 0.70 | 5.57 | 0.067 | 1.08 | 105.72 | 5.0 | 38 | 1.88 | 43 | 3 | – | – | 4.0 | 3225 | 5.2 | 4.6 | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Auto Color, enhanced | askew shapes | 0.64 | 3.72 | 0.044 | 0.36 | 9.49 | 4.0 | 24 | 1.00 | 100 | 3 | – | – | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Auto Color, enhanced | plain shapes | 0.09 | 1.88 | 0.039 | 0.29 | 0.29 | 6.0 | 30 | 1.00 | 67 | 54 | – | – | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | 100 | 0.00 | 0 | 0.0 | – | – | – |
| Auto Color, enhanced | flat colour art | 0.13 | 2.72 | 0.057 | 0.31 | 0.30 | 3.0 | 28 | 2.00 | 43 | 19 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 30 | 0.0 | – | – | – |
| Auto Color, enhanced | gradient, photo | 2.08 | 2.39 | – | – | – | 159.3 | 4439 | – | 22 | 9 | – | – | – | – | 41.7 | 51.9 | 0.0 | 0.5 | – | – | – | – | 0.00 | 8 | 0.0 | – | – | – |
| Auto Color, enhanced | linear gradients | 1.02 | 1.74 | – | – | – | 15.7 | 1272 | – | 12 | 8 | – | – | – | – | 9.1 | 7.4 | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | 0 | – | – |
| High Fidelity Photo, enhanced | curved shapes | 0.04 | 1.63 | 0.026 | 0.15 | 0.15 | 1.2 | 19 | 2.11 | 21 | 21 | 100 | 0.0 | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| High Fidelity Photo, enhanced | polygons | 0.06 | 2.33 | 0.039 | 0.50 | 1.32 | 1.7 | 17 | 1.47 | 88 | 7 | 93 | 0.0 | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| High Fidelity Photo, enhanced | thin lines | 0.35 | 6.05 | 0.085 | 0.90 | 4.35 | 40.3 | 348 | 17.42 | 35 | 9 | – | – | 27.3 | 4505 | 0.0 | – | 19.0 | 0.0 | – | – | – | – | 0.00 | 23 | 0.0 | – | – | – |
| High Fidelity Photo, enhanced | askew shapes | 0.17 | 3.97 | 0.100 | 0.94 | 1.73 | 8.7 | 157 | 6.54 | 71 | 5 | – | – | 4.3 | 3998 | 0.0 | – | 1.0 | 0.0 | – | – | – | – | 0.00 | 18 | 0.0 | – | – | – |
| High Fidelity Photo, enhanced | plain shapes | 0.08 | 1.66 | 0.035 | 0.25 | 0.28 | 6.0 | 50 | 1.68 | 62 | 51 | – | – | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | 0 | 0.00 | 0 | 0.0 | – | – | – |
| High Fidelity Photo, enhanced | flat colour art | 0.14 | 4.16 | 0.029 | 0.20 | 0.20 | 3.5 | 73 | 4.59 | 35 | 34 | 100 | 0.0 | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 25 | 0.0 | – | – | – |
| High Fidelity Photo, enhanced | gradient, photo | 1.80 | 2.05 | – | – | – | 134.7 | 11212 | – | 39 | 37 | – | – | – | – | 0.0 | – | 7.3 | 0.0 | – | – | – | – | – | 69 | 0.0 | – | – | – |
| High Fidelity Photo, enhanced | linear gradients | 0.64 | 3.29 | – | – | – | 34.5 | 4318 | – | 32 | 31 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | – | 72 | 0.0 | 0 | – | – |
| Low Fidelity Photo, enhanced | curved shapes | 0.06 | 2.52 | 0.043 | 0.31 | 0.31 | 1.2 | 12 | 1.29 | 18 | 18 | 100 | 0.0 | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 1.00 | 0 | 0.0 | – | – | – |
| Low Fidelity Photo, enhanced | polygons | 0.06 | 2.34 | 0.039 | 0.42 | 1.39 | 1.7 | 12 | 1.13 | 95 | 7 | 99 | 0.0 | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 1.00 | 0 | 0.0 | – | – | – |
| Low Fidelity Photo, enhanced | thin lines | 0.49 | 5.73 | 0.082 | 0.68 | 45.31 | 7.0 | 53 | 2.63 | 53 | 11 | – | – | 4.0 | 3989 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 1.67 | 0 | 0.0 | – | – | – |
| Low Fidelity Photo, enhanced | askew shapes | 0.23 | 4.54 | 0.055 | 0.70 | 21.55 | 6.0 | 40 | 1.68 | 94 | 3 | – | – | 2.3 | 2630 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 1.00 | 3 | 0.0 | – | – | – |
| Low Fidelity Photo, enhanced | plain shapes | 0.10 | 2.02 | 0.042 | 0.27 | 0.27 | 6.0 | 30 | 1.00 | 67 | 54 | – | – | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | 100 | 1.00 | 0 | 0.0 | – | – | – |
| Low Fidelity Photo, enhanced | flat colour art | 0.15 | 4.19 | 0.042 | 0.28 | 0.20 | 3.5 | 48 | 2.93 | 32 | 31 | 100 | 0.0 | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 1.00 | 25 | 0.0 | – | – | – |
| Low Fidelity Photo, enhanced | gradient, photo | 2.53 | 2.80 | – | – | – | 42.7 | 2307 | – | 43 | 37 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 60 | 0.0 | – | – | – |
| Low Fidelity Photo, enhanced | linear gradients | 0.69 | 3.09 | – | – | – | 24.3 | 1045 | – | 31 | 31 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 79 | 0.0 | 0 | – | – |
| 3 Colors, enhanced | curved shapes | 0.06 | 2.52 | 0.043 | 0.31 | 0.31 | 1.8 | 14 | 1.53 | 15 | 15 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 17 | 0.0 | – | – | – |
| 3 Colors, enhanced | polygons | 0.06 | 2.34 | 0.043 | 0.63 | 1.39 | 2.0 | 14 | 1.26 | 94 | 7 | 99 | 0.0 | 0.0 | 0 | 2.0 | 2.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 11 | 0.0 | – | – | – |
| 3 Colors, enhanced | thin lines | 0.43 | 4.49 | 0.117 | 1.04 | 45.31 | 5.0 | 41 | 2.05 | 57 | 13 | – | – | 2.0 | 308 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.33 | 0 | 0.0 | – | – | – |
| 3 Colors, enhanced | askew shapes | 0.18 | 3.53 | 0.052 | 0.70 | 14.59 | 4.0 | 25 | 1.04 | 97 | 3 | – | – | 0.7 | 23 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| 3 Colors, enhanced | plain shapes | 0.10 | 2.02 | 0.042 | 0.27 | 0.27 | 6.0 | 30 | 1.00 | 67 | 54 | – | – | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | 100 | 0.00 | 0 | 0.0 | – | – | – |
| 3 Colors, enhanced | flat colour art | 1.31 | 3.58 | 0.124 | 0.90 | 60.11 | 2.5 | 23 | 1.77 | 45 | 20 | 100 | 0.0 | 1.0 | 9912 | 9.0 | 9.0 | 0.0 | 0.0 | – | – | – | – | 0.33 | 25 | 0.0 | – | – | – |
| 3 Colors, enhanced | gradient, photo | 7.49 | 7.60 | – | – | – | 5.7 | 524 | – | 48 | 40 | – | – | – | – | 7.0 | 8.8 | 0.0 | 0.0 | – | – | – | – | 0.00 | 20 | 0.0 | – | – | – |
| 3 Colors, enhanced | linear gradients | 4.45 | 5.79 | – | – | – | 3.0 | 78 | – | 28 | 24 | – | – | – | – | 10.3 | 1.9 | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | 0 | – | – |
| 6 Colors, enhanced | curved shapes | 0.05 | 2.25 | 0.039 | 0.27 | 0.28 | 1.8 | 15 | 1.59 | 16 | 16 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 17 | 0.0 | – | – | – |
| 6 Colors, enhanced | polygons | 0.06 | 2.28 | 0.042 | 0.63 | 1.39 | 2.0 | 14 | 1.27 | 94 | 7 | 99 | 0.0 | 0.0 | 0 | 2.0 | 2.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 11 | 0.0 | – | – | – |
| 6 Colors, enhanced | thin lines | 0.49 | 4.79 | 0.108 | 0.70 | 45.31 | 4.0 | 30 | 1.50 | 63 | 19 | – | – | 1.7 | 413 | 8.1 | 2.3 | 0.0 | 0.0 | – | – | – | – | 0.33 | 0 | 0.0 | – | – | – |
| 6 Colors, enhanced | askew shapes | 0.18 | 3.59 | 0.053 | 0.73 | 36.06 | 4.0 | 25 | 1.03 | 99 | 3 | – | – | 0.3 | 11 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| 6 Colors, enhanced | plain shapes | 0.09 | 1.88 | 0.039 | 0.29 | 0.29 | 6.0 | 30 | 1.00 | 67 | 54 | – | – | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | 100 | 0.00 | 0 | 0.0 | – | – | – |
| 6 Colors, enhanced | flat colour art | 0.13 | 2.72 | 0.057 | 0.31 | 0.30 | 3.0 | 28 | 2.00 | 43 | 19 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 30 | 0.0 | – | – | – |
| 6 Colors, enhanced | gradient, photo | 5.00 | 5.28 | – | – | – | 13.0 | 828 | – | 37 | 28 | – | – | – | – | 7.5 | 10.6 | 0.0 | 0.0 | – | – | – | – | 0.00 | 13 | 0.0 | – | – | – |
| 6 Colors, enhanced | linear gradients | 2.07 | 3.07 | – | – | – | 6.2 | 266 | – | 20 | 14 | – | – | – | – | 2.4 | 9.3 | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | 0 | – | – |
| 16 Colors, enhanced | curved shapes | 0.05 | 2.06 | 0.034 | 0.24 | 0.25 | 1.8 | 16 | 1.64 | 16 | 16 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 17 | 0.0 | – | – | – |
| 16 Colors, enhanced | polygons | 0.05 | 2.22 | 0.041 | 0.63 | 1.39 | 2.0 | 14 | 1.28 | 94 | 7 | 99 | 0.0 | 0.0 | 0 | 2.0 | 2.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 11 | 0.0 | – | – | – |
| 16 Colors, enhanced | thin lines | 0.39 | 3.93 | 0.113 | 0.70 | 23.53 | 8.7 | 52 | 2.58 | 56 | 17 | – | – | 5.7 | 322 | 8.1 | 2.3 | 0.0 | 0.0 | – | – | – | – | 0.33 | 0 | 0.0 | – | – | – |
| 16 Colors, enhanced | askew shapes | 0.17 | 3.52 | 0.058 | 0.73 | 21.55 | 4.3 | 27 | 1.14 | 95 | 3 | – | – | 0.7 | 17 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| 16 Colors, enhanced | plain shapes | 0.08 | 1.70 | 0.035 | 0.13 | 0.13 | 6.0 | 30 | 1.00 | 67 | 55 | – | – | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | 100 | 0.00 | 0 | 0.0 | – | – | – |
| 16 Colors, enhanced | flat colour art | 0.12 | 2.53 | 0.052 | 0.31 | 0.30 | 3.0 | 28 | 2.11 | 43 | 19 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 30 | 0.0 | – | – | – |
| 16 Colors, enhanced | gradient, photo | 3.00 | 3.30 | – | – | – | 47.2 | 2126 | – | 30 | 16 | – | – | – | – | 15.2 | 19.6 | 0.0 | 0.0 | – | – | – | – | 0.00 | 15 | 0.0 | – | – | – |
| 16 Colors, enhanced | linear gradients | 0.98 | 1.67 | – | – | – | 15.8 | 1340 | – | 8 | 6 | – | – | – | – | 13.9 | 15.5 | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | 0 | – | – |
| Shades of Gray, enhanced | curved shapes | 0.06 | 2.53 | 0.044 | 0.31 | 0.31 | 1.2 | 12 | 1.29 | 18 | 18 | 100 | 0.0 | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 1.00 | 0 | 0.0 | – | – | – |
| Shades of Gray, enhanced | polygons | 0.07 | 2.49 | 0.042 | 0.50 | 1.26 | 1.7 | 11 | 1.03 | 99 | 5 | 100 | 0.0 | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 1.00 | 0 | 0.0 | – | – | – |
| Shades of Gray, enhanced | thin lines | 0.56 | 7.12 | 0.089 | 0.68 | 45.31 | 10.3 | 73 | 3.65 | 55 | 11 | – | – | 8.0 | 4322 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 1.00 | 0 | 0.0 | – | – | – |
| Shades of Gray, enhanced | askew shapes | 0.25 | 4.88 | 0.046 | 0.73 | 21.52 | 6.0 | 39 | 1.61 | 95 | 3 | – | – | 2.3 | 2638 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 1.00 | 3 | 0.0 | – | – | – |
| Shades of Gray, enhanced | plain shapes | 0.09 | 1.97 | 0.042 | 0.25 | 0.25 | 6.0 | 30 | 1.00 | 67 | 54 | – | – | 0.0 | 0 | 0.0 | – | 0.0 | 0.0 | – | – | – | 100 | 1.00 | 0 | 0.0 | – | – | – |
| Shades of Gray, enhanced | flat colour art | 7.33 | 11.09 | 0.244 | 1.47 | 179.46 | 4.7 | 161 | 13.44 | 24 | 24 | 50 | 0.0 | 3.7 | 173377 | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 1.33 | 36 | 0.0 | – | – | – |
| Shades of Gray, enhanced | gradient, photo | 9.71 | 9.55 | – | – | – | 18.3 | 753 | – | 51 | 41 | – | – | – | – | 0.0 | – | 0.5 | 0.0 | – | – | – | – | 0.00 | 53 | 0.0 | – | – | – |
| Shades of Gray, enhanced | linear gradients | 5.25 | 8.40 | – | – | – | 9.0 | 474 | – | 31 | 31 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 63 | 0.0 | 0 | – | – |
| Black and White Logo, enhanced | curved shapes | 0.03 | 1.46 | 0.022 | 0.16 | 0.16 | 1.2 | 16 | 1.67 | 21 | 21 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Black and White Logo, enhanced | polygons | 0.07 | 2.68 | 0.046 | 0.50 | 1.26 | 1.7 | 11 | 1.03 | 99 | 8 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Black and White Logo, enhanced | thin lines | 0.60 | 6.03 | 0.105 | 0.50 | 77.21 | 10.3 | 44 | 2.18 | 90 | 10 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Black and White Logo, enhanced | askew shapes | 0.49 | 11.08 | 0.232 | 1.46 | 9.63 | 4.0 | 25 | 1.03 | 98 | 70 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.67 | 0 | 0.0 | – | – | – |
| Black and White Logo, enhanced | plain shapes | 0.08 | 1.73 | 0.033 | 0.24 | 0.24 | 6.0 | 30 | 1.00 | 67 | 55 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | 100 | 0.00 | 0 | 0.0 | – | – | – |
| Black and White Logo, enhanced | flat colour art | 19.90 | 17.76 | 0.171 | 0.30 | 179.47 | 1.0 | 18 | 1.12 | 23 | 23 | 0 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Black and White Logo, enhanced | gradient, photo | 38.97 | 38.53 | – | – | – | 9.0 | 634 | – | 73 | 54 | – | – | – | – | – | – | 1.2 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Black and White Logo, enhanced | linear gradients | 25.62 | 29.78 | – | – | – | 1.5 | 127 | – | 46 | 33 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | 0 | – | – |
| Sketched Art, enhanced | curved shapes | 0.13 | 5.38 | 0.103 | 0.46 | 0.46 | 1.2 | 11 | 1.24 | 16 | 16 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Sketched Art, enhanced | polygons | 0.13 | 5.02 | 0.090 | 0.49 | 1.08 | 1.7 | 12 | 1.07 | 94 | 5 | 98 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Sketched Art, enhanced | thin lines | 0.47 | 5.72 | 0.114 | 0.96 | 52.59 | 4.3 | 29 | 1.45 | 64 | 3 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.33 | 0 | 0.0 | – | – | – |
| Sketched Art, enhanced | askew shapes | 0.32 | 6.06 | 0.092 | 0.45 | 35.94 | 3.3 | 23 | 0.94 | 96 | 4 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Sketched Art, enhanced | plain shapes | 0.31 | 5.96 | 0.105 | 0.25 | 0.25 | 6.0 | 30 | 1.00 | 67 | 54 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | 100 | 0.00 | 0 | 0.0 | – | – | – |
| Sketched Art, enhanced | flat colour art | 20.24 | 19.33 | 0.158 | 0.83 | 179.59 | 1.5 | 15 | 1.00 | 16 | 16 | 0 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.50 | 25 | 0.0 | – | – | – |
| Sketched Art, enhanced | gradient, photo | 45.42 | 45.21 | – | – | – | 1.0 | 244 | – | 46 | 35 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Sketched Art, enhanced | linear gradients | 29.20 | 33.47 | – | – | – | 1.5 | 54 | – | 35 | 34 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | 0 | – | – |
| Silhouettes, enhanced | curved shapes | 0.35 | 14.33 | 0.277 | 0.69 | 0.69 | 1.2 | 11 | 1.24 | 16 | 16 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Silhouettes, enhanced | polygons | 0.35 | 13.66 | 0.255 | 0.74 | 1.67 | 1.7 | 13 | 1.20 | 82 | 6 | 93 | 0.1 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.11 | 0 | 0.0 | – | – | – |
| Silhouettes, enhanced | thin lines | 0.99 | 13.79 | 0.267 | 1.08 | 18.26 | 5.0 | 30 | 1.50 | 67 | 12 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 1.00 | 0 | 0.0 | – | – | – |
| Silhouettes, enhanced | askew shapes | 0.58 | 14.94 | 0.298 | 0.57 | 0.55 | 4.0 | 24 | 1.01 | 95 | 3 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Silhouettes, enhanced | plain shapes | 0.76 | 15.32 | 0.293 | 0.63 | 0.63 | 6.0 | 30 | 1.00 | 67 | 50 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | 94 | 0.33 | 0 | 0.0 | – | – | – |
| Silhouettes, enhanced | flat colour art | 23.82 | 32.43 | 0.189 | 0.35 | 188.27 | 1.0 | 6 | 0.53 | 37 | 37 | 0 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Silhouettes, enhanced | gradient, photo | 61.57 | 61.32 | – | – | – | 0.2 | 2 | – | 47 | 21 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Silhouettes, enhanced | linear gradients | 39.85 | 46.48 | – | – | – | 1.0 | 37 | – | 1 | 0 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | 0 | – | – |
| Line Art, enhanced | curved shapes | 0.04 | 1.76 | 0.029 | 0.22 | 0.22 | 1.2 | 14 | 1.46 | 20 | 20 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Line Art, enhanced | polygons | 0.07 | 2.38 | 0.040 | 0.50 | 1.26 | 1.7 | 11 | 1.04 | 98 | 5 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Line Art, enhanced | thin lines | 0.52 | 4.12 | 0.040 | 0.47 | 105.72 | 3.3 | 29 | 1.45 | 40 | 0 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Line Art, enhanced | askew shapes | 0.12 | 3.08 | 0.042 | 0.48 | 9.60 | 4.0 | 24 | 1.01 | 98 | 3 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Line Art, enhanced | plain shapes | 0.07 | 1.55 | 0.032 | 0.13 | 0.13 | 6.0 | 30 | 1.00 | 67 | 55 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | 100 | 0.00 | 0 | 0.0 | – | – | – |
| Line Art, enhanced | flat colour art | 19.90 | 17.84 | 0.168 | 0.30 | 179.50 | 1.0 | 16 | 1.03 | 21 | 21 | 0 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Line Art, enhanced | gradient, photo | 38.98 | 38.54 | – | – | – | 7.7 | 472 | – | 58 | 44 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Line Art, enhanced | linear gradients | 25.63 | 29.80 | – | – | – | 1.5 | 95 | – | 33 | 32 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | 0 | – | – |
| Technical Drawing, enhanced | curved shapes | 0.03 | 1.46 | 0.022 | 0.16 | 0.16 | 1.2 | 16 | 1.67 | 21 | 21 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Technical Drawing, enhanced | polygons | 0.07 | 2.68 | 0.046 | 0.50 | 1.26 | 1.7 | 11 | 1.02 | 100 | 8 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Technical Drawing, enhanced | thin lines | 0.50 | 5.50 | 0.110 | 0.50 | 55.93 | 15.3 | 62 | 3.12 | 86 | 14 | – | – | – | – | – | – | 3.3 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Technical Drawing, enhanced | askew shapes | 0.48 | 11.01 | 0.230 | 1.46 | 2.11 | 4.3 | 26 | 1.08 | 98 | 69 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.67 | 0 | 0.0 | – | – | – |
| Technical Drawing, enhanced | plain shapes | 0.08 | 1.81 | 0.035 | 0.24 | 0.24 | 6.0 | 30 | 1.00 | 67 | 55 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | 100 | 0.00 | 0 | 0.0 | – | – | – |
| Technical Drawing, enhanced | flat colour art | 19.90 | 17.76 | 0.171 | 0.30 | 179.47 | 1.0 | 18 | 1.12 | 23 | 23 | 0 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Technical Drawing, enhanced | gradient, photo | 38.94 | 38.51 | – | – | – | 20.0 | 659 | – | 73 | 51 | – | – | – | – | – | – | 11.7 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Technical Drawing, enhanced | linear gradients | 25.62 | 29.80 | – | – | – | 1.5 | 118 | – | 49 | 33 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | 0 | – | – |
| Outline, enhanced | curved shapes | 23.38 | 44.76 | 0.499 | 0.71 | 0.52 | 1.2 | 16 | 1.70 | 20 | 20 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Outline, enhanced | polygons | 17.94 | 44.26 | 0.494 | 0.94 | 0.76 | 1.7 | 14 | 1.26 | 89 | 5 | 95 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Outline, enhanced | thin lines | 0.51 | 3.57 | 0.028 | 0.41 | 105.74 | 3.3 | 7 | 0.33 | 100 | 7 | – | – | – | – | – | – | 0.0 | 0.0 | 0.020 | 0.02 | 2.0 | – | 0.00 | 0 | 0.0 | – | – | – |
| Outline, enhanced | askew shapes | 23.68 | 36.53 | 0.422 | 0.62 | 6.62 | 4.0 | 20 | 0.83 | 100 | 3 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Outline, enhanced | plain shapes | 34.32 | 43.23 | 0.495 | 0.73 | 0.51 | 6.0 | 47 | 1.56 | 56 | 51 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | 0 | 0.00 | 0 | 0.0 | – | – | – |
| Outline, enhanced | flat colour art | 26.69 | 32.18 | 0.494 | 0.92 | 178.93 | 1.0 | 18 | 1.30 | 21 | 21 | 0 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Outline, enhanced | gradient, photo | 39.38 | 39.18 | – | – | – | 7.5 | 445 | – | 59 | 43 | – | – | – | – | – | – | 3.3 | 0.5 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Outline, enhanced | linear gradients | 25.08 | 30.80 | – | – | – | 1.5 | 92 | – | 33 | 32 | – | – | – | – | – | – | 0.0 | 0.5 | – | – | – | – | 0.00 | 0 | 0.0 | 0 | – | – |

## 3. Each preset on its inputs, by variant (256 px)

| preset | variant | ΔE | edge ΔE | off mean | off max | missed max | paths | anchors | × art | lines % | upright % | corners % | kinks | odd colours | odd px | gap px | overlap px | specks | outside px | centre off | width off | anchors a stroke | shapes % | false shapes | grouped % | gradient paths | found % | angle off | ramp off |
|:--|:--|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| Default | clean | 0.07 | 2.96 | 0.052 | 0.47 | 1.26 | 1.9 | 14 | 1.43 | 46 | 10 | 96 | 0.0 | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Default | aliased | 0.37 | 20.23 | 0.233 | 1.02 | 1.35 | 1.9 | 15 | 1.45 | 21 | 8 | 96 | 0.0 | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Default | blurred | 0.81 | 16.55 | 0.077 | 0.98 | 2.18 | 1.9 | 24 | 2.39 | 7 | 5 | 23 | 1.3 | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Default | noisy | 1.74 | 4.55 | 0.056 | 0.46 | 1.25 | 1.9 | 15 | 1.48 | 40 | 0 | 96 | 0.0 | – | – | 1.2 | 0.3 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Default | scanned | 15.06 | 20.16 | 0.118 | 0.74 | 2.14 | 1.9 | 25 | 2.49 | 3 | 0 | 30 | 1.3 | – | – | 1.0 | 3.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Auto Color | clean | 0.13 | 2.72 | 0.057 | 0.31 | 0.30 | 3.0 | 28 | 2.00 | 43 | 19 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Auto Color | aliased | 0.29 | 10.43 | 0.298 | 1.00 | 1.00 | 3.0 | 27 | 2.05 | 43 | 43 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Auto Color | blurred | 0.61 | 9.30 | 0.211 | 1.33 | 1.21 | 6.0 | 118 | 6.51 | 41 | 11 | 50 | 21.0 | 0.0 | 0 | 0.2 | 0.4 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Auto Color | noisy | 0.85 | 2.62 | 0.075 | 0.32 | 0.32 | 3.0 | 28 | 2.04 | 35 | 0 | 100 | 0.0 | 0.0 | 0 | 0.3 | 0.5 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Auto Color | scanned | 1.73 | 4.43 | 0.230 | 1.20 | 0.94 | 7.0 | 102 | 5.72 | 30 | 0 | 88 | 21.0 | 0.0 | 0 | 9.6 | 7.5 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| High Fidelity Photo | clean | 1.80 | 2.05 | – | – | – | 134.7 | 11212 | – | 39 | 37 | – | – | – | – | 0.0 | – | 7.3 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| High Fidelity Photo | aliased | 1.78 | 2.01 | – | – | – | 73.0 | 8021 | – | 40 | 38 | – | – | – | – | 0.0 | – | 5.5 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| High Fidelity Photo | blurred | 1.59 | 1.95 | – | – | – | 72.0 | 7450 | – | 40 | 39 | – | – | – | – | 0.0 | – | 0.5 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| High Fidelity Photo | noisy | 2.02 | 2.19 | – | – | – | 155.5 | 10670 | – | 33 | 31 | – | – | – | – | 0.0 | – | 18.5 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| High Fidelity Photo | scanned | 1.45 | 1.55 | – | – | – | 140.5 | 9723 | – | 36 | 35 | – | – | – | – | 0.0 | – | 24.5 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| Low Fidelity Photo | clean | 2.53 | 2.80 | – | – | – | 42.7 | 2307 | – | 43 | 37 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| Low Fidelity Photo | aliased | 2.53 | 2.78 | – | – | – | 36.5 | 1505 | – | 46 | 38 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| Low Fidelity Photo | blurred | 2.35 | 2.75 | – | – | – | 29.0 | 1990 | – | 44 | 37 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| Low Fidelity Photo | noisy | 2.70 | 2.86 | – | – | – | 32.0 | 1972 | – | 43 | 38 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| Low Fidelity Photo | scanned | 1.95 | 2.05 | – | – | – | 34.5 | 2550 | – | 42 | 33 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| 3 Colors | clean | 0.06 | 2.00 | 0.070 | 0.31 | 0.30 | 2.0 | 11 | 1.78 | 39 | 0 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 3 Colors | aliased | 0.13 | 8.77 | 0.242 | 0.91 | 0.91 | 2.0 | 12 | 2.00 | 39 | 39 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 3 Colors | blurred | 0.34 | 7.83 | 0.073 | 0.24 | 0.24 | 2.0 | 9 | 1.50 | 39 | 0 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 3 Colors | noisy | 0.71 | 2.03 | 0.114 | 0.32 | 0.32 | 2.0 | 11 | 1.83 | 39 | 0 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 3 Colors | scanned | 1.57 | 3.22 | 0.089 | 0.30 | 0.30 | 2.0 | 12 | 2.00 | 39 | 0 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 6 Colors | clean | 0.13 | 2.72 | 0.057 | 0.31 | 0.30 | 3.0 | 28 | 2.00 | 43 | 19 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 6 Colors | aliased | 0.29 | 10.43 | 0.298 | 1.00 | 1.00 | 3.0 | 27 | 2.05 | 43 | 43 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 6 Colors | blurred | 0.62 | 9.31 | 0.208 | 1.34 | 1.19 | 6.0 | 100 | 5.61 | 41 | 11 | 50 | 21.5 | 0.0 | 0 | 0.8 | 0.4 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 6 Colors | noisy | 0.85 | 2.59 | 0.075 | 0.32 | 0.32 | 3.0 | 28 | 2.04 | 35 | 0 | 100 | 0.0 | 0.0 | 0 | 0.3 | 0.3 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 6 Colors | scanned | 1.74 | 4.42 | 0.229 | 1.20 | 0.94 | 7.0 | 102 | 5.80 | 30 | 0 | 88 | 21.0 | 0.0 | 0 | 0.8 | 1.6 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 16 Colors | clean | 0.12 | 2.53 | 0.051 | 0.31 | 0.30 | 3.0 | 28 | 2.11 | 43 | 19 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 16 Colors | aliased | 0.29 | 10.38 | 0.290 | 1.00 | 1.00 | 3.0 | 31 | 2.25 | 43 | 43 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 16 Colors | blurred | 0.62 | 9.36 | 0.216 | 1.31 | 1.19 | 7.0 | 139 | 7.53 | 44 | 13 | 50 | 25.5 | 0.0 | 0 | 0.2 | 0.5 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 16 Colors | noisy | 0.85 | 2.57 | 0.074 | 0.32 | 0.32 | 3.0 | 29 | 2.15 | 34 | 0 | 100 | 0.0 | 0.0 | 0 | 0.2 | 0.2 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| 16 Colors | scanned | 1.72 | 4.44 | 0.257 | 1.09 | 0.96 | 7.5 | 158 | 8.81 | 32 | 0 | 88 | 28.5 | 0.0 | 0 | 3.4 | 3.6 | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Shades of Gray | clean | 9.71 | 9.55 | – | – | – | 18.3 | 753 | – | 51 | 41 | – | – | – | – | 0.0 | – | 0.5 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| Shades of Gray | aliased | 9.71 | 9.52 | – | – | – | 11.0 | 647 | – | 57 | 41 | – | – | – | – | 0.0 | – | 0.5 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| Shades of Gray | blurred | 9.66 | 9.32 | – | – | – | 11.5 | 585 | – | 55 | 41 | – | – | – | – | 0.0 | – | 0.5 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| Shades of Gray | noisy | 9.74 | 9.44 | – | – | – | 19.5 | 648 | – | 46 | 37 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| Shades of Gray | scanned | 6.58 | 6.46 | – | – | – | 19.0 | 764 | – | 47 | 37 | – | – | – | – | 0.0 | – | 1.0 | 0.0 | – | – | – | – | – | – | 0.0 | – | – | – |
| Black and White Logo | clean | 0.04 | 1.86 | 0.031 | 0.47 | 1.26 | 1.4 | 17 | 1.74 | 52 | 15 | 96 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Black and White Logo | aliased | 0.38 | 20.25 | 0.172 | 1.02 | 1.35 | 1.4 | 22 | 2.32 | 53 | 14 | 96 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Black and White Logo | blurred | 0.80 | 16.26 | 0.050 | 0.83 | 2.17 | 1.4 | 39 | 3.84 | 22 | 12 | 23 | 1.1 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Black and White Logo | noisy | 1.73 | 4.15 | 0.038 | 0.46 | 1.25 | 1.4 | 18 | 1.80 | 45 | 7 | 96 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Black and White Logo | scanned | 15.05 | 20.00 | 0.099 | 0.76 | 2.07 | 1.4 | 57 | 5.56 | 17 | 1 | 30 | 1.1 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Sketched Art | clean | 0.20 | 5.71 | 0.111 | 0.96 | 52.59 | 1.9 | 16 | 1.39 | 26 | 14 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Sketched Art | aliased | 0.47 | 19.58 | 0.213 | 1.00 | 105.54 | 1.6 | 11 | 1.10 | 26 | 9 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Sketched Art | blurred | 1.04 | 19.04 | 0.229 | 0.81 | 105.58 | 1.6 | 17 | 1.49 | 13 | 8 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Sketched Art | noisy | 1.76 | 6.07 | 0.113 | 0.97 | 1.06 | 2.0 | 17 | 1.44 | 17 | 0 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Sketched Art | scanned | 15.10 | 22.33 | 0.406 | 0.78 | 105.61 | 1.6 | 21 | 1.74 | 2 | 0 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Silhouettes | clean | 0.35 | 14.53 | 0.281 | 0.69 | 0.69 | 1.2 | 12 | 1.33 | 16 | 16 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Silhouettes | aliased | 0.39 | 21.69 | 0.289 | 1.00 | 1.00 | 1.2 | 9 | 0.99 | 12 | 9 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Silhouettes | blurred | 1.23 | 28.62 | 0.814 | 1.41 | 1.41 | 1.2 | 14 | 1.64 | 13 | 10 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Silhouettes | noisy | 2.12 | 11.10 | 0.285 | 0.68 | 0.69 | 1.2 | 12 | 1.30 | 9 | 0 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Silhouettes | scanned | 16.04 | 30.09 | 1.439 | 2.19 | 2.19 | 1.2 | 28 | 3.56 | 3 | 0 | 50 | 2.8 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Line Art | clean | 0.52 | 4.09 | 0.040 | 0.47 | 105.72 | 3.3 | 29 | 1.45 | 40 | 0 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Line Art | aliased | 0.67 | 14.44 | 0.112 | 0.73 | 55.44 | 14.0 | 59 | 2.95 | 87 | 1 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Line Art | blurred | 1.56 | 16.06 | 0.061 | 0.99 | 105.72 | 3.0 | 53 | 2.65 | 22 | 0 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Line Art | noisy | 1.03 | 4.10 | 0.035 | 0.44 | 56.07 | 8.0 | 50 | 2.50 | 43 | 0 | – | – | – | – | – | – | 1.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Line Art | scanned | 13.54 | 19.55 | 0.062 | 0.92 | 105.68 | 3.0 | 51 | 2.55 | 8 | 0 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Technical Drawing | clean | 0.23 | 4.73 | 0.092 | 1.46 | 55.93 | 4.9 | 26 | 1.58 | 93 | 21 | 95 | 0.0 | – | – | – | – | 0.7 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Technical Drawing | aliased | 0.50 | 19.60 | 0.238 | 1.37 | 55.44 | 6.2 | 31 | 1.85 | 96 | 23 | 94 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Technical Drawing | blurred | 1.06 | 16.51 | 0.130 | 1.16 | 105.64 | 2.4 | 49 | 3.84 | 47 | 13 | 30 | 2.7 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Technical Drawing | noisy | 1.59 | 5.74 | 0.103 | 1.17 | 31.79 | 7.0 | 34 | 1.98 | 93 | 21 | 95 | 0.0 | – | – | – | – | 1.4 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Technical Drawing | scanned | 14.69 | 20.15 | 0.136 | 1.17 | 105.57 | 2.4 | 73 | 5.42 | 49 | 12 | 39 | 3.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Outline | clean | 0.51 | 3.57 | 0.028 | 0.41 | 105.74 | 3.3 | 7 | 0.33 | 100 | 7 | – | – | – | – | – | – | 0.0 | 0.0 | 0.020 | 0.02 | 2.0 | – | 0.00 | – | 0.0 | – | – | – |
| Outline | aliased | 0.74 | 16.02 | 0.164 | 0.81 | 55.55 | 11.0 | 25 | 1.25 | 92 | 9 | – | – | – | – | – | – | 0.0 | 0.0 | 0.158 | 0.01 | 2.3 | – | 0.00 | – | 0.0 | – | – | – |
| Outline | blurred | 1.57 | 16.26 | 0.032 | 0.86 | 105.74 | 3.0 | 6 | 0.30 | 100 | 0 | – | – | – | – | – | – | 0.0 | 0.0 | 0.013 | 0.03 | 2.0 | – | 0.00 | – | 0.0 | – | – | – |
| Outline | noisy | 0.94 | 3.36 | 0.024 | 0.41 | 55.94 | 8.0 | 16 | 0.80 | 100 | 2 | – | – | – | – | – | – | 0.0 | 0.0 | 0.011 | 0.02 | 2.0 | – | 0.00 | – | 0.0 | – | – | – |
| Outline | scanned | 13.53 | 19.37 | 0.045 | 0.88 | 105.64 | 3.0 | 6 | 0.30 | 100 | 0 | – | – | – | – | – | – | 0.0 | 0.0 | 0.027 | 0.03 | 2.0 | – | 0.00 | – | 0.0 | – | – | – |
| Flat Logo | clean | 0.07 | 1.53 | 0.065 | 0.31 | 0.27 | 3.0 | 36 | 3.51 | 47 | 11 | 100 | 0.0 | 0.0 | 0 | 6.8 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Flat Logo | aliased | 0.13 | 5.87 | 0.218 | 0.65 | 0.65 | 3.0 | 31 | 1.96 | 44 | 19 | 50 | 0.0 | 0.0 | 0 | 5.8 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Flat Logo | blurred | 0.54 | 8.33 | 0.052 | 0.62 | 0.78 | 3.0 | 28 | 1.78 | 43 | 16 | 50 | 0.0 | 0.0 | 0 | 5.8 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Flat Logo | noisy | 0.81 | 1.77 | 0.076 | 0.39 | 0.32 | 3.5 | 44 | 2.63 | 31 | 0 | 100 | 1.0 | 0.0 | 0 | 6.9 | – | 0.5 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Flat Logo | scanned | 1.68 | 3.76 | 0.088 | 0.82 | 0.66 | 8.5 | 128 | 10.04 | 23 | 0 | 62 | 17.0 | 0.0 | 0 | 35.1 | – | 5.5 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| Default with Shapes | clean | 0.11 | 2.30 | 0.049 | 0.28 | 0.28 | 6.0 | 30 | 1.00 | 67 | 54 | – | – | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | 100 | 0.00 | – | 0.0 | – | – | – |
| Default with Shapes | aliased | 0.60 | 15.40 | 0.205 | 0.90 | 0.90 | 6.0 | 33 | 1.10 | 55 | 43 | – | – | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | 83 | 0.00 | – | 0.0 | – | – | – |
| Default with Shapes | blurred | 1.57 | 15.87 | 0.074 | 0.39 | 0.39 | 6.0 | 30 | 1.00 | 67 | 54 | – | – | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | 100 | 0.00 | – | 0.0 | – | – | – |
| Default with Shapes | noisy | 2.62 | 4.76 | 0.056 | 0.16 | 0.16 | 6.0 | 30 | 1.00 | 67 | 0 | – | – | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | 100 | 0.00 | – | 0.0 | – | – | – |
| Default with Shapes | scanned | 17.00 | 21.65 | 0.112 | 0.24 | 0.23 | 6.0 | 30 | 1.00 | 67 | 17 | – | – | – | – | 3.3 | 0.3 | 0.0 | 0.0 | – | – | – | 100 | 0.00 | – | 0.0 | – | – | – |
| 6 Colors with Shapes | clean | 0.09 | 1.88 | 0.039 | 0.29 | 0.29 | 6.0 | 30 | 1.00 | 67 | 54 | – | – | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | 100 | 0.00 | – | 0.0 | – | – | – |
| 6 Colors with Shapes | aliased | 0.59 | 15.23 | 0.180 | 0.60 | 0.60 | 6.0 | 30 | 1.00 | 68 | 55 | – | – | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | 100 | 0.00 | – | 0.0 | – | – | – |
| 6 Colors with Shapes | blurred | 1.56 | 15.56 | 0.118 | 0.23 | 0.23 | 6.0 | 30 | 1.00 | 67 | 55 | – | – | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | 100 | 0.00 | – | 0.0 | – | – | – |
| 6 Colors with Shapes | noisy | 1.78 | 3.31 | 0.041 | 0.11 | 0.11 | 6.0 | 30 | 1.00 | 67 | 0 | – | – | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | 100 | 0.00 | – | 0.0 | – | – | – |
| 6 Colors with Shapes | scanned | 2.10 | 6.74 | 0.085 | 0.30 | 0.24 | 6.0 | 30 | 1.00 | 67 | 0 | – | – | 0.0 | 0 | 14.7 | 8.7 | 0.0 | 0.0 | – | – | – | 100 | 0.00 | – | 0.0 | – | – | – |
| 6 Colors with Auto Grouping | clean | 0.13 | 2.72 | 0.057 | 0.31 | 0.30 | 3.0 | 28 | 2.00 | 43 | 19 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 30 | 0.0 | – | – | – |
| 6 Colors with Auto Grouping | aliased | 0.29 | 10.43 | 0.298 | 1.00 | 1.00 | 3.0 | 27 | 2.05 | 43 | 43 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 30 | 0.0 | – | – | – |
| 6 Colors with Auto Grouping | blurred | 0.62 | 9.31 | 0.208 | 1.34 | 1.19 | 6.0 | 100 | 5.61 | 41 | 11 | 50 | 21.5 | 0.0 | 0 | 0.8 | 0.4 | 0.0 | 0.0 | – | – | – | – | 0.00 | 23 | 0.0 | – | – | – |
| 6 Colors with Auto Grouping | noisy | 0.85 | 2.59 | 0.075 | 0.32 | 0.32 | 3.0 | 28 | 2.04 | 35 | 0 | 100 | 0.0 | 0.0 | 0 | 0.3 | 0.3 | 0.0 | 0.0 | – | – | – | – | 0.00 | 30 | 0.0 | – | – | – |
| 6 Colors with Auto Grouping | scanned | 1.74 | 4.42 | 0.229 | 1.20 | 0.94 | 7.0 | 102 | 5.80 | 30 | 0 | 88 | 21.0 | 0.0 | 0 | 0.8 | 1.6 | 0.0 | 0.0 | – | – | – | – | 0.00 | 15 | 0.0 | – | – | – |
| Shades of Gray with Auto Grouping | clean | 9.71 | 9.55 | – | – | – | 18.3 | 753 | – | 51 | 41 | – | – | – | – | 0.0 | – | 0.5 | 0.0 | – | – | – | – | – | 53 | 0.0 | – | – | – |
| Shades of Gray with Auto Grouping | aliased | 9.71 | 9.52 | – | – | – | 11.0 | 647 | – | 57 | 41 | – | – | – | – | 0.0 | – | 0.5 | 0.0 | – | – | – | – | – | 54 | 0.0 | – | – | – |
| Shades of Gray with Auto Grouping | blurred | 9.66 | 9.32 | – | – | – | 11.5 | 585 | – | 55 | 41 | – | – | – | – | 0.0 | – | 0.5 | 0.0 | – | – | – | – | – | 47 | 0.0 | – | – | – |
| Shades of Gray with Auto Grouping | noisy | 9.74 | 9.44 | – | – | – | 19.5 | 648 | – | 46 | 37 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | – | 28 | 0.0 | – | – | – |
| Shades of Gray with Auto Grouping | scanned | 6.58 | 6.46 | – | – | – | 19.0 | 764 | – | 47 | 37 | – | – | – | – | 0.0 | – | 1.0 | 0.0 | – | – | – | – | – | 37 | 0.0 | – | – | – |
| High Fidelity Photo with Auto Grouping | clean | 1.80 | 2.05 | – | – | – | 134.7 | 11212 | – | 39 | 37 | – | – | – | – | 0.0 | – | 7.3 | 0.0 | – | – | – | – | – | 69 | 0.0 | – | – | – |
| High Fidelity Photo with Auto Grouping | aliased | 1.78 | 2.01 | – | – | – | 73.0 | 8021 | – | 40 | 38 | – | – | – | – | 0.0 | – | 5.5 | 0.0 | – | – | – | – | – | 65 | 0.0 | – | – | – |
| High Fidelity Photo with Auto Grouping | blurred | 1.59 | 1.95 | – | – | – | 72.0 | 7450 | – | 40 | 39 | – | – | – | – | 0.0 | – | 0.5 | 0.0 | – | – | – | – | – | 65 | 0.0 | – | – | – |
| High Fidelity Photo with Auto Grouping | noisy | 2.02 | 2.19 | – | – | – | 155.5 | 10670 | – | 33 | 31 | – | – | – | – | 0.0 | – | 18.5 | 0.0 | – | – | – | – | – | 72 | 0.0 | – | – | – |
| High Fidelity Photo with Auto Grouping | scanned | 1.45 | 1.55 | – | – | – | 140.5 | 9723 | – | 36 | 35 | – | – | – | – | 0.0 | – | 24.5 | 0.0 | – | – | – | – | – | 75 | 0.0 | – | – | – |
| 6 Colors, no Gradients | clean | 2.07 | 3.07 | – | – | – | 6.2 | 266 | – | 20 | 14 | – | – | – | – | 2.4 | 9.3 | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| 6 Colors, no Gradients | aliased | 2.04 | 3.07 | – | – | – | 6.5 | 500 | – | 22 | 14 | – | – | – | – | 0.0 | 0.6 | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| 6 Colors, no Gradients | blurred | 2.08 | 3.69 | – | – | – | 6.0 | 344 | – | 23 | 14 | – | – | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| 6 Colors, no Gradients | noisy | 2.36 | 2.87 | – | – | – | 8.0 | 864 | – | 19 | 12 | – | – | – | – | 10.5 | 30.5 | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| 6 Colors, no Gradients | scanned | 2.16 | 2.41 | – | – | – | 6.0 | 776 | – | 20 | 12 | – | – | – | – | 9.8 | 30.4 | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| 6 Colors with Gradients | clean | 0.40 | 1.09 | – | – | – | 1.3 | 39 | – | 0 | 0 | – | – | – | – | 0.5 | 1.1 | 0.0 | 0.0 | – | – | – | – | – | – | 0.8 | 83 | 0.2 | 5 |
| 6 Colors with Gradients | aliased | 0.22 | 0.77 | – | – | – | 0.5 | 6 | – | 0 | 0 | – | – | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | – | – | 1.0 | 100 | 0.2 | 5 |
| 6 Colors with Gradients | blurred | 0.33 | 2.23 | – | – | – | 0.5 | 24 | – | 0 | 0 | – | – | – | – | 7.1 | 3.7 | 0.0 | 0.0 | – | – | – | – | – | – | 1.0 | 100 | 0.3 | 6 |
| 6 Colors with Gradients | noisy | 1.02 | 1.27 | – | – | – | 0.5 | 14 | – | 0 | 0 | – | – | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | – | – | 1.0 | 100 | 0.2 | 8 |
| 6 Colors with Gradients | scanned | 1.57 | 1.82 | – | – | – | 3.0 | 264 | – | 3 | 0 | – | – | – | – | 2.8 | 1.6 | 0.0 | 0.0 | – | – | – | – | – | – | 0.5 | 50 | 1.5 | 21 |
| High Fidelity Photo, no Gradients | clean | 0.64 | 3.29 | – | – | – | 34.5 | 4318 | – | 32 | 31 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| High Fidelity Photo, no Gradients | aliased | 0.67 | 3.46 | – | – | – | 37.5 | 3184 | – | 32 | 31 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| High Fidelity Photo, no Gradients | blurred | 0.72 | 3.25 | – | – | – | 33.5 | 3324 | – | 32 | 31 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| High Fidelity Photo, no Gradients | noisy | 1.15 | 2.67 | – | – | – | 37.5 | 3468 | – | 29 | 28 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| High Fidelity Photo, no Gradients | scanned | 1.05 | 1.62 | – | – | – | 54.5 | 3858 | – | 29 | 28 | – | – | – | – | 0.0 | – | 7.5 | 0.0 | – | – | – | – | – | – | 0.0 | 0 | – | – |
| High Fidelity Photo with Gradients | clean | 0.23 | 1.19 | – | – | – | 6.0 | 252 | – | 0 | 0 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | – | – | 0.8 | 83 | 0.2 | 5 |
| High Fidelity Photo with Gradients | aliased | 0.21 | 0.52 | – | – | – | 0.5 | 24 | – | 0 | 0 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | – | – | 1.0 | 100 | 0.2 | 5 |
| High Fidelity Photo with Gradients | blurred | 0.33 | 2.20 | – | – | – | 0.5 | 48 | – | 1 | 0 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | – | – | 1.0 | 100 | 0.3 | 6 |
| High Fidelity Photo with Gradients | noisy | 1.02 | 1.21 | – | – | – | 1.5 | 116 | – | 0 | 0 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | – | – | 1.0 | 100 | 0.2 | 8 |
| High Fidelity Photo with Gradients | scanned | 0.97 | 1.55 | – | – | – | 37.0 | 2868 | – | 5 | 3 | – | – | – | – | 0.0 | – | 5.5 | 0.0 | – | – | – | – | – | – | 0.5 | 50 | 1.5 | 21 |
| Default, enhanced | clean | 0.08 | 2.94 | 0.051 | 0.50 | 1.26 | 1.9 | 13 | 1.32 | 50 | 10 | 100 | 0.0 | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 14 | 0.0 | – | – | – |
| Default, enhanced | aliased | 0.36 | 19.23 | 0.194 | 1.00 | 1.35 | 1.9 | 14 | 1.42 | 28 | 8 | 98 | 0.0 | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.14 | 14 | 0.0 | – | – | – |
| Default, enhanced | blurred | 0.81 | 16.50 | 0.073 | 0.98 | 2.18 | 1.9 | 22 | 2.15 | 17 | 5 | 38 | 1.3 | – | – | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 14 | 0.0 | – | – | – |
| Default, enhanced | noisy | 1.74 | 4.45 | 0.053 | 0.46 | 1.25 | 1.9 | 13 | 1.36 | 44 | 0 | 100 | 0.0 | – | – | 1.2 | 0.3 | 0.0 | 0.0 | – | – | – | – | 0.00 | 14 | 0.0 | – | – | – |
| Default, enhanced | scanned | 15.06 | 20.14 | 0.111 | 0.74 | 2.14 | 1.9 | 22 | 2.23 | 13 | 0 | 43 | 1.3 | – | – | 1.0 | 3.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 14 | 0.0 | – | – | – |
| Auto Color, enhanced | clean | 0.13 | 2.72 | 0.057 | 0.31 | 0.30 | 3.0 | 28 | 2.00 | 43 | 19 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 30 | 0.0 | – | – | – |
| Auto Color, enhanced | aliased | 0.29 | 10.42 | 0.293 | 1.00 | 1.00 | 3.0 | 29 | 2.15 | 43 | 43 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 30 | 0.0 | – | – | – |
| Auto Color, enhanced | blurred | 0.62 | 9.36 | 0.208 | 1.33 | 0.72 | 6.0 | 112 | 6.21 | 46 | 16 | 100 | 21.0 | 0.0 | 0 | 0.2 | 0.4 | 0.0 | 0.0 | – | – | – | – | 0.00 | 23 | 0.0 | – | – | – |
| Auto Color, enhanced | noisy | 0.85 | 2.62 | 0.073 | 0.32 | 0.32 | 3.0 | 27 | 1.99 | 35 | 0 | 100 | 0.0 | 0.0 | 0 | 0.3 | 0.5 | 0.0 | 0.0 | – | – | – | – | 0.00 | 30 | 0.0 | – | – | – |
| Auto Color, enhanced | scanned | 1.73 | 4.43 | 0.229 | 1.20 | 0.68 | 7.0 | 84 | 4.87 | 39 | 0 | 100 | 21.0 | 0.0 | 0 | 8.9 | 6.9 | 0.0 | 0.0 | – | – | – | – | 0.00 | 15 | 0.0 | – | – | – |
| High Fidelity Photo, enhanced | clean | 1.80 | 2.05 | – | – | – | 134.7 | 11212 | – | 39 | 37 | – | – | – | – | 0.0 | – | 7.3 | 0.0 | – | – | – | – | – | 69 | 0.0 | – | – | – |
| High Fidelity Photo, enhanced | aliased | 1.78 | 2.01 | – | – | – | 73.0 | 8021 | – | 40 | 38 | – | – | – | – | 0.0 | – | 5.5 | 0.0 | – | – | – | – | – | 65 | 0.0 | – | – | – |
| High Fidelity Photo, enhanced | blurred | 1.59 | 1.95 | – | – | – | 72.0 | 7450 | – | 40 | 39 | – | – | – | – | 0.0 | – | 0.5 | 0.0 | – | – | – | – | – | 65 | 0.0 | – | – | – |
| High Fidelity Photo, enhanced | noisy | 2.02 | 2.19 | – | – | – | 155.5 | 10670 | – | 33 | 31 | – | – | – | – | 0.0 | – | 18.5 | 0.0 | – | – | – | – | – | 72 | 0.0 | – | – | – |
| High Fidelity Photo, enhanced | scanned | 1.45 | 1.55 | – | – | – | 140.5 | 9723 | – | 36 | 35 | – | – | – | – | 0.0 | – | 24.5 | 0.0 | – | – | – | – | – | 75 | 0.0 | – | – | – |
| Low Fidelity Photo, enhanced | clean | 2.53 | 2.80 | – | – | – | 42.7 | 2307 | – | 43 | 37 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 60 | 0.0 | – | – | – |
| Low Fidelity Photo, enhanced | aliased | 2.53 | 2.78 | – | – | – | 36.5 | 1505 | – | 46 | 38 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 68 | 0.0 | – | – | – |
| Low Fidelity Photo, enhanced | blurred | 2.35 | 2.75 | – | – | – | 29.0 | 1990 | – | 44 | 37 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 74 | 0.0 | – | – | – |
| Low Fidelity Photo, enhanced | noisy | 2.70 | 2.86 | – | – | – | 32.0 | 1972 | – | 43 | 38 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 57 | 0.0 | – | – | – |
| Low Fidelity Photo, enhanced | scanned | 1.95 | 2.05 | – | – | – | 34.5 | 2550 | – | 42 | 33 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 61 | 0.0 | – | – | – |
| 3 Colors, enhanced | clean | 0.06 | 2.00 | 0.070 | 0.31 | 0.30 | 2.0 | 11 | 1.78 | 39 | 0 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| 3 Colors, enhanced | aliased | 0.13 | 8.77 | 0.242 | 0.91 | 0.91 | 2.0 | 12 | 2.00 | 39 | 39 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| 3 Colors, enhanced | blurred | 0.34 | 7.83 | 0.073 | 0.24 | 0.24 | 2.0 | 9 | 1.50 | 39 | 0 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| 3 Colors, enhanced | noisy | 0.71 | 2.03 | 0.114 | 0.32 | 0.32 | 2.0 | 11 | 1.83 | 39 | 0 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| 3 Colors, enhanced | scanned | 1.57 | 3.22 | 0.089 | 0.30 | 0.30 | 2.0 | 12 | 2.00 | 39 | 0 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| 6 Colors, enhanced | clean | 0.13 | 2.72 | 0.057 | 0.31 | 0.30 | 3.0 | 28 | 2.00 | 43 | 19 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 30 | 0.0 | – | – | – |
| 6 Colors, enhanced | aliased | 0.29 | 10.42 | 0.293 | 1.00 | 1.00 | 3.0 | 29 | 2.15 | 43 | 43 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 30 | 0.0 | – | – | – |
| 6 Colors, enhanced | blurred | 0.62 | 9.33 | 0.205 | 1.34 | 0.68 | 6.0 | 94 | 5.31 | 46 | 16 | 100 | 21.5 | 0.0 | 0 | 0.8 | 0.4 | 0.0 | 0.0 | – | – | – | – | 0.00 | 23 | 0.0 | – | – | – |
| 6 Colors, enhanced | noisy | 0.85 | 2.59 | 0.073 | 0.32 | 0.32 | 3.0 | 27 | 1.99 | 35 | 0 | 100 | 0.0 | 0.0 | 0 | 0.3 | 0.3 | 0.0 | 0.0 | – | – | – | – | 0.00 | 30 | 0.0 | – | – | – |
| 6 Colors, enhanced | scanned | 1.74 | 4.42 | 0.228 | 1.20 | 0.68 | 7.0 | 85 | 4.95 | 39 | 0 | 100 | 21.0 | 0.0 | 0 | 0.0 | 1.1 | 0.0 | 0.0 | – | – | – | – | 0.00 | 15 | 0.0 | – | – | – |
| 16 Colors, enhanced | clean | 0.12 | 2.53 | 0.052 | 0.31 | 0.30 | 3.0 | 28 | 2.11 | 43 | 19 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 30 | 0.0 | – | – | – |
| 16 Colors, enhanced | aliased | 0.29 | 10.38 | 0.289 | 1.00 | 1.00 | 3.0 | 30 | 2.20 | 43 | 43 | 100 | 0.0 | 0.0 | 0 | 0.0 | 0.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 30 | 0.0 | – | – | – |
| 16 Colors, enhanced | blurred | 0.62 | 9.37 | 0.213 | 1.31 | 0.74 | 7.0 | 133 | 7.23 | 48 | 17 | 100 | 25.5 | 0.0 | 0 | 0.2 | 0.5 | 0.0 | 0.0 | – | – | – | – | 0.00 | 27 | 0.0 | – | – | – |
| 16 Colors, enhanced | noisy | 0.85 | 2.57 | 0.072 | 0.32 | 0.32 | 3.0 | 28 | 2.10 | 34 | 0 | 100 | 0.0 | 0.0 | 0 | 0.2 | 0.2 | 0.0 | 0.0 | – | – | – | – | 0.00 | 30 | 0.0 | – | – | – |
| 16 Colors, enhanced | scanned | 1.72 | 4.44 | 0.257 | 1.09 | 0.80 | 7.5 | 140 | 7.91 | 41 | 0 | 100 | 28.5 | 0.0 | 0 | 2.6 | 3.0 | 0.0 | 0.0 | – | – | – | – | 0.00 | 18 | 0.0 | – | – | – |
| Shades of Gray, enhanced | clean | 9.71 | 9.55 | – | – | – | 18.3 | 753 | – | 51 | 41 | – | – | – | – | 0.0 | – | 0.5 | 0.0 | – | – | – | – | 0.00 | 53 | 0.0 | – | – | – |
| Shades of Gray, enhanced | aliased | 9.71 | 9.52 | – | – | – | 11.0 | 647 | – | 57 | 41 | – | – | – | – | 0.0 | – | 0.5 | 0.0 | – | – | – | – | 0.00 | 54 | 0.0 | – | – | – |
| Shades of Gray, enhanced | blurred | 9.66 | 9.32 | – | – | – | 11.5 | 585 | – | 55 | 41 | – | – | – | – | 0.0 | – | 0.5 | 0.0 | – | – | – | – | 0.00 | 47 | 0.0 | – | – | – |
| Shades of Gray, enhanced | noisy | 9.74 | 9.44 | – | – | – | 19.5 | 648 | – | 46 | 37 | – | – | – | – | 0.0 | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 28 | 0.0 | – | – | – |
| Shades of Gray, enhanced | scanned | 6.58 | 6.46 | – | – | – | 19.0 | 764 | – | 47 | 37 | – | – | – | – | 0.0 | – | 1.0 | 0.0 | – | – | – | – | 0.00 | 37 | 0.0 | – | – | – |
| Black and White Logo, enhanced | clean | 0.05 | 1.98 | 0.032 | 0.50 | 1.26 | 1.4 | 14 | 1.39 | 55 | 15 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Black and White Logo, enhanced | aliased | 0.38 | 20.07 | 0.166 | 1.00 | 1.35 | 1.4 | 20 | 2.13 | 54 | 14 | 98 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.14 | 0 | 0.0 | – | – | – |
| Black and White Logo, enhanced | blurred | 0.80 | 16.27 | 0.056 | 0.83 | 2.17 | 1.4 | 31 | 3.07 | 30 | 12 | 38 | 1.1 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Black and White Logo, enhanced | noisy | 1.73 | 4.16 | 0.038 | 0.46 | 1.25 | 1.4 | 14 | 1.41 | 47 | 7 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Black and White Logo, enhanced | scanned | 15.05 | 19.98 | 0.097 | 0.76 | 2.07 | 1.4 | 43 | 4.19 | 24 | 1 | 43 | 1.1 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Sketched Art, enhanced | clean | 0.20 | 5.45 | 0.105 | 0.96 | 52.59 | 1.9 | 15 | 1.28 | 26 | 14 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.07 | 0 | 0.0 | – | – | – |
| Sketched Art, enhanced | aliased | 0.45 | 19.03 | 0.194 | 1.00 | 105.54 | 1.6 | 11 | 1.06 | 26 | 9 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Sketched Art, enhanced | blurred | 1.03 | 18.97 | 0.226 | 0.81 | 105.58 | 1.6 | 16 | 1.39 | 13 | 8 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Sketched Art, enhanced | noisy | 1.75 | 5.97 | 0.108 | 0.97 | 1.06 | 2.0 | 16 | 1.34 | 17 | 0 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Sketched Art, enhanced | scanned | 15.10 | 22.30 | 0.403 | 0.78 | 105.61 | 1.6 | 20 | 1.64 | 2 | 0 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Silhouettes, enhanced | clean | 0.35 | 14.33 | 0.277 | 0.69 | 0.69 | 1.2 | 11 | 1.24 | 16 | 16 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Silhouettes, enhanced | aliased | 0.37 | 20.27 | 0.243 | 1.00 | 1.00 | 1.2 | 10 | 1.32 | 12 | 9 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Silhouettes, enhanced | blurred | 1.23 | 28.51 | 0.809 | 1.41 | 1.41 | 1.2 | 12 | 1.34 | 13 | 10 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.50 | 0 | 0.0 | – | – | – |
| Silhouettes, enhanced | noisy | 2.11 | 11.00 | 0.281 | 0.68 | 0.69 | 1.2 | 11 | 1.24 | 9 | 0 | 100 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Silhouettes, enhanced | scanned | 16.04 | 30.10 | 1.440 | 2.19 | 2.19 | 1.2 | 26 | 3.36 | 3 | 0 | 50 | 2.8 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.25 | 0 | 0.0 | – | – | – |
| Line Art, enhanced | clean | 0.52 | 4.12 | 0.040 | 0.47 | 105.72 | 3.3 | 29 | 1.45 | 40 | 0 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Line Art, enhanced | aliased | 0.67 | 14.44 | 0.112 | 0.73 | 55.44 | 14.0 | 59 | 2.95 | 87 | 1 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Line Art, enhanced | blurred | 1.56 | 16.08 | 0.057 | 0.99 | 105.72 | 3.0 | 49 | 2.45 | 38 | 0 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Line Art, enhanced | noisy | 1.03 | 4.10 | 0.035 | 0.44 | 56.07 | 8.0 | 50 | 2.50 | 43 | 0 | – | – | – | – | – | – | 1.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Line Art, enhanced | scanned | 13.54 | 19.55 | 0.062 | 0.92 | 105.68 | 3.0 | 51 | 2.55 | 8 | 0 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Technical Drawing, enhanced | clean | 0.24 | 4.91 | 0.095 | 1.46 | 55.93 | 4.9 | 24 | 1.45 | 97 | 21 | 100 | 0.0 | – | – | – | – | 0.7 | 0.0 | – | – | – | – | 0.13 | 0 | 0.0 | – | – | – |
| Technical Drawing, enhanced | aliased | 0.50 | 19.31 | 0.229 | 1.37 | 55.44 | 6.2 | 31 | 1.79 | 98 | 23 | 97 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.20 | 0 | 0.0 | – | – | – |
| Technical Drawing, enhanced | blurred | 1.06 | 16.55 | 0.138 | 1.16 | 105.64 | 2.4 | 41 | 3.21 | 60 | 13 | 51 | 2.7 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Technical Drawing, enhanced | noisy | 1.59 | 5.75 | 0.103 | 1.17 | 31.79 | 7.0 | 32 | 1.86 | 97 | 21 | 100 | 0.0 | – | – | – | – | 1.4 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Technical Drawing, enhanced | scanned | 14.69 | 20.14 | 0.138 | 1.17 | 105.57 | 2.4 | 63 | 4.62 | 59 | 12 | 57 | 3.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | 0 | 0.0 | – | – | – |
| Outline, enhanced | clean | 0.51 | 3.57 | 0.028 | 0.41 | 105.74 | 3.3 | 7 | 0.33 | 100 | 7 | – | – | – | – | – | – | 0.0 | 0.0 | 0.020 | 0.02 | 2.0 | – | 0.00 | 0 | 0.0 | – | – | – |
| Outline, enhanced | aliased | 0.74 | 16.02 | 0.164 | 0.81 | 55.55 | 11.0 | 25 | 1.25 | 92 | 9 | – | – | – | – | – | – | 0.0 | 0.0 | 0.158 | 0.01 | 2.3 | – | 0.00 | 0 | 0.0 | – | – | – |
| Outline, enhanced | blurred | 1.57 | 16.26 | 0.032 | 0.86 | 105.74 | 3.0 | 6 | 0.30 | 100 | 0 | – | – | – | – | – | – | 0.0 | 0.0 | 0.013 | 0.03 | 2.0 | – | 0.00 | 0 | 0.0 | – | – | – |
| Outline, enhanced | noisy | 0.94 | 3.36 | 0.024 | 0.41 | 55.94 | 8.0 | 16 | 0.80 | 100 | 2 | – | – | – | – | – | – | 0.0 | 0.0 | 0.011 | 0.02 | 2.0 | – | 0.00 | 0 | 0.0 | – | – | – |
| Outline, enhanced | scanned | 13.53 | 19.37 | 0.045 | 0.88 | 105.64 | 3.0 | 6 | 0.30 | 100 | 0 | – | – | – | – | – | – | 0.0 | 0.0 | 0.027 | 0.03 | 2.0 | – | 0.00 | 0 | 0.0 | – | – | – |

## 4. Yardstick

Inkscape 1.4.2's Trace Bitmap, run as a black box through its `object-trace` action (two scans, background
removed, its other settings as they come) on the same one-colour drawings, and its SVG output measured by
the same code. Compare with the black-and-white presets on the same kinds of input in section 2 and 3.

| inputs | variant | ΔE | edge ΔE | off mean | off max | missed max | paths | anchors | × art | lines % | upright % | corners % | kinks | odd colours | odd px | gap px | overlap px | specks | outside px | centre off | width off | anchors a stroke | shapes % | false shapes | grouped % | gradient paths | found % | angle off | ramp off |
|:--|:--|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| curved shapes | clean | 0.25 | 11.05 | 0.198 | 0.97 | 0.97 | 1.0 | 15 | 1.98 | 11 | 7 | 83 | 1.8 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| curved shapes | aliased | 0.38 | 20.79 | 0.192 | 0.88 | 0.88 | 1.0 | 16 | 1.88 | 14 | 8 | 50 | 1.8 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| curved shapes | blurred | 0.79 | 16.57 | 0.182 | 0.86 | 1.14 | 1.0 | 15 | 1.84 | 6 | 5 | 0 | 1.8 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| curved shapes | noisy | 2.01 | 8.14 | 0.191 | 0.93 | 0.93 | 1.0 | 17 | 2.06 | 10 | 0 | 75 | 2.8 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| curved shapes | scanned | 15.31 | 20.22 | 0.191 | 0.97 | 1.14 | 1.0 | 16 | 1.98 | 7 | 2 | 0 | 2.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| polygons | clean | 0.16 | 6.45 | 0.124 | 1.26 | 2.36 | 1.0 | 21 | 1.93 | 27 | 4 | 34 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| polygons | aliased | 0.35 | 18.33 | 0.150 | 1.22 | 2.35 | 1.0 | 20 | 1.81 | 20 | 3 | 28 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| polygons | blurred | 0.90 | 17.54 | 0.190 | 1.20 | 2.56 | 1.0 | 19 | 1.75 | 0 | 0 | 0 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| polygons | noisy | 1.62 | 6.05 | 0.119 | 1.22 | 2.32 | 1.0 | 22 | 2.01 | 29 | 4 | 19 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| polygons | scanned | 14.75 | 20.30 | 0.170 | 1.49 | 2.55 | 1.0 | 21 | 1.90 | 4 | 0 | 3 | 0.0 | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| thin lines | clean | 0.44 | 6.05 | 0.109 | 0.98 | 76.81 | 1.0 | 34 | 1.72 | 2 | 0 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| thin lines | aliased | 0.74 | 16.37 | 0.099 | 1.17 | 55.97 | 1.0 | 22 | 1.10 | 6 | 0 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| thin lines | blurred | 1.72 | 18.80 | 0.254 | 0.81 | 106.00 | 1.0 | 18 | 0.90 | 0 | 0 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| thin lines | noisy | 1.02 | 6.25 | 0.112 | 1.00 | 37.89 | 1.0 | 25 | 1.25 | 0 | 0 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| thin lines | scanned | 28.94 | 46.83 | 12.147 | 43.11 | 20.79 | 1.0 | 2763 | 138.15 | 2 | 0 | – | – | – | – | – | – | 0.0 | 0.1 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| askew shapes | clean | 0.31 | 7.76 | 0.142 | 1.08 | 3.38 | 1.0 | 45 | 1.89 | 32 | 15 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| askew shapes | aliased | 0.45 | 14.06 | 0.149 | 1.14 | 1.35 | 1.0 | 47 | 1.96 | 30 | 4 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| askew shapes | blurred | 1.19 | 17.05 | 0.195 | 1.20 | 3.65 | 1.0 | 44 | 1.83 | 29 | 11 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| askew shapes | noisy | 2.03 | 6.74 | 0.116 | 0.85 | 1.04 | 1.0 | 62 | 2.58 | 37 | 7 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| askew shapes | scanned | 15.69 | 20.78 | 0.217 | 1.47 | 3.65 | 1.0 | 44 | 1.83 | 26 | 8 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | – | 0.00 | – | 0.0 | – | – | – |
| plain shapes | clean | 0.56 | 11.27 | 0.218 | 1.00 | 1.02 | 1.0 | 46 | 1.54 | 46 | 36 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | 0 | 0.00 | – | 0.0 | – | – | – |
| plain shapes | aliased | 0.57 | 14.69 | 0.239 | 1.00 | 1.00 | 1.0 | 49 | 1.63 | 47 | 25 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | 0 | 0.00 | – | 0.0 | – | – | – |
| plain shapes | blurred | 1.68 | 17.28 | 0.262 | 1.14 | 1.14 | 1.0 | 45 | 1.50 | 31 | 8 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | 0 | 0.00 | – | 0.0 | – | – | – |
| plain shapes | noisy | 2.94 | 9.59 | 0.234 | 0.98 | 0.98 | 1.0 | 53 | 1.77 | 50 | 25 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | 0 | 0.00 | – | 0.0 | – | – | – |
| plain shapes | scanned | 17.09 | 22.56 | 0.281 | 0.99 | 1.10 | 1.0 | 51 | 1.70 | 31 | 8 | – | – | – | – | – | – | 0.0 | 0.0 | – | – | – | 0 | 0.00 | – | 0.0 | – | – | – |

## 5. Cost

One trace at a time on this machine (all cores); memory is the peak allocated above the image itself. A trace of more than 2,000,000 anchors is refused, as in the app.

| preset | image | megapixels | result |
|:--|:--|--:|:--|
| Default | photo-like | 1 | 0.04 s, 29 paths, 6278 anchors, 12 MB |
| Default | four-colour badge | 1 | 0.02 s, 3 paths, 64 anchors, 12 MB |
| Auto Color | photo-like | 1 | 0.17 s, 732 paths, 45265 anchors, 24 MB |
| Auto Color | four-colour badge | 1 | 0.05 s, 5 paths, 84 anchors, 12 MB |
| High Fidelity Photo | photo-like | 1 | 0.22 s, 968 paths, 117041 anchors, 171 MB |
| High Fidelity Photo | four-colour badge | 1 | 0.06 s, 6 paths, 172 anchors, 23 MB |
| Low Fidelity Photo | photo-like | 1 | 0.12 s, 129 paths, 23014 anchors, 105 MB |
| Low Fidelity Photo | four-colour badge | 1 | 0.05 s, 6 paths, 136 anchors, 23 MB |
| 3 Colors | photo-like | 1 | 0.09 s, 9 paths, 3341 anchors, 18 MB |
| 3 Colors | four-colour badge | 1 | 0.05 s, 4 paths, 88 anchors, 13 MB |
| 6 Colors | photo-like | 1 | 0.10 s, 78 paths, 9076 anchors, 24 MB |
| 6 Colors | four-colour badge | 1 | 0.05 s, 5 paths, 84 anchors, 12 MB |
| 16 Colors | photo-like | 1 | 0.11 s, 178 paths, 20470 anchors, 26 MB |
| 16 Colors | four-colour badge | 1 | 0.05 s, 5 paths, 88 anchors, 12 MB |
| Shades of Gray | photo-like | 1 | 0.10 s, 81 paths, 6698 anchors, 41 MB |
| Shades of Gray | four-colour badge | 1 | 0.06 s, 10 paths, 725 anchors, 34 MB |
| Black and White Logo | photo-like | 1 | 0.04 s, 71 paths, 4933 anchors, 8 MB |
| Black and White Logo | four-colour badge | 1 | 0.02 s, 1 paths, 47 anchors, 6 MB |
| Sketched Art | photo-like | 1 | 0.04 s, 5 paths, 2873 anchors, 8 MB |
| Sketched Art | four-colour badge | 1 | 0.02 s, 2 paths, 114 anchors, 6 MB |
| Silhouettes | photo-like | 1 | 0.03 s, 2 paths, 556 anchors, 9 MB |
| Silhouettes | four-colour badge | 1 | 0.02 s, 1 paths, 24 anchors, 6 MB |
| Line Art | photo-like | 1 | 0.04 s, 54 paths, 4495 anchors, 8 MB |
| Line Art | four-colour badge | 1 | 0.02 s, 1 paths, 38 anchors, 6 MB |
| Technical Drawing | photo-like | 1 | 0.04 s, 221 paths, 6258 anchors, 8 MB |
| Technical Drawing | four-colour badge | 1 | 0.02 s, 1 paths, 47 anchors, 6 MB |
| Outline | photo-like | 1 | 0.05 s, 45 paths, 4110 anchors, 16 MB |
| Outline | four-colour badge | 1 | 0.03 s, 1 paths, 38 anchors, 11 MB |
| Flat Logo | photo-like | 1 | refused after 0.14 s (not flat art), 40 MB |
| Flat Logo | four-colour badge | 1 | 0.66 s, 5 paths, 78 anchors, 146 MB |
| Default with Shapes | photo-like | 1 | 0.05 s, 29 paths, 6278 anchors, 12 MB |
| Default with Shapes | four-colour badge | 1 | 0.02 s, 3 paths, 62 anchors, 12 MB |
| 6 Colors with Shapes | photo-like | 1 | 0.11 s, 78 paths, 9076 anchors, 24 MB |
| 6 Colors with Shapes | four-colour badge | 1 | 0.05 s, 5 paths, 84 anchors, 12 MB |
| 6 Colors with Auto Grouping | photo-like | 1 | 0.10 s, 78 paths, 9076 anchors, 24 MB |
| 6 Colors with Auto Grouping | four-colour badge | 1 | 0.05 s, 5 paths, 84 anchors, 12 MB |
| Shades of Gray with Auto Grouping | photo-like | 1 | 0.10 s, 81 paths, 6698 anchors, 41 MB |
| Shades of Gray with Auto Grouping | four-colour badge | 1 | 0.06 s, 10 paths, 725 anchors, 34 MB |
| High Fidelity Photo with Auto Grouping | photo-like | 1 | 0.24 s, 968 paths, 117041 anchors, 169 MB |
| High Fidelity Photo with Auto Grouping | four-colour badge | 1 | 0.05 s, 6 paths, 172 anchors, 23 MB |
| 6 Colors, no Gradients | photo-like | 1 | 0.09 s, 78 paths, 9076 anchors, 24 MB |
| 6 Colors, no Gradients | four-colour badge | 1 | 0.07 s, 5 paths, 84 anchors, 12 MB |
| 6 Colors with Gradients | photo-like | 1 | 0.15 s, 78 paths, 9076 anchors, 32 MB |
| 6 Colors with Gradients | four-colour badge | 1 | 0.08 s, 5 paths, 84 anchors, 12 MB |
| High Fidelity Photo, no Gradients | photo-like | 1 | 0.23 s, 968 paths, 117041 anchors, 170 MB |
| High Fidelity Photo, no Gradients | four-colour badge | 1 | 0.06 s, 6 paths, 172 anchors, 23 MB |
| High Fidelity Photo with Gradients | photo-like | 1 | 0.26 s, 968 paths, 117041 anchors, 169 MB |
| High Fidelity Photo with Gradients | four-colour badge | 1 | 0.09 s, 6 paths, 172 anchors, 23 MB |
| Default, enhanced | photo-like | 1 | 0.05 s, 29 paths, 6278 anchors, 12 MB |
| Default, enhanced | four-colour badge | 1 | 0.02 s, 3 paths, 62 anchors, 12 MB |
| Auto Color, enhanced | photo-like | 1 | 0.17 s, 732 paths, 45260 anchors, 27 MB |
| Auto Color, enhanced | four-colour badge | 1 | 0.05 s, 5 paths, 84 anchors, 12 MB |
| High Fidelity Photo, enhanced | photo-like | 1 | 0.26 s, 968 paths, 117041 anchors, 169 MB |
| High Fidelity Photo, enhanced | four-colour badge | 1 | 0.05 s, 6 paths, 172 anchors, 23 MB |
| Low Fidelity Photo, enhanced | photo-like | 1 | 0.14 s, 129 paths, 23014 anchors, 105 MB |
| Low Fidelity Photo, enhanced | four-colour badge | 1 | 0.05 s, 6 paths, 116 anchors, 23 MB |
| 3 Colors, enhanced | photo-like | 1 | 0.09 s, 9 paths, 3341 anchors, 18 MB |
| 3 Colors, enhanced | four-colour badge | 1 | 0.05 s, 4 paths, 76 anchors, 13 MB |
| 6 Colors, enhanced | photo-like | 1 | 0.10 s, 78 paths, 9076 anchors, 24 MB |
| 6 Colors, enhanced | four-colour badge | 1 | 0.05 s, 5 paths, 84 anchors, 12 MB |
| 16 Colors, enhanced | photo-like | 1 | 0.13 s, 178 paths, 20467 anchors, 31 MB |
| 16 Colors, enhanced | four-colour badge | 1 | 0.05 s, 5 paths, 88 anchors, 12 MB |
| Shades of Gray, enhanced | photo-like | 1 | 0.10 s, 81 paths, 6698 anchors, 40 MB |
| Shades of Gray, enhanced | four-colour badge | 1 | 0.06 s, 10 paths, 623 anchors, 35 MB |
| Black and White Logo, enhanced | photo-like | 1 | 0.05 s, 71 paths, 4933 anchors, 8 MB |
| Black and White Logo, enhanced | four-colour badge | 1 | 0.02 s, 1 paths, 46 anchors, 6 MB |
| Sketched Art, enhanced | photo-like | 1 | 0.04 s, 5 paths, 2873 anchors, 8 MB |
| Sketched Art, enhanced | four-colour badge | 1 | 0.02 s, 2 paths, 37 anchors, 6 MB |
| Silhouettes, enhanced | photo-like | 1 | 0.04 s, 2 paths, 556 anchors, 9 MB |
| Silhouettes, enhanced | four-colour badge | 1 | 0.02 s, 1 paths, 8 anchors, 6 MB |
| Line Art, enhanced | photo-like | 1 | 0.05 s, 54 paths, 4495 anchors, 8 MB |
| Line Art, enhanced | four-colour badge | 1 | 0.02 s, 1 paths, 37 anchors, 6 MB |
| Technical Drawing, enhanced | photo-like | 1 | 0.05 s, 221 paths, 6258 anchors, 8 MB |
| Technical Drawing, enhanced | four-colour badge | 1 | 0.02 s, 1 paths, 46 anchors, 6 MB |
| Outline, enhanced | photo-like | 1 | 0.05 s, 45 paths, 4110 anchors, 16 MB |
| Outline, enhanced | four-colour badge | 1 | 0.03 s, 1 paths, 38 anchors, 11 MB |
| Default | photo-like | 4 | 0.20 s, 48 paths, 14438 anchors, 49 MB |
| Default | four-colour badge | 4 | 0.10 s, 3 paths, 72 anchors, 48 MB |
| Auto Color | photo-like | 4 | 0.44 s, 1469 paths, 149389 anchors, 105 MB |
| Auto Color | four-colour badge | 4 | 0.19 s, 5 paths, 92 anchors, 47 MB |
| High Fidelity Photo | photo-like | 4 | 0.80 s, 4479 paths, 337884 anchors, 636 MB |
| High Fidelity Photo | four-colour badge | 4 | 0.25 s, 6 paths, 200 anchors, 94 MB |
| Low Fidelity Photo | photo-like | 4 | 0.40 s, 299 paths, 61601 anchors, 395 MB |
| Low Fidelity Photo | four-colour badge | 4 | 0.41 s, 6 paths, 152 anchors, 97 MB |
| 3 Colors | photo-like | 4 | 0.79 s, 20 paths, 8473 anchors, 73 MB |
| 3 Colors | four-colour badge | 4 | 0.30 s, 4 paths, 122 anchors, 53 MB |
| 6 Colors | photo-like | 4 | 0.34 s, 115 paths, 23295 anchors, 100 MB |
| 6 Colors | four-colour badge | 4 | 0.22 s, 5 paths, 92 anchors, 46 MB |
| 16 Colors | photo-like | 4 | 0.34 s, 353 paths, 52770 anchors, 129 MB |
| 16 Colors | four-colour badge | 4 | 0.22 s, 5 paths, 96 anchors, 47 MB |
| Shades of Gray | photo-like | 4 | 0.32 s, 137 paths, 18823 anchors, 156 MB |
| Shades of Gray | four-colour badge | 4 | 0.28 s, 10 paths, 1256 anchors, 136 MB |
| Black and White Logo | photo-like | 4 | 0.20 s, 208 paths, 13546 anchors, 32 MB |
| Black and White Logo | four-colour badge | 4 | 0.08 s, 1 paths, 57 anchors, 24 MB |
| Sketched Art | photo-like | 4 | 0.20 s, 9 paths, 5387 anchors, 32 MB |
| Sketched Art | four-colour badge | 4 | 0.09 s, 2 paths, 196 anchors, 24 MB |
| Silhouettes | photo-like | 4 | 0.21 s, 2 paths, 1465 anchors, 36 MB |
| Silhouettes | four-colour badge | 4 | 0.08 s, 1 paths, 28 anchors, 24 MB |
| Line Art | photo-like | 4 | 0.20 s, 127 paths, 11330 anchors, 32 MB |
| Line Art | four-colour badge | 4 | 0.09 s, 1 paths, 42 anchors, 24 MB |
| Technical Drawing | photo-like | 4 | 0.20 s, 931 paths, 20628 anchors, 32 MB |
| Technical Drawing | four-colour badge | 4 | 0.08 s, 1 paths, 57 anchors, 24 MB |
| Outline | photo-like | 4 | 0.25 s, 113 paths, 10815 anchors, 64 MB |
| Outline | four-colour badge | 4 | 0.13 s, 1 paths, 42 anchors, 44 MB |
| Flat Logo | photo-like | 4 | refused after 0.14 s (not flat art), 40 MB |
| Flat Logo | four-colour badge | 4 | 0.67 s, 5 paths, 69 anchors, 146 MB |
| Default with Shapes | photo-like | 4 | 0.22 s, 48 paths, 14438 anchors, 49 MB |
| Default with Shapes | four-colour badge | 4 | 0.10 s, 3 paths, 72 anchors, 48 MB |
| 6 Colors with Shapes | photo-like | 4 | 0.31 s, 115 paths, 23292 anchors, 100 MB |
| 6 Colors with Shapes | four-colour badge | 4 | 0.19 s, 5 paths, 92 anchors, 46 MB |
| 6 Colors with Auto Grouping | photo-like | 4 | 0.34 s, 115 paths, 23295 anchors, 100 MB |
| 6 Colors with Auto Grouping | four-colour badge | 4 | 0.21 s, 5 paths, 92 anchors, 47 MB |
| Shades of Gray with Auto Grouping | photo-like | 4 | 0.32 s, 137 paths, 18823 anchors, 155 MB |
| Shades of Gray with Auto Grouping | four-colour badge | 4 | 0.30 s, 10 paths, 1256 anchors, 136 MB |
| High Fidelity Photo with Auto Grouping | photo-like | 4 | 0.85 s, 4479 paths, 337884 anchors, 639 MB |
| High Fidelity Photo with Auto Grouping | four-colour badge | 4 | 0.23 s, 6 paths, 200 anchors, 94 MB |
| 6 Colors, no Gradients | photo-like | 4 | 0.27 s, 115 paths, 23295 anchors, 100 MB |
| 6 Colors, no Gradients | four-colour badge | 4 | 0.19 s, 5 paths, 92 anchors, 46 MB |
| 6 Colors with Gradients | photo-like | 4 | 0.78 s, 115 paths, 23295 anchors, 128 MB |
| 6 Colors with Gradients | four-colour badge | 4 | 0.49 s, 5 paths, 92 anchors, 48 MB |
| High Fidelity Photo, no Gradients | photo-like | 4 | 0.72 s, 4479 paths, 337884 anchors, 633 MB |
| High Fidelity Photo, no Gradients | four-colour badge | 4 | 0.23 s, 6 paths, 200 anchors, 97 MB |
| High Fidelity Photo with Gradients | photo-like | 4 | 1.06 s, 4479 paths, 337884 anchors, 642 MB |
| High Fidelity Photo with Gradients | four-colour badge | 4 | 0.34 s, 6 paths, 200 anchors, 96 MB |
| Default, enhanced | photo-like | 4 | 0.22 s, 48 paths, 14438 anchors, 49 MB |
| Default, enhanced | four-colour badge | 4 | 0.10 s, 3 paths, 72 anchors, 48 MB |
| Auto Color, enhanced | photo-like | 4 | 0.49 s, 1469 paths, 149374 anchors, 113 MB |
| Auto Color, enhanced | four-colour badge | 4 | 0.18 s, 5 paths, 92 anchors, 46 MB |
| High Fidelity Photo, enhanced | photo-like | 4 | 0.83 s, 4479 paths, 337884 anchors, 638 MB |
| High Fidelity Photo, enhanced | four-colour badge | 4 | 0.23 s, 6 paths, 200 anchors, 93 MB |
| Low Fidelity Photo, enhanced | photo-like | 4 | 0.39 s, 299 paths, 61598 anchors, 408 MB |
| Low Fidelity Photo, enhanced | four-colour badge | 4 | 0.23 s, 6 paths, 128 anchors, 93 MB |
| 3 Colors, enhanced | photo-like | 4 | 0.28 s, 20 paths, 8471 anchors, 73 MB |
| 3 Colors, enhanced | four-colour badge | 4 | 0.19 s, 4 paths, 84 anchors, 51 MB |
| 6 Colors, enhanced | photo-like | 4 | 0.30 s, 115 paths, 23292 anchors, 100 MB |
| 6 Colors, enhanced | four-colour badge | 4 | 0.19 s, 5 paths, 92 anchors, 46 MB |
| 16 Colors, enhanced | photo-like | 4 | 0.33 s, 353 paths, 52772 anchors, 126 MB |
| 16 Colors, enhanced | four-colour badge | 4 | 0.19 s, 5 paths, 96 anchors, 46 MB |
| Shades of Gray, enhanced | photo-like | 4 | 0.30 s, 137 paths, 18823 anchors, 163 MB |
| Shades of Gray, enhanced | four-colour badge | 4 | 0.26 s, 10 paths, 1056 anchors, 137 MB |
| Black and White Logo, enhanced | photo-like | 4 | 0.21 s, 208 paths, 13546 anchors, 32 MB |
| Black and White Logo, enhanced | four-colour badge | 4 | 0.08 s, 1 paths, 57 anchors, 24 MB |
| Sketched Art, enhanced | photo-like | 4 | 0.19 s, 9 paths, 5387 anchors, 32 MB |
| Sketched Art, enhanced | four-colour badge | 4 | 0.08 s, 2 paths, 38 anchors, 24 MB |
| Silhouettes, enhanced | photo-like | 4 | 0.22 s, 2 paths, 1465 anchors, 36 MB |
| Silhouettes, enhanced | four-colour badge | 4 | 0.08 s, 1 paths, 8 anchors, 24 MB |
| Line Art, enhanced | photo-like | 4 | 0.21 s, 127 paths, 11330 anchors, 32 MB |
| Line Art, enhanced | four-colour badge | 4 | 0.09 s, 1 paths, 42 anchors, 24 MB |
| Technical Drawing, enhanced | photo-like | 4 | 0.21 s, 931 paths, 20628 anchors, 32 MB |
| Technical Drawing, enhanced | four-colour badge | 4 | 0.08 s, 1 paths, 57 anchors, 24 MB |
| Outline, enhanced | photo-like | 4 | 0.24 s, 113 paths, 10815 anchors, 64 MB |
| Outline, enhanced | four-colour badge | 4 | 0.12 s, 1 paths, 42 anchors, 44 MB |
| Default | photo-like | 16 | 1.08 s, 127 paths, 34034 anchors, 199 MB |
| Default | four-colour badge | 16 | 0.51 s, 3 paths, 88 anchors, 184 MB |
| Auto Color | photo-like | 16 | 1.66 s, 5390 paths, 454670 anchors, 277 MB |
| Auto Color | four-colour badge | 16 | 0.84 s, 5 paths, 112 anchors, 184 MB |
| High Fidelity Photo | photo-like | 16 | 4.31 s, 31139 paths, 1192715 anchors, 880 MB |
| High Fidelity Photo | four-colour badge | 16 | 1.10 s, 6 paths, 264 anchors, 257 MB |
| Low Fidelity Photo | photo-like | 16 | 1.98 s, 2036 paths, 182778 anchors, 754 MB |
| Low Fidelity Photo | four-colour badge | 16 | 1.09 s, 6 paths, 180 anchors, 257 MB |
| 3 Colors | photo-like | 16 | 1.50 s, 76 paths, 23024 anchors, 290 MB |
| 3 Colors | four-colour badge | 16 | 1.05 s, 4 paths, 190 anchors, 213 MB |
| 6 Colors | photo-like | 16 | 1.57 s, 470 paths, 60831 anchors, 298 MB |
| 6 Colors | four-colour badge | 16 | 1.07 s, 5 paths, 112 anchors, 184 MB |
| 16 Colors | photo-like | 16 | 1.68 s, 1973 paths, 145642 anchors, 303 MB |
| 16 Colors | four-colour badge | 16 | 1.09 s, 5 paths, 114 anchors, 188 MB |
| Shades of Gray | photo-like | 16 | 2.48 s, 686 paths, 53654 anchors, 407 MB |
| Shades of Gray | four-colour badge | 16 | 1.96 s, 9 paths, 2208 anchors, 270 MB |
| Black and White Logo | photo-like | 16 | 1.37 s, 1128 paths, 45273 anchors, 129 MB |
| Black and White Logo | four-colour badge | 16 | 0.47 s, 1 paths, 69 anchors, 88 MB |
| Sketched Art | photo-like | 16 | 1.19 s, 15 paths, 11486 anchors, 128 MB |
| Sketched Art | four-colour badge | 16 | 0.52 s, 2 paths, 361 anchors, 88 MB |
| Silhouettes | photo-like | 16 | 1.45 s, 3 paths, 3350 anchors, 144 MB |
| Silhouettes | four-colour badge | 16 | 0.48 s, 1 paths, 32 anchors, 95 MB |
| Line Art | photo-like | 16 | 1.23 s, 713 paths, 37370 anchors, 129 MB |
| Line Art | four-colour badge | 16 | 0.50 s, 1 paths, 49 anchors, 88 MB |
| Technical Drawing | photo-like | 16 | 1.58 s, 4577 paths, 76559 anchors, 130 MB |
| Technical Drawing | four-colour badge | 16 | 0.44 s, 1 paths, 69 anchors, 88 MB |
| Outline | photo-like | 16 | 1.47 s, 632 paths, 34884 anchors, 256 MB |
| Outline | four-colour badge | 16 | 0.75 s, 1 paths, 49 anchors, 176 MB |
| Flat Logo | photo-like | 16 | refused after 0.20 s (not flat art), 40 MB |
| Flat Logo | four-colour badge | 16 | 0.80 s, 5 paths, 67 anchors, 146 MB |
| Default with Shapes | photo-like | 16 | 1.19 s, 127 paths, 34034 anchors, 199 MB |
| Default with Shapes | four-colour badge | 16 | 0.54 s, 3 paths, 88 anchors, 184 MB |
| 6 Colors with Shapes | photo-like | 16 | 1.40 s, 470 paths, 60823 anchors, 298 MB |
| 6 Colors with Shapes | four-colour badge | 16 | 1.00 s, 5 paths, 112 anchors, 185 MB |
| 6 Colors with Auto Grouping | photo-like | 16 | 1.64 s, 470 paths, 60831 anchors, 298 MB |
| 6 Colors with Auto Grouping | four-colour badge | 16 | 1.04 s, 5 paths, 112 anchors, 185 MB |
| Shades of Gray with Auto Grouping | photo-like | 16 | 2.19 s, 686 paths, 53654 anchors, 407 MB |
| Shades of Gray with Auto Grouping | four-colour badge | 16 | 1.63 s, 9 paths, 2208 anchors, 270 MB |
| High Fidelity Photo with Auto Grouping | photo-like | 16 | 9.72 s, 31139 paths, 1192715 anchors, 880 MB |
| High Fidelity Photo with Auto Grouping | four-colour badge | 16 | 1.64 s, 6 paths, 264 anchors, 257 MB |
| 6 Colors, no Gradients | photo-like | 16 | 1.66 s, 470 paths, 60831 anchors, 298 MB |
| 6 Colors, no Gradients | four-colour badge | 16 | 1.13 s, 5 paths, 112 anchors, 184 MB |
| 6 Colors with Gradients | photo-like | 16 | 3.71 s, 470 paths, 60831 anchors, 512 MB |
| 6 Colors with Gradients | four-colour badge | 16 | 1.71 s, 5 paths, 112 anchors, 188 MB |
| High Fidelity Photo, no Gradients | photo-like | 16 | 4.87 s, 31139 paths, 1192715 anchors, 880 MB |
| High Fidelity Photo, no Gradients | four-colour badge | 16 | 1.50 s, 6 paths, 264 anchors, 257 MB |
| High Fidelity Photo with Gradients | photo-like | 16 | 6.74 s, 31139 paths, 1192715 anchors, 880 MB |
| High Fidelity Photo with Gradients | four-colour badge | 16 | 1.68 s, 6 paths, 264 anchors, 257 MB |
| Default, enhanced | photo-like | 16 | 1.18 s, 127 paths, 34034 anchors, 199 MB |
| Default, enhanced | four-colour badge | 16 | 0.57 s, 3 paths, 88 anchors, 184 MB |
| Auto Color, enhanced | photo-like | 16 | 2.31 s, 5390 paths, 454626 anchors, 283 MB |
| Auto Color, enhanced | four-colour badge | 16 | 0.98 s, 5 paths, 112 anchors, 188 MB |
| High Fidelity Photo, enhanced | photo-like | 16 | 9.09 s, 31139 paths, 1192715 anchors, 880 MB |
| High Fidelity Photo, enhanced | four-colour badge | 16 | 1.47 s, 6 paths, 264 anchors, 263 MB |
| Low Fidelity Photo, enhanced | photo-like | 16 | 2.70 s, 2036 paths, 182770 anchors, 754 MB |
| Low Fidelity Photo, enhanced | four-colour badge | 16 | 1.60 s, 6 paths, 149 anchors, 257 MB |
| 3 Colors, enhanced | photo-like | 16 | 1.82 s, 76 paths, 23019 anchors, 290 MB |
| 3 Colors, enhanced | four-colour badge | 16 | 1.24 s, 4 paths, 98 anchors, 205 MB |
| 6 Colors, enhanced | photo-like | 16 | 2.20 s, 470 paths, 60823 anchors, 298 MB |
| 6 Colors, enhanced | four-colour badge | 16 | 2.19 s, 5 paths, 112 anchors, 185 MB |
| 16 Colors, enhanced | photo-like | 16 | 2.03 s, 1973 paths, 145623 anchors, 303 MB |
| 16 Colors, enhanced | four-colour badge | 16 | 1.13 s, 5 paths, 114 anchors, 188 MB |
| Shades of Gray, enhanced | photo-like | 16 | 2.76 s, 686 paths, 53648 anchors, 407 MB |
| Shades of Gray, enhanced | four-colour badge | 16 | 3.36 s, 9 paths, 1989 anchors, 270 MB |
| Black and White Logo, enhanced | photo-like | 16 | 1.58 s, 1128 paths, 45272 anchors, 129 MB |
| Black and White Logo, enhanced | four-colour badge | 16 | 0.59 s, 1 paths, 69 anchors, 88 MB |
| Sketched Art, enhanced | photo-like | 16 | 1.52 s, 15 paths, 11486 anchors, 128 MB |
| Sketched Art, enhanced | four-colour badge | 16 | 0.54 s, 2 paths, 44 anchors, 88 MB |
| Silhouettes, enhanced | photo-like | 16 | 1.82 s, 3 paths, 3350 anchors, 144 MB |
| Silhouettes, enhanced | four-colour badge | 16 | 0.56 s, 1 paths, 8 anchors, 95 MB |
| Line Art, enhanced | photo-like | 16 | 1.43 s, 713 paths, 37370 anchors, 129 MB |
| Line Art, enhanced | four-colour badge | 16 | 0.58 s, 1 paths, 49 anchors, 88 MB |
| Technical Drawing, enhanced | photo-like | 16 | 1.76 s, 4577 paths, 76558 anchors, 130 MB |
| Technical Drawing, enhanced | four-colour badge | 16 | 0.52 s, 1 paths, 69 anchors, 88 MB |
| Outline, enhanced | photo-like | 16 | 1.64 s, 632 paths, 34884 anchors, 256 MB |
| Outline, enhanced | four-colour badge | 16 | 0.73 s, 1 paths, 49 anchors, 176 MB |

## Centre lines

Measured 2026-10-17 with maximal-disc pruning and junction merging, against base `3e44478f`.
Earlier preset and cost tables retain their previous measurements. Regenerate only this class:

```text
TRACE_QUALITY_CLASS=centre-lines TRACE_QUALITY_REPORT=centre-lines.md cargo test --release -p vectorcraft-trace --test quality quality_report -- --ignored --nocapture
```

Junction clusters use their deepest pixel and a disc radius of `distance − 0.5` px. Terminal
branches shorter than `r + max(2, 0.5 × min(2r, Stroke))` are pruned, shortest first. Tiny
self-loops inside the junction disc are removed; overlapping junctions and geometrically split
crossings merge, and runs join through degree-two nodes. Simplification is bounded to 32 rounds;
nodes and branches are each capped at 200,000. An over-budget blob stays with the fill path.
Whole-blob selection, ink refinement, cap detection and curve fitting retain their current rules.

Code-drawn scenes only: 23 scenes, each anti-aliased (8×8 coverage samples) and hard (that coverage thresholded at 0.5). Coordinates, widths and scene Stroke values are fixed; junction/letter/crossing coordinates include the 0.27/0.41 px offset. Create Fills and Strokes are on, Stroke is the scene's limit (13 px, mix 6 px, varying 12 px), Black and White threshold 128, Noise 4, Ignore White on, other settings at their defaults. Today's tracer has no minimum-length setting (the dashed scenes specify 0 for future use).

Distances use dense exact centre samples (≤0.25 px), emitted curves sampled at ≤0.5 px control-polygon steps, and nearest-point searches capped at 30 px. Found is the fraction of true samples within 1.5 px; width errors use output samples within 2 px of a true stroke. Samples inside exact fills are excluded. Topology is recovered from emitted subpath endpoints and intersections, merging coincident positions within 0.05 px; an endpoint contributes one arm, an interior two. Junction matching radius is local true width + 3 px; ends use half-width + 4 px. Each match is used once. Free ends exclude true junctions, corners, other strokes and fills; unmatched emitted ends are spurs. Junctions/ends within 2 px of fills are excluded. End error is distance, with signed error along the outward tangent in parentheses. Coverage is a union of painted output at pixel centres (0.02 px flattening tolerance, actual emitted caps), compared with the exact swept shape at pixel centres, for both input variants. Area error is signed relative to exact area. Anchors counts stroke anchors, including all compound subpaths; paths counts stroked + filled paths. Time covers tracing only, one scene at a time. Profile-dependent columns are unavailable (–).

### Centre line and width (px)

| scene | input | paths | out→true mean / p95 | true→out mean / p95 | found % | width err uniform mean / p95 | variable mean / p95 | profile mean |
|---|---|---|---|---|---|---|---|---|
| lines w1 | aa | 93+0 | 0.09 / 0.16 | 2.64 / 30.00 | 90.9 | 0.05 / 0.13 | – | – |
| lines w1 | hard | 92+0 | 0.18 / 0.38 | 1.93 / 5.01 | 82.3 | 0.14 / 1.00 | – | – |
| lines w2 | aa | 24+0 | 0.11 / 0.27 | 0.16 / 0.29 | 100.0 | 0.03 / 0.06 | – | – |
| lines w2 | hard | 24+0 | 0.22 / 0.38 | 0.25 / 0.42 | 100.0 | 0.18 / 0.59 | – | – |
| lines w3 | aa | 24+0 | 0.09 / 0.21 | 0.15 / 0.27 | 100.0 | 0.04 / 0.09 | – | – |
| lines w3 | hard | 24+0 | 0.20 / 0.42 | 0.24 / 0.46 | 100.0 | 0.22 / 0.54 | – | – |
| lines w4 | aa | 24+0 | 0.09 / 0.20 | 0.15 / 0.26 | 100.0 | 0.06 / 0.12 | – | – |
| lines w4 | hard | 24+0 | 0.20 / 0.42 | 0.24 / 0.46 | 100.0 | 0.12 / 0.46 | – | – |
| lines w6 | aa | 24+0 | 0.10 / 0.22 | 0.15 / 0.26 | 100.0 | 0.09 / 0.19 | – | – |
| lines w6 | hard | 24+0 | 0.20 / 0.40 | 0.24 / 0.43 | 100.0 | 0.24 / 0.40 | – | – |
| lines w8 | aa | 24+0 | 0.11 / 0.26 | 0.16 / 0.29 | 100.0 | 0.13 / 0.26 | – | – |
| lines w8 | hard | 24+0 | 0.23 / 0.56 | 0.26 / 0.59 | 100.0 | 0.13 / 0.49 | – | – |
| lines w10 | aa | 24+0 | 0.11 / 0.24 | 0.16 / 0.29 | 100.0 | 0.16 / 0.32 | – | – |
| lines w10 | hard | 24+0 | 0.24 / 0.64 | 0.27 / 0.67 | 99.9 | 0.24 / 0.61 | – | – |
| lines w12 | aa | 24+0 | 0.12 / 0.30 | 0.17 / 0.32 | 100.0 | 0.20 / 0.39 | – | – |
| lines w12 | hard | 24+0 | 0.24 / 0.60 | 0.27 / 0.61 | 99.9 | 0.25 / 0.52 | – | – |
| curves w2 | aa | 4+0 | 0.11 / 0.22 | 0.15 / 0.26 | 100.0 | 0.01 / 0.01 | – | – |
| curves w2 | hard | 4+0 | 0.22 / 0.55 | 0.25 / 0.57 | 100.0 | 0.05 / 0.06 | – | – |
| curves w4 | aa | 4+0 | 0.12 / 0.25 | 0.16 / 0.28 | 99.9 | 0.01 / 0.03 | – | – |
| curves w4 | hard | 4+0 | 0.24 / 0.57 | 0.26 / 0.56 | 99.9 | 0.00 / 0.00 | – | – |
| curves w8 | aa | 4+0 | 0.16 / 0.30 | 0.22 / 0.34 | 98.7 | 0.02 / 0.06 | – | – |
| curves w8 | hard | 4+0 | 0.26 / 0.61 | 0.29 / 0.63 | 98.8 | 0.00 / 0.00 | – | – |
| circles | aa | 25+0 | 0.09 / 0.23 | 0.24 / 0.30 | 97.6 | 0.00 / 0.02 | – | – |
| circles | hard | 23+0 | 0.27 / 0.64 | 0.45 / 0.75 | 96.5 | 0.02 / 0.10 | – | – |
| junctions w2 | aa | 7+0 | 0.14 / 0.34 | 0.18 / 0.35 | 100.0 | 0.12 / 0.03 | – | – |
| junctions w2 | hard | 7+0 | 0.36 / 0.82 | 0.40 / 0.85 | 100.0 | 0.14 / 0.12 | – | – |
| junctions w4 | aa | 7+0 | 0.17 / 0.72 | 0.20 / 0.49 | 99.5 | 0.27 / 5.00 | – | – |
| junctions w4 | hard | 7+0 | 0.42 / 1.11 | 0.42 / 0.98 | 99.2 | 0.30 / 5.00 | – | – |
| junctions w8 | aa | 7+0 | 0.28 / 1.49 | 0.31 / 1.45 | 95.1 | 0.30 / 0.04 | – | – |
| junctions w8 | hard | 7+0 | 0.47 / 1.36 | 0.50 / 1.47 | 95.1 | 0.33 / 0.22 | – | – |
| crossings w3 | aa | 5+0 | 0.14 / 0.34 | 0.18 / 0.31 | 100.0 | 0.05 / 0.09 | – | – |
| crossings w3 | hard | 5+0 | 0.39 / 1.14 | 0.40 / 0.93 | 99.9 | 0.20 / 0.32 | – | – |
| crossings w6 | aa | 5+0 | 0.42 / 2.16 | 0.38 / 1.79 | 92.3 | 0.10 / 0.19 | – | – |
| crossings w6 | hard | 5+0 | 0.57 / 2.27 | 0.54 / 1.95 | 90.9 | 0.25 / 0.36 | – | – |
| dashed w2 | aa | 31+0 | 0.10 / 0.22 | 0.15 / 0.25 | 100.0 | 0.02 / 0.05 | – | – |
| dashed w2 | hard | 31+0 | 0.14 / 0.36 | 0.18 / 0.38 | 100.0 | 0.09 / 0.34 | – | – |
| dashed w4 | aa | 29+0 | 0.11 / 0.28 | 0.15 / 0.31 | 100.0 | 0.01 / 0.03 | – | – |
| dashed w4 | hard | 29+0 | 0.15 / 0.42 | 0.19 / 0.41 | 100.0 | 0.11 / 0.46 | – | – |
| letters w3 | aa | 7+0 | 0.19 / 0.86 | 0.22 / 0.62 | 98.7 | 0.01 / 0.13 | – | – |
| letters w3 | hard | 7+0 | 0.33 / 0.92 | 0.34 / 0.71 | 99.0 | 0.06 / 0.20 | – | – |
| letters w5 | aa | 7+0 | 0.27 / 1.49 | 0.29 / 1.31 | 95.9 | 0.02 / 0.20 | – | – |
| letters w5 | hard | 7+0 | 0.40 / 1.48 | 0.41 / 1.29 | 96.2 | 0.05 / 0.20 | – | – |
| mix | aa | 3+1 | 0.10 / 0.21 | 12.89 / 30.00 | 57.3 | 0.03 / 0.14 | – | – |
| mix | hard | 3+1 | 0.22 / 0.43 | 12.95 / 30.00 | 57.3 | 0.08 / 0.15 | – | – |
| varying | aa | 5+0 | 0.13 / 0.28 | 0.19 / 0.31 | 99.5 | 1.89 / 4.13 | – | – |
| varying | hard | 5+0 | 0.20 / 0.51 | 0.24 / 0.52 | 99.7 | 1.90 / 4.22 | – | – |

### Topology, ends, coverage

| scene | input | junctions ok / wrong deg / missed / extra | junction pos err | ends found | end err (signed) | spurs | IoU uniform / profile | area err uniform / profile | fill→fill % | anchors | width pts | ms |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| lines w1 | aa | 0 / 0 / 0 / 76 | – | 43/48 | 0.71 (-0.69) | 88 | 0.790 / – | -6.2% / – | – | 186 | – | 4.6 |
| lines w1 | hard | 0 / 0 / 0 / 0 | – | 42/48 | 1.50 (-1.42) | 142 | 0.644 / – | -25.8% / – | – | 184 | – | 3.6 |
| lines w2 | aa | 0 / 0 / 0 / 0 | – | 48/48 | 0.32 (-0.20) | 0 | 0.914 / – | -0.3% / – | – | 71 | – | 4.3 |
| lines w2 | hard | 0 / 0 / 0 / 0 | – | 48/48 | 0.38 (-0.10) | 0 | 0.808 / – | -2.0% / – | – | 58 | – | 4.2 |
| lines w3 | aa | 0 / 0 / 0 / 0 | – | 48/48 | 0.26 (-0.17) | 0 | 0.931 / – | -3.0% / – | – | 64 | – | 4.4 |
| lines w3 | hard | 0 / 0 / 0 / 0 | – | 48/48 | 0.46 (-0.24) | 0 | 0.881 / – | -2.5% / – | – | 50 | – | 4.3 |
| lines w4 | aa | 0 / 0 / 0 / 0 | – | 48/48 | 0.25 (-0.18) | 0 | 0.964 / – | -0.6% / – | – | 70 | – | 4.6 |
| lines w4 | hard | 0 / 0 / 0 / 0 | – | 48/48 | 0.40 (-0.15) | 0 | 0.906 / – | -0.1% / – | – | 54 | – | 4.5 |
| lines w6 | aa | 0 / 0 / 0 / 0 | – | 48/48 | 0.30 (-0.18) | 0 | 0.965 / – | -0.3% / – | – | 65 | – | 5.1 |
| lines w6 | hard | 0 / 0 / 0 / 0 | – | 48/48 | 0.45 (-0.12) | 0 | 0.923 / – | 1.4% / – | – | 55 | – | 4.9 |
| lines w8 | aa | 0 / 0 / 0 / 0 | – | 48/48 | 0.37 (-0.20) | 0 | 0.967 / – | -1.0% / – | – | 66 | – | 5.4 |
| lines w8 | hard | 0 / 0 / 0 / 0 | – | 48/48 | 0.46 (-0.18) | 0 | 0.934 / – | -0.3% / – | – | 53 | – | 5.4 |
| lines w10 | aa | 0 / 0 / 0 / 0 | – | 48/48 | 0.35 (-0.16) | 0 | 0.969 / – | -0.7% / – | – | 66 | – | 5.9 |
| lines w10 | hard | 0 / 0 / 0 / 0 | – | 48/48 | 0.50 (-0.12) | 0 | 0.942 / – | -0.1% / – | – | 52 | – | 5.9 |
| lines w12 | aa | 0 / 0 / 0 / 0 | – | 48/48 | 0.40 (-0.16) | 0 | 0.969 / – | -0.6% / – | – | 70 | – | 6.4 |
| lines w12 | hard | 0 / 0 / 0 / 0 | – | 48/48 | 0.54 (-0.11) | 0 | 0.949 / – | -0.2% / – | – | 53 | – | 6.3 |
| curves w2 | aa | 0 / 0 / 0 / 0 | – | 8/8 | 0.18 (-0.13) | 0 | 0.921 / – | -0.6% / – | – | 34 | – | 4.7 |
| curves w2 | hard | 0 / 0 / 0 / 0 | – | 8/8 | 0.22 (-0.05) | 0 | 0.821 / – | -2.4% / – | – | 29 | – | 4.7 |
| curves w4 | aa | 0 / 0 / 0 / 0 | – | 8/8 | 0.56 (-0.42) | 0 | 0.949 / – | -0.8% / – | – | 35 | – | 5.1 |
| curves w4 | hard | 0 / 0 / 0 / 0 | – | 8/8 | 0.66 (-0.38) | 0 | 0.904 / – | -0.5% / – | – | 30 | – | 5.2 |
| curves w8 | aa | 0 / 0 / 0 / 0 | – | 8/8 | 2.24 (-2.01) | 0 | 0.956 / – | -0.9% / – | – | 42 | – | 5.9 |
| curves w8 | hard | 0 / 0 / 0 / 0 | – | 8/8 | 1.97 (-1.67) | 0 | 0.936 / – | -0.5% / – | – | 31 | – | 5.9 |
| circles | aa | 0 / 0 / 0 / 0 | – | 0/0 | – (–) | 32 | 0.964 / – | -0.9% / – | – | 71 | – | 7.4 |
| circles | hard | 0 / 0 / 0 / 0 | – | 0/0 | – (–) | 28 | 0.888 / – | -0.7% / – | – | 52 | – | 8.0 |
| junctions w2 | aa | 7 / 0 / 0 / 0 | 1.17 | 23/23 | 0.32 (-0.16) | 0 | 0.889 / – | 6.1% / – | – | 84 | – | 3.7 |
| junctions w2 | hard | 7 / 0 / 0 / 0 | 1.17 | 23/23 | 0.49 (-0.15) | 0 | 0.771 / – | 4.9% / – | – | 59 | – | 3.6 |
| junctions w4 | aa | 7 / 0 / 0 / 1 | 1.62 | 23/23 | 0.33 (-0.26) | 0 | 0.919 / – | 5.0% / – | – | 100 | – | 4.1 |
| junctions w4 | hard | 7 / 0 / 0 / 1 | 1.62 | 23/23 | 0.65 (-0.32) | 0 | 0.850 / – | 7.0% / – | – | 81 | – | 4.0 |
| junctions w8 | aa | 7 / 0 / 0 / 2 | 2.76 | 23/23 | 0.43 (-0.27) | 0 | 0.937 / – | 2.7% / – | – | 93 | – | 4.7 |
| junctions w8 | hard | 7 / 0 / 0 / 1 | 2.76 | 23/23 | 0.80 (-0.35) | 0 | 0.913 / – | 2.5% / – | – | 77 | – | 4.7 |
| crossings w3 | aa | 5 / 0 / 0 / 2 | 0.25 | 20/20 | 0.27 (-0.18) | 0 | 0.935 / – | -1.1% / – | – | 74 | – | 2.8 |
| crossings w3 | hard | 5 / 0 / 0 / 5 | 0.25 | 20/20 | 0.41 (-0.17) | 0 | 0.791 / – | -7.6% / – | – | 69 | – | 2.8 |
| crossings w6 | aa | 5 / 0 / 0 / 1 | 0.25 | 20/20 | 0.35 (-0.17) | 0 | 0.925 / – | -3.6% / – | – | 112 | – | 3.3 |
| crossings w6 | hard | 5 / 0 / 0 / 4 | 0.25 | 20/20 | 0.51 (-0.20) | 0 | 0.872 / – | -2.0% / – | – | 81 | – | 3.4 |
| dashed w2 | aa | 0 / 0 / 0 / 0 | – | 62/62 | 0.28 (-0.20) | 0 | 0.927 / – | -2.6% / – | – | 64 | – | 2.0 |
| dashed w2 | hard | 0 / 0 / 0 / 0 | – | 62/62 | 0.33 (-0.11) | 0 | 0.926 / – | -3.6% / – | – | 67 | – | 1.9 |
| dashed w4 | aa | 0 / 0 / 0 / 0 | – | 58/58 | 0.30 (-0.20) | 0 | 0.952 / – | -0.1% / – | – | 61 | – | 2.1 |
| dashed w4 | hard | 0 / 0 / 0 / 0 | – | 58/58 | 0.37 (-0.08) | 0 | 0.944 / – | -1.5% / – | – | 60 | – | 2.1 |
| letters w3 | aa | 7 / 0 / 0 / 6 | 1.47 | 17/17 | 0.62 (-0.47) | 1 | 0.923 / – | -0.9% / – | – | 114 | – | 3.3 |
| letters w3 | hard | 7 / 0 / 0 / 3 | 1.47 | 17/17 | 0.38 (-0.21) | 1 | 0.859 / – | -0.6% / – | – | 81 | – | 3.3 |
| letters w5 | aa | 6 / 1 / 0 / 6 | 2.10 | 17/17 | 0.99 (-0.83) | 0 | 0.929 / – | -1.7% / – | – | 124 | – | 3.7 |
| letters w5 | hard | 6 / 1 / 0 / 8 | 2.10 | 17/17 | 1.05 (-0.79) | 0 | 0.887 / – | -2.0% / – | – | 100 | – | 3.7 |
| mix | aa | 0 / 0 / 0 / 0 | – | 6/7 | 0.24 (-0.17) | 0 | 0.986 / – | 0.1% / – | 99.7 | 11 | – | 6.5 |
| mix | hard | 0 / 0 / 0 / 0 | – | 6/7 | 0.57 (-0.46) | 0 | 0.969 / – | -0.0% / – | 99.1 | 11 | – | 6.4 |
| varying | aa | 0 / 0 / 0 / 0 | – | 10/10 | 2.03 (-1.08) | 0 | 0.735 / – | 4.8% / – | – | 22 | – | 5.2 |
| varying | hard | 0 / 0 / 0 / 0 | – | 10/10 | 1.92 (-0.94) | 0 | 0.736 / – | 5.2% / – | – | 22 | – | 5.3 |

**aa totals**: junctions 44 correct, 1 wrong degree, 0 missed, 94 extra (of 45); ends 682 of 688; spurs 121.

**hard totals**: junctions 44 correct, 1 wrong degree, 0 missed, 22 extra (of 45); ends 681 of 688; spurs 171.

**Scope of the improvement:** correct junction degrees rise from 30/45 to 44/45 in each variant;
missed junctions fall from two to zero. Extra junctions fall from 104 to 94 aa and 27 to 22 hard.
Spur totals remain 121 aa / 171 hard: the one-pixel-line and circle scenes contribute 120 / 170
of those, with unchanged path counts (one-pixel lines: 93 aa / 92 hard; circles: 25 aa / 23 hard).
The graph stage does not reconnect separate blobs. Raw graph tests cover straight lines at 24
angles, junctions and 20–90° crossings; these do not replace the emitted-path measurements above.
The remaining spur/extra-junction target is not met by this graph-only change.

## Revision history

| date | change |
|:--|:--|
| 2026-10-17 | Auto Color keeps rare but distinct colours (stems, highlights, near-white on pale backgrounds): 66 of 66 flat test logos keep every key colour (was 39); scans unchanged |
| 2026-10-17 | Images over 1500 px on a side trace with a warning; the anchor limit is 8,000,000 (2,000,000 on the web); time and memory of 2000 × 2000 to 6000 × 4000 code-drawn photos |
| 2026-10-17 | `Raster::new` returns an error for RGBA bytes of the wrong length (it failed an assertion) and `Raster::pixel` reads nothing past the raster; traces of valid images unchanged |
| 2026-10-17 | M12: maximal-disc graph pruning, split-crossing merges and degree-two joins; correct junction degrees 30→44/45 aa and hard, none missed; remaining spurs unchanged. Centre-lines rerun, scores and earlier tables unchanged. |
| 2026-10-17 | M12: exact per-blob Euclidean distances and Guo–Hall thinning; centre-lines rerun on 46 inputs. Ends found: aa 638→682/688, hard 631→681/688. Whole-blob stroke/fill choice retained; preset/cost tables unchanged. |
| 2026-10-17 | M12: add the ignored centre-lines class and measure the existing Create Strokes output on 23 code-drawn scenes, aa and hard; centre/width/topology/end/coverage metrics, no new images, dependencies or behaviour |
| 2026-10-16 | Image Trace: a loop or junction-returning piece whose fit collapsed to one cubic keeps the fewest segments that hold its area and course instead of every positioned point, so a higher Paths no longer gives fewer segments on code-drawn noisy images (six falling runs to none; one other cause remains). |
| 2026-10-16 | Image Trace: a notch corner is cut only when the straight edges beside it lie in the shared piece, so both regions of the piece get the same points (code-drawn noisy colour images: 93 uncovered sample points to 0). |
| 2026-10-16 | Image Trace: a crack's sub-pixel shift is the mean of the readings from its two sides, exactly antisymmetric, so abutting regions get the same points on thin slivers (a code-drawn noisy ripple went from 482–1,228 uncovered sample points per trace to 0). |
| 2026-10-16 | Image Trace: the contour walk works out each turn from the pixels as it goes instead of following a precomputed table of per-corner directions; identical loops on all 65,536 4 × 4 masks and 400 seeded masks, and no slower (whole 1 MP traces 1–5% faster). |
| 2026-10-14 | Closed fitting retains the positioned boundary when straightening collapses a nonzero-area loop; four generated regressions; preset scores unchanged |
| 2026-10-14 | Tiny closed fits retain their positioned outlines instead of losing islands/holes; three generated regression tests; quality-harness baseline unchanged |
| 2026-10-14 | Gradients: a gradient that noise or a soft rim broke into pieces is found in its largest piece and grown over the rest (blurred and noisy linear gradients: 3 to 11 paths beside the gradient down to 0.5 with 6 Colors, 24 to 56 down to 0.5 to 1.5 with High Fidelity Photo) |
| 2026-10-14 | Enhanced versions of the presets (Auto Grouping, and Shapes in all but High Fidelity Photo; Flat Logo has none), each measured beside its preset; a hole that one colour fills is the same shape as what fills it, rectangles are sized by where their sides lie, and a group keeps its paths in the order they were traced in (grouped, a letter with a counter was painted with a light seam: Default with Auto Grouping, edge ΔE 10.24 to 9.88) |
| 2026-10-14 | Create: Strokes: a line whose ink is cut square at its ends gets square caps, and a wide line whose skeleton hooks towards a corner of its end ends on the line it was following (Outline: largest distance 2.94 px to 0.88; aliased lines 0.88 to 0.81) |
| 2026-10-13 | Shapes: corners that anti-aliasing or blur took a little off are corners, a rounded rectangle gets the radius that fits, a polygon its sides from the outline itself, hard-edged outlines a little more room (plain shapes found: 57 to 62% up to 98 to 100%); a "false shapes" column |
| 2026-10-13 | Flat Logo: the tones a scan's uneven paper makes of one colour are one colour (scans: 470 paths and 21 px off to 8.5 paths and 0.09 px) |
| 2026-10-13 | Snap Curves To Lines moves no end of a line by more than 1.6 px: long lines a degree or two off stay as the image has them (Technical Drawing: largest distance 4.21 px to 1.46) |
| 2026-10-13 | Gradients (off unless asked for): areas that are a linear gradient are found before the palette is made and traced as one path with a gradient fill; linear-gradient inputs with known lines and stops, and four columns for them |
| 2026-10-13 | Auto Grouping (off unless asked for): each path that has others inside it is grouped with them; a "grouped %" column, and three presets measured with the option on |
| 2026-10-13 | Shapes (off unless asked for): filled outlines that are a circle, an ellipse, a rectangle or a regular polygon come out as that shape; corners whose pixel is mostly outside the shape are corners; plain-shape inputs and a "shapes %" column |
| 2026-10-13 | Create: Strokes: centre lines placed in the middle of their ink and as wide as it, fitted as outlines are; centre-line distance, width error and anchors per stroke measured against the thin lines' truth |
| 2026-10-13 | Snap Curves To Lines levels lines within 2.5 degrees of level or upright; askew inputs and an "upright %" column, so the baseline was measured again with them (the tracer of main at `baf9693f`) |
| 2026-10-13 | Ignore Color; the Auto Color and Outline presets, measured from here on (Outline is the first measure of Create: Strokes) |
| 2026-10-13 | Thin lines: black and white read against the image's darkest black and the paper near each edge |
| 2026-10-13 | Memory: layers traced within the box of their pixels and a few at a time (16 megapixel photo: 3.9 GB to 0.9 GB) |
| 2026-10-13 | Shared edges: outlines are cut at their junctions and each piece is fitted once, the same from either side |
| 2026-10-13 | Photos: continuous-tone images are smoothed within their tones before quantising, and stacked layers go by lightness |
| 2026-10-13 | The palette: made of the picture's own colours, noise-made colours merged, blend pixels settled, cuts where colours differ most, colours compared in Oklab |
| 2026-10-13 | Sub-pixel edges and the outline fitter in Black and White, Grayscale and Color; the harness gains an aliased variant and an "outside px" column, so the baseline was measured again with it (same tracer, main at `baf9693f`) |
| 2026-10-12 | The harness and the baseline: every preset measured before any change to the tracer |
