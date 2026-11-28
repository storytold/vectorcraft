//! The host of the 3D effects' dialog: opens it through the held-dialog mechanism, keeps the app facts its rows read,
//! runs the live preview, and refuses values the effect cannot draw. The rows are built by [`super::revolve`].

use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, Weak};

use craft::ui::contract::Bind;
use craft::ui::contract::overlay::{FloatPlace, Place as OverlayPlace};
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, ParamDialog, ParamLayout, ParamResult};
use craft::ui::runtime::env::Label;
use serde_json::{Map, Value};
use vectorcraft_doc::{Document, Selection, Unit};
use vectorcraft_effects::{MAP_ART, MAP_INVISIBLE, MAP_SHADE, MapSurface, is_classic_three_d};
use vectorcraft_engine::EngineError;

use super::form::{fields_of, json_of};
use super::host::{HeldOptions, open_held_with};
use super::preview::LivePreview;
use super::revolve::{self, BevelChoice, Place};
use crate::VectorcraftApp;
use crate::caches::MapSurfacesKey;

/// The page whose rows are Map Art's.
const MAP_ART_PAGE: &str = "mapArt";

/// The open 3D dialog and what its pump keeps between passes.
pub(crate) struct Session {
    id: DialogId,
    source: Option<(u64, u64)>,
    effect: String,
    label: String,
    // PLAN: the contract has `item: Option<usize>`; the command needs `null` (the object's effects) apart from an omitted item.
    item: Option<Value>,
    index: Option<usize>,
    edit_target: Value,
    aligned: bool,
    preview: LivePreview,
    preview_owner: Option<PreviewOwner>,
    /// The command params of the last Preview-off try; `None` while Preview is on, so turning it off tries again.
    tried: Option<Value>,
    /// What the rows read, pushed each pump.
    pub(crate) data: Arc<RwLock<Data>>,
}

/// The interaction and preview last written by the 3D dialog.
struct PreviewOwner {
    selection: Weak<Selection>,
    command: Option<(String, Value)>,
}

/// The app facts the 3D rows read.
#[derive(Clone)]
pub(crate) struct Data {
    /// The effect the rows are for.
    pub effect: Option<String>,
    /// The general unit lengths are shown in.
    pub unit: Unit,
    /// Numbers Without Units Are Points.
    pub bare_points: bool,
    /// The Bevel Shape entries.
    pub bevels: Vec<BevelChoice>,
    /// Whether a bevels file is open (Edit Bevels… needs it). PLAN: not in the contract's `Data`.
    pub bevels_file: bool,
    /// The active document, for the symbol pictures.
    pub doc: Option<Arc<Document>>,
    /// The opening document generation while it can still supply artwork to the Graphics drop target.
    pub document: Option<(u64, u64)>,
    /// The active document's version.
    pub revision: u64,
    /// Whether the active document has a selection.
    pub selected: bool,
    /// The numbered surfaces Map Art shows; set only while its page shows.
    pub surfaces: Option<Arc<Vec<MapSurface>>>,
}

fn read(data: &RwLock<Data>) -> RwLockReadGuard<'_, Data> {
    data.read().unwrap_or_else(PoisonError::into_inner)
}

/// The facts of `app` the rows of `effect` read, with `params` as the values the bevel list is built for.
pub(crate) fn data(app: &VectorcraftApp, effect: Option<&str>, params: &Map<String, Value>) -> Data {
    let st = app.session.active();
    Data {
        effect: effect.map(str::to_owned),
        unit: app.session.general_unit(),
        bare_points: app.session.prefs.numbers_without_units_are_points,
        bevels: effect.map(|e| revolve::bevel_options(app, params, e).0).unwrap_or_default(),
        bevels_file: app.session.bevels.path().is_some(),
        doc: st.map(|s| s.doc.clone()),
        document: st.map(|s| (s.uid, s.reverts)),
        revision: st.map_or(0, |s| s.revision),
        selected: st.is_some_and(|s| !s.selection.objects.is_empty()),
        surfaces: None,
    }
}

impl Session {
    fn is_current_drawing(&self, app: &VectorcraftApp) -> bool {
        self.source == active_drawing(app)
    }

    fn check_source(&self, app: &VectorcraftApp) -> Result<(), String> {
        let active = active_drawing(app);
        if active == self.source {
            Ok(())
        } else if active.map(|(id, _)| id) == self.source.map(|(id, _)| id) {
            Err(tl!("The document was reverted. Reopen this dialog to continue.").into())
        } else {
            Err(tl!("Return to the original drawing to apply the effect").into())
        }
    }

