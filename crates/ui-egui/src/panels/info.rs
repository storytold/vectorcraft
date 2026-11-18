//! Info panel: cursor X/Y, selection W/H, the pointer's distance/angle from the selection, the
//! readout of the tool in use (a move, a gradient vector, a Scale, Rotate, Reflect or Shear tool
//! drag, the Zoom tool's magnification, the Paintbrush's brush) and the selection's fill and
//! stroke colour values.

use egui::{Sense, Ui, vec2};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{NodeKind, Unit};
use vectorcraft_geom::Point;
use vectorcraft_tools::Readout;

use super::{current_paints, mixed_paints, pstate, set_pstate};
use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets::{self, menu_item};

/// Color values in the color's own model, for the Info panel.
pub fn color_values(c: &Color) -> Vec<(&'static str, String)> {
    match *c {
        Color::Rgb { r, g, b } => vec![("R", format!("{:.0}", r * 255.0)), ("G", format!("{:.0}", g * 255.0)), ("B", format!("{:.0}", b * 255.0))],
        Color::Cmyk { c, m, y, k } => {
            vec![
                ("C", format!("{:.0}%", c * 100.0)),
                ("M", format!("{:.0}%", m * 100.0)),
                ("Y", format!("{:.0}%", y * 100.0)),
                ("K", format!("{:.0}%", k * 100.0)),
            ]
        }
        Color::Gray { k } => vec![("K", format!("{:.0}%", k * 100.0))],
        Color::Lab { l, a, b } => vec![("L", format!("{l:.0}")), ("a", format!("{a:.0}")), ("b", format!("{b:.0}"))],
    }
}

/// The name Show Options adds for paint `p`: the swatch and the percentage of a tint, or the
/// gradient swatch a gradient is linked to.
fn paint_name(p: &Paint) -> Option<String> {
    match p {
        Paint::Solid { swatch: Some(n), tint, .. } if *tint < 1.0 => Some(format!("{n} {:.0}%", vectorcraft_color::tint_percent(*tint))),
        Paint::Gradient(g) => g.swatch.clone(),
        _ => None,
    }
}

/// One paint's row: its tile and values, and with `options` its name ([`paint_name`]). A paint the
/// selected objects do not share (`mixed`) leaves the row empty.
fn paint_block(ui: &mut Ui, p: &Paint, mixed: bool, options: bool) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
        if mixed {
            return;
        }
        widgets::swatch_tile(ui, r, p, false, false);
        match p {
            Paint::Solid { color, .. } => {
                for (k, v) in color_values(color) {
                    ui.label(egui::RichText::new(format!("{k}: {v}")).monospace().color(t.text));
                }
            }
            other => {
                let name = paint_name(other).filter(|_| options).unwrap_or_else(|| other.label());
                ui.label(egui::RichText::new(name).color(t.text_dim));
            }
        }
    });
    if let (false, true, Paint::Solid { .. }, Some(name)) = (mixed, options, p, paint_name(p)) {
        ui.label(egui::RichText::new(name).color(t.text_dim));
    }
}

/// What the panel reads from the document and the tool for its rows of numbers ([`Rows::of`]).
struct Shown<'a> {
    units: Unit,
    /// The rulers' zero point: X and Y count from it.
    origin: Point,
    /// The document point under the pointer.
    pointer: Point,
    /// The sides and the center of the selection's bounding box.
    size: Option<(f64, f64, Point)>,
    readout: Option<&'a Readout>,
    /// Show Options is on.
    options: bool,
}

/// The panel's rows of numbers: a left and a right column, row by row.
#[derive(Debug, Default, PartialEq)]
struct Rows {
    left: Vec<String>,
    right: Vec<String>,
}

