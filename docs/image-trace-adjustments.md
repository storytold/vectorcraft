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
| Paths | 0–100% | Higher values fit outlines more closely to the image. |
| Corners | 0–100% | Higher values preserve more sharp turns. |
| Noise | 1–100 px | Suppresses small connected regions; higher values remove more small details. |
| Method | Abutting / Overlapping | Chooses shared boundaries or stacked overlapping paths. |
| Create | Fills / Strokes | Creates filled areas, stroked lines, or both. |
| Ignore Color | Color picker | Leaves out the selected color (white by default). |

The built-in presets other than Auto Color and Flat Logo all use Paths 50, and so do their enhanced
versions. Among those presets, Corners is 100 in Technical Drawing, 75 in Default, Black and White
Logo, Line Art and Outline, and 50 in the other eight. Noise is 1 px in Technical Drawing, 5 in High Fidelity Photo and Black and White Logo, 8 in 6 Colors
and 16 Colors, 10 in Low Fidelity Photo, 15 in 3 Colors and Shades of Gray, 20 in Sketched Art, Line
Art and Outline, 25 in Default, and 90 in Silhouettes (see
[image-trace-quality.md](image-trace-quality.md#preset-paths-corners-and-noise)). High Fidelity Photo
traces with Palette › Full Tone at Color detail 85 and Method › Abutting (VectorCraft's own values; see
[image-trace-quality.md](image-trace-quality.md#high-fidelity-photo-in-full-tone)); Low Fidelity Photo
traces with a Limited palette of 20 colors, Overlapping.

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

**Keep Colors.** `"keepColors": ["#rrggbb", …]` in the params of `imageTrace.make` or
`imageTrace.makeAndExpand` traces only the palette colors within about 30 levels of a listed color,
the distance Ignore Color matches by, and leaves the other colors out. With Method Abutting, the paths
of the kept colors are the same as in a trace of every color. With Method Overlapping, a kept color's
paths reach under the kept colors above it in the stack and not under the colors left out, as with
Ignore Color. An empty list traces every color, and a value that is not a six-digit hex color is
refused. With Ignore Color as well, a kept color that Ignore Color matches is still left out. Flat
Logo does not use `keepColors`: it traces the colors named in `logoColors`, or the colors it finds.
The reply's `warnings` name each listed color that matches none of the colors the trace found. When
nothing is traced because no listed color matches, because Ignore Color leaves out every area of the
ones that match, or because the ones that match produce no paths, the first warning says so, and the
status bar shows it. In Flat Logo mode the
warning says that `keepColors` is not used.

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

**Ignore Color, background only.** Ignore Color leaves out every area of the chosen color. With
`"ignoreBackground": true` beside `"ignoreWhite": true` in the params of `imageTrace.make` or
`imageTrace.makeAndExpand`, it leaves out only the areas of that color that reach the image's edge,
joined through pixels above, below or to either side, and traces the areas of it that other colors
enclose: black line work inside a panel stays when the black around the panel is left out. Flat Logo
leaves out every white area whatever `ignoreBackground` is.

**Document Library** traces with the library's colors as they are. Each pixel takes the library
color nearest to it in lightness and hue, with a difference in hue or chroma counted three times
as much as one in lightness, so a gray or a muted brown takes a black, white or other neutral
library color rather than a saturated color of the same lightness.
**Abutting and Overlapping.** Abutting is the default Method, and the Default, Auto Color, High Fidelity
Photo, 3 Colors, 6 Colors and 16 Colors presets use it. With Abutting, two neighboring colors share one
exact edge. The canvas smooths the edges of each path on its own, so each of the two paths covers only part of the
pixels along that edge, and a thin line of the background can show between the colors. Web browsers
and PDF viewers usually draw an exported SVG or PDF file the same way and show the same line. Raster
exports (PNG, JPEG, TIFF and the other bitmap formats) with None anti-aliasing do not show it at any
size. Raster exports with Art Optimized anti-aliasing, the default, do not show it up to 10 million
pixels and 16,384 pixels on a side. A raster export shows the line as the canvas does when it uses
Type Optimized anti-aliasing, or when it uses Art Optimized and is larger than either limit.
Overlapping stacks the colors instead: a lower color continues under the edge of the color above it,
and no background shows along that edge. With Fills and without Create Strokes, Overlapping also
follows a fill rule of VectorCraft's own: each shape is filled whole, and the area of other colors
that a shape encloses is traced again as shapes of its own and stacked above it. Each enclosed
outline is then fitted once. The [Image Trace presets](https://helpx.adobe.com/illustrator/desktop/manage-objects/traces-mockups-symbols/image-trace-presets.html)
page describes Overlapping as stacking shapes where it can and overlapping their edges elsewhere. A
shape keeps a hole where the enclosed area holds a transparent pixel or a color that Ignore Color or
Keep Colors leaves out. A hole smaller than Noise is filled with the shape's own color, as before.
The pixels of an area inside other areas are scanned once more for each area around it, within a
work budget of VectorCraft's own: four times the image's pixels (counted as at least 65,536) times
its colors, with each area costing its bounding box times the colors in it. An area that costs more
than what is left of the budget keeps its holes and is not charged, so a smaller area after it can
still be filled.
With Create Strokes, or without Fills, shapes keep their holes.

**Flat Logo with named colors.** With `logoColors` named, Flat Logo decides whether the image is flat
art by the image's own colors. It finds them the same way as when no colors are named, and looks for as
many colors as are named, with white, or as the Colors setting gives, whichever is more. How far the named colors
are from the image's colors does not change whether the image is refused. While no more colors are
named than the Colors setting, an image is refused exactly when it is refused without named colors.
The trace then uses only the named colors, and white while white is a color of its own. An area of a
color that was not named is traced too: each of its pixels goes to the named color, white or
transparent that has the largest share in the blend of them that comes closest to the pixel.

**Flat Logo with a color between two others.** When a traced color lies between two others, as a gray
lies between black and white, an anti-aliased edge pixel between the outer two is also a blend of the
middle color and one of them. Flat Logo then reads the pixel as a blend of the outer two when the pixel
lies on an edge between them and that blend lies nearer the pixel than the blend with the middle color.
The pixel lies on such an edge when, along the line between the outer two colors, it reaches a pixel
darker than the middle color in at most 5 steps, each to the neighbor that lies farthest toward the
darker color while the pixels keep getting darker, and reaches a pixel lighter than the middle color the
same way. When the first step toward one outer color comes to a pixel that is less than half as far from
that color as from any other traced color, the path toward the other outer color stops at the first
pixel within 4 levels (out of 255) of the middle color that it comes to before it gets past the middle
color. A blend lies as near the pixel as the farther of its colors' nearest flat areas within 6 pixels.
The blend of the outer two also lies as near as the larger of the two step counts, when that number is
smaller. Each path counts its steps to the first pixel past the middle color that is less than half as
far from that path's outer color as from any other traced color.

A black disc beside a gray square on white traces with one gray shape, the square, and no gray slivers
around the disc, when its edges are sharp or blurred by up to 2 pixels, with the colors found or named.
A light gray (230) blurred by 1.5 pixels leaves up to 4 slivers of up to 3 pixels. With the colors named
and edges blurred by 3 pixels, a gray of 128 is still one shape, a gray of 200 leaves 4 slivers with no
area, and a light gray leaves 8 slivers of up to 21 pixels. An image with noise of a level or more in its
pixels can still leave gray slivers along the disc.

With sharp edges, a gray stroke two or three pixels wide beside black has no darker pixel next to it and
stays gray, and so does a gray band along black. The counts below are of pixel centers that trace gray.
With edges blurred by 0.6 pixels, as by ordinary anti-aliasing, a gray band 3 to 8 pixels wide along
black traces gray at no fewer than 4 of every 5 of its pixel centers. On edges blurred by a pixel or
more, gray near black traces gray at fewer of its pixel centers. A soft-edged gray band along black
traces as black and white, with gray at no more than 1 of every 16 of its pixel centers, when it is up
to 4 pixels wide at blurs of 1.5 and 2 pixels, up to 3 pixels wide at a blur of 3 pixels, or 2 pixels
wide at a blur of 1 pixel. A wider band loses some or most of its gray next to black. Bands 3 and 4
pixels wide blurred by 1 pixel trace gray at about 60 of their 336 pixel centers (over grays of 230, 200
and 128), bands 4 pixels wide blurred by 3 pixels at about 39 of 192, and a band 8 pixels wide at no
fewer than half of its pixel centers at blurs of 1 to 3 pixels. A light gray stroke 2 pixels wide,
blurred by 1.5 pixels, traces gray at about 10 of the 34 pixel centers along its middle when it lies 2
pixels from black and at about 25 when it lies 4 pixels away. At 6 or 8 pixels from black it traces gray
at all 34: its pixels do not reach black in 5 steps and are not read as an edge of black and white. A
soft-edged gray stroke 2 or 3 pixels wide running straight out of black, blurred by 1 or 1.5 pixels,
loses 3 or 4 of the 16 pixel centers along its middle nearest black. Blurred by 2 pixels, one 3 pixels
wide loses 2 or 3 of them, and one 2 pixels wide traces as black and white along its whole length. A
soft-edged gray stroke about two pixels wide lying along black traces as black and white, and one about
three pixels wide lying along black can lose most or all of its gray when it is short (12 pixels long or
less). A line of gray one
pixel wide between black and white stays gray when at least 13 of its pixels touch: such a line and a
straight black-to-white edge whose pixels are as light as the gray give the same pixels, and both
trace as gray. A one-pixel gray line between black and white that is shorter than 13 pixels, falls
between pixels or is blurred traces as black and white. A flat area whose color lies between two named colors, as when the colors are named
some levels off, counts as holding the named colors it is a blend of, and so do the pixels of exactly
its color that touch it.

The same holds when the left-out color lies beyond both colors of the edge, as a pale gray ground lies
beyond a gray panel and a charcoal disc: an edge pixel between the panel and the disc is also a blend of
the ground and the disc. Flat Logo reads it as a blend of the panel and the disc when, along the line
through the three colors, the pixel has a neighbor on each side of it, no neighbor lies past the panel
toward the ground, and a chain of neighbors, each nearer the panel's color than the one before, leads
from the pixel, in at most 6 steps, to the panel's color or to a flat area of the panel. The same rule
applies to a left-out color darker than both colors of the edge, and to colors that are not grays. A
charcoal disc on a gray panel on a paler ground then traces with no slivers of the ground around it when
its edges are sharp or blurred by 0.7 pixel. An edge blurred by a pixel or more can still leave slivers
of the ground. The pixels of a pale line on the disc, whether one pixel wide, blurred over three pixels
or falling between two pixels, and the edge of a gap of the ground's color between the disc and the
panel keep the ground in their blends.

**Large images.** An image of more than 1500 pixels on a side is traced as any other, only more
slowly, and the trace says so: `imageTrace.make` returns the note in `warnings` and the status bar
shows it (the [results optimization](https://helpx.adobe.com/illustrator/using/image-trace-results-optimization.html)
page describes the same warning for images over 1500 px). An image of more than 64 megapixels (up to
192) is traced from a copy scaled down to 64 megapixels, and the trace says that too. Flat Logo traces at
most 1024 pixels on a side: a larger decoded raster is averaged down in whole-number blocks first (a 1025-pixel
image is traced at 513, and a 16,000 × 12,000 image, decoded at 9,459 × 7,094, is traced at 946 × 710), and the trace names both sizes in `warnings`. Flat Logo gives that note in place
of the slower-trace note for an image over 1500 pixels and in place of the 64-megapixel note. An image at most
1024 pixels on a side is traced at its own size, also when the named `logoColors` differ slightly from the
image's colors. Only an image with more than 150,000 different pixel values to unmix (values that are
not exactly one traced color, and values that are, at pixels outside a flat area) is averaged down
further: the factor doubles until at most 150,000 remain or the working copy has at
most 16,384 pixels (at most six times). A trace that would
make more than 8,000,000 anchor points (2,000,000 on the web) is refused to protect memory, with advice:
raise Noise, lower Paths or Colors, or pick a lower-fidelity preset. A trace keeps
about 90 bytes an anchor; a 4000 × 3000 photo at Limited 64 colors, Paths 90, Corners 25 and Noise 4
makes about 1.9 million.
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

Regression coverage: `crates/ui-egui/src/panels/image_trace.rs` tests opening the panel,
a quick trace leaving it as it was, repeated opening, floating ownership, slider commits, visible threshold
changes, undo, and the Properties entry point. Tracing in the background:
`crates/ui-egui/src/tests/trace_runs.rs`. Tracer tests live in `crates/trace/src/tests.rs`.
