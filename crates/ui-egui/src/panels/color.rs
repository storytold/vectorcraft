//! Color panel: Grayscale / RGB / HSB / CMYK / Web Safe RGB sliders with colour-gradient tracks,
//! value fields, hex field, None/Black/White chips, spectrum ramp and the Fill/Stroke proxy.
//! When the active paint is a gradient, the sliders edit the Gradient panel's selected stop (on a
//! freeform gradient, its selected point). A colour (or stop) linked to a global or spot swatch
//! shows Tint mode instead: the swatch's chip and name and a T slider (0–100 % of the swatch);
//! picking a mode from the menu makes it a process colour.
//!
//! The mode follows the colour's own model (the last mode picked comes back for colours in its
//! model). Alt-clicking the spectrum or a chip paints the inactive proxy; Shift-clicking the
//! spectrum cycles the modes; Shift-dragging a slider moves the others in tandem; RGB and HSB show
//! the out-of-gamut warning; Hide Options leaves just the proxy and the spectrum.

use egui::{Color32, Rect, Sense, Stroke, StrokeKind, Ui, vec2};
use serde_json::json;
use vectorcraft_color::{Color, GradientKind, Paint};

use super::{active_paint, color_json, live_run, pstate, set_pstate};
use crate::theme::Tokens;
use crate::widgets::{self, Live, menu_item};
use crate::{VectorcraftApp, icons};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    Grayscale,
    #[default]
    Rgb,
    Hsb,
    Cmyk,
    WebSafe,
    /// CIE Lab: Swatch Options offers it; the panel shows it for Lab colours.
    Lab,
}

impl Mode {
    /// The panel menu's modes, in order.
    pub const ALL: [Mode; 5] = [Mode::Grayscale, Mode::Rgb, Mode::Hsb, Mode::Cmyk, Mode::WebSafe];

    pub fn label(self) -> &'static str {
        match self {
            Mode::Grayscale => "Grayscale",
            Mode::Rgb => "RGB",
            Mode::Hsb => "HSB",
            Mode::Cmyk => "CMYK",
            Mode::WebSafe => "Web Safe RGB",
            Mode::Lab => "Lab",
        }
    }
    pub fn labels(self) -> &'static [&'static str] {
        match self {
            Mode::Grayscale => &["K"],
            Mode::Rgb | Mode::WebSafe => &["R", "G", "B"],
            Mode::Hsb => &["H", "S", "B"],
            Mode::Cmyk => &["C", "M", "Y", "K"],
            Mode::Lab => &["L", "a", "b"],
        }
    }
    /// Maximum of each displayed component.
    pub fn max(self, i: usize) -> f32 {
        match self {
            Mode::Rgb | Mode::WebSafe => 255.0,
            Mode::Hsb if i == 0 => 360.0,
            Mode::Lab if i > 0 => 127.0,
            _ => 100.0,
        }
    }
    /// Minimum of each displayed component (0 but for Lab's a and b).
    pub fn min(self, i: usize) -> f32 {
        if self == Mode::Lab && i > 0 { -128.0 } else { 0.0 }
    }
    /// Slider position (0..1) of component `i` at value `v`.
    pub fn unit(self, i: usize, v: f32) -> f32 {
        (v - self.min(i)) / (self.max(i) - self.min(i))
    }
    /// Component `i` at slider position `t` (0..1), in whole units (Web Safe RGB snaps to its six
    /// levels).
    pub fn value_at(self, i: usize, t: f32) -> f32 {
        match self {
            Mode::WebSafe => ((t * 5.0).round() / 5.0) * self.max(i),
            _ => (self.min(i) + t * (self.max(i) - self.min(i))).round(),
        }
    }
    pub fn suffix(self, i: usize) -> &'static str {
        match self {
            Mode::Rgb | Mode::WebSafe | Mode::Lab => "",
            Mode::Hsb if i == 0 => "°",
            _ => "%",
        }
    }
    /// The mode a colour was authored in.
    pub fn of(c: &Color) -> Mode {
        match c {
            Color::Rgb { .. } => Mode::Rgb,
            Color::Cmyk { .. } => Mode::Cmyk,
            Color::Gray { .. } => Mode::Grayscale,
            Color::Lab { .. } => Mode::Lab,
        }
    }
}

/// Displayed component values of `c` in `mode` (RGB 0–255, HSB °/%/%, CMYK %, K %, Lab L*a*b*).
pub fn components(mode: Mode, c: &Color) -> Vec<f32> {
    match mode {
        Mode::Grayscale => {
            let k = match *c {
                Color::Gray { k } => k,
                _ => {
                    let [r, g, b] = c.to_rgb();
                    1.0 - (0.3 * r + 0.59 * g + 0.11 * b)
                }
            };
            vec![k * 100.0]
        }
        Mode::Rgb | Mode::WebSafe => c.to_rgb().iter().map(|v| (v * 255.0).clamp(0.0, 255.0)).collect(),
        Mode::Hsb => {
            let [h, s, b] = c.to_hsb();
            vec![h, s * 100.0, b * 100.0]
        }
        Mode::Cmyk => c.to_cmyk().iter().map(|v| v * 100.0).collect(),
        Mode::Lab => {
            let l = c.to_lab();
            vec![l.l, l.a, l.b]
        }
    }
}

/// Build a colour from displayed components (inverse of [`components`]).
pub fn from_components(mode: Mode, v: &[f32]) -> Color {
    let g = |i: usize| v.get(i).copied().unwrap_or(0.0);
    match mode {
        Mode::Grayscale => Color::gray((g(0) / 100.0).clamp(0.0, 1.0)),
        Mode::Rgb => Color::rgb((g(0) / 255.0).clamp(0.0, 1.0), (g(1) / 255.0).clamp(0.0, 1.0), (g(2) / 255.0).clamp(0.0, 1.0)),
        Mode::WebSafe => web_safe(&Color::rgb(g(0) / 255.0, g(1) / 255.0, g(2) / 255.0)),
        Mode::Hsb => Color::from_hsb(g(0).rem_euclid(360.0), (g(1) / 100.0).clamp(0.0, 1.0), (g(2) / 100.0).clamp(0.0, 1.0)),
        Mode::Cmyk => Color::cmyk(
            (g(0) / 100.0).clamp(0.0, 1.0),
            (g(1) / 100.0).clamp(0.0, 1.0),
            (g(2) / 100.0).clamp(0.0, 1.0),
            (g(3) / 100.0).clamp(0.0, 1.0),
        ),
        Mode::Lab => Color::lab(g(0).clamp(0.0, 100.0), g(1).clamp(-128.0, 127.0), g(2).clamp(-128.0, 127.0)),
    }
}

