//! Object → Envelope Distort's dialogs, previewed live on the canvas; OK keeps the preview as one
//! undo step:
//!
//! - **Warp Options** (`envelopeWarp`): Make with Warp, or Reset with Warp for a selected envelope
//!   (`reset`). Fields `style` (one of the 15 warp styles), `horizontal` (false: Vertical),
//!   `bend`, `h` and `v` (%, -100..100).
//! - **Envelope Mesh** (`envelopeMesh`): Make with Mesh, or Reset with Mesh (`reset`, with
//!   `maintainShape`). Fields `rows`, `cols` (1..50).
//! - **Envelope Options** (`envelopeOptions`): the selected envelopes' options, or with none
//!   selected the options new envelopes get (no preview then). Fields `antiAlias`,
//!   `preserveShape` (`clippingMask` | `transparency`), `fidelity` (0..100),
//!   `distortAppearance`, `distortLinearGradients`, `distortPatternFills`.
//!
//! The Control bar's envelope controls ([`control_bar`]) live here too.
//!
//! Every dialog also has `preview`. They open with the selected envelope's values
//! (`object.envelope.info`); the menu items and shortcuts of Make with Warp and Make with Mesh
//! open the Reset variant while an envelope is selected, as the reference app's do.

use serde_json::{Value, json};
use vectorcraft_doc::{EnvelopeKind, NodeKind};
use vectorcraft_render::effects::WARP_STYLES;

use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kinds.
pub const WARP: &str = "envelopeWarp";
pub const MESH: &str = "envelopeMesh";
pub const OPTIONS: &str = "envelopeOptions";

const MAKE_WARP: &str = "object.envelope.makeWithWarp";
const RESET_WARP: &str = "object.envelope.resetWithWarp";
const MAKE_MESH: &str = "object.envelope.makeWithMesh";
const RESET_MESH: &str = "object.envelope.resetWithMesh";
const OPTIONS_CMD: &str = "object.envelope.options";

/// Width of the label column.
const LABEL_W: f32 = 84.0;

pub(super) const SPEC: DialogSpec = DialogSpec { heading, body, confirm, preview: true, min_width: 340.0, ..DialogSpec::FORM };

fn heading(d: &Dialog) -> String {
    match d.kind.as_str() {
        WARP => tl!("Warp Options"),
        MESH if d.bool("reset") => tl!("Reset Envelope Mesh"),
        MESH => tl!("Envelope Mesh"),
        _ => tl!("Envelope Options"),
    }
    .into()
}

/// Does the menu item of command `id` open one of these dialogs?
pub fn opens(id: &str) -> bool {
    matches!(id, MAKE_WARP | RESET_WARP | MAKE_MESH | RESET_MESH | OPTIONS_CMD)
}

/// Open the dialog command `id`'s menu item opens, with the selected envelope's values.
pub fn open(app: &mut VectorcraftApp, id: &str) -> Result<Value, String> {
    let info = app.session.execute("object.envelope.info", &json!({})).map_err(|e| e.to_string())?;
    let envelope = !info["id"].is_null();
    if matches!(id, RESET_WARP | RESET_MESH) && !envelope {
        return Err("select an envelope".into());
    }
    let num = |k: &str, d: f64| info[k].as_f64().unwrap_or(d);
    let (kind, fields) = match id {
        MAKE_WARP | RESET_WARP => {
            let warp = info["type"] == "warp";
            let own = |k: &str, d: f64| if warp { num(k, d) } else { d };
            let style = if warp { info["style"].clone() } else { json!("arc") };
            let horizontal = !warp || info["horizontal"] != false;
            (
                WARP,
                json!({"reset": envelope, "style": style, "horizontal": horizontal, "bend": own("bend", 50.0), "h": own("h", 0.0), "v": own("v", 0.0), "preview": true}),
            )
        }
        MAKE_MESH | RESET_MESH => {
            let mesh = info["type"] == "mesh";
            let own = |k: &str| if mesh { num(k, 4.0) } else { 4.0 };
            (MESH, json!({"reset": envelope, "rows": own("rows"), "cols": own("cols"), "maintainShape": true, "preview": true}))
        }
        OPTIONS_CMD => {
            let mut f = json!({"__envelope": envelope, "preview": envelope});
            for k in ["antiAlias", "preserveShape", "fidelity", "distortAppearance", "distortLinearGradients", "distortPatternFills"] {
                f[k] = info[k].clone();
            }
            (OPTIONS, f)
        }
        _ => return Err(format!("`{id}` has no envelope dialog")),
    };
    app.ui.dialog = Some(Dialog::new(kind, fields));
    Ok(Value::Null)
}

