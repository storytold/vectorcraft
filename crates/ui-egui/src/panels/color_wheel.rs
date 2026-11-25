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
//! Show Out-of-Gamut Colors grays out the parts of the wheel's square and the box's field whose
//! colors the CMYK space cannot print, by the out-of-gamut warning's test ([`Cms::out_of_gamut`]).
//!
//! Show Harmony puts a marker on the ring for each other color of the harmony rule picked under
//! Harmony Rule (the Color Guide's rules whose colors each have a hue of their own), filled with
//! that color. The markers follow the current color, and a click on one applies its color as a
//! pick does.
//!
//! The panel menu's Show Opacity adds a row under the wheel or the box that sets the opacity of
//! what the panel edits: the fill item (`paint.setFill {opacity}`), the stroke item
//! (`paint.setStroke {opacity}`), the selected gradient stop or the selected freeform point (the
//! first stop or point when none is selected, as for a color pick). The row is gray with nothing
//! selected, for an object selected in a symbol instance, and when the fill or stroke item it
//! would set is on type characters or missing.
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
//! Show Opacity, Show Color Vision Preview and Show Document Colors are kept with the preferences
//! (`ui.colorWheel`).

use std::hash::{Hash, Hasher};
use std::sync::Arc;

// PLAN: assumed `craft-color-ui`, `craft-theme`, `craft-behavior` and `craft-keymap` are dependencies of
// vectorcraft-ui-egui (Cargo.toml is not in this set's table); `craft_ui` does not re-export them at the base.
use craft::behavior::ColumnWidth;
use craft::behavior::Mixed;
use craft::behavior::Selection;
use craft::color_ui::AreaMode;
use craft::color_ui::ui::{ChannelRow, ChannelUnit, ColorArea, ColorStrip, ColorWheel, WheelInner};
use craft::keymap::CommandRef;
use craft::theme::{ColorModel, HsbModel, ModelValues, RgbModel, Srgba};
use craft::ui::choice::{SwatchSpacing, SwatchTable};
use craft::ui::contract::{ActivateOn, Bind, EditResponse, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Shade, Text};
use craft::ui::layout::CellAlign;
use craft::ui::layout_components::{Grid, Row, Stack};
use craft::ui::menu::PanelRow;
use craft::ui::runtime::commands::{CommandState, CommandStates};
use craft::ui::runtime::env::Label;
use egui::Ui;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_color::cms::{self, Cms, ProofSetup, ProofTarget};
use vectorcraft_color::harmony::{self, Harmony};
use vectorcraft_color::oklab::{self, CHROMA_FULL};
use vectorcraft_color::{Color, Paint, hsb_to_rgb, hsl_to_rgb, rgb_to_hsb, rgb_to_hsl};
use vectorcraft_doc::{ColorMode, NodeId};

// PLAN: assumed `crate::edits::{apply_edits, EditPath}` (wg.A16, widgets-w13) and `crate::panels::Live` (co.A2, hub-w2).
use super::color::{Target, apply, gamut, wheel_target};
use super::{Live, alt_held};
use crate::VectorcraftApp;
use crate::edits::{EditPath, apply_edits};

pub const ID: &str = "colorWheel";

/// The columns of the panel's top row: the proxy as wide as its content, and the wheel or the box beside it in the
/// rest of the width.
const BESIDE: [ColumnWidth; 2] = [ColumnWidth::Content, ColumnWidth::fill(1.0)];

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

/// The component the box's strip edits; the field shows the other two.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Channel {
    #[default]
    Hue,
    Saturation,
    Brightness,
    Red,
    Green,
    Blue,
}

impl Channel {
    pub const ALL: [Channel; 6] = [Channel::Hue, Channel::Saturation, Channel::Brightness, Channel::Red, Channel::Green, Channel::Blue];

    pub fn id(self) -> &'static str {
        match self {
            Channel::Hue => "hue",
            Channel::Saturation => "saturation",
            Channel::Brightness => "brightness",
            Channel::Red => "red",
            Channel::Green => "green",
            Channel::Blue => "blue",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.id() == s)
    }

    /// The channel's name, translated: Hue, Saturation, Brightness, Red, Green or Blue.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Channel::Hue => tl!("Hue"),
            Channel::Saturation => tl!("Saturation"),
            Channel::Brightness => tl!("Brightness"),
            Channel::Red => tl!("Red"),
            Channel::Green => tl!("Green"),
            Channel::Blue => tl!("Blue"),
        }
    }

    pub(crate) fn is_hsb(self) -> bool {
        matches!(self, Channel::Hue | Channel::Saturation | Channel::Brightness)
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
    /// Show Opacity: the opacity row under the wheel or the box.
    #[serde(deserialize_with = "deserialize_flag")]
    pub opacity: bool,
    /// Show Color Vision Preview: the row of simulated colors under the wheel or the box. A saved
    /// `true` shows the row and any other value hides it.
    #[serde(deserialize_with = "crate::panels::color::deserialize_lenient_bool")]
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
            opacity: false,
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
    harmony::parse(s).filter(|h| h.hues_differ())
}

fn deserialize_flag<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    let v = <Value as Deserialize>::deserialize(d)?;
    Ok(v.as_bool().unwrap_or_default())
}

