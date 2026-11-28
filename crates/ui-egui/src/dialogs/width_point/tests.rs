use super::*;
use craft::ui::dialog::DialogHost;
use vectorcraft_engine::Session;
use vectorcraft_tools::{PointerEvent, PointerKind};

/// A 10 pt line from (100, 150) to (300, 150) with a 4 + 4 pt width point in the middle.
fn app() -> (VectorcraftApp, u64) {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let id = app.run("shape.line", json!({"x1": 100, "y1": 150, "x2": 300, "y2": 150})).unwrap()["id"].as_u64().unwrap();
    app.run("stroke.set", json!({"ids": [id], "weight": 10})).unwrap();
    app.run("stroke.widthPoint.set", json!({"id": id, "t": 0.5, "left": 4, "right": 4})).unwrap();
    (app, id)
}

fn points(app: &VectorcraftApp, id: u64) -> Vec<(f64, f64, f64)> {
    let st = app.session.active().unwrap().doc.node(NodeId(id)).unwrap().appearance.stroke().unwrap().clone();
    let round = |v: f64| (v * 1e9).round() / 1e9;
    st.profile.map(|p| p.points.into_iter().map(|(t, l, r)| (round(t), round(l), round(r))).collect()).unwrap_or_default()
}

/// Runs `f` on the window's dialog host.
fn host<R>(app: &VectorcraftApp, f: impl FnOnce(&mut DialogHost) -> R) -> R {
    app.dialogs().expect("the app has a window").with(f)
}

/// The values of the open Width Point Edit, as automation lists them.
fn fields(app: &VectorcraftApp) -> Value {
    host(app, |host| host.get::<WidthPointDialog>().and_then(|dialog| dialog.json_fields())).expect("Width Point Edit is open")
}

/// Whether Width Point Edit is open.
fn is_open(app: &VectorcraftApp) -> bool {
    host(app, |host| host.is_open::<WidthPointDialog>())
}

/// Sets `field` of the open dialog as `ui.dialog.set` does.
fn try_set(app: &VectorcraftApp, field: &str, value: Value) -> Result<(), String> {
    let ctx = app.ui_ctx.clone().expect("the app has a window");
    host(app, |host| host.set_json_field(&ctx, dialog_id(KIND), field, value))
}

/// [`try_set`] for a value the dialog takes.
fn set(app: &VectorcraftApp, field: &str, value: Value) {
    try_set(app, field, value).unwrap();
}

/// Presses OK (`choice` `"ok"`) or Delete (`"delete"`) as `ui.dialog.confirm` does, and runs the app's pump,
/// which answers the held press. A press the app refused is the reason it gave; the dialog stays open then.
fn press(app: &mut VectorcraftApp, choice: &str) -> Result<(), String> {
    let ctx = app.ui_ctx.clone().expect("the app has a window");
    let id = dialog_id(KIND);
    set(app, "choice", json!(choice));
    host(app, |host| host.confirm_by_id(&ctx, id));
    crate::dialogs::host::pump(app, &ctx);
    match host(app, |host| host.held_state(id)) {
        Some(HeldOk::Accepted) => {
            host(app, |host| host.confirm_by_id(&ctx, id));
            Ok(())
        }
        Some(HeldOk::Refused(reason)) => Err(reason.as_str().to_owned()),
        _ => Ok(()),
    }
}

/// Presses OK.
fn ok(app: &mut VectorcraftApp) -> Result<(), String> {
    press(app, "ok")
}

/// Presses Delete.
fn delete(app: &mut VectorcraftApp) -> Result<(), String> {
    press(app, "delete")
}

/// A press the app is to refuse: Delete for `"delete"`, otherwise OK after setting the side width `field` to 5 pt.
fn press_field(app: &mut VectorcraftApp, field: &str) -> Result<(), String> {
    if field == "delete" {
        return delete(app);
    }
    set(app, field, json!(5));
    ok(app)
}

/// Presses Cancel.
fn cancel(app: &VectorcraftApp) {
    let ctx = app.ui_ctx.clone().expect("the app has a window");
    host(app, |host| host.cancel_by_id(&ctx, dialog_id(KIND)));
}

#[test]
fn double_click_opens_it_and_ok_sets_the_widths() {
    for shift in [false, true] {
        let (mut app, id) = app();
        let before = app.session.doc().unwrap().doc.clone();
        let undo = app.session.doc().unwrap().history.undo.len();
        let view = app.view_info();
        app.select_tool("width");
        let mut ev = PointerEvent::new(PointerKind::DoubleClick, 200.0, 150.0);
        ev.mods.shift = shift;
        crate::canvas::dispatch(&mut app, &ev, view);
        assert!(is_open(&app), "Width Point Edit opened");
        let d = fields(&app);
        assert_eq!((d["side1"].clone(), d["side2"].clone(), d["linked"].clone()), (json!(4.0), json!(4.0), json!(true)));
        assert_eq!(d["adjustAdjoining"], json!(shift));
        set(&app, "linked", json!(false));
        set(&app, "side1", json!(6));
        set(&app, "side2", json!(2));
        ok(&mut app).unwrap();
        assert!(!is_open(&app));
        let ends = if shift { (1.5, 0.5) } else { (1.0, 1.0) };
        assert_eq!(points(&app, id), vec![(0.0, ends.0, ends.1), (0.5, 1.2, 0.4), (1.0, ends.0, ends.1)]);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
        if shift {
            let edited = app.session.doc().unwrap().doc.clone();
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(app.session.doc().unwrap().doc, before);
            app.run("edit.redo", json!({})).unwrap();
            assert_eq!(app.session.doc().unwrap().doc, edited);
            crate::canvas::dispatch(&mut app, &ev, view);
            set(&app, "side1", json!(20));
            cancel(&app);
            assert!(!is_open(&app));
            let st = app.session.doc().unwrap();
            assert_eq!(st.doc, edited);
            assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo + 1, 0));
        }
    }
}

#[test]
fn delete_removes_the_point() {
    let (mut app, id) = app();
    app.run("ui.widthPointEdit", json!({"id": id, "index": 1})).unwrap();
    delete(&mut app).unwrap();
    assert_eq!(points(&app, id).len(), 2);
    assert!(app.run("ui.widthPointEdit", json!({"id": id, "index": 9})).is_err());
}

fn creation_app() -> (VectorcraftApp, u64) {
    let (mut app, id) = app();
    app.run("stroke.set", json!({"ids": [id], "profile": "uniform"})).unwrap();
    app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 30, "height": 40})).unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.select_tool("width");
    (app, id)
}

