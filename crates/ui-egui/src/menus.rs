//! The menu bar (Illustrator's menu tree) and UI-level commands.
//!
//! Items bound to a command id run through [`VectorcraftApp::run`]. Items not implemented yet are
//! listed (disabled, with their shortcut) so the full surface is visible and discoverable; the
//! parity tracker drives them to "done".

use serde::Serialize;
use serde_json::{Value, json};

use vectorcraft_engine::cmd::fileio::SaveMode;

use crate::VectorcraftApp;
use crate::io;
use crate::panels::character::Face;
use crate::state::{DockTab, next_zoom};
use crate::theme::{self, Brightness, Tokens};
use crate::widgets;

#[derive(Clone, Debug)]
pub enum Item {
    /// (label, command id, params)
    Cmd(&'static str, &'static str, Value),
    /// Not implemented yet: (label, shortcut)
    Todo(&'static str, &'static str),
    Sub(&'static str, Vec<Item>),
    Sep,
    /// Section header (disabled label, e.g. "Vector Effects").
    Header(&'static str),
}

fn c(label: &'static str, id: &'static str) -> Item {
    Item::Cmd(label, id, Value::Null)
}
fn cp(label: &'static str, id: &'static str, p: Value) -> Item {
    Item::Cmd(label, id, p)
}
fn todo(label: &'static str) -> Item {
    Item::Todo(label, "")
}
fn todos(label: &'static str, sc: &'static str) -> Item {
    Item::Todo(label, sc)
}
fn sub(label: &'static str, items: Vec<Item>) -> Item {
    Item::Sub(label, items)
}
/// Window → … Libraries until the code-generated libraries land (disabled entries).
fn library_placeholders() -> Vec<Item> {
    vec![todo("Built-in Libraries"), todo("User Defined"), Sep, todo("Other Library…")]
}
use Item::Sep;

/// UI-level commands: (id, label, shortcut, params doc).
pub const UI_COMMANDS: &[(&str, &str, &str, &str)] = &[
    (
        "app.language",
        "Interface Language",
        "",
        "{lang: auto|<code>} the interface language, persisted as the `interfaceLanguage` preference (`auto` follows the system locale; codes: prefs.list › interfaceLanguage, e.g. en, ja, cs, es, zh-hant)",
    ),
    (
        "file.open",
        "Open…",
        "Cmd+O",
        "{path?} → with a path, document.open's result {index, title, format, warnings, …} (null when a dialog asks first or a library loads)",
    ),
    (
        "file.save",
        "Save",
        "Cmd+S",
        "{path?, format?, options?, svg?: {…SVG options}} = document.save, written through the app (a document saved as SVG, PDF or .ai, or opened from one Save can write back, saves as that again, with the same options); with no path known (never saved, converted) the Save As panel asks first",
    ),
    (
        "file.place",
        "Place…",
        "Cmd+Shift+P",
        "{path? | paths?} no file: pick files, then the Place dialog (Link, Template, Replace); paths: that dialog for them; path (or name+dataBase64, …file.place params): file.place, centred in the view (a PDF with several pages or a password and no page opens the Place PDF dialog first)",
    ),
    ("file.openRecent1", "Open Recent File 1", "", "{}"),
    ("file.openRecent2", "Open Recent File 2", "", "{}"),
    ("file.openRecent3", "Open Recent File 3", "", "{}"),
    ("file.openRecent4", "Open Recent File 4", "", "{}"),
    ("file.openRecent5", "Open Recent File 5", "", "{}"),
    ("file.openRecent6", "Open Recent File 6", "", "{}"),
    ("file.openRecent7", "Open Recent File 7", "", "{}"),
    ("file.openRecent8", "Open Recent File 8", "", "{}"),
    ("file.openRecent9", "Open Recent File 9", "", "{}"),
    ("file.openRecent10", "Open Recent File 10", "", "{}"),
    ("view.goto1", "Saved View 1", "", "{} go to the 1. saved view"),
    ("view.goto2", "Saved View 2", "", "{} go to the 2. saved view"),
    ("view.goto3", "Saved View 3", "", "{} go to the 3. saved view"),
    ("view.goto4", "Saved View 4", "", "{} go to the 4. saved view"),
    ("view.goto5", "Saved View 5", "", "{} go to the 5. saved view"),
    ("view.goto6", "Saved View 6", "", "{} go to the 6. saved view"),
    ("view.goto7", "Saved View 7", "", "{} go to the 7. saved view"),
    ("view.goto8", "Saved View 8", "", "{} go to the 8. saved view"),
    ("view.goto9", "Saved View 9", "", "{} go to the 9. saved view"),
    ("view.goto10", "Saved View 10", "", "{} go to the 10. saved view"),
    ("select.recall1", "Saved Selection 1", "", "{} select the 1. saved selection"),
    ("select.recall2", "Saved Selection 2", "", "{} select the 2. saved selection"),
    ("select.recall3", "Saved Selection 3", "", "{} select the 3. saved selection"),
    ("select.recall4", "Saved Selection 4", "", "{} select the 4. saved selection"),
    ("select.recall5", "Saved Selection 5", "", "{} select the 5. saved selection"),
    ("select.recall6", "Saved Selection 6", "", "{} select the 6. saved selection"),
    ("select.recall7", "Saved Selection 7", "", "{} select the 7. saved selection"),
    ("select.recall8", "Saved Selection 8", "", "{} select the 8. saved selection"),
    ("select.recall9", "Saved Selection 9", "", "{} select the 9. saved selection"),
    ("select.recall10", "Saved Selection 10", "", "{} select the 10. saved selection"),
    ("select.recall11", "Saved Selection 11", "", "{} select the 11. saved selection"),
    ("select.recall12", "Saved Selection 12", "", "{} select the 12. saved selection"),
    ("select.recall13", "Saved Selection 13", "", "{} select the 13. saved selection"),
    ("select.recall14", "Saved Selection 14", "", "{} select the 14. saved selection"),
    ("select.recall15", "Saved Selection 15", "", "{} select the 15. saved selection"),
    ("select.recall16", "Saved Selection 16", "", "{} select the 16. saved selection"),
    ("select.recall17", "Saved Selection 17", "", "{} select the 17. saved selection"),
    ("select.recall18", "Saved Selection 18", "", "{} select the 18. saved selection"),
    ("select.recall19", "Saved Selection 19", "", "{} select the 19. saved selection"),
    ("select.recall20", "Saved Selection 20", "", "{} select the 20. saved selection"),
    ("select.recall21", "Saved Selection 21", "", "{} select the 21. saved selection"),
    ("select.recall22", "Saved Selection 22", "", "{} select the 22. saved selection"),
    ("select.recall23", "Saved Selection 23", "", "{} select the 23. saved selection"),
    ("select.recall24", "Saved Selection 24", "", "{} select the 24. saved selection"),
    ("select.recall25", "Saved Selection 25", "", "{} select the 25. saved selection"),
    ("type.recentFont1", "Recent Font 1", "", "{} apply the 1. most recently used font"),
    ("type.recentFont2", "Recent Font 2", "", "{} apply the 2. most recently used font"),
    ("type.recentFont3", "Recent Font 3", "", "{} apply the 3. most recently used font"),
    ("type.recentFont4", "Recent Font 4", "", "{} apply the 4. most recently used font"),
    ("type.recentFont5", "Recent Font 5", "", "{} apply the 5. most recently used font"),
    ("type.recentFont6", "Recent Font 6", "", "{} apply the 6. most recently used font"),
    ("type.recentFont7", "Recent Font 7", "", "{} apply the 7. most recently used font"),
    ("type.recentFont8", "Recent Font 8", "", "{} apply the 8. most recently used font"),
    ("type.recentFont9", "Recent Font 9", "", "{} apply the 9. most recently used font"),
    ("type.recentFont10", "Recent Font 10", "", "{} apply the 10. most recently used font"),
    ("type.recentFont11", "Recent Font 11", "", "{} apply the 11. most recently used font"),
    ("type.recentFont12", "Recent Font 12", "", "{} apply the 12. most recently used font"),
    ("type.recentFont13", "Recent Font 13", "", "{} apply the 13. most recently used font"),
    ("type.recentFont14", "Recent Font 14", "", "{} apply the 14. most recently used font"),
    ("type.recentFont15", "Recent Font 15", "", "{} apply the 15. most recently used font"),
    ("file.clearRecent", "Clear Recent Files", "", "{}"),
    ("type.findFont", "Find Font…", "", "{} open the Find Font dialog (engine: text.fonts / text.replaceFont / select.font)"),
    (
        "ui.missingFontsDialog",
        "Missing Fonts Dialog",
        "",
        "{folder?} open Missing Fonts (dialog `missingFonts`) for the fonts the active document uses that aren't available (text.missingFonts); with folder, it looks for their files there at once (text.findFontFiles). An error when no font is missing. Opening a document whose fonts are missing shows it after the missing linked file questions (dialog `missingLinks`). Disabled on the web",
    ),
    (
        "ui.findFontsInFolder",
        "Find Fonts in Folder…",
        "",
        "{folder?} Type › Find Font's Find in Folder…: asks for a folder (or takes folder), then opens Missing Fonts looking there for the files of the fonts the active document misses (text.findFontFiles). Disabled where there is no folder picker (the web)",
    ),
    ("file.recentFiles", "Recent Files", "", "{} → [path…] most recent first"),
    (
        "file.export.svg",
        "Export As SVG…",
        "",
        "{} opens SVG Options; with params = document.export {path?, svg?: {…SVG options}, range?…} (a .svgz path writes it gzipped)",
    ),
    ("file.export.png", "Export As PNG…", "", "{path?, …document.export options}; no path: pick the file, then PNG Options"),
    (
        "file.exportForScreens",
        "Export for Screens…",
        "Cmd+Alt+E",
        "{} opens the dialog (on the document's last settings); with params = document.exportForScreens (no folder on the web: downloads the files or the zip; openLocation: shows the first file in the file manager)",
    ),
    ("file.documentSetup", "Document Setup…", "Cmd+Alt+P", "{}"),
    ("file.newDialog", "New…", "Cmd+N", "{} opens the New Document dialog"),
    ("app.home", "Home", "", "{} shows the Home screen (new file presets, Open) over the open documents; choosing a document tab returns to it"),
    ("edit.preferences", "Preferences…", "Cmd+K", "{category?} open Preferences (engine: prefs.get / prefs.set / prefs.list)"),
    ("edit.keyboardShortcuts", "Keyboard Shortcuts…", "Cmd+Alt+Shift+K", "{}"),
    ("shortcuts.set", "Set Keyboard Shortcut", "", "{id: command id or tool:<id>, shortcut: \"Cmd+Shift+K\" | \"\" (none) | null (default), force?}"),
    ("shortcuts.list", "List Keyboard Shortcuts", "", "{query?} → [{id, label, group, shortcut, default, overridden}]"),
    ("shortcuts.conflicts", "Keyboard Shortcut Conflicts", "", "{} → [{shortcut, ids}]"),
    ("shortcuts.reset", "Reset Keyboard Shortcuts", "", "{}"),
    (
        "shortcuts.preset",
        "Keyboard Shortcut Set",
        "",
        "{name: \"VectorCraft Defaults\" | \"Classic Defaults\"} (names of earlier versions are accepted)",
    ),
    ("shortcuts.export", "Export Keyboard Shortcuts…", "", "{path?}"),
    ("shortcuts.import", "Import Keyboard Shortcuts…", "", "{path? | data?}"),
    ("view.outline", "Outline", "Cmd+Y", "{} toggle Outline/Preview"),
    ("view.pixelPreview", "Pixel Preview", "Cmd+Alt+Y", "{}"),
    ("view.trimView", "Trim View", "", "{} toggle: hide everything outside the artboards"),
    ("view.cornerWidget", "Hide Corner Widget", "", "{} toggle the live corner widgets"),
    ("view.snapToPixel", "Snap to Pixel", "", "{} toggle: drawing and moving land on whole pixels"),
    ("view.textThreads", "Hide Text Threads", "Cmd+Shift+Y", "{} toggle the thread lines between threaded text frames"),
    ("view.gradientAnnotator", "Hide Gradient Annotator", "Cmd+Alt+G", "{} toggle the Gradient tool's annotator"),
    ("type.hiddenCharacters", "Show Hidden Characters", "Cmd+Alt+I", "{} toggle markers for spaces, paragraph ends and story ends"),
    (
        "type.bold",
        "Bold",
        "",
        "{} the selected type (or the Type tool's selection) in its family's Bold face, or back to the regular face when it is bold; keeps italics. A family without that face is left as it is, with a message. While the Type tool edits text, Cmd+Shift+B",
    ),
    (
        "type.italic",
        "Italic",
        "",
        "{} the selected type (or the Type tool's selection) in its family's Italic (or Oblique) face, or back upright when it is italic; keeps the weight. A family without that face is left as it is, with a message. While the Type tool edits text, Cmd+Shift+I",
    ),
    ("effect.last", "Last Effect…", "Cmd+Alt+Shift+E", "{} open the dialog of the last effect applied"),
    ("view.zoomIn", "Zoom In", "Cmd+=", "{}"),
    ("view.zoomOut", "Zoom Out", "Cmd+-", "{}"),
    ("view.fitArtboard", "Fit Artboard in Window", "Cmd+0", "{}"),
    ("view.fitAll", "Fit All in Window", "Cmd+Alt+0", "{}"),
    ("view.actualSize", "Actual Size", "Cmd+1", "{}"),
    ("view.setZoom", "Set Zoom", "", "{zoom: percent, center?: [x,y]}"),
    (
        "view.goToArtboard",
        "Go to Artboard",
        "",
        "{index: 0-based number | \"first\" | \"previous\" | \"next\" | \"last\"} make that artboard the status bar navigator's (the one Fit Artboard in Window and Actual Size show) and fit it in the window → {index}",
    ),
    ("view.edges", "Hide Edges", "Cmd+H", "{}"),
    ("view.artboards", "Hide Artboards", "Cmd+Shift+H", "{}"),
    ("view.rulers", "Show Rulers", "Cmd+R", "{}"),
    ("view.boundingBox", "Hide Bounding Box", "Cmd+Shift+B", "{}"),
    ("view.guides", "Hide Guides", "Cmd+;", "{}"),
    ("view.smartGuides", "Smart Guides", "Cmd+U", "{}"),
    ("view.grid", "Show Grid", "Cmd+'", "{}"),
    ("view.snapToGrid", "Snap to Grid", "Cmd+Shift+'", "{}"),
    ("view.snapToPoint", "Snap to Point", "Cmd+Alt+'", "{}"),
    ("view.presentation", "Presentation Mode", "Shift+F", "{}"),
    ("view.screenMode", "Screen Mode", "F", "{mode?: 0..2} (no param cycles)"),
    ("view.rotateReset", "Reset Rotate View", "Cmd+Shift+1", "{}"),
    ("window.control", "Control", "", "{}"),
    ("window.toolbar", "Tools", "", "{}"),
    (
        "window.toolbarColumns",
        "Toolbar: Single/Double Column",
        "",
        "{double?: bool} show the toolbar's tools in two columns (true), one (false) or toggle (omitted), as the double arrow at the top of the toolbar does; returns the new state",
    ),
    ("window.toolbarAdvanced", "Toolbar: Advanced / Basic", "", "{}"),
    (
        "window.floatTools",
        "Float Tool Group",
        "",
        "{tool: id, floating?: bool} float the toolbar group holding `tool` (in the current Basic or Advanced layout) as its own strip of tool buttons (true), put it back in the toolbar (false) or toggle (omitted), as dragging or clicking a flyout's tear-off bar and the strip's × do; returns the new state",
    ),
    ("window.taskBar", "Contextual Task Bar", "", "{}"),
    (
        "window.taskBar.pin",
        "Pin Bar Position",
        "",
        "{pinned?: bool} keep the Contextual Task Bar where it is instead of following the selection (true), let it follow the selection again from where it is (false) or toggle (omitted), as the bar's More Options menu does; returns the new state",
    ),
    ("window.taskBar.reset", "Reset Bar Position", "", "{} unpin the Contextual Task Bar and put it back under the selection"),
    ("window.dock", "Panels", "Tab", "{} show/hide all panels"),
    ("window.panel", "Show Panel", "", "{panel: id} e.g. layers, swatches, stroke (case-insensitive; display labels like \"Layers\" work too)"),
    (
        "window.panel.float",
        "Float Panel",
        "",
        "{panel: id or \"tools\", x?, y?, onto?: id, group?: bool} float a panel out of the dock (or out of its floating group) as its own floating group with its top-left corner at x, y in window points (default: cascaded), as dragging its tab out of the dock does; `onto` stacks it with the floating group holding that panel instead; `group` takes its whole group (its floating group, or the Properties | Layers | Libraries tabs left in the dock); a panel alone in its group is moved. \"tools\" floats the Tools panel. Returns {panel, floating, group, pos}",
    ),
    (
        "window.panel.dock",
        "Dock Panel",
        "",
        "{panel: id or \"tools\", group?: bool} put a floating panel (with `group`, its whole group) back in the dock, as dropping it on the dock or its group's × does: Properties, Layers and Libraries in the tabbed group, the other panels in the icon column; \"tools\" docks the Tools panel at the window's left edge",
    ),
    (
        "window.collapseDock",
        "Collapse Panels to Icons",
        "",
        "{collapsed?: bool} collapse the dock's Properties | Layers | Libraries group to icons (true), expand it (false) or toggle (omitted), as the double arrow at the top of the dock does; returns the new state",
    ),
    ("window.brightness", "UI Brightness", "", "{brightness: dark|mediumDark|mediumLight|light}"),
    ("window.workspace", "Workspace", "", "{name} switch workspace (Essentials, Essentials Classic, Painting, …)"),
    ("window.workspace.reset", "Reset Essentials", "", "{} reset the current workspace"),
    ("window.workspace.new", "New Workspace…", "", "{name?} save the current layout"),
    ("window.workspace.manage", "Manage Workspaces…", "", "{}"),
    ("window.workspace.delete", "Delete Workspace", "", "{name}"),
    ("window.workspace.rename", "Rename Workspace", "", "{name, to}"),
    ("window.workspace.list", "List Workspaces", "", "{}"),
    ("window.newWindow", "New Window", "", "{}"),
    ("tool.select", "Select Tool", "", "{tool: id} (see tools)"),
    (
        "tool.setOption",
        "Tool Option",
        "",
        "{key, value} | {values: {key: value…}}, tool?: id (default: the active tool) → the tool's options (`{}` reads them). The options a tool keeps (Liquify brush and tool options, Mirror & Cut, Puppet Warp, drawing tools…) last across tool switches and are saved with the preferences; another tool's are stored for when it is chosen",
    ),
    (
        "effect.dialog",
        "Effect…",
        "",
        "{effect: id, index?: int (edit that applied effect in place, prefilled; OK runs effect.setParams), item?: appearance item index|null (the fill/stroke whose effects, null the object's; default: the Appearance panel's active item)} open the effect's dialog with live preview; without `index`, an effect already in that list first opens `effectExists` (confirm edits it, discard adds another) → {pending: \"effectExists\"}",
    ),
    ("ui.paramDialog", "Command Dialog", "", "{command, label?, params} open a parameter dialog for any command"),
    (
        "ui.recolorDialog",
        "Recolor Artwork…",
        "",
        "{colors?: n (an n-colour job: n rows, Scale Tints) | [colour] (new colours to assign, in order; with no art selected and no group they are the rows, and OK saves them as a new colour group, field `groupName`: the Color Guide's Edit or Apply Colors), library?: id or name, or \"document\" (Limit to Library; \"\" the first library), group?: colour group (Edit or Apply Color Group: OK rewrites the group with the new colours and recolours the selected art, if any)} open Recolor Artwork (dialog `recolor`; engine: recolor.reduce / recolor.apply)",
    ),
    ("effect.applyLast", "Apply Last Effect", "Cmd+Shift+E", "{}"),
    (
        "file.export.pdf",
        "Save as PDF…",
        "",
        "{} opens the Save PDF dialog; with params = document.exportPdf options written to path (asked when missing; viewAfterSaving opens the file) → {path, bytes, warnings}",
    ),
    ("help.about", "About VectorCraft", "", "{}"),
    ("help.commandPalette", "Search Commands…", "Cmd+Shift+/", "{}"),
    ("app.quit", "Quit VectorCraft", "Cmd+Q", "{}"),
    (
        "ui.swatchOptions",
        "Swatch Options…",
        "",
        "{name} edit a swatch: Swatch Options for a colour (dialog `swatchOptions`, engine: swatch.edit) or a gradient (the same dialog with `name` only; it shows the gradient), pattern editing for a pattern",
    ),
    (
        "ui.newSwatch",
        "New Swatch…",
        "",
        "{spot?, group?: colour group name} open New Swatch for the active fill or stroke (dialog `newSwatch`, engine: swatch.new)",
    ),
    (
        "ui.newColorGroup",
        "New Color Group…",
        "",
        "{swatches?: [names]} open New Color Group, from those swatches or the selected artwork (dialog `newColorGroup`, engine: swatch.newGroup)",
    ),
    (
        "ui.colorPicker",
        "Color Picker…",
        "",
        "{stroke?: bool (default: the active proxy), color?: \"#rrggbb\"|[r,g,b]|{c,m,y,k}|{gray} (default: the proxy's colour)} open the Color Picker (fields: hex or color, channel, webOnly, swatches); OK runs paint.setFill / paint.setStroke",
    ),
    (
        "ui.graphicStyleOptions",
        "Graphic Style Options…",
        "",
        "{name?} open Graphic Style Options (dialog `graphicStyleOptions`, field `name`): for style `name` OK renames it (graphicStyle.rename); without, OK makes a new style of that name from the selection (graphicStyle.new)",
    ),
    (
        "tool.options",
        "Tool Options…",
        "",
        "{tool: id} what double-clicking a tool button opens: hand → fits the artboard in the window (view.fitArtboard), zoom → 100% (view.actualSize), rotate|scale|reflect|shear → that Object › Transform dialog (dialog `rotate`, `scale`, `reflect` or `shear`; error `nothing selected` without a selection), selection|directSelection|groupSelection → the Move dialog (dialog `move`; the same error), gradient → the Gradient panel (window.panel), eyedropper → Eyedropper Options (dialog `eyedropperOptions`, fields sampleSize, pickUp, apply; OK runs eyedropper.setOptions), printTiling → resets the print tiling (print.tiling.set {reset: true}), warp|twirl|pucker|bloat|scallop|crystallize|wrinkle → that tool's Tool Options (dialog `liquifyOptions`, fields tool, width, height, angle, intensity %, usePressure, detail, simplify, simplifyOn, rate, complexity, horizontal %, vertical %, affectAnchors, affectIn, affectOut, showBrush; OK runs tool.setOption {tool, values}), pencil|paintbrush|smooth|blobBrush|eraser → that tool's Tool Options (dialog `freehandOptions`, fields tool and the options the tool keeps: fidelity (pt), fill, closeWithin and editWithin (px, 0 is off), size (pt); OK runs tool.setOption {tool, values})",
    ),
    (
        "ui.colorGuideOptions",
        "Color Guide Options…",
        "",
        "{} open Color Guide Options (dialog `colorGuideOptions`, fields `steps` 1–20 and `amount` 0–100): OK sets the Color Guide panel's variation grid (read back with `ui.inspect`: ui.color_guide; engine: color.harmony)",
    ),
    (
        "ui.colorBalanceDialog",
        "Adjust Color Balance…",
        "",
        "{} open Adjust Colors for the selection (dialog `colorBalance`, fields `mode` gray|rgb|cmyk|global, channels r g b / c m y k / gray / tint (global) −100..100, `convert`, `fill`, `stroke`, `preview`): previews live, OK runs edit.colors.adjustBalance as one undo step",
    ),
    (
        "ui.saturateDialog",
        "Saturate…",
        "",
        "{} open Saturate for the selection (dialog `saturate`, fields `intensity` −100..100, `preview`): previews live, OK runs edit.colors.saturate as one undo step",
    ),
    (
        "window.swatchLibrary",
        "Swatch Library",
        "",
        "{library: id or name (see swatch.library.list) | null (close)} open the read-only library panel on a swatch library (UI state `library_panel`); clicking a swatch there runs swatch.library.add with apply → {open, name, count}",
    ),
    (
        "window.swatchLibrary.other",
        "Other Library…",
        "",
        "{path?} (default: pick a file) load a .vcswatches, .gpl or .ase library, or another document's swatches (engine: swatch.library.load), and open it in the library panel",
    ),
    (
        "ui.saveSwatchLibrary",
        "Save Swatch Library…",
        "",
        "{names?: [the swatches selected in the Swatches panel]} open Save Swatch Library (dialog `saveSwatchLibrary`: name, format: vcswatches|gpl|css, user: save to the user library folder, selectedOnly); OK runs swatch.library.save",
    ),
    ("window.userSwatchLibrary1", "User Swatch Library 1", "", "{} open the 1. User Defined swatch library (swatch.library.list, category user)"),
    ("window.userSwatchLibrary2", "User Swatch Library 2", "", "{} open the 2. User Defined swatch library"),
    ("window.userSwatchLibrary3", "User Swatch Library 3", "", "{} open the 3. User Defined swatch library"),
    ("window.userSwatchLibrary4", "User Swatch Library 4", "", "{} open the 4. User Defined swatch library"),
    ("window.userSwatchLibrary5", "User Swatch Library 5", "", "{} open the 5. User Defined swatch library"),
    ("window.userSwatchLibrary6", "User Swatch Library 6", "", "{} open the 6. User Defined swatch library"),
    ("window.userSwatchLibrary7", "User Swatch Library 7", "", "{} open the 7. User Defined swatch library"),
    ("window.userSwatchLibrary8", "User Swatch Library 8", "", "{} open the 8. User Defined swatch library"),
    ("window.userSwatchLibrary9", "User Swatch Library 9", "", "{} open the 9. User Defined swatch library"),
    ("window.userSwatchLibrary10", "User Swatch Library 10", "", "{} open the 10. User Defined swatch library"),
    (
        "ui.mergeGraphicStyles",
        "Merge Graphic Styles…",
        "",
        "{names: [two or more styles]} open Graphic Style Options (dialog `graphicStyleOptions`, field `name`) to name the style OK merges from them (graphicStyle.merge)",
    ),
    (
        "ui.layerOptions",
        "Options for Selection…",
        "",
        "{ids?|id?} open Layer Options for Layers panel rows (default: the highlighted rows, else the current layer), filled in from the first: dialog `layerOptions`, fields name, color (a preset name such as \"Light Blue\" or #rrggbb), template, locked, visible, printable, preview, dimImages (bool), dimPercent (0–100); an object's row has name, visible and locked. OK runs layer.setProps (one undo step)",
    ),
    (
        "ui.newLayer",
        "New Layer Options…",
        "",
        "{sublayer?: bool} open Layer Options for a new layer (or a sublayer of the current layer), named and coloured as it would be: dialog `layerOptions` with mode new|newSublayer; OK runs layer.new or layer.newSublayer with the fields",
    ),
    (
        "ui.layersPanelOptions",
        "Panel Options…",
        "",
        "{} open the Layers panel's Panel Options: dialog `layersPanelOptions`, fields layersOnly, rowSize (small|medium|large|other), otherSize (12–100 pt), thumbLayers, thumbGroups, thumbObjects; OK applies them (kept with the UI state)",
    ),
    (
        "ui.layersExpand",
        "Expand Layers Panel Rows",
        "",
        "{ids?: [id…] (default: every row that holds others), open?: bool (default true)} open or close rows of the Layers panel, as clicking their triangles does → {count}",
    ),
    (
        "ui.tileEdgeColor",
        "Tile Edge Color…",
        "",
        "{} open Tile Edge Color (dialog `tileEdgeColor`, field `color`: #rrggbb or a preset name such as \"Light Blue\"); OK sets the preference patternTileEdgeColor (prefs.set), the colour of the tile edge and swatch bounds in pattern editing mode",
    ),
    (
        "ui.flattenTransparencyDialog",
        "Flatten Transparency…",
        "",
        "{} open Flatten Transparency for the selection (dialog `flattenTransparency`, fields `preset` (a preset name: setting it loads its options), balance 0..100, lineArtPpi, gradientPpi 1..2400, textToOutlines, strokesToOutlines, clipComplexRegions, antiAlias, preserveAlpha, preserveOverprints, `preview` (off at first)): OK runs object.flattenTransparency with those options as one undo step",
    ),
    (
        "ui.flattenerPresetsDialog",
        "Transparency Flattener Presets…",
        "",
        "{selected?: preset name} open the presets manager (dialog `flattenerPresets`, fields `selected`, then the selected preset's `name` and option keys: setting them on a saved preset saves it, a new `name` renames it; built-in presets don't change). New, Delete, Import… and Export… run flattener.presets.save / delete / import / export",
    ),
    (
        "ui.flattenerPreview",
        "Flattener Preview",
        "",
        "{highlight?: none|rasterizedRegions|transparentObjects|allAffected|expandedPatterns|outlinedStrokes|outlinedText|allRasterized, overprints?: preserve|simulate|discard, preset?: name (loads its options), options?: {option keys, over the preset's}, showOptions?} set the Flattener Preview panel (ui.inspect: ui.flattener_preview), show it and refresh its snapshot → what flattener.preview answers for it",
    ),
    (
        "window.graphicStyleLibrary",
        "Graphic Style Library",
        "",
        "{library: id or name (see graphicStyle.libraries) | null (close)} open the read-only library panel on a graphic style library (UI state `library_panel`, kind graphicStyles); clicking a style there runs graphicStyle.addFromLibrary with apply (Alt: add) → {open, name, count}",
    ),
    (
        "window.graphicStyleLibrary.other",
        "Other Library…",
        "",
        "{path?} (default: pick a file) load a .vcstyles library, or another document's graphic styles (engine: graphicStyle.loadLibrary), and open it in the library panel",
    ),
    (
        "ui.saveGraphicStyleLibrary",
        "Save Graphic Style Library…",
        "",
        "{names?: [the styles selected in the Graphic Styles panel]} open Save Graphic Style Library (dialog `saveGraphicStyleLibrary`: name, user: save to the user library folder, selectedOnly); OK runs graphicStyle.saveLibrary",
    ),
    (
        "window.userGraphicStyleLibrary1",
        "User Graphic Style Library 1",
        "",
        "{} open the 1. User Defined graphic style library (graphicStyle.libraries, category user)",
    ),
    ("window.userGraphicStyleLibrary2", "User Graphic Style Library 2", "", "{} open the 2. User Defined graphic style library"),
    ("window.userGraphicStyleLibrary3", "User Graphic Style Library 3", "", "{} open the 3. User Defined graphic style library"),
    ("window.userGraphicStyleLibrary4", "User Graphic Style Library 4", "", "{} open the 4. User Defined graphic style library"),
    ("window.userGraphicStyleLibrary5", "User Graphic Style Library 5", "", "{} open the 5. User Defined graphic style library"),
    ("window.userGraphicStyleLibrary6", "User Graphic Style Library 6", "", "{} open the 6. User Defined graphic style library"),
    ("window.userGraphicStyleLibrary7", "User Graphic Style Library 7", "", "{} open the 7. User Defined graphic style library"),
    ("window.userGraphicStyleLibrary8", "User Graphic Style Library 8", "", "{} open the 8. User Defined graphic style library"),
    ("window.userGraphicStyleLibrary9", "User Graphic Style Library 9", "", "{} open the 9. User Defined graphic style library"),
    ("window.userGraphicStyleLibrary10", "User Graphic Style Library 10", "", "{} open the 10. User Defined graphic style library"),
    (
        "ui.expandDialog",
        "Expand…",
        "",
        "{} open Expand for the selection (dialog `expand`, fields object, fill, stroke (all on; one the selection has nothing for is disabled, see object.expand.info), gradient: objects|mesh, steps 1..1000 (255)): OK runs object.expand with them as one undo step",
    ),
    (
        "attributes.openUrl",
        "Browser",
        "",
        "{url?} open url, else the selection's URL (attributes.info), in the web browser (the Attributes panel's Browser button) → {url}",
    ),
    (
        "ui.spotColors",
        "Spot Colors…",
        "",
        "{} open Spot Colors (dialog `spotColors`, field `useLab`: true shows and separates Lab spot colours from their Lab values, false from their CMYK equivalents); OK runs swatch.spotOptions as one undo step",
    ),
    (
        "ui.menuDialog",
        "Menu Dialog",
        "",
        "{command: object.move|object.rotate|object.scale|object.reflect|object.shear|object.transformEach|path.average|object.path.offsetPath|object.path.simplify|object.path.splitIntoGrid|object.vectorHalftone|artboard.rearrange|object.envelope.makeWithWarp|object.envelope.resetWithWarp|object.envelope.makeWithMesh|object.envelope.resetWithMesh|object.envelope.options} open the dialog that command's menu item opens (dialog kind: move, rotate, scale, reflect, shear, transformEach, …, vectorHalftone, rearrangeArtboards, envelopeWarp, envelopeMesh, envelopeOptions; Scale and Transform Each have `corners` and `strokes`, from the preferences, which OK updates; the envelope dialogs start from the selected envelope, Make opens as Reset (`reset: true`) while one is selected)",
    ),
    (
        "ui.widthPointEdit",
        "Width Point Edit…",
        "",
        "{id, index} open Width Point Edit for width point `index` of path `id` (dialog `widthPoint`: side1, side2 (pt), linked, adjustAdjoining; double-clicking a width point with the Width tool opens it too): OK runs stroke.widthPoint.set, discard: true (the Delete button) stroke.widthPoint.remove",
    ),
    (
        "ui.corners",
        "Corners…",
        "",
        "{id?, corners?: [i…]} open Corners for path `id` (default: the selected one) and its corners (anchor indices of the path with its corners uncut, as object.setLiveShape takes them: a rectangle's 0 top-left, 1 top-right, 2 bottom-right, 3 bottom-left; default: the Direct-Selected corners, else every corner) (dialog `corners`: kind (round|invertedRound|chamfer), radius (pt); double-clicking a corner widget opens it too): OK runs object.setLiveShape",
    ),
    (
        "ui.colorGuideLimit",
        "Limit Color Guide to Library",
        "",
        "{library: swatch library id or name (see swatch.library.list) | \"document\" (the document's swatches) | \"\" or null (no limit)} limit the Color Guide panel's colours to that library: every harmony colour and variation snaps to its nearest colour, as color.harmony {limitTo} answers (ui.inspect: ui.color_guide_limit) → {limitTo, name}",
    ),
    (
        "ui.savePdfDialog",
        "Save PDF Dialog",
        "",
        "{path?, preset?, range?, …document.exportPdf options} open the Save PDF dialog with these options (fields = the options, sections are objects; OK writes path, else asks)",
    ),
    (
        "file.exportAs",
        "Export As…",
        "",
        "{format?, useArtboards?, range?} opens Export As (then the format's options); with {path, …document.export options} writes the file(s) → {path, warnings, files?}",
    ),
    (
        "ui.fileInfoDialog",
        "File Info Dialog",
        "",
        "{} open File Info (dialog `fileInfo`: the file.info fields, plus __keyword, keywords typed but not added yet); OK runs file.info. The File Info… menu item opens it too",
    ),
    (
        "ui.rasterEffectsSettingsDialog",
        "Document Raster Effects Settings Dialog",
        "",
        "{} open Document Raster Effects Settings (dialog `rasterEffectsSettings`: the document.rasterEffectsSettings fields); OK runs it. The menu item opens it too",
    ),
    (
        "ui.pdfPresetsDialog",
        "PDF Presets…",
        "",
        "{selected?: preset name} open Edit → PDF Presets (dialog `pdfPresets`, field `selected`): the built-in presets (read-only) and the saved ones, with the selected one's description and settings. New… and Edit… open the preset editor (ui.pdfPresetDialog); Delete, Import… and Export… run pdf.preset.delete / import / export",
    ),
    (
        "ui.pdfPresetDialog",
        "PDF Preset",
        "",
        "{name?: a saved preset to edit | preset?: the preset a new one starts from (default VectorCraft Default)} open the preset editor (dialog `pdfPreset`: the Save PDF dialog's option fields plus `name` and `description`); OK runs pdf.preset.save and returns to PDF Presets",
    ),
    ("file.openRecent11", "Open Recent File 11", "", "{}"),
    ("file.openRecent12", "Open Recent File 12", "", "{}"),
    ("file.openRecent13", "Open Recent File 13", "", "{}"),
    ("file.openRecent14", "Open Recent File 14", "", "{}"),
    ("file.openRecent15", "Open Recent File 15", "", "{}"),
    ("file.openRecent16", "Open Recent File 16", "", "{}"),
    ("file.openRecent17", "Open Recent File 17", "", "{}"),
    ("file.openRecent18", "Open Recent File 18", "", "{}"),
    ("file.openRecent19", "Open Recent File 19", "", "{}"),
    ("file.openRecent20", "Open Recent File 20", "", "{}"),
    ("file.openRecent21", "Open Recent File 21", "", "{}"),
    ("file.openRecent22", "Open Recent File 22", "", "{}"),
    ("file.openRecent23", "Open Recent File 23", "", "{}"),
    ("file.openRecent24", "Open Recent File 24", "", "{}"),
    ("file.openRecent25", "Open Recent File 25", "", "{}"),
    ("file.openRecent26", "Open Recent File 26", "", "{}"),
    ("file.openRecent27", "Open Recent File 27", "", "{}"),
    ("file.openRecent28", "Open Recent File 28", "", "{}"),
    ("file.openRecent29", "Open Recent File 29", "", "{}"),
    ("file.openRecent30", "Open Recent File 30", "", "{}"),
    ("file.reveal", "Show in Folder", "", "{} show the document's file in the system file manager (desktop) → {path}"),
    (
        "ui.dxfOptionsDialog",
        "DXF Options Dialog",
        "",
        "{path?, useArtboards?, range?, …document.exportDxf options} open DXF Options (dialog `dxfOptions`: fields = these options over the ones used last); OK checks them, remembers them and writes path (else asks). Export As… → DXF opens it too",
    ),
    (
        "links.editOriginal",
        "Edit Original",
        "",
        "{id?} open the file of linked image `id` (default: the selected linked image) in the system's default app for its type (Links panel, Edit › Edit Original); a placed document's file opens here in a new tab, and saving it updates the documents placing it → {path}; edits saved elsewhere show after links.update",
    ),
    (
        "links.reveal",
        "Show in Folder",
        "",
        "{id?} show the file of linked image `id` (default: the selected linked image) in the system's file manager → {path}",
    ),
    (
        "ui.placementOptionsDialog",
        "Placement Options…",
        "",
        "{ids?} open Placement Options for images `ids` (default: the selected ones; dialog `placementOptions`: ids, preserve, align, clip, see links.placementOptions); OK runs links.placementOptions",
    ),
    (
        "ui.packageDialog",
        "Package…",
        "Cmd+Alt+Shift+P",
        "{} open Package for the saved document (dialog `package`: folder, name, copyLinks, linksFolder, relink, copyFonts, report; a document never saved asks to Save As first); OK saves unsaved changes, runs file.package (the web downloads the zip) and offers file.showPackage",
    ),
    ("file.showPackage", "Show Package", "", "{folder} show a package folder (file.package's folder) in the file manager → {folder}"),
    (
        "docInfo.save",
        "Save Document Info…",
        "",
        "{path?, selectionOnly?} write Document Info's text report (document.info {format: \"text\"}) to path, else a picked file (the web downloads it) → {path}",
    ),
    (
        "ui.epsOptionsDialog",
        "EPS Options Dialog",
        "",
        "{path?, useArtboards?, range?, …document.exportEps options} open EPS Options (dialog `epsOptions`: fields = these options over the ones used last); OK checks them, remembers them and writes path (else asks). Export As… → EPS opens it too",
    ),
    (
        "file.saveForWeb",
        "Save for Web (Legacy)…",
        "Cmd+Alt+Shift+S",
        "{} open Save for Web (dialog `saveForWeb`: fields = the webExport.settings settings, `preset` loads a preset, __view: original|optimized|2up|4up, __zoom, __kbps; Save… remembers the settings and writes the files, Done only remembers them); with settings = document.exportForWeb, written to path, else a picked file (the web downloads the files) → {path, files, bytes}",
    ),
    (
        "file.saveForWeb.browser",
        "Preview in Browser",
        "",
        "{…document.exportForWeb settings} write the HTML page and its images to a temporary folder and open the page in the default browser (desktop) → {path}",
    ),
    (
        "file.exportSelection",
        "Export Selection…",
        "",
        "{multiple?: true} collect the selected objects as assets (assets.add: one per object; false: one of them all) and open Export for Screens on its Assets tab with them checked → {assets: [asset id…]}; document.exportSelection exports the selection in one go instead",
    ),
    (
        "css.copy",
        "Copy Selected Style",
        "",
        "{scope?: selection|all (default selection), …css.selection options} copy the CSS of the selection (css.selection; all: css.generate) to the clipboard (CSS Properties panel) → {css, rules: n}",
    ),
    (
        "css.exportFile",
        "Export CSS…",
        "",
        "{path?, scope?: selection|all (default selection), …css.selection options} css.export written to path, else a picked file (the web downloads it), with the pictures of rasterized art next to it (CSS Properties panel: Export Selected CSS…, Export All…) → {path, rules, images: [path…]}",
    ),
    (
        "file.print",
        "Print…",
        "Cmd+P",
        "{} open the Print dialog (dialog `print`: the print.setup settings, preset (setting it loads that print preset; Save Preset… saves them as one), printer, toFile; Print keeps the settings with the document and prints, Done (discard: true) only keeps them); with params {settings? (over the document's), printer? (print.printers name; \"\" the default), toFile?, path?, format?: pdf|postscript, level?, flattenerPreset? (as engine file.print)} print without it: the print-ready PDF (engine file.print) goes to the printer → {pages, printer, printed: true, warnings}; with toFile, a path or no printing here (no print service) it is saved as a PDF at path (else a picked file, the web downloads it) → {path, pages, printed: false, warnings}; a PostScript file (format postscript, or a .ps path) is always saved that way",
    ),
    (
        "print.printers",
        "Printers",
        "",
        "{} → {printers: [{name, default}] (the system's printers), service (printing is available; else Print saves a PDF), setup (print.printerSetup can open the printer settings)}",
    ),
    ("print.printerSetup", "Printer Setup…", "", "{printer?} open the system's settings of the printer (the Print dialog's Setup…; desktop)"),
    (
        "ui.printPresetsDialog",
        "Print Presets…",
        "",
        "{selected?: preset name} open Edit → Print Presets (dialog `printPresets`, field `selected`): [Default] (protected) and the saved presets, with how the selected one differs from [Default]. New… and Edit… open the preset editor (ui.printPresetDialog); Delete, Import… and Export… run print.presets.delete / import / export",
    ),
    (
        "ui.printPresetDialog",
        "Print Preset",
        "",
        "{name?: a saved preset to edit | preset?: the preset a new one starts from (default [Default])} open the preset editor (dialog `printPreset`: the Print dialog's settings fields plus `name`); OK runs print.presets.save and returns to Print Presets",
    ),
    (
        "plugin.dialog",
        "Plug-in…",
        "",
        "{id: an object filter plug-in (plugin.list), params?: {…}} Object › Plug-ins: run the filter at once when it takes no parameters or params are given (plugin.run), else open its dialog (dialog `plugin`: a field per parameter, `__plugin` the id, live preview; OK runs plugin.run as one undo step)",
    ),
    (
        "ui.installPlugin",
        "Install Plug-in…",
        "",
        "{path?: a .wasm file} install a WebAssembly plug-in (plugin.install); without a path pick one (the web opens the browser's file picker; File › Open of a .wasm installs it too)",
    ),
    (
        "ui.perspectiveGridDialog",
        "Define Perspective Grid…",
        "",
        "{} open View › Perspective Grid › Define Grid (dialog `perspectiveGrid`, prefilled from the grid: the fields of perspective.grid.define); OK runs perspective.grid.define",
    ),
    (
        "ui.perspectivePresetsDialog",
        "Perspective Grid Presets…",
        "",
        "{selected?} open Edit › Perspective Grid Presets (dialog `perspectiveGridPresets`, field `selected`): New… and Edit… open the preset editor (dialog `perspectiveGrid` with `__mode` edit), whose OK runs perspective.presets.save and comes back; Delete, Import… and Export… run perspective.presets.*",
    ),
    (
        "ui.savePerspectivePreset",
        "Save Grid as Preset…",
        "",
        "{} open View › Perspective Grid › Save Grid as Preset (dialog `perspectiveGrid` with `__mode` save, `name` a new preset name): OK runs perspective.presets.save with the fields",
    ),
    (
        "ui.perspectiveUserPreset1.1",
        "One Point Perspective Preset 1",
        "",
        "{} apply the 1. saved 1-point perspective grid preset (perspective.grid.preset)",
    ),
    (
        "ui.perspectiveUserPreset1.2",
        "One Point Perspective Preset 2",
        "",
        "{} apply the 2. saved 1-point perspective grid preset (perspective.grid.preset)",
    ),
    (
        "ui.perspectiveUserPreset1.3",
        "One Point Perspective Preset 3",
        "",
        "{} apply the 3. saved 1-point perspective grid preset (perspective.grid.preset)",
    ),
    (
        "ui.perspectiveUserPreset1.4",
        "One Point Perspective Preset 4",
        "",
        "{} apply the 4. saved 1-point perspective grid preset (perspective.grid.preset)",
    ),
    (
        "ui.perspectiveUserPreset1.5",
        "One Point Perspective Preset 5",
        "",
        "{} apply the 5. saved 1-point perspective grid preset (perspective.grid.preset)",
    ),
    (
        "ui.perspectiveUserPreset2.1",
        "Two Point Perspective Preset 1",
        "",
        "{} apply the 1. saved 2-point perspective grid preset (perspective.grid.preset)",
    ),
    (
        "ui.perspectiveUserPreset2.2",
        "Two Point Perspective Preset 2",
        "",
        "{} apply the 2. saved 2-point perspective grid preset (perspective.grid.preset)",
    ),
    (
        "ui.perspectiveUserPreset2.3",
        "Two Point Perspective Preset 3",
        "",
        "{} apply the 3. saved 2-point perspective grid preset (perspective.grid.preset)",
    ),
    (
        "ui.perspectiveUserPreset2.4",
        "Two Point Perspective Preset 4",
        "",
        "{} apply the 4. saved 2-point perspective grid preset (perspective.grid.preset)",
    ),
    (
        "ui.perspectiveUserPreset2.5",
        "Two Point Perspective Preset 5",
        "",
        "{} apply the 5. saved 2-point perspective grid preset (perspective.grid.preset)",
    ),
    (
        "ui.perspectiveUserPreset3.1",
        "Three Point Perspective Preset 1",
        "",
        "{} apply the 1. saved 3-point perspective grid preset (perspective.grid.preset)",
    ),
    (
        "ui.perspectiveUserPreset3.2",
        "Three Point Perspective Preset 2",
        "",
        "{} apply the 2. saved 3-point perspective grid preset (perspective.grid.preset)",
    ),
    (
        "ui.perspectiveUserPreset3.3",
        "Three Point Perspective Preset 3",
        "",
        "{} apply the 3. saved 3-point perspective grid preset (perspective.grid.preset)",
    ),
    (
        "ui.perspectiveUserPreset3.4",
        "Three Point Perspective Preset 4",
        "",
        "{} apply the 4. saved 3-point perspective grid preset (perspective.grid.preset)",
    ),
    (
        "ui.perspectiveUserPreset3.5",
        "Three Point Perspective Preset 5",
        "",
        "{} apply the 5. saved 3-point perspective grid preset (perspective.grid.preset)",
    ),
    (
        "ui.blendOptions",
        "Blend Options…",
        "",
        "{} open Blend Options (Object › Blend › Blend Options…, the Blend tool's double-click, Alt-click and toolbar button; dialog `blendOptions`: spacing smooth|steps|distance, steps, distance (pt), orientation page|path, preview) on the selected blend's options, previewed live: OK runs object.blend.options as one undo step; with no blend selected it sets what new blends start with",
    ),
    (
        "ui.perspectivePlane",
        "Perspective Plane Options…",
        "",
        "{plane?: left|right|ground (default: the active plane)} open the plane's options, as double-clicking its plane widget does (dialog `perspectivePlane`: location (pt along the plane's normal), objects: none|move|copy): OK runs perspective.plane.move",
    ),
];

/// Canonical panel id for `window.panel`: a dock tab's or an icon panel's id or display label,
/// matched case-insensitively (`"Layers"`, `"swatches"`, `"Color Guide"`).
pub(crate) fn normalize_panel(input: &str) -> Option<&'static str> {
    let name = input.trim();
    crate::state::all_panels().find(|(id, label)| name.eq_ignore_ascii_case(id) || name.eq_ignore_ascii_case(label)).map(|(id, _)| id)
}

/// An optional `{key?: bool}` param: none when it is omitted or null (the commands then toggle).
pub(crate) fn opt_bool(p: &Value, key: &str) -> Result<Option<bool>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(format!("`{key}` must be true or false")),
    }
}

