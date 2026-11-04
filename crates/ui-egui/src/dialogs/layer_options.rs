//! Layer Options (double-click a layer row, or ≡ › Options for Selection…) and the same dialog for
//! a new layer or sublayer (≡ › New Layer…, Alt-click Create New Layer). Objects' rows get Name,
//! Show and Lock only.
//!
//! Fields: `ids` (the rows edited; none for a new layer), `mode` (`edit`, `new` or `newSublayer`),
//! `layer` (the rows are layers), `name`, `color` (a preset's name such as "Light Blue", or
//! `#rrggbb`), `template`, `locked`, `visible`, `printable`, `preview`, `dimImages` and
//! `dimPercent`. OK runs `layer.setProps`, `layer.new` or `layer.newSublayer`. For several rows
//! the dialog shows the first row's settings, and OK sends only the ones set in the dialog.

use serde_json::{Value, json};
use vectorcraft_doc::{LAYER_COLORS, LayerColor, Node, NodeId, NodeKind};

use super::swatch_options::{grid, label};
use super::{DialogSpec, form, run_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Layer Options.
pub const KIND: &str = "layerOptions";

/// The field listing the fields set in the dialog, by its controls or `ui.dialog.set`.
const SET: &str = "__set";

/// The field holding, for several rows, the first row's Template, Lock, Dim Images and its
/// percentage as the dialog opened.
const OPENED: &str = "__opened";

/// Records that `field` was set.
fn mark(d: &mut Dialog, field: &str) {
    match d.fields.get_mut(SET) {
        Some(Value::Array(a)) => {
            if !a.iter().any(|k| k == field) {
                a.push(json!(field));
            }
        }
        _ => {
            d.fields.insert(SET.into(), json!([field]));
        }
    }
}

/// Records that `ui.dialog.set` sets `field` to `value`. Template also sets Lock and Dim Images,
/// as its checkbox does.
pub(super) fn note_set(d: &mut Dialog, field: &str, value: &Value) {
    if field.starts_with("__") {
        return;
    }
    match value.as_bool() {
        Some(on) if field == "template" && on != d.bool("template") => set_template(d, on),
        _ => mark(d, field),
    }
}

/// Turns Template on or off. A template is locked and dims its images, and turning Template off
/// unlocks it and turns Dim Images off. For several rows, OK sends Lock and Dim Images only when
/// they are set in the dialog. `layer.setProps` then locks each row that becomes a template and
/// dims it to 50%, unless a percentage is set in the dialog, so the dialog shows 50%; it unlocks
/// each row that stops being a template and stops dimming it.
/// When Template is set back to the first row's value, Lock and Dim Images that were not set in
/// the dialog show the first row's values again.
fn set_template(d: &mut Dialog, on: bool) {
    choose(d, "template", json!(on));
    d.fields.insert("locked".into(), json!(on));
    d.fields.insert("dimImages".into(), json!(on));
    let Some(opened) = d.fields.get(OPENED).cloned() else {
        return;
    };
    if opened.get("template") == Some(&json!(on)) {
        for key in ["locked", "dimImages", "dimPercent"] {
            if let Some(v) = opened.get(key).filter(|_| !is_set(d, key)) {
                d.fields.insert(key.into(), v.clone());
            }
        }
    } else if on && !is_set(d, "dimPercent") {
        d.fields.insert("dimPercent".into(), json!(50));
    }
}

/// Whether `field` was set in the dialog.
fn is_set(d: &Dialog, field: &str) -> bool {
    d.fields.get(SET).and_then(Value::as_array).is_some_and(|a| a.iter().any(|k| k == field))
}

/// Sets `key` to `value` from one of the dialog's controls.
fn choose(d: &mut Dialog, key: &str, value: Value) {
    d.fields.insert(key.into(), value);
    mark(d, key);
}

pub(super) const SPEC: DialogSpec = DialogSpec { heading, body, confirm, min_width: 320.0, ..DialogSpec::FORM };

fn heading(d: &Dialog) -> String {
    if d.bool("layer") || d.str("mode") != "edit" { tl!("Layer Options").into() } else { tl!("Options").into() }
}

/// A layer colour as the dialog shows it: a preset's name, else `#rrggbb`.
fn color_field(c: LayerColor) -> String {
    match c {
        LayerColor::Preset(i) => LAYER_COLORS.get(i as usize).map_or("Light Blue", |(n, _)| *n).to_string(),
        LayerColor::Custom([r, g, b]) => format!("#{r:02x}{g:02x}{b:02x}"),
    }
}

/// The fields for `n` (a layer or any other row).
/// `xml_id`: Preferences › Units › Identify Objects By › XML ID.
fn fields_of(n: &Node, xml_id: bool) -> serde_json::Map<String, Value> {
    let mut f = serde_json::Map::new();
    f.insert("name".into(), json!(n.identify_label(xml_id)));
    f.insert("visible".into(), json!(n.visible));
    f.insert("locked".into(), json!(n.locked));
    f.insert("layer".into(), json!(n.is_layer()));
    if let NodeKind::Layer { color, template, printable, preview, dim_images, .. } = &n.kind {
        f.insert("color".into(), json!(color_field(*color)));
        f.insert("template".into(), json!(template));
        f.insert("printable".into(), json!(printable));
        f.insert("preview".into(), json!(preview));
        f.insert("dimImages".into(), json!(dim_images.is_some()));
        f.insert("dimPercent".into(), json!(dim_images.unwrap_or(50)));
    }
    f
}

/// `ui.layerOptions {ids?|id?}`: open Layer Options for rows (default: the highlighted rows, else
/// the current layer), filled in from the first.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    // Rows given are used as given: a malformed or unknown id fails rather than being skipped or
    // the dialog opening on the highlighted rows instead.
    let row = |v: &Value| -> Result<NodeId, String> {
        let id = NodeId(v.as_u64().ok_or_else(|| format!("object ids are non-negative integers, not {v}"))?);
        st.doc.node(id).map(|_| id).ok_or_else(|| "no such layer or object".to_string())
    };
    let ids: Vec<NodeId> = match (p.get("ids"), p.get("id")) {
        (Some(Value::Array(a)), _) => a.iter().map(row).collect::<Result<_, _>>()?,
        (Some(v), _) => return Err(format!("`ids` must be an array of object ids, not {v}")),
        (None, Some(v)) => vec![row(v)?],
        (None, None) => {
            let rows = st.highlighted_rows();
            if rows.is_empty() { st.current_layer().into_iter().collect() } else { rows }
        }
    };
    let first = ids.first().and_then(|id| st.doc.node(*id)).ok_or("no such layer or object")?;
    let xml_id = app.session.prefs.identify_objects_by == "xmlId";
    let mut f = fields_of(first, xml_id);
    if ids.len() > 1 {
        let opened: serde_json::Map<_, _> =
            ["template", "locked", "dimImages", "dimPercent"].iter().filter_map(|k| Some((k.to_string(), f.get(*k)?.clone()))).collect();
        f.insert(OPENED.into(), Value::Object(opened));
    }
    f.insert("ids".into(), json!(ids.iter().map(|i| i.0).collect::<Vec<_>>()));
    f.insert("mode".into(), json!("edit"));
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields: f });
    Ok(Value::Null)
}

