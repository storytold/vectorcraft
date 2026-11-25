use super::*;
use crate::i18n::LangExt;
use craft::behavior::{EditEvent, Mixed};
use craft::text::ui::character::{BaselinePosition, Caps, CharacterChange, CharacterRows, CharacterValues, Kerning, Leading};
use craft::text::ui::type_options::TextAntiAlias;
use vectorcraft_doc::text::TypeAntiAlias;
use vectorcraft_doc::{CharStyle, NodeId, NodeKind};
use vectorcraft_engine::Session;

/// `value` is `want` (the panel's fields are single precision).
fn near(value: Option<f32>, want: f64) -> bool {
    value.is_some_and(|v| (f64::from(v) - want).abs() < 1e-6)
}

/// A change the Character controls raise, applied as one step the way the framework delivers it.
fn change(app: &mut VectorcraftApp, change: CharacterChange) {
    apply_character_event(app, EditEvent::set(Mixed::Mixed, change, "Character".into(), None));
}

/// The commands the panel menu lists, the rows of its submenus included.
fn listed(app: &VectorcraftApp) -> Vec<CommandRef> {
    fn walk(rows: &[PanelRow], out: &mut Vec<CommandRef>) {
        for row in rows {
            match row {
                PanelRow::Command(c) | PanelRow::Alternated(c, _) => out.push(c.clone()),
                PanelRow::Submenu(_, rows) => walk(rows, out),
                _ => {}
            }
        }
    }
    let mut out = vec![];
    walk(&menu_rows(app), &mut out);
    out
}

/// What the menu says of a row: (enabled, checked).
fn state(app: &VectorcraftApp, command: &CommandRef) -> (bool, bool) {
    let mut states = CommandStates::new();
    menu_states(app, &mut states);
    let s = states.get(command).unwrap_or_else(|| panic!("no state for {}", command.key()));
    (s.is_enabled(), s.is_checked() == Some(true))
}

/// Runs a menu row the way the app does: the panel's own `run`, else the engine command of that id.
fn fire(app: &mut VectorcraftApp, command: &CommandRef) {
    let params = if command.params.is_null() { json!({}) } else { command.params.clone() };
    let done = run(app, &command.id, &params).unwrap_or_else(|| app.run(&command.id, params.clone()).map_err(|e| e.to_string()));
    assert!(done.is_ok(), "{}: {done:?}", command.key());
}

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
    let v = values(&mut app);
    assert_eq!(v.size, None, "a collapsed scaled object has no size to show");
    assert_eq!(v.tracking, Some(40.0));
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

/// The Leading field shows the Auto leading the characters are set with, their paragraph's Auto
/// Leading share (120% by default) of their em size: 20 pt type measuring a 0.7 em cap height
/// is set at 20 / 0.7 pt. Paragraphs whose Auto leading differs show a blank field, and an
/// explicit leading shows as it is.
#[test]
fn leading_shows_the_auto_leading_of_the_em_size() {
    use vectorcraft_text::test_fonts::{HEIGHT_CAP, HEIGHT_FAMILY, height_font};
    vectorcraft_text::FontDb::global().add_font(height_font(true).unwrap());
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "Hx", "size": 20, "font": HEIGHT_FAMILY})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    let near = |l: Option<f64>, want: f64| l.is_some_and(|l| (l - want).abs() < 1e-9);
    assert!(near(shared_leading(&app), 24.0), "{:?}", shared_leading(&app));
    app.run("text.setFormat", json!({"fontHeightReference": "capHeight"})).unwrap();
    let cap = f64::from(HEIGHT_CAP) / 1000.0;
    assert!(near(shared_leading(&app), 1.2 * 20.0 / cap), "{:?}", shared_leading(&app));
    app.run("text.setFormat", json!({"autoLeading": 150})).unwrap();
    assert!(near(shared_leading(&app), 1.5 * 20.0 / cap), "{:?}", shared_leading(&app));
    // The second paragraph ends in a two-byte character.
    let two = app.run("text.create", json!({"x": 20, "y": 120, "text": "Hx\nHé", "size": 20, "font": HEIGHT_FAMILY})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [two]})).unwrap();
    assert!(near(shared_leading(&app), 24.0), "{:?}", shared_leading(&app));
    app.run("text.setFormat", json!({"id": two, "start": 3, "end": 6, "autoLeading": 150})).unwrap();
    assert_eq!(shared_leading(&app), None, "the second paragraph's Auto leading is 30 pt");
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.run("text.setStyle", json!({"leading": 30})).unwrap();
    assert!(near(shared_leading(&app), 30.0), "{:?}", shared_leading(&app));
}

/// An inline graphic keeps its size whatever the font height reference, so the Leading field
/// shows 120% of that size for it: blank for 20 pt Cap Height type with a graphic in it, and
/// 24 pt for the graphic alone.
#[test]
fn leading_of_an_inline_graphic_follows_its_own_size() {
    use vectorcraft_text::test_fonts::{HEIGHT_FAMILY, height_font};
    vectorcraft_text::FontDb::global().add_font(height_font(true).unwrap());
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    app.run("shape.ellipse", json!({"x": 200, "y": 150, "width": 10, "height": 10})).unwrap();
    app.run("symbol.new", json!({"name": "Dot"})).unwrap();
    app.run("edit.clear", json!({})).unwrap();
    app.session.select_tool("type", Default::default()).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "Hx", "size": 20, "font": HEIGHT_FAMILY})).unwrap()["id"].clone();
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    app.run("text.insertInline", json!({"id": id, "at": 2, "symbol": "Dot"})).unwrap();
    app.run("text.setRangeStyle", json!({"id": id, "start": 0, "end": 5, "fontHeightReference": "capHeight"})).unwrap();
    app.session.set_tool_option("select", &json!({"start": 0, "end": 5}));
    assert_eq!(shared_leading(&app), None, "the type is set with 34.3 pt Auto leading and the graphic with 24 pt");
    app.session.set_tool_option("select", &json!({"start": 2, "end": 5}));
    assert_eq!(text_editing(&app).map(|(_, a, b)| (a, b)), Some((2, 5)));
    let shown = shared_leading(&app);
    assert!(shown.is_some_and(|l| (l - 24.0).abs() < 1e-9), "{shown:?}");
}

/// A selected text object that ends in a line break has an empty last paragraph, which the
/// Leading field takes in; a Type tool selection of all the text does not.
#[test]
fn leading_of_a_whole_object_takes_in_the_empty_last_paragraph() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "Ab\n", "size": 20})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    let near = |l: Option<f64>, want: f64| l.is_some_and(|l| (l - want).abs() < 1e-9);
    assert!(near(shared_leading(&app), 24.0), "{:?}", shared_leading(&app));
    app.run("text.setFormat", json!({"id": id, "start": 3, "end": 3, "autoLeading": 150})).unwrap();
    assert_eq!(shared_leading(&app), None, "the empty last paragraph's Auto leading is 30 pt");
    app.session.select_tool("type", Default::default()).unwrap();
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    app.session.set_tool_option("select", &json!({"start": 0, "end": 3}));
    assert_eq!(text_editing(&app).map(|(_, a, b)| (a, b)), Some((0, 3)));
    assert!(near(shared_leading(&app), 24.0), "a line break belongs to the paragraph it ends: {:?}", shared_leading(&app));
}

/// With the Type tool, the Leading field shows the Auto leading of the paragraphs the selection
/// covers (a line break belongs to the paragraph it ends), and at a caret that of the caret's
/// paragraph.
#[test]
fn leading_follows_the_paragraphs_the_type_tool_selection_covers() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    app.session.select_tool("type", Default::default()).unwrap();
    // é is two bytes, 4..6.
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "Ab\nCé", "size": 20})).unwrap()["id"].clone();
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    app.run("text.setFormat", json!({"id": id, "start": 3, "end": 6, "autoLeading": 150})).unwrap();
    let ranges = [((0, 2), Some(24.0)), ((0, 3), Some(24.0)), ((3, 6), Some(30.0)), ((1, 4), None), ((1, 6), None), ((4, 4), Some(30.0))];
    for ((start, end), want) in ranges {
        app.session.set_tool_option("select", &json!({"start": start, "end": end}));
        assert_eq!(text_editing(&app).map(|(_, a, b)| (a, b)), Some((start, end)));
        let shown = shared_leading(&app);
        assert_eq!(shown.is_some(), want.is_some(), "{start}..{end}: {shown:?}");
        assert!(shown.zip(want).is_none_or(|(l, w)| (l - w).abs() < 1e-9), "{start}..{end}: {shown:?}");
    }
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

