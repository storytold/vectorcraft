use egui::{Pos2, Rect};
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

/// One frame of the canvas with `events`; where the row of alternates is afterwards, if it shows.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Option<Rect> {
    egui_extras::install_image_loaders(ctx);
    let raw = egui::RawInput { screen_rect: Some(SCREEN), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| super::super::show(app, ui));
    out.textures_delta.clear();
    bar_rect(ctx)
}

/// The glyphs the row offers and whether it has the arrow, as the canvas finds them.
fn offered(app: &mut VectorcraftApp, ctx: &egui::Context) -> Option<(Vec<u32>, bool)> {
    let mut found = None;
    let raw = egui::RawInput { screen_rect: Some(SCREEN), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| {
        found = glyphs::on_canvas(app, ui).map(|f| ((0..f.count()).filter_map(|i| f.gid(i)).collect(), f.more));
    });
    out.textures_delta.clear();
    found
}

/// A click at `at`, with a frame before it that lays the row out and moves the pointer there.
fn click(app: &mut VectorcraftApp, ctx: &egui::Context, at: Pos2) {
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(app, ctx, vec![egui::Event::PointerMoved(at)]);
    frame(app, ctx, vec![button(true), button(false)]);
}

/// The center of cell `i` of the row at `bar`.
fn cell(bar: Rect, i: usize) -> Pos2 {
    bar.min + vec2(PAD + CELL * (i as f32 + 0.5), PAD + CELL / 2.0)
}

