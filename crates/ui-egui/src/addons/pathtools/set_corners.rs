//! Set Corners in the interface: its dialog (radius, corner kind, which anchors: all, the
//! odd-numbered or the even-numbered ones, and the range of corner angles to round), which
//! previews on the canvas and runs `addon.pathtools.cornerSet` on OK, one undo step. It opens on
//! the Corner Tool's radius and kind, every corner angle (0° to 180°).

use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamDialog, ParamKind};
use craft::ui::runtime::env::UnitKind;
use serde_json::{Value, json};
use vectorcraft_tools::addons::corner_tool::{ID, MAX_RADIUS, RADIUS};

use crate::VectorcraftApp;
use crate::addons::dialogs::{self, Spec};
use crate::canvas::rulers::general_lengths;

/// The command.
pub const COMMAND: &str = "addon.pathtools.cornerSet";
/// The corner kinds, as the command names them, and their labels.
const KINDS: [(&str, &str); 3] = [("round", "Round"), ("invertedRound", "Inverted Round"), ("chamfer", "Chamfer")];
/// Which anchors, as the command's `every` names them, and their labels.
const EVERY: [(&str, &str); 3] = [("all", "All Anchors"), ("odd", "Odd-Numbered Anchors"), ("even", "Even-Numbered Anchors")];
/// The corner angles the command takes (degrees).
const ANGLES: std::ops::RangeInclusive<f64> = 0.0..=180.0;

/// How the dialog's values reach the command.
pub(crate) static SPEC: Spec = Spec { command: COMMAND, params, found: None };

/// The command's parameters from the dialog's fields.
pub(crate) fn params(fields: &DialogFields) -> Value {
    json!({
        "radius": fields.number("radius").unwrap_or(RADIUS),
        "kind": fields.text("kind").unwrap_or("round"),
        "every": fields.text("every").unwrap_or("all"),
        "minAngle": fields.number("minAngle").unwrap_or(*ANGLES.start()),
        "maxAngle": fields.number("maxAngle").unwrap_or(*ANGLES.end()),
    })
}

