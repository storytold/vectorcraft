//! Object → Envelope Distort's dialogs, previewed live on the canvas; OK keeps the preview as one
//! undo step:
//!
//! - **Warp Options** (`envelopeWarp`): Make with Warp, or Reset with Warp for a selected envelope
//!   (`reset`). Fields `style` (one of the 15 warp styles), `horizontal` (`horizontal` | `vertical`),
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
//! They open with the selected envelope's values (`object.envelope.info`); the menu items and
//! shortcuts of Make with Warp and Make with Mesh open the Reset variant while an envelope is
//! selected, as the reference app's do.

use std::ops::RangeInclusive;
use std::sync::atomic::{AtomicU64, Ordering};

use craft::behavior::IconId;
use craft::ui::buttons::{Button, IconButton};
use craft::ui::choice::{Radio, RadioGroup};
use craft::ui::contract::{Bind, Gap, Props as _, Show as _, ShowContainer as _, clicked};
use craft::ui::dialog::DialogId;
use craft::ui::display::Divider;
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Label;
use craft::ui::text::NumberField;
use serde_json::{Map, Value, json};
use vectorcraft_doc::{EnvelopeKind, NodeKind};

use super::form::{fields_of, json_of};
use super::host::{HeldOptions, open_held_with};
use super::preview::LivePreview;
use super::warp_effect::{percent_param, row_label, static_style, style_entries, style_param};
use crate::VectorcraftApp;

/// The dialog kinds.
pub const WARP: &str = "envelopeWarp";
pub const MESH: &str = "envelopeMesh";
pub const OPTIONS: &str = "envelopeOptions";

const MAKE_WARP: &str = "object.envelope.makeWithWarp";
const RESET_WARP: &str = "object.envelope.resetWithWarp";
const MAKE_MESH: &str = "object.envelope.makeWithMesh";
const RESET_MESH: &str = "object.envelope.resetWithMesh";
const OPTIONS_CMD: &str = "object.envelope.options";

/// Counts the opens, so a held Cancel of an earlier dialog leaves a later one's preview alone.
static OPENED: AtomicU64 = AtomicU64::new(0);

/// The open envelope dialog and the preview it keeps on the canvas.
pub(crate) struct Open {
    /// The hosted dialog.
    pub id: DialogId,
    /// Which of the three dialogs it is.
    pub kind: &'static str,
    /// The drawing that supplied the selection and defaults at opening.
    document: Option<u64>,
    /// The envelope the dialog previews.
    pub preview: LivePreview,
    // PLAN: ef.C11's `Open` has no `generation`; the same kind reopened has the same id, so the id cannot tell the held Cancel of the replaced dialog from the new one's.
    /// Which open this is ([`OPENED`]).
    pub generation: u64,
}

impl Open {
    fn on_original_drawing(&self, app: &VectorcraftApp) -> bool {
        app.session.active().map(|st| st.uid) == self.document
    }

    fn cancel_preview(self, app: &mut VectorcraftApp) {
        if self.on_original_drawing(app) {
            self.preview.cancel(app);
        }
    }
}

/// The dialog's title.
fn heading(kind: &str, reset: bool) -> Label {
    Label::from(
        match kind {
            WARP => tl!("Warp Options"),
            MESH if reset => tl!("Reset Envelope Mesh"),
            MESH => tl!("Envelope Mesh"),
            _ => tl!("Envelope Options"),
        }
        .to_string(),
    )
}

/// A whole-number row clamped to 1..50.
fn count(key: &str, label: Label) -> Param {
    Param::new(key, label, ParamKind::int(Some(1), Some(50), 4))
}

/// A check box row.
fn check(key: &str, label: Label, default: bool) -> Param {
    Param::new(key, label, ParamKind::bool(default))
}

/// A group of radio buttons without a title over `(id, label)` options.
fn choices(key: &str, options: [(&str, Label); 2]) -> Param {
    let options = options.into_iter().map(|(id, label)| (id.to_owned(), label)).collect();
    Param::new(key, Label::Empty, ParamKind::radio(options, None))
}