impl Rows {
    /// X and Y are the pointer's position, and during a Rotate, Reflect or Shear tool drag the
    /// center of the selection's bounding box. W and H are the selection's size, the change in x
    /// and y while a drag moves the selection or draws a gradient vector (with the distance and the
    /// angle under them), and the percentages of a Scale tool drag. The transform tools' angles
    /// take their place during their drags.
    fn of(s: &Shown) -> Self {
        let len = |v: f64| s.units.format(v);
        let at = match s.readout {
            Some(Readout::Rotate { center, .. } | Readout::Reflect { center, .. } | Readout::Shear { center, .. }) => *center,
            _ => s.pointer,
        };
        let mut rows = Rows { left: vec![format!("X: {}", len(at.x - s.origin.x)), format!("Y: {}", len(at.y - s.origin.y))], right: vec![] };
        match s.readout {
            Some(r @ Readout::Drag { dx, dy }) => {
                rows.right = vec![format!("W: {}", len(*dx)), format!("H: {}", len(*dy))];
                let (d, a) = r.distance_angle().unwrap_or_default();
                rows.left.push(format!("D: {}", len(d)));
                rows.right.push(format!("A: {a:.1}°"));
            }
            Some(Readout::Scale { sx, sy }) => rows.right = vec![format!("W: {:.1}%", sx * 100.0), format!("H: {:.1}%", sy * 100.0)],
            Some(Readout::Rotate { angle, .. } | Readout::Reflect { angle, .. }) => rows.right = vec![format!("A: {angle:.1}°")],
            Some(Readout::Shear { axis, angle, .. }) => rows.right = vec![format!("A: {axis:.1}°"), format!("{}: {angle:.1}°", tl!("Shear"))],
            Some(Readout::Brush { .. }) | None => {
                let (w, h) = s.size.map(|(w, h, _)| (len(w), len(h))).unwrap_or_default();
                rows.right = vec![format!("W: {w}"), format!("H: {h}")];
                if let (true, Some((.., c))) = (s.options, s.size) {
                    let (d, a) = Readout::drag(s.pointer - c).distance_angle().unwrap_or_default();
                    rows.left.push(format!("D: {}", len(d)));
                    rows.right.push(format!("A: {a:.1}°"));
                }
            }
        }
        rows
    }
}

/// The font and size the selected objects share when every one of them is type: the first
/// object's first style, as the Character panel shows its size. None when an object is not type or
/// two of them differ.
fn shared_type(app: &VectorcraftApp) -> Option<(String, f64, f64)> {
    let st = app.session.active()?;
    let mut shared = None;
    for id in &st.selection.objects {
        let NodeKind::Text(tx) = &st.doc.node(*id)?.kind else { return None };
        let raw = tx.first_style();
        // The size in document points, as the Character panel shows it (#1034); a collapsed
        // transform has none, so the stored size stands.
        let cs = tx.effective_char_style(&raw).unwrap_or(raw);
        let this = (format!("{} {}", cs.font_family, cs.font_style), cs.size, cs.tracking);
        match &shared {
            None => shared = Some(this),
            Some(first) if *first == this => {}
            Some(_) => return None,
        }
    }
    shared
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let options = !pstate::<bool>(ui.ctx(), "info-hide-options");
    // W and H are the sides of the bounding box the Transform panel shows, measured as Use Preview
    // Bounds says.
    let size = app.selection_box().map(|b| (b.rect.width(), b.rect.height(), b.center()));
    let readout = app.session.tool_readout();
    let rows = Rows::of(&Shown {
        units: app.session.general_unit(),
        // X and Y count from the rulers' zero point, as the rulers and the Transform panel do.
        origin: app.ruler_origin(),
        pointer: app.hover_doc.unwrap_or_default(),
        size,
        readout: readout.as_ref(),
        options,
    });
    let mono = |ui: &mut Ui, s: String| {
        ui.label(egui::RichText::new(s).monospace().color(t.text));
    };
    ui.columns(2, |cols| {
        for s in rows.left {
            mono(&mut cols[0], s);
        }
        for s in rows.right {
            mono(&mut cols[1], s);
        }
    });
    if app.session.tool_id() == "zoom" {
        let zoom = app.view().map_or(1.0, |v| v.zoom);
        mono(ui, format!("{}: {}", tl!("Zoom"), crate::state::zoom_label(zoom, app.session.prefs.display_print_size)));
    }
    if let Some(Readout::Brush { name: Some(name) }) = readout {
        mono(ui, name);
    }
    widgets::divider(ui);
    let (f, s) = current_paints(app);
    let (fill_mixed, stroke_mixed) = mixed_paints(app, ui.ctx());
    paint_block(ui, &f, fill_mixed, options);
    paint_block(ui, &s, stroke_mixed, options);
    if let Some((font, size, tracking)) = shared_type(app) {
        widgets::divider(ui);
        mono(ui, font);
        mono(
            ui,
            crate::i18n::fmt(
                tl!("Size: {size}  Tracking: {tracking}"),
                &[("size", &app.session.type_unit().format(size)), ("tracking", &format!("{tracking:.0}"))],
            ),
        );
    }
}

