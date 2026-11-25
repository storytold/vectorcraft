//! Color panel: the framework's fill and stroke layout over the document's paints. The rows of the
//! shown mode (Grayscale, RGB, HSB, CMYK, Web Safe RGB, or Lab for a Lab colour) edit the active
//! proxy's solid colour, the Gradient panel's selected stop (on a freeform gradient, its selected
//! point) or the picked mesh points. A colour (or stop) linked to a global or spot swatch shows the
//! Tint row instead; picking a mode from the menu makes it a process colour.
//!
//! The mode follows the colour's own model: the last mode picked comes back for colours in its model
//! and the defaults for new art, and the app keeps it between sessions. Alt on the spectrum, a chip
//! or a recent colour targets the inactive proxy (the framework's `PanelTarget::Background`);
//! Cmd/Ctrl-clicking the spectrum recolours (Complement, with Shift Invert).
//!
//! `ui.colorPanel` picks a mode, shows or hides the options and checks or unchecks Show Recent
//! Colors as the panel menu does, and switches RGB to percent and back. `ui.colorRgbPercent`
//! switches RGB between percent and 0 to 255 from the command palette or a keyboard shortcut.

use craft::behavior::Mixed;
use craft::color_ui::ui::{
    ColorPanel, ColorPanelAction, ColorPanelCommand, ColorPanelLayout, ColorPanelMenu, ColorPanelMode, ColorPanelOutput, ColorPanelShown,
    ColorPanelState, Gamut, PairWell, PanelTarget, PickedColor, TintRow, Well,
};
use craft::color_ui::web_safe;
use craft::keymap::CommandRef;
use craft::theme::{Channel, ColorModel, DefaultOn, MAX_CHANNELS, ModelValues, RgbModel, Srgba};
use craft::ui::contract::{Bind, CommandSet};
use craft::ui::menu::PanelRow;
use craft::ui::runtime::commands::CommandStates;
use egui::Ui;
use serde_json::{Map, Value, json};
use vectorcraft_color::cms;
use vectorcraft_color::{Color, GradientKind, Paint};

use super::{Live, active_paint, color_json, live_run, proxy_events};
use crate::VectorcraftApp;
use crate::edits::{EditPath, apply_event};
use crate::paint_preview::well_paint;

/// The layout the app's panel and its menu use.
const LAYOUT: ColorPanelLayout = ColorPanelLayout::FillStroke;

/// The control protocol's id of each menu mode; the only place that knows them.
const MODE_IDS: [(&str, ColorPanelMode); 5] = [
    ("grayscale", ColorPanelMode::GrayscaleSlider),
    ("rgb", ColorPanelMode::RgbSliders),
    ("hsb", ColorPanelMode::HsbSliders),
    ("cmyk", ColorPanelMode::CmykSliders),
    ("websafe", ColorPanelMode::WebColorSliders),
];

/// The menu mode whose protocol id is `id`.
fn mode_of_id(id: &str) -> Option<ColorPanelMode> {
    MODE_IDS.iter().find(|(i, _)| *i == id).map(|(_, m)| *m)
}

/// The protocol id of the mode the panel shows: a menu mode's, or `lab` for a Lab colour.
fn shown_id(mode: ColorPanelMode) -> &'static str {
    match MODE_IDS.iter().find(|(_, m)| *m == mode) {
        Some((id, _)) => id,
        None if mode == ColorPanelMode::LabSliders => "lab",
        None => "rgb",
    }
}

/// `mode` when it is one the menu picks: the stored default (a cube) means none was picked yet.
fn picked_mode(mode: ColorPanelMode) -> Option<ColorPanelMode> {
    ColorPanelMode::FILL_STROKE.contains(&mode).then_some(mode)
}

/// The colour model a mode edits (HSB and Web Safe RGB edit RGB colours).
fn mode_model(mode: ColorPanelMode) -> cms::Model {
    match mode {
        ColorPanelMode::GrayscaleSlider => cms::Model::Gray,
        ColorPanelMode::CmykSliders => cms::Model::Cmyk,
        ColorPanelMode::LabSliders => cms::Model::Lab,
        _ => cms::Model::Rgb,
    }
}