    fn owns_preview(&self, app: &VectorcraftApp) -> bool {
        self.is_current_drawing(app)
            && self.preview_owner.as_ref().is_some_and(|owner| {
                app.session.active().and_then(|doc| doc.interaction.as_ref()).is_some_and(|interaction| {
                    owner.selection.upgrade().is_some_and(|selection| Arc::ptr_eq(&selection, &interaction.selection))
                        && owner.command == interaction.preview
                })
            })
    }

    fn cancel_preview(&mut self, app: &mut VectorcraftApp) {
        let owned = self.owns_preview(app);
        let preview = std::mem::take(&mut self.preview);
        self.preview_owner = None;
        if owned {
            preview.cancel(app);
        }
    }

    fn ready_to_edit(&mut self, app: &mut VectorcraftApp) -> Result<bool, String> {
        if self.owns_preview(app) {
            return Ok(true);
        }
        self.preview = LivePreview::default();
        self.preview_owner = None;
        if app.session.in_interaction() {
            app.keep_typing()?;
        }
        Ok(!app.session.in_interaction())
    }

    fn run_preview(&mut self, app: &mut VectorcraftApp, cmd: &str, params: &Value) -> Option<Result<(), String>> {
        match self.ready_to_edit(app) {
            Ok(true) => {}
            Ok(false) => return None,
            Err(why) => return Some(Err(why)),
        }
        let verdict = self.preview.run(app, true, &self.label, cmd, params)?.map(|_| ());
        self.preview_owner = app
            .session
            .active()
            .and_then(|doc| doc.interaction.as_ref())
            .map(|interaction| PreviewOwner { selection: Arc::downgrade(&interaction.selection), command: interaction.preview.clone() });
        Some(verdict)
    }

    /// The command OK runs, and the params it takes.
    fn command(&self, params: &Map<String, Value>) -> (&'static str, Value) {
        super::effect::command_for(&self.effect, self.item.as_ref(), self.index, &self.edit_target, self.aligned, params.clone(), None)
    }

    /// The command, its params and the effect's params for the dialog's values.
    fn request(&self, fields: &DialogFields) -> (&'static str, Value, Map<String, Value>) {
        let rows = revolve::params(&read(&self.data), fields, Place::Dialog);
        let mut params = json_of(&rows, fields);
        for key in super::three_d_classic::MAP_ROWS {
            params.remove(key);
        }
        revolve::drop_untaken_light_keys(&self.effect, &mut params);
        let (cmd, p) = self.command(&params);
        (cmd, p, params)
    }
}

fn active_drawing(app: &VectorcraftApp) -> Option<(u64, u64)> {
    app.session.active().map(|doc| (doc.uid, doc.reverts))
}

