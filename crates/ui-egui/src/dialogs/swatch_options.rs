//! Swatch Options: name, colour type (process or spot), Global, colour mode with sliders and hex,
//! and the out-of-gamut and out-of-web-color warnings under the colour, previewed live on the canvas
//! (Cancel rolls the preview back). A click on a warning's icon or colour corrects the colour
//! (https://web.archive.org/web/20210601162814/https://helpx.adobe.com/pdf/illustrator_reference.pdf,
//! Swatch options; https://helpx.adobe.com/illustrator/using/adjusting-colors.html).
//! Double-clicking a swatch opens it for a solid colour; for a gradient it shows the name and the
//! gradient; patterns open pattern editing ([`open`]).
//!
//! Fields: `name`, `type` (`process` or `spot`), `global`, `mode` (`gray`, `rgb`, `hsb`, `lab`,
//! `cmyk` or `web`), `color` (`"#rrggbb"` or a colour object such as
//! `{"model": "cmyk", "c": 0.1, "m": 0.2, "y": 0.3, "k": 0}`) and `hex` (a hex colour that replaces
//! `color` in the next pass); a gradient swatch has `name` only. The Preview box is the dialog's.

use std::sync::Arc;

use craft::cms::convert::CmykConverter;
use craft::color_ui::gradient::{ColorStop, GradientStops, OpacityStop};
use craft::color_ui::ui::{ChannelRow, ChannelRule, ChannelUnit, ColorPanelMode, ColorWarning, ColorWarningChip, Gamut};
use craft::color_ui::{CmykModel, GrayModel, not_web_safe, web_safe};
use craft::draw::ui::GradientSwatch;
use craft::theme::{ColorModel, HsbModel, LabModel, ModelValues, RgbModel, Rgba, Srgba, kept_or_converted};
use craft::ui::choice::Swatch;
use craft::ui::contract::{Bind, Show as _, ShowContainer as _};
use craft::ui::dialog::DialogId;
use craft::ui::form::Field;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::patterns::form_dialogs::{CustomContext, DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use craft::ui::text::HexField;
use serde_json::{Value, json};
use vectorcraft_color::cms::Model;
use vectorcraft_color::{Color, GradientPaint, Paint};

use super::host::{HeldOptions, open_held_with};
use crate::VectorcraftApp;
use crate::panels::color::{color_of_values, gamut, model_values};

/// The dialog kind of Swatch Options.
pub const KIND: &str = "swatchOptions";

/// What a dialog says when the window has no dialog host to open it on.
pub(super) const NO_WINDOW: &str = "no window";
const PREVIEW_LABEL: &str = "Swatch Options";

/// The open Swatch Options dialog: the held dialog, the swatch it edits and the parameters its preview last ran with.
#[derive(Clone, Debug)]
pub(crate) struct Open {
    /// The held dialog.
    pub(crate) id: DialogId,
    opening: Arc<()>,
    document: u64,
    preview_reset: u64,
    /// The name of the swatch being edited.
    swatch: String,
    /// The `swatch.edit` parameters of the running preview; `None` while no preview runs.
    last: Option<Value>,
}

/// Colour modes, in menu order, with their `mode` field / `swatch.edit` ids and labels.
const MODES: [(ColorPanelMode, &str, &str); 6] = [
    (ColorPanelMode::GrayscaleSlider, "gray", "Grayscale"),
    (ColorPanelMode::RgbSliders, "rgb", "RGB"),
    (ColorPanelMode::HsbSliders, "hsb", "HSB"),
    (ColorPanelMode::LabSliders, "lab", "Lab"),
    (ColorPanelMode::CmykSliders, "cmyk", "CMYK"),
    (ColorPanelMode::WebColorSliders, "web", "Web Safe RGB"),
];

/// The web levels are 51 apart, so Web Safe RGB sliders step by that.
const WEB_STEP: f64 = 51.0;

pub(super) fn mode_id(m: ColorPanelMode) -> &'static str {
    MODES.iter().find(|x| x.0 == m).map_or("rgb", |x| x.1)
}

