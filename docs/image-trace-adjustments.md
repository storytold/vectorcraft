# Adjusting an Image Trace

Select a placed raster image, then choose **Image Trace** or a preset from its arrow
in Properties or the Control bar. The original image stays inside the tracing object so
that subsequent adjustments always use the original pixels.

To adjust the result, select the tracing object and click **Image Trace** in Properties or the
image icon in the Control bar: the Image Trace panel opens with **Advanced** expanded. An existing floating or docked panel is activated in
its current location. Opening the controls does not add a document undo step.

| Control | Range | Effect |
|---|---|---|
| Threshold (Black and White) | 0–255 | Higher values include lighter pixels as black ink. |
| Paths | 0–100% | Higher values fit outlines more closely to the image (see below). |
| Corners | 0–100% | Higher values turn more sharp bends into corner points (see below). |
| Noise | 1–100 px | An area in pixels: regions smaller than it are left out; higher values remove more small details (see below). |
| Method | Abutting / Overlapping | Chooses shared boundaries or stacked overlapping paths. |
| Create | Fills / Strokes | Creates filled areas, stroked lines, or both. |
| Ignore Color | Color picker | Leaves out the selected color (white by default). |

Sliders apply on release, and their numeric fields can be edited directly. Each committed
adjustment re-traces the selected object as one undo step and changes the preset to Custom.
While dragging, the preset and Enhanced row stay in place until release so the
slider retains its active interaction. Committed settings are synchronized immediately,
so Undo also restores the panel values before its next frame. View can show the source image, the result, outlines, or combinations without re-tracing.
Use **Expand** when finished to turn the result into ordinary editable vector objects;
after expanding, the tracing settings are no longer available. **Release** restores the source.