/// Handle a UI command. `None` = not a UI command (the engine handles it).
pub fn run_ui_command(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    if id == "app.language" {
        let lang = p.get("lang").and_then(Value::as_str).unwrap_or("");
        let value = if lang.eq_ignore_ascii_case("auto") {
            "auto".to_string()
        } else if let Some(l) = crate::i18n::Lang::from_code(lang) {
            l.code().to_string()
        } else {
            let codes: Vec<&str> = std::iter::once("auto").chain(crate::i18n::Lang::all().map(|l| l.code())).collect();
            return Some(Err(format!("lang must be one of {}", codes.join(", "))));
        };
        return Some(app.run("prefs.set", json!({"key": "interfaceLanguage", "value": value})).map(|_| json!(value)));
    }
    if let Some(r) = crate::panels::character::intercept_text_command(app, id) {
        return Some(r);
    }
    if let Some(r) = crate::shortcut_editor::run_command(app, id, p)
        .or_else(|| crate::workspaces::run_command(app, id, p))
        .or_else(|| crate::floating::command(app, id, p))
    {
        return Some(r);
    }
    let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
    let flag = |b: &mut bool| {
        *b = !*b;
        Ok(json!(*b))
    };
    let r = match id {
        "file.newDialog" => {
            crate::dialogs::open_new_document(app);
            Ok(Value::Null)
        }
        "app.home" => {
            app.ui.home = Some(home_key(app));
            Ok(Value::Null)
        }
        "file.open" => match s("path") {
            Some(path) => io::open_path(app, &path),
            None => io::open_dialog(app).map(|_| Value::Null),
        },
        // Saves write through the app (save panel, download); with no path known the panel and the
        // format's options dialog ask first.
        "file.save" => io::save(app, SaveMode::Save, p, true),
        "file.saveAs" => io::save(app, SaveMode::SaveAs, p, true),
        "file.saveCopy" => io::save(app, SaveMode::Copy, p, true),
        "file.saveAsTemplate" => io::save(app, SaveMode::Template, p, true),
        "document.exportSelection" => {
            io::save_command_output(app, id, "png", if p.is_object() { p.clone() } else { json!({}) }).map(|p| json!({"path": p}))
        }
        // Bytes sent by an agent go straight to the engine; a path or nothing opens it here.
        "file.newFromTemplate" if p.get("dataBase64").is_none() => io::new_from_template(app, s("path")),
        "file.revert" if p.get("confirmed").and_then(Value::as_bool) != Some(true) => io::ask_revert(app),
        "file.reveal" => io::reveal(app),
        id if id.starts_with("file.openRecent") => match recent_slot(app, id).cloned() {
            Some(path) => io::open_path(app, &path),
            None => Err("no such recent file".into()),
        },
        "type.findFont" => {
            crate::find_font::open(app);
            Ok(Value::Null)
        }
        "ui.missingFontsDialog" => crate::dialogs::missing_fonts::open_command(app, s("folder")),
        "ui.findFontsInFolder" => crate::dialogs::missing_fonts::find_in_folder_command(app, s("folder")),
        "file.clearRecent" => {
            app.ui.recent_files.clear();
            Ok(Value::Null)
        }
        "file.recentFiles" => Ok(json!(io::recent_files(app))),
        "file.place" => crate::place::run(app, p),
        "file.place.queue" => crate::place::queue(app, p),
        "file.export.svg" if p.as_object().is_none_or(|o| o.is_empty()) => {
            crate::dialogs::svg_options::open(app, crate::dialogs::svg_options::Mode::Export, None);
            Ok(Value::Null)
        }
        "file.export.svg" => io::export(app, Some("svg"), s("path"), p),
        "file.exportForScreens" if p.as_object().is_none_or(|o| o.is_empty()) => {
            crate::dialogs::open_export_for_screens(app);
            Ok(Value::Null)
        }
        "file.exportForScreens" => io::export_for_screens(app, p.clone()),
        "file.export.png" if s("path").is_none() => io::target_path(app, None, "png").and_then(|path| {
            let f = vectorcraft_engine::cmd::fileio::format("png").ok_or("no PNG encoder")?;
            let mut params = if p.is_object() { p.clone() } else { json!({}) };
            params["format"] = json!("png");
            params["path"] = json!(path);
            crate::dialogs::open_raster_options(app, f, params);
            Ok(Value::Null)
        }),
        "file.export.png" => io::export(app, Some("png"), s("path"), p),
        "file.documentSetup" => crate::dialogs::open_document_setup(app),
        "edit.preferences" => {
            crate::prefs_dialog::open(app, s("category").as_deref());
            Ok(Value::Null)
        }
        // Edit → Color Settings… / Assign Profile… (no params): show the colour-management panel.
        "edit.colorSettings" | "edit.assignProfile" if p.as_object().is_none_or(|o| o.is_empty()) => {
            app.ui.open_panel = Some("separations".into());
            app.ui.dock = true;
            app.session.execute(id, p).map_err(|e| e.to_string())
        }
        "view.outline" => flag(&mut app.ui.view.outline),
        "view.pixelPreview" => flag(&mut app.ui.view.pixel_preview),
        "view.trimView" => flag(&mut app.ui.view.trim_view),
        "view.cornerWidget" => flag(&mut app.ui.view.corner_widgets),
        // New View: the engine stores the current zoom, centre and rotation under the given name.
        "view.saved.new" if p.get("zoom").is_none() => {
            let Some(v) = app.view().copied() else { return Some(Err("no document".into())) };
            let mut params = if p.is_object() { p.clone() } else { json!({}) };
            params["center"] = json!([v.center.x, v.center.y]);
            params["zoom"] = json!(v.zoom);
            params["rotation"] = json!(v.rotation);
            app.session.execute(id, &params).map_err(|e| e.to_string())
        }
        id if id.starts_with("view.goto") => {
            let n: usize = id["view.goto".len()..].parse().unwrap_or(0);
            let saved = app.session.active().and_then(|d| n.checked_sub(1).and_then(|i| d.doc.views.get(i)).cloned());
            match (saved, app.view_mut()) {
                (Some(sv), Some(v)) => {
                    v.center = sv.center;
                    v.zoom = sv.zoom;
                    v.rotation = sv.rotation;
                    Ok(json!({ "name": sv.name }))
                }
                _ => Err("no such view".into()),
            }
        }
        id if saved_selection_slot(id).is_some() => match saved_selection_name(app, id) {
            Some(name) => app.run("select.recall", json!({ "name": name })),
            None => Err("no such saved selection".into()),
        },
        "view.snapToPixel" => flag(&mut app.ui.view.snap_to_pixel),
        "view.textThreads" => flag(&mut app.ui.view.text_threads),
        "type.hiddenCharacters" => flag(&mut app.ui.view.hidden_chars),
        "type.bold" => crate::panels::character::toggle_face(app, Face::Bold),
        "type.italic" => crate::panels::character::toggle_face(app, Face::Italic),
        "view.gradientAnnotator" => flag(&mut app.ui.view.gradient_annotator),
        "effect.last" => match app.last_effect.clone() {
            Some((e, params)) => {
                let label = vectorcraft_effects::effect_info(&e).map(|i| i.label.trim_end_matches('…').to_string()).unwrap_or_else(|| e.clone());
                let mut fields = params.as_object().cloned().unwrap_or_default();
                fields.insert("__effect".into(), json!(e));
                fields.insert("__label".into(), json!(label));
                fields.insert("preview".into(), json!(true));
                app.ui.dialog = Some(crate::state::Dialog { kind: "effect".into(), fields });
                Ok(Value::Null)
            }
            None => Err("no effect applied yet".into()),
        },
        id if id.starts_with("type.recentFont") => {
            let n: usize = id["type.recentFont".len()..].parse().unwrap_or(0);
            match n.checked_sub(1).and_then(|i| app.recent_fonts().get(i)).cloned() {
                Some(font) => app.run("text.setStyle", json!({ "font": font })),
                None => Err("no such recent font".into()),
            }
        }
        "view.edges" => flag(&mut app.ui.view.edges),
        "view.artboards" => flag(&mut app.ui.view.artboards),
        "view.rulers" => flag(&mut app.ui.view.rulers),
        "view.boundingBox" => flag(&mut app.ui.view.bounding_box),
        "view.guides" => {
            let r = flag(&mut app.ui.view.guides);
            // Hidden guides can't stay selected (selected, they are all that is).
            if !app.ui.view.guides && app.session.active().is_some_and(|d| !d.selection.guides.is_empty()) {
                // Deselecting a document that is open can't fail.
                let _ = app.run("select.none", json!({}));
            }
            r
        }
        "view.smartGuides" => flag(&mut app.ui.view.smart_guides),
        "view.grid" => flag(&mut app.ui.view.grid),
        "view.snapToGrid" => flag(&mut app.ui.view.snap_to_grid),
        "view.snapToPoint" => flag(&mut app.ui.view.snap_to_point),
        "view.zoomIn" | "view.zoomOut" => {
            let up = id == "view.zoomIn";
            // Selection & Anchor Display › Zoom to Selection: the selection comes to the middle.
            let focus = if app.session.prefs.zoom_to_selection { app.selection_box().map(|b| b.center()) } else { None };
            match app.view_mut() {
                Some(v) => {
                    v.zoom = next_zoom(v.zoom, up);
                    if let Some(c) = focus {
                        v.center = c;
                    }
                    Ok(json!({"zoom": v.zoom * 100.0}))
                }
                None => Err("no document".into()),
            }
        }
        "view.setZoom" => {
            let z = p.get("zoom").and_then(Value::as_f64).unwrap_or(100.0) / 100.0;
            let center =
                p.get("center").and_then(Value::as_array).and_then(|a| Some(vectorcraft_geom::Point::new(a.first()?.as_f64()?, a.get(1)?.as_f64()?)));
            match app.view_mut() {
                Some(v) => {
                    v.zoom = z.clamp(0.0313, 640.0);
                    v.fitted = true;
                    if let Some(c) = center {
                        v.center = c;
                    }
                    Ok(Value::Null)
                }
                None => Err("no document".into()),
            }
        }
        "view.fitArtboard" | "view.fitAll" | "view.actualSize" => {
            crate::canvas::fit(app, id);
            Ok(Value::Null)
        }
        "view.goToArtboard" => go_to_artboard(app, p),
        "view.presentation" => {
            app.ui.screen_mode = if app.ui.screen_mode == 3 { 0 } else { 3 };
            Ok(json!(app.ui.screen_mode))
        }
        "view.screenMode" => {
            app.ui.screen_mode = match p.get("mode").and_then(Value::as_u64) {
                Some(m) => m.min(3) as u8,
                // F cycles the three screen modes; from Presentation Mode (not in the cycle) it
                // goes back to Normal.
                None if app.ui.screen_mode >= 2 => 0,
                None => app.ui.screen_mode + 1,
            };
            Ok(json!(app.ui.screen_mode))
        }
        "view.rotateReset" => {
            if let Some(v) = app.view_mut() {
                v.rotation = 0.0;
            }
            Ok(Value::Null)
        }
        "window.control" => flag(&mut app.ui.control_bar),
        "window.toolbar" => flag(&mut app.ui.toolbar),
        "window.toolbarColumns" => opt_bool(p, "double").map(|on| {
            let on = on.unwrap_or(!app.ui.toolbar_double);
            app.ui.toolbar_double = on;
            json!(on)
        }),
        "window.toolbarAdvanced" => flag(&mut app.ui.toolbar_advanced),
        "window.floatTools" => {
            let floating = match opt_bool(p, "floating") {
                Ok(on) => on,
                Err(e) => return Some(Err(e)),
            };
            let Some(tool) = s("tool") else { return Some(Err("tool (a tool id) is required".into())) };
            crate::toolbar::float_group(app, &tool, floating).map(Value::Bool)
        }
        "window.taskBar" => flag(&mut app.ui.task_bar),
        "window.taskBar.pin" => {
            let place = &mut app.ui.task_bar_place;
            opt_bool(p, "pinned").map(|pinned| {
                let pinned = pinned.unwrap_or(!place.pinned);
                // Pinning holds the bar where it shows; unpinning lets it follow the selection from
                // there (the bar turns its pinned spot into an offset when it is next drawn).
                if pinned && !place.pinned {
                    place.pin_at = place.shown_at;
                }
                place.pinned = pinned;
                json!(pinned)
            })
        }
        "window.taskBar.reset" => {
            app.ui.task_bar_place = Default::default();
            Ok(Value::Null)
        }
        "window.dock" => {
            let on = !(app.ui.dock && app.ui.toolbar);
            app.ui.dock = on;
            app.ui.toolbar = on;
            app.ui.control_bar = on;
            Ok(json!(on))
        }
        "window.collapseDock" => opt_bool(p, "collapsed").map(|collapsed| {
            let collapsed = collapsed.unwrap_or(!app.ui.dock_collapsed);
            crate::dock::set_collapsed(app, collapsed);
            json!(collapsed)
        }),
        "window.panel" => {
            let raw = s("panel").unwrap_or_default();
            // Canonical id (case-insensitive; display labels work too), then the
            // dock tab it names, if any.
            let canonical = normalize_panel(&raw);
            // A floating panel shows its tab in its group.
            if let Some(p) = canonical
                && let Some(g) = crate::floating::group_of(&app.ui, p).and_then(|i| app.ui.floating_panels.get_mut(i))
            {
                g.active = g.panels.iter().position(|q| q == p).unwrap_or(g.active);
                let out = json!({ "floating": g.panels });
                app.ui.dock = true;
                return Some(Ok(out));
            }
            let tab = canonical.and_then(DockTab::from_id);
            match (canonical, tab) {
                // A collapsed dock pops the panel out of its icon, like the icon panels.
                (Some(p), Some(tab)) if app.ui.dock_collapsed => {
                    app.ui.dock_tab = tab;
                    app.ui.open_panel = if app.ui.open_panel.as_deref() == Some(p) { None } else { Some(p.to_string()) };
                    app.ui.dock = true;
                    Ok(json!({"open": app.ui.open_panel}))
                }
                (Some(_), Some(tab)) => {
                    app.ui.dock_tab = tab;
                    Ok(Value::Null)
                }
                (Some(p), None) => {
                    app.ui.open_panel = if app.ui.open_panel.as_deref() == Some(p) { None } else { Some(p.to_string()) };
                    app.ui.dock = true;
                    Ok(json!({"open": app.ui.open_panel}))
                }
                (None, _) => Err(format!("unknown panel `{raw}`")),
            }
        }
        "window.brightness" => match s("brightness").as_deref().and_then(Brightness::parse) {
            Some(b) => {
                app.ui.brightness = b;
                app.session.prefs.ui_brightness = b.id().into();
                app.canvas.key = None;
                Ok(json!(b.id()))
            }
            None => Err("brightness must be dark|mediumDark|mediumLight|light".into()),
        },
        "window.newWindow" => Err("multiple windows land with M11.5".into()),
        "tool.select" => match s("tool") {
            Some(t) if vectorcraft_tools::tool_info(&t).is_some() => {
                app.select_tool(&t);
                Ok(json!({"tool": app.session.tool_id()}))
            }
            Some(t) => Err(format!("unknown tool `{t}`")),
            None => Err("missing `tool`".into()),
        },
        "tool.setOption" => app.session.set_tool_option_cmd(p),
        "effect.dialog" => crate::dialogs::open_effect_dialog(app, p),
        "ui.recolorDialog" => crate::dialogs::recolor::open(app, p),
        "ui.paramDialog" => {
            let cmd = s("command").unwrap_or_default();
            let mut fields = p.get("params").and_then(Value::as_object).cloned().unwrap_or_default();
            fields.insert("__command".into(), json!(cmd));
            fields.insert("__label".into(), json!(s("label").unwrap_or(cmd.clone())));
            app.ui.dialog = Some(crate::state::Dialog { kind: "command".into(), fields });
            Ok(Value::Null)
        }
        "effect.applyLast" => match app.last_effect.clone() {
            Some((e, params)) => app.run("effect.apply", json!({"effect": e, "params": params})),
            None => Err("no effect applied yet".into()),
        },
        "file.export.pdf" if p.as_object().is_none_or(|o| o.is_empty()) => crate::dialogs::open_save_pdf(app, &json!({})),
        "file.export.pdf" => io::export_pdf(app, p.clone()),
        "help.about" => {
            app.ui.about = true;
            Ok(Value::Null)
        }
        "help.commandPalette" => {
            app.ui.palette_open = !app.ui.palette_open;
            app.ui.palette_query.clear();
            Ok(Value::Null)
        }
        // Closing asks Save / Don't Save / Cancel for modified documents first (`unsaved`).
        "file.close" => {
            let i = p.get("index").and_then(Value::as_u64).map(|i| i as usize).or(app.session.active_index())?;
            crate::unsaved::close(app, i)
        }
        "file.closeAll" => crate::unsaved::close_all(app, "closeAll"),
        "app.quit" => crate::unsaved::close_all(app, "quit"),
        "ui.swatchOptions" => match s("name") {
            Some(name) => crate::dialogs::swatch_options::open(app, &name),
            None => Err("missing `name`".into()),
        },
        "ui.newSwatch" => crate::dialogs::new_swatch::open(app, p.get("spot").and_then(Value::as_bool).unwrap_or(false), s("group").as_deref()),
        "ui.newColorGroup" => {
            let names = p.get("swatches").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect());
            crate::dialogs::new_color_group::open(app, names.unwrap_or_default())
        }
        "ui.colorPicker" => crate::dialogs::open_color_picker(app, p),
        "ui.graphicStyleOptions" => crate::dialogs::graphic_style_options::open(app, s("name").as_deref()),
        "tool.options" => crate::toolbar::open_options(app, &s("tool").unwrap_or_default()),
        "ui.colorGuideOptions" => {
            crate::dialogs::color_guide_options::open(app);
            Ok(Value::Null)
        }
        "ui.colorBalanceDialog" => {
            crate::dialogs::color_balance::open(app);
            Ok(Value::Null)
        }
        "ui.saturateDialog" => {
            crate::dialogs::saturate::open(app);
            Ok(Value::Null)
        }
        "window.swatchLibrary" => crate::panels::swatches::open_library(app, p),
        "window.swatchLibrary.other" => crate::panels::swatches::other_library(app, s("path")),
        "ui.saveSwatchLibrary" => {
            let names = p.get("names").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect());
            crate::dialogs::save_swatch_library::open(app, names.unwrap_or_default())
        }
        id if id.starts_with(crate::panels::swatches::USER_SLOT) => match crate::panels::swatches::user_library(app, id) {
            Some(l) => crate::panels::swatches::open_library(app, &json!({ "library": l.id })),
            None => Err("no such user library".into()),
        },
        "ui.mergeGraphicStyles" => {
            let names = p.get("names").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect());
            crate::dialogs::graphic_style_options::open_merge(app, names.unwrap_or_default())
        }
        "ui.tileEdgeColor" => crate::dialogs::tile_edge_color::open(app),
        "ui.layerOptions" => crate::dialogs::layer_options::open(app, p),
        "ui.newLayer" => crate::dialogs::layer_options::open_new(app, p),
        "ui.layersPanelOptions" => crate::dialogs::layers_panel_options::open(app),
        "ui.layersExpand" => crate::panels::layers::expand(app, p),
        "ui.flattenTransparencyDialog" => {
            crate::dialogs::flatten::open(app);
            Ok(Value::Null)
        }
        "ui.flattenerPresetsDialog" => {
            crate::dialogs::flattener_presets::open(app, s("selected").as_deref());
            Ok(Value::Null)
        }
        "ui.flattenerPreview" => crate::panels::flattener_preview::command(app, p),
        "window.graphicStyleLibrary" => crate::panels::graphic_styles::open_library(app, p),
        "window.graphicStyleLibrary.other" => crate::panels::graphic_styles::other_library(app, s("path")),
        "ui.saveGraphicStyleLibrary" => {
            let names = p.get("names").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect());
            crate::dialogs::save_style_library::open(app, names.unwrap_or_default())
        }
        id if id.starts_with(crate::panels::graphic_styles::USER_SLOT) => match crate::panels::graphic_styles::user_library(app, id) {
            Some(l) => crate::panels::graphic_styles::open_library(app, &json!({ "library": l.id })),
            None => Err("no such user library".into()),
        },
        "ui.expandDialog" => crate::dialogs::expand::open(app),
        "attributes.openUrl" => crate::panels::attributes::open_url(app, p),
        "ui.spotColors" => crate::dialogs::spot_colors::open(app),
        "ui.menuDialog" => match s("command").as_deref() {
            Some(c) if crate::dialogs::envelope::opens(c) => crate::dialogs::envelope::open(app, c),
            c => match c.and_then(menu_dialog) {
                Some((kind, fields)) => {
                    app.ui.dialog = Some(crate::state::Dialog::new(kind, fields));
                    Ok(Value::Null)
                }
                None => Err("`command` must be a command whose menu item opens a dialog (see ui.menuDialog)".into()),
            },
        },
        "ui.widthPointEdit" => crate::dialogs::width_point::open(app, p),
        "ui.corners" => crate::dialogs::corners::open(app, p),
        "ui.colorGuideLimit" => crate::panels::color_guide::set_limit(app, p),
        "ui.savePdfDialog" => crate::dialogs::open_save_pdf(app, p),
        "file.exportAs" if s("path").is_none() => {
            crate::dialogs::open_export_as(app, s("format").as_deref(), p);
            Ok(Value::Null)
        }
        "file.exportAs" => io::export(app, s("format").as_deref(), s("path"), p),
        "ui.fileInfoDialog" => crate::dialogs::file_info::open(app),
        "ui.rasterEffectsSettingsDialog" => crate::dialogs::raster_effects::open(app),
        "ui.pdfPresetsDialog" => {
            crate::dialogs::pdf_presets::open(app, s("selected").as_deref());
            Ok(Value::Null)
        }
        "ui.pdfPresetDialog" => crate::dialogs::open_pdf_preset(app, p),
        // Save for Office Documents…: its dialog; with options, pick the file (or write `path`).
        "document.exportForOffice" if p.as_object().is_none_or(|o| o.is_empty()) => crate::dialogs::office_export::open(app),
        "document.exportForOffice" => io::save_command_output(app, id, "png", p.clone()).map(|path| json!({ "path": path })),
        "ui.dxfOptionsDialog" if app.session.active().is_none() => Err("no document".into()),
        "ui.dxfOptionsDialog" => {
            crate::dialogs::dxf_options::open(app, p);
            Ok(Value::Null)
        }
        "links.editOriginal" => crate::panels::links::open_file(app, p, false),
        "links.reveal" => crate::panels::links::open_file(app, p, true),
        "ui.placementOptionsDialog" => crate::dialogs::placement_options::open(app, p),
        "ui.packageDialog" => crate::dialogs::package::open(app),
        "file.showPackage" => crate::dialogs::package::show_package(app, p),
        "docInfo.save" => crate::panels::doc_info::save_report(app, p),
        // Slice Options… and Divide Slices…: their dialogs, on the selected slices.
        "object.slice.options" if p.as_object().is_none_or(|o| o.is_empty()) => crate::dialogs::slices::open_options(app),
        "object.slice.divide" if p.as_object().is_none_or(|o| o.is_empty()) => crate::dialogs::slices::open_divide(app),
        "ui.epsOptionsDialog" if app.session.active().is_none() => Err("no document".into()),
        "ui.epsOptionsDialog" => {
            crate::dialogs::eps_options::open(app, p);
            Ok(Value::Null)
        }
        // Save for Web: its dialog; with settings, write the files.
        "file.saveForWeb" if p.as_object().is_none_or(|o| o.is_empty()) => crate::dialogs::save_for_web::open(app),
        "file.saveForWeb" => crate::dialogs::save_for_web::save(app, p.clone()),
        "file.saveForWeb.browser" => crate::dialogs::save_for_web::browser_preview(app, p),
        // File → Export Selection…: the selection becomes assets, shown checked on Export for
        // Screens' Assets tab.
        "file.exportSelection" => app.run("assets.add", json!({ "multiple": p.get("multiple").and_then(Value::as_bool).unwrap_or(true) })).map(|r| {
            let ids: Vec<u64> = r["assets"].as_array().into_iter().flatten().filter_map(Value::as_u64).collect();
            crate::dialogs::open_export_for_screens_assets(app, Some(&ids));
            json!({ "assets": ids })
        }),
        "css.copy" => crate::panels::css_properties::copy(app, p),
        "css.exportFile" => crate::panels::css_properties::export_file(app, p),
        // File → Print…: its dialog; with params, print (or save the job as a PDF) without it.
        "file.print" if p.as_object().is_none_or(|o| o.is_empty()) => crate::dialogs::print::open(app),
        "file.print" => crate::print::run(app, p),
        "print.printers" => Ok(crate::print::printers(app)),
        "print.printerSetup" => {
            let printer = s("printer").filter(|n| !n.is_empty());
            match app.services.print.as_mut() {
                Some(service) => service.setup(printer.as_deref()).map(|_| Value::Null),
                None => Err("printing isn't available here".into()),
            }
        }
        "ui.printPresetsDialog" => {
            crate::dialogs::print_presets::open(app, s("selected").as_deref());
            Ok(Value::Null)
        }
        "ui.printPresetDialog" => crate::dialogs::print::open_preset(app, p),
        "plugin.dialog" => crate::dialogs::plugin::open(app, p),
        "ui.installPlugin" => io::install_plugin(app, s("path")),
        "ui.perspectiveGridDialog" => crate::dialogs::perspective_grid::open(app),
        "ui.perspectivePresetsDialog" => {
            crate::dialogs::perspective_presets::open(app, s("selected").as_deref());
            Ok(Value::Null)
        }
        "ui.savePerspectivePreset" => crate::dialogs::perspective_grid::open_save(app),
        id if id.starts_with(crate::dialogs::perspective_presets::SLOT) => match crate::dialogs::perspective_presets::slot_preset(app, id) {
            Some(name) => app.run("perspective.grid.preset", json!({ "name": name })),
            None => Err("no such perspective grid preset".into()),
        },
        "ui.blendOptions" => crate::dialogs::blend_options::open(app),
        "ui.perspectivePlane" => crate::dialogs::perspective_plane::open(app, p),
        _ => return None,
    };
    Some(r)
}