/// The command OK runs, its parameters and the preview's undo label.
fn command(d: &Dialog) -> (&'static str, Value, &'static str) {
    let reset = d.bool("reset");
    match d.kind.as_str() {
        WARP => {
            let p = json!({"style": d.str("style"), "horizontal": d.bool("horizontal"), "bend": d.f64("bend", 50.0), "h": d.f64("h", 0.0), "v": d.f64("v", 0.0)});
            if reset { (RESET_WARP, p, "Reset with Warp") } else { (MAKE_WARP, p, "Make Envelope") }
        }
        MESH => {
            let p = json!({"rows": d.f64("rows", 4.0), "cols": d.f64("cols", 4.0)});
            if reset {
                let mut p = p;
                p["maintainShape"] = json!(d.bool("maintainShape"));
                (RESET_MESH, p, "Reset with Mesh")
            } else {
                (MAKE_MESH, p, "Make Envelope")
            }
        }
        _ => {
            let mut p = json!({"fidelity": d.f64("fidelity", 50.0), "preserveShape": d.str("preserveShape")});
            for k in ["antiAlias", "distortAppearance", "distortLinearGradients", "distortPatternFills"] {
                p[k] = json!(d.bool(k));
            }
            (OPTIONS_CMD, p, "Envelope Options")
        }
    }
}

fn set(d: &mut Dialog, key: &str, v: Value) {
    d.fields.insert(key.into(), v);
}

/// A −100..100 % slider row.
fn percent(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str) {
    let rail = Tokens::get(ui.ctx()).input_border;
    form::slider_w(ui, d, (key, label, LABEL_W), -100.0..=100.0, "%", &move |_| rail);
}

/// A whole number field row clamped to 1..50.
fn count(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str) {
    widgets::label_row(ui, label, LABEL_W, |ui| {
        if let Some(n) = widgets::range_field(ui, ("env-count", key), d.f64(key, 4.0), 1.0..=50.0, "", 0, 60.0) {
            set(d, key, json!(n));
        }
    });
}

fn warp_body(ui: &mut egui::Ui, d: &mut Dialog) {
    let styles: Vec<(&str, &str)> = WARP_STYLES.iter().map(|(id, label)| (*id, label.trim_end_matches('…'))).collect();
    form::choice(ui, d, "style", tl!("Style:"), (LABEL_W, 160.0), &styles);
    let horizontal = d.bool("horizontal");
    widgets::label_row(ui, "", LABEL_W, |ui| {
        for (h, label) in [(true, tl!("Horizontal")), (false, tl!("Vertical"))] {
            if widgets::radio(ui, label, horizontal == h, true) {
                set(d, "horizontal", json!(h));
            }
            ui.add_space(10.0);
        }
    });
    percent(ui, d, "bend", tl!("Bend:"));
    ui.add_space(6.0);
    widgets::subheader(ui, tl!("Distortion"));
    percent(ui, d, "h", tl!("Horizontal:"));
    percent(ui, d, "v", tl!("Vertical:"));
}

fn mesh_body(ui: &mut egui::Ui, d: &mut Dialog) {
    count(ui, d, "rows", tl!("Rows:"));
    count(ui, d, "cols", tl!("Columns:"));
    if d.bool("reset") {
        ui.add_space(4.0);
        form::check(ui, d, "maintainShape", tl!("Maintain Envelope Shape"));
    }
}

