use super::*;
use vectorcraft_doc::NodeId;
use vectorcraft_engine::Session;

fn alignment_row(app: &mut VectorcraftApp, ui: &mut Ui, size: f32) -> egui::Rect {
    let para = text_style(app).map_or_else(ParaStyle::default, |(_, p)| p);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        alignment_buttons(app, ui, &para, size);
    })
    .response
    .rect
}

// Inspect the actual selected wells, including the shared Properties-size buttons.
fn alignment_frame(ctx: &egui::Context, app: &mut VectorcraftApp, size: f32) -> (Vec<Justify>, egui::Pos2) {
    let mut row = egui::Rect::NOTHING;
    let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| row = alignment_row(app, ui, size));
    frame.textures_delta.clear();
    fn wells(shape: &egui::Shape, color: egui::Color32, size: f32, out: &mut Vec<egui::Pos2>) {
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| wells(s, color, size, out)),
            egui::Shape::Rect(r) if r.fill == color && r.rect.size() == vec2(size, size) => out.push(r.rect.center()),
            _ => {}
        }
    }
    let centers: [egui::Pos2; 7] = std::array::from_fn(|i| row.min + vec2(size / 2.0 + i as f32 * (size + 3.0), size / 2.0));
    let mut active = Vec::new();
    for shape in &frame.shapes {
        wells(&shape.shape, Tokens::get(ctx).tool_active, size, &mut active);
    }
    let selected = active
        .iter()
        .map(|pos| {
            let index = centers.iter().position(|center| center.distance(*pos) < 0.1).expect("alignment well");
            ALIGNMENTS[index].0
        })
        .collect();
    (selected, centers[1])
}

fn editing_text(text: &str) -> (VectorcraftApp, Value) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    app.session.select_tool("type", Default::default()).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": text})).unwrap()["id"].clone();
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    assert!(super::super::character::text_editing(&app).is_some());
    (app, id)
}

fn select_range(app: &mut VectorcraftApp, start: usize, end: usize) {
    app.session.set_tool_option("select", &json!({"start": start, "end": end}));
}

#[test]
fn paragraph_mixed_direction_buttons_show_neither() {
    let (mut app, id) = editing_text("A\nB");
    app.session.prefs.show_indic_options = true;
    app.run("text.setFormat", json!({"id": id, "direction": "leftToRight"})).unwrap();
    app.run("text.setFormat", json!({"id": id, "start": 2, "end": 3, "direction": "rightToLeft"})).unwrap();
    app.session.set_tool_option("selectAll", &json!(true));
    let para = text_style(&app).unwrap().1;
    assert_eq!(selected_paragraph_direction(&app, &para), None);
    app.session.set_tool_option("selectAll", &json!(true));
    app.run("text.setFormat", json!({"id": id, "direction": "leftToRight"})).unwrap();
    assert_eq!(selected_paragraph_direction(&app, &para), Some(false));
    app.session.set_tool_option("selectAll", &json!(true));
    app.run("text.setFormat", json!({"id": id, "direction": "rightToLeft"})).unwrap();
    assert_eq!(selected_paragraph_direction(&app, &para), Some(true));
}

#[test]
fn paragraph_mixed_composer_checks_neither_menu_item() {
    let (mut app, id) = editing_text("A\nB\n");
    app.run("text.setFormat", json!({"id": id, "composer": "singleLine"})).unwrap();
    app.run("text.setFormat", json!({"id": id, "start": 2, "end": 3, "composer": "everyLine"})).unwrap();
    app.session.set_tool_option("selectAll", &json!(true));
    assert_eq!(shared_composer(&app), None);
    app.run("text.setFormat", json!({"id": id, "composer": "everyLine"})).unwrap();
    assert_eq!(shared_composer(&app), Some(Composer::EveryLine));
}

