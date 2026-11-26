use super::*;
use vectorcraft_doc::Kinsoku;
use vectorcraft_doc::Mojikumi;
use vectorcraft_doc::NodeId;
use vectorcraft_engine::Session;

/// The alignments the panel shows selected: none while the paragraphs differ or no type is selected.
fn selected(app: &VectorcraftApp) -> Vec<Justify> {
    let row = values(app).align.and_then(|a| TEXT_ALIGNS.iter().position(|&t| t == a)).and_then(|i| ALIGNMENTS.get(i));
    row.map(|r| r.0).into_iter().collect()
}

/// Whether `states` holds `r` with exactly the state `state(enabled, checked)` makes.
fn has_state(states: &CommandStates, r: &CommandRef, enabled: bool, checked: Option<bool>) -> bool {
    format!("{:?}", states.get(r)) == format!("{:?}", Some(&state(enabled, checked)))
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

/// Cmd+Alt+Shift+C switches between Single-line and Every-line and keeps what each paragraph's
/// composer does with complex scripts: a Standard composer toggles to the other Standard one.
#[test]
fn toggling_the_line_composer_keeps_the_shaping() {
    let (mut app, id) = editing_text("A\nB\n");
    for (from, first, second) in [
        ("standardEveryLine", "standardSingleLine", "standardEveryLine"),
        ("everyLine", "singleLine", "everyLine"),
        ("standardSingleLine", "standardEveryLine", "standardSingleLine"),
    ] {
        app.run("text.setFormat", json!({"id": id, "composer": from})).unwrap();
        app.session.set_tool_option("selectAll", &json!(true));
        assert_eq!(toggle_composer(&mut app).unwrap(), json!(first), "{from}");
        assert_eq!(toggle_composer(&mut app).unwrap(), json!(second), "{from} again");
    }
    // Paragraphs that break differently take Every-line, as before the Standard composers existed.
    app.run("text.setFormat", json!({"id": id, "composer": "standardSingleLine"})).unwrap();
    app.run("text.setFormat", json!({"id": id, "start": 2, "end": 3, "composer": "singleLine"})).unwrap();
    app.session.set_tool_option("selectAll", &json!(true));
    assert_eq!(toggle_composer(&mut app).unwrap(), json!("everyLine"));
    // Paragraphs that both break every-line but differ in shaping: Single-line, Middle Eastern & South Asian.
    app.run("text.setFormat", json!({"id": id, "composer": "everyLine"})).unwrap();
    app.run("text.setFormat", json!({"id": id, "start": 2, "end": 3, "composer": "standardEveryLine"})).unwrap();
    app.session.set_tool_option("selectAll", &json!(true));
    assert_eq!(toggle_composer(&mut app).unwrap(), json!("singleLine"));
}

/// The Paragraph panel menu lists the four composers, and ticks the one every selected paragraph has.
#[test]
fn the_paragraph_menu_sets_each_of_the_four_composers() {
    let (mut app, _) = editing_text("A\nB\n");
    app.session.set_tool_option("selectAll", &json!(true));
    // Each composer row sets its composer, and the menu checks that row (and only it).
    for (_, chosen) in composers() {
        let key = composer_id(chosen);
        run(&mut app, "ui.paragraph.set", &json!({"key": "composer", "value": key})).unwrap().unwrap();
        app.session.set_tool_option("selectAll", &json!(true));
        assert_eq!(shared_composer(&app), Some(chosen), "{key}");
        let mut states = CommandStates::new();
        menu_states(&app, &mut states);
        for (_, row) in composers() {
            assert!(has_state(&states, &set_ref("composer", composer_id(row)), true, Some(row == chosen)), "{key}: row {}", composer_id(row));
        }
    }
    assert!(run(&mut app, "ui.paragraph.set", &json!({"key": "composer"})).unwrap().is_err(), "a row without its value is refused");
    assert!(run(&mut app, "ui.other", &json!({})).is_none());
}

/// Kinsoku Set shows a custom set by its name, a blank when the paragraphs' sets differ, even when
/// each has Hard under it; Bunri-Kinshi is available with a custom set.
#[test]
fn kinsoku_set_shows_a_custom_set_by_name() {
    let (mut app, id) = editing_text("A\nB\n");
    app.run("type.kinsokuSets.set", json!({"sets": [{"name": "Tight", "cannotStart": "」"}]})).unwrap();
    app.run("text.setFormat", json!({"id": id, "kinsoku": "hard"})).unwrap();
    app.run("text.setFormat", json!({"id": id, "start": 2, "end": 3, "kinsoku": "tight"})).unwrap();
    app.session.set_tool_option("selectAll", &json!(true));
    assert_eq!(shared_para(&app, |p| p.kinsoku), Some(Kinsoku::Hard));
    assert_eq!(shared_kinsoku_name(&app), None);
    app.run("text.setFormat", json!({"id": id, "kinsoku": "Tight"})).unwrap();
    assert_eq!(shared_kinsoku_name(&app).as_deref(), Some("Tight"));
    assert!(bunri_kinshi_item(&app).0);
    app.run("text.setFormat", json!({"id": id, "kinsoku": "none"})).unwrap();
    assert_eq!(shared_kinsoku_name(&app).as_deref(), Some("None"));
    assert!(!bunri_kinshi_item(&app).0);
}

/// The Mojikumi Set menu lists None, the four other predefined sets and Line-end Punctuation Half
/// Width, each under its own label in every interface language. Each entry sets the selected
/// paragraphs' set in one undo step and is shown as the current set; paragraphs with different
/// sets show a blank menu, and Reset Panel gives them Line-end Punctuation Half Width.
#[test]
fn mojikumi_menu_offers_and_sets_every_predefined_set() {
    let keys: Vec<&str> = mojikumi_sets().iter().map(|s| s.2).collect();
    assert_eq!(keys, ["none", "yakumonoHankaku", "gyomatsuYakumonoHankaku", "gyomatsuYakumonoZenkaku", "yakumonoZenkaku", "lineEndHalf"]);
    use crate::i18n::LangExt as _;
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

/// Kurikaeshi Moji Shori: new type has it off; the menu item turns it on for the paragraphs the Type
/// tool's selection touches in one undo step, a click while the selected paragraphs differ turns it
/// on for all of them, and a click while all have it turns it off. Reset Panel turns it off.
#[test]
fn kurikaeshi_moji_shori_toggle_follows_the_selected_paragraphs() {
    let (mut app, id) = editing_text("時々\nすゝ\n");
    app.session.prefs.show_east_asian_options = true;
    let flags = |app: &VectorcraftApp| {
        let node = app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap();
        let NodeKind::Text(text) = &node.kind else { panic!("text") };
        (text.para_at(0).kurikaeshi_moji_shori, text.para_at(1).kurikaeshi_moji_shori)
    };
    assert_eq!(flags(&app), (false, false), "new type has it off");
    select_range(&mut app, 0, 3);
    let history = app.session.active().unwrap().history.undo.len();
    run(&mut app, "ui.paragraph.toggle", &json!({"key": "kurikaeshiMojiShori"})).unwrap().unwrap();
    assert_eq!(flags(&app), (true, false));
    assert_eq!(app.session.active().unwrap().history.undo.len(), history + 1);
    app.session.set_tool_option("selectAll", &json!(true));
    toggle_kurikaeshi_moji_shori(&mut app);
    assert_eq!(flags(&app), (true, true), "mixed: a click turns it on for all");
    app.session.set_tool_option("selectAll", &json!(true));
    toggle_kurikaeshi_moji_shori(&mut app);
    assert_eq!(flags(&app), (false, false), "all on: a click turns it off");
    app.session.set_tool_option("selectAll", &json!(true));
    toggle_kurikaeshi_moji_shori(&mut app);
    app.session.set_tool_option("selectAll", &json!(true));
    reset_panel(&mut app);
    assert_eq!(flags(&app), (false, false), "Reset Panel turns it off");
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
    assert!(selected(&app).is_empty(), "mixed paragraphs must not light one alignment");
    apply(&mut app, &ParagraphChange::Align(TextAlign::Center));
    assert_eq!(selected(&app), [Justify::Center]);
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
    assert!(selected(&app).is_empty());
}

#[test]
fn paragraph_mixed_alignment_caret_unicode_boundaries_and_uniform_controls() {
    let (mut app, id) = editing_text("é\nאב\n");
    app.run("text.setStyle", json!({"id": id, "justify": "left"})).unwrap();
    app.run("text.setStyle", json!({"id": id, "start": 3, "end": 7, "justify": "right"})).unwrap();
    app.run("text.setStyle", json!({"id": id, "start": 8, "end": 8, "justify": "justifyAll"})).unwrap();
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
        assert_eq!(selected(&app), expected, "range {start}..{end}");
    }
    for (expected, _, _, key) in ALIGNMENTS {
        app.run("text.setStyle", json!({"id": id, "justify": key})).unwrap();
        app.session.set_tool_option("selectAll", &json!(true));
        assert_eq!(selected(&app), [expected]);
    }
}

#[test]
fn paragraph_mixed_alignment_auto_direction_and_split_runs() {
    let (mut app, id) = editing_text("é\nאב");
    // Character formatting splits the RTL paragraph across runs.
    app.run("text.setRangeStyle", json!({"id": id, "start": 3, "end": 5, "size": 24})).unwrap();
    app.session.set_tool_option("selectAll", &json!(true));
    assert!(selected(&app).is_empty(), "Auto resolves separately per paragraph");
    select_range(&mut app, 3, 7);
    assert_eq!(selected(&app), [Justify::Right]);
    app.run("text.setFormat", json!({"id": id, "start": 3, "end": 7, "direction": "leftToRight"})).unwrap();
    app.session.set_tool_option("selectAll", &json!(true));
    assert_eq!(selected(&app), [Justify::Left]);
    app.run("text.setStyle", json!({"id": id, "start": 0, "end": 2, "justify": "left"})).unwrap();
    assert_eq!(selected(&app), [Justify::Left], "explicit Left and resolved Auto agree");
    app.run("text.setFormat", json!({"id": id, "start": 0, "end": 2, "direction": "rightToLeft"})).unwrap();
    assert_eq!(selected(&app), [Justify::Left], "explicit alignment ignores direction");
}

#[test]
fn paragraph_mixed_alignment_explicit_rtl_and_empty_controls() {
    let (mut app, id) = editing_text("Latin\nאב\n\n");
    app.run("text.setStyle", json!({"id": id, "justify": "auto"})).unwrap();
    app.run("text.setFormat", json!({"id": id, "direction": "rightToLeft"})).unwrap();
    // Auto with an explicit direction still needs the real, possibly split, paragraph text.
    app.run("text.setRangeStyle", json!({"id": id, "start": 0, "end": 2, "size": 24})).unwrap();
    for (start, end, expected) in [
        (0, 5, Justify::Right),  // nonempty Latin with explicit RTL
        (6, 10, Justify::Right), // nonempty Hebrew with explicit RTL
        (11, 11, Justify::Left), // existing resolver's empty paragraph behavior
        (12, 12, Justify::Left), // empty trailing paragraph
    ] {
        select_range(&mut app, start, end);
        assert_eq!(selected(&app), [expected], "explicit RTL range {start}..{end}");
    }
    app.run("text.setStyle", json!({"id": id, "start": 0, "end": 5, "justify": "left"})).unwrap();
    select_range(&mut app, 0, 10);
    assert!(selected(&app).is_empty(), "explicit Left and Auto/RTL differ");
    let (mut empty, id) = editing_text("");
    empty.run("text.setStyle", json!({"id": id, "justify": "auto"})).unwrap();
    empty.run("text.setFormat", json!({"id": id, "direction": "rightToLeft"})).unwrap();
    assert_eq!(selected(&empty), [Justify::Left]);
}

#[test]
fn paragraph_mixed_alignment_object_group_and_no_text_controls() {
    let (mut app, id) = editing_text("A\nB");
    app.run("text.setStyle", json!({"id": id, "justify": "left"})).unwrap();
    app.run("text.setStyle", json!({"id": id, "start": 2, "end": 3, "justify": "right"})).unwrap();
    app.session.set_tool_option("stopEditing", &json!(true));
    assert!(selected(&app).is_empty(), "whole object covers every paragraph");
    app.run("object.group", json!({})).unwrap();
    assert!(selected(&app).is_empty(), "group descendant has mixed paragraphs");
    app.run("text.setStyle", json!({"justify": "center"})).unwrap();
    assert_eq!(selected(&app), [Justify::Center]);
    let group = app.session.active().unwrap().selection.objects.clone();
    let other = app.run("text.create", json!({"x": 20, "y": 100, "text": "C"})).unwrap()["id"].clone();
    let mut ids: Vec<Value> = group.iter().map(|id| json!(id.0)).collect();
    ids.push(other);
    app.run("select.set", json!({"ids": ids})).unwrap();
    assert!(selected(&app).is_empty(), "different selected objects");
    app.run("select.set", json!({"ids": []})).unwrap();
    assert!(text_style(&app).is_none(), "no text: the panel shows its empty state");
    assert!(selected(&app).is_empty());
    let (empty, _) = editing_text("");
    assert_eq!(selected(&empty), [Justify::Left]);
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
    // The direction buttons set the paragraph's direction and the panel shows it.
    apply(&mut app, &ParagraphChange::Direction(ParagraphDirection::RightToLeft));
    assert_eq!(text_style(&app).unwrap().1.direction, Some(ParaDirection::RightToLeft));
    assert_eq!(values(&app).direction, Some(ParagraphDirection::RightToLeft));
    apply(&mut app, &ParagraphChange::Direction(ParagraphDirection::LeftToRight));
    assert_eq!(values(&app).direction, Some(ParagraphDirection::LeftToRight));
}

/// Insert Kashidas is in the panel with the Middle Eastern (Indic) options only, shows the
/// paragraph's setting, and Reset Panel sets it back to None.
#[test]
fn insert_kashidas_shows_with_the_indic_options_and_reset_panel_turns_it_off() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "كتب محمد"})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    let kashida = |app: &VectorcraftApp| {
        let node = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap();
        let NodeKind::Text(text) = &node.kind else { panic!("text") };
        text.para_at(0).kashida
    };
    assert_eq!(shared_para(&app, |p| p.kashida), None.or(Some(vectorcraft_doc::text::Kashida::None)));
    format(&mut app, json!({"kashida": "medium"}));
    assert_eq!(kashida(&app), vectorcraft_doc::text::Kashida::Medium);
    assert_eq!(shared_para(&app, |p| p.kashida), Some(vectorcraft_doc::text::Kashida::Medium));
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
    toggle_list(&mut app, ListKind::Bullets);
    assert_eq!(lists(&app), (Some(ParaList::bulleted(Bullet::Dot)), None), "the caret's paragraph");
    assert_eq!(bullets_shown(&app), Some(Some(Bullet::Dot)), "the button shows pressed");
    // None, the first preset, is picked last.
    for (_, key, bullet) in bullet_presets().into_iter().rev() {
        set_bullet(&mut app, key);
        assert_eq!(lists(&app), (bullet.map(ParaList::bulleted), None), "{key}");
    }
    assert_eq!(bullets_shown(&app), Some(None), "None turned the list off");
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
    toggle_list(&mut app, ListKind::Numbers);
    assert_eq!(lists(&app), (numbered(NumberStyle::Arabic), None), "the caret's paragraph");
    assert_eq!(numbers_shown(&app), Some(Some(NumberStyle::Arabic)), "the button shows pressed");
    assert_eq!(number_sample(NumberStyle::UpperRoman, 3), "I, II, III");
    for style in NumberStyle::ALL {
        set_number_style(&mut app, Some(style));
        assert_eq!(lists(&app), (numbered(style), None), "{}", number_sample(style, 3));
    }
    set_number_style(&mut app, None);
    assert_eq!(lists(&app), (None, None));
    app.session.set_tool_option("selectAll", &json!(true));
    app.run("type.applyNumbers", json!({})).unwrap();
    assert_eq!(lists(&app), (numbered(NumberStyle::Arabic), numbered(NumberStyle::Arabic)));
    // The Numbered List button shows pressed and the Bullets button doesn't, and the other way
    // round for a bulleted list.
    let pressed = |app: &VectorcraftApp| (matches!(bullets_shown(app), Some(Some(_))), matches!(numbers_shown(app), Some(Some(_))));
    assert_eq!(pressed(&app), (false, true));
    app.session.set_tool_option("selectAll", &json!(true));
    app.run("type.applyBullets", json!({})).unwrap();
    assert_eq!(pressed(&app), (true, false));
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
    use crate::dialogs::bullets_numbering::BulletsDialog;
    use craft::ui::dialog::StackDialog as _;
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let (mut app, _) = editing_text("One\nTwo");
    app = crate::testing::windowed(app);
    select_range(&mut app, 0, 0);
    let shown_tab = |app: &VectorcraftApp| {
        app.dialogs().expect("a window").with(|host| host.get::<BulletsDialog>().map(|d| d.json_fields().unwrap_or_default()["tab"].clone()))
    };
    for tab in ["bullets", "numbers"] {
        more_options(&mut app, tab);
        assert_eq!(shown_tab(&app), Some(json!(tab)), "More Options opens the dialog on {tab}");
    }
    app.run("ui.bulletsAndNumbering", json!({})).unwrap();
    assert!(shown_tab(&app).is_some());
}

