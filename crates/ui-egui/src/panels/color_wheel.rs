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
//! Panel menu › Show Hex Field adds a field under the wheel or the box that shows the color's six
//! hex digits, and two more when the opacity that Show Opacity sets rounds to less than `ff`. It
//! takes a hex color of 3, 4, 6 or 8 digits and a CSS `rgb()`, `hsl()` or `oklch()` color
//! (`color.parse`). A typed color is applied in the same way as a pick, and an alpha sets the
//! opacity that Show Opacity sets, whether or not that row shows. Where that row is gray, only the
//! color is applied.
//!
//! Panel menu › Show Contrast adds a row under the wheel or the box with the WCAG 2 contrast ratio
//! (<https://www.w3.org/TR/WCAG22/#dfn-contrast-ratio>) of the color against the color drawn
//! behind the selection, or against a color pinned with Contrast Background › Current Color, and
//! whether it meets AA and AAA for the category picked under Contrast For (`color.contrast`).
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
//! Show Opacity, Show Hex Field, the contrast settings, Show Color Vision Preview and Show Document
//! Colors are kept with the preferences (`ui.colorWheel`).

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
use craft::ui::choice::{Swatch, SwatchSpacing, SwatchTable};
use craft::ui::contract::{ActivateOn, Bind, EditResponse, Props as _, Show as _, ShowContainer as _, Sizing};
use craft::ui::display::{Shade, Text};
use craft::ui::layout::CellAlign;
use craft::ui::layout_components::{Grid, Row, Stack};
use craft::ui::menu::PanelRow;
use craft::ui::runtime::commands::{CommandState, CommandStates};
use craft::ui::runtime::env::Label;
use craft::ui::text::TextField;
use egui::Ui;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_color::cms::{self, Cms, ProofSetup, ProofTarget};
use vectorcraft_color::harmony::{self, Harmony};
use vectorcraft_color::oklab::{self, CHROMA_FULL};
use vectorcraft_color::wcag::{self, Category};
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
    /// Show Hex Field: the field for a hex or CSS color under the wheel or the box.
    #[serde(deserialize_with = "deserialize_flag")]
    pub hex_field: bool,
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
            hex_field: false,
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

/// `ui.colorWheel {view?, channel?, sliders?, gamut?, harmony?, harmonyRule?, opacity?, hexField?,
/// contrast?, contrastFor?, contrastBackground?, vision?, documentColors?}`: show the wheel or the
/// box, pick the box's channel, show OKLCH Sliders, HSL Sliders or neither, show or hide the
/// out-of-gamut shading, show the colors of a harmony rule on the wheel, check or uncheck Show
/// Opacity and Show Hex Field, set the contrast row, its category and its pinned background
/// (`contrastBackground: null` unpins it), and show or hide the color vision row and the Show
/// Document Colors row. Keys left out keep their values, and so do the other keys when null; params
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
    const KEYS: [&str; 13] = [
        "view",
        "channel",
        "sliders",
        "gamut",
        "harmony",
        "harmonyRule",
        "opacity",
        "hexField",
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
                let ids: Vec<&str> = harmony_rules().map(Harmony::id).collect();
                format!("`{key}` must be one of {}", ids.join(", "))
            })?
        }
    }
    if let Some(on) = crate::menus::opt_bool(p, "opacity")? {
        set.opacity = on;
    }
    if let Some(on) = crate::menus::opt_bool(p, "hexField")? {
        set.hex_field = on;
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

fn category_row_id(c: Category) -> String {
    format!("contrastFor.{}", c.id())
}

/// A Contrast For category's menu label in the interface language.
pub(super) fn category_name(c: Category) -> &'static str {
    tl!(c.label())
}

