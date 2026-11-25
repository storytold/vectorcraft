//! Path Precision Tools in the interface: its submenu (Object › Plug-ins › Path Precision Tools),
//! which selects its tools, opens its panels and runs Split / Add Points at Crossings, Set Corners… ([`set_corners`]), Divide into
//! Dashes ([`dashes`]), Expand Dashes into Paths, Smooth Bridge ([`bridge`]), Convert to Parametric Shape, Tangent Circle… ([`tangent_circle`]), Add Bounding Rectangle…, Add Bounding Circle… and Add Inscribed Circle; the Anchor Inspector panel (the selected
//! anchor's position and handles as numbers, the add-on's anchor edits, the Corner Tool's values,
//! [`corner`], and the Shared Edge Merge Tool's, [`shared_edge`]); the Parametric Shape panel
//! ([`shape`]); and its tools' options dialog (a double-click on the tool). Everything acts through `addon.pathtools.*` and
//! `tool.setOption`.

pub mod bridge;
pub mod corner;
pub mod dashes;
pub mod set_corners;
pub mod shape;
pub mod shared_edge;
pub mod tangent_circle;

use craft::behavior::ColumnWidth;
use craft::menu::Node;
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _, Sizing, clicked};
use craft::ui::display::{Divider, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::{Grid, Row};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind};
use craft::ui::runtime::env::{Label, LengthUnits, UnitKind};
use craft::ui::text::NumberField;
use craft::ui::value_drag::SliderScale;
use egui::Ui;
use serde_json::{Map, Value, json};
use vectorcraft_geom::Point;
use vectorcraft_tools::addons::tangent_circle as tangent_circle_tool;
use vectorcraft_tools::addons::{anchor_eraser, extend_path, measure_inspect, parametric_shape, slide_anchor, trim};

use crate::canvas::rulers::general_lengths;
use crate::dialogs::host::open_held;
use crate::menus::{Sep, c, cp};
use crate::panels::empty_state;
use crate::{VectorcraftApp, fields};

/// The Anchor Inspector panel's id (`window.panel {panel: "anchorInspector"}`).
pub const ID: &str = "anchorInspector";
/// The tools' options dialog.
pub const KIND: &str = "pathToolOptions";
/// Common Tangents' command, whose menu item opens the command dialog with a Preview.
pub const COMMON_TANGENTS: &str = "addon.pathtools.commonTangents";
/// The kinds of tangent Common Tangents' `tangents` picks, and their labels.
pub const TANGENT_KINDS: &[(&str, &str)] = &[("All", "all"), ("External", "external"), ("Internal", "internal")];
/// Add Bounding Rectangle's and Add Bounding Circle's commands, whose menu items open the command
/// dialog (Offset, One per Object) with a Preview.
pub const BOUNDING_RECTANGLE: &str = "addon.pathtools.boundingRectangle";
pub const BOUNDING_CIRCLE: &str = "addon.pathtools.boundingCircle";
/// Add Inscribed Circle's command, whose menu item runs it with no dialog.
pub const INSCRIBED_CIRCLE: &str = "addon.pathtools.inscribedCircle";
/// The close modes, as `addon.pathtools.close` names them, and their labels.
const CLOSE_MODES: [(&str, &str); 3] = [("handles", "Keep Handles"), ("straight", "Straight"), ("smooth", "Smooth")];
/// The Anchor Inspector's anchor edits: X and Y, and each handle's length and angle, label and field side by side.
const COLUMNS: [ColumnWidth; 4] = [ColumnWidth::Content, ColumnWidth::fill(1.0), ColumnWidth::Content, ColumnWidth::fill(1.0)];
/// The close buttons, one share each.
const CLOSE_COLUMNS: [ColumnWidth; 3] = [ColumnWidth::fill(1.0), ColumnWidth::fill(1.0), ColumnWidth::fill(1.0)];
/// The buttons that run an add-on command on the selection, as (label, command).
const ACTIONS: [(&str, &str); 4] = [
    ("Make Smooth", "addon.pathtools.smooth"),
    ("Make Corner", "addon.pathtools.corner"),
    ("Equalize Handles", "addon.pathtools.equalize"),
    ("Remove Points, Keep Shape", "addon.pathtools.removeKeepShape"),
];
/// The Extend Path tool's modes, as (value, label).
const EXTEND_MODES: [(&str, &str); 3] = [("straight", "Straight"), ("bezier", "Stretch Curve"), ("arc", "Arc")];

