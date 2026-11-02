//! Type tool state-machine tests (the engine side is covered in vectorcraft-engine's
//! `tests_textedit`).

use super::*;
use crate::ImeSurrounding;
use crate::testutil::{cx, paint};
use vectorcraft_doc::{CharStyle, Document, Node, NodeKind, Selection, TextObject};
use vectorcraft_text::FontDb;

fn doc_with_text(text: &str) -> (Document, NodeId) {
    let mut d = Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let t = TextObject::point(Point::new(100.0, 100.0), text, CharStyle { size: 20.0, ..Default::default() });
    d.insert(Some(l), 0, Node::new(id, NodeKind::Text(Box::new(t)))).unwrap();
    (d, id)
}

fn editing(text: &str) -> (Document, NodeId, TypeTool) {
    let (d, id) = doc_with_text(text);
    let mut tool = TypeTool::new("type");
    tool.start_editing(id, 0);
    (d, id, tool)
}

fn key(tool: &mut TypeTool, d: &Document, k: ToolKey, mods: Mods) -> Vec<Action> {
    let (sel, p) = (Selection::default(), paint());
    tool.key(&cx(d, &sel, &p), k, mods)
}

const SHIFT: Mods = Mods { shift: true, alt: false, cmd: false, ctrl: false, space: false };
const CMD: Mods = Mods { shift: false, alt: false, cmd: true, ctrl: false, space: false };
const ALT: Mods = Mods { shift: false, alt: true, cmd: false, ctrl: false, space: false };
const SHIFT_ALT: Mods = Mods { shift: true, alt: true, cmd: false, ctrl: false, space: false };

/// Apply the tool's previews like the engine would (on top of the session snapshot).
fn apply(d: &mut Document, snapshot: &Document, acts: &[Action]) {
    for a in acts {
        if let Action::Preview(cmd, p) = a {
            assert_eq!(cmd, "text.editRange");
            *d = snapshot.clone();
            let id = NodeId(p["id"].as_u64().unwrap());
            let runs: Vec<TextRun> = serde_json::from_value(p["runs"].clone()).unwrap();
            if let Some(NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) {
                edit::replace_range_styled(&mut t.runs, p["start"].as_u64().unwrap() as usize, p["end"].as_u64().unwrap() as usize, &runs);
            }
        }
    }
}

#[test]
fn type_arrows_keep_devanagari_clusters_with_an_rtl_suffix() {
    // Use the text tests' installed-family order, requiring coverage of the unchanged Hindi input.
    // The Hebrew suffix selects the Type tool's existing visual-arrow route.
    let hindi = "हिन्दी";
    let second = "हि".len();
    assert_eq!((second, hindi.len()), (6, 18));
    let db = FontDb::global();
    let face = ["Kohinoor Devanagari", "ITF Devanagari", "Devanagari Sangam MN", "Noto Sans Devanagari", "Nirmala UI", "Mangal"]
        .into_iter()
        .filter_map(|name| db.face(name, "Regular").filter(|face| face.family == name))
        .find(|face| hindi.chars().all(|ch| face.covers(ch)));
    let Some(face) = face else {
        eprintln!("SKIPPED: type_arrows_keep_devanagari_clusters_with_an_rtl_suffix; no installed candidate font covers the Hindi input");
        return;
    };
    assert!(hindi.chars().all(|ch| face.covers(ch)), "the selected face covers the unchanged Hindi prefix");
    let style = CharStyle { font_family: face.family.clone(), font_style: face.style.clone(), size: 20.0, ..Default::default() };

    // The plain-LTR control runs first. Each case uses a fresh document and tool.
    for (case, text, visual_route) in [("plain_ltr", "हिन्दी", false), ("mixed_rtl", "हिन्दी שלום", true)] {
        let (mut d, id, mut tool) = editing(text);
        let NodeKind::Text(t) = &mut d.node_mut(id).unwrap().kind else { panic!("text fixture") };
        for run in &mut t.runs {
            run.style = style.clone();
        }
        assert_eq!(t.plain_text(), text);
        assert_eq!(&text[..hindi.len()], hindi);
        let lay = vectorcraft_text::layout(db, t);
        assert_eq!(lay.lines.len(), 1, "{case}: one ordinary horizontal line");
        assert!(!lay.vertical, "{case}: horizontal Type");
        let mut hindi_spans: Vec<_> = lay.glyphs.iter().filter(|g| g.byte < hindi.len()).map(|g| (g.byte, g.len)).collect();
        hindi_spans.sort_unstable();
        hindi_spans.dedup();
        // Raw shaping spans are diagnostic; editing boundaries do not depend on their count.
        assert!(!hindi_spans.is_empty(), "{case}: Hindi glyph source spans are present");
        for glyph in lay.glyphs.iter().filter(|g| g.byte < hindi.len()) {
            let used = db.face_by_id(glyph.font_id).expect("the Hindi glyph's loaded face");
            assert!(used.file_data() == face.file_data(), "{case}: Hindi uses the selected face bytes");
            assert_eq!(used.face_index(), face.face_index(), "{case}: Hindi uses the selected face index");
            assert_ne!(glyph.gid, 0, "{case}: Hindi glyph is present");
            assert!(!glyph.rtl, "{case}: Hindi prefix remains LTR");
        }
        assert_eq!(lay.glyphs.iter().any(|g| g.rtl), visual_route, "{case}: actual Type arrow-route predicate");
        if visual_route {
            assert!(lay.glyphs.iter().any(|g| g.rtl && g.byte > hindi.len()), "{case}: the ordinary suffix supplies RTL glyphs");
        }
        assert_eq!(edit::next_cluster(text, second), hindi.len(), "{case}: logical cluster end");
        assert_eq!(edit::prev_cluster(text, hindi.len()), second, "{case}: logical cluster start");
        tool.caret = second;
        tool.anchor = second;
        assert_eq!(tool.editing, Some(id));
        assert_eq!(tool.sel(), (second, second), "{case}: collapsed caret before Right");
        assert!(tool.typing.is_none(), "{case}: no pending typing session");
        key(&mut tool, &d, ToolKey::Right, Mods::default());
        assert_eq!((tool.caret, tool.anchor), (hindi.len(), hindi.len()), "{case}: Right must move 6 -> 18 across the Hindi cluster");
        key(&mut tool, &d, ToolKey::Left, Mods::default());
        assert_eq!((tool.caret, tool.anchor), (second, second), "{case}: Left must move 18 -> 6 across the Hindi cluster");
    }
}

fn assert_unwrapped_hindi_glyphs(
    db: &FontDb,
    face: &vectorcraft_text::FontFace,
    hindi_glyphs: &[&vectorcraft_text::PositionedGlyph],
    hindi: &str,
    text: &str,
) {
    for glyph in hindi_glyphs {
        let used = db.face_by_id(glyph.font_id).expect("the Hindi glyph's loaded face");
        assert!(used.file_data() == face.file_data(), "setup: Hindi uses the selected face bytes");
        assert_eq!(used.face_index(), face.face_index(), "setup: Hindi uses the selected face index");
        assert_ne!(glyph.gid, 0, "setup: Hindi glyph is present");
        assert!(!glyph.rtl, "setup: Hindi prefix is LTR");
        assert!(glyph.len > 0 && glyph.byte + glyph.len <= hindi.len(), "setup: Hindi glyph source range");
        assert!(
            text.is_char_boundary(glyph.byte) && text.is_char_boundary(glyph.byte + glyph.len),
            "setup: glyph source ranges are UTF-8 boundaries"
        );
    }
}

fn assert_wrapped_hindi_glyphs(db: &FontDb, face: &vectorcraft_text::FontFace, lay: &vectorcraft_text::TextLayout, hindi: &str) {
    for glyph in lay.glyphs.iter().filter(|g| g.byte < hindi.len()) {
        let used = db.face_by_id(glyph.font_id).expect("the wrapped Hindi glyph's loaded face");
        assert!(used.file_data() == face.file_data(), "setup: wrapped Hindi uses the selected face bytes");
        assert_eq!(used.face_index(), face.face_index(), "setup: wrapped Hindi uses the selected face index");
        assert_ne!(glyph.gid, 0, "setup: wrapped Hindi glyph is present");
        assert!(!glyph.rtl, "setup: wrapped Hindi prefix stays LTR");
    }
}

