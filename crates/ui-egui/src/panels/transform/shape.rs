//! The Transform panel's live-shape rows: the Rectangle, Polygon, Ellipse and Line Properties, also shown by the
//! Properties panel and the Contextual Task Bar.

use std::ops::RangeInclusive;

use craft::behavior::Mixed;
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Bracket, Props as _, Show as _, ShowContainer as _};
use craft::ui::form::Field;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::pickers::Select;
use craft::ui::text::{NumberField, link_toggle};
use craft::ui::value_drag::Slider;
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_doc::{LiveShape, Node, NodeKind};
use vectorcraft_geom::shapes::CornerKind;

use crate::VectorcraftApp;
use crate::canvas::rulers::general_lengths;
use crate::edits::{EditPath, apply_edits};
use crate::panels::{corner_entries, corner_radius_field, corner_radius_row, corner_style};

/// What a number row counts, which sets its unit, decimals and range.
#[derive(Clone, Copy)]
enum Quantity {
    /// Counterclockwise degrees, one decimal.
    Degrees,
    /// A length in the document's general unit.
    Length,
}

/// The side counts Polygon Properties' Sides slider spans; the field beside it takes 3 to 1000.
const SIDES_SLIDER: RangeInclusive<f64> = 3.0..=20.0;

/// A live rectangle's, polygon's, ellipse's or line's properties (Transform and Properties panels, and for the
/// first three the Contextual Task Bar). `headed` adds the Rectangle and Polygon Properties headings; an ellipse's
/// and a line's heading always show. Returns false, drawing nothing, for other live shapes, and for a line unless
/// every selected object is one.
pub(crate) fn live_shape_rows(app: &mut VectorcraftApp, ui: &mut Ui, n: &Node, live: &LiveShape, headed: bool) -> bool {
    match live {
        LiveShape::Rectangle { .. } => under_heading(ui, headed.then(|| tl!("Rectangle Properties:")), |ui| {
            angle_row(app, ui, live, "rectangleAngle");
            corner_radius_row(app, ui, n);
        }),
        LiveShape::Polygon { .. } => under_heading(ui, headed.then(|| tl!("Polygon Properties:")), |ui| polygon_rows(app, ui, n, live)),
        LiveShape::Ellipse { pie, .. } => under_heading(ui, Some(tl!("Ellipse Properties:")), |ui| {
            angle_row(app, ui, live, "ellipseAngle");
            pie_rows(app, ui, *pie);
        }),
        LiveShape::Line { .. } => return line_rows(app, ui, live),
        _ => return false,
    }
    true
}

/// `rows` under a sub-heading, or bare when there is none.
fn under_heading(ui: &mut Ui, heading: Option<&'static str>, mut rows: impl FnMut(&mut Ui)) {
    match heading {
        Some(title) => {
            crate::panels::subheading(title).show(ui, |ui, _| rows(ui));
        }
        None => rows(ui),
    }
}

/// Runs `object.setLiveShape` with `params`, reporting a refusal in the status bar.
fn set_live_shape(app: &mut VectorcraftApp, params: Value) {
    if let Err(e) = app.run("object.setLiveShape", params) {
        app.status(e);
    }
}

/// One labelled number field whose edits send `params(number)` to `object.setLiveShape`; a scrub is one undo step.
fn number_row(app: &mut VectorcraftApp, ui: &mut Ui, label: &'static str, value: Option<f64>, quantity: Quantity, params: impl Fn(f64) -> Value) {
    let mut edit = None;
    Field::new(label).show(ui, |ui, _| {
        let bind = Bind::given(value);
        let field = match quantity {
            Quantity::Length => NumberField::length(label, bind, general_lengths(app)),
            Quantity::Degrees => NumberField::new(label, bind).unit("°").commit_decimals(1),
        };
        edit = Some(field.show(ui).edit);
    });
    if let Some(edit) = edit {
        // `Scope`, not `Preview`: `Preview` names one command, and each field sends its own params.
        apply_edits(app, &edit, EditPath::Scope, |app, v, _| {
            if let Some(v) = v {
                set_live_shape(app, params(*v));
            }
        });
    }
}