fn creation_history(app: &VectorcraftApp) -> (usize, usize, bool) {
    let st = app.session.doc().unwrap();
    (st.history.undo.len(), st.history.redo.len(), st.is_dirty())
}

/// The actual tool-event sequence from two stationary canvas clicks. No profile or dialog
/// is installed by the fixture after either click; DoubleClick must open the pending point.
fn creation_double_click(app: &mut VectorcraftApp, x: f64, y: f64) {
    let before = app.session.doc().unwrap().doc.clone();
    let history = creation_history(app);
    let view = app.view_info();
    for kind in [PointerKind::Down, PointerKind::Up, PointerKind::Down, PointerKind::Up] {
        crate::canvas::dispatch(app, &PointerEvent::new(kind, x, y), view);
        assert!(!is_open(app));
        assert_eq!(app.session.doc().unwrap().doc, before, "click alone must not add a width point");
        assert_eq!(creation_history(app), history);
    }
    assert!(!app.session.in_interaction());
    crate::canvas::dispatch(app, &PointerEvent::new(PointerKind::DoubleClick, x, y), view);
    assert!(is_open(app), "double-clicking a stroked segment must open Width Point Edit");
    assert_eq!(app.session.doc().unwrap().doc, before, "opening creates no point yet");
    assert_eq!(creation_history(app), history);
    assert!(!app.session.in_interaction());
}

#[test]
fn new_width_point_double_click_creates_with_history() {
    let (mut app, id) = creation_app();
    app.session.doc_mut().unwrap().mark_saved();
    let before = app.session.doc().unwrap().doc.clone();
    let selection = app.session.doc().unwrap().selection.clone();
    let history = creation_history(&app);
    creation_double_click(&mut app, 200.0, 150.0);
    let d = fields(&app);
    assert_eq!(d["id"], json!(id));
    assert!(d["index"].is_null());
    assert!((d["t"].as_f64().unwrap() - 0.5).abs() < 1e-6);
    assert_eq!((d["side1"].clone(), d["side2"].clone(), d["linked"].clone()), (json!(5.0), json!(5.0), json!(true)));
    assert_eq!(d["adjustAdjoining"], json!(false));
    set(&app, "linked", json!(false));
    set(&app, "side1", json!(6));
    set(&app, "side2", json!(2));
    ok(&mut app).unwrap();
    assert!(!is_open(&app));
    assert_eq!(points(&app, id), vec![(0.0, 1.0, 1.0), (0.5, 1.2, 0.4), (1.0, 1.0, 1.0)]);
    let after = app.session.doc().unwrap().doc.clone();
    assert_eq!(after.node(NodeId(id)).unwrap().kind, before.node(NodeId(id)).unwrap().kind, "path geometry remains unchanged");
    before.walk(|node| {
        if node.id != NodeId(id) && matches!(node.kind, vectorcraft_engine::doc::NodeKind::Path { .. }) {
            assert_eq!(after.node(node.id), Some(node), "other artwork remains unchanged");
        }
    });
    assert_eq!(app.session.doc().unwrap().selection, selection);
    assert_eq!(creation_history(&app), (history.0 + 1, 0, true));
    assert_eq!(app.session.doc().unwrap().history.undo.last().unwrap().label, "Width Point");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, before);
    assert_eq!(app.session.doc().unwrap().selection, selection);
    assert_eq!(creation_history(&app), (history.0, 1, false));
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, after);
    assert_eq!(app.session.doc().unwrap().selection, selection);
    assert_eq!(creation_history(&app), (history.0 + 1, 0, true));
}

#[test]
fn new_width_point_cancel_and_discard_keep_redo() {
    for discard in [false, true] {
        let (mut app, _) = creation_app();
        app.run("shape.rectangle", json!({"x": 50, "y": 20, "width": 20, "height": 20})).unwrap();
        let redo = app.session.doc().unwrap().doc.clone();
        app.run("edit.undo", json!({})).unwrap();
        app.session.doc_mut().unwrap().mark_saved();
        let before = app.session.doc().unwrap().doc.clone();
        let selection = app.session.doc().unwrap().selection.clone();
        let history = creation_history(&app);
        assert_eq!(history.1, 1);
        creation_double_click(&mut app, 200.0, 150.0);
        set(&app, "side1", json!(20));
        if discard {
            delete(&mut app).unwrap();
        } else {
            cancel(&app);
        }
        assert!(!is_open(&app));
        assert_eq!(app.session.doc().unwrap().doc, before);
        assert_eq!(app.session.doc().unwrap().selection, selection);
        assert_eq!(creation_history(&app), history);
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc, redo);
    }
}

#[test]
fn new_width_point_profile_and_compound_target() {
    for compound in [false, true] {
        let (mut app, line) = creation_app();
        let (id, member) = if compound {
            let second = app.run("shape.line", json!({"x1": 320, "y1": 50, "x2": 320, "y2": 250})).unwrap()["id"].as_u64().unwrap();
            app.run("select.set", json!({"ids": [line, second]})).unwrap();
            (app.run("object.compoundPath.make", json!({})).unwrap()["id"].as_u64().unwrap(), second)
        } else {
            (line, line)
        };
        app.run("stroke.widthProfile.set", json!({"ids": [id], "points": [[0, 1, 0.5], [1, 2, 1.5]]})).unwrap();
        app.select_tool("width");
        app.session.doc_mut().unwrap().mark_saved();
        let before = app.session.doc().unwrap().doc.clone();
        let selection = app.session.doc().unwrap().selection.clone();
        let history = creation_history(&app);
        // Make Compound Path can reverse the member. Choose the quarter point from its
        // stored direction; on a straight segment this is exactly one quarter of its length.
        let path = before.node(NodeId(member)).unwrap().path_data().unwrap();
        assert_eq!(path.subpaths.len(), 1);
        let subpath = &path.subpaths[0];
        assert_eq!(subpath.anchors.len(), 2);
        assert!(subpath.segment_is_line(0));
        let quarter = subpath.anchors[0].p.lerp(subpath.anchors[1].p, 0.25);
        creation_double_click(&mut app, quarter.x, quarter.y);
        let d = fields(&app);
        assert_eq!(d["id"], json!(id), "the compound owns its members' stroke");
        assert!(d["index"].is_null());
        let actual_t = d["t"].as_f64().unwrap();
        assert!((actual_t - 0.25).abs() < 1e-6, "compound={compound} pointer={quarter:?} actual_t={actual_t} want=0.25");
        // The two-point profile is linear: at a quarter its sides are 1.25 and 0.75 of
        // 1..2 and 0.5..1.5, on a 10pt stroke (5pt half).
        assert!((d["side1"].as_f64().unwrap() - 6.25).abs() < 1e-6);
        assert!((d["side2"].as_f64().unwrap() - 3.75).abs() < 1e-6);
        assert_eq!(d["linked"], json!(false));
        set(&app, "side1", json!(8));
        set(&app, "side2", json!(3));
        ok(&mut app).unwrap();
        assert_eq!(points(&app, id), vec![(0.0, 1.0, 0.5), (0.25, 1.6, 0.6), (1.0, 2.0, 1.5)]);
        let after = app.session.doc().unwrap().doc.clone();
        assert_eq!(after.node(NodeId(id)).unwrap().kind, before.node(NodeId(id)).unwrap().kind, "member paths retain their geometry");
        assert_eq!(app.session.doc().unwrap().selection, selection);
        assert_eq!(creation_history(&app), (history.0 + 1, 0, true));
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc, before);
        assert_eq!(creation_history(&app), (history.0, 1, false));
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc, after);
    }
}

