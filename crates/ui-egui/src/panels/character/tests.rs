use super::*;
use vectorcraft_engine::Session;

#[test]
fn scaled_dimensions_do_not_show_false_mixed_values_from_roundoff() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let a = app.run("text.create", json!({"x": 20, "y": 50, "text": "Axis", "size": 12})).unwrap()["id"].clone();
    app.run("object.scale", json!({"sx": 120, "sy": 110})).unwrap();
    let b = app.run("text.create", json!({"x": 20, "y": 90, "text": "Label", "size": 24})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    app.run("text.setStyle", json!({"size": 7, "leading": 9})).unwrap();
    app.run("text.setFormat", json!({"hScale": 130, "baselineShift": 3})).unwrap();
    for (actual, expected) in [
        (shared_effective(&app, |s| s.size), 7.0),
        (shared_effective(&app, CharStyle::effective_leading), 9.0),
        (shared_effective(&app, |s| s.h_scale), 130.0),
        (shared_effective(&app, |s| s.baseline_shift), 3.0),
    ] {
        assert!((actual.expect("equal dimensions must not be mixed") - expected).abs() < 1e-8);
    }
}

/// A final empty paragraph counts: with the object selected, its Auto Leading differing from
/// the paragraph before it shows Leading as mixed.
#[test]
fn leading_field_counts_the_final_empty_paragraph() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "One\n", "size": 10})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    assert!((shared_leading(&app).unwrap() - 12.0).abs() < 1e-9);
    app.run("text.setFormat", json!({"autoLeading": 200, "start": 4, "end": 4})).unwrap();
    assert_eq!(shared_leading(&app), None, "the empty last paragraph differs");
}

/// Auto leading shows the leading the paragraph's Justification › Auto Leading sets.
#[test]
fn leading_field_shows_the_paragraph_auto_leading() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "One\nTwo", "size": 10})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    assert!((shared_leading(&app).unwrap() - 12.0).abs() < 1e-9);
    app.run("text.setFormat", json!({"autoLeading": 200, "start": 5, "end": 5})).unwrap();
    assert_eq!(shared_leading(&app), None, "the paragraphs differ");
    assert_eq!(shared(&app, |s| s.leading.is_none()), Some(true), "still Auto");
    app.run("text.setFormat", json!({"autoLeading": 200})).unwrap();
    assert!((shared_leading(&app).unwrap() - 20.0).abs() < 1e-9);
    app.run("text.setStyle", json!({"leading": 15})).unwrap();
    assert!((shared_leading(&app).unwrap() - 15.0).abs() < 1e-9, "a set leading is its own");
}

#[test]
fn scaled_collapsed_text_keeps_nondimensional_controls_available() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    app.run("text.create", json!({"x": 20, "y": 50, "text": "Axis", "size": 12})).unwrap();
    app.run("object.transform", json!({"matrix": [0, 0, 0, 0, 0, 0]})).unwrap();
    assert!(text_style(&app).is_some());
    assert_eq!(shared(&app, |s| s.tracking), Some(0.0));
    assert_eq!(shared_effective(&app, |s| s.size), None);
    app.run("text.setStyle", json!({"tracking": 40})).unwrap();
    app.run("text.setFormat", json!({"leftIndent": 4})).unwrap();
    assert_eq!(shared(&app, |s| s.tracking), Some(40.0));
    assert_eq!(text_style(&app).unwrap().1.left_indent, 4.0);
    let ctx = egui::Context::default();
    let texts = super::super::tests_appearance::frame_events(&ctx, &mut app, vec![], show);
    assert!(!texts.iter().any(|(s, _)| s == "No text selected"));
}

#[test]
fn scaled_character_fields_show_document_sizes_and_mixed_group_values() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let a = app.run("text.create", json!({"x": 20, "y": 50, "text": "Axis", "size": 12})).unwrap()["id"].clone();
    app.run("object.scale", json!({"sx": 200})).unwrap();
    assert_eq!(shared_effective(&app, |s| s.size), Some(24.0));
    assert_eq!(text_style(&app).unwrap().0.size, 12.0, "style definitions keep stored attributes");
    let b = app.run("text.create", json!({"x": 20, "y": 90, "text": "Label", "size": 24})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    app.run("object.group", json!({})).unwrap();
    assert_eq!(shared_effective(&app, |s| s.size), Some(24.0), "different local sizes, same displayed size");
    app.run("object.scale", json!({"sx": 200, "sy": 100})).unwrap();
    assert_eq!(shared_effective(&app, |s| s.h_scale), Some(200.0));
    app.run("text.setStyle", json!({"size": 10})).unwrap();
    assert_eq!(shared_effective(&app, |s| s.size), Some(10.0));
    app.run("text.setFormat", json!({"hScale": 100})).unwrap();
    assert_eq!(shared_effective(&app, |s| s.h_scale), Some(100.0));
}

#[test]
fn scaled_range_fields_follow_the_type_tool_selection() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    app.session.select_tool("type", Default::default()).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "Axis label", "size": 12})).unwrap()["id"].clone();
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    assert!(text_editing(&app).is_some());
    app.run("object.scale", json!({"sx": 200})).unwrap();
    app.run("text.setRangeStyle", json!({"id": id, "start": 0, "end": 4, "size": 10})).unwrap();
    app.session.set_tool_option("selectAll", &json!(true));
    assert_eq!(shared_effective(&app, |s| s.size), None);
    app.session.set_tool_option("select", &json!({"start": 0, "end": 4}));
    assert_eq!(shared_effective(&app, |s| s.size), Some(10.0));
    assert_eq!(text_style(&app).unwrap().0.size, 5.0);
}

