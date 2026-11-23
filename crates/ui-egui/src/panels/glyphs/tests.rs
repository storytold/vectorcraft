// PLAN: assumed `GlyphCell`, `CellInfo`, `AlternatePick`, the `GlyphSource` trait, `GlyphsPanelSettings`, `VcGlyphs`, `Fit`, `Pick`,
// `activate`, `pick_alternate`, `marked_cell`, `menu_rows`, `menu_states` and `run` reach this module through `use super::*`
// (type-w15 imports them in `glyphs.rs`).
use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_text::FontDb;

use super::*;

/// The Type tool editing `text` in a new document.
fn editing(text: &str) -> (VectorcraftApp, Value) {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    app.session.select_tool("type", Default::default()).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": text})).unwrap()["id"].clone();
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    (app, id)
}

fn select(app: &mut VectorcraftApp, start: usize, end: usize) {
    app.session.set_tool_option("select", &json!({"start": start, "end": end}));
}

fn undo_len(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().history.undo.len()
}

/// The plain text of the text object `id`.
fn plain_of(app: &VectorcraftApp, id: u64) -> String {
    match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.plain_text(),
        _ => panic!("text"),
    }
}

/// The face the panel browses: the chosen font, else the selected text's.
fn browsed(app: &VectorcraftApp) -> (String, String) {
    app.ui
        .glyphs_font
        .clone()
        .or_else(|| crate::panels::character::text_style(app).map(|(s, _)| (s.font_family, s.font_style)))
        .unwrap_or_else(|| (vectorcraft_text::FALLBACK_FAMILY.to_string(), "Regular".to_string()))
}

/// The panel's cell source, as the panel builds it.
fn source(app: &mut VectorcraftApp) -> VcGlyphs<'static> {
    VcGlyphs::of(app)
}

/// The cells the panel lists for `category` of the browsed face.
fn listed(app: &mut VectorcraftApp, category: &str) -> Vec<GlyphCell> {
    let (family, style) = browsed(app);
    source(app).cells(&family, &style, category)
}

/// What the source says about `cell` of the browsed face.
fn cell_info(app: &mut VectorcraftApp, cell: GlyphCell) -> CellInfo {
    let (family, style) = browsed(app);
    source(app).describe(&family, &style, cell)
}

/// The cells of the popup of `cell`, its own glyph first.
fn popup_of(app: &mut VectorcraftApp, cell: GlyphCell) -> Vec<GlyphCell> {
    let (family, style) = browsed(app);
    source(app).alternates(&family, &style, cell)
}

/// Use the popup of `cell` to pick `glyph`.
fn pick(app: &mut VectorcraftApp, cell: GlyphCell, glyph: GlyphCell) {
    pick_alternate(app, cell, glyph);
}

fn show_category(app: &mut VectorcraftApp, category: &str) {
    app.ui.glyphs_panel.category = category.into();
}

/// The glyph ids of the text `id`, in the order of its characters.
fn glyph_ids(app: &VectorcraftApp, id: &Value) -> Vec<u32> {
    let NodeKind::Text(text) = &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind else { panic!("text") };
    let mut glyphs = vectorcraft_text::layout(FontDb::global(), text).glyphs;
    glyphs.sort_by_key(|g| g.byte);
    glyphs.iter().map(|g| g.gid).collect()
}

/// The Type tool editing `text` set in the alternates test font.
fn basic_latin_of_the_alternates_font(text: &str) -> (VectorcraftApp, Value) {
    use vectorcraft_text::test_fonts::{alternates_family, alternates_font};
    FontDb::global().add_font(alternates_font(true).expect("the test font builds"));
    let (mut app, id) = editing(text);
    app.run("text.setRangeStyle", json!({"id": id, "font": alternates_family(true)})).unwrap();
    (app, id)
}

/// A story of three threaded frames in the alternates test font, the Type tool editing the first with everything selected.
fn threaded(text: &str, area: Value, frames: &[Value]) -> (VectorcraftApp, Vec<u64>) {
    use vectorcraft_text::test_fonts::{alternates_family, alternates_font};
    FontDb::global().add_font(alternates_font(true).expect("the test font builds"));
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 900, "height": 300})).unwrap();
    let first = app.run("text.create", json!({"x": 10, "y": 10, "text": text, "size": 12, "area": area})).unwrap()["id"].as_u64().unwrap();
    let mut ids = vec![first];
    for rect in frames {
        let from = *ids.last().unwrap_or(&first);
        ids.push(app.run("text.thread.add", json!({"from": from, "rect": rect})).unwrap()["id"].as_u64().unwrap());
    }
    for id in &ids {
        app.run("text.setRangeStyle", json!({"id": id, "font": alternates_family(true)})).unwrap();
    }
    app.run("select.set", json!({"ids": [first]})).unwrap();
    app.session.select_tool("type", Default::default()).unwrap();
    (app, ids)
}

/// What activating glyph `gid` of `face` would do to the selection ([`Fit::pick`], [`Fit::why`]).
fn verdict(app: &mut VectorcraftApp, face: &vectorcraft_text::FontFace, gid: u32) -> Result<Pick, String> {
    let fit = Fit::of(app, face);
    fit.pick(face, gid).ok_or_else(|| fit.why(face, gid))
}

/// The Kufi Murabba PIIIX Heavy (`~/Library/Fonts`), in the app's font database; `None` when it is not installed. A file that is
/// there but gives no face is a failure.
fn kufi_heavy() -> Option<std::sync::Arc<vectorcraft_text::FontFace>> {
    let path = std::path::Path::new(&std::env::var_os("HOME")?).join("Library/Fonts/KufiMurabbaPIIIX-Heavy.ttf");
    if !path.exists() {
        eprintln!("skipped: {} is not installed", path.display());
        return None;
    }
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{} exists but cannot be read: {e}", path.display()));
    FontDb::global().add_font(bytes);
    let face = FontDb::global().face("Kufi Murabba PIIIX", "Heavy").unwrap_or_else(|| panic!("{} exists but its face is missing", path.display()));
    assert_eq!((face.family.as_str(), face.style.as_str()), ("Kufi Murabba PIIIX", "Heavy"), "the installed font, not a fallback");
    Some(face)
}

fn gid_named(face: &vectorcraft_text::FontFace, name: &str) -> u32 {
    (0..face.glyph_count()).find(|g| face.glyph_name(*g).as_deref() == Some(name)).unwrap_or_else(|| panic!("{name} missing"))
}

/// The word al-Khwarizmi ending in `letter`, in Kufi Murabba PIIIX Heavy.
fn khwarizmi_in_heavy(letter: char) -> (VectorcraftApp, Value, usize) {
    let word = format!("\u{627}\u{644}\u{62E}\u{648}\u{627}\u{631}\u{632}\u{645}{letter}");
    let (mut app, id) = editing(&word);
    app.run("text.setRangeStyle", json!({"id": id, "font": "Kufi Murabba PIIIX", "style": "Heavy"})).unwrap();
    (app, id, word.len() - letter.len_utf8())
}

/// The characters of `face` a grid lists: no control or white space characters.
fn listed_chars(face: &vectorcraft_text::FontFace) -> Vec<(char, u32)> {
    face.chars().into_iter().filter(|(c, _)| !c.is_control() && !c.is_whitespace()).collect()
}

/// The command rows of the panel menu.
fn rows(app: &VectorcraftApp) -> Vec<CommandRef> {
    menu_rows(app)
        .into_iter()
        .filter_map(|row| match row {
            PanelRow::Command(command) => Some(command),
            _ => None,
        })
        .collect()
}

/// Choose the panel menu row of `form`: what a click on it runs.
fn choose(app: &mut VectorcraftApp, form: GlyphForm) {
    let row = form_ref(form);
    assert!(rows(app).contains(&row), "{form:?} missing from the menu");
    run(app, &row.id, &row.params).unwrap();
}

/// Whether the row of `form` is checked and enabled.
fn row_state(app: &VectorcraftApp, form: GlyphForm) -> (bool, bool) {
    let mut states = CommandStates::new();
    menu_states(app, &mut states);
    let state = states.get(&form_ref(form)).unwrap_or_else(|| panic!("no state for {form:?}"));
    (state.is_checked() == Some(true), state.is_enabled())
}