fn serialize_rule<S: serde::Serializer>(h: &Harmony, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(h.id())
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

/// `ui.colorWheel {view?, channel?, sliders?, gamut?, harmony?, harmonyRule?, opacity?, vision?,
/// documentColors?}`: show the wheel or the box, pick the box's channel, show OKLCH Sliders, HSL
/// Sliders or neither, show or hide the out-of-gamut shading, show the colors of a harmony rule on
/// the wheel, check or uncheck Show Opacity, and show or hide the color vision row and the Show
/// Document Colors row. Keys left out or null keep their values; params that are not an object,
/// another key or a value off the list are an error and change nothing.
/// `sliders: "oklch"` is an error while the active document's RGB space (its assigned profile,
/// else the working RGB space) is not sRGB. → the settings.
pub(crate) fn command(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    set(&mut app.ui.color_wheel, p, &cms::active())?;
    Ok(json!(app.ui.color_wheel))
}

/// Set the keys of `ui.colorWheel` params `p` in `settings`, with the color settings `cms`; on an
/// error `settings` stay as they were.
pub(crate) fn set(settings: &mut Settings, p: &Value, cms: &Cms) -> Result<(), String> {
    const KEYS: [&str; 9] = ["view", "channel", "sliders", "gamut", "harmony", "harmonyRule", "opacity", "vision", "documentColors"];
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
                let ids: Vec<&str> = harmony_rules().map(Harmony::id).collect();
                format!("`{key}` must be one of {}", ids.join(", "))
            })?
        }
    }
    if let Some(on) = crate::menus::opt_bool(p, "opacity")? {
        set.opacity = on;
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

/// One row of the panel menu: its command id below `ui.colorWheel.`, the label and state the menu
/// shows, and the `ui.colorWheel` params it runs.
struct Item {
    id: String,
    label: String,
    checked: bool,
    enabled: bool,
    hint: Option<String>,
    params: Value,
}

/// The id of the menu row `row`.
fn row_id(row: &str) -> String {
    format!("ui.colorWheel.{row}")
}

fn channel_row_id(ch: Channel) -> String {
    format!("channel.{}", ch.id())
}

fn rule_row_id(rule: Harmony) -> String {
    format!("rule.{}", rule.id())
}

/// Every row of the panel menu with its state for `set`, `srgb` telling whether the active
/// document's RGB space is sRGB.
fn items(set: &Settings, srgb: bool) -> Vec<Item> {
    let item = |id: String, label: &str, checked: bool, enabled: bool, params: Value| Item {
        id,
        label: label.to_string(),
        checked,
        enabled,
        hint: None,
        params,
    };
    let mut out = Vec::new();
    for (id, label, view) in [("view.wheel", tl!("Wheel View"), View::Wheel), ("view.box", tl!("Box View"), View::Box)] {
        out.push(item(id.into(), label, set.view == view, true, json!({ "view": view.id() })));
    }
    for ch in Channel::ALL {
        out.push(item(channel_row_id(ch), ch.name(), ch == set.channel, set.view == View::Box, json!({ "channel": ch.id() })));
    }
    out.push(item("gamut".into(), tl!("Show Out-of-Gamut Colors"), set.gamut, true, json!({ "gamut": !set.gamut })));
    let on = set.sliders == Sliders::Hsl;
    let sliders = if on { Sliders::None } else { Sliders::Hsl };
    out.push(item("sliders.hsl".into(), tl!("HSL Sliders"), on, true, json!({ "sliders": sliders.id() })));
    let on = set.sliders == Sliders::Oklch;
    let sliders = if on { Sliders::None } else { Sliders::Oklch };
    // While the rows show, the item stays enabled so that they can be hidden.
    let mut oklch = item("sliders.oklch".into(), tl!("OKLCH Sliders"), on, on || srgb, json!({ "sliders": sliders.id() }));
    oklch.hint = Some(tl!("OKLCH needs sRGB as the working RGB space").to_string());
    out.push(oklch);
    out.push(item("harmony".into(), tl!("Show Harmony"), set.harmony, set.view == View::Wheel, json!({ "harmony": !set.harmony })));
    for rule in harmony_rules() {
        out.push(item(
            rule_row_id(rule),
            harmony::label(rule),
            rule == set.harmony_rule,
            set.view == View::Wheel,
            json!({ "harmonyRule": rule.id() }),
        ));
    }
    out.push(item("opacity".into(), tl!("Show Opacity"), set.opacity, true, json!({ "opacity": !set.opacity })));
    out.push(item("vision".into(), tl!("Show Color Vision Preview"), set.vision, true, json!({ "vision": !set.vision })));
    out.push(item(
        "documentColors".into(),
        tl!("Show Document Colors"),
        set.document_colors,
        true,
        json!({ "documentColors": !set.document_colors }),
    ));
    out
}

/// The panel menu: Wheel View and Box View, and Box Channel while the box shows; Show Out-of-Gamut
/// Colors; HSL Sliders and OKLCH Sliders; Show Harmony and Harmony Rule while the wheel shows; Show
/// Opacity; Show Color Vision Preview; and Show Document Colors.
pub(crate) fn menu_rows(_app: &VectorcraftApp) -> Vec<PanelRow> {
    let command = |row: String| PanelRow::Command(CommandRef::new(row_id(&row)));
    let mut rows = vec![command("view.wheel".into()), command("view.box".into())];
    rows.push(PanelRow::Submenu(
        Label::from(tl!("Box Channel").to_string()),
        Channel::ALL.into_iter().map(|ch| command(channel_row_id(ch))).collect(),
    ));
    rows.push(PanelRow::Separator);
    rows.extend(["gamut", "sliders.hsl", "sliders.oklch", "harmony"].map(|row| command(row.into())));
    rows.push(PanelRow::Submenu(Label::from(tl!("Harmony Rule").to_string()), harmony_rules().map(|rule| command(rule_row_id(rule))).collect()));
    rows.extend(["opacity", "vision", "documentColors"].map(|row| command(row.into())));
    rows
}

/// The labels, checks and enabled states of [`menu_rows`].
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    for item in items(&app.ui.color_wheel, cms::active().rgb_is_srgb()) {
        let mut state = CommandState::new().title(item.label).enabled(item.enabled).checked(item.checked);
        if let Some(hint) = item.hint {
            state = state.hint(hint);
        }
        states.set(CommandRef::new(row_id(&item.id)), state);
    }
}

