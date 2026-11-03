# Installable Tangent Circle Tool Design

## Intent

Ship the circle-drag tangency feature as an installable VectorCraft WebAssembly plug-in. Once
installed, the plug-in appears as another item in the Ellipse tool's toolbar flyout and lets the
user drag any circle into exact external tangency with two nearby circles.

## Goals

- `Tangent Circle.wasm` installs through the existing Install Plug-in flow.
- Installing it adds a `Tangent Circle` item beside the Ellipse tool.
- The plug-in can be removed and disappears from the tool group.
- Dragging keeps the moved circle's radius and snaps its center to a two-circle external tangency.
- Plain and live circles participate; hidden, locked, guide, clipping, template and moving circles
  do not.
- The plug-in is sandboxed by the existing WebAssembly host and gets no filesystem, network or
  clock access.
- The host extension and plug-in source live in the custom feature branch so later upstream source
  updates can replay them.

## Non-goals

- Tangency to ellipses, arbitrary curves or lines.
- Resizing the dragged circle.
- Changing the reference circles.
- Compatibility with the unmodified official VectorCraft binary. Tool plug-ins require the small
  host extension in this branch.

## Architecture

### Plug-in ABI

Add `Kind::Tool`. A tool manifest adds `toolGroup` (initially only `ellipse`) and `icon` (default
`tool-ellipse`). Tool plug-ins reuse `vc_abi_version`, `vc_manifest`, `vc_alloc`, `vc_run` and the
existing sandbox and fuel limits.

For a tool, `vc_run` receives UTF-8 JSON:

```json
{
  "event": {"kind": "down", "pos": [x, y], "mods": {}},
  "state": null,
  "selection": [12],
  "circles": [{"id": 12, "center": [x, y], "radius": 20}],
  "zoom": 1.0,
  "tolerance": 4.0
}
```

It returns:

```json
{
  "state": {},
  "actions": [
    {"type": "exec", "command": "select.set", "params": {"ids": [12]}},
    {"type": "begin", "label": "Move"},
    {"type": "preview", "command": "object.transform", "params": {"matrix": [1,0,0,1,0,0]}},
    {"type": "commit"}
  ],
  "guides": [
    {"type": "line", "a": [x, y], "b": [x, y]},
    {"type": "label", "p": [x, y], "text": "tangent"}
  ],
  "error": "optional message"
}
```

Output is validated before use: finite coordinates, short labels/text, a bounded action count, and
only the documented action types. Actions still pass through the engine's normal validation.

### Host tool

The tools crate gains a `PluginTool` that holds the installed plug-in, the plug-in's opaque `state`,
and the latest guides. It serializes Selection-tool-style pointer events and a compact editable
circle list, calls the sandbox, translates returned actions and exposes the returned guides and
cursor.

The plug-in library gains a circle-context collector shared by tests and the host. It recognizes a
whole live ellipse with equal document radii, or a closed plain path whose samples stay within
0.1% of a circle. Collection is bounded to 4,096 circles.

### Catalogue and toolbar

Installed tool plug-ins receive the tool id `plugin.<manifest-id>`. The dynamic tool catalog
interprets their manifest into normal tool metadata. The default toolbar's Ellipse slot becomes
`ellipse` followed by installed tool plug-ins whose `toolGroup` is `ellipse`. Uninstall removes the
dynamic tool and switches back to Selection if that plug-in was active.

## Tangent behaviour

On press, the plug-in selects the topmost circle under the pointer and records where it was
grabbed. On drag, it moves that circle with the pointer. If the moved center comes within the Smart
Guides tolerance of a position that externally touches two other circles, it snaps to the exact
intersection of the two auxiliary tangency circles. It keeps the radius and shows two center-to-
contact guide lines and `tangent` labels.

Given references `(c1,r1)`, `(c2,r2)` and moved radius `r`, valid centers lie at the intersection
of circles `(c1,r+r1)` and `(c2,r+r2)`. Grid, pixel and Shift constraints are outside this plug-in's
scope; a plain drag gesture is used.

## Persistence

The feature is committed on `feature/tangent-circle-snap`. A custom update script fetches upstream,
rebases this branch, reruns the tests, rebuilds the host and plugin, and packages the app. The
official updater replaces the binary and cannot preserve an unpushed local host extension; the
documentation states this plainly.

## Verification

- Manifest and ABI tests for tool plug-ins.
- Circle context and two-circle intersection unit tests.
- Host tool tests with a WAT tool plug-in.
- Toolbar test proving install adds the item beside Ellipse and removal hides it.
- Engine test proving exact tangency through drag, Undo and Redo.
- Built app screenshot with the installed `Tangent Circle.wasm`.
