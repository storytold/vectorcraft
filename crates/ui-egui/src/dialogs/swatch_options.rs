//! Swatch Options: name, colour type (process or spot), Global, colour mode with sliders and hex,
//! and the out-of-gamut and out-of-web-color warnings under the colour, previewed live on the canvas
//! (Cancel rolls the preview back). A click on a warning's icon or colour corrects the colour
//! (https://web.archive.org/web/20210601162814/https://helpx.adobe.com/pdf/illustrator_reference.pdf,
//! Swatch options; https://helpx.adobe.com/illustrator/using/adjusting-colors.html).
//! Double-clicking a swatch opens it for a solid colour; for a gradient it shows the name and the
//! gradient; patterns open pattern editing ([`open`]).
//!
//! Fields: `__swatch` (the swatch being edited), `name`, `spot`, `global`, `mode` (`gray`, `rgb`,
//! `hsb`, `lab`, `cmyk` or `web`), `color` (`"#rrggbb"` or a colour object such as
//! `{"model": "cmyk", "c": 0.1, "m": 0.2, "y": 0.3, "k": 0}`) and `preview`; a gradient swatch has
//! `name`, `preview` and `__gradient` (the gradient shown) only.

use egui::vec2;
use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};

use super::{DialogSpec, form, run_and_close};
use crate::VectorcraftApp;
use crate::panels::color::{
    GAMUT_WARNING, Mode, WEB_WARNING, components, from_components, gamut_fix, is_web_safe, parse_hex, track_color, warning_chip, web_safe,
};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::widgets;

/// The dialog kind of Swatch Options.
pub const KIND: &str = "swatchOptions";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Swatch Options").into(), body, confirm, preview: true, min_width: 340.0, ..DialogSpec::FORM };

/// Colour modes, in menu order, with their `mode` field / `swatch.edit` ids.
const MODES: [(Mode, &str); 6] =
    [(Mode::Grayscale, "gray"), (Mode::Rgb, "rgb"), (Mode::Hsb, "hsb"), (Mode::Lab, "lab"), (Mode::Cmyk, "cmyk"), (Mode::WebSafe, "web")];

const TYPES: [&str; 2] = ["Process Color", "Spot Color"];

pub(super) fn mode_id(m: Mode) -> &'static str {
    MODES.iter().find(|x| x.0 == m).map_or("rgb", |x| x.1)
}

fn mode_of(d: &Dialog) -> Mode {
    let id = d.str("mode");
    MODES.iter().find(|x| x.1 == id).map_or(Mode::Rgb, |x| x.0)
}

/// The dialog's colour: a hex string or a serialized colour.
pub(super) fn color_of(d: &Dialog) -> Color {
    match d.fields.get("color") {
        Some(Value::String(s)) => Color::from_hex(s),
        Some(v) => serde_json::from_value(v.clone()).ok(),
        None => None,
    }
    .unwrap_or(Color::BLACK)
}

fn set_color(d: &mut Dialog, c: Color) {
    d.fields.insert("color".into(), json!(c));
}

/// Open the editor of swatch `name`: Swatch Options for a colour (a tint swatch's base) or a
/// gradient (its name only), pattern editing for a pattern.
pub fn open(app: &mut VectorcraftApp, name: &str) -> Result<Value, String> {
    let d = app.session.active().map(|st| &st.doc);
    let sw = d.and_then(|d| d.swatch(name)).ok_or_else(|| format!("no swatch `{name}`"))?;
    if let Some((base, _)) = sw.tint_of()
        && d.is_some_and(|d| d.swatch(base).is_some())
    {
        let base = base.to_string();
        return open(app, &base);
    }
    let sw = sw.clone();
    match &sw.paint {
        Paint::Solid { color, .. } => {
            let fields = json!({
                "__swatch": name, "name": name, "spot": sw.spot, "global": sw.global,
                "mode": mode_id(Mode::of(color)), "color": color, "preview": true,
            });
            app.ui.dialog = Some(Dialog::new(KIND, fields));
            Ok(Value::Null)
        }
        Paint::Gradient(g) => {
            let fields = json!({"__swatch": name, "name": name, "__gradient": g, "preview": true});
            app.ui.dialog = Some(Dialog::new(KIND, fields));
            Ok(Value::Null)
        }
        Paint::Pattern { pattern, .. } => {
            app.run("object.pattern.edit", json!({"name": pattern}))?;
            app.ui.open_panel = Some("patternOptions".into());
            Ok(Value::Null)
        }
        Paint::None => Err(format!("`{name}` has no options")),
    }
}

