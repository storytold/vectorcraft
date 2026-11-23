//! Smooth Bridge in the interface: its dialog (the arcs' radius as a percentage of the smaller
//! circle's radius, or with Constant Gap that radius plus a gap; the reach, unavailable with two
//! circles selected; Keep Originals), which previews on the canvas and runs
//! `addon.pathtools.bridge` on OK.

use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamDialog, ParamKind};
use craft::ui::runtime::env::UnitKind;
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::addons::dialogs::{self, Spec};
use crate::canvas::rulers::general_lengths;

/// The command.
pub const COMMAND: &str = "addon.pathtools.bridge";
/// The radius the command takes (%), and its default.
const RADII: std::ops::RangeInclusive<f64> = 1.0..=1000.0;
const RADIUS: f64 = 100.0;
/// The gap the command takes (pt), and the dialog's first one.
const GAPS: std::ops::RangeInclusive<f64> = 0.0..=1000.0;
const GAP: f64 = 4.0;
/// The reach the command takes (pt), and its default.
const REACHES: std::ops::RangeInclusive<f64> = 0.0..=10000.0;
const REACH: f64 = 10.0;

/// How the dialog's values reach the command.
pub(crate) static SPEC: Spec = Spec { command: COMMAND, params, found: None };

/// The command's parameters from the dialog's fields: the radius, or with Constant Gap the gap.
fn params(fields: &DialogFields) -> Value {
    let (reach, keep) = (fields.number("reach").unwrap_or(REACH), fields.bool("keepOriginals").unwrap_or(false));
    if fields.bool("constantGap").unwrap_or(false) {
        json!({ "gap": fields.number("gap").unwrap_or(GAP), "reach": reach, "keepOriginals": keep })
    } else {
        json!({ "radius": fields.number("radius").unwrap_or(RADIUS), "reach": reach, "keepOriginals": keep })
    }
}

/// Opens Smooth Bridge's dialog on the command's defaults.
pub(crate) fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    // The reach applies to three or more circles; two are bridged at any distance. Counted once, before a preview
    // replaces the selection with its result.
    let many = app.session.doc().map(|doc| doc.selection.objects.len() > 2).unwrap_or(true);
    let fields = DialogFields::from_pairs([
        ("radius", FieldValue::Number(RADIUS)),
        ("constantGap", FieldValue::Bool(false)),
        ("gap", FieldValue::Number(GAP)),
        ("reach", FieldValue::Number(REACH)),
        ("keepOriginals", FieldValue::Bool(false)),
        ("many", FieldValue::Bool(many)),
    ]);
    let lengths = general_lengths(app);
    dialogs::open(app, &SPEC, fields, move |_| {
        let rows = vec![
            Param::new("constantGap", tl!("Constant Gap"), ParamKind::bool(false)),
            Param::number("gap", tl!("Gap:"), GAP, 1.0, 2)
                .length(lengths)
                .bounds(Some(*GAPS.start()), Some(*GAPS.end()))
                .visible_when("constantGap", FieldValue::Bool(true)),
            Param::number("radius", tl!("Radius:"), RADIUS, 1.0, 0)
                .unit(UnitKind::Percent)
                .bounds(Some(*RADII.start()), Some(*RADII.end()))
                .visible_when("constantGap", FieldValue::Bool(false)),
            Param::number("reach", tl!("Reach:"), REACH, 1.0, 2)
                .length(lengths)
                .bounds(Some(*REACHES.start()), Some(*REACHES.end()))
                .enabled_when("many", FieldValue::Bool(true)),
            Param::new("keepOriginals", tl!("Keep Originals"), ParamKind::bool(false)),
        ];
        ParamDialog::new(COMMAND, tl!("Smooth Bridge"), rows).previews(true)
    })
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use vectorcraft_engine::Session;

    use crate::addons::dialogs::tests::{confirm, dialog_id, frame, read, set};
    use crate::{VectorcraftApp, menus};

    fn circle(app: &mut VectorcraftApp, x: f64, r: f64) -> u64 {
        app.run("shape.ellipse", json!({"x": x - r, "y": 100.0 - r, "width": 2.0 * r, "height": 2.0 * r})).unwrap()["id"].as_u64().unwrap()
    }

    #[test]
    fn constant_gap_shows_the_gap_and_bridges_many_circles_within_reach() {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        // Three circles 4 apart in a row and one far off.
        let ids: Vec<u64> = [100.0, 124.0, 148.0, 300.0].iter().map(|&x| circle(&mut app, x, 10.0)).collect();
        app.run("select.set", json!({ "ids": ids })).unwrap();
        menus::invoke(&mut app, super::COMMAND, Value::Null);
        let dialog = dialog_id(&app);
        set(&app, dialog, "constantGap", json!(true));
        // A preview leaves two objects selected (one shape and the far circle); Reach stays available for the
        // four circles the command still bridges.
        frame(&mut app);
        assert_eq!(app.session.doc().unwrap().selection.objects.len(), 2, "previewed");
        let (many, params) = read(&app, dialog, |fields| (fields.bool("many"), (super::SPEC.params)(fields)));
        assert_eq!(many, Some(true), "Reach available for four circles");
        assert_eq!(params, json!({"gap": 4.0, "reach": 10.0, "keepOriginals": false}), "the gap, not the radius");
        assert!(confirm(&app, dialog), "OK is on");
        frame(&mut app);
        let top: Vec<u64> = app.session.doc().unwrap().doc.layers[0].children().unwrap().iter().map(|n| n.id.0).collect();
        assert_eq!(top, [ids[2], ids[3]], "one shape in the topmost joined circle's place, and the far circle");
    }
}