/// The mode a colour was authored in.
fn own_mode(c: &Color) -> ColorPanelMode {
    match c {
        Color::Rgb { .. } => ColorPanelMode::RgbSliders,
        Color::Cmyk { .. } => ColorPanelMode::CmykSliders,
        Color::Gray { .. } => ColorPanelMode::GrayscaleSlider,
        Color::Lab { .. } => ColorPanelMode::LabSliders,
    }
}

/// Does `mode` show `c` in its own model?
fn holds(mode: ColorPanelMode, c: &Color) -> bool {
    mode_model(mode) == c.model()
}

/// The color's numeric components in the active working RGB space.
pub(crate) fn working_rgb(c: &Color) -> [f32; 3] {
    let cms = cms::active();
    cms.convert(c, cms::Model::Rgb, cms.settings().intent).to_rgb_uncalibrated()
}

/// `c` as the framework's encoded sRGB.
fn srgba(c: &Color) -> Srgba {
    let [r, g, b] = c.to_rgb();
    Srgba::rgb(r, g, b)
}

/// RGB in the active working space, 0 to 255: the panel's RGB rows show and edit the stored components, so a wide
/// working space shows its own numbers rather than sRGB's.
pub(crate) struct WorkingRgb;

impl ColorModel for WorkingRgb {
    fn id(&self) -> &'static str {
        RgbModel.id()
    }
    fn name(&self) -> &'static str {
        RgbModel.name()
    }
    fn channels(&self) -> &[Channel] {
        static RGB: RgbModel = RgbModel;
        RGB.channels()
    }
    fn from_srgb(&self, c: Srgba) -> ModelValues {
        let [r, g, b] = cms::active().srgb_to_rgb([c.r, c.g, c.b]);
        model_values(&Color::rgb(r, g, b))
    }
    fn to_srgb(&self, v: &[f32; MAX_CHANNELS]) -> Srgba {
        srgba(&Color::rgb(v[0] / 255.0, v[1] / 255.0, v[2] / 255.0))
    }
    fn default_on(&self) -> DefaultOn {
        RgbModel.default_on()
    }
}

/// The colour's values in its own model (working RGB 0 to 255, CMYK %, K %, Lab L*a*b*), which the
/// framework's rows show and edit.
pub fn model_values(c: &Color) -> ModelValues {
    match c {
        Color::Rgb { .. } => ModelValues::new("rgb", &working_rgb(c).map(|v| (v * 255.0).clamp(0.0, 255.0))),
        Color::Cmyk { .. } => ModelValues::new("cmyk", &c.to_cmyk().map(|v| v * 100.0)),
        Color::Gray { k } => ModelValues::new("gray", &[k * 100.0]),
        Color::Lab { .. } => {
            let l = c.to_lab();
            ModelValues::new("lab", &[l.l, l.a, l.b])
        }
    }
}

/// The colour `v` describes, in the model it names (inverse of [`model_values`]): components stay
/// native and out-of-range or non-finite values clamp. An id the panel does not edit reads as RGB.
pub fn color_of_values(v: &ModelValues) -> Color {
    let value = |i: usize| v.values.get(i).copied().filter(|x| x.is_finite()).unwrap_or(0.0);
    let unit = |i: usize, max: f32| (value(i) / max).clamp(0.0, 1.0);
    match v.model {
        "gray" => Color::gray(unit(0, 100.0)),
        "cmyk" => Color::cmyk(unit(0, 100.0), unit(1, 100.0), unit(2, 100.0), unit(3, 100.0)),
        "lab" => Color::lab(value(0).clamp(0.0, 100.0), value(1).clamp(-128.0, 127.0), value(2).clamp(-128.0, 127.0)),
        "hsb" => Color::from_hsb(value(0).rem_euclid(360.0), unit(1, 100.0), unit(2, 100.0)),
        _ => Color::rgb(unit(0, 255.0), unit(1, 255.0), unit(2, 255.0)),
    }
}

/// The colour a framework pick holds: its model values when it has them, else its sRGB.
fn picked_color(p: &PickedColor) -> Color {
    match &p.model {
        Some(values) => color_of_values(values),
        None => Color::rgb(p.color.r, p.color.g, p.color.b),
    }
}

