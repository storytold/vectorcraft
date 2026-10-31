//! The W/H link (Constrain Width and Height Proportions) in the Transform panel, the Properties
//! panel's Transform section and the Control bar: one preference, passed on as `proportional`.
//! Alt+Enter in their fields applies the value to a copy of the selection, passed on as `copy`, and
//! Ctrl/Cmd+Enter in W or H keeps the proportions whether the link is on or off.

use egui::{Event, Key, Modifiers, PointerButton};
use serde_json::json;

use super::tests_appearance::{frame_raw, run};
use super::*;

/// What draws a set of size fields.
type Draw = fn(&mut VectorcraftApp, &mut Ui);

/// A selected 100 × 50 pt rectangle.
fn rect_app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    run(&mut app, "file.new", json!({"width": 400, "height": 400}));
    let id = run(&mut app, "shape.rectangle", json!({"x": 10, "y": 10, "width": 100, "height": 50}))["id"].clone();
    run(&mut app, "select.set", json!({ "ids": [id] }));
    app
}

fn size(app: &VectorcraftApp) -> (f64, f64) {
    let st = app.session.active().unwrap();
    let b = st.doc.bounds_of(&st.selection.objects, false).unwrap();
    (b.width().round(), b.height().round())
}

/// How a field's editing ends: Enter, Enter in the frame the text is typed in, or a press away from
/// the field, with these modifiers held.
#[derive(Clone, Copy)]
enum End {
    Enter(Modifiers),
    TypedEnter(Modifiers),
    ClickAway(Modifiers),
}

/// Type `value` into the field of what `draw` shows that shows `field` (the first one drawn), and
/// end its editing with `end`. Returns the context the frames ran in.
fn set_field(app: &mut VectorcraftApp, draw: Draw, field: &str, value: &str, end: End) -> egui::Context {
    set_nth_field(app, draw, (field, 0), value, end)
}

/// [`set_field`] for the field drawn `nth` (from 0) among those that show `field`.
fn set_nth_field(app: &mut VectorcraftApp, draw: Draw, (field, nth): (&str, usize), value: &str, end: End) -> egui::Context {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut time = 0.0;
    let mut frame = |app: &mut VectorcraftApp, events: Vec<Event>| {
        time += 1.0;
        frame_raw(&ctx, app, egui::RawInput { events, time: Some(time), ..Default::default() }, draw)
    };
    // A window shows nothing in its first frame, which only measures it.
    frame(app, vec![]);
    let shown = frame(app, vec![]);
    let at = shown.iter().filter(|(s, _)| s == field).nth(nth).unwrap_or_else(|| panic!("no {field} field {nth} in {shown:?}")).1.center();
    let button = |pos, pressed, modifiers| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers };
    frame(app, vec![Event::PointerMoved(at), button(at, true, Modifiers::NONE), button(at, false, Modifiers::NONE)]);
    // Focus selected the whole field: typing replaces it.
    frame(app, vec![]);
    let text = Event::Text(value.into());
    if !matches!(end, End::TypedEnter(_)) {
        frame(app, vec![text.clone()]);
    }
    // The modifiers go down first, as a keyboard reports them.
    match end {
        End::Enter(modifiers) => {
            let enter = Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers };
            frame(app, vec![Event::ModifiersChanged(modifiers), enter]);
        }
        End::TypedEnter(modifiers) => {
            let enter = Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers };
            frame(app, vec![text, Event::ModifiersChanged(modifiers), enter]);
        }
        End::ClickAway(modifiers) => {
            let away = at + egui::vec2(0.0, 300.0);
            let press = [button(away, true, modifiers), button(away, false, modifiers)];
            frame(app, [Event::ModifiersChanged(modifiers), Event::PointerMoved(away)].into_iter().chain(press).collect());
        }
    }
    ctx
}

/// Type `value` into the W field (the one showing "100 pt") of what `draw` shows, and press Enter.
fn set_width(app: &mut VectorcraftApp, draw: Draw, value: &str) {
    set_field(app, draw, "100 pt", value, End::Enter(Modifiers::NONE));
}

/// The paths of the document, bottom to top: their size and angle (rounded), and whether each is
/// selected.
fn paths(app: &VectorcraftApp) -> Vec<((f64, f64), f64, bool)> {
    let st = app.session.active().unwrap();
    let mut out = vec![];
    st.doc.walk(|n| {
        if matches!(n.kind, vectorcraft_doc::NodeKind::Path { .. }) {
            let b = st.doc.oriented_bounds(&[n.id], false).unwrap();
            out.push(((b.rect.width().round(), b.rect.height().round()), b.angle.round(), st.selection.objects.contains(&n.id)));
        }
    });
    out
}