#[test]
fn new_width_point_command_opens_pending_point() {
    let (mut app, id) = creation_app();
    let before = app.session.doc().unwrap().doc.clone();
    let history = creation_history(&app);
    app.run("ui.widthPointEdit", json!({"id": id, "t": 0.25})).expect("the UI command accepts a prospective point");
    let d = fields(&app);
    assert!(d["index"].is_null());
    assert_eq!((d["t"].clone(), d["side1"].clone(), d["side2"].clone()), (json!(0.25), json!(5.0), json!(5.0)));
    cancel(&app);
    assert_eq!(app.session.doc().unwrap().doc, before);
    assert_eq!(creation_history(&app), history);
}

#[test]
fn new_width_point_invalid_command_keeps_document() {
    let (mut app, id) = creation_app();
    let before = app.session.doc().unwrap().doc.clone();
    let history = creation_history(&app);
    for p in [
        json!({"id": id, "t": -0.01}),
        json!({"id": id, "t": 1.01}),
        json!({"id": id, "t": "invalid"}),
        json!({"id": id, "index": "invalid", "t": 0.5}),
        json!({"id": id, "index": 999, "t": 0.5}),
    ] {
        assert!(app.run("ui.widthPointEdit", p).is_err());
        assert!(!is_open(&app));
        assert_eq!(app.session.doc().unwrap().doc, before);
        assert_eq!(creation_history(&app), history);
    }
}

#[test]
fn new_width_point_checkbox_adjusts_adjoining_in_one_step() {
    for adjust in [true, false] {
        let (mut app, id) = creation_app();
        app.run("stroke.widthProfile.set", json!({"ids": [id], "points": [[0, 1, 1], [1, 1, 1]]})).unwrap();
        app.session.doc_mut().unwrap().mark_saved();
        let before = app.session.doc().unwrap().doc.clone();
        let selection = app.session.doc().unwrap().selection.clone();
        let history = creation_history(&app);
        creation_double_click(&mut app, 200.0, 150.0);
        assert!(fields(&app)["index"].is_null());
        // The pending point is linked, so Side 1 sets both sides.
        set(&app, "adjustAdjoining", json!(adjust));
        set(&app, "side1", json!("10 pt"));
        let d = fields(&app);
        assert_eq!(d["adjustAdjoining"], json!(adjust));
        assert_eq!((d["side1"].clone(), d["side2"].clone()), (json!(10.0), json!(10.0)));
        assert_eq!(app.session.doc().unwrap().doc, before);
        assert_eq!(creation_history(&app), history);
        ok(&mut app).unwrap();
        assert!(!is_open(&app), "OK confirms the pending point");
        let end = if adjust { 2.0 } else { 1.0 };
        assert_eq!(points(&app, id), vec![(0.0, end, end), (0.5, 2.0, 2.0), (1.0, end, end)], "adjoining widths follow Adjust Adjoining");
        let after = app.session.doc().unwrap().doc.clone();
        assert_eq!(after.node(NodeId(id)).unwrap().kind, before.node(NodeId(id)).unwrap().kind);
        before.walk(|node| {
            if node.id != NodeId(id) && matches!(node.kind, vectorcraft_engine::doc::NodeKind::Path { .. }) {
                assert_eq!(after.node(node.id), Some(node), "other artwork remains unchanged");
            }
        });
        assert_eq!(app.session.doc().unwrap().selection, selection);
        assert_eq!(creation_history(&app), (history.0 + 1, 0, true));
        assert_eq!(app.session.doc().unwrap().history.undo.last().unwrap().label, "Width Point");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc, before);
        assert_eq!(app.session.doc().unwrap().selection, selection);
        assert_eq!(creation_history(&app), (history.0, 1, false));
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc, after);
        assert_eq!(app.session.doc().unwrap().selection, selection);
        assert_eq!(creation_history(&app), (history.0 + 1, 0, true));
    }
}

/// [`app`]'s line with a discontinuous point in the middle: 4 + 4 pt before it, 2 + 6 pt after it.
fn pair_app() -> (VectorcraftApp, u64) {
    let (mut app, id) = app();
    app.run("stroke.widthProfile.set", json!({"ids": [id], "points": [[0, 1, 1], [0.5, 0.8, 0.8], [0.5, 0.4, 1.2], [1, 1, 1]]})).unwrap();
    app.session.doc_mut().unwrap().mark_saved();
    (app, id)
}

fn assert_pair_fields(d: &Value) {
    assert_eq!((d["index"].clone(), d["outIndex"].clone()), (json!(1), json!(2)));
    assert_eq!((d["side1"].clone(), d["side2"].clone(), d["linked"].clone()), (json!(4.0), json!(4.0), json!(true)));
    assert_eq!((d["outSide1"].clone(), d["outSide2"].clone(), d["outLinked"].clone()), (json!(2.0), json!(6.0), json!(false)));
    assert_eq!(d["single"], json!(""));
}

#[test]
fn either_half_of_a_discontinuous_point_shows_both_sets_of_side_widths() {
    for index in [1, 2] {
        let (mut app, id) = pair_app();
        app.run("ui.widthPointEdit", json!({"id": id, "index": index})).unwrap();
        assert_pair_fields(&fields(&app));
    }
    // Double-clicking the point with the Width tool opens the same two sets.
    let (mut app, _) = pair_app();
    let view = app.view_info();
    app.select_tool("width");
    crate::canvas::dispatch(&mut app, &PointerEvent::new(PointerKind::DoubleClick, 200.0, 150.0), view);
    assert!(is_open(&app), "Width Point Edit opened");
    assert_pair_fields(&fields(&app));
    // A continuous point keeps one set.
    let (mut app, id) = pair_app();
    app.run("ui.widthPointEdit", json!({"id": id, "index": 0})).unwrap();
    let d = fields(&app);
    assert!(d.get("outIndex").is_none() && d.get("outSide1").is_none());
    assert!(try_set(&app, "outSide1", json!(3)).is_err() && try_set(&app, "single", json!("incoming")).is_err());
}

