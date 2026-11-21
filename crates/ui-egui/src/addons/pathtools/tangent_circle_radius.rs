//! The Tangent Circle Tool's radius dialog, which a click on the second segment opens when the
//! tool's Ask For Circle Radius option is on. It starts at the radius of the circle the click found,
//! and each radius previews on the canvas the circle of that radius touching the two segments
//! nearest the click ([`tangent_circle::radius_circle`]). A radius that gives no circle there shows
//! the tool's note and leaves OK off. OK draws the circle with `shape.circleThroughPoints` in one
//! undo step, and with Auto-Update Default on also sets the tool's `radius` to the one given; Cancel
//! draws nothing.

use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_tools::addons::tangent_circle;

use crate::VectorcraftApp;
use crate::dialogs::{DialogSpec, form};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::widgets;

/// The dialog's kind.
pub const KIND: &str = tangent_circle::RADIUS_DIALOG;
/// The command that draws the circle.
const DRAW: &str = "shape.circleThroughPoints";
const LABEL_W: f32 = 60.0;
const FIELD_W: f32 = 90.0;
/// What the tool found (a private field): the segments, the click and its circle.
const CLICK: &str = "__click";
/// The radius the circle was last looked for at, and the circle's parameters for [`DRAW`] (null:
/// none) (private fields).
const TRIED: &str = "__tried";
const CIRCLE: &str = "__circle";
/// The error for a radius outside the tool's range.
const RADIUS_RANGE: &str = "`radius` must be from 0.01 to 1000000 pt";

/// Open the dialog on what the tool found (its [`tangent_circle::RADIUS_DIALOG`] parameters).
pub(crate) fn open(app: &mut VectorcraftApp, p: &Value) {
    let radius = p.get("radius").cloned().unwrap_or(json!(tangent_circle::RADIUS));
    app.ui.dialog = Some(Dialog::new(KIND, json!({ "radius": radius, "preview": true, CLICK: p })));
}

pub(crate) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Tangent Circle").to_string(),
    body,
    confirm,
    preview: true,
    ok_enabled: Some(|_, d| circle_of(d).is_some_and(|c| c.is_ok_and(|c| c.is_some()))),
    min_width: 280.0,
    ..DialogSpec::FORM
};

/// Whether `r` is in the tool's radius range.
fn in_range(r: f64) -> bool {
    let (lo, hi) = tangent_circle::RADIUS_RANGE;
    (lo..=hi).contains(&r)
}

/// The radius typed, when it is in the tool's range or is the radius of the circle the click found
/// (which a click without Shift may find outside that range).
fn radius(d: &Dialog) -> Result<f64, String> {
    let found = d.fields.get(CLICK).and_then(|p| p.get("radius")).and_then(Value::as_f64);
    d.fields.get("radius").and_then(Value::as_f64).filter(|r| in_range(*r) || Some(*r) == found).ok_or_else(|| RADIUS_RANGE.to_string())
}

/// The circle last looked for, when it was looked for at the radius the dialog holds: its [`DRAW`]
/// parameters, or none.
fn circle_of(d: &Dialog) -> Option<Result<Option<Value>, String>> {
    let r = match radius(d) {
        Ok(r) => r,
        Err(e) => return Some(Err(e)),
    };
    (d.fields.get(TRIED).and_then(Value::as_f64) == Some(r)).then(|| Ok(d.fields.get(CIRCLE).filter(|c| !c.is_null()).cloned()))
}

/// The circle at the radius the dialog holds, looked for again only when the radius changed.
fn circle(d: &mut Dialog) -> Result<Option<Value>, String> {
    if let Some(found) = circle_of(d) {
        return found;
    }
    let r = radius(d)?;
    let found = d.fields.get(CLICK).and_then(|p| tangent_circle::radius_circle(p, r));
    d.fields.insert(TRIED.into(), json!(r));
    d.fields.insert(CIRCLE.into(), found.clone().unwrap_or(Value::Null));
    Ok(found)
}