/// `view.goToArtboard`: make artboard `index` (a number, or first / previous / next / last from
/// the current one) the navigator's, and fit it in the window.
fn go_to_artboard(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let n = app.session.active().map(|d| d.doc.artboards.len()).ok_or("no document open")?;
    let last = n.checked_sub(1).ok_or("the document has no artboards")?;
    let current = app.view().map_or(0, |v| v.artboard).min(last);
    let to = match p.get("index") {
        Some(Value::String(s)) => match s.as_str() {
            "first" => 0,
            "previous" => current.saturating_sub(1),
            "next" => current + 1,
            "last" => last,
            other => return Err(format!("index must be a number or first, previous, next or last, not `{other}`")),
        },
        Some(v) => v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or("index must be a number or first, previous, next or last")?,
        None => return Err("missing index".into()),
    }
    .min(last);
    if let Some(v) = app.view_mut() {
        v.artboard = to;
    }
    crate::canvas::fit(app, "view.fitArtboard");
    Ok(json!({ "index": to }))
}

/// Checked state for toggle items.
pub fn checked(app: &VectorcraftApp, id: &str, p: &Value) -> Option<bool> {
    if id == "app.language" {
        let lang = p.get("lang").and_then(Value::as_str).unwrap_or("");
        let pref = app.session.prefs.interface_language.as_str();
        return Some(if lang.eq_ignore_ascii_case("auto") {
            crate::i18n::Lang::from_code(pref).is_none()
        } else {
            crate::i18n::Lang::from_code(pref).is_some_and(|l| l.code().eq_ignore_ascii_case(lang))
        });
    }
    let v = &app.ui.view;
    // An item that can be on or off is `Some` whatever the document and selection (off with none):
    // the macOS menu bar can't change an item's kind without rebuilding.
    let selected_text = |onpath: bool| {
        let st = app.session.active()?;
        st.selection.objects.iter().find_map(|id| match st.doc.node(*id).map(|n| &n.kind) {
            Some(vectorcraft_engine::doc::NodeKind::Text(t)) if !onpath || matches!(t.kind, vectorcraft_engine::doc::TextKind::OnPath { .. }) => {
                Some(t)
            }
            _ => None,
        })
    };
    Some(match id {
        "type.orientation.vertical" | "type.orientation.horizontal" => {
            selected_text(false).is_some_and(|t| t.vertical == (id == "type.orientation.vertical"))
        }
        "view.outline" => v.outline,
        "view.pixelPreview" => v.pixel_preview,
        "view.trimView" => v.trim_view,
        "view.snapToPixel" => v.snap_to_pixel,
        // Type on a Path effect of the selected path type.
        "type.pathOptions" if p.get("effect").is_some() => {
            selected_text(true).is_some_and(|t| p.get("effect").and_then(Value::as_str) == Some(t.path_effect.id()))
        }
        "view.smartGuides" => v.smart_guides,
        "view.grid" => v.grid,
        "view.snapToGrid" => v.snap_to_grid,
        "view.snapToPoint" => v.snap_to_point,
        "view.rulers" => v.rulers,
        "window.control" => app.ui.control_bar,
        "window.toolbar" => app.ui.toolbar,
        "window.toolbarAdvanced" => app.ui.toolbar_advanced,
        "window.toolbarColumns" => app.ui.toolbar_double,
        "window.floatTools" => {
            let tool = p.get("tool").and_then(Value::as_str).unwrap_or("");
            app.ui.floating_flyouts.iter().any(|f| f.tools.iter().any(|id| id == tool))
        }
        "window.taskBar" => app.ui.task_bar,
        "window.taskBar.pin" => app.ui.task_bar_place.pinned,
        "window.panel" => {
            let panel = p.get("panel").and_then(Value::as_str).unwrap_or("");
            let canonical = normalize_panel(panel);
            match canonical.and_then(DockTab::from_id) {
                _ if canonical.is_some_and(|id| crate::floating::group_of(&app.ui, id).is_some()) => true,
                Some(tab) if !app.ui.dock_collapsed => crate::floating::shown_tab(&app.ui) == Some(tab),
                _ => canonical.is_some_and(|id| app.ui.open_panel.as_deref() == Some(id)),
            }
        }
        "window.collapseDock" => app.ui.dock_collapsed,
        "window.workspace" => p.get("name").and_then(Value::as_str) == Some(app.ui.workspace.as_str()),
        "window.brightness" => p.get("brightness").and_then(Value::as_str).and_then(Brightness::parse) == Some(app.ui.brightness),
        "view.slices.lock" => app.session.slices_locked(),
        "object.slice.clipToArtboard" => app.session.active().is_some_and(|d| d.doc.slices_clip_to_artboard),
        "view.proofColors" => vectorcraft_render::proof::view().proof_colors,
        "view.overprintPreview" => vectorcraft_render::proof::view().overprint,
        "view.proofSetup" => p.get("target").and_then(Value::as_str) == Some(vectorcraft_render::proof::view().setup.target.id().as_str()),
        "perspective.grid.snap" => perspective_grid(app).is_some_and(|g| g.snap),
        "perspective.grid.lockStation" => perspective_grid(app).is_some_and(|g| g.lock_station),
        "file.documentColorMode" | "object.convertDocumentColorMode" => {
            let cmyk = app.session.active().is_some_and(|d| d.doc.color_mode == vectorcraft_engine::doc::ColorMode::Cmyk);
            p.get("mode").and_then(Value::as_str) == Some(if cmyk { "cmyk" } else { "rgb" })
        }
        // The ruler context menu lists the document units, the current one checked.
        "document.setUnits" => {
            p.get("units").and_then(Value::as_str).and_then(vectorcraft_engine::doc::Unit::named) == Some(app.session.general_unit())
        }
        _ => return None,
    })
}