/// Opens the dialog for the record `effect::open` resolved: `effect`, `item`, `index`, `aligned`, `editTarget` and the
/// `params` to start from.
pub(crate) fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let effect = p.get("effect").and_then(Value::as_str).ok_or("no effect")?.to_owned();
    let info = vectorcraft_effects::effect_info(&effect).ok_or_else(|| format!("unknown effect `{effect}`"))?;
    let label = info.label.trim_end_matches('…').to_owned();
    let params = p.get("params").and_then(Value::as_object).cloned().or_else(|| info.defaults.as_object().cloned()).unwrap_or_default();
    let source = active_drawing(app);
    let shared = Arc::new(RwLock::new(data(app, Some(&effect), &params)));
    let mut fields = fields_of(&revolve::params(&read(&shared), &DialogFields::new(), Place::Dialog), &params);
    fields.set("__effect", effect.clone());
    fields.set("__page", revolve::pages(&effect, Place::Dialog).first().map_or("rotation", |p| p.0).to_string());
    fields.set("__light", 0i64);
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let build_data = Arc::clone(&shared);
    let title = label.clone();
    let footer_effect = effect.clone();
    // The canvas of the current window: the gizmo and the droppers work there while the dialog is open.
    let canvas = crate::doc_windows::current(app).map(crate::canvas::canvas_id);
    // At the window's right edge from the canvas's top, where the 3D panel docks, so the dialog leaves the art the
    // gizmo turns in view.
    let beside = app.canvas_view().map(|v| egui::pos2(ctx.content_rect().right(), v.rect.top()));
    let dialog = HostedParamDialog::new(&ctx, "effect.dialog", fields, move |f| {
        let rows = revolve::params(&read(&build_data), f, Place::Dialog);
        let reset = revolve::reset_all_button(&footer_effect, f).or_else(|| super::three_d_classic::map_done_button(f));
        let hint = hint_for(&footer_effect, f.bool("preview").unwrap_or(false), f);
        let dialog = ParamDialog::new("effect.dialog", title.clone(), rows).values(Bind::bound(f)).previews(true).layout(ParamLayout::Stacked);
        let dialog = match reset {
            Some(reset) => dialog.extra_buttons(vec![reset]).footer_note(hint),
            None => dialog,
        };
        let dialog = match beside {
            Some(corner) => {
                dialog.shaped(move |d| d.overlay(|o| o.place(OverlayPlace::Float(FloatPlace::corner_at(corner, egui::Align2::RIGHT_TOP)))))
            }
            None => dialog,
        };
        match canvas {
            Some(canvas) => dialog.canvas(canvas),
            None => dialog,
        }
    });
    let cancelled = Arc::clone(&shared);
    let options = HeldOptions { cancel: Some(Box::new(move |app| cancel(app, &cancelled))), ..HeldOptions::default() };
    let id = open_held_with(app, dialog, ok, options).ok_or("no window")?;
    if let Some(mut old) = app.three_d_dialog.take() {
        old.cancel_preview(app);
    }
    app.three_d_dialog = Some(Session {
        id,
        source,
        effect,
        label,
        item: p.get("item").cloned(),
        index: p.get("index").and_then(Value::as_u64).and_then(|k| usize::try_from(k).ok()),
        edit_target: p.get("editTarget").cloned().unwrap_or(Value::Null),
        aligned: p.get("aligned").and_then(Value::as_bool) == Some(true),
        preview: LivePreview::default(),
        preview_owner: None,
        tried: None,
        data: shared,
    });
    Ok(Value::Null)
}

/// OK: the preview becomes one undo step. A refusal keeps the dialog, and the session, open.
fn ok(app: &mut VectorcraftApp, answer: &ParamResult) -> Result<(), String> {
    let mut s = app.three_d_dialog.take().ok_or_else(|| "the 3D dialog is gone".to_string())?;
    let ready = s
        .check_source(app)
        .and_then(|()| if s.ready_to_edit(app)? { Ok(()) } else { Err(tl!("Finish the current edit before applying the effect").into()) });
    if let Err(why) = ready {
        app.three_d_dialog = Some(s);
        return Err(why);
    }
    let (cmd, p, _) = s.request(&answer.fields);
    match std::mem::take(&mut s.preview).commit(app, cmd, p.clone()) {
        Ok(_) => {
            // Last Effect repeats applied effects, not edits.
            if cmd == "effect.apply" {
                app.last_effect = Some((s.effect.clone(), p["params"].clone()));
            }
            Ok(())
        }
        Err(e) => {
            s.preview_owner = None;
            app.three_d_dialog = Some(s);
            Err(e)
        }
    }
}

/// Cancel: the preview rolls back.
fn cancel(app: &mut VectorcraftApp, opening: &Arc<RwLock<Data>>) {
    if app.three_d_dialog.as_ref().is_some_and(|s| Arc::ptr_eq(opening, &s.data))
        && let Some(mut s) = app.three_d_dialog.take()
    {
        s.cancel_preview(app);
    }
}

fn dialog_of<R>(app: &VectorcraftApp, id: DialogId, f: impl FnOnce(&mut HostedParamDialog) -> R) -> Option<R> {
    app.dialogs()?.with(|h| h.get_mut_by_id::<HostedParamDialog>(id).map(f))
}

fn peek<R>(app: &VectorcraftApp, f: impl FnOnce(&HostedParamDialog) -> R) -> Option<R> {
    let id = app.three_d_dialog.as_ref().filter(|s| s.is_current_drawing(app))?.id;
    app.dialogs()?.with(|h| h.get_by_id::<HostedParamDialog>(id).map(f))
}

/// Whether the active canvas should give its selection marks to the 3D dialog.
pub(crate) fn editing_current_drawing(app: &VectorcraftApp) -> bool {
    app.three_d_dialog.as_ref().is_some_and(|s| s.is_current_drawing(app))
}

/// Gives the gizmo and the outline the open dialog; `None` when there is none.
pub(crate) fn with_fields<R>(app: &mut VectorcraftApp, f: impl FnOnce(&mut HostedParamDialog) -> R) -> Option<R> {
    let id = app.three_d_dialog.as_ref()?.id;
    dialog_of(app, id, f)
}