/// The nearest web-safe colour (each channel a multiple of 0x33).
pub fn web_safe(c: &Color) -> Color {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 5.0).round() / 5.0;
    let [r, g, b] = c.to_rgb();
    Color::rgb(q(r), q(g), q(b))
}

pub fn is_web_safe(c: &Color) -> bool {
    web_safe(c).to_hex() == c.to_hex()
}

/// Track colour of slider `i` at position `t` (0..1) with the other components fixed (dynamic
/// colour sliders).
pub fn track_color(mode: Mode, comps: &[f32], i: usize, t: f32) -> Color {
    let mut v = comps.to_vec();
    if i < v.len() {
        v[i] = mode.min(i) + t * (mode.max(i) - mode.min(i));
    }
    // HSB hue track at full saturation/brightness reads better when S or B are 0.
    let m = if mode == Mode::WebSafe { Mode::Rgb } else { mode };
    from_components(m, &v)
}

/// Colour at a point of the spectrum ramp: hue across; white at the top, full colour in the
/// middle, black at the bottom. In Grayscale mode the ramp is a grey ramp.
pub fn spectrum_at(mode: Mode, x: f32, y: f32) -> Color {
    let x = x.clamp(0.0, 1.0);
    let y = y.clamp(0.0, 1.0);
    if mode == Mode::Grayscale {
        return Color::gray(x);
    }
    let h = x * 360.0;
    let c = if y < 0.5 { Color::from_hsb(h, y * 2.0, 1.0) } else { Color::from_hsb(h, 1.0, 1.0 - (y - 0.5) * 2.0) };
    match mode {
        Mode::Cmyk => {
            let [c0, m, yy, k] = c.to_cmyk();
            Color::cmyk(c0, m, yy, k)
        }
        Mode::WebSafe => web_safe(&c),
        Mode::Lab => c.in_model(vectorcraft_color::cms::Model::Lab),
        _ => c,
    }
}

/// The hex field text of a colour (`E67828`).
pub fn hex_digits(c: &Color) -> String {
    c.to_hex().trim_start_matches('#').to_uppercase()
}

/// Parse a hex field (`E67828`, `#e67828`, `fff`).
pub fn parse_hex(s: &str) -> Option<Color> {
    let s = s.trim().trim_start_matches('#');
    if !(s.len() == 3 || s.len() == 6) || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Color::from_hex(s)
}

/// The out-of-web-colour warning: its icon and tooltip.
pub(crate) const WEB_WARNING: (&str, &str) = ("dc-cube", "Out of Web Color Warning");
/// The out-of-gamut warning: its icon and tooltip.
pub(crate) const GAMUT_WARNING: (&str, &str) = ("dc-gamut", "Out of Gamut Warning");

/// A warning icon with the corrected colour beside it. Returns true when the colour is clicked.
pub(crate) fn warning_chip(ui: &mut Ui, (icon, warning): (&str, &str), fix: &Color) -> bool {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        icons::icon(ui, icon, 16.0, t.icon).on_hover_text(tl!(warning));
        let (r, resp) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
        widgets::swatch_tile(ui, r, &Paint::solid(*fix), false, resp.hovered());
        resp.on_hover_text(crate::i18n::fmt(
            tl!("{warning}: click to correct to the closest color ({hex})"),
            &[("warning", tl!(warning)), ("hex", &fix.to_hex())],
        ))
        .clicked()
    })
    .inner
}

/// The printable colour closest to `c` (its working-CMYK reproduction, back in RGB) when `c` is out
/// of the CMYK gamut, through `color.convert`.
pub(crate) fn in_gamut(app: &mut VectorcraftApp, c: &Color) -> Option<Color> {
    let r = app.run("color.convert", json!({"color": color_json(c), "to": "cmyk"})).ok()?;
    if !r["outOfGamut"].as_bool()? {
        return None;
    }
    let v = |r: &serde_json::Value, i: usize| r["values"][i].as_f64();
    let ink = json!({"c": v(&r, 0)?, "m": v(&r, 1)?, "y": v(&r, 2)?, "k": v(&r, 3)?});
    let rgb = app.run("color.convert", json!({"color": ink, "to": "rgb"})).ok()?;
    Some(Color::rgb(v(&rgb, 0)? as f32, v(&rgb, 1)? as f32, v(&rgb, 2)? as f32))
}

/// [`in_gamut`], cached under `key` for the last colour asked (panels ask every frame).
pub(crate) fn gamut_fix(app: &mut VectorcraftApp, ctx: &egui::Context, key: &str, c: &Color) -> Option<Color> {
    match pstate::<Option<(Color, Option<Color>)>>(ctx, key) {
        Some((k, fix)) if k == *c => fix,
        _ => {
            let fix = in_gamut(app, c);
            set_pstate(ctx, key, Some((*c, fix)));
            fix
        }
    }
}

/// The panel mode for `color`: the one picked last (`stored`) when it shows the colour's own
/// model (HSB and Web Safe RGB show RGB), else the colour's model.
pub fn display_mode(stored: Option<Mode>, color: Option<&Color>) -> Mode {
    match color {
        Some(c) => stored.filter(|m| m.holds(c)).unwrap_or_else(|| Mode::of(c)),
        None => stored.unwrap_or_default(),
    }
}

impl Mode {
    /// The colour model the mode edits.
    fn model(self) -> Mode {
        match self {
            Mode::Hsb | Mode::WebSafe => Mode::Rgb,
            m => m,
        }
    }
    /// Does the mode show `c` in its own model?
    pub fn holds(self, c: &Color) -> bool {
        self.model() == Mode::of(c)
    }
    /// The next mode (Shift-click on the spectrum).
    pub fn next(self) -> Mode {
        let i = Mode::ALL.iter().position(|m| *m == self).unwrap_or(0);
        Mode::ALL[(i + 1) % Mode::ALL.len()]
    }
    /// Can Shift-drag move the sliders in tandem? (Not HSB, and Grayscale has one slider.)
    fn tandem(self) -> bool {
        matches!(self, Mode::Rgb | Mode::WebSafe | Mode::Cmyk)
    }
}

/// `c` in `mode`'s model: picking CMYK or Grayscale converts the colour, Web Safe RGB snaps it.
pub fn convert_to(mode: Mode, c: &Color) -> Color {
    match mode {
        Mode::WebSafe => web_safe(c),
        m if m.holds(c) => *c,
        m => from_components(m.model(), &components(m.model(), c)),
    }
}

