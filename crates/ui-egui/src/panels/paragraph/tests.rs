use super::*;
use vectorcraft_doc::Mojikumi;
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

/// The Mojikumi Set menu lists None, the four other predefined sets and Line-end Punctuation Half
/// Width, each under its own label in every interface language. Each entry sets the selected
/// paragraphs' set in one undo step and is shown as the current set; paragraphs with different
/// sets show a blank menu, and Reset Panel gives them Line-end Punctuation Half Width.
#[test]
fn mojikumi_menu_offers_and_sets_every_predefined_set() {
    let keys: Vec<&str> = mojikumi_sets().iter().map(|s| s.2).collect();
    assert_eq!(keys, ["none", "yakumonoHankaku", "gyomatsuYakumonoHankaku", "gyomatsuYakumonoZenkaku", "yakumonoZenkaku", "lineEndHalf"]);
    for lang in crate::i18n::Lang::all() {
        crate::i18n::set_current(lang);
        let labels: std::collections::BTreeSet<&str> = mojikumi_sets().iter().map(|s| s.0).collect();
        crate::i18n::set_current(crate::i18n::Lang::EN);
        assert_eq!(labels.len(), 6, "{}: {labels:?}", lang.code());
    }
    let (mut app, id) = editing_text("「あ」。\nい、");
    app.session.prefs.show_east_asian_options = true;
    app.session.set_tool_option("selectAll", &json!(true));
    let steps = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
    for (_, set, key) in mojikumi_sets() {
        app.session.set_tool_option("selectAll", &json!(true));
        let before = (shared_para(&app, |p| p.mojikumi), steps(&app));
        format(&mut app, json!({"mojikumi": key}));
        assert_eq!(shared_para(&app, |p| p.mojikumi), Some(set), "{key}");
        if before.0 == Some(set) {
            continue;
        }
        assert_eq!(steps(&app), before.1 + 1, "{key}: one undo step");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(shared_para(&app, |p| p.mojikumi), before.0, "{key}: undone");
        app.run("edit.redo", json!({})).unwrap();
    }
    app.run("text.setFormat", json!({"id": id, "start": 0, "end": 0, "mojikumi": "yakumonoHankaku"})).unwrap();
    app.run("text.setFormat", json!({"id": id, "start": 13, "end": 13, "mojikumi": "gyomatsuYakumonoZenkaku"})).unwrap();
    app.session.set_tool_option("selectAll", &json!(true));
    assert_eq!(shared_para(&app, |p| p.mojikumi), None, "mixed sets");
    reset_panel(&mut app);
    assert_eq!(shared_para(&app, |p| p.mojikumi), Some(Mojikumi::LineEndHalf), "Reset Panel");
}

/// Kinsoku Shori Type: the menu's item sets the type of the paragraphs the Type tool's selection
/// touches in one undo step, and none is checked while the selected paragraphs differ.
#[test]
fn kinsoku_shori_type_sets_the_selected_paragraphs_in_one_undo_step() {
    let (mut app, id) = editing_text("あ\nい\n");
    app.session.prefs.show_east_asian_options = true;
    let types = |app: &VectorcraftApp| {
        let node = app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap();
        let NodeKind::Text(text) = &node.kind else { panic!("text") };
        (text.para_at(0).kinsoku_type, text.para_at(1).kinsoku_type)
    };
    select_range(&mut app, 0, 3);
    let history = app.session.active().unwrap().history.undo.len();
    set_kinsoku_type(&mut app, "pushInFirst");
    assert_eq!(types(&app), (KinsokuShoriType::PushInFirst, KinsokuShoriType::PushOutOnly));
    assert_eq!(app.session.active().unwrap().history.undo.len(), history + 1);
    app.session.set_tool_option("selectAll", &json!(true));
    assert_eq!(shared_para(&app, |p| p.kinsoku_type), None, "mixed: no item checked");
    set_kinsoku_type(&mut app, "pushOutFirst");
    assert_eq!(types(&app), (KinsokuShoriType::PushOutFirst, KinsokuShoriType::PushOutFirst));
    assert_eq!(shared_para(&app, |p| p.kinsoku_type), Some(KinsokuShoriType::PushOutFirst));
    app.run("edit.undo", json!({})).unwrap();
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(types(&app), (KinsokuShoriType::PushOutOnly, KinsokuShoriType::PushOutOnly));
}