/// `swatch.edit` parameters from the fields (a gradient swatch only renames).
fn edit_params(d: &Dialog) -> Value {
    if d.fields.contains_key("__gradient") {
        return json!({"name": d.str("__swatch"), "newName": d.str("name")});
    }
    json!({
        "name": d.str("__swatch"),
        "newName": d.str("name"),
        "color": color_of(d),
        "mode": d.str("mode"),
        "global": d.bool("global"),
        "spot": d.bool("spot"),
    })
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let changed = match d.fields.get("__gradient").and_then(|g| serde_json::from_value::<vectorcraft_color::GradientPaint>(g.clone()).ok()) {
        Some(g) => {
            let changed = grid(ui, |ui| name_row(ui, d));
            ui.add_space(6.0);
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), egui::Sense::hover());
            widgets::gradient_chip(ui, r, &g);
            changed
        }
        None => editor(app, ui, d),
    };
    ui.add_space(8.0);
    let mut pv = d.bool("preview");
    let pv_changed = widgets::check(ui, tl!("Preview"), pv, true);
    if pv_changed {
        pv = !pv;
        d.fields.insert("preview".into(), json!(pv));
    }
    if pv && (changed || pv_changed || !app.session.in_interaction()) {
        // A failure to begin (typing that can't be kept) shows, and nothing previews.
        if let Err(e) = app.begin_interaction("Swatch Options") {
            app.status(e);
        } else if let Err(e) = app.session.preview("swatch.edit", &edit_params(d)) {
            app.status(e.to_string());
        }
    } else if !pv && pv_changed {
        let _ = app.session.cancel_interaction();
    }
    false
}

/// Drop the preview and apply the edit as one undo step (which also updates the default paints).
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let _ = app.session.cancel_interaction();
    run_and_close(app, "swatch.edit", edit_params(d))
}

/// The fields' grid of the swatch dialogs.
pub(super) fn grid<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Grid::new("swatch-options").num_columns(2).spacing([10.0, 8.0]).show(ui, add).inner
}

/// A dimmed field label in the swatch dialogs' grid.
pub(super) fn label(ui: &mut egui::Ui, s: &str) {
    crate::widgets::field_label(ui, egui::RichText::new(s).color(Tokens::get(ui.ctx()).text_dim));
}

/// The Swatch Name row of the grid. Returns true when it changed.
pub(super) fn name_row(ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    label(ui, tl!("Swatch Name:"));
    let changed = form::text(ui, d, "name", 190.0);
    ui.end_row();
    changed
}

/// The shared fields of Swatch Options and New Swatch: name, colour type, Global, mode, sliders,
/// hex and the colour warnings. Returns true when anything changed.
pub(super) fn editor(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let mut changed = false;
    grid(ui, |ui| {
        changed |= name_row(ui, d);
        label(ui, tl!("Color Type:"));
        let spot = d.bool("spot");
        if let Some(i) = widgets::dropdown(ui, "swatch-type", TYPES[usize::from(spot)], &TYPES, 200.0) {
            d.fields.insert("spot".into(), json!(i == 1));
            changed = true;
        }
        ui.end_row();
        ui.label("");
        // Spot colours are always global.
        let global = spot || d.bool("global");
        if widgets::check(ui, tl!("Global"), global, !spot) {
            d.fields.insert("global".into(), json!(!global));
            changed = true;
        }
        ui.end_row();
        label(ui, tl!("Color Mode:"));
        let labels = MODES.map(|m| m.0.label());
        if let Some(i) = widgets::dropdown(ui, "swatch-mode", mode_of(d).label(), &labels, 200.0) {
            let m = MODES[i].0;
            // Switching modes converts the colour (HSB is a view of RGB), like the Color panel.
            let base = if m == Mode::Hsb { Mode::Rgb } else { m };
            set_color(d, from_components(base, &components(base, &color_of(d))));
            d.fields.insert("mode".into(), json!(mode_id(m)));
            changed = true;
        }
        ui.end_row();
    });
    ui.add_space(6.0);
    changed | sliders(app, ui, d)
}