/// The Kerning field at a caret kerns that pair, not the whole text (#2217).
#[test]
fn kerning_at_a_caret_sets_the_pair_not_the_whole_text() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    app.session.select_tool("type", Default::default()).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "AVAX", "size": 12})).unwrap()["id"].clone();
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    app.session.set_tool_option("select", &json!({"start": 2, "end": 2}));
    set_kerning(&mut app, json!(-50));
    let kerning = |app: &VectorcraftApp, at: usize| {
        let st = app.session.active().unwrap();
        let Some(NodeKind::Text(t)) = st.doc.node(NodeId(id.as_u64().unwrap())).map(|n| &n.kind) else { panic!("text") };
        vectorcraft_text::edit::style_at(&t.runs, at + 1).kerning // the character at `at`
    };
    assert_eq!(kerning(&app, 1), Some(-50.0), "the character before the caret");
    assert_eq!(kerning(&app, 3), None, "the rest is untouched");
    assert_eq!(kerning(&app, 0), None);
    // A selected range still takes it whole.
    app.session.set_tool_option("select", &json!({"start": 2, "end": 4}));
    set_kerning(&mut app, json!(30));
    assert_eq!((kerning(&app, 1), kerning(&app, 2), kerning(&app, 3)), (Some(-50.0), Some(30.0), Some(30.0)));
}

/// Highlight Kerning and Highlight Tracking show the Character panel and give its field the
/// keyboard focus with the value selected: what is typed replaces it, and Enter applies it.
/// Highlight Font Name opens the font family menu with its search field focused. Without
/// selected type the panel shows and nothing takes the focus.
#[test]
fn highlight_commands_focus_the_font_kerning_and_tracking_fields() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "AVAX", "size": 12})).unwrap()["id"].clone();
    let ctx = egui::Context::default();
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
    };
    let typed = |text: &str| {
        let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        vec![egui::Event::Text(text.into()), enter]
    };
    let spacing = |app: &VectorcraftApp| text_style(app).map(|(s, _)| (s.kerning, s.tracking)).unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    assert_eq!(app.run("type.highlightKerning", json!({})).unwrap(), json!({"field": "kerning"}));
    assert!(crate::panel_docking::showing(&app.ui, "character"), "the panel shows");
    frame(&mut app, vec![]);
    assert_eq!(app.character_focus, None, "the Kerning field took the focus");
    frame(&mut app, typed("-40"));
    assert_eq!(spacing(&app), (Some(-40.0), 0.0), "typing replaced the kerning");
    app.run("type.highlightTracking", json!({})).unwrap();
    frame(&mut app, vec![]);
    frame(&mut app, typed("75"));
    assert_eq!(spacing(&app), (Some(-40.0), 75.0), "typing replaced the tracking");
    // Highlighting a field again while it holds typed text that is not applied selects all of it.
    app.run("type.highlightKerning", json!({})).unwrap();
    frame(&mut app, vec![]);
    frame(&mut app, vec![egui::Event::Text("12345".into())]);
    app.run("type.highlightKerning", json!({})).unwrap();
    frame(&mut app, vec![]);
    frame(&mut app, typed("-20"));
    assert_eq!(spacing(&app), (Some(-20.0), 75.0), "typing replaced the text typed before");
    ctx.memory_mut(|m| m.stop_text_input());
    app.run("type.highlightFontName", json!({})).unwrap();
    frame(&mut app, vec![]);
    frame(&mut app, vec![]);
    assert!(egui::Popup::is_any_open(&ctx), "the font family menu is open");
    assert!(ctx.memory(|m| m.focused()).is_some(), "its search field has the focus");
    // No type selected: the panel shows, the command fails and nothing is armed.
    app.run("select.none", json!({})).unwrap();
    assert!(app.run("type.highlightKerning", json!({})).is_err());
    assert_eq!(app.character_focus, None);
}

/// A variable font gets the Variable Font button, which opens one slider per axis named as the
/// font names it; a drag along one sets that axis of the selected type in one undo step.
#[test]
fn variable_fonts_get_a_slider_per_axis() {
    use vectorcraft_text::test_fonts::{TWO_AXIS_FAMILY, two_axis_font};
    let db = vectorcraft_text::FontDb::global();
    db.add_font(two_axis_font().unwrap());
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let plain = app.run("text.create", json!({"x": 20, "y": 50, "text": "Static"})).unwrap()["id"].clone();
    let id = app.run("text.create", json!({"x": 20, "y": 120, "text": "lIn", "font": TWO_AXIS_FAMILY})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [plain]})).unwrap();
    assert!(!crate::tests_labels::painted_text(&mut app, show).contains("Variable Font"), "a static font");
    app.run("select.set", json!({"ids": [id]})).unwrap();
    assert!(crate::tests_labels::painted_text(&mut app, show).contains("Variable Font"));
    let face = db.face(TWO_AXIS_FAMILY, "Regular").unwrap();
    let text = crate::tests_labels::painted_text(&mut app, |app, ui| axis_sliders(app, ui, &face));
    assert!(text.contains("Weight\n") && text.contains("Width\n") && text.contains("400") && text.contains("100"), "{text}");
    // The labels are the font's names in every interface language, not catalog words (ja
    // translates the interface's Weight as the stroke weight).
    let ja = crate::i18n::Lang::from_code("ja").unwrap();
    crate::i18n::set_current(ja);
    let text = crate::tests_labels::painted_text(&mut app, |app, ui| axis_sliders(app, ui, &face));
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let weight = crate::i18n::tr(ja, "Weight");
    assert!(text.contains("Weight\n") && text.contains("Width\n") && (weight == "Weight" || !text.contains(weight)), "{text}");
    // Screen readers name each slider by its axis.
    let tree = crate::widgets::accessibility_tests::tree(|ui| axis_sliders(&mut app, ui, &face));
    for name in ["Weight", "Width"] {
        assert!(tree.iter().any(|n| n.role() == egui::accesskit::Role::Slider && n.label() == Some(name)), "{name}");
    }
    // ...with the number its field shows, on the axis's range.
    use crate::widgets::accessibility_tests::slider_value;
    let wght = face.axes().into_iter().find(|a| &a.tag == b"wght").unwrap();
    let range = (Some(f64::from(wght.min)), Some(f64::from(wght.max)));
    assert_eq!(slider_value(&tree, "Weight"), (Some(400.0), range.0, range.1, Some("400".into())));
    // Drag the Weight slider from its start to past its end.
    let ctx = egui::Context::default();
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let mut slider = None;
        let input = egui::RawInput { events, ..Default::default() };
        ctx.run_ui(input, |ui| {
            axis_sliders(app, ui, &face);
            slider = ctx.read_response(ui.scope_id().with(("ch-axis", *b"wght"))).map(|r| r.rect);
        })
        .textures_delta
        .clear();
        slider
    };
    frame(&mut app, vec![]);
    let r = frame(&mut app, vec![]).expect("the Weight slider is drawn");
    let none = egui::Modifiers::NONE;
    let (from, to) = (egui::pos2(r.left() + 4.0, r.center().y), egui::pos2(r.right() + 20.0, r.center().y));
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: none };
    frame(&mut app, vec![egui::Event::PointerMoved(from), button(from, true)]);
    for i in 1..=4 {
        frame(&mut app, vec![egui::Event::PointerMoved(from + (to - from) * i as f32 / 4.0)]);
    }
    frame(&mut app, vec![button(to, false)]);
    frame(&mut app, vec![]);
    let axes = |app: &VectorcraftApp| shared(app, |s| s.font_axes.clone()).unwrap();
    assert_eq!(axes(&app).into_iter().collect::<Vec<_>>(), [("wght".to_string(), 700.0)]);
    app.run("edit.undo", json!({})).unwrap();
    assert!(axes(&app).is_empty(), "one undo step");
    // Regular and Bold selected: the weights differ and the field is blank; the widths agree.
    let bold = app.run("text.create", json!({"x": 20, "y": 160, "text": "lIn", "font": TWO_AXIS_FAMILY, "style": "Bold"})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [id, bold]})).unwrap();
    let text = crate::tests_labels::painted_text(&mut app, |app, ui| axis_sliders(app, ui, &face));
    assert!(!text.contains("400") && !text.contains("700") && text.contains("100"), "{text}");
    let tree = crate::widgets::accessibility_tests::tree(|ui| axis_sliders(&mut app, ui, &face));
    assert_eq!(slider_value(&tree, "Weight"), (None, None, None, None), "a blank field: no value");
    assert_eq!(slider_value(&tree, "Width").0, Some(100.0));
    // Reset Panel leaves the settings, as it leaves the font and style.
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.run("text.setFormat", json!({"fontAxes": {"wght": 650}})).unwrap();
    reset_panel(&mut app);
    assert_eq!(axes(&app).into_iter().collect::<Vec<_>>(), [("wght".to_string(), 650.0)]);
}

