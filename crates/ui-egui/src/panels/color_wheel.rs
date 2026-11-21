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
//! Show Out-of-Gamut Colors grays out the cells of the wheel's square and the box's field whose
//! colors the CMYK space cannot print, by the out-of-gamut warning's test ([`Cms::out_of_gamut`]),
//! on a [`GAMUT_CELLS`] by [`GAMUT_CELLS`] grid kept until the channel, its value, the color
//! settings or the loaded profiles change.
//!
//! Show Harmony puts a marker on the ring for each other color of the harmony rule picked under
//! Harmony Rule (the Color Guide's rules whose colors each have a hue of their own), filled with
//! that color. The markers follow the current color, and a click on one applies its color as a
//! pick does.
//!
//! Panel menu › Show Contrast adds a row under the wheel or the box with the WCAG 2 contrast ratio
//! (<https://www.w3.org/TR/WCAG22/#dfn-contrast-ratio>) of the color against the color drawn
//! behind the selection, or against a color pinned with Contrast Background › Current Color, and
//! whether it meets AA and AAA for the category picked under Contrast For (`color.contrast`).
//! The wheel's square and the box's field then draw lines where the color picked there would meet
//! AA and AAA (AAA dashed), and when the color fails, a chip in the row offers the closest color
//! that passes, the one `color.contrast` answers as `fix`.
//!
//! Panel menu › Show Color Vision Preview adds a row under the wheel or the box with the color as
//! simulated for protanopia, deuteranopia and tritanopia, through the simulations View › Proof
//! Setup uses, and for achromatopsia as the gray with the color's CIE L*. The chips only show the
//! simulation.
//!
//! The panel menu's Show Document Colors adds a row under the wheel or the box with the colors
//! of the selection, or with nothing selected of the art Select All takes, most used first
//! (`color.used`). A click on one applies it as a pick does. The active proxy and a selected
//! gradient stop keep its swatch link; a freeform point takes the color alone.
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
//! The view, the channel, the slider rows, Show Out-of-Gamut Colors, Show Harmony, Harmony Rule,
//! the contrast settings, Show Color Vision Preview and Show Document Colors are kept with the
//! preferences (`ui.colorWheel`).

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use egui::{Color32, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_color::cms::{self, Cms, ColorSettings, ProofSetup, ProofTarget};
use vectorcraft_color::harmony::Harmony;
use vectorcraft_color::oklab::{self, CHROMA_FULL};
use vectorcraft_color::wcag::{self, Category};
use vectorcraft_color::{Color, Paint, hsb_to_rgb, hsl_to_rgb, rgb_to_hsb, rgb_to_hsl};
use vectorcraft_doc::{ColorMode, NodeId};
use vectorcraft_engine::cmd::{Backdrop, contrast_measure};

use super::color::{Target, apply, wheel_target};
use super::{color_json, pstate, set_pstate};
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
    #[serde(deserialize_with = "deserialize_sliders")]
    pub sliders: Sliders,
    /// Show Out-of-Gamut Colors: gray out the colors the CMYK space cannot print.
    #[serde(deserialize_with = "deserialize_flag")]
    pub gamut: bool,
    /// Show Harmony: markers on the ring for the other colors of `harmony_rule`.
    #[serde(deserialize_with = "deserialize_flag")]
    pub harmony: bool,
    /// The rule of Show Harmony, one of [`harmony_rules`].
    #[serde(serialize_with = "serialize_rule", deserialize_with = "deserialize_rule")]
    pub harmony_rule: Harmony,
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
    /// Show Color Vision Preview: the row of simulated colors under the wheel or the box. A saved
    /// `true` shows the row and any other value hides it.
    #[serde(deserialize_with = "crate::panels::color::deserialize_rgb_percent")]
    pub vision: bool,
    /// Show Document Colors: the row of the colors the art uses, under the wheel or the box.
    #[serde(deserialize_with = "deserialize_flag")]
    pub document_colors: bool,
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
            sliders: Sliders::default(),
            gamut: false,
            harmony: false,
            harmony_rule: Harmony::Complementary,
            contrast: false,
            contrast_for: Category::default(),
            contrast_background: None,
            vision: false,
            document_colors: false,
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

fn deserialize_flag<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    let v = <Value as Deserialize>::deserialize(d)?;
    Ok(v.as_bool().unwrap_or_default())
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

/// `ui.colorWheel {view?, channel?, sliders?, gamut?, harmony?, harmonyRule?, contrast?,
/// contrastFor?, contrastBackground?, vision?, documentColors?}`: show the wheel or the box, pick
/// the box's channel, show OKLCH Sliders, HSL Sliders or neither, show or hide the out-of-gamut
/// shading, show the colors of a harmony rule on the wheel, set the contrast row, its category and
/// its pinned background (`contrastBackground: null` unpins it), and show or hide the color vision
/// row and the Show Document Colors row. Keys left out keep their values, and so do the other keys
/// when null; params that are not an object, another key or a value off the list are an error and
/// change nothing.
/// `sliders: "oklch"` is an error while the active document's RGB space (its assigned profile,
/// else the working RGB space) is not sRGB. → the settings.
pub(crate) fn command(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    set(&mut app.ui.color_wheel, p, &cms::active())?;
    Ok(json!(app.ui.color_wheel))
}

/// Set the keys of `ui.colorWheel` params `p` in `settings`, with the color settings `cms`; on an
/// error `settings` stay as they were.
pub(crate) fn set(settings: &mut Settings, p: &Value, cms: &Cms) -> Result<(), String> {
    const KEYS: [&str; 11] = [
        "view",
        "channel",
        "sliders",
        "gamut",
        "harmony",
        "harmonyRule",
        "contrast",
        "contrastFor",
        "contrastBackground",
        "vision",
        "documentColors",
    ];
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
    let key = "gamut";
    match p.get(key) {
        None | Some(Value::Null) => {}
        Some(v) => set.gamut = v.as_bool().ok_or_else(|| format!("`{key}` must be true or false"))?,
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
    if let Some(on) = crate::menus::opt_bool(p, "vision")? {
        set.vision = on;
    }
    if let Some(on) = crate::menus::opt_bool(p, "documentColors")? {
        set.document_colors = on;
    }
    *settings = set;
    Ok(())
}

/// A Contrast For category's menu label in the interface language.
pub(super) fn category_name(c: Category) -> &'static str {
    tl!(c.label())
}

/// The panel menu: Wheel View and Box View, and Box Channel while the box shows; Show Out-of-Gamut
/// Colors; HSL Sliders and OKLCH Sliders; Show Harmony and Harmony Rule while the wheel shows; Show
/// Contrast, Contrast For and Contrast Background; Show Color Vision Preview; and Show Document
/// Colors.
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
    if menu_item(ui, tl!("Show Out-of-Gamut Colors"), true, set.gamut) {
        command(app, &json!({ "gamut": !set.gamut })).ok();
    }
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
    if menu_item(ui, tl!("Show Color Vision Preview"), true, set.vision) {
        command(app, &json!({ "vision": !set.vision })).ok();
    }
    if menu_item(ui, tl!("Show Document Colors"), true, set.document_colors) {
        command(app, &json!({ "documentColors": !set.document_colors })).ok();
    }
}