fn options_body(ui: &mut egui::Ui, d: &mut Dialog) {
    widgets::subheader(ui, tl!("Rasters"));
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        form::check(ui, d, "antiAlias", tl!("Anti-Alias"));
    });
    ui.add_space(4.0);
    widgets::subheader(ui, tl!("Preserve Shape Using:"));
    let clip = d.str("preserveShape") != "transparency";
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        for (value, label) in [("clippingMask", tl!("Clipping Mask")), ("transparency", tl!("Transparency"))] {
            if widgets::radio(ui, label, clip == (value == "clippingMask"), true) {
                set(d, "preserveShape", json!(value));
            }
            ui.add_space(10.0);
        }
    });
    ui.add_space(4.0);
    let rail = Tokens::get(ui.ctx()).input_border;
    form::slider_w(ui, d, ("fidelity", tl!("Fidelity:"), LABEL_W), 0.0..=100.0, "", &move |_| rail);
    ui.add_space(4.0);
    form::check(ui, d, "distortAppearance", tl!("Distort Appearance"));
    // Gradients and patterns bend only with the appearance.
    let appearance = d.bool("distortAppearance");
    for (key, label) in [("distortLinearGradients", tl!("Distort Linear Gradients")), ("distortPatternFills", tl!("Distort Pattern Fills"))] {
        let on = d.bool(key);
        ui.horizontal(|ui| {
            ui.add_space(20.0);
            if widgets::check(ui, label, on, appearance) {
                set(d, key, json!(!on));
            }
        });
    }
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    match d.kind.as_str() {
        WARP => warp_body(ui, d),
        MESH => mesh_body(ui, d),
        _ => options_body(ui, d),
    }
    // With no envelope selected, Envelope Options sets the defaults: nothing to preview.
    if d.kind != OPTIONS || d.bool("__envelope") {
        let (cmd, p, label) = command(d);
        form::preview(app, ui, d, label, cmd, p);
    }
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let (cmd, p, _) = command(d);
    form::commit_preview(app, cmd, p)
}

/// What the Control bar shows for the selected envelope.
#[derive(Clone, Debug, PartialEq)]
enum Bar {
    Warp { style: String, bend: f64, h: f64, v: f64, horizontal: bool },
    Mesh { rows: u32, cols: u32 },
    TopObject,
}

/// The selected envelope (or the one whose contents are selected), whether its contents are being
/// edited, and its settings.
fn bar_of(app: &VectorcraftApp) -> Option<(bool, Bar)> {
    let st = app.session.active()?;
    let env = st.selection.objects.first().and_then(|id| vectorcraft_doc::live::envelope_of(&st.doc, *id))?;
    let NodeKind::Envelope { kind, editing, .. } = &env.kind else { return None };
    let bar = match &**kind {
        EnvelopeKind::Warp { style, bend, h, v, horizontal } => {
            Bar::Warp { style: style.clone(), bend: *bend, h: *h, v: *v, horizontal: *horizontal }
        }
        EnvelopeKind::Mesh { rows, cols, .. } => Bar::Mesh { rows: *rows, cols: *cols },
        EnvelopeKind::TopObject { .. } => Bar::TopObject,
    };
    Some((*editing, bar))
}

