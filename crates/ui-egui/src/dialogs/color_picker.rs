//! The Color Picker (double-click a Fill/Stroke proxy, or `ui.colorPicker`): a colour field with a
//! channel slider for H, S, B, R, G or B; HSB, RGB, Lab and CMYK fields and hex; Only Web Colors;
//! the out-of-gamut and out-of-web warnings (click to correct); New/Original chips; an eyedropper
//! button that samples a color anywhere on the screen ([`crate::screen_color`]); and a Color
//! Swatches list. With art selected the colour shows on it live as it is picked; OK keeps it as one
//! undo step through `paint.setFill` / `paint.setStroke`, Cancel puts the art back.
//! Opened for an artboard (Properties › Artboards › Fill), it shows the color on the artboard's
//! paper live, and OK keeps it as one undo step through `artboard.setProps {index, color}`.
//!
//! Fields agents set with `ui.dialog.set`: `hex` (`"00FF00"`) or `color` (any colour the paint
//! commands take), `channel` (`hue`, `saturation`, `brightness`, `red`, `green`, `blue`), `webOnly`
//! and `swatches`. `stroke`, `artboard`, `keep` and `original` come from the opener.

use egui::{Rect, Sense, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_color::cms::{self, Lab};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::ColorMode;
use vectorcraft_engine::cmd::color_value;

use super::form::PREVIEWED;
use super::{DialogAction, DialogSpec};
use crate::panels::color::{GAMUT_WARNING, WEB_WARNING, gamut_fix, hex_digits, is_web_safe, parse_hex, warning_chip, web_safe};
use crate::panels::{c32, color_json};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

pub(super) const KIND: &str = "colorPicker";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Color Picker").into(),
    header: Some(header),
    body_with_actions: Some(body),
    confirm,
    preview: true,
    min_width: 0.0,
    ..DialogSpec::FORM
};

/// The preview and actions share the top row; the fields span both columns beneath it.
const HEADER_H: f32 = 30.0;
const CHIP_H: f32 = 64.0;
const ACTION_GAP: f32 = 2.0;
const ACTIONS_H: f32 = 3.0 * 28.0 + 2.0 * ACTION_GAP;
const ROW_H: f32 = 26.0;
const VALUE_GAP: f32 = 4.0;
const ROW_GAP: f32 = 1.0;
const GROUP_GAP: f32 = 2.0;
const PREVIEW_GAP: f32 = 12.0;
const FIELD: f32 = ACTIONS_H + PREVIEW_GAP + 7.0 * ROW_H + 6.0 * ROW_GAP + GROUP_GAP;
const MIN_FIELD: f32 = 160.0;
// channel_slider reserves 6 points on each side for its arrows: the painted ramp is 20 wide.
const SLIDER_W: f32 = 32.0;
const SLIDER_GAP: f32 = 16.0;
const COLUMN_GAP: f32 = 12.0;
// Leave room for two warning chips below the preview, instead of an empty side column.
const PREVIEW_W: f32 = 88.0;
/// Matches the outer width of the existing hex field.
const VALUE_W: f32 = 74.0;

/// The component the channel slider edits; the colour field shows the other two.
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
    fn label(self) -> &'static str {
        match self {
            Channel::Hue => "H",
            Channel::Saturation => "S",
            Channel::Brightness | Channel::Blue => "B",
            Channel::Red => "R",
            Channel::Green => "G",
        }
    }
    fn is_hsb(self) -> bool {
        matches!(self, Channel::Hue | Channel::Saturation | Channel::Brightness)
    }
    /// Indices of (slider, field x, field y) into the channel's space: [h, s, b] or [r, g, b].
    fn axes(self) -> (usize, usize, usize) {
        match self {
            Channel::Hue => (0, 1, 2),
            Channel::Saturation => (1, 0, 2),
            Channel::Brightness => (2, 0, 1),
            Channel::Red => (0, 2, 1),
            Channel::Green => (1, 2, 0),
            Channel::Blue => (2, 0, 1),
        }
    }
}

/// The picked colour with its HSB (hue in degrees), kept so the hue survives grey colours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Picked {
    pub color: Color,
    pub hsb: [f32; 3],
}

impl Picked {
    pub fn of(color: Color) -> Self {
        Self { color, hsb: color.to_hsb() }
    }
    fn from_hsb(hsb: [f32; 3]) -> Self {
        Self { color: Color::from_hsb(hsb[0], hsb[1], hsb[2]), hsb }
    }
    /// The nearest web colour when Only Web Colors is on.
    pub fn snapped(self, web: bool) -> Self {
        if web && !is_web_safe(&self.color) { Self::of(web_safe(&self.color)) } else { self }
    }
}

/// The slider value and the field point (x right, y down; 0..1 each) of a colour.
pub fn field_pos(ch: Channel, p: &Picked) -> (f32, f32, f32) {
    let v = if ch.is_hsb() { [p.hsb[0] / 360.0, p.hsb[1], p.hsb[2]] } else { p.color.to_rgb() };
    let (z, x, y) = ch.axes();
    (v[z], v[x], 1.0 - v[y])
}

/// The colour at slider value `z` and field point (x, y): the inverse of [`field_pos`].
pub fn field_pick(ch: Channel, z: f32, x: f32, y: f32) -> Picked {
    let (zi, xi, yi) = ch.axes();
    let mut v = [0.0; 3];
    v[zi] = z;
    v[xi] = x;
    v[yi] = 1.0 - y;
    let v = v.map(|c| c.clamp(0.0, 1.0));
    if ch.is_hsb() { Picked::from_hsb([v[0] * 360.0, v[1], v[2]]) } else { Picked::of(Color::rgb(v[0], v[1], v[2])) }
}

/// The channel slider's track at `z`: the full spectrum for hue, else the colour with only the
/// slider's component changing.
pub fn slider_color(ch: Channel, z: f32, x: f32, y: f32) -> Color {
    if ch == Channel::Hue { Color::from_hsb(z * 360.0, 1.0, 1.0) } else { field_pick(ch, z, x, y).color }
}

/// The number and suffix that [`value_fields`] shows in the field of `ch` for `p`, and the field's
/// largest number: H in degrees (to 360), S and B in percent (to 100), and R, G and B from 0 to 255.
fn channel_value(ch: Channel, p: &Picked) -> (f32, &'static str, f32) {
    let [r, g, b] = p.color.to_rgb();
    match ch {
        Channel::Hue => (p.hsb[0], "°", 360.0),
        Channel::Saturation => (p.hsb[1] * 100.0, "%", 100.0),
        Channel::Brightness => (p.hsb[2] * 100.0, "%", 100.0),
        Channel::Red => (r * 255.0, "", 255.0),
        Channel::Green => (g * 255.0, "", 255.0),
        Channel::Blue => (b * 255.0, "", 255.0),
    }
}

/// The colour the dialog holds: an edited `hex` wins over `color` (agents set either one).
fn picked(d: &Dialog) -> Picked {
    let hex = d.str("hex");
    let color = match parse_hex(&hex) {
        Some(c) if hex != d.str("__hex") => c,
        _ => d.fields.get("color").and_then(color_value).unwrap_or(Color::WHITE),
    };
    let hsb = d.fields.get("__hsb").and_then(|v| serde_json::from_value::<[f32; 3]>(v.clone()).ok());
    let p = match hsb {
        Some(h) if Picked::from_hsb(h).color.to_hex() == color.to_hex() => Picked { color, hsb: h },
        _ => Picked::of(color),
    };
    p.snapped(d.bool("webOnly"))
}

fn store(d: &mut Dialog, p: &Picked) {
    let hex = hex_digits(&p.color);
    d.fields.insert("color".into(), color_json(&p.color));
    d.fields.insert("hex".into(), json!(hex));
    d.fields.insert("__hex".into(), json!(hex));
    d.fields.insert("__hsb".into(), json!(p.hsb));
}