/// Korean Line Break Anywhere: new type has it off; the menu item turns it on for the paragraphs
/// the Type tool's selection touches in one undo step, is unchecked while the selected paragraphs
/// differ, turns it on for all of them then and off again once they all have it. Reset Panel turns
/// it off.
#[test]
fn korean_line_break_anywhere_toggles_the_selected_paragraphs_in_one_undo_step() {
    let (mut app, id) = editing_text("가나\n다라\n");
    app.session.prefs.show_east_asian_options = true;
    let flags = |app: &VectorcraftApp| {
        let node = app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap();
        let NodeKind::Text(text) = &node.kind else { panic!("text") };
        (text.para_at(0).korean_line_break_anywhere, text.para_at(1).korean_line_break_anywhere)
    };
    assert_eq!(flags(&app), (false, false), "new type has it off");
    select_range(&mut app, 0, 3);
    let history = app.session.active().unwrap().history.undo.len();
    toggle_korean_line_break_anywhere(&mut app);
    assert_eq!(flags(&app), (true, false));
    assert_eq!(app.session.active().unwrap().history.undo.len(), history + 1);
    app.session.set_tool_option("selectAll", &json!(true));
    assert_eq!(shared_para(&app, |p| p.korean_line_break_anywhere), None, "mixed: unchecked");
    toggle_korean_line_break_anywhere(&mut app);
    assert_eq!(flags(&app), (true, true));
    app.session.set_tool_option("selectAll", &json!(true));
    toggle_korean_line_break_anywhere(&mut app);
    assert_eq!(flags(&app), (false, false));
    app.session.set_tool_option("selectAll", &json!(true));
    toggle_korean_line_break_anywhere(&mut app);
    app.session.set_tool_option("selectAll", &json!(true));
    reset_panel(&mut app);
    assert_eq!(flags(&app), (false, false), "Reset Panel turns it off");
}