/// The rows of the dialog `kind`.
fn params(kind: &str, fields: &DialogFields) -> Vec<Param> {
    match kind {
        WARP => vec![
            style_param("style", fields.text("style").unwrap_or("arc")),
            choices("horizontal", [("horizontal", row_label(tl!("Horizontal"))), ("vertical", row_label(tl!("Vertical")))]),
            percent_param("bend", row_label(tl!("Bend:")), 50.0),
            Param::section(row_label(tl!("Distortion"))),
            percent_param("h", row_label(tl!("Horizontal:")), 0.0),
            percent_param("v", row_label(tl!("Vertical:")), 0.0),
        ],
        MESH => vec![
            count("rows", row_label(tl!("Rows:"))),
            count("cols", row_label(tl!("Columns:"))),
            check("maintainShape", row_label(tl!("Maintain Envelope Shape")), true).visible_when("reset", true),
        ],
        _ => vec![
            Param::section(row_label(tl!("Rasters"))),
            check("antiAlias", row_label(tl!("Anti-Alias")), true),
            Param::section(row_label(tl!("Preserve Shape Using:"))),
            choices("preserveShape", [("clippingMask", row_label(tl!("Clipping Mask"))), ("transparency", row_label(tl!("Transparency")))]),
            Param::new("fidelity", row_label(tl!("Fidelity:")), ParamKind::range(0.0, 100.0, 50.0, craft::ui::value_drag::SliderScale::Linear)),
            check("distortAppearance", row_label(tl!("Distort Appearance")), false),
            // Gradients and patterns bend only with the appearance.
            check("distortLinearGradients", row_label(tl!("Distort Linear Gradients")), false).enabled_when("distortAppearance", true),
            check("distortPatternFills", row_label(tl!("Distort Pattern Fills")), false).enabled_when("distortAppearance", true),
        ],
    }
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
            let horizontal = if !warp || info["horizontal"] != false { "horizontal" } else { "vertical" };
            (
                WARP,
                json!({"reset": envelope, "style": style, "horizontal": horizontal, "bend": own("bend", 50.0), "h": own("h", 0.0), "v": own("v", 0.0)}),
            )
        }
        MAKE_MESH | RESET_MESH => {
            let mesh = info["type"] == "mesh";
            let own = |k: &str| if mesh { info[k].as_i64().unwrap_or(4) } else { 4 };
            (MESH, json!({"reset": envelope, "rows": own("rows"), "cols": own("cols"), "maintainShape": true}))
        }
        OPTIONS_CMD => {
            let mut f = json!({"__envelope": envelope});
            for k in ["antiAlias", "preserveShape", "fidelity", "distortAppearance", "distortLinearGradients", "distortPatternFills"] {
                f[k] = info[k].clone();
            }
            (OPTIONS, f)
        }
        _ => return Err(format!("`{id}` has no envelope dialog")),
    };
    open_dialog(app, kind, fields.as_object().cloned().unwrap_or_default(), envelope)
}

/// Opens the hosted dialog `kind` over `values`, replacing an open envelope dialog; with no envelope selected the
/// Options dialog sets the defaults for new envelopes and has nothing to preview.
fn open_dialog(app: &mut VectorcraftApp, kind: &'static str, values: Map<String, Value>, envelope: bool) -> Result<Value, String> {
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let document = app.session.active().map(|st| st.uid);
    if let Some(previous) = app.effect_dialogs.envelope.take() {
        let id = previous.id;
        previous.cancel_preview(app);
        if let Some(dialogs) = app.dialogs() {
            dialogs.with(|host| host.cancel_by_id(&ctx, id));
        }
    }
    let fields = fields_of(&params(kind, &DialogFields::new()), &values);
    let dialog = HostedParamDialog::command(&ctx, kind, fields, move |fields| {
        let reset = fields.bool("reset").unwrap_or(false);
        ParamDialog::command(kind, heading(kind, reset), params(kind, fields)).previews(kind != OPTIONS || envelope)
    });
    let generation = OPENED.fetch_add(1, Ordering::Relaxed);
    let cancel = Box::new(move |app: &mut VectorcraftApp| {
        if app.effect_dialogs.envelope.as_ref().is_some_and(|open| open.generation == generation)
            && let Some(open) = app.effect_dialogs.envelope.take()
        {
            open.cancel_preview(app);
        }
    });
    let id = open_held_with(app, dialog, accept, HeldOptions { cancel: Some(cancel), ..Default::default() }).ok_or("no window")?;
    app.effect_dialogs.envelope = Some(Open { id, kind, document, preview: LivePreview::default(), generation });
    Ok(Value::Null)
}