/// The colour the proxy shows (a gradient's first stop; the last colour for None and patterns).
fn proxy_color(app: &VectorcraftApp, stroke: bool) -> Color {
    let (f, s) = crate::panels::current_paints(app);
    match if stroke { s } else { f } {
        Paint::Solid { color, .. } => color,
        Paint::Gradient(g) => g.gradient.stops.first().map_or(app.session.last_solid, |s| s.color),
        Paint::None | Paint::Pattern { .. } => app.session.last_solid,
    }
}

/// `ui.colorPicker {stroke?, artboard?, color?}`: open the picker for the fill (or stroke) proxy,
/// or for artboard `artboard`'s background color, starting from `color`, else the proxy's color
/// or the artboard's color (white when it has none).
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let stroke = p.get("stroke").and_then(Value::as_bool).unwrap_or(!app.session.fill_active);
    let artboard = match p.get("artboard") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let index = v.as_u64().and_then(|i| usize::try_from(i).ok());
            let found = index.and_then(|i| Some((i, app.session.active()?.doc.artboards.get(i)?.color)));
            let (i, color) = found.ok_or("no such artboard")?;
            Some((i, color.unwrap_or(Color::WHITE)))
        }
    };
    let color = match (p.get("color"), artboard) {
        (Some(v), _) => color_value(v).ok_or_else(|| format!("bad color {v}"))?,
        (None, Some((_, c))) => c,
        (None, None) => proxy_color(app, stroke),
    };
    let mut d = Dialog::new(
        KIND,
        json!({"stroke": stroke, "original": color_json(&color), "channel": Channel::default().id(), "webOnly": false, "swatches": false}),
    );
    if let Some((i, _)) = artboard {
        d.fields.insert("artboard".into(), json!(i));
        // OK with the artboard's color (or the white that stands in for none) unchanged keeps the
        // artboard as it is.
        if p.get("color").is_none() {
            d.fields.insert("keep".into(), json!(true));
        }
    }
    store(&mut d, &Picked::of(color));
    d.fields.insert(crate::screen_color::PICKER_ID.into(), json!(crate::screen_color::picker_id()));
    app.ui.dialog = Some(d);
    Ok(Value::Null)
}

/// Stable action width in both modes, including translated labels.
fn button_width(ui: &Ui) -> f32 {
    ["OK", "Cancel", "Color Swatches", "Color Models"].into_iter().fold(110.0_f32, |w, label| {
        let text = ui.painter().layout_no_wrap(tl!(label).into(), crate::theme::semibold(12.5), Tokens::get(ui.ctx()).text);
        w.max(text.size().x + 32.0)
    })
}

/// A compact width shared by the heading and body, bounded by the available window width.
fn layout_size(ui: &Ui) -> (f32, f32, f32, bool) {
    let button_w = button_width(ui);
    let values_w = PREVIEW_W + COLUMN_GAP + button_w;
    let fixed_w = SLIDER_GAP + SLIDER_W + COLUMN_GAP + values_w;
    let stacked = ui.available_width() < MIN_FIELD + fixed_w;
    let side = (ui.available_width() - fixed_w).clamp(MIN_FIELD, FIELD);
    (side, values_w, button_w, stacked)
}

/// Full-width, draggable title band using the app's existing theme colours.
fn header(ui: &mut Ui, title: &str) {
    let (side, values_w, _, stacked) = layout_size(ui);
    let field_w = side + SLIDER_GAP + SLIDER_W;
    let width = if stacked { field_w.max(values_w) } else { field_w + COLUMN_GAP + values_w };
    ui.set_width(width.min(ui.available_width()));
    let margin = f32::from(super::MARGIN);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEADER_H - margin), Sense::hover());
    let band = Rect::from_min_max(r.min - vec2(margin, margin), r.max + vec2(margin, 0.0));
    let t = Tokens::get(ui.ctx());
    let corners = ui.visuals().window_corner_radius;
    // The band extends into the window's existing margins, stopping at its outer edges.
    let painter = ui.painter().with_clip_rect(ui.ctx().content_rect());
    painter.rect_filled(band, egui::CornerRadius { sw: 0, se: 0, ..corners }, t.panel_darker);
    painter.hline(band.x_range(), band.bottom(), egui::Stroke::new(1.0, t.divider));
    painter.text(band.center(), egui::Align2::CENTER_CENTER, title, crate::theme::semibold(16.0), t.text);
    super::modal::drag_band(ui, band.bottom());
}

/// Draws the responsive picker and updates its color fields and live preview.
/// Returns the requested action for the shared frame to confirm or cancel.
fn body(app: &mut VectorcraftApp, ui: &mut Ui, d: &mut Dialog) -> DialogAction {
    let channel = Channel::parse(&d.str("channel")).unwrap_or_default();
    let web = d.bool("webOnly");
    let swatches = d.bool("swatches");
    let p = picked(d);
    let original = d.fields.get("original").and_then(color_value).unwrap_or(p.color);
    let gamut = gamut_fix(app, ui.ctx(), "picker-gamut", &p.color);
    let (side, values_w, button_w, stacked) = layout_size(ui);
    let field_w = side + SLIDER_GAP + SLIDER_W;
    let content_w = if stacked { field_w.max(values_w) } else { field_w + COLUMN_GAP + values_w };
    let mut pick: Option<Picked> = None;
    let mut new_channel = None;
    let mut toggle_swatches = false;
    let mut sample = false;
    let mut action = DialogAction::None;
    // Shrink the field first, then stack the columns in narrow windows. Only vertical
    // scrolling is needed, so no controls depend on a horizontal scrollbar.
    let height = (ui.ctx().content_rect().height() - 112.0).max(1.0);
    // Let the auto-sized window grow for the checkbox too, rather than clipping it to the
    // scroll area's previous height while the dialog measures itself.
    let field_h = side + 8.0 + 20.0 + 2.0 * ui.spacing().item_spacing.y;
    let body_height = if stacked { field_h + COLUMN_GAP + FIELD } else { field_h.max(FIELD) };
    let layout = if stacked { egui::Layout::top_down(egui::Align::Min) } else { egui::Layout::left_to_right(egui::Align::Min) };
    egui::ScrollArea::vertical()
        .id_salt("cp-content")
        .max_width(ui.available_width())
        .max_height(height)
        .min_scrolled_height((body_height + 32.0).min(height))
        .show(ui, |ui| {
            ui.set_min_width(content_w);
            // Keep the screen sampler above the field, outside the value grids' Tab order.
            ui.allocate_ui_with_layout(vec2(side, 24.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                sample = crate::screen_color::button(app, ui);
            });
            ui.add_space(4.0);
            ui.with_layout(layout, |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.allocate_ui_with_layout(vec2(side + SLIDER_GAP + SLIDER_W, side), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.horizontal_top(|ui| {
                        if swatches {
                            pick = swatch_list(app, ui, &p.color, side).map(Picked::of);
                        } else {
                            let (z, x, y) = field_pos(channel, &p);
                            let at = |x: f32, y: f32| c32(&field_pick(channel, z, x, y).snapped(web).color);
                            if let (Some((x, y)), _) = widgets::color_field(ui, "cp-field", vec2(side, side), (x, y), &at) {
                                pick = Some(field_pick(channel, z, x, y));
                            }
                            ui.add_space(SLIDER_GAP);
                            let track = |z: f32| c32(&Picked::of(slider_color(channel, z, x, y)).snapped(web).color);
                            let (v, suffix, max) = channel_value(channel, &p);
                            let shown = widgets::SliderValue::field(v as f64, 0.0..=max as f64, suffix, 0);
                            if let (Some(z), _) =
                                widgets::channel_slider_valued(ui, "cp-slider", tl!(channel.label()), Some(shown), z, vec2(SLIDER_W, side), &track)
                            {
                                pick = Some(field_pick(channel, z, x, y));
                            }
                        }
                    });
                    ui.add_space(8.0);
                    if widgets::check(ui, tl!("Only Web Colors"), web, true) {
                        d.fields.insert("webOnly".into(), json!(!web));
                    }
                });
                ui.add_space(COLUMN_GAP);
                ui.allocate_ui_with_layout(vec2(values_w, FIELD), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.spacing_mut().item_spacing = vec2(VALUE_GAP, ROW_GAP);
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.allocate_ui_with_layout(vec2(PREVIEW_W, ACTIONS_H), egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.set_min_size(vec2(PREVIEW_W, ACTIONS_H));
                            if new_original_chips(ui, &p.color, &original) {
                                pick = Some(Picked::of(original));
                            }
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = VALUE_GAP;
                                if let Some(fix) = gamut
                                    && warning_chip(ui, GAMUT_WARNING, &fix)
                                {
                                    pick = Some(Picked::of(fix));
                                }
                                let ws = web_safe(&p.color);
                                if !web && !is_web_safe(&p.color) && warning_chip(ui, WEB_WARNING, &ws) {
                                    pick = Some(Picked::of(ws));
                                }
                            });
                        });
                        ui.add_space(COLUMN_GAP);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = ACTION_GAP;
                            if widgets::primary_button_width(ui, "OK", button_w).clicked() {
                                action = DialogAction::Confirm;
                            }
                            if widgets::secondary_button_width(ui, "Cancel", button_w).clicked() {
                                action = DialogAction::Cancel;
                            }
                            toggle_swatches =
                                widgets::secondary_button_width(ui, if swatches { tl!("Color Models") } else { tl!("Color Swatches") }, button_w)
                                    .clicked();
                        });
                    });
                    ui.add_space(PREVIEW_GAP - ROW_GAP);
                    let (edit, ch) = value_fields(ui, &p, channel);
                    pick = pick.or(edit);
                    new_channel = ch;
                });
            });
        });
    if let Some(ch) = new_channel {
        d.fields.insert("channel".into(), json!(ch.id()));
    }
    if toggle_swatches {
        d.fields.insert("swatches".into(), json!(!swatches));
    }
    if sample && let Err(e) = app.run("ui.sampleScreenColor", json!({"picker": true})) {
        app.status(e);
    }
    store(d, &pick.unwrap_or(p).snapped(d.bool("webOnly")));
    preview(app, d);
    action
}