/// Bunri-Kinshi: new type has it off; the menu item turns it on for the paragraphs the Type tool's
/// selection touches in one undo step, is unchecked while the selected paragraphs differ, and is
/// unavailable while any of them has kinsoku None, which keeps the setting. Reset Panel turns it off.
#[test]
fn bunri_kinshi_toggle_follows_kinsoku_and_the_selected_paragraphs() {
    let (mut app, id) = editing_text("あ——い\nう……え\n");
    app.session.prefs.show_east_asian_options = true;
    let flags = |app: &VectorcraftApp| {
        let node = app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap();
        let NodeKind::Text(text) = &node.kind else { panic!("text") };
        (text.para_at(0).bunri_kinshi, text.para_at(1).bunri_kinshi)
    };
    assert_eq!(flags(&app), (false, false), "new type has it off");
    select_range(&mut app, 0, 3);
    assert_eq!(bunri_kinshi_item(&app), (true, false));
    let history = app.session.active().unwrap().history.undo.len();
    toggle_bunri_kinshi(&mut app);
    assert_eq!(flags(&app), (true, false));
    assert_eq!(app.session.active().unwrap().history.undo.len(), history + 1);
    assert_eq!(bunri_kinshi_item(&app), (true, true));
    app.session.set_tool_option("selectAll", &json!(true));
    assert_eq!(bunri_kinshi_item(&app), (true, false), "mixed: unchecked");
    toggle_bunri_kinshi(&mut app);
    assert_eq!(flags(&app), (true, true));
    // Kinsoku None on one paragraph: the item is unavailable and the setting stays.
    let second = "あ——い\n".len();
    app.run("text.setFormat", json!({"id": id, "start": second, "end": second, "kinsoku": "none"})).unwrap();
    app.session.set_tool_option("selectAll", &json!(true));
    assert_eq!(bunri_kinshi_item(&app), (false, true));
    assert_eq!(flags(&app), (true, true));
    app.run("text.setFormat", json!({"id": id, "kinsoku": "soft"})).unwrap();
    app.session.set_tool_option("selectAll", &json!(true));
    assert_eq!(bunri_kinshi_item(&app), (true, true));
    toggle_bunri_kinshi(&mut app);
    assert_eq!(flags(&app), (false, false));
    app.session.set_tool_option("selectAll", &json!(true));
    toggle_bunri_kinshi(&mut app);
    app.session.set_tool_option("selectAll", &json!(true));
    reset_panel(&mut app);
    assert_eq!(flags(&app), (false, false), "Reset Panel turns it off");
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

/// More Options at the end of the Bullets presets opens Bullets and Numbering on its Bullets
/// tab, and at the end of the Numbered List presets on its Numbers tab.
#[test]
fn more_options_opens_bullets_and_numbering_on_its_tab() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let (mut app, _) = editing_text("One\nTwo");
    select_range(&mut app, 0, 0);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let numbers = std::cell::Cell::new(false);
    let row = std::cell::Cell::new(egui::Rect::NOTHING);
    let mut draw = |app: &mut VectorcraftApp, ui: &mut Ui| {
        let r = ui.horizontal(|ui| if numbers.get() { numbers_button(app, ui, 28.0) } else { bullets_button(app, ui, 28.0) });
        row.set(r.response.rect);
    };
    for (on_numbers, tab) in [(false, "bullets"), (true, "numbers")] {
        numbers.set(on_numbers);
        super::super::tests_appearance::frame_events(&ctx, &mut app, vec![], &mut draw);
        let arrow = row.get().min + vec2(28.0 + 7.0, 14.0);
        super::super::tests_appearance::click(&ctx, &mut app, arrow, &mut draw);
        let texts = super::super::tests_appearance::frame_events(&ctx, &mut app, vec![], &mut draw);
        let at = super::super::tests_appearance::text_rect(&texts, "More Options").center();
        super::super::tests_appearance::click(&ctx, &mut app, at, &mut draw);
        let d = app.ui.dialog.take().unwrap_or_else(|| panic!("More Options opens the dialog from the {tab} presets"));
        assert_eq!((d.kind.as_str(), d.str("tab")), ("bulletsAndNumbering", tab.to_string()));
    }
    app.run("ui.bulletsAndNumbering", json!({})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some("bulletsAndNumbering"));
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

type Nodes = Vec<(egui::accesskit::NodeId, egui::accesskit::Node)>;

/// An English context for headless Control bar frames, with AccessKit on to find its controls.
fn control_bar_ctx() -> egui::Context {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    ctx
}

/// A document with the type `texts` (`text.create` params) makes, all of it selected.
fn control_bar_app(texts: &[Value]) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 800, "height": 400})).unwrap();
    let ids: Vec<Value> = texts.iter().map(|p| app.run("text.create", p.clone()).unwrap()["id"].clone()).collect();
    app.run("select.set", json!({ "ids": ids })).unwrap();
    app
}

/// The second of two headless frames of the whole Control bar, `width` wide (the first lays it out).
fn control_bar_nodes(ctx: &egui::Context, app: &mut VectorcraftApp, width: f32, events: Vec<egui::Event>) -> Nodes {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 300.0));
    let mut nodes = Vec::new();
    for events in [events, vec![]] {
        let mut out =
            ctx.run_ui(egui::RawInput { events, screen_rect: Some(screen), ..Default::default() }, |ui| crate::chrome::control_bar(app, ui));
        out.textures_delta.clear();
        nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
    }
    nodes
}

/// The control named `name` (a button's or link's label, a label's text).
fn control_bar_node(nodes: &Nodes, name: &str) -> Option<egui::accesskit::Node> {
    nodes.iter().find(|(_, n)| n.label() == Some(name) || n.value() == Some(name)).map(|(_, n)| n.clone())
}

/// Where the control named `name` ends, if it shows.
fn control_bar_end(nodes: &Nodes, name: &str) -> Option<f64> {
    control_bar_node(nodes, name).and_then(|n| n.bounds()).map(|b| b.x1)
}

/// The Control bar's Paragraph alignment buttons.
const BAR_ALIGNMENTS: [&str; 3] = ["Align left", "Align center", "Align right"];