/// Shift-drag of slider `i` to `v`: the other sliders move with it keeping their ratios, stopping
/// where one reaches its maximum. With slider `i` at 0 the others move by the same amount.
pub fn tandem(mode: Mode, comps: &[f32], i: usize, v: f32) -> Vec<f32> {
    let old = comps.get(i).copied().unwrap_or(0.0);
    if old <= 0.0 {
        return comps.iter().enumerate().map(|(j, c)| (c + v - old).clamp(0.0, mode.max(j))).collect();
    }
    let f = comps.iter().enumerate().filter(|(_, c)| **c > 0.0).fold((v / old).max(0.0), |f, (j, c)| f.min(mode.max(j) / c));
    comps.iter().map(|c| c * f).collect()
}

fn to32(c: &Color) -> Color32 {
    super::c32(c)
}

/// The global or spot swatch a colour is a tint of (Tint mode).
struct Tint {
    swatch: String,
    spot: bool,
    /// 0..1.
    tint: f32,
    /// The swatch's colour (100 %).
    base: Color,
}

impl Tint {
    /// The tint `link` (a swatch name) and `tint` show: `None` unless the swatch is a global colour.
    fn of(app: &VectorcraftApp, link: Option<&str>, tint: f32) -> Option<Self> {
        let name = link?;
        let d = &app.session.active()?.doc;
        let spot = d.swatch(name)?.spot;
        Some(Self { swatch: name.to_string(), spot, tint, base: d.global_color(name)? })
    }
}

/// What the panel is editing: the active proxy's solid colour or a selected gradient stop (with
/// the global swatch it is a tint of), or a selected freeform point.
enum Target {
    Paint(Option<Color>, Option<Tint>),
    Stop { paint: Paint, index: usize, color: Color, tint: Option<Tint> },
    Point { index: usize, color: Color },
}

impl Target {
    fn color(&self) -> Option<Color> {
        match self {
            Target::Paint(c, _) => *c,
            Target::Stop { color, .. } | Target::Point { color, .. } => Some(*color),
        }
    }
    fn tint(&self) -> Option<&Tint> {
        match self {
            Target::Paint(_, t) | Target::Stop { tint: t, .. } => t.as_ref(),
            Target::Point { .. } => None,
        }
    }
}

/// The active proxy's colour or gradient stop; no colour for None, patterns and a "?" proxy.
fn target(app: &VectorcraftApp, ui: &Ui) -> Target {
    if super::active_mixed(app, ui.ctx()) {
        return Target::Paint(None, None);
    }
    let p = active_paint(app);
    match &p {
        Paint::Gradient(g) if g.gradient.kind == GradientKind::Freeform => {
            // Like stops: without a selected point, the first.
            let f = super::gradient::shown_points(g);
            let index = app.session.selected_freeform_point().filter(|i| *i < f.points.len()).unwrap_or(0);
            Target::Point { index, color: f.points.get(index).map_or(Color::BLACK, |p| p.color) }
        }
        Paint::Gradient(g) => {
            let i = app.session.selected_stop().unwrap_or(0).min(g.gradient.stops.len().saturating_sub(1));
            let (color, tint) = g.gradient.stops.get(i).map_or((Color::BLACK, None), |s| (s.color, Tint::of(app, s.swatch.as_deref(), s.tint)));
            Target::Stop { paint: p.clone(), index: i, color, tint }
        }
        Paint::Solid { color, swatch, tint } => Target::Paint(Some(*color), Tint::of(app, swatch.as_deref(), *tint)),
        _ => Target::Paint(None, None),
    }
}

/// Apply `c` to the target, or with `behind` (an Alt-click) to the inactive proxy as a solid
/// colour, keeping the active proxy in front.
fn apply(app: &mut VectorcraftApp, tgt: &Target, c: Color, phase: Live, behind: bool) {
    match tgt {
        Target::Stop { paint: Paint::Gradient(g), index, .. } if !behind => {
            let mut stops = g.gradient.stops.clone();
            if let Some(s) = stops.get_mut(*index) {
                s.set_color(c, None);
            }
            super::gradient::set_stops(app, &stops, None, phase);
        }
        Target::Point { index, .. } if !behind => {
            let p = json!({"index": index, "color": color_json(&c), "stroke": !app.session.fill_active});
            live_run(app, "Color", "paint.freeform.setPoint", p, phase);
        }
        _ => {
            let cmd = super::proxy_cmd(app, behind);
            // The panel's sliders work in the model picked there, even in a CMYK document.
            live_run(app, "Color", cmd, json!({"color": color_json(&c), "focus": !behind, "keepModel": true}), phase);
        }
    }
}

/// Set the target's tint of its swatch to `t` (0..1), keeping the link (the T slider).
fn apply_tint(app: &mut VectorcraftApp, tgt: &Target, tint: &Tint, t: f32, phase: Live) {
    match tgt {
        Target::Stop { paint: Paint::Gradient(g), index, .. } => {
            let mut stops = g.gradient.stops.clone();
            if let Some(s) = stops.get_mut(*index) {
                s.set_color(tint.base.tinted(t), Some((tint.swatch.clone(), t)));
            }
            super::gradient::set_stops(app, &stops, None, phase);
        }
        _ => {
            let cmd = super::proxy_cmd(app, false);
            live_run(app, "Color", cmd, json!({"swatch": tint.swatch, "tint": t * 100.0}), phase);
        }
    }
}

/// Switch the panel to `mode`, converting the colour to its model (a tint becomes a process
/// colour).
fn set_mode(app: &mut VectorcraftApp, ctx: &egui::Context, tgt: &Target, mode: Mode) {
    set_pstate(ctx, "color-mode", Some(mode));
    if let Some(c) = tgt.color() {
        let conv = convert_to(mode, &c);
        if conv != c || tgt.tint().is_some() {
            apply(app, tgt, conv, Live::Released, false);
        }
    }
}