/// Bold and Italic pick the family's own faces by name, and toggle back (#724).
#[test]
fn bold_and_italic_pick_the_familys_own_faces() {
    let styles: Vec<String> =
        ["Light", "Narrow", "Regular", "Italic", "Narrow Italic", "SemiBold", "Bold", "Bold Condensed", "Narrow Bold", "Bold Italic", "Black"]
            .iter()
            .map(|s| s.to_string())
            .collect();
    assert_eq!(pick_face(&styles, true, false, "Regular"), Some("Bold"), "not Bold Condensed, Narrow Bold or SemiBold");
    assert_eq!(pick_face(&styles, true, true, "Italic"), Some("Bold Italic"));
    assert_eq!(pick_face(&styles, false, true, "Regular"), Some("Italic"));
    assert_eq!(pick_face(&styles, false, false, "Italic"), Some("Regular"), "upright again, not Narrow");
    assert_eq!(pick_face(&styles, true, false, "Narrow"), Some("Narrow Bold"), "a Narrow face stays Narrow");
    assert_eq!(pick_face(&styles, false, true, "Narrow"), Some("Narrow Italic"));
    let oblique: Vec<String> = ["Book", "Oblique", "Bold-Oblique"].iter().map(|s| s.to_string()).collect();
    assert_eq!(pick_face(&oblique, true, true, "Oblique"), Some("Bold-Oblique"));
    assert_eq!(pick_face(&oblique, false, false, "Oblique"), None, "Book has a weight word: no regular face");
    assert_eq!(face_of("Bold Italic"), (true, false, true, vec![]));
    assert_eq!(face_of("SemiBold"), (false, false, false, vec![]));
    assert_eq!(face_of("Narrow Italic"), (false, true, true, vec!["narrow".to_string()]));
    // No type selected: a message, nothing changed.
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    assert!(toggle_face(&mut app, Face::Bold).is_err());
    assert!(!crate::menus::enabled(&app, "type.bold"));
}

/// From one frame of the panel: a point inside each of the Small Caps, Superscript and
/// Subscript buttons, and which of them are drawn on. The two buttons with a "1" are
/// Superscript (the "1" up) and Subscript; Small Caps is the button before them.
fn script_buttons(ctx: &egui::Context, app: &mut VectorcraftApp) -> ([egui::Pos2; 3], [bool; 3]) {
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
    out.textures_delta.clear();
    fn walk(s: &egui::Shape, on: egui::Color32, ones: &mut Vec<egui::Pos2>, lit: &mut Vec<egui::Rect>) {
        match s {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, on, ones, lit)),
            egui::Shape::Text(t) if t.galley.text() == "1" => ones.push(t.visual_bounding_rect().center()),
            egui::Shape::Rect(r) if r.fill == on && r.rect.size() == vec2(30.0, 26.0) => lit.push(r.rect),
            _ => {}
        }
    }
    let (mut ones, mut lit) = (vec![], vec![]);
    for c in &out.shapes {
        walk(&c.shape, Tokens::get(ctx).tool_active, &mut ones, &mut lit);
    }
    let [sup, sub] = ones[..] else { panic!("two script buttons: {ones:?}") };
    assert!(sup.x < sub.x && sup.y < sub.y, "Superscript, then Subscript: {ones:?}");
    // Each "1" is drawn 6 points right of its button's center and 5 points above or below it.
    let y = (sup.y + sub.y) / 2.0;
    let (sup, sub) = (egui::pos2(sup.x - 6.0, y), egui::pos2(sub.x - 6.0, y));
    let points = [sup - vec2(sub.x - sup.x, 0.0), sup, sub];
    (points, points.map(|p| lit.iter().any(|r| r.contains(p))))
}

/// Each run's (small caps, position).
fn scripts(app: &VectorcraftApp, id: &Value) -> Vec<(Option<f64>, &'static str)> {
    match &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind {
        NodeKind::Text(t) => t.runs.iter().map(|r| (r.style.small_caps, r.style.position.id())).collect(),
        _ => panic!("text"),
    }
}