/// The active document's perspective grid (View → Perspective Grid toggles).
fn perspective_grid(app: &VectorcraftApp) -> Option<vectorcraft_tools::distort::perspective::PerspectiveGrid> {
    app.session.active().map(|d| vectorcraft_tools::distort::perspective::PerspectiveGrid::current(&d.doc))
}

/// Label for toggles whose text flips (Outline/Preview, Hide/Show …).
pub fn dynamic_label(app: &VectorcraftApp, id: &str, label: &str) -> String {
    let v = &app.ui.view;
    match id {
        "view.outline" => if v.outline { "Preview" } else { "Outline" }.into(),
        "window.workspace.reset" => format!("Reset {}", app.ui.workspace),
        "view.edges" => if v.edges { "Hide Edges" } else { "Show Edges" }.into(),
        "view.cornerWidget" => if v.corner_widgets { "Hide Corner Widget" } else { "Show Corner Widget" }.into(),
        "view.textThreads" => if v.text_threads { "Hide Text Threads" } else { "Show Text Threads" }.into(),
        "type.hiddenCharacters" => if v.hidden_chars { "Hide Hidden Characters" } else { "Show Hidden Characters" }.into(),
        "view.gradientAnnotator" => if v.gradient_annotator { "Hide Gradient Annotator" } else { "Show Gradient Annotator" }.into(),
        "effect.last" => match &app.last_effect {
            Some((e, _)) => format!("Last Effect: {}", vectorcraft_effects::effect_info(e).map(|i| i.label).unwrap_or(e.as_str())),
            None => label.into(),
        },
        id if id.starts_with("type.recentFont") => {
            let n: usize = id["type.recentFont".len()..].parse().unwrap_or(0);
            n.checked_sub(1).and_then(|i| app.recent_fonts().get(i)).cloned().unwrap_or_else(|| "—".into())
        }
        id if id.starts_with("view.goto") => {
            let n: usize = id["view.goto".len()..].parse().unwrap_or(0);
            app.session.active().and_then(|d| n.checked_sub(1).and_then(|i| d.doc.views.get(i)).map(|v| v.name.clone())).unwrap_or_else(|| "—".into())
        }
        id if saved_selection_slot(id).is_some() => saved_selection_name(app, id).unwrap_or_else(|| "—".into()),
        "view.artboards" => if v.artboards { "Hide Artboards" } else { "Show Artboards" }.into(),
        "view.rulers" => if v.rulers { "Hide Rulers" } else { "Show Rulers" }.into(),
        "view.boundingBox" => if v.bounding_box { "Hide Bounding Box" } else { "Show Bounding Box" }.into(),
        "view.transparencyGrid" => {
            if app.session.active().is_some_and(|d| d.transparency_grid) { "Hide Transparency Grid" } else { "Show Transparency Grid" }.into()
        }
        "view.guides" => if v.guides { "Hide Guides" } else { "Show Guides" }.into(),
        "view.grid" => if v.grid { "Hide Grid" } else { "Show Grid" }.into(),
        "view.guides.lock" => if app.session.guides_locked() { "Unlock Guides" } else { "Lock Guides" }.into(),
        "view.slices.hide" => if app.session.slices_hidden() { "Show Slices" } else { "Hide Slices" }.into(),
        "view.printTiling" => if app.session.active().is_some_and(|d| d.print_tiling) { "Hide Print Tiling" } else { "Show Print Tiling" }.into(),
        id if id.starts_with("file.openRecent") => recent_slot(app, id)
            .map(|p| std::path::Path::new(p).file_name().map_or(p.clone(), |f| f.to_string_lossy().to_string()))
            .unwrap_or_else(|| "—".into()),
        // Edit Contents reads Edit Envelope while an envelope's contents are being edited.
        "object.envelope.editContents" => {
            let editing = app.session.active().is_some_and(|st| {
                st.selection
                    .objects
                    .first()
                    .and_then(|id| vectorcraft_doc::live::envelope_of(&st.doc, *id))
                    .is_some_and(|e| matches!(e.kind, vectorcraft_doc::NodeKind::Envelope { editing: true, .. }))
            });
            if editing { "Edit Envelope" } else { "Edit Contents" }.into()
        }
        "edit.undo" => app.session.active().and_then(|d| d.history.undo.last()).map(|h| format!("Undo {}", h.label)).unwrap_or_else(|| "Undo".into()),
        "edit.redo" => app.session.active().and_then(|d| d.history.redo.last()).map(|h| format!("Redo {}", h.label)).unwrap_or_else(|| "Redo".into()),
        id if id.starts_with(crate::panels::swatches::USER_SLOT) => {
            crate::panels::swatches::user_library(app, id).map_or_else(|| "—".into(), |l| l.name)
        }
        id if id.starts_with(crate::panels::graphic_styles::USER_SLOT) => {
            crate::panels::graphic_styles::user_library(app, id).map_or_else(|| "—".into(), |l| l.name)
        }
        id if id.starts_with(crate::dialogs::perspective_presets::SLOT) => {
            crate::dialogs::perspective_presets::slot_preset(app, id).unwrap_or_else(|| "—".into())
        }
        "perspective.grid.show" => if perspective_grid(app).is_some_and(|g| g.visible) { "Hide Grid" } else { "Show Grid" }.into(),
        "perspective.grid.rulers" => if perspective_grid(app).is_some_and(|g| g.rulers) { "Hide Rulers" } else { "Show Rulers" }.into(),
        "perspective.grid.lock" => if perspective_grid(app).is_some_and(|g| g.locked) { "Unlock Grid" } else { "Lock Grid" }.into(),
        _ => label.into(),
    }
}

/// Does the menu item `id` show a name that is user, file or system data rather than an interface
/// label: a recent file, a saved view, a font, a user library or preset, a custom workspace, a
/// plug-in? Such names are shown as they are, never translated (a workspace the user calls
/// "Layers" stays "Layers").
fn shows_a_name(id: &str, label: &str) -> bool {
    const SLOTS: [&str; 6] = [
        "file.openRecent",
        "view.goto",
        "type.recentFont",
        crate::panels::swatches::USER_SLOT,
        crate::panels::graphic_styles::USER_SLOT,
        crate::dialogs::perspective_presets::SLOT,
    ];
    SLOTS.iter().any(|s| id.starts_with(s))
        || saved_selection_slot(id).is_some()
        || matches!(id, "text.setStyle" | "plugin.dialog")
        || (id == "window.workspace" && !crate::workspaces::is_builtin(label))
        || (matches!(id, "effect.apply" | "effect.dialog") && vectorcraft_effects::plugin_effects().iter().any(|e| e.label == label))
}

/// The label of a menu item as drawn: [`dynamic_label`] in the UI language. Labels assembled
/// around a name (Undo *Move*, Reset *Essentials*, Last Effect: *Drop Shadow*) are translated as
/// templates so the name can move; names that are user data (files, views, fonts, custom
/// workspaces, plug-ins: [`shows_a_name`]) pass through.
pub fn display_label(app: &VectorcraftApp, id: &str, label: &str) -> String {
    let lang = crate::i18n::current();
    let fmt = crate::i18n::fmt;
    match id {
        "edit.undo" | "edit.redo" => {
            let last = app.session.active().and_then(|d| if id == "edit.undo" { d.history.undo.last() } else { d.history.redo.last() });
            match last {
                Some(h) if id == "edit.undo" => fmt(tl!("Undo {name}"), &[("name", tl!(&h.label))]),
                Some(h) => fmt(tl!("Redo {name}"), &[("name", tl!(&h.label))]),
                None => tl!(if id == "edit.undo" { "Undo" } else { "Redo" }).to_string(),
            }
        }
        "window.workspace.reset" => {
            let name = &app.ui.workspace;
            fmt(tl!("Reset {name}"), &[("name", if crate::workspaces::is_builtin(name) { tl!(name) } else { name })])
        }
        "effect.last" => match &app.last_effect {
            Some((e, _)) => {
                let name = vectorcraft_effects::effect_info(e).map(|i| i.label).unwrap_or(e.as_str());
                fmt(tl!("Last Effect: {name}"), &[("name", tl!(name))])
            }
            None => crate::i18n::tr_id(lang, id, label).to_string(),
        },
        _ => {
            let shown = dynamic_label(app, id, label);
            if shows_a_name(id, &shown) { shown } else { crate::i18n::tr_id(lang, id, &shown).to_string() }
        }
    }
}

/// Menu slots that are left out while they have nothing to show (unused saved views, recent files,
/// User Defined swatch and graphic style libraries).
fn hidden_when_disabled(id: &str) -> bool {
    id.starts_with("view.goto")
        || saved_selection_slot(id).is_some()
        || id.starts_with("file.openRecent")
        || id.starts_with(crate::panels::swatches::USER_SLOT)
        || id.starts_with(crate::panels::graphic_styles::USER_SLOT)
        || id.starts_with(crate::dialogs::perspective_presets::SLOT)
}

/// Menu items another item stands in for right now: Envelope Distort's Reset with Warp and Reset
/// with Mesh take the place of Make with Warp and Make with Mesh while an envelope is selected.
fn swapped_out(app: &VectorcraftApp, id: &str) -> bool {
    let reset = match id {
        "object.envelope.makeWithWarp" | "object.envelope.makeWithMesh" => false,
        "object.envelope.resetWithWarp" | "object.envelope.resetWithMesh" => true,
        _ => return false,
    };
    enabled(app, "object.envelope.release") != reset
}

/// File → Open Recent Files slots (Preferences → File Handling shows 0–30 of them).
const RECENT_IDS: [&str; io::MAX_RECENT_FILES] = [
    "file.openRecent1",
    "file.openRecent2",
    "file.openRecent3",
    "file.openRecent4",
    "file.openRecent5",
    "file.openRecent6",
    "file.openRecent7",
    "file.openRecent8",
    "file.openRecent9",
    "file.openRecent10",
    "file.openRecent11",
    "file.openRecent12",
    "file.openRecent13",
    "file.openRecent14",
    "file.openRecent15",
    "file.openRecent16",
    "file.openRecent17",
    "file.openRecent18",
    "file.openRecent19",
    "file.openRecent20",
    "file.openRecent21",
    "file.openRecent22",
    "file.openRecent23",
    "file.openRecent24",
    "file.openRecent25",
    "file.openRecent26",
    "file.openRecent27",
    "file.openRecent28",
    "file.openRecent29",
    "file.openRecent30",
];

/// The saved selection a `select.recallN` slot names (none past the document's last).
fn saved_selection_name(app: &VectorcraftApp, id: &str) -> Option<String> {
    let n = saved_selection_slot(id)?.checked_sub(1)?;
    app.session.active()?.doc.saved_selections.get(n).map(|x| x.name.clone())
}

/// The recent file a `file.openRecentN` slot names (none past the preference's count).
fn recent_slot<'a>(app: &'a VectorcraftApp, id: &str) -> Option<&'a String> {
    let n: usize = id.strip_prefix("file.openRecent")?.parse().ok()?;
    io::recent_files(app).get(n.checked_sub(1)?)
}

/// How the menus show item `id` (params `p`) now: (enabled, checked: `Some` for an item that is on
/// or off), or `None` while they leave it out: an empty slot ([`hidden_when_disabled`]) or an item
/// another stands in for ([`swapped_out`]).
pub fn shown_state(app: &VectorcraftApp, id: &str, p: &Value) -> Option<(bool, Option<bool>)> {
    let en = enabled(app, id);
    if (!en && hidden_when_disabled(id)) || swapped_out(app, id) {
        return None;
    }
    Some((en, checked(app, id, p)))
}

/// One row of a menu as it shows now. The in-window menus draw these and the macOS menu bar
/// ([`crate::native_menu`]) is built from them, so the two can't drift.
pub enum Entry<'a> {
    Sep,
    /// A section header (a disabled label), in the UI language.
    Header(&'static str),
    /// A submenu: its label in the UI language, and its items.
    Sub(&'static str, &'a [Item]),
    Item(Row<'a>),
}

/// A menu item as it shows now ([`entry`]).
pub struct Row<'a> {
    /// The command and its params; `None` for an item not implemented yet (always disabled).
    pub command: Option<(&'static str, &'a Value)>,
    /// The tree's own English label, which [`click_target`] reads.
    pub source: &'static str,
    /// The label in the UI language, as it reads now (Undo *Move*, Show/Hide…, a recent file).
    pub label: std::borrow::Cow<'static, str>,
    /// The shortcut as the registry writes it (`Cmd+Shift+Z`), the user's overrides applied.
    pub shortcut: Option<&'static str>,
    pub enabled: bool,
    /// `Some` for an item that is on or off.
    pub checked: Option<bool>,
}

impl Row<'_> {
    /// What choosing it runs ([`click_target`]); `None` for an item not implemented yet.
    pub fn target(&self) -> Option<(String, Value)> {
        self.command.map(|(id, p)| click_target(self.source, id, p))
    }
}

/// Item `it` as the menus show it now, or `None` while they leave it out ([`shown_state`]).
pub fn entry<'a>(app: &VectorcraftApp, it: &'a Item) -> Option<Entry<'a>> {
    use crate::i18n::t;
    Some(match it {
        Item::Sep => Entry::Sep,
        Item::Header(h) => Entry::Header(t(h)),
        Item::Sub(label, children) => Entry::Sub(t(label), children),
        Item::Todo(label, sc) => Entry::Item(Row {
            command: None,
            source: label,
            label: t(label).into(),
            shortcut: Some(*sc).filter(|s| !s.is_empty()),
            enabled: false,
            checked: None,
        }),
        Item::Cmd(label, id, p) => {
            let (enabled, checked) = shown_state(app, id, p)?;
            Entry::Item(Row {
                command: Some((id, p)),
                source: label,
                label: display_label(app, id, label).into(),
                shortcut: item_shortcut(id, p),
                enabled,
                checked,
            })
        }
    })
}

/// Changes whenever a plug-in is installed or removed: Object › Plug-ins and Effect › Plug-ins
/// list them (the command palette's cache follows it).
pub fn plugin_revision() -> u64 {
    vectorcraft_plugins::registry::revision()
}

/// View → Perspective Grid → One/Two/Three Point Perspective: the saved-preset slots of each type.
const PERSPECTIVE_SLOTS: [[&str; crate::dialogs::perspective_presets::SLOTS]; 3] = [
    [
        "ui.perspectiveUserPreset1.1",
        "ui.perspectiveUserPreset1.2",
        "ui.perspectiveUserPreset1.3",
        "ui.perspectiveUserPreset1.4",
        "ui.perspectiveUserPreset1.5",
    ],
    [
        "ui.perspectiveUserPreset2.1",
        "ui.perspectiveUserPreset2.2",
        "ui.perspectiveUserPreset2.3",
        "ui.perspectiveUserPreset2.4",
        "ui.perspectiveUserPreset2.5",
    ],
    [
        "ui.perspectiveUserPreset3.1",
        "ui.perspectiveUserPreset3.2",
        "ui.perspectiveUserPreset3.3",
        "ui.perspectiveUserPreset3.4",
        "ui.perspectiveUserPreset3.5",
    ],
];

