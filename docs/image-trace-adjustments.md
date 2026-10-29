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

**Document Library linked colors.** With Library › Document Swatches, with or without a color group,
a traced fill or stroke whose color comes from a global or spot swatch is linked to that swatch, and
one whose color comes from a tint swatch is linked to its base at the tint. Editing the swatch
recolors the trace. Colors from process swatches and from swatch libraries are not linked.

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

Regression coverage: `crates/ui-egui/src/panels/image_trace.rs` tests opening the panel,
a quick trace leaving it as it was, repeated opening, floating ownership, slider commits, visible threshold
changes, undo, and the Properties entry point. Tracing in the background:
`crates/ui-egui/src/tests/trace_runs.rs`. Tracer tests live in `crates/trace/src/tests.rs`.