/// OK: keeps the preview as one undo step. A refused command keeps the dialog open.
fn accept(app: &mut VectorcraftApp, result: &ParamResult) -> Result<(), String> {
    let Some(open) = app.effect_dialogs.envelope.as_ref().filter(|open| open.id == result.id) else {
        return Err("the envelope dialog is gone".into());
    };
    if !open.on_original_drawing(app) {
        return Err(tl!("Return to the original drawing to apply the envelope").into());
    }
    let kind = open.kind;
    let (cmd, p, _) = command(kind, &json_of(&params(kind, &result.fields), &result.fields));
    let Open { id, document, preview, generation, .. } = app.effect_dialogs.envelope.take().ok_or("the envelope dialog is gone")?;
    match preview.commit(app, cmd, p) {
        Ok(_) => Ok(()),
        Err(e) => {
            // The attempt ended the preview; the next pass starts a new one over the same values. Nothing to undo
            // when no interaction was open, so the answer is not needed.
            let _ = app.session.cancel_interaction();
            app.effect_dialogs.envelope = Some(Open { id, kind, document, preview: LivePreview::default(), generation });
            Err(e)
        }
    }
}

/// The command OK runs, its parameters and the preview's undo label.
fn command(kind: &str, f: &Map<String, Value>) -> (&'static str, Value, &'static str) {
    let num = |k: &str, d: f64| f.get(k).and_then(Value::as_f64).unwrap_or(d);
    let flag = |k: &str, d: bool| f.get(k).and_then(Value::as_bool).unwrap_or(d);
    let reset = flag("reset", false);
    match kind {
        WARP => {
            let style = f.get("style").and_then(Value::as_str).unwrap_or("arc");
            let horizontal = f.get("horizontal").and_then(Value::as_str) != Some("vertical");
            let p = json!({"style": style, "horizontal": horizontal, "bend": num("bend", 50.0), "h": num("h", 0.0), "v": num("v", 0.0)});
            if reset { (RESET_WARP, p, "Reset with Warp") } else { (MAKE_WARP, p, "Make Envelope") }
        }
        MESH => {
            let p = json!({"rows": num("rows", 4.0), "cols": num("cols", 4.0)});
            if reset {
                let mut p = p;
                p["maintainShape"] = json!(flag("maintainShape", true));
                (RESET_MESH, p, "Reset with Mesh")
            } else {
                (MAKE_MESH, p, "Make Envelope")
            }
        }
        _ => {
            let preserve = f.get("preserveShape").and_then(Value::as_str).unwrap_or("clippingMask");
            let mut p = json!({"fidelity": num("fidelity", 50.0), "preserveShape": preserve});
            for k in ["antiAlias", "distortAppearance", "distortLinearGradients", "distortPatternFills"] {
                p[k] = json!(flag(k, false));
            }
            (OPTIONS_CMD, p, "Envelope Options")
        }
    }
}

/// Runs the preview of the open envelope dialog from its values, once per frame: its Preview box decides whether the
/// canvas shows them. The dialog's Cancel rolls the preview back through the held record's cancel.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(mut open) = app.effect_dialogs.envelope.take() else { return };
    if !open.on_original_drawing(app) {
        app.effect_dialogs.envelope = Some(open);
        return;
    }
    let shown = app
        .dialogs()
        .and_then(|dialogs| dialogs.with(|host| host.get_mut_by_id::<HostedParamDialog>(open.id).map(|d| (d.is_previewing(), d.fields().clone()))));
    if let Some((on, fields)) = shown {
        let (cmd, p, label) = command(open.kind, &json_of(&params(open.kind, &fields), &fields));
        // PLAN: the sheet's `LivePreview::run` has no undo label; wg.A20's contract row passes one first.
        // The answer is not used: a refused preview is reported on the status line.
        open.preview.run(app, on, label, cmd, &p);
    }
    app.effect_dialogs.envelope = Some(open);
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
        EnvelopeKind::Warp { style, bend, h, v, horizontal, .. } => {
            Bar::Warp { style: style.clone(), bend: *bend, h: *h, v: *v, horizontal: *horizontal }
        }
        EnvelopeKind::Mesh { rows, cols, .. } => Bar::Mesh { rows: *rows, cols: *cols },
        EnvelopeKind::TopObject { .. } => Bar::TopObject,
    };
    Some((*editing, bar))
}

