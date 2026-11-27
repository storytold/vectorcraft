//! Object → Blend → Blend Options (also the Blend tool's double-click, Alt-click and toolbar
//! button): Spacing (Smooth Color, Specified Steps or Specified Distance) with its value and
//! Orientation (Align to Page or Align to Path), opened on the selected blend's options and
//! previewed live on it; OK keeps the preview as one undo step (`object.blend.options`). With no
//! blend selected it sets the options new blends start with.
//!
//! Fields: `spacing` (`smooth` | `steps` | `distance`), `steps` (1..1000), `distance` (pt),
//! `orientation` (`page` | `path`), `preview`, and `__target` (`blend` or `defaults`, from
//! `object.blend.info`).

use craft::behavior::IconId;
use craft::ui::choice::Segmented;
use std::sync::{Arc, Weak};

use craft::ui::contract::{Bind, Entry, Show as _};
use craft::ui::dialog::dialog_id;
use craft::ui::patterns::form_dialogs::{CustomContext, DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::LengthUnits;
use serde_json::{Value, json};
use vectorcraft_doc::Selection;

use super::host::{HeldOptions, open_held_with};
use super::preview::LivePreview; // PLAN: assumed path of wg.A20's `LivePreview`
use crate::VectorcraftApp;
use crate::canvas::general_lengths;

/// The dialog kind of Blend Options.
pub const KIND: &str = "blendOptions";

const CMD: &str = "object.blend.options";

/// The label of the undo step the live preview records.
const PREVIEW_LABEL: &str = "Blend Options";

/// Spacing modes: (value, label).
pub(crate) const SPACING: [(&str, &str); 3] = [("smooth", "Smooth Color"), ("steps", "Specified Steps"), ("distance", "Specified Distance")];

pub(crate) struct Open {
    opening: Arc<()>,
    document: (u64, u64),
    preview: LivePreview,
    preview_selection: Option<Weak<Selection>>,
}

impl Open {
    fn drawing_problem(&self, app: &VectorcraftApp) -> Option<&'static str> {
        let (uid, reverts) = self.document;
        match app.session.active() {
            Some(st) if (st.uid, st.reverts) == (uid, reverts) => None,
            Some(st) if st.uid == uid => Some(tl!("The document was reverted. Reopen this dialog to continue.")),
            _ => Some(tl!("Return to the original drawing to apply Blend Options.")),
        }
    }

    fn selection_snapshot(app: &VectorcraftApp) -> Option<&Arc<Selection>> {
        app.session.active().and_then(|st| st.interaction.as_ref().map(|it| &it.selection))
    }

    fn owns_preview(&self, app: &VectorcraftApp) -> bool {
        self.drawing_problem(app).is_none()
            && self
                .preview_selection
                .as_ref()
                .zip(Self::selection_snapshot(app))
                .is_some_and(|(source, current)| source.as_ptr() == Arc::as_ptr(current))
    }

    fn take_preview(&mut self) -> LivePreview {
        self.preview_selection = None;
        std::mem::take(&mut self.preview)
    }

    fn cancel(self, app: &mut VectorcraftApp) {
        if self.owns_preview(app) {
            self.preview.cancel(app);
        }
    }
}