/// Every row of the panel menu with its state for `set`, `srgb` telling whether the active
/// document's RGB space is sRGB and `current` the color the panel edits, which Contrast Background
/// › Current Color pins.
fn items(set: &Settings, srgb: bool, current: Option<Color>) -> Vec<Item> {
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
    out.push(item("hexField".into(), tl!("Show Hex Field"), set.hex_field, true, json!({ "hexField": !set.hex_field })));
    out.push(item("contrast".into(), tl!("Show Contrast"), set.contrast, true, json!({ "contrast": !set.contrast })));
    for c in Category::ALL {
        out.push(item(category_row_id(c), category_name(c), set.contrast_for == c, true, json!({ "contrastFor": c.id() })));
    }
    let pinned = set.contrast_background.is_some();
    out.push(item("contrastBackground.below".into(), tl!("Behind Selection"), !pinned, true, json!({ "contrastBackground": null })));
    let params = current.map_or(Value::Null, |c| json!({ "contrastBackground": crate::panels::color_json(&c) }));
    out.push(item("contrastBackground.current".into(), tl!("Current Color"), pinned, current.is_some(), params));
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
/// Opacity and Show Hex Field; Show Contrast, Contrast For and Contrast Background; Show Color
/// Vision Preview; and Show Document Colors.
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
    rows.extend(["opacity", "hexField", "contrast"].map(|row| command(row.into())));
    rows.push(PanelRow::Submenu(
        Label::from(tl!("Contrast For").to_string()),
        Category::ALL.into_iter().map(|c| command(category_row_id(c))).collect(),
    ));
    rows.push(PanelRow::Submenu(
        Label::from(tl!("Contrast Background").to_string()),
        ["contrastBackground.below", "contrastBackground.current"].into_iter().map(|row| command(row.into())).collect(),
    ));
    rows.extend(["vision", "documentColors"].map(|row| command(row.into())));
    rows
}

/// The menu rows of `app`'s settings, with the color the panel edits for Current Color.
fn app_items(app: &VectorcraftApp) -> Vec<Item> {
    items(&app.ui.color_wheel, cms::active().rgb_is_srgb(), super::color::target_now(app).color())
}

/// The labels, checks and enabled states of [`menu_rows`].
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    for item in app_items(app) {
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
    if row == "contrastBackground.current" && super::color::target_now(app).color().is_none() {
        return Some(Err("Current Color needs a solid color in the active proxy".to_string()));
    }
    let params = app_items(app).into_iter().find(|item| item.id == row)?.params;
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
    /// The color behind the selection, as `#rrggbb`, with the [`BehindKey`] it was read for.
    pub(super) behind: Option<(BehindKey, Option<String>)>,
    /// The contrast row's `color.contrast` answer, with the [`RatioKey`] it was asked for.
    pub(super) ratio: Option<(RatioKey, Option<Value>)>,
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

/// What the color behind the selection depends on: the document and its revision, the selected
/// objects, the picked object of a dynamic symbol instance, the on-screen Appearance of Black and
/// the color settings ([`under`]).
pub(super) type BehindKey = (u64, u64, Vec<NodeId>, Option<Vec<u32>>, String, Under);

/// What the contrast row's answer depends on.
#[derive(Clone, Debug, PartialEq)]
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
    /// Whether the Live Paint Bucket paints, which gives the proxies its paints without a revision.
    pub(super) bucket: bool,
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
fn background(app: &mut VectorcraftApp, panel: &mut WheelUi) -> Option<Color> {
    if let Some(c) = app.ui.color_wheel.contrast_background {
        return Some(c);
    }
    let st = app.session.active()?;
    let key: BehindKey = (st.uid, st.revision, st.selection.objects.clone(), picked_part(st), app.session.prefs.black_on_screen.clone(), under());
    let hex = match &panel.behind {
        Some((k, hex)) if *k == key => hex.clone(),
        _ => {
            // Any opaque color reads the background; only `against` is used.
            let r = app.session.execute("color.contrast", &json!({ "color": "#000000" })).ok();
            let hex = r.and_then(|r| r["against"].as_str().map(str::to_string));
            panel.behind = Some((key, hex.clone()));
            hex
        }
    };
    // `against` is written in working RGB, the form a color takes.
    Color::from_hex(&hex?)
}

/// The `color.contrast` answer against `pinned`, else against the color behind the selection
/// (`against: "below"`), kept per [`RatioKey`].
fn ratio_answer(app: &mut VectorcraftApp, panel: &mut WheelUi, pinned: Option<Color>) -> Option<Value> {
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
        bucket: app.session.tool_paints(!app.session.fill_active),
    };
    match &panel.ratio {
        Some((k, r)) if *k == key => r.clone(),
        _ => {
            let against = pinned.as_ref().map_or_else(|| json!("below"), crate::panels::color_json);
            let r = app.session.execute("color.contrast", &json!({ "against": against })).ok();
            panel.ratio = Some((key, r.clone()));
            r
        }
    }
}

