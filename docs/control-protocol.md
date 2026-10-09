# Control protocol

`vectorcraft --control <port>` listens on `127.0.0.1:<port>` (loopback only). One JSON request per line:
`{"id": 1, "method": "ui.inspect", "params": {}}` → `{"id": 1, "ok": true, "result": {...}}` or `{"id":1,"ok":false,"error":"..."}`.

**Only requests are read.** Every line must be a JSON object with a string `method` (`id` and `params`
are optional; blank lines are skipped). Anything else gets one error reply
(`{"ok": false, "error": "… closing the connection"}`) and the server **closes the connection**, so
nothing sent after it on that connection runs. That covers text that isn't JSON, a JSON array or number,
an object without `method`, invalid UTF-8, and a line longer than 4 MiB. An HTTP request (for example a
web page's cross-origin `fetch` to `127.0.0.1:<port>`) therefore can't smuggle a command in its body:
its request line is rejected first. At most 16 connections are served at once; further ones get an error
line and are closed. Clients that get an error reply should reconnect. The port has no authentication,
so only enable it while you use it. Transport: `apps/vectorcraft/src/control_server.rs`.

| Method | Params | |
|---|---|---|
| `engine.execute` | `{command, params}` | run any engine or UI command (see `engine.commands`) |
| `engine.commands` | | every command with label, shortcut, params doc, enablement |
| `document.inspect` | `{depth?, childLimit?}` | layer tree, selection, history, paint defaults; the options slice the layer tree as `document.node {summary: true}` does |
| `ui.inspect` | | tool, UI state, `screenMode` (0 normal, 1 full screen with menu bar, 2 full screen, 3 Presentation Mode), `taskBar` (`pinned`, and `rect` `[x, y, w, h]` while the Contextual Task Bar shows), `freeTransformWidget` (`[x, y, w, h]` while the Free Transform widget shows), view, canvas rect, window size, perf, background saves and exports still running, `nativeMenuBar` (true when the menus are in the macOS menu bar rather than the window; `ui.menu.list` lists the in-window menus either way) |
| `ui.menu.list` / `ui.menu.invoke` | `{command, params}` | the full menu tree / invoke an item |
| `ui.contextMenu.list` | | the canvas context menu for the current selection, flattened like `ui.menu.list` (`path` holds its submenus). `ui.click {x, y, button: "right"}` on the canvas opens it, after selecting the object there unless it is already selected |
| `ui.tool.select` / `ui.tool.list` | `{tool}` | |
| `ui.pointer` | `{events:[{kind: down|drag|up|move|doubleclick, x, y, space?: "doc"|"screen", mods?}]}` | drive the active tool exactly like the mouse: `mods.cmd` held at a press with any tool but a selection tool drags with the selection tool chosen last (Direct Selection with the Pen until one is), and the release gives the tool back as it was |
| `ui.key` / `ui.text` | `{key, shift?, alt?, cmd?}` / `{text}` | synthetic keyboard input |
| `ui.wheel` | `{x, y, dy?, dx?, unit?: "line"\|"point", shift?, alt?, cmd?}` | a mouse wheel turn over screen point (x, y): `dy` notches up (+) or down, `dx` sideways. Over the canvas the wheel scrolls and Cmd- or Alt-wheel (Option on the Mac) zooms about the pointer; with the `zoomWithMouseWheel` preference the wheel and Alt-wheel zoom about the pointer, Shift-wheel scrolls up and down and Cmd-wheel (Ctrl on Windows and Linux) sideways. Over a focused numeric field (click it first) each notch steps its value as Up/Down do (Shift: ten, Cmd/Ctrl: a tenth) and the panel stays put; over anything else in a panel the wheel scrolls it |
| `ui.set` | `{brightness?, panel?, rulers?, outline?, grid?, smartGuides?, boundingBox?, controlBar?}` | |
| `ui.dialog.set` / `.confirm` / `.cancel` | `{field, value}` | fill and submit the open dialog |
| `ui.screenshot` | `{path?}` | capture the window (PNG). Needs a presented frame: with the screen locked or the window minimized/covered it fails after ~8 s with an explanatory error |
| `ui.render` | `{path?, scale?}` | render the artboard headlessly (PNG) |
| `ui.resize` / `ui.focus` | | |
| `app.open` / `app.save` / `app.export` / `app.quit` | `{path}` / `{path?}` / `{path?, format?, artboard?, range?, scale?, …}` | `app.open` reads every format `document.open` reads (see `document.formats`) and returns its result (`{index, title, format, warnings, …}`; `warnings` say what didn't come in as it was, such as an EPS shown as its preview image and why its PostScript couldn't be read), or null when a dialog asks first or a library loads. `app.export` encodes through the engine's `document.export` (same options; the document keeps its path) and writes `path` through the host; without `path` it returns `{dataBase64, format, bytes}`, as headless mode does. `app.quit`, `file.close` and `file.closeAll` first open a `saveChanges` dialog for each modified document (they return `{"pending": "saveChanges"}`): `ui.dialog.confirm` saves, `ui.dialog.set {field: "discard", value: true}` then confirm discards, `ui.dialog.cancel` cancels the whole close or quit |
| `app.export` with `useArtboards` | `{path, useArtboards: true, range? \| artboards?, …}` | Export As: PNG, JPEG and WebP write one file per chosen artboard (default all; SVG does so for a `range`), `<path stem>-<artboard>.<ext>`, and return `{path, files: [path…]}`; a PDF keeps them as pages of `path`. `useArtboards: false` covers the bounds of the visible art. The menu's Export As… is `file.exportAs`: the `exportAs` dialog (`format`, `useArtboards`, `all`, `range`), then a save dialog and the format's options: `pngOptions` / `jpgOptions` / `webpOptions`, `svgOptions`, or `savePdf` for a PDF with Use Artboards |
| `ui.dialog.*` on `gradientStop` | `{field: "color" \| "opacity" \| "location", value}` | double-clicking a stop on the Gradient tool's annotator opens its popover (fields `index`, `x`, `y`, `tab`); set `color` (hex), `opacity` or `location` (percentages) and `ui.dialog.confirm` to apply them to the selected stop. `gradient.selectStop {index}` picks the stop the annotator, the panels and Delete/←/→ (`ui.key`) act on |
| `engine.execute` `ui.savePdfDialog` | `{path?, preset?, range?, …document.exportPdf options}` | opens the Save PDF dialog (`file.export.pdf` with no params opens it too). Its fields are the `document.exportPdf` options: set `compatibility` with `ui.dialog.set {field: "compatibility", value: "1.5"}`, a whole section with `{field: "compression", value: {…}}`, the artboards with `__allArtboards: false` and `range`, the visible section with `__section` (General, Compression, Marks and Bleeds, Output, Advanced, Security, Summary). `ui.dialog.confirm` writes `path` (else asks for one), returns `{path, bytes, warnings}` and opens the file when `viewAfterSaving` is set |
| `file.save` / `file.saveAs` / `file.saveCopy` / `file.saveAsTemplate` (via `engine.execute`) | `{path?, format?, options?, svg?}` | Save writes the document's own file in its own format (a document opened from or saved as SVG/PDF saves as that again; the result's `warnings` say what the format loses). With `path` the file is written at once (Save As or a Copy to an SVG path without SVG options first opens SVG Options, below). Without one (never saved, a converted older file, Save As, a Copy, a Template) the app shows a save panel with one file type per save format (`vectorcraft`, `template`, `pdf`, `svg`, `svgz`), then the format's options: SVG opens the `svgOptions` dialog (below; returns `{"pending": "svgOptions", "path"}`), PDF the `savePdf` dialog (its `__save` field names the save command); `ui.dialog.confirm` finishes the save. On the web a `saveOptions` dialog (fields `path`, `format`) first names the file and picks the format. `file.formatOptions {format?}` reads the options a save would use. `file.revert` opens a `confirm` dialog (`{"pending": "confirm"}`); `ui.dialog.confirm` reloads the saved file in the same tab |
| SVG Options (`engine.execute` of `file.export.svg`, `file.saveAs`, `file.saveCopy`) | `{}` / `{path?, svg?: {…}}` | `file.export.svg` without params, and Save As or Save a Copy to an `.svg`/`.svgz` path without SVG options, open the `svgOptions` dialog (`ui.dialog.set` its fields: `styling`, `outlineText`, `images`, `objectIds`, `decimals`, `minify`, `responsive`, `fewerTspans`, `metadata`, `preserveEditing`, `useArtboards`, `allArtboards`, `range`, `showCode`, `profile` (`svg11`/`tiny12`), `encoding` (`utf8`/`utf16`/`latin1`), `embedFonts`; `ui.dialog.confirm` exports or saves and remembers the choices). With `svg: {…}` (the `document.formats` SVG options) they run directly. `app.save {path?, svg?}` takes the same options; a document saved as SVG saves as SVG again with the same options |
| `engine.execute` `file.place` | `{paths?}` / `{path \| name+dataBase64, …file.place params}` | with no file, picks files and opens the `place` dialog; with `paths`, opens it for them (fields `files`, `link`, `template`, `replace`; `__replace` says whether Replace applies, `__info` describes each file). `ui.dialog.confirm` places one file centred in the view (or in place of the selected object) and loads the place cursor with several (`file.place.queue`). With a file, it runs `file.place` centred in the view. Files dropped on the window with no document open open. On the web, files dropped on the canvas are placed at the drop point (linked; Shift embeds) and files dropped off it (the tab bar) open, as in Illustrator; desktop drops carry no pointer position, so documents (native, SVG, PDF, `.ai`, EPS, DXF, EMF/WMF) open as tabs of their own and pictures and text are placed in the middle of the view. With the cursor loaded (`ui.inspect` → `tool: "place"`, `toolOptions: {count, current, name, names}`), `ui.pointer` clicks and drags place files and `ui.key` ←/→/↑/↓ and Escape cycle and discard them |
| `app.export` raster options | `{path?, format: png \| jpg \| webp, ppi?, background?, antiAlias?, interlaced?, quality?}` | `ppi` sets the pixel size (72 = one pixel per point) and is stored in the file; `background` is `transparent`, `white`, `black` or `"#rrggbb"`; `antiAlias` is `none`, `art` or `type` (text snapped to pixels); `interlaced` writes an Adam7 PNG. The PNG Options dialog (`pngOptions`; `jpgOptions` / `webpOptions` for the other raster formats) has the same fields: fill them with `ui.dialog.set` and press OK with `ui.dialog.confirm` |
| `app.export` JPEG options | `{path?, format: jpg, quality?: 0–100, colorModel?: rgb \| cmyk \| gray, method?: baseline \| optimized \| progressive, scans?: 3–5, embedIcc?, imageMap?: none \| client \| server}` | `cmyk` writes ink amounts in the working CMYK space (CMYK colours keep their inks; RGB ones are separated with the colour settings); `embedIcc` (default true) embeds sRGB, the working CMYK profile or a grey profile, generated by the CMS; `imageMap` also writes `<stem>.html` (client-side `<map>`) or `<stem>.map` (NCSA) beside the image for the objects with a URL and an Image Map shape (`attributes.set`), returned in `linked`. The `jpgOptions` dialog has these fields too (`quality` is shown 0–10; a CMYK document defaults to `cmyk`) |
| `app.export` PNG-8 and GIF | `{path?, format: png8 \| gif, colors?: 2–256, reduction?: perceptual \| selective \| adaptive \| web \| blackWhite \| gray, dither?: none \| diffusion \| pattern \| noise, ditherAmount?: 0–100, transparency?, matte?, interlaced?, …raster options}` | a palette of at most `colors` entries (art with that many colours or fewer keeps them exactly); pixels under half opacity become one transparent entry unless `transparency` is false, partly transparent edges are blended over `matte` (`none` keeps their colour). `png8` writes an indexed `.png` (1–8 bits per pixel); Export for Screens rows take both. The `png8Options` / `gifOptions` dialogs have these fields |
| `app.export` Text and WebP | `{path?, format: txt, encoding?: utf8 \| utf16, lineEndings?: lf \| crlf, selectionOnly?}` / `{format: webp, lossless?, quality?}` | Text writes the stories in stacking order (back to front; a thread once; hidden objects and template layers left out), each line ended by `lineEndings`; `utf16` is little-endian with a byte order mark. Export As → Text opens `txtOptions` (fields `path`, `encoding`, `lineEndings`, `selectionOnly`; OK writes the file). WebP is always lossless: `lossless: false` adds a warning |
| `engine.execute` `document.exportForOffice` | `{}` / `{path?, ppi?: 72 \| 150 \| 300, transparent?, artboard?}` | File → Save for Office Documents…: without params opens `saveForOffice` (fields `ppi`, `transparent`, `artboard`; OK picks the file); with params writes the artboard as a PNG (`path`, else a picked file) → `{path}`. Headless it returns `{dataBase64, width, height}` |
| Background Save / Export | `app.save`, `app.export {path}`, `file.save`, `file.saveAs`, `file.saveCopy` | with the `backgroundSave` / `backgroundExport` preferences on (the default) the desktop app writes the file from a snapshot on a worker thread: the call returns `{path, background: true, status}` at once, `ui.inspect` lists the jobs still running in `background` (labels such as `"Saving a.vectorcraft"`), and the document counts as saved (as of the snapshot) once its file is written. Quit and Close wait for them. With the preferences off (`prefs.set`), saves return once written |
| `engine.execute` `file.exportForScreens` | `{}` / `{…document.exportForScreens params}` | without params opens `exportForScreens` on the settings the document last exported with (`document.exportSettings`). Fields: `tab` (`artboards` \| `assets`), `select` (`all` \| `range` \| `full`), `range` ("1-3, 5"), `boards` (a bool per artboard; checking them sets `select` and `range`), `includeBleed`, `folder` (desktop; default the Desktop, else home), `openLocation`, `subfolders`, `prefix`, `preset` (`""` \| `mobile` \| `density`), `formats` (rows `{format, scale: "2x" \| "100w" \| "100h" \| "72ppi", suffix, quality?, folder?}`), `settings` (`{png \| png8 \| jpg \| webp \| gif \| svg \| pdf: {…options}}`, the gear's Format Settings; `__settings` names the format shown). `ui.dialog.confirm` exports (a bad range or no folder keeps it open); on the web OK reads "Download" and the files download (several as one `.zip`). With params it runs `document.exportForScreens`: no `folder` returns the files (or with `zip: true` one store-only zip) as `dataBase64`; with `openLocation` the first file is shown in the file manager |
| Export for Screens' Assets tab, Asset Export panel | `ui.dialog.set` `tab: "assets"`, `assets: [asset id…]`; `window.panel {panel: "assetExport"}` | on the Assets tab of `exportForScreens` the `assets` field lists the checked asset ids (all of them when it opens); `ui.dialog.confirm` runs `document.exportForScreens {assets}` (nothing checked keeps it open) and OK reads "Export Asset". The Asset Export panel acts through `assets.*` and shares the format rows with the dialog (`assets.settings.set`) |
| `engine.execute` `file.exportSelection` | `{multiple?}` | File › Export Selection…: collects the selected objects as assets (`assets.add`; one per object unless `multiple` is false) and opens `exportForScreens` on its Assets tab with just them checked → `{assets}`. `document.exportSelection` still exports the selection in one go, without the dialog |
| CSS Properties panel | `window.panel {panel: "cssProperties"}`; `engine.execute` `css.copy` / `css.exportFile` | the panel shows `css.selection` of the selection (after Generate CSS, while nothing is selected, `css.generate`); its menu holds the options the commands take (`units`, `position`, `dimensions`, `unnamed`, `rasterize`). `css.copy {scope?, …options}` copies the CSS to the clipboard → `{css, rules}`; `css.exportFile {path?, scope?, …options}` writes `css.export` to `path` (else a picked file; the web downloads it) with the pictures of rasterized art next to it → `{path, rules, images}` |

| `ui.dialog.*` on `importPdf` | `{field: "page" \| "allPages" \| "range" \| "cropTo" \| "password", value}` | opening (`app.open`, `file.open`) a PDF or `.ai` with several pages or a password, or placing one (`file.place` without `page`), opens the Import PDF dialog (`mode`: `open` or `place`; `pageCount`). While `__locked` is true, set `password` and `ui.dialog.confirm` to unlock it (a one-page file then opens or is placed). Then set `allPages: false` and `range` ("2-3, 5") to open some pages, or `page` to place one, and `cropTo` (`bounding`, `art`, `crop`, `trim`, `bleed`, `media`); `ui.dialog.confirm` opens or places them (placing keeps the other `file.place` params, such as `link`, `template` and `at`, in `__place`). `document.open {pages, cropTo, password}` and `file.place {page, crop, password}` do the same without the dialog |

Swatch editors: `engine.execute` with `ui.swatchOptions {name}` opens the `swatchOptions` dialog for a colour swatch
(fields `name`, `spot`, `global`, `mode`: `gray`/`rgb`/`hsb`/`lab`/`cmyk`/`web`, `color`: `"#rrggbb"` or a colour object,
`preview`), which previews on the canvas while open; `ui.dialog.confirm` applies it with `swatch.edit` as one undo
step and `ui.dialog.cancel` rolls the preview back. For a gradient swatch the dialog has `name` only (it shows the
gradient) and OK renames it; pattern swatches open pattern editing instead. Dropping a gradient (or colour) on a
swatch of its kind in the Swatches panel with Alt held replaces it (`swatch.edit {name, paint}`).

Confirmations: deleting swatches from the Swatches panel opens a `confirm` dialog (fields `message`, `detail`);
`ui.dialog.confirm` runs the command it asks about (here `swatch.delete`) and `ui.dialog.cancel` drops it.

New swatches and colour groups: `ui.newSwatch {spot?, group?}` opens the `newSwatch` dialog prefilled from the active
fill or stroke (fields as in `swatchOptions` plus `group`; a gradient or pattern only takes `name`) and
`ui.newColorGroup {swatches?}` opens `newColorGroup` (fields `name`, `fromArtwork`, `toGlobal`, `includeTints`,
`swatches`); `ui.dialog.confirm` runs `swatch.new` / `swatch.newGroup`.

The Color Picker is a dialog too: `engine.execute {command: "ui.colorPicker", params: {stroke?, color?}}` opens it for the fill (or stroke) proxy; `ui.dialog.set {field: "hex", value: "00FF00"}` (or `color`, `channel`, `webOnly`, `swatches`) then `ui.dialog.confirm` applies the colour through `paint.setFill` / `paint.setStroke`.

Graphic styles: `ui.graphicStyleOptions {name}` opens the `graphicStyleOptions` dialog (field `name`) for a style and
`ui.dialog.confirm` renames it with `graphicStyle.rename` (a name another style has returns an error and keeps the
dialog open); without `name` it names a new style made from the selection (`graphicStyle.new`).
`ui.mergeGraphicStyles {names}` opens the same dialog to name the style `ui.dialog.confirm` merges from them
(`graphicStyle.merge`). Deleting styles from the panel asks first with a `confirm` dialog (`graphicStyle.delete`).

Gradient panel: double-clicking a stop on the panel's slider opens the same `gradientStop` popover, with a `screen`
field (`[x, y]`, screen points) in place of `x`/`y`. The panel's stop eyedropper selects the Eyedropper tool with the
tool option `stop` (the tool to return to; `tool.setOption {key: "stop", value: "gradient"}`): its next click on art
samples the colour there into the selected stop (`paint.sampleColor {color, stop}`) and switches back. Dragging a
swatch, a Fill/Stroke proxy or the panel's gradient thumbnail onto art runs `paint.setFill` / `paint.setStroke` (the
active proxy) with the paint's params and the object's `ids`; a colour dropped on the panel's ramp adds or recolours
a stop.