/// Show the picked colour on the selected art while the picker is open: each new colour replaces
/// the last preview. Nothing previews until the colour first changes, nor without a selection
/// (the colour then only becomes the default paint, on OK).
/// Opened for an artboard, the picker shows the color on that artboard's paper, with or without a
/// selection.
fn preview(app: &mut VectorcraftApp, d: &mut Dialog) {
    let (cmd, params) = paint_command(app, d);
    let first = !app.session.in_interaction();
    let unchanged = d.fields.get(PREVIEWED) == Some(&params) || first && d.fields.get("original").and_then(color_value) == Some(picked(d).color);
    let artboard = d.fields.contains_key("artboard");
    let selected = artboard || app.session.active().is_some_and(|st| !st.selection.objects.is_empty());
    if unchanged || !selected {
        return;
    }
    let _ = app.session.begin_interaction(if artboard {
        "Artboard Options"
    } else if d.bool("stroke") {
        "Stroke Color"
    } else {
        "Fill Color"
    });
    if let Err(e) = app.session.preview(cmd, &params) {
        app.status(e.to_string());
    }
    d.fields.insert(PREVIEWED.into(), params);
}

/// The command that applies the picked colour to the proxy, and its params (as CMYK in a CMYK
/// document). Opened for an artboard, the command sets that artboard's background color.
fn paint_command(app: &VectorcraftApp, d: &Dialog) -> (&'static str, Value) {
    let mut c = picked(d).color;
    if matches!(c, Color::Rgb { .. }) && app.session.active().is_some_and(|s| s.doc.color_mode == ColorMode::Cmyk) {
        let [cy, m, y, k] = c.to_cmyk_managed(cms::active().settings().intent);
        c = Color::cmyk(cy, m, y, k);
    }
    if let Some(i) = d.fields.get("artboard").and_then(Value::as_u64) {
        return ("artboard.setProps", json!({ "index": i, "color": color_json(&c) }));
    }
    (if d.bool("stroke") { "paint.setStroke" } else { "paint.setFill" }, json!({ "color": color_json(&c) }))
}

/// The New (top) and Original (bottom) chips. Returns true when Original is clicked.
fn new_original_chips(ui: &mut Ui, new: &Color, original: &Color) -> bool {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(64.0, CHIP_H), Sense::hover());
    let top = Rect::from_min_max(r.min, egui::pos2(r.right(), r.center().y));
    let bottom = Rect::from_min_max(egui::pos2(r.left(), r.center().y), r.max);
    widgets::paint_chip(ui, top, &Paint::solid(*new));
    widgets::paint_chip(ui, bottom, &Paint::solid(*original));
    ui.painter().rect_stroke(r, 0.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Outside);
    ui.interact(top, ui.id().with("cp-new"), Sense::hover()).on_hover_text(tl!("New"));
    ui.interact(bottom, ui.id().with("cp-original"), Sense::click()).on_hover_text(tl!("Original: click to restore")).clicked()
}

