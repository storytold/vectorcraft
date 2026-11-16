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
//! The wheel's square and the box's field then draw lines where the color picked there would meet
//! AA and AAA (AAA dashed), and when the color fails, a chip in the row offers the closest color
//! that passes, the one `color.contrast` answers as `fix`.

use std::collections::HashMap;

use egui::{Color32, Rect, Stroke, Ui, UiBuilder, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_color::cms::{self, Cms};
use vectorcraft_color::wcag::{self, Category};
use vectorcraft_color::{Color, Paint, hsb_to_rgb, rgb_to_hsb};
use vectorcraft_doc::{ColorMode, NodeId};
use vectorcraft_engine::cmd::{Backdrop, contrast_measure};

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
/// and its phase, and the field's rect.
fn show_box(ui: &mut Ui, ch: Channel, hsb: [f32; 3], marked: bool, size: f32) -> (Option<([f32; 3], Live)>, Rect) {
    let (z, x, y) = box_pos(ch, hsb);
    let mut pick = None;
    let mut field_rect = Rect::NOTHING;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = STRIP_GAP;
        let field = vec2((size - STRIP_GAP - STRIP).max(1.0), size);
        let at = |x: f32, y: f32| widgets::hsb32(box_pick(ch, z, x, y, hsb));
        let picked = widgets::color_field(ui, "color-wheel-box", (tl!("Color box"), ch.field_axes()), field, (x, y), marked, &at);
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
/// What the contrast row's answer depends on: the background, the active proxy's paint (with nothing
/// selected it is the default for new art, which no revision counts), the document, its revision,
/// the selection and its picked symbol object, the proxy, the appearance item, the selected
/// gradient stop and freeform point, the on-screen Appearance of Black, the color settings and the
/// Contrast For category, which the closest passing colors are for.
pub(super) type RatioKey =
    (String, String, Option<(u64, u64)>, Vec<NodeId>, Option<Vec<u32>>, bool, Option<usize>, Option<usize>, Option<usize>, String, Under, Category);

/// Panel state: what the contrast row's answer was measured against, with its [`RatioKey`].
const BACKDROP: &str = "color-wheel-backdrop";
/// Panel state: the contrast lines of the square or the field, with the [`LinesKey`] they were
/// found for.
pub(super) const LINES: &str = "color-wheel-lines";
/// What the contrast lines depend on: what the color is measured against, the category, the field
/// (the view, the channel and the component the field holds fixed) with the model a pick there
/// stores, and the color settings.
pub(super) type LinesKey = (Backdrop, Category, String, Under);
/// Per minimum of the category (AA, then AAA where it has one), the lines in field coordinates.
pub(super) type Lines = Vec<Vec<Vec<(f32, f32)>>>;

/// The columns and rows at which [`contrast_lines`] samples a field.
const LINE_SAMPLES: usize = 32;
/// The halvings that place a line on the edge between two samples.
const LINE_STEPS: usize = 10;

/// A point of a field: x across, y down, 0 to 1 each.
type FieldPoint = (f32, f32);

/// The passing bands found between the samples of one column or row: where each peaks along it, and
/// its two crossings.
type Bands = Vec<(f32, FieldPoint, FieldPoint)>;

/// Where the contrast `ratio_at` a field point crosses each of `minimums`: per minimum, polylines in
/// field coordinates. The field is sampled on a 33 × 33 grid, and each cell whose corners fall on
/// both sides gets a segment between the crossings on its edges, placed by bisection (marching
/// squares; a cell with two opposite corners passing is split by its center). Around a failing
/// sample higher than its neighbors in a column or a row, the highest ratio is looked for, so a
/// passing band narrower than the samples gets its two edges too, joined from column to column or
/// from row to row. Segments that share an end are joined into one polyline.
pub(super) fn contrast_lines(ratio_at: &dyn Fn(f32, f32) -> f64, minimums: &[f64]) -> Lines {
    let n = LINE_SAMPLES;
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
            for across in [false, true] {
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
/// of the chosen category against `backdrop`.
fn lines_for(app: &VectorcraftApp, ui: &Ui, backdrop: &Backdrop, tgt: &Target, hsb: [f32; 3], cmyk_doc: bool) -> Lines {
    let set = &app.ui.color_wheel;
    let (category, view, ch) = (set.contrast_for, set.view, set.channel);
    let was = replaced(app, tgt, false);
    // The component the field holds fixed: the box's strip value, or the wheel's hue.
    let fixed = if view == View::Box { box_pos(ch, hsb).0 } else { hsb[0] };
    let field = format!("{view:?} {ch:?} {fixed} {:?} {cmyk_doc}", was.map(|c| c.model()));
    let key: LinesKey = (backdrop.clone(), category, field, under());
    if let Some((k, lines)) = pstate::<Option<(LinesKey, Lines)>>(ui.ctx(), LINES)
        && k == key
    {
        return lines;
    }
    let cms = cms::active();
    let at = |x: f32, y: f32| if view == View::Box { box_pick(ch, fixed, x, y, hsb) } else { [hsb[0], x, 1.0 - y] };
    let ratio_at = |x: f32, y: f32| backdrop.ratio(cms.display_rgb(&backdrop.drawn_color(&stored(&cms, at(x, y), was.as_ref(), cmyk_doc))));
    let (aa, aaa) = category.minimums();
    let minimums: Vec<f64> = std::iter::once(aa).chain(aaa).collect();
    let lines = contrast_lines(&ratio_at, &minimums);
    set_pstate(ui.ctx(), LINES, Some((key, lines.clone())));
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

/// The contrast row's measurement: the background it was measured against, `color.contrast`'s
/// answer for the chosen category, and what the lines measure picks against.
struct Measured {
    against: Option<Color>,
    answer: Option<Value>,
    backdrop: Option<Backdrop>,
}

/// Measure the active color for the contrast row and the lines. Without a pinned background it
/// asks `color.contrast` for the color behind the selection (`against: "below"`), also during an
/// interaction, so translucent groups and layers count as the command counts them. The answer is
/// kept per [`RatioKey`].
fn measure(app: &mut VectorcraftApp, ui: &Ui) -> Measured {
    let category = app.ui.color_wheel.contrast_for;
    let pinned = app.ui.color_wheel.contrast_background.is_some();
    let against = background(app, ui);
    let sampled = !pinned;
    let mut backdrop = None;
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
            category,
        );
        let kept = pstate::<Option<(RatioKey, Option<Value>)>>(ui.ctx(), RATIO);
        let kept_backdrop = pstate::<Option<(RatioKey, Option<Backdrop>)>>(ui.ctx(), BACKDROP);
        match (kept, kept_backdrop) {
            (Some((k, r)), Some((kb, b))) if k == key && kb == key => {
                backdrop = b;
                r
            }
            _ => {
                let against = if sampled { json!("below") } else { color_json(bg) };
                let m = contrast_measure(&app.session, &json!({ "against": against, "for": category.id() })).ok();
                let (r, b) = m.map_or((None, None), |(r, b)| (Some(r), Some(b)));
                set_pstate(ui.ctx(), RATIO, Some((key.clone(), r.clone())));
                set_pstate(ui.ctx(), BACKDROP, Some((key, b.clone())));
                backdrop = b;
                r
            }
        }
    });
    Measured { against, answer, backdrop }
}

/// The contrast row: the background's chip, the ratio cut off at two decimals, the AA and AAA
/// results for the chosen category, and a chip of the closest color that passes the first level the
/// color fails; a dash when there is no background or no color to measure. Without a pinned
/// background the first chip shows the background in the answer, else the pinned color or the one
/// kept by [`background`]. The fix chip keeps its space while hidden. Returns the fix color when its
/// chip is clicked.
fn contrast_row(app: &VectorcraftApp, ui: &mut Ui, m: &Measured) -> Option<Color> {
    let category = app.ui.color_wheel.contrast_for;
    let pinned = app.ui.color_wheel.contrast_background.is_some();
    let sampled = !pinned;
    let (against, answer) = (m.against, m.answer.as_ref());
    let mut clicked = None;
    let resp = ui
        .horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(CHIP, CHIP), egui::Sense::hover());
            // The chip shows the background the ratio was measured against.
            let measured = answer.filter(|_| sampled).and_then(|r| r["against"].as_str()).and_then(Color::from_hex);
            match measured.or(against) {
                Some(c) => widgets::swatch_tile(ui, rect, &Paint::solid(c), false, false),
                None => widgets::swatch_tile(ui, rect, &Paint::None, false, false),
            }
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
    if !pinned {
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
    let measured = app.ui.color_wheel.contrast.then(|| measure(app, ui));
    let mut pick: Option<([f32; 3], Live)> = None;
    let mut field_rect = Rect::NOTHING;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(44.0);
            super::proxy(app, ui, 40.0);
        });
        let size = ui.available_width().min(MAX_WHEEL);
        if app.ui.color_wheel.view == View::Box {
            (pick, field_rect) = show_box(ui, app.ui.color_wheel.channel, hsb, marked, size);
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
    let cmyk_doc = app.session.active().is_some_and(|d| d.doc.color_mode == ColorMode::Cmyk);
    if let Some(m) = &measured {
        if let Some(backdrop) = &m.backdrop {
            let lines = lines_for(app, ui, backdrop, &tgt, hsb, cmyk_doc);
            draw_contrast_lines(ui, field_rect, &lines);
        }
        if let Some(c) = contrast_row(app, ui, m) {
            apply(app, &tgt, c, Live::Released, false);
        }
    }
    let Some((hsb, phase)) = pick else { return };
    let behind = super::alt_held(ui);
    let c = stored(&cms, hsb, replaced(app, &tgt, behind).as_ref(), cmyk_doc);
    apply(app, &tgt, c, phase, behind);
    if !behind {
        set_pstate(ui.ctx(), MEMORY, Some((Owner::of(app, &tgt, c), cms.display_rgb(&c), hsb)));
    }
}