/// The automatic pair value at a caret is laid out once per (document, revision, object, caret): the
/// memo is what keeps a repaint from laying the text out again, and a stale one would show another
/// edit's value.
#[test]
fn character_kerning_setter_and_automatic_pair_value() {
    let german = crate::i18n::Lang::from_code("de").unwrap();
    assert!(crate::i18n::has(german, "Optical"));
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    app.session.select_tool("type", Default::default()).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "III", "size": 100, "font": "JetBrains Mono"})).unwrap()["id"].as_u64().unwrap();
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    app.session.set_tool_option("select", &json!({"start": 1, "end": 1}));
    set_kerning(&mut app, json!("optical"));
    assert!(text_style(&app).unwrap().0.optical_kerning);
    let key = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        (st.uid, st.revision, NodeId(id), 1usize)
    };
    let (value, automatic) = caret_kerning(&mut app).unwrap();
    assert!(automatic && value != 0.0);
    assert_eq!(app.canvas.caret_kerning, Some((key(&app), value)));
    assert_eq!(caret_kerning(&mut app), Some((value, true)), "a second read keeps the value");
    assert_eq!(app.canvas.caret_kerning, Some((key(&app), value)));
    let revision = key(&app).1;
    app.run("text.setFormat", json!({"autoLeading": 150})).unwrap();
    assert_ne!(key(&app).1, revision);
    let (again, _) = caret_kerning(&mut app).unwrap();
    assert_eq!(app.canvas.caret_kerning, Some((key(&app), again)), "an edit makes a new key");
    set_kerning(&mut app, json!(50));
    assert_eq!(caret_kerning(&mut app), Some((50.0, false)));
    app.session.set_tool_option("select", &json!({"start": 0, "end": 2}));
    assert_eq!(shared(&app, |s| (s.optical_kerning, s.kerning)), None, "a mixed range shows a blank Kerning field");
}

/// Highlight Font Name, Font Size, Kerning and Tracking ask the framework to focus the field by its
/// document key, and nothing is focused when no type is selected.
#[test]
fn highlight_commands_ask_the_framework_to_focus_the_field_by_key() {
    use crate::tests::panel_harness::Panel;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "AVAX", "size": 12})).unwrap()["id"].clone();
    app.run("select.none", json!({})).unwrap();
    let fields = [(Field::FontName, "Font family"), (Field::FontSize, "Font size"), (Field::Kerning, "Kerning"), (Field::Tracking, "Tracking")];
    let mut panel = Panel::new(app, show);
    for (field, name) in fields {
        assert!(highlight(&mut panel.app(), field).is_err(), "{field:?} without selected type");
        panel.frame();
        // Without selected type the panel shows its empty state, so the field may not be there at all.
        assert!(!(panel.shows(name) && panel.focused(name)), "{name} took the focus without selected type");
        assert_eq!(panel.app().character_focus, None);
    }
    panel.app().run("select.set", json!({"ids": [id]})).unwrap();
    for (field, name) in fields {
        highlight(&mut panel.app(), field).unwrap();
        panel.frame();
        assert!(panel.focused(name), "{name} has the focus");
        assert_eq!(panel.app().character_focus, None, "{field:?}: the request is taken");
    }
}

/// The slider list and its gestures are the framework's. What the app decides: which axes the selection
/// shares, that a change carries only the axes that moved (so a mixed axis stays as it is), the
/// command (the object, or the Type tool's range) and that Reset Panel leaves the settings.
#[test]
fn variable_fonts_get_a_slider_per_axis() {
    use vectorcraft_text::test_fonts::{TWO_AXIS_FAMILY, two_axis_font};
    vectorcraft_text::FontDb::global().add_font(two_axis_font().unwrap());
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let plain = app.run("text.create", json!({"x": 20, "y": 50, "text": "Static"})).unwrap()["id"].clone();
    let id = app.run("text.create", json!({"x": 20, "y": 120, "text": "lIn", "font": TWO_AXIS_FAMILY})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [plain]})).unwrap();
    assert!(face_axes(&app).is_none() && values(&mut app).axes.is_none(), "a static font has no sliders");
    app.run("select.set", json!({"ids": [id]})).unwrap();
    let tags = face_axes(&app).map(|(axes, _)| axes.iter().map(|a| a.tag).collect::<Vec<_>>()).unwrap_or_default();
    assert!(tags.contains(b"wght") && tags.contains(b"wdth"), "{tags:?}");
    let axes = |app: &VectorcraftApp| shared(app, |s| s.font_axes.clone()).unwrap();
    change(&mut app, CharacterChange::Axes(vec![(*b"wght", 700.0)]));
    assert_eq!(axes(&app).into_iter().collect::<Vec<_>>(), [("wght".to_string(), 700.0)]);
    app.run("edit.undo", json!({})).unwrap();
    assert!(axes(&app).is_empty(), "one undo step");
    // Reset Panel leaves the settings, as it leaves the font and style.
    app.run("text.setFormat", json!({"fontAxes": {"wght": 650}})).unwrap();
    reset_panel(&mut app);
    assert_eq!(axes(&app).into_iter().collect::<Vec<_>>(), [("wght".to_string(), 650.0)]);
    // Weights differ across the selection, widths agree: a change of the width leaves the weights alone.
    let bold =
        app.run("text.create", json!({"x": 20, "y": 160, "text": "lIn", "font": TWO_AXIS_FAMILY, "style": "Bold"})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [id, bold]})).unwrap();
    let mixed = values(&mut app).mixed_axes;
    assert!(mixed.contains(b"wght") && !mixed.contains(b"wdth"), "{mixed:?}");
    change(&mut app, CharacterChange::Axes(vec![(*b"wdth", 90.0)]));
    let font_axes = |app: &VectorcraftApp, id: u64| {
        story_frame(app, id).runs.iter().map(|r| (r.style.font_axes.get("wght").copied(), r.style.font_axes.get("wdth").copied())).collect::<Vec<_>>()
    };
    assert_eq!(font_axes(&app, id), [(Some(650.0), Some(90.0))], "the mixed axis is untouched");
    assert_eq!(font_axes(&app, bold), [(None, Some(90.0))]);
    // The Type tool's range takes the change, not the whole object.
    app.run("select.set", json!({"ids": [bold]})).unwrap();
    app.select_tool("type");
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    app.session.set_tool_option("select", &json!({"start": 0, "end": 1}));
    change(&mut app, CharacterChange::Axes(vec![(*b"wght", 500.0)]));
    assert_eq!(shared(&app, |s| s.font_axes.get("wght").copied()), Some(Some(500.0)));
    app.session.set_tool_option("select", &json!({"start": 1, "end": 3}));
    assert_eq!(shared(&app, |s| s.font_axes.get("wght").copied()), Some(None), "only the selected range changed");
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

/// Each run's (small caps, position).
fn scripts(app: &VectorcraftApp, id: &Value) -> Vec<(Option<f64>, &'static str)> {
    match &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind {
        NodeKind::Text(t) => t.runs.iter().map(|r| (r.style.small_caps, r.style.position.id())).collect(),
        _ => panic!("text"),
    }
}