**Auto Color** (the Automatic palette) counts flat art's colours by area, and also keeps colours too
rare for that count when they are distinct and drawn as regions of their own (a stem, a highlight, a
white beside a cream), at most six; scattered specks of a colour add none. Adobe does not document how
its own palettes are chosen; this is VectorCraft's own method (see
[image-trace-quality.md](image-trace-quality.md#auto-color-keeps-rare-colours)).

**Document Library color groups.** With Palette › Document Library and Library › Document Swatches,
the panel's **Colors** menu traces with every document swatch (**All**) or with the colors of one
color group from the Swatches panel, as the public
[Image Trace panel options](https://helpx.adobe.com/illustrator/desktop/manage-objects/traces-mockups-symbols/image-trace-panel-options.html)
page ([archived](https://web.archive.org/web/20251230195658/https://helpx.adobe.com/illustrator/desktop/manage-objects/traces-mockups-symbols/image-trace-panel-options.html))
describes Document Library. In the params of `imageTrace.make` or `imageTrace.makeAndExpand`,
`"colorGroup": "<name>"` names the group. It needs `"mode": "color"`, `"palette": "documentLibrary"` and
`"library": "document"`; a `colorGroup` with a nonempty name and any other mode, palette or library is refused. An
empty name names no group and is accepted with any mode, palette or library; with Document Library and
`"library": "document"` it traces with every document swatch. A group whose name matches exactly is used first, then one whose name differs only in case. At
most `colors` of its colors are used. A missing group and a group with no solid colors are refused. Choosing another
mode, palette or library in the panel clears the group. A group the active document does not have shows as All
and traces as All.

**Document Library colors.** With Library › Document Swatches, with or without a color group,
traced fills and strokes take the library color without a link to its swatch. This includes global,
spot and tint swatches. Tints trace at the color shown by the tint swatch. Editing a swatch later
does not recolor the trace. Colors from other swatch libraries are also unlinked.

**Document Library** traces with the library's colors as they are. Each pixel takes the library
color nearest to it in lightness and hue, with a difference in hue or chroma counted three times
as much as one in lightness, so a gray or a muted brown takes a black, white or other neutral
library color rather than a saturated color of the same lightness.
**Abutting and Overlapping.** Abutting is the default Method, and the Default, Auto Color, 3 Colors,
6 Colors and 16 Colors presets use it. With Abutting, two neighboring colors share one exact edge. The
canvas smooths the edges of each path on its own, so each of the two paths covers only part of the
pixels along that edge, and a thin line of the background can show between the colors. Web browsers
and PDF viewers usually draw an exported SVG or PDF file the same way and show the same line. Raster
exports (PNG, JPEG, TIFF and the other bitmap formats) with None anti-aliasing do not show it at any
size. Raster exports with Art Optimized anti-aliasing, the default, do not show it up to 10 million
pixels and 16,384 pixels on a side. A raster export shows the line as the canvas does when it uses
Type Optimized anti-aliasing, or when it uses Art Optimized and is larger than either limit.
Overlapping stacks the colors instead: a lower color continues under the edge of the color above it,
and no background shows along that edge.
**Large images.** An image of more than 1500 pixels on a side is traced as any other, only more
slowly, and the trace says so: `imageTrace.make` returns the note in `warnings` and the status bar
shows it (the [results optimization](https://helpx.adobe.com/illustrator/using/image-trace-results-optimization.html)
page describes the same warning for images over 1500 px). An image of more than 64 megapixels (up to
192) is traced from a copy scaled down to 64 megapixels, and the trace says that too. Flat Logo traces at
most 1024 pixels on a side: a larger decoded raster is averaged down in whole-number blocks first (a 1025-pixel
image is traced at 513, and a 16,000 × 12,000 image, decoded at 9,459 × 7,094, is traced at 946 × 710), and the trace names both sizes in `warnings`. Flat Logo gives that note in place
of the slower-trace note for an image over 1500 pixels and in place of the 64-megapixel note. An image at most
1024 pixels on a side is traced at its own size, also when the named `logoColors` differ slightly from the
image's colors. Only an image with more than 150,000 different pixel values that are not exactly one traced
color is averaged down further: the factor doubles until at most 150,000 remain or the working copy has at
most 16,384 pixels (at most six times). A trace that would
make more than 8,000,000 anchor points (2,000,000 on the web) is refused to protect memory, with advice:
raise Noise, lower Paths or Colors, or pick a lower-fidelity preset. A trace keeps
about 90 bytes an anchor; a 4000 × 3000 photo at High Fidelity Photo makes about 1.9 million.
**Tracing in the background.** On the desktop a trace runs on a thread of its own, so the window
keeps answering while a photo or a large image is traced (#3098). Make, Make and Expand, the task
bar's Image Trace and the panel's Trace and presets start it. The status bar says "Tracing the
image…", and after 0.4 s a progress window shows how many of its colours are traced, with Cancel
(also Esc, `imageTrace.cancelRuns`) and Continue in Background (`imageTrace.hideRuns`). The result is
one undo step, as before. A new trace of the same object replaces one still running. A trace whose
image or settings changed meanwhile changes nothing, and the status bar says to trace it again.
`imageTrace.make` and `imageTrace.makeAndExpand` from MCP, scripts and the control channel trace at
once and return their result, as every trace on the web does.
These controls use VectorCraft's existing tracing algorithm. Parameter effects follow its
implementation; they do not claim pixel-identical results with another application.
The terminology and general control behavior are described in the
[public Image Trace panel documentation](https://helpx.adobe.com/illustrator/desktop/manage-objects/traces-mockups-symbols/image-trace-panel-options.html).

## Paths, Corners and Noise: documented meaning and VectorCraft's mapping

The public documentation describes what each control means, not the numbers behind it. VectorCraft
maps each control so that every documented property holds, and tests each one over every value.

| Control | Documented meaning (sources below) | VectorCraft's mapping |
|---|---|---|
| Paths | Sets the distance between the traced shape and the pixel shape: lower is a looser fit, higher a tighter one [1]. The scripting property `pathFitting` is the same rule seen from the tolerance side: a distance, lower is tighter [3]. | Paths sets the farthest a fitted curve may stray from the edge's points (anti-aliased edges): 0.495 px at 0, 0.31 px at 50, 0.08 px at 100, falling strictly with every step, `0.08 + 0.415 × (1 − Paths/100)^0.85` px. Paths 100 is the tightest fit the tracer makes. Hard (not anti-aliased) edges are known only to about half a pixel and get 2.5 times that. |
| Corners | Sets the emphasis on corners: how likely a sharp bend is to become a corner point; higher gives more corners [1]. The scripting property `cornerAngle` is the sharpness, in degrees, of a turn that counts as a corner [3]. | Corners sets the turn that makes a corner: 99.2° at 0 (only bends sharper than a right angle), 57.9° at 75, 35° at 100, falling strictly with every step, `99.2 − 64.2 × (Corners/100)^1.53` degrees. |
| Noise | An area in pixels that is ignored while tracing; higher gives less noise; suggested 1–10 for low-resolution and 20–50 for high-resolution images [1]. The scripting property `minArea` is the smallest feature traced, in square pixels [3]. | Noise is an area: a 4-connected region of fewer than Noise pixels (of the image as traced) is merged into its neighbour, whatever its shape (a 1 × 16 bar and a 4 × 4 block go at the same value); a region of exactly Noise pixels stays; Noise 0 and 1 leave every region. Unchanged by the Paths and Corners mapping. |

Where the documentation is silent (the tolerance in pixels, the corner angle in degrees, the curve
between the ends), the numbers are VectorCraft's own choices, tuned on code-drawn and public-domain test
images in VectorCraft; they are not verified values from another application and do not claim to
match its output. No Adobe application or its outputs were observed for this follow-up. Compared
with current main, Paths 0 changes from 0.78 to 0.495 px; Paths 50 changes from about 0.3275 to
0.3102 px; Corners 75 changes from 48.75° to about 57.87°. Thus default output changes even though
the numeric defaults stay the same. Paths 100 remains 0.08 px and Corners 100 remains 35°.
Public documentation establishes the controls' direction, not these numeric mappings.
Flat Logo mode keeps its own mapping (Paths 0.24 px at 0 to 0.04 px at 100, Corners as before), which
already has these properties.

The cited control documentation does not specify these default/preset parameter sets. VectorCraft's Default (Black and White, Threshold 128,
Paths 50, Corners 75, Noise 25) and its presets are its own choices; the Black and White default with
Threshold 128 matches commonly published tutorials ([4], [5]).

Tests (`crates/trace/src/tests.rs`, code-drawn images only):
- `paths_and_corners_map_strictly_and_paths_100_is_the_tightest_fit`: for every integer 0–100, a
  higher Paths gives a strictly smaller tolerance and a higher Corners a strictly smaller corner angle;
  each control acts alone; the ends are 0.08 / 0.495 px and 35° / 99.2°.
- `paths_100_gives_the_most_anchors_and_paths_0_the_fewest`: on soft and hard discs and wavy and bumpy
  blobs, no Paths value 0–100 gives more anchors or segments than Paths 100, and Paths 0 no more
  segments than Paths 100 (sweeps rerun against the current fitter).
- `a_higher_paths_never_gives_fewer_anchors_on_these_shapes`: on the discs and the hard bumpy blob,
  anchors and segments never fall from one Paths value to the next. This cannot be guaranteed for
  every shape: the curve fitter is greedy, and on the soft wavy blob a looser fit can split a long
  span where a tighter one does not (current results and baseline are recorded in the linked quality report).
- `paths_100_strays_least_from_the_drawn_outline`: on anti-aliased shapes (whose pixel shape is the
  drawn outline), no Paths value 0–100 strays farther from it than Paths 100 does, and the mean
  distance falls from Paths 0 to 50 to 100. The largest distance does not fall at every single step,
  for the same reason as above.
- `a_higher_corners_never_gives_fewer_corner_points`: on regular polygons of 3 to 12 sides, corner
  points never fall over Corners 0–100 (at Paths 25, 50 and 75), and Corners 100 has more than 0.
- `noise_is_an_area_in_pixels_and_more_noise_never_keeps_more`: bars and blocks of 1 to 120 pixels,
  Noise 0–120: exactly the regions of at least Noise pixels are kept, and never more at a higher Noise.

Before and after on code-drawn shapes are in [image-trace-quality.md](image-trace-quality.md#paths-and-corners-as-documented).

Sources (public documentation; feature names and short phrases only):
1. Image Trace panel options: https://helpx.adobe.com/illustrator/desktop/manage-objects/traces-mockups-symbols/image-trace-panel-options.html
2. Image Trace: https://helpx.adobe.com/illustrator/using/image-trace.html
3. Scripting reference, TracingOptions (`pathFitting`, `cornerAngle`, `minArea`): https://ai-scripting.docsforadobe.dev/jsobjref/TracingOptions/
4. https://designbuckle.com/how-to-trace-an-image-in-illustrator
5. https://illustratorhow.com/image-trace

Regression coverage: `crates/ui-egui/src/panels/image_trace.rs` tests opening the panel,
a quick trace leaving it as it was, repeated opening, floating ownership, slider commits, visible threshold
changes, undo, and the Properties entry point. Tracing in the background:
`crates/ui-egui/src/tests/trace_runs.rs`. Tracer tests live in `crates/trace/src/tests.rs`.