/// The Small Caps, Superscript and Subscript buttons set the selected type's small caps and
/// position, with one undo step for each click, and show them; Superscript and Subscript turn
/// each other off. While the Type tool edits, they set and show the selected characters.
#[test]
fn small_caps_superscript_and_subscript_buttons_set_and_show_the_text() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    app.session.select_tool("type", Default::default()).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "E=mc2", "size": 12})).unwrap()["id"].clone();
    let ctx = egui::Context::default();
    let (at, lit) = script_buttons(&ctx, &mut app);
    assert_eq!(lit, [false; 3]);
    let small = Some(app.session.active().unwrap().doc.setup.small_caps_size);
    for (button, runs, shown) in [
        (0, (small, "normal"), [true, false, false]),
        (1, (small, "superscript"), [true, true, false]),
        (2, (small, "subscript"), [true, false, true]),
        (2, (small, "normal"), [true, false, false]),
        (1, (small, "superscript"), [true, true, false]),
        (1, (small, "normal"), [true, false, false]),
        (0, (None, "normal"), [false; 3]),
    ] {
        let undo = app.session.active().unwrap().history.undo.len();
        super::super::tests_appearance::click(&ctx, &mut app, at[button], show);
        assert_eq!(scripts(&app, &id), [runs], "button {button}");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "button {button}");
        assert_eq!(script_buttons(&ctx, &mut app).1, shown, "button {button}");
    }
    // The Type tool's selection: "2" alone.
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    app.session.set_tool_option("select", &json!({"start": 4, "end": 5}));
    assert!(text_editing(&app).is_some());
    super::super::tests_appearance::click(&ctx, &mut app, at[1], show);
    assert_eq!(scripts(&app, &id), [(None, "normal"), (None, "superscript")]);
    assert_eq!(script_buttons(&ctx, &mut app).1, [false, true, false]);
    app.session.set_tool_option("select", &json!({"start": 0, "end": 4}));
    assert_eq!(script_buttons(&ctx, &mut app).1, [false; 3]);
}

/// The panel menu's Small Caps, Superscript and Subscript set the selected type's small caps
/// and position and show a check mark when on; Superscript and Subscript turn each other off.
#[test]
fn small_caps_superscript_and_subscript_menu_items_set_and_check_the_text() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "E=mc2", "size": 12})).unwrap()["id"].clone();
    let ctx = egui::Context::default();
    let checked = |app: &mut VectorcraftApp| {
        let texts = super::super::tests_appearance::frame_events(&ctx, app, vec![], menu);
        ["Small Caps", "Superscript", "Subscript"].map(|l| texts.iter().any(|(t, _)| *t == format!("✓ {l}")))
    };
    assert_eq!(checked(&mut app), [false; 3]);
    let small = Some(app.session.active().unwrap().doc.setup.small_caps_size);
    for (item, runs, shown) in [
        ("Small Caps", (small, "normal"), [true, false, false]),
        ("Superscript", (small, "superscript"), [true, true, false]),
        ("Subscript", (small, "subscript"), [true, false, true]),
        ("Subscript", (small, "normal"), [true, false, false]),
        ("Small Caps", (None, "normal"), [false; 3]),
    ] {
        let texts = super::super::tests_appearance::frame_events(&ctx, &mut app, vec![], menu);
        let at = texts.iter().find(|(t, _)| t.trim_start_matches(['✓', ' ']) == item).unwrap_or_else(|| panic!("{item}: {texts:?}")).1;
        super::super::tests_appearance::click(&ctx, &mut app, at.center(), menu);
        assert_eq!(scripts(&app, &id), [runs], "{item}");
        assert_eq!(checked(&mut app), shown, "{item}");
    }
}

/// Character panel menu › System Layout is off for a new document, and a click turns whole-point
/// spacing on and back off for every text object.
#[test]
fn system_layout_menu_item_toggles_the_document() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "Hi", "size": 13})).unwrap()["id"].as_u64().unwrap();
    let ctx = egui::Context::default();
    let on = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        let object = match &st.doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => t.system_layout,
            _ => panic!("text"),
        };
        (st.doc.setup.system_layout, object)
    };
    let row = |app: &mut VectorcraftApp| {
        let texts = super::super::tests_appearance::frame_events(&ctx, app, vec![], menu);
        texts.into_iter().find(|(t, _)| t.trim_start_matches(['✓', ' ']) == "System Layout")
    };
    assert_eq!(on(&app), (false, false));
    assert!(row(&mut app).is_some_and(|(t, _)| !t.starts_with('✓')));
    let at = row(&mut app).unwrap().1;
    super::super::tests_appearance::click(&ctx, &mut app, at.center(), menu);
    assert_eq!(on(&app), (true, true));
    assert!(row(&mut app).is_some_and(|(t, _)| t.starts_with('✓')));
    let at = row(&mut app).unwrap().1;
    super::super::tests_appearance::click(&ctx, &mut app, at.center(), menu);
    assert_eq!(on(&app), (false, false));
    assert!(row(&mut app).is_some_and(|(t, _)| !t.starts_with('✓')));
}

/// Character panel menu › Fractional Widths is on for new type, and a click turns it off and
/// back on for the whole type object.
#[test]
fn fractional_widths_menu_item_toggles_the_type_object() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "Hi", "size": 13})).unwrap()["id"].as_u64().unwrap();
    let ctx = egui::Context::default();
    let flag = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.fractional_widths,
        _ => panic!("text"),
    };
    let row = |app: &mut VectorcraftApp| {
        let texts = super::super::tests_appearance::frame_events(&ctx, app, vec![], menu);
        texts.into_iter().find(|(t, _)| t.trim_start_matches(['✓', ' ']) == "Fractional Widths")
    };
    assert!(flag(&app));
    assert!(row(&mut app).is_some_and(|(t, _)| t.starts_with('✓')));
    let at = row(&mut app).unwrap().1;
    super::super::tests_appearance::click(&ctx, &mut app, at.center(), menu);
    assert!(!flag(&app));
    assert!(row(&mut app).is_some_and(|(t, _)| !t.starts_with('✓')));
    let at = row(&mut app).unwrap().1;
    super::super::tests_appearance::click(&ctx, &mut app, at.center(), menu);
    assert!(flag(&app));
    assert!(row(&mut app).is_some_and(|(t, _)| t.starts_with('✓')));
}

