//! Live effect dialogs (Effect menu, Appearance panel): the effect's rows with a live Preview that
//! OK keeps as one undo step and Cancel rolls back. With an index the dialog edits that applied
//! effect in place (`effect.setParams`) instead of adding one (`effect.apply`).
//!
//! Choosing an effect that the target already carries first asks whether to edit it or add
//! another one. The question and the dialog share the one `effect` slot of [`EffectDialogs`].
//!
//! OK sends only the rows whose value differs from the values the dialog opened with
//! ([`Open::opened`]): an edit keeps every other stored value, and a new effect takes the
//! effect's defaults for the rest.

// PLAN: assumed the facade re-exports craft_host as `craft::host`
use craft::host::files::PickRequest;
use craft_ui_patterns::contract::Bind;
use craft_ui_patterns::dialog::{DialogId, DialogResult, PromptChoice, StandardPrompt};
use craft_ui_patterns::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, ParamDialog, ParamResult};
use craft_ui_patterns::runtime::{Lifetime, SaveKey};
use serde_json::{Map, Value, json};

use super::effect_params::{EffectCx, effect_params, heading, label};
use super::form::{self, LivePreview};
use super::host::{HeldOptions, open_held_with};
use crate::VectorcraftApp;
// PLAN: assumed path of the files area's asks (fl.A1, fl.A3)
use crate::file_asks::{self, Owner, Picked};

/// The reply while the Edit or Add New question is open.
const EXISTS: &str = "effectExists";

/// Where an effect's Warp rows remember the last values until the app quits.
const WARP_KEY: SaveKey = SaveKey::new("effect.warp");

/// The record of every open effect-area dialog.
#[derive(Default)]
pub(crate) struct EffectDialogs {
    /// The effect dialog, or the question that comes before it.
    pub effect: Option<Open>,
    /// The Halftone dialog.
    pub halftone: Option<super::halftone::Open>,
    /// The Document Raster Effects Settings and Rasterize dialogs.
    pub raster: Option<super::raster_effects::Open>,
    /// The envelope dialogs.
    pub envelope: Option<super::envelope::Open>,
    /// The Effect Gallery.
    pub gallery: Option<super::effect_gallery::Open>,
}

/// One effect dialog: what OK needs to build its command, and its live preview.
pub(crate) struct Open {
    /// The dialog's id; while [`asking`](Open::asking), the question's.
    pub id: DialogId,
    /// The item the effect goes to, resolved.
    target: Value,
    /// The parameters that find the applied effect (`item`, `aligned`).
    edit_target: Value,
    /// The applied effect being edited; none for a new one.
    index: Option<usize>,
    /// The item the command names: the edited effect's, or the one the caller gave.
    item: Option<Value>,
    /// A shared row of a mixed selection: OK edits each object's own copy.
    aligned: bool,
    /// The values the command's absent keys stand for, as the rows show them: the stored ones for
    /// an edit, the effect's defaults for a new effect.
    opened: Map<String, Value>,
    /// The live preview OK keeps and Cancel rolls back.
    pub preview: LivePreview,
    /// What the rows need from the app.
    pub cx: EffectCx,
    /// Whether the Edit or Add New question is still open.
    asking: bool,
}