#[test]
fn paragraph_mixed_hyphenate_checks_off() {
    let (mut app, id) = editing_text("A\nB\n");
    app.run("text.setFormat", json!({"id": id, "hyphenate": true})).unwrap();
    app.run("text.setFormat", json!({"id": id, "start": 2, "end": 3, "hyphenate": false})).unwrap();
    app.session.set_tool_option("selectAll", &json!(true));
    assert_eq!(shared_hyphenate(&app), None);
    app.run("text.setFormat", json!({"id": id, "hyphenate": true})).unwrap();
    assert_eq!(shared_hyphenate(&app), Some(true));
}

#[test]
fn paragraph_mixed_kinsoku_and_mojikumi_show_blank_dropdowns() {
    let (mut app, id) = editing_text("A\nB\n");
    app.session.prefs.show_east_asian_options = true;
    app.run("text.setFormat", json!({"id": id, "kinsoku": "hard", "mojikumi": "none"})).unwrap();
    app.run("text.setFormat", json!({"id": id, "start": 2, "end": 3, "kinsoku": "soft", "mojikumi": "lineEndHalf"})).unwrap();
    app.session.set_tool_option("selectAll", &json!(true));
    assert_eq!(shared_para(&app, |p| p.kinsoku), None);
    assert_eq!(shared_para(&app, |p| p.mojikumi), None);
    app.run("text.setFormat", json!({"id": id, "kinsoku": "hard", "mojikumi": "lineEndHalf"})).unwrap();
    assert_eq!(shared_para(&app, |p| p.kinsoku), Some(Kinsoku::Hard));
    assert_eq!(shared_para(&app, |p| p.mojikumi), Some(Mojikumi::LineEndHalf));
}

#[test]
fn paragraph_mixed_indent_fields_show_blank() {
    let (mut app, id) = editing_text("A\nB\n");
    app.run("text.setFormat", json!({"id": id, "leftIndent": 10})).unwrap();
    app.run("text.setFormat", json!({"id": id, "start": 2, "end": 3, "leftIndent": 20})).unwrap();
    app.session.set_tool_option("selectAll", &json!(true));
    assert_eq!(shared_para_field(&app, |p| p.left_indent), None);
    assert_eq!(shared_para_field(&app, |p| p.space_before), Some(0.0));
    app.run("text.setFormat", json!({"id": id, "leftIndent": 12})).unwrap();
    assert_eq!(shared_para_field(&app, |p| p.left_indent), Some(12.0));
    app.session.set_tool_option("stopEditing", &json!(true));
    let other = app.run("text.create", json!({"x": 40, "y": 80, "text": "C"})).unwrap()["id"].clone();
    app.run("text.setFormat", json!({"id": other, "leftIndent": 5})).unwrap();
    app.run("select.set", json!({"ids": [id, other]})).unwrap();
    assert_eq!(shared_para_field(&app, |p| p.left_indent), None);
}

#[test]
fn paragraph_mixed_alignment_click_center_and_undo() {
    let (mut app, id) = editing_text("é\nאב\n終");
    app.run("text.setStyle", json!({"id": id, "justify": "left"})).unwrap();
    app.run("text.setStyle", json!({"id": id, "start": 3, "end": 7, "justify": "right"})).unwrap();
    app.run("text.setStyle", json!({"id": id, "start": 8, "end": 11, "justify": "justifyAll"})).unwrap();
    app.run("text.setFormat", json!({"id": id, "start": 3, "end": 7, "leftIndent": 17, "spaceAfter": 9})).unwrap();
    select_range(&mut app, 0, 7);
    let before = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.undo.len();
    let ctx = egui::Context::default();
    let (selected, center) = alignment_frame(&ctx, &mut app, 28.0);
    assert!(selected.is_empty(), "mixed paragraphs must not light one alignment: {selected:?}");
    super::super::tests_appearance::click(&ctx, &mut app, center, |app, ui| {
        alignment_row(app, ui, 28.0);
    });
    assert_eq!(alignment_frame(&ctx, &mut app, 28.0).0, [Justify::Center]);
    let node = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap();
    let NodeKind::Text(text) = &node.kind else { panic!("text") };
    assert_eq!((text.para_at(0).justify, text.para_at(1).justify, text.para_at(2).justify), (Justify::Center, Justify::Center, Justify::JustifyAll));
    let NodeKind::Text(old) = &before.node(node.id).unwrap().kind else { panic!("text") };
    for i in 0..3 {
        let mut expected = old.para_at(i).clone();
        if i < 2 {
            expected.justify = Justify::Center;
        }
        assert_eq!(text.para_at(i), &expected, "other paragraph fields stay unchanged");
    }
    assert_eq!(text.runs, old.runs);
    assert_eq!(app.session.active().unwrap().history.undo.len(), history + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc, before);
    assert!(alignment_frame(&ctx, &mut app, 28.0).0.is_empty());
}