/// #5372: with type selected, the Control bar shows a Paragraph section after the Character one, as
/// the public documentation's Control panel does: a Paragraph: link that opens the Paragraph panel
/// in a popover under it, and the Align left, Align center and Align right buttons. A click aligns
/// the selected type, and the button of its alignment shows selected. The buttons show on a bar of
/// common window widths.
#[test]
fn the_control_bar_shows_the_paragraph_alignment_of_the_selected_type() {
    let ctx = control_bar_ctx();
    let mut app = control_bar_app(&[json!({"x": 20, "y": 50, "text": "Point"})]);
    // Pinned, so that the room the buttons need can't creep up past common window widths. The
    // bar's right end keeps the panel menu button's room.
    for width in [1536.0, 1920.0] {
        let nodes = control_bar_nodes(&ctx, &mut app, width, vec![]);
        let ends = BAR_ALIGNMENTS.map(|n| control_bar_end(&nodes, n));
        assert!(ends.iter().all(|e| e.is_some_and(|x| x <= f64::from(width) - 10.0)), "pinned: the buttons show on a {width} pt window: {ends:?}");
    }
    let width = 1920.0;
    let nodes = control_bar_nodes(&ctx, &mut app, width, vec![]);
    let center = |n: &egui::accesskit::Node| {
        let b = n.bounds().unwrap_or_else(|| panic!("{n:?}"));
        egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)
    };
    let click = |app: &mut VectorcraftApp, at: egui::Pos2| {
        let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        control_bar_nodes(&ctx, app, width, vec![egui::Event::PointerMoved(at), press(true)]);
        control_bar_nodes(&ctx, app, width, vec![press(false)]);
        control_bar_nodes(&ctx, app, width, vec![])
    };
    // A selected button shows pressed.
    let on = |n: &egui::accesskit::Node| n.toggled() == Some(egui::accesskit::Toggled::True);
    let button = control_bar_node(&nodes, "Align center").unwrap_or_else(|| panic!("the Control bar has Align center"));
    assert!(!on(&button), "left-aligned type: Align center is not selected");
    assert!(control_bar_node(&nodes, "Align left").is_some_and(|n| on(&n)), "Align left shows selected");
    let undo = app.session.active().unwrap().history.undo.len();
    let nodes = click(&mut app, center(&button));
    assert_eq!(text_style(&app).unwrap().1.justify, Justify::Center, "{}", app.ui.status);
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "one undo step");
    assert!(control_bar_node(&nodes, "Align center").is_some_and(|n| on(&n)), "Align center shows selected");
    assert!(!control_bar_node(&nodes, "Align left").is_some_and(|n| on(&n)), "Align left no longer selected");
    // The Paragraph: link opens the Paragraph panel in a popover: its justifications show there,
    // no panel docks, and a second click closes it.
    let link = control_bar_node(&nodes, "Paragraph:").unwrap_or_else(|| panic!("the Paragraph: link"));
    assert_eq!(link.description(), Some("Paragraph options"));
    assert!(control_bar_node(&nodes, "Justify all lines").is_none(), "closed");
    let nodes = click(&mut app, center(&link));
    assert!(control_bar_node(&nodes, "Justify all lines").is_some(), "the popover shows the Paragraph panel");
    assert_eq!(app.ui.open_panel, None, "no panel docks");
    let nodes = click(&mut app, center(&link));
    assert!(control_bar_node(&nodes, "Justify all lines").is_none(), "a second click closes it");
}

