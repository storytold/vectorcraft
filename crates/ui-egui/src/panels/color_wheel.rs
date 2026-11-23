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
//! Show Color Vision Preview and Show Document Colors are kept with the preferences
//! (`ui.colorWheel`).

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use egui::{Color32, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_color::cms::{self, Cms, ColorSettings, ProofSetup, ProofTarget};
use vectorcraft_color::harmony::Harmony;
use vectorcraft_color::oklab::{self, CHROMA_FULL};
use vectorcraft_color::{Color, Paint, hsb_to_rgb, hsl_to_rgb, rgb_to_hsb, rgb_to_hsl};
use vectorcraft_doc::{ColorMode, NodeId};

use super::color::{Target, apply, wheel_target};
use super::{pstate, set_pstate};
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

/// Serde reader for the saved settings ([`crate::UiState::color_wheel`]): a value that is not an
/// object reads as the defaults, so it doesn't lose the rest of the preferences.
pub fn deserialize_settings<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Settings, D::Error> {
    let v = <Value as Deserialize>::deserialize(d)?;
    Ok(serde_json::from_value(v).unwrap_or_default())
}

/// `ui.colorWheel {view?, channel?, sliders?, gamut?, harmony?, harmonyRule?, vision?,
/// documentColors?}`: show the wheel or the box, pick the box's channel, show OKLCH Sliders, HSL
/// Sliders or neither, show or hide the out-of-gamut shading, show the colors of a harmony rule on
/// the wheel, and show or hide the color vision row and the Show Document Colors row. Keys left
/// out or null keep their values; params that are not an object, another key or a value off the
/// list are an error and change nothing.
/// `sliders: "oklch"` is an error while the active document's RGB space (its assigned profile,
/// else the working RGB space) is not sRGB. → the settings.
pub(crate) fn command(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    set(&mut app.ui.color_wheel, p, &cms::active())?;
    Ok(json!(app.ui.color_wheel))
}

/// Set the keys of `ui.colorWheel` params `p` in `settings`, with the color settings `cms`; on an
/// error `settings` stay as they were.
pub(crate) fn set(settings: &mut Settings, p: &Value, cms: &Cms) -> Result<(), String> {
    const KEYS: [&str; 8] = ["view", "channel", "sliders", "gamut", "harmony", "harmonyRule", "vision", "documentColors"];
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
    if let Some(on) = crate::menus::opt_bool(p, "vision")? {
        set.vision = on;
    }
    if let Some(on) = crate::menus::opt_bool(p, "documentColors")? {
        set.document_colors = on;
    }
    *settings = set;
    Ok(())
}

/// The panel menu: Wheel View and Box View, and Box Channel while the box shows; Show Out-of-Gamut
/// Colors; HSL Sliders and OKLCH Sliders; Show Harmony and Harmony Rule while the wheel shows; Show
/// Color Vision Preview; and Show Document Colors.
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
/// HSB a click, a drag, a key or a screen reader's action picks, and its phase.
fn show_box(ui: &mut Ui, ch: Channel, hsb: [f32; 3], marked: bool, size: f32, gamut: Option<&Cms>) -> Option<([f32; 3], Live)> {
    let (z, x, y) = box_pos(ch, hsb);
    let mut pick = None;
    let grid = gamut.map(|cms| gamut_shade(ui, cms, ch, z));
    let shade = grid.as_ref().map_or(&[][..], |g| &g[..]);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = STRIP_GAP;
        let field = vec2((size - STRIP_GAP - STRIP).max(1.0), size);
        let at = |x: f32, y: f32| widgets::hsb32(box_pick(ch, z, x, y, hsb));
        let labels = (tl!("Color box"), ch.field_axes());
        if let (Some((x, y)), phase) = widgets::color_field_shaded(ui, "color-wheel-box", labels, field, (x, y), marked, (&at, shade)) {
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
    let gamut = app.ui.color_wheel.gamut.then_some(&*cms);
    let mut pick: Option<([f32; 3], Live)> = None;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(44.0);
            super::proxy(app, ui, 40.0);
        });
        let size = ui.available_width().min(MAX_WHEEL);
        if app.ui.color_wheel.view == View::Box {
            pick = show_box(ui, app.ui.color_wheel.channel, hsb, marked, size, gamut);
            return;
        }
        let ring = widgets::hue_ring(ui, "color-wheel-ring", tl!("Hue ring"), size, hsb[0], marked);
        let square = widgets::ring_square(ring.rect);
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
    let behind = super::alt_held(ui);
    if app.ui.color_wheel.document_colors
        && let Some(u) = document_colors_row(app, ui)
    {
        apply_used(app, &tgt, &u, behind);
        return;
    }
    let cmyk_doc = app.session.active().is_some_and(|d| d.doc.color_mode == ColorMode::Cmyk);
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