#[test]
fn paragraph_mixed_alignment_caret_unicode_boundaries_and_uniform_controls() {
    let (mut app, id) = editing_text("é\nאב\n");
    app.run("text.setStyle", json!({"id": id, "justify": "left"})).unwrap();
    app.run("text.setStyle", json!({"id": id, "start": 3, "end": 7, "justify": "right"})).unwrap();
    app.run("text.setStyle", json!({"id": id, "start": 8, "end": 8, "justify": "justifyAll"})).unwrap();
    let ctx = egui::Context::default();
    for (start, end, expected) in [
        (0, 2, vec![Justify::Left]),
        (2, 2, vec![Justify::Left]), // newline belongs to the preceding paragraph
        (3, 3, vec![Justify::Right]),
        (4, 6, vec![Justify::Right]),          // byte offsets inside multi-byte characters
        (0, 3, vec![]),                        // ending at the next paragraph's start touches both
        (7, 0, vec![]),                        // reversed selection
        (8, 8, vec![Justify::JustifyAll]),     // empty trailing paragraph
        (100, 100, vec![Justify::JustifyAll]), // existing selection clamping
    ] {
        select_range(&mut app, start, end);
        assert_eq!(alignment_frame(&ctx, &mut app, 24.0).0, expected, "range {start}..{end}");
    }
    for (expected, _, _, key) in ALIGNMENTS {
        app.run("text.setStyle", json!({"id": id, "justify": key})).unwrap();
        app.session.set_tool_option("selectAll", &json!(true));
        assert_eq!(alignment_frame(&ctx, &mut app, 28.0).0, [expected]);
    }
}

#[test]
fn paragraph_mixed_alignment_auto_direction_and_split_runs() {
    let (mut app, id) = editing_text("é\nאב");
    // Character formatting splits the RTL paragraph across runs.
    app.run("text.setRangeStyle", json!({"id": id, "start": 3, "end": 5, "size": 24})).unwrap();
    app.session.set_tool_option("selectAll", &json!(true));
    let ctx = egui::Context::default();
    assert!(alignment_frame(&ctx, &mut app, 28.0).0.is_empty(), "Auto resolves separately per paragraph");
    select_range(&mut app, 3, 7);
    assert_eq!(alignment_frame(&ctx, &mut app, 28.0).0, [Justify::Right]);
    app.run("text.setFormat", json!({"id": id, "start": 3, "end": 7, "direction": "leftToRight"})).unwrap();
    app.session.set_tool_option("selectAll", &json!(true));
    assert_eq!(alignment_frame(&ctx, &mut app, 28.0).0, [Justify::Left]);
    app.run("text.setStyle", json!({"id": id, "start": 0, "end": 2, "justify": "left"})).unwrap();
    assert_eq!(alignment_frame(&ctx, &mut app, 28.0).0, [Justify::Left], "explicit Left and resolved Auto agree");
    app.run("text.setFormat", json!({"id": id, "start": 0, "end": 2, "direction": "rightToLeft"})).unwrap();
    assert_eq!(alignment_frame(&ctx, &mut app, 28.0).0, [Justify::Left], "explicit alignment ignores direction");
}

