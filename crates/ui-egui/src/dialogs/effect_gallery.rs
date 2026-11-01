//! Effect → Effect Gallery: browse Photoshop-style raster filters by category, build a stack, and
//! apply it as separate appearance effects in one undo step.

use egui;
use serde_json::{Map, Value, json};
use vectorcraft_effects::{EffectInfo, PIXEL_EFFECTS};

use super::{DialogSpec, effect, form};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// Dialog kind for the Effect Gallery.
pub const KIND: &str = "effectGallery";

/// Raster effect submenus in the gallery (the six Photoshop-style folders it holds).
const CATEGORIES: [(&str, &[&str]); 6] = [
    ("Artistic", &["Effect", "Artistic"]),
    ("Brush Strokes", &["Effect", "Brush Strokes"]),
    ("Distort", &["Effect", "Distort"]),
    ("Sketch", &["Effect", "Sketch"]),
    ("Stylize", &["Effect", "Stylize"]),
    ("Texture", &["Effect", "Texture"]),
];

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Effect Gallery").into(),
    body,
    confirm,
    preview: true,
    min_width: 720.0,
    max_width: Some(820.0),
    inputs: |_| &["browseEffect", "browseParams"],
    ..DialogSpec::FORM
};

#[derive(Clone)]
struct StackEntry {
    effect: String,
    visible: bool,
    params: Map<String, Value>,
}

/// Open the gallery on the current selection.
pub fn open(app: &mut VectorcraftApp, _p: &Value) -> Result<Value, String> {
    if app.session.active().is_none_or(|s| s.selection.is_empty()) {
        return Err(tl!("Select artwork to apply effects").into());
    }
    let selected = effects_in_category(0).first().map(|e| e.id).unwrap_or("");
    app.ui.dialog = Some(Dialog::new(
        KIND,
        json!({ "category": 0, "selected": selected, "stack": [], "stackSelected": 0, "stackFocus": false, "preview": true }),
    ));
    Ok(Value::Null)
}

fn effects_in_category(cat: usize) -> Vec<EffectInfo> {
    let path = CATEGORIES.get(cat).map(|(_, p)| *p).unwrap_or(CATEGORIES[0].1);
    let mut out: Vec<EffectInfo> =
        vectorcraft_effects::effect_catalog().into_iter().filter(|e| e.menu == path && PIXEL_EFFECTS.contains(&e.id)).collect();
    out.sort_by(|a, b| a.label.cmp(b.label));
    out
}