/// Panel state: the Show Document Colors row's colors, with the [`UsedKey`] they were read for.
const USED: &str = "color-wheel-used";
/// The fewest chips the row asks `color.used` for; a row wider than this many chips asks for as
/// many as it holds.
const MAX_CHIPS: usize = 128;
/// Above this many color uses, an edit makes the row read the colors again at most once every
/// [`LARGE_EVERY`] seconds; a change of document or selection still reads them at once.
const LARGE_USES: u64 = 5_000;
const LARGE_EVERY: f64 = 0.5;

/// What the Show Document Colors row depends on: the document, its revision, and the number of
/// selected objects with a hash of their ids.
type UsedKey = Option<(u64, u64, usize, u64)>;

/// The row's colors as last read, kept in the panel state behind an `Arc`, so a frame that reads
/// them back copies no list.
#[derive(Debug, PartialEq)]
struct UsedCache {
    key: UsedKey,
    /// The frame time the colors were read at.
    at: f64,
    /// The uses of every color (`color.used`'s `uses`).
    uses: u64,
    /// Whether the colors are the selection's.
    selected: bool,
    /// The most colors asked for.
    limit: usize,
    colors: Vec<Used>,
}

/// A color of the row: the color, its hex value, how many times the art uses it, and the global
/// swatch it is linked to with the tint (0..1) of that swatch it is.
#[derive(Clone, Debug, PartialEq)]
struct Used {
    color: Color,
    hex: String,
    count: u64,
    link: Option<(String, f32)>,
}

impl Used {
    /// The paint the color applies and drags: linked to its swatch at its tint when it has one.
    fn paint(&self) -> Paint {
        match &self.link {
            Some((name, tint)) => Paint::Solid { color: self.color, swatch: Some(name.clone()), tint: *tint },
            None => Paint::solid(self.color),
        }
    }
}

/// The colors the row shows and whether they are the selection's, read with `color.used` once per
/// document revision and selection and kept as they are while an interaction is in progress (a
/// preview raises the revision on every frame of a drag). In art with more than [`LARGE_USES`]
/// color uses, an edit reads them again only after [`LARGE_EVERY`] seconds. A row that holds more
/// than the colors last asked for reads them again once no interaction is in progress, and in such
/// art only after the same wait.
fn used_colors(app: &mut VectorcraftApp, ui: &Ui, per_row: usize) -> Arc<UsedCache> {
    let limit = per_row.max(MAX_CHIPS);
    let key: UsedKey = app.session.active().map(|d| {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        d.selection.objects.hash(&mut h);
        (d.uid, d.revision, d.selection.objects.len(), h.finish())
    });
    let now = ui.input(|i| i.time);
    if let Some(cache) = pstate::<Option<Arc<UsedCache>>>(ui.ctx(), USED) {
        let fits = cache.limit >= per_row || cache.colors.len() < cache.limit;
        if app.session.in_interaction() || (cache.key == key && fits) {
            return cache;
        }
        let scope = |k: &UsedKey| k.map(|(doc, _, n, h)| (doc, n, h));
        let wait = LARGE_EVERY - (now - cache.at);
        if cache.uses > LARGE_USES && scope(&cache.key) == scope(&key) && wait > 0.0 {
            ui.ctx().request_repaint_after_secs(wait as f32);
            return cache;
        }
    }
    let r = key.as_ref().and_then(|_| app.session.execute("color.used", &json!({ "limit": limit })).ok());
    let colors: Vec<Used> = r
        .as_ref()
        .and_then(|r| r["colors"].as_array())
        .map(|a| {
            a.iter()
                .filter_map(|c| {
                    let color = serde_json::from_value::<Color>(c["color"].clone()).ok()?;
                    let hex = c["hex"].as_str().map_or_else(|| color.to_hex(), str::to_string);
                    let link = c["swatch"].as_str().zip(c["tint"].as_f64()).map(|(n, t)| (n.to_string(), (t / 100.0).clamp(0.0, 1.0) as f32));
                    Some(Used { color, hex, count: c["count"].as_u64().unwrap_or(0), link })
                })
                .collect()
        })
        .unwrap_or_default();
    let selected = r.as_ref().is_some_and(|r| r["selection"] == true);
    let uses = r.as_ref().and_then(|r| r["uses"].as_u64()).unwrap_or(0);
    let cache = Arc::new(UsedCache { key, at: now, uses, selected, limit, colors });
    set_pstate(ui.ctx(), USED, Some(Arc::clone(&cache)));
    cache
}

