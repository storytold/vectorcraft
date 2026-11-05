//! The 3D Box and customizable shapes add-on in the interface: its submenu (Object › Plug-ins ›
//! 3D Box), which selects the 3D Box tool, and the context-sensitive Control bar options shown
//! when a shape tool (`rectangle`, `roundedRectangle`, `ellipse`, `polygon`, `star`, `spiral`,
//! `box3d`) is active.

use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_geom::shapes::EllipseMode;
use vectorcraft_tools::addons::box3d::{self, CustomShape};

use crate::menus::Item;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

pub fn menu() -> Vec<Item> {
    vec![Item::Cmd("3D Box Tool", "tool.select", json!({"tool": box3d::ID}))]
}

/// Draw context-sensitive shape customization controls in the top Control bar when a shape tool is
/// active. Returns `true` when controls were drawn.
pub fn control_bar(app: &mut VectorcraftApp, ui: &mut Ui) -> bool {
    let tool = app.session.tool_id().to_string();
    if !matches!(tool.as_str(), "rectangle" | "roundedRectangle" | "ellipse" | "polygon" | "star" | "spiral" | "box3d") {
        return false;
    }
    let opts = app.session.tool_options();
    let active_shape = if tool == "box3d" { opts["shape"].as_str().unwrap_or("box3d").to_string() } else { tool.clone() };
    let selected_custom: Option<CustomShape> = app.session.active().and_then(|st| {
        st.selection.objects.iter().find_map(|&id| st.doc.node(id).and_then(|n| CustomShape::read_for_update(n, active_shape == "spiral")))
    });

    if tool == "box3d" {
        for (s_id, lbl, icon) in [
            ("box3d", "3D Box", "box"),
            ("rectangle", "Rectangle", "square"),
            ("ellipse", "Ellipse", "dc-ellipse"),
            ("polygon", "Polygon", "dc-polygon"),
            ("star", "Star", "star"),
            ("spiral", "Spiral", "tornado"),
        ] {
            if widgets::icon_button(ui, icon, tl!(lbl), active_shape == s_id, 22.0).clicked() {
                app.run("tool.setOption", json!({ "key": "shape", "value": s_id })).ok();
            }
        }
        ui.separator();
    }

    match active_shape.as_str() {
        "ellipse" => draw_ellipse_controls(app, ui, &opts, selected_custom),
        "rectangle" | "roundedRectangle" => draw_rectangle_controls(app, ui, &opts, selected_custom),
        "polygon" | "star" => draw_polygon_controls(app, ui, &opts, &active_shape, selected_custom),
        "spiral" => draw_spiral_controls(app, ui, &opts, selected_custom),
        "box3d" => draw_box3d_controls(app, ui, &opts, selected_custom),
        _ => return false,
    }
    true
}

fn apply_shape(app: &mut VectorcraftApp, tool_kvs: &[(&str, Value)], live_patch: Value) {
    for (k, v) in tool_kvs {
        app.run("tool.setOption", json!({ "key": *k, "value": v.clone() })).ok();
    }
    if app.session.active().is_some_and(|st| !st.selection.is_empty()) {
        app.run("addon.shapes.configure", live_patch).ok();
    }
}

fn draw_arc_controls(app: &mut VectorcraftApp, ui: &mut Ui, start_deg: f64, end_deg: f64, mode: &str) {
    let t = Tokens::get(ui.ctx());
    widgets::field_label(ui, egui::RichText::new(tl!("Start:")).size(12.0).color(t.text_dim));
    if let Some(v) = widgets::plain_field(ui, ("cb-shape", "arc-start"), start_deg, "°", 1, 52.0) {
        let next_mode = if mode == "whole" { "pie" } else { mode };
        apply_shape(app, &[("arcStart", json!(v)), ("arcMode", json!(next_mode))], json!({ "startAngle": v, "arcMode": next_mode }));
    }
    widgets::field_label(ui, egui::RichText::new(tl!("End:")).size(12.0).color(t.text_dim));
    if let Some(v) = widgets::plain_field(ui, ("cb-shape", "arc-end"), end_deg, "°", 1, 52.0) {
        let next_mode = if mode == "whole" { "pie" } else { mode };
        apply_shape(app, &[("arcEnd", json!(v)), ("arcMode", json!(next_mode))], json!({ "endAngle": v, "arcMode": next_mode }));
    }
    for (m_id, tip_label, icon) in
        [("pie", "Slice", "circle-dot"), ("chord", "Closed", "circle-off"), ("arc", "Arc", "dc-arc"), ("whole", "Open", "dc-ellipse")]
    {
        if widgets::icon_button(ui, icon, tl!(tip_label), mode == m_id, 22.0).clicked() {
            let (s0, e0) = if m_id == "whole" {
                (0.0, 0.0)
            } else if (end_deg - start_deg).abs() < 1e-4 || ((end_deg - start_deg).abs() - 360.0).abs() < 1e-4 {
                (0.0, 270.0)
            } else {
                (start_deg, end_deg)
            };
            apply_shape(
                app,
                &[("arcMode", json!(m_id)), ("arcStart", json!(s0)), ("arcEnd", json!(e0))],
                json!({ "arcMode": m_id, "startAngle": s0, "endAngle": if m_id == "whole" { 360.0 } else { e0 } }),
            );
        }
    }
}

