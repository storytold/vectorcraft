//! The Parametric Shape panel (Path Precision Tools): the selected parametric shape's numbers as
//! fields (kind, count, size, rotation, the kind's own values, corner radius), edited with
//! `addon.pathtools.shape.set`; with no shape selected, the same fields set the Parametric Shape
//! Tool's next shape (`tool.setOption`). Convert turns selected plain shapes into parametric ones.
//! Canceling a field edit or a Shape choice restores its opening value: on a selected shape the
//! edit is one undo step that the cancel takes back, and the tool's next shape keeps its option.
//! For the tool's next shape, a highlighted Shape entry applies only once it is chosen.

use craft::behavior::EditEvents;
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

use super::{row, run};
use crate::canvas::rulers::general_lengths;
use crate::edits::{EditPath, apply_event};
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

/// Whether anchors are direct-selected, so that the selection's box is the box around them.
fn anchors_selected(app: &VectorcraftApp) -> bool {
    app.session
        .active()
        .is_some_and(|st| vectorcraft_effects::preview::anchor_bounds_with(&st.selection, &st.doc, app.session.bounds_measure()).is_some())
}

/// Whether something is selected and none of it is a parametric shape Convert could take.
pub(super) fn plain_selected(app: &VectorcraftApp) -> bool {
    app.session.active().is_some_and(|st| !st.selection.is_empty()) && selected(app).is_none()
}

/// The params a number field's edits send, `to` of each number it leaves; a cleared field sends none,
/// and its gesture still begins and ends.
fn number(events: EditEvents<Option<f64>>, to: impl Fn(f64) -> Value) -> EditEvents<Option<Value>> {
    events.map(|v| v.map(&to))
}

