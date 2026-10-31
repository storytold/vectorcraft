//! The Parametric Shape panel (Path Precision Tools): the selected parametric shape's numbers as
//! fields (kind, count, size, rotation, the kind's own values, corner radius), edited with
//! `addon.pathtools.shape.set`; with no shape selected, the same fields set the Parametric Shape
//! Tool's next shape (`tool.setOption`). Convert turns selected plain shapes into parametric ones.

use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_geom::Point;
use vectorcraft_tools::addons::parametric_shape::{self as tool, MAX_COUNT, MAX_SIZE, MIN_COUNT, ParametricShape, ShapeKind, apply, read};

use crate::VectorcraftApp;
use crate::panels::empty_state;
use crate::widgets::{self, dim_label};

/// The panel's id (`window.panel {panel: "parametricShape"}`).
pub const ID: &str = "parametricShape";
const LABEL_W: f32 = 96.0;
const FIELD_W: f32 = 72.0;

/// The kind's name in the interface.
fn kind_label(kind: ShapeKind) -> &'static str {
    match kind {
        ShapeKind::Rectangle => "Rectangle",
        ShapeKind::Ellipse => "Ellipse",
        ShapeKind::Polygon => "Polygon",
        ShapeKind::Star => "Star",
        ShapeKind::Ring => "Ring",
        ShapeKind::Arrow => "Arrow",
        ShapeKind::Cross => "Cross",
        ShapeKind::Heart => "Heart",
        ShapeKind::Gear => "Gear",
        ShapeKind::Bubble => "Speech Bubble",
    }
}

/// The labels of the kind's own values `a` and `b`.
fn value_labels(kind: ShapeKind) -> [&'static str; 2] {
    match kind {
        ShapeKind::Star | ShapeKind::Ring => ["Inner Radius:", ""],
        ShapeKind::Arrow => ["Head:", "Shaft:"],
        ShapeKind::Cross => ["Offset:", "Thickness:"],
        ShapeKind::Heart => ["Depth:", "Point:"],
        ShapeKind::Gear => ["Tooth Height:", ""],
        ShapeKind::Bubble => ["Tail:", "Tip:"],
        ShapeKind::Rectangle | ShapeKind::Ellipse | ShapeKind::Polygon => ["", ""],
    }
}

fn run(app: &mut VectorcraftApp, id: &str, p: Value) {
    if let Err(e) = app.run(id, p) {
        app.status(e);
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(st) = app.session.active() else {
        empty_state(ui, "hexagon", tl!("No document"), tl!("Open a document to draw parametric shapes."));
        return;
    };
    // The selected shape, else the tool's next one.
    let selected = match st.selection.objects.as_slice() {
        [id] => st.doc.node(*id).and_then(read).map(|s| (*id, s)),
        _ => None,
    };
    let plain_selected = !st.selection.is_empty() && selected.is_none();
    let fallback = ParametricShape::new(ShapeKind::Polygon, Point::ZERO, 100.0, 100.0);
    let s = selected.map(|(_, s)| s).unwrap_or_else(|| apply(fallback, &app.session.tool_options_of(tool::ID)).unwrap_or(fallback));
    let unit = app.session.general_unit();
    if selected.is_none() {
        dim_label(ui, tl!("Select a parametric shape to edit it. With none selected, these set the next shape."));
    }
    let mut change: Option<Value> = None;
    let labels = ShapeKind::ALL.map(kind_label);
    widgets::label_row(ui, "Shape:", LABEL_W, |ui| {
        if let Some(k) = widgets::dropdown(ui, "parametric-shape-kind", kind_label(s.kind), &labels, 150.0).and_then(|i| ShapeKind::ALL.get(i)) {
            change = Some(json!({ "kind": k.id() }));
        }
    });
    if s.kind.counted() {
        widgets::label_row(ui, "Count:", LABEL_W, |ui| {
            let range = f64::from(MIN_COUNT)..=f64::from(MAX_COUNT);
            if let Some(v) = widgets::range_field(ui, "parametric-shape-count", f64::from(s.count), range, "", 0, FIELD_W) {
                change = Some(json!({ "count": v }));
            }
        });
    }
    // Width and height side by side, as the Transform panel has them.
    egui::Grid::new("parametric-shape-size").num_columns(4).spacing([4.0, 6.0]).min_col_width(0.0).show(ui, |ui| {
        for (label, key, v) in [("W:", "width", s.width), ("H:", "height", s.height)] {
            dim_label(ui, label);
            if let Some(v) = widgets::size_field_named(ui, ("parametric-shape", key), Some(tl!(label)), Some(v), unit, FIELD_W) {
                change = Some(json!({ key: v }));
            }
        }
        ui.end_row();
    });
    widgets::label_row(ui, "Rotation:", LABEL_W, |ui| {
        if let Some(v) = widgets::mixed_field(ui, "parametric-shape-rotation", Some(s.rotation), "°", 1, FIELD_W) {
            change = Some(json!({ "rotation": v }));
        }
    });
    // The kind's own values, as percentages of their fraction.
    for ((key, v), (range, label)) in [("a", s.a), ("b", s.b)].into_iter().zip(s.kind.values().into_iter().zip(value_labels(s.kind))) {
        let Some(r) = range else { continue };
        widgets::label_row(ui, label, LABEL_W, |ui| {
            if let Some(x) = widgets::range_field(ui, ("parametric-shape", key), v * 100.0, r.min * 100.0..=r.max * 100.0, "%", 0, FIELD_W) {
                change = Some(json!({ key: x / 100.0 }));
            }
        });
    }
    if s.kind.rounds() {
        widgets::label_row(ui, "Corner Radius:", LABEL_W, |ui| {
            if let Some(v) = widgets::size_field(ui, "parametric-shape-radius", Some(s.radius), unit, FIELD_W) {
                change = Some(json!({ "radius": v.clamp(0.0, MAX_SIZE) }));
            }
        });
    }
    widgets::divider(ui);
    let width = ui.available_width();
    let convert = ui.add_enabled_ui(plain_selected, |ui| widgets::flat_button(ui, tl!("Convert to Parametric Shape"), width).clicked()).inner;
    match (change, selected) {
        (Some(mut p), Some((id, _))) => {
            p["id"] = json!(id.0);
            run(app, "addon.pathtools.shape.set", p);
        }
        (Some(values), None) => run(app, "tool.setOption", json!({ "tool": tool::ID, "values": values })),
        (None, _) => {}
    }
    if convert {
        run(app, "addon.pathtools.shape.convert", json!({}));
    }
}