#[test]
fn the_link_keeps_proportions_in_every_size_field() {
    let surfaces: [(&str, Draw); 3] =
        [("Transform panel", transform::show), ("Properties", properties::transform_section), ("Control bar", crate::chrome::control_bar)];
    for (name, draw) in surfaces {
        // Off: W alone.
        let mut app = rect_app();
        set_width(&mut app, draw, "200");
        assert_eq!(size(&app), (200.0, 50.0), "{name}, unlinked");
        // On: H follows.
        let mut app = rect_app();
        run(&mut app, "prefs.set", json!({"key": "constrainProportions", "value": true}));
        set_width(&mut app, draw, "200");
        assert_eq!(size(&app), (200.0, 100.0), "{name}, linked");
    }
}

/// Alt+Enter (Option+Return) applies a field's value to a copy of the selection and selects the
/// copy, in one undo step; a press elsewhere with Alt held applies it to the selection.
#[test]
fn alt_enter_applies_the_value_to_a_copy_in_every_size_field() {
    let surfaces: [(&str, Draw); 3] =
        [("Transform panel", transform::show), ("Properties", properties::transform_section), ("Control bar", crate::chrome::control_bar)];
    for (name, draw) in surfaces {
        let mut app = rect_app();
        set_field(&mut app, draw, "100 pt", "200", End::Enter(Modifiers::ALT));
        assert_eq!(paths(&app), [((100.0, 50.0), 0.0, false), ((200.0, 50.0), 0.0, true)], "{name}");
        run(&mut app, "edit.undo", json!({}));
        assert_eq!(paths(&app), [((100.0, 50.0), 0.0, true)], "{name}, undone");
        let mut app = rect_app();
        set_field(&mut app, draw, "100 pt", "200", End::ClickAway(Modifiers::ALT));
        assert_eq!(paths(&app), [((200.0, 50.0), 0.0, true)], "{name}, a press away");
    }
}

/// Alt+Enter in Rotate turns a copy to the angle and leaves the original, in the Transform panel
/// and the Properties panel; in the Transform panel, Alt+Enter in Shear shears a copy.
#[test]
fn alt_enter_in_rotate_and_shear_transforms_a_copy() {
    for (name, draw) in [("Transform panel", transform::show as Draw), ("Properties", properties::transform_section)] {
        let mut app = rect_app();
        // Rotate is the first field showing 0° (the Transform panel's Shear, after it, rests at 0° too).
        set_field(&mut app, draw, "0°", "45", End::Enter(Modifiers::ALT));
        assert_eq!(paths(&app), [((100.0, 50.0), 0.0, false), ((100.0, 50.0), 45.0, true)], "{name}");
    }
    let mut app = rect_app();
    // The second field showing 0° is Shear.
    set_nth_field(&mut app, transform::show, ("0°", 1), "30", End::Enter(Modifiers::ALT));
    assert_eq!(paths(&app), [((100.0, 50.0), 0.0, false), ((129.0, 50.0), 0.0, true)]);
}

/// With anchors selected with Direct Selection, the fields show those anchors' box, and Alt+Enter
/// applies the value to them as Enter does.
#[test]
fn alt_enter_moves_selected_anchors_without_a_copy() {
    let mut app = rect_app();
    let id = app.session.active().unwrap().selection.objects[0].0;
    run(&mut app, "select.anchors", json!({"id": id, "anchors": [[0, 1]]}));
    let before = paths(&app).len();
    // The anchor's X (the top-right corner of the rectangle at 10, 10).
    set_field(&mut app, transform::show, "110 pt", "150", End::Enter(Modifiers::ALT));
    assert_eq!(paths(&app).len(), before);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.bounds_of(&st.selection.objects, false).unwrap().width().round(), 140.0);
}

/// Ctrl/Cmd+Enter (Command+Return) in W or H applies the value and keeps the proportions with the
/// link off, and the field's editing ends as with Enter.
#[test]
fn ctrl_cmd_enter_in_a_size_field_scales_proportionally() {
    let surfaces: [(&str, Draw); 3] =
        [("Transform panel", transform::show), ("Properties", properties::transform_section), ("Control bar", crate::chrome::control_bar)];
    for (name, draw) in surfaces {
        for (field, value) in [("100 pt", "200"), ("50 pt", "100")] {
            let mut app = rect_app();
            let ctx = set_field(&mut app, draw, field, value, End::Enter(Modifiers::COMMAND));
            assert_eq!(size(&app), (200.0, 100.0), "{name}, {field}");
            assert_eq!(ctx.memory(|m| m.focused()), None, "{name}, {field}: the field's editing ended");
        }
        // The text typed in the frame of the Ctrl/Cmd+Enter is taken too.
        let mut app = rect_app();
        set_field(&mut app, draw, "100 pt", "300", End::TypedEnter(Modifiers::COMMAND));
        assert_eq!(size(&app), (300.0, 150.0), "{name}, typed with the Ctrl/Cmd+Enter");
    }
}