/// Tint mode's rows: the swatch's chip and name, and the T slider and field. Returns a new tint
/// (0..1) and its phase.
fn tint_rows(ui: &mut Ui, tint: &Tint, slider_w: f32, field_w: f32) -> Option<(f32, Live)> {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
        widgets::swatch_tile(ui, r, &Paint::solid(tint.base), false, false);
        super::swatches::global_mark(ui, r, tint.spot);
        ui.label(egui::RichText::new(&tint.swatch).size(12.5).color(t.text));
    });
    ui.horizontal(|ui| {
        ui.add_sized(vec2(12.0, 22.0), egui::Label::new(egui::RichText::new("T").size(12.5).color(t.text)));
        let track = |x: f32| to32(&tint.base.tinted(x));
        let (nv, phase) = widgets::color_slider(ui, ("color-slider", "tint"), tint.tint, slider_w, &track);
        let field = widgets::plain_field(ui, ("color-field", "tint"), (tint.tint * 100.0).round() as f64, "%", 0, field_w);
        match (nv, field) {
            (Some(v), _) => Some(((v * 100.0).round() / 100.0, phase)),
            (None, Some(v)) => Some(((v as f32 / 100.0).clamp(0.0, 1.0), Live::Released)),
            _ => None,
        }
    })
    .inner
}

/// What happened on the spectrum ramp.
enum SpectrumHit {
    Pick(Color, Live),
    /// Shift-click: the next colour mode.
    Cycle,
}