fn mode_of(fields: &DialogFields) -> ColorPanelMode {
    let id = fields.text("mode");
    MODES.iter().find(|x| Some(x.1) == id).map_or(ColorPanelMode::RgbSliders, |x| x.0)
}

/// The mode a colour was authored in.
pub(super) fn mode_of_color(c: &Color) -> ColorPanelMode {
    match c.model() {
        Model::Rgb => ColorPanelMode::RgbSliders,
        Model::Cmyk => ColorPanelMode::CmykSliders,
        Model::Gray => ColorPanelMode::GrayscaleSlider,
        Model::Lab => ColorPanelMode::LabSliders,
    }
}

/// The colour model the colour has to be in to be edited in `mode` (HSB and Web Safe RGB are views of RGB).
fn model_kind(mode: ColorPanelMode) -> Model {
    match mode {
        ColorPanelMode::GrayscaleSlider => Model::Gray,
        ColorPanelMode::LabSliders => Model::Lab,
        ColorPanelMode::CmykSliders => Model::Cmyk,
        _ => Model::Rgb,
    }
}

/// The channels and tracks the sliders of `mode` show.
fn channel_model(mode: ColorPanelMode) -> Box<dyn ColorModel> {
    match mode {
        ColorPanelMode::GrayscaleSlider => Box::new(GrayModel),
        ColorPanelMode::HsbSliders => Box::new(HsbModel),
        ColorPanelMode::LabSliders => Box::new(LabModel),
        // PLAN: the CMYK tracks run through the built-in coated converter; color-w9 gives no model over the app's CMS.
        ColorPanelMode::CmykSliders => Box::new(CmykModel::new(CmykConverter::coated().clone())),
        _ => Box::new(RgbModel),
    }
}

/// The dialog's colour: a hex string or a serialized colour.
pub(super) fn color_of(fields: &DialogFields) -> Color {
    match fields.get("color") {
        Some(FieldValue::Text(s) | FieldValue::Json(Value::String(s))) => Color::from_hex(s),
        Some(FieldValue::Json(v)) => serde_json::from_value(v.clone()).ok(),
        _ => None,
    }
    .unwrap_or(Color::BLACK)
}

pub(super) fn set_color(fields: &mut DialogFields, c: Color) {
    fields.set("color", FieldValue::Json(json!(c)));
}

fn is_spot(fields: &DialogFields) -> bool {
    fields.text("type") == Some("spot")
}

fn srgba(c: &Color) -> Srgba {
    let [r, g, b] = c.to_rgb();
    Srgba::rgb(r, g, b)
}

fn to8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// `c` on the nearest web-safe colour.
fn web_color(c: &Color) -> Color {
    let [r, g, b] = web_safe(c.to_rgb());
    Color::rgb(r, g, b)
}

/// The colour that `values`, edited in `mode`, give.
fn colour_from(mode: ColorPanelMode, values: ModelValues) -> Color {
    let c = if values.model == HsbModel.id() {
        let s = HsbModel.to_srgb(&values.values);
        Color::rgb(s.r, s.g, s.b)
    } else {
        color_of_values(&values)
    };
    if mode == ColorPanelMode::WebColorSliders { web_color(&c) } else { c }
}

/// The HSB values kept from the last edit, which keep the hue of a grey and the hue and saturation of black.
fn kept_hsb(fields: &DialogFields) -> Option<ModelValues> {
    let v = match fields.get("hsb")? {
        FieldValue::Json(v) => serde_json::from_value::<[f32; 3]>(v.clone()).ok()?,
        FieldValue::Color(c) => *c,
        _ => return None,
    };
    Some(ModelValues::new(HsbModel.id(), &v))
}

/// The values the sliders of `mode` show for `color`.
fn shown_values(mode: ColorPanelMode, color: &Color, fields: &DialogFields) -> ModelValues {
    if mode == ColorPanelMode::HsbSliders { kept_or_converted(&HsbModel, srgba(color), kept_hsb(fields).as_ref()) } else { model_values(color) }
}