/// The Control bar's envelope controls (as in the reference app) while an envelope, or content
/// whose envelope is being edited, is selected: Edit Envelope / Edit Contents; a warp's style,
/// orientation, bend and distortions, or a mesh's rows and columns; Reset Envelope Shape; and
/// Envelope Options.
pub fn control_bar(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    let Some((editing, bar)) = bar_of(app) else { return };
    let mut run: Option<(&str, Value)> = None;
    ui.separator();
    for (contents, icon, tip) in [(false, "dc-mesh", tl!("Edit Envelope")), (true, "shapes", tl!("Edit Contents"))] {
        if widgets::icon_button(ui, icon, tip, editing == contents, 24.0).clicked() && editing != contents {
            run = Some(("object.envelope.editContents", json!({ "editing": contents })));
        }
    }
    ui.add_space(4.0);
    let field = |ui: &mut egui::Ui, key: &str, label: &str, value: f64, suffix: &str| {
        widgets::dim_label(ui, label);
        widgets::plain_field(ui, ("cb-envelope", key), value, suffix, 0, 46.0)
    };
    match &bar {
        Bar::Warp { style, bend, h, v, horizontal } => {
            let styles: Vec<&str> = WARP_STYLES.iter().map(|(_, l)| l.trim_end_matches('…')).collect();
            let current = WARP_STYLES.iter().find(|(id, _)| id == style).map_or(style.as_str(), |(_, l)| l.trim_end_matches('…'));
            widgets::dim_label(ui, tl!("Style:"));
            if let Some((id, _)) = widgets::dropdown(ui, "cb-envelope-style", current, &styles, 110.0).and_then(|i| WARP_STYLES.get(i)) {
                run = Some((OPTIONS_CMD, json!({ "style": id })));
            }
            for (h, label) in [(true, tl!("Horizontal")), (false, tl!("Vertical"))] {
                if widgets::radio(ui, label, *horizontal == h, true) {
                    run = Some((OPTIONS_CMD, json!({ "horizontal": h })));
                }
            }
            for (key, label, value) in [("bend", tl!("Bend:"), *bend), ("h", tl!("H:"), *h), ("v", tl!("V:"), *v)] {
                if let Some(n) = field(ui, key, label, value, "%") {
                    run = Some((OPTIONS_CMD, json!({ key: n.round().clamp(-100.0, 100.0) })));
                }
            }
        }
        Bar::Mesh { rows, cols } => {
            for (key, label, value) in [("rows", tl!("Rows:"), *rows), ("cols", tl!("Columns:"), *cols)] {
                if let Some(n) = field(ui, key, label, f64::from(value), "") {
                    run = Some((RESET_MESH, json!({ key: n.round().clamp(1.0, 50.0), "maintainShape": true })));
                }
            }
        }
        Bar::TopObject => {}
    }
    // Reset Envelope Shape: an unbent warp, a flat mesh.
    let reset = match bar {
        Bar::Warp { .. } => Some((OPTIONS_CMD, json!({"bend": 0, "h": 0, "v": 0}))),
        Bar::Mesh { .. } => Some((RESET_MESH, json!({"maintainShape": false}))),
        Bar::TopObject => None,
    };
    if let Some(r) = reset
        && widgets::flat_button(ui, tl!("Reset"), 50.0).on_hover_text(tl!("Reset Envelope Shape")).clicked()
    {
        run = Some(r);
    }
    if widgets::icon_button(ui, "dc-options", tl!("Envelope Options"), false, 24.0).clicked() {
        crate::menus::invoke(app, OPTIONS_CMD, json!({}));
    }
    if let Some((cmd, p)) = run
        && let Err(e) = app.run(cmd, p)
    {
        app.status(e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_doc::live::EnvelopeKind;
    use vectorcraft_doc::{NodeId, NodeKind};
    use vectorcraft_engine::Session;

    fn app_with_rect() -> (VectorcraftApp, NodeId) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 200, "height": 100})).unwrap()["id"].as_u64().unwrap();
        app.run("select.all", json!({})).unwrap();
        (app, NodeId(id))
    }

    fn frame(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
    }

    fn envelope(app: &VectorcraftApp) -> (NodeId, vectorcraft_doc::Node) {
        let st = app.session.doc().unwrap();
        let id = st.selection.objects[0];
        (id, st.doc.node(id).unwrap().clone())
    }

    #[test]
    fn warp_dialog_previews_and_ok_keeps_one_step() {
        let (mut app, rect) = app_with_rect();
        crate::menus::invoke(&mut app, MAKE_WARP, json!({}));
        assert_eq!(app.ui.dialog.as_ref().map(|d| (d.kind.as_str(), d.fields["reset"].clone())), Some((WARP, json!(false))));
        frame(&mut app);
        // The preview made the envelope already.
        assert!(matches!(envelope(&app).1.kind, NodeKind::Envelope { .. }));
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("style".into(), json!("flag"));
        d.fields.insert("horizontal".into(), json!(false));
        d.fields.insert("bend".into(), json!(-30));
        frame(&mut app);
        let undo = app.session.doc().unwrap().history.undo.len();
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
        let (_, n) = envelope(&app);
        match &n.kind {
            NodeKind::Envelope { kind, content, .. } => {
                let EnvelopeKind::Warp { style, bend, horizontal, .. } = &**kind else { panic!("{kind:?}") };
                assert_eq!((style.as_str(), *bend, *horizontal), ("flag", -30.0, false));
                assert_eq!(content[0].id, rect);
            }
            k => panic!("{k:?}"),
        }
        // With the envelope selected, the same item resets it, starting from its values.
        crate::menus::invoke(&mut app, MAKE_WARP, json!({}));
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.fields["reset"].clone(), d.fields["style"].clone(), d.fields["bend"].clone()), (json!(true), json!("flag"), json!(-30.0)));
        crate::dialogs::cancel(&mut app);
        assert!(
            matches!(envelope(&app).1.kind, NodeKind::Envelope { ref kind, .. } if matches!(**kind, EnvelopeKind::Warp { .. })),
            "cancel rolls back"
        );
    }

    #[test]
    fn reset_items_take_the_make_items_place_while_an_envelope_is_selected() {
        let labels = |app: &VectorcraftApp| -> Vec<String> {
            crate::menus::menu_entries(app)
                .into_iter()
                .filter(|e| e.path.last().map(String::as_str) == Some("Envelope Distort"))
                .map(|e| e.label)
                .collect()
        };
        let (mut app, _) = app_with_rect();
        let before = labels(&app);
        assert!(before.contains(&"Make with Warp…".to_string()) && !before.contains(&"Reset with Warp…".to_string()), "{before:?}");
        app.run(MAKE_MESH, json!({})).unwrap();
        let after = labels(&app);
        assert!(after.contains(&"Reset with Warp…".to_string()) && after.contains(&"Reset with Mesh…".to_string()), "{after:?}");
        assert!(!after.contains(&"Make with Warp…".to_string()) && !after.contains(&"Make with Mesh…".to_string()), "{after:?}");
        assert!(app.run("ui.menuDialog", json!({"command": RESET_WARP})).is_ok());
        app.run("select.none", json!({})).unwrap();
        assert!(app.run("ui.menuDialog", json!({"command": RESET_WARP})).is_err(), "nothing to reset");
    }

    #[test]
    fn reset_mesh_dialog_switches_the_kind_and_cancel_rolls_back() {
        let (mut app, _) = app_with_rect();
        app.run(MAKE_WARP, json!({"style": "arc", "bend": 50})).unwrap();
        app.run("ui.menuDialog", json!({"command": RESET_MESH})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(MESH));
        frame(&mut app);
        assert!(
            matches!(envelope(&app).1.kind, NodeKind::Envelope { ref kind, .. } if matches!(**kind, EnvelopeKind::Mesh { rows: 4, cols: 4, .. })),
            "previewed"
        );
        crate::dialogs::cancel(&mut app);
        assert!(matches!(envelope(&app).1.kind, NodeKind::Envelope { ref kind, .. } if matches!(**kind, EnvelopeKind::Warp { .. })));
        app.run("ui.menuDialog", json!({"command": RESET_MESH})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("rows".into(), json!(2));
        super::super::confirm(&mut app).unwrap();
        assert!(
            matches!(envelope(&app).1.kind, NodeKind::Envelope { ref kind, .. } if matches!(**kind, EnvelopeKind::Mesh { rows: 2, cols: 4, .. }))
        );
    }

    #[test]
    fn options_dialog_starts_from_the_envelope_and_without_one_sets_the_defaults() {
        let (mut app, _) = app_with_rect();
        app.run(MAKE_WARP, json!({})).unwrap();
        app.run("object.envelope.options", json!({"fidelity": 80, "distortPatternFills": true})).unwrap();
        crate::menus::invoke(&mut app, OPTIONS_CMD, json!({}));
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!((d.fields["fidelity"].clone(), d.fields["distortPatternFills"].clone()), (json!(80.0), json!(true)));
        d.fields.insert("preserveShape".into(), json!("transparency"));
        frame(&mut app);
        super::super::confirm(&mut app).unwrap();
        let info = app.session.execute("object.envelope.info", &json!({})).unwrap();
        assert_eq!((info["preserveShape"].clone(), info["fidelity"].clone()), (json!("transparency"), json!(80.0)));
        // Nothing selected: the defaults for new envelopes, no preview.
        app.run("select.none", json!({})).unwrap();
        crate::menus::invoke(&mut app, OPTIONS_CMD, json!({}));
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!(d.fields["preview"], json!(false));
        d.fields.insert("distortAppearance".into(), json!(false));
        frame(&mut app);
        assert!(!app.session.in_interaction());
        super::super::confirm(&mut app).unwrap();
        assert_eq!(app.session.execute("object.envelope.info", &json!({})).unwrap()["distortAppearance"], json!(false));
    }

    #[test]
    fn edit_contents_reads_edit_envelope_while_editing() {
        let (mut app, _) = app_with_rect();
        app.run(MAKE_WARP, json!({})).unwrap();
        let label = |app: &VectorcraftApp| crate::menus::dynamic_label(app, "object.envelope.editContents", "Edit Contents");
        assert_eq!(label(&app), "Edit Contents");
        app.run("object.envelope.editContents", json!({})).unwrap();
        assert_eq!(label(&app), "Edit Envelope");
        assert!(crate::menus::shortcut_of("object.envelope.editContents").is_none(), "its reference shortcut is Paste in Place's here");
    }

    /// One Control bar frame with `events`; the rectangles of its 24 pt icon buttons, left to right.
    fn bar_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<egui::Rect> {
        let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1800.0, 600.0)));
        let mut out = ctx.run_ui(egui::RawInput { events, screen_rect, ..Default::default() }, |ui| crate::chrome::control_bar(app, ui));
        out.textures_delta.clear();
        let mut r: Vec<egui::Rect> = ctx.viewport(|vp| {
            vp.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).filter(|w| w.rect.size() == egui::vec2(24.0, 24.0)).map(|w| w.rect).collect()
        });
        r.sort_by(|a, b| a.left().total_cmp(&b.left()));
        r
    }

    #[test]
    fn the_control_bar_switches_to_edit_contents_and_back() {
        let (mut app, rect) = app_with_rect();
        app.run(MAKE_WARP, json!({"style": "arch"})).unwrap();
        assert!(bar_of(&app).is_some_and(|(editing, bar)| !editing && matches!(bar, Bar::Warp { ref style, .. } if style == "arch")));
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let click = |app: &mut VectorcraftApp, at: egui::Pos2| {
            let button =
                |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            bar_frame(app, &ctx, vec![egui::Event::PointerMoved(at), button(true)]);
            bar_frame(app, &ctx, vec![button(false)]);
        };
        bar_frame(&mut app, &ctx, vec![]);
        let buttons = bar_frame(&mut app, &ctx, vec![]);
        // Edit Envelope, Edit Contents come first.
        click(&mut app, buttons[1].center());
        assert_eq!(app.session.doc().unwrap().selection.objects, vec![rect], "Edit Contents selects the content");
        assert!(bar_of(&app).is_some_and(|(editing, _)| editing), "the content keeps the envelope's controls");
        let buttons = bar_frame(&mut app, &ctx, vec![]);
        click(&mut app, buttons[0].center());
        assert!(bar_of(&app).is_some_and(|(editing, _)| !editing));
        // A mesh envelope shows its rows and columns instead.
        app.run(RESET_MESH, json!({"rows": 3, "cols": 2})).unwrap();
        assert_eq!(bar_of(&app).map(|b| b.1), Some(Bar::Mesh { rows: 3, cols: 2 }));
        bar_frame(&mut app, &ctx, vec![]);
    }
}