/// Run the menu row `id` (`ui.colorWheel.<row>`); `None` when it is not one of this panel's.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    let row = id.strip_prefix("ui.colorWheel.")?;
    let params = items(&app.ui.color_wheel, cms::active().rgb_is_srgb()).into_iter().find(|item| item.id == row)?.params;
    Some(command(app, &params))
}

/// The fewest chips the row asks `color.used` for.
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

/// The panel's runtime state: the remembered picks and the memos keyed by what they were made
/// for. `UiState::color_wheel_panel`, never saved; the saved settings are `UiState::color_wheel`.
// PLAN: hub-w1 adds `UiState::color_wheel_panel: WheelUi` (`#[serde(skip)]`).
#[derive(Clone, Debug, Default)]
pub struct WheelUi {
    /// The HSB picked last, with its [`Owner`] and the display sRGB of the color it stored.
    hsb: Option<(Owner, [f32; 3], [f32; 3])>,
    /// The OKLCH the slider rows asked for last, with the [`Owner`] and display sRGB of the color they stored.
    oklch: Option<(Owner, [f32; 3], [f64; 3])>,
    /// The HSL the slider rows asked for last, with the [`Owner`] and display sRGB of the color they stored.
    hsl: Option<(Owner, [f32; 3], [f32; 3])>,
    /// The Show Document Colors row's colors.
    used: Option<Arc<UsedCache>>,
    /// The colors of the L, C and H tracks of OKLCH Sliders, each with the OKLCH (its own
    /// component left out) they were made for: the L and H tracks search the gamut for each color
    /// outside sRGB.
    oklch_tracks: [Option<([u64; 3], Vec<Srgba>)>; 3],
    /// The slider rows whose drag previews on an open interaction, until it is released.
    rows_live: Option<Sliders>,
    /// Whether a drag of the Show Opacity row previews on an open interaction, until it is released.
    opacity_live: bool,
}

/// The colors the row shows and whether they are the selection's, read with `color.used` once per
/// document revision and selection and kept as they are while an interaction is in progress (a
/// preview raises the revision on every frame of a drag). In art with more than [`LARGE_USES`]
/// color uses, an edit reads them again only after [`LARGE_EVERY`] seconds. `now` is the frame's
/// running time.
fn used_colors(app: &mut VectorcraftApp, panel: &mut WheelUi, ctx: &egui::Context, now: f64) -> Arc<UsedCache> {
    let key: UsedKey = app.session.active().map(|d| {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        d.selection.objects.hash(&mut h);
        (d.uid, d.revision, d.selection.objects.len(), h.finish())
    });
    if let Some(cache) = &panel.used {
        if app.session.in_interaction() || cache.key == key {
            return Arc::clone(cache);
        }
        let scope = |k: &UsedKey| k.map(|(doc, _, n, h)| (doc, n, h));
        let wait = LARGE_EVERY - (now - cache.at);
        if cache.uses > LARGE_USES && scope(&cache.key) == scope(&key) && wait > 0.0 {
            ctx.request_repaint_after_secs(wait as f32);
            return Arc::clone(cache);
        }
    }
    let r = key.as_ref().and_then(|_| app.session.execute("color.used", &json!({ "limit": MAX_CHIPS })).ok());
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
    let cache = Arc::new(UsedCache { key, at: now, uses, selected, colors });
    panel.used = Some(Arc::clone(&cache));
    cache
}