#[test]
fn paragraph_mixed_alignment_explicit_rtl_and_empty_controls() {
    let (mut app, id) = editing_text("Latin\nאב\n\n");
    app.run("text.setStyle", json!({"id": id, "justify": "auto"})).unwrap();
    app.run("text.setFormat", json!({"id": id, "direction": "rightToLeft"})).unwrap();
    // Auto with an explicit direction still needs the real, possibly split, paragraph text.
    app.run("text.setRangeStyle", json!({"id": id, "start": 0, "end": 2, "size": 24})).unwrap();
    let ctx = egui::Context::default();
    for (start, end, expected) in [
        (0, 5, Justify::Right),  // nonempty Latin with explicit RTL
        (6, 10, Justify::Right), // nonempty Hebrew with explicit RTL
        (11, 11, Justify::Left), // existing resolver's empty paragraph behavior
        (12, 12, Justify::Left), // empty trailing paragraph
    ] {
        select_range(&mut app, start, end);
        assert_eq!(alignment_frame(&ctx, &mut app, 28.0).0, [expected], "explicit RTL range {start}..{end}");
    }
    app.run("text.setStyle", json!({"id": id, "start": 0, "end": 5, "justify": "left"})).unwrap();
    select_range(&mut app, 0, 10);
    assert!(alignment_frame(&ctx, &mut app, 28.0).0.is_empty(), "explicit Left and Auto/RTL differ");
    let (mut empty, id) = editing_text("");
    empty.run("text.setStyle", json!({"id": id, "justify": "auto"})).unwrap();
    empty.run("text.setFormat", json!({"id": id, "direction": "rightToLeft"})).unwrap();
    assert_eq!(alignment_frame(&egui::Context::default(), &mut empty, 28.0).0, [Justify::Left]);
}

#[test]
fn paragraph_mixed_alignment_object_group_and_no_text_controls() {
    let (mut app, id) = editing_text("A\nB");
    app.run("text.setStyle", json!({"id": id, "justify": "left"})).unwrap();
    app.run("text.setStyle", json!({"id": id, "start": 2, "end": 3, "justify": "right"})).unwrap();
    app.session.set_tool_option("stopEditing", &json!(true));
    let ctx = egui::Context::default();
    assert!(alignment_frame(&ctx, &mut app, 28.0).0.is_empty(), "whole object covers every paragraph");
    app.run("object.group", json!({})).unwrap();
    assert!(alignment_frame(&ctx, &mut app, 28.0).0.is_empty(), "group descendant has mixed paragraphs");
    app.run("text.setStyle", json!({"justify": "center"})).unwrap();
    assert_eq!(alignment_frame(&ctx, &mut app, 28.0).0, [Justify::Center]);
    let group = app.session.active().unwrap().selection.objects.clone();
    let other = app.run("text.create", json!({"x": 20, "y": 100, "text": "C"})).unwrap()["id"].clone();
    let mut ids: Vec<Value> = group.iter().map(|id| json!(id.0)).collect();
    ids.push(other);
    app.run("select.set", json!({"ids": ids})).unwrap();
    assert!(alignment_frame(&ctx, &mut app, 28.0).0.is_empty(), "different selected objects");
    app.run("select.set", json!({"ids": []})).unwrap();
    assert_eq!(alignment_frame(&ctx, &mut app, 28.0).0, [Justify::Left], "no-text default stays unchanged");
    let (mut empty, _) = editing_text("");
    assert_eq!(alignment_frame(&egui::Context::default(), &mut empty, 28.0).0, [Justify::Left]);
}