/// Whether the artboard shows the rotation and light gizmos.
pub(crate) fn gizmo_shown(app: &VectorcraftApp) -> bool {
    peek(app, HostedParamDialog::is_previewing).unwrap_or(false)
}

/// The hint the status bar and the dialog show; `None` when no 3D dialog is open.
pub(crate) fn hint(app: &VectorcraftApp) -> Option<&'static str> {
    let effect = app.three_d_dialog.as_ref()?.effect.clone();
    peek(app, |d| hint_for(&effect, d.is_previewing(), d.fields()))
}

/// The hint for `effect` with the dialog's `fields`: the gizmo's use while Preview is on and the page has one.
pub(crate) fn hint_for(effect: &str, previewing: bool, fields: &DialogFields) -> &'static str {
    let lighting = fields.text("__page") == Some("lighting");
    let lit = if is_classic_three_d(effect) {
        matches!(fields.text("surface"), Some("diffuse" | "plastic"))
    } else {
        fields.bool("shade").unwrap_or(false)
    };
    if !previewing || (lighting && !lit) {
        tl!("Drag values or labels. Double-click a value to type.")
    } else if lighting {
        tl!("Drag the light, or its rings. Shift snaps to 15°.")
    } else {
        tl!("Drag rings to rotate. Shift snaps to 15°.")
    }
}

/// The surfaces Map Art numbers, kept in the canvas cache until the document, the object or a setting that shapes the
/// solid changes (the maps themselves do not).
fn surfaces(app: &mut VectorcraftApp, s: &Session, fields: &DialogFields, params: &Map<String, Value>) -> Option<Arc<Vec<MapSurface>>> {
    if !vectorcraft_effects::takes_map_art(&s.effect) || fields.text("__page") != Some(MAP_ART_PAGE) {
        return None;
    }
    let st = app.session.active()?;
    let object = *st.selection.objects.first()?;
    let mut shaping = params.clone();
    for key in [MAP_ART, MAP_SHADE, MAP_INVISIBLE] {
        shaping.remove(key);
    }
    let item = s.item.as_ref().and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok());
    let key =
        MapSurfacesKey { uid: st.uid, revision: st.revision, object, item, index: s.index, effect: s.effect.clone(), params: Value::Object(shaping) };
    if let Some((kept_key, kept)) = &app.canvas.map_surfaces
        && *kept_key == key
    {
        return Some(kept.clone());
    }
    let listed = Arc::new(vectorcraft_effects::map_surfaces_with(Some(&st.doc), st.doc.node(object)?, (item, s.index), &s.effect, &key.params)?);
    app.canvas.map_surfaces = Some((key, listed.clone()));
    Some(listed)
}

/// Refresh drop eligibility before the dialog can receive a release in this frame.
pub(crate) fn sync_graphics_source(app: &VectorcraftApp) {
    let Some(open) = &app.three_d_dialog else { return };
    let source = open.source.filter(|&(uid, reverts)| app.session.documents().iter().any(|d| (d.uid, d.reverts) == (uid, reverts)));
    open.data.write().unwrap_or_else(PoisonError::into_inner).document = source;
}

/// Once per frame: refreshes the rows' data, runs the preview, and, with Preview off, tries the values so the dialog
/// still says why OK would fail. `refuse` is called only from a result this pump produced: an idle pump leaves a
/// refusal standing until the values change.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(mut s) = app.three_d_dialog.take() else { return };
    let current = s.is_current_drawing(app);
    // A dialog that closed is finished by the held drain (OK or Cancel), which owns the session's end.
    let Some((fields, previewing)) = dialog_of(app, s.id, |d| {
        match d.take_action().as_deref() {
            Some(revolve::RESET_ALL) => revolve::reset_all(d, &s.effect),
            Some(super::three_d_classic::MAP_DONE) => super::three_d_classic::map_done(d, &s.effect),
            _ => {}
        }
        (d.fields().clone(), d.is_previewing())
    }) else {
        app.three_d_dialog = Some(s);
        return;
    };
    let (cmd, p, params) = s.request(&fields);
    // Map Art keeps its read-only surface list current. While away, editable document data
    // such as symbol names and artwork drop ownership still belongs to the opening drawing.
    let mut fresh = if current { data(app, Some(&s.effect), &params) } else { read(&s.data).clone() };
    fresh.surfaces = surfaces(app, &s, &fields, &params);
    *s.data.write().unwrap_or_else(PoisonError::into_inner) = fresh;
    if !current {
        s.tried = None;
        app.three_d_dialog = Some(s);
        return;
    }
    let verdict = if previewing {
        s.tried = None;
        s.run_preview(app, cmd, &p)
    } else {
        s.cancel_preview(app);
        if s.tried.as_ref() == Some(&p) {
            None
        } else if app.session.in_interaction() {
            // Clear the old refusal now and try again once the other interaction ends.
            Some(Ok(()))
        } else {
            let tested = try_values(app, &s.label, cmd, &p);
            s.tried = Some(p);
            Some(tested)
        }
    };
    if let Some(v) = verdict {
        dialog_of(app, s.id, |d| d.refuse(v.err().map(Label::from)));
    }
    app.three_d_dialog = Some(s);
}

