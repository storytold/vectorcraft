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
//! Picker's axes. The view and the channel are kept with the preferences (`ui.colorWheel`).
//!
//! Panel menu › Show Contrast adds a row under the wheel or the box with the WCAG 2 contrast ratio
//! (<https://www.w3.org/TR/WCAG22/#dfn-contrast-ratio>) of the color against the color drawn
//! behind the selection, or against a color pinned with Contrast Background › Current Color, and
//! whether it meets AA and AAA for the category picked under Contrast For (`color.contrast`).

use egui::{Ui, UiBuilder, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_color::cms::{self, Cms};
use vectorcraft_color::wcag::{self, Category};
use vectorcraft_color::{Color, Paint, hsb_to_rgb, rgb_to_hsb};
use vectorcraft_doc::{ColorMode, NodeId};

use super::color::{Target, apply, wheel_target};
use super::{color_json, pstate, set_pstate};
use crate::VectorcraftApp;
use crate::dialogs::color_picker::Channel;
use crate::widgets::{self, FieldAxis, Live, SliderValue, menu_item};

pub const ID: &str = "colorWheel";
/// The widest the wheel is drawn.
const MAX_WHEEL: f32 = 200.0;
/// The width of the box's channel strip, its arrows included.
const STRIP: f32 = 22.0;
/// The space between the box's field and its channel strip.
const STRIP_GAP: f32 = 6.0;

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
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    #[serde(deserialize_with = "deserialize_view")]
    pub view: View,
    #[serde(serialize_with = "serialize_channel", deserialize_with = "deserialize_channel")]
    pub channel: Channel,
    /// Show Contrast: the contrast row under the wheel or the box.
    #[serde(deserialize_with = "deserialize_flag")]
    pub contrast: bool,
    /// Contrast For: the category whose AA and AAA results the row shows.
    #[serde(serialize_with = "serialize_category", deserialize_with = "deserialize_category")]
    pub contrast_for: Category,
    /// Contrast Background › Current Color: the pinned background; None for the color behind the
    /// selection.
    #[serde(deserialize_with = "deserialize_background")]
    pub contrast_background: Option<Color>,
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
    Ok(<Value as Deserialize>::deserialize(d)?.as_bool().unwrap_or_default())
}

fn serialize_category<S: serde::Serializer>(c: &Category, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(c.id())
}

fn deserialize_category<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Category, D::Error> {
    let v = <Value as Deserialize>::deserialize(d)?;
    Ok(v.as_str().and_then(Category::parse).unwrap_or_default())
}

/// A pinned background as saved (`{model, …}`) or as `ui.colorWheel` takes it (any color the
/// paint commands take); anything else reads as no pin.
fn background_value(v: &Value) -> Option<Color> {
    serde_json::from_value(v.clone()).ok().or_else(|| vectorcraft_engine::cmd::color_value(v))
}

fn deserialize_background<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Color>, D::Error> {
    Ok(background_value(&<Value as Deserialize>::deserialize(d)?))
}

/// Serde reader for the saved settings ([`crate::UiState::color_wheel`]): a value that is not an
/// object reads as the defaults, so it doesn't lose the rest of the preferences.
pub fn deserialize_settings<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Settings, D::Error> {
    let v = <Value as Deserialize>::deserialize(d)?;
    Ok(serde_json::from_value(v).unwrap_or_default())
}