/// A live rectangle's, ellipse's or polygon's Angle row (counterclockwise): the angle the selected shapes of its
/// kind share, blank when they differ. `key` is the `object.setLiveShape` param an edit sets, which turns each of
/// them about its own center.
fn angle_row(app: &mut VectorcraftApp, ui: &mut Ui, live: &LiveShape, key: &'static str) {
    let angle = shared_shape_angle(app, live);
    number_row(app, ui, tl!("Angle"), angle, Quantity::Degrees, |a| json!({ key: a }));
}

/// The angle ([`LiveShape::shape_angle`]) every selected live shape of `live`'s kind shares, `None` when they
/// differ. Measured once per (document uid, revision) and kind: the panels read it every frame, and Select All can
/// select hundreds of thousands of shapes.
fn shared_shape_angle(app: &mut VectorcraftApp, live: &LiveShape) -> Option<f64> {
    let st = app.session.active()?;
    let key = (st.uid, st.revision, std::mem::discriminant(live));
    if let Some((k, angle)) = &app.canvas.shape_angle
        && *k == key
    {
        return *angle;
    }
    let mut angles = st
        .selection
        .objects
        .iter()
        .filter_map(|id| st.doc.node(*id)?.live_shape())
        .filter(|l| std::mem::discriminant(*l) == key.2)
        .filter_map(LiveShape::shape_angle);
    // With no shape of its kind selected, the panel shows `live`'s own angle.
    let angle = match angles.next() {
        None => live.shape_angle(),
        Some(first) => angles
            .all(|a| {
                let d = (a - first).rem_euclid(360.0);
                d.min(360.0 - d) < 1e-9
            })
            .then_some(first),
    };
    app.canvas.shape_angle = Some((key, angle));
    angle
}

/// Whether every selected object is a live line (and one is selected).
fn only_lines_selected(app: &VectorcraftApp) -> bool {
    let Some(st) = app.session.active() else { return false };
    let line = |n: &Node| matches!(&n.kind, NodeKind::Path { live: Some(l), .. } if matches!(**l, LiveShape::Line { .. }));
    !st.selection.objects.is_empty() && st.selection.objects.iter().all(|o| st.doc.node(*o).is_some_and(line))
}

/// A live line's Line Properties, under its heading: its length, end to end, and its angle (counterclockwise from
/// 3 o'clock, from its first end to its second). A new length keeps each selected line's first end in place, and a
/// new angle turns each one about its midpoint. Returns false, drawing nothing, unless every selected object is a
/// live line.
fn line_rows(app: &mut VectorcraftApp, ui: &mut Ui, live: &LiveShape) -> bool {
    if !only_lines_selected(app) {
        return false;
    }
    under_heading(ui, Some(tl!("Line Properties:")), |ui| {
        number_row(app, ui, tl!("Line Length"), live.line_length(), Quantity::Length, |l| json!({ "lineLength": l }));
        number_row(app, ui, tl!("Line Angle"), Some(live.line_angle().unwrap_or(0.0)), Quantity::Degrees, |a| json!({ "lineAngle": a }));
    });
    true
}

/// An ellipse's Pie Start and End Angle (degrees, counterclockwise from 3 o'clock; 0 to 360 is the whole
/// ellipse), the Constrain Pie Angle link beside them, and Invert Pie, which shows the other part of it. With the
/// link on, a typed angle moves the other one by as much, so each selected ellipse keeps its span.
fn pie_rows(app: &mut VectorcraftApp, ui: &mut Ui, pie: (f64, f64)) {
    let constrain = app.ui.constrain_pie_angle;
    // The link sits halfway between the two fields, with no bracket drawn to them.
    Row::new().bracket(Bracket::Unlinked).show(ui, |ui, _| {
        Stack::new().show(ui, |ui, _| {
            for (label, key, v) in [(tl!("Pie Start Angle"), "pieStart", pie.0), (tl!("Pie End Angle"), "pieEnd", pie.1)] {
                number_row(app, ui, label, Some(v), Quantity::Degrees, |a| {
                    if constrain { json!({ key: a, "constrainPie": true }) } else { json!({ key: a }) }
                });
            }
        });
        link_toggle(tl!("Constrain Pie Angle"), Bind::bound(&mut app.ui.constrain_pie_angle)).show(ui);
    });
    if Button::new(tl!("Invert Pie")).show(ui).clicked() {
        set_live_shape(app, json!({"invertPie": true}));
    }
}