/// The Show Document Colors row: a heading (Selection Colors or Document Colors) and the colors,
/// most used first, in one row. Returns the color clicked.
fn document_colors_row(app: &mut VectorcraftApp, panel: &mut WheelUi, ui: &mut Ui, cms: &Cms, now: f64) -> Option<Used> {
    let cache = used_colors(app, panel, ui.ctx(), now);
    Text::new(if cache.selected { tl!("Selection Colors").to_string() } else { tl!("Document Colors").to_string() }).shade(Shade::Muted).show(ui);
    let entries: Vec<(Label, Srgba)> = cache
        .colors
        .iter()
        .map(|u| {
            let tip = format!("{}\n{}", u.hex, crate::i18n::tn(u.count, "Used {n} time", "Used {n} times"));
            (Label::from(tip), srgb(cms.display_rgb(&u.color)))
        })
        .collect();
    let drag = Arc::clone(&cache);
    let out = SwatchTable::new(tl!("Document colors").to_string(), &entries)
        .spacing(SwatchSpacing::Spaced)
        .tips(true)
        .max_rows(1)
        .activate(ActivateOn::Click)
        // The row picks by click and holds no selection of its own.
        .selection(Bind::given(Selection::default()))
        // PLAN: assumed `crate::panel_drag::PanelDrag::paint` survives widgets-w15 as the panel drag payload.
        .drag_payload(move |d| d.keys.first().and_then(|&i| drag.colors.get(i)).map(|u| crate::panel_drag::PanelDrag::paint(u.paint())))
        .show_all(ui);
    out.open.and_then(|i| cache.colors.get(i).cloned())
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

/// The widgets' HSB values (degrees, percent, percent) of HSB `hsb` (degrees, 0 to 1, 0 to 1).
fn hsb_values(hsb: [f32; 3]) -> ModelValues {
    ModelValues::new(HsbModel.id(), &[hsb[0], hsb[1] * 100.0, hsb[2] * 100.0])
}

/// The widgets' RGB values (0 to 255) of HSB `hsb`.
fn rgb_values(hsb: [f32; 3]) -> ModelValues {
    ModelValues::new(RgbModel.id(), &hsb_to_rgb(hsb).map(|v| v * 255.0))
}

/// The values the box of channel `ch` shows for HSB `hsb`, in its model.
fn box_values(ch: Channel, hsb: [f32; 3]) -> ModelValues {
    if ch.is_hsb() { hsb_values(hsb) } else { rgb_values(hsb) }
}

/// The widgets' mode for the box of channel `ch`: the strip's component, and the field's two on the
/// Color Picker's axes.
fn area_mode(ch: Channel) -> AreaMode {
    match ch {
        Channel::Hue => AreaMode::Hue,
        Channel::Saturation => AreaMode::Saturation,
        Channel::Brightness => AreaMode::Brightness,
        Channel::Red => AreaMode::Red,
        Channel::Green => AreaMode::Green,
        Channel::Blue => AreaMode::Blue,
    }
}

/// The HSB a widget's values pick, with `shown` the HSB the panel showed. A pick in an RGB model
/// that has no hue (a gray) keeps the hue of `shown`.
fn picked_hsb(v: &ModelValues, shown: [f32; 3]) -> [f32; 3] {
    if v.model == HsbModel.id() {
        return [v.values[0], v.values[1] / 100.0, v.values[2] / 100.0];
    }
    let hsb = rgb_to_hsb([v.values[0] / 255.0, v.values[1] / 255.0, v.values[2] / 255.0]);
    if hsb[1] <= 0.0 || hsb[2] <= 0.0 { [shown[0], hsb[1], hsb[2]] } else { hsb }
}

/// The widgets' color of display sRGB `rgb`.
fn srgb([r, g, b]: [f32; 3]) -> Srgba {
    Srgba::rgb(r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0))
}

/// The values a widget shows: those of the solid color, or the mixed value with no solid color
/// (None, a pattern or a "?" proxy), which draws no marker and still takes a pick.
fn bind<'a>(values: Option<ModelValues>) -> Bind<'a, ModelValues> {
    match values {
        Some(v) => Bind::given(v),
        None => Bind::given(Mixed::Mixed),
    }
}

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

/// The Show Color Vision Preview row: four chips across the panel with `color` as each color
/// vision deficiency shows it, named in the tooltip and to screen readers; the hex value of a solid
/// color is the chip's value. A click or a key on a chip does nothing. With no solid color the chips
/// are empty and keep their names.
fn vision_row(ui: &mut Ui, cms: &Cms, color: Option<&Color>) {
    let colors = color.map(|c| vision_colors(cms, c));
    let entries: Vec<(Label, Srgba)> = vision_names()
        .into_iter()
        .enumerate()
        .map(|(i, name)| (Label::from(name), colors.map_or(Srgba::new(0.0, 0.0, 0.0, 0.0), |cs| srgb(cs[i]))))
        .collect();
    // The chips hold no selection: a click reports one the row ignores.
    let _ = SwatchTable::new(tl!("Color vision preview").to_string(), &entries)
        .per_row(4)
        .span(true)
        .tips(true)
        .selection(Bind::given(Selection::default()))
        .show_all(ui);
}