/// One member of the bar's envelope group: it leaves the bar whole, with the others of its rank, when the row is short.
fn member(ui: &mut egui::Ui, mut body: impl FnMut(&mut egui::Ui)) {
    Row::new().gap(Gap::Item).fit_rank(crate::chrome::rank::GROUP).show(ui, |ui, _| body(ui));
}

/// A labelled whole-number field of the bar over `range`; the value a commit gave.
fn field(ui: &mut egui::Ui, key: &'static str, caption: Label, value: f64, range: RangeInclusive<f64>, unit: Option<&'static str>) -> Option<f64> {
    let mut committed = None;
    Field::new(caption.clone()).id_salt(key).show(ui, |ui, _| {
        let mut number = NumberField::new(caption.clone(), Bind::given(Some(value))).range(range.clone()).commit_decimals(0);
        if let Some(unit) = unit {
            number = number.unit(unit);
        }
        if let Some(&n) = number.show(ui).edit.events.committed() {
            committed = n;
        }
    });
    committed
}

/// The Control bar's envelope controls (as in the reference app) while an envelope, or content
/// whose envelope is being edited, is selected: Edit Envelope / Edit Contents; a warp's style,
/// orientation, bend and distortions, or a mesh's rows and columns; Reset Envelope Shape; and
/// Envelope Options. Each control is a member of the bar that leaves it whole when the row is short.
pub fn control_bar(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    let Some((editing, bar)) = bar_of(app) else { return };
    let mut run: Option<(&'static str, Value)> = None;
    let mut options = false;
    member(ui, |ui| {
        Divider::vertical().show(ui);
        for (contents, icon, tip) in [(false, "dc-mesh", tl!("Edit Envelope")), (true, "shapes", tl!("Edit Contents"))] {
            let name = row_label(tip);
            let button = IconButton::new(IconId::new(icon)).label(name.clone()).tooltip(name).selected(editing == contents).show(ui);
            if clicked(&button) && editing != contents {
                run = Some(("object.envelope.editContents", json!({ "editing": contents })));
            }
        }
    });
    match &bar {
        Bar::Warp { style, bend, h, v, horizontal } => {
            member(ui, |ui| {
                let entries = style_entries(false);
                let caption = row_label(tl!("Style:"));
                Field::new(caption.clone()).id_salt("style").show(ui, |ui, _| {
                    let pick = Select::new(caption.clone(), Bind::given(static_style(style)), &entries).show(ui);
                    if let Some(&id) = pick.events.committed() {
                        run = Some((OPTIONS_CMD, json!({ "style": id })));
                    }
                });
            });
            member(ui, |ui| {
                let group = RadioGroup::new(row_label(tl!("Warp Options")), Bind::given(*horizontal)).show(ui, |ui, _| {
                    Row::new().gap(Gap::Item).show(ui, |ui, _| {
                        Radio::new(true).label(row_label(tl!("Horizontal"))).show(ui);
                        Radio::new(false).label(row_label(tl!("Vertical"))).show(ui);
                    });
                });
                if let Some(&h) = group.output.events.committed() {
                    run = Some((OPTIONS_CMD, json!({ "horizontal": h })));
                }
            });
            for (key, label, value) in [("bend", tl!("Bend:"), *bend), ("h", tl!("H:"), *h), ("v", tl!("V:"), *v)] {
                member(ui, |ui| {
                    if let Some(n) = field(ui, key, row_label(label), value, -100.0..=100.0, Some("%")) {
                        run = Some((OPTIONS_CMD, json!({ key: n.round().clamp(-100.0, 100.0) })));
                    }
                });
            }
        }
        Bar::Mesh { rows, cols } => {
            for (key, label, value) in [("rows", tl!("Rows:"), *rows), ("cols", tl!("Columns:"), *cols)] {
                member(ui, |ui| {
                    if let Some(n) = field(ui, key, row_label(label), f64::from(value), 1.0..=50.0, None) {
                        run = Some((RESET_MESH, json!({ key: n.round().clamp(1.0, 50.0), "maintainShape": true })));
                    }
                });
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
    if let Some(reset) = reset {
        member(ui, |ui| {
            let button = Button::new(row_label(tl!("Reset"))).tooltip(row_label(tl!("Reset Envelope Shape"))).show(ui);
            if clicked(&button) {
                run = Some(reset.clone());
            }
        });
    }
    member(ui, |ui| {
        let name = row_label(tl!("Envelope Options"));
        options |= clicked(&IconButton::new(IconId::new("dc-options")).label(name.clone()).tooltip(name).show(ui));
    });
    if options {
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
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::super::tests_warp::{accept, dismiss, host, json_fields, open_ids, pump, set, windowed};
    use super::*;
    use vectorcraft_doc::live::EnvelopeKind;
    use vectorcraft_doc::{NodeId, NodeKind};
    use vectorcraft_engine::Session;

    fn app_with_rect() -> (VectorcraftApp, NodeId) {
        let mut app = windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 200, "height": 100})).unwrap()["id"].as_u64().unwrap();
        app.run("select.all", json!({})).unwrap();
        (app, NodeId(id))
    }

    fn envelope(app: &VectorcraftApp) -> (NodeId, vectorcraft_doc::Node) {
        let st = app.session.doc().unwrap();
        let id = st.selection.objects[0];
        (id, st.doc.node(id).unwrap().clone())
    }

    /// The open envelope dialog.
    fn dialog(app: &VectorcraftApp) -> DialogId {
        app.effect_dialogs.envelope.as_ref().map(|open| open.id).expect("an open envelope dialog")
    }

    #[test]
    fn warp_dialog_previews_and_ok_keeps_one_step() {
        let (mut app, rect) = app_with_rect();
        crate::menus::invoke(&mut app, MAKE_WARP, json!({}));
        let id = dialog(&app);
        assert_eq!(app.effect_dialogs.envelope.as_ref().map(|open| open.kind), Some(WARP));
        assert_eq!(json_fields(&app, id)["reset"], json!(false));
        pump(&mut app);
        // The preview made the envelope already.
        assert!(matches!(envelope(&app).1.kind, NodeKind::Envelope { .. }));
        set(&app, id, "style", json!("flag"));
        set(&app, id, "horizontal", json!("vertical"));
        set(&app, id, "bend", json!(-30));
        pump(&mut app);
        let undo = app.session.doc().unwrap().history.undo.len();
        accept(&mut app, id);
        assert!(open_ids(&app).is_empty());
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
        let id = dialog(&app);
        let fields = json_fields(&app, id);
        assert_eq!((fields["reset"].clone(), fields["style"].clone(), fields["bend"].as_f64()), (json!(true), json!("flag"), Some(-30.0)));
        dismiss(&mut app, id);
        assert!(
            matches!(envelope(&app).1.kind, NodeKind::Envelope { ref kind, .. } if matches!(**kind, EnvelopeKind::Warp { .. })),
            "cancel rolls back"
        );
    }

    #[test]
    fn envelope_warp_refuses_preview_and_ok_in_the_other_drawing() {
        let (mut app, _) = app_with_rect();
        open(&mut app, MAKE_WARP).unwrap();
        let id = dialog(&app);
        set(&app, id, "bend", json!(23));
        pump(&mut app);
        app.run("file.new", json!({"width": 300, "height": 150})).unwrap();
        app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 60, "height": 50})).unwrap();
        app.run("select.all", json!({})).unwrap();
        let undo = app.session.active().unwrap().history.undo.len();
        set(&app, id, "bend", json!(31));
        pump(&mut app);
        assert!(matches!(envelope(&app).1.kind, NodeKind::Path { .. }), "the other drawing keeps the rectangle");
        let ctx = app.ui_ctx.clone().unwrap();
        host(&app, |h| h.confirm_by_id(&ctx, id));
        pump(&mut app);
        assert!(
            matches!(host(&app, |h| h.held_state(id)), Some(craft::ui::dialog::HeldOk::Refused(reason)) if reason.as_str().contains("original drawing"))
        );
        assert_eq!(json_fields(&app, id)["bend"], 31.0);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
        app.run("document.activate", json!({"index": 0})).unwrap();
        accept(&mut app, id);
        let (_, node) = envelope(&app);
        assert!(matches!(node.kind, NodeKind::Envelope { ref kind, .. } if matches!(**kind, EnvelopeKind::Warp { bend: 31.0, .. })));
    }

    #[test]
    fn cancelling_an_envelope_dialog_keeps_the_other_drawings_active_edit() {
        let (mut app, _) = app_with_rect();
        open(&mut app, MAKE_WARP).unwrap();
        let id = dialog(&app);
        pump(&mut app);
        app.run("file.new", json!({"width": 300, "height": 150})).unwrap();
        app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 60, "height": 50})).unwrap();
        app.run("select.all", json!({})).unwrap();
        app.session.begin_interaction("Other drawing's edit").unwrap();
        app.session.preview("paint.setFill", &json!({"color": "#00aa00"})).unwrap();
        dismiss(&mut app, id);
        assert!(app.session.in_interaction(), "the other drawing's edit stays active");
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
        let id = dialog(&app);
        assert_eq!(app.effect_dialogs.envelope.as_ref().map(|open| open.kind), Some(MESH));
        pump(&mut app);
        assert!(
            matches!(envelope(&app).1.kind, NodeKind::Envelope { ref kind, .. } if matches!(**kind, EnvelopeKind::Mesh { rows: 4, cols: 4, .. })),
            "previewed"
        );
        dismiss(&mut app, id);
        assert!(matches!(envelope(&app).1.kind, NodeKind::Envelope { ref kind, .. } if matches!(**kind, EnvelopeKind::Warp { .. })));
        app.run("ui.menuDialog", json!({"command": RESET_MESH})).unwrap();
        let id = dialog(&app);
        set(&app, id, "rows", json!(2));
        accept(&mut app, id);
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
        let id = dialog(&app);
        let fields = json_fields(&app, id);
        assert_eq!((fields["fidelity"].as_f64(), fields["distortPatternFills"].clone()), (Some(80.0), json!(true)));
        set(&app, id, "preserveShape", json!("transparency"));
        pump(&mut app);
        accept(&mut app, id);
        let info = app.session.execute("object.envelope.info", &json!({})).unwrap();
        assert_eq!((info["preserveShape"].clone(), info["fidelity"].clone()), (json!("transparency"), json!(80.0)));
        // Nothing selected: the defaults for new envelopes, no preview.
        app.run("select.none", json!({})).unwrap();
        crate::menus::invoke(&mut app, OPTIONS_CMD, json!({}));
        let id = dialog(&app);
        assert_eq!(host(&app, |h| h.get_mut_by_id::<HostedParamDialog>(id).map(|d| d.is_previewing())), Some(false));
        set(&app, id, "distortAppearance", json!(false));
        pump(&mut app);
        assert!(!app.session.in_interaction());
        accept(&mut app, id);
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
        crate::tests_synthetic::key_frame(&mut app, &crate::testing::ui_context(), vec![]);
        assert!(crate::menus::shortcut_of(&app, "object.envelope.editContents").is_none(), "its reference shortcut is Paste in Place's here");
    }

    #[test]
    fn the_control_bar_switches_to_edit_contents_and_back() {
        let (mut app, rect) = app_with_rect();
        app.run(MAKE_WARP, json!({"style": "arch"})).unwrap();
        assert!(bar_of(&app).is_some_and(|(editing, bar)| !editing && matches!(bar, Bar::Warp { ref style, .. } if style == "arch")));
        let app = Rc::new(RefCell::new(app));
        let shown = Rc::clone(&app);
        let mut fixture = craft::ui::testing::Fixture::new(move |ui| crate::chrome::control_bar(&mut shown.borrow_mut(), ui));
        fixture.frame(Vec::new());
        fixture.frame(Vec::new());
        let contents = fixture.by_label("Edit Contents").expect("the Edit Contents button");
        fixture.click(&contents);
        assert_eq!(app.borrow().session.doc().unwrap().selection.objects, vec![rect], "Edit Contents selects the content");
        assert!(bar_of(&app.borrow()).is_some_and(|(editing, _)| editing), "the content keeps the envelope's controls");
        fixture.frame(Vec::new());
        let envelope = fixture.by_label("Edit Envelope").expect("the Edit Envelope button");
        fixture.click(&envelope);
        assert!(bar_of(&app.borrow()).is_some_and(|(editing, _)| !editing));
    }
}