#[test]
fn type_arrows_keep_devanagari_clusters_across_wrapped_lines() {
    use vectorcraft_doc::{Composer, Justify};
    use vectorcraft_geom::{Affine, PathData};

    // Use the installed-family order and unchanged Hindi input from the single-line test.
    // Shaping and wrapping capabilities vary by face; an eligibility skip is not arrow coverage.
    let hindi = "हिन्दी";
    let text = "हिन्दी שלום";
    let second = "हि".len();
    let split = "हिन्".len();
    assert_eq!((second, split, hindi.len()), (6, 12, 18));
    let db = FontDb::global();
    let face = ["Kohinoor Devanagari", "ITF Devanagari", "Devanagari Sangam MN", "Noto Sans Devanagari", "Nirmala UI", "Mangal"]
        .into_iter()
        .filter_map(|name| db.face(name, "Regular").filter(|face| face.family == name))
        .find(|face| hindi.chars().all(|ch| face.covers(ch)));
    let Some(face) = face else {
        eprintln!("SKIPPED: type_arrows_keep_devanagari_clusters_across_wrapped_lines; no installed candidate font covers the Hindi input");
        return;
    };
    let style = CharStyle { font_family: face.family.clone(), font_style: face.style.clone(), size: 20.0, ..Default::default() };
    let (mut d, id, mut tool) = editing(text);
    let NodeKind::Text(t) = &mut d.node_mut(id).unwrap().kind else { panic!("text fixture") };
    for run in &mut t.runs {
        run.style = style.clone();
    }
    t.para.justify = Justify::Left;
    t.para.composer = Composer::SingleLine;
    t.para.hyphenate = false;
    assert_eq!(t.plain_text(), text);
    assert_eq!(&text[..hindi.len()], hindi);

    // Measure once on the selected face. No width sweep or invented line endpoints.
    let natural = vectorcraft_text::layout(db, t);
    assert_eq!(natural.lines.len(), 1, "setup: natural point text is a single line");
    assert!(!natural.vertical, "setup: horizontal text");
    let hindi_glyphs: Vec<_> = natural.glyphs.iter().filter(|g| g.byte < hindi.len()).collect();
    assert!(!hindi_glyphs.is_empty(), "setup: actual Hindi glyphs exist");
    assert_unwrapped_hindi_glyphs(db, &face, &hindi_glyphs, hindi, text);
    let prefix_width: f64 = hindi_glyphs.iter().filter(|g| g.byte < split).map(|g| g.advance).sum();
    let split_width: f64 = hindi_glyphs.iter().filter(|g| g.byte == split).map(|g| g.advance).sum();
    assert!(prefix_width.is_finite() && prefix_width > 0.0, "setup: prefix has positive finite advance");
    assert!(split_width.is_finite(), "setup: the internal raw-group advance is finite");
    if split_width <= 0.0 {
        eprintln!(
            "SKIPPED: type_arrows_keep_devanagari_clusters_across_wrapped_lines; layout capability: the selected face has no positive-advance raw group at internal byte {split}"
        );
        return;
    }
    let width = prefix_width + split_width * 0.5;
    assert!(width.is_finite() && width > prefix_width && width < prefix_width + split_width, "setup: one measured width separates the raw groups");

    t.kind = TextKind::Area { frame: PathData::from_bezpath(&Rect::new(0.0, 0.0, width, 500.0).to_path(0.1)) };
    t.xf = Affine::IDENTITY;
    assert_eq!((t.area.rows, t.area.columns, t.area.inset), (1, 1, 0.0));
    assert_eq!((t.para.left_indent, t.para.first_line_indent, t.para.right_indent), (0.0, 0.0, 0.0));
    let lay = vectorcraft_text::layout(db, t);
    assert!(!lay.overflow, "setup: the full RTL suffix fits the tall frame");
    assert!(!lay.vertical, "setup: horizontal area text");
    assert!(lay.glyphs.iter().any(|g| g.rtl && g.byte > hindi.len()), "setup: actual RTL suffix selects Type's visual-arrow route");
    assert_wrapped_hindi_glyphs(db, &face, &lay, hindi);
    assert_eq!(edit::next_cluster(text, second), hindi.len(), "setup: logical cluster end is 18");
    assert_eq!(edit::prev_cluster(text, hindi.len()), second, "setup: logical cluster start is 6");
    assert_eq!(edit::prev_cluster(text, split), second, "setup: raw line endpoint 12 is inside the grapheme");
    assert_eq!(edit::next_cluster(text, split), hindi.len(), "setup: raw line endpoint 12 is not its end");
    if lay.lines.len() < 2 || (lay.lines[0].start, lay.lines[0].end, lay.lines[1].start) != (0, split, split) {
        eprintln!(
            "SKIPPED: type_arrows_keep_devanagari_clusters_across_wrapped_lines; layout capability: the measured frame does not expose adjacent wrapped endpoints at internal byte {split}"
        );
        return;
    }
    assert!(lay.lines.len() >= 2, "setup: actual wrapping occurred");
    assert_eq!(
        (lay.lines[0].start, lay.lines[0].end, lay.lines[1].start),
        (0, split, split),
        "setup: actual adjacent line endpoints reach internal byte 12"
    );
    assert_eq!(lay.line_of(second), 0, "setup: caret 6 is on the preceding line");
    assert!(!lay.lines[0].rtl, "setup: the preceding paragraph line is LTR");
    tool.caret = second;
    tool.anchor = second;
    assert_eq!(tool.editing, Some(id));
    assert_eq!(tool.sel(), (second, second), "setup: collapsed caret before Right");
    assert!(tool.typing.is_none(), "setup: no pending typing session");
    key(&mut tool, &d, ToolKey::Right, Mods::default());
    assert_eq!((tool.caret, tool.anchor), (hindi.len(), hindi.len()), "wrapped Right must move 6 -> 18 across the Hindi grapheme");
    key(&mut tool, &d, ToolKey::Left, Mods::default());
    assert_eq!((tool.caret, tool.anchor), (second, second), "wrapped Left must move 18 -> 6 across the Hindi grapheme");
}

#[test]
fn typing_is_one_coalesced_preview() {
    let (mut d, id, mut tool) = editing("Hello");
    let snap = d.clone();
    let (sel, p) = (Selection::default(), paint());
    tool.caret = 5;
    tool.anchor = 5;
    let a1 = tool.text_input(&cx(&d, &sel, &p), " w");
    assert_eq!(a1[0], Action::Begin("Typing".into()));
    apply(&mut d, &snap, &a1);
    let a2 = tool.text_input(&cx(&d, &sel, &p), "orld");
    apply(&mut d, &snap, &a2);
    let NodeKind::Text(t) = &d.node(id).unwrap().kind else { panic!() };
    assert_eq!(t.plain_text(), "Hello world");
    assert!(!a2.iter().any(|a| matches!(a, Action::Begin(_))), "same session");
    let Action::Preview(cmd, params) = a2.last().unwrap() else { panic!("{a2:?}") };
    assert_eq!(cmd, "text.editRange");
    assert_eq!(params["id"], json!(id.0));
    assert_eq!((params["start"].as_u64(), params["end"].as_u64()), (Some(5), Some(5)));
    let runs: Vec<TextRun> = serde_json::from_value(params["runs"].clone()).unwrap();
    assert_eq!(runs[0].text, " world");
    assert_eq!(tool.caret, 11);
    // Moving the caret ends the session.
    let a3 = key(&mut tool, &d, ToolKey::Left, Mods::default());
    assert_eq!(a3, vec![Action::Commit]);
    assert_eq!(tool.caret, 10);
    // A document change behind the tool's back (undo) starts a fresh session.
    let a4 = tool.text_input(&cx(&snap, &sel, &p), "!");
    assert!(a4.contains(&Action::Begin("Typing".into())));
}

/// What the engine does around Edit › Undo or Redo (`what`) that puts the document back to `to`:
/// it ends the typing session, runs the step, tells the tool, then lets it finish the command.
fn history_step(tool: &mut TypeTool, to: &Document, id: NodeId, what: &str) {
    let (mut sel, p) = (Selection::default(), paint());
    sel.set([id]);
    tool.set_option("endTyping", &json!(true));
    tool.notify(&cx(to, &sel, &p), what);
    assert_eq!(tool.after_command(&cx(to, &sel, &p)), vec![]);
}

fn type_in(tool: &mut TypeTool, d: &mut Document, s: &str) {
    let (sel, p) = (Selection::default(), paint());
    let snap = d.clone();
    let acts = tool.text_input(&cx(d, &sel, &p), s);
    apply(d, &snap, &acts);
}

fn plain_of(d: &Document, id: NodeId) -> String {
    let NodeKind::Text(t) = &d.node(id).unwrap().kind else { panic!("text") };
    t.plain_text()
}

/// Undo of typing puts the caret where the typed text began, and Redo puts it after the text.
#[test]
fn undo_and_redo_of_typing_put_the_caret_where_the_text_changed() {
    let (mut d, id, mut tool) = editing("XYZ");
    let before = d.clone();
    type_in(&mut tool, &mut d, "abc");
    assert_eq!((plain_of(&d, id).as_str(), tool.caret), ("abcXYZ", 3));
    let after = d.clone();
    history_step(&mut tool, &before, id, "edit.undo");
    assert_eq!((tool.caret, tool.anchor), (0, 0));
    history_step(&mut tool, &after, id, "edit.redo");
    assert_eq!((tool.caret, tool.anchor), (3, 3));
    // Typing in the middle of the text: Undo goes back to where the typing began.
    tool.caret = 4;
    tool.anchor = 4;
    type_in(&mut tool, &mut d, "-");
    assert_eq!(plain_of(&d, id), "abcX-YZ");
    history_step(&mut tool, &after, id, "edit.undo");
    assert_eq!((tool.caret, tool.anchor), (4, 4));
}

/// Undo of a deletion puts the caret after the text it puts back, and Redo where it was taken out.
#[test]
fn undo_of_a_deletion_puts_the_caret_after_the_restored_text() {
    let (mut d, id, mut tool) = editing("Hello big world");
    let before = d.clone();
    let snap = d.clone();
    tool.caret = 10;
    tool.anchor = 10;
    let acts = key(&mut tool, &d, ToolKey::Backspace, Mods { alt: true, ..Default::default() });
    apply(&mut d, &snap, &acts);
    assert_eq!((plain_of(&d, id).as_str(), tool.caret), ("Hello world", 6));
    let after = d.clone();
    // The caret moved away before the Undo.
    key(&mut tool, &d, ToolKey::End, Mods::default());
    history_step(&mut tool, &before, id, "edit.undo");
    assert_eq!((tool.caret, tool.anchor), (10, 10));
    history_step(&mut tool, &after, id, "edit.redo");
    assert_eq!((tool.caret, tool.anchor), (6, 6));
}

/// A step that changes no characters (a formatting change) leaves the caret and the selection, and
/// the Undo of the typing before it still puts the caret where the typing began.
#[test]
fn undo_of_formatting_leaves_the_caret_and_selection() {
    let (mut d, id, mut tool) = editing("Hello world");
    let before = d.clone();
    tool.caret = 5;
    tool.anchor = 5;
    type_in(&mut tool, &mut d, ",");
    let typed = d.clone();
    // A panel makes the typed text larger (a command from outside the tool).
    let mut large = d.clone();
    let NodeKind::Text(t) = &mut large.node_mut(id).unwrap().kind else { panic!("text") };
    edit::style_range(&mut t.runs, 0, 6, |s| s.size = 30.0);
    let (mut sel, p) = (Selection::default(), paint());
    sel.set([id]);
    tool.set_option("endTyping", &json!(true));
    tool.after_command(&cx(&large, &sel, &p));
    tool.anchor = 2;
    tool.caret = 8;
    history_step(&mut tool, &typed, id, "edit.undo");
    assert_eq!((tool.anchor, tool.caret), (2, 8));
    history_step(&mut tool, &before, id, "edit.undo");
    assert_eq!((tool.anchor, tool.caret), (5, 5));
}

/// The caret lands on character boundaries in multi-byte text.
#[test]
fn undo_and_redo_keep_the_caret_on_character_boundaries() {
    let (mut d, id, mut tool) = editing("café 🙂 ok");
    let before = d.clone();
    tool.caret = "café 🙂".len();
    tool.anchor = tool.caret;
    type_in(&mut tool, &mut d, "é🙂");
    assert_eq!(plain_of(&d, id), "café 🙂é🙂 ok");
    let after = d.clone();
    history_step(&mut tool, &before, id, "edit.undo");
    assert_eq!(tool.caret, "café 🙂".len());
    history_step(&mut tool, &after, id, "edit.redo");
    assert_eq!(tool.caret, "café 🙂é🙂".len());
}

