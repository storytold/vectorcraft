# 3D lighting performance

Measured 2026-10-17 for PR #2706 on one macOS machine.

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

Source gradients retain their independently placed per-facet vector compositor. They remain a more expensive case. Patterns retain their neutral shading approximation. White-light Inflate uses the original polygon shading, paint composition and default seam joins; the seam width retains its 0.05 to 0.75 pt bounds.

Geometry/effects tests cover legacy polygons and seam bounds, shared smooth normals, finite ramps, holes and constant group/clip counts. Engine tests cover the batched SVG round trip, vector PDF, expansion, undo, compound holes and surface opacity.

## Legacy white-light comparison

Four original synthetic scenes were exported at 1200 x 1200 from unchanged main `f7de3d6a` and the updated implementation. The PNG files match byte for byte in all four cases: the legacy white-lit ellipse above; the same ellipse with both sides, hollow cap, twist 60 degrees, taper 70%, perspective 80 degrees and view Y rotation 145 degrees; an unshaded ellipse; and the transparent source gradient above under the legacy white light. The command recipe was also checked against the measured colored fixture's image pixels.