/// The center of the arrow of a row of `count` cells at `bar`.
fn arrow(bar: Rect, count: usize) -> Pos2 {
    bar.min + vec2(PAD + CELL * count as f32 + MORE / 2.0, PAD + CELL / 2.0)
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

/// The screen box of bytes `a..b` of the text, as the canvas of [`frame`] draws it.
fn character_box(app: &VectorcraftApp, id: &Value, a: usize, b: usize) -> Rect {
    let NodeKind::Text(text) = &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind else { panic!("text") };
    let lay = vectorcraft_text::layout(FontDb::global(), text);
    let xf = app.canvas_xf().expect("a frame was drawn");
    let corners: Vec<Pos2> = vectorcraft_text::selection_quads(&lay, a, b).iter().flatten().map(|p| xf.to_screen(text.xf * *p)).collect();
    Rect::from_points(&corners)
}

/// With the Type tool and exactly one character selected, the canvas shows up to five of that
/// character's alternates beside it, without the glyph it is drawn with, and an arrow when it has
/// more. A click draws the character with the glyph as one undo step and keeps the text. A caret,
/// a longer selection, a character without alternates, another tool and the preference turned off
/// each show no row.
#[test]
fn one_selected_character_shows_its_alternates_and_a_click_replaces_its_glyph() {
    use vectorcraft_text::test_fonts::{alternates_family, alternates_font, unencoded_glyphs};
    FontDb::global().add_font(alternates_font(true).expect("the test font builds"));
    let [u1, ..] = unencoded_glyphs().expect("unencoded glyphs");
    let (mut app, id) = editing("xay", false);
    app.run("text.setRangeStyle", json!({"id": id, "font": alternates_family(true)})).unwrap();
    let ctx = egui::Context::default();
    let drawn = glyph_ids(&app, &id);

    select(&mut app, 1, 1);
    assert_eq!(frame(&mut app, &ctx, vec![]), None, "a caret selects no character");
    select(&mut app, 0, 2);
    assert_eq!(frame(&mut app, &ctx, vec![]), None, "two characters are selected");
    select(&mut app, 0, 1);
    assert_eq!(frame(&mut app, &ctx, vec![]), None, "x has no alternates");

    select(&mut app, 1, 2);
    let bar = frame(&mut app, &ctx, vec![]).expect("the a has alternates");
    let task_bar = super::super::task_bar_rect(&ctx).expect("the task bar shows for type being edited");
    let (gids, more) = offered(&mut app, &ctx).expect("offered");
    assert_eq!(gids.len(), glyphs::ON_CANVAS_ALTERNATES, "five of the six alternates");
    assert!(more, "the sixth is in the Glyphs panel");
    assert_eq!(gids[0], u1);
    assert!(!gids.contains(&drawn[1]), "the glyph the a is drawn with is not offered");
    assert_eq!(bar.size(), bar_size(5, true), "five cells and the arrow");
    // The Contextual Task Bar is under the type, so the row goes over the character.
    // The row is placed on whole pixels, so it is within one of where the character's box puts it.
    let sel = character_box(&app, &id, 1, 2);
    assert!(task_bar.intersects(Rect::from_min_size(pos2(bar.left(), sel.bottom() + GAP), bar.size())), "{task_bar:?} {sel:?}");
    assert!((sel.top() - bar.bottom() - GAP).abs() < 1.0, "over the character: {bar:?} {sel:?}");
    assert!((bar.center().x - sel.center().x).abs() < 1.0, "centered on it");
    // Without the task bar the row is under the character.
    app.ui.task_bar = false;
    frame(&mut app, &ctx, vec![]);
    let under = frame(&mut app, &ctx, vec![]).expect("the row");
    assert!((under.top() - sel.bottom() - GAP).abs() < 1.0, "under the character: {under:?} {sel:?}");
    app.ui.task_bar = true;
    frame(&mut app, &ctx, vec![]);
    assert_eq!(frame(&mut app, &ctx, vec![]), Some(bar));

    // A click on the first glyph.
    let undo = undo_steps(&app);
    click(&mut app, &ctx, cell(bar, 0));
    assert_eq!(undo_steps(&app), undo + 1, "one undo step; status: {}", app.ui.status);
    assert_eq!(glyph_ids(&app, &id), [drawn[0], u1, drawn[2]], "the a is drawn with the glyph");
    let NodeKind::Text(text) = &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind else { panic!("text") };
    assert_eq!(text.plain_text(), "xay", "the text keeps its characters");

    // The row stays for the same character: its default glyph first, the drawn glyph left out.
    let bar = frame(&mut app, &ctx, vec![]).expect("still one character selected");
    let (gids, _) = offered(&mut app, &ctx).expect("offered");
    assert_eq!(gids[0], drawn[1], "the default glyph comes first");
    assert!(!gids.contains(&u1));
    click(&mut app, &ctx, cell(bar, 0));
    assert_eq!(undo_steps(&app), undo + 2);
    assert_eq!(glyph_ids(&app, &id), drawn, "the default glyph again");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(glyph_ids(&app, &id), [drawn[0], u1, drawn[2]], "undo takes one pick back");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(glyph_ids(&app, &id), drawn);

    // Preferences › Type › Show Character Alternates.
    select(&mut app, 1, 2);
    assert!(frame(&mut app, &ctx, vec![]).is_some());
    app.run("prefs.set", json!({"key": "showCharacterAlternates", "value": false})).unwrap();
    assert_eq!(frame(&mut app, &ctx, vec![]), None, "the preference is off");
    app.run("prefs.set", json!({"key": "showCharacterAlternates", "value": true})).unwrap();
    assert!(frame(&mut app, &ctx, vec![]).is_some());

    app.session.select_tool("select", Default::default()).unwrap();
    assert_eq!(frame(&mut app, &ctx, vec![]), None, "the Selection tool edits no text");
}

/// The arrow at the end of the row opens the Glyphs panel on Show › Alternates for Current
/// Selection and changes nothing in the document.
#[test]
fn the_arrow_opens_the_glyphs_panel_on_the_alternates() {
    use vectorcraft_text::test_fonts::{alternates_family, alternates_font};
    FontDb::global().add_font(alternates_font(true).expect("the test font builds"));
    let (mut app, id) = editing("xay", false);
    app.run("text.setRangeStyle", json!({"id": id, "font": alternates_family(true)})).unwrap();
    select(&mut app, 1, 2);
    let ctx = egui::Context::default();
    app.run("window.panel.close", json!({"panel": "glyphs"})).ok();
    assert!(!crate::panel_docking::visible(&app.ui, "glyphs"));
    let bar = frame(&mut app, &ctx, vec![]).expect("the row");
    let (undo, drawn) = (undo_steps(&app), glyph_ids(&app, &id));
    click(&mut app, &ctx, arrow(bar, 5));
    assert!(crate::panel_docking::visible(&app.ui, "glyphs"), "status: {}", app.ui.status);
    // The Show menu's entry after its seven character subsets.
    assert_eq!(crate::panels::pstate::<usize>(&ctx, "gl-subset"), 7);
    assert_eq!(undo_steps(&app), undo);
    assert_eq!(glyph_ids(&app, &id), drawn);
}

/// A medial letter of a joined Arabic word gets the alternates of its medial form, and a click
/// applies the feature that draws one to that letter alone: its neighbors keep their joining
/// forms. Three alternates need no arrow.
#[test]
fn a_letter_of_a_joined_word_keeps_the_word_joined() {
    use vectorcraft_text::test_fonts::{JOINING_ALTERNATES_FAMILY, joining_alternates_font, source_sans_gid};
    FontDb::global().add_font(joining_alternates_font().expect("the test font builds"));
    let g = |c| u32::from(source_sans_gid(c).expect("a glyph"));
    let (mut app, id) = editing("\u{628}\u{62A}\u{646}", false);
    app.run("text.setRangeStyle", json!({"id": id, "font": JOINING_ALTERNATES_FAMILY})).unwrap();
    assert_eq!(glyph_ids(&app, &id), [g('B'), g('u'), g('M')]);
    select(&mut app, 2, 4);
    let ctx = egui::Context::default();
    let bar = frame(&mut app, &ctx, vec![]).expect("the medial teh has alternates");
    let (gids, more) = offered(&mut app, &ctx).expect("offered");
    assert_eq!((gids.len(), more), (3, false), "the medial form's three alternates: {gids:?}");
    assert_eq!(bar.size(), bar_size(3, false), "three cells and no arrow");
    let at = gids.iter().position(|gid| *gid == g('x')).expect("the alternate salt=2 draws");
    let undo = undo_steps(&app);
    click(&mut app, &ctx, cell(bar, at));
    assert_eq!(undo_steps(&app), undo + 1, "status: {}", app.ui.status);
    assert_eq!(glyph_ids(&app, &id), [g('B'), g('x'), g('M')], "the neighbors keep their initial and final forms");
    let NodeKind::Text(text) = &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind else { panic!("text") };
    assert_eq!(text.plain_text(), "\u{628}\u{62A}\u{646}");
    assert!(text.runs.iter().all(|r| r.style.glyph_alternate.is_none()), "a feature draws it, not a glyph choice");
}

/// The row stands on the right of a character of vertical type, level with it.
#[test]
fn the_row_stands_beside_a_character_of_vertical_type() {
    use vectorcraft_text::test_fonts::{alternates_family, alternates_font};
    FontDb::global().add_font(alternates_font(true).expect("the test font builds"));
    let (mut app, id) = editing("xay", true);
    app.run("text.setRangeStyle", json!({"id": id, "font": alternates_family(true)})).unwrap();
    select(&mut app, 1, 2);
    let ctx = egui::Context::default();
    app.ui.task_bar = false;
    // The first frame fits the view to the canvas.
    frame(&mut app, &ctx, vec![]);
    let bar = frame(&mut app, &ctx, vec![]).expect("the row");
    let sel = character_box(&app, &id, 1, 2);
    assert!((bar.left() - sel.right() - GAP).abs() < 1.0, "on the right of the character: {bar:?} {sel:?}");
    assert!((bar.center().y - sel.center().y).abs() < 1.0, "level with it");
}

/// [`place`]: under the character, over it when the canvas ends below or the task bar is there,
/// on the right or left of vertical type, and inside the canvas when neither side has room.
#[test]
fn the_row_is_placed_on_the_side_that_has_room() {
    let area = Rect::from_min_max(pos2(0.0, 0.0), pos2(400.0, 300.0));
    let size = vec2(100.0, 34.0);
    let sel = Rect::from_min_max(pos2(190.0, 100.0), pos2(210.0, 130.0));
    assert_eq!(place(sel, size, area, false, None), pos2(150.0, 136.0), "under");
    let low = sel.translate(vec2(0.0, 150.0));
    assert_eq!(place(low, size, area, false, None), pos2(150.0, 210.0), "over: no room under");
    let task_bar = Rect::from_min_max(pos2(100.0, 140.0), pos2(300.0, 180.0));
    assert_eq!(place(sel, size, area, false, Some(task_bar)), pos2(150.0, 60.0), "over: the task bar is under");
    let edge = sel.translate(vec2(-185.0, 0.0));
    assert_eq!(place(edge, size, area, false, None), pos2(0.0, 136.0), "moved along the line to stay on the canvas");
    assert_eq!(place(sel, size, area, true, None), pos2(216.0, 98.0), "vertical type: on the right");
    let right = sel.translate(vec2(150.0, 0.0));
    assert_eq!(place(right, size, area, true, None), pos2(234.0, 98.0), "vertical type: on the left");
    let small = Rect::from_min_max(pos2(0.0, 0.0), pos2(400.0, 60.0));
    let inside = Rect::from_min_max(pos2(190.0, 20.0), pos2(210.0, 40.0));
    assert_eq!(place(inside, size, small, false, None), pos2(150.0, 26.0), "no room on either side: kept inside the canvas");
}