/// The choice options of `list`, as `(id, label)`.
fn choice(list: &[(&str, &'static str)]) -> ParamKind {
    ParamKind::choice(list.iter().map(|&(id, label)| (id.to_string(), tl!(label).into())).collect(), list.first().map(|(id, _)| id.to_string()))
}

/// Opens Set Corners' dialog on the Corner Tool's radius and kind, every anchor and every corner angle.
pub(crate) fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let options = app.session.tool_options_of(ID);
    let radius = options["radius"].as_f64().unwrap_or(RADIUS);
    let kind = options["kind"].as_str().unwrap_or("round").to_string();
    let fields = DialogFields::from_pairs([
        ("radius", FieldValue::Number(radius)),
        ("kind", FieldValue::Text(kind)),
        ("every", FieldValue::Text("all".into())),
        ("minAngle", FieldValue::Number(*ANGLES.start())),
        ("maxAngle", FieldValue::Number(*ANGLES.end())),
    ]);
    let lengths = general_lengths(app);
    dialogs::open(app, &SPEC, fields, move |fields| {
        // Only the corners whose sides meet at an angle in this range: small for a sharp spike, near 180° for a
        // shallow bend. Each end is bounded by the other, so a typed end past the other clamps.
        let (min, max) = (fields.number("minAngle").unwrap_or(*ANGLES.start()), fields.number("maxAngle").unwrap_or(*ANGLES.end()));
        let rows = vec![
            Param::number("radius", tl!("Radius:"), RADIUS, 1.0, 2).length(lengths).bounds(Some(0.0), Some(MAX_RADIUS)),
            Param::new("kind", tl!("Corner:"), choice(&KINDS)),
            Param::new("every", tl!("Anchors:"), choice(&EVERY)),
            Param::number("minAngle", tl!("Angle:"), *ANGLES.start(), 1.0, 1).unit(UnitKind::Degrees).bounds(Some(*ANGLES.start()), Some(max)),
            Param::number("maxAngle", tl!("to"), *ANGLES.end(), 1.0, 1).unit(UnitKind::Degrees).bounds(Some(min), Some(*ANGLES.end())),
        ];
        ParamDialog::new(COMMAND, tl!("Set Corners"), rows).previews(true)
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_doc::{LiveCorners, NodeId};
    use vectorcraft_engine::Session;

    use crate::addons::dialogs::tests::{cancel, confirm, dialog_id, frame, set};
    use crate::{VectorcraftApp, menus};

    /// The anchor indices of `id`'s uncut outline with a corner drawn.
    fn cut(app: &VectorcraftApp, id: u64) -> Vec<usize> {
        let doc = &app.session.doc().unwrap().doc;
        let lc = LiveCorners::of(doc.node(NodeId(id)).unwrap()).unwrap();
        (0..lc.base.anchor_count()).filter(|k| lc.drawn_radius(*k) > 0.0).collect()
    }

    /// A window's app with a document and the closed path through `anchors` selected; the path's id.
    fn selected_path(anchors: serde_json::Value) -> (VectorcraftApp, u64) {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let id = app.run("path.create", json!({"anchors": anchors, "closed": true})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        (app, id)
    }

    #[test]
    fn set_corners_opens_from_the_menu_previews_odd_anchors_and_commits_one_step() {
        let (mut app, id) =
            selected_path(json!([{"x": 0, "y": 0}, {"x": 100, "y": 0}, {"x": 100, "y": 100}, {"x": 0, "y": 100}, {"x": -50, "y": 50}]));
        menus::invoke(&mut app, super::COMMAND, json!({}));
        let dialog = dialog_id(&app);
        frame(&mut app);
        assert_eq!(cut(&app, id), vec![0, 1, 2, 3, 4], "all previewed from the start");
        let n = app.session.doc().unwrap().history.undo.len();
        set(&app, dialog, "every", json!("odd"));
        frame(&mut app);
        assert_eq!(cut(&app, id), vec![0, 2, 4], "odd previewed");
        cancel(&app, dialog);
        frame(&mut app);
        assert_eq!(cut(&app, id), Vec::<usize>::new(), "rolled back");
        menus::invoke(&mut app, super::COMMAND, json!({}));
        let dialog = dialog_id(&app);
        set(&app, dialog, "every", json!("even"));
        set(&app, dialog, "radius", json!(6));
        assert!(confirm(&app, dialog), "OK is on");
        frame(&mut app);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), n + 1);
        assert_eq!(cut(&app, id), vec![1, 3]);
    }

    #[test]
    fn set_corners_previews_only_the_corners_in_its_angle_range() {
        // A house: square corners at 0, 1 and 3, the roof's 135° corners at 2 and 4.
        let (mut app, id) =
            selected_path(json!([{"x": 0, "y": 0}, {"x": 100, "y": 0}, {"x": 100, "y": 100}, {"x": 50, "y": 150}, {"x": 0, "y": 100}]));
        menus::invoke(&mut app, super::COMMAND, json!({}));
        let dialog = dialog_id(&app);
        let n = app.session.doc().unwrap().history.undo.len();
        set(&app, dialog, "minAngle", json!(120));
        frame(&mut app);
        assert_eq!(cut(&app, id), vec![2, 4], "the roof previewed");
        // The ends bound each other, and a row refuses a value past its bound: the lower end goes first, then the
        // upper one above it.
        set(&app, dialog, "minAngle", json!(80));
        frame(&mut app);
        set(&app, dialog, "maxAngle", json!(100));
        frame(&mut app);
        assert_eq!(cut(&app, id), vec![0, 1, 3], "the square corners previewed");
        assert!(confirm(&app, dialog), "OK is on");
        frame(&mut app);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), n + 1);
        assert_eq!(cut(&app, id), vec![0, 1, 3]);
    }
}