/// `effect.dialog {effect, index?, item?}`: edit applied effect `index` of `item` (null: the
/// object's effects; omitted: the active item's, else the object's) prefilled with its values,
/// or add `effect`, first asking when that list already has it.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let id = p.get("effect").and_then(Value::as_str).unwrap_or_default();
    // An SVG filter is chosen and edited in Apply SVG Filter.
    if id == vectorcraft_effects::SVG_FILTER {
        return super::svg_filter::open(app, p);
    }
    if vectorcraft_effects::is_three_d(id) {
        return super::three_d::open(app, p);
    }
    vectorcraft_effects::effect_info(id).ok_or_else(|| format!("unknown effect `{id}`"))?;
    // The list the effect goes to: `item` as given, else the active item's (the object's without
    // one). A new effect keeps `item` unresolved, so `effect.apply` targets each object as usual.
    let given = p.get("item").cloned();
    let target = given.clone().unwrap_or_else(|| json!(app.session.appearance_item()));
    let (edit_target, applied) = target_effects(app, p);
    let target = edit_target.get("item").cloned().unwrap_or(target);
    let mut open = Open {
        id: DialogId::NULL,
        target: target.clone(),
        edit_target,
        index: None,
        item: given,
        aligned: p.get("aligned").and_then(Value::as_bool) == Some(true),
        opened: Map::new(),
        preview: LivePreview::default(),
        cx: EffectCx::new(app, id, false),
        asking: false,
    };
    match p.get("index").and_then(Value::as_u64) {
        Some(k) => {
            let k = k as usize;
            let e = applied.get(k).ok_or_else(|| format!("no effect at index {k}"))?;
            if e.id != id {
                return Err(format!("effect {k} is `{}`, not `{id}`", e.id));
            }
            open.index = Some(k);
            open.item = Some(target);
            open.cx.editing = true;
            launch(app, open, params_of(e), Map::new())
        }
        None => match applied.iter().rposition(|e| e.id == id) {
            Some(k) => {
                open.index = Some(k);
                ask(app, open)
            }
            None => launch(app, open, defaults_of(id), Map::new()),
        },
    }
}

/// Last Effect…: a new dialog of `effect` opened with the values it was last applied with.
pub(crate) fn open_last(app: &mut VectorcraftApp, effect: &str, params: Value) -> Result<Value, String> {
    vectorcraft_effects::effect_info(effect).ok_or_else(|| format!("unknown effect `{effect}`"))?;
    let open = Open {
        id: DialogId::NULL,
        target: Value::Null,
        edit_target: json!({}),
        index: None,
        item: None,
        aligned: false,
        opened: Map::new(),
        preview: LivePreview::default(),
        cx: EffectCx::new(app, effect, false),
        asking: false,
    };
    launch(app, open, defaults_of(effect), params.as_object().cloned().unwrap_or_default())
}

/// The function returns target parameters and effects at `item` (an index or null).
/// An omitted `item` uses the active row.
fn target_effects(app: &VectorcraftApp, p: &Value) -> (Value, Vec<vectorcraft_doc::Effect>) {
    let mut params = json!({"aligned": p.get("aligned").and_then(Value::as_bool).unwrap_or(false)});
    if let Some(item) = p.get("item") {
        params["item"] = item.clone();
    }
    app.session.appearance_effect_target(&params).map(|(target, effects)| (target, effects.to_vec())).unwrap_or_else(|| (json!({}), vec![]))
}

/// Is `id` a Pathfinder effect whose only options are its Pathfinder Options (every one but Soft
/// Mix, which also has a Mixing Rate)?
pub(super) fn has_only_pathfinder_options(id: &str) -> bool {
    vectorcraft_effects::takes_pathfinder_options(id)
        && vectorcraft_effects::default_params(id).is_some_and(|d| d.as_object().is_none_or(Map::is_empty))
}

/// A Pathfinder effect's Pathfinder Options in `fields`, where they are missing: for a new effect
/// (`fresh`) the Pathfinder panel's current options, as `effect.apply` takes them; for an applied
/// one saved without them, the values it evaluates with. A precision out of range (one that Scale
/// Strokes & Effects took past it) or not a number opens at the value it evaluates with.
fn fill_pathfinder_options(app: &VectorcraftApp, id: &str, fields: &mut Map<String, Value>, fresh: bool) {
    if !vectorcraft_effects::takes_pathfinder_options(id) {
        return;
    }
    let options =
        if fresh { super::pathfinder_options::options_in_force(app) } else { vectorcraft_effects::PathfinderEffectOptions::default().to_params() };
    for (key, v) in options.as_object().into_iter().flatten() {
        fields.entry(key.clone()).or_insert_with(|| v.clone());
    }
    if fields.get("precision").is_some_and(|v| !v.as_f64().is_some_and(|p| vectorcraft_effects::PATHFINDER_EFFECT_PRECISION.contains(&p))) {
        let evaluated = vectorcraft_effects::PathfinderEffectOptions::from_params(&json!({"precision": fields["precision"]})).precision;
        fields.insert("precision".into(), json!(evaluated));
    }
}

