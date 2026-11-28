//! Object → Expand: what to expand (Object: live shapes, type and effects; Fill: gradient and pattern fills;
//! Stroke: strokes into filled outlines) and what gradient fills become (a gradient mesh, or a
//! number of objects). Options the selection has nothing for are disabled. OK runs
//! `object.expand` as one undo step and keeps its choices for the rest of the app's run: the dialog
//! opens on them, and Object › Expand… chosen with Alt (Option) expands with them at once
//! ([`expand_now`]).
//!
//! Fields: `object`, `fill`, `stroke`, `gradient` (`objects` | `mesh`), `steps` (1..1000), and the
//! dialog-only `__canObject`, `__canFill`, `__canStroke` (that option has something to expand:
//! `object.expand.info`), `__canGradient` (Expand Gradient To applies) and `__canSteps` (it
//! applies and makes objects). What only the art of symbol instances holds needs Object on.

use craft::ui::contract::SaveKey;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::memory::Lifetime;
use serde_json::{Value, json};
use vectorcraft_engine::cmd::expand::{DEFAULT_STEPS, MAX_STEPS};

use crate::VectorcraftApp;

/// The dialog kind of Expand.
pub const KIND: &str = "expand";

const CMD: &str = "object.expand";
const OPENING_DRAWING: &str = "__openingDrawing";

/// Where the choices OK expanded with are kept for the rest of the app's run.
const REMEMBER: SaveKey = SaveKey::new("vectorcraft.dialog.expand");

/// The options and their checkbox labels.
const OPTIONS: [(&str, &str); 3] = [("object", "Object"), ("fill", "Fill"), ("stroke", "Stroke")];

/// Why Object › Expand… is dimmed: the selection has appearance attributes, which Expand Appearance
/// turns into plain art first (#3089).
pub(crate) fn appearance_first() -> &'static str {
    tl!("Expand Appearance first (Object › Expand Appearance)")
}

/// What `object.expand.info` says the selection holds for each option, and which of it only the art of
/// symbol instances holds (Fill and Stroke reach that only when Object expands the instances).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ExpandInfo {
    object: bool,
    fill: bool,
    stroke: bool,
    gradient: bool,
    fill_symbol: bool,
    stroke_symbol: bool,
    gradient_symbol: bool,
}

impl ExpandInfo {
    /// The info for the selection; a selection with appearance attributes refuses with
    /// [`appearance_first`].
    fn read(app: &mut VectorcraftApp) -> Result<Self, String> {
        let info = app.session.execute("object.expand.info", &json!({})).map_err(|e| e.to_string())?;
        if info["appearance"] == json!(true) {
            return Err(appearance_first().into());
        }
        let flag = |v: &Value| *v == json!(true);
        let art = &info["symbolArt"];
        Ok(ExpandInfo {
            object: flag(&info["object"]),
            fill: flag(&info["fill"]),
            stroke: flag(&info["stroke"]),
            gradient: flag(&info["gradient"]),
            fill_symbol: flag(&art["fill"]),
            stroke_symbol: flag(&art["stroke"]),
            gradient_symbol: flag(&art["gradient"]),
        })
    }

    /// Does the selection hold something option `key` expands?
    fn has(self, key: &str) -> bool {
        match key {
            "object" => self.object,
            "fill" => self.fill,
            "stroke" => self.stroke,
            "gradient" => self.gradient,
            _ => false,
        }
    }

    /// Does only the art of symbol instances hold what option `key` expands?
    fn symbol_art_only(self, key: &str) -> bool {
        match key {
            "fill" => self.fill_symbol,
            "stroke" => self.stroke_symbol,
            "gradient" => self.gradient_symbol,
            _ => false,
        }
    }
}

/// The dialog-only field that says whether the row of `key` takes input.
fn can_key(key: &str) -> &'static str {
    match key {
        "object" => "__canObject",
        "fill" => "__canFill",
        "stroke" => "__canStroke",
        "gradient" => "__canGradient",
        _ => "__canSteps",
    }
}

/// Is the switch `key` on? One not set yet is on, as the dialog opens.
fn on(fields: &DialogFields, key: &str) -> bool {
    fields.bool(key).unwrap_or(true)
}

/// Has option `key` something to expand? What only the art of symbol instances holds needs Object on.
fn option_enabled(info: ExpandInfo, fields: &DialogFields, key: &str) -> bool {
    info.has(key) && (key == "object" || on(fields, "object") || !info.symbol_art_only(key))
}