fn body(app: &mut VectorcraftApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let unit = app.session.general_unit();
    // The field keeps a radius as typed, so one outside the tool's range shows [`RADIUS_RANGE`].
    widgets::label_row(ui, tl!("Radius:"), LABEL_W, |ui| {
        form::length_within(ui, d, "radius", unit, FIELD_W, None);
    });
    let note = match circle(d) {
        Ok(Some(points)) => {
            form::preview_without_check(app, d, "Tangent Circle", DRAW, points);
            return false;
        }
        Ok(None) => tl!(tangent_circle::NO_CIRCLE).to_string(),
        Err(e) => crate::i18n::msg(&e),
    };
    form::rollback_preview(app, d);
    ui.add_space(6.0);
    ui.label(egui::RichText::new(note).color(Tokens::get(ui.ctx()).error));
    false
}

/// OK: draw the circle (keeping its preview) and close; with Auto-Update Default on, the radius
/// becomes the tool's when it is in the tool's range. Without a circle the dialog stays open.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut d = d.clone();
    let r = radius(&d)?;
    let points = circle(&mut d)?.ok_or_else(|| tangent_circle::NO_CIRCLE.to_string())?;
    let auto = app.session.tool_options_of(tangent_circle::ID).get("autoUpdateDefault").and_then(Value::as_bool) == Some(true);
    form::commit_preview(app, DRAW, points)?;
    if auto && in_range(r) {
        app.run("tool.setOption", json!({ "tool": tangent_circle::ID, "key": "radius", "value": r }))?;
    }
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use vectorcraft_engine::{Session, ViewInfo};
    use vectorcraft_tools::addons::tangent_circle as tool;
    use vectorcraft_tools::{PointerEvent, PointerKind};

    use crate::VectorcraftApp;
    use crate::tests_labels::painted_text;

    fn count(app: &VectorcraftApp) -> usize {
        app.session.doc().unwrap().doc.layers[0].children().unwrap().len()
    }

    fn undo_steps(app: &VectorcraftApp) -> usize {
        app.session.doc().unwrap().history.undo.len()
    }

    /// A click with the active tool at (`x`, `y`), Shift held when `shift`, with what it asks the
    /// interface for done.
    fn click(app: &mut VectorcraftApp, x: f64, y: f64, shift: bool) {
        for kind in [PointerKind::Down, PointerKind::Up] {
            let mut ev = PointerEvent::new(kind, x, y);
            ev.mods.shift = shift;
            let r = app.session.pointer(&ev, ViewInfo::default());
            crate::canvas::apply_requests(app, r);
        }
    }

    /// A frame of the open dialog: the text it shows.
    fn frame(app: &mut VectorcraftApp) -> String {
        painted_text(app, |app, ui| crate::dialogs::show(app, ui.ctx()))
    }

    fn set_radius(app: &mut VectorcraftApp, r: Value) {
        app.ui.dialog.as_mut().unwrap().fields.insert("radius".into(), r);
    }

    fn ok_enabled(app: &VectorcraftApp) -> bool {
        super::SPEC.ok_enabled.is_some_and(|f| f(app, app.ui.dialog.as_ref().unwrap()))
    }

    /// Click the dialog's Radius field, select its text, type `text` over it and press Tab.
    fn type_radius(app: &mut VectorcraftApp, text: &str) {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::theme::install_fonts(&ctx);
        let mut time = 0.0;
        let mut run = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            time += 0.1;
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0));
            let input = egui::RawInput { events, time: Some(time), screen_rect: Some(screen), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| crate::dialogs::show(app, ui.ctx()));
            out.textures_delta.clear();
            out
        };
        run(app, vec![]);
        let out = run(app, vec![]);
        let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
        // The Radius field is the dialog's only text field.
        let fields: Vec<_> = nodes.iter().filter(|(_, n)| n.role() == egui::accesskit::Role::TextInput).filter_map(|(_, n)| n.bounds()).collect();
        let [b] = fields[..] else { panic!("one text field, not {fields:?}") };
        let at = egui::pos2((b.x0 + b.x1) as f32 * 0.5, (b.y0 + b.y1) as f32 * 0.5);
        for pressed in [true, false] {
            let button = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
            run(app, vec![egui::Event::PointerMoved(at), button]);
        }
        let all = egui::Modifiers { ctrl: true, command: true, ..Default::default() };
        let select_all = egui::Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: all };
        run(app, vec![select_all, egui::Event::Text(text.into())]);
        for pressed in [true, false] {
            run(app, vec![egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed, repeat: false, modifiers: Default::default() }]);
        }
        run(app, vec![]);
    }

    #[test]
    fn a_radius_typed_outside_the_tool_range_shows_the_error_and_leaves_ok_off() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 500, "height": 500})).unwrap();
        for y in [100, 140] {
            app.run("path.create", json!({"anchors": [{"x": 100, "y": y}, {"x": 300, "y": y}]})).unwrap();
        }
        app.session.select_tool(tool::ID, ViewInfo::default()).unwrap();
        app.run("tool.setOption", json!({"tool": tool::ID, "key": "askRadius", "value": true})).unwrap();
        let paths = count(&app);
        click(&mut app, 150.0, 100.0, false);
        click(&mut app, 200.0, 140.0, false);
        frame(&mut app);
        assert!(ok_enabled(&app));
        // A radius typed in the field is kept as typed, not moved into the tool's range.
        for (typed, kept) in [("0", 0.0), ("-20", -20.0), ("2000000", 2e6)] {
            type_radius(&mut app, typed);
            assert_eq!(app.ui.dialog.as_ref().unwrap().f64("radius", f64::NAN), kept, "{typed}");
            assert!(frame(&mut app).contains(super::RADIUS_RANGE), "{typed}");
            assert!(!ok_enabled(&app), "{typed}");
            assert_eq!(count(&app), paths, "{typed}");
        }
        // Typing the click's radius again previews its circle and turns OK on.
        type_radius(&mut app, "20");
        frame(&mut app);
        assert!(ok_enabled(&app));
        assert_eq!(count(&app), paths + 1);
    }

    #[test]
    fn the_radius_dialog_previews_says_when_there_is_none_cancels_and_draws_on_ok() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 500, "height": 500})).unwrap();
        for (a, b) in [((100, 100), (300, 100)), ((100, 140), (300, 140))] {
            app.run("path.create", json!({"anchors": [{"x": a.0, "y": a.1}, {"x": b.0, "y": b.1}]})).unwrap();
        }
        app.run("path.create", json!({"anchors": [{"x": 100, "y": 300}, {"x": 300, "y": 300}, {"x": 300, "y": 400}]})).unwrap();
        app.session.select_tool(tool::ID, ViewInfo::default()).unwrap();
        app.run("tool.setOption", json!({"tool": tool::ID, "key": "askRadius", "value": true})).unwrap();
        let (paths, steps) = (count(&app), undo_steps(&app));
        // A click on each line: the dialog at the click's circle (radius 20), previewed.
        click(&mut app, 150.0, 100.0, false);
        click(&mut app, 200.0, 140.0, false);
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(super::KIND));
        assert!((app.ui.dialog.as_ref().unwrap().f64("radius", 0.0) - 20.0).abs() < 1e-6);
        let shown = frame(&mut app);
        for s in ["Tangent Circle", "Radius:"] {
            assert!(shown.contains(s), "{s} in {shown}");
        }
        assert_eq!(count(&app), paths + 1, "previewed: {}", app.ui.status);
        assert!(ok_enabled(&app));
        // Lines 40 pt apart take no other radius: the note, nothing previewed, and OK off.
        set_radius(&mut app, json!(25));
        assert!(frame(&mut app).contains(tool::NO_CIRCLE));
        assert_eq!(count(&app), paths);
        assert!(!ok_enabled(&app));
        assert_eq!(crate::dialogs::confirm(&mut app), Err(tool::NO_CIRCLE.to_string()));
        assert!(app.ui.dialog.is_some() && count(&app) == paths);
        // A radius that is not a number in the tool's range (as a control request can set it) is an
        // error.
        for bad in [json!(0), json!(-20), json!(2e6), json!("x"), Value::Null] {
            set_radius(&mut app, bad.clone());
            assert!(frame(&mut app).contains(super::RADIUS_RANGE), "{bad}");
            assert!(!ok_enabled(&app), "{bad}");
            assert_eq!(crate::dialogs::confirm(&mut app), Err(super::RADIUS_RANGE.to_string()), "{bad}");
            assert_eq!(count(&app), paths, "{bad}");
        }
        // Back at 20, previewed again; Cancel draws nothing and leaves no undo step.
        set_radius(&mut app, json!(20));
        frame(&mut app);
        assert_eq!(count(&app), paths + 1);
        crate::dialogs::cancel(&mut app);
        assert!(app.ui.dialog.is_none());
        assert_eq!((count(&app), undo_steps(&app)), (paths, steps));
        // The first line stays picked: a click on the second asks again, and OK draws the circle in
        // one undo step; the tool's radius stays.
        click(&mut app, 200.0, 140.0, false);
        frame(&mut app);
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!((count(&app), undo_steps(&app)), (paths + 1, steps + 1));
        let b = app.session.doc().unwrap().doc.bounds_of(&[app.session.doc().unwrap().doc.layers[0].children().unwrap()[paths].id], false).unwrap();
        assert!((b.x0 - 130.0).abs() < 1e-3 && (b.y0 - 100.0).abs() < 1e-3 && (b.width() - 40.0).abs() < 1e-3, "{b:?}");
        assert_eq!(app.session.tool_options_of(tool::ID)["radius"], json!(tool::RADIUS));
        // Auto-Update Default: Shift in the L's corner asks at the tool's radius; Cancel keeps the
        // tool's radius, and OK with another radius draws that fillet and makes it the tool's.
        app.run("tool.setOption", json!({"tool": tool::ID, "key": "autoUpdateDefault", "value": true})).unwrap();
        click(&mut app, 150.0, 300.0, false);
        click(&mut app, 300.0, 350.0, true);
        assert_eq!(app.ui.dialog.as_ref().unwrap().f64("radius", 0.0), tool::RADIUS);
        set_radius(&mut app, json!(12));
        frame(&mut app);
        crate::dialogs::cancel(&mut app);
        assert_eq!(app.session.tool_options_of(tool::ID)["radius"], json!(tool::RADIUS));
        click(&mut app, 300.0, 350.0, true);
        set_radius(&mut app, json!(10));
        frame(&mut app);
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!((count(&app), undo_steps(&app)), (paths + 2, steps + 2));
        assert_eq!(app.session.tool_options_of(tool::ID)["radius"], json!(10.0));
    }

    #[test]
    fn a_click_found_radius_outside_the_tool_range_can_be_drawn() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 500, "height": 500})).unwrap();
        // Lines 2,400,000 pt apart: a click on each finds the circle of radius 1,200,000, past the
        // largest radius the tool takes.
        for y in [100, 2_400_100] {
            app.run("path.create", json!({"anchors": [{"x": 100, "y": y}, {"x": 300, "y": y}]})).unwrap();
        }
        app.session.select_tool(tool::ID, ViewInfo::default()).unwrap();
        for key in ["askRadius", "autoUpdateDefault"] {
            app.run("tool.setOption", json!({"tool": tool::ID, "key": key, "value": true})).unwrap();
        }
        let (paths, steps) = (count(&app), undo_steps(&app));
        click(&mut app, 150.0, 100.0, false);
        click(&mut app, 200.0, 2_400_100.0, false);
        assert!((app.ui.dialog.as_ref().unwrap().f64("radius", 0.0) - 1_200_000.0).abs() < 1e-3, "{}", app.ui.status);
        frame(&mut app);
        assert!(ok_enabled(&app));
        // OK draws it; the tool's radius stays, since the tool does not take that radius.
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!((count(&app), undo_steps(&app)), (paths + 1, steps + 1));
        assert_eq!(app.session.tool_options_of(tool::ID)["radius"], json!(tool::RADIUS));
    }
}