/// Three width points each within 1e-6 of the next, but the outer two farther apart: the halves
/// are paired as `stroke.widthPoint.set` pairs them, from the incoming half's place, whichever
/// of them is opened. The first two make one discontinuous point, and the third stays a
/// continuous point.
#[test]
fn points_in_a_chain_pair_as_the_engine_pairs_them() {
    let chain = json!([[0, 1, 1], [0.5, 0.8, 0.8], [0.50000075, 0.4, 1.2], [0.5000015, 0.6, 0.6], [1, 1, 1]]);
    for index in [1, 2] {
        let (mut app, id) = app();
        app.run("stroke.widthProfile.set", json!({"ids": [id], "points": chain})).unwrap();
        app.run("ui.widthPointEdit", json!({"id": id, "index": index})).unwrap();
        let d = fields(&app);
        assert_eq!((d["index"].clone(), d["outIndex"].clone()), (json!(1), json!(2)), "opened {index}");
        set(&app, "outSide1", json!(1));
        ok(&mut app).unwrap();
        assert!(!is_open(&app), "opened {index}");
        assert_eq!(points(&app, id)[2], (0.50000075, 0.2, 1.2), "opened {index}");
    }
    let (mut app, id) = app();
    app.run("stroke.widthProfile.set", json!({"ids": [id], "points": chain})).unwrap();
    app.run("ui.widthPointEdit", json!({"id": id, "index": 3})).unwrap();
    let d = fields(&app);
    assert_eq!(d["index"], json!(3));
    assert!(d.get("outIndex").is_none());
    set(&app, "linked", json!(false));
    set(&app, "side1", json!(1));
    ok(&mut app).unwrap();
    assert!(!is_open(&app));
    assert_eq!(points(&app, id)[3], (0.5000015, 0.2, 0.6));
}

#[test]
fn ok_sets_both_halves_of_a_discontinuous_point_in_one_step() {
    let (mut app, id) = pair_app();
    let before = app.session.doc().unwrap().doc.clone();
    let history = creation_history(&app);
    // Cancel after edits changes nothing.
    app.run("ui.widthPointEdit", json!({"id": id, "index": 2})).unwrap();
    set(&app, "outSide1", json!(9));
    cancel(&app);
    assert_eq!(app.session.doc().unwrap().doc, before);
    assert_eq!(creation_history(&app), history);
    app.run("ui.widthPointEdit", json!({"id": id, "index": 2})).unwrap();
    // The incoming set is linked; unlink it so each side takes its own width.
    for (key, value) in [
        ("linked", json!(false)),
        ("outLinked", json!(false)),
        ("side1", json!(6)),
        ("side2", json!(2)),
        ("outSide1", json!(1)),
        ("outSide2", json!(3)),
    ] {
        set(&app, key, value);
    }
    ok(&mut app).unwrap();
    assert!(!is_open(&app));
    assert_eq!(points(&app, id), vec![(0.0, 1.0, 1.0), (0.5, 1.2, 0.4), (0.5, 0.2, 0.6), (1.0, 1.0, 1.0)]);
    assert_eq!(creation_history(&app), (history.0 + 1, 0, true));
    let after = app.session.doc().unwrap().doc.clone();
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, before);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, after);
    // OK with nothing changed records no undo step.
    app.run("ui.widthPointEdit", json!({"id": id, "index": 1})).unwrap();
    ok(&mut app).unwrap();
    assert_eq!(creation_history(&app), (history.0 + 1, 0, true));
}

#[test]
fn delete_removes_both_halves_of_a_discontinuous_point() {
    let (mut app, id) = pair_app();
    let before = app.session.doc().unwrap().doc.clone();
    let history = creation_history(&app);
    app.run("ui.widthPointEdit", json!({"id": id, "index": 1})).unwrap();
    delete(&mut app).unwrap();
    assert_eq!(points(&app, id), vec![(0.0, 1.0, 1.0), (1.0, 1.0, 1.0)]);
    assert_eq!(creation_history(&app).0, history.0 + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, before);
}

#[test]
fn single_width_only_keeps_the_chosen_half_as_one_point() {
    for (choice, kept) in [("Incoming", (0.8, 0.8)), ("Outgoing", (0.4, 1.2))] {
        let (mut app, id) = pair_app();
        let before = app.session.doc().unwrap().doc.clone();
        let history = creation_history(&app);
        app.run("ui.widthPointEdit", json!({"id": id, "index": 1})).unwrap();
        set(&app, "single", json!(choice.to_lowercase()));
        assert_eq!(fields(&app)["single"], json!(choice.to_lowercase()));
        ok(&mut app).unwrap();
        assert!(!is_open(&app));
        assert_eq!(points(&app, id), vec![(0.0, 1.0, 1.0), (0.5, kept.0, kept.1), (1.0, 1.0, 1.0)], "{choice}");
        assert_eq!(creation_history(&app), (history.0 + 1, 0, true));
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc, before);
    }
}

#[test]
fn a_compound_member_edits_both_halves_of_its_owners_point() {
    let (mut app, line) = app();
    let second = app.run("shape.line", json!({"x1": 320, "y1": 50, "x2": 320, "y2": 250})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [line, second]})).unwrap();
    let owner = app.run("object.compoundPath.make", json!({})).unwrap()["id"].as_u64().unwrap();
    app.run("stroke.widthProfile.set", json!({"ids": [owner], "points": [[0, 1, 1], [0.5, 0.8, 0.8], [0.5, 0.4, 1.2], [1, 1, 1]]})).unwrap();
    app.run("ui.widthPointEdit", json!({"id": second, "index": 2})).unwrap();
    assert_eq!(fields(&app)["id"], json!(owner));
    set(&app, "adjustAdjoining", json!(true));
    set(&app, "linked", json!(false));
    set(&app, "side1", json!(8));
    set(&app, "outSide2", json!(3));
    ok(&mut app).unwrap();
    // The incoming change (Side 1 doubled) doubles the start's Side 1; the outgoing one (Side 2
    // halved) halves the end's Side 2.
    assert_eq!(points(&app, owner), vec![(0.0, 2.0, 1.0), (0.5, 1.6, 0.8), (0.5, 0.4, 0.6), (1.0, 1.0, 0.5)]);
}

