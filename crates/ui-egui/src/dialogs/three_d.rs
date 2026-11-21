//! The host of the 3D effects' dialog: opens it through the held-dialog mechanism, keeps the app facts its rows read,
//! runs the live preview, and refuses values the effect cannot draw. The rows are built by [`super::revolve`].

use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard};

use craft::ui::runtime::env::Label;
use craft::ui::dialog::DialogId;
// PLAN: assumed paths of the form-dialog and live-preview items for the app crate.
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, ParamDialog, ParamLayout, ParamResult};
use serde_json::{Map, Value};
use vectorcraft_doc::{Document, Unit};
use vectorcraft_effects::{MAP_ART, MAP_INVISIBLE, MAP_SHADE, MapSurface, is_classic_three_d};

use super::form::{fields_of, json_of};
use super::host::{HeldOptions, open_held_with};
use super::revolve::{self, BevelChoice, Place};
use crate::VectorcraftApp;
use crate::caches::MapSurfacesKey;
use crate::live_preview::LivePreview;

/// The page whose rows are Map Art's.
const MAP_ART_PAGE: &str = "mapArt";

/// The open 3D dialog and what its pump keeps between passes.
pub(crate) struct Session {
    id: DialogId,
    effect: String,
    label: String,
    // PLAN: the contract has `item: Option<usize>`; the command needs `null` (the object's effects) apart from an omitted item.
    item: Option<Value>,
    index: Option<usize>,
    // PLAN: ef.C13 lacks the edit target and `aligned`; effect.rs resolves them and hands them over (open question 3).
    edit_target: Value,
    aligned: bool,
    preview: LivePreview,
    /// The command params of the last Preview-off try; `None` while Preview is on, so turning it off tries again.
    tried: Option<Value>,
    /// What the rows read, pushed each pump.
    pub(crate) data: Arc<RwLock<Data>>,
}

/// The app facts the 3D rows read.
#[derive(Clone)]
pub(crate) struct Data {
    /// The effect the rows are for.
    pub effect: Option<String>,
    /// The general unit lengths are shown in.
    pub unit: Unit,
    /// The Bevel Shape entries.
    pub bevels: Vec<BevelChoice>,
    /// Whether a bevels file is open (Edit Bevels… needs it). PLAN: not in the contract's `Data`.
    pub bevels_file: bool,
    /// The active document, for the symbol pictures.
    pub doc: Option<Arc<Document>>,
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
        bevels: effect.map(|e| revolve::bevel_options(app, params, e).0).unwrap_or_default(),
        bevels_file: app.session.bevels.path().is_some(),
        // PLAN: assumed `st.doc` is an `Arc<Document>`.
        doc: st.map(|s| s.doc.clone()),
        revision: st.map_or(0, |s| s.revision),
        selected: st.is_some_and(|s| !s.selection.objects.is_empty()),
        surfaces: None,
    }
}

impl Session {
    /// The command OK runs, and the params it takes.
    fn command(&self, params: &Map<String, Value>) -> (&'static str, Value) {
        // PLAN: assumed effect.rs's shared builder `command(effect, item, index, edit_target, aligned, &Map)`.
        super::effect::command(&self.effect, self.item.as_ref(), self.index, &self.edit_target, self.aligned, params)
    }

    /// The command, its params and the effect's params for the dialog's values.
    fn request(&self, fields: &DialogFields) -> (&'static str, Value, Map<String, Value>) {
        let rows = revolve::params(&read(&self.data), fields, Place::Dialog);
        let params = json_of(&rows, fields);
        let (cmd, p) = self.command(&params);
        (cmd, p, params)
    }
}

/// Opens the dialog for the record `effect::open` resolved: `effect`, `item`, `index`, `aligned`, `editTarget` and the
/// `params` to start from.
pub(crate) fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let effect = p.get("effect").and_then(Value::as_str).ok_or("no effect")?.to_owned();
    let info = vectorcraft_effects::effect_info(&effect).ok_or_else(|| format!("unknown effect `{effect}`"))?;
    let label = info.label.trim_end_matches('…').to_owned();
    let params = p.get("params").and_then(Value::as_object).cloned().or_else(|| info.defaults.as_object().cloned()).unwrap_or_default();
    let shared = Arc::new(RwLock::new(data(app, Some(&effect), &params)));
    let mut fields = fields_of(&revolve::params(&read(&shared), &DialogFields::new(), Place::Dialog), &params);
    fields.set("__effect", effect.clone());
    fields.set("__page", revolve::pages(&effect, Place::Dialog).first().map_or("rotation", |p| p.0).to_string());
    fields.set("__light", 0i64);
    let ctx = app.ctx().ok_or("no window")?; // PLAN: assumed `VectorcraftApp::ctx() -> Option<egui::Context>`
    let build_data = Arc::clone(&shared);
    let title = label.clone();
    let dialog = HostedParamDialog::new(&ctx, "effect.dialog", fields, move |f| {
        // PLAN: assumed `ParamDialog::new(key, title)` and `crate::canvas::main_canvas_id()`.
        ParamDialog::new("effect.dialog", title.clone())
            .params(revolve::params(&read(&build_data), f, Place::Dialog))
            .previews(true)
            .canvas(crate::canvas::main_canvas_id())
            .layout(ParamLayout::Stacked)
    });
    if let Some(old) = app.three_d_dialog.take() {
        old.preview.cancel(app);
    }
    let options = HeldOptions { cancel: Some(Box::new(cancel)), ..HeldOptions::default() };
    let id = open_held_with(app, dialog, ok, options).ok_or("no window")?;
    app.three_d_dialog = Some(Session {
        id,
        effect,
        label,
        item: p.get("item").cloned(),
        index: p.get("index").and_then(Value::as_u64).and_then(|k| usize::try_from(k).ok()),
        edit_target: p.get("editTarget").cloned().unwrap_or(Value::Null),
        aligned: p.get("aligned").and_then(Value::as_bool) == Some(true),
        preview: LivePreview::default(),
        tried: None,
        data: shared,
    });
    Ok(Value::Null)
}