/// The app's printable test: maps a colour to its reproduction in the active CMYK space (the
/// document's assigned profile or else the working one) and back, when it is out of that gamut.
/// It reads the active colour management when called, so a settings or profile change follows.
pub(crate) fn gamut(_app: &VectorcraftApp) -> Gamut<'static> {
    Gamut::new(|c: Srgba| {
        let color = Color::rgb(c.r, c.g, c.b);
        if !color.out_of_gamut() {
            return c;
        }
        let cms = cms::active();
        let [r, g, b] = cms.convert(&color, cms::Model::Cmyk, cms.settings().intent).to_rgb();
        Srgba::new(r, g, b, c.a)
    })
}

/// Serde reader for [`crate::UiState::color_panel`]: junk gives the default, so an unknown value
/// doesn't lose the rest of the preferences.
pub fn deserialize_panel_state<'de, D: serde::Deserializer<'de>>(d: D) -> Result<ColorPanelState, D::Error> {
    let v = <Value as serde::Deserialize>::deserialize(d)?;
    Ok(serde_json::from_value(v).unwrap_or_default())
}

/// The panel settings of an old `ui` table, which kept them in four keys: `color_mode` (a menu
/// mode id, else none picked), `color_hide_options` (hidden only when `true`),
/// `color_show_recent_colors` (hidden only when `false`) and `color_rgb_percent` (percent only when
/// `true`).
pub fn legacy_panel_state(old: &Map<String, Value>) -> ColorPanelState {
    let mut state = ColorPanelState::default();
    if let Some(mode) = old.get("color_mode").and_then(Value::as_str).and_then(mode_of_id) {
        state.mode = mode;
    }
    let flag = |key: &str, default: bool| old.get(key).and_then(Value::as_bool).unwrap_or(default);
    state.hide_options = flag("color_hide_options", false);
    state.recent_colors = flag("color_show_recent_colors", true);
    state.rgb_percent = flag("color_rgb_percent", false);
    state
}

/// Serde reader for a saved on/off setting: `true` reads on and any other value off, so an unknown
/// value doesn't lose the rest of the preferences.
pub fn deserialize_lenient_bool<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    let v = <Value as serde::Deserialize>::deserialize(d)?;
    Ok(v.as_bool().unwrap_or_default())
}

/// Serde reader for the saved Show Recent Colors of the Swatches panel: `false` hides the row and
/// any other value shows it, so an unknown value doesn't lose the rest of the preferences.
pub fn deserialize_show_recent_colors<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    let v = <Value as serde::Deserialize>::deserialize(d)?;
    Ok(v.as_bool().unwrap_or(true))
}

/// What the panel shows for `tgt`: the Tint row of a swatch-linked colour, else the mode picked last
/// for new art (nothing selected) or when it holds the colour's own model, else the colour's model.
pub(crate) fn shown(app: &VectorcraftApp, tgt: &Target) -> ColorPanelShown {
    if let Some(t) = tgt.tint() {
        let mut row = TintRow::new(t.swatch.clone(), srgba(&t.base), t.tint * 100.0);
        // `Tint::of` only exists for global swatches.
        row.global = true;
        row.spot = t.spot;
        return ColorPanelShown::Tint(row);
    }
    let stored = picked_mode(app.ui.color_panel.mode);
    let defaults = app.session.active().is_none_or(|st| st.selection.subjects().is_empty());
    let mode = match (tgt.color(), stored) {
        (_, Some(m)) if defaults => m,
        (Some(c), stored) => stored.filter(|m| holds(*m, &c)).unwrap_or_else(|| own_mode(&c)),
        (None, stored) => stored.unwrap_or(ColorPanelMode::RgbSliders),
    };
    ColorPanelShown::Mode(mode)
}

/// `c` in `mode`'s model: picking CMYK or Grayscale converts the colour, Web Safe RGB snaps it.
fn convert_to(mode: ColorPanelMode, c: &Color) -> Color {
    if mode == ColorPanelMode::WebColorSliders {
        let [r, g, b] = web_safe(working_rgb(c));
        return Color::rgb(r, g, b);
    }
    if holds(mode, c) {
        return *c;
    }
    match mode_model(mode) {
        cms::Model::Gray => {
            let [r, g, b] = c.to_rgb();
            Color::gray((1.0 - (0.3 * r + 0.59 * g + 0.11 * b)).clamp(0.0, 1.0))
        }
        cms::Model::Cmyk => {
            let [c0, m, y, k] = c.to_cmyk();
            Color::cmyk(c0, m, y, k)
        }
        cms::Model::Lab => {
            let l = c.to_lab();
            Color::lab(l.l, l.a, l.b)
        }
        _ => {
            let [r, g, b] = working_rgb(c).map(|v| v.clamp(0.0, 1.0));
            Color::rgb(r, g, b)
        }
    }
}

