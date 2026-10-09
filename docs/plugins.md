# Plug-ins: the WebAssembly plug-in API (ABI v1)

Vector W3K2 does not host native plug-ins built for other illustration apps: that needs unsafe FFI into
arbitrary machine code and cannot work in the browser build. Instead, plug-ins are **WebAssembly modules**
run in a sandbox. One `.wasm` file works on macOS, Windows, Linux and the web. The design follows PhotoCraft's
plug-in API (same sandbox, same manifest and parameter schema), extended for vector documents.

There are two kinds of plug-in:

- **Object filters** read the selected paths and compound paths (geometry, fills, strokes, gradients, opacity)
  and return the objects to keep, change or add. A run is one undo step. Use them for recolouring, path
  clean-up, halftones, generative patterns and the like.
- **Live effects** rewrite one object's geometry each time it is drawn, from its appearance stack, like the
  built-in Distort & Transform effects. The art stays editable: move or reshape it and the effect follows.

This page is the contract for plug-in authors and the reference for the host (`crates/plugins`, the
`plugin.*` commands in `crates/engine/src/cmd/plugin.rs`).

## Using plug-ins

| Command | Params | What it does |
|---|---|---|
| `plugin.list` | `{}` | `{"plugins": [{id, name, version, kind, author, description, params, paramsSchema, effect?, size, source}], "folder": report}` |
| `plugin.info` | `{"id"}` | One plug-in: the `plugin.list` entry plus `manifest`, `defaults` and `lastError` (a live effect's last failure). |
| `plugin.run` | `{"id", "params"?: {…}, "ids"?: [...]}` (plug-in params may also be top-level keys) | Runs an object filter on the selection (or `ids`). One undo step; the result is selected. |
| `plugin.install` | `{"path": "/x/y.wasm"}` (desktop) or `{"dataBase64": "…", "name"?}`, `"replace": bool = true` | Validates and installs a module; returns its `plugin.list` entry. |
| `plugin.remove` | `{"id"}` | Uninstalls. |
| `plugin.reload` | `{"path"?: "folder"}` (default: the preference folder) | Installs every `*.wasm` in a folder; a bad module is reported and skipped. |

Live effects are applied like any effect: `effect.apply {"effect": "plugin.<id>", "params": {…}}`, edited with
`effect.setParams`, listed by `effect.list` (menu `Effect › Plug-ins`) and baked by Object › Expand Appearance.

In the app, installed filters appear under **Object › Plug-ins**, next to **Install Plug-in…** and **Reload
Plug-ins**; live effects under **Effect › Plug-ins** and in the Appearance panel's effect menu. A plug-in with
parameters gets a dialog generated from its schema (numbers, whole numbers, checkboxes, dropdowns) with a live
preview; without parameters it runs at once. **File › Open** of a `.wasm` file installs it too (on the web this is
how plug-ins get in).

**Preference folder.** Edit › Preferences › Performance & Storage › *Additional Plug-ins Folder*: when set, every
`*.wasm` in that folder (not recursive) is installed at startup and whenever the preference changes (desktop builds;
the web build installs from bytes).

Plug-ins are installed **per process**, not per document. Documents store a live effect as an ordinary effect record
(`{"id": "plugin.<id>", "params": {…}}`): while the plug-in is installed it is drawn, exported and expanded; without it
the object is drawn as if the effect weren't there, and the record is kept for when the plug-in comes back.

## The sandbox

The host is [`wasmi`](https://github.com/wasmi-labs/wasmi), a pure-Rust WebAssembly interpreter
(MIT OR Apache-2.0). It builds for `wasm32-unknown-unknown` too, so plug-ins also run in the web build. Limits
(`vectorcraft_plugins::Limits`, defaults):

| Limit | Default | On violation |
|---|---|---|
| Host imports | none: no WASI, file system, network, clock or randomness | module rejected at install |
| Module size | 32 MiB | rejected |
| Linear memory | 512 MiB per instance (input and parameters: at most half) | `memory.grow` returns -1; oversized input or initial memory fails |
| Output | 64 MiB of JSON | `Limit("output size")` |
| Instructions (fuel) | 50 M per call + 4,000 per input byte for `vc_run` | `Limit("instruction budget")` |
| Wall clock | 60 s per filter run, 5 s per live-effect run (desktop; checked every 20 M instructions) | `Limit("time budget")` |
| Call depth | 1,024 frames | trap |
| Module shape | wasmi's strict limits (≤ 10,000 functions, 1 memory, …) | rejected |

Every run gets a **fresh instance**, so no state survives between calls. Traps, exhausted budgets, malformed
modules, bad manifests, out-of-range pointers, negative return codes and invalid output all become errors, and the
document is left untouched: a filter's output is checked completely before one edit applies it. A live effect that
fails draws the object's geometry unchanged; `plugin.info` reports why (`lastError`).

wasmi is built with its *portable* (loop) dispatcher: the default tail-call dispatcher relies on LLVM
sibling-call optimisation, and when that doesn't happen a long-running module overflows the host stack, which aborts
the process instead of returning an error.

Drawing evaluates effects often, so live-effect results are cached by plug-in installation, parameters and input
geometry: a plug-in runs again only when one of them changes.

## ABI v1

A plug-in is a core WebAssembly module (MVP + the usual post-MVP features: bulk memory, sign extension, multi-value,
saturating conversions; no SIMD, no memory64) with **no imports** and these exports. Integers are `i32` unless noted;
pointers are offsets into the module's memory.

| Export | Signature | Meaning |
|---|---|---|
| `memory` | memory | The module's linear memory. |
| `vc_abi_version` | `() -> i32` | Must return `1`. |
| `vc_manifest` | `() -> i64` | `(len << 32) \| ptr` of the UTF-8 manifest JSON (≤ 64 KiB). |
| `vc_alloc` | `(size) -> ptr` | A block of `size` bytes the host may write, or `0` on failure. Never freed: each run gets a fresh instance. |
| `vc_run` | `(input, input_len, params, params_len) -> i64` | Runs the plug-in on the input JSON with the parameters JSON; returns `(len << 32) \| ptr` of the output JSON, or a negative error code. |

### Manifest

```json
{
  "id": "org.example.tidy",
  "name": "Tidy Paths",
  "version": "1.0.0",
  "kind": "filter",
  "author": "Example",
  "description": "What it does.",
  "params": {
    "amount": {"type": "number", "min": 0, "max": 100, "default": 50},
    "steps":  {"type": "int", "min": 1, "max": 16, "default": 4},
    "closed": {"type": "bool", "default": false},
    "mode":   {"type": "choice", "options": ["soft", "hard"], "default": "soft"}
  }
}
```

- `id`: 1–64 characters from `A-Z a-z 0-9 . _ -` (reverse-DNS recommended). Installing a module with the same id
  replaces the old one. A live effect's effect id is `plugin.<id>`.
- `name`: the menu label (1–64 printable characters). `kind`: `"filter"` (object filter) or `"effect"` (live effect).
- `params`: in dialog order. Names are `[A-Za-z0-9_]`, not starting with `_`, and not `id`, `ids`, `params` or
  `preview`. The host validates the user's values before calling: missing values take the default, numbers are
  clamped to `min..max` (ints rounded), a wrong type or an unknown choice is an error.

### Input

`input` is UTF-8 JSON: `{"objects": [...]}`. An object filter gets the selected paths and compound paths in paint
order (bottom first): the selected ones and those inside selected groups and layers; hidden, locked, guide and clipping
paths are left out, and so are other kinds of object (type, images, symbols, live blends…), which a filter never
touches. With nothing selected the list is empty (a generator then adds its own objects). A live effect gets one
object, holding only geometry (`type`, `path`, `fillRule`, `bounds`).

```json
{"objects": [
  {"id": 12, "type": "path", "name": "Leaf",
   "path": {"subpaths": [{"closed": true, "anchors": [
     {"p": [10, 10]},
     {"p": [60, 10], "in": [45, 0], "out": [75, 20], "kind": "Smooth"},
     {"p": [35, 60]}]}]},
   "fillRule": "nonzero",
   "fills": [{"type": "solid", "color": {"model": "rgb", "r": 0.9, "g": 0.2, "b": 0.1}}],
   "strokes": [{"paint": {"type": "gradient", "gradient": {"kind": "Linear", "stops": [
       {"offset": 0, "color": {"model": "gray", "k": 0}},
       {"offset": 1, "color": {"model": "cmyk", "c": 0, "m": 0.5, "y": 1, "k": 0}}]}, "angle": 0},
     "width": 2}],
   "opacity": 1,
   "bounds": [10, 0, 75, 60]}
]}
```

- `type`: `"path"` or `"compound"` (a compound path: `path` holds its members' subpaths, painted as one).
- `path`: Vector W3K2's own path form. Each subpath is a run of anchors; `p` is the anchor, `in` / `out` its handles
  (absent: no handle), `kind` `"Smooth"` when the handles stay collinear. Coordinates are points, y down.
- `fills`, `strokes`: the object's fills and strokes in paint order (bottom first); `null` is no paint. Paints use the
  native file format's form: `{"type": "solid", "color", "swatch"?, "tint"?}`, `{"type": "gradient", "gradient":
  {"kind": "Linear"|"Radial"|"Freeform", "stops": [{offset, color, opacity, midpoint}]}, "geom"?, "angle",
  "freeform"?}`, `{"type": "pattern", "pattern", "xf"?}`. Colours are `{"model": "rgb", r, g, b}`, `{"model": "cmyk",
  c, m, y, k}`, `{"model": "gray", k}` (ink: 0 white, 1 black) or `{"model": "lab", l, a, b}`; components 0–1 except
  Lab's.
- `bounds`: `[x0, y0, x1, y1]`, the geometric bounds (read only).

### Parameters

`params` points at `params_len` bytes of UTF-8 JSON: the validated parameters, plus `_context`:

```json
{"amount": 50, "mode": "soft",
 "_context": {"mode": "filter", "bounds": [10, 0, 75, 60], "artboard": [0, 0, 612, 792]}}
```

`mode` is `"filter"` or `"effect"`; `bounds` the selection's geometric bounds (a live effect: the reference box of the
effect, the object's bounds or those of the previous effect's result); `artboard` (filters) the artboard under the
selection's centre, else the first.

### Output

The output is UTF-8 JSON in the input's shape: `{"objects": [...]}`. To decline with a message for the user, return
`{"error": "Select two paths."}`.

For an **object filter**, the output is the new state of the input objects:

- An object with the `id` of an input object updates it; keys it leaves out keep their value (`{"id": 12}` keeps the
  object as it is). `fills` and `strokes` set the object's fills and strokes in order: extra entries add fills (above
  the last one) or strokes, missing ones are removed; each stroke keeps its weight unless `width` is given (a new one: 1 pt).
- An object without an `id` is new and needs a `path`. It goes just above the object listed before it, or, listed
  before any input object, just below the first one listed after it. New objects take a path's defaults: no fills or
  strokes, full opacity.
- Input objects missing from the output are deleted. The order of the input objects themselves doesn't change.
- `type` turns a path into a compound path (one member per subpath) or back; `name: ""` clears the name.

The host validates everything before the document changes: coordinates must be finite and within ±4,000,000 pt, at
most 100,000 objects and 4,000,000 anchors, 256 fills, strokes or gradient stops; colours, opacities and offsets are
clamped into range, stroke weights to 0–1,000 pt; a pattern must exist in the document. A paint returned unchanged
keeps its swatch link; any other paint is unlinked.

For a **live effect**, the geometry of every returned object's `path` (their subpaths, in order) replaces the
object's; the rest is ignored (the object keeps its own appearance). An empty list removes the geometry.

## Writing a plug-in in Rust

[`crates/plugins/example`](../crates/plugins/example) is a complete object filter, **Desaturate**: it moves every
colour of the selection towards grey, in each colour's own model, using `serde_json` (no imports, no WASI). Build it
with:

```sh
rustup target add wasm32-unknown-unknown
crates/plugins/example/build.sh      # cargo build --release --target wasm32-unknown-unknown
```

The script also refreshes `crates/plugins/tests/fixtures/desaturate.wasm`, the test fixture built from that source.
The crate is not a workspace member. Any language that targets core WebAssembly works (C, Zig, AssemblyScript,
WAT…), as long as the module has no imports: in Rust, `std` on `wasm32-unknown-unknown` is fine as long as it never
touches WASI. `vectorcraft_plugins::wat::module` is the skeleton of a plug-in in WebAssembly text, which the tests
build their plug-ins from.

## Performance

The interpreter costs roughly 10–100× native speed per instruction. Filters handle a selection in one call, so a
plug-in that parses and writes JSON with `serde_json` takes milliseconds for hundreds of paths. Live effects run once
per change of their input (results are cached), not per frame.

## Not in v1

- Tool plug-ins (pointer events on the canvas, a guide line or rectangle, geometry back), panel plug-ins, host
  callbacks (progress, cancel), plug-in file formats.
- Live effects that change paint (they reshape geometry; an object filter recolours), and filters on type, images or
  symbols.
- Apply Last Effect and Last Effect work for plug-in effects; there is no "last filter" for object filters yet.
