//! Transform dialogs: Move, Rotate, Scale, Reflect and Shear (with Transform Objects, Transform
//! Patterns, Copy and Preview; Scale also has Uniform, Scale Corners and Scale Strokes & Effects;
//! Reflect's axis is Horizontal, Vertical or an angle). With Preview on, the transform shows on the
//! canvas as the values change.
//!
//! Move opens on the offset moved last, by a drag or by itself, and with the Copy and Transform
//! Objects boxes it was last confirmed with, so Enter, Enter repeats a move or a copy at the same
//! spacing.

use craft::ui::contract::Bind;
use craft::ui::dialog::DialogId; // PLAN: assumed path of `DialogId` in the craft_ui facade
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::memory::SaveKey;
use serde_json::{Map, Value, json};

use super::form::{fields_of, grid_params};
use super::host::{HeldOptions, open_held_with};
use super::preview::LivePreview;
use crate::VectorcraftApp;
use crate::canvas::rulers::general_lengths;

/// Where Move keeps its Copy and Transform Objects boxes between runs.
const MOVE_KEY: SaveKey = SaveKey::new("vectorcraft.move");

/// The Distance and Angle Move's last link pass saw: a change in them since, with Horizontal and
/// Vertical unchanged, means the polar pair was edited. Hidden fields (`__` keys never leave the dialog).
// PLAN: the `__polar` list is kept as two numbers; the bundle gives no `FieldValue` list reader
const POLAR_DISTANCE: &str = "__polarDistance";
const POLAR_ANGLE: &str = "__polarAngle";

/// The five transform dialogs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kind {
    /// Object › Transform › Move.
    Move,
    /// Object › Transform › Rotate and the Rotate tool.
    Rotate,
    /// Object › Transform › Scale and the Scale tool.
    Scale,
    /// Object › Transform › Reflect and the Reflect tool.
    Reflect,
    /// Object › Transform › Shear and the Shear tool.
    Shear,
}

impl Kind {
    /// The dialog a menu command opens.
    pub(crate) fn of_command(id: &str) -> Option<Kind> {
        Some(match id {
            "object.move" => Kind::Move,
            "object.rotate" => Kind::Rotate,
            "object.scale" => Kind::Scale,
            "object.reflect" => Kind::Reflect,
            "object.shear" => Kind::Shear,
            _ => return None,
        })
    }

    /// The dialog a tool's double click or Enter opens; Move has no tool of its own.
    pub(crate) fn of_tool(kind: &str) -> Option<Kind> {
        Some(match kind {
            "rotate" => Kind::Rotate,
            "scale" => Kind::Scale,
            "reflect" => Kind::Reflect,
            "shear" => Kind::Shear,
            _ => return None,
        })
    }

    /// The command OK runs, which is also the dialog's key.
    pub(crate) fn command(self) -> &'static str {
        match self {
            Kind::Move => "object.move",
            Kind::Rotate => "object.rotate",
            Kind::Scale => "object.scale",
            Kind::Reflect => "object.reflect",
            Kind::Shear => "object.shear",
        }
    }

    /// The kind name `grid_params` keys its rows by.
    pub(crate) fn dialog_kind(self) -> &'static str {
        match self {
            Kind::Move => "move",
            Kind::Rotate => "rotate",
            Kind::Scale => "scale",
            Kind::Reflect => "reflect",
            Kind::Shear => "shear",
        }
    }

    /// The dialog's title, and the name of its preview's undo step.
    pub(crate) fn title(self) -> &'static str {
        match self {
            Kind::Move => tl!("Move"),
            Kind::Rotate => tl!("Rotate"),
            Kind::Scale => tl!("Scale"),
            Kind::Reflect => tl!("Reflect"),
            Kind::Shear => tl!("Shear"),
        }
    }
}

/// The open transform dialog: which one, the reference point the tool click chose (none from a menu),
/// and its live preview.
pub(crate) struct Open {
    /// The dialog's id on the window's host.
    pub id: DialogId,
    /// The drawing whose selection and reference point the dialog acts on: its uid and revert count.
    document: Option<(u64, u64)>,
    /// Which transform it is.
    pub kind: Kind,
    /// The reference point of a tool click, sent with every command but Move's.
    pub origin: Option<Value>,
    /// The preview on the canvas.
    pub preview: LivePreview,
}