pub fn menu(_app: &mut VectorcraftApp, ui: &mut Ui) {
    let hidden: bool = pstate(ui.ctx(), "info-hide-options");
    if menu_item(ui, if hidden { tl!("Show Options") } else { tl!("Hide Options") }, true, false) {
        set_pstate(ui.ctx(), "info-hide-options", !hidden);
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests_appearance::{app_with_rect, frame_events, run};
    use super::*;
    use serde_json::json;
    use vectorcraft_engine::ViewInfo;
    use vectorcraft_tools::{PointerEvent, PointerKind};

    fn texts(app: &mut VectorcraftApp) -> Vec<String> {
        frame_events(&egui::Context::default(), app, vec![], show).into_iter().map(|(t, _)| t).collect()
    }

    fn view() -> ViewInfo {
        ViewInfo { smart_guides: false, snap_to_point: false, ..Default::default() }
    }

    fn pointer(app: &mut VectorcraftApp, kind: PointerKind, x: f64, y: f64) {
        app.session.pointer(&PointerEvent::new(kind, x, y), view()).unwrap();
    }

    fn shown(readout: Option<&Readout>, options: bool) -> Rows {
        let size = Some((100.0, 60.0, Point::new(100.0, 80.0)));
        Rows::of(&Shown { units: Unit::Points, origin: Point::new(10.0, 20.0), pointer: Point::new(130.0, 40.0), size, readout, options })
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn values_follow_model() {
        let v = color_values(&Color::rgb8(230, 120, 40));
        assert_eq!(v[0], ("R", "230".to_string()));
        assert_eq!(color_values(&Color::cmyk(0.1, 0.0, 0.0, 0.5))[3].1, "50%");
        assert_eq!(color_values(&Color::gray(0.25)).len(), 1);
    }

    /// #1882: scaled type shows its size in document points, as the Character panel does (#1034).
    #[test]
    fn scaled_type_shows_its_scaled_size() {
        let mut app = app_with_rect();
        let id = run(&mut app, "text.create", json!({"x": 10, "y": 150, "text": "Hi", "size": 12}))["id"].clone();
        run(&mut app, "select.set", json!({ "ids": [id] }));
        run(&mut app, "object.scale", json!({"sx": 200}));
        let texts = frame_events(&egui::Context::default(), &mut app, vec![], show);
        let size = texts.iter().find(|(t, _)| t.starts_with("Size:")).map(|(t, _)| t.as_str());
        assert_eq!(size, Some("Size: 24 pt  Tracking: 0"), "{texts:?}");
    }

    #[test]
    fn the_rows_follow_the_tool_in_use() {
        // No tool readout: the pointer from the zero point, the selection's size, and with Show
        // Options the pointer's distance and angle from the selection's center.
        let idle = shown(None, true);
        assert_eq!(idle.left, strings(&["X: 120 pt", "Y: 20 pt", "D: 50 pt"]));
        assert_eq!(idle.right, strings(&["W: 100 pt", "H: 60 pt", "A: 53.1°"]));
        assert_eq!(shown(None, false), Rows { left: strings(&["X: 120 pt", "Y: 20 pt"]), right: strings(&["W: 100 pt", "H: 60 pt"]) });
        // The Paintbrush leaves the numbers as they are.
        assert_eq!(shown(Some(&Readout::Brush { name: None }), true), idle);
        // A drag: the change in x and y, the distance and the angle, with Show Options off too.
        let drag = shown(Some(&Readout::Drag { dx: -30.0, dy: 40.0 }), false);
        assert_eq!(drag.left, strings(&["X: 120 pt", "Y: 20 pt", "D: 50 pt"]));
        assert_eq!(drag.right, strings(&["W: -30 pt", "H: 40 pt", "A: -126.9°"]));
        let scale = shown(Some(&Readout::Scale { sx: 1.5, sy: -0.25 }), true);
        assert_eq!(scale.left, strings(&["X: 120 pt", "Y: 20 pt"]));
        assert_eq!(scale.right, strings(&["W: 150.0%", "H: -25.0%"]));
        // Rotate, Reflect and Shear: the center of the selection's box and the angles.
        let center = Point::new(60.0, 70.0);
        let rotate = shown(Some(&Readout::Rotate { center, angle: 30.0 }), true);
        assert_eq!(rotate, Rows { left: strings(&["X: 50 pt", "Y: 50 pt"]), right: strings(&["A: 30.0°"]) });
        assert_eq!(shown(Some(&Readout::Reflect { center, angle: 45.0 }), true).right, strings(&["A: 45.0°"]));
        let shear = shown(Some(&Readout::Shear { center, axis: 90.0, angle: -12.5 }), true);
        assert_eq!(shear, Rows { left: strings(&["X: 50 pt", "Y: 50 pt"]), right: strings(&["A: 90.0°", "Shear: -12.5°"]) });
    }

    #[test]
    fn w_and_h_follow_use_preview_bounds() {
        let mut app = app_with_rect();
        run(&mut app, "stroke.set", json!({"weight": 10}));
        let has = |app: &mut VectorcraftApp, s: &str| texts(app).iter().any(|t| t == s);
        assert!(has(&mut app, "W: 100 pt") && has(&mut app, "H: 100 pt"), "{:?}", texts(&mut app));
        run(&mut app, "prefs.set", json!({"key": "usePreviewBounds", "value": true}));
        assert!(has(&mut app, "W: 110 pt") && has(&mut app, "H: 110 pt"), "{:?}", texts(&mut app));
    }

    #[test]
    fn a_move_shows_the_change_until_the_release() {
        let mut app = app_with_rect();
        app.session.select_tool("selection", view()).unwrap();
        pointer(&mut app, PointerKind::Down, 60.0, 60.0);
        pointer(&mut app, PointerKind::Drag, 90.0, 20.0);
        let during = texts(&mut app);
        for want in ["W: 30 pt", "H: -40 pt", "D: 50 pt", "A: 53.1°"] {
            assert!(during.iter().any(|t| t == want), "{want}: {during:?}");
        }
        pointer(&mut app, PointerKind::Up, 90.0, 20.0);
        let after = texts(&mut app);
        assert!(after.iter().any(|t| t == "W: 100 pt") && !after.iter().any(|t| t == "W: 30 pt"), "{after:?}");
    }

    #[test]
    fn the_zoom_tool_shows_the_magnification() {
        let mut app = app_with_rect();
        assert!(!texts(&mut app).iter().any(|t| t.starts_with("Zoom:")));
        app.session.select_tool("zoom", view()).unwrap();
        let zoom = app.view().map_or(1.0, |v| v.zoom);
        let want = format!("Zoom: {}", crate::state::zoom_label(zoom, app.session.prefs.display_print_size));
        assert!(texts(&mut app).contains(&want), "{want}: {:?}", texts(&mut app));
    }

    #[test]
    fn the_paintbrush_shows_its_brush() {
        let mut app = app_with_rect();
        app.session.select_tool("paintbrush", view()).unwrap();
        let brush = run(&mut app, "brush.list", json!({}))["brushes"][0]["name"].as_str().map(str::to_string);
        let Some(brush) = brush else { panic!("the document has no brush") };
        assert!(!texts(&mut app).contains(&brush));
        app.session.set_tool_option("brush", &json!(brush));
        assert!(texts(&mut app).contains(&brush), "{brush}: {:?}", texts(&mut app));
    }

    #[test]
    fn several_objects_show_only_the_paint_they_share() {
        let mut app = app_with_rect();
        run(&mut app, "paint.setFill", json!({"color": "#ff0000"}));
        let other = run(&mut app, "shape.rectangle", json!({"x": 120, "y": 10, "width": 40, "height": 40}))["id"].clone();
        run(&mut app, "paint.setFill", json!({"color": "#ff0000"}));
        run(&mut app, "select.all", json!({}));
        assert!(texts(&mut app).iter().any(|t| t == "R: 255"), "{:?}", texts(&mut app));
        run(&mut app, "select.set", json!({ "ids": [other] }));
        run(&mut app, "paint.setFill", json!({"color": "#00ff00"}));
        assert!(texts(&mut app).iter().any(|t| t == "G: 255"));
        run(&mut app, "select.all", json!({}));
        let all = texts(&mut app);
        assert!(!all.iter().any(|t| t == "G: 255" || t == "R: 255"), "{all:?}");
    }

    #[test]
    fn show_options_names_a_tint_and_a_gradient_swatch() {
        let tint = Paint::Solid { color: Color::rgb8(255, 128, 128), swatch: Some("Brand".into()), tint: 0.4 };
        assert_eq!(paint_name(&tint), Some("Brand 40%".into()));
        let full = Paint::Solid { color: Color::rgb8(255, 0, 0), swatch: Some("Brand".into()), tint: 1.0 };
        assert_eq!(paint_name(&full), None);
        assert_eq!(paint_name(&Paint::None), None);
        let mut g = vectorcraft_color::GradientPaint::new(vectorcraft_color::Gradient::default());
        assert_eq!(paint_name(&Paint::Gradient(Box::new(g.clone()))), None);
        g.swatch = Some("Sunset".into());
        assert_eq!(paint_name(&Paint::Gradient(Box::new(g))), Some("Sunset".into()));
    }
}
