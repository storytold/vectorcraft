# 3D lighting performance

Measured 2026-10-17 for PR #2706 on one macOS machine (an Apple M1 Pro, below); the round seam joins on 2026-11-12 on another (an Apple M4 Pro, in their own section).

## Method

- Apple M1 Pro, 10 cores, 16 GiB RAM, macOS 26.5.1 (arm64).
- `cargo build --release -p vectorcraft-cli`, the workspace release profile (thin LTO, one codegen unit), without bundled fonts.
- Before: published PR head `f63fafdb`. After: the reviewed implementation rebased on main `f7de3d6a`, with legacy white-light Inflate restored and colored solid-fill channel groups batched per surface. The main update is included in this comparison.
- Both executables were retained separately. No compilation ran during measurement. Three paired repetitions alternate before/after for each fixture. Each `bench` invocation warms its renderer once, then averages five frames for each thread setting. The table reports the median of the three reported averages.
- `bench` renders a 400 x 400 pt artboard at 1200 x 1200 pixels, fitting it to 95% of the viewport. The scene contains one white-filled ellipse, centered at (200, 200), with radii 110 and 140 pt, no stroke, Inflate volume 50%, depth 0, solid cap, one side, view rotation (-20, 25, 0), no perspective, and 15% neutral ambient.
- Two lights: orange `#ff8040`, intensity 70%, rotation 135 degrees, height 35 degrees, softness 15%; blue `#4080ff`, intensity 65%, rotation -35 degrees, height 25 degrees, softness 10%. Both are in front.
- The one-light case uses the legacy fields: `#ff8040`, intensity 80%, rotation 125.26438968275465 degrees, height 30 degrees, softness 0%. The white case changes its color to `#ffffff`.
- The source-gradient case uses the same two lights with a linear source gradient from (90, 200) to (310, 200), orange at opacity 0.45 and blue at opacity 0.7.
- `perf` covers the standard 50,000-path synthetic budget scene. It was also run three times per executable, alternating before/after. Its table reports medians.

## Reproduce

Create the two-light scene using the updated CLI, then use the same saved file with both executables:

```sh
vectorcraft-cli run \
  --cmd file.new --params '{"width":400,"height":400}' \
  --cmd document.setup --params '{"backgroundContents":"white"}' \
  --cmd path.create --params '{"d":"M310.0 200.0C310.0 277.3198649763111 260.7513224813873 340.0 200.0 340.0C139.2486775186127 340.0 90.0 277.3198649763111 90.0 200.0C90.0 122.6801350236889 139.2486775186127 60.0 200.0 60.0C260.7513224813873 60.0 310.0 122.6801350236889 310.0 200.0Z"}' \
  --cmd paint.setFill --params '{"color":"#ffffff"}' \
  --cmd paint.setStroke --params '{"none":true}' \
  --cmd effect.apply --params '{"effect":"threeD.inflate","item":null,"params":{"rotationX":-20,"rotationY":25,"rotationZ":0,"ambient":15,"volume":50,"depth":0,"cap":true,"bothSides":false,"lights":[{"color":"#ff8040","intensity":70,"rotation":135,"height":35,"softness":15},{"color":"#4080ff","intensity":65,"rotation":-35,"height":25,"softness":10}]}}' \
  --cmd document.save --params '{"path":"/tmp/inflate-two-lights.vectorcraft"}'

vectorcraft-cli bench /tmp/inflate-two-lights.vectorcraft --size 1200x1200 --iters 5
vectorcraft-cli perf
```

## Measurements