impl Open {
    fn in_source(&self, app: &VectorcraftApp) -> bool {
        self.document == app.session.active().map(|doc| (doc.uid, doc.reverts))
    }

    /// Whether the active drawing is the opening one, reverted since the dialog opened.
    fn reverted(&self, app: &VectorcraftApp) -> bool {
        self.document.is_some_and(|(uid, reverts)| app.session.active().is_some_and(|doc| doc.uid == uid && doc.reverts != reverts))
    }

    fn owns_preview(&self, app: &VectorcraftApp) -> bool {
        app.session.active().and_then(|doc| doc.interaction.as_ref()).is_some_and(|interaction| {
            interaction.preview.as_ref().map_or_else(|| interaction.label == self.kind.title(), |(command, _)| command == self.kind.command())
        })
    }

    fn cancel_preview(&mut self, app: &mut VectorcraftApp) {
        let preview = std::mem::take(&mut self.preview);
        if self.in_source(app) && self.owns_preview(app) {
            preview.cancel(app);
        }
    }
}

/// Opens the dialog of `kind`; `tool` is the tool click's params (its `origin`), or `{}` from a menu.
pub(crate) fn open(app: &mut VectorcraftApp, kind: Kind, tool: &Value) {
    let Some(ctx) = app.ui_ctx.clone() else {
        app.status("no window".to_string());
        return;
    };
    let Value::Object(opening) = opening(app, kind) else { return };
    let document = app.session.active().map(|doc| (doc.uid, doc.reverts));
    let rows = rows(app, kind, &opening);
    let fields = fields_of(&rows, &opening);
    let dialog = HostedParamDialog::new(&ctx, kind.command(), fields, move |f| {
        if kind == Kind::Move {
            link_distance_and_angle(f);
        }
        let dialog = ParamDialog::new(kind.command(), kind.title(), rows.clone()).previews(true).values(Bind::bound(f));
        if kind == Kind::Move { dialog.remember(MOVE_KEY) } else { dialog }
    });
    // A dialog this one replaces leaves its preview behind otherwise.
    cancel_preview(app);
    let ok_ctx = ctx.clone();
    let options = HeldOptions { parent: None, cancel: Some(Box::new(cancel_preview)) };
    let Some(id) = open_held_with(app, dialog, move |app, result| ok(app, kind, &ok_ctx, result), options) else {
        app.status("no window".to_string());
        return;
    };
    app.object_dialogs.transform = Some(Open { id, document, kind, origin: tool.get("origin").cloned(), preview: LivePreview::default() });
}

/// Rolls the preview back and drops the record: Cancel, Esc and the close box.
fn cancel_preview(app: &mut VectorcraftApp) {
    if let Some(mut open) = app.object_dialogs.transform.take() {
        open.cancel_preview(app);
    }
}

/// Runs the preview of the open dialog from its current values; OK and Cancel end it.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(record) = app.object_dialogs.transform.as_ref() else { return };
    let (id, kind) = (record.id, record.kind);
    let Some(dialogs) = app.dialogs() else { return };
    let shown = dialogs.with(|host| host.get_mut_by_id::<HostedParamDialog>(id).map(|d| (d.is_previewing(), d.fields().clone())));
    let Some((on, fields)) = shown else { return };
    let Some(mut open) = app.object_dialogs.transform.take() else { return };
    if !open.in_source(app) {
        open.preview = LivePreview::default();
        app.object_dialogs.transform = Some(open);
        return;
    }
    if !open.owns_preview(app) {
        open.preview = LivePreview::default();
    }
    let (cmd, mut params) = command(app, kind, &fields, open.origin.as_ref());
    if kind == Kind::Scale {
        add_scale_options(app, &fields, &mut params);
    }
    // `run` reports a refusal on the status line itself.
    open.preview.run(app, on, kind.title(), cmd, &params);
    app.object_dialogs.transform = Some(open);
}