/// Character Alignment and Proportional Metrics are in the panel menu with the East Asian
/// options only (as Mojikumi Set and Top-to-Top Leading are in the Paragraph panel).
#[test]
fn character_alignment_shows_with_the_east_asian_options() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "雅楽"})).unwrap()["id"].clone();
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    let shown = |app: &mut VectorcraftApp| {
        let text = crate::tests_labels::painted_text(app, menu);
        (text.contains("Character Alignment"), text.contains("Proportional Metrics"))
    };
    assert_eq!(shown(&mut app), (false, false));
    app.session.prefs.show_east_asian_options = true;
    assert_eq!(shown(&mut app), (true, true));
}

/// Tate-chu-yoko is in the panel menu with the East Asian options, and choosing it marks the
/// selected characters.
#[test]
fn tate_chu_yoko_menu_item_shows_with_east_asian_options_and_toggles() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "2026"})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    let ctx = egui::Context::default();
    let painted = |app: &mut VectorcraftApp| crate::tests_labels::painted_text(app, menu);
    assert!(!painted(&mut app).contains("Tate-chu-yoko"));
    app.session.prefs.show_east_asian_options = true;
    let on = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.runs.iter().all(|r| r.style.tate_chu_yoko),
        _ => panic!("text"),
    };
    assert!(!on(&app));
    let texts = super::super::tests_appearance::frame_events(&ctx, &mut app, vec![], menu);
    let at = texts.iter().find(|(t, _)| t.trim_start_matches(['✓', ' ']) == "Tate-chu-yoko").unwrap_or_else(|| panic!("{texts:?}")).1;
    super::super::tests_appearance::click(&ctx, &mut app, at.center(), menu);
    assert!(on(&app));
    assert!(painted(&mut app).contains("✓ Tate-chu-yoko"));
    let texts = super::super::tests_appearance::frame_events(&ctx, &mut app, vec![], menu);
    let at = texts.iter().find(|(t, _)| t.trim_start_matches(['✓', ' ']) == "Tate-chu-yoko").unwrap().1;
    super::super::tests_appearance::click(&ctx, &mut app, at.center(), menu);
    assert!(!on(&app));
}

/// Digits is in the panel menu with the Middle Eastern (Indic) options only, as Paragraph
/// Direction is in the Paragraph panel.
#[test]
fn digits_show_with_the_indic_options() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "صفحة 3"})).unwrap()["id"].clone();
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    let shown = |app: &mut VectorcraftApp| crate::tests_labels::painted_text(app, menu).contains("Digits");
    assert!(!shown(&mut app));
    app.session.prefs.show_indic_options = true;
    assert!(shown(&mut app));
}

/// Character Direction is in the panel menu with the Middle Eastern (Indic) options, beside
/// Digits. Its submenu checks the direction the characters the panel acts on share: the
/// selected type's, or the range the Type tool has selected. It checks no item when they differ.
#[test]
fn character_direction_checks_the_direction_of_the_selection() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "שלום 21 10 2026"})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    let shown = |app: &mut VectorcraftApp| crate::tests_labels::painted_text(app, menu).contains("Character Direction");
    assert!(!shown(&mut app));
    app.session.prefs.show_indic_options = true;
    assert!(shown(&mut app));
    let checked = |app: &mut VectorcraftApp| {
        let text = crate::tests_labels::painted_text(app, char_direction_items);
        text.lines().filter_map(|l| l.strip_prefix("✓ ")).map(str::to_string).collect::<Vec<_>>()
    };
    assert_eq!(checked(&mut app), ["Default"]);
    app.run("text.setRangeStyle", json!({"id": id, "start": 9, "end": 19, "charDirection": "leftToRight"})).unwrap();
    // The object's characters differ: nothing is checked.
    assert!(checked(&mut app).is_empty());
    // The Type tool's selection: the date, then the Hebrew word.
    app.select_tool("type");
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    app.session.set_tool_option("select", &json!({"start": 9, "end": 19}));
    assert_eq!(checked(&mut app), ["Left to Right"]);
    app.session.set_tool_option("select", &json!({"start": 0, "end": 8}));
    assert_eq!(checked(&mut app), ["Default"]);
}

#[test]
fn shared_text_flag_optical_margin_three_object_selections() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({})).unwrap();
    let a = app.run("text.create", json!({"x": 10, "y": 40, "text": "a"})).unwrap()["id"].clone();
    let b = app.run("text.create", json!({"x": 10, "y": 80, "text": "b"})).unwrap()["id"].clone();
    let c = app.run("text.create", json!({"x": 10, "y": 120, "text": "c"})).unwrap()["id"].clone();
    app.run("type.opticalMarginAlignment", json!({"on": true, "id": a})).unwrap();
    app.run("type.opticalMarginAlignment", json!({"on": false, "id": b})).unwrap();
    app.run("type.opticalMarginAlignment", json!({"on": false, "id": c})).unwrap();
    app.run("select.set", json!({"ids": [a.clone(), b.clone(), c.clone()]})).unwrap();
    assert_eq!(shared_text_flag(&app, |t| t.optical_margin), None);
    app.run("type.opticalMarginAlignment", json!({"on": false, "id": a})).unwrap();
    app.run("type.opticalMarginAlignment", json!({"on": true, "id": b})).unwrap();
    app.run("type.opticalMarginAlignment", json!({"on": true, "id": c})).unwrap();
    app.run("select.set", json!({"ids": [a.clone(), b.clone(), c.clone()]})).unwrap();
    assert_eq!(shared_text_flag(&app, |t| t.optical_margin), None);
    app.run("type.opticalMarginAlignment", json!({"on": true, "id": a})).unwrap();
    app.run("type.opticalMarginAlignment", json!({"on": true, "id": b})).unwrap();
    app.run("type.opticalMarginAlignment", json!({"on": true, "id": c})).unwrap();
    app.run("select.set", json!({"ids": [a, b, c]})).unwrap();
    assert_eq!(shared_text_flag(&app, |t| t.optical_margin), Some(true));
}