/// Typing or deleting the same characters as the text next to it: Undo and Redo put the caret where
/// the tool made the change, not at the other end of the repeated characters.
#[test]
fn undo_and_redo_of_repeated_characters_find_the_tools_own_edit() {
    for (text, at, typed, undone, redone) in [("aaa", 0, "a", 0, 1), ("éé", 2, "é", 2, 4), ("ab ab", 3, "ab ", 3, 6)] {
        let (mut d, id, mut tool) = editing(text);
        let before = d.clone();
        tool.caret = at;
        tool.anchor = at;
        type_in(&mut tool, &mut d, typed);
        let after = d.clone();
        history_step(&mut tool, &before, id, "edit.undo");
        assert_eq!((tool.caret, tool.anchor), (undone, undone), "{text}: undo");
        history_step(&mut tool, &after, id, "edit.redo");
        assert_eq!((tool.caret, tool.anchor), (redone, redone), "{text}: redo");
    }
    // A deleted character next to the same character comes back with the caret after it.
    let (mut d, id, mut tool) = editing("aaa");
    let before = d.clone();
    tool.caret = 1;
    tool.anchor = 1;
    let snap = d.clone();
    let acts = key(&mut tool, &d, ToolKey::Backspace, Mods::default());
    apply(&mut d, &snap, &acts);
    assert_eq!(plain_of(&d, id), "aa");
    history_step(&mut tool, &before, id, "edit.undo");
    assert_eq!(tool.caret, 1);
    // Two steps whose texts are each other's: each Undo and Redo finds its own step.
    let (mut d, id, mut tool) = editing("aaa");
    let first = d.clone();
    type_in(&mut tool, &mut d, "a");
    let second = d.clone();
    key(&mut tool, &d, ToolKey::End, Mods::default());
    let snap = d.clone();
    let acts = key(&mut tool, &d, ToolKey::Backspace, Mods::default());
    apply(&mut d, &snap, &acts);
    let third = d.clone();
    assert_eq!(plain_of(&third, id), "aaa");
    for (to, what, at) in [(&second, "edit.undo", 4), (&first, "edit.undo", 0), (&second, "edit.redo", 1), (&third, "edit.redo", 3)] {
        history_step(&mut tool, to, id, what);
        assert_eq!(tool.caret, at, "{what} to {}", plain_of(to, id));
    }
    // A step from outside the tool after an Undo drops the undone step from Redo: Redo of the
    // outside step does not take the dropped step's place.
    let (mut d, id, mut tool) = editing("aaa");
    let first = d.clone();
    type_in(&mut tool, &mut d, "a");
    history_step(&mut tool, &first, id, "edit.undo");
    assert_eq!(tool.caret, 0);
    let mut appended = first.clone();
    let NodeKind::Text(t) = &mut appended.node_mut(id).unwrap().kind else { panic!("text") };
    edit::replace_range(&mut t.runs, 3, 3, "a");
    let (mut sel, p) = (Selection::default(), paint());
    sel.set([id]);
    assert_eq!(tool.after_command(&cx(&appended, &sel, &p)), vec![]);
    history_step(&mut tool, &first, id, "edit.undo");
    history_step(&mut tool, &appended, id, "edit.redo");
    assert_eq!(tool.caret, 4);
    // A command from outside the tool that leaves the text as it is (a query) keeps the undone
    // step for Redo.
    let (mut d, id, mut tool) = editing("aaa");
    let first = d.clone();
    type_in(&mut tool, &mut d, "a");
    let second = d.clone();
    history_step(&mut tool, &first, id, "edit.undo");
    let (mut sel, p) = (Selection::default(), paint());
    sel.set([id]);
    assert_eq!(tool.after_command(&cx(&first, &sel, &p)), vec![]);
    history_step(&mut tool, &second, id, "edit.redo");
    assert_eq!(tool.caret, 1);
    // Commands from outside the tool that repeat the change of a step the tool made are steps of
    // their own: Undo of the last one does not take back the tool's step.
    let (mut d, id, mut tool) = editing("aaa");
    let first = d.clone();
    type_in(&mut tool, &mut d, "a");
    let typed = d.clone();
    let (mut sel, p) = (Selection::default(), paint());
    sel.set([id]);
    tool.set_option("endTyping", &json!(true));
    let mut removed = typed.clone();
    let NodeKind::Text(t) = &mut removed.node_mut(id).unwrap().kind else { panic!("text") };
    edit::replace_range(&mut t.runs, 3, 4, "");
    assert_eq!(tool.after_command(&cx(&removed, &sel, &p)), vec![]);
    let mut appended = removed.clone();
    let NodeKind::Text(t) = &mut appended.node_mut(id).unwrap().kind else { panic!("text") };
    edit::replace_range(&mut t.runs, 3, 3, "a");
    assert_eq!(tool.after_command(&cx(&appended, &sel, &p)), vec![]);
    for (to, what, at) in [(&removed, "edit.undo", 3), (&typed, "edit.undo", 4), (&first, "edit.undo", 0), (&typed, "edit.redo", 1)] {
        history_step(&mut tool, to, id, what);
        assert_eq!(tool.caret, at, "{what} to {}", plain_of(to, id));
    }
    // A typing session that leaves the text as it was (a character typed and deleted again) does
    // not stop Undo of the step before it from finding where that step typed.
    let (mut d, id, mut tool) = editing("aaa");
    let first = d.clone();
    type_in(&mut tool, &mut d, "a");
    key(&mut tool, &d, ToolKey::End, Mods::default());
    // The session's previews each replace the text it started from.
    let session = d.clone();
    type_in(&mut tool, &mut d, "b");
    let acts = key(&mut tool, &d, ToolKey::Backspace, Mods::default());
    apply(&mut d, &session, &acts);
    assert_eq!(plain_of(&d, id), "aaaa");
    history_step(&mut tool, &d, id, "edit.undo");
    history_step(&mut tool, &first, id, "edit.undo");
    assert_eq!(tool.caret, 0);
}

#[test]
fn shift_arrows_select_and_typing_replaces() {
    let (d, _, mut tool) = editing("Hello world");
    key(&mut tool, &d, ToolKey::End, Mods::default());
    assert_eq!(tool.caret, 11);
    key(&mut tool, &d, ToolKey::Left, Mods { alt: true, ..SHIFT });
    assert_eq!(tool.sel(), (6, 11));
    let (sel, p) = (Selection::default(), paint());
    let acts = tool.text_input(&cx(&d, &sel, &p), "there");
    let Action::Preview(_, params) = acts.last().unwrap() else { panic!() };
    assert_eq!((params["start"].as_u64(), params["end"].as_u64()), (Some(6), Some(11)));
    assert_eq!(tool.caret, 11);
    assert_eq!(tool.sel(), (11, 11));
}

#[test]
fn word_and_line_navigation_keys() {
    let (d, _, mut tool) = editing("One two three");
    key(&mut tool, &d, ToolKey::Right, CMD);
    assert_eq!(tool.caret, 3);
    key(&mut tool, &d, ToolKey::Right, CMD);
    assert_eq!(tool.caret, 7);
    key(&mut tool, &d, ToolKey::Left, CMD);
    assert_eq!(tool.caret, 4);
    key(&mut tool, &d, ToolKey::End, SHIFT);
    assert_eq!(tool.sel(), (4, 13));
    // Left collapses a selection to its start.
    key(&mut tool, &d, ToolKey::Left, Mods::default());
    assert_eq!(tool.sel(), (4, 4));
    key(&mut tool, &d, ToolKey::Home, Mods::default());
    assert_eq!(tool.caret, 0);
    key(&mut tool, &d, ToolKey::End, CMD);
    assert_eq!(tool.caret, 13);
}

#[test]
fn up_down_move_between_lines() {
    let (d, _, mut tool) = editing("First line\nSecond line");
    tool.caret = 3;
    tool.anchor = 3;
    key(&mut tool, &d, ToolKey::Down, Mods::default());
    assert!(tool.caret > 11 && tool.caret < 16, "{}", tool.caret);
    key(&mut tool, &d, ToolKey::Up, Mods::default());
    assert_eq!(tool.caret, 3, "goal column kept");
    key(&mut tool, &d, ToolKey::Down, SHIFT);
    key(&mut tool, &d, ToolKey::Down, SHIFT);
    assert_eq!(tool.sel(), (3, 22));
}

#[test]
fn backspace_deletes_selection_or_word() {
    let (d, _, mut tool) = editing("Hello big world");
    tool.caret = 9;
    tool.anchor = 9;
    let acts = key(&mut tool, &d, ToolKey::Backspace, Mods { alt: true, ..Default::default() });
    let Action::Preview(_, params) = acts.last().unwrap() else { panic!() };
    assert_eq!((params["start"].as_u64(), params["end"].as_u64()), (Some(6), Some(9)));
    assert_eq!(tool.caret, 6);
}

#[test]
fn double_and_triple_click_select_word_and_paragraph() {
    let (d, id, mut tool) = editing("Alpha beta\ngamma");
    let (sel, p) = (Selection::default(), paint());
    let t = TypeTool::text(&cx(&d, &sel, &p), id).unwrap().clone();
    let lay = vectorcraft_text::layout(FontDb::global(), &t);
    let (top, bot) = vectorcraft_text::caret_position(&lay, 7);
    let at = t.xf * top.midpoint(bot);
    let c = cx(&d, &sel, &p);
    for k in [PointerKind::Down, PointerKind::Up, PointerKind::Down, PointerKind::Up, PointerKind::DoubleClick] {
        tool.pointer(&c, &PointerEvent::new(k, at.x, at.y));
    }
    assert_eq!(tool.sel(), (6, 10), "word");
    tool.pointer(&c, &PointerEvent::new(PointerKind::Down, at.x, at.y));
    tool.pointer(&c, &PointerEvent::new(PointerKind::Up, at.x, at.y));
    assert_eq!(tool.sel(), (0, 10), "paragraph");
}

#[test]
fn drag_selects_and_overlay_highlights() {
    let (d, id, mut tool) = editing("Select me please");
    let (sel, p) = (Selection::default(), paint());
    let c = cx(&d, &sel, &p);
    let t = TypeTool::text(&c, id).unwrap().clone();
    let lay = vectorcraft_text::layout(FontDb::global(), &t);
    let pt = |b: usize| {
        let (a, bb) = vectorcraft_text::caret_position(&lay, b);
        t.xf * a.midpoint(bb)
    };
    let (a, b) = (pt(7), pt(9));
    tool.pointer(&c, &PointerEvent::new(PointerKind::Down, a.x + 0.1, a.y));
    assert!(tool.busy());
    tool.pointer(&c, &PointerEvent::new(PointerKind::Drag, b.x + 0.1, b.y));
    tool.pointer(&c, &PointerEvent::new(PointerKind::Up, b.x + 0.1, b.y));
    assert_eq!(tool.sel(), (7, 9));
    let o = tool.overlays(&c);
    assert!(o.iter().any(|o| matches!(o, Overlay::Highlight { .. })));
    let opts = tool.options();
    assert_eq!((opts["start"].as_u64(), opts["end"].as_u64()), (Some(7), Some(9)));
}

#[test]
fn a_drag_across_type_not_being_edited_selects_its_text() {
    let (d, id) = doc_with_text("Select me please");
    let mut tool = TypeTool::new("type");
    let (sel, p) = (Selection::default(), paint());
    let c = cx(&d, &sel, &p);
    let t = TypeTool::text(&c, id).unwrap().clone();
    let lay = vectorcraft_text::layout(FontDb::global(), &t);
    let pt = |b: usize| {
        let (a, bb) = vectorcraft_text::caret_position(&lay, b);
        t.xf * a.midpoint(bb)
    };
    let (a, b) = (pt(7), pt(9));
    // The press starts editing the type and the drag selects, in one gesture.
    let out = tool.pointer(&c, &PointerEvent::new(PointerKind::Down, a.x + 0.1, a.y));
    assert!(out.contains(&Action::Exec("select.set".into(), json!({"ids": [id.0]}))));
    tool.pointer(&c, &PointerEvent::new(PointerKind::Drag, b.x + 0.1, b.y));
    tool.pointer(&c, &PointerEvent::new(PointerKind::Up, b.x + 0.1, b.y));
    assert_eq!(tool.editing, Some(id));
    assert_eq!(tool.sel(), (7, 9));
    // A plain click still just places the caret.
    let mut tool = TypeTool::new("type");
    tool.pointer(&c, &PointerEvent::new(PointerKind::Down, a.x + 0.1, a.y));
    tool.pointer(&c, &PointerEvent::new(PointerKind::Up, a.x + 0.1, a.y));
    assert_eq!((tool.editing, tool.sel()), (Some(id), (7, 7)));
}