/// Select → saved selections: the n-th saved selection of the active document.
const SAVED_SELECTION_IDS: [&str; vectorcraft_engine::doc::SavedSelection::MAX] = [
    "select.recall1",
    "select.recall2",
    "select.recall3",
    "select.recall4",
    "select.recall5",
    "select.recall6",
    "select.recall7",
    "select.recall8",
    "select.recall9",
    "select.recall10",
    "select.recall11",
    "select.recall12",
    "select.recall13",
    "select.recall14",
    "select.recall15",
    "select.recall16",
    "select.recall17",
    "select.recall18",
    "select.recall19",
    "select.recall20",
    "select.recall21",
    "select.recall22",
    "select.recall23",
    "select.recall24",
    "select.recall25",
];

/// The 1-based slot a `select.recallN` id stands for.
fn saved_selection_slot(id: &str) -> Option<usize> {
    id.strip_prefix("select.recall")?.parse().ok()
}

/// Type → Recent Fonts slots (Preferences › Type › Number of Recent Fonts shows up to 15).
const RECENT_FONT_IDS: [&str; crate::MAX_RECENT_FONTS] = [
    "type.recentFont1",
    "type.recentFont2",
    "type.recentFont3",
    "type.recentFont4",
    "type.recentFont5",
    "type.recentFont6",
    "type.recentFont7",
    "type.recentFont8",
    "type.recentFont9",
    "type.recentFont10",
    "type.recentFont11",
    "type.recentFont12",
    "type.recentFont13",
    "type.recentFont14",
    "type.recentFont15",
];

/// Effective shortcut of a command: the user's override (Edit → Keyboard Shortcuts) or the default.
pub fn shortcut_of(id: &str) -> Option<&'static str> {
    crate::shortcut_editor::command_shortcut(id)
}

/// Is a command currently enabled?
/// Commands that would act on the text being typed, held back while an IME composes in the Type
/// tool (its marked text isn't committed yet): Undo/Redo — the native menu takes ⌘Z ahead of the
/// IME —, the clipboard and the selection.
pub(crate) fn waits_for_ime(app: &VectorcraftApp, id: &str) -> bool {
    app.session.tool_composing()
        && (matches!(
            id,
            "edit.undo" | "edit.redo" | "edit.cut" | "edit.copy" | "edit.clear" | "edit.duplicate" | "select.all" | "select.none" | "select.inverse"
        ) || id.starts_with("edit.paste"))
}

pub fn enabled(app: &VectorcraftApp, id: &str) -> bool {
    if waits_for_ime(app, id) {
        return false;
    }
    if let Some(c) = vectorcraft_engine::find_command(id) {
        // The system clipboard's contents can be pasted with an empty internal clipboard.
        return (c.enabled)(&app.session).is_ok() || app.system_paste && id.starts_with("edit.paste");
    }
    match id {
        // Save is off for a clean document that already has its own file.
        "file.save" => app.session.active().is_some_and(|d| d.path.is_none() || d.converted || d.is_dirty()),
        "type.bold" | "type.italic" => crate::panels::character::text_style(app).is_some(),
        "file.reveal" => app.services.reveal.is_some() && app.session.active().is_some_and(|d| d.path.is_some()),
        "file.place"
        | "file.export.svg"
        | "file.export.png"
        | "file.exportAs"
        | "file.exportForScreens"
        | "file.documentSetup"
        | "view.zoomIn"
        | "view.zoomOut"
        | "view.fitArtboard"
        | "view.fitAll"
        | "view.actualSize" => app.session.active().is_some(),
        id if id.starts_with("file.openRecent") => recent_slot(app, id).is_some(),
        "file.clearRecent" => !app.ui.recent_files.is_empty(),
        id if id.starts_with("type.recentFont") => {
            id["type.recentFont".len()..].parse::<usize>().is_ok_and(|n| n >= 1 && n <= app.recent_fonts().len()) && app.session.active().is_some()
        }
        id if saved_selection_slot(id).is_some() => saved_selection_name(app, id).is_some(),
        id if id.starts_with("view.goto") => {
            id["view.goto".len()..].parse::<usize>().is_ok_and(|n| n >= 1 && app.session.active().is_some_and(|d| n <= d.doc.views.len()))
        }
        "effect.dialog" | "ui.recolorDialog" => app.session.active().is_some_and(|d| !d.selection.is_empty()),
        "effect.applyLast" | "effect.last" => app.last_effect.is_some() && app.session.active().is_some_and(|d| !d.selection.is_empty()),
        "file.export.pdf" | "ui.savePdfDialog" | "ui.fileInfoDialog" | "ui.rasterEffectsSettingsDialog" => app.session.active().is_some(),
        "ui.missingFontsDialog" => !cfg!(target_arch = "wasm32") && app.session.active().is_some(),
        "ui.findFontsInFolder" => crate::picks::can(app, &crate::picks::PickRequest::Folder) && app.session.active().is_some(),
        "ui.swatchOptions" | "ui.newSwatch" | "ui.newColorGroup" => app.session.active().is_some(),
        "ui.graphicStyleOptions" => app.session.active().is_some(),
        "ui.colorBalanceDialog" | "ui.saturateDialog" => app.session.active().is_some_and(|d| !d.selection.is_empty()),
        "ui.saveSwatchLibrary" => app.session.active().is_some(),
        id if id.starts_with(crate::panels::swatches::USER_SLOT) => crate::panels::swatches::user_library(app, id).is_some(),
        "ui.flattenTransparencyDialog" => app.session.active().is_some_and(|d| !d.selection.is_empty()),
        "ui.saveGraphicStyleLibrary" => app.session.active().is_some(),
        id if id.starts_with(crate::panels::graphic_styles::USER_SLOT) => crate::panels::graphic_styles::user_library(app, id).is_some(),
        "ui.expandDialog" => app.session.active().is_some_and(|d| !d.selection.is_empty()),
        "ui.spotColors" => app.session.active().is_some(),
        "ui.menuDialog" => app.session.active().is_some_and(|d| !d.selection.is_empty()),
        "ui.dxfOptionsDialog" => app.session.active().is_some(),
        "links.editOriginal" | "links.reveal" => selected_image(app, |im| im.link.is_some()) || selected_placed(app),
        "ui.placementOptionsDialog" => selected_image(app, |_| true) || selected_placed(app),
        "ui.packageDialog" | "docInfo.save" => app.session.active().is_some(),
        "ui.epsOptionsDialog" => app.session.active().is_some(),
        "file.saveForWeb" | "file.saveForWeb.browser" => app.session.active().is_some(),
        "file.exportSelection" => app.session.active().is_some_and(|d| !d.selection.is_empty()),
        "css.copy" | "css.exportFile" => app.session.active().is_some(),
        "print.printerSetup" => app.services.print.as_ref().is_some_and(|s| s.has_setup()),
        "plugin.dialog" => app.session.active().is_some(),
        "ui.perspectiveGridDialog" | "ui.savePerspectivePreset" => app.session.active().is_some(),
        id if id.starts_with(crate::dialogs::perspective_presets::SLOT) => {
            app.session.active().is_some() && crate::dialogs::perspective_presets::slot_preset(app, id).is_some()
        }
        "ui.perspectivePlane" => app.session.active().is_some(),
        _ => true,
    }
}

/// Is a placed document selected?
fn selected_placed(app: &VectorcraftApp) -> bool {
    app.session.active().is_some_and(|st| {
        st.selection.objects.iter().any(|id| st.doc.node(*id).is_some_and(|n| matches!(n.kind, vectorcraft_doc::NodeKind::PlacedDocument(_))))
    })
}

/// Is an image `keep` accepts selected?
fn selected_image(app: &VectorcraftApp, keep: impl Fn(&vectorcraft_doc::ImageObject) -> bool) -> bool {
    app.session.active().is_some_and(|st| {
        st.selection.objects.iter().any(|id| st.doc.node(*id).is_some_and(|n| matches!(&n.kind, vectorcraft_doc::NodeKind::Image(im) if keep(im))))
    })
}

pub fn menu_tree() -> Vec<(&'static str, Vec<Item>)> {
    let panel = |label: &'static str, id: &'static str| cp(label, "window.panel", json!({ "panel": id }));
    vec![
        (
            "VectorCraft",
            vec![
                c("About VectorCraft", "help.about"),
                c("Join Our Discord", "help.discord"),
                Sep,
                c("Settings…", "edit.preferences"),
                sub(
                    "Language",
                    std::iter::once(cp("Automatic", "app.language", json!({"lang": "auto"})))
                        .chain(std::iter::once(Sep))
                        .chain(crate::i18n::Lang::all().map(|l| cp(l.name(), "app.language", json!({"lang": l.code()}))))
                        .collect(),
                ),
                Sep,
                sub("UI Brightness", Brightness::ALL.iter().map(|b| cp(b.label(), "window.brightness", json!({"brightness": b.id()}))).collect()),
                Sep,
                c("Quit VectorCraft", "app.quit"),
            ],
        ),
        (
            "File",
            vec![
                c("New…", "file.newDialog"),
                c("New from Template…", "file.newFromTemplate"),
                c("Open…", "file.open"),
                sub("Open Recent Files", {
                    let mut v: Vec<Item> = RECENT_IDS.iter().map(|id| c("Recent File", id)).collect();
                    v.push(Sep);
                    v.push(c("Clear Recent Files", "file.clearRecent"));
                    v
                }),
                c("Show in Folder", "file.reveal"),
                Sep,
                c("Close", "file.close"),
                c("Close All", "file.closeAll"),
                c("Save", "file.save"),
                c("Save As…", "file.saveAs"),
                c("Save a Copy…", "file.saveCopy"),
                c("Save as Template…", "file.saveAsTemplate"),
                c("Save as PDF…", "file.export.pdf"),
                c("Save for Office Documents…", "document.exportForOffice"),
                c("Revert", "file.revert"),
                Sep,
                c("Place…", "file.place"),
                Sep,
                sub(
                    "Export",
                    vec![
                        c("Export for Screens…", "file.exportForScreens"),
                        c("Export As…", "file.exportAs"),
                        c("Export As SVG…", "file.export.svg"),
                        c("Export As PNG…", "file.export.png"),
                        c("Save for Web (Legacy)…", "file.saveForWeb"),
                    ],
                ),
                c("Export Selection…", "file.exportSelection"),
                Sep,
                c("Package…", "ui.packageDialog"),
                sub("Scripts", vec![todos("Other Script…", "Cmd+F12")]),
                Sep,
                c("Document Setup…", "file.documentSetup"),
                sub(
                    "Document Color Mode",
                    vec![
                        cp("CMYK Color", "file.documentColorMode", json!({"mode": "cmyk"})),
                        cp("RGB Color", "file.documentColorMode", json!({"mode": "rgb"})),
                    ],
                ),
                c("File Info…", "file.info"),
                Sep,
                c("Print…", "file.print"),
            ],
        ),
        (
            "Edit",
            vec![
                c("Undo", "edit.undo"),
                c("Redo", "edit.redo"),
                Sep,
                c("Cut", "edit.cut"),
                c("Copy", "edit.copy"),
                c("Paste", "edit.paste"),
                c("Paste in Front", "edit.pasteInFront"),
                c("Paste in Back", "edit.pasteInBack"),
                c("Paste in Place", "edit.pasteInPlace"),
                c("Paste on All Artboards", "edit.pasteOnAllArtboards"),
                c("Paste without Formatting", "edit.pasteWithoutFormatting"),
                c("Clear", "edit.clear"),
                Sep,
                cp("Find and Replace…", "edit.findReplace", json!({"find": "", "replace": "", "matchCase": false, "wholeWord": false})),
                cp("Find Next", "edit.findNext", json!({"find": ""})),
                sub("Spelling", vec![todo("Auto Spell Check"), todos("Check Spelling…", "Cmd+I"), todo("Edit Custom Dictionary…")]),
                Sep,
                sub(
                    "Edit Colors",
                    vec![
                        c("Recolor Artwork…", "ui.recolorDialog"),
                        sub(
                            "Recolor with Preset",
                            vec![
                                cp("1 Color Job…", "ui.recolorDialog", json!({"colors": 1})),
                                cp("2 Color Job…", "ui.recolorDialog", json!({"colors": 2})),
                                cp("3 Color Job…", "ui.recolorDialog", json!({"colors": 3})),
                                cp("Color Library…", "ui.recolorDialog", json!({"library": ""})),
                            ],
                        ),
                        c("Adjust Color Balance…", "ui.colorBalanceDialog"),
                        c("Blend Front to Back", "edit.colors.blendFrontToBack"),
                        c("Blend Horizontally", "edit.colors.blendHorizontally"),
                        c("Blend Vertically", "edit.colors.blendVertically"),
                        c("Convert to CMYK", "edit.colors.toCMYK"),
                        c("Convert to Grayscale", "edit.colors.toGrayscale"),
                        c("Convert to RGB", "edit.colors.toRGB"),
                        c("Invert Colors", "edit.colors.invert"),
                        cp(
                            "Overprint Black…",
                            "edit.colors.overprintBlack",
                            json!({"remove": false, "percentage": 100, "fill": true, "stroke": true, "includeCmyBlacks": false, "includeSpotBlacks": false}),
                        ),
                        c("Saturate…", "ui.saturateDialog"),
                    ],
                ),
                c("Edit Original", "links.editOriginal"),
                Sep,
                c("Transparency Flattener Presets…", "ui.flattenerPresetsDialog"),
                c("Print Presets…", "ui.printPresetsDialog"),
                c("PDF Presets…", "ui.pdfPresetsDialog"),
                c("Perspective Grid Presets…", "ui.perspectivePresetsDialog"),
                Sep,
                c("Color Settings…", "edit.colorSettings"),
                c("Assign Profile…", "edit.assignProfile"),
                Sep,
                c("Keyboard Shortcuts…", "edit.keyboardShortcuts"),
                c("Preferences…", "edit.preferences"),
            ],
        ),
        (
            "Object",
            vec![
                sub("Transform", transform_items()),
                sub("Arrange", arrange_items()),
                sub(
                    "Align",
                    vec![
                        cp("Horizontal Align Left", "object.align", json!({"horizontal": "left"})),
                        cp("Horizontal Align Center", "object.align", json!({"horizontal": "center"})),
                        cp("Horizontal Align Right", "object.align", json!({"horizontal": "right"})),
                        cp("Vertical Align Top", "object.align", json!({"vertical": "top"})),
                        cp("Vertical Align Center", "object.align", json!({"vertical": "center"})),
                        cp("Vertical Align Bottom", "object.align", json!({"vertical": "bottom"})),
                    ],
                ),
                Sep,
                c("Group", "object.group"),
                c("Ungroup", "object.ungroup"),
                sub(
                    "Lock",
                    vec![c("Selection", "object.lock"), c("All Artwork Above", "object.lock.above"), c("Other Layers", "object.lock.otherLayers")],
                ),
                c("Unlock All", "object.unlockAll"),
                sub(
                    "Hide",
                    vec![c("Selection", "object.hide"), c("All Artwork Above", "object.hide.above"), c("Other Layers", "object.hide.otherLayers")],
                ),
                c("Show All", "object.showAll"),
                Sep,
                c("Expand…", "ui.expandDialog"),
                c("Expand Appearance", "effect.expandAppearance"),
                c("Crop Image", "object.cropImage"),
                c("Rasterize…", "object.rasterize"),
                cp("Create Gradient Mesh…", "object.mesh.create", json!({"rows": 4, "cols": 4, "appearance": "flat", "highlight": 100})),
                cp(
                    "Create Object Mosaic…",
                    "object.createObjectMosaic",
                    json!({"columns": 10, "rows": 10, "spacingX": 0, "spacingY": 0, "gray": false, "deleteRaster": false}),
                ),
                c("Vector Halftone…", "object.vectorHalftone"),
                c("Create Trim Marks", "object.createTrimMarks"),
                c("Flatten Transparency…", "ui.flattenTransparencyDialog"),
                Sep,
                c("Make Pixel Perfect", "object.makePixelPerfect"),
                Sep,
                sub(
                    "Slice",
                    vec![
                        c("Make", "object.slice.make"),
                        c("Release", "object.slice.release"),
                        c("Create from Guides", "object.slice.fromGuides"),
                        c("Create from Selection", "object.slice.fromSelection"),
                        Sep,
                        c("Duplicate Slice", "object.slice.duplicate"),
                        c("Combine Slices", "object.slice.combine"),
                        c("Divide Slices…", "object.slice.divide"),
                        Sep,
                        c("Delete All", "object.slice.deleteAll"),
                        c("Slice Options…", "object.slice.options"),
                        c("Clip to Artboard", "object.slice.clipToArtboard"),
                    ],
                ),
                Sep,
                sub(
                    "Path",
                    vec![
                        c("Join", "path.join"),
                        c("Average…", "path.average"),
                        Sep,
                        c("Outline Stroke", "object.path.outlineStroke"),
                        c("Offset Path…", "object.path.offsetPath"),
                        c("Reverse Path Direction", "path.reverse"),
                        Sep,
                        c("Simplify…", "object.path.simplify"),
                        c("Add Anchor Points", "object.path.addAnchorPoints"),
                        c("Remove Anchor Points", "path.removeAnchors"),
                        c("Divide Objects Below", "object.path.divideObjectsBelow"),
                        c("Split Into Grid…", "object.path.splitIntoGrid"),
                        Sep,
                        cp("Clean Up…", "object.path.cleanUp", json!({"strayPoints": true, "unpaintedObjects": true, "emptyTextPaths": true})),
                    ],
                ),
                sub("Shape", vec![c("Convert to Shape", "object.shape.convertToShape"), c("Expand Shape", "object.expandShape")]),
                sub(
                    "Pattern",
                    vec![c("Make", "object.pattern.make"), c("Edit Pattern", "object.pattern.edit"), c("Tile Edge Color…", "ui.tileEdgeColor")],
                ),
                sub(
                    "Repeat",
                    vec![
                        c("Radial", "object.repeat.radial"),
                        c("Grid", "object.repeat.grid"),
                        c("Mirror", "object.repeat.mirror"),
                        Sep,
                        c("Release", "object.repeat.release"),
                        c("Options…", "object.repeat.options"),
                    ],
                ),
                sub(
                    "Blend",
                    vec![
                        c("Make", "object.blend.make"),
                        c("Release", "object.blend.release"),
                        Sep,
                        c("Blend Options…", "object.blend.options"),
                        Sep,
                        c("Expand", "object.blend.expand"),
                        Sep,
                        c("Replace Spine", "object.blend.replaceSpine"),
                        c("Reverse Spine", "object.blend.reverseSpine"),
                        c("Reverse Front to Back", "object.blend.reverseFrontToBack"),
                    ],
                ),
                sub(
                    "Envelope Distort",
                    vec![
                        // While an envelope is selected, Reset takes Make's place (`swapped_out`).
                        c("Make with Warp…", "object.envelope.makeWithWarp"),
                        c("Reset with Warp…", "object.envelope.resetWithWarp"),
                        c("Make with Mesh…", "object.envelope.makeWithMesh"),
                        c("Reset with Mesh…", "object.envelope.resetWithMesh"),
                        c("Make with Top Object", "object.envelope.makeWithTopObject"),
                        Sep,
                        c("Release", "object.envelope.release"),
                        c("Envelope Options…", "object.envelope.options"),
                        c("Expand", "object.envelope.expand"),
                        Sep,
                        c("Edit Contents", "object.envelope.editContents"),
                    ],
                ),
                sub(
                    "Perspective",
                    vec![
                        c("Attach to Active Plane", "perspective.attach"),
                        c("Release with Perspective", "perspective.release"),
                        c("Move Plane to Match Object", "perspective.plane.matchObject"),
                        c("Edit Text", "perspective.editText"),
                    ],
                ),
                sub(
                    "Live Paint",
                    vec![
                        c("Make", "livePaint.make"),
                        c("Merge", "livePaint.merge"),
                        c("Release", "livePaint.release"),
                        Sep,
                        todo("Gap Options…"),
                        Sep,
                        c("Expand", "livePaint.expand"),
                    ],
                ),
                sub(
                    "Image Trace",
                    vec![
                        // Traced at once with the Default preset, as the Control bar's Image Trace
                        // button does; other presets come from its arrow or the Image Trace panel
                        // (#543: the "…" form asked for a preset's name in a text box).
                        cp("Make", "imageTrace.make", json!({"preset": "Default"})),
                        cp("Make and Expand", "imageTrace.makeAndExpand", json!({"preset": "Default"})),
                        c("Release", "imageTrace.release"),
                        c("Expand", "imageTrace.expand"),
                    ],
                ),
                sub(
                    "Text Wrap",
                    vec![
                        c("Make", "object.textWrap.make"),
                        c("Release", "object.textWrap.release"),
                        c("Text Wrap Options…", "object.textWrap.options"),
                    ],
                ),
                Sep,
                sub(
                    "Clipping Mask",
                    vec![
                        c("Make", "object.clippingMask.make"),
                        c("Release", "object.clippingMask.release"),
                        c("Edit Contents", "object.clippingMask.editContents"),
                        c("Edit Clipping Path", "object.clippingMask.editMask"),
                    ],
                ),
                sub("Compound Path", vec![c("Make", "object.compoundPath.make"), c("Release", "object.compoundPath.release")]),
                sub(
                    "Artboards",
                    vec![
                        c("Convert to Artboards", "artboard.convertToArtboards"),
                        c("Rearrange All Artboards…", "artboard.rearrange"),
                        Sep,
                        c("Fit to Artwork Bounds", "artboard.fitToArt"),
                        c("Fit to Selected Art", "artboard.fitToSelection"),
                    ],
                ),
                sub("Graph", vec![c("Type…", "graph.setType"), c("Data…", "graph.setData"), todo("Design…"), todo("Column…"), todo("Marker…")]),
                sub(
                    "Collect for Export",
                    vec![
                        cp("As Single Asset", "assets.add", json!({"multiple": false})),
                        cp("As Multiple Assets", "assets.add", json!({"multiple": true})),
                    ],
                ),
                Sep,
                sub("Plug-ins", crate::dialogs::plugin::object_menu()),
            ],
        ),
        (
            "Type",
            vec![
                sub("Font", font_items()),
                sub("Recent Fonts", RECENT_FONT_IDS.iter().map(|id| c("Recent Font", id)).collect()),
                sub("Size", TYPE_SIZES.iter().map(|(l, n)| cp(l, "text.setStyle", json!({ "size": n }))).collect()),
                c("Bold", "type.bold"),
                c("Italic", "type.italic"),
                Sep,
                panel("Glyphs", "glyphs"),
                sub("Insert Special Character", insert_items(INSERT_SPECIAL)),
                sub("Insert Whitespace Character", insert_items(INSERT_WHITESPACE)),
                sub("Insert Break Character", insert_items(INSERT_BREAK)),
                Sep,
                c("Convert To Area Type", "type.convertToAreaType"),
                c("Convert To Point Type", "type.convertToPointType"),
                c("Area Type Options…", "text.areaOptions"),
                sub(
                    "Type on a Path",
                    PATH_EFFECTS
                        .iter()
                        .map(|&(label, effect)| cp(label, "type.pathOptions", json!({ "effect": effect })))
                        .chain([Sep, c("Type on a Path Options…", "type.pathOptions")])
                        .collect(),
                ),
                sub(
                    "Threaded Text",
                    vec![
                        c("Create", "text.thread.create"),
                        c("Release Selection", "text.thread.releaseSelection"),
                        c("Remove Threading", "text.thread.remove"),
                    ],
                ),
                c("Fit Headline", "text.fitHeadline"),
                Sep,
                c("Find Font…", "type.findFont"),
                sub(
                    "Change Case",
                    vec![
                        cp("UPPERCASE", "type.changeCase", json!({"case": "upper"})),
                        cp("lowercase", "type.changeCase", json!({"case": "lower"})),
                        cp("Title Case", "type.changeCase", json!({"case": "title"})),
                        cp("Sentence case", "type.changeCase", json!({"case": "sentence"})),
                    ],
                ),
                cp("Smart Punctuation…", "type.smartPunctuation", json!({"quotes": true, "dashes": true, "ellipsis": true, "scope": "selection"})),
                Sep,
                c("Create Outlines", "type.createOutlines"),
                todo("Optical Margin Alignment"),
                Sep,
                c("Fill with Placeholder Text", "type.fillPlaceholder"),
                c("Insert Inline Symbol", "text.insertInline"),
                Sep,
                c("Show Hidden Characters", "type.hiddenCharacters"),
                sub("Type Orientation", vec![c("Horizontal", "type.orientation.horizontal"), c("Vertical", "type.orientation.vertical")]),
            ],
        ),
        ("Select", {
            let mut v = vec![
                c("All", "select.all"),
                c("All on Active Artboard", "select.allOnArtboard"),
                c("Deselect", "select.none"),
                c("Reselect", "select.reselect"),
                Sep,
                c("Inverse", "select.inverse"),
                Sep,
                c("Next Object Above", "select.nextAbove"),
                c("Next Object Below", "select.nextBelow"),
                Sep,
                sub(
                    "Same",
                    vec![
                        c("Appearance", "select.same.appearance"),
                        c("Appearance Attribute", "select.same.appearanceAttribute"),
                        c("Blending Mode", "select.same.blendingMode"),
                        c("Fill & Stroke", "select.same.fillAndStroke"),
                        c("Fill Color", "select.same.fillColor"),
                        c("Opacity", "select.same.opacity"),
                        c("Stroke Color", "select.same.strokeColor"),
                        c("Stroke Weight", "select.same.strokeWeight"),
                        c("Graphic Style", "select.same.graphicStyle"),
                        c("Shape", "select.same.shapeType"),
                        c("Symbol Instance", "select.same.symbolInstance"),
                        Sep,
                        c("Font Family", "select.same.fontFamily"),
                        c("Font Family & Style", "select.same.fontFamilyStyle"),
                        c("Font Family, Style & Size", "select.same.fontFamilyStyleSize"),
                        c("Font Size", "select.same.fontSize"),
                        c("Text Fill Color", "select.same.textFillColor"),
                        c("Text Stroke Color", "select.same.textStrokeColor"),
                    ],
                ),
                sub(
                    "Object",
                    vec![
                        c("All on Same Layers", "select.object.allOnSameLayers"),
                        c("Direction Handles", "select.object.directionHandles"),
                        Sep,
                        c("Brush Strokes", "select.object.brushStrokes"),
                        c("Bristle Brush Strokes", "select.object.bristleBrushStrokes"),
                        c("Clipping Masks", "select.object.clippingMasks"),
                        c("Slices", "select.object.slices"),
                        c("Stray Points", "select.object.strayPoints"),
                        c("Open Paths", "select.object.openPaths"),
                        Sep,
                        c("All Text Objects", "select.object.textObjects"),
                        c("Point Text Objects", "select.object.pointText"),
                        c("Area Text Objects", "select.object.areaText"),
                    ],
                ),
                todo("Start Global Edit"),
                Sep,
                c("Save Selection…", "select.save"),
                c("Edit Selection…", "select.editSaved"),
                Sep,
            ];
            v.extend(SAVED_SELECTION_IDS.iter().map(|id| c("Saved Selection", id)));
            v
        }),
        ("Effect", effect_menu()),
        (
            "View",
            vec![
                c("Outline", "view.outline"),
                c("Overprint Preview", "view.overprintPreview"),
                c("Pixel Preview", "view.pixelPreview"),
                c("Trim View", "view.trimView"),
                c("Presentation Mode", "view.presentation"),
                sub(
                    "Screen Mode",
                    vec![
                        cp("Normal Screen Mode", "view.screenMode", json!({"mode": 0})),
                        cp("Full Screen Mode with Menu Bar", "view.screenMode", json!({"mode": 1})),
                        cp("Full Screen Mode", "view.screenMode", json!({"mode": 2})),
                    ],
                ),
                Sep,
                sub(
                    "Proof Setup",
                    vec![
                        cp("Working CMYK", "view.proofSetup", json!({"target": "workingCmyk", "proof": true})),
                        cp("Legacy Macintosh RGB (Gamma 1.8)", "view.proofSetup", json!({"target": "legacyMacRgb", "proof": true})),
                        cp("Internet Standard RGB (sRGB)", "view.proofSetup", json!({"target": "srgb", "proof": true})),
                        cp("Monitor RGB", "view.proofSetup", json!({"target": "monitorRgb", "proof": true})),
                        cp("Color blindness – Protanopia-type", "view.proofSetup", json!({"target": "protanopia", "proof": true})),
                        cp("Color blindness – Deuteranopia-type", "view.proofSetup", json!({"target": "deuteranopia", "proof": true})),
                        cp("Customize…", "window.panel", json!({"panel": "separations"})),
                    ],
                ),
                c("Proof Colors", "view.proofColors"),
                Sep,
                c("Zoom In", "view.zoomIn"),
                c("Zoom Out", "view.zoomOut"),
                c("Fit Artboard in Window", "view.fitArtboard"),
                c("Fit All in Window", "view.fitAll"),
                c("Actual Size", "view.actualSize"),
                Sep,
                c("Reset Rotate View", "view.rotateReset"),
                Sep,
                c("Hide Edges", "view.edges"),
                c("Hide Artboards", "view.artboards"),
                c("Show Print Tiling", "view.printTiling"),
                c("Hide Slices", "view.slices.hide"),
                c("Lock Slices", "view.slices.lock"),
                Sep,
                sub("Rulers", vec![c("Show Rulers", "view.rulers"), todos("Change to Global Rulers", "Cmd+Alt+R"), todo("Show Video Rulers")]),
                c("Hide Bounding Box", "view.boundingBox"),
                c("Show Transparency Grid", "view.transparencyGrid"),
                c("Hide Text Threads", "view.textThreads"),
                c("Hide Gradient Annotator", "view.gradientAnnotator"),
                c("Hide Corner Widget", "view.cornerWidget"),
                Sep,
                sub(
                    "Guides",
                    vec![
                        c("Hide Guides", "view.guides"),
                        c("Lock Guides", "view.guides.lock"),
                        c("Make Guides", "view.guides.make"),
                        c("Release Guides", "view.guides.release"),
                        c("Clear Guides", "view.guides.clear"),
                    ],
                ),
                c("Smart Guides", "view.smartGuides"),
                sub(
                    "Perspective Grid",
                    vec![
                        c("Show Grid", "perspective.grid.show"),
                        c("Show Rulers", "perspective.grid.rulers"),
                        c("Snap to Grid", "perspective.grid.snap"),
                        c("Lock Grid", "perspective.grid.lock"),
                        c("Lock Station Point", "perspective.grid.lockStation"),
                        Sep,
                        c("Define Grid…", "ui.perspectiveGridDialog"),
                        sub("One Point Perspective", crate::dialogs::perspective_presets::menu(1, PERSPECTIVE_SLOTS[0])),
                        sub("Two Point Perspective", crate::dialogs::perspective_presets::menu(2, PERSPECTIVE_SLOTS[1])),
                        sub("Three Point Perspective", crate::dialogs::perspective_presets::menu(3, PERSPECTIVE_SLOTS[2])),
                        Sep,
                        c("Save Grid as Preset…", "ui.savePerspectivePreset"),
                    ],
                ),
                c("Show Grid", "view.grid"),
                c("Snap to Grid", "view.snapToGrid"),
                c("Snap to Pixel", "view.snapToPixel"),
                c("Snap to Point", "view.snapToPoint"),
                todo("Snap to Glyph"),
                Sep,
                c("New View…", "view.saved.new"),
                c("Edit Views…", "view.saved.edit"),
                Sep,
                c("Saved View 1", "view.goto1"),
                c("Saved View 2", "view.goto2"),
                c("Saved View 3", "view.goto3"),
                c("Saved View 4", "view.goto4"),
                c("Saved View 5", "view.goto5"),
                c("Saved View 6", "view.goto6"),
                c("Saved View 7", "view.goto7"),
                c("Saved View 8", "view.goto8"),
                c("Saved View 9", "view.goto9"),
                c("Saved View 10", "view.goto10"),
            ],
        ),
        (
            "Window",
            vec![
                c("New Window", "window.newWindow"),
                sub("Arrange", vec![todo("Cascade"), todo("Tile"), todo("Float in Window"), todo("Consolidate All Windows")]),
                sub("Workspace", crate::workspaces::menu_items()),
                Sep,
                c("Control", "window.control"),
                c("Contextual Task Bar", "window.taskBar"),
                c("Tools", "window.toolbar"),
                sub("Toolbars", vec![c("Advanced", "window.toolbarAdvanced"), c("Double Column", "window.toolbarColumns")]),
                Sep,
                panel("Actions", "actions"),
                panel("Align", "align"),
                panel("Appearance", "appearance"),
                panel("Artboards", "artboards"),
                panel("Asset Export", crate::panels::asset_export::ID),
                panel("Attributes", "attributes"),
                panel("Brushes", "brushes"),
                panel("Color", "color"),
                panel("Color Guide", "colorGuide"),
                panel("Color Themes", "colorThemes"),
                panel("CSS Properties", crate::panels::css_properties::ID),
                panel("Document Info", "docInfo"),
                panel("Flattener Preview", crate::panels::flattener_preview::ID),
                panel("Gradient", "gradient"),
                panel("Graphic Styles", "graphicStyles"),
                panel("History", "history"),
                panel("Image Trace", "imageTrace"),
                panel("Info", "info"),
                panel("Layers", "layers"),
                panel("Libraries", "libraries"),
                panel("Links", crate::panels::links::ID),
                panel("Magic Wand", "magicWand"),
                panel("Navigator", "navigator"),
                panel("Pathfinder", "pathfinder"),
                panel("Pattern Options", "patternOptions"),
                panel("Properties", "properties"),
                panel("Separations Preview", "separations"),
                panel("Stroke", "stroke"),
                todo("SVG Interactivity"),
                panel("Swatches", "swatches"),
                panel("Symbols", "symbols"),
                panel("Transform", "transform"),
                panel("Transparency", "transparency"),
                sub(
                    "Type",
                    vec![
                        panel("Character", "character"),
                        panel("Character Styles", "charStyles"),
                        panel("Glyphs", "glyphs"),
                        panel("OpenType", "openType"),
                        panel("Paragraph", "paragraph"),
                        panel("Paragraph Styles", "paraStyles"),
                        panel("Tabs", "tabs"),
                    ],
                ),
                todo("Variables"),
                Sep,
                sub("Brush Libraries", library_placeholders()),
                sub("Graphic Style Libraries", crate::panels::graphic_styles::window_menu()),
                sub("Swatch Libraries", crate::panels::swatches::window_menu()),
                sub("Symbol Libraries", library_placeholders()),
            ],
        ),
        (
            "Help",
            vec![
                c("Join Our Discord", "help.discord"),
                c("ArtCraft Website", "help.website"),
                c("VectorCraft on getartcraft.com", "help.appPage"),
                c("VectorCraft on GitHub", "help.github"),
                Sep,
                c("Search Commands…", "help.commandPalette"),
                todos("VectorCraft Help…", "F1"),
                Sep,
                c("About VectorCraft", "help.about"),
            ],
        ),
    ]
}

