//! The 3D Box and customizable shapes add-on in the interface: its submenu (Object › Plug-ins ›
//! 3D Box), which selects the 3D Box tool, and the context-sensitive Control bar options shown
//! when a shape tool (`rectangle`, `roundedRectangle`, `ellipse`, `polygon`, `star`, `spiral`,
//! `box3d`) is active. Canceling a numeric edit restores its future-tool options, including coupled
//! scalar and per-corner radii; a retired edit cannot restore options into a different tool.

use std::ops::RangeInclusive;

use craft::behavior::{CancelReason, EditEvent, GestureId, IconId, NumberCodec};
use craft::menu::Node;
use craft::ui::buttons::{Button, ToggleButton};
use craft::ui::choice::Segmented;
use craft::ui::contract::{Bind, EditResponse, Entry, Props as _, Show as _, ShowContainer as _, SideLabel};
use craft::ui::display::Divider;
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::runtime::env::{Env, LengthUnits, UnitKind};
use craft::ui::text::NumberField;
use egui::Ui;
use serde_json::{Map, Value, json};
use vectorcraft_geom::shapes::EllipseMode;
use vectorcraft_tools::addons::box3d::{self, CustomShape};

use crate::VectorcraftApp;
use crate::canvas::rulers::general_lengths;
use crate::chrome::rank;
use crate::edits::{self, EditPath};
use crate::menus::cp;
use crate::panels::{Live, live_run};

