use super::*;
use vectorcraft_engine::Session;
use vectorcraft_tools::{PointerEvent, PointerKind};

/// A 10 pt line from (100, 150) to (300, 150) with a 4 + 4 pt width point in the middle.
fn app() -> (VectorcraftApp, u64) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
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

fn frame(app: &mut VectorcraftApp) -> Vec<String> {
    fn texts(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    // A new window lays itself out in its first frame and paints in the next.
    ctx.run_ui(Default::default(), |ui| super::super::show(app, ui.ctx())).textures_delta.clear();
    let mut out = ctx.run_ui(Default::default(), |ui| super::super::show(app, ui.ctx()));
    out.textures_delta.clear();
    let mut v = vec![];
    out.shapes.iter().for_each(|c| texts(&c.shape, &mut v));
    v
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
        let d = app.ui.dialog.clone().expect("Width Point Edit opened");
        assert_eq!((d.kind.as_str(), d.f64("side1", 0.0), d.f64("side2", 0.0), d.bool("linked")), (KIND, 4.0, 4.0, true));
        assert_eq!(d.bool("adjustAdjoining"), shift);
        let shown = frame(&mut app);
        for s in ["Width Point Edit", "Side 1:", "Side 2:", "Total Width:", "Adjust Adjoining Width Points", "Delete"] {
            assert!(shown.iter().any(|t| t == s), "{s} in {shown:?}");
        }
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("side1".into(), json!(6));
        d.fields.insert("side2".into(), json!(2));
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
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
            app.ui.dialog.as_mut().unwrap().fields.insert("side1".into(), json!(20));
            super::super::cancel(&mut app);
            assert!(app.ui.dialog.is_none());
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
    app.ui.dialog.as_mut().unwrap().fields.insert("discard".into(), json!(true));
    super::super::confirm(&mut app).unwrap();
    assert_eq!(points(&app, id).len(), 2);
    assert!(app.run("ui.widthPointEdit", json!({"id": id, "index": 9})).is_err());
}

#[test]
fn it_does_not_stretch_to_the_screen() {
    let height = |screen_h: f32| {
        let (mut app, id) = app();
        app.run("ui.widthPointEdit", json!({"id": id, "index": 1})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, screen_h));
        for _ in 0..4 {
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            ctx.run_ui(input, |ui| super::super::show(&mut app, ui.ctx())).textures_delta.clear();
        }
        ctx.memory(|m| m.area_rect(egui::Id::unique(("dialog", KIND)))).unwrap().height()
    };
    let (short, tall) = (height(1000.0), height(2000.0));
    assert!((short - tall).abs() < 1.0 && short < 400.0, "{short} vs {tall}");
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
        assert!(app.ui.dialog.is_none());
        assert_eq!(app.session.doc().unwrap().doc, before, "click alone must not add a width point");
        assert_eq!(creation_history(app), history);
    }
    assert!(!app.session.in_interaction());
    crate::canvas::dispatch(app, &PointerEvent::new(PointerKind::DoubleClick, x, y), view);
    assert!(app.ui.dialog.is_some(), "double-clicking a stroked segment must open Width Point Edit");
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
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(d.kind, KIND);
    assert_eq!(d.fields["id"], json!(id));
    assert!(d.fields.get("index").is_none_or(Value::is_null));
    assert!((d.f64("t", -1.0) - 0.5).abs() < 1e-6);
    assert_eq!((d.f64("side1", 0.0), d.f64("side2", 0.0), d.bool("linked")), (5.0, 5.0, true));
    assert!(!d.bool("adjustAdjoining"));
    let shown = frame(&mut app);
    for label in ["Width Point Edit", "Side 1:", "Side 2:", "Total Width:", "Cancel"] {
        assert!(shown.iter().any(|text| text == label), "{label} in {shown:?}");
    }
    let d = app.ui.dialog.as_mut().unwrap();
    d.fields.insert("side1".into(), json!(6));
    d.fields.insert("side2".into(), json!(2));
    super::super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
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
        app.ui.dialog.as_mut().unwrap().fields.insert("side1".into(), json!(20));
        if discard {
            app.ui.dialog.as_mut().unwrap().fields.insert("discard".into(), json!(true));
            super::super::confirm(&mut app).unwrap();
        } else {
            super::super::cancel(&mut app);
        }
        assert!(app.ui.dialog.is_none());
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
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!(d.fields["id"], json!(id), "the compound owns its members' stroke");
        assert!(d.fields.get("index").is_none_or(Value::is_null));
        let actual_t = d.f64("t", -1.0);
        assert!((actual_t - 0.25).abs() < 1e-6, "compound={compound} pointer={quarter:?} actual_t={actual_t} want=0.25");
        // The two-point profile is linear: at a quarter its sides are 1.25 and 0.75 of
        // 1..2 and 0.5..1.5, on a 10pt stroke (5pt half).
        assert!((d.f64("side1", 0.0) - 6.25).abs() < 1e-6);
        assert!((d.f64("side2", 0.0) - 3.75).abs() < 1e-6);
        assert!(!d.bool("linked"));
        d.fields.insert("side1".into(), json!(8));
        d.fields.insert("side2".into(), json!(3));
        super::super::confirm(&mut app).unwrap();
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
    let d = app.ui.dialog.as_ref().unwrap();
    assert!(d.fields.get("index").is_none_or(Value::is_null));
    assert_eq!((d.f64("t", -1.0), d.f64("side1", 0.0), d.f64("side2", 0.0)), (0.25, 5.0, 5.0));
    super::super::cancel(&mut app);
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
        assert!(app.ui.dialog.is_none());
        assert_eq!(app.session.doc().unwrap().doc, before);
        assert_eq!(creation_history(&app), history);
    }
}

