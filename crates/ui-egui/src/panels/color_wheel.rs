//! Color Wheel panel (Window › Color Wheel): the Fill and Stroke proxy beside a hue ring around a
//! square with saturation across and brightness up at the ring's hue. The wheel edits what the
//! Color panel's sliders edit (the active proxy's color, the selected gradient stop or freeform
//! point) through the same paint commands, and a drag is one undo step. Alt paints the inactive
//! proxy and Shift steps the ring by 45°. A pick is display sRGB, converted into the working RGB
//! space: an RGB, CMYK or Lab color keeps its model, and a Gray color or a tint of a global swatch
//! becomes a process color in the document's color mode. The HSB picked last stays while the
//! objects, the proxy, the color and the color settings are unchanged, so a gray keeps its hue.
//!
//! The panel menu's Box View puts a box in the wheel's place: a color field with a vertical
//! channel strip at its right, for the channel picked under Box Channel (Hue, Saturation,
//! Brightness, Red, Green or Blue). The field shows the other two components on the Color
//! Picker's axes.
//!
//! The panel menu's Show Opacity adds a row under the wheel or the box that sets the opacity of
//! what the panel edits: the fill or stroke item (`paint.setFill {opacity}`), the selected
//! gradient stop or the selected freeform point (the first stop or point when none is selected,
//! as for a color pick). The row is gray with nothing selected, for an object selected in a symbol
//! instance, and when the fill or stroke item it would set is on type characters or missing. The
//! setting is kept with the preferences (`ui.colorWheel`).
//!
//! Panel menu › Show Hex Field adds a field under the wheel or the box that shows the color's six
//! hex digits, and two more when the opacity that Show Opacity sets rounds to less than `ff`. It
//! takes a hex color of 3, 4, 6 or 8 digits and a CSS `rgb()`, `hsl()` or `oklch()` color
//! (`color.parse`). A typed color is applied in the same way as a pick, and an alpha sets the
//! opacity that Show Opacity sets, whether or not that row shows. Where that row is gray, only the
//! color is applied.
//!
//! Show Harmony puts a marker on the ring for each other color of the harmony rule picked under
//! Harmony Rule (the Color Guide's rules whose colors each have a hue of their own), filled with
//! that color. The markers follow the current color, and a click on one applies its color as a
//! pick does.
//!
//! Panel menu › Show Color Vision Preview adds a row under the wheel or the box with the color as
//! simulated for protanopia, deuteranopia and tritanopia, through the simulations View › Proof
//! Setup uses, and for achromatopsia as the gray with the color's CIE L*. The chips only show the
//! simulation.
//!
//! Panel menu › OKLCH Sliders shows L, C and H rows under the wheel or the box that edit the color
//! by OKLCH lightness, chroma and hue (CSS Color 4:
//! <https://www.w3.org/TR/css-color-4/#specifying-oklab-oklch>), while the active document's RGB
//! space ([`cms::active`]: the profile assigned to the document, else the working RGB space of the
//! color settings) is sRGB. Panel menu › HSL Sliders shows H, S and L rows in their place that edit the color by the
//! hue, saturation and lightness of CSS Color 4's HSL
//! (<https://www.w3.org/TR/css-color-4/#the-hsl-notation>). The choice is kept with the
//! preferences too.
//!
//! The view, the channel, Show Opacity, Show Hex Field, the slider rows, Show Harmony, Harmony Rule
//! and Show Color Vision Preview are kept with the preferences (`ui.colorWheel`).

use egui::{Color32, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_color::cms::{self, Cms, ProofSetup, ProofTarget};
use vectorcraft_color::harmony::Harmony;
use vectorcraft_color::oklab::{self, CHROMA_FULL};
use vectorcraft_color::{Color, Paint, hsb_to_rgb, hsl_to_rgb, rgb_to_hsb, rgb_to_hsl};
use vectorcraft_doc::{ColorMode, NodeId};

use super::color::{Target, apply, wheel_target};
use super::{color_json, live_run, pstate, set_pstate};
use crate::VectorcraftApp;
use crate::dialogs::color_picker::Channel;
use crate::theme::Tokens;
use crate::widgets::{self, FieldAxis, Live, SliderValue, menu_item};

pub const ID: &str = "colorWheel";
/// The widest the wheel is drawn.
const MAX_WHEEL: f32 = 200.0;
/// The width of the box's channel strip, its arrows included.
const STRIP: f32 = 22.0;
/// The space between the box's field and its channel strip.
const STRIP_GAP: f32 = 6.0;
/// The width of the opacity row's percent field.
const OPACITY_FIELD: f32 = 46.0;

/// What the panel shows right of the proxy: the wheel, or the box (a color field and a channel
/// strip).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum View {
    #[default]
    Wheel,
    Box,
}

impl View {
    pub const ALL: [View; 2] = [View::Wheel, View::Box];

    pub fn id(self) -> &'static str {
        match self {
            View::Wheel => "wheel",
            View::Box => "box",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.id() == s)
    }
}

/// The panel's settings, kept with the preferences (`UiState::color_wheel`, `ui.colorWheel`).
/// Each key reads on its own, and a value this build does not know reads as the key's default.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    #[serde(deserialize_with = "deserialize_view")]
    pub view: View,
    #[serde(serialize_with = "serialize_channel", deserialize_with = "deserialize_channel")]
    pub channel: Channel,
    /// Show Opacity: the opacity row under the wheel or the box.
    #[serde(deserialize_with = "deserialize_flag")]
    pub opacity: bool,
    /// Show Hex Field: the field for a hex or CSS color under the wheel or the box.
    #[serde(deserialize_with = "deserialize_flag")]
    pub hex_field: bool,
    #[serde(deserialize_with = "deserialize_sliders")]
    pub sliders: Sliders,
    /// Show Harmony: markers on the ring for the other colors of `harmony_rule`.
    #[serde(deserialize_with = "deserialize_flag")]
    pub harmony: bool,
    /// The rule of Show Harmony, one of [`harmony_rules`].
    #[serde(serialize_with = "serialize_rule", deserialize_with = "deserialize_rule")]
    pub harmony_rule: Harmony,
    /// Show Color Vision Preview: the row of simulated colors under the wheel or the box. A saved
    /// `true` shows the row and any other value hides it.
    #[serde(deserialize_with = "crate::panels::color::deserialize_rgb_percent")]
    pub vision: bool,
}

/// The slider rows under the wheel or the box (panel menu, `ui.colorWheel {sliders}`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Sliders {
    #[default]
    None,
    /// OKLCH Sliders: lightness, chroma and hue.
    Oklch,
    /// HSL Sliders: hue, saturation and lightness.
    Hsl,
}

impl Sliders {
    pub const ALL: [Sliders; 3] = [Sliders::None, Sliders::Oklch, Sliders::Hsl];

    /// The id `ui.colorWheel` and the preferences use.
    pub fn id(self) -> &'static str {
        match self {
            Sliders::None => "none",
            Sliders::Oklch => "oklch",
            Sliders::Hsl => "hsl",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.id() == s)
    }
}

fn deserialize_sliders<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Sliders, D::Error> {
    let v = <Value as Deserialize>::deserialize(d)?;
    Ok(v.as_str().and_then(Sliders::parse).unwrap_or_default())
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            view: View::default(),
            channel: Channel::default(),
            opacity: false,
            hex_field: false,
            sliders: Sliders::default(),
            harmony: false,
            harmony_rule: Harmony::Complementary,
            vision: false,
        }
    }
}

/// The harmony rules the wheel shows: those whose colors each have a hue of their own, so that no
/// two markers sit at the same place on the ring.
pub fn harmony_rules() -> impl Iterator<Item = Harmony> {
    Harmony::ALL.into_iter().filter(|h| h.hues_differ())
}

/// The wheel's harmony rule named by `s`, its id or its label.
fn parse_rule(s: &str) -> Option<Harmony> {
    Harmony::parse(s).filter(|h| h.hues_differ())
}

fn serialize_rule<S: serde::Serializer>(h: &Harmony, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&h.id())
}

