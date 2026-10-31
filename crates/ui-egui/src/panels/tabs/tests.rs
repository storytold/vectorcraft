use super::*;
use vectorcraft_engine::Session;

fn frame(app: &mut VectorcraftApp) {
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        show(app, ui);
        menu(app, ui);
    });
    out.textures_delta.clear();
}

#[test]
fn panel_draws_empty_with_text_and_with_stops() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({})).unwrap();
    frame(&mut app);
    app.session.execute("text.create", &json!({"x": 10, "y": 20, "text": "a\tb", "area": {"width": 300, "height": 100}})).unwrap();
    frame(&mut app);
    app.session.execute("text.tabs.set", &json!({"stops": [{"position": 50}, {"position": 120, "align": "decimal"}]})).unwrap();
    frame(&mut app);
    let (_, stops, span) = current(&app).unwrap();
    assert_eq!(stops.len(), 2);
    assert_eq!(span, 300.0);
    // The stops survive a JSON round trip through the panel's encoding.
    let back: Vec<serde_json::Value> = stops_json(&stops).as_array().unwrap().clone();
    assert_eq!(back[1]["align"], json!("decimal"));
}

/// In vertical type the Left and Right alignment buttons are named Top-Justified Tab and
/// Bottom-Justified Tab, in their tool tips and for screen readers, and the position field is
/// labeled Y. Horizontal type keeps Left-Justified Tab, Right-Justified Tab and X.
#[test]
fn vertical_type_names_the_alignments_top_and_bottom_and_the_position_y() {
    use crate::widgets::accessibility_tests::{role_and_state, tree};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({})).unwrap();
    app.session.execute("text.create", &json!({"x": 10, "y": 20, "text": "a	b", "area": {"width": 300, "height": 100}})).unwrap();
    app.session.execute("text.tabs.set", &json!({"stops": [{"position": 50}]})).unwrap();
    let horizontal = ["Left-Justified Tab", "Center-Justified Tab", "Right-Justified Tab", "Decimal-Justified Tab"];
    let vertical = ["Top-Justified Tab", "Center-Justified Tab", "Bottom-Justified Tab", "Decimal-Justified Tab"];
    for (names, other, axis, other_axis) in [(horizontal, vertical, "X:", "Y:"), (vertical, horizontal, "Y:", "X:")] {
        if axis == "Y:" {
            app.session.execute("type.orientation.vertical", &json!({})).unwrap();
        }
        // Screen readers find the four buttons by name, and the selected stop's alignment is pressed.
        let nodes = tree(|ui| show(&mut app, ui));
        for (i, name) in names.iter().enumerate() {
            let pressed = if i == 0 { egui::accesskit::Toggled::True } else { egui::accesskit::Toggled::False };
            assert_eq!(role_and_state(&nodes, name), (egui::accesskit::Role::Button, Some(pressed)), "{name}");
        }
        for name in other.iter().filter(|n| !names.contains(n)) {
            assert!(!nodes.iter().any(|n| n.label() == Some(name)), "{name} with {axis}");
        }
        // Hovering each button shows its tool tip, and the panel shows the position field's label.
        let ctx = egui::Context::default();
        ctx.global_style_mut(|s| s.interaction.tooltip_delay = 0.0);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 400.0));
        let mut time = 0.0;
        let mut frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            time += 0.5;
            let input = egui::RawInput { time: Some(time), screen_rect: Some(screen), events, ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| show(app, ui));
            out.textures_delta.clear();
            out.shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    egui::Shape::Text(t) => Some((t.galley.text().to_string(), t.visual_bounding_rect())),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let texts = frame(&mut app, vec![]);
        assert!(texts.iter().any(|(t, _)| t == axis) && !texts.iter().any(|(t, _)| t == other_axis), "{axis} in {texts:?}");
        for ((_, glyph), name) in ALIGNS.iter().zip(names) {
            let at = texts.iter().find(|(t, _)| t == glyph).unwrap_or_else(|| panic!("{glyph} in {texts:?}")).1.center();
            frame(&mut app, vec![egui::Event::PointerMoved(at)]);
            let tips = (0..3).flat_map(|_| frame(&mut app, vec![])).collect::<Vec<_>>();
            assert!(tips.iter().any(|(t, _)| t == name), "the tip {name} in {tips:?}");
        }
    }
}

/// The position field's label is translated: French puts a no-break space before the colon,
/// Chinese writes a full-width colon.
#[test]
fn position_label_is_translated() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({})).unwrap();
    app.session.execute("text.create", &json!({"x": 10, "y": 20, "text": "a	b", "area": {"width": 300, "height": 100}})).unwrap();
    app.session.execute("text.tabs.set", &json!({"stops": [{"position": 50}]})).unwrap();
    let labels = |app: &mut VectorcraftApp| {
        let text = crate::tests_labels::painted_text(app, show);
        ["X:", "Y:", "X\u{a0}:", "Y\u{a0}:", "X：", "Y："].into_iter().filter(|l| text.lines().any(|t| t.trim() == *l)).collect::<Vec<_>>()
    };
    for (code, horizontal, vertical) in [("fr", "X\u{a0}:", "Y\u{a0}:"), ("zh-hans", "X：", "Y："), ("en", "X:", "Y:")] {
        crate::i18n::set_current(crate::i18n::Lang::from_code(code).unwrap());
        app.session.execute("type.orientation.horizontal", &json!({})).unwrap();
        assert_eq!(labels(&mut app), [horizontal], "{code}");
        app.session.execute("type.orientation.vertical", &json!({})).unwrap();
        assert_eq!(labels(&mut app), [vertical], "{code}");
    }
}