#[test]
fn the_selected_character_is_the_range_start_or_the_one_before_the_caret() {
    let (mut app, id) = editing("aHé");
    select(&mut app, 1, 2);
    assert_eq!(selected_char(&app), Some('H'));
    select(&mut app, 2, 2);
    assert_eq!(selected_char(&app), Some('H'));
    select(&mut app, 4, 1);
    assert_eq!(selected_char(&app), Some('H'), "a backward range starts at its lower end");
    select(&mut app, 0, 0);
    assert_eq!(selected_char(&app), None, "nothing before the caret");
    // Across runs: a style on `H` alone splits the text into three runs.
    app.run("text.setRangeStyle", json!({"id": id, "start": 1, "end": 2, "kerning": 50})).unwrap();
    select(&mut app, 1, 1);
    assert_eq!(selected_char(&app), Some('a'));
    select(&mut app, 2, 2);
    assert_eq!(selected_char(&app), Some('H'));
    select(&mut app, 2, 4);
    assert_eq!(selected_char(&app), Some('é'));
    app.session.select_tool("select", Default::default()).unwrap();
    assert_eq!(selected_char(&app), None, "only the Type tool's selection counts");
}

/// With Show East Asian Options, the panel menu lists the East Asian glyph forms; running one draws the Type tool's selected
/// characters in it, checks it, and Revert To Default Forms takes it away. A form the font has no feature for is disabled. The
/// second half needs craft-fonts' BIZ UDMincho, which has JIS 78 forms and no JIS 04 forms (skipped without it).
#[test]
fn east_asian_glyph_forms_in_the_panel_menu_set_the_selected_characters_form() {
    let (mut app, id) = editing("亜唖");
    app.session.prefs.show_east_asian_options = false;
    assert!(!rows(&app).contains(&form_ref(GlyphForm::Jis78)), "hidden without Show East Asian Options");
    app.session.prefs.show_east_asian_options = true;
    let shown = rows(&app);
    for form in GlyphForm::ALL {
        assert!(shown.contains(&form_ref(form)), "{form:?} in {shown:?}");
    }
    const FAMILY: &str = "BIZ UDMincho";
    if FontDb::global().face(FAMILY, "Regular").is_none_or(|f| f.family != FAMILY) {
        eprintln!("skipped: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout to run it)");
        return;
    }
    app.run("text.setRangeStyle", json!({"id": id, "font": FAMILY})).unwrap();
    let forms = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap_or(0))).unwrap().kind {
        NodeKind::Text(t) => t.runs.iter().map(|r| (r.text.clone(), r.style.glyph_form)).collect::<Vec<_>>(),
        _ => panic!("text"),
    };
    select(&mut app, 3, 6);
    choose(&mut app, GlyphForm::Jis78);
    assert_eq!(forms(&app), [("亜".to_string(), GlyphForm::Default), ("唖".to_string(), GlyphForm::Jis78)]);
    assert_eq!(row_state(&app, GlyphForm::Jis78), (true, true), "checked");
    // A selection whose first font has no JIS 83 forms, and whose second one does, takes them.
    let fallback = vectorcraft_text::FALLBACK_FAMILY;
    if FontDb::global().face(fallback, "Regular").is_some_and(|f| !f.has_substitution_feature(*b"jp83")) {
        app.run("text.setRangeStyle", json!({"id": id, "start": 0, "end": 3, "font": fallback})).unwrap();
        select(&mut app, 0, 6);
        choose(&mut app, GlyphForm::Jis83);
        assert!(forms(&app).iter().all(|(_, f)| *f == GlyphForm::Jis83), "{:?}", forms(&app));
        app.run("text.setRangeStyle", json!({"id": id, "start": 0, "end": 3, "font": FAMILY})).unwrap();
        app.run("text.setRangeStyle", json!({"id": id, "glyphForm": "default"})).unwrap();
        select(&mut app, 3, 6);
        choose(&mut app, GlyphForm::Jis78);
    }
    // Choosing it again changes nothing, and the font has no JIS 04 forms: that row is disabled.
    let n = undo_len(&app);
    choose(&mut app, GlyphForm::Jis78);
    assert_eq!(row_state(&app, GlyphForm::Jis04), (false, false), "no feature for it");
    assert_eq!(undo_len(&app), n);
    assert_eq!(forms(&app)[1].1, GlyphForm::Jis78);
    choose(&mut app, GlyphForm::Default);
    assert!(forms(&app).iter().all(|(_, f)| *f == GlyphForm::Default), "{:?}", forms(&app));
}

/// At a caret the glyph form rows act on all the text of the object, so they are checked, and skipped as changing nothing, by
/// every character's form, not the caret's: with the caret in text of the default form and other text in JIS 78 forms, Revert To
/// Default Forms reverts that text.
#[test]
fn glyph_form_items_at_a_caret_follow_all_the_text() {
    let (mut app, id) = editing("亜唖");
    app.session.prefs.show_east_asian_options = true;
    let node = NodeId(id.as_u64().unwrap_or(0));
    let forms = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(node).unwrap().kind {
        NodeKind::Text(t) => t.runs.iter().map(|r| r.style.glyph_form).collect::<Vec<_>>(),
        _ => panic!("text"),
    };
    app.run("text.setRangeStyle", json!({"id": id, "start": 0, "end": 3, "glyphForm": "jis78"})).unwrap();
    select(&mut app, 6, 6);
    assert_eq!(selection_glyph_form(&app), None, "the caret's form is not every character's");
    choose(&mut app, GlyphForm::Default);
    assert!(forms(&app).iter().all(|f| *f == GlyphForm::Default), "{:?}", forms(&app));
    app.run("text.setRangeStyle", json!({"id": id, "glyphForm": "jis78"})).unwrap();
    select(&mut app, 6, 6);
    assert_eq!(selection_glyph_form(&app), Some(GlyphForm::Jis78));
}

/// Choosing the form every character already has is skipped as changing nothing, unless a character has a glyph choice: a form
/// other than the default takes it away, and Revert To Default Forms keeps it.
#[test]
fn a_glyph_choice_lets_the_current_form_be_chosen_again() {
    use vectorcraft_text::test_fonts::{alternates_family, alternates_font, unencoded_glyphs};
    FontDb::global().add_font(alternates_font(false).expect("the test font builds"));
    let [u1, ..] = unencoded_glyphs().expect("unencoded glyphs");
    let (mut app, id) = editing("xay");
    app.run("text.setRangeStyle", json!({"id": id, "font": alternates_family(false), "glyphForm": "jis78"})).unwrap();
    select(&mut app, 3, 3);
    assert!(!glyph_form_changes(&app, GlyphForm::Jis78), "every character has it");
    assert!(glyph_form_changes(&app, GlyphForm::Default));
    app.run("text.setGlyphAlternate", json!({"id": id, "start": 1, "gid": u1})).unwrap();
    select(&mut app, 3, 3);
    assert!(glyph_form_changes(&app, GlyphForm::Jis78), "it takes away the glyph choice");
    app.run("text.setRangeStyle", json!({"id": id, "glyphForm": "default"})).unwrap();
    select(&mut app, 3, 3);
    assert!(!glyph_form_changes(&app, GlyphForm::Default), "Revert To Default Forms keeps the glyph choice");
}