/// The HSB and RGB fields with the channel radios, hex, Lab and CMYK. Returns the edited colour
/// and the newly chosen channel.
///
/// HSB, RGB and hex are in one grid, and Lab and CMYK are in a second grid beside it. Tab goes
/// through the fields in the order they are drawn: H, S, B, R, G, B, hex, then L, a, b, C, M, Y,
/// K, with a stop at each channel's radio button before its field.
fn value_fields(ui: &mut Ui, p: &Picked, channel: Channel) -> (Option<Picked>, Option<Channel>) {
    let t = Tokens::get(ui.ctx());
    let rgb = p.color.to_rgb();
    let lab = p.color.to_lab();
    let cms = cms::active();
    let cmyk = p.color.to_cmyk_managed(cms.settings().intent);
    let mut edit = None;
    let mut chosen = None;
    let spacing = vec2(VALUE_GAP, ROW_GAP);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = spacing.x;
        let left = egui::Grid::new("cp-values").num_columns(2).min_row_height(0.0).spacing(spacing).show(ui, |ui| {
            for i in 0..7 {
                if i == 3 {
                    for _ in 0..2 {
                        ui.allocate_exact_size(vec2(0.0, GROUP_GAP - ROW_GAP), Sense::hover());
                    }
                    ui.end_row();
                }
                if let Some(&ch) = Channel::ALL.get(i) {
                    if widgets::radio(ui, ch.label(), ch == channel, true) {
                        chosen = Some(ch);
                    }
                    let (v, suffix) = match i {
                        0 => (p.hsb[0], "°"),
                        1 | 2 => (p.hsb[i] * 100.0, "%"),
                        _ => (rgb[i - 3] * 255.0, ""),
                    };
                    if let Some(nv) = widgets::plain_field(ui, ("cp-left", i), v as f64, suffix, 0, VALUE_W) {
                        let nv = nv as f32;
                        edit = Some(match i {
                            0..3 => {
                                let mut hsb = p.hsb;
                                hsb[i] = if i == 0 { nv.clamp(0.0, 360.0) } else { (nv / 100.0).clamp(0.0, 1.0) };
                                Picked::from_hsb(hsb)
                            }
                            _ => {
                                let mut c = rgb;
                                c[i - 3] = (nv / 255.0).clamp(0.0, 1.0);
                                Picked::of(Color::rgb(c[0], c[1], c[2]))
                            }
                        });
                    }
                } else if let Some(c) = widgets::hex_field(ui, "cp-hex", &hex_digits(&p.color)).as_deref().and_then(parse_hex) {
                    edit = Some(Picked::of(c));
                }
                ui.end_row();
            }
        });
        egui::Grid::new("cp-values-right").num_columns(2).min_row_height(0.0).spacing(spacing).show(ui, |ui| {
            for i in 0..7 {
                if i == 3 {
                    for _ in 0..2 {
                        ui.allocate_exact_size(vec2(0.0, GROUP_GAP - ROW_GAP), Sense::hover());
                    }
                    ui.end_row();
                }
                let (label, v, suffix) = match i {
                    0 => ("L", lab.l, ""),
                    1 => ("a", lab.a, ""),
                    2 => ("b", lab.b, ""),
                    _ => (["C", "M", "Y", "K"][i - 3], cmyk[i - 3] * 100.0, "%"),
                };
                // The hex field is taller than a value field. K's row takes the height of the hex
                // field's row beside it.
                let height = if i == 6 { (left.response.rect.bottom() - ui.cursor().top()).round() } else { ui.spacing().interact_size.y };
                let size = vec2(ui.available_size_before_wrap().x, height);
                ui.allocate_ui_with_layout(size, egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add_space(12.0);
                    ui.label(egui::RichText::new(label).size(12.5).color(t.text));
                });
                if let Some(nv) = widgets::plain_field(ui, ("cp-right", i), v as f64, suffix, 0, VALUE_W) {
                    let nv = nv as f32;
                    edit = Some(Picked::of(if i < 3 {
                        let mut l = [lab.l, lab.a, lab.b];
                        l[i] = if i == 0 { nv.clamp(0.0, 100.0) } else { nv.clamp(-128.0, 127.0) };
                        cms.from_lab(Lab::new(l[0], l[1], l[2]))
                    } else {
                        let mut k = cmyk;
                        k[i - 3] = (nv / 100.0).clamp(0.0, 1.0);
                        Color::cmyk(k[0], k[1], k[2], k[3])
                    }));
                }
                ui.end_row();
            }
        });
    });
    (edit, chosen)
}

/// The document's colour swatches as a list; returns the clicked swatch's colour.
fn swatch_list(app: &VectorcraftApp, ui: &mut Ui, current: &Color, side: f32) -> Option<Color> {
    let t = Tokens::get(ui.ctx());
    let mut chosen = None;
    ui.allocate_ui(vec2(side + SLIDER_GAP + SLIDER_W, side), |ui| {
        ui.set_min_size(vec2(side + SLIDER_GAP + SLIDER_W, side));
        widgets::list_box(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("cp-swatches").max_height((side - 8.0).max(1.0)).show(ui, |ui| {
                ui.set_width(side + SLIDER_GAP + SLIDER_W - 8.0);
                let Some(st) = app.session.active() else {
                    widgets::dim_label(ui, tl!("Open a document to see its swatches."));
                    return;
                };
                let doc = &st.doc;
                for s in doc.swatches.iter().chain(doc.swatch_groups.iter().flat_map(|g| g.swatches.iter())) {
                    let Paint::Solid { color, .. } = &s.paint else { continue };
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(r, 0.0, t.hover);
                    }
                    let chip = Rect::from_min_size(r.left_center() + vec2(4.0, -7.0), vec2(14.0, 14.0));
                    widgets::swatch_tile(ui, chip, &s.paint, color == current, false);
                    ui.painter().text(
                        egui::pos2(chip.right() + 8.0, r.center().y),
                        egui::Align2::LEFT_CENTER,
                        &s.name,
                        egui::FontId::proportional(12.0),
                        t.text,
                    );
                    if resp.clicked() {
                        chosen = Some(*color);
                    }
                }
            });
        });
    });
    chosen
}

/// OK: apply the colour to the proxy, keeping the live preview as one undo step. For an artboard,
/// OK with the starting color unchanged and nothing previewed closes the picker and leaves the
/// artboard as it was: without a background color, or with its color in its own color model.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    if d.bool("keep") && !app.session.in_interaction() && d.fields.get("original").and_then(color_value) == Some(picked(d).color) {
        app.ui.dialog = None;
        return Ok(Value::Null);
    }
    let (cmd, params) = paint_command(app, d);
    super::form::commit_preview(app, cmd, params)
}

#[cfg(test)]
mod tests {
    use vectorcraft_engine::Session;

