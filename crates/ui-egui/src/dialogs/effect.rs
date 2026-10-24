//! Live effect dialogs (Effect menu, Appearance panel): the effect's parameters (`__effect`, headed
//! `__label`) with a live Preview that OK keeps as one undo step and Cancel rolls back. `__item`,
//! when present, is the appearance item the effect goes to (else the Appearance panel's active
//! item applies). With `__index` the dialog edits that applied effect in place
//! (`effect.setParams`) instead of adding one (`effect.apply`).
//!
//! Choosing an effect that the target already carries first asks ([`EXISTS`]) whether to edit it
//! or add another one.

use serde_json::{Map, Value, json};

use vectorcraft_color::Color;

use super::{DialogSpec, form, free_distort, transform_each};
use crate::panels::c32;
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |d| if super::warp_effect::is_warp(&d.str("__effect")) { tl!("Warp Options").to_string() } else { heading_label(d) },
    body,
    confirm,
    preview: true,
    ..DialogSpec::FORM
};

/// The effect's name (`__label`, English) in the UI language: the catalog may carry it with or
/// without its trailing ellipsis. A plug-in effect's name is shown as it is.
fn heading_label(d: &Dialog) -> String {
    let label = d.str("__label");
    if vectorcraft_plugins::effect::plugin_id(&d.str("__effect")).is_some() {
        return label;
    }
    let dotted = format!("{label}…");
    let shown = tl!(&dotted);
    if shown != dotted { shown.trim_end_matches('…').to_string() } else { tl!(&label).to_string() }
}

/// The dialog kind asking whether to edit an effect that is already applied or add another.
/// Fields: `__effect`, `__label`, `__item?` (as given to `effect.dialog`), `__target` and
/// `__index` (the applied effect's item and position). OK edits it; `discard: true` then OK adds
/// a new one.
pub const EXISTS: &str = "effectExists";

pub(super) const EXISTS_SPEC: DialogSpec = DialogSpec {
    heading: |d| crate::i18n::fmt(tl!("{effect} is already applied"), &[("effect", &heading_label(d))]),
    body: |_, ui, _| {
        ui.label(egui::RichText::new(tl!("Edit the applied effect, or add another one?")).color(Tokens::get(ui.ctx()).text_dim));
        false
    },
    confirm: confirm_exists,
    ok: Some("Edit"),
    discard: Some("Add New"),
    max_width: Some(420.0),
    ..DialogSpec::FORM
};

/// `effect.dialog {effect, index?, item?}`: edit applied effect `index` of `item` (null: the
/// object's effects; omitted: the active item's, else the object's) prefilled with its values,
/// or add `effect`, first asking when that list already has it.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let id = p.get("effect").and_then(Value::as_str).unwrap_or_default();
    let info = vectorcraft_effects::effect_info(id).ok_or_else(|| format!("unknown effect `{id}`"))?;
    // The list the effect goes to: `item` as given, else the active item's (the object's without
    // one). A new effect keeps `item` unresolved, so `effect.apply` targets each object as usual.
    let target = p.get("item").cloned().unwrap_or_else(|| json!(app.session.appearance_item()));
    let applied = target_effects(app, &target);
    let mut fields = Map::new();
    fields.insert("__effect".into(), json!(id));
    fields.insert("__label".into(), json!(info.label.trim_end_matches('…')));
    if let Some(item) = p.get("item") {
        fields.insert("__item".into(), item.clone());
    }
    match p.get("index").and_then(Value::as_u64) {
        Some(k) => {
            let e = applied.get(k as usize).ok_or_else(|| format!("no effect at index {k}"))?;
            if e.id != id {
                return Err(format!("effect {k} is `{}`, not `{id}`", e.id));
            }
            fields.insert("__item".into(), target);
            fields.insert("__index".into(), json!(k));
            fields.extend(params_of(e));
        }
        None => {
            if let Some(k) = applied.iter().rposition(|e| e.id == id) {
                fields.insert("__target".into(), target);
                fields.insert("__index".into(), json!(k));
                app.ui.dialog = Some(Dialog { kind: EXISTS.into(), fields });
                return Ok(json!({ "pending": EXISTS }));
            }
            fields.extend(info.defaults.as_object().cloned().unwrap_or_default());
            if super::warp_effect::is_warp(id) {
                super::warp_effect::merge_remembered(&mut fields, app.last_warp.as_ref());
            }
        }
    }
    fields.insert("preview".into(), json!(true));
    app.ui.dialog = Some(Dialog { kind: "effect".into(), fields });
    Ok(Value::Null)
}

/// The effects of the first selected object that `item` (an index or null) addresses.
fn target_effects(app: &VectorcraftApp, item: &Value) -> Vec<vectorcraft_doc::Effect> {
    let Some(st) = app.session.active() else { return vec![] };
    let node = st.selection.objects.first().and_then(|id| st.doc.node(*id));
    node.and_then(|n| n.appearance.effects_at(item.as_u64().map(|i| i as usize))).cloned().unwrap_or_default()
}