/// The Control bar's Character and Paragraph groups hold the type controls; each off leaves the
/// other, and with Paragraph off the Align buttons fit a narrower window.
#[test]
fn the_character_and_paragraph_groups_hold_the_type_controls() {
    let ctx = control_bar_ctx();
    let mut app = control_bar_app(&[json!({"x": 20, "y": 50, "text": "Point"})]);
    let set = |app: &mut VectorcraftApp, group: &str, visible: bool| {
        app.run("window.control.options", json!({ "group": group, "visible": visible })).unwrap();
    };
    // The narrowest window, in steps of 10 points, whose bar shows the Align buttons.
    let narrowest = |app: &mut VectorcraftApp| {
        (40..=240).map(|w| w as f32 * 10.0).find(|&w| control_bar_node(&control_bar_nodes(&ctx, app, w, vec![]), "Horizontal Align Left").is_some())
    };
    let nodes = control_bar_nodes(&ctx, &mut app, 1920.0, vec![]);
    for name in ["Character:", "Paragraph:", "Align left"] {
        assert!(control_bar_node(&nodes, name).is_some(), "{name} shows");
    }
    let all = narrowest(&mut app).expect("the Align buttons show in a wide window");
    set(&mut app, "paragraph", false);
    let nodes = control_bar_nodes(&ctx, &mut app, 1920.0, vec![]);
    assert!(control_bar_node(&nodes, "Paragraph:").is_none() && control_bar_node(&nodes, "Align left").is_none(), "Paragraph is off");
    assert!(control_bar_node(&nodes, "Character:").is_some(), "Character stays");
    let fewer = narrowest(&mut app).expect("the Align buttons show without Paragraph");
    assert!(fewer < all, "the Align buttons fit {fewer} wide without Paragraph, {all} with it");
    set(&mut app, "paragraph", true);
    set(&mut app, "character", false);
    let nodes = control_bar_nodes(&ctx, &mut app, 1920.0, vec![]);
    assert!(control_bar_node(&nodes, "Character:").is_none(), "Character is off");
    assert!(control_bar_node(&nodes, "Paragraph:").is_some(), "Paragraph stays");
}

/// #5372: at any width the Control bar trims itself from its right end, ending inside its margin.
/// With type selected the inline X/Y/W/H fields go first, then the Distribute buttons, the Align To
/// menu and the Align buttons, area type's vertical alignment, the Paragraph buttons and last the
/// Paragraph: link, while the Transform link keeps its room; a wider bar never leaves out what a
/// narrower one showed. For one point type object, area type (its vertical alignment) and two
/// objects (the Distribute buttons).
#[test]
fn the_control_bar_trims_its_paragraph_section_after_the_controls_to_its_right() {
    let ctx = control_bar_ctx();
    let point = |x: i32| json!({"x": x, "y": 50, "text": "Point"});
    let area = json!({"x": 20, "y": 20, "text": "Area", "area": {"width": 100, "height": 120}});
    const VERTICAL: [&str; 4] = ["Align Top", "Align Center", "Align Bottom", "Justify Vertically"];
    for (case, texts) in [("point type", vec![point(20)]), ("area type", vec![area]), ("two objects", vec![point(20), point(300)])] {
        let mut app = control_bar_app(&texts);
        // Each group of controls in the bar's order, by its first and last control.
        let mut groups: Vec<(&str, Vec<&str>)> = vec![("link", vec!["Paragraph:"]), ("buttons", BAR_ALIGNMENTS.to_vec())];
        if case == "area type" {
            groups.push(("vertical alignment", VERTICAL.to_vec()));
        }
        groups.push(("Align", vec!["Horizontal Align Left", "Vertical Align Bottom"]));
        groups.push(("Align To", vec!["Align To:"]));
        if case == "two objects" {
            groups.push(("Distribute", vec!["Vertical Distribute Top", "Horizontal Distribute Right"]));
        }
        groups.push(("X/Y/W/H", vec!["X:", "H:"]));
        let mut seen = vec![false; groups.len()];
        for width in (600..=2600).step_by(10) {
            let nodes = control_bar_nodes(&ctx, &mut app, width as f32, vec![]);
            let inside = |x: f64| x <= f64::from(width) - 10.0;
            let shown: Vec<bool> = groups
                .iter()
                .map(|(group, names)| {
                    let ends: Vec<Option<f64>> = names.iter().map(|n| control_bar_end(&nodes, n)).collect();
                    assert!(ends.iter().all(Option::is_some) || ends.iter().all(Option::is_none), "{case}, {width}: {group} {ends:?}");
                    if let Some(Some(end)) = ends.last() {
                        assert!(inside(*end), "{case}, {width}: {group} ends at {end}");
                    }
                    ends.iter().all(Option::is_some)
                })
                .collect();
            // Narrower bars are already full before the section (the Character controls).
            let transform = control_bar_end(&nodes, "Transform");
            assert!(!shown[0] || transform.is_some_and(inside), "{case}, {width}: Transform ends at {transform:?}");
            for (i, ((group, _), on)) in groups.iter().zip(&shown).enumerate() {
                assert!(!seen[i] || *on, "{case}, {width}: {group} left out after a narrower bar showed it");
                assert!(!*on || shown[..i].iter().all(|s| *s), "{case}, {width}: {group} shows while a group before it is left out");
                seen[i] |= on;
            }
        }
        assert!(seen.iter().all(|s| *s), "{case}: a wide bar shows every group: {seen:?}");
    }
}