/// The Anchor Inspector panel's run-only state.
pub(crate) struct Inspector {
    /// How many points Add Points puts on each segment.
    pub points_per_segment: u32,
}

impl Default for Inspector {
    fn default() -> Self {
        Inspector { points_per_segment: 1 }
    }
}

/// The first selected anchor as the panel shows it: positions less the ruler origin, handles as length and angle.
pub(crate) struct AnchorReadout {
    pub x: f64,
    pub y: f64,
    /// The In and Out handles' lengths; 0 for a handle that is not there.
    pub lengths: [f64; 2],
    /// The In and Out handles' directions in degrees; none for a handle that is not there.
    pub angles: [Option<f64>; 2],
}

/// What the Anchor Inspector shows: how many anchors are selected and the first one.
pub(crate) struct Readout {
    pub count: usize,
    pub anchor: Option<AnchorReadout>,
}

/// The parameters of the add-on's Path Effects that pick one of some values, as (label, value).
pub(crate) fn effect_choices(effect: &str, key: &str) -> Option<&'static [(&'static str, &'static str)]> {
    use vectorcraft_effects::addons::path as fx;
    match (effect, key) {
        (fx::ADD_ANCHORS, "target") => Some(&[("All", "all"), ("Curved", "curved"), ("Straight", "straight")]),
        (fx::ADD_ANCHORS, "spacing") => Some(&[("By Count", "count"), ("By Distance", "distance")]),
        (fx::EXTEND, "startShape" | "endShape") => Some(&[("Straight", "straight"), ("Arc", "arc")]),
        (fx::CLOSE, "mode") => Some(&[("Straight", "straight"), ("Smooth", "smooth")]),
        (fx::RETRACT, "handles") => Some(&[("All", "all"), ("Shorter Than Length", "shorter")]),
        (fx::SPLIT, "anchors") => Some(&[("Corner", "corner"), ("Smooth", "smooth"), ("Both", "both")]),
        _ => None,
    }
}

/// Object › Plug-ins › Path Precision Tools.
pub fn menu() -> Vec<Node> {
    let tool = |label, id: &str| cp(label, "tool.select", json!({ "tool": id }));
    vec![
        tool("Anchor Eraser Brush", anchor_eraser::ID),
        tool("Extend Path Tool", extend_path::ID),
        tool("Measure & Inspect Tool", measure_inspect::ID),
        tool("Parametric Shape Tool", parametric_shape::ID),
        cp("Corner Tool", corner::COMMAND, json!({})),
        c("Set Corners…", set_corners::COMMAND),
        tool("Slide Anchor Along Path", slide_anchor::ID),
        cp("Shared Edge Merge Tool", shared_edge::COMMAND, json!({})),
        tool("Tangent Circle Tool", tangent_circle_tool::ID),
        tool("Trim at Crossings Tool", trim::ID),
        Sep,
        cp("Anchor Inspector", "window.panel", json!({ "panel": ID })),
        cp("Parametric Shape", "window.panel", json!({ "panel": shape::ID })),
        Sep,
        cp("Add Points at Crossings", "addon.pathtools.crossings", json!({})),
        cp("Split at Crossings", "addon.pathtools.crossings", json!({ "cut": true })),
        c("Divide into Dashes…", dashes::COMMAND),
        cp("Expand Dashes into Paths", "addon.pathtools.expandDashes", json!({})),
        c("Smooth Bridge…", bridge::COMMAND),
        cp("Convert to Parametric Shape", "addon.pathtools.shape.convert", json!({})),
        c("Tangent Circle…", tangent_circle::COMMAND),
        cp("Common Tangents…", COMMON_TANGENTS, json!({ "tangents": "all", "centerline": false })),
        cp("Add Bounding Rectangle…", BOUNDING_RECTANGLE, json!({ "offset": 0, "each": false })),
        cp("Add Bounding Circle…", BOUNDING_CIRCLE, json!({ "offset": 0, "each": false })),
        cp("Add Inscribed Circle", INSCRIBED_CIRCLE, json!({})),
    ]
}