fn creation_dialog_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<(String, egui::Rect)> {
    fn collect(shape: &egui::Shape, clip: egui::Rect, texts: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Text(t) if clip.contains_rect(t.visual_bounding_rect()) => {
                texts.push((t.galley.text().to_string(), t.visual_bounding_rect()));
            }
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, clip, texts)),
            _ => {}
        }
    }
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1000.0, 700.0))), events, ..Default::default() };
    let mut output = ctx.run_ui(input, |ui| super::super::show(app, ui.ctx()));
    output.textures_delta.clear();
    let mut texts = vec![];
    output.shapes.iter().for_each(|shape| collect(&shape.shape, shape.clip_rect, &mut texts));
    texts
}

fn creation_dialog_click(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str) {
    for _ in 0..4 {
        creation_dialog_frame(app, ctx, vec![]);
    }
    let texts = creation_dialog_frame(app, ctx, vec![]);
    let pos = texts.iter().find(|(text, _)| text == label).unwrap_or_else(|| panic!("missing visible control {label}: {texts:?}")).1.center();
    for pressed in [true, false] {
        creation_dialog_frame(
            app,
            ctx,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() },
            ],
        );
    }
    creation_dialog_frame(app, ctx, vec![]);
}

fn creation_adjoining_checkbox(adjust: bool) {
    let (mut app, id) = creation_app();
    app.run("stroke.widthProfile.set", json!({"ids": [id], "points": [[0, 1, 1], [1, 1, 1]]})).unwrap();
    app.session.doc_mut().unwrap().mark_saved();
    let before = app.session.doc().unwrap().doc.clone();
    let selection = app.session.doc().unwrap().selection.clone();
    let history = creation_history(&app);
    creation_double_click(&mut app, 200.0, 150.0);
    assert!(app.ui.dialog.as_ref().unwrap().fields["index"].is_null());
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    crate::theme::apply(&ctx, Default::default());
    for _ in 0..5 {
        creation_dialog_frame(&mut app, &ctx, vec![]);
    }
    // The dialog naturally focuses and selects Side 1. Its linked partner changes
    // when the checkbox click commits the typed value; no fields are seeded.
    assert!(ctx.memory(|m| m.focused()).is_some());
    creation_dialog_frame(&mut app, &ctx, vec![egui::Event::Text("10 pt".into())]);
    creation_dialog_click(&mut app, &ctx, "Adjust Adjoining Width Points");
    assert!(app.ui.dialog.as_ref().unwrap().bool("adjustAdjoining"), "real checkbox click enables adjustment");
    if !adjust {
        creation_dialog_click(&mut app, &ctx, "Adjust Adjoining Width Points");
    }
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(d.bool("adjustAdjoining"), adjust);
    assert_eq!((d.f64("side1", 0.0), d.f64("side2", 0.0)), (10.0, 10.0));
    assert_eq!(app.session.doc().unwrap().doc, before);
    assert_eq!(creation_history(&app), history);
    creation_dialog_click(&mut app, &ctx, "OK");
    assert!(app.ui.dialog.is_none(), "real OK click confirms the pending point");
    let end = if adjust { 2.0 } else { 1.0 };
    assert_eq!(points(&app, id), vec![(0.0, end, end), (0.5, 2.0, 2.0), (1.0, end, end)], "checkbox choice controls adjoining widths");
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

#[test]
fn new_width_point_checkbox_adjusts_adjoining_in_one_step() {
    creation_adjoining_checkbox(true);
}

#[test]
fn new_width_point_checkbox_off_preserves_adjoining() {
    creation_adjoining_checkbox(false);
}

/// [`app`]'s line with a discontinuous point in the middle: 4 + 4 pt before it, 2 + 6 pt after it.
fn pair_app() -> (VectorcraftApp, u64) {
    let (mut app, id) = app();
    app.run("stroke.widthProfile.set", json!({"ids": [id], "points": [[0, 1, 1], [0.5, 0.8, 0.8], [0.5, 0.4, 1.2], [1, 1, 1]]})).unwrap();
    app.session.doc_mut().unwrap().mark_saved();
    (app, id)
}

fn assert_pair_fields(d: &Dialog) {
    assert_eq!((d.fields["index"].clone(), d.fields["outIndex"].clone()), (json!(1), json!(2)));
    assert_eq!((d.f64("side1", 0.0), d.f64("side2", 0.0), d.bool("linked")), (4.0, 4.0, true));
    assert_eq!((d.f64("outSide1", 0.0), d.f64("outSide2", 0.0), d.bool("outLinked")), (2.0, 6.0, false));
    assert_eq!(d.str("single"), "");
}

#[test]
fn either_half_of_a_discontinuous_point_shows_both_sets_of_side_widths() {
    for index in [1, 2] {
        let (mut app, id) = pair_app();
        app.run("ui.widthPointEdit", json!({"id": id, "index": index})).unwrap();
        assert_pair_fields(app.ui.dialog.as_ref().unwrap());
        let shown = frame(&mut app);
        for s in ["Incoming", "Outgoing", "Single Width Only", "None", "Adjust Adjoining Width Points", "Delete"] {
            assert!(shown.iter().any(|t| t == s), "{s} in {shown:?}");
        }
        for s in ["Side 1:", "Side 2:", "Total Width:"] {
            assert_eq!(shown.iter().filter(|t| *t == s).count(), 2, "{s} in each set: {shown:?}");
        }
    }
    // Double-clicking the point with the Width tool opens the same two sets.
    let (mut app, _) = pair_app();
    let view = app.view_info();
    app.select_tool("width");
    crate::canvas::dispatch(&mut app, &PointerEvent::new(PointerKind::DoubleClick, 200.0, 150.0), view);
    assert_pair_fields(app.ui.dialog.as_ref().expect("Width Point Edit opened"));
    // A continuous point keeps one set.
    let (mut app, id) = pair_app();
    app.run("ui.widthPointEdit", json!({"id": id, "index": 0})).unwrap();
    assert!(app.ui.dialog.as_ref().unwrap().fields.get("outIndex").is_none());
    let shown = frame(&mut app);
    assert!(!shown.iter().any(|t| t == "Incoming" || t == "Outgoing"), "{shown:?}");
    assert_eq!(shown.iter().filter(|t| *t == "Side 1:").count(), 1);
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
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!((d.fields["index"].clone(), d.fields["outIndex"].clone()), (json!(1), json!(2)), "opened {index}");
        d.fields.insert("outSide1".into(), json!(1));
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none(), "opened {index}");
        assert_eq!(points(&app, id)[2], (0.50000075, 0.2, 1.2), "opened {index}");
    }
    let (mut app, id) = app();
    app.run("stroke.widthProfile.set", json!({"ids": [id], "points": chain})).unwrap();
    app.run("ui.widthPointEdit", json!({"id": id, "index": 3})).unwrap();
    let d = app.ui.dialog.as_mut().unwrap();
    assert_eq!(d.fields["index"], json!(3));
    assert!(d.fields.get("outIndex").is_none());
    d.fields.insert("side1".into(), json!(1));
    super::super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(points(&app, id)[3], (0.5000015, 0.2, 0.6));
}

