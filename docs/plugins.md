# Plug-ins: the WebAssembly plug-in API (ABI v1)

VectorCraft does not host native plug-ins built for other illustration apps: that needs unsafe FFI into
arbitrary machine code and cannot work in the browser build. Instead, plug-ins are **WebAssembly modules**
run in a sandbox. One `.wasm` file works on macOS, Windows, Linux and the web. The design follows PhotoCraft's
plug-in API (same sandbox, same manifest and parameter schema), extended for vector documents.

There are four kinds of plug-in:

- **Object filters** read the selected paths and compound paths (geometry, fills, strokes, gradients, opacity)
  and return the objects to keep, change or add. A run is one undo step. Use them for recolouring, path
  clean-up, halftones, generative patterns and the like.
- **Live effects** rewrite one object's geometry each time it is drawn, from its appearance stack, like the
  built-in Distort & Transform effects. The art stays editable: move or reshape it and the effect follows. A live
  effect may also paint what it returns ([painted live effects](#painted-live-effects)).
- **Exporters** write the whole document in a file format of their own, for pen plotters, cutting machines,
  laser cutters and the like. File › Export › Export As lists them after the built-in formats, and the host writes the
  bytes they return: a plug-in still never touches a file.
- **Importers** read files in a format of their own, such as a plotter's or a cutting machine's, into a new document of
  paths and compound paths. File › Open lists their files, File › Open With lists the importers by name, and the host
  hands them the file's bytes.

This page is the contract for plug-in authors and the reference for the host (`crates/plugins`, the
`plugin.*` commands in `crates/engine/src/cmd/plugin.rs`).

## Using plug-ins

| Command | Params | What it does |
|---|---|---|
| `plugin.list` | `{}` | `{"plugins": [{id, name, version, kind, author, description, params, paramsSchema, effect?, format?, extensions?, budget?, commands?, size, source, api, paintOutput?, permissions, hash, cacheBytes}], "folder": report}` (`api` is the manifest's [plug-in API level](#permissions); `budget: "heavy"` for a [heavy filter](#heavy-filters); `commands` lists its [commands](#commands); `paintOutput: true` marks a [painted live effect](#painted-live-effects); `permissions` lists what it [may do](#permissions), `hash` is the module's SHA-256; `cacheBytes` is the size of its [cache](#settings-and-cache)) |
| `plugin.info` | `{"id"}` | One plug-in: the `plugin.list` entry plus `manifest`, `defaults` and `lastError` (a live effect's last failure). |
| `plugin.run` | `{"id", "params"?: {…}, "ids"?: [...]}` (plug-in params may also be top-level keys) | Runs an object filter on the selection (or `ids`; an `ids` that is not a list of object ids is an error). One undo step; the result is selected, except the objects it locked or made guides. |
| `plugin.install` | `{"path": "/x/y.wasm"}` (desktop) or `{"dataBase64": "…", "name"?}`, `"replace"?: bool`, `"allowHeavy": bool = false`, `"grant"?: [permission]`, `"hash"?: "sha256 hex"` | Validates and installs a module; returns its `plugin.list` entry. The desktop app and `vectorcraft-cli` keep it in the `Plug-ins` folder next to the preferences, so it is installed again after a restart (#2541). A module with other bytes than the plug-in installed with its id, or, with none installed, one whose id has [settings and a cache](#settings-and-cache) in the `Plug-ins` folder, installs only with `replace: true`, once the user agreed; it takes that plug-in's place and gets its settings and cache. The same module installs again without `replace`, and `replace: false` refuses a module whose id is installed. A [heavy filter](#heavy-filters) installs only with `allowHeavy: true`, once the user agreed. A plug-in whose manifest lists [permissions](#permissions) installs only when `grant` lists every one of them, once the user agreed, unless the user already granted a module with the same SHA-256. With `hash`, a module whose SHA-256 differs is refused. |
| `plugin.inspect` | as `plugin.install`: `{"path"}` or `{"dataBase64", "name"?}` | Validates a module without installing it; returns the `plugin.list` entry it would have, plus `needsApproval` (true when `plugin.install` needs `grant`) and `needsReplace` (true when `plugin.install` needs `replace: true`). |
| `plugin.remove` | `{"id"}` | Uninstalls, and deletes it from the `Plug-ins` folder with its [settings and cache](#settings-and-cache). |
| `plugin.clearCache` | `{"id"?}` | Deletes every entry in the [cache](#settings-and-cache) of plug-in `id`, or of every installed plug-in; settings stay. Returns `{"cleared": [id], "bytesFreed"}`. |
| `plugin.reload` | `{"path"?: "folder"}` (default: the preference folder) | Installs every `*.wasm` in a folder; a bad module is reported and skipped, and so are a heavy filter and a plug-in whose permissions the user hasn't granted (nobody was asked). |

Live effects are applied like any effect: `effect.apply {"effect": "plugin.<id>", "params": {…}}`, edited with
`effect.setParams`, listed by `effect.list` (menu `Effect › Plug-ins`) and baked by Object › Expand Appearance.

An exporter adds a write-only format, `plugin.<id>`, which `document.formats` lists with its `plugin` id.
`document.export {"format": "plugin.<id>", "params"?: {…}, "path"?}` writes it, and so does a `path` ending in one of
its extensions that no built-in format has; `selectedOnly` hands it the selection, as for the built-in formats.

An importer adds a read-only format, `plugin.<id>`, which `document.formats` lists with its `plugin` id. `openExtensions`
holds the extensions File › Open opens with the importer (the lists below describe which are left out). `document.open {"path"}` (or `{"name", "dataBase64"}`) opens a file with it when no built-in format
reads the file, by its content or by its extension, and the importer lists the file's extension; when several do, the
one whose id sorts first opens it. `document.open {"importer": "plugin.<id>", "params"?: {…}}` names the importer,
which must list the file's extension, also one a built-in format reads (an importer that lists `svg` opens an `.svg`
only this way). A damaged file of a built-in format reports that format's error, and the file is not handed to an
importer. The importer's objects become a new document whose artboard is their bounds (at least 1 pt wide and high,
and within ±4,000,000 pt); the document keeps no path, so Save asks for a name. File › Place reads only the built-in formats, and a file only an importer reads opens as a new document when it is dropped on the canvas. These rules are
VectorCraft's own.

In the app, installed filters appear under **Object › Plug-ins**, next to **Install Plug-in…** and **Reload
Plug-ins**; live effects under **Effect › Plug-ins** and in the Appearance panel's effect menu. A plug-in with
parameters gets a dialog generated from its schema (numbers, whole numbers, checkboxes, dropdowns, text, lists
of numbers and colors, labelled, explained and sorted into sections as its manifest says) with a live preview; without
parameters it runs at once. Edit › Keyboard Shortcuts lists each installed filter under Object › Plug-ins, where
it can be given a shortcut (`plugin:<id>`); the shortcut stays saved while the plug-in is removed and applies again
when it is reinstalled. Parameters too many for the window's height scroll inside the dialog, which keeps its
heading and buttons in view. Exporters are formats of **File › Export › Export As**, where Use
Artboards is greyed out (the plug-in writes the whole document); one with parameters asks for them in a dialog of the
same kind once the file is picked, and its Export button writes the file. Importers' files are listed under File ›
Open's "All readable files" and under a filter of their own, named after the importer (other open dialogs, such as New
from Template and Load Styles, list only the files the built-in formats read). These lists leave out an importer's
extensions that a built-in format reads, and those File › Open installs, imports or loads as something else (`.wasm`,
`.kys`, `.jsx`, presets, libraries and swatch or graphic style libraries such as `.ase`). File › Open does not give
those files to the importer, and Open With does. **File › Open With** lists the
installed importers by name (`file.openWith {"importer", "path"?, "params"?}`), and each opens a file picked among its
extensions with that importer (on the web, File › Open's picker opens instead) and adds it to Open Recent. On the
desktop the importer runs off the UI thread, and a run that takes a while shows a progress window with Cancel, which
stops it. An importer
with parameters, given no `params`, asks for them in a dialog of the same kind once the file is picked, and its Open
button opens the file; File › Open runs an importer with its parameters' defaults. Open Recent opens a file the way
File › Open does: it does not remember the importer or the parameters given to Open With. A built-in format reads the
file if one can, and otherwise the first importer by id that lists its extension reads it with its parameters' defaults.
**File › Open** of a `.wasm` file installs
a plug-in too (on the web this is how plug-ins get in). A heavy filter asks before it is installed, and runs in the
background ([below](#heavy-filters)). A plug-in that lists permissions asks for them before it is installed
([below](#permissions)).

### Commands

A filter or a live effect may declare commands of its own in its manifest (`commands`, [below](#manifest)). Each
becomes the command `plugin.<id>.<command>` while the plug-in is installed, and is gone once the plug-in is removed or
replaced by a module without it.

- **Menus.** A plug-in with commands is a submenu of **Object › Plug-ins** (a live effect: **Effect › Plug-ins**)
  named after the plug-in. The submenu holds the plug-in's own item, then its commands. A command with parameters has
  an ellipsis and opens a dialog for them, with a live preview, as a filter's own item does. A manifest cannot put a
  command in another menu: a `menu` key in a command is ignored.
- **Keyboard Shortcuts.** Edit › Keyboard Shortcuts lists each command under its submenu, and the user can give it a
  shortcut. A manifest cannot give a command a shortcut.
- **Actions and agents.** A command run is recorded in an action like any other command and plays back with its
  parameters. `run`, MCP and `command.batch` call it as `plugin.<id>.<command>` with `{"ids"?: [...], "params"?: {…}}`
  (its parameters may also be top-level keys). MCP's `command_list` and `list_commands` list it, and `plugin.list`
  lists a plug-in's commands as `commands: [{id, command, label, params, paramsSchema}]`.
- **Runs.** The parameters are checked against the command's declarations before the plug-in runs. The plug-in runs
  through `vc_run` on the selection (or `ids`) as a filter run does, with `_context.mode` `"command"` and
  `_context.command` set to the command's id ([Parameters](#parameters)), under the plug-in's
  [permissions](#permissions): a live effect with commands may also hold `doc.write` and `state`, which only its
  commands use. `ids` left out or null means the selection; an `ids` that is not a list of object ids is an error. One run
  is one undo step named after the command. A run that traps, runs out of its instruction budget or fails leaves the
  document and the undo history as they were, and the error starts with the plug-in's name and the command's label
  (`“Cmd Test” › Grow: …`).

**Preference folder.** Edit › Preferences › Plug-ins & Scratch Disks › *Additional Plug-ins Folder*: when set, every
`*.wasm` in that folder (not recursive) is installed at startup and whenever the preference changes (desktop builds;
the web build installs from bytes). On the desktop, Choose… beside the field picks the folder; the dialog refuses a
picked or typed folder whose contents can't be listed, and `prefs.set` refuses such a folder too.

Plug-ins are installed **per process**, not per document. Documents store a live effect as an ordinary effect record
(`{"id": "plugin.<id>", "params": {…}}`): while the plug-in is installed it is drawn, exported and expanded; without it
the object is drawn as if the effect weren't there, and the record is kept for when the plug-in comes back.

## The sandbox

The host is [`wasmi`](https://github.com/wasmi-labs/wasmi), a pure-Rust WebAssembly interpreter
(MIT OR Apache-2.0). It builds for `wasm32-unknown-unknown` too, so plug-ins also run in the web build. Limits
(`vectorcraft_plugins::Limits`, defaults):

| Limit | Default | On violation |
|---|---|---|
| Host imports | none at API level 1; at level 2 only the [host imports](#host-imports-api-level-2): no WASI, file system, network, clock or randomness | module rejected at install, naming the import |
| Module size | 32 MiB | rejected |
| Linear memory | 512 MiB per instance (input and parameters: at most half) | `memory.grow` returns -1; oversized input or initial memory fails |
| Output | 64 MiB of JSON | `Limit("output size")` |
| Instructions (fuel) | 50 M per call + 4,000 per input byte for `vc_run`; a [heavy filter](#heavy-filters) 1 G + 4,000 per byte; an export or import 10 G + 4,000 per byte | `Limit("instruction budget")` |
| Wall clock | 60 s per filter run, 5 s per live-effect run, 300 s per heavy filter run, export or import (desktop; checked every 20 M instructions) | `Limit("time budget")` |
| Import input | 128 MiB of the file (`vectorcraft_plugins::import::MAX_INPUT_BYTES`) | the file doesn't open; a larger file named by `path` is not read past that |
| Call depth | 1,024 frames | trap |
| Module shape | wasmi's strict limits (≤ 10,000 functions, 1 memory, …) | rejected |

An export runs where the export runs: with Background Export on, on the export's worker, whose Cancel stops the
plug-in between two of those 20 M-instruction slices; nothing is written then. An import stops the same way when the
open it runs in is cancelled, and nothing opens then.

### Heavy filters

Some filters do heavy but legitimate work, such as routing a plotter's pen, packing shapes onto a sheet or running a
simulation, which a minute and 50 M instructions don't cover. Such a filter declares `"budget": "heavy"` in its
manifest (only a filter may: a live effect runs whenever the document is drawn, an exporter has an export's budget and
an importer an import's). Each of its runs gets 1 G instructions (20 times a light run's) plus the 4,000 per input byte, and 300 s
(`vectorcraft_plugins::{HEAVY_FUEL, HEAVY_WALL_TIME_MS}`). Its output can't add to the budget: the host learns how
much there is only once the run has ended.

- **Installing asks first.** Install Plug-in… and File › Open show what the filter asks for, and install it once the
  user clicks OK; `plugin.install` refuses it without `allowHeavy: true`, which an agent passes once the user agreed
  (`plugin.inspect` tells it beforehand). A filter kept in the `Plug-ins` folder was agreed to and is installed again
  at startup. The Additional Plug-ins Folder skips heavy filters with a note in its report, since loading a folder
  asks nobody.
- **It runs in the background.** Its dialog has no Preview. OK (or the menu item, without parameters) runs it on a
  thread of its own while editing goes on, shown in the status bar and, after a moment, in a progress window with
  Cancel (also Esc) and Continue in Background (`plugin.cancelRuns`, `plugin.hideRuns`). When it is done its result is
  applied as `plugin.run` with the same parameters and objects, as one undo step, unless the document or the plug-in
  changed meanwhile: then nothing changes, and the status bar says to run it again. On the web, which has no threads,
  it runs at once; the instruction budget alone bounds it there.
- `plugin.run` itself runs a heavy filter where it is called, as it runs any filter: an MCP client waits for it, and
  its `notifications/cancelled` stops it.

### Permissions

A manifest may list what the plug-in may do in `permissions`. `doc.read`, `doc.write` and `state` follow the
capability model proposed for PhotoCraft plug-ins
([storytold/photocraft#662](https://github.com/storytold/photocraft/issues/662)); the other names are VectorCraft's own:

| Permission | Kinds | What it allows |
|---|---|---|
| `doc.read` | filter, effect, exporter | Read the objects it is given. |
| `doc.write` | filter, effect with `commands` | Change, add and delete objects in the document. |
| `state` | filter, effect with `commands` | Keep data of its own with the document (`documentData`). |
| `effect.render` | effect | Change how the objects its effect is applied to are drawn. |
| `export.write` | exporter | Write the document as a file in its own format. |
| `import.read` | importer | Read files you open in its format and make documents from them. |
| `progress` | all | Show its progress, see Cancel and write log lines through the [host imports](#host-imports-api-level-2). Needs `api` 2. |
| `settings` | all | Keep settings of its own between runs and sessions ([settings and cache](#settings-and-cache)). Needs `api` 2. |
| `cache` | all | Keep a cache of its own, which the user can clear ([settings and cache](#settings-and-cache)). Needs `api` 2. |

- A manifest without `permissions` gets the ones its kind implies, as plug-ins did before the key existed: a filter
  `doc.read`, `doc.write` and `state`; an effect `doc.read` and `effect.render`, and also `doc.write` and `state`
  when it declares [commands](#commands); an exporter `doc.read` and `export.write`; an importer `import.read`. Such a
  plug-in installs without a prompt. This rule is VectorCraft's own. A filter whose list is empty may do none of
  these, and also installs without a prompt.
- An effect, an exporter or an importer that lists permissions lists all of its implied ones (`doc.read` and
  `effect.render`; `doc.read` and `export.write`; `import.read`). An effect with commands may add `doc.write` and
  `state`, which only its commands use. A filter may list any of its three.
  Any kind may add `progress`, `settings` and `cache`, which no kind implies. An unknown name, a name listed twice, a
  name that doesn't apply to the kind, more than 16 names, `progress`, `settings` or `cache` with an `api` below 2, or
  one of the names kept for a later plug-in API
  (`ui.panel`, `ui.tool`) is refused when the module is installed, and the error names it.
- **Installing asks first.** Install Plug-in… and File › Open show the plug-in's name, id, version, the SHA-256 of
  the file and each permission in plain words, and install it once the user clicks OK, if the file still has the
  SHA-256 shown; Cancel installs nothing. The
  names shown have control and bidirectional characters escaped (`\u{202E}`) and are cut to 64 characters.
  `plugin.install` refuses the module, naming the permissions it needs, unless `grant` lists them all, which an agent
  passes once the user agreed (`plugin.inspect`'s `needsApproval` tells it beforehand), with `hash` set to the
  `plugin.inspect` entry's `hash` so that the module installed is the one the user saw. `plugin.install` does not
  check that a person approved `grant`: it trusts the client that sends it.
- **Replacing asks first.** When the file is another module than the plug-in installed with its id, or, with none
  installed, its id has settings and a cache in the `Plug-ins` folder, Install Plug-in… and File › Open show its id
  and SHA-256 and ask whether to install it in place of that plug-in, which gives it the settings and cache kept for
  the id. `plugin.install` refuses such a module, naming it, unless `replace` is `true`, which an agent passes once
  the user agreed (`plugin.inspect`'s `needsReplace` tells it beforehand). The Additional Plug-ins Folder and
  `plugin.reload` do not ask.
- The grant is recorded with the plug-in's id, the module's hash and the permissions, in `grants.json` in the
  `Plug-ins` folder, and covers only that module. Installing the same bytes again needs no new approval. A module with
  other bytes asks again, even when it lists the same permissions, and does not get the id's settings and cache until
  it is approved. Until then, the installed module and its grant stay as they are; once it is approved, the grant
  records the new hash. When the new module or its grant can't be written to the `Plug-ins` folder, the folder keeps
  the old module and its grant, and the old module loads at the next start. If the write that puts the old module back
  fails as well, the folder holds the new module with the old grant, the grant does not cover that module, and the
  plug-in is skipped at the next start until it is approved again. A grant whose hash is not 64 lower-case
  hexadecimal digits covers nothing. `plugin.remove` forgets the grant, and so does a successful `plugin.install` of a
  module that lists no permissions under that id, so the earlier bytes need a new approval. When `plugin.remove` can't
  remove the grant from `grants.json`, it returns an error, the plug-in stays installed, and a second `plugin.remove`
  tries again. A refused install leaves the grant as it is. When an install of a module that lists no permissions can't
  write the module to the `Plug-ins` folder, or can't remove the grant from `grants.json`, the folder keeps the module
  it held and `grants.json` keeps the grant. The new module runs until the session ends. When the grant can't be
  removed and the module already stored can't be read back, the new module stays in the folder, and the grant does not
  count for the rest of the session. The grant also does not count when the folder held no module for that id. A
  `grants.json` larger than 1 MiB, or one that is not a JSON list, grants nothing: the new module stays in the folder,
  and `plugin.remove` removes the plug-in. A `grants.json` that can't be opened or read counts as a grant that can't be
  removed. When VectorCraft loads the `Plug-ins` folder, it forgets any grant for an id whose stored module lists no
  permissions. When it can't remove that grant from `grants.json`, or can't read the file, the grant does not count for
  the rest of the session, or until the plug-in is granted again, including for the next load, which tries the removal
  again. In Safe Mode, and for
  a stored module that the user disabled,
  VectorCraft does not read the module, and `grants.json` keeps the grant for its id. The Additional Plug-ins Folder
  and `plugin.reload` do not change the `Plug-ins` folder: when they leave a module that lists no permissions under an
  id, the process forgets its own grant for that id, and a grant kept in `grants.json` stays with the stored module it
  covers. An entry of `grants.json` that this version can't use (a permission it doesn't know, an invalid id) is
  skipped and stays in the file until its plug-in is granted again, replaced by a module that lists no permissions, or
  removed. A `grants.json` that is not a JSON list is never overwritten, and the file is never written larger than
  1 MiB, the most that is read. When a `Plug-ins` folder is in use, only the grants kept
  in it count for `plugin.install`, `plugin.inspect`, `plugin.reload` and the Additional Plug-ins Folder. At startup,
  a stored plug-in is skipped when no grant covers its bytes and the permissions it lists. The Additional Plug-ins
  Folder and `plugin.reload` install only plug-ins that such a grant covers, and report the others. A module without a
  `permissions` list, or with an empty one, needs no grant.
- The host enforces the grant on a filter run and a command run: without `doc.read` the filter is given no objects; without `state` its
  `documentData` is `null` and an output that sets it is refused; without `doc.write` an output that changes the
  document is refused. A refused output leaves the document as it was.
- `api` is the plug-in API level the manifest is written for: `1` (the default) or `2`, which adds the [host
  imports](#host-imports-api-level-2), a filter's `inputs` ([reading type, images and
  symbols](#reading-type-images-and-symbols)) and a live effect's [`paintOutput`](#painted-live-effects). A higher one
  is refused. `plugin.list` and `plugin.info` give it as `api`. `inputs` is not a permission: the objects it asks for
  are read under `doc.read`, which a filter that lists `inputs` must list in `permissions`, so installing it asks the
  user to grant them. A level 2 effect that declares `paintOutput` needs no other permission than `doc.read` and
  `effect.render`.
- Parameter declarations are unchanged by permissions (`params`, below).

Every run gets a **fresh instance**, so no state survives between calls except what a plug-in writes to its
[settings and cache](#settings-and-cache). Traps, exhausted budgets, malformed
modules, bad manifests, out-of-range pointers, negative return codes and invalid output all become errors, and the
document is left untouched: a filter's output is checked completely before one edit applies it. A live effect that
fails draws the object's geometry unchanged; `plugin.info` reports why (`lastError`). A run that succeeds clears the
error of a failed run. A check that finds every swatch and pattern a painted result names clears only the error of a
refused swatch or pattern.

wasmi is built with its *portable* (loop) dispatcher: the default tail-call dispatcher relies on LLVM
sibling-call optimisation, and when that doesn't happen a long-running module overflows the host stack, which aborts
the process instead of returning an error.

wasmi compiles each function when it is first called, and the instruction budget doesn't pay for that, so a run
takes as many instructions the first time as the next. Charged to the run, as wasmi charges by default, compiling
could fail it: when the 20 M-instruction slice the run was in ran out while a function compiled, wasmi ended the run
with an error rather than a pause the host resumes, and the run stopped with most of its budget left.

Drawing evaluates effects often, so live-effect results are cached by plug-in installation, parameters, input
geometry, fill rule, reference box and the paints the effect is given: a plug-in runs again only when one of them
changes. Failures are cached too. The swatches and patterns a painted result names are checked against the document
each time it is drawn, and a paint linked to a global swatch is drawn in that swatch's current color at its own tint.

## ABI v1

A plug-in is a core WebAssembly module (MVP + the usual post-MVP features: bulk memory, sign extension, multi-value,
saturating conversions; no SIMD, no memory64) with these exports and no imports other than the [host
imports](#host-imports-api-level-2) of API level 2. Integers are `i32` unless noted; pointers are offsets into the
module's memory.

| Export | Signature | Meaning |
|---|---|---|
| `memory` | memory | The module's linear memory. |
| `vc_abi_version` | `() -> i32` | Must return `1`. |
| `vc_manifest` | `() -> i64` | `(len << 32) \| ptr` of the UTF-8 manifest JSON (≤ 64 KiB). |
| `vc_alloc` | `(size) -> ptr` | A block of `size` bytes the host may write, or `0` on failure. Never freed: each run gets a fresh instance. |
| `vc_run` | `(input, input_len, params, params_len) -> i64` | Runs the plug-in on the input JSON with the parameters JSON; returns `(len << 32) \| ptr` of the output JSON, or a negative error code. |

### Host imports (API level 2)

A plug-in whose manifest gives `"api": 2` and lists the [permission](#permissions) an import needs may import these
functions from module `env` (the module a Rust or C toolchain gives `extern` functions by default). `vc_abi_version`
still returns `1`. `vc_progress`, `vc_cancelled` and `vc_log` need `progress`; the `vc_settings_*` imports need
`settings` and the `vc_cache_*` imports need `cache` ([settings and cache](#settings-and-cache)).

| Import | Signature | Meaning |
|---|---|---|
| `vc_progress` | `(done: i64, total: i64)` | `done` of `total` steps are finished; `total` 0 or less when not known. Negative values count as 0, and `done` is clamped to `total`. |
| `vc_cancelled` | `() -> i32` | `1` once the user cancelled the run or its time budget ran out, else `0`. |
| `vc_log` | `(level, ptr, len) -> i32` | Writes the UTF-8 text at `ptr` as a log line of `level` (0 error, 1 warning, 2 information, 3 debug). Returns `0` when written, `1` when the run's log budget is spent and the line was dropped, `-1` when the text doesn't lie inside the module's memory, `-2` when it is not UTF-8, `-3` for another level. |

- The host checks the imports when the module is installed: an import it doesn't offer, one with another signature,
  and one the manifest doesn't allow (an `api` below 2, or without the import's permission) refuse the module, and the
  error names the import and the permission.
- A filter's or an exporter's progress is shown as a share: a heavy filter's progress window and the export's progress
  window draw a bar with a percentage, and Cancel there makes `vc_cancelled` return `1`. A plug-in that checks it can
  stop early, and the host discards what it returns after the Cancel or after its time budget ran out; one that
  ignores it is still stopped by the instruction and time budgets. With a known total, a report is passed on only when
  it reaches another tenth of a percent; with an unknown total, each change of `done` is passed on. After 2,000 reports
  in a run, one change in 1,000 is passed on, and so is the first report after the total becomes known or unknown. A
  live effect's progress is not shown, since it is drawn as part of something else.
- Log lines go to the app's log (target `vectorcraft_plugins`) as `<plug-in id>: <text>`, with control and
  bidirectional characters escaped. The whole text must be UTF-8. A line shows its first 1,024 bytes, as at most 1,024
  characters with the escapes, and ends with `…` when it is cut. A run writes at most 200 lines and 32 KiB of shown
  text, and the host reads at most 1 MiB of log text in a run; past any of these, one "log truncated" line is written
  and the rest are dropped.

### Settings and cache

A plug-in with the `settings` permission keeps named values between runs and sessions, and one with `cache` keeps
named entries that the user can clear (#2191). It names each value by a key and never by a path. Each import takes
the key as `(key, key_len)`, the UTF-8 bytes at `key` in its memory:

| Import | Signature | Meaning |
|---|---|---|
| `vc_settings_get` | `(key, key_len, out, out_cap) -> i32` | The setting's length in bytes. When it fits in the `out_cap` bytes at `out`, its bytes are copied there; otherwise nothing is copied. |
| `vc_settings_set` | `(key, key_len, value, value_len) -> i32` | Sets the setting to the `value_len` bytes at `value`. Returns `0`. |
| `vc_settings_delete` | `(key, key_len) -> i32` | Removes the setting. Returns `0`. |
| `vc_cache_read` | `(key, key_len, out, out_cap) -> i32` | As `vc_settings_get`, for the cache entry. |
| `vc_cache_write` | `(key, key_len, value, value_len) -> i32` | As `vc_settings_set`, for the cache entry. |
| `vc_cache_clear` | `(key, key_len) -> i32` | Removes the cache entry; with `key_len` 0, every entry. Returns `0`. |

A call that fails returns `-1` when the key has no value, `-2` when a key, value or buffer doesn't lie inside the
module's memory, `-3` when the key is not a valid key, `-4` when the write would pass a cap (nothing changes), `-5`
when there is nowhere to keep anything, `-6` when reading or writing the file failed, and `-7` when the run has used
its 10,000 storage calls or its 1 GiB of storage reads and writes. Each call counts the bytes it reads and writes. A
settings call counts the whole settings file when it reads the file, and a set or delete also counts the whole file
when it writes the file; a cache call counts the entry it reads or writes. A read counts them even when the value
doesn't fit and nothing is copied. A call that would pass the 1 GiB returns `-7` before it reads or writes anything.

The app keeps the settings in memory after the first call and reads `settings.json` again only when the file's size
or modified time differs from the last read or write. A get answered from memory reads no file and counts no bytes. A
set to the value the key already has writes nothing, and neither does a delete of a key that has no value. A delete
of the last setting removes the file and counts no written bytes. Every other successful set or delete writes the
file before the call returns. A cache call reads or writes the whole entry in memory in one step, up to 64 MiB, and
the run waits for it.
A live effect runs while the art draws, so keep the entries it uses small.

- A key is 1 to 64 characters from `A-Z`, `a-z`, `0-9`, `_` and `-`, and is not a Windows device name (`CON`, `PRN`,
  `AUX`, `NUL`, `COM0` to `COM9`, `LPT0` to `LPT9`, in any case). Keys differ in case: `tile` and `Tile` are two.
- These caps are VectorCraft's own values: at most 256 settings of at most 64 KiB each, in a settings file of at most
  1 MiB; cache entries of at most 64 MiB each, at most 4,096 entries and 512 MiB per plug-in. The cache has no
  automatic eviction: a write past a cap fails with `-4`, and the plug-in or the user clears entries. A cache write
  also fails with `-4` when the `cache` folder holds more than 8,192 items; clearing the cache removes every file in
  it.
- The desktop app and `vectorcraft-cli` keep them in a folder named from the id under `storage/` in the `Plug-ins`
  folder (ids that differ only in case get two folders): `settings.json` and a
  `cache` folder with one file per entry, named by the host. A settings file that is too large or doesn't parse is
  moved to `settings.unreadable.json`, replacing an earlier one there, and the settings start empty. A link in that
  folder is refused (`-5`). Writes go to a temporary file that then replaces the old one; a temporary file a failed
  write leaves in the `cache` folder does not count toward the caps, and clearing the cache removes it.
- A plug-in reaches only the storage of its own id. Replacing it with another module of the same id
  (`plugin.install` with `replace: true`, after the user agreed) keeps the storage; `plugin.remove` deletes it, and a run of the plug-in still going gets `-5` from every storage call after that.
  `plugin.remove` first renames the folder (adding `.removed` to its name). When that fails, `plugin.remove` fails
  and the plug-in stays installed with its storage as it was. Files that can't be deleted after the rename stay in the
  `.removed` folder, where no plug-in reaches them. A later `plugin.remove` tries again to delete them when a plug-in
  with that id is installed or a storage folder for the id exists, and that deletion can fail too. For an id that
  isn't installed and has only a `.removed` folder, `plugin.remove` does not delete the folder and fails as not found.
  `plugin.remove` with the id of a plug-in that isn't installed deletes the storage kept for that id, when there is
  any.
  `plugin.clearCache` empties the cache and leaves the settings, and `plugin.list` and
  `plugin.info` give the cache's size as `cacheBytes`.
- Without a `Plug-ins` folder (the web build, and embedders that attach none) the storage calls return `-5`.

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
    "amount": {"type": "number", "min": 0, "max": 100, "default": 50, "unit": "%"},
    "steps":  {"type": "int", "min": 1, "max": 16, "default": 4, "label": "Steps per curve"},
    "closed": {"type": "bool", "default": false},
    "mode":   {"type": "choice", "options": ["soft", "hard"], "default": "soft",
               "help": "Soft keeps the corners round; hard keeps them sharp."}
  }
}
```

- `id`: 1–64 characters from `A-Z a-z 0-9 . _ -` (reverse-DNS recommended). Installing a module with the same id
  replaces the old one. A live effect's effect id is `plugin.<id>`.
- `name`: the menu label, or an exporter's or importer's format name in Export As, Open and Open With (1–64 printable
  characters). `kind`: `"filter"` (object filter), `"effect"` (live effect), `"exporter"` or `"importer"`. Exporters
  and importers came after the first ABI v1 hosts, which refuse the kind when the plug-in is installed.
- `budget` (filters only): `"light"` (the default) or `"heavy"`, for work that takes minutes ([heavy
  filters](#heavy-filters)). Hosts from before it ignore the key, so there the filter gets a light run's budget.
- `api`: the plug-in API level, `1` when left out. `permissions`: what the plug-in may do, a list of names
  ([permissions](#permissions)); without it, what its kind implies.
- `inputs` (filters at `api` 2 only): the kinds of object besides paths and compound paths the filter reads, any of
  `"type"`, `"image"` and `"symbolInstance"`, each at most once (`"inputs": ["type", "image"]`). Left out or `[]`, the
  filter gets paths and compound paths alone, as before. An unknown, repeated or wrongly typed entry, `inputs` on
  another kind, below `api` 2, or without a `permissions` list that holds `doc.read`, is refused when the plug-in is
  installed, and so is `api` 2 on hosts from before it.
- `paintOutput` (live effects at `api` 2 only): `true` makes the effect return [painted pieces](#painted-live-effects)
  instead of geometry alone. `true` is the only value the key takes; `false`, `null`, another type, another kind or
  `api` 1 is refused when the module is installed, and the error names `paintOutput`. Without the key the effect returns
  geometry, at any level. Hosts at `api` 1 refuse the manifest for its `api`, so such an effect never draws unpainted
  there.
- `extensions` (exporters and importers only, and required): 1–8 file extensions, lower-case letters and digits
  without the dot (`["plt"]`); the first is the one Export As gives the file. A built-in format keeps an extension both
  list (`svg`): `document.export` then writes the exporter's format only when asked for by its format id, and
  `document.open` reads a file with the importer only when `importer` names it.
- `commands` (filters and live effects; not heavy filters): up to 32 commands of the plug-in's own
  ([Commands](#commands)), each `{"id": "grow", "label": "Grow", "params"?: {…}}`. An `id` is 1–32 characters from
  `a-z 0-9 _` that starts with a letter, and appears once. A `label` is 1–64 printable characters; menus show it with
  control and bidirectional characters escaped. `params` are declared as the plug-in's own `params` are. A malformed
  entry is refused when the module is installed, and the error names the command by its id (or its place, `[0]`).
- `params`: in dialog order. Names are `[A-Za-z0-9_]`, not starting with `_`, and not `id`, `ids`, `params` or
  `preview`. The host validates the user's values before calling: missing values take the default, numbers are
  clamped to `min..max` (ints rounded), a wrong type or an unknown choice is an error. Numeric defaults
  are also clamped to their range, including the implicit zero default of an `int` parameter. Besides `number`,
  `int`, `bool` and `choice` there are `string` (`{"type": "string", "default": "", "maxLength": 256}`: a text field;
  longer text is cut to `maxLength`, at most 4,096 characters) and `numberList` (`{"type": "numberList", "min": 0,
  "max": 360, "default": [45, 135], "minItems": 1, "maxItems": 16}`: numbers typed separated by commas or spaces,
  each clamped to `min..max`; a list of too few or too many is an error, and `maxItems` is at most 256) and `color`
  (`{"type": "color", "default": "#3a7bd5"}`: a color button that opens a color picker; the default, black when left
  out, and a value given to `plugin.run` may be a color object as paints hold one or a hex string, and the plug-in
  always gets a color object, its components clamped, such as `{"model": "rgb", "r": 0.23, "g": 0.48, "b": 0.84}`; a
  hex string and the picker's choice are display colors, given as the working RGB color that shows as them, so in a
  working RGB space other than sRGB the numbers differ from the hex). Hosts from before `color` refuse a manifest that
  uses it. A plug-in declares up to 256
  parameters, at most 64 of them outside groups.

Each parameter may also say how dialogs show it. Hosts from before these keys ignore them, so a plug-in that
declares them still installs there:

| Key | What the dialog does with it |
|---|---|
| `label` | Labels the field (1–64 characters). Without one, the label is the name spaced out (`rowSpacing` → Row Spacing). |
| `unit` | Shows the unit the value is in after it (1–16 characters, such as `mm` or `°`). Nothing is converted: the plug-in gets the number as typed. |
| `group` | Puts the parameter in a section that folds (1–64 characters). Parameters outside any group come first, then each group in the order it first appears. |
| `help` | Shows what the parameter does when the pointer rests on its label (1–1,000 characters; line breaks allowed). |
| `helpUrl` | Opens this `https://` page (at most 512 bytes) from a button after the field. |
| `visibleWhen` | `{"pattern": ["lines", "crosshatch"], "outline": [true]}` shows the parameter only while each parameter it names (a checkbox or a choice declared before it) has one of the values listed and is shown itself. |

A hidden parameter still goes to the plug-in with its value: what the dialog showed never changes a run. The
filter, live-effect and exporter dialogs use all of these keys. The Appearance panel's editor of a plug-in effect's
parameters uses `label`, `unit`, `help` and `visibleWhen`, and lists the parameters without sections or help buttons.

### Input

`input` is UTF-8 JSON: `{"objects": [...]}`. An object filter gets the selected paths and compound paths in paint
order (bottom first): the selected ones and those inside selected groups and layers; hidden, locked, guide (compound
guides too) and clipping paths are left out, and so are other kinds of object (type, images, symbols, live blends…),
which a filter never touches unless its manifest lists them in `inputs` ([reading type, images and
symbols](#reading-type-images-and-symbols)). With nothing selected the list is empty (a generator then adds its own objects). A live effect gets one
object: its geometry (`type`, `path`, `fillRule`, `bounds`) and its `fills` and `strokes`, as a filter gets them (hosts
from before these give geometry only). The paints are read only: an effect may draw as wide as a stroke or use a
fill's colour to decide its shape, but it changes only the geometry, unless it declares `paintOutput` ([painted live
effects](#painted-live-effects)).

An **exporter** gets something else: the native `.vectorcraft` document JSON without indentation, optional preview/PDF
renditions or save-preference metadata (the format is documented in [`crates/format/src/lib.rs`](../crates/format/src/lib.rs)). That is every layer, object,
appearance and effect record, artboard, swatch and symbol, so an exporter reads whatever the document holds; it checks
`version`, since a later host may write a newer one. Images of 8 KiB or more are listed in `images` with their MIME
type and `blob` but without their data, which follows the JSON in a saved file; smaller ones keep their base64 `data`.

An **importer** gets the bytes of the file being opened, as they are (not JSON), at most 128 MiB of them.

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
     "width": 2, "join": "round", "cap": "butt", "miterLimit": 10}],
   "effects": [{"id": "distort.roughen", "params": {"detail": 10, "points": "smooth", "relative": true, "seed": 0,
     "size": 5}, "visible": true}],
   "opacity": 1,
   "bounds": [10, 0, 75, 60]}
]}
```

- `type`: `"path"` or `"compound"` (a compound path: `path` holds its members' subpaths, painted as one).
- `path`: VectorCraft's own path form. Each subpath is a run of anchors; `p` is the anchor, `in` / `out` its handles
  (absent: no handle), `kind` `"Smooth"` when the handles stay collinear. Coordinates are points, y down.
- `fills`, `strokes`: the object's fills and strokes in paint order (bottom first); `null` is no paint. A stroke has its
  `paint`, `width` in points, `join` (`"miter"`, `"round"` or `"bevel"`), `cap` (`"butt"`, `"round"` or `"square"`)
  and `miterLimit`. Paints use the
  native file format's form: `{"type": "solid", "color", "swatch"?, "tint"?}`, `{"type": "gradient", "gradient":
  {"kind": "Linear"|"Radial"|"Freeform", "stops": [{offset, color, opacity, midpoint}], "dither"?, "method"?: "Perceptual"}, "geom"?, "angle",
  "freeform"?}`, `{"type": "pattern", "pattern", "xf"?}`. Colours are `{"model": "rgb", r, g, b}`, `{"model": "cmyk",
  c, m, y, k}`, `{"model": "gray", k}` (ink: 0 white, 1 black) or `{"model": "lab", l, a, b}`; components 0–1 except
  Lab's.
- `effects` (filters): the object's own effects in order, as its effect records: `id` (as `effect.list` lists it, or
  `plugin.<id>` for a live-effect plug-in), `params` and `visible`. Its fills' and strokes' own effects aren't listed.
- `bounds`: `[x0, y0, x1, y1]`, the geometric bounds (read only).

### Reading type, images and symbols

A filter whose manifest lists `inputs` also gets the selected objects of those kinds, in paint order among the paths
and under the same rules (hidden and locked ones, objects that clip a clip group, alone or inside a group that clips
it, and anything inside a symbol, a placed document or a live object are left out). They are never turned into
outlines, rectangles or the symbol's art. Each has `id`, `type`, `name` (when it has one), `bounds` (`null` until the
object has been laid out or drawn) and
`transform`, `[a, b, c, d, e, f]` mapping its own space (text space, pixel space, the symbol's art) into the document.
In this example the character and paragraph styles are cut short; a filter gets every attribute:

```json
{"objects": [
  {"id": 17, "type": "type", "bounds": [10, 38, 52, 52], "transform": [1, 0, 0, 1, 10, 50],
   "text": {"kind": "point", "vertical": false,
            "runs": [{"text": "Hello\nthere", "style": {"font_family": "Source Sans 3", "size": 12}}],
            "paragraphs": [{"justify": "Left"}, {"justify": "Center"}]},
   "truncated": false},
  {"id": 18, "type": "image", "bounds": [0, 0, 4, 2], "transform": [2, 0, 0, 2, 0, 0], "pixelSize": [2, 1],
   "colorMode": "rgb", "linked": false, "pixelsOmitted": false, "pixelsBase64": "AAAegGQAHoA="},
  {"id": 19, "type": "symbolInstance", "bounds": [100, 0, 110, 10], "transform": [1, 0, 0, 1, 100, 0],
   "symbolId": "Star", "overrides": {}}
]}
```

- Type (`"type"`): `text.kind` is `"point"`, `"area"` or `"path"`; `runs` are its text runs with their character
  attributes as the native file stores them (`inline` when a run is an inline graphic), and `paragraphs` holds one
  paragraph style per paragraph (the paragraphs are split at `\n`). No font data or glyph outlines are given. Type
  whose runs hold more than 256 KiB of UTF-8 text, or more than 4,096 runs or paragraphs, comes with `truncated:
  true` and empty `runs` and `paragraphs`. The runs and paragraphs of all the type in one input take at most 16 MiB
  of JSON: type whose runs and paragraphs don't fit what the type before it in paint order left comes with
  `truncated: true` too.
- An image (`"image"`): `pixelSize` is its width and height in pixels; `colorMode` is `"rgb"`, `"cmyk"`, `"gray"` or
  `"unknown"`, read from the image itself. An embedded image whose pixels take at most 1 MiB in base64 (443 × 443
  pixels fit) has them in `pixelsBase64`: straight RGBA, 8 bits a channel, row by row from the top, and
  `pixelsOmitted: false`. A CMYK image's pixels are its inks through the working CMYK profile. The pixels of all the
  images in one input take at most 16 MiB in base64. A larger image, one whose pixels don't fit what the images before
  it in paint order left, and a linked image (`linked: true`) have
  `pixelsOmitted: true` and no pixels; the host checks an image's size before it decodes the image. The host never
  reads a linked image's file for a filter and never gives its path.
- A symbol instance (`"symbolInstance"`): `symbolId` is the name of the symbol it shows, with `missing: true` when the
  document has no symbol of that name. `overrides` is `{}`.

These objects are read only. The output must list each of them exactly once, as `{"id": 17}` alone or exactly as it
was given (numbers compare by value, so `1` and `1.0` are the same). A changed or added field, one left out, an object
listed twice or left out, and a new object with one of these types are refused, and the document and undo history
stay as they were: `objects[1] (id 17): read-only field 'text.runs[0].text' changed`, `objects (id 17): a read-only
object was left out`. The field is named with control and bidirectional characters escaped (`\u{202E}`), cut to 64
characters. New objects listed after one of them go just above it.

Every object in the input counts toward the half of the memory cap that the input and the parameters share, in paint
order, with its id, bounds, transform and name. Type whose runs and paragraphs don't fit what is left comes with
`truncated: true`, and an image whose pixels don't fit comes with `pixelsOmitted: true`. When the other fields of one
of these objects don't fit what is left, the run fails before the plug-in starts, naming the object and the bytes it
needs: `plug-in exceeded its memory budget (object 17 needs 4200 bytes of input, 1000 left)`. A path or compound path
that doesn't fit fails it with `plug-in exceeded its memory budget (the selection is too large to hand over)` before
the objects after it are written. An output lists at most 100,000 objects, so a run given more than 100,000 of these
objects fails with `plug-in exceeded its object budget (more than 100000 read-only objects)`. An echo of them counts
toward the 64 MiB output cap.

### Parameters

`params` points at `params_len` bytes of UTF-8 JSON: the validated parameters, plus `_context`:

```json
{"amount": 50, "mode": "soft",
 "_context": {"mode": "filter", "bounds": [10, 0, 75, 60], "artboard": [0, 0, 612, 792], "documentData": {"pens": 4}}}
```

`mode` is `"filter"`, `"effect"`, `"export"`, `"import"` or `"command"` (a [command](#commands) run, whose
`command` is the command's id); `bounds` the selection's geometric bounds (a live effect: the reference
box of the effect, the object's bounds or those of the previous effect's result); `artboard` (filters) the artboard
under the selection's centre, else the first; `documentData` (filters) what the plug-in last kept with the document
(`null` until it keeps something; see Output). A live effect's `item` says where it sits: `"object"` (one of the
object's effects), or `{"fill": i}` / `{"stroke": i}` (in the own effects of the object's `i`th fill or stroke,
counted from the bottom as in `fills` and `strokes`). An export's `_context` is `{"mode": "export", "extension": "plt",
"name": "Poster"}` instead: the extension of the file being written (one the exporter lists) and its name without the
extension (the document's name when the export is not given a path), which some formats keep in their header. An
import's `_context` is `{"mode": "import", "extension": "plt", "name": "Poster"}`: the extension of the file being
opened and its name without the extension.

### Output

The output is UTF-8 JSON in the input's shape: `{"objects": [...]}`. To decline with a message for the user, return
`{"error": "Select two paths."}`.

For an **object filter**, the output is the new state of the input objects:

- An object with the `id` of an input object updates it; keys it leaves out keep their value (`{"id": 12}` keeps the
  object as it is). `fills` and `strokes` set the object's fills and strokes in order: extra entries add fills (above
  the last one) or strokes, missing ones are removed; each stroke keeps its weight, join, cap and miter limit unless
  `width`, `join`, `cap` or `miterLimit` are given (a new one: 1 pt, miter join, butt cap, miter limit 10).
- `effects` replaces the object's own effects, in order: each `{id, params?, visible?}`. A record the object already had
  stays as it is; a new one is admitted as `effect.apply` admits it: an effect `effect.list` lists or an installed
  live-effect plug-in, with parameters it takes (filled in to their defaults; a plug-in's checked against its
  manifest), at most one 3D effect (its fills' and strokes' own effects counted), which must fit the object, and a
  Glass `textureImage` as Glass stores it (base64 of a PNG: a plug-in can't have the host read a file). A tool can
  then set up objects for a live effect in one step.
- An object without an `id` is new and needs a `path`. It goes just above the object listed before it, or, listed
  before any input object, just below the first one listed after it. New objects take a path's defaults: no fills or
  strokes, full opacity.
- Input objects missing from the output are deleted. The order of the input objects themselves doesn't change.
- `type` turns a path into a compound path (one member per subpath) or back; `name: ""` clears the name.
- `guide: true` makes the object a guide, as View › Guides › Make Guides does (a compound path's members become
  guide paths), and `guide: false` a plain path again; `locked: true` locks it and `locked: false` unlocks it. Objects
  the run locked or made guides are left out of the selection that follows. Hosts from before these keys ignore them.
- A `path` returned as it was keeps a live shape live (a rectangle keeps its corner widgets); a changed one makes it a
  plain path.

The host validates everything before the document changes: coordinates must be finite and within ±4,000,000 pt, at
most 100,000 objects and 4,000,000 anchors, 256 fills, strokes or gradient stops; colours, opacities and offsets are
clamped into range, stroke weights to 0–1,000 pt and miter limits to 1–500; a pattern must exist in the document. A
paint returned unchanged keeps its swatch link; any other paint is unlinked. One refused effect record refuses the
whole output.

A filter's output may also hold `documentData`: any JSON of at most 1 MiB, which replaces what the plug-in keeps with
the document (`null` removes it), in the same undo step as its objects. Settings that belong to the whole document
rather than to one object go there, such as the pens a plotter holds. A filter reads and writes only its own data. It
is the document's foreign data, as the perspective grid is: the native file keeps it in `document.unknown.pluginData`
under the plug-in's id, and hosts from before `documentData` keep it when they save. An exporter is given the whole
document, so it can read what every plug-in keeps there, not only its own. A live effect isn't given it: its geometry
is worked out from the object alone.

For a **live effect**, the geometry of every returned object's `path` (their subpaths, in order) replaces the
object's; the rest is ignored (the object keeps its own appearance). An empty list removes the geometry. A live effect
that declares `paintOutput` returns painted pieces instead ([below](#painted-live-effects)).

#### Painted live effects

A live effect whose manifest has `"api": 2` and `"paintOutput": true` returns `{"objects": [...]}` too, but each entry
is a piece drawn by itself, in the order returned, with paints of its own:

```json
{"objects": [
  {"path": {…}, "fills": [{"type": "solid", "color": {"model": "rgb", "r": 0, "g": 0.6, "b": 0.3}}]},
  {"path": {…}, "fillRule": "evenodd", "strokes": [{"paint": null}, {"paint": {…}, "width": 4, "join": "round"}]},
  {"path": {…}}
]}
```

- `path` is required; `fillRule` defaults to the object's. `type` and `bounds` may be handed back as given and are
  ignored. Any other key (`opacity`, `effects`, `guide`, `locked`, `name`, a top-level `documentData`…) refuses the
  result, as does a key that a paint, its color, a gradient stop, a gradient's placement or a freeform point does not
  have, such as `opacity` on a solid paint.
- `fills` and `strokes` take the forms a filter returns ([Input](#input)). A piece without `fills` (or `strokes`) is
  painted with the ones the effect was given; an empty list paints none. Given ones replace the object's in order, as
  a filter's output sets them: an existing fill or stroke keeps its own effects, opacity and blending, and a new stroke
  starts at 1 pt, miter join, butt cap, miter limit 10.
- An effect on a fill (`_context.item` `{"fill": i}`) returns only `fills`, which replace that fill on each piece; an
  effect on a stroke returns only `strokes`. The object's other fills and strokes stay where they are.
- The effects after it in the stack apply to the pieces together, the reference box being the bounds of all of them.
  Bounds, raster effects, Pathfinder effects, compound shapes and Envelope Options › Distort Appearance use the pieces
  as they are drawn. Outside outline mode, an image is also hit where the pieces of its own fills and strokes paint; a
  path or compound path is hit-tested on its source path and appearance. On an object with no geometry effect of its
  own, Pathfinder effects and compound shapes use the object's path for an effect on a fill or stroke, as they do for
  every other effect on a fill or stroke; on an object that has one, they also use the pieces of a painted effect on a
  fill or stroke. A 3D effect later in the stack uses the
  object's input path when the pieces name a swatch or pattern. Each piece counts against the budget of the
  object's geometry effects as well as its anchors, so all the painted effects in a stack make at most 100,000 pieces
  together; a run whose pieces would pass the budget draws its input as it was.
- The result is checked whole before any of it is drawn: the limits of a filter's output (100,000 objects, 4,000,000
  anchors, 256 fills and 256 strokes per piece, 256 stops per gradient), finite numbers in range with nothing clamped
  (color components, tints, offsets, opacities, midpoints 0.13 to 0.87, stops in order of `offset`, stroke
  widths 0 to 1,000 pt, miter limits 1 to 500), and every swatch a solid paint, gradient or stop links to and every pattern named in
  the document. One fault refuses the whole result: that step of the stack draws its input as it was, the effect
  record stays, and `plugin.info`'s `lastError` names the plug-in and the field, for example
  `Test Tone: plug-in output: objects[1].strokes[0].width: …`.
- The pieces are never written into the document while it is drawn, measured, previewed, saved or exported. PDF and SVG
  export draw them as the canvas does, from a copy of the document. Object › Expand Appearance writes each piece as an
  ordinary path or compound path with its fills and strokes, as one undo step. Object › Expand of gradient and pattern
  fills expands the fills of each piece, and leaves a piece with other paints as it is drawn.

An **exporter** returns `{"dataBase64": "…", "warnings"?: ["…"]}`: the file, in base64, which the host writes as it
is, and what the export left out or changed, for the user (at most 100 warnings of at most 500 characters each, shown
with the export's own). A decline (`{"error": …}`), a missing `dataBase64` or data that isn't base64 writes nothing.

An **importer** returns `{"objects": [...], "warnings"?: ["…"]}`: new paths and compound paths (no `id`) in the
filters' object form, checked as a filter's new objects are, with warnings capped as an exporter's are and shown with
the open's own. An output with no objects, with keys other than `objects`, `warnings` and `error`, or with an object
that fails the checks opens nothing, and neither does a decline (`{"error": …}`).

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
WAT…), as long as the module imports nothing but the host imports: in Rust, `std` on `wasm32-unknown-unknown` is fine as long as it never
touches WASI. `vectorcraft_plugins::wat::module` is the skeleton of a plug-in in WebAssembly text, which the tests
build their plug-ins from; `wat::echo_exporter` is an exporter whose file is its input or its parameters (with a
base64 encoder written out in WebAssembly text), which the tests read back what the host hands a plug-in with, and
`wat::when_given` a plug-in whose output depends on whether its input or parameters hold a given text, which shows in a
live effect's geometry. `wat::reporting` and `wat::cooperating` build level 2 plug-ins that import the host functions.

## Performance

The interpreter costs roughly 10–100× native speed per instruction. Filters handle a selection in one call, so a
plug-in that parses and writes JSON with `serde_json` takes milliseconds for hundreds of paths. Live effects run once
per change of their input, parameters or paints (results are cached), not per frame.

## Not in v1

- Tool plug-ins (pointer events on the canvas, a guide line or rectangle, geometry back), panel plug-ins, and
  importers that place files (File › Place reads only the built-in formats) or make type, images, layers or
  artboards.
- Live effects that change opacity, blending or effects (at `api` 2 one may [paint](#painted-live-effects) the pieces
  it returns), and filters that change type, images or symbol instances (a filter at `api` 2 reads them, see
  [`inputs`](#reading-type-images-and-symbols)).
- Apply Last Effect and Last Effect work for plug-in effects; there is no "last filter" for object filters yet.