/// The first direct-selected anchor (path, subpath, anchor) and how many are selected.
fn first_anchor(app: &VectorcraftApp) -> Option<(vectorcraft_geom::Anchor, usize)> {
    let st = app.session.active()?;
    let count = st.selection.anchors.values().map(|a| a.len()).sum();
    let (id, set) = st.selection.anchors.iter().find(|(_, a)| !a.is_empty())?;
    let (si, ai) = set.first()?;
    let a = st.doc.node(*id)?.path_data()?.subpaths.get(*si)?.anchors.get(*ai)?;
    Some((*a, count))
}

/// A handle's (length, direction in degrees counter-clockwise on the page); `None` for none.
fn polar(p: Point, h: Point) -> Option<(f64, f64)> {
    let v = h - p;
    (v.hypot() > 1e-9).then(|| (v.hypot(), (-v.y).atan2(v.x).to_degrees() + 0.0))
}

/// What the Anchor Inspector shows for the selection.
pub(crate) fn readout(app: &VectorcraftApp) -> Readout {
    let Some((a, count)) = first_anchor(app) else { return Readout { count: 0, anchor: None } };
    let origin = app.ruler_origin();
    let handles = [a.h_in, a.h_out].map(|h| polar(a.p, h));
    let anchor = AnchorReadout {
        x: a.p.x - origin.x,
        y: a.p.y - origin.y,
        lengths: handles.map(|h| h.map_or(0.0, |h| h.0)),
        angles: handles.map(|h| h.map(|h| h.1)),
    };
    Readout { count, anchor: Some(anchor) }
}

fn run(app: &mut VectorcraftApp, id: &str, p: Value) {
    if let Err(e) = app.run(id, p) {
        app.status(e);
    }
}

/// What a number field changed this pass, if it did and left a number.
pub(super) fn typed(value: Option<&Option<f64>>) -> Option<f64> {
    value.copied().flatten()
}