/// An applied effect's values over its defaults.
fn params_of(e: &vectorcraft_doc::Effect) -> Map<String, Value> {
    match vectorcraft_effects::merged_params(&e.id, &e.params) {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

/// OK in [`EXISTS`]: the dialog editing the applied effect, or (`discard`) a fresh one.
fn confirm_exists(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut fields = d.fields.clone();
    fields.remove("discard");
    let target = fields.remove("__target").unwrap_or(Value::Null);
    if d.bool("discard") {
        fields.remove("__index");
        let id = d.str("__effect");
        let defaults = vectorcraft_effects::default_params(&id);
        fields.extend(defaults.and_then(|v| v.as_object().cloned()).unwrap_or_default());
        if super::warp_effect::is_warp(id.as_str()) {
            super::warp_effect::merge_remembered(&mut fields, app.last_warp.as_ref());
        }
    } else {
        let k = fields.get("__index").and_then(Value::as_u64).unwrap_or(0) as usize;
        fields.extend(target_effects(app, &target).get(k).map(params_of).unwrap_or_default());
        fields.insert("__item".into(), target);
    }
    fields.insert("preview".into(), json!(true));
    app.ui.dialog = Some(Dialog { kind: "effect".into(), fields });
    Ok(Value::Null)
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let (id, relative) = (d.str("__effect"), d.bool("relative"));
    if super::warp_effect::is_warp(&id) {
        super::warp_effect::body(ui, d);
        let (cmd, params) = command(d);
        form::preview(app, ui, d, "Warp Options", cmd, params);
        return false;
    }
    if vectorcraft_effects::is_three_d(&id) {
        super::revolve::body(app, ui, d);
        let (cmd, params) = command(d);
        form::preview(app, ui, d, &d.str("__label"), cmd, params);
        return false;
    }
    // Plug-in effects get fields from their parameter schema; colour adjustments their sliders.
    let changed = match vectorcraft_plugins::effect::installed(&id) {
        Some(plugin) => form::schema_fields(app, ui, d, plugin.manifest()),
        None if vectorcraft_effects::is_adjustment(&id) => adjust_fields(ui, d),
        None if id == GLASS => glass_fields(app, ui, d, GLASS),
        None if id == TEXTURIZER => glass_fields(app, ui, d, TEXTURIZER),
        None if id == TRANSFORM => transform_fields(ui, d, app.session.general_unit()),
        None if id == ZIG_ZAG => zig_zag_fields(ui, d, app.session.general_unit()),
        None if id == DROP_SHADOW => drop_shadow_fields(ui, d, app.session.general_unit()),
        None if id == "artistic.filmGrain" => film_grain_fields(ui, d),
        None if id == free_distort::FREE_DISTORT => {
            free_distort::fields(app, ui, d);
            values_changed(d)
        }
        None if id == vectorcraft_effects::RASTERIZE => {
            super::raster_effects::effect_fields(app, ui, d);
            values_changed(d)
        }
        None => {
            let doc = vectorcraft_effects::effect_info(&id).map(|e| e.params).unwrap_or_default();
            let unit = app.session.general_unit();
            let changed =
                form::param_fields(ui, d, &|k| vectorcraft_effects::is_length(&id, k, relative), &|k| choices(&id, k), &|k| doc_rank(doc, k), unit);
            // The fields show what the effect stores: Scribble's 270° angle reads -90°.
            if let Value::Object(m) = vectorcraft_effects::upgrade_params(&id, &Value::Object(d.fields.clone())) {
                d.fields = m;
            }
            changed
        }
    };
    ui.add_space(6.0);
    let pv_changed = widgets::check(ui, tl!("Preview"), d.bool("preview"), true);
    let pv = d.bool("preview") != pv_changed;
    d.fields.insert("preview".into(), json!(pv));
    if pv && (changed || pv_changed || !app.session.in_interaction()) {
        let label = d.str("__label");
        let _ = app.session.begin_interaction(&label);
        let (cmd, p) = command(d);
        let _ = app.session.preview(cmd, &p);
    } else if !pv && pv_changed {
        let _ = app.session.cancel_interaction();
    }
    false
}

/// Film Grain's bounded, whole-number controls, in the same order as its parameter docs.
pub(crate) const FILM_GRAIN_FIELDS: [(&str, &str, f64); 3] =
    [("grain", "Grain", 20.0), ("highlightArea", "Highlight Area", 20.0), ("intensity", "Intensity", 10.0)];

/// Paint Daubs › Brush Type labels ([`vectorcraft_effects::pixel::PAINT_DAUB_BRUSH_TYPES`]); extend
/// [`crate::menus::menu_strings`] with these like [`FILM_GRAIN_FIELDS`].
pub(crate) const PAINT_DAUB_BRUSH_TYPE_LABELS: [&str; 6] = ["Simple", "Light Rough", "Dark Rough", "Wide Sharp", "Wide Blurry", "Sparkle"];

fn film_grain_fields(ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let before = d.fields.clone();
    // Use one column wide enough for this language's longest label, keeping every rail aligned.
    let font = egui::TextStyle::Body.resolve(ui.style());
    let text = Tokens::get(ui.ctx()).text;
    let label_w = FILM_GRAIN_FIELDS
        .iter()
        .map(|(_, label, _)| ui.painter().layout_no_wrap(tl!(label).to_string(), font.clone(), text).size().x)
        .fold(form::SLIDER_LABEL, f32::max)
        .ceil()
        + 4.0;
    for (key, label, max) in FILM_GRAIN_FIELDS {
        form::slider_w(ui, d, (key, label, label_w), 0.0..=max, "", &grey_rail);
    }
    before != d.fields
}

/// The Glass effect (Effect › Distort › Glass…).
const GLASS: &str = "distort.glass";
/// The Texturizer effect (Effect › Texture › Texturizer…).
const TEXTURIZER: &str = "texture.texturizer";

/// Glass and Texturizer: parameters, the Texture menu ending with Loaded Texture once a texture is
/// loaded, and Load Texture… with the loaded texture's size. Returns whether the values differ from
/// those last previewed.
fn glass_fields(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog, id: &str) -> bool {
    use vectorcraft_effects::pixel::{TEXTURE_IMAGE, texture};
    // The stored texture is shown as its size, not as a text field.
    let stored = d.fields.remove(TEXTURE_IMAGE);
    let size = stored.as_ref().and_then(Value::as_str).and_then(texture).map(|t| t.size());
    let menu = if id == TEXTURIZER { texturizer_textures(size.is_some()) } else { glass_textures(size.is_some()) };
    let doc = vectorcraft_effects::effect_info(id).map(|e| e.params).unwrap_or_default();
    let unit = app.session.general_unit();
    let pick = |k: &str| if k == "texture" { Some(menu) } else { choices(id, k) };
    form::param_fields(ui, d, &|k| vectorcraft_effects::is_length(id, k, false), &pick, &|k| doc_rank(doc, k), unit);
    if let Some(v) = stored {
        d.fields.insert(TEXTURE_IMAGE.into(), v);
    }
    let mut load = false;
    ui.horizontal(|ui| {
        let can_read = app.services.read.is_some();
        load = ui.add_enabled_ui(can_read, |ui| widgets::flat_button(ui, tl!("Load Texture…"), 110.0)).inner.clicked();
        if let Some((w, h)) = size {
            widgets::dim_label(ui, &format!("{w} × {h} px"));
        }
    });
    if load
        && let Err(e) = crate::picks::in_dialog(app, d, load_texture)
        && e != "cancelled"
    {
        app.status(e);
    }
    values_changed(d)
}

/// Glass's Texture menu, ending with Loaded Texture once a texture is `loaded`.
fn glass_textures(loaded: bool) -> form::Choices {
    use vectorcraft_effects::pixel::{GLASS_TEXTURES, GLASS_TEXTURES_LOADED};
    if loaded { &GLASS_TEXTURES_LOADED } else { &GLASS_TEXTURES }
}

/// Texturizer's Texture menu, ending with Loaded Texture once a texture is `loaded`.
fn texturizer_textures(loaded: bool) -> form::Choices {
    use vectorcraft_effects::pixel::{TEXTURES, TEXTURES_LOADED};
    if loaded { &TEXTURES_LOADED } else { &TEXTURES }
}

/// Glass's Load Texture…: an image file the user picks, read into `textureImage` as Glass stores
/// it and picked in the Texture menu.
fn load_texture(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
    use vectorcraft_engine::cmd::fileio;
    let filters = crate::FilePick::filters(fileio::FORMATS.iter().filter(|f| f.read && f.raster).map(|f| (f.label, f.extensions)));
    let path = crate::picks::open(app, &crate::FilePick { filters, ..Default::default() }).ok_or("cancelled")?;
    // The button is off where files can't be read.
    let Some(read) = app.services.read.as_ref() else { return Ok(()) };
    let stored = fileio::texture_image(&read(&path)?).map_err(|e| e.to_string())?;
    d.fields.insert(vectorcraft_effects::pixel::TEXTURE_IMAGE.into(), json!(stored));
    d.fields.insert("texture".into(), json!(vectorcraft_effects::pixel::LOADED));
    Ok(())
}

/// The parameters of the effect dialogs that pick one of some values, as (label, value).
fn choices(effect: &str, key: &str) -> Option<form::Choices> {
    match (effect, key) {
        ("blur.radial", "method") => Some(&[("Spin", "spin"), ("Zoom", "zoom")]),
        ("blur.radial", "quality") => Some(&[("Draft", "draft"), ("Good", "good"), ("Best", "best")]),
        ("blur.smart", "quality") => Some(&[("Low", "low"), ("Medium", "medium"), ("High", "high")]),
        ("blur.smart", "blurMode") => Some(&vectorcraft_effects::pixel::SMART_BLUR_MODES),
        ("brushStrokes.sprayedStrokes", "strokeDirection") => Some(&vectorcraft_effects::pixel::STROKE_DIRECTIONS),
        ("distort.glass", "texture") => Some(&vectorcraft_effects::pixel::GLASS_TEXTURES),
        ("pixelate.mezzotint", "type") => Some(&vectorcraft_effects::pixel::MEZZOTINT_TYPES),
        ("artistic.paintDaubs", "brushType") => Some(&vectorcraft_effects::pixel::PAINT_DAUB_BRUSH_TYPES),
        ("sketch.graphicPen", "strokeDirection") => Some(&vectorcraft_effects::pixel::STROKE_DIRECTIONS),
        ("sketch.halftonePattern", "patternType") => Some(&vectorcraft_effects::pixel::HALFTONE_TYPES),
        ("sketch.plaster", "lightDirection") => Some(&vectorcraft_effects::pixel::LIGHT_DIRECTIONS),
        ("texture.grain", "grainType") => Some(&vectorcraft_effects::pixel::GRAIN_TYPES),
        ("texture.texturizer" | "artistic.underpainting" | "sketch.conteCrayon", "texture") => Some(&vectorcraft_effects::pixel::TEXTURES),
        ("sketch.basRelief" | "texture.texturizer" | "artistic.underpainting" | "sketch.conteCrayon", "lightDirection") => {
            Some(&vectorcraft_effects::pixel::LIGHT_DIRECTIONS)
        }
        ("video.deinterlace", "eliminate") => Some(&[("Odd Fields", "odd"), ("Even Fields", "even")]),
        ("video.deinterlace", "create") => Some(&[("Duplication", "duplication"), ("Interpolation", "interpolation")]),
        ("stylize.innerGlow", "source") => Some(&[("Center", "center"), ("Edge", "edge")]),
        ("stylize.dropShadow", "shade") => Some(SHADES),
        ("path.offsetPath", "joins") => Some(&[("Miter", "miter"), ("Round", "round"), ("Bevel", "bevel")]),
        ("distort.roughen" | "distort.zigZag", "points") => Some(&[("Smooth", "smooth"), ("Corner", "corner")]),
        _ => None,
    }
}

/// The parameters whose dialogs list their values in their own bodies, as (label, value), each
/// label as that dialog passes it to its dropdown or radio buttons, which translate it into the UI
/// language: Revolve's edge, Extrude & Bevel's bevel (Rounded translated in the "3d" context
/// first), every Warp style's orientation, Rasterize's Color Model (its `document` value labeled
/// CMYK when `cmyk`, for a document in CMYK, and RGB otherwise), Background and Anti-aliasing, and
/// the channel of Levels and Curves.
pub(super) fn own_choices(effect: &str, key: &str, cmyk: bool) -> Option<Vec<(&'static str, &'static str)>> {
    let c3 = |s: &'static str| crate::i18n::tr_ctx(crate::i18n::current(), "3d", s);
    Some(match (effect, key) {
        (vectorcraft_effects::REVOLVE, "edge") => vec![(tl!("Left"), "left"), (tl!("Right"), "right")],
        (vectorcraft_effects::EXTRUDE, "bevel") => vec![
            (tl!("None"), "none"),
            (tl!("Classic"), "classic"),
            (c3("Rounded"), "rounded"),
            (tl!("Cove"), "cove"),
            (tl!("Step"), "step"),
            (tl!("Ogee"), "ogee"),
        ],
        (_, "orientation") if super::warp_effect::is_warp(effect) => vec![(tl!("Horizontal"), "horizontal"), (tl!("Vertical"), "vertical")],
        (vectorcraft_effects::RASTERIZE, "colorModel") => {
            vec![(if cmyk { "CMYK" } else { "RGB" }, "document"), (tl!("Grayscale"), "grayscale"), (tl!("Bitmap"), "bitmap")]
        }
        (vectorcraft_effects::RASTERIZE, "background") => vec![(tl!("White"), "white"), (tl!("Transparent"), "transparent")],
        (vectorcraft_effects::RASTERIZE, "antiAlias") => vectorcraft_render::AntiAlias::ALL.iter().map(|a| (a.label(), a.id())).collect(),
        ("adjust.levels" | "adjust.curves", "channel") => vec![("RGB", "rgb"), (tl!("Red"), "red"), (tl!("Green"), "green"), (tl!("Blue"), "blue")],
        _ => return None,
    })
}

/// The values that parameter `key` of applied effect `effect` picks from, as its dialog lists
/// them, as (label, value): our own effects' labels in the UI language, a plug-in's choices as the
/// plug-in names them. Glass's Texture menu ends with Loaded Texture when the effect's values
/// `params` hold a loaded texture, and the lists of [`own_choices`] are translated as their
/// dialogs translate them (`cmyk`: the document is in CMYK). `None` for a parameter that is not one
/// of these lists.
pub(crate) fn shown_choices(effect: &str, key: &str, params: &Map<String, Value>, cmyk: bool) -> Option<Vec<(String, String)>> {
    if let Some(plugin) = vectorcraft_plugins::effect::installed(effect) {
        return plugin.manifest().param(key).and_then(|p| match &p.spec {
            vectorcraft_plugins::ParamSpec::Choice { options, .. } => Some(options.iter().map(|o| (o.clone(), o.clone())).collect()),
            _ => None,
        });
    }
    if let Some(own) = own_choices(effect, key, cmyk) {
        return Some(own.into_iter().map(|(label, value)| (tl!(label).to_string(), value.to_string())).collect());
    }
    let options = if (effect == GLASS || effect == TEXTURIZER) && key == "texture" {
        use vectorcraft_effects::pixel::{TEXTURE_IMAGE, texture};
        let loaded = params.get(TEXTURE_IMAGE).and_then(Value::as_str).and_then(texture).is_some();
        if effect == GLASS { glass_textures(loaded) } else { texturizer_textures(loaded) }
    } else {
        choices(effect, key)?
    };
    Some(options.iter().map(|(label, value)| (form::choice_label(label).to_string(), value.to_string())).collect())
}

/// Where parameter `key` comes in the effect's parameter documentation `doc` (`{radius: …,
/// threshold: …}`), which lists them in its dialog's order; the end when it isn't there.
pub(super) fn doc_rank(doc: &str, key: &str) -> usize {
    let starts = |at: usize| doc.get(..at).and_then(|s| s.chars().last()).is_some_and(|c| c == '{' || c == ' ');
    doc.match_indices(key)
        .map(|(at, _)| at)
        .find(|at| starts(*at) && doc.get(at + key.len()..).is_some_and(|rest| rest.starts_with(':') || rest.starts_with("?:")))
        .unwrap_or(usize::MAX)
}

/// A slider of a colour adjustment dialog: (effect, field, label, range, rail colour at 0..1).
type Slider = (&'static str, &'static str, &'static str, (f64, f64), fn(f32) -> egui::Color32);

fn rgb(r: f32, g: f32, b: f32) -> egui::Color32 {
    c32(&Color::rgb(r, g, b))
}

fn grey_rail(x: f32) -> egui::Color32 {
    rgb(x, x, x)
}

fn hue_rail(x: f32) -> egui::Color32 {
    c32(&Color::from_hsb(((x - 0.5) * 360.0).rem_euclid(360.0), 0.8, 0.9))
}

fn saturation_rail(x: f32) -> egui::Color32 {
    c32(&Color::from_hsb(0.0, x, 0.9))
}

fn temperature_rail(x: f32) -> egui::Color32 {
    rgb(0.35 + 0.65 * x, 0.55 + 0.05 * x, 1.0 - 0.8 * x)
}

fn tint_rail(x: f32) -> egui::Color32 {
    rgb(0.3 + 0.6 * x, 0.85 - 0.55 * x, 0.3 + 0.6 * x)
}

const SLIDERS: [Slider; 12] = [
    ("adjust.brightnessContrast", "brightness", "Brightness", (-100.0, 100.0), grey_rail),
    ("adjust.brightnessContrast", "contrast", "Contrast", (-100.0, 100.0), grey_rail),
    ("adjust.hueSaturation", "hue", "Hue", (-180.0, 180.0), hue_rail),
    ("adjust.hueSaturation", "saturation", "Saturation", (-100.0, 100.0), saturation_rail),
    ("adjust.hueSaturation", "lightness", "Lightness", (-100.0, 100.0), grey_rail),
    ("adjust.levels", "inputBlack", "Input Black", (0.0, 255.0), grey_rail),
    ("adjust.levels", "inputWhite", "Input White", (0.0, 255.0), grey_rail),
    ("adjust.levels", "outputBlack", "Output Black", (0.0, 255.0), grey_rail),
    ("adjust.levels", "outputWhite", "Output White", (0.0, 255.0), grey_rail),
    ("adjust.shiftToColor", "amount", "Amount", (0.0, 100.0), grey_rail),
    ("adjust.temperatureTint", "temperature", "Temperature", (-100.0, 100.0), temperature_rail),
    ("adjust.temperatureTint", "tint", "Tint", (-100.0, 100.0), tint_rail),
];

/// The colour adjustment dialogs (Effect → Color Adjustments): a slider per amount, then the
/// channel as a menu and the other values (Curves' points, Levels' gamma, the target colour) in
/// the same label column, and the options. Returns whether the values differ from those last
/// previewed.
fn adjust_fields(ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    const LABEL_W: f32 = 84.0;
    let id = d.str("__effect");
    for (_, key, label, (min, max), rail) in SLIDERS.iter().filter(|s| s.0 == id) {
        form::slider_w(ui, d, (key, label, LABEL_W), *min..=*max, "", rail);
    }
    if d.fields.contains_key("channel") {
        let channels: Vec<(&str, &str)> =
            own_choices(&id, "channel", false).unwrap_or_default().into_iter().map(|(label, value)| (value, label)).collect();
        form::choice(ui, d, "channel", tl!("Channel"), (LABEL_W, 120.0), &channels);
    }
    if d.fields.contains_key("points") {
        ui.horizontal(|ui| {
            ui.add_space(LABEL_W + ui.spacing().item_spacing.x);
            curve_graph(ui, d);
        });
    }
    for (key, label, width) in [("points", tl!("Points"), 220.0), ("color", tl!("Color"), 120.0)] {
        if d.fields.contains_key(key) {
            widgets::label_row(ui, label, LABEL_W, |ui| {
                form::text(ui, d, key, width);
            });
        }
    }
    if d.fields.contains_key("gamma") {
        widgets::label_row(ui, tl!("Gamma"), LABEL_W, |ui| {
            if let Some(g) = widgets::range_field(ui, "gamma", d.f64("gamma", 1.0), 0.1..=10.0, "", 2, 60.0) {
                d.fields.insert("gamma".into(), json!(g));
            }
        });
    }
    for (key, label) in [("colorize", tl!("Colorize")), ("preserveLightness", tl!("Preserve Lightness"))] {
        if d.fields.contains_key(key) {
            form::check(ui, d, key, label);
        }
    }
    values_changed(d)
}

/// Do the dialog's values differ from those last previewed? (Remembers them for the next frame.)
fn values_changed(d: &mut Dialog) -> bool {
    let params = form::params(d);
    let changed = d.fields.get(form::PREVIEWED) != Some(&params);
    d.fields.insert(form::PREVIEWED.into(), params);
    changed
}

/// The Transform effect (Effect › Distort & Transform › Transform…).
const TRANSFORM: &str = "distort.transform";

/// The Transform effect's dialog: Transform Each's Scale and Move sliders and Rotate dial, then
/// the options Transform Objects, Transform Patterns and Scale Strokes & Effects (#885), Copies,
/// the reflections, the reference point and Random. Returns whether the values differ from those
/// last previewed.
fn transform_fields(ui: &mut egui::Ui, d: &mut Dialog, unit: vectorcraft_doc::Unit) -> bool {
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| transform_each::sections(ui, d, unit));
        ui.add_space(16.0);
        ui.vertical(|ui| {
            widgets::subheader(ui, tl!("Options"));
            ui.add_space(4.0);
            form::check(ui, d, "objects", tl!("Transform Objects"));
            form::check(ui, d, "patterns", tl!("Transform Patterns"));
            form::check(ui, d, "strokes", tl!("Scale Strokes & Effects"));
            ui.horizontal(|ui| {
                widgets::dim_label(ui, tl!("Copies:"));
                if let Some(n) = widgets::range_field(ui, "fx-copies", d.f64("copies", 0.0), 0.0..=1000.0, "", 0, 50.0) {
                    d.fields.insert("copies".into(), json!(n));
                }
            });
            transform_each::reflect_options(ui, d);
        });
    });
    values_changed(d)
}