/// Superscript and Subscript decide from the characters they change. At a Type tool caret they
/// change every character of the text, so text that is only partly superscript turns superscript
/// everywhere, with the caret after a superscript character too, and the field there shows off.
/// Superscripts in different proportions count as superscript, so a press turns them off.
#[test]
fn superscript_and_subscript_decide_from_the_characters_they_change() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "x2", "size": 12})).unwrap()["id"].clone();
    app.run("text.setRangeStyle", json!({"id": id, "start": 1, "end": 2, "position": "superscript"})).unwrap();
    app.run("select.set", json!({"ids": [id.clone()]})).unwrap();
    app.select_tool("type");
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    app.session.set_tool_option("select", &json!({"start": 2, "end": 2}));
    assert_eq!(text_editing(&app).map(|(_, a, b)| (a, b)), Some((2, 2)));
    assert_ne!(values(&mut app).position, Some(BaselinePosition::Superscript), "the text is not all superscript");
    assert_eq!(app.run("type.superscript", json!({})).unwrap(), json!("superscript"));
    assert!(scripts(&app, &id).iter().all(|(_, p)| *p == "superscript"), "{:?}", scripts(&app, &id));
    app.run("type.superscript", json!({})).unwrap();
    assert!(scripts(&app, &id).iter().all(|(_, p)| *p == "normal"), "{:?}", scripts(&app, &id));

    // A file whose two superscript runs have different proportions.
    app.select_tool("selection");
    app.run("text.setRangeStyle", json!({"id": id, "position": "superscript"})).unwrap();
    app.run("text.setRangeStyle", json!({"id": id, "start": 1, "end": 2, "underline": true})).unwrap();
    let mut d = (*app.session.active().unwrap().doc).clone();
    if let Some(NodeKind::Text(t)) = d.node_mut(NodeId(id.as_u64().unwrap())).map(|n| &mut n.kind) {
        assert_eq!(t.runs.len(), 2);
        t.runs[0].style.position = vectorcraft_doc::CharPosition::Superscript(vectorcraft_doc::ScriptMetrics { size: 50.0, position: 40.0 });
    }
    let data = vectorcraft_format::base64_encode(&vectorcraft_format::save_file(&d));
    app.run("document.open", json!({"name": "scripts.vectorcraft", "dataBase64": data})).unwrap();
    app.run("select.set", json!({"ids": [id.clone()]})).unwrap();
    let runs = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind {
        NodeKind::Text(t) => t.runs.iter().map(|r| r.style.position).collect::<Vec<_>>(),
        _ => panic!("text"),
    };
    let before = runs(&app);
    assert!(before.len() == 2 && before[0] != before[1] && before.iter().all(|p| p.id() == "superscript"), "{before:?}");
    assert_eq!(values(&mut app).position, Some(BaselinePosition::Superscript), "the field shows superscript");
    assert_eq!(app.run("type.superscript", json!({})).unwrap(), json!("normal"));
    assert!(scripts(&app, &id).iter().all(|(_, p)| *p == "normal"), "{:?}", scripts(&app, &id));
}

/// With Real-time Drawing off, Superscript, Subscript and Underline decide from the text with the
/// typing applied: after a Backspace not yet applied takes out the one plain character, the rest
/// is all superscript (or underlined), so a press turns it off.
#[test]
fn superscript_and_underline_decide_from_the_text_with_the_typing_applied() {
    for (command, set, off) in
        [("type.superscript", json!({"position": "superscript"}), json!("normal")), ("type.underline", json!({"underline": true}), json!(false))]
    {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        app.session.execute("prefs.set", &json!({"key": "realTimeDrawing", "value": false})).unwrap();
        let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "x2", "size": 12})).unwrap()["id"].clone();
        let mut range = set.clone();
        range["id"] = id.clone();
        range["start"] = json!(1);
        range["end"] = json!(2);
        app.run("text.setRangeStyle", range).unwrap();
        app.run("select.set", json!({"ids": [id.clone()]})).unwrap();
        app.select_tool("type");
        app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
        app.session.set_tool_option("select", &json!({"start": 1, "end": 1}));
        let mut app = crate::testing::windowed(app);
        let ctx = app.ui_ctx.clone().unwrap();
        let backspace =
            |pressed| egui::Event::Key { key: egui::Key::Backspace, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
        // The keymap builds in the first pass.
        crate::tests::synthetic::key_frame(&mut app, &ctx, vec![]);
        crate::tests::synthetic::key_frame(&mut app, &ctx, vec![backspace(true), backspace(false)]);
        assert_eq!(app.session.tool_options()["typing"], json!(true), "{command}");
        let node = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind {
            NodeKind::Text(t) => (t.plain_text(), t.runs.iter().map(|r| (r.style.position.id(), r.style.underline)).collect::<Vec<_>>()),
            _ => panic!("text"),
        };
        assert_eq!(node(&app).0, "x2", "{command}: the Backspace is not applied yet");
        assert_eq!(app.run(command, json!({})).unwrap(), off, "{command}");
        assert_eq!(node(&app), ("2".to_string(), vec![("normal", false)]), "{command}");
    }
}

/// Each change the controls raise runs its commands with the panel's params: the size, Auto leading,
/// the caps and position choices, the anti-aliasing method, kerning, and the East Asian and Middle
/// Eastern values clamped to their ranges. Each takes one undo step, on the selected type and on the
/// Type tool's range.
#[test]
fn a_change_runs_its_commands_with_the_panels_params() {
    use vectorcraft_doc::text::{MAX_AKI, MAX_DIACRITIC_POSITION, MAX_TSUME};
    type Check = fn(&CharacterValues) -> bool;
    let table: [(&str, CharacterChange, Check); 9] = [
        ("Size", CharacterChange::Size(24.0), |v| near(v.size, 24.0)),
        ("Leading(Auto)", CharacterChange::Leading(Leading::Auto), |v| matches!(v.leading, Some(Leading::Auto))),
        ("Caps(SmallCaps)", CharacterChange::Caps(Caps::SmallCaps), |v| matches!(v.caps, Some(Caps::SmallCaps))),
        ("Position(Superscript)", CharacterChange::Position(BaselinePosition::Superscript), |v| {
            matches!(v.position, Some(BaselinePosition::Superscript))
        }),
        ("AntiAlias(Crisp)", CharacterChange::AntiAlias(TextAntiAlias::Crisp), |v| matches!(v.anti_alias, Some(TextAntiAlias::Crisp))),
        ("Kerning(Optical)", CharacterChange::Kerning(Kerning::Optical), |v| matches!(v.kerning, Some(Kerning::Optical))),
        ("AkiLeft past the maximum", CharacterChange::AkiLeft(2000.0), |v| near(v.aki_left, MAX_AKI)),
        ("Tsume past the maximum", CharacterChange::Tsume(500.0), |v| near(v.tsume, MAX_TSUME)),
        ("DiacriticX past the maximum", CharacterChange::DiacriticX(5000.0), |v| near(v.diacritic_x, MAX_DIACRITIC_POSITION)),
    ];
    let new_app = || {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "（日本）", "size": 12})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        app.run("text.setStyle", json!({"leading": 30})).unwrap();
        app
    };
    for (label, c, check) in table {
        let mut app = new_app();
        assert!(!check(&values(&mut app)), "{label}: not set yet");
        let undo = undo_steps(&app);
        change(&mut app, c);
        assert!(check(&values(&mut app)), "{label}");
        assert_eq!(undo_steps(&app), undo + 1, "{label}: one undo step");
    }
    // Pressing the pressed position again is Position(Normal): back to normal.
    let mut app = new_app();
    change(&mut app, CharacterChange::Position(BaselinePosition::Superscript));
    change(&mut app, CharacterChange::Position(BaselinePosition::Normal));
    assert_eq!(values(&mut app).position, Some(BaselinePosition::Normal));
    // The Type tool's selection takes it, the rest of the text does not.
    let mut app = new_app();
    app.select_tool("type");
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    app.session.set_tool_option("select", &json!({"start": 0, "end": 3}));
    change(&mut app, CharacterChange::Caps(Caps::SmallCaps));
    assert_eq!(values(&mut app).caps, Some(Caps::SmallCaps));
    app.session.set_tool_option("select", &json!({"start": 3, "end": 6}));
    assert_eq!(values(&mut app).caps, Some(Caps::Normal), "the rest of the text is unchanged");
    // The method belongs to the object: another with a different one leaves the field blank.
    let mut app = new_app();
    change(&mut app, CharacterChange::AntiAlias(TextAntiAlias::Crisp));
    let first = story_selected_ids(&app);
    // A new object is selected once made: the change above went to the first one alone, as production's steps did.
    let other = app.run("text.create", json!({"x": 20, "y": 120, "text": "Bremen", "size": 12})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [first[0], other]})).unwrap();
    assert!(values(&mut app).anti_alias.is_none(), "the methods differ");
}