/// With Preview off, try changed values as a temporary interaction to expose engine refusals.
fn try_values(app: &mut VectorcraftApp, label: &str, cmd: &str, params: &Value) -> Result<(), String> {
    app.begin_interaction(label)?;
    let verdict = app.session.preview(cmd, params).map(|_| ()).map_err(|error| match error {
        EngineError::BadParams { msg, .. } => msg,
        other => other.to_string(),
    });
    let _ = app.session.cancel_interaction();
    verdict
}

#[cfg(test)]
pub(crate) mod tests_support;

#[cfg(test)]
mod drawing_tests {
    use craft::ui::dialog::StackDialog as _;
    use serde_json::json;
    use vectorcraft_doc::NodeId;
    use vectorcraft_effects::EXTRUDE;

    use super::*;

    fn drawing() -> (VectorcraftApp, u64) {
        let mut app = crate::testing::windowed(VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default()));
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let node = rectangle(&mut app, 10);
        (app, node)
    }

    fn rectangle(app: &mut VectorcraftApp, x: u32) -> u64 {
        app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap()
    }

    fn second_drawing(app: &mut VectorcraftApp) -> u64 {
        app.run("file.new", json!({"width": 220, "height": 220})).unwrap();
        rectangle(app, 80)
    }

    fn open_effect(app: &mut VectorcraftApp, preview: bool) {
        app.run("effect.dialog", json!({"effect": EXTRUDE})).unwrap();
        let ctx = app.ui_ctx.clone().unwrap();
        with_fields(app, |dialog| dialog.set_json_field(&ctx, "preview", json!(preview))).unwrap().unwrap();
    }

    fn applied(app: &VectorcraftApp, node: u64) -> Vec<String> {
        app.session.active().unwrap().doc.node(NodeId(node)).unwrap().appearance.effects.iter().map(|effect| effect.id.clone()).collect()
    }

    fn frame(app: &mut VectorcraftApp) {
        let ctx = app.ui_ctx.clone().unwrap();
        super::super::host::pump(app, &ctx);
    }

    fn hold_ok(app: &VectorcraftApp) {
        let ctx = app.ui_ctx.clone().unwrap();
        let id = app.three_d_dialog.as_ref().unwrap().id;
        assert!(!app.dialogs().unwrap().with(|host| host.confirm_by_id(&ctx, id)));
    }

    #[test]
    fn three_d_ownership_preview_resumes_only_in_the_opening_drawing() {
        let (mut app, original) = drawing();
        open_effect(&mut app, true);
        frame(&mut app);
        assert_eq!(applied(&app, original), [EXTRUDE]);
        let opening_document = read(&app.three_d_dialog.as_ref().unwrap().data).doc.clone().unwrap();
        let other = second_drawing(&mut app);
        frame(&mut app);
        assert!(applied(&app, other).is_empty(), "3D preview reached the other drawing");
        assert!(!app.session.in_interaction());
        assert!(!gizmo_shown(&app));
        assert!(!crate::canvas::revolve_editing(&app));
        let drawn_document = read(&app.three_d_dialog.as_ref().unwrap().data).doc.clone().unwrap();
        assert!(Arc::ptr_eq(&drawn_document, &opening_document), "Map Art keeps the opening document");
        app.run("document.activate", json!({"index": 0})).unwrap();
        frame(&mut app);
        assert_eq!(applied(&app, original), [EXTRUDE]);
        assert!(gizmo_shown(&app));
        assert!(crate::canvas::revolve_editing(&app));
    }

    #[test]
    fn three_d_ownership_ok_refuses_the_other_drawing_and_can_retry() {
        let (mut app, original) = drawing();
        open_effect(&mut app, false);
        hold_ok(&app);
        let other = second_drawing(&mut app);
        let history = app.session.active().unwrap().history.undo.len();
        frame(&mut app);
        assert!(applied(&app, other).is_empty(), "3D OK reached the other drawing");
        assert_eq!(app.session.active().unwrap().history.undo.len(), history);
        app.run("document.activate", json!({"index": 0})).unwrap();
        let history = app.session.active().unwrap().history.undo.len();
        hold_ok(&app);
        frame(&mut app);
        assert_eq!(applied(&app, original), [EXTRUDE]);
        assert_eq!(app.session.active().unwrap().history.undo.len(), history + 1);
    }

    #[test]
    fn three_d_ownership_cancel_leaves_the_other_drawings_preview_running() {
        let (mut app, _) = drawing();
        open_effect(&mut app, true);
        frame(&mut app);
        second_drawing(&mut app);
        app.session.begin_interaction("Other drawing's edit").unwrap();
        app.session.preview("paint.setFill", &json!({"color": "#00aa00"})).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let ctx = app.ui_ctx.clone().unwrap();
        let id = app.three_d_dialog.as_ref().unwrap().id;
        assert!(app.dialogs().unwrap().with(|host| host.cancel_by_id(&ctx, id)));
        frame(&mut app);
        assert!(app.session.in_interaction(), "3D Cancel ended the other drawing's edit");
        assert!(app.session.active().unwrap().doc == before);
    }

    #[test]
    fn three_d_ownership_preview_off_preserves_another_drawings_interaction() {
        let (mut app, _) = drawing();
        open_effect(&mut app, true);
        frame(&mut app);
        let other = second_drawing(&mut app);
        app.session.begin_interaction("Other drawing's edit").unwrap();
        app.session.preview("paint.setFill", &json!({"color": "#00aa00"})).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let ctx = app.ui_ctx.clone().unwrap();
        with_fields(&mut app, |dialog| dialog.set_json_field(&ctx, "preview", json!(false))).unwrap().unwrap();
        frame(&mut app);
        assert!(app.session.in_interaction());
        assert!(app.session.active().unwrap().doc == before);
        assert!(applied(&app, other).is_empty());
    }

    #[test]
    fn three_d_ownership_replacement_keeps_its_own_preview() {
        let (mut app, original) = drawing();
        open_effect(&mut app, true);
        frame(&mut app);
        app.run("effect.dialog", json!({"effect": vectorcraft_effects::REVOLVE})).unwrap();
        let ctx = app.ui_ctx.clone().unwrap();
        with_fields(&mut app, |dialog| dialog.set_json_field(&ctx, "preview", json!(true))).unwrap().unwrap();
        frame(&mut app);
        assert!(app.three_d_dialog.is_some(), "a replaced 3D dialog's Cancel removed the new opening");
        assert_eq!(applied(&app, original), [vectorcraft_effects::REVOLVE]);
        hold_ok(&app);
        frame(&mut app);
        assert_eq!(applied(&app, original), [vectorcraft_effects::REVOLVE]);
    }

    #[test]
    fn three_d_ownership_ok_refuses_the_reloaded_drawing() {
        let (mut app, original) = drawing();
        app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|e| e.to_string())));
        app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string())));
        let folder = vectorcraft_testkit::TempDir::new("three-d-opening-revert");
        let path = folder.join("drawing.vectorcraft");
        app.run("file.saveAs", json!({"path": path})).unwrap();
        rectangle(&mut app, 80);
        app.run("select.set", json!({"ids": [original]})).unwrap();
        let source = app.session.active().unwrap();
        let (uid, generation) = (source.uid, source.reverts);
        open_effect(&mut app, false);
        hold_ok(&app);
        app.run("file.revert", json!({"confirmed": true})).unwrap();
        let reloaded = app.session.active().unwrap();
        assert_eq!(reloaded.uid, uid);
        assert_ne!(reloaded.reverts, generation);
        app.run("select.set", json!({"ids": [original]})).unwrap();
        let history = app.session.active().unwrap().history.undo.len();
        frame(&mut app);
        assert!(applied(&app, original).is_empty(), "3D OK reached the reloaded drawing");
        assert_eq!(app.session.active().unwrap().history.undo.len(), history);
        open_effect(&mut app, false);
        hold_ok(&app);
        frame(&mut app);
        assert_eq!(applied(&app, original), [EXTRUDE]);
    }
}