/// New Hebrew type aligns to the start of its direction (Align Right shows), an explicit
/// alignment stays as chosen, and Paragraph Direction (with the Indic options) sets the
/// direction: Auto then follows it.
#[test]
fn alignment_buttons_show_the_resolved_alignment_and_direction_sets_it() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 200})).unwrap();
    app.session.execute("text.create", &json!({"x": 200, "y": 50, "text": "שלום"})).unwrap();
    let shown = |app: &VectorcraftApp| {
        let para = text_style(app).unwrap().1;
        shown_alignment(para.justify, resolved_rtl(app, &para))
    };
    assert_eq!(shown(&app), Justify::Right);
    app.session.execute("object.group", &json!({})).unwrap();
    assert_eq!(shown(&app), Justify::Right, "group controls resolve the same text descendant's direction");
    app.session.execute("text.setStyle", &json!({"justify": "left"})).unwrap();
    assert_eq!(shown(&app), Justify::Left);
    app.session.execute("text.setStyle", &json!({"justify": "auto"})).unwrap();
    app.session.execute("text.setFormat", &json!({"direction": "leftToRight"})).unwrap();
    assert_eq!(shown(&app), Justify::Left);
    // The direction buttons show with the Indic options only.
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let tips = |app: &mut VectorcraftApp| {
        let ctx = egui::Context::default();
        let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
        frame.textures_delta.clear();
        frame.shapes.len()
    };
    let without = tips(&mut app);
    app.session.prefs.show_indic_options = true;
    assert!(tips(&mut app) > without, "two more buttons");
}

/// Insert Kashidas is in the panel with the Middle Eastern (Indic) options only, shows the
/// paragraph's setting, and Reset Panel sets it back to None.
#[test]
fn insert_kashidas_shows_with_the_indic_options_and_reset_panel_turns_it_off() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "كتب محمد"})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let painted = |app: &mut VectorcraftApp| crate::tests_labels::painted_text(app, show);
    assert!(!painted(&mut app).contains("Insert Kashidas"));
    app.session.prefs.show_indic_options = true;
    let text = painted(&mut app);
    assert!(text.contains("Insert Kashidas") && text.contains("None"), "{text}");
    app.run("text.setFormat", json!({"kashida": "medium"})).unwrap();
    assert!(painted(&mut app).contains("Medium"));
    let kashida = |app: &VectorcraftApp| {
        let node = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap();
        let NodeKind::Text(text) = &node.kind else { panic!("text") };
        text.para_at(0).kashida
    };
    assert_eq!(kashida(&app), vectorcraft_doc::text::Kashida::Medium);
    reset_panel(&mut app);
    assert_eq!(kashida(&app), vectorcraft_doc::text::Kashida::None);
}

/// The Bullets button makes the paragraphs the Type tool's selection touches a bulleted list
/// with the dot and shows pressed; the arrow's presets pick another bullet, None turns the list
/// off, Type › Bullets and Numbering › Apply Bullets applies a list, and Reset Panel turns it
/// off.
#[test]
fn the_bullets_button_and_its_presets_set_the_list_and_reset_panel_clears_it() {
    use vectorcraft_doc::text::{Bullet, ParaList};
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let (mut app, id) = editing_text("One\nTwo");
    select_range(&mut app, 0, 0);
    let lists = |app: &VectorcraftApp| {
        let node = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap();
        let NodeKind::Text(text) = &node.kind else { panic!("text") };
        (text.para_at(0).list, text.para_at(1).list)
    };
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let row = std::cell::Cell::new(egui::Rect::NOTHING);
    let mut draw = |app: &mut VectorcraftApp, ui: &mut Ui| {
        row.set(ui.horizontal(|ui| bullets_button(app, ui, 28.0)).response.rect);
    };
    super::super::tests_appearance::frame_events(&ctx, &mut app, vec![], &mut draw);
    let min = row.get().min;
    let (button, arrow) = (min + vec2(14.0, 14.0), min + vec2(28.0 + 7.0, 14.0));
    super::super::tests_appearance::click(&ctx, &mut app, button, &mut draw);
    assert_eq!(lists(&app), (Some(ParaList::bulleted(Bullet::Dot)), None), "the caret's paragraph");
    assert_eq!(shared_para(&app, |p| p.list.map(|l| l.bullet)), Some(Some(Bullet::Dot)), "the button shows pressed");
    let mut pick = |app: &mut VectorcraftApp, name: &str| {
        super::super::tests_appearance::click(&ctx, app, arrow, &mut draw);
        let texts = super::super::tests_appearance::frame_events(&ctx, app, vec![], &mut draw);
        let at = super::super::tests_appearance::text_rect(&texts, name).center();
        super::super::tests_appearance::click(&ctx, app, at, &mut draw);
    };
    pick(&mut app, "Filled Square");
    assert_eq!(lists(&app), (Some(ParaList::bulleted(Bullet::Square)), None));
    pick(&mut app, "None");
    assert_eq!(lists(&app), (None, None));
    app.session.set_tool_option("selectAll", &json!(true));
    app.run("type.applyBullets", json!({})).unwrap();
    assert_eq!(lists(&app), (Some(ParaList::bulleted(Bullet::Dot)), Some(ParaList::bulleted(Bullet::Dot))));
    app.run("type.applyBullets", json!({"bullet": "dash"})).unwrap();
    assert_eq!(lists(&app).0, Some(ParaList::bulleted(Bullet::Dash)));
    app.session.set_tool_option("selectAll", &json!(true));
    reset_panel(&mut app);
    assert_eq!(lists(&app), (None, None));
}