    use super::*;
    use crate::panels::color::in_gamut;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
        app
    }

    fn frame(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(Default::default(), |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
    }

    fn fill_hex(app: &VectorcraftApp) -> String {
        crate::panels::current_paints(app).0.color().unwrap().to_hex()
    }

    /// Runs one frame of the dialog with `events` in `ctx`, which keeps the keyboard focus from
    /// frame to frame, and returns the texts drawn with their screen rects.
    fn frame_in(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>) -> Vec<(String, Rect)> {
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
        fn collect(s: &egui::Shape, out: &mut Vec<(String, Rect)>) {
            match s {
                egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect())),
                egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
                _ => {}
            }
        }
        let mut texts = vec![];
        out.shapes.iter().for_each(|c| collect(&c.shape, &mut texts));
        texts
    }

    /// Returns the label of the value field that has the keyboard, which is the letter (or `#`)
    /// nearest to the field's left on its row, and the field's rect. Returns `None` when no text
    /// field has the keyboard.
    fn focused_field(ctx: &egui::Context, texts: &[(String, Rect)]) -> Option<(String, Rect)> {
        let id = ctx.memory(|m| m.focused()).filter(|&id| egui::TextEdit::load_state(ctx, id).is_some())?;
        let field = ctx.read_response(id)?.rect;
        let labels = ["H", "S", "B", "R", "G", "#", "L", "a", "b", "C", "M", "Y", "K"];
        let (label, _) = texts
            .iter()
            .filter(|(t, r)| labels.contains(&t.as_str()) && r.max.x <= field.min.x && field.y_range().contains(r.center().y))
            .max_by(|a, b| a.1.max.x.total_cmp(&b.1.max.x))?;
        Some((label.clone(), field))
    }

    #[test]
    fn field_and_slider_map_back_per_channel() {
        let p = Picked::of(Color::rgb8(51, 153, 204));
        for ch in Channel::ALL {
            let (z, x, y) = field_pos(ch, &p);
            assert_eq!(field_pick(ch, z, x, y).color.to_hex(), "#3399cc", "{ch:?}");
            assert_eq!(Channel::parse(ch.id()), Some(ch));
        }
        // Hue: x is saturation, y brightness (white at the top left, black along the bottom).
        assert_eq!(field_pick(Channel::Hue, 0.0, 0.0, 0.0).color.to_hex(), "#ffffff");
        assert_eq!(field_pick(Channel::Hue, 0.0, 1.0, 0.0).color.to_hex(), "#ff0000");
        assert_eq!(field_pick(Channel::Hue, 0.5, 1.0, 1.0).color.to_hex(), "#000000");
        // Red: the slider is red, x blue and y green (green grows upwards).
        assert_eq!(field_pick(Channel::Red, 1.0, 1.0, 0.0).color.to_hex(), "#ffffff");
        assert_eq!(field_pick(Channel::Red, 0.0, 1.0, 1.0).color.to_hex(), "#0000ff");
        assert_eq!(field_pick(Channel::Green, 1.0, 0.0, 1.0).color.to_hex(), "#00ff00");
        assert_eq!(field_pick(Channel::Blue, 0.0, 1.0, 1.0).color.to_hex(), "#ff0000");
        // The hue slider shows the spectrum whatever the colour; the others vary one component.
        assert_eq!(slider_color(Channel::Hue, 1.0 / 3.0, 0.0, 1.0).to_hex(), "#00ff00");
        assert_eq!(slider_color(Channel::Saturation, 0.0, 0.0, 0.0).to_hex(), "#ffffff");
        // A grey keeps the hue it was picked with.
        let grey = field_pick(Channel::Hue, 0.5, 0.0, 0.5);
        assert_eq!(grey.hsb[0], 180.0);
        assert_eq!(field_pos(Channel::Hue, &grey).0, 0.5);
    }

    #[test]
    fn only_web_colors_snaps() {
        let p = Picked::of(Color::rgb8(230, 120, 40));
        assert_eq!(p.snapped(false), p);
        assert_eq!(p.snapped(true).color.to_hex(), "#ff6633");
        assert!(is_web_safe(&p.snapped(true).color));
        let mut d = Dialog::new(KIND, json!({"color": "#e67828", "webOnly": true}));
        assert_eq!(picked(&d).color.to_hex(), "#ff6633");
        d.fields.insert("webOnly".into(), json!(false));
        assert_eq!(picked(&d).color.to_hex(), "#e67828");
    }

    #[test]
    fn gamut_correction_gives_a_printable_colour() {
        let mut app = app();
        let blue = Color::rgb(0.0, 0.0, 1.0);
        assert!(blue.out_of_gamut());
        let fix = in_gamut(&mut app, &blue).expect("pure blue can't be printed");
        assert!(!fix.out_of_gamut(), "{fix:?}");
        assert!(matches!(fix, Color::Rgb { .. }));
        assert_eq!(in_gamut(&mut app, &Color::rgb8(128, 128, 128)), None);
        // The dialog and the Color panel ask through a cache.
        let ctx = egui::Context::default();
        assert_eq!(gamut_fix(&mut app, &ctx, "picker-gamut", &blue), Some(fix));
        assert_eq!(gamut_fix(&mut app, &ctx, "picker-gamut", &blue), Some(fix));
        assert_eq!(gamut_fix(&mut app, &ctx, "picker-gamut", &Color::rgb8(128, 128, 128)), None);
    }

    /// The selected art takes each colour as it is picked; OK keeps the last as one undo step and
    /// Cancel puts the art back.
    #[test]
    fn the_selection_shows_the_colour_live() {
        let mut app = app();
        let undo = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
        let before = undo(&app);
        app.run("ui.colorPicker", json!({})).unwrap();
        frame(&mut app);
        assert!(!app.session.in_interaction(), "nothing previews before the colour changes");
        for hex in ["ff0000", "00ff00"] {
            app.ui.dialog.as_mut().unwrap().fields.insert("hex".into(), json!(hex));
            frame(&mut app);
            assert_eq!(fill_hex(&app), format!("#{hex}"), "live, before OK");
        }
        super::super::confirm(&mut app).unwrap();
        assert_eq!(fill_hex(&app), "#00ff00");
        assert_eq!(undo(&app), before + 1, "one step");
        // Cancel: back to the colour it had.
        app.run("ui.colorPicker", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("hex".into(), json!("0000ff"));
        frame(&mut app);
        assert_eq!(fill_hex(&app), "#0000ff");
        super::super::cancel(&mut app);
        assert_eq!(fill_hex(&app), "#00ff00");
        assert_eq!(undo(&app), before + 1);
    }

    /// Opened for an artboard (Properties › Artboards › Fill), the picker starts from the artboard's
    /// background color (white without one) and shows each color on its paper live, with nothing
    /// selected. OK keeps the last as one undo step, the art's fill unchanged, and Cancel puts the
    /// artboard back.
    #[test]
    fn an_artboard_shows_its_background_color_live() {
        let mut app = app();
        app.run("select.set", json!({"ids": []})).unwrap();
        app.run("artboard.new", json!({})).unwrap();
        let color = |app: &VectorcraftApp| app.session.doc().unwrap().doc.artboards[1].color.map(|c| c.to_hex());
        let undo = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
        let art = app.session.doc().unwrap().doc.layers.clone();
        let before = undo(&app);
        app.run("ui.colorPicker", json!({"artboard": 1})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("hex"), "FFFFFF", "starts from white paper");
        frame(&mut app);
        assert!(!app.session.in_interaction(), "nothing previews before the color changes");
        for hex in ["ff0000", "00ff00"] {
            app.ui.dialog.as_mut().unwrap().fields.insert("hex".into(), json!(hex));
            frame(&mut app);
            assert_eq!(color(&app), Some(format!("#{hex}")), "live, before OK");
        }
        super::super::confirm(&mut app).unwrap();
        assert_eq!(color(&app).as_deref(), Some("#00ff00"));
        assert_eq!(undo(&app), before + 1, "one step");
        assert_eq!(app.session.doc().unwrap().doc.layers, art, "the art keeps its fill");
        // Cancel: back to the color it had, which the picker starts from.
        app.run("ui.colorPicker", json!({"artboard": 1})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("hex"), "00FF00");
        app.ui.dialog.as_mut().unwrap().fields.insert("hex".into(), json!("0000ff"));
        frame(&mut app);
        assert_eq!(color(&app).as_deref(), Some("#0000ff"));
        super::super::cancel(&mut app);
        assert_eq!(color(&app).as_deref(), Some("#00ff00"));
        assert_eq!(undo(&app), before + 1);
        assert!(app.run("ui.colorPicker", json!({"artboard": 9})).is_err());
    }

    /// OK on a picker opened for an artboard without a background color, with the color left at the
    /// white it starts from, leaves the artboard without one and adds no undo step. A white the
    /// opener passes as `color` is set.
    #[test]
    fn an_untouched_picker_leaves_an_artboard_without_a_color() {
        let mut app = app();
        app.run("artboard.new", json!({})).unwrap();
        let color = |app: &VectorcraftApp| app.session.doc().unwrap().doc.artboards[1].color.map(|c| c.to_hex());
        let undo = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
        let before = undo(&app);
        app.run("ui.colorPicker", json!({"artboard": 1})).unwrap();
        frame(&mut app);
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none(), "OK closes the picker");
        assert_eq!(color(&app), None, "no color");
        assert_eq!(undo(&app), before, "no undo step");
        app.run("ui.colorPicker", json!({"artboard": 1, "color": "#ffffff"})).unwrap();
        frame(&mut app);
        super::super::confirm(&mut app).unwrap();
        assert_eq!(color(&app).as_deref(), Some("#ffffff"));
        assert_eq!(undo(&app), before + 1);
    }

    /// OK on an untouched picker opened for an artboard with an RGB background color in a CMYK
    /// document keeps the RGB color and adds no undo step. A color picked in it is set as CMYK.
    #[test]
    fn an_untouched_picker_keeps_an_artboards_rgb_color_in_a_cmyk_document() {
        let mut app = app();
        app.run("artboard.new", json!({})).unwrap();
        app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
        app.run("artboard.setProps", json!({"index": 1, "color": "#ff0000"})).unwrap();
        let color = |app: &VectorcraftApp| app.session.doc().unwrap().doc.artboards[1].color;
        let undo = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
        let start = color(&app);
        assert!(matches!(start, Some(Color::Rgb { .. })), "set as RGB");
        let before = undo(&app);
        app.run("ui.colorPicker", json!({"artboard": 1})).unwrap();
        frame(&mut app);
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none(), "OK closes the picker");
        assert_eq!(color(&app), start, "the RGB color is kept");
        assert_eq!(undo(&app), before, "no undo step");
        app.run("ui.colorPicker", json!({"artboard": 1})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("hex".into(), json!("00ff00"));
        frame(&mut app);
        super::super::confirm(&mut app).unwrap();
        assert!(matches!(color(&app), Some(Color::Cmyk { .. })), "a picked color is set as CMYK");
        assert_eq!(undo(&app), before + 1);
    }

    /// OK on an untouched picker opened for an artboard with a background color in an RGB
    /// document keeps the color and adds no undo step.
    #[test]
    fn an_untouched_picker_adds_no_undo_step_for_an_artboards_color() {
        let mut app = app();
        app.run("artboard.new", json!({})).unwrap();
        app.run("artboard.setProps", json!({"index": 1, "color": "#ff0000"})).unwrap();
        let color = |app: &VectorcraftApp| app.session.doc().unwrap().doc.artboards[1].color.map(|c| c.to_hex());
        let undo = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
        let before = undo(&app);
        app.run("ui.colorPicker", json!({"artboard": 1})).unwrap();
        frame(&mut app);
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none(), "OK closes the picker");
        assert_eq!(color(&app).as_deref(), Some("#ff0000"));
        assert_eq!(undo(&app), before, "no undo step");
    }

    #[test]
    fn open_set_hex_and_confirm_fills() {
        let mut app = app();
        app.run("ui.colorPicker", json!({})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!(super::super::DialogKind::of(&d.kind), Some(super::super::DialogKind::ColorPicker));
        assert_eq!(d.str("hex"), "FFFFFF", "starts from the fill");
        assert!(!d.bool("stroke"));
        frame(&mut app);
        // `ui.dialog.set {field: "hex"}` then `ui.dialog.confirm`.
        app.ui.dialog.as_mut().unwrap().fields.insert("hex".into(), json!("00ff00"));
        frame(&mut app);
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("hex"), "00FF00");
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(fill_hex(&app), "#00ff00");
        // Without a frame in between, a set hex still wins; `color` works as well.
        app.run("ui.colorPicker", json!({"stroke": true})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("hex".into(), json!("#3366CC"));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(crate::panels::current_paints(&app).1.color().unwrap().to_hex(), "#3366cc");
        // The stroke proxy is in front now: the opener follows it unless told otherwise.
        app.run("ui.colorPicker", json!({})).unwrap();
        assert!(app.ui.dialog.as_ref().unwrap().bool("stroke"));
        app.run("ui.colorPicker", json!({"stroke": false})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("color".into(), json!({"c": 0, "m": 1, "y": 1, "k": 0}));
        super::super::confirm(&mut app).unwrap();
        assert!(matches!(crate::panels::current_paints(&app).0.color(), Some(Color::Cmyk { .. })), "CMYK is kept");
        assert!(app.run("ui.colorPicker", json!({"color": "nope"})).is_err());
        // Colours take every form the paint commands take (CMYK in percent too).
        app.run("ui.colorPicker", json!({"stroke": false, "color": {"c": 0, "m": 100, "y": 100, "k": 0}})).unwrap();
        super::super::confirm(&mut app).unwrap();
        assert_eq!(crate::panels::current_paints(&app).0.color(), Some(Color::cmyk(0.0, 1.0, 1.0, 0.0)));
    }

    /// #1692: Tab goes through one color model's fields before the next: H, S, B, then R, G, B, the
    /// hex field, then L, a, b, then C, M, Y, K. Each Lab or CMYK field is level with the field on
    /// its left.
    #[test]
    fn tab_goes_through_one_color_models_fields_before_the_next() {
        let mut app = app();
        app.run("ui.colorPicker", json!({})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let tab = egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        // The picker opens with the keyboard in H. The loop presses Tab until K has the keyboard
        // and records each stop. A stop is `None` when no text field has the keyboard, as at a
        // channel's radio button.
        let mut texts = vec![];
        for _ in 0..3 {
            texts = frame_in(&ctx, &mut app, vec![]);
        }
        let mut stops = vec![focused_field(&ctx, &texts)];
        while stops.len() < 40 && stops.last().and_then(|s| s.as_ref()).is_none_or(|(label, _)| label != "K") {
            texts = frame_in(&ctx, &mut app, vec![tab.clone()]);
            stops.push(focused_field(&ctx, &texts));
        }
        let names: Vec<Option<&str>> = stops.iter().map(|s| s.as_ref().map(|(label, _)| label.as_str())).collect();
        let fields: Vec<&str> = names.iter().flatten().copied().collect();
        assert_eq!(fields, ["H", "S", "B", "R", "G", "B", "#", "L", "a", "b", "C", "M", "Y", "K"], "{names:?}");
        assert_eq!(names[names.len() - 2..], [Some("Y"), Some("K")], "one Tab takes Y to K");
        let found: Vec<&(String, Rect)> = stops.iter().flatten().collect();
        for (left, right) in found[..7].iter().zip(&found[7..]) {
            assert_eq!(left.1.center().y, right.1.center().y, "{} and {} are level", left.0, right.0);
        }
    }

    #[test]
    fn every_mode_draws_and_cmyk_documents_get_cmyk() {
        let mut app = app();
        for fields in
            [json!({"channel": "red"}), json!({"channel": "brightness", "webOnly": true}), json!({"swatches": true}), json!({"color": "#0000ff"})]
        {
            app.run("ui.colorPicker", json!({})).unwrap();
            let d = app.ui.dialog.as_mut().unwrap();
            for (k, v) in fields.as_object().unwrap() {
                d.fields.insert(k.clone(), v.clone());
            }
            frame(&mut app);
            assert!(app.ui.dialog.is_some());
        }
        app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
        app.run("ui.colorPicker", json!({"color": "#ff0000"})).unwrap();
        super::super::confirm(&mut app).unwrap();
        assert!(matches!(crate::panels::current_paints(&app).0.color(), Some(Color::Cmyk { .. })));
    }
    /// Inspect actual painted controls, rather than a second implementation of the layout.
    #[derive(Default)]
    struct Drawn {
        texts: Vec<(String, Rect)>,
        visible_texts: Vec<String>,
        boxes: Vec<Rect>,
        fields: Vec<Rect>,
    }

    impl Drawn {
        /// Collects painted text, control bounds and color meshes, recording text visibility
        /// against the clip rectangle so tests can detect clipped labels.
        fn collect(&mut self, shape: &egui::Shape, clip: Rect) {
            match shape {
                egui::Shape::Text(t) => {
                    self.texts.push((t.galley.text().into(), t.visual_bounding_rect()));
                    if clip.expand(1.0).contains_rect(t.visual_bounding_rect()) {
                        self.visible_texts.push(t.galley.text().into());
                    }
                }
                egui::Shape::Rect(r) => self.boxes.push(r.rect),
                egui::Shape::Mesh(m) if m.vertices.len() > 100 => self.fields.push(m.calc_bounds()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| self.collect(s, clip)),
                _ => {}
            }
        }

        /// Returns the bounds of exactly one painted label, failing on missing or duplicate text.
        fn text(&self, label: &str) -> Rect {
            let found: Vec<_> = self.texts.iter().filter(|(s, _)| s == label).collect();
            assert_eq!(found.len(), 1, "one {label}, including no duplicate footer: {:?}", self.texts);
            found[0].1
        }

        /// Finds the smallest painted control containing a unique label's center.
        fn control(&self, label: &str) -> Rect {
            let center = self.text(label).center();
            self.boxes.iter().filter(|r| r.contains(center)).min_by(|a, b| a.area().total_cmp(&b.area())).copied().unwrap()
        }
    }

    /// Runs a dialog frame with real egui input at a physical window size, accounting for UI zoom,
    /// and returns the painted controls for layout and interaction assertions.
    fn draw(ctx: &egui::Context, app: &mut VectorcraftApp, size: egui::Vec2, events: Vec<egui::Event>) -> Drawn {
        let input =
            egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size / ctx.zoom_factor())), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            crate::shortcuts::handle(app, ui.ctx());
            super::super::show(app, ui.ctx());
        });
        let mut drawn = Drawn::default();
        for shape in &out.shapes {
            drawn.collect(&shape.shape, shape.clip_rect);
        }
        out.textures_delta.clear();
        drawn
    }

    /// The physical window size used for pointer and keyboard interaction tests.
    fn screen() -> egui::Vec2 {
        vec2(1280.0, 900.0)
    }

    /// Lets the auto-sized dialog settle over six frames and returns the next frame's controls.
    fn settled(ctx: &egui::Context, app: &mut VectorcraftApp, size: egui::Vec2) -> Drawn {
        for _ in 0..6 {
            draw(ctx, app, size, vec![]);
        }
        draw(ctx, app, size, vec![])
    }

    /// Creates a persistent test context with the application's fonts and default theme.
    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, Default::default());
        ctx
    }

    /// Sends a primary pointer press and release in separate frames at the given position.
    fn click_at(ctx: &egui::Context, app: &mut VectorcraftApp, at: egui::Pos2) {
        for pressed in [true, false] {
            draw(
                ctx,
                app,
                screen(),
                vec![
                    egui::Event::PointerMoved(at),
                    egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() },
                ],
            );
        }
    }

    /// Creates an unmodified, non-repeating key press for dialog shortcut tests.
    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }
    }

    /// Checks accessible action names and each channel slider's displayed value, range and units.
    #[test]
    fn compact_picker_preserves_screen_reader_controls_and_channel_values() {
        use egui::accesskit::Role;

        for (channel, value, maximum, text) in [
            (Channel::Hue, 210.0, 360.0, "210°"),
            (Channel::Saturation, 67.0, 100.0, "67%"),
            (Channel::Brightness, 60.0, 100.0, "60%"),
            (Channel::Red, 51.0, 255.0, "51"),
            (Channel::Green, 102.0, 255.0, "102"),
            (Channel::Blue, 153.0, 255.0, "153"),
        ] {
            let mut app = app();
            app.services.screen_color = Some(Box::new(|| Ok(std::sync::mpsc::channel().1)));
            open(&mut app, &json!({"color": "#336699"})).unwrap();
            let mut d = app.ui.dialog.take().unwrap();
            d.fields.insert("channel".into(), json!(channel.id()));
            let tree = widgets::accessibility_tests::tree(|ui| {
                body(&mut app, ui, &mut d);
            });
            let slider = tree.iter().find(|n| n.role() == Role::Slider && n.label() == Some(channel.label())).unwrap();
            assert_eq!(slider.numeric_value(), Some(value));
            assert_eq!(slider.min_numeric_value(), Some(0.0));
            assert_eq!(slider.max_numeric_value(), Some(maximum));
            assert_eq!(slider.value(), Some(text));
            for label in ["OK", "Cancel", "Color Swatches", "Sample Color from Screen"] {
                assert!(tree.iter().any(|n| n.role() == Role::Button && n.label() == Some(label)), "missing button: {label}");
            }
        }
    }

    /// Checks aligned actions and input rows at two UI scales, and stable dimensions when
    /// switching between color models and swatches.
    #[test]
    fn inline_actions_are_aligned_and_modes_keep_the_window_size() {
        // Language selection is process-global; German is checked in the native app instead
        // of racing the other parallel UI tests by changing the locale here.
        for zoom in [1.0, 1.5] {
            let (ctx, mut app) = (context(), app());
            ctx.set_zoom_factor(zoom);
            app.run("ui.colorPicker", json!({})).unwrap();
            let drawn = settled(&ctx, &mut app, screen());
            let ok = drawn.control(tl!("OK"));
            let cancel = drawn.control(tl!("Cancel"));
            let toggle = drawn.control(tl!("Color Swatches"));
            assert!(ok.bottom() < cancel.top() && cancel.bottom() < toggle.top());
            for r in [cancel, toggle] {
                assert!((r.left() - ok.left()).abs() < 1.0 && (r.width() - ok.width()).abs() < 1.0);
            }
            let field = drawn.fields.iter().max_by(|a, b| a.area().total_cmp(&b.area())).unwrap();
            assert!((field.width() - field.height()).abs() < 1.0);
            assert!((field.top() - ok.top()).abs() < 1.0);
            let k = drawn.text("K");
            let last_input = drawn
                .boxes
                .iter()
                .find(|r| (r.width() - VALUE_W).abs() < 0.5 && r.left() > k.right() && r.contains(egui::pos2(r.center().x, k.center().y)))
                .unwrap();
            assert!((last_input.bottom() - field.bottom()).abs() < 2.0, "{last_input:?} vs {field:?}");
            // Fonts and translated actions may leave one normal gutter, but never an unused column.
            assert!((last_input.right() - ok.right()).abs() < COLUMN_GAP, "no unused column beside the inputs: {last_input:?} vs {ok:?}");
            let id = egui::Id::new(("dialog", KIND));
            let before = ctx.memory(|m| m.area_rect(id)).unwrap();
            assert!(drawn.boxes.iter().all(|r| r.left() >= before.left() - 1.0 && r.right() <= before.right() + 1.0), "controls fit horizontally");
            let title = drawn.text(tl!("Color Picker"));
            assert!((title.center().x - before.center().x).abs() < 1.0, "centred heading");
            assert!(title.bottom() < field.top(), "a separate title band above the content");
            assert!(drawn.visible_texts.iter().any(|s| s == tl!("Only Web Colors")), "the checkbox label is fully visible");
            assert!(drawn.text("L").top() - toggle.bottom() >= PREVIEW_GAP, "space below the actions");
            click_at(&ctx, &mut app, toggle.center());
            assert!(app.ui.dialog.as_ref().unwrap().bool("swatches"));
            let drawn = settled(&ctx, &mut app, screen());
            assert!((drawn.control(tl!("Color Models")).width() - ok.width()).abs() < 1.0);
            let after = ctx.memory(|m| m.area_rect(id)).unwrap();
            assert!((before.size() - after.size()).length() < 1.0, "{before:?} vs {after:?}");
            assert!(
                drawn.boxes.iter().all(|r| r.left() >= after.left() - 1.0 && r.right() <= after.right() + 1.0),
                "swatch controls fit horizontally"
            );
            click_at(&ctx, &mut app, drawn.text(tl!("Color Models")).center());
            assert!(!app.ui.dialog.as_ref().unwrap().bool("swatches"));
        }
    }

    /// Clicks the sampler above the field, receives a color and checks Cancel restores the paint.
    #[test]
    fn sampler_above_the_field_previews_and_cancel_restores_the_original() {
        let (ctx, mut app) = (context(), app());
        let (tx, rx) = std::sync::mpsc::channel();
        let mut receiver = Some(rx);
        app.services.screen_color = Some(Box::new(move || receiver.take().ok_or("sampler already started".into())));
        app.run("ui.colorPicker", json!({})).unwrap();
        let original = fill_hex(&app);
        let drawn = settled(&ctx, &mut app, screen());
        let field = drawn.fields.iter().max_by(|a, b| a.area().total_cmp(&b.area())).unwrap();
        let hover = egui::pos2(field.right() - 12.0, field.top() - 20.0);
        draw(&ctx, &mut app, screen(), vec![egui::Event::PointerMoved(hover)]);
        let drawn = settled(&ctx, &mut app, screen());
        let sampler = drawn.boxes.iter().find(|r| r.size() == vec2(24.0, 24.0) && r.bottom() < field.top()).unwrap();
        assert!(sampler.top() > drawn.text(tl!("Color Picker")).bottom() && sampler.right() <= field.right());
        click_at(&ctx, &mut app, sampler.center());
        assert!(app.screen_sampler.is_some(), "the button above the field starts the host sampler");
        tx.send(Ok(Some([0.0, 1.0, 0.0]))).unwrap();
        crate::screen_color::poll(&mut app, &ctx);
        let drawn = settled(&ctx, &mut app, screen());
        assert_eq!(fill_hex(&app), "#00ff00");
        click_at(&ctx, &mut app, drawn.text(tl!("Cancel")).center());
        assert!(app.ui.dialog.is_none());
        assert_eq!(fill_hex(&app), original);
    }

    /// Checks both correction warnings fit below the preview without widening the dialog,
    /// and that clicking the web warning still corrects the color.
    #[test]
    fn warnings_fit_below_the_preview_without_widening_the_dialog() {
        let (ctx, mut app) = (context(), app());
        app.run("ui.colorPicker", json!({"color":"#ffffff"})).unwrap();
        settled(&ctx, &mut app, screen());
        let id = egui::Id::new(("dialog", KIND));
        let before = ctx.memory(|m| m.area_rect(id)).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("hex".into(), json!("0000FE"));
        let drawn = settled(&ctx, &mut app, screen());
        let ok = drawn.control(tl!("OK"));
        let toggle = drawn.control(tl!("Color Swatches"));
        let h = drawn.text("H");
        let mut warnings: Vec<_> = drawn
            .boxes
            .iter()
            .filter(|r| (r.width() - 16.0).abs() < 0.5 && (r.height() - 16.0).abs() < 0.5 && r.bottom() < h.top())
            .copied()
            .collect();
        warnings.dedup(); // The tile's fill and border share one rectangle.
        assert_eq!(warnings.len(), 2, "gamut and web correction chips are both present");
        for r in &warnings {
            assert!(r.right() < ok.left() && r.bottom() <= toggle.bottom());
            assert!(r.top() > ok.top() + CHIP_H, "warnings below the preview");
        }
        let after = ctx.memory(|m| m.area_rect(id)).unwrap();
        assert!((before.size() - after.size()).length() < 1.0, "{before:?} vs {after:?}");
        click_at(&ctx, &mut app, warnings[1].center());
        assert!(is_web_safe(&picked(app.ui.dialog.as_ref().unwrap()).color), "web correction remains clickable");
    }

    /// Drags the custom title band and checks the dialog moves by the pointer's displacement.
    #[test]
    fn colour_picker_moves_by_its_title_band() {
        let (ctx, mut app) = (context(), app());
        app.run("ui.colorPicker", json!({})).unwrap();
        let drawn = settled(&ctx, &mut app, screen());
        let title = drawn.text(tl!("Color Picker")).center();
        let id = egui::Id::new(("dialog", KIND));
        let before = ctx.memory(|m| m.area_rect(id)).unwrap();
        let pointer = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        draw(&ctx, &mut app, screen(), vec![egui::Event::PointerMoved(title), pointer(title, true)]);
        for i in 1..=4 {
            draw(&ctx, &mut app, screen(), vec![egui::Event::PointerMoved(title + vec2(6.0, 4.0) * i as f32)]);
        }
        draw(&ctx, &mut app, screen(), vec![pointer(title + vec2(24.0, 16.0), false)]);
        settled(&ctx, &mut app, screen());
        let after = ctx.memory(|m| m.area_rect(id)).unwrap();
        assert!((after.center() - before.center() - vec2(24.0, 16.0)).length() < 1.5);
        assert!(app.ui.dialog.is_some());
    }

    /// Checks OK and Enter commit typed values as one undo step, while Cancel and Escape
    /// restore the original paint after a live preview.
    #[test]
    fn inline_ok_commits_typed_input_and_cancel_restores_the_preview() {
        for enter in [false, true] {
            let (ctx, mut app) = (context(), app());
            app.run("ui.colorPicker", json!({"color": "#ff0000"})).unwrap();
            let drawn = settled(&ctx, &mut app, screen());
            let undo = app.session.doc().unwrap().history.undo.len();
            // The first numeric field has selected text, just as in the other form dialogs.
            draw(&ctx, &mut app, screen(), vec![egui::Event::Text("120".into())]);
            if enter {
                draw(&ctx, &mut app, screen(), vec![key(egui::Key::Enter)]);
            } else {
                click_at(&ctx, &mut app, drawn.text(tl!("OK")).center());
            }
            assert!(app.ui.dialog.is_none());
            assert_eq!(fill_hex(&app), "#00ff00");
            assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
            app.run("ui.colorPicker", json!({})).unwrap();
            app.ui.dialog.as_mut().unwrap().fields.insert("hex".into(), json!("0000ff"));
            let drawn = settled(&ctx, &mut app, screen());
            assert_eq!(fill_hex(&app), "#0000ff");
            if enter {
                draw(&ctx, &mut app, screen(), vec![key(egui::Key::Escape)]);
            } else {
                click_at(&ctx, &mut app, drawn.text(tl!("Cancel")).center());
            }
            assert!(app.ui.dialog.is_none());
            assert_eq!(fill_hex(&app), "#00ff00");
            assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
        }
    }

    /// Checks shrinking and stacking keep the dialog inside small windows, vertical scrolling
    /// reaches the actions and last input, and Enter can still confirm.
    #[test]
    fn compact_picker_shrinks_then_scrolls_without_enlarging_the_window() {
        for size in [vec2(740.0, 600.0), vec2(540.0, 420.0), vec2(400.0, 280.0)] {
            let (ctx, mut app) = (context(), app());
            app.run("ui.colorPicker", json!({"color": "#0000ff"})).unwrap();
            let drawn = settled(&ctx, &mut app, size);
            let outer = ctx.memory(|m| m.area_rect(egui::Id::new(("dialog", KIND)))).unwrap();
            assert!(outer.width() <= size.x && outer.height() <= size.y, "{outer:?} in {size:?}");
            let field = drawn.fields.iter().max_by(|a, b| a.area().total_cmp(&b.area())).unwrap();
            assert!((MIN_FIELD..=FIELD).contains(&field.width()));
            assert!((field.width() - field.height()).abs() < 1.0);
            assert!(drawn.boxes.iter().all(|r| r.left() >= outer.left() - 1.0 && r.right() <= outer.right() + 1.0), "controls fit horizontally");
            if size.x < 500.0 {
                assert!(drawn.control(tl!("OK")).top() > field.bottom(), "narrow windows stack the controls below the field");
                let mut actions_visible = false;
                let mut last_input_visible = false;
                for _ in 0..30 {
                    draw(
                        &ctx,
                        &mut app,
                        size,
                        vec![
                            egui::Event::PointerMoved(outer.min + vec2(30.0, 80.0)),
                            egui::Event::MouseWheel {
                                unit: egui::MouseWheelUnit::Point,
                                delta: vec2(0.0, -20.0),
                                phase: egui::TouchPhase::Move,
                                modifiers: Default::default(),
                            },
                        ],
                    );
                    let scrolled = settled(&ctx, &mut app, size);
                    actions_visible |= scrolled.visible_texts.iter().any(|s| s == tl!("OK"));
                    last_input_visible |= scrolled.visible_texts.iter().any(|s| s == "K");
                }
                assert!(actions_visible && last_input_visible, "vertical scrolling reveals actions and the last input");
            }
            // Keyboard confirmation remains available even when the actions require scrolling.
            draw(&ctx, &mut app, size, vec![key(egui::Key::Enter)]);
            assert!(app.ui.dialog.is_none());
        }
    }
}