/// Reset Panel gives the paragraphs it acts on new type's Mojikumi Set (Line-end Punctuation
/// Half Width) and no tab stops, as one undo step named Reset Panel: while the Type tool edits,
/// the paragraph at its caret (the typing before it stays a step of its own), and then the
/// selected type's paragraphs. In an undo group that is already open, the reset is part of
/// that group's step.
#[test]
fn reset_panel_resets_the_mojikumi_set_and_tab_stops_in_one_step() {
    let (mut app, id) = editing_text("First\nSecond");
    app.run("text.setFormat", json!({"id": id, "mojikumi": "none"})).unwrap();
    app.run("text.tabs.set", json!({"id": id, "stops": [{"position": 36}, {"position": 90, "align": "right"}]})).unwrap();
    let text = |app: &VectorcraftApp| {
        let node = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap();
        let NodeKind::Text(text) = &node.kind else { panic!("text") };
        text.as_ref().clone()
    };
    let paras = |app: &VectorcraftApp| {
        let text = text(app);
        [text.para_at(0), text.para_at(1)].map(|p| (p.mojikumi, p.tabs.len()))
    };
    let custom = (Mojikumi::None, 2);
    let reset = (Mojikumi::LineEndHalf, 0);
    assert_eq!(paras(&app), [custom, custom]);
    let steps = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
    let last_step = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.last().map(|e| e.label.clone());
    // Type over the second paragraph's text: the typing session is still open.
    select_range(&mut app, 6, 12);
    let before = steps(&app);
    let ctx = egui::Context::default();
    let raw = egui::RawInput { events: vec![egui::Event::Text("X".into())], ..Default::default() };
    ctx.run_ui(raw, |ui| crate::shortcuts::handle(&mut app, ui.ctx())).textures_delta.clear();
    assert_eq!(app.session.tool_options()["typing"], json!(true));
    reset_panel(&mut app);
    assert_eq!(paras(&app), [custom, reset], "only the paragraph at the caret");
    assert_eq!((steps(&app), last_step(&app)), (before + 2, Some("Reset Panel".to_string())));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(paras(&app), [custom, custom]);
    assert_eq!(text(&app).plain_text(), "First\nX", "the typing stays");
    app.run("edit.redo", json!({})).unwrap();
    app.session.set_tool_option("stopEditing", &json!(true));
    let doc = app.session.active().unwrap().doc.clone();
    let before = steps(&app);
    reset_panel(&mut app);
    assert_eq!(paras(&app), [reset, reset]);
    assert_eq!((steps(&app), last_step(&app)), (before + 1, Some("Reset Panel".to_string())));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc, doc, "one Undo takes the whole reset back");
    // In an undo group that is already open (MCP `command_batch` with `atomic`), the reset is
    // part of that group's step.
    app.run("edit.undoGroup.begin", json!({"label": "Batch"})).unwrap();
    let before = steps(&app);
    reset_panel(&mut app);
    assert_eq!(paras(&app), [reset, reset]);
    app.run("text.setFormat", json!({"id": id, "mojikumi": "none"})).unwrap();
    app.run("edit.undoGroup.end", json!({})).unwrap();
    assert_eq!((steps(&app), last_step(&app)), (before + 1, Some("Batch".to_string())));
}

