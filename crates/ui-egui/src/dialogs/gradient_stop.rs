//! The gradient stop popover: double-clicking a stop on the Gradient tool's annotator (or the
//! Gradient panel's slider) edits it next to its chip — its colour (the Color panel's controls,
//! or a document swatch, which stays linked when it is a global or spot colour), opacity and
//! location. It edits the selected stop (`gradient.selectStop`) through `paint.editGradient`, and
//! closes on Escape or a click elsewhere. On a freeform gradient it edits the selected point
//! (`paint.freeform.selectPoint`) instead: its colour, opacity and spread, through
//! `paint.freeform.setPoint`.
//!
//! Fields: `index`, `x`, `y` (the chip, document coordinates) or `screen` ([x, y], screen points,
//! from the panel), and `tab` (`color` or `swatches`).
//! Agents can set `color` (hex), `opacity` and `location` (a point: `spread`) as percentages,
//! and confirm.

use egui::{Sense, vec2};
use serde_json::{Value, json};
use vectorcraft_color::gradient::move_stop;
use vectorcraft_color::{Color, FreeformPoint, GradientKind, GradientStop, Paint};
use vectorcraft_geom::Point;
use vectorcraft_tools::params::color_json;

use super::{DialogResult, DialogSpec};
use crate::VectorcraftApp;
use crate::canvas::Xf;
use crate::panels::active_paint;
use crate::panels::gradient::{edit_point, set_stops, shown_points};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::widgets::{self, Live};

pub(super) const SPEC: DialogSpec = DialogSpec::window(show, confirm);

/// The field holding the frame the popover was first drawn in.
const OPENED: &str = "__opened";

/// What the popover edits: the selected stop of the gradient behind the active proxy (with all
/// its stops), or the selected point of a freeform gradient.
enum Target {
    Stop(Vec<GradientStop>, usize),
    Point(FreeformPoint, usize),
}

impl Target {
    fn color(&self) -> Color {
        match self {
            Target::Stop(stops, i) => stops[*i].color,
            Target::Point(p, _) => p.color,
        }
    }
    fn opacity(&self) -> f32 {
        match self {
            Target::Stop(stops, i) => stops[*i].opacity,
            Target::Point(p, _) => p.opacity,
        }
    }
}

fn selected(app: &VectorcraftApp) -> Option<Target> {
    let Paint::Gradient(g) = active_paint(app) else { return None };
    if g.gradient.kind == GradientKind::Freeform {
        let f = shown_points(&g);
        let i = app.session.selected_freeform_point().filter(|i| *i < f.points.len())?;
        return Some(Target::Point(f.points[i], i));
    }
    let i = app.session.selected_stop().filter(|i| *i < g.gradient.stops.len())?;
    Some(Target::Stop(g.gradient.stops, i))
}

/// `stops` with stop `i` given `color`, linked to `link` (a global swatch and tint) or unlinked.
fn recolor(mut stops: Vec<GradientStop>, i: usize, color: Color, link: Option<(String, f32)>) -> Vec<GradientStop> {
    stops[i].set_color(color, link);
    stops
}

/// Solid colours of the document's swatches (groups included), with their names.
fn swatch_colors(app: &VectorcraftApp) -> Vec<(String, Color)> {
    let Some(st) = app.session.active() else { return vec![] };
    let d = &st.doc;
    d.swatches.iter().chain(d.swatch_groups.iter().flat_map(|g| g.swatches.iter())).filter_map(|s| Some((s.name.clone(), s.paint.color()?))).collect()
}

fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(d) = app.ui.dialog.clone() else { return };
    // The stop or point went away (undo, another object selected): nothing left to edit.
    let Some(target) = selected(app) else {
        app.ui.dialog = None;
        return;
    };
    let t = Tokens::get(ctx);
    // Opened from the Gradient panel: at `screen`; from the annotator: beside the chip.
    let screen =
        d.fields.get("screen").and_then(Value::as_array).and_then(|a| Some(egui::pos2(a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32)));
    let chip = Point::new(d.f64("x", 0.0), d.f64("y", 0.0));
    let area = egui::Id::new("gradient-stop-popover");
    let size = ctx.memory(|m| m.area_rect(area)).map_or(POPOVER_SIZE, |r| r.size());
    let pos = screen.unwrap_or_else(|| match app.canvas_rect.zip(app.view().copied()) {
        Some((canvas, v)) => {
            let xf = Xf::new(canvas, &v);
            let at = xf.to_screen(chip);
            selection_on_screen(app, &xf).map_or(at + vec2(14.0, 14.0), |art| beside_art(at, art, size, canvas))
        }
        None => ctx.content_rect().center(),
    });
    let swatches = d.str("tab") == "swatches";
    let mut tab = None;
    let resp = egui::Area::new(area)
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .constrain(true)
        .show(ctx, |ui| {
            egui::Frame::popup(&ctx.global_style()).fill(t.panel).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                ui.set_width(250.0);
                ui.horizontal(|ui| {
                    for (icon, tip, on) in [("palette", tl!("Color"), !swatches), ("swatch-book", tl!("Swatches"), swatches)] {
                        if widgets::icon_button(ui, icon, tip, on, 24.0).clicked() {
                            tab = Some(if icon == "palette" { "color" } else { "swatches" });
                        }
                    }
                });
                ui.add_space(6.0);
                if swatches {
                    swatch_grid(app, ui, &target);
                } else {
                    // The Color panel edits the selected stop (or point) of the active gradient.
                    crate::panels::color::show(app, ui);
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    widgets::dim_label(ui, tl!("Opacity:"));
                    if let Some(v) = widgets::plain_field(ui, "stop-pop-op", target.opacity() as f64 * 100.0, "%", 0, 54.0) {
                        apply(app, &target, &json!({ "opacity": v }));
                    }
                    let (label, key, value) = match &target {
                        Target::Stop(stops, i) => (tl!("Location:"), "location", stops[*i].offset),
                        Target::Point(p, _) => (tl!("Spread:"), "spread", p.spread),
                    };
                    widgets::dim_label(ui, label);
                    if let Some(v) = widgets::plain_field(ui, ("stop-pop", key), value as f64 * 100.0, "%", 1, 54.0) {
                        apply(app, &target, &json!({ key: v }));
                    }
                });
            });
        })
        .response;
    // The frame it opened in (its first drawn).
    let frame = ctx.cumulative_frame_nr();
    let opened = d.fields.get(OPENED).and_then(Value::as_u64).unwrap_or(frame);
    if let Some(d) = app.ui.dialog.as_mut() {
        if let Some(tab) = tab {
            d.fields.insert("tab".into(), json!(tab));
        }
        d.fields.insert(OPENED.into(), json!(opened));
    }
    // The click that opened the popover (the Gradient panel's colour chip) or the double-click
    // (a stop's chip) lands outside it: only a later click elsewhere closes it (#577).
    if frame > opened && resp.clicked_elsewhere() && !ctx.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary)) {
        app.ui.dialog = None;
    }
}

/// The popover's size before it has been drawn.
const POPOVER_SIZE: egui::Vec2 = vec2(272.0, 360.0);

/// The selected art's box on the screen.
fn selection_on_screen(app: &VectorcraftApp, xf: &Xf) -> Option<egui::Rect> {
    let st = app.session.active()?;
    let b = st.doc.bounds_of(&st.selection.objects, false)?;
    Some(egui::Rect::from_points(&xf.quad(b)))
}

/// Where the popover opened from a stop's chip at `chip` goes: beside the selected `art` on the
/// first side of it with room for a popover of `size` in `room` (right, left, below, above), so
/// the art it recolours stays in view (#862); else beside the chip.
fn beside_art(chip: egui::Pos2, art: egui::Rect, size: egui::Vec2, room: egui::Rect) -> egui::Pos2 {
    const GAP: f32 = 12.0;
    let x = (chip.x - size.x / 2.0).clamp(room.left(), (room.right() - size.x).max(room.left()));
    let y = (chip.y - size.y / 2.0).clamp(room.top(), (room.bottom() - size.y).max(room.top()));
    [
        egui::pos2(art.right() + GAP, y),
        egui::pos2(art.left() - GAP - size.x, y),
        egui::pos2(x, art.bottom() + GAP),
        egui::pos2(x, art.top() - GAP - size.y),
    ]
    .into_iter()
    .find(|p| room.contains_rect(egui::Rect::from_min_size(*p, size)))
    .unwrap_or(chip + vec2(14.0, 14.0))
}