/// What the HSB picked last belongs to: the document, the selected objects or text characters,
/// the appearance item, the proxy, the gradient stop or freeform point, its paint, the color the
/// pick stored, and the document's undo step. The wheel shows that HSB only while all of them are
/// unchanged.
#[derive(Clone, Debug, PartialEq)]
struct Owner {
    doc: Option<u64>,
    /// The document's [`History::step`](vectorcraft_engine::History::step): another edit, an Undo
    /// or a Redo changes it, also when the same color comes back.
    step: Option<(u64, u64)>,
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
            step: st.map(|d| d.history.step()),
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

/// The components remembered in `slot` for the target's `color`: the display sRGB the color had
/// when they were stored, and the components. A record whose owner or display color differs from
/// what the panel shows now is dropped, so that it does not come back when the same color returns
/// later.
fn recall<T: Clone>(
    slot: &mut Option<(Owner, [f32; 3], T)>,
    app: &VectorcraftApp,
    tgt: &Target,
    color: Option<Color>,
    cms: &Cms,
) -> Option<([f32; 3], T)> {
    let kept = slot.as_ref().and_then(|(owner, shown, v)| match color {
        Some(c) if *owner == Owner::of(app, tgt, c) && cms.display_rgb(&c) == *shown => Some((*shown, v.clone())),
        _ => None,
    });
    if kept.is_none() {
        *slot = None;
    }
    kept
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

/// The message for OKLCH Sliders while the active document's RGB space is not sRGB.
const OKLCH_NEEDS_SRGB: &str = "OKLCH needs sRGB as the working RGB space";

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

/// The color a track shows at OKLCH `lch`: the color itself inside sRGB, and outside it dimmed
/// (the same color at 30 % opacity). A color outside sRGB shows at the largest chroma that fits at
/// its L and H, the color a pick there stores. With `max_chroma` (the C track, whose L and H stay
/// fixed) that chroma is given for the colors past it; without it, or in a gap below it where the
/// ray leaves sRGB and comes back, it is found for each color.
pub(crate) fn oklch_track(lch: [f64; 3], max_chroma: Option<f64>) -> Srgba {
    let exact = oklab::oklch_to_srgb(lch[0], lch[1], lch[2]);
    let (rgb, outside) = match max_chroma {
        Some(m) if lch[1] > m => (oklab::oklch_to_srgb(lch[0], m, lch[2]), true),
        Some(_) if oklab::in_srgb(exact) => (exact, false),
        _ => {
            let (rgb, lowered) = oklab::from_oklch(lch[0], lch[1], lch[2]);
            (rgb.map(f64::from), lowered)
        }
    };
    let [r, g, b] = rgb.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as f32 / 255.0);
    Srgba::new(r, g, b, if outside { 0.3 } else { 1.0 })
}

/// How many colors along a cached track.
const TRACK_SAMPLES: usize = 128;
/// How many colors along an HSL track.
const HSL_STOPS: usize = 13;

/// The OKLCH with row `i` (L, C or H) set to `v` in the row's units: lightness and chroma in
/// percent (chroma of [`CHROMA_FULL`]), hue in degrees.
fn oklch_with(lch: [f64; 3], i: usize, v: f64) -> [f64; 3] {
    let mut out = lch;
    match i {
        0 => out[0] = (v / 100.0).clamp(0.0, 1.0),
        1 => out[1] = (v / 100.0 * CHROMA_FULL).clamp(0.0, CHROMA_FULL),
        _ => out[2] = v.clamp(0.0, 360.0),
    }
    out
}

/// The HSL with row `i` (H, S or L) set to `v`: hue in degrees, saturation and lightness in percent.
fn hsl_with(hsl: [f32; 3], i: usize, v: f64) -> [f32; 3] {
    let mut out = hsl;
    match i {
        0 => out[0] = v.clamp(0.0, 360.0) as f32,
        1 => out[1] = (v / 100.0).clamp(0.0, 1.0) as f32,
        _ => out[2] = (v / 100.0).clamp(0.0, 1.0) as f32,
    }
    out
}

/// The HSL `[h°, s, l]` the rows show for `c` through `cms` when `hsl` was asked for last and `c`
/// then displayed as the sRGB `shown`: `hsl` while `c` still displays as `shown` (the hue of a gray
/// and the saturation of black and white stay), else `c`'s own.
pub(crate) fn remembered_hsl(cms: &Cms, c: &Color, shown: [f32; 3], hsl: [f32; 3]) -> [f32; 3] {
    if cms.display_rgb(c) == shown { hsl } else { rgb_to_hsl(clipped_rgb(cms, c)) }
}

/// What a slider row asked for, kept as the components shown while the color stays.
enum Remembered {
    Hsl([f32; 3]),
    Oklch([f64; 3]),
}

impl Remembered {
    fn remember(self, panel: &mut WheelUi, owner: Owner, shown: [f32; 3]) {
        match self {
            Remembered::Hsl(v) => panel.hsl = Some((owner, shown, v)),
            Remembered::Oklch(v) => panel.oklch = Some((owner, shown, v)),
        }
    }
}

/// What a pick needs to become a color of the target: the target, the color settings, Alt (`behind`),
/// the document's color mode and the HSB the panel showed.
struct Pick<'a> {
    tgt: &'a Target,
    color: Option<Color>,
    cms: &'a Cms,
    behind: bool,
    cmyk_doc: bool,
    shown: [f32; 3],
}

impl Pick<'_> {
    /// Apply the pick of display HSB `hsb` in `phase` and remember it.
    fn hsb(&self, app: &mut VectorcraftApp, panel: &mut WheelUi, hsb: [f32; 3], phase: Live) {
        let c = stored(self.cms, hsb, replaced(app, self.tgt, self.behind).as_ref(), self.cmyk_doc);
        apply(app, self.tgt, c, phase, self.behind);
        if !self.behind {
            panel.hsb = Some((Owner::of(app, self.tgt, c), self.cms.display_rgb(&c), hsb));
        }
    }

    /// Apply the pick of display sRGB `srgb` a slider row asked for as `asked`, in `phase`.
    fn rows(&self, app: &mut VectorcraftApp, panel: &mut WheelUi, srgb: [f32; 3], phase: Live, asked: Remembered) {
        // A hue on a gray, or a saturation on black or white, leaves the displayed color as it is:
        // the rows keep what they were given, and the paint (a Gray color too) stays unchanged.
        let current = self.color.filter(|c| {
            let d = self.cms.display_rgb(c);
            !self.behind && panel.rows_live.is_none() && (0..3).all(|i| (d[i] - srgb[i]).abs() <= 1e-5)
        });
        if let Some(c) = current {
            asked.remember(panel, Owner::of(app, self.tgt, c), self.cms.display_rgb(&c));
            return;
        }
        let c = stored_srgb(self.cms, srgb, replaced(app, self.tgt, self.behind).as_ref(), self.cmyk_doc);
        apply(app, self.tgt, c, phase, self.behind);
        panel.rows_live = (phase == Live::Dragging).then_some(app.ui.color_wheel.sliders);
        if !self.behind {
            asked.remember(panel, Owner::of(app, self.tgt, c), self.cms.display_rgb(&c));
        }
    }

    /// Apply the picks a wheel, field or strip edit carries.
    fn edits(&self, app: &mut VectorcraftApp, panel: &mut WheelUi, edit: &EditResponse<ModelValues>) {
        apply_edits(app, edit, EditPath::Preview, |app, v, phase| self.hsb(app, panel, picked_hsb(v, self.shown), phase));
    }
}

