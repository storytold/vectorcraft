//! Set Corners in the interface: its dialog (radius, corner kind, which anchors: all, the
//! odd-numbered or the even-numbered ones, and the range of corner angles to round), which
//! previews on the canvas and runs `addon.pathtools.cornerSet` on OK, one undo step. It opens on
//! the Corner Tool's radius and kind, every corner angle (0° to 180°).

use craft_ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamDialog, ParamKind};
use craft_ui::runtime::env::UnitKind;
use serde_json::{Value, json};
use vectorcraft_tools::addons::corner_tool::{ID, MAX_RADIUS, RADIUS};

use crate::VectorcraftApp;
use crate::addons::dialogs::{self, Confirm, Spec};
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
pub(crate) static SPEC: Spec = Spec { command: COMMAND, params, found: None, confirm: Confirm::Commit };

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
fn choice(list: &[(&str, &str)]) -> ParamKind {
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
    use serde_json::{Value, json};
    use vectorcraft_doc::{LiveCorners, NodeId};
    use vectorcraft_engine::Session;

    use crate::{VectorcraftApp, menus};

    // PLAN: assumed `app.ui_ctx: egui::Context` (lib.rs:203) and `OpenDialog.id` (host.rs:385)
    fn set(app: &VectorcraftApp, key: &str, value: Value) {
        let ctx = app.ui_ctx.clone();
        app.dialogs().expect("window host").with(|host| {
            let id = host.open_dialogs().first().map(|d| d.id).expect("a dialog is open");
            host.set_json_field(&ctx, id, key, value)
        })
        .unwrap();
    }

    fn pump(app: &mut VectorcraftApp) {
        let ctx = app.ui_ctx.clone();
        crate::dialogs::host::pump(app, &ctx);
    }

    /// Answers the open dialog with OK or Cancel, then runs the frame's pump.
    fn answer(app: &mut VectorcraftApp, ok: bool) {
        let ctx = app.ui_ctx.clone();
        app.dialogs().expect("window host").with(|host| {
            let id = host.open_dialogs().first().map(|d| d.id).expect("a dialog is open");
            if ok { host.confirm_by_id(&ctx, id) } else { host.cancel_by_id(&ctx, id) }
        });
        pump(app);
    }

    /// The anchor indices of `id`'s uncut outline with a corner drawn.
    fn cut(app: &VectorcraftApp, id: u64) -> Vec<usize> {
        let doc = &app.session.doc().unwrap().doc;
        let lc = LiveCorners::of(doc.node(NodeId(id)).unwrap()).unwrap();
        (0..lc.base.anchor_count()).filter(|k| lc.drawn_radius(*k) > 0.0).collect()
    }

    #[test]
    fn set_corners_opens_from_the_menu_previews_odd_anchors_and_commits_one_step() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let anchors = json!([{"x": 0, "y": 0}, {"x": 100, "y": 0}, {"x": 100, "y": 100}, {"x": 0, "y": 100}, {"x": -50, "y": 50}]);
        let id = app.run("path.create", json!({"anchors": anchors, "closed": true})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        menus::invoke(&mut app, super::COMMAND, json!({}));
        pump(&mut app);
        assert_eq!(cut(&app, id), vec![0, 1, 2, 3, 4], "all previewed from the start");
        let n = app.session.doc().unwrap().history.undo.len();
        set(&app, "every", json!("odd"));
        pump(&mut app);
        assert_eq!(cut(&app, id), vec![0, 2, 4], "odd previewed");
        answer(&mut app, false);
        assert_eq!(cut(&app, id), Vec::<usize>::new(), "rolled back");
        menus::invoke(&mut app, super::COMMAND, json!({}));
        set(&app, "every", json!("even"));
        set(&app, "radius", json!(6));
        answer(&mut app, true);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), n + 1);
        assert_eq!(cut(&app, id), vec![1, 3]);
    }

    #[test]
    fn set_corners_previews_only_the_corners_in_its_angle_range() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        // A house: square corners at 0, 1 and 3, the roof's 135° corners at 2 and 4.
        let anchors = json!([{"x": 0, "y": 0}, {"x": 100, "y": 0}, {"x": 100, "y": 100}, {"x": 50, "y": 150}, {"x": 0, "y": 100}]);
        let id = app.run("path.create", json!({"anchors": anchors, "closed": true})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        menus::invoke(&mut app, super::COMMAND, json!({}));
        let n = app.session.doc().unwrap().history.undo.len();
        set(&app, "minAngle", json!(120));
        pump(&mut app);
        assert_eq!(cut(&app, id), vec![2, 4], "the roof previewed");
        // The ends bound each other: the upper end goes first, then the lower one below it.
        set(&app, "maxAngle", json!(100));
        set(&app, "minAngle", json!(80));
        pump(&mut app);
        assert_eq!(cut(&app, id), vec![0, 1, 3], "the square corners previewed");
        answer(&mut app, true);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), n + 1);
        assert_eq!(cut(&app, id), vec![0, 1, 3]);
    }
}