fn draw_ellipse_controls(app: &mut VectorcraftApp, ui: &mut Ui, opts: &Value, selected_custom: Option<CustomShape>) {
    let (start_deg, end_deg, mode) = match selected_custom {
        Some(CustomShape::Ellipse { pie: (a0, a1), arc_mode, .. }) => (a0, a1, mode_str(arc_mode)),
        _ => (opts["arcStart"].as_f64().unwrap_or(0.0), opts["arcEnd"].as_f64().unwrap_or(0.0), opts["arcMode"].as_str().unwrap_or("whole")),
    };
    draw_arc_controls(app, ui, start_deg, end_deg, mode);
    if widgets::flat_button(ui, tl!("Reset"), 52.0).clicked() {
        apply_shape(app, &[("arcMode", json!("whole")), ("arcStart", json!(0.0)), ("arcEnd", json!(0.0))], json!({ "reset": true }));
    }
    ui.separator();
}

fn draw_rectangle_controls(app: &mut VectorcraftApp, ui: &mut Ui, opts: &Value, selected_custom: Option<CustomShape>) {
    let t = Tokens::get(ui.ctx());
    let unit = app.session.general_unit();
    let sym = opts["cornerSymmetry"].as_bool().unwrap_or(true);
    let mut radii = [opts["cornerRadius"].as_f64().unwrap_or(12.0); 4];
    if let Some(arr) = opts["cornerRadii"].as_array()
        && arr.len() == 4
    {
        for (i, item) in arr.iter().enumerate() {
            radii[i] = item.as_f64().unwrap_or(radii[i]);
        }
    }
    let (mut start_deg, mut end_deg, mut mode) =
        (opts["arcStart"].as_f64().unwrap_or(0.0), opts["arcEnd"].as_f64().unwrap_or(0.0), opts["arcMode"].as_str().unwrap_or("whole"));
    if let Some(CustomShape::Rectangle { radii: nr, pie: (a0, a1), arc_mode, .. }) = selected_custom {
        radii = nr;
        start_deg = a0;
        end_deg = a1;
        mode = mode_str(arc_mode);
    }
    if widgets::icon_button(ui, if sym { "link" } else { "unlink" }, tl!("Link"), sym, 22.0).clicked() {
        let next_sym = !sym;
        apply_shape(app, &[("cornerSymmetry", json!(next_sym))], if next_sym { json!({ "radius": radii[0] }) } else { json!({ "radii": radii }) });
    }
    if sym {
        widgets::field_label(ui, egui::RichText::new(tl!("Radius:")).size(12.0).color(t.text_dim));
        if let Some(v) = widgets::num_field(ui, ("cb-shape", "rect-r"), Some(radii[0]), unit, 54.0) {
            let r = v.max(0.0);
            apply_shape(app, &[("cornerRadius", json!(r)), ("cornerRadii", json!([r, r, r, r]))], json!({ "radius": r }));
        }
    } else {
        for (i, lbl) in [(0, "TL"), (1, "TR"), (2, "BR"), (3, "BL")] {
            widgets::field_label(ui, egui::RichText::new(lbl).size(11.0).color(t.text_dim));
            if let Some(v) = widgets::num_field(ui, ("cb-shape-r", i), Some(radii[i]), unit, 44.0) {
                let mut next = radii;
                next[i] = v.max(0.0);
                apply_shape(app, &[("cornerRadii", json!(next))], json!({ "radii": next }));
            }
        }
    }
    ui.separator();
    draw_arc_controls(app, ui, start_deg, end_deg, mode);
    if widgets::flat_button(ui, tl!("Reset"), 52.0).clicked() {
        apply_shape(
            app,
            &[
                ("cornerRadius", json!(0.0)),
                ("cornerRadii", json!([0.0, 0.0, 0.0, 0.0])),
                ("arcMode", json!("whole")),
                ("arcStart", json!(0.0)),
                ("arcEnd", json!(0.0)),
            ],
            json!({ "reset": true }),
        );
    }
    ui.separator();
}

