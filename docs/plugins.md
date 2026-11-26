# Plug-ins: the WebAssembly plug-in API (ABI v1)

VectorCraft does not host native plug-ins built for other illustration apps: that needs unsafe FFI into
arbitrary machine code and cannot work in the browser build. Instead, plug-ins are **WebAssembly modules** run in a
sandbox. One `.wasm` file works on macOS, Windows, Linux and the web. The design follows PhotoCraft's plug-in API (same
sandbox, same manifest and parameter schema), extended for vector documents.

There are four kinds of plug-in:

- **Object filters** read the selected paths and compound paths (geometry, fills, strokes, gradients, opacity)
  and return the objects to keep, change or add. A run is one undo step, and a run that changes nothing records
  none. Use them for recoloring, path
  clean-up, halftones, generative patterns and the like.
- **Live effects** rewrite one object's geometry each time it is drawn, from its appearance stack, like the
  built-in Distort & Transform effects. The art stays editable: move or reshape it and the effect follows. A live
  effect may also paint what it returns ([painted live effects](#painted-live-effects)).
- **Exporters** write the whole document in a file format of their own, for pen plotters, cutting machines,
  laser cutters and the like. File › Export › Export As lists them after the built-in formats, and the host writes the
  bytes they return: a plug-in still never touches a file.
- **Importers** read files in a format of their own, such as a plotter's or a cutting machine's, into a new document of
  paths and compound paths, and at API level 4 also of point type, embedded images, layers and artboards ([importer
  output kinds](#importer-output-kinds)). File › Open lists their files, File › Open With lists the importers by name,
  and the host hands them the file's bytes.

This page is the contract for plug-in authors and the reference for the host (`crates/plugins`, the `plugin.*` commands
in `crates/engine/src/cmd/plugin.rs`).

## Using plug-ins

| Command | Params | What it does |
|---|---|---|
| `plugin.list` | `{}` | `{"plugins": [{id, name, version, kind, author, description, params, paramsSchema, effect?, format?, extensions?, budget?, commands?, commandAllowlist?, context?, size, source, api, paintOutput?, outputKinds?, permissions, hash, cacheBytes}], "folder": report}` (`api` is the manifest's [plug-in API level](#permissions); `context` lists the [document information](#document-information) it reads; `budget: "heavy"` for a [heavy filter](#heavy-filters); `commands` lists its [commands](#commands); `commandAllowlist` lists the commands it may [call](#command-calls); `paintOutput: true` marks a [painted live effect](#painted-live-effects); `outputKinds` lists what an importer returns besides paths ([importer output kinds](#importer-output-kinds)); `permissions` lists what it [may do](#permissions), `hash` is the module's SHA-256; `cacheBytes` is the size of its [cache](#settings-and-cache)) |
| `plugin.info` | `{"id"}` | One plug-in: the `plugin.list` entry plus `manifest`, `defaults` and `lastError` (a live effect's last failure). |
| `plugin.run` | `{"id", "params"?: {…}, "ids"?: [...]}` (plug-in params may also be top-level keys) | Runs an object filter on the selection (or `ids`; an `ids` that is not a list of object ids is an error). One undo step; the result is selected, except the objects it locked or made guides. An output that changes no object and none of the data it keeps with the document ([`documentData`](#output)) records no undo step and leaves the document unmodified. |
| `plugin.install` | `{"path": "/x/y.wasm"}` (desktop) or `{"dataBase64": "…", "name"?}`, `"replace"?: bool`, `"allowHeavy": bool = false`, `"grant"?: [permission]`, `"hash"?: "sha256 hex"` | Validates and installs a module; returns its `plugin.list` entry. The desktop app and `vectorcraft-cli` keep it in the `Plug-ins` folder next to the preferences, so it is installed again after a restart (#2541). A module with other bytes than the plug-in installed with its id, or, with none installed, one whose id has [settings and a cache](#settings-and-cache) in the `Plug-ins` folder, installs only with `replace: true`, once the user agreed; it takes that plug-in's place and gets its settings and cache. The same module installs again without `replace`, and `replace: false` refuses a module whose id is installed. A [heavy filter](#heavy-filters) installs only with `allowHeavy: true`. A plug-in whose manifest lists [permissions](#permissions) installs only when `grant` lists every one of them, unless a grant is already recorded for a module with the same SHA-256. A request's `grant` and `allowHeavy` are refused except in a session started with `vectorcraft-cli --allow-plugin-grants`. With `hash`, a module whose SHA-256 differs is refused. A module whose `commandAllowlist` lists a command no plug-in may call is refused; ids no command has are listed as `unknownCommands` ([command calls](#command-calls)). |
| `plugin.inspect` | as `plugin.install`: `{"path"}` or `{"dataBase64", "name"?}` | Validates a module without installing it; returns the `plugin.list` entry it would have, plus `needsApproval` (true when installing it needs a person's approval), `needsReplace` (true when `plugin.install` needs `replace: true`), and the `commandAllowlist` ids no command has (`unknownCommands`) and those no plug-in may call (`refusedCommands`). |
| `plugin.remove` | `{"id"}` | Uninstalls, and deletes it from the `Plug-ins` folder with its [settings and cache](#settings-and-cache). |
| `plugin.clearCache` | `{"id"?}` | Deletes every entry in the [cache](#settings-and-cache) of plug-in `id`, or of every installed plug-in; settings stay. Returns `{"cleared": [id], "bytesFreed"}`. |
| `plugin.reload` | `{"path"?: "folder"}` (default: the preference folder) | Installs every `*.wasm` in a folder, the first 1,000 by file name; a bad module is reported and skipped, and so are an entry that is not a regular file, a heavy filter and a plug-in whose permissions the user hasn't granted (nobody was asked). Returns `{"folder", "loaded", "failed": [{file, error}], "leftOut"}`; `leftOut` counts the `*.wasm` files past the first 1,000, which were not tried. |

Live effects are applied like any effect: `effect.apply {"effect": "plugin.<id>", "params": {…}}`, edited with
`effect.setParams`, listed by `effect.list` (menu `Effect › Plug-ins`) and baked by Object › Expand Appearance.

An exporter adds a write-only format, `plugin.<id>`, which `document.formats` lists with its `plugin` id.
`document.export {"format": "plugin.<id>", "params"?: {…}, "path"?}` writes it, and so does a `path` ending in one of
its extensions that no built-in format has; `selectedOnly` hands it the selection, as for the built-in formats.

An importer adds a read-only format, `plugin.<id>`, which `document.formats` lists with its `plugin` id. `openExtensions`
holds the extensions File › Open opens with the importer (the lists below describe which are left out). `document.open
{"path"}` (or `{"name", "dataBase64"}`) opens a file with it when no built-in format reads the file, by its content or
by its extension, and the importer lists the file's extension; when several do, the one whose id sorts first opens it.
`document.open {"importer": "plugin.<id>", "params"?: {…}}` names the importer, which must list the file's extension,
also one a built-in format reads (an importer that lists `svg` opens an `.svg` only this way). A damaged file of a
built-in format reports that format's error, and the file is not handed to an importer. The importer's objects become a
new document whose artboard is their bounds (at least 1 pt wide and high, and within ±4,000,000 pt); the document keeps
no path, so Save asks for a name. A file only an importer reads opens as a new document when it is dropped on the
canvas, unless an importer that places its files reads it (below). These rules are VectorCraft's own.

An importer whose manifest has `"place": true` also places its files. `document.formats` and `plugin.list` mark it with
`place: true`. `file.place`, `file.place.info` and `file.place.queue` read a file with it by the rules above, among the
importers that set `place`: when no built-in format reads the file, the first such importer by id that lists its
extension reads it, and `"importer": "plugin.<id>"` names one, which must set `place` and list the file's extension,
also one a built-in format reads. `"params"?: {…}` holds its parameters, which take their defaults when left out. The
importer is given at most 128 MiB of the file and `_context.mode` `"place"`, and returns its objects as it does for
File › Open. They are placed in the open document as
one group named after the file, in the order the importer returned them, at the place, size or object `at`, `rect`,
`replace` and `template` give, as one undo step; guides are left out, and the group's box is the bounds of what the
other objects draw. The file is embedded whatever `link`
says, as a format that can't be linked is, and the reply's `linked` is `false`. A decline, invalid output, a trap or
Cancel places nothing and leaves the document and its undo steps as they were. An importer without `place` is not used
by File › Place, and naming it is an error. A larger file is refused from its size. With `importer`, a `path` is refused
before any of it is read and `dataBase64` before any of it is decoded. Without `importer`, the first 64 KiB of the file
are read or decoded first to look for a built-in format. The web's File › Place reads the picked file in the browser
before the 128 MiB limit applies. In the app, File › Place lists a placing importer's extensions under "All placeable
files" and a filter of its own, named after the importer. Extensions that a built-in format reads are left out of both,
and an importer that lists only such extensions gets no filter. On the web, a dropped file that no built-in format
reads, by its extension or its content, is placed on the canvas by the first placing importer by id that lists its
extension (the desktop opens a dropped document, as it does an SVG); a dropped file that a built-in format reads goes
to that format. The importer then runs with its parameters' defaults. A file placed with `importer` goes to that
importer without the import options File › Place shows for a text file, a DXF drawing, a PDF or a PSD file. These rules
are VectorCraft's own.

In the app, installed filters appear under **Object › Plug-ins**, next to **Install Plug-in…** and **Reload
Plug-ins**; live effects under **Effect › Plug-ins** and in the Appearance panel's effect menu. A plug-in with
parameters gets a dialog generated from its schema (numbers, whole numbers, checkboxes, dropdowns, text, lists
of numbers and colors, labelled, explained and sorted into sections as its manifest says) with a live preview; without
parameters it runs at once. A filter's dialog opens with the parameters that filter last ran with in the session,
else with its defaults. Object › Plug-ins starts with **Apply Last Filter** (`plugin.applyLastFilter`), which runs
the filter that last ran again with the same parameters on the selection, without its dialog, as one undo step, and
**Last Filter…** (`plugin.lastFilter`), which opens that filter's dialog with those parameters, its OK running it as a
new run. Once a filter has run they read **Apply** followed by the filter's name and the filter's name with an
ellipsis. A run counts once it succeeds: from its menu item, its dialog's OK, a shortcut, `plugin.run` or a
`command.batch` step (each such step in the order the batch ran them, so the last one runs again), an Action Batch (`action.batch`) step once a file or data set
ran it, and a background run once its
result is applied. A preview, Cancel, a failed or canceled run, a plug-in's own commands and live effects don't count,
and neither does a batch step whose filter the batch installed when a later step of the batch installs or removes
that same plug-in, or reloads plug-ins. Both items are grayed out with no document open, before any filter ran, and once the filter that
ran last was removed, installed again (the same module too) or no longer takes those parameters as they are;
installing another plug-in keeps them. They don't need a selection: a filter given no objects can still add new ones. The parameters are
kept for the session only, for at most 64 filters. Where the two items sit, when they are grayed out and how long the
parameters are kept are VectorCraft's own rules. Edit › Keyboard Shortcuts lists each installed filter under Object ›
Plug-ins, where
it can be given a shortcut (`plugin:<id>`); the shortcut stays saved while the plug-in is removed and applies again
when it is reinstalled. Parameters too many for the window's height scroll inside the dialog, which keeps its
heading and buttons in view. Exporters are formats of **File › Export › Export As**, where Use Artboards is greyed out
(the plug-in writes the whole document); one with parameters asks for them in a dialog of the same kind once the file is
picked, and its Export button writes the file. Importers' files are listed under File › Open's "All readable files" and
under a filter of their own, named after the importer (other open dialogs, such as New from Template and Load Styles,
list only the files the built-in formats read). These lists leave out an importer's extensions that a built-in format
reads, and those File › Open installs, imports or loads as something else (`.wasm`, `.kys`, `.jsx`, presets, libraries
and swatch or graphic style libraries such as `.ase`). File › Open does not give those files to the importer, and Open
With does. **File › Open With** lists the installed importers by name (`file.openWith {"importer", "path"?,
"params"?}`), and each opens a file picked among its extensions with that importer (on the web, File › Open's picker
opens instead) and adds it to Open Recent. On the desktop the importer runs off the UI thread, and a run that takes a
while shows a progress window with Cancel, which stops it. An importer with parameters, given no `params`, asks for them
in a dialog of the same kind once the file is picked, and its Open button opens the file; File › Open runs an importer
with its parameters' defaults. Open Recent opens a file the way File › Open does: it does not remember the importer or
the parameters given to Open With. A built-in format reads the file if one can, and otherwise the first importer by id
that lists its extension reads it with its parameters' defaults. **File › Open** of a `.wasm` file installs a plug-in
too (on the web this is how plug-ins get in). A heavy filter asks before it is installed, and runs in the background
([below](#heavy-filters)). A plug-in that lists permissions asks for them before it is installed
([below](#permissions)).

### Commands

A filter or a live effect may declare commands of its own in its manifest (`commands`, [below](#manifest)). Each
becomes the command `plugin.<id>.<command>` while the plug-in is installed, and is gone once the plug-in is removed or
replaced by a module without it.

- **Menus.** A plug-in with commands is a submenu of **Object › Plug-ins** (a live effect: **Effect › Plug-ins**)
  named after the plug-in. The submenu holds the plug-in's own item, then its commands. A command with parameters has
  an ellipsis and opens a dialog for them, with a live preview, as a filter's own item does. A command whose manifest
  gives a `menu` is listed in that menu instead ([below](#placing-a-command-in-another-menu)).
- **Keyboard Shortcuts.** Edit › Keyboard Shortcuts lists each command under its menu, and the user can give it a
  shortcut. A manifest cannot give a command a shortcut.
- **Actions and agents.** A command run is recorded in an action like any other command and plays back with its
  parameters. `run`, MCP and `command.batch` call it as `plugin.<id>.<command>` with `{"ids"?: [...], "params"?: {…}}`
  (its parameters may also be top-level keys). MCP's `command_list` and `list_commands` list it, and `plugin.list`
  lists a plug-in's commands as `commands: [{id, command, label, params, paramsSchema, menu?}]`, with `menu` for a
  command whose manifest places it in another menu.
- **Runs.** The parameters are checked against the command's declarations before the plug-in runs. The plug-in runs
  through `vc_run` on the selection (or `ids`) as a filter run does, with `_context.mode` `"command"` and
  `_context.command` set to the command's id ([Parameters](#parameters)), and with the [document
  information](#document-information) its manifest's `context` lists, under the plug-in's
  [permissions](#permissions): a live effect with commands may also hold `doc.write` and `state`, which only its
  commands use. `ids` left out or null means the selection; an `ids` that is not a list of object ids is an error. One run
  is one undo step named after the command, and a run that changes nothing records none. A run that traps, runs out of its instruction budget or fails leaves the
  document and the undo history as they were, and the error starts with the plug-in's name and the command's label
  (`“Cmd Test” › Grow: …`).

#### Placing a command in another menu

At `api` 2 a command may name the menu it is listed in with `menu`: `{"path": [menu, …], "after"?: item}`. For
example, `"menu": {"path": ["Select"], "after": "Inverse"}` lists the command in the Select menu right after
Inverse.

- `path` is one of these menus, by their English labels: `["Edit"]`, `["Edit", "Edit Colors"]`, `["Object"]`,
  `["Object", "Transform"]`, `["Object", "Arrange"]`, `["Object", "Path"]`, `["Object", "Compound Path"]`,
  `["Object", "Clipping Mask"]`, `["Type"]`, `["Select"]`, `["Select", "Same"]`, `["Select", "Object"]`, `["Effect"]`
  and `["View"]`. The File, Window and Help menus, the application menu, Edit › Preferences and the submenus that list
  files, fonts, panels or plug-ins cannot be named. Any other `path`, a `menu` that is not an object or is given twice,
  a key other than `path` and `after`, and an `after` that is not 1–64 printable characters are refused when the
  module is installed, and the error names the command.
- `after` is the English label of a built-in item or submenu of that menu, with or without its closing ellipsis
  (`"Inverse"`). The command is listed right after it. An item that switches between two labels is matched by either
  of them (`"Show Edges"` and `"Hide Edges"`, `"Start Global Edit"` and `"Stop Global Edit"`). Without `after`, or when no
  built-in item of the menu has that label, the command is listed at the end of the menu, and the module still
  installs. Items of plug-ins are never matched.
- The item's label is the command's label followed by the plug-in's name in parentheses (`Low Area Closed Paths
  (Tidy)`). Keyboard Shortcuts, the command palette and `command_list` list the command under that menu with that
  label.
- A command stays in its plug-in's submenu, under its own label, when its label with the plug-in's name has the same
  letters and digits, ignoring case, as a built-in item or submenu of that menu (`Paste (in Place)` and Edit › Paste
  in Place, `Hide (Edges)` and View › Hide Edges; either label of one that switches between two), or as a command that
  a plug-in with a lower id, or an earlier command of the same plug-in, places in that menu. The desktop app and
  `vectorcraft-cli` give the same result. `plugin.list` then gives the command's `menu` with
  `"placed": false`.
- Commands of several plug-ins placed at the same spot are listed in plug-in id order, then in manifest order. A
  plug-in whose commands all name another menu is listed in Object › Plug-ins (or Effect › Plug-ins) as a single item,
  without a submenu. Commands without `menu` stay in the plug-in's submenu.
- A placed command leaves the menus when its plug-in is removed or replaced by a module without the command. Turning
  the plug-in off (`plugin.setEnabled`) removes it at the next launch.
- At `api` 1 a `menu` key, repeated or not, is ignored and the command stays in the plug-in's submenu.

**Preference folder.** Edit › Preferences › Plug-ins & Scratch Disks › *Additional Plug-ins Folder*: when set, every
`*.wasm` in that folder (not recursive) is installed at startup and whenever the preference changes (desktop builds;
the web build installs from bytes). On the desktop, Choose… beside the field picks the folder; the dialog refuses a
picked or typed folder whose contents can't be listed, and `prefs.set` refuses such a folder too. A folder load reads at
most 1,000 `.wasm` files, the first ones by file name (VectorCraft's own limit); its report counts the files left out
(`leftOut`). An entry that is not a regular file, such as a subfolder or a named pipe with a `.wasm` name, is listed in
the report and skipped, and the load does not wait on a named pipe.

**Loading.** On the desktop, the plug-ins in the `Plug-ins` folder and the Additional Plug-ins Folder load on up to four
worker threads and install in file-name order, whatever order their loads finish in; at most eight loaded modules wait
to be installed at once. When the desktop app starts, its window shows while they load, and they install as their loads
finish. Plug-in commands, commands that edit a document, exports, File › Open, control-channel requests and files
opened at launch wait for them first; view commands and queries chosen from a menu do not. Until then a live effect
whose plug-in is still loading is drawn as if it weren't there, and another window's `plugin.list` reports the folder
as `{"folder", "loading": true}`. A folder set later in Preferences, `vectorcraft-cli` and `plugin.reload` wait for
the loads before they go on. A module whose load panics is reported as failed or skipped with a log record, and the
others install. So is a module whose read and load take 10 s or longer (VectorCraft's own limit), also one that would have
finished later, such as a module on a slow network share. The launch marker names every module still loading, so when
the app stops during the loads, the next launch's Safe Mode choice lists all of them. A module whose load panicked or
failed is no longer named; one past its 10 s limit is named until its load returns. A module is also skipped when the
plug-in with its id was installed or removed in another window while it loaded. A module from the `Plug-ins` folder
installs only when its file, read again on the worker thread after the module loaded, still holds that module, whatever
the file's modified time. When a file fails because no thread was left to load it, the Additional Plug-ins Folder is
loaded again the next time the preferences are applied. When the app quits while plug-ins are
loading, it waits for the modules already loading, for at most 10 s, before it exits. The web build loads one
module at a time.

Plug-ins are installed **per process**, not per document. Documents store a live effect as an ordinary effect record
(`{"id": "plugin.<id>", "params": {…}}`): while the plug-in is installed it is drawn, exported and expanded; without it
the object is drawn as if the effect weren't there, and the record is kept for when the plug-in comes back.

## The sandbox

The host is [`wasmi`](https://github.com/wasmi-labs/wasmi), a pure-Rust WebAssembly interpreter (MIT OR Apache-2.0). It
builds for `wasm32-unknown-unknown` too, so plug-ins also run in the web build. Limits (`vectorcraft_plugins::Limits`,
defaults):

| Limit | Default | On violation |
|---|---|---|
| Host imports | none at API level 1; at level 2 only the [host imports](#host-imports-api-level-2): no WASI, file system, network, clock or randomness | module rejected at install, naming the import |
| Module size | 32 MiB | rejected |
| Linear memory | 512 MiB per instance (input and parameters: at most half) | `memory.grow` returns -1; oversized input or initial memory fails |
| Output | 64 MiB of JSON | `Limit("output size")` |
| Instructions (fuel) | 50 M per call + 4,000 per input byte for `vc_run`; an object filter with the [Large instruction budget](#instruction-budget-preference) 250 M + 4,000 per byte; a [heavy filter](#heavy-filters) 1 G + 4,000 per byte; an export or import 10 G + 4,000 per byte; [storage calls](#settings-and-cache) take one more for every 64 bytes they move | `Limit("instruction budget")` |
| Wall clock | 60 s per filter run, 5 s per live-effect run, 300 s per heavy filter run, export or import (checked every 20 M instructions and after each [storage call](#settings-and-cache); the web build reads the page's `performance.now()`) | `Limit("time budget")` |
| Import input | 128 MiB of the file (`vectorcraft_plugins::import::MAX_INPUT_BYTES`) | the file doesn't open; a larger file named by `path` is not read past that |
| Call depth | 1,024 frames | trap |
| Module shape | wasmi's strict limits (≤ 10,000 functions, 1 memory, …) | rejected |

An export runs where the export runs: with Background Export on, on the export's worker, whose Cancel stops the
plug-in between two of those 20 M-instruction slices or after a storage call; nothing is written then. An import stops the same way when the
open it runs in is cancelled, and nothing opens then.

### Instruction budget preference

Preferences › Plug-ins & Scratch Disks › Plug-in Instruction Budget (`pluginInstructionBudget`, VectorCraft's own
setting) sets the instructions each object filter run gets, from the app, `plugin.run`, a plug-in's commands, scripts
and MCP alike:

- **Normal** (`normal`, the default): the limits in the table above, 50 M + 4,000 per input byte.
- **Large** (`large`): 250 M + 4,000 per input byte (`vectorcraft_plugins::LARGE_FUEL`, 5 times Normal). The 60 s time
  budget, the memory, output and call depth limits stay as they are.

A [heavy filter](#heavy-filters) keeps its 1 G and 300 s with either choice. Live effects, exporters and importers keep
their own budgets. A change applies to the next run of every installed filter, without installing it again. Cancel and
the time budget stop a run between two 20 M-instruction slices, or after a storage call, with either choice. `prefs.set {key:
"pluginInstructionBudget", value}` takes `normal` or `large` (or the labels Normal and Large) and refuses any other
value; a saved preferences file with another value reads as Normal.

### Live effects while a document opens

While the app draws a document on screen, the live-effect runs of its first view share a 2,000 ms budget
(`vectorcraft_plugins::deferred::OPENING_BUDGET_MS`, VectorCraft's own value), measured from the first run. Each run
takes the lesser of its 5 s limit and what is left of the budget; once the budget is spent no run starts, and a result
that comes back after it is not used. A stage left out this way, or one that times out, traps, returns output the host
refuses or has no plug-in, draws its input art unchanged and is deferred. The status bar then says how many stages
are deferred, and later drawings leave them as they are until their input or parameters change. Effect › Refresh
Deferred Effects (`effect.refreshDeferred`) runs them again with a fresh budget, and `effect.deferredStatus` lists
them with the reason (`budget`, `timeout`, `missing`, `failed`, `safeMode`). Each document has its own budget.
Exports, Expand Appearance and commands run every stage under the 5 s limit. The web build has the same budget,
measured with the page's `performance.now()`.

### Heavy filters

Some filters do heavy but legitimate work, such as routing a plotter's pen, packing shapes onto a sheet or running a
simulation, which a minute and 50 M instructions don't cover. Such a filter declares `"budget": "heavy"` in its
manifest (only a filter may: a live effect runs whenever the document is drawn, an exporter has an export's budget and
an importer an import's). Each of its runs gets 1 G instructions (20 times a light run's) plus the 4,000 per input byte,
and 300 s (`vectorcraft_plugins::{HEAVY_FUEL, HEAVY_WALL_TIME_MS}`). Its output can't add to the budget: the host learns
how much there is only once the run has ended.

- **Installing asks first.** Install Plug-in… and File › Open show what the filter asks for, and install it once the
  user clicks OK (see [Permissions](#permissions) for who can). `plugin.install` refuses it without `allowHeavy: true`,
  and refuses `allowHeavy` from a request except in a session started with `vectorcraft-cli --allow-plugin-grants`
  (`plugin.inspect` shows the budget beforehand). A filter kept in the `Plug-ins` folder was agreed to and is installed
  again at startup. The Additional Plug-ins Folder skips heavy filters with a note in its report, since loading a folder
  asks nobody.
- **It runs in the background.** Its dialog has no Preview. OK (or the menu item, without parameters) runs it on a
  thread of its own while editing goes on, shown in the status bar and, after a moment, in a progress window with
  Cancel (also Esc) and Continue in Background (`plugin.cancelRuns`, `plugin.hideRuns`). When it is done its result is
  applied as `plugin.run` with the same parameters and objects, as one undo step, unless the document or the plug-in
  changed meanwhile: then nothing changes, and the status bar says to run it again. On the web, which has no threads,
  it runs at once, under the same instruction budget and 300 s limit.
- `plugin.run` itself runs a heavy filter where it is called, as it runs any filter: an MCP client waits for it, and
  its `notifications/cancelled` stops it.

### Permissions

A manifest may list what the plug-in may do in `permissions`. `doc.read`, `doc.write` and `state` follow the
capability model proposed for PhotoCraft plug-ins
([storytold/photocraft#662](https://github.com/storytold/photocraft/issues/662)); the other names are VectorCraft's own:

| Permission | Kinds | What it allows |
|---|---|---|
| `doc.read` | filter, effect, exporter | Read the objects it is given, and the [document information](#document-information) its `context` lists. |
| `doc.write` | filter, effect with `commands` | Change, add and delete objects in the document. |
| `state` | filter, effect with `commands` | Keep data of its own with the document (`documentData`). |
| `effect.render` | effect | Change how the objects its effect is applied to are drawn. |
| `export.write` | exporter | Write the document as a file in its own format. |
| `import.read` | importer | Read files you open in its format and make documents from them. |
| `progress` | all | Show its progress, see Cancel and write log lines through the [host imports](#host-imports-api-level-2). Needs `api` 2. |
| `settings` | all | Keep settings of its own between runs and sessions ([settings and cache](#settings-and-cache)). Needs `api` 2. |
| `cache` | all | Keep a cache of its own, which the user can clear ([settings and cache](#settings-and-cache)). Needs `api` 2. |
| `commands` | filter | Run the commands its `commandAllowlist` lists on the document ([command calls](#command-calls)). Needs `api` 2. |

- A manifest without `permissions` gets the ones its kind implies, as plug-ins did before the key existed: a filter
  `doc.read`, `doc.write` and `state`; an effect `doc.read` and `effect.render`, and also `doc.write` and `state`
  when it declares [commands](#commands); an exporter `doc.read` and `export.write`; an importer `import.read`. Such a
  plug-in installs without a prompt. This rule is VectorCraft's own. A filter whose list is empty may do none of
  these, and also installs without a prompt.
- An effect, an exporter or an importer that lists permissions lists all of its implied ones (`doc.read` and
  `effect.render`; `doc.read` and `export.write`; `import.read`). An effect with commands may add `doc.write` and
  `state`, which only its commands use. A filter may list any of its three, and `commands` with a `commandAllowlist`.
  Any kind may add `progress`, `settings` and `cache`, which no kind implies. An unknown name, a name listed twice, a
  name that doesn't apply to the kind, more than 16 names, `progress`, `settings`, `cache` or `commands` with an `api`
  below 2, or one of the names kept for a later plug-in API (`ui.panel`, `ui.tool`) is refused when the module is
  installed, and the error names it.
- **Installing asks first.** Install Plug-in… and File › Open show the plug-in's name, id, version, the SHA-256 of
  the file and each permission in plain words, and install it once the user clicks OK, if the file still has the
  SHA-256 shown; Cancel installs nothing. The names shown have control and bidirectional characters escaped (`\u{202E}`)
  and are cut to 64 characters. Only a person can press its OK: `ui.dialog.confirm`, MCP's `answer_dialog` and injected
  input (`ui.key`, `ui.click`) are refused and the dialog stays open. Enter confirms it only when OK has the keyboard
  focus. `ui.dialog.set` can't change what it installs. OK stays disabled until the question has been on screen in the
  focused window for half a second (VectorCraft's own value, from the delay the security guidance at
  https://chromium.googlesource.com/chromium/src/+/refs/heads/main/docs/security/security-considerations-for-browser-ui.md
  gives for dialog boxes), so a click, Enter or Space, or a press begun before then, installs nothing. The half second
  starts over for every new question, even one that reads the same, and when the window gets the focus back. Cancel
  and Escape work at once. A module that would take the place of the installed plug-in with its id asks in the same
  dialog.
- **An install that automation asked for is not approved in its dialog.** When a control request, MCP, injected input,
  or a file dialog that one of them opened (even when the person picks the file there) leads to the approval dialog, its
  OK stays disabled, a confirm is refused even after the half second, and the dialog says that automation asked for the
  install. `ui.installPlugin` returns `{pending: "pluginInstall", approval: "personMustStartInstall"}`. Cancel still
  closes it. To install the plug-in, the person chooses Object › Plug-ins › Install Plug-in… and picks the file, or
  opens it with File › Open or drops it on the window: the module is inspected again and the new question waits its
  half second. A request's `replace: true` does not skip asking about taking another module's place. A module that
  needs no approval still installs at once from any request. This is VectorCraft's own rule.
- `plugin.install` refuses the module, naming the permissions it needs, unless this module already has a recorded grant
  (`plugin.inspect`'s `needsApproval` tells it beforehand). A request's `grant` is refused too: `plugin.install` takes
  it only with the approval the dialog gives. A `vectorcraft-cli mcp` or `run` started with `--allow-plugin-grants`
  takes `grant` and `allowHeavy` from any request; only the person starting the process can set that flag, and it
  doesn't apply to a running app. `mcp` refuses the flag together with `--automation-read-root` or
  `--automation-write-root`: a server confined to folders has no plug-in store, so it reads no `grants.json`, and a
  module that lists permissions or asks for a heavy budget does not install there. Without the flag, MCP in headless
  mode and the CLI install a module that lists permissions only when its hash is recorded in the shared `grants.json`.
  A person's approval in the desktop app records it, and so does a `grant` accepted under `--allow-plugin-grants`. A
  recorded grant counts as approved in later sessions without the flag and in the desktop app. With
  `VECTORCRAFT_NO_PREFS` set there is no store, and a `grant` accepted under the flag lasts for that session only.
  With `--allow-plugin-grants`, pass `hash` set to the `plugin.inspect` entry's `hash` so that the module installed is
  the one the user saw.
- The approval protects against plug-ins and automation clients (MCP, scripts and CLI requests): none of them can
  approve a module's permissions. VectorCraft's own code records a grant only after that approval and when it loads
  `grants.json`, and installs a module that lists permissions only under a grant recorded for this very module,
  also when it loads the Additional Plug-ins Folder. The approval does not protect against native code linked into
  VectorCraft or against another program running as the same user, which can change the files in the `Plug-ins`
  folder. `grants.json` is not signed, so such a program can add a grant to it. `--allow-plugin-grants` is a choice
  the person starting a headless session makes.
- **Replacing asks first.** When the file is another module than the plug-in installed with its id, or, with none
  installed, its id has settings and a cache in the `Plug-ins` folder, Install Plug-in… and File › Open show its id
  and SHA-256 and ask whether to install it in place of that plug-in, which gives it the settings and cache kept for
  the id. When the same module also needs its permissions or a heavy budget approved, the one approval dialog asks
  about all of them, and only a person can press its OK. `plugin.install` refuses such a module, naming it, unless
  `replace` is `true`, which an agent passes once the user agreed (`plugin.inspect`'s `needsReplace` tells it
  beforehand). The Additional Plug-ins Folder and `plugin.reload` do not ask.
- The grant is recorded with the plug-in's id, the module's hash and the permissions, in `grants.json` in the
  `Plug-ins` folder, and covers only that module. Installing the same bytes again needs no new approval. A module with
  other bytes asks again, even when it lists the same permissions, and does not get the id's settings and cache until
  it is approved. Until then, the installed module and its grant stay as they are; once it is approved, the grant
  records the new hash. When the new module or its grant can't be written to the `Plug-ins` folder, the folder keeps
  the old module and its grant, and the old module loads at the next start. If the write that puts the old module back
  fails as well, the folder holds the new module with the old grant, the grant does not cover that module, and the
  plug-in is skipped at the next start until it is approved again. When the folder held no module for that id, that
  write removes the new module; when the removal fails, the folder holds the new module and no grant covers it. In
  both cases a disabled plug-in is turned on, as for any install the folder holds. A grant whose hash is not 64 lower-case
  hexadecimal digits covers nothing. `plugin.remove` forgets the grant, and so does a successful `plugin.install` of a
  module that lists no permissions under that id, so the earlier bytes need a new approval. When `plugin.remove` can't
  remove the grant from `grants.json`, it returns an error, the plug-in stays installed, and a second `plugin.remove`
  tries again. A refused install leaves the grant, the plug-in installed with the id, the `Plug-ins` folder and the
  list of disabled plug-ins as they are. When another session installs a plug-in with the id or changes its grant
  while `plugin.install` checks a module, the install is refused, before anything is written to the `Plug-ins` folder,
  if the other plug-in would need `replace: true` or the changed grant no longer approves the module. A
  `plugin.remove` in another session waits until the install has written the `Plug-ins` folder, and then removes the
  module and its grant. Loading the `Plug-ins` folder or a plug-ins folder in another session waits for it too.
  Another process using the same `Plug-ins` folder, for example `vectorcraft-cli` beside the app, waits as well: each
  window or process locks `disabled.lock` in the folder while it reads and changes the stored modules, `grants.json`,
  `disabled.json` and the plug-ins' settings and caches. An install therefore keeps a module with its own grant, and a
  grant or setting written in one process does not undo one written in another. When `disabled.lock` can't be
  locked, `plugin.install` and `plugin.remove` fail and change nothing, `plugin.setEnabled` keeps its change in the
  process until a later change can be saved, and loading the folder writes nothing to it. When
  an install of a module that lists no permissions can't
  write the module to the `Plug-ins` folder, or can't remove the grant from `grants.json`, the folder keeps the module
  it held and `grants.json` keeps the grant. The new module runs until the session ends. When the grant can't be
  removed and the module already stored can't be read back or written back, the new module stays in the folder, and
  the grant stops counting. The grant also stops counting when the folder held no module for that id. A
  `grants.json` larger than 1 MiB, or one that is not a JSON list, grants nothing: the new module stays in the folder,
  and `plugin.remove` removes the plug-in. A `grants.json` that can't be opened or read counts as a grant that can't be
  removed. When VectorCraft loads the `Plug-ins` folder, it forgets any grant for an id whose stored module lists no
  permissions. When it can't remove that grant from `grants.json`, or can't read the file, the grant stops counting,
  including for the next load, which tries the removal again. When a later file in the folder holds a module with the
  same id that the forgotten grant covers, the grant is written back and that module installs. When the grant can't be
  written back, that module is skipped, and when the module doesn't install, the grant is removed again. A grant that
  stops counting in either of these cases
  does not count for any session in the same process that uses the same `Plug-ins` folder, by any path to it, until
  `grants.json` can be written for that plug-in, as when the plug-in is granted again. Another process on the same
  folder, for example `vectorcraft-cli` beside the app, still reads `grants.json` as it is written. A grant for the
  same id kept in another `Plug-ins` folder counts. In Safe Mode, and for a stored module that the user disabled,
  VectorCraft does not read the module, and `grants.json` keeps the grant for its id. The Additional Plug-ins Folder
  and `plugin.reload` do not change the `Plug-ins` folder: when they leave a module that lists no permissions under an
  id, the process forgets its own grant for that id, and a grant kept in `grants.json` stays with the stored module it
  covers. An entry of `grants.json` that this version can't use (a permission it doesn't know, an invalid id) is
  skipped and stays in the file until its plug-in is granted again, replaced by a module that lists no permissions, or
  removed. When such an entry can't be removed from the file, `plugin.install` and `plugin.remove` treat it as a grant
  that can't be removed. A `grants.json` that is not a JSON list is never overwritten, and the file is never written
  larger than 1 MiB, the most that is read. When a `Plug-ins` folder is in use, only the grants kept in it count for
  `plugin.install`, `plugin.inspect`, `plugin.reload` and the Additional Plug-ins Folder. At startup, a stored plug-in
  is skipped when no grant covers its bytes and the permissions it lists. The Additional Plug-ins Folder and
  `plugin.reload` install only plug-ins that such a grant covers, and report the others. A module without a
  `permissions` list, or with an empty one, needs no grant.
- The host enforces the grant on a filter run and a command run: without `doc.read` the filter is given no objects;
  without `state` its `documentData` is `null` and an output that sets it is refused; without `doc.write` an output that
  changes the document is refused. A refused output leaves the document as it was.
- `api` is the plug-in API level the manifest is written for: `1` (the default); `2`, which adds the [host
  imports](#host-imports-api-level-2), a filter's `inputs` ([reading type, images and
  symbols](#reading-type-images-and-symbols)) and a live effect's [`paintOutput`](#painted-live-effects); `3`, which
  adds `context` ([document information](#document-information)); or `4`, which adds an importer's
  [`outputKinds`](#importer-output-kinds). A higher one is refused. `plugin.list` and
  `plugin.info` give it as `api`. `inputs` is not a permission: the objects it asks for are read under `doc.read`, which
  a filter that lists `inputs` must list in `permissions`, so installing it asks the user to grant them. `context` is
  not a permission either: it is read under `doc.read` in the same way, and the install prompt says that the plug-in
  reads the document's settings, artboards, layers and swatches. A level 2 effect that declares `paintOutput` needs no
  other permission than `doc.read` and `effect.render`.
- Parameter declarations are unchanged by permissions (`params`, below).

Every run gets a **fresh instance**, so no state survives between calls except what a plug-in writes to its
[settings and cache](#settings-and-cache). Traps, exhausted budgets, malformed modules, bad manifests, out-of-range
pointers, negative return codes and invalid output all become errors, and the document is left untouched: a filter's
output is checked completely before one edit applies it. A live effect that fails draws the object's geometry unchanged;
`plugin.info` reports why (`lastError`). A run that succeeds clears the error of a failed run. A check that finds every
swatch and pattern a painted result names clears only the error of a refused swatch or pattern.

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

A plug-in is a core WebAssembly module (MVP + these post-MVP features: mutable globals, sign extension, saturating
conversions, multi-value, bulk memory, reference types, tail calls and extended constant expressions) with one memory,
these exports and no imports other than the [host imports](#host-imports-api-level-2) of API level 2. Before wasmi reads
a module, the host validates it with only these features on, and refuses a module that uses any other feature. The error
names the feature for SIMD, relaxed SIMD, memory64, multi-memory, custom page sizes, wide arithmetic, threads,
exceptions, function references and garbage collection. Integers are `i32` unless noted;
pointers are offsets into the module's memory.

| Export | Signature | Meaning |
|---|---|---|
| `memory` | memory | The module's linear memory. |
| `vc_abi_version` | `() -> i32` | Must return `1`. Needed only by a module without a `vc_manifest` custom section. |
| `vc_manifest` | `() -> i64` | `(len << 32) \| ptr` of the UTF-8 manifest JSON (≤ 64 KiB). Needed only by a module without a `vc_manifest` custom section. |
| `vc_alloc` | `(size) -> ptr` | A block of `size` bytes the host may write, or `0` on failure. Never freed: each run gets a fresh instance. |
| `vc_run` | `(input, input_len, params, params_len) -> i64` | Runs the plug-in on the input JSON with the parameters JSON; returns `(len << 32) \| ptr` of the output JSON, or a negative error code. |

### Manifest section

A module may carry its [manifest](#manifest) in a [custom
section](https://webassembly.github.io/spec/core/binary/modules.html#custom-section) named `vc_manifest`, holding the
same UTF-8 JSON that the `vc_manifest` export returns, plus `"abi": 1`, the ABI version that `vc_abi_version` returns.
The host then reads the manifest from the section and loads the module without instantiating it: no start function,
`vc_abi_version`, `vc_manifest` or data segment initialization runs when the module is installed, inspected or loaded at
startup. The host checks the imports against the manifest's permissions, the exports `memory`, `vc_alloc` and `vc_run`
by their types, that the initial memory fits in the [linear memory limit](#the-sandbox), and that the module defines at
most 4 tables of at most 100,000 elements each. The module is instantiated for the first time when it runs, and a start
function then runs at the start of each run.

- A module with more than one `vc_manifest` section, or one longer than 64 KiB, is refused before the section's contents
  are decoded. A section that is not UTF-8, not JSON or not a valid manifest is refused too, and so is one whose
  manifest does not give `"abi": 1`.
- When a module has both the section and a `vc_manifest` export, the section is the manifest and the export is never
  called. Keep both to load in versions of VectorCraft that read only the export.
- A module without the section is loaded as before: the host instantiates it and calls `vc_abi_version` and
  `vc_manifest`.

### Host imports (API level 2)

A plug-in whose manifest gives `"api": 2` and lists the [permission](#permissions) an import needs may import these
functions from module `env` (the module a Rust or C toolchain gives `extern` functions by default). `vc_abi_version`
still returns `1`. `vc_progress`, `vc_cancelled` and `vc_log` need `progress`; the `vc_settings_*` imports need
`settings` and the `vc_cache_*` imports need `cache` ([settings and cache](#settings-and-cache)); `vc_call` and
`vc_call_result` need `commands` ([command calls](#command-calls)).

| Import | Signature | Meaning |
|---|---|---|
| `vc_progress` | `(done: i64, total: i64)` | `done` of `total` steps are finished; `total` 0 or less when not known. Negative values count as 0, and `done` is clamped to `total`. |
| `vc_cancelled` | `() -> i32` | `1` once the user cancelled the run or its time budget ran out, else `0`. |
| `vc_log` | `(level, ptr, len) -> i32` | Writes the UTF-8 text at `ptr` as a log line of `level` (0 error, 1 warning, 2 information, 3 debug). Returns `0` when written, `1` when the run's log budget is spent and the line was dropped, `-1` when the text doesn't lie inside the module's memory, `-2` when it is not UTF-8, `-3` for another level. |

- The host checks the imports when the module is installed: an import it doesn't offer, one with another signature,
  and one the manifest doesn't allow (an `api` below 2, or without the import's permission) refuse the module, and the
  error names the import and the permission.
- A filter's or an exporter's progress is shown as a share: a heavy filter's progress window and the export's progress
  window draw a bar with a percentage, and Cancel there makes `vc_cancelled` return `1`. In the desktop app a light
  filter that lists `progress` runs in the background as a heavy one does, under its light budget, when it runs at once
  from Object › Plug-ins or its shortcut, or when OK runs it with Preview off; with Preview on, its dialog keeps the
  previewed run. A filter that also lists a `commandAllowlist` runs where it is called, since its command calls need
  the document. A plug-in that checks `vc_cancelled` can stop early, and the host discards what it returns after the
  Cancel or after its time budget ran out; one that ignores it is still stopped by the instruction and time budgets.
  With a known total, a report is passed on only when it reaches another tenth of a percent; with an unknown total, each
  change of `done` is passed on. After 2,000 reports in a run, one change in 1,000 is passed on, and so is the first
  report after the total becomes known or unknown. A live effect's progress is not shown, since it is drawn as part of
  something else.
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
The bytes the calls count also cost the run one instruction of its budget for every 64 bytes (VectorCraft's own
value), counted over the whole run, so the 1 GiB costs about 16.8 M instructions. A get also pays at that rate for the
bytes it copies into the module's memory that it didn't count, such as a value answered from memory. A call stops
before a read or write of a file that the run's instructions can't pay for, and the run ends with
`Limit("instruction budget")`; a set or delete may already have read the settings file then. A call takes
instructions beyond the current 20 M-instruction slice only before Cancel and the time budget; after either, a read or
write that needs them doesn't happen. After every storage call the run checks its time budget and Cancel, and stops
if either has come. A read or write that would pass the 1 GiB still returns `-7`, whatever the instructions left.

The app keeps the settings in memory after the first call and reads `settings.json` again only when the file's size
or modified time differs from the last read or write. A get answered from memory reads no file and counts no bytes. A
set to the value the key already has writes nothing, and neither does a delete of a key that has no value. A delete
of the last setting removes the file and counts no written bytes. Every other successful set or delete writes the
file before the call returns. A cache call reads or writes the whole entry in memory in one step, up to 64 MiB, and
the run waits for it. A live effect runs while the art draws, so keep the entries it uses small.

- A key is 1 to 64 characters from `A-Z`, `a-z`, `0-9`, `_` and `-`, and is not a Windows device name (`CON`, `PRN`,
  `AUX`, `NUL`, `COM0` to `COM9`, `LPT0` to `LPT9`, in any case). Keys differ in case: `tile` and `Tile` are two.
- These caps are VectorCraft's own values: at most 256 settings of at most 64 KiB each, in a settings file of at most
  1 MiB; cache entries of at most 64 MiB each, at most 4,096 entries and 512 MiB per plug-in. The cache has no
  automatic eviction: a write past a cap fails with `-4`, and the plug-in or the user clears entries. A cache write
  also fails with `-4` when the `cache` folder holds more than 8,192 items; clearing the cache removes every file in
  it.
- The desktop app and `vectorcraft-cli` keep them in a folder named from the id under `storage/` in the `Plug-ins`
  folder (ids that differ only in case get two folders): `settings.json` and a `cache` folder with one file per entry,
  named by the host. A settings file that is too large or doesn't parse is moved to `settings.unreadable.json`,
  replacing an earlier one there, and the settings start empty. A link in that folder is refused (`-5`). Writes go to a
  temporary file that then replaces the old one; a temporary file a failed write leaves in the `cache` folder does not
  count toward the caps, and clearing the cache removes it.
- A plug-in reaches only the storage of its own id. Replacing it with another module of the same id
  (`plugin.install` with `replace: true`, after the user agreed) keeps the storage; `plugin.remove` deletes it, and a
  run of the plug-in still going gets `-5` from every storage call after that. `plugin.remove` first renames the folder,
  adding `.removed` to its name (or `.removing` while a `.removed` folder holds settings an earlier removal kept). When
  that fails, `plugin.remove` fails and the plug-in stays installed with its storage as it was. Before VectorCraft
  deletes a renamed folder, it writes an empty file beside it with `.delete` added to the folder's name, and removes
  that file once the folder is gone. When that file can't be written, the folder is not deleted and is treated like any
  renamed folder without a `.delete` file (see below). Files that can't be deleted stay in the renamed folder with the
  `.delete` file beside it, where no plug-in reaches them. A later `plugin.remove` tries again to delete them when a
  plug-in with that id is installed or a storage folder for the id exists, and that deletion can fail too. For an id
  that isn't installed and has only such a folder, `plugin.remove` does not delete it and fails as not found. When
  `plugin.remove` can't remove the grant and can't rename the folder back either, its error names the renamed folder.
  The next load, install or `plugin.remove` of the plug-in renames a renamed folder that has no `.delete` file back
  while the plug-in has no folder of its own; when it has one, that folder is used and the renamed folder stays until
  `plugin.remove` deletes both. `plugin.remove` with the id of a plug-in that isn't installed deletes the storage kept
  for that id, when there is any. `plugin.clearCache` empties the cache and leaves the settings, and `plugin.list` and
  `plugin.info` give the cache's size as `cacheBytes`.
- Without a `Plug-ins` folder (the web build, and embedders that attach none) the storage calls return `-5`.

### Command calls

A filter whose manifest lists the `commands` permission and names commands in `commandAllowlist` runs those commands
on the document while it runs (#2186). It names each command by its id, as `command_list` lists it, and passes its
arguments as a JSON object, the params the command takes from any request:

| Import | Signature | Meaning |
|---|---|---|
| `vc_call` | `(id, id_len, args, args_len) -> i32` | Runs the command whose UTF-8 id is at `id` with the JSON object at `args` (`args_len` 0 means `{}`). Returns `0` when it ran, else a negative code. |
| `vc_call_result` | `(out, out_cap) -> i32` | The JSON result of the last call that ran: its length in bytes. When it fits in the `out_cap` bytes at `out`, its bytes are copied there; otherwise nothing is copied. `-10` when no call has a result, `-1` when the buffer doesn't lie inside the module's memory. |

`vc_call` returns `-1` when the id or the arguments don't lie inside the module's memory, `-2` when they are not UTF-8,
`-3` when the arguments are not a JSON object, `-4` when `commandAllowlist` doesn't list the id, `-5` when no command
has the id, `-6` when a plug-in may not call the command, `-7` when the command can't run now (nothing is selected),
`-8` when the command ran and failed, `-9` when a limit below is passed, and `-11` when nothing can be called (from
the module's start function, in a run that isn't a filter's run on a document, or after a call in the run failed).

- Every code also fails the run, whatever the plug-in does next, and the error names the call and why it failed. The
  run's calls and its output are one undo step, named after the plug-in or its command. A run that fails, traps, runs
  out of budget or is canceled leaves every document, its selection and its history as they were.
- Each call runs on the document the run is for, with its params checked as any request's are, and sees what the calls
  before it changed: a `select.all` followed by `object.move` moves everything. The filter's own input is the one it
  was given when the run started; an edit still in progress (typing, or a shape being drawn) is kept as its own undo
  step before that input is made.
- A plug-in never calls a command that reads or writes a file or folder, uses the clipboard, changes preferences or
  the view, runs a script, an action or a plug-in (a `plugin.*` command, or arguments naming one, such as an
  `effect.apply` of `plugin.<id>`), opens, closes or switches documents, or undoes or redoes, whatever its manifest
  lists. The same goes for a command that changes a tool's options, keeps a drag's state or adds font names for the
  whole app, and for a command VectorCraft adds later until it has been checked for plug-in calls. Installing a
  manifest that lists such a command is refused, and the error names it; `plugin.inspect` lists them as
  `refusedCommands`.
- A call whose arguments hold a file or folder key (`path`, `paths`, `file`, `files`, `folder`, `folders`, `dir`,
  `directory`, `destinationFolder`, `sourceFolder` or `url`) at any depth, such as an effect's `textureImage.path`,
  returns `-6`; only a key that names part of the document in that command, as its own param, is let through. A
  command a plug-in calls can't read or write any file or folder.
- An id in `commandAllowlist` that no command has installs, so a plug-in can list a command a later version adds;
  `plugin.install` and `plugin.inspect` list such ids as `unknownCommands`, and calling one returns `-5` (`-6` without
  `doc.read`).
- Without `doc.read`, every call of a listed id fails with `-6`, since a command's result describes the document.
  Without `doc.write`, a call that changes the document fails with `-6`.
- These limits are VectorCraft's own values: at most 1,000 calls a run, 8 MiB of arguments or result a call and 64 MiB
  of arguments and results a run. Each call costs 10,000 instructions of the run's budget, plus one for every 64 bytes
  of its arguments and its result, and the run checks Cancel and its time budget before and after each call.
- A filter that calls commands runs on the app's main thread, so it can't ask for a heavy budget: a manifest with
  `commandAllowlist` and `budget: "heavy"` is refused.

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
- `context` (filters, and live effects with `commands`, at `api` 3 only): the parts of the active document a run reads
  in `_context.document`, any of `"settings"`, `"artboards"`, `"layers"` and `"swatches"`, each at most once
  (`"context": ["artboards", "layers"]`). Left out or `[]`, a run gets none, as before. An unknown, repeated or wrongly
  typed entry, `context` on another kind or on a live effect without commands, below `api` 3, or without a
  `permissions` list that holds `doc.read`, is refused when the plug-in is installed, and so is `api` 3 on hosts from
  before it ([document information](#document-information)).
- `paintOutput` (live effects at `api` 2 only): `true` makes the effect return [painted pieces](#painted-live-effects)
  instead of geometry alone. `true` is the only value the key takes; `false`, `null`, another type, another kind or
  `api` 1 is refused when the module is installed, and the error names `paintOutput`. Without the key the effect returns
  geometry, at any level. Hosts at `api` 1 refuse the manifest for its `api`, so such an effect never draws unpainted
  there.
- `outputKinds` (importers at `api` 4 only): the kinds of content besides paths and compound paths the importer
  returns, any of `"type"`, `"image"`, `"layer"` and `"artboard"`, each at most once (`"outputKinds": ["type",
  "layer"]`). Left out, `null` or `[]`, the importer returns paths and compound paths alone, as before. An unknown,
  repeated or wrongly typed entry, `outputKinds` on another kind, or below `api` 4 is refused when the plug-in is
  installed, and hosts from before level 4 refuse the manifest for its `api`. It needs no permission besides
  `import.read`. See [importer output kinds](#importer-output-kinds).
- `extensions` (exporters and importers only, and required): 1–8 file extensions, lower-case letters and digits
  without the dot (`["plt"]`); the first is the one Export As gives the file. A built-in format keeps an extension both
  list (`svg`): `document.export` then writes the exporter's format only when asked for by its format id, and
  `document.open` reads a file with the importer only when `importer` names it.
- `place` (importers only): `true` lets File › Place read the importer's files too ([using plug-ins](#using-plug-ins));
  `false`, the default, keeps it to File › Open. A value that isn't `true` or `false`, or the key on another kind, is
  refused when the plug-in is installed. It asks for no permission beyond `import.read`: the importer is given the file
  and its parameters alone, and the host places what it returns. Hosts from before the key ignore it, so there the
  importer opens its files and places none.
- `commands` (filters and live effects; not heavy filters): up to 32 commands of the plug-in's own
  ([Commands](#commands)), each `{"id": "grow", "label": "Grow", "params"?: {…}, "menu"?: {…}}`. An `id` is 1–32
  characters from `a-z 0-9 _` that starts with a letter, and appears once. A `label` is 1–64 printable characters;
  menus show it with control and bidirectional characters escaped. `params` are declared as the plug-in's own `params`
  are. `menu` (`api` 2 and later; ignored at `api` 1) places the command in [another menu](#placing-a-command-in-another-menu).
  A malformed entry is refused when the module is installed, and the error names the command by its id (or its place,
  `[0]`).
- `commandAllowlist` (filters at `api` 2 with the usual budget, and only with the `commands` permission, which needs
  it): the ids of the commands the filter may [call](#command-calls), at most 256, each once (`"commandAllowlist":
  ["select.all", "object.move"]`). An id is 1–128 characters from `A-Z a-z 0-9 . _ -` that starts with a letter, with
  no empty part between dots. A malformed list is refused when the module is installed, and the error names the id.
- `params`: in dialog order. Names are `[A-Za-z0-9_]`, not starting with `_`, and not `id`, `ids`, `params` or
  `preview`. The host validates the user's values before calling: missing values take the default, numbers are
  clamped to `min..max` (ints rounded), a wrong type or an unknown choice is an error. Numeric defaults are also clamped
  to their range, including the implicit zero default of an `int` parameter. Besides `number`, `int`, `bool` and
  `choice` there are `string` (`{"type": "string", "default": "", "maxLength": 256}`: a text field; longer text is cut
  to `maxLength`, at most 4,096 characters) and `numberList` (`{"type": "numberList", "min": 0, "max": 360, "default":
  [45, 135], "minItems": 1, "maxItems": 16}`: numbers typed separated by commas or spaces, each clamped to `min..max`; a
  list of too few or too many is an error, and `maxItems` is at most 256) and `color` (`{"type": "color", "default":
  "#3a7bd5"}`: a color button that opens a color picker; the default, black when left out, and a value given to
  `plugin.run` may be a color object as paints hold one or a hex string, and the plug-in always gets a color object, its
  components clamped, such as `{"model": "rgb", "r": 0.23, "g": 0.48, "b": 0.84}`; a hex string and the picker's choice
  are display colors, given as the working RGB color that shows as them, so in a working RGB space other than sRGB the
  numbers differ from the hex). Hosts from before `color` refuse a manifest that uses it. A plug-in declares up to 256
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
symbols](#reading-type-images-and-symbols)). With nothing selected the list is empty (a generator then adds its own
objects). A live effect gets one object: its geometry (`type`, `path`, `fillRule`, `bounds`) and its `fills` and
`strokes`, as a filter gets them (hosts from before these give geometry only). The paints are read only: an effect may
draw as wide as a stroke or use a fill's colour to decide its shape, but it changes only the geometry, unless it
declares `paintOutput` ([painted live effects](#painted-live-effects)).

An **exporter** gets something else: the native `.vectorcraft` document JSON without indentation, optional preview/PDF
renditions or save-preference metadata (the format is documented in
[`crates/format/src/lib.rs`](../crates/format/src/lib.rs)). That is every layer, object, appearance and effect record,
artboard, swatch and symbol, so an exporter reads whatever the document holds; it checks `version`, since a later host
may write a newer one. Images of 8 KiB or more are listed in `images` with their MIME type and `blob` but without their
data, which follows the JSON in a saved file; smaller ones keep their base64 `data`.

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
  and `miterLimit`. Paints use the native file format's form: `{"type": "solid", "color", "swatch"?, "tint"?}`,
  `{"type": "gradient", "gradient": {"kind": "Linear"|"Radial"|"Freeform", "stops": [{offset, color, opacity,
  midpoint, swatch?, tint?, plates?}], "dither"?, "method"?: "Perceptual"|"HueClockwise"|"HueCounterclockwise"}, "geom"?, "angle", "freeform"?}`, `{"type": "pattern", "pattern",
  "xf"?}`. Colours are `{"model": "rgb", r, g, b}`, `{"model": "cmyk", c, m, y, k}`, `{"model": "gray", k}` (ink: 0
  white, 1 black) or `{"model": "lab", l, a, b}`; components 0–1 except Lab's.
- `effects` (filters): the object's own effects in order, as its effect records: `id` (as `effect.list` lists it, or
  `plugin.<id>` for a live-effect plug-in), `params` and `visible`. Its fills' and strokes' own effects aren't listed.
- `bounds`: `[x0, y0, x1, y1]`, the geometric bounds (read only).

### Reading type, images and symbols

A filter whose manifest lists `inputs` also gets the selected objects of those kinds, in paint order among the paths
and under the same rules (hidden and locked ones, objects that clip a clip group, alone or inside a group that clips
it, and anything inside a symbol, a placed document or a live object are left out). They are never turned into
outlines, rectangles or the symbol's art. Each has `id`, `type`, `name` (when it has one), `bounds` (`null` until the
object has been laid out or drawn) and `transform`, `[a, b, c, d, e, f]` mapping its own space (text space, pixel space,
the symbol's art) into the document. In this example the character and paragraph styles are cut short; a filter gets
every attribute:

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
  `"unknown"`, read from the image itself. An image whose corners Perspective Distort or Free Distort moved to a shape
  that is not a parallelogram also has `corners`, its pixel rectangle's corners on the page (top left, top right,
  bottom right, bottom left), and its `transform` is then the affine placement nearest them, the one that matches the
  perspective at the middle of its pixels. Corners that make a parallelogram are described by `transform` alone. An
  embedded image whose pixels take at most 1 MiB in base64 (443 × 443
  pixels fit) has them in `pixelsBase64`: straight RGBA, 8 bits a channel, row by row from the top, and
  `pixelsOmitted: false`. A CMYK image's pixels are its inks through the working CMYK profile, as sRGB. The pixels of
  all the images in one input take at most 16 MiB in base64, and an image the host starts to decode takes its size
  from those 16 MiB even when decoding fails. A larger image, one whose pixels don't fit what the images before
  it in paint order left, and a linked image (`linked: true`) have `pixelsOmitted: true` and no pixels; the host checks
  an image's size before it decodes the image. An image of any size also has `pixelsOmitted: true` when its data is
  missing from the document, when its width or height is 0, when its data cannot be decoded (including a non-CMYK image
  whose decoder enforces the host's 64 MiB memory limit and needs more, and a TIFF the host rejects as too large), or
  when the dimensions in its data differ from `pixelSize`. A plug-in checks `pixelsOmitted` before it reads
  `pixelsBase64`, which is present only when `pixelsOmitted` is `false`. The host never reads a linked image's file for
  a filter and never gives its path.
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

### Document information

A filter, or a live effect with [commands](#commands), whose manifest lists `context` (plug-in API level 3) gets those
parts of the active document in `_context.document` on each filter or command run, whether or not anything is
selected. Drawing a live effect never gets them. The host reads them when the run starts, and they stay as read for
the whole run. `revision` is the document's revision at that time, so a plug-in that [calls commands](#command-calls)
can tell that a call changed the document. Only the sections the manifest lists are given:

```json
"document": {"revision": 7,
 "settings": {"name": "Poster.vectorcraft", "colorMode": "cmyk", "units": "Millimeters", "width": 612, "height": 792,
              "bleed": [9, 9, 9, 9], "rasterEffectsPpi": 300, "activeArtboard": 1},
 "artboards": [{"id": 1, "index": 0, "name": "Front", "rect": [0, 0, 612, 792], "active": true}],
 "layers": [{"id": 2, "parentId": null, "depth": 0, "name": "Art", "visible": true, "locked": false, "template": false,
             "color": "Light Blue", "sublayers": 1, "current": false},
            {"id": 5, "parentId": 2, "depth": 1, "name": "Notes", "visible": false, "locked": true, "template": false,
             "color": "Red", "sublayers": 0, "current": true}],
 "swatches": [{"name": "Brand Red", "kind": "color", "group": null, "global": true, "spot": false,
               "color": {"model": "cmyk", "c": 0, "m": 0.9, "y": 0.8, "k": 0}, "hex": "#e8392c"}]}
```

- `settings`: `name` is the document's name as its tab shows it, without its folder; `colorMode` is `"rgb"` or
  `"cmyk"`; `units` are the ruler units as `document.setup` names them; `width` and `height` are the active artboard's
  size in points (`null` without artboards); `bleed` is `[top, bottom, left, right]` in points; `rasterEffectsPpi` is
  the Document Raster Effects Settings resolution; `activeArtboard` is the active artboard's id.
- `artboards`: every artboard in the Artboards panel's order, with its `id`, which stays the same when artboards are
  reordered, its `index` (its place now), `name`, `rect` (`[x0, y0, x1, y1]` in the document points `bounds` uses) and
  `active`.
- `layers`: every layer and sublayer, depth first in document order: the bottom layer first, each layer followed by its
  sublayers, bottom first. `id` is the id layer commands take; `parentId` is the layer it is in (`null` at the top
  level); `depth` is 0 for a top-level layer. `visible`, `locked` and `template` are the layer's own settings, so a
  sublayer of a hidden layer keeps its own `visible`. `color` is the layer color's name or `#rrggbb`; `sublayers` counts
  the layers directly inside it; `current` marks the current layer. Objects are not listed.
- `swatches`: every swatch as `swatch.list` lists it (the swatches outside color groups, with the built-in Registration
  swatch after None, then each color group's): `name`, `kind`, `group`, `global`, `spot`, and for a color its `color` and `hex` (and `tintOf` and `tint`
  for a tint). A gradient swatch gives its gradient's kind and a pattern swatch its pattern's name, not their art.

The document information holds no file path or folder, no linked file, font file or placed document, and nothing from
another document. A document with more than 10,000 rows in the sections asked for (artboards, layers and sublayers,
and swatches together), or whose rows take more than 4 MiB of JSON, fails the run before the plug-in starts, with `the
document information the plug-in asks for is too large to hand over`, and the document stays as it was. Each row takes
1,000 instructions from the run's [instruction budget](#instruction-budget-preference), and the document information
counts toward the half of the memory cap that the input and the parameters share. These rules are VectorCraft's own.

### Parameters

`params` points at `params_len` bytes of UTF-8 JSON: the validated parameters, plus `_context`:

```json
{"amount": 50, "mode": "soft",
 "_context": {"mode": "filter", "bounds": [10, 0, 75, 60], "artboard": [0, 0, 612, 792], "documentData": {"pens": 4}}}
```

`mode` is `"filter"`, `"effect"`, `"export"`, `"import"`, `"place"` or `"command"` (a [command](#commands) run, whose
`command` is the command's id); `bounds` the selection's geometric bounds (a live effect: the reference box of the
effect, the object's bounds or those of the previous effect's result); `artboard` (filters) the artboard under the
selection's centre, else the first; `documentData` (filters) what the plug-in last kept with the document (`null` until
it keeps something; see Output); `document` (filters and commands whose manifest lists `context`) the [document
information](#document-information) asked for. A live effect's `item` says where it sits: `"object"` (one of the object's effects), or
`{"fill": i}` / `{"stroke": i}` (in the own effects of the object's `i`th fill or stroke, counted from the bottom as in
`fills` and `strokes`). An export's `_context` is `{"mode": "export", "extension": "plt", "name": "Poster"}` instead:
the extension of the file being written (one the exporter lists) and its name without the extension (the document's name
when the export is not given a path), which some formats keep in their header. An import's `_context` is `{"mode":
"import", "extension": "plt", "name": "Poster"}`: the extension of the file being opened and its name without the
extension. A place's `_context` is the same with `"mode": "place"`; the importer is given nothing of the document the
file is placed in.

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
- A compound path whose `path` is returned as it was, or left out, keeps its members as they are, hidden and locked
  ones included; a changed `path` replaces its members with one plain member per subpath. Each new member keeps only
  the id of the old member in its place: its name, live shape, opacity, blending mode, appearance, opacity mask and
  other settings of its own are reset to a new path's defaults. The host refuses a `path` with no anchor points, so return a
  compound path that has none without `path`. While a member is hidden, locked or a guide, or is not a path (its
  geometry is not in the input's `path`), the host refuses a changed `path` and `type: "path"` for it (a guide member
  is allowed when the output also sets `guide`). `guide` applies to every member, so `guide: false` on a compound path
  makes each of its guide members an ordinary path.
- The host compares a returned `path` with the input coordinate by coordinate, exactly. A JSON parser that does not
  read numbers back exactly as they were written (in Rust, `serde_json` without its `float_roundtrip` feature) can
  change the last digit of a coordinate, so leave `path` out when the geometry does not change.

The host validates everything before the document changes: coordinates must be finite and within ±4,000,000 pt, at
most 100,000 objects and 4,000,000 anchors, 256 fills, strokes or gradient stops; colours, opacities and offsets are
clamped into range, stroke weights to 0–1,000 pt and miter limits to 1–500; a pattern must exist in the document. A
gradient stop's four process `plates` are clamped to 0–1 as well: a value that is not a number becomes 0, and an
infinite one becomes 0 or 1 by its sign. A paint returned unchanged keeps its swatch link; any other paint is
unlinked, and each of its gradient stops then has no swatch, a tint of 1 and no `plates`. One refused effect record refuses the whole output.

A filter's output may also hold `documentData`: any JSON of at most 1 MiB, which replaces what the plug-in keeps with
the document (`null` removes it), in the same undo step as its objects. Settings that belong to the whole document
rather than to one object go there, such as the pens a plotter holds. A filter reads and writes only its own data. It
is the document's foreign data, as the perspective grid is: the native file keeps it in `document.unknown.pluginData`
under the plug-in's id, and hosts from before `documentData` keep it when they save. An exporter is given the whole
document, so it can read what every plug-in keeps there, not only its own. A live effect isn't given it: its geometry
is worked out from the object alone.

To report what a run did, a filter or a plug-in command adds `notice`: one string, such as
`{"objects": [...], "notice": "Selected 12 objects"}`. It needs no permission and no host import. Once the output is
applied, the desktop app and the web build show it in the status bar as `Plug-in “name”: notice`, the plug-in's name
first, and `plugin.run` and the plug-in command return it as `notice` in their result, with the plug-in's name as
`pluginName`. The host trims the white space around it, writes control, bidirectional and line
separator characters as `\u{202E}` escapes and cuts it at 200 characters, escapes included, ending it with `…`. Each run
reports at most one notice, and a later status message replaces it. A plug-in step of an action or a `command.batch`
shows its notice too; when several steps report one, the last of them shows. A `notice` that is not a string, or has no visible
character in the part the status bar shows, refuses the whole output, and nothing changes; white space, control characters and the invisible
characters that are not escaped (soft hyphen, zero-width and filler characters, variation selectors, tags) do not
count as visible. A run that declines, fails, is refused, is canceled or
finishes after the document changed shows none; an output that changes nothing still shows its notice. The status
bar translates the words around the notice, never the notice itself. A live effect's
`notice`, painted or not, is ignored without being checked, since the effect runs whenever its object is drawn. These limits are VectorCraft's own values. Hosts
from before `notice` ignore it.

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
  ignored, as is a top-level `notice`. Any other key (`opacity`, `effects`, `guide`, `locked`, `name`, a top-level
  `documentData`…) refuses the
  result, as does a key that a paint, its color, a gradient stop, a gradient's placement or a freeform point does not
  have, such as `opacity` on a solid paint.
- `fills` and `strokes` take the forms a filter returns ([Input](#input)). A piece without `fills` (or `strokes`) is
  painted with the ones the effect was given; an empty list paints none. Given ones replace the object's in order, as
  a filter's output sets them: an existing fill or stroke keeps its own effects, opacity and blending, and a new stroke
  starts at 1 pt, miter join, butt cap, miter limit 10.
- An effect on a fill (`_context.item` `{"fill": i}`) returns only `fills`, which replace that fill on each piece; an
  effect on a stroke returns only `strokes`. The object's other fills and strokes stay where they are.
- Each fill or stroke such an effect returns keeps the effects after it in that fill's or stroke's stack. A later
  live effect, and a later geometry effect on strokes, runs once on each of them in paint order: it is given the
  piece's `fills` and `strokes` with `_context.item` naming that one (`{"fill": 1}` for the second fill on the
  piece), and its result replaces only that fill or stroke. A run that fails, is refused, is deferred or would pass
  the budget leaves its own fill or stroke as it was, and the others still run.
- The effects after it in the stack apply to the pieces together, the reference box being the bounds of all of them.
  Bounds, raster effects, Pathfinder effects, compound shapes and Envelope Options › Distort Appearance use the pieces
  as they are drawn. Outside outline mode, an image is also hit where the pieces of its own fills and strokes paint; a
  path or compound path is hit-tested on its source path and appearance. On an object with no geometry effect of its
  own, Pathfinder effects and compound shapes use the object's path for an effect on a fill or stroke, as they do for
  every other effect on a fill or stroke; on an object that has one, they also use the pieces of a painted effect on a
  fill or stroke. A 3D effect later in the stack uses the object's input path when the pieces name a swatch or pattern.
  Each piece counts against the budget of the object's geometry effects as well as its anchors, so all the painted
  effects in a stack make at most 100,000 pieces together; a run whose pieces would pass the budget draws its input as
  it was.
- The result is checked whole before any of it is drawn: the limits of a filter's output (100,000 objects, 4,000,000
  anchors, 256 fills and 256 strokes per piece, 256 stops per gradient), finite numbers in range with nothing clamped
  (color components, tints, offsets, opacities, midpoints 0.13 to 0.87, stops in order of `offset`, stroke
  widths 0 to 1,000 pt, miter limits 1 to 500), and every swatch a solid paint, gradient or stop links to and every
  pattern named in the document. A gradient stop's `plates` need that stop's own link to a spot swatch of the document;
  the gradient's `swatch` does not link its stops. One fault refuses the whole result: that step of the stack draws
  its input as it was, the effect record stays, and `plugin.info`'s `lastError` names the plug-in and the field, for
  example `Test Tone: plug-in output: objects[1].strokes[0].width: …`.
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
that fails the checks opens nothing, and neither does a decline (`{"error": …}`). An importer that places its files
returns the same output when `_context.mode` is `"place"`; an output that fails, or whose objects other than guides draw
nothing of any width or height, places nothing.

#### Importer output kinds

An importer at `api` 4 whose manifest lists [`outputKinds`](#manifest) may return more than paths. The host checks the
whole output before it makes the document, and one entry that fails a check opens nothing; the error names the entry
(`objects[2]`, `layers[0]`) and the limit or field. Content of a kind the manifest doesn't list is refused, and the
error names the kind. An importer that lists no kinds is read as before, and the keys below are errors for it.

```json
{
  "objects": [
    {"type": "type", "text": "Hello", "fontFamily": "Source Sans 3", "fontStyle": "Bold", "fontSize": 24,
     "fill": {"type": "solid", "color": {"model": "rgb", "r": 0, "g": 0, "b": 1}}, "transform": [1, 0, 0, 1, 10, 40]},
    {"type": "image", "pixelSize": [2, 1], "pixelsBase64": "/wAA/wAA/4A=", "transform": [10, 0, 0, 10, 0, 100]},
    {"type": "path", "path": {"subpaths": [{"closed": true, "anchors": [{"p": [0, 0]}, {"p": [10, 0]}, {"p": [10, 10]}]}]}}
  ],
  "layers": [{"name": "Art", "objects": [1, 2]}, {"name": "Words", "visible": false, "locked": true, "objects": [0]}],
  "artboards": [{"name": "Front", "rect": [0, 0, 200, 100]}, {"name": "Back", "rect": [220, 0, 200, 100]}]
}
```

- `"type": "type"` (kind `type`) is point type in the document's Normal Character Style with the face, size and fill
  it gives: `text` (at most 1,048,576 bytes of UTF-8, a `\n` between paragraphs, no control characters other than
  line breaks and tabs), `fontFamily`, `fontStyle` (left out: the family's regular style), `fontSize` (0.1–1296 pt),
  `fill` (a color or a gradient, in the filters' paint form; left out: black), `transform` and `name`. The first
  baseline starts at the origin of the type's own space. A font that isn't installed keeps its name, and the open
  reports it as missing. All the type of one output holds at most 16,777,216 bytes of text.
- `"type": "image"` (kind `image`) is an image embedded in the document, stored as a PNG of its pixels: `pixelSize`
  (width and height, 1–16,384 each), `pixelsBase64` (straight RGBA, 8 bits a channel, rows from the top, exactly 4 ×
  width × height bytes in padded base64), `transform` (from pixel space, one unit a pixel, into the document) and
  `name`. All the images of one output hold at most 33,554,432 bytes of RGBA. The host checks both limits from the
  sizes the images give, and the length of `pixelsBase64`, before it decodes any pixels.
- `transform` is six finite numbers `[a, b, c, d, e, f]` within ±4,000,000 that don't flatten the object; left out,
  the identity. Type or an image that then reaches more than 4,000,000 pt from the origin is refused.
- `layers` (kind `layer`): 1–256 top-level layers, bottom first. Each has a `name`, `visible` (default `true`),
  `locked` (default `false`) and `objects`, the indexes of its objects in `objects` in increasing order. Every object
  is in exactly one layer, and a layer may hold none. Without `layers`, every object goes in one layer.
- `artboards` (kind `artboard`): 1–100 artboards, each with a `name` used once and a `rect`, `[x, y, width, height]`
  with a width and height above 0, within ±4,000,000. They replace the one artboard around the art.
- A `name` (of type, an image, a layer or an artboard) and a font's family and style are each 1–256 characters, not
  only spaces, without control characters or characters that change the direction of text.
- An output whose `objects` is empty opens as a blank document when it has `layers` or `artboards`.
- To place (`_context.mode` `"place"`), the objects of the visible layers are placed in one group with their type and
  images, the objects of hidden layers are left out, and `artboards` are left out.

## Writing a plug-in in Rust

[`crates/plugins/example`](../crates/plugins/example) is a complete object filter, **Desaturate**: it moves every
colour of the selection towards grey, in each colour's own model, using `serde_json` (no imports, no WASI). Build it
with:

```sh
rustup target add wasm32-unknown-unknown
crates/plugins/example/build.sh      # cargo build --release --target wasm32-unknown-unknown
```

The script also refreshes `crates/plugins/tests/fixtures/desaturate.wasm`, the test fixture built from that source.
The crate is not a workspace member. It puts its manifest in the [manifest section](#manifest-section) with a byte
array static marked `#[unsafe(link_section = "vc_manifest")]` (the Rust Reference's
[`link_section`](https://doc.rust-lang.org/reference/abi.html#the-link_section-attribute)), and keeps the
`vc_manifest` export for older hosts. Any language that targets core WebAssembly works (C, Zig, AssemblyScript,
WAT…), as long as the module imports nothing but the host imports: in Rust, `std` on `wasm32-unknown-unknown` is fine as
long as it never touches WASI. `vectorcraft_plugins::wat::module` is the skeleton of a plug-in in WebAssembly text,
which the tests build their plug-ins from, and `wat::sectioned` the same plug-in with its manifest in the [manifest
section](#manifest-section) and no `vc_abi_version` or `vc_manifest` export (in WebAssembly text, a custom section is
written `(@custom "vc_manifest" "…")`); `wat::echo_exporter` is an exporter whose file is its input or its parameters
(with a base64 encoder written out in WebAssembly text), which the tests read back what the host hands a plug-in with,
and `wat::when_given` a plug-in whose output depends on whether its input or parameters hold a given text, which shows
in a live effect's geometry. `wat::reporting` and `wat::cooperating` build level 2 plug-ins that import the host
functions.

## Performance

The interpreter costs roughly 10–100× native speed per instruction. Filters handle a selection in one call, so a
plug-in that parses and writes JSON with `serde_json` takes milliseconds for hundreds of paths. Live effects run once
per change of their input, parameters or paints (results are cached), not per frame.

## Not in v1

- Tool plug-ins (pointer events on the canvas, a guide line or rectangle, geometry back), panel plug-ins, importers
  that make area type, type on a path, nested layers or linked images (an importer at `api` 4 makes point type,
  embedded images, top-level layers and artboards, see [importer output kinds](#importer-output-kinds)), and placed
  importer files that stay linked to their file.
- Live effects that change opacity, blending or effects (at `api` 2 one may [paint](#painted-live-effects) the pieces
  it returns), and filters that change type, images or symbol instances (a filter at `api` 2 reads them, see
  [`inputs`](#reading-type-images-and-symbols)).
- Scripts can't run object filters (`plugin.*` commands are refused there), so a script doesn't change what Apply
  Last Filter runs.