/// OK: the preview is rolled back and the command run as without one, so its undo step, its journal
/// entry and Transform Again are the same either way.
fn ok(app: &mut VectorcraftApp, kind: Kind, ctx: &egui::Context, result: &ParamResult) -> Result<(), String> {
    let Some(mut open) = app.object_dialogs.transform.take() else { return Err("the Transform dialog is closed".into()) };
    if open.id != result.id || open.kind != kind {
        app.object_dialogs.transform = Some(open);
        return Err("the Transform dialog was replaced".into());
    }
    if !open.in_source(app) {
        let why = if open.reverted(app) {
            tl!("The document was reverted. Reopen this dialog to continue.")
        } else {
            tl!("Return to the original drawing to apply the transform")
        };
        app.object_dialogs.transform = Some(open);
        return Err(why.into());
    }
    open.cancel_preview(app);
    let fields = &result.fields;
    let (cmd, mut params) = command(app, kind, fields, open.origin.as_ref());
    app.object_dialogs.transform = Some(open);
    if kind == Kind::Scale {
        params = with_scale_options(app, fields, params)?;
    }
    app.run(cmd, params)?;
    if kind == Kind::Move {
        // Move's Transform Patterns stays as confirmed: it is the preference it starts from.
        let patterns = transform_patterns(app, fields);
        if patterns != app.session.prefs.transform_pattern_tiles {
            app.run("prefs.set", json!({"key": "transformPatternTiles", "value": patterns}))?;
        }
    }
    result.remember(ctx);
    app.object_dialogs.transform = None;
    Ok(())
}

/// The fields `kind` opens with: the last offset for Move, the reference values of the others, and the
/// option boxes from the preferences.
fn opening(app: &VectorcraftApp, kind: Kind) -> Value {
    let prefs = &app.session.prefs;
    let mut fields = match kind {
        Kind::Move => {
            let [dx, dy, distance, angle] = offset(last_move(app).map_or([0.0; 2], |(dx, dy)| [dx, dy]));
            json!({"dx": dx, "dy": dy, "distance": distance, "angle": angle, POLAR_DISTANCE: distance, POLAR_ANGLE: angle})
        }
        Kind::Rotate => json!({"angle": 0}),
        Kind::Scale => json!({"sx": 100, "sy": 100, "uniform": true, "corners": prefs.scale_corners, "strokes": prefs.scale_strokes}),
        Kind::Reflect => json!({"axis": "vertical", "angle": 90}),
        Kind::Shear => json!({"angle": 0, "axis": "horizontal", "axisAngle": 0}),
    };
    fields["objects"] = json!(true);
    fields["patterns"] = json!(prefs.transform_pattern_tiles);
    fields
}

/// The dialog's rows: the grid of `kind`, then its check boxes. Move's number rows and Transform
/// Patterns are decided by the journal and the preference, not by what the dialog remembers.
fn rows(app: &VectorcraftApp, kind: Kind, opening: &Map<String, Value>) -> Vec<Param> {
    let mut rows = grid_params(kind.dialog_kind(), opening, general_lengths(app));
    if kind == Kind::Move {
        rows = rows.into_iter().map(Param::not_remembered).collect();
    }
    let mut checks: Vec<(&str, &str)> = Vec::new();
    if kind == Kind::Scale {
        checks.push(("uniform", tl!("Uniform")));
        checks.extend(SCALE_OPTIONS.map(|(field, _, label)| (field, tl!(label))));
    }
    checks.extend([("objects", tl!("Transform Objects")), ("patterns", tl!("Transform Patterns")), ("copy", tl!("Copy (make a transformed copy)"))]);
    rows.extend(checks.into_iter().map(|(key, label)| {
        let default = opening.get(key).and_then(Value::as_bool).unwrap_or(false);
        let row = Param::new(key, label, ParamKind::bool(default));
        if key == "patterns" { row.not_remembered() } else { row }
    }));
    rows
}

/// `[dx, dy, distance, angle]` of the offset `(dx, dy)`. The angle is counter-clockwise on the page and
/// y points down, so 90° moves up.
fn offset([dx, dy]: [f64; 2]) -> [f64; 4] {
    [dx, dy, dx.hypot(dy), (-dy).atan2(dx).to_degrees()].map(clean)
}

/// Move's Distance and Angle (fields `distance`, `angle`) and Horizontal and Vertical (`dx`, `dy`)
/// are the same offset: the pair edited since the last call (the hidden polar fields hold the values
/// then) sets the other.
fn link_distance_and_angle(f: &mut DialogFields) {
    let [dx, dy, distance, angle] = ["dx", "dy", "distance", "angle"].map(|k| f.number(k).unwrap_or(0.0));
    let polar_edited = match (f.number(POLAR_DISTANCE), f.number(POLAR_ANGLE)) {
        (Some(d), Some(a)) => (d, a) != (distance, angle),
        _ => false,
    };
    let [dx, dy, distance, angle] = if polar_edited {
        let a = angle.to_radians();
        // An angle past 180° reads as its negative one (270° is -90°), as the documented dialog has it.
        let turn = angle.rem_euclid(360.0);
        let angle = if turn > 180.0 { turn - 360.0 } else { turn };
        [distance * a.cos(), -distance * a.sin(), distance, angle].map(clean)
    } else {
        offset([dx, dy])
    };
    for (k, v) in [("dx", dx), ("dy", dy), ("distance", distance), ("angle", angle), (POLAR_DISTANCE, distance), (POLAR_ANGLE, angle)] {
        f.set(k, v);
    }
}