/// An applied effect's values over its defaults.
fn params_of(e: &vectorcraft_doc::Effect) -> Map<String, Value> {
    match vectorcraft_effects::merged_params(&e.id, &e.params) {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

/// The defaults of effect `id`.
fn defaults_of(id: &str) -> Map<String, Value> {
    vectorcraft_effects::default_params(id).and_then(|v| v.as_object().cloned()).unwrap_or_default()
}

/// `values` as the dialog's rows show them: in the effect's current form, a Pathfinder effect's
/// options filled in, Duotone's tones padded and Radial Blur's centre within its bounds.
fn normalized(app: &VectorcraftApp, id: &str, values: Map<String, Value>, fresh: bool) -> Map<String, Value> {
    let mut values = match vectorcraft_effects::upgrade_params(id, &Value::Object(values)) {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    fill_pathfinder_options(app, id, &mut values, fresh);
    // PLAN: assumed `fn pad_tones(&mut Map<String, Value>)` (effects-w6) and `fn clamp_blur_center(&mut Map<String, Value>)` (effects-w5)
    if id == vectorcraft_effects::addons::duotone::DUOTONE {
        super::duotone::pad_tones(&mut values);
    } else if id == super::effect_params::RADIAL_BLUR {
        super::effect_rows::clamp_blur_center(&mut values);
    }
    values
}

/// Asks whether to edit the applied effect or add another one; [`pump`] reads the answer.
fn ask(app: &mut VectorcraftApp, mut open: Open) -> Result<Value, String> {
    let name = label(&open.cx.id);
    let prompt = StandardPrompt::custom(tl!("Edit the applied effect, or add another one?").to_string())
        .title(crate::i18n::fmt(tl!("{effect} is already applied"), &[("effect", &name)]))
        .button(PromptChoice::Proceed, tl!("Edit").to_string())
        .button(PromptChoice::Alternate, tl!("Add New").to_string())
        .button(PromptChoice::Cancel, tl!("Cancel").to_string());
    open.id = app.dialogs().open(prompt);
    open.asking = true;
    app.effect_dialogs.effect = Some(open);
    Ok(json!({ "pending": EXISTS }))
}

/// Opens the dialog of `open`: `baseline` is what the command's absent keys stand for, `seed`
/// the values shown over it.
fn launch(app: &mut VectorcraftApp, mut open: Open, baseline: Map<String, Value>, seed: Map<String, Value>) -> Result<Value, String> {
    // PLAN: assumed `VectorcraftApp::egui_context(&self) -> Option<egui::Context>`
    let ctx = app.egui_context().ok_or("no window")?;
    if let Some(old) = app.effect_dialogs.effect.take() {
        old.preview.cancel(app);
    }
    let id = open.cx.id.clone();
    let fresh = open.index.is_none();
    let mut shown = baseline.clone();
    shown.extend(seed);
    let (baseline, shown) = (normalized(app, &id, baseline, fresh), normalized(app, &id, shown, fresh));
    let rows = effect_params(&open.cx, &DialogFields::new());
    open.opened = form::json_of(&rows, &form::fields_of(&rows, &baseline));
    let fields = form::fields_of(&rows, &shown);

    let (cx, title) = (open.cx.clone(), heading(&open.cx));
    let warp_new = fresh && super::warp_effect::is_warp(&id);
    let dialog = HostedParamDialog::new(&ctx, "effect", fields, move |f| {
        let rows = effect_params(&cx, f);
        let dialog = ParamDialog::new("effect", title.clone(), rows).values(Bind::bound(f)).previews(true);
        if warp_new { dialog.remember(WARP_KEY).remember_for(Lifetime::Run) } else { dialog }
    });
    let cancel: Box<dyn FnOnce(&mut VectorcraftApp)> = Box::new(|app| {
        if let Some(open) = app.effect_dialogs.effect.take() {
            open.preview.cancel(app);
        }
    });
    let options = HeldOptions { cancel: Some(cancel), ..Default::default() };
    let dialog_id = open_held_with(app, dialog, move |app, result| finish(app, &ctx, result), options).ok_or("no window")?;
    open.id = dialog_id;
    open.asking = false;
    app.effect_dialogs.effect = Some(open);
    Ok(Value::Null)
}

/// What OK runs: the command keeps the preview as one undo step. An `Err` keeps the dialog open
/// with the reason, and the record with a fresh preview.
fn finish(app: &mut VectorcraftApp, ctx: &egui::Context, result: &ParamResult) -> Result<(), String> {
    let Some(mut open) = app.effect_dialogs.effect.take() else { return Ok(()) };
    let (cmd, params) = command(&open, &result.fields);
    let preview = std::mem::take(&mut open.preview);
    match preview.commit(app, cmd, params.clone()) {
        Ok(_) => {
            // Last Effect repeats applied effects, not edits.
            if cmd == "effect.apply" {
                let effect = params["effect"].as_str().unwrap_or(&open.cx.id).to_string();
                app.last_effect = Some((effect, params["params"].clone()));
            }
            if super::warp_effect::is_warp(&open.cx.id) {
                result.remember(ctx);
            }
            Ok(())
        }
        Err(e) => {
            app.effect_dialogs.effect = Some(open);
            Err(e)
        }
    }
}

/// What OK runs: `effect.setParams` on the edited effect, else `effect.apply` of the dialog's
/// effect, with the rows whose value differs from the opening values.
fn command(open: &Open, fields: &DialogFields) -> (&'static str, Value) {
    let rows = effect_params(&open.cx, fields);
    let mut params = form::json_of(&rows, fields);
    params.retain(|k, v| open.opened.get(k) != Some(v));
    // The Style row's companion says the user chose a style: the other targets keep their own
    // Warp ids otherwise.
    let style = (fields.bool("styleChosen") == Some(true)).then(|| fields.text("warpStyle").map(str::to_string)).flatten();
    params.remove("warpStyle");
    params.remove("styleChosen");
    if let Some(index) = open.index {
        let mut p = if open.edit_target.is_object() { open.edit_target.clone() } else { json!({}) };
        p["index"] = json!(index);
        p["item"] = open.item.clone().unwrap_or(Value::Null);
        p["params"] = Value::Object(params);
        if let Some(style) = style {
            p["warpStyle"] = json!(style);
        }
        if open.aligned {
            p["aligned"] = json!(true);
        }
        return ("effect.setParams", p);
    }
    let effect = match style {
        Some(style) if super::warp_effect::is_warp(&open.cx.id) => format!("warp.{style}"),
        _ => open.cx.id.clone(),
    };
    let mut p = json!({"effect": effect, "params": params});
    if let Some(item) = &open.item {
        p["item"] = item.clone();
    }
    ("effect.apply", p)
}

/// Each frame: the answer of the question, then the open dialog's live preview, colour and file
/// requests.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let _ = ctx;
    let Some(open) = app.effect_dialogs.effect.take() else { return };
    let open = if open.asking {
        match answer(app, open) {
            Some(open) => open,
            None => return,
        }
    } else {
        open
    };
    drive(app, open);
}

/// The answer of the Edit or Add New question; `open` back while it is still asked.
fn answer(app: &mut VectorcraftApp, mut open: Open) -> Option<Open> {
    let result = app.dialogs().take_by_id::<StandardPrompt>(open.id)?;
    let choice = match result {
        DialogResult::Ok(answer) => answer.choice,
        DialogResult::Cancel => PromptChoice::Cancel,
    };
    let id = open.cx.id.clone();
    let launched = match (choice, open.index) {
        (PromptChoice::Proceed, Some(k)) => {
            let applied = app.session.appearance_effect_target(&open.edit_target).map(|(_, effects)| effects.to_vec()).unwrap_or_default();
            let stored = applied.get(k).map(params_of).unwrap_or_default();
            open.item = Some(open.target.clone());
            open.cx.editing = true;
            launch(app, open, stored, Map::new())
        }
        (PromptChoice::Alternate, _) => {
            open.index = None;
            launch(app, open, defaults_of(&id), Map::new())
        }
        _ => return None,
    };
    if let Err(e) = launched {
        app.status(e);
    }
    None
}

/// One frame of the open dialog: the preview of its values, and the requests its rows filed.
fn drive(app: &mut VectorcraftApp, mut open: Open) {
    let Some(dialog) = app.dialogs().get_mut_by_id::<HostedParamDialog>(open.id) else {
        app.effect_dialogs.effect = Some(open);
        return;
    };
    let (fields, previewing) = (dialog.fields().clone(), dialog.is_previewing());
    let (color, path) = (dialog.pick_color(), dialog.pick_path());
    let (cmd, params) = command(&open, &fields);
    let english = vectorcraft_effects::effect_info(&open.cx.id).map(|i| i.label.trim_end_matches('…').to_string()).unwrap_or_default();
    // `run` reports a failed preview itself.
    let _ = open.preview.run(app, previewing, &english, cmd, &params);
    if let Some(request) = color {
        // PLAN: assumed `color_picker::open_child(app, parent, request)`, which answers through `DialogHost::answer_color` (color-w12)
        super::color_picker::open_child(app, open.id, request);
    }
    if let Some(request) = path {
        ask_texture(app, open.id, request.key);
    }
    app.effect_dialogs.effect = Some(open);
}

/// The files area's open pick for Load Texture…: the image goes into the dialog's `key` row and its
/// `textureImage`.
fn ask_texture(app: &mut VectorcraftApp, dialog: DialogId, key: String) {
    use vectorcraft_engine::cmd::fileio;
    let mut request = PickRequest::open("texture");
    for f in fileio::FORMATS.iter().filter(|f| f.read && f.raster) {
        request = request.filter(f.label, fileio::read_extensions(f));
    }
    file_asks::ask(app, request, Owner::Document, move |app: &mut VectorcraftApp, picked: Picked| {
        match texture_fields(app, &picked) {
            Ok(Some(pairs)) => {
                if let Some(d) = app.dialogs().get_mut_by_id::<HostedParamDialog>(dialog) {
                    for (k, v) in pairs {
                        d.set_field(k, v);
                    }
                    if let Some(path) = picked.first_path() {
                        d.answer_path(key, path);
                    }
                }
            }
            Ok(None) => {}
            Err(e) => app.status(e),
        }
    });
}

/// The fields a picked texture image sets: `textureImage` as the effect stores it and `texture`
/// at Loaded Texture. `None` when the pick was cancelled.
pub(super) fn texture_fields(app: &VectorcraftApp, picked: &Picked) -> Result<Option<[(&'static str, FieldValue); 2]>, String> {
    use vectorcraft_engine::cmd::fileio;
    // PLAN: assumed `Picked::{Paths, Contents, Cancelled, Unavailable}` and `app.files().read(path)` (files area, fl.A1)
    let bytes = match picked {
        Picked::Cancelled => return Ok(None),
        Picked::Unavailable(cause) => return Err(cause.to_string()),
        Picked::Paths(paths) => {
            let path = paths.first().ok_or("no file picked")?;
            app.files().read(path).map_err(|e| e.to_string())?
        }
        Picked::Contents(files) => files.first().ok_or("no file picked")?.bytes.clone(),
    };
    let stored = fileio::texture_image(&bytes).map_err(|e| e.to_string())?;
    Ok(Some([
        (vectorcraft_effects::pixel::TEXTURE_IMAGE, FieldValue::Text(stored)),
        ("texture", FieldValue::Text(vectorcraft_effects::pixel::LOADED.to_string())),
    ]))
}

#[cfg(test)]
mod tests {
    use super::super::effect_params::{DROP_SHADOW, GLASS, RADIAL_BLUR, ROUGH_PASTELS, TEXTURIZER, choices, own_choices, shown_choices};
    use super::*;

    // PLAN: assumed test harness: an app with a window (`egui_context`) and `dialogs::host::pump`
    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app
    }

    fn rect(app: &mut VectorcraftApp, x: u32) -> u64 {
        app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap()
    }

    fn id(app: &VectorcraftApp) -> DialogId {
        app.effect_dialogs.effect.as_ref().unwrap().id
    }

    fn set(app: &mut VectorcraftApp, key: &str, value: Value) {
        let (ctx, id) = (app.egui_context().unwrap(), id(app));
        app.dialogs().set_json_field(&ctx, id, key, value).unwrap();
    }

    fn number(app: &mut VectorcraftApp, key: &str) -> Option<f64> {
        let id = id(app);
        app.dialogs().get_mut_by_id::<HostedParamDialog>(id).unwrap().fields().number(key)
    }

    fn ok(app: &mut VectorcraftApp) {
        let (ctx, id) = (app.egui_context().unwrap(), id(app));
        assert!(app.dialogs().confirm_by_id(&ctx, id));
        super::super::host::pump(app, &ctx);
    }

    fn params(app: &VectorcraftApp, node: u64) -> Value {
        app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(node)).unwrap().appearance.effects[0].params.clone()
    }

    fn steps(app: &VectorcraftApp) -> usize {
        app.session.doc().unwrap().history.undo.len()
    }

    /// An applied effect's dialog opened on a shared row of a mixed selection (`aligned`) edits
    /// each object's own copy on OK: the row changed is sent, every other value stays as stored.
    #[test]
    fn a_shared_rows_dialog_edits_each_objects_copy() {
        let mut app = app();
        let (a, b) = (rect(&mut app, 10), rect(&mut app, 80));
        for (node, detail) in [(a, 3), (b, 8)] {
            app.run("effect.apply", json!({"ids": [node], "effect": "distort.roughen", "item": null, "params": {"size": 5, "detail": detail}})).unwrap();
        }
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        app.run("effect.dialog", json!({"effect": "distort.roughen", "index": 0, "item": null, "aligned": true})).unwrap();
        set(&mut app, "size", json!(20));
        ok(&mut app);
        let (pa, pb) = (params(&app, a), params(&app, b));
        assert_eq!((pa["size"].as_f64(), pb["size"].as_f64()), (Some(20.0), Some(20.0)));
        assert_eq!((pa["detail"].as_f64(), pb["detail"].as_f64()), (Some(3.0), Some(8.0)), "each keeps its untouched value");
    }

    /// The lists the dialogs draw hold the values the effect's parameter documentation lists, in
    /// its order; Rasterize's Color Model names the document's mode first.
    #[test]
    fn lists_drawn_in_a_dialogs_body_are_the_documented_values() {
        let documented = |effect: &str, key: &str| {
            let doc = vectorcraft_effects::effect_info(effect).unwrap().params;
            doc.split(&format!("{key}: ")).nth(1).unwrap().split([',', '}', ' ']).next().unwrap().replace('"', "")
        };
        for (effect, key) in [
            ("path.offsetPath", "joins"),
            ("distort.zigZag", "points"),
            ("distort.roughen", "points"),
            ("stylize.innerGlow", "source"),
            (DROP_SHADOW, "shade"),
            ("artistic.paintDaubs", "brushType"),
        ] {
            let values: Vec<&str> = choices(effect, key).unwrap().iter().map(|(_, v)| *v).collect();
            assert_eq!(values.join("|"), documented(effect, key), "{effect} {key}");
        }
        for (effect, key) in [
            (vectorcraft_effects::REVOLVE, "edge"),
            (vectorcraft_effects::EXTRUDE, "bevel"),
            (vectorcraft_effects::CLASSIC_EXTRUDE, "bevel"),
            (vectorcraft_effects::CLASSIC_EXTRUDE, "surface"),
            (vectorcraft_effects::CLASSIC_EXTRUDE, "shadingColor"),
            (vectorcraft_effects::CLASSIC_REVOLVE, "edge"),
            (vectorcraft_effects::CLASSIC_REVOLVE, "surface"),
            (vectorcraft_effects::CLASSIC_REVOLVE, "shadingColor"),
            (vectorcraft_effects::CLASSIC_ROTATE, "surface"),
            (vectorcraft_effects::CLASSIC_ROTATE, "shadingColor"),
            ("warp.arc", "orientation"),
            ("warp.twist", "orientation"),
            (vectorcraft_effects::RASTERIZE, "background"),
            (vectorcraft_effects::RASTERIZE, "antiAlias"),
            ("adjust.levels", "channel"),
            ("adjust.curves", "channel"),
        ] {
            let values: Vec<&str> = own_choices(effect, key, false).unwrap().iter().map(|(_, v)| *v).collect();
            assert_eq!(values.join("|"), documented(effect, key), "{effect} {key}");
        }
    }

    /// Glass, Texturizer and Rough Pastels: a picked image goes into `textureImage` and
    /// `texture: "loaded"`, the Texture options end with Loaded Texture, and OK applies both.
    #[test]
    fn load_texture_reads_the_picked_image_into_the_effect() {
        let mut png = Vec::new();
        let img: image::DynamicImage = image::RgbaImage::from_fn(6, 4, |x, y| image::Rgba([(x * 40) as u8, (y * 60) as u8, 90, 255])).into();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        for effect in [GLASS, TEXTURIZER, ROUGH_PASTELS] {
            let mut app = app();
            let node = rect(&mut app, 10);
            app.run("select.all", json!({})).unwrap();
            app.run("effect.dialog", json!({"effect": effect})).unwrap();
            // PLAN: assumed constructor of a pick answered with the file's bytes
            let picked = Picked::contents_of(vec![("texture.png".to_string(), png.clone())]);
            let pairs = texture_fields(&app, &picked).unwrap().unwrap();
            let dialog = id(&app);
            let d = app.dialogs().get_mut_by_id::<HostedParamDialog>(dialog).unwrap();
            for (k, v) in pairs {
                d.set_field(k, v);
            }
            let stored = d.fields().text(vectorcraft_effects::pixel::TEXTURE_IMAGE).unwrap().to_string();
            assert_eq!(d.fields().text("texture"), Some("loaded"), "{effect}");
            let mut values = Map::new();
            values.insert(vectorcraft_effects::pixel::TEXTURE_IMAGE.into(), json!(stored));
            let shown = shown_choices(effect, "texture", &values, false).unwrap();
            assert_eq!(shown.last().map(|(_, v)| v.as_str()), Some("loaded"), "{effect}");
            ok(&mut app);
            let p = params(&app, node);
            assert_eq!((&p["texture"], &p["textureImage"]), (&json!("loaded"), &json!(stored)), "{effect}");
        }
    }

    /// A Pathfinder effect opens with its own Pathfinder Options (or, saved without them, the
    /// values it evaluates with); OK with nothing changed records no step, a change one step.
    #[test]
    fn a_pathfinder_effect_opens_pathfinder_options_with_its_own_values() {
        let mut app = app();
        rect(&mut app, 10);
        rect(&mut app, 40);
        app.run("select.all", json!({})).unwrap();
        app.run("effect.apply", json!({"effect": "pathfinder.add", "params": {"precision": 0.5}})).unwrap();
        let g = app.session.doc().unwrap().selection.objects[0];
        let open = |app: &mut VectorcraftApp| app.run("effect.dialog", json!({"effect": "pathfinder.add", "index": 0, "item": null})).unwrap();
        open(&mut app);
        let dialog = id(&app);
        let fields = app.dialogs().get_mut_by_id::<HostedParamDialog>(dialog).unwrap().fields().clone();
        assert!(fields.contains("removeRedundantPoints") && fields.contains("removeUnpainted"));
        assert_eq!(fields.number("precision"), Some(0.5));
        let before = steps(&app);
        ok(&mut app);
        assert_eq!(steps(&app), before, "OK with nothing changed records no step");
        open(&mut app);
        set(&mut app, "removeRedundantPoints", json!(true));
        ok(&mut app);
        assert_eq!(steps(&app), before + 1, "one step");
        assert_eq!(app.session.doc().unwrap().doc.node(g).unwrap().appearance.effects[0].params["removeRedundantPoints"], json!(true));
        // Saved without options, it opens with the values it evaluates with.
        app.session
            .edit("t", |d, _| {
                d.node_mut(g).unwrap().appearance.effects[0].params = json!({});
                Ok(())
            })
            .unwrap();
        open(&mut app);
        let dialog = id(&app);
        let fields = app.dialogs().get_mut_by_id::<HostedParamDialog>(dialog).unwrap().fields().clone();
        assert_eq!((fields.number("precision"), fields.bool("removeRedundantPoints"), fields.bool("removeUnpainted")), (Some(0.01), Some(false), Some(false)));
        // A new Soft Mix takes the panel's options and shows Mixing Rate beside them.
        app.run("object.pathfinder.options", json!({"removeUnpainted": true})).unwrap();
        app.run("effect.dialog", json!({"effect": "pathfinder.softMix"})).unwrap();
        let dialog = id(&app);
        let fields = app.dialogs().get_mut_by_id::<HostedParamDialog>(dialog).unwrap().fields().clone();
        assert!(fields.contains("mixingRate") || fields.contains("rate"), "Mixing Rate beside the options");
        assert_eq!(fields.bool("removeUnpainted"), Some(true));
    }

    /// A Pathfinder effect whose precision Scale Strokes & Effects took past the range opens at
    /// the bound it evaluates at. OK with nothing changed or another option changed keeps the
    /// stored precision, and a precision that differs from the opened one replaces it; typing the
    /// bound itself is the opened value, so the stored one stays.
    #[test]
    fn a_pathfinder_precision_scaled_past_its_range_opens_at_the_bound_and_ok_keeps_it() {
        let mut app = app();
        rect(&mut app, 10);
        rect(&mut app, 40);
        app.run("select.all", json!({})).unwrap();
        app.run("effect.apply", json!({"effect": "pathfinder.add", "params": {"precision": 8}})).unwrap();
        let g = app.session.doc().unwrap().selection.objects[0];
        app.run("object.scale", json!({"sx": 200, "strokes": true})).unwrap();
        let stored = |app: &VectorcraftApp| app.session.doc().unwrap().doc.node(g).unwrap().appearance.effects[0].params.clone();
        assert_eq!(stored(&app)["precision"].as_f64(), Some(16.0));
        let open = |app: &mut VectorcraftApp| {
            app.run("effect.dialog", json!({"effect": "pathfinder.add", "index": 0, "item": null})).unwrap();
            assert_eq!(number(app, "precision"), Some(10.0));
        };
        open(&mut app);
        let before = steps(&app);
        ok(&mut app);
        assert_eq!((steps(&app), stored(&app)["precision"].as_f64()), (before, Some(16.0)), "OK with nothing changed");
        open(&mut app);
        set(&mut app, "removeRedundantPoints", json!(true));
        ok(&mut app);
        assert_eq!((stored(&app)["precision"].as_f64(), &stored(&app)["removeRedundantPoints"]), (Some(16.0), &json!(true)));
        open(&mut app);
        set(&mut app, "precision", json!(2.5));
        ok(&mut app);
        assert_eq!(stored(&app)["precision"].as_f64(), Some(2.5));
        app.run("object.scale", json!({"sx": 800, "strokes": true})).unwrap();
        assert_eq!(stored(&app)["precision"].as_f64(), Some(20.0));
        open(&mut app);
        set(&mut app, "precision", json!(10.0));
        ok(&mut app);
        assert_eq!(stored(&app)["precision"].as_f64(), Some(20.0), "the bound typed is the opened value, which is not sent");
    }

    /// A Radial Blur whose saved Blur Center lies outside the object's bounds opens with the
    /// center at the nearest edge. OK with nothing changed sends no center, so the stored value is
    /// neither refused nor rewritten; a typed center replaces it.
    #[test]
    fn radial_blur_center_outside_the_bounds_opens_at_the_edge_and_ok_keeps_it() {
        let mut app = app();
        let node = rect(&mut app, 10);
        app.run("select.all", json!({})).unwrap();
        app.run("effect.apply", json!({"effect": RADIAL_BLUR, "params": {"amount": 20}})).unwrap();
        {
            let st = app.session.doc_mut().unwrap();
            let d = std::sync::Arc::make_mut(&mut st.doc);
            let fx = &mut d.node_mut(vectorcraft_doc::NodeId(node)).unwrap().appearance.effects[0];
            fx.params["centerX"] = json!(1.5);
            fx.params["centerY"] = json!(-0.25);
        }
        app.run("effect.dialog", json!({"effect": RADIAL_BLUR, "index": 0})).unwrap();
        // PLAN: assumed row keys `centerX` and `centerY` of effects-w5's Blur Center row
        assert_eq!((number(&mut app, "centerX"), number(&mut app, "centerY")), (Some(1.0), Some(0.0)), "the rows hold the center the box shows");
        let before = steps(&app);
        ok(&mut app);
        let p = params(&app, node);
        assert_eq!((steps(&app), p["centerX"].as_f64(), p["centerY"].as_f64()), (before, Some(1.5), Some(-0.25)), "nothing sent, nothing rewritten");
        app.run("effect.dialog", json!({"effect": RADIAL_BLUR, "index": 0})).unwrap();
        set(&mut app, "centerX", json!(0.5));
        ok(&mut app);
        assert_eq!(params(&app, node)["centerX"].as_f64(), Some(0.5));
    }
}
