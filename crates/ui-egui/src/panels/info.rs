//! Info panel: cursor X/Y, selection W/H and area, the pointer's distance/angle from the selection,
//! and the selection's fill and stroke colour values.

use egui::{Sense, Ui, vec2};
use vectorcraft_color::{Color, Paint};

use super::{current_paints, first_selected, pstate, set_pstate};
use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets::{self, menu_item};
use vectorcraft_doc::{NodeId, Unit};
use vectorcraft_engine::cmd::area::{self, Area, SQUARE_METRES_PER_SQUARE_POINT};

/// An area in square points as people buy print material: square metres (square centimetres
/// below 0.01 m²), or square feet (square inches below one) when the document measures in inches,
/// feet or yards.
pub fn format_area(pt2: f64, unit: Unit) -> String {
    let m2 = pt2 * SQUARE_METRES_PER_SQUARE_POINT;
    if matches!(unit, Unit::Inches | Unit::FeetInches | Unit::Feet | Unit::Yards) {
        let in2 = pt2 / (72.0 * 72.0);
        return if in2 >= 144.0 { format!("{:.3} ft²", in2 / 144.0) } else { format!("{in2:.2} in²") };
    }
    if m2 >= 0.01 { format!("{m2:.3} m²") } else { format!("{:.2} cm²", m2 * 10_000.0) }
}

/// The selection's area, measured again only when the document or the selection changes.
fn selection_area(app: &VectorcraftApp, ctx: &egui::Context) -> Option<Area> {
    type Cached = Option<(usize, Vec<NodeId>, Option<Area>)>;
    let st = app.session.active()?;
    if st.selection.objects.is_empty() {
        return None;
    }
    let doc = std::sync::Arc::as_ptr(&st.doc) as usize;
    if let Some((d, ids, a)) = pstate::<Cached>(ctx, "info-area")
        && d == doc
        && ids == st.selection.objects
    {
        return a;
    }
    let a = area::measure(&st.doc, &st.selection.objects);
    set_pstate::<Cached>(ctx, "info-area", Some((doc, st.selection.objects.clone(), a)));
    a
}

/// Colour values in the colour's own model, like Illustrator's Info panel.
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

fn paint_block(ui: &mut Ui, p: &Paint) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
        widgets::swatch_tile(ui, r, p, false, false);
        match p {
            Paint::Solid { color, .. } => {
                for (k, v) in color_values(color) {
                    ui.label(egui::RichText::new(format!("{k}: {v}")).monospace().color(t.text));
                }
            }
            other => {
                ui.label(egui::RichText::new(other.label()).color(t.text_dim));
            }
        }
    });
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let units = app.session.general_unit();
    let p = app.hover_doc.unwrap_or_default();
    // X and Y count from the rulers' zero point, as the rulers and the Transform panel do.
    let origin = app.ruler_origin();
    let bounds = app.session.active().and_then(|st| st.doc.bounds_of(&st.selection.objects, false));
    let mono = |ui: &mut Ui, s: String| {
        ui.label(egui::RichText::new(s).monospace().color(t.text));
    };
    ui.columns(2, |cols| {
        mono(&mut cols[0], format!("X: {}", units.format(p.x - origin.x)));
        mono(&mut cols[0], format!("Y: {}", units.format(p.y - origin.y)));
        let (w, h) = bounds.map(|b| (units.format(b.width()), units.format(b.height()))).unwrap_or_default();
        mono(&mut cols[1], format!("W: {w}"));
        mono(&mut cols[1], format!("H: {h}"));
    });
    if let Some(a) = selection_area(app, ui.ctx()) {
        let row = |ui: &mut Ui, label: &str, value: f64, hint: &str| {
            ui.label(egui::RichText::new(crate::i18n::fmt(label, &[("area", &format_area(value, units))])).monospace().color(t.text))
                .on_hover_text(hint);
        };
        row(ui, tl!("Area: {area}"), a.bounds, tl!("Width × height of the selection: the sheet it needs"));
        if let Some(shape) = a.shape {
            row(ui, tl!("Shape area: {area}"), shape, tl!("Area the closed shapes enclose: holes left out, overlaps counted once"));
        }
    }
    if !pstate::<bool>(ui.ctx(), "info-hide-options")
        && let Some(b) = bounds
    {
        let c = b.center();
        let d = ((p.x - c.x).powi(2) + (p.y - c.y).powi(2)).sqrt();
        let a = (-(p.y - c.y)).atan2(p.x - c.x).to_degrees();
        ui.columns(2, |cols| {
            mono(&mut cols[0], format!("D: {}", units.format(d)));
            mono(&mut cols[1], format!("A: {a:.1}°"));
        });
    }
    widgets::divider(ui);
    let (f, s) = current_paints(app);
    paint_block(ui, &f);
    paint_block(ui, &s);
    if let Some(n) = first_selected(app)
        && let vectorcraft_doc::NodeKind::Text(tx) = &n.kind
    {
        widgets::divider(ui);
        let raw = tx.first_style();
        // The size in document points, as the Character panel shows it (#1034); a collapsed
        // transform has none, so the stored size stands.
        let st = tx.effective_char_style(&raw).unwrap_or(raw);
        mono(ui, format!("{} {}", st.font_family, st.font_style));
        mono(
            ui,
            crate::i18n::fmt(
                tl!("Size: {size}  Tracking: {tracking}"),
                &[("size", &app.session.type_unit().format(st.size)), ("tracking", &format!("{:.0}", st.tracking))],
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
    use super::*;

    #[test]
    fn areas_read_in_square_metres_or_feet() {
        let m = 72.0 / 0.0254;
        assert_eq!(format_area(2.0 * m * m, Unit::Millimeters), "2.000 m²");
        assert_eq!(format_area(m * m / 10_000.0, Unit::Centimeters), "1.00 cm²");
        assert_eq!(format_area(72.0 * 72.0 * 144.0 * 3.0, Unit::Inches), "3.000 ft²");
        assert_eq!(format_area(72.0 * 72.0 * 6.0, Unit::Inches), "6.00 in²");
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
        use super::super::tests_appearance::{app_with_rect, frame_events, run};
        use serde_json::json;
        let mut app = app_with_rect();
        let id = run(&mut app, "text.create", json!({"x": 10, "y": 150, "text": "Hi", "size": 12}))["id"].clone();
        run(&mut app, "select.set", json!({ "ids": [id] }));
        run(&mut app, "object.scale", json!({"sx": 200}));
        let texts = frame_events(&egui::Context::default(), &mut app, vec![], show);
        let size = texts.iter().find(|(t, _)| t.starts_with("Size:")).map(|(t, _)| t.as_str());
        assert_eq!(size, Some("Size: 24 pt  Tracking: 0"), "{texts:?}");
    }
}