/// The spectrum ramp: hue across, white to full colour to black down (a grey ramp in Grayscale).
fn spectrum(ui: &mut Ui, mode: Mode, size: egui::Vec2) -> Option<SpectrumHit> {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(size, Sense::hover());
    let resp = ui.interact(r, ui.id().with("color-spectrum"), Sense::click_and_drag());
    ui.painter().add(widgets::color_mesh(r, (72, 12), &|x, y| to32(&spectrum_at(mode, x, y))));
    ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, t.border), StrokeKind::Outside);
    if let Some(p) = resp.hover_pos() {
        ui.painter().rect_stroke(Rect::from_center_size(p, vec2(5.0, 5.0)), 0.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Middle);
    }
    if resp.clicked() && ui.input(|i| i.modifiers.shift) {
        return Some(SpectrumHit::Cycle);
    }
    let (p, phase) = widgets::pointer_phase(&resp)?;
    Some(SpectrumHit::Pick(spectrum_at(mode, (p.x - r.left()) / r.width(), (p.y - r.top()) / r.height()), phase))
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let tgt = target(app, ui);
    let color = tgt.color();
    let mode = display_mode(pstate(ui.ctx(), "color-mode"), color.as_ref());
    let alt = super::alt_held(ui);
    // Hide Options: just the proxy and the spectrum.
    if pstate::<bool>(ui.ctx(), "color-hide-options") {
        let hit = ui
            .horizontal(|ui| {
                super::proxy(app, ui, 40.0);
                spectrum(ui, mode, vec2(ui.available_width(), 40.0))
            })
            .inner;
        match hit {
            Some(SpectrumHit::Pick(c, phase)) => apply(app, &tgt, c, phase, alt),
            Some(SpectrumHit::Cycle) => set_mode(app, ui.ctx(), &tgt, mode.next()),
            None => {}
        }
        return;
    }
    // Keep the displayed components while the colour is unchanged (hue survives S = 0 etc.).
    let key = color.map(|c| (c.to_hex(), mode as u8));
    let comps: Vec<f32> = match (color, pstate::<Option<((String, u8), Vec<f32>)>>(ui.ctx(), "color-comps")) {
        (Some(_), Some((k, v))) if Some(&k) == key.as_ref() => v,
        (Some(c), _) => components(mode, &c),
        (None, _) => vec![0.0; mode.labels().len()],
    };
    let recent = super::recent_colors_row(app, ui);
    widgets::divider(ui);
    // An edit of the active colour with its displayed components (`new`), or a colour clicked with
    // Alt held for the inactive proxy (`behind`).
    let mut new: Option<(Color, Live, Vec<f32>)> = None;
    let mut behind: Option<(Color, Live)> = None;
    let mut tinted: Option<(f32, Live)> = None;
    // The warnings the mode shows: out of gamut in RGB and HSB, out of web colours but in Web Safe
    // RGB and Grayscale.
    let (shows_gamut, shows_web) = (matches!(mode, Mode::Rgb | Mode::Hsb), !matches!(mode, Mode::WebSafe | Mode::Grayscale));
    let gamut = color.filter(|_| shows_gamut).and_then(|c| gamut_fix(app, ui.ctx(), "color-gamut", &c));
    ui.horizontal(|ui| {
        // Left column: proxy, out-of-gamut and out-of-web warnings.
        ui.vertical(|ui| {
            ui.set_width(44.0);
            super::proxy(app, ui, 40.0);
            let web = color.filter(|c| shows_web && !is_web_safe(c)).map(|c| web_safe(&c));
            let warnings: Vec<_> = [(GAMUT_WARNING, gamut), (WEB_WARNING, web)].into_iter().filter_map(|(w, fix)| Some((w, fix?))).collect();
            for (warning, fix) in &warnings {
                ui.add_space(6.0);
                if warning_chip(ui, *warning, fix) {
                    new = Some((*fix, Live::Released, components(mode, fix)));
                }
            }
            // Room for the mode's other warnings: the panel keeps its height as the colour changes,
            // else the spectrum below moves under a dragging pointer and the colour flips every
            // frame, e.g. between a pale colour out of gamut and white above the spectrum (#578).
            for _ in warnings.len()..usize::from(shows_gamut) + usize::from(shows_web) {
                ui.add_space(6.0);
                ui.scope(|ui| {
                    ui.set_invisible();
                    warning_chip(ui, WEB_WARNING, &Color::WHITE)
                });
            }
        });
        // Sliders (Shift-drag moves them in tandem).
        ui.vertical(|ui| {
            let shift = ui.input(|i| i.modifiers.shift);
            let field_w = 48.0;
            let slider_w = (ui.available_width() - field_w - 24.0).max(60.0);
            if let Some(tint) = tgt.tint() {
                tinted = tint_rows(ui, tint, slider_w, field_w);
                return;
            }
            let enabled = color.is_some();
            for (i, lbl) in mode.labels().iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.add_sized(vec2(12.0, 22.0), egui::Label::new(egui::RichText::new(*lbl).size(12.5).color(t.text)));
                    let v = comps.get(i).copied().unwrap_or(0.0);
                    let track = |x: f32| to32(&track_color(mode, &comps, i, x));
                    let grey = |_x: f32| t.input;
                    let (nv, phase) = if enabled {
                        widgets::color_slider(ui, ("color-slider", i), mode.unit(i, v), slider_w, &track)
                    } else {
                        widgets::color_slider(ui, ("color-slider", i), 0.0, slider_w, &grey)
                    };
                    if let Some(nv) = nv
                        && enabled
                    {
                        let nv = mode.value_at(i, nv);
                        let c2 = if shift && mode.tandem() {
                            tandem(mode, &comps, i, nv)
                        } else {
                            let mut c2 = comps.clone();
                            c2[i] = nv;
                            c2
                        };
                        new = Some((from_components(mode, &c2), phase, c2));
                    }
                    if let Some(fv) = widgets::plain_field(ui, ("color-field", i), v as f64, mode.suffix(i), 0, field_w)
                        && enabled
                    {
                        let mut c2 = comps.clone();
                        c2[i] = (fv as f32).clamp(mode.min(i), mode.max(i));
                        new = Some((from_components(mode, &c2), Live::Released, c2));
                    }
                });
            }
        });
    });
    ui.add_space(4.0);
    // A clicked chip or spectrum colour: the active colour, or with Alt the inactive proxy's.
    let mut pick = |c: Color, phase: Live, new: &mut Option<(Color, Live, Vec<f32>)>| {
        if alt {
            behind = Some((c, phase));
        } else {
            *new = Some((c, phase, components(mode, &c)));
        }
    };
    // A recent colour, like the chips, recolours the selected gradient stop (or point).
    if let Some(c) = recent {
        pick(c, Live::Released, &mut new);
    }
    // None / Black / White chips and the hex field.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (p, tip) in [(Paint::None, tl!("None")), (Paint::solid(Color::BLACK), tl!("Black")), (Paint::solid(Color::WHITE), tl!("White"))] {
            let (r, resp) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
            widgets::swatch_tile(ui, r, &p, false, resp.hovered());
            if resp.on_hover_text(tip).clicked() {
                match p.color() {
                    Some(c) => pick(c, Live::Released, &mut new),
                    None => super::apply_click(app, ui, json!({"none": true})),
                }
            }
        }
        ui.spacing_mut().item_spacing.x = 6.0;
        if matches!(mode, Mode::Rgb | Mode::WebSafe | Mode::Hsb) {
            ui.add_space((ui.available_width() - 96.0).max(4.0));
            let hex = color.map(|c| hex_digits(&c)).unwrap_or_default();
            if let Some(c) = widgets::hex_field(ui, "hex", &hex).as_deref().and_then(parse_hex) {
                new = Some((c, Live::Released, components(mode, &c)));
            }
        }
    });
    ui.add_space(4.0);
    match spectrum(ui, mode, vec2(ui.available_width(), 46.0)) {
        Some(SpectrumHit::Pick(c, phase)) => pick(c, phase, &mut new),
        Some(SpectrumHit::Cycle) => set_mode(app, ui.ctx(), &tgt, mode.next()),
        None => {}
    }
    if let Some((c, phase)) = behind {
        apply(app, &tgt, c, phase, true);
    }
    if let (Some((t, phase)), Some(tint)) = (tinted, tgt.tint()) {
        apply_tint(app, &tgt, tint, t, phase);
    }
    if let Some((c, phase, comps2)) = new {
        let key = (c.to_hex(), mode as u8);
        set_pstate(ui.ctx(), "color-comps", Some((key, comps2)));
        apply(app, &tgt, c, phase, false);
    }
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let hidden: bool = pstate(ui.ctx(), "color-hide-options");
    if menu_item(ui, if hidden { tl!("Show Options") } else { tl!("Hide Options") }, true, false) {
        set_pstate(ui.ctx(), "color-hide-options", !hidden);
    }
    ui.separator();
    let tgt = target(app, ui);
    let color = tgt.color();
    // Tint mode checks no colour mode: picking one makes the tint a process colour.
    let cur = display_mode(pstate(ui.ctx(), "color-mode"), color.as_ref());
    for m in Mode::ALL {
        if menu_item(ui, tl!(m.label()), true, m == cur && tgt.tint().is_none()) {
            set_mode(app, ui.ctx(), &tgt, m);
        }
    }
    ui.separator();
    // Solid colours (and a "?" proxy) go through the proxy commands (each selected object keeps
    // its colour model); a gradient stop is recoloured in place.
    let recolor = color.is_some() || super::active_mixed(app, ui.ctx());
    for (label, cmd, f) in [
        (tl!("Invert"), "paint.invert", Color::invert_keep_model as fn(&Color) -> Color),
        (tl!("Complement"), "paint.complement", Color::complement_keep_model),
    ] {
        if menu_item(ui, label, recolor, false) {
            match (&tgt, color) {
                (Target::Stop { .. } | Target::Point { .. }, Some(c)) => apply(app, &tgt, f(&c), Live::Released, false),
                _ => {
                    app.run(cmd, json!({})).ok();
                }
            }
        }
    }
    ui.separator();
    if menu_item(ui, tl!("Create New Swatch…"), color.is_some(), false)
        && let Some(c) = color
    {
        // A tint saves a tint swatch ("Name 40%").
        let p = match tgt.tint() {
            Some(t) => json!({"swatch": t.swatch, "tint": t.tint * 100.0}),
            None => json!({"color": color_json(&c)}),
        };
        app.run("swatch.new", p).ok();
    }
    if menu_item(ui, tl!("Copy Color Value (Hex)"), color.is_some(), false)
        && let Some(c) = color
    {
        ui.ctx().copy_text(c.to_hex());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
        app
    }

    /// Run the panel for one frame with `events` and `modifiers`; returns the spectrum's rect (when
    /// the options are shown) and the panel's height.
    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>, modifiers: egui::Modifiers) -> (Option<Rect>, f32) {
        frame_with(app, ctx, events, modifiers, "color-spectrum")
    }

    /// [`frame`] returning the rect of the panel's widget `id` instead of the spectrum's.
    fn frame_with(
        app: &mut VectorcraftApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
        id: impl std::hash::Hash + std::fmt::Debug,
    ) -> (Option<Rect>, f32) {
        let mut out = (None, 0.0);
        let events = std::iter::once(egui::Event::ModifiersChanged(modifiers)).chain(events).collect();
        let input = egui::RawInput { events, ..Default::default() };
        let mut full = ctx.run_ui(input, |ui| {
            show(app, ui);
            out = (ctx.read_response(ui.id().with(&id)).map(|r| r.rect), ui.min_rect().height());
        });
        full.textures_delta.clear();
        out
    }

    /// Click the spectrum at (x, y) (0..1 each) with `modifiers` held.
    fn click_spectrum(app: &mut VectorcraftApp, ctx: &egui::Context, (x, y): (f32, f32), modifiers: egui::Modifiers) {
        let r = frame(app, ctx, vec![], modifiers).0.expect("the spectrum is drawn");
        let pos = egui::pos2(r.left() + x * r.width(), r.top() + y * r.height());
        let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
        frame(app, ctx, vec![egui::Event::PointerMoved(pos), button(true)], modifiers);
        frame(app, ctx, vec![button(false)], modifiers);
        frame(app, ctx, vec![], egui::Modifiers::NONE);
    }

    #[test]
    fn a_recent_colour_recolours_the_selected_gradient_stop() {
        // #835: it replaced the whole gradient with the colour.
        let mut app = app();
        app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
        let stops = json!([{"offset": 0, "color": "#ffffff"}, {"offset": 1, "color": "#000000"}]);
        app.run("paint.setFill", json!({"gradient": {"stops": stops, "start": [10, 35], "end": [60, 35]}})).unwrap();
        app.select_tool("gradient");
        app.run("gradient.selectStop", json!({"index": 1})).unwrap();
        let red = app.session.recent_colors.iter().position(|c| c.to_hex() == "#ff0000").expect("red is a recent colour");
        let ctx = egui::Context::default();
        let none = egui::Modifiers::NONE;
        let chip = frame_with(&mut app, &ctx, vec![], none, ("recent", red)).0.expect("the chip is drawn").center();
        let button = |pressed| egui::Event::PointerButton { pos: chip, button: egui::PointerButton::Primary, pressed, modifiers: none };
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(chip), button(true)], none);
        frame(&mut app, &ctx, vec![button(false)], none);
        let Paint::Gradient(g) = crate::panels::current_paints(&app).0 else { panic!("still a gradient") };
        let hex: Vec<String> = g.gradient.stops.iter().map(|s| s.color.to_hex()).collect();
        assert_eq!(hex, ["#ffffff", "#ff0000"]);
    }

    fn paints_hex(app: &VectorcraftApp) -> (String, String) {
        let (f, s) = crate::panels::current_paints(app);
        (f.color().map(|c| c.to_hex()).unwrap_or_default(), s.color().map(|c| c.to_hex()).unwrap_or_default())
    }

    #[test]
    fn a_drag_on_the_spectrum_holds_still_near_white() {
        let mut app = app();
        let ctx = egui::Context::default();
        let none = egui::Modifiers::NONE;
        // White needs no warning, #fefefe the web one, a pale violet both: one panel height.
        let heights: Vec<f32> = ["#ffffff", "#fefefe", "#ded3ff"]
            .iter()
            .map(|c| {
                app.run("paint.setFill", json!({"color": c})).unwrap();
                frame(&mut app, &ctx, vec![], none);
                frame(&mut app, &ctx, vec![], none).1
            })
            .collect();
        assert!(heights.windows(2).all(|w| w[0] == w[1]), "{heights:?}");
        // Drag along the spectrum's top edge (pale colours, some out of gamut), stopping at each
        // point: the colour holds while the pointer does.
        let r = frame(&mut app, &ctx, vec![], none).0.expect("the spectrum is drawn");
        let at = |x: f32| egui::pos2(r.left() + x * r.width(), r.top() + 2.0);
        let button = |pressed| egui::Event::PointerButton { pos: at(0.02), button: egui::PointerButton::Primary, pressed, modifiers: none };
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at(0.02)), button(true)], none);
        for i in 1..20 {
            frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at(i as f32 * 0.05))], none);
            let fills: Vec<String> = (0..3)
                .map(|_| {
                    frame(&mut app, &ctx, vec![], none);
                    paints_hex(&app).0
                })
                .collect();
            assert!(fills.windows(2).all(|w| w[0] == w[1]), "at {}: {fills:?}", i as f32 * 0.05);
        }
        frame(&mut app, &ctx, vec![button(false)], none);
    }

    #[test]
    fn tandem_keeps_ratios() {
        let t = tandem(Mode::Rgb, &[100.0, 50.0, 25.0], 0, 200.0);
        assert_eq!(t, vec![200.0, 100.0, 50.0]);
        // It stops where a component reaches its maximum.
        let t = tandem(Mode::Rgb, &[200.0, 100.0, 0.0], 1, 200.0);
        assert_eq!(t, vec![255.0, 127.5, 0.0]);
        let t = tandem(Mode::Cmyk, &[10.0, 40.0, 20.0, 5.0], 1, 20.0);
        assert_eq!(t, vec![5.0, 20.0, 10.0, 2.5]);
        // From 0 the others move by the same amount.
        assert_eq!(tandem(Mode::Rgb, &[0.0, 50.0, 250.0], 0, 10.0), vec![10.0, 60.0, 255.0]);
        assert!(Mode::Rgb.tandem() && Mode::Cmyk.tandem() && !Mode::Hsb.tandem() && !Mode::Grayscale.tandem());
    }

    #[test]
    fn the_mode_follows_the_colours_model() {
        let cmyk = Color::cmyk(0.1, 0.2, 0.3, 0.4);
        let rgb = Color::rgb(0.2, 0.4, 0.6);
        assert_eq!(display_mode(Some(Mode::Rgb), Some(&cmyk)), Mode::Cmyk);
        assert_eq!(display_mode(Some(Mode::Hsb), Some(&rgb)), Mode::Hsb, "HSB is remembered for RGB colours");
        assert_eq!(display_mode(Some(Mode::Cmyk), Some(&rgb)), Mode::Rgb);
        assert_eq!(display_mode(Some(Mode::Rgb), Some(&Color::gray(0.5))), Mode::Grayscale);
        assert_eq!(display_mode(Some(Mode::Cmyk), None), Mode::Cmyk);
        assert_eq!(convert_to(Mode::Hsb, &rgb), rgb);
        assert!(matches!(convert_to(Mode::Cmyk, &rgb), Color::Cmyk { .. }));
        assert_eq!(convert_to(Mode::WebSafe, &rgb).to_hex(), "#336699");
        assert_eq!(Mode::Rgb.next(), Mode::Hsb);
        assert_eq!(Mode::WebSafe.next(), Mode::Grayscale);
        // In the panel: an RGB pick, then a CMYK object shows its 4 sliders.
        let mut app = app();
        let ctx = egui::Context::default();
        set_pstate(&ctx, "color-mode", Some(Mode::Rgb));
        app.run("paint.setFill", json!({"color": {"c": 0.1, "m": 0.2, "y": 0.3, "k": 0.4}})).unwrap();
        frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
        let _ = ctx.run_ui(Default::default(), |ui| {
            let mode = display_mode(pstate(ui.ctx(), "color-mode"), target(&app, ui).color().as_ref());
            assert_eq!(mode.labels(), ["C", "M", "Y", "K"]);
        });
    }

    #[test]
    fn lab_colours_show_lab_sliders_with_signed_ranges() {
        let lab = Color::lab(50.0, -20.0, 30.0);
        assert_eq!(Mode::of(&lab), Mode::Lab);
        assert_eq!(display_mode(Some(Mode::Rgb), Some(&lab)), Mode::Lab);
        assert!(!Mode::ALL.contains(&Mode::Lab), "not a panel menu mode");
        assert_eq!(components(Mode::Lab, &lab), vec![50.0, -20.0, 30.0]);
        assert_eq!(from_components(Mode::Lab, &[50.0, -20.0, 30.0]), lab);
        assert_eq!(from_components(Mode::Lab, &[150.0, -300.0, 300.0]), Color::lab(100.0, -128.0, 127.0), "clamped");
        assert_eq!((Mode::Lab.unit(1, -128.0), Mode::Lab.unit(2, 127.0), Mode::Lab.unit(0, 50.0)), (0.0, 1.0, 0.5));
        assert_eq!((Mode::Lab.value_at(1, 0.0), Mode::Lab.value_at(0, 1.0)), (-128.0, 100.0));
        assert_eq!((Mode::Rgb.value_at(0, 0.5), Mode::WebSafe.value_at(0, 0.45)), (128.0, 102.0), "other modes as before");
        assert_eq!(track_color(Mode::Lab, &[50.0, -20.0, 30.0], 1, 0.0), Color::lab(50.0, -128.0, 30.0));
        assert_eq!(convert_to(Mode::Lab, &Color::rgb(1.0, 0.0, 0.0)).model(), vectorcraft_color::cms::Model::Lab);
        // In the panel: a Lab fill shows L, a, b.
        let mut app = app();
        let ctx = egui::Context::default();
        app.run("paint.setFill", json!({"color": {"l": 50, "a": -20, "b": 30}})).unwrap();
        frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
        let _ = ctx.run_ui(Default::default(), |ui| {
            let mode = display_mode(pstate(ui.ctx(), "color-mode"), target(&app, ui).color().as_ref());
            assert_eq!(mode.labels(), ["L", "a", "b"]);
        });
    }

    #[test]
    fn pure_blue_is_out_of_gamut_in_rgb() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.run("paint.setFill", json!({"color": "#0000ff"})).unwrap();
        frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
        let (of, fix) = pstate::<Option<(Color, Option<Color>)>>(&ctx, "color-gamut").expect("RGB mode checks the gamut");
        assert_eq!(of.to_hex(), "#0000ff");
        assert!(fix.is_some_and(|f| !f.out_of_gamut()));
        app.run("paint.setFill", json!({"color": "#808080"})).unwrap();
        frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
        assert_eq!(pstate::<Option<(Color, Option<Color>)>>(&ctx, "color-gamut").unwrap().1, None);
    }

    #[test]
    fn alt_click_on_the_spectrum_paints_the_stroke() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.run("paint.setFill", json!({"color": "#123456"})).unwrap();
        app.run("paint.setStroke", json!({"color": "#000000", "focus": false})).unwrap();
        assert!(app.session.fill_active);
        click_spectrum(&mut app, &ctx, (0.0, 0.5), egui::Modifiers::ALT);
        assert_eq!(paints_hex(&app), ("#123456".into(), "#ff0000".into()), "the stroke changes");
        assert!(app.session.fill_active, "the fill stays in front");
        // A plain click paints the fill.
        click_spectrum(&mut app, &ctx, (0.0, 0.5), egui::Modifiers::NONE);
        assert_eq!(paints_hex(&app).0, "#ff0000");
    }

    #[test]
    fn shift_click_on_the_spectrum_cycles_modes() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.run("paint.setFill", json!({"color": "#336699"})).unwrap();
        click_spectrum(&mut app, &ctx, (0.5, 0.5), egui::Modifiers::SHIFT);
        assert_eq!(pstate::<Option<Mode>>(&ctx, "color-mode"), Some(Mode::Hsb));
        assert_eq!(paints_hex(&app).0, "#336699", "HSB keeps the RGB colour");
        click_spectrum(&mut app, &ctx, (0.5, 0.5), egui::Modifiers::SHIFT);
        assert_eq!(pstate::<Option<Mode>>(&ctx, "color-mode"), Some(Mode::Cmyk));
        assert!(matches!(crate::panels::current_paints(&app).0.color(), Some(Color::Cmyk { .. })), "CMYK converts the colour");
    }

    #[test]
    fn double_clicking_a_proxy_opens_the_color_picker_for_it() {
        let mut app = app();
        let ctx = egui::Context::default();
        // One frame of the proxy alone; returns the stroke square's rect.
        let run = |app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>| {
            let input = egui::RawInput { time: Some(time), events, ..Default::default() };
            let mut rect = None;
            let mut out = ctx.run_ui(input, |ui| {
                crate::panels::proxy(app, ui, 40.0);
                rect = ctx.read_response(ui.id().with("stroke-proxy")).map(|r| r.rect);
            });
            out.textures_delta.clear();
            rect
        };
        let pos = run(&mut app, 0.0, vec![]).expect("the stroke square is drawn").right_bottom() - vec2(3.0, 3.0);
        let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        run(&mut app, 0.1, vec![egui::Event::PointerMoved(pos), button(true)]);
        run(&mut app, 0.15, vec![button(false)]);
        assert!(!app.session.fill_active, "a click brings the stroke to the front");
        assert!(app.ui.dialog.is_none());
        run(&mut app, 0.2, vec![button(true)]);
        run(&mut app, 0.25, vec![button(false)]);
        let d = app.ui.dialog.as_ref().expect("a double-click opens the Color Picker");
        assert_eq!(d.kind, "colorPicker");
        assert!(d.bool("stroke"));
    }

    #[test]
    fn a_global_colour_shows_tint_mode_and_the_t_slider_sets_its_tint() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.run("swatch.new", json!({"name": "Ink", "color": {"c": 0, "m": 1, "y": 0, "k": 0}, "spot": true})).unwrap();
        app.run("paint.setFill", json!({"swatch": "Ink", "tint": 50})).unwrap();
        frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
        let _ = ctx.run_ui(Default::default(), |ui| {
            let tgt = target(&app, ui);
            let t = tgt.tint().expect("Tint mode");
            assert_eq!((t.swatch.as_str(), t.spot, t.tint, t.base), ("Ink", true, 0.5, Color::cmyk(0.0, 1.0, 0.0, 0.0)));
        });
        // The chip's name and the T row are drawn; a click on the slider's right part raises the
        // tint and keeps the link.
        let texts = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            fn walk(s: &egui::Shape, out: &mut Vec<(String, egui::Pos2)>) {
                match s {
                    egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.pos + t.galley.rect.center().to_vec2())),
                    egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                    _ => {}
                }
            }
            let screen = Rect::from_min_size(egui::Pos2::ZERO, vec2(260.0, 600.0));
            let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: Some(screen), ..Default::default() }, |ui| show(app, ui));
            out.textures_delta.clear();
            let mut v = vec![];
            out.shapes.iter().for_each(|c| walk(&c.shape, &mut v));
            v
        };
        let drawn = texts(&mut app, vec![]);
        let at = |label: &str| drawn.iter().find(|(t, _)| t == label).map(|(_, p)| *p);
        assert!(at("Ink").is_some(), "the swatch's name: {drawn:?}");
        let (t_label, field) = (at("T").expect("the T row"), at("50%").expect("the tint field"));
        let pos = egui::pos2(field.x - 40.0, t_label.y);
        let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        texts(&mut app, vec![egui::Event::PointerMoved(pos), button(true)]);
        texts(&mut app, vec![button(false)]);
        let Paint::Solid { swatch, tint, .. } = crate::panels::current_paints(&app).0 else { panic!() };
        assert_eq!(swatch.as_deref(), Some("Ink"));
        assert!(tint > 0.6 && tint < 1.0, "{tint}");
        // Picking a mode makes it a process colour.
        let _ = ctx.run_ui(Default::default(), |ui| {
            let tgt = target(&app, ui);
            set_mode(&mut app, ui.ctx(), &tgt, Mode::Cmyk);
        });
        assert!(matches!(crate::panels::current_paints(&app).0, Paint::Solid { swatch: None, .. }));
    }

    #[test]
    fn hide_options_leaves_the_proxy_and_spectrum() {
        let mut app = app();
        let ctx = egui::Context::default();
        let (spectrum, full) = frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
        assert!(spectrum.is_some());
        set_pstate(&ctx, "color-hide-options", true);
        let (_, hidden) = frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
        assert!(hidden <= 44.0 && full > 150.0, "{hidden} / {full}");
    }

    #[test]
    fn a_mixed_selection_has_no_colour_to_edit() {
        let mut app = app();
        let ctx = egui::Context::default();
        let b = app.run("shape.rectangle", json!({"x": 80, "y": 10, "width": 50, "height": 50})).unwrap()["id"].clone();
        app.run("select.all", json!({})).unwrap();
        app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
        app.run("paint.setFill", json!({"color": "#00ff00", "ids": [b]})).unwrap();
        frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
        assert_eq!(crate::panels::mixed_paints(&app, &ctx), (true, false));
        let _ = ctx.run_ui(Default::default(), |ui| assert_eq!(target(&app, ui).color(), None));
        app.run("paint.setFill", json!({"color": "#0000ff"})).unwrap();
        assert_eq!(crate::panels::mixed_paints(&app, &ctx), (false, false), "a new revision recomputes it");
    }

    #[test]
    fn rgb_roundtrip() {
        let c = Color::rgb8(230, 120, 40);
        let v = components(Mode::Rgb, &c);
        assert_eq!(v.iter().map(|x| x.round() as i32).collect::<Vec<_>>(), vec![230, 120, 40]);
        assert_eq!(from_components(Mode::Rgb, &v).to_hex(), c.to_hex());
    }

    #[test]
    fn hsb_roundtrip_and_ranges() {
        let c = Color::rgb8(230, 120, 40);
        let v = components(Mode::Hsb, &c);
        assert!(v[0] > 0.0 && v[0] < 360.0 && v[1] <= 100.0 && v[2] <= 100.0);
        assert_eq!(from_components(Mode::Hsb, &v).to_hex(), c.to_hex());
        assert_eq!(Mode::Hsb.max(0), 360.0);
        assert_eq!(Mode::Hsb.suffix(0), "°");
    }

    #[test]
    fn cmyk_and_gray_keep_model() {
        let c = from_components(Mode::Cmyk, &[10.0, 20.0, 30.0, 40.0]);
        assert!(matches!(c, Color::Cmyk { .. }));
        let v = components(Mode::Cmyk, &c);
        assert!((v[3] - 40.0).abs() < 1e-3);
        let g = from_components(Mode::Grayscale, &[25.0]);
        assert_eq!(g, Color::gray(0.25));
        assert!((components(Mode::Grayscale, &g)[0] - 25.0).abs() < 1e-4);
        // Grayscale view of an RGB colour uses luminance.
        assert!((components(Mode::Grayscale, &Color::WHITE)[0]).abs() < 1e-4);
    }

    #[test]
    fn web_safe_snaps() {
        let c = Color::rgb8(230, 120, 40);
        let w = web_safe(&c);
        assert_eq!(w.to_hex(), "#ff6633");
        assert!(is_web_safe(&w));
        assert!(!is_web_safe(&c));
        assert_eq!(from_components(Mode::WebSafe, &[230.0, 120.0, 40.0]).to_hex(), "#ff6633");
    }

    #[test]
    fn slider_tracks_vary_one_component() {
        let comps = [230.0, 120.0, 40.0];
        let a = track_color(Mode::Rgb, &comps, 0, 0.0).to_rgba8(1.0);
        let b = track_color(Mode::Rgb, &comps, 0, 1.0).to_rgba8(1.0);
        assert_eq!((a[0], a[1], a[2]), (0, 120, 40));
        assert_eq!((b[0], b[1], b[2]), (255, 120, 40));
        let h = track_color(Mode::Hsb, &[0.0, 100.0, 100.0], 0, 1.0 / 3.0).to_hex();
        assert_eq!(h, "#00ff00");
    }

    #[test]
    fn spectrum_and_hex() {
        assert_eq!(spectrum_at(Mode::Rgb, 0.0, 0.0).to_hex(), "#ffffff");
        assert_eq!(spectrum_at(Mode::Rgb, 0.0, 0.5).to_hex(), "#ff0000");
        assert_eq!(spectrum_at(Mode::Rgb, 0.3, 1.0).to_hex(), "#000000");
        assert_eq!(spectrum_at(Mode::Grayscale, 1.0, 0.3), Color::gray(1.0));
        assert_eq!(parse_hex("E67828").unwrap().to_hex(), "#e67828");
        assert_eq!(parse_hex("#fff").unwrap().to_hex(), "#ffffff");
        assert!(parse_hex("zz").is_none());
        assert!(parse_hex("12345").is_none());
    }
}