fn default_params(id: &str) -> Map<String, Value> {
    match vectorcraft_effects::effect_info(id).map(|e| e.defaults) {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

fn merged_params(id: &str, stored: &Map<String, Value>) -> Map<String, Value> {
    match vectorcraft_effects::merged_params(id, &Value::Object(stored.clone())) {
        Value::Object(m) => m,
        _ => default_params(id),
    }
}

fn stack_of(d: &Dialog) -> Vec<StackEntry> {
    d.fields
        .get("stack")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let effect = row.get("effect").and_then(Value::as_str)?;
                    let visible = row.get("visible").and_then(Value::as_bool).unwrap_or(true);
                    let params = row
                        .get("params")
                        .and_then(Value::as_object)
                        .cloned()
                        .map(|m| merged_params(effect, &m))
                        .unwrap_or_else(|| default_params(effect));
                    Some(StackEntry { effect: effect.to_string(), visible, params })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn write_stack(d: &mut Dialog, stack: &[StackEntry]) {
    let rows: Vec<Value> = stack.iter().map(|e| json!({ "effect": e.effect, "visible": e.visible, "params": e.params })).collect();
    d.fields.insert("stack".into(), Value::Array(rows));
}

fn category_ix(d: &Dialog) -> usize {
    d.fields.get("category").and_then(Value::as_u64).unwrap_or(0) as usize
}

fn selected_id(d: &Dialog) -> String {
    d.str("selected")
}

fn set_selected(d: &mut Dialog, id: &str) {
    d.fields.insert("selected".into(), json!(id));
}

fn stack_focus(d: &Dialog) -> bool {
    d.fields.get("stackFocus").and_then(Value::as_bool).unwrap_or(false)
}

fn set_stack_focus(d: &mut Dialog, on: bool) {
    d.fields.insert("stackFocus".into(), json!(on));
}

fn browse_params(d: &Dialog) -> Map<String, Value> {
    let effect = d.fields.get("browseEffect").and_then(Value::as_str).unwrap_or("");
    let stored = d.fields.get("browseParams").and_then(Value::as_object).cloned().unwrap_or_default();
    if effect.is_empty() { stored } else { merged_params(effect, &stored) }
}

fn set_browse_params(d: &mut Dialog, effect: &str, params: Map<String, Value>) {
    d.fields.insert("browseEffect".into(), json!(effect));
    d.fields.insert("browseParams".into(), Value::Object(params));
}

/// The effect whose parameters the panel edits, and its values.
fn param_target(d: &Dialog, stack: &[StackEntry], stack_sel: usize, browse: &str) -> (String, Map<String, Value>) {
    if stack_focus(d) {
        let entry = stack.get(stack_sel);
        if let Some(e) = entry {
            return (e.effect.clone(), e.params.clone());
        }
    }
    let id = browse.to_string();
    let params = if d.fields.get("browseEffect").and_then(Value::as_str) == Some(browse) { browse_params(d) } else { default_params(browse) };
    (id, params)
}

fn write_param_target(d: &mut Dialog, stack: &mut [StackEntry], stack_sel: usize, browse: &str, effect: &str, params: Map<String, Value>) {
    if stack_focus(d) {
        if let Some(e) = stack.get_mut(stack_sel)
            && e.effect == effect
        {
            e.params = params;
        }
        return;
    }
    if browse == effect {
        set_browse_params(d, effect, params);
    }
}

/// Reuse the live effect dialog's fields for `effect_id` and `params`.
fn effect_param_fields(app: &mut VectorcraftApp, ui: &mut egui::Ui, effect_id: &str, params: &mut Map<String, Value>) {
    if effect_id.is_empty() {
        return;
    }
    let label =
        vectorcraft_effects::effect_info(effect_id).map(|e| e.label.trim_end_matches('…').to_string()).unwrap_or_else(|| effect_id.to_string());
    let mut fields = params.clone();
    fields.insert("__effect".into(), json!(effect_id));
    fields.insert("__label".into(), json!(label));
    fields.insert("preview".into(), json!(false));
    let mut temp = Dialog::new("effect", Value::Object(fields));
    (effect::SPEC.body)(app, ui, &mut temp);
    if let Value::Object(m) = form::params(&temp) {
        *params = m;
    }
    if let Value::Object(m) = vectorcraft_effects::upgrade_params(effect_id, &Value::Object(params.clone())) {
        *params = m;
    }
}

fn stack_batch_commands(stack: &[StackEntry]) -> Vec<Value> {
    let mut commands = Vec::with_capacity(stack.len() * 2);
    for (i, entry) in stack.iter().enumerate() {
        let params = Value::Object(entry.params.clone());
        commands.push(json!({ "command": "effect.apply", "params": { "effect": entry.effect, "params": params } }));
        if !entry.visible {
            commands.push(json!({ "command": "effect.setParams", "params": { "index": i, "visible": false } }));
        }
    }
    commands
}

fn stack_batch_params(stack: &[StackEntry]) -> Value {
    json!({ "label": "Effect Gallery", "commands": stack_batch_commands(stack) })
}

/// Live canvas preview of the stack (`command.batch` would settle the dialog interaction each frame).
fn preview_stack_on_canvas(app: &mut VectorcraftApp, stack: &[StackEntry]) {
    if app.session.in_interaction() {
        let _ = app.session.cancel_interaction();
    }
    let _ = app.session.begin_interaction("Effect Gallery");
    for (i, entry) in stack.iter().enumerate() {
        let params = Value::Object(entry.params.clone());
        if app.run("effect.apply", json!({ "effect": entry.effect, "params": params })).is_err() {
            let _ = app.session.cancel_interaction();
            return;
        }
        if !entry.visible && app.run("effect.setParams", json!({ "index": i, "visible": false })).is_err() {
            let _ = app.session.cancel_interaction();
            return;
        }
    }
}

fn stack_preview_row(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog, stack: &[StackEntry]) {
    const LAST: &str = super::form::PREVIEWED;
    ui.add_space(8.0);
    let mut on = d.bool("preview");
    if widgets::check(ui, tl!("Preview"), on, true) {
        on = !on;
        d.fields.insert("preview".into(), json!(on));
    }
    if !on {
        if d.fields.remove(LAST).is_some() {
            let _ = app.session.cancel_interaction();
        }
        return;
    }
    if stack.is_empty() {
        if d.fields.remove(LAST).is_some() {
            let _ = app.session.cancel_interaction();
        }
        return;
    }
    let batch = stack_batch_params(stack);
    if d.fields.get(LAST) != Some(&batch) || !app.session.in_interaction() {
        preview_stack_on_canvas(app, stack);
        d.fields.insert(LAST.into(), batch);
    }
}

fn effect_label(id: &str) -> String {
    vectorcraft_effects::effect_info(id).map(|e| tl!(e.label).trim_end_matches('…').to_string()).unwrap_or_else(|| id.to_string())
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let cat = category_ix(d);
    let effects = effects_in_category(cat);
    let mut sel = selected_id(d);
    if !effects.iter().any(|e| e.id == sel) {
        sel = effects.first().map(|e| e.id.to_string()).unwrap_or_default();
        set_selected(d, &sel);
    }

    let mut stack = stack_of(d);
    let mut stack_sel = d.fields.get("stackSelected").and_then(Value::as_u64).unwrap_or(0) as usize;
    if stack_sel >= stack.len() && !stack.is_empty() {
        stack_sel = stack.len() - 1;
    }
    let (param_effect, mut param_values) = param_target(d, &stack, stack_sel, &sel);
    if !param_effect.is_empty() {
        widgets::subheader(ui, tl!("Settings"));
        egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
            effect_param_fields(app, ui, &param_effect, &mut param_values);
        });
        write_param_target(d, &mut stack, stack_sel, &sel, &param_effect, param_values);
    }

    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            widgets::subheader(ui, tl!("Categories"));
            ui.set_width(120.0);
            for (i, (name, _)) in CATEGORIES.iter().enumerate() {
                let count = effects_in_category(i).len();
                if count == 0 {
                    continue;
                }
                let label = tl!(name);
                if ui.add(egui::Button::selectable(cat == i, label)).clicked() {
                    d.fields.insert("category".into(), json!(i));
                    let first = effects_in_category(i).first().map(|e| e.id).unwrap_or("");
                    set_selected(d, first);
                    set_stack_focus(d, false);
                }
            }
        });
        ui.add_space(8.0);
        ui.vertical(|ui| {
            widgets::subheader(ui, tl!("Filters"));
            ui.set_width(220.0);
            egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                for e in &effects {
                    let picked = e.id == sel;
                    let name = tl!(e.label).trim_end_matches('…').to_string();
                    let resp = ui.add(egui::Button::selectable(picked, name));
                    if resp.clicked() {
                        set_selected(d, e.id);
                        set_stack_focus(d, false);
                    }
                    if resp.double_clicked() {
                        set_selected(d, e.id);
                        set_stack_focus(d, false);
                        add_selected(d, e.id);
                        stack = stack_of(d);
                    }
                }
            });
            if widgets::flat_button(ui, tl!("Add to Stack"), 120.0).clicked() && !sel.is_empty() {
                add_selected(d, &sel);
                stack = stack_of(d);
            }
        });
    });

    ui.add_space(10.0);
    widgets::subheader(ui, tl!("Applied Filters"));
    let mut stack_sel = d.fields.get("stackSelected").and_then(Value::as_u64).unwrap_or(0) as usize;
    if stack_sel >= stack.len() && !stack.is_empty() {
        stack_sel = stack.len() - 1;
    }
    enum StackOp {
        Toggle(usize),
        Up(usize),
        Down(usize),
        Remove(usize),
        Select(usize),
    }
    let mut op = None;
    let stack_len = stack.len();
    egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
        if stack.is_empty() {
            ui.label(egui::RichText::new(tl!("Add filters from the list above")).color(t.text_dim));
            return;
        }
        for (i, entry) in stack.iter().enumerate().take(stack_len) {
            ui.horizontal(|ui| {
                let eye = if entry.visible { "eye" } else { "eye-off" };
                if widgets::icon_button(ui, eye, tl!("Click to toggle visibility"), false, 22.0).clicked() {
                    op = Some(StackOp::Toggle(i));
                }
                let name = effect_label(&entry.effect);
                if ui.add(egui::Button::selectable(stack_sel == i, name)).clicked() {
                    op = Some(StackOp::Select(i));
                }
                if widgets::icon_button_enabled(ui, "dc-arrow-up", tl!("Move Up"), false, i > 0, 22.0).clicked() {
                    op = Some(StackOp::Up(i));
                }
                if widgets::icon_button_enabled(ui, "dc-arrow-down", tl!("Move Down"), false, i + 1 < stack_len, 22.0).clicked() {
                    op = Some(StackOp::Down(i));
                }
                if widgets::flat_button(ui, tl!("Remove"), 64.0).clicked() {
                    op = Some(StackOp::Remove(i));
                }
            });
        }
    });
    match op {
        Some(StackOp::Toggle(i)) if stack.get(i).is_some() => stack[i].visible = !stack[i].visible,
        Some(StackOp::Select(i)) => {
            stack_sel = i;
            set_stack_focus(d, true);
        }
        Some(StackOp::Up(i)) if i > 0 => {
            stack.swap(i, i - 1);
            stack_sel = i - 1;
        }
        Some(StackOp::Down(i)) if i + 1 < stack.len() => {
            stack.swap(i, i + 1);
            stack_sel = i + 1;
        }
        Some(StackOp::Remove(i)) if i < stack.len() => {
            stack.remove(i);
            if stack_sel >= stack.len() && !stack.is_empty() {
                stack_sel = stack.len() - 1;
            }
        }
        _ => {}
    }
    write_stack(d, &stack);
    d.fields.insert("stackSelected".into(), json!(stack_sel));
    stack_preview_row(app, ui, d, &stack);
    false
}