#[test]
fn ok_sets_both_halves_of_a_discontinuous_point_in_one_step() {
    let (mut app, id) = pair_app();
    let before = app.session.doc().unwrap().doc.clone();
    let history = creation_history(&app);
    // Cancel after edits changes nothing.
    app.run("ui.widthPointEdit", json!({"id": id, "index": 2})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("outSide1".into(), json!(9));
    super::super::cancel(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, before);
    assert_eq!(creation_history(&app), history);
    app.run("ui.widthPointEdit", json!({"id": id, "index": 2})).unwrap();
    let d = app.ui.dialog.as_mut().unwrap();
    for (key, value) in [("side1", 6), ("side2", 2), ("outSide1", 1), ("outSide2", 3)] {
        d.fields.insert(key.into(), json!(value));
    }
    super::super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(points(&app, id), vec![(0.0, 1.0, 1.0), (0.5, 1.2, 0.4), (0.5, 0.2, 0.6), (1.0, 1.0, 1.0)]);
    assert_eq!(creation_history(&app), (history.0 + 1, 0, true));
    let after = app.session.doc().unwrap().doc.clone();
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, before);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, after);
    // OK with nothing changed records no undo step.
    app.run("ui.widthPointEdit", json!({"id": id, "index": 1})).unwrap();
    super::super::confirm(&mut app).unwrap();
    assert_eq!(creation_history(&app), (history.0 + 1, 0, true));
}