/// The ids of the selected objects.
fn story_selected_ids(app: &VectorcraftApp) -> Vec<u64> {
    app.session.active().unwrap().selection.objects.iter().map(|id| id.0).collect()
}

/// The panel menu's rows run their commands and check by state: the toggles on the text and the
/// document, the East Asian and Middle Eastern rows only with their preferences, the dialogs, and the
/// submenus' checks (none for a mixed selection).
#[test]
fn the_panel_menu_rows_run_their_commands_and_check_by_state() {
    use craft::ui::dialog::dialog_id;
    use craft::ui::patterns::form_dialogs::HostedParamDialog;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    // The settings rows open their dialogs on the window's host.
    app = crate::testing::windowed(app);
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "ABC", "size": 13})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    // PLAN: assumed row ids and params (type-w10's table): `ui.character.toggle {key}` and `ui.character.set {key, value}`.
    let toggle = |key: &str| CommandRef::with_params("ui.character.toggle", json!({"key": key}));
    let set = |key: &str, value: &str| CommandRef::with_params("ui.character.set", json!({"key": key, "value": value}));
    type On = fn(&VectorcraftApp, u64) -> bool;
    let rows: [(CommandRef, bool, On); 9] = [
        (toggle("smallCaps"), false, |a, id| story_frame(a, id).runs.iter().all(|r| r.style.small_caps.is_some())),
        (CommandRef::new("type.superscript"), false, |a, id| story_frame(a, id).runs.iter().all(|r| r.style.position.id() == "superscript")),
        (CommandRef::new("type.subscript"), false, |a, id| story_frame(a, id).runs.iter().all(|r| r.style.position.id() == "subscript")),
        (CommandRef::new("type.systemLayout"), false, |a, id| {
            a.session.active().unwrap().doc.setup.system_layout && story_frame(a, id).system_layout
        }),
        (CommandRef::new("type.fractionalWidths"), false, |a, id| story_frame(a, id).fractional_widths),
        (toggle("tateChuYoko"), true, |a, id| story_frame(a, id).runs.iter().all(|r| r.style.tate_chu_yoko)),
        (toggle("warichu"), true, |a, id| story_frame(a, id).runs.iter().all(|r| r.style.warichu)),
        (toggle("standardVerticalRoman"), true, |a, id| story_frame(a, id).runs.iter().all(|r| r.style.standard_vertical_roman)),
        (toggle("noBreak"), false, |a, id| story_frame(a, id).runs.iter().all(|r| r.style.no_break)),
    ];
    for (command, east_asian, _) in &rows {
        assert_eq!(listed(&app).contains(command), !east_asian, "{}: East Asian rows only with their preference", command.key());
    }
    let (alignment, settings) = (set("charAlign", "emBoxTop"), CommandRef::new("ui.tateChuYokoSettings"));
    let (digits, direction) = (set("digits", "tamil"), set("charDirection", "rightToLeft"));
    let (kashidas, warichu_settings) = (toggle("kashidas"), CommandRef::new("ui.warichuSettings"));
    for hidden in [&alignment, &settings, &warichu_settings, &digits, &direction, &kashidas] {
        assert!(!listed(&app).contains(hidden), "{} needs its preference", hidden.key());
    }
    app.session.prefs.show_east_asian_options = true;
    app.session.prefs.show_indic_options = true;
    for (command, _, on) in &rows {
        assert!(listed(&app).contains(command), "{}", command.key());
        let start = on(&app, id);
        assert_eq!(state(&app, command), (true, start), "{}", command.key());
        fire(&mut app, command);
        assert_eq!(on(&app, id), !start, "{}: run", command.key());
        assert_eq!(state(&app, command).1, !start, "{}: checked after", command.key());
        fire(&mut app, command);
        assert_eq!(on(&app, id), start, "{}: run again", command.key());
    }
    // The settings rows open their dialogs.
    for (row, kind) in [(&settings, crate::dialogs::tate_chu_yoko::KIND), (&warichu_settings, crate::dialogs::warichu::KIND)] {
        assert!(listed(&app).contains(row), "{}", row.key());
        fire(&mut app, row);
        let opened = app.dialogs().is_some_and(|d| d.with(|h| h.get_by_id::<HostedParamDialog>(dialog_id(kind)).is_some()));
        assert!(opened, "{}", row.key());
    }
    // Submenus check the value the selected characters share, and none when they differ.
    for row in [&alignment, &digits, &direction] {
        assert!(listed(&app).contains(row), "{}", row.key());
        assert!(!state(&app, row).1, "{}", row.key());
    }
    app.run("text.setRangeStyle", json!({"id": id, "start": 0, "end": 1, "charAlign": "emBoxTop", "digits": "tamil"})).unwrap();
    assert!(!state(&app, &alignment).1 && !state(&app, &digits).1, "the characters differ");
    app.run("text.setRangeStyle", json!({"id": id, "charAlign": "emBoxTop", "digits": "tamil"})).unwrap();
    assert!(state(&app, &alignment).1 && state(&app, &digits).1, "the characters share them");
    fire(&mut app, &set("digits", "hindi"));
    assert!(state(&app, &set("digits", "hindi")).1 && !state(&app, &digits).1, "a pick moves the check");
}

/// The panel's rows follow the East Asian and Middle Eastern preferences and the Font Height option,
/// which Hide Options leaves alone.
#[test]
fn the_panels_rows_follow_the_preferences_and_hide_options() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "Hx", "size": 12})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    let has = |app: &VectorcraftApp, group: CharacterRows| rows(app).contains(group);
    assert!(!has(&app, CharacterRows::EAST_ASIAN) && !has(&app, CharacterRows::MIDDLE_EASTERN) && !has(&app, CharacterRows::FONT_HEIGHT));
    app.session.prefs.show_east_asian_options = true;
    assert!(has(&app, CharacterRows::EAST_ASIAN) && !has(&app, CharacterRows::MIDDLE_EASTERN));
    app.session.prefs.show_indic_options = true;
    assert!(has(&app, CharacterRows::EAST_ASIAN) && has(&app, CharacterRows::MIDDLE_EASTERN));
    // PLAN: assumed row id and params (type-w10's table).
    let font_height = CommandRef::with_params("ui.character.show", json!({"option": "fontHeight"}));
    assert!(listed(&app).contains(&font_height));
    fire(&mut app, &font_height);
    assert!(app.ui.character_font_height && has(&app, CharacterRows::FONT_HEIGHT), "Show Font Height Options");
    app.ui.character_hide_options = true;
    assert!(has(&app, CharacterRows::FONT_HEIGHT), "Hide Options keeps the font height reference");
    app.ui.character_hide_options = false;
    fire(&mut app, &font_height);
    assert!(!app.ui.character_font_height && !has(&app, CharacterRows::FONT_HEIGHT), "Hide Font Height Options");
}

/// Reset Panel closes only the undo group it opens: in an undo group that is already open,
/// the reset is part of that group's step, and an undo group open in another document stays
/// open, with that document's later edits part of its step.
#[test]
fn reset_panel_closes_only_its_own_undo_group() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let a = app.run("text.create", json!({"x": 20, "y": 50, "text": "A"})).unwrap()["id"].clone();
    let steps = |st: &vectorcraft_engine::DocState| (st.history.undo.len(), st.history.undo.last().map(|e| e.label.clone()));
    // An undo group open in the same document (MCP `command_batch` with `atomic`).
    app.run("edit.undoGroup.begin", json!({"label": "Batch"})).unwrap();
    app.run("text.setFormat", json!({"id": a, "antiAlias": "crisp"})).unwrap();
    reset_panel(&mut app);
    assert!(app.session.active().unwrap().undo_group.is_some(), "the batch's undo group stays open");
    app.run("text.setFormat", json!({"id": a, "antiAlias": "strong"})).unwrap();
    app.run("edit.undoGroup.end", json!({})).unwrap();
    let a_steps = steps(app.session.active().unwrap());
    assert_eq!(a_steps.1, Some("Batch".to_string()), "the reset is part of the Batch step");
    // An undo group open in another document. The second document is open before the group
    // begins, so switching to it opens no group there.
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    assert_eq!(app.session.active_index(), Some(1));
    let b = app.run("text.create", json!({"x": 20, "y": 50, "text": "B"})).unwrap()["id"].clone();
    // A reset that changes nothing records no step, so B starts away from the panel's defaults.
    app.run("text.setFormat", json!({"id": b, "antiAlias": "crisp"})).unwrap();
    app.session.set_active(0);
    app.run("edit.undoGroup.begin", json!({"label": "Batch"})).unwrap();
    app.run("text.setFormat", json!({"id": a, "antiAlias": "none"})).unwrap();
    let a_steps = steps(app.session.active().unwrap());
    app.session.set_active(1);
    assert!(app.session.active().unwrap().undo_group.is_none());
    app.run("select.set", json!({"ids": [b]})).unwrap();
    let b_steps = steps(app.session.active().unwrap()).0;
    reset_panel(&mut app);
    assert_eq!(steps(app.session.active().unwrap()), (b_steps + 1, Some("Reset Panel".to_string())));
    assert!(app.session.active().unwrap().undo_group.is_none());
    assert!(app.session.documents()[0].undo_group.is_some(), "the other document's undo group stays open");
    app.session.set_active(0);
    app.run("text.setFormat", json!({"id": a, "antiAlias": "crisp"})).unwrap();
    app.run("edit.undoGroup.end", json!({})).unwrap();
    assert_eq!(steps(app.session.active().unwrap()), a_steps, "the later edit is part of the Batch step");
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