/// Object → Transform (also in the canvas context menu).
fn transform_items() -> Vec<Item> {
    vec![
        c("Transform Again", "object.transformAgain"),
        Sep,
        c("Move…", "object.move"),
        c("Rotate…", "object.rotate"),
        c("Reflect…", "object.reflect"),
        c("Scale…", "object.scale"),
        c("Shear…", "object.shear"),
        Sep,
        c("Transform Each…", "object.transformEach"),
        Sep,
        c("Reset Bounding Box", "object.resetBoundingBox"),
    ]
}

/// Object → Arrange (also in the canvas context menu).
fn arrange_items() -> Vec<Item> {
    vec![
        c("Bring to Front", "object.arrange.bringToFront"),
        c("Bring Forward", "object.arrange.bringForward"),
        c("Send Backward", "object.arrange.sendBackward"),
        c("Send to Back", "object.arrange.sendToBack"),
        Sep,
        c("Send to Current Layer", "object.arrange.sendToCurrentLayer"),
    ]
}

/// The canvas context menu (right-click): what applies to the selection, or to the view when
/// nothing is selected. Commands that can't run now are left out rather than greyed out.
pub fn context_items(app: &VectorcraftApp) -> Vec<Item> {
    use vectorcraft_doc::NodeKind;
    let Some(st) = app.session.active() else { return vec![] };
    let roots: Vec<&vectorcraft_doc::Node> = st.selection.objects.iter().filter_map(|id| st.doc.node(*id)).collect();
    let any = |f: fn(&NodeKind) -> bool| roots.iter().any(|n| f(&n.kind));
    let several = roots.len() >= 2;
    let mut v = vec![c("Undo", "edit.undo"), c("Redo", "edit.redo"), Sep];
    if st.isolation.is_some() {
        v.extend([c("Exit Isolation Mode", "object.exitIsolation"), Sep]);
    }
    if roots.is_empty() {
        v.extend([
            c("Paste", "edit.paste"),
            Sep,
            c("Zoom In", "view.zoomIn"),
            c("Zoom Out", "view.zoomOut"),
            c("Fit Artboard in Window", "view.fitArtboard"),
            Sep,
            c("Show Rulers", "view.rulers"),
            c("Show Grid", "view.grid"),
            c("Hide Guides", "view.guides"),
            c("Lock Guides", "view.guides.lock"),
            Sep,
            c("Select All", "select.all"),
        ]);
    } else {
        v.extend([c("Cut", "edit.cut"), c("Copy", "edit.copy"), c("Paste", "edit.paste"), Sep]);
        if let [one] = roots.as_slice()
            && one.is_container()
        {
            v.push(c("Isolate Selected Group", "object.isolate"));
        }
        if several {
            v.push(c("Group", "object.group"));
        }
        if any(|k| matches!(k, NodeKind::Group { clip: false, .. })) {
            v.push(c("Ungroup", "object.ungroup"));
        }
        let paths = any(|k| matches!(k, NodeKind::Path { .. }));
        if paths {
            v.extend([c("Join", "path.join"), c("Average…", "path.average")]);
        }
        if !st.selection.anchors.is_empty() {
            v.extend([c("Remove Anchor Points", "path.removeAnchors"), c("Cut Path at Selected Anchor Points", "path.cutAtAnchors")]);
        }
        if several {
            v.push(c("Make Clipping Mask", "object.clippingMask.make"));
        }
        if any(|k| matches!(k, NodeKind::Group { clip: true, .. })) {
            v.push(c("Release Clipping Mask", "object.clippingMask.release"));
        }
        if several && paths {
            v.push(c("Make Compound Path", "object.compoundPath.make"));
        }
        if any(|k| matches!(k, NodeKind::Compound { .. })) {
            v.push(c("Release Compound Path", "object.compoundPath.release"));
        }
        v.extend([
            c("Make Guides", "view.guides.make"),
            Sep,
            sub("Transform", transform_items()),
            sub("Arrange", arrange_items()),
            sub(
                "Select",
                vec![c("Next Object Above", "select.nextAbove"), c("Next Object Below", "select.nextBelow"), Sep, c("Deselect", "select.none")],
            ),
            Sep,
            c("Export Selection…", "file.exportSelection"),
        ]);
    }
    prune(app, v)
}

/// `items` without the commands that can't run now (Undo and Redo stay, greyed out), submenus
/// left empty, and separators that no longer separate anything.
fn prune(app: &VectorcraftApp, items: Vec<Item>) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::with_capacity(items.len());
    for it in items {
        let it = match it {
            Item::Sub(l, children) => Item::Sub(l, prune(app, children)),
            it => it,
        };
        let keep = match &it {
            Item::Cmd(_, id, _) => matches!(*id, "edit.undo" | "edit.redo") || enabled(app, id),
            Item::Sub(_, children) => !children.is_empty(),
            Item::Sep => out.last().is_some_and(|l| !matches!(l, Item::Sep)),
            _ => true,
        };
        if keep {
            out.push(it);
        }
    }
    if matches!(out.last(), Some(Item::Sep)) {
        out.pop();
    }
    out
}

/// The canvas context menu's popup; the item clicked goes in `clicked`, for [`invoke`].
pub fn context_menu_body(app: &VectorcraftApp, ui: &mut egui::Ui, clicked: &mut Option<(String, Value)>) {
    widgets::menu_scroll(ui, |ui| {
        ui.set_min_width(200.0);
        // Its toggles already say what they do (Show Rulers, Hide Guides): no check-mark gutter.
        render_items(app, ui, &context_items(app), false, clicked);
    });
}

/// The document units a right-click on a ruler offers: each runs `document.setUnits`, exactly as
/// Preferences ▸ Units ▸ General does, with the current one checked ([`checked`]).
pub fn ruler_unit_items() -> Vec<Item> {
    vectorcraft_engine::doc::Unit::ALL.iter().map(|u| cp(u.label(), "document.setUnits", json!({ "units": u.label() }))).collect()
}

/// The ruler context menu's popup (right-click a ruler or the origin box); the unit clicked goes in
/// `clicked`, for [`invoke`].
pub fn ruler_menu_body(app: &VectorcraftApp, ui: &mut egui::Ui, clicked: &mut Option<(String, Value)>) {
    widgets::menu_scroll(ui, |ui| {
        ui.set_min_width(160.0);
        render_items(app, ui, &ruler_unit_items(), true, clicked);
    });
}

/// Render the menu bar.
/// The in-window menu bar. Returns where its titles end (x): the bar itself takes the full width.
pub fn menu_bar(app: &mut VectorcraftApp, ui: &mut egui::Ui) -> f32 {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    // The press that opens a menu takes the keyboard from a focused text field: the field and its
    // selection are kept while the menus are open, so their Select All, Cut, Copy and Paste act on
    // its text (#516), the frame after the click that chose them (that click takes the keyboard
    // from it again).
    type Field = (egui::Id, egui::text_edit::TextEditState);
    let (field_key, chosen_key) = (egui::Id::new("menu-bar-field"), egui::Id::new("menu-bar-field-event"));
    if let Some(((field, state), e)) = ctx.data(|d| d.get_temp::<(Field, egui::Event)>(chosen_key)) {
        ctx.data_mut(|d| d.remove::<(Field, egui::Event)>(chosen_key));
        state.store(&ctx, field);
        to_field(&ctx, field, e);
    }
    if let Some(field) = focused_text_field(&ctx).and_then(|id| Some((id, egui::TextEdit::load_state(&ctx, id)?))) {
        ctx.data_mut(|d| d.insert_temp::<Field>(field_key, field));
    }
    let mut clicked: Option<(String, Value)> = None;
    let tree = menu_tree();
    let (end, open) = egui::MenuBar::new()
        .ui(ui, |ui| {
            let mut titles = Vec::with_capacity(tree.len());
            for (i, (title, items)) in tree.iter().enumerate() {
                let text = if i == 0 {
                    egui::RichText::new(tl!(title)).font(theme::semibold(13.0)).color(t.text)
                } else {
                    egui::RichText::new(tl!(title)).size(13.0).color(t.text)
                };
                titles.push(ui.menu_button(text, |ui| menu_body(app, ui, items, &mut clicked)).response);
            }
            (ui.cursor().min.x, switch_on_hover(ui.ctx(), &titles))
        })
        .inner;
    let field = ctx.data(|d| d.get_temp::<Field>(field_key));
    if !open && focused_text_field(&ctx).is_none() {
        ctx.data_mut(|d| d.remove::<Field>(field_key));
    }
    if let Some((id, p)) = clicked {
        match field.and_then(|f| Some((f, field_event(app, &id)?))) {
            Some(chosen) => {
                ctx.data_mut(|d| d.insert_temp(chosen_key, chosen));
                ctx.request_repaint();
            }
            None => invoke(app, &id, p),
        }
    }
    end
}

/// Whether the pointer at `p` is really over the menu title `title`: inside it, and with no
/// popup above it (a tall menu that egui moves up can cover the bar; hovering that menu must
/// not switch to the title under it).
fn pointer_reaches_title(ctx: &egui::Context, title: &egui::Response, p: egui::Pos2) -> bool {
    title.interact_rect.contains(p) && ctx.layer_id_at(p) == Some(title.layer_id)
}

/// Like a native menu bar: while one top-level menu is open, moving the pointer onto another
/// title opens that menu instead (egui alone needs a click on each title). Returns whether a menu
/// is open.
fn switch_on_hover(ctx: &egui::Context, titles: &[egui::Response]) -> bool {
    let ids: Vec<egui::Id> = titles.iter().map(egui::Popup::default_response_id).collect();
    let Some(open) = ids.iter().position(|id| egui::Popup::is_id_open(ctx, *id)) else {
        return false;
    };
    // `Response::hovered` is false while a menu's popup is open, so hit-test the titles here.
    // Only a moving pointer switches: one resting on a title leaves the open menu alone.
    let moving = ctx.input(|i| i.pointer.delta() != egui::Vec2::ZERO);
    if let Some(p) = ctx.pointer_hover_pos().filter(|_| moving)
        && let Some(i) = titles.iter().position(|title| pointer_reaches_title(ctx, title, p))
        && i != open
        && let Some(id) = ids.get(i)
    {
        egui::Popup::open_id(ctx, *id);
        ctx.request_repaint();
    }
    true
}

/// A top-level menu's popup: as wide as its widest item (label plus shortcut), at least 230 pt;
/// it scrolls when it is taller than the window.
fn menu_body(app: &VectorcraftApp, ui: &mut egui::Ui, items: &[Item], clicked: &mut Option<(String, Value)>) {
    widgets::menu_scroll(ui, |ui| {
        ui.set_min_width(230.0);
        render_items(app, ui, items, true, clicked);
    });
}

/// `items` as menu buttons; with `checks`, items that can be on or off show a check mark (or the
/// room for one).
fn render_items(app: &VectorcraftApp, ui: &mut egui::Ui, items: &[Item], checks: bool, clicked: &mut Option<(String, Value)>) {
    let t = Tokens::get(ui.ctx());
    for e in items.iter().filter_map(|it| entry(app, it)) {
        match e {
            Entry::Sep => {
                ui.separator();
            }
            Entry::Header(h) => {
                ui.label(egui::RichText::new(h).size(11.0).color(t.text_dim));
            }
            Entry::Sub(label, children) => {
                ui.menu_button(label, |ui| {
                    widgets::menu_scroll(ui, |ui| {
                        ui.set_min_width(200.0);
                        render_items(app, ui, children, checks, clicked);
                    });
                });
            }
            Entry::Item(row) => render_row(app, ui, &row, checks, clicked),
        }
    }
}