/// `ui.newLayer {sublayer?}`: Layer Options for a new layer (or sublayer), with the name and
/// colour it would get.
pub fn open_new(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    let mut layers = vec![];
    st.doc.walk(|n| {
        if n.is_layer() {
            layers.push(n.display_name());
        }
    });
    let mut i = layers.len() + 1;
    while layers.contains(&format!("Layer {i}")) {
        i += 1;
    }
    let sublayer = p.get("sublayer").and_then(Value::as_bool).unwrap_or(false);
    let color = LayerColor::Preset((layers.len() % LAYER_COLORS.len()) as u8);
    let name = format!("Layer {i}");
    let name = if app.session.prefs.identify_objects_by == "xmlId" { vectorcraft_doc::xml_id::from_object_name(&name) } else { name };
    let fields = json!({
        "mode": if sublayer { "newSublayer" } else { "new" }, "layer": true, "name": name, "color": color_field(color),
        "template": false, "locked": false, "visible": true, "printable": true, "preview": true, "dimImages": false, "dimPercent": 50,
    });
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

fn toggle(ui: &mut egui::Ui, d: &mut Dialog, key: &str, text: &str, enabled: bool) -> bool {
    let v = d.bool(key);
    let changed = widgets::check(ui, text, v, enabled);
    if changed {
        choose(d, key, json!(!v));
    }
    changed
}

/// The size of the colour chip before each name in the Color list.
const CHIP: egui::Vec2 = egui::vec2(16.0, 10.0);

/// The Color list: each preset's name after a chip of its colour, then Custom (no chip, its
/// name aligned with the others). Returns the chosen row; Custom is `LAYER_COLORS.len()`.
fn color_dropdown(ui: &mut egui::Ui, shown: &str) -> Option<usize> {
    let t = crate::theme::Tokens::get(ui.ctx());
    let border = t.input_border;
    let rows = LAYER_COLORS.iter().map(|(n, c)| (*n, Some(*c))).chain([("Custom", None)]);
    widgets::combo(ui, "layer-color", tl!(shown), 140.0, |ui| {
        let mut chosen = None;
        for (i, (name, rgb)) in rows.enumerate() {
            let chip = egui::IdSalt::new(("layer-color-chip", i));
            let r = egui::Button::selectable(name == shown, (egui::Atom::custom(chip, CHIP), tl!(name))).atom_ui(ui);
            if let (Some(rect), Some([cr, cg, cb])) = (r.rect(chip), rgb) {
                ui.painter().rect_filled(rect, t.cr_small(), egui::Color32::from_rgb(cr, cg, cb));
                ui.painter().rect_stroke(rect, t.cr_small(), egui::Stroke::new(1.0, border), egui::StrokeKind::Inside);
            }
            if r.response.clicked() {
                chosen = Some(i);
            }
        }
        chosen
    })
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let layer = d.bool("layer");
    let many = d.fields.get("ids").and_then(Value::as_array).is_some_and(|a| a.len() > 1);
    grid(ui, |ui| {
        if !many {
            label(ui, tl!("Name:"));
            form::text(ui, d, "name", 190.0);
            ui.end_row();
        }
        if layer {
            label(ui, tl!("Color:"));
            ui.horizontal(|ui| {
                let cur = d.str("color");
                let shown = LAYER_COLORS.iter().map(|(n, _)| *n).find(|n| n.eq_ignore_ascii_case(&cur)).unwrap_or("Custom");
                if let Some((n, _)) = color_dropdown(ui, shown).and_then(|i| LAYER_COLORS.get(i)) {
                    choose(d, "color", json!(n));
                }
                let [r, g, b] = LAYER_COLORS
                    .iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case(&cur))
                    .map(|(_, c)| *c)
                    .or_else(|| {
                        vectorcraft_color::Color::from_hex(&cur).map(|c| {
                            let [r, g, b, _] = c.to_rgba8(1.0);
                            [r, g, b]
                        })
                    })
                    .unwrap_or(LAYER_COLORS[0].1);
                let mut rgb = [r, g, b];
                if ui.color_edit_button_srgb(&mut rgb).changed() {
                    choose(d, "color", json!(format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])));
                }
            });
            ui.end_row();
        }
    });
    ui.add_space(8.0);
    if layer {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                let template = d.bool("template");
                if widgets::check(ui, tl!("Template"), template, true) {
                    set_template(d, !template);
                }
                toggle(ui, d, "visible", tl!("Show"), true);
                toggle(ui, d, "preview", tl!("Preview"), true);
                ui.horizontal(|ui| {
                    toggle(ui, d, "dimImages", tl!("Dim Images to:"), true);
                    let v = d.f64("dimPercent", 50.0);
                    let dim = d.bool("dimImages");
                    if let Some(v) = ui
                        .add_enabled_ui(dim, |ui| {
                            crate::widgets::range_field_named(ui, "dimPercent", Some(tl!("Dim Images to:")), v, 0.0..=100.0, "%", 0, 52.0)
                        })
                        .inner
                    {
                        choose(d, "dimPercent", json!(v));
                    }
                });
            });
            ui.add_space(24.0);
            ui.vertical(|ui| {
                let template = d.bool("template");
                toggle(ui, d, "locked", tl!("Lock"), !template);
                toggle(ui, d, "printable", tl!("Print"), !template);
            });
        });
    } else {
        toggle(ui, d, "visible", tl!("Show"), true);
        toggle(ui, d, "locked", tl!("Lock"), true);
    }
    false
}