#[test]
fn shared_text_flag_vertical_three_object_selections() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({})).unwrap();
    let v0 = app.run("text.create", json!({"x": 10, "y": 40, "text": "v0", "vertical": true})).unwrap()["id"].clone();
    let h1 = app.run("text.create", json!({"x": 10, "y": 80, "text": "h1"})).unwrap()["id"].clone();
    let h2 = app.run("text.create", json!({"x": 10, "y": 120, "text": "h2"})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [v0.clone(), h1.clone(), h2.clone()]})).unwrap();
    assert_eq!(shared_text_flag(&app, |t| t.vertical), None);
    let h0 = app.run("text.create", json!({"x": 10, "y": 160, "text": "h0"})).unwrap()["id"].clone();
    let v1 = app.run("text.create", json!({"x": 10, "y": 200, "text": "v1", "vertical": true})).unwrap()["id"].clone();
    let v2 = app.run("text.create", json!({"x": 10, "y": 240, "text": "v2", "vertical": true})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [h0.clone(), v1.clone(), v2.clone()]})).unwrap();
    assert_eq!(shared_text_flag(&app, |t| t.vertical), None);
    app.run("select.set", json!({"ids": [v0.clone(), v1.clone(), v2.clone()]})).unwrap();
    assert_eq!(shared_text_flag(&app, |t| t.vertical), Some(true));
}

/// `type.underline` does what the Underline button does: with only some of the selected characters
/// underlined it underlines all of them, and with all of them underlined it turns underlining off.
#[test]
fn underline_command_underlines_a_mixed_selection() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "AB", "size": 12})).unwrap()["id"].as_u64().unwrap();
    app.run("text.setRangeStyle", json!({"id": id, "start": 0, "end": 1, "underline": true})).unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    let underlined = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.runs.iter().map(|r| r.style.underline).collect::<Vec<_>>(),
        _ => panic!("text"),
    };
    assert_eq!(super::toggle_underline(&mut app), Ok(json!(true)));
    assert!(underlined(&app).iter().all(|u| *u), "{:?}", underlined(&app));
    assert_eq!(super::toggle_underline(&mut app), Ok(json!(false)));
    assert!(underlined(&app).iter().all(|u| !*u), "{:?}", underlined(&app));
}

/// At a caret in text the Type tool edits, with no characters selected, the Character
/// Direction rows are disabled: running one leaves the direction of every character as it was.
#[test]
fn character_direction_does_nothing_at_an_empty_caret() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.prefs.show_indic_options = true;
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
    // PLAN: assumed row id and params (type-w10's table).
    let rtl = CommandRef::with_params("ui.character.set", json!({"key": "charDirection", "value": "rightToLeft"}));
    assert!(listed(&app).contains(&rtl));
    assert!(!state(&app, &rtl).0, "disabled at an empty caret");
    // The result is not the point: a disabled row must leave the text as it was.
    let _ = run(&mut app, &rtl.id, &rtl.params);
    assert_eq!(directions(&app), before);
    // With characters selected, the same row sets their direction.
    app.session.set_tool_option("select", &json!({"start": 0, "end": 4}));
    assert!(state(&app, &rtl).0);
    fire(&mut app, &rtl);
    assert_ne!(directions(&app), before);
}

/// Kashidas appears with Middle Eastern options, starts checked, and a mixed selection is
/// unchecked until the user enables all of its characters.
#[test]
fn kashidas_menu_item_handles_default_and_mixed_ranges() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.prefs.show_indic_options = true;
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "AB"})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    // PLAN: assumed row id and params (type-w10's table).
    let row = CommandRef::with_params("ui.character.toggle", json!({"key": "kashidas"}));
    assert!(listed(&app).contains(&row));
    assert!(state(&app, &row).1);
    app.run("text.setRangeStyle", json!({"id": id, "start": 0, "end": 1, "kashidas": false})).unwrap();
    assert!(!state(&app, &row).1);
    fire(&mut app, &row);
    assert!(state(&app, &row).1);
    let kashidas = |app: &VectorcraftApp| story_frame(app, id).runs.iter().map(|r| r.style.kashidas).collect::<Vec<_>>();
    assert!(kashidas(&app).iter().all(|v| *v));
    app.select_tool("type");
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    app.session.set_tool_option("select", &json!({"start": 1, "end": 1}));
    assert!(!state(&app, &row).0, "disabled at an empty caret");
    // The result is not the point: a disabled row must leave the text as it was.
    let _ = run(&mut app, &row.id, &row.params);
    assert!(kashidas(&app).iter().all(|v| *v), "an empty caret leaves all characters unchanged");
}

/// Reset Panel sets every character attribute the panel shows, except the font family, style
/// and size, back to its default (Small Caps, the position, Character Alignment, Proportional
/// Metrics, Tate-chu-yoko and its settings, No Break, Character Direction, Digits and the font height reference
/// included), in the selected type and in the text the Type tool has selected while it edits.
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
            s.tcy_up_down != d.tcy_up_down,
            s.tcy_left_right != d.tcy_left_right,
            s.aki_left != d.aki_left,
            s.aki_right != d.aki_right,
            s.tsume != d.tsume,
            s.standard_vertical_roman != d.standard_vertical_roman,
            s.no_break != d.no_break,
            s.char_direction != d.char_direction,
            s.digits != d.digits,
            s.font_height_reference != d.font_height_reference,
            s.diacritic_horizontal != d.diacritic_horizontal,
            s.diacritic_vertical != d.diacritic_vertical,
        ]
    };
    let custom = |app: &mut VectorcraftApp| {
        app.run(
            "text.setRangeStyle",
            json!({
                "id": id, "tracking": 50, "leading": 30, "kerning": 20, "baselineShift": 3, "hScale": 120, "vScale": 90, "rotation": 15,
                "underline": true, "strikethrough": true, "allCaps": true, "smallCaps": true, "position": "superscript",
                "charAlign": "emBoxCenter", "proportionalMetrics": true, "tateChuYoko": true, "tcyUpDown": 2, "tcyLeftRight": -3,
                "akiLeft": 250, "akiRight": 500, "tsume": 50, "standardVerticalRoman": true, "noBreak": true, "charDirection": "rightToLeft", "digits": "hindi", "fontHeightReference": "capHeight",
                "diacriticHorizontal": 120, "diacriticVertical": -80
            }),
        )
        .unwrap();
    };
    let plain = runs(&app)[0].1.clone();
    assert_eq!(differs(&plain, &CharStyle::default()), [false; 27]);
    custom(&mut app);
    let styled = runs(&app)[0].1.clone();
    assert_eq!(differs(&styled, &CharStyle::default()), [true; 27]);
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