/// The colors of the track of OKLCH row `i`, from the cache while the other two components are
/// unchanged. `max_c` is the largest chroma sRGB holds at the row's L and H.
fn oklch_stops(panel: &mut WheelUi, lch: [f64; 3], i: usize, max: f64, max_c: f64) -> Vec<Srgba> {
    let mut key = lch.map(f64::to_bits);
    key[i] = 0;
    if let Some((k, stops)) = &panel.oklch_tracks[i]
        && *k == key
    {
        return stops.clone();
    }
    let last = (TRACK_SAMPLES - 1) as f64;
    let stops: Vec<Srgba> = (0..TRACK_SAMPLES).map(|s| oklch_track(oklch_with(lch, i, s as f64 / last * max), (i == 1).then_some(max_c))).collect();
    panel.oklch_tracks[i] = Some((key, stops.clone()));
    stops
}

/// The L, C and H rows of OKLCH Sliders for the target's color through the pick's color settings:
/// lightness from 0 to 100 %, chroma from 0 to 100 % of 0.4 (as CSS reads a chroma percentage) and
/// hue from 0 to 360°. Each track shows the colors its slider reaches and dims the part outside
/// sRGB, and a mark on the C track sits at the largest chroma sRGB holds at the row's L and H. The
/// rows are gray with no color to edit and while the working RGB space is not sRGB. The edits apply
/// as picks.
fn oklch_rows(app: &mut VectorcraftApp, panel: &mut WheelUi, ui: &mut Ui, cx: &Pick) {
    let enabled = cx.color.is_some() && cx.cms.rgb_is_srgb();
    let lch = match (cx.color, recall(&mut panel.oklch, app, cx.tgt, cx.color, cx.cms)) {
        (Some(c), Some((shown, lch))) => remembered_oklch(cx.cms, &c, shown, lch),
        (Some(c), _) => oklab::to_oklch(clipped_rgb(cx.cms, &c)),
        (None, _) => [0.0; 3],
    };
    // Each row: its letter, its name, the end of its range and its unit.
    let rows = [
        ("L", tl!("Lightness").to_string(), 100.0, ChannelUnit::Percent),
        ("C", tl!("Chroma").to_string(), 100.0, ChannelUnit::Percent),
        ("H", tl!("Hue").to_string(), 360.0, ChannelUnit::Degrees),
    ];
    let shown = [lch[0] * 100.0, lch[1] / CHROMA_FULL * 100.0, lch[2]];
    // The C track's L and H stay fixed, so its largest chroma is found once per frame.
    let max_c = if enabled { oklab::max_chroma(lch[0], lch[2]) } else { 0.0 };
    let marks = [(max_c / CHROMA_FULL * 100.0).clamp(0.0, 100.0)];
    // The rows read dim and take no edit without a colour to edit.
    Stack::new().enabled(enabled).show(ui, |ui, _| {
        for (i, &(letter, ref name, max, unit)) in rows.iter().enumerate() {
            let rule = super::whole_steps(0.0..=max);
            let mut row = ChannelRow::new(name.clone(), Bind::given(shown[i].clamp(0.0, max)), rule).short(letter).unit(unit);
            if enabled {
                row = row.track(oklch_stops(panel, lch, i, max, max_c));
            }
            if i == 1 && enabled {
                row = row.marks(&marks);
            }
            if let Some(edit) = row.show(ui) {
                apply_edits(app, &edit, EditPath::Preview, |app, v, phase| {
                    let lch = oklch_with(lch, i, *v);
                    cx.rows(app, panel, oklab::from_oklch(lch[0], lch[1], lch[2]).0, phase, Remembered::Oklch(lch));
                });
            }
        }
    });
}