/// `ui.colorWheel {view?, channel?, contrast?, contrastFor?, contrastBackground?}`: show the wheel
/// or the box, pick the box's channel, and set the contrast row, its category and its pinned
/// background (`contrastBackground: null` unpins it). Keys left out keep their values; params that
/// are not an object, another key or a value off the list are an error and change nothing. → the
/// settings.
pub(crate) fn command(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    const KEYS: [&str; 5] = ["view", "channel", "contrast", "contrastFor", "contrastBackground"];
    match p {
        Value::Null => {}
        Value::Object(o) => {
            if let Some(key) = o.keys().find(|k| !KEYS.contains(&k.as_str())) {
                return Err(format!("unknown parameter `{key}`; the command takes: {}", KEYS.join(", ")));
            }
        }
        got => return Err(format!("the params must be an object, not {got}")),
    }
    let mut set = app.ui.color_wheel.clone();
    match p.get("view") {
        None | Some(Value::Null) => {}
        Some(v) => set.view = v.as_str().and_then(View::parse).ok_or("`view` must be wheel or box")?,
    }
    match p.get("channel") {
        None | Some(Value::Null) => {}
        Some(v) => set.channel = v.as_str().and_then(Channel::parse).ok_or("`channel` must be hue, saturation, brightness, red, green or blue")?,
    }
    match p.get("contrast") {
        None | Some(Value::Null) => {}
        Some(v) => {
            let key = "contrast";
            set.contrast = v.as_bool().ok_or_else(|| format!("`{key}` must be true or false"))?;
        }
    }
    match p.get("contrastFor") {
        None | Some(Value::Null) => {}
        Some(v) => {
            let key = "contrastFor";
            let ids: Vec<&str> = Category::ALL.iter().map(|c| c.id()).collect();
            set.contrast_for = v.as_str().and_then(Category::parse).ok_or_else(|| format!("`{key}` must be one of {}", ids.join(", ")))?;
        }
    }
    match p.get("contrastBackground") {
        None => {}
        Some(Value::Null) => set.contrast_background = None,
        Some(v) => {
            let key = "contrastBackground";
            set.contrast_background = Some(background_value(v).ok_or_else(|| format!("`{key}` must be a color or {}", "null"))?);
        }
    }
    app.ui.color_wheel = set;
    Ok(json!(app.ui.color_wheel))
}

/// A Contrast For category's menu label in the interface language.
pub(super) fn category_name(c: Category) -> &'static str {
    tl!(c.label())
}

/// The panel menu: Wheel View and Box View, Box Channel while the box shows, Show Contrast,
/// Contrast For and Contrast Background.
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
    if menu_item(ui, tl!("Show Contrast"), true, set.contrast) {
        command(app, &json!({ "contrast": !set.contrast })).ok();
    }
    ui.menu_button(format!("   {}", tl!("Contrast For")), |ui| {
        for c in Category::ALL {
            if menu_item(ui, category_name(c), true, set.contrast_for == c) {
                command(app, &json!({ "contrastFor": c.id() })).ok();
            }
        }
    });
    ui.menu_button(format!("   {}", tl!("Contrast Background")), |ui| {
        if menu_item(ui, tl!("Behind Selection"), true, set.contrast_background.is_none()) {
            command(app, &json!({ "contrastBackground": null })).ok();
        }
        let current = wheel_target(app, ui).color();
        if menu_item(ui, tl!("Current Color"), current.is_some(), set.contrast_background.is_some())
            && let Some(c) = current
        {
            command(app, &json!({ "contrastBackground": color_json(&c) })).ok();
        }
    });
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
/// markers when `marked`. Returns the HSB a click, a drag, a key or a screen reader's action picks,
/// and its phase.
fn show_box(ui: &mut Ui, ch: Channel, hsb: [f32; 3], marked: bool, size: f32) -> Option<([f32; 3], Live)> {
    let (z, x, y) = box_pos(ch, hsb);
    let mut pick = None;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = STRIP_GAP;
        let field = vec2((size - STRIP_GAP - STRIP).max(1.0), size);
        let at = |x: f32, y: f32| widgets::hsb32(box_pick(ch, z, x, y, hsb));
        if let (Some((x, y)), phase) = widgets::color_field(ui, "color-wheel-box", (tl!("Color box"), ch.field_axes()), field, (x, y), marked, &at) {
            pick = Some((box_pick(ch, z, x, y, hsb), phase));
        }
        // The hue strip shows the whole spectrum; the others change only their own component.
        let track = |z: f32| widgets::hsb32(if ch == Channel::Hue { [z * 360.0, 1.0, 1.0] } else { box_pick(ch, z, x, y, hsb) });
        let (v, suffix, max) = strip_number(ch, hsb);
        let shown = SliderValue::field(v, 0.0..=max, suffix, 0);
        match widgets::channel_slider_stepped(ui, "color-wheel-strip", ch.name(), Some(shown), z, marked, vec2(STRIP, size), &track) {
            (Some(z), phase, _) => pick = Some((box_pick(ch, z, x, y, hsb), phase)),
            (None, _, Some(v)) => pick = Some((box_pick(ch, (v / max) as f32, x, y, hsb), Live::Released)),
            _ => {}
        }
    });
    pick
}