/// #924: Leader and Align On keep what is typed, frame after frame, until Enter applies it
/// (spaces kept, eight characters at most).
#[test]
fn leader_and_align_on_keep_what_is_typed_until_enter() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({})).unwrap();
    app.session.execute("text.create", &json!({"x": 10, "y": 20, "text": "a	b", "area": {"width": 300, "height": 100}})).unwrap();
    app.session.execute("text.tabs.set", &json!({"stops": [{"position": 50, "align": "decimal"}]})).unwrap();
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(320.0, 400.0));
    // One frame with `events` → the texts painted and where.
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
        out.shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) => Some((t.galley.text().to_string(), t.visual_bounding_rect())),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    let click = |at| {
        [true, false].map(|pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        })
    };
    let stop = |app: &VectorcraftApp| current(app).unwrap().1[0].clone();
    for (label, typed, want) in [("Leader:", [". ", "_", "123456789"], ". _12345"), ("Align On:", [",", "", ""], ",")] {
        let texts = frame(&mut app, vec![]);
        let r = texts.iter().find(|(t, _)| t == label).unwrap_or_else(|| panic!("{label} in {texts:?}")).1;
        let at = egui::pos2(r.right() + 30.0, r.center().y);
        let [down, up] = click(at);
        frame(&mut app, vec![egui::Event::PointerMoved(at), down]);
        frame(&mut app, vec![up]);
        // Select what is there, then type in pieces over several frames.
        frame(
            &mut app,
            vec![egui::Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND }],
        );
        for piece in typed.iter().filter(|p| !p.is_empty()) {
            frame(&mut app, vec![egui::Event::Text(piece.to_string())]);
            frame(&mut app, vec![]);
        }
        frame(&mut app, vec![key(egui::Key::Enter)]);
        frame(&mut app, vec![]);
        let s = stop(&app);
        let got = if label == "Leader:" { s.leader } else { s.align_on.to_string() };
        assert_eq!(got, want, "{label}");
    }
}

/// #1458: a stop moves when the drag starts on it, even though egui only reports the drag once
/// the pointer is past its threshold (farther from the press than the 5 px hit tolerance).
#[test]
fn dragging_a_stop_moves_it() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({})).unwrap();
    app.session.execute("text.create", &json!({"x": 10, "y": 20, "text": "a	b", "area": {"width": 300, "height": 100}})).unwrap();
    app.session.execute("text.tabs.set", &json!({"stops": [{"position": 50}]})).unwrap();
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(320.0, 400.0));
    // One frame with `events` → where the ruler's inch labels (9 pt) are painted.
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
        out.shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) if t.galley.job.sections.first().is_some_and(|s| s.format.font_id.size == 9.0) => {
                    Some((t.galley.text().to_string(), t.pos))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let labels = frame(&mut app, vec![]);
    let at = |label: &str| labels.iter().find(|(t, _)| t == label).unwrap_or_else(|| panic!("{label} in {labels:?}")).1;
    // A label sits 2 px right of its tick: 0 and 100 pt give the ruler's scale.
    let (zero, hundred) = (at("0"), at("100"));
    let to_x = |pt: f32| zero.x - 2.0 + pt * (hundred.x - zero.x) / 100.0;
    let y = zero.y + 15.0;
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    // Press 3 px right of the stop, then drag right; egui starts the drag 6+ px from the press.
    let press = egui::pos2(to_x(50.0) + 3.0, y);
    frame(&mut app, vec![egui::Event::PointerMoved(press), button(press, true)]);
    for dx in [9.0, 20.0, 40.0] {
        frame(&mut app, vec![egui::Event::PointerMoved(press + vec2(dx, 0.0))]);
    }
    let end = press + vec2(40.0, 0.0);
    frame(&mut app, vec![button(end, false)]);
    frame(&mut app, vec![]);
    let stops = current(&app).unwrap().1;
    assert_eq!(stops.len(), 1);
    let moved_to = to_x(stops[0].position as f32);
    assert!((moved_to - end.x).abs() < 1.0, "stop at {moved_to}, released at {}", end.x);
}

/// One frame of the panel with `events`, Shift held when `shift` → the ruler's labels (9 pt) and
/// where they are painted, left to right.
fn ruler_frame(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>, shift: bool) -> Vec<(String, egui::Pos2)> {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 400.0));
    let modifiers = if shift { egui::Modifiers::SHIFT } else { egui::Modifiers::NONE };
    let events = std::iter::once(egui::Event::ModifiersChanged(modifiers)).chain(events).collect();
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| show(app, ui));
    out.textures_delta.clear();
    out.shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.job.sections.first().is_some_and(|s| s.format.font_id.size == 9.0) => {
                Some((t.galley.text().to_string(), t.pos))
            }
            _ => None,
        })
        .collect()
}

/// The labels the ruler shows in a frame of [`ruler_frame`].
fn ruler_labels(ctx: &egui::Context, app: &mut VectorcraftApp) -> Vec<String> {
    ruler_frame(ctx, app, vec![], false).into_iter().map(|(t, _)| t).collect()
}

/// Press on the ruler at `from` (points), move to `to` and release there, Shift held when
/// `shift`. A label sits 2 px right of its tick, so the first two labels give the ruler's scale.
fn drag_on_ruler(ctx: &egui::Context, app: &mut VectorcraftApp, from: f64, to: f64, shift: bool) {
    let labels = ruler_frame(ctx, app, vec![], shift);
    let [(zero, a), (next, b), ..] = &labels[..] else { panic!("ruler labels {labels:?}") };
    assert_eq!(zero, "0");
    let per = next.parse::<f64>().unwrap() * app.session.general_unit().points();
    let x = |pt: f64| a.x - 2.0 + (pt / per) as f32 * (b.x - a.x);
    let y = a.y + 15.0;
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let (press, end) = (egui::pos2(x(from), y), egui::pos2(x(to), y));
    ruler_frame(ctx, app, vec![egui::Event::PointerMoved(press), button(press, true)], shift);
    if press != end {
        for f in [0.25, 0.5, 1.0] {
            ruler_frame(ctx, app, vec![egui::Event::PointerMoved(press.lerp(end, f))], shift);
        }
    }
    ruler_frame(ctx, app, vec![button(end, false)], shift);
    ruler_frame(ctx, app, vec![], shift);
}

fn indent_app(left: f64, first: f64) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({})).unwrap();
    app.run("text.create", json!({"x": 10, "y": 20, "text": "one\ntwo", "area": {"width": 300, "height": 100}})).unwrap();
    app.run("text.setFormat", json!({"leftIndent": left, "firstLineIndent": first})).unwrap();
    app
}

