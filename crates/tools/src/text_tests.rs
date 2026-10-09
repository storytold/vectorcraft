//! Type tool state-machine tests (the engine side is covered in vectorcraft-engine's
//! `tests_textedit`).

use super::*;
use crate::testutil::{cx, paint};
use vectorcraft_doc::{CharStyle, Document, Node, Selection};

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
    tool.pointer(&c, &PointerEvent::new(PointerKind::Move, a.x + 0.1, a.y));
    assert!(
        tool.overlays(&c).iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "Click to edit text"))
    );
    let out = tool.pointer(&c, &PointerEvent::new(PointerKind::Down, a.x + 0.1, a.y));
    assert!(out.contains(&Action::Exec("select.set".into(), json!({"ids": [id.0]}))));
    assert!(!tool.overlays(&c).iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "Click to edit text")));
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