/// A warning under the colour of the swatch dialogs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Warning {
    Gamut,
    Web,
}

/// The warnings the swatch dialogs show in `mode`: out of gamut in RGB, HSB and Lab, and out of web
/// colors in every mode but Web Safe RGB.
fn shown_warnings(mode: Mode) -> (bool, bool) {
    (matches!(mode, Mode::Rgb | Mode::Hsb | Mode::Lab), mode != Mode::WebSafe)
}

/// The colour and mode a click on `warning` gives `c`, edited in `mode`: the nearest web-safe colour
/// (in RGB unless the mode is RGB or HSB), or the colour's CMYK equivalent in the active CMYK space
/// (in CMYK). `None` when the conversion returns no CMYK values.
fn corrected(app: &mut VectorcraftApp, warning: Warning, mode: Mode, c: &Color) -> Result<Option<(Color, Mode)>, String> {
    match warning {
        Warning::Web => Ok(Some((web_safe(c), if matches!(mode, Mode::Rgb | Mode::Hsb) { mode } else { Mode::Rgb }))),
        Warning::Gamut => {
            let r = app.run("color.convert", json!({"color": crate::panels::color_json(c), "to": "cmyk"}))?;
            let v = |i: usize| r["values"][i].as_f64().map(|v| v as f32);
            Ok(v(0).zip(v(1)).zip(v(2)).zip(v(3)).map(|(((c, m), y), k)| (Color::cmyk(c, m, y, k), Mode::Cmyk)))
        }
    }
}