fn indent_values(app: &VectorcraftApp, paragraph: usize) -> (f64, f64) {
    let (node, _, _) = current(app).unwrap();
    let NodeKind::Text(t) = node.kind else { panic!("selected text") };
    let p = t.para_at(paragraph);
    (p.left_indent, p.first_line_indent)
}

fn marker_event(ctx: &egui::Context, app: &mut VectorcraftApp, from: f64, to: f64, upper: bool, command: bool, shift: bool) {
    let labels = ruler_frame(ctx, app, vec![], shift);
    let [(zero, a), (next, b), ..] = &labels[..] else { panic!("ruler labels {labels:?}") };
    assert_eq!(zero, "0");
    let per = next.parse::<f64>().unwrap() * app.session.general_unit().points();
    let x = |pt: f64| a.x - 2.0 + (pt / per) as f32 * (b.x - a.x);
    let y = a.y + if upper { 3.0 } else { 11.0 };
    let (press, end) = (egui::pos2(x(from), y), egui::pos2(x(to), y));
    let modifiers = match (command, shift) {
        (true, true) => egui::Modifiers { command: true, shift: true, ..Default::default() },
        (true, false) => egui::Modifiers::COMMAND,
        (false, true) => egui::Modifiers::SHIFT,
        (false, false) => egui::Modifiers::NONE,
    };
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
    ruler_frame(ctx, app, vec![egui::Event::ModifiersChanged(modifiers), egui::Event::PointerMoved(press), button(press, true)], shift);
    if press != end {
        for f in [0.25, 0.5, 1.0] {
            ruler_frame(ctx, app, vec![egui::Event::ModifiersChanged(modifiers), egui::Event::PointerMoved(press.lerp(end, f))], shift);
        }
    }
    ruler_frame(ctx, app, vec![egui::Event::ModifiersChanged(modifiers), button(end, false)], shift);
    ruler_frame(ctx, app, vec![], shift);
}

#[test]
fn indent_markers_show_positions_and_select_without_adding_stops() {
    let mut app = indent_app(24.0, 12.0);
    let ctx = egui::Context::default();
    assert_eq!(marker_position(IndentMarker::Lower, &current_para(&app).unwrap().1), 24.0);
    assert_eq!(marker_position(IndentMarker::Upper, &current_para(&app).unwrap().1), 36.0);
    let labels = ruler_frame(&ctx, &mut app, vec![], false);
    let [(zero, a), (next, b), ..] = &labels[..] else { panic!("ruler labels {labels:?}") };
    assert_eq!(zero, "0");
    let per = next.parse::<f64>().unwrap() * app.session.general_unit().points();
    let x = |pt: f64| a.x - 2.0 + (pt / per) as f32 * (b.x - a.x);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 400.0));
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| show(&mut app, ui));
    out.textures_delta.clear();
    let triangles: Vec<egui::Pos2> = out
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Path(path) if path.points.len() == 3 && path.points.iter().all(|p| p.y >= a.y && p.y <= a.y + 16.0) => {
                Some(egui::pos2(path.points.iter().map(|p| p.x).sum::<f32>() / 3.0, path.points.iter().map(|p| p.y).sum::<f32>() / 3.0))
            }
            _ => None,
        })
        .collect();
    assert!(triangles.iter().any(|p| (p.x - x(36.0)).abs() < 0.5 && p.y < a.y + 8.0), "upper marker at 36 pt: {triangles:?}");
    assert!(triangles.iter().any(|p| (p.x - x(24.0)).abs() < 0.5 && p.y > a.y + 8.0), "lower marker at 24 pt: {triangles:?}");
    marker_event(&ctx, &mut app, 36.0, 36.0, true, false, false);
    assert_eq!(pstate::<Option<IndentMarker>>(&ctx, "tabs-marker-sel"), Some(IndentMarker::Upper));
    assert!(current(&app).unwrap().1.is_empty());
    marker_event(&ctx, &mut app, 24.0, 24.0, false, false, false);
    assert_eq!(pstate::<Option<IndentMarker>>(&ctx, "tabs-marker-sel"), Some(IndentMarker::Lower));
    assert!(current(&app).unwrap().1.is_empty());
}

#[test]
fn upper_and_lower_marker_drags_map_indents_and_undo_once() {
    let mut app = indent_app(24.0, 12.0);
    let ctx = egui::Context::default();
    let before = app.session.active().unwrap().history.undo.len();
    marker_event(&ctx, &mut app, 36.0, 60.0, true, false, false);
    assert_eq!(indent_values(&app, 0), (24.0, 36.0));
    assert_eq!(app.session.active().unwrap().history.undo.len(), before + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(indent_values(&app, 0), (24.0, 12.0));
    marker_event(&ctx, &mut app, 24.0, 40.0, false, false, false);
    assert_eq!(indent_values(&app, 0), (40.0, -4.0));
    marker_event(&ctx, &mut app, 40.0, 50.0, false, true, false);
    assert_eq!(indent_values(&app, 0), (50.0, -4.0));
}

#[test]
fn marker_shift_drag_snaps_and_coincident_halves_remain_separate() {
    let mut app = indent_app(24.0, 0.0);
    let ctx = egui::Context::default();
    marker_event(&ctx, &mut app, 24.0, 33.4, true, false, true);
    assert_eq!(indent_values(&app, 0), (24.0, 11.0));
    app.run("text.setFormat", json!({"leftIndent": 24, "firstLineIndent": 0})).unwrap();
    marker_event(&ctx, &mut app, 24.0, 40.0, false, false, false);
    assert_eq!(indent_values(&app, 0), (40.0, -16.0));
}

#[test]
fn mixed_indent_upper_marker_keeps_each_left_indent_in_one_undo_step() {
    let mut app = indent_app(24.0, 12.0);
    let id = current(&app).unwrap().0.id.0;
    app.run("text.setFormat", json!({"id": id, "start": 4, "end": 4, "leftIndent": 60, "firstLineIndent": -10})).unwrap();
    let ctx = egui::Context::default();
    let before = app.session.active().unwrap().history.undo.len();
    marker_event(&ctx, &mut app, 36.0, 70.0, true, false, false);
    assert_eq!(indent_values(&app, 0), (24.0, 46.0));
    assert_eq!(indent_values(&app, 1), (60.0, 10.0));
    assert_eq!(app.session.active().unwrap().history.undo.len(), before + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(indent_values(&app, 0), (24.0, 12.0));
    assert_eq!(indent_values(&app, 1), (60.0, -10.0));
    marker_event(&ctx, &mut app, 60.0, 60.0, false, false, false);
    assert_ne!(pstate::<Option<IndentMarker>>(&ctx, "tabs-marker-sel"), Some(IndentMarker::Lower));

    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({})).unwrap();
    let a = app.run("text.create", json!({"x": 10, "y": 20, "text": "one", "area": {"width": 300, "height": 100}})).unwrap()["id"].clone();
    let b = app.run("text.create", json!({"x": 10, "y": 140, "text": "two", "area": {"width": 300, "height": 100}})).unwrap()["id"].clone();
    app.run("text.setFormat", json!({"id": a, "leftIndent": 24, "firstLineIndent": 12})).unwrap();
    app.run("text.setFormat", json!({"id": b, "leftIndent": 60, "firstLineIndent": -10})).unwrap();
    app.run("select.set", json!({"ids": [a, b.clone()]})).unwrap();
    let ctx = egui::Context::default();
    assert_eq!(current_para(&app).unwrap().1.left_indent, 24.0);
    marker_event(&ctx, &mut app, 36.0, 70.0, true, false, false);
    assert_eq!(indent_values(&app, 0), (24.0, 46.0));
    let second = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(b.as_u64().unwrap())).unwrap();
    let NodeKind::Text(second) = &second.kind else { panic!("second text") };
    assert_eq!((second.para_at(0).left_indent, second.para_at(0).first_line_indent), (60.0, 10.0));
}