/// A labelled row of a panel section: `field` draws the control and returns what it edited this pass.
pub(super) fn row<R>(ui: &mut Ui, label: &'static str, salt: &'static str, mut field: impl FnMut(&mut Ui) -> Option<R>) -> Option<R> {
    Field::new(tl!(label)).id_salt(salt).show(ui, |ui, _| field(ui)).inner.flatten()
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    if app.session.active().is_none() {
        empty_state(ui, "dc-anchor-smooth", tl!("No document"), tl!("Open a document to edit its anchor points."));
        return;
    }
    let readout = readout(app);
    let anchor = readout.anchor.as_ref();
    let selected = app.session.active().is_some_and(|st| !st.selection.is_empty());
    let lengths = general_lengths(app);
    let origin = app.ruler_origin();
    let mut points = app.addon_panels.inspector.points_per_segment;
    let mut set: Option<Value> = None;
    let mut action: Option<(&str, Value)> = None;
    match anchor {
        Some(_) => Text::new(crate::i18n::fmt(tl!("Anchors selected: {count}"), &[("count", &readout.count.to_string())])).show(ui),
        None => Text::new(tl!("Direct-select anchor points to edit them.")).show(ui),
    };
    let enabled = anchor.is_some();
    let length = |ui: &mut Ui, salt: &'static str, field: NumberField<'static>| typed(field.id_salt(salt).enabled(enabled).show(ui).value());
    let angle = |ui: &mut Ui, salt: &'static str, v: Option<f64>| {
        let field = NumberField::new(tl!("Angle"), Bind::given(v)).unit_kind(UnitKind::Degrees).fixed_decimals(1);
        typed(field.id_salt(salt).enabled(enabled).show(ui).value())
    };
    let _ = Grid::new(&COLUMNS).show(ui, |ui, _| {
        for (label, key, v, o) in [("X", "x", anchor.map(|a| a.x), origin.x), ("Y", "y", anchor.map(|a| a.y), origin.y)] {
            Text::new(tl!(label)).show(ui);
            if let Some(v) = length(ui, key, NumberField::length(tl!(label), Bind::given(v), lengths)) {
                set = Some(json!({ key: v + o }));
            }
        }
        for (label, side, i) in [("In", "in", 0), ("Out", "out", 1)] {
            Text::new(tl!(label)).show(ui);
            // A handle's length is a size: `50%` is half of it.
            let len = fields::size_field(ui, tl!("Length"), anchor.map(|a| a.lengths[i]), lengths);
            if let Some(v) = length(ui, if i == 0 { "in-length" } else { "out-length" }, len) {
                set = Some(json!({ format!("{side}Length"): v }));
            }
            Text::new("∠").show(ui);
            if let Some(v) = angle(ui, if i == 0 { "in-angle" } else { "out-angle" }, anchor.and_then(|a| a.angles[i])) {
                set = Some(json!({ format!("{side}Angle"): v }));
            }
        }
    });
    Divider::horizontal().show(ui);
    for (label, id) in ACTIONS {
        if clicked(&Button::new(tl!(label)).sizing(Sizing::FillWidth).enabled(selected).show(ui)) {
            action = Some((id, json!({})));
        }
    }
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        if clicked(&Button::new(tl!("Add Points")).enabled(selected).show(ui)) {
            action = Some(("addon.pathtools.addPoints", json!({ "count": f64::from(points) })));
        }
        let field = NumberField::new(tl!("Points per segment"), Bind::given(Some(f64::from(points)))).range(1.0..=100.0).fixed_decimals(0);
        if let Some(v) = typed(field.id_salt("points-per-segment").enabled(selected).show(ui).value()) {
            points = v as u32;
        }
    });
    // The label above and the three ways under it: beside the label they overflow a narrow dock.
    Text::new(tl!("Close Path:")).show(ui);
    let _ = Grid::new(&CLOSE_COLUMNS).show(ui, |ui, _| {
        for (mode, label) in CLOSE_MODES {
            if clicked(&Button::new(tl!(label)).enabled(selected).show(ui)) {
                action = Some(("addon.pathtools.close", json!({ "mode": mode })));
            }
        }
    });
    app.addon_panels.inspector.points_per_segment = points;
    if let Some(p) = set {
        run(app, "addon.pathtools.setAnchor", p);
    }
    if let Some((id, p)) = action {
        run(app, id, p);
    }
    Divider::horizontal().show(ui);
    corner::section(app, ui);
    Divider::horizontal().show(ui);
    shared_edge::section(app, ui);
}

// ---------- the tools' options ----------

/// Tools whose options the dialog edits.
pub fn has_options(tool: &str) -> bool {
    [anchor_eraser::ID, extend_path::ID, tangent_circle_tool::ID].contains(&tool)
}

/// The dialog's values from a tool's options object.
fn fields_of(options: &Value) -> DialogFields {
    let mut fields = DialogFields::new();
    for (key, value) in options.as_object().into_iter().flatten() {
        match value {
            Value::Bool(on) => fields.set(key.clone(), FieldValue::Bool(*on)),
            Value::Number(n) => {
                if let Some(n) = n.as_f64() {
                    fields.set(key.clone(), FieldValue::Number(n));
                }
            }
            Value::String(text) => fields.set(key.clone(), FieldValue::Text(text.clone())),
            _ => {}
        }
    }
    fields
}