fn add_selected(d: &mut Dialog, id: &str) {
    let params =
        if !stack_focus(d) && d.fields.get("browseEffect").and_then(Value::as_str) == Some(id) { browse_params(d) } else { default_params(id) };
    let mut stack = stack_of(d);
    stack.push(StackEntry { effect: id.to_string(), visible: true, params });
    write_stack(d, &stack);
    d.fields.insert("stackSelected".into(), json!(stack.len().saturating_sub(1)));
    set_stack_focus(d, true);
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let stack = stack_of(d);
    if stack.is_empty() {
        if app.session.in_interaction() {
            let _ = app.session.cancel_interaction();
        }
        app.ui.dialog = None;
        return Ok(Value::Null);
    }
    let batch = stack_batch_params(&stack);
    if d.bool("preview") && app.session.in_interaction() {
        return super::form::commit_preview(app, "command.batch", batch);
    }
    let r = app.run("command.batch", batch);
    if r.is_ok() {
        app.ui.dialog = None;
    }
    r
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_engine::Session;

    use crate::VectorcraftApp;

    fn frame(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, Default::default());
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 900.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| crate::dialogs::show(app, ui.ctx()));
        out.textures_delta.clear();
    }

    #[test]
    fn stack_preview_is_one_interaction() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
        app.run("ui.effectGalleryDialog", json!({})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("stack".into(), json!([{ "effect": "artistic.cutout", "visible": true, "params": { "levels": 3 } }]));
        frame(&mut app);
        assert!(app.session.in_interaction(), "the stack previews on the canvas");
        crate::dialogs::cancel(&mut app);
        assert!(!app.session.in_interaction());
        assert!(app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects.is_empty());
    }

    #[test]
    fn stack_ok_is_one_undo_step() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
        app.run("ui.effectGalleryDialog", json!({})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(super::KIND));
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert(
            "stack".into(),
            json!([
                { "effect": "artistic.cutout", "visible": true, "params": { "levels": 3 } },
                { "effect": "stylize.glowingEdges", "visible": false, "params": { "edgeWidth": 2 } }
            ]),
        );
        frame(&mut app);
        let steps = app.session.active().unwrap().history.undo.len();
        crate::dialogs::confirm(&mut app).unwrap();
        let st = app.session.active().unwrap();
        let n = st.doc.node(vectorcraft_doc::NodeId(id)).unwrap();
        assert_eq!(n.appearance.effects.len(), 2);
        assert_eq!(n.appearance.effects[0].id, "artistic.cutout");
        assert_eq!(n.appearance.effects[0].params.get("levels").and_then(|v| v.as_i64()), Some(3));
        assert_eq!(n.appearance.effects[1].id, "stylize.glowingEdges");
        assert!(!n.appearance.effects[1].visible);
        assert_eq!(st.history.undo.len(), steps + 1);
    }
}