/// The contrast row: the background's chip, the ratio cut off at two decimals and the AA and AAA
/// results for the chosen category; a dash when there is no background or no color to measure.
/// Without a pinned background the row asks `color.contrast` for the color behind the selection
/// (`against: "below"`), also during an interaction, so translucent groups and layers count as the
/// command counts them; the chip then shows the background in the answer. Only when there is no
/// answer is the background read on its own ([`background`]), so the chip still shows it. The chip
/// is a color well named Background whose value is the background's hex value, as the color vision
/// chips are; a click on it does nothing.
fn contrast_row(app: &mut VectorcraftApp, panel: &mut WheelUi, ui: &mut Ui, cms: &Cms) {
    let category = app.ui.color_wheel.contrast_for;
    let pinned = app.ui.color_wheel.contrast_background;
    let answer = ratio_answer(app, panel, pinned);
    // The background the ratio was measured against.
    let measured = answer.as_ref().filter(|_| pinned.is_none()).and_then(|r| r["against"].as_str()).and_then(Color::from_hex);
    let chip = match pinned.or(measured) {
        Some(c) => Some(c),
        None => background(app, panel),
    };
    let mut tip = tl!(
        "WCAG 2.2 contrast minimums: normal text AA 4.5:1 (1.4.3) and AAA 7:1 (1.4.6), large text AA 3:1 and AAA 4.5:1, graphics and controls AA 3:1 (1.4.11)."
    )
    .to_string();
    if pinned.is_none() {
        tip.push(' ');
        tip.push_str(tl!("The background is read at the center of the selection."));
    }
    let ratio = answer.as_ref().and_then(|r| r["ratio"].as_f64());
    Row::new().show(ui, |ui, _| {
        let well = Mixed::Same(chip.map(|c| srgb(cms.display_rgb(&c))));
        let _ = Swatch::optional(tl!("Background").to_string(), well).tooltip(tip.clone()).show(ui);
        let Some(ratio) = ratio else {
            Text::new("—").tooltip(tip.clone()).show(ui);
            return;
        };
        Text::new(wcag::ratio_text(ratio)).tooltip(tip.clone()).show(ui);
        let (aa, aaa) = category.passes(ratio);
        let result = |pass: bool| if pass { tl!("Pass") } else { tl!("Fail") };
        Text::new(format!("AA {}", result(aa))).tooltip(tip.clone()).show(ui);
        if let Some(aaa) = aaa {
            Text::new(format!("AAA {}", result(aaa))).tooltip(tip.clone()).show(ui);
        }
    });
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

/// Move the record in `slot` from owner `was` to owner `now`, when `was` holds it.
fn rekey<T>(slot: &mut Option<(Owner, [f32; 3], T)>, was: &Owner, now: &Owner) {
    if let Some((owner, ..)) = slot
        && owner == was
    {
        *owner = now.clone();
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
/// the document's color mode and the HSB the panel showed. `drawn` is the HSB the wheel or the box
/// was drawn with when the hex field's text changed the color in the same frame: the pick then
/// sets only the components it moved.
struct Pick<'a> {
    tgt: &'a Target,
    color: Option<Color>,
    cms: &'a Cms,
    behind: bool,
    cmyk_doc: bool,
    shown: [f32; 3],
    drawn: Option<[f32; 3]>,
}

/// The values `hsb` gives in `model`, the widgets' HSB or RGB.
fn values_in(model: &'static str, hsb: [f32; 3]) -> ModelValues {
    if model == HsbModel.id() { hsb_values(hsb) } else { rgb_values(hsb) }
}

/// The widget values `v` with each component left as the widget was drawn with `drawn` taken from
/// `now` instead: a pick read after the hex field's text sets only the components it moved, on the
/// color the text left.
fn rebased(v: ModelValues, drawn: [f32; 3], now: [f32; 3]) -> ModelValues {
    let (was, now) = (values_in(v.model, drawn), values_in(v.model, now));
    let mut out = v;
    for (i, slot) in out.values.iter_mut().enumerate().take(3) {
        if (v.values[i] - was.values[i]).abs() <= 1e-3 {
            *slot = now.values[i];
        }
    }
    out
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
        apply_edits(app, edit, EditPath::Preview, |app, v, phase| {
            let v = match self.drawn {
                Some(drawn) => rebased(*v, drawn, self.shown),
                None => *v,
            };
            self.hsb(app, panel, picked_hsb(&v, self.shown), phase);
        });
    }

    /// Apply what the wheel or the box reported in the frame.
    fn picked(&self, app: &mut VectorcraftApp, panel: &mut WheelUi, picked: Picked) {
        if let Some(hsb) = picked.marker {
            self.hsb(app, panel, hsb, Live::Released);
        }
        for edit in &picked.edits {
            self.edits(app, panel, edit);
        }
    }
}

/// What the wheel or the box reported in a frame: the color of a harmony marker clicked, and the
/// edits of the wheel, the field and the strip. They are applied after the hex field's text.
#[derive(Default)]
struct Picked {
    marker: Option<[f32; 3]>,
    edits: Vec<EditResponse<ModelValues>>,
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
    // The components the rows applied in this frame: a value typed in one row's field and committed
    // by a click on another row keeps both.
    let mut now = lch;
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
                    now = oklch_with(now, i, *v);
                    cx.rows(app, panel, oklab::from_oklch(now[0], now[1], now[2]).0, phase, Remembered::Oklch(now));
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
    // The components the rows applied in this frame, as for the OKLCH rows.
    let mut now = hsl;
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
                    now = hsl_with(now, i, *v);
                    cx.rows(app, panel, hsl_to_rgb(now), phase, Remembered::Hsl(now));
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
/// row sets it. With nothing selected, or while the active tool paints with the chosen fill or
/// stroke of the proxy in front ([`vectorcraft_engine::Session::tool_paints`]: the Live Paint
/// Bucket's, the Shape Builder's fill), the stop or point belongs to the defaults for new art,
/// which the row leaves alone, and an object selected in a symbol instance has no opacity the row
/// can set.
fn shown_opacity(app: &VectorcraftApp, tgt: &Target) -> (f32, bool) {
    let stroke = !app.session.fill_active;
    let selected =
        !app.session.tool_paints(stroke) && app.session.active().is_some_and(|d| !d.selection.is_empty() && d.selection.symbol_part.is_none());
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
/// to set. Returns the row's edit when it has events, for [`apply_opacity_edit`] after the hex
/// field's text.
fn opacity_row(app: &mut VectorcraftApp, panel: &mut WheelUi, ui: &mut Ui, tgt: &Target, color: &Color, cms: &Cms) -> Option<EditResponse<f64>> {
    let (o, live) = shown_opacity(app, tgt);
    let [r, g, b] = cms.display_rgb(color).map(|v| v.clamp(0.0, 1.0));
    let track = vec![Srgba::new(r, g, b, 0.0), Srgba::new(r, g, b, 1.0)];
    let mut out = None;
    // The row reads dim and takes no edit with nothing it can set.
    Stack::new().enabled(live).show(ui, |ui, _| {
        let rule = super::whole_steps(0.0..=100.0);
        let row = ChannelRow::new(tl!("Opacity").to_string(), Bind::given(f64::from(o) * 100.0), rule)
            .unit(ChannelUnit::Percent)
            .track(track.clone())
            .checker(true);
        out = row.show(ui).filter(|edit| live && edit.events.iter().next().is_some());
    });
    // A drag whose row turned gray reports no release.
    if !live {
        end_opacity_drag(app, panel);
    }
    out
}

/// Apply the Show Opacity row's `edit` to `tgt`.
fn apply_opacity_edit(app: &mut VectorcraftApp, panel: &mut WheelUi, tgt: &Target, edit: &EditResponse<f64>) {
    apply_edits(app, edit, EditPath::Preview, |app, v, phase| {
        apply_opacity(app, tgt, (*v / 100.0) as f32, phase);
        panel.opacity_live = phase == Live::Dragging;
    });
}

/// The six hex digits of a color's working RGB components (`E67828`).
fn hex_digits(c: &Color) -> String {
    let [r, g, b] = super::color::working_rgb(c).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    format!("{r:02X}{g:02X}{b:02X}")
}

/// The text the hex field shows for `color` and the opacity `o` the row of Show Opacity edits
/// (`None` when it has nothing to edit): six hex digits of the document's RGB numbers, and two more
/// when the opacity rounds to less than `ff`. Empty without a solid color.
pub(crate) fn hex_text(color: Option<&Color>, o: Option<f32>) -> String {
    let Some(c) = color else { return String::new() };
    let mut text = hex_digits(c);
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
            let mut p = json!({"index": index, "color": super::color_json(&c), "stroke": !app.session.fill_active, "keepModel": true});
            if let Some(a) = alpha {
                p["opacity"] = json!(a);
            }
            super::live_run(app, "Color", "paint.freeform.setPoint", p, Live::Released);
        }
        _ => {
            let cmd = super::proxy_cmd(app, false);
            let mut p = json!({"color": super::color_json(&c), "focus": true, "keepModel": true});
            if let Some(a) = alpha {
                p["opacity"] = json!(f64::from(a) * 100.0);
            }
            super::live_run(app, "Color", cmd, p, Live::Released);
        }
    }
}

/// The text the hex field shows for `tgt` and its color, as [`hex_text`] gives it.
fn hex_shown(app: &VectorcraftApp, tgt: &Target, color: Option<&Color>) -> String {
    let (o, live) = shown_opacity(app, tgt);
    hex_text(color, live.then_some(o))
}

/// The hex field's id salt for `tgt`: it names the document, the selected objects or text
/// characters, the object selected in a symbol instance, the appearance item, the proxy and the
/// gradient stop or freeform point, but not the color, so a text typed for one of them is dropped
/// when another one is targeted, even one showing the same hex.
fn hex_field_salt(app: &VectorcraftApp, tgt: &Target) -> u64 {
    let o = Owner::of(app, tgt, Color::rgb8(0, 0, 0));
    let part = app.session.active().and_then(|d| d.selection.symbol_part.as_ref().map(|p| (p.instance, p.path.clone())));
    let mut h = std::collections::hash_map::DefaultHasher::new();
    ("color-wheel-hex", o.doc, o.selection, part, o.text_selection, o.appearance_item, o.fill, o.slot).hash(&mut h);
    h.finish()
}

/// The Show Hex Field row for `tgt`: a field as wide as the row with a `#` before the text,
/// showing `shown`. Returns the text committed with Enter or by moving the focus away.
fn hex_row(app: &VectorcraftApp, ui: &mut Ui, tgt: &Target, shown: &str) -> Option<String> {
    let edit = TextField::new(tl!("Hex or CSS color").to_string(), Bind::given(shown.to_owned()))
        .prefix("#")
        .submits()
        .select_on_focus()
        .id_salt(hex_field_salt(app, tgt))
        .sizing(Sizing::FillWidth)
        .show(ui);
    edit.events.committed().cloned()
}

/// Apply the hex field's committed `text`: it goes through `color.parse` as typed, and a text it
/// refuses shows its error in the status bar and changes nothing. The text's alpha is not applied
/// when `opacity_set` (the opacity row set the opacity in this frame), so a press on that row that
/// commits the text keeps the opacity it set. When the text is the one the field shows
/// (`retyped`), its alpha stands for the opacity the field shows, not the value its two digits
/// round to, so retyping it leaves that opacity as it was.
fn apply_text(app: &mut VectorcraftApp, ui: &Ui, text: &str, opacity_set: bool, retyped: bool) {
    match app.run("color.parse", json!({ "text": text })) {
        Ok(r) => {
            let Ok(rgb) = serde_json::from_value::<Color>(r["color"].clone()) else { return };
            let tgt = wheel_target(app, ui);
            // The document's RGB numbers, in the model a pick would store them in.
            let cmyk_doc = app.session.active().is_some_and(|d| d.doc.color_mode == ColorMode::Cmyk);
            let c = stored_rgb(&cms::active(), rgb, replaced(app, &tgt, false).as_ref(), cmyk_doc);
            let alpha = r["opacity"].as_f64().map(|o| (o / 100.0) as f32).filter(|_| !opacity_set);
            let alpha = alpha.map(|a| if retyped { shown_opacity(app, &tgt).0 } else { a });
            apply_typed(app, &tgt, c, alpha);
        }
        Err(e) => app.status(e),
    }
}

/// The wheel: the hue ring around the saturation and brightness square, with Show Harmony's markers
/// on the ring. A click on a marker applies its color; a press held on one keeps the ring from
/// picking.
fn show_wheel(app: &VectorcraftApp, ui: &mut Ui, cx: &Pick, set: &Settings) -> Picked {
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
        Some(&hsb) => Picked { marker: Some(hsb), edits: Vec::new() },
        None => Picked { marker: None, edits: vec![out.edit] },
    }
}

