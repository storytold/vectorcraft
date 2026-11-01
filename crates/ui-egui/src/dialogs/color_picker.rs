//! The Color Picker (double-click a Fill/Stroke proxy, or `ui.colorPicker`): a colour field with a
//! channel slider for H, S, B, R, G or B; HSB, RGB, Lab and CMYK fields and hex; Only Web Colors;
//! the out-of-gamut and out-of-web warnings (click to correct); New/Original chips; an eyedropper
//! button that samples a color anywhere on the screen ([`crate::screen_color`]); and a Color
//! Swatches list. With art selected the colour shows on it live as it is picked; OK keeps it as one
//! undo step through `paint.setFill` / `paint.setStroke` and makes the color the Color Guide's
//! base color; Cancel puts the art back.
//! Opened for an artboard (Properties › Artboards › Fill), it shows the color on the artboard's
//! paper live, and OK keeps it as one undo step through `artboard.setProps {index, color}`.
//! Opened over Recolor Artwork (double-click a row's new color), its OK sets that row's new color
//! and goes back to Recolor Artwork, whose preview the art keeps showing.
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

/// The field of a Color Picker opened over Recolor Artwork ([`over_recolor`]): the row whose new
/// color it picks.
const RECOLOR_ROW: &str = "__recolorRow";

/// The Color Picker over the Recolor Artwork dialog `parent`, on `color`, the new color of row
/// `row`: it previews nothing itself, its OK goes back to `parent` with the row's new color set
/// ([`super::recolor::with_new_color`]) and its Cancel goes back to `parent` as it was.
pub(super) fn over_recolor(parent: &Dialog, color: Color, row: usize) -> Dialog {
    let mut d = Dialog::new(
        KIND,
        json!({"original": color_json(&color), "channel": Channel::default().id(), "webOnly": false, "swatches": false, RECOLOR_ROW: row}),
    );
    d.fields.insert(super::PARENT.into(), serde_json::to_value(parent).unwrap_or_default());
    store(&mut d, &Picked::of(color));
    d.fields.insert(crate::screen_color::PICKER_ID.into(), json!(crate::screen_color::picker_id()));
    d
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
    // Over Recolor Artwork the art shows that dialog's preview, which this one must not replace.
    if d.fields.contains_key(RECOLOR_ROW) {
        return;
    }
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
    let c = in_document_model(app, picked(d).color);
    if let Some(i) = d.fields.get("artboard").and_then(Value::as_u64) {
        return ("artboard.setProps", json!({ "index": i, "color": color_json(&c) }));
    }
    (if d.bool("stroke") { "paint.setStroke" } else { "paint.setFill" }, json!({ "color": color_json(&c) }))
}

/// The picked colour as the document takes it: an RGB colour is CMYK in a CMYK document; CMYK,
/// grey and Lab colours stay as they are.
fn in_document_model(app: &VectorcraftApp, c: Color) -> Color {
    if matches!(c, Color::Rgb { .. }) && app.session.active().is_some_and(|s| s.doc.color_mode == ColorMode::Cmyk) {
        let [cy, m, y, k] = c.to_cmyk_managed(cms::active().settings().intent);
        return Color::cmyk(cy, m, y, k);
    }
    c
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
    ui.interact(top, ui.scope_id().with("cp-new"), Sense::hover()).on_hover_text(tl!("New"));
    ui.interact(bottom, ui.scope_id().with("cp-original"), Sense::click()).on_hover_text(tl!("Original: click to restore")).clicked()
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
                    if let Some(nv) = widgets::plain_field_named(ui, ("cp-left", i), Some(ch.label()), v as f64, suffix, 0, VALUE_W) {
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
                } else if let Some(c) =
                    widgets::hex_field_named(ui, "cp-hex", Some(tl!("Color")), &hex_digits(&p.color)).as_deref().and_then(parse_hex)
                {
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
                if let Some(nv) = widgets::plain_field_named(ui, ("cp-right", i), Some(label), v as f64, suffix, 0, VALUE_W) {
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
    if let Some(row) = d.fields.get(RECOLOR_ROW).and_then(Value::as_u64) {
        let parent = super::parent(d).ok_or("Recolor Artwork is no longer open")?;
        let c = in_document_model(app, picked(d).color);
        app.ui.dialog = Some(super::recolor::with_new_color(app, parent, usize::try_from(row).unwrap_or(usize::MAX), c));
        return Ok(Value::Null);
    }
    if d.bool("keep") && !app.session.in_interaction() && d.fields.get("original").and_then(color_value) == Some(picked(d).color) {
        app.ui.dialog = None;
        return Ok(Value::Null);
    }
    let (cmd, params) = paint_command(app, d);
    let c = in_document_model(app, picked(d).color);
    let r = super::form::commit_preview(app, cmd, params)?;
    // A color picked for the fill or stroke also becomes the Color Guide's base color.
    if !d.fields.contains_key("artboard") {
        crate::panels::color_guide::choose_base(app, c);
    }
    Ok(r)
}

#[cfg(test)]
mod tests;