/// One menu item: its label (with a check mark, or the room for one, under `checks`), its shortcut,
/// and Type › Font's samples.
fn render_row(app: &VectorcraftApp, ui: &mut egui::Ui, row: &Row, checks: bool, clicked: &mut Option<(String, Value)>) {
    let sc = row.shortcut.map(pretty_shortcut).unwrap_or_default();
    let Some((id, p)) = row.command else {
        ui.add_enabled_ui(false, |ui| {
            ui.add(egui::Button::new(row.label.as_ref()).shortcut_text(sc));
        })
        .response
        .on_disabled_hover_text(tl!("Coming soon — tracked in the parity plan"));
        return;
    };
    let text = match row.checked.filter(|_| checks) {
        Some(true) => format!("✓  {}", row.label),
        Some(false) => format!("     {}", row.label),
        None => row.label.to_string(),
    };
    // Type → Font: each family's sample after its name (Enable in-menu font previews).
    let sampled = p.get("font").and_then(Value::as_str).filter(|_| id == "text.setStyle" && app.session.prefs.font_preview);
    let r = match sampled {
        Some(family) => {
            let slot = ui.id().with(("font-sample", family));
            let button = egui::Button::new(text).right_text(egui::Atom::custom(slot, crate::font_menu::MENU_SAMPLE_SIZE));
            let out = ui.add_enabled_ui(row.enabled, |ui| button.atom_ui(ui)).inner;
            if let Some(rect) = out.rect(slot) {
                crate::font_menu::menu_item_sample(ui, rect, family);
            }
            out.response
        }
        None => ui.add_enabled(row.enabled, egui::Button::new(text).shortcut_text(sc)),
    };
    if r.clicked() {
        *clicked = row.target();
        ui.close();
    }
}

/// What the Home screen remembers when it opens (`app.home`): the active document and the
/// document count. When either changes, the Home screen gives way to the document.
pub(crate) fn home_key(app: &VectorcraftApp) -> (Option<u64>, usize) {
    (app.session.active().map(|d| d.uid), app.session.documents().len())
}

/// Whether the Home screen is up: chosen with the Home button (`app.home`), or no document is open
/// and Preferences › General › Show The Home Screen When No Documents Are Open is on (#394). Off,
/// an app with no document shows an empty window, and the Home button still opens the screen.
pub(crate) fn home_showing(app: &VectorcraftApp) -> bool {
    app.ui.home.is_some() || (app.session.active().is_none() && app.session.prefs.show_home_screen)
}

/// What a menu click runs: items whose label ends with "…" and that carry default params open a
/// generic parameter dialog (fields = the defaults) instead of running immediately.
pub fn click_target(label: &str, id: &str, p: &Value) -> (String, Value) {
    let dialog = label.ends_with('…')
        && p.as_object().is_some_and(|o| !o.is_empty())
        && !matches!(id, "effect.dialog" | "window.panel" | "window.brightness" | "view.screenMode" | "plugin.dialog");
    if dialog {
        return ("ui.paramDialog".into(), json!({"command": id, "label": label.trim_end_matches('…'), "params": p}));
    }
    (id.to_string(), if p.is_null() { json!({}) } else { p.clone() })
}

/// The dialog of command `id` that shows the selection's current values (its query): its heading
/// and the queried values it doesn't show. Type on a Path Options leaves the brackets where they
/// are.
fn queried_dialog(id: &str) -> Option<(&'static str, &'static [&'static str])> {
    Some(match id {
        // Whether the text overflows, and Shrink Text's factor, are read-only facts.
        "text.areaOptions" => ("Area Type Options", &["overflow", "fitScale"]),
        "type.pathOptions" => ("Type on a Path Options", &["start", "end"]),
        _ => return None,
    })
}

/// The dialog (kind, fields) the menu item of command `id` opens, as the reference app's does.
fn menu_dialog(id: &str) -> Option<(&'static str, Value)> {
    Some(match id {
        "object.move" => ("move", json!({"dx": "0 pt", "dy": "0 pt"})),
        "object.rotate" => ("rotate", json!({"angle": 0})),
        "object.scale" => ("scale", json!({"sx": 100, "sy": 100, "uniform": true})),
        "object.transformEach" => (crate::dialogs::transform_each::KIND, crate::dialogs::transform_each::fields()),
        "object.reflect" => ("reflect", json!({"axis": "vertical"})),
        "object.shear" => ("shear", json!({"angle": 0, "axis": "horizontal"})),
        "path.average" => ("average", json!({"axis": "both"})),
        "object.path.offsetPath" => ("offsetPath", json!({"offset": "10 pt", "joins": "miter", "miterLimit": 4})),
        "object.path.simplify" => ("simplify", json!({"tolerance": "1 pt"})),
        "object.path.splitIntoGrid" => ("splitIntoGrid", json!({"rows": 2, "columns": 2, "gutter": "12 pt"})),
        "object.vectorHalftone" => (crate::dialogs::halftone::KIND, crate::dialogs::halftone::fields()),
        "artboard.rearrange" => (crate::dialogs::rearrange_artboards::KIND, crate::dialogs::rearrange_artboards::fields()),
        _ => return None,
    })
}

/// The event a focused text field takes for Edit-menu command `id` (Select All, Cut, Copy,
/// Paste), as the same keys would send it.
fn field_event(app: &mut VectorcraftApp, id: &str) -> Option<egui::Event> {
    Some(match id {
        "select.all" => egui::Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND },
        "edit.copy" => egui::Event::Copy,
        "edit.cut" => egui::Event::Cut,
        "edit.paste" | "edit.pasteWithoutFormatting" => egui::Event::Paste(app.system_clipboard_text()?),
        _ => return None,
    })
}

/// Invoke an item clicked in the system menu bar (macOS; its key equivalents come back to egui as
/// key presses, see [`crate::native_menu`]). While a text field has the keyboard, Select All, Cut,
/// Copy and Paste act on the field's text, as their keys do; everything else (and those commands
/// with no field focused) goes to [`invoke`].
pub fn invoke_from_system_menu(app: &mut VectorcraftApp, ctx: &egui::Context, id: &str, p: Value) {
    match focused_text_field(ctx).and_then(|f| Some((f, field_event(app, id)?))) {
        Some((field, e)) => to_field(ctx, field, e),
        None => invoke(app, id, p),
    }
}

/// The text field that has the keyboard, if any.
fn focused_text_field(ctx: &egui::Context) -> Option<egui::Id> {
    ctx.memory(|m| m.focused()).filter(|&id| egui::TextEdit::load_state(ctx, id).is_some())
}

/// Give text field `field` the keyboard (back) and event `e`, a [`field_event`], this frame.
fn to_field(ctx: &egui::Context, field: egui::Id, e: egui::Event) {
    ctx.memory_mut(|m| m.request_focus(field));
    ctx.input_mut(|i| i.events.push(e));
}

/// Invoke a menu/command id with UI side effects (dialogs for "…" commands that need input).
pub fn invoke(app: &mut VectorcraftApp, id: &str, p: Value) {
    if waits_for_ime(app, id) {
        return;
    }
    if let Some((kind, fields)) = menu_dialog(id)
        && p.as_object().is_none_or(|o| o.is_empty())
    {
        app.ui.dialog = Some(crate::state::Dialog::new(kind, fields));
        return;
    }
    // Envelope Distort: Warp Options, Envelope Mesh and Envelope Options, from the selection.
    if crate::dialogs::envelope::opens(id) && p.as_object().is_none_or(|o| o.is_empty()) {
        if let Err(e) = crate::dialogs::envelope::open(app, id) {
            app.status(e);
        }
        return;
    }
    // Repeat Options: a dialog with the selected repeat's current values.
    if id == "object.repeat.options"
        && p.as_object().is_none_or(|o| o.is_empty())
        && let Some(fields) = crate::panels::pattern_options::repeat_fields(app)
    {
        let _ = app.run("ui.paramDialog", json!({"command": id, "label": "Repeat Options", "params": fields}));
        return;
    }
    // Area Type Options and Type on a Path Options: dialogs with the selected type's current
    // values (the command's query), less those the dialog leaves alone.
    if let Some((label, hidden)) = queried_dialog(id)
        && p.as_object().is_none_or(|o| o.is_empty())
    {
        match app.session.execute(id, &json!({})) {
            Ok(mut fields) => {
                if let Some(o) = fields.as_object_mut() {
                    o.retain(|k, _| !hidden.contains(&k.as_str()));
                }
                let _ = app.run("ui.paramDialog", json!({"command": id, "label": label, "params": fields}));
            }
            Err(e) => app.status(e.to_string()),
        }
        return;
    }
    // Object → Graph → Type… / Data…: dialogs with the selected graph's current values.
    if matches!(id, "graph.setType" | "graph.setData") && p.as_object().is_none_or(|o| o.is_empty()) {
        match app.session.execute(id, &json!({})) {
            Ok(v) => {
                let (label, fields) = if id == "graph.setData" { ("Graph Data", json!({"csv": v["csv"]})) } else { ("Graph Type", v) };
                let _ = app.run("ui.paramDialog", json!({"command": id, "label": label, "params": fields}));
            }
            Err(e) => app.status(e.to_string()),
        }
        return;
    }
    // New View… / Edit Views…: name dialogs.
    if id == "view.saved.new" && p.as_object().is_none_or(|o| o.is_empty()) {
        let n = app.session.active().map_or(0, |d| d.doc.views.len()) + 1;
        let _ = app.run("ui.paramDialog", json!({"command": id, "label": "New View", "params": {"name": format!("View {n}")}}));
        return;
    }
    if id == "view.saved.edit" && p.as_object().is_none_or(|o| o.is_empty()) {
        let first = app.session.active().and_then(|d| d.doc.views.first().map(|v| v.name.clone()));
        match first {
            Some(name) => {
                let _ = app
                    .run("ui.paramDialog", json!({"command": id, "label": "Edit Views", "params": {"name": name, "newName": "", "delete": false}}));
            }
            None => app.status("No saved views (View → New View…)"),
        }
        return;
    }
    // Save Selection…: a name dialog, starting from the first free "Selection N".
    if id == "select.save" && p.as_object().is_none_or(|o| o.is_empty()) {
        let name = app.session.active().map(|d| vectorcraft_engine::doc::SavedSelection::default_name(&d.doc.saved_selections)).unwrap_or_default();
        let _ = app.run("ui.paramDialog", json!({"command": id, "label": "Save Selection", "params": {"name": name}}));
        return;
    }
    // Edit Selection…: the saved selections in a list, to rename or delete.
    if id == "select.editSaved" && p.as_object().is_none_or(|o| o.is_empty()) {
        if let Err(e) = crate::dialogs::edit_selection::open(app) {
            app.status(e);
        }
        return;
    }
    // Document Raster Effects Settings and File Info: their dialogs.
    if matches!(id, "document.rasterEffectsSettings" | "file.info") && p.as_object().is_none_or(|o| o.is_empty()) {
        let r = if id == "file.info" { crate::dialogs::file_info::open(app) } else { crate::dialogs::raster_effects::open(app) };
        if let Err(e) = r {
            app.status(e);
        }
        return;
    }
    // Rasterize…: its options, starting from the document's raster effects settings.
    if id == "object.rasterize" && p.as_object().is_none_or(|o| o.is_empty()) {
        match app.session.execute("document.rasterEffectsSettings", &json!({})) {
            Ok(v) => {
                let params = json!({"ppi": v["resolution"], "colorModel": v["colorModel"], "background": v["background"], "antiAlias": if v["antiAlias"] == true { "art" } else { "none" }, "clippingMask": v["clippingMask"], "addAround": v["addAround"]});
                let _ = app.run("ui.paramDialog", json!({"command": id, "label": "Rasterize", "params": params}));
            }
            Err(e) => app.status(e.to_string()),
        }
        return;
    }
    // Text Wrap Options: a dialog with the selected wrap object's current values.
    if id == "object.textWrap.options" && p.as_object().is_none_or(|o| o.is_empty()) {
        match app.session.execute(id, &json!({})) {
            Ok(fields) => {
                let _ = app.run("ui.paramDialog", json!({"command": id, "label": "Text Wrap Options", "params": fields}));
            }
            Err(e) => app.status(e.to_string()),
        }
        return;
    }
    // Blend Options…: its dialog, on the selected blend's options.
    if id == "object.blend.options" && p.as_object().is_none_or(|o| o.is_empty()) {
        if let Err(e) = crate::dialogs::blend_options::open(app) {
            app.status(e);
        }
        return;
    }
    // Expand…: its dialog.
    if id == "object.expand" && p.as_object().is_none_or(|o| o.is_empty()) {
        if let Err(e) = crate::dialogs::expand::open(app) {
            app.status(e);
        }
        return;
    }
    // Help links: the command returns the URL; a menu click opens it (agents just get the URL).
    if matches!(id, "help.discord" | "help.website" | "help.appPage" | "help.github") {
        app.open_link(id);
        return;
    }
    if let Err(e) = app.run(id, p) {
        app.status(e);
    } else if matches!(id, "object.pattern.make" | "object.pattern.edit") {
        app.ui.open_panel = Some("patternOptions".into());
    }
}

/// `Cmd+Shift+]` → `⇧⌘]` on macOS, `Ctrl+Shift+]` elsewhere.
pub fn pretty_shortcut(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    if cfg!(target_os = "macos") {
        let mut mods = String::new();
        let parts: Vec<&str> = s.split('+').collect();
        let (key, ms): (&str, &[&str]) = if s.ends_with("++") {
            ("+", parts.get(..parts.len().saturating_sub(2)).unwrap_or_default())
        } else {
            parts.split_last().map_or(("", &[][..]), |(k, rest)| (*k, rest))
        };
        for m in ["Ctrl", "Alt", "Shift", "Cmd"] {
            if ms.contains(&m) {
                mods.push_str(match m {
                    "Ctrl" => "⌃",
                    "Alt" => "⌥",
                    "Shift" => "⇧",
                    _ => "⌘",
                });
            }
        }
        format!("{mods}{key}")
    } else {
        s.replace("Cmd", "Ctrl")
    }
}

#[derive(Serialize)]
pub struct MenuEntry {
    pub path: Vec<String>,
    pub label: String,
    pub command: Option<String>,
    pub params: Value,
    pub enabled: bool,
    pub shortcut: String,
}

/// Flattened menu for `ui.menu.list`.
pub fn menu_entries(app: &VectorcraftApp) -> Vec<MenuEntry> {
    let mut out = vec![];
    for (title, items) in menu_tree() {
        flatten(app, vec![title.to_string()], &items, &mut out);
    }
    out
}

/// Flattened canvas context menu for `ui.contextMenu.list` (paths are its submenus).
pub fn context_entries(app: &VectorcraftApp) -> Vec<MenuEntry> {
    let mut out = vec![];
    flatten(app, vec![], &context_items(app), &mut out);
    out
}

fn flatten(app: &VectorcraftApp, path: Vec<String>, items: &[Item], out: &mut Vec<MenuEntry>) {
    for it in items {
        match it {
            Item::Cmd(l, id, p) => {
                let Some((enabled, _)) = shown_state(app, id, p) else { continue };
                out.push(MenuEntry {
                    path: path.clone(),
                    label: dynamic_label(app, id, l),
                    command: Some(id.to_string()),
                    params: p.clone(),
                    enabled,
                    shortcut: item_shortcut(id, p).or_else(|| shortcut_of(id)).unwrap_or("").to_string(),
                })
            }
            Item::Todo(l, sc) => out.push(MenuEntry {
                path: path.clone(),
                label: l.to_string(),
                command: None,
                params: Value::Null,
                enabled: false,
                shortcut: sc.to_string(),
            }),
            Item::Sub(l, ch) => {
                let mut p = path.clone();
                p.push(l.to_string());
                flatten(app, p, ch, out);
            }
            _ => {}
        }
    }
}

/// Type → Size presets.
const TYPE_SIZES: [(&str, u32); 14] = [
    ("6 pt", 6),
    ("8 pt", 8),
    ("9 pt", 9),
    ("10 pt", 10),
    ("11 pt", 11),
    ("12 pt", 12),
    ("14 pt", 14),
    ("18 pt", 18),
    ("24 pt", 24),
    ("30 pt", 30),
    ("36 pt", 36),
    ("48 pt", 48),
    ("60 pt", 60),
    ("72 pt", 72),
];

/// Type → Type on a Path effects (label, `type.pathOptions` effect), also the choices of its
/// Options dialog.
pub(crate) const PATH_EFFECTS: &[(&str, &str)] =
    &[("Rainbow", "rainbow"), ("Skew", "skew"), ("3D Ribbon", "3dRibbon"), ("Stair Step", "stairStep"), ("Gravity", "gravity")];

const INSERT_SPECIAL: &[(&str, &str)] = &[
    ("Bullet", "bullet"),
    ("Copyright Symbol", "copyright"),
    ("Ellipsis", "ellipsis"),
    ("Paragraph Symbol", "paragraph"),
    ("Registered Trademark Symbol", "registered"),
    ("Section Symbol", "section"),
    ("Trademark Symbol", "trademark"),
    ("Em Dash", "emDash"),
    ("En Dash", "enDash"),
    ("Discretionary Hyphen", "discretionaryHyphen"),
    ("Nonbreaking Hyphen", "nonBreakingHyphen"),
    ("Double Left Quotation Marks", "doubleLeftQuote"),
    ("Double Right Quotation Marks", "doubleRightQuote"),
    ("Single Left Quotation Mark", "singleLeftQuote"),
    ("Single Right Quotation Mark", "singleRightQuote"),
];
const INSERT_WHITESPACE: &[(&str, &str)] = &[
    ("Em Space", "emSpace"),
    ("En Space", "enSpace"),
    ("Hair Space", "hairSpace"),
    ("Sixth Space", "sixthSpace"),
    ("Thin Space", "thinSpace"),
    ("Nonbreaking Space", "nonBreakingSpace"),
    ("Figure Space", "figureSpace"),
    ("Punctuation Space", "punctuationSpace"),
    ("Third Space", "thirdSpace"),
    ("Quarter Space", "quarterSpace"),
];
const INSERT_BREAK: &[(&str, &str)] = &[("Forced Line Break", "forcedLineBreak"), ("Tab", "tab")];

fn insert_items(list: &[(&'static str, &'static str)]) -> Vec<Item> {
    list.iter().map(|(l, c)| cp(l, "type.insert", json!({ "char": c }))).collect()
}

/// Type → Font: one item per available family, the installed fonts included. Built again only when
/// the fonts change (the menu tree is built every frame); labels are interned once each, so
/// rebuilding doesn't allocate forever.
fn font_items() -> Vec<Item> {
    static NAMES: std::sync::Mutex<std::collections::BTreeSet<&'static str>> = std::sync::Mutex::new(std::collections::BTreeSet::new());
    static ITEMS: std::sync::Mutex<(u64, Vec<Item>)> = std::sync::Mutex::new((u64::MAX, Vec::new()));
    let db = vectorcraft_text::FontDb::global();
    // Read first: fonts that load meanwhile make the next frame build the list again.
    let generation = db.generation();
    let fams = db.menu_family_list();
    let (Ok(mut names), Ok(mut items)) = (NAMES.lock(), ITEMS.lock()) else { return vec![] };
    if items.0 != generation {
        let list = fams
            .iter()
            .map(|f| {
                let label: &'static str = match names.get(f.as_str()) {
                    Some(n) => n,
                    None => {
                        let n: &'static str = Box::leak(f.clone().into_boxed_str());
                        names.insert(n);
                        n
                    }
                };
                cp(label, "text.setStyle", json!({ "font": label }))
            })
            .collect();
        *items = (generation, list);
    }
    items.1.clone()
}

/// The raster effects' submenus of the Effect menu (the Photoshop-style effects, below the
/// vector effects): (submenu, the catalogue's menu path of its effects).
const RASTER_MENUS: [(&str, &[&str]); 2] = [("Blur", &["Effect", "Blur"]), ("Sharpen", &["Effect", "Sharpen"])];

/// The Effect menu, built from the effects catalogue (vector effects), plus raster effects.
fn effect_menu() -> Vec<Item> {
    let cat = vectorcraft_effects::effect_catalog();
    let mut out = vec![
        c("Apply Last Effect", "effect.applyLast"),
        c("Last Effect…", "effect.last"),
        Sep,
        c("Document Raster Effects Settings…", "document.rasterEffectsSettings"),
        Sep,
        Item::Header("Vector Effects"),
    ];
    // Submenus (and Crop Marks, an item of its own) in the reference app's order.
    let order = [
        "3D and Materials",
        "Color Adjustments",
        "Convert to Shape",
        "Crop Marks",
        "Distort & Transform",
        "Path",
        "Pathfinder",
        "Stylize",
        "SVG Filters",
        "Warp",
    ];
    let top_level = |e: &vectorcraft_effects::EffectInfo| e.menu == ["Effect"] && order.contains(&e.label);
    // The effects whose catalogue menu path is `path`.
    let items_at = |path: &[&str]| -> Vec<Item> {
        cat.iter()
            .filter(|e| e.menu == path)
            .map(|e| match e.defaults.as_object().is_some_and(|o| o.is_empty()) {
                // No options (Effect → Pathfinder): apply directly, like Illustrator.
                true => Item::Cmd(e.label, "effect.apply", json!({ "effect": e.id })),
                false => Item::Cmd(e.label, "effect.dialog", json!({ "effect": e.id })),
            })
            .collect()
    };
    for sub_name in order {
        if let Some(e) = cat.iter().find(|e| top_level(e) && e.label == sub_name) {
            out.push(Item::Cmd(e.label, "effect.apply", json!({ "effect": e.id })));
            continue;
        }
        let items = items_at(&["Effect", sub_name]);
        if items.is_empty() {
            let placeholder = match sub_name {
                "3D and Materials" => vec![todo("Extrude & Bevel…"), todo("Revolve…"), todo("Inflate…"), todo("Rotate…"), todo("Materials…")],
                "SVG Filters" => vec![todo("Apply SVG Filter…")],
                _ => continue,
            };
            out.push(sub(sub_name, placeholder));
        } else {
            out.push(sub(sub_name, items));
        }
    }
    // Live-effect plug-ins close the vector effects.
    out.extend(crate::dialogs::plugin::effect_menu());
    let raster: Vec<Item> = RASTER_MENUS
        .iter()
        .filter_map(|(name, path)| {
            let items = items_at(path);
            (!items.is_empty()).then(|| sub(name, items))
        })
        .collect();
    if !raster.is_empty() {
        out.push(Sep);
        out.push(Item::Header("Raster Effects"));
        out.extend(raster);
    }
    // Anything not placed above (future effects) still shows up.
    let placed = |e: &vectorcraft_effects::EffectInfo| {
        top_level(e) || order.iter().any(|s| e.menu == ["Effect", *s]) || RASTER_MENUS.iter().any(|(_, path)| e.menu == *path)
    };
    for e in cat.iter().filter(|e| !placed(e)) {
        out.push(Item::Cmd(e.label, "effect.dialog", json!({ "effect": e.id })));
    }
    out
}

/// The shortcut a menu item shows: its command's (items without params), or a panel's for the
/// Window menu's `window.panel {panel}` items.
pub fn item_shortcut(id: &str, p: &Value) -> Option<&'static str> {
    match (id, p.get("panel").and_then(Value::as_str)) {
        ("window.panel", Some(panel)) => crate::shortcut_editor::panel_shortcut(panel),
        _ if p.is_null() => shortcut_of(id),
        _ => None,
    }
}

/// Labels only the canvas context menu ([`context_items`]) shows, so the catalog tests can insist
/// every one of them is translated (a test checks the list against the menu).
pub const CONTEXT_LABELS: &[&str] = &[
    "Isolate Selected Group",
    "Exit Isolation Mode",
    "Make Clipping Mask",
    "Release Clipping Mask",
    "Make Compound Path",
    "Release Compound Path",
    "Select All",
];

/// The labels [`dynamic_label`] can show in place of an item's own (Show/Hide pairs and the like),
/// so the catalog tests can insist every one of them is translated.
pub const DYNAMIC_LABELS: &[&str] = &[
    "Preview",
    "Outline",
    "Hide Edges",
    "Show Edges",
    "Hide Corner Widget",
    "Show Corner Widget",
    "Hide Text Threads",
    "Show Text Threads",
    "Hide Hidden Characters",
    "Show Hidden Characters",
    "Hide Gradient Annotator",
    "Show Gradient Annotator",
    "Hide Artboards",
    "Show Artboards",
    "Hide Rulers",
    "Show Rulers",
    "Hide Bounding Box",
    "Show Bounding Box",
    "Hide Transparency Grid",
    "Show Transparency Grid",
    "Hide Guides",
    "Show Guides",
    "Hide Grid",
    "Show Grid",
    "Unlock Guides",
    "Lock Guides",
    "Show Slices",
    "Hide Slices",
    "Hide Print Tiling",
    "Show Print Tiling",
    "Edit Envelope",
    "Edit Contents",
    "Undo",
    "Redo",
    "Unlock Grid",
    "Lock Grid",
];