/// OK: the preview becomes one undo step. A refusal keeps the dialog, and the session, open.
fn ok(app: &mut VectorcraftApp, answer: &ParamResult) -> Result<(), String> {
    let mut s = app.three_d_dialog.take().ok_or_else(|| "the 3D dialog is gone".to_string())?;
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
            app.three_d_dialog = Some(s);
            Err(e)
        }
    }
}

/// Cancel: the preview rolls back.
fn cancel(app: &mut VectorcraftApp) {
    if let Some(s) = app.three_d_dialog.take() {
        s.preview.cancel(app);
    }
}

fn dialog_of<R>(app: &mut VectorcraftApp, id: DialogId, f: impl FnOnce(&mut HostedParamDialog) -> R) -> Option<R> {
    app.dialogs().get_mut_by_id::<HostedParamDialog>(id).map(f) // PLAN: assumed `DialogHost::get_mut_by_id`
}

fn peek<R>(app: &VectorcraftApp, f: impl FnOnce(&HostedParamDialog) -> R) -> Option<R> {
    let id = app.three_d_dialog.as_ref()?.id;
    app.dialogs_ref().get_by_id::<HostedParamDialog>(id).map(f) // PLAN: assumed shared access to the dialog host
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
    let lit = if is_classic_three_d(effect) { matches!(fields.text("surface"), Some("diffuse" | "plastic")) } else { fields.bool("shade").unwrap_or(false) };
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
    let key = MapSurfacesKey { uid: st.uid, revision: st.revision, object, item, index: s.index, effect: s.effect.clone(), params: Value::Object(shaping) };
    if let Some((kept_key, kept)) = &app.canvas.map_surfaces
        && *kept_key == key
    {
        return Some(kept.clone());
    }
    let listed = Arc::new(vectorcraft_effects::map_surfaces_with(Some(&st.doc), st.doc.node(object)?, (item, s.index), &s.effect, &key.params)?);
    app.canvas.map_surfaces = Some((key, listed.clone()));
    Some(listed)
}

/// Once per frame: refreshes the rows' data, runs the preview, and, with Preview off, tries the values so the dialog
/// still says why OK would fail. `refuse` is called only from a result this pump produced: an idle pump leaves a
/// refusal standing until the values change.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(mut s) = app.three_d_dialog.take() else { return };
    // A dialog that closed is finished by the held drain (OK or Cancel), which owns the session's end.
    let Some((fields, previewing)) = dialog_of(app, s.id, |d| (d.fields().clone(), d.is_previewing())) else {
        app.three_d_dialog = Some(s);
        return;
    };
    let (cmd, p, params) = s.request(&fields);
    let mut fresh = data(app, Some(&s.effect), &params);
    fresh.surfaces = surfaces(app, &s, &fields, &params);
    *s.data.write().unwrap_or_else(PoisonError::into_inner) = fresh;
    let mut verdict = s.preview.run(app, previewing, &s.label, cmd, &p).map(|r| r.map(|_| ()));
    if previewing {
        s.tried = None;
    } else if !app.session.in_interaction() && s.tried.as_ref() != Some(&p) {
        // Waits for another interaction to end: trying inside it would replace its preview and cancel it. A failure
        // to begin leaves no interaction: the try then reports it.
        let _ = app.begin_interaction(&s.label);
        verdict = Some(app.session.preview(cmd, &p).map(|_| ()));
        // Only a try: without an interaction there is nothing to roll back.
        let _ = app.session.cancel_interaction();
        s.tried = Some(p);
    }
    if let Some(v) = verdict {
        dialog_of(app, s.id, |d| d.refuse(v.err().map(Label::from)));
    }
    app.three_d_dialog = Some(s);
}

#[cfg(test)]
pub(crate) mod tests_support;