/// Document swatches as tiles: a click gives the stop (or point) that colour.
fn swatch_grid(app: &mut VectorcraftApp, ui: &mut egui::Ui, target: &Target) {
    let colors = swatch_colors(app);
    if colors.is_empty() {
        widgets::dim_label(ui, tl!("The document has no colour swatches."));
        return;
    }
    let current = target.color();
    let mut pick = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
        for (name, c) in &colors {
            let (r, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
            widgets::swatch_tile(ui, r, &Paint::solid(*c), current == *c, resp.hovered());
            if resp.on_hover_text(name).clicked() {
                pick = Some((name.clone(), *c));
            }
        }
    });
    let Some((name, c)) = pick else { return };
    match target {
        // A global or spot colour (or a tint of one) stays linked.
        Target::Stop(stops, i) => {
            let link = crate::panels::gradient::dropped_link(app, &json!({ "swatch": name }));
            set_stops(app, &recolor(stops.clone(), *i, c, link), Some(*i), Live::Released);
        }
        Target::Point(_, i) => edit_point(app, "paint.freeform.setPoint", json!({ "index": i, "color": color_json(&c) })),
    }
}

/// Apply the `color` (hex), `opacity` and `location` or `spread` (percentages) set in `f` to the
/// target as one edit; returns the stop's (or point's) index after it.
fn apply(app: &mut VectorcraftApp, target: &Target, f: &Value) -> usize {
    let pct = |k: &str| f.get(k).and_then(Value::as_f64).map(|v| (v / 100.0).clamp(0.0, 1.0));
    let color = f.get("color").and_then(Value::as_str).and_then(crate::panels::color::parse_hex);
    match target {
        Target::Stop(stops, i) => {
            let (mut stops, mut i) = (stops.clone(), *i);
            if let Some(c) = color {
                stops = recolor(stops, i, c, None);
            }
            if let Some(o) = pct("opacity") {
                stops[i].opacity = o as f32;
            }
            if let Some(l) = f.get("location").and_then(Value::as_f64) {
                (stops, i) = move_stop(&stops, i, (l / 100.0) as f32);
            }
            set_stops(app, &stops, Some(i), Live::Released);
            i
        }
        Target::Point(_, i) => {
            let mut p = json!({ "index": i });
            if let Some(c) = color {
                p["color"] = color_json(&c);
            }
            for k in ["opacity", "spread"] {
                if let Some(v) = pct(k) {
                    p[k] = json!(v);
                }
            }
            edit_point(app, "paint.freeform.setPoint", p);
            *i
        }
    }
}

/// OK (agents): apply the `color`, `opacity` and `location` (a point: `spread`) fields that are
/// set, then close.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> DialogResult {
    let target = selected(app).ok_or("no gradient stop or freeform point is selected")?;
    if let Some(hex) = d.fields.get("color").and_then(Value::as_str) {
        crate::panels::color::parse_hex(hex).ok_or_else(|| format!("bad colour `{hex}` (#rrggbb)"))?;
    }
    let i = apply(app, &target, &Value::Object(d.fields.clone()));
    app.ui.dialog = None;
    Ok(json!({ "index": i }))
}

#[cfg(test)]
mod tests {
    use vectorcraft_engine::Session;
    use vectorcraft_tools::{PointerEvent, PointerKind};

    use super::*;