/// Reset Panel gives paragraphs that start with different Mojikumi Sets and tab stops the same
/// Mojikumi Set and no tab stops, as one undo step, and one Undo gives each paragraph its own
/// values back: while the Type tool edits, the paragraphs the selection touches, and then the
/// paragraphs of every selected type object.
#[test]
fn reset_panel_resets_paragraphs_with_different_values_in_one_step() {
    let (mut app, id) = editing_text("First\nSecond\nThird");
    app.run("text.setFormat", json!({"id": id, "start": 0, "end": 1, "mojikumi": "none"})).unwrap();
    app.run("text.setFormat", json!({"id": id, "start": 6, "end": 7, "mojikumi": "lineEndHalf"})).unwrap();
    app.run("text.setFormat", json!({"id": id, "start": 13, "end": 14, "mojikumi": "none"})).unwrap();
    app.run("text.tabs.set", json!({"id": id, "start": 0, "end": 1, "stops": [{"position": 36}, {"position": 90, "align": "right"}]})).unwrap();
    app.run("text.tabs.set", json!({"id": id, "start": 6, "end": 7, "stops": [{"position": 120, "align": "decimal", "leader": "."}]})).unwrap();
    let paras = |app: &VectorcraftApp, id: &Value| {
        let node = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap();
        let NodeKind::Text(text) = &node.kind else { panic!("text") };
        text.paragraph_styles().into_iter().map(|p| (p.mojikumi, p.tabs.iter().map(|t| t.position).collect::<Vec<_>>())).collect::<Vec<_>>()
    };
    let first = (Mojikumi::None, vec![36.0, 90.0]);
    let second = (Mojikumi::LineEndHalf, vec![120.0]);
    let third = (Mojikumi::None, vec![]);
    let reset = (Mojikumi::LineEndHalf, vec![]);
    assert_eq!(paras(&app, &id), [first.clone(), second.clone(), third.clone()]);
    let steps = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
    let last_step = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.last().map(|e| e.label.clone());
    // A selection from the first paragraph into the second.
    select_range(&mut app, 2, 8);
    let doc = app.session.active().unwrap().doc.clone();
    let before = steps(&app);
    reset_panel(&mut app);
    assert_eq!(paras(&app, &id), [reset.clone(), reset.clone(), third.clone()], "only the paragraphs the selection touches");
    assert_eq!((steps(&app), last_step(&app)), (before + 1, Some("Reset Panel".to_string())));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc, doc, "one Undo takes the whole reset back");
    assert_eq!(paras(&app, &id), [first.clone(), second.clone(), third.clone()]);
    // Two selected type objects.
    app.session.set_tool_option("stopEditing", &json!(true));
    let other = app.run("text.create", json!({"x": 40, "y": 120, "text": "Other"})).unwrap()["id"].clone();
    app.run("text.setFormat", json!({"id": other, "mojikumi": "none"})).unwrap();
    app.run("text.tabs.set", json!({"id": other, "stops": [{"position": 48, "align": "center"}]})).unwrap();
    let other_values = (Mojikumi::None, vec![48.0]);
    app.run("select.set", json!({"ids": [id, other]})).unwrap();
    let doc = app.session.active().unwrap().doc.clone();
    let before = steps(&app);
    reset_panel(&mut app);
    assert_eq!(paras(&app, &id), [reset.clone(), reset.clone(), reset.clone()]);
    assert_eq!(paras(&app, &other), [reset]);
    assert_eq!((steps(&app), last_step(&app)), (before + 1, Some("Reset Panel".to_string())));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc, doc, "one Undo takes the whole reset back");
    assert_eq!(paras(&app, &id), [first, second, third]);
    assert_eq!(paras(&app, &other), [other_values]);
}

/// Reset Panel closes only the undo group it opens: an undo group open in another document
/// stays open, and that document's later edits stay part of its step.
#[test]
fn reset_panel_leaves_an_undo_group_open_in_another_document() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    let a = app.run("text.create", json!({"x": 20, "y": 50, "text": "A"})).unwrap()["id"].clone();
    // The second document is open before the group begins, so switching to it opens no group in it.
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    assert_eq!(app.session.active_index(), Some(1));
    app.run("text.create", json!({"x": 20, "y": 50, "text": "B"})).unwrap();
    app.session.set_active(0);
    app.run("edit.undoGroup.begin", json!({"label": "Batch"})).unwrap();
    app.run("text.setFormat", json!({"id": a, "mojikumi": "none"})).unwrap();
    let steps = |st: &vectorcraft_engine::DocState| (st.history.undo.len(), st.history.undo.last().map(|e| e.label.clone()));
    let a_steps = steps(&app.session.documents()[0]);
    assert_eq!(a_steps.1, Some("Batch".to_string()));
    app.session.set_active(1);
    assert!(app.session.active().unwrap().undo_group.is_none());
    let b_steps = steps(app.session.active().unwrap()).0;
    reset_panel(&mut app);
    assert_eq!(steps(app.session.active().unwrap()), (b_steps + 1, Some("Reset Panel".to_string())));
    assert!(app.session.active().unwrap().undo_group.is_none());
    assert!(app.session.documents()[0].undo_group.is_some(), "the other document's undo group stays open");
    app.session.set_active(0);
    app.run("text.setFormat", json!({"id": a, "mojikumi": "lineEndHalf"})).unwrap();
    app.run("edit.undoGroup.end", json!({})).unwrap();
    assert_eq!(steps(app.session.active().unwrap()), a_steps, "the later edit is part of the Batch step");
}