/// Panel state: the HSB picked last, its [`Owner`] and the display sRGB of the color it stored.
const MEMORY: &str = "color-wheel-hsb";
/// Panel state: the color behind the selection, as `#rrggbb`, with the [`BehindKey`] it was read for.
const BEHIND: &str = "color-wheel-behind";
/// Panel state: the contrast row's `color.contrast` answer, with the [`RatioKey`] it was asked for.
const RATIO: &str = "color-wheel-ratio";
/// The side of the contrast row's background chip.
const CHIP: f32 = 16.0;

/// What the color behind the selection depends on: the document and its revision, the selected
/// objects, the picked object of a dynamic symbol instance, the on-screen Appearance of Black and
/// the color settings ([`under`]).
type BehindKey = (u64, u64, Vec<NodeId>, Option<Vec<u32>>, String, Under);
/// What the contrast row's answer depends on: the background, the active proxy's paint (with nothing
/// selected it is the default for new art, which no revision counts), the document, its revision,
/// the selection and its picked symbol object, the proxy, the appearance item, the selected
/// gradient stop and freeform point, the on-screen Appearance of Black and the color settings.
pub(super) type RatioKey =
    (String, String, Option<(u64, u64)>, Vec<NodeId>, Option<Vec<u32>>, bool, Option<usize>, Option<usize>, Option<usize>, String, Under);

/// The active color settings and profiles generation, which change how a color displays without
/// a document revision (Edit › Color Settings, a document's assigned profiles).
pub(super) type Under = (cms::ColorSettings, u64);

fn under() -> Under {
    (cms::active_settings(), cms::profiles_generation())
}

/// The path of the picked object of a dynamic symbol instance, if one is picked.
fn picked_part(st: &vectorcraft_engine::DocState) -> Option<Vec<u32>> {
    st.selection.symbol_part.as_ref().map(|sp| sp.path.clone())
}

/// The background the contrast row measures against: the pinned color, else the color
/// `color.contrast` reads behind the selection, kept per document revision and selection, and kept
/// as it is while an interaction is in progress (the selection being dragged is hidden from it).
/// None with nothing selected.
fn background(app: &mut VectorcraftApp, ui: &Ui) -> Option<Color> {
    if let Some(c) = app.ui.color_wheel.contrast_background {
        return Some(c);
    }
    let st = app.session.active()?;
    let key: BehindKey = (st.uid, st.revision, st.selection.objects.clone(), picked_part(st), app.session.prefs.black_on_screen.clone(), under());
    // During an interaction only the revision may differ: another document, selection or setting is
    // read again.
    let same_but_revision = |k: &BehindKey| k.0 == key.0 && k.2 == key.2 && k.3 == key.3 && k.4 == key.4 && k.5 == key.5;
    let hex = match pstate::<Option<(BehindKey, Option<String>)>>(ui.ctx(), BEHIND) {
        Some((k, hex)) if k == key || (app.session.in_interaction() && same_but_revision(&k)) => hex,
        _ => {
            // Any opaque color reads the background; only `against` is used.
            let r = app.session.execute("color.contrast", &json!({ "color": "#000000" })).ok();
            let hex = r.and_then(|r| r["against"].as_str().map(str::to_string));
            set_pstate(ui.ctx(), BEHIND, Some((key, hex.clone())));
            hex
        }
    };
    // `against` is written in working RGB, the form a color takes.
    Color::from_hex(&hex?)
}