/// The Zig Zag effect (Effect › Distort & Transform › Zig Zag…).
const ZIG_ZAG: &str = "distort.zigZag";

/// Zig Zag's dialog (#1133): Size and Ridges per segment as sliders with their fields, Relative or
/// Absolute and the Points as radio buttons. Returns whether the values differ from those last
/// previewed.
fn zig_zag_fields(ui: &mut egui::Ui, d: &mut Dialog, unit: vectorcraft_doc::Unit) -> bool {
    const LABEL_W: f32 = 124.0;
    const FIELD_W: f32 = 70.0;
    let rail = Tokens::get(ui.ctx()).input_border;
    let track = move |_| rail;
    let relative = d.bool("relative");
    widgets::subheader(ui, tl!("Options"));
    // Size is a share of the object's size (Relative) or a distance (Absolute).
    widgets::label_row(ui, tl!("Size:"), LABEL_W, |ui| {
        let shown = if relative {
            Some(widgets::SliderValue::field(d.f64("size", 10.0).clamp(0.0, 100.0), 0.0..=100.0, "%", 0))
        } else {
            form::length_value(d, "size", unit, 0.0..=100.0)
        };
        form::slider_rail_valued(ui, d, "size", tl!("Size:"), 0.0..=100.0, shown, &track);
        if !relative {
            form::length(ui, d, "size", unit, FIELD_W);
        } else if let Some(n) = widgets::range_field(ui, ("zz", "size"), d.f64("size", 10.0), 0.0..=100.0, "%", 0, FIELD_W) {
            d.fields.insert("size".into(), json!(n));
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(LABEL_W + ui.spacing().item_spacing.x);
        for (label, on) in [(tl!("Relative"), true), (tl!("Absolute"), false)] {
            if widgets::radio(ui, label, relative == on, true) {
                d.fields.insert("relative".into(), json!(on));
            }
        }
    });
    widgets::label_row(ui, tl!("Ridges per segment:"), LABEL_W, |ui| {
        form::slider_rail_named(ui, d, "ridges", tl!("Ridges per segment:"), 0.0..=100.0, &track);
        if let Some(n) = widgets::range_field(ui, ("zz", "ridges"), d.f64("ridges", 4.0), 0.0..=100.0, "", 0, FIELD_W) {
            d.fields.insert("ridges".into(), json!(n));
        }
    });
    ui.add_space(4.0);
    widgets::subheader(ui, crate::i18n::tr_ctx(crate::i18n::current(), "effect", "Points"));
    let points = d.str("points");
    ui.horizontal(|ui| {
        for (label, value) in choices(ZIG_ZAG, "points").unwrap_or_default() {
            if widgets::radio(ui, crate::i18n::tr_ctx(crate::i18n::current(), "effect", label), points == *value, true) {
                d.fields.insert("points".into(), json!(value));
            }
        }
    });
    values_changed(d)
}

/// Drop Shadow (Effect › Stylize › Drop Shadow…).
const DROP_SHADOW: &str = "stylize.dropShadow";

/// What Drop Shadow's shadow takes its color from (`shade`), as (label, value).
pub(crate) const SHADES: form::Choices = &[("Color", "color"), ("Darkness", "darkness")];

/// Drop Shadow's dialog: Mode, Opacity, the offsets and Blur as the other effects show them, then
/// Color and Darkness as radio buttons, each with its field (the field of the one not chosen is
/// dimmed). Returns whether the values differ from those last previewed.
fn drop_shadow_fields(ui: &mut egui::Ui, d: &mut Dialog, unit: vectorcraft_doc::Unit) -> bool {
    let doc = vectorcraft_effects::effect_info(DROP_SHADOW).map(|e| e.params).unwrap_or_default();
    // The generic rows, without the three this dialog shows as radio buttons.
    let own: Vec<(String, Value)> = ["shade", "color", "darkness"].iter().filter_map(|k| d.fields.remove(*k).map(|v| (k.to_string(), v))).collect();
    form::param_fields(ui, d, &|k| vectorcraft_effects::is_length(DROP_SHADOW, k, false), &|k| choices(DROP_SHADOW, k), &|k| doc_rank(doc, k), unit);
    d.fields.extend(own);
    let darkness = d.str("shade").eq_ignore_ascii_case("darkness");
    ui.add_space(4.0);
    egui::Grid::new("fx-shade").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
        if widgets::radio(ui, tl!("Color"), !darkness, true) {
            d.fields.insert("shade".into(), json!("color"));
        }
        ui.add_enabled_ui(!darkness, |ui| match d.fields.get("color") {
            Some(Value::String(_)) | None => {
                form::text(ui, d, "color", 128.0);
            }
            Some(other) => {
                widgets::dim_label(ui, &other.to_string());
            }
        });
        ui.end_row();
        if widgets::radio(ui, tl!("Darkness"), darkness, true) {
            d.fields.insert("shade".into(), json!("darkness"));
        }
        ui.add_enabled_ui(darkness, |ui| {
            if let Some(v) = widgets::range_field(ui, "fx-darkness", d.f64("darkness", 100.0), 0.0..=100.0, "%", 0, 70.0) {
                d.fields.insert("darkness".into(), json!(v));
            }
        });
        ui.end_row();
    });
    values_changed(d)
}

