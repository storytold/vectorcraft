//! Type tool state-machine tests (the engine side is covered in vectorcraft-engine's
//! `tests_textedit`).

use super::*;
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
    for glyph in &hindi_glyphs {
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
    for glyph in lay.glyphs.iter().filter(|g| g.byte < hindi.len()) {
        let used = db.face_by_id(glyph.font_id).expect("the wrapped Hindi glyph's loaded face");
        assert!(used.file_data() == face.file_data(), "setup: wrapped Hindi uses the selected face bytes");
        assert_eq!(used.face_index(), face.face_index(), "setup: wrapped Hindi uses the selected face index");
        assert_ne!(glyph.gid, 0, "setup: wrapped Hindi glyph is present");
        assert!(!glyph.rtl, "setup: wrapped Hindi prefix stays LTR");
    }
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

#[test]
fn vertical_tools_preserve_their_ids_and_create_vertical_text() {
    for id in ["verticalType", "verticalAreaType", "verticalTypeOnPath"] {
        let tool = TypeTool::new(id);
        assert_eq!(tool.id(), id);
        assert!(tool.vertical);
    }
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

/// Selected area type whose text doesn't fit its frame shows the red out port with any tool, not
/// only the Type tools (#2323); text that fits shows none.
#[test]
fn overflowing_selected_type_shows_its_marker_without_the_type_tool() {
    let (mut d, id) = crate::testutil::doc_with_area_type();
    let (mut sel, p) = (Selection::default(), paint());
    sel.set([id]);
    let mut marks = OverflowMarks::default();
    assert!(overflow_markers(&mut marks, &cx(&d, &sel, &p)).is_empty(), "the text fits");
    if let Some(NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) {
        t.runs = vec![TextRun::new("Words that fill more than the frame holds. ".repeat(20), CharStyle { size: 20.0, ..Default::default() })];
    }
    let ov = overflow_markers(&mut marks, &cx(&d, &sel, &p));
    assert_eq!(ov.len(), 3, "a box and a plus: {ov:?}");
    sel.clear();
    assert!(overflow_markers(&mut marks, &cx(&d, &sel, &p)).is_empty(), "only selected type shows it");
}