/// Show › Alternates for Selection lists the selected character's alternates from the font of its text, whatever font the panel
/// browses; a pick draws the character with one. A character without alternates lists nothing, and Show stays on the alternates.
#[test]
fn alternates_for_current_selection_lists_and_replaces_the_selected_character() {
    use vectorcraft_text::test_fonts::unencoded_glyphs;
    let [u1, ..] = unencoded_glyphs().expect("unencoded glyphs");
    let (mut app, id) = basic_latin_of_the_alternates_font("xay");
    select(&mut app, 2, 2);
    let entire = listed(&mut app, "Entire Font").len();
    show_category(&mut app, "Alternates for Selection");
    let list = listed(&mut app, "Alternates for Selection");
    assert_eq!(list.len(), 7, "a, its six alternates and nothing else (the whole font lists {entire})");
    assert_eq!(marked_cell(&mut app), list.first().copied(), "the default glyph is the one drawn");
    app.run("text.setRangeStyle", json!({"id": id, "start": 1, "end": 2, "allCaps": true})).unwrap();
    assert_eq!(marked_cell(&mut app), None, "All Caps draws a different glyph, so no alternate of a is marked");
    app.run("text.setRangeStyle", json!({"id": id, "start": 1, "end": 2, "allCaps": false})).unwrap();
    assert_eq!(marked_cell(&mut app), list.first().copied());

    // The panel's own font does not change what is listed.
    app.ui.glyphs_font = Some(("Source Serif 4".into(), "Regular".into()));
    assert_eq!(listed(&mut app, "Alternates for Selection").len(), 7);
    app.ui.glyphs_font = Some(("Unavailable Panel Font".into(), "Regular".into()));
    assert_eq!(listed(&mut app, "Alternates for Selection").len(), 7, "the selected text's face still supplies alternates");
    app.ui.glyphs_font = None;

    // A double-click on the second cell draws the a with that glyph, as one undo step.
    let undo = undo_len(&app);
    activate(&mut app, list[1]);
    assert_eq!(undo_len(&app), undo + 1, "status: {}", app.ui.status);
    assert_eq!(plain_of(&app, id.as_u64().unwrap()), "xay");
    assert_eq!(glyph_ids(&app, &id)[1], u1, "the a is drawn with the chosen glyph");
    assert_eq!(marked_cell(&mut app), Some(list[1]), "the chosen glyph is marked");

    select(&mut app, 1, 1);
    assert!(listed(&mut app, "Alternates for Selection").is_empty(), "x has no alternates");
    assert_eq!(app.ui.glyphs_panel.category, "Alternates for Selection", "Show stays on the alternates");
    app.run("text.setRangeStyle", json!({"id": id, "font": "No Such Font Family"})).unwrap();
    select(&mut app, 2, 2);
    assert!(listed(&mut app, "Alternates for Selection").is_empty(), "the selected character's font is not available");
    app.session.select_tool("select", Default::default()).unwrap();
    assert!(listed(&mut app, "Alternates for Selection").is_empty(), "no Type tool selection");

    // Back to Entire Font: the font's characters again.
    show_category(&mut app, "Entire Font");
    assert!(listed(&mut app, "Entire Font").len() > 7);
}

/// The alternates and the glyph drawn are found once per document revision: a change that does not count as an edit leaves what
/// was found as it was, and the next revision finds them again.
#[test]
fn alternates_are_found_once_per_revision() {
    use vectorcraft_text::test_fonts::{alternates_family, unencoded_glyphs};
    let [u1, ..] = unencoded_glyphs().expect("unencoded glyphs");
    let (mut app, id) = basic_latin_of_the_alternates_font("xay");
    select(&mut app, 2, 2);
    show_category(&mut app, "Alternates for Selection");
    let key = |app: &mut VectorcraftApp| {
        listed(app, "Alternates for Selection");
        // PLAN: assumed `FoundKey: Clone + PartialEq + Debug`.
        app.canvas.glyphs_found.as_ref().map(|(k, _)| *k).expect("the alternates were found")
    };
    let first = key(&mut app);
    assert_eq!(marked_cell(&mut app), listed(&mut app, "Alternates for Selection").first().copied(), "the default glyph is drawn");
    let face = FontDb::global().face(alternates_family(true), "Regular").expect("the test face");
    let node = NodeId(id.as_u64().unwrap());
    let state = app.session.active_mut().unwrap();
    let Some(NodeKind::Text(text)) = std::sync::Arc::make_mut(&mut state.doc).node_mut(node).map(|n| &mut n.kind) else { panic!("text") };
    vectorcraft_text::edit::style_range(&mut text.runs, 1, 2, |st| st.glyph_alternate = Some(face.choice('a', u1)));
    assert_eq!(key(&mut app), first, "the same revision keeps what was found");
    app.session.active_mut().unwrap().revision += 1;
    assert_ne!(key(&mut app), first, "the next revision finds the chosen glyph");
}

/// A font whose `aalt` does not list the default glyph (here no `aalt` at all) still lists it, first, so a chosen alternate can
/// be put back.
#[test]
fn the_default_glyph_is_listed_when_the_font_leaves_it_out() {
    use vectorcraft_text::test_fonts::{alternates_family, alternates_font, unencoded_glyphs};
    FontDb::global().add_font(alternates_font(false).expect("the test font builds"));
    let [u1, ..] = unencoded_glyphs().expect("unencoded glyphs");
    let (mut app, id) = editing("xay");
    app.run("text.setRangeStyle", json!({"id": id, "font": alternates_family(false)})).unwrap();
    select(&mut app, 2, 2);
    show_category(&mut app, "Alternates for Selection");
    let list = listed(&mut app, "Alternates for Selection");
    assert_eq!(list.len(), 7, "the default glyph and the six alternates");
    assert_eq!(marked_cell(&mut app), list.first().copied(), "the default glyph is first and drawn");
    let face = FontDb::global().face(alternates_family(false), "Regular").expect("the test face");
    assert_eq!(cell_info(&mut app, list[0]).gid, Some(face.glyph_for('a')), "describe names the default glyph");
    app.run("text.setGlyphAlternate", json!({"id": id, "start": 1, "gid": u1})).unwrap();
    let list = listed(&mut app, "Alternates for Selection");
    assert_eq!(list.len(), 7);
    assert_eq!(marked_cell(&mut app), Some(list[5]), "the chosen glyph, after b, u2, u4 and u3");
}

/// For the selected medial letter of an Arabic word the panel lists that form's alternates, and activating one applies the
/// feature that draws it to that letter alone, as one undo step: the word keeps its joining forms and the text its characters;
/// undo takes it back.
#[test]
fn a_double_click_applies_a_feature_value_to_one_letter_of_a_joined_word() {
    use vectorcraft_text::test_fonts::{JOINING_ALTERNATES_FAMILY, joining_alternates_font, source_sans_gid};
    FontDb::global().add_font(joining_alternates_font().expect("the test font builds"));
    let g = |c| u32::from(source_sans_gid(c).expect("a glyph"));
    let (mut app, id) = editing("\u{628}\u{62A}\u{646}");
    app.run("text.setRangeStyle", json!({"id": id, "font": JOINING_ALTERNATES_FAMILY})).unwrap();
    assert_eq!(glyph_ids(&app, &id), [g('B'), g('u'), g('M')]);
    select(&mut app, 2, 4);
    show_category(&mut app, "Alternates for Selection");
    let list = listed(&mut app, "Alternates for Selection");
    assert_eq!(list.len(), 4, "the medial teh and its three alternates, not the isolated form's");
    assert_eq!(marked_cell(&mut app), list.first().copied(), "the form the letter has is first and drawn");
    let undo = undo_len(&app);
    activate(&mut app, list[2]);
    assert_eq!(undo_len(&app), undo + 1, "one undo step; status: {}", app.ui.status);
    let NodeKind::Text(text) = &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind else { panic!("text") };
    assert_eq!(text.plain_text(), "\u{628}\u{62A}\u{646}", "the text keeps its characters");
    let features: Vec<(String, Vec<String>)> = text.runs.iter().map(|r| (r.text.clone(), r.style.features.clone())).collect();
    assert_eq!(
        features,
        [("\u{628}".to_string(), vec![]), ("\u{62A}".to_string(), vec!["salt=2".to_string()]), ("\u{646}".to_string(), vec![])],
        "the feature is on exactly that letter"
    );
    assert!(text.runs.iter().all(|r| r.style.glyph_alternate.is_none()));
    assert_eq!(glyph_ids(&app, &id), [g('B'), g('x'), g('M')], "the neighbours keep their initial and final forms");
    assert_eq!(marked_cell(&mut app), Some(list[2]), "the applied variant is the one drawn");
    // The default glyph takes the feature off again, and undo takes each step back.
    activate(&mut app, list[0]);
    assert_eq!(glyph_ids(&app, &id), [g('B'), g('u'), g('M')]);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(glyph_ids(&app, &id), [g('B'), g('x'), g('M')]);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(glyph_ids(&app, &id), [g('B'), g('u'), g('M')]);
    let NodeKind::Text(text) = &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind else { panic!("text") };
    assert!(text.runs.iter().all(|r| r.style.features.is_empty()), "undo restores the features");
}

/// A variant no feature draws is still enabled: it replaces the glyph (`text.setGlyphAlternate`).
#[test]
fn a_variant_no_feature_draws_replaces_the_glyph() {
    use vectorcraft_text::test_fonts::alternates_family;
    let (mut app, id) = basic_latin_of_the_alternates_font("xay");
    select(&mut app, 2, 2);
    show_category(&mut app, "Alternates for Selection");
    let list = listed(&mut app, "Alternates for Selection");
    let face = FontDb::global().face(alternates_family(true), "Regular").expect("the test face");
    // The last cell is cv01's glyph c, which the style has no tag for.
    activate(&mut app, list[list.len() - 1]);
    assert_eq!(glyph_ids(&app, &id)[1], face.glyph_for('c'));
    let NodeKind::Text(text) = &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind else { panic!("text") };
    assert!(text.runs.iter().any(|r| r.style.glyph_alternate.is_some()));
    // A feature variant afterwards takes the choice away in the same step.
    let undo = undo_len(&app);
    activate(&mut app, list[2]);
    assert_eq!(undo_len(&app), undo + 1);
    let NodeKind::Text(text) = &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind else { panic!("text") };
    assert!(text.runs.iter().all(|r| r.style.glyph_alternate.is_none()), "the choice is gone");
}