/// The global or spot swatch a colour is a tint of (Tint mode).
pub(crate) struct Tint {
    swatch: String,
    spot: bool,
    /// 0..1.
    tint: f32,
    /// The swatch's colour (100 %).
    base: Color,
}

impl Tint {
    /// The tint `link` (a swatch name) and `tint` show: `None` unless the swatch is a global colour.
    fn of(app: &VectorcraftApp, link: Option<&str>, tint: f32) -> Option<Self> {
        let name = link?;
        let d = &app.session.active()?.doc;
        let spot = d.swatch(name)?.spot;
        Some(Self { swatch: name.to_string(), spot, tint, base: d.global_color(name)? })
    }
}

/// What the panel is editing: the active proxy's solid colour or a selected gradient stop (with
/// the global swatch it is a tint of), or a selected freeform point.
pub(crate) enum Target {
    Paint(Option<Color>, Option<Tint>),
    Stop { paint: Paint, index: usize, color: Color, tint: Option<Tint> },
    Point { index: usize, color: Color },
}

impl Target {
    pub(crate) fn color(&self) -> Option<Color> {
        match self {
            Target::Paint(c, _) => *c,
            Target::Stop { color, .. } | Target::Point { color, .. } => Some(*color),
        }
    }
    pub(crate) fn tint(&self) -> Option<&Tint> {
        match self {
            Target::Paint(_, t) | Target::Stop { tint: t, .. } => t.as_ref(),
            Target::Point { .. } => None,
        }
    }
}

/// The active proxy's colour or gradient stop; no colour for None, patterns and a "?" proxy.
fn target(app: &VectorcraftApp, ui: &Ui) -> Target {
    target_with(app, super::active_mixed(app, ui.ctx()))
}

/// Whether the active proxy is "?", read from the session (the panel reads its per-frame cache).
fn mixed_now(app: &VectorcraftApp) -> bool {
    let (fill, stroke) = app.session.proxy_mixed();
    if app.session.fill_active { fill } else { stroke }
}

/// [`target`] with `mixed` for a "?" active proxy: the panel reads it from its per-frame cache,
/// `ui.colorPanel` from the session.
fn target_with(app: &VectorcraftApp, mixed: bool) -> Target {
    if mixed {
        return Target::Paint(None, None);
    }
    let p = active_paint(app);
    match &p {
        Paint::Gradient(g) if g.gradient.kind == GradientKind::Freeform => {
            // Like stops: without a selected point, the first.
            let f = super::gradient::shown_points(g);
            let index = app.session.selected_freeform_point().filter(|i| *i < f.points.len()).unwrap_or(0);
            Target::Point { index, color: f.points.get(index).map_or(Color::BLACK, |p| p.color) }
        }
        Paint::Gradient(g) => {
            let i = app.session.selected_stop().unwrap_or(0).min(g.gradient.stops.len().saturating_sub(1));
            let (color, tint) = g.gradient.stops.get(i).map_or((Color::BLACK, None), |s| (s.color, Tint::of(app, s.swatch.as_deref(), s.tint)));
            Target::Stop { paint: p.clone(), index: i, color, tint }
        }
        Paint::Solid { color, swatch, tint } => Target::Paint(Some(*color), Tint::of(app, swatch.as_deref(), *tint)),
        _ => Target::Paint(None, None),
    }
}

/// [`target`] for the Color Wheel panel, which edits what this panel's sliders edit.
pub(crate) fn wheel_target(app: &VectorcraftApp, ui: &Ui) -> Target {
    target(app, ui)
}

