# VectorCraft MCP server

`vectorcraft-cli mcp` runs a [Model Context Protocol](https://modelcontextprotocol.io) server on stdio
(newline-delimited JSON-RPC 2.0; see [Protocol](#protocol) for version negotiation). Agents use it to
draw, inspect and look at VectorCraft documents.

It has two backends:

| Mode | How | What works |
|---|---|---|
| **Remote** | `vectorcraft-cli mcp --connect 127.0.0.1:7979` (the app must run with `vectorcraft --control 7979`) | Everything. Tool calls are forwarded over the [control protocol](control-protocol.md), so you watch the app change live. |
| **Headless** | `vectorcraft-cli mcp --headless` | An in-process engine session with a CPU renderer. Everything except the UI-only tools (`inspect_ui` or `ui_inspect`, `type_text`, `answer_dialog`, `open_panel`, `screenshot {window:true}` or `ui_screenshot`). |

With no flag, the server tries `127.0.0.1:7979` and falls back to headless. Logs go to stderr; stdout carries
only protocol messages.

## Install and register

From the repository root, with Rust 1.95+ and the platform's source-build prerequisites installed:

```sh
cargo install --path apps/vectorcraft-cli --locked
# POSIX shells, for Cargo's default install location:
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
vectorcraft-cli --help
claude mcp add vectorcraft -- vectorcraft-cli mcp
# or pin a mode:
claude mcp add vectorcraft-headless -- vectorcraft-cli mcp --headless
claude mcp add vectorcraft-app -- vectorcraft-cli mcp --connect 127.0.0.1:7979
```

Cargo installs a binary, not a symlink into `target/`, so it survives `cargo clean`.
Re-run the install command after updating the checkout.
By default the executable goes in `$CARGO_HOME/bin` (`~/.cargo/bin` when unset), or
`%USERPROFILE%\.cargo\bin` on Windows. If you set `--root`, `CARGO_INSTALL_ROOT`, or
Cargo's `install.root`, use that installation root's `bin` directory instead. Add it
to the `PATH` inherited by the MCP client, not just an interactive shell, and restart
the client after changing its environment. Installing does not start or register a server.

Other clients use the same command in their JSON config:

```json
{"mcpServers": {"vectorcraft": {"command": "vectorcraft-cli", "args": ["mcp"]}}}
```

If a client reports `ENOENT` or "Executable not found in $PATH", verify the install with
`command -v vectorcraft-cli` (PowerShell: `Get-Command vectorcraft-cli`) in the client's
environment. If that environment cannot be changed, set JSON `command` to the absolute
installed binary path, not a literal `~` or `$HOME` expression. Avoid symlinks into
`target/release`: cleaning the build directory breaks them. A source build alone does
not put the CLI on `PATH`.

For a live session, start the app first: `cargo run --release -p vectorcraft -- --control 7979`.

### From an installed release

The release packages ship `vectorcraft-cli` alongside the desktop app, so no build is needed:

| Install | CLI |
|---|---|
| Windows (MSI) | `%LOCALAPPDATA%\Programs\ArtCraft\VectorCraft\vectorcraft-cli.exe` by default (`C:\Program Files\ArtCraft\VectorCraft\vectorcraft-cli.exe` for an all-users install, under `Program Files (x86)` instead for the 32-bit build, or wherever you installed it otherwise), not on `PATH` |
| Linux (deb, rpm) | `/usr/bin/vectorcraft-cli` |
| macOS | the separate `vectorcraft-cli-<version>-macos-<arch>.zip` release asset (the `.app` holds only the desktop app) |

```sh
# Windows (PowerShell), default install folder
claude mcp add vectorcraft -- "$env:LOCALAPPDATA\Programs\ArtCraft\VectorCraft\vectorcraft-cli.exe" mcp
# Linux, or macOS with the CLI unzipped onto PATH
claude mcp add vectorcraft -- vectorcraft-cli mcp
```

For a live session, start the installed app with the control port before the MCP client starts the server:

```sh
# macOS: quit VectorCraft first, since `open -a` passes no arguments to an app that is already running
open -a VectorCraft --args --control 7979
# Windows (PowerShell), default install folder
& "$env:LOCALAPPDATA\Programs\ArtCraft\VectorCraft\vectorcraft.exe" --control 7979
# Linux (deb, rpm)
vectorcraft --control 7979
```

The Dock, Start menu and desktop launchers start the app without `--control`, so the app opens no control
port unless `VECTORCRAFT_CONTROL_PORT` is set. `vectorcraft-cli mcp` with no flag tries `127.0.0.1:7979`
once, when it starts: a server that started before the app stays headless until the MCP client restarts
it. `open -a` and PowerShell's `&` return before the app opens the port, so wait until the VectorCraft
window is open before you start the MCP client. With `--connect`, a server that finds no app stops and
prints `vectorcraft-cli: cannot connect to 127.0.0.1:7979: ` followed by the system's connection error, for
example `connection timed out`.

## Confining file access

```sh
vectorcraft-cli mcp --automation-read-root /work/project --automation-write-root /work/project/out
vectorcraft-cli mcp --automation-read-root /work/project --automation-read-root /work/shared-assets \
  --automation-write-root /work/project/out --automation-write-root /tmp/scratch
```

```json
{"mcpServers": {"vectorcraft": {"command": "vectorcraft-cli",
  "args": ["mcp", "--automation-read-root", "/work/project", "--automation-write-root", "/work/project/out"]}}}
```

The two flags (PhotoCraft and the other Craft apps take the same ones, so one client config works across
the suite) keep the agent to the folders you give it:

- Every file a command reads must lie inside a **read root**: `open_file`, `file.place`, relinking and
  updating links, the images an SVG links to, swatch library, graphic style library, symbol library, brush library and colour-profile loads, plug-ins,
  `file.newFromTemplate`, preset imports, shortcut sets (`shortcutSet.read`), action sets (`actionSet.read`), the folders `text.findFontFiles` and `links.findFiles` search, the
  scripts `script.check` reads and the files they include.
- Every file a command writes must lie inside a **write root**: `save_file`, `export`, the save and
  export commands, Export for Screens, Package, Export for Web, print to file, `screenshot {path}`,
  library and preset saves to a path, the copies `links.relink {copy}` makes.
- `file.list {path}` (through `run_command`) lists a folder's files and folders, held to the read roots like any read, so an agent can see what it was given (#1163).
- Each flag may be given more than once (#3546): a path inside any of the read roots may be read, one inside any
  of the write roots written, so one session can read a project folder and a shared assets folder, or write to an
  output folder and a scratch folder, without widening a root to their common parent. A folder given twice
  counts once.
- Read and write access are separate. Give the same folder to both flags for both; a kind left out grants none
  of its access (with only `--automation-read-root`, nothing can be written to disk, though `export` and
  the save commands without a path still return the bytes). Without either flag nothing changes.
- A path is made absolute against the server's working directory, then its links are followed: a file
  (or folder) link that leads out of a root is outside it. For a file that doesn't exist yet, its nearest
  existing folder's links are followed and the rest must be plain names, so `..` below a folder that
  doesn't exist is refused, as is a link that leads nowhere (writing through it would create its target).
  The result is compared with the root by whole folder names: `/work2/x` is not inside `/work`. On
  Windows case is ignored, UNC paths (`\\server\share\…`) are resolved like drive-letter ones, and
  device names (`CON`, `NUL`, `COM1`…) and a `:` in a name after the drive (an NTFS alternate data stream
  such as `x.svg:evil`, which hides data in a stream no folder listing shows) are refused.
- With `--connect`, the server makes the paths of `open_file`, `save_file`, `export` (`path` or
  `options.path`) and `screenshot {path}` absolute against its own working directory before it sends
  them to the app. Paths inside `run_command` and `command_batch` params go to the app as given, and the
  app resolves them against its own working directory.
- A refused path is an ordinary tool error the agent can read and correct, for example
  `automation path rejected: outside the write root /work/project/out: /etc/x.png` (with several,
  `outside the write roots /work/project/out, /tmp/scratch: …`) or
  `automation filesystem access is not granted: write authority is absent: /work/project/out/x.png`.
  Links in an opened document that point outside the read roots show as missing.
- The preferences that move folders the app reads or writes on its own (`fontsFolder`,
  `pluginsFolder`, `recoveryFolder`, `templatesFolder`) can only be set inside the roots. The app's own folders
  (preferences, Data Recovery, the User Defined library folders, VectorCraft's Fonts folder) are not
  confined: no agent names a path there.
- The roots must be existing folders; a missing one stops the server with an error. Each flag takes one
  folder per use (`--flag <dir>` or `--flag=<dir>`) and may be used again for more.

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
accepted too. Every request and notification must carry `"jsonrpc": "2.0"`: anything else is
refused with `-32600` (requests) or ignored (notifications, which have no id to address a reply
to) before it is dispatched. When a tool call reaches a running app, it is one control-channel
request, and the app echoes its `id` in the reply: a reply with any other `id` is rejected, the
connection dropped, and the next call starts a fresh one — the call reports uncertainty like any
reply that never arrived whole. A malformed JSON reply also drops the connection and reports that
uncertainty; the failed operation is never replayed automatically. The app's refusals carry no `id`
(a connection past its 16, a line it would not read): the call reports the app's reason and says nothing was run. Before each call the
server checks that the app still holds the connection and opens a new one when it doesn't (the app
closes a connection left idle for 5 minutes, and a restarted app is a new one), so a request is
never written to a closed socket; when the app can't be reached, the call says that nothing was
sent. Clients requesting `2026-07-28` through `initialize`, or through a request's
`_meta["io.modelcontextprotocol/protocolVersion"]`, receive `resultType: "complete"`,
`ttlMs` and `cacheScope: "private"` on tool/resource/template/prompt lists and resource/prompt reads.
List TTL is 600000 ms; live reads use zero. Older clients keep the original response shapes.
This does not add streaming or input requests — see [Not implemented](#not-implemented).

One message is one line of at most 16 MiB: a longer line is answered with `-32600` and skipped,
and the server keeps serving what follows. A batch holds at most 64 messages. `command_batch`
holds at most 256 steps, and the step results it keeps (for later steps' `"$N.path"` references
and the reply) total at most 8 MiB — past that the batch stops with the reason, closing its atomic
undo group as usual. Replies from a connected app are refused past 256 MiB and the connection
dropped; the call reports uncertainty like any reply that never arrived whole.

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

In the summary an object reports its paint as `fill`, `stroke` (labels) and `strokeWidth`. So does a compound path,
whose paint is its own: its member paths report `compoundMember: true` instead of a paint they don't have. Every
node (layers and groups too) reports its opacity when it is below 1, in both scales: `opacity` is the 0–1 fraction
the document stores and `opacityPct` the 0–100 % that `object.setProps`, `transparency.set`, `draw_path` and
`draw_shape` take (so an agent passes `opacityPct` back, never `opacity`); and `blend` when it isn't Normal, as
the label `object.setProps {blend}` takes (`"Multiply"`, `"Color Burn"`).

On a large document, drill instead of dumping: `inspect_document {depth: 0}` (or `run_command document.inspect
{depth?, childLimit?}`, same rules) is the skeleton (artboards with top layers and counts), `run_command
document.find {name?, kind?, text?, within?, intersects?, minArea?, …, select?, limit?}` locates nodes across the
whole tree, and `document.node` reads the
interesting ones sliced. `document.inspect` and the `doc_inspect` / `inspect_document` tools also scope the
reply themselves: `ids: [id…]` lists just those nodes (any level, in the order given, an unknown id is an error,
at most 10000), `kind` lists every node of one panel label (ignoring case; within the subtrees of `ids` when both
are given) with its ancestor `path`, and `limit` / `offset` page the list. A scoped reply carries the nodes as
`nodes` instead of `layers` (each the summary the full tree gives it, sliced by `depth` / `childLimit`) plus
`total`, the entries before paging; artboards, selection and history stay whole. `limit` / `offset` alone page the
top layers (`layers` and `total`). Without these options the reply is the whole layer tree as before.
`document.find` needs at least one filter and all given filters must match; matching
ignores case, `name` is a substring of the Layers panel name, `text` a substring of type content and `kind` the
exact panel label (`Layer`, `Group`, `Clip Group`, `Path`, `Compound Path`, `Type`, `Image`, `Rectangle`, ...).
The geometric filters read each object's geometric bounds, in the document points `document.inspect` reports
(groups included; layers, sublayers and a node without bounds do not match): `within: [x, y, width, height]` keeps
objects inside that rectangle (edges included), `intersects: [x, y, width, height]` objects touching or overlapping it,
`minWidth` / `maxWidth` and `minHeight` / `maxHeight` limit the bounds' size, `minThickness` / `maxThickness`
the smaller of width and height (thin slivers of either orientation), and `minArea` / `maxArea` width × height.
`minAnchors` / `maxAnchors` count a path's anchor points, a compound path's members together; other nodes do not
match them. Limits are inclusive, not negative, and a minimum above its maximum is an error. `select: true`
also selects the matches, leaving out hidden and locked ones, those inside another match, and a compound path's
members unless the compound path is isolated; Select › Reselect repeats the find, and an action being recorded
records it as a step (a find without `select` is not recorded). For example,
`document.find {maxThickness: 1, kind: "path", select: true}` followed by `edit.clear` deletes hairline slivers.
It answers `{matches: [{id, name, kind, path}], total}` top of the stack first, where `path` is the ancestor
chain (layer first) and `limit` caps `matches` (default 100; `0` only counts); with `select` the reply adds
`selected`, the number of objects selected. `document.json` stays whole: it is the fidelity path, not the way to
look around.

`run_command document.compare {path, artboard?, background?, threshold?, diffPath?}` measures how far an
artboard is from an image file (PNG, JPEG, WebP, TIFF or BMP). It renders the
artboard (default 0) at the image's size, as `export` draws a PNG: the scale is the image's width in pixels over
the artboard's width in points, and an image whose height differs from the artboard's height at that scale by
more than one pixel is an error that names both sizes. The render and the image are both flattened on
`background` (`"#rrggbb"`, default `"#ffffff"`). The reply is `{width, height, rmse, over, total, threshold}`:
`rmse` is the root mean square of the R, G and B differences on a 0 to 1 scale, rounded to 6 decimals, and
`over` counts the pixels with a channel that differs by more than `threshold` (0 to 1, default 0.08) of the
total `total`. `diffPath` writes a PNG of each pixel's absolute R, G and B differences (black where the two
match) and adds `diffPath` to the reply. As in a PNG export, linked images and placed documents are drawn
from their files rather than from the previews saved for them. The reply's `warnings` are a PNG export's, in
the same order: type on the artboard that doesn't fit its frame, an embedded image that couldn't be decoded (it
is missing from the render), type in a font or style that isn't available, and a linked file that can't be read
(its preview is drawn). `path` is read through the automation read root, and `diffPath` is written through the
write root. The command does not change the document, and it is not available in the web app.

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

### Long calls: cancellation, progress and a busy engine

A call runs to the end before its reply, and the engine runs one call at a time, in the order the
lines arrived. A thread of its own reads the stream meanwhile, so the server still hears the client
while an export runs (#2165, #2171, #2172):

- **Cancellation.** `notifications/cancelled {requestId}`
  ([spec](https://modelcontextprotocol.io/specification/2025-06-18/basic/utilities/cancellation))
  takes a request out of the queue, or stops the one that is running, and **that request gets no
  reply**. A running export, flatten, bake or render notices at its next checkpoint (between pages
  and artboards, between flattened groups, while drawing objects), returns, and writes nothing: the
  document is exactly as it was and the engine is free for the next call. An unknown, finished or
  malformed `requestId` is ignored, and so is a cancel of `initialize`. A call without checkpoints
  (every ordinary edit is over in milliseconds) runs to its end, but its reply is dropped all the
  same. Against a running app (`--connect`) the server stops waiting and drops the connection, but
  the app cannot be told to stop what it started.
- **Progress.** A request with `_meta.progressToken` gets `notifications/progress`
  ([spec](https://modelcontextprotocol.io/specification/2025-06-18/basic/utilities/progress)) before
  its reply: the PDF writer reports "Writing page 2 of 5" with `progress` 2 and `total` 5, raster
  export reports each artboard, `document.exportForScreens` and `assets.export` report each file
  ("Exporting file 3 of 250", in place of their encoders' steps), and the PDF flattening stage reports a message without a total. `progress` only grows, updates come at most every 200 ms (the last page always),
  and none follows the reply. A client that does not read fast enough misses updates, never the reply.
- **A busy engine.** `ping`, `tools/list`, `resources/list`, `resources/templates/list`,
  `prompts/list` and `prompts/get` need no engine and are answered at once, whatever is running.
  Anything else that arrives while a call has already run for two seconds is not queued behind it:
  it is answered with error `-32003` (`data.busy` names the running call, its request id and how
  long it has run) and was **not run**, so the client can retry later or cancel the running call.
  Requests that arrive sooner than that queue and are answered in order, as before, so a client
  that sends a few quick calls together sees no difference. At most 64 lines wait; past that the
  server stops reading until the engine catches up.

`vectorcraft_mcp::Server::serve` takes the input and the output on separate threads (`Send`), and
`Server::with_busy_after` changes the two seconds. Calling `handle_line` directly is as before:
one line in, one reply out, no threads.

### Not implemented

- **`resources/subscribe` / `notifications/resources/updated`.** It would also have to be honest about
  *whose* edits it sees: comparing the undo history's length misses edits made in the app window,
  which is the main reason to run against a live document. `document.inspect`'s `revision` is the
  right signal — it is monotonic and already bumped when a document is replaced — but noticing an
  edit still means someone asking.
- **Reading the document during a long call.** Requests that need the engine (`doc_inspect`,
  `resources/read`, `command_list`) are refused as busy while an export runs, not answered from a
  snapshot.
- **`sampling/createMessage` and `elicitation/create`.** Both are server→client requests. Supporting
  them means being able to suspend a tool call mid-flight, which is what the `2026-07-28` revision's
  input-requests mechanism exists for.
- **Streamable HTTP / SSE.** stdio only.

`completion/complete` covers the "what could I pass here" half of the same problem, which is why it
is here and subscriptions are not.

## Tools

### Common command tools

These names match the conventions in [FilmCraft #28](https://github.com/storytold/filmcraft/pull/28).
The older names below remain listed because existing workflows use them.

| Tool | Arguments and result |
|---|---|
| `command_list` | Optional `filter` (or `query`), `menu` (a menu path substring, levels joined with ` > `), `enabled_only`, `brief` (no `params` text); returns the command array. `list_commands` lists the same commands; it takes `enabledOnly` and returns `{count, commands}` |
| `command_run` | `id`, optional `params`; runs the command through the existing backend. An unknown key in `params` is an error that names it and the nearest key the command takes (`invalid parameters for `file.new`: unknown parameter `widht`; did you mean `width`?`), so a misspelt key no longer runs the command with its default (see [Parameter keys](#parameter-keys)). `run_command` runs a command the same way and takes its id as `command` |
| `command_batch` | `steps: [{id, params?}]` (at most 256), optional `stop_on_error` (default true), `atomic` and `label`; returns `completed`, `failed`, `results: [{ok, result\|error}]` and `created` (the ids the `draw_path` / `draw_shape` steps made, in order). A step's `id` is a command id, or `draw_path` / `draw_shape` with that tool's arguments as `params` (checked as `tools/call` checks them; the step's result is the tool's, so `"$N.id"` is the new object). Each edit has its own undo step, or with `atomic: true` the batch's edits are one undo step named `label` (else after its first edit; `edit.undoGroup.begin`/`end` underneath), and the edits made in a document that a step opens or creates (`document.open`, `file.new`) are one undo step there, named the same, also when no document was open before the batch. In a document that was open before the batch and that a step switches to (`document.activate`), each edit stays its own undo step; failures set `isError`. A param string `"$N.path"` or `"$last.path"` takes a value from an earlier step's result (see `run` below). Kept step results total at most 8 MiB; past that the batch stops with the reason. This is the MCP tool, not the engine command `command.batch` (that one takes `commands: [{command, params}]` and is what the Actions panel plays); use `command_run` with `command.batch` when replaying a recorded action |
| `command_list` | Optional `filter` (or `query`), `menu` (a menu path substring, levels joined with ` > `), `enabled_only`, `brief` (no `params` text); returns the command array |
| `command_run` | `id`, optional `params`; runs the command through the existing backend. An unknown key in `params` is an error that names it and the nearest key the command takes (`invalid parameters for `file.new`: unknown parameter `widht`; did you mean `width`?`), so a misspelt key no longer runs the command with its default (see [Parameter keys](#parameter-keys)) |
| `doc_inspect` | Optional `depth`, `childLimit`, `ids`, `kind`, `limit`, `offset`; same summary as `inspect_document` |
| `render_preview` | Optional `max_side` (1–4096, default 1024), `artboard` (0-based, default 0) and `traceViews`; inline PNG of that artboard without editing it, drawn as `export` draws a PNG with `background: "white"` at the scale that fits `max_side` (at most 1). With `traceViews: true`, Image Trace objects draw in the view set with `imageTrace.setView` (outlines, the source image…), as in the app's window; by default they draw their tracing result. Artboards too large to render within the allocation bound return a tool error |
| `ui_inspect`, `ui_screenshot` | Connected desktop state/window capture; tool errors in headless mode. `ui_inspect` is the same call as `inspect_ui`, and `ui_screenshot` is `screenshot` with `window: true`. `ui_screenshot` takes an optional `max_side` (1–4096), as `screenshot {window: true}` does: a capture whose longer side is larger is scaled down to `max_side` pixels on that side, and the reply's text gives the returned size as `width`/`height` and the capture's as `capturedWidth`/`capturedHeight`. Without `max_side` the full capture comes back |

Every tool declares read-only, destructive, idempotent and open-world hints. File-writing
`screenshot`, `save_file` and `export` are conservatively marked as mutations. Unknown top-level
argument keys return JSON-RPC `-32602`; command failures and caught tool panics return `isError`
content, and the server continues serving. Command `params` are passed through: the registry
currently describes them in prose, so MCP does not guess schemas or silently remove keys.


Coordinates are points in document space: y points down, the origin is the first artboard's top-left, and a new
document is 612 × 792 (US Letter). New objects become the selection. Most commands act on the selection or on
explicit `ids`. A command that leaves out an object selected inside a selected group also leaves it out when `ids`
list it together with a group that holds it. The commands that select their `ids` first (`object.group`, the
`object.arrange.*` commands, `object.move`, the Pathfinder commands and the others like them) refuse an id that names a
layer or sublayer, names no object or is not a non-negative integer, with an error that names the command that was run,
and leave the document and the selection unchanged; `layer.move` reorders layers.
explicit `ids`, which stand in for the selection, so they work with nothing selected. Where a command's params say
`ids?|id?` (or `id?, ids?: … (instead of id)`), `id` is `ids` of one object: `{"id": 7}` does what `{"ids": [7]}`
does, with or without a selection, and an `id` or `ids` that is malformed (not a non-negative integer) or names no
object is refused under the command's name, never replaced by the selection (`effect.apply {"id": "warp.arc"}`
without `effect` names the effect). Other commands that act on `id` or else the selection, the highlighted layer row
or a new object (`css.export`, `object.isolate`, `layer.locate`, `layer.releaseToLayers`, `graphicStyle.new`,
`graphicStyle.redefine`, `charStyle.new`, `charStyle.apply`, `charStyle.redefine` and their `paraStyle.*`
counterparts, `image.info`, `links.info`, `links.goTo`, `links.unembed`, `shaper.release`, `shaper.expand`,
`transparency.editOpacityMask`, `transparency.viewOpacityMask`, `symbol.edit`, the `object.blend.spine.*` commands
and `path.curvature`) refuse an `id` that is not a non-negative integer (`"abc"`, `"5"`, `true`) with an error and
change nothing; an omitted or null `id` keeps the default. A command that leaves out an object selected inside a
selected group also leaves it out when `ids` list it together with a group that holds it. Every command that takes
`ids` (or `slices`, for the slice commands) refuses a list with an entry that is not a non-negative integer, or a value
that is not a list, and changes nothing; it does not act on the other entries. The error for an entry names the key,
the entry's index (from 0) and its value: "`ids` entry 1 is true, not a non-negative integer object id".

Paint values (`fill`, `stroke`) accept `"#rrggbb"`, `"none"`, `[r,g,b]` (0..1), `{"c","m","y","k"}`, `{"gray"}`,
or a full `paint.setFill` params object (`{"gradient": …}`, `{"swatch": "name"}`).
A color read this way (a fill or stroke color, a swatch's, an artboard's or new type's color, and the other colors
commands read in these forms) that is given as a hex color with an alpha part (`#rrggbbaa` or `#rgba`, with or without
the `#`) is an error and changes nothing, as VectorCraft's own rule. The `color` of Drop Shadow, Inner Glow, Outer Glow
and Shift to Color is read differently and takes `#rrggbbaa`, as described below.
For `draw_shape` and `draw_path`, omitted paint fields inherit the current [new-art appearance](#new-art). Both
also style the new object in the same call: `dash` (lengths, or null for solid), `cap`
(butt|round|square) and `join` (miter|round|bevel) go to `stroke.set` with `strokeWidth`; `opacity` (0–100 %)
and `name` to `object.setProps`; `effects: [{effect, params?}]` are applied in order as `apply_effect`
does, to the whole object. They are checked before anything is drawn (effect ids against `effect.list`),
so a bad one draws nothing. The object and its styling are one `command.batch`, so the call is one undo step
(named as the drawing command names its own, e.g. Pen or Rectangle) and a paint the engine refuses (a colour
it can't read, a swatch the document lacks) fails the call and leaves the document as it was. The reply adds `name`, `opacity` and `effects` (each `effect.apply` result)
when given; `paint` lists the stroke options applied.
A gradient given to several objects fits each one's own bounds; `paint.setFill` / `paint.setStroke` with
`span: "selection"` place a linear or radial gradient once on the union of the targets' bounds instead, so one
ramp runs across them.
`paint.setFill` without `item` on a grayscale or 1-bit image sets the color the image is drawn in: each pixel takes
the color as far as the pixel is dark, and white pixels are transparent. `none` removes the color. An image with an
alpha channel takes only a spot color, black or white; a process color is refused.
For `draw_shape` and `draw_path`, omitted paint fields inherit the current [new-art appearance](#new-art).
For a filled shape or path without an outline, pass an explicit `fill` and `stroke: "none"`.
Lines start with no fill and a visible stroke.
The Fill/Stroke proxy commands run through `run_command`: `paint.invert` and `paint.complement` recolor the active
proxy, including a grayscale image's colorized fill, while keeping each color's model. `paint.lastColor` /
`paint.lastGradient` re-apply the last solid color or gradient, and `paint.recent` returns the recent colors that every
paint command (and the eyedropper) feeds: up to 30, newest first and each once, kept across documents for the session.
`paint.proxies` returns what the proxies show: the fill and stroke, which one is active, and whether the selected
objects' fills or strokes differ (`fillMixed` / `strokeMixed`, drawn as a "?" proxy).

In the app's Color panel and Color Picker, RGB, HSB, hex and Web Safe RGB values describe the active working RGB
space. RGB edits preserve the other native channels; HSB uses those same working RGB components. Colors outside
display sRGB retain their native numeric values. `ui.colorPanel` selects the panel mode and `ui.colorPicker`
opens the picker through `run_command` with the remote backend.

| Tool | Arguments | Notes |
|---|---|---|
| `list_commands` | `{filter?, query?, menu?, enabledOnly?, brief?}` | The command catalogue: id, label, menu, shortcut, params doc (left out with `brief`), enablement. |
| `run_command` | `{command, params?}` | Runs any command. Use it for everything without a dedicated tool. |
| `inspect_document` | `{depth?, childLimit?, ids?, kind?, limit?, offset?}` | Artboards (with `color`, `#rrggbb`, for one with a background color), layer tree (ids, kinds, bounds, paint), selection, history, tool. A type node's `fill`, `stroke` and `strokeWidth` are the paint its characters show (its first run's); fills and strokes of the type object itself come as `objectFill`, `objectStroke` and `objectStrokeWidth`. Paint comes as a label (`"#ff0000"`, `"None"`, `"G1 40% (#ff9999)"` for a swatch at a tint); `paint.proxies` gives it as an object. |
| `inspect_ui` | `{}` | UI state. Remote mode only. |
| `select_tool` | `{tool}` | `selection`, `directSelection`, `pen`, `rectangle`, `ellipse`, `polygon`, `star`, `lineSegment`, … |
| `pointer_gesture` | `{events:[{kind,x,y,mods?,backquote?,pressure?,tiltX?,tiltY?,rotation?,holdMs?}], tool?, mods?, backquote?}` | `kind` is one of `down`, `drag`, `up`, `move`, `doubleclick`. `backquote` is the held state for each event; the top-level value supplies its default, and a `backquote` that is not a boolean is an error. Events go through the same path as the mouse. Headless, a `move` replies after the tool's highlight under the pointer is ready, including the Shape Builder's and the Live Paint Bucket's regions of a large selection, which the desktop app finds off the UI thread and highlights after the reply. |
| `pointer_gesture` | `{events:[{kind,x,y,mods?,backquote?,pressure?,tiltX?,tiltY?,rotation?,holdMs?}], tool?, mods?, backquote?}` | `kind` is one of `down`, `drag`, `up`, `move`, `doubleclick`. `backquote` is the held state for each event; the top-level value supplies its default. Events go through the same path as the mouse. Headless, a `move` replies after the tool's highlight under the pointer is ready, including the Shape Builder's and the Live Paint Bucket's regions of a large selection, which the desktop app finds off the UI thread and highlights after the reply. Remote, the events wait until the keys and other input sent before them (`press_key`, `type_text`) have reached the app, so a key pressed between two calls of one drag applies before the release. When that input is dropped because the document, a dialog or the focus changed first, the call fails and its events are not sent. |
| `draw_path` | `{points \| d, closed?, fill?, stroke?, strokeWidth?, dash?, cap?, join?, opacity?, name?, effects?}` | `points` is `[[x,y],…]` or `[{x,y,in?,out?,smooth?},…]`. `d` is SVG path data. The options are below. |
| `draw_shape` | `{shape, …geometry, fill?, stroke?, strokeWidth?, dash?, cap?, join?, opacity?, name?, effects?}` | `rectangle`/`ellipse`: `x,y,width,height` (plus `radius` for corners). `polygon`: `cx,cy,radius,sides`. `star`: `cx,cy,radius1,radius2,points`. `line`: `x1,y1,x2,y2`. |
| `set_paint` | `{fill?, stroke?, strokeWidth?, ids?}` | Applies to the selection (or `ids`) and becomes the default for new art. Returns `{applied, objects}`: `objects` is the number of objects painted, as the `objects` of the `paint.setFill`, `paint.setStroke` and `stroke.set` replies (the largest of them): a selected group counts the objects inside it. When it is 0, a `note` says that only the paint for new art changed. |
| `press_key` | `{key, mods?}` | Remote: a real key event. Headless: runs the command or tool bound to that shortcut, or sends the key to the busy tool (digits too: `5` while dragging with the Perspective Selection tool; `e` while dragging a Selection or Group Selection marquee; `c` and `f` while dragging out an arc; `f`, `v`, `x` and `c` while dragging out a rectangular or polar grid). |
| `type_text` | `{text}` | Remote only. |
| `answer_dialog` | `{set?: [{field, value?}], action?}` | Remote only. Sets fields of the app's open dialog in the order given with one `ui.dialog.set {fields}` (a missing `value` is `null`). The app stores all of them or none. When the app refuses the `ui.dialog.set`, for example because no dialog is open, the call is an error and `action` is not sent. Then `action` `confirm` (OK, `ui.dialog.confirm`) or `cancel` (`ui.dialog.cancel`). Give `set`, `action` or both; `set` holds at most 256 fields. `inspect_ui` shows the open dialog and its fields as `ui.dialog`. Returns `{dialog, result}`: the dialog after the fields were set (or `null`) and the reply to `confirm` or `cancel` (or `null`). After `invoke_menu` `app.quit`, `file.close` or `file.closeAll` replies `{"pending": "saveChanges"}`, `{action: "confirm"}` saves, `{set: [{field: "discard", value: true}], action: "confirm"}` closes without saving and `{action: "cancel"}` cancels the close or quit. |
| `invoke_menu` | `{command, params?}` | Invokes a menu item by command id. Includes UI commands such as `view.*` and `window.*` in remote mode. |
| `open_panel` | `{panel}` | Remote only. `panel` is a panel id (`layers`, `swatches`, `colorGuide`, …, as `window.panel` takes) or its display label (`"Color Guide"`), in any case. |
| `screenshot` | `{path?, scale?, artboard?, window?, max_side?, traceViews?}` | Returns MCP image content (`image/png`, base64) plus a text block. Renders the artboard with the pixels of a PNG `export` with `background: "white"` at the same scale; `traceViews:true` draws Image Trace objects in the view set with `imageTrace.setView`, as in the app's window; `window:true` captures the app window (remote only), as `ui_screenshot` does, and `max_side` (1–4096, window captures only) scales the returned image down as described for `ui_screenshot`. With `path` the backend (the headless session or the app) writes the PNG there too; a window capture written there keeps its full size with `max_side`. |
| `open_file` | `{path}` | Opens any readable file as a new active document: `.vectorcraft`/`.drawcraft`, `.vctemplate`, `.svg`/`.svgz`, `.pdf`/`.ai`, `.ait`, `.eps`/`.epsf`, `.ps`, `.dxf`, `.emf`, `.wmf`, PNG (`.png`/`.pns`), JPEG, GIF, WebP, TIFF, BMP (`.bmp`/`.dib`/`.rle`), Targa (`.tga`/`.vda`/`.icb`/`.vst`), PCX, Pixar (`.pxr`), JPEG 2000 (`.jpf`/`.jpx`/`.jp2`/`.j2k`/`.j2c`/`.jpc`), HEIC/HEIF (`.heic`/`.heif`), PSD (`.psd`/`.pdd`)/PSB (an image opens as a document of its pixel size; a Photoshop file as its merged image). Templates (`.vctemplate`, `.ait`) open as a new untitled document. PDF, `.ai` and SVG files saved with Preserve Editing reopen as the document they carry. The reply is `document.open`'s: its `warnings` say what didn't come in as it was (an EPS whose PostScript can't be read opens as its preview, and the warning names the PostScript error, the operator and the procedure). `run_command document.formats` lists the formats. |
| `save_file` | `{path?, compress?, version?, preview?, precision?, options?}` | Runs `document.save`: the document's own file in its own format (native `.vectorcraft` unless it was opened from or saved as SVG, PDF or a restorable `.ai`; then `warnings` say what that format loses). A path's extension picks the format (`.vectorcraft`, `.vctemplate`, `.pdf`, `.svg`, `.svgz`, `.ai`: a PDF carrying the native document, which reopens editable). For a native file, `compress: true` gzips it (default: the `useCompression` preference, Preferences › File Handling › Use Compression), `version` (1 to 4) writes it for an older VectorCraft (1 and 2 are never compressed, and `version: 2` with `compress: true` is an error), `preview: true` embeds a PNG of the first artboard, and `precision` (0 to 9) rounds path anchors and handles to that many decimal places for that save only (colors, transforms and type stay exact). `options` passes the format's save options as `run_command document.formats` lists them. When `compress`, `version`, `preview` or `precision` is given both at the top level and in `options` with different values, the call is an error (`` `compress: true` and `options.compress: false` disagree: give the option once ``) and nothing is written. The save drops each option the written format doesn't read. The reply's `warnings` then name each such option given (for example `` `compress` was not used: SVG files don't read it `` for an `.svg` path, and `` `options.preview` was not used: … `` for a key in `options`; a `.ai` file reads `compress` but not `version`, `preview` or `precision`). `run_command document.save` and the other save commands refuse any option given beside the path and in `options` with different values, and give the same warning for a top-level `compress`, `version`, `preview` or `precision`. The headless session and the app receive the same `app.save` params, and the app is also sent `wait: true`: the call returns once the file is written, or with the error (such as a path outside the write root), even with Background Save on. When the save asks first (the question before replacing a file that opened with losses, or on macOS and Linux the file dialog for an untitled document without `path`), the reply is `{pending}` or `"cancelled"` as before, and the save that runs after the answer uses the Background Save preference. |
| `open_file` | `{path}` | Opens any readable file as a new active document: `.vectorcraft`/`.drawcraft`, `.vctemplate`, `.svg`/`.svgz`, `.pdf`/`.ai`, `.ait`, `.eps`/`.epsf`, `.ps`, `.dxf`, `.emf`, `.wmf`, PNG (`.png`/`.pns`), JPEG, GIF, WebP, TIFF, BMP (`.bmp`/`.dib`/`.rle`), Targa (`.tga`/`.vda`/`.icb`/`.vst`), PCX, Pixar (`.pxr`), JPEG 2000 (`.jpf`/`.jpx`/`.jp2`/`.j2k`/`.j2c`/`.jpc`), PSD (`.psd`/`.pdd`)/PSB (an image opens as a document of its pixel size; a Photoshop file as its merged image). Templates (`.vctemplate`, `.ait`) open as a new untitled document. PDF, `.ai` and SVG files saved with Preserve Editing reopen as the document they carry. The reply is `document.open`'s: its `warnings` say what didn't come in as it was (an EPS whose PostScript can't be read opens as its preview, and the warning names the PostScript error, the operator and the procedure). `run_command document.formats` lists the formats. |
| `save_file` | `{path?, compress?, version?, preview?, precision?, options?}` | Runs `document.save`: the document's own file in its own format (native `.vectorcraft` unless it was opened from or saved as SVG, PDF or a restorable `.ai`; then `warnings` say what that format loses). A path's extension picks the format (`.vectorcraft`, `.vctemplate`, `.pdf`, `.svg`, `.svgz`, `.ai`: a PDF carrying the native document, which reopens editable). For a native file, `compress: true` gzips it (default: the `useCompression` preference, Preferences › File Handling › Use Compression), `version` (1 to 4) writes it for an older VectorCraft (1 and 2 are never compressed, and `version: 2` with `compress: true` is an error), `preview: true` embeds a PNG of the first artboard, and `precision` (0 to 9) rounds path anchors and handles to that many decimal places for that save only (colors, transforms and type stay exact). `options` passes the format's save options as `run_command document.formats` lists them. When `compress`, `version`, `preview` or `precision` is given both at the top level and in `options` with different values, the call is an error (`` `compress: true` and `options.compress: false` disagree: give the option once ``) and nothing is written. The save drops each option the written format doesn't read. The reply's `warnings` then name each such option given (for example `` `compress` was not used: SVG files don't read it `` for an `.svg` path, and `` `options.preview` was not used: … `` for a key in `options`; a `.ai` file reads `compress` but not `version`, `preview` or `precision`). For a `.ai` file, these warnings name the kind of `.ai` written: a legacy .ai (version 7) for `aiVersion: "7"`, a non-PDF-compatible .ai for `pdfCompatible: false`, and a PDF-compatible .ai otherwise, including a save that reuses the options the document was last saved with. `run_command document.save` and the other save commands refuse any option given beside the path and in `options` with different values, and give the same warnings for a top-level `compress`, `version`, `preview` or `precision` and for each key in `options` that the format doesn't read. The headless session and the app receive the same `app.save` params, and the app is also sent `wait: true`: the call returns once the file is written, or with the error (such as a path outside the write root), even with Background Save on. When the save asks first (the question before replacing a file that opened with losses, or on macOS and Linux the file dialog for an untitled document without `path`), the reply is `{pending}` or `"cancelled"` as before, and the save that runs after the answer uses the Background Save preference. |
| `export` | `{path?, format?, scale?, artboard?, range?, selection?, outlineText?, options?}` | `svg`, `svgz`, `pdf`, `ai` (the PDF-compatible `.ai` that Save writes, which carries the document when every artboard is written in order), `eps`, `dxf`, `emf`, `wmf`, `png`, `jpg`, `webp`, `gif`, `png8` (an indexed `.png`), `tiff`, `bmp`, `tga`, `psd` (layered), `txt` (the document's text), `vectorcraft` or `template` (a native template). The tool's `format` enum and `document.formats` list them, generated from the engine's format table. When `format` is omitted, it comes from the path's extension. PDF and `.ai` write one page per artboard: all of them, or `artboard` (0-based) / `range` (`"1-3, 5"`, 1-based). SVG and SVGZ write one file per chosen artboard. EPS writes one file of the visible art; with `artboard`, `range`, `options.artboards` or `options: {"useArtboards": true}` it writes one file per chosen artboard, after a central file of the visible art when there are two or more, and `useArtboards: false` keeps the one file. Rasters, DXF, EMF and WMF write one artboard, or one file per chosen artboard with `options: {"useArtboards": true}`. Rasters also take `options: {"rect": [x, y, width, height]}` (page points, y down): that area of the page at the given `ppi`/`scale`, without an artboard and without changing the document; it is refused with `artboard`, `artboards`, `range` or `useArtboards`, with a non-positive or non-finite size, and for non-raster formats. `options` carries more format options (e.g. `{"quality": 80}` for JPEG). `selection: true` exports the selected objects cropped to their bounds (the reply adds their `bounds`, and reports `format` and the encoder's `warnings` as a whole-document export does); `outlineText: true` writes SVG text as paths. Template layers are left out, live effects are kept, and exporting `vectorcraft` never changes the document's path. Without `path` the bytes come back as `dataBase64`. Both backends run the same `document.export` call. |
| `add_text` | `{text, x?, y?, width?, height?, fit?, fitMinPercent?, path?, mode?, pathEffect?, size?, font?, color?, justify?, vertical?}` | Point type at (x, y) (`justify`: `left`, `center` or `right` starts, centres or ends each line on x); area type with `width`/`height` (`fit`: `none`, `autoHeight`, `shrinkText`, as `text.areaOptions`); or `path` + `mode` (`area`/`onPath`) to flow text in or along a path, with `pathEffect` (`rainbow`, `skew`, `3dRibbon`, `stairStep`, `gravity`; SVG export writes type on a path with any effect other than `rainbow` as outlines, with a warning); `justify` (also `auto`, `justifyAll`) aligns the paragraphs of every kind, as `text.setStyle` does later. `vertical: true` uses the existing vertical writing mode for every kind (default `false`, horizontal). Returns `{id}`, with `overflow: true` when the text doesn't fit its frame or path. |
| `apply_effect` | `{effect?, params?, ids?}` | Appends a live effect. Without `effect`, returns the effect catalogue with parameters and defaults. |
| `pathfinder` | `{operation, ids?}` | `unite`, `minusFront`, `intersect`, `exclude`, `divide`, `trim`, `merge`, `crop`, `outline`, `minusBack`. For a live version, apply the `pathfinder.*` effect to a group (`effect.apply` groups several loose selected objects first). In remote desktop mode, `run_command` and `invoke_menu` can both call `pathfinder.repeat` with `{}`. Successful panel operations, direct `object.pathfinder.*` commands, and `command.batch` steps (including Actions panel playback) set its history. Repeat applies it to the current selection using current Pathfinder Options. Trap, compound-shape edits, live effects and `action.batch` do not set history. With no history it returns "no Pathfinder operation applied yet". |
| `transform` | `{ids?, dx?, dy?, rotate?, scale?, scaleX?, scaleY?, reflect?, shear?, origin?, copy?}` | Runs move, rotate, scale, reflect, shear in that order. With `copy`, the first step duplicates. |
| `create_graph` | `{type?, x, y, width, height, csv? \| series?, categories?, rows?}` | The nine Illustrator graph types: column, stacked column, bar, stacked bar, line, area, scatter, pie and radar. An empty CSV cell or a `null` in `rows` is a blank value; a number in straight quotes is a label. Edit later with `graph.setData` / `graph.setType` via `run_command`. `graph.column {design, columnType: vertical\|uniform\|repeating\|sliding, each, fractions: chop\|scale}` draws the columns or bars of the series picked with Group Selection (or `seriesIndexes`, or all of them) with a graph design saved by `graph.design`; with no `design` it returns the series' current design and options. Type in the design that holds a `%` followed by two digits shows each column's value, with at least the first digit's places before the decimal point and the second digit's places after it. |
| `text_wrap` | `{ids?, offset?, invert?, release?}` | Area type below the objects (same layer) flows around them. |
| `undo` / `redo` | `{}` | Many calls that change nothing record no undo step, for example `object.align` with nothing to move, `object.move` by 0 without `copy` and `object.showAll` with nothing hidden. `undo` (`edit.undo`) after such a call undoes the step before it. `inspect_document` lists the undo steps as `history`. `run_command` with `edit.clearHistory` removes every undo and redo step of the active document and returns `{cleared}`, the number of steps removed; the document stays as it is, and the command records no undo step. `run_command` with `edit.newDocumentFromState` and `{index?, created?}` makes a new document holding the art as it was at that state and makes it the active document, and returns `{index, title}`. `index` is a row of the History panel, with 0 the oldest state kept, and an omitted `index` uses the current state. An `index` that names no state is an error and changes nothing. `created` is File Info's created date in Unix seconds or null, and defaults to now. The new document takes the next `Untitled-N` name, is marked unsaved, has nothing selected and has no steps to undo or redo. It is out of symbol, opacity-mask and pattern editing, and the art drawn in a pattern's tile is stored in the pattern, as `object.pattern.done` stores it. The source document and its steps stay as they are, except that typing or a drag in progress is first kept as a step. While the Type tool edits type, `undo` and `redo` put its caret, with no characters selected, at the end of the text the step changed: after the text it puts back or retypes, or where the text it takes out was. A step that changes no characters, such as a formatting change, leaves the caret and the selection where they are. |

For example, create editable Japanese area type with the engine's vertical writing mode:

```json
{"name":"add_text","arguments":{"text":"こんにちは","x":40,"y":40,"width":160,"height":240,"size":20,"vertical":true}}
```

`vertical` also applies to point type and to `path` + `mode` (`area` or `onPath`). Omit it or pass `false`
for horizontal writing. This is the same option as `text.create` / `text.createInPath`. Vertical text turns
on the OpenType `vert` feature, so a font's vertical glyph alternates are used; it does not change the engine's
CJK composition.

`type.orientation.vertical` and `type.orientation.horizontal` (Type › Type Orientation) switch existing type objects
(the selection, or `ids`) and reply `{vertical, ids}`. The writing direction is a setting of the whole thread: the
commands set every frame of a selected frame's thread, and `ids` lists them all. Threading frames gives the thread its
first frame's direction, frames taken out of a thread keep theirs, and a file whose frames of one thread differ opens
with the first frame's direction in all of them.

For SVG/SVGZ, `export`'s top-level `outlineText: true` writes text as paths; DXF takes it too (its type is outlined anyway unless `preserve: editability`).
PDF instead uses `options: {"advanced": {"outlineText": true}}`; its default is false (live text).

With `mods.shift`, a marquee dragged with the Selection tool toggles the objects it reaches (the selected ones leave
the selection, the others join it, the rest stays), and one dragged with Direct or Group Selection toggles the anchors
inside it (a path left with none leaves the selection, one with all of them is selected whole). The Lasso adds anchors
with `mods.shift` and takes them away with `mods.alt`. They run `select.toggle {ids}` and `select.anchorsMany {items,
mode: "set"|"add"|"toggle"|"subtract"}`:

```json
{"name":"pointer_gesture","arguments":{"tool":"selection","mods":{"shift":true},"events":[{"kind":"down","x":180,"y":80},{"kind":"drag","x":300,"y":200},{"kind":"up","x":380,"y":200}]}}
```

`press_key {key: "e"}` while a Selection or Group Selection marquee is being dragged makes the marquee select only the
objects that lie entirely inside it, measured by their geometric bounds, and `e` pressed again switches it back to the default mode. The
Selection tool selects a group only when the whole group lies inside; Group Selection selects the objects inside
groups, each as a whole object. Both run `select.set {ids}` (`select.toggle {ids}` with `mods.shift`). The marquee
also selects the artboard guides that lie entirely inside it, and no canvas guide. Each marquee starts in the default
mode. To press the key during a drag, end one
`pointer_gesture` after the `drag` event and send the rest in another with the same `tool`, which keeps the drag:

```json
{"name":"pointer_gesture","arguments":{"tool":"selection","events":[{"kind":"down","x":80,"y":80},{"kind":"drag","x":220,"y":170}]}}
{"name":"press_key","arguments":{"key":"e"}}
{"name":"pointer_gesture","arguments":{"tool":"selection","events":[{"kind":"drag","x":230,"y":170},{"kind":"up","x":230,"y":170}]}}
```

`press_key {key: "Escape"}` between two `pointer_gesture` calls of a Selection tool drag (a move or an Alt-drag copy, a
bounding-box scale or rotation, a marquee, a corner widget, pie widget or bracket drag) cancels it: the art goes back,
no copy is left, no undo step is added, and the `up` event that follows does nothing.

With nothing being dragged, headless `press_key` `e` chooses the tool bound to E. Remote `press_key` does not run
single-key shortcuts (see the Arc tool below).

While an arc is being dragged out with the Arc tool, `press_key {key: "c"}` switches it between an open and a closed
path, `{key: "f"}` switches its base between the X and Y axes in the same box, and `{key: "up"}` and `{key: "down"}`
raise and lower its slope by 10, within −100…100. Each press switches once. The arc drawn on release uses these
values, and they stay as the Arc tool's `closed`, `baseAlong` and `slope` options for the next arc. With nothing being
dragged, headless `press_key` `c` chooses the tool bound to C (headless has no binding for F). In remote mode,
`press_key` sends the key event without the letter it types, and the app runs a single-key shortcut such as C or F only
from the typed letter, so with nothing being dragged `c` and `f` do not run their shortcuts there. Use `select_tool` or
`invoke_menu` instead.

While a spiral is being dragged out with the Spiral tool, a `drag` event with `mods.alt` or `mods.cmd` keeps the
spiral's inner end at the distance from the center it had on the first modified `drag` event, and the outer end follows
the pointer. With `mods.alt` alone, dragging outward adds whole winds of four segments and dragging inward removes
them. The fitted segment count rounds to the nearest whole wind from the count on the first Alt-only `drag`, within 2…1000.
With `mods.cmd`, decay changes to fit within 5…99.99 %; it takes precedence when both modifiers are held. Switching
back to Alt uses the current count as its new base while keeping the held inner end. The spiral drawn on release uses
the new values, and they stay as the Spiral tool's `segments` and `decay` options for the next spiral:

```json
{"name":"pointer_gesture","arguments":{"tool":"spiral","events":[{"kind":"down","x":100,"y":100},{"kind":"drag","x":200,"y":100,"mods":{"alt":true}},{"kind":"drag","x":300,"y":100,"mods":{"alt":true}},{"kind":"up","x":300,"y":100}]}}
```

While a grid is being dragged out with the Rectangular Grid or Polar Grid tool, `press_key {key: "f"}` and
`{key: "v"}` lower and raise the skew of the horizontal dividers (`rowSkew`) or the radial dividers (`radialSkew`) by
10, and `{key: "x"}` and `{key: "c"}` lower and raise that of the vertical dividers (`columnSkew`) or the concentric
dividers (`concentricSkew`), within ±500. Plain and Shift presses change the skew; with Alt, Ctrl or Cmd held they do not.
The grid drawn on release uses these skews, and they stay as the tool's options for the next grid. With nothing
being dragged, headless `press_key` runs the shortcuts bound to `v`, `x` and `c` (it has none for `f`), and remote
`press_key` does not run single-key shortcuts (see the Arc tool above).

Appearance stacks: an object can carry several fills and strokes (`appearance.addFill`, `appearance.addStroke`), indexed
in paint order (0 is painted first, the bottom row of the Appearance panel). Cmd+/ runs `appearance.addFill` and
Cmd+Alt+/ runs `appearance.addStroke` (`press_key {key: "/", mods: {cmd: true, alt: true}}`). `paint.setFill`, `paint.setStroke`,
`stroke.set`, `stroke.setAdvanced`, `paint.editGradient`, `paint.setGradientGeom` and `transparency.set` take
`item` to edit one of them; `run_command appearance.setActiveItem {"index": n}` makes that row the target of later
calls that omit `item` (as clicking the row in the Appearance panel does) until the selection changes; the proxy of
its kind (`paint.proxies`) and the Gradient tool's annotator then show that row.
`inspect_document` reports it as `paint.appearanceItem`. Live effects take the same `item` to apply to one fill or
stroke instead of the whole object: `run_command effect.apply {"effect": "path.offsetPath", "item": 0}`, and
`effect.remove`, `effect.setParams` (`visible` toggles one) and `effect.duplicate` address that item's effects;
`apply_effect` uses the active item. `effect.list` reports each object's item effects under `applied[].items`.

On a path directly inside a graph series, `appearance.setItem`, `appearance.addFill`, `appearance.addStroke`,
`appearance.removeItem`, `appearance.moveItem`, `appearance.duplicateItem`, `appearance.clear`,
`appearance.reduceToBasic`, `appearance.showAllHidden`, `appearance.transfer`, `appearance.copyFrom`, `paint.default`,
`graphicStyle.apply` and `graphicStyle.redefine` (on the marks still linked to the style) store the paint, weight and
opacity of the topmost visible fill and stroke on the graph. `graphicStyle.apply` with `add` stores only the kinds of row
that are visible in the style. `paint.swap` stores the swapped paints and the stroke weight, keeps the stored opacity,
and does not store a hidden topmost row that it swaps. The graph does not store the rest of a graphic style's look (more
fill and stroke rows, effects, the object's opacity and blend mode, and the link to the style), and a mark loses it when
the graph is drawn again. When the graph is drawn again, columns, bars, wedges, areas, markers and legend swatches take
the stored fill and stroke. A thin line takes the stored stroke and has no fill. A Filled Lines band takes the stored
fill, or the stored stroke when no fill is stored and that stroke has paint, and has a 0.25 pt black outline, so these
commands do not store a stroke paint or weight for a series that draws a band. Except for `paint.swap`, they still store
a stroke opacity set on a band, and the band's outline takes it. Graph designs keep their own paint. A row that had no
paint before the command and has none after it does not change what the graph stores. `paint.setFill`,
`paint.setStroke`, `paint.none` and the commands that apply one paint kind to the other store the topmost row of the kind
even when it is hidden, and store a stroke set on a band. `stroke.set`, `paint.editGradient` and the recolor commands do
not store a series' paint. An `appearance.*` command or `graphicStyle.apply` given a series group's id, and
`graphicStyle.redefine` on a series group linked to the style, edit the group's own rows, and the graph does not store
them.

`effect.apply {"effect": "path.outlineObject"}` (Effect › Path › Outline Object) takes no parameters. Type that carries
it is drawn, exported and expanded as its glyph outlines and stays editable type, and the effects after it reshape
those outlines. On paths, compound paths and images it does not change what is drawn.

Errors (unknown tool, bad arguments, a disabled or failing command) come back as a normal result with
`isError: true` and a message. The model can read the message and retry.

Swatches go through `run_command`: `swatch.list` lists every swatch with its colour group, kind and colour;
`swatch.edit {name, newName?, color?, mode?, global?, spot?}` edits one (fills, strokes and text linked to a
global or spot swatch follow its colour and name, as one undo step, in the art, opacity masks, symbols, graph
designs, pattern tiles, graphic styles and the art of brushes); `swatch.delete {names, unlink?}` deletes
swatches and colour groups in one undo step (art using a deleted global swatch keeps its colour, unlinked).
`swatch.new {color?, mode?, spot?, global?, group?}` saves a colour (into a colour group with `group`), gradient
or pattern (a gradient swatch holds no link to the swatch its gradient came from, and a pattern swatch holds the
pattern untransformed); `swatch.newGroup {fromArtwork: true, toGlobal?, includeTints?}` makes a colour group of the selected
art's colours (by default as global swatches the art links to).
`swatch.move {names, to?, group?}` reorders swatches or moves them into or out of a colour group (as dragging
them in the Swatches panel does); give colour group names instead to reorder the groups. Dropping a swatch on art
in the app runs `paint.setFill` (or `paint.setStroke`, the active proxy) with `ids: [the object under the pointer]`.
The Swatches panel menu's commands: `swatch.addUsedColors {selection?, global?}`, `swatch.unused` (a query: the
names Select All Unused selects), `swatch.merge {names}` (the first is kept), `swatch.ungroup {name}` and
`swatch.sortByKind`.
`brush.move {name | names, to?, type?}` reorders brushes among the brushes of their type, as dragging one onto
another of its type in the Brushes panel does. `to` is an index among the brushes of that type in `brush.list`
order: the brushes go before the first brush of that type at or after `to` that is not moving, and after the last
brush of that type when there is none or `to` is left out. A move that keeps the order records no undo step.
Brushes of different types, or a `type` other than theirs, are refused with "brushes move only within their type".
Document colour mode: `file.new {colorMode: "cmyk"}` starts a CMYK document with CMYK default swatches, and RGB
colors applied to it (`paint.setFill`/`setStroke` colors, gradient stops and freeform points, `paint.editGradient`
stops, `paint.freeform.addPoint`/`setPoint`, `paint.sampleColor`, `object.mesh.setPointColor`/`addLine`, the
`text.create` and `text.createInPath` color, `text.setStyle` and `text.setRangeStyle` fills and strokes, a
`shapeBuilder.merge` hex fill (its `fill: {color, keepModel: true}` form keeps RGB), the
`symbol.adjust` stain color, `swatch.new {color}` and `{colors}`, `swatch.newGroup`, `swatch.editGroup` and
`swatch.edit`) are stored as CMYK, converted through the color settings (`edit.colorSettings`: the CMYK
working space and intent, as Document Color Mode converts them);
Gray stays Gray, and `keepModel: true` keeps a color as given. A color that `swatch.edit` or `swatch.editGroup` is
given and the swatch already has keeps its model, as does a stop color that `paint.editGradient` is given and the
gradient already has; a stop that names a swatch takes the swatch's color as it is, and `swatch.new` or `swatch.edit`
with `mode` converts a color into that mode only. RGB documents keep colors as given. Harmonies, Edit
Colors blends, inversions and Recolor Artwork keep each colour's model (a blend between models takes the document's).
`swatch.new {colors: [...]}` saves several colours as swatches in one undo step. `color.harmony {color, rule, steps?, variation?, amount?}` answers what the Color
Guide panel shows: the harmony rule's colours, base first, and per colour its row of `2·steps+1` variations
(shades, cool or muted on the left, tints, warm or vivid on the right). With `limitTo` (a swatch library id or name,
or `"document"` for the document's swatches) every colour snaps to the library's nearest colour (ΔE 2000), as the
panel's Limit to Library does (`ui.colorGuideLimit`); Recolor Artwork's `limitTo` takes the same values. A `limitTo`
that is not a string, such as `5`, names no library and gets the same error as an unknown name; `null` or `""` sets no
limit.

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
with a warning. Type names its faces by numeric `font-weight` (600 for Semibold and for a Japanese face's W6, 300 for Light, `bold` for 700),
embedded or not. `svgz` takes the same options and writes the SVG gzipped (`.svgz` files also open,
place and paste). `run_command document.save {path: "x.svg", svg: {…}}` saves as SVG (or `.svgz`).
Colorized grayscale and 1-bit images export in their fill color; linked mode writes a coverage PNG, and Tiny 1.2 writes a colorized PNG.

Symbols export as one `<symbol>` with a `<use>` per instance. An instance the def can't stand for is written as its
own art: one stained by its fill; one scaled while the symbol has strokes (their weight doesn't scale), or rotated or
scaled while it has effects, brushes or live objects; and every instance of a symbol with pattern paints or unlinked
opacity masks (those stay on the page). `hiddenLayers: true` keeps hidden layers and objects, not displayed
(`display="none"`); `document.save` and the CLI's `convert` keep them unless told otherwise, other exports leave
them out, and they reopen hidden. An object's own data (`object.setProps {data: {pivot: "100,180"}}`) is written as
`data-*` attributes (`data-pivot="100,180"`), and an SVG's `data-*` attributes come in as it. Attributes in other applications' XML namespaces
(`plotter:pen="2"`) come in on their objects too and are written back, each namespace declared once on the root. A `preserveEditing` SVG carries the native document (CDATA in `<metadata>`) and a hash of the
markup around it: if another app changed the SVG since, `document.open` reads it as plain SVG and says so in
`warnings`.

Saving beyond `save_file` goes through `run_command`: `file.saveAs {path?, format?, options?, svg?}` (the document takes
on the new file and format; for `format: "ait"`, or a `.ait` path without `format`, it fails with an error that names
`file.saveAsTemplate`), `file.saveCopy` (the document keeps its path, title and modified state),
`file.saveAsTemplate {path?, format?}` (it writes a `.vctemplate`, or, for `format: "ait"` or a `.ait` path, a
PDF-compatible `.ait` written as a `.ai` file is, and with `aiVersion: "7"` it fails with an error; both open untitled;
a path whose extension names another format has
it replaced by the template's extension, and any other path that does not end in that extension has it added, so
`card.png` is saved as `card.vctemplate` and `card` as `card.vctemplate`; the suggested name and the Templates folder
come back without a path), `file.newFromTemplate {path}`, `file.revert` and `file.formatOptions {format?}` (a format's options
with the values a save would use). Without a path the save commands return `{dataBase64, name, folder?, warnings}`. A document saved as SVG or PDF
remembers its options, so the next save reuses them; a document not yet saved as a native file is offered the
VectorCraft Options the last native Save or Save As used (not its artboards, nor Use Compression). A PDF opened with `pages`, `cropTo` or `password` is only part
of the file, so Save asks for a new name instead of writing it back.
`file.documentColorMode {mode, convert?, intent?, grays?}` switches the document color mode, converting the colors of
the art (opacity masks, effect colors and the series paints and label styles that graphs keep included), symbols,
graph designs, pattern tiles (with the tile that pattern editing's Cancel puts back), graphic styles (their effect
colors included), brushes (their art and a Hue Shift key color) and swatches through the color settings, in one undo
step (`object.convertDocumentColorMode` is an alias kept for older scripts). It also converts the fill, stroke and
effect colors that new art takes; Undo restores their original values and Redo reapplies the conversion. The light
colors of 3D effects stay RGB.
Gray colors stay Gray and print
on the black plate only. RGB grays are colors like any
other: to CMYK they separate through the profile into light grays in cyan, magenta and yellow only, darker
four-color grays and a rich black. `grays: "black"` puts RGB greys with R = G = B on the black plate instead, K = their grey value (the inks Edit Colors ›
Convert to Grayscale before the switch would give); other colors convert as usual. The reply's `changed` counts the
colors that changed in the art, symbols, graph designs and pattern tiles; it does not count the colors of graphic
styles and brushes.
`edit.colorSettings`, `file.documentColorMode`, `color.convert`, `color.gamutCheck`, `color.loadProfile`,
`color.removeProfile`, `view.proofSetup`, `view.proofColors`, `view.overprintPreview`, `view.separationsPreview` and
`swatch.setSpot` reject a param of the wrong type, such as `"bpc": "no"`, `"on": 1`, `"to": 5` or `"colors": "#fff"`,
and change nothing. Omitting a param or giving null keeps its default.

SVG import keeps what the canvas edits live. `<pattern>` becomes a pattern swatch. A `<symbol>` with `<use>` becomes a
symbol (named after its `data-name`, else its id) with an instance per `<use>`; a `<use>` that shows it differently
(paint inherited from the `<use>`, a viewport that cuts the art) or that the canvas would draw otherwise (a scaled
symbol with strokes) stays art. `feGaussianBlur`, `feDropShadow` and the usual shadow chains (offset, blur, flood or
colour matrix, composite, merge) become Gaussian Blur, Drop Shadow, Outer Glow, Inner Glow and Feather effects; other
filters are listed in `warnings`. Undisplayed objects (`display:none`) come back hidden, nested clip paths intersect,
and reflected or repeated gradients (`spreadMethod`) are expanded into stops. `<image>` files (relative links are
found in the SVG's folder; only files on this computer or the SVG's network share are read, and pasted SVG reads only
absolute paths) stay linked images (see Linked images); SVG files and `data:` SVGs become art; a file that
can't be read becomes a placeholder in its box, named in `warnings` and `missingLinks`.

Native files: `run_command document.save {compress: true}` writes a gzip-compressed `.vectorcraft` (the
`useCompression` preference makes that the default for saves; compressed files open like any other, told apart by
their content). `version: 2` or `1` writes a file older VectorCraft versions open (format name `drawcraft`, never
compressed), and `preview: true` embeds a PNG of the first artboard, at most 256 px on its longer side.
`precision: 4` (0 to 9) rounds the anchors and handles of paths to that many decimal places (in the layers, symbols,
graph designs, patterns, opacity masks and brushes); colors, transforms, opacity and type are written exactly.
Each coordinate goes to the nearest step of its stored binary value, and a value exactly halfway rounds away from
zero (`0.125` at 2 places is `0.13`); `1.005` is stored as slightly less than that and is written as `1.0`. Each anchor is rounded on
its own, so a closed path stays closed and a shape smaller than one step keeps its anchors on one point. Opening the
file and saving it again does not bring back the dropped digits. The embedded preview and the PDF that `pdfCompatible: true` adds show the rounded paths. Without `precision` a save writes the coordinates exactly, and Save does not remember it. A `.ai` file ignores
`precision`. A save writes
a temporary file and renames it over the old one, so a failed save never damages the file; only the images the
document uses are written, and document keys written by a newer version are kept.
An uncompressed file is indented JSON when the document has at most 2,000 layers and objects and the indented
document, as the chosen version writes it, is at most 1 MiB; otherwise the file is compact JSON.

## Resources

Fixed resources and the four templates (same as [Resource templates](#resource-templates) under Protocol):

| URI | Content |
|---|---|
| `vectorcraft://document` | `document.inspect` summary (JSON) |
| `vectorcraft://document/json` | The complete document model (JSON) |
| `vectorcraft://commands` | Live command catalog with enabled state |
| `vectorcraft://object/{id}` | One layer or object (`document.node {id, summary: true}`) |
| `vectorcraft://command/{id}` | One command's metadata and enablement |
| `vectorcraft://effect/{id}` | One live effect with parameters and defaults |
| `vectorcraft://swatch/{name}` | One swatch (percent-encode spaces) |

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
SVG writes one file per artboard of a `range`); `useArtboards: false` exports the bounds of the visible art with what
its effects paint, the effects of groups, type and images included. `artboard.fitToArt` measures the art the same
way.

Raster exports default to the document's raster effects resolution. An explicit `scale` sets pixels per
point (72 times that value in ppi); `options.ppi` wins over either scale option. CLI `run --export` and
`convert` follow the same rule and accept `--ppi N` (which wins over `--scale N`). Documents at the
default 72 ppi still use 72 ppi; use `--ppi 300` for print output from those documents.

Raster exports take more options in `options`: `ppi` (72, 150, 300…; stored in the file, except in GIF, WebP and
Targa), `background` (`transparent`, `white`, `black` or `"#rrggbb"`; `#rrggbbaa` and `#rgba` are refused), `antiAlias` (`none`, `art`, `type`: text
snapped to pixels) and, for PNG, `interlaced`. `width` or `height` sets the size of each file in pixels: each artboard
(with `useArtboards`, each file), `rect` or art bounds gets the scale that makes it that wide or high, within 0.01–64
pixels per point, and the file stores the resolution that results, as with `ppi`. `width` wins over `height`, and
either one wins over `ppi` and `scale`. A value that is not a positive number is refused. Export for Screens rows keep
their own size (`scale: "512w"`).

Long-tail commands:

```json
{"name":"list_commands","arguments":{"filter":"align"}}
{"name":"run_command","arguments":{"command":"select.all"}}
{"name":"run_command","arguments":{"command":"object.align","params":{"horizontal":"left"}}}
{"name":"run_command","arguments":{"command":"object.group"}}
{"name":"run_command","arguments":{"command":"document.exportPdf","params":{"path":"/tmp/art.pdf","range":"1, 3","compatibility":"1.5"}}}
{"name":"run_command","arguments":{"command":"document.pdfSettings","params":{"marks":{"trim":true},"includeDocument":true}}}
```

PDF files take the Save PDF dialog's options: `preset`, `standard`, `compatibility`, the General toggles and the
`compression`, `marks`, `bleed`, `output`, `advanced` and `security` sections (`list_commands` with filter `exportPdf`
documents every field). `document.exportPdf` and `export` (format `pdf`, the options in `options`) return `warnings`:
options accepted but not applied yet, and features approximated or left out. An image whose pixel data can't be
decoded (a PNG cut short) is left out of the pages, and `warnings` holds the note "an image couldn't be decoded and
is missing from the picture" once, as raster exports give it; `.ai` saves and `file.print` do the same. A standard with a
PDF version it doesn't allow is refused (PDF/A-2b at 1.3 or 2.0, PDF/X-4 above 1.6, PDF/X-1a and PDF/X-3 above 1.4);
choosing a standard without giving `compatibility` sets its version (PDF 1.3 for PDF/X-1a and PDF/X-3).
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
A 16-bit PNG that is not resampled, recompressed or converted keeps its 16 bits a sample in PDF 1.5 and later files,
and is written at 8 bits in PDF 1.3 and 1.4 files.
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
A page with a side longer than 14,400 pt is written with a `UserUnit` (PDF 1.6): its longer side divided by 14,400,
rounded up to the next thousandth, with the page's sizes and coordinates in those units, so that no side is longer than
14,400 units (ISO 32000-1, Annex C.2). In PDF 1.3 to 1.5 files and PDF/X files, such a page is written in points.

An Illustrator EPS (version 9 on) or `.ai` file opens from the editing data it carries (after the EPS page; in the
`.ai` file's PDF private data): layers and sublayers (name, order, visibility, lock, printing, preview, dimming,
colour), groups as they were nested, compound paths, clipping groups, object names, hidden and locked objects, fills and
strokes (spot colours as spot swatches), linear and radial gradients, opacity, blend modes, isolation and knockout,
embedded images with their alpha channel, guides, and every artboard where it is. An object with several fills or
strokes, effects or a brush comes in as its drawn look (a group named after it). Type is made from the file's text
document, whole and editable: point type, area type (a story in several frames as threaded type) and type on a path,
with its fonts, size, leading, tracking, scaling, baseline shift, fill and stroke (grey, RGB or CMYK), opacity, blend mode, alignment,
indents and paragraph spacing. Where the text document can't say, type that shows is the page's, in its text object's
place. Where a layer's or group's drawn look draws a copy of art with type, the import puts the page's type that
lies over that copy's other art into the copy's group, unless the file is past the import's text-object limits.
A text object on a layer that shows and doesn't print keeps the type the text document makes for it, on that
layer, even where the page doesn't draw it. A file whose editing data can't be decompressed or has symbols,
pattern fills, placed files or anything else the reader doesn't read on a layer that shows, or whose layers look
different from its page, opens as before: an EPS as its printed page, a `.ai` file from its PDF content (where plain
groups open ungrouped), with a warning saying why. Layers look different when they draw an object the page doesn't (or
miss one it draws), and, for an EPS whose page is the box of its art, when they print art outside it. When the PDF
content of a `.ai` has a page for each artboard, the layers are compared with each page on its artboard, until the
pages compared take 8,388,608 pixels at the comparison's scale (about 67 letter-size pages, or 4 when the PDF content
is only images) or number 128; when it has pages for some of the artboards, or its pages are cropped to the art, they
are compared with the first page only. An artboard, or the page of an EPS's editing data, whose sides are within half
a point of its page's is compared on an area of the page's size. A file whose editing data is for a page of another
size opens the same way, and its warning gives both sizes in points, with the scale of the editing data's page to the
file's page (1:10 for a tenth of its size) when their sides are in proportion.
With `textAs: "outlines"` a `.ai` file whose type shows opens from its PDF content, which has the type's outlines. A PDF
saved with Illustrator's editing data holds it in `AIPDFPrivateData` streams and opens from it as a `.ai` file does.

The document then has what the file's page doesn't print: hidden objects and layers, layers that don't print, guides and
the art outside the artboards, as the app that wrote the file shows them. Exports and `artboard.fitToArt` leave out the
hidden art, the guides and template layers; an EPS's artboard is the file's artboard, not the page's bounding box. A
print pipeline that wants only what the page prints opens the file with `editingData: false` (default `true`):
an EPS as its page, a `.ai` file from its PDF content (a `.ai` saved without PDF compatibility then can't be opened).

```json
{"name":"run_command","arguments":{"command":"document.open","params":{"path":"/tmp/label.eps","editingData":false}}}
```

Opening a PDF (or `.ai`) imports every page as an artboard and layer; `document.open` takes `pages` ("2-3, 5", 1-based),
`cropTo` (`bounding` (the art's bounds), `art`, `crop` (default), `trim`, `bleed`, `media`: the box each artboard gets;
with `trim` the bleed box round it becomes the document's bleed, and a PDF-compatible `.ai` or a PDF saved with its editing data opens on its trim boxes for
`crop`, its crop box taking the bleed in) and `password` for an encrypted file. `document.pdfInfo` reads a file without opening it: the page count, each page's
size and boxes, `needsPassword`, and with `thumbnail: n` a PNG of page n. It reads a PostScript file (an EPS, a `.ps`
file, a `.ai` in PostScript form) as one page the size of its bounding box (a US Letter page without one), with an
empty `boxes`, and `thumbnail: 1` shows what that page prints. A page whose `UserUnit` (PDF 1.6) is above 1
opens at its size in points, the file's numbers times the `UserUnit` (one above 75,000 counts as 75,000), and
`document.pdfInfo` gives its size and boxes in points too. A `.ai` whose editing data has its artboards in the units of
such a page opens from that data at the page's size.
`document.open`, `file.place` and `document.pdfInfo` reject a `cropTo`, `crop`, `password`, `colorMode` or `textAs`
that is not a string, and `document.pdfInfo` rejects a `thumbnailSize` that is not a number. The error names the param,
and nothing opens or is placed. Leaving a param out or giving null keeps its default.
Imported colours keep their model:
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

PostScript files (`.eps`, `.epsf`, `.ps`, and `.ai` files saved in older formats or without PDF compatibility) open through the EPS
reader (see EPS and PostScript import). An `.ai` saved without PDF compatibility (its PDF part is only a placeholder
page) opens from its editing data alone: its type is made from the file's text document where it can be (point type,
area type and type on a path), and what can't be is left out with a warning. Non-native art (the content of a placed
PDF, which Illustrator shows but doesn't edit and keeps as a PDF inside the editing data) is drawn by the PDF importer,
as a group named "Non-native art" fitted to the object's box; one whose PDF can't be read is left out with a warning.
Without editing data it says it can't be opened. `pages` that name every page of a file saved without PDF compatibility also open it from its
editing data.
Without editing data it says it can't be opened. `pages` that name every page of such a file also open it from its
editing data. `file.place` reads it from its editing data too and places the art of the layers that show on artboard
`page`, clipped to that artboard unless `crop` is `bounding`. `clipboard.importPdf` takes every object of its editing

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
The restored document opens on the artboard its editing copy names as active. A file saved with `editingData: false` has
no editing copy and opens on the first artboard.
`document.export` / `serialize` to `.ai` (and the CLI's `run --export x.ai` and `convert in.svg out.ai`) write the same file
without giving the document a path. `options: {aiVersion: "7"}` writes legacy `.ai` files of version 7 instead (the
PostScript form, which keeps layers, groups, compound paths and clipping groups in apps that read it), one per artboard
of `range` (default all; `{name}-{artboard}.ai` when several, listed in the result's `files`), with
`transparentImages: "flatten"` (default) or `"omit"` for images with see-through pixels, opacity, a blending mode
or a colorize color (except a colorized image of black and white pixels, which is written as a 1-bit image mask in
its color with its white pixels clear),
`hiddenObjects: "layer"` (default: a hidden layer of each layer's hidden objects), `"omit"` or `"visible"`, and
`outlineText: true` to write type as outlines instead of text objects. Type with rotated characters (Character
Rotation) is always written as outlines, which keep the rotation; without `outlineText: true` this gives the warning "type with rotated characters is
written as outlines, which keep the rotation, and is no longer editable text (legacy .ai text can't rotate
characters)". An image that can't be decoded is left out of
the file and does not show in art that is flattened or whose raster effects are expanded; `warnings` carries "an image
couldn't be decoded and is missing from the picture" once for all such images. From the CLI: `vectorcraft-cli convert in.svg
out.ai --ai-version 7 [--hidden-objects layer|omit|visible] [--transparent-images flatten|omit] [--outline-text]
[--range R]`. `document.export`, `serialize`, `exportSelection` and `exportForScreens` refuse a `.ait` path or
`format: "ait"` with an error that names `file.saveAsTemplate`, which writes it. The CLI's `run --export x.ait` and
`convert in.svg out.ait` write it as `file.saveAsTemplate` does, with every artboard; `convert` refuses `--artboard`
and `--range` for a `.ait`.

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

Dither (the Gradient panel's box) draws a linear or radial gradient fill with fine noise that hides banding;
SVG and PDF exports write such a fill as an image at the Document Raster Effects resolution. It applies to fills
(`stroke: true` is refused):

```json
{"name":"run_command","arguments":{"command":"paint.editGradient","params":{"dither":true}}}
```

Method (the Gradient panel's menu): `classic` (the default) mixes the colours between stops in display RGB,
`perceptual` mixes a linear or radial gradient's colours in OKLab so the steps look even. SVG exports write a
Perceptual gradient as an image at the Document Raster Effects resolution (a dithered Perceptual fill with its noise).
PDF exports write it the same way, except for a linear or radial gradient with spot color stops whose other stops
are all paper white or written in CMYK or gray, that is not a dithered fill and whose curve does not have too many
stops. An export that converts colors to a CMYK destination profile writes RGB and Lab stops in CMYK, and so does a
CMYK document unless the export converts colors to an RGB destination profile. PDF writes tints of one spot ink as a Separation shading in that ink, and spot inks mixed with other spot, CMYK
or gray inks as a DeviceN shading.
`paint.setFill`/`paint.setStroke` take it in `gradient` too:

```json
{"name":"run_command","arguments":{"command":"paint.editGradient","params":{"method":"perceptual"}}}
```

Freeform gradients: `paint.editGradient {kind: "freeform"}` places four or more colour points inside each selected
object (coloured along the stops; `mode: "points"|"lines"` is the Draw toggle). `paint.freeform.get` lists the points
(document coordinates), lines and the selected point; `paint.freeform.addPoint {at, color?, opacity?, spread?,
line?}`, `setPoint {index?, …}`, `deletePoint {index?}`, `addLine {points}`, `splitLine {line, segment, t?}` and
`selectPoint {index|null}` edit them (`index` defaults to the selected point; each edit is one undo step). The color
of a point colored along the stops, of a point `splitLine` adds and of a point `addPoint` adds without `color` is a mix
of the colors around it (the two stops, the line's two points, or all the points) in the color model those colors
share, so CMYK stops and points give CMYK points. With the
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

`run` prints one JSON line per step (`open`, `cmd`, `export`) and exits non-zero on the first failure, naming the step
(counting from 0, as `"$N"` references do) and its command. `--params` applies to the `--cmd` just before it. `run`
also accepts the host commands `file.open`, `file.save`, `file.export`, `file.exportForScreens` and `tool.select`.
`run --in`, `convert` and `info` read every format `document.open` reads (`vectorcraft-cli --help` lists them).

### Steps from a file or standard input

A command line has a length limit (32,767 characters on Windows), which long parameters such as a placed file's
`dataBase64` or a long list of ids soon reach (#3147). `--commands FILE` reads the steps from a file instead, and
`--commands -` from standard input:

```sh
vectorcraft-cli run --in sheet.vectorcraft --commands steps.json --export out.pdf
```

```json
[
  {"command": "file.place", "params": {"name": "code.svg", "dataBase64": "PHN2ZyB4bWxucz0i…", "rect": [20, 40, 16, 57]}},
  {"command": "object.setProps", "params": {"id": "$0.ids.0", "name": "barcode"}}
]
```

The text is a JSON array of steps, or one step per line (JSON Lines; blank lines are skipped). A step is an object
with `command`, the command id (or `id`, as `command_batch` names it), and optional `params`, a JSON object; any
other key is refused. The steps run where `--commands` stands among the `--cmd` and `--export` options, in order,
and count with them for `"$N"`, so a file can refer to the ids an earlier `--cmd` made and a later `--cmd` to the
file's results. `--commands` may be given more than once (standard input once). The text is untrusted input: it
must be UTF-8 (a byte order mark is skipped), at most 64 MiB and 100,000 steps, and a malformed step stops the run
before anything runs, with the step's number and its entry (`entry 3`) or line (`line 3`) in the message.

### What a headless session remembers

`vectorcraft-cli run` and the headless `mcp` server keep two things between runs, in folders next to the
preferences (the folder the desktop app uses, so both see the same ones; `VECTORCRAFT_CONFIG_DIR` moves it):

- **Saved PDF presets** (`pdf.preset.save`, `.delete`, `.import`): one `.vcpdfpresets` file per preset in
  `PDF Presets`, written as the command runs. Presets saved earlier are there at the next start and work as
  `preset` in the PDF options.
- **ICC profiles** (`color.loadProfile`): a copy of the file goes into `Color Profiles` (the result says
  `remembered: true`), and every profile there is registered again at the next start, before the first
  command, so `edit.colorSettings {"cmyk": "…"}` or a preset's output intent can name it in a later run. The
  copy is what counts: the original may move or be deleted. `color.removeProfile {name}` forgets a profile and its
  copy (not while it is a working space).

Not remembered: Color Settings, the document, anything else. Nothing is read or written when
`VECTORCRAFT_NO_PREFS` is set, and a server confined with `--automation-read-root` or
`--automation-write-root` keeps nothing either (the folders are outside its roots). A file that is damaged, too large
(a profile over 32 MB, a presets file over 1 MB) or not a valid preset or profile is skipped and logged. When the
desktop app starts, the presets it kept in its preferences file before this folder existed are moved into it; after
that the folder is the one place they live, so a preset deleted by `vectorcraft-cli` is gone in the app too.

### Reproducible output

PNG, PDF, EPS, legacy `.ai`, packaged zips and saved `.vectorcraft` files record the time they were
made in (PDF `CreationDate` and `ModDate`, the PNG `Creation Time` entry, a zip entry's date, a
document's `created` and `modified`), so the same commands run twice gave different bytes though the
pixels matched (#2880). Setting **`SOURCE_DATE_EPOCH`** to a non-negative whole number of seconds
since the epoch records that instant instead of the clock, and the bytes repeat
(<https://reproducible-builds.org/specs/source-date-epoch/>):

```sh
SOURCE_DATE_EPOCH=1791208980 vectorcraft-cli run --cmd file.new --params '{}' --export out.pdf
```

A command that takes its own date (`file.new {created}`, a save's `modified`) still wins over the
variable. A blank, negative or non-numeric value is ignored and the clock decides, so an export is
never refused over the environment. SVG output carries no time and was already reproducible.

Two things are not pinned by the variable: the recovery heartbeat, which stays on the real clock,
and the random seed PDF encryption draws on, which mixes the OS random source with the clock. **A
PDF exported with a password at PDF 1.4 or later therefore still differs between runs** whatever
`SOURCE_DATE_EPOCH` says, because that seed supplies random bytes in its encryption dictionary and,
at PDF 1.6 and later, its AES initialization vectors. Encryption at PDF 1.3 (RC4 40-bit) draws
nothing from the seed, so that export repeats with the variable set. Encryption keeps the file `/ID`
the export already wrote, which also repeats with the variable set; the seed supplies an `/ID` only
for a file that has none. Reproducible output means an unencrypted export or a password-protected
PDF 1.3 export.

### Parameter keys

The engine checks the keys of `params` of every command it runs against the command's params description (the
`params` text `command_list` returns): `command_run`, each step of `command_batch` and of the `command.batch` and
`action.batch` commands, `run --cmd … --params`, the control channel's `engine.execute`, and the app's own calls. A
batch with such a step fails before its first step runs. The object the description
starts with, `{x, width?: pt, name?|title?: …}`, names the keys; `?` marks an optional one. A key it doesn't name is
refused before the command runs:

```json
{"id": "file.new", "params": {"widht": 300}}
→ invalid parameters for `file.new`: unknown parameter `widht`; did you mean `width`?
```

Only the top level is checked: the keys of an object inside a value (`artboardLayout: {…}`) are the command's to
read. The one exception is an effect's `params`: `effect.apply` (and `appearance.addEffect`) and `effect.setParams`
check them against the effect's own entry in `effect.list`, its `defaults` and the keys its `params` text names. A key
the effect doesn't take, or a value of another type than its default (a string for a number, a number for a switch),
is refused the same way:

```json
{"id": "effect.apply", "params": {"effect": "blur.gaussian", "params": {"blurr": 3}}}
→ invalid parameters for `effect.apply`: unknown parameter `blurr`; the command takes: radius
```

The `color` of Drop Shadow, Inner Glow, Outer Glow and Shift to Color must be one the effect can read: a hex color
(`#rrggbb`, `#rgb` or `#rrggbbaa`, with or without the `#`), `black`, `white`, `red`, `green` or `blue`, `[r, g, b]`
(three numbers, each read on a 0 to 1 scale), or a color object (`{"model": "rgb", "r": 1, "g": 0, "b": 0}`).
`effect.apply` and `effect.setParams` refuse any other value themselves (where the parameter check runs first, a number
or a boolean is refused as not a string), so the app's effect dialogs and Appearance panel, a step of
`command.batch` and a replayed journal get the same error, and the document and its undo history stay as they were:

```json
{"id": "effect.setParams", "params": {"index": 0, "params": {"color": "#ff00"}}}
→ invalid parameters for `effect.setParams`: `color` must be a #rrggbb color
```

A document saved with such a color opens as before, and the effect draws its default color (an array of three or more
entries reads an entry that isn't a number as 0).

The `channel` of Levels and Curves must be `rgb`, `red`, `green` or `blue` (`r`, `g`, `b` and any letter case are read
too). `effect.apply` and `effect.setParams` refuse any other value themselves in the same way, and a document saved with
one opens as before and draws it as `rgb`:

```json
{"id": "effect.setParams", "params": {"index": 0, "params": {"channel": "alpha"}}}
→ invalid parameters for `effect.setParams`: `channel` must be one of rgb, red, green, blue
```

A number outside the range the effect documents is still stored as given and clamped when the effect draws. A
plug-in effect's parameters are resolved against its manifest instead. `effect.setParams` as a step of
`command.batch` is checked when the step runs, against the effects the steps before it added, and the batch then
fails with no change. A description with a `…` entry takes more keys than it lists: those of the command it names
(`…settings as file.new`), or any key when it names none, as `document.export` and `document.save` do for the options
of the format they write (one object can carry several formats' options). A command that takes its own reply back
(`document.setup`, `text.areaOptions`) says so the same way.

Saved actions aren't checked this way. When the Actions panel plays an action (Play at any speed, an action's function
key, the Batch dialog or `ui.action.play`), a key a command no longer reads, recorded by an older version, is ignored
and the action still plays. `Session::replay` plays a journal the same way. The command still refuses an effect
`color` or `channel` it can't read.

An unknown command id is matched the same way as an unknown parameter key: the error names the nearest command id when
one is near, ignoring case. A step of `command.batch` or `action.batch` gets the same suggestion in its step error:

```json
{"id": "select.al"}
→ unknown command `select.al`; did you mean `select.all`?
```

These Object menu commands refuse a string or another value that is not a number with
`` `scaleH` must be a number, not "50" ``, naming the key, and change nothing: `scaleH`, `scaleV`, `moveH`, `moveV`
and `rotate` in `object.transformEach`; `ppi` in `object.rasterize`; `width`, `height`, `spacingX` and `spacingY` in
`object.createObjectMosaic`; `offset`, `length`, `bleed` and `weight` in `object.createTrimMarks`; and `spacing` in
`artboard.rearrange`. A null is read as a key left out. `object.cropImage` refuses a `rect` that is not an array of
four numbers, such as a string, with `rect must be [x, y, width, height]`; a null `rect` crops to the artboard, as a
missing one does.

`object.createObjectMosaic` (`columns`, `rows`), `object.transformEach` (`reference`, `seed`) and `artboard.rearrange`
(`columns`) take whole numbers, written `4` or `4.0`. A fraction or a negative number is refused with
`` `columns` must be a whole number ``. A string or another value that is not a number is refused with
`` `columns` must be a number, not "4" ``. Each message names the key that was given, and the command then changes
nothing.

These Object menu commands refuse a string, a number or another value that is not true or false with
`` `copy` must be true or false ``, naming the key, and change nothing: `reflectX`, `reflectY`, `random`, `copy`,
`relative`, `strokes`, `corners`, `objects` and `patterns` in `object.transformEach`; `gray` and `deleteRaster` in
`object.createObjectMosaic`; `trim` in `object.cropImage`; `allArtboards` in `object.createTrimMarks`; and `byColumn`,
`moveArtwork`, `patterns` and `lockedAndHidden` in `artboard.rearrange`. A null is read as a key left out. `object.createTrimMarks`
refuses a `style` that is not a string, such as `5`, with `` `style` must be roman or japanese ``, the message it gives
for a string that names no style.

A whole number that is larger than the build's `usize` can hold (4294967295 on a 32-bit build, such as the
`wasm32-unknown-unknown` web build) counts as larger than any index, place, byte offset or count. An index, such as
a `subpath`, `anchor`, `segment`, document `index`, `artboard`, appearance `item` or `index`, gradient `stop` or
freeform point `index`, names nothing, and the command does what it does for any other index past the end: most
commands fail and change nothing, and `appearance.removeItem` removes nothing. A place (`to` in `swatch.move`,
`artboard.reorder`, `graphicStyle.move`, `appearance.moveItem`, `appearance.duplicateItem` and `effect.move`) puts
the item last. A text `start` or `end` is past the end of the text. `object.slice.divide` refuses such a `rows` or
`columns`, `recolor.reduce` keeps each color in its own row for such a `colors`, and `layer.setProps` refuses such a
`color`. `object.setBounds` reads a `reference` past 8 as 8, as `object.transformEach` does.
The blend, envelope and gradient mesh commands refuse a string or another value that is not a number with the same
message, as in `` `steps` must be a number, not "8" ``, and change nothing: `steps`, `distance` and `value` in
`object.blend.make` and `object.blend.options`; `bend`, `h`, `v`, `horizontalDistortion` and `verticalDistortion` in
`object.envelope.makeWithWarp`, `resetWithWarp` and `options`; `rows`, `cols` and `columns` in
`object.envelope.makeWithMesh`, `resetWithMesh` and `object.mesh.create`; `fidelity` in `object.envelope.options`;
`highlight` in `object.mesh.create`; and `opacity` in `object.mesh.setPointColor`. A null is read as a key left out.
`object.mesh.create` refuses an `at` that is not two numbers with `` `at` must be [x, y] ``.

The transform, Align, Object Properties and Live Shape Properties commands refuse a string, a number or another value
that is not true or false with `` `strokes` must be true or false ``, naming the key, and leave the document and its
undo history as they were: the options `copy`, `strokes`, `corners`, `patterns`, `objects` and `typeAreas` in
`object.transform`, `object.move`, `object.nudge`, `object.rotate`, `object.scale`, `object.reflect`, `object.shear`,
`object.transformAgain` and `object.setBounds`; `anchors` and `releaseOutside` in `object.transform`; `big` in `object.nudge`;
`absolute` in `object.rotate`; `proportional` in `object.setBounds`; `patterns` in `object.align`, `object.distribute`
and `object.distributeSpacing`; `visible`, `locked`, `isolate` and `knockoutShape` in `object.setProps`; and
`invertPie`, `makeSidesEqual` and `relativeRadii` in `object.setLiveShape`. A null is read as a key left out.

They also refuse a string or another value that is not a number with `` `opacity` must be a number, not "50" ``,
naming the key, and leave the document and its undo history as they were: `opacity` in `object.setProps`; `radius` and
`sides` in `object.setLiveShape`; `reference` in `object.setBounds`; and `artboard` in `object.align`,
`object.distribute` and `object.distributeSpacing`. A null is read as a key left out.

The blend and envelope commands refuse a string, a number or another value that is not true or false with
`` `smooth` must be true or false ``, naming the key, and leave the document, its undo history and the options new
blends and envelopes start with as they were: `smooth` in `object.blend.make` and `object.blend.options`;
`independent` in `object.blend.spine.moveAnchor`; `perpendicular` in `object.blend.spine.reshapeSegment`; `horizontal`
in `object.envelope.makeWithWarp`, `resetWithWarp` and `options`, also when an `orientation` is given; `maintainShape`
in `object.envelope.resetWithMesh`; `antiAlias`, `distortAppearance`, `distortLinearGradients` and
`distortPatternFills` in `object.envelope.options`; and `editing` in `object.envelope.editContents`. A null is read as
a key left out.

`object.envelope.makeWithWarp`, `resetWithWarp` and `options` refuse a `style` that is not a string with
`` `style` must be a string `` and a name that is no warp style with `` unknown warp style `foo` ``, and an
`orientation` other than `horizontal` or `vertical`, a number or `true` included, with
`` `orientation` must be one of horizontal, vertical ``. `object.envelope.options` refuses a `preserveShape` that is
not a string with `` `preserveShape` must be a string ``, and checks the warp keys when no envelope or only mesh
envelopes are selected, too. Each leaves the document, its undo history and the options new envelopes start with as
they were. A null is read as a key left out.

These commands refuse a string, a number or another value that is not true or false with
`` `closed` must be true or false ``, naming the key, and leave the document and its undo history as they were:
`clockwise` in `shape.spiral`; `closed` in `shape.arc` and `path.create`; `vertical` and `placeholder` in `text.create`
and `text.createInPath`; `underline`, `strikethrough`, `allCaps`, `proportionalMetrics`, `tateChuYoko`, `standardVerticalRoman` and `noBreak`
in `text.setRangeStyle` and `text.setFormat`, which also checks `hyphenate`, `hyphenateCapitalized` and
`romanHangingPunctuation`; `effective` and `story` in `text.getRange`; `story` in `text.setRangeStyle`; `on` in `type.opticalMarginAlignment`, `type.systemLayout`
and `type.fractionalWidths`; `remove` and `overhang` in `type.ruby`; `expandInstances` in `symbol.delete`; `alt` in
`symbol.spray` and `symbol.adjust`; `spot` and `global` in `swatch.new` and `swatch.edit`; `unlink` in
`swatch.delete`; `fromArtwork`, `toGlobal` and `includeTints` in `swatch.newGroup`; `global` and `selection` in
`swatch.addUsedColors`; `visible` in `appearance.setItem`; `copy` in `appearance.transfer`; `reverse` and `append` in
`appearance.copyFrom`; `reverse` and `stroke` in `paint.editGradient`; `stroke` in `paint.setGradientGeom` and in
`paint.freeform.addPoint`, `setPoint`, `deletePoint`, `addLine`, `splitLine` and `get`; `erase`,
`openFilledAsClosed` and `splitStroke` in `shapeBuilder.merge`; `openFilledAsClosed` in `shapeBuilder.regions`; `closeGaps` in `livePaint.gapOptions`; and
`enhanced` in `imageTrace.make` and `imageTrace.makeAndExpand`. A null is read as a key left out.

These commands refuse a string or another value that is not a number with `` `radius` must be a number, not "2" ``,
naming the key, and change nothing: `radius` in `shape.rectangle`; `sides` and `rotation` in `shape.polygon`;
`radius2`, `points` and `rotation` in `shape.star`; `diameter`, `opacity`, `brightness`, `growth`, `fuzziness`,
`rays`, `longest`, `rayFuzziness`, `rotation`, `rings`, `largest`, `x2`, `y2`, `direction`, `pathLength` and `seed`
in `shape.flare`; `decay` and `segments` in `shape.spiral`; `rows` and `columns` in `shape.rectangularGrid`; `concentric`
and `radial` in `shape.polarGrid`; `size` and the `width`, `height` and `fitMinPercent` of `area` in `text.create`;
`size` and `fitMinPercent` in `text.createInPath`; `start`, `end`, `size`, `tracking`,
`baselineShift`, `hScale`, `vScale`, `rotation` and `strokeWidth` in `text.setRangeStyle`; `start` and `end` in
`text.editRange` and `text.getRange`; `at` in `text.startList`; `start`, `hScale`, `vScale` and `rotation` in `text.touchType`; `start` in
`text.glyphAlternates` and `text.setGlyphAlternate`; `by`, `start` and
`end` in `type.step`; `by` in `type.size.increase` and `type.size.decrease`; `size` in `type.ruby`; `width`, `height`
and `fitMinPercent` in `text.areaOptions`; `x` and `y` in `symbol.place`; `radius`, `density` and `scale` in
`symbol.spray`; `radius` and `intensity` in `symbol.adjust`; `to` in `swatch.move`; `weight` and `opacity` in
`appearance.setItem`; `r`, `g`, `b`, `c`, `m`, `y`, `k`, `gray` and `tint` in `edit.colors.adjustBalance`; `steps`
and `amount` in `color.harmony`; `angle` in `paint.editGradient` and in the `gradient` of a command that takes a paint; `t` in
`paint.freeform.splitLine`; `tolerance` and `gap` in `shapeBuilder.merge`; `gap` in `shapeBuilder.regions`; and
`width` and `tolerance` in `livePaint.strokeEdge`. A null is read as a key left out. A gradient stop that names a
swatch refuses a `tint` that is not a number with `` stop 0: `tint` must be a number ``.

A byte offset must be a whole number of at least 0. `at` in `text.startList`, and `start` and `end` in
`text.editRange`, `text.getRange` and `text.setRangeStyle`, and in `type.step` and `addon.type.textTweaker` given an `id`,
refuse a negative or fractional number with
`` `at` must be a whole number ``, naming the key, and change nothing. `2.0` is read as `2`, and a null as the key left out.

A step can use an earlier step's result, in `run` as in the `command_batch` tool. A parameter string that is exactly
`$N` or `$N.path` takes that value from the result of step `N` (counting from 0, in the order they ran; `run --in`'s
open isn't a step), and `$last…` from the step before. A path goes into objects by key and arrays by index:
`"$1.id"`, `"$2.ids.0"`, `"$last.ids[0]"`. A string starting with `$$` stands for one `$` (`"$$1.99"` is `$1.99`). A
reference that names no value fails the step, so a command never runs with one left in it:

```sh
vectorcraft-cli run --cmd file.new --params '{"width":600,"height":200}'   --cmd text.create --params '{"x":10,"y":60,"text":"plain and bold words","size":30}'   --cmd text.setRangeStyle --params '{"id":"$1.id","start":10,"end":14,"style":"Bold"}'
```

## Checking scripts written for the reference app

`script.check {path? | data? | dataBase64?, name?, includes?}` reads a script written for the reference app
(`.jsx`, `.jsxinc`, `.js`, or a compiled `.jsxbin`) and reports what it uses. The check does not run any of the
script, and it does not change the document or the preferences. The reply's `kind` is `script`, `compiled` (a
`.jsxbin` file, or text that starts with `@JSXBIN@`, which the check doesn't read), `module` (a file with a
top-level `import` or `export`, such as a React `.jsx` file), `notText` (a NUL byte, or more than one byte in ten a
control character, in the first 8 KiB) or `tooLarge` (over 8 MiB). For a script, the reply holds these lists:

- `needs` lists what the script needs from the app that runs it: `language` (the script language's own objects
  and directives, such as `$`, `alert`, `UnitValue` and `#target`), `dom` (the reference app's document model),
  `files`, `menuCommands`, `dialogs`, and `hostOnly` (features only the reference app has: palettes,
  `#targetengine`, BridgeTalk, sockets, native code, XMP, the XML object and E4X, operator overloading,
  reflection, actions and `selectTool`).
- `directives` lists the `#include`, `#includepath`, `#script`, `#strict`, `#target` and `#targetengine` lines,
  and `targets` splits each `#target` line's application specifier into `app`, `instance`, `version` and
  `locale`. A directive is read where it starts a line, written `#name` or `//@name`.
- `features` lists the platform features the script uses by id (such as `file.write`, `scriptui.dialog`,
  `menuCommand` or `e4x`), each with a count and its lines.
- `dom.members` lists the document-model member chains as the script writes them
  (`app.activeDocument.layers[].name`, with `[]` for an index and `()` for a call); a variable assigned from a chain
  is replaced by that chain, and a name written with `\u` escapes is read as the name it spells. `dom.types` lists
  the type names the script compares with `.typename` or creates with `new`, apart from the language's, the platform's
  and the script's own functions.
- `menuCommands.strings` lists the string literals the script passes to `executeMenuCommand`
  (`app.executeMenuCommand('…')`), each with a count and its lines, and `menuCommands.dynamic` counts the calls
  whose argument isn't a string literal.
- `outside` lists what the script does outside the document: `filesWritten` (with the open modes used),
  `filesOpened` (files opened in other apps), `network`, `nativeCode`, `otherApps` (messages to other apps),
  `environment` (environment variables), `unseenCode` (`eval`, `new Function` and `$.evalFile`) and
  `preferences` (the reference app's preferences changed).
- `problems` lists what would stop the script loading as written (strings, comments, regular expressions,
  template literals and XML literals that aren't closed, brackets that don't match, syntax newer than ES3 and
  unknown directives) and notes on its text (bytes that aren't valid UTF-8, mixed line endings, and nesting deeper
  than the check follows: more than 4,096 open brackets, or XML literals and the code in their braces nested more
  than 4,096 levels deep).

Each entry of `targets`, `directives`, `includes` and `problems` has the `line` it is on (a problem with the whole
text has none) and, when it is in an included file, that file's path as `file`. `features`, `menuCommands.strings`
and `outside` count the script and its included files together and list up to 20 of the script's own lines.

With a path and `includes: true`, the check also reads the files the `#include` lines name, and the files their own
`#include` lines name. When the script or one of these files assigns a member chain or a File object to a variable,
the check treats the variable as holding it in every file. The check looks for a name in the folder of the file that
holds the `#include` line, then in each
folder that the last `#includepath` line before it names, taken from that same folder. It reads only files inside
the script's folder: it doesn't follow an absolute name, a name, folder or link that leads out of the script's
folder, a name or folder longer than 4,096 bytes, a file outside the automation read root, or the `JSINCLUDE`
environment variable, and it reads each file once. An include's `status` is `checked`, `missing`, `notFollowed`,
`cycle` (the file it names includes the file the line is in, directly or through other files), `notRead` (includes
weren't read: no `includes`, no path, or the web app), `limit` (one of the limits below was reached) or `unreadable`
(a compiled file, or not text), and its `path` is the file found, relative to the script's folder. The check reads
at most 8 MiB of a file and 32 MiB in all, follows 64 included files to a depth of 16 and looks up at most 1,024
paths for them, keeps 1,000 entries in each list, follows at most 1,000 variables that hold member chains and 1,000
that hold File objects (`truncated` is true when an entry or a variable was left out), and cuts the text it echoes
from the script to 120 characters.

File › Scripts › Check Script… picks a `.jsx`, `.jsxinc` or `.jsxbin` file and shows its report, and File › Open and
drag-and-drop show the report for those files too; the web app checks the script without its included files. Over
the control channel, `ui.checkScript {path? | data? | dataBase64?, name?}` does the same, and with a path it checks
the included files too.

```json
{"name":"run_command","arguments":{"command":"script.check","params":{"path":"/Users/me/Scripts/Tidy Layers.jsx","includes":true}}}
```

```sh
vectorcraft-cli run --cmd script.check --params '{"path": "Tidy Layers.jsx"}'
```

## Transparency and opacity masks

Transparency and opacity-mask commands take `ids` (or `id`), so they need no selection. Opacity is a percentage
(0..100) in every command (`transparency.set`, `object.setProps`, `appearance.setItem`). `transparency.info` returns
the Transparency panel's values, with `null` where the objects differ. Making a mask from one object gives it an
empty mask and enters mask editing: art drawn then becomes the mask, and the mask commands act on the masked object
until `transparency.stopEditingOpacityMask`. While a mask is edited, `object.exitIsolation` (Esc, the isolation bar's
back arrow, Exit Isolation Mode) leaves mask editing the same way, in one undo step. Saves and exports never include
the editing layer. With gradient mesh points picked, `opacity` goes to those points (see Picking several mesh points).

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
The command rejects an `on` value that is not a boolean. Omitting `on` toggles the current value.

Isolation mode and mask editing exclude each other. In isolation mode `transparency.editOpacityMask` and
`transparency.viewOpacityMask` are off, and `transparency.makeOpacityMask` refuses one target, an object or a layer,
which it would take into mask editing; a call returns an error that ends "exit isolation mode first". While a mask is edited,
`object.isolate` and `perspective.editText` are off, and a call returns an error that ends "stop editing the opacity
mask first". Pattern editing mode does not count as isolation mode here.

`view.transparencyGrid {on?}` (View → Show Transparency Grid; default: toggle) is view state of each open document,
like the mask view: the grid shows behind the artboards of the documents that turn it on, on the canvas and in
`ui.screenshot`, never in exports. It returns `{on}`, isn't undoable and doesn't mark the document changed.
The command rejects an `on` value that is not a boolean. Omitting `on` toggles the current value.

`view.globalRulers {global?}` (View → Rulers → Change to Global / Artboard Rulers, Cmd+Alt+R; default: toggle) is
saved with the document and undoable. With artboard rulers (a new document's) the rulers count from the active
artboard's top-left corner, or the zero point set for it; with global rulers, from one document zero point. Command
coordinates stay absolute either way. `document.rulerOrigin {artboard?}` returns the zero point the rulers use with that
artboard active; `document.setRulerOrigin {x, y, artboard?}` and `document.resetRulerOrigin {artboard?}` set or reset
the document's (global rulers) or that artboard's (artboard rulers, kept relative to it so it follows the artboard).

```json
{"name":"run_command","arguments":{"command":"view.globalRulers","params":{"global":true}}}
```

`view.videoRulers {}` (View → Rulers → Show / Hide Video Rulers) is a UI view flag, not saved with the document and not
undoable. Show Video Rulers shows the rulers and has them count the active artboard in video pixels from its top-left
corner, whatever the ruler origin: a pixel is as many points wide as the artboard's `videoRulerPixelAspectRatio`
(`artboard.setProps`) and one point tall, so a 720 × 480 pt artboard at 0.9 reads 800 × 480. Hide Video Rulers puts the
rulers back in the General unit from the ruler origin; they stay shown. It returns whether video rulers show.

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

`select.key {id}` makes `id` the key object that Align aligns to and Distribute Spacing spaces from (`object.align` without `to`, `object.distributeSpacing` with `spacing`), and `select.key {}` cancels it.
The id must name a selected top-level object.
Layers, compound-path members and objects inside another selected object are not top-level objects.
Any other id is an error naming that value, and the key is left unchanged.
A null `id` is an error too.
A click with the Selection tool on a selected top-level object, while two or more objects are selected or ruler guides are selected with it, runs `select.key {id}`, and a click on the key object again runs `select.key {}`.
On a single selected object with no ruler guide selected, or on a group selected with only its own members, `select.key` sets no key.
With a key object (or a key guide), `object.align` moves the other selected ruler guides too: a vertical guide goes to the key's left edge, centre or right edge, a horizontal one to its top, centre or bottom (#2143).
Aligning to the selection or the artboard leaves guides where they are.
`select.key {guide}` makes a selected ruler guide (its `guide.list` index) the key instead, while one or more objects are selected (#2143).
`object.align` then lines the objects up on the guide and the guide stays put: a vertical guide sets where their left edges, centres or right edges go and vertical alignment leaves them where they are, a horizontal guide the other way round.
Selected anchor points align to it the same way. `object.distribute {spacing}` and `object.distributeSpacing {spacing}` space the objects from it (see [Align to Artboard](#align-to-artboard)).
A click with the Selection tool on a guide selected with objects runs `select.key {guide}`, and a click on the key guide again runs `select.key {}`.
The key guide goes when it leaves the selection or no object is left selected, and an object key replaces it (and the other way round).
It returns `{key}`, the key object's id or null, with `guide`, the key guide's index, when a guide is the key.
`document.inspect` (the `inspect_document` tool) reports the key object as `key`, null without one, and the key guide as `keyGuide`.
The `ui.pointer` and `pointer_gesture` replies list it as `key` beside `selection`.
The command palette lists `select.key` as Cancel Key Object and runs it without an id.

`edit.clear {ids?}` deletes the exact selected objects, including individual compound-path members;
their unselected siblings remain in the compound. Supplying `ids` works without a selection and
overrides selected anchors or guides. Every explicit id is validated before any deletion; a malformed
or unknown id leaves the document unchanged. An empty `ids` array does nothing. Layers themselves
are kept, and selecting an ancestor together with its descendants deletes that subtree once.
Without `ids`, direct-selected anchors or selected ruler guides retain their usual Clear behavior.
Cut still removes the objects it copied, including a whole compound when a member is selected.
With `ids`, `guides: true` also deletes the selected ruler guides. The command rejects a `guides` value that is not a
boolean.

`object.setProps` with `visible: false` or `locked: true` deselects every selected object that is hidden or
locked after the call: the selected objects it hides or locks, selected objects inside them, and selected objects
that were already hidden or locked. The key object is cleared when it is one of them, and `edit.undo` selects
them again with the key object. When the call changes nothing, it records no undo step and still deselects them.
Other `object.setProps` calls leave the selection unchanged.

`object.unlockAll` and `object.showAll` unlock or show every locked or hidden object in the document and select
them → `{count}`. In isolation mode they take only the objects inside the isolated object. With `scope: "group"` they
take only the objects inside the groups that hold the selected objects:
for each selected object, the group nearest above it, or the object itself when it is a group. The objects of a
group inside another such group are counted once. With no selected object in a group, the call returns the error
"nothing selected is in a group" and changes nothing. `scope: "document"` is the default, and any other value is
refused. Object › Unlock All and Object › Show All clicked in the in-window menu bar with Shift+Alt (Shift+Option)
held run with `scope: "group"`.

`layer.move`, `node.move`, `layer.merge`, `layer.flatten`, `layer.collectInNew`, `dataset.select`, `dataset.next`
and `dataset.prev` also deselect every selected object that is hidden or locked after the call, such as objects
moved into a hidden or locked layer or group, or objects that a data set hides. The key object is cleared when it
is one of them, and `edit.undo` selects them again with the key object.

`object.hide.unselected` (Object › Hide › Unselected Artwork, Cmd+Alt+Shift+3) hides, in one undo step, every
object that `select.all` takes and that is neither selected nor holds a selected object, and returns `{count}`. In a
group that holds a selected object it hides the group's other members one by one; a clip group, a compound path or
another object that holds a selected object stays as it is with its members. The objects of a selected layer count as
selected. While Lock Guides is on, the guide paths count as selected and stay as they are. The selection stays as it is, and `object.showAll` shows the objects again. With nothing to
hide it records no undo step. In isolation mode it hides only unselected objects inside the isolated object.

`object.lock.unselected` (Cmd+Alt+Shift+2; shortcut and command, with no Object › Lock menu item) locks unselected
artwork in one undo step and returns `{count}`. In a partly selected group, clipping group, or compound path, it locks
the unselected members and leaves the container and selected members unlocked. It includes unselected guides even
while Lock Guides is on. Hidden art and art on locked layers stay individually unlocked; a selected layer's artwork
counts as selected. In ordinary isolation mode it locks only unselected art inside the isolated object. The selection
stays as it is. `object.unlockAll` unlocks and selects all locked artwork in the document, or inside the isolated
object in isolation mode, including art locked before this command. With nothing to lock it records no undo step.

These Select menu commands select only the objects that `select.all` selects and the objects inside them: every
`select.same.*` and `select.object.*` command except `select.object.slices`, `select.nextAbove`, `select.nextBelow`
and `select.recall`. They leave out hidden and locked objects with the objects inside them, the objects on hidden,
locked or template layers and sublayers and, in isolation mode, everything outside the isolated group or layer. When
the isolated group or layer, or a group or layer it is in, is hidden, locked or a template layer, they also leave out
everything in it, and `select.all`, `select.allOnArtboard` and `select.inverse` select nothing.
`select.object.bristleBrushStrokes` selects the objects whose topmost stroke has a bristle brush (type `bristle` in
`brush.list`), whatever the brush is named; a brush of another type with "Bristle" in its name does not count.
`select.nextAbove` and `select.nextBelow` select the nearest such object above or below the first selected object
among the objects beside it in its group or layer, passing over hidden and locked objects and sublayers. When there
is no such object, the selection stays as it is.

`select.same.textFillAndStrokeColor` (Select › Same › Text Fill & Stroke Color) selects the type objects that have
characters whose fill and stroke both match the first character of the first selected type, and returns `{count}`.
`select.same.textFillColor` and `select.same.textStrokeColor` compare the fill or the stroke alone. In type with
several runs, one run has to match both: type with the fill in one run and the stroke in another is left out. The fill
and stroke of other objects are not compared. Without type among the selected objects it returns the error "select a
text object", and it records no undo step.

The font and text color commands (`select.same.fontFamily`, `fontFamilyStyle`, `fontFamilyStyleSize`, `fontSize`,
`textFillColor`, `textStrokeColor` and `textFillAndStrokeColor`) take the first character of the first type among the
selected objects and the objects inside them, in selection order, as the one to match. Type that is hidden or locked,
inside a hidden or locked group or layer, on a template layer, or outside the isolated group is passed over. When the
selection holds no other type, they return the error "select a text object".

`select.same.fillColor`, `strokeColor`, `fillAndStroke`, `strokeWeight`, `opacity`, `blendingMode`, `appearance`,
`shapeType`, `graphicStyle`, `appearanceAttribute` and `symbolInstance` compare the other objects with the first
selected object, in selection order, that can be selected. That object can be of any kind: a group, a member of a
group selected on its own, or, for `symbolInstance`, an object that is not a symbol instance, which matches nothing and
leaves the selection empty. A selected object that is hidden or locked, inside a hidden or locked group or layer, on a
template layer, on a locked artboard while nothing is isolated, or outside the isolated group is passed over. New art,
such as a symbol instance just placed, is not passed over for being on a locked artboard while it stays selected. When
the selection holds no other object, they return the error "nothing selected".

`select.object.allOnSameLayers` selects the objects on the layers and sublayers of the selected objects that can be
selected. A selected object that is hidden or locked, inside a hidden or locked group or layer, on a template layer, on
a locked artboard while nothing is isolated, or outside the isolated group adds no layer. When the selection holds no
other object, the command clears the selection.

## Shaper Groups

`shaper.releaseSource {source, group?}` takes the original shape `source` out of its Shaper Group: the shape becomes a
path just above the group with its own fill and stroke, and is selected. The group's result is computed again from the
original shapes left, and a group left with none is removed. When `group` is given, it must be the Shaper Group that
holds `source`. A source that is not an original shape of an editable Shaper Group is an error, and the document and its
undo history stay as they were. The command returns `{id}`, the shape's id, and records one undo step, Remove from Shaper
Group. Construction mode ends when the group was in it.

`object.transform` with `releaseOutside: true` does the same for each moved original shape whose bounds end wholly
outside the bounds the group's original shapes had before the move, in the move's undo step. A shape whose bounds still
overlap them only moves. The Selection tool sends it at the end of a move in Construction mode.

## Intertwine

`object.intertwine.arrange {id?, at?|area?, order?, member?}` changes the order the objects of an Intertwine group are
drawn in, in the overlap areas under `at` or enclosed by the polygon `area`. `order` is `toggle` (the default: the
object right below the top comes to the front), `front`, `forward`, `backward` or `back`. `forward` and `backward` move
`member` one place up or down among the objects that overlap in that area, and `front` and `back` move it to the top or
the bottom there; the other objects keep their order. Without `member`, `front` and `forward` move the object right
below the top, and `back` and `backward` move the top one. The command returns `{count, top}`: the number of areas
whose order changed and the id of the object now on top in each. A move that changes no area returns
`{count: 0, top: []}` and records no undo step. With the Intertwine tool, a right-click where three or more objects
overlap offers Bring to Front, Bring Forward, Send Backward and Send to Back, which run this command for the object
nearest to the pointer.

## Global Edit

Select › Start Global Edit is `select.globalEdit {appearance?, size?, artboards?, includeCanvas?}`. With one path,
compound path or symbol instance selected, it finds the objects of the same shape (moved or scaled copies; not rotated
ones), among those `select.all` would take and the objects in their groups, and returns `{active: true, count, ids}`,
the selected object first (at most 10,000). With a group of such objects selected (nested groups and clipping paths
included; not type or images), it finds the groups like it: as many objects, in the same order, each of the same
shape, lying the same way relative to each other (the group moved or scaled as a whole). `appearance` also asks for
the same fill and stroke (for groups, of each object in them), `size` for the same width and height; `artboards` is
`"all"` or a range such as `"1, 3-5"`, and `includeCanvas: false` leaves out the objects on no artboard. Missing
params come from `select.globalEditOptions` (the Global Edit Options, kept for the session; it returns them with
`count`, the objects a start would take now). The selection stays the one object. Until the next `select.globalEdit`
(Stop Global Edit, `{active: false}`), what an edit of a selected match changed is given to the other matches, in the
same undo step, and nothing else. A changed outline or anchors go to each match in proportion to its size. A changed
opacity or blending, and each changed setting of a fill or stroke (paint, weight, opacity, blending, effects…), are
copied as they are to the match's fill or stroke of the same kind and rank; its other fills and strokes, and their
unchanged settings, stay its own (a stroke weight change leaves the fills). A stroke weight set on the selected match
is given as it is, and resizing the outline in proportion never scales a match's stroke weight. Hidden or locked
matches are left as they are. A fill or stroke added or removed is added or removed on the matches whose fills and
strokes lay in the same order; the others, and any other change to the list, take the whole list. In groups, an edit
made to an object inside one of them (selected on its own) is given by the same rules to the object at its place in
each of the others, whose hidden or locked objects are left as they are; when the group gains or loses objects, each
other group takes its objects at that level, keeping its own ids in order. Whenever the document is put back (Undo,
Redo, a cancelled drag or scrub), the matches it no longer has leave the Global Edit, which ends when fewer than two
are left. An edit that would take any match past the canvas fails with the coordinates-out-of-range error and changes
nothing. Nothing of it is saved.

`select.globalEditExclude {id}` leaves the match `id` out of the Global Edit, as a Shift-click on a highlighted match
(or on an object inside it) with the Selection tool does: it is no longer highlighted, and later edits leave it as it
is. It returns `{active, count, ids}`, the objects left in the Global Edit; with fewer than two left, the Global Edit
ends (`{active: false, count: 0, ids: []}`). The object the Global Edit started from can't be left out, and neither
can an object that is not a match. It is not an undo step. A Shift-drag that starts on a match selects it and moves
it with the selection, and leaves it in the Global Edit. Source: [Update assets quickly with Global
Edit](https://helpx.adobe.com/ph_fil/illustrator/how-to/global-editing-sensei.html).

## Saved selections

Select → Save Selection… keeps the selected objects under a name, in the document: `select.save {name?}` (default
the first free "Selection N"; an existing name is replaced with the current selection; at most 25 per document,
names up to 255 characters) → `{name}`, one undo step. `select.savedList` lists the names in menu order, and
`select.recall {name}` selects those objects again, except the ones deleted since and the ones hidden, locked or
outside the isolated group or layer (see Object selection) → `{count}`. Update Selection: `select.update {}`
replaces the objects of the saved selection last recalled in the document with the selected objects → `{name}`,
one undo step. The command is enabled while objects are selected and the document holds that saved selection,
under its own name or a name `select.editSaved` gave it; Undo and Redo do not change which saved selection is
recalled. Edit Selection…
renames and deletes: `select.editSaved {name, newName?, delete?}`, or several at once as
`{edits: [{name, newName?, delete?}…]}` in one undo step, where every `name` is the name before the edit and the
names must stay unique. In the desktop app the saved selections are listed at the bottom of the Select menu
(`select.recall1` … `select.recall25`) and in the Layers panel menu's Save Selection submenu
(`select.recall {name}`). A file's saved selections are checked when it opens: past 25, blank or
repeated names, and ids the document doesn't have are dropped.

## Saved views

View → New View… keeps a view under a name, in the document: `view.saved.new {name?, center: [x, y], zoom,
rotation?}` (the desktop app fills in its current view; default the first free "View N"; at most 25 per document,
names up to 255 characters, each name once) → `{name, index}`, one undo step. `view.saved.list` →
`{views: [{name, center, zoom, rotation}…]}` in menu order. Edit Views… renames and deletes:
`view.saved.edit {name, newName?, delete?}`, or several at once as `{edits: [{name, newName?, delete?}…]}` in one
undo step (→ `{views}`), where every `name` is the name before the edit and the names must stay unique; it is
enabled while the document has a saved view. In the desktop app the views are listed at the bottom of the View menu
(`view.goto1` … `view.goto25`); choosing one sets the current window's zoom, centre and rotation. A file's views are
checked when it opens: past 25, blank or repeated names, and views without a finite centre and a positive zoom are
dropped.

## Navigator panel options

The Navigator panel menu's Panel Options… opens the `navigatorPanelOptions` dialog (fields `color`, `solidDashes`),
and `ui.navigatorPanelOptions {color?, solidDashes?}` sets the options without it → `{color, solidDashes}`; with
neither key it opens the dialog. `color` is the color the panel's view box is drawn in: `#rrggbb` or a preset name.
The presets are VectorCraft's own list: Light Red (`#ff3030`, the default), then the layer colors ("Light Blue",
"Red", "Green" and the rest). `solidDashes` (Draw Dashed Lines as Solid Lines, off by default) makes the panel's
thumbnail draw dashed strokes without their dashes; the canvas, printing and exports keep the dashes. Both are kept
with the UI state. A `color` that is not a color, or a `solidDashes` that is not true or false, is an error and
changes neither option. Behavior reference: [View artwork](https://helpx.adobe.com/illustrator/using/viewing-artwork.html).

```json
{"name":"invoke_menu","arguments":{"command":"ui.navigatorPanelOptions","params":{"color":"Light Blue","solidDashes":true}}}
```

## Tools panel placement

By default the docked Tools panel sits at the window's left edge, outside any panels docked on the left.
`window.toolsBesideCanvas {on?}` (Window › Toolbars › Keep Tools Beside the Canvas, remote mode, `invoke_menu`)
docks it between those panels and the canvas instead (`on: true`), back at the window's edge (`on: false`), or
toggles it (`on` omitted or null), and answers the new state. Any other value of `on` is an error and changes
nothing. With the setting on, a floating Tools panel docks when it is dropped on the canvas's left edge as well as
on the window's left edge, and `window.panel.dock {panel: "tools"}` docks it beside the canvas. A floating Tools
panel stays floating when the setting changes. The setting is VectorCraft's own layout option, off by default; it is
kept with the workspace, and built-in workspaces and workspaces saved without it keep Tools at the window's edge.

```json
{"name":"invoke_menu","arguments":{"command":"window.toolsBesideCanvas","params":{"on":true}}}
```

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
View › Guides › Lock Guides (`view.guides.lock`) deselects them and the paths made into guides, in every open
document, and refuses these commands until unlocked. While it is on, a command that selects art, such as `edit.undo`,
`edit.redo`, `object.exitIsolation`, `object.unlockAll`, `object.showAll` or `edit.paste`, leaves the guides out.
`view.guides.lock` rejects a `locked` value that is not a boolean. Omitting `locked` toggles the current value.

`view.guides.release {index?, ids?}` (View › Guides › Release Guides) turns guides into paths → `{count}`. With
`index` it releases that ruler guide, and with `ids` it releases those guide paths (and the guide paths inside them).
Otherwise it releases the selected guide paths and ruler guides, and with no guide selected it releases every guide
path. A released ruler guide becomes a straight path with no fill or stroke, at the top of the guide's layer (or where
new art goes, for a guide on no layer). An artboard guide becomes a line across its artboard, and a canvas guide
becomes a line across the bounds of all the artboards. `index` and `ids` are refused while guides are locked. The
release is one undo step, and the released paths are selected. With the Selection, Direct Selection or Group
Selection tool, a double-click with Cmd/Ctrl and Shift held releases the ruler guide or guide path under the pointer
(`pointer_gesture` with a `doubleclick` event and `mods: {cmd: true, shift: true}`), unless guides are hidden or
locked.

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

## Symbols, brushes, variables and Image Trace

These families are in the live catalog (`command_list` / `vectorcraft://commands`); the prompt `trace-and-style`
already walks Image Trace. Short agent entry points:

- Symbols: `symbol.list` (each row includes `registration` 0..8, `exportType`: `movieClip`|`graphic` and `type`:
  `static`|`dynamic`),
  `symbol.new {name?, registration?, exportType?, nineSlice?, nineSliceGuides?, type?, ids?}`,
  `symbol.options {name?, newName?, registration?, exportType?, nineSlice?, nineSliceGuides?, type?}` (`nineSlice` bool turns
  9-slice guides on or off; `nineSliceGuides` is `[left, top, right, bottom]`, fractions 0..1 of the symbol size, a
  quarter each when none are given; rows of `symbol.list` carry `nineSlice`: the guides or `null`; a scaled instance of
  a symbol with guides keeps its corners at their size, and `symbol.breakLink` gives its art as it draws; dragging a
  guide on the canvas in symbol editing mode sends `nineSliceGuides` in one undo step; `type` is
  Symbol Type, `static` by default for the command and `dynamic` by default in the Symbol Options dialog),
  `symbol.place {name?, x?, y?}`, `symbol.breakLink {ids?}` (the art keeps the instance's opacity mask), `symbol.replace`,
  `symbol.edit {id?, ids?, name?, x?, y?, artboard?, viewCenter?}` (symbol editing mode: the symbol's art on a temporary
  isolated layer, where the instance `id` shows it or centered on (x, y), by default on the center of artboard
  `artboard`, the active one the app passes, or in a document without artboards on `viewCenter`, the view's center the
  app passes; every instance follows each edit and stays linked; while another symbol is edited, that edit ends first,
  and a call that fails leaves it open), `symbol.exitEditing`
  (leaves the mode; without an edit it adds no undo step), `symbol.duplicate`,
  `symbol.selectInstances {name}`, `symbol.delete {name | names, expandInstances?}` (`names` deletes several in one
  undo step). `symbol.new` and `symbol.duplicate {name, newName?}` trim the white space around the
  name they are given, as `symbol.options` trims `newName`; a blank one gives "New Symbol" or "<name> copy".
- Reset Transformation (the Symbols panel menu, and Reset on the Control bar while symbol instances are selected):
  `symbol.instance.resetTransform {ids?}` gives the selected symbol instances, in groups too, their symbol's own
  scale again, without rotation, shear or reflection, as one undo step. VectorCraft's own rules: each instance keeps
  the center of its bounding box in place; other objects, instances in perspective and instances already at their
  symbol's scale are left as they are, and no undo step is added when nothing changes; an id that names no object, or
  objects without a symbol instance, is an error. It returns `{ids}`, the instances it changed. The menu item and the
  button are enabled only while a selected instance would change.
- Dynamic symbols: an instance of a symbol whose `type` is `dynamic` changes the appearance of objects in the symbol's
  art for itself only and stays linked. `symbol.instance.setAppearance {id?, path, fill?, stroke?, opacity?, blend?}`
  names the object by `path`, child indices in paint order from the art's root down through its groups (`[]` is the
  root; `[1, 0]` is the first object of the art's second member when that member is a group). `fill` and `stroke` take
  a colour string or the paint params of `paint.setFill` (`color`, `none`, `swatch`, `tint`, `gradient`). Paths,
  compound paths, type, groups and layers take both. A symbol instance takes a solid color or `none` as its fill,
  which stains its art, and no gradient or pattern fill and no stroke. An image takes a fill or stroke only when that
  fill or stroke has a Convert to Shape or Outline Object effect. A guide, blend, envelope, Repeat, Intertwine group,
  gradient mesh or placed document takes neither. A refused fill or stroke (VectorCraft's own rule) gives an error
  that names the key and the object, and nothing of the call is applied; `opacity`, `blend` and `null` are taken for
  every object, so a stored change can be taken back. `opacity` is 0..100 and `blend` a blend mode name; `null` gives
  one back to the symbol. It is refused for a static symbol and for a path that names no object. On the canvas (VectorCraft's own rule), a Direct Selection click on an instance of a
  dynamic symbol selects the path, compound path or type object of its art under the pointer and outlines it
  (`symbol.instance.selectPart {id, path}`; Alt or Shift selects the instance whole). While it is selected,
  `paint.setFill` and `paint.setStroke` with a solid color or none, `paint.invert` and `paint.complement` on a solid
  color, all without `ids`, `id` or `item`, and the Eyedropper's `appearance.copyFrom` with a solid fill and stroke
  change that object's fill or stroke for this instance only, each as one undo step. A painted stroke on type without a
  weight gets 1 pt. A gradient or pattern for the object is refused with a status message, and so are `paint.invert` and
  `paint.complement` on type whose characters differ in color, Eyedropper options that pick up neither color, an
  Eyedropper source that is not a path, compound path or type (a symbol instance or a group, for which the paint defaults
  stay as they are too), and a click
  or `symbol.instance.selectPart` on any other object of the art (a group, image, blend or nested instance among them).
  The commands that change the selected objects without `ids` are refused the same way and change nothing, among them
  opacity and blending mode, moving and transforming, `path.moveAnchors`, `file.place` with `replace`, Puppet Warp, Delete
  and Cut, Group, Hide, Lock, Arrange, Expand,
  Recolor Artwork, plug-in filters, effects, the gradient commands, `gradient.selectStop`, the Appearance panel's rows and
  `symbol.instance.setAppearance`. A Cmd-drag that lends the Selection or Group Selection tool selects the instance whole
  first and moves it. `swatch.unused` and `recolor.colors` count the fills and strokes an instance keeps for objects of its
  art. Commands that change nothing in the document, such as `attributes.info`, `css.selection`, `css.export`,
  `effect.list` and `object.expand.info`, read the instance. These commands do not check for a selected object and use the instance:
  `edit.copy` copies the instance whole; `document.exportSelection`, `document.export` with `selectionOnly` and
  selection-mode `document.exportForScreens` export it whole; `artboard.fitToSelection` fits the artboard to the whole instance;
  `object.liquify` without `ids` takes the instance as its target and, since it reshapes paths only, leaves it as it is
  and lists it in `skipped` when the brush reaches it; `object.isolate` without `id` isolates the instance and clears the
  selection; `library.add` without `ids` adds a copy of the whole instance as a `graphic`, and its fill and stroke kinds
  read the instance rather than the selected object; `select.save` saves the instance, and `select.recall` then selects it
  whole. Direct Selection passes over hidden, locked and template objects of the art, and
  `symbol.instance.selectPart` refuses a path to one or to an object inside one, and an instance that is hidden or
  locked or sits in a hidden or locked group or layer. Choosing another selection tool selects the instance whole again.
  `symbol.instance.reset {ids?}` drops the selected instances' changes (the Properties
  panel's Reset). Redefining the symbol (`symbol.update`) reshapes every instance and keeps each change whose path the
  new art still has; `symbol.breakLink` bakes the changes into the expanded art; `symbol.edit` edits the symbol's own
  art, and each edit there keeps the changes whose path the art still has; making the symbol static drops them, and
  so does `symbol.replace` with a different symbol (a replace with the instance's own symbol keeps them).

```json
{"name":"run_command","arguments":{"command":"symbol.instance.setAppearance","params":{"id":42,"path":[1],"fill":"#d0021b","opacity":80}}}
```
- The Symbols panel menu's Select All Unused: `symbol.unused` (a query) lists the symbols nothing places, in panel
  order, and the panel shows them selected; its Delete Symbol then deletes them all, asked once, in one undo step.
  Instances in the art (opacity masks included), graph designs, pattern tiles (with the tiles that pattern editing's
  Cancel puts back) and the art of the document's brushes count, and so do the instances in the art of a symbol that
  is used; a symbol placed only inside an unused symbol is unused too.
- The Brushes panel menu's Select All Unused: `brush.unused` (a query) lists the brushes no stroke paints with, in
  library order, and the panel selects them. Strokes in the art (opacity masks included), symbols, graph designs,
  pattern tiles (with the tiles that pattern editing's Cancel puts back) and graphic styles count, and so do the
  strokes in the art of a brush that is used. The art of a brush that is not used does not count, so a brush whose own
  art is the only art that paints with it is listed. The panel's Delete Brush asks Yes/No when every brush it deletes
  is unused, and offers Remove Strokes or Cancel when one of them is in use.
- Brushes: `brush.list`, `brush.get {name}`, `brush.apply {name, ids?}`, `brush.new {type, name?, params?, ids?}`,
  `brush.setCurrent {name}`, `brush.options {name, params?, newName?}`, `brush.delete {name} | {names}`,
  `brush.duplicate {name, newName?} | {names}`, `brush.freehand`, `object.expandBrush {ids?}`. `brush.new`,
  `brush.duplicate` and `brush.options` trim the white space around the name they are given. A blank `name` gives
  the type's default name ("New Calligraphic Brush" for a calligraphic brush), and a blank `newName` gives
  "`<name>` copy" in `brush.duplicate` and keeps the name in `brush.options`. A Hue Shift `colorization` given to
  `brush.new` or `brush.options` without a `key` takes the color that the art's fills and strokes use most (on a tie,
  the one found first), or black when they use no solid color. `brush.options` changes only the fields of
  `colorization` and of an art brush's `scale` that `params` gives, so `{colorization: {method: "hueShift"}}` keeps a
  Hue Shift brush's key. `brush.new` and `brush.options` take a bristle brush name of at most 31 characters:
  `brush.new` refuses a longer one, counting the number it adds to a taken name, and `brush.options` refuses a
  longer `newName`. `brush.duplicate` does not check the length. A bristle brush that already has a longer name
  keeps it, and `brush.options` without a new name, or with a `newName` that only trims the white space around the
  name, does not refuse. Other brush types take a name of any length. A pattern brush's `outer_source` and `inner_source` set
  what that kind of corner draws when the brush has no `outer_corner` or `inner_corner` art for it: `none` draws no
  corner tile, and `autoCentered`, `autoBetween`, `autoSliced` and `autoOverlap` make one from the side tile. Without
  a source, the corner draws the other kind of corner's art. `brush.options {params: {outer_corner: null}}` removes
  the art. Outer and inner corner art are both drawn as the corner that joins a side tile on its right to one below
  it. The brush turns each corner tile so that those two arms meet the sides at the corner: 90° from one corner to
  the next on a rectangle, and halfway between the two sides at other angles. Pattern Brush Options also accepts `side_tile`, `outer_corner_tile`, `inner_corner_tile`,
  `start_tile` and `end_tile` in `params`. Each takes `none`, `original` or the name of a pattern
  swatch in the active document, and `{"pattern": "<name>"}` names a swatch even when its name is one of those
  words. Corner tiles also take `autoCentered`, `autoBetween`, `autoSliced` and `autoOverlap`. A swatch choice copies its whole art and tile frame into the brush. `original`
  keeps the current art and clears its swatch label. `side_tile: "none"` fails with "You must choose a
  side pattern." An unknown swatch name also fails; neither error edits the brush or adds an Undo step.
- Pen tilt, bearing and barrel rotation (#764): `path.freehand` and `brush.freehand` take points as
  `[x, y, pressure, tiltX, tiltY, rotation]` or `{x, y, pressure?, tiltX?, tiltY?, rotation?}`, with `null` for what a
  point lacks, and a `pointer_gesture` event takes `tiltX`, `tiltY` and `rotation`, which the Paintbrush records.
  `tiltX` and `tiltY` are degrees −90..90 in document axes, positive leaning right and leaning down the page (one
  without the other is 0); `rotation` is the barrel's rotation in degrees clockwise. The stroke keeps how far the pen
  leaned (`tilt`, 0 upright to 1 lying flat), which way (`bearing`, degrees counter-clockwise from leaning right; a
  pen within 1° of upright takes the bearing before it, or at the start of the stroke its first one) and how far the
  barrel turned (`rotation`, degrees counter-clockwise). A point without a tilt that other points have counts as
  leaning 45°, and one without a rotation as unturned. Calligraphic and Scatter `modes` take `tilt`, `bearing` and
  `rotation`. A Calligraphic value on Tilt goes from the value less the variation with the pen lying flat to the value
  plus the variation upright. Bearing and Rotation add
  the variation times the bearing or the rotation over 180°, and the angle turns on past ±180°. A Scatter value on
  Tilt goes from its first value with the pen lying flat to its second upright, and on Bearing or Rotation from its
  first value at −180° to its second at 180°. A stroke without that input draws the brush's own value (a Scatter
  brush's value half way between the two).
- Variables: `variable.list`, `variable.define {name, kind: text|visibility|image|graph}`, `variable.bind {variable, ids?}`,
  `variable.makeTextDynamic`, `variable.makeVisibilityDynamic`, `variable.makeImageDynamic`, `variable.makeGraphDynamic`
  (`variable.bind` and `variable.makeTextDynamic` refuse type that already has another text variable and name it),
  `variable.unbind`, `variable.rename`, `variable.delete`, `variable.highlight` (image values are file paths, graph
  values CSV as `graph.setData`). `variable.lock {locked?}` (Lock Variables) locks or unlocks the variables;
  without `locked` it toggles, and the lock is saved with the document. While the variables are locked,
  `variable.define`, `variable.delete`, `variable.rename`, the four `variable.make…Dynamic` commands and a
  `variable.loadLibrary` that would delete or define a variable (below) fail with "variables are locked";
  `variable.bind`, `variable.unbind` and the `dataset.*` commands still run (except a `dataset.import` whose header
  would define a variable, below), and `variable.list` reports `locked`. A variable that an opened `.ai` declares
  without a kind VectorCraft reads is listed with `kind: "unknown"`: it takes no values, and `variable.bind`
  refuses it. When an opened `.ai`'s layers are read, the objects it binds to its text, visibility and image
  variables come in bound when they are of the variable's kind, locked and hidden objects included, and
  `variable.list` counts them in `bindings`.
- Data sets: `dataset.list`, `dataset.new {name, values?}`, `dataset.set {name, newName?, values?}` (`newName` also
  renames the row, in the same undo step; Edit Data Set's Name field sends it), `dataset.rename`,
  `dataset.delete`, `dataset.capture`, `dataset.update`, `dataset.select {name}`, `dataset.next` /
  `dataset.prev` (each `dataset.select` / next / prev is one undo step over every bound object), `dataset.import {path|data|dataBase64|csv}` /
  `dataset.export {path?}` (CSV: one column per variable, and no column of data set names). `dataset.import` also
  reads tab-separated files. Every header cell is a variable, whatever its case (`name` and `Name` included). The
  first row holds the field names, as the
  [set-up page](https://helpx.adobe.com/illustrator/desktop/automate-visualize-data/automate-actions/set-up-data-source-files.html)
  describes. The rows replace every data set of the document and are named `Data Set 1`, `Data Set 2` and so on, in file order;
  the document's variables and their bindings stay, and the reply's `deletedDataSets` lists the data sets that were replaced. An import error
  about a row's values gives the file line the row starts on. A header cell that names no variable defines one, unbound, in the same undo step as the
  rows. `#Shown` defines a visibility variable `Shown`, `@Photo` defines an image variable `Photo`, and a cell without
  a prefix defines a text variable. `dataset.export` writes a visibility variable's header as `#Shown` and an image
  variable's as `@Photo`, and a text or graph variable's as its name alone. Its reply's `warning` names the variables
  whose header or values make an import into a document without them refuse the file (a `%` header, a `#` header
  whose values aren't true or false). Otherwise it names the variables that such an import does not read back with
  their kind: text variables whose names start with `#` or `@`, and graph variables. Export is refused, naming both variables, when a variable's header is
  another variable's name (a visibility variable `Show` next to a text variable `#Show`). A prefixed cell whose name
  is already a variable of the prefix's kind names it;
  one whose name is a variable of another kind fails the import. A `%` cell that isn't a variable's exact name (graph
  data files) is refused, and so are two cells that name one variable (`City` twice, or `Shown` and `#Shown`). The
  reply's `defined` lists the variables the import defined. While the variables are
  locked, an import whose header would define a variable fails with "variables are locked".
  A `dataset.import` file that isn't UTF-8 is read as Windows-1252, unless it starts with the UTF-8 byte-order mark.
  A file that starts with a UTF-16 byte-order mark is read as UTF-16, and a file without one that holds NUL bytes is
  refused. `csv` and `data` are read as the text they hold.
- Variable libraries: `variable.loadLibrary {path|xml|dataBase64}` (the Variables panel menu's Load Variable
  Library…) replaces the document's variables and data sets with a variable library's in one undo step. A load, like a
  `dataset.import`, adds no undo step when it leaves the variables and data sets as they were. The library
  is XML whose `variableSets` element holds `variableSet` elements, each with a `variables` element of `variable`
  elements (`varName`, `trait`) and a `sampleDataSets` element of `sampleDataSet` elements (`dataSetName`). Every
  data set is deleted, and every variable of the document is deleted with its bindings and values, except a text
  variable the library also declares: it keeps its bindings, so selecting one of the library's data sets changes the
  objects bound to it. Then the library's other variables whose `trait` is `textcontent` are defined as text
  variables, unbound. In each data set, the element named
  after a text variable gives its value: the text of its `p` elements, one paragraph each. An element without a `p`
  element gives no value. Variables of other kinds are left out with their values: the reply lists them in `skipped`
  (`name`, `trait`), and its `warning` names each `trait` with its count. The reply's `deleted` and `deletedDataSets`
  list the variables and data sets the load deleted. While the variables are locked, a library that would delete or
  define a variable fails with "variables are locked". The reply is `{variables, deleted, deletedDataSets, names,
  skipped, warning?}`. A library larger than 8 MiB, one whose elements nest more than 256 deep, and one with more
  than 256 variables or 1,000 data sets are refused. Its document type declaration may hold an external identifier,
  comments, processing instructions and up to 64 entity declarations whose values are plain text (no `<`, `&` or
  `%`). A declaration that holds anything else, and entity references that expand the library past 8 MiB, are
  refused. So are a variable or data set declared twice and a value for a variable the library doesn't declare.
  `variable.saveLibrary {path?}` (the panel menu's Save Variable Library…) writes the document's text variables and
  every data set as a variable library in the layout `variable.loadLibrary` reads: a `variable` element with
  `varName` and `trait="textcontent"` for each text variable, and a `sampleDataSet` element with `dataSetName` for
  each data set, holding an element named after each text variable it has a value for, with one `p` element per line
  of the value. Loading the file gives back the same text variables, data set names and text values. Variables of
  other kinds are left out with their values: the reply lists them in `skipped` (`name`, `kind`), and its `warning`
  names each kind with its count. Saving is refused for a document without a text variable, for a text variable whose
  name isn't an XML name (it starts with a letter or `_` and holds only letters, digits, `-`, `_` and `.`), for a
  data set whose name or text value holds a control character, for a data set name with white space at either end,
  for a name longer than 255 characters or used twice, and past `variable.loadLibrary`'s caps (256 variables, 1,000
  data sets, 8 MiB). Without `path` the reply is `{xml, variables, datasets, skipped, warning?}`;
  with it, the file is written and the reply is `{path, variables, datasets, skipped, warning?}`.
- Batch: `action.batch {commands, source?: folder|datasets, folder?, destination?: none|saveAndClose|folder,
  destinationFolder?, overrideOpen?, overrideSave?, overrideExport?, includeSubdirs?, stopOnError?}` plays an
  action on every file in a folder or every data set (the Actions panel menu's Batch…). Override Save and Override
  Export end the file name of a step with `format` in that `format`, in lower case and without a leading dot, when it
  is one of the format's extensions (`.svg` and `SVG` give `.svg`, `.JPEG` gives `.jpeg`), and otherwise in the
  format's first extension (`png8` gives `.png`, `template` gives `.vctemplate`). A step without `format` keeps the
  source file's extension (`.vectorcraft` over data sets).
- Action set files: `ui.actionSet.save {set?: index, id?, path?}` writes the Actions panel's set at `set` (0 is the
  top set; by default the current set) with its actions to an action set file at `path`, else a picked file named
  after the set (the web downloads it), and returns `{path}`. `id` holds the panel's own set id, which the panel menu
  passes: the set with this id is saved wherever it is when the file dialog answers, and `set` is not read. A set that
  would be larger than 8 MiB as a file, has more than 1,000 actions or has an action of more than 10,000 steps is not
  written, and the command returns the same error as a load of such a file. `ui.actionSet.load {path? | data?}` reads
  the file at `path`, else a picked one, or the file's text `data`, adds its set after the other sets and returns
  `{set, name, actions}`. A set named like one already in the panel gets the next free number (`Print 2`), and the
  other sets and the current set stay as they are. An action set file (`.vcactions`) is JSON:
  `{"format": "vcactions", "version": 1, 2 or 3, "name", "actions": [{"name", "steps": [{"command", "params", "excluded"?}]}]}`, each step
  a command id and its params as `command.batch` takes them, or a stop: the command `action.stop` with
  `{message, allowContinue}`. `excluded: true` preserves a step but omits it from playback;
  an absent or false flag includes the step. Files with stops use version 3, and files with excluded steps and no stops
  use version 2, so older builds refuse them
  instead of playing the excluded steps or failing at a stop; other files use version 1. A file with another `format` or a `version` other than
  1, 2 or 3, a stop whose message is not text or whose `allowContinue` is not true or false, a file larger than 8 MiB, one with more than 1,000 actions and one with an action of more than 10,000 steps are
  refused, and no set is added. The Actions panel menu's Save Actions… and Load Actions… run these; Load Actions… is
  disabled where there is no open dialog (the web). `ui.actionSet.load` also reads the action sets other apps save as
  text (`.aia`); see "Action sets from other apps".
- Actions: `ui.action.rename {set, action, name}` renames the Actions panel's action at `action` in set `set` (0 is
  the top set) to `name` with spaces trimmed from both ends, keeps its steps, function key and color, and returns
  `{name}`. An index past the last set or action, or an empty name, is an error, and then nothing is renamed.
- Actions: `ui.action.stepsIncluded {set, action?, step?, included, others?}` includes or excludes all steps of a set,
  one action, or one step. With `step` and `others: true`, it leaves that step as it was and applies `included` to the
  other steps of its action (the panel's Alt/Option-click). An invalid set, action or step changes nothing. The panel's
  checkmarks run this command; normal playback and Batch… skip excluded steps.
- Actions: `ui.action.new {set, name, key?, cmd?, shift?, color?}` adds an action named `name` (spaces
  trimmed) after the last action of set `set` (0 is the top set), starts recording into it and returns
  `{set, action}`; Stop Playing/Recording (`ui.action.stop`) ends the recording. `key` 1 to 15 is the function key F1 to F15 that plays the
  action (0 or null for none), with `cmd` (Ctrl, Command on macOS) and `shift`. `color` is its Button Mode color
  (`red`, `yellow`, `green`, `blue` or `violet`; null or `""` for none), which the panel does not show. Action function
  keys take priority over command shortcuts and preserve the command's binding. On Windows F1, Ctrl+F4 and Ctrl+F6
  cannot be used. While an action is recording, and for an index past the last set,
  an empty name, or a key or color that is not one of these, it is an error and nothing is added. The panel's Create
  New Action button and menu's New Action… open the `actionNew` dialog, whose Record runs this.
- Actions: `ui.action.record {set?}` starts recording a new action in the chosen set, or the current set when `set`
  is left out, and returns `{set, name}`. With no sets, it creates Set 1. The action gets the next available
  `Action N` name in that set. The Actions panel's Begin Recording button runs this command. An invalid set or an
  existing recording is an error and leaves the sets and recording unchanged.
- Actions: `ui.action.options {set, action, name?, key?, cmd?, shift?, color?}` changes the action at
  `action` in set `set` and keeps its steps: `name` renames it, `key` with `cmd` and `shift` sets its function key,
  and `color` its Button Mode color, as `ui.action.new` takes them; a param left out keeps what the action has, and
  `cmd` or `shift` alone changes that modifier of its key. It
  returns `{name, key, color}`. The panel menu's Action Options… runs this for the selected action. A function key
  plays its action from the keyboard; like command shortcuts, it does nothing while a dialog, the command palette or
  a text field has the keyboard, or while a tool is busy. Editing command shortcuts (`shortcuts.set`, Keyboard Shortcuts), loading preferences and
  `ui.actionSet.load` preserve action function keys. When several actions have the same key, the first in panel order
  plays. A stored key reserved on the current platform does not play an action.
- Actions: `ui.action.play {action, set?}` plays the action named `action` at the speed `ui.action.playbackOptions`
  sets; `set` names the set to look in (default: every set), and a `set` that is not a string is an error. At
  Accelerated it plays the action as one undoable batch and returns what `command.batch` returns. At Step by Step and
  Pause For it plays the first step at once and returns `{results: [the first step's result], pending}`, where
  `pending` counts the steps still to play; the app plays them one at a time, each as its own undo step. When the next
  step is due and the active document is not the one that was active after the step before (another document was
  made active, or that document was closed), the playback ends with a status and the rest do not play; a step that
  makes another document active itself does not end it. A name that no
  action has, or that more than one action in the sets looked in has, or whose every step is excluded, is an error, and
  then nothing plays. So is an action played while another is still playing at Step by Step or Pause For. A
  `command.batch` step, which an action played or a panel operation run as one batch records, plays as its own steps,
  here and in Batch…. An action with an included stop plays up to the stop at any speed and then waits, with its
  message box open, until `ui.action.continue` or `ui.action.stop`. At Accelerated the steps between two stops are one
  undoable batch each. The result is then `{results, pending, atStop?}`, with `atStop: true` when the action already
  waits. Batch… refuses an action with an included stop, and a stop that is excluded does not hold playback.
- Actions: `ui.action.insertStop {set?, action?, step?, message?, allowContinue?}` inserts a stop. With `set` and
  `action`, the stop goes into that action, after step `step`, else after its last step, and the command returns
  `{set, action, step}`. Without them it goes into the recording in progress, after the steps recorded so far, and
  returns `{recording: true}`. `message` is the text playback shows (default: none), and `allowContinue` (default:
  false) offers Continue in the message box. An action with an empty message still waits at the stop, which is
  VectorCraft's own rule. A stop in an action becomes the panel's selected step, and its action and set become the
  selected ones. An index past the last set, action or step, a param of another type, params that are not an
  object, or no action and no recording is an error, and then nothing changes. The panel menu's Insert Stop… opens the `actionStopRecord` dialog
  (`message`, `allowContinue`, `set`, `action`, `step`), whose OK runs this.
- Actions: `ui.action.continue {}` plays on from the stop the playing action waits at, on the document active now, and
  closes the stop's message box (`actionStopMessage`). The next step runs at once, or at Accelerated the steps up to
  the next stop as one undo step. It returns `{results, pending, atStop?}`. With no action waiting at a stop it is an
  error. The panel's Play Current Selection runs this while an action waits. In the message box, Continue (shown with
  Allow Continue) runs this, and Stop closes the box while the action keeps waiting for the manual task.
- Actions: `ui.action.deleteStep {set, action, step}` deletes one step, a stop or a command, of the action at `action`
  in set `set` (0 is the top set, action or step), and returns `{set, action, step}`. The steps after it keep whether
  they are excluded, and no step of that action stays selected. A Record Stop dialog open on that step closes, and one
  open on a later step of the action follows it up. A missing or invalid index is an error, and then nothing changes.
  Undo does not restore a deleted step. The panel's Delete Selection runs this for the selected step.
- Actions: `ui.action.playbackOptions {speed?, seconds?}` sets how the Actions panel plays actions and returns
  `{speed, seconds}`. `speed` is `accelerated` (the default: every step at once), `step` (one step at a time, with the
  window drawn after each step before the next runs) or `pause` (a pause of `seconds` between steps). `seconds` is a
  whole number from 1 to 60, VectorCraft's own range, and stays set while another speed is chosen. A param left out
  keeps its value. Any other `speed` or `seconds` is an error, and then nothing changes. The setting is kept with the UI
  preferences. The panel menu's Playback Options… opens the `actionPlayback` dialog (`speed`, `seconds`), whose OK runs
  this. Batch… (`action.batch`) plays every step at once whatever the setting.
- Actions: `ui.action.stop {}` ends the recording in progress and keeps its steps, in the action `ui.action.new` made
  for it, else in a new action after the last action of its set, and returns `{stopped}`. It also ends an action
  playing at Step by Step or Pause For, or one waiting at a stop, whose message box closes: the steps already played
  stay, and the rest do not play. With nothing
  recording or playing it changes nothing. The panel's Stop Playing/Recording button runs this.
- Action sets: `ui.actionSet.delete {sets?: [index], ids?: [id]}` deletes the Actions panel's sets at these indexes
  (0 is the top set) with their actions, by default the current set, and returns `{deleted}`. An index past the last
  set is an error, and then no set is deleted. `ids` holds the panel's own set ids, which its question passes: the
  sets with these ids are deleted wherever they are now and `sets` is not read; when one of them no longer exists, the
  command returns an error and deletes no set. A recording into a deleted set stops and keeps no steps; a recording into another set
  goes on in that set. The panel's Delete Selection (the trash button and the panel menu's Delete) deletes a selected
  step (`ui.action.deleteStep`), else a selected action. Otherwise it asks before it runs
  this for the selected sets, and Shift-click on a set's name selects the sets from the current one to it. Undo does
  not restore a deleted set.
  `ui.actionSet.duplicate {set?: index, action?: index}` copies the action at `action` in set `set` to right after it
  in the same set, and returns `{set, action}`; without `action` it copies the set with all its actions to right after
  it, and returns `{set}`. In both cases `set` defaults to the current set. The copy is named after the original with
  " copy" added, a copied set gets an id of its own, and the copy is selected. An index past the last set or action is an error. The
  panel menu's Duplicate runs this for the selected action, else the current set.
- Image Trace: `imageTrace.presets`, `imageTrace.make {id?, preset?, params?, view?}` (or `imageTrace.makeAndExpand`),
  `imageTrace.setView`, `imageTrace.expand`, `imageTrace.release`. A key inside `params` that is not one of the trace
  settings, such as `colours`, is an error that names it and the nearest setting (`colors`), as for a command's own keys.
  With `byColor: true`, `imageTrace.expand` and `imageTrace.makeAndExpand` put each color's paths in a group of their
  own, named by the global swatch the color links to (a tint with its percentage, as `Sky 50%`) or as `#rrggbb`;
  gradient fills share a group named `Gradients`. The groups stack in the order of the lowest path in each, and each
  keeps its paths' order. For either Expand command, a path that would move under a path of another
  color whose painted bounds, effects and shadows included, come within one document point of it starts another group
  of its color instead of joining an earlier one. Paths whose painted bounds come within one document point of an
  intervening color stay in separate groups to preserve their stacking order; at small view or raster-export scales,
  regrouped paths farther apart can still share antialiased pixels, so edge pixels may differ. In a Flat Logo trace of a
  black ring around a white disc around a black dot, black gets two groups. `layer.releaseToLayers` on the expanded group then
  gives each group a sublayer. A trace made with `autoGrouping` is refused. Expanding by color also refuses a trace
  with a live Pathfinder effect.
  `imageTrace.presets.save {name?, newName?, preset?, enhanced?, params?}` saves tracing settings as a preset kept with
  the preferences: `params` over `preset`'s (default: the saved preset `name`, else Default), checked as `imageTrace.make`
  checks them; without `name` it is a new "Tracing Preset N", and `newName` renames a saved preset (the reply's
  `renamedFrom` holds the old name). `imageTrace.presets.delete {name}` deletes one. Built-in presets and the name
  `Custom` can't be saved over, renamed or deleted. `imageTrace.presets` lists the built-in presets with
  `builtIn: true`, then the saved ones with `builtIn: false`, and returns a `revision` that grows each time the saved
  presets change. `imageTrace.make {preset}` takes a saved preset's name, with ASCII letters matching in either case.
  Its journal entry records the saved preset's settings as `savedPreset`, so a replay traces with them after the preset
  is changed, renamed or deleted. In the app,
  `ui.imageTracePresetDialog {params?, rename?}` opens the Image Trace panel's Save as New Preset… or Rename… dialog.
- Live Paint: `livePaint.make`, `livePaint.merge`, `livePaint.fill {group?, point}`, `livePaint.strokeEdge`,
  `livePaint.info {group}`, `livePaint.release`, `livePaint.expand`, and `livePaint.gapOptions {ids?, detect?, size?,
  color?, closeGaps?}` (Object › Live Paint › Gap Options). Make gives each covered face the fill of its front-most
  covering source, including compound paths with shared curved boundaries. Expand keeps those face fills.
  With `detect: true`, paint stops at the gaps that run
  from a loose end of a path to the nearest path, up to `size` wide (`small` 3 pt, `medium` 6 pt, `large` 12 pt, or a
  number of points up to 72); `closeGaps: true` adds an unpainted path across each of them. `color` (the Gap Preview
  Color) is `#rrggbb` or a layer color's name; a hex color with an alpha part, `#rrggbbaa` or `#rgba`, is an error and
  changes nothing. The options are saved with the group, and the reply
  and `livePaint.info` list the gaps up to `size` wide that no path closes, as `[x0, y0, x1, y1]`. `livePaint.merge`
  keeps the bottom group's options when its Gap Detection is on, and otherwise turns Gap Detection on with `small`
  when another merged group has it on. When Gap Detection is off in every merged group, it stays off.
  The Gap Options dialog (`ui.livePaintGapOptions`) takes `color` as `#rrggbb` or a layer color's name; its OK gives
  an error for a `color` with an alpha part (`#rrggbbaa` or `#rgba`) and changes nothing. With Preview on, a
  `#rrggbbaa` color is not previewed: the groups keep their own color, and the previewed gaps draw in black.
  `view.livePaintGaps` (View › Show Live Paint Gaps, or Hide Live Paint Gaps while it is on) turns the drawing of
  those gaps on or off. While it is on and edges show, the canvas draws the gaps of each selected Live Paint group in
  that group's Gap Preview Color. The reply is the new state (`true` when on).
  The Live Paint Selection tool's options (`tool.setOption {tool: "livePaintSelection", values}`; double-clicking the
  tool opens them, dialog `livePaintSelectionOptions`) are `selectFills` and `selectStrokes` (both on), `color` (the
  highlight color, `#rrggbb` or a layer color's name; `#ff2a2a`) and `width` (the highlight's width, 1 to 20 screen
  pixels; 4). The dialog's OK gives an error for a `color` with an alpha part (`#rrggbbaa` or `#rgba`) and changes
  nothing.
  With `selectStrokes` off, Alt-click selects the face under the pointer as a click does; with
  `selectFills` off, a click selects the edge nearest the pointer as Alt-click does; with both off, a click selects
  no face or edge. A drag of 3 screen pixels or more draws a marquee, and the release runs `select.set {ids}` with
  the faces and edges it touches (a face or edge partly inside counts, and so does a face that holds the whole
  marquee), or `select.none` when it touches none. With Shift held at the press, the release runs
  `select.toggle {ids}` with the faces and edges it touches, each of which leaves the selection or joins it, and runs
  nothing when it touches none. `selectFills` off leaves the faces out, and `selectStrokes` off leaves the edges out. A click and a marquee run their command on the
  release, and Escape between the press and the release removes the marquee so that the release runs nothing. The
  tool does not highlight or select the faces and edges of a Live Paint group that is locked, on a locked layer or on
  a locked artboard, or in isolation mode outside the isolated art.
  The Live Paint Bucket's options (`tool.setOption {tool: "livePaintBucket", values}`; double-clicking the tool opens
  them, dialog `livePaintBucketOptions`) are `paintFills` (on), `paintStrokes` (off), `highlight` (on), `color` (the
  highlight color, `#rrggbb` or a layer color's name; `#ff2a2a`) and `width` (the face highlight's width, 1 to 20 screen
  pixels; 4; an edge's highlight is half as wide). The dialog's OK gives an error for a `color` with an alpha part
  (`#rrggbbaa` or `#rgba`) and changes nothing. With `paintFills` alone on, a click runs `livePaint.fill` and Shift-click runs `livePaint.strokeEdge`.
  With `paintStrokes` alone on, a click runs `livePaint.strokeEdge` on the edge within 4 screen pixels of the pointer,
  or nothing when none is in reach, and Shift-click runs `livePaint.fill`. With both on, a click runs
  `livePaint.strokeEdge` when an edge is within 4 screen pixels of the pointer and `livePaint.fill` otherwise, and
  Shift-click runs `livePaint.fill`. With both off, a click runs nothing. With `highlight` off, the face or edge under
  the pointer is not outlined.
  `livePaint.merge` keeps the bottom Live Paint group's id, name, lock, visibility, transparency, appearance, opacity
  mask and graphic style link. With a Live Paint group among the objects, `livePaint.make` adds the others to it as
  `livePaint.merge` does, and so does `livePaint.fill {ids}` when `point` is outside that group's faces.
  `livePaint.fill` does not fill a face that is hidden on its own. With `group`, a point on a hidden face is an
  error. With `ids`, at a point on a hidden face it fills the face there of another Live Paint group among the
  objects; otherwise it returns an error and does not merge the objects.

`brush.selectionArt {ids?|id?}` (a query) replies `{art}`: the object that `brush.new` given the same `ids` or `id`,
or the same selection, takes as a new scatter or art brush's art and as a new pattern brush's side tile. Several
objects become one group, and `art` is `null` when nothing is selected or `ids` name no object. In the app, the
Brushes panel's New Brush button, its menu's New Brush… and art dropped on the panel open New Brush
(`ui.newBrush {ids?}`, dialog `newBrush`, field `type`). Scatter, art and pattern need art. OK opens that type's
Brush Options for the new brush; for a scatter, art or pattern brush they preview the art from `brush.selectionArt`.
Their OK runs `brush.new {type, name, params, ids?}`, and Cancel adds no brush.

`variable.bind`, `variable.unbind`, `variable.delete`, `dataset.set`, `dataset.update`, `dataset.delete`,
`dataset.select`, `dataset.next` and `dataset.prev` add no undo step when they leave the document as it was, such as
binding art to a variable it already has or applying the active data set again: the steps to redo stay, and a saved
document stays unmodified.

`dataset.select`, `dataset.next` and `dataset.prev` reply `{name, applied, skipped}`. They add `overflow: [id…]`
when area type or type on a path whose text they rewrote doesn't fit, as `text.setText` does (see
[Fitting area type](#fitting-area-type-auto-size-shrink-text-to-fit-overflow)). They skip an object that is
locked, inside a locked layer or group, or on a locked artboard, and count it in `skipped`. They rewrite hidden objects and objects inside
a hidden layer or group, which stay hidden unless one of the data set's visibility values shows them. When such a
layer is shown again, its type holds the data set's text. They apply the values in the order the variables are
defined. Of two variables of the same kind on one object, the one defined last wins. `dataset.list`,
`dataset.capture` and `dataset.update` read hidden objects and objects inside a hidden layer or group as well.

A text value bound to any frame of threaded text replaces the whole story, and the text flows through the frames
again; `overflow` names the story's last frame when the text does not fit. `dataset.capture`, `dataset.update` and
`dataset.list` read such a frame as the whole story, every frame's text joined in thread order. When any frame of
the story is locked or inside a locked layer or group, the text value is skipped and counted in `skipped`, and the
story is not read. Two text variables
bound in one story apply in the order they are defined, and the one defined last wins.

```json
{"name":"run_command","arguments":{"command":"symbol.list","params":{}}}
{"name":"run_command","arguments":{"command":"brush.list","params":{}}}
{"name":"run_command","arguments":{"command":"variable.list","params":{}}}
{"name":"run_command","arguments":{"command":"dataset.list","params":{}}}
{"name":"run_command","arguments":{"command":"dataset.capture","params":{}}}
{"name":"run_command","arguments":{"command":"dataset.select","params":{"name":"Data Set 1"}}}
{"name":"run_command","arguments":{"command":"imageTrace.presets","params":{}}}
{"name":"run_command","arguments":{"command":"command.batch","params":{"label":"Offset","commands":[{"command":"object.move","params":{"dx":10,"dy":10,"copy":true}}]}}}
```

When no object is bound to a variable that the data set has a value for, for example after `dataset.import` defines
new variables without binding them, `dataset.select`, `dataset.next` and `dataset.prev` apply nothing and add
`warning`, which the app's status bar shows: "nothing in data set A was applied: no object is bound to a variable it
has a value for". A data set with a value for a bound variable gives no warning even when every object bound to it is
locked; `skipped` counts those objects.

A `dataset.import` error about a value says why it was refused: a visibility value other than true, false, 1, 0, yes
or no, or a graph value longer than 262,144 bytes.

`dataset.update {all?}` writes what the bound objects hold now into the active data set, for each variable that the
data set holds. A variable whose objects report no value, because they are locked, inside a locked layer or group,
on a locked artboard, or deleted, keeps the data set's value. With `all: true` it also adds every other variable that the
objects report a value for. The reply gives `values`, the number of values it wrote, and `left`, the variables it
did not write. When every bound object is locked, it writes no value and adds no undo step, and `dataset.capture`
returns the error "no bound object reports a value (locked objects and embedded images report none)".

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
`effect.move` rejects a `copy` value that is not a boolean, and every command that takes `aligned` rejects an `aligned`
value that is not a boolean.

Effect dialogs use the targeted object, group, or layer shown in Appearance. The Transform dialog previews valid
numbers as they are typed, including Copies, distances with units, and rotation. With Preview off, changes apply
on OK. Cancel restores the original artwork.

## Overprint

Each fill and stroke (and each character's fill and stroke) can overprint: its inks print over the inks below instead
of knocking them out. `object.setOverprint {fill?, stroke?, item?, ids?}` sets it (groups set their contents, type its
characters too; `item` aims at one appearance item) and `attributes.info {ids?}` reads it back (`null` where the
objects differ). Overprint Preview (`view.overprintPreview`) and Separations Preview show it; rendering approximates
it by darkening (the stronger ink of each plate wins, so a colour overprinting itself stays that colour). Older files that listed Overprint Black objects in the document get the flags on load.
PDF export writes them with `advanced.overprint: "preserve"` (the default): each overprinting fill and stroke sets a
graphics state with `/OP true /op true /OPM 1` (`/OPM 0` in PDF/A). With `discardWhiteOverprint` (`document.setup`, on by
default) white ones knock out instead; `"discard"` writes none.

```json
{"name":"run_command","arguments":{"command":"object.setOverprint","params":{"ids":[12],"stroke":true}}}
{"name":"run_command","arguments":{"command":"attributes.info","params":{"ids":[12]}}}
{"name":"run_command","arguments":{"command":"view.overprintPreview","params":{"on":true}}}
```

## Revolve (initial 3D effect)

`effect.apply {effect: "threeD.revolve", params: {...}, ids?, item?}` applies a live Revolve to a path
or compound path. `effect.dialog` opens compact options with artboard rotation and light gizmos, `effect.setParams` edits it, and
`effect.expandAppearance` bakes it into shaded vector faces. Native saves keep the editable source;
SVG/PDF exports bake the same faces the canvas draws. Only one Revolve is supported per profile.

Parameters: `angle` (degrees, 0..360, default 360), `offset` (points, 0..100000, default 0),
`edge` (`"left"` or `"right"`), `rotationX/Y/Z` (degrees, defaults 0/0/0), `perspectiveAngle`
(degrees, 0..160, default 0; an earlier `perspective`, 0..100%, is read as the angle that shows the same view), `segments` (8..128, default 64), `shade` (default true), `lightColor` (opaque `#rrggbb`, default `#ffffff`), `lightIntensity` (0..100%, default 80),
`lightRotation` (degrees, -180..180, default 125.26; 0 on the object's right, counterclockwise), `lightHeight`
(degrees, 0..90, default 30; 90 straight in front), `lightBehind` (default false), `lightSoftness` (0..100%,
default 0), `ambient` (Ambient Light, 0..200%, default 25; an earlier `lightAzimuth` and `lightElevation` are read
as the same light), `cap` (boolean, default true), and `expandVisibleOnly` (boolean, default true). A vertical-line profile needs a positive offset to have a radius.
`cap` closes each circular endpoint of an open profile with one curved, uniformly shaded
vector path. Body faces share that curved rim. Partial sweeps use sectors and leave their angular
cut sides open. Closed or coincident-end profiles are unchanged.
Missing `cap` enables caps; set it to false for open endpoints. Shaded Revolve surfaces automatically interpolate curved-surface lighting with vector gradients while preserving profile corners and flat caps. There is no separate smoothing parameter. `shade: false` disables all lighting. Visibility of intersecting surfaces is approximate.
`lights` is null by default, retaining the earlier single-light controls. Set an array of one to ten objects to replace that light: `{color: "#rrggbb", intensity: 0..100, rotation: -180..180, height: 0..90, softness: 0..100, behind: bool}`. Defaults per light are white, 80%, 125.26 degrees, 30 degrees, 0%, false. Omitted fields use defaults; unknown nested keys and malformed lists are refused before mutation. `ambient` is shared and counted once. Set `lights: null` to restore the earlier controls. The list is saved in the native file and survives preview, undo and expansion.

```json
{"name":"run_command","arguments":{"command":"effect.setParams","params":{"index":0,"item":null,"params":{"lights":[{"color":"#ff8040","rotation":135,"height":35,"intensity":70},{"color":"#4080ff","rotation":-45,"height":20,"intensity":40}],"ambient":20}}}}
```

`lightColor` changes only the earlier directional light; ambient stays neutral. Invalid command colors are refused before editing, and invalid loaded colors fall back to white. Colored smooth faces use ordinary RGB vector gradients; source gradients keep their placement and opacity with vector blending/masks. Pattern tiles retain neutral shading.
The dialog's Lighting tab manipulates the same `lightRotation` and `lightHeight` parameters;
its guide visibility and tab choice are UI state, excluded from saved effect parameters.
`expandVisibleOnly` is saved with the effect. It trims covered portions of opaque solid surfaces only
when running `effect.expandAppearance`, preserving visible interiors through openings. Set it to false
to expand all faces. Live preview and SVG/PDF baking keep the full surface. Transparent/unknown paint,
non-normal blending, later geometry effects or bounded visibility failures preserve all faces.
See [Revolve](revolve.md) for the scope and limitations.

```json
{"name":"run_command","arguments":{"command":"effect.apply","params":{"effect":"threeD.revolve","item":null,"params":{"angle":360,"offset":20,"edge":"left","rotationX":0,"rotationY":0,"rotationZ":0}}}}
{"name":"run_command","arguments":{"command":"effect.setParams","params":{"index":0,"item":null,"params":{"angle":180,"lightRotation":30,"lightColor":"#ff8040","cap":true}}}}
{"name":"run_command","arguments":{"command":"effect.setParams","params":{"index":0,"item":null,"params":{"expandVisibleOnly":true}}}}
{"name":"run_command","arguments":{"command":"effect.expandAppearance","params":{}}}
```

## 3D lighting presets

`effect.lightingPreset {index: int, preset: "standard" | "diffuse" | "topLeft" | "right", item?, ids?, target?}`
replaces the lighting rig and ambient of an existing Revolve, Extrude & Bevel, Inflate or Rotate
effect. `item: null` addresses object-level effects; an item index addresses a fill/stroke's
effects. All targets must have a 3D effect at `index`, otherwise nothing changes. The operation
is one undo step and preserves Shading, geometry, source paint and the view.

The four original rigs use one white front light at 80% intensity. Standard keeps the existing
default; Diffuse uses 100% Softness and 35% Ambient Light, Top Left uses Rotation 135 and Height
45, and Right uses Rotation 0 and Height 30. Full values and the public behavior reference are in
[3D lighting presets](three-d.md#presets). These are original settings, not imported preset files.
The document stores their numeric settings; manual edits remain possible through `effect.setParams`.

```json
{"name":"run_command","arguments":{"command":"effect.lightingPreset","params":{"index":0,"item":null,"preset":"topLeft"}}}
```

## 3D object type

`effect.setParams {index: int, threeDType: "plane" | "extrude" | "revolve" | "inflate", params?, item?, ids?, aligned?}`
changes an applied 3D effect into Rotate, Extrude & Bevel, Revolve or Inflate in place, as the object types of
**Window › 3D and Materials** do ([3D and Materials panel](three-d.md#3d-and-materials-panel)). The settings both
effects take keep their values (rotation angles are wrapped into -180..180 when the new type is not Revolve), the
settings only the new effect takes start at its defaults, and `params` then merge into the new effect's settings,
checked as that effect's. It is one undo step. Any other `threeDType` value, or an effect at `index` that is not a
3D effect, is an error and changes nothing. The type the effect already has, with no `params`, `visible` or
`warpStyle`, changes nothing, records no undo step and returns `{ids: []}`.

```json
{"name":"run_command","arguments":{"command":"effect.setParams","params":{"index":0,"item":null,"threeDType":"inflate","params":{"volume":70}}}}
```

## Extrude & Bevel, Inflate and Rotate

`effect.apply {effect: "threeD.extrude" | "threeD.inflate" | "threeD.rotate", params: {...}, ids?, item?}` applies
the other 3D types to a path or compound path, live like Revolve (the source stays editable; SVG/PDF exports and
`effect.expandAppearance` use the same shaded faces). An object takes one 3D effect, Revolve included.

`threeD.extrude`: `depth` (points, 0..2000, default 50), `twist` (degrees, 0..360, 0), `taper` (1..100%, 100),
`cap` (true solid, false hollow), `bevel` (`none`, `classic`, `rounded`, `cove`, `step`, `ogee`), `bevelWidth`
and `bevelHeight` (points, default 4), `bevelRepeat` (1..32), `bevelSpace` (0..90%), `bevelInside`,
`bevelBothSides`. `threeD.inflate`: `depth` (points, default 0), `twist` (degrees, 0..360, 0), `taper` (1..100%, 100), `cap` (true
solid, false open behind), `volume` (0..100%, default 50), `bothSides`.
All three take `rotationX/Y/Z` (degrees, -180..180, default the Off-Axis Front position -21.88/-23.4/9.06),
`perspectiveAngle` and the lighting parameters as Revolve. See [three-d.md](three-d.md) for the Position presets,
the scope and what differs from the public documentation.


## Edit Colors and Recolor Artwork

`edit.colors.invert`, `edit.colors.toCMYK`, `edit.colors.toGrayscale`, `edit.colors.toRGB`, `edit.colors.saturate`,
`edit.colors.adjustBalance` and `recolor.apply` recolor everything inside the selection (or `ids`) in one undo step:
fills, strokes, text, gradient stops, gradient-mesh points, colorized image fills, embedded images (a recolored copy of
the image; linked images are left alone) and the tiles of pattern fills and strokes (a recolored copy saved as a new
pattern swatch, e.g. "Dots 2"; the original pattern is untouched). Colorized image fills are recolored when fills are in
scope, but their pixels are never recolored. `includeImages: false` leaves other images' pixels unchanged.
For `edit.colors.*`, `fill: false` leaves colorized image fills unchanged. `recolor.apply` always includes fills.
`includePatterns: false` leaves patterns out. The Blend commands also grade gradient meshes, which keep their shading.
`edit.colors.saturate` needs `intensity`, a number from -100 to 100 (a number outside the range counts as its nearest
end). Without one, or with a value that is not a number, it answers `` missing number `intensity` `` and changes nothing.

```json
{"name":"run_command","arguments":{"command":"recolor.colors","params":{}}}
{"name":"run_command","arguments":{"command":"recolor.apply","params":{"map":{"#ff0000":"#0055ff"},"includeImages":false}}}
{"name":"run_command","arguments":{"command":"edit.colors.adjustBalance","params":{"mode":"cmyk","m":-20,"k":10}}}
```

Recolor Artwork keys colours by identity, not hex: `recolor.colors` lists each colour's `key` (`"rgb 255 0 0"`,
`"cmyk 0 100 100 0"`, `"gray 40"`, `"lab 55 60 -40"`; RGB in 0–255 levels, CMYK and Gray in percent), so CMYK,
Lab and RGB colours that look alike stay apart, and every colour parameter also accepts a key.
`recolor.colors` and `recolor.reduce` read the objects in `ids`, else the selection. With nothing selected they read
the objects `select.all` takes, which leaves out hidden and locked art, and the selection is left unchanged; hidden
and locked objects inside a group are left out too, also in isolation mode.
`recolor.reduce {colors?, method?, preserve?, limitTo?, ids?}` groups those colours into rows of similar colours (k-means in Lab, weighted by use;
tints of a global swatch share its row; White and Black are preserved by default) and returns a map of rows
`[{from: [keys], to: key}]` to edit and pass to `recolor.apply {map, method?, model?, limitTo?, group?, groupColors?, rename?, globalAdjust?}`.
For scripts and MCP, `model` selects the replacement color model: `keep` (default) retains each
original color's model, `new` takes the replacement's model, and `document` converts replacements
and edited color groups to the document's model. A row's own `model` (the same values) takes the
place of the command's for that row's colors. Invalid values fail before editing. The Recolor
Artwork dialog gives a row `model: "new"` when its color sliders edit the row's new color in a color
mode. A returned `note` reports a model conversion in `keep` mode.

`method` says how a row's colours take its new colour: `exact`, `preserveTints` (tints of the row's darkest colour
stay tints), `scaleTints` (every colour becomes a tint as light, relative to the darkest), `tintsShades` (lighter and
darker than the row's average become tints and shades) or `hueShift` (the most saturated colour takes the new colour,
the others turn by the same hue). `limitTo` snaps new colours to a swatch library's nearest colour;
`recolor.randomize {map, order?, saturationBrightness?, seed?}` shuffles or varies new colours; a row with
`exclude: true` keeps its colours. With `limitTo`, a row's `tint` (a percentage, default 100) gives the art
linked to the global or spot swatch its new color is that share of its tint; `tint` must be a number. `recolor.randomize`
moves a row's `tint` with its new color and removes it from a color it changes, and moves a row's `model` with its new color. `swatch.editGroup {group, colors, rename?}` rewrites a colour group in one undo
step (art linked to its global swatches follows), and `recolor.apply` with `group` recolours the art and rewrites the
group together. Art linked to one of the group's global swatches stays linked when the swatch is rewritten with its
row's new color (the default without `groupColors` when the rows follow the group's order), so a later `swatch.edit` still
reaches it; the reply's `relinked` counts the other linked paints that took the group's new colors. Art in a row that
is linked to a swatch rewritten with another color keeps its row's new color, even an unchanged one, and is no longer
linked to the swatch, as does art whose color the row converts into the art's own color model. Without `group`,
the swatch keeps its color and the recolored art is no longer linked to it.

`globalAdjust: {saturation?, brightness?, temperature?, luminosity?}` is the dialog's Global Adjust: each amount is a
percentage from -100 to 100 (0 when left out), and all four at 0 change nothing. Every color that a row which is not
excluded recolors takes the same change after its row's new color, in this order: saturation multiplies HSB
saturation by 1 + saturation/100; brightness moves HSB brightness that share of the way to full brightness (above 0)
or to black (below 0); temperature adds 0.2 × temperature/100 to red and subtracts it from blue; luminosity moves Lab
lightness that share of the way to 100 or to 0 and keeps Lab a and b, so the hue stays. Each color keeps its own
model, and a changed color is no longer linked to its swatch. The colors a `group` is rewritten with take the same
change. With `limitTo`, each row's new color takes the change before it snaps to the library, so the art still takes
the library's colors. Colors that no row holds, such as preserved White and Black, and excluded rows stay as they are.

```json
{"name":"run_command","arguments":{"command":"recolor.apply","params":{"map":{"#ff0000":"#ff0000","#0000ff":"#0000ff"},"globalAdjust":{"saturation":-40,"temperature":25}}}}
```

The `recolor.*` commands refuse parameters of the wrong form. A `limitTo` or `method` that is not a string gets the
error for an unknown library or method. Other wrong-form parameters get an error that names them.
`includeImages`, `includePatterns`, a row's `exclude`, `order`, `saturationBrightness` and the `white`, `black` and
`grays` switches of `preserve` must be true or false; `colors` and `seed` must be whole numbers; `group` and `rename`
must be strings, `groupColors` must be an array, `preserve` must be an object, and `globalAdjust` must be an object
whose amounts are numbers from -100 to 100. Each refusal leaves the document unchanged and records no undo step. A `null`
counts as a value left out except for `recolor.apply`'s `model` and a `map` row's `model`, which must be strings when given.

```json
{"name":"run_command","arguments":{"command":"recolor.reduce","params":{"colors":2,"preserve":{"grays":true}}}}
{"name":"run_command","arguments":{"command":"recolor.apply","params":{"map":[{"from":["cmyk 0 100 100 0","cmyk 0 40 40 0"],"to":"cmyk 100 50 0 0"}],"method":"scaleTints"}}}
{"name":"run_command","arguments":{"command":"swatch.editGroup","params":{"group":"Brights","colors":["#ff0000","cmyk 0 0 100 0"]}}}
```

## Group, layer and type appearance

Groups and layers carry fills, strokes and effects of their own, as in the reference app: their fills and strokes
paint every member's geometry, and their effects apply to the members as one piece (one combined drop shadow; a
Transform or Warp moves or bends the whole group). Their Contents row (type: Characters) is a slot in the stack:
fills and strokes above it paint over the members (characters), those below under them; new items go above it
(on an image, below it).
`appearance.moveItem {"from": "contents", "to": n}` puts the row above the bottom `n` items, and an item move takes
`contents` to say where the row ends up in the same undo step; `document.node` shows the slot as
`appearance.contents_index`. The `appearance.*`, `effect.*` and `graphicStyle.apply` commands edit the selected
objects themselves (a group's own stack; layers through `ids`) or, with `target: "contents"`, the objects inside the
groups and layers. `appearance.targetContents` selects a group's members (double-clicking the Contents row).
SVG and PDF export bake a group's own fills, strokes and geometry effects into paths; its raster effects stay a
filter on the whole group in SVG and become an image of the whole group in PDF.
A geometry effect on one of type's own fills or strokes (`effect.apply` with `item`) reshapes the whole outline that
the fill or stroke paints, and SVG, PDF, EPS and the other vector exports write such type as paths. On type that
holds inline graphics, those effects are drawn and exported only when the type has a geometry effect or perspective
of its own.
An image's own fills and strokes draw only when they carry a visible Convert to Shape or Outline Object effect: Convert
to Shape sizes its shape from the image's box, Outline Object gives the image's frame, the effects after it reshape
that shape, and the effects before it are left out. Its Contents row stands for the image; `appearance.addFill` and
`appearance.addStroke` put a new item right below that row, so it paints under the image. SVG, PDF, EPS and the other
vector exports write those items as paths, and `effect.expandAppearance` turns them into objects in a group with the
image.
Use Preview Bounds measures an image by its frame.

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
swatch in the library panel does). The pattern library Geometric Patterns (`geometric-patterns`, category
`patterns`) holds pattern swatches drawn in code; adding one also adds its pattern, as pasting art painted with it does
(the same pattern the document has is reused, and a name another pattern or swatch has gets a number).
`swatch.resetDefaults {replace?}` brings back the missing default swatches.

```json
{"name":"run_command","arguments":{"command":"swatch.library.get","params":{"library":"earth-tones"}}}
{"name":"run_command","arguments":{"command":"swatch.library.add","params":{"library":"earth-tones","names":["Clay"]}}}
```
`addon.color.swatchLegend {names?, columns?, size?, gap?, values?, x?, y?}` draws a legend of colour swatches on the page as one
group, in one undo step, and selects it: a square chip per swatch (`size` pt, 72 by default, `columns` to a row, 4 by
default, `gap` pt apart, 12 by default) filled with the swatch and linked to it, labelled inside with the swatch's name
and its values (`values`: any of `hex`, `rgb`, `cmyk`, `lab`, `gray`; by default the swatch's own colour model; spot
swatches also say Spot), in black on light chips and white on dark ones. `names` picks swatches (default: every colour
swatch, in panel order); `x`, `y` place the top-left (default: the first artboard's, inset by `gap`). Object ›
Plug-ins › Color Effects › Create Swatch Legend… sets these in a dialog with a preview, and can limit it to the
swatches selected in the Swatches panel (#4919). → `{id, count}`

```json
{"name":"run_command","arguments":{"command":"addon.color.swatchLegend","params":{"columns":6,"values":["hex","cmyk"]}}}
```
`swatch.library.save {path?, format?: "vcswatches"|"gpl"|"ase"|"css", names?, name?, user?}` writes the document's
swatches as a library (`.vcswatches` keeps colour models, global, spot, gradients and colour
groups; `.gpl` is 8-bit RGB; a swatch exchange `.ase` file keeps solid colors in their own model
(RGB, CMYK, Lab or Gray) as global, spot or process colors, and color groups, writes a tint swatch
as the color it shows and leaves gradients out; CSS writes custom properties); without `path`
it returns `{data}`, or `{dataBase64}` for `.ase`, and `user: true` saves into the user library
folder of the desktop app (listed as category `user`, User Defined). The reply's `count` is the number of swatches
written: `.gpl` and `.ase` hold solid colors only, and CSS leaves freeform gradients out. `swatch.library.load {path? | data? |
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
These commands refuse a parameter of the wrong form with an error that names it: `library`, `apply`, `path`, `data`,
`dataBase64`, `format` and `name` must be strings, `names` must be a list of names, and `focus`, `replace` and `user`
must be true or false. `graphicStyle.saveLibrary` checks `path`, `name`, `names` and `user` the same way,
`graphicStyle.addFromLibrary` checks `name`, `names`, `apply`, `add` and `ids` before it adds a style, and `graphicStyle.loadLibrary` checks `path`, `data`, `dataBase64` and `name`. A `null` for one of these parameters counts
as a value left out.

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
file, or another document's graphic styles, as a library to add from (a `.ai` or EPS whose editing data holds graphic
styles and no layers gives the styles of its style list, in that order); a file in the user library folder, or a file
with the same extension and bytes as one there, loads as that User Defined library.

## Symbol libraries

Symbol libraries are read-only sets of symbols: one generated in code (Basic Shapes), the `.vectorcraft` files in
the user library folder of the desktop app (User Defined) and the symbols of any document loaded with
`symbol.loadLibrary`. `symbol.libraries` lists them (`id`, `name`, `category`: `builtIn`, `user` or `loaded`, `count`)
and `symbol.library {library}` returns one's symbols (`name`, `size`, `registration`, `exportType`; `library` is an
id or a name). `symbol.addFromLibrary {library, name? | names?}` copies symbols into the document's Symbols as one
undo step (no names: the whole library), with their Symbol Options and the resources they use: the symbols they nest,
patterns, swatches (`swatchConflict` as for `edit.paste`), brushes, graphic styles and images. Symbols the document
already has (same name and art) are reported under `existing`; a name another symbol has gets a number, and `names`
gives each one's name in the document. The first one becomes the current symbol. `place: true` also places an
instance of it with its registration point at `x`, `y` (default: the first artboard's centre) in the same step:
what dragging a symbol out of the library panel onto the canvas does.

```json
{"name":"run_command","arguments":{"command":"symbol.addFromLibrary","params":{"library":"basic-shapes","name":"Star","place":true,"x":120,"y":80}}}
```
`symbol.saveLibrary {path?, names?, name?, user?}` writes the document's symbols as a symbol library: a `.vectorcraft`
document holding only them and what they use; without `path` it returns `{dataBase64}`, and `user: true` saves into
the user library folder (category `user`, User Defined). `symbol.loadLibrary {path? | data? | dataBase64?, name?}`
loads the symbols of any document VectorCraft opens as a library to add from; a file it can't read is an error.

## Brush libraries

Brush libraries are read-only sets of brushes: four generated in code (Art Ribbons, Calligraphic Nibs, Pattern
Borders, Scatter Shapes), the `.vectorcraft` files in the user library folder of the desktop app (User Defined) and
the brushes of any document loaded with `brush.loadLibrary` (a document without brushes of its own has the default
ones). `brush.libraries` lists them (`id`, `name`, `category`: `builtIn`, `user` or `loaded`, `count`) and
`brush.library {library}` returns one's brushes (`name`, `type`; `library` is an id or a name).
`brush.addFromLibrary {library, name? | names?}` copies brushes into the document's Brushes as one undo step (no
names: the whole library), with the resources their art uses: patterns, symbols, swatches (`swatchConflict` as for
`edit.paste`), graphic styles and images. Brushes the document already has (same name and definition) are reported
under `existing`; a name another brush has gets a number, and `names` gives each one's name in the document. The
first one becomes the current brush. `apply: true` also applies it to the paths `ids` (default: the selected ones)
in the same step, as `brush.apply` does: what clicking a library brush with paths selected, or dragging one onto a
path, does.

```json
{"name":"run_command","arguments":{"command":"brush.addFromLibrary","params":{"library":"art-ribbons","name":"Striped Ribbon","apply":true}}}
```

`brush.saveLibrary {path?, names?, name?, user?}` writes the document's brushes as a brush library: a `.vectorcraft`
document holding only them and what their art uses; without `path` it returns `{dataBase64}`, and `user: true`
saves into the user library folder (category `user`, User Defined). `brush.loadLibrary {path? | data? |
dataBase64?, name?}` loads the brushes of any document VectorCraft opens as a library to add from; a file it can't
read is an error.

## Library panels

Each swatch, graphic style, symbol or brush library opens in a read-only panel of its own (#2603), as in Illustrator,
and several can be open at once. `window.swatchLibrary {library}`, `window.graphicStyleLibrary {library}`,
`window.symbolLibrary {library}` and `window.brushLibrary {library}`
(remote mode, `invoke_menu`) open one floating, or bring its panel forward when it is already open, and answer
`{open, panel, name, count}`; `library: null` closes every panel of that kind. The panel id is
`library:<kind>:<library id>` (`library:swatches:pastels`, `library:graphicStyles:hand-drawn`,
`library:swatches:loaded/C:/Palettes/Brand.ase`), and the panel commands take it like any other panel's:
`window.panel.float {panel, onto?, x?, y?}` groups or moves it, `window.panel.dock {panel}` docks it,
`window.panel.close {panel}` closes it. `window.swatchLibrary.other {path}`, `window.graphicStyleLibrary.other
{path}`, `window.symbolLibrary.other {path}` and `window.brushLibrary.other {path}` load a file and open it the same
way.

`ui.libraryPanel {panel, library?, view?, showFind?, persistent?}` sets one panel: `library` shows another library
of its kind in that panel (what Previous Library and Next Library do; a library open in a panel of its own is
brought forward instead), `view` is `smallThumb`, `mediumThumb`, `largeThumb`, `smallList` or `largeList`,
`showFind` shows or hides the find field, and `persistent: true` is the panel menu's Persistent: the panel
reopens when the app starts, with its view and find field, where the saved layout had it (or floating). A
library loaded from a file is read again from its path. When the file is gone, the panel still reopens and
stays in the preferences: its tab shows the file's name, its body shows `Missing:` with the file's path and a
Locate Library… button, and the status bar names the file. A persistent panel whose User Defined library's
file is deleted while the app runs stays open with the same tab and `Missing:` body; any other panel closes.
A closed persistent panel can still be set, so `persistent: false` stops it reopening. The reply is
`{panel, kind, library, view, showFind, persistent, open}`, and `ui.inspect` lists every panel's settings
under `ui.library_panels`.

`ui.libraryPanel.locate {panel, path?}` is Locate Library…: it loads the library at `path` (without one, it
asks for a file) as Other Library… does and shows it in the open panel `panel`, which keeps its view, find
field and Persistent, and answers as `ui.libraryPanel` does. When the file's library already has a panel open,
`panel` closes and is forgotten, and that panel is brought forward, made persistent when `panel` was. It
runs on any open panel whose library is missing, with Persistent on or off. On a panel whose library isn't
missing, it answers an error without asking for or loading a file, and the panel keeps its library.
`window.swatchLibrary` and `window.graphicStyleLibrary` also take a missing library's id, so a closed missing
panel opens again, and an open panel whose library is missing is brought forward with Persistent on or off;
they answer its file's name without the extension and `count: 0`.

```json
{"name":"invoke_menu","arguments":{"command":"window.swatchLibrary","params":{"library":"pastels"}}}
{"name":"invoke_menu","arguments":{"command":"ui.libraryPanel","params":{"panel":"library:swatches:pastels","view":"smallList","persistent":true}}}
{"name":"invoke_menu","arguments":{"command":"window.panel.close","params":{"panel":"library:swatches:pastels"}}}
```

## Libraries

The Libraries panel's libraries hold graphics, colours, gradients, patterns and character and paragraph styles that any
document can use.
They are kept on this machine: each is a `.vclibrary` file (JSON) named after the library (`CON-.vclibrary` for a
library named after a Windows device such as `CON`) in the `Libraries` folder next to the preferences
(none in the web app and headless sessions, where they last for the session). `library.list` lists them (`id`, `name`,
the count of each kind, `current`, `folder`); `library.get {library?}` returns one's items (graphics with `id`, `name`,
size and a PNG `thumbnail` in base64; colours with `name`, `hex` and `color`; styles with `name` and `attrs`).
`library.create {name?}`, `library.rename {library?, name}`, `library.delete {library?}` and
`library.setCurrent {library}` manage them; `library` is an id or a name, and without it the current library (the one
the panel shows) is meant.

`library.add {library?, kind, ids?, name?, color?, swatch?, group?, before? | after?}` adds from the selection:
`graphic` (a copy of the objects with the images, symbols, patterns, swatches, styles and brushes they use, as Copy
takes them), `fillColor` / `strokeColor` (the first selected object's solid colour, else the default one, or `color`),
`charStyle` / `paraStyle` (the selected text's attributes). The new item goes into `group` (by default into none) and,
with `before` or `after` (another item of its kind), just before or after that item, as `library.moveItem` places one;
art and swatches dropped among the panel's items are added this way. An item the library already has is reported as
`existing`, and stays where it is. `library.use {library?, kind, item,
center?, to?}` uses one: a graphic is placed centred on `center` (pasted with its resources; a swatch name the document
gives another colour merges into the document's), a colour paints the selection's fill or stroke, and a style is added
to the document under its name without the white space around it (numbered when that name is taken by other
attributes) and applied to the selected text, each as one undo step. `library.removeItem {library?, kind, item}` removes
one.

Gradients and patterns are kinds of their own (the panel's + button lists Fill Gradient, Stroke Gradient, Fill Pattern
and Stroke Pattern, and a gradient or pattern swatch dragged from the Swatches panel onto the list is added).
`library.add {kind: "fillGradient" | "strokeGradient", swatch?}` adds the first selected object's fill or stroke
gradient (else the default one, or the gradient swatch `swatch`) without its placement or swatch links, named after its
swatch, else `Gradient`; `library.use` paints the selection's fill or stroke with it, fitted to each object.
`library.add {kind: "fillPattern" | "strokePattern", swatch?}` adds the first selected object's fill or stroke pattern
(else the default one, or the pattern swatch `swatch`) with the images, symbols, patterns, swatches, styles and brushes
its art uses, and a thumbnail of one swatch's worth of it; `library.use` adds the pattern and its swatch to the document (the
same pattern there is reused, and a name the document gives another pattern gets a number) and paints the selection's
fill or stroke with it, as one undo step, and returns the pattern's name in the document as `pattern`. `library.get`
lists them as `gradients: [{name, gradient}]` and `patterns: [{name, pattern, thumbnail}]`.

Items can be put in user-named groups, which the panel shows as collapsible sections after the ungrouped items (its
search field filters every item by name; `library.get` lists them all to filter). `library.createGroup {library?,
name?, items?: [{kind, item}]}` makes one (a name another group has, ignoring case, gets a number) with those items
moved into it; `library.renameGroup {library?, group, name}` renames one; `library.deleteGroup {library?, group}`
deletes one, its items staying in the library ungrouped; and `library.moveItem {library?, kind, item, group?, before?
| after?}` moves an item into a group (without `group`, out of its group) and, with `before` or `after` (another item
of its kind), just before or after that item in the order its kind is listed and shown in. `library.moveGroup
{library?, group, to}` moves a group to place `to` of the list (0: first; past the end: last). Dragging items and
group headers in the panel runs these. An item is in one group at most; `library.get` lists the groups as `groups:
[{name, items: [{kind: "color"|"gradient"|"pattern"|"charStyle"|"paraStyle"|"graphic", item}]}]` (a graphic by its id), and each kind's
items in the order the panel shows them.

`library.export {library?, path?}` writes a library (its colours, gradients, patterns, styles, graphics with their art
and thumbnails, and its groups) as a `.vclibrary` file, the JSON the library folder keeps (without `path`: `{data}`, the file's text).
`library.import {path? | data? | dataBase64?, name?}` adds the library of such a file as a new library and makes it
the current one; a name another library has gets a number (`Brand (2)`). Imported files are checked: at most 10,000
items of each kind and 1,000 groups, names clipped and made unique, thumbnails kept only when they are small PNGs,
gradients with fewer than 2 or more than 1,024 stops dropped and the others' offsets, opacities and midpoints put in
range, and groups naming items that aren't there dropped. Files written before groups load as they are. Using a
pattern whose data is damaged fails with an error and leaves the document as it was.

```json
{"name":"run_command","arguments":{"command":"library.add","params":{"kind":"graphic","name":"Logo"}}}
{"name":"run_command","arguments":{"command":"library.use","params":{"kind":"graphic","item":"Logo","center":[300,200]}}}
{"name":"run_command","arguments":{"command":"library.createGroup","params":{"name":"Logos","items":[{"kind":"graphic","item":"Logo"}]}}}
{"name":"run_command","arguments":{"command":"library.export","params":{"path":"/tmp/Brand.vclibrary"}}}
```

## Strokes on type

`stroke.set` without an `item` gives type its characters' stroke: weight, cap, join, miter limit, `align` and the
dash options go to every run (the object keeps no stroke of its own); an `item` still edits an object-level stroke added
with `appearance.addStroke`, which takes every stroke option (dashes, profile, opacity, blend) like a path's and paints under or over the
characters as its place relative to the Characters row says. To stroke
some characters, use `text.setRangeStyle {id, start, end, strokeOptions: {weight?, cap?, join?, miterLimit?, align?,
dash?, dashOffset?, alignDashes?}}`. `text.setStyle {ids?, stroke?, strokeWidth?, start?, end?}` sets the stroke color
and weight of the characters of each object given (without `start` and `end`: all of its text), with its other keys in
one undo step; `strokeWidth` is in document points, as `stroke.set` gives the weight of scaled type. A `stroke` color
without `strokeWidth` gives characters that have no stroke weight 1 pt, as `paint.setStroke` does.
`text.setRangeStyle` does the same for its range, and its `strokeWidth` and `strokeOptions` lengths are in document
points too. The 1 pt default of both commands is in the type's own units, so on type scaled to 200% it draws 2 pt wide.
`align: "inside"|"outside"` keeps the characters' stroke on that side of the
glyph outlines, drawn at twice the weight and clipped, while the type stays editable (beyond Illustrator); SVG export
writes the text live and that stroke as glyph outlines over it. `inspect_document` reports each object's stroke as `strokeOptions` (type: its first
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
that split into many regions. The result reports `{ids, vector, rasterized, options}`. When an image in the art
can't be decoded, the image that replaces the art does not show it, and the result also carries `warning`: "an image
couldn't be decoded and is missing from the picture", the note raster exports give.

```json
{"name":"run_command","arguments":{"command":"object.flattenTransparency","params":{"ids":[12,15],"preset":"high"}}}
{"name":"run_command","arguments":{"command":"object.flattenTransparency","params":{"balance":0,"lineArtPpi":150}}}
```

## Pattern editing display

In pattern editing mode the tile edge (`pattern.options {showTileEdge}`) and the swatch bounds (`pattern.options
{showSwatchBounds}`: the part of the tiling the swatch repeats, dashed) are drawn in the preference
`patternTileEdgeColor`. There `object.exitIsolation` (the Exit Isolation Mode item of the canvas context menu and of the
Layers panel menu) saves the tile art into the pattern and leaves the mode, as `object.pattern.done` does.

`object.pattern.make {name?}`, `object.pattern.saveCopy {name?}` and `pattern.options {newName?}` trim the white space
around the name they are given. The first two number a name that a pattern, a swatch or a color group has ("Dots 2"),
and `pattern.options` refuses a name that another pattern, a swatch or a color group has. It takes a name that only the
pattern's own swatches have (a name Swatch Options gave the pattern's swatch, for example) when none of them still has
the old name. A rename gives the new name to the pattern's swatches that had the old one; any other swatch that has the
old name keeps it. A blank name gives "New Pattern N" in `object.pattern.make` and "`<pattern>` copy" in
`object.pattern.saveCopy`, and `pattern.options` keeps the name.

```json
{"name":"run_command","arguments":{"command":"pattern.options","params":{"showSwatchBounds":true}}}
{"name":"run_command","arguments":{"command":"prefs.set","params":{"key":"patternTileEdgeColor","value":"#ff4f4f"}}}
```

Pattern editing mode and opacity mask editing exclude each other. In pattern editing mode
`transparency.editOpacityMask` and `transparency.viewOpacityMask` are off, and `transparency.makeOpacityMask` refuses
one target, an object or a layer, which it would take into mask editing; a call returns an error that ends "finish
editing the pattern first". While a mask is edited, `object.pattern.edit` and `object.pattern.make` (also with
`edit: false`) are off, and a call returns an error that ends "stop editing the opacity mask first".

Pathfinder Options: `object.pathfinder.options {precision?, removeRedundantPoints?, removeUnpainted?}` (or
`prefs.get`/`prefs.set` with key `pathfinder`) reads and sets the options every later Shape Mode and Pathfinder
operation uses: `precision` (0.001–10 pt, default 0.01; a string with a unit also reads) is how closely results follow
the paths (they are refit to within it and pieces thinner than it are dropped), `removeRedundantPoints` takes the
anchors that don't change a result's shape off it, and `removeUnpainted` makes Divide and Outline leave out the pieces
of objects with no fill (or only hidden fills). Both default to false; `{}` reads the options. Each Shape Mode and
Pathfinder command (`object.pathfinder.unite` … `minusBack`) also takes `precision`, `removeRedundantPoints` and
`removeUnpainted` for one run; the values it used join its journal entry, so a replay or an action gives the same art
whatever the options are then.

```json
{"name":"run_command","arguments":{"command":"object.pathfinder.options","params":{"precision":0.1,"removeUnpainted":true}}}
```

Compound shapes (the Pathfinder panel menu, and Alt-clicking a Shape Mode button): `object.compoundShape.make
{mode?, ids?}` makes the two or more selected objects (paths, compound paths, groups, compound shapes, type, blends,
envelopes) the members of one live compound shape, each in shape mode `mode` (`"add"`, the default, `"subtract"`,
`"intersect"` or `"exclude"`). It draws, prints and exports as the area its members make from the backmost up (a blend
as its steps, an envelope as its distorted art, an object with a warp or another geometry effect as it draws, each
updated when the member is edited), painted with its own
appearance, which it takes from the topmost member not in Subtract (with every member subtracting, the backmost); a
fill or stroke given to it or to a member paints the compound shape. The members stay editable (Direct Selection,
the Layers panel, isolation mode). `object.compoundShape.setMode {mode, ids?}` sets the mode of the members the
selection is or is in, `object.compoundShape.release` gives the members back as they were, and
`object.compoundShape.expand` turns each compound shape into the path, or compound path, of its area. A saved file
keeps the compound shape (`compound_shape` on the group, `shape_mode` on each member); a mode the file doesn't name
reads as Add.

```json
{"name":"run_command","arguments":{"command":"object.compoundShape.make","params":{"mode":"subtract"}}}
{"name":"run_command","arguments":{"command":"object.compoundShape.expand","params":{}}}
```

The Eyedropper: `appearance.copyFrom {source}` copies what the Eyedropper Options pick up and apply from `source` to the
selection (`ids`); `reverse: true` copies the selection's attributes onto `source` (Alt-click), or with nothing selected
what the last copy without `reverse` picked up (a copy made then, kept for the session), and `append: true` adds
the source's fills and strokes on top of each target's stack (Shift+Alt-click). `defaultsOnly: true` copies the enabled
fill, stroke and weight into paint defaults without changing selected art; it cannot be combined with `ids`, `reverse`
or `append`. The Live Paint Bucket uses this for Alt-click sampling on vector art. On an image, it uses
`paint.sampleColor {color, stroke: false, ids: [], clearStroke: true}` to set a solid fill default from the sampled
pixel and clear the stroke default. `clearStroke: true` requires `stroke: false`, `ids: []` and no `stop`.
Alt+Shift-click samples only the color at the pointer into the active paint default through
`paint.sampleColor {color, ids: []}`. The options live in the preferences:
`eyedropper.setOptions {sampleSize?, pickUp?, apply?}` (or `prefs.get`/`prefs.set` with key `eyedropper`) reads and
sets them; a tree names flags under `appearance` (`transparency`, `fill` `{color, transparency, overprint}`, `stroke`
`{color, transparency, overprint, weight, cap, join, miter, dash}`), `character` and `paragraph`, and a bool for a
branch sets all of it. `paint.sampleColor {color}` puts a sampled color into the active proxy (in a CMYK document an
RGB color is stored as CMYK, as `paint.setFill` stores it). Clicking an image samples its pixels: a CMYK image (a
four-component JPEG, or an 8- or 16-bit chunky CMYK TIFF) gives its ink amounts as they are, and an RGB image's sRGB
pixel is taken into the working RGB space.

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
  too); `add` (Shift) adds it, or removes it when it's all selected. Alt-click on a layer's or sublayer's name runs
  `layer.selectAll {id}` and then `layer.setCurrent {id}`.
- **Eye and lock**: `layer.setProps {ids?, visible?, locked?, name?}` for any rows, plus Layer Options for layers:
  `template` (also locks the layer and dims its images to 50% unless given), `printable`, `preview` (off: the layer
  draws and is clicked in outline, Ctrl/Cmd-click its eye), `dimImages` (0–100, or false) and `color` (an index 0–26,
  `#rrggbb` or a preset name; a hex color with an alpha part, `#rrggbbaa` or `#rgba`, is an error, and so it is in
  `layer.new` and `layer.newSublayer`). Several rows change in one undo step (a drag down the eye or lock column).
- **Dragging rows**: `layer.move {ids, target, place?: above|below|inside, copy?}` moves rows beside a row or into a
  layer or group (on top of its contents), keeping their stacking order; `copy` (Alt-drag) moves copies. Layers go
  only in layers or at the top level and objects in layers and groups (an object placed beside a top-level layer goes
  inside it); a row never goes into itself or a row inside it, and a locked layer or group takes nothing. Dragging the
  selected-art square is `layer.move` with the selected objects. `node.move {id, parent?, index}` is the low-level
  form with the same nesting rules; unlike `layer.move`, it also moves a row into a locked layer or group.
- **Buttons**: `layer.new {name?, top?, below?, …options}` (above the current layer at its level; `top` with
  Ctrl/Cmd; `below`, directly below the current layer at its level, with Ctrl/Cmd+Alt),
  `layer.newSublayer {parent?, bottom?, name?, …options}` (on top of the parent's contents; `bottom`, under them but
  above a clipping layer's clipping path, with Ctrl/Cmd+Alt; both refused while an opacity mask is edited),
  `layer.delete {ids?}` (rows with what they hold; the
  last layer stays), `layer.locate {id?}` (highlights the selected object's row; the panel opens the rows around it)
  and `layer.clippingMask.toggle`.
- **Panel menu**: `layer.duplicate {ids?}`, `layer.merge {ids?}` (into the layer highlighted last, keeping the
  stacking order), `layer.flatten {id?}` (every other visible layer's art into one layer; hidden layers are deleted,
  templates stay), `layer.collectInNew {ids?}`, `layer.releaseToLayers {id?, build?}` and
  `layer.releaseToLayersBuild` (each object of a layer or group in a sublayer of its own; Build adds up copies),
  `layer.reverse {ids?}`, `layer.template {ids?, on?}`, `layer.hideOthers` / `layer.showAll`,
  `layer.outlineOthers` / `layer.previewAll` (Ctrl/Cmd+Alt-clicking an eye runs Outline Others for the row's layer, or
  Preview All Layers when the others are in Outline already), `layer.lockOthers` / `layer.unlockAll` (Alt-clicking an eye or a lock
  runs Hide or Lock Others for the row's layer, or Show or Unlock All when the others already are), `layer.pasteRemembersLayers`,
  `object.isolate {id?}` (a group, a layer or a single object, by default the first selected object; not type, not a
  hidden or locked object or one in a hidden or locked group or layer, not a guide path while Lock Guides is on, and
  while a symbol is edited only the editing layer or art on it) / `object.exitIsolation`.
- **Dialogs and panel state** (control channel and the app's MCP): `ui.layerOptions {ids?}` and `ui.newLayer
  {sublayer?}` open Layer Options (dialog `layerOptions`: name, color, template, locked, visible, printable, preview,
  dimImages, dimPercent; OK is one undo step; `ui.dialog.set` of `template` also sets `locked` and `dimImages`, as the
  checkbox does, except a Dim Images set before it; for several `ids` the dialog shows the first row's settings, and OK sends only the fields set with a
  control or `ui.dialog.set`; Cmd/Ctrl+Alt+L runs `ui.newLayer` for a new layer), `ui.layersPanelOptions` opens Panel Options (`layersPanelOptions`:
  layersOnly, locateSelected, rowSize small|medium|large|other, otherSize, thumbLayers, thumbGroups, thumbObjects), and
  `ui.layersExpand {ids?, open?}` opens or closes rows as their triangles do (Alt-click: everything inside).
  With `locateSelected` on (Locate Selected Objects, off by default), when the last selected object changes on the canvas
  or by a command, the panel opens the layers and groups around it and scrolls to its row, as after `layer.locate`, but
  does not highlight the row. When Show Layers Only, a search or the filter leaves the object without a row, the panel
  opens no rows for it and scrolls as with the option off. A change made with the panel's own rows, buttons or menu is
  not located, and a change made during a drag is located when the drag ends.
  A document opens with only its top-level layers open; the open rows are saved in the native file and reopen that way.
- **Lock and Hide in isolation mode**: Object › Lock's and Object › Hide's items (`object.lock`, `object.lock.above`,
  `object.lock.otherLayers`, `object.lock.unselected`, `object.hide`, `object.hide.above`, `object.hide.otherLayers`,
  `object.hide.unselected`) are on while an object inside the isolated object is selected, and they change only objects
  inside it. All Artwork Above locks or hides the selected objects as well as the objects above them that overlap them.
  Other Layers changes nothing in an isolated group, layer or sublayer, returns a count of 0 and records no undo step.
  `object.unlockAll` and `object.showAll` are on with nothing selected, and they unlock or show and select only the
  objects inside the isolated object. Pattern editing mode and opacity mask editing do not count as isolation mode for
  these commands.
- **Search and filter** (control channel and the app's MCP): the Search All field keeps the rows whose names hold the
  text typed, and `ui.layersFilter {objects?, layers?}` sets the choices of the Filter button beside it. `objects` (By
  Object) takes `paths` (paths and compound paths), `shapes` (live rectangles, ellipses, polygons and lines), `text`,
  `images`, `effects` (rows with an effect on themselves or on a fill or stroke, shown or hidden), `groups`,
  `linkedFiles` (linked images and placed documents) and `guides` (paths made into guides, and ruler guides); `layers`
  (By Layer) takes `locked` and `hidden` (the row's own lock and eye). A row stays when it is one of those kinds and in
  one of those states (an empty list takes every row) and its name holds the search text, or when a row inside it
  stays. A key left out keeps its list. The filter lasts the session and does not change the document, its open rows
  or the selection. `ui.layersFilter` answers `{objects, layers}`.

These Layers panel commands refuse a string, a number or another value that is not true or false with
`` `copy` must be true or false ``, naming the key, and leave the document, the selection, the highlighted rows and
the undo history as they were: `top` in `layer.new` and `layer.newSublayer`, `add` in `layer.selectAll` and
`layer.target`, `copy` in `layer.move`, `on` in `layer.template` and `layer.pasteRemembersLayers`, and `build` in
`layer.releaseToLayers`. A null is read as a key left out.

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
fill alone and each stroke a copy of the path stroked by that stroke alone, its weight, caps, joins, dashes, arrowheads,
alignment and width profile kept as stroke attributes (a brushed stroke: its brush art), grouped in paint order
under the object's id; each piece takes its fill's or stroke's opacity and blend mode, and the group keeps the
object's transparency and opacity mask. Geometry effects are baked; raster effects become an embedded image at the
document's raster effects resolution (shadows and outer glows under the art; a blur, feather or inner glow replaces
the object with the image). Type with a geometry effect (Outline Object included) or with fills and strokes of its own
is outlined; type that has no fills or strokes of its own and whose only effects are shadows or outer glows stays
type, with the image under it. A group's or layer's own fills and strokes become objects among its members, and its
members are expanded too. The result is `{ids}`; when an image in the art can't be decoded, the images made from
raster effects do not show it, and the result also carries the `warning` that Flatten Transparency gives.
A path's strokes stay strokes, as the
[Expand objects](https://helpx.adobe.com/illustrator/desktop/manage-objects/arrange-objects/expand-objects.html) help
page's two steps imply (Expand Appearance first, then Expand, whose Stroke option outlines strokes): `object.expand`
or `object.path.outlineStroke` outline them (#3089). Object → Expand… in the menus is dimmed while the selection has
an appearance Expand Appearance expands (`object.expand.info`'s `appearance`); `object.expand` itself still bakes
effects and outlines strokes in one step.

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
in gradient swatches. A spot stop separates on its plate. A linear or radial gradient whose stops are all tints of one spot ink (or
paper white, 0 %) exports to PDF as a Separation shading, and one that mixes spot inks with other spot, CMYK or gray
inks exports as a DeviceN shading. A gradient that mixes a spot ink with colors written in RGB or Lab is written in
process colors, with a warning; an export that converts colors to a CMYK destination profile writes RGB and Lab stops
in CMYK, and so does a CMYK document unless the export converts colors to an RGB destination profile. Freeform gradients and dithered fills export to PDF as images in process colors, and Method
above covers Perceptual gradients.

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

A symbol instance is measured by its symbol's art, placed by the instance's transform. The bounds in
`document.inspect` and `document.node`, the Align commands, `object.setBounds`, the bounding box, clicks, marquees,
Fit to Artwork Bounds and exports of the art use that box. So do `artboard.rearrange` and `artboard.move` (which
artboard the art moves with), `artboard.convertToArtboards`, `select.allOnArtboard`, `object.transformEach`,
`object.lock.above` and `object.hide.above`. `edit.copy` and `edit.cut` record the artboard under the center of that
box as the one the copy comes from, and `edit.pasteInPlace`, `edit.pasteInFront`, `edit.pasteInBack` and
`edit.pasteOnAllArtboards` keep the copy's offset to it. `edit.paste {center}`, `library.use {center}` and the
`clipboard.import*` commands center that box on the point, and `library.add` gives a graphic the size of that box.
`clipboard.exportPng`, `clipboard.exportPdf` and the PNG and PDF that Copy offers other apps are cropped to that box.
Visual bounds (Use Preview Bounds, Fit to Selected Art) take in the strokes in the art at their own weight, as the
canvas draws them when the instance scales the art. An instance is measured as the 20 × 20
square around the origin, which symbols made in the app keep their art in, when its symbol is missing,
when its symbol or a symbol nested in its art holds an instance of itself, and when symbols
nest 32 deep in its art. PDF and EPS export and the SVG of an artboard leave out art outside the page or artboard, and
Skip Blank Artboards skips an artboard with no art on it. For both, the box of an instance whose art holds brushed
strokes grows by 16 times the widest stroke (at least 1 pt) of the objects in its art that have a brushed stroke,
plus 8 pt. The SVG of an artboard also keeps an object whose effects paint onto the artboard, and an object that holds
an instance when the effects in the symbol's art, or the instance's own, paint onto it.

## Expand

`object.expand {object?, fill?, stroke?, gradient?, steps?}` (Object → Expand…) turns the selection into plain art in
one undo step. `object` (on by default) turns the symbol instances in the selection without effects (on the instance
or on its fills and strokes) and not in perspective, those in a placed document's art included, into their art as `symbol.breakLink` places it (the instances
that art holds stay instances; a hidden or locked instance's art stays hidden or locked), outlines type, turns live shapes into paths and bakes effects; `stroke` (on)
outlines strokes into filled paths, the stroked paths that baking effects leaves included (a group's own stroke among
them, #3089); `fill` (on) expands gradient fills: with `gradient: "objects"` (the default) into
`steps` (1–1000, default 255) solid objects (rectangles across a linear gradient, each from its band to the far end so
no seams show; concentric ellipses for a radial one, largest first; just the bands when stops are translucent), with
`gradient: "mesh"` into a gradient mesh that paints the gradient (columns or rings where its colour changes, sharp at
coincident stops), each inside a clip group shaped like the object. A freeform gradient becomes a mesh shaped like the
object either way. Expanded colours keep the colour model their stops share. Expand mixes a freeform gradient's point
colors in the model they share to color the mesh points; it mixes C, M, Y and K as inks. When the models differ, the
colors mix in display RGB and are stored in the working RGB space, and a CMYK document stores them as CMYK through the
color settings. `fill` also expands pattern fills: each
becomes the pattern's tiles that reach the object's box, placed by the pattern's transform, in a clip group shaped like
the object that isolates their blending, with the fill's opacity, blend mode and effects; the object's other fills
and strokes stay on copies of it in paint order. The tiles' stroke weights and effect distances scale with the
pattern, and opacity masks in the tile art, unlinked ones included, move with each tile. When the pattern is rotated,
reflected, stretched or sheared, or scaled while its art has geometry effects (Roughen's detail is per inch), the
strokes (brushed ones included) and geometry effects of the tiles' paths become filled art and their live shapes
become plain paths, in their opacity masks too. A path in the tile art with a Transform effect set to Random, or with
a brush that varies at random (a calligraphic or scatter brush with Random variation, or a bristle brush), becomes
filled art first, so every tile shows the one result the canvas draws. A pattern fill stays as it is when it lays no
tiles there (as when the box would take more than 250,000 tile positions), when a group or type in its art has a
Transform effect set to Random, when the pattern is scaled, rotated, reflected, stretched or sheared and its art holds
a symbol instance (the canvas scales the instance's strokes with the tile), and when the pattern is rotated,
reflected, stretched or sheared, or scaled while its art has geometry effects, and its art has raster or 3D effects,
or a geometry effect on a group or type, which the canvas draws turned or scaled with each tile. The geometry effects of a gradient or pattern fill (such as Offset Path)
and those of the object shape the outline of the clip group, the gradient fits that outline and the tiles reach its
box, as the canvas paints the fill; the expanded art does not keep them. With `fill: false` gradient and pattern fills
stay live. `object.expand.info` answers which options have something to expand in the selection
(`{object, fill, stroke, gradient, appearance, symbolArt?}`; `gradient` says whether Fill has a gradient fill, which Expand
Gradient To applies to; `appearance` whether the selection has appearance attributes Expand Appearance expands first,
for which Object → Expand… is dimmed in the menus; `symbolArt: {fill, stroke, gradient}`, present when the selection
holds symbol instances that `object` expands, whether only their art holds it, which Fill and Stroke reach only with
`object` on), and `object.mesh.create` on a gradient-filled object colours the
mesh points as the gradient paints them, with the colors Object › Expand gives them.
`object.expand`, `effect.expandAppearance`, `type.createOutlines` and `object.path.outlineStroke` turn a dimension whose
art they change into a plain group of that art, in the same undo step (see [dimensions.md](dimensions.md)).

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

## SVG interactivity

The SVG Interactivity panel (`window.panel {panel: "svgInteractivity"}`) reads `svgInteractivity.info {ids?}`:
`events` (`[{event, script}]` in the panel's order, null where the objects differ), `files` (the linked JavaScript
files) and `eventNames` (the 19 events the panel lists, from `onfocusin` to `onscroll`).
`svgInteractivity.setEvent {event, script, ids?}` sets the JavaScript an event runs on the objects, replacing the
event's script where they have one (an empty `script` removes it); `svgInteractivity.removeEvent {event, ids?}` and
`svgInteractivity.clearEvents {ids?}` remove one event or all of them. Each is one undo step.
`svgInteractivity.addFile {file}` and `svgInteractivity.removeFile {file}` link and unlink a JavaScript file (a path
or a URL, kept as given without surrounding spaces); a document links at most 64 files. `ui.javascriptFilesDialog {}`
opens the JavaScript Files dialog, whose Choose… puts a picked file in as a `file:///` URL. SVG export writes each event
as an event attribute on the object's element (`onclick="…"`; with the SVG Tiny 1.2 profile it leaves them out with a
warning) and each file as `<script xlink:href>`. SVG import reads the event attributes the panel lists and the
`<script>` links back as text; it never reads a script file or runs a script, and leaves inline scripts and the links
after the first 64 out with a warning. An unnamed `<g>` with event attributes of its own stays a group, apart from the object in
it. The native format keeps the events and the files.

```json
{"name":"run_command","arguments":{"command":"svgInteractivity.setEvent","params":{"ids":[12],"event":"onclick","script":"showMenu()"}}}
{"name":"run_command","arguments":{"command":"svgInteractivity.addFile","params":{"file":"menu.js"}}}
```

## SVG filters

Effect › SVG Filters lists the document's SVG filters below Apply SVG Filter… and Import SVG Filter…; choosing one
applies it to the selection. `svgFilter.list {}` returns them as `filters: [{name, xml}]`. Each is one `<filter>`
element whose `id` is its name, with 1 to 64 primitives and at most 64 KB of XML (`feTurbulence` octaves up to 10,
`feConvolveMatrix` kernels up to 9 cells a side, `feMorphology` radii and `kernelUnitLength` up to 100 and
`stdDeviation` up to 500 user units, elements nested at most 16 deep). It holds only filter primitives,
what they hold, and `desc` and `title`: no scripts, no event attributes, and images only as PNG, JPEG, GIF or WebP
`data:` URLs of at most 4096 × 4096 pixels. A document
holds at most 64. `svgFilter.define {xml, replace?}` adds
a filter after the others, or with `replace` replaces the filter of that name, which keeps its name; a name another
filter has is refused, and the same filter again records no undo step. `svgFilter.delete {name}` removes one, and `svgFilter.import
{path?|svg?}` adds the `<filter>` elements of an SVG file (at most 16 MB, not SVGZ, the first 256 filters) or of SVG text, leaving out the names the
document already has and the filters `svgFilter.define` would refuse (`{added, skipped}`). Each change is one undo
step. `svgFilter.apply {name, ids?|id?, item?}` applies one as the live effect `svg.filter`, as `effect.apply` does,
in one undo step. The effect (`{name, xml, scale, origin}`) names the document's filter: editing the filter changes
what every object that uses it draws, in the edit's one undo step, and an application after an edit takes the edited
filter. Deleting a filter that objects use leaves their effects in place, still named, drawing nothing (their `xml`
empty) until a filter of that name is defined or imported again. Opened from such an effect, Apply SVG Filter shows the
name above the list, not selectable. `effect.apply {effect: "svg.filter", params: {xml}}` takes a filter
directly and names it by its `id`; an `xml` that isn't one such filter is an error. `ui.svgFilterDialog {}` opens Apply
SVG Filter (New, Edit, Delete, Import…), and from the Appearance panel it edits which filter an applied effect runs;
`ui.importSvgFilter {}` picks an SVG file and imports its filters.

The canvas, raster export, printing and PDF draw the filter with the object as its source graphic, its user space
starting at `origin` (`[x, y]` in points, `[0, 0]` by default) with its units `scale` points (1 by default), its
object bounding box the drawn object's, and at the Document Raster Effects resolution when the view is finer. The
filter runs on coarser pixels when a morphology radius or kernel unit would span more than 32 of them, or a standard
deviation more than 512. An effect whose XML these rules refuse (from a file or a script) is not drawn or exported. The
filter is drawn and written rebuilt from its filter elements and their filter attributes alone, without comments,
text, descriptions or other attributes. An image in a filter is drawn only when its bytes are a PNG, JPEG, GIF or WebP
within the pixel limit. A filter region in user space given in percentages is measured against the object's box (x
and y from its top-left corner), and one in em or ex or left out takes the default region of that box (10% beyond
each side); these are VectorCraft's own rules. Scale Strokes & Effects scales
`scale`. SVG export writes the filter as a `<filter>` element when it is the object's last effect, its only pixel
effect and `scale` is 1 (with a region measured against the object's box, its only raster effect), with the object in a
group that places the filter's user space and such a region written in user units. The filter is written in SVG 1.1
terms: `feImage`'s reference as `xlink:href`; each `feDropShadow` as the blur, offset, flood, composite and merge that
[Filter Effects](https://www.w3.org/TR/filter-effects-1/#feDropShadowElement) gives as its equivalent; colors as
`#rrggbb` with a flood color's alpha in `flood-opacity`; `image-rendering` as `optimizeSpeed` or `optimizeQuality`;
the inputs `BackgroundImage`, `BackgroundAlpha`, `FillPaint` and `StrokePaint` as `SourceGraphic`, and an input that
names no earlier result as the previous primitive's result;
and an `feComposite` operator SVG 1.1 lacks, or `edgeMode` on a primitive other than `feConvolveMatrix`, left out, as
the canvas draws them. A filter with an `feBlend` mode SVG 1.1 lacks (`overlay` and the others), an `feDropShadow`
whose `flood-color` is `currentColor` or with `primitiveUnits="objectBoundingBox"`, or more than 64 primitives once
its shadows are expanded, has no SVG 1.1 form. Neither does a filter whose written form, with its region and id, is
one SVG import would not read, such as one longer than 64 KB. In that case, otherwise, and with the SVG Tiny 1.2 profile, the object
becomes an image. SVG import keeps a filter that the live effects (Drop Shadow, the
glows, Gaussian Blur, Feather) can't express as an `svg.filter` effect holding the filter's XML, with `scale` and
`origin` from the object's user space, unless that user space is turned, skewed or scaled past 0.001 to 1000 points
to a unit, or the filter is styled by the document's style sheets: such a filter is left out with a warning. The native format keeps the document's filters and the effects.

```json
{"name":"run_command","arguments":{"command":"svgFilter.define","params":{"xml":"<filter id=\"Soft\"><feGaussianBlur stdDeviation=\"2\"/></filter>"}}}
{"name":"run_command","arguments":{"command":"svgFilter.apply","params":{"name":"Soft","ids":[12]}}}
```

## Removing anchor points

`path.removeAnchors {}` (Object › Path › Remove Anchor Points) removes the direct-selected anchors (`select.anchors
{id, anchors: [[subpath, anchor]…], mode?}`) without opening their paths, in one undo step, and answers
`{removedObjects}` (paths left with no segment go). Each removed point's neighbours keep their handle directions and
their facing handles are refitted so one cubic follows the two old segments; two straight sides become one.
The anchor after each removed one is then selected (the one before at an open path's end), so running
`path.removeAnchors` again goes on along the path.
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
(the Pen's join, #776: drawing on from one open path, a click on another's end makes them one path). `path.averageJoin {to: "corner"|"smooth", ids?}`
(Average & Join, Cmd+Alt+Shift+J) moves two ends to their midpoint and joins them there, the joined anchor a corner or a smooth point, in one
undo step: the two direct-selected anchors when they are open path ends, else the nearest ends of the two open paths in `ids` or the
selection (both ends of one, closing it); anything else is refused. `path.convertAnchor {id, subpath?, anchor, to, x?, y?}` and
`path.split {id, subpath?, anchor}` do the same to one anchor (the Anchor Point and Scissors tools); the Pen with Alt
held over a selected path's handle, anchor or segment works as the Anchor Point tool. `press_key` Escape between two
`pointer_gesture` calls of an Anchor Point drag cancels it: the path goes back and no undo step is added. `path.reshapeSegment {id,
subpath?, segment, t, dx, dy, perpendicular?}` (a segment dragged with Direct Selection or the Anchor Point tool) moves the segment's
point at `t` by `dx`, `dy`: a smooth anchor at either end stays smooth, its other handle turning with the moved one.
With `perpendicular: true` (Shift held during the drag) both of the segment's handles stand at right angles to the
line between its anchors, on the same side and at the same length, and the point at `t` lies as far from that line
as the point moved by `dx`, `dy` does; a segment whose anchors coincide bends as it does without `perpendicular`.
`path.reshapeSegment` refuses a delta of 1e9 pt or longer ("invalid delta"), as `object.blend.spine.reshapeSegment`
does. `path.convertAnchor` (its `x`, `y`) and `path.freehand` (each of its `points`) refuse a point whose x or y is
1e9 pt or more in size ("x and y must be finite"). The canvas check after an edit measures a segment of a selected
path by its control points when one of them has an x or y that is infinite or larger in size than `f64::MAX / 16`
(about 1.1e307), so an edit that leaves such a path selected, `path.create` included, fails with "result would exceed
the canvas (coordinates out of range)".
`path.curvatureEdit`, `path.removeAnchor`, `path.convertAnchor`, `path.reshapeSegment`, `path.split`,
`path.setHandle`, `path.moveAnchor` and `path.insertAnchor` edit subpath 0 when `subpath` is absent or null, and refuse a `subpath` that
is not a whole number of 0 or more, such as 1.5, -1 or "1" ("missing integer `subpath`"); `path.split` refuses such
an `anchor` or `segment` in the same way. The document does not change and no undo step is recorded.

These commands refuse a string or another value that is not a number with `` `dy` must be a number, not "10" ``,
naming the key; the document does not change and no undo step is recorded: `t`, `dx` and `dy` in
`path.reshapeSegment` and `object.blend.spine.reshapeSegment`; `x`, `y` and `t` in `path.curvatureEdit`; `x` and `y`
in `path.convertAnchor`; `t` in `path.insertAnchor`; `fidelity` and the `within` of `redraw` in `path.freehand` (and in
`brush.freehand`, which runs it); `size`, `sizeVariation`, `angle` and `roundness` in `path.eraseRegion`;
`size`, `sizeVariation`, `angle`, `roundness` and `fidelity` in `path.blob`; `width` in `path.eraseSegments`;
`radius` and `fidelity` in `path.smoothRegion`; `tolerance` in `path.joinScrub`; `dx` and `dy` in `path.moveAnchors`;
and `dx`, `dy`, `tol` and `radius` in `path.reshape`. A null is read as a key left out. `path.convertAnchor` refuses
an `x` without a `y`, or a `y` without an `x`, with "x and y must be given together". `path.freehand`,
`path.curvature`, `path.knife`, `path.eraseRegion`, `path.eraseSegments`, `path.smoothRegion`, `path.joinScrub` and
`path.blob` refuse a point in `points` whose x or y is not a number, such as `["10", 10]` or a point written as a
string, with "x and y must both be numbers". All of them except `path.curvature` refuse a pen pressure, tilt or
rotation that is neither a number nor null with `` `pressure` must be a number, not "0.5" ``, naming the key
(`pressure`, `tiltX`, `tiltY` or `rotation`). `path.close` refuses an `in` that is not `[x, y]`
with `` `in` must be [x, y] ``.

These commands refuse a string, a number or another value that is not true or false with
`` `closed` must be true or false ``, naming the key; the document does not change and no undo step is recorded:
`independent` in `path.close` and `path.setHandle`; `patterns` in `path.moveAnchors`; `closed`, `keepSelected` and
`fill` in `path.freehand` (and in `brush.freehand`); `closed` and the `corner` of a point in `path.curvature`; `corner`
in `path.curvatureEdit`; `perpendicular` in `path.reshapeSegment`; and `keepSelected`, `mergeOnlySelection` and
`merge` in `path.blob`. A null is read as a key left out.

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

## Simplify

`object.path.simplify` takes the amount as `tolerance` (points, 1 by default) or as the Simplify slider's `level`, a whole
number from 0 (the fewest anchor points) to 100 (the closest fit). Level 50 is a tolerance of 1 pt, and every 10 levels
halve or double it, so level 0 is 32 pt and level 100 is 1/32 pt. `auto: true` (Auto-Simplify) uses the lowest level
whose tolerance is at most 1/400 of the selection's longer side, or level 100 when that side is under 12.5 pt. The
result and the original path stay within `tolerance` of each other, except that a closed subpath that refits to one curve
is dropped, so a small circle in a compound path can be removed at a large `tolerance`. A segment that would need more
than 65,536 samples at that tolerance stays as it is, and so does a segment after a run's first 16,777,216 samples or
2,097,152 result segments. With `straightLines`, such a segment becomes evenly spaced lines, up to 65,536, and past those
limits the lines can be farther than `tolerance` from the segment. With `level` or `auto`, the reply also has the `level`
used:

```json
{"name":"run_command","arguments":{"command":"object.path.simplify","params":{"auto":true}}}
```

With segments of a path direct-selected (`select.segments`, or `select.anchors` with both anchors of a segment), Simplify
refits only each run of consecutive selected segments, between the run's end anchors: they stay where they are, with
their handles on the side away from the run, and every other anchor and segment of the path stays exactly as it was. A
closed path's run may cross its start point; the path then starts at the run's end anchor. Separate runs on one path
simplify independently, in one undo step, and the runs stay direct-selected (as segments when segments were picked).
A path with only lone anchors selected, and every path with `wholePaths: true`, simplifies whole:

```json
{"name":"run_command","arguments":{"command":"select.anchors","params":{"id":12,"anchors":[[0,20],[0,21],[0,22],[0,23]]}}}
{"name":"run_command","arguments":{"command":"object.path.simplify","params":{"auto":true}}}
```

In the desktop app, Object › Path › Simplify opens the slider (`ui.menuDialog {command: "object.path.simplify"}`,
dialog `simplifySlider` with field `level`) at the Auto-Simplify level for the selection and previews the result until
`ui.dialog.confirm`. `ui.simplifyAuto` moves the open slider back to the Auto-Simplify level, and `ui.simplifyMoreOptions`
opens the Simplify dialog at the slider's tolerance. When OK confirms the dialog with `retain` ticked, the menu item opens
the dialog on those settings next time.

## Smooth

`object.path.smooth` smooths the selected paths (or the objects in `ids`), compound path members included, as one undo
step. `level` is the Smooth slider's, a whole number from 0 (minimum smoothing: the path stays within 0.1% of its longer
side) to 100 (maximum smoothing); `auto: true` (Auto-Smooth) uses level 50, the default. Each path is smoothed along
its length (by up to 5% of its longer side; none at level 0) and refitted with curves within 0.1% to 2.5% of it,
keeping a corner only where it turns back on itself, so bumps, jags and kinks become one curve; with anchor points or
segments of a path direct-selected (and no `ids`), only the sections between them are smoothed, as Simplify does. A refit never has more anchor points than the path had, so a shape whose corners the tolerance can't
round, such as a rectangle, stays as it is. Closed paths stay closed and open paths keep their end points. The reply has the anchor counts `before` and `after` and the `level` used:

```json
{"name":"run_command","arguments":{"command":"object.path.smooth","params":{"level":80}}}
```

In the desktop app, Object › Path › Smooth opens the slider (`ui.menuDialog {command: "object.path.smooth"}`, dialog
`smoothSlider` with field `level`) at the Auto-Smooth level and previews the result until `ui.dialog.confirm`;
`ui.smoothAuto` moves the open slider back to the Auto-Smooth level.

## Editing a path with the Curvature tool

The Curvature tool edits any selected path, whatever drew it, keeping its shape except where edited (#798). Each edit
is one undo step and leaves the anchor it edited direct-selected (the tool's current point):
`path.curvatureEdit {id, subpath?, op, …}` answers `{anchor}`. `op: "move"` with `anchor`, `x`, `y` moves a point and
re-curves only its two segments: a smooth point takes the tool's handles (along the circle through it and its neighbours; the one handle into
the curve at an end), and the neighbours' handles keep their directions (their lengths scale with the chord), so the segments beyond don't change.
`op: "insert"` with `segment`, `t` adds a smooth point there without changing the shape (with `x`, `y` it is then
moved there: the tool's click-and-drag on a segment). `op: "extend"` with `x`, `y` goes on from the `end` (default)
or the `start` of an open subpath (the new end curves into the path unless `corner` is true), and `op: "close"` closes it from that end; `from` says how the old end bends into
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
document's with `useDocument`, set by `document.setup {bleed}`; the art there is kept; a PDF/X file with no bleed on a
side where the document's bleed has one is written as asked and comes back with a warning) and its MediaBox the bleed
box grown to hold the marks (the art is clipped to the bleed box). Trim marks, registration targets and the page
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
the document's CMYK profile, RGB as sRGB, grey with the sRGB tone curve (PDF/A, PDF/X-3 and PDF/X-4 files are
always tagged, whatever `profiles` says). `outputIntent` embeds a profile as the file's
`/GTS_PDFX` output intent with `outputCondition`, `outputConditionId` and `registry`, and `trapped` sets `/Trapped`
(PDF/A files keep their own output intent). Type is written as selectable, searchable text in embedded subset fonts
with a ToUnicode map (fonts whose licence forbids embedding stay outlines, with a warning); `advanced: {outlineText:
true}` writes every glyph as an outline instead. The hyphen that ends a line broken inside a word (`hyphenate`, or a
soft hyphen) is written as text where the line draws it: the character `-`, or U+2010 when the font gives the hyphen's
glyph to U+2010 and not to `-`. Kashidas are drawn as paths. Type that Transform effects only move and copy (no scale, rotation,
reflection or Random) is written as text once in each place when its characters are in solid colors without underline
or strikethrough, the type object has no fills or strokes of its own, no inline graphics and no perspective, its
transform does not scale or shear stroked characters, and the copies do not overlap. When the effect is on the type
itself, the type can also have transparency, an opacity mask and raster effects such as a drop shadow. When the effect
is on the group or layer holding the type, the type has no transparency or effects of its own and is not in a clipping
path, the group or layer has no fill, stroke or Pathfinder effect of its own, and no group between them has a fill,
stroke, Pathfinder or geometry effect. Type is written as outlines when the effect stops its copies early, or when the
copies would number more than 10,000 or take more than 20,000,000 bytes of document data. Other type under a geometry
effect is written as outlines.

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
`object.transformAgain` applies the last transform's matrix with the `copy`, `strokes`, `corners`, `patterns` and
`typeAreas` values that transform used, and a true or false value given in its own params takes precedence. After a
drag of art without a pattern, `patterns` follows the preference `transformPatternTiles`. After a Direct Selection or
Group Selection drag that moved direct-selected anchors (`path.moveAnchors`), it moves the selected anchors and the
objects selected whole by the same distance. Its journal entry records the `strokes`, `corners` and `patterns` it
used, as a transform's does.

```json
{"name":"run_command","arguments":{"command":"object.scale","params":{"sx":200,"strokes":true}}}
{"name":"run_command","arguments":{"command":"object.transformEach","params":{"scaleH":50,"scaleV":50,"strokes":false}}}
```

## Transform Each: Relative and Absolute

`object.transformEach` scales by percentages of each object's size (the dialog's Relative, the default), or with
`relative: false` to the width and height each object becomes: `scaleH` and `scaleV` in pt, as the dialog's Absolute.
Absolute is measured on each object's own geometric bounds, so objects of different sizes come out alike about their
own reference points. A side left out keeps its size, a side the object doesn't have (a line's height, say) keeps its
factor 1, and a scale of 0 is refused in either mode, as it collapses the object. `object.transformAgain` repeats the
Absolute target, so it leaves objects that already have that size as they are; repeat a scale with Relative instead.

```json
{"name":"run_command","arguments":{"command":"object.transformEach","params":{"relative":false,"scaleH":50,"scaleV":50}}}
```

## Align to Pixel Grid

`object.setPixelAligned {ids?|id?, on?}` turns the pixel-aligned property of the objects given (default: the selected
objects) on or off (`on` defaults to true), as the Transform panel's Align to Pixel Grid checkbox does (described on
the help page,
<https://helpx.adobe.com/illustrator/desktop/get-started/learn-the-basics/pixel-preview-mode-for-web-graphics.html>).
It returns `{ids}`, the objects whose property changed. While the property is on:

- The object's horizontal and vertical straight segments sit on whole points, or on half points when a stroke centered
  on the path has an odd weight, so that the stroke's edges fall on the pixel grid. A stroke that is hidden or has no
  paint does not count. Its stroke weights are whole points, at least 1.
- Every move, scale, rotation, reflection, shear, Transform Each, Align or Distribute move (of objects or of
  direct-selected anchors), Transform panel change, move of direct-selected anchors or of one anchor
  (`path.moveAnchor`), new anchors from `path.setAnchors`, live shape change (`object.setLiveShape`), move of a
  compound path's path on its own, and artboard move or resize that carries the art (`artboard.move` with
  `moveArt`, `artboard.setProps` with art, Rearrange Artboards) snaps those segments again, as do the art copies of `artboard.move` with `copy` and Duplicate
  Artboards, the copies that `edit.duplicate` and the paste commands move, and `object.slice.move` of an object slice,
  and so does every `stroke.set`, `stroke.weightUp` and `stroke.weightDown` edit, which also rounds the new weight. An
  appearance edit that adds, removes, hides, repaints or reweighs a stroke snaps them again too:
  `appearance.setItem`, `appearance.addStroke`, `appearance.removeItem`, `appearance.clear`,
  `appearance.reduceToBasic`, `appearance.showAllHidden`, `appearance.transfer` (both objects), `appearance.copyFrom`
  (the Eyedropper), `paint.setStroke` and `graphicStyle.apply`, in the same undo step as the edit, as is every member
  of an active Global Edit that such an edit or move reaches. Every other command that changes a pixel-aligned object
  (for example Object › Path › Average) snaps it again as part of its undo step. Each anchor that such a segment shares moves with its handles, so diagonal segments and curves keep their shape apart from those
  anchors. A rotated shape with no horizontal or vertical segment is left as it is. Pixel-aligned art of a linked
  opacity mask snaps when its object moves.
- Coordinates that differ stay at least 1 pt apart within a subpath, so a narrow shape does not collapse. A
  coordinate on a whole point snaps to the half point above it and one on a half point to the whole point below it, so
  hiding and showing an odd stroke returns the path to where it was. A placed gradient and the art of a linked opacity
  mask move with the snapped anchors, and so do pattern tiles when the move transforms patterns. A pixel-aligned member
  of an Objects on Path group snaps again after the group places it on its path.
- A live rectangle, polygon, line or pie is moved and scaled as a whole to the snapped bounds, and stays live. A full
  ellipse has only curves and is left as it is. A path with Live Corners snaps the outline its corners are cut from and keeps
  its Live Corners. A compound path snaps each of its paths with the compound path's stroke.
- Make Compound Path from paths or compound paths that include a pixel-aligned one gives a pixel-aligned compound path
  and snaps it; its paths do not keep the property, and a path inside a compound path cannot take it alone (an error).
  Release Compound Path gives each released path the property of the compound path. A path pasted into a compound
  path, alone or from a pasted compound path, drops its own property and snaps with the compound path.

Turning the property on snaps the objects at once; turning it off keeps their geometry. These rules are VectorCraft's
own: only paths and compound paths can be aligned, and type, images, groups, guides and other objects are an error
that leaves the document as it was. The change is one undo step, and when every object already has the value nothing
changes and no undo step is made. The property is saved in the native file (`pixel_aligned` in `document.node`), and
files saved without it read back with it off.

```json
{"name":"run_command","arguments":{"command":"object.setPixelAligned","params":{"on":true}}}
{"name":"run_command","arguments":{"command":"object.setPixelAligned","params":{"ids":[12],"on":false}}}
```

## Live Corners

`object.setLiveShape {id?, ids?, radius?, kind?, corners?, items?}` sets the corners of any path (one undo step): a live
rectangle's or polygon's, a star's, a pen path's. A corner is an anchor where two sides, straight or curved, meet at an angle
(not an open path's ends, not a smooth anchor, not one the sides run straight on through); a curved side is trimmed to
what the cut leaves of it. `radius` (pt) and `kind`
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
rounds the corners whose widgets show (every corner, or the Direct-Selected ones) on every selected path at once,
outlining in red those that reach their limit; the Selection tool shows the widgets of live rectangles and polygons,
the Direct Selection tool those of any path. Several objects each with their own corners are one call:
`items: [{id, corners?}]` in place of `ids` and `corners`. Alt-clicking one cycles their kind and double-clicking one opens Corners (`ui.corners {id?, corners?}`,
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

## Align to Artboard

`object.align {to: "artboard", artboard?}` aligns the selected objects to artboard `artboard` (a 0-based index).
Without `artboard`, or with an index the document doesn't have, it aligns them to the artboard under the center of the
selection, or to the first artboard when no artboard is under the center. With `to: "selection"` or `"key"`,
`object.align` does not use `artboard`. `select.allOnArtboard {artboard?}` (Select › All on Active Artboard) selects
the objects on artboard `artboard`, or on the first artboard when the call gives none; an index the document doesn't
have is an error.

`object.distribute {to: "artboard", artboard?}` and `object.distributeSpacing {to: "artboard", artboard?}` (the
Distribute Objects and Distribute Spacing buttons with Align to Artboard) use the same artboard as `object.align`.
`object.distribute` moves the first object, in the order of the line it spaces (left edges, centers or right edges; top
edges, centers or bottom edges), until its left or top edge is on the artboard's left or top edge, and it moves the
last object until its right or bottom edge is on the artboard's right or bottom edge. It spaces the other objects
evenly by that line between them, and two objects are enough. `object.distributeSpacing` without `spacing` puts the
left or top edge of the first object, in the order of those edges, on the artboard's left or top edge, and it puts the
right or bottom edge of the last object on the artboard's right or bottom edge, with equal gaps between the objects.
With `to: "selection"` or `"key"`, or without `to`, `object.distribute` and `object.distributeSpacing` without `spacing`
keep the first and last objects where they are, and `object.distribute` without `spacing` moves nothing with fewer than
three objects. With `spacing` and a key object, `object.distribute` and `object.distributeSpacing` do not use `to`;
with a key guide, `to` decides whether the guide is the key. With anchor points selected (Direct Selection), both commands space the points
among themselves and do not use `to` or `artboard`.

`object.distribute {spacing}` (the Distribute Objects buttons while the selection has a key object, `select.key`) keeps
the key object where it is, with the compound path or selected group that contains it. It places the objects' lines
(left edges, centers or right edges; top edges, centers or bottom edges) `spacing` points apart, outward from the key
object's line, in their order on that line. With `spacing: 0`, every object ends on the key object's line. Two objects
are enough. Without a key object or key guide, or with anchor points selected, `object.distribute` does not use
`spacing`. A `spacing` that is not a number is an error.

`object.distribute` and `object.distributeSpacing` space the selected ruler guides across the axis (vertical guides
for the horizontal options, horizontal guides for the vertical ones) with the selected objects, as items with no size
(#1597). A guide's left edge, center and right edge are all its line, so the three options of an axis give the same
result for it. Guides along the axis stay where they are. Two or more guides selected without objects are enough, and
the Align panel's buttons are enabled for them; with `to: "artboard"`, the outermost items go to the artboard's edges.

With a key guide (`select.key {guide}`, #2143), `object.distribute {spacing}` and `object.distributeSpacing {spacing}`
(the Align panel's Distribute buttons send the Distribute Spacing value then too) keep the guide where it is and
space the other items from it, as from a key object with no size: `object.distribute` places their lines `spacing`
points apart, outward from the guide's line, and `object.distributeSpacing` leaves gaps of `spacing` points, the guide
included. With `spacing: 0`, `object.distribute` puts every line on the guide. A key guide along the axis leaves
everything where it is, as `object.align` does. `to: "selection"` or `"artboard"` distributes without the key guide
as the key, as with `object.align`, and without `spacing` a key guide is spaced as any selected guide.

Each button of the Align panel's Align Objects, Distribute Objects and Distribute Spacing rows is also a UI command
that takes no params and runs what a click on the button runs: `align.horizontalLeft`, `align.horizontalCenter`,
`align.horizontalRight`, `align.horizontalVerticalCenter`, `align.verticalTop`, `align.verticalCenter`,
`align.verticalBottom`, `align.distributeVerticalTop`, `align.distributeVerticalCenter`,
`align.distributeVerticalBottom`, `align.distributeHorizontalLeft`, `align.distributeHorizontalCenter`,
`align.distributeHorizontalRight`, `align.distributeVerticalSpace` and `align.distributeHorizontalSpace`. The Align
commands send `to: "key"` while the selection has a key, else `"artboard"` for one selected object without selected
anchor points, else the Align To choice. The Distribute commands send `to: "key"` while the selection has a key, else
the Align To choice, and while there is a key object or a key guide they send the panel's Distribute Spacing value as
`spacing`. Edit › Keyboard Shortcuts lists them under Align, with no default shortcut, and
`shortcuts.set {id, shortcut}` gives one a shortcut.

Edit › Keyboard Shortcuts and `shortcuts.list` leave out a command with no default shortcut when a key press cannot
run it: no item of the menu bar runs it, it opens no dialog when run without params, and it is one of these:

- a query with a menu path, such as `swatch.unused`;
- a command with a menu path whose params doc has a required field (one with no `?` and no `=` default), such as
  `document.setUnits {units}` (File › Document Setup › Units);
- a command that sets nothing or fails without a param: `transparency.set`, `transparency.setOpacityMask`,
  `stroke.set`, `layer.setProps`, `object.align`, `object.distribute`, `swatch.delete`, `swatch.move`,
  `swatch.spotOptions`, `edit.colors.adjustBalance`, `swatch.library.load`, `graphicStyle.loadLibrary`,
  `symbol.loadLibrary`, `brush.loadLibrary`, `dataset.import` and `variable.loadLibrary`.

They list such a command while the user has changed its shortcut (`overridden: true` in `shortcuts.list`), and
`shortcuts.set` can change or clear it. When every menu bar item that runs a command gives it the same params, a
shortcut for that command runs what a click on the item runs: Object › Path › Clean Up…, Print Ink Fixes… and Edit ›
Find and Replace… open their dialogs.

Each installed filter plug-in's Object › Plug-ins item is its own row, with id `plugin:<id>` (the plug-in's id), its
menu label and no default shortcut, so two filters with the same name have two rows. `shortcuts.set {id:
"plugin:<id>", shortcut}`, `shortcuts.list` and Import Shortcut Set use that id, `shortcuts.exportText` writes one line
for the row under its menu path and label, and the key runs what a click on the item runs, `plugin.dialog {id}`. A
removed plug-in's row goes away and its key runs nothing; its shortcut stays saved and applies again when a plug-in
with the same id is installed.

`shortcuts.exportText {path?}` saves the current set's shortcut list as a UTF-8 text file, as Edit › Keyboard
Shortcuts › Export Text… does with the dialog's set as edited so far. The file starts with a "Keyboard Shortcuts" line
and a "Set:" line with the set's name, followed by a Tools section and a Menu Commands section. Each section lists the
rows Edit › Keyboard Shortcuts lists, in its order and with names as it shows them: one line per row, with the name, a
tab and the shortcuts as the dialog shows them on the current platform. A row with no shortcut ends after the tab.
Without `path` a save dialog asks for the file. A `path` that is not a string, or a file that cannot be written,
returns an error, and nothing is written. The layout is VectorCraft's own. The reference app's [Export keyboard
shortcuts list as a text document](https://helpx.adobe.com/illustrator/desktop/get-started/preferences-and-settings/export-keyboard-shortcuts-list.html)
page describes saving the list from the Keyboard Shortcuts dialog as a text file, and does not describe its layout.

```json
{"name":"run_command","arguments":{"command":"shortcuts.exportText","params":{"path":"/Users/me/Shortcuts.txt"}}}
```

The desktop app adds its active artboard, the one the Artboards panel highlights, to the `object.align`,
`object.distribute`, `object.distributeSpacing` and `select.allOnArtboard` calls it runs without `artboard`, as it does
for `edit.pasteInPlace`, `edit.pasteInFront` and `edit.pasteInBack`, and to each `command.batch`. A batch passes its
`artboard` on to each of these steps that names
none, when the step runs. When the document a step runs in has no artboard at that index (for example, after an
earlier step deleted the active artboard when it was the last one), the step gets that document's last artboard. In
remote mode, MCP's `run_command` calls and each step of the `command_batch` tool get the active artboard too. The
Actions panel plays an action as a `command.batch`, omitting steps whose playback checkmarks are
cleared. An action set saved as `.vcactions` keeps each excluded step with `excluded: true`; loading
an earlier version 1 set without that flag includes every step. The Actions panel menu's Batch… runs
`action.batch`: the same steps on every readable file in a folder (or every data set of the open document), with
destination `none` | `saveAndClose` | `folder` and Override Open / Save / Export. With a Data Sets source,
`saveAndClose` saves the open document over its own file after each data set, so the file keeps only the last one,
and a document that was never saved, was converted from an older version or was restored by Data Recovery is
refused before a data set is selected, unless a recorded Save or Save As step (not in the template format) writes a
file and makes it the document's own first. When `destinationFolder` is set and Override Save points a recorded Save or
Save As step into it, that step makes each data set's file the document's own, and `saveAndClose` saves each data
set to its own file. The result carries `warning` when every data set saves over the same file:
with `saveAndClose` without such a step, or with a Save, Save a Copy, Save as Template or export step (PDF, EPS, DXF, Save for Web and
Save for Office Documents included) that writes a file and that the batch does not point into `destinationFolder`. The dialog shows the same
warning. For a Data Sets source,
`datasetFileNaming?: fileNumber | fileDataSetName | dataSetName` chooses File + Number (`source001`),
File + Data Set Name (`source_Alpha`), or Data Set Name (`Alpha`). An omitted parameter keeps File + Data Set Name
for existing callers; the dialog starts with File + Number. A folder source ignores this parameter. The folder
destination and overridden Save/Export paths use the chosen stem with the output format's extension. An empty data
set name under Data Set Name falls back to the document stem plus its three-digit ordinal. Batch writes `/`, `\`,
`:`, `<`, `>`, `"`, `|`, `?`, `*` and control characters in the data set's name as `-`, and adds the next free `-2`,
`-3` suffix to a stem already used in the batch (compared without case). An output already in a destination folder
gets the next free suffix only when the batch may read and write that folder before it starts. A save in VectorCraft,
template, AI or AIT format counts artboard file names when the document has any artboard; a per-artboard export
counts them when the document has several artboards.
Under Data Set Name, VectorCraft makes Windows device names safe (`CON` becomes `CON-`). This is
VectorCraft's naming behavior. Caps: 500
files or data sets, 8 folder levels, 256 steps. An action recorded in the Actions panel leaves the artboard out,
and its steps of these commands get the artboard that is active when the action plays. A press with the Selection,
Direct Selection or Group Selection tool inside an artboard makes that artboard the active one; with the Selection
tool, a Shift-click on an empty spot inside an artboard makes it active and keeps the selection. Choosing the Artboard
tool (`tool.select {tool: "artboard"}`) starts it on the active artboard: its `active` option is that artboard's index.
A command that starts the Artboard tool afresh in the same document (`file.revert`, or a `command.batch` or
`action.batch` that reverts the document or switches to another document and back) keeps the active artboard and
starts the tool on it. When a later step of the batch chooses the tool's artboards (a Paste or Cut), the artboard the
tool then has active becomes the active one.

`artboard.fitToArt {index? | indices?}` (Object › Artboards › Fit to Artwork Bounds) fits each artboard it names to
the art on that artboard, in one undo step. The art is what an export draws, measured with its strokes and with the
effects on paths, compound paths and symbol instances. An object counts for every artboard its bounds overlap, also
when it reaches past the artboard's edge, and a group or a clipping layer counts whole. An artboard with no art on it,
or a locked one, keeps its size and place. The call fails with `the artboard is locked` when every artboard it names is
locked, and with `no artwork` when none of the others has art. When every artboard already fits its art, no undo step
is recorded.
`artboard.fitToSelection {index?}` (Object › Artboards › Fit to Selected Art) fits artboard `index` to the selected
art. Both commands take 0-based indexes and resize the first artboard when the call names none. The desktop app adds
the selected artboards (those the Artboard tool has selected, else the active one) as `indices` to the
`artboard.fitToArt` calls it runs without `index` or `indices`. It adds its active artboard as `index` to the
`artboard.fitToSelection` calls it runs without `index`. Both include MCP's `run_command` calls in remote mode. The
Artboards panel menu's Fit to Artwork Bounds also passes the selected artboards. A batch passes its `artboard` on to
each of their steps that names no `index`, as the step's `index`. When the document a step runs in has no artboard at
that index, the step gets that document's last artboard. An action recorded in the Actions panel leaves `index` and
`indices` out of these steps, and they resize the artboard that is active when the action plays.

```json
{"name":"run_command","arguments":{"command":"object.align","params":{"horizontal":"left","to":"artboard","artboard":1}}}
{"name":"run_command","arguments":{"command":"object.distribute","params":{"horizontal":"left","to":"artboard","artboard":1}}}
{"name":"run_command","arguments":{"command":"select.allOnArtboard","params":{"artboard":1}}}
```

## Align to Glyph Bounds

Align and Distribute normally measure type by its em box (point type) or its frame (area type). With the preference
`alignGlyphBoundsPoint` on (`prefs.set {key: "alignGlyphBoundsPoint", value: true}`; also the Align panel flyout's
Align to Glyph Bounds › Point Text), `object.align`, `object.distribute` and `object.distributeSpacing` measure point
type by the ink of its glyphs instead: lowercase "ace" aligned to the top of a rectangle lines up the tops of its
letters, not the space above them that the em box keeps for capitals and ascenders. `alignGlyphBoundsArea` (Area Text)
does the same for area type. The two are independent and off by default; type on a path, type that draws nothing and
type inside a selected group are measured as before. With Use Preview Bounds on, the glyphs' ink takes in the
characters' strokes. [Align text](https://helpx.adobe.com/illustrator/desktop/design-with-text/edit-format-text/align-text.html).

```json
{"name":"run_command","arguments":{"command":"prefs.set","params":{"values":{"alignGlyphBoundsPoint":true,"alignGlyphBoundsArea":true}}}}
{"name":"run_command","arguments":{"command":"object.align","params":{"vertical":"top"}}}
```

## Use Preview Bounds

With the preference `usePreviewBounds` on (`prefs.set {key: "usePreviewBounds", value: true}`; also the Align panel
flyout's Use Preview Bounds), the Transform panel's and Control bar's X, Y, W and H, the bounding box, and
`object.align`, `object.distribute` and `object.distributeSpacing` measure visual bounds, which take in the whole
stroke (see Stroke bounds and clicks); off, they measure the paths. The three Align commands also take
`bounds: "preview"|"geometric"` for one call. `object.setBounds` then sets the visual box: with Scale Strokes &
Effects off a 100 pt wide rectangle with a 10 pt stroke set to `width: 220` gets a 210 pt path.

With Use Preview Bounds on, these measures take in each object as it is drawn after its geometry effects. A path is
measured by its effected shape, and a fill or stroke with geometry effects of its own is measured by the shape it
paints. Type, images, live objects and symbol instances are measured by their reshaped art, and a symbol instance is
also measured by its reshaped art when its symbol's art has geometry effects. A group or layer with geometry or
Pathfinder effects, or with a fill or stroke of its own that has geometry effects or paints members with geometry
effects, is measured by its evaluated art. A blend, envelope or repeat whose objects have geometry effects is
measured by its evaluated art, with those objects measured after their effects. A 100 pt square without a stroke,
with Offset Path 10, measures 120 pt, and type with Outline Object is measured by its glyph outlines. Raster effects such as shadows and glows are not
measured. In one object, the art of at most 4,096 symbol instances is measured with its geometry effects, and the
other instances are measured without them. `object.setBounds` with `width: 100` gives that type a glyph box 100 pt
wide.

```json
{"name":"run_command","arguments":{"command":"object.align","params":{"horizontal":"left","bounds":"preview"}}}
```

`object.align` needs `horizontal` or `vertical`. The three Align commands, `object.reflect` and `object.shear` return
an error that lists the choices of a `horizontal`, `vertical`, `to`, `axis` or `bounds` param given another value
(`object.reflect` also takes an angle and names the value given), and one for a `spacing` that is not a number; nothing
moves.

## Selection preferences

The Selection & Anchor Display and General preferences apply to `pointer_gesture` as they do to the mouse:

- `selectionTolerance` (1–8 px, 3 by default): how near a click must be to a path to pick it, and to an anchor,
  handle or segment for Direct Selection, the Anchor Point, Add and Delete Anchor Point and Scissors tools, and the
  Pen with Alt. A handle shorter than that leaves its anchor to be picked when pressed nearer the anchor.
- `objectSelectionByPathOnly`: a click inside a filled path or compound path doesn't select it, one on its path does.
- `ctrlClickSelectsBehind` (on by default): a Selection tool click with `mods: {cmd: true}` (Command on macOS, Ctrl
  elsewhere) selects the object under the selected one there, the next such click the one under that, then the
  topmost again. Cmd held to borrow the selection tool from another tool clicks as usual.
- `doubleClickToIsolate` (on by default): a `doubleclick` with the Selection tool isolates a group, a compound path or a
  single path, image, mesh or placed file (not type, blends, envelopes, repeats or symbol instances); off, it no longer does.
- `usePreciseCursors`: the Pen, Eyedropper, Slice and Blend tools' pointers are a crosshair.
- `snapToPointTolerance` (1–8 px, 2 by default): with View › Snap to Point on and Smart Guides off (desktop app), the
  point a selection is dragged by, a drawn point and a transform tool's reference point land on an anchor or a ruler
  guide that near.
- `moveLockedWithArtboard`: `artboard.move {moveArt: true}`, the Artboard tool and `artboard.rearrange` move locked and
  hidden art, and art that is also on a locked artboard, with the artboard too, and `artboard.setProps {scaleArt: true}`
  scales it too; off (the default) it stays where it is. `artboard.duplicate`, `artboard.copy` and `artboard.cut` copy or
  cut locked and hidden art only with it on. A `lockedAndHidden` param on any of these commands takes its place.
- `penRubberBand`, `curvatureRubberBand` (on by default): off, the Pen and Curvature tools draw no preview segment to
  the pointer.
- `showHandlesMultipleAnchors` (on by default): off, Direct Selection (and the Anchor Point tool, and the Pen with Alt)
  shows and drags direction handles only while a single anchor is selected. On, the handles of the direct-selected
  anchors, or of every anchor of a path selected as a whole, show and drag. `handleStyle` (`solid`, `hollow`, `large`) draws their ends (desktop app).
- `hideCornerWidgetAbove` (177° by default): corners wider than this show no Live Corners widget (a rectangle's right
  angles hide below 90°).
- `transformPatternTiles` (off by default): the default of the transforms' `patterns` param (`object.transform`,
  `object.move`, `object.rotate`, `object.scale`, `object.reflect`, `object.shear`, `object.transformEach`,
  `object.align`, `object.distribute`, `object.distributeSpacing`, the Selection and transform tools, the dialogs'
  Transform Patterns) and of the same param on the commands that move or duplicate art with an artboard (`artboard.move`
  and the Artboard tool, `artboard.duplicate`, `artboard.rearrange`, `artboard.setProps`): pattern fills and strokes
  move with the art. `object.nudge` (the arrow keys) and `path.moveAnchors` (a Direct Selection drag of selected
  anchors, and a Direct Selection or Group Selection drag from an object while anchors are selected) take the same
  param, with the same default, for the objects they move whole.
- `selectSameTintPercent` (off by default): `select.same.fillColor`, `strokeColor` and `fillAndStroke` take every tint
  of a global or spot swatch; on, only the same tint.

```json
{"name":"run_command","arguments":{"command":"prefs.set","params":{"key":"objectSelectionByPathOnly","value":true}}}
{"name":"pointer_gesture","arguments":{"tool":"selection","mods":{"cmd":true},"events":[{"kind":"down","x":175,"y":125},{"kind":"up","x":175,"y":125}]}}
```

The preferences for the view need the desktop app (`vectorcraft-cli mcp --connect`): `zoomWithMouseWheel` (the
wheel zooms about the pointer, Shift-wheel scrolls up and down, Cmd/Ctrl-wheel sideways; the control channel's
`ui.wheel` turns the wheel), `trackpadRotateView` (on by default: a two-finger rotation on a Mac trackpad, in the
macOS app or in Safari, turns the view about the pointer once it reaches 5°; with it off, the gesture does not turn
the view), `zoomToSelection` (on: Zoom In and Zoom Out centre the selection), `showToolTips`,
`anchorSize` (1–7), `gridColor`, `gridStyle`, `gridsInBack`, `guideColor`, `guideStyle`, `showPixelGrid` (on by
default: in View › Pixel Preview at 600% zoom and above, a line at every document pixel over the art),
`recentFontsCount`, `antiAliasedArtwork` (on by default; off, the canvas draws the art with hard edges — a pixel is
painted when the art covers at least half of it — while raster effects and pattern tiles stay smooth; exports keep
their own Anti-aliasing option),
`scrubNumericFields` (on: a horizontal drag on a numeric field's label steps the field, one undo step per drag; the
control channel's `ui.drag` scrubs), `showHomeScreen` (on by default: the Home screen while no document is open;
off, an empty window, and the Home button or `app.home` still shows the screen), `autoCollapseIconPanels` (off by
default: on, a click away from a panel popped out of the icon column puts it away), `displayPrintSize` (off by
default: View › Actual Size / `view.actualSize` is one document point per screen point; on, one document inch fills
96 screen points, an inch at the system's reference density whatever the display scaling). The tool shortcut
preference `holdToolKeys` also needs the desktop app. It is on by default: a single-key tool shortcut held for 0.25 s
or more, or held while its tool is used, chooses the tool until the key is let go and then returns to the tool active
before, after any drag in progress; a quicker press chooses the tool for good. With it off, every press and every key
repeat chooses the tool. `highlightAlternateGlyphs` (off by default: on, the canvas paints a pink cell behind glyphs OpenType features
substituted — ligatures, contextual forms — on screen; Document Setup's Highlight Substituted Glyphs stays
separate). `highlightSubstitutedFonts` (on by default: the canvas paints pink behind text drawn in a substitute for a
missing font, in every document). `blackOnScreen`
(Appearance of Black › On Screen, `accurate` by default) sets how the canvas shows pure CMYK black, black ink alone
at 100%: `accurate` shows it through the CMYK profile, a dark grey, and `rich` shows it as RGB black. Rich blacks,
black tints and images show the same either way, and exports are unchanged. `blackOutput` (Appearance of Black ›
Printing / Exporting, `accurate` by default) does the same for RGB raster exports (`document.export`,
`document.serialize`, `document.exportSelection`, `document.exportForScreens`, `document.exportForOffice`) and for
Print as Bitmap: `rich` draws pure CMYK black as RGB black in the pixels; CMYK ink exports and the document's
colours stay as they are. `antiAliasedBitmaps` (off by default;
desktop app): in View › Pixel Preview, on softens placed and embedded bitmaps; off, their pixels stay hard-edged.
`disableAutoAddDelete` (off by default): on, the Pen no longer adds or deletes an anchor when a click lands on a
segment or an anchor of a selected path.

`ctrlWheelScrollsSideways` (on by default): on, Cmd/Ctrl-wheel scrolls the canvas sideways instead of zooming while
`zoomWithMouseWheel` is off; off, Cmd/Ctrl-wheel zooms about the pointer. Alt + Cmd/Ctrl-wheel and a pinch on a Mac
trackpad, on a touch screen or in the web build still zoom about the pointer. With `zoomWithMouseWheel` off, once a
trackpad gesture holds Alt, it zooms until it ends, also after Alt is let go. The control channel's
`ui.wheel` with `cmd: true` turns the wheel with Cmd/Ctrl held.

`gridlineEvery` (72 pt by default) and `gridSubdivisions` (8 by default) set the document grid that View › Show Grid
draws, and View › Snap to Grid snaps to its subdivisions. Changing either sets the grid of every open document.
`file.new` gives a new document that grid, and `document.open`, `file.newFromTemplate` and `file.revert` give it to
every document they read except a native file and the native document that an SVG, PDF, `.ai` or EPS file carries
(`restored: true`), which keep the grid they were saved with.

`view.rotate {angle}` (desktop app) turns the view to `angle` degrees about the center of the canvas; a positive angle
turns the art clockwise on screen. View › Rotate View and the status bar's angle menu run it. It returns `{angle}`, the
angle it set, normalized to (-180, 180], and `view.rotateReset` turns the view back to 0.
`view.rotateToSelection` (desktop app, View › Rotate View to Selection) turns the view about the center of the canvas to
the angle the selected objects share (`selectionRotation` in `document.inspect`, 0 when their angles differ), so their
bounding box is square to the window. It returns `{angle}`, and with nothing selected it returns "nothing selected".

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
- `spacingGuides` (on by default): a moved selection (Selection tool / `pointer_gesture` with `selection`) snaps so
  the gap to a neighbour matches the equal spacing among other objects nearby; off, only alignment and construction
  guides pull.
- `distanceGuides` (Invoke Distance Guides, on by default): with Smart Guides on, something selected and the Selection
  tool hovering with `mods.alt` (a `pointer` event of kind `move`), the canvas measures the selection's bounds to the
  object under the pointer (the gap between their facing edges on an axis on which they lie apart, else the offsets
  of their two edges on it) or, over no object, to the left, right, top and bottom edges of the artboard under the
  pointer: measured lines labelled in the General unit. Off, Alt-hovering shows none.
- `snapToLastLocation` (Snap to Last Location, off by default): with Smart Guides on, a selection moved by dragging
  (Selection, Group Selection or Direct Selection tool / `pointer_gesture` with `selection`, Shift too) goes back to
  where the move began while the move is no longer than `snappingTolerance`, labeled "last location"; farther, it
  snaps as before. An Alt copy is not pulled back onto its original, and with View › Snap to Grid or Snap to Pixel on
  the move snaps to the grid or to whole pixels instead. A value that is not a boolean is refused.

The Snap to Glyph options (the desktop app's Character panel menu › Show Snap to Glyph Options; View › Snap to Glyph
is the desktop app's): `view.snapToGlyph.options {baseline?, xHeight?, bounds?, proximity?, angular?, anchors?}` (or
`prefs.get`/`prefs.set` with key `glyphSnap`, a partial object) reads and sets which guides Snap to Glyph gives, all on
by default: the baseline and the x-height of a single line of type, the glyph bounds (each glyph's edges and corners, turning with rotated type), the
proximity guides at both edges of each character's width, the angular guides (along a glyph's slanted strokes, and
every guide of rotated type, which runs at its angle) and the glyphs' anchor points. `{}` reads them; a key it doesn't
know or a value that isn't a boolean is refused and changes nothing.

```json
{"name":"run_command","arguments":{"command":"view.snapToGlyph.options","params":{"angular":false,"anchors":false}}}
```

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

## Type preferences

- `placeholderText` (on by default): type the Type tools place (`pointer_gesture` with `type`, `areaType`…) starts with
  placeholder text, selected, so `type_text` replaces it; `text.create` and `text.createInPath` take `placeholder: true`.
- `typedLists` (on by default; Type › Start Lists From Typed Markers): a list marker and a space typed with the Type tool
  at the start of a paragraph start a list (see `text.startList`). Off, the Type tool leaves them as typed text;
  `text.startList`, `text.setFormat {list}` and the Bullets and Numbered List buttons work as before.
- `typeSizeIncrement`, `trackingIncrement`, `baselineShiftIncrement`: what `type.step {attribute: "size" | "leading" |
  "tracking" | "kerning" | "baselineShift", by?}` steps by, on the text range given, the type objects in `ids`, the
  Type tool's selected text or the selected type objects. `type.size.increase` / `type.size.decrease` (Cmd+Shift+. and
  Cmd+Shift+,) step the size. `type.size.increaseByFive` (Cmd+Alt+Shift+., `press_key {key: ".", mods: {cmd: true,
  alt: true, shift: true}}`) and `type.size.decreaseByFive` (Cmd+Alt+Shift+,) step it by five times `typeSizeIncrement`.
  The Type tool's Alt+arrows (`press_key {key: "Right", mods: {alt: true}}`) step kerning at
  a caret or tracking of the selection (←/→ in horizontal type, ↓/↑ with or without Shift in vertical type),
  leading (↑/↓ in horizontal type, ←/→ in vertical type), and baseline shift (Shift+↑/↓ in horizontal type,
  Shift+→/← in vertical type). Mixed selected text objects use the horizontal mapping; Cmd/Ctrl makes five steps.
  `type.resetTracking` (Cmd+Alt+Q, `press_key {key: "Q", mods: {cmd: true, alt: true}}`) sets tracking to 0 and
  clears manual kerning in the type objects in `ids`, the Type tool's selected text or the selected type objects,
  keeping Optical; at a caret
  it resets only the kerning of the character before it. `type.resetHorizontalScale` (Cmd+Shift+X,
  `press_key {key: "X", mods: {cmd: true, shift: true}}`) sets the horizontal scale to 100%, as the Character panel
  shows it, in the same text; at a caret it leaves the text as it is. In the app, `type.highlightKerning` (Cmd+Alt+K),
  `type.highlightTracking` (no default key) and `type.highlightFontName` (Cmd+Alt+Shift+F) show the Character panel and
  give its Kerning or Tracking field the keyboard focus with the value selected, or open its font family menu with the
  search field focused; each replies `{field: "kerning" | "tracking" | "fontName"}`. Without selected type or text the
  Type tool edits, the panel shows and the command fails.
  The UI command `type.underline` (Cmd+Shift+U,
  in remote mode `press_key {key: "U", mods: {cmd: true, shift: true}}` or `invoke_menu`) does what the Character
  panel's Underline button does: it underlines the Type tool's selected text or the selected type objects, or removes
  the underline when the text the panel shows is underlined, and returns whether the text is now underlined.
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
- `hyphenationExceptions` (empty by default): Preferences › Hyphenation › Exceptions. A comma-separated list of words,
  matched without regard to case: a word with no hyphens is never hyphenated (`typography`), and a word with hyphens
  breaks only where they are (`hap-pen`). `prefs.set` pushes the list into the hyphenator and reflows open documents.
- `highlightAlternateGlyphs` (off by default): Preferences › Type › Highlight Alternate Glyphs. On, the canvas paints
  pink behind OpenType substitutions (ligatures, contextual forms). Document Setup's Highlight Substituted Glyphs
  stays separate. Agents set it with `prefs.set`; the desktop canvas reads it on the next paint.
- `highlightSubstitutedFonts` (on by default): Preferences › Type › Highlight Substituted Fonts. On, the canvas paints
  pink behind text drawn in a substitute for a missing font, in every open document. Off, no document shows that
  highlight. `document.setup` ignores this key and adds a `warnings` entry that names `prefs.set` to its reply. A file
  saved with the key in its Document Setup still opens and keeps the value it was saved with, and the preference
  decides whether that file shows the highlight.

```json
{"name":"run_command","arguments":{"command":"prefs.set","params":{"values":{"typeSizeIncrement":4,"trackingIncrement":50}}}}
{"name":"run_command","arguments":{"command":"prefs.set","params":{"key":"fontNamesInEnglish","value":false}}}
{"name":"run_command","arguments":{"command":"prefs.set","params":{"key":"hyphenationExceptions","value":"typography, hap-pen"}}}
{"name":"run_command","arguments":{"command":"prefs.set","params":{"key":"highlightAlternateGlyphs","value":true}}}
{"name":"run_command","arguments":{"command":"type.step","params":{"attribute":"tracking","by":-2}}}
```

File Handling › `appendConverted` (on by default): an older native file (the former `.drawcraft` extension or an
older format version) opens titled `<name> [Converted]`, and Save doesn't write over it: `document.save` without a
`path` answers with a suggested `.vectorcraft` name (the app asks for one). Off, it opens under its own name and Save
writes over it.

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
same way. With the Width tool, `press_key` Escape deselects the selected width points (`handledBy: "tool"`) and
leaves the document and its undo history as they were.
A compound path's stroke is the compound's: width point commands given one of its members edit the compound's
profile, which runs along every subpath (the Width tool shows each subpath's points).

`object.puppetWarp {ids?, pins, moved, angles?, expand?}` warps art as rigidly as possible so that each pin (a point
on the art) lands on its `moved` point. `angles` (degrees, or null for a free pin; as long as `pins`) holds the turn
the art takes around a pin, as Alt-dragging near a selected pin with the Puppet Warp tool does: one pin with
`angles: [90]` turns the art a quarter turn around it. `press_key` Escape between two `pointer_gesture` calls of a
pin drag or a turn cancels it: the art, its pins and the selected pins go back to how the press found them, and no
undo step is added.

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

Initially (and after `paint.default`) new art has a white fill and a 1 pt black stroke.
New objects take the fill and stroke proxies (`paint.setFill` / `paint.setStroke` and the weight, whatever is
selected) on top of a template. With nothing selected, `stroke.set` / `stroke.setAdvanced` (cap, join, dashes,
arrowheads, profile…) and `graphicStyle.apply` (every fill, stroke and effect, opacity, blend mode and the link to the
style; `add: true` stacks it) set that template instead of editing art. With the Appearance panel's New Art Has Basic
Appearance off (`appearance.setNewArtBasic {on: false}`, the preference `newArtBasic`) new art takes the whole
appearance of the last selected object. `appearance.newArt` reports what the next object gets; `paint.default`
resets it.
`appearance.setNewArtBasic` rejects an `on` value that is not a boolean. Omitting `on` toggles the current preference.

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
`object.path.splitIntoGrid` and `text.setStyle` `size`/`leading` also take a string with a unit, `"5 mm"`). A length
string takes the same arithmetic as the desktop app's fields, where `%` is a share of the value before it:
`"200mm*50%"` is 100 mm and `"50pt+25%"` is 62.5 pt. The units
only change what the UI shows and how it reads typed numbers. Preferences ▸ Units has
four: General (rulers, positions and sizes, the Info panel, dialog distances, canvas measurement labels), Stroke
(weights, dashes), Type (font size, leading, baseline shift) and East Asian Type (no field uses it yet). General is the active document's units
(`document.inspect` → `units`): `document.setUnits {units}` (Document Setup) sets it for that document, and
`prefs.set {key: "unitsGeneral", value}` sets it for the active document too (one undo step) and is the units
`file.new` starts in when it gets no `units`. Stroke, Type and East Asian Type are the preferences `unitsStroke`,
`unitsType` and `unitsAsianType`. Only `unitsAsianType` also takes `q` (Q, `Q`) and `ha` (Ha, `Ha`), each a quarter of
a millimetre (0.709 pt); the other preferences, `document.setUnits` and the rulers do not list them, and it is Points
by default. A length typed with a unit
reads `13q` or `13h` (also `13 Q`, `13Ha`) in any field or string param (`text.setStyle {size: "13 Q"}`): the value
is stored in points.
`document.switchUnits` (Cmd/Ctrl+Alt+Shift+U) sets the active document's units to the next unit in the order a
right-click on a ruler lists them (Points, Picas, Inches, Millimeters, Centimeters, Pixels, Feet & Inches, Meters,
Yards, Feet, then Points again), in one undo step, and replies `{units}`.
`numbersWithoutUnitsArePoints` (on by default) makes the desktop app's length fields in picas read a number typed
with no unit in points (`12` is 12 pt, `2p6` and `1p` stay picas, `12 mm` millimetres); off, it is in picas. Fields in
other units read it in their unit either way, and the Preferences dialog dims the option unless a unit is Picas.
Command params are in points either way. `identifyObjectsBy` (`objectName` by default, or `xmlId`) sets how Layers,
Variables and Layer Options show and edit object names: Object Name keeps free text; XML ID shows AI-style XML ids
(`My Rect #2` as `My_Rect__x23_2`) and `layer.setProps` / rename require a valid XML ID (letter, underscore or colon
first; no spaces), stored back as the object name.
`prefs.list` marks the length preferences (`keyboardIncrement`, `cornerRadius`, `pasteOffset`, `gridlineEvery`,
`typeSizeIncrement`, `baselineShiftIncrement`) with `measure: "general"|"type"`; they are kept in points and also take
a string with a unit. Unit names: `Points`, `Picas`, `Inches`, `Millimeters`, `Centimeters`, `Pixels`, `Feet & Inches`,
`Meters`, `Yards`, `Feet` (the preferences use `points`, `picas`, … `feetInches`, `meters`, `yards`, `feet`).
`edit.preferences {category?}` opens the Preferences dialog (`preferences`) with its search field focused. Its
`__search` field (`ui.dialog.set`) holds the search text: while it has text, the category list is replaced by the
settings whose label, section or category contains it, ignoring case, in the interface language or in English, or by
"No settings found". Choosing a setting sets `__category` to its category, sets `__found` to its key (the row is
highlighted until another category is shown) and clears `__search`. Cmd/Ctrl+F focuses the field again; Escape in the
field with text clears it, and Escape in the empty field closes the dialog.

```json
{"name":"run_command","arguments":{"command":"prefs.set","params":{"key":"unitsGeneral","value":"millimeters"}}}
```

## Document Setup

`document.setup` with no params reports the document setup: units, the bleed ([top, bottom, left, right] in pt, drawn
as a red outline around each artboard), the transparency grid (size and two colours; the first is also the simulated
paper colour), the flattener preset and Discard White Overprint (Overprint Preview keeps white overprints visible while
it is on), the type options (language and its quotes, Use Typographer's Quotes for typed straight quotes and a typed `”` where a quote opens, superscript, subscript
and small caps proportions, SVG text export) and the background contents (white: raster exports are white behind the
art). Pass any of these keys to change them in one undo step; `document.setUnits` stays as an alias for `units`.
A file whose bleed (0 to 72 pt a side), superscript and subscript sizes (1 to 500%) and positions (-500 to 500%) or
small caps size (1 to 200%) lies outside those ranges opens with the nearer end of each range.

```json
{"name":"run_command","arguments":{"command":"document.setup","params":{"gridColors":"Blue","gridSize":"large","language":"German","bleed":9}}}
```

## New Document

`file.new` takes everything New Document sets: `preset` (a name from `file.newPresets`), `name`, `width`/`height`
(points, or lengths such as `"210 mm"`), `units`, `orientation`, `artboards` with `artboardLayout {layout, columns,
spacing, rightToLeft}`, `bleed`, `backgroundContents`, `colorMode`, `rasterEffectsPpi` (1 to 2,400; a saved preset's
value outside that range is read as the nearer end) and `previewMode`.
`file.newPresets {category?}` lists the categories and presets (Recent: the last sizes used; Saved: the user's presets,
kept in the preferences by `file.newPresets.save` and removed by `file.newPresets.delete`). A listed preset can be
passed straight back to `file.new`. Print presets and sizes without `units` start in `unitsGeneral`; screen presets
(mobile, web, video, social) in Pixels. A saved or Recent preset whose size (up to 163,830 pt), `artboards` (1 to
1,000), spacing (0 to 100,000 pt) or bleed (0 to 72 pt a side) lies outside those ranges is read with the nearer end of
each range, and a size of 0 or less is read as 1 pt.
The desktop app's `file.newDialog` (Cmd/Ctrl+N) opens the New Document dialog on the first Recent entry, or on the
Print category's Letter while Recent is empty. `file.newWithoutDialog` (Cmd/Ctrl+Alt+N) runs `file.new` with those
settings and the next untitled name, without opening the dialog, and returns what `file.new` returns.
`window.nextDocument` (Cmd/Ctrl+F6) and `window.previousDocument` (Cmd/Ctrl+Shift+F6) make the next and the previous
open document in tab order the current one, as a click on its tab does, and return `{index}`. After the last document
comes the first, and before the first document comes the last. From the Home screen they go to the first and the last
document. With no document open they fail with "no document open". `document.activate {index}` makes a given document
the current one.
`window.arrange.tile` (Window › Arrange › Tile) shows every open document in its own pane, edge to edge in the document
area, in a grid of as many columns as the square root of the number of documents, rounded up, and fits each document to
its pane; with fewer than two documents open it leaves the view and `tiled` as they are. It returns `{tiled, panes}`.
The current document's pane is the canvas, and a press in another pane makes its document the current one.
`window.arrange.consolidateAllWindows` (Window › Arrange › Consolidate All Windows) shows one document under its tab
again and returns `{tiled: false}`. The arrangement is not saved. `ui.inspect` reports `tiled` and `panes` (each pane's
`document` index, `rect` and whether it is `active`); canvas coordinates map to the current document's pane
(`canvasRect`).
its pane; it returns `{tiled, panes}`. The current document's pane is the canvas, and a press in another pane makes its
document the current one. `window.arrange.floatInWindow` (Window › Arrange › Float in Window) moves the current document
out of the tab strip into a floating window of its own, `window.arrange.floatAllInWindows` (Float All in Windows) does
so for every tabbed document, and `window.arrange.cascade` (Cascade) floats every document and stacks the windows 30 pt
apart from the upper left of the document area; each returns `{floating}`, the floating documents' indices. On the
desktop each window is an OS window; on the web, on Wayland and while the control channel is attached (`--control`),
it is a window inside the app's window, so `ui.click`, `ui.drag` and `ui.screenshot` reach it. The current
document's window holds the canvas, and a press in another document's window makes its document the current one.
`window.arrange.consolidateAllWindows` (Window › Arrange › Consolidate All Windows) shows one document under its tab
again, floating ones included, and returns `{tiled: false, floating: []}`; Tile brings floating documents back into its
grid. `window.arrange.floatInWindow {index, at: [x, y]}` floats document `index`, makes it the current one and puts its
window's top-left corner at `at` in the main window's points, as dragging its tab more than 24 pt above or below the tab
strip does. `window.arrange.consolidateAllWindows {index}` brings only document `index` back under its tab and makes it
the current one, as dragging its window by the title bar (an OS window by the tab at its top) onto the tab strip does;
it returns `{tiled, floating}`. An `index` that names no open document fails with "no such document". The arrangement
is not saved. `ui.inspect` reports `tiled`, `panes` (each pane's `document` index, `rect` and
whether it is `active`) and `floating`; canvas coordinates map to the current document's pane or window
(`canvasRect`), in that window's coordinates while it floats.
`window.newWindow` (Window › New Window) opens another view of the current document, with its own zoom, center,
rotation, artboard and Outline or Preview, makes it the current window and returns `{index, view}`, the view's number.
Each view has a tab of its own; while a document has more than one view, its tabs and windows show the view's number
after the name ("Name:2"). It opens in a floating window when the view it came from is in one. With no document open
it fails with "no document open". The Arrange commands treat each view as a window: `floatInWindow` and
`consolidateAllWindows` take `view` beside `index` (the document's current view without it), and Tile and Cascade
count views. `file.close` without an `index` closes the current view, and the document's last view closes the document;
with an `index` it closes the document and all its views. `window.nextDocument` and `window.previousDocument` step
through the views in tab order and add `view` to their reply for a document with more than one view. `ui.inspect`
reports `windows` (every window as `[document index, view number]`, in tab order), `currentWindow` and each pane's
`view`. Views are not saved.
`window.control.dock {edge}` (desktop app) docks the Control bar to the top of the window, under the menu bar
(`edge: "top"`, the default), or to its bottom (`"bottom"`), and returns `{edge}`. The bar's panel menu runs it with
Dock to Top and Dock to Bottom, the edge in use checked, and so does a drag of the bar's gripper, which docks the bar at
the edge the pointer is released nearer and shows a band along that edge while it moves. The edge is kept with the
workspace, and a workspace saved without it docks the bar at the top. Another `edge`, or none, fails with "`edge` must
be one of top, bottom" and leaves the bar where it is.

```json
{"name":"run_command","arguments":{"command":"file.new","params":{"preset":"A4","orientation":"landscape","artboards":4,"artboardLayout":{"columns":2},"bleed":9}}}
```

## Place

`file.place` puts another file's art into the active document as one undo step without touching the clipboard:
a raster image at 100% of its physical size (the resolution its file declares, else 72 ppi; linked to its `path`
unless `link: false`; a Photoshop document as its merged image), an SVG as one group (fitted by its art's bounds; with `crop: "crop"` given, framed and clipped by its viewBox, #788), a PDF/.ai page as one clipped group, with the images, symbols, patterns
and swatches it uses. A VectorCraft document read from `path` is a placed document: one locked object showing its
artboard `page` (or, with `crop: "bounding"`, its art's bounds), linked to the file and read again when it changes
(see Linked images), and vectors in every output; with `link: false` (or from `dataBase64`) it is an editable copy
of its art, as one clipped group. A PDF, `.ai` or `.ait` page read from `path` (with neither `password` nor
`textAs: "outlines"`) is a placed document the same way, showing page `page` clipped to its `crop` box (or its
art's bounds); with `link: false` it is placed as one clipped group. EPS and PostScript `.ai` files are embedded. `at` centres it, `rect` fits it, `replace`
swaps the selected object (keeping its place and transform), `template` puts it on a new template layer.
The art's bounds are the bounds of the art that shows, with its strokes and effects, as an export measures them:
hidden objects, guides and template layers do not count, and symbol instances are measured by their art. A page or artboard holds the objects
whose shown art lies on it; a hidden object on it is still placed and stays hidden, as do the hidden members of the groups placed.
`replace` and the `width` and `height` that `file.place` answers also measure symbol instances by their art.
`file.place` decodes the pixels of a PNG, JPEG, GIF or WebP file within the decode allowance before placing it. When
they can't be read, as with a PNG cut short by a download or a sync still running, it fails with "the image's pixels
can't be read" and places nothing; `clipboard.importImage` fails the same way. A JPEG cut short is placed when its
pixels still decode. Both keep the decoded pixels (except a CMYK JPEG's) in the image store the renderers share, so the first render
that draws the image does not decode the file again.
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
pastes objects back into the layers they came from, by name, making missing ones. Drawing modes (`view.drawMode`) apply
too: in Draw Inside, `edit.paste`, `edit.pasteInPlace` and `edit.pasteOnAllArtboards` put the objects inside the
object drawn into (its clip group, made on the first paste), whatever Paste Remembers Layers says; in Draw Behind,
`edit.pasteInPlace` and `edit.pasteOnAllArtboards` put them just below the selection. `edit.pasteInFront` and
`edit.pasteInBack` ignore the drawing mode. When the object drawn into is the one isolation mode isolates (`object.isolate`,
or a double-click on it with the Selection tool), art drawn or pasted inside it takes isolation mode into its clip group,
so the new art can be picked and isn't dimmed; `edit.undo` or `edit.redo` that removes the isolated object ends
isolation mode.

Artboards copy with their art. `artboard.copy {index? | indices?, art?}` puts artboards on the clipboard together
with the art fully inside each (`index` and `indices` default to the artboards the Artboard tool has selected; `art`
defaults to the tool's `moveArt` option, Move/Copy Artwork with Artboard; locked and hidden art only with
`lockedAndHidden: true`, default: prefs `moveLockedWithArtboard`); with the Artboard tool chosen, `edit.copy` does this for the tool's selected artboards and
`edit.cut` runs `artboard.cut`, which also deletes them (one artboard is always left: a cut of every artboard fails).
Then `edit.paste` adds a copy of each artboard, together right of the last one and placed as they lay, with their art
in the layers it came from, in one undo step (`edit.pasteInPlace`, `edit.pasteInFront` and `edit.pasteInBack` put
them where they were; `edit.pasteOnAllArtboards` pastes only the art), in this document or another; the result's
`artboards` lists the new artboards' indices and `artboard` is the first of them, and the Artboard tool, when chosen,
selects them. `edit.paste {artboardsAt: [x, y]}` instead centers the copied artboards on that point, as they lay, and a
value that is not `[x, y]` fails with `` `artboardsAt` must be [x, y] ``. `artboard.duplicate {index? | indices?, art?}` does the same in one step → `{index, indices, ids}`.
Window › Artboards › Duplicate Artboards passes the selected artboards (those the Artboard tool has selected, else the
active one) as `indices`, and a row dragged onto New Artboard passes its artboard as `index`.
`artboard.new {beside?, side}` adds an empty artboard the size of artboard `beside` (default: the last), 20 pt from
that side of it, and `artboard.duplicate {index, side}` puts the copies there instead, together as they lay on that side
of the artboards they copy; `side` is `"left"`, `"right"`, `"top"` or `"bottom"`. Another `side` fails with
`` `side` must be "left", "right", "top" or "bottom" ``, `beside` without `side` fails with `` `beside` needs `side` ``,
and `artboard.new` refuses `side` given with `x`, `y`, `width` or `height`; each leaves the document and its undo
history as they were. With the Artboard tool and one artboard selected, a button with a plus shows just outside the
middle of each edge where an artboard of the same size fits on that side (within the canvas and overlapping no other
artboard): a click runs `artboard.new {beside, side}`, an Alt-click `artboard.duplicate {index, side, art: true}`, and
the new artboard becomes the active one. An Alt-click on a locked artboard's button does nothing.
`artboard.move {copy: true, moveArt: true}` is the Artboard tool's Alt-drag. `artboard.duplicate` and `artboard.move`
also move the pattern tiles of the copies when `patterns` is true (default: prefs `transformPatternTiles`);
`edit.paste` leaves the tiles of a pasted artboard's art where they were.
`artboard.duplicate`, `artboard.copy`, `artboard.cut`, `artboard.move` and `artboard.rearrange` take `lockedAndHidden?`
(default: prefs `moveLockedWithArtboard`), and with the Artboard tool chosen `edit.copy` and `edit.cut` pass it on. When
art goes along, the journal records the value, so a replay under another preference takes the same art.
Every artboard command that takes an `index` fails with `no such artboard` when the `index` is not the
0-based index of an artboard (past the last one, negative, fractional, a string or null), and the document and the
clipboard stay as they were.
These artboard commands refuse a string, a number or another value that is not true or false with
`` `art` must be true or false ``, naming the key, and leave the document, its undo history and the clipboard as they
were: `moveArt`, `scaleArt`, `strokes`, `corners`, `patterns`, `lockedAndHidden` and an artboard's `locked`,
`showCenterMark`, `showCrossHairs` and `showVideoSafeAreas` in `artboard.setProps`; `showCenterMark`, `showCrossHairs`
and `showVideoSafeAreas` in `artboard.new`; `moveArt`, `copy`, `patterns` and `lockedAndHidden` in `artboard.move`; `art`, `patterns` and `lockedAndHidden` in `artboard.duplicate`;
and `art` and `lockedAndHidden` in `artboard.copy` and `artboard.cut`. `artboard.setProps` refuses an artboard's `x`, `y`, `width` or
`height` that is not a number, and `artboard.move` refuses such a `dx` or `dy`, with
`` `dx` must be a number, not "10" ``. A null is read as a key left out.
`artboard.new` refuses an `x`, `y`, `width` or `height` that is not a number with the same message, and
`artboard.new` and `artboard.setProps` refuse a `name` that is not a string, such as 5, with
`` `name` must be a string ``. Both leave the artboards and the undo history as they were, and read a null as the
key left out.

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

`artboard.setProps {artboards: [{index, name?, color?, locked?, showCenterMark?, showCrossHairs?, showVideoSafeAreas?, videoRulerPixelAspectRatio?, x?, y?, width?, height?}…], scaleArt?, moveArt?, …}` sets several
artboards in one undo step, each with its own fields; the other params apply to all of them. The art that moves or
scales with each artboard is the art inside it before any of them changes, and an object inside several of them goes
with the first one listed that moves or scales its art. An empty list and a list that names an artboard twice are
refused.

With several artboards selected with the Artboard tool, the desktop app gives a new size to each of them in one such
call. Properties' W and H and the Transform panel's W and H do this, and so do Properties' Preset and orientation
buttons, the Control bar's preset, orientation, W and H, and a new Width or Height in Artboard Options. The W and H of
the Transform panel and the Control bar resize each artboard about its own reference point, and with the proportions
linked they keep each artboard's own proportions. Artboard Options resizes each artboard about the reference point its
form used for the new Width or Height. Properties' W and H and the preset and orientation buttons keep each artboard's
top-left corner. A preset sizes each artboard in its own orientation, and an orientation
swaps the width and height of each artboard that is not square and not already in it. A new X or Y moves the active
artboard only, and Artboard Options' Name renames only the artboard it shows. A locked artboard among the other
selected artboards keeps its size.

```json
{"name":"run_command","arguments":{"command":"artboard.setProps","params":{"artboards":[{"index":0,"width":300},{"index":2,"width":300}],"scaleArt":true}}}
```

Delete Empty Artboards: `artboard.deleteEmpty {}` deletes every artboard with no artwork on it, with its guides, in
one undo step, and returns the indices it deleted (numbered as before) as `{deleted: [index…]}`. An artboard has
artwork when an object (hidden, locked and template art included; guides not) has visual bounds that meet or touch
it; a group counts by its members, a clip group or clipping layer by its members inside the clip. Locked artboards stay; when every artboard is empty and none is locked, artboard 1 stays (with a locked
artboard left, artboard 1 can go). With
nothing to delete the command is disabled and fails with `no empty artboards to delete`.

Rename Artboards: `artboard.rename {index? | indices?, name}` renames artboards in one undo step, by default those the
Artboard tool has selected, else its active one. One artboard takes `name`, trimmed. Several take it numbered in artboard
order, `name-1`, `name-2` and so on. A locked artboard is renamed too. An empty name fails with
`` `name` must not be empty ``, a name that is not a string fails with `` `name` must be a string ``, and an artboard
that is not in the document fails with `no such artboard`. When every artboard already has its new name, the command
records no undo step → `{renamed: [index…]}`. With no params, a menu click opens the `renameArtboards` dialog (fields
`name` and `indices`), and OK with an empty name changes nothing. The Artboards panel's name field runs this command too.

Switch Orientation: `artboard.switchOrientation {index? | indices?, scaleArt?, patterns?, lockedAndHidden?}` swaps the width and height of each
artboard in one undo step, by default those the Artboard tool has selected, else its active one. Each top-left corner
stays put, as with the Properties panel's Portrait and Landscape buttons. This rule for several artboards is VectorCraft's
own. A square artboard and a locked one stay as they are, and the call fails with `the artboard is locked` when every
artboard it names is locked. When no artboard changes, the command records no undo step → `{switched: [index…]}`.
`scaleArt` scales the art with each artboard and defaults to the Artboard tool's Scale Artwork with Artboard option.
`patterns` and `lockedAndHidden` work as they do on `artboard.setProps`.

When a call to `artboard.duplicate`, `artboard.copy`, `artboard.cut`, `artboard.rename` or `artboard.switchOrientation`
names no artboard, its journal entry records the artboards it acted on as `selectedArtboards`. A replay of the journal
acts on those artboards, and a recorded action leaves the value out and acts on the selection when it plays.

`artboard.selectAll {}` chooses the Artboard tool when another tool is in use and selects every artboard. The active
artboard stays active, the art's selection stays as it is, and no undo step is recorded → `{selected: [index…]}`, the
active artboard first.

The canvas context menu with the Artboard tool lists the entries of the public
[Use the artboard context menu](https://helpx.adobe.com/illustrator/desktop/create-manage-artboards/organize-manage-artboards/use-artboard-context-menu.html)
page in the page's order, and no entries for objects or the view: Cut (`artboard.cut`), Copy (`artboard.copy`), Paste
(`edit.paste`), Undo, Redo, New Artboard (`artboard.new`), Duplicate Artboards (`artboard.duplicate`), Rename
(`artboard.rename`), Delete Artboards (`artboard.delete` with `indices`), Delete Empty Artboards
(`artboard.deleteEmpty`), Select All Artboards (`artboard.selectAll`), Lock Artboards and Unlock Artboards
(`artboard.setProps` with `locked`), Rearrange (`artboard.rearrange`), Fit to Artwork Bounds (`artboard.fitToArt`),
Switch Orientation (`artboard.switchOrientation`), Artboard Options (`tool.options {tool: "artboard"}`) and Export
(`file.exportForScreens`). Exit Isolation Mode and the Unlock submenu for locked objects under the pointer follow them.
Cut, Copy, Duplicate Artboards, Rename, Delete Artboards, Lock Selected, Unlock Selected, Fit to Artwork Bounds, Switch
Orientation and Export act on the artboards the tool has selected, else the active one. Artboard Options opens on the
active artboard. New Artboard, Paste, Undo, Redo, Delete Empty Artboards, Select All Artboards and Rearrange do not act on
the selected artboards. Lock All and Unlock All act on every artboard, and Lock All Except Selected acts on the artboards
that are not selected. A right-click on an artboard that is not selected first selects it alone. An entry that cannot
run is left out: Cut with a locked artboard or every artboard selected, Delete Artboards with no artboard to delete, Select All Artboards with every artboard selected, Rearrange with
one artboard, Fit to Artwork Bounds with no art on a selected unlocked artboard, Switch Orientation with no selected
artboard to change, and a lock entry with no artboard to change. The menu measures the art for Delete Empty Artboards and
Fit to Artwork Bounds once, at the right-click. Paste centers copied artboards on the right-click's point
(`edit.paste {artboardsAt: [x, y]}`) and centers copied objects there too (`center`). Chosen from a menu or the command
palette with the tool in use, `artboard.new` makes the new artboard active, `artboard.duplicate` selects the copies,
and `artboard.delete` and `artboard.deleteEmpty` make a remaining artboard active.

Locked artboards: `artboard.setProps {index, locked: true}` locks an artboard and `{index, locked: false}` unlocks it,
each in one undo step. The Artboards panel's lock column, left of a row's options icon, shows a lock on a locked
artboard, and a click there runs this command. A locked artboard keeps its place and size. `artboard.setProps` with a
new `x`, `y`, `width` or `height` fails with `the artboard is locked` unless the same call unlocks it, and
`artboard.move` without `copy`, `artboard.delete`, `artboard.cut` and `artboard.fitToSelection` fail the same way.
`artboard.fitToArt` and `artboard.rearrange` leave a locked artboard where it is, and a copy of one
(`artboard.duplicate`, `artboard.move {copy}`, Paste) starts unlocked. The art on a locked artboard is each object of
a layer or sublayer whose bounds, with its strokes, overlap the artboard; a group counts as one object, and a hidden
object, a guide or the art of pattern editing mode is on no artboard. That art acts as locked art: clicks, marquees, `select.set`, `select.all` and the
tools pass over it, and commands that act on the selection leave it. Locking the artboard deselects it, and so does an
edit that moves art onto a locked artboard. Art that an edit draws or pastes onto a locked artboard stays selected and
editable until a selection leaves it out or an edit moves it off the locked artboards, and is then protected like the
other art on it. `edit.undo`, `edit.redo` and a failed `command.batch` bring back the selection a step had, with that
art selected and editable when it was. When another artboard moves, scales or is cut (`artboard.move`,
`artboard.setProps`, `artboard.cut`, `artboard.rearrange`), art that is also on a locked artboard stays where it is
unless `lockedAndHidden` is true (default: prefs `moveLockedWithArtboard`); a copy of an artboard copies
it. With the Artboard tool, a click
selects a locked artboard, but a drag doesn't move or copy it, it shows no handles, Delete leaves it, and a
double-click on its name above the canvas doesn't edit the name (`artboard.setProps {name}` and the Artboards panel
still rename it). A Shift-click or Shift-drag on a locked artboard selects or draws as on any other artboard. The
document summary (`document.inspect`) adds `locked: true` to a locked artboard.

An artboard's background color (Properties › Artboards › Fill, Artboard Options' Background Color): `artboard.setProps
{index, color}` sets it, with a color as the paint commands take it (`"#rrggbb"`, `{c, m, y, k}`, …; `#rrggbbaa` and `#rgba` are refused), and
`artboard.setProps {index, color: null}` removes it, each in one undo step; `artboard.new {color}` makes an artboard
with one. The canvas draws that artboard's paper in
the color. With Simulate Colored Paper on, it multiplies the color with the paper color, each sRGB channel as
artboard × paper / 255; artboards without a color show the paper color. The transparency grid still covers it while
it shows. Copies, duplicates and the native format keep the color. Exports draw it behind the art of each artboard
they write (raster formats, a transparent background too; PDF, SVG and EPS pages; `.ai` files' PDF pages; Export for
Screens; `file.print` and its preview), as a rectangle of the artboard filled with the color in its own model, on a
layer of its own below the others; files that carry the editing data (Preserve Editing, `.ai`, EPS, native rows of
Export for Screens) keep it as the artboard's color, not as art. `includeArtboardFill: false` on `document.export`,
`document.serialize`, `document.exportForScreens` (Include Artboard Fill) or `file.print` leaves it out. `inspect_document`
lists it as the artboard's `color` (`#rrggbb`), and `ui.colorPicker {artboard: index}` opens the Color Picker for it.

Artboard Options' Show Center Mark and Show Cross Hairs: `artboard.setProps {index, showCenterMark?: bool,
showCrossHairs?: bool}` (or each entry of `artboards`) turns them on or off for that artboard in one undo step, and
`artboard.new` takes them too. The canvas draws the center mark as a small cross at the artboard's center and the cross
hairs as lines from the middle of each side through the center, 1 point wide on screen at any zoom; exports and print
leave them out. Duplicates, copies and the native format keep them, and `inspect_document` adds `showCenterMark: true`
and `showCrossHairs: true` to an artboard that shows them.

Artboard Options' video display options: `artboard.setProps {index, showVideoSafeAreas?: bool,
videoRulerPixelAspectRatio?: number}` (or each entry of `artboards`) sets them for that artboard in one undo step, and
`artboard.new` takes them too. Show Video Safe Areas outlines the artboard's action-safe area 3.5% and its title-safe
area 5% in from each edge, as EBU R 95 sets them (https://tech.ebu.ch/docs/r/r095.pdf), 1 point wide on screen and only
on the canvas. The Video Ruler Pixel Aspect Ratio, from 0.1 to 10 (default 1, square pixels; the commands refuse anything
else, and a file's outside the range opens brought into it),
is the width of one video pixel over its height, which `view.videoRulers` counts the artboard in. Duplicates, copies and
the native format keep both (written only when on or not 1), and `inspect_document` adds `showVideoSafeAreas: true` and
`videoRulerPixelAspectRatio` to an artboard that has them.

Artboard Options' Global checkboxes are application preferences, not artboard properties: `fadeRegionOutsideArtboard`
(Fade Region Outside Artboard) and `artboardUpdateWhileDragging` (Update While Dragging), both on by default
(`prefs.set {values: {fadeRegionOutsideArtboard: false}}`; also Edit › Preferences › General › Artboard Options). With
the first on, the canvas draws everything outside the artboards darker while the Artboard tool is active, the pasteboard
and the art on it: each color channel at 60% of its value, VectorCraft's own amount. With the second off, that area
draws as usual while the Artboard tool moves or resizes an artboard and darker again when the drag ends. When crossing
artboards would leave more than 4,096 separate rectangles in view, only the area outside the box around all the
artboards draws darker. Presentation Mode, exports and print do not change. The user guide describes both checkboxes under Artboard options
(https://web.archive.org/web/20210601162814/https://helpx.adobe.com/pdf/illustrator_reference.pdf).

```json
{"name":"run_command","arguments":{"command":"artboard.copy","params":{"index":0}}}
{"name":"run_command","arguments":{"command":"edit.paste","params":{}}}
{"name":"run_command","arguments":{"command":"artboard.duplicate","params":{"index":0,"art":true}}}
{"name":"run_command","arguments":{"command":"artboard.setProps","params":{"index":1,"locked":true}}}
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
all of it as Dublin Core. PDF export and PDF-compatible `.ai` and `.ait` saves write the `created` date (the time of
writing when the document has none) as the file's creation date and the time of writing as its modification date.
They also write the `copyrightNotice` as `dc:rights`, the `copyrightUrl` as `xmpRights:WebStatement` and the
`copyrightStatus` as `xmpRights:Marked` (`True` for copyrighted, `False` for public domain, none for unknown) to the
XMP packet.
A PDF, `.ai` or `.ait` opened from its pages or its editing copy (`document.open`,
`file.newFromTemplate`, `file.revert`; not one restored from the document it carries) opens with the `author`,
`description`, `keywords`, `copyrightStatus`, `copyrightNotice`, `copyrightUrl`, `created` and `modified` that its XMP
packet (`dc:creator`, `dc:description`, `dc:subject` or `pdf:Keywords`, `xmpRights:Marked`, `dc:rights`,
`xmpRights:WebStatement`, `xmp:CreateDate`, `xmp:ModifyDate`) and its document information dictionary (`Author`,
`Subject`, `Keywords`, `CreationDate`, `ModDate`) give, the XMP packet's value where both give one. A value that
`file.info` would refuse is left out, and the title is set as for any other open (the file name, or a new untitled
name for a template).
`document.inspect` (`inspect_document`) and the `document.open` (`open_file`) reply report the File Info title as
`fileInfoTitle`. Their `title` is the document's tab name, which comes from the file name once the document is saved
to or opened from a file (with ` [Converted]` or ` [Recovered]` where that applies).
`file.new {created}` and a save to a file (`document.save` or `file.saveAs` with
`{path, modified}`) use the given date (Unix seconds, or `null` for none) instead of now. Their journal entries always
record the date they used, so replaying the journal makes the same document whenever it runs (an action recorded in
the Actions panel leaves the date out: playing it later dates the document then). As a step of `command.batch`, a
command records the date it used (and any value it takes from the preferences) in its step of the batch's journal entry.
A batch may open, switch, close or revert documents: each document it edits gets one undo step, the batch is
journaled whichever document it ends in, and an error rolls every document back (the ones it opened close, the ones
it closed or reverted come back). The rollback puts the documents back, not the disk: files its steps wrote (a save, a
Save As, an export) stay written, new files included, as Undo leaves a saved file on disk. A caller that wants nothing
written unless the whole batch succeeds puts the saves and exports last, or runs them after the batch.

```json
{"name":"run_command","arguments":{"command":"file.info","params":{"author":"Ada","keywords":"poster, fair","rating":4,"copyrightStatus":"copyrighted","copyrightNotice":"© 2026 Ada"}}}
```

## Document Raster Effects Settings

`document.rasterEffectsSettings` holds how raster effects (shadows, glows, blurs, feathers) become images when PDF
export or Expand Appearance renders them: `resolution` (1 to 2,400 ppi, also New Document's `rasterEffectsPpi`; a
file's value outside that range opens as the nearer end), `colorModel`
(the document's `rgb`/`cmyk`, `grayscale` or `bitmap`), `background` (`white` makes the images opaque), `antiAlias`
(off: hard edges), `clippingMask` (the white stays under the art only), `addAround` (0 to 1,000 points of room
around the art; a file's value outside that range opens as the nearer end) and `preserveSpotColors` (stored). They are
also the defaults of `object.rasterize`, whose params override them (its
`clippingMask` puts the image in a clip group with the art's outline). No params reports them; any of them changes them
in one undo step. PDF export renders the raster effects of every kind of object this way (paths, groups, layers, type,
images, symbol instances, live objects, and those in opacity masks, symbols and pattern tiles): shadows and outer glows
go in an image under the untouched vector art, other effects replace the object with its image, and a path whose
fills or strokes carry effects is written as one piece per fill and stroke, so only the affected ones become images.
When an image in the art can't be decoded, `object.rasterize` and `object.cropImage` (without `trim`) make their image
without it, and their results carry the `warning` that Flatten Transparency gives. `object.rasterize` takes each frame
of threaded text that it rasterizes out of its thread with the text it shows, as `type.createOutlines` does (see
Threading text from the in and out ports), and the other frames keep the text they hold.

```json
{"name":"run_command","arguments":{"command":"document.rasterEffectsSettings","params":{"resolution":"high","background":"white","addAround":36}}}
```

`effect.apply {effect: "rasterize"}` (Effect › Rasterize…) draws an object as an image of square pixels while its
paths, type and other art stay editable. It takes the options of Object › Rasterize under these names: `colorModel`
(`document`, `grayscale` or `bitmap`), `useDocumentResolution` (true by default: the resolution above) or
`resolution` (1 to 2400 ppi), `background` (`transparent` by default, or `white`), `antiAlias` (`art`, `type` or
`none`), `clippingMask` and `addAround` (points). The image covers the object's bounds and the room around them,
its first pixel at their top-left corner. Both `object.rasterize` and the effect measure the art as drawn: with its
geometry effects, its brush art and its raster effects. The effects above Rasterize in the stack, shadows included,
are part of the image, and the shadows and glows below it are cast by the image. The canvas draws an image of up to
8.4 million pixels, at most 32,768 a side, on its own grid and then at the view's zoom: an image pixel wider than a
screen pixel shows as one flat square, and smaller ones are averaged. A larger image is made from the art drawn at the canvas's resolution
(lower for very large art), and its pixels no wider than a pixel of that drawing show the art as drawn.
When Rasterize is an object's only raster effect, Expand Appearance and PDF and SVG export replace the object with the
image `object.rasterize` makes with the same options.

```json
{"name":"run_command","arguments":{"command":"effect.apply","params":{"effect":"rasterize","params":{"useDocumentResolution":false,"resolution":150,"background":"white"}}}}
```

## Glass textures

Glass (`distort.glass`) takes an image file as its surface in `params.textureImage`, as `{path}` or
`{name?, dataBase64}`: a PNG, JPEG, GIF, WebP, TIFF, BMP, Targa, PSD, PSB, PCX, Pixar, JPEG 2000 or HEIC/HEIF file, decoded within the pixel allowance the
renderer draws images in, counted as RGBA. `effect.apply` and `effect.setParams` store the image's gray levels in the effect as a grayscale PNG
in base64 and set `texture: "loaded"` unless the params give a `texture`. `effect.list` reports the stored string,
and a command given that string stores it unchanged. Any other string, a file that is not an image, or an image past
the allowance is an error. A save keeps the texture with the effect. Glass lays the texture one image pixel per point
at Scaling 100%, centered on the object's center and repeated in both directions. Each pixel's luma is the surface's
height, from 0 for black to 1 pt for white. A transparent pixel counts as middle gray, and a partly transparent pixel
is mixed with middle gray by its alpha.

```json
{"name":"run_command","arguments":{"command":"effect.apply","params":{"effect":"distort.glass","params":{"distortion":10,"textureImage":{"path":"/tmp/texture.png"}}}}}
```

## Linked images

A placed image linked to its file (`file.place {path}`, Link on) records the file's absolute path, its size,
modification time and hash, and keeps a low-resolution preview (at most 256 px a side): a `.vectorcraft` save writes
the preview instead of the pixels, plus the path relative to the saved file. Links inside symbols, graph designs,
patterns and opacity masks are saved with their relative paths too. `document.open` reads
the linked files again, looking for each at its path, then at its relative path and by name in the document's folder
(so a folder moved with its links still opens), and each link, in the layers or inside that art, takes the path where
its file was found. Only files on this computer, or on the network share the document is
on, are looked at: a path to another computer, a device or a URL counts as missing. On Windows, a document opened from
a mapped drive (`Z:\…`) is on the share the drive is mapped to, and links on that share (`\\server\share\…`) are
looked at. A link on a drive mapped to another share (`Y:\…`) counts as missing, from a document on this computer too.
The links of an open document keep their relative paths from the file it was opened from (for a `.ai` file another
app wrote, that file), also after a save to another folder. When a link's file is not at its path, a save and Package
write the path relative to the new file of the place the file is found, or for a missing file of the place that
relative path names, and a document made with New Document from State keeps the same places, so a document moved while
its linked files were missing finds them once they are back in that folder. In the open document, a link
is looked for at its path, then at its relative path from the folder of the file the document was opened from and from
the document's own folder, then by name in those two folders. A link found away from its path takes the relative path
of the place it was found.
The result lists `missingLinks`
(their images show the preview), `modifiedLinks` (left as they were; read again only with the preference
`updateLinks: "automatically"`, then they are in `updatedLinks`), each as `{name, path, ids}`, with `refused` (why the
path isn't followed) when it isn't. The desktop app also checks the active document's links when its window
comes back to the front, following the same preference. `links.check` reports every link's `status` (`ok`, `modified`,
`missing`), `links.update {ids?}` reads modified files again and `links.relink {ids?, path | folder}` points images at
another file (or each at the file of its name in a folder); images keep their bounds, one undo step each.
`links.relink`'s `scope` picks which links take the new file (Relink's Replace choice): `selected` (the default) relinks `ids`; `all` also relinks every other object in the layers linked to the same file as
one of them; `missingInFolder` relinks `ids` to `path`, then every other missing link to the file of its name in
`path`'s folder, and lists the names not there in `notFound` (with `copy`, only missing images are looked for); `each` takes `paths: [{ids, path}]` in place of `ids`,
`path` and `folder` and relinks each link to its own file. An unknown scope, `paths` without `each`, `each` with `ids`,
`path` or `folder`, an entry without `path` or `ids`, an id in two entries and `missingInFolder` with `folder` are
errors, and nothing is relinked. Each call is one undo step. The Links panel's Relink… opens the `relink` dialog
(`ui.relinkDialog`) with a Replace menu of these four choices. A PNG, JPEG,
another file (or each at the file of its name in a folder); images keep their bounds, one undo step each.
`links.update`, and `document.open` when it reads a modified file, also read the file again for the images and placed
documents inside symbols, graph designs, patterns and opacity masks that show the same content (by default also for
those that only that art shows); they have no ids, so the results do not list them. Saving a document also reads it
again for the placed documents of it inside that art in the other open documents. A PNG, JPEG,
GIF or WebP file whose pixels can't be read is not shown: `links.relink` fails with "the image's pixels can't be
read", and `links.update`, or `document.open` when it reads the file, lists it as missing, its images keeping what they
showed. Without a file system (the web), linked images show their previews. A link whose path has a colon and no `/`
or `\` is read as a classic Mac OS path (`Disk:Folder:file.tif`), a form that older `.ai` files can hold. The link
keeps that path and is named by its last part. `links.info` gives that part as `fileName` and the rest as `location`,
and `links.relink {folder}`, the search of the document's folder and `links.findFiles` look for a file of that name.

`links.findFiles {folder}` looks in an absolute folder and its subfolders, on separate threads, for the files of the
active document's missing links, or of the images and placed documents `ids` whatever their status, and returns at once
with an `id` and the search's `state` (`searching` while it runs). It finds the files with each linked file's name, in
any letter case, and lists them best first as `{path, size, match, caseDiffers?}`: `same` when the file has the size and
hash saved with the link, `size` when it has the saved size and the link saved no hash (or the file is larger than 1 GiB,
which a search doesn't hash), else `name`. Within each kind, a file whose name has the link's letter case comes first.
Then each file's folder names are compared with the saved path's, from the innermost outward up to the first one that
differs, and a file with more matching names comes first; files that still tie are listed by path. Before the
comparison, the saved path's parent-folder steps (`..`, or the empty components of a classic Mac `::` run) are resolved,
and `.` components in a slash path are skipped. `links.findFiles {}`
returns the last search's state, and `{stop: true}` stops it. A search looks for at most 1,000 linked files, keeps the
first 10 files of each one's name and up to 10 that are the same file, and ends once every linked file has its same
file; it searches only the folders `text.findFontFiles` may search, with the same `maxSeconds`, depth and count limits.
`links.relink {ids, path, copy: true}` copies the file into the saved document's folder and links images to the copy,
never replacing a file there: the names `Name.png`, `Name 2.png` up to `Name 99.png` are tried in turn until one is
free, which gets the copy (`copied`), or is a file with the same contents, which is used (`kept`). With `copy`, a call
that includes a placed document fails and relinks nothing; placed documents are relinked without `copy`, where their
files are. Find in Folder… in the Linked File Not Found dialog and in the Links panel menu (`ui.findLinksInFolder`) runs
both. The web build has no folder search.

Placed documents (a `.vectorcraft` file, or a PDF or `.ai` page, placed linked)
work the same way: the document keeps the file's bytes and a preview (JPEG or PNG, 1 px per point, at most 1024 px
a side), and a save writes the preview (Include Linked Files: the file); output reads the file again, or uses the
preview with a warning when it is gone or changed.
`links.list` and `links.info` give them `document: true`, the file's `format` (`VectorCraft`, `PDF`,
`PDF-compatible .ai`) and the artboard's or page's `pageWidth` and `pageHeight` (pt). A relinked PDF shows the same
page and box of the new file. File › Package copies a placed PDF or `.ai` file as it is. For a `.ai` file read from
its editing data, Package also copies the files its art places into the same folder. That file finds them by name,
and Package warns when one is copied under another name: the copy then shows the file of that name, or its preview.
Saving the file in the app updates the open documents that place it. `links.embed` (the Links panel's Break Link)
turns one into the editable copy placing without link gives, its symbols, patterns, swatches and images joining
the document; so does `object.expand`.

```json
{"name":"run_command","arguments":{"command":"links.check","params":{}}}
{"name":"run_command","arguments":{"command":"links.relink","params":{"ids":[12],"path":"/new/photo.png"}}}
{"name":"run_command","arguments":{"command":"links.update","params":{}}}
{"name":"run_command","arguments":{"command":"links.findFiles","params":{"folder":"/Users/me/Archive"}}}
{"name":"run_command","arguments":{"command":"links.findFiles","params":{}}}
{"name":"run_command","arguments":{"command":"links.relink","params":{"ids":[12],"path":"/Users/me/Archive/2024/photo.png","copy":true}}}
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

An RTF file (any file but a `.txt` that starts with `{\rtf`) is placed the same way, without `text` options, and keeps
its formatting: each run's font, size, bold and italic, underline, strikethrough, all caps, small caps, superscript or
subscript, baseline shift and color, and each paragraph's alignment, indents, space before and after and direction.
Table cells are separated by tabs and each row is a paragraph; hidden text, pictures, headers, footers and notes are
left out. `file.place` answers `format: "rtf"`.

A Word document (a ZIP package named `.docx`, or one that holds `word/document.xml`, except a `.txt` file) is placed
the same way and keeps the same formatting, taken from the document defaults, the paragraph style and the character
style (and the styles each is based on) and direct formatting; theme fonts are read from the theme part. A paragraph
justified with a low, medium or high kashida length (`w:jc` `lowKashida`, `mediumKashida` or `highKashida`) is
justified with `kashida` `"short"`, `"medium"` or `"long"` (see [Insert Kashidas](#insert-kashidas)), and one
justified with `both` has none. Table cells
are separated by tabs and each row is a paragraph. Hidden and deleted text, field instructions, pictures, text boxes,
headers, footers, notes and list numbers are left out. A package with more than 10,000 files, a part that unpacks to
more than 16 MiB or nests elements more than 256 deep, and a document with more than 4 MiB of text or 100,000
paragraphs or runs are refused.
`file.place` answers `format: "docx"`.

A Word 97-2003 document (a compound file named `.doc`, or one that holds a `WordDocument` stream; an RTF file named
`.doc` is placed as RTF) is placed the same way and keeps the same formatting, taken from the style sheet's default
font, the paragraph style and the character style (and the styles each is based on) and direct formatting. Its main
text is read; table cells are separated by tabs and each row is a paragraph, and a field places as its result. Hidden
and deleted text, field instructions, pictures and embedded objects, headers, footers, notes, comments and text boxes
are left out, and `warnings` names each kind of content left out or changed once (pictures and objects, fields,
tracked changes, tables, and the other stories the document has); a field with no result is left out with its own
warning. Documents saved by Word 6.0 or Word 95, encrypted or password-protected documents and documents last saved with
fast save are refused, and so are a stream larger than 64 MiB, a document with more than 4 MiB of text, 100,000 pieces,
16,384 pages of properties, 100,000 runs of character or of paragraph properties, 16 MiB of properties in its styles
and property runs, or 100,000 paragraphs (hidden and deleted ones and those in field instructions counted) or runs of
placed type, one whose 16-bit text holds half of a surrogate pair, one whose structures lie outside their streams, one whose property pages are out of order, listed twice or overlap, one whose
style sheet counts 4,094 styles or more or holds fewer styles than it counts, and one whose styles are based on each
other in a loop.
`file.place` answers `format: "doc"`.

## PDF presets

`pdf.preset.list` lists the built-in presets (`VectorCraft Default`, which preserves editing, then High Quality Print,
Press Quality, Smallest File Size and the PDF/X presets, `builtIn: true`; `supported` says the writer produces the
preset's standard) and the saved ones, each with its `description` and `settings`. `pdf.preset.save {name?,
newName?, description?, preset?, …document.exportPdf options}` creates or changes a saved preset (a change starts from
the preset's own settings; `newName` renames it; built-in presets are read-only; passwords are never stored);
`pdf.preset.delete {name}` deletes one. `pdf.preset.export {names?, path?}` writes them as a `.vcpdfpresets` JSON file
(without `path` it returns `data`), and `pdf.preset.import {path? | data? | dataBase64?, replace?}` adds a file's
presets (names in use get a number unless `replace`). Saved presets work as `preset`
in `document.exportPdf`, `document.pdfSettings`, `document.export`/`serialize` with format `pdf` and `.ai` saves.
They are kept across restarts, in the desktop app and in `vectorcraft-cli` alike (see
[What a headless session remembers](#what-a-headless-session-remembers)).

```json
{"name":"run_command","arguments":{"command":"pdf.preset.save","params":{"name":"Web","preset":"Smallest File Size","compatibility":"1.5"}}}
{"name":"run_command","arguments":{"command":"document.exportPdf","params":{"preset":"Web","path":"/tmp/web.pdf"}}}
```

## Clipboard formats

Besides SVG, Copy offers other apps PNG and PDF of the copied objects, and their text when they are all type.
`clipboard.flavours` lists what Copy offers for the current clipboard, best first, by the Clipboard Handling
preferences: `text/plain` (the text of a type-only copy, else the SVG markup with `copyAsSvg` when the images it
embeds total at most 1 MiB, so a paste never reads a photo back as text, #2213), `image/svg+xml`
(`copyAsSvg`), `application/pdf` (`copyAsPdf`) and `image/png` (always; PDF and PNG need objects with an
area). Each one has a query command:
`clipboard.exportSvg`, `clipboard.exportText` (null unless every copied object is type), `clipboard.exportPdf`
(a one-page PDF, the page the objects' visual bounds) and `clipboard.exportPng {scale?}` (cropped to the objects,
transparent around them). As in `document.export`, the PDF's and the PNG's replies list their notes in `warnings`
(type written in the fallback font…; the PNG's also say when an image couldn't be decoded). After Copy or Cut the app
shows the first note of the PDF and PNG it offers in the status bar.

What other apps copied goes into the clipboard with `clipboard.importSvg`, `clipboard.importPdf {dataBase64, page?,
password?, editingData?}` (the page's objects; with `editingData: true` the data is a `.ai` or `.ait` file, and every
object of its editing data comes in as `document.open` reads it, each layer's objects in stacking order and a hidden
layer's objects hidden, or the page's objects when `document.open` would not use that copy; a `.ai` saved without PDF
compatibility comes in from its editing data the same way with or without `editingData`), `clipboard.importEps
{dataBase64}` (an EPS or other PostScript picture's objects, as `document.open` reads one, a hidden layer's objects
hidden), `clipboard.importImage {dataBase64, mime?}` (an embedded image at 100% of its physical size) or
`clipboard.importText {text, table?}` (point text in the default type style; cells copied from a spreadsheet,
tab-separated rows with the same number of cells, become a group `Table` of point type per cell over 0.5 pt
`Gridlines`, unless `table: false`), each centred on `center`
(default: the first artboard); then any `edit.paste*` command pastes it. The desktop app does this itself: Copy and
Cut publish the formats to the system clipboard, and a Paste outside text editing first reads what another app
copied (SVG, then PDF, then text, then a bitmap; SVG markup in text counts as SVG). A background thread makes the
formats and they are published when they are ready. While that thread has a save, an export, a print or a recovery
copy to finish, Copy and Cut make and publish them at once. When another Copy or Cut, or a copy of text in
VectorCraft, comes first, they are dropped. Pasting what VectorCraft copied keeps the lossless internal clipboard,
also before its formats are published. While a text field or the Type tool has the keyboard, the app waits for
them. Windows carries every format both ways and pastes a bitmap from PNG, else
from the device-independent bitmap screenshots copy (`CF_DIBV5`, then `CF_DIB`; alpha kept, opaque when it is zero
throughout, at most 32768 px a side); macOS and Linux carry one format (the text, else the PNG) and paste text and
bitmaps (macOS the pasteboard's PNG, else TIFF, as screenshots copy them). A Paste from the menus or the keys decodes
a bitmap (a copied image file too) on the thread that reads placed files, with "Pasting" in the status bar and the
busy spinner, and pastes it once it is decoded, in the document it was given in: Windows' device-independent bitmap and
Linux's image are encoded as PNG there, and the bitmap is read as `clipboard.importImage` reads it. The paste is
centered where the view was and goes where new art went (the current layer, the isolated group, the object Draw
Inside drew into), and Paste in Front and Paste in Back paste next
to the objects selected when it was given (a second paste given before the first is pasted goes next to the first).
A Copy or Cut made meanwhile keeps the internal clipboard it made. `edit.paste*` over MCP and scripts reads the bitmap at once.
On the web Copy publishes SVG text, and Paste takes SVG text and the pictures and files a paste carries (a screenshot pastes as an embedded image). A `.ai`
or `.ait` file that holds a PDF, copied in a file manager or carried by a paste, is loaded with `clipboard.importPdf
{editingData: true}`, and its objects go into the active layer.

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
straight paths become polylines (an open path of one straight segment becomes a line, except for a stroke weight
before 2000), curved ones cubic splines through every anchor, fills their outlines followed by
solid hatches (laser and cutter software reads the outlines and skips hatches; R12 has no hatches: the outlines
alone), strokes lines with their lineweight and a linetype per dash pattern, and placed images image entities linked
to PNG or JPEG files (`rasterFormat`) written next to the drawing (`linked` in the result). Coordinates are y up
from the bottom-left corner of the first artboard (or `artboard`), in drawing units: `scale` units per `unit` (default
1 mm = 1 unit, which sets `$INSUNITS`); `scaleLineweights` scales the lineweights with them. `colors` is `8`, `16` or
`256` indexed colours, or `true` (default; true colour with the nearest index, DXF 2004 and later). `preserve:
"appearance"` (default) writes type as glyph outlines and the strokes a CAD line can't draw (inside or outside,
width profiles, arrowheads, dashes offset into their pattern) as filled outlines; `"editability"` keeps type as text
and every stroke a line. `alterPaths` writes as filled outlines those strokes (with `"editability"` too), strokes
wider than the widest lineweight (2.11 mm), and every stroke in R13 and R14, which have no lineweights; it keeps other
strokes as lines. `outlineText` outlines type, `selectedOnly` writes only the
selected objects in their layers (every export takes it), and `useArtboards: true` writes one drawing per chosen
artboard holding the art over it. `dxfPreset: "cnc"` (CNC / CAM; default `"custom"`) sets the options for cutting
software that the call does not give: version `2000` (the oldest version that has both lightweight polylines and lineweights), `unit:
"mm"` and `scale: 1`, `colors: 256` (all version 2000 holds), `preserve: "editability"` with `alterPaths: false` (every
stroke a center line), `outlineText: true` (type as paths), and `rasterFormat: "png"`. DXF Options has the same choice as its Preset menu,
which shows Custom again once a field it sets is changed. What DXF can't hold (gradients and patterns as one colour,
blending, opacity masks, raster effects, clipping) comes back in `warnings`. In R13 and later, an image that can't be decoded is left out with
no linked file, and `warnings` carries "an image couldn't be decoded and is missing from the picture" once for all such
images.

DWG can't be written (no openly licensed writer exists): `document.formats` lists it under `unsupported` with that
hint, and exporting to it answers with the hint to export DXF. PICT files are not supported either and say so when
opened.

```json
{"name":"run_command","arguments":{"command":"document.exportDxf","params":{"path":"/tmp/plan.dxf","version":"2013","unit":"mm","scale":10,"preserve":"editability"}}}
{"name":"export","arguments":{"path":"/tmp/plan.dxf","options":{"useArtboards":true,"colors":256}}}
{"name":"run_command","arguments":{"command":"document.exportDxf","params":{"path":"/tmp/sign.dxf","dxfPreset":"cnc"}}}
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
pixelHeight, path?, found?, page?, preview?, note?}` (`note`: the image takes more than the decode allowance, 2 GiB
or 512 MiB in the web app, as RGBA or as decoded, and it is drawn at the largest size within the allowance ("shown at
reduced size"; the document keeps the whole file) or, when it can't be read that way or has more pixels than the
allowance has bytes, drawn as a crossed gray box on the canvas and left out of exports ("too large to draw"); `document.open` and `file.place` say the same in their `warnings`). `links.info {id?}` is the Link Info: `image.info`'s fields plus
`status`, `format`, the file's `ppi` and the `effectivePpi`, `scale` (% of 100%), `rotation` (degrees,
counter-clockwise), `placement` and, for a linked file, `fileName`, `location`, `fileSize`, `created`, `modified`.
`links.goTo {id}` selects an image; `links.embed {ids?}` keeps linked files' pixels in the document (an image whose
file is missing stays linked: relink it first); `links.unembed {id, path}` writes an embedded image to a file
(another extension converts it) and links to it, and without `path` returns `{name, dataBase64}`.
`links.placementOptions {ids?, preserve?: transforms|bounds|fileDimensions|fit|fill, align?: topLeftÃ¢â‚¬Â¦bottomRight,
clip?}` decides how a relinked or updated file takes an image's place (default `bounds`: stretched into the old
bounds; `clip` puts it in a clip group of the old bounds when it is larger). The UI commands `links.editOriginal` and
`links.reveal` open the linked file in its app or show it in its folder (desktop). Edit Original runs only from the app's window,
except for a placed document, which opens in a tab. With the preference `editOriginalSystemDefaults` (Preferences ›
File Handling › Use System Defaults for Edit Original, on by default) off, Edit Original opens a raster image in the app
that made it, PhotoCraft, where it is installed, and anything else in the system's default app (#5236).

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
past the nesting limit stops there, each with a line in `warnings` and the report. A `folder/name` where a packaged file
would replace the document or one of its linked files (the document's own folder, say) is an error, and nothing is
written. Every option defaults to true. Without `folder` (the web, or an agent that wants the bytes) the
result carries the same files as a zip (`{name: "<name>.zip", dataBase64}`, entries under `<name>/`).

`document.info {selectionOnly?, category?, format?: "text"}` adds `sections` (`[{id, title, rows: [[label, value]]}]`:
document, objects, graphicStyles, brushes, spotColors, patterns, gradients, symbols, fonts, fontDetails, linkedImages,
embeddedImages, repeats), `brushNames` (the Brushes panel's brushes; with `selectionOnly`, the brushes the selected
objects' strokes use), `repeatObjects` (`[{id, name, type, instances, rows?, cols?}]`: every Repeat object, those in
groups and in other repeats included, with its type `radial`, `grid` or `mirror` and the number of instances it makes,
and a grid's `rows` and `cols`; with `selectionOnly`, the Repeat objects in the selection), `patternObjects`
(`[{id, name, patterns}]`: every object filled or stroked with a pattern, once each, those in groups and type objects
whose drawn characters use a pattern included, hidden fills and strokes, strokes with no width, guides, a compound path
member's own fills and strokes, an image's fills and strokes that no Convert to Shape or Outline Object effect gives a
shape, and characters that draw nothing (spaces and line breaks without an underline or strikethrough) left out, with
the patterns it uses, which the patterns category lists as Pattern Objects and `objects.patterns` counts; with `selectionOnly`, those in the selection),
`patternNames` (the patterns those objects use) and `pathLength` (pt: the total length of the paths, compound paths' pieces included and guides left
out, which objects lists as Path Length in the document's units, say for a cut line or an LED strip along the letters);
`format: "text"` returns the plain-text report Document Info › Save… (`docInfo.save {path?, selectionOnly?, category?}`:
only `category` when given, else every category) and the package report write.

```json
{"name":"run_command","arguments":{"command":"file.package","params":{"folder":"/tmp/handoff","copyFonts":false}}}
{"name":"run_command","arguments":{"command":"document.info","params":{"format":"text","category":"fontDetails"}}}
```

## Slices

Object → Slice cuts the art into the pieces web output saves. User slices are rectangles of their own (ids from the
object id counter); `object.slice.make` turns the selected objects into object slices, whose slice follows the
object's bounds; auto slices fill what no other slice covers (the artboards with Clip to Artboard on, the default,
else the art and the slices). `slice.list` returns every slice as laid out,
numbered left to right and top to bottom: `{number, id (null for auto slices), source: user|object|auto, name, x, y,
width, height, options, selected}`, plus `clipToArtboard`, `hidden` and `locked`.

The Object → Slice commands act on the selected slices: the ones `select.object.slices` (Select → Object → Slices)
selects, and the selected objects that are object slices. `slice.list` reports both kinds as `selected`.
`object.slice.release` (an object slice leaves its object, a user slice becomes an unpainted rectangle),
`object.slice.fromGuides` (the grid the ruler guides cut the artboards into), `object.slice.fromSelection`,
`object.slice.duplicate {dx?, dy?}`, `object.slice.combine`, `object.slice.divide {rows? | rowHeight?, columns? |
columnWidth?}`, `object.slice.deleteAll`, `object.slice.options {kind?: image|noImage|htmlText, name?, url?, target?,
message?, alt?, text?, background?: ""|matte|#rrggbb (#rrggbbaa and #rgba refused), hAlign?, vAlign?}` and `object.slice.clipToArtboard {on?}` are
each one undo step. `view.slices.hide {hidden?}` and `view.slices.lock {locked?}` are session view state; the canvas
draws the slices in the `sliceLineColor` preference, numbered while `showSliceNumbers` is on. Locking or hiding
deselects the slices in every open document; unlocking or showing them again leaves them deselected and does not
restore the selection that was cleared. While the slices are locked or hidden, a command that selects slices, such as
`select.object.slices` (which replies `count: 0`), `object.slice.select`, `object.slice.create`, `edit.undo` or
`edit.redo`, leaves them out; a selected object that is an object slice stays selected.
`object.slice.clipToArtboard`, `view.slices.hide`, and `view.slices.lock` reject a flag that is not a boolean. Omitting the flag toggles its current value.

```json
{"name":"run_command","arguments":{"command":"object.slice.make","params":{}}}
{"name":"run_command","arguments":{"command":"object.slice.options","params":{"name":"logo","url":"https://example.com","alt":"Logo"}}}
{"name":"run_command","arguments":{"command":"slice.list","params":{}}}
```

The Slice tool (`slice`, Shift+K) and the Slice Selection tool (`sliceSelection`) reduce to commands:
`object.slice.create {x, y, width, height}` (the Slice tool's drag; it snaps to ruler guides), `object.slice.select
{slices?, toggle?}`, `object.slice.move {slices?, dx, dy}` (an object slice moves its object),
`object.slice.setRect {id, x, y, width, height}` (user slices) and `object.slice.delete {slices?}`. Slice ids are user
slice ids and the ids of objects with an object slice. `object.slice.select` with `slices` left out or null deselects
the slices.

```json
{"name":"run_command","arguments":{"command":"object.slice.create","params":{"x":0,"y":0,"width":200,"height":80}}}
{"name":"run_command","arguments":{"command":"object.slice.move","params":{"dx":10,"dy":0}}}
```

## Export for Screens

`document.exportForScreens` writes every chosen artboard (`range`, `artboards`; default all) in every format row
into `folder`, or returns the files as `dataBase64` without one; `zip: true` packs them into one `.zip`,
each file deflated, or stored when deflating does not make it smaller.
A row is `{format, scale?}` where `scale` is a factor (`2`, `"2x"`), a pixel width (`"100w"` or `width: 100`), a
height (`"100h"`) or a resolution (`"72ppi"`); raster rows name their files `@2x`, `@100w`… unless `suffix` says
otherwise. `fullDocument` writes one file per row instead (a PDF of every artboard, other formats the bounds of all
art), `includeBleed` grows each artboard by the document's bleed, `includeArtboardFill: false` (Include Artboard
Fill unchecked) leaves the artboards' background colors out of the files, `pdfs: "single"` (Export PDFs As › Single file)
writes the artboards a PDF row exports as the pages of one PDF named after the document, `subfolders` puts each
row's files in a sub-folder (its size or format, or its own `folder`; `folders: "format"` sorts by format instead), `preset: "mobile"` or `"density"` (Android-style ldpi…xxxhdpi
sub-folders) replaces the rows, and `settings: {png: {…}, jpg: {…}, svg: {…}, pdf: {preset}}` gives every row of a
format its options. On every platform, `/`, `\`, `:`, `<`, `>`, `"`, `|`, `?`, `*` and control characters from a
prefix, a suffix, a name pattern or an item's name become `-` in file and folder names, a folder name ends without
periods or spaces, and a Windows device name (`CON`, `PRN`, `AUX`, `NUL`, `COM1` to `COM9`, `COM¹` to `COM³`, `LPT1` to `LPT9`, `LPT¹` to `LPT³`) that
is the whole name or comes before its first period gets a `-` after it (`CON-.png`). When a row's file would take a
name another row's file already has, compared without case (two PNG rows with different backgrounds and no suffix),
the later file is written as `-2`, `-3`… (`Artboard-1-2.png`) and listed under that name in `files`; two identical
rows write one file, and row folders that differ only in case (`Print`, `print`) are one folder, spelled as the first
row spells it. The document remembers the last params: `document.exportSettings`. As in `document.export`,
the reply's `warnings` lists the notes of every file exported (an image that couldn't be decoded, type written in the
fallback font…), each note once; the export still writes or returns every file.

Batch exports (#1935):
- **What:** `layers: [id…]` (each layer's art alone) or `selection: "one" | "each"` instead of artboards
  (`assets`, `fullDocument` as before).
- **Names:** `name` is a pattern for every file: `<filename>` (the document's file name without its extension, or
  its title before it is saved; the Full Document file and the `.zip` use the same name), `<name>` (the artboard,
  asset, layer or object), `<index>`, `####` (a counter zero-padded to the run of `#`, from `counterStart`; `counterPer: "item"`
  or `"file"`), `YYYYMMDD`, `YYYY-MM-DD`, `<date>`, `<time>`, `<format>`, `<scale>` and `<suffix>` (else the row's
  suffix ends the name; `date` sets the day the pattern uses). Different files it names alike get `-2`, `-3`….
- **Folders:** `folders: "none" | "rows" | "format" | "item" | "itemFormat"` (all in one, a folder per row, per
  extension, per item, or item then extension).
- **Formats:** rows may also be `eps`, `ai` and `vectorcraft` (a document of the item alone). A PDF row with
  `marks` (`{trim, registration, colorBars, pageInfo}`) and a suffix such as `-print` sits next to a plain PDF row.
- **Presets:** `document.exportPresets.list` (built-in: Logo Package, Print-Ready PDF), `.save {name, settings}`
  (kept with the preferences; a saved name is replaced) and `.delete {name}`; `exportPreset: "<name>"` uses one under
  the call's own params. `.export {names?, path?}` writes presets (every saved one, or the named ones, built-in ones too)
  as a `.vcexportpresets` JSON file (without `path` it returns `data`; more than 1000 presets or a file larger than 4194304 bytes is an error), and `.import {path? | data? | dataBase64?,
  replace?}` adds a file's presets (a name in use gets a number unless `replace`; a built-in name always gets one; a
  name that already ends in a number gets the next free one, so `Web 2` comes in as `Web 3`; a file with settings
  `.save` refuses, with more than 1000 presets, or larger than 4194304 bytes adds none). A preset name, saved or imported, keeps its first 255
  characters. Opening a `.vcexportpresets` file imports it.
  The Asset Export panel menu lists presets and holds Save As Preset…, Save Changes, Delete Preset, Import Preset…
  and Export Preset…. `assets.settings.set {exportPreset}` applies a preset to the settings the panel shares: its rows or
  `preset`, its format settings, its prefix (none when it has none), its `folders` (Create Sub-folders is on for `rows`
  only) and anti-aliasing. With Create Sub-folders off, `assets.export` writes every file into the folder itself,
  mobile and density rows included.

```json
{"name":"run_command","arguments":{"command":"document.exportForScreens","params":{"range":"1-3","name":"COMPANY-Logo-<filename>-####-YYYYMMDD","folders":"format","formats":[{"format":"svg"},{"format":"png"},{"format":"pdf"},{"format":"pdf","suffix":"-print","marks":{"trim":true,"registration":true,"colorBars":true,"pageInfo":true}}]}}}
{"name":"run_command","arguments":{"command":"document.exportForScreens","params":{"exportPreset":"Logo Package","layers":[2,9]}}}
{"name":"run_command","arguments":{"command":"document.exportForScreens","params":{"preset":"density","zip":true}}}
{"name":"run_command","arguments":{"command":"document.exportForScreens","params":{"range":"1-2","formats":[{"format":"png","scale":"512w"},{"format":"jpg","quality":80,"scale":"2x"},{"format":"svg"}],"settings":{"png":{"background":"white"}}}}}
{"name":"run_command","arguments":{"command":"document.exportSettings","params":{}}}
```

## EPS export

`document.exportEps` (also `document.export` / `export` with format `eps`) writes Encapsulated PostScript: a DSC header
(`%!PS-Adobe-3.0 EPSF-3.0`, `%%BoundingBox`, `%%HiResBoundingBox`, `%%LanguageLevel`, spot colours as
`%%DocumentCustomColors`), then the page. Without `useArtboards` the file covers the visible art, its bounding box y up
from the first artboard's bottom-left corner; `useArtboards: true` (or `range`, `artboards`, `artboard`) writes one file
per artboard, `{stem}_{artboard}.eps`, each bounded by its artboard. When it writes two or more artboards, it also
writes a central file first under the path's own name, `{stem}.eps`: the whole document, bounded by the visible art as
without `useArtboards` (no central file when no art shows). `level: 3` (default) writes gradients as smooth
shadings and masks transparent image pixels out; `level: 2` writes gradients as bands of colour (as does
`compatibleGradients`) and transparent pixels white. Type is always glyph outlines, so no fonts are needed (`embedFonts`
is accepted); spot colours are Separation colour spaces, and overprinting fills and strokes overprint unless
`overprints: "discard"`. RGB documents are written in CMYK unless `cmykPostScript: false`. When the file would hold an
image that can't be decoded, the image is left out, and `warnings` carries "an image couldn't be decoded and is missing
from the picture" once for all such images.
Colorized grayscale and 1-bit images use their process or spot ink; the flattener leaves them alone unless they have opacity, a blending mode, an opacity mask or effects, or overlap other transparent art. Binary coverage leaves underlying art untouched. For a grayscale image, Level 3 leaves white (zero-coverage) pixels out, Level 2 writes them white, and gray midtones are written over white at both levels.

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

File → Print lays the job out as a print-ready PDF, in which each page is a sheet of the chosen paper. The desktop app
sends the PDF to the system's print service, and the web build opens the browser's print dialog on it. With PDF File
picked as the printer, File → Print saves the PDF instead (see Printing). Headless, `file.print {settings?, path?}` →
`{pages, path | dataBase64, bytes, warnings}`; `print.preview {settings?}`
answers what it would print without writing it: `{pages (copies included), sheets: [{artboard, tile?, ink?, width,
height, orientation, scale}] (one copy, in order), tiles: [{artboard, columns, rows, printed, tiles}] (document
space), inks: [{name, spot, print, frequency, angle}], warnings, settings}`. `print.setup {settings?}` saves the
settings with the document (one undo step, kept in the native file as `print_setup`); both other commands start from
the saved settings, or from a print preset's with `preset` (see Print presets), with `settings` over them (`null` keeps
a value). `list_commands` with filter `print.setup`
documents every field:

- General: `copies` (1–999), `collate`, `reverse`; `artboards: all|range|ignore` with `range: "1-3, 5"` (ignore: all
  the art as one page), `skipBlank`; `media: letter|legal|tabloid|a3|a4|a5|a6|b4|b5|b4 (jis)|b5 (jis)|b6 (jis)|japanese postcard|japanese double postcard|japanese envelope #3|japanese envelope #4|kaku #2|kaku #3|yonroku-ban|custom` (`width`, `height` in pt; a paper is named by its id or its dialog label, any case and spacing, so `B5 (JIS)` works; `b4` and `b5` stay ISO 216 and the JIS sizes are `b4 (jis)`, `b5 (jis)` and `b6 (jis)`),
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
as they do for art larger than the imageable area. Type in a font family that isn't available prints in the fallback
font, and the warnings of `file.print` and `print.preview` name those families. They count only type that prints:
visible type that fits its frame or path, on the layers `printLayers` picks, on the paper of the artboards or tiles
that print, and the type in the symbols, patterns, scatter brushes, opacity masks and placed documents of the art that
prints. The warnings also give the ids of the area type and type on a path that print and whose text doesn't fit
(objects of the document only, not type in the art of symbols, patterns, brushes, opacity masks or placed documents;
see [Fitting area type](#fitting-area-type-auto-size-shrink-text-to-fit-overflow)).

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
none when the app runs with `VECTORCRAFT_NO_PREFS`) or, on the web, in browser storage. `prefs.set` returns an error
and changes nothing when a new `recoveryFolder` is missing, is not a folder, can't be listed, or is a folder that a
folder can't be created in. It does not check an empty value or the folder the preference already holds, and the web
build does not check folders. The folder in the example below must exist before the call.

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
- `pdfCompatible`: native files (default true) also carry a PDF of every artboard (`pdf` in the file). This makes saves
  larger and adds PDF rendering work. Set it to false to leave the PDF out; Save remembers that choice. `.ai` files
  (default true) with false write blank pages around the native document (smaller; other apps show empty pages).
  Native files without artboards and Data Recovery copies omit the PDF.
  Reopening a native file keeps its saved PDF preference, falling back to whether it declared a PDF in older files.
  Existing files saved without a PDF remain compact.
- `.ai` only: `compress` (default true) is the PDF option `compression.compressText`.
- The editing copy of a `.ai` or `.ait` file (`editingData`) names its active artboard (`CropAreaActive`): the active
  artboard of the app's view when the app saves or exports the file, otherwise the artboard the document opened on (the
  active artboard of the `.ai` file it was opened from), else the first. A `.ai` file read from its editing copy
  opens on that artboard, and so does a `.ai` file restored from the native document it carries.

File → Save As asks for these in the save options dialog (VectorCraft Options) after the save panel, and Save a Copy
does too for a `.ai` file; Save, Save a Copy to a native file and Save as Template reuse the options as last saved (or Use Compression).

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
white and gradients and patterns as one colour. What a format leaves out comes back in `warnings`. When the picture
would hold an image that can't be decoded, the image is left out, and `warnings` carries "an image couldn't be decoded
and is missing from the picture" once for all such images.

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
on white. TIFF, BMP and Targa files open again (a 16-bit Targa file with its one-bit alpha); `document.formats` lists
every option.

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
GIF/JPEG comment; `convertToSrgb` marks PNGs as sRGB. `<stem>` is the name of `path` without its extension, or
without `path` the document's title with the characters Windows does not allow in a file name as `-` and a `-` after
a Windows device name. With slices (`slice.list`) each image slice becomes `images/<slice name>.<ext>`, with a `-`
after a Windows device name such as `aux` (`slices: selected` writes the selected ones, `none` one image);
`output: html` adds
`<stem>.html` placing them, with each slice's URL, target, alt text and status message, and No Image / HTML Text cells.
Without `path` the files come back as `{name, dataBase64}`. As in the raster exports, `warnings` says when an image
couldn't be decoded and is missing from the image; the files are written all the same.

`document.exportForWeb.preview` returns what Save writes for the same settings without writing it: `bytes` (the exact
file size), `seconds` to download at `kbps` (default 56.6), the size, and for palette formats the colour table
(`colors: [{color, source, transparent, locked, webShifted, webSafe}]`). The colour table's lists name colours by
`source` (the colour the reduction made), so a colour keeps its edits when it is web-shifted or snapped. `slice: n`
previews one slice; `image: true` adds the file; `warnings` holds the export's warnings.

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

Assets are pieces of art collected for export on their own (Window › Asset Export, Object › Collect for Export, the
Layers panel ≡ menu and row menu, File › Export Selection…).
`assets.add {ids?, multiple?}` collects the selected objects (or `ids`): one asset per object, or with
`multiple: false` one asset of them all, named after the object or `Asset 1`, `Asset 2`…; a layer id collects the art
under it (sublayers included). Art already collected keeps its asset. Assets name their objects, so they follow edits:
moving the art moves the crop, and deleting it removes it from its asset (an asset left with nothing goes).
`assets.list` gives each asset's id, name, objects and crop (`bounds`); `assets.rename {asset, name}` and
`assets.remove {assets}` are undo steps.

`assets.export {assets?, folder?, zip?}` writes each asset's art alone, cropped to it and named after it, in every
format row of the document's export settings, the ones Export for Screens remembers (`document.exportSettings`); any
`formats`, `preset`, `settings`, `prefix` or `subfolders` given win for that export. `assets.settings.set` changes
those shared settings (not an undo step); its `subfolders: true` sets Export for Screens' Folders to a folder per row,
and `subfolders: false` sets a folder per row back to All in One Folder. `document.exportForScreens {assets: [id…]}` does the same with its own params
(the dialog's Assets tab). An asset's crop, and the crop of `export` with `selection: true`, is the bounds the
renderer measures for what the art draws, its effects included.

```json
{"name":"run_command","arguments":{"command":"assets.add","params":{"multiple":true}}}
{"name":"run_command","arguments":{"command":"assets.add","params":{"ids":[12],"multiple":true}}}
{"name":"run_command","arguments":{"command":"assets.settings.set","params":{"formats":[{"format":"png","scale":"1x"},{"format":"png","scale":"2x"},{"format":"svg"}]}}}
{"name":"run_command","arguments":{"command":"assets.export","params":{"zip":true}}}
```
A layer id in `ids` (12 above) is the same as Collect for Export on that Layers row: every object under the layer
becomes an asset (or one asset with `multiple: false`).

### 3D assets

Asset Export's menu item Export 3D Object (`assets.add3D {ids?, format?}`) collects each selected object (or `ids`)
that carries Extrude & Bevel, Revolve, Inflate or Rotate as a 3D asset, named after the object; an object already
collected as a 3D asset keeps it, and an object without such an effect is refused. A 3D asset's Format is `usda` (the
default), `usdz`, `gltf` or `obj`; the panel shows a Format row for the 3D assets it would export, and
`assets.set3DFormat {assets, format}` sets it (an undo step only when the format changes). `assets.list` gives each
asset's `format3D` (null for a picture asset).

`assets.export3D {assets?, folder?, zip?}` writes each 3D asset (default: all of them) as a mesh file named after it:
`.usda`, `.usdz`, `.gltf` (glTF 2.0, its buffer embedded) or `.obj` with its `.mtl` (whitespace and `#` in an OBJ's
name become `_`). Every solid of the object (the
fill's, each stroke's, a group's members') is its own mesh, with a two-sided material in the color it is painted
(a gradient's stops averaged, a pattern gray). The mesh holds the solid's faces before they are projected onto the
page, those turned away from the viewer included, at the object's 3D rotation, y up, about the point the object turns
about: in points (USD's layer gives `metersPerUnit`), in meters in glTF. Files that would share a name get `-2`, `-3` and so on.
With `zip`, the files go into one ZIP named after the document with `-3D` added (`Untitled-1-3D.zip`), or
`3D Assets.zip` when the document's title gives no file name.
An object whose face has a hole that crosses the face's outline or another hole is refused, by `assets.add3D` and
`assets.export3D` alike, rather than exported with the hole filled. So is an object whose meshes would hold more than
2,000,000 triangles together, VectorCraft's own limit.
`assets.export` leaves 3D assets out and refuses one named in its `assets`. On the 3D export page
(<https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/export-3d-vector-artwork.html>)
the 3D asset's Format is picked in Asset Export, with USDA first; the file layouts follow the glTF 2.0 specification
(<https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html>) and OpenUSD (<https://openusd.org/release/index.html>).

```json
{"name":"run_command","arguments":{"command":"assets.add3D","params":{"ids":[12],"format":"gltf"}}}
{"name":"run_command","arguments":{"command":"assets.set3DFormat","params":{"assets":[40],"format":"obj"}}}
{"name":"run_command","arguments":{"command":"assets.export3D","params":{"folder":"/tmp/meshes"}}}
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
writes the `.css` file with the PNGs of rasterized art next to it (no path: `data` and `images` as base64). Its
`warnings` add the PNGs' notes to the CSS's, each once (an image that couldn't be decoded, type written in the
fallback font…). In the app, `css.copy` copies the CSS and `css.exportFile` writes it through a save panel (the web
downloads it) and shows the first note in the status bar.

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
unless `replace`). Saved presets live with the preferences. `print.setup`, `print.preview` and `file.print` take
`preset`, the name of `[Default]` or of a saved preset in any case: the preset's settings are used instead of the
document's, with `settings` over them, and `print.setup {preset}` saves them with the document. An unknown name is
refused with the list of presets.

```json
{"name":"run_command","arguments":{"command":"print.presets.save","params":{"name":"Posters","settings":{"scaling":"tileImageable","overlap":18,"marks":{"trim":true}}}}}
{"name":"run_command","arguments":{"command":"file.print","params":{"preset":"Posters","path":"/tmp/posters.pdf"}}}
```

## EPS and PostScript import

`document.open` and `file.place` read `.eps`, `.epsf` and `.ps` files, and PostScript by any name (an `.ai` saved
without PDF compatibility opens as artwork; an `.ait` one as a new untitled document). A `.ps` file reports the format
`ps`, which `document.export` doesn't write (`file.print` writes PostScript). An EPS file VectorCraft wrote restores the
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
refused with a message saying why. A PostScript error is named as PostScript spells it, followed by what it means:
``a PostScript error (typecheck in `add`; a value has the wrong type for the operator)``. The status bar shows
the meaning in the interface language, and the name stays as it is.

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
`view.printTiling` rejects an `on` value that is not a boolean. Omitting `on` toggles the current value.

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
as Overprint Preview shows them (darkened); either way the file has no overprint left. Separations always keep
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
square to the page. `object.resetBoundingBox` squares the box again without moving the art. It also squares every
object inside the selected ones: the children of groups, compound paths, clip groups, blends, envelopes, repeats and
intertwines, and the art of linked opacity masks (an unlinked mask's art keeps its angle). Ungroup, Release Clipping
Mask, Release Compound Path, Blend Expand, Release Envelope, Release Repeat, Release Intertwine and Release Opacity Mask
then bring back boxes square to the page.

```json
{"name":"run_command","arguments":{"command":"object.rotate","params":{"angle":45,"absolute":true}}}
```

## Select All while editing type

While the Type tool edits text (a click into type, `pointer_gesture` with `"tool":"type"`), `select.all` (Cmd+A,
`press_key {key: "A", mods: {cmd: true}}`) selects all of that text instead of the art, as in the reference app, and
returns `{editing, start, end}` (the text's id and the selected byte range, which `text.getRange` and
`text.setRangeStyle` take); the art selection stays as it is. Without text being edited it selects every object and
returns `{count}`. In a frame of threaded text, `select.all` selects the whole story: the Type tool then edits the
thread's first frame that holds text (which is selected when it was not) with the text of every frame selected, and the reply adds
`story`, the frames in thread order. `text.getRange {id, story: true}` returns the story's text and runs, which Copy
and Cut take. Typing, Delete and Cut replace the whole story as one undo step (`text.editRange` with `clear`, the other
frames whose text goes), and a fill, stroke, `type.step` or Character or Paragraph panel change applies to the text of
every frame (the Character panel sends `text.setRangeStyle {id, story: true}`, which styles every frame's text in one
undo step). Remove Line Breaks, Remove Paragraph Breaks and Convert to Text also act on the text of every frame, as
one undo step. The panels change the frames that are in the edited frame's thread at the time of the change, locked and
hidden frames included, and not a frame that has left it. A caret move or a click ends the story selection.
one undo step. A caret move or a click ends the story selection. Switching to the Eyedropper keeps the story selected,
and switching back to a Type tool restores it. `appearance.copyFrom` and `paint.sampleColor` then apply to the text of
every frame as one undo step, and `appearance.copyFrom` leaves out a frame that is its `source`. While the story stays
selected, its frames are the ones threaded with the edited frame at that moment: a frame that `text.thread.add` adds is
highlighted and takes the edit, and a frame that leaves the thread is not highlighted and keeps its text. The story
stays selected when a frame added before the edited frame takes all of its text. Once a thread is removed, typing replaces the text of the edited frame only.

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
frame anchors with their handles (Direct Selection dragging a corner, an edge's two ends or the frame anchors its
marquee selected, and the arrow keys nudging those); `{items: [{id, anchors}…], dx, dy}` moves the anchors of several
frames in one step, and
`text.areaOptions {width?, height?}` sizes the area from its top-left corner (the query returns both). Each is one
undo step. Object › Transform › Scale, the Scale tool and the Transform panel still scale the type.
`text.reshapePath {id, anchors: [[subpath, anchor]…], dx, dy}` does the same for the path of type on a path
(Direct Selection dragging one of its anchors): the start and end brackets keep their fractions of the path and the
text reflows along it.

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

### Text flow

`text.areaOptions {flow: columns|rows}` (Area Type Options › Text Flow) sets the order area type with several rows
and columns fills its cells: `columns` (the default) runs down each column's rows, then on to the next column;
`rows` runs across each row's columns, then on to the next row. The reply reports `flow`; other values are refused.
Native files save it only when it is `rows`, so files from before it open by columns.

```json
{"name":"run_command","arguments":{"command":"text.areaOptions","params":{"rows":2,"columns":3,"flow":"rows"}}}
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

Given for several area types (`ids`, or a selection of several), `text.areaOptions` changes only the options given on
each one: each frame keeps its other options (it used to take the first frame's), and `fitMinPercent` alone changes only
the frames that already shrink text to fit. Options that change nothing (the vertical alignment already set, say) add
no undo step and keep redo.

Text that doesn't fit is drawn up to the last line that does, so a one-line headline in a frame lower than its line
draws nothing. The commands that make type or change what it holds say so in their own reply: `text.create`,
`text.createInPath`, `text.setText` and `text.setStyle` add `overflow: [id…]` (the area type and type on a path among
the objects they made or changed whose text doesn't fit), and the `add_text` tool adds `overflow: true`. The key is
left out when everything fits. `text.create` and `text.createInPath` also return `bounds: {x, y, width, height}`, the new type's box
as `document.inspect` reports it (after `justify`), so a script can place the type without a helper object (#2413). The warnings of `file.print` and `print.preview` give the ids of the objects that print
and whose text doesn't fit: the first 20 ids, then how many more.

In PDF, EPS, SVG, EMF, WMF and the raster formats, the warnings of `document.export`, `document.exportPdf` and
`document.exportEps` give the ids in the same way for the visible objects that the export draws and whose text doesn't
fit. The export draws the objects on the pages or artboards it writes; an EPS of the visible art and an SVG without
chosen artboards draw all of them. A PDF leaves out the layers that don't print unless `includeNonPrinting` or
`createLayers` is set. A `.ai` file keeps the whole text in its editing data and gives no such warning.

```json
{"name":"run_command","arguments":{"command":"text.areaOptions","params":{"fit":"shrinkText","fitMinPercent":60}}}
```

## Threading text from the in and out ports

As in the reference app, selected area type and type on a path show an in port and an out port: area type's on the
frame's left side just below the top-left corner and on its right side just above the bottom-right corner (so the
corner handles stay free), type on a path's just before its start bracket and just past its end bracket. A port is
empty while the frame isn't threaded on that side, shows an arrow when it is, and the out port shows a red "+" when
the text doesn't fit (with more than 1,000 such objects selected no ports are drawn). A click with the Selection tool
on the out port loads the pointer with the text. The next click
threads it into the closed path, area type or type on a path under the pointer, which becomes the next frame; a click
on empty canvas makes a new frame of the first one's kind, shape and size with its top-left corner at the click, and a
drag draws a rectangle for the new frame. A click on the in port works the same way, but the frame placed comes
before the clicked one: the story starts there and flows on into it. Escape or another tool drops the loaded text.
Each placement is one undo step (`text.thread.add`), which agents can run directly:

`text.thread.add {from, to? | rect?: [x, y, width, height] | at?: [x, y], before?}` threads area type or type on a
path `from` into one more frame, put right after it in its thread (a new thread of the two when `from` has none), or
right before it with `before: true` (the in port): `to`, a closed path (converted to an empty frame), area type or
type on a path that is not threaded yet; `rect`, a new rectangular frame (positive width and height); or `at`, a new
frame of `from`'s kind, shape and size whose top-left is at `at`. Give exactly one of the three. New frames take
`from`'s type options, paint and first character and paragraph attributes, and go just above it. The story reflows
through the thread, the two frames are selected, and it returns `{id}`, the frame threaded into. It refuses an `id`
that doesn't exist, a `from` that is point type, a `to` that is an open path, `from` itself, inside a compound path
or already in a thread (`from` may be threaded already: the frame goes in beside it), coordinates that aren't finite
numbers and a `before` that isn't true or false.

A double-click on a port where the frame is threaded breaks the thread there (one undo step, `text.thread.break`):

`text.thread.break {id, port: "in" | "out"}` splits the thread of area type or type on a path `id` before it (`in`) or
after it (`out`). The whole story flows into the frames before the break; the frames after it are left empty, still
threaded with each other when two or more remain, and the first of them starts a paragraph. It refuses an `id` that
doesn't exist, a `port` other than `in` or `out`, and a side where no frame of a thread comes before or after `id`.

`select.same.linkBlockSeries` (Select › Same › Link Block Series) selects every frame of the threads that the selected
type objects, and the type inside selected groups, are in, in thread order, and returns `{count}`. Type in no thread
selects itself, and other selected objects are left out. With frames of several threads selected, it selects the frames
of all of them. Type that is hidden or locked, inside a hidden or locked group or layer, on a template layer, or outside
the isolated group does not count as selected, and such frames of a thread are left out. It is off while no other type
is selected, and records no undo step.

In every thread, a paragraph that continues from one frame into the next has its first line, first-line indent and
bullet or number in the frame where it starts. In the next frame its first line starts where its wrapped lines start,
and it has no bullet or number. In a frame that is no longer threaded (after `text.thread.releaseSelection` or
`text.thread.remove`), in a copy of a frame and in point type converted from one, the first paragraph has its first-line
indent and bullet or number again.

```json
{"name":"run_command","arguments":{"command":"text.thread.add","params":{"from":42,"at":[300,120]}}}
```

`type.createOutlines` (Type › Create Outlines) and `effect.expandAppearance` (when it outlines type, as Expand
Appearance describes) take an outlined frame of threaded text out of its thread with the text it shows, and the other
frames keep the text they hold. When the thread's last frame is outlined, the text it has no room for, which its
outlines do not draw, goes after the text of the last frame that stays. A thread left with one frame is no longer a
thread. When `effect.expandAppearance` keeps a frame as type in a group (type with no fills or strokes of its own whose only
effects are shadows, outer glows or Crop Marks), the type stays in the thread in the frame's place. Object › Expand
(`object.expand`) outlines type with these two commands. `object.flattenTransparency` takes each frame of threaded text
that it outlines (`textToOutlines`) or turns into flat regions or an image out of its thread in the same way.
`symbol.new` (New Symbol) and `symbol.update` (Redefine Symbol) take each frame of threaded text in the selection out
of its thread in the same way, and the symbol's art holds a copy of the frame with the text it shows. When every frame
of a thread goes into the symbol, the copy of the last frame keeps the text it has no room for.

## Converting between point type and area type

With the Selection tool, a single selected point or area type object shows the type widget: a small circle beside
the middle of its bounding box's right side, hollow on point type and filled on area type. Double-clicking it runs
`type.convertToAreaType` (point type gets a frame around its text, which keeps its place and doesn't rewrap) or
`type.convertToPointType` (soft line wraps become line breaks), the same commands as the Type menu's Convert To Area
Type and Convert To Point Type. Either keeps the text and its styles and is one undo step; `ids` picks the objects
(default: the selection). A frame of threaded text that `type.convertToPointType` converts leaves its thread with the
text it holds, and the story flows through the other frames without that text; a thread left with one frame is no
longer a thread.

```json
{"name":"run_command","arguments":{"command":"type.convertToAreaType","params":{"ids":[42]}}}
```

## Hyphenation options

`text.setFormat` sets the Paragraph panel menu's Hyphenation… options of the selected type (or `ids`, or the
paragraphs a `start` / `end` range touches), which apply while `hyphenate` is on:

- `wordsLongerThan` (letters, 1 to 25, default 5): only words with more letters than this are hyphenated.
- `afterFirst` and `beforeLast` (letters, 1 to 15, default 3): the fewest letters a word keeps before and after the
  hyphen.
- `hyphenLimit` (lines, 0 to 25, default 0): the most consecutive lines that may end with a hyphen; 0 sets no limit. A
  line that ends at a soft hyphen counts, and a soft hyphen still breaks a line past the limit.
- `hyphenationZone` (pt, 0 to 1296, default 0): with the Single-line Composer, a line whose text before the next word
  already ends within this distance of the right edge breaks before that word instead of hyphenating it. The Every-line
  Composer does not use it.
- `hyphenateCapitalized` (default `true`): `false` leaves words that start with a capital letter whole. Words in
  capitals only are never hyphenated.

The letter and line counts are rounded, and every value is clamped to its range. Preferences › Hyphenation › Exceptions still decide the breaks of a
listed word. The options are saved with the paragraph (`hyphenation` in `para`) only when they differ from the
defaults, and paragraph styles carry them (`paraStyle.new {attrs: {hyphenation: {words_longer_than: 8}}}`). One undo
step. `ui.hyphenation` opens the Hyphenation dialog on the values the Paragraph panel shows; OK sends the values that
changed. The UI command `type.hyphenate` (Cmd+Alt+Shift+H, in remote mode `press_key {key: "H", mods: {cmd: true,
alt: true, shift: true}}` or `invoke_menu`) does what the Hyphenate checkbox does: it turns `hyphenate` off when
every paragraph of the selected type (or every paragraph the Type tool's selection touches) hyphenates, else on, and
returns the new value.

```json
{"name":"run_command","arguments":{"command":"text.setFormat","params":{"hyphenate":true,"wordsLongerThan":8,"hyphenLimit":2}}}
```


## Justification options

`text.setFormat` sets the Paragraph panel menu's Justification… options of the selected type (or `ids`, or the
paragraphs a `start` / `end` range touches), in percent:

- `wordSpacing` (0 to 1000, default 80 / 100 / 133), `letterSpacing` (-100 to 500, a share of the font's word space,
  default 0 / 0 / 0) and `glyphScaling` (50 to 200, default 100 / 100 / 100), each `[min, desired, max]` or
  `{min?, desired?, max?}` (the values not given are kept). Desired applies to every line: word spaces are that share
  of the font's word space, each letter followed by another is followed by that share of a word space, and glyphs are
  that wide. A justified line may move them between Minimum and Maximum to fill its measure: word spacing first, then
  letter spacing, then glyph scaling; past all three its word spaces take the rest. Values are clamped to their range,
  and Minimum ≤ Desired ≤ Maximum must hold in every paragraph afterwards, or nothing changes and the command fails.
- `autoLeading` (0 to 500, default 120): the leading of characters whose leading is Auto, as a share of their size.
- `singleWordJustification` (`"fullJustify"` (default), `"alignLeft"`, `"alignCenter"` or `"alignRight"`): how a
  justified line holding a single word is set.

The options are saved with the paragraph (`justification` in `para`) only when they differ from the defaults, and
paragraph styles carry them (`paraStyle.new {attrs: {justification: {auto_leading: 150}}}`). Vertical type isn't glyph
scaled, and type on a path doesn't use them. One undo step. `ui.justification` opens the Justification dialog on the
values the Paragraph panel shows, with Preview; OK sends the values that changed.

```json
{"name":"run_command","arguments":{"command":"text.setFormat","params":{"wordSpacing":[80,100,150],"letterSpacing":{"max":5},"singleWordJustification":"alignLeft"}}}
```

## Insert Kashidas

`text.setFormat {kashida}` (Paragraph panel › Insert Kashidas, shown with the Indic options) sets how the justified
lines of the paragraphs of the selected type (or `ids`, or the paragraphs a `start` / `end` range touches) lengthen
their Arabic words: `"none"` (the default), `"short"`, `"medium"`, `"long"` or `"stylistic"`. On a justified line of
horizontal area type that is shorter than its measure, each Arabic word with joined letters gets one kashida between its
last pair of them: a tatweel (U+0640) in the font of the letter after it. Words end at spaces, inline graphics and
characters other than letters and marks, such as punctuation, digits and symbols. The kashidas share the line's room
equally, each at most a quarter of an em (short), half an em (medium) or an em (long) of that letter's size at its
horizontal scale; a stylistic kashida is a row of the font's own tatweels, as many whole ones as fit in its share and in
an em. The word spaces and the rest of justification take the room the kashidas leave, so the line keeps its width.
Line breaks are the same as with none. Lines that are not justified, words without joined letters and fonts without a
tatweel get no kashidas. Each tatweel is a glyph of the layout that stands for no character and does not advance; the
letter after the kashida advances over it, so the caret before that letter, a selection of it and a click on it take
the kashida in. The setting is saved with the paragraph (`kashida` in `para`) only when it is not none, paragraph
styles carry it, and Reset Panel sets it back to none. The change is one undo step.

```json
{"name":"run_command","arguments":{"command":"text.setFormat","params":{"kashida":"medium"}}}
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
`text.create`) takes `standard`; documents from before it and imported text keep `none`, except legacy `.ai` type
with binding punctuation (`Tg`) on, which opens as `forced`. `none` isn't saved. One undo step.

```json
{"name":"run_command","arguments":{"command":"text.setFormat","params":{"burasagari":"forced"}}}
```

## Tate-chu-yoko Settings

`text.setFormat` and `text.setRangeStyle {tcyUpDown?, tcyLeftRight?}` set the Character panel menu's Tate-chu-yoko
Settings of the characters, in points from -1296 to 1296 (document points, as `baselineShift`). In vertical type a
block of tate-chu-yoko characters (`tateChuYoko`) moves up by a positive Up/Down and down by a negative one, and right
by a positive Left/Right and left by a negative one. The block moves as one piece with the values of its first
character, and keeps its one em down the column, its caret positions and its hit region. Two or three digits set
across the column without the mark do not move, and horizontal type keeps the values without moving the characters.
Neither value turns `tateChuYoko` on. Both are 0 by default and saved only when not 0, and character styles carry
them. A value that is not a number is refused with `` `tcyUpDown` must be a number ``, and one outside the range with
`` `tcyUpDown` must be from -1296 to 1296 ``, naming the key. One undo step. `ui.tateChuYokoSettings` opens the
Tate-chu-yoko Settings dialog (the menu item shows with Show East Asian Options) on the values the selected characters
share, a blank field where they differ; OK sends the values that changed and leaves a blank field's value alone. OK
with a field that is not a length returns `Up/Down must be a number` or `Left/Right must be a number` and keeps the
dialog open.

```json
{"name":"run_command","arguments":{"command":"text.setRangeStyle","params":{"id":12,"start":0,"end":4,"tateChuYoko":true,"tcyUpDown":2,"tcyLeftRight":-1}}}
```

## Composite Fonts

A composite font sets each class of character in a font of its own
([Creating composite fonts](https://help.adobe.com/archive/en_US/illustrator/cs3/illustrator_cs3_help.pdf#page=347)).
`type.compositeFonts.set {fonts: [{name, parts}]}` replaces the active document's composite fonts as one undo step,
and adds no step when nothing changes. `parts` holds `kanji` (the base font), `kana`, `punctuation`, `symbols`, `roman`
and `numerals`, each `{font, style?, size?, baseline?, hScale?, vScale?, scaleAtCenter?}`; a part left out copies the
`kanji` part. Which characters fall in each class is VectorCraft's own rule: Kanji takes ideographs, full-width
letters and digits and every character no other class takes; Kana takes hiragana and katakana of both widths;
Punctuation takes East Asian and full-width punctuation, quotes and dashes; Symbols takes arrows, math, shapes and
other symbols; Roman takes half-width letters, their punctuation and spaces; Numerals takes the digits 0 to 9.

`size` (10 to 400, 100 by default) and `baseline` (-100 to 100, 0 by default; positive raises) are percentages of the
type's size, VectorCraft's own ranges. `hScale` and `vScale` (10 to 400, 100 by default) scale Kana, Roman and
Numerals only, and `scaleAtCenter` (Kana only) scales kana from the center of the em box instead of the baseline. A
value out of range, a scale on another class, an unknown class or key, a name that is empty, longer than 255
characters, shared by two fonts (in any case) or the name of an installed font, and a part set in a composite font are
each refused with an error, and nothing changes. A document keeps at most 256.

Type set in a composite font names it as its family (`text.setStyle {font: "My Composite"}`), and the font menus list
composite fonts first. The layout sets each character in its class's font, a letter and its combining marks together.
A composite font taken out of the document leaves its type naming it, and the type shows as missing. Composite fonts
are saved with the document. SVG and legacy `.ai` write type in one of the document's composite fonts as outlines, and
SVG, PDF, EPS and legacy `.ai` export draw it in the font's parts also when the document was read without an open app.
`type.compositeFonts.list` returns the document's composite fonts.

`text.replaceFont` and `addon.cleanup.replaceFonts` with a composite font as the new font set the type in it by its own
name, and the type keeps its style. `text.fonts` lists a composite font by its name; its `status` is the worst among the
parts its characters are drawn in, and `missingGlyphs` counts the characters that their part's font lacks.
`text.missingFonts`, File › Package and the warnings about fonts that aren't available name the part fonts that the
characters are drawn in. `text.glyphAlternates` lists the alternates of a character from its part's font.

`type.compositeFonts.export {path?, names?, fonts?}` writes the document's composite fonts (all, or those `names`
lists), or the `fonts` given, to a `.vccomposite` file; without `path` it returns the file's text as `data`.
`type.compositeFonts.import {path | data | dataBase64, apply?}` reads a file of at most 4 MB. With `apply` (the
default) its fonts join the document's, replacing a font of the same name, as one undo step; with `apply: false` it
only returns them. Opening a `.vccomposite` file imports it.

Type › Composite Fonts… (shown with Show East Asian Options) opens the Composite Fonts dialog on the document's fonts:
Font picks one, and a table sets each class's font, style, size, baseline and scales. New adds a font based on the one
picked, Delete Font takes it out, Import… adds the fonts of a file and Export… writes the one picked. OK runs
`type.compositeFonts.set` with every font as one undo step; a refused font keeps the dialog open.

```json
{"name":"run_command","arguments":{"command":"type.compositeFonts.set","params":{"fonts":[{"name":"My Composite","parts":{"kanji":{"font":"Source Han Sans"},"roman":{"font":"Source Sans 3","size":95,"baseline":-2},"numerals":{"font":"Source Sans 3"}}}]}}}
```

## Optical Margin Alignment

`type.opticalMarginAlignment {on?: bool}` (Type › Optical Margin Alignment) turns Optical Margin Alignment on or off
for the selected type objects (or `ids`); without `on` it sets the opposite of the first object's setting. It replies
`{on, ids}`, and the menu item is checked while the first type object it acts on (inside a selected group too) has it
on. Optical Margin Alignment is a setting of the whole thread: the command sets every frame of a selected frame's
thread, and `ids` lists them all. Threading frames gives the thread its first frame's setting, frames taken out of
a thread keep it, and a file whose frames of one thread differ opens with the first frame's setting in all of them.
In every paragraph of point and area type, these characters hang outside the margin when they
start or end a line: quotation marks, hyphens (a hyphenated line's hyphen too), full stops and commas hang wholly;
asterisks, tildes, ellipses, en and em dashes, colons and semicolons hang by half their width; and A, V, W, Y, v, w
and y hang by a tenth of theirs. A punctuation mark followed by a quotation mark hangs with it.
Left-aligned lines hang at their left margin, right-aligned lines hang at their right margin, and centered and
justified lines hang at both. Auto aligns a right-to-left line at its right. A right-to-left line is read from right
to left: where it hangs at its left margin, `."` ending it hangs both glyphs. In vertical type the margins are the
top and bottom of a column. The space that justification adds between glyphs leaves out the gaps next to a
hanging punctuation mark. Lines break where they did
without it, and a line of one glyph does not hang. East Asian commas and full stops hang only through burasagari, and
a line that ends in one hangs only at its start. Type on a path hangs at its brackets, measured along the path:
left-aligned, Justify All and Justify with Last Line Aligned Left type hangs at the start bracket, right-aligned and
Justify with Last Line Aligned Right type hangs at the end bracket, and centered and Justify with Last Line Aligned
Center type hangs at both. Auto type hangs at the bracket it is placed at: the end bracket for horizontal type whose
first paragraph runs right to left, and the start bracket otherwise. Glyphs that hang past the end bracket do not make
the type overflow. On an open path a hang stops at the path's end, and on a closed path the two hangs together take
at most the stretch between the end and start brackets. SVG export writes type on a path whose glyphs hang as
outlines, with a warning.
New type and documents from before it have it off, and it is saved only when on. One undo step.

```json
{"name":"run_command","arguments":{"command":"type.opticalMarginAlignment","params":{"on":true}}}
```

## Roman Hanging Punctuation

`text.setFormat {romanHangingPunctuation: bool}` (Paragraph panel menu › Roman Hanging Punctuation) turns it on or off
for the paragraphs of the selected type (or `ids`, or the paragraphs a `start` / `end` range touches). In those
paragraphs the punctuation Optical Margin Alignment hangs hangs outside the margin by the same amounts (quotation marks,
hyphens, full stops and commas wholly; asterisks, tildes, ellipses, en and em dashes, colons and semicolons by half),
on the same sides (left-aligned lines at their left, right-aligned at their right, centered and justified at both), and
letters don't hang. With Optical Margin Alignment on as well, the letters hang too. East Asian punctuation hangs only
through burasagari. When the first paragraph of type on a path has Roman Hanging Punctuation on, the type hangs its
punctuation at its brackets as Optical Margin Alignment does, and no letters. Off by default, saved with the paragraph
(`roman_hanging` in `para`) only when on, carried by paragraph styles, and turned off by Reset Panel. One undo step.
The menu item is checked while the paragraph the Paragraph panel shows has it on.

```json
{"name":"run_command","arguments":{"command":"text.setFormat","params":{"romanHangingPunctuation":true}}}
```

## Bulleted and numbered lists

`text.setFormat {list: "none"|"bullet"|"number", bullet: "none"|"dot"|"dash"|"circle"|"square", number:
"none"|"arabic"|"upperLetters"|"lowerLetters"|"upperRoman"|"lowerRoman"|"twoDigits"|"threeDigits"}` (Paragraph panel ›
Bullets and Numbered List) makes the paragraphs of the selected type (or `ids`, or the paragraphs a `start` / `end`
range touches) a bulleted or numbered list or takes them out of one. `bullet` sets the bullet: a filled dot (•), a dash
(–), a hollow circle (◦) or a filled square (▪). `list: "bullet"` keeps the bullet of a paragraph that is already in a
list and gives the others the dot. `number` sets the number style: 1, 2, 3; A, B, C; a, b, c; I, II, III; i, ii, iii;
01, 02, 03; or 001, 002, 003. `list: "number"` keeps the style of a paragraph that is already in a numbered list and
gives the others 1, 2, 3. Without `list`, a `number` style makes a numbered list and a `bullet` makes a bulleted one; a
numbered list keeps its bullet for when it is made bulleted again. `"none"` in any of the three keys turns the list off
and leaves the indents as they are. An unknown value is an error and changes nothing. A paragraph that joins a list
while its left and first-line indents are both 0 gets a left indent of 18 pt and a first-line indent of -18 pt, unless
the same call sets `leftIndent` or `firstLineIndent`; other indents stay as they are, also when a list changes between
bullets and numbers. In a horizontal right-to-left list paragraph the indents are mirrored: the left and first-line
indents are measured from the right edge, and the right indent from the left edge. Its marker starts the first line at
the right, and the lines it wraps to start at the right where the first line's text starts after the marker. A
right-to-left paragraph without a list keeps its left and first-line indents at the left, and vertical type keeps its
indents as they are. In threaded type, a list paragraph that continues into the next frame starts that frame where its
wrapped lines start, without the bullet or number, and the paragraph after it takes the next number.

The layout draws the marker (the bullet, or the number and its period) before the first line of each list paragraph
that has text, in the font, size, scales and color of the paragraph's first character, without that character's
baseline shift, rotation, superscript or subscript, tracking, manual kerning and tate-chu-yoko. The marker is not a
character of the text: copied text leaves it out, the caret before the paragraph's first character stands at the text,
a selection does not highlight the marker, and Touch Type moves that character without the marker. The text after it
starts at the next tab stop, the left indent counting as a stop when no explicit stop comes before it, measured from the
right in a right-to-left list paragraph; a number that reaches past the left indent takes the text to the next tab stop
after it. The lines the paragraph wraps to start at the left indent. Type on a path draws the marker and the same gap
before the paragraph's text; a right-to-left item on a path has its marker at the right of its text. Justification and
Optical Margin Alignment leave the marker and the gap after it as they are, and an underline or strikethrough starts at
the text. Return at the end of a list item starts another item in the same list. The list is saved with the paragraph
(`list` in `para`, for example `{"bullet": "square"}` or `{"bullet": "dot", "number": "upperRoman"}`) only when it is
set, and paragraph styles carry it: a style captured from a paragraph without a list turns the list off. Reset Panel
turns it off. The change is one undo step.

`text.setFormat {listAlign: "left"|"right"}` (Bullets and Numbering › Alignment) sets where the marker stands before
the first line's text. With `"left"`, the default, the marker starts where the first line starts, at the left indent
plus the first-line indent. With `"right"` the marker ends where the text starts, against the text. The text starts at
the same place with either alignment, so numbers of different widths aligned right end at the same place, and a number
that reaches past the left indent ends at the next tab stop, where the text starts. In a right-to-left list paragraph a
marker aligned right starts where the text ends. `listAlign` sets the paragraphs that are in a list, and those that the
same call puts in one; a paragraph in no list stays out of one. A list keeps its alignment when it changes between
bullets and numbers, and a list that is turned off and on again starts aligned left. An unknown value is an error and
changes nothing. The list saves the alignment (`"align": "right"`) only when it is right.

`text.setFormat {listLevel: 1..9}` (Bullets and Numbering › Level) sets the level of the list paragraphs it touches, and
of those that the same call puts in a list; a paragraph in no list stays out of one. The value is a whole number from 1
to 9 (`2` or `2.0`); any other value is an error and changes nothing. Each level past the first
sets the paragraph 18 pt further in than its left indent: its marker, its text and the lines it wraps to move together,
and in a right-to-left list paragraph they move in from the right. A list keeps its level when it changes between
bullets and numbers, and a list that is turned off and on again starts at level 1. The list saves the level
(`"level": 3`) only when it is not 1. In the Type tool, Tab with the caret at the start of a list paragraph (before its
first character, with nothing selected) runs `text.setFormat {listLevel}` with the next level, and Shift+Tab with the
level before; each is one undo step, and the text is unchanged. Tab at level 9 and Shift+Tab at level 1 do nothing.
Anywhere else, with characters selected, or in a paragraph in no list, Tab and Shift+Tab insert a tab.

A numbered paragraph's number counts the numbered paragraphs of its story up to and including it: the count runs through
consecutive list paragraphs, through the frames of threaded type in thread order, and starts again at 1 after a
paragraph in no list. An empty paragraph has no number and is not counted, and a bulleted paragraph keeps its bullet and
is not counted. Each level counts its own numbers: a list paragraph that has text sets the counts of the levels
deeper than its own back to 0, so a deeper numbered paragraph after it counts from 1, and a paragraph back at a
shallower level goes on with that level's count. Letters run from A to Z, then AA to ZZ, then AAA; roman numerals use IV, IX, XL, XC, CD and CM; 01 and
001 keep every digit past 99 and 999; letters and roman numerals past 3999 are written in Arabic digits. In a
right-to-left paragraph the number reads left to right with its period at its left. Each threaded frame saves the count
that its earlier frames carry into it at level 1 (`listCount`, only when it is not 0) and at the levels past it
(`listLevelCounts`, level 2 first, only when one of them is not 0). Type that is in no thread counts from 1:
after Remove Threading or Release Selection, in a frame converted to point type, and in every copy of a frame (pasted,
duplicated, copied in the Layers panel, or taken out of a symbol or a placed file). A symbol made of a thread keeps the
numbers in the symbol's art.

The canvas, EPS, PDF and Type › Create Outlines draw the marker. PDF writes the marker as text of its own, where the
canvas draws it, and a PDF viewer selects, copies and finds its characters. Each of its glyphs stands for the marker
character that the glyph's font maps to that glyph in its character map. Glyphs that an OpenType feature such as
oldstyle figures replaced stand for the characters left over, in order, when there are as many of those glyphs as
characters left over. A marker glyph that stands for no character, such as the missing-glyph box when no font has a
glyph for it, is drawn as a path. Live SVG text writes the marker's characters before the paragraph's text, except
type on a path with a list, which it writes as outlines with a warning. DXF writes the marker as a TEXT entity of its
own, its characters in the order they are drawn. Legacy `.ai` writes the paragraph's text without the marker and warns
that bulleted and numbered lists are left out.

Type › Bullets and Numbering › Apply Bullets and the canvas context menu's Bullets and Numbering › Apply Bullets run
`type.applyBullets {bullet?}` on the selected type's paragraphs, or on those the Type tool's selection touches. Without
`bullet` it runs `text.setFormat {list: "bullet"}`, and with it it runs `text.setFormat {bullet}`. Apply Numbers in the
same menus runs `type.applyNumbers {number?}`. Without `number` it runs `text.setFormat {list: "number"}`, and with it
it runs `text.setFormat {number}`.

More Options at the end of the Paragraph panel's Bullets and Numbered List presets runs `ui.bulletsAndNumbering {tab?: "bullets"|"numbers"}`. It opens the Bullets and Numbering
dialog (`bulletsAndNumbering`) on the paragraphs the Paragraph panel acts on, on that tab; without `tab` it opens on
Numbers when the paragraphs are a numbered list and on Bullets otherwise. On both tabs the Level field, above the
presets, holds the paragraphs' list level (1 to 9; 1 for paragraphs in no list). The Bullets tab has the five bullet
presets and the Numbers tab has None and the seven number styles. The Position group has Alignment (Left or Right),
Left Indent and First Line Indent. The dialog opens with the values the Paragraph panel shows; where the paragraphs
differ, no preset is picked and the field is blank. Its fields are `tab`, `list` (`none`, `bullet` or `number`: the
kind of the preset picked), `bullet`, `number`, `listAlign`, `listLevel`, `leftIndent` and `firstLineIndent` (points),
and `ui.dialog.set` sets them, including those the dialog opened blank or without. The type shows the dialog's values while it is open. A preset picked for paragraphs that are in no list and have
no indents fills in 18 pt and -18 pt, and None takes them out again while they are unchanged. OK runs `text.setFormat`
with the values that changed, as one undo step, and Cancel puts the type back as it was. When a preset puts the
paragraphs in a list, OK also sends the indents the dialog shows, and the paragraphs take them, 0 included. OK does not
send an Alignment or Level change for paragraphs that stay in no list. With no type selected, or
with an unknown `tab`, `ui.bulletsAndNumbering` is an error.

Type › Bullets and Numbering › Convert to Text runs `type.convertListToText` on the selected type's paragraphs, or on
those the Type tool's selection touches, and that runs `text.convertListToText {ids?, start?, end?}`. The marker of
each list paragraph becomes characters at the start of the paragraph: the bullet, or the number and its period as the
paragraph showed it, then a tab. They are set in the marker's font, size and color: the style of the paragraph's first
character without the attributes the marker leaves out. The paragraph leaves its list. A paragraph at a list level
past the first keeps the level's indent in its left indent, so its marker and text stay where they were. A marker aligned right
(`listAlign` `right`) is written the same way: its characters go to the first line's start, where a marker aligned left
is drawn, and the text after its tab stays where it was. When the marker's tab went to the left indent, the paragraph gets a left-aligned tab stop without a leader there, in place of any stop there, so that
the text after the tab starts where it started after the marker. When the marker's tab went to a tab stop, the
converted tab goes to the same place when that stop is left-aligned and no stop there or past it has a leader. When a
stop there or past it has a leader or is not left-aligned, or a right, center or decimal stop lies between the marker
and the place its tab ended, the paragraph gets a left-aligned stop without a leader where the marker's tab ended, in
place of any stop there and of those right, center and decimal stops before it. A paragraph that holds a tab of its
own keeps its stops past the left indent as they are. The characters are text like any other: they can be selected,
styled, edited and deleted, and copied text includes them. In threaded type the numbers are the ones the story counted
through its frames, and a paragraph that continues into the next frame gets its characters once, where its text
starts. An empty list paragraph has no marker and only leaves the list. A numbered paragraph after the converted ones
that stays in its list counts from 1 again, as after any paragraph in no list. A paragraph with no direction set runs
the way its first strong character does; when a letter or a roman numeral put before right-to-left text would make it
run left to right, the paragraph gets the Right to Left direction. A right-to-left paragraph that leaves its list no
longer has its indents mirrored, and on a path the tab takes its own width, as every tab on a path does. With the
Every-line Composer, a paragraph that holds a tab is broken line by line, so a converted paragraph of several lines
can break at other places than its list paragraph did. The change is one undo step, and the reply gives the `ids` of
the type it changed, how many paragraphs were `converted`, and the characters `inserted`, each with the type object's
`id`, the byte `at` which they went in (counted in the text before the change) and their `length` in bytes. While the
Type tool edits, `text.convertListToText` ends its typing first and moves its anchor and caret past the characters
inserted at or before them, whether the call comes from the menu, MCP or the control channel.
After Select All in a frame of threaded text, a call without `ids`, `id`, `start` or `end` converts every paragraph of
the story's frames that are neither locked nor hidden. While the story is selected, its locked and hidden frames are
not converted, even when `ids` names them, and neither is a paragraph whose text starts in one. When the call converts
all of its other frames, the whole story stays selected.
In threaded type, when the caret's character moved to another frame, the Type tool edits that frame with the caret
next to the same character, and that frame becomes the selection when it was not selected.

Typing starts a list too. When the Type tool types a space on its own (one key press, or `type_text {text: " "}`) right
after `1.` or `1)`, `A.` or `A)`, or one of `*`, `#`, `>` and `+` at the start of a paragraph that is in no list, the
typed characters and the space are removed and the paragraph becomes a numbered list in 1, 2, 3 or A, B, C, or a
bulleted list: `*` gives the filled dot (•), `+` the hollow circle (◦), `#` the filled square (▪) and `>` the dash
(–). A list started with `1)` or `A)` also writes its numbers with a period. A paragraph
whose left and first-line indents are both 0 gets 18 pt and -18 pt. The tool runs `text.startList {id, at}`, with `at`
the byte after the space, as an undo step of its own after the typing: one Undo puts the characters back as text with no
list, and typing goes on after them; Redo removes them again, and typing goes on before the paragraph's text. The same
characters later in a paragraph start nothing, and so do list characters that arrive in one piece with the space: a
paste, an IME commit, or `type_text` of a longer string. With the `typedLists` preference off (Preferences › Type ›
Start Lists From Typed Markers), the Type tool starts no list from typed characters. From a script, `text.startList {id, at?}` (`at` defaults to the end of the text) does
the same with the text before `at` and returns `started` and `caret`. When the list starts, `caret` is the byte where
the paragraph's text now starts; when the text before `at` starts no list, nothing changes, `started` is false and
`caret` is `at`.
```json
{"name":"run_command","arguments":{"command":"text.setFormat","params":{"bullet":"square"}}}
{"name":"run_command","arguments":{"command":"text.setFormat","params":{"number":"upperRoman"}}}
{"name":"run_command","arguments":{"command":"text.setFormat","params":{"listAlign":"right"}}}
{"name":"run_command","arguments":{"command":"text.setFormat","params":{"listLevel":2,"start":4,"end":4}}}
{"name":"invoke_menu","arguments":{"command":"ui.bulletsAndNumbering","params":{"tab":"numbers"}}}
{"name":"run_command","arguments":{"command":"text.convertListToText","params":{}}}
```

## Kerning

`text.setRangeStyle {id, start, end, kerning}` (a range) and `text.setFormat {ids?, kerning}`
(the selected type) accept `kerning: "auto" | "optical" | number` (1/1000 em). Auto uses the font's
pair metrics; Optical spaces horizontal pairs from their glyph shapes, moving each toward the font's own
stem spacing; ideographs, kana, fullwidth forms and dashes keep the font's spacing. Vertical text uses font
metrics in either mode. A number sets a manual pair value,
replacing that pair's automatic correction while tracking remains additive. Choosing `"optical"` or
`"auto"` clears existing manual pair values in the text it applies to. New text and files without an Optical setting use Auto. The Character
panel offers both automatic modes.

## Small Caps, Superscript and Subscript
`text.setFormat {smallCaps, position}` (the selected type, or `ids`) and `text.setRangeStyle {id, start, end, smallCaps,
position}` (a range) set what the Character panel's Small Caps, Superscript and Subscript buttons and menu items set.
`smallCaps: true` draws lowercase letters as capitals at Document Setup's small caps size (70% of the font size by
default); capitals keep their size, and with All Caps on every letter is a full-size capital. `position: "superscript"`
or `"subscript"` draws the characters at Document Setup's superscript or subscript size, raised or lowered by its
position (by default 58.3% of the font size, moved by 33.3% of it), and `"normal"` puts them back on the baseline.
A file whose characters, character styles or graph label styles hold a superscript or subscript size outside 1 to
500%, a position outside -500 to 500% or a small caps size outside 1 to 200% opens with the nearer end of each range.
Setting superscript on subscript characters replaces the subscript, and setting subscript on superscript characters
replaces the superscript. When `document.setup {superscript, subscript, smallCapsSize}` changes the proportions, the
characters take the new ones. One undo step.
{"name":"run_command","arguments":{"command":"text.setFormat","params":{"position":"superscript","smallCaps":false}}}
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

## Proportional Metrics

`text.setFormat {proportionalMetrics: true}` (the selected type, or `ids`) and `text.setRangeStyle {id, start, end,
proportionalMetrics}` (a range) set full-width glyphs on the proportional widths the font gives them with the
OpenType `palt` feature; glyphs the font doesn't re-space keep their full width. In vertical type, upright glyphs take
the proportional heights of the font's `vpal` feature (the glyphs are shaped top to bottom for it) and move with them;
a font without vertical metrics keeps its cells. Line-end Punctuation Half Width then takes nothing more off the
punctuation `palt` or `vpal` re-spaced (the opening bracket at a line's start, consecutive punctuation, the closing
mark at a line's end); punctuation left full width is trimmed as before. With automatic kerning the font's kerning
for the re-spaced glyphs comes too: `apkn` with `palt`, and `vkrn` and `vapk` with `vpal` (vertical type is otherwise
not kerned). Off by default and saved only when on. One undo step.

```json
{"name":"run_command","arguments":{"command":"text.setFormat","params":{"proportionalMetrics":true}}}
```

## Variable fonts

`text.setFormat {fontAxes: {"wght": 650}}` (the selected type, or `ids`) and `text.setRangeStyle {id, start, end,
fontAxes}` (a range) set variation axes of a variable font, as the Character panel's Variable Font sliders do: each tag
named takes the value given, in the axis's own units and clamped to its range, in place of the style's setting of that
axis, and `null` sets it back to the style's setting. The other axes keep their settings. A tag the font has no axis for is
left out, and a value equal to the style's setting is not kept; for a font that isn't installed the values are kept as
given. Layout, outlines, the substituted-glyph highlight and PDF export use the font instanced at the settings; SVG export
writes them as `font-variation-settings` (and the `wght` setting as the weight, the `wdth` setting as `font-stretch`) and
embeds the font instanced there, with a warning when embedded faces of one weight, style and width share characters; SVG
import reads `font-variation-settings` back. Another font or style (`text.setStyle {font, style}`, `text.setRangeStyle
{font, style}`, `text.replaceFont`, and a character style applied or redefined with another font unless it sets them)
clears the settings, and Missing Glyph Protection keeps them on the characters that keep their font. They are saved as
`fontAxes` only when set, and a character style captured from type (`charStyle.new`, `charStyle.redefine`) sets them
explicitly, `{}` for none. Each command is one undo step.

```json
{"name":"run_command","arguments":{"command":"text.fontList","params":{"family":"Bahnschrift","style":"Regular"}}}
{"name":"run_command","arguments":{"command":"text.setFormat","params":{"fontAxes":{"wght":650,"wdth":87.5}}}}
```

## Character Direction

`text.setFormat {charDirection: "default"|"leftToRight"|"rightToLeft"}` (the selected type, or `ids`) and
`text.setRangeStyle {id, start, end, charDirection}` (a range) set the direction of the characters inside their
paragraph, as the Character panel menu's Character Direction does with the Middle Eastern (Indic) options shown. The bidi
algorithm (UAX #9) resolves each stretch of characters with a direction as an embedding in that direction, as if an LRE
or RLE stood before it and a PDF after it: `leftToRight` keeps a date written with spaces, such as `21 10 2026`, in its
order inside a right-to-left paragraph, and `rightToLeft` sets a Hebrew or Arabic word and the punctuation after it right
to left inside a left-to-right paragraph. Characters keep their own strong direction inside an embedding: Latin letters
set Right to Left still read left to right. `default` leaves the characters to the bidi algorithm. The paragraph's own
direction is unchanged. SVG export writes type with a character drawn right to left, or with a run set Right to Left,
as outlines; live SVG text writes `direction="ltr"` and `unicode-bidi="embed"` on each run set Left to Right. Character
Direction is Default until set and is saved only when set, and character styles captured from type carry it. Each
command is one undo step.

```json
{"name":"run_command","arguments":{"command":"text.setRangeStyle","params":{"id":12,"start":9,"end":19,"charDirection":"leftToRight"}}}
```

## Digits

`text.setFormat {digits: "default"|"arabic"|"hindi"|"farsi"|"devanagari"|"bengali"|"gurmukhi"|"gujarati"|"odia"|"tamil"|"telugu"|"kannada"|"malayalam"}`
(the selected type, or `ids`) and `text.setRangeStyle {id, start, end, digits}` (a range) set the numeral system the
characters' digits are drawn in, as the Character panel menu's Digits does with the Middle Eastern (Indic) options shown:
`arabic` draws every digit as the Western 0–9, `hindi` as the Arabic-Indic ٠–٩ (U+0660–U+0669) and `farsi` as the Eastern
Arabic-Indic ۰–۹ (U+06F0–U+06F9), whichever of the three sets was typed; the Indic ones draw them as their script's own
digits (#4942): `devanagari` ०–९ (U+0966–U+096F), `bengali` ০–৯ (U+09E6), `gurmukhi` ੦–੯ (U+0A66), `gujarati` ૦–૯
(U+0AE6), `odia` ୦–୯ (U+0B66), `tamil` ௦–௯ (U+0BE6), `telugu` ౦–౯ (U+0C66), `kannada` ೦–೯ (U+0CE6) and `malayalam` ൦–൯
(U+0D66); `default` leaves each digit as typed. Any other value is refused. The text keeps the digits that were typed (`text.getRange`, search,
the caret and selection are unchanged); only the glyphs change, as with All Caps, and SVG export writes the digits as drawn.
A digit the font lacks comes from a fallback font. Default and saved only when set. One undo step.

```json
{"name":"run_command","arguments":{"command":"text.setFormat","params":{"digits":"hindi"}}}
```

## Font height reference

`text.setFormat {fontHeightReference: "emBox"|"capHeight"|"xHeight"|"icfBox"}` (the selected type, or `ids`) and
`text.setRangeStyle {id, start, end, fontHeightReference}` (a range) set the height of the font that the characters' font
size measures, as the Character panel's Set Font Height Reference dropdown does (panel menu › Show Font Height Options).
With `emBox`, the default, the font size is the size of the em square. With `capHeight`, `xHeight` or `icfBox`, the
characters are set at the em size at which the font's cap height or x height (OS/2 `sCapHeight`, `sxHeight`), or the
height of its ideographic character face (the BASE table's `icfb` and `icft`, else the ink of some ideographs and kana;
across a vertical line, its width), equals the font size: 20 pt Cap Height type in a font whose capitals are 0.7 em
tall is set at about 28.6 pt, and its font size stays 20 pt. With a font that has no such value, or one outside 0.1 to 2
em, the size measures the em box. Auto leading, tracking and kerning follow the em size the characters are set at (the
Character panel's Leading field shows that Auto leading, and `type.step {attribute: "leading"}` starts from it);
explicit leading and baseline shift stay in points, and an inline graphic keeps its size.
SVG, PDF and legacy `.ai` export and the CSS Properties rules write the em size. DXF export writes 0.7 times the em
size as each TEXT entity's height (group 40, the height of its capitals).
Saved only when not `emBox`. One undo step.

```json
{"name":"run_command","arguments":{"command":"text.setFormat","params":{"fontHeightReference":"capHeight"}}}
```

## Language

`text.setFormat {language}` (the selected type, or `ids`) and `text.setRangeStyle {id, start, end, language}` (a range)
set the language of the characters, as the Character panel's Language menu does: one of Document Setup's languages
(`document.setup {language}`), by name or BCP 47 tag, case-insensitive (`"Turkish"` or `"tr"`, `"Marathi"` or `"mr"`).
An unknown language is an error. The characters are shaped in it, so the font uses that language's forms (OpenType
`locl`): in Turkish, Source Serif 4 keeps `fi` as two letters and the dot on the i, where English makes the fi ligature.
Runs are saved with the tag, and English: USA, the default, as no language, so files without languages load and save
as before. New type takes the document's language. Character styles captured from type carry it, and `charStyle.new` /
`charStyle.setAttrs` take `attrs: {language}` the same way. One undo step.

```json
{"name":"run_command","arguments":{"command":"text.setFormat","params":{"language":"Turkish"}}}
```

## Touch Type

The Touch Type tool (`touchType`, Shift+T) changes one character at a time. A click on a character of a type object
(`pointer_gesture` with `touchType`) selects the object and that character and shows a box around the character.
Dragging the box or its lower-left handle moves the character along its line and up or down. The upper-left handle
scales it vertically, the lower-right handle scales it horizontally, the upper-right handle scales it proportionally,
and the handle in the middle of the top edge rotates it about its pen position on the baseline. Escape lets go of the character. Each
drag is one `text.touchType {id, start, dx?, dy?, hScale?, vScale?, rotation?}` and one undo step. `start` is the byte
offset of the character; a ligature (one glyph for several characters) changes as a whole, and the reply's `start` and
`end` give its bytes. `dx` (document points, in reading order) changes the kerning of the character before it on
its line and its own kerning, so the character moves by `dx` and the characters after it stay where they are. The first
character of a line and a right-to-left character take no `dx`, and `dx` is limited to what kerning from -1000 to 10000 (1/1000 em) on the two
characters can give. `dy` raises it by baseline shift, which moves the character up from its line at any rotation, a quarter
turn included; a character rotated with a baseline shift turns about its shifted pen position. The tool splits a drag
into its part along the line (kerning) and its part up from the line (baseline shift), so the character follows the
pointer at any rotation; the first character of a line and a right-to-left character follow it only up and down. `hScale`, `vScale`
and `rotation` set its scales and rotation as `text.setRangeStyle` does. Where the font kerned the character with its
neighbors, the kerning of the two characters keeps it and the characters after it where they were along the line; a
wider or narrower character moves the characters after it by the change in its width. Touch Type gives the character
before it a kerning value of its own (with `dx`, and with any other change where the font kerned the character with
its neighbors), and that character can then move by the font's kerning pair with the one before
it, as with any style change to one character (`text.setRangeStyle`). The type stays editable.

```json
{"name":"run_command","arguments":{"command":"text.touchType","params":{"id":7,"start":1,"dx":10,"dy":4}}}
```

## Alternate glyphs

The Glyphs panel's Show › Alternates for Current Selection lists the alternate glyphs of the character selected with
the Type tool (a range's first character, else the one before the caret), with the default glyph first when the font
does not list it, and a double-click on one draws that character with it. `text.glyphAlternates {id, start}` lists them for the character at byte `start`: `{char, font, style, default,
chosen, alternates: [{gid, feature}]}`, where `default` is the glyph the font's character map gives, `chosen` the glyph
drawn now, and `alternates` the glyphs the font's `GSUB` single and alternate substitutions put in place of the default
glyph: `aalt` first, then `salt`, `ss01` to `ss20`, `swsh`, `cv01` to `cv99`, `hist`, `jalt`, `nalt`, `ornm` and
`titl`, in lookup order within each, each glyph once. The default glyph is listed only when `aalt` lists it. Ligatures
and contextual substitutions are not listed. `text.setGlyphAlternate {id, start, gid}` draws the character with glyph
`gid`, one of those alternates or the default glyph, as one undo step; `gid: null` takes the choice away, and the
character is drawn as its style draws it (a feature such as `salt` can draw another glyph than the default). The text
keeps the character and its other attributes, and characters typed next to it take their default glyphs. The choice is
saved with the character's style as `glyphAlternate: {ch, gid, font, version}`, the face's PostScript name and version
string: set in another font or version, or with the font missing, the character takes its default glyph. Both commands
refuse a character whose font is not available, and `text.setGlyphAlternate` refuses a glyph that is not one of the
character's alternates and a glyph that the character's style does not draw (All Caps, Small Caps or another numeral
system). Choosing the glyph a character already draws is not an edit. Character styles do not take `glyphAlternate`:
`charStyle.new`, `charStyle.edit` and `charStyle.setAttrs` refuse it.

```json
{"name":"run_command","arguments":{"command":"text.glyphAlternates","params":{"id":7,"start":2}}}
{"name":"run_command","arguments":{"command":"text.setGlyphAlternate","params":{"id":7,"start":2,"gid":1043}}}
```

## New type in a Japanese interface

While the interface is in Japanese (Preferences › User Interface › Language, or `auto` on a Japanese system), new type starts with em
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
default) along the path, each character by its own font, size and vertical scale: `ascender` runs the top of the
font's em box along it (its ideographic em box, else an em box centered on the cap height), `descender` runs the
bottom of the font's bounding box along it, and `center` runs a line at half the height of the top of the font's H
above the baseline along it (in vertical type, `ascender` and `descender` move the characters half an em with their
horizontal scale, and `center` leaves them on the path); `spacing` (points) closes glyphs up round the outside of
curves and opens them up round the inside. With none of them it queries the first selected type on a path:
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
that range of characters. When every paragraph that `text.tabs.set` or `text.tabs.clear` targets already has those
stops, the command changes nothing and records no undo step. `text.tabs.set` rejects a supplied `align`, `leader`, or
`alignOn` value that is not a string, and an empty `alignOn` value.

`text.tabs.get {start?}` and `paraStyle.new` / `paraStyle.redefine {id, start?}` read the
paragraph at `start`; `paraStyle.list` counts paragraphs as uses, and `charStyle.list` counts runs; both count the type in
the layers, opacity masks, symbols, graph designs, pattern tiles and the art of brushes. `charStyle.unused` and
`paraStyle.unused` (Select All Unused in the Character Styles and Paragraph Styles panel menus) list the styles with no
uses, in panel order and never the Normal style, and the panel highlights them; `charStyle.delete` and
`paraStyle.delete` take `{names}` to delete several styles in one undo step. `charStyle.load` and `paraStyle.load`
(Load Character Styles… and Load Paragraph Styles… in the panel menus, where `ui.loadCharStyles` and
`ui.loadParaStyles` pick the file) copy the styles of that kind from another document VectorCraft opens (`path`, `data`
or `dataBase64`), and `textStyle.loadAll` (Load All Styles…, `ui.loadAllStyles`) copies both kinds, each in one undo
step. The swatches and patterns the styles paint with, the symbols and brushes those patterns use, and the styles of
either kind that type in those patterns and symbols is set in, come along as Paste brings those of pasted type; a global or spot color whose name the document gives another color
is the document's. A loaded style that is the same as the style of
its name, or of its name with a number (`Body 2`), is not added again; any other is added under a free name. The
document's styles and the type that uses them keep their attributes, and the file's Normal styles are not loaded. The
reply lists the names `added` and `existing` in the document and the names `skipped` in the file (a style whose
attributes do not read or are out of the range `charStyle.new` takes, or whose name is empty); `textStyle.loadAll`
replies `{charStyles, paraStyles}`, and `charStyle.load` and `paraStyle.load` list only the styles of their own kind. A Return the Type tool types (or `text.editRange`
inserting `\n`) continues the style of the paragraph it splits; deleting a break keeps the first paragraph's style;
`text.setText` gives every paragraph the first one's attributes. Threaded text keeps each paragraph's attributes as
the story re-flows (a paragraph split between frames has them in both).

`ui.newCharStyle` and `ui.newParaStyle` (Alt-click on New in the Character Styles or Paragraph Styles panel) open
New Character Style or New Paragraph Style (dialog `newTextStyle`, field `name`) with the name `charStyle.new {}` or
`paraStyle.new {}` would give the style, and add no style yet. OK runs `charStyle.new` or `paraStyle.new` with the
name and the Type tool's selection as it was when the dialog opened (else the selected type); a name another style
has, or an empty one, keeps the dialog open, and Cancel adds no style. A plain click on New still makes the style at
once.

`ui.charStyleOptions {name}` opens Character Style Options for an existing named style.
Character Style Options uses `charStyle.edit {name, to, attrs}` to replace the name and complete sparse
attribute map in one undo step. `to` is trimmed and must be unique. The command updates text using the style
while preserving local overrides; Undo/Redo restores the definition and its users. The `charStyleOptions`
dialog exposes `name`, `font_family`, `font_style`, `size`, `leading`, `kerning`, `tracking`, `case`,
`position`, `underline`, `strikethrough`, `charAlign`, `h_scale`, `v_scale`, `baseline_shift`, `rotation`,
`language`, `fill`, `fill_tint`, `stroke`, `stroke_tint`, `stroke_width`, `overprint_fill`, `overprint_stroke`,
`ot_figure`, `ot_position`, `ot_liga`, `ot_calt`, `ot_dlig`, `ot_swsh`, `ot_salt`, `ot_titl`, `ot_ordn`, `ot_frac`,
`ot_sets`, `section` (`general`, `basic`, `advanced`, `color` or `opentype`), and `preview`. An empty field leaves its
attribute unset. `leading` and `kerning` also accept `Auto` to store an explicit automatic value.
Advanced Character Formats holds `h_scale` and `v_scale` (percent, 1 to 10000),
`baseline_shift` (points, -1296 to 1296), `rotation` (degrees, stored from -180 to 180) and `language` (a name or tag from Document
Setup's languages; English: USA stores the explicit default `null`, and a tag not in the list that the style
already has is kept). Language sets the language the characters are shaped in; it does not change hyphenation
or spelling.
Character Color holds `fill` and `stroke` (a paint: `{"type": "none"}` or a solid color; the menu lists None and
the document's solid swatches, a global swatch or a tint of one as a paint linked to it), `fill_tint` and
`stroke_tint` (percent, 0 to 100, for a paint linked to a global swatch), `stroke_width` (points, 0 to 4000000)
and the `overprint_fill` and `overprint_stroke` checkboxes. A gradient or pattern paint the style already has is
kept until another paint is chosen. A paint linked to a global swatch takes the swatch's color at its tint, and
editing the swatch later updates the style and its text. `swatch.unused` counts a swatch a character style's fill or
stroke links to as used, whether or not text uses the style.
OpenType Features holds `ot_figure` (`default`, `tabularLining`, `proportionalLining`, `proportionalOldstyle` or
`tabularOldstyle`), `ot_position` (`default`, `superscript`, `subscript`, `numerator` or `denominator`), the on,
off or unset checkboxes `ot_liga`, `ot_calt`, `ot_dlig`, `ot_swsh`, `ot_salt`, `ot_titl`, `ot_ordn` and `ot_frac`,
and `ot_sets` (a list of stylistic sets `ss01` to `ss20`, each on, or off with a leading `-`, as in
`["ss02", "-ss05"]`; a set not listed is unset). They write the style's `features` list: a changed control
replaces only its own tags, so tags the dialog does not show stay, and Default Figure or Default Position stores
its features turned off, distinct from unset.
`charStyle.new`, `charStyle.setAttrs` and `charStyle.edit` reject a tint outside 0 to 1, a link to a swatch that is
not a global swatch of the document, a stroke weight outside 0 to 4000000, more than 256 feature tags and an
OpenType feature tag the shaper does not know, unless the style or the text it is made from already has that value.
They also reject a size outside 0.1 to 1296, explicit leading outside 0.1 to 5000, horizontal or vertical scale
outside 1 to 10000, and baseline shift or Tate-chu-yoko offsets outside -1296 to 1296. `leading: null` keeps
Auto leading. An invalid value leaves the style, its text, and Undo history unchanged.
The New Character Style dialog accepts the same character fields and starts with the selected text's
captured attributes. Without selected text, it starts with the character defaults for the interface language
(including `charAlign: "emBoxCenter"` in Japanese); changing a Basic field keeps other seeded attributes.
Without selected text, the New Paragraph Style dialog sends no `attrs`, and the engine gives the new style the
paragraph defaults for the interface language. Reset Panel in Basic Character
Formats removes those fields from the definition, Reset Panel in Advanced Character Formats removes its five, Reset
Panel in Character Color removes its five attributes and Reset Panel in OpenType Features removes `features`.
`ui.charStyleOptions.resetPanel {}` performs that reset for the section selected in either character-style
dialog; it is disabled on General.

In the native file a text object's `para` holds the first paragraph's attributes and `paras` (left out when every
paragraph is alike) one entry per paragraph, so older readers keep the first paragraph's. SVG export anchors each
centred or right-aligned line on its own (`<tspan text-anchor>`), and SVG import gives each line's paragraph its
alignment back, with indents that keep it in place. When a native file's type or character styles hold a size, a
horizontal scale or a vertical scale of 0 or less, the value opens as 12 pt or 100%. A negative leading opens as Auto,
and a size, leading or scale past 4,000,000 opens as 4,000,000. A baseline shift, an indent or a space before or after
past 4,000,000 pt either way, in the type or in character or paragraph styles, opens at that limit, and a tab stop
outside 0 to 100,000 pt opens at the nearer end.

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

## Transform panel menu

The Transform panel menu's Transform Object Only, Transform Pattern Only and Transform Both are the
`transformPanelPatterns` preference (`object`, `pattern`, `both`). Once one is chosen, the fields, flips and menu
flips of the Transform panel and the Properties panel's Transform section send it as `objects` (false for `pattern`)
and `patterns` (true for `pattern` and `both`) to `object.setBounds`, `object.rotate`, `object.shear` and
`object.reflect`; agents pass those switches themselves. Until then (`default`, also after `prefs.reset`) they send
neither, so pattern tiles follow `transformPatternTiles`, and the menu checks Transform Both while it is on and
Transform Object Only while it is off. Use Registration Point for Symbol is the
`useRegistrationPointForSymbol` preference: with it on and one symbol instance selected, those panels show and set X
and Y at the symbol's registration point (`symbol.options` `registration`) where the instance's transform puts it
(after a flip the art's top-left corner is the box's top-right one), give `reference` as the box's point nearest it
and turn, shear and flip about it (`origin`).

```json
{"name":"run_command","arguments":{"command":"prefs.set","params":{"key":"transformPanelPatterns","value":"pattern"}}}
{"name":"run_command","arguments":{"command":"object.setBounds","params":{"x":170,"reference":4,"objects":false,"patterns":true}}}
```

## Plug-ins

WebAssembly plug-ins ([plugins.md](plugins.md)) are reached through commands; the desktop app and an unconfined
`vectorcraft-cli` keep the ones `plugin.install` installs in a `Plug-ins` folder next to the preferences and install
them again at startup (`plugin.remove` deletes them there):
`plugin.install {path | dataBase64}` installs one, `plugin.list` / `plugin.info {id}` describe them (parameters,
schema, defaults). An object filter runs on the selected paths and compound paths with `plugin.run {id, params}` (one
undo step; it may change, delete or add objects, and the result is selected). A live-effect plug-in is an effect with
id `plugin.<id>`: `effect.apply`, `effect.setParams` and `effect.expandAppearance` work as for built-in effects, and
`effect.list` lists it. An exporter adds a write-only file format `plugin.<id>` (listed by `document.formats`):
`document.export {format: "plugin.<id>", params?, path?}` writes it, as does a `path` ending in one of its extensions
that no built-in format has. A failing plug-in returns an error and leaves the document as it was (an export writes
no file). A filter whose manifest asks for a heavy budget (minutes rather than a minute, `budget: "heavy"` in its
`plugin.inspect {path | dataBase64}` entry) needs a person's approval, and `plugin.install` refuses `allowHeavy: true`
from a request. `plugin.run`
runs it where it is called, so the call may take minutes; `notifications/cancelled` stops it.
A plug-in whose manifest lists permissions (`permissions` in its `plugin.inspect` entry, with `needsApproval: true`)
needs a person's approval too, and `plugin.install` refuses `grant: [permission]` from a request. With a desktop
app connected, `ui.installPlugin` opens the approval dialog and returns `{pending: "pluginInstall"}`; `answer_dialog`,
`ui.dialog.confirm` and injected input can't confirm it. A `vectorcraft-cli mcp` started with `--allow-plugin-grants`
accepts `grant` and `allowHeavy` from any request, in that session only (headless; it can't be combined with
`--connect`). Without `--allow-plugin-grants` and without a desktop approval, `plugin.install` returns
an error naming the permissions needed. The approval is kept with the module's hash and covers only that module: the
same bytes install again without one, also from a headless session that reads the same `grants.json`, and a module
with other bytes has `needsApproval: true` and needs a new one even when it lists the same permissions. `plugin.remove`
forgets the approval. A
filter without `doc.write` or `state` that returns output needing them gets an error and the document is unchanged.
A plug-in of API level 2 (`api: 2` in its entry) that lists `progress` may report its progress, see Cancel and write
log lines: `notifications/cancelled` makes its `vc_cancelled` return 1, and its log lines go to the server's log
under its id. One that lists `settings` or `cache` keeps settings or a cache of its own in the `Plug-ins` folder
(VectorCraft's own caps: 256 settings and 1 MiB, 512 MiB of cache): `plugin.list` and `plugin.info` give the cache's
size as `cacheBytes`, `plugin.clearCache {id?}` empties the cache of one plug-in or of all of them and returns
`{cleared: [id], bytesFreed}`, replacing a plug-in keeps both, and `plugin.remove` deletes both (also for an id that
isn't installed but still has them). A module with other bytes than the plug-in installed with its id, or, with none
installed, one whose id has settings or a cache in the
`Plug-ins` folder (`needsReplace: true` in its `plugin.inspect` entry), installs only with `replace: true`: ask the
user first, since it takes that plug-in's place and gets its settings and cache. Without it, `plugin.install` returns
an error naming the plug-in. The same module installs again without `replace`, and `replace: false` refuses a module
whose id is installed. A module that imports
anything else, or imports one of these without `api` 2 and the import's permission, is refused by `plugin.install` and
`plugin.inspect` with an error naming the import and the permission.
`plugin.install` rejects a `replace` or `allowHeavy` value that is not a boolean, a `grant` that is not a list of
permission names, and a `hash` that is not the module's SHA-256. `prefs.set` returns an error and
changes nothing when a new `pluginsFolder` is missing, is not a folder or can't be listed. It does not check an empty
value or the folder the preference already holds, and the web build does not check folders.

Safe Mode starts VectorCraft without plug-ins: `vectorcraft --safe-mode`, `vectorcraft-cli --safe-mode <subcommand>`, or
the choice the desktop app offers when the last launch stopped while a plug-in was loading (Disable and Continue, Start
Normally, Start in Safe Mode). It names that plug-in without saying it caused the stop. Until the user picks Disable and
Continue or Start Normally, the next launch asks again, also after a `--safe-mode` launch, which asks nothing.
`vectorcraft --safe-mode` opens its files in a window of its own, and on Windows and Linux a file opened from the file
manager while the app is in Safe Mode starts a window with plug-ins. Files given on the command line, or opened from
the file manager while the choice is up, open after the user picks. On Windows and Linux, when the choice is Start in Safe
Mode, the files from the file manager open in a new window with plug-ins. `vectorcraft-cli --safe-mode mcp` serves a headless
session and refuses `--connect`. In Safe Mode no plug-in loads or
runs: `plugin.install`, `plugin.run`, `plugin.reload` and exporters return an error, and a live-effect plug-in draws its
object's geometry unchanged while the object keeps the effect: `effect.list` gives that effect `skipped: true`, and the
Appearance panel shows "(Safe Mode)" after its name and the plug-in's id in its tooltip. `effect.expandAppearance` returns an
error and changes nothing when an object it would expand holds such an effect, in it or in a symbol it places. `vectorcraft` rejects an option it does not know (such as
`--safe-mod`) instead of opening it as a file; `--` before a file name that starts with `-` opens it. The web build keeps
no installed plug-ins from one page load to the next and has no launch marker, so it never offers the choice. `plugin.safeMode` returns `{active, source, disabled,
pendingItem}`; a command does not turn Safe Mode on or off. `plugin.setEnabled {id | file, enabled}` turns a plug-in on or
off for the next launch, as Help › Safe Mode's Enable on Re-launch does; it does not load, unload or uninstall anything
now, and `plugin.install` of a disabled plug-in turns it on. Turning a plug-in on by `id` or by installing it also turns
on the file it is stored in, which names it when it stopped a launch while loading; when the new version can't be saved
over that file, the plug-in stays disabled. `plugin.list` reports `safeMode` and `disabled`, and
`plugin.reload` skips disabled plug-ins. A change to the disabled list is saved on top of the list the file holds at
that moment, so a change another window saved to the same preferences meanwhile is kept. When the list file can't be
read (it is too large) or holds another format or version, it is left as it is; a file that isn't JSON is replaced.
`plugin.setEnabled` returns `saved: false` when its change isn't in the plug-in folder: the session has none (a server
confined with `--automation-read-root` or `--automation-write-root`), the list there can't be read or written, or it
already holds 1,000 plug-ins, as another window may have saved it. The
change stays in the process and is saved with the next change that saves, and the app shows a status message. When
Disable and Continue can't save the plug-in it turns off, this launch skips it and the next launch asks again.

```json
{"name":"run_command","arguments":{"command":"plugin.safeMode","params":{}}}
{"name":"run_command","arguments":{"command":"plugin.setEnabled","params":{"id":"org.example.wobble","enabled":true}}}
{"name":"run_command","arguments":{"command":"plugin.install","params":{"path":"/plugins/desaturate.wasm"}}}
{"name":"run_command","arguments":{"command":"plugin.run","params":{"id":"org.vectorcraft.example.desaturate","params":{"amount":60}}}}
{"name":"run_command","arguments":{"command":"effect.apply","params":{"effect":"plugin.org.example.wobble","params":{"size":4}}}}
{"name":"run_command","arguments":{"command":"plugin.install","params":{"path":"/plugins/tiles-2.wasm","replace":true}}}
{"name":"run_command","arguments":{"command":"plugin.clearCache","params":{"id":"org.example.tiles"}}}
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
tone (mono, or CMYK screens multiplied over each other), clipped to the art's outline by default. An image in the
art that can't be decoded is sampled as blank (no dots, or full dots with `invert`), and the result carries the
`warning` that Flatten Transparency gives:

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
`/usr/local/share/fonts`, `~/.fonts` and the XDG data folders' `fonts`, `~/.local/share/fonts` among them, in a
Flatpak sandbox the host's fonts, and the folders fontconfig's configuration lists (`<dir>` in `/etc/fonts/fonts.conf`
and the files it includes: where packages add theirs, such as TeX fonts under `/usr/share/texmf/fonts`)). Faces without outlines VectorCraft draws (no `glyf`, `CFF`, `CFF2` or `VARC` table, such
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
family's styles (upright by weight, then italics) and fails when the family isn't available; `axes` lists a variable
font's variation axes (`tag`, `name`, `min`, `default`, `max`, `hidden`, and `value`, the setting of `style`, else of the
first style), empty for a static font. `text.rescanFonts`
(Character panel menu › Refresh Font List) scans the font folders again, for fonts installed or removed since the app
started: type set in a font that became available redraws in it, and type set in a family that was removed redraws in
the fallback font and is listed as missing, without editing the document. The app does so by
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
other file in `skipped`. It never replaces a file: the names `Name.otf`, `Name 2.otf` up to `Name 99.otf` are tried in
turn until one is free, which gets the copy, or is a file with the same contents, which is kept (`kept`).
Copy only fonts you own or are licensed to install. `command.batch` returns before a search ends; later requests over MCP or the control channel poll
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
polygon sides and star points, the Flare Tool Options and the Shape Builder Tool Options. The Liquify tools share one set of Global Brush Dimensions (width, height, angle,
intensity), and the Symbolism tools one brush. Interaction state (pins, a reference point) starts afresh. In the
desktop app, while the Artboard tool is in use, a change to its `selected` or `active` makes the artboard it then has
active the active artboard (the navigator's and the Artboards panel's).

```json
{"name":"run_command","arguments":{"command":"tool.setOption","params":{"tool":"twirl","values":{"width":60,"rate":90}}}}
```

`tool.options {tool}` opens what double-clicking the tool's button opens (its params doc lists every tool), in the
desktop app. The Flare tool's are its Flare Tool Options (dialog `flareOptions`): `diameter` and `pathLength` (pt),
`opacity`, `brightness`, `growth`, `fuzziness`, `longest`, `rayFuzziness` and `largest` (%), `rays`, `rings`,
`direction` (°), and `raysOn` and `ringsOn` (the Rays and Rings checkboxes; off, the flare has none). While Alt
(Option on macOS) is held, the dialog's Cancel button reads Reset. Reset puts every option back to the tool's default,
leaves a click's `x` and `y` unchanged and keeps the dialog open; `ui.dialogReset` does the same. OK keeps the options
(`tool.setOption {tool: "flare", values}`) and the next flare dragged out uses them; a click with the tool opens the
same dialog, and its OK also draws a flare there. While the center is dragged out, each drag update in which the
pointer's document y grows adds one degree per document point to `shape.flare`'s `rotation` (it turns
only the rays), decreasing y subtracts the same rate, and unchanged y leaves it. This application rate is independent of event frequency; Shift held keeps
the turn as it is. Space held keeps the turn as well. Once the center drag has passed its starting distance, and in the
drag that places the rings' end point, Space held moves the flare at its current size. The Artboard tool's are the active artboard's Artboard Options
(dialog `artboardOptions`; the Artboard tool's active artboard while it is in use, else the window's; its `x` and `y`
are the artboard's top-left corner in document points, while the dialog shows X and Y at the reference point the
Transform panel uses, counted from the ruler origin), a graph tool's
the selected graph's Graph Type (an error without one), a Symbolism tool's the Symbolism Tools Options (dialog
`symbolismOptions`: `diameter` in points, `intensity` and `density` 1–10, one brush for the eight tools) and the
Magic Wand tool's its panel. `ui.newArtboard` (Alt-click on the Artboards panel's New Artboard) opens Artboard Options
for a new artboard, with `new: true` and the name, position and size that `artboard.new {}` would give it. OK runs
`artboard.new` with the fields and makes the new artboard the active one, and Cancel adds no artboard.
`artboard.new` and `artboard.setProps` trim the white space around `name`, and a blank name gives the artboard its
default name, "Artboard N" for the Nth artboard.
A canvas-click dialog also has `preview` (initially on): tentative settings draw at the remembered click, Reset refreshes that preview, and Cancel discards it without persisting settings. OK accepts one flare as one Undo step. Toolbar-only options draw no artwork.

```json
{"name":"run_command","arguments":{"command":"tool.setOption","params":{"tool":"flare","values":{"rays":24,"ringsOn":false}}}}
```

The Shape Builder tool's options are its Shape Builder Tool Options (dialog `shapeBuilderOptions`):
`openFilledAsClosed` (on: a filled open path bounds a region as if an invisible edge closed it; off: it cuts regions
as an unfilled path does), `pickColorFrom` (`artwork`, the default: a merged shape takes the appearance, opacity and
blending mode of the front-most selected object under the drag's first point, else of the one under its last point,
else of the front-most selected object; `swatches`: the current fill, and the Left and Right arrow keys step the
current fill through the document's color swatches), `cursorSwatchPreview` (off; with `swatches`, chips above the
pointer show the current fill and the swatches before and after it), `selection` (`freeform`, the default, or
`straightLine`: a drag touches what the line from its first point to the pointer crosses), `highlightFill` (on by
default; when it is off, merge mode shades no regions), `gapDetection` (off) with `gapLength` (pt, 0 to 72; 3), `clickStrokeSplitsPath` (In Merge Mode,
Clicking Stroke Splits the Path; off), `highlightStroke` (Highlight Stroke When Editable; on) and `highlightColor`
(`#rrggbb` or a layer color's name; `#ff4f4f`, the layer color Red). The dialog's OK gives an error for a
`highlightColor` with an alpha part (`#rrggbbaa` or `#rgba`) and changes nothing. With `clickStrokeSplitsPath` on, in merge mode,
the pointer shows scissors within 4 screen pixels of a stroke, `highlightStroke` draws the run of stroke under it in
`highlightColor`, and a click splits that run off its path: the run goes from the anchor or crossing point before the
pointer to the one after it, the path keeps the rest of it, and a copy of the path with the run, in the path's
direction, goes directly above it. Both keep the path's gradients where they were on it.
A lone rectangle's side is such a run. A lone line is a run that is all of its path, and a run that would leave 0.05 pt
or less of its path (a ten-thousandth of the larger side of a path over 500 pt) counts as all of it, so the pointer
shows no scissors over it and a click on it does not split it. A release up to 3 screen pixels from the press is a
click when the pointer went no farther during the drag.
With `pickColorFrom` `swatches`, the run takes the current fill as its stroke. The tool sends `openFilledAsClosed`,
`pickColorFrom`, the gap length (`gap`, 0 with Gap Detection off) and, for such a click, `splitStroke: true` with
`shapeBuilder.merge`, which takes them all as params, and `shapeBuilder.regions` takes `openFilledAsClosed` and `gap`.
`shapeBuilder.merge` with `splitStroke` returns the run's id as `split`.

```json
{"name":"run_command","arguments":{"command":"tool.setOption","params":{"tool":"shapeBuilder","values":{"selection":"straightLine","pickColorFrom":"swatches"}}}}
{"name":"run_command","arguments":{"command":"shapeBuilder.merge","params":{"points":[[50,-1]],"splitStroke":true}}}
```

With Shift held at the press, a Shape Builder drag draws a rectangular marquee from where the drag started to the
pointer, and the Selection option does not apply to it. The regions the marquee reaches are shaded while it is drawn
and merge into one shape on release; with Alt also held at the press, they are deleted. The marquee reaches a region
when the region's outline crosses the rectangle or has an anchor inside it, or when the region holds the whole
rectangle. The tool sends the marquee to `shapeBuilder.merge` as `rect: [x, y, width, height]` in place of `points`:
`x` and `y` are where the drag started, and the width and height are negative for a drag up or to the left. For Pick
Color From Artwork, the corner where the drag started counts as the drag's first point, and the opposite corner counts
as its last: the merged shape takes the look of the front-most selected object under the first corner, else of the one
under the opposite corner, else of the front-most selected object. A Shift-click that does not move sends `points`
as a click does, so with `clickStrokeSplitsPath` on, a Shift-click on a stroke splits it. A marquee splits no stroke,
however little the pointer moves: the tool sends no `splitStroke` with `rect`, and `shapeBuilder.merge` does not split
a stroke when it has `rect`. With both `rect` and `points`, the command also touches what the points touch.
`press_key` Escape during a Shape Builder drag, a marquee included, cancels it: the release sends no
`shapeBuilder.merge`.

```json
{"name":"run_command","arguments":{"command":"shapeBuilder.merge","params":{"rect":[50,50,200,200]}}}
```

## Perspective grid

The grid is document data (`perspective.grid.set` edits the model directly). `perspective.grid.get` reads it:
`grid` (the model), `define` (the Define Grid fields; its `name` is the preset only while the grid still has that
preset's fields, else `""` for a custom grid, so dragging the horizon, a vanishing point or the station point clears it
and moving a plane does not), `station` (the viewer: `x` the centre of vision on the
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
cells while they're small) reshape the grid; Lock Grid stops them. `press_key` Escape during a drag of one of them,
or of a plane widget, cancels it and puts the grid back. The Plane Switching Widget stays put in a corner
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
A top object's outline is split into four sides at four corners. Each corner is the anchor nearest a corner of the
outline's bounding box among the anchors within 47.5% of the box's width and height from that box corner that count as
corners: an anchor with a handle 0.01 pt long or shorter, or one whose tangent unit vectors differ by 0.01 or more in
x or y and turn out of the outline. In this test a straight segment counts as having its handles a third of the way
along it, so an anchor between two straight segments longer than 0.03 pt is a corner only when it turns out of the
outline. On a tie, the anchor met first along the outline in the order it was drawn wins.
Without such an anchor, the corner is the outline's point nearest the box corner, moved onto the anchor at the end of
its segment when within 0.5% of the segment's parameter. Each piece of a side between anchors and corners takes a
share of the content's width or height in proportion to the length of its control polygon, spread along the piece by
its curve parameter, so content spreads evenly along a segment without handles. An envelope opened from an `.ai` file
keeps the corners the file saves, and they stay on the same anchors when the envelope is moved, rotated or
reflected. These are VectorCraft's own rules, and existing top-object envelopes redraw by them.
`object.envelope.info` reads the selected envelope's settings (`type`, the warp's or mesh's values, `fidelity` and
the options, `editing`), or with no envelope selected the options new envelopes get. With an envelope selected,
`object.envelope.resetWithWarp` and `object.envelope.resetWithMesh` (`rows`, `cols`, `maintainShape`: true keeps
the current shape) switch its kind and keep its content. `object.envelope.options` sets `fidelity` and the options
(`antiAlias`, `preserveShape`: clippingMask or transparency, `distortAppearance`, `distortLinearGradients`,
`distortPatternFills`; the last two need `distortAppearance`); with no envelope selected it sets them for new
envelopes. With `antiAlias` false, the envelope's distorted images are drawn without smoothing, SVG export writes
them with `image-rendering: pixelated` and PDF export writes them without `/Interpolate`. With `preserveShape`
`transparency`, each piece of a distorted image is the image with a white opacity mask in the piece's shape instead
of a clip group, after Expand too. New envelopes start with Distort Appearance on, as in the reference app; envelopes saved before these
options existed keep their old look (appearance applied after the distortion).

An envelope opened from a `.ai` can have margins, written to native files as `boundsMargin` and reported by
`object.envelope.info` when they are not zero: `[top, bottom, left, right]`, as shares of the content's height and
width. While `distortAppearance` is on, the content maps from its geometric bounds grown by the margins, and while it
is off, the content maps from its geometric bounds. Leaving Edit Contents measures the margins again when the content's
strokes reach differently than when Edit Contents began, and always for an envelope whose file opened in Edit Contents.
New envelopes have no margins, and no command sets them.

`object.envelope.release` gives back the content and the envelope's shape (a grey gradient mesh for warp and mesh
envelopes, the path for a top-object envelope); the content keeps the envelope's opacity, blend mode and opacity mask
(a group around it carries them when there are several objects). `object.envelope.expand`, and `object.expand` with
`object` on, replace envelopes by groups of their distorted content that keep the name, transparency, knockout and
opacity mask.
Type inside an envelope distorts as its glyph outlines, run by run in each run's paint, on the canvas and in every
export (SVG, PDF, EPS, EMF/WMF, DXF).
An envelope maps type from the bounds of its glyph outlines, underlines and strikethroughs instead of its laid-out
box (ascent to descent, or an area's frame) when the type is on its own, in groups and layers, or used as a clipping
path. Inline graphics in the type count by their boxes. Rotated type and type in perspective map from the bounds of
their outlines as they are drawn. `object.envelope.makeWithMesh`, `object.envelope.makeWithTopObject` and
`object.envelope.resetWithMesh` with `maintainShape: false` use those bounds. Type that draws nothing keeps its
laid-out box, and so does type inside any other object, such as a blend, a repeat or a symbol instance. This is
VectorCraft's own rule, and envelopes of type in saved files redraw by it.
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
its current size (with Shift too it keeps its proportions). `press_key` Escape during a stroke cancels it and adds no
undo step; during an Alt-drag it puts the brush size back. With Use Pressure Pen on, each `pointer_gesture` event's
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

The Liquify, Puppet Warp and perspective commands refuse a string or another value that is not a number with
`` `diameter` must be a number, not "40" ``, naming the key, and leave the document and its undo history as they
were: `diameter`, `width`, `height`, `angle`, `intensity`, `detail`, `simplify`, `rate`, `complexity`, `horizontal`
and `vertical` in `object.liquify`, with any `tool`; `expand` in `object.puppetWarp` and `object.puppetWarp.pins`;
`depth` in `perspective.transform`; `dx` and `dy` in `perspective.nudge`; and `offset` and `by` in
`perspective.plane.move`. A null is read as a key left out. `object.puppetWarp` refuses an `angles` entry that is not
a number or null with "angles must be a list as long as pins (degrees or null)", `perspective.draw` refuses an `at`
that is not two numbers, such as `["450", 380]`, with "`at` must be [x, y]", and `stroke.widthProfile.set` refuses a
point that is not three numbers, such as `[0.5, "1", 2, 2]`, with "points must be [t, left, right] with non-negative
widths".

## Perspective Selection

Objects in perspective keep their attachment themselves (`perspective` on the object: `plane` and `depth`, the
distance along the plane's normal), so copies, duplicates and pastes stay attached. With the
`perspectiveSelection` tool, a drag moves the selection within its plane (`perspective.move`; Alt copies), the
bounding-box handles scale it in plane space (`perspective.transform`; Shift keeps proportions, Alt scales about the
centre), and `press_key` `5` during a drag switches the move to perpendicular to the plane (press again to switch
back). Holding Shift during an in-plane move constrains each object's displacement to an axis or 45° diagonal of its
own plane. `perspective.move` accepts `constrain: true` (default false) and adjusts Snap to Grid along the constrained
direction on the first object's plane and depth. With Snap to Grid, every object on the first object's plane moves
by the same plane offset, whatever its depth, and the edge of their joint bounds nearer a gridline lands on it.
`press_key` Escape during a move, a resize or a plane widget drag cancels it and puts the objects and the
grid back. The arrow keys nudge a selection in perspective by the keyboard increment (`perspective.nudge`; Shift ×10, Alt
copies). Object › Transform › Transform Again repeats the last perspective move or scale in plane space.

```json
{"name":"pointer_gesture","arguments":{"tool":"perspectiveSelection","events":[{"kind":"down","x":480,"y":410},{"kind":"drag","x":470,"y":405}]}}
{"name":"press_key","arguments":{"key":"5"}}
{"name":"pointer_gesture","arguments":{"events":[{"kind":"up","x":470,"y":405}]}}
{"name":"run_command","arguments":{"command":"perspective.transform","params":{"matrix":[1.5,0,0,1.5,0,0],"copy":true}}}
{"name":"run_command","arguments":{"command":"object.transformAgain","params":{}}}
```

The perspective, width point, Liquify and Puppet Warp commands refuse a string, a number or another value that is not
true or false with `` `copy` must be true or false ``, naming the key, and leave the document and its undo history as
they were: `copy`, `perpendicular` and `snap` in `perspective.move`; `copy` in `perspective.transform`; `big` and
`copy` in `perspective.nudge`; `snap` in `perspective.draw`; `reproject` in `perspective.grid.set`; `visible` in
`perspective.grid.show`; `adjustAdjoining` in `stroke.widthPoint.set`; `usePressure`, `simplifyOn`, `affectAnchors`,
`affectIn` and `affectOut` in `object.liquify`, with any `tool`; and `rest` in `object.puppetWarp`. A null is read as
a key left out.

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
else Smooth Color and Align to Page. Smooth Color counts the steps between each pair of keys on its own, from the
keys' colors and visible bounds rather than along the spine. The counts below are VectorCraft's own values. The
colors compared are each key's solid fill and stroke, the fills and strokes of two text objects' runs at the same fraction of their characters, and
those of two groups' members paired in order. When they differ, the count is 255 times the largest change of one
channel as a fraction of its range, a half step rounding down, from 1 to 254. An RGB channel's range is 256 values,
so pure red to pure blue makes 254 steps. CMYK and gray count by their percentages, two tints of the same swatch by
their tints, and colors in different spaces (or Lab) by their RGB values over 255. The count is then at most eight
per point of the largest of the keys' widths and heights and the distance between their centers along either axis,
and when a key has no bounds it is the color count alone. When the colors match, a solid paint meets no paint, or a
key holds a gradient or a pattern, the count is the largest distance between the keys' corresponding visible
bounding-box edges divided by their mean width (left and right edges) or height (top and bottom), rounded, less one,
from 1 to 1000. Keys with no width or no height are measured by their other side, keys with neither in points, and
keys without bounds make 1 step. `starts` gives, per object, the anchor of its first subpath the blend starts
from (as the Blend tool's clicks on anchor points do; an open path's last anchor runs it the other way). With the
Blend tool, `press_key` Escape drops the object clicked first and stops the next click adding to the blend
just made; the document and the selection stay as they are. A blend
among the objects takes the others in as more key objects, keeping its options, name and transparency:

```json
{"name":"run_command","arguments":{"command":"object.blend.make","params":{"ids":[12,15],"starts":[0,2],"steps":8}}}
{"name":"run_command","arguments":{"command":"object.blend.make","params":{"ids":[20,31]}}}
{"name":"run_command","arguments":{"command":"object.blend.info","params":{}}}
```

With the Blend tool active, `tool.select {tool: "blend"}` finishes the blend, as a click on the tool's button or W
does: it drops the object clicked first and stops the next click adding to the blend just made. The document and the
selection stay as they are. Choosing any other tool that is already active leaves its work as it is.

A blend's spine is the straight lines between its key centres until it is edited. `object.blend.info` lists it
(`spine.anchors`, `keyAnchors`: the point each key sits on). `object.blend.spine.addAnchor {x, y}` adds a point where
the spine passes nearest, `object.blend.spine.moveAnchor {anchor, x, y}` moves a point (a key on it moves along; with
`handle: "in" | "out"` it places that handle), `object.blend.spine.convertAnchor {anchor, to: "smooth" | "corner", x?, y?}` converts a point's handles (a Smooth drag with `x, y` creates opposite handles; Corner retracts both), `object.blend.spine.reshapeSegment {segment, t, dx, dy, perpendicular?}` bends a segment
so its point at `t` moves by (dx, dy), turning a straight spine into a curve (`perpendicular` works as in `path.reshapeSegment`), and `object.blend.spine.removeAnchor
{anchor}` deletes a point no key sits on; the first edit turns the spine into a path. Moving a key moves its end of the spine. Release
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

## Objects on Path

`object.objectsOnPath.attach {path?, ids?, pivot?, rotate?}` attaches the selected objects (or `ids`) to the path
`path`: the objects and the path go into an Objects on Path group, with the path at its bottom. The objects go along
the path in their order on the canvas, column by column from left to right and top to bottom in each column. The first
one sits on the path's first anchor, the last one sits on the end of an open path, and the others are spread evenly
between them by length; on a closed path the gaps run all the way round. The attachment point of each object (`pivot`,
0 to 8 row by row from the top left of its bounds without strokes) sits on the path, and the object is turned to the
path's direction there. An object rotated by other than a multiple of 90° is first turned to the nearest multiple.
`rotate` tilts every object by that many degrees counter-clockwise. Without `pivot` or `rotate`, the command uses the
Objects on Path tool's options (4, the center, and 0 until they are changed). Without `path`, the command chooses the
Objects on Path tool (`objectsOnPath`), whose click on a path sends `{path, pivot, rotate}` from its options. As the
path, the command
refuses a guide, a compound path, type, a path without length, a path already in an Objects on Path group, a
clipping path, and a path inside a compound path, blend, envelope, repeat or Intertwine group. As objects, it skips
layers and refuses graphs, objects already in an Objects on Path group, clipping paths, and objects inside a blend,
envelope, repeat or Intertwine group; a member of a compound path attaches as its whole compound path. The tool
does not highlight a clipping path or a path inside a blend, envelope, repeat or Intertwine group, and a click on one
attaches nothing.

The group stays live: after every edit its objects go back on its path. Editing the path moves them, an object moved
on its own goes back to its place, an object moved into the group (`layer.move` with `place: "inside"`) joins the end
of the path, an object moved out of it leaves, and art that replaces an object (`symbol.breakLink`) keeps its place.
`object.objectsOnPath.options {ids?, pivot?, rotate?, spacing?, start?}` changes Pivot, Rotate and Space, and
`object.objectsOnPath.expand {ids?}` makes the groups plain groups. Both act on the groups that `ids` names (Objects
on Path groups or objects in them), or on the groups of the selection when `ids` is not given. The native file keeps
the group with an `objects_on_path` record; SVG, PDF and the other exports write the path and the objects as plain
art.

Space sets the distance along the path between neighboring objects' attachment points. `spacing` takes points, 0 or
more, or `"auto"`, the default, which spreads the objects evenly with the last one at the end of an open path. `start`
is the distance in points along the path from its first anchor to the first object's attachment point (default 0). A
`spacing` larger than the path has room for is drawn smaller: on an open path the last object stops at the end, and on
a closed path the objects go no more than once around. The value is kept, so lengthening the path again brings the
objects back to it. On a closed path, `start` counts around from the first anchor. Pivot, Rotate, moving the group and
saving the file keep the Space. Scaling, rotating or shearing the group keeps the first and last objects where the
transform takes them on the new path, and sets `spacing` and `start` to match; a value that cannot be kept that way
scales as much as the path's length.

In the desktop app, double-clicking the Objects on Path tool (`tool.options {tool: "objectsOnPath"}`) opens Objects on
Path Tool Options (dialog `objectsOnPathOptions`): the nine-point Pivot grid and the Rotate field (°). Save sets the
tool's options (`tool.setOption {tool: "objectsOnPath", values: {pivot, rotate}}`), which the objects attached next
use; groups attached before keep their Pivot and Rotate. Reset (or Cancel while Alt, Option on macOS, is held, or
`ui.dialogReset`) puts the center and 0 back and keeps the dialog open. The options last across tool switches and are
saved with the preferences.

With one Objects on Path group selected, the Selection and Objects on Path tools show the Rotate All widget, a round
handle beyond the top of the group's first object joined by a line to its attachment point. Dragging it turns every
object about its attachment point by the angle the pointer turns about the first object's attachment point,
counter-clockwise on the page: the drag previews `object.objectsOnPath.options {ids: [group], rotate}` with the
group's Rotate plus that angle and keeps it as one undo step on release. Shift turns in steps of 15°, and Escape
cancels the drag.

With two or more objects in the group, the same tools show the Space widget, a square handle beyond the group's last
object, ahead along the path and joined by a line to its attachment point. Dragging it moves the last object along the
path as far as the pointer moves along the path, and previews
`object.objectsOnPath.options {ids: [group], spacing, start}` and keeps it as one undo step on release. The first
object stays where it is and the others move toward or away from it; on an open path the last object goes no further
than the end. With Alt (Option on macOS) held, the objects spread from both ends toward or away from the center of an
open path, or of the first anchor of a closed path, up to the whole path. With measurement labels on, the distance
between the objects shows next to the pointer in the document's units. Escape cancels the drag.

```json
{"name":"run_command","arguments":{"command":"tool.setOption","params":{"tool":"objectsOnPath","values":{"pivot":7,"rotate":30}}}}
{"name":"run_command","arguments":{"command":"object.objectsOnPath.attach","params":{"path":30,"ids":[12,15,18]}}}
{"name":"run_command","arguments":{"command":"object.objectsOnPath.options","params":{"pivot":7,"rotate":15}}}
{"name":"run_command","arguments":{"command":"object.objectsOnPath.options","params":{"spacing":24,"start":0}}}
{"name":"run_command","arguments":{"command":"object.objectsOnPath.expand","params":{}}}
```

## Paragraph composer

`text.setFormat {ids?, composer: "singleLine"|"everyLine"}` sets how a text object's lines are broken (the Paragraph
panel menu's Single-line and Every-line Composer; stored in the paragraph attributes as `para.composer`, saved only
when it is Single-line). Every-line, the default, picks the breaks of the whole paragraph together in area type:
justified text gets even word spacing, ragged text (left, centre or right aligned) an even rag, so it may move a word
that fits to the next line to avoid a lone short word on the last line or a full line next to a short one.
Single-line fills each line as far as it goes. Point type and type on a path have no line width, so both composers
give the same result there. A paragraph whose characters differ in line metrics (type size, font, leading or vertical
scale, or an inline graphic taller than the text) is composed with Every-line in rectangular area type, rows and
columns included; in area type of another shape, or with text wrap, it falls back to Single-line.
The UI command `type.lineComposer` (Cmd+Alt+Shift+C, in remote mode `press_key {key: "C", mods: {cmd: true, alt: true,
shift: true}}` or `invoke_menu`) sets `singleLine` when every paragraph of the selected type (or every paragraph the
Type tool's selection touches) uses Every-line, else `everyLine`, and returns the composer it set.

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
adds a row and a column where it clicks inside. `press_key` Escape during a drag of a point or handle of a mesh
envelope or a gradient mesh cancels it: the point goes back and no undo step is added. The Control bar shows Edit Envelope / Edit Contents, the warp's
style, orientation, bend and distortions (or the mesh's rows and columns), Reset (an unbent warp, a flat mesh) and
Envelope Options.

```json
{"name":"run_command","arguments":{"command":"object.envelope.editContents","params":{"editing":true}}}
{"name":"run_command","arguments":{"command":"object.mesh.movePoint","params":{"id":12,"index":4,"x":220,"y":140,"handle":0}}}
```

## Picking several mesh points

The Mesh tool and Direct Selection pick mesh points on gradient meshes and mesh envelopes. A click picks one point
(its handles show); Shift-click adds a point or takes a picked one away. Direct Selection's marquee picks the points
inside it (Shift toggles them) and selects their meshes, as does a Lasso loop round them (Shift adds, Alt takes
away); its click on a gradient mesh's line picks the line's two
ends, and a click inside a patch picks the four points around it. The picked points show filled. Dragging one of them
moves them all as one undo step (`object.mesh.movePoints`), a click on one of several picked points picks it alone,
and the arrow keys nudge them by the keyboard increment. The active tool lists them in its options
(`tool.setOption {}` returns them as `meshPoints: [{id, index}…]`). On gradient meshes a solid fill colour (`paint.setFill`, from the
Color panel, Swatches or the Eyedropper) recolours just the picked points, and when every target of
`transparency.set` is a gradient mesh with points picked, its `opacity` fades those points instead of the objects
(`transparency.info` then shows their opacity, `null` when it differs). Both are one undo step.

`object.mesh.movePoints {id, points: [index…], dx, dy}` (or `{items: [{id, points}…], dx, dy}` over several meshes)
moves points by (dx, dy), their handles with them; `object.mesh.setPointColor` takes `points` or `items` the same
way to colour or fade several points at once. An index out of range, an index that is not a whole number ≥ 0, or a
move past the coordinate range fails and changes nothing.

```json
{"name":"run_command","arguments":{"command":"object.mesh.movePoints","params":{"id":12,"points":[0,4],"dx":10,"dy":-5}}}
{"name":"run_command","arguments":{"command":"object.mesh.setPointColor","params":{"id":12,"points":[0,4],"color":"#3366ff","opacity":0.5}}}
```

## Shortcut sets from other apps

`shortcutSet.read {path? | data? | dataBase64?, name?}` reads a shortcut set another app saved as text (a `.kys` file
that starts with `/Menus {`) without changing the document or the preferences. Each block that sets `/Key` is an
entry, listed with its `name`, its `section` (the names of the blocks around it), its codes as the file writes them
(`key`, `modifiers`, `context`, `represent`) and its `line`. `codes` counts the entries that use each key and modifier
code, and `unread` lists the tokens the reader skipped, with their lines. The reader stops at 4 MiB, 16 open blocks,
20,000 entries and 8 MiB of names over the entries and their sections, cuts names at 1,024 bytes, and sets `truncated` when it reaches one of these limits. An XML shortcut
file fails with "This shortcut file is in a form VectorCraft can't read."

The import matches record what the names and codes in such files stand for in VectorCraft. They are kept with the
preferences, and resetting the preferences keeps them. `importMatches.set {matches: [{kind, name, target, label?}]}`
records them: kind `name` maps an entry name to a command id, `tool:<id>` or `panel:<id>`, and kinds `key` and
`modifiers` map a code, written in decimal, to a key or to modifiers as Edit › Keyboard Shortcuts writes them (`K`,
`Cmd+Shift`). A `target` of `""` records that there is no equivalent (for kind `modifiers`, no modifiers), and `null`
removes the match. `importMatches.list {kind?, query?}` lists them and `importMatches.reset {kind?}` removes them.
`shortcutSet.read` then adds the saved `target` to each entry, a `chord` to each entry whose codes are both known, and
the saved `key` or `mods` to each code.

In the desktop app, Edit › Keyboard Shortcuts › Import… and File › Open show such a file in Import Shortcut Set
(`ui.shortcutImportDialog {path? | dataBase64?, name?}` over the control channel). The dialog proposes targets from
the entry names and decodes the codes that VectorCraft's default shortcuts agree on.

```json
{"name":"run_command","arguments":{"command":"shortcutSet.read","params":{"path":"/Users/me/Studio.kys"}}}
{"name":"run_command","arguments":{"command":"importMatches.set","params":{"matches":[{"kind":"name","name":"Group","target":"object.group"},{"kind":"key","name":"901","target":"G"}]}}}
```

## Action sets from other apps

`actionSet.read {path? | data? | dataBase64?, name?}` reads an action set another app saved as text (an `.aia` file
that starts with `/version`) without changing anything. Give exactly one of `path`, `data` and `dataBase64`. It returns
the set's name (`set`, else the file's name, else empty), its `version`, and its `actions`, each with its `name` and
`steps`. A step lists its `name` (the file's `/internalName`), its `label` (the `/localizedName` it is shown with, or
null when it has none or it is blank), `on` (false when the file sets `/isOn 0`), `params` (how many recorded values it
has) and its `line`. `unread` lists the tokens the reader skipped, with their lines. The reader stops at 8 MiB, 16 open
blocks, 1,000 actions, 10,000 steps in an action and 100,000 steps in the set, and cuts names at 1,024 bytes. It sets
`truncated` when it reaches one of these limits, when the text ends inside a block or a value or after a field's name,
when `/actionCount`, `/eventCount` or a step's `/parameterCount` differs from the actions, steps or recorded values read
(a negative count, one too large for a number and one that is not a number always differ), or when a hexadecimal name,
label or recorded `/value` is not as many bytes as its count says. Text that does not start with `/version` and a
number is an error.

`ui.actionSet.load` (the Actions panel menu's Load Actions…) reads such a file too, from `path`, a picked file or
`data`, and adds its set after the other sets with all its actions. The set's name is the file's `/name`, else the
file's name, else `Actions`. A step with a label and no recorded values comes in as the VectorCraft command with that
label, in English or in the interface language, in any case and without a final `…`, when exactly one menu command has
that label and that command is directly in a top menu, can be recorded in actions, is what its menu bar item runs with
no values and opens no dialog (Group, Ungroup). A submenu's items, file and document commands, Paste, Undo, Redo and
add-on commands are never matched. The step runs with no params, and a step that is off in the file is excluded from
playback. Other steps are left out, including every step with no label and every step recorded with values (recorded
menu items too). The reply lists the first 1,000 in `skipped` (`{action, step}`, the step by its label, else its name),
`leftOut` counts them, and the status bar says how many were left out. An action with no name is called `Action N`, by
its place in the file. Function keys and colors are not read. A file marked `truncated` is refused, and no set is added.
Text that starts `/version` and is not an action set gives "not an action set file". Tokens in `unread` do not stop a
load.

```json
{"name":"run_command","arguments":{"command":"actionSet.read","params":{"path":"/Users/me/Prep.aia"}}}
{"name":"run_command","arguments":{"command":"ui.actionSet.load","params":{"path":"/Users/me/Prep.aia"}}}
```

## Artwork Cleanup add-on

Artwork Cleanup is a built-in add-on: its commands are `addon.cleanup.*`, and in the desktop app it lives in Object ›
Plug-ins › Artwork Cleanup and in its panel (`window.panel {panel: "artworkCleanup"}`).

`addon.cleanup.simplify {tolerance?, protectCorners?, ids?}` removes the anchors each selected path can do without and
refits the handles beside each one, so the outline keeps its shape. `tolerance` runs from 1 to 100 (20 by default); the
outline stays within tolerance × 0.02 pt of where it was, measured against the original outline, so removals don't add
up. With `protectCorners`, anchors where the path turns by more than 30° stay. Open ends always stay. Locked and hidden
paths, and paths in locked or hidden groups, are left out. It reports the anchor counts `before` and `after`, as one
undo step.

The health check looks for issues in the selection, or in the whole document while nothing is selected (`scope:
"selection"|"document"` picks one). Locked and hidden objects, and the insides of live or generated art (blends,
envelopes, graphs, symbols), are left out.

- `addon.cleanup.check {scope?, kinds?}` changes nothing and lists `findings`: for each kind found, its `count`, the
  `ids` of the objects it is on (at most 1,000) and, for empty artboards, their indexes.
- `addon.cleanup.show {kind, scope?}` selects the objects with that issue.
- `addon.cleanup.fix {scope?, kinds?}` fixes the issues as one undo step and reports how many of each kind it fixed.
  The kinds run in a fixed order, so art removed by one kind can leave a group that a later kind unwraps, and they
  run again (three rounds at most) while a round still fixes something.

| Kind | Finds | Fix |
|---|---|---|
| `emptyText` | Text with no characters, or only spaces (not clipping paths) | Removes it |
| `strayPoints` | Paths of one anchor (not clipping paths) | Removes them |
| `unpainted` | Paths and compound paths with no fill and no stroke | Removes them |
| `zeroOpacity` | Objects at 0 % opacity | Removes them |
| `duplicateObjects` | A path stacked directly on an identical one (geometry, paint, opacity), or type stacked directly on the same type (text, styles, paint) within 1 pt of it | Removes the upper one |
| `offArtboard` | Top-level objects that touch no artboard (not clipping paths) | Removes them |
| `duplicateAnchors` | Neighbouring anchors within 0.01 pt | Merges them |
| `tinyHandles` | Handles shorter than 0.01 pt | Retracts them |
| `collinearAnchors` | Corner anchors on a straight run (turning by a sine of 0.005 or less) | Removes them |
| `nearPoints` | Anchors within 0.01 pt of anchors of other paths | Snaps them together |
| `nearlyClosed` | Open paths whose ends lie within 0.01 pt | Closes them |
| `filledOpenPaths` | Open paths with a fill and no stroke | Closes them |
| `needlessClipGroups` | Clipping groups whose unpainted clip clips nothing, hides all their art, or is a rectangle round all of it | Removes the group (and the art it hides), or leaves the art |
| `needlessCompounds` | Compound paths of one path or none | Leaves the path, painted as the compound was |
| `needlessGroups` | Groups of one member or none, with no paint, effects or transparency of their own | Leaves the member |
| `emptyArtboards` | Artboards with no art (document scope only; one artboard always stays) | Deletes them |

`addon.cleanup.deepUngroup {includeClipGroups?, ids?}` takes apart every level of grouping in the selection. Clipping
groups stay whole unless `includeClipGroups` is true; their clipping paths then become plain paths. Objects keep the
opacity of the groups they were in. Locked and hidden groups stay whole, and locked and hidden objects stay locked or
hidden and are left out of the selection.

`addon.cleanup.rejoin {gap?, ids?}` welds the selected open paths whose ends lie within `gap` (0 to 6 pt, 0.1 by
default) back into paths. Each end joins one other end: where several are in reach, the one the path turns least to go
on into. Pieces may run either way, and a chain whose ends meet closes. Each new path takes the place and paint of its
topmost piece.

```json
{"name":"run_command","arguments":{"command":"addon.cleanup.check","params":{"scope":"document"}}}
{"name":"run_command","arguments":{"command":"addon.cleanup.fix","params":{"kinds":["strayPoints","needlessGroups"]}}}
{"name":"run_command","arguments":{"command":"addon.cleanup.simplify","params":{"tolerance":40,"protectCorners":true}}}
```

### Text tools

The add-on's text tools act on the selected type (or `ids`), each as one undo step, and refuse with an error when there
is nothing for them to do. The pieces they make are point type that keeps its characters' styles, each placed so the
glyph of its first character lands where it was drawn.

- `addon.cleanup.breakText {by?, ids?}` turns each horizontal point or area type object into point type objects, one
  per line (`by: "lines"`, the default), paragraph (`"paragraphs"`), word (`"words"`) or glyph (`"glyphs"`; a ligature
  stays one piece). Spaces between the pieces go. A paragraph keeps its lines: where the layout wrapped one, the piece
  has a forced line break. Justified lines close up, and a paragraph broken out whole keeps its bullet (not its
  number; the other pieces have no list markers). Type on a path, vertical type, threaded type and area type whose text doesn't fit stay as they are and are
  listed in `skipped`.
- `addon.cleanup.combineText {ids?}` merges one-line horizontal point type into as few objects as it can, every glyph
  staying where it was (within 0.01 pt). Pieces turned and sheared alike whose baselines lie within 0.4 em of each
  other join left to right (a baseline a little off becomes baseline shift); a space goes between pieces at least half
  a space apart, and tracking on the character before each piece keeps its place. A gap wider than 10 em, an overlap of
  more than 1 em, another look (paint, opacity, effects) or a piece that can't keep its place once joined (a ligature
  forming across the join) starts another object. Each object made takes the place and paint of its leftmost piece. The
  main use is type from a PDF that arrives one glyph per object.
- `addon.cleanup.joinText {ids?}` (Join Point Text) joins one-line horizontal point type into one point type object,
  for text that arrives one line or fragment per object. Pieces turned and sheared like the top-left one are read as
  lines (baselines within 0.4 em), top to bottom; a line's pieces join left to right with a space between them. Each
  line becomes a left-aligned paragraph whose leading is the distance from the baseline above, so the lines stay where
  they were. Character styles stay; the object takes the top-left piece's place and paint. Returns `{id, joined, lines}`.
- `addon.cleanup.removeBreaks {kind, ids?}` removes the forced line breaks (`kind: "line"`) or paragraph returns
  (`"paragraph"`) in the characters selected with a Type tool, else in all the text of the selected type. After
  Select All in a frame of threaded text, it removes them from the text of every frame of the story, as one undo step.
  A break between two words becomes a space; merged paragraphs take the first one's paragraph style. The frames of a
  threaded story, selected all together or through Select All, are read as one text, so a break at the end of one
  frame before a word in the next one also becomes a space.
- `addon.cleanup.splitText {}` works while a Type tool edits horizontal point or area type: the text before the
  selected characters, those characters and the text after them become point type objects (at a bare cursor, the text
  before and after it). Spaces at the ends of each piece go; a piece that starts inside a left-aligned line takes a
  first-line indent, so its later lines stay where they were. Editing ends with the pieces selected.
- `addon.cleanup.alignInPlace {align, ids?}` aligns every paragraph of each horizontal point type object `left`,
  `center` or `right` and moves the object so its longest line stays where it is: a single line doesn't move at all.
- `addon.cleanup.resetTextTransform {scale?, ids?}` takes the rotation, shear and reflection off point type, keeping its
  size and the centre of its text. With `scale: true` the horizontal scale goes too, the object's and each character's.
- `addon.cleanup.replaceFonts {font?}` sets every character of the document's type whose font family isn't available
  in `font` (by default the default type font, Source Sans 3), in that family's style closest to the one it had. The
  type in the art of the document's symbols is replaced too.

In the panel, the Text row (which the panel can hide) runs them: Shift-click Lines to break into paragraphs, and
Alt-click Reset Transform to reset the horizontal scale too. Recovering live text from outlines, combining vertical
type and evening out horizontal scales are not there yet.

```json
{"name":"run_command","arguments":{"command":"addon.cleanup.breakText","params":{"by":"words"}}}
{"name":"run_command","arguments":{"command":"addon.cleanup.combineText","params":{}}}
{"name":"run_command","arguments":{"command":"addon.cleanup.removeBreaks","params":{"kind":"paragraph"}}}
```

## Path Precision Tools add-on

Path Precision Tools is a built-in add-on for exact anchor and path work: its commands are `addon.pathtools.*`, and in
the desktop app it lives in Object › Plug-ins › Path Precision Tools, in its Anchor Inspector panel
(`window.panel {panel: "anchorInspector"}`), in its Parametric Shape panel (`window.panel {panel: "parametricShape"}`)
and in seven tools that the toolbars don't show until they are added from the All Tools drawer (or chosen with
`tool.select`): `anchorEraserBrush`, `extendPath`, `measureInspect`, `parametricShape`, `cornerTool`, `slideAnchor` and
`sharedEdge`.
None has a shortcut. Each edit is one undo step, and an edit that changes nothing makes no step.

Tools:

- **Anchor Eraser Brush** (`anchorEraserBrush`): a drag over selected paths runs `addon.pathtools.erase` with the
  stroke. Options (`tool.setOption`, or a double-click on the tool): `size` (screen pixels, 1 to 2500, 20), `tolerance`
  (1 to 100, 10) and `pressure` (a pen's pressure scales the brush). Alt removes every anchor under the brush that can
  go whatever the tolerance; Cmd/Ctrl deletes them without refitting; `[` and `]` resize the brush.
- **Extend Path tool** (`extendPath`): a drag from an end of an open path runs `addon.pathtools.extend` with the length
  the pointer asks for: forward carries the path on, back along it shortens it. Option `mode`: `straight` (default),
  `bezier` or `arc`; E cycles it. Shift steps the length by the keyboard increment.
- **Measure & Inspect tool** (`measureInspect`): a drag measures distance, angle and offsets, as the Measure tool does
  (`tool.options` reports the last one); hovering a path reads it, as `addon.pathtools.inspect` does. Nothing goes into
  the document.
- **Parametric Shape Tool** (`parametricShape`): draws shapes that stay editable as numbers. A drag from the centre runs
  `addon.pathtools.shape.create`: the drag's length is the shape's size (height 2 × the distance, width in the last
  shape's proportions), its direction the rotation (Shift: 45° steps). While dragging, ↑/↓ change the kind's count or
  first value and ←/→ its second value or corner radius. A click selects a parametric shape and shows its handles:
  centre (move), the box's lower right corner (size; Shift keeps the proportions), a handle above the top (rotation;
  Shift: 45° steps), a dot on the top edge (corner radius) and one of the kind's own (a star's or ring's inner radius, a
  gear's tooth height, an arrow's head, a cross's bar, a heart's notch, a bubble's tail tip); each drag runs
  `addon.pathtools.shape.set`. A double-click on the canvas places a shape with the last settings. Options
  (`tool.setOption`, or the panel with no shape selected) are the next shape's numbers, as the commands take them.
- **Corner Tool** (`cornerTool`, or the UI command `addon.pathtools.cornerTool`, which also shows the Anchor Inspector
  panel with its Corners section): edits the app's Live Corners on any path, selected or not. Over an anchor where two
  sides meet at an angle the anchor shows; a click runs `addon.pathtools.cornerSet` with the tool's `radius` (pt, 10)
  and `kind` (`round`, `invertedRound` or `chamfer`), and a drag from it sizes the corner (Shift: whole values of the
  ruler's unit; no larger than fits). A cut corner shows a marker at the centre of its cut: a click selects the corner
  and loads its radius and kind into the tool, a drag resizes it (to 0 it goes), a double-click removes it. An Alt-drag
  cuts corners on every anchor in the marquee, replacing those there; a plain marquee selects the corners inside (Shift
  toggles), and a click away deselects. The panel section sets `radius` and `kind`, counts the selected corners and has
  Apply to Selected Anchors.
- **Slide Anchor Along Path** (`slideAnchor`): a drag on an anchor runs `addon.pathtools.slideAnchor`, previewed while
  dragging: the anchor runs along the curve it sits on, as far as just short of its neighbours, and the two segments
  beside it are refitted. Shift snaps to where the path's tangent is at a multiple of 45° from the Constrain Angle. Ends
  of open paths don't slide.
- **Shared Edge Merge Tool** (`sharedEdge`, or the UI command `addon.pathtools.sharedEdgeTool`, which also shows the
  Anchor Inspector panel with its Shared Edges section): works on any two unlocked paths under its round brush, selected
  or not. Where they touch at two coincident anchors, hovering shows the two stretches between those anchors dashed and
  the edge replacing them solid (green when they are one edge already); a click runs `addon.pathtools.mergeEdge`, half
  way between the two. Shift keeps the upper path's outline, Alt the lower's. Where the anchors aren't there yet a click
  makes an end: two near anchors meet half way (or, with Shift or Alt, on the path kept), and an anchor only one path has
  there gets a match on the other. Shift+Alt makes an end even inside a stretch that could merge. Stretches over 10%
  different in length, or at over 120° at an end, are left alone. Options (`tool.setOption`, or the panel, kept with the
  preferences): `diameter` (0.25 to 200 pt, 20), `tolerance` (how near two anchors count as coincident, 0 to 3 pt,
  0.002) and `together` (Keep Shared Points Together).

Commands:

- `addon.pathtools.erase {points, diameter?, tolerance?, ignoreTolerance?, plain?, ids?}` removes the anchors of the
  selected paths that a round brush of `diameter` pt (20 by default), swept through `points` (`[x, y]`, or `[x, y,
  pressure]` to scale it), passes over, wherever the outline stays within tolerance × 0.02 pt of where it was (the
  scale `addon.cleanup.simplify` uses), refitting the handles beside each. Open ends stay. → `{ids, removed}`.
- `addon.pathtools.extend {id, subpath?, end, length | to, mode?}` lengthens an open path at `end` (`"start"` or
  `"end"`) by `length` pt, or shortens it by a negative length (never all of it). With `to: [x, y]` instead, the length
  is the one that brings the end nearest that point. `mode`: `straight` (along the end's direction), `bezier` (the last
  segment's curve stretched on, no anchor added) or `arc` (along the circle the end curves on, at most half way round;
  a straight end goes on straight).
- `addon.pathtools.inspect {id, x, y, tolerance?}` (a query) reads the path at its point nearest `(x, y)`: open or
  closed, anchor count, the subpath's length and enclosed area (an open one closed by a straight line), the segment's
  length, the direction the path runs there and its radius of curvature (`null` where straight), and, within
  `tolerance` (3 pt) of an anchor, that anchor's angle (90 at a square's corner, 180 where the path runs straight on;
  `null` at an open end) and its handles' lengths and directions.
- The Anchor Inspector's edits act on the direct-selected anchors, or on every anchor of a path selected whole:
  `addon.pathtools.setAnchor {x?, y?, inLength?, inAngle?, outLength?, outAngle?}` (page coordinates; angles in
  degrees counter-clockwise; a length of 0 retracts), `addon.pathtools.smooth {ratio?}`, `addon.pathtools.corner
  {retract?}` and `addon.pathtools.equalize {}`. Make Smooth mirrors a lone handle; an anchor without handles gets two
  at right angles to the bisector of the angle its neighbours make at it, each `ratio` (0.1 to 0.5, 0.3905 by default:
  the ratio that makes a square's corners a circle) × the nearer neighbour's distance long; an open end aims its handle
  at its neighbour's. Equalize gives both handles their average length and makes a missing one opposite the other.
- `addon.pathtools.removeKeepShape {}` removes the direct-selected anchors, refitting the handles beside each.
- `addon.pathtools.addPoints {count?}` splits each selected segment (between two selected anchors, or every segment of
  a path selected whole) into `count + 1` (1 to 100) pieces of equal length.
- `addon.pathtools.close {mode?, ratio?}` closes the selected open paths keeping the ends' handles (`handles`), with a
  straight segment (`straight`) or with both ends made smooth (`smooth`).
- `addon.pathtools.crossings {cut?, selfCrossings?, ids?}` adds anchors where the selected paths cross one another (and
  where each crosses itself, with `selfCrossings`), or with `cut` also cuts them there: a path of one subpath becomes a
  path per piece. Paths that meet at a shared anchor don't cross there; at most 10,000 segments at once.
- `addon.pathtools.divideDashes {dash?, gap?, fit?, removeGaps?}` cuts the selected paths into separate open paths
  along the curve, each `dash` long (0.01 to 1,000,000 pt, 10 by default) with `gap` between them (0 to 1,000,000 pt,
  0 by default: the pieces touch), every piece keeping the path's appearance, as one undo step. The gap pieces are left
  out unless `removeGaps` is false. With direct-selected segments (or anchors) only those runs are divided, the
  pattern running on across segments that follow one another; the rest of such a path stays as pieces of its own.
  With `fit`, every run between corners and ends holds a whole number of dashes, all stretched or squeezed alike so it
  starts and ends on a dash (a closed path with no corner holds whole periods); otherwise the lengths are exact and the
  last piece of each run is what is left. The pieces are selected; more than 100,000 is refused. → `{ids, dashes}`.
  Divide into Dashes… opens a dialog (`ui.menuDialog {command}`, kind `divideDashes`) whose fields are these
  parameters, with Preview.
- `addon.pathtools.expandDashes {ids?}` turns the dashed stroke of each path and compound path under `ids` (default:
  the selection) into separate open paths, one per dash, as one undo step (Object › Plug-ins › Path Precision Tools ›
  Expand Dashes into Paths). The dashes are laid out exactly as the stroke draws them: its dash pattern (an odd one
  repeats with dashes and gaps swapped), its dash offset and Align Dashes to Corners and Path Ends, restarting on each
  subpath and running on through the start of a closed one; a dash running all the way round a closed path stays
  closed. Each dash keeps the path's appearance with the top stroke's dashes off, so the gaps become real gaps; with a
  round or projecting cap a dash of no length (a dot) becomes a path on its point, which draws the cap (under a butt
  cap it paints nothing, so it makes nothing). The stroke's arrowheads go, the line is first trimmed under them as
  drawn, and the path's fills stay on it, below its dashes (a path without fills goes). A compound path's dashes are
  plain paths with its appearance. Paths without a dashed stroke stay as they are, and it is an error if none has one.
  The dashes are selected; more than 100,000 (or a pattern too fine to draw) is refused. → `{ids, dashes}`.
- `addon.pathtools.cornerSet {ids?, anchors?, items?, radius, kind?, every?, minAngle?, maxAngle?}` cuts live corners, the ones Direct Selection's
  corner widgets and the Corners dialog edit, as one undo step. `ids` (default: the selection) are the paths and
  `anchors` the anchor indices of each path's outline with no corners cut; without `anchors`, a path's direct-selected
  anchors, or all of them. `items: [{id, anchors?}…]` gives each path its own instead. Only anchors where two sides meet
  at an angle take a corner: smooth anchors and open ends are skipped, and it is an error if none is left. `radius` is
  0 to 1,000,000 pt, drawn no larger than fits; `kind` is `round`, `invertedRound` or `chamfer` (default: each corner's
  own). `every` (`all`, `odd` or `even`; default `all`) keeps, of those anchors, only the odd- or even-numbered ones,
  counting each path's anchors from 1 (the 1st, 3rd, 5th… are odd): an anchor that takes no corner still counts, and it
  is an error if none is left. `minAngle` and `maxAngle` (degrees 0 to 180; default 0 and 180) keep only the corners
  whose sides meet at an angle in that range (small for a sharp spike, near 180 for a shallow bend); `minAngle` can't
  be more than `maxAngle`, and it is an error, saying so, if no chosen corner is in the range. Object › Plug-ins › Path Precision Tools › Set Corners… (also in the Anchor Inspector's
  Corners section) sets these with Preview. → `{ids, corners}`.
- `addon.pathtools.cornerRemove {ids?, anchors?, items?}` makes those corners sharp again, as one undo step; a plain path
  with none left is plain again. → `{ids, corners}`.
- `addon.pathtools.slideAnchor {id, subpath?, anchor, t}` moves an anchor along its path, the segments beside it refitted
  so the outline stays (the anchor count and index stay). `t`, -0.98 to 0.98, says where: positive, a curve parameter on
  the segment after the anchor; negative, back from the end of the one before. An anchor at a sharp corner loses it, as
  the corner isn't on the way. → `{id, strayed}`, how far the outline moved in points.
- `addon.pathtools.mergeEdge {a, b, at, diameter?, tolerance?, follow?, ends?}` looks within `diameter` (0.25 to 200 pt,
  default 20) of `at` ([x, y]) on paths `a` and `b`, two editable paths: where they touch at two anchors coincident within
  `tolerance` (0 to 3 pt, default 0.002), one each way along them, it replaces the stretches between with one edge;
  otherwise it makes an end there, and `ends: true` asks for that first. `follow` says which outline stays: `average`
  (default; half way between), `a` or `b`. Each path stays closed or open, and anchors either stretch has inside are on
  the merged edge. One undo step. → `{a, b, made}`, with `made` `edge`, `end`, or `nothing` when they are merged already
  (no undo step); it is an error if there is nothing to merge there.
- `addon.pathtools.movePointsTogether {enabled?}` sets Keep Shared Points Together (without `enabled`, turns it the other
  way). While it is on, anchors of any editable path coincident, within the Shared Edge Merge Tool's `tolerance`, with
  one that `addon.pathtools.slideAnchor`, `extend` or `setAnchor` moves go with it, in the same undo step; an Extend Path
  that adds an anchor leaves the old end, and what shares it, where they are. Nothing else in the app changes. It is the
  `together` option of tool `sharedEdge`, off by default. → `{enabled}`.
- `addon.pathtools.shape.create {center, kind?, width?, height?, rotation?, count?, a?, b?, radius?}` draws a parametric
  shape with the current paint, selected. `kind`: `rectangle`, `ellipse`, `polygon` (default), `star`, `ring`, `arrow`,
  `cross`, `heart`, `gear` or `bubble` (a speech bubble). `width` and `height` are 0.01 to 16,384 pt (100);
  `rotation` is in degrees clockwise (0: upright, a polygon's first corner up); `count` is a polygon's sides, a star's
  points or a gear's teeth (3 to 1,000); `radius` rounds the corners (0 to 16,384 pt; no corner passes the middle of a
  side). `a` and `b` are the kind's own values, fractions of its size: star, inner radius (0.01 to 1, 0.5); ring, the
  hole (0.01 to 0.99, 0.5); arrow, head length and shaft width (0.05 to 0.95, 0.4 each); cross, the bar's offset (−0.9
  to 0.9, 0; negative is up) and the arms' thickness (0.05 to 0.95, 1/3); heart, the notch's depth and the point's
  roundness (0 to 1; 0.4 and 0); gear, tooth height (0.01 to 0.9, 0.2); bubble, the tail's height (0.05 to 0.9, 0.25)
  and where its tip is across (−1 to 1, −0.4). → `{id, shape}`.
- `addon.pathtools.shape.set {id, …}` changes a parametric shape's numbers (the same keys; a new `kind` takes its own
  defaults for the values not given) and redraws it in place. With no change it makes no step. → `{id, shape}` (its
  numbers now, so `{id}` alone reads them).
- `addon.pathtools.shape.convert {ids?}` makes the selected plain closed paths that are rectangles, ellipses or
  polygons with equal sides and corners (stretched along their own axes at most) parametric shapes with the same
  outline. → `{ids}` (those converted).

A parametric shape is an ordinary path. It keeps its numbers, with a fingerprint of the outline they drew, in one
entry of its object data (`vc-pathtools-shape`, as `document.node` lists `attrs.data`), which SVG export leaves out;
copies and duplicates keep it. Moved, rotated or scaled with any tool it stays editable: its centre, size and rotation
follow (a shape with rounded corners only when scaled evenly). Skewed, mirrored, or with its anchors edited, it is a
plain path from then on.

```json
{"name":"run_command","arguments":{"command":"addon.pathtools.extend","params":{"id":12,"end":"end","length":40,"mode":"arc"}}}
{"name":"run_command","arguments":{"command":"addon.pathtools.erase","params":{"points":[[100,40],[100,160]],"diameter":30}}}
{"name":"run_command","arguments":{"command":"addon.pathtools.crossings","params":{"cut":true}}}
{"name":"run_command","arguments":{"command":"addon.pathtools.shape.create","params":{"kind":"star","center":[200,150],"width":120,"height":120,"count":7,"a":0.4}}}
{"name":"run_command","arguments":{"command":"addon.pathtools.shape.set","params":{"id":14,"kind":"gear","count":16,"radius":2}}}
```

### Path Effects (live)

Effect › Path Effects holds the add-on's live geometry effects, applied with `effect.apply {effect, params}` like any
other effect, with no shortcuts. Each draws, measures, exports and stacks as the other geometry effects do; the object's
own path stays as it is until Expand Appearance (`effect.expandAppearance`) writes the result. `effect.list` documents
every parameter. Out-of-range values are clamped, and an effect whose result would pass the geometry effects' budget
(2 million anchors) leaves the path as it is.

| Id | Menu item | Parameters |
| --- | --- | --- |
| `addon.path.dashes` | Dashes to Geometry… | `dash1`…`dash5`, `gap1`…`gap5` (pt 0 to 10000; 12 and 6, then 0), `offset` (pt), `alignCorners` (bool, false) |
| `addon.path.addAnchors` | Add Anchors… | `target` (`all`, `curved`, `straight`), `spacing` (`count`, `distance`), `count` (1 to 1000, 1), `distance` (pt, 20), `maxCount` (1 to 1000, 100) |
| `addon.path.extend` | Extend Path… | `startLength`, `endLength` (pt, 10; negative shortens), `startPercent`, `endPercent` (% of the path's length, 0), `startShape`, `endShape` (`straight`, `arc`) |
| `addon.path.close` | Close Path… | `mode` (`straight`, `smooth`) |
| `addon.path.smooth` | Smooth Anchors… | `ratio` (0.01 to 0.5, 0.3905) |
| `addon.path.retract` | Retract Handles… | `handles` (`all`, `shorter`), `length` (pt, 5) |
| `addon.path.reverse` | Reverse Direction | none |
| `addon.path.simplify` | Simplify Anchors… | `tolerance` (0 to 200, 10; each step lets the outline drift 0.02 pt), `protectCorners` (bool, true) |
| `addon.path.split` | Split at Anchors… | `anchors` (`corner`, `smooth`, `both`) |

Dashes to Geometry turns the path into open subpaths, one per dash; with `alignCorners`, every run between corners and
ends holds whole periods with a dash centred on each corner and end, and the offset doesn't apply. A dash of 0 becomes a
point that a round or projecting cap paints. Extend Path adds an end's length and percent together; closed subpaths stay
as they are, and so does an end that would be shortened to nothing. An arc follows the end's curvature for at most half
a circle. Split at Anchors tells smooth anchors by their handles standing in line. Simplify Anchors leaves a path of more
than 50000 anchors as it is.

```json
{"id": 1, "method": "engine.execute", "params": {"command": "effect.apply", "params": {"effect": "addon.path.dashes", "params": {"dash1": 8, "gap1": 4, "dash2": 2, "gap2": 4, "alignCorners": true}}}}
```

## Smart Pen add-on

Smart Pen is a built-in add-on: one tool that draws paths and edits their points, handles and segments without
switching tools. Its commands are `addon.pen.*`; in the desktop app it lives in Object › Plug-ins › Smart Pen, in its
panel (`window.panel {panel: "smartPen"}`) and in the Smart Pen Tool (`smartPen`), which the toolbars don't show until
it is added from the All Tools drawer (or chosen with `tool.select`); it has no shortcut. Each gesture and each command is
one undo step; an edit that changes nothing makes no step. The standard Pen tool is unchanged.

The tool (`smartPen`), option `next`: `"straight"` (default) or `"curved"`, the segment a click adds:

- Drawing: a click adds the next anchor with a straight segment into it (the previous anchor's handle on it retracted)
  or a curved one (the previous anchor gets a handle if it has none: in line with its other handle, or, after a straight
  segment, along it, so the curve leaves tangent). A drag makes a smooth anchor; Alt breaks the outgoing handle off,
  Shift keeps it to 45° steps, Space moves the anchor. Tab switches the segment into the anchor being placed, or the
  last segment drawn, between straight and curved. A rubber band shows the next segment. A press on the first anchor
  closes the path; on an end of another open path it joins the two (`path.join`); with no path being drawn, a click on
  an end of an open path draws on from it. Enter or Esc ends the path.
- Points of the shown (selected) paths: a click selects one (`select.anchors`; Shift adds or removes it), and on an end
  of an open path draws on from there; a drag moves it (`path.moveAnchor`, Shift: 45°); Alt-drag pulls out its outgoing
  handle (`path.setHandle`); Alt-click removes it keeping the shape and Shift+Alt-click deletes it with the handles
  beside it left as they are (`addon.pen.removePoint`); a double-click toggles it between smooth and corner
  (`addon.pen.togglePoint`).
- Handles of selected anchors drag (`path.setHandle`; Shift: 45°, Alt: the handle keeps its angle); a double-click on
  one toggles its anchor.
- Segments drag to reshape (`addon.pen.reshape`; Shift: `symmetric`, Alt: `keepAngles`); Alt-click adds an anchor
  (`path.insertAnchor`); a click on a path that isn't selected selects it.
- With no path being drawn, an Alt-drag away from paths draws a marquee that selects the points of the editable paths
  inside it (`select.anchorsMany`; with Shift, toggles them).

Commands:

- `addon.pen.append {id, subpath?, x, y, in?, out?, segment?}` adds an anchor at the end of an open subpath (by default
  the last open one) and selects it. `segment`: `"straight"` (the previous anchor's handle on the new segment and the
  new anchor's incoming handle retracted) or `"curved"` (the previous anchor gets a handle on it as a click does);
  without it the anchor takes the handles given and the previous anchor stays as it is.
- `addon.pen.close {id, subpath?, segment?}` closes an open subpath of two anchors or more, the closing segment
  straight, curved or (without `segment`) with the handles its ends have.
- `addon.pen.segment {id, subpath?, segment, curved}` makes one segment straight or curved → `{changed}`.
- `addon.pen.removePoint {id, subpath?, anchor, plain?}` removes one anchor: the handles beside it are refitted so the
  outline keeps its shape (an open end takes its segment with it), or with `plain` left as they are. A subpath keeps two
  anchors (three when closed).
- `addon.pen.togglePoint {id, subpath?, anchor, ratio?}`: a smooth anchor becomes a corner with its handles where they
  are; any other is made smooth by the auto-smooth rule (`ratio` 0.1 to 0.5, 0.3905) → `{smooth}`.
- `addon.pen.reshape {id, subpath?, segment, t?, dx, dy, mode?}` bends a segment so its point at `t` (0 to 1, held to
  0.05…0.95) moves by `(dx, dy)`: `free` (default; both handles move, the one nearer the grab more), `keepAngles` (each
  handle only changes length; a segment whose two directions are parallel bends freely) or `symmetric` (both handles
  at right angles to the chord at one length). Missing handles grow out of their anchors; a smooth anchor stays smooth.
- `addon.pen.nextSegment {curved}` sets the tool's `next` option → `{next}`.
- `addon.pen.closeJoin {}`: with exactly two open path ends direct-selected, joins them (as `path.join`); otherwise
  closes the selected open paths keeping their ends' handles (as `addon.pathtools.close`).

The panel sets the next segment and runs Remove Points Keep Shape (`addon.pathtools.removeKeepShape`), Make Smooth
(`addon.pathtools.smooth`), Make Corner (`addon.pathtools.corner {retract: false}`: nothing moves), Retract Handles
(`addon.pathtools.corner {retract: true}`) and Close or Join (`addon.pen.closeJoin`).

```json
{"name":"run_command","arguments":{"command":"addon.pen.nextSegment","params":{"curved":true}}}
{"name":"run_command","arguments":{"command":"addon.pen.append","params":{"id":12,"x":200,"y":150,"segment":"curved"}}}
{"name":"run_command","arguments":{"command":"addon.pen.reshape","params":{"id":12,"segment":0,"t":0.3,"dx":0,"dy":-25}}}
```

## Contour add-on

Contour is a built-in add-on: it grows or shrinks a copy of a path or shape by a distance, live. Its command is
`addon.contour.set`; in the desktop app it lives in Object › Plug-ins › Contour and in the Contour Tool (`contour`), which
the toolbars don't show until it is added from the All Tools drawer (or chosen with `tool.select`); it has no shortcut. A
floating bar over the selection (the distance, Sharp / Round / Square and Bake) acts through the same commands.

`addon.contour.set {offset, corners?, miterLimit?}` puts a copy of each selected path, shape or compound path directly above
it with a live Offset Path effect of `offset` points (negative insets), and selects the copies. `corners` is `"sharp"`
(default), `"round"` or `"square"` (corners cut off flat), `miterLimit` 1 to 500 (4; sharp corners). A selected object that
already has that effect (a Contour made earlier) has its effect changed instead of getting another, so the same call
adjusts a Contour. The original is kept, and nothing is baked: `effect.expandAppearance` turns a Contour into plain paths.
Type, images and groups are left alone; with nothing to offset it says "select paths or shapes". One undo step. Result:
`{ids, created, updated}`.

The tool (`contour`) drags a distance from the selected shape's edge, outward to grow and inward to shrink: the press
changes nothing until the pointer moves, a Contour that already stands at a distance continues from it, Shift rounds to
whole points and Escape cancels. A drag keeps a Contour's own corners and miter limit; a new Contour takes the tool's
options `corners` and `miterLimit` (`tool.setOption`). A border with a gap is two Contours of one shape (the gap first, so
it ends up in front), baked with `effect.expandAppearance`, then `object.pathfinder.minusFront`.

## Color Effects add-on

Color Effects is a built-in add-on: its effects are colour adjustments in Effect › Color Effects, its command is
`addon.color.filter`, and in the desktop app it also lives in Object › Plug-ins › Color Effects and in its panel
(`window.panel {panel: "colorEffects"}`).

Its live effects go on through `effect.apply` like any other and recolour what the object paints, in stack order with
the other colour adjustments (Effect › Color Adjustments); each colour keeps its colour model:

| Effect | Parameters | Does |
|---|---|---|
| `addon.color.exposure` (Exposure…) | `exposure` stops −20…20 (0), `offset` −0.5…0.5 (0), `gamma` 0.1…10 (1) | Scales the light in linear light (each stop doubles or halves it, mostly in the highlights), adds the offset (shadows and midtones) and applies the gamma |
| `addon.color.desaturate` (Desaturate) | none | Turns every colour to the grey of its lightness |
| `addon.color.duotone` (Duotone…) | `tones`, `skipSpot` (below) | Prints each colour's tone with one to four inks |
| `addon.color.invert` (Invert Colors) | none | Turns every colour to its RGB inverse |

`addon.color.filter {effect, params?, fill?, stroke?, includeImages?, includePatterns?, ids?}` applies any colour
adjustment, the add-on's or one of Effect › Color Adjustments', to the colours themselves instead of as a live effect:
fills, strokes, type, gradient meshes, embedded images and pattern tiles, as `edit.colors.invert` reaches them, as one
undo step. Missing `params` take the effect's dialog defaults; the objects' own effects stay as they are.

```json
{"name":"run_command","arguments":{"command":"effect.apply","params":{"effect":"addon.color.exposure","params":{"exposure":1.5}}}}
{"name":"run_command","arguments":{"command":"addon.color.filter","params":{"effect":"adjust.hueSaturation","params":{"hue":120}}}}
```

The panel picks an adjustment (Brightness/Contrast, Hue/Saturation, Levels, Temperature/Tint, Exposure, Desaturate or
Invert Colors), sets its values, and then Add as Effect (`effect.apply`) or Apply as Filter (`addon.color.filter`).

`addon.color.select {kind}` (Object › Plug-ins › Color Effects › Select by Color Kind) selects, among the objects Select
All reaches and the objects inside groups (locked and hidden ones left out), those of one kind: `spotColors` (painted
with a spot colour swatch, solid or in a gradient's stops), `colorEffects` (carrying a visible colour adjustment effect,
on the object or on one of its fills or strokes; a group carrying one is selected whole), `rgbImages`, `cmykImages` or
`grayImages` (images whose pixels are RGB, CMYK, or grayscale or 1-bit, read from the image's header). It returns the
`count` selected.

### Selection Tools

Selection Tools is a built-in add-on: its commands are `addon.select.*`, and in the desktop app it lives in Object ›
Plug-ins › Selection Tools.

`addon.select.byArtboards {artboards, relation?}` (Select by Artboards…, #5525) selects objects by where they sit
relative to one or more artboards. `artboards` lists 0-based artboard indices (at least one; one listed twice counts
once). `relation` is `overlap` (the default: on at least one of them, as `select.allOnArtboard` takes it, the bounds
overlapping the artboard or the centre on it), `inside` (the bounds wholly within one of them, edges included; an object
straddling two touching artboards is inside neither) or `outside` (on none of them). It looks at the objects Select All
takes, or the isolated container's in isolation mode, so hidden and locked objects, objects on hidden, locked or template
layers and art held by a locked artboard are left out. Bounds are geometric, a symbol instance's measured by its art. It
returns the `count` selected. An empty list, an index with no artboard, a value that isn't an index, or another
relation is an error, and the selection is left as it was.

```json
{"name":"run_command","arguments":{"command":"addon.select.byArtboards","params":{"artboards":[2,6,11],"relation":"inside"}}}
```

The dialog lists every artboard with its number, name and size in the document's units. Search filters the list by
name; a click chooses one artboard, Shift-click adds a range and Cmd/Ctrl-click toggles one, and Select All / None act
on the rows shown (rows hidden by the search stay chosen). It opens with the active artboard chosen and Overlapping.

`addon.select.byLinkStatus {status, ids?, add?, zoom?}` (Select Linked / Embedded / Missing / Modified and Select by
Link Status…, #5526) selects the image objects and placed documents whose link has a status, as `links.list` reports
it: `linked` (its file found unchanged; `links.list` says `ok`), `embedded`, `missing` or `modified` (its file changed
since it was read). `status` is one of them or a list of them (at least one; one listed twice counts once). `ids` picks
only among those objects (other objects among them are passed over); `add: true` adds them to the selection instead of
replacing it; `zoom: true` asks for Zoom to Selection afterwards, which the desktop app runs (`addon.arrange.zoomSelection`)
when something was selected, and which other callers get back as `zoom: true`. Objects that are hidden or locked, inside
a hidden or locked object or layer, on a template layer, on a locked artboard while nothing is isolated, or outside the
isolated group or layer are left out and listed with the reason. Only the selection changes, so there is nothing to
undo. It returns `{count, selected: [ids] (bottom first), skipped: [{id, reason: hidden|locked|template|isolation}]}`.
A missing or unknown status, an id naming no object, or a value of the wrong type is an error, and the selection is left
as it was.

```json
{"name":"run_command","arguments":{"command":"addon.select.byLinkStatus","params":{"status":["missing","modified"],"zoom":true}}}
```

In the desktop app, Object › Plug-ins › Selection Tools and the Links panel menu have Select Linked, Select Embedded,
Select Missing and Select Modified, each off when no image has that status; with Shift held they add to the selection.
The status bar says what was selected and what was left out, e.g. "Selected 9 missing images; 3 more are hidden or
locked". Select by Link Status… lists every image and placed document with its name, status and format, searches by
name and filters by status (Show), chooses rows with checkboxes (All / None act on the rows shown), and on Select
selects the rows chosen, zooming to them when Zoom to Selection is on (the default).

### Vector Halftone (live)

`addon.color.halftone` (Effect › Color Effects › Vector Halftone…, also in Object › Plug-ins › Color Effects) draws its
object as vector halftone dots sized by the object's tone, the dots `object.vectorHalftone` makes, and keeps them in step
as the art or the settings change. It goes on through `effect.apply` like any other effect; its dialog has the fields of
Object › Vector Halftone without Keep Original, and the Appearance panel reopens it.

| Parameter | Values (default) | Does |
|---|---|---|
| `shape` | `circle`, `ellipse`, `square`, `diamond`, `line` (`circle`) | The dot shape; lines join along a row |
| `frequency` | lines per inch 1…300 (20) | Cells per inch of the screen |
| `angle` | degrees (45) | The screen angle; CMYK: the black screen's, cyan −30°, magenta +30°, yellow −45° from it |
| `mode` | `mono`, `cmyk` (`mono`) | One screen following lightness, or four ink screens following the art's ink amounts, multiplied over each other |
| `color` | `#rrggbb` (`#000000`) | The mono dot colour |
| `fromArt` | bool (false) | Mono: each dot takes the art's colour under it (one path per colour) |
| `invert` | bool (false) | Dots for light tones |
| `clip` | bool (true) | The dots clipped to the art's outline |

The object is sampled as it draws with its other effects (its own transparency and opacity mask apply to the dots).
Each screen takes at most 100000 cells: past that, or for art without area, the object draws as it is. The dots are kept
while the object and what it reads from the document (symbols, patterns, fonts, colour settings) stay the same, so
redrawing, measuring or exporting unchanged art doesn't sample it again. Effect › Expand Appearance turns the object
into the dots, a group of one path per ink (per colour with `fromArt`), and the SVG, PDF, EPS and AI exporters write
those paths.

```json
{"name":"run_command","arguments":{"command":"effect.apply","params":{"effect":"addon.color.halftone","params":{"frequency":24,"shape":"diamond","angle":30}}}}
```

Not yet: stochastic (FM) and radial patterns, a dot-gain curve, per-screen separation settings, and type or symbols as
dots.

### Duotone (live)

`addon.color.duotone` (Effect › Color Effects › Duotone…, also in Object › Plug-ins › Color Effects and the panel's Inks)
prints the grey-equivalent tone of each colour the object paints with one to four inks: a monotone, duotone, tritone or
quadtone. It is a colour adjustment like the add-on's others, so it goes on through `effect.apply`, stacks with the
other adjustments and works as a filter through `addon.color.filter {effect: "addon.color.duotone", params}`; Expand
Appearance gives the same colours as the filter.

| Parameter | Values (default) | Does |
|---|---|---|
| `tones` | up to four `{color, swatch?, curve?, active?}` (one black tone) | The inks; tones past the fourth are left out, and one not `active` (true) is skipped |
| `tones[].color` | `#rrggbb`, `[r, g, b]` or a colour object (`#000000`) | The ink's colour; with a `swatch`, that swatch's colour (the dialog fills it in) |
| `tones[].swatch` | a global or spot swatch's name (none) | The swatch the ink is |
| `tones[].curve` | `"tint,ink tint,ink …"` or `[[tint, ink], …]`, 0…100, at most 64 points (`"0,0 100,100"`) | The tone's tint to the ink's tint, a smooth curve through the points, flat past its ends |
| `skipSpot` | bool (false) | Colours linked to a spot swatch keep their colour |

The tone of a colour is its grey equivalent (a grey's ink; for other colours one minus their luma). With one tone of a
swatch the document has, the result is a tint of that swatch, linked to it, so a spot tone separates onto its own plate.
With several tones the inks overprint: the result is the CMYK mix of their tints, each ink multiplying what the others
leave of the paper. A colour links to one swatch at one tint, so a mix of several spot inks becomes process colour.
Gradient stops, type, gradient meshes and embedded images are recoloured as by the other adjustments.

```json
{"name":"run_command","arguments":{"command":"effect.apply","params":{"effect":"addon.color.duotone","params":{"tones":[{"color":"#000000"},{"color":{"model":"cmyk","c":0,"m":0.5,"y":1,"k":0},"swatch":"Orange","curve":"0,0 50,70 100,100"}]}}}}
```

The dialog has a row per tone (on or off, a colour chip, the swatch or Process Color with its hex value), the curve of the
tone picked under Curve (the graph, its points and Reset to Linear or Reset to Inverse) and Skip Objects with Spot Colors.

Not yet: an opaque undercoat, tones for type only, extra gradient stops for smoother tints, and buttons that flip a
curve or set it to 100 %.

### Print Ink Fixes

`addon.color.printFixes {scope?, inkLimit?, excludePureBlack?, removeWhiteOverprint?, overprintBlack?, textOnly?, fill?,
stroke?, ids?}` (Object › Plug-ins › Color Effects › Print Ink Fixes…, also under the panel's Inks) prepares a CMYK
document's inks for print, as one undo step:

| Parameter | Values (default) | Does |
|---|---|---|
| `scope` | `selection`, `document` (the selection, or the document while nothing is selected) | What it fixes; `ids` stand for the selection |
| `inkLimit` | total ink % 1…400, 0 off (off) | CMYK colours (solid, gradient stops, gradient meshes) with more ink than this are scaled down to it; colours of spot swatches and Registration stay |
| `excludePureBlack` | bool (true) | Black-only CMYK and grey colours keep their ink |
| `removeWhiteOverprint` | bool (false) | White fills and strokes, and characters', stop overprinting |
| `overprintBlack` | black % 1…100, 0 off (off) | Black-only and grey fills and strokes this dark or darker overprint, as `edit.colors.overprintBlack` sets it (not spot-linked or rich blacks) |
| `textOnly` | bool (false) | Overprint Black on type only |
| `fill`, `stroke` | bool (true) | Which paints the fixes reach |

It returns `changed`, `inkLimited` (paints and meshes scaled), `whiteOverprintRemoved` and `blackOverprinted` (objects);
nothing changed is no undo step. It needs a CMYK document (File › Document Color Mode › CMYK Color) and refuses an RGB
one, out of range values and a request with no fix on.

```json
{"name":"run_command","arguments":{"command":"addon.color.printFixes","params":{"scope":"document","inkLimit":300,"removeWhiteOverprint":true,"overprintBlack":100}}}
```

### Find Nearest Swatch

`addon.color.nearestSwatch {library?, color?, fill?, stroke?, count?, ids?}` (a query: no undo step) lists, for each
colour of the selected objects, the colour swatches of a library closest to it, closest first, by CIEDE2000 difference
(`deltaE`: under 1 hardly tells apart, about 2 to 3 close) (#4923). `library` is a swatch library's id or name, as
`swatch.library.list` lists them: the built-in ones, and the libraries and colour books (`.acb`, `.ase`, `.gpl`) the
user saved or loaded with `swatch.library.load`; `"document"` (the default) is the document's own swatches. The colours
are the solid colours and gradient stops of the selection's fills (`fill`, true) and strokes (`stroke`, true), of the
objects inside it and of its type, once each, the first 100; `color` looks one colour up instead. `count` is 1…50 (5).
Tint swatches, None and Registration aren't matched. → `{library, colors: [{values, hex, matches: [{name, values, hex,
spot, deltaE}]}]}` (`values` as a default swatch name reads them: `C=10 M=20 Y=30 K=0`, `R=255 G=128 B=0`…)

`addon.color.applyNearestSwatch {library?, fill?, stroke?, ids?}` (Object › Plug-ins › Color Effects › Find Nearest
Swatch…, whose dialog lists each colour's three closest swatches with their difference; Apply runs it) gives each solid
colour and gradient stop of the selected objects' fills and strokes (and gradient meshes) the colour of its nearest
swatch, as one undo step. A global or spot swatch's colour links to it. The library's swatches the colours take are
added to the document's first, as `swatch.library.add` adds them, so a spot colour from a colour book stays spot;
images and patterns keep their colours. → `{changed, swatches: [the document swatches the colours took], added}`

```json
{"name":"run_command","arguments":{"command":"addon.color.nearestSwatch","params":{"library":"web-safe-216","color":{"c":10,"m":80,"y":90,"k":0},"count":3}}}
{"name":"run_command","arguments":{"command":"addon.color.applyNearestSwatch","params":{"library":"document","stroke":false}}}
```

## Arrange Tools add-on

Arrange Tools is a built-in add-on that lays out, swaps, replaces and sizes selected objects: its commands are
`addon.arrange.*`, and in the desktop app it lives in Object › Plug-ins › Arrange Tools. Each command acts on the
top-level selected objects and is one undo step; a run that moves nothing makes no step. Places and sizes are measured
as `object.setBounds` measures them (Use Preview Bounds), except Arrange in Grid, which uses visual bounds.

| Command | Parameters (default) | Does |
|---|---|---|
| `addon.arrange.grid` | `layout`: `grid`, `row`, `column` (`grid`); `columns`: 1…10000 (⌈√count⌉, never more than the count); `gapX` pt (10), `gapY` pt (`gapX`), each −1000000…1000000; `centerOnArtboard` (false) | Lays the objects out in their stacking order from the back, left to right and top to bottom. Each cell is as wide as the widest object in its column and as tall as the tallest in its row, and each object is centred in its cell. The grid starts at the selection's top-left, or is centred on the artboard under the selection's centre (the first artboard when none is there). → `{count}` |
| `addon.arrange.swap` | `size` (false); `scaleStrokes` (the Scale Strokes & Effects preference) | Swaps the bounds centres of exactly two selected objects (off otherwise); with `size`, each also takes the other's width and height. |
| `addon.arrange.replace` | `fit`: `keep`, `bounds` (the replaced object's width and height), `proportional` (fits inside them) (`keep`); `scaleStrokes`; `keep` (false) | Puts a copy of the topmost selected object centred on each of the other selected objects, in that object's place in the stacking order; the replaced objects are deleted unless `keep` (the copy then goes right above each). Needs two objects or more; the copies are selected. → `{ids}` |
| `addon.arrange.size` | `width`, `height` pt, 0.001…1000000 (at least one); `proportional` (false: a side not given stays; true: it follows the given one in proportion); `reference` 0…8 (4, the centre); `scaleStrokes` | Gives every selected object that width and height, keeping its reference point in place (the 9-point grid row by row from the top-left). |
| `addon.arrange.groupOverlapping` | none | Groups the selected objects (at most 20000) wherever their visual bounds overlap or touch, chains included: when A overlaps B and B overlaps C, all three go in one group. Only objects in the same layer or group are grouped together; objects that overlap nothing stay as they are. Each new group goes where its frontmost member was in the stacking order, and the groups and the objects left alone stay selected; with nothing overlapping it makes no step. Needs two objects or more. → `{groups: [id…], grouped}` (how many objects went into groups) |
| `addon.arrange.randomizeOpacity` | `min`, `max` % 0…100 (30, 100; `min` not more than `max`); `seed` whole number (0); `ids` (the selection) | Gives each selected object its own opacity (the object opacity the Transparency panel sets), picked at random and evenly from `min` to `max`; the same seed, objects and range give the same opacities. Groups take one opacity of their own; the objects inside keep theirs. → `{count}` |
| `addon.arrange.sort` | `by`, `apply`: `width`, `height`, `area` (width × height), `x` (left edge), `y` (top edge), `opacity` (%), `stacking` (stacking order from the back) (`area`, `stacking`); `order`: `ascending` (small to large), `descending` (large to small), `random` (`ascending`); `distribute` (false); `seed` whole number (0, for `random`); `scaleStrokes`; `ids` (the selection) | Sorts the selected objects (at most 20000) by `by` and gives them the values of `apply` in that order: the first gets the smallest value. Without `distribute` the objects trade the values they have; with it the values step evenly from the smallest to the largest. A width, height or area changes about each object's centre (an area in proportion), a place moves the top-left corner. Stacking order always trades places, and only among objects in the same layer or group. Needs two objects or more. → `{count}` |

Zoom to Selection is the UI command `addon.arrange.zoomSelection {}` (remote mode only): it zooms and centres the
active view so the selection's visual bounds fill the window with a margin, as `view.fitArtboard` fits the artboard,
and fails with nothing selected. Arrange in Grid…, Replace with Top Object…, Set Size of Each…, Randomize Opacity… and Sort Objects… open dialogs
(`ui.menuDialog {command}`, dialog kinds `arrangeGrid`, `arrangeReplace`, `arrangeSize`, `arrangeOpacity` and `arrangeSort`) whose
fields are the command's parameters, with Preview (Randomize Opacity's seed, and Sort Objects' `seed`, which is not shown, start new whenever the document has changed); Set Size of Each's `__width` and `__height` say which sizes OK applies.

```json
{"name":"run_command","arguments":{"command":"addon.arrange.grid","params":{"columns":4,"gapX":12,"gapY":12}}}
{"name":"run_command","arguments":{"command":"addon.arrange.replace","params":{"fit":"proportional"}}}
{"name":"run_command","arguments":{"command":"addon.arrange.size","params":{"width":72,"proportional":true}}}
```

## Document Tools add-on

Document Tools is a built-in add-on for whole-document chores: its commands are `addon.document.*`, and in the desktop
app it lives in Object › Plug-ins › Document Tools.

`addon.document.saveTimestampedCopy {folder?, format?, time?}` writes a copy of the document named
`<name>_YYYY-MM-DD_HH-MM.<ext>`, with `-2`, `-3`… added when that name is taken. The date and time are `time` (Unix
seconds; now by default) in UTC. The copy goes in the document's own folder, or in `folder`, which a document never
saved needs (the desktop app asks for one with the folder picker); `format` is a save format (`vectorcraft`, `pdf`,
`svg`, `svgz`, `ai`, `eps`; the document's own by default). It is written as `file.saveCopy` writes, inside the
automation roots in force: the document keeps its path, title and modified state, and no undo step is made. The
command is off where there is no clock (the web). → `{path, format, bytes, warnings}`

```json
{"name":"run_command","arguments":{"command":"addon.document.saveTimestampedCopy","params":{}}}
{"name":"run_command","arguments":{"command":"addon.document.saveTimestampedCopy","params":{"folder":"/Users/me/Versions","format":"pdf"}}}
```

`addon.document.exportAllOpen {folder, format?}` writes a copy of every open document, in tab order, to `folder`,
each named after its document (`<name>.<ext>`, with `-2`, `-3`… added when that name is taken there or by another
copy, so no file is replaced). `format` is a save format (`pdf` by default; `svg`, `svgz`, `vectorcraft`, `ai`,
`eps`). Each copy is written as `file.saveCopy` writes it, inside the automation roots in force: every document keeps
its path, title and modified state, no undo step is made, and the active document stays active. A document that
can't be written is listed in `failed` and the others are still written (#4920). The desktop app's Export All Open
Documents as PDF and as SVG ask for the folder. → `{files: [{title, path, warnings}], failed: [{title, error}], format}`

```json
{"name":"run_command","arguments":{"command":"addon.document.exportAllOpen","params":{"folder":"/Users/me/Out","format":"svg"}}}
```

`addon.document.batchTraceFolder {folder, destinationFolder, preset?, expand?, format?, includeSubdirs?}` opens each
image in `folder` (PNG, JPEG, TIFF, BMP, GIF, WebP, PSD; with `includeSubdirs`, in its subfolders too), traces it with
`preset` (an `imageTrace.presets` name; Default by default; with `expand`, as Make and Expand), saves it into
`destinationFolder` as `<name>.<ext>` in `format` (`vectorcraft` by default; `svg`, `pdf`, `ai`, `eps`), with `-2`,
`-3`… added when the name is taken so no file is replaced, and closes it again whether it traced or not; the active
document stays active. Images that can't be traced are listed in `failed` and the others are still traced. Optional
`format` and `preset` accept strings; `expand` and `includeSubdirs` accept true or false. Null and values of another
type return an error before any image is opened. Up to 500 images and 8 folder levels (a folder with more than 1,000
entries is refused); not on the web. The desktop app's Batch
Trace Folder asks for both folders (#4921). → `{traced: [{path, output}], failed: [{path, error}], format}`

```json
{"name":"run_command","arguments":{"command":"addon.document.batchTraceFolder","params":{"folder":"/Users/me/Scans","destinationFolder":"/Users/me/Traced","preset":"Black and White Logo","format":"svg"}}}
```

`addon.document.symbolsFromLayers {layers?, replace?}` makes one symbol of each top-level layer's art, as `symbol.new`
makes one (registration in the centre), named after the layer with a number added when a symbol has that name, as
one undo step. `layers` lists layer ids (top-level layers only; anything else is refused); by default it takes the
layers highlighted in the Layers panel when two or more are, else every layer. With `replace` (Convert Layers to
Symbols in the menu) each layer's art is replaced by an instance of its symbol at the same place and size, the
instances are selected, and locked layers are skipped. Layers with no art are skipped too; with nothing to make
there is no step. → `{symbols: [name…], ids: [instance id…], skipped: [{layer, name, reason: "empty" | "locked"}…]}`

```json
{"name":"run_command","arguments":{"command":"addon.document.symbolsFromLayers","params":{"replace":true}}}
```

## Type Tools add-on

Type Tools is a built-in add-on for type: its commands are `addon.type.textTweaker` and `addon.type.releasePath`, and
in the desktop app it lives in Object › Plug-ins › Type Tools (#4924, #5532). Text Tweaker gives one character attribute to the characters letter by letter,
from a start value to an end value along a shape, as one undo step; the text stays live and editable. Each value is set
as `text.setRangeStyle` sets it (the Character panel), in its units and ranges ([formatting type](https://helpx.adobe.com/illustrator/using/formatting-type.html)), so on scaled
type it is the size shown.
It changes the range of `id` (`start`/`end` bytes), else all the text of the type objects in `ids`, else the text the
Type tool has selected (at a caret: all of that object's text), else all the text of every selected type object; each
object gets its own ramp, and at most 100000 characters change in one run. A run that changes nothing makes no step.
A `start` or `end` byte inside a character rounds down to that character's start.

| Parameter | Values (default) |
|---|---|
| `attribute` | `size` (pt 0.1…1296), `baselineShift` (pt −1296…1296), `rotation` (° −360…360), `hScale`, `vScale` (% 1…10000), `tracking` (1/1000 em −1000…10000), `leading` (pt 0.1…5000), `strokeWidth` (pt 0…1000; shows only on characters with a stroke) (`size`) |
| `from`, `to` | the first and last character's values, in the attribute's units and range (size 12 → 36, baselineShift 0 → 12, rotation 0 → 30, hScale and vScale 100 → 150, tracking 0 → 200, leading 12 → 36, strokeWidth 0 → 2) |
| `shape` | `straight` (an even ramp), `ease` (slow at both ends), `bow` (`from` at both ends, rising smoothly to `to` in the middle), `spike` (the same, in straight lines), `random` (each character anywhere from `from` to `to`) (`straight`) |
| `seed` | whole number (0): `random` with the same seed and text gives the same values |
| `skipSpaces` | true: spaces and line breaks keep their values and don't count |

→ `{count (characters changed), ids}`. Text Tweaker… opens a dialog (`ui.menuDialog {command}`, dialog kind
`textTweaker`) whose fields are the command's parameters, with Preview; another attribute starts from its own
default values, and the seed starts new whenever the document has changed.

```json
{"name":"run_command","arguments":{"command":"addon.type.textTweaker","params":{"attribute":"baselineShift","from":0,"to":18,"shape":"bow"}}}
{"name":"run_command","arguments":{"command":"addon.type.textTweaker","params":{"attribute":"rotation","from":-15,"to":15,"shape":"random","seed":4}}}
```

### Release Type on a Path

`addon.type.releasePath` splits each type on a path (`text.createInPath` with `mode: "onPath"`) into point type that
holds the same text and the plain path it was set on (#5532). The reference app has no command for this, so it is a
VectorCraft addition and changes no parity score.

- **The path** keeps its exact shape (open or closed, every anchor and handle, the object's transform) and goes in the
  type's layer at its place in the stacking order. Type paths carry no paint of their own (making type on a path drops
  the path's fill and stroke), so the released path has no fill and no stroke.
- **The text** keeps its runs with their character attributes and its paragraph attributes, and becomes point type on
  one straight line along the object's own axis, with the baseline starting where the first character sat on the path
  (type with no characters starts at its start bracket). Type on a Path Options (effect, align to path, spacing, flip,
  brackets) don't apply to point type and are dropped. Without `keepOriginal` the type object itself becomes the point
  type, keeping its id, so a frame of threaded text leaves its thread with the text it holds, as
  `type.convertToPointType` does for area type.
- It acts on the type on a path among `ids` (or `id`, else the selection), groups included; other objects are left
  alone. With none among them nothing changes and no undo step is made. Otherwise the whole run is one undo step and
  the released objects are selected.

| Parameter | Values (default) |
|---|---|
| `mode` | `both` (point type and the path), `path` (the path only: the type goes), `text` (point type only) (`both`) |
| `keepOriginal` | false; true: the type on a path stays as it is and the released objects go just above it |
| `ids`, `id` | the objects to release (default: the selection) |

→ `{ids (the released objects, bottom to top), paths, texts}`. Release Type on a Path… opens the command dialog
(`ui.paramDialog`) on Mode (Text and Path, Path Only, Text Only) and Keep Original, with Preview: while Preview is on
the canvas shows the release, Cancel restores the original with no undo step, and OK applies it as one, as Type on a
Path Options' Preview does (#5360).

```json
{"name":"run_command","arguments":{"command":"addon.type.releasePath","params":{}}}
{"name":"run_command","arguments":{"command":"addon.type.releasePath","params":{"mode":"path","keepOriginal":true}}}
```