/// `v` without the float noise of the trigonometry (cos 90° is not quite 0). Adding 0 turns -0
/// into 0.
fn clean(v: f64) -> f64 {
    (v * 1e6).round() / 1e6 + 0.0
}

/// The offset of the last move the session's journal keeps: a drag that moved the selection (a
/// Transform that only translates) or Move.
fn last_move(app: &VectorcraftApp) -> Option<(f64, f64)> {
    let length = |v: Option<&Value>| match v {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => vectorcraft_doc::Unit::Points.parse(s),
        _ => Some(0.0),
    };
    let finite = |(dx, dy): (f64, f64)| (dx.is_finite() && dy.is_finite()).then_some((dx, dy));
    app.session.journal.iter().rev().find_map(|(id, p)| match id.as_str() {
        "object.move" => finite((length(p.get("dx"))?, length(p.get("dy"))?)),
        "object.transform" => {
            let [a, b, c, d, e, f] = vectorcraft_engine::cmd::matrix_param(p, "matrix")?.as_coeffs();
            let translates = (a, b, c, d) == (1.0, 0.0, 0.0, 1.0) && (e, f) != (0.0, 0.0);
            translates.then_some((e, f)).and_then(finite)
        }
        _ => None,
    })
}

/// Transform Patterns: the dialog's field, else General › Transform Pattern Tiles.
pub(super) fn transform_patterns(app: &VectorcraftApp, f: &DialogFields) -> bool {
    f.bool("patterns").unwrap_or(app.session.prefs.transform_pattern_tiles)
}

/// Transform Objects: the dialog's field, on until it is switched off (the pattern tiles alone
/// transform then, with Transform Patterns).
pub(super) fn transform_objects(f: &DialogFields) -> bool {
    f.bool("objects").unwrap_or(true)
}

/// Add the dialog's Transform Objects and Transform Patterns options to command `params`.
pub(super) fn add_object_options(app: &VectorcraftApp, f: &DialogFields, params: &mut Value) {
    params["objects"] = json!(transform_objects(f));
    params["patterns"] = json!(transform_patterns(app, f));
}

/// The scale options: (dialog field, preference, label).
const SCALE_OPTIONS: [(&str, &str, &str); 2] = [("corners", "scaleCorners", "Scale Corners"), ("strokes", "scaleStrokes", "Scale Strokes & Effects")];

/// A scale option's value: the dialog's field, else the preference.
pub(super) fn scale_option(app: &VectorcraftApp, f: &DialogFields, field: &str) -> bool {
    let prefs = &app.session.prefs;
    f.bool(field).unwrap_or(if field == "corners" { prefs.scale_corners } else { prefs.scale_strokes })
}

/// Add the dialog's scale options to command `params`.
pub(super) fn add_scale_options(app: &VectorcraftApp, f: &DialogFields, params: &mut Value) {
    for (field, _, _) in SCALE_OPTIONS {
        params[field] = json!(scale_option(app, f, field));
    }
}

/// `params` with the dialog's scale options, which also become the preferences (they are shared
/// with the Transform panel and Preferences, as in the reference app).
pub(super) fn with_scale_options(app: &mut VectorcraftApp, f: &DialogFields, mut params: Value) -> Result<Value, String> {
    add_scale_options(app, f, &mut params);
    let prefs: Map<String, Value> = SCALE_OPTIONS.iter().map(|(field, pref, _)| (pref.to_string(), params[*field].clone())).collect();
    app.run("prefs.set", json!({ "values": prefs }))?;
    Ok(params)
}

/// Pass a tool-chosen reference point through to transform commands.
fn origin_params(origin: Option<&Value>, mut p: Value) -> Value {
    if let Some(o) = origin {
        p["origin"] = o.clone();
    }
    p
}