/// Does Expand Gradient To apply? Only gradient fills that Fill expands care.
fn gradient_to_enabled(info: ExpandInfo, fields: &DialogFields) -> bool {
    option_enabled(info, fields, "gradient") && option_enabled(info, fields, "fill") && on(fields, "fill")
}

/// Writes the dialog-only fields the rows read for the values in `fields`.
fn refresh(info: ExpandInfo, fields: &mut DialogFields) {
    for (key, _) in OPTIONS {
        let enabled = option_enabled(info, fields, key);
        fields.set(can_key(key), enabled);
    }
    let gradient_to = gradient_to_enabled(info, fields);
    let steps = gradient_to && fields.text("gradient") != Some("mesh");
    fields.set(can_key("gradient"), gradient_to);
    fields.set(can_key("steps"), steps);
}

/// The rows of the dialog.
fn rows() -> Vec<Param> {
    let mut rows = vec![Param::section(tl!("Expand"))];
    rows.extend(OPTIONS.map(|(key, label)| Param::new(key, label, ParamKind::bool(true)).enabled_when(can_key(key), true)));
    rows.push(Param::section(tl!("Expand Gradient To")));
    let choices = vec![("mesh".to_owned(), tl!("Gradient Mesh").into()), ("objects".to_owned(), tl!("Specify:").into())];
    rows.push(Param::new("gradient", "", ParamKind::radio(choices, Some("objects".to_owned()))).enabled_when(can_key("gradient"), true));
    let steps = ParamKind::int(Some(1), Some(i64::from(MAX_STEPS)), i64::from(DEFAULT_STEPS));
    rows.push(Param::new("steps", tl!("Objects"), steps).enabled_when(can_key("steps"), true));
    rows
}

/// The dialog for a selection with `info`, on the choices OK last expanded with in this run of the app or
/// every option on before that.
fn dialog(app: &VectorcraftApp, info: ExpandInfo) -> Result<HostedParamDialog, String> {
    let ctx = app.ui_ctx.as_ref().ok_or("no window")?;
    let active = app.session.active().ok_or("no document open")?;
    let mut fields = DialogFields::new();
    fields.set(OPENING_DRAWING, FieldValue::Json(json!({"uid": active.uid, "reverts": active.reverts})));
    let rows = rows();
    Ok(HostedParamDialog::new(ctx, KIND, fields, move |fields| {
        refresh(info, fields);
        ParamDialog::new(KIND, tl!("Expand"), rows.clone()).remember(REMEMBER).remember_for(Lifetime::Run)
    }))
}

/// Shows `dialog`; its OK expands.
fn show(app: &mut VectorcraftApp, dialog: HostedParamDialog) -> Result<Value, String> {
    crate::dialogs::host::open_held(app, dialog, |app, result: &ParamResult| {
        let Some(FieldValue::Json(opening)) = result.fields.get(OPENING_DRAWING) else {
            return Err(tl!("Return to the original drawing to expand the artwork").into());
        };
        let active = app
            .session
            .active()
            .filter(|st| Some(st.uid) == opening["uid"].as_u64())
            .ok_or_else(|| tl!("Return to the original drawing to expand the artwork").to_owned())?;
        if Some(active.reverts) != opening["reverts"].as_u64() {
            return Err(tl!("The document was reverted. Reopen this dialog to continue.").into());
        }
        ok(app, result)
    })
    .ok_or("no window")?;
    Ok(Value::Null)
}

/// Open Expand for the selection, on the choices OK last expanded with in this run of the app, or
/// every option on (as `object.expand {}` does) before that. A selection with appearance attributes
/// (effects, several fills or strokes…) refuses with [`appearance_first`]: the menu item is dimmed
/// for it.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let info = ExpandInfo::read(app)?;
    let d = dialog(app, info)?;
    show(app, d)
}

/// Object › Expand… chosen with Alt (Option): expand the selection at once, in one undo step, with
/// the choices OK last expanded with in this run of the app (the dialog's defaults before that), as
/// OK in the dialog opened on them would. When none of those choices has something to expand in the
/// selection, the dialog opens on them instead, and nothing changes. A selection with appearance
/// attributes refuses with [`appearance_first`], as [`open`] does.
pub fn expand_now(app: &mut VectorcraftApp) -> Result<Value, String> {
    let info = ExpandInfo::read(app)?;
    let d = dialog(app, info)?;
    let fields = d.fields();
    if !OPTIONS.iter().any(|(key, _)| on(fields, key) && option_enabled(info, fields, key)) {
        return show(app, d);
    }
    app.run(CMD, params(fields))
}