/// A live polygon's Polygon Properties: its side count (a field and a slider), angle (counterclockwise), corner
/// type and radius, radius (center to vertex) and side length, and Make Sides Equal, which an uneven scale enables.
fn polygon_rows(app: &mut VectorcraftApp, ui: &mut Ui, n: &Node, live: &LiveShape) {
    let LiveShape::Polygon { sides, .. } = live else { return };
    let radius = live.polygon_radius();
    let side = radius.map(|r| 2.0 * r * (std::f64::consts::PI / f64::from((*sides).max(3))).sin());
    sides_row(app, ui, *sides);
    angle_row(app, ui, live, "polygonAngle");
    corner_kind_row(app, ui, corner_style(app, n).1);
    Field::new(tl!("Corner Radius")).show(ui, |ui, _| corner_radius_field(app, ui, n));
    number_row(app, ui, tl!("Radius"), radius, Quantity::Length, |r| json!({ "polygonRadius": r }));
    number_row(app, ui, tl!("Side Length"), side, Quantity::Length, |l| json!({ "sideLength": l }));
    if Button::new(tl!("Make Sides Equal")).enabled(!live.polygon_sides_equal()).show(ui).clicked() {
        set_live_shape(app, json!({"makeSidesEqual": true}));
    }
}

/// Polygon Properties' Sides row for a polygon of `sides` sides: a field that takes 3 to 1000, and after it a slider
/// over [`SIDES_SLIDER`]. A typed count is one undo step. A slider drag previews the count and its release records
/// one undo step; Escape during the drag puts the count back. A click on the rail or an arrow key sets the count in
/// one undo step. A count above 20 puts the thumb at the rail's right end, and screen readers read the real count.
fn sides_row(app: &mut VectorcraftApp, ui: &mut Ui, sides: u32) {
    let label = tl!("Sides");
    let count = f64::from(sides);
    let (mut typed, mut slid) = (None, None);
    Field::new(label).show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            typed = Some(NumberField::new(label, Bind::given(Some(count))).range(3.0..=1000.0).fixed_decimals(0).id_salt("field").show(ui).edit);
            let (lo, hi) = (*SIDES_SLIDER.start(), *SIDES_SLIDER.end());
            slid = Some(
                Slider::new(Bind::given(count.clamp(lo, hi)), SIDES_SLIDER)
                    .step(1.0)
                    .short(true)
                    .value_text(sides.to_string())
                    .label(label)
                    .id_salt("slider")
                    .show(ui),
            );
        });
    });
    if let Some(edit) = typed {
        apply_edits(app, &edit, EditPath::Scope, |app, v, _| {
            if let Some(v) = v {
                set_live_shape(app, json!({ "sides": *v as u64 }));
            }
        });
    }
    if let Some(edit) = slid {
        apply_edits(app, &edit, EditPath::Preview, |app, v, phase| {
            crate::panels::live_run(app, "Live Shape Properties", "object.setLiveShape", json!({ "sides": v.round() as u64 }), phase);
        });
    }
}

/// The polygon's Corner type select, blank while `kind` is `None` (the corners differ).
fn corner_kind_row(app: &mut VectorcraftApp, ui: &mut Ui, kind: Option<CornerKind>) {
    let entries = corner_entries();
    let mut edit = None;
    Field::new(tl!("Corner")).show(ui, |ui, _| {
        edit = Some(Select::new(tl!("Corner"), Bind::given(kind.map_or(Mixed::Mixed, Mixed::Same)), &entries).show(ui));
    });
    if let Some(edit) = edit {
        apply_edits(app, &edit, EditPath::Scope, |app, k, _| set_live_shape(app, json!({ "kind": k })));
    }
}
