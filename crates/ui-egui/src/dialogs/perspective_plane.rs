//! Perspective plane options: double-clicking a plane widget of the perspective grid (or
//! `ui.perspectivePlane {plane}`) moves that plane to an exact location, alone, with the objects on
//! it, or with copies of them. OK runs `perspective.plane.move`.
//!
//! Fields: `plane` (`left`, `right` or `ground`), `location` (points along the plane's normal; 0
//! is its place in the grid's definition), `objects` (`none`, `move` or `copy`).

use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::LengthUnits;
use serde_json::{Value, json};
use vectorcraft_tools::distort::perspective::{PerspectiveGrid, Plane};

use super::form;
use super::host::open_held;
use crate::VectorcraftApp;
// PLAN: assumed `crate::canvas::general_lengths(app: &VectorcraftApp) -> LengthUnits` (cv.B5b)
use crate::canvas::general_lengths;

/// The dialog kind of the plane options.
pub const KIND: &str = "perspectivePlane";

/// What becomes of the objects on the plane: (`objects` value, label).
const OBJECTS: [(&str, &str); 3] = [("none", "Do Not Move"), ("move", "Move All Objects"), ("copy", "Copy All Objects")];

/// The dialog's title: the plane it moves.
fn title(plane: Plane) -> String {
    match plane {
        Plane::Right => tl!("Right Plane"),
        Plane::Ground => tl!("Horizontal Plane"),
        _ => tl!("Left Plane"),
    }
    .into()
}

/// The rows: the location (stored in points, shown in the general unit) and what becomes of the objects.
fn params(lengths: LengthUnits) -> Vec<Param> {
    let objects = OBJECTS.iter().map(|(v, l)| (v.to_string(), (*l).into())).collect();
    vec![
        Param::number("location", tl!("Location:"), 0.0, 1.0, 3).length(lengths),
        Param::new("objects", "", ParamKind::radio(objects, Some("none".into()))),
    ]
}

fn build<'a>(title: &str, lengths: LengthUnits, _fields: &'a mut DialogFields) -> ParamDialog<'a> {
    ParamDialog::new(KIND, title.to_string(), params(lengths))
}