/// Open Blend Options on the selected blend's options (else those new blends start with).
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    if let Some(retired) = app.object_dialogs.blend.take() {
        retired.cancel(app);
    }
    let mut info = app.session.execute("object.blend.info", &json!({})).map_err(|e| e.to_string())?;
    let target = info["target"].as_str().unwrap_or("defaults").to_owned();
    let Some(o) = info.as_object_mut() else { return Err("no blend options".into()) };
    o.retain(|k, _| matches!(k.as_str(), "spacing" | "steps" | "distance" | "orientation"));
    let mut fields = DialogFields::new();
    fields.set("spacing", FieldValue::Text(o.get("spacing").and_then(Value::as_str).unwrap_or("smooth").to_owned()));
    fields.set("steps", FieldValue::Int(o.get("steps").and_then(Value::as_f64).map_or(5, |n| n.round() as i64)));
    fields.set("distance", FieldValue::Number(o.get("distance").and_then(Value::as_f64).unwrap_or(10.0)));
    fields.set("orientation", FieldValue::Json(o.get("orientation").cloned().unwrap_or_else(|| json!("page"))));
    fields.set("__opened", FieldValue::Json(Value::Object(o.clone())));
    fields.set("__target", FieldValue::Text(target.clone()));
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let lengths = general_lengths(app);
    // Only a blend previews: the defaults target has nothing on the canvas to show.
    let on_blend = target == "blend";
    let opening = Arc::new(());
    let cancelled = Arc::clone(&opening);
    let document = app.session.active().map(|st| (st.uid, st.reverts));
    let dialog = HostedParamDialog::new(&ctx, KIND, fields, move |fields| dialog(on_blend, lengths).values(Bind::bound(fields)));
    let cancel: Box<dyn FnOnce(&mut VectorcraftApp)> = Box::new(move |app| {
        if app.object_dialogs.blend.as_ref().is_some_and(|open| Arc::ptr_eq(&open.opening, &cancelled))
            && let Some(open) = app.object_dialogs.blend.take()
        {
            open.cancel(app);
        }
    });
    if open_held_with(app, dialog, ok, HeldOptions { cancel: Some(cancel), ..Default::default() }).is_none() {
        return Err("no window".into());
    }
    if on_blend && let Some(document) = document {
        app.object_dialogs.blend = Some(Open { opening, document, preview: LivePreview::default(), preview_selection: None });
    }
    Ok(Value::Null)
}

/// The dialog over its rows; Steps shows (disabled for Smooth Color) with the two modes that have a count of steps.
fn dialog<'a>(on_blend: bool, lengths: LengthUnits) -> ParamDialog<'a> {
    let spacing = SPACING.iter().map(|(value, label)| ((*value).to_owned(), tl!(label).into())).collect();
    let params = vec![
        Param::new("spacing", tl!("Spacing:"), ParamKind::choice(spacing, None)),
        Param::new("steps", tl!("Steps"), ParamKind::int(Some(1), Some(1000), 5))
            .enabled_when("spacing", "steps")
            .visible_when_in("spacing", ["smooth", "steps"]),
        Param::number("distance", tl!("Distance"), 10.0, 1.0, 3).length(lengths).bounds(Some(0.001), None).visible_when("spacing", "distance"),
        Param::custom("orientation", tl!("Orientation:"), orientation_row),
    ];
    ParamDialog::new(KIND, tl!("Blend Options"), params).previews(on_blend)
}

/// The orientation the fields hold: `page` unless it is `path`.
fn orientation(fields: &DialogFields) -> &'static str {
    if matches!(fields.get("orientation"), Some(FieldValue::Json(Value::String(s))) if s == "path") { "path" } else { "page" }
}

/// The Align to Page / Align to Path buttons.
fn orientation_row(ui: &mut egui::Ui, fields: &mut DialogFields, _cx: &mut CustomContext) -> egui::Response {
    let entries = [
        Entry::new("page", tl!("Align to Page")).icon(IconId::new("dc-blend-align-page")),
        Entry::new("path", tl!("Align to Path")).icon(IconId::new("dc-blend-align-path")),
    ];
    let shown = orientation(fields);
    let mut value = shown;
    let response = Segmented::new(tl!("Orientation:"), Bind::bound(&mut value), &entries).show(ui).response;
    if value != shown {
        fields.set("orientation", FieldValue::Json(json!(value)));
    }
    response
}

/// `object.blend.options` parameters from the fields differing from the values the dialog opened with,
/// so each selected blend keeps its own values for the rest. Spacing and its count go together.
fn params(fields: &DialogFields) -> Value {
    let opened = |k: &str| match fields.get("__opened") {
        Some(FieldValue::Json(o)) => o.get(k).cloned().unwrap_or(Value::Null),
        _ => Value::Null,
    };
    let mode = fields.text("spacing").unwrap_or_default();
    let count = match mode {
        "steps" => Some(("steps", fields.int("steps").unwrap_or(5).clamp(1, 1000) as f64)),
        "distance" => Some(("distance", fields.number("distance").unwrap_or(10.0))),
        _ => None,
    };
    let mut p = json!({});
    if opened("spacing") != json!(mode) || count.is_some_and(|(k, v)| opened(k).as_f64() != Some(v)) {
        p["spacing"] = json!(mode);
        if let Some((k, v)) = count {
            p[k] = json!(v);
        }
    }
    let orientation = orientation(fields);
    if opened("orientation") != json!(orientation) {
        p["orientation"] = json!(orientation);
    }
    p
}