#[test]
fn select_all_and_styled_paste() {
    let (d, _, mut tool) = editing("abc");
    tool.set_option("selectAll", &json!(true));
    let (sel, p) = (Selection::default(), paint());
    let c = cx(&d, &sel, &p);
    tool.key(&c, ToolKey::Right, SHIFT);
    assert_eq!(tool.sel(), (0, 3));
    let big = TextRun { text: "XY".into(), style: CharStyle { size: 40.0, ..Default::default() }, inline: None };
    tool.set_option("copy", &serde_json::to_value(vec![big.clone()]).unwrap());
    let acts = tool.text_input(&c, "XY");
    let Action::Preview(_, params) = acts.last().unwrap() else { panic!() };
    let runs: Vec<TextRun> = serde_json::from_value(params["runs"].clone()).unwrap();
    assert_eq!(runs, vec![big]);
}

#[test]
fn common_affixes_are_char_safe() {
    assert_eq!(common_affixes("hello", "help", usize::MAX), (3, 0));
    assert_eq!(common_affixes("aXb", "ab", usize::MAX), (1, 1));
    assert_eq!(common_affixes("aaa", "aaaa", usize::MAX), (3, 0));
    assert_eq!(common_affixes("é1", "è1", usize::MAX), (0, 1));
    // Return at the end of a paragraph: the span starts where the edit did (before the old break).
    assert_eq!(common_affixes("abc\ndef", "abc\n\ndef", 3), (3, 4));
    assert_eq!(common_affixes("abc\ndef", "abc\n\ndef", 4), (4, 3));
}

#[test]
fn area_and_path_tools_convert_clicked_paths() {
    let (d, rect) = crate::testutil::doc_with_rect();
    let (sel, p) = (Selection::default(), paint());
    let c = cx(&d, &sel, &p);
    let mut tool = TypeTool::new("areaType");
    assert_eq!(tool.id(), "areaType");
    tool.pointer(&c, &PointerEvent::new(PointerKind::Down, 100.0, 150.0));
    let acts = tool.pointer(&c, &PointerEvent::new(PointerKind::Up, 100.0, 150.0));
    assert!(
        acts.iter().any(|a| matches!(a, Action::Exec(cmd, p) if cmd == "text.createInPath" && p["path"] == json!(rect.0) && p["mode"] == "area")),
        "{acts:?}"
    );
    let mut tool = TypeTool::new("typeOnPath");
    tool.pointer(&c, &PointerEvent::new(PointerKind::Down, 150.0, 100.0));
    let acts = tool.pointer(&c, &PointerEvent::new(PointerKind::Up, 150.0, 100.0));
    assert!(acts.iter().any(|a| matches!(a, Action::Exec(cmd, p) if cmd == "text.createInPath" && p["mode"] == "onPath")), "{acts:?}");
}

/// Area Type and Type on a Path follow the reference app: click the path outline, not the fill interior
/// (helpx: add-text-work-with-type-objects, creating-type-path).
#[test]
fn area_and_path_tools_hit_path_outline_not_fill_interior() {
    let (mut d, rect) = crate::testutil::doc_with_rect();
    if let Some(n) = d.node_mut(rect) {
        n.appearance.set_stroke(vectorcraft_color::Paint::None);
    }
    let (sel, p) = (Selection::default(), paint());
    let c = cx(&d, &sel, &p);
    let edge = (100.0, 150.0);
    let interior = (150.0, 150.0);
    for tool_id in ["areaType", "typeOnPath"] {
        let mut tool = TypeTool::new(tool_id);
        tool.pointer(&c, &PointerEvent::new(PointerKind::Down, edge.0, edge.1));
        let acts = tool.pointer(&c, &PointerEvent::new(PointerKind::Up, edge.0, edge.1));
        assert!(
            acts.iter().any(|a| matches!(a, Action::Exec(cmd, v) if cmd == "text.createInPath" && v["path"] == json!(rect.0))),
            "{tool_id} on outline: {acts:?}"
        );
        let mut tool = TypeTool::new(tool_id);
        tool.pointer(&c, &PointerEvent::new(PointerKind::Down, interior.0, interior.1));
        let acts = tool.pointer(&c, &PointerEvent::new(PointerKind::Up, interior.0, interior.1));
        assert!(
            !acts.iter().any(|a| matches!(a, Action::Exec(cmd, _) if cmd == "text.createInPath")),
            "{tool_id} in fill interior should not attach to path: {acts:?}"
        );
    }
}

#[test]
fn vertical_tools_preserve_their_ids_and_create_vertical_text() {
    for id in ["verticalType", "verticalAreaType", "verticalTypeOnPath"] {
        let tool = TypeTool::new(id);
        assert_eq!(tool.id(), id);
        assert!(tool.vertical);
    }
}

/// A press from `tool` at `from` with `down` held, dragged to `to` (none for a click) and released
/// with `up` held → the actions of the release.
fn gesture(tool: &mut TypeTool, c: &ToolContext, from: (f64, f64), to: Option<(f64, f64)>, down: Mods, up: Mods) -> Vec<Action> {
    tool.pointer(c, &PointerEvent::new(PointerKind::Down, from.0, from.1).with_mods(down));
    let end = to.unwrap_or(from);
    if to.is_some() {
        tool.pointer(c, &PointerEvent::new(PointerKind::Drag, end.0, end.1).with_mods(down));
    }
    tool.pointer(c, &PointerEvent::new(PointerKind::Up, end.0, end.1).with_mods(up))
}

/// The command that makes new type among `acts`, with its params.
fn made(acts: &[Action]) -> Option<(&str, &Value)> {
    acts.iter().find_map(|a| match a {
        Action::Exec(cmd, p) if cmd == "text.create" || cmd == "text.createInPath" => Some((cmd.as_str(), p)),
        _ => None,
    })
}

/// [`crate::testutil::doc_with_rect`] plus an open path from (100, 400) to (400, 400).
fn doc_with_rect_and_line() -> (Document, NodeId, NodeId) {
    let (mut d, rect) = crate::testutil::doc_with_rect();
    let l = d.layers[0].id;
    let line = d.alloc_id();
    let path = vectorcraft_geom::PathData::from_bezpath(&BezPath::from_vec(vec![
        vectorcraft_geom::PathEl::MoveTo(Point::new(100.0, 400.0)),
        vectorcraft_geom::PathEl::LineTo(Point::new(400.0, 400.0)),
    ]));
    d.insert(Some(l), 1, Node::path(line, path, vectorcraft_doc::Appearance::default_art())).unwrap();
    (d, rect, line)
}

/// Shift held at the press makes the new type as the horizontal or vertical counterpart of each
/// Type tool would, for a click and for a drag; the keys at the release do not matter, and the
/// next press without Shift acts as the tool chosen.
#[test]
fn shift_switches_between_the_horizontal_and_the_vertical_type_tool() {
    let (d, rect) = crate::testutil::doc_with_rect();
    let (sel, p) = (Selection::default(), paint());
    let c = cx(&d, &sel, &p);
    for (id, at, cmd) in [
        ("type", (400.0, 400.0), "text.create"),
        ("verticalType", (400.0, 400.0), "text.create"),
        ("areaType", (100.0, 150.0), "text.createInPath"),
        ("verticalAreaType", (100.0, 150.0), "text.createInPath"),
        ("typeOnPath", (150.0, 100.0), "text.createInPath"),
        ("verticalTypeOnPath", (150.0, 100.0), "text.createInPath"),
    ] {
        let vertical = id.starts_with("vertical");
        let mut tool = TypeTool::new(id);
        let acts = gesture(&mut tool, &c, at, None, SHIFT, Mods::default());
        let (made_by, params) = made(&acts).unwrap_or_else(|| panic!("{id}: {acts:?}"));
        assert_eq!(made_by, cmd, "{id}");
        assert_eq!(params["vertical"], json!(!vertical), "{id} with Shift");
        if cmd == "text.createInPath" {
            assert_eq!(params["path"], json!(rect.0), "{id}");
            assert_eq!(params["mode"], if id.contains("OnPath") { "onPath" } else { "area" }, "{id}");
        }
        assert_eq!(tool.id(), id, "the tool chosen stays");
        let acts = gesture(&mut tool, &c, at, None, Mods::default(), SHIFT);
        assert_eq!(made(&acts).map(|(_, p)| p["vertical"].clone()), Some(json!(vertical)), "{id} without Shift at the press");
    }
    // A frame dragged out with Shift held.
    let mut tool = TypeTool::new("type");
    let acts = gesture(&mut tool, &c, (300.0, 300.0), Some((400.0, 400.0)), SHIFT, Mods::default());
    let (cmd, params) = made(&acts).unwrap_or_else(|| panic!("{acts:?}"));
    assert_eq!((cmd, &params["area"], &params["vertical"]), ("text.create", &json!({"width": 100.0, "height": 100.0}), &json!(true)));
}

/// Alt held at the press makes the new type as Type on a Path would with the Area Type tools and
/// as Area Type would with the Type on a Path tools, keeping their writing direction; with Shift
/// too, the direction switches as well. The Type tools have no counterpart for Alt.
#[test]
fn alt_switches_between_area_type_and_type_on_a_path() {
    let (d, rect, line) = doc_with_rect_and_line();
    let (sel, p) = (Selection::default(), paint());
    let c = cx(&d, &sel, &p);
    let (on_line, on_rect) = ((250.0, 400.0), (100.0, 150.0));
    for (id, at, mods, path, mode, vertical) in [
        ("areaType", on_line, ALT, line, "onPath", false),
        ("verticalAreaType", on_line, ALT, line, "onPath", true),
        ("typeOnPath", on_rect, ALT, rect, "area", false),
        ("verticalTypeOnPath", on_rect, ALT, rect, "area", true),
        ("areaType", on_line, SHIFT_ALT, line, "onPath", true),
        ("verticalTypeOnPath", on_rect, SHIFT_ALT, rect, "area", false),
    ] {
        let mut tool = TypeTool::new(id);
        let acts = gesture(&mut tool, &c, at, None, mods, Mods::default());
        let (cmd, params) = made(&acts).unwrap_or_else(|| panic!("{id} {mods:?}: {acts:?}"));
        assert_eq!(cmd, "text.createInPath", "{id} {mods:?}");
        assert_eq!((&params["path"], &params["mode"], &params["vertical"]), (&json!(path.0), &json!(mode), &json!(vertical)), "{id} {mods:?}");
        assert_eq!(tool.id(), id);
    }
    // An open path is no frame for the Area Type tool without Alt, and Type on a Path keeps the
    // rectangle as a baseline.
    let acts = gesture(&mut TypeTool::new("areaType"), &c, on_line, None, Mods::default(), Mods::default());
    assert_eq!(made(&acts).map(|(cmd, _)| cmd), Some("text.create"), "{acts:?}");
    let acts = gesture(&mut TypeTool::new("typeOnPath"), &c, on_rect, None, Mods::default(), Mods::default());
    assert_eq!(made(&acts).map(|(_, p)| p["mode"].clone()), Some(json!("onPath")), "{acts:?}");
    for id in ["type", "verticalType"] {
        let acts = gesture(&mut TypeTool::new(id), &c, (450.0, 450.0), None, ALT, Mods::default());
        let (cmd, params) = made(&acts).unwrap_or_else(|| panic!("{id}: {acts:?}"));
        assert_eq!((cmd, &params["vertical"]), ("text.create", &json!(id == "verticalType")), "{id} with Alt");
    }
}

