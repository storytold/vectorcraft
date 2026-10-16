//! The Color Picker (double-click a Fill/Stroke proxy, or `ui.colorPicker`): a colour field with a
//! channel slider for H, S, B, R, G or B; HSB, RGB, Lab and CMYK fields and hex; Only Web Colors;
//! the out-of-gamut and out-of-web warnings (click to correct); New/Original chips; an eyedropper
//! button that samples a color anywhere on the screen ([`crate::screen_color`]); and a Color
//! Swatches list. With art selected the colour shows on it live as it is picked; OK keeps it as one
//! undo step through `paint.setFill` / `paint.setStroke`, Cancel puts the art back.
//!
//! Fields agents set with `ui.dialog.set`: `hex` (`"00FF00"`) or `color` (any colour the paint
//! commands take), `channel` (`hue`, `saturation`, `brightness`, `red`, `green`, `blue`), `webOnly`
//! and `swatches`. `stroke` and `original` come from the opener.

use egui::{Rect, Sense, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_color::cms::{self, Lab};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::ColorMode;
use vectorcraft_engine::cmd::color_value;

use super::DialogSpec;
use super::form::PREVIEWED;
use crate::panels::color::{GAMUT_WARNING, WEB_WARNING, gamut_fix, hex_digits, is_web_safe, parse_hex, warning_chip, web_safe};
use crate::panels::{c32, color_json};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

pub(super) const KIND: &str = "colorPicker";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Color Picker").into(),
    body,
    confirm,
    preview: true,
    min_width: 600.0,
    max_width: Some(640.0),
    ..DialogSpec::FORM
};

/// Side of the colour field (and height of the channel slider).
const FIELD: f32 = 240.0;
/// Width of the value fields.
const VALUE_W: f32 = 54.0;

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

/// `ui.colorPicker {stroke?, color?}`: open the picker for the fill (or stroke) proxy, starting
/// from `color` or the proxy's colour.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let stroke = p.get("stroke").and_then(Value::as_bool).unwrap_or(!app.session.fill_active);
    let color = match p.get("color") {
        Some(v) => color_value(v).ok_or_else(|| format!("bad color {v}"))?,
        None => proxy_color(app, stroke),
    };
    let mut d = Dialog::new(
        KIND,
        json!({"stroke": stroke, "original": color_json(&color), "channel": Channel::default().id(), "webOnly": false, "swatches": false}),
    );
    store(&mut d, &Picked::of(color));
    d.fields.insert(crate::screen_color::PICKER_ID.into(), json!(crate::screen_color::picker_id()));
    app.ui.dialog = Some(d);
    Ok(Value::Null)
}

fn body(app: &mut VectorcraftApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let channel = Channel::parse(&d.str("channel")).unwrap_or_default();
    let web = d.bool("webOnly");
    let swatches = d.bool("swatches");
    let p = picked(d);
    let original = d.fields.get("original").and_then(color_value).unwrap_or(p.color);
    let gamut = gamut_fix(app, ui.ctx(), "picker-gamut", &p.color);
    let mut pick: Option<Picked> = None;
    let mut new_channel = None;
    let mut toggle_swatches = false;
    let mut sample = false;
    ui.horizontal_top(|ui| {
        if swatches {
            pick = swatch_list(app, ui, &p.color).map(Picked::of);
        } else {
            let (z, x, y) = field_pos(channel, &p);
            let at = |x: f32, y: f32| c32(&field_pick(channel, z, x, y).snapped(web).color);
            if let (Some((x, y)), _) = widgets::color_field(ui, "cp-field", vec2(FIELD, FIELD), (x, y), &at) {
                pick = Some(field_pick(channel, z, x, y));
            }
            ui.add_space(6.0);
            let track = |z: f32| c32(&Picked::of(slider_color(channel, z, x, y)).snapped(web).color);
            if let (Some(z), _) = widgets::channel_slider(ui, "cp-slider", z, vec2(22.0, FIELD), &track) {
                pick = Some(field_pick(channel, z, x, y));
            }
        }
        ui.add_space(14.0);
        ui.vertical(|ui| {
            ui.horizontal_top(|ui| {
                if new_original_chips(ui, &p.color, &original) {
                    pick = Some(Picked::of(original));
                }
                ui.add_space(4.0);
                sample = crate::screen_color::button(app, ui);
                ui.add_space(4.0);
                ui.vertical(|ui| {
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
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    toggle_swatches = widgets::flat_button(ui, if swatches { tl!("Color Models") } else { tl!("Color Swatches") }, 110.0).clicked();
                });
            });
            ui.add_space(10.0);
            let (edit, ch) = value_fields(ui, &p, channel);
            pick = pick.or(edit);
            new_channel = ch;
        });
    });
    ui.add_space(8.0);
    if widgets::check(ui, tl!("Only Web Colors"), web, true) {
        d.fields.insert("webOnly".into(), json!(!web));
    }
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
    false
}

/// Show the picked colour on the selected art while the picker is open: each new colour replaces
/// the last preview. Nothing previews until the colour first changes, nor without a selection
/// (the colour then only becomes the default paint, on OK).
fn preview(app: &mut VectorcraftApp, d: &mut Dialog) {
    let (cmd, params) = paint_command(app, d);
    let first = !app.session.in_interaction();
    let unchanged = d.fields.get(PREVIEWED) == Some(&params) || first && d.fields.get("original").and_then(color_value) == Some(picked(d).color);
    let selected = app.session.active().is_some_and(|st| !st.selection.objects.is_empty());
    if unchanged || !selected {
        return;
    }
    let _ = app.session.begin_interaction(if d.bool("stroke") { "Stroke Color" } else { "Fill Color" });
    if let Err(e) = app.session.preview(cmd, &params) {
        app.status(e.to_string());
    }
    d.fields.insert(PREVIEWED.into(), params);
}

/// The command that applies the picked colour to the proxy, and its params (as CMYK in a CMYK
/// document).
fn paint_command(app: &VectorcraftApp, d: &Dialog) -> (&'static str, Value) {
    let mut c = picked(d).color;
    if matches!(c, Color::Rgb { .. }) && app.session.active().is_some_and(|s| s.doc.color_mode == ColorMode::Cmyk) {
        let [cy, m, y, k] = c.to_cmyk_managed(cms::active().settings().intent);
        c = Color::cmyk(cy, m, y, k);
    }
    (if d.bool("stroke") { "paint.setStroke" } else { "paint.setFill" }, json!({ "color": color_json(&c) }))
}

/// The New (top) and Original (bottom) chips. Returns true when Original is clicked.
fn new_original_chips(ui: &mut Ui, new: &Color, original: &Color) -> bool {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(64.0, 60.0), Sense::hover());
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
    let spacing = vec2(6.0, 4.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = spacing.x;
        let left = egui::Grid::new("cp-values").spacing(spacing).show(ui, |ui| {
            for i in 0..7 {
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
        egui::Grid::new("cp-values-right").spacing(spacing).show(ui, |ui| {
            for i in 0..7 {
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
fn swatch_list(app: &VectorcraftApp, ui: &mut Ui, current: &Color) -> Option<Color> {
    let t = Tokens::get(ui.ctx());
    let mut chosen = None;
    ui.allocate_ui(vec2(FIELD + 28.0, FIELD), |ui| {
        widgets::list_box(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("cp-swatches").max_height(FIELD).show(ui, |ui| {
                ui.set_width(FIELD + 24.0);
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

/// OK: apply the colour to the proxy, keeping the live preview as one undo step.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
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
}