/// The label of a row, without the colon the form adds.
pub(super) fn field_label(text: &str) -> String {
    text.trim_end_matches(':').to_owned()
}

/// The Swatch Name row.
pub(super) fn name_row() -> Param {
    Param::new("name", field_label(tl!("Swatch Name:")), ParamKind::text(""))
}

/// A hosted dialog for the command `key`, whose rows are `rows`: the one way the colour dialogs build theirs.
pub(super) fn hosted(
    ctx: &egui::Context,
    key: &'static str,
    title: &str,
    fields: DialogFields,
    rows: Vec<Param>,
    previews: bool,
) -> HostedParamDialog {
    let title = title.to_owned();
    HostedParamDialog::new(ctx, key, fields, move |values| {
        ParamDialog::new(key, title.clone(), rows.clone()).values(Bind::bound(values)).previews(previews)
    })
}

/// Runs `f` on the hosted dialog `id`; `None` when the dialog or the window is gone.
pub(super) fn with_dialog<R>(app: &VectorcraftApp, id: DialogId, f: impl FnOnce(&mut HostedParamDialog) -> R) -> Option<R> {
    app.dialogs()?.with(|host| host.get_mut_by_id::<HostedParamDialog>(id).map(f))
}

/// The rows of Swatch Options and New Swatch for a colour: name, type, Global, mode and the colour with its
/// warnings. `gamut` is the printable test the warnings use; it holds while the dialog is open, since the modal
/// stack keeps the colour settings and the document from changing.
pub(super) fn colour_rows(gamut: Gamut<'static>) -> Vec<Param> {
    let gamut = Arc::new(gamut);
    let types = vec![("process".to_owned(), tl!("Process Color").to_owned().into()), ("spot".to_owned(), tl!("Spot Color").to_owned().into())];
    let modes = MODES.iter().map(|(_, id, label)| ((*id).to_owned(), Label::from(tl!(label).to_owned()))).collect();
    vec![
        name_row(),
        Param::new("type", field_label(tl!("Color Type:")), ParamKind::choice(types, Some("process".to_owned()))),
        // Spot colours are always global.
        Param::new("global", tl!("Global").to_owned(), ParamKind::bool(false)).enabled_when("type", "process").disabled_value(true),
        Param::new("mode", field_label(tl!("Color Mode:")), ParamKind::choice(modes, Some("rgb".to_owned()))),
        Param::custom("color", tl!("Color").to_owned(), move |ui, fields, context| edit_colour(ui, fields, &gamut, context))
            .companions(["hex", "hsb"]),
    ]
}

/// The colour fields of a swatch: its type, Global, mode and colour.
pub(super) fn colour_fields(fields: &mut DialogFields, color: Color, spot: bool, global: bool) {
    fields.set("type", if spot { "spot" } else { "process" });
    fields.set("global", spot || global);
    fields.set("mode", mode_id(mode_of_color(&color)));
    set_color(fields, color);
}

/// `swatch.edit` and `swatch.new` parameters of the dialog's colour: the colour, its mode, spot and global.
pub(super) fn colour_params(fields: &DialogFields) -> Value {
    let spot = is_spot(fields);
    json!({
        "color": color_of(fields),
        "mode": mode_id(mode_of(fields)),
        "global": spot || fields.bool("global").unwrap_or(false),
        "spot": spot,
    })
}