fn deserialize_rule<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Harmony, D::Error> {
    let v = <Value as Deserialize>::deserialize(d)?;
    Ok(v.as_str().and_then(parse_rule).unwrap_or(Harmony::Complementary))
}

fn deserialize_view<'de, D: serde::Deserializer<'de>>(d: D) -> Result<View, D::Error> {
    let v = <Value as Deserialize>::deserialize(d)?;
    Ok(v.as_str().and_then(View::parse).unwrap_or_default())
}

fn serialize_channel<S: serde::Serializer>(c: &Channel, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(c.id())
}

fn deserialize_channel<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Channel, D::Error> {
    let v = <Value as Deserialize>::deserialize(d)?;
    Ok(v.as_str().and_then(Channel::parse).unwrap_or_default())
}

fn deserialize_flag<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    let v = <Value as Deserialize>::deserialize(d)?;
    Ok(v.as_bool().unwrap_or_default())
}

/// Serde reader for the saved settings ([`crate::UiState::color_wheel`]): a value that is not an
/// object reads as the defaults, so it doesn't lose the rest of the preferences.
pub fn deserialize_settings<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Settings, D::Error> {
    let v = <Value as Deserialize>::deserialize(d)?;
    Ok(serde_json::from_value(v).unwrap_or_default())
}

/// `ui.colorWheel {view?, channel?, opacity?, hexField?, sliders?, harmony?, harmonyRule?, vision?}`:
/// show the wheel or the box, pick the box's channel, check or uncheck Show Opacity and Show Hex
/// Field, show OKLCH Sliders, HSL Sliders or neither, show the colors of a harmony rule on the
/// wheel, and show or hide the color vision row. Keys left out or null keep their values; params
/// that are not an object, another key or a value off the list are an error and change nothing.
/// `sliders: "oklch"` is an error while the active document's RGB space (its assigned profile,
/// else the working RGB space) is not sRGB. → the settings.
pub(crate) fn command(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    set(&mut app.ui.color_wheel, p, &cms::active())?;
    Ok(json!(app.ui.color_wheel))
}

/// Set the keys of `ui.colorWheel` params `p` in `settings`, with the color settings `cms`; on an
/// error `settings` stay as they were.
pub(crate) fn set(settings: &mut Settings, p: &Value, cms: &Cms) -> Result<(), String> {
    const KEYS: [&str; 8] = ["view", "channel", "opacity", "hexField", "sliders", "harmony", "harmonyRule", "vision"];
    match p {
        Value::Null => {}
        Value::Object(o) => {
            if let Some(key) = o.keys().find(|k| !KEYS.contains(&k.as_str())) {
                return Err(format!("unknown parameter `{key}`; the command takes: {}", KEYS.join(", ")));
            }
        }
        got => return Err(format!("the params must be an object, not {got}")),
    }
    let mut set = settings.clone();
    match p.get("view") {
        None | Some(Value::Null) => {}
        Some(v) => set.view = v.as_str().and_then(View::parse).ok_or("`view` must be wheel or box")?,
    }
    match p.get("channel") {
        None | Some(Value::Null) => {}
        Some(v) => set.channel = v.as_str().and_then(Channel::parse).ok_or("`channel` must be hue, saturation, brightness, red, green or blue")?,
    }
    match p.get("opacity") {
        None | Some(Value::Null) => {}
        Some(v) => set.opacity = v.as_bool().ok_or("`opacity` must be true or false")?,
    }
    match p.get("hexField") {
        None | Some(Value::Null) => {}
        Some(v) => set.hex_field = v.as_bool().ok_or("`hexField` must be true or false")?,
    }
    match p.get("sliders") {
        None | Some(Value::Null) => {}
        Some(v) => {
            let sliders = v.as_str().and_then(Sliders::parse).ok_or("`sliders` must be none, oklch or hsl")?;
            if sliders == Sliders::Oklch && !cms.rgb_is_srgb() {
                return Err(OKLCH_NEEDS_SRGB.to_string());
            }
            set.sliders = sliders;
        }
    }
    let key = "harmony";
    match p.get(key) {
        None | Some(Value::Null) => {}
        Some(v) => set.harmony = v.as_bool().ok_or_else(|| format!("`{key}` must be true or false"))?,
    }
    let key = "harmonyRule";
    match p.get(key) {
        None | Some(Value::Null) => {}
        Some(v) => {
            set.harmony_rule = v.as_str().and_then(parse_rule).ok_or_else(|| {
                let ids: Vec<String> = harmony_rules().map(Harmony::id).collect();
                format!("`{key}` must be one of {}", ids.join(", "))
            })?
        }
    }
    if let Some(on) = crate::menus::opt_bool(p, "vision")? {
        set.vision = on;
    }
    *settings = set;
    Ok(())
}

/// The panel menu: Wheel View and Box View, and Box Channel while the box shows; HSL Sliders and
/// OKLCH Sliders; Show Opacity and Show Hex Field; Show Harmony and Harmony Rule while the wheel
/// shows; and Show Color Vision Preview.
pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let set = app.ui.color_wheel.clone();
    for (label, view) in [(tl!("Wheel View"), View::Wheel), (tl!("Box View"), View::Box)] {
        if menu_item(ui, label, true, set.view == view) {
            command(app, &json!({ "view": view.id() })).ok();
        }
    }
    ui.add_enabled_ui(set.view == View::Box, |ui| {
        ui.menu_button(format!("   {}", tl!("Box Channel")), |ui| {
            for ch in Channel::ALL {
                if menu_item(ui, ch.name(), true, ch == set.channel) {
                    command(app, &json!({ "channel": ch.id() })).ok();
                }
            }
        });
    });
    ui.separator();
    let on = set.sliders == Sliders::Hsl;
    if menu_item(ui, tl!("HSL Sliders"), true, on) {
        let sliders = if on { Sliders::None } else { Sliders::Hsl };
        if let Err(e) = app.run("ui.colorWheel", json!({ "sliders": sliders.id() })) {
            app.status(e);
        }
    }
    let on = set.sliders == Sliders::Oklch;
    // While the rows show, the item stays enabled so that they can be hidden.
    let enabled = on || cms::active().rgb_is_srgb();
    if widgets::menu_item_hint(ui, tl!("OKLCH Sliders"), enabled, on, tl!("OKLCH needs sRGB as the working RGB space")) {
        let sliders = if on { Sliders::None } else { Sliders::Oklch };
        if let Err(e) = app.run("ui.colorWheel", json!({ "sliders": sliders.id() })) {
            app.status(e);
        }
    }
    ui.separator();
    if menu_item(ui, tl!("Show Opacity"), true, set.opacity) {
        command(app, &json!({ "opacity": !set.opacity })).ok();
    }
    if menu_item(ui, tl!("Show Hex Field"), true, set.hex_field) {
        command(app, &json!({ "hexField": !set.hex_field })).ok();
    }
    if menu_item(ui, tl!("Show Harmony"), set.view == View::Wheel, set.harmony) {
        command(app, &json!({ "harmony": !set.harmony })).ok();
    }
    ui.add_enabled_ui(set.view == View::Wheel, |ui| {
        ui.menu_button(format!("   {}", tl!("Harmony Rule")), |ui| {
            for rule in harmony_rules() {
                if menu_item(ui, rule.label(), true, rule == set.harmony_rule) {
                    command(app, &json!({ "harmonyRule": rule.id() })).ok();
                }
            }
        });
    });
    if menu_item(ui, tl!("Show Color Vision Preview"), true, set.vision) {
        command(app, &json!({ "vision": !set.vision })).ok();
    }
}

/// The display values of HSB `hsb` in `ch`'s space, from 0 to 1: hue, saturation and brightness,
/// or red, green and blue.
fn box_values(ch: Channel, hsb: [f32; 3]) -> [f32; 3] {
    if ch.is_hsb() { [hsb[0] / 360.0, hsb[1], hsb[2]] } else { hsb_to_rgb(hsb) }
}