/// Apply `c` to the target, or with `behind` (an Alt-click) to the inactive proxy as a solid
/// colour, keeping the active proxy in front.
pub(crate) fn apply(app: &mut VectorcraftApp, tgt: &Target, c: Color, phase: Live, behind: bool) {
    match tgt {
        Target::Stop { paint: Paint::Gradient(g), index, .. } if !behind => {
            let mut stops = g.gradient.stops.clone();
            if let Some(s) = stops.get_mut(*index) {
                s.set_color(c, None);
            }
            super::gradient::set_stops_keeping_models(app, &stops, None, phase);
        }
        Target::Point { index, .. } if !behind => {
            let p = json!({"index": index, "color": color_json(&c), "stroke": !app.session.fill_active, "keepModel": true});
            live_run(app, "Color", "paint.freeform.setPoint", p, phase);
        }
        _ => {
            let cmd = super::proxy_cmd(app, behind);
            // The panel's sliders work in the model picked there, even in a CMYK document.
            live_run(app, "Color", cmd, json!({"color": color_json(&c), "focus": !behind, "keepModel": true}), phase);
        }
    }
}

/// [`apply`] a color chosen in the panel; once the pick ends, it also becomes the Color Guide's
/// base color.
fn choose(app: &mut VectorcraftApp, tgt: &Target, c: Color, phase: Live, behind: bool) {
    if phase == Live::Released {
        super::color_guide::choose_base(app, c);
    }
    apply(app, tgt, c, phase, behind);
}

/// Set the target's tint of its swatch to `t` (0..1), keeping the link (the T slider).
fn apply_tint(app: &mut VectorcraftApp, tgt: &Target, tint: &Tint, t: f32, phase: Live) {
    match tgt {
        Target::Stop { paint: Paint::Gradient(g), index, .. } => {
            let mut stops = g.gradient.stops.clone();
            if let Some(s) = stops.get_mut(*index) {
                s.set_color(tint.base.tinted(t), Some((tint.swatch.clone(), t)));
            }
            super::gradient::set_stops(app, &stops, None, phase);
        }
        _ => {
            let cmd = super::proxy_cmd(app, false);
            live_run(app, "Color", cmd, json!({"swatch": tint.swatch, "tint": t * 100.0}), phase);
        }
    }
}

/// Switch the panel to `mode`, converting the colour to its model (a tint becomes a process
/// colour).
fn set_mode(app: &mut VectorcraftApp, tgt: &Target, mode: ColorPanelMode) {
    app.ui.color_panel.mode = mode;
    if let Some(c) = tgt.color() {
        let conv = convert_to(mode, &c);
        if conv != c || tgt.tint().is_some() {
            apply(app, tgt, conv, Live::Released, false);
        }
    }
}

/// Invert (`invert`) or Complement, as the panel menu's items do: a gradient stop or freeform point
/// is recolored in place, and a solid color or a "?" proxy goes through `paint.invert` or
/// `paint.complement` (each selected object keeps its color model). Without a color or a "?"
/// proxy nothing changes. With `behind` (Alt held) the command recolors the inactive proxy instead.
fn recolor(app: &mut VectorcraftApp, tgt: &Target, invert: bool, behind: bool) {
    let (cmd, f): (&str, fn(&Color) -> Color) =
        if invert { ("paint.invert", Color::invert_keep_model) } else { ("paint.complement", Color::complement_keep_model) };
    if behind {
        app.run(cmd, json!({ "stroke": app.session.fill_active })).ok();
        return;
    }
    match (tgt, tgt.color()) {
        (Target::Stop { .. } | Target::Point { .. }, Some(c)) => apply(app, tgt, f(&c), Live::Released, false),
        (_, color) if color.is_some() || mixed_now(app) => {
            app.run(cmd, json!({})).ok();
        }
        _ => {}
    }
}

/// Run what the panel reported this pass on the document: the edits (the inactive proxy for a
/// `Background` target), the Tint row, None, a recolour click and the proxy's own events. The
/// gradient stop popover, which embeds the panel, hands its output here too.
pub(crate) fn apply_output(app: &mut VectorcraftApp, out: &ColorPanelOutput) {
    let tgt = target_now(app);
    apply_with(app, &tgt, out);
}

/// [`target`] with the session's "?" state, for callers without the panel's pass.
pub(crate) fn target_now(app: &VectorcraftApp) -> Target {
    target_with(app, mixed_now(app))
}