fn draw_polygon_controls(app: &mut VectorcraftApp, ui: &mut Ui, opts: &Value, active_shape: &str, selected_custom: Option<CustomShape>) {
    let t = Tokens::get(ui.ctx());
    let mut is_star = if active_shape == "star" { true } else { opts["polyIsStar"].as_bool().unwrap_or(false) };
    let mut count = if is_star { opts["points"].as_u64().unwrap_or(5) } else { opts["sides"].as_u64().unwrap_or(6) } as u32;
    let mut spoke = opts["starRatio"].as_f64().unwrap_or(0.5);
    let mut rounded = opts["rounded"].as_f64().unwrap_or(0.0);
    let mut randomized = opts["randomized"].as_f64().unwrap_or(0.0);
    let (mut start_deg, mut end_deg, mut mode) =
        (opts["arcStart"].as_f64().unwrap_or(0.0), opts["arcEnd"].as_f64().unwrap_or(0.0), opts["arcMode"].as_str().unwrap_or("whole"));
    if let Some(CustomShape::Polygon { sides, spoke_ratio, rounded: rnd, randomized: rand, pie: (a0, a1), arc_mode, .. }) = selected_custom {
        count = sides;
        is_star = spoke_ratio.is_some();
        if let Some(sr) = spoke_ratio {
            spoke = sr;
        }
        rounded = rnd;
        randomized = rand;
        start_deg = a0;
        end_deg = a1;
        mode = mode_str(arc_mode);
    }
    if widgets::icon_button(ui, "dc-polygon", tl!("Polygon"), !is_star, 22.0).clicked() {
        apply_shape(app, &[("polyIsStar", json!(false))], json!({ "isStar": false }));
    }
    if widgets::icon_button(ui, "star", tl!("Star"), is_star, 22.0).clicked() {
        apply_shape(app, &[("polyIsStar", json!(true))], json!({ "isStar": true, "spokeRatio": spoke }));
    }
    widgets::field_label(ui, egui::RichText::new(if is_star { tl!("Points:") } else { tl!("Sides:") }).size(12.0).color(t.text_dim));
    if let Some(v) = widgets::plain_field(ui, ("cb-shape", "poly-corners"), f64::from(count), "", 0, 44.0) {
        let c = (v.round() as i64).clamp(3, 1000) as u32;
        apply_shape(app, &[("sides", json!(c)), ("points", json!(c))], json!({ "sides": c }));
    }
    if is_star {
        widgets::field_label(ui, egui::RichText::new(tl!("Ratio")).size(12.0).color(t.text_dim));
        if let Some(v) = widgets::plain_field(ui, ("cb-shape", "poly-spoke"), spoke, "", 2, 46.0) {
            let sr = v.clamp(0.01, 1.0);
            apply_shape(app, &[("starRatio", json!(sr))], json!({ "spokeRatio": sr }));
        }
    }
    widgets::field_label(ui, egui::RichText::new(tl!("Round")).size(12.0).color(t.text_dim));
    if let Some(v) = widgets::plain_field(ui, ("cb-shape", "poly-round"), rounded, "", 2, 46.0) {
        let rnd = v.clamp(-10.0, 10.0);
        apply_shape(app, &[("rounded", json!(rnd))], json!({ "rounded": rnd }));
    }
    widgets::field_label(ui, egui::RichText::new(tl!("Random")).size(12.0).color(t.text_dim));
    if let Some(v) = widgets::plain_field(ui, ("cb-shape", "poly-rand"), randomized, "", 2, 46.0) {
        let rand = v.clamp(-10.0, 10.0);
        apply_shape(app, &[("randomized", json!(rand))], json!({ "randomized": rand }));
    }
    ui.separator();
    draw_arc_controls(app, ui, start_deg, end_deg, mode);
    if widgets::flat_button(ui, tl!("Reset"), 52.0).clicked() {
        apply_shape(
            app,
            &[("rounded", json!(0.0)), ("randomized", json!(0.0)), ("arcMode", json!("whole")), ("arcStart", json!(0.0)), ("arcEnd", json!(0.0))],
            json!({ "reset": true }),
        );
    }
    ui.separator();
}