/// Ctrl/Cmd+Enter in X or Y does nothing: the field keeps the focus and the selection stays.
#[test]
fn ctrl_cmd_enter_in_a_position_field_keeps_editing() {
    for (name, draw) in [("Transform panel", transform::show as Draw), ("Properties", properties::transform_section)] {
        let mut app = rect_app();
        // X of the rectangle's center.
        let ctx = set_field(&mut app, draw, "60 pt", "200", End::Enter(Modifiers::COMMAND));
        assert_eq!(paths(&app), [((100.0, 50.0), 0.0, true)], "{name}");
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.bounds_of(&st.selection.objects, false).unwrap().x0, 10.0, "{name}");
        assert!(ctx.memory(|m| m.focused().is_some()), "{name}: the field keeps the focus");
    }
}

/// With the Artboard tool, Ctrl/Cmd+Enter in W keeps the active artboard's proportions.
#[test]
fn ctrl_cmd_enter_scales_the_artboard_proportionally() {
    let board = |end| {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        run(&mut app, "file.new", json!({"width": 200, "height": 150}));
        app.run("tool.select", json!({"tool": "artboard"})).unwrap();
        set_field(&mut app, transform::show, "200 pt", "400", end);
        let r = app.session.active().unwrap().doc.artboards[0].rect;
        (r.width().round(), r.height().round())
    };
    assert_eq!(board(End::Enter(Modifiers::NONE)), (400.0, 150.0));
    assert_eq!(board(End::Enter(Modifiers::COMMAND)), (400.0, 300.0));
}

/// A percentage typed alone in W or H (`25%`) is a share of the size the field shows, as in
/// Illustrator: with the link off only that side changes, with it on the other side follows, and
/// Ctrl/Cmd+Enter keeps the proportions as with a length.
#[test]
fn a_percentage_in_a_size_field_scales_the_size_it_shows() {
    let surfaces: [(&str, Draw); 3] =
        [("Transform panel", transform::show), ("Properties", properties::transform_section), ("Control bar", crate::chrome::control_bar)];
    for (name, draw) in surfaces {
        let mut app = rect_app();
        set_field(&mut app, draw, "100 pt", "25%", End::Enter(Modifiers::NONE));
        assert_eq!(size(&app), (25.0, 50.0), "{name}, W");
        let mut app = rect_app();
        set_field(&mut app, draw, "50 pt", "150 %", End::Enter(Modifiers::NONE));
        assert_eq!(size(&app), (100.0, 75.0), "{name}, H");
        let mut app = rect_app();
        set_field(&mut app, draw, "100 pt", "200%", End::Enter(Modifiers::COMMAND));
        assert_eq!(size(&app), (200.0, 100.0), "{name}, Ctrl/Cmd+Enter");
        let mut app = rect_app();
        run(&mut app, "prefs.set", json!({"key": "constrainProportions", "value": true}));
        set_field(&mut app, draw, "50 pt", "300%", End::Enter(Modifiers::NONE));
        assert_eq!(size(&app), (300.0, 150.0), "{name}, linked");
    }
}

/// A length field of the New Document dialog keeps the focus on Ctrl/Cmd+Enter, and the dialog
/// creates no document.
#[test]
fn ctrl_cmd_enter_in_a_dialog_field_keeps_editing() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.newDialog", json!({})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    let unit = d.fields.get("units").and_then(|u| u.as_str()).and_then(vectorcraft_doc::Unit::named).unwrap_or_default();
    let shown = unit.format(d.fields["width"].as_f64().unwrap());
    let dialogs: Draw = |app, ui| crate::dialogs::show(app, ui.ctx());
    let ctx = set_field(&mut app, dialogs, &shown, "300", End::Enter(Modifiers::COMMAND));
    assert!(app.ui.dialog.is_some() && app.session.documents().is_empty());
    assert!(ctx.memory(|m| m.focused().is_some()), "the field keeps the focus");
}

#[test]
fn the_link_is_a_preference_agents_can_read_and_set() {
    let mut app = rect_app();
    assert_eq!(run(&mut app, "prefs.get", json!({"key": "constrainProportions"})), json!(false));
    run(&mut app, "prefs.set", json!({"key": "constrainProportions", "value": true}));
    assert!(app.session.prefs.constrain_proportions);
    // What the fields send with the link on.
    run(&mut app, "object.setBounds", json!({"height": 100, "reference": 4, "proportional": true}));
    assert_eq!(size(&app), (200.0, 100.0));
}