/// Every English string the menus, the command palette, the panel registry, the toolbar and the
/// Preferences dialog can show: what a complete language catalog has to cover. Names that are user
/// data (fonts, recent files, saved views, libraries) and the point sizes are left out.
pub fn menu_strings() -> std::collections::BTreeSet<String> {
    use std::collections::BTreeSet;
    fn user_data(id: &str) -> bool {
        hidden_when_disabled(id) || id.starts_with("type.recentFont")
    }
    fn walk(items: &[Item], under: &str, out: &mut BTreeSet<String>) {
        for it in items {
            match it {
                Item::Cmd(l, id, _) => {
                    let size = l.strip_suffix(" pt").is_some_and(|n| n.chars().all(|c| c.is_ascii_digit()));
                    // VectorCraft › Language lists each language by its own name.
                    let language_name = *id == "app.language" && crate::i18n::Lang::all().any(|lang| lang.name() == *l);
                    if !user_data(id) && !size && !language_name && *l != "—" {
                        out.insert(l.to_string());
                    }
                }
                Item::Todo(l, _) | Item::Header(l) => {
                    out.insert(l.to_string());
                }
                Item::Sub(l, children) => {
                    out.insert(l.to_string());
                    // Type › Font lists the installed families: names, not UI text.
                    if !(under == "Type" && *l == "Font") {
                        walk(children, l, out);
                    }
                }
                Item::Sep => {}
            }
        }
    }
    let mut out = BTreeSet::new();
    for (title, items) in menu_tree() {
        out.insert(title.to_string());
        walk(&items, title, &mut out);
    }
    out.extend(crate::native_menu::MAC_LABELS.iter().map(|s| s.to_string()));
    out.extend(DYNAMIC_LABELS.iter().map(|s| s.to_string()));
    out.extend(CONTEXT_LABELS.iter().map(|s| s.to_string()));
    out.extend(UI_COMMANDS.iter().map(|c| c.1.to_string()));
    for c in vectorcraft_engine::cmd::command_specs() {
        out.insert(c.label.to_string());
        out.extend(c.menu.iter().map(|m| m.to_string()));
    }
    out.extend(crate::state::all_panels().map(|(_, label)| label.to_string()));
    out.extend(vectorcraft_tools::catalog::all_tools().map(|t| t.label.to_string()));
    out.extend(crate::toolbar::BASIC.iter().map(|c| c.0.to_string()));
    out.extend(vectorcraft_color::BlendMode::ALL.iter().map(|m| m.label().to_string()));
    out.extend(vectorcraft_effects::effect_catalog().iter().map(|e| e.label.to_string()));
    out.extend(vectorcraft_engine::cmd::prefscmds::PREF_CATEGORIES.iter().map(|c| c.to_string()));
    for sp in vectorcraft_engine::cmd::prefscmds::PREF_SPECS {
        out.insert(sp.label.to_string());
        if !sp.section.is_empty() {
            out.insert(sp.section.to_string());
        }
        if let vectorcraft_engine::cmd::prefscmds::PrefKind::Choice(opts) = sp.kind {
            out.extend(opts.iter().map(|o| o.1.to_string()));
        }
    }
    out.extend(crate::prefs_dialog::UI_FIELDS.iter().map(|f| f.2.to_string()));
    out.extend(crate::dialogs::button_labels().into_iter().map(str::to_string));
    out.extend(crate::chrome::hint_strings().into_iter().map(str::to_string));
    out.remove("");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One headless frame of the in-window menu bar; returns its titles (left to right) as
    /// (rect, id of the title's popup).
    fn bar_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<(egui::Rect, egui::Id)> {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 700.0));
        let mut layer = None;
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| {
            layer = Some(ui.layer_id());
            menu_bar(app, ui);
        });
        out.textures_delta.clear();
        let layer = layer.unwrap();
        let mut titles: Vec<(egui::Rect, egui::Id)> = ctx.viewport(|vp| {
            vp.prev_pass
                .widgets
                .get_layer(layer)
                .filter(|w| w.sense.senses_click() && w.rect.top() < 30.0)
                // `egui::Popup::default_response_id` of the title's response.
                .map(|w| (w.rect, w.id.with("popup")))
                .collect()
        });
        titles.sort_by(|a, b| a.0.left().total_cmp(&b.0.left()));
        titles
    }

    fn open_titles(ctx: &egui::Context, titles: &[(egui::Rect, egui::Id)]) -> Vec<usize> {
        (0..titles.len()).filter(|&i| egui::Popup::is_id_open(ctx, titles[i].1)).collect()
    }

    #[test]
    fn hovering_another_title_switches_the_open_menu() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        theme::apply(&ctx, Default::default());
        bar_frame(&mut app, &ctx, vec![]);
        let titles = bar_frame(&mut app, &ctx, vec![]);
        assert_eq!(titles.len(), menu_tree().len());
        // 0 is the app menu, then File, Edit, Object.
        let (file, edit, object) = (titles[1].0.center(), titles[2].0.center(), titles[3].0.center());
        let frames = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let mut t = bar_frame(app, &ctx, events);
            for _ in 0..2 {
                t = bar_frame(app, &ctx, vec![]);
            }
            t
        };
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };

        // Nothing open: hovering a title opens nothing.
        let t = frames(&mut app, vec![egui::Event::PointerMoved(edit)]);
        assert!(open_titles(&ctx, &t).is_empty());

        // Click File, then move onto Edit and on to Object: each opens in turn, alone.
        frames(&mut app, vec![egui::Event::PointerMoved(file)]);
        frames(&mut app, vec![button(file, true)]);
        let t = frames(&mut app, vec![button(file, false)]);
        assert_eq!(open_titles(&ctx, &t), vec![1], "a click opens File");
        let t = frames(&mut app, vec![egui::Event::PointerMoved(edit)]);
        assert_eq!(open_titles(&ctx, &t), vec![2], "hovering Edit opens it and closes File");
        let t = frames(&mut app, vec![egui::Event::PointerMoved(object)]);
        assert_eq!(open_titles(&ctx, &t), vec![3], "hovering Object opens it and closes Edit");

        // A pointer resting on a title doesn't switch: File opened another way (the keyboard)
        // stays open while the pointer stays still over Object.
        egui::Popup::open_id(&ctx, t[1].1);
        let t = frames(&mut app, vec![]);
        assert_eq!(open_titles(&ctx, &t), vec![1], "a still pointer leaves the open menu alone");
    }

    #[test]
    fn a_popup_covering_a_title_does_not_switch_menus() {
        let ctx = egui::Context::default();
        let p = egui::pos2(90.0, 12.0);
        let mut reaches = (true, false);
        // The popup becomes hit-testable once egui has laid it out: draw a few frames.
        for _ in 0..3 {
            let raw =
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 200.0))), ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| {
                let ctx = ui.ctx().clone();
                let covered = ui.interact(egui::Rect::from_center_size(p, egui::vec2(80.0, 24.0)), egui::Id::new("covered"), egui::Sense::click());
                let q = p + egui::vec2(0.0, 100.0);
                let free = ui.interact(egui::Rect::from_center_size(q, egui::vec2(80.0, 24.0)), egui::Id::new("free"), egui::Sense::click());
                egui::Area::new(egui::Id::new("popup")).order(egui::Order::Foreground).fixed_pos(p - egui::vec2(10.0, 10.0)).show(&ctx, |ui| {
                    ui.allocate_space(egui::vec2(20.0, 20.0));
                });
                reaches = (pointer_reaches_title(&ctx, &covered, p), pointer_reaches_title(&ctx, &free, q));
            });
            out.textures_delta.clear();
        }
        assert_eq!(reaches, (false, true));
    }

    #[test]
    fn names_in_menus_are_not_interface_labels() {
        for (id, label) in [
            ("file.openRecent1", "Layers.svg"),
            ("view.goto2", "Layers"),
            ("type.recentFont1", "Regular"),
            ("text.setStyle", "Black"),
            ("window.workspace", "Layers"),
            ("plugin.dialog", "Group"),
            ("window.userSwatchLibrary3", "Default"),
        ] {
            assert!(shows_a_name(id, label), "{id} {label}");
        }
        for (id, label) in [("object.group", "Group"), ("window.workspace", "Essentials"), ("effect.apply", "Drop Shadow")] {
            assert!(!shows_a_name(id, label), "{id} {label}");
        }
    }

    #[test]
    fn last_effect_dialog_and_view_toggles() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
        assert!(!enabled(&app, "effect.last"));
        app.last_effect = Some(("distort.roughen".into(), json!({"size": 9})));
        assert!(enabled(&app, "effect.last"));
        assert_eq!(dynamic_label(&app, "effect.last", "Last Effect…"), "Last Effect: Roughen…");
        app.run("effect.last", json!({})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.kind.as_str(), d.str("__effect"), d.f64("size", 0.0)), ("effect", "distort.roughen".to_string(), 9.0));
        for (id, on, off) in [
            ("view.textThreads", "Hide Text Threads", "Show Text Threads"),
            ("view.gradientAnnotator", "Hide Gradient Annotator", "Show Gradient Annotator"),
            ("type.hiddenCharacters", "Show Hidden Characters", "Hide Hidden Characters"),
        ] {
            assert_eq!(dynamic_label(&app, id, ""), on);
            app.run(id, json!({})).unwrap();
            assert_eq!(dynamic_label(&app, id, ""), off);
        }
    }

    #[test]
    fn panel_ids_are_case_insensitive_and_labels_work() {
        assert_eq!(normalize_panel("layers"), Some("layers"));
        assert_eq!(normalize_panel("Layers"), Some("layers"));
        assert_eq!(normalize_panel("LAYERS"), Some("layers"));
        assert_eq!(normalize_panel("Swatches"), Some("swatches"));
        assert_eq!(normalize_panel("swatches"), Some("swatches"));
        assert_eq!(normalize_panel("Color Guide"), Some("colorGuide"));
        assert_eq!(normalize_panel("colorguide"), Some("colorGuide"));
        assert_eq!(normalize_panel("  Stroke  "), Some("stroke"));
        assert_eq!(normalize_panel("nope"), None);

        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({})).unwrap();
        app.run("window.panel", json!({"panel": "Layers"})).unwrap();
        assert_eq!(app.ui.dock_tab, crate::state::DockTab::Layers);
        app.run("window.panel", json!({"panel": "Swatches"})).unwrap();
        assert_eq!(app.ui.open_panel.as_deref(), Some("swatches"));
        assert!(checked(&app, "window.panel", &json!({"panel": "Swatches"})).unwrap());
        assert!(app.run("window.panel", json!({"panel": "nope"})).is_err());
        // A collapsed dock pops the panel out of its icon, by canonical id.
        app.ui.dock_collapsed = true;
        app.run("window.panel", json!({"panel": "Layers"})).unwrap();
        assert_eq!(app.ui.open_panel.as_deref(), Some("layers"));
        assert!(checked(&app, "window.panel", &json!({"panel": "Layers"})).unwrap());
        app.ui.dock_collapsed = false;
    }

    #[test]
    fn saved_views_store_and_restore_the_view() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({})).unwrap();
        {
            let v = app.view_mut().unwrap();
            v.zoom = 3.0;
            v.center = vectorcraft_geom::Point::new(120.0, 80.0);
        }
        app.run("view.saved.new", json!({"name": "Detail"})).unwrap();
        assert!(enabled(&app, "view.goto1") && !enabled(&app, "view.goto2"));
        assert_eq!(dynamic_label(&app, "view.goto1", ""), "Detail");
        app.view_mut().unwrap().zoom = 0.5;
        app.run("view.goto1", json!({})).unwrap();
        let v = app.view().unwrap();
        assert_eq!((v.zoom, v.center.x, v.center.y), (3.0, 120.0, 80.0));
        // Unused slots stay out of the menu listing agents see.
        assert!(!menu_entries(&app).iter().any(|e| e.command.as_deref() == Some("view.goto2")));
    }

    fn app_with_a_rectangle() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        app
    }

    #[test]
    fn save_selection_asks_for_a_name() {
        let mut app = app_with_a_rectangle();
        invoke(&mut app, "select.save", json!({}));
        let d = app.ui.dialog.clone().expect("Save Selection dialog");
        assert_eq!(
            (d.kind.as_str(), d.str("__command"), d.str("__label"), d.str("name")),
            ("command", "select.save".into(), "Save Selection".into(), "Selection 1".into())
        );
        // OK saves under the name typed.
        let mut d = d;
        d.fields.insert("name".into(), json!("Logo"));
        app.ui.dialog = Some(d);
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(app.run("select.savedList", json!({})).unwrap(), json!(["Logo"]));
        // The next one starts from the first free name.
        invoke(&mut app, "select.save", json!({}));
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.str("name")), Some("Selection 1".into()));
    }

    #[test]
    fn saved_selections_are_listed_at_the_bottom_of_the_select_menu() {
        let mut app = app_with_a_rectangle();
        app.run("shape.ellipse", json!({"x": 40, "y": 0, "width": 10, "height": 10})).unwrap();
        // Nothing saved: no slot is listed or enabled.
        assert!(!enabled(&app, "select.recall1"));
        assert!(!menu_entries(&app).iter().any(|e| e.command.as_deref() == Some("select.recall1")));
        app.run("select.save", json!({"name": "Ellipse"})).unwrap();
        app.run("select.none", json!({})).unwrap();
        app.run("select.all", json!({})).unwrap();
        app.run("select.save", json!({"name": "Everything"})).unwrap();
        assert!(enabled(&app, "select.recall1") && enabled(&app, "select.recall2") && !enabled(&app, "select.recall3"));
        assert_eq!(dynamic_label(&app, "select.recall2", "Saved Selection"), "Everything");
        // A name is shown as it is, not translated.
        assert_eq!(display_label(&app, "select.recall2", "Saved Selection"), "Everything");
        assert!(menu_entries(&app).iter().any(|e| e.command.as_deref() == Some("select.recall2")));
        assert!(!menu_entries(&app).iter().any(|e| e.command.as_deref() == Some("select.recall3")));
        // Choosing one selects its objects again.
        app.run("select.none", json!({})).unwrap();
        let r = app.run("select.recall2", json!({})).unwrap();
        assert!(r["count"].as_u64().unwrap() >= 1);
        assert_eq!(app.session.active().unwrap().selection.len(), 2);
        // A deleted selection's slot leaves the menus.
        app.run("select.editSaved", json!({"name": "Ellipse", "delete": true})).unwrap();
        assert!(!enabled(&app, "select.recall2"));
        assert!(shown_state(&app, "select.recall2", &Value::Null).is_none());
        assert!(shown_state(&app, "select.recall1", &Value::Null).is_some());
    }

    #[test]
    fn edit_selection_opens_its_dialog_on_the_saved_selections() {
        let mut app = app_with_a_rectangle();
        // Nothing saved yet: the item is off, and invoking it (palette, shortcut) only says so.
        assert!(!enabled(&app, "select.editSaved"));
        invoke(&mut app, "select.editSaved", json!({}));
        assert!(app.ui.dialog.is_none());
        assert!(app.ui.status.contains("No saved selections"), "{}", app.ui.status);
        app.run("select.save", json!({"name": "Logo"})).unwrap();
        assert!(enabled(&app, "select.editSaved"));
        invoke(&mut app, "select.editSaved", json!({}));
        let d = app.ui.dialog.clone().expect("Edit Selection dialog");
        assert_eq!((d.kind.as_str(), d.str("orig0"), d.str("name0")), (crate::dialogs::edit_selection::KIND, "Logo".into(), "Logo".into()));
    }

    #[test]
    fn recent_fonts_and_corner_widget_toggle() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({})).unwrap();
        let fams = vectorcraft_text::FontDb::global().families();
        let (a, b) = (fams[0].clone(), fams[fams.len() - 1].clone());
        app.run("text.create", json!({"x": 10, "y": 10, "text": "Hi"})).unwrap();
        app.run("text.setStyle", json!({"font": a})).unwrap();
        app.run("text.setStyle", json!({"font": b})).unwrap();
        assert_eq!(app.ui.recent_fonts, vec![b.clone(), a.clone()]);
        assert_eq!(dynamic_label(&app, "type.recentFont2", "Recent Font"), a);
        assert!(enabled(&app, "type.recentFont2") && !enabled(&app, "type.recentFont3"));
        app.run("type.recentFont2", json!({})).unwrap();
        assert_eq!(app.ui.recent_fonts[0], a);
        assert_eq!(dynamic_label(&app, "view.cornerWidget", ""), "Hide Corner Widget");
        app.run("view.cornerWidget", json!({})).unwrap();
        assert_eq!(dynamic_label(&app, "view.cornerWidget", ""), "Show Corner Widget");
    }

    /// Selection & Anchor Display › Zoom to Selection (#394): Zoom In and Zoom Out bring the
    /// selection to the middle; off, they keep the view's centre.
    #[test]
    fn zoom_to_selection_centres_the_selection() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 20, "width": 40, "height": 20})).unwrap();
        let center = |app: &VectorcraftApp| app.view().unwrap().center;
        let start = center(&app);
        app.run("prefs.set", json!({"key": "zoomToSelection", "value": false})).unwrap();
        app.run("view.zoomIn", json!({})).unwrap();
        assert_eq!(center(&app), start, "off: the view's centre stays");
        app.run("prefs.set", json!({"key": "zoomToSelection", "value": true})).unwrap();
        app.run("view.zoomOut", json!({})).unwrap();
        assert_eq!(center(&app), vectorcraft_geom::Point::new(30.0, 30.0), "on: the selection's centre");
    }

    /// Preferences › Type › Number of Recent Fonts (#394): Type › Recent Fonts lists that many.
    #[test]
    fn number_of_recent_fonts_sets_how_many_are_listed() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({})).unwrap();
        app.ui.recent_fonts = (1..=15).map(|i| format!("Font {i}")).collect();
        let listed = |app: &VectorcraftApp| RECENT_FONT_IDS.iter().filter(|id| enabled(app, id)).count();
        assert_eq!(listed(&app), 10, "10 by default");
        for n in [15, 2] {
            app.run("prefs.set", json!({"key": "recentFontsCount", "value": n})).unwrap();
            assert_eq!(listed(&app), n);
        }
        assert_eq!(dynamic_label(&app, "type.recentFont2", "Recent Font"), "Font 2");
        assert!(app.run("type.recentFont3", json!({})).is_err(), "past the count");
    }

    #[test]
    fn recent_files_track_opens_and_saves() {
        let dir = std::env::temp_dir().join(format!("dc-recent-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = VectorcraftApp::new(
            vectorcraft_engine::Session::new(),
            crate::Services {
                read: Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string()))),
                write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
                ..Default::default()
            },
        );
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        let (a, b) = (dir.join("a.vectorcraft"), dir.join("b.vectorcraft"));
        for p in [&a, &b, &a] {
            app.run("file.saveAs", json!({"path": p.to_string_lossy()})).unwrap();
        }
        assert_eq!(app.ui.recent_files, [a.to_string_lossy(), b.to_string_lossy()]);
        assert_eq!(dynamic_label(&app, "file.openRecent2", ""), "b.vectorcraft");
        assert!(enabled(&app, "file.openRecent2") && !enabled(&app, "file.openRecent3"));
        app.run("file.openRecent2", json!({})).unwrap();
        assert_eq!(app.session.documents().len(), 2);
        assert_eq!(app.ui.recent_files[0], b.to_string_lossy(), "reopening moves it to the top");
        app.run("file.clearRecent", json!({})).unwrap();
        assert!(app.ui.recent_files.is_empty() && app.run("file.openRecent1", json!({})).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn every_bound_menu_command_exists() {
        fn walk(items: &[Item], bad: &mut Vec<String>) {
            for it in items {
                match it {
                    Item::Cmd(_, id, _)
                        if vectorcraft_engine::find_command(id).is_none()
                            && !UI_COMMANDS.iter().any(|c| c.0 == *id)
                            && !id.starts_with("object.path.")
                            && *id != "type.createOutlines" =>
                    {
                        bad.push(id.to_string());
                    }
                    Item::Sub(_, ch) => walk(ch, bad),
                    _ => {}
                }
            }
        }
        let mut bad = vec![];
        for (_, items) in menu_tree() {
            walk(&items, &mut bad);
        }
        assert!(bad.is_empty(), "menu items bound to unknown commands: {bad:?}");
    }

    #[test]
    fn shortcut_pretty() {
        if cfg!(target_os = "macos") {
            assert_eq!(pretty_shortcut("Cmd+Shift+]"), "⇧⌘]");
            assert_eq!(pretty_shortcut("Cmd+Alt+2"), "⌥⌘2");
        }
        assert_eq!(pretty_shortcut(""), "");
    }

    /// #543: Object › Image Trace › Make and Make and Expand trace with the Default preset at once,
    /// with no form asking for a preset's name.
    #[test]
    fn image_trace_make_traces_with_the_default_preset() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({})).unwrap();
        for cmd in ["imageTrace.make", "imageTrace.makeAndExpand"] {
            let e = menu_entries(&app).into_iter().find(|e| e.command.as_deref() == Some(cmd)).unwrap();
            assert_eq!(e.path, ["Object", "Image Trace"]);
            assert_eq!(click_target(&e.label, cmd, &e.params), (cmd.to_string(), json!({"preset": "Default"})), "{}", e.label);
        }
    }

    #[test]
    fn expand_appearance_is_an_object_menu_item_only() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({})).unwrap();
        let paths: Vec<Vec<String>> =
            menu_entries(&app).into_iter().filter(|e| e.command.as_deref() == Some("effect.expandAppearance")).map(|e| e.path).collect();
        assert_eq!(paths, [vec!["Object".to_string()]]);
    }

    #[test]
    fn menus_are_as_wide_as_their_items() {
        // Items size the popup to the widest label and shortcut instead of the widest a menu may be.
        let app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0));
        let tree = menu_tree();
        let (_, file) = tree.iter().find(|(t, _)| *t == "File").unwrap();
        let id = egui::Id::new("test-menu");
        for _ in 0..3 {
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                egui::Area::new(id).show(ui.ctx(), |ui| {
                    egui::containers::menu::menu_style(ui.style_mut());
                    ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| menu_body(&app, ui, file, &mut None));
                });
            });
            out.textures_delta.clear();
        }
        let w = ctx.memory(|m| m.area_rect(id)).unwrap().width();
        assert!((230.0..340.0).contains(&w), "File menu is {w} pt wide");
    }

    #[test]
    fn long_menus_scroll_inside_a_short_window() {
        // The Window menu is taller than a 600 pt window: it stops above the bottom edge and
        // scrolls instead of running off screen.
        let app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let tree = menu_tree();
        let (_, window) = tree.iter().find(|(t, _)| *t == "Window").unwrap();
        let id = egui::Id::new("test-long-menu");
        let rect = |height: f32| {
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, height));
            for _ in 0..3 {
                let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
                let mut out = ctx.run_ui(input, |ui| {
                    egui::Area::new(id).fixed_pos(egui::pos2(300.0, 30.0)).show(ui.ctx(), |ui| {
                        egui::containers::menu::menu_style(ui.style_mut());
                        ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| menu_body(&app, ui, window, &mut None));
                    });
                });
                out.textures_delta.clear();
            }
            ctx.memory(|m| m.area_rect(id)).unwrap()
        };
        let short = rect(600.0);
        assert!(short.bottom() <= 600.0, "Window menu ends at {} in a 600 pt window", short.bottom());
        // With room to spare it shows whole, taller than the short window allowed.
        let tall = rect(3000.0);
        assert!(tall.height() > short.height() + 100.0, "{} vs {}", tall.height(), short.height());
        assert!(tall.bottom() < 3000.0);
    }
}