fn apply_with(app: &mut VectorcraftApp, tgt: &Target, out: &ColorPanelOutput) {
    for (which, event) in &out.edits {
        let behind = matches!(which, PanelTarget::Background);
        apply_event(app, event, &EditPath::Preview, &mut |app, picked: &PickedColor, phase| {
            choose(app, tgt, picked_color(picked), phase, behind);
        });
    }
    if let (Some(event), Some(tint)) = (&out.tint, tgt.tint()) {
        apply_event(app, event, &EditPath::Preview, &mut |app, percent: &f32, phase| {
            let t = (percent / 100.0).clamp(0.0, 1.0);
            if phase == Live::Released {
                super::color_guide::choose_linked_base(app, tint.base.tinted(t), Some((tint.swatch.clone(), t)));
            }
            apply_tint(app, tgt, tint, t, phase);
        });
    }
    if let Some(which) = out.none {
        let cmd = super::proxy_cmd(app, matches!(which, PanelTarget::Background));
        app.run(cmd, json!({"none": true})).ok();
    }
    if let Some(r) = out.recolor {
        recolor(app, tgt, r.invert, r.other);
    }
    if let Some(pair) = &out.pair {
        proxy_events(app, pair);
    }
}

/// `ui.colorPanel {mode?, options?, recentColors?, rgbPercent?}`: pick a mode as the panel menu
/// does ([`set_mode`]), Show Options (true) or Hide Options (false), check (true) or uncheck
/// (false) Show Recent Colors, and show RGB in percent (true) or 0 to 255 (false). → {mode,
/// options, recentColors, rgbPercent, shown}: the mode picked last (null: none yet), whether the
/// options show, whether Show Recent Colors is checked, whether RGB shows in percent, and the mode
/// the panel shows the color in (`tint` for a tint of a global or spot swatch).
pub(crate) fn command(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let mode = match p.get("mode") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_str().and_then(mode_of_id).ok_or("`mode` must be grayscale, rgb, hsb, cmyk or websafe")?),
    };
    let options = crate::menus::opt_bool(p, "options")?;
    let recent = crate::menus::opt_bool(p, "recentColors")?;
    let percent = crate::menus::opt_bool(p, "rgbPercent")?;
    if let Some(m) = mode {
        set_mode(app, &target_now(app), m);
    }
    if let Some(shown) = options {
        app.ui.color_panel.hide_options = !shown;
    }
    if let Some(on) = recent {
        app.ui.color_panel.recent_colors = on;
    }
    if let Some(on) = percent {
        app.ui.color_panel.rgb_percent = on;
    }
    let shown = match shown(app, &target_now(app)) {
        ColorPanelShown::Tint(_) => "tint",
        ColorPanelShown::Mode(m) => shown_id(m),
        _ => "rgb",
    };
    let state = &app.ui.color_panel;
    Ok(json!({
        "mode": picked_mode(state.mode).map(shown_id),
        "options": !state.hide_options,
        "recentColors": state.recent_colors,
        "rgbPercent": state.rgb_percent,
        "shown": shown,
    }))
}

/// `ui.colorRgbPercent {}`: [`switch_rgb_units`]. A parameter is an error and changes nothing.
pub(crate) fn rgb_percent_command(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    match p {
        Value::Null => {}
        Value::Object(o) => {
            if let Some(key) = o.keys().next() {
                return Err(format!("unknown parameter `{key}`; the command takes no parameters"));
            }
        }
        got => return Err(format!("the params must be an object, not {got}")),
    }
    Ok(json!(switch_rgb_units(app)))
}

/// Switch the RGB mode's R, G and B between percent and 0 to 255, as a double-click right of an
/// RGB field does. → whether they now show in percent.
fn switch_rgb_units(app: &mut VectorcraftApp) -> bool {
    app.ui.color_panel.rgb_percent = !app.ui.color_panel.rgb_percent;
    app.ui.color_panel.rgb_percent
}

/// The proxy's paint as one well, drawn as the Tools panel's proxy draws it (a gradient, None or a
/// pattern as itself), and the color the panel is given for it: a solid paint's color, black for any other paint.
fn well(label: &'static str, paint: &Paint, mixed: bool) -> (PairWell, Srgba) {
    let color = srgba(&paint.color().unwrap_or(Color::BLACK));
    (PairWell::paint(label, if mixed { Mixed::Mixed } else { Mixed::Same(well_paint(paint, &|_| None)) }), color)
}