/// The H, S and L rows of HSL Sliders for the target's color through the pick's color settings: hue
/// from 0 to 360°, saturation and lightness from 0 to 100 %, CSS Color 4's HSL of the display sRGB.
/// Each track shows the colors its slider reaches. The rows are gray with no color to edit. The
/// edits apply as picks.
fn hsl_rows(app: &mut VectorcraftApp, panel: &mut WheelUi, ui: &mut Ui, cx: &Pick) {
    let enabled = cx.color.is_some();
    let hsl = match (cx.color, recall(&mut panel.hsl, app, cx.tgt, cx.color, cx.cms)) {
        (Some(c), Some((shown, hsl))) => remembered_hsl(cx.cms, &c, shown, hsl),
        (Some(c), _) => rgb_to_hsl(clipped_rgb(cx.cms, &c)),
        (None, _) => [0.0; 3],
    };
    // Each row: its letter, its name, the end of its range and its unit. The names start with "HSL"
    // so that they differ from the box strip's HSB Hue and Saturation.
    let hsl_name = |name: &str| format!("HSL {name}");
    let rows = [
        ("H", hsl_name(tl!("Hue")), 360.0, ChannelUnit::Degrees),
        ("S", hsl_name(tl!("Saturation")), 100.0, ChannelUnit::Percent),
        ("L", hsl_name(tl!("Lightness")), 100.0, ChannelUnit::Percent),
    ];
    let shown = [f64::from(hsl[0]), f64::from(hsl[1]) * 100.0, f64::from(hsl[2]) * 100.0];
    // The rows read dim and take no edit without a colour to edit.
    Stack::new().enabled(enabled).show(ui, |ui, _| {
        for (i, &(letter, ref name, max, unit)) in rows.iter().enumerate() {
            let rule = super::whole_steps(0.0..=max);
            let mut row = ChannelRow::new(name.clone(), Bind::given(shown[i].clamp(0.0, max)), rule).short(letter).unit(unit);
            if enabled {
                let last = (HSL_STOPS - 1) as f64;
                row = row.track((0..HSL_STOPS).map(|s| srgb(hsl_to_rgb(hsl_with(hsl, i, s as f64 / last * max)))).collect());
            }
            if let Some(edit) = row.show(ui) {
                apply_edits(app, &edit, EditPath::Preview, |app, v, phase| {
                    let hsl = hsl_with(hsl, i, *v);
                    cx.rows(app, panel, hsl_to_rgb(hsl), phase, Remembered::Hsl(hsl));
                });
            }
        }
    });
}

/// The opacity (0 to 1) the row shows for `tgt` and whether the row can set it: the selected
/// gradient stop's or freeform point's (the first when none is selected, as [`wheel_target`]
/// picks it), else that of the fill or stroke item
/// ([`vectorcraft_engine::Session::proxy_opacity`]), which nothing selected and type characters
/// do not have. A stop or point of a gradient on type characters belongs to the gradient, so the
/// row sets it. With nothing selected, or while the Live Paint Bucket paints
/// ([`vectorcraft_engine::Session::bucket_paints`]), the stop or point belongs to the defaults for
/// new art, which the row leaves alone, and an object selected in a symbol instance has no opacity
/// the row can set.
fn shown_opacity(app: &VectorcraftApp, tgt: &Target) -> (f32, bool) {
    let stroke = !app.session.fill_active;
    let selected = !app.session.bucket_paints() && app.session.active().is_some_and(|d| !d.selection.is_empty() && d.selection.symbol_part.is_none());
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
            super::live_run(app, "Opacity", "paint.freeform.setPoint", p, phase);
        }
        _ => {
            let cmd = super::proxy_cmd(app, false);
            super::live_run(app, "Opacity", cmd, json!({"opacity": f64::from(o) * 100.0}), phase);
        }
    }
}

/// End the drag of the Show Opacity row when the row is hidden (`ui.colorWheel {opacity: false}`)
/// or turned gray before the release: the preview is kept as one undo step and the interaction
/// ends.
fn end_opacity_drag(app: &mut VectorcraftApp, panel: &mut WheelUi) {
    if !panel.opacity_live {
        return;
    }
    if app.session.in_interaction()
        && let Err(e) = app.session.commit_interaction()
    {
        app.status(e.to_string());
    }
    app.sync_views();
    panel.opacity_live = false;
}

/// The Show Opacity row: a slider whose track shows `color` through `cms` from clear to solid over
/// the transparency checkerboard, and a percent field. Gray when [`shown_opacity`] finds nothing
/// to set.
fn opacity_row(app: &mut VectorcraftApp, panel: &mut WheelUi, ui: &mut Ui, tgt: &Target, color: &Color, cms: &Cms) {
    let (o, live) = shown_opacity(app, tgt);
    let [r, g, b] = cms.display_rgb(color).map(|v| v.clamp(0.0, 1.0));
    let track = vec![Srgba::new(r, g, b, 0.0), Srgba::new(r, g, b, 1.0)];
    // The row reads dim and takes no edit with nothing it can set.
    Stack::new().enabled(live).show(ui, |ui, _| {
        let rule = super::whole_steps(0.0..=100.0);
        let row = ChannelRow::new(tl!("Opacity").to_string(), Bind::given(f64::from(o) * 100.0), rule)
            .unit(ChannelUnit::Percent)
            .track(track.clone())
            .checker(true);
        if let Some(edit) = row.show(ui)
            && live
        {
            apply_edits(app, &edit, EditPath::Preview, |app, v, phase| {
                apply_opacity(app, tgt, (*v / 100.0) as f32, phase);
                panel.opacity_live = phase == Live::Dragging;
            });
        }
    });
    // A drag whose row turned gray reports no release.
    if !live {
        end_opacity_drag(app, panel);
    }
}

