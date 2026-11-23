//! The Parametric Shape panel (Path Precision Tools): the selected parametric shape's numbers as
//! fields (kind, count, size, rotation, the kind's own values, corner radius), edited with
//! `addon.pathtools.shape.set`; with no shape selected, the same fields set the Parametric Shape
//! Tool's next shape (`tool.setOption`). Convert turns selected plain shapes into parametric ones.

use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, Sizing, clicked};
use craft::ui::display::{Divider, Text};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::UnitKind;
use craft::ui::text::NumberField;
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_geom::Point;
use vectorcraft_tools::addons::parametric_shape::{self as tool, MAX_COUNT, MAX_SIZE, MIN_COUNT, MIN_SIZE, ParametricShape, ShapeKind, apply, read};

use super::{row, run, typed};
use crate::canvas::rulers::general_lengths;
use crate::panels::empty_state;
use crate::{VectorcraftApp, fields};

/// The panel's id (`window.panel {panel: "parametricShape"}`).
pub const ID: &str = "parametricShape";

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
        ShapeKind::Star | ShapeKind::Ring => ["Inner Radius", ""],
        ShapeKind::Arrow => ["Head", "Shaft"],
        ShapeKind::Cross => ["Offset", "Thickness"],
        ShapeKind::Heart => ["Depth", "Point"],
        ShapeKind::Gear => ["Tooth Height", ""],
        ShapeKind::Bubble => ["Tail", "Tip"],
        ShapeKind::Rectangle | ShapeKind::Ellipse | ShapeKind::Polygon => ["", ""],
    }
}

/// The one selected object that reads as a parametric shape.
fn selected(app: &VectorcraftApp) -> Option<(NodeId, ParametricShape)> {
    let st = app.session.active()?;
    match st.selection.objects.as_slice() {
        [id] => st.doc.node(*id).and_then(read).map(|s| (*id, s)),
        _ => None,
    }
}

/// What the panel edits: the selected parametric shape, else the tool's next shape over a 100 × 100 polygon.
pub(super) fn target(app: &VectorcraftApp) -> (Option<NodeId>, ParametricShape) {
    if let Some((id, shape)) = selected(app) {
        return (Some(id), shape);
    }
    let fallback = ParametricShape::new(ShapeKind::Polygon, Point::ZERO, 100.0, 100.0);
    (None, apply(fallback, &app.session.tool_options_of(tool::ID)).unwrap_or(fallback))
}

/// Whether something is selected and none of it is a parametric shape Convert could take.
pub(super) fn plain_selected(app: &VectorcraftApp) -> bool {
    app.session.active().is_some_and(|st| !st.selection.is_empty()) && selected(app).is_none()
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    if app.session.active().is_none() {
        empty_state(ui, "hexagon", tl!("No document"), tl!("Open a document to draw parametric shapes."));
        return;
    }
    let (id, s) = target(app);
    let plain = plain_selected(app);
    let lengths = general_lengths(app);
    if id.is_none() {
        Text::new(tl!("Select a parametric shape to edit it. With none selected, these set the next shape.")).show(ui);
    }
    let mut change: Option<Value> = None;
    let entries: Vec<Entry<ShapeKind>> = ShapeKind::ALL
        .into_iter()
        .map(|k| {
            let label = kind_label(k);
            Entry::new(k, tl!(label))
        })
        .collect();
    let kind = |ui: &mut Ui| Select::new(tl!("Shape"), Bind::given(s.kind), &entries).show(ui).value().copied();
    if let Some(k) = row(ui, "Shape", "parametric-shape-kind", kind) {
        change = Some(json!({ "kind": k.id() }));
    }
    if s.kind.counted() {
        let count = |ui: &mut Ui| {
            let field = NumberField::new(tl!("Count"), Bind::given(Some(f64::from(s.count)))).range(f64::from(MIN_COUNT)..=f64::from(MAX_COUNT));
            typed(field.fixed_decimals(0).show(ui).value())
        };
        if let Some(v) = row(ui, "Count", "parametric-shape-count", count) {
            change = Some(json!({ "count": v }));
        }
    }
    for (label, key, v) in [("W", "width", s.width), ("H", "height", s.height)] {
        let size = |ui: &mut Ui| typed(fields::size_field(ui, tl!(label), Some(v), lengths).range(MIN_SIZE..=MAX_SIZE).show(ui).value());
        if let Some(v) = row(ui, label, key, size) {
            change = Some(json!({ key: v }));
        }
    }
    let rotation = |ui: &mut Ui| {
        let field = NumberField::new(tl!("Rotation"), Bind::given(Some(s.rotation))).unit_kind(UnitKind::Degrees);
        typed(field.fixed_decimals(1).show(ui).value())
    };
    if let Some(v) = row(ui, "Rotation", "parametric-shape-rotation", rotation) {
        change = Some(json!({ "rotation": v }));
    }
    // The kind's own values, as percentages of their fraction.
    for ((key, v), (range, label)) in [("a", s.a), ("b", s.b)].into_iter().zip(s.kind.values().into_iter().zip(value_labels(s.kind))) {
        let Some(r) = range else { continue };
        let own = |ui: &mut Ui| {
            let field = NumberField::new(tl!(label), Bind::given(Some(v * 100.0))).range(r.min * 100.0..=r.max * 100.0);
            typed(field.unit_kind(UnitKind::Percent).fixed_decimals(0).show(ui).value())
        };
        if let Some(x) = row(ui, label, key, own) {
            change = Some(json!({ key: x / 100.0 }));
        }
    }
    if s.kind.rounds() {
        let radius =
            |ui: &mut Ui| typed(fields::size_field(ui, tl!("Corner Radius"), Some(s.radius), lengths).range(0.0..=MAX_SIZE).show(ui).value());
        if let Some(v) = row(ui, "Corner Radius", "parametric-shape-radius", radius) {
            change = Some(json!({ "radius": v }));
        }
    }
    Divider::horizontal().show(ui);
    let convert = clicked(&Button::new(tl!("Convert to Parametric Shape")).sizing(Sizing::FillWidth).enabled(plain).show(ui));
    match (change, id) {
        (Some(mut p), Some(id)) => {
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