/// [`app`]'s line with width points at a quarter (4 + 2 pt), the middle (4 + 4 pt) and three
/// quarters (2 + 6 pt).
fn three_app() -> (VectorcraftApp, u64) {
    let (mut app, id) = app();
    let points = json!([[0, 1, 1], [0.25, 0.8, 0.4], [0.5, 0.8, 0.8], [0.75, 0.4, 1.2], [1, 1, 1]]);
    app.run("stroke.widthProfile.set", json!({"ids": [id], "points": points})).unwrap();
    app.session.doc_mut().unwrap().mark_saved();
    (app, id)
}

/// Open Width Point Edit on the points at a quarter and three quarters, set `fields` and press OK.
fn ok_on_two(fields: &[(&str, Value)]) -> (VectorcraftApp, u64) {
    let (mut app, id) = three_app();
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [3, 1, 3]})).unwrap();
    for (key, value) in fields {
        set(&app, key, value.clone());
    }
    ok(&mut app).unwrap();
    assert!(!is_open(&app));
    (app, id)
}

#[test]
fn several_points_show_blank_fields_where_they_differ() {
    let (mut app, id) = three_app();
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [3, 1, 3]})).unwrap();
    let d = fields(&app);
    assert_eq!(d["indices"], json!([1, 3]));
    for key in ["side1", "side2", "totalWidth"] {
        assert!(d[key].is_null(), "{key}: {:?}", d[key]);
    }
    assert_eq!(d["linked"], json!(false));
    assert!(d.get("outSide1").is_none());
    // Points with the same widths show them.
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [1, 2]})).unwrap();
    let d = fields(&app);
    assert_eq!((d["side1"].clone(), d["side2"].clone(), d["totalWidth"].clone()), (json!(4.0), Value::Null, Value::Null));
}

#[test]
fn a_side_width_sets_that_side_of_every_selected_point() {
    let (app, id) = ok_on_two(&[("side1", json!(5))]);
    assert_eq!(points(&app, id), vec![(0.0, 1.0, 1.0), (0.25, 1.0, 0.4), (0.5, 0.8, 0.8), (0.75, 1.0, 1.2), (1.0, 1.0, 1.0)]);
    let (app, id) = ok_on_two(&[("side2", json!("3 pt"))]);
    assert_eq!(points(&app, id), vec![(0.0, 1.0, 1.0), (0.25, 0.8, 0.6), (0.5, 0.8, 0.8), (0.75, 0.4, 0.6), (1.0, 1.0, 1.0)]);
    // With the link on, a side width sets both sides.
    let (app, id) = ok_on_two(&[("linked", json!(true)), ("side1", json!(3))]);
    assert_eq!(points(&app, id), vec![(0.0, 1.0, 1.0), (0.25, 0.6, 0.6), (0.5, 0.8, 0.8), (0.75, 0.6, 0.6), (1.0, 1.0, 1.0)]);
    // Total Width scales each point's own two sides.
    let (app, id) = ok_on_two(&[("totalWidth", json!(12))]);
    assert_eq!(points(&app, id), vec![(0.0, 1.0, 1.0), (0.25, 1.6, 0.8), (0.5, 0.8, 0.8), (0.75, 0.6, 1.8), (1.0, 1.0, 1.0)]);
}

#[test]
fn ok_on_several_points_is_one_undo_step() {
    let (mut app, id) = three_app();
    app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 20, "height": 20})).unwrap();
    app.run("edit.undo", json!({})).unwrap();
    let before = app.session.doc().unwrap().doc.clone();
    let history = creation_history(&app);
    // Cancel, and OK with nothing changed, change nothing and keep redo.
    for pressed in [false, true] {
        app.run("ui.widthPointEdit", json!({"id": id, "indices": [1, 3]})).unwrap();
        if pressed {
            ok(&mut app).unwrap();
        } else {
            set(&app, "side1", json!(9));
            cancel(&app);
        }
        assert!(!is_open(&app));
        assert_eq!(app.session.doc().unwrap().doc, before);
        assert_eq!(creation_history(&app), history);
    }
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [1, 3]})).unwrap();
    set(&app, "side1", json!(5));
    ok(&mut app).unwrap();
    assert_eq!(creation_history(&app), (history.0 + 1, 0, true));
    let after = app.session.doc().unwrap().doc.clone();
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, before);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, after);
}

#[test]
fn a_side_width_set_by_control_applies_even_when_it_is_the_value_shown() {
    // Side 1 is 4 pt at both points; with the link on, setting it to 4 pt sets Side 2 to 4 pt too.
    let (mut app, id) = three_app();
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [1, 2]})).unwrap();
    set(&app, "linked", json!(true));
    set(&app, "side1", json!(4));
    let d = fields(&app);
    assert_eq!((d["side1"].clone(), d["side2"].clone()), (json!(4.0), json!(4.0)));
    ok(&mut app).unwrap();
    assert_eq!(points(&app, id), vec![(0.0, 1.0, 1.0), (0.25, 0.8, 0.8), (0.5, 0.8, 0.8), (0.75, 0.4, 1.2), (1.0, 1.0, 1.0)]);
}

#[test]
fn adjoining_points_follow_every_changed_point_once() {
    // Side 1 doubles at a quarter and quadruples at three quarters: the start doubles, the end
    // quadruples and the middle, next to both, takes both.
    let (app, id) = ok_on_two(&[("adjustAdjoining", json!(true)), ("side1", json!(8))]);
    assert_eq!(points(&app, id), vec![(0.0, 2.0, 1.0), (0.25, 1.6, 0.4), (0.5, 6.4, 0.8), (0.75, 1.6, 1.2), (1.0, 4.0, 1.0)]);
}

#[test]
fn delete_removes_every_selected_point() {
    let (mut app, id) = three_app();
    let before = app.session.doc().unwrap().doc.clone();
    let history = creation_history(&app);
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [1, 3]})).unwrap();
    delete(&mut app).unwrap();
    assert_eq!(points(&app, id), vec![(0.0, 1.0, 1.0), (0.5, 0.8, 0.8), (1.0, 1.0, 1.0)]);
    assert_eq!(creation_history(&app).0, history.0 + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, before);
}

