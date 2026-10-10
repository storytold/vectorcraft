# VectorCraft MCP server

`vectorcraft-cli mcp` runs a [Model Context Protocol](https://modelcontextprotocol.io) server on stdio
(newline-delimited JSON-RPC 2.0; see [Protocol](#protocol) for version negotiation). Agents use it to
draw, inspect and look at VectorCraft documents.

It has two backends:

| Mode | How | What works |
|---|---|---|
| **Remote** | `vectorcraft-cli mcp --connect 127.0.0.1:7979` (the app must run with `vectorcraft --control 7979`) | Everything. Tool calls are forwarded over the [control protocol](control-protocol.md), so you watch the app change live. |
| **Headless** | `vectorcraft-cli mcp --headless` | An in-process engine session with a CPU renderer. Everything except the UI-only tools (`inspect_ui`, `type_text`, `open_panel`, `screenshot {window:true}`). |

With no flag, the server tries `127.0.0.1:7979` and falls back to headless. Logs go to stderr; stdout carries
only protocol messages.

## Build and register

```sh
cargo build --release -p vectorcraft-cli
claude mcp add vectorcraft -- "$PWD/target/release/vectorcraft-cli" mcp
# or pin a mode:
claude mcp add vectorcraft-headless -- "$PWD/target/release/vectorcraft-cli" mcp --headless
claude mcp add vectorcraft-app -- "$PWD/target/release/vectorcraft-cli" mcp --connect 127.0.0.1:7979
```

Other clients use the same command in their JSON config:

```json
{"mcpServers": {"vectorcraft": {"command": "/abs/path/target/release/vectorcraft-cli", "args": ["mcp"]}}}
```

For a live session, start the app first: `cargo run --release -p vectorcraft -- --control 7979`.

### From an installed release

The release packages ship `vectorcraft-cli` alongside the desktop app, so no build is needed:

| Install | CLI |
|---|---|
| Windows (MSI) | `C:\Program Files\VectorCraft\vectorcraft-cli.exe` by default (wherever you installed it otherwise), not on `PATH` |
| Linux (deb, rpm) | `/usr/bin/vectorcraft-cli` |
| macOS | the separate `vectorcraft-cli-<version>-macos-<arch>.zip` release asset (the `.app` holds only the desktop app) |

```sh
# Windows, default install folder
claude mcp add vectorcraft -- "C:\Program Files\VectorCraft\vectorcraft-cli.exe" mcp
# Linux, or macOS with the CLI unzipped onto PATH
claude mcp add vectorcraft -- vectorcraft-cli mcp
```

## Confining file access

```sh
vectorcraft-cli mcp --automation-read-root /work/project --automation-write-root /work/project/out
```

```json
{"mcpServers": {"vectorcraft": {"command": "/abs/path/target/release/vectorcraft-cli",
  "args": ["mcp", "--automation-read-root", "/work/project", "--automation-write-root", "/work/project/out"]}}}
```

The two flags (PhotoCraft and the other Craft apps take the same ones, so one client config works across
the suite) keep the agent to the folders you give it:

- Every file a command reads must lie inside the **read root**: `open_file`, `file.place`, relinking and
  updating links, the images an SVG links to, swatch library, graphic style library and colour-profile loads, plug-ins,
  `file.newFromTemplate`, preset imports, the folders `text.findFontFiles` searches.
- Every file a command writes must lie inside the **write root**: `save_file`, `export`, the save and
  export commands, Export for Screens, Package, Export for Web, print to file, `screenshot {path}`,
  library and preset saves to a path.
- Read and write access are separate. Give the same folder twice for both; a root left out grants none
  of its access (with only `--automation-read-root`, nothing can be written to disk, though `export` and
  the save commands without a path still return the bytes). Without either flag nothing changes.
- A path is made absolute against the server's working directory, then its links are followed: a file
  (or folder) link that leads out of a root is outside it. For a file that doesn't exist yet, its nearest
  existing folder's links are followed and the rest must be plain names, so `..` below a folder that
  doesn't exist is refused, as is a link that leads nowhere (writing through it would create its target).
  The result is compared with the root by whole folder names: `/work2/x` is not inside `/work`. On
  Windows case is ignored, UNC paths (`\\server\share\…`) are resolved like drive-letter ones, and
  device names (`CON`, `NUL`, `COM1`…) are refused.
- A refused path is an ordinary tool error the agent can read and correct, for example
  `automation path rejected: outside the write root /work/project/out: /etc/x.png` or
  `automation filesystem access is not granted: write authority is absent: /work/project/out/x.png`.
  Links in an opened document that point outside the read root show as missing.
- The preferences that move folders the app reads or writes on its own (`fontsFolder`,
  `pluginsFolder`, `recoveryFolder`, `templatesFolder`) can only be set inside the roots. The app's own folders
  (preferences, Data Recovery, the User Defined library folders, VectorCraft's Fonts folder) are not
  confined: no agent names a path there.
- The roots must be existing folders; a missing one stops the server with an error. Each flag is
  given once (`--flag <dir>` or `--flag=<dir>`).

The flags confine the headless server and imply `--headless`; `--connect` with them is an error, since
the server would otherwise talk to an app that isn't confined. A running app confines itself: start it
with the same flags, `vectorcraft --control 7979 --automation-read-root <dir> --automation-write-root
<dir>`, and connect as usual (see [Control protocol](control-protocol.md#confining-file-access)). The
server then writes nothing itself: `screenshot {path}` asks the app to write the file.

The checks hold for every thread of the server process. A path is checked, then opened: another program
that swaps a link in between can still win that race, so the roots keep an agent to the files it was
given rather than guarding against other software on the computer.

## Protocol

Newline-delimited JSON-RPC 2.0 on stdio. The revision is **`2025-06-18`**; `2025-03-26` and `2024-11-05` are
accepted too. Clients requesting `2026-07-28` through `initialize`, or through a request's
`_meta["io.modelcontextprotocol/protocolVersion"]`, receive `resultType: "complete"`,
`ttlMs` and `cacheScope: "private"` on tool/resource/template/prompt lists and resource/prompt reads.
List TTL is 600000 ms; live reads use zero. Older clients keep the original response shapes.
This does not add streaming or input requests — see [Not implemented](#not-implemented).

`initialize` advertises:

| Capability | What it covers |
|---|---|
| `tools` | The tools below |
| `resources` | Three fixed resources and four templates |
| `prompts` | Five ready-made workflows |
| `completions` | `completion/complete` for prompt arguments and template variables |
| `logging` | `logging/setLevel` and `notifications/message` |

### Prompts

`prompts/list` offers five workflows; `prompts/get` returns one templated user message with its
arguments filled in. A missing required argument is `-32602`.

| Prompt | Arguments (required first) |
|---|---|
| `poster` | `brief`, `palette`, `text` |
| `icon-set` | `subject`, `count`, `detail` |
| `recolor` | `palette`, `method` |
| `trace-and-style` | `path`, `preset`, `effect` |
| `export-set` | `formats`, `directory`, `scale` |

```sh
echo '{"jsonrpc":"2.0","id":1,"method":"prompts/get","params":{"name":"poster","arguments":{"brief":"a jazz festival"}}}' \
  | vectorcraft-cli mcp --headless
```

### Resource templates

`vectorcraft://document` (summary), `vectorcraft://document/json` (the whole model), and
`vectorcraft://commands` (the live command catalog with enabled state) are the fixed resources. The templates read one thing at a time, which matters on a real document: the full model
is thousands of lines an agent pays for again on every change.

| `uriTemplate` | Reads |
|---|---|
| `vectorcraft://object/{id}` | One layer or object with its children, bounds and paint: `document.node {id, summary: true}`, the node as `document.inspect` lists it. A large container can be sliced with `run_command document.node {id, summary: true, depth?, childLimit?}` (a level that shows fewer children reports `childCount`) |
| `vectorcraft://command/{id}` | One command: label, menu path, shortcut, parameter description, enablement |
| `vectorcraft://effect/{id}` | One live effect with its parameters and defaults |
| `vectorcraft://swatch/{name}` | One swatch, colour, gradient or pattern swatch (percent-encode spaces) |

An unknown URI is `-32002`; a template with no value, or a value that names nothing, is `-32602`.

The same reads work as query commands through `run_command`: `document.node {id}` returns the object's full model
JSON (geometry, appearance, every attribute) and `document.node {id, summary: true}` the compact summary
`document.inspect` gives for it (id, name, kind, bounds, paint labels, children), without summarizing the whole
document. A summary read slices a large container: `depth` is how many child levels it includes (`0`: the node
alone) and `childLimit` how many children each node shows, top of the stack first (both default to all). A level
that shows fewer children than it has reports their total as `childCount`, so truncation is never silent; with
nothing truncated the reply is the plain summary. Both must be non-negative integers and need `summary: true`
(the full object JSON is never truncated); anything else is an error.

On a large document, drill instead of dumping: `inspect_document {depth: 0}` (or `run_command document.inspect
{depth?, childLimit?}`, same rules) is the skeleton (artboards with top layers and counts), `run_command
document.find {name?, kind?, text?, limit?}` locates nodes across the whole tree, and `document.node` reads the
interesting ones sliced. `document.find` needs at least one filter and all given filters must match; matching
ignores case, `name` is a substring of the Layers panel name, `text` a substring of type content and `kind` the
exact panel label (`Layer`, `Group`, `Clip Group`, `Path`, `Compound Path`, `Type`, `Image`, `Rectangle`, ...).
It answers `{matches: [{id, name, kind, path}], total}` top of the stack first, where `path` is the ancestor
chain (layer first) and `limit` caps `matches` (default 100; `0` only counts). `document.json` stays whole: it is
the fidelity path, not the way to look around.

### Completions

`completion/complete` answers for a prompt argument (`ref/prompt`) or a template variable
(`ref/resource`), ranked by prefix and then by substring, capped at 100 values with `total` and
`hasMore`. The values are read from the live catalogues, so they follow the app: `formats` suggests
what the engine writes, `preset` the Image Trace presets, `effect` the effect ids, `{id}` the layer
and object ids in the document, `{name}` its swatch names. An unknown reference comes back empty
rather than as an error, since a completion is asked for mid-typing.

### Logging

`logging/setLevel` accepts any RFC 5424 severity — `debug`, `info`, `notice`, `warning`, `error`,
`critical`, `alert`, `emergency` — and an unknown one is `-32602` with the previous level left in
place. From then on the engine's own `log` records arrive as `notifications/message` with `level`,
`logger` and `data`. A `null` level turns it back off. Nothing is sent before the client asks, and
records queued at the old level are dropped rather than replayed at the new one.

Installing the logger that receives those records is `vectorcraft-cli`'s job, not the library's: a
crate that installed one would override whatever logger an embedder had already set up. Embedding
`vectorcraft-mcp` directly means calling `vectorcraft_mcp::logging::install()` yourself if you want
log records to reach your client.

### Synchronous calls

Exports complete synchronously. A `_meta.progressToken` is harmlessly ignored, as are
`notifications/cancelled` (including unknown request ids). There are no background export jobs
or progress notifications; cancellation cannot interrupt a running call.

### Not implemented

Everything below needs the same thing first: the server reads one line at a time and answers it with
one reply, so **while a `tools/call` is running it neither reads an incoming line nor writes an
outgoing one.** Progress and cancellation are therefore both meaningless as they stand, and a
subscription could only ever be checked between calls — which is not what a client asking to be told
about a change expects.

- **`resources/subscribe` / `notifications/resources/updated`.** It would also have to be honest about
  *whose* edits it sees: comparing the undo history's length misses edits made in the app window,
  which is the main reason to run against a live document. `document.inspect`'s `revision` is the
  right signal — it is monotonic and already bumped when a document is replaced — but noticing an
  edit still means someone asking.
- **`notifications/progress`.** It would have to be written *before* the reply it belongs to, since a
  progress token stops meaning anything once the request has completed.
- **`notifications/cancelled`.** It would have to be read *during* the call it cancels.
- **`sampling/createMessage` and `elicitation/create`.** Both are server→client requests, blocked by
  the same one-line-one-reply loop. Supporting them means being able to suspend a tool call
  mid-flight, which is what the `2026-07-28` revision's input-requests mechanism exists for.
- **Streamable HTTP / SSE.** stdio only.

`completion/complete` covers the "what could I pass here" half of the same problem, which is why it
is here and subscriptions are not. The fix for the rest is a reader thread (or an event loop) around
the transport — a design change rather than a feature, so it is not in this crate yet.

## Tools

### Common command tools

These names match the conventions in [FilmCraft #28](https://github.com/storytold/filmcraft/pull/28).
The older names below remain listed because existing workflows use them.

| Tool | Arguments and result |
|---|---|
| `command_list` | Optional `filter`, `enabled_only`; returns the command array |
| `command_run` | `id`, optional `params`; runs the command through the existing backend |
| `command_batch` | `steps: [{id, params?}]`, optional `stop_on_error` (default true); returns `completed`, `failed`, `results: [{ok, result\|error}]`. Each edit has its own undo step; failures set `isError`. A param string `"$N.path"` or `"$last.path"` takes a value from an earlier step's result (see `run` below) |
| `doc_inspect` | Optional `depth`, `childLimit`; same summary as `inspect_document` |
| `render_preview` | Optional `max_side` (1–4096, default 1024); inline PNG of the first artboard without editing it. Artboards too large to render within the allocation bound return a tool error |
| `ui_inspect`, `ui_screenshot` | Connected desktop state/window capture; tool errors in headless mode |

Every tool declares read-only, destructive, idempotent and open-world hints. File-writing
`screenshot`, `save_file` and `export` are conservatively marked as mutations. Unknown top-level
argument keys return JSON-RPC `-32602`; command failures and caught tool panics return `isError`
content, and the server continues serving. Command `params` are passed through: the registry
currently describes them in prose, so MCP does not guess schemas or silently remove keys.


Coordinates are points in document space: y points down, the origin is the first artboard's top-left, and a new
document is 612 × 792 (US Letter). New objects become the selection. Most commands act on the selection or on
explicit `ids`.

Paint values (`fill`, `stroke`) accept `"#rrggbb"`, `"none"`, `[r,g,b]` (0..1), `{"c","m","y","k"}`, `{"gray"}`,
or a full `paint.setFill` params object (`{"gradient": …}`, `{"swatch": "name"}`).
The Fill/Stroke proxy commands run through `run_command`: `paint.invert` and `paint.complement` recolour the active
proxy keeping each colour's model, `paint.lastColor` / `paint.lastGradient` re-apply the last solid colour or gradient,
and `paint.recent` returns the recent colours that every paint command (and the eyedropper) feeds.
`paint.proxies` returns what the proxies show: the fill and stroke, which one is active, and whether the selected
objects' fills or strokes differ (`fillMixed` / `strokeMixed`, drawn as a "?" proxy).

| Tool | Arguments | Notes |
|---|---|---|
| `list_commands` | `{filter?, enabledOnly?}` | The command catalogue: id, label, menu, shortcut, params doc, enablement. |
| `run_command` | `{command, params?}` | Runs any command. Use it for everything without a dedicated tool. |
| `inspect_document` | `{depth?, childLimit?}` | Artboards, layer tree (ids, kinds, bounds, paint), selection, history, tool. A type node's `fill`, `stroke` and `strokeWidth` are the paint its characters show (its first run's); fills and strokes of the type object itself come as `objectFill`, `objectStroke` and `objectStrokeWidth`. Paint comes as a label (`"#ff0000"`, `"None"`, `"G1 40% (#ff9999)"` for a swatch at a tint); `paint.proxies` gives it as an object. |
| `inspect_ui` | `{}` | UI state. Remote mode only. |
| `select_tool` | `{tool}` | `selection`, `directSelection`, `pen`, `rectangle`, `ellipse`, `polygon`, `star`, `lineSegment`, … |
| `pointer_gesture` | `{events:[{kind,x,y,mods?}], tool?, mods?}` | `kind` is one of `down`, `drag`, `up`, `move`, `doubleclick`. Events go through the same path as the mouse. |
| `draw_path` | `{points \| d, closed?, fill?, stroke?, strokeWidth?}` | `points` is `[[x,y],…]` or `[{x,y,in?,out?,smooth?},…]`. `d` is SVG path data. |
| `draw_shape` | `{shape, …geometry, fill?, stroke?, strokeWidth?}` | `rectangle`/`ellipse`: `x,y,width,height` (plus `radius` for corners). `polygon`: `cx,cy,radius,sides`. `star`: `cx,cy,radius1,radius2,points`. `line`: `x1,y1,x2,y2`. |
| `set_paint` | `{fill?, stroke?, strokeWidth?, ids?}` | Applies to the selection (or `ids`) and becomes the default for new art. |
| `press_key` | `{key, mods?}` | Remote: a real key event. Headless: runs the command or tool bound to that shortcut, or sends the key to the busy tool (digits too: `5` while dragging with the Perspective Selection tool). |
| `type_text` | `{text}` | Remote only. |
| `invoke_menu` | `{command, params?}` | Invokes a menu item by command id. Includes UI commands such as `view.*` and `window.*` in remote mode. |
| `open_panel` | `{panel}` | Remote only. `panel` is a panel id (`layers`, `swatches`, `colorGuide`, …, as `window.panel` takes) or its display label (`"Color Guide"`), in any case. |
| `screenshot` | `{path?, scale?, artboard?, window?}` | Returns MCP image content (`image/png`, base64) plus a text block. Renders the artboard; `window:true` captures the app window (remote only). With `path` the backend (the headless session or the app) writes the PNG there too. |
| `open_file` | `{path}` | Opens any readable file as a new active document: `.vectorcraft`/`.drawcraft`, `.vctemplate`, `.svg`/`.svgz`, `.pdf`/`.ai`, `.ait`, `.eps`, `.dxf`, `.emf`, `.wmf`, PNG, JPEG, GIF, WebP, TIFF, BMP, PSD/PSB (an image opens as a document of its pixel size; a Photoshop file as its merged image). Templates (`.vctemplate`, `.ait`) open as a new untitled document. PDF, `.ai` and SVG files saved with Preserve Editing reopen as the document they carry. The reply is `document.open`'s: its `warnings` say what didn't come in as it was (an EPS whose PostScript can't be read opens as its preview, and the warning names the PostScript error, the operator and the procedure). `run_command document.formats` lists the formats. |
| `save_file` | `{path?}` | Runs `document.save`: the document's own file in its own format (native `.vectorcraft` unless it was opened from or saved as SVG, PDF or a restorable `.ai`; then `warnings` say what that format loses). A path's extension picks the format (`.vectorcraft`, `.vctemplate`, `.pdf`, `.svg`, `.svgz`, `.ai`: a PDF carrying the native document, which reopens editable). |
| `export` | `{path?, format?, scale?, artboard?, range?, selection?, outlineText?, options?}` | `svg`, `svgz`, `pdf`, `eps`, `dxf`, `emf`, `wmf`, `png`, `jpg`, `webp`, `gif`, `png8` (an indexed `.png`), `tiff`, `bmp`, `tga`, `psd` (layered), `txt` (the document's text), `vectorcraft` or `template` (a native template). The tool's `format` enum and `document.formats` list them, generated from the engine's format table. When `format` is omitted, it comes from the path's extension. PDF writes one page per artboard: all of them, or `artboard` (0-based) / `range` (`"1-3, 5"`, 1-based); the other formats write one artboard. `options` carries more format options (e.g. `{"quality": 80}` for JPEG). `selection: true` exports the selected objects cropped to their bounds (the reply adds their `bounds`, and reports `format` and the encoder's `warnings` as a whole-document export does); `outlineText: true` writes SVG text as paths. Template layers are left out, live effects are kept, and exporting `vectorcraft` never changes the document's path. Without `path` the bytes come back as `dataBase64`. Both backends run the same `document.export` call. |
| `add_text` | `{text, x?, y?, width?, height?, path?, mode?, pathEffect?, size?, font?, color?}` | Point type at (x, y); area type with `width`/`height`; or `path` + `mode` (`area`/`onPath`) to flow text in or along a path, with `pathEffect` (`rainbow`, `skew`, `3dRibbon`, `stairStep`, `gravity`). |
| `apply_effect` | `{effect?, params?, ids?}` | Appends a live effect. Without `effect`, returns the effect catalogue with parameters and defaults. |
| `pathfinder` | `{operation, ids?}` | `unite`, `minusFront`, `intersect`, `exclude`, `divide`, `trim`, `merge`, `crop`, `outline`, `minusBack`. For a live version, apply the `pathfinder.*` effect to a group (`effect.apply` groups several loose selected objects first). |
| `transform` | `{ids?, dx?, dy?, rotate?, scale?, scaleX?, scaleY?, reflect?, shear?, origin?, copy?}` | Runs move, rotate, scale, reflect, shear in that order. With `copy`, the first step duplicates. |
| `create_graph` | `{type?, x, y, width, height, csv? \| series?, categories?, rows?}` | The nine Illustrator graph types: column, stacked column, bar, stacked bar, line, area, scatter, pie and radar. Edit later with `graph.setData` / `graph.setType` via `run_command`. |
| `text_wrap` | `{ids?, offset?, invert?, release?}` | Area type below the objects (same layer) flows around them. |
| `undo` / `redo` | `{}` | |

With `mods.shift`, a marquee dragged with the Selection tool toggles the objects it reaches (the selected ones leave
the selection, the others join it, the rest stays), and one dragged with Direct or Group Selection toggles the anchors
inside it (a path left with none leaves the selection, one with all of them is selected whole). The Lasso adds anchors
with `mods.shift` and takes them away with `mods.alt`. They run `select.toggle {ids}` and `select.anchorsMany {items,
mode: "set"|"add"|"toggle"|"subtract"}`:

```json
{"name":"pointer_gesture","arguments":{"tool":"selection","mods":{"shift":true},"events":[{"kind":"down","x":180,"y":80},{"kind":"drag","x":300,"y":200},{"kind":"up","x":380,"y":200}]}}
```

Appearance stacks: an object can carry several fills and strokes (`appearance.addFill`, `appearance.addStroke`), indexed
in paint order (0 is painted first, the bottom row of the Appearance panel). `paint.setFill`, `paint.setStroke`,
`stroke.set`, `stroke.setAdvanced`, `paint.editGradient`, `paint.setGradientGeom` and `transparency.set` take
`item` to edit one of them; `run_command appearance.setActiveItem {"index": n}` makes that row the target of later
calls that omit `item` (as clicking the row in the Appearance panel does) until the selection changes; the proxy of
its kind (`paint.proxies`) and the Gradient tool's annotator then show that row.
`inspect_document` reports it as `paint.appearanceItem`. Live effects take the same `item` to apply to one fill or
stroke instead of the whole object: `run_command effect.apply {"effect": "path.offsetPath", "item": 0}`, and
`effect.remove`, `effect.setParams` (`visible` toggles one) and `effect.duplicate` address that item's effects;
`apply_effect` uses the active item. `effect.list` reports each object's item effects under `applied[].items`.

Errors (unknown tool, bad arguments, a disabled or failing command) come back as a normal result with
`isError: true` and a message. The model can read the message and retry.

Swatches go through `run_command`: `swatch.list` lists every swatch with its colour group, kind and colour;
`swatch.edit {name, newName?, color?, mode?, global?, spot?}` edits one (fills, strokes and text linked to a
global or spot swatch follow its colour and name, as one undo step); `swatch.delete {names, unlink?}` deletes
swatches and colour groups in one undo step (art using a deleted global swatch keeps its colour, unlinked).
`swatch.new {color?, mode?, spot?, global?, group?}` saves a colour (into a colour group with `group`), gradient
or pattern; `swatch.newGroup {fromArtwork: true, toGlobal?, includeTints?}` makes a colour group of the selected
art's colours (by default as global swatches the art links to).
`swatch.move {names, to?, group?}` reorders swatches or moves them into or out of a colour group (as dragging
them in the Swatches panel does); give colour group names instead to reorder the groups. Dropping a swatch on art
in the app runs `paint.setFill` (or `paint.setStroke`, the active proxy) with `ids: [the object under the pointer]`.
The Swatches panel menu's commands: `swatch.addUsedColors {selection?, global?}`, `swatch.unused` (a query: the
names Select All Unused selects), `swatch.merge {names}` (the first is kept), `swatch.ungroup {name}` and
`swatch.sortByKind`.
Document colour mode: `file.new {colorMode: "cmyk"}` starts a CMYK document with CMYK default swatches, and RGB
colours applied to it (`paint.setFill`/`setStroke` colours and gradient stops, `swatch.new`) are stored as CMYK;
Gray stays Gray, and `keepModel: true` keeps a colour as given. RGB documents keep colours as given. Harmonies, Edit
Colors blends, inversions and Recolor Artwork keep each colour's model (a blend between models takes the document's).
`swatch.new {colors: [...]}` saves several colours as swatches in one undo step. `color.harmony {color, rule, steps?, variation?, amount?}` answers what the Color
Guide panel shows: the harmony rule's colours, base first, and per colour its row of `2·steps+1` variations
(shades, cool or muted on the left, tints, warm or vivid on the right). With `limitTo` (a swatch library id or name,
or `"document"` for the document's swatches) every colour snaps to the library's nearest colour (ΔE 2000), as the
panel's Limit to Library does (`ui.colorGuideLimit`); Recolor Artwork's `limitTo` takes the same values.

Color Themes (local only: no online service) keeps five-colour themes in the preferences. `colorTheme.save
{colors: [...]}` saves up to five colours as they are, `colorTheme.save {color, rule}` saves the five-colour theme a
harmony rule makes from a base colour (base first; `name?` defaults to "Theme N", `replace: "<name>"` overwrites
that theme in place). `colorTheme.list` answers `{themes: [{name, colors: [hex], keys: [colour keys], rule?}]}`,
`colorTheme.delete {name}` removes one and `colorTheme.addToSwatches {name}` adds one to the Swatches panel as a
colour group (one undo step).

```json
{"name":"run_command","arguments":{"command":"colorTheme.save","params":{"color":"#2266aa","rule":"splitComplementary","name":"Harbor"}}}
{"name":"run_command","arguments":{"command":"colorTheme.addToSwatches","params":{"name":"Harbor"}}}
```

SVG Options: `export` to SVG takes them in `options`, flat or as `{"svg": {…}}`: `styling` (`presentation`,
`style`, `entities`, `css`), `outlineText`, `images` (`embed`, or `link`: embedded images are written next to the
SVG, or returned as `linked`), `objectIds` (`layerNames`, `minimal`, `unique`), `decimals` (1–7), `minify`,
`responsive`, `useArtboards`, `range: "all"` (one SVG per artboard, listed in `files`, each holding only the art over its artboard), `preserveEditing` (the SVG
reopens as the full document), `metadata` and `fewerTspans` (one `<tspan>` per line of type). Unknown keys inside `svg` are rejected; `run_command document.formats`
lists every option with its default. `encoding` is `utf8`, `utf16` (big-endian after a byte order mark) or `latin1`
(ISO 8859-1, other characters as `&#x…;` references); `document.serialize` still answers `text`, plus `dataBase64`
(the file) when it isn't UTF-8, and such files open again. `profile: "tiny12"` writes a simplified SVG Tiny 1.2
(presentation attributes only; filters, masks, blend modes and embedded fonts left out, symbols as art, each with
a warning). `embedFonts: true` embeds the fonts type uses as `@font-face` rules subset to the characters used; a
font whose licence (OS/2 `fsType`) forbids subsetting is embedded whole, one that forbids embedding is left out
with a warning. Type names its faces by numeric `font-weight` (600 for Semibold, 300 for Light, `bold` for 700),
embedded or not. `svgz` takes the same options and writes the SVG gzipped (`.svgz` files also open,
place and paste). `run_command document.save {path: "x.svg", svg: {…}}` saves as SVG (or `.svgz`).

Symbols export as one `<symbol>` with a `<use>` per instance. An instance the def can't stand for is written as its
own art: one stained by its fill; one scaled while the symbol has strokes (their weight doesn't scale), or rotated or
scaled while it has effects, brushes or live objects; and every instance of a symbol with pattern paints or unlinked
opacity masks (those stay on the page). `hiddenLayers: true` keeps hidden layers and objects, not displayed
(`display="none"`); `document.save` and the CLI's `convert` keep them unless told otherwise, other exports leave
them out, and they reopen hidden. An object's own data (`object.setProps {data: {pivot: "100,180"}}`) is written as
`data-*` attributes (`data-pivot="100,180"`), and an SVG's `data-*` attributes come in as it. A `preserveEditing` SVG carries the native document (CDATA in `<metadata>`) and a hash of the
markup around it: if another app changed the SVG since, `document.open` reads it as plain SVG and says so in
`warnings`.

Saving beyond `save_file` goes through `run_command`: `file.saveAs {path?, format?, options?, svg?}` (the document takes
on the new file and format), `file.saveCopy` (the document keeps its path, title and modified state),
`file.saveAsTemplate` (a `.vctemplate` that opens untitled; the suggested name and the Templates folder come back
without a path), `file.newFromTemplate {path}`, `file.revert` and `file.formatOptions {format?}` (a format's options
with the values a save would use). Without a path the save commands return `{dataBase64, name, folder?, warnings}`. A document saved as SVG or PDF
remembers its options, so the next save reuses them. A PDF opened with `pages`, `cropTo` or `password` is only part
of the file, so Save asks for a new name instead of writing it back.
`file.documentColorMode {mode, convert?, intent?, grays?}` switches the document colour mode, converting the colours of
the art, symbols, pattern tiles and swatches through the colour settings (`object.convertDocumentColorMode` is an
alias kept for older scripts). Gray colours stay Gray and print on the black plate only. RGB greys are colours like any
other: to CMYK they separate through the profile into four-colour greys and a rich black, as Illustrator converts
them. `grays: "black"` puts RGB greys with R = G = B on the black plate instead, K = their grey value (the inks Edit Colors ›
Convert to Grayscale before the switch would give); other colours convert as usual.

SVG import keeps what the canvas edits live. `<pattern>` becomes a pattern swatch. A `<symbol>` with `<use>` becomes a
symbol (named after its `data-name`, else its id) with an instance per `<use>`; a `<use>` that shows it differently
(paint inherited from the `<use>`, a viewport that cuts the art) or that the canvas would draw otherwise (a scaled
symbol with strokes) stays art. `feGaussianBlur`, `feDropShadow` and the usual shadow chains (offset, blur, flood or
colour matrix, composite, merge) become Gaussian Blur, Drop Shadow, Outer Glow, Inner Glow and Feather effects; other
filters are listed in `warnings`. Undisplayed objects (`display:none`) come back hidden, nested clip paths intersect,
and reflected or repeated gradients (`spreadMethod`) are expanded into stops. `<image>` files (relative links are
found in the SVG's folder) stay linked images (see Linked images); SVG files and `data:` SVGs become art; a file that
can't be read becomes a placeholder in its box, named in `warnings` and `missingLinks`.

Native files: `run_command document.save {compress: true}` writes a gzip-compressed `.vectorcraft` (the
`useCompression` preference makes that the default for saves; compressed files open like any other, told apart by
their content). `version: 2` or `1` writes a file older VectorCraft versions open (format name `drawcraft`, never
compressed), and `preview: true` embeds a PNG of the first artboard, at most 256 px on its longer side. A save writes
a temporary file and renames it over the old one, so a failed save never damages the file; only the images the
document uses are written, and document keys written by a newer version are kept.

## Resources

| URI | Content |
|---|---|
| `vectorcraft://document` | `document.inspect` summary (JSON) |
| `vectorcraft://document/json` | The complete document model (JSON) |

## Examples

Draw, look, export:

```json
{"name":"draw_shape","arguments":{"shape":"rectangle","x":72,"y":72,"width":200,"height":120,"radius":12,"fill":"#1e88e5","stroke":"none"}}
{"name":"draw_shape","arguments":{"shape":"star","cx":400,"cy":300,"radius1":90,"radius2":40,"points":5,"fill":"#ffc107","stroke":"#000000","strokeWidth":2}}
{"name":"draw_path","arguments":{"d":"M72 400 C 150 300 250 500 330 400","stroke":"#e53935","strokeWidth":4,"fill":"none"}}
{"name":"screenshot","arguments":{"scale":0.5}}
{"name":"export","arguments":{"path":"/tmp/art.svg"}}
{"name":"export","arguments":{"path":"/tmp/art.png","options":{"ppi":144,"background":"white","antiAlias":"type"}}}
{"name":"export","arguments":{"path":"/tmp/icon.png","range":"1-3","options":{"useArtboards":true}}}
```

Export As goes through `options` too: for PNG, JPEG and WebP `useArtboards: true` writes one file per artboard (all,
or `range`), named `<file>-<artboard>.<ext>`, and returns `{path, files}` (a PDF keeps them as pages of one file; an
SVG writes one file per artboard of a `range`); `useArtboards: false` exports the bounds of the visible art.

Raster exports take more options in `options`: `ppi` (72, 150, 300…; stored in the file), `background`
(`transparent`, `white`, `black` or `"#rrggbb"`), `antiAlias` (`none`, `art`, `type`: text snapped to pixels) and,
for PNG, `interlaced`.

Long-tail commands:

```json
{"name":"list_commands","arguments":{"filter":"align"}}
{"name":"run_command","arguments":{"command":"select.all"}}
{"name":"run_command","arguments":{"command":"object.align","params":{"align":"left"}}}
{"name":"run_command","arguments":{"command":"object.group"}}
{"name":"run_command","arguments":{"command":"document.exportPdf","params":{"path":"/tmp/art.pdf","range":"1, 3","compatibility":"1.5"}}}
{"name":"run_command","arguments":{"command":"document.pdfSettings","params":{"marks":{"trim":true},"includeDocument":true}}}
```

PDF files take the Save PDF dialog's options: `preset`, `standard`, `compatibility`, the General toggles and the
`compression`, `marks`, `bleed`, `output`, `advanced` and `security` sections (`list_commands` with filter `exportPdf`
documents every field). `document.exportPdf` and `export` (format `pdf`, the options in `options`) return `warnings`:
options accepted but not applied yet, and features approximated or left out. A standard with a PDF version it doesn't
allow is refused (PDF/A-2b at 1.3 or 2.0, PDF/X-4 above 1.6, PDF/X-1a and PDF/X-3 above 1.4); choosing a
standard without giving `compatibility` sets its version (PDF 1.3 for PDF/X-1a and PDF/X-3).
A `security` password encrypts the file (RC4 40-bit at PDF 1.3, RC4 128-bit at 1.4–1.5, AES-128 at 1.6, AES-256 at
1.7 and 2.0) with its permissions; either password opens it (`document.open {password}`), and a password with PDF/A or PDF/X is refused.
Pattern fills and strokes are written as their tiles clipped to the area they paint (a stroke's outline, with its
dashes, caps, profile, arrowheads and alignment), and freeform gradients as an image of their colour field at the
document's raster effects resolution, clipped the same way.
`createLayers` writes each layer and sublayer (template layers are left out) as a PDF layer, an optional content group
named as the layer and listed under its parent layer's: hidden layers are off (their art is written, for the reader to
show), non-printing ones have `/PrintState /OFF` and locked ones are locked. It needs PDF 1.5 or later (at 1.4 it
warns), and the file reopens with those layers and sublayers. A `.ai` file writes them unless `createLayers` is false,
so apps that read its PDF part find the layers, hidden ones included; hidden objects aren't in the PDF part.
Images follow the `compression` settings of their kind (`color`, `gray`, or `mono` for black-and-white images): above
`abovePpi` as placed they are resampled (`downsample`: `average`, `subsample` or `bicubic`) to `ppi`, and compressed
with `zip`, `jpeg` (at `quality`; images with transparency stay lossless) or `auto` (JPEGs stay JPEG, the others
lossless). `none`, `jpeg2000`, CCITT and `runLength` are written as ZIP, with a warning when an image needs them.
CMYK images stay CMYK (DeviceCMYK, or ICC-based with the CMYK profile when colours are tagged, as in PDF/X-3 and
PDF/X-4): a CMYK JPEG neither resampled nor recompressed is written unchanged, and the others' ink amounts are
resampled and compressed again (CMYK JPEG or ZIP). `output.conversion` converts them like CMYK colours: `destination`
to another CMYK profile or to RGB, `preserveNumbers` (and PDF/X-1a) keeps their numbers in a CMYK destination.
`document.pdfSettings` lists the options that differ from the preset and the warnings without writing a file.
`thumbnails: true` embeds each page drawn small (106 px on its long side, without the layers the page leaves out) as
its `/Thumb` image. `fastWebView: true` writes a linearised file (the linearization dictionary first, then the first
page with hint tables saying where every other page's objects are), which stays linearised when a password encrypts it.
`compatibility: "1.3"` writes a PDF 1.3 file, which has no transparency: a copy of the document is flattened first
with `flattenerPreset` (empty: High Resolution; `high`, `medium`, `low` or a saved preset, `flattener.presets.list`)
and `flattener` options over it (as `object.flattenTransparency` takes them); images with see-through pixels count as
transparency and rasterized areas are clipped to their regions. A file that would still have transparency fails the
export. PDF/X-1a and PDF/X-3 files are PDF 1.3 files too.

An Illustrator EPS (version 9 on) or `.ai` file opens from the editing data it carries (after the EPS page; in the
`.ai` file's PDF private data): layers and sublayers (name, order, visibility, lock, printing, preview, dimming,
colour), groups as they were nested, compound paths, clipping groups, object names, hidden and locked objects, fills and
strokes (spot colours as spot swatches), linear and radial gradients, opacity, blend modes, isolation and knockout,
embedded images with their alpha channel, guides, and every artboard where it is. An object with several fills or
strokes, effects or a brush comes in as its drawn look (a group named after it). Type is made from the file's text
document, whole and editable: point type, area type (a story in several frames as threaded type) and type on a path,
with its fonts, size, leading, tracking, scaling, baseline shift, fill and stroke (grey, RGB or CMYK), alignment,
indents and paragraph spacing. Where the text document can't say, type that shows is the page's, in its text object's
place. A file whose editing data has symbols,
pattern fills, placed files or anything else the reader doesn't read on a layer that shows, or whose layers look
different from its page, opens as before: an EPS as its printed page, a `.ai` file from its PDF content (where plain
groups open ungrouped), with a warning saying why. Layers look different when they draw an object the page doesn't (or
miss one it draws), and, for an EPS whose page is the box of its art, when they print art outside it. With
`textAs: "outlines"` a `.ai` file whose type shows opens from its PDF content, which has the type's outlines.

The document then has what the file's page doesn't print: hidden objects and layers, layers that don't print, guides and
the art outside the artboards, as the app that wrote the file shows them. Exports and `artboard.fitToArt` leave out the
hidden art, the guides and template layers; an EPS's artboard is the file's artboard, not the page's bounding box. A
print pipeline that wants only what the page prints opens the file with `editingData: false` (default `true`):
an EPS as its page, a `.ai` file from its PDF content (a `.ai` saved without PDF compatibility then can't be opened).

```json
{"name":"run_command","arguments":{"command":"document.open","params":{"path":"/tmp/label.eps","editingData":false}}}
```

Opening a PDF (or `.ai`) imports every page as an artboard and layer; `document.open` takes `pages` ("2-3, 5", 1-based),
`cropTo` (`bounding` (the art's bounds), `art`, `crop` (default), `trim`, `bleed`, `media`: the box each artboard gets)
and `password` for an encrypted file. `document.pdfInfo` reads a file without opening it: the page count, each page's
size and boxes, `needsPassword`, and with `thumbnail: n` a PNG of page n. Imported colours keep their model:
DeviceCMYK and CMYK ICC colours stay CMYK (a file painted mostly in CMYK opens as a CMYK document), DeviceGray is Gray,
and Separation and DeviceN inks become spot swatches the art links to at its tint (gradient stops too). CMYK images
(DeviceCMYK, or ICC-based with four components) keep their ink amounts: a CMYK JPEG as it is, any other as a CMYK TIFF
(masked CMYK images, and ones with 1, 2 or 4 bits per sample, open in RGB with a warning). Placing a CMYK TIFF keeps it
CMYK too.
`colorMode: "rgb" | "cmyk"` opens any file in that mode instead, its colours converted as `file.documentColorMode` does
(`grays` too):

```json
{"name":"run_command","arguments":{"command":"document.pdfInfo","params":{"path":"/tmp/brochure.pdf","thumbnail":2,"cropTo":"trim"}}}
{"name":"run_command","arguments":{"command":"document.open","params":{"path":"/tmp/brochure.pdf","pages":"2-3","cropTo":"trim"}}}
```

PostScript files (`.eps`, and `.ai` files saved in older formats or without PDF compatibility) open through the EPS
reader (see EPS and PostScript import). An `.ai` saved without PDF compatibility (its PDF part is only a placeholder
page) opens from its editing data alone: its type is made from the file's text document where it can be (point type,
area type and type on a path), and what can't be is left out with a warning; so is non-native art (the content of a
placed PDF, which Illustrator shows but doesn't edit and keeps as a PDF inside the editing data); without editing data
it says it can't be opened.

What a PDF holds comes in as editable art: soft masks become opacity masks (an alpha mask as a white copy of its art;
the backdrop colour gives Clip, an inverting transfer function Invert), transparency groups keep isolation and knockout,
tiling patterns become pattern swatches, patch and triangle mesh shadings become gradient meshes, and gradients keep
their stop opacity and stop where the shading doesn't extend. Text becomes point type, one object per run of a line in
the file's font (by name; fonts that aren't available are listed in `warnings` and show in the fallback font, and
every export that draws that type — PDF, EPS, EMF/WMF, raster images, SVG with outlined or embedded fonts — says in
its `warnings` that it wrote the fallback font) — `textAs: "outlines"` keeps the glyph outlines the file draws instead
(its embedded fonts, installed or not). Lines wrapped in a frame (each wrapped line ending in a space, sharing a left
edge, font and leading) come back as one area type object, left aligned or justified, where laying it out breaks the
lines where the file does; a stroke written as each glyph's outline stroked is the type object's stroke; glyphs set
apart by gaps wider than a space keep their places. Strokes stay live strokes (width, cap, join, miter limit, dash and paint), and
an object written as a fill and then a stroke of the same outline is one path with both. Optional content groups (the
layers of PDF and PDF-compatible `.ai` files) become layers with their name, visibility (the default configuration's,
or a view state that is off), print state and lock, nested as sublayers the way the file's layer order nests them, stacked as the pages mark them
(layers with no art come in empty); art that is off comes in as a hidden layer. Art outside them goes to a layer per page (except the opaque white page a `.ai`
paints under its layers, which isn't art). `layers: false` gives one layer per page of only what shows:

```json
{"name":"run_command","arguments":{"command":"document.open","params":{"path":"/tmp/map.pdf","textAs":"outlines","layers":false}}}
```

Preserve Editing (`preserveEditing`, on in the `VectorCraft Default` preset) embeds the native document in the PDF as
an embedded file (`vectorcraft-editing.vectorcraft`; files named `drawcraft-editing.drawcraft` are read too) with a
hash of the pages it was written with. `open_file` / `document.open` of such a PDF, `.ai` or `.ait` (no `pages` picked) restores the
document exactly (`restored: true`); when another app changed the pages, or the data can't be read, the artwork is
imported instead and the first warning says why. Choosing a standard turns it off (PDF/A refuses it). Save to a `.ai`
path (`document.save {path: "art.ai"}` or `{format: "ai"}`) writes a PDF-compatible file that always carries the
document, takes the PDF options and keeps its path when reopened, so Save writes `.ai` again; a `.ait` opens untitled.
`document.export` / `serialize` to `.ai` (and the CLI's `run --export x.ai` and `convert in.svg out.ai`) write the same file
without giving the document a path.

Drive a tool like a mouse:

```json
{"name":"pointer_gesture","arguments":{"tool":"ellipse","events":[
  {"kind":"down","x":100,"y":100},{"kind":"drag","x":150,"y":140},{"kind":"up","x":200,"y":180}]}}
```

Edit a gradient with the Gradient tool's annotator (the bar runs along the vector; stop chips sit 10 px under
it, midpoint diamonds 7 px above): a click on the bar adds a stop, dragging a chip moves it (drag it off the bar to
delete it, hold Alt to copy it), dragging the end handle changes the vector. The selected stop
(`gradient.selectStop`) takes Delete and ←/→:

```json
{"name":"run_command","arguments":{"command":"paint.setFill","params":{"gradient":{"start":[100,150],"end":[200,150]}}}}
{"name":"pointer_gesture","arguments":{"tool":"gradient","events":[{"kind":"down","x":130,"y":150},{"kind":"up","x":130,"y":150}]}}
{"name":"pointer_gesture","arguments":{"events":[{"kind":"down","x":130,"y":160},{"kind":"drag","x":160,"y":160},{"kind":"up","x":160,"y":160}]}}
{"name":"press_key","arguments":{"key":"Delete"}}
```

A radial gradient's annotator also draws its extent: a dashed ellipse around the centre (the start, drawn as a ring)
with a dot on it across the bar (drag it to change the aspect ratio); dragging the ellipse elsewhere rotates it.
The dot inside the centre ring is the focal point, where the first stop sits: drag it for an off-centre radial, back
onto the centre to centre it. `paint.setGradientGeom` sets the same things directly (`aspect` in %, `focal` in
document coordinates or `null`; `start`/`end` may be left out), and they export as SVG `fx`/`fy` and PDF two-point
radial shadings:

```json
{"name":"run_command","arguments":{"command":"paint.setGradientGeom","params":{"aspect":60,"focal":[130,140]}}}
```

Freeform gradients: `paint.editGradient {kind: "freeform"}` places four or more colour points inside each selected
object (coloured along the stops; `mode: "points"|"lines"` is the Draw toggle). `paint.freeform.get` lists the points
(document coordinates), lines and the selected point; `paint.freeform.addPoint {at, color?, opacity?, spread?,
line?}`, `setPoint {index?, …}`, `deletePoint {index?}`, `addLine {points}`, `splitLine {line, segment, t?}` and
`selectPoint {index|null}` edit them (`index` defaults to the selected point; each edit is one undo step). With the
Gradient tool on a freeform gradient a click on a point selects it (drag to move it), a click on a line adds a point
on it, a click elsewhere on the art adds a point (in Lines mode joined to the selected one), and Delete removes the
selected point:

```json
{"name":"run_command","arguments":{"command":"paint.editGradient","params":{"kind":"freeform","mode":"lines"}}}
{"name":"run_command","arguments":{"command":"paint.freeform.addPoint","params":{"at":[150,150],"color":"#ff3366","spread":20}}}
{"name":"pointer_gesture","arguments":{"tool":"gradient","events":[{"kind":"down","x":180,"y":170},{"kind":"up","x":180,"y":170}]}}
```

On the canvas the selected point also shows its spread as a dashed ring with a handle 16 px (or the spread, if larger)
to its right: drag the ring or handle to change the spread. Dragging a point out of the object removes it, and
double-clicking one opens its popover (dialog `gradientStop`, which on a freeform gradient edits the selected point:
set `color`, `opacity` and `spread` and confirm). In Lines mode successive clicks on the art draw one smooth line
through the points they add; a click on an existing point first continues the line from it, and `press_key` Escape
(or a click off the art) ends it:

```json
{"name":"pointer_gesture","arguments":{"tool":"gradient","events":[{"kind":"down","x":110,"y":190},{"kind":"up","x":110,"y":190},{"kind":"down","x":150,"y":170},{"kind":"up","x":150,"y":170}]}}
{"name":"press_key","arguments":{"key":"Escape"}}
```

Raw protocol (for debugging):

```sh
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"sh","version":"0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"draw_shape","arguments":{"shape":"ellipse","x":10,"y":10,"width":100,"height":80,"fill":"#ff0000"}}}' \
  '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"inspect_document","arguments":{}}}' \
  | target/release/vectorcraft-cli mcp --headless
```

## Headless batch CLI

```sh
vectorcraft-cli commands                          # command catalogue (JSON)
vectorcraft-cli run --in in.svg \
  --cmd select.all --cmd paint.setFill --params '{"color":"#ff0000"}' \
  --export out.svg --export out.png --scale 2
vectorcraft-cli run --cmd file.new --params '{"width":800,"height":600}' \
  --cmd shape.star --params '{"cx":400,"cy":300,"radius1":200,"radius2":90}' \
  --export star.vectorcraft
```

`run` prints one JSON line per step (`open`, `cmd`, `export`) and exits non-zero on the first failure. `--params`
applies to the `--cmd` just before it. `run` also accepts the host commands `file.open`, `file.save`, `file.export`,
`file.exportForScreens` and `tool.select`. `run --in`, `convert` and `info` read every format `document.open` reads
(`vectorcraft-cli --help` lists them).

A step can use an earlier step's result, in `run` as in the `command_batch` tool. A parameter string that is exactly
`$N` or `$N.path` takes that value from the result of step `N` (counting from 0, in the order they ran; `run --in`'s
open isn't a step), and `$last…` from the step before. A path goes into objects by key and arrays by index:
`"$1.id"`, `"$2.ids.0"`, `"$last.ids[0]"`. A string starting with `$$` stands for one `$` (`"$$1.99"` is `$1.99`). A
reference that names no value fails the step, so a command never runs with one left in it:

```sh
vectorcraft-cli run --cmd file.new --params '{"width":600,"height":200}'   --cmd text.create --params '{"x":10,"y":60,"text":"plain and bold words","size":30}'   --cmd text.setRangeStyle --params '{"id":"$1.id","start":10,"end":14,"style":"Bold"}'
```

## Transparency and opacity masks

Transparency and opacity-mask commands take `ids` (or `id`), so they need no selection. Opacity is a percentage
(0..100) in every command (`transparency.set`, `object.setProps`, `appearance.setItem`). `transparency.info` returns
the Transparency panel's values, with `null` where the objects differ. Making a mask from one object gives it an
empty mask and enters mask editing: art drawn then becomes the mask, and the mask commands act on the masked object
until `transparency.stopEditingOpacityMask`. Saves and exports never include the editing layer.

```json
{"name":"run_command","arguments":{"command":"transparency.set","params":{"ids":[12],"opacity":40,"blend":"Multiply"}}}
{"name":"run_command","arguments":{"command":"transparency.makeOpacityMask","params":{"ids":[12,15],"invert":true}}}
{"name":"run_command","arguments":{"command":"transparency.setOpacityMask","params":{"id":12,"clip":false}}}
{"name":"run_command","arguments":{"command":"transparency.info","params":{"ids":[12,20]}}}
```

Knockout Group has three states: `transparency.set {knockout: "on"|"off"|"neutral"}` (`true` = on, `false` =
neutral, the default). In a knockout group each child hides what it covers of the children below it; neutral groups
pass the enclosing group's setting to their children, off groups never knock out. `knockoutShape: true` makes the
object's opacity and opacity mask scale how much it knocks out. `transparency.togglePageKnockoutGroup` and
`transparency.togglePageIsolatedBlending` (`{value?}`) treat the whole page as a knockout or isolated group; they are
saved with the document and undoable, and `transparency.info` reports them. PDF and SVG write knockout groups as
soft-masked groups with the same look (the PDF export reports a warning).

Groups are not isolated unless Isolate Blending is on: a blend mode inside a group with opacity, a blend mode, an
opacity mask, knockout or a clip reaches the art below the group, on screen and in PDF (a knockout group's elements
then composite against the art below it). SVG has no non-isolated groups, so there such groups stay isolated.

`transparency.viewOpacityMask {on?, id?}` is Alt-clicking the mask thumbnail: the canvas shows only the mask of `id`
(default: the mask being edited, else the first selected masked object) as greyscale coverage (white = opaque) and the
mask is edited; called again it shows the artwork, still editing. It is view state of the open document: the canvas
(and `ui.screenshot`) shows it, exports never do, and it ends when mask editing ends.

`view.transparencyGrid {on?}` (View → Show Transparency Grid; default: toggle) is view state of each open document,
like the mask view: the grid shows behind the artboards of the documents that turn it on, on the canvas and in
`ui.screenshot`, never in exports. It returns `{on}`, isn't undoable and doesn't mark the document changed.

## Clipping masks

`object.clippingMask.make` clips the selected objects by the topmost one, which may be a path, a compound path or a
text object (it loses its paint). Compound holes, even-odd fills, glyph outlines and the union of a group's members
clip alike on screen, in raster export and in SVG and PDF. `object.clippingMask.release` turns the clip group into a
plain group and keeps the clipping path with whatever paint it has.

A clipping path can be painted after Make (`paint.setFill`, `paint.setStroke`, `stroke.set` with its `ids`): its fill
paints behind the clipped art and its stroke over it, not clipped, on screen and in SVG and PDF.

```json
{"name":"run_command","arguments":{"command":"select.set","params":{"ids":[12,15]}}}
{"name":"run_command","arguments":{"command":"object.clippingMask.make","params":{}}}
```

`layer.clippingMask.toggle {id?}` is the Layers panel's clipping mask button: the top object of the layer `id` (default:
the one highlighted layer or group row, else the one selected group, else the current layer) becomes its clipping path (unpainted, moved to the bottom of the layer,
so art added later is clipped too); called again it releases the mask. It returns `{clip}`, and the Layers panel
underlines clipping-path names.

## Object selection

`select.set {ids}` replaces the selection, `select.add {ids}` adds to it, and
`select.toggle {id}` or `select.toggle {ids}` toggles each target.
Every id must be a non-negative integer naming an existing object.
A malformed or unknown id is an error naming that value; the entire selection is left unchanged.
All three commands return `{count, ids}` for the resulting selection, in selection order.
An empty array clears the selection for `select.set` and leaves it alone for Add and Toggle.

`edit.clear {ids?}` deletes the exact selected objects, including individual compound-path members;
their unselected siblings remain in the compound. Supplying `ids` works without a selection and
overrides selected anchors or guides. Every explicit id is validated before any deletion; a malformed
or unknown id leaves the document unchanged. An empty `ids` array does nothing. Layers themselves
are kept, and selecting an ancestor together with its descendants deletes that subtree once.
Without `ids`, direct-selected anchors or selected ruler guides retain their usual Clear behavior.
Cut still removes the objects it copied, including a whole compound when a member is selected.

## Saved selections

Select → Save Selection… keeps the selected objects under a name, in the document: `select.save {name?}` (default
the first free "Selection N"; an existing name is replaced with the current selection; at most 25 per document,
names up to 255 characters) → `{name}`, one undo step. `select.savedList` lists the names in menu order, and
`select.recall {name}` selects those objects again (ones deleted since are left out) → `{count}`. Edit Selection…
renames and deletes: `select.editSaved {name, newName?, delete?}`, or several at once as
`{edits: [{name, newName?, delete?}…]}` in one undo step, where every `name` is the name before the edit and the
names must stay unique. In the desktop app the saved selections are listed at the bottom of the Select menu
(`select.recall1` … `select.recall25`). A file's saved selections are checked when it opens: past 25, blank or
repeated names, and ids the document doesn't have are dropped.

## Ruler guides

Ruler guides are numbered in the order they were made. `guide.add {vertical, pos, artboard?}` makes one (the x of a
vertical guide, the y of a horizontal one, in points) → `{index}`. With `artboard` (an artboard's index) it is an
artboard guide: it runs across that artboard only, moves with it (`artboard.move`, a pure move in
`artboard.setProps`, `artboard.rearrange`), scales with it (`artboard.setProps {scaleArt}`), is copied with it (`artboard.duplicate`, `artboard.move {copy}`) and is
deleted with it (`artboard.delete`); without, a canvas guide runs across the whole canvas. `guide.list` →
`[{index, vertical, pos, selected, artboard?}…]` (`artboard` only for artboard guides).
`guide.select {indexes: [index…], toggle?}` selects guides on their own (the art is deselected; `toggle` adds or
removes them) → `{selected}`. `guide.move {index, pos}` puts one guide somewhere, `guide.move {dx?, dy?, copy?}`
moves the selected ones (vertical guides by `dx`, horizontal ones by `dy`; `copy` leaves them and selects the
moved copies), and `guide.remove {index?}` deletes one guide or the selected ones → `{count}`; each is one undo
step. With guides selected, `edit.clear` (Delete) deletes them and `object.nudge` (the arrow keys) nudges them.
View › Guides › Lock Guides (`view.guides.lock`) deselects them and refuses these commands until unlocked.

The Selection, Direct Selection and Group Selection tools pick a guide within the selection tolerance, over the art
(an anchor on it comes first with Direct Selection), and drag the selected guides: `mods.alt` copies them,
`mods.shift` snaps the dragged guide to the ruler's ticks, and otherwise it snaps to whole pixels, the grid or, with
Smart Guides, the edges, centres (so the sides' midpoints) and anchors of the art and of the artboards; with Smart
Guides off, View › Snap to Point pulls it onto anchors. In the desktop app a guide dragged out of a ruler snaps the
same way and is made where it is released over the canvas (with the Artboard tool, as an artboard guide of the
active artboard), a guide dropped off the canvas onto its ruler is deleted, and hidden guides (View › Guides › Hide
Guides) can't be picked. An artboard guide is picked, drawn and snapped to across its artboard only.

```json
{"name":"run_command","arguments":{"command":"guide.add","params":{"vertical":true,"pos":100}}}
{"name":"run_command","arguments":{"command":"guide.add","params":{"vertical":false,"pos":200,"artboard":0}}}
{"name":"pointer_gesture","arguments":{"tool":"selection","events":[{"kind":"down","x":100,"y":50},{"kind":"drag","x":140,"y":50},{"kind":"up","x":140,"y":50}]}}
{"name":"run_command","arguments":{"command":"guide.list","params":{}}}
```

## Graphic styles

A graphic style holds an appearance (fills, strokes, effects) plus opacity, blend mode, isolate and knockout.
`graphicStyle.new {name?, id?}` captures an object (a group without its own fills or strokes lends its topmost
object's, type its characters'). `graphicStyle.apply {name, ids?}` gives the objects the style and links them;
`add: true` adds the style on top of the existing appearance instead. Linked objects stay linked while they keep the
style's look: editing their appearance or transparency breaks the link, and `graphicStyle.redefine {name?, id?}`
updates only the objects still linked. `graphicStyle.list` returns each style with the ids linked to it and the
style of the first selected object; `select.same.graphicStyle` selects an object's fellow users.
Styles keep placed gradients relative to the bounds of the object they were made from, and each object a
style is applied to gets them at the same place relative to its own bounds.

```json
{"name":"run_command","arguments":{"command":"graphicStyle.new","params":{"id":12,"name":"Glow"}}}
{"name":"run_command","arguments":{"command":"graphicStyle.apply","params":{"name":"Glow","ids":[15,20]}}}
{"name":"run_command","arguments":{"command":"graphicStyle.redefine","params":{"id":12}}}
{"name":"run_command","arguments":{"command":"graphicStyle.list","params":{}}}
```

`graphicStyle.merge {names, name?}` adds a style stacking the fills, strokes and effects of two or more styles
(each on top of the ones before it) with the first one's transparency; `graphicStyle.move {name, to}` reorders the
list (what dragging a style in the panel does). Override Character Color (`graphicStyle.setOptions
{overrideCharColor}`, the preference `overrideCharColor`, on by default) makes a style applied to type replace its
characters' fill and stroke with the style's fills and strokes; off, the characters keep their colour under them.

## Editing appearance stacks

`effect.move {from, to, fromItem?, toItem?, copy?}` reorders an effect or moves it between the object's effects
(`null`) and a fill's or stroke's (an item index), as dragging its row in the Appearance panel does; `copy: true` copies
it (Alt-drag). `appearance.duplicateItem {index, to?}` / `{indices}` and `appearance.removeItem {index | indices}`
act on several fills/strokes at once, and `appearance.showAllHidden` makes every hidden fill, stroke and effect
visible again. In remote mode, `effect.dialog {effect, index, item}` opens an applied effect's dialog prefilled
(`ui.dialog.confirm` runs `effect.setParams`); see the control protocol for the "already applied" question.

## Overprint

Each fill and stroke (and each character's fill and stroke) can overprint: its inks print over the inks below instead
of knocking them out. `object.setOverprint {fill?, stroke?, item?, ids?}` sets it (groups set their contents, type its
characters too; `item` aims at one appearance item) and `attributes.info {ids?}` reads it back (`null` where the
objects differ). Overprint Preview (`view.overprintPreview`) and Separations Preview show it; rendering approximates
it by multiplying. Older files that listed Overprint Black objects in the document get the flags on load.
PDF export writes them with `advanced.overprint: "preserve"` (the default): each overprinting fill and stroke sets a
graphics state with `/OP true /op true /OPM 1` (`/OPM 0` in PDF/A). With `discardWhiteOverprint` (`document.setup`, on by
default) white ones knock out instead; `"discard"` writes none.

```json
{"name":"run_command","arguments":{"command":"object.setOverprint","params":{"ids":[12],"stroke":true}}}
{"name":"run_command","arguments":{"command":"attributes.info","params":{"ids":[12]}}}
{"name":"run_command","arguments":{"command":"view.overprintPreview","params":{"on":true}}}
```

## Edit Colors and Recolor Artwork

`edit.colors.invert`, `edit.colors.toCMYK`, `edit.colors.toGrayscale`, `edit.colors.toRGB`, `edit.colors.saturate`,
`edit.colors.adjustBalance` and `recolor.apply` recolour everything inside the selection (or `ids`) in one undo step:
fills, strokes, text, gradient stops, gradient-mesh points, embedded images (a recoloured copy of the image; linked
images are left alone) and the tiles of pattern fills and strokes (a recoloured copy saved as a new pattern swatch,
e.g. "Dots 2"; the original pattern is untouched). `includeImages: false` / `includePatterns: false` leave images or
patterns out. The Blend commands also grade gradient meshes, which keep their shading.

```json
{"name":"run_command","arguments":{"command":"recolor.colors","params":{}}}
{"name":"run_command","arguments":{"command":"recolor.apply","params":{"map":{"#ff0000":"#0055ff"},"includeImages":false}}}
{"name":"run_command","arguments":{"command":"edit.colors.adjustBalance","params":{"mode":"cmyk","m":-20,"k":10}}}
```

Recolor Artwork keys colours by identity, not hex: `recolor.colors` lists each colour's `key` (`"rgb 255 0 0"`,
`"cmyk 0 100 100 0"`, `"gray 40"`, `"lab 55 60 -40"`; RGB in 0–255 levels, CMYK and Gray in percent), so CMYK,
Lab and RGB colours that look alike stay apart, and every colour parameter also accepts a key.
`recolor.reduce {colors?, method?, preserve?, limitTo?}` groups the selection's colours into rows of similar colours (k-means in Lab, weighted by use;
tints of a global swatch share its row; White and Black are preserved by default) and returns a map of rows
`[{from: [keys], to: key}]` to edit and pass to `recolor.apply {map, method?, limitTo?, group?, groupColors?, rename?}`.
`method` says how a row's colours take its new colour: `exact`, `preserveTints` (tints of the row's darkest colour
stay tints), `scaleTints` (every colour becomes a tint as light, relative to the darkest), `tintsShades` (lighter and
darker than the row's average become tints and shades) or `hueShift` (the most saturated colour takes the new colour,
the others turn by the same hue). `limitTo` snaps new colours to a swatch library's nearest colour;
`recolor.randomize {map, order?, saturationBrightness?, seed?}` shuffles or varies new colours; a row with
`exclude: true` keeps its colours. `swatch.editGroup {group, colors, rename?}` rewrites a colour group in one undo
step (art linked to its global swatches follows), and `recolor.apply` with `group` recolours the art and rewrites the
group together.

```json
{"name":"run_command","arguments":{"command":"recolor.reduce","params":{"colors":2,"preserve":{"grays":true}}}}
{"name":"run_command","arguments":{"command":"recolor.apply","params":{"map":[{"from":["cmyk 0 100 100 0","cmyk 0 40 40 0"],"to":"cmyk 100 50 0 0"}],"method":"scaleTints"}}}
{"name":"run_command","arguments":{"command":"swatch.editGroup","params":{"group":"Brights","colors":["#ff0000","cmyk 0 0 100 0"]}}}
```

## Group, layer and type appearance

Groups and layers carry fills, strokes and effects of their own, as in the reference app: their fills and strokes
paint every member's geometry, and their effects apply to the members as one piece (one combined drop shadow; a
Transform or Warp moves or bends the whole group). Their Contents row (type: Characters) is a slot in the stack:
fills and strokes above it paint over the members (characters), those below under them; new items go above it.
`appearance.moveItem {"from": "contents", "to": n}` puts the row above the bottom `n` items, and an item move takes
`contents` to say where the row ends up in the same undo step; `document.node` shows the slot as
`appearance.contents_index`. The `appearance.*`, `effect.*` and `graphicStyle.apply` commands edit the selected
objects themselves (a group's own stack; layers through `ids`) or, with `target: "contents"`, the objects inside the
groups and layers. `appearance.targetContents` selects a group's members (double-clicking the Contents row).
SVG and PDF export bake a group's own fills, strokes and geometry effects into paths; its raster effects stay a
filter on the whole group in SVG and become an image of the whole group in PDF.

```json
{"name":"run_command","arguments":{"command":"appearance.addFill","params":{"ids":[7]}}}
{"name":"run_command","arguments":{"command":"appearance.moveItem","params":{"ids":[7],"from":"contents","to":1}}}
{"name":"run_command","arguments":{"command":"effect.apply","params":{"ids":[2],"effect":"stylize.dropShadow"}}}
{"name":"run_command","arguments":{"command":"appearance.addStroke","params":{"ids":[7],"target":"contents"}}}
```

## Swatch libraries

Swatch libraries are read-only sets of swatches computed in code (Web Safe 216, Grays and Neutrals, Earth Tones,
Skin Tone Ramps, Pastels, Brights, Metallic Gradients, Perceptual Scales, Harmony Sets). `swatch.library.list` lists
them (`id`, `name`, `category`, `count`) and `swatch.library.get {library}` returns one's swatches and colour groups
in the `swatch.list` shape (`library` is an id or a name). `swatch.library.add {library, names?}` copies swatches
into the document as one undo step: a colour group's name brings the whole group, a swatch's name the swatch alone,
no names the whole library; swatches the document already has (same name and paint) are reported under `existing`
and not added again. `apply: "fill"|"stroke"` also applies the first one, in the same undo step (what clicking a
swatch in the library panel does). `swatch.resetDefaults {replace?}` brings back the missing default swatches.

```json
{"name":"run_command","arguments":{"command":"swatch.library.get","params":{"library":"earth-tones"}}}
{"name":"run_command","arguments":{"command":"swatch.library.add","params":{"library":"earth-tones","names":["Clay"]}}}
```
`swatch.library.save {path?, format?: "vcswatches"|"gpl"|"ase"|"css", names?, name?, user?}` writes the document's
swatches as a library (`.vcswatches` keeps colour models, global, spot, gradients and colour
groups; `.gpl` is 8-bit RGB; a swatch exchange `.ase` file keeps solid colors in their own model
(RGB, CMYK, Lab or Gray) as global, spot or process colors, and color groups, writes a tint swatch
as the color it shows and leaves gradients out; CSS writes custom properties); without `path`
it returns `{data}`, or `{dataBase64}` for `.ase`, and `user: true` saves into the user library
folder of the desktop app (listed as category `user`, User Defined). `swatch.library.load {path? | data? |
dataBase64?, name?}` loads a `.vcswatches`, `.gpl`, swatch exchange (`.ase`) or color book (`.acb`) file, or another
document's swatches, as a library to add from. From an `.ase` file it reads RGB, CMYK, Lab and Gray colors as global,
spot or process swatches and keeps their color groups. A color book (such as a Pantone book you own) gives its RGB,
CMYK or Lab colors named with the book's prefix and suffix ("PANTONE 185 C"), as spot colors unless the book marks
them as process colors; `.acb` files are read, never written. A file in the user library folder, or a file with the
same extension and bytes as one there, loads as that User Defined library (category `user`).
`swatch.library.copyToUser {library}` copies a loaded library into the user library folder of the desktop
app (a library file as it is; a document's swatches or a library loaded from `data` or `dataBase64`
as `.vcswatches`) under a name no file there has, and lists it as User Defined from then on; until the app
quits, commands given its loaded id use the copy → `{library, name, count, path, copied}` (`copied: false`
when the folder already held the same file). Without a user library folder the command is disabled.

## Graphic style libraries

Graphic style libraries are read-only sets of graphic styles generated in code (Shadows and Glows, Outlines and
Rules, Hand-Drawn, Gradient Finishes, Shape Effects, Blends and Transparency). `graphicStyle.libraries` lists them
(`id`, `name`, `category`: `builtIn`, `user` or `loaded`, `count`) and `graphicStyle.library {library}` returns one's
styles in the `graphicStyle.list` shape (`library` is an id or a name). `graphicStyle.addFromLibrary {library, name? |
names?}` copies styles into the document's Graphic Styles as one undo step (no names: the whole library); styles the
document already has (same name and look) are reported under `existing`, a name another look has gets a number, and
the patterns the styles paint with come along. `apply: true` also applies the first one to `ids` or the selection
(`add: true` adds its appearance on top instead), in the same undo step: what clicking a style in the library panel
does. Library styles keep placed gradients in unit-box space, so each object gets them at the same place relative to
its bounds.

```json
{"name":"run_command","arguments":{"command":"graphicStyle.library","params":{"library":"hand-drawn"}}}
{"name":"run_command","arguments":{"command":"graphicStyle.addFromLibrary","params":{"library":"hand-drawn","name":"Crosshatch","apply":true}}}
```
`graphicStyle.saveLibrary {path?, names?, name?, user?}` writes the document's styles as a `.vcstyles` library (JSON:
the styles unlinked from swatches, with their opacity, blend mode, isolate and knockout, and the patterns they paint
with); without `path` it returns `{data}`, and `user: true` saves into the user library folder of the desktop app
(category `user`, User Defined). `graphicStyle.loadLibrary {path? | data? | dataBase64?, name?}` loads a `.vcstyles`
file, or another document's graphic styles, as a library to add from; a file in the user library folder, or a file
with the same extension and bytes as one there, loads as that User Defined library.

## Libraries

The Libraries panel's libraries hold graphics, colours and character and paragraph styles that any document can use.
They are kept on this machine: each is a `.vclibrary` file (JSON) in the `Libraries` folder next to the preferences
(none in the web app and headless sessions, where they last for the session). `library.list` lists them (`id`, `name`,
the count of each kind, `current`, `folder`); `library.get {library?}` returns one's items (graphics with `id`, `name`,
size and a PNG `thumbnail` in base64; colours with `name`, `hex` and `color`; styles with `name` and `attrs`).
`library.create {name?}`, `library.rename {library?, name}`, `library.delete {library?}` and
`library.setCurrent {library}` manage them; `library` is an id or a name, and without it the current library (the one
the panel shows) is meant.

`library.add {library?, kind, ids?, name?, color?}` adds from the selection: `graphic` (a copy of the objects with the
images, symbols, patterns, swatches, styles and brushes they use, as Copy takes them), `fillColor` / `strokeColor` (the
first selected object's solid colour, else the default one, or `color`), `charStyle` / `paraStyle` (the selected
text's attributes). An item the library already has is reported as `existing`. `library.use {library?, kind, item,
center?, to?}` uses one: a graphic is placed centred on `center` (pasted with its resources; a swatch name the document
gives another colour merges into the document's), a colour paints the selection's fill or stroke, and a style is added
to the document (numbered when its name is taken by other attributes) and applied to the selected text, each as one
undo step. `library.removeItem {library?, kind, item}` removes one.

```json
{"name":"run_command","arguments":{"command":"library.add","params":{"kind":"graphic","name":"Logo"}}}
{"name":"run_command","arguments":{"command":"library.use","params":{"kind":"graphic","item":"Logo","center":[300,200]}}}
```

## Strokes on type

`stroke.set` without an `item` gives type its characters' stroke: weight, cap, join, miter limit and the dash
options go to every run (the object keeps no stroke of its own); an `item` still edits an object-level stroke added
with `appearance.addStroke`, which takes every stroke option (dashes, profile, opacity, blend) like a path's and paints under or over the
characters as its place relative to the Characters row says. To stroke
some characters, use `text.setRangeStyle {id, start, end, strokeOptions: {weight?, cap?, join?, miterLimit?, dash?,
dashOffset?, alignDashes?}}`. `inspect_document` reports each object's stroke as `strokeOptions` (type: its first
run's) in `stroke.set` terms. `stroke.set` on a group leaves the images and symbol instances in it alone.

## Inline graphics in type

A document symbol can sit in a line of type like a character (beyond Illustrator; like InDesign's inline
anchored objects), e.g. mana symbols in card rules text. `text.insertInline {id?, at?, symbol?, scale?: 1,
shift?: 0}` inserts one at byte offset `at` (default: the Type tool's selection, which it replaces) of text `id`
(default: the text being edited), showing `symbol` (default: the Symbols panel's current symbol) → `{id, caret}`;
Type → Insert Inline Symbol does the same at the caret. The graphic is one character, U+FFFC, in a run of its own:
`text.getRange` reports it as `{text: "\uFFFC", style, inline: {symbol, scale, baseline_shift}}`, plain text (and
copied text) shows U+FFFC, and `text.editRange` with styled `runs` can insert one. Typing over it, Backspace and
Delete remove it whole; neighbouring text never merges into it.

Layout: the art is scaled uniformly to `scale` × the run's font size tall, its left edge on the pen and its
vertical centre on the middle of the cap height raised by `shift` points. Its advance is the scaled art width plus
the run's tracking, so it breaks like a word (no break between it and punctuation stuck to it) and justifies like
a glyph; a graphic taller than the font's ascent or descent opens up its line. Its art follows the symbol
(Redefine Symbol resizes it); a missing or deleted symbol leaves an empty slot one em square that draws nothing.
Its em box is its run's, whatever its scale: Character Alignment moves it with its run's text (it is one of
the line's largest characters only when its run's size is), and Top-to-Top leading spaces its line by its run's
em box (Roman leading by its run's leading). In right-to-left text it is a bidi neutral (U+FFFC): it takes the
direction of the text around it and the line's reordering places it.
The canvas draws the art inside the type's transparency (its opacity and blend mode apply). SVG keeps the text live
(the text after a graphic is positioned after it) and writes each graphic as a `<use>` of the symbol's `<symbol>`
def (or a copy of its art); type on a path with graphics is written as outlines. PDF draws the art as vector paths
where the layout puts it.

## Flatten Transparency

`object.flattenTransparency {ids?, preset?, …options}` turns transparent art into opaque art that looks the same, in
one undo step. The targets split into groups of overlapping objects; groups without transparency stay as they are.
Each other group becomes one group of atomic regions: paths filled with the flat colour the art showed there (over
white; over nothing, keeping alpha, with `preserveAlpha: true`), plus one image where gradients, patterns, images,
opacity masks or raster effects reach (clipped to those regions with `clipComplexRegions`, a rectangle without).
`preset` is `high`, `medium` (the default) or `low`; option keys (`balance` 0–100, `lineArtPpi`, `gradientPpi`,
`textToOutlines`, `strokesToOutlines`, `clipComplexRegions`, `antiAlias`, `preserveAlpha`, `preserveOverprints`)
override it, at the top level or in `options`. `balance: 0` rasterizes everything; lower balances rasterize groups
that split into many regions. The result reports `{ids, vector, rasterized, options}`.

```json
{"name":"run_command","arguments":{"command":"object.flattenTransparency","params":{"ids":[12,15],"preset":"high"}}}
{"name":"run_command","arguments":{"command":"object.flattenTransparency","params":{"balance":0,"lineArtPpi":150}}}
```

## Pattern editing display

In pattern editing mode the tile edge (`pattern.options {showTileEdge}`) and the swatch bounds (`pattern.options
{showSwatchBounds}`: the part of the tiling the swatch repeats, dashed) are drawn in the preference
`patternTileEdgeColor`.

```json
{"name":"run_command","arguments":{"command":"pattern.options","params":{"showSwatchBounds":true}}}
{"name":"run_command","arguments":{"command":"prefs.set","params":{"key":"patternTileEdgeColor","value":"#ff4f4f"}}}
```

The Eyedropper: `appearance.copyFrom {source}` copies what the Eyedropper Options pick up and apply from `source` to the
selection (`ids`); `reverse: true` copies the selection's attributes onto `source` (Alt-click) and `append: true` adds
the source's fills and strokes on top of each target's stack (Shift+Alt-click). The options live in the preferences:
`eyedropper.setOptions {sampleSize?, pickUp?, apply?}` (or `prefs.get`/`prefs.set` with key `eyedropper`) reads and
sets them; a tree names flags under `appearance` (`transparency`, `fill` `{color, transparency, overprint}`, `stroke`
`{color, transparency, overprint, weight, cap, join, miter, dash}`), `character` and `paragraph`, and a bool for a
branch sets all of it. `paint.sampleColor {color}` puts a sampled colour (in its own model) into the active proxy.

```json
{"name":"run_command","arguments":{"command":"eyedropper.setOptions","params":{"pickUp":{"appearance":{"stroke":{"weight":false}}}}}}
{"name":"run_command","arguments":{"command":"appearance.copyFrom","params":{"source":12,"ids":[7,8]}}}
```

## The Layers panel: rows, layers and sublayers

Every layer, sublayer, group and object is a row of the Layers panel. Sublayers are layers inside layers: they are not
objects, so clicking or marquee-selecting art in a sublayer, Select All and the other Select commands take the art
itself, and each sublayer has its own colour for the selection highlight. `document.inspect` lists the tree (layers
report `color`, `template`, `printable`, `preview`, `dimImages` and `clip`), the `currentLayer` (where new art goes)
and `layerRows`, the rows highlighted in the panel.

- **Clicking rows** (not undo steps): `layer.setCurrent {id}` is a plain click on any row: it alone is highlighted
  and its layer (the row itself when it is a layer or sublayer) becomes current. `layer.highlight {ids, mode?:
  set|add|toggle}` is Shift-click (a range) and Ctrl/Cmd-click (toggle). Selecting art makes its layer current.
  Without `ids`, the panel commands below act on the highlighted rows, else on the current layer.
- **Selection column**: `layer.selectAll {id, add?}` selects a row's visible, unlocked art (a layer's sublayers'
  too); `add` (Shift) adds it, or removes it when it's all selected.
- **Eye and lock**: `layer.setProps {ids?, visible?, locked?, name?}` for any rows, plus Layer Options for layers:
  `template` (also locks the layer and dims its images to 50% unless given), `printable`, `preview` (off: the layer
  draws and is clicked in outline, Ctrl/Cmd-click its eye), `dimImages` (0–100, or false) and `color` (an index 0–26,
  `#rrggbb` or a preset name). Several rows change in one undo step (a drag down the eye or lock column).
- **Dragging rows**: `layer.move {ids, target, place?: above|below|inside, copy?}` moves rows beside a row or into a
  layer or group (on top of its contents), keeping their stacking order; `copy` (Alt-drag) moves copies. Layers go
  only in layers or at the top level and objects in layers and groups (an object placed beside a top-level layer goes
  inside it); a row never goes into itself or a row inside it, and a locked layer or group takes nothing. Dragging the
  selected-art square is `layer.move` with the selected objects. `node.move {id, parent?, index}` is the low-level
  form with the same rules.
- **Buttons**: `layer.new {name?, top?, …options}` (above the current layer at its level; `top` with Ctrl/Cmd),
  `layer.newSublayer {parent?, name?, …options}` (on top of the parent's contents), `layer.delete {ids?}` (rows with
  what they hold; the last layer stays), `layer.locate {id?}` (highlights the selected object's row; the panel opens
  the rows around it) and `layer.clippingMask.toggle`.
- **Panel menu**: `layer.duplicate {ids?}`, `layer.merge {ids?}` (into the layer highlighted last, keeping the
  stacking order), `layer.flatten {id?}` (every other visible layer's art into one layer; hidden layers are deleted,
  templates stay), `layer.collectInNew {ids?}`, `layer.releaseToLayers {id?, build?}` and
  `layer.releaseToLayersBuild` (each object of a layer or group in a sublayer of its own; Build adds up copies),
  `layer.reverse {ids?}`, `layer.template {ids?, on?}`, `layer.hideOthers` / `layer.showAll`,
  `layer.outlineOthers` / `layer.previewAll`, `layer.lockOthers` / `layer.unlockAll` (Alt-clicking an eye or a lock
  runs Hide or Lock Others for the row's layer, or Show or Unlock All when the others already are), `layer.pasteRemembersLayers`,
  `object.isolate {id}` / `object.exitIsolation`.
- **Dialogs and panel state** (control channel and the app's MCP): `ui.layerOptions {ids?}` and `ui.newLayer
  {sublayer?}` open Layer Options (dialog `layerOptions`: name, color, template, locked, visible, printable, preview,
  dimImages, dimPercent; OK is one undo step), `ui.layersPanelOptions` opens Panel Options (`layersPanelOptions`:
  layersOnly, rowSize small|medium|large|other, otherSize, thumbLayers, thumbGroups, thumbObjects), and
  `ui.layersExpand {ids?, open?}` opens or closes rows as their triangles do (Alt-click: everything inside).
  A document opens with only its top-level layers open; the open rows are saved in the native file and reopen that way.

```json
{"name":"run_command","arguments":{"command":"layer.newSublayer","params":{"name":"Shadows"}}}
{"name":"run_command","arguments":{"command":"layer.move","params":{"ids":[12,15],"target":7,"place":"inside"}}}
{"name":"run_command","arguments":{"command":"layer.setProps","params":{"ids":[7],"preview":false,"color":"Orange"}}}
{"name":"run_command","arguments":{"command":"layer.highlight","params":{"ids":[3,7]}}}
{"name":"run_command","arguments":{"command":"layer.merge","params":{}}}
```

## Targeting layers and moving appearances

`layer.target {id}` is the Layers panel's target circle: a layer gets its visible, unlocked art (its sublayers' too) selected and is itself
the target, so `appearance.*`, `effect.*`, `transparency.*` and the opacity-mask commands without `ids` act on the
layer (its opacity, its own fills and effects, an opacity mask on the whole layer); a group or object is simply
selected. `document.inspect` reports it as `target`, and any other selection change ends it.
`appearance.transfer {source, target, copy?}` is dragging a target circle onto another: the target gets the source's
fills, strokes, effects and transparency, and the source is cleared unless `copy` (Alt-drag). Dropping a circle on the
panel's trash is `appearance.clear {ids: [id]}`. Masked objects' names have a dashed underline; while an opacity mask
is edited the panel lists only an `<Opacity Mask>` entry and the document tab says `(<Opacity Mask>/Opacity Mask)`.

```json
{"name":"run_command","arguments":{"command":"layer.target","params":{"id":2}}}
{"name":"run_command","arguments":{"command":"transparency.set","params":{"opacity":50}}}
{"name":"run_command","arguments":{"command":"appearance.transfer","params":{"source":12,"target":2,"copy":true}}}
```

## Expand Appearance

`effect.expandAppearance {ids?, target?}` (Object → Expand Appearance, enabled when a selected or targeted object's appearance isn't basic)
turns appearances into plain objects in one undo step: each visible fill becomes a copy of the path painted by that
fill alone and each stroke its outline filled with its paint (a brushed stroke: its brush art), grouped in paint order
under the object's id; each piece takes its fill's or stroke's opacity and blend mode, and the group keeps the
object's transparency and opacity mask. Geometry effects are baked; raster effects become an embedded image at the
document's raster effects resolution (shadows and outer glows under the art; a blur, feather or inner glow replaces
the object with the image). Type with effects or fills and strokes of its own is outlined. A group's or layer's own
fills and strokes become objects among its members, and its members are expanded too.

```json
{"name":"run_command","arguments":{"command":"effect.expandAppearance","params":{"ids":[12]}}}
```

## Tints of global and spot colours

A colour linked to a global or spot swatch has a tint (the reference app's T slider): `paint.setFill {swatch,
tint?: 0..100}` (and `paint.setStroke`) applies the swatch at that percentage, linked, so `swatch.edit` recolours it
at its own tint. `paint.proxies` shows the paint as `{type: "solid", color, swatch, tint}` (`tint` 0..1, left out at
100 %); `document.inspect` reports it as a label, the swatch name, the tint below 100 % and the colour it gives
(`"G1 40% (#ff9999)"`, `"G1 (#ff0000)"`). `swatch.new {tint?}` with the current fill (or `swatch`) saves a tint swatch,
"Name 40%": `swatch.list` reports it with `tintOf` and `tint`, applying it links to its base at that tint, and it
follows edits of its base. A spot tint prints that percentage of its plate (Separations Preview, PDF Separation
value). `edit.colors.adjustBalance {tint: -100..100}` (Global mode) shifts the tints of the selection's linked
colours and leaves the rest alone.

```json
{"name":"run_command","arguments":{"command":"paint.setFill","params":{"swatch":"Ink","tint":40}}}
{"name":"run_command","arguments":{"command":"edit.colors.adjustBalance","params":{"mode":"global","tint":-20}}}
```

## Linked gradient stops

Applying a gradient swatch (`paint.setFill {swatch}`) records it as the gradient's `swatch` (the Swatches panel
highlights it; new stops drop the link). A gradient stop can link to a global or spot swatch like a solid colour:
give it `swatch` (and `tint` %, default 100 or a tint swatch's own) instead of `color` in `paint.editGradient
{stops}` or a `gradient` paint's stops. Editing or deleting the swatch then recolours or unlinks the stop, in art and
in gradient swatches. A spot stop separates on its plate; a gradient whose stops are all tints of one spot ink (or
paper white, 0 %) exports to PDF as a Separation shading (the writer has no DeviceN, so a gradient mixing a spot
ink with other colours is written in process colours, with a warning).

```json
{"name":"run_command","arguments":{"command":"paint.editGradient","params":{"stops":[{"offset":0,"swatch":"Ink"},{"offset":1,"swatch":"Ink","tint":20}]}}}
```

## Transparency flattener presets

`flattener.presets.list` lists the built-in presets (High, Medium and Low Resolution, `builtIn: true`) and the saved
ones, each with its `options`. `flattener.presets.save {name?, newName?, preset?, …options}` creates or changes a saved
preset (a change starts from the preset's own options; `newName` renames it; built-in presets can't change);
`flattener.presets.delete {name}` deletes one. `flattener.presets.export {names?, path?}` writes them as a
`.vcflattener` JSON file (without `path` it returns `data`), and `flattener.presets.import {path? | data? |
dataBase64?, replace?}` adds a file's presets (names in use get a number unless `replace`). Saved presets live with
the preferences and work as `preset` in `object.flattenTransparency` (and anywhere else flattener options are taken).

```json
{"name":"run_command","arguments":{"command":"flattener.presets.save","params":{"name":"Press","preset":"high","balance":60}}}
{"name":"run_command","arguments":{"command":"object.flattenTransparency","params":{"preset":"Press"}}}
```

## Flattener Preview

`flattener.preview {highlight?, overprints?, preset?, …options, ids?}` reports what flattening the document (or
`ids`) would do without changing it: `counts` of transparent objects, all affected objects, patterns, outlined
strokes and type, rasterized complex regions (areas the raster/vector balance rasterizes whole), all rasterized areas
and flat-colour regions, plus `regions` (bounds, and `id` for objects) for the `highlight` asked for. `overprints:
"discard"` or `"simulate"` flatten without preserving overprints.

```json
{"name":"run_command","arguments":{"command":"flattener.preview","params":{"highlight":"allRasterized","preset":"low"}}}
```

## Width profiles

The Profile list holds the built-in variable-width profiles (`uniform`, `lens`, `taperStart`, `taperEnd`, `pinch`,
`teardrop`, `wave`) and profiles saved from strokes; saved ones are kept with the preferences, not in the document
(no undo step). `stroke.widthProfile.list` returns every row (`id`, `label`, `builtIn`, `points` as `[t, left,
right]`) and `current`, the selected stroke's profile (`"custom"` when it isn't listed).
`stroke.widthProfile.add {name?}` saves the selected stroke's variable width (default name "Width Profile N"),
`stroke.widthProfile.delete {name?}` removes a saved one (default: the selected stroke's; built-ins can't be deleted)
and `stroke.widthProfile.reset` removes every saved one. `stroke.set {profile}` takes a built-in id or a saved name.

```json
{"name":"run_command","arguments":{"command":"stroke.widthProfile.add","params":{"name":"Ribbon"}}}
{"name":"run_command","arguments":{"command":"stroke.set","params":{"ids":[9],"profile":"Ribbon"}}}
```

## Arrowheads

`stroke.set {startArrow, endArrow}` takes any of 40 generated heads by name (the params doc lists them; `null` for
none): arrows (`Arrow`, `Barbed`, `Concave`, `DoubleArrow`, `HalfArrowLeft`/`Right`, `Chevron`, `Feather`,
`Swallowtail`…), filled and open shapes (`Triangle`, `Circle`, `Oval`, `Target`, `Tag`, `Diamond`, `Hexagon`, `Star`,
the `…Open` rings…) and marks (`Bar`, `DoubleBar`, `DotOnBar`, `Slash`, `DoubleSlash`, `Bracket`, `Fork`, `Cross`,
`Plus`). Each head fits a box about four times its weight (stroke weight × `arrowScale`) long and wide.

## Stroke bounds and clicks

Visual bounds (Fit to Selected Art, Rasterize, exporting all art) and clicks on the canvas take
in the whole stroke as drawn: arrowheads, inside/outside alignment (an outside stroke is hit outside the path, an
inside one inside it), the width profile's width where you click, projecting caps and miter spikes up to the miter
limit. A rectangle's right-angle miters stay within half the weight of its edges.

## Expand

`object.expand {object?, fill?, stroke?, gradient?, steps?}` (Object → Expand…) turns the selection into plain art in
one undo step. `object` (on by default) outlines type, turns live shapes into paths and bakes effects; `stroke` (on)
outlines strokes into filled paths; `fill` (on) expands gradient fills: with `gradient: "objects"` (the default) into
`steps` (1–1000, default 255) solid objects (rectangles across a linear gradient, each from its band to the far end so
no seams show; concentric ellipses for a radial one, largest first; just the bands when stops are translucent), with
`gradient: "mesh"` into a gradient mesh that paints the gradient (columns or rings where its colour changes, sharp at
coincident stops), each inside a clip group shaped like the object. A freeform gradient becomes a mesh shaped like the
object either way. Expanded colours keep the colour model their stops share. With `fill: false` gradient fills stay
live. `object.expand.info` answers which options have something to expand in the selection (`{object, fill,
stroke}`), and `object.mesh.create` on a gradient-filled object colours the mesh points as the gradient paints them.

```json
{"name":"run_command","arguments":{"command":"object.expand","params":{"stroke":false,"gradient":"mesh"}}}
{"name":"run_command","arguments":{"command":"object.expand","params":{"steps":16}}}
```

## Lab spot colours

Colours can be CIE Lab (D50): give `{"l": 55, "a": 60, "b": 40}` (L 0–100, a and b about −128–127) wherever a
colour is taken; documents store them as `{"model": "lab", …}`. `swatch.new {color, spot: true}` or
`swatch.edit {name, mode: "lab"}` defines a spot colour in Lab (Swatch Options' Lab mode). `swatch.spotOptions
{useLab}` sets the document's Spot Colors options: with `true` (the default) Lab spot colours show and print from
their Lab values and PDF export writes their Separation spaces with a Lab alternate; with `false` art linked to them
takes their working-CMYK equivalents and PDF uses a DeviceCMYK alternate. Without `useLab` it answers the current
setting.

```json
{"name":"run_command","arguments":{"command":"swatch.new","params":{"name":"Lab Ink","color":{"l":55,"a":60,"b":40},"spot":true}}}
{"name":"run_command","arguments":{"command":"swatch.spotOptions","params":{"useLab":false}}}
```

## Attributes, URLs and image maps

The Attributes panel (`window.panel {panel: "attributes"}`, Cmd+F11) reads `attributes.info {ids?}`:
`overprintFill`, `overprintStroke`, `showCenter`, `imageMap`, `url`, `note`, `fillRule` and `reversed` (null where the
objects differ) and `recentUrls`. `attributes.set {overprintFill?, overprintStroke?, showCenter?, imageMap?, url?,
note?, ids?}` sets them in one undo step; `path.setFillRule {rule: "nonZero"|"evenOdd"}` sets the fill rule of the
paths and compound paths, and `path.reverse {reversed?}` makes subpaths run counter-clockwise (true) or clockwise
(false). SVG export wraps an object with a URL in `<a xlink:href>`, and SVG import reads `<a href>` links back.

```json
{"name":"run_command","arguments":{"command":"attributes.set","params":{"ids":[12],"url":"https://example.com","imageMap":"rectangle"}}}
{"name":"run_command","arguments":{"command":"path.setFillRule","params":{"rule":"evenOdd"}}}
```

## Removing anchor points

`path.removeAnchors {}` (Object › Path › Remove Anchor Points) removes the direct-selected anchors (`select.anchors
{id, anchors: [[subpath, anchor]…], mode?}`) without opening their paths, in one undo step, and answers
`{removedObjects}` (paths left with no segment go). Each removed point's neighbours keep their handle directions and
their facing handles are refitted so one cubic follows the two old segments; two straight sides become one.
`path.removeAnchor {id, subpath?, anchor}` (the Delete Anchor Point tool) removes one anchor the same way, and
`path.deleteAnchors {}` (the Delete key) deletes the selected anchors with their segments, opening closed paths there.

```json
{"name":"run_command","arguments":{"command":"select.anchors","params":{"id":12,"anchors":[[0,1],[0,3]]}}}
{"name":"run_command","arguments":{"command":"path.removeAnchors","params":{}}}
```

## Converting and cutting anchor points

The Control bar's and the Properties panel's anchor buttons, one undo step each, act on the direct-selected anchors
(all anchors of a path selected as a whole): `path.convertAnchors {to: "corner"|"smooth"}` retracts both handles,
or pulls handles out in line with the neighbouring anchors (a smooth anchor keeps its own); `path.cutAtAnchors {}`
(Cut Path at Selected Anchor Points) cuts there and answers `{ids}`: a closed path opens at the cut, its two ends
on top of each other, and an open path becomes one path per piece. Each cut leaves one of its two anchors selected,
so `path.moveAnchors {dx, dy}` (or a Direct Selection drag) pulls the path apart there; `path.join {}` (Connect
Selected End Points) joins the ends again; `path.join {ids: [a, b], ends: ["last", "first"]}` joins the ends asked for
(the Pen's join, #776: drawing on from one open path, a click on another's end makes them one path). `path.convertAnchor {id, subpath?, anchor, to, x?, y?}` and
`path.split {id, subpath?, anchor}` do the same to one anchor (the Anchor Point and Scissors tools); the Pen with Alt
held over a selected path's handle, anchor or segment works as the Anchor Point tool. `path.reshapeSegment {id,
subpath?, segment, t, dx, dy}` (a segment dragged with Direct Selection or the Anchor Point tool) moves the segment's
point at `t` by `dx`, `dy`: a smooth anchor at either end stays smooth, its other handle turning with the moved one.

A `pointer_gesture` with `mods: {cmd: true}` held at the press, with any tool but the selection tools (Command on
macOS, Ctrl elsewhere), drags with the selection tool chosen last (`select_tool`), or Direct Selection with the Pen,
Curvature and anchor tools until one is chosen; the release gives the tool back as it was, so the Pen goes on drawing
the path it had begun while that stays selected. Its reply's `tool` is the tool given back.

```json
{"name":"pointer_gesture","arguments":{"tool":"pen","events":[{"kind":"down","x":100,"y":300},{"kind":"up","x":100,"y":300},{"kind":"down","x":200,"y":300},{"kind":"drag","x":250,"y":300},{"kind":"up","x":250,"y":300}]}}
{"name":"pointer_gesture","arguments":{"mods":{"cmd":true},"events":[{"kind":"down","x":250,"y":300},{"kind":"drag","x":250,"y":260},{"kind":"up","x":250,"y":260}]}}
{"name":"pointer_gesture","arguments":{"events":[{"kind":"down","x":300,"y":350},{"kind":"up","x":300,"y":350}]}}
```

```json
{"name":"run_command","arguments":{"command":"select.anchors","params":{"id":12,"anchors":[[0,2]]}}}
{"name":"run_command","arguments":{"command":"path.cutAtAnchors","params":{}}}
{"name":"run_command","arguments":{"command":"path.moveAnchors","params":{"dx":0,"dy":40}}}
```

## Editing a path with the Curvature tool

The Curvature tool edits any selected path, whatever drew it, keeping its shape except where edited (#798). Each edit
is one undo step and leaves the anchor it edited direct-selected (the tool's current point):
`path.curvatureEdit {id, subpath?, op, …}` answers `{anchor}`. `op: "move"` with `anchor`, `x`, `y` moves a point and
re-curves only its two segments: a smooth point turns along the line through its neighbours, and the neighbours'
handles keep their directions (their lengths scale with the chord), so the segments beyond don't change.
`op: "insert"` with `segment`, `t` adds a smooth point there without changing the shape (with `x`, `y` it is then
moved there: the tool's click-and-drag on a segment). `op: "extend"` with `x`, `y` goes on from the `end` (default)
or the `start` of an open subpath, and `op: "close"` closes it from that end; `from` says how the old end bends into
the new segment: `"keep"` (default) keeps its curve, `"smooth"` curves it through its neighbours (a point the tool
placed), `"corner"` leaves it without handles. Alt-click or double-click toggles a point with `path.convertAnchor`, and
Backspace/Delete removes the current point with `path.removeAnchor`.

```json
{"name":"run_command","arguments":{"command":"path.curvatureEdit","params":{"id":12,"op":"move","anchor":2,"x":260,"y":140}}}
{"name":"run_command","arguments":{"command":"path.curvatureEdit","params":{"id":12,"op":"extend","end":"end","x":550,"y":120}}}
{"name":"run_command","arguments":{"command":"path.curvatureEdit","params":{"id":12,"op":"extend","x":650,"y":60,"from":"smooth"}}}
```

## Registration and trim marks

Every document has the built-in `[Registration]` swatch (listed after None by `swatch.list`): a colour that prints on
every plate, process and spot. `paint.setStroke {swatch: "[Registration]"}` applies it; it can't be edited, moved,
duplicated, merged or deleted. Separations Preview shows it on each plate and PDF export writes it as
`/Separation /All`. `object.createTrimMarks {style?, allArtboards?}` draws trim marks in Registration around the
selection, or around every artboard when nothing is selected; `effect.apply {effect: "cropMarks"}` adds live crop
marks that follow the object. Both use Japanese marks (double lines at the trim and bleed edges, centre marks) when
the preference `japaneseCropMarks` is on or `style: "japanese"` is given.

```json
{"name":"run_command","arguments":{"command":"prefs.set","params":{"key":"japaneseCropMarks","value":true}}}
{"name":"run_command","arguments":{"command":"object.createTrimMarks","params":{}}}
```

PDF export draws printer's marks and bleed itself (the Save PDF dialog's Marks and Bleeds section):
`document.exportPdf {marks: {trim, registration, colorBars, pageInfo, kind, weight, offset}, bleed: {useDocument, top,
bottom, left, right}}`. Each page's TrimBox is its artboard, its BleedBox the artboard grown by the bleed (the
document's with `useDocument`, set by `document.setup {bleed}`; the art there is kept) and its MediaBox the bleed box
grown to hold the marks (the art is clipped to the bleed box). Trim marks, registration targets and the page
information (title, artboard, export date in UTC) are drawn in Registration (`/Separation /All`) outside the bleed;
the colour bars are process, spot and black-tint patches. Layers whose Print option is off (`layer.setProps
{printable: false}`) are left out unless `includeNonPrinting` (or `createLayers`) is on.

```json
{"name":"run_command","arguments":{"command":"document.exportPdf","params":{"path":"/tmp/press.pdf","preset":"Press Quality","marks":{"trim":true,"registration":true,"colorBars":true,"pageInfo":true}}}}
```

The Output and Advanced sections convert colours and write real text. `output: {conversion, destination}` converts
every colour into the destination profile's model (`destination`; CMYK of another profile too) or only the colours of
the other model (`preserveNumbers`: colours already in it keep their numbers); grey stays grey, images are converted
too and printer's marks are not. The destination is any profile `edit.colorSettings` lists (default: the document's
for its colour mode). `profiles` (`all`, `destination`, or `taggedSource` for a document assigned profiles with
`edit.assignProfile`) writes colours in ICC-based spaces with their profiles embedded: CMYK with the destination's or
the document's CMYK profile, RGB as sRGB, grey with the sRGB tone curve. `outputIntent` embeds a profile as the file's
`/GTS_PDFX` output intent with `outputCondition`, `outputConditionId` and `registry`, and `trapped` sets `/Trapped`
(PDF/A files keep their own output intent). Type is written as selectable, searchable text in embedded subset fonts
with a ToUnicode map (fonts whose licence forbids embedding stay outlines, with a warning); `advanced: {outlineText:
true}` writes every glyph as an outline instead.

```json
{"name":"run_command","arguments":{"command":"document.exportPdf","params":{"path":"/tmp/press.pdf","output":{"conversion":"preserveNumbers","destination":"VectorCraft Generic CMYK (SWOP-like)","profiles":"destination","outputIntent":"VectorCraft Generic CMYK (SWOP-like)","trapped":true},"advanced":{"outlineText":false}}}}
```

## Scale Strokes & Effects

`object.scale`, `object.transform` and `object.transformEach` take `strokes?: bool` (Scale Strokes & Effects) and
`corners?: bool` (Scale Corners); without them the preferences `scaleStrokes` and `scaleCorners` (both off by
default) apply (`prefs.set {key, value}`). With strokes on, a scale by k (the square root of the transform's determinant)
multiplies stroke weights, dash lengths and dash offsets, and every distance parameter of the object's, its fills' and
its strokes' effects (drop shadow offsets and blur, offsets, radii…; `effect.list` gives each effect's `lengths`;
relative Roughen, Tweak and Zig Zag sizes stay percentages), in groups and layers too. With it off, nothing painted
changes size, and type keeps its character strokes' weight. With Scale Corners off, live corner radii keep their
size. The journal entry of a scaling command records the `strokes` and `corners` it used, so actions replay alike.

```json
{"name":"run_command","arguments":{"command":"object.scale","params":{"sx":200,"strokes":true}}}
{"name":"run_command","arguments":{"command":"object.transformEach","params":{"scaleH":50,"scaleV":50,"strokes":false}}}
```

## Live Corners

`object.setLiveShape {id?, ids?, radius?, kind?, corners?}` sets the corners of any path (one undo step): a live
rectangle's or polygon's, a star's, a pen path's. A corner is an anchor without handles between two straight sides
(not an open path's ends, not a smooth anchor, not one the sides run straight on through). `radius` (pt) and `kind`
(`round`, `invertedRound` or `chamfer`) go to the `corners` given, else to the corners holding a Direct-Selected
anchor (`select.anchors`), else to every corner. `corners` are anchor indices of the path with its corners uncut,
counting every subpath's anchors in order: a rectangle's 0 top-left, 1 top-right, 2 bottom-right, 3 bottom-left; a
polygon's from its first vertex clockwise; a star's from its first tip; an index past the last anchor is an error.
Each corner keeps its own radius and kind (the shape's `live` in queries has `radii` and, when a corner isn't round,
`kinds`); a corner with no radius is one anchor, a cut one two, and Direct-Selected corners stay selected as that
changes. A path that isn't a live shape keeps its uncut outline (`live` `{"shape": "path", "base", "radii"}`) so its
corners stay editable, and is a plain path again once no corner is cut; a polygon stays a live polygon, its corners
keeping the radius they shared when `sides` changes. Every corner is a circular arc tangent to both sides: a radius
draws no larger than takes the cut halfway along the corner's shorter side (half the shorter side of a rectangle, the
same limit for its four corners), the corners stay circular through uneven scales (the radius scales by the mean scale,
or keeps its size with Scale Corners off), and a rectangle from a file that kept an uneven scale in its transform
(elliptical corners) gets circular ones in document units when its corners are next set. Dragging a corner widget
rounds the corners whose widgets show (every corner, or the Direct-Selected ones), outlining in red those that reach
their limit; the Selection tool shows the widgets of live rectangles and polygons, the Direct Selection tool those of
any path. Alt-clicking one cycles their kind and double-clicking one opens Corners (`ui.corners {id?, corners?}`,
dialog `corners`: `kind`, `radius`; OK runs `object.setLiveShape`).

```json
{"name":"run_command","arguments":{"command":"object.setLiveShape","params":{"id":12,"corners":[1],"radius":16}}}
{"name":"run_command","arguments":{"command":"object.setLiveShape","params":{"id":12,"corners":[0,3],"radius":8,"kind":"chamfer"}}}
{"name":"run_command","arguments":{"command":"shape.star","params":{"cx":200,"cy":200,"radius1":60,"radius2":30}}}
{"name":"run_command","arguments":{"command":"object.setLiveShape","params":{"radius":5}}}
```

The same command sets the rest of a live shape's properties, as the Properties and Transform panels show them. A
polygon (Polygon Properties): `sides` (3–1000), `polygonAngle` (degrees counterclockwise, its first vertex straight up
at 0), `polygonRadius` (pt, centre to vertex), `sideLength` (pt, which sets the radius) and `makeSidesEqual: true`,
which drops an uneven scale or shear (its sides then differ) and keeps its centre, angle and mean radius; a radius or
length that isn't a number above 0 is an error. An ellipse (Ellipse Properties): `pieStart` and `pieEnd` (degrees
counterclockwise from 3 o'clock; 0 to 360 is the whole ellipse) and `invertPie: true`, which swaps them. Each call is
one undo step.

```json
{"name":"run_command","arguments":{"command":"object.setLiveShape","params":{"sides":8,"polygonAngle":22.5,"sideLength":40}}}
{"name":"run_command","arguments":{"command":"object.setLiveShape","params":{"makeSidesEqual":true}}}
```

## Use Preview Bounds

With the preference `usePreviewBounds` on (`prefs.set {key: "usePreviewBounds", value: true}`; also the Align panel
flyout's Use Preview Bounds), the Transform panel's and Control bar's X, Y, W and H, the bounding box, and
`object.align`, `object.distribute` and `object.distributeSpacing` measure visual bounds, which take in the whole
stroke (see Stroke bounds and clicks); off, they measure the paths. The three Align commands also take
`bounds: "preview"|"geometric"` for one call. `object.setBounds` then sets the visual box: with Scale Strokes &
Effects off a 100 pt wide rectangle with a 10 pt stroke set to `width: 220` gets a 210 pt path.

```json
{"name":"run_command","arguments":{"command":"object.align","params":{"horizontal":"left","bounds":"preview"}}}
```

## Selection preferences

The Selection & Anchor Display and General preferences apply to `pointer_gesture` as they do to the mouse:

- `selectionTolerance` (1–8 px, 3 by default): how near a click must be to a path to pick it, and to an anchor,
  handle or segment for Direct Selection, the Anchor Point, Add and Delete Anchor Point and Scissors tools, and the
  Pen with Alt. A handle shorter than that leaves its anchor to be picked when pressed nearer the anchor.
- `objectSelectionByPathOnly`: a click inside a filled path or compound path doesn't select it, one on its path does.
- `ctrlClickSelectsBehind` (on by default): a Selection tool click with `mods: {cmd: true}` (Command on macOS, Ctrl
  elsewhere) selects the object under the selected one there, the next such click the one under that, then the
  topmost again. Cmd held to borrow the selection tool from another tool clicks as usual.
- `doubleClickToIsolate` (on by default): off, a `doubleclick` on a group with the Selection tool no longer isolates it.
- `usePreciseCursors`: the Pen, Eyedropper, Slice and Blend tools' pointers are a crosshair.
- `snapToPointTolerance` (1–8 px, 2 by default): with View › Snap to Point on and Smart Guides off (desktop app), the
  point a selection is dragged by, a drawn point and a transform tool's reference point land on an anchor or a ruler
  guide that near.
- `moveLockedWithArtboard`: `artboard.move {moveArt: true}`, the Artboard tool and `artboard.rearrange` move locked and
  hidden art with the artboard too, and `artboard.setProps {scaleArt: true}` scales it too; off (the default) it stays
  where it is.
- `penRubberBand`, `curvatureRubberBand` (on by default): off, the Pen and Curvature tools draw no preview segment to
  the pointer.
- `showHandlesMultipleAnchors` (on by default): off, Direct Selection (and the Anchor Point tool, and the Pen with Alt)
  shows and drags direction handles only while a single anchor is selected. On, the handles of the direct-selected
  anchors, or of every anchor of a path selected as a whole, show and drag. `handleStyle` (`solid`, `hollow`, `large`) draws their ends (desktop app).
- `hideCornerWidgetAbove` (177° by default): corners wider than this show no Live Corners widget (a rectangle's right
  angles hide below 90°).
- `transformPatternTiles` (off by default): the default of the transforms' `patterns` param (`object.transform`,
  `object.move`, `object.rotate`, `object.scale`, `object.reflect`, `object.shear`, `object.transformEach`, the
  Selection and transform tools, the dialogs' Transform Patterns): pattern fills and strokes move with the art.
- `selectSameTintPercent` (off by default): `select.same.fillColor`, `strokeColor` and `fillAndStroke` take every tint
  of a global or spot swatch; on, only the same tint.

```json
{"name":"run_command","arguments":{"command":"prefs.set","params":{"key":"objectSelectionByPathOnly","value":true}}}
{"name":"pointer_gesture","arguments":{"tool":"selection","mods":{"cmd":true},"events":[{"kind":"down","x":175,"y":125},{"kind":"up","x":175,"y":125}]}}
```

The preferences for the view need the desktop app (`vectorcraft-cli mcp --connect`): `zoomWithMouseWheel` (the
wheel zooms about the pointer, Shift-wheel scrolls up and down, Cmd/Ctrl-wheel sideways; the control channel's
`ui.wheel` turns the wheel), `zoomToSelection` (on: Zoom In and Zoom Out centre the selection), `showToolTips`,
`anchorSize` (1–7), `gridColor`, `gridStyle`, `gridsInBack`, `guideColor`, `guideStyle`, `showPixelGrid` (on by
default: in View › Pixel Preview at 600% zoom and above, a line at every document pixel over the art),
`recentFontsCount`, `antiAliasedArtwork` (on by default; off, the canvas draws the art with hard edges — a pixel is
painted when the art covers at least half of it — while raster effects and pattern tiles stay smooth; exports keep
their own Anti-aliasing option),
`scrubNumericFields` (on: a horizontal drag on a numeric field's label steps the field, one undo step per drag; the
control channel's `ui.drag` scrubs), `showHomeScreen` (on by default: the Home screen while no document is open;
off, an empty window, and the Home button or `app.home` still shows the screen), `autoCollapseIconPanels` (off by
default: on, a click away from a panel popped out of the icon column puts it away) and `displayPrintSize` (off by
default: View › Actual Size / `view.actualSize` is one document point per screen point; on, one document inch fills
96 screen points, an inch at the system's reference density whatever the display scaling).

The Smart Guides preferences (Preferences › Smart Guides) apply to `pointer_gesture` with Smart Guides on (the
default view) and to the mouse; they change what the tools show and how far a target pulls, and only Construction
Guides change where a snapped point lands:

- `smartGuideColor` (`#ff3dfc` by default): the colour of the smart guides' lines and labels.
- `alignmentGuides` (on by default): off, no line is drawn along the edge or centre the art lines up with; the art
  still snaps into line.
- `anchorPathLabels` (on by default): off, no "anchor", "center", "path" or "align" label.
- `measurementLabels` (on by default): off, no size or offset readout while drawing, moving (Free Transform's move
  too), dragging a ruler guide, drawing and resizing an artboard, or moving and resizing a slice.
- `transformToolsGuides` (on by default): off, no readout while scaling or rotating with the Selection tool, the
  Free Transform tool or the Rotate, Scale, Shear and Reflect tools.
- `objectHighlighting` (on by default; desktop app): off, the Selection tools no longer outline the object under the
  pointer. The outline also needs View › Smart Guides on.
- `snappingTolerance` (0–40 px, 4 by default): how near an anchor, edge, centre or artboard edge pulls a drawn point,
  a dragged selection, a bounding-box handle, a ruler guide or an artboard. With Smart Guides off, Snap to Point uses
  `snapToPointTolerance` instead.
- `constructionGuides` (on by default) and `constructionAngles` (`90° & 45° Angles` by default; also `90° Angles`,
  `45° Angles`, `60° Angles`, `30° Angles`, `90° & 45° & 30° Angles`): a point drawn near a line at one of those
  angles through the point it leaves from (the Pen's or Curvature tool's last anchor, a line's start) lands on that
  line, where a target's alignment line crosses it if one does nearby. "90°" gives the horizontal and the vertical,
  "45°" the diagonals, "60°" and "30°" the multiples of those angles in between. Off, the point only lines up.

With Smart Guides on, a bounding-box handle dragged with the Selection or Free Transform tool (`mods.shift`
proportional, `mods.alt` from the centre) lands on another object's anchor or centre (labelled "anchor" or
"center"), else its corners and side line up with the edges and centres of the other objects and the artboards (a
line and "align"). The Rotate, Scale, Shear and Reflect tools start a drag from the anchor or centre they grab and
snap the pointer as drawing tools do, so dragging a corner onto another object's anchor scales exactly onto it:

```json
{"name":"pointer_gesture","arguments":{"tool":"scale","events":[{"kind":"down","x":100,"y":100},{"kind":"up","x":100,"y":100},{"kind":"down","x":199,"y":198},{"kind":"drag","x":302,"y":262},{"kind":"up","x":302,"y":262}]}}
```

Every point a drawing tool places snaps the same way, through one snapper (#506): the Pen's and Curvature tool's
anchors, the Pencil's first and last point, the shape tools' start and dragged corner or radius (`rectangle`,
`roundedRectangle`, `ellipse`, `polygon`, `star`, `lineSegment`, `arc`, `spiral`, `rectangularGrid`, `polarGrid`), the
Type tool's click and frame, and a new artboard's corners. The point lands on another object's anchor, centre or
path ("anchor", "center", "path"), else lines up with the anchors, edges and centres of the art and the artboards (a
line from the target and "align"); the Pen's lands on the path being drawn too, so a click on its first anchor closes
it exactly. With `mods.shift` the point keeps to its 45° step from the anchor before it (from `constrainAngle`; a box
dragged keeps its corner on a diagonal) and slides along that way into line with the nearest target. A `move` event
before the press shows where it would go, as hovering does with the mouse. The targets are the art in the window (all
of it headless), gathered once per document state; the shape being drawn is never one of them:

```json
{"name":"pointer_gesture","arguments":{"tool":"rectangle","events":[{"kind":"down","x":202,"y":203},{"kind":"drag","x":330,"y":341},{"kind":"up","x":330,"y":341}]}}
{"name":"pointer_gesture","arguments":{"tool":"pen","events":[{"kind":"down","x":300,"y":420},{"kind":"up","x":300,"y":420},{"kind":"down","x":151.5,"y":431},{"kind":"up","x":151.5,"y":431}]}}
```

With a 100 × 100 square at (100, 100), the first starts the rectangle on its corner (200, 200); the second puts the
Pen's second anchor at (150, 431), in line with the square's centre.

Not read yet: Spacing Guides (the tools draw none).

## Type preferences

- `placeholderText` (on by default): type the Type tools place (`pointer_gesture` with `type`, `areaType`…) starts with
  placeholder text, selected, so `type_text` replaces it; `text.create` and `text.createInPath` take `placeholder: true`.
- `typeSizeIncrement`, `trackingIncrement`, `baselineShiftIncrement`: what `type.step {attribute: "size" | "leading" |
  "tracking" | "kerning" | "baselineShift", by?}` steps by, on the text range given, the Type tool's selected text or the
  selected type objects. `type.size.increase` / `type.size.decrease` (Cmd+Shift+. and Cmd+Shift+,) step the size; the
  Type tool's Alt+arrows (`press_key {key: "Right", mods: {alt: true}}`) step kerning at a caret or tracking of the
  selection (←/→), leading (↑/↓) and baseline shift (Shift+↑/↓), five steps with Cmd/Ctrl too.
- `missingGlyphProtection` (on by default): `text.setStyle` and `text.setRangeStyle` changing the font leave the
  characters the new font has no glyph for in the font that had one.
- `typeSelectionByPathOnly` (off by default): on, the Selection tools (`pointer_gesture` with `selection`, and the
  mouse) pick type on its type path only — the baseline of each of its lines (a vertical column's centre line), area
  type's frame, type on a path's path — within `selectionTolerance`; a click among the glyphs, between the lines or
  inside the frame selects nothing. Off, anywhere in the type's bounds selects it. Marquee selection is unchanged, and
  the Type tools (a click among the characters edits them) and the Eyedropper (`eyedropper`) are not affected.
- `fontNamesInEnglish` (on by default): Preferences › Type › Show Font Names in English. Off, the Character panel and
  Type → Font menus label a family by its native-language name when the font has one. `text.fontList` and
  `text.setStyle {font}` always use the English (canonical) family name; a document may still name a font by any of
  its aliases (`ヒラギノ角ゴシック` resolves to Hiragino Sans).

```json
{"name":"run_command","arguments":{"command":"prefs.set","params":{"values":{"typeSizeIncrement":4,"trackingIncrement":50}}}}
{"name":"run_command","arguments":{"command":"prefs.set","params":{"key":"fontNamesInEnglish","value":false}}}
{"name":"run_command","arguments":{"command":"type.step","params":{"attribute":"tracking","by":-2}}}
```

## Width points

`stroke.widthPoint.set {id, t, left, right, index?, adjustAdjoining?}` adds a width point (side widths in points) or,
with `index`, edits or moves one; `adjustAdjoining` changes the nearest points either side in proportion. Moving a
point onto another one's `t` (or `stroke.widthPoint.copy {id, index, t}` landing there) makes a discontinuous point:
two points at the same `t` (the width before it, then after it), and the stroke's width steps there.
`document.inspect` reports a path's `strokeOptions.widthPoints` (`[t, left, right]`, factors of half the weight; null
for a uniform stroke). `stroke.widthPoint.remove {id, index | indices}` deletes points; `stroke.widthProfile.set {ids,
points}` replaces them all, as dragging several Shift-selected points with the Width tool does in one undo step.
With the Width tool, `press_key` Delete or Backspace removes the selected width points (`handledBy: "tool"`); with none
selected the key clears the selected objects as usual (`edit.clear`). The Puppet Warp tool's selected pins go the
same way.
A compound path's stroke is the compound's: width point commands given one of its members edit the compound's
profile, which runs along every subpath (the Width tool shows each subpath's points).

`object.puppetWarp {ids?, pins, moved, angles?, expand?}` warps art as rigidly as possible so that each pin (a point
on the art) lands on its `moved` point. `angles` (degrees, or null for a free pin; as long as `pins`) holds the turn
the art takes around a pin, as Alt-dragging near a selected pin with the Puppet Warp tool does: one pin with
`angles: [90]` turns the art a quarter turn around it.

The Puppet Warp tool's pins live in the document while the same art stays selected (never saved; Undo and Redo take
them back with the art). `object.puppetWarp.pins {ids?}` reads them: `pins` (where each sits on the shape the pins
started from), `moved` (where it is now), `angles`, and `auto: true` while none was placed (the tool's automatic pins:
the centre and the end of each limb). Edit `moved`, `angles` or the lists and pass the result back to
`object.puppetWarp`: with `rest: true` the warp always starts from that rest shape, so warps don't stack and putting
a pin back restores the original; every pin must be on the art's mesh, and empty lists remove the pins.

```json
{"name":"run_command","arguments":{"command":"object.puppetWarp.pins","params":{}}}
{"name":"run_command","arguments":{"command":"object.puppetWarp","params":{"rest":true,"pins":[[250,150],[110,150],[390,150]],"moved":[[250,150],[110,150],[390,90]],"angles":[null,null,null]}}}
```

```json
{"name":"run_command","arguments":{"command":"stroke.widthPoint.set","params":{"id":9,"t":0.5,"left":4,"right":4}}}
{"name":"run_command","arguments":{"command":"stroke.widthPoint.set","params":{"id":9,"t":0.8,"left":12,"right":12}}}
{"name":"run_command","arguments":{"command":"stroke.widthPoint.set","params":{"id":9,"index":1,"t":0.8,"left":4,"right":4}}}
```

## New art

New objects take the fill and stroke proxies (`paint.setFill` / `paint.setStroke` and the weight, whatever is
selected) on top of a template. With nothing selected, `stroke.set` / `stroke.setAdvanced` (cap, join, dashes,
arrowheads, profile…) and `graphicStyle.apply` (every fill, stroke and effect, opacity, blend mode and the link to the
style; `add: true` stacks it) set that template instead of editing art. With the Appearance panel's New Art Has Basic
Appearance off (`appearance.setNewArtBasic {on: false}`, the preference `newArtBasic`) new art takes the whole
appearance of the last selected object. `appearance.newArt` reports what the next object gets; `paint.default`
resets it.

```json
{"name":"run_command","arguments":{"command":"select.none","params":{}}}
{"name":"run_command","arguments":{"command":"stroke.set","params":{"cap":"round","dash":[6,3]}}}
{"name":"run_command","arguments":{"command":"appearance.newArt","params":{}}}
```

## Gradients on strokes

A linear or radial gradient on a stroke lies within it (placed on the page like a fill's gradient: the default), along
it (from the start of each subpath to its end) or across it (from the stroke's left edge to its right edge, left of the
path's direction; an inside or outside stroke spans the side it shows on). `paint.editGradient {stroke: true,
strokeMode: "within"|"along"|"across"}` sets it (the Gradient panel's Stroke buttons), and `document.inspect` reports it
as `strokeOptions.gradientMode`. Width profiles, dashes and arrowheads keep working: the gradient runs on through gaps
and into the heads. Outline Stroke and Expand turn such a stroke into gradient meshes clipped to its outline; SVG and PDF
write it as slices of linear gradients clipped to the outline (`vectorcraft_svg::export_with_report` and the PDF
report warn about it).

```json
{"name":"run_command","arguments":{"command":"paint.editGradient","params":{"stroke":true,"strokeMode":"along"}}}
```

## Units

Commands take and return lengths in points (the distances of `object.path.offsetPath`, `object.path.simplify`,
`object.path.splitIntoGrid` and `text.setStyle` `size`/`leading` also take a string with a unit, `"5 mm"`). The units
only change what the UI shows and how it reads typed numbers. Preferences ▸ Units has
three: General (rulers, positions and sizes, the Info panel, dialog distances, canvas measurement labels), Stroke
(weights, dashes) and Type (font size, leading, baseline shift). General is the active document's units
(`document.inspect` → `units`): `document.setUnits {units}` (Document Setup) sets it for that document, and
`prefs.set {key: "unitsGeneral", value}` sets it for the active document too (one undo step) and is the units
`file.new` starts in when it gets no `units`. Stroke and Type are the preferences `unitsStroke` and `unitsType`.
`numbersWithoutUnitsArePoints` (on by default) makes the desktop app's length fields in picas read a number typed
with no unit in points (`12` is 12 pt, `2p6` and `1p` stay picas, `12 mm` millimetres); off, it is in picas. Fields in
other units read it in their unit either way, and the Preferences dialog dims the option unless a unit is Picas.
Command params are in points either way.
`prefs.list` marks the length preferences (`keyboardIncrement`, `cornerRadius`, `pasteOffset`, `gridlineEvery`,
`typeSizeIncrement`, `baselineShiftIncrement`) with `measure: "general"|"type"`; they are kept in points and also take
a string with a unit. Unit names: `Points`, `Picas`, `Inches`, `Millimeters`, `Centimeters`, `Pixels`, `Feet & Inches`,
`Meters`, `Yards`, `Feet` (the preferences use `points`, `picas`, … `feetInches`, `meters`, `yards`, `feet`).

```json
{"name":"run_command","arguments":{"command":"prefs.set","params":{"key":"unitsGeneral","value":"millimeters"}}}
```

## Document Setup

`document.setup` with no params reports the document setup: units, the bleed ([top, bottom, left, right] in pt, drawn
as a red outline around each artboard), the transparency grid (size and two colours; the first is also the simulated
paper colour), the flattener preset and Discard White Overprint (Overprint Preview keeps white overprints visible while
it is on), the type options (language and its quotes, Use Typographer's Quotes for typed quotes, superscript, subscript
and small caps proportions, SVG text export) and the background contents (white: raster exports are white behind the
art). Pass any of these keys to change them in one undo step; `document.setUnits` stays as an alias for `units`.

```json
{"name":"run_command","arguments":{"command":"document.setup","params":{"gridColors":"Blue","gridSize":"large","language":"German","bleed":9}}}
```

## New Document

`file.new` takes everything New Document sets: `preset` (a name from `file.newPresets`), `name`, `width`/`height`
(points, or lengths such as `"210 mm"`), `units`, `orientation`, `artboards` with `artboardLayout {layout, columns,
spacing, rightToLeft}`, `bleed`, `backgroundContents`, `colorMode`, `rasterEffectsPpi` and `previewMode`.
`file.newPresets {category?}` lists the categories and presets (Recent: the last sizes used; Saved: the user's presets,
kept in the preferences by `file.newPresets.save` and removed by `file.newPresets.delete`). A listed preset can be
passed straight back to `file.new`. Print presets and sizes without `units` start in `unitsGeneral`; screen presets
(mobile, web, video, social) in Pixels.

```json
{"name":"run_command","arguments":{"command":"file.new","params":{"preset":"A4","orientation":"landscape","artboards":4,"artboardLayout":{"columns":2},"bleed":9}}}
```

## Place

`file.place` puts another file's art into the active document as one undo step without touching the clipboard:
a raster image at 100% of its physical size (the resolution its file declares, else 72 ppi; linked to its `path`
unless `link: false`; a Photoshop document as its merged image), an SVG as one group, a PDF/.ai page as one clipped group, with the images, symbols, patterns
and swatches it uses. A VectorCraft document read from `path` is a placed document: one locked object showing its
artboard `page` (or, with `crop: "bounding"`, its art's bounds), linked to the file and read again when it changes
(see Linked images), and vectors in every output; with `link: false` (or from `dataBase64`) it is an editable copy
of its art, as one clipped group. `at` centres it, `rect` fits it, `replace`
swaps the selected object (keeping its place and transform), `template` puts it on a new template layer.
`file.place.info` describes a file without placing it and `image.info` reports a placed image's link, colour mode and
effective ppi. `file.place.queue` loads the place cursor (the `place` tool) with several files: headless, drive it
with `pointer_gesture` (a click places at 100% with the top-left corner there, a drag at the dragged size) and
`press_key` (Left/Right/Up/Down cycle, Escape discards the current file).

```json
{"name":"run_command","arguments":{"command":"file.place","params":{"path":"/tmp/photo.jpg","at":[300,200]}}}
{"name":"run_command","arguments":{"command":"file.place","params":{"name":"logo.svg","dataBase64":"PHN2Zy…","rect":[0,0,100,100]}}}
{"name":"run_command","arguments":{"command":"file.place.queue","params":{"paths":["/tmp/a.png","/tmp/b.pdf"]}}}
{"name":"pointer_gesture","arguments":{"events":[{"kind":"down","x":40,"y":40},{"kind":"up","x":40,"y":40}]}}
```

## Copy and paste between documents

`edit.copy` keeps the copied objects with the document resources they use: image blobs, symbols, patterns, global
and spot swatches (with the tint swatches of the tints used; never the built-in [Registration] swatch), the gradient
swatches their gradients came from, graphic styles the objects are linked to, character and paragraph styles and
brushes. Width profiles live in the preferences, so they need no copying. Every `edit.paste*` command (also `edit.pasteWithoutFormatting`, which leaves
text styles behind) brings them into the active document in the same undo step: an identical resource is reused,
a missing one added, and one of the same name that differs is added renamed ("Mark 2"; a copy an earlier paste
added is reused), which the pasted objects follow. Linked colours show the document's swatch at their tint (a Lab
spot colour as its Spot Colors option says). The result reports `{ids, added, merged, renamed: [{kind, from,
to}]}`.

A global or spot swatch whose name the document gives another colour is a conflict: `clipboard.conflicts` lists
them, and `swatchConflict` answers: `"merge"` (the default: the objects take the document's swatch), `"add"` (the
pasted swatch comes in renamed) or one answer per name. Pasting back into the document the objects came from uses
its resources as they are now and raises no conflict.

Placement: `edit.paste {center}` centres the objects on a point (the app passes the view centre), else offsets them
by `dx`/`dy` (the Paste Offset preference). With nothing selected, `edit.pasteInFront` / `edit.pasteInBack` put them
on top / at the bottom of the current layer. `edit.pasteOnAllArtboards` keeps their offset to the artboard they were
copied from. `layer.pasteRemembersLayers {on?}` (a document option, `document.inspect` → `pasteRemembersLayers`)
pastes objects back into the layers they came from, by name, making missing ones.

Artboards copy with their art. `artboard.copy {index?, art?}` puts an artboard on the clipboard together with the
art fully inside it (`art` defaults to the Artboard tool's `moveArt` option, Move/Copy Artwork with Artboard; locked
and hidden art only with prefs `moveLockedWithArtboard`); with the Artboard tool chosen, `edit.copy` does this for
the tool's artboard and `edit.cut` runs `artboard.cut`, which also deletes them (never the only artboard). Then
`edit.paste` adds a copy of the artboard right of the last one, with its art in the layers it came from, in one undo
step (`edit.pasteInPlace`, `edit.pasteInFront` and `edit.pasteInBack` put it where it was; `edit.pasteOnAllArtboards`
pastes only the art), in this document or another; the result's `artboard` is the new artboard's index.
`artboard.duplicate {index?, art?}` (Window › Artboards › Duplicate Artboards, or a row dragged onto New Artboard)
does the same in one step → `{index, ids}`, and `artboard.move {copy: true, moveArt: true}` is the Artboard tool's
Alt-drag.

Scale Artwork with Artboard (#602): `artboard.setProps {index, x?, y?, width?, height?, scaleArt: true}` resizes an
artboard and scales the art fully inside it, and its artboard guides, from the old rectangle onto the new one, each side
by its own ratio (keep the proportions for an even scale), in one undo step → `{scaled: [id…]}`. Locked and hidden art
scales only with `lockedAndHidden: true` (default: prefs `moveLockedWithArtboard`); strokes, effects and corners scale
as with `object.scale` (`strokes?`, `corners?`, default: Scale Strokes & Effects and Scale Corners), pattern tiles with
`patterns?` (default: Transform Pattern Tiles). A move that keeps the size scales nothing (art moves with
`artboard.move {moveArt}`). The Artboard tool's `scaleArt` option (`tool.setOption {tool: "artboard", key: "scaleArt",
value: true}`; the Control bar's and Properties' Scale Artwork with Artboard check box, beside Move Artwork with
Artboard) makes its handle drags proportional and passes `scaleArt`, as do Properties' and the Transform panel's
artboard size fields while the tool is in use.

```json
{"name":"run_command","arguments":{"command":"artboard.copy","params":{"index":0}}}
{"name":"run_command","arguments":{"command":"edit.paste","params":{}}}
{"name":"run_command","arguments":{"command":"artboard.duplicate","params":{"index":0,"art":true}}}
{"name":"run_command","arguments":{"command":"clipboard.conflicts","params":{}}}
{"name":"run_command","arguments":{"command":"edit.paste","params":{"center":[300,200],"swatchConflict":{"Brand":"add"}}}}
{"name":"run_command","arguments":{"command":"layer.pasteRemembersLayers","params":{"on":true}}}
```

## File Info

`file.info` with no params reports the document's File Info: `title`, `author`, `authorTitle`, `description`,
`keywords`, `rating` (0–5), `copyrightStatus` (`unknown`, `copyrighted`, `publicDomain`), `copyrightNotice`,
`copyrightUrl`, and the read-only `created` (set by `file.new`) and `modified` (set by every save to a file) dates as
ISO 8601 UTC. Pass any of the editable keys to change them in one undo step (`null` clears one; `keywords` takes a
list or a comma-separated string and keeps each word once); bad values are refused. PDF export writes the title,
author, description and keywords to the document info and XMP, PNG export writes them (with the copyright and
creation date) as text chunks, and SVG export writes the description as `<desc>` and, with the `metadata` option,
all of it as Dublin Core. `file.new {created}` and a save to a file (`document.save` or `file.saveAs` with
`{path, modified}`) use the given date (Unix seconds, or `null` for none) instead of now. Their journal entries always
record the date they used, so replaying the journal makes the same document whenever it runs (an action recorded in
the Actions panel leaves the date out: playing it later dates the document then). As a step of `command.batch`, a
command records the date it used (and any value it takes from the preferences) in its step of the batch's journal entry.
A batch may open, switch, close or revert documents: each document it edits gets one undo step, the batch is
journaled whichever document it ends in, and an error rolls every document back (the ones it opened close, the ones
it closed or reverted come back).

```json
{"name":"run_command","arguments":{"command":"file.info","params":{"author":"Ada","keywords":"poster, fair","rating":4,"copyrightStatus":"copyrighted","copyrightNotice":"© 2026 Ada"}}}
```

## Document Raster Effects Settings

`document.rasterEffectsSettings` holds how raster effects (shadows, glows, blurs, feathers) become images when PDF
export or Expand Appearance renders them: `resolution` (ppi, also New Document's `rasterEffectsPpi`), `colorModel`
(the document's `rgb`/`cmyk`, `grayscale` or `bitmap`), `background` (`white` makes the images opaque), `antiAlias`
(off: hard edges), `clippingMask` (the white stays under the art only), `addAround` (points of room around the art)
and `preserveSpotColors` (stored). They are also the defaults of `object.rasterize`, whose params override them (its
`clippingMask` puts the image in a clip group with the art's outline). No params reports them; any of them changes them
in one undo step. PDF export renders the raster effects of every kind of object this way (paths, groups, layers, type,
images, symbol instances, live objects, and those in opacity masks, symbols and pattern tiles): shadows and outer glows
go in an image under the untouched vector art, other effects replace the object with its image, and a path whose
fills or strokes carry effects is written as one piece per fill and stroke, so only the affected ones become images.

```json
{"name":"run_command","arguments":{"command":"document.rasterEffectsSettings","params":{"resolution":"high","background":"white","addAround":36}}}
```

## Linked images

A placed image linked to its file (`file.place {path}`, Link on) records the file's absolute path, its size,
modification time and hash, and keeps a low-resolution preview (at most 256 px a side): a `.vectorcraft` save writes
the preview instead of the pixels, plus the path relative to the saved file. `document.open` reads
the linked files again, looking for each at its path, then at its relative path and by name in the document's folder
(so a folder moved with its links still opens): the result lists `missingLinks` (their images show the preview),
`modifiedLinks` (left as they were; read again only with the preference `updateLinks: "automatically"`, then they are
in `updatedLinks`), each as `{name, path, ids}`. The desktop app also checks the active document's links when its window
comes back to the front, following the same preference. `links.check` reports every link's `status` (`ok`, `modified`,
`missing`), `links.update {ids?}` reads modified files again and `links.relink {ids?, path | folder}` points images at
another file (or each at the file of its name in a folder); images keep their bounds, one undo step each. Without a
file system (the web), linked images show their previews.

Placed documents (a `.vectorcraft` file placed linked) work the same way: the document keeps the file's bytes and a
preview (JPEG or PNG, 1 px per point, at most 1024 px a side), and a save writes the preview (Include Linked Files:
the file); output reads the file again, or uses the preview with a warning when it is gone or changed.
`links.list` and `links.info` give them `document: true` and the artboard's `pageWidth` and `pageHeight` (pt).
Saving the file in the app updates the open documents that place it. `links.embed` (the Links panel's Break Link)
turns one into the editable copy placing without link gives, its symbols, patterns, swatches and images joining
the document; so does `object.expand`.

```json
{"name":"run_command","arguments":{"command":"links.check","params":{}}}
{"name":"run_command","arguments":{"command":"links.relink","params":{"ids":[12],"path":"/new/photo.png"}}}
{"name":"run_command","arguments":{"command":"links.update","params":{}}}
```

Placing a text file (`.txt`) sets it as area type: `file.place {path | name+dataBase64, text?: {characterSet?:
"unicode" | "ansi", platform?: "windows" | "mac", removeLineReturns?, removeParagraphReturns?, replaceSpaces?: n}}`
(Text Import Options). Unicode reads UTF-8, or UTF-16 with a byte-order mark; ANSI reads Windows-1252 (Mac Roman on
`mac`). `removeLineReturns` joins the lines of each block into one paragraph, `removeParagraphReturns` drops blank
lines, `replaceSpaces: 3` turns runs of 3 or more spaces into tabs. The frame fills `rect`, the replaced object's
bounds, or the artboard less a 36 pt margin.

```json
{"name":"run_command","arguments":{"command":"file.place","params":{"path":"/tmp/notes.txt","text":{"removeLineReturns":true,"removeParagraphReturns":true}}}}
```

## PDF presets

`pdf.preset.list` lists the built-in presets (`VectorCraft Default`, which preserves editing, then High Quality Print,
Press Quality, Smallest File Size and the PDF/X presets, `builtIn: true`; `supported` says the writer produces the
preset's standard) and the saved ones, each with its `description` and `settings`. `pdf.preset.save {name?,
newName?, description?, preset?, …document.exportPdf options}` creates or changes a saved preset (a change starts from
the preset's own settings; `newName` renames it; built-in presets are read-only; passwords are never stored);
`pdf.preset.delete {name}` deletes one. `pdf.preset.export {names?, path?}` writes them as a `.vcpdfpresets` JSON file
(without `path` it returns `data`), and `pdf.preset.import {path? | data? | dataBase64?, replace?}` adds a file's
presets (names in use get a number unless `replace`). Saved presets live with the preferences and work as `preset`
in `document.exportPdf`, `document.pdfSettings`, `document.export`/`serialize` with format `pdf` and `.ai` saves.

```json
{"name":"run_command","arguments":{"command":"pdf.preset.save","params":{"name":"Web","preset":"Smallest File Size","compatibility":"1.5"}}}
{"name":"run_command","arguments":{"command":"document.exportPdf","params":{"preset":"Web","path":"/tmp/web.pdf"}}}
```

## Clipboard formats

Besides SVG, Copy offers other apps PNG and PDF of the copied objects, and their text when they are all type.
`clipboard.flavours` lists what Copy offers for the current clipboard, best first, by the Clipboard Handling
preferences: `text/plain` (the text of a type-only copy, else the SVG markup with `copyAsSvg`), `image/svg+xml`
(`copyAsSvg`), `application/pdf` (`copyAsPdf`) and `image/png` (always; PDF and PNG need objects with an
area). Each one has a query command:
`clipboard.exportSvg`, `clipboard.exportText` (null unless every copied object is type), `clipboard.exportPdf`
(a one-page PDF, the page the objects' visual bounds) and `clipboard.exportPng {scale?}` (cropped to the objects,
transparent around them).

What other apps copied goes into the clipboard with `clipboard.importSvg`, `clipboard.importPdf {dataBase64, page?,
password?}` (the page's objects), `clipboard.importImage {dataBase64, mime?}` (an embedded image at 100% of its
physical size) or `clipboard.importText {text}` (point text in the default type style), each centred on `center`
(default: the first artboard); then any `edit.paste*` command pastes it. The desktop app does this itself: Copy and
Cut publish the formats to the system clipboard, and a Paste outside text editing first reads what another app
copied (SVG, then PDF, then text, then a bitmap; SVG markup in text counts as SVG). Pasting what VectorCraft copied
keeps the lossless internal clipboard. Windows carries every format both ways and pastes a bitmap from PNG, else
from the device-independent bitmap screenshots copy (`CF_DIBV5`, then `CF_DIB`; alpha kept, opaque when it is zero
throughout, at most 32768 px a side); macOS and Linux carry one format (the text, else the PNG) and paste text and
bitmaps (macOS the pasteboard's PNG, else TIFF, as screenshots copy them). On the web Copy publishes SVG text, and
Paste takes SVG text and the pictures and files a paste carries (a screenshot pastes as an embedded image).

The `pasteTextFormatting` preference (`keep` or `plain`): with `plain`, text the Type tool copied pastes into type
without its formatting, taking the style at the caret.

```json
{"name":"run_command","arguments":{"command":"clipboard.flavours","params":{}}}
{"name":"run_command","arguments":{"command":"clipboard.exportPng","params":{"scale":2}}}
{"name":"run_command","arguments":{"command":"clipboard.importImage","params":{"dataBase64":"iVBORw0KGgo…","center":[300,200]}}}
{"name":"run_command","arguments":{"command":"edit.paste","params":{"center":[300,200]}}}
```

## DXF export

`document.exportDxf` (also `document.export` / `export` with format `dxf`) writes a CAD drawing, ASCII DXF R12 to 2018
(`version`: `R12`, `R13`, `R14`, `2000`, `2004`, `2007`, `2010`, `2013`, `2018`; default `2018`). Each layer becomes a
DXF layer (hidden layers switched off, locked ones locked, non-printing ones not plotted; template layers left out),
straight paths become polylines, curved ones cubic splines through every anchor, fills their outlines followed by
solid hatches (laser and cutter software reads the outlines and skips hatches; R12 has no hatches: the outlines
alone), strokes lines with their lineweight and a linetype per dash pattern, and placed images image entities linked
to PNG or JPEG files (`rasterFormat`) written next to the drawing (`linked` in the result). Coordinates are y up
from the bottom-left corner of the first artboard (or `artboard`), in drawing units: `scale` units per `unit` (default
1 mm = 1 unit, which sets `$INSUNITS`); `scaleLineweights` scales the lineweights with them. `colors` is `8`, `16` or
`256` indexed colours, or `true` (default; true colour with the nearest index, DXF 2004 and later). `preserve:
"appearance"` (default) writes type as glyph outlines and the strokes a CAD line can't draw (inside or outside,
width profiles, arrowheads) as filled outlines; `"editability"` keeps type as text and every stroke a line.
`alterPaths` writes every stroke as its filled outline, `outlineText` outlines type, `selectedOnly` writes only the
selected objects in their layers (every export takes it), and `useArtboards: true` writes one drawing per chosen
artboard holding the art over it. What DXF can't hold (gradients and patterns as one colour, blending, opacity masks,
raster effects, clipping) comes back in `warnings`.

DWG can't be written (no openly licensed writer exists): `document.formats` lists it under `unsupported` with that
hint, and exporting to it answers with the hint to export DXF. PICT files are not supported either and say so when
opened.

```json
{"name":"run_command","arguments":{"command":"document.exportDxf","params":{"path":"/tmp/plan.dxf","version":"2013","unit":"mm","scale":10,"preserve":"editability"}}}
{"name":"export","arguments":{"path":"/tmp/plan.dxf","options":{"useArtboards":true,"colors":256}}}
```

## DXF import

`document.open` reads ASCII DXF drawings of any version (by content or the `.dxf` extension), and `file.place`
places one as a group. Both take `dxf: {layout, unit, scale, fit, fitTo, scaleLineweights, center, mergeLayers}`
(all optional). `document.dxfInfo` reads a drawing without opening it: its `version`, drawing `units`, `layouts`
(`Model` first, then the paper layouts), `layers`, and the default ratio (`unit`, `scale`: the drawing at 1:1 in its
own unit; unitless metric drawings read millimetres, imperial ones inches). A ratio is 1 `unit` of the art = `scale`
drawing units; `fit: true` instead scales the art to fit `fitTo` (a letter page when opening; when placing, the
artboard under `at`). `center: false` puts the drawing's origin (fitted art: its bottom-left corner) on the
artboard's bottom-left corner; `mergeLayers` puts all art on one layer. Each DXF layer holding art becomes a layer
(off and frozen layers hidden, locked ones locked, non-plotting ones non-printing). Lines, polylines (arc segments from
bulges), circles, arcs, ellipses, splines (NURBS, within a millionth of their size), solids and hatches become paths (solid
hatches filled, pattern hatches as their boundaries), text and multiline text point type, named blocks symbols
(one per look when their art takes colour "by block"), anonymous blocks such as dimensions groups. Indexed colours
(7 is black on paper), true colour, lineweights (`scaleLineweights` scales them with the art), linetypes and
transparency resolve through layers and blocks. What is left out (viewports, images, meshes, unknown entities) is
listed in `warnings`. Binary DXF and DWG files answer an error saying to save them as ASCII DXF.

```json
{"name":"run_command","arguments":{"command":"document.dxfInfo","params":{"path":"/tmp/plan.dxf"}}}
{"name":"run_command","arguments":{"command":"document.open","params":{"path":"/tmp/plan.dxf","dxf":{"layout":"Layout1","fit":true,"mergeLayers":true}}}}
{"name":"run_command","arguments":{"command":"file.place","params":{"path":"/tmp/detail.dxf","dxf":{"unit":"mm","scale":10,"center":false}}}}
```

## Links panel

`links.list {show?: all|missing|modified|embedded, sort?: name|kind|status}` lists every image in the layers, top
first, as the Links panel does: `{id, name, linked, status: ok|modified|missing|embedded, format, pixelWidth,
pixelHeight, path?, found?, page?, preview?}`. `links.info {id?}` is the Link Info: `image.info`'s fields plus
`status`, `format`, the file's `ppi` and the `effectivePpi`, `scale` (% of 100%), `rotation` (degrees,
counter-clockwise), `placement` and, for a linked file, `fileName`, `location`, `fileSize`, `created`, `modified`.
`links.goTo {id}` selects an image; `links.embed {ids?}` keeps linked files' pixels in the document (an image whose
file is missing stays linked: relink it first); `links.unembed {id, path}` writes an embedded image to a file
(another extension converts it) and links to it, and without `path` returns `{name, dataBase64}`.
`links.placementOptions {ids?, preserve?: transforms|bounds|fileDimensions|fit|fill, align?: topLeftÃ¢â‚¬Â¦bottomRight,
clip?}` decides how a relinked or updated file takes an image's place (default `bounds`: stretched into the old
bounds; `clip` puts it in a clip group of the old bounds when it is larger). The UI commands `links.editOriginal` and
`links.reveal` open the linked file in its app or show it in its folder (desktop).

```json
{"name":"run_command","arguments":{"command":"links.list","params":{"show":"missing"}}}
{"name":"run_command","arguments":{"command":"links.placementOptions","params":{"ids":[12],"preserve":"fit","align":"top"}}}
{"name":"run_command","arguments":{"command":"links.unembed","params":{"id":14,"path":"/tmp/logo.png"}}}
```

## Package and Document Info

`file.package {folder?, name?, copyLinks?, linksFolder?, relink?, copyFonts?, report?}` copies a saved document (an
unsaved one is an error) into `folder/name` (default name `<document> Folder`): `<document>.vectorcraft`, its linked
files in `Links/` (relinked: the packaged document points at the copies; the open one doesn't change), the fonts its
type uses in `Fonts/` (fonts whose licence doesn't allow embedding are listed in `skippedFonts` instead) and
`<document> Report.txt`. A placed `.vectorcraft` document is packaged with its own linked files and fonts, relinked to
the copies (each file copied once); one that can't be read is copied as it is, and a cycle of placed documents or one
past the nesting limit stops there, each with a line in `warnings` and the report. Every option defaults to true. Without `folder` (the web, or an agent that wants the bytes) the
result carries the same files as a zip (`{name: "<name>.zip", dataBase64}`, entries under `<name>/`).

`document.info {selectionOnly?, category?, format?: "text"}` adds `sections` (`[{id, title, rows: [[label, value]]}]`:
document, objects, graphicStyles, spotColors, patterns, gradients, symbols, fonts, fontDetails, linkedImages,
embeddedImages); `format: "text"` returns the plain-text report Document Info › Save… (`docInfo.save {path?}`) and
the package report write.

```json
{"name":"run_command","arguments":{"command":"file.package","params":{"folder":"/tmp/handoff","copyFonts":false}}}
{"name":"run_command","arguments":{"command":"document.info","params":{"format":"text","category":"fontDetails"}}}
```

## Slices

Object → Slice cuts the art into the pieces web output saves. User slices are rectangles of their own
(`document.slices`, ids from the object id counter); `object.slice.make` turns the selected objects into object
slices, whose slice follows the object's bounds; auto slices fill what no other slice covers (the artboards with
Clip to Artboard on, the default, else the art and the slices). `slice.list` returns every slice as laid out,
numbered left to right and top to bottom: `{number, id (null for auto slices), source: user|object|auto, name, x, y,
width, height, options, selected}`, plus `clipToArtboard`, `hidden` and `locked`.

The Object → Slice commands act on the selected slices: the ones `select.object.slices` (Select → Object → Slices)
selects, and the selected objects that are object slices. `object.slice.release` (an object slice leaves its object,
a user slice becomes an unpainted rectangle), `object.slice.fromGuides` (the grid the ruler guides cut the artboards
into), `object.slice.fromSelection`, `object.slice.duplicate {dx?, dy?}`, `object.slice.combine`,
`object.slice.divide {rows? | rowHeight?, columns? | columnWidth?}`, `object.slice.deleteAll`,
`object.slice.options {kind?: image|noImage|htmlText, name?, url?, target?, message?, alt?, text?, background?:
""|matte|#rrggbb, hAlign?, vAlign?}` and `object.slice.clipToArtboard {on?}` are each one undo step.
`view.slices.hide {hidden?}` and `view.slices.lock {locked?}` are session view state; the canvas draws the slices in
the `sliceLineColor` preference, numbered while `showSliceNumbers` is on.

```json
{"name":"run_command","arguments":{"command":"object.slice.make","params":{}}}
{"name":"run_command","arguments":{"command":"object.slice.options","params":{"name":"logo","url":"https://example.com","alt":"Logo"}}}
{"name":"run_command","arguments":{"command":"slice.list","params":{}}}
```

The Slice tool (`slice`, Shift+K) and the Slice Selection tool (`sliceSelection`) reduce to commands:
`object.slice.create {x, y, width, height}` (the Slice tool's drag; it snaps to ruler guides), `object.slice.select
{slices?, toggle?}`, `object.slice.move {slices?, dx, dy}` (an object slice moves its object),
`object.slice.setRect {id, x, y, width, height}` (user slices) and `object.slice.delete {slices?}`. Slice ids are user
slice ids and the ids of objects with an object slice.

```json
{"name":"run_command","arguments":{"command":"object.slice.create","params":{"x":0,"y":0,"width":200,"height":80}}}
{"name":"run_command","arguments":{"command":"object.slice.move","params":{"dx":10,"dy":0}}}
```

## Export for Screens

`document.exportForScreens` writes every chosen artboard (`range`, `artboards`; default all) in every format row
into `folder`, or returns the files as `dataBase64` without one; `zip: true` packs them into one store-only `.zip`.
A row is `{format, scale?}` where `scale` is a factor (`2`, `"2x"`), a pixel width (`"100w"` or `width: 100`), a
height (`"100h"`) or a resolution (`"72ppi"`); raster rows name their files `@2x`, `@100w`… unless `suffix` says
otherwise. `fullDocument` writes one file per row instead (a PDF of every artboard, other formats the bounds of all
art), `includeBleed` grows each artboard by the document's bleed, `subfolders` puts each row's files in a sub-folder
(its size or format, or its own `folder`), `preset: "mobile"` or `"density"` (Android-style ldpi…xxxhdpi
sub-folders) replaces the rows, and `settings: {png: {…}, jpg: {…}, svg: {…}, pdf: {preset}}` gives every row of a
format its options. The document remembers the last params: `document.exportSettings`.

```json
{"name":"run_command","arguments":{"command":"document.exportForScreens","params":{"preset":"density","zip":true}}}
{"name":"run_command","arguments":{"command":"document.exportForScreens","params":{"range":"1-2","formats":[{"format":"png","scale":"512w"},{"format":"jpg","quality":80,"scale":"2x"},{"format":"svg"}],"settings":{"png":{"background":"white"}}}}}
{"name":"run_command","arguments":{"command":"document.exportSettings","params":{}}}
```

## EPS export

`document.exportEps` (also `document.export` / `export` with format `eps`) writes Encapsulated PostScript: a DSC header
(`%!PS-Adobe-3.0 EPSF-3.0`, `%%BoundingBox`, `%%HiResBoundingBox`, `%%LanguageLevel`, spot colours as
`%%DocumentCustomColors`), then the page. Without `useArtboards` the file covers the visible art, its bounding box y up
from the first artboard's bottom-left corner; `useArtboards: true` (or `range`, `artboards`, `artboard`) writes one file
per artboard, `{stem}_{artboard}.eps`, each bounded by its artboard. `level: 3` (default) writes gradients as smooth
shadings and masks transparent image pixels out; `level: 2` writes gradients as bands of colour (as does
`compatibleGradients`) and transparent pixels white. Type is always glyph outlines, so no fonts are needed (`embedFonts`
is accepted); spot colours are Separation colour spaces, and overprinting fills and strokes overprint unless
`overprints: "discard"`. RGB documents are written in CMYK unless `cmykPostScript: false`.

PostScript has no transparency: it is flattened first with `flattenerPreset` (`high`, `medium` (default), `low` or a
saved preset, see `flattener.presets.list`) and `flattener: {…}` options over it (those of
`object.flattenTransparency`); a warning says so. `previewFormat` adds a TIFF preview behind a binary header (`tiffColor`
(default), transparent unless `transparentPreview: false`, or 1-bit `tiffBw`; `none` writes plain PostScript),
`thumbnails` a PNG thumbnail in comments, and the native document always rides along in the trailer's comments (with
linked images whole when `includeLinkedFiles`), so a later EPS import can reopen the file as it was. Export for
Screens doesn't write EPS.

```json
{"name":"run_command","arguments":{"command":"document.exportEps","params":{"path":"/tmp/logo.eps","level":2,"previewFormat":"tiffBw","flattenerPreset":"high"}}}
{"name":"export","arguments":{"path":"/tmp/art.eps","options":{"useArtboards":true,"cmykPostScript":false}}}
```

## Print

File → Print writes the job as a print-ready PDF (there is no printer driver): each page is a sheet of the chosen
paper. `file.print {settings?, path?}` → `{pages, path | dataBase64, bytes, warnings}`; `print.preview {settings?}`
answers what it would print without writing it: `{pages (copies included), sheets: [{artboard, tile?, ink?, width,
height, orientation, scale}] (one copy, in order), tiles: [{artboard, columns, rows, printed, tiles}] (document
space), inks: [{name, spot, print, frequency, angle}], warnings, settings}`. `print.setup {settings?}` saves the
settings with the document (one undo step, kept in the native file as `print_setup`); both other commands start from
the saved settings, with `settings` over them (`null` keeps a value). `list_commands` with filter `print.setup`
documents every field:

- General: `copies` (1–999), `collate`, `reverse`; `artboards: all|range|ignore` with `range: "1-3, 5"` (ignore: all
  the art as one page), `skipBlank`; `media: letter|legal|tabloid|a3|a4|a5|b4|b5|custom` (`width`, `height` in pt),
  `orientation`, `autoRotate` (on by default: the paper turns to each artboard), `transverse`; `printLayers:
  visiblePrintable|visible|all` (template layers never print); `placement: {origin, x, y}` places the printed area
  (artboard, bleed and marks) on the imageable area (the paper inside `margin`); `scaling: none|fit|custom|tileFull|
  tileImageable` with `scale: {width, height}` (%), `overlap` and `tileRange` (tiles numbered across then down).
- `marks` and `bleed` as `document.exportPdf` takes them (the bleed is the document's unless `useDocument: false`),
  drawn around the artboard at the paper's scale; the page information adds the tile and the ink with its screen.
- `output: {mode: composite|separations, emulsion: up|down, image: positive|negative, spotsToProcess, inks: [{name,
  print, frequency, angle}]}`: separations print one page per ink in the grey of its coverage (images too;
  overprinting fills and strokes don't knock out), Registration on every plate; emulsion down mirrors the page and a
  negative inverts it.
- `graphics: {autoFlatness, flatness, fonts}` and `color: {intent, preserveNumbers}` (the intent colours separate
  with; without preserveNumbers CMYK colours are separated again).

Halftone screens and a fixed flatness are not written to the PDF (the output device's apply): the warnings say so,
as they do for art larger than the imageable area.

```json
{"name":"run_command","arguments":{"command":"print.setup","params":{"settings":{"media":"a4","scaling":"fit","marks":{"trim":true}}}}}
{"name":"run_command","arguments":{"command":"print.preview","params":{"settings":{"output":{"mode":"separations"}}}}}
{"name":"run_command","arguments":{"command":"file.print","params":{"path":"/tmp/job.pdf","settings":{"copies":2}}}}
```

Print to a PostScript file: `file.print {format: "postscript"}` (the default for a `.ps` path) writes the same job as
PostScript, `level: 3` (default) or `2`, answering `format: "postscript"` (`"pdf"` otherwise; in the app the job is
saved, not sent to a printer). The file follows the
DSC: `%%Pages` is the page count (copies included), each `%%Page` sets its paper size, separations name their ink
(`%%PlateColor`) and set its halftone screen (`frequency`, `angle`, a round dot), a fixed flatness is written
(`setflat`), negatives invert the transfer, and the marks print in Registration. PostScript has no transparency: it
is flattened first with `flattenerPreset` (`medium` by default; `high`, `low` or a saved one), with a warning.

```json
{"name":"run_command","arguments":{"command":"file.print","params":{"path":"/tmp/job.ps","level":2,"settings":{"output":{"mode":"separations"}}}}}
```

## Data Recovery

Modified documents get recovery copies (`file.recovery.save`; the app runs it every `autosaveInterval` minutes while
`autosaveRecovery` is on, skipping documents of more than 20,000 objects while `recoveryOffForComplex` is on). A copy
goes when its document is saved, reverted or closed, so copies still there after the app quits were left by a crash.
The copies live in the `recoveryFolder` preference's folder (default: `Data Recovery` beside the app's preferences;
none when the app runs with `VECTORCRAFT_NO_PREFS`) or, on the web, in browser storage.

Each running app (each browser tab) keeps its copies in an area of its own (`<area>/<name>`): a sub-folder whose
`.lock` file it keeps locked while it runs, or on the web an area with a heartbeat it refreshes every minute and a Web
Lock the browser holds until the tab is gone (where the browser has Web Locks: secure pages). Only areas nobody holds
are offered: their lock is free, or no tab holds their Web Lock and their heartbeat is older than three intervals (at
least three minutes). A background tab whose timers are paused therefore keeps its copies. Without Web Locks another
tab can take such a tab for gone; when it resumes, its next heartbeat or recovery save writes its missing copies
again. Several apps running at once (agents' instances included) never see each other's copies as crash leftovers,
and an area being restored or discarded is held, so two apps launched together never both take it.

`file.recovery.list` → `{copies: [{file, title, path, format, saved, open, running}], location}` (`open`: the copy of a
document open here; `running`: kept by another VectorCraft that is running). `file.recovery.restore {file?}` opens
copies left behind (default: all) as `"<name> [Recovered]"`: modified, and Save asks where to save them (suggesting
their original file); the copy moves into this app's area. `file.recovery.discard {file?}` deletes them. Without a
folder or browser storage the commands are disabled and say how to set one.

```json
{"name":"run_command","arguments":{"command":"prefs.set","params":{"key":"recoveryFolder","value":"/tmp/vc-recovery"}}}
{"name":"run_command","arguments":{"command":"file.recovery.save","params":{}}}
{"name":"run_command","arguments":{"command":"file.recovery.restore","params":{"file":"1759650000-1/Poster-1"}}}
```

## Native save options

`document.save` / `file.saveAs` / `file.saveCopy` to a native (`.vectorcraft`, `.vctemplate`) or `.ai` file take, flat
or in `options` (`file.formatOptions {format}` lists them with their values):

- `separateArtboards: true` also writes each artboard of `range` (`"1-3, 5"`, default `"all"`) to
  `<name>-<artboard>.<ext>` beside the file: that artboard and the art touching it. The result's `files` lists every
  file written, the master file first (without a path: `files: [{name, dataBase64}]`). 3 artboards give 4 files.
- `includeLinked: true` keeps linked images' own pixels in the file (they stay linked), not just the previews.
- `embedProfiles` (default true) carries the ICC profiles the document is tagged with that were loaded from files;
  opening the file installs them where they are missing (a warning names one that can't be used).
- `pdfCompatible`: native files (default false) also carry a PDF of every artboard (`pdf` in the file); `.ai` files
  (default true) with false write blank pages around the native document (smaller; other apps show empty pages).
- `.ai` only: `compress` (default true) is the PDF option `compression.compressText`.

File → Save As asks for these in the save options dialog (VectorCraft Options) after the save panel; Save, Save a Copy
and Save as Template reuse the options as last saved (or Use Compression).

```json
{"name":"run_command","arguments":{"command":"file.saveAs","params":{"path":"/tmp/set.vectorcraft","separateArtboards":true,"range":"2-3","includeLinked":true}}}
{"name":"run_command","arguments":{"command":"file.saveAs","params":{"path":"/tmp/set.ai","pdfCompatible":false}}}
```

## EMF and WMF

`document.export` (and `export`) with format `emf` or `wmf` writes a Windows metafile of the first artboard (or
`artboard`); `useArtboards: true` writes one file per chosen artboard (`{stem}-{artboard}.emf`), `false` the bounds of
the visible art. EMF keeps Bézier curves, solid fills (brushes), strokes as geometric pens with their width, caps,
joins, miter limit and dashes, clipping masks (path clips), images with their transparency (AlphaBlend) and type as
outlines; gradients become images clipped to their shape, pattern fills their tiles clipped to the shape, and raster
effects images rendered at the document's raster effects resolution. The header's frame is the artboard in 0.01 mm.
WMF flattens curves into polygons in 16-bit units behind a placeable header (1440 units an inch, fewer for pictures
over about 22 inches); it has no clipping, transparency or gradients, so clipped art is written whole, images over
white and gradients and patterns as one colour. What a format leaves out comes back in `warnings`.

`document.open` and `file.place` read `.emf` and `.wmf` (placeable or not) into one artboard, the picture's frame, and
one layer: paths with their fills and strokes (pens become strokes with caps, joins and dashes), clipping groups,
images and point type in the font the file names. Records VectorCraft doesn't read (EMF+ drawing among them) are
skipped with one warning. `clipboard.importEmf {dataBase64, center?}` loads an EMF or WMF picture into the clipboard
for `edit.paste`; the app pastes `image/emf` from the system clipboard ahead of text and bitmaps when the platform's
clipboard service offers it (the Windows desktop service doesn't read metafiles yet).

```json
{"name":"export","arguments":{"path":"/tmp/logo.emf"}}
{"name":"run_command","arguments":{"command":"document.export","params":{"format":"wmf","useArtboards":true,"path":"/tmp/icons.wmf"}}}
{"name":"run_command","arguments":{"command":"file.place","params":{"path":"/tmp/chart.emf","at":[300,200]}}}
```

## TIFF, BMP and Targa export

`document.export` (and `export`) writes them like the other raster formats: one artboard (or `useArtboards`, one
file each), at `ppi`, over `background`, with `antiAlias`. TIFF takes `colorModel` (`rgb` keeps transparency as an
unassociated alpha channel, `cmyk` writes ink amounts in the working CMYK space with CMYK colours keeping their inks,
`gray` is flattened on white), `lzw` (default true), `byteOrder` (`little`, the `II` files PCs write, or `big`, `MM`)
and `embedIcc` (sRGB, the working CMYK profile or grey). BMP takes `depth` (1 is black and white; 4 and 8 use a
palette chosen by `reduction` and `dither`; 16, 24, or 32, which keeps transparency), `colorModel` (`rgb` or `gray`),
`fileFormat` (`windows` or `os2`: 1, 4, 8 or 24 bits only), `rle` (RLE4/RLE8 for 4- and 8-bit Windows bitmaps) and
`flipRows` (rows top-down, a negative height; not with `rle` or `os2`); impossible combinations are refused. Targa
takes `depth`: 16 (a one-bit alpha), 24 (default) or 32 (with alpha). Formats and depths without alpha are flattened
on white. TIFF and BMP files open again; `document.formats` lists every option.

```json
{"name":"export","arguments":{"path":"/tmp/print.tif","options":{"ppi":300,"colorModel":"cmyk","byteOrder":"big"}}}
{"name":"run_command","arguments":{"command":"document.export","params":{"format":"bmp","depth":8,"rle":true}}}
{"name":"export","arguments":{"path":"/tmp/sprite.tga","options":{"depth":32}}}
```

## Save for Web

File → Export → Save for Web (Legacy) writes optimised web images: `document.exportForWeb {format: gif|jpg|png8|png24,
…}` renders the artboard (`clipToArtboard`, the default; `artboard` picks one) or the visible art and the slices, at
`width`, `height` or `percent`, and encodes it with the format's settings: for GIF and PNG-8 `reduction`, `colors`,
`dither`, `ditherAmount`, `transparency`, `matte`, `interlaced`, `webSnap`, `lossy` (GIF) and `colorTable {locked,
transparent, webShift, sort}`; for JPEG `quality`, `progressive`, `optimized`, `embedProfile` and `matte`; for PNG-24
`transparency`, `matte` and `interlaced`. `metadata` (none, copyright, contact, all) writes File Info as PNG text or a
GIF/JPEG comment; `convertToSrgb` marks PNGs as sRGB. With slices (`slice.list`) each image slice becomes
`images/<slice name>.<ext>` (`slices: selected` writes the selected ones, `none` one image); `output: html` adds
`<stem>.html` placing them, with each slice's URL, target, alt text and status message, and No Image / HTML Text cells.
Without `path` the files come back as `{name, dataBase64}`.

`document.exportForWeb.preview` returns what Save writes for the same settings without writing it: `bytes` (the exact
file size), `seconds` to download at `kbps` (default 56.6), the size, and for palette formats the colour table
(`colors: [{color, source, transparent, locked, webShifted, webSafe}]`). The colour table's lists name colours by
`source` (the colour the reduction made), so a colour keeps its edits when it is web-shifted or snapped. `slice: n`
previews one slice; `image: true` adds the file.

Presets: `webExport.presets.list` (built-in ones, then saved ones), `webExport.presets.save {name, newName?, …settings}`,
`webExport.presets.delete {name}`; any name works as `preset` in the other commands (its settings, then the keys
given). `webExport.settings {…}` remembers the settings the dialog opens on (`{}` reads them, `reset: true` restores the
defaults); presets and remembered settings are preferences.

```json
{"name":"run_command","arguments":{"command":"document.exportForWeb.preview","params":{"format":"gif","colors":32,"dither":"none"}}}
{"name":"run_command","arguments":{"command":"document.exportForWeb","params":{"path":"/tmp/web/page.html","format":"png8","output":"html"}}}
{"name":"run_command","arguments":{"command":"webExport.presets.save","params":{"name":"Banner","format":"jpg","quality":70}}}
```

## Asset Export

Assets are pieces of art collected for export on their own (Window › Asset Export, Object › Collect for Export, File ›
Export Selection…).
`assets.add {ids?, multiple?}` collects the selected objects (or `ids`): one asset per object, or with
`multiple: false` one asset of them all, named after the object or `Asset 1`, `Asset 2`…; art already collected keeps
its asset. Assets name their objects, so they follow edits: moving the art moves the crop, and deleting it
removes it from its asset (an asset left with nothing goes). `assets.list` gives each asset's id, name, objects and
crop (`bounds`); `assets.rename {asset, name}` and `assets.remove {assets}` are undo steps.

`assets.export {assets?, folder?, zip?}` writes each asset's art alone, cropped to it and named after it, in every
format row of the document's export settings, the ones Export for Screens remembers (`document.exportSettings`); any
`formats`, `preset`, `settings`, `prefix` or `subfolders` given win for that export. `assets.settings.set` changes
those shared settings (not an undo step). `document.exportForScreens {assets: [id…]}` does the same with its own params
(the dialog's Assets tab).

```json
{"name":"run_command","arguments":{"command":"assets.add","params":{"multiple":true}}}
{"name":"run_command","arguments":{"command":"assets.settings.set","params":{"formats":[{"format":"png","scale":"1x"},{"format":"png","scale":"2x"},{"format":"svg"}]}}}
{"name":"run_command","arguments":{"command":"assets.export","params":{"zip":true}}}
```

## CSS Properties

`css.selection {ids?, units?, position?, dimensions?, unnamed?, rasterize?}` returns the CSS web pages style the
selected objects (or `ids`) with, one rule per object, as Window › CSS Properties shows it: a rectangle's or ellipse's
fill as `background-color` or a `linear-gradient()`/`radial-gradient()`, its stroke as `border`, its corners as
`border-radius`; type's font, `color`, spacing and alignment; opacity, blend mode, shadows and glows (`box-shadow`,
`text-shadow`) and blur (`filter`). Layers and plain groups stand for what they hold. Properties SVG export writes too
(font, spacing, opacity, blend mode, colours, gradient stops) read exactly as in its style sheets. Each rule in `rules`
says when CSS can't describe the art exactly (`unsupported`: other shapes, images, patterns, live effects…); it is then
written as its box, or with `rasterize: true` as `background-image: url(<class>.png)`. `units` is `px` (1 px per
point, as in SVG export), `pt`, `mm`, `cm` or `in`; `unnamed: false` leaves out unnamed objects (counted in
`skipped`). `css.generate` does the same for the whole document, and `css.export {path?, scope?: selection|all}`
writes the `.css` file with the PNGs of rasterized art next to it (no path: `data` and `images` as base64). In the app,
`css.copy` copies the CSS and `css.exportFile` writes it through a save panel (the web downloads it).

```json
{"name":"run_command","arguments":{"command":"css.selection","params":{"units":"px","position":true}}}
{"name":"run_command","arguments":{"command":"css.export","params":{"path":"/tmp/site/styles.css","scope":"all","rasterize":true}}}
```

## PSD export

`document.export` (and `export`) writes layered bitmaps (`.psd`, 8 bits per channel) like the other raster formats:
one artboard (or `useArtboards`, one file each), at `ppi`, with `antiAlias`, in `colorModel` `rgb`, `cmyk` (ink
amounts in the working CMYK space) or `gray`, with `embedIcc`. With `layers` (default true) each top-level layer is a
pixel layer of its own, drawn alone and cropped to what it paints, with its name, opacity and blend mode; a
`background` colour becomes a `Background` layer under them, and the merged image (what readers that skip layers
show) is the flat render. `maxEditability: true` turns layers and sublayers into groups and every object into a layer
named as the Layers panel names it (text objects by their text); layers with a clipping mask, an opacity mask, an
appearance of their own or knockout stay one pixel layer. Hidden layers and objects are left out unless
`hiddenLayers: true` writes them as hidden layers. `layers: false` writes one flat image, on white where nothing is
drawn. Files are at most 30000 pixels a side.

Photoshop documents (`.psd`, and `.psb`, the large document format) open and place as their merged image, the
flattened picture every Photoshop file carries (layers aren't read): Bitmap, Grayscale, Duotone, Indexed, RGB, CMYK
(converted with the colour settings' CMYK) and Lab, at 1, 8, 16 and 32 bits per channel, sized by their resolution.
A document with transparency keeps it. `file.place {path}` links the file like any image (`links.update` reads it
again when it changes); Multichannel files are refused.

```json
{"name":"export","arguments":{"path":"/tmp/poster.psd","options":{"ppi":300,"maxEditability":true}}}
{"name":"run_command","arguments":{"command":"document.export","params":{"format":"psd","layers":false,"colorModel":"cmyk"}}}
```

## Printing

`print.setup {settings?}` keeps print settings with the document (one undo step; without `settings` it answers the
current ones): the copies, artboards, paper, orientation, placement, scaling and tiling, marks and bleed, composite or
separations output, graphics and colour management options of File › Print. `print.preview {settings?}` lays the job
out without printing: `pages`, then one copy's `sheets`, each with its page size, scale, the `transform` from the
document onto the page (pt, y down), the document `area` it prints and its `trim` box on the page, plus `tiles`, the
`inks` of a separation and `warnings`. `file.print {settings?, path?}` writes the job as a print-ready PDF (one page per
sheet, tile, ink and copy); headless it answers `dataBase64`. In the app, `file.print` with no params opens the Print
dialog, and with params it prints through the system (`printer`: a name from `print.printers`, default the system's
default printer) or, with `toFile`, a `path` or no printing available, saves the PDF.

```json
{"name":"run_command","arguments":{"command":"print.setup","params":{"settings":{"media":"a4","scaling":"fit","marks":{"trim":true}}}}}
{"name":"run_command","arguments":{"command":"print.preview","params":{"settings":{"copies":2,"output":{"mode":"separations"}}}}}
{"name":"run_command","arguments":{"command":"file.print","params":{"path":"/tmp/job.pdf"}}}
```

## Print presets

`print.presets.list` lists `[Default]` (the default print settings, `builtIn: true`, protected) and the saved presets,
each with its `settings` and how they differ from `[Default]` (`changed`). `print.presets.save {name?, newName?,
preset?, settings?}` creates or changes a saved preset (starting from its own settings, or from `preset`, with
`settings` over them) and `print.presets.delete {name}` deletes one; `[Default]` can be neither changed nor deleted.
`print.presets.export {names?, path?}` writes them as a `.vcprintpresets` JSON file (without `path` it returns `data`),
and `print.presets.import {path? | data? | dataBase64?, replace?}` adds a file's presets (a name in use gets a number
unless `replace`). Saved presets live with the preferences. To print with a preset, pass its `settings` to
`print.setup`, `print.preview` or `file.print`.

```json
{"name":"run_command","arguments":{"command":"print.presets.save","params":{"name":"Posters","settings":{"scaling":"tileImageable","overlap":18,"marks":{"trim":true}}}}}
```

## EPS and PostScript import

`document.open` and `file.place` read `.eps` files, and PostScript by any name (an `.ai` saved without PDF
compatibility opens as artwork; an `.ait` one as a new untitled document). An EPS file VectorCraft wrote restores the
document it carries (`restored: true`, as Preserve Editing does for PDF and SVG); when that document can't be read the
PostScript is read instead and the first warning says why.

Other files are run through a small PostScript interpreter (Level 3, first page only): paths, fills and strokes with
their width, caps, joins, miter limit and dashes (a fill and a stroke of the same path become one object), grey, RGB,
CMYK, indexed and spot colours (a Separation ink becomes a spot swatch, painted at its tint), clips (clipping groups;
`clipsave`/`cliprestore`), `gsave`/`grestore`, `save`/`restore`, transforms, procedures with `bind def`, loops,
dictionaries (the standard ones are values in `systemdict`; `dictstack`, `internaldict`), arrays and strings whose
intervals share storage, resources (categories made from `Generic`, `resourceforall`), executable filters (`cvx exec`
runs their data; `flushfile` skips it), axial and radial shadings and shading patterns (gradients), images and image
masks (data in the file through ASCII85, hex, run-length, Flate, LZW or DCT filters, or from procedures), and type as
point type in the font the file names (embedded font programs are skipped). In a file in Illustrator's own format
(Illustrator 3–8 `.ai` and their EPS, written with the prolog that defines its operators) the groups it writes (`u` …
`U`) come in as groups, nested as they were; clips and groups nest at most 128 deep. The artboard is the `%%HiResBoundingBox`
(else `%%BoundingBox`; a letter page without one). A program the interpreter can't run (an operator it doesn't know,
an error, a runaway loop) or that draws nothing comes in as its TIFF preview (palette previews with an alpha channel
too) with a warning; without a preview, the art drawn up to the error is kept with a warning, and a file with none is
refused with a message saying why.

```json
{"name":"run_command","arguments":{"command":"document.open","params":{"path":"/tmp/logo.eps"}}}
{"name":"run_command","arguments":{"command":"file.place","params":{"path":"/tmp/logo.eps","at":[300,300]}}}
```

## PDF/X

`document.exportPdf {standard: "pdfX1a" | "pdfX3" | "pdfX4"}` (or the built-in PDF/X presets) writes print exchange
files as their standards say. Every PDF/X file gets a `/GTS_PDFX` output intent (`output.outputIntent`; blank, the
CMYK profile in effect: the destination when converting to CMYK, else the working one, embedded; in PDF/X-1a and
PDF/X-3 a name that isn't a profile needs `output.registry`, a registered printing condition named without a profile;
PDF/X-4 always embeds one), a TrimBox on every page (the artboard), `/Trapped` True or False, `/GTS_PDFXVersion`, a title (`Untitled` when the
document has none) and dates; type is outlined or embedded; editing data, passwords and, in PDF/X-1a and PDF/X-3, PDF
layers are refused.

- **pdfX1a** (PDF/X-1a:2001, written as PDF 1.3): CMYK, grey and spot colours only. Every colour and image is converted
  to `output.destination` (blank: the output intent's CMYK profile, else the working CMYK profile), untagged; an RGB
  destination or output intent is refused. Transparency is flattened before writing.
- **pdfX3** (PDF/X-3:2002, written as PDF 1.3): colours are tagged with ICC profiles; transparency is flattened.
- **pdfX4** (PDF/X-4:2010, PDF 1.6 at most): transparency and layers are kept, colours are tagged, and the XMP metadata
  names the standard.

Flattening uses `flattenerPreset` (default High Resolution, see PDF 1.3 above) on a copy (the document is untouched; a
warning says so), and images with see-through pixels count as transparency. The written file is checked against
its standard: what it still breaks (transparency, RGB in PDF/X-1a, a font not embedded) fails the export instead of
writing a file that only claims the standard.

```json
{"name":"run_command","arguments":{"command":"document.exportPdf","params":{"path":"/tmp/press.pdf","preset":"PDF/X-1a:2001","output":{"outputIntent":"VectorCraft Generic CMYK (SWOP-like)","outputCondition":"Coated","trapped":false}}}}
```

## Print tiling

View → Show Print Tiling (`view.printTiling {on?}`, per document, not undoable) draws the pages of the document's print
settings on the canvas: each page's paper edge, its imageable area dashed, numbered (tiles across then down; tiles
outside `tileRange` dimmer). The Print Tiling tool (`printTiling`) shows them too. `print.tiling {settings?}` answers
what is drawn, from the layout `print.preview` and both writers (PDF and PostScript) use, so the canvas and the
printed job agree: `{tileOrigin, pages: [{artboard, number, printed, page, imageable}]}` (`[x0, y0, x1, y1]` in document space; every tile when tiling, else
each page once, not once per ink).

Dragging with the tool puts the top-left corner of the first page's imageable area where the pointer is (snapping to
the artboard's edges): `print.tiling.set {origin: [x, y], artboard?}` saves it with the print settings as
`tileOrigin: {placed: true, x, y}`, measured from the artboard's top-left corner (one undo step). It replaces the
placement, and tiles start there (going on left and up as far as the art does). A double click on the canvas, or on
the tool's button, runs `print.tiling.set {reset: true}`: the placement places the pages again. In the Print dialog
the placement then shows as placed by the tool, with a Reset button; dragging the preview moves the origin.

```json
{"name":"run_command","arguments":{"command":"view.printTiling","params":{"on":true}}}
{"name":"run_command","arguments":{"command":"print.tiling.set","params":{"origin":[-36,-36]}}}
{"name":"run_command","arguments":{"command":"print.tiling","params":{"settings":{"scaling":"tileImageable","scale":{"width":300,"height":300}}}}}
```

## Print advanced options

The Print dialog's Advanced section and the printer profile are print settings too (`print.setup`, `print.preview`,
`file.print`): `advanced: {printAsBitmap, overprints: preserve|discard|simulate, flattenerPreset}` and `color:
{profile}`. In composite output, `preserve` writes overprinting fills and strokes with an overprinting graphics state
(`/OP`, as PDF export does), `discard` makes overprinting fills and strokes knock out and `simulate` prints them
as Overprint Preview shows them (multiplied); either way the file has no overprint left. Separations always keep
overprints. `flattenerPreset` (High Resolution, Medium Resolution, Low Resolution or a saved preset,
`flattener.presets.list`) flattens transparency before printing, as Object → Flatten Transparency does; empty, a
PDF keeps it live (PostScript, which has none, uses `file.print`'s `flattenerPreset`, else this one, else medium).
`printAsBitmap` prints each composite page as one image of the art at the document's raster effects resolution.
`color.profile` names an RGB or CMYK profile: composite PDF colours are converted to it with `color.intent` (CMYK
colours keep their numbers with `preserveNumbers`), so a CMYK profile writes device CMYK; separations separate with a
CMYK one. Settings saved before these options load with their defaults.

```json
{"name":"run_command","arguments":{"command":"file.print","params":{"path":"/tmp/proof.pdf","settings":{"advanced":{"overprints":"simulate","flattenerPreset":"High Resolution"},"color":{"profile":"VectorCraft Generic CMYK (SWOP-like)","intent":"perceptual"}}}}}
```

### Rotated bounding boxes

Every object keeps the angle of its own axes (counter-clockwise degrees; `document.inspect` reports it as a node's
`rotation`, and the selection's shared angle as `selectionRotation`, 0 when the selected objects differ). Transforms
turn it with the object while its path geometry is still baked, so after a rotation the bounding box and its handles
stay square to it, as in the reference app. `object.rotate {angle, absolute: true}` turns the selection to an angle
(the Transform and Properties panels' Rotate field), and `object.setBounds` measures width, height and the reference
point along the turned box (x, y stay page coordinates). Objects turned together share the box. A new group starts
square to the page. `object.resetBoundingBox` squares the box again without moving the art.

```json
{"name":"run_command","arguments":{"command":"object.rotate","params":{"angle":45,"absolute":true}}}
```

## Select All while editing type

While the Type tool edits text (a click into type, `pointer_gesture` with `"tool":"type"`), `select.all` (Cmd+A,
`press_key {key: "A", mods: {cmd: true}}`) selects all of that text instead of the art, as in the reference app, and
returns `{editing, start, end}` (the text's id and the selected byte range, which `text.getRange` and
`text.setRangeStyle` take); the art selection stays as it is. Without text being edited it selects every object and
returns `{count}`. Text in threaded frames is selected one frame at a time.

```json
{"name":"press_key","arguments":{"key":"A","mods":{"cmd":true}}}
```

## Empty point type

Point type the Type tool places with a click and leaves empty is discarded when editing ends: Escape, another tool,
a click elsewhere, switching documents, or a command that takes the text out of the selection (`select.none`, a
`select.set` of other objects). The steps since the click go with it (`text.discardEmpty {id}`), so the document and
its undo history are as if the click never happened. Commands on the edited text (the Character panel's
`text.setRangeStyle {id}`) keep it in editing. Area type dragged as a frame and type in or on a path keep their frame
(Object → Path → Clean Up removes empty text).

```json
{"name":"run_command","arguments":{"command":"text.discardEmpty","params":{"id":42}}}
```

## Resizing area type

As in the reference app, dragging a bounding-box handle of area type resizes its type area instead of scaling the
type: the Selection tool sends `object.transform {matrix, typeAreas: true}`, which reshapes the frame of each area
type object it transforms (other objects, type inside groups and point type transform as usual) and reflows the
text at its size, through its thread too. `text.reshapeArea {id, anchors: [[subpath, anchor]…], dx, dy}` moves
frame anchors with their handles (Direct Selection dragging a corner, or an edge's two ends), and
`text.areaOptions {width?, height?}` sizes the area from its top-left corner (the query returns both). Each is one
undo step. Object › Transform › Scale, the Scale tool and the Transform panel still scale the type.

```json
{"name":"run_command","arguments":{"command":"text.reshapeArea","params":{"id":42,"anchors":[[0,2]],"dx":40,"dy":60}}}
```

### Vertical alignment

`text.areaOptions {verticalAlign: top|center|bottom|justify}` (Area Type Options › Align) places the lines of each
row/column cell on its own: `center` centres the block of lines in the cell, `bottom` puts the last line's descent on
the cell's bottom (inside the inset), `justify` keeps the first line at the top, moves the last one to the bottom and
shares the space left over equally between the lines (there is no paragraph spacing limit: every line gap grows by
the same amount). A cell with a single line justifies to the top, and a full or overflowing cell only moves by the
less-than-a-line of space it has left. In rectangular frames without text wrap the lines just move; in other
frames (and around wrap objects) the text flows again starting lower until it settles, never losing text that fit
top-aligned. Vertical type aligns along its block axis (`bottom` is the frame's left edge). Exports lay type out
again, so SVG, PDF and EPS show the aligned text.

```json
{"name":"run_command","arguments":{"command":"text.areaOptions","params":{"verticalAlign":"center"}}}
```

## Fitting area type: Auto Size, Shrink Text to Fit, overflow

`text.areaOptions` (and `text.create`'s `area`, `text.createInPath` in area mode) take a `fit`:

- `none` (default): the frame keeps its size; text that doesn't fit overflows (the red "+").
- `autoHeight`: Auto Size. After every edit (typing, `text.editRange`, character and paragraph changes, a new
  width) the frame's bottom moves to just below the last line plus the inset, in the same undo step as the edit.
  In several columns the frame gets the least height (to 0.01 pt) at which the columns hold the text. It applies
  to rectangular frames of horizontal type in one row; `height` is ignored while it is on, and setting the height
  by hand (a handle drag, `text.reshapeArea` moving the bottom) turns it off, as does threading the frame. The
  `autoSizeAreaType` preference ("Auto Size New Area Type") gives new area type `autoHeight`.
- `shrinkText` (beyond the reference app), with `fitMinPercent` (10–100, default 50): when the text overflows,
  every run's size, leading (explicit leading; auto leading follows the size) and baseline shift are scaled by the
  largest factor down to `fitMinPercent` % that makes it fit. Paragraph spacing and indents stay. The stored
  sizes don't change: the scaling happens at layout time, so rendering and every export see it. The factor is
  found by bisection over the first run's size in steps of 0.1 pt (its scaled size is a whole number of tenths of
  a point, so the result is deterministic); at `fitMinPercent` the text may still overflow.

The file stores `"fit": "autoHeight"` or `"fit": {"shrinkText": {"minPercent": 40}}` in the object's `area`; the
command also accepts that object form. The query (and every reply) of `text.areaOptions` reports `fit` as its id,
`fitMinPercent`, `overflow` (the text doesn't fit its frame) and `fitScale` (Shrink Text's factor, 1 unshrunk).
`document.inspect` (and `document.node {summary: true}`) reports `overflow` for area type and type on a path (text past
the end of the path), and `fit` and `fitScale` for area type.

```json
{"name":"run_command","arguments":{"command":"text.areaOptions","params":{"fit":"shrinkText","fitMinPercent":60}}}
```

## Converting between point type and area type

With the Selection tool, a single selected point or area type object shows the type widget: a small circle beside
the middle of its bounding box's right side, hollow on point type and filled on area type. Double-clicking it runs
`type.convertToAreaType` (point type gets a frame around its text, which keeps its place and doesn't rewrap) or
`type.convertToPointType` (soft line wraps become line breaks), the same commands as the Type menu's Convert To Area
Type and Convert To Point Type. Either keeps the text and its styles and is one undo step; `ids` picks the objects
(default: the selection).

```json
{"name":"run_command","arguments":{"command":"type.convertToAreaType","params":{"ids":[42]}}}
```

## Kinsoku Set

`text.setFormat {kinsoku: "hard"|"soft"|"none"}` sets the Paragraph panel's Kinsoku Set of the selected type (or `ids`):
which Japanese characters may not start or end a line. **Hard** (the default; new type and documents from before the
setting) keeps closing brackets, commas, full stops, middle dots, iteration marks, the prolonged sound mark ー and small
kana off the line start, and opening brackets off the line end. **Soft** lets 々, ー and small kana start a line, as
JLREQ's level 3 line-breaking rules do (https://www.w3.org/TR/jlreq/#addendum_a3); the rest of the set stays.
**None** applies no kinsoku. `hard` isn't saved. One undo step.

```json
{"name":"run_command","arguments":{"command":"text.setFormat","params":{"kinsoku":"soft"}}}
```

## Hanging punctuation (burasagari)

`text.setFormat {burasagari: "none"|"standard"|"forced"}` sets the Paragraph panel menu's Burasagari (None / Regular /
Force) of the selected type (or `ids`): an East Asian comma or full stop ending an area type line (、。，．, half-width
､｡) hangs outside the frame, Regular only when it doesn't fit, Force always (the rest of the line then fills the
measure). Closing brackets and Latin punctuation don't hang; point type, right-to-left and hyphenated lines are left as
they are. The hanging mark is as wide as its punctuation spacing makes it (`mojikumi`). New type (the Type tools,
`text.create`) takes `standard`; documents from before it and imported text keep `none`, and `none` isn't saved. One
undo step.

```json
{"name":"run_command","arguments":{"command":"text.setFormat","params":{"burasagari":"forced"}}}
```

## Character Alignment

`text.setFormat {charAlign}` (the selected type, or `ids`) and `text.setRangeStyle {id, start, end, charAlign}` (a range)
set where characters smaller than the largest on their line line up with it: `romanBaseline` (the default), the em box's
`emBoxTop`, `emBoxCenter` or `emBoxBottom`, or the ideographic character face's (ICF) `icfTop` or `icfBottom`. In
vertical type top and bottom are the right and left of the column. The ICF comes from the font's BASE table (`icfb`,
`icft`, as the OpenType baseline tags define them), else from the average ink box of some ideographs and kana; a font
without ideographs uses its em box. One undo step.

```json
{"name":"run_command","arguments":{"command":"text.setFormat","params":{"charAlign":"icfTop"}}}
```

## New type in a Japanese interface

While the interface is in Japanese (VectorCraft › Language, or `auto` on a Japanese system), new type starts with em
box top-to-top leading (`leadingModel: "emBoxTop"`) and em box centre character alignment (`charAlign: "emBoxCenter"`);
in other languages it starts on the Roman baseline (#432). This covers the Type tools,
`text.create` and `text.createInPath`, which also take `leadingModel` and `charAlign` to choose, and `paraStyle.new` /
`charStyle.new` made with no `attrs` and no text selected. The values used are kept in the journal, so a replay sets the
same in any language. Saved documents keep what they say, and imported text (PDF, SVG, DXF…) keeps the Roman baseline.

```json
{"name":"run_command","arguments":{"command":"text.create","params":{"x":40,"y":60,"text":"雅楽","leadingModel":"emBoxTop","charAlign":"emBoxCenter"}}}
```

## Moving and flipping type on a path

Type on a path flows between a start and an end bracket, stored as fractions of its path's length.
`type.pathOptions {start?, end?, flip?, effect?, alignToPath?, spacing?, ids?}` sets them and the rest of Type on a
Path Options: `end: null` puts the end bracket back at the end of the path (or once round a closed one); `flip`
then turns the type to the other side of its path, the path running the other way and the brackets swapping ends so
the type keeps its stretch of the path; `alignToPath` runs the `ascender`, `descender`, `center` or `baseline` (the
default) along the path; `spacing` (points) closes glyphs up round the outside of curves and opens them up round the
inside. With none of them it queries the first selected type on a path:
`{start, end, flip: false, effect, alignToPath, spacing}`. The Options dialog shows Effect, Flip, Align to Path and
Spacing and leaves the brackets where they are. As in the reference app, the Selection and Direct Selection tools
show selected type on a path's brackets: dragging the start or end bracket (`pointer_gesture`) sets where the type
begins or ends, dragging the centre bracket slides the type along its path and, dragged across the path, flips it
(with Cmd/Ctrl held it only slides). Each is one undo step.

```json
{"name":"run_command","arguments":{"command":"type.pathOptions","params":{"start":0.25,"end":0.75,"flip":true}}}
```

## Paragraph attributes per paragraph

Each paragraph (text split at `\n`) has its own alignment, indents, space before and after, hyphenation,
punctuation spacing (`mojikumi`), hanging punctuation (`burasagari`), direction, leading model, tab stops and
paragraph style. `text.setStyle {justify}`, `text.setFormat {leftIndent, rightIndent, firstLineIndent,
spaceBefore, spaceAfter, hyphenate, mojikumi, burasagari, direction, leadingModel}`, `text.tabs.set` / `text.tabs.clear` and `paraStyle.apply` take optional
`start` / `end` byte offsets of the plain text (as `text.setRangeStyle` does): they change the paragraphs the range
touches (a caret, `start == end`, touches its paragraph). Without a range they change every paragraph of the
targeted objects, as before. With a range, the character attributes of `text.setStyle` and `text.setFormat` style
that range of characters. `text.tabs.get {start?}` and `paraStyle.new` / `paraStyle.redefine {id, start?}` read the
paragraph at `start`; `paraStyle.list` counts paragraphs as uses. A Return the Type tool types (or `text.editRange`
inserting `\n`) continues the style of the paragraph it splits; deleting a break keeps the first paragraph's style;
`text.setText` gives every paragraph the first one's attributes. Threaded text keeps each paragraph's attributes as
the story re-flows (a paragraph split between frames has them in both).

In the native file a text object's `para` holds the first paragraph's attributes and `paras` (left out when every
paragraph is alike) one entry per paragraph, so older readers keep the first paragraph's. SVG export anchors each
centred or right-aligned line on its own (`<tspan text-anchor>`), and SVG import gives each line's paragraph its
alignment back, with indents that keep it in place.

```json
{"name":"run_command","arguments":{"command":"text.setStyle","params":{"id":42,"justify":"center","start":0,"end":0}}}
{"name":"run_command","arguments":{"command":"text.setFormat","params":{"ids":[42],"spaceBefore":6,"start":12,"end":30}}}
{"name":"run_command","arguments":{"command":"paraStyle.apply","params":{"name":"Heading","id":42,"start":0,"end":0}}}
```

## Constrain proportions

The link between W and H in the Transform panel, the Properties panel and the Control bar is the
`constrainProportions` preference (`prefs.get` / `prefs.set`). With it on, those fields send `proportional: true`
to `object.setBounds`, which scales the other dimension by the same factor; agents pass `proportional` themselves.

```json
{"name":"run_command","arguments":{"command":"prefs.set","params":{"key":"constrainProportions","value":true}}}
{"name":"run_command","arguments":{"command":"object.setBounds","params":{"width":200,"proportional":true}}}
```

## Plug-ins

WebAssembly plug-ins ([plugins.md](plugins.md)) are installed per process and reached through commands:
`plugin.install {path | dataBase64}` installs one, `plugin.list` / `plugin.info {id}` describe them (parameters,
schema, defaults). An object filter runs on the selected paths and compound paths with `plugin.run {id, params}` (one
undo step; it may change, delete or add objects, and the result is selected). A live-effect plug-in is an effect with
id `plugin.<id>`: `effect.apply`, `effect.setParams` and `effect.expandAppearance` work as for built-in effects, and
`effect.list` lists it. A failing plug-in returns an error and leaves the document as it was.

```json
{"name":"run_command","arguments":{"command":"plugin.install","params":{"path":"/plugins/desaturate.wasm"}}}
{"name":"run_command","arguments":{"command":"plugin.run","params":{"id":"org.vectorcraft.example.desaturate","params":{"amount":60}}}}
{"name":"run_command","arguments":{"command":"effect.apply","params":{"effect":"plugin.org.example.wobble","params":{"size":4}}}}
```

## Colour adjustments

Effect → Color Adjustments holds live effects that recolour what an object paints
with (fills and strokes, gradient stops, type, gradient meshes, embedded images; on a group or layer every member),
each colour keeping its colour model: `adjust.brightnessContrast`, `adjust.curves` (`points` as `"x,y x,y …"` or
`[[x, y], …]`, 0–255), `adjust.levels`, `adjust.hueSaturation`, `adjust.shiftToColor` and `adjust.temperatureTint`
(`effect.list` documents their parameters). Apply them with `effect.apply` like any effect, or with `item` to recolour
one fill or stroke; they stay editable (`effect.setParams`), export with the adjusted colours (an adjusted image as a
recoloured copy) and become permanent with `effect.expandAppearance`. Patterns and the shadows and glows of raster
effects keep their colours.

## Vector halftone

`object.vectorHalftone` turns the selection into vector dots, lines or shapes sized by its
tone (mono, or CMYK screens multiplied over each other), clipped to the art's outline by default:

```json
{"name":"run_command","arguments":{"command":"object.vectorHalftone","params":{"shape":"circle","frequency":25,"angle":45,"mode":"cmyk"}}}
```

## Fonts

Type can use the bundled fonts, fonts added to the session and the fonts installed on the system (none on the web):
the system's and the user's font folders (Windows: `Fonts` and `%LOCALAPPDATA%\Microsoft\Windows\Fonts`, plus fonts
registered outside them, such as fonts installed as shortcuts, and in the desktop app and `vectorcraft-cli` (MCP
included) the fonts in DirectWrite's system font collection, such as those Adobe Fonts activates while Creative Cloud runs; macOS: `/System/Library/Fonts`, `/Library/Fonts`,
`/Network/Library/Fonts`, `~/Library/Fonts` and downloaded system fonts, plus in the desktop app and `vectorcraft-cli`
the fonts CoreText's font manager lists outside them, such as those apps and font managers register from their own
folders; Linux and BSD: `/usr/share/fonts`,
`/usr/local/share/fonts`, `~/.fonts` and the XDG data folders' `fonts`, `~/.local/share/fonts` among them, and in a
Flatpak sandbox the host's fonts). Faces without outlines VectorCraft draws (no `glyf`, `CFF`, `CFF2` or `VARC` table, such
as bitmap-only fonts) are left out. The scan also reads the folder the `fontsFolder` preference names (Preferences › Type ›
Additional Fonts Folder) with its subfolders and, in the desktop app and `vectorcraft-cli`, VectorCraft's own `Fonts` folder
next to the preferences: `~/Library/Application Support/VectorCraft/Fonts` on macOS, `%APPDATA%\VectorCraft\Fonts` on
Windows, and `$XDG_CONFIG_HOME/vectorcraft/Fonts` or `~/.config/vectorcraft/Fonts` on Linux and BSD. `text.addFontFiles`
copies fonts into that folder. The installed fonts are cataloged once per session (in the background when the app starts, else on the first lookup
by family name), so opening, placing, pasting and importing files find them whatever ran before. `text.fontList`
lists every family available, the installed ones included, each by its English (canonical) name — the same string
`text.setStyle {font}` and documents use — without the system's hidden families, whose names start with "."
(macOS's ".SF NS", ".LastResort"), which still resolve when a document names them. Preferences › Type › Show Font
Names in English (`fontNamesInEnglish`) only changes the UI menu labels, not this list. With `family` it gives that
family's styles (upright by weight, then italics) and fails when the family isn't available. `text.rescanFonts`
(Character panel menu › Refresh Font List) scans the font folders again, for fonts installed or removed since the app
started: type set in a font that became available redraws in it, without editing the document. The app does so by
itself when its window comes to the front and a font folder changed meanwhile.

```json
{"name":"run_command","arguments":{"command":"text.fontList","params":{}}}
{"name":"run_command","arguments":{"command":"text.fontList","params":{"family":"Source Serif 4"}}}
{"name":"run_command","arguments":{"command":"text.rescanFonts","params":{}}}
```

`text.missingFonts` lists the fonts that the active document's type uses, in its layers and its symbols, and that aren't
available as named: `{fonts: [{family, style, status, resolved}], count, fontsNextToDocument?}`. `status` is `missing` or
`substitute`, as in `text.fonts`. CSS generic families such as `sans-serif`, which no font file provides, and names
longer than 256 bytes are left out. `fontsNextToDocument` is the `Fonts` folder next to the saved document
(File › Package writes it), when there is one and a search may start there. `text.findFontFiles {folder}` looks for
the fonts' files in an absolute folder and its subfolders on separate threads and returns at once with
`state: "searching"` and an `id`; `fonts: [{family, style?}]` names other fonts to look for. `text.findFontFiles {}`
returns the last search's state with the files found so far for each font: `searching`, `done`, `stopped` (with
`stopped`: `stop`, `time` or `limit`) or `failed` (with `error`); before the first search it returns
`{state: "idle", fonts: []}`, without an `id`. `{stop: true}` stops the search. A search reads the table directories
and the `name`, `fvar` and `OS/2` tables of `.ttf`, `.otf`, `.ttc` and `.otc` files and, on macOS, of suitcase fonts
(extensionless or `.suit` files whose fonts are in their resource fork), and it lists a file for a font only when adding
the file makes the font resolve exactly as `text.fonts` resolves it. It ends after `maxSeconds` (60 by default, 1 to
600), at its limits on the entries listed (10,000,000), the folders waiting to be listed (1,000,000), the font files
read (10,000) and the files kept (1,000), or once every font has a file. Folders more than 64 levels below the picked
one are skipped and counted in `skipped`. A search does not read a font file larger than 256 MB, which
`text.addFontFiles` does not copy, or a suitcase font whose resource fork is larger than 8 MB.

A search does not enter app and media library packages (`.app`, `.bundle`, `.framework`, `.photoslibrary` and the like),
folders named `Program Files` (also `Program Files (x86)` and `Program Files (Arm)`), `ProgramData`, `$Recycle.Bin` or
`System Volume Information`, the system's folders at the root of a volume (a macOS, Linux, BSD or Windows root, told by
the folders it holds), the `Shared` and `Public` folders in `Users`, a home folder's `Library`, `AppData`,
`Applications` and `snap` folders and the folders in it whose names start with a dot, the app data folders in a
`Library` folder (`Application Support`, `Containers`, `Group Containers`, `Caches`, `Preferences`), or the folders the
environment names for apps and the system (such as `APPDATA`, `LOCALAPPDATA`, `ProgramFiles`, `SystemRoot` and
`XDG_CONFIG_HOME`), and a search of a folder inside one of them fails. The walk also skips other hidden folders and
folders whose contents are in the cloud, and it follows no links; a picked folder may be one of those, and a picked path
that goes through a link is searched at its target. Programs installed in other folders, such as a folder on another
drive, are searched as any other folder is.

`text.addFontFiles {files}` copies files that the session's last search found into VectorCraft's `Fonts` folder, then
scans the fonts again, and type set in them redraws. It copies only regular `.ttf`, `.otf`, `.ttc`, `.otc`, `.woff`
and `.woff2` files and suitcase fonts (with their resource fork), at most 256 MB a file and 1 GB in all, and lists any
other file in `skipped`. It never replaces a file: a file with the same contents is kept (`kept`), and when another
file has the name, the copy gets a number (`Name 2.otf`). Copy only fonts you own or are licensed to install. `command.batch` returns before a search ends; later requests over MCP or the control channel poll
`text.findFontFiles {}`. `vectorcraft-cli run` stops the search when its last step ends. The web build has no Fonts
folder and no folder search.

```json
{"name":"run_command","arguments":{"command":"text.missingFonts","params":{}}}
{"name":"run_command","arguments":{"command":"text.findFontFiles","params":{"folder":"/Users/me/Downloads"}}}
{"name":"run_command","arguments":{"command":"text.findFontFiles","params":{}}}
{"name":"run_command","arguments":{"command":"text.addFontFiles","params":{"files":["/Users/me/Downloads/Example/Example-Regular.otf"]}}}
```

## Tool options

`tool.setOption` reads and sets a tool's options, headless too: `{key, value}` or `{values: {…}}` sets them on the
active tool, `tool` names another one, and `{}` just reads them; the result is the tool's options. The options a
tool keeps last across tool switches (and documents) and are saved with the preferences (`toolSettings` in
`prefs.get`'s full object), as the reference app keeps them: the Liquify tools' brush and tool options, Mirror & Cut's
axis and side, Puppet Warp's mesh, the Symbolism brush, the line, grid, pencil, brush and eraser tools' options,
polygon sides and star points, and the Flare Tool Options. The Liquify tools share one set of Global Brush Dimensions (width, height, angle,
intensity), and the Symbolism tools one brush. Interaction state (pins, a reference point) starts afresh.

```json
{"name":"run_command","arguments":{"command":"tool.setOption","params":{"tool":"twirl","values":{"width":60,"rate":90}}}}
```

`tool.options {tool}` opens what double-clicking the tool's button opens (its params doc lists every tool), in the
desktop app. The Flare tool's are its Flare Tool Options (dialog `flareOptions`): `diameter` and `pathLength` (pt),
`opacity`, `brightness`, `growth`, `fuzziness`, `longest`, `rayFuzziness` and `largest` (%), `rays`, `rings`,
`direction` (°), and `raysOn` and `ringsOn` (the Rays and Rings checkboxes; off, the flare has none). OK keeps them
(`tool.setOption {tool: "flare", values}`) and the next flare dragged out uses them; a click with the tool opens the
same dialog, and its OK also draws a flare there. The Artboard tool's are the active artboard's Artboard Options
(dialog `artboardOptions`; the Artboard tool's active artboard while it is in use, else the window's), a graph tool's
the selected graph's Graph Type (an error without one), a Symbolism tool's the Symbolism Tools Options (dialog
`symbolismOptions`: `diameter` in points, `intensity` and `density` 1–10, one brush for the eight tools) and the
Magic Wand tool's its panel.

```json
{"name":"run_command","arguments":{"command":"tool.setOption","params":{"tool":"flare","values":{"rays":24,"ringsOn":false}}}}
```

## Perspective grid

The grid is document data (`perspective.grid.set` edits the model directly). `perspective.grid.get` reads it:
`grid` (the model), `define` (the Define Grid fields), `station` (the viewer: `x` the centre of vision on the
horizon `y`, `distance` from the picture plane, in points) and `defined` (false while the document uses the default
grid). `perspective.grid.define` sets the grid from the Define Grid fields: `kind`, `units`, `scale`
(`[artboard, real world]`), `gridline`, `angle` (the viewing angle, 0–90°), `distance` (the viewing distance),
`horizonHeight`, `thirdVp` (`[x right, y up]` from the centre of vision), `leftColor`, `rightColor`, `groundColor`
(`"#rrggbb"`) and `opacity` (0–100). Lengths are real-world lengths in `units` drawn at `scale`; missing fields keep the
grid's. Only what differs changes (an unchanged OK is no undo step): a new type, angle or distance moves the vanishing
points around the station point, which stays put. In two- and three-point grids the viewer looks at the left plane at
the viewing angle, so the vanishing points sit at `x − distance/tan(angle)` and `x + distance·tan(angle)`.

```json
{"name":"run_command","arguments":{"command":"perspective.grid.get","params":{}}}
{"name":"run_command","arguments":{"command":"perspective.grid.define","params":{"units":"inches","gridline":0.5,"angle":30,"distance":6,"horizonHeight":3}}}
```

Perspective grid presets: `perspective.presets.list` lists the built-in views (`[1P-Normal View]`, `[1P-Low View]`,
`[1P-High View]`, `[2P-Normal View]`, `[2P-Low View]`, `[2P-High View]`, `[3P-Normal View]`, `[3P-Low View]`;
`builtIn: true`, fitted to the first artboard, protected), then the saved ones, each with its Define Grid fields.
`perspective.grid.preset {name}` (or `{kind}`, that type's normal view) resets the grid to one, fitted to the first
artboard. `perspective.presets.save {name?, newName?, preset?, …fields}` saves a preset: just `{name}` saves the
document's grid (View → Perspective Grid → Save Grid as Preset). `perspective.presets.delete {name}`,
`perspective.presets.export {names?, path?}` (a `.vcperspective` JSON file; without `path` it returns `data`) and
`perspective.presets.import {path? | data? | dataBase64?, replace?}` manage them. Saved presets are kept with the
preferences, and a grid whose fields are a preset's is named after it (`name`).

```json
{"name":"run_command","arguments":{"command":"perspective.grid.preset","params":{"name":"[2P-Low View]"}}}
{"name":"run_command","arguments":{"command":"perspective.presets.save","params":{"name":"Street","units":"feet","scale":[1,48],"gridline":1}}}
```

Perspective grid view options (View → Perspective Grid; view state saved with the document, no undo step, each
`{on?: bool}`, no param toggles, → `{on}`): `perspective.grid.lock` (Lock Grid: the grid's widgets can't be dragged),
`perspective.grid.lockStation` (Lock Station Point: dragging one vanishing point turns the view around the station
point, so the other one moves), `perspective.grid.snap` (Snap to Grid, on by default: `perspective.draw` lands the
drawn corners, and `perspective.move` the nearer edge, on gridlines within a quarter cell; both take `snap: false`)
and `perspective.grid.rulers` (Show Rulers: a ruler up the line where the planes meet, in the grid's units at its
scale). Gridlines draw in the Define Grid colours at its opacity.

Perspective grid widgets: with the Perspective Grid tool (`pointer_gesture`), the vanishing points (Shift keeps the
horizon; with Lock Station Point the other one swings), the horizon, the third vanishing point, the origin, the left
and right ground-level points (24 px from the origin along each wall's ground line: they move the whole grid; Shift
keeps one axis), the left and right extents (separate: `extent` and `extentRight`; Alt drags both, Shift goes by
whole cells), the vertical extent and the cell size widget (on the line where the planes meet, a cell up, or more
cells while they're small) reshape the grid; Lock Grid stops them. The Plane Switching Widget stays put in a corner
of the document window (headless: the first artboard's top-left corner); a press on it picks the plane with any tool
while the grid shows (choosing a perspective tool shows it, and `perspective.grid.show` hides it again with the tool
still chosen), and the keys 1–4 (`key` with `"1"`…`"4"`) pick the left, horizontal, right and no plane.
`perspective.widget.options {show?, position?}` (Perspective Grid Options, double-click the tool) hides it or moves
it to `topLeft`, `topRight`, `bottomLeft` or `bottomRight`, kept with the preferences (`prefs.get`/`prefs.set` key
`perspectiveWidget`).

## Envelopes

Object → Envelope Distort wraps the selection in a live envelope: `object.envelope.makeWithWarp` (`style`: arc,
arcLower, arcUpper, arch, bulge, shellLower, shellUpper, flag, wave, fish, rise, fisheye, inflate, squeeze or twist;
`bend`, `h` and `v` in % from -100 to 100; `horizontal` or `orientation`), `object.envelope.makeWithMesh` (`rows`,
`cols`: 1–50) and `object.envelope.makeWithTopObject` (the topmost selected path becomes the envelope).
`object.envelope.info` reads the selected envelope's settings (`type`, the warp's or mesh's values, `fidelity` and
the options, `editing`), or with no envelope selected the options new envelopes get. With an envelope selected,
`object.envelope.resetWithWarp` and `object.envelope.resetWithMesh` (`rows`, `cols`, `maintainShape`: true keeps
the current shape) switch its kind and keep its content. `object.envelope.options` sets `fidelity` and the options
(`antiAlias`, `preserveShape`: clippingMask or transparency, `distortAppearance`, `distortLinearGradients`,
`distortPatternFills`; the last two need `distortAppearance`); with no envelope selected it sets them for new
envelopes. New envelopes start with Distort Appearance on, as in the reference app; envelopes saved before these
options existed keep their old look (appearance applied after the distortion).

`object.envelope.release` gives back the content and the envelope's shape (a grey gradient mesh for warp and mesh
envelopes, the path for a top-object envelope); the content keeps the envelope's opacity, blend mode and opacity mask
(a group around it carries them when there are several objects). `object.envelope.expand`, and `object.expand` with
`object` on, replace envelopes by groups of their distorted content that keep the name, transparency, knockout and
opacity mask.
Type inside an envelope distorts as its glyph outlines, run by run in each run's paint, on the canvas and in every
export (SVG, PDF, EPS, EMF/WMF, DXF).
Everything inside bends with the envelope: images as a raster mesh warp (cut into pieces by Fidelity, each a clipped
image under its own affine map; SVG shares the pixels between the pieces), symbol instances as their symbol's art,
and with Distort Appearance strokes (as their filled outlines) and geometry effects; with it, Distort Linear Gradients
bends linear gradients and Distort Pattern Fills the pattern tiles. A transformed envelope keeps its warp in its own
frame (`kind.frame` in `document.inspect`; rotating a warp envelope turns the warp with it), and move and scale keep
it square to the page.

```json
{"name":"run_command","arguments":{"command":"object.envelope.makeWithWarp","params":{"style":"arch","bend":40}}}
{"name":"run_command","arguments":{"command":"object.envelope.resetWithMesh","params":{"rows":3,"cols":3,"maintainShape":true}}}
{"name":"run_command","arguments":{"command":"object.envelope.options","params":{"fidelity":80,"distortAppearance":true,"distortLinearGradients":true}}}
{"name":"run_command","arguments":{"command":"object.envelope.info","params":{}}}
```

## Liquify tools

The Liquify tools (`warp`, `twirl`, `pucker`, `bloat`, `scallop`, `crystallize`, `wrinkle`) drag a brush over paths:
the selected ones, or with nothing selected every path the brush passes over. A stroke is one undo step and one
`object.liquify` journal entry, which an agent can also run directly. Their options (`tool.setOption`) are the Global
Brush Dimensions shared by the seven tools (`width`, `height` in pt, `angle`, `intensity` 0..1, `usePressure`,
`showBrush`) and the tool's own: `detail` (1..10); `simplify` (0..100) with `simplifyOn` (Warp, Twirl, Pucker,
Bloat); `rate` (Twirl, -180..180°); `complexity` (0..15) and `affectAnchors`, `affectIn`, `affectOut` (Scallop,
Crystallize, Wrinkle: what the brush moves); `horizontal`, `vertical` (Wrinkle, 0..1). Alt-drag sizes the brush from
its current size (with Shift too it keeps its proportions). With Use Pressure Pen on, each `pointer_gesture` event's
`pressure` (0..1) is the intensity there, and `object.liquify` takes points as `[x, y, pressure]`.

```json
{"name":"run_command","arguments":{"command":"tool.setOption","params":{"tool":"bloat","values":{"width":80,"height":80,"usePressure":true}}}}
{"name":"pointer_gesture","arguments":{"tool":"bloat","events":[{"kind":"down","x":280,"y":150,"pressure":0.3},{"kind":"drag","x":280,"y":200,"pressure":0.8},{"kind":"up","x":280,"y":250}]}}
{"name":"run_command","arguments":{"command":"object.liquify","params":{"tool":"scallop","points":[[300,150],[300,250]],"complexity":3,"affectAnchors":false}}}
```

Holding the brush still keeps Twirl, Pucker and Bloat working, scaled by the time held: every tenth of a second the
stroke's last point repeats in `object.liquify`'s `points`, one more dab there (other tools ignore repeated points).
Give a `pointer_gesture` event `holdMs` (0..60000) to hold the pointer that long after it, button down; the same
time gives the same result. Liquify reshapes paths only: type, symbols, images, graphs, meshes, and envelopes,
repeats and blends with their contents stay as they are, and guides are never touched. `object.liquify` returns
those under the brush in `skipped` with a `warning`, which the gesture's `requests` carry as `{"status": …}` (the
status bar in the app). A drag applies only the dabs each new sample adds; the result equals applying the whole
stroke at once, so the journal entry replays it exactly.

```json
{"name":"pointer_gesture","arguments":{"tool":"twirl","events":[{"kind":"down","x":300,"y":200,"holdMs":800},{"kind":"up","x":300,"y":200}]}}
```

## Perspective Selection

Objects in perspective keep their attachment themselves (`perspective` on the object: `plane` and `depth`, the
distance along the plane's normal), so copies, duplicates and pastes stay attached. With the
`perspectiveSelection` tool, a drag moves the selection within its plane (`perspective.move`; Alt copies), the
bounding-box handles scale it in plane space (`perspective.transform`; Shift keeps proportions, Alt scales about the
centre), and `press_key` `5` during a drag switches the move to perpendicular to the plane (press again to switch
back). The arrow keys nudge a selection in perspective by the keyboard increment (`perspective.nudge`; Shift ×10, Alt
copies). Object › Transform › Transform Again repeats the last perspective move or scale in plane space.

```json
{"name":"pointer_gesture","arguments":{"tool":"perspectiveSelection","events":[{"kind":"down","x":480,"y":410},{"kind":"drag","x":470,"y":405}]}}
{"name":"press_key","arguments":{"key":"5"}}
{"name":"pointer_gesture","arguments":{"events":[{"kind":"up","x":470,"y":405}]}}
{"name":"run_command","arguments":{"command":"perspective.transform","params":{"matrix":[1.5,0,0,1.5,0,0],"copy":true}}}
{"name":"run_command","arguments":{"command":"object.transformAgain","params":{}}}
```

Planes move along their normals: `perspective.plane.move {plane, offset | by, objects}` (`leftOffset`,
`rightOffset`, `groundOffset` in the grid). The plane widgets (a diamond on each plane, drawn while the grid shows)
do it with the Perspective Grid and Perspective Selection tools: a plain drag moves the plane alone, Shift-drag moves
the objects on it too, Alt-drag copies them; double-clicking a widget (or `ui.perspectivePlane {plane}`) opens the
`perspectivePlane` dialog (`location` in points, `objects`: none, move or copy). Objects keep their place when the
grid's definition changes, as in the reference app; `perspective.grid.set` with `reproject: true` moves them with it
instead. `perspective.plane.matchObject` (Object › Perspective › Move Plane to Match Object) moves the plane of the
selected object onto it.

```json
{"name":"run_command","arguments":{"command":"perspective.plane.move","params":{"plane":"right","offset":40,"objects":"move"}}}
{"name":"run_command","arguments":{"command":"perspective.plane.matchObject","params":{}}}
{"name":"run_command","arguments":{"command":"ui.perspectivePlane","params":{"plane":"ground"}}}
```

Type and symbol instances attached to a plane stay type and symbol instances: their `perspective` record keeps a
`projection` (a 3 × 3 matrix) the canvas and every export (SVG, PDF, EPS, EMF/WMF, DXF, PNG) draw their outlines
through, so they are really foreshortened and still editable. `perspective.editText` (Object › Perspective › Edit
Text, or double-clicking the type with the Perspective Selection tool) shows the type flat where it is drawn, in
isolation mode, for the Type tool and `text.*` commands; `object.exitIsolation` projects it again. Release with
Perspective keeps their look. Shape, spiral, polar grid and flare drags draw on the active plane while the grid
shows, and so do the click-to-size dialogs (`perspective.draw` with `at`: the click; sizes are plane units).

```json
{"name":"run_command","arguments":{"command":"perspective.editText","params":{}}}
{"name":"run_command","arguments":{"command":"text.setText","params":{"text":"OPEN"}}}
{"name":"run_command","arguments":{"command":"object.exitIsolation","params":{}}}
{"name":"run_command","arguments":{"command":"perspective.draw","params":{"command":"shape.rectangle","params":{"x":470,"y":300,"width":50,"height":30},"at":[470,300]}}}
```

## Blends

`object.blend.make` blends the selected objects (or `ids`) into a live blend. Spacing and orientation default to
what `object.blend.options` set with no blend selected (`object.blend.info` reads them: `target: "defaults"`),
else Smooth Color and Align to Page. `starts` gives, per object, the anchor of its first subpath the blend starts
from (as the Blend tool's clicks on anchor points do; an open path's last anchor runs it the other way). A blend
among the objects takes the others in as more key objects, keeping its options, name and transparency:

```json
{"name":"run_command","arguments":{"command":"object.blend.make","params":{"ids":[12,15],"starts":[0,2],"steps":8}}}
{"name":"run_command","arguments":{"command":"object.blend.make","params":{"ids":[20,31]}}}
{"name":"run_command","arguments":{"command":"object.blend.info","params":{}}}
```

A blend's spine is the straight lines between its key centres until it is edited. `object.blend.info` lists it
(`spine.anchors`, `keyAnchors`: the point each key sits on). `object.blend.spine.addAnchor {x, y}` adds a point where
the spine passes nearest, `object.blend.spine.moveAnchor {anchor, x, y}` moves a point (a key on it moves along; with
`handle: "in" | "out"` it places that handle) and `object.blend.spine.removeAnchor {anchor}` deletes a point no key
sits on; the first edit turns the spine into a path. Moving a key moves its end of the spine. Release
(`object.blend.release`) leaves the spine as a path that paints nothing (`spines` in the result):

```json
{"name":"run_command","arguments":{"command":"object.blend.spine.addAnchor","params":{"id":40,"x":150,"y":100}}}
{"name":"run_command","arguments":{"command":"object.blend.spine.moveAnchor","params":{"id":40,"anchor":1,"x":150,"y":20}}}
```

How blends interpolate: closed shapes start where they twist least unless `starts` picks the anchors; strokes
interpolate weight, dashes (a solid stroke counts as the dashed one with its gaps closed), miter limit, arrowhead scale
and width profiles, while caps, joins, alignment, arrowheads and brushes switch halfway; gradients of one kind with
different stop counts resample to a common set of stops. Groups pair their members in stacking order (members only
one group has grow out of the other's centre), compound paths stay compound paths, and type, symbol instances and
images interpolate their transforms (type also its colours, stroke weight and size run by run; the words and the
symbol switch halfway). New blends are knockout groups (`object.setProps {knockout}` changes it), drawn on the
canvas as they export. `object.blend.expand` and `object.blend.release` keep the blend's name, transparency,
opacity mask and appearance (Release on a group around the keys and spine when the blend has any), and
`object.expand` with `object: true` expands the blends in the selection.

## Paragraph composer

`text.setFormat {ids?, composer: "singleLine"|"everyLine"}` sets how a text object's lines are broken (the Paragraph
panel menu's Single-line and Every-line Composer; stored in the paragraph attributes as `para.composer`, saved only
when it is Single-line). Every-line, the default, picks the breaks of the whole paragraph together in area type:
justified text gets even word spacing, ragged text (left, centre or right aligned) an even rag, so it may move a word
that fits to the next line to avoid a lone short word on the last line or a full line next to a short one.
Single-line fills each line as far as it goes. Point type and type on a path have no line width, so both composers
give the same result there; paragraphs that mix type sizes fall back to Single-line.

```json
{"name":"run_command","arguments":{"command":"text.setFormat","params":{"ids":[7],"composer":"singleLine"}}}
```

## Editing envelopes

`object.envelope.editContents {editing}` switches between editing the envelope and its contents (the menu item reads
Edit Envelope while the contents are edited; it has no default shortcut, since the reference app's is Paste in Place
here). While the contents are edited, clicks hit the content where it sits undistorted and the Selection tool selects
it. A mesh envelope's points have bezier handles (`kind.handles`, offsets
right/left/down/up; none stored means the smooth mesh through the points, as older files have):
`object.envelope.setMeshPoint {id, index, x, y, handle?}` and `object.mesh.movePoint`, `object.mesh.addLine` and
`object.mesh.deletePoint` edit them like a gradient mesh's (one undo step each). On the canvas a selected envelope
shows its mesh; the Mesh tool and Direct Selection drag its points and the clicked point's handles, and the Mesh tool
adds a row and a column where it clicks inside. The Control bar shows Edit Envelope / Edit Contents, the warp's
style, orientation, bend and distortions (or the mesh's rows and columns), Reset (an unbent warp, a flat mesh) and
Envelope Options.

```json
{"name":"run_command","arguments":{"command":"object.envelope.editContents","params":{"editing":true}}}
{"name":"run_command","arguments":{"command":"object.mesh.movePoint","params":{"id":12,"index":4,"x":220,"y":140,"handle":0}}}
```