/// Open the editor of swatch `name`: Swatch Options for a colour (a tint swatch's base) or a
/// gradient (its name only), pattern editing for a pattern.
pub fn open(app: &mut VectorcraftApp, name: &str) -> Result<Value, String> {
    let d = app.session.active().map(|st| &st.doc);
    let sw = d.and_then(|d| d.swatch(name)).ok_or_else(|| format!("no swatch `{name}`"))?;
    if let Some((base, _)) = sw.tint_of()
        && d.is_some_and(|d| d.swatch(base).is_some())
    {
        let base = base.to_string();
        return open(app, &base);
    }
    let sw = sw.clone();
    let mut fields = DialogFields::new();
    fields.set("name", name);
    match &sw.paint {
        Paint::Solid { color, .. } => {
            colour_fields(&mut fields, *color, sw.spot, sw.global);
            launch(app, name, fields, colour_rows(gamut(app)))
        }
        Paint::Gradient(g) => {
            let stops = Arc::new(stops_of(g));
            let chip = Param::custom("gradient", tl!("Gradient").to_owned(), move |ui, _, _| {
                ui.scope(|ui| {
                    GradientSwatch::new(tl!("Gradient").to_owned(), &stops).show(ui);
                })
                .response
            });
            launch(app, name, fields, vec![name_row(), chip])
        }
        Paint::Pattern { pattern, .. } => {
            app.run("object.pattern.edit", json!({"name": pattern}))?;
            app.ui.open_panel = Some("patternOptions".into());
            Ok(Value::Null)
        }
        Paint::None => Err(format!("`{name}` has no options")),
    }
}

/// The gradient as the chip draws it: its colour stops and their opacities.
fn stops_of(g: &GradientPaint) -> GradientStops {
    let (colors, opacities) = g
        .gradient
        .stops
        .iter()
        .map(|s| {
            let mut stop = ColorStop::new(s.offset, srgba(&s.color));
            stop.midpoint = s.midpoint;
            (stop, OpacityStop::new(s.offset, s.opacity))
        })
        .unzip();
    GradientStops::with_opacity(colors, opacities)
}

/// Opens the held dialog over swatch `swatch`: OK is one `swatch.edit` undo step, Cancel rolls the preview back.
fn launch(app: &mut VectorcraftApp, swatch: &str, fields: DialogFields, rows: Vec<Param>) -> Result<Value, String> {
    let document = app.session.active().ok_or("no drawing")?.uid;
    let ctx = app.ui_ctx.clone().ok_or(NO_WINDOW)?;
    let dialog = hosted(&ctx, KIND, tl!("Swatch Options"), fields, rows, true);
    if owns_preview(app) {
        let _ = app.session.cancel_interaction();
    }
    let opening = Arc::new(());
    let cancelled = Arc::clone(&opening);
    let ok = |app: &mut VectorcraftApp, answer: &ParamResult| -> Result<(), String> {
        let open = app.ui.swatch_options.as_ref().ok_or("the Swatch Options dialog is gone")?;
        if app.session.active().is_none_or(|st| st.uid != open.document) {
            return Err(tl!("Return to the original drawing to edit this swatch").into());
        }
        let edited = open.swatch.clone();
        // The edit replaces the preview, so the preview is dropped first.
        if owns_preview(app) {
            let _ = app.session.cancel_interaction();
        } else if app.session.in_interaction() {
            app.keep_typing()?;
            if app.session.in_interaction() {
                return Err(tl!("Finish the current edit before changing this swatch").into());
            }
        }
        app.run("swatch.edit", edit_params(&edited, &answer.fields))?;
        app.ui.swatch_options = None;
        Ok(())
    };
    let cancel = move |app: &mut VectorcraftApp| {
        if app.ui.swatch_options.as_ref().is_some_and(|open| Arc::ptr_eq(&open.opening, &cancelled)) {
            if owns_preview(app) {
                let _ = app.session.cancel_interaction();
            }
            app.ui.swatch_options = None;
        }
    };
    let id = open_held_with(app, dialog, ok, HeldOptions { parent: None, cancel: Some(Box::new(cancel)) }).ok_or(NO_WINDOW)?;
    app.ui.swatch_options = Some(Open { id, opening, document, preview_reset: app.session.tool_resets(), swatch: swatch.to_owned(), last: None });
    Ok(Value::Null)
}

/// `swatch.edit` parameters from the fields (a gradient swatch only renames).
fn edit_params(swatch: &str, fields: &DialogFields) -> Value {
    let mut p = if fields.contains("color") { colour_params(fields) } else { json!({}) };
    p["name"] = json!(swatch);
    p["newName"] = json!(fields.text("name").unwrap_or_default());
    p
}