#[test]
fn delete_removes_both_halves_of_a_discontinuous_point() {
    let (mut app, id) = pair_app();
    let before = app.session.doc().unwrap().doc.clone();
    let history = creation_history(&app);
    app.run("ui.widthPointEdit", json!({"id": id, "index": 1})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("discard".into(), json!(true));
    super::super::confirm(&mut app).unwrap();
    assert_eq!(points(&app, id), vec![(0.0, 1.0, 1.0), (1.0, 1.0, 1.0)]);
    assert_eq!(creation_history(&app).0, history.0 + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, before);
}

/// Click the last visible text `label` in the dialog (a radio after a heading of the same name).
fn click_last(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str) {
    for _ in 0..4 {
        creation_dialog_frame(app, ctx, vec![]);
    }
    let texts = creation_dialog_frame(app, ctx, vec![]);
    let pos = texts.iter().rev().find(|(text, _)| text == label).unwrap_or_else(|| panic!("missing visible control {label}: {texts:?}")).1.center();
    for pressed in [true, false] {
        let button = egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        creation_dialog_frame(app, ctx, vec![egui::Event::PointerMoved(pos), button]);
    }
    creation_dialog_frame(app, ctx, vec![]);
}

#[test]
fn single_width_only_keeps_the_chosen_half_as_one_point() {
    for (choice, kept) in [("Incoming", (0.8, 0.8)), ("Outgoing", (0.4, 1.2))] {
        let (mut app, id) = pair_app();
        let before = app.session.doc().unwrap().doc.clone();
        let history = creation_history(&app);
        app.run("ui.widthPointEdit", json!({"id": id, "index": 1})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, Default::default());
        click_last(&mut app, &ctx, choice);
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("single"), choice.to_lowercase());
        click_last(&mut app, &ctx, "OK");
        assert!(app.ui.dialog.is_none());
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
    let d = app.ui.dialog.as_mut().unwrap();
    assert_eq!(d.fields["id"], json!(owner));
    d.fields.insert("adjustAdjoining".into(), json!(true));
    d.fields.insert("side1".into(), json!(8));
    d.fields.insert("outSide2".into(), json!(3));
    super::super::confirm(&mut app).unwrap();
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
    let d = app.ui.dialog.as_mut().unwrap();
    for (key, value) in fields {
        d.fields.insert((*key).into(), value.clone());
    }
    super::super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    (app, id)
}

#[test]
fn several_points_show_blank_fields_where_they_differ() {
    let (mut app, id) = three_app();
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [3, 1, 3]})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(d.fields["indices"], json!([1, 3]));
    for key in ["side1", "side2", "totalWidth"] {
        assert!(d.fields[key].is_null(), "{key}: {:?}", d.fields[key]);
    }
    assert!(!d.bool("linked"));
    assert!(d.fields.get("outSide1").is_none());
    let shown = frame(&mut app);
    for s in ["Side 1:", "Side 2:", "Total Width:", "Adjust Adjoining Width Points", "Delete"] {
        assert_eq!(shown.iter().filter(|t| *t == s).count(), 1, "{s} in {shown:?}");
    }
    // Points with the same widths show them.
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [1, 2]})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.fields["side1"].clone(), d.fields["side2"].clone(), d.fields["totalWidth"].clone()), (json!(4.0), Value::Null, Value::Null));
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
    for ok in [false, true] {
        app.run("ui.widthPointEdit", json!({"id": id, "indices": [1, 3]})).unwrap();
        if ok {
            super::super::confirm(&mut app).unwrap();
        } else {
            app.ui.dialog.as_mut().unwrap().fields.insert("side1".into(), json!(9));
            super::super::cancel(&mut app);
        }
        assert!(app.ui.dialog.is_none());
        assert_eq!(app.session.doc().unwrap().doc, before);
        assert_eq!(creation_history(&app), history);
    }
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [1, 3]})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("side1".into(), json!(5));
    super::super::confirm(&mut app).unwrap();
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
    let d = app.ui.dialog.as_mut().unwrap();
    super::super::set_field(d, "linked".into(), json!(true));
    super::super::set_field(d, "side1".into(), json!(4));
    assert_eq!((d.fields["side1"].clone(), d.fields["side2"].clone()), (json!(4.0), json!(4.0)));
    super::super::confirm(&mut app).unwrap();
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
    app.ui.dialog.as_mut().unwrap().fields.insert("discard".into(), json!(true));
    super::super::confirm(&mut app).unwrap();
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
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(d.fields["indices"], json!([1, 2, 3]));
    // Incoming: the quarter (4 + 2) and the incoming half (4 + 4); outgoing: the quarter and the
    // outgoing half (2 + 6).
    assert_eq!((d.fields["side1"].clone(), d.fields["side2"].clone()), (json!(4.0), Value::Null));
    assert_eq!((d.fields["outSide1"].clone(), d.fields["outSide2"].clone()), (Value::Null, Value::Null));
    let shown = frame(&mut app);
    for s in ["Incoming", "Outgoing"] {
        assert!(shown.iter().any(|t| t == s), "{s} in {shown:?}");
    }
    assert_eq!(shown.iter().filter(|t| *t == "Side 1:").count(), 2);
    app.ui.dialog.as_mut().unwrap().fields.insert("outSide2".into(), json!(5));
    super::super::confirm(&mut app).unwrap();
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
    let d = app.ui.dialog.as_mut().unwrap();
    assert_eq!(d.fields["indices"], json!([1, 2, 3]));
    d.fields.insert("side1".into(), json!(5));
    d.fields.insert("outSide2".into(), json!(5));
    super::super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
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
    let d = app.ui.dialog.as_mut().unwrap();
    assert_eq!(d.fields["indices"], json!([1, 2, 3]));
    d.fields.insert("linked".into(), json!(false));
    d.fields.insert("side2".into(), json!(5));
    super::super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(points(&app, id), vec![(0.0, 1.0, 1.0), (0.5, 0.8, 1.0), (0.5000005, 0.4, 1.0), (0.5000009, 0.6, 1.0), (1.0, 1.0, 1.0)]);
    let (mut app, id) = pair_app();
    let pair = app.session.active().unwrap().doc.node(NodeId(id)).unwrap().appearance.stroke().unwrap().profile.clone().unwrap();
    let a = (0..pair.points.len()).find(|&a| width_point_pair(&pair.points, a).is_some()).unwrap();
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [a + 1, a]})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.fields["index"].clone(), d.fields["outIndex"].clone()), (json!(a), json!(a + 1)));
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
    let d = app.ui.dialog.as_mut().unwrap();
    assert_eq!(d.fields["indices"], json!([1, 2, 3]));
    d.fields.insert("outSide1".into(), json!(4));
    super::super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
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
fn double_clicking_one_of_several_selected_points_and_typing_edits_them_all() {
    let (mut app, id) = three_app();
    let view = app.view_info();
    app.select_tool("width");
    let send = |app: &mut VectorcraftApp, kind: PointerKind, x: f64, shift: bool| {
        let mut ev = PointerEvent::new(kind, x, 150.0);
        ev.mods.shift = shift;
        crate::canvas::dispatch(app, &ev, view);
    };
    for (x, shift) in [(150.0, false), (250.0, true)] {
        send(&mut app, PointerKind::Down, x, shift);
        send(&mut app, PointerKind::Up, x, shift);
    }
    for kind in [PointerKind::Down, PointerKind::Up, PointerKind::Down, PointerKind::Up, PointerKind::DoubleClick] {
        send(&mut app, kind, 150.0, false);
    }
    assert_eq!(app.ui.dialog.as_ref().expect("Width Point Edit opened").fields["indices"], json!([1, 3]));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    crate::theme::apply(&ctx, Default::default());
    for _ in 0..5 {
        creation_dialog_frame(&mut app, &ctx, vec![]);
    }
    // The blank Side 1 has the focus: typing sets Side 1 of both points.
    creation_dialog_frame(&mut app, &ctx, vec![egui::Event::Text("5 pt".into())]);
    creation_dialog_click(&mut app, &ctx, "OK");
    assert!(app.ui.dialog.is_none());
    assert_eq!(points(&app, id), vec![(0.0, 1.0, 1.0), (0.25, 1.0, 0.4), (0.5, 0.8, 0.8), (0.75, 1.0, 1.2), (1.0, 1.0, 1.0)]);
}