/// The pointer shows the Type tool that a press would act as with the keys held, and while a press
/// is held, the one it acts as.
#[test]
fn the_pointer_shows_the_type_tool_shift_or_alt_switches_to() {
    let (d, _, _) = doc_with_rect_and_line();
    let (sel, p) = (Selection::default(), paint());
    let c = cx(&d, &sel, &p);
    let (away, on_line) = (Point::new(450.0, 450.0), Point::new(250.0, 400.0));
    let cursor = |id: &str, at: Point, mods: Mods| TypeTool::new(id).cursor(&c, at, mods);
    assert_eq!(cursor("type", away, SHIFT), Cursor::TypeVertical);
    assert_eq!(cursor("verticalType", away, SHIFT), Cursor::Text);
    assert_eq!(cursor("verticalAreaType", away, SHIFT), Cursor::Text);
    assert_eq!(cursor("areaType", on_line, Mods::default()), Cursor::Text, "no frame for area type");
    assert_eq!(cursor("areaType", on_line, ALT), Cursor::Crosshair, "a baseline for type on a path");
    assert_eq!(cursor("typeOnPath", on_line, ALT), Cursor::Text);
    let mut tool = TypeTool::new("type");
    tool.pointer(&c, &PointerEvent::new(PointerKind::Down, away.x, away.y).with_mods(SHIFT));
    assert_eq!(tool.cursor(&c, away, Mods::default()), Cursor::TypeVertical, "the press keeps the tool it acts as");
    tool.pointer(&c, &PointerEvent::new(PointerKind::Up, away.x, away.y));
    assert_eq!(tool.cursor(&c, away, Mods::default()), Cursor::Text);
}

/// While type is being edited, Shift and Alt switch no tool: a frame dragged out is the tool
/// chosen's, and the pointer stays its own.
#[test]
fn shift_and_alt_switch_no_tool_while_type_is_edited() {
    let (d, _, mut tool) = editing("Hello");
    let (sel, p) = (Selection::default(), paint());
    let c = cx(&d, &sel, &p);
    assert_eq!(tool.cursor(&c, Point::new(400.0, 400.0), SHIFT), Cursor::Text);
    let acts = gesture(&mut tool, &c, (300.0, 300.0), Some((400.0, 400.0)), SHIFT_ALT, SHIFT_ALT);
    let (cmd, params) = made(&acts).unwrap_or_else(|| panic!("{acts:?}"));
    assert_eq!((cmd, &params["vertical"]), ("text.create", &json!(false)));
}

#[test]
fn ime_ranges_convert_characters_to_bytes() {
    assert_eq!(char_range_to_bytes("ががく", 1..3), Some(3..9));
    assert_eq!(char_range_to_bytes("ががく", 3..3), Some(9..9));
    assert_eq!(char_range_to_bytes("a雅b", 1..2), Some(1..4));
    assert_eq!(char_range_to_bytes("ががく", 2..4), None, "past the end");
    #[allow(clippy::reversed_empty_ranges)]
    let backwards = 2..1;
    assert_eq!(char_range_to_bytes("ががく", backwards), None, "backwards");
}

/// Outgoing surrounding text excludes the pre-edit and remaps a selection that crosses it, not
/// only a caret at its end.
#[test]
fn ime_surrounding_remaps_a_selection_across_the_preedit() {
    let (mut d, _id, mut tool) = editing("abcd");
    let snap = d.clone();
    let (sel, p) = (Selection::default(), paint());
    tool.caret = 2;
    tool.anchor = 2;
    let acts = tool.ime_preedit(&cx(&d, &sel, &p), "xy", None);
    apply(&mut d, &snap, &acts);
    assert_eq!(tool.preedit.as_ref().map(|p| p.range.clone()), Some(2..4));
    // A selection from before the marked text to after it: the IME receives the committed text
    // with both offsets folded across the removed pre-edit.
    tool.anchor = 1;
    tool.caret = 6;
    let ImeSurrounding::Available { text, cursor, anchor } = tool.ime_surrounding(&cx(&d, &sel, &p), 4000) else {
        panic!("the pre-edit has committed text around it");
    };
    assert_eq!(text, "abcd");
    assert_eq!((cursor, anchor), (4, 1));
}

#[test]
fn marked_text_is_underlined_with_the_converting_clause_thick() {
    let (mut d, id, mut tool) = editing("曲");
    let snap = d.clone();
    let (sel, p) = (Selection::default(), paint());
    tool.caret = 3;
    tool.anchor = 3;
    let acts = tool.ime_preedit(&cx(&d, &sel, &p), "雅楽演奏", Some(2..4));
    apply(&mut d, &snap, &acts);
    let NodeKind::Text(t) = &d.node(id).unwrap().kind else { panic!() };
    assert_eq!(t.plain_text(), "曲雅楽演奏");
    assert_eq!(tool.preedit, Some(Preedit { range: 3..15, active: Some(6..12) }));
    let widths: Vec<f32> = tool
        .overlays(&cx(&d, &sel, &p))
        .iter()
        .filter_map(|o| match o {
            Overlay::Path { width, .. } => Some(*width),
            _ => None,
        })
        .collect();
    assert_eq!(widths, vec![1.0, 2.5], "a thin underline, then the thick one under 演奏");
    // The candidate window follows the converting clause.
    let (top, _) = tool.ime_caret(&cx(&d, &sel, &p)).unwrap();
    let lay = vectorcraft_text::layout(FontDb::global(), t);
    let (at, _) = vectorcraft_text::caret_position(&lay, 9);
    assert!((top - t.xf * at).hypot() < 1e-6);
    // Keys wait for the IME; a commit replaces the marked text.
    assert!(key(&mut tool, &d, ToolKey::Backspace, Mods::default()).is_empty());
    let acts = tool.text_input(&cx(&d, &sel, &p), "雅楽演奏会");
    apply(&mut d, &snap, &acts);
    let NodeKind::Text(t) = &d.node(id).unwrap().kind else { panic!() };
    assert_eq!(t.plain_text(), "曲雅楽演奏会");
    assert!(!tool.composing());
}

#[test]
fn spacing_arrows_follow_each_text_objects_orientation() {
    let cases = [
        (ToolKey::Down, ALT, "kerning", 1.0),
        (ToolKey::Up, ALT, "kerning", -1.0),
        (ToolKey::Left, ALT, "leading", 1.0),
        (ToolKey::Right, ALT, "leading", -1.0),
        (ToolKey::Right, SHIFT_ALT, "baselineShift", 1.0),
        (ToolKey::Left, SHIFT_ALT, "baselineShift", -1.0),
        (ToolKey::Down, SHIFT_ALT, "kerning", 1.0),
        (ToolKey::Up, SHIFT_ALT, "kerning", -1.0),
        (ToolKey::Down, Mods { cmd: true, ..ALT }, "kerning", 5.0),
        (ToolKey::Up, Mods { cmd: true, ..ALT }, "kerning", -5.0),
    ];
    for kind in 0..3 {
        for selected in [false, true] {
            let (mut d, id, mut tool) = editing("abcd");
            let NodeKind::Text(t) = &mut d.node_mut(id).unwrap().kind else { panic!("text") };
            t.vertical = true;
            let path = vectorcraft_geom::PathData::from_bezpath(&Rect::new(0.0, 0.0, 100.0, 100.0).to_path(0.1));
            t.kind = match kind {
                1 => TextKind::Area { frame: path },
                2 => TextKind::OnPath { path, start: 0.0, end: None },
                _ => TextKind::Point,
            };
            tool.caret = 2;
            tool.anchor = if selected { 1 } else { 2 };
            for (key_code, mods, caret_attribute, by) in cases {
                let before = (tool.anchor, tool.caret);
                let actions = key(&mut tool, &d, key_code, mods);
                let step = actions
                    .iter()
                    .find_map(|action| match action {
                        Action::Exec(command, params) if command == "type.step" => Some(params),
                        _ => None,
                    })
                    .expect("spacing command");
                assert_eq!(
                    step["attribute"],
                    if selected && caret_attribute == "kerning" { "tracking" } else { caret_attribute },
                    "kind {kind}, selected {selected}"
                );
                assert_eq!(step["by"], by);
                assert_eq!((tool.anchor, tool.caret), before);
            }
        }
    }
    let (mut d, id, mut tool) = editing("abcd");
    let NodeKind::Text(t) = &mut d.node_mut(id).unwrap().kind else { panic!("text") };
    t.vertical = true;
    key(&mut tool, &d, ToolKey::Down, SHIFT);
    assert_eq!((tool.anchor, tool.caret), (0, 1));
    let (d, _, mut tool) = editing("abcd");
    let actions = key(&mut tool, &d, ToolKey::Right, ALT);
    assert!(actions.iter().any(
        |action| matches!(action, Action::Exec(command, params) if command == "type.step" && params["attribute"] == "kerning" && params["by"] == 1.0)
    ));
}

#[test]
fn arrows_follow_the_columns_of_vertical_type() {
    let (mut d, id) = doc_with_text("§§§\n§§§");
    if let Some(NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) {
        t.vertical = true;
    }
    let mut tool = TypeTool::new("type");
    tool.start_editing(id, 0);
    let s = "§".len();
    key(&mut tool, &d, ToolKey::Down, Mods::default());
    assert_eq!(tool.caret, s, "↓ goes down the column to the next character");
    key(&mut tool, &d, ToolKey::Left, Mods::default());
    assert_eq!(tool.caret, "§§§\n§".len(), "← goes on to the next column, keeping the place in it");
    key(&mut tool, &d, ToolKey::Right, Mods::default());
    assert_eq!(tool.caret, s, "→ comes back");
    key(&mut tool, &d, ToolKey::Up, Mods::default());
    assert_eq!(tool.caret, 0);
}