/// Shows the edit on the canvas while the Preview box is on and drops it when it goes off. It runs each pass; the
/// parameters it last previewed with are remembered, so a pass that changes nothing runs nothing.
fn owns_preview(app: &VectorcraftApp) -> bool {
    app.ui.swatch_options.as_ref().is_some_and(|open| {
        open.last.is_some()
            && open.preview_reset == app.session.tool_resets()
            && app
                .session
                .active()
                .is_some_and(|st| st.uid == open.document && st.interaction.as_ref().is_some_and(|interaction| interaction.label == PREVIEW_LABEL))
    })
}

pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(open) = app.ui.swatch_options.as_ref().filter(|open| app.session.active().is_some_and(|st| st.uid == open.document)) else { return };
    let (id, swatch, last) = (open.id, open.swatch.clone(), open.last.clone());
    let owns_preview = owns_preview(app);
    correct(app, id);
    let Some((previewing, fields)) = with_dialog(app, id, |d| (d.is_previewing(), d.fields().clone())) else { return };
    let want = previewing.then(|| edit_params(&swatch, &fields));
    if let Some(params) = &want {
        // A refused OK drops the interaction under an unchanged dialog: the preview starts again.
        if want == last && owns_preview {
            return;
        }
        if !owns_preview && app.session.in_interaction() {
            if let Err(e) = app.keep_typing() {
                app.status(e);
                return;
            }
            if app.session.in_interaction() {
                return;
            }
        }
        let begun = owns_preview || app.begin_interaction(PREVIEW_LABEL).map_err(|e| app.status(e)).is_ok();
        if begun && let Err(e) = app.session.preview("swatch.edit", params) {
            app.status(e.to_string());
        }
    } else if owns_preview {
        // The Preview box went off: the document shows its own colour again. Nothing to report if none was open.
        let _ = app.session.cancel_interaction();
    }
    if let Some(open) = app.ui.swatch_options.as_mut() {
        open.last = want;
        open.preview_reset = app.session.tool_resets();
    }
}

/// A warning under the colour of the swatch dialogs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Warning {
    Gamut,
    Web,
}

impl Warning {
    /// The action the colour row asks the host for when the warning is clicked.
    const fn action(self) -> &'static str {
        match self {
            Warning::Gamut => "correctGamut",
            Warning::Web => "correctWeb",
        }
    }

    fn of(action: &str) -> Option<Warning> {
        [Warning::Gamut, Warning::Web].into_iter().find(|w| w.action() == action)
    }
}

/// The warnings the swatch dialogs show in `mode`: out of gamut in RGB, HSB and Lab, and out of web
/// colors in every mode but Web Safe RGB.
pub(super) fn shown_warnings(mode: ColorPanelMode) -> (bool, bool) {
    (matches!(mode, ColorPanelMode::RgbSliders | ColorPanelMode::HsbSliders | ColorPanelMode::LabSliders), mode != ColorPanelMode::WebColorSliders)
}

/// The colour and mode a click on `warning` gives `c`, edited in `mode`: the nearest web-safe colour
/// (in RGB unless the mode is RGB or HSB), or the colour's CMYK equivalent in the active CMYK space
/// (in CMYK). `None` when the conversion returns no CMYK values.
pub(super) fn corrected(
    app: &mut VectorcraftApp,
    warning: Warning,
    mode: ColorPanelMode,
    c: &Color,
) -> Result<Option<(Color, ColorPanelMode)>, String> {
    match warning {
        Warning::Web => {
            let kept = if matches!(mode, ColorPanelMode::RgbSliders | ColorPanelMode::HsbSliders) { mode } else { ColorPanelMode::RgbSliders };
            Ok(Some((web_color(c), kept)))
        }
        Warning::Gamut => {
            let r = app.run("color.convert", json!({"color": crate::panels::color_json(c), "to": "cmyk"}))?;
            let v = |i: usize| r["values"][i].as_f64().map(|v| v as f32);
            Ok(v(0).zip(v(1)).zip(v(2)).zip(v(3)).map(|(((c, m), y), k)| (Color::cmyk(c, m, y, k), ColorPanelMode::CmykSliders)))
        }
    }
}

