use super::state::{SUBJECT_COMMANDS, has_subject, waits_for_ime};
use super::*;

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
pub(super) fn menu_dialog(app: &VectorcraftApp, id: &str) -> Option<(&'static str, Value)> {
    Some(match id {
        // Move opens on the offset moved last and the checkboxes it was last confirmed with.
        "object.move" => ("move", crate::dialogs::transform::move_fields(app)),
        "object.rotate" => ("rotate", json!({"angle": 0})),
        "object.scale" => ("scale", json!({"sx": 100, "sy": 100, "uniform": true})),
        // Transform Each opens on the values it was last confirmed with (#1732).
        "object.transformEach" => (crate::dialogs::transform_each::KIND, crate::dialogs::transform_each::open_fields(app)),
        "object.reflect" => ("reflect", json!({"axis": "vertical", "angle": 90})),
        "object.shear" => ("shear", json!({"angle": 0, "axis": "horizontal"})),
        "path.average" => ("average", json!({"axis": "both"})),
        // Offset Path opens on the values it was last confirmed with.
        "object.path.offsetPath" if app.ui.offset_path.is_object() => ("offsetPath", app.ui.offset_path.clone()),
        "object.path.offsetPath" => ("offsetPath", json!({"offset": "10 pt", "joins": "miter", "miterLimit": 4})),
        // Simplify opens its slider, or the dialog on the fields it was last confirmed with when
        // "Open this dialog with these settings next time" was ticked.
        "object.path.simplify" if app.ui.simplify.is_object() => ("simplify", app.ui.simplify.clone()),
        "object.path.simplify" => (crate::dialogs::simplify_slider::KIND, crate::dialogs::simplify_slider::fields(app)),
        "object.path.splitIntoGrid" => {
            // The grid starts at the size of the selection.
            let size =
                app.session.active().and_then(|st| st.doc.bounds_of(&st.selection.objects, false)).map_or((0.0, 0.0), |b| (b.width(), b.height()));
            ("splitIntoGrid", crate::dialogs::path_ops::split_fields(size.0, size.1))
        }
        "object.vectorHalftone" => (crate::dialogs::halftone::KIND, crate::dialogs::halftone::fields()),
        "artboard.rearrange" => (crate::dialogs::rearrange_artboards::KIND, crate::dialogs::rearrange_artboards::fields()),
        _ => return None,
    })
}

/// The event a focused text field takes for Edit-menu command `id` (Select All, Cut, Copy,
/// Paste), as the same keys would send it.
pub(super) fn field_event(app: &mut VectorcraftApp, id: &str) -> Option<egui::Event> {
    Some(match id {
        "select.all" => egui::Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND },
        "edit.copy" => egui::Event::Copy,
        "edit.cut" => egui::Event::Cut,
        "edit.paste" | "edit.pasteWithoutFormatting" => egui::Event::Paste(app.written_clipboard_text()?),
        _ => return None,
    })
}

/// Invoke an item clicked in the system menu bar (macOS; its key equivalents come back to egui as
/// key presses, see [`crate::native_menu`]). While a text field has the keyboard, Select All, Cut,
/// Copy and Paste act on the field's text, as their keys do. While its keys select every artboard
/// ([`crate::panels::artboards::selects_all`]), Select All does too. Everything else (and those
/// commands with no field focused) goes to [`invoke`].
pub fn invoke_from_system_menu(app: &mut VectorcraftApp, ctx: &egui::Context, id: &str, p: Value) {
    match focused_text_field(ctx).and_then(|f| Some((f, field_event(app, id)?))) {
        Some((field, e)) => to_field(ctx, field, e),
        None if id == "select.all" && crate::panels::artboards::selects_all(app, ctx) => crate::panels::artboards::select_all(app),
        None => invoke(app, id, p),
    }
}

/// The text field that has the keyboard, if any.
pub(super) fn focused_text_field(ctx: &egui::Context) -> Option<egui::Id> {
    ctx.memory(|m| m.focused()).filter(|&id| egui::TextEdit::load_state(ctx, id).is_some())
}

/// Give text field `field` the keyboard (back) and event `e`, a [`field_event`], this frame.
pub(super) fn to_field(ctx: &egui::Context, field: egui::Id, e: egui::Event) {
    ctx.memory_mut(|m| m.request_focus(field));
    ctx.input_mut(|i| i.events.push(e));
}