/// Bullets and Numbering draws every preset of each tab, and its buttons.
#[test]
fn bullets_and_numbering_draws_every_preset_and_its_buttons() {
    use vectorcraft_doc::text::NumberStyle;
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let (app, _) = editing_text("One\nTwo");
    let mut p = crate::tests::panel_harness::Panel::new(crate::testing::windowed(app), crate::tests::picks::dialogs);
    select_range(&mut p.app(), 0, 0);
    let last_number = NumberStyle::ALL.last().map(|&style| crate::panels::paragraph::number_sample(style, 3)).unwrap_or_default();
    for (tab, last) in [("bullets", "Filled Square".to_owned()), ("numbers", last_number)] {
        p.app().run("ui.bulletsAndNumbering", json!({ "tab": tab })).unwrap();
        p.frame();
        for name in ["None", last.as_str(), "OK", "Cancel"] {
            assert!(p.shows(name), "{name} is drawn on {tab}");
        }
    }
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
        matches!(item, crate::menus::Node::Submenu { label, children, .. } if label == "Bullets and Numbering"
            && children.iter().any(|n| matches!(n, crate::menus::Node::Item(i) if i.label == "Convert to Text" && i.command.id == "type.convertListToText")))
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

/// With the Selection tool, Convert to Text takes the same story as Apply Numbers: the numbers
/// of every frame of the selected story, its locked and hidden frames included, become characters
/// of the text in one undo step, and other type keeps its list.
#[test]
fn selection_tool_convert_to_text_takes_every_frame_of_the_story() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 900, "height": 300})).unwrap();
    let story = "The quick brown fox jumps over the lazy dog.\n".repeat(12);
    let area = json!({"x": 10, "y": 10, "text": story, "size": 12, "area": {"width": 150, "height": 60}});
    let a = app.run("text.create", area).unwrap()["id"].as_u64().unwrap();
    let b = app.run("text.thread.add", json!({"from": a, "rect": [200, 10, 150, 60]})).unwrap()["id"].as_u64().unwrap();
    let c = app.run("text.thread.add", json!({"from": b, "rect": [400, 10, 150, 200]})).unwrap()["id"].as_u64().unwrap();
    let other = app.run("text.create", json!({"x": 10, "y": 250, "text": "Other", "size": 12})).unwrap()["id"].as_u64().unwrap();
    app.run("text.setFormat", json!({"ids": [other], "list": "bullet"})).unwrap();
    app.run("select.set", json!({"ids": [b]})).unwrap();
    app.run("object.lock", json!({})).unwrap();
    app.run("select.set", json!({"ids": [c]})).unwrap();
    app.run("object.hide", json!({})).unwrap();
    app.run("select.set", json!({"ids": [a]})).unwrap();
    let frame = |app: &VectorcraftApp, id: u64| match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => (t.plain_text(), (0..t.paragraph_count()).map(|i| t.para_at(i).list.is_some()).collect::<Vec<_>>()),
        _ => panic!("not text"),
    };
    let undo_steps = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
    app.run("type.applyNumbers", json!({})).unwrap();
    for id in [a, b, c] {
        let (_, lists) = frame(&app, id);
        assert!(!lists.is_empty() && lists.iter().all(|l| *l), "Apply Numbers numbers frame {id}");
    }
    let before = frame(&app, c);

    let steps = undo_steps(&app);
    app.run("type.convertListToText", json!({})).unwrap();
    assert_eq!(undo_steps(&app), steps + 1, "one undo step");
    for id in [a, b, c] {
        let (plain, lists) = frame(&app, id);
        assert!(lists.iter().all(|l| !*l), "frame {id} leaves the list");
        assert!(plain.contains(".\tThe quick"), "frame {id} holds its numbers as text: {plain:?}");
    }
    assert_eq!(frame(&app, other), ("Other".to_string(), vec![true]), "other type keeps its list");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(frame(&app, c), before, "undo restores the hidden frame");
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

