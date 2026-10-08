//! Transform panel: reference point, X/Y/W/H with the constrain link, rotate and shear, flips,
//! live-shape properties and the Scale Corners / Scale Strokes & Effects options.

use egui::Ui;
use serde_json::json;
use vectorcraft_doc::NodeKind;

use super::{first_selected, pstate, set_pstate};
use crate::theme::Tokens;
use crate::widgets::{self, menu_item};
use crate::{VectorcraftApp, icons};

pub const ANGLE_PRESETS: [f64; 9] = [-180.0, -135.0, -90.0, -45.0, 0.0, 45.0, 90.0, 135.0, 180.0];

/// Proportional size: the other dimension when one changes with the link on.
pub fn constrained(w: f64, h: f64, new_w: Option<f64>, new_h: Option<f64>) -> (f64, f64) {
    match (new_w, new_h) {
        (Some(nw), _) if w.abs() > 1e-9 => (nw, h * nw / w),
        (_, Some(nh)) if h.abs() > 1e-9 => (w * nh / h, nh),
        (a, b) => (a.unwrap_or(w), b.unwrap_or(h)),
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    if app.session.active().is_none() {
        widgets::dim_label(ui, tl!("No document"));
        return;
    }
    let units = app.session.general_unit();
    // The bounding box, rotated with rotated objects: W/H are its own sides, X/Y its reference
    // point on the page.
    let bx = app.selection_box();
    let bounds = bx.map(|b| b.rect);
    let refi: usize = ui.data(|d| d.get_temp(egui::Id::new("refpt"))).unwrap_or(4);
    let link = app.session.prefs.constrain_proportions;
    let has = bounds.is_some();
    let rp = bx.map(|b| b.reference_point(refi));
    ui.horizontal(|ui| {
        if let Some(i) = widgets::reference_point(ui, refi) {
            ui.data_mut(|d| d.insert_temp(egui::Id::new("refpt"), i));
        }
        ui.add_space(4.0);
        ui.add_enabled_ui(has, |ui| {
            egui::Grid::new("xfp-grid").num_columns(4).spacing([4.0, 6.0]).min_col_width(0.0).show(ui, |ui| {
                widgets::dim_label(ui, "X:");
                if let Some(v) = widgets::num_field(ui, "xfp-x", rp.map(|p| p.x), units, 80.0) {
                    app.run("object.setBounds", json!({"x": v, "reference": refi})).ok();
                }
                widgets::dim_label(ui, "W:");
                if let Some(v) = widgets::num_field(ui, "xfp-w", bounds.map(|b| b.width()), units, 80.0) {
                    app.run("object.setBounds", json!({"width": v, "reference": refi, "proportional": link})).ok();
                }
                ui.end_row();
                widgets::dim_label(ui, "Y:");
                if let Some(v) = widgets::num_field(ui, "xfp-y", rp.map(|p| p.y), units, 80.0) {
                    app.run("object.setBounds", json!({"y": v, "reference": refi})).ok();
                }
                widgets::dim_label(ui, "H:");
                if let Some(v) = widgets::num_field(ui, "xfp-h", bounds.map(|b| b.height()), units, 80.0) {
                    app.run("object.setBounds", json!({"height": v, "reference": refi, "proportional": link})).ok();
                }
                ui.end_row();
            });
        });
        constrain_link(app, ui);
    });
    ui.add_space(4.0);
    let origin = rp.map(|p| json!([p.x, p.y]));
    ui.horizontal(|ui| {
        ui.add_enabled_ui(has, |ui| {
            icons::icon(ui, "rotate-ccw", 16.0, t.icon).on_hover_text(tl!("Rotate"));
            // The bounding box's angle: a new value turns the selection to it.
            let angle = bx.map_or(0.0, |b| b.angle);
            if let Some(a) = widgets::spin_plain(ui, "xfp-rot", angle, "°", 2, 96.0, 15.0, -360.0, &ANGLE_PRESETS)
                && a != angle
            {
                app.run("object.rotate", json!({"angle": a, "absolute": true, "origin": origin})).ok();
            }
            ui.add_space(4.0);
            icons::icon(ui, "dc-shear", 16.0, t.icon).on_hover_text(tl!("Shear"));
            if let Some(a) = widgets::plain_field(ui, "xfp-shear", 0.0, "°", 1, 56.0)
                && a != 0.0
            {
                app.run("object.shear", json!({"angle": a, "axis": "horizontal", "origin": origin})).ok();
            }
        });
    });
    ui.horizontal(|ui| {
        if widgets::icon_button_enabled(ui, "flip-horizontal-2", tl!("Flip Horizontal"), false, has, 24.0).clicked() {
            app.run("object.reflect", json!({"axis": "vertical", "origin": origin})).ok();
        }
        if widgets::icon_button_enabled(ui, "flip-vertical-2", tl!("Flip Vertical"), false, has, 24.0).clicked() {
            app.run("object.reflect", json!({"axis": "horizontal", "origin": origin})).ok();
        }
    });
    // Live shape properties.
    if let Some(n) = first_selected(app)
        && let NodeKind::Path { live: Some(live), .. } = &n.kind
    {
        widgets::divider(ui);
        match live {
            vectorcraft_doc::LiveShape::Rectangle { radii, .. } => {
                widgets::subheader(ui, tl!("Rectangle Properties:"));
                widgets::dim_label(ui, tl!("Corner Radius:"));
                corner_fields(app, ui, "xfp-radius", *radii, units);
            }
            vectorcraft_doc::LiveShape::Polygon { sides, .. } => {
                widgets::subheader(ui, tl!("Polygon Properties:"));
                ui.horizontal(|ui| {
                    widgets::dim_label(ui, tl!("Sides:"));
                    if let Some(s) = widgets::plain_field(ui, "xfp-sides", *sides as f64, "", 0, 60.0) {
                        app.run("object.setLiveShape", json!({"sides": s.clamp(3.0, 20.0) as u32})).ok();
                    }
                });
            }
            _ => {
                widgets::subheader(ui, tl!("Shape Properties:"));
                widgets::dim_label(ui, tl!(n.kind_label()));
            }
        }
    }
    if pstate::<bool>(ui.ctx(), "xf-hide-options") {
        return;
    }
    widgets::divider(ui);
    let (sc, ss) = (app.session.prefs.scale_corners, app.session.prefs.scale_strokes);
    if widgets::check(ui, tl!("Scale Corners"), sc, true) {
        set_pref(app, "scaleCorners", !sc);
    }
    if widgets::check(ui, tl!("Scale Strokes & Effects"), ss, true) {
        set_pref(app, "scaleStrokes", !ss);
    }
}

/// A live rectangle's four corner radii (top-left, top-right / bottom-left, bottom-right, as they
/// sit on the shape) and the link that makes one value set all four (on by default).
pub fn corner_fields(app: &mut VectorcraftApp, ui: &mut Ui, salt: &str, radii: [f64; 4], units: vectorcraft_doc::Unit) {
    let unlinked_key = format!("{salt}-unlinked");
    let linked = !pstate::<bool>(ui.ctx(), &unlinked_key);
    ui.horizontal(|ui| {
        egui::Grid::new((salt, "grid")).num_columns(2).spacing([4.0, 4.0]).show(ui, |ui| {
            for row in [[0, 1], [3, 2]] {
                for k in row {
                    if let Some(r) = widgets::num_field(ui, (salt, k), Some(radii[k]), units, 80.0) {
                        let mut next = radii;
                        if linked {
                            next = [r; 4];
                        } else {
                            next[k] = r;
                        }
                        app.run("object.setLiveShape", json!({ "radii": next })).ok();
                    }
                }
                ui.end_row();
            }
        });
        if widgets::icon_button(ui, if linked { "link" } else { "link-2-off" }, tl!("Link Radius Values"), linked, 22.0).clicked() {
            set_pstate(ui.ctx(), &unlinked_key, linked);
        }
    });
}

/// The link between W and H (Transform panel, Properties panel, Control bar): one toggle, the
/// `constrainProportions` preference, which the size fields pass on as `proportional`.
pub fn constrain_link(app: &mut VectorcraftApp, ui: &mut Ui) {
    let on = app.session.prefs.constrain_proportions;
    if widgets::icon_button(ui, if on { "link" } else { "link-2-off" }, tl!("Constrain Width and Height Proportions"), on, 22.0).clicked() {
        set_pref(app, "constrainProportions", !on);
    }
}

/// Toggle a boolean preference (through `prefs.set`, as agents do).
pub fn set_pref(app: &mut VectorcraftApp, key: &str, on: bool) {
    if let Err(e) = app.run("prefs.set", json!({ "key": key, "value": on })) {
        app.status(e);
    }
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let hidden: bool = pstate(ui.ctx(), "xf-hide-options");
    let has = super::selection_len(app) > 0;
    if menu_item(ui, if hidden { tl!("Show Options") } else { tl!("Hide Options") }, true, false) {
        set_pstate(ui.ctx(), "xf-hide-options", !hidden);
    }
    ui.separator();
    if menu_item(ui, tl!("Flip Horizontal"), has, false) {
        app.run("object.reflect", json!({"axis": "vertical"})).ok();
    }
    if menu_item(ui, tl!("Flip Vertical"), has, false) {
        app.run("object.reflect", json!({"axis": "horizontal"})).ok();
    }
    ui.separator();
    let ss = app.session.prefs.scale_strokes;
    if menu_item(ui, tl!("Scale Strokes & Effects"), true, ss) {
        set_pref(app, "scaleStrokes", !ss);
    }
    ui.separator();
    menu_item(ui, tl!("Transform Object Only"), false, true);
    menu_item(ui, tl!("Transform Pattern Only"), false, false);
    menu_item(ui, tl!("Transform Both"), false, false);
    ui.separator();
    menu_item(ui, tl!("Use Registration Point for Symbol"), false, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One headless frame of the panel (or the Align panel's menu): the texts drawn.
    fn texts(app: &mut VectorcraftApp, draw: fn(&mut VectorcraftApp, &mut Ui)) -> Vec<String> {
        fn walk(s: &egui::Shape, out: &mut Vec<String>) {
            match s {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| draw(app, ui));
        out.textures_delta.clear();
        let mut v = vec![];
        out.shapes.iter().for_each(|c| walk(&c.shape, &mut v));
        v
    }

    #[test]
    fn use_preview_bounds_shows_the_visual_size_and_checks_the_align_flyout() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 50})).unwrap();
        app.run("stroke.set", json!({"weight": 10})).unwrap();
        assert!(texts(&mut app, show).iter().any(|t| t == "100 pt"));
        assert!(texts(&mut app, crate::panels::align::menu).iter().any(|t| t == "   Use Preview Bounds"));
        set_pref(&mut app, "usePreviewBounds", true);
        let shown = texts(&mut app, show);
        assert!(shown.iter().any(|t| t == "110 pt") && shown.iter().any(|t| t == "60 pt"), "{shown:?}");
        assert!(texts(&mut app, crate::panels::align::menu).iter().any(|t| t == "✓ Use Preview Bounds"));
    }

    #[test]
    fn rotation_field_shows_the_angle_the_box_keeps() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 50})).unwrap();
        app.run("object.rotate", json!({"angle": 45, "absolute": true})).unwrap();
        let shown = texts(&mut app, show);
        // The angle stays, and W/H are the rectangle's own sides.
        assert!(shown.iter().any(|t| t == "45°"), "{shown:?}");
        assert!(shown.iter().any(|t| t == "100 pt") && shown.iter().any(|t| t == "50 pt"), "{shown:?}");
        app.run("object.resetBoundingBox", json!({})).unwrap();
        let shown = texts(&mut app, show);
        assert!(shown.iter().any(|t| t == "0°") && !shown.iter().any(|t| t == "100 pt"), "{shown:?}");
    }

    #[test]
    fn constrain_proportions() {
        assert_eq!(constrained(100.0, 50.0, Some(200.0), None), (200.0, 100.0));
        assert_eq!(constrained(100.0, 50.0, None, Some(25.0)), (50.0, 25.0));
        assert_eq!(constrained(0.0, 50.0, Some(10.0), None), (10.0, 50.0));
    }
}