/// The box: the field of channel `set.channel` and its strip.
fn show_box(app: &VectorcraftApp, ui: &mut Ui, cx: &Pick, set: &Settings) -> Picked {
    let ch = set.channel;
    let mode = area_mode(ch);
    let values = cx.color.map(|_| box_values(ch, cx.shown));
    let mut area = ColorArea::new(tl!("Color box").to_string(), mode, bind(values));
    if set.gamut {
        area = area.gamut(gamut(app));
    }
    let field = area.show(ui);
    let strip = ColorStrip::new(ch.name().to_string(), mode, bind(values)).show(ui);
    Picked { marker: None, edits: vec![field, strip] }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    // The panel's state is taken out of the app so that the picks can borrow both.
    let mut panel = std::mem::take(&mut app.ui.color_wheel_panel);
    show_with(app, &mut panel, ui);
    app.ui.color_wheel_panel = panel;
}

/// The HSB the wheel shows for `tgt`'s `color`: the one picked last while it still applies, else
/// the color's display HSB. With None, a pattern or a "?" proxy, that of the last solid color.
fn shown_hsb(app: &VectorcraftApp, panel: &mut WheelUi, tgt: &Target, color: Option<Color>, cms: &Cms) -> [f32; 3] {
    match (color, recall(&mut panel.hsb, app, tgt, color, cms)) {
        (Some(c), Some((shown, hsb))) => remembered_hsb(cms, &c, shown, hsb),
        (c, _) => display_hsb(cms, &c.unwrap_or(app.session.last_solid)),
    }
}