/// The Numbered List button makes the paragraphs the Type tool's selection touches a numbered
/// list 1, 2, 3 and shows pressed; the arrow's presets, each shown by its first three numbers,
/// pick another style, None turns the list off, Type › Bullets and Numbering › Apply Numbers
/// applies a numbered list, and Reset Panel turns it off.
#[test]
fn the_numbered_list_button_and_its_presets_set_the_list_and_reset_panel_clears_it() {
    use vectorcraft_doc::text::{NumberStyle, ParaList};
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let (mut app, id) = editing_text("One\nTwo");
    select_range(&mut app, 0, 0);
    let lists = |app: &VectorcraftApp| {
        let node = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap();
        let NodeKind::Text(text) = &node.kind else { panic!("text") };
        (text.para_at(0).list, text.para_at(1).list)
    };
    let numbered = |style| Some(ParaList::numbered(style));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let row = std::cell::Cell::new(egui::Rect::NOTHING);
    let mut draw = |app: &mut VectorcraftApp, ui: &mut Ui| {
        row.set(ui.horizontal(|ui| numbers_button(app, ui, 28.0)).response.rect);
    };
    super::super::tests_appearance::frame_events(&ctx, &mut app, vec![], &mut draw);
    let min = row.get().min;
    let (button, arrow) = (min + vec2(14.0, 14.0), min + vec2(28.0 + 7.0, 14.0));
    super::super::tests_appearance::click(&ctx, &mut app, button, &mut draw);
    assert_eq!(lists(&app), (numbered(NumberStyle::Arabic), None), "the caret's paragraph");
    assert_eq!(shared_para(&app, |p| p.list.and_then(|l| l.number)), Some(Some(NumberStyle::Arabic)), "the button shows pressed");
    let mut pick = |app: &mut VectorcraftApp, name: &str| {
        super::super::tests_appearance::click(&ctx, app, arrow, &mut draw);
        let texts = super::super::tests_appearance::frame_events(&ctx, app, vec![], &mut draw);
        let at = super::super::tests_appearance::text_rect(&texts, name).center();
        super::super::tests_appearance::click(&ctx, app, at, &mut draw);
    };
    for (name, style) in [
        ("I, II, III", NumberStyle::UpperRoman),
        ("a, b, c", NumberStyle::LowerLetters),
        ("001, 002, 003", NumberStyle::ThreeDigits),
        ("1, 2, 3", NumberStyle::Arabic),
    ] {
        pick(&mut app, name);
        assert_eq!(lists(&app), (numbered(style), None), "{name}");
    }
    pick(&mut app, "None");
    assert_eq!(lists(&app), (None, None));
    app.session.set_tool_option("selectAll", &json!(true));
    app.run("type.applyNumbers", json!({})).unwrap();
    assert_eq!(lists(&app), (numbered(NumberStyle::Arabic), numbered(NumberStyle::Arabic)));
    // The Numbered List button shows pressed and the Bullets button doesn't, and the other way
    // round for a bulleted list.
    // Each button and the arrow beside it have the same name; only a button shows pressed.
    let pressed = |app: &mut VectorcraftApp| {
        let nodes = crate::widgets::accessibility_tests::tree(|ui| {
            ui.horizontal(|ui| {
                bullets_button(app, ui, 28.0);
                numbers_button(app, ui, 28.0);
            });
        });
        let on = |label| nodes.iter().any(|n| n.label() == Some(label) && n.toggled() == Some(egui::accesskit::Toggled::True));
        (on("Bullets"), on("Numbered List"))
    };
    assert_eq!(pressed(&mut app), (false, true));
    app.session.set_tool_option("selectAll", &json!(true));
    app.run("type.applyBullets", json!({})).unwrap();
    assert_eq!(pressed(&mut app), (true, false));
    app.session.set_tool_option("selectAll", &json!(true));
    app.run("type.applyNumbers", json!({})).unwrap();
    app.run("type.applyNumbers", json!({"number": "upperLetters"})).unwrap();
    assert_eq!(lists(&app).0, numbered(NumberStyle::UpperLetters));
    app.session.set_tool_option("selectAll", &json!(true));
    reset_panel(&mut app);
    assert_eq!(lists(&app), (None, None));
}