/// A positional alternate without a supported feature still goes through the engine's glyph choice validation, even though the
/// isolated character has different alternates.
#[test]
fn a_positional_variant_no_feature_draws_replaces_the_glyph() {
    use vectorcraft_text::test_fonts::{JOINING_ALTERNATES_FAMILY, joining_alternates_font, source_sans_gid};
    FontDb::global().add_font(joining_alternates_font().expect("the test font builds"));
    let face = FontDb::global().face(JOINING_ALTERNATES_FAMILY, "Regular").expect("the test face");
    let gid = u32::from(source_sans_gid('d').expect("a glyph"));
    assert!(face.alternates('\u{646}').is_empty(), "the isolated noon has no alternates");
    assert!(!face.is_unlisted_alternate('\u{646}', gid), "cv01 reaches the glyph");
    for view in ["Alternates for Selection", "Entire Font"] {
        let (mut app, id) = editing("\u{628}\u{646}\u{62A}");
        app.run("text.setRangeStyle", json!({"id": id, "font": JOINING_ALTERNATES_FAMILY})).unwrap();
        let plain = glyph_ids(&app, &id);
        select(&mut app, 2, 4);
        show_category(&mut app, view);
        let cell = if view == "Entire Font" {
            GlyphCell::Glyph(gid)
        } else {
            let list = listed(&mut app, view);
            assert_eq!(list.len(), 2, "the medial noon and its cv01 alternate");
            list[1]
        };
        let undo = undo_len(&app);
        activate(&mut app, cell);
        assert_eq!(undo_len(&app), undo + 1, "{view}: {}", app.ui.status);
        assert_eq!(glyph_ids(&app, &id)[1], gid, "{view}");
        assert_eq!(plain_of(&app, id.as_u64().unwrap()), "\u{628}\u{646}\u{62A}", "the characters stay unchanged");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(glyph_ids(&app, &id), plain, "{view}: undo restores the word");
    }
}

/// A cell whose glyph has alternates has a popup of the glyph and its alternates in the font's order, and picking an alternate
/// inserts the character at the caret drawn with it, as one undo step.
#[test]
fn a_cells_corner_triangle_opens_its_alternates_and_a_pick_inserts_one() {
    use vectorcraft_text::test_fonts::{alternates_family, unencoded_glyphs};
    let [u1, u2, ..] = unencoded_glyphs().expect("unencoded glyphs");
    let (mut app, id) = basic_latin_of_the_alternates_font("xy");
    let face = FontDb::global().face(alternates_family(true), "Regular").expect("the test face");
    let a = face.glyph_for('a');
    assert!(face.has_alternates(a));
    let cell = GlyphCell::Char('a');
    assert!(cell_info(&mut app, cell).has_alternates, "the cell has the corner action");
    assert!(!cell_info(&mut app, GlyphCell::Char('x')).has_alternates);
    select(&mut app, 1, 1);
    let popup = popup_of(&mut app, cell);
    assert_eq!(popup.len(), 7, "a and its six alternates");
    let plain = glyph_ids(&app, &id);
    let undo = undo_len(&app);
    // a, u1, b, u2: the fourth cell is ss01's u2.
    pick(&mut app, cell, popup[3]);
    assert_eq!(plain_of(&app, id.as_u64().unwrap()), "xay", "status: {}", app.ui.status);
    assert_eq!(glyph_ids(&app, &id), [plain[0], u2, plain[1]], "the inserted a is drawn with the pick");
    assert_eq!(undo_len(&app), undo + 1, "one undo step");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(glyph_ids(&app, &id), plain, "undo takes the character back out");
    // The cell's own glyph inserts the plain character, as an activation does.
    select(&mut app, 1, 1);
    pick(&mut app, cell, popup[0]);
    assert_eq!(glyph_ids(&app, &id), [plain[0], a, plain[1]]);
    // An alternate with no feature that draws it here (u1 is listed by aalt) still goes on.
    select(&mut app, 2, 2);
    pick(&mut app, cell, popup[1]);
    assert_eq!(glyph_ids(&app, &id), [plain[0], a, u1, plain[1]], "status: {}", app.ui.status);
}

/// With the Type tool's selection exactly the cell's character, a pick draws that character with the glyph and keeps the text;
/// picking the glyph it already has adds no undo step. Without the Type tool, and with text in another font, the status says
/// why and nothing changes.
#[test]
fn a_pick_replaces_the_selected_character_and_says_why_when_it_cannot() {
    use vectorcraft_text::test_fonts::{alternates_family, unencoded_glyphs};
    let [_, u2, ..] = unencoded_glyphs().expect("unencoded glyphs");
    let (mut app, id) = basic_latin_of_the_alternates_font("xay");
    let cell = GlyphCell::Char('a');
    let plain = glyph_ids(&app, &id);
    select(&mut app, 1, 2);
    let undo = undo_len(&app);
    let popup = popup_of(&mut app, cell);
    pick(&mut app, cell, popup[3]);
    assert_eq!(plain_of(&app, id.as_u64().unwrap()), "xay", "the selected a stays: status {}", app.ui.status);
    assert_eq!(glyph_ids(&app, &id), [plain[0], u2, plain[2]]);
    assert_eq!(undo_len(&app), undo + 1);
    select(&mut app, 1, 2);
    pick(&mut app, cell, popup[3]);
    assert_eq!(undo_len(&app), undo + 1, "the glyph it already has changes nothing");
    assert_eq!(glyph_ids(&app, &id), [plain[0], u2, plain[2]]);

    // Text set in another font than the one the panel browses.
    app.ui.glyphs_font = Some((alternates_family(true).to_string(), "Regular".to_string()));
    app.run("text.setRangeStyle", json!({"id": id, "font": vectorcraft_text::FALLBACK_FAMILY})).unwrap();
    select(&mut app, 3, 3);
    let undo = undo_len(&app);
    let before = glyph_ids(&app, &id);
    let popup = popup_of(&mut app, cell);
    pick(&mut app, cell, popup[3]);
    assert_eq!(glyph_ids(&app, &id), before);
    assert_eq!(undo_len(&app), undo);
    assert!(app.ui.status.contains("not of the font the selected text is set in"), "{}", app.ui.status);

    // No Type tool caret.
    app.session.select_tool("select", Default::default()).unwrap();
    app.ui.status.clear();
    pick(&mut app, cell, popup[3]);
    assert_eq!(glyph_ids(&app, &id), before);
    assert_eq!(undo_len(&app), undo);
    assert_eq!(app.ui.status, "Select text or click into text with the Type tool to insert glyphs");
}