    /// A rectangle with a three-stop gradient from (100, 150) to (200, 150), the Gradient tool
    /// and stop 1 selected.
    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
        let stops = json!([{"offset": 0, "color": "#ffffff"}, {"offset": 0.5, "color": "#00ff00"}, {"offset": 1, "color": "#000000"}]);
        app.run("paint.setFill", json!({"gradient": {"stops": stops, "start": [100, 150], "end": [200, 150]}})).unwrap();
        app.select_tool("gradient");
        app.run("gradient.selectStop", json!({"index": 1})).unwrap();
        app
    }

    /// #862: the popover goes beside the art, on the first side with room, not over it.
    #[test]
    fn the_popover_goes_beside_the_art_it_edits() {
        let room = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1200.0, 800.0));
        let size = egui::vec2(272.0, 360.0);
        let place = |art: egui::Rect| egui::Rect::from_min_size(beside_art(art.center(), art, size, room), size);
        // Room on the right.
        let art = egui::Rect::from_min_max(egui::pos2(300.0, 300.0), egui::pos2(500.0, 450.0));
        assert!(place(art).left() > art.right() && room.contains_rect(place(art)));
        // Against the right edge: on the left.
        let art = egui::Rect::from_min_max(egui::pos2(900.0, 300.0), egui::pos2(1150.0, 450.0));
        assert!(place(art).right() < art.left());
        // As wide as the room: below it, or above it near the bottom.
        let art = egui::Rect::from_min_max(egui::pos2(100.0, 20.0), egui::pos2(1100.0, 300.0));
        assert!(place(art).top() > art.bottom());
        let art = egui::Rect::from_min_max(egui::pos2(100.0, 420.0), egui::pos2(1100.0, 780.0));
        assert!(place(art).bottom() < art.top());
        // Nowhere to go: beside the chip, as before.
        let art = room.shrink(10.0);
        assert_eq!(beside_art(art.center(), art, size, room), art.center() + egui::vec2(14.0, 14.0));
    }

    fn stops(app: &VectorcraftApp) -> Vec<GradientStop> {
        match selected(app) {
            Some(Target::Stop(stops, _)) => stops,
            _ => panic!("no stop selected"),
        }
    }

    thread_local! {
        // One context per test, so frames follow one another as in the app.
        static CTX: egui::Context = {
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            ctx
        };
    }

    /// One headless frame of the dialog layer and the shortcuts.
    fn frame(app: &mut VectorcraftApp, events: Vec<egui::Event>) {
        let ctx = CTX.with(Clone::clone);
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            crate::shortcuts::handle(app, ui.ctx());
            super::super::show(app, ui.ctx());
        });
        out.textures_delta.clear();
    }

    fn key(k: egui::Key) -> egui::Event {
        egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }
    }

    #[test]
    fn double_clicking_a_stop_opens_the_popover_which_draws_both_tabs() {
        let mut app = app();
        let view = app.view_info();
        let chip = Point::new(150.0, 150.0 + 10.0 / view.zoom);
        crate::canvas::dispatch(&mut app, &PointerEvent { kind: PointerKind::DoubleClick, pos: chip, mods: Default::default(), pressure: 1.0 }, view);
        let d = app.ui.dialog.clone().expect("the popover opened");
        assert_eq!((d.kind.as_str(), d.fields["index"].clone(), d.str("tab")), ("gradientStop", json!(1), "color".into()));
        assert_eq!(app.session.selected_stop(), Some(1));
        frame(&mut app, vec![]);
        app.ui.dialog.as_mut().unwrap().fields.insert("tab".into(), json!("swatches"));
        frame(&mut app, vec![]);
        assert!(app.ui.dialog.is_some(), "still open");
        // Escape closes it.
        frame(&mut app, vec![key(egui::Key::Escape)]);
        assert!(app.ui.dialog.is_none());
    }

    #[test]
    fn confirm_applies_colour_opacity_and_location() {
        let mut app = app();
        app.ui.dialog = Some(Dialog::new("gradientStop", json!({"index": 1, "x": 150, "y": 160, "color": "#ff0000", "opacity": 40, "location": 80})));
        assert_eq!(super::super::confirm(&mut app).unwrap(), json!({"index": 1}));
        let s = stops(&app);
        assert_eq!((s[1].color.to_hex(), s[1].opacity, (s[1].offset * 100.0).round()), ("#ff0000".to_string(), 0.4, 80.0));
        assert!(app.ui.dialog.is_none());
        // Moving it past the last stop keeps it selected at its new index.
        app.ui.dialog = Some(Dialog::new("gradientStop", json!({"location": 100})));
        super::super::confirm(&mut app).unwrap();
        assert_eq!((app.session.selected_stop(), stops(&app)[2].color.to_hex()), (Some(2), "#ff0000".to_string()));
        app.ui.dialog = Some(Dialog::new("gradientStop", json!({"color": "red"})));
        assert!(super::super::confirm(&mut app).is_err());
    }

    fn click(at: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
    }

    #[test]
    fn the_opening_double_click_keeps_it_open_and_a_later_click_elsewhere_closes_it() {
        let mut app = app();
        app.ui.dialog = Some(Dialog::new("gradientStop", json!({"index": 1, "x": 150, "y": 160})));
        // The frame the double-click lands in (the popover opens beside the pointer, not under it).
        let at = egui::pos2(5.0, 5.0);
        let double = vec![egui::Event::PointerMoved(at), click(at, true), click(at, false), click(at, true), click(at, false)];
        frame(&mut app, double);
        assert!(app.ui.dialog.is_some(), "the opening double-click must not close it");
        frame(&mut app, vec![egui::Event::PointerMoved(at), click(at, true), click(at, false)]);
        assert!(app.ui.dialog.is_none(), "a click elsewhere closes it");
    }

    /// #577: the Gradient panel's colour chip opens it with a click whose release lands in the
    /// frame it opens in, outside it: that click doesn't close it.
    #[test]
    fn the_click_that_opens_it_from_the_panel_keeps_it_open() {
        let mut app = app();
        let at = egui::pos2(5.0, 5.0);
        frame(&mut app, vec![egui::Event::PointerMoved(at), click(at, true)]);
        // The panel opens it while the click is released.
        app.ui.dialog = Some(Dialog::new("gradientStop", json!({"index": 1, "screen": [200.0, 200.0], "tab": "color"})));
        frame(&mut app, vec![click(at, false)]);
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        assert!(app.ui.dialog.is_some(), "the opening click must not close it");
        let away = egui::pos2(5.0, 600.0);
        frame(&mut app, vec![egui::Event::PointerMoved(away), click(away, true), click(away, false)]);
        assert!(app.ui.dialog.is_none(), "a later click elsewhere closes it");
    }

    #[test]
    fn the_popover_closes_when_its_stop_goes_away() {
        let mut app = app();
        app.ui.dialog = Some(Dialog::new("gradientStop", json!({"index": 1, "x": 150, "y": 160})));
        app.run("gradient.selectStop", json!({"index": null})).unwrap();
        frame(&mut app, vec![]);
        assert!(app.ui.dialog.is_none());
    }

    #[test]
    fn delete_removes_the_selected_stop_instead_of_the_object() {
        let mut app = app();
        frame(&mut app, vec![key(egui::Key::Delete)]);
        assert_eq!(stops(&app).len(), 2);
        frame(&mut app, vec![key(egui::Key::Backspace)]);
        assert_eq!(stops(&app).len(), 2, "never below two stops");
        assert_eq!(app.session.doc().unwrap().selection.objects.len(), 1, "the rectangle stays");
        // The arrows nudge it.
        let before = stops(&app)[1].offset;
        frame(&mut app, vec![key(egui::Key::ArrowLeft)]);
        assert!((stops(&app)[1].offset - (before - 0.01)).abs() < 1e-6);
    }

    #[test]
    fn on_a_freeform_gradient_it_edits_the_selected_point() {
        let mut app = app();
        app.run("paint.editGradient", json!({"kind": "freeform"})).unwrap();
        app.run("paint.freeform.addPoint", json!({"at": [150, 150], "color": "#0000ff"})).unwrap();
        let point = |app: &VectorcraftApp| match selected(app) {
            Some(Target::Point(p, i)) => (p, i),
            _ => panic!("no point selected"),
        };
        let (p, i) = point(&app);
        // Double-clicking the point opens the popover beside it.
        let view = app.view_info();
        crate::canvas::dispatch(&mut app, &PointerEvent { kind: PointerKind::DoubleClick, pos: p.at, mods: Default::default(), pressure: 1.0 }, view);
        let d = app.ui.dialog.clone().expect("the popover opened");
        assert_eq!((d.kind.as_str(), d.fields["index"].clone()), ("gradientStop", json!(i)));
        frame(&mut app, vec![]);
        app.ui.dialog.as_mut().unwrap().fields.insert("tab".into(), json!("swatches"));
        frame(&mut app, vec![]);
        assert!(app.ui.dialog.is_some(), "both tabs draw");
        // Agents set its colour, opacity and spread.
        for (k, v) in [("color", json!("#ff0000")), ("opacity", json!(40)), ("spread", json!(25))] {
            app.ui.dialog.as_mut().unwrap().fields.insert(k.into(), v);
        }
        assert_eq!(super::super::confirm(&mut app).unwrap(), json!({"index": i}));
        let (p, _) = point(&app);
        assert_eq!((p.color.to_hex(), p.opacity, p.spread), ("#ff0000".to_string(), 0.4, 0.25));
        assert!(app.ui.dialog.is_none());
    }
}