#[test]
fn lower_marker_keeps_each_first_line_position_in_one_undo_step() {
    let mut app = indent_app(24.0, 12.0);
    let id = current(&app).unwrap().0.id.0;
    app.run("text.setFormat", json!({"id": id, "start": 4, "end": 7, "firstLineIndent": 28})).unwrap();
    assert_eq!(indent_values(&app, 0), (24.0, 12.0));
    assert_eq!(indent_values(&app, 1), (24.0, 28.0));
    let ctx = egui::Context::default();
    let before = app.session.active().unwrap().history.undo.len();
    marker_event(&ctx, &mut app, 24.0, 40.0, false, false, false);
    assert_eq!(indent_values(&app, 0), (40.0, -4.0));
    assert_eq!(indent_values(&app, 1), (40.0, 12.0));
    assert_eq!(app.session.active().unwrap().history.undo.len(), before + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(indent_values(&app, 0), (24.0, 12.0));
    assert_eq!(indent_values(&app, 1), (24.0, 28.0));
    marker_event(&ctx, &mut app, 24.0, 40.0, false, true, false);
    assert_eq!(indent_values(&app, 0), (40.0, 12.0));
    assert_eq!(indent_values(&app, 1), (40.0, 28.0));
}

#[test]
fn lower_marker_bounds_cover_each_touched_first_line() {
    let targets = [IndentTarget { id: 1, start: 0, left: 24.0, first_line: -20.0 }, IndentTarget { id: 1, start: 4, left: 24.0, first_line: 280.0 }];
    assert_eq!(bounded_marker_position(IndentMarker::Lower, 40.0, 300.0, &targets, true), Some(20.0));
    assert_eq!(bounded_marker_position(IndentMarker::Lower, 1200.0, 300.0, &targets, false), Some(300.0));
}

#[test]
fn hidden_lower_selection_returns_x_field_to_tab_stop() {
    let mut app = indent_app(24.0, 12.0);
    app.run("text.tabs.set", json!({"stops": [{"position": 100}]})).unwrap();
    let ctx = egui::Context::default();
    marker_event(&ctx, &mut app, 24.0, 24.0, false, false, false);
    assert_eq!(pstate::<Option<IndentMarker>>(&ctx, "tabs-marker-sel"), Some(IndentMarker::Lower));
    let id = current(&app).unwrap().0.id.0;
    app.run("text.setFormat", json!({"id": id, "start": 4, "end": 7, "leftIndent": 60})).unwrap();
    assert_eq!(indent_values(&app, 0), (24.0, 12.0));
    assert_eq!(indent_values(&app, 1), (60.0, 12.0));
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 400.0));
    let frame = |app: &mut VectorcraftApp, events| {
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
        out.shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) => Some((t.galley.text().to_string(), t.visual_bounding_rect())),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let texts = frame(&mut app, vec![]);
    let at = texts.iter().find(|(text, _)| text == "100 pt").unwrap_or_else(|| panic!("tab X field missing: {texts:?}")).1.center();
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    frame(&mut app, vec![egui::Event::PointerMoved(at), button(true)]);
    frame(&mut app, vec![button(false)]);
    let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    frame(&mut app, vec![key(egui::Key::A, egui::Modifiers::COMMAND)]);
    frame(&mut app, vec![egui::Event::Text("120".into())]);
    frame(&mut app, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
    assert_eq!(indent_values(&app, 0).0, 24.0);
    assert_eq!(indent_values(&app, 1).0, 60.0);
    assert_eq!(current(&app).unwrap().1[0].position, 120.0);
}