#[test]
fn hebrew_typing_and_visual_arrows_keep_logical_text() {
    let (mut d, id, mut tool) = editing("");
    let snap = d.clone();
    let (sel, p) = (Selection::default(), paint());
    let actions = tool.text_input(&cx(&d, &sel, &p), "שלום");
    apply(&mut d, &snap, &actions);
    assert_eq!(tool.caret, "שלום".len());
    key(&mut tool, &d, ToolKey::Right, Mods::default());
    assert_eq!(tool.caret, 6, "Right moves toward the logical start in Hebrew");
    key(&mut tool, &d, ToolKey::Left, Mods::default());
    assert_eq!(tool.caret, 8);
    let NodeKind::Text(t) = &d.node(id).unwrap().kind else { panic!() };
    assert_eq!(t.plain_text(), "שלום");
}

/// View › Snap to Glyph: new point type placed near live text snaps to its baseline.
#[test]
fn snap_to_glyph_lines_up_new_point_type_with_a_baseline() {
    let (d, _) = doc_with_text("Baseline");
    let (sel, p) = (Selection::default(), paint());
    let mut c = cx(&d, &sel, &p);
    c.snap_to_glyph = true;
    let existing = match &d.layers[0].children().unwrap()[0].kind {
        NodeKind::Text(t) => t,
        _ => panic!("text"),
    };
    let lay = vectorcraft_text::layout(FontDb::global(), existing);
    let baseline_y = (existing.xf * Point::new(0.0, lay.lines[0].baseline)).y;
    let mut tool = TypeTool::new("type");
    tool.pointer(&c, &PointerEvent::new(PointerKind::Down, 250.0, baseline_y + 2.0));
    let acts = tool.pointer(&c, &PointerEvent::new(PointerKind::Up, 250.0, baseline_y + 2.0));
    assert!(
        acts.iter().any(|a| matches!(a, Action::Exec(cmd, v) if cmd == "text.create" && (v["y"].as_f64().unwrap() - baseline_y).abs() < 0.5)),
        "{acts:?}"
    );
    c.snap_to_glyph = false;
    tool.pointer(&c, &PointerEvent::new(PointerKind::Down, 250.0, baseline_y + 2.0));
    let acts = tool.pointer(&c, &PointerEvent::new(PointerKind::Up, 250.0, baseline_y + 2.0));
    assert!(
        acts.iter().any(|a| matches!(a, Action::Exec(cmd, v) if cmd == "text.create" && (v["y"].as_f64().unwrap() - (baseline_y + 2.0)).abs() < 0.5)),
        "{acts:?}"
    );
}

/// Where new type goes snaps to Smart Guides (#506): a click beside a rect lines up with its
/// centre, and hovering shows it first; an area's corners land on its anchors.
#[test]
fn new_type_snaps_to_smart_guides() {
    let (d, _) = crate::testutil::doc_with_rect();
    let (sel, p) = (Selection::default(), paint());
    let c = cx(&d, &sel, &p);
    let mut tool = TypeTool::new("type");
    tool.pointer(&c, &PointerEvent::new(PointerKind::Move, 151.0, 330.0));
    assert!(tool.overlays(&c).iter().any(|o| matches!(o, Overlay::Label { text, p, .. } if text == "align" && *p == Point::new(150.0, 330.0))));
    tool.pointer(&c, &PointerEvent::new(PointerKind::Down, 151.0, 330.0));
    let acts = tool.pointer(&c, &PointerEvent::new(PointerKind::Up, 151.0, 330.0));
    assert!(acts.iter().any(|a| matches!(a, Action::Exec(cmd, v) if cmd == "text.create" && v["x"] == 150.0 && v["y"] == 330.0)), "{acts:?}");
    let mut tool = TypeTool::new("type");
    tool.pointer(&c, &PointerEvent::new(PointerKind::Down, 102.0, 98.0));
    tool.pointer(&c, &PointerEvent::new(PointerKind::Drag, 199.0, 202.0));
    let acts = tool.pointer(&c, &PointerEvent::new(PointerKind::Up, 199.0, 202.0));
    let area = json!({"width": 100.0, "height": 100.0});
    assert!(acts.iter().any(|a| matches!(a, Action::Exec(cmd, v) if cmd == "text.create" && v["x"] == 100.0 && v["area"] == area)), "{acts:?}");
}

#[test]
fn area_type_drag_measure_labels_honor_preference() {
    let (d, _) = crate::testutil::doc_with_rect();
    let (sel, p) = (Selection::default(), paint());
    let on = ToolContext { measurement_labels: true, ..cx(&d, &sel, &p) };
    let off = ToolContext { measurement_labels: false, ..cx(&d, &sel, &p) };
    let mut tool = TypeTool::new("type");
    tool.pointer(&on, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
    tool.pointer(&on, &PointerEvent::new(PointerKind::Drag, 200.0, 150.0));
    let want = on.size_label(100.0, 50.0);
    assert!(tool.overlays(&on).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == &want)));
    assert!(!tool.overlays(&off).iter().any(|o| matches!(o, Overlay::Measure { .. })));
}

#[test]
fn layout_cache_follows_fonts_added_later() {
    // Other tests can load system fonts; this assertion needs its own process-wide database.
    #[cfg(not(target_arch = "wasm32"))]
    if std::env::var("VECTORCRAFT_FONT_CACHE_TEST_CHILD").as_deref() != Ok("1") {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "text::tests::layout_cache_follows_fonts_added_later", "--nocapture", "--test-threads=1"])
            .env("VECTORCRAFT_FONT_CACHE_TEST_CHILD", "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("test result: ok. 1 passed; 0 failed;"),
            "isolated font cache test failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr),
        );
        return;
    }
    // The web build adds fonts after startup: a caret layout cached before one arrived is stale.
    let (d, id, tool) = editing("abc");
    let NodeKind::Text(t) = &d.node(id).unwrap().kind else { panic!("not text") };
    let before = tool.layout(t);
    assert!(Arc::ptr_eq(&before, &tool.layout(t)), "cached while the fonts don't change");
    let font = vectorcraft_text::test_fonts::variable_font().unwrap();
    assert!(FontDb::global().add_font(font) > 0, "the test font is new to this process");
    assert!(!Arc::ptr_eq(&before, &tool.layout(t)), "a font added after the layout must refresh it");
}

#[test]
fn escape_returns_to_previous_tool_when_set() {
    let (d, _id, mut tool) = editing("Hello");
    tool.set_option("prevTool", &json!("directSelection"));
    let acts = key(&mut tool, &d, ToolKey::Escape, Mods::default());
    assert_eq!(acts.last(), Some(&Action::SwitchTool("directSelection".into())));

    // Without prevTool set, Escape defaults to Selection tool.
    let (d2, _id2, mut tool2) = editing("World");
    let acts2 = key(&mut tool2, &d2, ToolKey::Escape, Mods::default());
    assert_eq!(acts2.last(), Some(&Action::SwitchTool("selection".into())));
}

/// A click from `tool` at (x, y) → the actions of the press and the release.
fn click(tool: &mut TypeTool, c: &ToolContext, x: f64, y: f64) -> Vec<Action> {
    let mut acts = tool.pointer(c, &PointerEvent::new(PointerKind::Down, x, y));
    acts.extend(tool.pointer(c, &PointerEvent::new(PointerKind::Up, x, y)));
    acts
}

fn creates(acts: &[Action]) -> bool {
    acts.iter().any(|a| matches!(a, Action::Exec(cmd, _) if cmd == "text.create" || cmd == "text.createInPath"))
}

/// While type is being edited, a click on blank canvas ends the editing and makes no new type;
/// the Type tool stays, and the next click makes point type (#2871, #2854). The same holds when
/// the editing began with a double-click from a selection tool.
#[test]
fn a_click_away_from_the_type_being_edited_only_ends_editing() {
    for prev in [None, Some("selection")] {
        let (d, _, mut tool) = editing("Hello");
        if let Some(prev) = prev {
            tool.set_option("prevTool", &json!(prev));
        }
        let (sel, p) = (Selection::default(), paint());
        let c = cx(&d, &sel, &p);
        let acts = click(&mut tool, &c, 400.0, 400.0);
        assert!(!creates(&acts), "{prev:?}: {acts:?}");
        assert!(!acts.iter().any(|a| matches!(a, Action::SwitchTool(_))), "{prev:?}: {acts:?}");
        assert_eq!((tool.id(), tool.editing), ("type", None));
        assert_eq!(tool.prev_tool.as_deref(), prev);
        let acts = click(&mut tool, &c, 400.0, 400.0);
        assert!(
            acts.iter().any(|a| matches!(a, Action::Exec(cmd, v) if cmd == "text.create" && v["x"] == 400.0 && v.get("area").is_none())),
            "{acts:?}"
        );
    }
}

/// Point type a click placed and left empty is still discarded by the click away, and that click
/// places no other type.
#[test]
fn a_click_away_from_empty_new_type_discards_it_and_makes_none() {
    let (d, id, mut tool) = editing("");
    tool.fresh = Some(id);
    let (sel, p) = (Selection::default(), paint());
    let acts = click(&mut tool, &cx(&d, &sel, &p), 400.0, 400.0);
    assert_eq!(acts, vec![Action::Exec("text.discardEmpty".into(), json!({"id": id.0}))]);
}

/// Area Type and Type on a Path: a click on a path while other type is being edited ends the
/// editing only; the next click turns the path into a frame or a baseline.
#[test]
fn a_click_on_a_path_while_editing_only_ends_editing() {
    let (mut d, rect) = crate::testutil::doc_with_rect();
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let t = TextObject::point(Point::new(300.0, 400.0), "Hello", CharStyle { size: 20.0, ..Default::default() });
    d.insert(Some(l), 1, Node::new(id, NodeKind::Text(Box::new(t)))).unwrap();
    let (sel, p) = (Selection::default(), paint());
    let c = cx(&d, &sel, &p);
    for (tool_id, at) in [("areaType", (100.0, 150.0)), ("typeOnPath", (150.0, 100.0))] {
        let mut tool = TypeTool::new(tool_id);
        tool.start_editing(id, 0);
        let acts = click(&mut tool, &c, at.0, at.1);
        assert!(!creates(&acts), "{tool_id}: {acts:?}");
        assert_eq!(tool.editing, None);
        let acts = click(&mut tool, &c, at.0, at.1);
        assert!(
            acts.iter().any(|a| matches!(a, Action::Exec(cmd, v) if cmd == "text.createInPath" && v["path"] == json!(rect.0))),
            "{tool_id}: {acts:?}"
        );
    }
}