/// Runs the live preview of the open dialog while its Preview box is on.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(mut open) = app.object_dialogs.blend.take() else { return };
    let state = app
        .dialogs()
        .and_then(|d| d.with(|host| host.get_by_id::<HostedParamDialog>(dialog_id(KIND)).map(|d| (d.is_previewing(), params(d.fields())))));
    let owns_preview = open.owns_preview(app);
    if let Some((previewing, p)) = state
        && open.drawing_problem(app).is_none()
        && (!app.session.in_interaction() || owns_preview)
    {
        if !owns_preview {
            open.take_preview();
        }
        // With no blend selected, the command sets application defaults instead of previewing artwork.
        let on = previewing && app.session.has_selected_blend();
        let ran = open.preview.run(app, on, PREVIEW_LABEL, CMD, &p).is_some();
        if !on {
            open.preview_selection = None;
        } else if ran {
            open.preview_selection = Open::selection_snapshot(app).map(Arc::downgrade);
        }
    }
    app.object_dialogs.blend = Some(open);
}

/// OK: keeps the preview as one undo step on a blend; sets the defaults otherwise.
fn ok(app: &mut VectorcraftApp, result: &ParamResult) -> Result<(), String> {
    let on_blend = result.fields.text("__target") == Some("blend");
    if on_blend {
        let open = app.object_dialogs.blend.as_ref().ok_or("Blend Options is no longer open")?;
        if let Some(reason) = open.drawing_problem(app) {
            return Err(reason.to_owned());
        }
        if app.session.in_interaction() && !open.owns_preview(app) {
            return Err(tl!("Finish the current edit before applying Blend Options.").into());
        }
    }
    if on_blend != app.session.has_selected_blend() {
        let reason = if on_blend {
            tl!("Select a blend before applying these options.")
        } else {
            tl!("Deselect any selected blends before applying the options for new blends.")
        };
        return Err(reason.to_owned());
    }
    let p = params(&result.fields);
    if !on_blend {
        return app.run(CMD, p).map(|_| ());
    }
    let mut open = app.object_dialogs.blend.take().ok_or("Blend Options is no longer open")?;
    let owned = open.owns_preview(app);
    let preview = open.take_preview();
    let preview = if owned { preview } else { LivePreview::default() };
    preview.commit(app, CMD, p).map(|_| ()).inspect_err(|_| {
        // The dialog stays open on the refusal and can start a new preview.
        app.object_dialogs.blend = Some(open);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft::ui::dialog::DialogHost;
    use vectorcraft_doc::NodeKind;
    use vectorcraft_doc::live::{BlendOrientation, BlendSpacing};
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app = crate::testing::windowed(app); // PLAN: assumed path of sh.SH4
        app
    }

    /// One app frame: the dialog pump runs and the dialogs draw, so an accepted OK closes its dialog.
    fn frame(app: &mut VectorcraftApp) {
        let ctx = app.ui_ctx.clone().unwrap();
        crate::testing::app_frame(app, &ctx, egui::RawInput::default());
    }

    fn with<R>(app: &VectorcraftApp, f: impl FnOnce(&mut DialogHost) -> R) -> R {
        app.dialogs().unwrap().with(f)
    }

    fn fields(app: &VectorcraftApp) -> DialogFields {
        with(app, |h| h.get_by_id::<HostedParamDialog>(dialog_id(KIND)).map(|d| d.fields().clone())).expect("Blend Options is open")
    }

    fn is_open(app: &VectorcraftApp) -> bool {
        with(app, |h| h.get_by_id::<HostedParamDialog>(dialog_id(KIND)).is_some())
    }

    fn set(app: &VectorcraftApp, key: &str, value: Value) {
        let ctx = app.ui_ctx.clone().unwrap();
        with(app, |h| h.set_json_field(&ctx, dialog_id(KIND), key, value)).unwrap();
    }

    /// Presses OK; the pump then runs the held answer.
    fn confirm(app: &mut VectorcraftApp) {
        let ctx = app.ui_ctx.clone().unwrap();
        with(app, |h| h.confirm_by_id(&ctx, dialog_id(KIND)));
        frame(app);
        frame(app);
    }

    fn cancel(app: &mut VectorcraftApp) {
        let ctx = app.ui_ctx.clone().unwrap();
        with(app, |h| h.cancel_by_id(&ctx, dialog_id(KIND)));
        frame(app);
    }

    fn spec(app: &VectorcraftApp, id: u64) -> vectorcraft_doc::BlendSpec {
        match &app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
            NodeKind::Blend { spec, .. } => (**spec).clone(),
            _ => panic!("not a blend"),
        }
    }

    fn app_with_smooth_blend() -> (VectorcraftApp, u64) {
        let mut app = app();
        let id = add_blend_drawing(&mut app);
        (app, id)
    }

    fn add_blend_drawing(app: &mut VectorcraftApp) -> u64 {
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        for x in [0, 200] {
            app.run("shape.rectangle", json!({"x": x, "y": 0, "width": 20, "height": 20})).unwrap();
        }
        app.run("select.all", json!({})).unwrap();
        app.run("object.blend.make", json!({"smooth": true})).unwrap()["id"].as_u64().unwrap()
    }

    #[test]
    fn opening_blend_defaults_does_not_edit_a_later_selected_blend() {
        let (mut app, id) = app_with_smooth_blend();
        app.run("select.none", json!({})).unwrap();
        app.run("ui.blendOptions", json!({})).unwrap();
        assert_eq!(fields(&app).text("__target"), Some("defaults"));
        set(&app, "spacing", json!("steps"));
        set(&app, "steps", json!(7));
        app.run("select.set", json!({"ids": [id]})).unwrap();

        let undo = app.session.active().unwrap().history.undo.len();
        confirm(&mut app);
        assert_eq!(spec(&app, id).spacing, BlendSpacing::SmoothColor, "a newly selected blend keeps its spacing");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
        assert!(is_open(&app));

        app.run("select.none", json!({})).unwrap();
        confirm(&mut app);
        assert!(!is_open(&app));
        assert_eq!(app.session.prefs.blend_options.unwrap().spacing, BlendSpacing::Steps(7));
    }

    #[test]
    fn opening_blend_options_for_a_selection_does_not_write_defaults() {
        let (mut app, id) = app_with_smooth_blend();
        let defaults = app.session.prefs.blend_options;
        app.run("ui.blendOptions", json!({})).unwrap();
        assert_eq!(fields(&app).text("__target"), Some("blend"));
        set(&app, "spacing", json!("steps"));
        set(&app, "steps", json!(7));
        app.run("select.none", json!({})).unwrap();

        frame(&mut app);
        assert_eq!(app.session.prefs.blend_options, defaults, "the blend preview leaves new-blend defaults intact");
        assert!(!app.session.in_interaction());
        confirm(&mut app);
        assert_eq!(app.session.prefs.blend_options, defaults);
        assert!(is_open(&app));

        app.run("select.set", json!({"ids": [id]})).unwrap();
        confirm(&mut app);
        assert!(!is_open(&app));
        assert_eq!(spec(&app, id).spacing, BlendSpacing::Steps(7));
    }

    #[test]
    fn opens_on_the_blend_previews_and_keeps_smooth_color() {
        let mut app = app();
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
        app.run("shape.rectangle", json!({"x": 200, "y": 0, "width": 20, "height": 20})).unwrap();
        app.run("select.all", json!({})).unwrap();
        let id = app.run("object.blend.make", json!({"smooth": true})).unwrap()["id"].as_u64().unwrap();
        // The menu item opens the dialog on the blend's own options.
        crate::menus::invoke(&mut app, CMD, json!({}));
        assert!(is_open(&app));
        frame(&mut app);
        assert_eq!(fields(&app).text("spacing"), Some("smooth"));
        // OK without changes keeps Smooth Color (it used to become 5 steps).
        confirm(&mut app);
        assert_eq!(spec(&app, id).spacing, BlendSpacing::SmoothColor);
        // Specified Steps and Align to Path preview on the blend; Cancel rolls them back.
        app.run("ui.blendOptions", json!({})).unwrap();
        set(&app, "spacing", json!("steps"));
        set(&app, "steps", json!(3));
        set(&app, "orientation", json!("path"));
        frame(&mut app);
        assert_eq!(spec(&app, id).spacing, BlendSpacing::Steps(3), "previewed");
        cancel(&mut app);
        assert_eq!(spec(&app, id).spacing, BlendSpacing::SmoothColor);
        // OK keeps them as one undo step.
        app.run("ui.blendOptions", json!({})).unwrap();
        set(&app, "spacing", json!("distance"));
        set(&app, "distance", json!(25));
        frame(&mut app);
        let depth = app.session.doc().unwrap().history.undo.len();
        confirm(&mut app);
        assert!(!is_open(&app));
        assert_eq!(spec(&app, id).spacing, BlendSpacing::Distance(25.0));
        assert_eq!(app.session.doc().unwrap().history.undo.len(), depth + 1);
    }

    #[test]
    fn on_several_blends_ok_sets_only_the_fields_changed() {
        let mut app = app();
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        let blend = |app: &mut VectorcraftApp, y: f64, options: Value| {
            let ids: Vec<Value> =
                [0.0, 200.0].map(|x| app.run("shape.rectangle", json!({"x": x, "y": y, "width": 20, "height": 20})).unwrap()["id"].clone()).into();
            app.run("select.set", json!({ "ids": ids })).unwrap();
            app.run("object.blend.make", options).unwrap()["id"].as_u64().unwrap()
        };
        let a = blend(&mut app, 0.0, json!({"spacing": "steps", "steps": 3}));
        let b = blend(&mut app, 100.0, json!({"spacing": "distance", "distance": 12, "orientation": "path"}));
        let both = |app: &VectorcraftApp| {
            let (sa, sb) = (spec(app, a), spec(app, b));
            ((sa.spacing, sa.orientation), (sb.spacing, sb.orientation))
        };
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        // The dialog shows the first blend's options; changing only the orientation leaves each
        // blend's spacing as it was.
        app.run("ui.blendOptions", json!({})).unwrap();
        set(&app, "orientation", json!("path"));
        frame(&mut app);
        confirm(&mut app);
        let path = BlendOrientation::AlignToPath;
        assert_eq!(both(&app), ((BlendSpacing::Steps(3), path), (BlendSpacing::Distance(12.0), path)));
        // Changing only the step count gives both blends that count and keeps their orientation.
        app.run("object.blend.options", json!({"id": b, "orientation": "page"})).unwrap();
        app.run("ui.blendOptions", json!({})).unwrap();
        set(&app, "steps", json!(8));
        frame(&mut app);
        confirm(&mut app);
        assert_eq!(both(&app), ((BlendSpacing::Steps(8), path), (BlendSpacing::Steps(8), BlendOrientation::AlignToPage)));
        // An orientation set back to the opened value sends nothing: each blend keeps its own.
        app.run("ui.blendOptions", json!({})).unwrap();
        set(&app, "orientation", json!("path"));
        frame(&mut app);
        confirm(&mut app);
        assert_eq!(both(&app), ((BlendSpacing::Steps(8), path), (BlendSpacing::Steps(8), BlendOrientation::AlignToPage)));
        // OK without changes changes nothing and records no undo step.
        let depth = app.session.doc().unwrap().history.undo.len();
        app.run("ui.blendOptions", json!({})).unwrap();
        frame(&mut app);
        confirm(&mut app);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), depth);
    }

    #[test]
    fn with_nothing_selected_it_sets_what_new_blends_start_with() {
        let mut app = app();
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("ui.blendOptions", json!({})).unwrap();
        assert_eq!(fields(&app).text("__target"), Some("defaults"));
        set(&app, "spacing", json!("steps"));
        set(&app, "steps", json!(7));
        set(&app, "orientation", json!("path"));
        frame(&mut app);
        confirm(&mut app);
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
        app.run("shape.rectangle", json!({"x": 200, "y": 0, "width": 20, "height": 20})).unwrap();
        app.run("select.all", json!({})).unwrap();
        let id = app.run("object.blend.make", json!({})).unwrap()["id"].as_u64().unwrap();
        let s = spec(&app, id);
        assert_eq!((s.spacing, s.orientation), (BlendSpacing::Steps(7), BlendOrientation::AlignToPath));
    }

    #[test]
    fn blend_step_count_keeps_a_live_keyboard_edit() {
        use craft::ui::testing::Fixture;
        use std::cell::RefCell;
        use std::rc::Rc;

        let (mut app, blend) = app_with_smooth_blend();
        app.run("object.blend.options", json!({"id": blend, "spacing": "steps", "steps": 7})).unwrap();
        let app = Rc::new(RefCell::new(app));
        let shown = app.clone();
        let mut fixture = Fixture::new(move |ui| {
            let mut app = shown.borrow_mut();
            let dialogs = app.dialogs().unwrap();
            dialogs.with(|host| host.run(ui));
            super::super::host::pump(&mut app, ui.ctx());
        });
        fixture.room(egui::vec2(880.0, 720.0));
        {
            let mut app = app.borrow_mut();
            app.ui_ctx = Some(fixture.ctx().clone());
            open(&mut app).unwrap();
        }
        fixture.settle();
        assert_eq!(fields(&app.borrow()).int("steps"), Some(7), "the dialog opens with the blend's existing step count");
        let steps = fixture.by_label("Steps").expect("the enabled Steps field");
        fixture.click(&steps).key(egui::Key::A, egui::Modifiers::COMMAND).type_text("9").key(egui::Key::Tab, egui::Modifiers::NONE).settle();
        {
            let mut app = app.borrow_mut();
            assert_eq!(fields(&app).int("steps"), Some(9), "the hosted form keeps its keyboard edit");
            confirm(&mut app);
            assert_eq!(spec(&app, blend).spacing, BlendSpacing::Steps(9));
        }
    }

    #[test]
    fn blend_options_preview_and_cancel_leave_another_drawings_blend_alone() {
        let (mut app, original) = app_with_smooth_blend();
        let second = add_blend_drawing(&mut app);
        app.run("document.activate", json!({"index": 0})).unwrap();
        open(&mut app).unwrap();
        set(&app, "spacing", json!("steps"));
        set(&app, "steps", json!(7));
        frame(&mut app);
        assert_eq!(spec(&app, original).spacing, BlendSpacing::Steps(7));

        app.run("document.activate", json!({"index": 1})).unwrap();
        frame(&mut app);
        assert_eq!(spec(&app, second).spacing, BlendSpacing::SmoothColor);
        confirm(&mut app);
        assert!(is_open(&app));
        assert_eq!(spec(&app, second).spacing, BlendSpacing::SmoothColor);

        app.session.begin_interaction("An independent blend edit").unwrap();
        app.session.preview(CMD, &json!({"id": second, "steps": 11})).unwrap();
        cancel(&mut app);
        assert!(app.session.in_interaction());
        assert_eq!(spec(&app, second).spacing, BlendSpacing::Steps(11));
    }

    #[test]
    fn blend_options_does_not_apply_to_art_restored_by_revert() {
        let (mut app, blend) = app_with_smooth_blend();
        let dir = vectorcraft_testkit::TempDir::new("blend-options-revert");
        app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|e| e.to_string())));
        app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string())));
        app.run("file.saveAs", json!({"path": dir.join("Art.vectorcraft")})).unwrap();
        app.run(CMD, json!({"id": blend, "steps": 4})).unwrap();
        open(&mut app).unwrap();
        set(&app, "steps", json!(7));
        frame(&mut app);
        assert_eq!(spec(&app, blend).spacing, BlendSpacing::Steps(7));

        app.run("file.revert", json!({"confirmed": true})).unwrap();
        app.run("select.set", json!({"ids": [blend]})).unwrap();
        frame(&mut app);
        assert_eq!(spec(&app, blend).spacing, BlendSpacing::SmoothColor);
        let undo = app.session.active().unwrap().history.undo.len();
        confirm(&mut app);
        assert!(is_open(&app));
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);

        cancel(&mut app);
        open(&mut app).unwrap();
        set(&app, "spacing", json!("steps"));
        set(&app, "steps", json!(7));
        confirm(&mut app);
        assert_eq!(spec(&app, blend).spacing, BlendSpacing::Steps(7));
    }
}