/// `object.expand` parameters from the fields.
fn params(fields: &DialogFields) -> Value {
    json!({
        "object": on(fields, "object"),
        "fill": on(fields, "fill"),
        "stroke": on(fields, "stroke"),
        "gradient": fields.text("gradient").unwrap_or_default(),
        "steps": fields.number("steps").unwrap_or(f64::from(DEFAULT_STEPS)),
    })
}

/// OK: expand, and keep the choices once the expansion succeeded; a failure keeps the dialog open with
/// the reason.
fn ok(app: &mut VectorcraftApp, result: &ParamResult) -> Result<(), String> {
    app.run(CMD, params(&result.fields))?;
    if let Some(ctx) = &app.ui_ctx {
        result.remember(ctx);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft::ui::dialog::{DialogHost, DialogId, HeldOk, StackDialog as _, dialog_id};
    use vectorcraft_engine::Session;
    use vectorcraft_engine::doc::NodeKind;

    fn app() -> VectorcraftApp {
        crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()))
    }

    fn ctx(app: &VectorcraftApp) -> egui::Context {
        app.ui_ctx.clone().expect("the test context")
    }

    fn host<R>(app: &VectorcraftApp, f: impl FnOnce(&mut DialogHost) -> R) -> R {
        app.dialogs().expect("a window").with(f)
    }

    fn id() -> DialogId {
        dialog_id(KIND)
    }

    fn is_open(app: &VectorcraftApp) -> bool {
        host(app, |h| h.get_by_id::<HostedParamDialog>(id()).is_some())
    }

    /// The values the open Expand dialog shows.
    fn fields(app: &VectorcraftApp) -> DialogFields {
        host(app, |h| h.get_by_id::<HostedParamDialog>(id()).map(|d| d.fields().clone())).expect("the Expand dialog")
    }

    /// The open dialog's `field` set as its row would.
    fn set(app: &VectorcraftApp, field: &str, value: Value) -> Result<(), String> {
        let ctx = ctx(app);
        host(app, |h| h.set_json_field(&ctx, id(), field, value))
    }

    /// OK as a click: the answer waits for the pump, which runs the expansion and answers it.
    fn press_ok(app: &mut VectorcraftApp) -> Option<HeldOk> {
        let ctx = ctx(app);
        host(app, |h| h.confirm_by_id(&ctx, id()));
        crate::dialogs::host::pump(app, &ctx);
        host(app, |h| h.get_by_id::<HostedParamDialog>(id()).and_then(|d| d.held_state()))
    }

    /// Takes the dialog off the host, as the pass that closes an accepted one does: a trial draws no pass.
    fn close(app: &VectorcraftApp) {
        let ctx = ctx(app);
        host(app, |h| h.cancel_by_id(&ctx, id()));
    }

    /// OK that the expansion accepts, and the dialog closing.
    fn ok_and_close(app: &mut VectorcraftApp) {
        assert!(matches!(press_ok(app), Some(HeldOk::Accepted)), "OK expands");
        close(app);
    }

    #[test]
    fn opens_with_what_applies_and_ok_expands_the_gradient_to_a_mesh() {
        let mut app = app();
        gradient_rectangle(&mut app);
        // The menu item (and `object.expand` without params) opens the dialog.
        crate::menus::invoke(&mut app, "object.expand", json!({}));
        let shown = fields(&app);
        assert_eq!(
            (shown.bool("__canObject"), shown.bool("__canFill"), shown.bool("__canStroke")),
            (Some(true), Some(true), Some(false)),
            "no stroke to expand"
        );
        assert_eq!(params(&shown), json!({"object": true, "fill": true, "stroke": true, "gradient": "objects", "steps": 255.0}));
        let undo = app.session.doc().unwrap().history.undo.len();
        set(&app, "gradient", json!("mesh")).unwrap();
        ok_and_close(&mut app);
        let st = app.session.doc().unwrap();
        assert_eq!(st.history.undo.len(), undo + 1);
        let g = st.doc.node(st.selection.objects[0]).unwrap();
        let NodeKind::Group { children, clip: true } = &g.kind else { panic!("a clip group: {:?}", g.kind) };
        assert!(matches!(children[1].kind, NodeKind::Mesh(_)));
    }

    #[test]
    fn expand_keeps_its_draft_when_another_drawing_is_active() {
        let mut app = app();
        gradient_rectangle(&mut app);
        open(&mut app).unwrap();
        set(&app, "gradient", json!("mesh")).unwrap();
        gradient_rectangle(&mut app);
        let undo = app.session.active().unwrap().history.undo.len();
        let before = app.session.active().unwrap().doc.clone();
        assert!(matches!(press_ok(&mut app), Some(HeldOk::Refused(reason)) if reason.as_str().contains("original drawing")));
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
        assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &before));
        assert_eq!(fields(&app).text("gradient"), Some("mesh"));
        app.run("document.activate", json!({"index": 0})).unwrap();
        ok_and_close(&mut app);
        assert!(expanded_to_a_mesh(&app));
    }

    #[test]
    fn expand_gradient_to_is_disabled_for_a_pattern_fill_alone() {
        let mut app = app();
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let dot = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
        app.run("object.pattern.make", json!({"ids": [dot], "name": "Dots", "width": 20, "height": 20, "edit": false})).unwrap();
        app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 100, "height": 60})).unwrap();
        app.run("paint.setFill", json!({"swatch": "Dots"})).unwrap();
        app.run("ui.expandDialog", json!({})).unwrap();
        let info = ExpandInfo::read(&mut app).unwrap();
        assert_eq!(fields(&app).bool("__canFill"), Some(true), "Fill expands the pattern fill");
        assert!(!gradient_to_enabled(info, &fields(&app)), "no gradient fill for Expand Gradient To");
        // A gradient fill enables it.
        app.run("paint.setFill", json!({"gradient": {"kind": "linear"}})).unwrap();
        app.run("ui.expandDialog", json!({})).unwrap();
        let info = ExpandInfo::read(&mut app).unwrap();
        assert!(gradient_to_enabled(info, &fields(&app)));
    }

    /// Stroke for the stroke in a symbol instance's art is enabled only while Object is on, since
    /// Object expands the instance.
    #[test]
    fn stroke_for_a_symbol_instances_art_needs_object() {
        let mut app = app();
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 100, "height": 60})).unwrap();
        app.run("paint.setFill", json!({"none": true})).unwrap();
        app.run("paint.setStroke", json!({"color": "#000000"})).unwrap();
        app.run("symbol.new", json!({"name": "Frame"})).unwrap();
        app.run("ui.expandDialog", json!({})).unwrap();
        let info = ExpandInfo::read(&mut app).unwrap();
        let shown = fields(&app);
        assert!(option_enabled(info, &shown, "object") && option_enabled(info, &shown, "stroke"));
        set(&app, "object", json!(false)).unwrap();
        assert!(!option_enabled(info, &fields(&app), "stroke"), "Stroke can't reach the art with Object off");
    }

    /// A line with a stroke and a Zig Zag effect is selected (#3089).
    fn zig_zag_line(app: &mut VectorcraftApp) {
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("path.create", json!({"d": "M 20 100 L 180 100"})).unwrap();
        app.run("paint.setFill", json!({"none": true})).unwrap();
        app.run("paint.setStroke", json!({"color": "#000000"})).unwrap();
        app.run("effect.apply", json!({"effect": "distort.zigZag", "params": {"size": 10, "ridges": 6, "relative": false}})).unwrap();
    }

    #[test]
    fn expand_waits_for_expand_appearance_on_a_line_with_an_effect() {
        let mut app = app();
        zig_zag_line(&mut app);
        assert!(!crate::menus::enabled(&app, "ui.expandDialog"), "Expand… is dimmed for the Zig Zag effect");
        assert!(crate::menus::enabled(&app, "effect.expandAppearance"));
        // A shortcut or the command palette doesn't open it either; the status bar says why.
        let before = app.session.doc().unwrap().doc.clone();
        crate::menus::invoke(&mut app, "object.expand", json!({}));
        assert!(!app.any_dialog_open());
        assert!(!is_open(&app));
        assert_eq!(app.ui.status, appearance_first());
        assert_eq!(app.run("ui.expandDialog", json!({})).unwrap_err(), appearance_first());
        assert_eq!(app.session.doc().unwrap().doc, before, "the line is left as it was");
        // Expand Appearance first: the line stays a stroked line, and Expand… is available for it.
        crate::menus::invoke(&mut app, "effect.expandAppearance", json!({}));
        let st = app.session.doc().unwrap();
        let line = st.doc.node(st.selection.objects[0]).unwrap();
        assert!(line.appearance.effects.is_empty() && !line.appearance.stroke_paint().is_none());
        assert!(crate::menus::enabled(&app, "ui.expandDialog"));
        crate::menus::invoke(&mut app, "object.expand", json!({}));
        assert!(is_open(&app));
    }

    #[test]
    fn expand_is_dimmed_for_two_fills_but_not_for_a_plain_rectangle() {
        let mut app = app();
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 100, "height": 60})).unwrap();
        assert!(crate::menus::enabled(&app, "ui.expandDialog"), "a basic appearance");
        app.run("appearance.addFill", json!({})).unwrap();
        assert!(!crate::menus::enabled(&app, "ui.expandDialog"), "two fills");
        // Nothing selected: dimmed as before.
        app.run("select.none", json!({})).unwrap();
        assert!(!crate::menus::enabled(&app, "ui.expandDialog"));
    }

    /// A rectangle with a linear gradient fill and no stroke is selected.
    fn gradient_rectangle(app: &mut VectorcraftApp) {
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 100, "height": 60})).unwrap();
        app.run("paint.setFill", json!({"gradient": {"kind": "linear"}})).unwrap();
        app.run("paint.setStroke", json!({"none": true})).unwrap();
    }

    /// Object › Expand… with Alt held or not: the Alt entry runs `ui.expandDialog` with `immediate`.
    fn choose_expand(app: &mut VectorcraftApp, alt: bool) {
        let params = if alt { json!({"immediate": true}) } else { json!({}) };
        app.run("ui.expandDialog", params).unwrap();
    }

    /// Whether the selected object is a clip group holding a gradient mesh (Expand Gradient To ›
    /// Gradient Mesh) rather than solid objects; a stroke left as it was is grouped beside that clip group.
    fn expanded_to_a_mesh(app: &VectorcraftApp) -> bool {
        let st = app.session.doc().unwrap();
        let mut g = st.doc.node(st.selection.objects[0]).unwrap();
        if let NodeKind::Group { children, clip: false } = &g.kind
            && let [fill, stroke] = children.as_slice()
            && matches!(stroke.kind, NodeKind::Path { .. })
        {
            g = fill;
        }
        let NodeKind::Group { children, clip: true } = &g.kind else { panic!("a clip group: {:?}", g.kind) };
        children.iter().any(|c| matches!(c.kind, NodeKind::Mesh(_)))
    }

    #[test]
    fn the_dialog_opens_on_the_choices_ok_last_expanded_with() {
        let mut app = app();
        gradient_rectangle(&mut app);
        // A stroke enables Stroke, and Objects takes a count while Expand Gradient To is Specify.
        app.run("paint.setStroke", json!({"color": "#000000"})).unwrap();
        choose_expand(&mut app, false);
        set(&app, "steps", json!(12)).unwrap();
        set(&app, "gradient", json!("mesh")).unwrap();
        set(&app, "stroke", json!(false)).unwrap();
        ok_and_close(&mut app);
        assert!(expanded_to_a_mesh(&app));
        app.run("edit.undo", json!({})).unwrap();
        let kept = json!({"object": true, "fill": true, "stroke": false, "gradient": "mesh", "steps": 12.0});
        choose_expand(&mut app, false);
        assert_eq!(params(&fields(&app)), kept, "it opens on the choices OK expanded with");
        // Cancel leaves them as they were.
        set(&app, "gradient", json!("objects")).unwrap();
        close(&app);
        choose_expand(&mut app, false);
        assert_eq!(params(&fields(&app)), kept);
        // A count the row refuses is not taken.
        assert!(set(&app, "steps", json!(5000)).is_err());
        assert_eq!(params(&fields(&app)), kept);
        // An OK that fails keeps the dialog open with the reason, and nothing changes.
        app.run("select.none", json!({})).unwrap();
        let before = app.session.doc().unwrap().doc.clone();
        assert!(matches!(press_ok(&mut app), Some(HeldOk::Refused(_))), "the expansion has no selection to work on");
        assert!(is_open(&app));
        assert_eq!(app.session.doc().unwrap().doc, before);
        close(&app);
        app.run("select.all", json!({})).unwrap();
        choose_expand(&mut app, false);
        assert_eq!(params(&fields(&app)), kept);
    }

    #[test]
    fn alt_expands_at_once_with_the_choices_ok_last_expanded_with() {
        let mut app = app();
        gradient_rectangle(&mut app);
        choose_expand(&mut app, false);
        set(&app, "gradient", json!("mesh")).unwrap();
        ok_and_close(&mut app);
        app.run("edit.undo", json!({})).unwrap();
        let undo = app.session.doc().unwrap().history.undo.len();
        choose_expand(&mut app, true);
        assert!(!is_open(&app), "no dialog");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
        assert!(expanded_to_a_mesh(&app), "the kept Gradient Mesh choice");
    }

    #[test]
    fn alt_before_any_ok_expands_with_the_defaults() {
        let mut app = app();
        gradient_rectangle(&mut app);
        let undo = app.session.doc().unwrap().history.undo.len();
        choose_expand(&mut app, true);
        assert!(!is_open(&app), "no dialog");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
        assert!(!expanded_to_a_mesh(&app), "the gradient becomes objects, the default");
        app.run("edit.undo", json!({})).unwrap();
        choose_expand(&mut app, false);
        assert_eq!(
            params(&fields(&app)),
            json!({"object": true, "fill": true, "stroke": true, "gradient": "objects", "steps": 255.0}),
            "Alt keeps no choices of its own"
        );
    }

    #[test]
    fn alt_opens_the_dialog_when_the_kept_choices_expand_nothing_in_the_selection() {
        let mut app = app();
        gradient_rectangle(&mut app);
        // Fill alone is kept by a real OK; a stroke enables Stroke, to be turned off.
        app.run("paint.setStroke", json!({"color": "#000000"})).unwrap();
        choose_expand(&mut app, false);
        set(&app, "object", json!(false)).unwrap();
        set(&app, "stroke", json!(false)).unwrap();
        ok_and_close(&mut app);
        // A plain rectangle has no gradient or pattern fill.
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 100, "height": 60})).unwrap();
        // A new object takes the gradient fill set last; this one gets a plain colour.
        app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
        let before = app.session.doc().unwrap().doc.clone();
        let undo = app.session.doc().unwrap().history.undo.len();
        choose_expand(&mut app, true);
        assert!(is_open(&app), "the Expand dialog");
        assert_eq!(
            params(&fields(&app)),
            json!({"object": false, "fill": true, "stroke": false, "gradient": "objects", "steps": 255.0}),
            "it opens on the kept choices"
        );
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo, "nothing recorded");
        assert_eq!(app.session.doc().unwrap().doc, before);
    }

    #[test]
    fn alt_refuses_a_selection_with_appearance_attributes_and_a_bad_immediate() {
        let mut app = app();
        zig_zag_line(&mut app);
        let before = app.session.doc().unwrap().doc.clone();
        assert_eq!(app.run("ui.expandDialog", json!({"immediate": true})).unwrap_err(), appearance_first());
        assert!(!is_open(&app));
        assert_eq!(app.session.doc().unwrap().doc, before, "the line is left as it was");
        app.run("effect.expandAppearance", json!({})).unwrap();
        assert!(app.run("ui.expandDialog", json!({"immediate": "yes"})).is_err());
        assert!(!is_open(&app));
        app.run("ui.expandDialog", json!({"immediate": false})).unwrap();
        assert!(is_open(&app));
    }

    #[test]
    fn expand_approval_does_not_expand_artwork_restored_by_revert() {
        let mut app = app();
        gradient_rectangle(&mut app);
        let rectangle = app.session.active().unwrap().selection.objects[0];
        let dir = vectorcraft_testkit::TempDir::new("expand-revert");
        app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|e| e.to_string())));
        app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string())));
        app.run("file.saveAs", json!({"path": dir.join("Art.vectorcraft")})).unwrap();
        app.run("object.move", json!({"dx": 10, "dy": 0})).unwrap();
        open(&mut app).unwrap();
        set(&app, "gradient", json!("mesh")).unwrap();

        app.run("file.revert", json!({"confirmed": true})).unwrap();
        app.run("select.set", json!({"ids": [rectangle.0]})).unwrap();
        let restored = app.session.active().unwrap().doc.clone();
        assert!(matches!(press_ok(&mut app), Some(HeldOk::Refused(_))));
        assert_eq!(app.session.active().unwrap().doc, restored);

        close(&app);
        open(&mut app).unwrap();
        set(&app, "gradient", json!("mesh")).unwrap();
        ok_and_close(&mut app);
        let st = app.session.active().unwrap();
        let art = st.doc.node(st.selection.objects[0]).unwrap();
        let NodeKind::Group { children, clip: true } = &art.kind else { panic!("expected a clip group") };
        assert!(matches!(children[1].kind, NodeKind::Mesh(_)));
    }
}