/// While type is being edited, a drag that draws an area-type frame still makes it, and a click
/// on other type edits that type at once.
#[test]
fn a_drag_or_a_click_on_other_type_while_editing_acts_at_once() {
    let (mut d, id, mut tool) = editing("Hello");
    let (sel, p) = (Selection::default(), paint());
    let acts = {
        let c = cx(&d, &sel, &p);
        tool.pointer(&c, &PointerEvent::new(PointerKind::Down, 300.0, 300.0));
        tool.pointer(&c, &PointerEvent::new(PointerKind::Drag, 400.0, 400.0));
        tool.pointer(&c, &PointerEvent::new(PointerKind::Up, 400.0, 400.0))
    };
    let area = json!({"width": 100.0, "height": 100.0});
    assert!(acts.iter().any(|a| matches!(a, Action::Exec(cmd, v) if cmd == "text.create" && v["area"] == area)), "{acts:?}");
    let l = d.layers[0].id;
    let other = d.alloc_id();
    let t = TextObject::point(Point::new(100.0, 300.0), "Other", CharStyle { size: 20.0, ..Default::default() });
    d.insert(Some(l), 1, Node::new(other, NodeKind::Text(Box::new(t)))).unwrap();
    let c = cx(&d, &sel, &p);
    let t = TypeTool::text(&c, other).unwrap().clone();
    let lay = vectorcraft_text::layout(FontDb::global(), &t);
    let (top, bot) = vectorcraft_text::caret_position(&lay, 2);
    let at = t.xf * top.midpoint(bot);
    tool.start_editing(id, 0);
    let acts = click(&mut tool, &c, at.x, at.y);
    assert!(!creates(&acts), "{acts:?}");
    assert!(acts.contains(&Action::Exec("select.set".into(), json!({"ids": [other.0]}))), "{acts:?}");
    assert_eq!(tool.editing, Some(other));
}

/// #2572: selected area type shows its in and out ports with any tool, not only the Type tools
/// (#2323): empty while its text fits and it isn't threaded, the out port's red "+" when the text
/// doesn't fit, an arrow in the port on the side where it is threaded.
#[test]
fn selected_type_shows_its_ports_in_each_state() {
    let (mut d, id) = crate::testutil::doc_with_area_type();
    let (mut sel, p) = (Selection::default(), paint());
    sel.set([id]);
    let mut marks = PortMarks::default();
    // (white boxes, outlines, glyph strokes, red outlines)
    let count = |ov: &[Overlay]| {
        let n = |f: &dyn Fn(&Overlay) -> bool| ov.iter().filter(|o| f(o)).count();
        (
            n(&|o| matches!(o, Overlay::Highlight { color: [255, 255, 255, 255], .. })),
            n(&|o| matches!(o, Overlay::Path { .. })),
            n(&|o| matches!(o, Overlay::Line { .. })),
            n(&|o| matches!(o, Overlay::Path { color: [230, 30, 30], .. })),
        )
    };
    assert_eq!(count(&thread_ports(&mut marks, &cx(&d, &sel, &p))), (2, 2, 0, 0), "the text fits: two empty ports");
    let Some(NodeKind::Text(t)) = d.node(id).map(|n| &n.kind) else { panic!("not text") };
    let (input, output) = (t.xf * Point::new(0.0, PORT_RISE_PX), t.xf * Point::new(120.0, 40.0 - PORT_RISE_PX));
    let near = |a: Option<Point>, b: Point| a.is_some_and(|a| a.distance(b) < 1e-9);
    assert!(near(port_centre(t, Port::In, 1.0), input), "on the left side, below the top-left corner");
    assert!(near(port_centre(t, Port::Out, 1.0), output), "on the right side, above the bottom-right corner");
    let ov = thread_ports(&mut marks, &cx(&d, &sel, &p));
    let box_at = |c: Point| ov.iter().any(|o| matches!(o, Overlay::Highlight { quad, .. } if quad[0].midpoint(quad[2]).distance(c) < 1e-9));
    assert!(box_at(input) && box_at(output), "{ov:?}");
    if let Some(NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) {
        t.runs = vec![TextRun::new("Words that fill more than the frame holds. ".repeat(20), CharStyle { size: 20.0, ..Default::default() })];
    }
    assert_eq!(count(&thread_ports(&mut marks, &cx(&d, &sel, &p))), (2, 2, 2, 1), "an empty in port and the red plus");
    assert_eq!(port_states(&d, id, true), (PortState::Empty, PortState::Overflow));
    // Threaded on both sides: arrows (three strokes each), the out port's in place of the plus.
    let (a, b) = (d.alloc_id(), d.alloc_id());
    d.text_threads = vec![vec![a, id, b]];
    assert_eq!(port_states(&d, id, true), (PortState::Threaded, PortState::Threaded));
    assert_eq!(count(&thread_ports(&mut marks, &cx(&d, &sel, &p))), (2, 2, 6, 0));
    d.text_threads = vec![vec![id, a]];
    assert_eq!(port_states(&d, id, false), (PortState::Empty, PortState::Threaded));
    d.text_threads = vec![vec![a, id]];
    assert_eq!(port_states(&d, id, false), (PortState::Threaded, PortState::Empty));
    assert_eq!(port_states(&d, id, true), (PortState::Threaded, PortState::Overflow), "the last frame's text doesn't fit");
    sel.clear();
    assert!(thread_ports(&mut marks, &cx(&d, &sel, &p)).is_empty(), "only selected type shows them");
}

/// Past [`PORTS_MAX`] selected frames (select all of an opened PDF) no ports are drawn or clicked:
/// they would bury the canvas in boxes; up to it every frame has its two.
#[test]
fn a_huge_selection_of_type_shows_no_ports() {
    let (mut d, id) = crate::testutil::doc_with_area_type();
    let node = d.node(id).unwrap().clone();
    let l = d.layers[0].id;
    let mut sel = Selection::default();
    sel.set([id]);
    for _ in 0..PORTS_MAX {
        let copy = d.alloc_id();
        d.insert(Some(l), 1, Node::new(copy, node.kind.clone())).unwrap();
        sel.add(copy);
    }
    let p = paint();
    let Some(NodeKind::Text(t)) = d.node(id).map(|n| &n.kind) else { panic!("not text") };
    let out = port_centre(t, Port::Out, 1.0).unwrap();
    let mut marks = PortMarks::default();
    assert!(thread_ports(&mut marks, &cx(&d, &sel, &p)).is_empty());
    assert_eq!(port_at(&cx(&d, &sel, &p), out), None);
    sel.remove(id);
    let ports = thread_ports(&mut marks, &cx(&d, &sel, &p));
    assert_eq!(ports.iter().filter(|o| matches!(o, Overlay::Highlight { .. })).count(), 2 * PORTS_MAX, "at the limit");
}

/// Type in perspective has no ports, as it can't thread ([`crate::select::is_threadable`]): none
/// are drawn with any tool, the Type tools included, and it doesn't count toward [`PORTS_MAX`].
#[test]
fn type_in_perspective_has_no_ports_and_doesnt_count() {
    let (mut d, id) = crate::testutil::doc_with_area_type();
    d.node_mut(id).unwrap().extras_mut().perspective = Some(Box::new(vectorcraft_doc::PerspectiveAttachment::new("left", 0.0)));
    let (mut sel, p) = (Selection::default(), paint());
    sel.set([id]);
    let mut marks = PortMarks::default();
    assert!(thread_ports(&mut marks, &cx(&d, &sel, &p)).is_empty(), "no ports on type in perspective");
    let boxes = |o: &[Overlay]| o.iter().filter(|o| matches!(o, Overlay::Highlight { color: [255, 255, 255, 255], .. })).count();
    assert_eq!(boxes(&TypeTool::new("type").overlays(&cx(&d, &sel, &p))), 0, "nor with the Type tool");
    // With PORTS_MAX frames that have ports selected besides it, theirs still show.
    let mut node = d.node(id).unwrap().clone();
    node.extras_mut().perspective = None;
    let l = d.layers[0].id;
    for _ in 0..PORTS_MAX {
        let copy = d.alloc_id();
        d.insert(Some(l), 1, Node::new(copy, node.kind.clone())).unwrap();
        sel.add(copy);
    }
    assert_eq!(boxes(&thread_ports(&mut marks, &cx(&d, &sel, &p))), 2 * PORTS_MAX);
}

/// The Type tool draws the ports of the type it edits as of the rest of the selection: none past
/// [`PORTS_MAX`] selected frames, where they can't be clicked either.
#[test]
fn the_edited_types_ports_follow_the_limit_too() {
    let (mut d, id) = crate::testutil::doc_with_area_type();
    let node = d.node(id).unwrap().clone();
    let l = d.layers[0].id;
    let mut sel = Selection::default();
    sel.set([id]);
    for _ in 1..PORTS_MAX {
        let copy = d.alloc_id();
        d.insert(Some(l), 1, Node::new(copy, node.kind.clone())).unwrap();
        sel.add(copy);
    }
    let p = paint();
    let mut tool = TypeTool::new("type");
    tool.start_editing(id, 0);
    let boxes = |o: &[Overlay]| o.iter().filter(|o| matches!(o, Overlay::Highlight { color: [255, 255, 255, 255], .. })).count();
    assert_eq!(boxes(&tool.overlays(&cx(&d, &sel, &p))), 2 * PORTS_MAX, "at the limit, the edited one's included");
    let copy = d.alloc_id();
    d.insert(Some(l), 1, Node::new(copy, node.kind.clone())).unwrap();
    sel.add(copy);
    assert_eq!(boxes(&tool.overlays(&cx(&d, &sel, &p))), 0, "past it, not even the edited one's");
}

/// Type on a path has its in port just before its start bracket and its out port just past its
/// end bracket, along the path, clear of the brackets.
#[test]
fn type_on_a_path_has_its_ports_beyond_its_brackets() {
    let (d, id) = crate::testutil::doc_with_path_type();
    let Some(NodeKind::Text(t)) = d.node(id).map(|n| &n.kind) else { panic!("not text") };
    let near = |a: Option<Point>, b: Point| a.is_some_and(|a| a.distance(b) < 1e-6);
    assert!(near(port_centre(t, Port::In, 1.0), Point::new(160.0 - PORT_RISE_PX, 300.0)), "{:?}", port_centre(t, Port::In, 1.0));
    assert!(near(port_centre(t, Port::Out, 1.0), Point::new(400.0 + PORT_RISE_PX, 300.0)), "{:?}", port_centre(t, Port::Out, 1.0));
    assert!(near(port_centre(t, Port::In, 0.5), Point::new(160.0 - PORT_RISE_PX / 2.0, 300.0)), "screen pixels at 200%");
}

/// The outgoing query copies only the 4 KiB window around the selection, not the whole document,
/// and reports offsets into that window.
#[test]
fn ime_surrounding_bounds_a_long_document() {
    let source: String = (0..10_000).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
    let (d, _id, mut tool) = editing(&source);
    tool.caret = 5000;
    tool.anchor = 5000;
    let ImeSurrounding::Available { text, cursor, anchor } = tool.ime_surrounding(&cx(&d, &Selection::default(), &paint()), 4000) else {
        panic!("a caret in a long document is available");
    };
    assert_eq!(text.len(), 4000, "only the bounded window is copied");
    assert_eq!(text.as_str(), &source[3000..7000]);
    assert_eq!((cursor, anchor), (2000, 2000), "offsets are relative to the returned window");

    tool.anchor = 0;
    let ImeSurrounding::Unavailable(reason) = tool.ime_surrounding(&cx(&d, &Selection::default(), &paint()), 4000) else {
        panic!("a selection wider than the limit must be explicit");
    };
    assert!(reason.contains("selection"), "{reason}");
}

