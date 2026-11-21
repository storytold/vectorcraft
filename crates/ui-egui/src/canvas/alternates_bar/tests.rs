use std::{cell::RefCell, rc::Rc};

use craft::behavior::overlay::Side;
use craft::i18n::Direction;
use craft::ui::testing::Fixture;
use egui::{Pos2, Rect, pos2};
use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_text::FontDb;

use super::*;

/// The Type tool editing `text` in a new document.
fn editing(text: &str, vertical: bool) -> (VectorcraftApp, Value) {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    app.session.select_tool("type", Default::default()).unwrap();
    let id = app.run("text.create", json!({"x": 120, "y": 80, "text": text, "vertical": vertical})).unwrap()["id"].clone();
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    (app, id)
}

fn select(app: &mut VectorcraftApp, start: usize, end: usize) {
    app.session.set_tool_option("select", &json!({"start": start, "end": end}));
}

const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(800.0, 600.0));

/// The glyphs the row offers and whether it has the arrow, as the canvas finds them.
fn offered(app: &mut VectorcraftApp) -> Option<(Vec<u32>, bool)> {
    glyphs::on_canvas(app).map(|f| ((0..f.count()).filter_map(|i| f.gid(i)).collect(), f.more))
}

/// Whether the canvas shows a row found by its access name, after two frames (the access tree comes from a finished pass).
fn row_shown(app: &Rc<RefCell<VectorcraftApp>>) -> bool {
    let shared = Rc::clone(app);
    let mut fixture = Fixture::new(move |ui| super::super::show(&mut shared.borrow_mut(), ui));
    fixture.room(SCREEN.size());
    fixture.frame(vec![]);
    fixture.frame(vec![]);
    fixture.by_label("Character Alternates").is_some()
}

fn glyph_ids(app: &VectorcraftApp, id: &Value) -> Vec<u32> {
    let NodeKind::Text(text) = &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind else { panic!("text") };
    let mut glyphs = vectorcraft_text::layout(FontDb::global(), text).glyphs;
    glyphs.sort_by_key(|g| g.byte);
    glyphs.iter().map(|g| g.gid).collect()
}

fn undo_steps(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().history.undo.len()
}

/// With the Type tool and exactly one character selected, the canvas offers up to five of that character's alternates, without
/// the glyph it is drawn with, and an arrow when it has more. A pick draws the character with the glyph as one undo step and
/// keeps the text. A caret, a longer selection, a character without alternates, another tool and the preference turned off each
/// offer nothing and show no row.
#[test]
fn one_selected_character_shows_its_alternates_and_a_click_replaces_its_glyph() {
    use vectorcraft_text::test_fonts::{alternates_family, alternates_font, unencoded_glyphs};
    FontDb::global().add_font(alternates_font(true).expect("the test font builds"));
    let [u1, ..] = unencoded_glyphs().expect("unencoded glyphs");
    let (mut app, id) = editing("xay", false);
    app.run("text.setRangeStyle", json!({"id": id, "font": alternates_family(true)})).unwrap();
    let drawn = glyph_ids(&app, &id);

    select(&mut app, 1, 1);
    assert_eq!(offered(&mut app), None, "a caret selects no character");
    select(&mut app, 0, 2);
    assert_eq!(offered(&mut app), None, "two characters are selected");
    select(&mut app, 0, 1);
    assert_eq!(offered(&mut app), None, "x has no alternates");

    select(&mut app, 1, 2);
    let (gids, more) = offered(&mut app).expect("the a has alternates");
    assert_eq!(gids.len(), glyphs::ON_CANVAS_ALTERNATES, "five of the six alternates");
    assert!(more, "the sixth is in the Glyphs panel");
    assert_eq!(gids[0], u1);
    assert!(!gids.contains(&drawn[1]), "the glyph the a is drawn with is not offered");

    // A pick of the first glyph.
    let undo = undo_steps(&app);
    let found = glyphs::on_canvas(&mut app).expect("offered");
    found.pick(&mut app, 0);
    assert_eq!(undo_steps(&app), undo + 1, "one undo step; status: {}", app.ui.status);
    assert_eq!(glyph_ids(&app, &id), [drawn[0], u1, drawn[2]], "the a is drawn with the glyph");
    let NodeKind::Text(text) = &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind else { panic!("text") };
    assert_eq!(text.plain_text(), "xay", "the text keeps its characters");

    // The row stays for the same character: its default glyph first, the drawn glyph left out.
    let (gids, _) = offered(&mut app).expect("still one character selected");
    assert_eq!(gids[0], drawn[1], "the default glyph comes first");
    assert!(!gids.contains(&u1));
    let found = glyphs::on_canvas(&mut app).expect("offered");
    found.pick(&mut app, 0);
    assert_eq!(undo_steps(&app), undo + 2);
    assert_eq!(glyph_ids(&app, &id), drawn, "the default glyph again");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(glyph_ids(&app, &id), [drawn[0], u1, drawn[2]], "undo takes one pick back");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(glyph_ids(&app, &id), drawn);

    // Preferences › Type › Show Character Alternates.
    select(&mut app, 1, 2);
    let app = Rc::new(RefCell::new(app));
    assert!(row_shown(&app), "the row is found by its access name");
    app.borrow_mut().run("prefs.set", json!({"key": "showCharacterAlternates", "value": false})).unwrap();
    assert!(offered(&mut app.borrow_mut()).is_none(), "the preference is off");
    assert!(!row_shown(&app), "no row with the preference off");
    app.borrow_mut().run("prefs.set", json!({"key": "showCharacterAlternates", "value": true})).unwrap();
    assert!(offered(&mut app.borrow_mut()).is_some());

    app.borrow_mut().session.select_tool("select", Default::default()).unwrap();
    assert!(offered(&mut app.borrow_mut()).is_none(), "the Selection tool edits no text");
    assert!(!row_shown(&app), "no row for the Selection tool");
}