Freeform gradients: double-clicking a point with the Gradient tool opens the same `gradientStop` popover on the selected
point (`paint.freeform.selectPoint`); its fields are `color` (hex), `opacity` and `spread` (percentages), applied by
`ui.dialog.confirm` through `paint.freeform.setPoint`. `ui.key` Escape ends the line Lines mode is drawing.

Tool options: double-clicking a tool button runs `tool.options {tool}`. For `gradient` it opens the Gradient panel;
for `eyedropper` it opens Eyedropper Options, an `eyedropperOptions` dialog (fields `sampleSize` 1/3/5, `pickUp` and
`apply`, the attribute trees of `eyedropper.setOptions`) whose `ui.dialog.confirm` runs `eyedropper.setOptions` (what
`appearance.copyFrom` copies). For `hand` it fits the artboard in the window (`view.fitArtboard`) and for `zoom` it
shows 100% (`view.actualSize`). For `rotate`, `scale`, `reflect` and `shear` it opens the same dialog as Object ›
Transform (dialog kind = the tool id), or fails with `nothing selected`. For `selection`, `directSelection` and
`groupSelection` it opens the Move dialog (kind `move`), with the same failure. `ui.key` Enter with one of those seven
tools opens its dialog too (nothing happens without a selection), with the transform tools' `origin` at their reference
point. Gradient tool handles snap to anchors, edges and smart guides; Shift constrains them to 45° steps from the
`constrainAngle` preference.