/// #4232: the panel menu's Show Snap to Glyph Options shows the Snap to Glyph section (with no
/// type selected too) and Hide Snap to Glyph Options hides it; its checkboxes, dimmed while View ›
/// Snap to Glyph is off, run `view.snapToGlyph.options`.
#[test]
fn snap_to_glyph_options_show_from_the_menu_and_set_the_options() {
    use crate::tests::panel_harness::Panel;
    // PLAN: assumed row id and params (type-w10's table).
    let toggle = CommandRef::with_params("ui.character.show", json!({"option": "snapToGlyph"}));
    let labels = ["Baseline", "x-height", "Glyph Bounds", "Proximity Guides", "Angular Guides", "Anchor Point"];
    let mut panel = Panel::new(VectorcraftApp::new(Session::new(), Default::default()), show);
    panel.frame();
    assert!(!panel.shows("Proximity Guides"), "hidden at first");
    fire(&mut panel.app(), &toggle);
    assert!(panel.app().ui.snap_to_glyph_options);
    panel.frame();
    assert!(labels.iter().all(|l| panel.shows(l)), "every option of the block shows");
    assert!(!panel.enabled("Angular Guides"), "dimmed while Snap to Glyph is off");
    panel.click("Angular Guides");
    assert!(panel.app().session.prefs.glyph_snap.angular, "a dimmed checkbox does nothing");
    panel.app().ui.view.snap_to_glyph = true;
    panel.frame();
    assert!(panel.enabled("Angular Guides"));
    panel.click("Angular Guides");
    panel.click("x-height");
    let o = panel.app().session.prefs.glyph_snap;
    assert!(!o.angular && !o.x_height && o.baseline && o.bounds && o.proximity && o.anchors, "{o:?}");
    panel.click("Angular Guides");
    assert!(panel.app().session.prefs.glyph_snap.angular);
    fire(&mut panel.app(), &toggle);
    panel.frame();
    assert!(!panel.shows("Proximity Guides"));
}

/// #5273: the Snap to Glyph options come after every character attribute, so showing them from the
/// panel menu asks the panel to scroll to them; showing them does not turn Snap to Glyph on, their
/// first checkbox does.
#[test]
fn snap_to_glyph_options_scroll_into_view_and_turn_snapping_on() {
    use crate::tests::panel_harness::Panel;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let id = app.run("text.create", json!({"x": 10, "y": 40, "text": "Glyphs"})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    let mut panel = Panel::new(app, show);
    // PLAN: assumed row id and params (type-w10's table).
    fire(&mut panel.app(), &CommandRef::with_params("ui.character.show", json!({"option": "snapToGlyph"})));
    assert!(panel.app().ui.snap_to_glyph_reveal, "the menu asks for the scroll");
    panel.frame();
    assert!(!panel.app().ui.view.snap_to_glyph, "showing the options leaves snapping off");
    panel.click("Snap to Glyph");
    assert!(panel.app().ui.view.snap_to_glyph, "the first checkbox turns Snap to Glyph on");
    panel.click("Anchor Point");
    assert!(!panel.app().session.prefs.glyph_snap.anchors, "the options take clicks once it is on");
}

/// Copy and Cut take a selected story after a frame threaded in before the edited one took its whole
/// text.
#[test]
fn copy_and_cut_take_a_story_that_left_the_edited_frame() {
    for cut in [false, true] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 600, "height": 300})).unwrap();
        let story = "The quick brown fox jumps over the lazy dog and keeps running far beyond the first frame. ".repeat(4);
        let area = json!({"x": 10, "y": 10, "text": story, "size": 12, "area": {"width": 150, "height": 60}});
        let a = app.run("text.create", area).unwrap()["id"].as_u64().unwrap();
        let b = app.run("shape.rectangle", json!({"x": 200, "y": 10, "width": 150, "height": 200})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        app.run("text.thread.create", json!({})).unwrap();
        app.session.select_tool("type", Default::default()).unwrap();
        app.session.set_tool_option("textSelection", &json!({"editing": a, "anchor": 0, "caret": 0}));
        app.run("select.all", json!({})).unwrap();
        let c = app.run("text.thread.add", json!({"from": a, "before": true, "rect": [700, 10, 400, 600]})).unwrap()["id"].as_u64().unwrap();
        let text = |app: &VectorcraftApp, id: u64| match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => t.plain_text(),
            _ => panic!("not text"),
        };
        assert_eq!((text(&app, a), text(&app, b), text(&app, c)), (String::new(), String::new(), story.clone()));
        assert!(intercept_text_command(&mut app, if cut { "edit.cut" } else { "edit.copy" }).is_some());
        let copied: Vec<String> = app
            .session
            .clipboard
            .nodes
            .iter()
            .filter_map(|n| match &n.kind {
                NodeKind::Text(t) => Some(t.plain_text()),
                _ => None,
            })
            .collect();
        assert_eq!(copied, std::slice::from_ref(&story), "cut: {cut}");
        assert_eq!(text(&app, c).is_empty(), cut);
        if cut {
            // A story that holds no text has nothing to copy, so Cut does not run its delete.
            app.run("select.all", json!({})).unwrap();
            assert_eq!(text_story(&app).map(|frames| frames.len()), Some(3));
            assert!(!text_copy(&mut app, false));
        }
    }
}

/// After Select All in a frame of threaded text, the Character and Paragraph panels and Copy act on
/// the text of every frame of the story.
#[test]
fn select_all_in_threaded_text_formats_and_copies_the_whole_story() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 600, "height": 300})).unwrap();
    let story = "The quick brown fox jumps over the lazy dog and keeps running far beyond the first frame. ".repeat(4);
    let area = json!({"x": 10, "y": 10, "text": story, "size": 12, "area": {"width": 150, "height": 60}});
    let a = app.run("text.create", area).unwrap()["id"].as_u64().unwrap();
    let b = app.run("shape.rectangle", json!({"x": 200, "y": 10, "width": 150, "height": 200})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    app.run("text.thread.create", json!({})).unwrap();
    app.run("select.set", json!({"ids": [b]})).unwrap();
    app.session.select_tool("type", Default::default()).unwrap();
    app.session.set_tool_option("textSelection", &json!({"editing": b, "anchor": 0, "caret": 0}));
    app.run("select.all", json!({})).unwrap();
    assert_eq!(text_story(&app), Some(vec![NodeId(a), NodeId(b)]));
    let frame = |app: &VectorcraftApp, id: u64| match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("not text"),
    };
    format(&mut app, json!({"size": 9}));
    crate::panels::paragraph::run_on_paragraphs(&mut app, "text.setFormat", json!({"spaceBefore": 4})).unwrap();
    for id in [a, b] {
        let t = frame(&app, id);
        assert!(t.runs.iter().all(|r| (r.style.size - 9.0).abs() < 1e-9), "every character of frame {id} is 9 pt");
        assert!((t.para_at(0).space_before - 4.0).abs() < 1e-9, "frame {id}'s paragraphs take the space");
    }
    assert!(intercept_text_command(&mut app, "edit.copy").is_some());
    let copied: Vec<String> = app
        .session
        .clipboard
        .nodes
        .iter()
        .filter_map(|n| match &n.kind {
            NodeKind::Text(t) => Some(t.plain_text()),
            _ => None,
        })
        .collect();
    assert_eq!(copied, [story]);
}

/// A story threaded through three frames in order, the first `height` tall.
fn three_frame_story(height: u32) -> (VectorcraftApp, u64, u64, u64) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 900, "height": 300})).unwrap();
    let story = "The quick brown fox jumps over the lazy dog and keeps running far beyond the first frame. ".repeat(6);
    let area = json!({"x": 10, "y": 10, "text": story, "size": 12, "area": {"width": 150, "height": height}});
    let a = app.run("text.create", area).unwrap()["id"].as_u64().unwrap();
    let b = app.run("text.thread.add", json!({"from": a, "rect": [200, 10, 150, 60]})).unwrap()["id"].as_u64().unwrap();
    let c = app.run("text.thread.add", json!({"from": b, "rect": [400, 10, 150, 200]})).unwrap()["id"].as_u64().unwrap();
    (app, a, b, c)
}

/// Select All with the Type tool in frame `id` of a story.
fn select_all_from(app: &mut VectorcraftApp, id: u64) {
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.session.select_tool("type", Default::default()).unwrap();
    app.session.set_tool_option("textSelection", &json!({"editing": id, "anchor": 0, "caret": 0}));
    app.run("select.all", json!({})).unwrap();
}