#[test]
fn selected_marker_position_field_sets_its_absolute_position() {
    let mut app = indent_app(24.0, 12.0);
    let ctx = egui::Context::default();
    marker_event(&ctx, &mut app, 36.0, 36.0, true, false, false);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 400.0));
    let field_frame = |app: &mut VectorcraftApp, events| {
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
        out.shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) => Some((t.galley.text().to_string(), t.visual_bounding_rect())),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let texts = field_frame(&mut app, vec![]);
    let at = texts.iter().find(|(text, _)| text == "36 pt").unwrap_or_else(|| panic!("36 pt in {texts:?}")).1.center();
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    field_frame(&mut app, vec![egui::Event::PointerMoved(at), button(true)]);
    field_frame(&mut app, vec![button(false)]);
    let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    field_frame(&mut app, vec![key(egui::Key::A, egui::Modifiers::COMMAND)]);
    field_frame(&mut app, vec![egui::Event::Text("80".into())]);
    field_frame(&mut app, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
    assert_eq!(indent_values(&app, 0), (24.0, 56.0));
    marker_event(&ctx, &mut app, 24.0, 24.0, false, false, false);
    let texts = field_frame(&mut app, vec![]);
    let at = texts.iter().find(|(text, _)| text == "24 pt").unwrap_or_else(|| panic!("24 pt in {texts:?}")).1.center();
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    field_frame(&mut app, vec![egui::Event::PointerMoved(at), button(true)]);
    field_frame(&mut app, vec![button(false)]);
    field_frame(&mut app, vec![key(egui::Key::A, egui::Modifiers::COMMAND)]);
    field_frame(&mut app, vec![egui::Event::Text("40".into())]);
    field_frame(&mut app, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
    assert_eq!(indent_values(&app, 0), (40.0, 40.0));
}

/// The ruler is numbered in the General unit (the document's units), with labels at least 50 px
/// apart as on the canvas rulers.
#[test]
fn ruler_is_numbered_in_the_general_unit() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({})).unwrap();
    app.session.execute("text.create", &json!({"x": 10, "y": 20, "text": "a	b", "area": {"width": 300, "height": 100}})).unwrap();
    let ctx = egui::Context::default();
    assert_eq!(ruler_labels(&ctx, &mut app), ["0", "50", "100", "150", "200", "250", "300"]);
    app.session.execute("document.setUnits", &json!({"units": "in"})).unwrap();
    assert_eq!(ruler_labels(&ctx, &mut app), ["0", "1", "2", "3", "4"]);
    app.session.execute("document.setUnits", &json!({"units": "mm"})).unwrap();
    assert_eq!(ruler_labels(&ctx, &mut app), ["0", "25", "50", "75", "100"]);
}

/// Snap to Unit, chosen from the panel menu with or without text selected, puts a stop dragged
/// or placed by a click on the ruler on its nearest tick (every 5 pt on this points ruler).
/// Chosen again, it turns off.
#[test]
fn snap_to_unit_puts_dragged_and_clicked_stops_on_the_nearest_tick() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({})).unwrap();
    let ctx = egui::Context::default();
    let row = |app: &mut VectorcraftApp| {
        let rows = menu_frame(&ctx, app, vec![]);
        rows.into_iter().find(|(t, _)| t.trim_start_matches(['✓', ' ']) == "Snap to Unit").unwrap_or_else(|| panic!("Snap to Unit"))
    };
    choose(&ctx, &mut app, "Snap to Unit");
    assert_eq!(row(&mut app).0, "✓ Snap to Unit");
    app.session.execute("text.create", &json!({"x": 10, "y": 20, "text": "a	b", "area": {"width": 300, "height": 100}})).unwrap();
    app.session.execute("text.tabs.set", &json!({"stops": [{"position": 50}]})).unwrap();
    assert_eq!(ruler_labels(&ctx, &mut app)[..2], ["0", "50"]);
    let positions = |app: &VectorcraftApp| current(app).unwrap().1.iter().map(|s| s.position).collect::<Vec<_>>();
    drag_on_ruler(&ctx, &mut app, 50.0, 13.4, false);
    assert_eq!(positions(&app), [15.0]);
    // A click at 33.4 pt adds a stop at 35.
    drag_on_ruler(&ctx, &mut app, 33.4, 33.4, false);
    assert_eq!(positions(&app), [15.0, 35.0]);
    assert_eq!(row(&mut app).0, "✓ Snap to Unit");
    // Chosen again, Snap to Unit turns off and the stop lands on the whole point nearest where it
    // is dropped.
    let at = row(&mut app).1.center();
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    menu_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(at), button(true)]);
    menu_frame(&ctx, &mut app, vec![button(false)]);
    assert_eq!(row(&mut app).0.trim_start(), "Snap to Unit");
    drag_on_ruler(&ctx, &mut app, 35.0, 23.4, false);
    assert_eq!(positions(&app), [15.0, 23.0]);
}

/// A snapped click more than 5 px from a stop, nearest to the tick that stop is on, selects the
/// stop and records no undo step. On this millimeters ruler the ticks are 2.5 mm (7.09 pt, 12.4
/// px) apart.
#[test]
fn a_snapped_click_onto_a_stop_selects_it() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({})).unwrap();
    app.session.execute("document.setUnits", &json!({"units": "mm"})).unwrap();
    app.session.execute("text.create", &json!({"x": 10, "y": 20, "text": "a	b", "area": {"width": 224, "height": 100}})).unwrap();
    app.session.execute("text.tabs.set", &json!({"stops": [{"position": 100}]})).unwrap();
    let ctx = egui::Context::default();
    assert_eq!(ruler_labels(&ctx, &mut app), ["0", "25", "50", "75"]);
    set_pstate(&ctx, "tabs-snap", true);
    let tick = 2.5 * 72.0 / 25.4;
    drag_on_ruler(&ctx, &mut app, 28.4, 28.4, false);
    let positions = |app: &VectorcraftApp| current(app).unwrap().1.iter().map(|s| s.position).collect::<Vec<_>>();
    assert_eq!(positions(&app).len(), 2);
    assert!((positions(&app)[0] - 4.0 * tick).abs() < 1e-9, "{:?}", positions(&app));
    set_pstate(&ctx, "tabs-sel", 1usize);
    let steps = app.session.active().unwrap().history.undo.len();
    // The click is 3.3 pt (5.8 px) right of the stop, past the 5 px hit tolerance and nearest its tick.
    drag_on_ruler(&ctx, &mut app, 4.0 * tick + 3.3, 4.0 * tick + 3.3, false);
    assert_eq!(positions(&app).len(), 2, "{:?}", positions(&app));
    assert_eq!(pstate::<usize>(&ctx, "tabs-sel"), 0);
    assert_eq!(app.session.active().unwrap().history.undo.len(), steps, "no undo step");
}