/// A pick the inserted character cannot be drawn with (All Caps keeps a glyph choice from being drawn) takes the character back
/// out and keeps the status that says why: on its own, and inside an undo group that is already open, which stays open with its
/// earlier edit and takes a pick that can be drawn in place of the selection.
#[test]
fn a_pick_that_cannot_be_drawn_leaves_the_text_as_it_was() {
    let [_, u2, ..] = vectorcraft_text::test_fonts::unencoded_glyphs().expect("unencoded glyphs");
    let (mut app, id) = basic_latin_of_the_alternates_font("xy");
    let n = id.as_u64().unwrap();
    let cell = GlyphCell::Char('a');
    let popup = popup_of(&mut app, cell);
    app.run("text.setFormat", json!({"ids": [id], "allCaps": true})).unwrap();
    // A step to redo, which the failed pick keeps.
    app.run("text.setFormat", json!({"ids": [id], "underline": true})).unwrap();
    app.run("edit.undo", json!({})).unwrap();
    let undo = undo_len(&app);
    select(&mut app, 1, 1);
    app.ui.status.clear();
    pick(&mut app, cell, popup[1]);
    assert_eq!(plain_of(&app, n), "xy");
    assert_eq!(undo_len(&app), undo, "no undo step is left");
    assert_eq!(app.session.active().unwrap().history.redo.len(), 1, "the redo step stays");
    assert!(!app.ui.status.is_empty(), "the status says why");

    // In an open group with no edit yet, the failed pick leaves the group with nothing to undo.
    app.run("edit.undoGroup.begin", json!({})).unwrap();
    select(&mut app, 0, 1);
    pick(&mut app, cell, popup[1]);
    assert_eq!(plain_of(&app, n), "xy");
    assert!(!app.ui.status.is_empty(), "the status says why");
    app.run("edit.undoGroup.end", json!({})).unwrap();
    assert_eq!(undo_len(&app), undo, "the empty group adds no step");

    app.run("edit.undoGroup.begin", json!({})).unwrap();
    app.run("text.setFormat", json!({"ids": [id], "underline": true})).unwrap();
    select(&mut app, 0, 1);
    app.ui.status.clear();
    pick(&mut app, cell, popup[1]);
    assert_eq!(plain_of(&app, n), "xy", "the replaced x is back: status {}", app.ui.status);
    assert!(!app.ui.status.is_empty(), "the status says why");
    assert!(app.session.active().unwrap().undo_group.is_some(), "the group stays open");
    // A pick that can be drawn replaces the selection inside the group.
    app.run("text.setFormat", json!({"ids": [id], "allCaps": false})).unwrap();
    select(&mut app, 0, 1);
    let plain = glyph_ids(&app, &id);
    pick(&mut app, cell, popup[3]);
    assert_eq!(plain_of(&app, n), "ay", "status {}", app.ui.status);
    assert_eq!(glyph_ids(&app, &id), [u2, plain[1]]);
    assert!(app.session.active().unwrap().undo_group.is_some(), "the group stays open");
    app.run("edit.undoGroup.end", json!({})).unwrap();
    assert_eq!(undo_len(&app), undo + 1, "the group is one step");
}

/// After Select All in a threaded story, a pick replaces the whole story's text, as typing does: the character drawn with the
/// glyph stays, in one undo step.
#[test]
fn a_pick_replaces_a_threaded_story_that_select_all_selected() {
    let [_, u2, ..] = vectorcraft_text::test_fonts::unencoded_glyphs().expect("unencoded glyphs");
    let story = "a bad cab ".repeat(30);
    let (mut app, ids) = threaded(&story, json!({"width": 150, "height": 60}), &[json!([200, 10, 150, 60]), json!([400, 10, 150, 200])]);
    let a = ids[0];
    app.session.set_tool_option("textSelection", &json!({"editing": a, "anchor": 0, "caret": 0}));
    app.run("select.all", json!({})).unwrap();
    assert_eq!(crate::panels::character::text_story(&app).map(|s| s.len()), Some(3), "the whole story is selected");
    let undo = undo_len(&app);
    let cell = GlyphCell::Char('a');
    let popup = popup_of(&mut app, cell);
    pick(&mut app, cell, popup[3]);
    let texts: String = ids.iter().map(|id| plain_of(&app, *id)).collect();
    assert_eq!(texts, "a", "status: {}", app.ui.status);
    let first = ids.iter().copied().find(|id| !plain_of(&app, *id).is_empty()).expect("a frame with the character");
    assert_eq!(glyph_ids(&app, &json!(first)), [u2]);
    assert_eq!(undo_len(&app), undo + 1, "one undo step");
}

/// After Select All in a threaded story whose first frame holds only the picked character, a pick still replaces the whole
/// story's text: the other frames' text goes too.
#[test]
fn a_pick_replaces_a_threaded_story_whose_first_frame_holds_one_character() {
    let [_, u2, ..] = vectorcraft_text::test_fonts::unencoded_glyphs().expect("unencoded glyphs");
    // The first frame is wide enough for the "a" only; the ideographs flow on to the second.
    let (mut app, ids) = threaded("a中中中", json!({"width": 16, "height": 16}), &[json!([200, 10, 150, 60])]);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!((plain_of(&app, a), plain_of(&app, b)), ("a".to_string(), "中中中".to_string()));
    app.session.set_tool_option("textSelection", &json!({"editing": a, "anchor": 0, "caret": 0}));
    app.run("select.all", json!({})).unwrap();
    assert_eq!(crate::panels::character::text_editing(&app), Some((NodeId(a), 0, 1)), "the selection is the one character");
    assert_eq!(crate::panels::character::text_story(&app).map(|s| s.len()), Some(2), "the whole story is selected");
    let undo = undo_len(&app);
    let cell = GlyphCell::Char('a');
    let popup = popup_of(&mut app, cell);
    pick(&mut app, cell, popup[3]);
    let texts: String = [a, b].iter().map(|id| plain_of(&app, *id)).collect();
    assert_eq!(texts, "a", "status: {}", app.ui.status);
    assert_eq!(glyph_ids(&app, &json!(a)), [u2]);
    assert_eq!(undo_len(&app), undo + 1, "one undo step");
}

/// A pick at the caret at the end of a full frame of threaded text puts the character in, and the text flows it on to the next
/// frame: it is drawn with the glyph there, and the Type tool edits that frame with the caret after it.
#[test]
fn a_pick_at_the_end_of_a_full_frame_follows_the_character_to_the_next_frame() {
    use vectorcraft_text::test_fonts::{alternates_family, alternates_font, unencoded_glyphs};
    let [_, u2, ..] = unencoded_glyphs().expect("unencoded glyphs");
    FontDb::global().add_font(alternates_font(true).expect("the test font builds"));
    let face = FontDb::global().face(alternates_family(true), "Regular").expect("the test face");
    let gid = |c| face.chars().into_iter().find(|(x, _)| *x == c).map(|(_, g)| g).expect("a glyph");
    let pt = |c| face.advance(gid(c)) * 12.0 / face.units_per_em();
    // The first frame holds the line "a xx " and not the next word. Every character is in the test font, so no fallback face
    // measures the line.
    let width = pt('a') + 2.0 * pt(' ') + 2.0 * pt('x') + pt('x') / 2.0;
    let (mut app, ids) = threaded("a xx xx", json!({"width": width, "height": 16}), &[json!([200, 10, 150, 60])]);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!((plain_of(&app, a), plain_of(&app, b)), ("a xx ".to_string(), "xx".to_string()));
    let end = "a xx ".len();
    app.session.set_tool_option("textSelection", &json!({"editing": a, "anchor": end, "caret": end}));
    let undo = undo_len(&app);
    let cell = GlyphCell::Char('a');
    let popup = popup_of(&mut app, cell);
    pick(&mut app, cell, popup[3]);
    assert_eq!((plain_of(&app, a), plain_of(&app, b)), ("a xx ".to_string(), "axx".to_string()), "status: {}", app.ui.status);
    assert_eq!(glyph_ids(&app, &json!(b))[0], u2);
    assert_eq!(crate::panels::character::text_editing(&app), Some((NodeId(b), 1, 1)), "the caret is after the character");
    assert_eq!(undo_len(&app), undo + 1, "one undo step");
}

/// A pick on the selected last character of a full frame of threaded text, with a glyph wider than the one it had, flows the
/// character on to the next frame: the Type tool edits that frame with the character still selected.
#[test]
fn a_wider_pick_on_the_selected_character_moves_the_selection_with_it() {
    use vectorcraft_text::test_fonts::{alternates_family, alternates_font};
    FontDb::global().add_font(alternates_font(true).expect("the test font builds"));
    let face = FontDb::global().face(alternates_family(true), "Regular").expect("the test face");
    let gid = |c| face.chars().into_iter().find(|(x, _)| *x == c).map(|(_, g)| g).expect("a glyph");
    let plain_a = gid('a');
    // The popup's glyphs in its order: the cell's own glyph, then the alternates.
    let mut list = vec![plain_a];
    for alt in face.alternates('a') {
        if !list.contains(&alt.gid) {
            list.push(alt.gid);
        }
    }
    let (index, wide) =
        list.iter().copied().enumerate().max_by(|x, y| face.advance(x.1).total_cmp(&face.advance(y.1))).expect("the popup lists glyphs");
    assert!(face.advance(wide) > face.advance(plain_a), "an alternate wider than the a");
    // The first frame holds "x a" with the plain a, and not with the wider glyph.
    let pt = |g: u32| face.advance(g) * 12.0 / face.units_per_em();
    let width = pt(gid('x')) + pt(gid(' ')) + (pt(plain_a) + pt(wide)) / 2.0;
    let (mut app, ids) = threaded("x a", json!({"width": width, "height": 16}), &[json!([200, 10, 150, 60])]);
    let (a, b) = (ids[0], ids[1]);
    assert_eq!((plain_of(&app, a), plain_of(&app, b)), ("x a".to_string(), String::new()));
    app.session.set_tool_option("textSelection", &json!({"editing": a, "anchor": 2, "caret": 3}));
    let undo = undo_len(&app);
    let cell = GlyphCell::Char('a');
    let popup = popup_of(&mut app, cell);
    pick(&mut app, cell, popup[index]);
    assert_eq!((plain_of(&app, a), plain_of(&app, b)), ("x ".to_string(), "a".to_string()), "status: {}", app.ui.status);
    assert_eq!(glyph_ids(&app, &json!(b)), [wide]);
    assert_eq!(crate::panels::character::text_editing(&app), Some((NodeId(b), 0, 1)), "the character stays selected");
    assert_eq!(undo_len(&app), undo + 1, "one undo step");
}

