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
middle color and one of them. Flat Logo then reads the pixel as the blend of the colors that lie nearest
it in the flat areas within 6 pixels, when the pixel has a neighbor darker and a neighbor lighter than the
middle color, along the line between the other two. A black disc beside a gray square on white traces
with one gray shape, the square, and no gray slivers around the disc, when its edges are sharp or blurred
by up to about half a pixel. An edge blurred by a pixel or more can still leave gray slivers along the
disc, and so can an image with noise of a level or more in its pixels. A gray stroke two or three pixels
wide beside black has no darker pixel next to it and stays gray. A soft-edged gray stroke about two
pixels wide that touches black traces as black and white, and one about three pixels wide that touches
black can lose most or all of its gray when it is short (12 pixels long or less). A line of gray one
pixel wide between black and white stays gray when at least 13 of its pixels touch: such a line and a
straight black-to-white edge whose pixels are as light as the gray give the same pixels, and both
trace as gray. A one-pixel gray line between black and white that is shorter than 13 pixels, falls
between pixels or is blurred traces as black and white. A flat area whose color lies between two named colors, as when the colors are named
some levels off, counts as holding the named colors it is a blend of, and so do the pixels of exactly
its color that touch it.

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

Regression coverage: `crates/ui-egui/src/panels/image_trace.rs` tests opening the panel,
a quick trace leaving it as it was, repeated opening, floating ownership, slider commits, visible threshold
changes, undo, and the Properties entry point. Tracing in the background:
`crates/ui-egui/src/tests/trace_runs.rs`. Tracer tests live in `crates/trace/src/tests.rs`.