/// With Snap to Unit off, Shift held during the drag puts the stop on the ruler's nearest tick.
#[test]
fn shift_drag_snaps_a_stop_to_the_nearest_tick() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({})).unwrap();
    app.session.execute("text.create", &json!({"x": 10, "y": 20, "text": "a	b", "area": {"width": 300, "height": 100}})).unwrap();
    app.session.execute("text.tabs.set", &json!({"stops": [{"position": 50}]})).unwrap();
    let ctx = egui::Context::default();
    let position = |app: &VectorcraftApp| current(app).unwrap().1[0].position;
    drag_on_ruler(&ctx, &mut app, 50.0, 13.4, false);
    assert!((position(&app) - 13.4).abs() < 1.0, "{}", position(&app));
    let from = position(&app);
    drag_on_ruler(&ctx, &mut app, from, 41.6, true);
    assert_eq!(position(&app), 40.0);
    // In an inches document the ticks divide the inch in half, not in ten (#2827), so the
    // nearest one to 100 pt is 99 pt -- 1 3/8 in, an eighth of an inch from 1 1/4.
    app.session.execute("document.setUnits", &json!({"units": "in"})).unwrap();
    drag_on_ruler(&ctx, &mut app, 40.0, 100.0, true);
    assert!((position(&app) - 99.0).abs() < 1e-9, "{}", position(&app));
    assert_eq!((position(&app) / 72.0 * 8.0).round(), position(&app) / 72.0 * 8.0, "a binary fraction of an inch");
}

/// One frame of the panel with `events` → the texts painted and where.
fn panel_frame(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>) -> Vec<(String, egui::Rect)> {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 400.0));
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| show(app, ui));
    out.textures_delta.clear();
    out.shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) => Some((t.galley.text().to_string(), t.visual_bounding_rect())),
            _ => None,
        })
        .collect()
}

/// A stop dragged past another stays selected once the stops are sorted, so the X field shows
/// where it was dropped.
#[test]
fn a_stop_dragged_past_another_stays_selected() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({})).unwrap();
    app.session.execute("text.create", &json!({"x": 10, "y": 20, "text": "a	b", "area": {"width": 300, "height": 100}})).unwrap();
    app.session.execute("text.tabs.set", &json!({"stops": [{"position": 30}, {"position": 50}]})).unwrap();
    let ctx = egui::Context::default();
    drag_on_ruler(&ctx, &mut app, 50.0, 10.0, false);
    let stops = current(&app).unwrap().1;
    assert_eq!(stops.len(), 2);
    assert!((stops[0].position - 10.0).abs() < 1.0 && stops[1].position == 30.0, "{stops:?}");
    assert_eq!(pstate::<usize>(&ctx, "tabs-sel"), 0);
    let shown = app.session.general_unit().format(stops[0].position);
    let texts = panel_frame(&ctx, &mut app, vec![]);
    assert!(texts.iter().any(|(t, _)| *t == shown), "{shown} in {texts:?}");
}

/// A stop placed by a click on the ruler or dragged on it lands on the nearest whole point, and
/// the moved stop stays selected. A position typed in the X field keeps its hundredths of a point.
#[test]
fn the_ruler_places_and_moves_stops_on_whole_points() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({})).unwrap();
    app.session.execute("text.create", &json!({"x": 10, "y": 20, "text": "a	b", "area": {"width": 300, "height": 100}})).unwrap();
    app.session.execute("text.tabs.set", &json!({"stops": [{"position": 50}]})).unwrap();
    let ctx = egui::Context::default();
    let positions = |app: &VectorcraftApp| current(app).unwrap().1.iter().map(|s| s.position).collect::<Vec<_>>();
    // A click at 33.4 pt adds a stop at 33 pt and selects it.
    drag_on_ruler(&ctx, &mut app, 33.4, 33.4, false);
    assert_eq!(positions(&app), [33.0, 50.0]);
    assert_eq!(pstate::<usize>(&ctx, "tabs-sel"), 0);
    // The stop at 50 pt dragged to 13.6 pt lands on 14 pt, left of the stop at 33 pt, and stays selected.
    drag_on_ruler(&ctx, &mut app, 50.0, 13.6, false);
    assert_eq!(positions(&app), [14.0, 33.0]);
    assert_eq!(pstate::<usize>(&ctx, "tabs-sel"), 0);
    // 13.47 typed in the X field keeps its hundredths.
    let texts = panel_frame(&ctx, &mut app, vec![]);
    let at = texts.iter().find(|(t, _)| t == "14 pt").unwrap_or_else(|| panic!("14 pt in {texts:?}")).1.center();
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    panel_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(at), button(true)]);
    panel_frame(&ctx, &mut app, vec![button(false)]);
    panel_frame(&ctx, &mut app, vec![key(egui::Key::A, egui::Modifiers::COMMAND)]);
    panel_frame(&ctx, &mut app, vec![egui::Event::Text("13.47".into())]);
    panel_frame(&ctx, &mut app, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
    assert_eq!(positions(&app), [13.47, 33.0]);
    assert_eq!(pstate::<usize>(&ctx, "tabs-sel"), 0);
}