/// One slider and value field per component of the dialog's mode, the hex field (RGB, HSB and Web),
/// a chip of the colour and under it the mode's warnings. Returns true when the colour changed.
fn sliders(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let mode = mode_of(d);
    let color = color_of(d);
    let (shows_gamut, shows_web) = shown_warnings(mode);
    let gamut = if shows_gamut { gamut_fix(app, ui.ctx(), "swatch-gamut", &color) } else { None };
    let web = (shows_web && !is_web_safe(&color)).then(|| web_safe(&color));
    let mut clicked: Option<Warning> = None;
    // Keep the displayed components while the colour is unchanged (hue survives S = 0 etc.).
    let key = format!("{}|{}", mode_id(mode), serde_json::to_string(&color).unwrap_or_default());
    let comps: Vec<f32> = match d.fields.get("__comps") {
        Some(v) if d.str("__compsKey") == key => serde_json::from_value(v.clone()).unwrap_or_else(|_| components(mode, &color)),
        _ => components(mode, &color),
    };
    let mut new: Option<(Color, Vec<f32>)> = None;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            for (i, lbl) in mode.labels().iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.add_sized(vec2(12.0, 22.0), egui::Label::new(egui::RichText::new(*lbl).size(12.5).color(t.text)));
                    let v = comps.get(i).copied().unwrap_or(0.0);
                    let track = |x: f32| crate::panels::c32(&track_color(mode, &comps, i, x));
                    let shown = widgets::SliderValue::field(v as f64, mode.min(i) as f64..=mode.max(i) as f64, mode.suffix(i), 0);
                    if let (Some(nv), _) = widgets::color_slider_valued(ui, ("swatch-slider", i), lbl, Some(shown), mode.unit(i, v), 200.0, &track) {
                        let mut c2 = comps.clone();
                        c2[i] = mode.value_at(i, nv);
                        new = Some((from_components(mode, &c2), c2));
                    }
                    let bounds = f64::from(mode.min(i))..=f64::from(mode.max(i));
                    if let Some(fv) = widgets::range_field(ui, ("swatch-field", i), f64::from(v), bounds, mode.suffix(i), 0, 52.0) {
                        let mut c2 = comps.clone();
                        c2[i] = (fv as f32).clamp(mode.min(i), mode.max(i));
                        new = Some((from_components(mode, &c2), c2));
                    }
                });
            }
            if matches!(mode, Mode::Rgb | Mode::Hsb | Mode::WebSafe) {
                ui.horizontal(|ui| {
                    ui.add_sized(vec2(12.0, 22.0), egui::Label::new(egui::RichText::new("#").size(12.5).color(t.text)));
                    let hex = color.to_hex().trim_start_matches('#').to_uppercase();
                    if let Some(c) = hex_field(ui, &hex) {
                        let c = if mode == Mode::WebSafe { web_safe(&c) } else { c };
                        new = Some((c, components(mode, &c)));
                    }
                });
            }
        });
        ui.vertical(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(44.0, 44.0), egui::Sense::hover());
            widgets::swatch_tile(ui, r, &Paint::solid(color), false, false);
            let warnings: Vec<_> = [(Warning::Gamut, GAMUT_WARNING, gamut), (Warning::Web, WEB_WARNING, web)]
                .into_iter()
                .filter_map(|(w, chip, fix)| Some((w, chip, fix?)))
                .collect();
            for (w, chip, fix) in &warnings {
                ui.add_space(4.0);
                if warning_chip(ui, *chip, fix) {
                    clicked = Some(*w);
                }
            }
            // Room for the mode's other warnings, so the dialog keeps its height as the colour changes.
            for _ in warnings.len()..usize::from(shows_gamut) + usize::from(shows_web) {
                ui.add_space(4.0);
                ui.scope(|ui| {
                    ui.set_invisible();
                    warning_chip(ui, WEB_WARNING, &Color::WHITE)
                });
            }
        });
    });
    let mut mode = mode;
    if let Some(w) = clicked {
        match corrected(app, w, mode, &color) {
            Ok(Some((c, m))) => {
                mode = m;
                d.fields.insert("mode".into(), json!(mode_id(m)));
                new = Some((c, components(m, &c)));
            }
            Ok(None) => {}
            Err(e) => app.status(e),
        }
    }
    let Some((c, comps)) = new else { return false };
    set_color(d, c);
    d.fields.insert("__compsKey".into(), json!(format!("{}|{}", mode_id(mode), serde_json::to_string(&c).unwrap_or_default())));
    d.fields.insert("__comps".into(), json!(comps));
    // The warnings above were drawn for the colour before the change.
    ui.ctx().request_repaint();
    true
}