/// Curves' graph of its `points` (input across, output up, 0..255): drag a point to move it
/// between its neighbours, press on the graph away from the points to add one there, drag one
/// off the graph to remove it (two always stay).
fn curve_graph(ui: &mut egui::Ui, d: &mut Dialog) {
    const SIZE: f32 = 200.0;
    /// How near (screen points) a press must be to take a point.
    const GRAB: f32 = 8.0;
    let t = Tokens::get(ui.ctx());
    let mut pts = vectorcraft_effects::curve_points(d.fields.get("points"));
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(SIZE, SIZE), egui::Sense::click_and_drag());
    let screen = |(x, y): (f32, f32)| egui::pos2(rect.left() + x * SIZE, rect.bottom() - y * SIZE);
    let unit = |p: egui::Pos2| (((p.x - rect.left()) / SIZE).clamp(0.0, 1.0), ((rect.bottom() - p.y) / SIZE).clamp(0.0, 1.0));
    let held = ui.id().with("curve-point");
    let mut grabbed: Option<usize> = ui.data(|m| m.get_temp(held)).flatten();
    let mut changed = false;
    if let Some(p) = resp.interact_pointer_pos()
        && resp.drag_started()
    {
        let near =
            pts.iter().enumerate().map(|(i, q)| (i, screen(*q).distance(p))).filter(|(_, dist)| *dist <= GRAB).min_by(|a, b| a.1.total_cmp(&b.1));
        grabbed = match near {
            Some((i, _)) => Some(i),
            None => {
                let q = unit(p);
                let i = pts.partition_point(|a| a.0 < q.0);
                pts.insert(i, q);
                changed = true;
                Some(i)
            }
        };
    }
    if let (Some(i), Some(p)) = (grabbed, resp.interact_pointer_pos())
        && resp.dragged()
    {
        if !rect.expand(24.0).contains(p) && pts.len() > 2 {
            pts.remove(i.min(pts.len() - 1));
            grabbed = None;
        } else {
            let step = 1.0 / 255.0;
            let lo = if i == 0 { 0.0 } else { pts.get(i - 1).map_or(0.0, |a| a.0 + step) };
            let hi = pts.get(i + 1).map_or(1.0, |a| a.0 - step);
            let (x, y) = unit(p);
            if let Some(q) = pts.get_mut(i) {
                *q = (x.clamp(lo, hi.max(lo)), y);
            }
        }
        changed = true;
    }
    if resp.drag_stopped() {
        grabbed = None;
    }
    ui.data_mut(|m| m.insert_temp(held, grabbed));
    // The graph: quarter grid, the identity line, the curve and its points.
    let painter = ui.painter_at(rect.expand(4.0));
    painter.rect_filled(rect, 2.0, t.input);
    let faint = egui::Stroke::new(1.0, t.input_border);
    for k in 1..4 {
        let f = k as f32 / 4.0;
        painter.line_segment([screen((f, 0.0)), screen((f, 1.0))], faint);
        painter.line_segment([screen((0.0, f)), screen((1.0, f))], faint);
    }
    painter.line_segment([screen((0.0, 0.0)), screen((1.0, 1.0))], faint);
    let curve: Vec<egui::Pos2> = (0..=64).map(|k| k as f32 / 64.0).map(|x| screen((x, vectorcraft_effects::curve_at(&pts, x)))).collect();
    painter.add(egui::Shape::line(curve, egui::Stroke::new(1.5, t.text)));
    for (i, q) in pts.iter().enumerate() {
        let fill = if Some(i) == grabbed { t.accent } else { t.panel };
        painter.circle(screen(*q), 4.0, fill, egui::Stroke::new(1.5, t.text));
    }
    painter.rect_stroke(rect, 2.0, egui::Stroke::new(1.0, t.input_border), egui::StrokeKind::Inside);
    if changed {
        let text: Vec<String> = pts.iter().map(|(x, y)| format!("{},{}", (x * 255.0).round(), (y * 255.0).round())).collect();
        d.fields.insert("points".into(), json!(text.join(" ")));
    }
}