fn draw_spiral_controls(app: &mut VectorcraftApp, ui: &mut Ui, opts: &Value, selected_custom: Option<CustomShape>) {
    let t = Tokens::get(ui.ctx());
    let (mut turns, mut divergence, mut inner_r) =
        (opts["turns"].as_f64().unwrap_or(3.0), opts["divergence"].as_f64().unwrap_or(1.0), opts["innerRadius"].as_f64().unwrap_or(0.0));
    if let Some(CustomShape::Spiral { turns: t_v, divergence: div_v, inner_radius: ir_v, .. }) = selected_custom {
        turns = t_v;
        divergence = div_v;
        inner_r = ir_v;
    }
    widgets::field_label(ui, egui::RichText::new(tl!("Segments")).size(12.0).color(t.text_dim));
    if let Some(v) = widgets::plain_field(ui, ("cb-shape", "spiral-turns"), turns, "", 2, 50.0) {
        let tv = v.clamp(0.01, 1024.0);
        apply_shape(app, &[("turns", json!(tv))], json!({ "turns": tv }));
    }
    widgets::field_label(ui, egui::RichText::new(tl!("Ratio")).size(12.0).color(t.text_dim));
    if let Some(v) = widgets::plain_field(ui, ("cb-shape", "spiral-div"), divergence, "", 2, 50.0) {
        let dv = v.clamp(0.0, 1000.0);
        apply_shape(app, &[("divergence", json!(dv))], json!({ "divergence": dv }));
    }
    widgets::field_label(ui, egui::RichText::new(tl!("Inner Radius:")).size(12.0).color(t.text_dim));
    if let Some(v) = widgets::plain_field(ui, ("cb-shape", "spiral-inner"), inner_r, "", 2, 50.0) {
        let iv = v.clamp(0.0, 0.999);
        apply_shape(app, &[("innerRadius", json!(iv))], json!({ "innerRadius": iv }));
    }
    if widgets::flat_button(ui, tl!("Reset"), 52.0).clicked() {
        apply_shape(app, &[("turns", json!(3.0)), ("divergence", json!(1.0)), ("innerRadius", json!(0.0))], json!({ "reset": true }));
    }
    ui.separator();
}

fn draw_box3d_controls(app: &mut VectorcraftApp, ui: &mut Ui, opts: &Value, selected_custom: Option<CustomShape>) {
    let t = Tokens::get(ui.ctx());
    let (mut ax, mut ay, mut az) =
        (opts["angleX"].as_f64().unwrap_or(30.0), opts["angleY"].as_f64().unwrap_or(30.0), opts["angleZ"].as_f64().unwrap_or(0.0));
    let (mut px, mut py, mut pz) =
        (opts["parallelX"].as_bool().unwrap_or(false), opts["parallelY"].as_bool().unwrap_or(true), opts["parallelZ"].as_bool().unwrap_or(false));
    if let Some(CustomShape::Box3d { angle_x, angle_y, angle_z, parallel_x, parallel_y, parallel_z, .. }) = selected_custom {
        ax = angle_x;
        ay = angle_y;
        az = angle_z;
        px = parallel_x;
        py = parallel_y;
        pz = parallel_z;
    }
    for (lbl, a_key, p_key, a_val, p_val) in
        [("X:", "angleX", "parallelX", ax, px), ("Y:", "angleY", "parallelY", ay, py), ("Angle:", "angleZ", "parallelZ", az, pz)]
    {
        widgets::field_label(ui, egui::RichText::new(tl!(lbl)).size(12.0).color(t.text_dim));
        if let Some(v) = widgets::plain_field(ui, ("cb-box3d", a_key), a_val, "°", 1, 52.0) {
            apply_shape(app, &[(a_key, json!(v))], json!({ a_key: v }));
        }
        if widgets::icon_button(ui, "dc-line", tl!("Link"), p_val, 20.0).clicked() {
            let next = !p_val;
            apply_shape(app, &[(p_key, json!(next))], json!({ p_key: next }));
        }
    }
    if widgets::flat_button(ui, tl!("Reset"), 52.0).clicked() {
        apply_shape(
            app,
            &[
                ("angleX", json!(30.0)),
                ("angleY", json!(30.0)),
                ("angleZ", json!(0.0)),
                ("parallelX", json!(false)),
                ("parallelY", json!(true)),
                ("parallelZ", json!(false)),
            ],
            json!({ "reset": true }),
        );
    }
    ui.separator();
}

fn mode_str(m: EllipseMode) -> &'static str {
    match m {
        EllipseMode::Slice => "pie",
        EllipseMode::Chord => "chord",
        EllipseMode::Arc => "arc",
        EllipseMode::Whole => "whole",
    }
}