/// The strip's value and the field's point (x across, y down; 0 to 1 each) of HSB `hsb` in the box
/// of channel `ch`.
pub(crate) fn box_pos(ch: Channel, hsb: [f32; 3]) -> (f32, f32, f32) {
    let v = box_values(ch, hsb);
    let (z, x, y) = ch.axes();
    (v[z], v[x], 1.0 - v[y])
}

/// The HSB at strip value `z` and field point (x, y) in the box of channel `ch`: the inverse of
/// [`box_pos`]. A pick in an RGB channel that has no hue (a gray) keeps the hue of `was`.
pub(crate) fn box_pick(ch: Channel, z: f32, x: f32, y: f32, was: [f32; 3]) -> [f32; 3] {
    let (zi, xi, yi) = ch.axes();
    let mut v = [0.0; 3];
    v[zi] = z;
    v[xi] = x;
    v[yi] = 1.0 - y;
    let v = v.map(|c| c.clamp(0.0, 1.0));
    if ch.is_hsb() {
        return [v[0] * 360.0, v[1], v[2]];
    }
    let hsb = rgb_to_hsb(v);
    if hsb[1] <= 0.0 || hsb[2] <= 0.0 { [was[0], hsb[1], hsb[2]] } else { hsb }
}

/// The number the strip of channel `ch` reads to screen readers for HSB `hsb`, its suffix and its
/// largest value: hue in degrees, saturation and brightness in percent, red, green and blue from 0
/// to 255.
fn strip_number(ch: Channel, hsb: [f32; 3]) -> (f64, &'static str, f64) {
    let (units, suffix) = match ch {
        Channel::Hue => (360.0, "°"),
        Channel::Saturation | Channel::Brightness => (100.0, "%"),
        Channel::Red | Channel::Green | Channel::Blue => (255.0, ""),
    };
    let (z, _, _) = box_pos(ch, hsb);
    (f64::from(z) * units, suffix, units)
}

/// The box in a `size` × `size` area: the field of channel `ch` and its strip, on HSB `hsb`, with
/// markers when `marked`. Returns what a click, a drag, a key or a screen reader's action picks,
/// and its phase.
fn show_box(ui: &mut Ui, ch: Channel, hsb: [f32; 3], marked: bool, size: f32) -> Option<(Picked, Live)> {
    let (z, x, y) = box_pos(ch, hsb);
    let mut pick = None;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = STRIP_GAP;
        let field = vec2((size - STRIP_GAP - STRIP).max(1.0), size);
        let at = |x: f32, y: f32| widgets::hsb32(box_pick(ch, z, x, y, hsb));
        if let (Some((x, y)), phase) = widgets::color_field(ui, "color-wheel-box", (tl!("Color box"), ch.field_axes()), field, (x, y), marked, &at) {
            pick = Some((Picked::Field(ch, x, y), phase));
        }
        // The hue strip shows the whole spectrum; the others change only their own component.
        let track = |z: f32| widgets::hsb32(if ch == Channel::Hue { [z * 360.0, 1.0, 1.0] } else { box_pick(ch, z, x, y, hsb) });
        let (v, suffix, max) = strip_number(ch, hsb);
        let shown = SliderValue::field(v, 0.0..=max, suffix, 0);
        match widgets::channel_slider_stepped(ui, "color-wheel-strip", ch.name(), Some(shown), z, marked, vec2(STRIP, size), &track) {
            (Some(z), phase, _) => pick = Some((Picked::Strip(ch, z), phase)),
            (None, _, Some(v)) => pick = Some((Picked::Strip(ch, (v / max) as f32), Live::Released)),
            _ => {}
        }
    });
    pick
}

/// What a press, a drag, a key or a screen reader's action sets on the wheel or the box: the
/// components it gives, the others coming from the HSB it is applied to ([`Picked::hsb`]).
#[derive(Clone, Copy)]
enum Picked {
    /// The hue ring's hue.
    Hue(f32),
    /// The square's saturation and brightness.
    Square(f32, f32),
    /// The box's field point (x across, y down) in channel `ch`.
    Field(Channel, f32, f32),
    /// The box's strip value in channel `ch`.
    Strip(Channel, f32),
    /// A whole HSB, such as a harmony marker's color.
    Whole([f32; 3]),
}

impl Picked {
    /// The HSB this pick gives on a color showing HSB `was`.
    fn hsb(self, was: [f32; 3]) -> [f32; 3] {
        match self {
            Picked::Hue(h) => [h, was[1], was[2]],
            Picked::Whole(hsb) => hsb,
            Picked::Square(s, b) => [was[0], s, b],
            Picked::Field(ch, x, y) => box_pick(ch, box_pos(ch, was).0, x, y, was),
            Picked::Strip(ch, z) => {
                let (_, x, y) = box_pos(ch, was);
                box_pick(ch, z, x, y, was)
            }
        }
    }
}

/// The opacity (0 to 1) the row shows for `tgt` and whether the row can set it: the selected
/// gradient stop's or freeform point's (the first when none is selected, as [`wheel_target`]
/// picks it), else that of the fill or stroke item
/// ([`vectorcraft_engine::Session::proxy_opacity`]), which nothing selected and type characters
/// do not have. A stop or point of a gradient on type characters belongs to the gradient, so the
/// row sets it. With nothing selected the stop or point belongs to the defaults for new art, which
/// the row leaves alone, and an object selected in a symbol instance has no opacity the row can set.
fn shown_opacity(app: &VectorcraftApp, tgt: &Target) -> (f32, bool) {
    let stroke = !app.session.fill_active;
    let selected = app.session.active().is_some_and(|d| !d.selection.is_empty() && d.selection.symbol_part.is_none());
    let point = |index: usize| match super::active_paint(app) {
        Paint::Gradient(g) => super::gradient::shown_points(&g).points.get(index).map(|p| p.opacity),
        _ => None,
    };
    let found = match tgt {
        _ if !selected => None,
        Target::Stop { paint: Paint::Gradient(g), index, .. } => g.gradient.stops.get(*index).map(|s| s.opacity),
        Target::Point { index, .. } => point(*index),
        _ => app.session.proxy_opacity(stroke),
    };
    (found.unwrap_or(1.0), found.is_some())
}

/// Set the opacity (0 to 1) of what the row edits for `tgt`: a stop through the Gradient panel's
/// edit, a freeform point through `paint.freeform.setPoint`, else the fill or stroke item through
/// `paint.setFill` or `paint.setStroke`. A drag previews and commits as one undo step.
fn apply_opacity(app: &mut VectorcraftApp, tgt: &Target, o: f32, phase: Live) {
    let o = o.clamp(0.0, 1.0);
    match tgt {
        Target::Stop { paint: Paint::Gradient(g), index, .. } => {
            let mut stops = g.gradient.stops.clone();
            if let Some(s) = stops.get_mut(*index) {
                s.opacity = o;
            }
            super::gradient::set_stops_keeping_models(app, &stops, None, phase);
        }
        Target::Point { index, .. } => {
            let p = json!({"index": index, "opacity": o, "stroke": !app.session.fill_active});
            live_run(app, "Opacity", "paint.freeform.setPoint", p, phase);
        }
        _ => {
            let cmd = super::proxy_cmd(app, false);
            live_run(app, "Opacity", cmd, json!({"opacity": f64::from(o) * 100.0}), phase);
        }
    }
}