/// The rows of `tool`'s options dialog.
fn options_params(tool: &str, lengths: LengthUnits) -> Vec<Param> {
    if tool == extend_path::ID {
        let modes = EXTEND_MODES.iter().map(|(value, label)| ((*value).to_owned(), tl!(*label).into())).collect();
        return vec![Param::new("mode", tl!("Mode"), ParamKind::choice(modes, Some("straight".into())))];
    }
    if tool == tangent_circle_tool::ID {
        let (lo, hi) = tangent_circle_tool::RADIUS_RANGE;
        let kind = ParamKind::number(Some(lo), Some(hi), tangent_circle_tool::RADIUS, 1.0, 2);
        let colors =
            tangent_circle_tool::COLORS.iter().zip(TANGENT_CIRCLE_COLORS).map(|((name, _), label)| ((*name).to_owned(), tl!(label).into())).collect();
        let (snap_lo, snap_hi) = tangent_circle_tool::SNAP_RANGE;
        let snap = ParamKind::number(Some(snap_lo), Some(snap_hi), tangent_circle_tool::SNAP_PX, 1.0, 2);
        return vec![
            Param::new("radius", tl!("Radius"), kind).length(lengths),
            Param::new("center", tl!("Circle Center"), ParamKind::bool(true)),
            Param::new("radii", tl!("Circle Radii Lines"), ParamKind::bool(true)),
            Param::new("color", tl!("Color"), ParamKind::choice(colors, Some("red".into()))),
            Param::new("snapTolerance", tl!("Snap Tolerance"), snap).unit(UnitKind::Pixels),
            Param::new("snapWithoutGuides", tl!("Snap When Smart Guides Are Off"), ParamKind::bool(true)),
        ];
    }
    let (size, tolerance) = (anchor_eraser::SIZE_RANGE, anchor_eraser::TOLERANCE_RANGE);
    vec![
        Param::new("size", tl!("Size"), ParamKind::range(size.0, size.1, anchor_eraser::SIZE, SliderScale::Linear)).unit(UnitKind::Pixels),
        Param::new("tolerance", tl!("Tolerance"), ParamKind::range(tolerance.0, tolerance.1, anchor_eraser::TOLERANCE, SliderScale::Linear)),
        Param::new("pressure", tl!("Pressure"), ParamKind::bool(false)),
    ]
}

/// The labels of the Tangent Circle tool's colors ([`tangent_circle_tool::COLORS`]), in its order.
const TANGENT_CIRCLE_COLORS: [&str; 6] = ["Red", "Blue", "Magenta", "Green", "Black", "Gray"];

/// Open the options of add-on tool `tool` with its current values. OK sets them; when the tool refuses one, the dialog
/// stays open with the reason.
pub fn open_options(app: &mut VectorcraftApp, tool: &str) -> Result<Value, String> {
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let label = vectorcraft_tools::tool_info(tool).map_or("", |t| t.label);
    let title = crate::i18n::fmt(tl!("{tool} Options"), &[("tool", tl!(label))]);
    let lengths = general_lengths(app);
    let fields = fields_of(&app.session.tool_options_of(tool));
    let tool = tool.to_owned();
    let shown = tool.clone();
    let dialog = HostedParamDialog::new(&ctx, KIND, fields, move |fields| {
        ParamDialog::new(KIND, Label::from(title.clone()), options_params(&shown, lengths)).values(Bind::bound(fields))
    });
    open_held(app, dialog, move |app, answer| {
        let values: Map<String, Value> = answer.fields.iter().filter_map(|(k, v)| Some((k.to_owned(), serde_json::to_value(v).ok()?))).collect();
        app.run("tool.setOption", json!({ "tool": tool, "values": values })).map(|_| ())
    })
    .ok_or("no window")?;
    Ok(json!({ "dialog": KIND }))
}

#[cfg(test)]
mod tests;