/// Open the options of `plane` (`{plane}`; default the active plane), filled in with its location.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let doc = &app.session.active().ok_or("no document")?.doc;
    let g = PerspectiveGrid::current(doc);
    let plane = match p.get("plane").and_then(Value::as_str) {
        Some(s) => Plane::parse(s).ok_or("plane must be left, right or ground")?,
        None => g.plane,
    };
    if plane == Plane::None {
        return Err("no active perspective plane".into());
    }
    let offset = g.offset(plane);
    let lengths = general_lengths(app);
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let Value::Object(map) = json!({"plane": plane.id(), "location": offset, "objects": "none"}) else { return Err("no window".into()) };
    let heading = title(plane);
    let dialog = HostedParamDialog::new(&ctx, KIND, form::fields_of(&params(lengths), &map), move |f| build(&heading, lengths, f));
    // A refusal keeps the dialog open with the reason under its rows.
    let ok = move |app: &mut VectorcraftApp, r: &ParamResult| {
        let objects = r.fields.text("objects").unwrap_or("none");
        let p = json!({"plane": plane.id(), "offset": r.fields.number("location").unwrap_or(0.0), "objects": objects});
        app.run("perspective.plane.move", p).map(|_| ())
    };
    open_held(app, dialog, ok).ok_or("no window")?;
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft::ui::dialog::DialogHost;
    use vectorcraft_engine::Session;
    use vectorcraft_tools::{PointerEvent, PointerKind};

    fn app() -> VectorcraftApp {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
        app
    }

    fn host<R>(app: &VectorcraftApp, f: impl FnOnce(&mut DialogHost) -> R) -> R {
        app.dialogs().expect("a window").with(f)
    }

    /// The one open dialog: its id and its fields.
    fn the_dialog(app: &VectorcraftApp) -> (craft::ui::dialog::DialogId, Value) {
        let d = host(app, |h| h.open_dialogs().into_iter().next()).expect("a dialog is open");
        (d.id, d.fields.unwrap_or(Value::Null))
    }

    fn set(app: &VectorcraftApp, field: &str, value: Value) {
        let (ctx, (id, _)) = (app.ui_ctx.clone().unwrap(), the_dialog(app));
        host(app, |h| h.set_json_field(&ctx, id, field, value)).unwrap();
    }

    /// OK on the open dialog, then the frame's pump runs the held OK.
    fn ok(app: &mut VectorcraftApp) {
        let (ctx, (id, _)) = (app.ui_ctx.clone().unwrap(), the_dialog(app));
        // A held OK reports `false` and files its answer for the app; the dialog closes at its next pass.
        assert!(host(app, |h| h.confirm_by_id(&ctx, id) || h.held_state(id) == Some(craft::ui::dialog::HeldOk::Waiting)), "OK is enabled");
        crate::dialogs::host::pump(app, &ctx);
        crate::tests_synthetic::key_frame(app, &ctx, vec![]);
    }

    fn right_plane_with_art(app: &mut VectorcraftApp) {
        app.run("perspective.grid.preset", json!({"kind": 2})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 450, "y": 380, "width": 60, "height": 60})).unwrap()["id"].clone();
        app.run("perspective.attach", json!({"ids": [id], "plane": "right"})).unwrap();
    }

    #[test]
    fn plane_options_keep_the_plane_and_copied_objects_in_the_opening_document() {
        let mut app = app();
        right_plane_with_art(&mut app);
        let opening = app.session.doc().unwrap().doc.clone();
        app.run("ui.perspectivePlane", json!({"plane": "right"})).unwrap();
        set(&app, "location", json!(40.0));
        set(&app, "objects", json!("copy"));

        app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
        right_plane_with_art(&mut app);
        let other = app.session.doc().unwrap().doc.clone();
        let history = app.session.doc().unwrap().history.undo.len();
        let (id, _) = the_dialog(&app);
        ok(&mut app);

        assert_eq!(app.session.doc().unwrap().doc, other, "foreign OK leaves the other drawing and its art unchanged");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), history);
        assert!(matches!(host(&app, |h| h.held_state(id)), Some(craft::ui::dialog::HeldOk::Refused(_))));
        assert_eq!(the_dialog(&app).1["location"], 40.0);
        assert_eq!(the_dialog(&app).1["objects"], "copy");

        app.run("document.activate", json!({"index": 0})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc, opening);
        ok(&mut app);
        let doc = &app.session.doc().unwrap().doc;
        assert_eq!(PerspectiveGrid::current(doc).right_offset, 40.0);
        assert_eq!(doc.layers[0].children().unwrap().len(), 2, "one copy in the opening drawing");
    }

    #[test]
    fn replacement_plane_options_use_the_replacement_drawing_and_plane() {
        let mut app = app();
        app.run("perspective.grid.preset", json!({"kind": 2})).unwrap();
        let opening = app.session.doc().unwrap().doc.clone();
        app.run("ui.perspectivePlane", json!({"plane": "right"})).unwrap();

        app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
        app.run("perspective.grid.preset", json!({"kind": 2})).unwrap();
        app.run("ui.perspectivePlane", json!({"plane": "ground"})).unwrap();
        set(&app, "location", json!(40.0));
        ok(&mut app);

        let grid = PerspectiveGrid::current(&app.session.doc().unwrap().doc);
        assert_eq!(grid.offset(Plane::Ground), 40.0, "OK moves the replacement opening's plane");
        assert_eq!(grid.offset(Plane::Right), 0.0);
        app.run("document.activate", json!({"index": 0})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc, opening);
    }

    #[test]
    fn double_clicking_a_plane_widget_opens_its_options_and_ok_moves_it() {
        let mut app = app();
        app.run("perspective.grid.preset", json!({"kind": 2})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 450, "y": 380, "width": 60, "height": 60})).unwrap()["id"].clone();
        app.run("perspective.attach", json!({"ids": [id], "plane": "right"})).unwrap();
        let g = PerspectiveGrid::current(&app.session.active().unwrap().doc);
        let (_, w) = g.plane_widgets().into_iter().find(|(p, _)| *p == Plane::Right).unwrap();
        app.select_tool("perspectiveGrid");
        let view = app.view_info();
        crate::canvas::dispatch(&mut app, &PointerEvent::new(PointerKind::DoubleClick, w.x, w.y), view);
        let (_, fields) = the_dialog(&app);
        assert_eq!((fields["plane"].as_str(), fields["location"].as_f64()), (Some("right"), Some(0.0)));
        set(&app, "location", json!(40.0));
        set(&app, "objects", json!("copy"));
        ok(&mut app);
        let doc = &app.session.active().unwrap().doc;
        assert_eq!(PerspectiveGrid::current(doc).right_offset, 40.0);
        assert_eq!(doc.layers[0].children().unwrap().len(), 2, "copied");
        // The UI command opens it too; unknown planes are refused.
        host(&app, |h| h.open_dialogs().iter().map(|d| d.id).collect::<Vec<_>>())
            .into_iter()
            .for_each(|id| drop(host(&app, |h| h.take_by_id::<HostedParamDialog>(id))));
        app.run("ui.perspectivePlane", json!({"plane": "ground"})).unwrap();
        assert_eq!(the_dialog(&app).1["plane"], "ground");
        assert_eq!(title(Plane::Ground), "Horizontal Plane");
        assert!(app.run("ui.perspectivePlane", json!({"plane": "up"})).is_err());
    }

    #[test]
    fn a_click_to_size_rectangle_goes_on_the_active_plane() {
        let mut app = app();
        app.run("perspective.grid.preset", json!({"kind": 2})).unwrap();
        app.run("perspective.plane.set", json!({"plane": "right"})).unwrap();
        app.select_tool("rectangle");
        let view = app.view_info();
        for kind in [PointerKind::Down, PointerKind::Up] {
            crate::canvas::dispatch(&mut app, &PointerEvent::new(kind, 470.0, 300.0), view);
        }
        // The rectangle's click dialog is the one open dialog.
        ok(&mut app);
        assert_eq!(app.session.journal.last().unwrap().0, "perspective.draw");
        let st = app.session.active().unwrap();
        let id = *st.selection.objects.first().unwrap();
        assert_eq!(vectorcraft_tools::distort::perspective::attachment(st.doc.node(id).unwrap()).map(|a| a.0), Some(Plane::Right));
    }
}