/// The Show Document Colors row: a heading (Selection Colors or Document Colors) and the colors,
/// most used first, as many as fit in one row. Returns the color clicked.
fn document_colors_row(app: &mut VectorcraftApp, ui: &mut Ui) -> Option<Used> {
    let width = ui.available_width();
    // Chips are 16 px on a 19 px pitch, inset 3 px, with 2 px kept clear of the right edge, as
    // in the Recent Colors row.
    let per_row = (((width - 21.0) / 19.0).floor().max(0.0) as usize + 1).max(1);
    let cache = used_colors(app, ui, per_row);
    widgets::subheader(ui, if cache.selected { tl!("Selection Colors") } else { tl!("Document Colors") });
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(width, 22.0), Sense::hover());
    ui.painter().rect_stroke(r, 0.0, egui::Stroke::new(1.0, t.input_border), egui::StrokeKind::Inside);
    let mut chosen = None;
    for (i, u) in cache.colors.iter().take(per_row).enumerate() {
        let cell = Rect::from_min_size(r.min + vec2(3.0 + i as f32 * 19.0, 3.0), vec2(16.0, 16.0));
        let resp = ui.interact(cell, ui.scope_id().with(("document-color", i)), Sense::click_and_drag());
        let paint = u.paint();
        widgets::drag_source(ui, &resp, || widgets::PanelDrag::paint(paint.clone()));
        widgets::swatch_tile(ui, cell, &paint, false, resp.hovered());
        let tip = format!("{}\n{}", u.hex, crate::i18n::tn(u.count, "Used {n} time", "Used {n} times"));
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &tip));
        if resp.on_hover_text(&tip).clicked() {
            chosen = Some(u.clone());
        }
    }
    chosen
}

/// Apply a color of the Show Document Colors row as a pick does, to the inactive proxy when
/// `behind`. A color linked to a swatch stays linked at its tint, except at a freeform gradient's
/// point, which takes the color alone.
fn apply_used(app: &mut VectorcraftApp, tgt: &Target, u: &Used, behind: bool) {
    let Some((name, tint)) = &u.link else { return apply(app, tgt, u.color, Live::Released, behind) };
    match tgt {
        Target::Stop { paint: Paint::Gradient(g), index, .. } if !behind => {
            let mut stops = g.gradient.stops.clone();
            if let Some(s) = stops.get_mut(*index) {
                s.set_color(u.color, Some((name.clone(), *tint)));
            }
            super::gradient::set_stops(app, &stops, None, Live::Released);
        }
        Target::Point { .. } if !behind => apply(app, tgt, u.color, Live::Released, behind),
        _ => {
            let mut p = super::paint_params(&u.paint());
            p["focus"] = json!(!behind);
            super::live_run(app, "Color", super::proxy_cmd(app, behind), p, Live::Released);
        }
    }
}

/// The cells across and down of the grid the out-of-gamut shading tests.
pub(crate) const GAMUT_CELLS: usize = 16;

/// The cells of a field whose colors are out of the CMYK gamut, a row per [`GAMUT_CELLS`] from the
/// top: bit `i` of a row is its cell `i` from the left.
pub(crate) type GamutGrid = [u16; GAMUT_CELLS];

/// The out-of-gamut cells of the box of channel `ch` at strip value `z` through `cms`, each tested
/// at its center by the out-of-gamut warning's test ([`Cms::out_of_gamut`]) on the working-RGB
/// color of a pick there, before any conversion to CMYK or Lab in [`stored`]. The wheel's square is
/// the box of [`Channel::Hue`] at the ring's hue.
pub(crate) fn gamut_grid(cms: &Cms, ch: Channel, z: f32) -> GamutGrid {
    let n = GAMUT_CELLS as f32;
    let mut grid = [0; GAMUT_CELLS];
    for (row, bits) in grid.iter_mut().enumerate() {
        for col in 0..GAMUT_CELLS {
            let (x, y) = ((col as f32 + 0.5) / n, (row as f32 + 0.5) / n);
            // A gray has no hue, and the hue a gray keeps does not change its color.
            let [r, g, b] = cms.srgb_to_rgb(hsb_to_rgb(box_pick(ch, z, x, y, [0.0; 3])));
            if cms.out_of_gamut(&Color::rgb(r, g, b)) {
                *bits |= 1 << col;
            }
        }
    }
    grid
}

/// What the out-of-gamut grid depends on: the channel, its value's bits, the color settings and
/// [`cms::profiles_generation`], which changes when a profile is loaded again under the same name.
pub(crate) type GamutKey = (Channel, u32, ColorSettings, u64);