#[test]
fn several_points_with_a_discontinuous_one_edit_each_set() {
    let (mut app, id) = app();
    let profile = json!([[0, 1, 1], [0.25, 0.8, 0.4], [0.5, 0.8, 0.8], [0.5, 0.4, 1.2], [1, 1, 1]]);
    app.run("stroke.widthProfile.set", json!({"ids": [id], "points": profile})).unwrap();
    // One half of the discontinuous point brings the other.
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [1, 2]})).unwrap();
    let d = fields(&app);
    assert_eq!(d["indices"], json!([1, 2, 3]));
    // Incoming: the quarter (4 + 2) and the incoming half (4 + 4); outgoing: the quarter and the
    // outgoing half (2 + 6).
    assert_eq!((d["side1"].clone(), d["side2"].clone()), (json!(4.0), Value::Null));
    assert_eq!((d["outSide1"].clone(), d["outSide2"].clone()), (Value::Null, Value::Null));
    set(&app, "outSide2", json!(5));
    ok(&mut app).unwrap();
    assert_eq!(points(&app, id), vec![(0.0, 1.0, 1.0), (0.25, 0.8, 1.0), (0.5, 0.8, 0.8), (0.5, 0.4, 1.0), (1.0, 1.0, 1.0)]);
}

/// Three width points each within 1e-6 of the next, but the outer two farther apart: with several
/// points selected, the first two are the halves of one discontinuous point and the third is a
/// continuous point, as `stroke.widthPoint.set` pairs them.
#[test]
fn several_points_in_a_chain_pair_as_the_engine_pairs_them() {
    let (mut app, id) = app();
    let chain = json!([[0, 1, 1], [0.5, 0.8, 0.8], [0.50000075, 0.4, 1.2], [0.5000015, 0.6, 0.6], [1, 1, 1]]);
    app.run("stroke.widthProfile.set", json!({"ids": [id], "points": chain})).unwrap();
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [2, 3]})).unwrap();
    assert_eq!(fields(&app)["indices"], json!([1, 2, 3]));
    set(&app, "side1", json!(5));
    set(&app, "outSide2", json!(5));
    ok(&mut app).unwrap();
    assert!(!is_open(&app));
    // Incoming: the incoming half and the continuous point, both linked; outgoing: the outgoing
    // half and the continuous point.
    assert_eq!(points(&app, id), vec![(0.0, 1.0, 1.0), (0.5, 1.0, 1.0), (0.50000075, 0.4, 1.0), (0.5000015, 1.0, 1.0), (1.0, 1.0, 1.0)]);
}

/// Three width points within 1e-6 of one another are no discontinuous point: selected together,
/// the dialog edits all three, and a discontinuous point alone still opens the one-point dialog.
#[test]
fn several_points_at_nearly_one_place_edit_them_all() {
    let (mut app, id) = app();
    let profile = json!([[0, 1, 1], [0.5, 0.8, 0.8], [0.5000005, 0.4, 0.4], [0.5000009, 0.6, 0.6], [1, 1, 1]]);
    app.run("stroke.widthProfile.set", json!({"ids": [id], "points": profile})).unwrap();
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [1, 2, 3]})).unwrap();
    assert_eq!(fields(&app)["indices"], json!([1, 2, 3]));
    set(&app, "linked", json!(false));
    set(&app, "side2", json!(5));
    ok(&mut app).unwrap();
    assert!(!is_open(&app));
    assert_eq!(points(&app, id), vec![(0.0, 1.0, 1.0), (0.5, 0.8, 1.0), (0.5000005, 0.4, 1.0), (0.5000009, 0.6, 1.0), (1.0, 1.0, 1.0)]);
    let (mut app, id) = pair_app();
    let pair = app.session.active().unwrap().doc.node(NodeId(id)).unwrap().appearance.stroke().unwrap().profile.clone().unwrap();
    let a = (0..pair.points.len()).find(|&a| width_point_pair(&pair.points, a).is_some()).unwrap();
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [a + 1, a]})).unwrap();
    let d = fields(&app);
    assert_eq!((d["index"].clone(), d["outIndex"].clone()), (json!(a), json!(a + 1)));
}

/// With Adjust Adjoining Width Points, the outgoing half of a discontinuous point measures its
/// neighbors from the point's place, as `stroke.widthPoint.set` does: a point within 1e-6 of the
/// outgoing half but not of the place is the point after the pair.
#[test]
fn several_points_adjust_the_point_after_a_pair_from_its_place() {
    let (mut app, id) = app();
    let profile = json!([[0, 1, 1], [0.25, 0.8, 0.8], [0.5, 0.8, 0.8], [0.5000009, 0.4, 0.4], [0.5000018, 0.6, 0.6], [1, 1, 1]]);
    app.run("stroke.widthProfile.set", json!({"ids": [id], "points": profile})).unwrap();
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [1, 3], "adjustAdjoining": true})).unwrap();
    assert_eq!(fields(&app)["indices"], json!([1, 2, 3]));
    set(&app, "outSide1", json!(4));
    ok(&mut app).unwrap();
    assert!(!is_open(&app));
    // The outgoing set is linked, so both sides of the outgoing half double, and so do both sides
    // of the point after the pair; the end keeps its widths.
    assert_eq!(
        points(&app, id),
        vec![(0.0, 1.0, 1.0), (0.25, 0.8, 0.8), (0.5, 0.8, 0.8), (0.5000009, 0.8, 0.8), (0.5000018, 1.2, 1.2), (1.0, 1.0, 1.0)]
    );
}

#[test]
fn pairs_match_the_engine() {
    let profiles: [&[(f64, f64, f64)]; 5] = [
        &[],
        &[(0.5, 1.0, 1.0)],
        &[(0.0, 1.0, 1.0), (0.5, 1.0, 1.0), (0.5, 1.0, 1.0), (1.0, 1.0, 1.0)],
        &[(0.0, 1.0, 1.0), (0.5, 1.0, 1.0), (0.5, 1.0, 1.0), (0.5, 1.0, 1.0), (1.0, 1.0, 1.0)],
        &[
            (0.5, 1.0, 1.0),
            (0.50000075, 1.0, 1.0),
            (0.5000015, 1.0, 1.0),
            (0.5000024, 1.0, 1.0),
            (f64::NAN, 1.0, 1.0),
            (0.7, 1.0, 1.0),
            (0.7, 1.0, 1.0),
        ],
    ];
    for points in profiles {
        let engine: Vec<Option<usize>> = (0..points.len()).map(|a| width_point_pair(points, a)).collect();
        assert_eq!(pairs(points), engine, "{points:?}");
    }
}