/// Type › Bullets and Numbering › Convert to Text (`type.convertListToText`) acts on the
/// paragraphs the Type tool's selection touches: their numbers become characters of the text
/// and their list goes, while the other paragraphs stay in the list, and the Type tool's caret
/// stays next to the characters it was next to. The menu item is there and is enabled while
/// type is selected or edited.
#[test]
fn convert_to_text_makes_the_selected_paragraphs_numbers_characters() {
    use vectorcraft_doc::text::{NumberStyle, ParaList};
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let (mut app, id) = editing_text("One\nTwo");
    let text = |app: &VectorcraftApp| {
        let node = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap();
        let NodeKind::Text(text) = &node.kind else { panic!("text") };
        (text.plain_text(), text.para_at(0).list, text.para_at(1).list)
    };
    app.session.set_tool_option("selectAll", &json!(true));
    app.run("type.applyNumbers", json!({})).unwrap();
    let numbered = Some(ParaList::numbered(NumberStyle::Arabic));
    assert_eq!(text(&app), ("One\nTwo".to_string(), numbered, numbered));
    assert!(crate::menus::enabled(&app, "type.convertListToText"));
    select_range(&mut app, 5, 5);
    app.run("type.convertListToText", json!({})).unwrap();
    assert_eq!(text(&app), ("One\n2.\tTwo".to_string(), numbered, None), "the caret's paragraph");
    let options = app.session.tool_options();
    assert_eq!((options["anchor"].as_u64(), options["caret"].as_u64()), (Some(8), Some(8)), "the caret stays after the T of Two");
    let typemenu = crate::menus::menu_tree().into_iter().find(|(title, _)| *title == "Type").map(|(_, items)| items).unwrap_or_default();
    let found = typemenu.iter().any(|item| {
        matches!(item, crate::menus::Item::Sub("Bullets and Numbering", items)
            if items.iter().any(|i| matches!(i, crate::menus::Item::Cmd("Convert to Text", "type.convertListToText", _))))
    });
    assert!(found, "Type › Bullets and Numbering › Convert to Text");
}