#[test]
fn a_negative_width_typed_for_several_points_is_refused() {
    let (mut app, id) = three_app();
    let before = app.session.doc().unwrap().doc.clone();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    crate::theme::apply(&ctx, Default::default());
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [1, 3]})).unwrap();
    for _ in 0..5 {
        creation_dialog_frame(&mut app, &ctx, vec![]);
    }
    // The blank Side 1 has the focus.
    creation_dialog_frame(&mut app, &ctx, vec![egui::Event::Text("-3 pt".into())]);
    creation_dialog_click(&mut app, &ctx, "OK");
    assert!(app.ui.dialog.is_some(), "OK refuses it and the dialog stays open");
    assert_eq!(app.session.doc().unwrap().doc, before);
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
        assert!(app.ui.dialog.is_none(), "{p}");
        assert_eq!(app.session.doc().unwrap().doc, before);
        assert_eq!(creation_history(&app), history);
    }
    // A point moved since the dialog opened is refused at OK, even with its widths unchanged.
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [1, 3]})).unwrap();
    app.run("stroke.widthPoint.set", json!({"id": id, "index": 1, "t": 0.3, "left": 4, "right": 2})).unwrap();
    let moved = app.session.doc().unwrap().doc.clone();
    app.ui.dialog.as_mut().unwrap().fields.insert("side1".into(), json!(5));
    assert!(super::super::confirm(&mut app).is_err());
    assert_eq!(app.session.doc().unwrap().doc, moved);
    app.run("edit.undo", json!({})).unwrap();
    // So is Delete after a point was added before them.
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [1, 3]})).unwrap();
    app.run("stroke.widthPoint.set", json!({"id": id, "t": 0.1, "left": 5, "right": 5})).unwrap();
    let added = app.session.doc().unwrap().doc.clone();
    app.ui.dialog.as_mut().unwrap().fields.insert("discard".into(), json!(true));
    assert!(super::super::confirm(&mut app).is_err());
    assert_eq!(app.session.doc().unwrap().doc, added);
    app.run("edit.undo", json!({})).unwrap();
    // A value that is not a width is refused at OK, and the dialog stays open.
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [1, 3]})).unwrap();
    for bad in [json!(-1), json!("wide"), json!(1.0e6)] {
        app.ui.dialog.as_mut().unwrap().fields.insert("side1".into(), bad.clone());
        assert!(super::super::confirm(&mut app).is_err(), "{bad}");
        assert!(app.ui.dialog.is_some());
        assert_eq!(app.session.doc().unwrap().doc, before);
    }
}