/// The contrast row: the background's chip, the ratio cut off at two decimals and the AA and AAA
/// results for the chosen category; a dash when there is no background or no color to measure.
/// Without a pinned background the row asks `color.contrast` for the color behind the selection
/// (`against: "below"`), also during an interaction, so translucent groups and layers count as the
/// command counts them; the chip then shows the background in the answer, else the pinned color or
/// the one kept by [`background`].
fn contrast_row(app: &mut VectorcraftApp, ui: &mut Ui) {
    let category = app.ui.color_wheel.contrast_for;
    let pinned = app.ui.color_wheel.contrast_background.is_some();
    let against = background(app, ui);
    let sampled = !pinned;
    let answer = against.as_ref().and_then(|bg| {
        let st = app.session.active();
        let key: RatioKey = (
            if sampled { "below".to_string() } else { format!("{bg:?}") },
            format!("{:?}", super::active_paint(app)),
            st.map(|d| (d.uid, d.revision)),
            st.map(|d| d.selection.objects.clone()).unwrap_or_default(),
            st.and_then(picked_part),
            app.session.fill_active,
            app.session.appearance_item(),
            app.session.selected_stop(),
            app.session.selected_freeform_point(),
            app.session.prefs.black_on_screen.clone(),
            under(),
        );
        match pstate::<Option<(RatioKey, Option<Value>)>>(ui.ctx(), RATIO) {
            Some((k, r)) if k == key => r,
            _ => {
                let against = if sampled { json!("below") } else { color_json(bg) };
                let r = app.session.execute("color.contrast", &json!({ "against": against })).ok();
                set_pstate(ui.ctx(), RATIO, Some((key, r.clone())));
                r
            }
        }
    });
    let resp = ui
        .horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(CHIP, CHIP), egui::Sense::hover());
            // The chip shows the background the ratio was measured against.
            let measured = answer.as_ref().filter(|_| sampled).and_then(|r| r["against"].as_str()).and_then(Color::from_hex);
            match measured.or(against) {
                Some(c) => widgets::swatch_tile(ui, rect, &Paint::solid(c), false, false),
                None => widgets::swatch_tile(ui, rect, &Paint::None, false, false),
            }
            let Some(ratio) = answer.as_ref().and_then(|r| r["ratio"].as_f64()) else {
                ui.label("—");
                return;
            };
            ui.label(wcag::ratio_text(ratio));
            let (aa, aaa) = category.passes(ratio);
            let result = |pass: bool| if pass { tl!("Pass") } else { tl!("Fail") };
            ui.label(format!("AA {}", result(aa)));
            if let Some(aaa) = aaa {
                ui.label(format!("AAA {}", result(aaa)));
            }
        })
        .response;
    let mut tip = tl!(
        "WCAG 2.2 contrast minimums: normal text AA 4.5:1 (1.4.3) and AAA 7:1 (1.4.6), large text AA 3:1 and AAA 4.5:1, graphics and controls AA 3:1 (1.4.11)."
    )
    .to_string();
    if !pinned {
        tip.push(' ');
        tip.push_str(tl!("The background is read at the center of the selection."));
    }
    resp.on_hover_text(tip);
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
    let [r, g, b] = cms.srgb_to_rgb(hsb_to_rgb(hsb));
    let rgb = Color::rgb(r, g, b);
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

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let tgt = wheel_target(app, ui);
    let color = tgt.color();
    let cms = cms::active();
    // With None, a pattern or a "?" proxy the wheel shows the last solid color without markers,
    // and a click, an arrow key or a screen reader's action applies the picked color.
    let hsb = match (color, pstate::<Option<(Owner, [f32; 3], [f32; 3])>>(ui.ctx(), MEMORY)) {
        (Some(c), Some((owner, shown, hsb))) if owner == Owner::of(app, &tgt, c) => remembered_hsb(&cms, &c, shown, hsb),
        (c, _) => display_hsb(&cms, &c.unwrap_or(app.session.last_solid)),
    };
    let marked = color.is_some();
    let mut pick: Option<([f32; 3], Live)> = None;
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
        if let Some((h, phase)) = ring.hue {
            pick = Some(([h, hsb[1], hsb[2]], phase));
        }
        // A press between the square and the ring picks the closest point of the square, which
        // takes the keyboard focus.
        if let Some((p, phase)) = ring.inner {
            let field_id = widgets::id_with(field_ui.scope_id(), "field");
            field_ui.memory_mut(|m| m.request_focus(field_id));
            let f = |v: f32, lo: f32, len: f32| ((v - lo) / len).clamp(0.0, 1.0);
            pick = Some(([hsb[0], f(p.x, square.left(), square.width()), 1.0 - f(p.y, square.top(), square.height())], phase));
        }
        if let Some((x, y)) = field {
            pick = Some(([hsb[0], x, 1.0 - y], field_phase));
        }
    });
    if app.ui.color_wheel.contrast {
        contrast_row(app, ui);
    }
    let Some((hsb, phase)) = pick else { return };
    let behind = super::alt_held(ui);
    let cmyk_doc = app.session.active().is_some_and(|d| d.doc.color_mode == ColorMode::Cmyk);
    let c = stored(&cms, hsb, replaced(app, &tgt, behind).as_ref(), cmyk_doc);
    apply(app, &tgt, c, phase, behind);
    if !behind {
        set_pstate(ui.ctx(), MEMORY, Some((Owner::of(app, &tgt, c), cms.display_rgb(&c), hsb)));
    }
}