/// The dialog's command and its params (Scale's without its scale options).
fn command(app: &VectorcraftApp, kind: Kind, f: &DialogFields, origin: Option<&Value>) -> (&'static str, Value) {
    let number = |key: &str, default: f64| f.number(key).unwrap_or(default);
    let axis = f.text("axis").unwrap_or("");
    let mut params = match kind {
        Kind::Move => json!({"dx": number("dx", 0.0), "dy": number("dy", 0.0)}),
        Kind::Rotate => json!({"angle": number("angle", 0.0)}),
        Kind::Scale => {
            let sx = number("sx", 100.0);
            let sy = if f.bool("uniform").unwrap_or(false) { sx } else { number("sy", 100.0) };
            json!({"sx": sx, "sy": sy})
        }
        // The axis by name, or the angle it is at.
        Kind::Reflect if axis == "angle" => json!({"axis": number("angle", 90.0)}),
        Kind::Reflect => json!({"axis": if axis.is_empty() { "vertical" } else { axis }}),
        Kind::Shear if axis == "angle" => json!({"angle": number("angle", 0.0), "axis": number("axisAngle", 0.0)}),
        Kind::Shear => json!({"angle": number("angle", 0.0), "axis": axis}),
    };
    params["copy"] = json!(f.bool("copy").unwrap_or(false));
    add_object_options(app, f, &mut params);
    // Move has no reference point.
    let params = if kind == Kind::Move { params } else { origin_params(origin, params) };
    (kind.command(), params)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use vectorcraft_engine::Session;

    use super::*;

    /// An app with one rectangle (100, 100, 60 × 20) and a window context whose host the frames run.
    struct Rig {
        app: VectorcraftApp,
        ctx: egui::Context,
    }

    impl Rig {
        fn new() -> Self {
            let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
            app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
            app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 60, "height": 20})).unwrap();
            let ctx = app.ui_ctx.clone().expect("a window");
            // The window's first frame gives it its dialog host.
            crate::testing::app_frame(&mut app, &ctx, Default::default());
            Rig { app, ctx }
        }

        /// One frame: the host shows its dialogs, then the module pumps and the held OKs run.
        fn frame(&mut self) {
            let host = self.app.dialogs().unwrap();
            let mut out = self.ctx.run_ui(egui::RawInput::default(), |ui| host.with(|h| h.run(ui)));
            out.textures_delta.clear();
            crate::dialogs::host::pump(&mut self.app, &self.ctx);
        }

        fn id(&self) -> egui::Id {
            self.app.object_dialogs.transform.as_ref().unwrap().id
        }

        /// Opens the dialog of menu command `cmd` and runs its first frame.
        fn open(&mut self, cmd: &str) {
            assert!(crate::dialogs::object_dialogs::open_from_menu(&mut self.app, cmd));
            self.frame();
        }

        /// Sets a field as a control client would, with no frame after it.
        fn set_now(&mut self, field: &str, value: Value) {
            let id = self.id();
            self.app.dialogs().unwrap().with(|h| h.set_json_field(&self.ctx, id, field, value)).unwrap();
        }

        fn set(&mut self, field: &str, value: Value) {
            self.set_now(field, value);
            self.frame();
        }

        /// OK: the held OK is answered in the first frame and the dialog closes in the second.
        fn confirm(&mut self) {
            let id = self.id();
            self.app.dialogs().unwrap().with(|h| h.confirm_by_id(&self.ctx, id));
            self.frame();
            self.frame();
        }

        fn cancel(&mut self) {
            let id = self.id();
            self.app.dialogs().unwrap().with(|h| h.cancel_by_id(&self.ctx, id));
            self.frame();
        }

        fn number(&self, key: &str) -> f64 {
            let id = self.id();
            self.app.dialogs().unwrap().with(|h| h.get_by_id::<HostedParamDialog>(id).and_then(|d| d.fields().number(key))).unwrap()
        }

        fn flag(&self, key: &str) -> bool {
            let id = self.id();
            self.app.dialogs().unwrap().with(|h| h.get_by_id::<HostedParamDialog>(id).and_then(|d| d.fields().bool(key))).unwrap()
        }

        fn bounds(&self) -> vectorcraft_geom::Rect {
            self.app.session.doc().unwrap().doc.layers[0].children().unwrap()[0].geometric_bounds().unwrap()
        }

        fn size(&self) -> (f64, f64) {
            let b = self.bounds();
            (b.width().round(), b.height().round())
        }

        fn undo_steps(&self) -> usize {
            self.app.session.doc().unwrap().history.undo.len()
        }

        fn last_command(&self) -> (String, Value) {
            self.app.session.journal.last().cloned().unwrap()
        }
    }

    #[test]
    fn transform_preview_pauses_before_editing_a_second_drawing() {
        let mut rig = two_drawings();
        rig.open("object.move");
        rig.set("dx", json!(35));
        assert_eq!(rig.bounds().x0, 135.0);
        rig.app.run("document.activate", json!({"index": 1})).unwrap();
        rig.frame();
        assert_eq!(rig.bounds().x0, 10.0);
        assert!(!rig.app.session.in_interaction());

        rig.app.run("document.activate", json!({"index": 0})).unwrap();
        rig.frame();
        assert_eq!(rig.bounds().x0, 135.0);
        rig.confirm();
        assert_eq!(rig.bounds().x0, 135.0);
        assert!(!rig.app.session.in_interaction());
    }

    #[test]
    fn transform_ok_waits_for_its_original_drawing() {
        let mut rig = two_drawings();
        rig.open("object.scale");
        rig.set("preview", json!(false));
        rig.set("sx", json!(200));
        rig.app.run("document.activate", json!({"index": 1})).unwrap();
        let steps = rig.undo_steps();
        let id = rig.id();
        let dialogs = rig.app.dialogs().unwrap();
        rig.confirm();
        assert!(matches!(dialogs.with(|h| h.held_state(id)), Some(craft::ui::dialog::HeldOk::Refused(_))));
        assert_eq!(rig.size(), (25.0, 15.0));
        assert_eq!(rig.undo_steps(), steps);

        rig.app.run("document.activate", json!({"index": 0})).unwrap();
        crate::tests::picks::answer_dialog(&mut rig.app, id, &[], true);
        assert_eq!(rig.size().0, 120.0);
        assert!(rig.app.object_dialogs.transform.is_none());
        assert!(dialogs.with(|h| h.get_by_id::<HostedParamDialog>(id).is_none()));
    }

    #[test]
    fn cancelling_transform_leaves_a_second_drawings_interaction_alone() {
        let mut rig = two_drawings();
        rig.open("object.move");
        rig.set("dx", json!(35));
        assert!(rig.app.session.in_interaction());
        rig.app.run("document.activate", json!({"index": 1})).unwrap();
        rig.app.session.begin_interaction("The second drawing's edit").unwrap();
        rig.cancel();
        assert_eq!(rig.app.session.active().unwrap().interaction.as_ref().unwrap().label, "The second drawing's edit");
        assert_eq!(rig.bounds().x0, 10.0);
    }

    #[test]
    fn transform_refuses_its_draft_after_file_revert() {
        let mut rig = Rig::new();
        let dir = vectorcraft_testkit::TempDir::new("transform-revert");
        rig.app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|e| e.to_string())));
        rig.app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string())));
        rig.app.run("file.saveAs", json!({"path": dir.join("Art.vectorcraft")})).unwrap();
        rig.open("object.move");
        rig.set("dx", json!(35));
        assert_eq!(rig.bounds().x0, 135.0);

        rig.app.run("file.revert", json!({"confirmed": true})).unwrap();
        rig.app.run("select.all", json!({})).unwrap();
        rig.frame();
        assert_eq!(rig.bounds().x0, 100.0, "the draft must not preview on the restored drawing");
        let steps = rig.undo_steps();
        let id = rig.id();
        let dialogs = rig.app.dialogs().unwrap();
        rig.confirm();
        assert_eq!(rig.bounds().x0, 100.0, "OK must not move the restored drawing's art");
        assert_eq!(rig.undo_steps(), steps);
        assert!(matches!(
            dialogs.with(|h| h.held_state(id)),
            Some(craft::ui::dialog::HeldOk::Refused(reason)) if reason.as_str().contains("document was reverted")
        ));
    }

    fn two_drawings() -> Rig {
        let mut rig = Rig::new();
        rig.app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        rig.app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 25, "height": 15})).unwrap();
        rig.app.run("document.activate", json!({"index": 0})).unwrap();
        rig
    }

    #[test]
    fn scale_options_start_from_and_update_the_preferences() {
        let mut rig = Rig::new();
        rig.app.run("select.all", json!({})).unwrap();
        rig.app.run("prefs.set", json!({"values": {"scaleStrokes": true, "scaleCorners": false}})).unwrap();
        rig.open("object.scale");
        assert_eq!((rig.flag("strokes"), rig.flag("corners")), (true, false));
        rig.set("sx", json!(200));
        rig.set("strokes", json!(false));
        rig.set("corners", json!(true));
        rig.confirm();
        let prefs = &rig.app.session.prefs;
        assert_eq!((prefs.scale_strokes, prefs.scale_corners), (false, true));
        let (cmd, p) = rig.last_command();
        assert_eq!((cmd.as_str(), &p["strokes"], &p["corners"]), ("object.scale", &json!(false), &json!(true)));
    }

    /// OK with Preview on keeps one undo step that Transform Again repeats; Cancel leaves the
    /// document and its history as they were.
    #[test]
    fn ok_with_a_preview_is_one_undo_step_and_cancel_restores() {
        let mut rig = Rig::new();
        let steps = rig.undo_steps();
        rig.open("object.rotate");
        rig.set("angle", json!(90));
        assert!(rig.app.session.in_interaction());
        assert_eq!(rig.size(), (20.0, 60.0), "the canvas shows the preview");
        rig.cancel();
        assert!(!rig.app.session.in_interaction());
        assert_eq!((rig.size(), rig.undo_steps()), ((60.0, 20.0), steps));
        rig.open("object.rotate");
        rig.set("angle", json!(90));
        rig.confirm();
        assert!(!rig.app.session.in_interaction());
        assert_eq!((rig.size(), rig.undo_steps()), ((20.0, 60.0), steps + 1));
        assert_eq!(rig.last_command().0, "object.rotate");
        rig.app.run("object.transformAgain", json!({})).unwrap();
        assert_eq!(rig.size(), (60.0, 20.0));
    }

    /// A transform dialog sends its command's params: Reflect and Shear by name and by angle, and
    /// Transform Objects off.
    #[test]
    fn a_transform_dialog_sends_its_commands_params() {
        type Fields<'a> = Vec<(&'a str, Value)>;
        let rows: [(&str, Fields<'_>, &str, Value); 5] = [
            ("object.reflect", vec![("axis", json!("horizontal"))], "axis", json!("horizontal")),
            ("object.reflect", vec![("axis", json!("angle")), ("angle", json!(45))], "axis", json!(45.0)),
            ("object.shear", vec![("axis", json!("vertical")), ("angle", json!(45))], "axis", json!("vertical")),
            ("object.shear", vec![("axis", json!("angle")), ("axisAngle", json!(30)), ("angle", json!(45))], "axis", json!(30.0)),
            ("object.reflect", vec![("objects", json!(false))], "objects", json!(false)),
        ];
        for (cmd, sets, key, want) in rows {
            let mut rig = Rig::new();
            rig.open(cmd);
            for (field, value) in sets {
                rig.set(field, value);
            }
            rig.confirm();
            let (id, p) = rig.last_command();
            assert_eq!((id.as_str(), &p[key]), (cmd, &want), "{cmd} {key}");
        }
    }

    /// #1915: Move's Distance and Angle follow Horizontal and Vertical, and they follow them back.
    /// The angle is counter-clockwise on the page and Vertical is down = +, so 90° moves up.
    #[test]
    fn move_takes_a_distance_and_an_angle() {
        let mut rig = Rig::new();
        rig.open("object.move");
        let fields = |rig: &Rig| ["dx", "dy", "distance", "angle"].map(|k| rig.number(k));
        assert_eq!(fields(&rig), [0.0, 0.0, 0.0, 0.0]);
        rig.set("distance", json!(10.0));
        rig.set("angle", json!(90.0));
        rig.frame();
        assert_eq!(fields(&rig), [0.0, -10.0, 10.0, 90.0]);
        rig.set("dx", json!(3.0));
        rig.set("dy", json!(4.0));
        rig.frame();
        assert_eq!(fields(&rig), [3.0, 4.0, 5.0, -53.130102]);
        rig.set("distance", json!(10.0));
        rig.set("angle", json!(270.0));
        rig.frame();
        assert_eq!(fields(&rig), [0.0, 10.0, 10.0, -90.0], "270° is -90°");
        rig.set("distance", json!(10.0));
        rig.set("angle", json!(90.0));
        rig.confirm();
        let (cmd, p) = rig.last_command();
        assert_eq!((cmd.as_str(), &p["dx"], &p["dy"]), ("object.move", &json!(0.0), &json!(-10.0)));
        let b = rig.bounds();
        assert_eq!((b.x0, b.y0), (100.0, 90.0));
    }

    /// A Distance and Angle set before the dialog's first frame, or just before OK, move by that
    /// offset (as `ui.dialog.set` then `ui.dialog.confirm` from a control client).
    #[test]
    fn move_takes_a_distance_and_an_angle_set_before_a_frame() {
        let mut rig = Rig::new();
        // Up 10 pt with no frame, then right 20 pt (Move opens on the last offset) with one.
        for (with_frame, distance, angle, want, corner_after) in
            [(false, 10.0, 90.0, (0.0, -10.0), (100.0, 90.0)), (true, 20.0, 0.0, (20.0, 0.0), (120.0, 90.0))]
        {
            assert!(crate::dialogs::object_dialogs::open_from_menu(&mut rig.app, "object.move"));
            rig.set_now("distance", json!(distance));
            rig.set_now("angle", json!(angle));
            if with_frame {
                rig.frame();
                assert_eq!([rig.number("distance"), rig.number("angle")], [distance, angle], "the first frame keeps them");
            }
            rig.confirm();
            let (cmd, p) = rig.last_command();
            assert_eq!((cmd.as_str(), &p["dx"], &p["dy"]), ("object.move", &json!(want.0), &json!(want.1)), "with a frame: {with_frame}");
            let b = rig.bounds();
            assert_eq!((b.x0, b.y0), corner_after);
        }
    }

    /// Move opens on the offset of the last drag or Move, and with the Copy and Transform Objects
    /// boxes it was last confirmed with: Enter, Enter makes copies at the same spacing.
    #[test]
    fn move_opens_on_the_last_move_and_checkboxes() {
        let mut rig = Rig::new();
        let open = |rig: &mut Rig| {
            crate::toolbar::open_options(&mut rig.app, "selection").unwrap();
            rig.frame();
            ([rig.number("dx"), rig.number("dy")], ["copy", "objects"].map(|k| rig.flag(k)))
        };
        assert_eq!(open(&mut rig), ([0.0, 0.0], [false, true]), "nothing moved yet");
        rig.cancel();
        // A drag: its offset, and a rotation since doesn't count.
        rig.app.session.begin_interaction("Move").unwrap();
        rig.app.session.preview("object.transform", &json!({"matrix": [1, 0, 0, 1, 20, 5]})).unwrap();
        rig.app.session.commit_interaction().unwrap();
        rig.app.run("object.rotate", json!({"angle": 90})).unwrap();
        rig.app.run("object.rotate", json!({"angle": -90})).unwrap();
        assert_eq!(open(&mut rig).0, [20.0, 5.0]);
        rig.set("dx", json!(30));
        rig.set("dy", json!(0));
        rig.set("copy", json!(true));
        rig.confirm();
        assert_eq!(open(&mut rig), ([30.0, 0.0], [true, true]));
        rig.confirm();
        let kids = rig.app.session.doc().unwrap().doc.layers[0].children().unwrap();
        let lefts: Vec<f64> = kids.iter().map(|n| n.geometric_bounds().unwrap().x0).collect();
        // The drag moved the rectangle from 100 to 120; each copy is 30 pt further.
        assert_eq!(lefts, [120.0, 150.0, 180.0], "a copy at 30 pt each time");
    }

    /// The Move dialog's Transform Patterns is as it was last confirmed with: its OK sets General ›
    /// Transform Pattern Tiles, which it starts from.
    #[test]
    fn move_keeps_transform_patterns_in_the_preference() {
        let mut rig = Rig::new();
        rig.app.run("prefs.set", json!({"key": "transformPatternTiles", "value": false})).unwrap();
        rig.open("object.move");
        assert!(!rig.flag("patterns"));
        rig.set("patterns", json!(true));
        rig.confirm();
        assert!(rig.app.session.prefs.transform_pattern_tiles, "OK wrote the preference");
        rig.open("object.move");
        assert!(rig.flag("patterns"), "as confirmed");
        rig.cancel();
        rig.app.run("prefs.set", json!({"key": "transformPatternTiles", "value": false})).unwrap();
        rig.open("object.move");
        assert!(!rig.flag("patterns"), "the preference");
        rig.cancel();
    }
}