/// Draw numeric paint controls and run what they report.
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let tgt = target(app, ui);
    let (fill_mixed, stroke_mixed) = super::mixed_paints(app, ui.ctx());
    let (fill, stroke) = super::current_paints(app);
    let (fill_well, fill_color) = well(tl!("Fill"), &fill, fill_mixed);
    let (stroke_well, stroke_color) = well(tl!("Stroke"), &stroke, stroke_mixed);
    let fill_active = app.session.fill_active;
    let colors = if fill_active { [fill_color, stroke_color] } else { [stroke_color, fill_color] };
    let active = if fill_active { Well::Primary } else { Well::Secondary };
    let edited = tgt.color().map(|c| PickedColor::in_model(srgba(&c), model_values(&c)));
    let rows = shown(app, &tgt);
    let printable = gamut(app);
    let stored: Vec<Color> = app.session.recent_colors.to_vec();
    let chips: Vec<Srgba> = stored.iter().map(srgba).collect();
    let before = app.ui.color_panel;
    let out = ColorPanel::new(Bind::bound(&mut app.ui.color_panel), colors)
        .layout(LAYOUT)
        .wells([fill_well, stroke_well], Bind::given(active))
        .edited(edited)
        .shown(rows)
        .recent(&chips)
        // The drag keeps a recent colour's own model for the Swatches panel.
        .recent_payload(move |i| stored.get(i).copied())
        .gamut(printable)
        .working_rgb(&WorkingRgb)
        .show(ui);
    // PLAN: a click on a recent chip reports only its sRGB, so a CMYK recent colour paints as RGB; co.K18 needs the edit to name the chip.
    apply_with(app, &tgt, &out);
    let now = app.ui.color_panel.mode;
    // Shift-click on the spectrum cycles the mode inside the panel; the colour follows it here.
    if now != before.mode && picked_mode(now).is_some() {
        set_mode(app, &tgt, now);
    }
}

/// Carry out what the panel menu asked of the app.
fn carry_out(app: &mut VectorcraftApp, tgt: &Target, action: ColorPanelAction) {
    match action {
        ColorPanelAction::Mode(m) => set_mode(app, tgt, m),
        ColorPanelAction::Copy(text) => {
            if let Some(ctx) = &app.ui_ctx {
                ctx.copy_text(text);
            }
        }
        ColorPanelAction::Invert => recolor(app, tgt, true, false),
        ColorPanelAction::Complement => recolor(app, tgt, false, false),
        ColorPanelAction::NewSwatch => {
            if let Some(c) = tgt.color() {
                // A tint saves a tint swatch ("Name 40%"); a color keeps its model.
                let p = match tgt.tint() {
                    Some(t) => json!({"swatch": t.swatch, "tint": t.tint * 100.0}),
                    None => json!({"color": color_json(&c), "keepModel": true}),
                };
                app.run("swatch.new", p).ok();
            }
        }
        _ => {}
    }
}

/// The panel menu: Hide Options and Show Recent Colors, the five modes, Invert, Complement, Create
/// New Swatch and Copy Color Value.
pub(crate) fn menu_rows(_app: &VectorcraftApp) -> Vec<PanelRow> {
    ColorPanelMenu::rows(LAYOUT)
}

/// The checks, titles and enabled states of [`menu_rows`]: the mode the panel shows is checked, and
/// the rows that need a colour are off without one.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    let tgt = target_now(app);
    let has_color = tgt.color().is_some() || mixed_now(app);
    for (command, state) in ColorPanelMenu::states(LAYOUT, &app.ui.color_panel, &shown(app, &tgt), has_color) {
        states.set(command, state);
    }
}

/// Run the menu row `id` (a `colorPanel.*` command); `None` when it is not one of this panel's.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    let command = ColorPanelCommand::from_ref(&CommandRef::new(id))?;
    let tgt = target_now(app);
    let edited = srgba(&tgt.color().unwrap_or(Color::BLACK));
    if let Some(action) = ColorPanelMenu::run_command(&mut app.ui.color_panel, command, edited) {
        carry_out(app, &tgt, action);
    }
    Some(Ok(Value::Null))
}

#[cfg(test)]
mod tests;