#[test]
fn a_total_width_above_the_side_limit_does_not_block_ok() {
    // Two points of 60000 + 60000 pt on the 10 pt stroke: Total Width shows 120000 pt.
    let (mut app, id) = three_app();
    let wide = json!([[0, 1, 1], [0.25, 12000, 12000], [0.5, 0.8, 0.8], [0.75, 12000, 12000], [1, 1, 1]]);
    app.run("stroke.widthProfile.set", json!({"ids": [id], "points": wide})).unwrap();
    let before = app.session.doc().unwrap().doc.clone();
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [1, 3]})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["totalWidth"], json!(120000.0));
    super::super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.session.doc().unwrap().doc, before);
    // A side width typed in applies with the total shown.
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [1, 3]})).unwrap();
    let d = app.ui.dialog.as_mut().unwrap();
    d.fields.insert("linked".into(), json!(false));
    super::super::set_field(d, "side1".into(), json!(5));
    super::super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(points(&app, id), vec![(0.0, 1.0, 1.0), (0.25, 1.0, 12000.0), (0.5, 0.8, 0.8), (0.75, 1.0, 12000.0), (1.0, 1.0, 1.0)]);
    // A value set above the limit is refused even next to the 100000 pt shown.
    let (mut app, id) = three_app();
    let edge = json!([[0, 1, 1], [0.25, 20000, 0.4], [0.5, 0.8, 0.8], [0.75, 20000, 1.2], [1, 1, 1]]);
    app.run("stroke.widthProfile.set", json!({"ids": [id], "points": edge})).unwrap();
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [1, 3]})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("side1".into(), json!(100000.0000005));
    assert!(super::super::confirm(&mut app).is_err());
}