/// A choice's value, its label and its icon name.
type Choice = (&'static str, &'static str, &'static str);

pub fn menu() -> Vec<Node> {
    vec![cp("3D Box Tool", "tool.select", json!({"tool": box3d::ID}))]
}

/// Draw context-sensitive shape customization controls in the top Control bar when a shape tool is
/// active. Returns `true` when a shape tool is active. Every member is a ranked row, so a narrow bar
/// leaves members out from the reading end, last first.
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

    divider(ui);
    if tool == "box3d" {
        let shapes: [Choice; 6] =
            [("box3d", "3D Box"), ("rectangle", "Rectangle"), ("ellipse", "Ellipse"), ("polygon", "Polygon"), ("star", "Star"), ("spiral", "Spiral")]
                .map(|(id, label)| (id, label, icon_for_shape(id)));
        if let Some(shape) = choice(ui, "Shape", &active_shape, &shapes) {
            app.run("tool.setOption", json!({ "key": "shape", "value": shape })).ok();
        }
        divider(ui);
    }
    // The tools with live shape widgets: Hide/Show Shape Widgets first, so a narrow bar leaves out
    // the shape's fields before it.
    if vectorcraft_tools::shape::shows_shape_widgets(&tool) {
        // The command is in no menu, so the button names it: Hide, or Show while the widgets are hidden.
        let label = if app.ui.view.shape_widgets { tl!("Hide Shape Widgets") } else { tl!("Show Shape Widgets") };
        let mut clicked = false;
        member(ui, |ui| clicked |= Button::new(label.to_string()).show(ui).clicked());
        if clicked {
            crate::menus::invoke(app, "view.shapeWidgets", json!({}));
        }
        divider(ui);
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

/// One ranked member of the group: the unit a narrow bar leaves out whole.
fn member(ui: &mut Ui, mut add: impl FnMut(&mut Ui)) {
    Row::new().fit_rank(rank::GROUP).show(ui, |ui, _| add(ui));
}

/// The rule that starts a group of members.
fn divider(ui: &mut Ui) {
    member(ui, |ui| {
        Divider::vertical().show(ui);
    });
}

/// The translated `text` as an owned label.
fn tr(text: &str) -> String {
    tl!(text).to_string()
}

/// A member with one icon segment per choice over the current `current`; the value picked, if any.
fn choice(ui: &mut Ui, caption: &'static str, current: &str, choices: &[Choice]) -> Option<&'static str> {
    let entries: Vec<Entry<&'static str>> =
        choices.iter().map(|&(value, label, icon)| Entry::new(value, tr(label)).icon(IconId::new(icon))).collect();
    let current = choices.iter().map(|c| c.0).find(|&value| value == current).unwrap_or("");
    let mut picked = None;
    member(ui, |ui| {
        picked = Segmented::new(tr(caption), Bind::given(current), &entries).show(ui).value().copied();
    });
    picked
}

/// A member with a two-state icon button; the new state when it was clicked.
fn link(ui: &mut Ui, on: bool, icon: IconId, on_icon: Option<IconId>) -> Option<bool> {
    let mut flipped = None;
    member(ui, |ui| {
        let mut button = ToggleButton::new(tr("Link"), Bind::given(on)).icon(icon.clone()).icon_only();
        if let Some(on_icon) = &on_icon {
            button = button.on_icon(on_icon.clone());
        }
        flipped = button.show(ui).value().copied();
    });
    flipped
}

/// A member with the Reset button; whether it was clicked.
fn reset(ui: &mut Ui) -> bool {
    let mut clicked = false;
    member(ui, |ui| clicked = Button::new(tr("Reset")).show(ui).clicks > 0);
    clicked
}

/// The caption of a field without its colon: the form adds it.
fn caption_of(caption: &str) -> String {
    tl!(caption).trim_end_matches(':').to_string()
}

/// A number field of `kind` (none for a plain number) written with `decimals`, within `range` when given.
fn plain(caption: &str, value: f64, kind: Option<UnitKind>, decimals: usize, range: Option<RangeInclusive<f64>>, ui: &Ui) -> NumberField<'static> {
    let codec = NumberCodec::new(Env::of(ui).units().reader(kind), decimals);
    let mut field = NumberField::new(caption_of(caption), Bind::given(Some(value))).codec(codec);
    if let Some(kind) = kind {
        field = field.unit_kind(kind);
    }
    match range {
        Some(range) => field.range(range),
        None => field,
    }
}

/// A length field keeping points, shown in the user's unit, that is never negative.
fn length(caption: &str, value: f64, lengths: LengthUnits) -> NumberField<'static> {
    NumberField::length(caption_of(caption), Bind::given(Some(value)), lengths).range(0.0..=f64::MAX)
}

/// A member with `field` under a scrubbable side label. A committed or scrubbed value goes to `apply`
/// with its phase, so a scrub is one live edit; an emptied field changes nothing.
fn number(
    app: &mut VectorcraftApp,
    ui: &mut Ui,
    key: &'static str,
    caption: &str,
    field: NumberField<'static>,
    mut apply: impl FnMut(&mut VectorcraftApp, f64, Live),
) {
    let mut field = Some(field.scrub_by_label());
    member(ui, |ui| {
        let mut response: Option<EditResponse<Option<f64>>> = None;
        Field::new(caption_of(caption)).id_salt(("cb-shape", key)).side(SideLabel::new().scrub(true)).show(ui, |ui, _| {
            response = field.take().map(|field| field.show(ui).edit);
        });
        if let Some(response) = response {
            shape_number_edits(app, ui.ctx(), key, &response, |app, value, phase| {
                if let Some(value) = *value {
                    apply(app, value, phase);
                }
            });
        }
    });
}

#[derive(Clone)]
struct ShapeToolGesture {
    id: GestureId,
    tool: String,
    target: Option<(u64, u64, Vec<vectorcraft_doc::NodeId>)>,
    opening: Map<String, Value>,
    latest: Value,
}

fn shape_target(app: &VectorcraftApp) -> Option<(u64, u64, Vec<vectorcraft_doc::NodeId>)> {
    app.session.active().map(|st| (st.uid, st.reverts, st.selection.objects.clone()))
}

/// Tool options are outside the document preview snapshot. A canceled scrub restores this
/// field's complete option group, including settings that the rollback changes indirectly.
fn shape_number_edits(
    app: &mut VectorcraftApp,
    ctx: &egui::Context,
    field: &'static str,
    response: &EditResponse<Option<f64>>,
    mut apply: impl FnMut(&mut VectorcraftApp, &Option<f64>, Live),
) {
    let keys: &[&str] = match field {
        "arc-start" => &["arcStart", "arcMode"],
        "arc-end" => &["arcEnd", "arcMode"],
        "rect-r" => &["cornerRadius", "cornerRadii"],
        "rect-tl" | "rect-tr" | "rect-br" | "rect-bl" => &["cornerRadii", "cornerRadius"],
        "poly-corners" => &["sides", "points"],
        "poly-spoke" => &["starRatio"],
        "poly-round" => &["rounded"],
        "poly-rand" => &["randomized"],
        "spiral-turns" => &["turns"],
        "spiral-div" => &["divergence"],
        "spiral-inner" => &["innerRadius"],
        _ => std::slice::from_ref(&field),
    };
    let key = response.response.id.with("shape-tool-gesture");
    let mut held = ctx.data(|data| data.get_temp::<ShapeToolGesture>(key));
    for event in response.events.iter() {
        if let EditEvent::Begin { id, .. } = event {
            let latest = app.session.tool_options();
            held = Some(ShapeToolGesture {
                id: *id,
                tool: app.session.tool_id().to_owned(),
                target: shape_target(app),
                opening: keys.iter().filter_map(|key| latest.get(*key).cloned().map(|value| ((*key).to_owned(), value))).collect(),
                latest,
            });
        }
        let restore = if let EditEvent::Cancel { id, before, reason, .. } = event {
            if held.as_ref().is_some_and(|snapshot| snapshot.id == *id) {
                held.take().filter(|snapshot| {
                    before.is_some()
                        && !reason.document_moved()
                        && *reason != CancelReason::Removed
                        && snapshot.tool == app.session.tool_id()
                        && snapshot.target == shape_target(app)
                        && snapshot.latest == app.session.tool_options()
                })
            } else {
                None
            }
        } else {
            None
        };
        edits::apply_event(app, event, &EditPath::Preview, &mut apply);
        if let Some(snapshot) = restore {
            // Radius writes the array with symmetry on; the array writes the scalar either way.
            // Each row's order restores its full representation rather than sorted JSON keys.
            for key in keys {
                if let Some(value) = snapshot.opening.get(*key)
                    && let Err(error) = app.run("tool.setOption", json!({"tool": snapshot.tool, "key": key, "value": value}))
                {
                    app.status(error);
                    break;
                }
            }
        }
        match event {
            EditEvent::Update { id, .. } => {
                if let Some(snapshot) = held.as_mut().filter(|snapshot| snapshot.id == *id) {
                    let latest = app.session.tool_options();
                    snapshot.latest = latest;
                }
            }
            EditEvent::Commit { id, .. } if held.as_ref().is_some_and(|snapshot| snapshot.id == *id) => held = None,
            EditEvent::Set { .. } => held = None,
            _ => {}
        }
    }
    ctx.data_mut(|data| {
        data.remove::<ShapeToolGesture>(key);
        if let Some(snapshot) = held {
            data.insert_temp(key, snapshot);
        }
    });
}

/// A click's change of the shape: the tool's options, and the selected live shapes in one step.
fn apply_shape(app: &mut VectorcraftApp, tool_kvs: &[(&str, Value)], live_patch: Value) {
    shape_edit(app, tool_kvs, live_patch, Live::Released);
}

/// Sets the tool's options and edits the selected live shapes in phase `phase` of a field's edit: a
/// scrub previews them, and its release makes the one step.
fn shape_edit(app: &mut VectorcraftApp, tool_kvs: &[(&str, Value)], live_patch: Value, phase: Live) {
    for (k, v) in tool_kvs {
        app.run("tool.setOption", json!({ "key": *k, "value": v.clone() })).ok();
    }
    if app.session.active().is_some_and(|st| !st.selection.is_empty()) {
        live_run(app, "Live Shape", "addon.shapes.configure", live_patch, phase);
    }
}

fn draw_arc_controls(app: &mut VectorcraftApp, ui: &mut Ui, start_deg: f64, end_deg: f64, mode: &str) {
    // A whole ellipse becomes a pie once an angle is typed.
    let next_mode = if mode == "whole" { "pie" } else { mode };
    let start = plain(tl!("Start:"), start_deg, Some(UnitKind::Degrees), 1, None, ui);
    number(app, ui, "arc-start", "Start:", start, |app, v, phase| {
        shape_edit(app, &[("arcStart", json!(v)), ("arcMode", json!(next_mode))], json!({ "startAngle": v, "arcMode": next_mode }), phase);
    });
    let end = plain(tl!("End:"), end_deg, Some(UnitKind::Degrees), 1, None, ui);
    number(app, ui, "arc-end", "End:", end, |app, v, phase| {
        shape_edit(app, &[("arcEnd", json!(v)), ("arcMode", json!(next_mode))], json!({ "endAngle": v, "arcMode": next_mode }), phase);
    });
    const MODES: [Choice; 4] =
        [("pie", "Slice", "target"), ("chord", "Closed", "circle-off"), ("arc", "Arc", "dc-arc"), ("whole", "Open", "dc-ellipse")];
    if let Some(m_id) = choice(ui, "Arc", mode, &MODES) {
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

fn draw_ellipse_controls(app: &mut VectorcraftApp, ui: &mut Ui, opts: &Value, selected_custom: Option<CustomShape>) {
    let (start_deg, end_deg, mode) = match selected_custom {
        Some(CustomShape::Ellipse { pie: (a0, a1), arc_mode, .. }) => (a0, a1, mode_str(arc_mode)),
        _ => (opts["arcStart"].as_f64().unwrap_or(0.0), opts["arcEnd"].as_f64().unwrap_or(0.0), opts["arcMode"].as_str().unwrap_or("whole")),
    };
    draw_arc_controls(app, ui, start_deg, end_deg, mode);
    if reset(ui) {
        apply_shape(app, &[("arcMode", json!("whole")), ("arcStart", json!(0.0)), ("arcEnd", json!(0.0))], json!({ "reset": true }));
    }
}

fn draw_rectangle_controls(app: &mut VectorcraftApp, ui: &mut Ui, opts: &Value, selected_custom: Option<CustomShape>) {
    let lengths = general_lengths(app);
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
    if let Some(next_sym) = link(ui, sym, IconId::new("unlink"), Some(IconId::new("link"))) {
        apply_shape(app, &[("cornerSymmetry", json!(next_sym))], if next_sym { json!({ "radius": radii[0] }) } else { json!({ "radii": radii }) });
    }
    if sym {
        number(app, ui, "rect-r", "Radius:", length("Radius:", radii[0], lengths), |app, v, phase| {
            let r = v.max(0.0);
            shape_edit(app, &[("cornerRadius", json!(r)), ("cornerRadii", json!([r, r, r, r]))], json!({ "radius": r }), phase);
        });
    } else {
        for (i, (key, caption)) in [("rect-tl", "TL"), ("rect-tr", "TR"), ("rect-br", "BR"), ("rect-bl", "BL")].into_iter().enumerate() {
            number(app, ui, key, caption, length(caption, radii[i], lengths), |app, v, phase| {
                let mut next = radii;
                next[i] = v.max(0.0);
                shape_edit(app, &[("cornerRadii", json!(next))], json!({ "radii": next }), phase);
            });
        }
    }
    divider(ui);
    draw_arc_controls(app, ui, start_deg, end_deg, mode);
    if reset(ui) {
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
}

fn draw_polygon_controls(app: &mut VectorcraftApp, ui: &mut Ui, opts: &Value, active_shape: &str, selected_custom: Option<CustomShape>) {
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
    let kinds: [Choice; 2] = [("polygon", "Polygon", icon_for_shape("polygon")), ("star", "Star", icon_for_shape("star"))];
    match choice(ui, "Shape", if is_star { "star" } else { "polygon" }, &kinds) {
        Some("star") => apply_shape(app, &[("polyIsStar", json!(true))], json!({ "isStar": true, "spokeRatio": spoke })),
        Some(_) => apply_shape(app, &[("polyIsStar", json!(false))], json!({ "isStar": false })),
        None => {}
    }
    let corners = if is_star { "Points:" } else { "Sides:" };
    number(app, ui, "poly-corners", corners, plain(corners, f64::from(count), None, 0, Some(3.0..=1000.0), ui), |app, v, phase| {
        let c = (v.round() as i64).clamp(3, 1000) as u32;
        shape_edit(app, &[("sides", json!(c)), ("points", json!(c))], json!({ "sides": c }), phase);
    });
    if is_star {
        number(app, ui, "poly-spoke", "Ratio", plain("Ratio", spoke, None, 2, Some(0.01..=1.0), ui), |app, v, phase| {
            let sr = v.clamp(0.01, 1.0);
            shape_edit(app, &[("starRatio", json!(sr))], json!({ "spokeRatio": sr }), phase);
        });
    }
    number(app, ui, "poly-round", "Round", plain("Round", rounded, None, 2, Some(-10.0..=10.0), ui), |app, v, phase| {
        let rnd = v.clamp(-10.0, 10.0);
        shape_edit(app, &[("rounded", json!(rnd))], json!({ "rounded": rnd }), phase);
    });
    number(app, ui, "poly-rand", "Random", plain("Random", randomized, None, 2, Some(-10.0..=10.0), ui), |app, v, phase| {
        let rand = v.clamp(-10.0, 10.0);
        shape_edit(app, &[("randomized", json!(rand))], json!({ "randomized": rand }), phase);
    });
    divider(ui);
    draw_arc_controls(app, ui, start_deg, end_deg, mode);
    if reset(ui) {
        apply_shape(
            app,
            &[("rounded", json!(0.0)), ("randomized", json!(0.0)), ("arcMode", json!("whole")), ("arcStart", json!(0.0)), ("arcEnd", json!(0.0))],
            json!({ "reset": true }),
        );
    }
}

fn draw_spiral_controls(app: &mut VectorcraftApp, ui: &mut Ui, opts: &Value, selected_custom: Option<CustomShape>) {
    let (mut turns, mut divergence, mut inner_r) =
        (opts["turns"].as_f64().unwrap_or(3.0), opts["divergence"].as_f64().unwrap_or(1.0), opts["innerRadius"].as_f64().unwrap_or(0.0));
    if let Some(CustomShape::Spiral { turns: t_v, divergence: div_v, inner_radius: ir_v, .. }) = selected_custom {
        turns = t_v;
        divergence = div_v;
        inner_r = ir_v;
    }
    number(app, ui, "spiral-turns", "Segments", plain("Segments", turns, None, 2, Some(0.01..=1024.0), ui), |app, v, phase| {
        let tv = v.clamp(0.01, 1024.0);
        shape_edit(app, &[("turns", json!(tv))], json!({ "turns": tv }), phase);
    });
    number(app, ui, "spiral-div", "Ratio", plain("Ratio", divergence, None, 2, Some(0.0..=1000.0), ui), |app, v, phase| {
        let dv = v.clamp(0.0, 1000.0);
        shape_edit(app, &[("divergence", json!(dv))], json!({ "divergence": dv }), phase);
    });
    number(app, ui, "spiral-inner", "Inner Radius:", plain("Inner Radius:", inner_r, None, 2, Some(0.0..=0.999), ui), |app, v, phase| {
        let iv = v.clamp(0.0, 0.999);
        shape_edit(app, &[("innerRadius", json!(iv))], json!({ "innerRadius": iv }), phase);
    });
    if reset(ui) {
        apply_shape(app, &[("turns", json!(3.0)), ("divergence", json!(1.0)), ("innerRadius", json!(0.0))], json!({ "reset": true }));
    }
}

fn draw_box3d_controls(app: &mut VectorcraftApp, ui: &mut Ui, opts: &Value, selected_custom: Option<CustomShape>) {
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
        number(app, ui, a_key, lbl, plain(lbl, a_val, Some(UnitKind::Degrees), 1, None, ui), |app, v, phase| {
            shape_edit(app, &[(a_key, json!(v))], json!({ a_key: v }), phase);
        });
        if let Some(next) = link(ui, p_val, IconId::new("dc-line"), None) {
            apply_shape(app, &[(p_key, json!(next))], json!({ p_key: next }));
        }
    }
    if reset(ui) {
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
}

/// The toolbar icon of the shape tool `id`, so the Control bar's shape buttons show the same icon.
fn icon_for_shape(id: &str) -> &'static str {
    vectorcraft_tools::tool_info(id).map_or("square-dashed", |t| crate::icons::tool_icon(t.icon))
}

fn mode_str(m: EllipseMode) -> &'static str {
    match m {
        EllipseMode::Slice => "pie",
        EllipseMode::Chord => "chord",
        EllipseMode::Arc => "arc",
        EllipseMode::Whole => "whole",
    }
}

#[cfg(test)]
mod cancel_tests {
    use super::*;
    use crate::tests::panel_harness::Panel;
    use craft::ui::contract::Role;
    use egui::{Key, Modifiers, vec2};

    fn window(app: &mut VectorcraftApp, ui: &mut Ui) {
        app.tick(ui.ctx());
        app.ui(ui);
    }

    fn application() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
        app.run("window.control", json!({})).unwrap();
        app
    }

    fn radius(panel: &Panel, node: vectorcraft_doc::NodeId) -> [f64; 4] {
        let app = panel.app();
        let shape = CustomShape::read_for_update(app.session.active().unwrap().doc.node(node).unwrap(), false).unwrap();
        let CustomShape::Rectangle { radii, .. } = shape else { panic!("the fixture is a live rectangle") };
        radii
    }

    #[test]
    fn a_canceled_radius_scrub_restores_future_tool_options_and_selected_geometry() {
        for tool in ["roundedRectangle", "box3d"] {
            for selected in [false, true] {
                for cancel in [false, true] {
                    let mut app = application();
                    let node = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 100, "height": 80, "radius": 3.5})).unwrap()["id"]
                        .as_u64()
                        .unwrap();
                    app.run("tool.select", json!({"tool": tool})).unwrap();
                    let values = if tool == "box3d" {
                        json!({"shape": "rectangle", "cornerSymmetry": true, "cornerRadius": 7.25, "cornerRadii": [7.25, 8.25, 9.25, 10.25]})
                    } else {
                        json!({"cornerRadius": 7.25})
                    };
                    app.run("tool.setOption", json!({"values": values})).unwrap();
                    if tool == "box3d" {
                        app.run("tool.setOption", json!({"key": "cornerRadii", "value": [7.25, 8.25, 9.25, 10.25]})).unwrap();
                        assert_eq!(app.session.tool_options()["cornerRadii"], json!([7.25, 8.25, 9.25, 10.25]));
                    }
                    app.run("select.set", json!({"ids": if selected { vec![node] } else { vec![] }})).unwrap();
                    let options = app.session.tool_options();
                    let original = app.session.active().unwrap().doc.clone();
                    let undo = app.session.active().unwrap().history.undo.len();
                    let mut panel = Panel::new(app, window);
                    panel.room(vec2(2200.0, 1000.0));
                    panel.frame();
                    panel.frame();
                    panel.drag_hold_role("Radius", Role::LABEL, vec2(12.0, 0.0));
                    let changed = panel.app().session.tool_options();
                    assert_ne!(changed, options, "the actual side-label scrub updates the future shape settings");
                    if selected {
                        assert_ne!(radius(&panel, vectorcraft_doc::NodeId(node)), [3.5; 4], "the selected shape previews the scrub");
                    }
                    if cancel {
                        panel.key(Key::Escape, Modifiers::NONE);
                        assert_eq!(
                            panel.app().session.tool_options(),
                            options,
                            "Escape restores the exact original tool settings, including coupled radii"
                        );
                        assert_eq!(panel.app().session.active().unwrap().doc, original);
                    }
                    panel.release();
                    assert_eq!(
                        panel.app().session.tool_options(),
                        if cancel { options.clone() } else { changed.clone() },
                        "release after Escape cannot reapply the scrub"
                    );
                    assert_eq!(panel.app().session.active().unwrap().history.undo.len(), undo + usize::from(selected && !cancel));
                    if selected && !cancel {
                        panel.app().run("edit.undo", json!({})).unwrap();
                    }
                    assert_eq!(panel.app().session.active().unwrap().doc, original);

                    // A normal subsequent canvas drag uses the settings that the field left behind.
                    let from = panel.app().canvas_view().unwrap().rect.min + vec2(120.0, 120.0);
                    panel.drag_between(from, from + vec2(100.0, 80.0), Modifiers::NONE);
                    let created = panel.app().session.active().unwrap().selection.objects[0];
                    assert_ne!(created.0, node, "the canvas drag creates new art rather than grabbing the selected shape's widgets");
                    let expected = if cancel { &options } else { &changed };
                    let radii = if tool == "box3d" && expected["cornerSymmetry"].as_bool() == Some(false) {
                        std::array::from_fn(|i| expected["cornerRadii"][i].as_f64().unwrap())
                    } else {
                        [expected["cornerRadius"].as_f64().unwrap(); 4]
                    };
                    assert_eq!(radius(&panel, created), radii, "the next shape uses the accepted or restored defaults");
                }
            }
        }
    }

    #[test]
    fn a_retired_shape_scrub_cannot_restore_options_into_the_new_tool() {
        let mut app = application();
        app.run("tool.select", json!({"tool": "roundedRectangle"})).unwrap();
        app.run("tool.setOption", json!({"key": "cornerRadius", "value": 7.25})).unwrap();
        let mut panel = Panel::new(app, window);
        panel.room(vec2(2200.0, 1000.0));
        panel.frame();
        panel.frame();
        panel.drag_hold_role("Radius", Role::LABEL, vec2(12.0, 0.0));
        assert_ne!(panel.app().session.tool_options()["cornerRadius"], json!(7.25));
        panel.app().run("tool.select", json!({"tool": "ellipse"})).unwrap();
        let before = panel.app().session.tool_options();
        let document = panel.app().session.active().unwrap().doc.clone();
        let undo = panel.app().session.active().unwrap().history.undo.len();
        panel.frame();
        panel.key(Key::Escape, Modifiers::NONE);
        panel.release();
        assert_eq!(panel.app().session.tool_id(), "ellipse");
        assert_eq!(panel.app().session.tool_options(), before, "retired Radius events cannot edit the replacement tool");
        assert_eq!(panel.app().session.active().unwrap().doc, document);
        assert_eq!(panel.app().session.active().unwrap().history.undo.len(), undo);
    }

    #[test]
    fn a_canceled_corner_edit_restores_a_scalar_changed_only_by_rollback() {
        let mut app = application();
        app.run("tool.select", json!({"tool": "box3d"})).unwrap();
        app.run("tool.setOption", json!({"values": {"shape": "rectangle", "cornerSymmetry": false}})).unwrap();
        app.run("tool.setOption", json!({"key": "cornerRadii", "value": [7.25, 8.25, 9.25, 10.25]})).unwrap();
        app.run("tool.setOption", json!({"key": "cornerRadius", "value": 4.125})).unwrap();
        let before = app.session.tool_options();
        let document = app.session.active().unwrap().doc.clone();
        let undo = app.session.active().unwrap().history.undo.len();
        let mut panel = Panel::new(app, window);
        panel.room(vec2(2200.0, 1000.0));
        panel.frame();
        panel.click_role("TL", Role::SPIN_BUTTON);
        panel.key(Key::A, Modifiers::COMMAND);
        panel.type_text("4.125");
        panel.frame();
        let proposed = panel.app().session.tool_options();
        assert_ne!(proposed["cornerRadii"], before["cornerRadii"]);
        assert_eq!(proposed["cornerRadius"], before["cornerRadius"], "the edit leaves the hidden scalar unchanged");
        panel.key(Key::Escape, Modifiers::NONE);
        panel.frame();
        assert_eq!(panel.app().session.tool_options(), before, "rollback must restore the scalar even when only restoring the array would change it");
        assert_eq!(panel.app().session.active().unwrap().doc, document);
        assert_eq!(panel.app().session.active().unwrap().history.undo.len(), undo);
    }

    #[test]
    fn canceled_individual_corner_scrubs_preserve_the_distinct_hidden_scalar_radius() {
        for label in ["TL", "TR"] {
            for cancel in [false, true] {
                let mut app = application();
                app.run("tool.select", json!({"tool": "box3d"})).unwrap();
                app.run("tool.setOption", json!({"values": {"shape": "rectangle", "cornerSymmetry": false}})).unwrap();
                app.run("tool.setOption", json!({"key": "cornerRadii", "value": [7.25, 8.25, 9.25, 10.25]})).unwrap();
                app.run("tool.setOption", json!({"key": "cornerRadius", "value": 4.125})).unwrap();
                let before = app.session.tool_options();
                assert_eq!(before["cornerRadius"], json!(4.125));
                assert_eq!(before["cornerRadii"], json!([7.25, 8.25, 9.25, 10.25]));
                let document = app.session.active().unwrap().doc.clone();
                let undo = app.session.active().unwrap().history.undo.len();
                let mut panel = Panel::new(app, window);
                panel.room(vec2(2200.0, 1000.0));
                panel.frame();
                panel.frame();
                panel.drag_hold_role(label, Role::LABEL, vec2(12.0, 0.0));
                let proposed = panel.app().session.tool_options();
                assert_ne!(proposed["cornerRadii"], before["cornerRadii"]);
                assert_ne!(proposed["cornerRadius"], before["cornerRadius"], "the array setter also changes the hidden scalar");
                if cancel {
                    panel.key(Key::Escape, Modifiers::NONE);
                    assert_eq!(panel.app().session.tool_options(), before, "both the array and its distinct scalar restore before release");
                }
                panel.release();
                assert_eq!(panel.app().session.tool_options(), if cancel { before } else { proposed });
                assert_eq!(panel.app().session.active().unwrap().doc, document);
                assert_eq!(panel.app().session.active().unwrap().history.undo.len(), undo);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use craft::behavior::IconId;
    use craft::ui::primitives::{has_icon, register_app_icons};

    use super::icon_for_shape;

    /// The Control bar's shape buttons look their icons up by tool id; an id that names no tool
    /// would show the placeholder instead of the toolbar's icon.
    #[test]
    fn every_shape_button_shows_its_tools_icon() {
        register_app_icons(&crate::icons::APP_ICONS);
        for id in ["box3d", "rectangle", "ellipse", "polygon", "star", "spiral"] {
            let tool = vectorcraft_tools::tool_info(id).unwrap_or_else(|| panic!("{id} is a tool"));
            assert_eq!(icon_for_shape(id), crate::icons::tool_icon(tool.icon), "{id}");
            assert!(has_icon(&IconId::new(icon_for_shape(id))), "{id}'s icon {} isn't an icon", icon_for_shape(id));
        }
        assert_eq!(icon_for_shape("no-such-tool"), "square-dashed");
    }
}