fn show_with(app: &mut VectorcraftApp, panel: &mut WheelUi, ui: &mut Ui) {
    let tgt = wheel_target(app, ui);
    let color = tgt.color();
    let cms = cms::active();
    // With None, a pattern or a "?" proxy the widgets show no markers, and a pick applies its color.
    let hsb = shown_hsb(app, panel, &tgt, color, &cms);
    let behind = alt_held(ui);
    let cmyk_doc = app.session.active().is_some_and(|d| d.doc.color_mode == ColorMode::Cmyk);
    let cx = Pick { tgt: &tgt, color, cms: &cms, behind, cmyk_doc, shown: hsb, drawn: None };
    let set = app.ui.color_wheel.clone();
    let now = ui.input(|i| i.time);
    // The stack's gap separates the rows.
    Stack::new().show(ui, |ui, _| show_rows(app, panel, ui, &cx, &set, now));
}

/// The panel's rows: the Fill/Stroke proxy beside the box or the wheel, level with its top, then the opacity row, the
/// hex field, the slider rows, the contrast row, the color vision row and the document colors row that `set` turns on.
fn show_rows(app: &mut VectorcraftApp, panel: &mut WheelUi, ui: &mut Ui, cx: &Pick, set: &Settings, now: f64) {
    let (cms, behind, cmyk_doc, hsb) = (cx.cms, cx.behind, cx.cmyk_doc, cx.shown);
    let mut picked = Picked::default();
    Grid::new(&BESIDE).cells(CellAlign::Top).show(ui, |ui, _| {
        super::proxy(app, ui);
        match set.view {
            View::Box => {
                Row::new().fill_width().show(ui, |ui, _| picked = show_box(app, ui, cx, set));
            }
            View::Wheel => picked = show_wheel(app, ui, cx, set),
        }
    });
    // The hex field shows the text of the frame's start, so a text committed when a press on the
    // opacity row takes the focus is kept.
    let hex = set.hex_field.then(|| hex_shown(app, cx.tgt, cx.color.as_ref()));
    let opacity_edit = if set.opacity {
        let shown = cx.color.unwrap_or(app.session.last_solid);
        opacity_row(app, panel, ui, cx.tgt, &shown, cms)
    } else {
        end_opacity_drag(app, panel);
        None
    };
    let typed = hex.and_then(|hex| {
        hex_row(app, ui, cx.tgt, &hex).map(|text| {
            let shown = text.strip_prefix('#').unwrap_or(&text).eq_ignore_ascii_case(&hex);
            (text, shown)
        })
    });
    // A text committed by a press on the wheel, the box or the opacity row is applied first; the
    // opacity edit then goes to the target as the text left it, and a pick to the target as both
    // left it, so a stop keeps all of them.
    if let Some((text, shown)) = &typed {
        apply_text(app, ui, text, opacity_edit.is_some(), *shown);
    }
    let after_text;
    let tgt = if typed.is_some() {
        after_text = wheel_target(app, ui);
        &after_text
    } else {
        cx.tgt
    };
    if let Some(edit) = &opacity_edit {
        let owner = tgt.color().map(|c| (c, Owner::of(app, tgt, c)));
        apply_opacity_edit(app, panel, tgt, edit);
        // An opacity edit adds an undo step, and on a gradient stop or point it also changes the
        // gradient its owner holds. The HSB, HSL and OKLCH picked last stay with the edited paint.
        if let Some((c, was)) = owner {
            let moved = Owner::of(app, tgt, c);
            rekey(&mut panel.hsb, &was, &moved);
            rekey(&mut panel.hsl, &was, &moved);
            rekey(&mut panel.oklch, &was, &moved);
        }
    }
    let changed = typed.is_some() || opacity_edit.is_some();
    let after_opacity;
    let tgt = if opacity_edit.is_some() {
        after_opacity = wheel_target(app, ui);
        &after_opacity
    } else {
        tgt
    };
    let color = if changed { tgt.color() } else { cx.color };
    // After a text that changed the color, a pick sets the components it moved on the color the
    // text applied; a text that left the color as it was keeps the remembered hue.
    let shown = if changed { shown_hsb(app, panel, tgt, color, cms) } else { hsb };
    let drawn = (typed.is_some() && shown != hsb).then_some(hsb);
    let any_pick = picked.marker.is_some() || picked.edits.iter().any(|e| e.events.iter().next().is_some());
    let pick = Pick { tgt, color, cms, behind, cmyk_doc, shown, drawn };
    pick.picked(app, panel, picked);
    // The slider rows show the target as the picks left it.
    let after_pick;
    let tgt = if any_pick {
        after_pick = wheel_target(app, ui);
        &after_pick
    } else {
        tgt
    };
    let color = if any_pick { tgt.color() } else { color };
    let shown = if any_pick { shown_hsb(app, panel, tgt, color, cms) } else { shown };
    let rows = Pick { tgt, color, cms, behind, cmyk_doc, shown, drawn: None };
    let cx = &rows;
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
    if set.contrast {
        contrast_row(app, panel, ui, cx.cms);
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