/// Applies the corrections the colour row asked for since the last pass: the colour and the mode it is edited in
/// change in the dialog's fields only, and OK is what makes them an edit.
pub(super) fn correct(app: &mut VectorcraftApp, id: DialogId) {
    let Some((actions, fields)) = with_dialog(app, id, |d| (d.take_actions(), d.fields().clone())) else { return };
    for warning in actions.iter().filter_map(|a| Warning::of(a)) {
        match corrected(app, warning, mode_of(&fields), &color_of(&fields)) {
            Ok(Some((c, mode))) => {
                with_dialog(app, id, |d| {
                    d.set_field("mode", FieldValue::Text(mode_id(mode).to_owned()));
                    d.set_field("color", FieldValue::Json(json!(c)));
                });
            }
            Ok(None) => {}
            Err(e) => app.status(e),
        }
    }
}

/// The colour row: one slider and value field per component of the dialog's mode, the hex field (RGB, HSB and
/// Web), a chip of the colour and under it the mode's warnings. The colour follows the mode: a colour in another
/// model is converted when the mode changes.
fn edit_colour(ui: &mut egui::Ui, fields: &mut DialogFields, gamut: &Gamut<'static>, context: &mut CustomContext) -> egui::Response {
    // A hex colour set from outside the row (automation) replaces the colour once.
    if let Some(hex) = fields.text("hex").map(str::to_owned) {
        fields.remove("hex");
        if let Some(c) = Color::from_hex(&hex) {
            set_color(fields, c);
        }
    }
    let mode = mode_of(fields);
    let mut color = color_of(fields);
    if color.model() != model_kind(mode) {
        color = color.in_model(model_kind(mode));
        set_color(fields, color);
    }
    let model = channel_model(mode);
    let values = shown_values(mode, &color, fields);
    let web = mode == ColorPanelMode::WebColorSliders;
    let (shows_gamut, shows_web) = shown_warnings(mode);
    let near = srgba(&color);
    let mut warnings = Vec::new();
    if shows_gamut && let Some(nearest) = gamut.nearest(near) {
        warnings.push((Warning::Gamut, ColorWarning::new(tl!("Out of Gamut Warning").to_owned(), nearest)));
    }
    if shows_web && let Some([r, g, b]) = not_web_safe(color.to_rgb()) {
        warnings.push((Warning::Web, ColorWarning::web_colors(tl!("Out of Web Color Warning").to_owned(), Srgba::rgb(r, g, b))));
    }
    let mut edit: Option<(Color, Option<ModelValues>)> = None;
    let mut actions = Vec::new();
    let shown = Row::new().show(ui, |ui, _| {
        Stack::new().show(ui, |ui, _| {
            for (i, ch) in model.channels().iter().enumerate() {
                let mut rule = ChannelRule::of(ch);
                if web {
                    rule.step = WEB_STEP;
                }
                // The hue track is the whole spectrum, whatever the saturation and brightness.
                let mut track_at = values;
                if model.id() == HsbModel.id() && i == 0 {
                    track_at.values[1] = 100.0;
                    track_at.values[2] = 100.0;
                }
                let shown = values.values.get(i).copied().unwrap_or_default();
                let row = ChannelRow::new(Label::from(ch.name), Bind::given(f64::from(shown)), rule)
                    .short(ch.label)
                    .unit(ChannelUnit::of(ch.unit))
                    .track(model.track(i, &track_at))
                    .show(ui);
                if let Some(v) = row.as_ref().and_then(|r| r.value()).copied().filter(|v| v.is_finite()) {
                    let mut next = values;
                    if let Some(slot) = next.values.get_mut(i) {
                        *slot = v as f32;
                    }
                    edit = Some((colour_from(mode, next), Some(next)));
                }
            }
            if matches!(mode, ColorPanelMode::RgbSliders | ColorPanelMode::HsbSliders | ColorPanelMode::WebColorSliders) {
                let [r, g, b] = color.to_rgb();
                let hex = Rgba([to8(r), to8(g), to8(b), 255]);
                let name = tl!("Hex Color").to_owned();
                let field = Field::new(name.clone()).side(HexField::hash_label());
                let typed = field.show(ui, |ui, _| HexField::hex(name.clone(), Bind::given(hex)).show(ui)).inner;
                if let Some(Rgba([r8, g8, b8, _])) = typed.and_then(|out| out.events.value().copied()) {
                    let c = Color::rgb8(r8, g8, b8);
                    edit = Some((if web { web_color(&c) } else { c }, None));
                }
            }
        });
        Stack::new().show(ui, |ui, _| {
            Swatch::new(tl!("Color").to_owned(), near).show(ui);
            actions.clear();
            for (warning, chip) in &warnings {
                if ColorWarningChip::new(chip).show(ui).clicked {
                    actions.push(warning.action());
                }
            }
        });
    });
    for action in actions {
        context.request_action(action);
    }
    if let Some((c, kept)) = edit {
        set_color(fields, c);
        if let Some(k) = kept.filter(|k| k.model == HsbModel.id()) {
            fields.set("hsb", FieldValue::Json(json!([k.values[0], k.values[1], k.values[2]])));
        }
    }
    shown.output
}