#[test]
fn several_points_refuse_bad_indices() {
    let (mut app, id) = three_app();
    let before = app.session.doc().unwrap().doc.clone();
    let history = creation_history(&app);
    for p in [
        json!({"id": id, "indices": []}),
        json!({"id": id, "indices": [1, -1]}),
        json!({"id": id, "indices": [1, 1.5]}),
        json!({"id": id, "indices": [1, "3"]}),
        json!({"id": id, "indices": [1, 99]}),
        json!({"id": id, "indices": 1}),
        json!({"id": id, "indices": [1, 3], "index": 1}),
        json!({"id": id, "indices": [1, 3], "t": 0.5}),
        json!({"id": id, "index": 1, "t": 0.5}),
    ] {
        assert!(app.run("ui.widthPointEdit", p.clone()).is_err(), "{p}");
        assert!(!is_open(&app), "{p}");
        assert_eq!(app.session.doc().unwrap().doc, before);
        assert_eq!(creation_history(&app), history);
    }
    // A point moved since the dialog opened is refused at OK, even with its widths unchanged.
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [1, 3]})).unwrap();
    app.run("stroke.widthPoint.set", json!({"id": id, "index": 1, "t": 0.3, "left": 4, "right": 2})).unwrap();
    let moved = app.session.doc().unwrap().doc.clone();
    set(&app, "side1", json!(5));
    assert!(ok(&mut app).is_err());
    assert_eq!(app.session.doc().unwrap().doc, moved);
    app.run("edit.undo", json!({})).unwrap();
    // So is Delete after a point was added before them.
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [1, 3]})).unwrap();
    app.run("stroke.widthPoint.set", json!({"id": id, "t": 0.1, "left": 5, "right": 5})).unwrap();
    let added = app.session.doc().unwrap().doc.clone();
    assert!(delete(&mut app).is_err());
    assert_eq!(app.session.doc().unwrap().doc, added);
    app.run("edit.undo", json!({})).unwrap();
    // A value that is not a width is refused where it is set, and "3 pt" is read as 3 pt.
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [1, 3]})).unwrap();
    let shown = fields(&app);
    for bad in [json!(-1), json!("wide"), json!(1.0e6)] {
        assert!(try_set(&app, "side1", bad.clone()).is_err(), "{bad}");
        assert_eq!(fields(&app), shown, "{bad}");
    }
    set(&app, "side1", json!("3 pt"));
    assert_eq!(fields(&app)["side1"], json!(3.0));
    assert_eq!(app.session.doc().unwrap().doc, before);
}

#[test]
fn a_total_width_above_the_side_limit_does_not_block_ok() {
    // Two points of 60000 + 60000 pt on the 10 pt stroke: Total Width shows 120000 pt.
    let (mut app, id) = three_app();
    let wide = json!([[0, 1, 1], [0.25, 12000, 12000], [0.5, 0.8, 0.8], [0.75, 12000, 12000], [1, 1, 1]]);
    app.run("stroke.widthProfile.set", json!({"ids": [id], "points": wide})).unwrap();
    let before = app.session.doc().unwrap().doc.clone();
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [1, 3]})).unwrap();
    assert_eq!(fields(&app)["totalWidth"], json!(120000.0));
    ok(&mut app).unwrap();
    assert!(!is_open(&app));
    assert_eq!(app.session.doc().unwrap().doc, before);
    // A side width entered applies with the total shown.
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [1, 3]})).unwrap();
    set(&app, "linked", json!(false));
    set(&app, "side1", json!(5));
    ok(&mut app).unwrap();
    assert!(!is_open(&app));
    assert_eq!(points(&app, id), vec![(0.0, 1.0, 1.0), (0.25, 1.0, 12000.0), (0.5, 0.8, 0.8), (0.75, 1.0, 12000.0), (1.0, 1.0, 1.0)]);
    // A value set above the limit is refused even next to the 100000 pt shown.
    let (mut app, id) = three_app();
    let edge = json!([[0, 1, 1], [0.25, 20000, 0.4], [0.5, 0.8, 0.8], [0.75, 20000, 1.2], [1, 1, 1]]);
    app.run("stroke.widthProfile.set", json!({"ids": [id], "points": edge})).unwrap();
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [1, 3]})).unwrap();
    assert!(try_set(&app, "side1", json!(100000.0000005)).is_err());
}

#[test]
fn several_points_refuse_any_change_since_the_dialog_opened() {
    let (mut app, id) = three_app();
    // A change too small to show in the fields.
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [1, 3]})).unwrap();
    let nudged = json!([[0, 1, 1], [0.25, 0.8000001, 0.4], [0.5, 0.8, 0.8], [0.75, 0.4, 1.2], [1, 1, 1]]);
    app.run("stroke.widthProfile.set", json!({"ids": [id], "points": nudged})).unwrap();
    let changed = app.session.doc().unwrap().doc.clone();
    for field in ["side2", "delete"] {
        assert!(press_field(&mut app, field).is_err(), "{field}");
        assert!(is_open(&app), "a refused press keeps the dialog open: {field}");
        assert_eq!(app.session.doc().unwrap().doc, changed);
    }
    app.run("edit.undo", json!({})).unwrap();
    // OK with no width changed still checks the points.
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [1, 3]})).unwrap();
    app.run("stroke.widthPoint.set", json!({"id": id, "index": 1, "t": 0.3, "left": 4, "right": 2})).unwrap();
    assert!(ok(&mut app).is_err());
    assert!(is_open(&app));
}

/// Open a second document holding [`three_app`]'s line with the same id, and make it active.
fn second_document(app: &mut VectorcraftApp, id: u64) {
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let other = app.run("shape.line", json!({"x1": 100, "y1": 150, "x2": 300, "y2": 150})).unwrap()["id"].as_u64().unwrap();
    assert_eq!(other, id, "the same id in both documents");
    app.run("stroke.set", json!({"ids": [other], "weight": 10})).unwrap();
    let points = json!([[0, 1, 1], [0.25, 0.8, 0.4], [0.5, 0.8, 0.8], [0.75, 0.4, 1.2], [1, 1, 1]]);
    app.run("stroke.widthProfile.set", json!({"ids": [other], "points": points})).unwrap();
}

struct SavedFile(std::path::PathBuf);