/// Runs `addon.pathtools.shape.set` with `p` on the selected shape `id`.
fn set(app: &mut VectorcraftApp, id: NodeId, p: &Value) {
    let mut p = p.clone();
    p["id"] = json!(id.0);
    // The Transform panel and the Control bar share one reference point locator. W and H
    // keep the point of the shape's bounding box chosen there in place. With anchors
    // direct-selected the locator shows their box, so the shape keeps its center instead.
    if p.get("width").or_else(|| p.get("height")).is_some() && !anchors_selected(app) {
        p["reference"] = json!(app.ui.transform_panel.reference_point);
    }
    run(app, "addon.pathtools.shape.set", p);
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    if app.session.active().is_none() {
        empty_state(ui, "dc-parametric-shape", tl!("No document"), tl!("Open a document to draw parametric shapes."));
        return;
    }
    let (id, s) = target(app);
    let plain = plain_selected(app);
    let lengths = general_lengths(app);
    if id.is_none() {
        Text::new(tl!("Select a parametric shape to edit it. With none selected, these set the next shape.")).show(ui);
    }
    // Each field's edits this pass, as the params they send.
    let mut edits: Vec<EditEvents<Option<Value>>> = Vec::new();
    let entries: Vec<Entry<ShapeKind>> = ShapeKind::ALL
        .into_iter()
        .map(|k| {
            let label = kind_label(k);
            Entry::new(k, tl!(label))
        })
        .collect();
    let kind = |ui: &mut Ui| Some(Select::new(tl!("Shape"), Bind::given(s.kind), &entries).show(ui).events.map(|k| Some(json!({ "kind": k.id() }))));
    let shape = row(ui, "Shape", "parametric-shape-kind", kind).unwrap_or_default();
    if s.kind.counted() {
        let count = |ui: &mut Ui| {
            let field = NumberField::new(tl!("Count"), Bind::given(Some(f64::from(s.count)))).range(f64::from(MIN_COUNT)..=f64::from(MAX_COUNT));
            Some(number(field.fixed_decimals(0).show(ui).edit.events, |v| json!({ "count": v })))
        };
        edits.extend(row(ui, "Count", "parametric-shape-count", count));
    }
    for (label, key, v) in [("W", "width", s.width), ("H", "height", s.height)] {
        let size = |ui: &mut Ui| {
            let field = fields::size_field(ui, tl!(label), Some(v), lengths).range(MIN_SIZE..=MAX_SIZE);
            Some(number(field.show(ui).edit.events, |v| json!({ key: v })))
        };
        edits.extend(row(ui, label, key, size));
    }
    let rotation = |ui: &mut Ui| {
        let field = NumberField::new(tl!("Rotation"), Bind::given(Some(s.rotation))).unit_kind(UnitKind::Degrees);
        Some(number(field.fixed_decimals(1).show(ui).edit.events, |v| json!({ "rotation": v })))
    };
    edits.extend(row(ui, "Rotation", "parametric-shape-rotation", rotation));
    // The kind's own values, as percentages of their fraction.
    for ((key, v), (range, label)) in [("a", s.a), ("b", s.b)].into_iter().zip(s.kind.values().into_iter().zip(value_labels(s.kind))) {
        let Some(r) = range else { continue };
        let own = |ui: &mut Ui| {
            let field = NumberField::new(tl!(label), Bind::given(Some(v * 100.0))).range(r.min * 100.0..=r.max * 100.0);
            Some(number(field.unit_kind(UnitKind::Percent).fixed_decimals(0).show(ui).edit.events, |x| json!({ key: x / 100.0 })))
        };
        edits.extend(row(ui, label, key, own));
    }
    if s.kind.rounds() {
        let radius = |ui: &mut Ui| {
            let field = fields::size_field(ui, tl!("Corner Radius"), Some(s.radius), lengths).range(0.0..=MAX_SIZE);
            Some(number(field.show(ui).edit.events, |v| json!({ "radius": v })))
        };
        edits.extend(row(ui, "Corner Radius", "parametric-shape-radius", radius));
    }
    Divider::horizontal().show(ui);
    let convert = clicked(&Button::new(tl!("Convert to Parametric Shape")).sizing(Sizing::FillWidth).enabled(plain).show(ui));
    match id {
        // One undo step per gesture; a cancel reverts it.
        Some(id) => {
            for event in shape.iter().chain(edits.iter().flatten()) {
                apply_event(app, event, &EditPath::Scope, &mut |app, p: &Option<Value>, _| {
                    if let Some(p) = p {
                        set(app, id, p);
                    }
                });
            }
        }
        // The value each field holds after its events: a cancel's is the opening value. A Shape entry
        // applies once it is chosen, since a new kind resets the kind's own count and values.
        None => {
            for values in shape.committed().into_iter().chain(edits.iter().filter_map(EditEvents::landed)).flatten() {
                run(app, "tool.setOption", json!({ "tool": tool::ID, "values": values }));
            }
        }
    }
    if convert {
        run(app, "addon.pathtools.shape.convert", json!({}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::panel_harness::Panel;
    use craft::ui::contract::Role;
    use egui::{Key, Modifiers};
    use vectorcraft_tools::{PointerEvent, PointerKind};

    /// The text fields with what each test types: the tool kind that shows the field, its label, its option key and
    /// the text typed.
    const TYPED: [(&str, &str, &str, &str); 7] = [
        ("gear", "W", "width", "160"),
        ("gear", "H", "height", "40"),
        ("gear", "Count", "count", "9"),
        ("gear", "Rotation", "rotation", "30"),
        ("gear", "Tooth Height", "a", "50"),
        ("arrow", "Shaft", "b", "70"),
        ("gear", "Corner Radius", "radius", "7"),
    ];

    fn app(kind: &str) -> VectorcraftApp {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("tool.setOption", json!({"tool": tool::ID, "values": {"kind": kind}})).unwrap();
        app
    }

    /// `app` with a shape of `kind` drawn and selected.
    fn app_with_shape(kind: &str) -> (VectorcraftApp, NodeId) {
        let mut app = app(kind);
        let params = json!({"kind": kind, "center": [150, 150], "width": 100, "height": 80, "radius": 2});
        let id = NodeId(app.run("addon.pathtools.shape.create", params).unwrap()["id"].as_u64().unwrap());
        (app, id)
    }

    fn shape_of(app: &VectorcraftApp, id: NodeId) -> ParametricShape {
        app.session.active().and_then(|st| st.doc.node(id)).and_then(read).unwrap()
    }

    fn undo_len(app: &VectorcraftApp) -> usize {
        app.session.active().unwrap().history.undo.len()
    }

    /// The tool's next shape from the tool options `options`.
    fn next_shape(options: &Value) -> ParametricShape {
        let fallback = ParametricShape::new(ShapeKind::Polygon, Point::ZERO, 100.0, 100.0);
        apply(fallback, options).unwrap()
    }

    /// Double-clicks the canvas with the Parametric Shape Tool and answers the shape it places.
    fn draw_next(app: &mut VectorcraftApp) -> ParametricShape {
        app.run("select.set", json!({"ids": []})).unwrap();
        app.select_tool(tool::ID);
        let view = app.view_info();
        crate::canvas::dispatch(app, &PointerEvent::new(PointerKind::DoubleClick, 150.0, 150.0), view);
        let (_, shape) = selected(app).expect("the double-click places a selected parametric shape");
        shape
    }

    #[test]
    fn canceling_a_next_shape_field_restores_the_tool_option_and_enter_keeps_it() {
        for (kind, label, key, text) in TYPED {
            for cancel in [false, true] {
                let mut panel = Panel::new(app(kind), show);
                let before = panel.app().session.tool_options_of(tool::ID);
                let doc = panel.app().session.active().unwrap().doc.clone();
                let undo = undo_len(&panel.app());
                panel.click_role(label, Role::SPIN_BUTTON);
                panel.key(Key::A, Modifiers::COMMAND);
                panel.type_text(text);
                panel.frame();
                let proposed = panel.app().session.tool_options_of(tool::ID);
                assert_ne!(proposed[key], before[key], "{label} changes the next shape while typing");
                if cancel {
                    panel.key(Key::Escape, Modifiers::NONE);
                    assert_eq!(panel.app().session.tool_options_of(tool::ID), before, "{label} restores on Escape");
                } else {
                    panel.key(Key::Enter, Modifiers::NONE);
                }
                panel.frame();
                let kept = if cancel { before.clone() } else { proposed.clone() };
                assert_eq!(panel.app().session.tool_options_of(tool::ID), kept, "{label}, canceled: {cancel}");
                assert!(panel.shows(label), "{label}: the panel stays open");
                let mut app = panel.app();
                assert_eq!(app.session.active().unwrap().doc, doc, "{label} only changes the tool's next shape");
                assert_eq!(undo_len(&app), undo, "{label} adds no undo step");
                let drawn = draw_next(&mut app);
                assert_eq!(drawn, ParametricShape { center: drawn.center, ..next_shape(&kept) }, "{label}: the next shape drawn");
            }
        }
    }

    #[test]
    fn canceling_a_next_shape_kind_keeps_the_tool_options_and_enter_chooses_it() {
        let mut app = app("gear");
        app.run("tool.setOption", json!({"tool": tool::ID, "values": {"count": 9, "a": 0.5}})).unwrap();
        let mut panel = Panel::new(app, show);
        let before = panel.app().session.tool_options_of(tool::ID);
        panel.click_role("Shape", Role::COMBO_BOX);
        panel.key(Key::ArrowDown, Modifiers::NONE);
        panel.frame();
        assert_eq!(panel.app().session.tool_options_of(tool::ID), before, "a highlighted entry is not applied before it is chosen");
        panel.key(Key::Escape, Modifiers::NONE);
        panel.frame();
        assert_eq!(panel.app().session.tool_options_of(tool::ID), before, "Escape keeps the next shape's kind, count and values");
        assert!(panel.shows("Shape"), "the panel stays open");
        {
            let mut app = panel.app();
            let drawn = draw_next(&mut app);
            assert_eq!(drawn, ParametricShape { center: drawn.center, ..next_shape(&before) }, "the next shape drawn keeps the earlier options");
            assert_eq!((drawn.kind, drawn.count), (ShapeKind::Gear, 9));
            app.select_tool("selection");
            app.run("select.set", json!({"ids": []})).unwrap();
        }
        panel.frame();
        panel.click_role("Shape", Role::COMBO_BOX);
        panel.key(Key::ArrowDown, Modifiers::NONE);
        panel.key(Key::Enter, Modifiers::NONE);
        panel.frame();
        let chosen = panel.app().session.tool_options_of(tool::ID);
        assert_eq!(chosen["kind"], json!("bubble"), "Enter chooses the highlighted kind");
    }

    #[test]
    fn canceling_a_selected_shape_field_restores_the_shape_and_enter_is_one_undo_step() {
        for (kind, label, _, text) in TYPED {
            for cancel in [false, true] {
                let (app, id) = app_with_shape(kind);
                let mut panel = Panel::new(app, show);
                let options = panel.app().session.tool_options_of(tool::ID);
                let doc = panel.app().session.active().unwrap().doc.clone();
                let before = shape_of(&panel.app(), id);
                let undo = undo_len(&panel.app());
                panel.click_role(label, Role::SPIN_BUTTON);
                panel.key(Key::A, Modifiers::COMMAND);
                panel.type_text(text);
                panel.frame();
                let proposed = shape_of(&panel.app(), id);
                assert_ne!(proposed, before, "{label} changes the shape while typing");
                if cancel {
                    panel.key(Key::Escape, Modifiers::NONE);
                } else {
                    panel.key(Key::Enter, Modifiers::NONE);
                }
                panel.frame();
                assert!(panel.shows(label), "{label}: the panel stays open");
                let app = panel.app();
                if cancel {
                    assert_eq!(shape_of(&app, id), before, "{label} restores the shape on Escape");
                    assert_eq!(app.session.active().unwrap().doc, doc, "{label}: Escape leaves the document as it was");
                    assert_eq!(undo_len(&app), undo, "{label}: Escape leaves no undo step");
                } else {
                    assert_eq!(shape_of(&app, id), proposed, "{label}: Enter keeps the edit");
                    assert_eq!(undo_len(&app), undo + 1, "{label}: the edit is one undo step");
                }
                assert_eq!(app.session.tool_options_of(tool::ID), options, "{label} leaves the tool's next shape alone");
            }
        }
    }

    #[test]
    fn a_selected_shape_field_left_empty_ends_its_undo_step() {
        let (app, id) = app_with_shape("gear");
        let mut panel = Panel::new(app, show);
        let undo = undo_len(&panel.app());
        panel.click_role("W", Role::SPIN_BUTTON);
        panel.key(Key::A, Modifiers::COMMAND);
        panel.type_text("160");
        panel.frame();
        let typed = shape_of(&panel.app(), id);
        assert!((typed.width - 160.0).abs() < 1e-9, "W changes the shape while typing: {typed:?}");
        panel.key(Key::A, Modifiers::COMMAND);
        panel.key(Key::Backspace, Modifiers::NONE);
        panel.key(Key::Enter, Modifiers::NONE);
        panel.frame();
        let mut app = panel.app();
        assert_eq!(shape_of(&app, id), typed, "an empty field leaves the last width");
        assert_eq!(undo_len(&app), undo + 1, "the edit is one undo step");
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap();
        assert_eq!(undo_len(&app), undo + 2, "the next edit is a step of its own");
    }

    #[test]
    fn canceling_a_selected_shape_kind_restores_the_shape_and_enter_is_one_undo_step() {
        for cancel in [false, true] {
            let (app, id) = app_with_shape("gear");
            let mut panel = Panel::new(app, show);
            let before = shape_of(&panel.app(), id);
            let undo = undo_len(&panel.app());
            panel.click_role("Shape", Role::COMBO_BOX);
            panel.key(Key::ArrowDown, Modifiers::NONE);
            panel.frame();
            let proposed = shape_of(&panel.app(), id);
            assert_ne!(proposed.kind, before.kind, "the highlighted entry previews the kind");
            panel.key(if cancel { Key::Escape } else { Key::Enter }, Modifiers::NONE);
            panel.frame();
            assert!(panel.shows("Shape"), "the panel stays open");
            let app = panel.app();
            if cancel {
                assert_eq!(shape_of(&app, id), before, "Escape restores the kind");
                assert_eq!(undo_len(&app), undo, "Escape leaves no undo step");
            } else {
                assert_eq!(shape_of(&app, id).kind, proposed.kind, "Enter keeps the kind");
                assert_eq!(undo_len(&app), undo + 1, "the choice is one undo step");
            }
        }
    }
}