/// The names the Control bar's Paragraph section shows, from the second of two headless frames.
fn bar_names(app: &mut VectorcraftApp) -> Vec<String> {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    // PLAN: assumed `testing::ui_context()` (hub-w1, S-SH4) returns an `egui::Context` with the framework installed.
    let ctx = crate::testing::ui_context();
    ctx.enable_accesskit();
    let mut names = Vec::new();
    for _ in 0..2 {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            control_bar(app, ui);
        });
        out.textures_delta.clear();
        let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
        names = nodes.iter().flat_map(|(_, n)| [n.label(), n.value()]).flatten().map(str::to_string).collect();
    }
    names
}

/// #5372: with type selected the Control bar shows a Paragraph section: the Paragraph: link and the
/// Align left, Align center and Align right buttons (the justifying four are in the panel the link
/// opens); with no type selected it shows none of them.
// PLAN: the trimming order as the bar narrows (toggles out before the link, ch.K30) is chrome-w16's `Row::fit_rank` and is tested with its fixture.
#[test]
fn the_control_bar_shows_its_paragraph_section_for_selected_type_only() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 800, "height": 400})).unwrap();
    let section = ["Paragraph:", "Align left", "Align center", "Align right"];
    let names = bar_names(&mut app);
    assert!(section.iter().all(|s| !names.iter().any(|n| n == s)), "nothing without type: {names:?}");
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "One"})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    let names = bar_names(&mut app);
    assert!(section.iter().all(|s| names.iter().any(|n| n == s)), "the link and three alignments: {names:?}");
    assert!(!names.iter().any(|n| n.starts_with("Justify")), "the justifying alignments are in the panel: {names:?}");
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
    let ctx = crate::testing::ui_context();
    // The keymap builds in the first pass.
    crate::tests::synthetic::key_frame(&mut app, &ctx, vec![]);
    crate::tests::synthetic::key_frame(&mut app, &ctx, vec![egui::Event::Text("X".into())]);
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
    let b = app.run("text.create", json!({"x": 20, "y": 50, "text": "B"})).unwrap()["id"].clone();
    // A reset that changes nothing records no step, so B starts away from the panel's defaults.
    app.run("text.setFormat", json!({"id": b, "mojikumi": "none"})).unwrap();
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