Effect dialogs: `engine.execute {command: "effect.dialog", params: {effect, index?, item?}}` opens the `effect` dialog
(fields: the effect's parameters, `preview`). With `index` it edits that applied effect of `item` (null: the object's
effects) prefilled with its values, and `ui.dialog.confirm` runs `effect.setParams`; otherwise confirm runs
`effect.apply`. Choosing an effect that the list already has returns `{"pending": "effectExists"}` and opens the
`effectExists` question: `ui.dialog.confirm` opens the applied effect's dialog, `ui.dialog.set {field: "discard",
value: true}` then confirm opens a fresh one that adds another, `ui.dialog.cancel` drops it.

Plug-in dialogs: `engine.execute {command: "plugin.dialog", params: {id}}` (Object › Plug-ins) runs an object filter
plug-in at once when it takes no parameters, else opens the `plugin` dialog: one field per declared parameter (named
after it), `__plugin` (the plug-in id) and `preview`. It previews `plugin.run` on the canvas while open;
`ui.dialog.confirm` keeps the result as one undo step and `ui.dialog.cancel` rolls it back. Live-effect plug-ins open
the `effect` dialog (`effect.dialog {effect: "plugin.<id>"}`). `ui.installPlugin {path?}` installs a `.wasm` (without
a path it asks for one); see [plugins.md](plugins.md).

Overprint Black: the Edit → Edit Colors → Overprint Black… menu item opens a `command` parameter dialog for
`edit.colors.overprintBlack` (fields `remove`, `percentage`, `fill`, `stroke`, `includeCmyBlacks`,
`includeSpotBlacks`); `ui.dialog.set` then `ui.dialog.confirm` runs it on the selection.

Color Guide: `color.harmony {color, rule, steps?, variation?, amount?}` returns the harmony group (base first) and
its variation grid; the panel draws the same grid. `ui.colorGuideOptions` opens the `colorGuideOptions` dialog
(fields `steps` 1–20, `amount` 0–100) and `ui.dialog.confirm` sets the panel's options, which `ui.inspect` reports
as `ui.color_guide` (`variation`, `steps`, `amount`). Save Colors as Swatches runs `swatch.new {colors: [...]}`
(one swatch per colour, one undo step).

Color Guide Limit to Library: `ui.colorGuideLimit {library}` (a swatch library id or name, `"document"` for the
document's swatches, `""` for none) limits the panel's colours; `ui.inspect` reports it as `ui.color_guide_limit`, and
`color.harmony {..., limitTo}` answers the limited guide (every colour snaps to the library's nearest colour, ΔE 2000).
Edit or Apply Colors runs `ui.recolorDialog {colors, library?}` with the harmony colours: on the selected art it
recolours as usual; with nothing selected the colours themselves are the rows and `ui.dialog.confirm` saves them as a
new colour group (`swatch.newGroup`, named by the `groupName` field).

Color Themes panel (`window.panel {panel: "colorThemes"}`): its Create tab makes a theme on a harmony wheel and its
My Themes tab lists the saved themes; every action runs a command (`colorTheme.save`, `colorTheme.delete`,
`colorTheme.addToSwatches`, `swatch.newGroup` for the theme being made), and `colorTheme.list` reads the library.

Edit Colors dialogs: `ui.colorBalanceDialog` opens Adjust Colors (`colorBalance`: fields `mode` `gray`/`rgb`/`cmyk`/
`global`, the channels `r` `g` `b` / `c` `m` `y` `k` / `gray` / `tint` (global mode) from −100 to 100, `convert`, `fill`, `stroke`,
`preview`) and `ui.saturateDialog` opens Saturate (`saturate`: `intensity` −100..100, `preview`). Both preview on the
canvas while open; `ui.dialog.confirm` keeps the result as one undo step (`edit.colors.adjustBalance` /
`edit.colors.saturate`) and `ui.dialog.cancel` rolls it back. Global mode answers with an error until tints of global
and spot colours exist; the dialog then stays open.

Recolor Artwork: `ui.recolorDialog {colors?, library?, group?}` opens the `recolor` dialog on the selected art
(`colors`: a count for an n-colour job, or colours to assign as the new colours; `library`: Limit to Library, a library
or `"document"`, `""` for the first library; `group`: a colour group, Edit or Apply Color Group). Fields: `rows` (`[{from: [keys], to: key, exclude?}]`
as `recolor.reduce` returns them), `colors` (null for Auto), `method`, `preserveWhite`, `preserveBlack`,
`preserveGrays`, `limitTo`, `groupName`, `tab` (`assign` or `edit`), `rule` (harmony id), `linked`, `preview`. Setting
`colors`, a preserve flag or `limitTo` reduces the rows again (also on `ui.dialog.confirm`). The dialog previews
`recolor.apply` on the canvas; confirm keeps it as one undo step (with a group, the group is rewritten in the same
step; with no art selected only the group changes). In the Swatches panel, double-clicking a colour group's folder,
or the options button while a group is selected, opens it.

Library panel: `engine.execute {command: "window.swatchLibrary", params: {library}}` opens the read-only library
panel on a swatch library (`ui.inspect` shows it as `library_panel: {kind, id}`; `library: null` closes it).
Clicking a swatch there runs `swatch.library.add {library, names: [name], apply}` (the active proxy, or the other
one with Alt), so one undo step adds and applies it; Shift/Cmd-clicks select swatches and colour groups for
Add to Swatches.
`window.swatchLibrary.other {path?}` loads a library file (or another document's swatches) and opens it there;
opening a `.vcswatches`, `.gpl` or `.ase` file with `app.open` does the same. `ui.saveSwatchLibrary {names?}` opens the
`saveSwatchLibrary` dialog (fields `name`, `format`: `vcswatches`/`gpl`/`css`, `user`: save to the user library
folder, `selectedOnly` with `names`); `ui.dialog.confirm` runs `swatch.library.save` (to a file it asks for a path).

Graphic style libraries open in the same panel: `window.graphicStyleLibrary {library}` (`library_panel: {kind:
"graphicStyles", id}`; `library: null` closes it). Clicking a style there runs `graphicStyle.addFromLibrary {library,
name, apply: true, add}` (`add` with Alt), adding and applying it in one undo step; Shift/Cmd-clicks select styles
for Add to Graphic Styles. `window.graphicStyleLibrary.other {path?}` loads a `.vcstyles` file (or another
document's graphic styles) and opens it there; opening a `.vcstyles` file with `app.open` does the same. The User
Defined libraries are `window.userGraphicStyleLibrary1`…`10` in Window → Graphic Style Libraries.
`ui.saveGraphicStyleLibrary {names?}` opens the `saveGraphicStyleLibrary` dialog (fields `name`, `user`: save to the
user library folder, `selectedOnly` with `names`); `ui.dialog.confirm` runs `graphicStyle.saveLibrary` (to a file it
asks for a path).

Tile Edge Color: Object → Pattern → Tile Edge Color… (`ui.tileEdgeColor`) opens the `tileEdgeColor` dialog (field
`color`: `#rrggbb` or a preset name such as "Light Blue"); `ui.dialog.confirm` sets the preference
`patternTileEdgeColor` (`prefs.set`), the colour pattern editing mode draws the tile edge and the swatch bounds in.