/// Convert to Text in threaded type: when the converted paragraph breaks its lines at other
/// places and its text moves to the frame before, the Type tool's caret goes with the
/// character it was next to, and that frame becomes the selection when it was not selected.
#[test]
fn convert_to_text_keeps_the_caret_by_its_character_in_threaded_type() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 600, "height": 300})).unwrap();
    let long = "The quick brown fox jumps over the lazy dog. ".repeat(4);
    let text = format!("One\nTwo\n{long}\nFour");
    let a = app.run("text.create", json!({"x": 10, "y": 10, "text": text, "area": {"width": 150, "height": 60}})).unwrap()["id"].as_u64().unwrap();
    let b = app.run("shape.rectangle", json!({"x": 200, "y": 10, "width": 150, "height": 200})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    app.run("text.thread.create", json!({})).unwrap();
    app.run("text.setFormat", json!({"ids": [a, b], "list": "number"})).unwrap();
    let plain = |app: &VectorcraftApp, id: u64| match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.plain_text(),
        _ => panic!("text"),
    };
    assert!(plain(&app, b).starts_with("dog. "), "the second frame starts with the dog: {:?}", plain(&app, b));
    app.run("select.set", json!({"ids": [b]})).unwrap();
    app.session.select_tool("type", Default::default()).unwrap();
    // The caret after the d of dog, at the start of the second frame.
    app.session.set_tool_option("textSelection", &json!({"editing": b, "anchor": 1, "caret": 1}));
    assert_eq!(super::super::character::text_editing(&app).map(|(id, _, _)| id), Some(NodeId(b)));
    app.run("type.convertListToText", json!({})).unwrap();
    let first = plain(&app, a);
    assert!(first.ends_with("lazy dog. "), "the dog moved to the first frame: {first:?}");
    let options = app.session.tool_options();
    let after_d = first.len() - "og. ".len();
    assert_eq!(
        (options["editing"].as_u64(), options["anchor"].as_u64(), options["caret"].as_u64()),
        (Some(a), Some(after_d as u64), Some(after_d as u64)),
        "the caret is after the d of dog in the first frame"
    );
    assert_eq!(app.session.active().unwrap().selection.objects, vec![NodeId(a)], "the first frame is selected");
    // The next command keeps the Type tool editing the first frame.
    app.run("text.setFormat", json!({"ids": [a], "composer": "singleLine"})).unwrap();
    assert_eq!(super::super::character::text_editing(&app).map(|(id, _, _)| id), Some(NodeId(a)));
}

/// Reset Panel turns Hyphenate and Roman Hanging Punctuation off and sets the Hyphenation and
/// Justification options back to their defaults in every paragraph it acts on, so the paragraphs
/// save neither.
#[test]
fn reset_panel_resets_the_hyphenation_options() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "First\nSecond"})).unwrap()["id"].clone();
    app.run(
        "text.setFormat",
        json!({
            "hyphenate": true, "wordsLongerThan": 8, "afterFirst": 2, "beforeLast": 4, "hyphenLimit": 2, "hyphenationZone": 18, "hyphenateCapitalized": false,
            "wordSpacing": [60, 90, 200], "letterSpacing": [-5, 0, 5], "glyphScaling": [95, 100, 105], "autoLeading": 150, "singleWordJustification": "alignLeft", "romanHangingPunctuation": true
        }),
    )
    .unwrap();
    let paras = |app: &VectorcraftApp| {
        let node = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap();
        let NodeKind::Text(text) = &node.kind else { panic!("text") };
        [text.para_at(0).clone(), text.para_at(1).clone()]
    };
    for para in paras(&app) {
        assert!(para.hyphenate);
        assert_ne!(para.hyphenation, HyphenationOptions::default());
        assert!(para.roman_hanging);
        assert_ne!(para.justification, vectorcraft_doc::JustificationOptions::default());
    }
    reset_panel(&mut app);
    for para in paras(&app) {
        assert!(!para.hyphenate);
        assert_eq!(para.hyphenation, HyphenationOptions::default());
        assert_eq!(serde_json::to_value(&para).unwrap().get("hyphenation"), None, "saves no hyphenation options");
        assert!(!para.roman_hanging, "Roman Hanging Punctuation off");
        assert_eq!(serde_json::to_value(&para).unwrap().get("justification"), None, "saves no justification options");
    }
}