fn story_frame(app: &VectorcraftApp, id: u64) -> vectorcraft_doc::TextObject {
    match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("not text"),
    }
}

fn story_sizes(app: &VectorcraftApp, id: u64) -> Vec<f64> {
    story_frame(app, id).runs.iter().map(|r| r.style.size).collect()
}

/// The frames that Select All saved are not the frames of the story once the first of them has
/// left the thread: the edited frame's story takes the change.
#[test]
fn story_changes_follow_the_edited_frames_thread_after_the_first_saved_frame_leaves() {
    // The first frame is too short for a line, so Select All edits the second.
    let (mut app, a, b, c) = three_frame_story(1);
    select_all_from(&mut app, c);
    assert_eq!(app.session.tool_options()["story"], json!([a, b, c]));
    assert_eq!(text_editing(&app).unwrap().0, NodeId(b));
    app.run("text.thread.break", json!({"id": a, "port": "out"})).unwrap();
    assert_eq!(text_editing(&app).unwrap().0, NodeId(b));

    assert_eq!(text_story(&app), Some(vec![NodeId(b), NodeId(c)]));
    format(&mut app, json!({"size": 9}));
    assert!(story_sizes(&app, a).iter().all(|s| (*s - 12.0).abs() < 1e-9), "the frame that left keeps its size");
    for id in [b, c] {
        assert!(story_sizes(&app, id).iter().all(|s| (*s - 9.0).abs() < 1e-9), "frame {id} takes the size");
    }
}

/// A frame that left the thread after Select All takes no change, and a frame added to it does.
#[test]
fn story_changes_leave_out_a_frame_that_left_and_take_in_a_frame_that_joined() {
    let (mut app, a, b, c) = three_frame_story(60);
    select_all_from(&mut app, c);
    assert_eq!(text_editing(&app).unwrap().0, NodeId(a));
    app.run("text.thread.break", json!({"id": b, "port": "out"})).unwrap();
    assert_eq!(text_story(&app), Some(vec![NodeId(a), NodeId(b)]));
    let d = app.run("text.thread.add", json!({"from": a, "rect": [600, 10, 150, 60]})).unwrap()["id"].as_u64().unwrap();
    assert_eq!(text_story(&app), Some(vec![NodeId(a), NodeId(d), NodeId(b)]));

    crate::panels::paragraph::run_on_paragraphs(&mut app, "text.setFormat", json!({"spaceBefore": 4})).unwrap();
    for id in [a, d, b] {
        assert!((story_frame(&app, id).para_at(0).space_before - 4.0).abs() < 1e-9, "frame {id} takes the space");
    }
    assert!(story_frame(&app, c).para_at(0).space_before.abs() < 1e-9, "the frame that left keeps its paragraphs");
}

/// With the thread gone, the selection is in the one edited frame.
#[test]
fn story_changes_stay_in_the_edited_frame_once_its_thread_is_removed() {
    let (mut app, a, b, c) = three_frame_story(60);
    select_all_from(&mut app, c);
    assert_eq!(text_editing(&app).unwrap().0, NodeId(a));
    app.run("text.thread.remove", json!({})).unwrap();
    assert_eq!(text_story(&app), None);

    crate::panels::paragraph::run_on_paragraphs(&mut app, "text.setFormat", json!({"spaceBefore": 4})).unwrap();
    assert!((story_frame(&app, a).para_at(0).space_before - 4.0).abs() < 1e-9);
    for id in [b, c] {
        assert!(story_frame(&app, id).para_at(0).space_before.abs() < 1e-9, "frame {id} keeps its paragraphs");
    }
}

/// A selected story is one text: its locked and hidden frames take the change too.
#[test]
fn story_changes_take_in_locked_and_hidden_frames() {
    let (mut app, a, b, c) = three_frame_story(60);
    app.run("select.set", json!({"ids": [b]})).unwrap();
    app.run("object.lock", json!({})).unwrap();
    app.run("select.set", json!({"ids": [c]})).unwrap();
    app.run("object.hide", json!({})).unwrap();
    select_all_from(&mut app, a);
    assert_eq!(text_story(&app), Some(vec![NodeId(a), NodeId(b), NodeId(c)]));

    format(&mut app, json!({"size": 9}));
    for id in [a, b, c] {
        assert_eq!(story_sizes(&app, id).iter().filter(|s| (**s - 9.0).abs() > 1e-9).count(), 0, "frame {id} takes the size");
    }
}

/// [`three_frame_story`] with its second frame locked and its third hidden, a type object of its own
/// below it, and the first frame selected with the Selection tool → the app, the frames and the
/// other type object.
fn story_selected_in_its_first_frame() -> (VectorcraftApp, [u64; 3], u64) {
    let (mut app, a, b, c) = three_frame_story(60);
    let other = app.run("text.create", json!({"x": 10, "y": 250, "text": "Other", "size": 12})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [b]})).unwrap();
    app.run("object.lock", json!({})).unwrap();
    app.run("select.set", json!({"ids": [c]})).unwrap();
    app.run("object.hide", json!({})).unwrap();
    app.run("select.set", json!({"ids": [a]})).unwrap();
    assert_eq!(text_editing(&app), None);
    (app, [a, b, c], other)
}

fn undo_steps(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().history.undo.len()
}

/// With the Selection tool, a selected frame of a threaded story stands for the whole story: the
/// panel's changes reach its locked and hidden frames, each in one undo step, and leave other type
/// alone.
#[test]
fn selection_tool_changes_reach_every_frame_of_the_story() {
    let (mut app, frames, other) = story_selected_in_its_first_frame();
    assert_eq!(selected_stories(&app), frames.map(NodeId).to_vec());

    let steps = undo_steps(&app);
    style(&mut app, json!({"size": 9}));
    assert_eq!(undo_steps(&app), steps + 1, "one undo step");
    for id in frames {
        assert!(story_sizes(&app, id).iter().all(|s| (*s - 9.0).abs() < 1e-9), "frame {id} takes the size");
    }
    assert!(story_sizes(&app, other).iter().all(|s| (*s - 12.0).abs() < 1e-9), "other type keeps its size");
    app.run("edit.undo", json!({})).unwrap();
    for id in frames {
        assert!(story_sizes(&app, id).iter().all(|s| (*s - 12.0).abs() < 1e-9), "undo gives frame {id} its size back");
    }

    format(&mut app, json!({"underline": true}));
    for id in frames {
        assert!(story_frame(&app, id).runs.iter().all(|r| r.style.underline), "frame {id} is underlined");
    }
    assert!(story_frame(&app, other).runs.iter().all(|r| !r.style.underline));

    // Tate-chu-yoko Settings and Warichu Settings set their values through `run_format`.
    run_format(&mut app, json!({"tcyUpDown": 2})).unwrap();
    for id in frames {
        assert!(story_frame(&app, id).runs.iter().all(|r| (r.style.tcy_up_down - 2.0).abs() < 1e-9), "frame {id} takes Up/Down");
    }
    assert!(story_frame(&app, other).runs.iter().all(|r| r.style.tcy_up_down.abs() < 1e-9));
}

/// The fields show a value only when every frame of the selected story has it: a hidden frame that
/// differs leaves them blank.
#[test]
fn selection_tool_fields_read_every_frame_of_the_story() {
    let (mut app, [a, b, c], _) = story_selected_in_its_first_frame();
    assert_eq!(shared(&app, |s| s.size), Some(12.0));
    assert_eq!(shared_anti_alias(&app), Some(TypeAntiAlias::Sharp));
    app.run("text.setStyle", json!({"ids": [c], "size": 20})).unwrap();
    app.run("text.setFormat", json!({"ids": [b], "antiAlias": TypeAntiAlias::Crisp.id()})).unwrap();
    app.run("select.set", json!({"ids": [a]})).unwrap();
    assert_eq!(shared(&app, |s| s.size), None, "the hidden frame's size differs");
    assert_eq!(shared_anti_alias(&app), None, "the locked frame's anti-aliasing differs");
}