/// Unicode offsets are UTF-8 bytes and a reversed selection stays reversed through the query.
#[test]
fn ime_surrounding_preserves_reversed_unicode_selection_and_empty_text() {
    let (d, _id, mut tool) = editing("aé😀z");
    tool.caret = 1;
    tool.anchor = 7;
    let ImeSurrounding::Available { text, cursor, anchor } = tool.ime_surrounding(&cx(&d, &Selection::default(), &paint()), 4000) else {
        panic!("the short text is available");
    };
    assert_eq!(text, "aé😀z");
    assert_eq!((cursor, anchor), (1, 7), "cursor is the primary end, anchor the other");

    let (d, _id, tool) = editing("");
    let ImeSurrounding::Available { text, cursor, anchor } = tool.ime_surrounding(&cx(&d, &Selection::default(), &paint()), 4000) else {
        panic!("an empty document is available, not unavailable");
    };
    assert_eq!((text.as_str(), cursor, anchor), ("", 0, 0));
}

/// Deleting around a selection removes only the surrounding text, keeps the reversed selection
/// direction, and leaves it for the following commit to replace.
#[test]
fn ime_delete_surrounding_preserves_a_reversed_selection() {
    let (mut d, _id, mut tool) = editing("abcd");
    let snap = d.clone();
    let (sel, p) = (Selection::default(), paint());
    tool.anchor = 3; // "bc", selected backwards
    tool.caret = 1;
    let acts = tool.ime_delete_surrounding(&cx(&d, &sel, &p), 1, 1, SurroundingUnit::Bytes);
    apply(&mut d, &snap, &acts);
    let NodeKind::Text(t) = &d.node(_id).unwrap().kind else { panic!() };
    assert_eq!(t.plain_text(), "bc", "the selection itself is preserved");
    assert_eq!((tool.caret, tool.anchor), (0, 2), "direction is kept, shifted left by the preceding deletion");
    let ImeSurrounding::Available { text, cursor, anchor } = tool.ime_surrounding(&cx(&d, &sel, &p), 4000) else {
        panic!("the selection is still an available context");
    };
    assert_eq!((text.as_str(), cursor, anchor), ("bc", 0, 2));
}

/// Deleting around a composition removes only the surrounding committed text, keeps the pre-edit
/// in place for the commit to replace, and remaps the composition range.
#[test]
fn ime_delete_surrounding_preserves_the_preedit() {
    let (mut d, id, mut tool) = editing("abcd");
    let snap = d.clone();
    let (sel, p) = (Selection::default(), paint());
    tool.caret = 2;
    tool.anchor = 2;
    let acts = tool.ime_preedit(&cx(&d, &sel, &p), "xy", None);
    apply(&mut d, &snap, &acts);
    assert_eq!(tool.preedit.as_ref().map(|p| p.range.clone()), Some(2..4));
    let acts = tool.ime_delete_surrounding(&cx(&d, &sel, &p), 1, 0, SurroundingUnit::Bytes);
    apply(&mut d, &snap, &acts);
    let NodeKind::Text(t) = &d.node(id).unwrap().kind else { panic!() };
    assert_eq!(t.plain_text(), "axycd", "only the byte before the composition is deleted");
    assert_eq!(tool.preedit.as_ref().map(|p| p.range.clone()), Some(1..3), "the composition stayed and moved left");
    let ImeSurrounding::Available { text, cursor, anchor } = tool.ime_surrounding(&cx(&d, &sel, &p), 4000) else {
        panic!("the composition has committed text around it");
    };
    assert_eq!((text.as_str(), cursor, anchor), ("acd", 1, 1), "the pre-edit is excluded and remapped");
}

/// Edit-session identity changes only when editing starts afresh, not on ordinary edits or caret
/// moves; the owner keeps the document uid and target id.
#[test]
fn ime_target_is_stable_within_an_edit_session() {
    let (d, id, mut tool) = editing("abc");
    let (sel, p) = (Selection::default(), paint());
    let first = tool.ime_target(&cx(&d, &sel, &p)).expect("editing has an owner");
    assert_eq!((first.0, first.1), (0, id.0));
    tool.caret = 1;
    tool.anchor = 3;
    assert_eq!(tool.ime_target(&cx(&d, &sel, &p)), Some(first), "caret and selection moves keep the context");
    tool.start_editing(id, 0);
    let second = tool.ime_target(&cx(&d, &sel, &p)).expect("re-entering has an owner");
    assert_eq!((second.0, second.1), (first.0, first.1));
    assert_ne!(second.2, first.2, "a new edit session is a replacement epoch");
}

/// The bounded query walks the styled runs without joining them, and its byte offsets still refer
/// to the concatenated logical text.
#[test]
fn ime_surrounding_maps_across_styled_runs() {
    use vectorcraft_doc::TextRun;
    let (mut d, id, mut tool) = editing("aé😀z");
    let NodeKind::Text(t) = &mut d.node_mut(id).unwrap().kind else { panic!() };
    t.runs =
        vec![TextRun::new("aé", CharStyle { size: 20.0, ..Default::default() }), TextRun::new("😀z", CharStyle { size: 24.0, ..Default::default() })];
    tool.caret = 1;
    tool.anchor = 7;
    let ImeSurrounding::Available { text, cursor, anchor } = tool.ime_surrounding(&cx(&d, &Selection::default(), &paint()), 4000) else {
        panic!("the short text is available");
    };
    assert_eq!((text.as_str(), cursor, anchor), ("aé😀z", 1, 7));
}

/// Deletion counts start at the caret, including when it is in a later styled run. Character
/// units count Unicode scalars; byte units round inward instead of cutting a multibyte scalar.
#[test]
fn ime_character_deletion_walks_styled_runs_backwards_from_the_caret() {
    use vectorcraft_doc::TextRun;
    for (caret, before, after, unit, expected, shifted, committed) in [
        (7, 1, 0, SurroundingUnit::Chars, "aéz!x", 3, "aéQz!x"),
        (8, 3, 0, SurroundingUnit::Chars, "a!x", 1, "aQ!x"),
        (7, 3, 0, SurroundingUnit::Bytes, "aé😀z!x", 7, "aé😀Qz!x"),
        (7, 1, 1, SurroundingUnit::Chars, "aé!x", 3, "aéQ!x"),
        (7, 4, 1, SurroundingUnit::Bytes, "aé!x", 3, "aéQ!x"),
        (9, 1, 1, SurroundingUnit::Chars, "aé😀z", 8, "aé😀zQ"),
    ] {
        let (mut d, id, mut tool) = editing("aé😀z!x");
        let NodeKind::Text(t) = &mut d.node_mut(id).unwrap().kind else { panic!() };
        t.runs = vec![
            TextRun::new("aé", CharStyle { size: 20.0, ..Default::default() }),
            TextRun::new("😀z", CharStyle { size: 24.0, ..Default::default() }),
            TextRun::new("!x", CharStyle { size: 28.0, ..Default::default() }),
        ];
        tool.caret = caret;
        tool.anchor = caret;
        let snap = d.clone();
        let (sel, p) = (Selection::default(), paint());
        let owner = tool.ime_target(&cx(&d, &sel, &p));
        let acts = tool.ime_delete_surrounding(&cx(&d, &sel, &p), before, after, unit);
        apply(&mut d, &snap, &acts);
        let NodeKind::Text(t) = &d.node(id).unwrap().kind else { panic!() };
        assert_eq!(t.plain_text(), expected, "caret={caret} before={before} after={after} unit={unit:?}");
        assert_eq!((tool.caret, tool.anchor), (shifted, shifted));
        assert_eq!(tool.ime_target(&cx(&d, &sel, &p)), owner, "deletion preserves the immutable edit target");
        let acts = tool.text_input(&cx(&d, &sel, &p), "Q");
        apply(&mut d, &snap, &acts);
        let NodeKind::Text(t) = &d.node(id).unwrap().kind else { panic!() };
        assert_eq!(t.plain_text(), committed, "the following commit uses the shifted caret");
    }
}

/// A recreated Type tool (a document switch, File › Revert) must not reuse the replacement epoch
/// of the same document and text node: a still-cached native IME context would look current.
#[test]
fn a_recreated_type_tool_gets_a_new_replacement_epoch() {
    let (d, id) = doc_with_text("Same");
    let (sel, p) = (Selection::default(), paint());
    let mut first = TypeTool::new("type");
    first.start_editing(id, 0);
    let a = first.ime_target(&cx(&d, &sel, &p)).expect("the first tool edits");
    let mut second = TypeTool::new("type");
    second.start_editing(id, 0);
    let b = second.ime_target(&cx(&d, &sel, &p)).expect("the recreated tool edits");
    assert_eq!((a.0, a.1), (b.0, b.1), "the owner key is the document and text node");
    assert_ne!(a.2, b.2, "edit sessions are numbered process-wide, not per tool");
}

/// The surrounding query never publishes a typing session's text after the document changed
/// outside it: it reports the document's own text (and drops the stale marked-text range), and the
/// commit-time session check keeps the reset from earning an IME receipt.
#[test]
fn ime_surrounding_reports_the_document_when_it_changed_under_the_session() {
    let (mut d, id, mut tool) = editing("abc");
    let (sel, p) = (Selection::default(), paint());
    tool.caret = 3;
    tool.anchor = 3;
    let snap = d.clone();
    let acts = tool.text_input(&cx(&d, &sel, &p), "x");
    apply(&mut d, &snap, &acts);
    let ImeSurrounding::Available { text, .. } = tool.ime_surrounding(&cx(&d, &sel, &p), 4000) else {
        panic!("the session matches the document");
    };
    assert_eq!(text, "abcx");
    // A panel, menu or control edit replaces the text while the typing session is open.
    let NodeKind::Text(t) = &mut d.node_mut(id).unwrap().kind else { panic!() };
    let len = edit::runs_len(&t.runs);
    let _ = edit::replace_range(&mut t.runs, 0, len, "XYZ");
    let stale = tool.ime_surrounding(&cx(&d, &sel, &p), 4000);
    assert_eq!(stale, ImeSurrounding::Available { text: "XYZ".into(), cursor: 3, anchor: 3 });
    assert!(!tool.ime_session_matches_document(&cx(&d, &sel, &p)), "the commit starts a fresh session");
}

#[test]
fn shift_enter_inserts_soft_return() {
    let (mut d, id, mut tool) = editing("Hello World");
    let (sel, p) = (Selection::default(), paint());
    tool.caret = 5;
    tool.anchor = 5;
    let snap = d.clone();
    let shift = Mods { shift: true, ..Default::default() };
    let acts = tool.key(&cx(&d, &sel, &p), ToolKey::Enter, shift);
    apply(&mut d, &snap, &acts);
    let NodeKind::Text(t) = &d.node(id).unwrap().kind else { panic!() };
    assert_eq!(t.plain_text(), "Hello\u{2028} World");
    // Plain Enter inserts standard newline
    let acts = tool.key(&cx(&d, &sel, &p), ToolKey::Enter, Mods::default());
    apply(&mut d, &snap, &acts);
    let NodeKind::Text(t) = &d.node(id).unwrap().kind else { panic!() };
    assert_eq!(t.plain_text(), "Hello\u{2028}\n World");
}