impl Drop for SavedFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn exercise_reverted_point(prospective: bool) {
    let (mut app, id) = if prospective { creation_app() } else { app() };
    app.services.read = Some(Box::new(|path: &str| std::fs::read(path).map_err(|e| e.to_string())));
    app.services.write = Some(Box::new(|path: &str, bytes: &[u8]| std::fs::write(path, bytes).map_err(|e| e.to_string())));
    let path = SavedFile(std::env::temp_dir().join(format!("vc-ui-width-point-revert-{}-{prospective}.vectorcraft", std::process::id())));
    app.run("file.saveAs", json!({"path": path.0})).unwrap();
    assert!(path.0.is_file());
    let saved = points(&app, id);
    let state = app.session.active().unwrap();
    let (document, reverts) = (state.uid, state.reverts);
    app.run("shape.rectangle", json!({"x": 30, "y": 30, "width": 10, "height": 10})).unwrap();
    let target = if prospective { json!({"id": id, "t": 0.4}) } else { json!({"id": id, "index": 1}) };
    app.run("ui.widthPointEdit", target.clone()).unwrap();
    set(&app, "side1", json!(7));

    app.run("file.revert", json!({"confirmed": true})).unwrap();
    let state = app.session.active().unwrap();
    assert_eq!((state.uid, state.reverts), (document, reverts + 1));
    assert_eq!(points(&app, id), saved, "the reloaded path still looks like the original target");
    let undo = state.history.undo.len();
    let accepted = ok(&mut app);

    assert_eq!(points(&app, id), saved, "the old Width Point draft leaves the reloaded path alone");
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
    assert!(accepted.is_err() && is_open(&app));
    cancel(&app);
    app.run("ui.widthPointEdit", target).unwrap();
    set(&app, "side1", json!(7));
    ok(&mut app).unwrap();
    assert_ne!(points(&app, id), saved);
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
}

#[test]
fn existing_width_point_draft_cannot_edit_a_reverted_path() {
    exercise_reverted_point(false);
}

#[test]
fn prospective_width_point_draft_cannot_edit_a_reverted_path() {
    exercise_reverted_point(true);
}

#[test]
fn ok_and_delete_act_only_in_the_document_the_dialog_opened_in() {
    for opened in [json!({"indices": [1, 3]}), json!({"index": 2})] {
        for field in ["side1", "delete"] {
            let (mut app, id) = three_app();
            let mut p = opened.clone();
            p["id"] = json!(id);
            app.run("ui.widthPointEdit", p).unwrap();
            // The dialog stays open on the host while another document opens and becomes active.
            second_document(&mut app, id);
            let other = app.session.doc().unwrap().doc.clone();
            assert!(is_open(&app), "{opened} {field}");
            assert!(press_field(&mut app, field).is_err(), "{opened} {field}");
            assert_eq!(app.session.doc().unwrap().doc, other, "{opened} {field}");
        }
    }
}

#[test]
fn one_point_refuses_a_change_since_the_dialog_opened() {
    for field in ["side1", "delete"] {
        // A point added before the opened one moves it to the next index.
        let (mut app, id) = app();
        app.run("ui.widthPointEdit", json!({"id": id, "index": 1})).unwrap();
        app.run("stroke.widthPoint.set", json!({"id": id, "t": 0.25, "left": 3, "right": 3})).unwrap();
        let added = app.session.doc().unwrap().doc.clone();
        assert!(press_field(&mut app, field).is_err(), "{field}");
        assert_eq!(app.session.doc().unwrap().doc, added, "{field}");
        // The outgoing half of a discontinuous point changed.
        let (mut app, id) = pair_app();
        app.run("ui.widthPointEdit", json!({"id": id, "index": 1})).unwrap();
        app.run("stroke.widthProfile.set", json!({"ids": [id], "points": [[0, 1, 1], [0.5, 0.8, 0.8], [0.5, 0.2, 1.2], [1, 1, 1]]})).unwrap();
        let changed = app.session.doc().unwrap().doc.clone();
        assert!(press_field(&mut app, field).is_err(), "{field}");
        assert_eq!(app.session.doc().unwrap().doc, changed, "{field}");
        // The stroke weight changed.
        let (mut app, id) = self::app();
        app.run("ui.widthPointEdit", json!({"id": id, "index": 1})).unwrap();
        app.run("stroke.set", json!({"ids": [id], "weight": 20})).unwrap();
        let heavier = app.session.doc().unwrap().doc.clone();
        assert!(press_field(&mut app, field).is_err(), "{field}");
        assert_eq!(app.session.doc().unwrap().doc, heavier, "{field}");
        // A later point moved onto the opened one makes it one half of a discontinuous point.
        let (mut app, id) = self::app();
        app.run("stroke.widthPoint.set", json!({"id": id, "t": 0.75, "left": 3, "right": 3})).unwrap();
        app.run("ui.widthPointEdit", json!({"id": id, "index": 1})).unwrap();
        app.run("stroke.widthPoint.set", json!({"id": id, "index": 2, "t": 0.5, "left": 3, "right": 3})).unwrap();
        let paired = app.session.doc().unwrap().doc.clone();
        assert_eq!(points(&app, id)[1], (0.5, 0.8, 0.8), "the opened point keeps its index");
        assert!(press_field(&mut app, field).is_err(), "{field}");
        assert_eq!(app.session.doc().unwrap().doc, paired, "{field}");
    }
}

#[test]
fn several_points_refuse_a_change_in_which_points_pair() {
    // A point added at a selected point's place, after it, pairs with it; a third point at a
    // discontinuous point's place unpairs its halves. The selected points keep their index, place
    // and widths.
    let cases = [
        (
            three_app as fn() -> (VectorcraftApp, u64),
            json!([1, 3]),
            json!([[0, 1, 1], [0.25, 0.8, 0.4], [0.5, 0.8, 0.8], [0.75, 0.4, 1.2], [0.75, 0.3, 0.3], [1, 1, 1]]),
        ),
        (pair_app, json!([0, 1]), json!([[0, 1, 1], [0.5, 0.8, 0.8], [0.5, 0.4, 1.2], [0.5, 0.3, 0.3], [1, 1, 1]])),
    ];
    for (make, indices, points) in cases {
        for field in ["side2", "delete"] {
            let (mut app, id) = make();
            app.run("ui.widthPointEdit", json!({"id": id, "indices": indices})).unwrap();
            assert!(fields(&app).get("indices").is_some(), "{indices}: the several-point dialog opened");
            app.run("stroke.widthProfile.set", json!({"ids": [id], "points": points})).unwrap();
            let changed = app.session.doc().unwrap().doc.clone();
            assert!(press_field(&mut app, field).is_err(), "{indices} {field}");
            assert!(is_open(&app), "a refused press keeps the dialog open: {indices} {field}");
            assert_eq!(app.session.doc().unwrap().doc, changed, "{indices} {field}");
        }
    }
}