Only the legacy white-light row and the white-light comparison below are affected by the round seam joins of 2026-11-12: they were measured on 2026-10-17, and colored-light seams were already round; see [Round seam joins](#round-seam-joins).

Milliseconds per frame, medians of three five-frame averages. `8` is the CLI default thread count on this machine; `0` selects the single-thread renderer.

| Inflate scene | Before, 8 | After, 8 | Before, 0 | After, 0 |
|---|---:|---:|---:|---:|
| Two colored lights, solid fill | 136.2 | 96.8 | 147.2 | 100.6 |
| One colored light, solid fill | 94.4 | 96.7 | 107.0 | 100.0 |
| Legacy white light, restored polygon shading | 12.3 | 4.1 | 27.3 | 9.5 |
| Two colored lights, transparent source gradient | 145.6 | 146.8 | 156.9 | 157.0 |

The two-light solid-fill case takes 28.9% less frame time at the default thread count and 31.7% less in the single-thread renderer. The one-light case is 2.4% higher with eight threads and 6.5% lower single-threaded. The source-gradient case retains its previous per-facet compositor and changes by less than 1% in these medians. Restoring legacy white-light polygons also removes the extra smooth triangles from that path.

All six `perf` runs exited successfully, with no busy-machine warning and every individual budget row passing. The reported one-minute load averages were 4.5 to 5.9 on ten cores. Median milliseconds:

| Standard 50,000-path workload | Before | After | Budget |
|---|---:|---:|---:|
| render: fit page, 2880×1800 | 47.15 | 47.80 | 100 |
| render: background refresh after a pan | 48.18 | 46.71 | 100 |
| render: 400% zoom, 2880×1800 | 13.45 | 13.68 | 16 |
| hit test (per click) | 0.43 | 0.42 | 2 |
| save .vectorcraft | 52.78 | 52.15 | 300 |
| open .vectorcraft | 143.84 | 142.55 | 300 |
| export SVG | 207.37 | 206.25 | 500 |
| Pathfinder Unite, 1,000 paths | 43.18 | 43.29 | 150 |
| handle drag, 5,000-anchor path (per move) | 0.94 | 0.84 | 8 |
| segment drag, 5,000-anchor path (per move) | 0.87 | 0.92 | 8 |

The colored fixture uses the same projected mesh before and after. The solid-fill batch reduces compositor groups and clips; it retains three sets of channel facet paths. Smooth vector export still has many gradient elements.

## Rendering structure

For a smooth colored solid fill, all facets share one compound coverage clip and three isolated channel groups. Each channel has an opaque black backdrop; Screen combines the disjoint RGB channels and Multiply applies the source color. Face order is retained in each channel. Surface opacity and outer coverage apply once. The group/clip counts are independent of triangle count, and export remains vector artwork.

Source gradients retain their independently placed per-facet vector compositor. They remain a more expensive case. Patterns retain their neutral shading approximation. White-light Inflate uses the original polygon shading and paint composition; the seam width retains its 0.05 to 0.75 pt bounds. Its seams kept the default Miter Join until 2026-11-12, when every 3D seam took round joins (a mitered seam drew thorns past facets seen almost edge-on), so white-light output is no longer identical to the comparison below.

Geometry/effects tests cover legacy polygons and seam bounds, shared smooth normals, finite ramps, holes and constant group/clip counts. Engine tests cover the batched SVG round trip, vector PDF, expansion, undo, compound holes and surface opacity.

## Legacy white-light comparison

Four original synthetic scenes were exported at 1200 x 1200 from unchanged main `f7de3d6a` and the updated implementation. The PNG files matched (2026-10-17) byte for byte in all four cases: the legacy white-lit ellipse above; the same ellipse with both sides, hollow cap, twist 60 degrees, taper 70%, perspective 80 degrees and view Y rotation 145 degrees; an unshaded ellipse; and the transparent source gradient above under the legacy white light. The command recipe was also checked against the measured colored fixture's image pixels.

## Round seam joins

Measured once, on 2026-11-12, on another machine (an Apple M4 Pro with 14 cores) that was not idle: its one-minute load average was 6 to 13, so these are one loaded run, not the idle measurements above. Two `vectorcraft-cli` builds, each in release (`cargo build --release -p vectorcraft-cli`): main at `6d6bbff53`, and that commit with the round seam joins, run in turn, three times each. `bench` fits the artboard to 95% of 2880 x 1800 pixels, draws one frame to warm each renderer and reports the average of the frames after it: `threads 8` with the renderer's default workers (one per core it may use, up to 8, one core left free), `threads 0` on the calling thread alone. A cold frame is a whole `run` that opens the scene, builds its 3D art and draws it once.

White light: extruded type with a cove bevel, an Inflate, an Extrude & Bevel (Classic) star with a classic bevel and a Revolve, no strokes (6,283 nodes drawn). The median of three runs of 20 frames, on 8 threads and on one: main 8.0 ms and 18.3 ms; with the round joins 7.5 ms and 17.7 ms. Cold frames took 0.08 s of processor time on each. The differences are within noise for one loaded run. Not measured: `vectorcraft-cli perf`, the legacy white-light Inflate row above (still the 2026-10-17 figures), frames whose 3D art is rebuilt (while dragging a 3D slider, say) apart from the cold run, and SVG/PDF exports.

```
{"command":"file.new","params":{"width":800,"height":600}}
{"command":"text.create","params":{"x":40,"y":120,"text":"SAMPLE TYPE","size":64,"font":"Source Sans 3","color":"#3366cc"}}
{"command":"effect.apply","params":{"effect":"threeD.extrude","item":null,"params":{"depth":40,"rotationX":-35,"rotationY":25,"taper":47.5,"bevel":"cove","bevelHeight":2}}}
{"command":"path.create","params":{"d":"M100 300C100 245 145 200 200 200C255 200 300 245 300 300C300 355 255 400 200 400C145 400 100 355 100 300Z"}}
{"command":"paint.setFill","params":{"color":"#cc6633"}}
{"command":"paint.setStroke","params":{"none":true}}
{"command":"effect.apply","params":{"effect":"threeD.inflate","item":null,"params":{}}}
{"command":"path.create","params":{"d":"M400 200L440 280L520 290L460 340L480 420L400 380L320 420L340 340L280 290L360 280Z"}}
{"command":"paint.setFill","params":{"color":"#33aa66"}}
{"command":"paint.setStroke","params":{"none":true}}
{"command":"effect.apply","params":{"effect":"threeD.classicExtrude","item":null,"params":{"depth":50,"bevel":"classic","bevelHeight":4,"rotationX":80,"rotationY":10}}}
{"command":"path.create","params":{"d":"M600 200L640 200L660 300L640 400L600 400Z"}}
{"command":"paint.setFill","params":{"color":"#aa33aa"}}
{"command":"paint.setStroke","params":{"none":true}}
{"command":"effect.apply","params":{"effect":"threeD.revolve","item":null,"params":{}}}
```

The type is set in Source Sans 3, which a build has only with craft-fonts (`CRAFT_FONTS_DIR`, see `docs/development.md` › Fonts); without it the type falls back to another face. To reproduce, save the commands as `scene.jsonl` and, with each build:

```
vectorcraft-cli run --commands scene.jsonl --export scene.vectorcraft
vectorcraft-cli bench scene.vectorcraft --iters 20
/usr/bin/time -p vectorcraft-cli run --in scene.vectorcraft --export scene.png
```