/// An X typed past the next stop keeps the stop selected once the stops are sorted. An X the
/// stops cannot take (past 100,000 pt) leaves the stops and the selection as they were.
#[test]
fn an_x_typed_past_the_next_stop_keeps_the_stop_selected() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({})).unwrap();
    app.session.execute("text.create", &json!({"x": 10, "y": 20, "text": "a	b", "area": {"width": 300, "height": 100}})).unwrap();
    app.session.execute("text.tabs.set", &json!({"stops": [{"position": 30}, {"position": 50}]})).unwrap();
    let ctx = egui::Context::default();
    let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    // Click the X field showing `shown`, type `typed` over it and press Enter → the texts painted next.
    let type_x = |app: &mut VectorcraftApp, shown: &str, typed: &str| {
        let texts = panel_frame(&ctx, app, vec![]);
        let at = texts.iter().find(|(t, _)| t == shown).unwrap_or_else(|| panic!("{shown} in {texts:?}")).1.center();
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        panel_frame(&ctx, app, vec![egui::Event::PointerMoved(at), button(true)]);
        panel_frame(&ctx, app, vec![button(false)]);
        panel_frame(&ctx, app, vec![key(egui::Key::A, egui::Modifiers::COMMAND)]);
        panel_frame(&ctx, app, vec![egui::Event::Text(typed.into())]);
        panel_frame(&ctx, app, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
        panel_frame(&ctx, app, vec![])
    };
    let positions = |app: &VectorcraftApp| current(app).unwrap().1.iter().map(|s| s.position).collect::<Vec<_>>();
    set_pstate(&ctx, "tabs-sel", 0usize);
    let texts = type_x(&mut app, "30 pt", "70");
    assert_eq!(positions(&app), [50.0, 70.0]);
    assert_eq!(pstate::<usize>(&ctx, "tabs-sel"), 1);
    assert!(texts.iter().any(|(t, _)| t == "70 pt"), "70 pt in {texts:?}");
    set_pstate(&ctx, "tabs-sel", 0usize);
    let texts = type_x(&mut app, "50 pt", "200000");
    assert_eq!(positions(&app), [50.0, 70.0]);
    assert_eq!(pstate::<usize>(&ctx, "tabs-sel"), 0);
    assert!(texts.iter().any(|(t, _)| t == "50 pt"), "50 pt in {texts:?}");
    // An X less than 1e-6 pt above another stop merges the two into the stop already there,
    // which is selected.
    app.session.execute("text.tabs.set", &json!({"stops": [{"position": 30}, {"position": 50}, {"position": 70}]})).unwrap();
    set_pstate(&ctx, "tabs-sel", 0usize);
    type_x(&mut app, "30 pt", "50.0000001");
    assert_eq!(positions(&app), [50.0, 70.0]);
    assert_eq!(pstate::<usize>(&ctx, "tabs-sel"), 0);
}

/// One frame of the panel with `events` → the stops' stems on the ruler, left to right, with
/// their colors.
fn stems_frame(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>) -> Vec<(f32, egui::Color32)> {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 400.0));
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| show(app, ui));
    out.textures_delta.clear();
    out.shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::LineSegment { points: [a, b], stroke } if stroke.width == 1.5 && a.x == b.x => Some((a.x, stroke.color)),
            _ => None,
        })
        .collect()
}

/// In the frame where Enter commits an X the stops cannot take (past 100,000 pt), the ruler
/// shows the stops and the selected stop as they were.
#[test]
fn a_refused_x_leaves_the_ruler_as_it_was() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({})).unwrap();
    app.session.execute("text.create", &json!({"x": 10, "y": 20, "text": "a	b", "area": {"width": 300, "height": 100}})).unwrap();
    app.session.execute("text.tabs.set", &json!({"stops": [{"position": 30}, {"position": 50}]})).unwrap();
    let ctx = egui::Context::default();
    set_pstate(&ctx, "tabs-sel", 1usize);
    let before = stems_frame(&ctx, &mut app, vec![]);
    assert_eq!(before.len(), 2, "{before:?}");
    let texts = panel_frame(&ctx, &mut app, vec![]);
    let at = texts.iter().find(|(t, _)| t == "50 pt").unwrap_or_else(|| panic!("50 pt in {texts:?}")).1.center();
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    panel_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(at), button(true)]);
    panel_frame(&ctx, &mut app, vec![button(false)]);
    panel_frame(&ctx, &mut app, vec![key(egui::Key::A, egui::Modifiers::COMMAND)]);
    panel_frame(&ctx, &mut app, vec![egui::Event::Text("200000".into())]);
    let committed = stems_frame(&ctx, &mut app, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
    assert_eq!(current(&app).unwrap().1.iter().map(|s| s.position).collect::<Vec<_>>(), [30.0, 50.0]);
    assert_eq!(pstate::<usize>(&ctx, "tabs-sel"), 1);
    assert_eq!(committed, before);
}

/// One frame of the panel menu with `events` → the rows' texts and where they are painted.
fn menu_frame(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>) -> Vec<(String, egui::Rect)> {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(320.0, 400.0));
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| menu(app, ui));
    out.textures_delta.clear();
    out.shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) => Some((t.galley.text().to_string(), t.visual_bounding_rect())),
            _ => None,
        })
        .collect()
}

/// Click the panel menu's row `label`, which shows no check mark.
fn choose(ctx: &egui::Context, app: &mut VectorcraftApp, label: &str) {
    let rows = menu_frame(ctx, app, vec![]);
    let (text, r) = rows.iter().find(|(t, _)| t.trim_start_matches(['✓', ' ']) == label).unwrap_or_else(|| panic!("{label} in {rows:?}"));
    assert!(!text.contains('✓'), "{label} shows a check mark");
    let at = r.center();
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    menu_frame(ctx, app, vec![egui::Event::PointerMoved(at), button(true)]);
    menu_frame(ctx, app, vec![button(false)]);
}

/// The panel menu's Repeat Tab, Delete Tab and Clear All Tabs can be chosen while the text has
/// tab stops and show no check mark. Repeat Tab and Delete Tab act on the stop selected on the
/// ruler, and Clear All Tabs removes every stop.
#[test]
fn panel_menu_repeats_deletes_and_clears_tab_stops() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({})).unwrap();
    app.session.execute("text.create", &json!({"x": 10, "y": 20, "text": "a	b", "area": {"width": 300, "height": 100}})).unwrap();
    app.session.execute("text.tabs.set", &json!({"stops": [{"position": 30}, {"position": 50, "leader": "."}]})).unwrap();
    let ctx = egui::Context::default();
    let positions = |app: &VectorcraftApp| current(app).unwrap().1.iter().map(|s| s.position).collect::<Vec<_>>();
    // Repeat Tab with the second stop selected: 20 copies of it, 20 pt apart (its distance from
    // the first stop), leader and all.
    set_pstate(&ctx, "tabs-sel", 1usize);
    choose(&ctx, &mut app, "Repeat Tab");
    let repeated = positions(&app);
    assert_eq!((repeated.len(), &repeated[..4]), (22, &[30.0, 50.0, 70.0, 90.0][..]));
    assert!(current(&app).unwrap().1[2..].iter().all(|s| s.leader == "."));
    // Delete Tab removes the selected stop.
    choose(&ctx, &mut app, "Delete Tab");
    assert_eq!(positions(&app)[..3], [30.0, 70.0, 90.0]);
    // Clear All Tabs removes every stop, in one undo step of its own name.
    choose(&ctx, &mut app, "Clear All Tabs");
    assert!(positions(&app).is_empty());
    assert_eq!(app.session.active().unwrap().history.undo.last().unwrap().label, "Clear All Tabs");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(positions(&app).len(), 21);
}