Window title bar: on Windows and Linux the window has no OS decorations and the app bar is the title bar. Its
caption buttons (Minimize, Maximize/Restore, Close) sit at the bar's right end, 46 pt each; they are window chrome,
not commands, so drive them with `ui.click` if needed. Close runs `app.quit` (the same `saveChanges` questions for
modified documents), and empty bar space and the 5 pt window edges move and resize the window. macOS and the web
build are unchanged.

Fill/Stroke chips and panel shortcuts: the Control bar's and Properties' Fill and Stroke chips bring their proxy
forward (`paint.toggleActive {fill}`) and open a popover with the Swatches panel (Shift-click: the Color panel's mixer);
a swatch clicked there runs `paint.setFill` / `paint.setStroke`. The Properties panel has them with nothing selected
too (they set up the next object), with the Control bar's Stroke link (the Stroke panel as a popover) and weight
spinner. Panel keys (Brushes F5, Color F6, Layers F7, Color Guide Shift+F3, Graphic Styles Shift+F5, Appearance
Shift+F6, Align Shift+F7, Transform Shift+F8, Info Cmd+F8, Gradient Cmd+F9, Pathfinder Cmd+Shift+F9, Stroke Cmd+F10,
Transparency Cmd+Shift+F10, Attributes Cmd+F11, Symbols Cmd+Shift+F11, Character Cmd+T, Paragraph Cmd+Alt+T, Tabs
Cmd+Shift+T, OpenType Cmd+Alt+Shift+T; Cmd is Ctrl on Windows and Linux) run `window.panel {panel}`, can be changed in
Edit › Keyboard Shortcuts (entry `panel:<id>`, every panel) and pressed with `ui.key`; `ui.menu.list` shows them on
the Window menu's items.
`window.panel` takes a panel id in any case or the panel's display label (`"Layers"`, `"Color Guide"`).

Collapsing the dock: `window.collapseDock {collapsed?}` (the » at the top of the dock; omitted toggles) hides the
Properties | Layers | Libraries group and puts its three panels as icons at the top of the icon column, under a «
that expands them again. While collapsed, those icons and `window.panel {panel: "properties"|"layers"|"libraries"}`
pop the panel out next to the column like the other icon panels (`ui.dock_collapsed`, `ui.open_panel` in
`ui.inspect`); expanding with one popped out shows its tab. The state is saved with the preferences and in user
workspaces; the built-in workspaces expand the dock.

Floating tool groups: dragging or clicking the tear-off bar down a tool group's flyout (or releasing the long press
that opened it over the bar) floats the group as a strip of tool buttons, moved by the same bar and put back in the
toolbar by the × at its top; `window.floatTools {tool, floating?}` does the same for the group of the current
layout holding `tool` (omitted toggles; a tool alone in its slot is an error). While a group floats, the presses
that open its flyout raise the strip instead. The strips (`ui.floating_flyouts` in `ui.inspect`: the group's tools
and the strip's top-left corner) are saved with the preferences and in user workspaces; the built-in workspaces
float none.

The Contextual Task Bar: its handle (the grip at its left end) drags it anywhere on the canvas, which always holds
it whole, also when the window shrinks. Unpinned, the bar keeps that offset as it follows the selection, until
another document becomes active. Its More Options (…) menu has Hide Bar (`window.taskBar`); Pin Bar Position
(`window.taskBar.pin {pinned?}`, omitted toggles, checked while pinned), which holds the bar where it is across
selection changes (the handle still moves it) and, turned off, lets it follow the selection again from there; Reset
Bar Position (`window.taskBar.reset`), which unpins it and puts it back under the selection; and Show Properties
Panel. As in Illustrator, neither the position nor the pin is saved: each launch starts with the bar under the
selection.

The Free Transform widget: while the Free Transform tool (E) is active, a small box at the canvas's top left holds
Constrain over the tool's three modes, Free Transform, Perspective Distort and Free Distort. The buttons set the tool's
options, as `tool.setOption {key: "mode", value: "free"|"perspective"|"distort"}` and `{key: "constrain", value}` do:
Constrain acts as Shift held (proportional scaling, moves and rotations by 45°) and does nothing in the distort modes,
where it is greyed. The modifier keys still work while dragging (Cmd on a corner distorts it freely, Cmd+Alt+Shift in
perspective, Cmd on a side shears).

Floating panels: dragging a dock tab, a panel icon or a popped-out panel's title out of the dock floats that panel
inside the window, and the strip right of the dock's tabs floats the whole Properties | Layers | Libraries group. A
floating group moves by its title bar (or the strip right of its tabs); a tab dragged out of it floats on its own;
dropped on another group's title bar or tabs it stacks with that group, and dropped on the dock (lit up while the
pointer is over it) or closed with its × its panels go back to the tabbed group or the icon column.
`window.panel.float {panel, x?, y?, onto?, group?}` floats a panel (with `group`, its whole group) with its top-left
corner at `x`, `y` (window points; default cascaded), or stacks it with the floating group holding `onto`;
`window.panel.dock {panel, group?}` puts it back. `panel` takes the `window.panel` ids and labels, and `"tools"` for
the Tools panel, which floats by its title bar and docks on the window's left edge. `window.panel` on a floating
panel shows its tab. `ui.floating_panels` in `ui.inspect` lists the groups (panel ids, the index of the tab shown,
the top-left corner) and `ui.toolbar_pos` the Tools panel's corner (null: docked); both are saved with the
preferences and in user workspaces (the built-in workspaces dock everything), and a corner saved on a bigger window
is clamped into this one when drawn. Floating panels stay inside the app window (no separate OS windows).

Flatten Transparency: `ui.flattenTransparencyDialog` opens the `flattenTransparency` dialog for the selection
(fields `preset`: a preset name, setting it loads that preset's options; the option keys of
`object.flattenTransparency`: `balance` 0–100, `lineArtPpi` and `gradientPpi` 1–2400, `textToOutlines`,
`strokesToOutlines`, `clipComplexRegions`, `antiAlias`, `preserveAlpha`, `preserveOverprints`; `preview`, off at first).
With `preview` on it flattens on the canvas while open; `ui.dialog.confirm` keeps the result as one undo step and
`ui.dialog.cancel` rolls it back. Out-of-range values answer with an error and the dialog stays open.

Flattener presets: `ui.flattenerPresetsDialog {selected?}` opens Edit → Transparency Flattener Presets (dialog
`flattenerPresets`). Fields: `selected` (a preset name), then the selected preset's `name` and option keys. On a saved
preset, `ui.dialog.set` of an option saves it and a new `name` renames it (both through `flattener.presets.save`);
built-in presets don't change. New, Delete, Import… and Export… are `flattener.presets.save {preset}`,
`flattener.presets.delete`, `flattener.presets.import` and `flattener.presets.export`; `ui.dialog.confirm` closes.
Opening a `.vcflattener` file with `app.open` imports its presets. In the Flatten Transparency dialog, Save Preset…
keeps the dialog's options as a saved preset.