/// In Entire Font, a glyph no character maps whose substitutions give other glyphs has a popup too: it lists the glyph and those
/// glyphs, and a pick goes on the selected character as activating that glyph's cell would put it.
#[test]
fn an_unencoded_glyphs_corner_triangle_applies_its_alternates_to_the_selection() {
    use vectorcraft_text::test_fonts::{JOINING_ALTERNATES_FAMILY, joining_alternates_font, source_sans_gid};
    FontDb::global().add_font(joining_alternates_font().expect("the test font builds"));
    let g = |c| u32::from(source_sans_gid(c).expect("a glyph"));
    let (mut app, id) = editing("\u{628}\u{62A}\u{646}");
    app.run("text.setRangeStyle", json!({"id": id, "font": JOINING_ALTERNATES_FAMILY})).unwrap();
    select(&mut app, 2, 4);
    show_category(&mut app, "Entire Font");
    let face = FontDb::global().face(JOINING_ALTERNATES_FAMILY, "Regular").expect("the test face");
    let medial = g('u');
    assert!(face.unencoded_glyphs().contains(&medial) && face.has_alternates(medial), "the medial teh is unencoded and has alternates");
    let cell = GlyphCell::Glyph(medial);
    assert!(cell_info(&mut app, cell).has_alternates, "the cell has the corner action");
    let popup = popup_of(&mut app, cell);
    assert_eq!(popup.len(), 4, "the medial teh and its three alternates");
    let undo = undo_len(&app);
    pick(&mut app, cell, popup[2]);
    assert_eq!(glyph_ids(&app, &id), [g('B'), g('x'), g('M')], "status: {}", app.ui.status);
    assert_eq!(undo_len(&app), undo + 1, "one undo step");
}

/// Show › Entire Font also lists the glyphs no character maps: the encoded characters first, then those.
#[test]
fn entire_font_lists_the_unencoded_glyphs() {
    use vectorcraft_text::test_fonts::{JOINING_ALTERNATES_FAMILY, joining_alternates_font};
    FontDb::global().add_font(joining_alternates_font().expect("the test font builds"));
    let (mut app, id) = editing("\u{628}");
    app.run("text.setRangeStyle", json!({"id": id, "font": JOINING_ALTERNATES_FAMILY})).unwrap();
    let face = FontDb::global().face(JOINING_ALTERNATES_FAMILY, "Regular").expect("the test face");
    let encoded = listed_chars(&face);
    let unencoded = face.unencoded_glyphs();
    assert!(unencoded.len() > 1000 && encoded.len() == 4, "{} unencoded, {} encoded", unencoded.len(), encoded.len());
    let list = listed(&mut app, "Entire Font");
    assert_eq!(list.len(), encoded.len() + unencoded.len());
    let expected: Vec<GlyphCell> = encoded.iter().map(|(c, _)| GlyphCell::Char(*c)).chain(unencoded.iter().map(|g| GlyphCell::Glyph(*g))).collect();
    assert_eq!(list, expected, "characters first, then the unencoded glyphs");
    let first = cell_info(&mut app, GlyphCell::Glyph(unencoded[0]));
    assert!(first.name.is_some(), "the font names its glyphs");
    assert_eq!(first.gid, Some(unencoded[0]));
}

/// The panel menu lists the browsed font's OpenType categories after the character categories, a stylistic set by the font's
/// name for it; showing one lists the glyphs its features give. A font without that category shows Entire Font.
#[test]
fn the_show_menu_lists_the_fonts_opentype_categories() {
    use vectorcraft_text::test_fonts::{CATEGORIES_FAMILY, HALF_FORM_FAMILY, categories_font, half_form_font};
    FontDb::global().add_font(categories_font().expect("the test font builds"));
    FontDb::global().add_font(half_form_font(false).expect("the test font builds"));
    let (mut app, _) = editing("a");
    app.ui.glyphs_font = Some((CATEGORIES_FAMILY.to_string(), "Regular".to_string()));
    let faces = source(&mut app).categories(CATEGORIES_FAMILY, "Regular");
    let names: Vec<&str> = faces.iter().map(|(_, name)| name.as_str()).collect();
    assert_eq!(names, ["Ligatures", "Round Dots", "Small Caps", "Figures"]);
    let id_of = |name: &str| faces.iter().find(|(_, n)| n == name).map(|(id, _)| id.clone()).expect("the category");
    let (figures, dots) = (id_of("Figures"), id_of("Round Dots"));
    run(&mut app, "ui.glyphs.show", &json!({"category": figures})).unwrap();
    assert_eq!(app.ui.glyphs_panel.category, figures);
    assert_eq!(listed(&mut app, &figures).len(), 2);
    run(&mut app, "ui.glyphs.show", &json!({"category": dots})).unwrap();
    assert_eq!(listed(&mut app, &dots).len(), 1);
    // A face without the category shows Entire Font.
    app.ui.glyphs_font = Some((HALF_FORM_FAMILY.to_string(), "Regular".to_string()));
    assert!(source(&mut app).categories(HALF_FORM_FAMILY, "Regular").is_empty(), "the half form font has no categories of its own");
    run(&mut app, "ui.glyphs.show", &json!({"category": dots})).unwrap();
    assert_eq!(app.ui.glyphs_panel.category, "Entire Font", "a category the face lacks leaves Entire Font shown");
}

/// The panel's remembered state is saved with the preferences: a `serde(skip)` slip would lose it, and an older preferences file
/// reads the default.
#[test]
fn the_preview_step_is_kept_in_the_ui_preferences() {
    let mut ui = crate::state::UiState::default();
    let default = ui.glyphs_panel.clone();
    ui.glyphs_panel.category = "Currency".into();
    ui.glyphs_panel.zoom = 1.5;
    let saved = serde_json::to_value(&ui).unwrap();
    assert_eq!(saved["glyphs_panel"]["category"], json!("Currency"), "the category is saved");
    assert!(saved["glyphs_panel"].get("zoom").is_some(), "the zoom is saved");
    let back: crate::state::UiState = serde_json::from_value(saved).unwrap();
    assert_eq!(back.glyphs_panel, ui.glyphs_panel);
    let old: crate::state::UiState = serde_json::from_value(json!({})).unwrap();
    assert_eq!(old.glyphs_panel, default, "an older preferences file reads the default");
}

/// Applying each alternate of the final letter of a word in Kufi Murabba PIIIX Heavy, with the letter selected as the Type tool
/// selects it (the UTF-8 byte range of the character, the last one of a right-to-left word), by activating its cell in
/// Alternates for Selection and in Entire Font: the same glyph, one undo step, the rest of the word as it was. Persian yeh,
/// Arabic yeh and alef maksura each have their own `.fina.alt1` to `.fina.alt6`.
#[test]
fn a_final_letters_alternates_apply_from_alternates_and_from_entire_font() {
    let Some(face) = kufi_heavy() else { return };
    for (letter, base) in [('\u{06CC}', "yehfarsi"), ('\u{064A}', "yeh"), ('\u{0649}', "alefmaksura")] {
        let (mut app, id, last) = khwarizmi_in_heavy(letter);
        let plain = glyph_ids(&app, &id);
        let ends = plain.len() - 1;
        assert_eq!(plain[ends], gid_named(&face, &format!("{base}.fina")), "{base}: the final form");
        select(&mut app, last, last + letter.len_utf8());
        assert_eq!(selected_char(&app), Some(letter), "{base}: the selection is the last letter");
        let alternates: Vec<u32> = (1..=6).map(|n| gid_named(&face, &format!("{base}.fina.alt{n}"))).collect();
        for view in ["Alternates for Selection", "Entire Font"] {
            show_category(&mut app, view);
            for (n, alt) in alternates.iter().enumerate() {
                let undo = undo_len(&app);
                let cell = if view == "Entire Font" {
                    assert!(!cell_info(&mut app, GlyphCell::Glyph(*alt)).dimmed, "{base} {view}: {alt} is not dimmed for its own letter");
                    GlyphCell::Glyph(*alt)
                } else {
                    let list = listed(&mut app, view);
                    assert_eq!(list.len(), 7, "{base}: the final form and its six alternates");
                    list[n + 1]
                };
                activate(&mut app, cell);
                let mut want = plain.clone();
                want[ends] = *alt;
                assert_eq!(glyph_ids(&app, &id), want, "{base} {view} alt{}: only the last letter changes; status: {}", n + 1, app.ui.status);
                assert_eq!(undo_len(&app), undo + 1, "{base} {view} alt{}: one undo step", n + 1);
                app.run("edit.undo", json!({})).unwrap();
                assert_eq!(glyph_ids(&app, &id), plain, "{base} {view}: undo");
            }
        }
    }
}