#[test]
fn panel_menu_does_not_change_stops_while_an_indent_marker_is_selected() {
    let mut app = indent_app(24.0, 12.0);
    app.session.execute("text.tabs.set", &json!({"stops": [{"position": 50}, {"position": 70}]})).unwrap();
    let ctx = egui::Context::default();
    set_pstate(&ctx, "tabs-sel", 1usize);
    marker_event(&ctx, &mut app, 36.0, 36.0, true, false, false);
    assert_eq!(pstate::<Option<IndentMarker>>(&ctx, "tabs-marker-sel"), Some(IndentMarker::Upper));
    let before = current(&app).unwrap().1;

    choose(&ctx, &mut app, "Repeat Tab");
    assert_eq!(current(&app).unwrap().1, before);
    choose(&ctx, &mut app, "Delete Tab");
    assert_eq!(current(&app).unwrap().1, before);
}

/// Repeat Tab on the first stop spaces the copies by the stop's distance from the paragraph's
/// left indent, from the paragraph the caret is in while the Type tool edits text.
#[test]
fn repeat_tab_measures_the_first_stop_from_the_left_indent() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({})).unwrap();
    app.session.execute("text.create", &json!({"x": 10, "y": 20, "text": "a	b\nc	d", "area": {"width": 300, "height": 100}})).unwrap();
    app.session.execute("text.setFormat", &json!({"leftIndent": 20})).unwrap();
    app.session.execute("text.tabs.set", &json!({"stops": [{"position": 50}]})).unwrap();
    let ctx = egui::Context::default();
    let positions = |app: &VectorcraftApp| current(app).unwrap().1.iter().map(|s| s.position).collect::<Vec<_>>();
    // 30 pt from the indent at 20: copies at 80, 110, ...
    set_pstate(&ctx, "tabs-sel", 0usize);
    choose(&ctx, &mut app, "Repeat Tab");
    let repeated = positions(&app);
    assert_eq!((repeated.len(), &repeated[..4]), (21, &[50.0, 80.0, 110.0, 140.0][..]));
    // A stop left of the indent: 15 pt from the indent at 20.
    app.session.execute("text.tabs.set", &json!({"stops": [{"position": 5}]})).unwrap();
    choose(&ctx, &mut app, "Repeat Tab");
    assert_eq!(positions(&app)[..4], [5.0, 20.0, 35.0, 50.0]);
    // The Type tool's caret in the second paragraph, indented 40 pt: its stop at 50 repeats
    // every 10 pt, and the first paragraph keeps its stop.
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({})).unwrap();
    app.session.select_tool("type", Default::default()).unwrap();
    let id = app.run("text.create", json!({"x": 10, "y": 20, "text": "a	b\nc	d"})).unwrap()["id"].clone();
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    app.run("text.setFormat", json!({"id": id, "leftIndent": 20})).unwrap();
    app.run("text.setFormat", json!({"id": id, "start": 4, "end": 7, "leftIndent": 40})).unwrap();
    app.run("text.tabs.set", json!({"id": id, "stops": [{"position": 50}]})).unwrap();
    app.session.set_tool_option("select", &json!({"start": 5, "end": 5}));
    choose(&ctx, &mut app, "Repeat Tab");
    assert_eq!(positions(&app)[..3], [50.0, 60.0, 70.0]);
    app.session.set_tool_option("select", &json!({"start": 0, "end": 0}));
    assert_eq!(positions(&app), [50.0]);
}

/// After Delete Tab removes the last stop, the ruler shows the stop before it selected, and
/// Delete Tab removes that one next.
#[test]
fn delete_tab_after_the_last_stop_deletes_the_stop_the_ruler_shows_selected() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({})).unwrap();
    app.session.execute("text.create", &json!({"x": 10, "y": 20, "text": "a	b", "area": {"width": 300, "height": 100}})).unwrap();
    app.session.execute("text.tabs.set", &json!({"stops": [{"position": 30}, {"position": 50}]})).unwrap();
    let ctx = egui::Context::default();
    set_pstate(&ctx, "tabs-sel", 1usize);
    choose(&ctx, &mut app, "Delete Tab");
    assert_eq!(current(&app).unwrap().1.len(), 1);
    choose(&ctx, &mut app, "Delete Tab");
    assert!(current(&app).unwrap().1.is_empty());
}

/// Position Panel Above Text floats the panel over the text, its ruler's 0 on the frame's left
/// edge and a stop's mark over the column it sets, at any zoom (#729).
#[test]
fn above_text_puts_the_ruler_over_the_text_at_the_zoom() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    app.session.execute("text.create", &json!({"x": 40, "y": 200, "text": "a	b", "area": {"width": 300, "height": 100}})).unwrap();
    app.canvas_rect = Some(egui::Rect::from_min_size(egui::pos2(50.0, 40.0), vec2(1000.0, 700.0)));
    for zoom in [1.0, 2.0] {
        if let Some(v) = app.view_mut() {
            v.zoom = zoom;
        }
        let (node, _, span) = current(&app).unwrap();
        let ctx = egui::Context::default();
        above_text(&mut app, &ctx, &node, span).unwrap();
        let layout = app.ui.docking.clone().unwrap();
        let g = layout.floating.iter().find(|w| w.contains(&"tabs".to_string())).unwrap();
        let xf = crate::canvas::Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let frame = xf.rect_to_screen(node.geometric_bounds().unwrap());
        assert!((g.rect[0] + RULER_LEFT - frame.left()).abs() < 0.01, "0 on the frame's left edge");
        assert!(g.rect[1] < frame.top(), "above the text");
        // The ruler (the panel less its margins) spans the frame at this zoom.
        let ruler = g.rect[2] - RULER_LEFT * 2.0;
        assert!((ruler - frame.width()).abs() < 0.01, "{ruler} {}", frame.width());
        assert!((ruler - 300.0 * zoom as f32).abs() < 0.01);
    }
    assert_eq!(app.ui.docking.unwrap().floating.len(), 1, "placed again, not floated twice");
}