/// The Show Opacity row: the Transparency panel's icon, a slider whose track shows `color` from
/// clear to solid, and a percent field. Gray when [`shown_opacity`] finds nothing to set. True when
/// it set the opacity.
fn opacity_row(app: &mut VectorcraftApp, ui: &mut Ui, tgt: &Target, color: &Color) -> bool {
    let (o, live) = shown_opacity(app, tgt);
    let percent = f64::from(o) * 100.0;
    let mut edit: Option<(f32, Live)> = None;
    ui.add_enabled_ui(live, |ui| {
        ui.horizontal(|ui| {
            let t = crate::theme::Tokens::get(ui.ctx());
            crate::icons::icon(ui, "dc-transparency", 16.0, if live { t.icon } else { t.text_disabled }).on_hover_text(tl!("Opacity"));
            let width = (ui.available_width() - OPACITY_FIELD - ui.spacing().item_spacing.x).max(40.0);
            let shown = SliderValue::field(percent, 0.0..=100.0, "%", 0);
            match widgets::opacity_slider(ui, "color-wheel-opacity", tl!("Opacity"), Some(shown), o, width, super::c32(color)) {
                (Some(v), phase, _) => edit = Some((v, phase)),
                (None, _, Some(v)) => edit = Some(((v / 100.0) as f32, Live::Released)),
                _ => {}
            }
            if let Some(v) = widgets::plain_field_named(ui, "color-wheel-opacity-field", Some(tl!("Opacity")), percent, "%", 0, OPACITY_FIELD) {
                edit = Some(((v.clamp(0.0, 100.0) / 100.0) as f32, Live::Released));
            }
        });
    });
    let Some((v, phase)) = edit.filter(|_| live) else { return false };
    apply_opacity(app, tgt, v, phase);
    true
}

/// The text the hex field shows for `color` and the opacity `o` the row of Show Opacity edits
/// (`None` when it has nothing to edit): six hex digits of the document's RGB numbers, and two more
/// when the opacity rounds to less than `ff`. Empty without a solid color.
pub(crate) fn hex_text(color: Option<&Color>, o: Option<f32>) -> String {
    let Some(c) = color else { return String::new() };
    let mut text = super::color::hex_digits(c);
    let alpha = o.map(|o| (o.clamp(0.0, 1.0) * 255.0).round() as u8).filter(|a| *a < 255);
    if let Some(a) = alpha {
        text.push_str(&format!("{a:02X}"));
    }
    text
}

/// Apply `c`, typed into the hex field, to `tgt` in one undo step, as a pick applies a color; the
/// alpha `alpha` (0 to 1) also sets the opacity of what the row of Show Opacity edits, when it has
/// something to edit.
fn apply_typed(app: &mut VectorcraftApp, tgt: &Target, c: Color, alpha: Option<f32>) {
    let alpha = alpha.filter(|_| shown_opacity(app, tgt).1).map(|a| a.clamp(0.0, 1.0));
    match tgt {
        Target::Stop { paint: Paint::Gradient(g), index, .. } => {
            let mut stops = g.gradient.stops.clone();
            if let Some(s) = stops.get_mut(*index) {
                s.set_color(c, None);
                if let Some(a) = alpha {
                    s.opacity = a;
                }
            }
            super::gradient::set_stops_keeping_models(app, &stops, None, Live::Released);
        }
        Target::Point { index, .. } => {
            let mut p = json!({"index": index, "color": color_json(&c), "stroke": !app.session.fill_active, "keepModel": true});
            if let Some(a) = alpha {
                p["opacity"] = json!(a);
            }
            live_run(app, "Color", "paint.freeform.setPoint", p, Live::Released);
        }
        _ => {
            let cmd = super::proxy_cmd(app, false);
            let mut p = json!({"color": color_json(&c), "focus": true, "keepModel": true});
            if let Some(a) = alpha {
                p["opacity"] = json!(f64::from(a) * 100.0);
            }
            live_run(app, "Color", cmd, p, Live::Released);
        }
    }
}

/// The text the hex field shows for `tgt` and its color, as [`hex_text`] gives it.
fn hex_shown(app: &VectorcraftApp, tgt: &Target, color: Option<&Color>) -> String {
    let (o, live) = shown_opacity(app, tgt);
    hex_text(color, live.then_some(o))
}

/// The hex field's id for `tgt`: it names the document, the selected objects or text characters,
/// the object selected in a symbol instance, the appearance item, the proxy and the gradient stop or
/// freeform point, but not the color, so a text typed for one of them is dropped when another one is
/// targeted, even one showing the same hex.
fn hex_field_id(app: &VectorcraftApp, tgt: &Target) -> impl std::hash::Hash + use<> {
    let o = Owner::of(app, tgt, Color::rgb8(0, 0, 0));
    let part = app.session.active().and_then(|d| d.selection.symbol_part.as_ref().map(|p| (p.instance, p.path.clone())));
    ("color-wheel-hex", o.doc, o.selection, part, o.text_selection, o.appearance_item, o.fill, o.slot)
}

/// The Show Hex Field row for `tgt`: a `#` and a field as wide as the row, showing `shown`. A
/// committed text goes through `color.parse` as typed; a text it refuses shows its error in the
/// status bar and changes nothing. The text's alpha is not applied when `opacity_set` (the opacity
/// row set the opacity in this frame), so a press on that row that commits the text keeps the
/// opacity it set. True when a text was committed.
fn hex_row(app: &mut VectorcraftApp, ui: &mut Ui, tgt: &Target, shown: &str, opacity_set: bool) -> bool {
    let mut typed = None;
    let id = hex_field_id(app, tgt);
    ui.horizontal(|ui| {
        let t = Tokens::get(ui.ctx());
        ui.label(egui::RichText::new("#").size(15.0).color(t.text));
        let width = ui.available_width();
        typed = widgets::exact_text_field_named(ui, id, Some(tl!("Hex or CSS color")), shown, width);
    });
    let Some(text) = typed else { return false };
    match app.run("color.parse", json!({ "text": text })) {
        Ok(r) => {
            let Ok(rgb) = serde_json::from_value::<Color>(r["color"].clone()) else { return true };
            // The target is read again, so a text committed when a press on the opacity row takes
            // the focus keeps the opacity that press set.
            let tgt = &wheel_target(app, ui);
            // The document's RGB numbers, in the model a pick would store them in.
            let cmyk_doc = app.session.active().is_some_and(|d| d.doc.color_mode == ColorMode::Cmyk);
            let c = stored_rgb(&cms::active(), rgb, replaced(app, tgt, false).as_ref(), cmyk_doc);
            let alpha = r["opacity"].as_f64().map(|o| (o / 100.0) as f32).filter(|_| !opacity_set);
            apply_typed(app, tgt, c, alpha);
        }
        Err(e) => app.status(e),
    }
    true
}

/// Panel state: the HSB picked last, its [`Owner`] and the display sRGB of the color it stored.
const MEMORY: &str = "color-wheel-hsb";
/// Panel state: the OKLCH the slider rows asked for last, the [`Owner`] of the color they stored
/// and its display sRGB.
const OKLCH_MEMORY: &str = "color-wheel-oklch";
/// Panel state: the slider rows whose drag previews on an open interaction, until it is released.
const ROWS_LIVE: &str = "color-wheel-rows-live";
/// The message for OKLCH Sliders while the active document's RGB space is not sRGB.
const OKLCH_NEEDS_SRGB: &str = "OKLCH needs sRGB as the working RGB space";
/// Panel state: the HSL the slider rows asked for last, the [`Owner`] of the color they stored and
/// its display sRGB.
const HSL_MEMORY: &str = "color-wheel-hsl";
/// The height of the color vision chips and the space between them.
const VISION_CHIP: f32 = 16.0;
const VISION_GAP: f32 = 3.0;

/// The display sRGB of `c` as simulated for protanopia, deuteranopia and tritanopia, through
/// `cms`'s Proof Setup simulations, and for achromatopsia: the gray with `c`'s CIE L*, through the
/// same display transform.
pub(crate) fn vision_colors(cms: &Cms, c: &Color) -> [[f32; 3]; 4] {
    let srgb = cms.display_rgb(c);
    let proof = |target| cms.proof_srgb(srgb, &ProofSetup { target, ..Default::default() });
    let gray = cms.display_rgb(&Color::lab(cms.lab(c).l, 0.0, 0.0));
    [proof(ProofTarget::Protanopia), proof(ProofTarget::Deuteranopia), proof(ProofTarget::Tritanopia), gray]
}

/// The names of the color vision chips, in the order of [`vision_colors`].
fn vision_names() -> [String; 4] {
    [tl!("Protanopia"), tl!("Deuteranopia"), tl!("Tritanopia"), tl!("Achromatopsia")].map(|s| s.to_string())
}