/// A glyph of another letter is dimmed in Entire Font and activating it changes nothing: the note names the glyph's letter,
/// where in a word it stands and the selected letter.
#[test]
fn a_glyph_that_cannot_go_on_the_selection_is_dimmed_and_says_why() {
    let Some(face) = kufi_heavy() else { return };
    let glyph = gid_named(&face, "yehfarsi.fina.alt2");
    assert_eq!(glyph, 820, "the owner's glyph");
    let cell = GlyphCell::Glyph(glyph);
    for (letter, selected) in [('\u{064A}', "ي (U+064A)"), ('\u{0649}', "ى (U+0649)")] {
        let (mut app, id, last) = khwarizmi_in_heavy(letter);
        let plain = glyph_ids(&app, &id);
        select(&mut app, last, last + letter.len_utf8());
        let why = verdict(&mut app, &face, glyph).err().expect("not applicable");
        assert_eq!(why, format!("yehfarsi.fina.alt2 belongs to ی (U+06CC) at the end of a word; your selection is {selected}."));
        let info = cell_info(&mut app, cell);
        assert!(info.dimmed, "{letter}: dimmed");
        assert_eq!(info.note.as_deref(), Some(why.as_str()), "the note says why");
        let (undo, status) = (undo_len(&app), app.ui.status.clone());
        activate(&mut app, cell);
        assert_eq!(glyph_ids(&app, &id), plain, "{letter}: nothing changes");
        assert_eq!(undo_len(&app), undo);
        assert_eq!(app.ui.status, status, "activating a dimmed cell leaves the status alone");
    }
    // The same glyph on Persian yeh applies.
    let (mut app, id, last) = khwarizmi_in_heavy('\u{06CC}');
    select(&mut app, last, last + 2);
    assert!(!cell_info(&mut app, cell).dimmed);
    activate(&mut app, cell);
    assert_eq!(glyph_ids(&app, &id).last(), Some(&glyph));
    // Nothing selected, and a variant of the letter's other form.
    select(&mut app, 0, 0);
    let why = "Select one character with the Type tool first.";
    assert_eq!(verdict(&mut app, &face, glyph).err().as_deref(), Some(why));
    let info = cell_info(&mut app, cell);
    assert!(info.dimmed, "dimmed without a selection");
    assert_eq!(info.note.as_deref(), Some(why));
    select(&mut app, last, last + 2);
    let medial = gid_named(&face, "yehfarsi.medi.alt1");
    assert_eq!(
        verdict(&mut app, &face, medial).err().as_deref(),
        Some("yehfarsi.medi.alt1 is not an alternate of ی (U+06CC) as it stands in its word."),
        "its own letter, another place in a word"
    );
}

/// A glyph no feature reaches (`a.a`, `a.b` of a font with no feature for them) is applied by Entire Font to the selected
/// character as a glyph choice, not dimmed; one the font's naming gives to another letter (`a.a` on `y`) stays dimmed, and so
/// does every glyph with no character selected.
#[test]
fn entire_font_applies_a_glyph_no_feature_reaches_to_its_letter() {
    use vectorcraft_text::test_fonts::alternates_family;
    let (mut app, id) = basic_latin_of_the_alternates_font("xay");
    let face = FontDb::global().face(alternates_family(true), "Regular").expect("the test face");
    let glyph = gid_named(&face, "a.a");
    assert!(face.feature_reaching(glyph).is_none() && face.glyph_origin(glyph).is_none(), "no feature reaches it");
    let cell = GlyphCell::Glyph(glyph);
    show_category(&mut app, "Entire Font");
    let plain = glyph_ids(&app, &id);
    // Another letter, and nothing selected: dimmed, and activating changes nothing.
    for (start, end) in [(2, 3), (0, 0)] {
        select(&mut app, start, end);
        assert!(verdict(&mut app, &face, glyph).is_err());
        assert!(cell_info(&mut app, cell).dimmed, "{start}..{end}: dimmed");
        activate(&mut app, cell);
        assert_eq!(glyph_ids(&app, &id), plain);
    }
    // The letter itself: enabled, one undo step, drawn with that glyph.
    select(&mut app, 1, 2);
    assert!(matches!(verdict(&mut app, &face, glyph), Ok(Pick::Glyph(g)) if g == glyph));
    assert!(!cell_info(&mut app, cell).dimmed);
    let undo = undo_len(&app);
    activate(&mut app, cell);
    assert_eq!(undo_len(&app), undo + 1);
    assert_eq!(glyph_ids(&app, &id), [plain[0], glyph, plain[2]]);
}

/// A presentation form activated in Entire Font (beh medial, U+FE92) with the caret before the last letter would be a fixed
/// shape that does not join the word: the letter (beh) is inserted, so the word is shaped as if it were typed, and the status
/// says so.
#[test]
fn a_presentation_form_inserts_its_letter_so_the_word_joins() {
    let Some(face) = kufi_heavy() else { return };
    let (mut app, id, last) = khwarizmi_in_heavy('\u{06CC}');
    let shaped = |word: &str| {
        let (mut other, other_id) = editing(word);
        other.run("text.setRangeStyle", json!({"id": other_id, "font": "Kufi Murabba PIIIX", "style": "Heavy"})).unwrap();
        glyph_ids(&other, &other_id)
    };
    let typed = shaped("\u{627}\u{644}\u{62E}\u{648}\u{627}\u{631}\u{632}\u{645}\u{628}\u{6CC}");
    // Typed as the presentation form itself, the same word is shaped differently: it does not join.
    let literal = shaped("\u{627}\u{644}\u{62E}\u{648}\u{627}\u{631}\u{632}\u{645}\u{FE92}\u{6CC}");
    assert_ne!(literal, typed, "the presentation form is not shaped like the letter");
    let gid_of = |c| face.chars().into_iter().find(|(ch, _)| *ch == c).expect("the character is in the font").1;
    assert_eq!(base_letter(&face, '\u{FE92}', gid_of('\u{FE92}')), Some('\u{0628}'), "medial beh is a form of beh");
    assert_eq!(base_letter(&face, '\u{0628}', gid_of('\u{0628}')), None, "beh itself inserts as it is");
    let tip = encoded_tip(&face, '\u{FE92}', gid_of('\u{FE92}'));
    assert!(tip.contains("(U+0628)"), "{tip}");
    select(&mut app, last, last);
    show_category(&mut app, "Entire Font");
    activate(&mut app, GlyphCell::Char('\u{FE92}'));
    let NodeKind::Text(text) = &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind else { panic!("text") };
    assert!(text.plain_text().ends_with("\u{645}\u{628}\u{6CC}"), "the letter is in the text: {}", text.plain_text());
    assert_eq!(glyph_ids(&app, &id), typed, "shaped as the word typed with beh");
    assert!(app.ui.status.starts_with("Inserted ب (U+0628)"), "{}", app.ui.status);
}