/// Small Caps, Superscript and Subscript buttons and menu items stay off when the selection mixes them.
#[test]
fn character_mixed_small_caps_and_position_show_off() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let a = app.run("text.create", json!({"x": 20, "y": 50, "text": "A"})).unwrap()["id"].clone();
    let b = app.run("text.create", json!({"x": 20, "y": 90, "text": "B"})).unwrap()["id"].clone();
    app.run("text.setRangeStyle", json!({"id": a, "start": 0, "end": 1, "smallCaps": true, "position": "superscript"})).unwrap();
    app.run("text.setRangeStyle", json!({"id": b, "start": 0, "end": 1, "smallCaps": false, "position": "subscript"})).unwrap();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    assert_eq!(shared(&app, |s| s.small_caps.is_some()), None);
    assert_eq!(shared(&app, |s| s.position), None);
    let ctx = egui::Context::default();
    assert_eq!(script_buttons(&ctx, &mut app).1, [false, false, false]);
    let checked = |app: &mut VectorcraftApp| {
        let texts = super::super::tests_appearance::frame_events(&ctx, app, vec![], menu);
        ["Small Caps", "Superscript", "Subscript"].map(|l| texts.iter().any(|(t, _)| *t == format!("✓ {l}")))
    };
    assert_eq!(checked(&mut app), [false, false, false]);
}

/// At a caret in text the Type tool edits, with no characters selected, the Character
/// Direction items are disabled: a click on one leaves the direction of every character as it was.
#[test]
fn character_direction_does_nothing_at_an_empty_caret() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "שלום 21 10 2026"})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.select_tool("type");
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    app.session.set_tool_option("select", &json!({"start": 3, "end": 3}));
    let directions = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.runs.iter().map(|r| (r.text.clone(), r.style.char_direction)).collect::<Vec<_>>(),
        _ => panic!("text"),
    };
    let before = directions(&app);
    let ctx = egui::Context::default();
    let click_rtl = |app: &mut VectorcraftApp| {
        let texts = super::super::tests_appearance::frame_events(&ctx, app, vec![], char_direction_items);
        let at = texts.iter().find(|(t, _)| t.trim_start_matches(['✓', ' ']) == "Right to Left").unwrap_or_else(|| panic!("{texts:?}")).1;
        super::super::tests_appearance::click(&ctx, app, at.center(), char_direction_items);
    };
    click_rtl(&mut app);
    assert_eq!(directions(&app), before);
    // With characters selected, the same click sets their direction.
    app.session.set_tool_option("select", &json!({"start": 0, "end": 4}));
    click_rtl(&mut app);
    assert_ne!(directions(&app), before);
}

/// No Break is in the panel menu, and choosing it marks the selected characters.
#[test]
fn no_break_menu_item_toggles_the_selected_characters() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "Hello"})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    let ctx = egui::Context::default();
    let painted = |app: &mut VectorcraftApp| crate::tests_labels::painted_text(app, menu);
    assert!(painted(&mut app).contains("No Break"));
    assert!(!painted(&mut app).contains("✓ No Break"));
    let on = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.runs.iter().all(|r| r.style.no_break),
        _ => panic!("text"),
    };
    assert!(!on(&app));
    let texts = super::super::tests_appearance::frame_events(&ctx, &mut app, vec![], menu);
    let at = texts.iter().find(|(t, _)| t.trim_start_matches(['✓', ' ']) == "No Break").unwrap_or_else(|| panic!("{texts:?}")).1;
    super::super::tests_appearance::click(&ctx, &mut app, at.center(), menu);
    assert!(on(&app));
    assert!(painted(&mut app).contains("✓ No Break"));
    let texts = super::super::tests_appearance::frame_events(&ctx, &mut app, vec![], menu);
    let at = texts.iter().find(|(t, _)| t.trim_start_matches(['✓', ' ']) == "No Break").unwrap().1;
    super::super::tests_appearance::click(&ctx, &mut app, at.center(), menu);
    assert!(!on(&app));
}

/// Reset Panel sets every character attribute the panel shows, except the font family, style
/// and size, back to its default (Small Caps, the position, Character Alignment, Proportional
/// Metrics, Tate-chu-yoko, No Break, Character Direction and Digits included), in the selected
/// type and in the text the Type tool has selected while it edits.
#[test]
fn reset_panel_resets_every_attribute_but_the_font() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "One 12"})).unwrap()["id"].as_u64().unwrap();
    let runs = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.runs.iter().map(|r| (r.text.clone(), r.style.clone())).collect::<Vec<_>>(),
        _ => panic!("text"),
    };
    let differs = |s: &CharStyle, d: &CharStyle| {
        [
            s.tracking != d.tracking,
            s.leading != d.leading,
            s.kerning != d.kerning,
            s.baseline_shift != d.baseline_shift,
            s.h_scale != d.h_scale,
            s.v_scale != d.v_scale,
            s.rotation != d.rotation,
            s.underline != d.underline,
            s.strikethrough != d.strikethrough,
            s.all_caps != d.all_caps,
            s.small_caps != d.small_caps,
            s.position != d.position,
            s.char_align != d.char_align,
            s.proportional_metrics != d.proportional_metrics,
            s.tate_chu_yoko != d.tate_chu_yoko,
            s.no_break != d.no_break,
            s.char_direction != d.char_direction,
            s.digits != d.digits,
        ]
    };
    let custom = |app: &mut VectorcraftApp| {
        app.run(
            "text.setRangeStyle",
            json!({
                "id": id, "tracking": 50, "leading": 30, "kerning": 20, "baselineShift": 3, "hScale": 120, "vScale": 90, "rotation": 15,
                "underline": true, "strikethrough": true, "allCaps": true, "smallCaps": true, "position": "superscript",
                "charAlign": "emBoxCenter", "proportionalMetrics": true, "tateChuYoko": true, "noBreak": true,
                "charDirection": "rightToLeft", "digits": "hindi"
            }),
        )
        .unwrap();
    };
    let plain = runs(&app)[0].1.clone();
    assert_eq!(differs(&plain, &CharStyle::default()), [false; 18]);
    custom(&mut app);
    let styled = runs(&app)[0].1.clone();
    assert_eq!(differs(&styled, &CharStyle::default()), [true; 18]);
    app.run("select.set", json!({"ids": [id]})).unwrap();
    reset_panel(&mut app);
    assert_eq!(runs(&app), [("One 12".to_string(), plain.clone())]);
    // While the Type tool edits text, Reset Panel resets only its selection.
    custom(&mut app);
    app.select_tool("type");
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    app.session.set_tool_option("select", &json!({"start": 4, "end": 6}));
    assert!(text_editing(&app).is_some());
    reset_panel(&mut app);
    assert_eq!(runs(&app), [("One ".to_string(), styled), ("12".to_string(), plain)]);
}

