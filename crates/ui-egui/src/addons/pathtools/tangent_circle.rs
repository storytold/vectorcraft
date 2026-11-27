//! Tangent Circle in the interface: its dialog (the radius, with two shapes selected, and which
//! circle to draw, or all), which previews the circles on the canvas and runs
//! `addon.pathtools.tangentCircle` on OK.

use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind};
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::addons::dialogs::{self, Spec};
use crate::canvas::rulers::general_lengths;

/// The command.
pub const COMMAND: &str = "addon.pathtools.tangentCircle";
/// The radius the command takes (pt).
const RADII: std::ops::RangeInclusive<f64> = 0.01..=1e6;
/// The most circles the command lists.
const MAX_CIRCLES: u64 = 64;
/// The radius the dialog opens on (pt).
const RADIUS: f64 = 36.0;

/// How the dialog's values reach the command; the preview's answer says how many circles were found.
pub(crate) static SPEC: Spec = Spec { command: COMMAND, params, found: Some(found) };

/// The circle number the choice holds (an option id): 0 is every circle, `k` the k-th, counted from 1.
fn circle(fields: &DialogFields) -> u64 {
    fields.text("circle").and_then(|id| id.parse().ok()).unwrap_or(0)
}

/// The command's parameters: the radius only with two shapes.
fn params(fields: &DialogFields) -> Value {
    let mut p = json!({ "circle": circle(fields) });
    if fields.bool("twoShapes").unwrap_or(false) {
        p["radius"] = json!(fields.number("radius").unwrap_or(RADIUS));
    }
    p
}

/// Takes the preview's answer into the dialog: the circles found, which the Circle choice lists. A preview that
/// failed found none, so the choice goes back to All.
fn found(dialog: &mut HostedParamDialog, answer: Result<&Value, &str>) {
    match answer {
        Ok(result) => dialog.set_field("found", FieldValue::Int(result["found"].as_i64().unwrap_or(0))),
        Err(_) => {
            dialog.set_field("found", FieldValue::Int(0));
            dialog.set_field("circle", FieldValue::Text("0".into()));
        }
    }
}

/// Opens Tangent Circle's dialog on a radius of 36 pt and every circle.
pub(crate) fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    // The preview selects the circles it draws, so the selection's size is counted once: two shapes take a radius,
    // three set it themselves.
    let two = app.session.active().is_some_and(|st| st.selection.objects.len() == 2);
    let fields = DialogFields::from_pairs([
        ("radius", FieldValue::Number(RADIUS)),
        ("circle", FieldValue::Text("0".into())),
        ("twoShapes", FieldValue::Bool(two)),
        ("found", FieldValue::Int(0)),
    ]);
    let lengths = general_lengths(app);
    dialogs::open(app, &SPEC, fields, move |fields| {
        // The command lists at most 64 circles; the choice never holds more.
        let listed = u64::try_from(fields.int("found").unwrap_or(0)).unwrap_or(0).min(MAX_CIRCLES).max(circle(fields).min(MAX_CIRCLES));
        let options =
            std::iter::once(("0".to_string(), tl!("All").into())).chain((1..=listed).map(|n| (n.to_string(), n.to_string().into()))).collect();
        let rows = vec![
            Param::number("radius", tl!("Radius:"), RADIUS, 1.0, 2)
                .length(lengths)
                .bounds(Some(*RADII.start()), Some(*RADII.end()))
                .enabled_when("twoShapes", FieldValue::Bool(true)),
            Param::new("circle", tl!("Circle:"), ParamKind::choice(options, Some("0".into()))),
        ];
        ParamDialog::new(COMMAND, tl!("Tangent Circle"), rows).previews(true)
    })
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use vectorcraft_engine::Session;

    use crate::addons::dialogs::tests::{cancel, confirm, ctx, dialog_id, frame, read, set};
    use crate::{VectorcraftApp, menus};

    fn circle(app: &mut VectorcraftApp, cx: f64, cy: f64, r: f64) -> u64 {
        app.run("shape.ellipse", json!({"x": cx - r, "y": cy - r, "width": 2.0 * r, "height": 2.0 * r})).unwrap()["id"].as_u64().unwrap()
    }

    #[test]
    fn tangent_circle_previews_picks_one_cancels_and_runs_on_ok() {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let a = circle(&mut app, 100.0, 100.0, 10.0);
        let b = circle(&mut app, 130.0, 100.0, 10.0);
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        menus::invoke(&mut app, super::COMMAND, Value::Null);
        let dialog = dialog_id(&app);
        let count = |app: &VectorcraftApp| app.session.doc().unwrap().doc.layers[0].children().unwrap().len();
        // Preview is on from the start: radius 36 gives eight circles, and the choice lists them.
        frame(&mut app);
        assert_eq!(count(&app), 10, "previewed on opening: {}", app.ui.status);
        assert_eq!(read(&app, dialog, |f| f.int("found")), Some(8));
        set(&app, dialog, "radius", json!(20));
        frame(&mut app);
        // Six circles of radius 20 touch both: all previewed.
        assert_eq!(count(&app), 8, "previewed: {}", app.ui.status);
        assert_eq!(read(&app, dialog, |f| f.int("found")), Some(6));
        set(&app, dialog, "circle", json!("2"));
        frame(&mut app);
        assert_eq!(count(&app), 3, "one previewed");
        // A pick past the circles found is refused: the choice holds only the circles it lists.
        let past = json!(u64::MAX.to_string());
        assert!(app.dialogs().unwrap().with(|h| h.set_json_field(&ctx(&app), dialog, "circle", past)).is_err());
        // Radius 2 cannot reach across the 10 pt gap: no circles are found, and the choice goes back to All.
        set(&app, dialog, "radius", json!(2));
        frame(&mut app);
        assert_eq!(
            read(&app, dialog, |f| (f.int("found"), f.text("circle").map(str::to_string))),
            (Some(0), Some("0".to_string())),
            "{}",
            app.ui.status
        );
        cancel(&app, dialog);
        frame(&mut app);
        assert_eq!(count(&app), 2, "rolled back");
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        menus::invoke(&mut app, super::COMMAND, Value::Null);
        let dialog = dialog_id(&app);
        let n = app.session.doc().unwrap().history.undo.len();
        set(&app, dialog, "radius", json!(5));
        assert!(confirm(&app, dialog), "OK is on");
        frame(&mut app);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), n + 1);
        // Radius 5: the one circle in the gap.
        assert_eq!(count(&app), 3);
    }
}