PDF presets: `ui.pdfPresetsDialog {selected?}` opens Edit → PDF Presets (dialog `pdfPresets`, field `selected`: a
preset name). Built-in presets are read-only. New… and Edit… open the preset editor, which `ui.pdfPresetDialog {name?
(a saved preset to edit) | preset? (the preset a new one starts from)}` opens too: dialog `pdfPreset`, the Save PDF
dialog's option fields plus `name` and `description`; `ui.dialog.confirm` runs `pdf.preset.save` and returns to PDF
Presets (a new preset can't take a name in use). Delete, Import… and Export… are `pdf.preset.delete`,
`pdf.preset.import` and `pdf.preset.export`; `ui.dialog.confirm` closes. Opening a `.vcpdfpresets` file with `app.open`
imports its presets. In the Save PDF dialog, Save Preset… shows a name field (`__savePresetAs`): while it shows,
`ui.dialog.confirm` saves the dialog's settings as that preset and selects it instead of writing the PDF.

Flattener Preview: `window.panel {panel: "flattenerPreview"}` shows the panel; `ui.flattenerPreview {highlight?,
overprints?, preset?, options?, showOptions?}` sets it (a preset loads its options, `options` adjust them), shows it
and refreshes its snapshot of the document, answering as `flattener.preview` does. `ui.inspect` reports the settings
as `ui.flattener_preview`. The panel keeps its snapshot until Refresh (it notes when the document or the settings
changed since); its preview greys the art and paints the highlighted objects or areas red.

Expand: Object → Expand… (`ui.expandDialog`, or `object.expand` without params from the menus, palette and shortcuts)
opens the `expand` dialog for the selection. Fields: `object`, `fill`, `stroke` (all on; `__object`, `__fill`,
`__stroke` say which the selection has something for, as `object.expand.info` does, and the others are disabled),
`gradient` (`objects` or `mesh`) and `steps` (1–1000, default 255). `ui.dialog.confirm` runs `object.expand` with them as
one undo step.

Attributes panel: `window.panel {panel: "attributes"}` (Cmd+F11) shows it; its controls run `attributes.set`,
`path.reverse {reversed}` and `path.setFillRule {rule}`, and its Browser button runs the UI command
`attributes.openUrl {url?}`, which opens the URL (default: the selection's) in the web browser and answers `{url}`.

Spot Colors: the Swatches panel menu's Spot Colors… (`ui.spotColors`) opens the `spotColors` dialog (field `useLab`:
true shows and separates spot colours defined in Lab from their Lab values, false from their working-CMYK
equivalents); `ui.dialog.confirm` runs `swatch.spotOptions {useLab}` as one undo step.

Scale options: `ui.menuDialog {command}` opens the dialog a command's menu item opens (`object.scale` → `scale`,
`object.transformEach` → `transformEach`, also Move, Rotate, Reflect, Shear and the Path dialogs). The Scale dialog
(`scale`) and Object → Transform → Transform Each… (dialog `transformEach`: `scaleH`,
`scaleV` %, `moveH`, `moveV` pt, `rotate` °, `reflectX`, `reflectY`, `random`, `reference` 0–8, `copy`, `preview`)
have the fields `corners` (Scale Corners) and `strokes` (Scale Strokes & Effects). They start from the preferences
`scaleCorners` and `scaleStrokes`, and `ui.dialog.confirm` keeps them as the preferences (`prefs.set`) before running
`object.scale` / `object.transformEach` with them. The Transform panel's checkboxes and flyout item and the Properties
panel's checkbox set the same preferences through `prefs.set`.

Use Preview Bounds: the Align panel flyout's Use Preview Bounds item shows and toggles the `usePreviewBounds`
preference (`prefs.set`); while it is on, the bounding box and the Transform panel, Properties and Control bar
fields measure visual bounds.

Width Point Edit: double-clicking a width point with the Width tool, or `ui.widthPointEdit {id, index}`, opens the
`widthPoint` dialog (fields `id`, `index`, `t`, `side1` and `side2`: the left and right widths in points, `linked`,
`adjustAdjoining`). `ui.dialog.confirm` runs `stroke.widthPoint.set` with them; `ui.dialog.set {field: "discard",
value: true}` then confirm (the Delete button) removes the point with `stroke.widthPoint.remove`.

Corners: double-clicking a Live Corners widget with the Selection or Direct Selection tool, or `ui.corners {id?,
corners?}`, opens the `corners` dialog for a path's corners (the Direct-Selected ones, else every corner; fields `id`,
`corners`: anchor indices of the path with its corners uncut, a rectangle's 0–3 clockwise from the top-left, `kind`:
round, invertedRound or chamfer, `radius` in points; `kind` or `radius` is absent while the corners differ). `ui.dialog.confirm` runs `object.setLiveShape` with them.

Perspective plane options: double-clicking a plane widget of the perspective grid, or `ui.perspectivePlane {plane}`,
opens the `perspectivePlane` dialog (fields `plane`: left, right or ground; `location`: points along the plane's
normal; `objects`: none, move or copy). `ui.dialog.confirm` runs `perspective.plane.move` with them.

Units: dialog distance fields (Move's `dx`/`dy`, shape sizes, Offset Path's `offset`, Split Into Grid's `gutter`,
Artboard Options' sizes, effects' distances, `transformEach`'s `moveH`/`moveV`, New Document's `width`/`height`, the
length preferences in `preferences`) show in the General unit (the active document's units, or `unitsGeneral` without a
document). Through `ui.dialog.set` a number is in points and a string may carry its unit (`"10 mm"`; a bare number in a
string is points). The `preferences` dialog's `unitsGeneral` starts as the active document's units, and OK sets them.
`newDocument` has `units` (a unit label; print presets start in `unitsGeneral`, screen presets in Pixels).

Document Setup: File → Document Setup… (`file.documentSetup`, Cmd+Alt+P) opens the `documentSetup` dialog. Its fields
are what `document.setup` reports (`units`, `bleed` [top, bottom, left, right] in pt, `outlineImages`,
`highlightSubstitutedFonts`, `highlightSubstitutedGlyphs`, `gridSize`, `gridColors`, `simulatePaper`, `flattenerPreset`,
`discardWhiteOverprint`, `language`, `quotes`, `typographersQuotes`, `superscript`, `subscript`, `smallCapsSize`,
`exportText`, `backgroundContents`) plus `tab` (`General`, `Transparency` or `Type`) and `bleedLinked`.
`ui.dialog.confirm` runs `document.setup` with them as one undo step; a bad value answers with an error and the dialog
stays open.