/// With the Selection tool a selected frame of a threaded story stands for the whole story: the
/// Paragraph panel's changes and Reset Panel reach its locked and hidden frames, each in one undo
/// step, other type keeps its paragraphs, and the alignment buttons read every frame.
#[test]
fn selection_tool_paragraph_changes_reach_every_frame_of_the_story() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 900, "height": 300})).unwrap();
    let story = "The quick brown fox jumps over the lazy dog.\n".repeat(12);
    let area = json!({"x": 10, "y": 10, "text": story, "size": 12, "area": {"width": 150, "height": 60}});
    let a = app.run("text.create", area).unwrap()["id"].as_u64().unwrap();
    let b = app.run("text.thread.add", json!({"from": a, "rect": [200, 10, 150, 60]})).unwrap()["id"].as_u64().unwrap();
    let c = app.run("text.thread.add", json!({"from": b, "rect": [400, 10, 150, 200]})).unwrap()["id"].as_u64().unwrap();
    let other = app.run("text.create", json!({"x": 10, "y": 250, "text": "Other", "size": 12})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [b]})).unwrap();
    app.run("object.lock", json!({})).unwrap();
    app.run("select.set", json!({"ids": [c]})).unwrap();
    app.run("object.hide", json!({})).unwrap();
    app.run("select.set", json!({"ids": [a]})).unwrap();
    let paragraphs = |app: &VectorcraftApp, id: u64| match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => (0..t.paragraph_count()).map(|i| t.para_at(i).clone()).collect::<Vec<_>>(),
        _ => panic!("not text"),
    };
    let undo_steps = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
    for id in [a, b, c] {
        assert!(!paragraphs(&app, id).is_empty(), "frame {id} holds text");
    }

    let steps = undo_steps(&app);
    format(&mut app, json!({"spaceBefore": 4}));
    assert_eq!(undo_steps(&app), steps + 1, "one undo step");
    for id in [a, b, c] {
        assert!(paragraphs(&app, id).iter().all(|p| (p.space_before - 4.0).abs() < 1e-9), "frame {id} takes the space");
    }
    assert!(paragraphs(&app, other).iter().all(|p| p.space_before.abs() < 1e-9), "other type keeps its paragraphs");
    assert_eq!(shared_para_field(&app, |p| p.space_before), Some(4.0));

    align(&mut app, "center").unwrap();
    for id in [a, b, c] {
        assert!(paragraphs(&app, id).iter().all(|p| p.justify == Justify::Center), "frame {id} is centered");
    }
    assert!(paragraphs(&app, other).iter().all(|p| p.justify != Justify::Center));
    assert_eq!(selected_alignment(&app, &ParaStyle::default()), Some(Justify::Center));
    // A hidden frame aligned otherwise leaves no alignment button on.
    app.run("text.setStyle", json!({"ids": [c], "justify": "right"})).unwrap();
    app.run("select.set", json!({"ids": [a]})).unwrap();
    assert_eq!(selected_alignment(&app, &ParaStyle::default()), None);

    let steps = undo_steps(&app);
    reset_panel(&mut app);
    assert_eq!(undo_steps(&app), steps + 1, "Reset Panel is one undo step");
    for id in [a, b, c] {
        assert!(paragraphs(&app, id).iter().all(|p| p.space_before.abs() < 1e-9 && p.justify == Justify::Auto), "frame {id} is reset");
    }
}