#[test]
fn an_invalid_width_stays_for_ok_to_refuse() {
    let (mut app, id) = three_app();
    let before = app.session.doc().unwrap().doc.clone();
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [1, 3]})).unwrap();
    let d = app.ui.dialog.as_mut().unwrap();
    super::super::set_field(d, "side1".into(), json!("wide"));
    super::super::set_field(d, "side2".into(), json!("3 pt"));
    assert_eq!(d.fields["side1"], json!("wide"));
    assert!(super::super::confirm(&mut app).is_err());
    assert!(app.ui.dialog.is_some());
    assert_eq!(app.session.doc().unwrap().doc, before);
    // Corrected, it applies with the Side 2 already set.
    super::super::set_field(app.ui.dialog.as_mut().unwrap(), "side1".into(), json!(5));
    super::super::confirm(&mut app).unwrap();
    assert_eq!(points(&app, id), vec![(0.0, 1.0, 1.0), (0.25, 1.0, 0.6), (0.5, 0.8, 0.8), (0.75, 1.0, 0.6), (1.0, 1.0, 1.0)]);
}

#[test]
fn several_points_refuse_any_change_since_the_dialog_opened() {
    let (mut app, id) = three_app();
    // A change too small to show in the fields.
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [1, 3]})).unwrap();
    let nudged = json!([[0, 1, 1], [0.25, 0.8000001, 0.4], [0.5, 0.8, 0.8], [0.75, 0.4, 1.2], [1, 1, 1]]);
    app.run("stroke.widthProfile.set", json!({"ids": [id], "points": nudged})).unwrap();
    let changed = app.session.doc().unwrap().doc.clone();
    for field in ["side2", "discard"] {
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert(field.into(), if field == "discard" { json!(true) } else { json!(5) });
        assert!(super::super::confirm(&mut app).is_err(), "{field}");
        assert_eq!(app.session.doc().unwrap().doc, changed);
    }
    app.run("edit.undo", json!({})).unwrap();
    // OK with no width changed still checks the points.
    app.run("ui.widthPointEdit", json!({"id": id, "indices": [1, 3]})).unwrap();
    app.run("stroke.widthPoint.set", json!({"id": id, "index": 1, "t": 0.3, "left": 4, "right": 2})).unwrap();
    assert!(super::super::confirm(&mut app).is_err());
    assert!(app.ui.dialog.is_some());
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