/// The 0 to 255 values of display sRGB `rgb`.
pub(crate) fn bytes(rgb: [f32; 3]) -> [u8; 3] {
    rgb.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// The hex text of 0 to 255 values, in the form of the Color panel's hex field (`E67828`).
pub(crate) fn hex([r, g, b]: [u8; 3]) -> String {
    format!("{r:02X}{g:02X}{b:02X}")
}

/// The Show Color Vision Preview row: four chips across the panel with `color` as each color
/// vision deficiency shows it. Each chip takes the keyboard focus and reports its name and, with a
/// solid color, its hex value as its value to screen readers; its tooltip shows both. A click or a
/// key on a chip does nothing. With no solid color the chips are empty and keep their names in the tooltip and for screen
/// readers.
fn vision_row(ui: &mut Ui, cms: &Cms, color: Option<&Color>) {
    let t = Tokens::get(ui.ctx());
    let colors = color.map(|c| vision_colors(cms, c));
    let width = ui.available_width();
    let (row, _) = ui.allocate_exact_size(vec2(width, VISION_CHIP), Sense::hover());
    let w = ((width - 3.0 * VISION_GAP) / 4.0).max(1.0);
    let enabled = ui.is_enabled();
    for (i, name) in vision_names().into_iter().enumerate() {
        let cell = Rect::from_min_size(row.min + vec2(i as f32 * (w + VISION_GAP), 0.0), vec2(w, VISION_CHIP));
        let resp = ui.interact(cell, ui.scope_id().with(("color-vision", i)), Sense::focusable_noninteractive());
        let value = colors.map(|cs| {
            let [r, g, b] = bytes(cs[i]);
            ui.painter().rect_filled(cell, 0.0, egui::Color32::from_rgb(r, g, b));
            hex([r, g, b])
        });
        let stroke = if resp.has_focus() { ui.visuals().selection.stroke } else { Stroke::new(1.0, t.border) };
        ui.painter().rect_stroke(cell, 0.0, stroke, StrokeKind::Inside);
        let tip = match &value {
            Some(v) => format!("{name}\n{v}"),
            None => name.clone(),
        };
        resp.widget_info(|| egui::WidgetInfo { current_text_value: value.clone(), ..egui::WidgetInfo::labeled(egui::Role::Image, enabled, &name) });
        resp.on_hover_text(tip);
    }
}

/// What the HSB picked last belongs to: the document, the selected objects or text characters,
/// the appearance item, the proxy, the gradient stop or freeform point, its paint, and the color
/// the pick stored. The wheel shows that HSB only while all of them are unchanged.
#[derive(Clone, Debug, PartialEq)]
struct Owner {
    doc: Option<u64>,
    selection: Vec<NodeId>,
    /// The Type tool's selected characters, when it edits the paint.
    text_selection: Option<(NodeId, usize, usize)>,
    /// The Appearance panel's active fill or stroke row.
    appearance_item: Option<usize>,
    fill: bool,
    /// The selected gradient stop (`Some((false, i))`) or freeform point (`Some((true, i))`).
    slot: Option<(bool, usize)>,
    /// The gradient behind a selected stop or point, including its other colors.
    gradient: Option<Paint>,
    color: Color,
}

impl Owner {
    fn of(app: &VectorcraftApp, tgt: &Target, color: Color) -> Self {
        let st = app.session.active();
        let slot = match tgt {
            Target::Paint(..) => None,
            Target::Stop { index, .. } => Some((false, *index)),
            Target::Point { index, .. } => Some((true, *index)),
        };
        Self {
            doc: st.map(|d| d.uid),
            selection: st.map(|d| d.selection.objects.clone()).unwrap_or_default(),
            text_selection: super::character::text_editing(app),
            appearance_item: app.session.appearance_item(),
            fill: app.session.fill_active,
            slot,
            gradient: slot.map(|_| super::active_paint(app)),
            color,
        }
    }
}

/// The components remembered under `key` for the target's `color`: the display sRGB the color had
/// when they were stored, and the components. A record whose owner or display color differs from
/// what the panel shows now is dropped, so that it does not come back when the same color returns
/// later.
fn recall<T: Clone + Send + Sync + 'static>(
    app: &VectorcraftApp,
    ui: &Ui,
    key: &str,
    tgt: &Target,
    color: Option<Color>,
    cms: &Cms,
) -> Option<([f32; 3], T)> {
    let (owner, shown, v) = pstate::<Option<(Owner, [f32; 3], T)>>(ui.ctx(), key)?;
    match color {
        Some(c) if owner == Owner::of(app, tgt, c) && cms.display_rgb(&c) == shown => Some((shown, v)),
        _ => {
            set_pstate(ui.ctx(), key, None::<(Owner, [f32; 3], T)>);
            None
        }
    }
}

/// The HSB a color shows on the wheel: that of its display sRGB through `cms`.
pub(crate) fn display_hsb(cms: &Cms, c: &Color) -> [f32; 3] {
    rgb_to_hsb(cms.display_rgb(c))
}

/// The HSB the wheel shows for `c` through `cms` when `hsb` was picked last and `c` then displayed
/// as the sRGB `shown`: `hsb` while `c` still displays as `shown`, else `c`'s own display HSB (after
/// a change of the color settings or the document's profile).
pub(crate) fn remembered_hsb(cms: &Cms, c: &Color, shown: [f32; 3], hsb: [f32; 3]) -> [f32; 3] {
    if cms.display_rgb(c) == shown { hsb } else { display_hsb(cms, c) }
}

/// The color a pick of display HSB `hsb` stores in place of `was`, through `cms`. The picked sRGB
/// goes into the working RGB space. An RGB color stays RGB, and a CMYK or Lab color converts to its
/// model with the color settings. Anything else (no color, a Gray color, a tint of a global swatch)
/// becomes a process color in the document's color mode: managed CMYK when `cmyk_doc`, else RGB.
pub(crate) fn stored(cms: &Cms, hsb: [f32; 3], was: Option<&Color>, cmyk_doc: bool) -> Color {
    stored_srgb(cms, hsb_to_rgb(hsb), was, cmyk_doc)
}

/// [`stored`] for a pick of display sRGB `srgb`.
pub(crate) fn stored_srgb(cms: &Cms, srgb: [f32; 3], was: Option<&Color>, cmyk_doc: bool) -> Color {
    let [r, g, b] = cms.srgb_to_rgb(srgb);
    stored_rgb(cms, Color::rgb(r, g, b), was, cmyk_doc)
}

/// [`stored`] for the RGB color `rgb` in the working RGB space.
fn stored_rgb(cms: &Cms, rgb: Color, was: Option<&Color>, cmyk_doc: bool) -> Color {
    let cmyk = || {
        let [c, m, y, k] = cms.to_cmyk(&rgb, cms.settings().intent);
        Color::cmyk(c, m, y, k)
    };
    match was {
        Some(Color::Rgb { .. }) => rgb,
        Some(Color::Cmyk { .. }) => cmyk(),
        Some(Color::Lab { .. }) => {
            let l = cms.lab(&rgb);
            Color::lab(l.l, l.a, l.b)
        }
        _ if cmyk_doc => cmyk(),
        _ => rgb,
    }
}

/// The color a pick replaces, which sets the model of the new one: the target's, or with Alt
/// (`behind`) the inactive proxy's solid color. None for a tint of a global swatch.
fn replaced(app: &VectorcraftApp, tgt: &Target, behind: bool) -> Option<Color> {
    if !behind {
        return tgt.color().filter(|_| tgt.tint().is_none());
    }
    let (fill, stroke) = super::current_paints(app);
    match if app.session.fill_active { stroke } else { fill } {
        Paint::Solid { color, swatch, .. } => {
            let global = swatch.is_some_and(|n| app.session.active().is_some_and(|d| d.doc.global_color(&n).is_some()));
            (!global).then_some(color)
        }
        _ => None,
    }
}