/// Invoke a menu/command id with UI side effects (dialogs for "…" commands that need input).
/// Object › Graph › Type… or Data… (`id`): its dialog, with the selected graph's current values
/// (the command's query).
pub(crate) fn graph_dialog(app: &mut VectorcraftApp, id: &str) -> Result<Value, String> {
    // Graph Data: the worksheet.
    if id == "graph.setData" {
        return crate::dialogs::graph_data::open(app);
    }
    let v = app.session.execute(id, &json!({})).map_err(|e| e.to_string())?;
    let (label, fields) = ("Graph Type", v);
    app.run("ui.paramDialog", json!({"command": id, "label": label, "params": fields})).map(|_| json!({ "dialog": "command" }))
}

pub fn invoke(app: &mut VectorcraftApp, id: &str, p: Value) {
    if waits_for_ime(app, id) {
        return;
    }
    if SUBJECT_COMMANDS.contains(&id) && p.get("ids").is_none() && p.get("id").is_none() && !has_subject(app) {
        return;
    }
    // Params first: some dialogs' fields take a look through the session (Move's last move).
    if p.as_object().is_none_or(|o| o.is_empty())
        && let Some((kind, fields)) = menu_dialog(app, id)
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
    // Variables (data merge): a name and a kind, a name and a row of values.
    if crate::dialogs::variables::opens(id) && p.as_object().is_none_or(|o| o.is_empty()) {
        crate::dialogs::variables::open(app, id, None);
        return;
    }
    // Pathfinder Options: a dialog with the current options.
    if id == "object.pathfinder.options" && p.as_object().is_none_or(|o| o.is_empty()) {
        crate::dialogs::pathfinder_options::open(app);
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
        if let Err(e) = graph_dialog(app, id) {
            app.status(e);
        }
        return;
    }
    // Object → Graph → Design… / Column… / Marker…: the document's graph designs.
    if matches!(id, "graph.design" | "graph.column" | "graph.marker") && p.as_object().is_none_or(|o| o.is_empty()) {
        let r = match id {
            "graph.design" => crate::dialogs::graph_design::open_design(app),
            "graph.column" => crate::dialogs::graph_design::open_column(app),
            _ => crate::dialogs::graph_design::open_marker(app),
        };
        if let Err(e) = r {
            app.status(e);
        }
        return;
    }
    // New View…: a name dialog, starting from the first free "View N".
    if id == "view.saved.new" && p.as_object().is_none_or(|o| o.is_empty()) {
        let name = app.session.active().map(|d| vectorcraft_engine::doc::SavedView::default_name(&d.doc.views)).unwrap_or_default();
        let _ = app.run("ui.paramDialog", json!({"command": id, "label": "New View", "params": {"name": name}}));
        return;
    }
    // Edit Views…: the saved views in a list, to rename or delete.
    if id == "view.saved.edit" && p.as_object().is_none_or(|o| o.is_empty()) {
        if let Err(e) = crate::dialogs::edit_views::open(app) {
            app.status(e);
        }
        return;
    }
    // Rotate View with no angle, as the command palette runs it, opens a dialog for the angle, with
    // the view's angle filled in.
    if id == "view.rotate" && p.as_object().is_none_or(|o| o.is_empty()) {
        match app.view_mut().map(|v| v.rotation) {
            Some(angle) => {
                app.ui.dialog = Some(crate::state::Dialog::new("command", json!({"__command": id, "__label": "Rotate View", "angle": angle})));
            }
            None => app.status("no document"),
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
    // Object → Align aligns to what the align buttons align to, unless the call names `to`. With
    // anchors selected (Direct Selection) it sends its params unchanged, and the points align to
    // their own bounds.
    let follows_align_to = id == "object.align"
        && p.as_object().is_some_and(|o| !o.contains_key("to"))
        && app.session.active().is_some_and(|st| st.selection.anchors.is_empty());
    let p = if follows_align_to { crate::panels::align::with_align_to(app, p) } else { p };
    if let Err(e) = app.run(id, p) {
        app.status(e);
    } else if matches!(id, "object.pattern.make" | "object.pattern.edit") {
        app.ui.open_panel = Some("patternOptions".into());
    }
}