#[test]
fn ok_and_delete_act_only_in_the_document_the_dialog_opened_in() {
    for opened in [json!({"indices": [1, 3]}), json!({"index": 2})] {
        for field in ["side1", "discard"] {
            let (mut app, id) = three_app();
            let mut p = opened.clone();
            p["id"] = json!(id);
            app.run("ui.widthPointEdit", p).unwrap();
            let d = app.ui.dialog.take();
            second_document(&mut app, id);
            let other = app.session.doc().unwrap().doc.clone();
            app.ui.dialog = d;
            let d = app.ui.dialog.as_mut().unwrap();
            d.fields.insert(field.into(), if field == "discard" { json!(true) } else { json!(5) });
            assert!(super::super::confirm(&mut app).is_err(), "{opened} {field}");
            assert_eq!(app.session.doc().unwrap().doc, other, "{opened} {field}");
        }
    }
}

#[test]
fn one_point_refuses_a_change_since_the_dialog_opened() {
    for field in ["side1", "discard"] {
        // A point added before the opened one moves it to the next index.
        let (mut app, id) = app();
        app.run("ui.widthPointEdit", json!({"id": id, "index": 1})).unwrap();
        app.run("stroke.widthPoint.set", json!({"id": id, "t": 0.25, "left": 3, "right": 3})).unwrap();
        let added = app.session.doc().unwrap().doc.clone();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert(field.into(), if field == "discard" { json!(true) } else { json!(5) });
        assert!(super::super::confirm(&mut app).is_err(), "{field}");
        assert_eq!(app.session.doc().unwrap().doc, added, "{field}");
        // The outgoing half of a discontinuous point changed.
        let (mut app, id) = pair_app();
        app.run("ui.widthPointEdit", json!({"id": id, "index": 1})).unwrap();
        app.run("stroke.widthProfile.set", json!({"ids": [id], "points": [[0, 1, 1], [0.5, 0.8, 0.8], [0.5, 0.2, 1.2], [1, 1, 1]]})).unwrap();
        let changed = app.session.doc().unwrap().doc.clone();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert(field.into(), if field == "discard" { json!(true) } else { json!(5) });
        assert!(super::super::confirm(&mut app).is_err(), "{field}");
        assert_eq!(app.session.doc().unwrap().doc, changed, "{field}");
        // The stroke weight changed.
        let (mut app, id) = self::app();
        app.run("ui.widthPointEdit", json!({"id": id, "index": 1})).unwrap();
        app.run("stroke.set", json!({"ids": [id], "weight": 20})).unwrap();
        let heavier = app.session.doc().unwrap().doc.clone();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert(field.into(), if field == "discard" { json!(true) } else { json!(5) });
        assert!(super::super::confirm(&mut app).is_err(), "{field}");
        assert_eq!(app.session.doc().unwrap().doc, heavier, "{field}");
        // A later point moved onto the opened one makes it one half of a discontinuous point.
        let (mut app, id) = self::app();
        app.run("stroke.widthPoint.set", json!({"id": id, "t": 0.75, "left": 3, "right": 3})).unwrap();
        app.run("ui.widthPointEdit", json!({"id": id, "index": 1})).unwrap();
        app.run("stroke.widthPoint.set", json!({"id": id, "index": 2, "t": 0.5, "left": 3, "right": 3})).unwrap();
        let paired = app.session.doc().unwrap().doc.clone();
        assert_eq!(points(&app, id)[1], (0.5, 0.8, 0.8), "the opened point keeps its index");
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert(field.into(), if field == "discard" { json!(true) } else { json!(5) });
        assert!(super::super::confirm(&mut app).is_err(), "{field}");
        assert_eq!(app.session.doc().unwrap().doc, paired, "{field}");
    }
}