/// The OKLCH `[l, c, h°]` the rows show for `c` through `cms` when `lch` was asked for last and `c`
/// then displayed as the sRGB `shown`: `lch` while `c` still displays as `shown` (a chroma past the
/// gamut and the hue of a gray stay), else `c`'s own.
pub(crate) fn remembered_oklch(cms: &Cms, c: &Color, shown: [f32; 3], lch: [f64; 3]) -> [f64; 3] {
    if cms.display_rgb(c) == shown { lch } else { oklab::to_oklch(clipped_rgb(cms, c)) }
}

/// The display sRGB of `c` through `cms` clipped to sRGB: the color the slider rows show and edit
/// for a color outside sRGB.
fn clipped_rgb(cms: &Cms, c: &Color) -> [f32; 3] {
    cms.display_rgb(c).map(|v| v.clamp(0.0, 1.0))
}

/// The color a track shows at OKLCH `lch`: the color itself inside sRGB, and outside it dimmed.
/// A color outside sRGB shows at the largest chroma that fits at its L and H, the color a pick
/// there stores. With `max_chroma` (the C track, whose L and H stay fixed) that chroma is given
/// for the colors past it; without it, or in a gap below it where the ray leaves sRGB and comes
/// back, it is found for each color.
pub(crate) fn oklch_track(lch: [f64; 3], max_chroma: Option<f64>) -> Color32 {
    let exact = oklab::oklch_to_srgb(lch[0], lch[1], lch[2]);
    let (rgb, outside) = match max_chroma {
        Some(m) if lch[1] > m => (oklab::oklch_to_srgb(lch[0], m, lch[2]), true),
        Some(_) if oklab::in_srgb(exact) => (exact, false),
        _ => {
            let (rgb, lowered) = oklab::from_oklch(lch[0], lch[1], lch[2]);
            (rgb.map(f64::from), lowered)
        }
    };
    let [r, g, b] = rgb.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    let c = Color32::from_rgb(r, g, b);
    if outside { c.gamma_multiply(0.3) } else { c }
}

/// Where the C track's tick sits, from 0 at its left to 1 at its right: at the largest chroma
/// `max_chroma`, of the track's 0 to 0.4.
pub(crate) fn chroma_tick(max_chroma: f64) -> f32 {
    (max_chroma / CHROMA_FULL).clamp(0.0, 1.0) as f32
}

/// The OKLCH rows' edits of one frame with row `i`'s edit added: row `i` takes its value from
/// `asked`, on top of `out` (the other rows' edits so far, or none on top of `lch`). A drag on one
/// row stays in progress when another row's field commits in the same frame.
pub(crate) fn oklch_row_edit(
    out: Option<([f64; 3], [bool; 3], Live)>,
    lch: [f64; 3],
    asked: [f64; 3],
    i: usize,
    phase: Live,
) -> ([f64; 3], [bool; 3], Live) {
    let (mut base, mut rows, was) = out.unwrap_or((lch, [false; 3], phase));
    base[i] = asked[i];
    rows[i] = true;
    let phase = if matches!(was, Live::Dragging) { Live::Dragging } else { phase };
    (base, rows, phase)
}

/// The L, C and H rows of OKLCH Sliders for the target's `color` through `cms`: lightness from 0 to
/// 100 %, chroma from 0 to 100 % of 0.4 (as CSS reads a chroma percentage) and hue from 0 to 360°.
/// Each track shows the colors its slider reaches and dims the part outside sRGB, and a tick on the
/// C track marks the largest chroma sRGB holds at the row's L and H. The rows are gray with no
/// color to edit and while the working RGB space is not sRGB. Returns the OKLCH a drag, an arrow
/// key, a screen reader or a field asks for, which rows set it (L, C and H in that order) and the
/// phase of the last edit. A field that commits when a click on another row takes its focus keeps
/// its value along with the click's.
pub(crate) fn oklch_rows(app: &VectorcraftApp, ui: &mut Ui, tgt: &Target, color: Option<Color>, cms: &Cms) -> Option<([f64; 3], [bool; 3], Live)> {
    let t = Tokens::get(ui.ctx());
    let enabled = color.is_some() && cms.rgb_is_srgb();
    let lch = match (color, recall::<[f64; 3]>(app, ui, OKLCH_MEMORY, tgt, color, cms)) {
        (Some(c), Some((shown, lch))) => remembered_oklch(cms, &c, shown, lch),
        (Some(c), _) => oklab::to_oklch(clipped_rgb(cms, &c)),
        (None, _) => [0.0; 3],
    };
    // Each row: its letter, its screen-reader name, the end of its range and the field's suffix.
    let rows = [("L", tl!("Lightness"), 100.0, "%"), ("C", tl!("Chroma"), 100.0, "%"), ("H", tl!("Hue"), 360.0, "°")];
    let shown = [lch[0] * 100.0, lch[1] / CHROMA_FULL * 100.0, lch[2]];
    // The OKLCH with row `i` set to `v` in the row's units.
    let with = |i: usize, v: f64| {
        let mut out = lch;
        match i {
            0 => out[0] = (v / 100.0).clamp(0.0, 1.0),
            1 => out[1] = (v / 100.0 * CHROMA_FULL).clamp(0.0, CHROMA_FULL),
            _ => out[2] = v.clamp(0.0, 360.0),
        }
        out
    };
    let mut out: Option<([f64; 3], [bool; 3], Live)> = None;
    let mut set = |asked: [f64; 3], i: usize, phase: Live| out = Some(oklch_row_edit(out, lch, asked, i, phase));
    // The L and H tracks search the gamut for each color outside sRGB, so their colors are kept
    // until the other two components change.
    let mut tracks = pstate::<[Option<([u64; 3], Vec<Color32>)>; 3]>(ui.ctx(), OKLCH_TRACKS);
    ui.add_enabled_ui(enabled, |ui| {
        let field_w = 48.0;
        let slider_w = (ui.available_width() - field_w - 12.0 - 3.0 * ui.spacing().item_spacing.x).max(60.0);
        // The C track's L and H stay fixed, so its largest chroma is found once per frame.
        let max_c = if enabled { oklab::max_chroma(lch[0], lch[2]) } else { 0.0 };
        for (i, (letter, name, max, suffix)) in rows.into_iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_sized(vec2(12.0, 22.0), egui::Label::new(egui::RichText::new(letter).size(12.5).color(t.text)));
                let v = shown[i].clamp(0.0, max);
                let at = |x: f32| with(i, f64::from(x) * max);
                let mut key = lch.map(f64::to_bits);
                key[i] = 0;
                let colors = match &tracks[i] {
                    Some((k, colors)) if i != 1 && enabled && *k == key => colors.clone(),
                    _ if i != 1 && enabled => {
                        let last = (TRACK_SAMPLES - 1) as f32;
                        let colors: Vec<Color32> = (0..TRACK_SAMPLES).map(|s| oklch_track(at(s as f32 / last), None)).collect();
                        tracks[i] = Some((key, colors.clone()));
                        colors
                    }
                    _ => Vec::new(),
                };
                let track = |x: f32| match colors.get((x.clamp(0.0, 1.0) * (TRACK_SAMPLES - 1) as f32).round() as usize) {
                    Some(c) => *c,
                    None if enabled => oklch_track(at(x), (i == 1).then_some(max_c)),
                    None => t.input,
                };
                let value = SliderValue::field(v, 0.0..=max, suffix, 0);
                let (dragged, phase, keyed) =
                    widgets::color_slider_stepped(ui, ("color-wheel-oklch", i), name, Some(value), (v / max) as f32, slider_w, &track);
                // The tick goes on the bar of the rail as the row laid it out.
                let rail = (i == 1 && enabled).then(|| ui.ctx().read_response(widgets::id_with(ui.scope_id(), ("color-wheel-oklch", i)))).flatten();
                if let Some(rail) = rail {
                    let bar = widgets::color_rail_bar(rail.rect);
                    let x = bar.left() + chroma_tick(max_c) * bar.width();
                    ui.painter().line_segment([pos2(x, bar.top() - 3.0), pos2(x, bar.bottom())], Stroke::new(1.5, t.text));
                    let tip = Rect::from_x_y_ranges(x - 3.0..=x + 3.0, bar.top() - 3.0..=bar.bottom());
                    ui.interact(tip, ui.scope_id().with("color-wheel-oklch-tick"), egui::Sense::hover())
                        .on_hover_text(tl!("The largest chroma sRGB holds at this lightness and hue"));
                }
                if let Some(x) = dragged {
                    set(at(x), i, phase);
                }
                if let Some(k) = keyed {
                    set(with(i, k), i, Live::Released);
                }
                if let Some(f) = widgets::plain_field_named(ui, ("color-wheel-oklch-field", i), Some(name), v, suffix, 0, field_w) {
                    set(with(i, f), i, Live::Released);
                }
            });
        }
    });
    set_pstate(ui.ctx(), OKLCH_TRACKS, tracks);
    out.filter(|_| enabled)
}