/// A medial letter of a joined Arabic word gets the alternates of its medial form, and a pick applies the feature that draws one
/// to that letter alone: its neighbors keep their joining forms. Three alternates need no arrow.
#[test]
fn a_letter_of_a_joined_word_keeps_the_word_joined() {
    use vectorcraft_text::test_fonts::{JOINING_ALTERNATES_FAMILY, joining_alternates_font, source_sans_gid};
    FontDb::global().add_font(joining_alternates_font().expect("the test font builds"));
    let g = |c| u32::from(source_sans_gid(c).expect("a glyph"));
    let (mut app, id) = editing("\u{628}\u{62A}\u{646}", false);
    app.run("text.setRangeStyle", json!({"id": id, "font": JOINING_ALTERNATES_FAMILY})).unwrap();
    assert_eq!(glyph_ids(&app, &id), [g('B'), g('u'), g('M')]);
    select(&mut app, 2, 4);
    let (gids, more) = offered(&mut app).expect("the medial teh has alternates");
    assert_eq!((gids.len(), more), (3, false), "the medial form's three alternates: {gids:?}");
    let at = gids.iter().position(|gid| *gid == g('x')).expect("the alternate salt=2 draws");
    let undo = undo_steps(&app);
    let found = glyphs::on_canvas(&mut app).expect("offered");
    found.pick(&mut app, at);
    assert_eq!(undo_steps(&app), undo + 1, "status: {}", app.ui.status);
    assert_eq!(glyph_ids(&app, &id), [g('B'), g('x'), g('M')], "the neighbors keep their initial and final forms");
    let NodeKind::Text(text) = &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind else { panic!("text") };
    assert_eq!(text.plain_text(), "\u{628}\u{62A}\u{646}");
    assert!(text.runs.iter().all(|r| r.style.glyph_alternate.is_none()), "a feature draws it, not a glyph choice");
}

/// [`side`]: under horizontal type, over it when the task bar lies under the character's column; on the physical right of
/// vertical type (`End` under LTR, `Start` under RTL), the opposite side when the task bar lies there.
#[test]
fn the_row_is_placed_on_the_side_that_has_room() {
    let sel = Rect::from_min_max(pos2(190.0, 100.0), pos2(210.0, 130.0));
    for direction in [Direction::Ltr, Direction::Rtl] {
        assert_eq!(side(sel, false, None, direction), Side::Below, "horizontal type: under");
        let under = Rect::from_min_max(pos2(100.0, 140.0), pos2(300.0, 180.0));
        assert_eq!(side(sel, false, Some(under), direction), Side::Above, "the task bar is under the character's column");
        let beside = Rect::from_min_max(pos2(300.0, 140.0), pos2(400.0, 180.0));
        assert_eq!(side(sel, false, Some(beside), direction), Side::Below, "a task bar outside the column leaves the row under");
    }
    let (right_of_sel, left_of_sel) = (Side::End, Side::Start);
    assert_eq!(side(sel, true, None, Direction::Ltr), right_of_sel, "vertical type: the physical right under LTR");
    assert_eq!(side(sel, true, None, Direction::Rtl), left_of_sel, "the physical right is the start side under RTL");
    let on_right = Rect::from_min_max(pos2(220.0, 90.0), pos2(400.0, 140.0));
    assert_eq!(side(sel, true, Some(on_right), Direction::Ltr), left_of_sel, "the task bar is on the right: the left");
    assert_eq!(side(sel, true, Some(on_right), Direction::Rtl), right_of_sel, "mirrored under RTL");
}