/// The grid for `key` from `slot`, computed with `compute` and kept in `slot` when `slot` holds
/// another key's.
pub(crate) fn cached_grid(slot: &mut Option<(GamutKey, GamutGrid)>, key: GamutKey, compute: impl FnOnce() -> GamutGrid) -> GamutGrid {
    match slot {
        Some((k, grid)) if *k == key => *grid,
        _ => {
            let grid = compute();
            *slot = Some((key, grid));
            grid
        }
    }
}

/// Panel state: the out-of-gamut grid drawn last and its [`GamutKey`].
pub(crate) const GAMUT: &str = "color-wheel-gamut";

/// The out-of-gamut grid of channel `ch` at value `z` through `cms`, kept in the panel state so a
/// frame with the same channel, value, color settings and loaded profiles does not test the colors
/// again.
fn gamut_shade(ui: &Ui, cms: &Cms, ch: Channel, z: f32) -> GamutGrid {
    let mut slot = pstate::<Option<(GamutKey, GamutGrid)>>(ui.ctx(), GAMUT);
    let key = (ch, z.to_bits(), cms.settings().clone(), cms::profiles_generation());
    let grid = cached_grid(&mut slot, key, || gamut_grid(cms, ch, z));
    set_pstate(ui.ctx(), GAMUT, slot);
    grid
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
/// markers when `marked` and the field's out-of-gamut cells grayed out when `gamut`. Returns the
/// HSB a click, a drag, a key or a screen reader's action picks, and its phase, and the field's
/// rect.
fn show_box(ui: &mut Ui, ch: Channel, hsb: [f32; 3], marked: bool, size: f32, gamut: Option<&Cms>) -> (Option<([f32; 3], Live)>, Rect) {
    let (z, x, y) = box_pos(ch, hsb);
    let mut pick = None;
    let mut field_rect = Rect::NOTHING;
    let grid = gamut.map(|cms| gamut_shade(ui, cms, ch, z));
    let shade = grid.as_ref().map_or(&[][..], |g| &g[..]);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = STRIP_GAP;
        let field = vec2((size - STRIP_GAP - STRIP).max(1.0), size);
        let at = |x: f32, y: f32| widgets::hsb32(box_pick(ch, z, x, y, hsb));
        let labels = (tl!("Color box"), ch.field_axes());
        let picked = widgets::color_field_shaded(ui, "color-wheel-box", labels, field, (x, y), marked, (&at, shade));
        // The field is the first thing in the row.
        field_rect = Rect::from_min_size(ui.min_rect().min, field);
        if let (Some((x, y)), phase) = picked {
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
    (pick, field_rect)
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
/// What the contrast row's answer depends on.
#[derive(Clone, PartialEq)]
pub(super) struct RatioKey {
    /// The pinned background, or `below` for the color behind the selection.
    pub(super) background: String,
    /// The active proxy's paint; with nothing selected it is the default for new art, which no
    /// revision counts.
    pub(super) paint: String,
    /// The document and its revision.
    pub(super) doc: Option<(u64, u64)>,
    pub(super) selection: Vec<NodeId>,
    /// The picked object of a dynamic symbol instance.
    pub(super) symbol_part: Option<Vec<u32>>,
    /// Whether the Fill proxy is active.
    pub(super) fill_active: bool,
    pub(super) appearance_item: Option<usize>,
    pub(super) stop: Option<usize>,
    pub(super) freeform_point: Option<usize>,
    /// The on-screen Appearance of Black.
    pub(super) black_on_screen: String,
    pub(super) under: Under,
    /// The picked gradient mesh points, which the active tool keeps without a revision.
    pub(super) mesh_points: Vec<(NodeId, Vec<usize>)>,
    /// The Contrast For category, which the closest passing colors are for.
    pub(super) category: Category,
}

/// Panel state: what the contrast row's answer was measured against, with its [`RatioKey`].
const BACKDROP: &str = "color-wheel-backdrop";
/// Panel state: the contrast lines of the square or the field, with the [`LinesKey`] they were
/// found for.
pub(super) const LINES: &str = "color-wheel-lines";
/// What the contrast lines depend on: what the color is measured against, the category, the field
/// (the view, the channel and the component the field holds fixed) with the model a pick there
/// stores, the color settings, and whether they were found coarsely during a drag.
pub(super) type LinesKey = (Backdrop, Category, String, Under, bool);
/// Per minimum of the category (AA, then AAA where it has one), the lines in field coordinates.
pub(super) type Lines = Vec<Vec<Vec<(f32, f32)>>>;

/// The columns and rows at which [`contrast_lines`] samples a field.
const LINE_SAMPLES: usize = 32;
/// The columns and rows at which [`contrast_lines`] samples a field, without the narrow-band
/// search, while a drag changes the field (the hue ring, the box's strip).
const COARSE_LINE_SAMPLES: usize = 8;
/// The halvings that place a line on the edge between two samples.
const LINE_STEPS: usize = 10;

/// A point of a field: x across, y down, 0 to 1 each.
type FieldPoint = (f32, f32);

/// The passing bands found between the samples of one column or row: where each peaks along it, and
/// its two crossings.
type Bands = Vec<(f32, FieldPoint, FieldPoint)>;

/// Where the contrast `ratio_at` a field point crosses each of `minimums`: per minimum, polylines in
/// field coordinates. The field is sampled on a grid of `n` + 1 columns and rows, and each cell
/// whose corners fall on both sides gets a segment between the crossings on its edges, placed by
/// bisection (marching squares; a cell with two opposite corners passing is split by its center).
/// With `bands`, around a failing sample higher than its neighbors in a column or a row, the highest
/// ratio is looked for, so a passing band narrower than the samples gets its two edges too, joined
/// from column to column or from row to row. Segments that share an end are joined into one
/// polyline.
pub(super) fn contrast_lines(ratio_at: &dyn Fn(f32, f32) -> f64, minimums: &[f64], n: usize, narrow: bool) -> Lines {
    let n = n.max(1);
    let at = |i: usize| i as f32 / n as f32;
    // `grid[i][j]` is column i (x) and row j (y).
    let grid: Vec<Vec<f64>> = (0..=n).map(|i| (0..=n).map(|j| ratio_at(at(i), at(j))).collect()).collect();
    let r = |i: usize, j: usize| grid.get(i).and_then(|c| c.get(j)).copied().unwrap_or(f64::NAN);
    minimums
        .iter()
        .map(|&min| {
            let pass = |p: FieldPoint| ratio_at(p.0, p.1) >= min;
            // The crossing between `p`, on the side `above` says, and `q`.
            let cross = |p: FieldPoint, q: FieldPoint, above: bool| {
                let lerp = |t: f32| (p.0 + (q.0 - p.0) * t, p.1 + (q.1 - p.1) * t);
                let (mut lo, mut hi) = (0.0, 1.0);
                for _ in 0..LINE_STEPS {
                    let mid = (lo + hi) / 2.0;
                    if pass(lerp(mid)) == above { lo = mid } else { hi = mid }
                }
                lerp((lo + hi) / 2.0)
            };
            let mut edges: HashMap<(usize, usize, bool), Option<FieldPoint>> = HashMap::new();
            // The crossing on the edge from sample (i, j) across (`across`) or down.
            let mut edge = |i: usize, j: usize, across: bool| {
                *edges.entry((i, j, across)).or_insert_with(|| {
                    let (i2, j2) = if across { (i + 1, j) } else { (i, j + 1) };
                    let (a, b) = (r(i, j) >= min, r(i2, j2) >= min);
                    (a != b).then(|| cross((at(i), at(j)), (at(i2), at(j2)), a))
                })
            };
            let mut segments: Vec<[FieldPoint; 2]> = vec![];
            for i in 0..n {
                for j in 0..n {
                    let (top, right, bottom, left) = (edge(i, j, true), edge(i + 1, j, false), edge(i, j + 1, true), edge(i, j, false));
                    match (top, right, bottom, left) {
                        (Some(t), Some(rt), Some(b), Some(l)) => {
                            // Opposite corners pass: the center says which pair of them it joins.
                            let half = 0.5 / n as f32;
                            if pass((at(i) + half, at(j) + half)) == (r(i, j) >= min) {
                                segments.extend([[t, rt], [b, l]]);
                            } else {
                                segments.extend([[t, l], [rt, b]]);
                            }
                        }
                        _ => {
                            let found: Vec<FieldPoint> = [top, right, bottom, left].into_iter().flatten().collect();
                            if let &[p, q] = found.as_slice() {
                                segments.push([p, q]);
                            }
                        }
                    }
                }
            }
            // Bands narrower than the samples, along each column and then each row.
            for across in [false, true].into_iter().filter(|_| narrow) {
                let point = |k: usize, t: f32| if across { (t, at(k)) } else { (at(k), t) };
                let value = |k: usize, s: usize| if across { r(s, k) } else { r(k, s) };
                let bands: Vec<Bands> = (0..=n)
                    .map(|k| {
                        let mut found = vec![];
                        for s in 0..=n {
                            let (before, after) = (s.saturating_sub(1), (s + 1).min(n));
                            let v = value(k, s);
                            if v >= min || v < value(k, before) || (after != s && v <= value(k, after)) {
                                continue;
                            }
                            let peak = wcag::peak_between(f64::from(at(before)), f64::from(at(after)), &|t| {
                                let p = point(k, t as f32);
                                ratio_at(p.0, p.1)
                            }) as f32;
                            if pass(point(k, peak)) {
                                found.push((
                                    peak,
                                    cross(point(k, at(before)), point(k, peak), false),
                                    cross(point(k, peak), point(k, at(after)), true),
                                ));
                            }
                        }
                        found
                    })
                    .collect();
                for pair in bands.windows(2) {
                    let [this, next] = pair else { continue };
                    for (peak, a, b) in this {
                        if let Some((_, c, d)) = next.iter().find(|(p, ..)| (p - peak).abs() <= 1.5 / n as f32) {
                            segments.extend([[*a, *c], [*b, *d]]);
                        }
                    }
                }
            }
            chain(segments)
        })
        .collect()
}

/// `segments` joined end to end into polylines where they share an end.
fn chain(segments: Vec<[FieldPoint; 2]>) -> Vec<Vec<FieldPoint>> {
    let key = |p: FieldPoint| (p.0.to_bits(), p.1.to_bits());
    let mut ends: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (s, seg) in segments.iter().enumerate() {
        for p in seg {
            ends.entry(key(*p)).or_default().push(s);
        }
    }
    let mut used = vec![false; segments.len()];
    let mut lines = vec![];
    for (start, [a, b]) in segments.iter().enumerate() {
        if used.get(start).copied().unwrap_or(true) {
            continue;
        }
        if let Some(u) = used.get_mut(start) {
            *u = true;
        }
        let mut line = std::collections::VecDeque::from([*a, *b]);
        for back in [true, false] {
            loop {
                let end = if back { line.back() } else { line.front() };
                let Some(end) = end.copied() else { break };
                let next = ends.get(&key(end)).and_then(|ss| ss.iter().copied().find(|s| !used.get(*s).copied().unwrap_or(true)));
                let Some((s, [p, q])) = next.and_then(|s| segments.get(s).map(|seg| (s, *seg))) else { break };
                if let Some(u) = used.get_mut(s) {
                    *u = true;
                }
                let other = if key(p) == key(end) { q } else { p };
                if back { line.push_back(other) } else { line.push_front(other) }
            }
        }
        lines.push(line.into_iter().collect());
    }
    lines
}

/// The contrast lines of the wheel's square or the box's field on HSB `hsb`, kept per [`LinesKey`]:
/// where a pick there, stored as the wheel stores it in place of `tgt`'s color, meets each minimum
/// of the chosen category against `backdrop`. While a drag is live and the field differs from the
/// one the kept lines were found for, the lines are found coarsely ([`COARSE_LINE_SAMPLES`]), and
/// in full once the drag ends.
fn lines_for(app: &VectorcraftApp, ui: &Ui, backdrop: &Backdrop, tgt: &Target, hsb: [f32; 3], cmyk_doc: bool) -> Lines {
    let set = &app.ui.color_wheel;
    let (category, view, ch) = (set.contrast_for, set.view, set.channel);
    let was = replaced(app, tgt, false);
    // The component the field holds fixed: the box's strip value, or the wheel's hue.
    let fixed = if view == View::Box { box_pos(ch, hsb).0 } else { hsb[0] };
    let field = format!("{view:?} {ch:?} {fixed} {:?} {cmyk_doc}", was.map(|c| c.model()));
    let dragging = app.session.in_interaction();
    let key = |coarse: bool| -> LinesKey { (backdrop.clone(), category, field.clone(), under(), coarse) };
    if let Some((k, lines)) = pstate::<Option<(LinesKey, Lines)>>(ui.ctx(), LINES)
        && (k == key(false) || (dragging && k == key(true)))
    {
        return lines;
    }
    let cms = cms::active();
    let at = |x: f32, y: f32| if view == View::Box { box_pick(ch, fixed, x, y, hsb) } else { [hsb[0], x, 1.0 - y] };
    let ratio_at = |x: f32, y: f32| backdrop.ratio(backdrop.shown(&cms, &stored(&cms, at(x, y), was.as_ref(), cmyk_doc)));
    let (aa, aaa) = category.minimums();
    let minimums: Vec<f64> = std::iter::once(aa).chain(aaa).collect();
    let samples = if dragging { COARSE_LINE_SAMPLES } else { LINE_SAMPLES };
    let lines = contrast_lines(&ratio_at, &minimums, samples, !dragging);
    set_pstate(ui.ctx(), LINES, Some((key(dragging), lines.clone())));
    lines
}

/// Draw `lines` (from [`contrast_lines`]) over the field in `rect`: AA solid and AAA dashed, each
/// a light line over a dark one, as the field's marker is drawn.
fn draw_contrast_lines(ui: &Ui, rect: Rect, lines: &Lines) {
    let painter = ui.painter().with_clip_rect(rect);
    let at = |(x, y): &(f32, f32)| egui::pos2(rect.left() + x * rect.width(), rect.top() + y * rect.height());
    for (level, polylines) in lines.iter().enumerate() {
        for line in polylines {
            let points: Vec<egui::Pos2> = line.iter().map(at).collect();
            for stroke in [Stroke::new(3.0, Color32::BLACK), Stroke::new(1.0, Color32::WHITE)] {
                if level == 0 {
                    painter.add(egui::Shape::line(points.clone(), stroke));
                } else {
                    painter.extend(egui::Shape::dashed_line(&points, stroke, 4.0, 3.0));
                }
            }
        }
    }
}

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
/// `color.contrast` reads behind the selection, kept per document revision and selection, so a
/// drag that moves the selection reads it again. None with nothing selected.
fn background(app: &mut VectorcraftApp, ui: &Ui) -> Option<Color> {
    if let Some(c) = app.ui.color_wheel.contrast_background {
        return Some(c);
    }
    let st = app.session.active()?;
    let key: BehindKey = (st.uid, st.revision, st.selection.objects.clone(), picked_part(st), app.session.prefs.black_on_screen.clone(), under());
    let hex = match pstate::<Option<(BehindKey, Option<String>)>>(ui.ctx(), BEHIND) {
        Some((k, hex)) if k == key => hex,
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

/// The contrast row's measurement: `color.contrast`'s answer for the chosen category, and what the
/// lines measure picks against.
struct Measured {
    answer: Option<Value>,
    backdrop: Option<Backdrop>,
}

/// The `color.contrast` answer for the chosen category against the pinned background, else
/// against the color behind the selection (`against: "below"`), with what it measured the color
/// against, kept per [`RatioKey`].
fn measure(app: &VectorcraftApp, ui: &Ui) -> Measured {
    let category = app.ui.color_wheel.contrast_for;
    let pinned = app.ui.color_wheel.contrast_background;
    let st = app.session.active();
    let key = RatioKey {
        background: pinned.map_or_else(|| "below".to_string(), |bg| format!("{bg:?}")),
        paint: format!("{:?}", super::active_paint(app)),
        doc: st.map(|d| (d.uid, d.revision)),
        selection: st.map(|d| d.selection.objects.clone()).unwrap_or_default(),
        symbol_part: st.and_then(picked_part),
        fill_active: app.session.fill_active,
        appearance_item: app.session.appearance_item(),
        stop: app.session.selected_stop(),
        freeform_point: app.session.selected_freeform_point(),
        black_on_screen: app.session.prefs.black_on_screen.clone(),
        under: under(),
        mesh_points: app.session.fill_mesh_points(),
        category,
    };
    let kept = pstate::<Option<(RatioKey, Option<Value>)>>(ui.ctx(), RATIO);
    let kept_backdrop = pstate::<Option<(RatioKey, Option<Backdrop>)>>(ui.ctx(), BACKDROP);
    match (kept, kept_backdrop) {
        (Some((k, answer)), Some((kb, backdrop))) if k == key && kb == key => Measured { answer, backdrop },
        _ => {
            let against = pinned.as_ref().map_or_else(|| json!("below"), color_json);
            let m = contrast_measure(&app.session, &json!({ "against": against, "for": category.id() })).ok();
            let (answer, backdrop) = m.map_or((None, None), |(r, b)| (Some(r), Some(b)));
            set_pstate(ui.ctx(), RATIO, Some((key.clone(), answer.clone())));
            set_pstate(ui.ctx(), BACKDROP, Some((key, backdrop.clone())));
            Measured { answer, backdrop }
        }
    }
}

/// The contrast row: the background's chip, the ratio cut off at two decimals, the AA and AAA
/// results for the chosen category, and a chip of the closest color that passes the first level the
/// color fails; a dash when there is no background or no color to measure. Without a pinned
/// background the first chip shows the background in the answer. Only when there is no answer is
/// the background read on its own ([`background`]), so the chip still shows it. The fix chip keeps
/// its space while hidden. Returns the fix color when its chip is clicked.
fn contrast_row(app: &mut VectorcraftApp, ui: &mut Ui, m: &Measured) -> Option<Color> {
    let category = app.ui.color_wheel.contrast_for;
    let pinned = app.ui.color_wheel.contrast_background;
    let answer = m.answer.as_ref();
    // The background the ratio was measured against.
    let measured = answer.filter(|_| pinned.is_none()).and_then(|r| r["against"].as_str()).and_then(Color::from_hex);
    let chip = match pinned.or(measured) {
        Some(c) => Some(c),
        None => background(app, ui),
    };
    let mut clicked = None;
    let resp = ui
        .horizontal(|ui| {
            // The chip takes the keyboard focus and gives screen readers its name and hex value, as
            // the color vision chips do.
            let (rect, chip_resp) = ui.allocate_exact_size(vec2(CHIP, CHIP), Sense::focusable_noninteractive());
            match chip {
                Some(c) => widgets::swatch_tile(ui, rect, &Paint::solid(c), false, false),
                None => widgets::swatch_tile(ui, rect, &Paint::None, false, false),
            }
            if chip_resp.has_focus() {
                ui.painter().rect_stroke(rect, 0.0, ui.visuals().selection.stroke, StrokeKind::Inside);
            }
            let enabled = ui.is_enabled();
            let value = chip.map(|c| hex(bytes(cms::active().display_rgb(&c))));
            chip_resp.widget_info(|| egui::WidgetInfo {
                current_text_value: value.clone(),
                ..egui::WidgetInfo::labeled(egui::Role::Image, enabled, tl!("Background"))
            });
            let Some(ratio) = answer.and_then(|r| r["ratio"].as_f64()) else {
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
            clicked = fix_chip(ui, answer, aa, aaa);
        })
        .response;
    let mut tip = tl!(
        "WCAG 2.2 contrast minimums: normal text AA 4.5:1 (1.4.3) and AAA 7:1 (1.4.6), large text AA 3:1 and AAA 4.5:1, graphics and controls AA 3:1 (1.4.11)."
    )
    .to_string();
    if pinned.is_none() {
        tip.push(' ');
        tip.push_str(tl!("The background is read at the center of the selection."));
    }
    resp.on_hover_text(tip);
    clicked
}

/// The fix chip at the end of the contrast row: the closest color that passes AA when the color
/// fails AA (`aa`), else AAA when it fails AAA (`aaa`), from `color.contrast`'s `fix`. Its space is
/// kept while there is none, so the row does not move as a drag crosses a line. Returns the color
/// when the chip is clicked or activated.
fn fix_chip(ui: &mut Ui, answer: Option<&Value>, aa: bool, aaa: Option<bool>) -> Option<Color> {
    let level = if !aa {
        Some(("aa", "AA"))
    } else if aaa == Some(false) {
        Some(("aaa", "AAA"))
    } else {
        None
    };
    let fix = level.and_then(|(key, name)| {
        let v = answer?.get("fix")?.get(key)?;
        serde_json::from_value::<Color>(v.clone()).ok().map(|c| (c, name))
    });
    let Some((color, name)) = fix else {
        ui.allocate_exact_size(vec2(CHIP, CHIP), egui::Sense::hover());
        return None;
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(CHIP, CHIP), egui::Sense::click());
    widgets::swatch_tile(ui, rect, &Paint::solid(color), false, false);
    let label = crate::i18n::fmt(tl!("The closest color that passes {level}: click to apply it."), &[("level", name)]);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &label));
    let resp = resp.on_hover_text(&label);
    resp.clicked().then_some(color)
}

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

/// The L, C and H rows of OKLCH Sliders for the target's `color` through `cms`: lightness from 0 to
/// 100 %, chroma from 0 to 100 % of 0.4 (as CSS reads a chroma percentage) and hue from 0 to 360°.
/// Each track shows the colors its slider reaches and dims the part outside sRGB, and a tick on the
/// C track marks the largest chroma sRGB holds at the row's L and H. The rows are gray with no
/// color to edit and while the working RGB space is not sRGB. Returns the OKLCH a drag, an arrow
/// key, a screen reader or a field asks for, and its phase.
pub(crate) fn oklch_rows(app: &VectorcraftApp, ui: &mut Ui, tgt: &Target, color: Option<Color>, cms: &Cms) -> Option<([f64; 3], Live)> {
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
    let mut out = None;
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
                    out = Some((at(x), phase));
                }
                if let Some(k) = keyed {
                    out = Some((with(i, k), Live::Released));
                }
                if let Some(f) = widgets::plain_field_named(ui, ("color-wheel-oklch-field", i), Some(name), v, suffix, 0, field_w) {
                    out = Some((with(i, f), Live::Released));
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
/// drag, an arrow key, a screen reader or a field asks for, and its phase.
pub(crate) fn hsl_rows(app: &VectorcraftApp, ui: &mut Ui, tgt: &Target, color: Option<Color>, cms: &Cms) -> Option<([f32; 3], Live)> {
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
    let mut out = None;
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
                    out = Some((at(x), phase));
                }
                if let Some(k) = keyed {
                    out = Some((with(i, k), Live::Released));
                }
                if let Some(f) = widgets::plain_field_named(ui, ("color-wheel-hsl-field", i), Some(&name), v, suffix, 0, field_w) {
                    out = Some((with(i, f), Live::Released));
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
    let measured = app.ui.color_wheel.contrast.then(|| measure(app, ui));
    let gamut = app.ui.color_wheel.gamut.then_some(&*cms);
    let mut pick: Option<([f32; 3], Live)> = None;
    let mut field_rect = Rect::NOTHING;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(44.0);
            super::proxy(app, ui, 40.0);
        });
        let size = ui.available_width().min(MAX_WHEEL);
        if app.ui.color_wheel.view == View::Box {
            (pick, field_rect) = show_box(ui, app.ui.color_wheel.channel, hsb, marked, size, gamut);
            return;
        }
        let ring = widgets::hue_ring(ui, "color-wheel-ring", tl!("Hue ring"), size, hsb[0], marked);
        let square = widgets::ring_square(ring.rect);
        field_rect = square;
        let mut field_ui = ui.new_child(UiBuilder::new().id_salt("color-wheel-square").max_rect(square));
        let axes =
            [FieldAxis { name: tl!("Saturation"), units: 100.0, suffix: "%" }, FieldAxis { name: tl!("Brightness"), units: 100.0, suffix: "%" }];
        let at = |x: f32, y: f32| widgets::hsb32([hsb[0], x, 1.0 - y]);
        let name = tl!("Saturation and brightness");
        // The square is the hue box at the ring's hue, so it shares the box's grid.
        let grid = gamut.map(|cms| gamut_shade(ui, cms, Channel::Hue, box_pos(Channel::Hue, hsb).0));
        let shade = grid.as_ref().map_or(&[][..], |g| &g[..]);
        let pos = (hsb[1], 1.0 - hsb[2]);
        let (field, field_phase) = widgets::color_field_shaded(&mut field_ui, "field", (name, axes), square.size(), pos, marked, (&at, shade));
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
                pick = Some((c, Live::Released));
            }
        }
        if let Some((h, phase)) = ring.hue
            && !held
            && pick.is_none()
        {
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
    let cmyk_doc = app.session.active().is_some_and(|d| d.doc.color_mode == ColorMode::Cmyk);
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
    if let Some(m) = &measured {
        if let Some(backdrop) = &m.backdrop {
            let lines = lines_for(app, ui, backdrop, &tgt, hsb, cmyk_doc);
            draw_contrast_lines(ui, field_rect, &lines);
        }
        if let Some(c) = contrast_row(app, ui, m) {
            apply(app, &tgt, c, Live::Released, false);
        }
    }
    if app.ui.color_wheel.vision {
        ui.add_space(4.0);
        vision_row(ui, &cms, color.as_ref());
    }
    let behind = super::alt_held(ui);
    if app.ui.color_wheel.document_colors
        && let Some(u) = document_colors_row(app, ui)
    {
        apply_used(app, &tgt, &u, behind);
        return;
    }
    let rows = match (picked_hsl, picked_lch) {
        (Some((hsl, phase)), _) => Some((hsl_to_rgb(hsl), phase, HSL_MEMORY, Remembered::Hsl(hsl))),
        (_, Some((lch, phase))) => Some((oklab::from_oklch(lch[0], lch[1], lch[2]).0, phase, OKLCH_MEMORY, Remembered::Oklch(lch))),
        _ => None,
    };
    if let Some((srgb, phase, key, asked)) = rows {
        // A hue on a gray, or a saturation on black or white, leaves the displayed color as it is:
        // the rows keep what they were given, and the paint (a Gray color too) stays unchanged.
        let current = color.filter(|c| {
            let d = cms.display_rgb(c);
            !behind && live.is_none() && (0..3).all(|i| (d[i] - srgb[i]).abs() <= 1e-5)
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
    let Some((hsb, phase)) = pick else { return };
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