/// The wheel: the hue ring around the saturation and brightness square, with Show Harmony's markers
/// on the ring. A click on a marker applies its color; a press held on one keeps the ring from
/// picking.
fn show_wheel(app: &mut VectorcraftApp, panel: &mut WheelUi, ui: &mut Ui, cx: &Pick, set: &Settings) {
    let marked = cx.color.is_some();
    let others: Vec<[f32; 3]> = if set.harmony && marked { set.harmony_rule.apply_hsb(cx.shown).into_iter().skip(1).collect() } else { Vec::new() };
    let markers: Vec<Srgba> = others.iter().map(|&hsb| srgb(hsb_to_rgb(hsb))).collect();
    let names: Vec<Label> = others.iter().map(|_| Label::from(tl!("Harmony color").to_string())).collect();
    let mut wheel = ColorWheel::new(tl!("Color wheel").to_string(), bind(marked.then(|| hsb_values(cx.shown))))
        .inner(WheelInner::Square)
        .markers(&markers)
        .marker_names(names);
    if set.gamut {
        wheel = wheel.gamut(gamut(app));
    }
    let out = wheel.show(ui);
    match out.picked.and_then(|i| others.get(i)) {
        Some(&hsb) => cx.hsb(app, panel, hsb, Live::Released),
        None => cx.edits(app, panel, &out.edit),
    }
}

/// The box: the field of channel `set.channel` and its strip.
fn show_box(app: &mut VectorcraftApp, panel: &mut WheelUi, ui: &mut Ui, cx: &Pick, set: &Settings) {
    let ch = set.channel;
    let mode = area_mode(ch);
    let values = cx.color.map(|_| box_values(ch, cx.shown));
    let mut area = ColorArea::new(tl!("Color box").to_string(), mode, bind(values));
    if set.gamut {
        area = area.gamut(gamut(app));
    }
    cx.edits(app, panel, &area.show(ui));
    cx.edits(app, panel, &ColorStrip::new(ch.name().to_string(), mode, bind(values)).show(ui));
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    // The panel's state is taken out of the app so that the picks can borrow both.
    let mut panel = std::mem::take(&mut app.ui.color_wheel_panel);
    show_with(app, &mut panel, ui);
    app.ui.color_wheel_panel = panel;
}

fn show_with(app: &mut VectorcraftApp, panel: &mut WheelUi, ui: &mut Ui) {
    let tgt = wheel_target(app, ui);
    let color = tgt.color();
    let cms = cms::active();
    // With None, a pattern or a "?" proxy the widgets show no markers, and a pick applies its color.
    let hsb = match (color, recall(&mut panel.hsb, app, &tgt, color, &cms)) {
        (Some(c), Some((shown, hsb))) => remembered_hsb(&cms, &c, shown, hsb),
        (c, _) => display_hsb(&cms, &c.unwrap_or(app.session.last_solid)),
    };
    let behind = alt_held(ui);
    let cmyk_doc = app.session.active().is_some_and(|d| d.doc.color_mode == ColorMode::Cmyk);
    let cx = Pick { tgt: &tgt, color, cms: &cms, behind, cmyk_doc, shown: hsb };
    let set = app.ui.color_wheel.clone();
    let now = ui.input(|i| i.time);
    // The stack's gap separates the rows.
    Stack::new().show(ui, |ui, _| show_rows(app, panel, ui, &cx, &set, now));
}

/// The panel's rows: the Fill/Stroke proxy beside the box or the wheel, level with its top, then the slider rows,
/// the color vision row and the document colors row that `set` turns on.
fn show_rows(app: &mut VectorcraftApp, panel: &mut WheelUi, ui: &mut Ui, cx: &Pick, set: &Settings, now: f64) {
    Grid::new(&BESIDE).cells(CellAlign::Top).show(ui, |ui, _| {
        super::proxy(app, ui);
        match set.view {
            View::Box => {
                Row::new().fill_width().show(ui, |ui, _| show_box(app, panel, ui, cx, set));
            }
            View::Wheel => show_wheel(app, panel, ui, cx, set),
        }
    });
    if set.opacity {
        let (color, tgt) = (cx.color, cx.tgt);
        let shown = color.unwrap_or(app.session.last_solid);
        let owner = color.map(|c| Owner::of(app, tgt, c));
        opacity_row(app, panel, ui, tgt, &shown, cx.cms);
        // An opacity edit of a gradient stop or point changes the gradient its owner holds, and the
        // HSB picked last stays with the new gradient.
        if let (Some(c), Some(was)) = (color, owner)
            && let Some((remembered, ..)) = &mut panel.hsb
            && *remembered == was
        {
            *remembered = Owner::of(app, tgt, c);
        }
    } else {
        end_opacity_drag(app, panel);
    }
    match set.sliders {
        Sliders::None => {}
        Sliders::Oklch => oklch_rows(app, panel, ui, cx),
        Sliders::Hsl => hsl_rows(app, panel, ui, cx),
    }
    // A row drag whose rows were hidden or replaced (`ui.colorWheel {sliders}`), or turned gray
    // (OKLCH rows after the working RGB space stops being sRGB), reports no release: its preview is
    // kept as one undo step and the interaction ends.
    let sliders = app.ui.color_wheel.sliders;
    if panel.rows_live.is_some_and(|rows| rows != sliders || (rows == Sliders::Oklch && !cx.cms.rgb_is_srgb())) {
        if app.session.in_interaction()
            && let Err(e) = app.session.commit_interaction()
        {
            app.status(e.to_string());
        }
        app.sync_views();
        panel.rows_live = None;
    }
    if set.vision {
        vision_row(ui, cx.cms, cx.color.as_ref());
    }
    if set.document_colors
        && let Some(u) = document_colors_row(app, panel, ui, cx.cms, now)
    {
        apply_used(app, cx.tgt, &u, cx.behind);
    }
}