/// A hex field showing `hex`; returns the colour typed into it once committed.
fn hex_field(ui: &mut egui::Ui, hex: &str) -> Option<Color> {
    let t = Tokens::get(ui.ctx());
    let id = ui.scope_id().with("swatch-hex");
    widgets::hex_text(ui, id, hex, |ui, buf, editing| {
        egui::Frame::NONE
            .fill(t.input)
            .stroke(egui::Stroke::new(1.0, if editing { t.accent } else { t.input_border }))
            .corner_radius(2)
            .inner_margin(egui::Margin::symmetric(6, 4))
            .show(ui, |ui| {
                crate::theme::strong_selected_text(ui);
                ui.add(
                    egui::TextEdit::singleline(buf)
                        .id(id)
                        .frame(egui::Frame::NONE)
                        .align(egui::Align2::LEFT_CENTER)
                        .desired_width(70.0)
                        .char_limit(widgets::HEX_FIELD_CHARS),
                )
            })
            .inner
    })
    .as_deref()
    .and_then(parse_hex)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// A test context with the application's fonts and default theme.
    pub(crate) fn context() -> egui::Context {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, Default::default());
        ctx
    }

    /// Runs one frame of the open dialog with `events` and returns the rectangles it paints.
    pub(crate) fn draw(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>) -> Vec<egui::Rect> {
        fn collect(s: &egui::Shape, out: &mut Vec<egui::Rect>) {
            match s {
                egui::Shape::Rect(r) => out.push(r.rect),
                egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
                _ => {}
            }
        }
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1280.0, 900.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
        let mut boxes = vec![];
        out.shapes.iter().for_each(|c| collect(&c.shape, &mut boxes));
        boxes
    }

    /// Lets the auto-sized dialog settle and returns the rectangles of the next frame.
    pub(crate) fn settled(ctx: &egui::Context, app: &mut VectorcraftApp) -> Vec<egui::Rect> {
        for _ in 0..6 {
            draw(ctx, app, vec![]);
        }
        draw(ctx, app, vec![])
    }

    /// A primary-button press and release at `at`, in two frames.
    pub(crate) fn click_at(ctx: &egui::Context, app: &mut VectorcraftApp, at: egui::Pos2) {
        for pressed in [true, false] {
            let button = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            draw(ctx, app, vec![egui::Event::PointerMoved(at), button]);
        }
    }

    /// The warning rows under the dialog's 44-point colour chip, top first: the centre of each
    /// row's icon and the rectangle of its corrected colour.
    pub(crate) fn warning_rows(boxes: &[egui::Rect]) -> Vec<(egui::Pos2, egui::Rect)> {
        let near = |a: f32, b: f32| (a - b).abs() < 0.5;
        let chip = boxes.iter().find(|r| near(r.width(), 44.0) && near(r.height(), 44.0)).copied().unwrap();
        let mut rows: Vec<egui::Rect> = boxes
            .iter()
            .filter(|r| near(r.width(), 16.0) && near(r.height(), 16.0))
            .filter(|r| r.top() >= chip.bottom() && r.left() >= chip.left() - 1.0 && r.right() <= chip.right() + 1.0)
            .copied()
            .collect();
        rows.sort_by(|a, b| a.top().total_cmp(&b.top()));
        rows.dedup_by(|a, b| near(a.top(), b.top())); // The tile's fill and border share one rectangle.
        rows.into_iter().map(|r| (r.center() - vec2(19.0, 0.0), r)).collect()
    }

    fn dialog(app: &VectorcraftApp) -> &Dialog {
        app.ui.dialog.as_ref().unwrap()
    }

    #[test]
    fn swatch_options_shows_and_corrects_both_warnings() {
        let (ctx, mut app) = (context(), VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default()));
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("swatch.new", json!({"name": "Sky", "color": "#0000fe", "spot": true})).unwrap();
        app.run("ui.swatchOptions", json!({"name": "Sky"})).unwrap();
        let rows = warning_rows(&settled(&ctx, &mut app));
        assert_eq!(rows.len(), 2, "out of gamut and out of web colors");
        // The cube's icon shifts to the nearest web-safe colour, in the same mode.
        click_at(&ctx, &mut app, rows[1].0);
        assert_eq!((color_of(dialog(&app)).to_hex(), dialog(&app).str("mode")), ("#0000ff".to_string(), "rgb".to_string()));
        let rows = warning_rows(&settled(&ctx, &mut app));
        assert_eq!(rows.len(), 1, "web-safe blue is still out of gamut");
        // The triangle's colour shifts to the CMYK equivalent, edited in CMYK.
        let ink = corrected(&mut app, Warning::Gamut, Mode::Rgb, &Color::rgb8(0, 0, 255)).unwrap().unwrap().0;
        click_at(&ctx, &mut app, rows[0].1.center());
        assert_eq!((color_of(dialog(&app)), dialog(&app).str("mode")), (ink, "cmyk".to_string()));
        assert!(app.ui.dialog.is_some(), "a correction only edits the fields");
        super::super::confirm(&mut app).unwrap();
        let sw = app.session.active().unwrap().doc.swatch("Sky").cloned().unwrap();
        assert_eq!((sw.paint.color(), sw.spot, sw.global), (Some(ink), true, true), "still a spot colour");
        app.run("edit.undo", json!({})).unwrap();
        let sw = app.session.active().unwrap().doc.swatch("Sky").cloned().unwrap();
        assert_eq!(sw.paint.color().map(|c| c.to_hex()), Some("#0000fe".into()), "one undo step");
    }

    #[test]
    fn swatch_warnings_follow_the_mode() {
        let (ctx, mut app) = (context(), VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default()));
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("swatch.new", json!({"name": "Mid", "color": "#808080"})).unwrap();
        app.run("ui.swatchOptions", json!({"name": "Mid"})).unwrap();
        let mut rows_in = |mode: &str, c: Color| {
            let d = app.ui.dialog.as_mut().unwrap();
            d.fields.insert("mode".into(), json!(mode));
            set_color(d, c);
            warning_rows(&settled(&ctx, &mut app)).len()
        };
        assert_eq!(rows_in("gray", Color::gray(0.5)), 1, "a gray off the web palette, never out of gamut");
        let cyan = Color::cmyk(1.0, 0.0, 0.0, 0.0);
        assert_eq!(rows_in("cmyk", cyan), usize::from(!is_web_safe(&cyan)), "CMYK is never out of gamut");
        assert_eq!(rows_in("web", Color::rgb8(0, 0, 255)), 0, "Web Safe RGB shows no warning");
        assert_eq!(rows_in("rgb", Color::rgb8(0, 0, 255)), 1, "RGB blue is out of gamut");
        assert_eq!(rows_in("lab", Color::lab(30.0, 60.0, -100.0)), 2, "Lab shows both");
        // A web correction of a Lab colour is edited in RGB.
        let rows = warning_rows(&settled(&ctx, &mut app));
        click_at(&ctx, &mut app, rows[1].0);
        assert_eq!(dialog(&app).str("mode"), "rgb");
        assert!(is_web_safe(&color_of(dialog(&app))));
    }

    #[test]
    fn fields_round_trip_modes_and_colours() {
        for (m, id) in MODES {
            assert_eq!(mode_of(&Dialog::new(KIND, json!({"mode": id}))), m);
        }
        let d = Dialog::new(KIND, json!({"color": "#ff8000"}));
        assert_eq!(color_of(&d).to_hex(), "#ff8000");
        let mut d = Dialog::new(KIND, json!({}));
        set_color(&mut d, Color::cmyk(0.1, 0.2, 0.3, 0.4));
        assert_eq!(color_of(&d), Color::cmyk(0.1, 0.2, 0.3, 0.4));
        let p = edit_params(&Dialog::new(KIND, json!({"__swatch": "A", "name": "B", "spot": true, "color": "#000000", "mode": "cmyk"})));
        assert_eq!(
            (p["name"].as_str(), p["newName"].as_str(), p["spot"].as_bool(), p["mode"].as_str()),
            (Some("A"), Some("B"), Some(true), Some("cmyk"))
        );
    }

    #[test]
    fn lab_mode_defines_a_spot_colour_in_lab() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("swatch.new", json!({"name": "Ink", "color": {"c": 0, "m": 80, "y": 60, "k": 0}, "spot": true})).unwrap();
        app.run("ui.swatchOptions", json!({"name": "Ink"})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("mode".into(), json!("lab"));
        // Drawn headlessly: three Lab sliders (a and b signed) preview the edit.
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(Default::default(), |ui| super::super::show(&mut app, ui.ctx()));
        out.textures_delta.clear();
        assert_eq!(mode_of(app.ui.dialog.as_ref().unwrap()).labels(), ["L", "a", "b"]);
        super::super::confirm(&mut app).unwrap();
        let ink = app.session.active().unwrap().doc.swatch("Ink").unwrap().paint.color().unwrap();
        assert_eq!(ink.model(), vectorcraft_color::cms::Model::Lab);
        assert_eq!(ink.to_hex(), Color::cmyk(0.0, 0.8, 0.6, 0.0).to_hex(), "the same colour, now in Lab");
        // Lab values typed into the fields.
        app.run("ui.swatchOptions", json!({"name": "Ink"})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("mode"), "lab", "opens in the colour's mode");
        app.ui.dialog.as_mut().unwrap().fields.insert("color".into(), json!(Color::lab(40.0, -30.0, 20.0)));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(app.session.active().unwrap().doc.swatch("Ink").unwrap().paint.color(), Some(Color::lab(40.0, -30.0, 20.0)));
    }
}