#[test]
fn an_unlisted_choice_without_gsub_can_be_reset_from_the_default_cell() {
    use vectorcraft_text::test_fonts::{UNLISTED_VARIABLE_FAMILY, unlisted_variable_font};
    let db = FontDb::global();
    db.add_font(unlisted_variable_font().expect("the test font builds"));
    let face = db.face(UNLISTED_VARIABLE_FAMILY, "Regular").expect("the test face");
    let glyph = gid_named(&face, "a.a");
    let (mut app, id) = editing("a");
    app.run("text.setRangeStyle", json!({"id": id, "font": UNLISTED_VARIABLE_FAMILY})).unwrap();
    select(&mut app, 0, 1);
    let plain = glyph_ids(&app, &id);
    app.run("text.setGlyphAlternate", json!({"id": id, "start": 0, "gid": glyph})).unwrap();
    assert_eq!(glyph_ids(&app, &id), [glyph]);
    show_category(&mut app, "Alternates for Selection");
    let list = listed(&mut app, "Alternates for Selection");
    assert_eq!(list.len(), 1, "the default remains available without any GSUB alternates");
    let undo = undo_len(&app);
    activate(&mut app, list[0]);
    assert_eq!(glyph_ids(&app, &id), plain);
    assert_eq!(undo_len(&app), undo + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(glyph_ids(&app, &id), [glyph]);
}

#[test]
fn entire_font_applies_an_unlisted_glyph_at_the_selections_variable_axes() {
    use vectorcraft_text::test_fonts::{UNLISTED_VARIABLE_FAMILY, unlisted_variable_font};
    let db = FontDb::global();
    db.add_font(unlisted_variable_font().expect("the test font builds"));
    let base = db.face(UNLISTED_VARIABLE_FAMILY, "Regular").expect("the test face");
    let glyph = gid_named(&base, "a.a");
    let (mut app, id) = editing("a");
    app.run("text.setRangeStyle", json!({"id": id, "font": UNLISTED_VARIABLE_FAMILY, "fontAxes": {"wght": 650}})).unwrap();
    select(&mut app, 0, 1);
    let target = selected_target(&app).expect("the selected character");
    let face = db.style_face(target.style).expect("the variable instance");
    assert_ne!(face.id(), base.id(), "a non-default setting has its own face id");
    show_category(&mut app, "Entire Font");
    let cell = GlyphCell::Glyph(glyph);
    assert!(!cell_info(&mut app, cell).dimmed, "the instance's alternate is enabled");
    activate(&mut app, cell);
    assert_eq!(glyph_ids(&app, &id), [glyph]);
    assert_eq!(selected_target(&app).unwrap().style.font_axes["wght"], 650.0);
    app.run("edit.undo", json!({})).unwrap();
    assert_ne!(glyph_ids(&app, &id), [glyph]);
}

#[test]
fn encoded_characters_normalize_only_arabic_presentation_forms() {
    use vectorcraft_text::test_fonts::{ALIASED_JOINING_FAMILY, aliased_joining_font};
    let db = FontDb::global();
    db.add_font(aliased_joining_font().expect("the test font builds"));
    let face = db.face(ALIASED_JOINING_FAMILY, "Regular").expect("the test face");
    let chars = listed_chars(&face);
    for (c, expected) in [('B', "B"), ('\u{FE91}', "\u{0628}")] {
        let (mut app, id) = editing("a");
        app.run("text.setRangeStyle", json!({"id": id, "font": ALIASED_JOINING_FAMILY})).unwrap();
        select(&mut app, 0, 1);
        let at = chars.iter().position(|(ch, _)| *ch == c).expect("an encoded character");
        let origin = face.glyph_origin(chars[at].1).expect("the shared positional glyph");
        assert_eq!(origin.ch, '\u{0628}');
        assert!(origin.position().is_some());
        app.ui.glyphs_font = Some((ALIASED_JOINING_FAMILY.to_string(), "Regular".to_string()));
        activate(&mut app, GlyphCell::Char(c));
        assert_eq!(plain_of(&app, id.as_u64().unwrap()), expected, "the encoded {c} cell inserts the intended character");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(plain_of(&app, id.as_u64().unwrap()), "a");
    }
}

#[test]
fn a_normalized_glyph_without_a_text_target_preserves_the_insertion_error() {
    use vectorcraft_text::test_fonts::{ALIASED_JOINING_FAMILY, aliased_joining_font};
    FontDb::global().add_font(aliased_joining_font().expect("the test font builds"));
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    let undo = undo_len(&app);
    app.ui.glyphs_font = Some((ALIASED_JOINING_FAMILY.to_string(), "Regular".to_string()));
    activate(&mut app, GlyphCell::Char('B'));
    let failed = app.ui.status.clone();
    assert!(!failed.is_empty(), "the ordinary character reports the missing text target");
    activate(&mut app, GlyphCell::Char('\u{FE91}'));
    assert_eq!(app.ui.status, failed, "normalization must not overwrite the insertion failure with a success");
    let state = app.session.active().unwrap();
    assert_eq!(state.history.undo.len(), undo);
    let mut texts = 0;
    state.doc.walk(|n| texts += usize::from(matches!(n.kind, NodeKind::Text(_))));
    assert_eq!(texts, 0, "neither glyph created or changed text");
}

/// The panel lists, for Entire Font and for every category of a face, the cells of the face's own character list: the encoded
/// characters, then the glyphs no character maps. This holds across a change of face back and forth and for a font with no `cmap`.
#[test]
fn the_grid_lists_the_same_cells_from_the_character_list_read_once() {
    use vectorcraft_text::test_fonts::{CATEGORIES_FAMILY, CMAPLESS_FAMILY, categories_font, cmapless_font};

    /// The cells a face's character list gives: for a category, its glyphs mapped by a character first, then the rest; for
    /// Entire Font, every character and the unencoded glyphs.
    fn expected(face: &vectorcraft_text::FontFace, category: Option<usize>) -> Vec<GlyphCell> {
        match category.and_then(|i| face.glyph_categories().get(i)) {
            Some(c) => {
                let wanted: std::collections::HashSet<u32> = c.glyphs.iter().copied().collect();
                let mut mapped: std::collections::HashSet<u32> = std::collections::HashSet::new();
                let chars: Vec<GlyphCell> = listed_chars(face)
                    .into_iter()
                    .filter(|(_, g)| wanted.contains(g) && mapped.insert(*g))
                    .map(|(c, _)| GlyphCell::Char(c))
                    .collect();
                let rest = c.glyphs.iter().copied().filter(|g| !mapped.contains(g)).map(GlyphCell::Glyph);
                chars.into_iter().chain(rest).collect()
            }
            None => listed_chars(face)
                .into_iter()
                .map(|(c, _)| GlyphCell::Char(c))
                .chain(face.unencoded_glyphs().iter().map(|g| GlyphCell::Glyph(*g)))
                .collect(),
        }
    }

    let db = FontDb::global();
    db.add_font(categories_font().expect("the categories test font builds"));
    db.add_font(cmapless_font().expect("the cmapless test font builds"));
    let (mut app, _) = editing("a");
    let faces = [
        (vectorcraft_text::FALLBACK_FAMILY, "Regular"),
        ("Inter", "Regular"),
        (CATEGORIES_FAMILY, "Regular"),
        (vectorcraft_text::FALLBACK_FAMILY, "Regular"),
        (CMAPLESS_FAMILY, "Regular"),
        ("JetBrains Mono", "Regular"),
    ];
    let categories = db.face(CATEGORIES_FAMILY, "Regular").expect("the categories face");
    assert!(!categories.glyph_categories().is_empty(), "the test font has categories");
    let none = db.face(CMAPLESS_FAMILY, "Regular").expect("the cmapless face");
    assert!(none.mapped_chars().is_empty(), "no character is mapped");
    for (family, style) in faces {
        let face = db.face(family, style).unwrap_or_else(|| panic!("{family} {style} is loaded"));
        app.ui.glyphs_font = Some((family.to_string(), style.to_string()));
        assert_eq!(listed(&mut app, "Entire Font"), expected(&face, None), "{family} Entire Font");
        let ids = source(&mut app).categories(family, style);
        for (i, c) in face.glyph_categories().iter().enumerate() {
            let id = &ids.get(i).unwrap_or_else(|| panic!("{family}: category {:?} is listed", c.kind)).0;
            assert_eq!(listed(&mut app, id), expected(&face, Some(i)), "{family} {:?}", c.kind);
        }
    }
    // Entire Font lists every glyph of a font with no `cmap` by glyph id.
    app.ui.glyphs_font = Some((CMAPLESS_FAMILY.to_string(), "Regular".to_string()));
    let list = listed(&mut app, "Entire Font");
    assert!(list.iter().all(|c| matches!(c, GlyphCell::Glyph(_))), "no character is listed");
    assert!(!list.is_empty(), "the glyphs are listed by glyph id");
}