/// What OK runs: `effect.setParams` on the edited effect (`__index`), else `effect.apply` of the
/// dialog's effect with its values on the target item.
fn command(d: &Dialog) -> (&'static str, Value) {
    let params = form::params(d);
    let item = d.fields.get("__item").cloned();
    if let Some(index) = d.fields.get("__index") {
        let mut p = json!({"index": index, "item": item.unwrap_or(Value::Null), "params": params});
        let id = d.str("__effect");
        // The displayed style comes from the first target. Preserve other targets' Warp IDs
        // unless the user deliberately chooses Style, including after a preview rollback.
        if super::warp_effect::is_warp(&id) && d.bool("__warp_style_changed") {
            p["warpStyle"] = json!(id.strip_prefix("warp."));
        }
        return ("effect.setParams", p);
    }
    let mut p = json!({"effect": d.str("__effect"), "params": params});
    if let Some(item) = item {
        p["item"] = item;
    }
    ("effect.apply", p)
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let (cmd, p) = command(d);
    // A live preview commits as one undo step. When the command refuses the values (a Color the
    // effect can't draw), the dialog stays open and the error is returned.
    let r = form::commit_preview(app, cmd, p.clone())?;
    // Last Effect repeats applied effects, not edits.
    if cmd == "effect.apply" {
        let id = d.str("__effect");
        app.last_effect = Some((id.clone(), p["params"].clone()));
        if super::warp_effect::is_warp(id.as_str()) {
            app.last_warp = Some(super::warp_effect::remembered_params(d));
        }
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_that_pick_a_value_are_dropdowns_of_its_values() {
        // The values come from the effect's parameter documentation (`joins: "miter"|"round"|…`).
        for (effect, key) in [
            ("path.offsetPath", "joins"),
            ("distort.zigZag", "points"),
            ("distort.roughen", "points"),
            ("stylize.innerGlow", "source"),
            (DROP_SHADOW, "shade"),
            ("artistic.paintDaubs", "brushType"),
        ] {
            let doc = vectorcraft_effects::effect_info(effect).unwrap().params;
            let listed = doc.split(&format!("{key}: ")).nth(1).unwrap().split([',', '}', ' ']).next().unwrap().to_string();
            let values: Vec<String> = choices(effect, key).unwrap().iter().map(|(_, v)| format!("\"{v}\"")).collect();
            assert_eq!(values.join("|"), listed, "{effect} {key}");
        }
    }

    /// The lists that dialogs draw in their own bodies hold the values the effect's parameter
    /// documentation lists, in its order; Rasterize's Color Model names the document's mode first.
    #[test]
    fn lists_drawn_in_a_dialogs_body_are_the_documented_values() {
        for (effect, key) in [
            (vectorcraft_effects::REVOLVE, "edge"),
            (vectorcraft_effects::EXTRUDE, "bevel"),
            ("warp.arc", "orientation"),
            ("warp.twist", "orientation"),
            (vectorcraft_effects::RASTERIZE, "background"),
            (vectorcraft_effects::RASTERIZE, "antiAlias"),
            ("adjust.levels", "channel"),
            ("adjust.curves", "channel"),
        ] {
            let doc = vectorcraft_effects::effect_info(effect).unwrap().params;
            let listed = doc.split(&format!("{key}: ")).nth(1).unwrap().split([',', '}', ' ']).next().unwrap().replace('"', "");
            let values: Vec<&str> = own_choices(effect, key, false).unwrap().iter().map(|(_, v)| *v).collect();
            assert_eq!(values.join("|"), listed, "{effect} {key}");
        }
        for (cmyk, mode) in [(false, "RGB"), (true, "CMYK")] {
            let models = own_choices(vectorcraft_effects::RASTERIZE, "colorModel", cmyk).unwrap();
            assert_eq!(models, [(mode, "document"), ("Grayscale", "grayscale"), ("Bitmap", "bitmap")]);
        }
        assert!(own_choices("distort.twist", "orientation", false).is_none() && own_choices("adjust.hueSaturation", "channel", false).is_none());
    }

    /// Glass's Load Texture… reads the picked image into the dialog, the Texture menu shows Loaded
    /// Texture and the texture's size instead of its stored text, and OK applies it.
    #[test]
    fn glass_load_texture_reads_the_picked_image_into_the_effect() {
        let dir = std::env::temp_dir().join(format!("vectorcraft-glass-dialog-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("texture.png");
        image::RgbaImage::from_fn(6, 4, |x, y| image::Rgba([(x * 40) as u8, (y * 60) as u8, 90, 255])).save(&path).unwrap();
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let picked = path.to_string_lossy().into_owned();
        app.services.pick_open = Some(Box::new(move |_: &crate::FilePick| Some(picked.clone())));
        app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
        app.run("select.all", json!({})).unwrap();
        app.run("effect.dialog", json!({"effect": GLASS})).unwrap();
        let paint = |app: &mut VectorcraftApp| crate::tests_labels::painted_text(app, |app, ui| super::super::show(app, ui.ctx()));
        let text = paint(&mut app);
        assert!(text.lines().any(|l| l == "Load Texture…") && text.lines().any(|l| l == "Frosted"), "{text}");
        assert!(!text.contains("Loaded Texture") && !text.contains(" px"), "{text}");
        let mut d = app.ui.dialog.take().unwrap();
        load_texture(&mut app, &mut d).unwrap();
        let stored = d.str(vectorcraft_effects::pixel::TEXTURE_IMAGE);
        assert_eq!((d.str("texture"), vectorcraft_effects::pixel::texture(&stored).map(|t| t.size())), ("loaded".to_string(), Some((6, 4))));
        app.ui.dialog = Some(d);
        let text = paint(&mut app);
        assert!(text.lines().any(|l| l == "Loaded Texture") && text.lines().any(|l| l == "6 × 4 px"), "{text}");
        assert!(!text.contains(&stored[..24]), "the stored text isn't shown");
        super::super::confirm(&mut app).unwrap();
        let st = app.session.doc().unwrap();
        let params = &st.doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects[0].params;
        assert_eq!((&params["texture"], &params["textureImage"]), (&json!("loaded"), &json!(stored)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Texturizer's Load Texture… matches Glass: pick, Loaded Texture in the menu, OK applies.
    #[test]
    fn texturizer_load_texture_reads_the_picked_image_into_the_effect() {
        let dir = std::env::temp_dir().join(format!("vectorcraft-texturizer-dialog-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("texture.png");
        image::RgbaImage::from_fn(6, 4, |x, y| image::Rgba([(x * 40) as u8, (y * 60) as u8, 90, 255])).save(&path).unwrap();
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let picked = path.to_string_lossy().into_owned();
        app.services.pick_open = Some(Box::new(move |_: &crate::FilePick| Some(picked.clone())));
        app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
        app.run("select.all", json!({})).unwrap();
        app.run("effect.dialog", json!({"effect": TEXTURIZER})).unwrap();
        let paint = |app: &mut VectorcraftApp| crate::tests_labels::painted_text(app, |app, ui| super::super::show(app, ui.ctx()));
        let text = paint(&mut app);
        assert!(text.lines().any(|l| l == "Load Texture…") && text.lines().any(|l| l == "Canvas"), "{text}");
        let mut d = app.ui.dialog.take().unwrap();
        load_texture(&mut app, &mut d).unwrap();
        let stored = d.str(vectorcraft_effects::pixel::TEXTURE_IMAGE);
        assert_eq!((d.str("texture"), vectorcraft_effects::pixel::texture(&stored).map(|t| t.size())), ("loaded".to_string(), Some((6, 4))));
        app.ui.dialog = Some(d);
        super::super::confirm(&mut app).unwrap();
        let st = app.session.doc().unwrap();
        let params = &st.doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects[0].params;
        assert_eq!((&params["texture"], &params["textureImage"]), (&json!("loaded"), &json!(stored)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
