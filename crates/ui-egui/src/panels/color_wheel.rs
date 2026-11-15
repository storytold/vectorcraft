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
//! The panel menu's Show Document Colors adds a row under the wheel or the box with the colors
//! of the selection, or with nothing selected of the art Select All takes, most used first
//! (`color.used`). A click on one applies it as a pick does. The active proxy and a selected
//! gradient stop keep its swatch link; a freeform point takes the color alone.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use egui::{Rect, Sense, Ui, UiBuilder, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_color::cms::{self, Cms};
use vectorcraft_color::{Color, Paint, hsb_to_rgb, rgb_to_hsb};
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
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    #[serde(deserialize_with = "deserialize_view")]
    pub view: View,
    #[serde(serialize_with = "serialize_channel", deserialize_with = "deserialize_channel")]
    pub channel: Channel,
    /// Show Document Colors: the row of the colors the art uses, under the wheel or the box.
    #[serde(deserialize_with = "deserialize_flag")]
    pub document_colors: bool,
}

fn deserialize_flag<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(<Value as Deserialize>::deserialize(d)?.as_bool().unwrap_or_default())
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

/// `ui.colorWheel {view?, channel?, documentColors?}`: show the wheel or the box, pick the box's
/// channel, and show or hide the Show Document Colors row. Keys
/// left out keep their values; params that are not an object, another key or a value off the list
/// are an error and change nothing. → the settings.
pub(crate) fn command(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    const KEYS: [&str; 3] = ["view", "channel", "documentColors"];
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
    if let Some(on) = crate::menus::opt_bool(p, "documentColors")? {
        set.document_colors = on;
    }
    app.ui.color_wheel = set;
    Ok(json!(app.ui.color_wheel))
}

/// The panel menu: Wheel View and Box View, and Box Channel while the box shows.
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
    let behind = super::alt_held(ui);
    if app.ui.color_wheel.document_colors
        && let Some(u) = document_colors_row(app, ui)
    {
        apply_used(app, &tgt, &u, behind);
        return;
    }
    let Some((hsb, phase)) = pick else { return };
    let cmyk_doc = app.session.active().is_some_and(|d| d.doc.color_mode == ColorMode::Cmyk);
    let c = stored(&cms, hsb, replaced(app, &tgt, behind).as_ref(), cmyk_doc);
    apply(app, &tgt, c, phase, behind);
    if !behind {
        set_pstate(ui.ctx(), MEMORY, Some((Owner::of(app, &tgt, c), cms.display_rgb(&c), hsb)));
    }
}