/// The command and params OK runs. For several rows only the settings set in the dialog are sent,
/// so each row keeps its own values for the rest.
pub(crate) fn params(d: &Dialog) -> (&'static str, Value) {
    let mut p = serde_json::Map::new();
    let many = d.fields.get("ids").and_then(Value::as_array).is_some_and(|a| a.len() > 1);
    if !many {
        p.insert("name".into(), json!(d.str("name")));
    }
    p.insert("visible".into(), json!(d.bool("visible")));
    p.insert("locked".into(), json!(d.bool("locked")));
    if d.bool("layer") {
        p.insert("color".into(), json!(d.str("color")));
        p.insert("template".into(), json!(d.bool("template")));
        p.insert("printable".into(), json!(d.bool("printable")));
        p.insert("preview".into(), json!(d.bool("preview")));
        p.insert("dimImages".into(), if d.bool("dimImages") { json!(d.f64("dimPercent", 50.0).clamp(0.0, 100.0)) } else { json!(false) });
    }
    if many {
        // A percentage set in the dialog is sent whenever Dim Images is on, also when Dim Images
        // was turned on by Template.
        let percent = d.bool("dimImages") && is_set(d, "dimPercent");
        p.retain(|k, _| is_set(d, k) || (k == "dimImages" && percent));
    }
    match d.str("mode").as_str() {
        "new" => ("layer.new", Value::Object(p)),
        "newSublayer" => ("layer.newSublayer", Value::Object(p)),
        _ => {
            p.insert("ids".into(), d.fields.get("ids").cloned().unwrap_or(json!([])));
            ("layer.setProps", Value::Object(p))
        }
    }
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let (cmd, p) = params(d);
    run_and_close(app, cmd, p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app
    }

    fn draw(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(Default::default(), |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
    }

    /// Sets a field as `ui.dialog.set` does.
    fn set(app: &mut VectorcraftApp, k: &str, v: Value) {
        super::super::set_field(app.ui.dialog.as_mut().unwrap(), k.into(), v);
    }

    /// Explicit object ids must not silently disappear or fall back to the highlighted row.
    #[test]
    fn invalid_layer_options_targets_do_not_open_a_partial_dialog() {
        let mut app = app();
        let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        for params in
            [json!({"ids": [id, "invalid"]}), json!({"ids": [id, 999999]}), json!({"ids": "invalid"}), json!({"id": "invalid"}), json!({"id": null})]
        {
            assert!(app.run("ui.layerOptions", params.clone()).is_err(), "accepted {params}");
            assert!(app.ui.dialog.is_none(), "opened partial options for {params}");
        }
        app.run("ui.layerOptions", json!({"ids": [id]})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields["ids"], json!([id]));
    }

    #[test]
    fn layer_options_edit_the_layer_and_ok_is_one_undo_step() {
        let mut app = app();
        let layer = app.session.doc().unwrap().doc.layers[0].id;
        app.run("ui.layerOptions", json!({})).unwrap();
        let d = app.ui.dialog.clone().unwrap();
        assert_eq!((d.kind.as_str(), d.str("name"), d.str("color")), (KIND, "Layer 1".to_string(), "Light Blue".to_string()));
        draw(&mut app);
        set(&mut app, "name", json!("Sky"));
        set(&mut app, "color", json!("#00ff00"));
        set(&mut app, "dimImages", json!(true));
        set(&mut app, "dimPercent", json!(30));
        set(&mut app, "preview", json!(false));
        let undo = app.session.doc().unwrap().history.undo.len();
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let n = app.session.doc().unwrap().doc.node(layer).unwrap().clone();
        assert_eq!(n.display_name(), "Sky");
        assert!(matches!(n.kind, NodeKind::Layer { color: LayerColor::Custom([0, 255, 0]), dim_images: Some(30), preview: false, .. }));
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
    }

    /// Two layers with different options, the first plain and the second green, locked, in
    /// outline view and dimmed to 30%.
    fn two_layers(app: &mut VectorcraftApp) -> (NodeId, NodeId) {
        let first = app.session.doc().unwrap().doc.layers[0].id;
        let p = json!({"color": "#00ff00", "locked": true, "preview": false, "dimImages": 30});
        let second = NodeId(app.session.execute("layer.new", &p).unwrap()["id"].as_u64().unwrap());
        app.run("ui.layerOptions", json!({"ids": [first.0, second.0]})).unwrap();
        draw(app);
        (first, second)
    }

    fn layer(app: &VectorcraftApp, id: NodeId) -> Node {
        app.session.doc().unwrap().doc.node(id).unwrap().clone()
    }

    /// A setting set to the first layer's own value still goes to every layer.
    #[test]
    fn layer_options_for_several_layers_send_only_the_settings_set() {
        let mut app = app();
        let (first, second) = two_layers(&mut app);
        set(&mut app, "printable", json!(false));
        set(&mut app, "color", json!("Light Blue"));
        let (_, p) = params(app.ui.dialog.as_ref().unwrap());
        assert_eq!(p, json!({"printable": false, "color": "Light Blue", "ids": [first.0, second.0]}));
        super::super::confirm(&mut app).unwrap();
        let (a, b) = (layer(&app, first), layer(&app, second));
        assert!(matches!(a.kind, NodeKind::Layer { color: LayerColor::Preset(0), printable: false, preview: true, dim_images: None, .. }));
        assert!(!a.locked);
        let NodeKind::Layer { color, printable, preview, dim_images, .. } = b.kind else { panic!("not a layer") };
        assert_eq!((color, printable, preview, dim_images, b.locked), (LayerColor::Preset(0), false, false, Some(30), true));
    }

    /// Template locks and dims to 50% each layer whose Template it changes; a Dim Images
    /// percentage set as well goes to every layer.
    #[test]
    fn template_for_several_layers_leaves_lock_and_dimming_to_each_layer() {
        let mut app = app();
        let (first, second) = two_layers(&mut app);
        app.run("layer.setProps", json!({"ids": [second.0], "template": true, "dimImages": 30})).unwrap();
        app.run("ui.layerOptions", json!({"ids": [first.0, second.0]})).unwrap();
        set(&mut app, "template", json!(true));
        let (_, p) = params(app.ui.dialog.as_ref().unwrap());
        assert_eq!(p, json!({"template": true, "ids": [first.0, second.0]}));
        super::super::confirm(&mut app).unwrap();
        let (a, b) = (layer(&app, first), layer(&app, second));
        assert!(a.is_template() && a.locked && matches!(a.kind, NodeKind::Layer { dim_images: Some(50), .. }));
        assert!(b.is_template() && b.locked && matches!(b.kind, NodeKind::Layer { dim_images: Some(30), .. }));
        // The first layer is dimmed to 50% now; 50% set again goes to both.
        app.run("ui.layerOptions", json!({"ids": [first.0, second.0]})).unwrap();
        set(&mut app, "dimPercent", json!(50));
        let (_, p) = params(app.ui.dialog.as_ref().unwrap());
        assert_eq!(p, json!({"dimImages": 50.0, "ids": [first.0, second.0]}));
        super::super::confirm(&mut app).unwrap();
        assert!(matches!(layer(&app, second).kind, NodeKind::Layer { dim_images: Some(50), .. }));
        // A percentage set while Dim Images is off is sent once Dim Images is on.
        app.run("layer.setProps", json!({"ids": [first.0], "template": false, "dimImages": false})).unwrap();
        app.run("ui.layerOptions", json!({"ids": [first.0, second.0]})).unwrap();
        set(&mut app, "dimPercent", json!(20));
        assert_eq!(params(app.ui.dialog.as_ref().unwrap()).1, json!({"ids": [first.0, second.0]}));
        set(&mut app, "dimImages", json!(true));
        assert_eq!(params(app.ui.dialog.as_ref().unwrap()).1, json!({"dimImages": 20.0, "ids": [first.0, second.0]}));
    }

    /// A percentage set while Dim Images is off is sent when Template turns Dim Images on.
    #[test]
    fn a_percentage_set_before_template_turns_dimming_on_goes_to_every_layer() {
        let mut app = app();
        let (first, second) = two_layers(&mut app);
        set(&mut app, "dimPercent", json!(20));
        set(&mut app, "template", json!(true));
        let (_, p) = params(app.ui.dialog.as_ref().unwrap());
        assert_eq!(p, json!({"template": true, "dimImages": 20.0, "ids": [first.0, second.0]}));
        super::super::confirm(&mut app).unwrap();
        for id in [first, second] {
            assert!(matches!(layer(&app, id).kind, NodeKind::Layer { template: true, dim_images: Some(20), .. }));
        }
    }

    /// The open dialog of `app` in a test harness, drawn and ready for clicks.
    fn harness(app: VectorcraftApp) -> egui_kittest::Harness<'static, VectorcraftApp> {
        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 700.0)).build_ui_state(
            move |ui, app: &mut VectorcraftApp| {
                if !std::mem::replace(&mut fonts, true) {
                    crate::theme::install_fonts(ui.ctx());
                    return;
                }
                super::super::show(app, ui.ctx());
            },
            app,
        );
        h.run_steps(4);
        h
    }

    /// Clicking Template ticks Lock and Dim Images and leaves them to each layer; clicking Dim
    /// Images off afterwards sends that to every layer.
    #[test]
    fn dim_images_clicked_after_template_goes_to_every_layer() {
        use egui_kittest::kittest::Queryable;
        let mut app = app();
        let (first, second) = two_layers(&mut app);
        let mut h = harness(app);
        h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Template").click();
        h.run_steps(3);
        let d = h.state().ui.dialog.as_ref().unwrap();
        assert!(d.bool("template") && d.bool("locked") && d.bool("dimImages"));
        assert_eq!(params(d).1, json!({"template": true, "ids": [first.0, second.0]}));
        h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Dim Images to:").click();
        h.run_steps(3);
        let (_, p) = params(h.state().ui.dialog.as_ref().unwrap());
        assert_eq!(p, json!({"template": true, "dimImages": false, "ids": [first.0, second.0]}));
        let app = h.state_mut();
        super::super::confirm(app).unwrap();
        for id in [first, second] {
            let n = layer(app, id);
            assert!(n.locked && matches!(n.kind, NodeKind::Layer { template: true, dim_images: None, .. }));
        }
    }

    /// Template set through `ui.dialog.set` locks and dims as its checkbox does, and set back it
    /// shows the first layer's Lock and Dim Images again and sends neither.
    #[test]
    fn template_set_and_set_back_leaves_lock_and_dimming_alone() {
        let mut app = app();
        let (first, second) = two_layers(&mut app);
        app.run("layer.setProps", json!({"ids": [first.0], "locked": true})).unwrap();
        app.run("ui.layerOptions", json!({"ids": [first.0, second.0]})).unwrap();
        set(&mut app, "template", json!(true));
        let d = app.ui.dialog.as_ref().unwrap();
        assert!(d.bool("locked") && d.bool("dimImages"));
        assert_eq!(params(d).1, json!({"template": true, "ids": [first.0, second.0]}));
        set(&mut app, "template", json!(false));
        let d = app.ui.dialog.as_ref().unwrap();
        assert!(d.bool("locked") && !d.bool("dimImages"));
        assert_eq!(params(d).1, json!({"template": false, "ids": [first.0, second.0]}));
        super::super::confirm(&mut app).unwrap();
        assert!(layer(&app, first).locked && layer(&app, second).locked);
        assert!(matches!(layer(&app, second).kind, NodeKind::Layer { dim_images: Some(30), .. }));
    }

    /// Template turned on shows the 50% that `layer.setProps` dims each new template to, and
    /// turned back off shows the first layer's own percentage again.
    #[test]
    fn template_shows_the_percentage_it_dims_to() {
        let mut app = app();
        let (first, second) = two_layers(&mut app);
        app.run("layer.setProps", json!({"ids": [first.0], "dimImages": 30})).unwrap();
        app.run("ui.layerOptions", json!({"ids": [first.0, second.0]})).unwrap();
        set(&mut app, "template", json!(true));
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!(d.f64("dimPercent", 0.0), 50.0);
        set(&mut app, "template", json!(false));
        let d = app.ui.dialog.as_ref().unwrap();
        assert!(d.bool("dimImages"));
        assert_eq!(d.f64("dimPercent", 0.0), 30.0);
        assert_eq!(params(d).1, json!({"template": false, "ids": [first.0, second.0]}));
        set(&mut app, "template", json!(true));
        super::super::confirm(&mut app).unwrap();
        assert!(matches!(layer(&app, first).kind, NodeKind::Layer { template: true, dim_images: Some(50), .. }));
    }

    /// Template set through `ui.dialog.set` for one layer locks it and dims its images, as the
    /// checkbox does.
    #[test]
    fn template_set_for_one_layer_locks_and_dims_it() {
        let mut app = app();
        let id = app.session.doc().unwrap().doc.layers[0].id;
        app.run("ui.layerOptions", json!({"ids": [id.0]})).unwrap();
        set(&mut app, "template", json!(true));
        super::super::confirm(&mut app).unwrap();
        let n = layer(&app, id);
        assert!(n.locked && matches!(n.kind, NodeKind::Layer { template: true, dim_images: Some(50), .. }));
    }

    /// A percentage typed in while Dim Images is on stays set when Template is then clicked on,
    /// since Template leaves Dim Images on.
    #[test]
    fn a_percentage_typed_before_template_goes_to_every_layer() {
        use egui_kittest::kittest::Queryable;
        let mut app = app();
        let (first, second) = two_layers(&mut app);
        app.run("layer.setProps", json!({"ids": [first.0], "dimImages": 30})).unwrap();
        app.run("ui.layerOptions", json!({"ids": [first.0, second.0]})).unwrap();
        set(&mut app, "dimPercent", json!(40));
        let mut h = harness(app);
        h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Template").click();
        h.run_steps(3);
        let (_, p) = params(h.state().ui.dialog.as_ref().unwrap());
        assert_eq!(p, json!({"template": true, "dimImages": 40.0, "ids": [first.0, second.0]}));
        // Template off again shows the first layer's Dim Images, on, and the percentage still
        // goes to every layer.
        h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Template").click();
        h.run_steps(3);
        let (_, p) = params(h.state().ui.dialog.as_ref().unwrap());
        assert_eq!(p, json!({"template": false, "dimImages": 40.0, "ids": [first.0, second.0]}));
    }

    #[test]
    fn new_layer_options_make_a_layer_and_objects_get_name_show_and_lock() {
        let mut app = app();
        app.run("ui.newLayer", json!({})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("name"), "Layer 2");
        set(&mut app, "name", json!("Ink"));
        set(&mut app, "template", json!(true));
        set(&mut app, "locked", json!(true));
        draw(&mut app);
        super::super::confirm(&mut app).unwrap();
        let d = &app.session.doc().unwrap().doc;
        let top = d.layers.last().unwrap();
        assert_eq!(top.display_name(), "Ink");
        assert!(top.is_template() && top.locked);
        // An object's row: Name, Show and Lock.
        let r = app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10}));
        assert!(r.is_ok());
        app.session.execute("layer.setCurrent", &json!({"id": app.session.doc().unwrap().doc.layers[0].id.0})).unwrap();
        let rect = app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
        app.run("ui.layerOptions", json!({"id": rect})).unwrap();
        assert!(!app.ui.dialog.as_ref().unwrap().bool("layer"));
        draw(&mut app);
        set(&mut app, "name", json!("Box"));
        set(&mut app, "locked", json!(true));
        let (cmd, p) = params(app.ui.dialog.as_ref().unwrap());
        assert_eq!(cmd, "layer.setProps");
        assert!(p.get("template").is_none());
        super::super::confirm(&mut app).unwrap();
        let n = app.session.doc().unwrap().doc.node(NodeId(rect)).unwrap().clone();
        assert_eq!((n.display_name(), n.locked), ("Box".to_string(), true));
    }
}