#[test]
fn ruby_params_set_and_remove_the_reading() {
    let id = NodeId(7);
    let f = RubyFields {
        reading: " かんじ ".into(),
        position: "below".into(),
        align: "end".into(),
        size: 40.0,
        overhang: false,
        ..RubyFields::default()
    };
    let p = ruby_params(id, 3, 9, &f, false);
    assert_eq!(p["id"], 7);
    assert_eq!((p["start"].as_u64(), p["end"].as_u64()), (Some(3), Some(9)));
    assert_eq!(p["text"], "かんじ", "the reading is trimmed");
    assert_eq!(p["position"], "below");
    assert_eq!(p["align"], "end");
    assert_eq!(p["size"], 40.0);
    assert_eq!(p["overhang"], false);
    let r = ruby_params(id, 3, 9, &f, true);
    assert_eq!(r["remove"], true);
    assert!(r.get("text").is_none());
}

#[test]
fn ruby_fields_load_the_ruby_under_the_selection() {
    let mut t = TextObject::point(vectorcraft_geom::Point::ZERO, "漢字です", CharStyle::default());
    let span =
        RubySpan { position: RubyPosition::Below, align: RubyAlign::End, size_percent: 40.0, overhang: false, ..RubySpan::new(0, 6, "かんじ") };
    t.ruby = vec![span];
    let found = ruby_in(&t, 0, 6).expect("the ruby under the selection");
    let f = RubyFields::from_span(Some(found));
    assert_eq!(f.reading, "かんじ");
    assert_eq!((f.position.as_str(), f.align.as_str()), ("below", "end"));
    assert!((f.size - 40.0).abs() < 1e-9 && !f.overhang);
    assert!(ruby_covers(&t, 0, 6), "the only span over exactly the selection");
    // A selection outside it has none; no span is the defaults.
    assert!(ruby_in(&t, 9, 12).is_none() && !ruby_covers(&t, 9, 12));
    assert_eq!(RubyFields::from_span(None), RubyFields::default());
    // The fields reload when the selection or its ruby changes.
    assert_ne!(ruby_sync_key(NodeId(1), 0, 6, Some(found)), ruby_sync_key(NodeId(1), 0, 3, Some(found)));
    assert_ne!(ruby_sync_key(NodeId(1), 0, 6, Some(found)), ruby_sync_key(NodeId(1), 0, 6, None));
    // A partial selection and a second reading under it are not re-applied in place.
    assert!(!ruby_covers(&t, 0, 3), "the selection is only part of the span");
    t.ruby.push(RubySpan::new(3, 6, "じ"));
    assert!(!ruby_covers(&t, 0, 6), "two readings under the selection");
}

/// The reading typed into the panel is applied to the selected text through `type.ruby`, shown
/// as applied, and removable again; Undo brings it back (one step per action).
#[test]
fn ruby_panel_applies_and_removes_the_reading() {
    use super::super::tests_appearance::{click, frame_events, text_rect};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = NodeId(app.run("text.create", json!({"x": 20, "y": 50, "text": "漢字です", "size": 12})).unwrap()["id"].as_u64().unwrap());
    app.run("select.set", json!({"ids": [id.0]})).unwrap();
    let ctx = egui::Context::default();
    let texts = frame_events(&ctx, &mut app, vec![], show);
    let labels: Vec<&str> = texts.iter().map(|(s, _)| s.as_str()).collect();
    for label in ["Ruby", "Reading:", "Position:", "Align:", "Size:", "Overhang", "Apply", "Remove"] {
        assert!(labels.contains(&label), "`{label}` not drawn: {labels:?}");
    }
    assert!(labels.contains(&"Above") && labels.contains(&"Center"), "the defaults: {labels:?}");
    // Type the reading into its field, then Apply.
    let row = text_rect(&texts, "Reading:");
    click(&ctx, &mut app, row.right_center() + egui::vec2(30.0, 0.0), show);
    frame_events(&ctx, &mut app, vec![egui::Event::Text("かんじ".into())], show);
    let texts = frame_events(&ctx, &mut app, vec![], show);
    assert!(texts.iter().any(|(s, _)| s == "かんじ"), "the field shows the typed reading: {texts:?}");
    click(&ctx, &mut app, text_rect(&texts, "Apply").center(), show);
    let ruby = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => t.ruby.clone(),
        _ => panic!("text"),
    };
    let r = ruby(&app);
    assert_eq!((r.len(), r[0].start, r[0].end, r[0].text.as_str()), (1, 0, 12, "かんじ"), "the whole selected text (12 bytes)");
    assert_eq!(r[0].position, RubyPosition::Above);
    assert!(r[0].overhang);
    // The panel shows it as applied, and Remove takes it off; Undo puts it back.
    let texts = frame_events(&ctx, &mut app, vec![], show);
    assert!(texts.iter().any(|(s, _)| s == "かんじ"), "the reading stays in the field: {texts:?}");
    click(&ctx, &mut app, text_rect(&texts, "Remove").center(), show);
    assert!(ruby(&app).is_empty());
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(ruby(&app).len(), 1);
}