/// Panel state: the colors of the L and H tracks of OKLCH Sliders, each with the OKLCH (its own
/// component left out) they were drawn for.
const OKLCH_TRACKS: &str = "color-wheel-oklch-tracks";
/// How many colors along a cached track.
const TRACK_SAMPLES: usize = 128;

/// The HSL `[h°, s, l]` the rows show for `c` through `cms` when `hsl` was asked for last and `c`
/// then displayed as the sRGB `shown`: `hsl` while `c` still displays as `shown` (the hue of a gray
/// and the saturation of black and white stay), else `c`'s own.
pub(crate) fn remembered_hsl(cms: &Cms, c: &Color, shown: [f32; 3], hsl: [f32; 3]) -> [f32; 3] {
    if cms.display_rgb(c) == shown { hsl } else { rgb_to_hsl(clipped_rgb(cms, c)) }
}

/// The color a track shows at HSL `hsl`.
fn hsl32(hsl: [f32; 3]) -> Color32 {
    let [r, g, b] = hsl_to_rgb(hsl).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    Color32::from_rgb(r, g, b)
}

/// The H, S and L rows of HSL Sliders for the target's `color` through `cms`: hue from 0 to 360°,
/// saturation and lightness from 0 to 100 %, CSS Color 4's HSL of the display sRGB. Each track
/// shows the colors its slider reaches. The rows are gray with no color to edit. Returns the HSL a
/// drag, an arrow key, a screen reader or a field asks for, which rows set it (H, S and L in that
/// order) and the phase of the last edit. A drag on one row stays in progress when another row's
/// field commits in the same frame.
pub(crate) fn hsl_rows(app: &VectorcraftApp, ui: &mut Ui, tgt: &Target, color: Option<Color>, cms: &Cms) -> Option<([f32; 3], [bool; 3], Live)> {
    let t = Tokens::get(ui.ctx());
    let enabled = color.is_some();
    let hsl = match (color, recall::<[f32; 3]>(app, ui, HSL_MEMORY, tgt, color, cms)) {
        (Some(c), Some((shown, hsl))) => remembered_hsl(cms, &c, shown, hsl),
        (Some(c), _) => rgb_to_hsl(clipped_rgb(cms, &c)),
        (None, _) => [0.0; 3],
    };
    // Each row: its letter, its screen-reader name, the end of its range and the field's suffix. The
    // names start with "HSL" so that they differ from the box strip's HSB Hue and Saturation.
    let hsl_name = |name: &str| format!("HSL {name}");
    let rows =
        [("H", hsl_name(tl!("Hue")), 360.0, "°"), ("S", hsl_name(tl!("Saturation")), 100.0, "%"), ("L", hsl_name(tl!("Lightness")), 100.0, "%")];
    let shown = [f64::from(hsl[0]), f64::from(hsl[1]) * 100.0, f64::from(hsl[2]) * 100.0];
    // The HSL with row `i` set to `v` in the row's units.
    let with = |i: usize, v: f64| {
        let mut out = hsl;
        match i {
            0 => out[0] = v.clamp(0.0, 360.0) as f32,
            1 => out[1] = (v / 100.0).clamp(0.0, 1.0) as f32,
            _ => out[2] = (v / 100.0).clamp(0.0, 1.0) as f32,
        }
        out
    };
    let mut out: Option<([f32; 3], [bool; 3], Live)> = None;
    let mut set = |asked: [f32; 3], i: usize, phase: Live| {
        let (mut base, mut edited, was) = out.unwrap_or((hsl, [false; 3], phase));
        base[i] = asked[i];
        edited[i] = true;
        let phase = if matches!(was, Live::Dragging) { Live::Dragging } else { phase };
        out = Some((base, edited, phase));
    };
    ui.add_enabled_ui(enabled, |ui| {
        let field_w = 48.0;
        let slider_w = (ui.available_width() - field_w - 12.0 - 3.0 * ui.spacing().item_spacing.x).max(60.0);
        for (i, (letter, name, max, suffix)) in rows.into_iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_sized(vec2(12.0, 22.0), egui::Label::new(egui::RichText::new(letter).size(12.5).color(t.text)));
                let v = shown[i].clamp(0.0, max);
                let at = |x: f32| with(i, f64::from(x) * max);
                let track = |x: f32| if enabled { hsl32(at(x)) } else { t.input };
                let value = SliderValue::field(v, 0.0..=max, suffix, 0);
                let (dragged, phase, keyed) =
                    widgets::color_slider_stepped(ui, ("color-wheel-hsl", i), &name, Some(value), (v / max) as f32, slider_w, &track);
                if let Some(x) = dragged {
                    set(at(x), i, phase);
                }
                if let Some(k) = keyed {
                    set(with(i, k), i, Live::Released);
                }
                if let Some(f) = widgets::plain_field_named(ui, ("color-wheel-hsl-field", i), Some(&name), v, suffix, 0, field_w) {
                    set(with(i, f), i, Live::Released);
                }
            });
        }
    });
    out.filter(|_| enabled)
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let tgt = wheel_target(app, ui);
    let color = tgt.color();
    let cms = cms::active();
    // With None, a pattern or a "?" proxy the wheel shows the last solid color without markers,
    // and a click, an arrow key or a screen reader's action applies the picked color.
    let hsb = match (color, recall::<[f32; 3]>(app, ui, MEMORY, &tgt, color, &cms)) {
        (Some(c), Some((shown, hsb))) => remembered_hsb(&cms, &c, shown, hsb),
        (c, _) => display_hsb(&cms, &c.unwrap_or(app.session.last_solid)),
    };
    let marked = color.is_some();
    let mut pick: Option<(Picked, Live)> = None;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(44.0);
            super::proxy(app, ui, 40.0);
        });
        let size = ui.available_width().min(MAX_WHEEL);
        if app.ui.color_wheel.view == View::Box {
            pick = show_box(ui, app.ui.color_wheel.channel, hsb, marked, size);
            return;
        }
        let ring = widgets::hue_ring(ui, "color-wheel-ring", tl!("Hue ring"), size, hsb[0], marked);
        let square = widgets::ring_square(ring.rect);
        let mut field_ui = ui.new_child(UiBuilder::new().id_salt("color-wheel-square").max_rect(square));
        let axes =
            [FieldAxis { name: tl!("Saturation"), units: 100.0, suffix: "%" }, FieldAxis { name: tl!("Brightness"), units: 100.0, suffix: "%" }];
        let at = |x: f32, y: f32| widgets::hsb32([hsb[0], x, 1.0 - y]);
        let name = tl!("Saturation and brightness");
        let (field, field_phase) = widgets::color_field(&mut field_ui, "field", (name, axes), square.size(), (hsb[1], 1.0 - hsb[2]), marked, &at);
        // Show Harmony: the rule's other colors for the current one, which follow it. While a press
        // is held on a marker, the ring's drag under it picks nothing.
        let set = &app.ui.color_wheel;
        let mut held = false;
        if set.harmony && marked {
            let others: Vec<[f32; 3]> = set.harmony_rule.apply_hsb(hsb).into_iter().skip(1).collect();
            let markers = widgets::ring_markers(ui, "color-wheel-harmony", ring.rect, &others, tl!("Harmony color"));
            held = markers.held;
            if let Some(i) = markers.picked
                && let Some(&c) = others.get(i)
            {
                pick = Some((Picked::Whole(c), Live::Released));
            }
        }
        if let Some((h, phase)) = ring.hue
            && !held
            && pick.is_none()
        {
            pick = Some((Picked::Hue(h), phase));
        }
        // A press between the square and the ring picks the closest point of the square, which
        // takes the keyboard focus.
        if let Some((p, phase)) = ring.inner {
            let field_id = widgets::id_with(field_ui.scope_id(), "field");
            field_ui.memory_mut(|m| m.request_focus(field_id));
            let f = |v: f32, lo: f32, len: f32| ((v - lo) / len).clamp(0.0, 1.0);
            pick = Some((Picked::Square(f(p.x, square.left(), square.width()), 1.0 - f(p.y, square.top(), square.height())), phase));
        }
        if let Some((x, y)) = field {
            pick = Some((Picked::Square(x, 1.0 - y), field_phase));
        }
    });
    let (mut picked_lch, mut picked_hsl) = (None, None);
    match app.ui.color_wheel.sliders {
        Sliders::None => {}
        Sliders::Oklch => {
            ui.add_space(4.0);
            picked_lch = oklch_rows(app, ui, &tgt, color, &cms);
        }
        Sliders::Hsl => {
            ui.add_space(4.0);
            picked_hsl = hsl_rows(app, ui, &tgt, color, &cms);
        }
    }
    // A row drag whose rows were hidden or replaced (`ui.colorWheel {sliders}`), or turned gray
    // (OKLCH rows after the working RGB space stops being sRGB), reports no release: its preview is
    // kept as one undo step and the interaction ends.
    let live = pstate::<Option<Sliders>>(ui.ctx(), ROWS_LIVE);
    if live.is_some_and(|rows| rows != app.ui.color_wheel.sliders || (rows == Sliders::Oklch && !cms.rgb_is_srgb())) {
        if app.session.in_interaction()
            && let Err(e) = app.session.commit_interaction()
        {
            app.status(e.to_string());
        }
        app.sync_views();
        set_pstate(ui.ctx(), ROWS_LIVE, None::<Sliders>);
    }
    if app.ui.color_wheel.vision {
        ui.add_space(4.0);
        vision_row(ui, &cms, color.as_ref());
    }
    // The hex field compares its text with what it showed before an opacity edit of this frame, so
    // a text committed when a press on the opacity row takes the focus is not dropped.
    let hex = app.ui.color_wheel.hex_field.then(|| hex_shown(app, &tgt, color.as_ref()));
    let mut opacity_set = false;
    if app.ui.color_wheel.opacity {
        let shown = color.unwrap_or(app.session.last_solid);
        let owner = color.map(|c| Owner::of(app, &tgt, c));
        opacity_set = opacity_row(app, ui, &tgt, &shown);
        // An opacity edit of a gradient stop or point changes the gradient its owner holds, and the
        // HSB, the OKLCH and the HSL picked last stay with the new gradient.
        if let (Some(c), Some(was)) = (color, owner) {
            let now = Owner::of(app, &tgt, c);
            if now != was {
                if let Some((remembered, rgb, hsb)) = pstate::<Option<(Owner, [f32; 3], [f32; 3])>>(ui.ctx(), MEMORY)
                    && remembered == was
                {
                    set_pstate(ui.ctx(), MEMORY, Some((now.clone(), rgb, hsb)));
                }
                if let Some((remembered, rgb, lch)) = pstate::<Option<(Owner, [f32; 3], [f64; 3])>>(ui.ctx(), OKLCH_MEMORY)
                    && remembered == was
                {
                    set_pstate(ui.ctx(), OKLCH_MEMORY, Some((now.clone(), rgb, lch)));
                }
                if let Some((remembered, rgb, hsl)) = pstate::<Option<(Owner, [f32; 3], [f32; 3])>>(ui.ctx(), HSL_MEMORY)
                    && remembered == was
                {
                    set_pstate(ui.ctx(), HSL_MEMORY, Some((now, rgb, hsl)));
                }
            }
        }
    }
    let mut committed = false;
    if let Some(hex) = hex {
        ui.add_space(4.0);
        committed = hex_row(app, ui, &tgt, &hex, opacity_set);
    }
    // A pick or a slider row edit read in the frame of an opacity edit or of a text commit goes to
    // the target as that edit or text left it, so a stop keeps both. After a text commit that
    // changed the color, the components the pick or the row does not set come from the color the
    // text applied; a text that left the color as it was keeps the remembered hue.
    let tgt = if opacity_set || committed { wheel_target(app, ui) } else { tgt };
    let now = tgt.color().filter(|c| committed && Some(*c) != color);
    let behind = super::alt_held(ui);
    let cmyk_doc = app.session.active().is_some_and(|d| d.doc.color_mode == ColorMode::Cmyk);
    let rows = match (picked_hsl, picked_lch) {
        (Some((asked, edited, phase)), _) => {
            let hsl = match now {
                Some(c) => {
                    let mut hsl = rgb_to_hsl(clipped_rgb(&cms, &c));
                    for row in (0..3).filter(|r| edited[*r]) {
                        hsl[row] = asked[row];
                    }
                    hsl
                }
                None => asked,
            };
            Some((hsl_to_rgb(hsl), phase, HSL_MEMORY, Remembered::Hsl(hsl)))
        }
        (_, Some((asked, edited, phase))) => {
            let lch = match now {
                Some(c) => {
                    let mut lch = oklab::to_oklch(clipped_rgb(&cms, &c));
                    for row in (0..3).filter(|r| edited[*r]) {
                        lch[row] = asked[row];
                    }
                    lch
                }
                None => asked,
            };
            Some((oklab::from_oklch(lch[0], lch[1], lch[2]).0, phase, OKLCH_MEMORY, Remembered::Oklch(lch)))
        }
        _ => None,
    };
    if let Some((srgb, phase, key, asked)) = rows {
        // A hue on a gray, or a saturation on black or white, leaves the displayed color as it is:
        // the rows keep what they were given, and the paint (a Gray color too) stays unchanged.
        let current = color.filter(|c| {
            let d = cms.display_rgb(c);
            !behind && live.is_none() && now.is_none() && (0..3).all(|i| (d[i] - srgb[i]).abs() <= 1e-5)
        });
        if let Some(c) = current {
            asked.remember(ui, key, Owner::of(app, &tgt, c), cms.display_rgb(&c));
            return;
        }
        let c = stored_srgb(&cms, srgb, replaced(app, &tgt, behind).as_ref(), cmyk_doc);
        apply(app, &tgt, c, phase, behind);
        set_pstate(ui.ctx(), ROWS_LIVE, (phase == Live::Dragging).then_some(app.ui.color_wheel.sliders));
        if !behind {
            asked.remember(ui, key, Owner::of(app, &tgt, c), cms.display_rgb(&c));
        }
        return;
    }
    let Some((picked, phase)) = pick else { return };
    let hsb = picked.hsb(now.map_or(hsb, |c| display_hsb(&cms, &c)));
    let c = stored(&cms, hsb, replaced(app, &tgt, behind).as_ref(), cmyk_doc);
    apply(app, &tgt, c, phase, behind);
    if !behind {
        set_pstate(ui.ctx(), MEMORY, Some((Owner::of(app, &tgt, c), cms.display_rgb(&c), hsb)));
    }
}

/// What a slider row asked for, kept under its memory key.
enum Remembered {
    Hsl([f32; 3]),
    Oklch([f64; 3]),
}

impl Remembered {
    fn remember(self, ui: &Ui, key: &str, owner: Owner, shown: [f32; 3]) {
        match self {
            Remembered::Hsl(v) => set_pstate(ui.ctx(), key, Some((owner, shown, v))),
            Remembered::Oklch(v) => set_pstate(ui.ctx(), key, Some((owner, shown, v))),
        }
    }
}