/// Two frames of one story, or a frame in a selected group, count once; an unthreaded type
/// object is a story of its own.
#[test]
fn selected_stories_list_each_frame_once() {
    let (mut app, [a, b, c], other) = story_selected_in_its_first_frame();
    app.run("object.unlockAll", json!({})).unwrap();
    app.run("select.set", json!({"ids": [b, a]})).unwrap();
    assert_eq!(selected_stories(&app), [a, b, c].map(NodeId).to_vec(), "thread order, each once");
    app.run("select.set", json!({"ids": [other, a]})).unwrap();
    let group = app.run("object.group", json!({})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [group]})).unwrap();
    let mut listed = selected_stories(&app);
    listed.sort_by_key(|id| id.0);
    let mut expected = [a, b, c, other].map(NodeId).to_vec();
    expected.sort_by_key(|id| id.0);
    assert_eq!(listed, expected);
    app.run("select.set", json!({"ids": []})).unwrap();
    assert!(selected_stories(&app).is_empty());
}

/// Check that the kept list of the selected stories, and the type objects it resolves to, are the
/// ones a fresh build works out.
fn assert_stories_are_fresh(app: &VectorcraftApp, what: &str) {
    let st = app.session.active().unwrap();
    let fresh = build_selected_stories(st);
    assert_eq!(selected_stories(app), fresh, "{what}");
    let texts = selected_story_texts(app);
    let expected: Vec<_> = fresh
        .iter()
        .filter_map(|id| match &st.doc.node(*id)?.kind {
            NodeKind::Text(t) => Some(t.as_ref()),
            _ => None,
        })
        .collect();
    assert_eq!(texts.len(), expected.len(), "{what}: type objects");
    assert!(texts.iter().zip(&expected).all(|(t, e)| std::ptr::eq(*t, *e)), "{what}: the current document's type objects, in order");
}

/// The kept list of the selected stories follows every change it is worked out from: an edit, a
/// new selection (in another order, or as long with other objects, also without a new revision),
/// a selected group and a switch to another document at the same revision.
#[test]
fn kept_selected_stories_follow_the_document_and_the_selection() {
    let (mut app, [a, b, c], other) = story_selected_in_its_first_frame();
    assert_stories_are_fresh(&app, "a frame of a story");
    let builds = STORY_BUILDS.get();
    assert_eq!(selected_stories(&app), [a, b, c].map(NodeId).to_vec());
    assert_eq!(STORY_BUILDS.get(), builds, "a second read keeps the list");

    app.run("text.setStyle", json!({"size": 9})).unwrap();
    assert_stories_are_fresh(&app, "after an edit");
    app.run("select.set", json!({"ids": [other, a]})).unwrap();
    assert_stories_are_fresh(&app, "two stories");
    app.run("select.set", json!({"ids": [a, other]})).unwrap();
    assert_stories_are_fresh(&app, "the same objects in another order");
    app.run("select.set", json!({"ids": [other, b]})).unwrap();
    assert_stories_are_fresh(&app, "as many other objects");
    app.session.doc_mut().unwrap().selection.objects = vec![NodeId(other)];
    assert_stories_are_fresh(&app, "a new selection at the same revision");
    assert_eq!(selected_stories(&app), vec![NodeId(other)]);

    app.run("select.set", json!({"ids": [other, a]})).unwrap();
    let group = app.run("object.group", json!({})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [group]})).unwrap();
    assert_stories_are_fresh(&app, "a selected group");

    let revision = app.session.active().unwrap().revision;
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let only = app.run("text.create", json!({"x": 20, "y": 50, "text": "Only", "size": 12})).unwrap()["id"].as_u64().unwrap();
    let st = app.session.doc_mut().unwrap();
    st.selection.objects = vec![NodeId(group)];
    st.revision = revision;
    assert_stories_are_fresh(&app, "another document with the first one's selection and revision");
    assert_ne!(selected_stories(&app), [a, b, c, other].map(NodeId).to_vec());
    app.run("select.set", json!({"ids": [only]})).unwrap();
    assert_stories_are_fresh(&app, "the other document's own type");
    app.run("document.activate", json!({"index": 0})).unwrap();
    assert_stories_are_fresh(&app, "back in the first document");

    app.run("select.set", json!({"ids": []})).unwrap();
    assert!(selected_stories(&app).is_empty());
    assert!(selected_story_texts(&app).is_empty());
}

/// The values of the Character and Paragraph panels, and of their fields, with the list of the
/// selected stories kept and with it built for every read.
fn panels_kept_and_unkept(app: &mut VectorcraftApp) -> [(String, String); 2] {
    let read = |app: &mut VectorcraftApp, unkept: bool| {
        STORIES_UNKEPT.set(unkept);
        // PLAN: assumed pub(crate) fn values(&VectorcraftApp) -> ParagraphValues in panels/paragraph.rs (type-w12)
        let panels = format!("{:?}", (values(app), crate::panels::paragraph::values(app)));
        let fields = format!(
            "{:?} {:?}",
            [
                shared_effective(app, |s| s.size),
                shared_effective(app, |s| s.h_scale),
                shared_effective(app, |s| s.v_scale),
                shared_effective(app, |s| s.baseline_shift),
                shared_leading(app),
                shared(app, |s| s.tracking),
            ],
            (
                shared(app, |s| (s.font_family.clone(), s.font_style.clone())),
                shared(app, |s| (s.all_caps, s.underline, s.strikethrough, s.position)),
                selected_fonts(app),
                any_style(app, |s| s.underline),
                selected_stories(app),
                selected_story_texts(app).len(),
                text_style(app).map(|(c, p)| (c.size, p.left_indent)),
            )
        );
        STORIES_UNKEPT.set(false);
        (panels, fields)
    };
    [read(app, true), read(app, false)]
}

/// The panels show the same thing whether the list of the selected stories is kept or worked out
/// for each field: with nothing selected, one story, several stories with mixed values and a
/// Type tool selection inside one story.
#[test]
fn panels_show_the_same_with_the_kept_selected_stories() {
    let (mut app, [a, b, c], other) = story_selected_in_its_first_frame();
    let same = |app: &mut VectorcraftApp, what: &str| {
        let [unkept, kept] = panels_kept_and_unkept(app);
        assert_eq!(unkept.1, kept.1, "{what}: field values");
        assert_eq!(unkept.0, kept.0, "{what}: what the panels show");
        unkept.1
    };
    app.run("select.set", json!({"ids": []})).unwrap();
    same(&mut app, "nothing selected");
    app.run("select.set", json!({"ids": [a]})).unwrap();
    let one = same(&mut app, "one story");
    app.run("text.setStyle", json!({"ids": [c], "size": 20, "tracking": 50})).unwrap();
    app.run("text.setFormat", json!({"ids": [b], "underline": true, "hScale": 120})).unwrap();
    app.run("select.set", json!({"ids": [other, a]})).unwrap();
    let mixed = same(&mut app, "several stories with mixed values");
    assert_ne!(one, mixed);
    app.session.select_tool("type", Default::default()).unwrap();
    app.session.set_tool_option("textSelection", &json!({"editing": a, "anchor": 4, "caret": 15}));
    assert_eq!(text_editing(&app), Some((NodeId(a), 4, 15)));
    same(&mut app, "a Type tool selection inside one story");
}

/// One frame of the Character and Paragraph panels works out the selected stories once, though
/// many of their fields read them; a change to the document works them out again.
#[test]
fn panels_work_out_the_selected_stories_once_per_frame() {
    let (mut app, [a, ..], other) = story_selected_in_its_first_frame();
    app.run("select.set", json!({"ids": [other, a]})).unwrap();
    let ctx = crate::testing::ui_context();
    let frame = |app: &mut VectorcraftApp| {
        let builds = STORY_BUILDS.get();
        super::super::tests_appearance::frame_events(&ctx, app, vec![], |app, ui| {
            show(app, ui);
            crate::panels::paragraph::show(app, ui);
        });
        STORY_BUILDS.get() - builds
    };
    frame(&mut app);
    app.run("text.setStyle", json!({"size": 9})).unwrap();
    assert_eq!(frame(&mut app), 1, "one build after an edit");
    assert_eq!(frame(&mut app), 0, "nothing changed");
    STORIES_UNKEPT.set(true);
    let reads = frame(&mut app);
    STORIES_UNKEPT.set(false);
    assert!(reads > 1, "the fields read the list {reads} times");
}