/// With the Type tool, the section acts on the selected characters only, not the whole text.
#[test]
fn ruby_panel_acts_on_the_type_tools_selection() {
    use super::super::tests_appearance::{click, frame_events, text_rect};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    app.session.select_tool("type", Default::default()).unwrap();
    let id = NodeId(app.run("text.create", json!({"x": 20, "y": 50, "text": "漢字です", "size": 12})).unwrap()["id"].as_u64().unwrap());
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    app.session.set_tool_option("select", &json!({"start": 0, "end": 3}));
    let ctx = egui::Context::default();
    let texts = frame_events(&ctx, &mut app, vec![], show);
    let row = text_rect(&texts, "Reading:");
    click(&ctx, &mut app, row.right_center() + egui::vec2(30.0, 0.0), show);
    frame_events(&ctx, &mut app, vec![egui::Event::Text("かん".into())], show);
    let texts = frame_events(&ctx, &mut app, vec![], show);
    click(&ctx, &mut app, text_rect(&texts, "Apply").center(), show);
    let NodeKind::Text(t) = &app.session.active().unwrap().doc.node(id).unwrap().kind else { panic!("text") };
    assert_eq!(t.ruby.len(), 1);
    assert_eq!((t.ruby[0].start, t.ruby[0].end, t.ruby[0].text.as_str()), (0, 3, "かん"), "only the selected character");
}

/// A typed size commits on Enter and re-applies the ruby, like dragging the value does.
#[test]
fn ruby_panel_typing_a_size_reapplies_the_ruby() {
    use super::super::tests_appearance::{click, frame_events, text_rect};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = NodeId(app.run("text.create", json!({"x": 20, "y": 50, "text": "漢字", "size": 12})).unwrap()["id"].as_u64().unwrap());
    app.run("select.set", json!({"ids": [id.0]})).unwrap();
    app.run("type.ruby", json!({"text": "かんじ"})).unwrap();
    let ctx = egui::Context::default();
    let texts = frame_events(&ctx, &mut app, vec![], show);
    click(&ctx, &mut app, text_rect(&texts, "Size:").right_center() + egui::vec2(30.0, 0.0), show);
    let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    frame_events(&ctx, &mut app, vec![key(egui::Key::A, egui::Modifiers::COMMAND), egui::Event::Text("40".into())], show);
    frame_events(&ctx, &mut app, vec![key(egui::Key::Enter, Default::default())], show);
    let NodeKind::Text(t) = &app.session.active().unwrap().doc.node(id).unwrap().kind else { panic!("text") };
    assert_eq!(t.ruby.len(), 1);
    assert_eq!(t.ruby[0].text, "かんじ");
    assert!((t.ruby[0].size_percent - 40.0).abs() < 1e-9, "the typed size is applied: {:?}", t.ruby[0]);
}

/// Option edits re-apply only the one ruby over exactly the selection: over two readings,
/// changing Position leaves them alone until Apply replaces them over the selection.
#[test]
fn ruby_panel_option_edits_do_not_merge_overlapping_readings() {
    use super::super::tests_appearance::{click, frame_events, text_rect};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = NodeId(app.run("text.create", json!({"x": 20, "y": 50, "text": "漢字", "size": 12})).unwrap()["id"].as_u64().unwrap());
    app.run("select.set", json!({"ids": [id.0]})).unwrap();
    app.run("type.ruby", json!({"start": 0, "end": 3, "text": "かん"})).unwrap();
    app.run("type.ruby", json!({"start": 3, "end": 6, "text": "じ"})).unwrap();
    let readings = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => t.ruby.iter().map(|r| (r.start, r.end, r.text.clone(), r.position)).collect::<Vec<_>>(),
        _ => panic!("text"),
    };
    let ctx = egui::Context::default();
    let texts = frame_events(&ctx, &mut app, vec![], show);
    // Open Position and pick Below: the field shows it, but both readings stay.
    click(&ctx, &mut app, text_rect(&texts, "Above").center(), show);
    let texts = frame_events(&ctx, &mut app, vec![], show);
    click(&ctx, &mut app, text_rect(&texts, "Below").center(), show);
    assert_eq!(
        readings(&app),
        [(0, 3, "かん".to_string(), RubyPosition::Above), (3, 6, "じ".to_string(), RubyPosition::Above)],
        "no option edit collapses the readings"
    );
    // Apply still replaces the ruby over the whole selection.
    let texts = frame_events(&ctx, &mut app, vec![], show);
    click(&ctx, &mut app, text_rect(&texts, "Apply").center(), show);
    assert_eq!(readings(&app), [(0, 6, "かん".to_string(), RubyPosition::Below)]);
}

/// #4232: the panel menu's Show Snap to Glyph Options shows the Snap to Glyph section (with no
/// type selected too) and Hide Snap to Glyph Options hides it; its checkboxes, dimmed while View ›
/// Snap to Glyph is off, set the options.
#[test]
fn snap_to_glyph_options_show_from_the_menu_and_set_the_options() {
    use super::super::tests_appearance::{click, frame_events, text_rect};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    let toggle = |app: &mut VectorcraftApp, label: &str| {
        let texts = frame_events(&ctx, app, vec![], menu);
        let at = texts.iter().find(|(t, _)| t.trim() == label).unwrap_or_else(|| panic!("{label} in {texts:?}")).1;
        click(&ctx, app, at.center(), menu);
    };
    let section = |app: &mut VectorcraftApp| frame_events(&ctx, app, vec![], show);
    let labels = ["Baseline", "x-height", "Glyph Bounds", "Proximity Guides", "Angular Guides", "Anchor Point"];
    assert!(!section(&mut app).iter().any(|(t, _)| t == "Proximity Guides"), "hidden at first");
    toggle(&mut app, "Show Snap to Glyph Options");
    let texts = section(&mut app);
    assert!(labels.iter().all(|l| texts.iter().any(|(t, _)| t == l)), "{texts:?}");
    let check = |app: &mut VectorcraftApp, label: &str| {
        let at = text_rect(&section(app), label).center();
        click(&ctx, app, at, show);
    };
    check(&mut app, "Angular Guides");
    assert!(app.session.prefs.glyph_snap.angular, "dimmed while Snap to Glyph is off");
    app.ui.view.snap_to_glyph = true;
    check(&mut app, "Angular Guides");
    check(&mut app, "x-height");
    let o = app.session.prefs.glyph_snap;
    assert!(!o.angular && !o.x_height && o.baseline && o.bounds && o.proximity && o.anchors, "{o:?}");
    check(&mut app, "Angular Guides");
    assert!(app.session.prefs.glyph_snap.angular);
    toggle(&mut app, "Hide Snap to Glyph Options");
    assert!(!section(&mut app).iter().any(|(t, _)| t == "Proximity Guides"));
}