New Document: File → New… (`file.newDialog`, Cmd+N) opens the `newDocument` dialog. Its fields are `file.new`'s params
(`preset`, `name`, `width` and `height` in pt, `units`, `artboards`, `artboardLayout`, `bleed`, `backgroundContents`,
`colorMode`, `rasterEffectsPpi`, `previewMode`) plus `category` (the tab: Recent, Saved, Mobile, Web, Print, Film &
Video, Art & Illustration, Branding, Social), `advanced` (Advanced Options open), `bleedLinked` and `presetName`
(Save Preset's name). Setting `preset` to a name from `file.newPresets` fills the other fields from that preset;
changing a field afterwards makes the settings custom (`preset` becomes ""). More Settings is the `newDocumentMore`
dialog over the same fields (its Profile is `category`). `ui.dialog.confirm` runs `file.new` with the fields (with
`previewMode: "pixel"` the app turns Pixel Preview on); on an error the dialog stays open.

Swatch Conflict: through the app, when the pasted objects' global or spot swatches conflict with the document's
(see `clipboard.conflicts`) and the params have no `swatchConflict`, every Paste command opens the `swatchConflict`
dialog instead and answers `{dialog: "swatchConflict"}`. Fields: `conflicts` (`[{name, document, clipboard,
spot}]`), `index` (the conflict asked about), `choice` (`merge` or `add`), `applyToAll` and `choices` (the answers so
far). `ui.dialog.confirm` records the answer and asks about the next conflict (with `applyToAll`, answers the rest
the same); the last one runs the paste with `swatchConflict`. `ui.dialog.cancel` pastes nothing.

Paste placement: through the app, `edit.paste` and `edit.pasteWithoutFormatting` without `center`, `dx` or `dy`
paste at the centre of the view. The Paste menu items are enabled while the system clipboard holds something to
paste (SVG, PDF, text or a bitmap on Windows; text or a bitmap on macOS; text on Linux, whose paste keys still take a
bitmap; SVG on the web, whose paste keys also take pictures and files), even with nothing copied in the app (looked
at up to four times a second; `ui.menu.list` shows it). The Layers panel menu (≡ on
the dock's tab strip while Layers shows) lists the layer commands (New Layer…, Duplicate and Delete Selection,
Options for Selection…, clipping mask, isolation, Locate Object, Merge Selected, Flatten Artwork, Collect in New
Layer, Release to Layers, Reverse Order, Template, Hide/Outline/Lock Others or Show/Preview/Unlock All Layers), Paste
Remembers Layers (`layer.pasteRemembersLayers`, checked when on) and Panel Options…. Layer Options is the
`layerOptions` dialog (`ui.layerOptions {ids?}`, `ui.newLayer {sublayer?}`), Panel Options the `layersPanelOptions`
dialog (`ui.layersPanelOptions`), and `ui.layersExpand {ids?, open?}` opens or closes rows; see the Layers panel
section of `docs/mcp.md` for the row commands (`layer.setCurrent`, `layer.highlight`, `layer.move`…).

File Info: File → File Info… (`file.info` from the menu or Cmd+Alt+Shift+I; `ui.fileInfoDialog` for agents) opens the
`fileInfo` dialog. Its fields are what `file.info` reports (`title`, `author`, `authorTitle`, `description`,
`keywords` (a list: the chips), `rating`, `copyrightStatus`, `copyrightNotice`, `copyrightUrl`; `created` and
`modified` are shown read-only) plus `__keyword`, keywords typed but not added yet (comma-separated; OK adds them).
`ui.dialog.confirm` runs `file.info` with them as one undo step; a bad value answers with an error and the dialog
stays open.

Document Raster Effects Settings: Effect → Document Raster Effects Settings… (`ui.rasterEffectsSettingsDialog` for
agents) opens the `rasterEffectsSettings` dialog. Its fields are what `document.rasterEffectsSettings` reports
(`resolution`, `colorModel`, `background`, `antiAlias`, `clippingMask`, `addAround` in pt, `preserveSpotColors`);
`ui.dialog.confirm` runs it with them as one undo step; a bad value keeps the dialog open. Object → Rasterize… opens a
`command` dialog for `object.rasterize` whose fields (`ppi`, `colorModel`, `background`, `antiAlias`, `clippingMask`,
`addAround`) start from these settings.

Missing linked files: `app.open` of a document whose linked images' files can't be found (`document.open`'s
`missingLinks`) opens the `missingLinks` dialog for the first (fields `missing`: `[{name, path, ids}]`, `modified`,
`applyToAll`, `path`). `ui.dialog.set {field: "path", value}` then `ui.dialog.confirm` replaces it with that file
(`links.relink`; without `path` a file is picked), and with `applyToAll` the others are looked for by name in the same
folder; `ui.dialog.set {field: "discard", value: true}` then confirm ignores it (the images keep their preview), with
`applyToAll` the rest too. The next missing file is asked about after each answer; `ui.dialog.cancel` stops asking.
Then, with the preference `updateLinks: "askWhenModified"`, modified linked files are offered for update in a
`confirm` dialog whose `ui.dialog.confirm` runs `links.update`.

Missing fonts: once the missing linked file questions are answered, or at once when there are none, `app.open` of a
document whose type uses fonts that aren't available (`text.missingFonts`) opens the `missingFonts` dialog, one document
at a time, and so does `links.editOriginal` of a placed document. When another dialog takes its place (Import PDF for a
file opened next, for example), the dialog opens again once that one closes. Fields: `document` (the document's uid),
`fonts` (`[{family, style, status, resolved}]`), `fontsNextToDocument` (the `Fonts` folder next to the document, or
empty), `state` (empty, `searching`, `done`, `stopped` or `failed`), `search` (the search's `id`), `folder`, `found`
(`text.findFontFiles`'s `fonts`, with the files found), `searched`, `skipped`, `unreadable`, `stopped`, `error` and
`chosen` (the files to add; a font's first file is chosen when the search finds it).
`ui.dialog.set {field: "searchFolder", value}` then `ui.dialog.confirm` searches that folder (`text.findFontFiles`),
keeps the dialog open and clears `searchFolder`, also when the search doesn't start. The fields change as frames draw
the dialog; `text.findFontFiles {}` reports the search directly. `ui.dialog.confirm` without `searchFolder` copies the
files in `chosen` (`text.addFontFiles`) and closes the dialog, and `ui.dialog.cancel` closes it and stops its search.
`ui.missingFontsDialog {folder?}` opens the dialog for the active document, searching `folder` when given.
`ui.findFontsInFolder {folder?}` (Find in Folder… in Type › Find Font…, shown when the document misses fonts) asks for a
folder, then opens the dialog searching it. The web shows a status line instead of the dialog, and both commands are
disabled there.

Text Import Options: placing a `.txt` file through the app (`file.place` with a file and no `text` options, the Place
dialog, a drop) opens the `textImport` dialog (fields `platform`: `windows`/`mac`, `characterSet`: `unicode`/`ansi`,
`removeLineReturns`, `removeParagraphReturns`, `replaceSpaces`, `spaces`). `ui.dialog.confirm` places the file as area
type with those options (`file.place {text: {…}}`).

System clipboard formats: through the desktop app, `edit.copy` and `edit.cut` publish what `clipboard.flavours`
lists to the system clipboard, and every Paste command first loads what another app copied (`clipboard.importSvg`,
`importPdf`, `importText` or `importImage`, centred in the view); an import error answers the Paste with that error
and pastes nothing. Cmd+V also pastes a bitmap or PDF with no text beside it (on the key's release, as egui sends no
Paste event for it). Screenshots paste on every platform: Windows reads `PNG`, else `CF_DIBV5`/`CF_DIB`, macOS the
pasteboard's PNG or TIFF, Linux `image/png`, and the web the picture the browser's paste event carries (#457).

File menu details: `file.openRecent1`…`file.openRecent30` open the recent files the `recentFilesCount` preference
lists (0 hides them all; `file.recentFiles` returns the listed paths), `file.reveal` shows the document's file in the
system file manager (desktop only; answers `{path}`), and `file.newFromTemplate` without params opens a file picker
in the Templates folder (preference `templatesFolder`). Older native files open as "<name> [Converted]" (preference
`appendConverted`), so their Save asks for a new `.vectorcraft` name. Native files reopen at the view they were saved
with.

DXF Options: Export As… with the DXF format (`file.exportAs {format: "dxf"}`, then the save dialog), or
`ui.dxfOptionsDialog {path?, …document.exportDxf options}` for agents, opens the `dxfOptions` dialog. Its fields are
`document.export` params: `version`, `unit` (a unit name such as `Millimeters`), `scale`, `scaleLineweights`, `colors`
(`8`, `16`, `256`, `true`), `rasterFormat` (`png`, `jpeg`), `preserve` (`appearance`, `editability`), `selectedOnly`,
`alterPaths`, `outlineText`, plus `path` and the artboard choice. `ui.dialog.confirm` checks them (a bad value, or
Export Selected Art Only with nothing selected, keeps the dialog open), remembers them for the next DXF Options and
writes the file (asking for a path when there is none). Export As also lists DWG, greyed out: setting its `format` to
`dwg` shows, and on confirm answers, the hint to export DXF instead.

DXF Import Options: opening a DXF drawing through the app (`app.open`, `file.open`, a drop) or placing one
(`file.place` with a `.dxf` file and no `dxf` options, which answers `{"dialog": "dxfImport"}`) opens the `dxfImport`
dialog (`mode`: `open` or `place`; `layouts`, `units` and `version` describe the drawing). Its fields are the
`document.open` `dxf` options: `layout` (one of `layouts`), `fit`, `unit` (a unit name such as `Millimeters`) and
`scale` (1 unit = `scale` drawing units; ignored with `fit`), `scaleLineweights`, `center`, `mergeLayers`.
`ui.dialog.confirm` opens or places the drawing (placing keeps the other `file.place` params in `__place`) and
remembers `fit`, `scaleLineweights`, `center` and `mergeLayers` for the next drawing; an error (an unknown layout,
art too large at that scale) keeps the dialog open.

Placement Options: the Links panel's flyout (or `ui.placementOptionsDialog {ids?}` for agents) opens the
`placementOptions` dialog for the selected images (fields `ids`, `preserve`: `transforms`/`bounds`/`fileDimensions`/
`fit`/`fill`, `align`: `topLeft` … `bottomRight`, `clip`). `ui.dialog.confirm` runs `links.placementOptions` with them as
one undo step. The Links panel (`window.panel {panel: "links"}`) shows `links.list` and the first selected image's
`links.info`; `links.editOriginal` and `links.reveal` (Edit › Edit Original, Show in Folder) hand the linked file to
the system and return `{path}`.

Package: File → Package… (`ui.packageDialog`) opens the `package` dialog for a saved document (a document never saved
gets a `confirm` dialog whose OK runs `file.saveAs`). Fields: `folder` (the location), `name` (the package folder),
`copyLinks`, `linksFolder`, `relink`, `copyFonts`, `report`. `ui.dialog.confirm` saves unsaved changes, runs
`file.package` and then opens a `confirm` dialog whose OK runs `file.showPackage {folder}`; the web downloads the zip
instead. Document Info's flyout picks a category and Save… runs `docInfo.save {path?, selectionOnly?}`.

Slices: through the app, `object.slice.options` and `object.slice.divide` with no params open their dialogs on the
selected slices. `sliceOptions` (fields `kind`: `image`/`noImage`/`htmlText`, `name`, `url`, `target`, `message`,
`alt`, `text`, `background`: `""`/`matte`/`#rrggbb`, `hAlign`, `vAlign`; `__html` says whether HTML Text applies)
runs `object.slice.options` on `ui.dialog.confirm`; `divideSlices` (fields `divideRows`, `rows`, `rowMode`:
`count`/`size`, `rowHeight`, and `divideColumns`, `columns`, `columnMode`, `columnWidth`) runs `object.slice.divide`.
Read slice options with `slice.list`.

The Slice tools: `ui.pointer` drags with the `slice` tool make a user slice (Shift square, Alt from the centre; one
undo step). With `sliceSelection` a click selects a slice (Shift toggles), a drag moves the selected slices, a drag on
a selected user slice's handle resizes it, a double click opens `sliceOptions` and `ui.key` Delete deletes the
selected slices. Hidden or locked slices (`view.slices.hide`, `view.slices.lock`) can't be picked.

EPS Options: Export As… with the EPS format (`file.exportAs {format: "eps"}`, then the save dialog), or
`ui.epsOptionsDialog {path?, …document.exportEps options}` for agents, opens the `epsOptions` dialog. Its fields are
`document.export` params: `previewFormat` (`none`, `tiffBw`, `tiffColor`), `transparentPreview`, `overprints`
(`preserve`, `discard`), `flattenerPreset` (a built-in or saved preset's name), `includeLinkedFiles`, `thumbnails`,
`cmykPostScript`, `compatibleGradients`, `level` (`2`, `3`), plus `path` and the artboard choice (`useArtboards`,
`range`). `ui.dialog.confirm` checks them (a bad value or an unknown preset keeps the dialog open), remembers them for
the next EPS Options and writes the file(s), asking for a path when there is none.

Data Recovery: at launch, copies left behind by a crash open the `recovery` dialog (never copies another running
instance keeps, so instances started side by side for screenshots don't get it). Fields: `copies` (`[{file, title,
saved}]`, from `file.recovery.list`) and `discard`. `ui.dialog.confirm` restores them all (`file.recovery.restore`
each, opening "<name> [Recovered]" documents), `discard: true` then confirm deletes them, `ui.dialog.cancel` keeps them
for the next launch. While copies are written in the background `ui.inspect` lists them under `background`
("Saving recovery data for <name>").

Native save options: File → Save As to a native or `.ai` file opens the `saveOptions` dialog after the save panel
(heading "VectorCraft Options" or "PDF-compatible .ai Options"). Fields: `separateArtboards`, `all` (true: every
artboard; false: `range`, such as "1-3, 5"), `includeLinked`, `embedProfiles`, `pdfCompatible`, `compress`, and for
native files `version` and `preview`. A bad range keeps the dialog open; `ui.dialog.confirm` writes the file and the
artboards' files.

EMF and WMF: Export As… lists both formats (`file.exportAs {format: "emf"}` or `"wmf"`); they have no options
dialog, so `ui.dialog.confirm` asks for the path and writes the file (one per artboard with Use Artboards). Open and
Place read `.emf` and `.wmf`; a paste of `image/emf` from a system clipboard service runs `clipboard.importEmf`.

TIFF, BMP and Targa Options: Export As… with the TIFF, BMP or Targa format (`file.exportAs {format: "tiff"}`, then
the save dialog) opens `tiffOptions`, `bmpOptions` or `tgaOptions`: the raster rows of PNG Options (`ppi`,
`background`, `antiAlias`), then TIFF's `colorModel` (`rgb`, `cmyk`, `gray`; a CMYK document starts on `cmyk`),
`byteOrder` (`little`, `big`), `lzw` and `embedIcc`; BMP's `colorModel` (`rgb`, `gray`), `fileFormat` (`windows`,
`os2`), `depth` (1, 4, 8, 16, 24, 32), `reduction` and `dither` (4 and 8 bits), `rle` and `flipRows`; Targa's `depth`
(16, 24, 32). The BMP dialog keeps its choices writable together (OS/2 falls back to 24 bits and turns RLE and flipped
rows off; RLE turns flipped rows off). `ui.dialog.set` the fields and `ui.dialog.confirm` writes the file(s) with
`document.export`.

Save for Web: File → Export → Save for Web (Legacy)… (`file.saveForWeb`, Ctrl/Cmd+Alt+Shift+S) opens the `saveForWeb`
dialog on the remembered settings (`webExport.settings`). Its fields are the `document.exportForWeb` settings
(`format`, `colors`, `reduction`, `dither`, `quality`, `colorTable`, `width`, `percent`, `output`…; set `preset` to load
a preset) plus `__view` (`original`, `optimized`, `2up`, `4up`), `__zoom` (`fit` or a percentage), `__kbps` and
`__color` (the colour table's selected colour). `ui.dialog.confirm` remembers the settings and saves (a save panel for
the image, or the page with HTML output; the web downloads the files); with `discard: true` it is Done: it remembers
the settings and closes. `file.saveForWeb` with settings saves without the dialog, and `file.saveForWeb.browser`
writes the HTML page to a temporary folder and opens it in the default browser (desktop).

PSD Options: Export As… with the PSD format (`file.exportAs {format: "psd"}`, then the save dialog) opens `psdOptions`:
the raster rows of PNG Options (`ppi`, `background`, `antiAlias`), then `colorModel` (`rgb`, `cmyk`, `gray`; a CMYK
document starts on `cmyk`), `layers` (true: Write Layers; false: Flat Image, whose background list starts at White),
`maxEditability` and `hiddenLayers` (with layers) and `embedIcc`. `ui.dialog.set` the fields and `ui.dialog.confirm`
writes the file(s) with `document.export`.

Print: File → Print… (`file.print` with no params, Ctrl/Cmd+P) opens the `print` dialog on the document's print
settings. Its fields are the `print.setup` settings (`copies`, `artboards`, `range`, `media`, `orientation`,
`placement`, `scaling`, `marks`, `bleed`, `output`, `graphics`, `color`, `margin`…; a section is an object, as
`{field: "marks", value: {"trim": true}}`) plus `printer` (a name from `print.printers`, `""` the system's default) and
`toFile` (save the job as a PDF), and the UI-only `__section` (General, Marks and Bleed, Output, Graphics, Color
Management, Advanced, Summary) and `__sheet` (the page the preview shows, 0-based). `ui.dialog.confirm` (Print) keeps
the settings with the document (`print.setup`, one undo step) and prints them: to the printer through the system's
print queue (the web: the browser's print dialog), else, with `toFile` or no printing available, a save panel for the
PDF. With `discard: true` it is Done: it only keeps the settings. Settings that can't print (a missing artboard, a
range of no tiles) answer with an error and keep the dialog open. `file.print {settings?, printer?, toFile?, path?}`
prints without the dialog; `print.printers` lists the printers and `print.printerSetup {printer?}` opens their system
settings (Setup…).

Print presets: in the `print` dialog, `preset` names the print preset whose settings were loaded last; setting it
(`ui.dialog.set {field: "preset", value: "Posters"}`) loads that preset's settings, and the list reads `[Custom]` once
they differ. Save Preset… shows a name field (`__savePresetAs`): while it shows, `ui.dialog.confirm` saves the dialog's
settings as that preset (`print.presets.save`) and selects it instead of printing. `ui.printPresetsDialog {selected?}`
opens Edit → Print Presets (dialog `printPresets`, field `selected`); `[Default]` is protected. New… and Edit… open the
preset editor, which `ui.printPresetDialog {name? (a saved preset to edit) | preset? (the preset a new one starts
from)}` opens too: dialog `printPreset`, the Print dialog's settings fields plus `name`; `ui.dialog.confirm` runs
`print.presets.save` and returns to Print Presets (a new preset can't take a name in use). Delete, Import… and
Export… are `print.presets.delete`, `print.presets.import` and `print.presets.export`; `ui.dialog.confirm` closes.
Opening a `.vcprintpresets` file with `app.open` imports its presets.

Modifiers on synthetic input: `ui.key`, `ui.click`, `ui.drag` and `ui.wheel` take `shift`, `alt`, `ctrl` and `cmd` (Command on
macOS, Ctrl elsewhere), and the app holds them for the frames the input spans, as if the keys were down: a key's
press, its `text` and its release; a click's press and release; a drag from the press through every move to the
release. Every handler sees them (Shift+arrow nudges by the big increment, Alt+arrow nudges a copy, Shift-clicking a
Layers selection square adds to the selection); the keyboard's own modifiers apply again afterwards.

Cutting tools: Mirror & Cut (`mirrorCut`), Line Cut (`lineCut`) and Rectangle Cut (`rectCut`) cut the selected paths
and compound paths. `ui.tool.select` one, then drag with `ui.pointer`: the cut previews live and the release keeps it
as one undo step (`ui.key` Escape cancels). They run `path.mirrorCut`, `path.lineCut` and `path.rectCut`, which an
agent can call directly. Mirror & Cut's tool options (`tool.setOption`, shown in the Control bar) are `axis`
(`free` | `vertical` | `horizontal`; a constrained axis follows the pointer and a click places it) and `keep`
(`left` | `right` | `top` | `bottom`); Alt on release keeps the other side.

Puppet Warp (`puppetWarp`): with art selected, the tool shows pins at once (automatic ones at the centre and the end
of each limb until pins are placed). `ui.pointer` clicks on the art add pins (each new pin is selected; Shift-click
adds a pin to the selection or takes it out), dragging a pin moves every selected pin and warps the art, and
Alt-dragging near (not on) a selected pin turns the art around it. A click off the mesh adds nothing: the cursor shows
not-allowed there and the canvas says so. `ui.key` Delete or Backspace removes the selected pins. Adding, deleting and
each drag are one undo step each, running `object.puppetWarp {rest: true, …}`; the pins live in the document, so
Undo/Redo take them back with the art, they stay across tool switches while the selection is unchanged, and another
selection starts afresh. `object.puppetWarp.pins` reads them. Its tool options (`tool.setOption`, shown in the
Control bar) are `expand` (Expand Mesh, points, 0 allowed), `showMesh` (on by default) and `selectAllPins` (`true`
selects every pin, `false` none); `ui.inspect` → `toolOptions` lists the `pins` the tool last showed and the
`selected` ones.

Colour adjustment effects (Effect → Color Adjustments) open the `effect` dialog like the other effects
(`effect.dialog {effect: "adjust.hueSaturation"}`): its fields are the effect's parameters (sliders for the amounts,
`channel` for Curves and Levels, `points` for Curves, `color` for Shift to Color) and they preview live; set them with
`ui.dialog.set` and `ui.dialog.confirm` to apply (Curves also has a graph: drag its points). Object → Vector
Halftone… (`engine.execute {command: "ui.menuDialog", params: {command: "object.vectorHalftone"}}`) opens the
`vectorHalftone` dialog (fields `shape`, `frequency`, `angle`, `mode`, `color`, `invert`, `clip`, `keepOriginal`,
`preview`): it previews live and `ui.dialog.confirm` keeps the halftone as one undo step.

Tool options last: `tool.setOption {key, value}` or `{values: {…}}` (and `tool` to set another tool's) sets them, and
the options a tool keeps (Liquify brush and tool options, Mirror & Cut, Puppet Warp, Symbolism, drawing tools) stay
when you switch tools or documents and are saved with the preferences. The Liquify tools share their Global Brush
Dimensions (`width`, `height`, `angle`, `intensity`).

Perspective grid: View → Perspective Grid → Define Grid… (`ui.perspectiveGridDialog`) opens the `perspectiveGrid`
dialog prefilled from the grid. Its fields are those of `perspective.grid.define` (`kind`, `units`, `scale`,
`gridline`, `angle`, `distance`, `horizonHeight`, `thirdVp`, `leftColor`, `rightColor`, `groundColor`, `opacity`;
lengths in `units`, so setting `units` converts them). `ui.dialog.confirm` runs `perspective.grid.define`; a refused
value keeps the dialog open.

Object → Envelope Distort's dialogs open through their menu items or `ui.menuDialog {command}`. Make with Warp…
(`object.envelope.makeWithWarp`) opens `envelopeWarp` (fields `style`, `horizontal`, `bend`, `h`, `v`, `preview`, and
`reset`); while an envelope is selected the menu shows Reset with Warp… instead, and the same item, shortcut or
`object.envelope.resetWithWarp` opens it with `reset: true` and the envelope's values. Make with Mesh… / Reset with
Mesh… open `envelopeMesh` (`rows`, `cols`, `reset`, and `maintainShape` when resetting). Envelope Options…
(`object.envelope.options`) opens `envelopeOptions` (`antiAlias`, `preserveShape`, `fidelity`, `distortAppearance`,
`distortLinearGradients`, `distortPatternFills`) with the selected envelope's values, or with nothing selected the
defaults for new envelopes. They preview live while an envelope is involved, and `ui.dialog.confirm` keeps the result
as one undo step.

Liquify Tool Options: double-clicking a Liquify tool (`tool.options {tool: "twirl"}`) opens a `liquifyOptions` dialog.
Its fields are `tool`, the Global Brush Dimensions `width`, `height` (pt), `angle`, `intensity` (%) and `usePressure`,
then the tool's options `detail`, `simplify` and `simplifyOn` (Warp, Twirl, Pucker, Bloat), `rate` (Twirl),
`complexity`, `affectAnchors`, `affectIn` and `affectOut` (Scallop, Crystallize, Wrinkle), `horizontal` and
`vertical` (Wrinkle, %), and `showBrush`. `ui.dialog.confirm` runs `tool.setOption {tool, values}`. `ui.pointer`
events take `pressure` (0..1, default 1): it is the Liquify intensity while Use Pressure Pen is on.

Freehand Tool Options: double-clicking the Pencil, Paintbrush, Smooth, Blob Brush or Eraser tool
(`tool.options {tool: "pencil"}`) opens a `freehandOptions` dialog. Its fields are `tool` and the options the tool
keeps: `fidelity` (pt; Pencil, Paintbrush, Smooth), `fill` (Pencil, Paintbrush), `closeWithin` and `editWithin` (screen
pixels, 0 turns it off; Pencil, Paintbrush) and `size` (pt; Blob Brush, Eraser). `ui.dialog.confirm` runs
`tool.setOption {tool, values}`.

`ui.pointer` events also take `holdMs` (0..60000): the pointer then holds still that long, button down, before the
next event. Twirl, Pucker and Bloat keep applying while held (a repeat of the last point every 0.1 s), exactly as
for the same time held with the mouse. Liquify leaves type, symbols, images, graphs, meshes, envelopes, repeats and
blends as they are and says so in the status bar.

Perspective grid presets: in the `perspectiveGrid` dialog, `name` is the preset whose fields are loaded (setting the
fields of a preset, as the Preset menu does, keeps its name; other changes clear it, which reads [Custom]). Save
Preset… sets `__mode: "save"` and `__from: "define"` with `name` the new preset's name: `ui.dialog.confirm` then saves
the fields as that preset (`perspective.presets.save`) and returns to Define Grid on it. View → Perspective Grid →
Save Grid as Preset… (`ui.savePerspectivePreset`) opens the same dialog in `save` mode; OK saves and closes. Edit →
Perspective Grid Presets… (`ui.perspectivePresetsDialog {selected?}`) opens dialog `perspectiveGridPresets` (field
`selected`): New… and Edit… open the preset editor (`perspectiveGrid` with `__mode: "edit"`, `__original` the preset
edited), whose OK runs `perspective.presets.save` and comes back; Delete, Import… and Export… are
`perspective.presets.delete`, `import` and `export`. View → Perspective Grid → One/Two/Three Point Perspective list
that type's built-in views and the saved presets (`ui.perspectiveUserPreset<type>.<n>`, hidden while empty). Opening a
`.vcperspective` file with `app.open` imports its presets.

Blend Options (Object › Blend › Blend Options…, `ui.blendOptions`, the Blend tool's double-click, Alt-click and
toolbar button) opens the `blendOptions` dialog on the selected blend's options: `spacing` (`smooth` | `steps` |
`distance`), `steps`, `distance` (pt), `orientation` (`page` | `path`) and `preview`; it previews live and
`ui.dialog.confirm` keeps the change as one undo step. With no blend selected (`__target: "defaults"`) OK sets what
new blends start with. The Blend tool (`ui.tool.select {tool: "blend"}`): `ui.pointer` down on an object, then on
another, blends them; down on an anchor point blends from that point; each further object clicked joins the blend.
A selected blend shows its spine. With Direct Selection (`directSelection`) a `ui.pointer` drag from a spine point
moves it (`object.blend.spine.moveAnchor`, one undo step) and, once a point is clicked, from one of its handles
reshapes the curve; Direct and Group Selection pick a blend's key objects. With the Pen tool a click on a selected
blend's spine adds a point (on a point no key sits on, deletes it).

Perspective grid view options: View → Perspective Grid lists Show/Hide Grid, Show/Hide Rulers, Snap to Grid
(checked), Lock/Unlock Grid and Lock Station Point (checked); `ui.menu.list` reports the current labels. They run
`perspective.grid.show`, `rulers`, `snap`, `lock` and `lockStation`.

Perspective grid widgets: the Plane Switching Widget is fixed in a corner of the canvas (`ui.pointer` with
`space: "screen"` reaches it at the same pixels whatever the zoom); a press on it with any tool picks the plane while
the grid shows. `ui.key` with `key` `"1"`…`"4"` picks the left, horizontal, right or no plane. Double-clicking the
Perspective Grid tool (`tool.options {tool: "perspectiveGrid"}`) opens `perspectiveGridOptions` (fields `show`,
`position`: `topLeft`, `topRight`, `bottomLeft`, `bottomRight`); `ui.dialog.confirm` runs `perspective.widget.options`.

Envelopes on the canvas: `pointer_gesture` with the Mesh tool (`mesh`) or Direct Selection on a selected mesh envelope
drags its points (a press on a point focuses it, and a press on one of the focused point's handle ends drags that
handle); a Mesh tool click inside a mesh envelope adds a row and a column. While an envelope or its content is
selected the Control bar shows its controls (Edit Envelope / Edit Contents, warp style, orientation, bend and
distortions or mesh rows and columns, Reset, Envelope Options); each runs an `object.envelope.*` command.