#[cfg(test)]
mod tests {
    use craft::ui::dialog::dialog_id;
    use vectorcraft_engine::Session;

    use super::*;

    /// An app with a window for its dialogs and one open document.
    fn setup() -> (egui::Context, VectorcraftApp) {
        let ctx = crate::testing::ui_context();
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.ui_ctx = Some(ctx.clone());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        (ctx, app)
    }

    /// Writes `fields` into the open dialog as automation does.
    fn set(app: &VectorcraftApp, ctx: &egui::Context, fields: &[(&str, Value)]) {
        let host = app.dialogs().unwrap();
        for (field, value) in fields {
            host.with(|h| h.set_json_field(ctx, dialog_id(KIND), field, value.clone())).unwrap();
        }
    }

    /// The colour, spot and global flags of swatch `name`.
    fn swatch(app: &VectorcraftApp, name: &str) -> (Option<Color>, bool, bool) {
        let sw = app.session.active().unwrap().doc.swatch(name).unwrap();
        (sw.paint.color(), sw.spot, sw.global)
    }

    #[test]
    fn swatch_options_refuses_a_matching_name_in_another_drawing() {
        let (ctx, mut app) = setup();
        app.run("swatch.new", json!({"name": "Twin", "color": "#123456"})).unwrap();
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.run("swatch.new", json!({"name": "Twin", "color": "#6789ab"})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 30})).unwrap();
        let activate = |app: &mut VectorcraftApp, index| app.run("document.activate", json!({"index": index})).unwrap();
        let other = swatch(&app, "Twin");
        activate(&mut app, 0);
        open(&mut app, "Twin").unwrap();
        set(&app, &ctx, &[("name", json!("Source paint")), ("preview", json!(false))]);
        activate(&mut app, 1);
        app.session.begin_interaction("Other drawing's edit").unwrap();
        app.session.preview("paint.setFill", &json!({"color": "#00aa00"})).unwrap();
        let dialogs = app.dialogs().unwrap();
        let id = dialog_id(KIND);
        dialogs.with(|h| h.confirm_by_id(&ctx, id));
        crate::dialogs::host::pump(&mut app, &ctx);
        assert!(app.session.in_interaction(), "the other drawing's edit stays active");
        assert_eq!(swatch(&app, "Twin"), other);

        app.session.cancel_interaction().unwrap();
        activate(&mut app, 0);
        dialogs.with(|h| h.confirm_by_id(&ctx, id));
        crate::dialogs::host::pump(&mut app, &ctx);
        assert!(app.session.active().unwrap().doc.swatch("Source paint").is_some());
    }

    #[test]
    fn swatch_options_waits_for_another_preview_in_the_original_drawing() {
        let (ctx, mut app) = setup();
        app.run("swatch.new", json!({"name": "Editable", "color": "#123456"})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 30})).unwrap();
        open(&mut app, "Editable").unwrap();
        set(&app, &ctx, &[("name", json!("Renamed")), ("preview", json!(false))]);
        app.session.begin_interaction("An independent preview").unwrap();
        app.session.preview("paint.setFill", &json!({"color": "#00aa00"})).unwrap();
        let dialogs = app.dialogs().unwrap();
        let id = dialog_id(KIND);
        dialogs.with(|host| host.confirm_by_id(&ctx, id));
        crate::dialogs::host::pump(&mut app, &ctx);
        assert!(app.session.in_interaction());
        assert!(app.session.active().unwrap().doc.swatch("Renamed").is_none());

        app.session.cancel_interaction().unwrap();
        let undo = app.session.active().unwrap().history.undo.len();
        dialogs.with(|host| host.confirm_by_id(&ctx, id));
        crate::dialogs::host::pump(&mut app, &ctx);
        assert!(app.session.active().unwrap().doc.swatch("Renamed").is_some());
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
    }

    #[test]
    fn reopening_swatch_options_rolls_back_the_replaced_preview() {
        let (ctx, mut app) = setup();
        for name in ["First color", "Second color"] {
            app.run("swatch.new", json!({"name": name, "color": "#123456"})).unwrap();
        }
        open(&mut app, "First color").unwrap();
        set(&app, &ctx, &[("name", json!("Temporary name")), ("preview", json!(true))]);
        crate::dialogs::host::pump(&mut app, &ctx);
        assert!(app.session.active().unwrap().doc.swatch("Temporary name").is_some());
        open(&mut app, "Second color").unwrap();
        assert!(app.session.active().unwrap().doc.swatch("First color").is_some());
        assert!(app.session.active().unwrap().doc.swatch("Temporary name").is_none());

        set(&app, &ctx, &[("name", json!("New name")), ("preview", json!(false))]);
        let dialogs = app.dialogs().unwrap();
        dialogs.with(|host| host.confirm_by_id(&ctx, dialog_id(KIND)));
        crate::dialogs::host::pump(&mut app, &ctx);
        assert!(app.session.active().unwrap().doc.swatch("New name").is_some());
        assert!(app.session.active().unwrap().doc.swatch("First color").is_some());
    }

    #[test]
    fn swatch_options_shows_and_corrects_both_warnings() {
        assert_eq!(shown_warnings(ColorPanelMode::RgbSliders), (true, true), "RGB shows both");
        assert_eq!(shown_warnings(ColorPanelMode::CmykSliders), (false, true), "CMYK is never out of gamut");
        assert_eq!(shown_warnings(ColorPanelMode::WebColorSliders), (false, false), "Web Safe RGB shows none");
        let (ctx, mut app) = setup();
        app.run("swatch.new", json!({"name": "Sky", "color": "#0000fe", "spot": true})).unwrap();
        app.run("ui.swatchOptions", json!({"name": "Sky"})).unwrap();
        let blue = Color::from_hex("#0000fe").unwrap();
        // The cube's correction shifts to the nearest web-safe colour, in the same mode.
        let (web, mode) = corrected(&mut app, Warning::Web, ColorPanelMode::RgbSliders, &blue).unwrap().unwrap();
        assert_eq!((web.to_hex(), mode), ("#0000ff".to_string(), ColorPanelMode::RgbSliders));
        // The triangle's correction shifts to the CMYK equivalent, edited in CMYK.
        let (ink, mode) = corrected(&mut app, Warning::Gamut, mode, &web).unwrap().unwrap();
        assert_eq!(mode, ColorPanelMode::CmykSliders);
        set(&app, &ctx, &[("color", json!(ink)), ("mode", json!(mode_id(mode)))]);
        let id = dialog_id(KIND);
        app.dialogs().unwrap().with(|h| h.confirm_by_id(&ctx, id));
        crate::dialogs::host::pump(&mut app, &ctx);
        assert_eq!(swatch(&app, "Sky"), (Some(ink), true, true), "still a spot colour");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(swatch(&app, "Sky").0.map(|c| c.to_hex()), Some("#0000fe".into()), "one undo step");
    }
}
