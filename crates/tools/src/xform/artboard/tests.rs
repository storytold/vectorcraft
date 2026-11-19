use super::*;
use crate::testutil::*;
use vectorcraft_doc::Selection;

fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
    PointerEvent::new(kind, x, y)
}

/// [`doc_with_rect`] (artboard 1 at 0–500, a rectangle at 100–200) plus Artboard 2 at x 600–800,
/// y 0–200.
fn two_boards() -> vectorcraft_doc::Document {
    let (mut d, _) = doc_with_rect();
    let mut ab = d.artboards[0].clone();
    ab.rect = Rect::new(600.0, 0.0, 800.0, 200.0);
    ab.name = "Artboard 2".into();
    d.artboards.push(ab);
    d
}

fn preview_params(a: &[Action]) -> Value {
    a.iter()
        .find_map(|a| match a {
            Action::Preview(_, p) => Some(p.clone()),
            _ => None,
        })
        .unwrap()
}

/// Drag artboard 2 from (700, 100) by (dx, dy) and return the preview's params.
fn drag_board2(cx: &ToolContext, t: &mut ArtboardTool, dx: f64, dy: f64) -> Value {
    t.pointer(cx, &ev(PointerKind::Down, 700.0, 100.0));
    preview_params(&t.pointer(cx, &ev(PointerKind::Drag, 700.0 + dx, 100.0 + dy)))
}

#[test]
fn artboard_resize_readout_honors_transform_tools_guides() {
    let d = two_boards();
    let (s, p) = (Selection::default(), paint());
    let measures = |t: &ArtboardTool, c: &ToolContext| t.overlays(c).iter().filter(|o| matches!(o, Overlay::Measure { .. })).count();
    let mut t = ArtboardTool { active: 1, ..Default::default() };
    for on in [true, false] {
        let c = ToolContext { transform_tools_guides: on, ..cx(&d, &s, &p) };
        t.pointer(&c, &ev(PointerKind::Down, 800.0, 100.0));
        t.pointer(&c, &ev(PointerKind::Drag, 820.0, 100.0));
        assert_eq!(measures(&t, &c), usize::from(on), "transform tools {on}");
        t.pointer(&c, &ev(PointerKind::Up, 820.0, 100.0));
    }
    let mut t = ArtboardTool::default();
    let create = ToolContext { measurement_labels: true, transform_tools_guides: false, ..cx(&d, &s, &p) };
    t.pointer(&create, &ev(PointerKind::Down, 900.0, 300.0));
    t.pointer(&create, &ev(PointerKind::Drag, 1000.0, 400.0));
    assert_eq!(measures(&t, &create), 1, "new artboard drag still uses measurement labels");
}

#[test]
fn bounds_follow_the_artboard_while_the_document_shows_the_preview() {
    let mut d = two_boards();
    let (s, p) = (Selection::default(), paint());
    let mut t = ArtboardTool::default();
    assert_eq!(drag_board2(&cx(&d, &s, &p), &mut t, 10.0, 0.0)["dx"], 10.0);
    // The engine applies the preview to the document, as it does during a drag.
    d.artboards[1].rect = Rect::new(610.0, 0.0, 810.0, 200.0);
    let cx = cx(&d, &s, &p);
    assert_eq!(preview_params(&t.pointer(&cx, &ev(PointerKind::Drag, 720.0, 100.0)))["dx"], 20.0);
    let corners: Vec<Point> = t
        .overlays(&cx)
        .iter()
        .filter_map(|o| match o {
            Overlay::Anchor { p, .. } => Some(*p),
            _ => None,
        })
        .collect();
    assert_eq!(corners[0], Point::new(620.0, 0.0), "the handles sit on the moved artboard, not 10 pt past it");
}

#[test]
fn moving_snaps_to_other_artboards_with_smart_guides() {
    let d = two_boards();
    let (s, p) = (Selection::default(), paint());
    let mut t = ArtboardTool::default();
    let mut c = cx(&d, &s, &p);
    // Left edge 3 pt right of artboard 1's right edge (500) → lands on it.
    assert_eq!(drag_board2(&c, &mut t, -97.0, 0.0)["dx"], -100.0);
    assert!(t.overlays(&c).iter().any(|o| matches!(o, Overlay::Line { color, .. } if *color == crate::guides::MAGENTA)));
    t.pointer(&c, &ev(PointerKind::Up, 603.0, 100.0));
    assert!(!t.overlays(&c).iter().any(|o| matches!(o, Overlay::Line { .. })), "guides go away on release");
    c.smart_guides = false;
    assert_eq!(drag_board2(&c, &mut t, -97.0, 0.0)["dx"], -97.0);
}

#[test]
fn moving_never_snaps_to_itself_or_the_art_it_carries() {
    let mut d = two_boards();
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let art = vectorcraft_geom::shapes::rectangle(Rect::new(650.0, 50.0, 660.0, 60.0));
    d.insert(Some(l), 0, vectorcraft_doc::Node::path(id, art, vectorcraft_doc::Appearance::default_art())).unwrap();
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    let mut t = ArtboardTool::default();
    // Its own old left edge (600) isn't a target.
    assert_eq!(drag_board2(&c, &mut t, 4.0, 0.0)["dx"], 4.0);
    t.pointer(&c, &ev(PointerKind::Up, 704.0, 100.0));
    // The rectangle moves along: its left edge (650) isn't a target either…
    assert_eq!(drag_board2(&c, &mut t, 52.0, 0.0)["dx"], 52.0);
    t.pointer(&c, &ev(PointerKind::Up, 752.0, 100.0));
    // …unless it stays behind.
    t.move_art = false;
    assert_eq!(drag_board2(&c, &mut t, 52.0, 0.0)["dx"], 50.0);
}

/// Art that is also on a locked artboard stays behind when its artboard moves, so the move
/// snaps to it, as to any art that stays.
#[test]
fn moving_snaps_to_the_art_a_locked_artboard_keeps() {
    let mut d = two_boards();
    let mut locked = d.artboards[0].clone();
    locked.rect = Rect::new(640.0, 40.0, 700.0, 70.0);
    locked.locked = true;
    d.artboards.push(locked);
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let art = vectorcraft_geom::shapes::rectangle(Rect::new(650.0, 50.0, 660.0, 60.0));
    d.insert(Some(l), 0, vectorcraft_doc::Node::path(id, art, vectorcraft_doc::Appearance::default_art())).unwrap();
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    let mut t = ArtboardTool::default();
    // Artboard 2's left edge, 600 + 52, snaps to the rectangle's at 650.
    assert_eq!(drag_board2(&c, &mut t, 52.0, 0.0)["dx"], 50.0);
}

#[test]
fn moving_snaps_to_grid_and_pixels() {
    let d = two_boards();
    let (s, p) = (Selection::default(), paint());
    let mut t = ArtboardTool::default();
    let mut c = cx(&d, &s, &p);
    c.snap_to_grid = true;
    // Grid step 72 / 8 = 9: x 610 → 612, y 4 → 0.
    let v = drag_board2(&c, &mut t, 10.0, 4.0);
    assert_eq!((v["dx"].as_f64(), v["dy"].as_f64()), (Some(12.0), Some(0.0)));
    t.pointer(&c, &ev(PointerKind::Up, 710.0, 104.0));
    c.snap_to_grid = false;
    c.snap_to_pixel = true;
    let v = drag_board2(&c, &mut t, 10.4, 3.6);
    assert_eq!((v["dx"].as_f64(), v["dy"].as_f64()), (Some(10.0), Some(4.0)));
}

#[test]
fn resizing_snaps_the_dragged_handle() {
    let d = two_boards();
    let (s, p) = (Selection::default(), paint());
    let mut t = ArtboardTool { active: 1, ..Default::default() };
    let c = cx(&d, &s, &p);
    // Left handle to 3 pt off artboard 1's right edge.
    t.pointer(&c, &ev(PointerKind::Down, 600.0, 100.0));
    let v = preview_params(&t.pointer(&c, &ev(PointerKind::Drag, 503.0, 100.0)));
    assert_eq!((v["x"].as_f64(), v["width"].as_f64()), (Some(500.0), Some(300.0)));
    t.pointer(&c, &ev(PointerKind::Up, 503.0, 100.0));
    // The handle, not the pointer, follows: grabbed 2 pt left of it, dragged 10 pt.
    t.pointer(&c, &ev(PointerKind::Down, 798.0, 100.0));
    let v = preview_params(&t.pointer(&c, &ev(PointerKind::Drag, 808.0, 100.0)));
    assert_eq!(v["width"].as_f64(), Some(210.0));
}

/// In a view turned 90° the handles' cursors turn with the view.
#[test]
fn handle_cursors_turn_with_the_view() {
    let d = two_boards();
    let (s, p) = (Selection::default(), paint());
    let t = ArtboardTool { active: 1, ..Default::default() };
    let mut c = cx(&d, &s, &p);
    let (top, right) = (Point::new(700.0, 0.0), Point::new(800.0, 100.0));
    assert_eq!(t.cursor(&c, top, Mods::default()), Cursor::ResizeV);
    c.screen = turned_view(90.0);
    assert_eq!(t.cursor(&c, top, Mods::default()), Cursor::ResizeH);
    assert_eq!(t.cursor(&c, right, Mods::default()), Cursor::ResizeV);
}

/// Shift-click selects several artboards (#1139): a drag moves them all, Shift-click takes one
/// out again, a plain click on an unselected one selects just it, and Delete removes all the
/// selected ones (one artboard is always left).
#[test]
fn shift_click_selects_several_artboards_that_move_and_delete_together() {
    let mut d = two_boards();
    let mut third = d.artboards[0].clone();
    third.rect = Rect::new(0.0, 600.0, 200.0, 800.0);
    d.artboards.push(third);
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    let shift = Mods { shift: true, ..Default::default() };
    let click = |t: &mut ArtboardTool, x: f64, y: f64, m: Mods| {
        t.pointer(&c, &ev(PointerKind::Down, x, y).with_mods(m));
        t.pointer(&c, &ev(PointerKind::Up, x, y).with_mods(m));
    };
    let mut t = ArtboardTool::default();
    click(&mut t, 450.0, 450.0, Mods::default());
    click(&mut t, 700.0, 100.0, shift);
    assert_eq!(t.options()["selected"], json!([1, 0]), "the Shift-clicked one is active");
    // A drag from either moves both.
    t.pointer(&c, &ev(PointerKind::Down, 450.0, 450.0));
    let v = preview_params(&t.pointer(&c, &ev(PointerKind::Drag, 470.0, 450.0)));
    assert_eq!((v["indices"].clone(), v["dx"].as_f64()), (json!([0, 1]), Some(20.0)));
    assert_eq!(t.pointer(&c, &ev(PointerKind::Up, 470.0, 450.0)), vec![Action::Commit]);
    // Delete removes both; with all three selected the keys retain the original first board.
    assert_eq!(t.key(&c, ToolKey::Delete, Mods::default()), vec![Action::Exec("artboard.delete".into(), json!({"indices": [0, 1]}))]);
    for key in [ToolKey::Delete, ToolKey::Backspace] {
        t.set_option("selected", &json!([2, 0, 1]));
        assert!(t.claims_key(&c, key));
        assert_eq!(t.key(&c, key, Mods::default()), vec![Action::Exec("artboard.delete".into(), json!({"indices": [1, 2]}))]);
        assert_eq!(t.options()["selected"], json!([0]));
    }
    t.set_option("selected", &json!([0, 1, 2]));
    // Shift-click takes one out; a plain click on an unselected one selects just it.
    click(&mut t, 100.0, 700.0, shift);
    assert_eq!(t.options()["selected"], json!([0, 1]));
    click(&mut t, 100.0, 700.0, Mods::default());
    assert_eq!(t.options()["selected"], json!([2]));
}

/// The arrow keys move the selected artboards by the keyboard increment, Shift ten times as far,
/// with their art when Move Artwork with Artboard is on (#2103): the tool claims them, so they
/// no longer fall through to object.nudge and move only the selected art. A locked artboard
/// stays, and during a drag the arrows do nothing.
#[test]
fn arrow_keys_nudge_the_selected_artboards() {
    let mut d = two_boards();
    let mut third = d.artboards[0].clone();
    third.rect = Rect::new(0.0, 600.0, 200.0, 800.0);
    third.locked = true;
    d.artboards.push(third);
    let (s, p) = (Selection::default(), paint());
    let mut c = cx(&d, &s, &p);
    c.keyboard_increment = 2.0;
    let shift = Mods { shift: true, ..Default::default() };
    let mut t = ArtboardTool { active: 1, ..Default::default() };
    let moved = |p: Value| vec![Action::Exec("artboard.move".into(), p)];
    for key in [ToolKey::Left, ToolKey::Right, ToolKey::Up, ToolKey::Down] {
        assert!(t.claims_key(&c, key), "{key:?}");
    }
    assert_eq!(t.key(&c, ToolKey::Right, Mods::default()), moved(json!({"index": 1, "dx": 2.0, "dy": 0.0, "moveArt": true})));
    assert_eq!(t.key(&c, ToolKey::Up, shift), moved(json!({"index": 1, "dx": 0.0, "dy": -20.0, "moveArt": true})));
    t.move_art = false;
    t.set_option("selected", &json!([0, 1, 2]));
    assert_eq!(
        t.key(&c, ToolKey::Left, Mods::default()),
        moved(json!({"indices": [0, 1], "dx": -2.0, "dy": 0.0, "moveArt": false})),
        "the locked artboard stays"
    );
    t.set_option("selected", &json!([2]));
    assert_eq!(t.key(&c, ToolKey::Down, Mods::default()), vec![], "only a locked artboard selected");
    t.set_option("selected", &json!([1]));
    t.pointer(&c, &ev(PointerKind::Down, 700.0, 100.0));
    assert_eq!(t.key(&c, ToolKey::Down, Mods::default()), vec![], "not during a drag");
}

/// A click selects a locked artboard, but a drag neither moves it nor copies it (Alt), a press
/// on its edge doesn't resize it, it shows no handles, the pointer over it is the arrow, and
/// Delete leaves it. With an unlocked artboard selected too, a drag and Delete act on that one.
/// With Shift the pointer over it is the crosshair, a Shift-click toggles it, and a Shift-drag
/// inside it draws a new artboard, as over any artboard.
#[test]
fn a_locked_artboard_is_not_moved_resized_or_deleted() {
    let mut d = two_boards();
    d.artboards[1].locked = true;
    let (s, p) = (Selection::default(), paint());
    let mut c = cx(&d, &s, &p);
    c.smart_guides = false;
    let mut t = ArtboardTool::default();
    for m in [Mods::default(), Mods { alt: true, ..Default::default() }] {
        t.pointer(&c, &ev(PointerKind::Down, 700.0, 100.0).with_mods(m));
        assert!(t.pointer(&c, &ev(PointerKind::Drag, 750.0, 100.0).with_mods(m)).is_empty());
        assert!(t.pointer(&c, &ev(PointerKind::Up, 750.0, 100.0).with_mods(m)).is_empty());
    }
    assert_eq!(t.options()["selected"], json!([1]), "the click selects it");
    assert!(!t.overlays(&c).iter().any(|o| matches!(o, Overlay::Anchor { .. })), "no handles");
    // A press on its left edge, where a handle would be.
    t.pointer(&c, &ev(PointerKind::Down, 600.0, 100.0));
    assert!(t.pointer(&c, &ev(PointerKind::Drag, 550.0, 100.0)).is_empty());
    t.pointer(&c, &ev(PointerKind::Up, 550.0, 100.0));
    for x in [600.0, 700.0] {
        assert_eq!(t.cursor(&c, Point::new(x, 100.0), Mods::default()), Cursor::Arrow);
    }
    assert!(t.key(&c, ToolKey::Delete, Mods::default()).is_empty());
    t.set_option("selected", &json!([0, 1]));
    t.pointer(&c, &ev(PointerKind::Down, 450.0, 450.0));
    assert_eq!(preview_params(&t.pointer(&c, &ev(PointerKind::Drag, 470.0, 450.0)))["indices"], json!([0]));
    t.pointer(&c, &ev(PointerKind::Up, 470.0, 450.0));
    t.set_option("selected", &json!([0, 1]));
    assert_eq!(t.key(&c, ToolKey::Delete, Mods::default()), vec![Action::Exec("artboard.delete".into(), json!({"index": 0}))]);
    let shift = Mods { shift: true, ..Default::default() };
    assert_eq!(t.cursor(&c, Point::new(700.0, 100.0), shift), Cursor::Crosshair);
    t.set_option("selected", &json!([0, 1]));
    t.pointer(&c, &ev(PointerKind::Down, 700.0, 100.0).with_mods(shift));
    assert!(t.pointer(&c, &ev(PointerKind::Up, 700.0, 100.0).with_mods(shift)).is_empty());
    assert_eq!(t.options()["selected"], json!([0]), "the Shift-click takes it out");
    t.pointer(&c, &ev(PointerKind::Down, 650.0, 50.0).with_mods(shift));
    assert!(t.pointer(&c, &ev(PointerKind::Drag, 690.0, 80.0).with_mods(shift)).is_empty(), "no move preview");
    assert_eq!(t.pointer(&c, &ev(PointerKind::Up, 690.0, 80.0).with_mods(shift)), create_artboard(Rect::new(650.0, 50.0, 690.0, 80.0)));
}

/// A Shift-click toggles its artboard when the button comes up, so the press alone leaves the
/// selection as it was.
#[test]
fn a_shift_click_toggles_its_artboard_on_release() {
    let d = two_boards();
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    let shift = Mods { shift: true, ..Default::default() };
    let mut t = ArtboardTool::default();
    assert_eq!(t.pointer(&c, &ev(PointerKind::Down, 700.0, 100.0).with_mods(shift)), vec![]);
    assert_eq!(t.options()["selected"], json!([0]), "the press alone selects nothing");
    assert_eq!(t.pointer(&c, &ev(PointerKind::Up, 701.0, 100.0).with_mods(shift)), vec![]);
    assert_eq!(t.options()["selected"], json!([1, 0]));
    for kind in [PointerKind::Down, PointerKind::Up] {
        t.pointer(&c, &ev(kind, 700.0, 100.0).with_mods(shift));
    }
    assert_eq!(t.options()["selected"], json!([0]), "a second Shift-click takes it out");
}

/// Shift-drag inside an artboard draws a new artboard there, neither square nor moving the
/// artboard it starts in, and the cursor shows a crosshair while Shift is held.
#[test]
fn a_shift_drag_inside_an_artboard_draws_a_new_one() {
    let d = two_boards();
    let (s, p) = (Selection::default(), paint());
    let mut c = cx(&d, &s, &p);
    c.smart_guides = false;
    let shift = Mods { shift: true, ..Default::default() };
    let mut t = ArtboardTool::default();
    assert_eq!(t.cursor(&c, Point::new(300.0, 300.0), Mods::default()), Cursor::Move);
    assert_eq!(t.cursor(&c, Point::new(300.0, 300.0), shift), Cursor::Crosshair);
    assert_eq!(t.pointer(&c, &ev(PointerKind::Down, 300.0, 300.0).with_mods(shift)), vec![]);
    assert_eq!(t.pointer(&c, &ev(PointerKind::Drag, 420.0, 380.0).with_mods(shift)), vec![], "no move preview");
    assert!(matches!(t.overlays(&c)[0], Overlay::Marquee(_)));
    let a = t.pointer(&c, &ev(PointerKind::Up, 420.0, 380.0).with_mods(shift));
    assert_eq!(a, create_artboard(Rect::new(300.0, 300.0, 420.0, 380.0)));
    assert_eq!(t.options()["selected"], json!([0]), "the artboard drawn in is not toggled");
}

/// Shift-drag on the pasteboard is a marquee: the artboards it touches leave the selection
/// or join it, the last one joining becoming active, and no artboard is drawn.
#[test]
fn a_shift_drag_on_the_pasteboard_selects_the_artboards_it_touches() {
    let mut d = two_boards();
    let mut third = d.artboards[0].clone();
    third.rect = Rect::new(0.0, 600.0, 200.0, 800.0);
    d.artboards.push(third);
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    let shift = Mods { shift: true, ..Default::default() };
    let mut t = ArtboardTool::default();
    let marquee = |t: &mut ArtboardTool, from: (f64, f64), to: (f64, f64)| {
        assert_eq!(t.pointer(&c, &ev(PointerKind::Down, from.0, from.1).with_mods(shift)), vec![]);
        assert_eq!(t.pointer(&c, &ev(PointerKind::Drag, to.0, to.1).with_mods(shift)), vec![]);
        assert!(t.overlays(&c).iter().any(|o| matches!(o, Overlay::Marquee(_))));
        t.pointer(&c, &ev(PointerKind::Up, to.0, to.1).with_mods(shift))
    };
    // From between the artboards across Artboard 1 (selected) and Artboard 3: 1 leaves, 3 joins.
    assert_eq!(marquee(&mut t, (550.0, 150.0), (150.0, 650.0)), vec![]);
    assert_eq!(t.options()["selected"], json!([2]));
    // Right of Artboard 2 across its corner: it joins and becomes active.
    assert_eq!(marquee(&mut t, (850.0, -50.0), (700.0, 100.0)), vec![]);
    assert_eq!(t.options()["selected"], json!([1, 2]));
    // A Shift-click on the pasteboard changes nothing.
    assert_eq!(marquee(&mut t, (550.0, 150.0), (551.0, 150.0)), vec![]);
    assert_eq!(t.options()["selected"], json!([1, 2]));
}

/// A drag begun without Shift still moves the artboard, and on the pasteboard still draws one;
/// Shift pressed during either no longer keeps the move at 45° or the new artboard square.
#[test]
fn a_plain_drag_still_moves_and_shift_no_longer_constrains_it() {
    let d = two_boards();
    let (s, p) = (Selection::default(), paint());
    let mut c = cx(&d, &s, &p);
    c.smart_guides = false;
    let shift = Mods { shift: true, ..Default::default() };
    let mut t = ArtboardTool::default();
    t.pointer(&c, &ev(PointerKind::Down, 700.0, 100.0));
    let v = preview_params(&t.pointer(&c, &ev(PointerKind::Drag, 730.0, 110.0).with_mods(shift)));
    assert_eq!((v["index"].as_u64(), v["dx"].as_f64(), v["dy"].as_f64()), (Some(1), Some(30.0), Some(10.0)));
    assert_eq!(t.pointer(&c, &ev(PointerKind::Up, 730.0, 110.0).with_mods(shift)), vec![Action::Commit]);
    t.pointer(&c, &ev(PointerKind::Down, 1000.0, 1000.0));
    t.pointer(&c, &ev(PointerKind::Drag, 1100.0, 1050.0).with_mods(shift));
    let a = t.pointer(&c, &ev(PointerKind::Up, 1100.0, 1050.0).with_mods(shift));
    assert_eq!(a, create_artboard(Rect::new(1000.0, 1000.0, 1100.0, 1050.0)));
}

/// Escape during a marquee or while a new artboard is being drawn ends the gesture without
/// selecting or drawing anything and keeps the tool; the next Escape leaves it.
#[test]
fn escape_cancels_a_marquee_or_a_new_artboard_and_keeps_the_tool() {
    let d = two_boards();
    let (s, p) = (Selection::default(), paint());
    let mut c = cx(&d, &s, &p);
    c.smart_guides = false;
    let shift = Mods { shift: true, ..Default::default() };
    let mut t = ArtboardTool::default();
    // A Shift-drag marquee on the pasteboard, a Shift-drag inside Artboard 1 and a plain drag on
    // the pasteboard.
    let drags =
        [((850.0, -50.0), (700.0, 100.0), shift), ((300.0, 300.0), (420.0, 380.0), shift), ((1000.0, 1000.0), (1100.0, 1050.0), Mods::default())];
    for (from, to, mods) in drags {
        t.pointer(&c, &ev(PointerKind::Down, from.0, from.1).with_mods(mods));
        t.pointer(&c, &ev(PointerKind::Drag, to.0, to.1).with_mods(mods));
        assert_eq!(t.key(&c, ToolKey::Escape, Mods::default()), vec![], "Escape from {from:?} only ends the gesture");
        assert!(!t.busy());
        assert_eq!(t.pointer(&c, &ev(PointerKind::Up, to.0, to.1).with_mods(mods)), vec![], "the release draws nothing");
        assert_eq!(t.options()["selected"], json!([0]), "the release selects nothing");
    }
    assert_eq!(t.key(&c, ToolKey::Escape, Mods::default()), vec![Action::SwitchTool("selection".into())]);
}

/// Scale Artwork with Artboard (#602): the resize asks for the art to scale, keeps the artboard's
/// proportions, and never snaps to the art scaling with it.
#[test]
fn scale_art_resizes_proportionally_without_snapping_to_its_art() {
    let mut d = two_boards();
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let art = vectorcraft_geom::shapes::rectangle(Rect::new(700.0, 50.0, 710.0, 60.0));
    d.insert(Some(l), 0, vectorcraft_doc::Node::path(id, art, vectorcraft_doc::Appearance::default_art())).unwrap();
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    // Off: the right handle 3 pt past the art's right edge (710) lands on it; the height stays.
    let mut t = ArtboardTool { active: 1, ..Default::default() };
    t.pointer(&c, &ev(PointerKind::Down, 800.0, 100.0));
    let v = preview_params(&t.pointer(&c, &ev(PointerKind::Drag, 713.0, 100.0)));
    assert_eq!((v["width"].as_f64(), v["height"].as_f64(), v.get("scaleArt")), (Some(110.0), Some(200.0), None));
    t.pointer(&c, &ev(PointerKind::Up, 713.0, 100.0));
    // On: the art scales along, so it's no target, and the height follows the width.
    t.set_option("scaleArt", &json!(true));
    assert_eq!(t.options()["scaleArt"], true);
    t.pointer(&c, &ev(PointerKind::Down, 800.0, 100.0));
    let v = preview_params(&t.pointer(&c, &ev(PointerKind::Drag, 713.0, 100.0)));
    assert_eq!((v["width"].as_f64(), v["scaleArt"].as_bool()), (Some(113.0), Some(true)));
    assert!((v["height"].as_f64().unwrap() - 113.0).abs() < 1e-9);
}

/// Over an artboard's right or bottom edge the cursor shows Move and a click selects that
/// artboard, as on its left or top edge.
#[test]
fn a_click_on_an_artboards_right_or_bottom_edge_selects_it() {
    let d = two_boards();
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    for at in [Point::new(800.0, 100.0), Point::new(700.0, 200.0)] {
        let mut t = ArtboardTool::default();
        assert_eq!(t.cursor(&c, at, Mods::default()), Cursor::Move, "{at:?}");
        t.pointer(&c, &ev(PointerKind::Down, at.x, at.y));
        t.pointer(&c, &ev(PointerKind::Up, at.x, at.y));
        assert_eq!(t.options()["selected"], json!([1]), "{at:?}");
    }
}

/// A proportional resize never asks for a side under 1 pt, which the command would lengthen
/// alone (scaling the art unevenly): both sides grow by one factor instead.
#[test]
fn a_proportional_resize_keeps_its_proportions_at_the_smallest_size() {
    let mut d = two_boards();
    d.artboards[1].rect = Rect::new(600.0, 0.0, 800.0, 20.0);
    let (s, p) = (Selection::default(), paint());
    let mut c = cx(&d, &s, &p);
    c.smart_guides = false;
    let mut t = ArtboardTool { active: 1, scale_art: true, ..Default::default() };
    // The right handle to 2.5 % of the width: 5 × 0.5, grown to 10 × 1 about the left middle.
    t.pointer(&c, &ev(PointerKind::Down, 800.0, 10.0));
    let v = preview_params(&t.pointer(&c, &ev(PointerKind::Drag, 605.0, 10.0)));
    let got = ["x", "y", "width", "height"].map(|k| v[k].as_f64().unwrap());
    assert!(got.iter().zip([600.0, 9.5, 10.0, 1.0]).all(|(a, b)| (a - b).abs() < 1e-9), "{got:?}");
}

#[test]
fn bleed_edges_snap_to_bleed_edges() {
    let mut d = two_boards();
    d.setup.bleed = [10.0; 4];
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    let mut t = ArtboardTool::default();
    // Artboard 2's bleed (left edge 590) lands on artboard 1's bleed (right edge 510): dragged
    // 77 pt left it's 3 pt off, nothing else is within reach.
    assert_eq!(drag_board2(&c, &mut t, -77.0, 0.0)["dx"], -80.0);
    t.pointer(&c, &ev(PointerKind::Up, 623.0, 100.0));
    // Resizing: the left handle's bleed edge snaps the same way.
    t.pointer(&c, &ev(PointerKind::Down, 600.0, 100.0));
    let v = preview_params(&t.pointer(&c, &ev(PointerKind::Drag, 523.0, 100.0)));
    assert_eq!((v["x"].as_f64(), v["width"].as_f64()), (Some(520.0), Some(280.0)));
}

#[test]
fn move_resize_create_delete() {
    let (mut d, _) = doc_with_rect();
    let mut ab = d.artboards[0].clone();
    ab.rect = Rect::new(600.0, 0.0, 800.0, 200.0);
    ab.name = "Artboard 2".into();
    d.artboards.push(ab);
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = ArtboardTool::default();
    // Click the second artboard → active; drag moves it.
    t.pointer(&cx, &ev(PointerKind::Down, 700.0, 100.0));
    assert_eq!(t.active, 1);
    let a = t.pointer(&cx, &ev(PointerKind::Drag, 710.0, 120.0));
    assert_eq!(a[0], Action::Begin("Move Artboard".into()));
    assert_eq!(a[1], Action::Preview("artboard.move".into(), json!({"index": 1, "dx": 10.0, "dy": 20.0, "moveArt": true, "copy": false})));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 710.0, 120.0)), vec![Action::Commit]);
    assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "02 - Artboard 2")));
    // Handle drag resizes.
    t.pointer(&cx, &ev(PointerKind::Down, 800.0, 100.0));
    let a = t.pointer(&cx, &ev(PointerKind::Drag, 900.0, 100.0));
    assert_eq!(a[1], Action::Preview("artboard.setProps".into(), json!({"index": 1, "x": 600.0, "y": 0.0, "width": 300.0, "height": 200.0})));
    t.pointer(&cx, &ev(PointerKind::Up, 900.0, 100.0));
    // Drag on the pasteboard creates one.
    t.pointer(&cx, &ev(PointerKind::Down, 1000.0, 1000.0));
    t.pointer(&cx, &ev(PointerKind::Drag, 1100.0, 1050.0));
    assert!(matches!(t.overlays(&cx)[0], Overlay::Marquee(_)));
    let a = t.pointer(&cx, &ev(PointerKind::Up, 1100.0, 1050.0));
    assert_eq!(a[0], Action::Exec("artboard.new".into(), json!({"x": 1000.0, "y": 1000.0, "width": 100.0, "height": 50.0})));
    assert_eq!(a[1], Action::Notify("created".into()));
    // Delete removes the active artboard, ahead of the Clear shortcut.
    assert!(t.claims_key(&cx, ToolKey::Delete) && t.claims_key(&cx, ToolKey::Backspace));
    assert_eq!(t.key(&cx, ToolKey::Delete, Mods::default()), vec![Action::Exec("artboard.delete".into(), json!({"index": 1}))]);
    assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![Action::SwitchTool("selection".into())]);
}

#[test]
fn an_alt_drag_copy_snaps_to_the_artboard_it_leaves_behind() {
    let d = two_boards();
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    let alt = Mods { alt: true, ..Mods::default() };
    let mut t = ArtboardTool::default();
    // A copy of artboard 2 dragged 203 pt right: its left edge 3 pt off the original's right
    // edge (800), which stays put and is a target.
    t.pointer(&c, &ev(PointerKind::Down, 700.0, 100.0).with_mods(alt));
    let v = preview_params(&t.pointer(&c, &ev(PointerKind::Drag, 903.0, 100.0).with_mods(alt)));
    assert_eq!((v["dx"].as_f64(), v["copy"].as_bool()), (Some(200.0), Some(true)));
    // Released Alt: a plain move, and the artboard's own old edge is no target.
    let v = preview_params(&t.pointer(&c, &ev(PointerKind::Drag, 903.0, 100.0)));
    assert_eq!((v["dx"].as_f64(), v["copy"].as_bool()), (Some(203.0), Some(false)));
}

#[test]
fn alt_drag_moves_a_copy_of_the_artboard() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let alt = Mods { alt: true, ..Mods::default() };
    let moved =
        |dx: f64, copy: bool| Action::Preview("artboard.move".into(), json!({"index": 0, "dx": dx, "dy": 0.0, "moveArt": true, "copy": copy}));
    let mut t = ArtboardTool::default();
    t.pointer(&cx, &ev(PointerKind::Down, 100.0, 100.0).with_mods(alt));
    let a = t.pointer(&cx, &ev(PointerKind::Drag, 150.0, 100.0).with_mods(alt));
    assert_eq!(a, vec![Action::Begin("Duplicate Artboard".into()), moved(50.0, true)]);
    // Alt released mid-drag: a plain move after all, and back.
    let a = t.pointer(&cx, &ev(PointerKind::Drag, 160.0, 100.0));
    assert_eq!(a, vec![Action::Cancel, Action::Begin("Move Artboard".into()), moved(60.0, false)]);
    let a = t.pointer(&cx, &ev(PointerKind::Drag, 170.0, 100.0).with_mods(alt));
    assert_eq!(a, vec![Action::Cancel, Action::Begin("Duplicate Artboard".into()), moved(70.0, true)]);
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 170.0, 100.0).with_mods(alt)), vec![Action::Commit]);
}

/// The sides of the active artboard that show an add button.
fn button_sides(t: &ArtboardTool, c: &ToolContext) -> Vec<ArtboardSide> {
    t.edge_buttons(c).into_iter().map(|(side, _)| side).collect()
}

/// The rings the add buttons draw.
fn button_rings(t: &ArtboardTool, c: &ToolContext) -> usize {
    t.overlays(c).iter().filter(|o| matches!(o, Overlay::Path { dashed: false, .. })).count()
}

#[test]
fn add_buttons_show_only_on_sides_with_room_for_an_artboard_of_the_same_size() {
    let mut d = two_boards();
    let (s, p) = (Selection::default(), paint());
    let t = ArtboardTool::default();
    // Artboard 1 (0–500): a copy on its right (520–1020) would overlap artboard 2 (600–800).
    let c = cx(&d, &s, &p);
    assert_eq!(button_sides(&t, &c), vec![ArtboardSide::Top, ArtboardSide::Bottom, ArtboardSide::Left]);
    assert_eq!(button_rings(&t, &c), 3);
    // Artboard 2 (600–800 × 0–200): its left would overlap artboard 1, the rest has room.
    let t2 = ArtboardTool { active: 1, ..Default::default() };
    assert_eq!(button_sides(&t2, &c), vec![ArtboardSide::Top, ArtboardSide::Right, ArtboardSide::Bottom]);
    // At the canvas's edge there is no room past it.
    let m = vectorcraft_doc::MAX_COORD;
    d.artboards[1].rect = Rect::new(m - 200.0, 0.0, m, 200.0);
    let c = cx(&d, &s, &p);
    assert_eq!(button_sides(&t2, &c), vec![ArtboardSide::Top, ArtboardSide::Bottom, ArtboardSide::Left]);
    // The buttons sit just outside the middle of each edge.
    let off = c.tol(EDGE_BUTTON_OFFSET);
    assert_eq!(t.edge_buttons(&c)[0].1, Point::new(250.0, -off));
}

#[test]
fn add_buttons_hide_with_several_artboards_selected_or_during_a_drag() {
    let d = two_boards();
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    let t = ArtboardTool { active: 1, others: vec![0], ..Default::default() };
    assert!(button_sides(&t, &c).is_empty());
    assert_eq!(button_rings(&t, &c), 0);
    let off = c.tol(EDGE_BUTTON_OFFSET);
    assert_eq!(t.edge_button_at(&c, Point::new(700.0, 200.0 + off)), None, "no button takes a press");
    let mut t = ArtboardTool::default();
    t.pointer(&c, &ev(PointerKind::Down, 700.0, 100.0));
    t.pointer(&c, &ev(PointerKind::Drag, 720.0, 100.0));
    assert!(button_sides(&t, &c).is_empty(), "none while an artboard is dragged");
}

/// The tooltips the add buttons carry: each one's center, radius and text.
fn button_tips(t: &ArtboardTool, c: &ToolContext) -> Vec<(Point, f64, &'static str)> {
    t.overlays(c)
        .into_iter()
        .filter_map(|o| if let Overlay::HoverText { center, radius, text } = o { Some((center, radius, text)) } else { None })
        .collect()
}

#[test]
fn each_add_button_has_a_tooltip_over_the_area_a_press_takes() {
    let mut d = two_boards();
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    let t = ArtboardTool::default();
    let radius = c.tol(EDGE_BUTTON_RADIUS);
    let want: Vec<_> = t.edge_buttons(&c).into_iter().map(|(_, at)| (at, radius, EDGE_BUTTON_TIP)).collect();
    assert_eq!(want.len(), 3, "no button and no tooltip on the side without room");
    assert_eq!(button_tips(&t, &c), want);
    // The tooltip's disc is the one a press takes.
    for (at, r, _) in &want {
        assert!(t.edge_button_at(&c, *at + Vec2::new(r * 0.99, 0.0)).is_some());
        assert!(t.edge_button_at(&c, *at + Vec2::new(r * 1.01, 0.0)).is_none());
    }
    // None with several artboards selected or during a drag.
    let several = ArtboardTool { active: 1, others: vec![0], ..Default::default() };
    assert!(button_tips(&several, &c).is_empty());
    let mut dragging = ArtboardTool::default();
    dragging.pointer(&c, &ev(PointerKind::Down, 700.0, 100.0));
    dragging.pointer(&c, &ev(PointerKind::Drag, 720.0, 100.0));
    assert!(button_tips(&dragging, &c).is_empty(), "none while an artboard is dragged");
    // A locked artboard's buttons tell only what a click does: an Alt-click does nothing there.
    d.artboards[0].locked = true;
    let c = cx(&d, &s, &p);
    let tips = button_tips(&t, &c);
    assert_eq!(tips.len(), 3);
    assert!(tips.iter().all(|(_, _, text)| *text == EDGE_BUTTON_TIP_LOCKED), "{tips:?}");
}

#[test]
fn an_add_button_adds_an_empty_artboard_and_alt_duplicates_on_that_side() {
    let d = two_boards();
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    let off = c.tol(EDGE_BUTTON_OFFSET);
    let below = Point::new(250.0, 500.0 + off);
    let mut t = ArtboardTool::default();
    assert_eq!(t.cursor(&c, below, Mods::default()), Cursor::Arrow);
    let a = t.pointer(&c, &ev(PointerKind::Down, below.x, below.y));
    assert_eq!(a, vec![Action::Exec("artboard.new".into(), json!({"beside": 0, "side": "bottom"})), Action::Notify("created".into())]);
    // The release that follows adds nothing more.
    assert_eq!(t.pointer(&c, &ev(PointerKind::Up, below.x, below.y)), vec![]);
    let alt = Mods { alt: true, ..Mods::default() };
    let left = Point::new(-off, 250.0);
    let a = t.pointer(&c, &ev(PointerKind::Down, left.x, left.y).with_mods(alt));
    assert_eq!(
        a,
        vec![Action::Exec("artboard.duplicate".into(), json!({"index": 0, "side": "left", "art": true})), Action::Notify("created".into())]
    );
    // The new artboard, last in the document, becomes the active one.
    let mut d3 = d.clone();
    d3.artboards.push(d.artboards[0].clone());
    t.notify(&cx(&d3, &s, &p), "created");
    assert_eq!((t.active, t.others.clone()), (2, vec![]));
}

/// On a locked artboard, a click on an add button still adds an empty artboard beside it, but an
/// Alt-click does nothing: the tool doesn't copy a locked artboard.
#[test]
fn an_alt_click_on_a_locked_artboards_add_button_does_not_duplicate_it() {
    let mut d = two_boards();
    d.artboards[0].locked = true;
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    let off = c.tol(EDGE_BUTTON_OFFSET);
    let left = Point::new(-off, 250.0);
    let mut t = ArtboardTool::default();
    let alt = Mods { alt: true, ..Mods::default() };
    assert_eq!(t.pointer(&c, &ev(PointerKind::Down, left.x, left.y).with_mods(alt)), vec![]);
    assert_eq!(t.pointer(&c, &ev(PointerKind::Up, left.x, left.y).with_mods(alt)), vec![]);
    assert_eq!((t.active, t.others.clone()), (0, vec![]));
    let a = t.pointer(&c, &ev(PointerKind::Down, left.x, left.y));
    assert_eq!(a, vec![Action::Exec("artboard.new".into(), json!({"beside": 0, "side": "left"})), Action::Notify("created".into())]);
}

/// Pixel snapping is per operation: moving-only pixels must not suppress the
/// ordinary Artboard resize's enabled stroke-edge targets.
#[test]
fn resizing_to_stroke_edges_uses_scaling_pixel_option_not_moving() {
    let (mut d, id) = doc_with_rect();
    d.node_mut(id).unwrap().appearance.stroke_mut().unwrap().width = 10.0;
    d.artboards[0].rect = Rect::new(300.0, 100.0, 400.0, 200.0);
    let (selection, paint) = (Selection::default(), paint());
    for (scaling, expected) in [(false, 205.0), (true, 207.0)] {
        let mut c = cx(&d, &selection, &paint);
        c.smart_guides = true;
        c.snap_to_stroke_edges = true;
        c.snapping_tolerance = 4.0;
        c.snap_to_grid = false;
        c.snap_to_pixel = true;
        c.pixel_snap.moving = true;
        c.pixel_snap.scaling = scaling;
        c.pixel_snap.drawing = false;
        let mut tool = ArtboardTool::default();
        tool.pointer(&c, &ev(PointerKind::Down, 300.0, 150.0));
        let actions = tool.pointer(&c, &ev(PointerKind::Drag, 207.0, 150.0));
        assert!(
            actions.iter().any(|a| matches!(a,Action::Preview(id,_) if id=="artboard.setProps")),
            "the real pointer gesture must resize, not move/create"
        );
        let v = preview_params(&actions);
        assert_eq!(v["x"].as_f64(), Some(expected), "scaling pixels={scaling}");
        assert_eq!(v["width"].as_f64(), Some(400.0 - expected));
        assert_eq!((v["y"].as_f64(), v["height"].as_f64()), (Some(100.0), Some(100.0)));
        assert!(matches!(tool.pointer(&c, &ev(PointerKind::Up, 207.0, 150.0)).as_slice(), [Action::Commit]));
    }
}

fn artboard_with_sloping_stroke(a: Point, b: Point) -> vectorcraft_doc::Document {
    let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
    d.artboards[0].rect = Rect::new(300.0, 100.0, 400.0, 200.0);
    let layer = d.layers[0].id;
    let id = d.alloc_id();
    let mut appearance = vectorcraft_doc::Appearance::default_art();
    appearance.set_fill(vectorcraft_color::Paint::None);
    appearance.stroke_mut().unwrap().width = 10.0;
    d.insert(Some(layer), 0, vectorcraft_doc::Node::path(id, vectorcraft_geom::shapes::line(a, b), appearance)).unwrap();
    d
}

/// An edge handle must meet the sloping outline on its horizontal way;
/// discarding the y of a free nearest point would leave it off the stroke.
#[test]
fn resizing_an_artboard_edge_contacts_a_sloping_stroke_on_its_axis() {
    let d = artboard_with_sloping_stroke(Point::new(100.0, 100.0), Point::new(200.0, 200.0));
    let (selection, paint) = (Selection::default(), paint());
    let c = ToolContext { snap_to_stroke_edges: true, snapping_tolerance: 4.0, ..cx(&d, &selection, &paint) };
    let mut tool = ArtboardTool::default();
    tool.pointer(&c, &ev(PointerKind::Down, 300.0, 150.0));
    let actions = tool.pointer(&c, &ev(PointerKind::Drag, 159.0, 150.0));
    assert!(actions.iter().any(|a| matches!(a, Action::Preview(id, _) if id == "artboard.setProps")));
    let v = preview_params(&actions);
    let x = v["x"].as_f64().unwrap();
    let expected = 150.0 + 5.0 * std::f64::consts::SQRT_2;
    assert!((x - expected).abs() < 1e-8, "{v:?}");
    assert!((v["width"].as_f64().unwrap() - (400.0 - expected)).abs() < 1e-8);
    assert_eq!((v["y"].as_f64(), v["height"].as_f64()), (Some(100.0), Some(100.0)));
    assert!(((x - 150.0) / std::f64::consts::SQRT_2 - 5.0).abs() < 1e-8, "the final handle is on the 10 pt stroke outline");
    assert!(matches!(tool.pointer(&c, &ev(PointerKind::Up, 159.0, 150.0)).as_slice(), [Action::Commit]));
}

/// Shift constrains a corner before finding its contour contact, so the
/// resulting bounds remain square and keep the opposite corner fixed.
#[test]
fn resizing_an_artboard_proportionally_contacts_a_sloping_stroke() {
    let d = artboard_with_sloping_stroke(Point::new(0.0, 180.0), Point::new(220.0, -40.0));
    let (selection, paint) = (Selection::default(), paint());
    let c = ToolContext { snap_to_stroke_edges: true, snapping_tolerance: 4.0, ..cx(&d, &selection, &paint) };
    let shift = Mods { shift: true, ..Mods::default() };
    let mut tool = ArtboardTool::default();
    tool.pointer(&c, &ev(PointerKind::Down, 300.0, 100.0).with_mods(shift));
    let actions = tool.pointer(&c, &ev(PointerKind::Drag, 194.5, 30.0).with_mods(shift));
    assert!(actions.iter().any(|a| matches!(a, Action::Preview(id, _) if id == "artboard.setProps")));
    let v = preview_params(&actions);
    let [x, y, w, h] = ["x", "y", "width", "height"].map(|key| v[key].as_f64().unwrap());
    let band = 5.0 * std::f64::consts::SQRT_2;
    let expected_x = (380.0 + band) / 2.0;
    let expected_y = expected_x - 200.0;
    assert!((x - expected_x).abs() < 1e-8 && (y - expected_y).abs() < 1e-8, "{v:?}");
    assert!((w - h).abs() < 1e-8 && (x + w - 400.0).abs() < 1e-8 && (y + h - 200.0).abs() < 1e-8, "{v:?}");
    assert!(((x + y - 180.0) / std::f64::consts::SQRT_2 - 5.0).abs() < 1e-8, "the final proportional corner contacts the outline");
    assert!(matches!(tool.pointer(&c, &ev(PointerKind::Up, 194.5, 30.0).with_mods(shift)).as_slice(), [Action::Commit]));
}

fn review_462530_stroke(d: &mut vectorcraft_doc::Document, x: f64, width: f64, y0: f64, y1: f64) -> vectorcraft_doc::NodeId {
    let layer = d.layers[0].id;
    let id = d.alloc_id();
    let mut appearance = vectorcraft_doc::Appearance::default_art();
    appearance.set_fill(vectorcraft_color::Paint::None);
    appearance.stroke_mut().unwrap().width = width;
    d.insert(Some(layer), 0, vectorcraft_doc::Node::path(id, vectorcraft_geom::shapes::line(Point::new(x, y0), Point::new(x, y1)), appearance))
        .unwrap();
    id
}

fn review_462530_resize(d: &vectorcraft_doc::Document, start: Point, end: Point) -> Value {
    let (selection, paint) = (Selection::default(), paint());
    let c = ToolContext {
        smart_guides: true,
        snap_to_stroke_edges: true,
        snapping_tolerance: 4.0,
        snap_to_grid: false,
        snap_to_pixel: false,
        ..cx(d, &selection, &paint)
    };
    let mut tool = ArtboardTool::default();
    tool.pointer(&c, &ev(PointerKind::Down, start.x, start.y));
    let actions = tool.pointer(&c, &ev(PointerKind::Drag, end.x, end.y));
    assert!(actions.iter().any(|a| matches!(a, Action::Preview(id, _) if id == "artboard.setProps")), "CONTROL: a handle resize must preview");
    preview_params(&actions)
}

#[test]
fn review_4625303998_mirrored_bleed_uses_the_actual_asymmetric_side() {
    let scene = |left, right, edge: f64| {
        let mut d = vectorcraft_doc::Document::new(800.0, 600.0);
        d.artboards[0].rect = Rect::new(300.0, 100.0, 400.0, 200.0);
        d.setup.bleed = [3.0, 11.0, left, right];
        let x = if edge < 400.0 { 330.0 } else { 430.0 };
        review_462530_stroke(&mut d, x, 2.0 * (edge - x), 80.0, 220.0);
        d
    };
    // Original left-side offset remains the ordinary unmirrored control.
    let d = scene(2.0, 7.0, 347.0);
    let v = review_462530_resize(&d, Point::new(300.0, 150.0), Point::new(350.0, 150.0));
    assert!((v["x"].as_f64().unwrap() - 349.0).abs() < 1e-8, "CONTROL: unmirrored left bleed contacts 347: {v}");
    for left in [2.0, 0.0] {
        let d = scene(left, 7.0, 458.0);
        let v = review_462530_resize(&d, Point::new(300.0, 150.0), Point::new(450.0, 150.0));
        let right = v["x"].as_f64().unwrap() + v["width"].as_f64().unwrap();
        assert!((v["x"].as_f64().unwrap() - 400.0).abs() < 1e-8, "CONTROL: crossing keeps the opposite edge fixed");
        assert!((right - 451.0).abs() < 1e-8, "REGRESSION: final right bleed is 7, even when original left bleed is {left}: {v}");
        assert!((right + 7.0 - 458.0).abs() < 1e-8, "REGRESSION: the final bleed point must meet the contour");
    }
    // A contact near the old left offset must not guide the final right side.
    let d = scene(5.0, 7.0, 444.0);
    let v = review_462530_resize(&d, Point::new(300.0, 150.0), Point::new(450.0, 150.0));
    assert!((v["x"].as_f64().unwrap() + v["width"].as_f64().unwrap() - 450.0).abs() < 1e-8, "REGRESSION: reject the stale-side contact: {v}");
}

#[test]
fn review_4625304007_artboard_tries_the_next_valid_stroke_contact() {
    let scene = |invalid, valid| {
        let mut d = vectorcraft_doc::Document::new(800.0, 600.0);
        d.artboards[0].rect = Rect::new(300.0, 100.0, 400.0, 200.0);
        if invalid {
            review_462530_stroke(&mut d, 290.0, 21.8, 80.0, 220.0); // right edge300.9: width0.9
        }
        if valid {
            review_462530_stroke(&mut d, 310.0, 15.0, 80.0, 220.0); // left edge302.5: width2.5
        }
        d
    };
    let resize = |d: &vectorcraft_doc::Document| review_462530_resize(d, Point::new(400.0, 150.0), Point::new(301.2, 150.0));
    let v = resize(&scene(false, true));
    assert!((v["width"].as_f64().unwrap() - 2.5).abs() < 1e-8, "CONTROL: the farther contact is reachable and valid: {v}");
    let v = resize(&scene(true, false));
    assert!((v["width"].as_f64().unwrap() - 1.2).abs() < 1e-8, "CONTROL: with no valid stroke contact legacy fallback retains raw width: {v}");
    let v = resize(&scene(true, true));
    assert!((v["width"].as_f64().unwrap() - 2.5).abs() < 1e-8, "REGRESSION: a closer undersized contact must not hide the next valid one: {v}");
}

fn review_462530_move(selected: &[usize], move_art: bool, copy: bool) -> Value {
    let mut d = vectorcraft_doc::Document::new(800.0, 600.0);
    d.artboards[0].rect = Rect::new(100.0, 100.0, 303.0, 200.0);
    let mut peer = d.artboards[0].clone();
    peer.rect = Rect::new(300.0, 100.0, 500.0, 200.0);
    peer.name = "Peer".into();
    d.artboards.push(peer);
    let target = review_462530_stroke(&mut d, 350.0, 90.0, 120.0, 180.0); // left edge305
    assert!(!d.art_moving_with(0, false).contains(&target), "CONTROL: target art is not owned by the dragged board");
    assert!(d.art_moving_with(1, false).contains(&target), "CONTROL: peer artwork really moves with board1");
    let (selection, paint) = (Selection::default(), paint());
    let c = ToolContext {
        zoom: 2.0,
        smart_guides: true,
        snap_to_stroke_edges: true,
        snapping_tolerance: 4.0,
        snap_to_grid: false,
        snap_to_pixel: false,
        ..cx(&d, &selection, &paint)
    };
    let mut tool = ArtboardTool::default();
    tool.set_option("selected", &json!(selected));
    tool.set_option("moveArt", &json!(move_art));
    let mods = Mods { alt: copy, ..Mods::default() };
    tool.pointer(&c, &ev(PointerKind::Down, 200.0, 170.0).with_mods(mods));
    // At2x, 1.6pt is3.2px: beyond the normal3px drag threshold, without crossing the target contour.
    let actions = tool.pointer(&c, &ev(PointerKind::Drag, 201.6, 170.0).with_mods(mods));
    assert!(actions.iter().any(|a| matches!(a, Action::Preview(id, _) if id == "artboard.move")), "CONTROL: a real board move must preview");
    let v = preview_params(&actions);
    assert_eq!(v["copy"], copy);
    assert_eq!(v["moveArt"], move_art);
    if selected.len() > 1 {
        assert_eq!(v["indices"], json!(selected), "CONTROL: preview carries the exact selected move set");
    }
    v
}

#[test]
fn review_4625304019_moving_peer_art_is_excluded_but_original_copies_remain_targets() {
    for (selected, move_art, copy) in [(&[0][..], true, false), (&[0, 1][..], false, false), (&[0, 1][..], true, true)] {
        let v = review_462530_move(selected, move_art, copy);
        assert!((v["dx"].as_f64().unwrap() - 2.0).abs() < 1e-8, "CONTROL: stationary artwork or original-copy targets remain eligible: {v}");
    }
    let v = review_462530_move(&[0, 1], true, false);
    assert!(
        (v["dx"].as_f64().unwrap() - 1.6).abs() < 1e-8,
        "REGRESSION: all moving peer artwork must be excluded; no stationary contour holds the raw delta: {v}"
    );
}

fn review_462630_resize(d: &vectorcraft_doc::Document, start: Point, end: Point, stroke_edges: bool) -> Value {
    let (selection, paint) = (Selection::default(), paint());
    let c = ToolContext {
        smart_guides: true,
        snap_to_stroke_edges: stroke_edges,
        snapping_tolerance: 4.0,
        snap_to_grid: false,
        snap_to_pixel: false,
        ..cx(d, &selection, &paint)
    };
    let mut tool = ArtboardTool::default();
    tool.set_option("scaleArt", &json!(false));
    tool.pointer(&c, &ev(PointerKind::Down, start.x, start.y));
    let actions = tool.pointer(&c, &ev(PointerKind::Drag, end.x, end.y));
    assert!(actions.iter().any(|action| matches!(action, Action::Preview(id, _) if id == "artboard.setProps")), "a real handle resize must preview");
    preview_params(&actions)
}

fn review_462630_bounds(value: &Value, expected: Rect) {
    for (key, expected) in ["x", "y", "width", "height"].into_iter().zip([expected.x0, expected.y0, expected.width(), expected.height()]) {
        let actual = value[key].as_f64().unwrap();
        assert!((actual - expected).abs() <= 1e-6, "{key}: expected {expected}, got {actual}: {value}");
    }
}

fn review_462630_floor_scene(edge: f64) -> vectorcraft_doc::Document {
    let mut doc = vectorcraft_doc::Document::new(22000.0, 1000.0);
    doc.artboards[0].rect = Rect::new(0.0, 0.0, 20000.0, 1000.0);
    review_462530_stroke(&mut doc, edge - 10.0, 20.0, -1000.0, 2000.0);
    doc
}

#[test]
fn review_597185_4626304652_artboard_accepts_the_exact_solver_minimum_scale() {
    let start = Point::new(20000.0, 500.0);
    let raw = Point::new(2.25, 500.0);
    let above = review_462630_resize(&review_462630_floor_scene(2.5), start, raw, true);
    review_462630_bounds(&above, Rect::new(0.0, 0.0, 2.5, 1000.0));
    let below = review_462630_resize(&review_462630_floor_scene(1.5), start, raw, true);
    review_462630_bounds(&below, Rect::new(0.0, 0.0, 2.25, 1000.0));
    let doc = review_462630_floor_scene(2.0);
    let off = review_462630_resize(&doc, start, raw, false);
    review_462630_bounds(&off, Rect::new(0.0, 0.0, 2.25, 1000.0));
    let exact = review_462630_resize(&doc, start, raw, true);
    review_462630_bounds(&exact, Rect::new(0.0, 0.0, 2.0, 1000.0));
}

fn review_462630_corner_scene() -> vectorcraft_doc::Document {
    let mut doc = vectorcraft_doc::Document::new(500.0, 500.0);
    doc.artboards[0].rect = Rect::new(0.0, 0.0, 20.0, 20.0);
    let intercept = 11.0 - 14.0 / std::f64::consts::SQRT_2;
    let mut appearance = vectorcraft_doc::Appearance::default_art();
    appearance.set_fill(vectorcraft_color::Paint::None);
    appearance.stroke_mut().unwrap().width = 14.0;
    let layer = doc.layers[0].id;
    let node = vectorcraft_doc::Node::path(
        doc.alloc_id(),
        vectorcraft_geom::shapes::line(Point::new(-100.0, -100.0 + intercept), Point::new(20.0, 20.0 + intercept)),
        appearance,
    );
    doc.insert(Some(layer), 0, node).unwrap();
    doc
}

#[test]
fn review_597185_4626304623_artboard_finds_the_feasible_part_of_one_contour_segment() {
    let doc = review_462630_corner_scene();
    let start = Point::new(20.0, 20.0);
    let control = review_462630_resize(&doc, start, Point::new(2.0, 13.0), true);
    review_462630_bounds(&control, Rect::new(0.0, 0.0, 2.0, 13.0));
    let raw = Point::new(2.0, 10.0);
    let off = review_462630_resize(&doc, start, raw, false);
    review_462630_bounds(&off, Rect::new(0.0, 0.0, 2.0, 10.0));
    let feasible = review_462630_resize(&doc, start, raw, true);
    review_462630_bounds(&feasible, Rect::new(0.0, 0.0, 1.0, 12.0));
    assert!(feasible["width"].as_f64().unwrap() >= 1.0 && feasible["height"].as_f64().unwrap() >= 1.0);
    assert!(
        (feasible["height"].as_f64().unwrap() - feasible["width"].as_f64().unwrap() - 11.0).abs() <= 1e-6,
        "the final corner is on the actual painted side"
    );
}

fn review_462630_anchor_scene(anchor: f64) -> vectorcraft_doc::Document {
    let mut doc = vectorcraft_doc::Document::new(500.0, 500.0);
    doc.artboards[0].rect = Rect::new(0.0, 0.0, 20.0, 20.0);
    review_462530_stroke(&mut doc, -8.5, 20.0, -1000.0, 2000.0); // right painted edge1.5
    let mut triangle = vectorcraft_geom::BezPath::new();
    triangle.move_to((anchor, 10.0));
    triangle.line_to((-50.0, -30.0));
    triangle.line_to((-50.0, 50.0));
    triangle.close_path();
    let appearance =
        vectorcraft_doc::Appearance::basic(vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK), vectorcraft_color::Paint::None, 0.0);
    let layer = doc.layers[0].id;
    let node = vectorcraft_doc::Node::path(doc.alloc_id(), vectorcraft_geom::PathData::from_bezpath(&triangle), appearance);
    doc.insert(Some(layer), usize::MAX, node).unwrap();
    doc
}

#[test]
fn review_597185_4626304646_invalid_artboard_anchor_does_not_block_a_valid_stroke_contact() {
    let start = Point::new(20.0, 20.0);
    let raw = Point::new(2.0, 10.0);
    let valid = review_462630_resize(&review_462630_anchor_scene(1.25), start, raw, true);
    review_462630_bounds(&valid, Rect::new(0.0, 0.0, 1.25, 10.0));
    let doc = review_462630_anchor_scene(0.5);
    let off = review_462630_resize(&doc, start, raw, false);
    review_462630_bounds(&off, Rect::new(0.0, 0.0, 0.5, 10.0));
    let stroke = review_462630_resize(&doc, start, raw, true);
    review_462630_bounds(&stroke, Rect::new(0.0, 0.0, 1.5, 10.0));
}
fn review_597368_resize(d: &vectorcraft_doc::Document, start: Point, end: Point, mods: Mods, stroke_edges: bool, view: (f64, f64)) -> Value {
    review_597569_resize_guides(d, start, end, mods, stroke_edges, view).0
}

fn review_597569_resize_guides(
    d: &vectorcraft_doc::Document,
    start: Point,
    end: Point,
    mods: Mods,
    stroke_edges: bool,
    view: (f64, f64),
) -> (Value, Vec<Overlay>) {
    let (selection, paint) = (Selection::default(), paint());
    let c = ToolContext {
        smart_guides: true,
        snap_to_stroke_edges: stroke_edges,
        zoom: view.0,
        snapping_tolerance: view.1,
        snap_to_grid: false,
        snap_to_pixel: false,
        ..cx(d, &selection, &paint)
    };
    let original = d.clone();
    let mut tool = ArtboardTool::default();
    tool.set_option("scaleArt", &json!(false));
    tool.pointer(&c, &ev(PointerKind::Down, start.x, start.y));
    let actions = tool.pointer(&c, &ev(PointerKind::Drag, end.x, end.y).with_mods(mods));
    assert!(tool.busy(), "CONTROL: a real Artboard handle drag remains active until release");
    assert!(actions.iter().any(|a| matches!(a, Action::Begin(label) if label == "Resize Artboard")));
    assert!(actions.iter().any(|a| matches!(a, Action::Preview(id, _) if id == "artboard.setProps")), "CONTROL: this is a real handle resize");
    let value = preview_params(&actions);
    let guides = tool.overlays(&c);
    assert!(matches!(tool.pointer(&c, &ev(PointerKind::Up, end.x, end.y).with_mods(mods)).as_slice(), [Action::Commit]));
    assert!(!tool.busy());
    assert_eq!(d, &original, "CONTROL: the tool only emits a preview; source artwork/paint/bleed/resources stay exact");
    (value, guides)
}

fn review_597368_horizontal_edge(d: &mut vectorcraft_doc::Document, x0: f64, x1: f64, y: f64) -> vectorcraft_doc::NodeId {
    let layer = d.layers[0].id;
    let id = d.alloc_id();
    let mut appearance = vectorcraft_doc::Appearance::default_art();
    appearance.set_fill(vectorcraft_color::Paint::None);
    appearance.stroke_mut().unwrap().width = 10.0;
    d.insert(
        Some(layer),
        0,
        vectorcraft_doc::Node::path(id, vectorcraft_geom::shapes::line(Point::new(x0, y - 5.0), Point::new(x1, y - 5.0)), appearance),
    )
    .unwrap();
    id
}

fn review_597368_painted(d: &vectorcraft_doc::Document, id: vectorcraft_doc::NodeId, contact: Point, tol: f64) {
    let node = d.node(id).unwrap();
    let path = node.path_data().unwrap();
    assert!(path.nearest(contact).unwrap().4 > tol, "CONTROL: the source centerline cannot explain this contour contact");
    let painted = vectorcraft_effects::stroke::outline_region(path, vectorcraft_geom::FillRule::NonZero, node.appearance.stroke().unwrap());
    assert!(painted.nearest(contact).unwrap().4 < 1e-7, "CONTROL: the expected contact is on the actual painted stroke: {contact:?}");
}

#[test]
fn review_597368_artboard_collinear_contacts_keep_side_and_coupled_minima() {
    let view = (1.0, 4.0);
    let mut side = vectorcraft_doc::Document::new(2000.0, 1200.0);
    side.artboards[0].rect = Rect::new(0.0, 0.0, 100.0, 100.0);
    let side_stroke = review_597368_horizontal_edge(&mut side, -10.0, 110.0, 50.0);
    review_597368_painted(&side, side_stroke, Point::new(1.0, 50.0), 4.0);
    // All ordinary controls precede the first regression.
    let start = Point::new(100.0, 50.0);
    let raw = Point::new(0.5, 50.0);
    review_462630_bounds(&review_597368_resize(&side, start, raw, Mods::default(), false, view), Rect::new(0.0, 0.0, 0.5, 100.0));
    review_462630_bounds(&review_597368_resize(&side, start, Point::new(2.0, 50.0), Mods::default(), true, view), Rect::new(0.0, 0.0, 2.0, 100.0));
    let mut crossing = vectorcraft_doc::Document::new(2000.0, 1200.0);
    crossing.artboards[0].rect = side.artboards[0].rect;
    let crossing_stroke = review_462530_stroke(&mut crossing, -4.0, 10.0, -1000.0, 2000.0);
    review_597368_painted(&crossing, crossing_stroke, Point::new(1.0, 50.0), 4.0);
    review_462630_bounds(&review_597368_resize(&crossing, start, raw, Mods::default(), true, view), Rect::new(0.0, 0.0, 1.0, 100.0));

    let mut wide = vectorcraft_doc::Document::new(2000.0, 1200.0);
    wide.artboards[0].rect = Rect::new(0.0, 0.0, 1000.0, 1.0);
    let wide_stroke = review_597368_horizontal_edge(&mut wide, 980.0, 1020.0, 0.5);
    // At zoom12, the5px handle radius is smaller than half the1pt height,
    // so this really presses Right rather than the earlier TopRight hit.
    // The ordinary allowed8px snap tolerance becomes2/3pt, still covering999.5->1000.
    let wide_view = (12.0, 8.0);
    let wide_start = Point::new(1000.0, 0.5);
    review_597368_painted(&wide, wide_stroke, Point::new(1000.0, 0.5), 8.0 / 12.0);
    review_462630_bounds(
        &review_597368_resize(&wide, wide_start, Point::new(1002.0, 0.5), Mods::default(), false, wide_view),
        Rect::new(0.0, 0.0, 1002.0, 1.0),
    );
    let shift = Mods { shift: true, ..Mods::default() };
    review_462630_bounds(
        &review_597368_resize(&wide, wide_start, Point::new(1000.25, 0.5), shift, true, wide_view),
        Rect::new(0.0, -0.000125, 1000.25, 1.000125),
    );
    let (off, off_guides) = review_597569_resize_guides(&wide, wide_start, Point::new(999.5, 0.5), shift, false, wide_view);
    review_462630_bounds(&off, Rect::new(0.0, 0.0, 1000.0, 1.0));
    assert!(
        !off_guides.iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "stroke edge")),
        "CONTROL: the same minimum-side bounds with stroke snapping OFF do not claim a painted contact"
    );

    let mirrored = |left: f64| {
        let mut d = vectorcraft_doc::Document::new(2000.0, 1200.0);
        d.artboards[0].rect = Rect::new(300.0, 100.0, 400.0, 200.0);
        d.setup.bleed = [3.0, 11.0, left, 7.0];
        let stroke = review_597368_horizontal_edge(&mut d, 407.0, 430.0, 150.0);
        review_597368_painted(&d, stroke, Point::new(408.0, 150.0), 4.0);
        (d, stroke)
    };
    let mirror_start = Point::new(300.0, 150.0);
    let mirror_raw = Point::new(400.5, 150.0);
    for left in [2.0, 0.0] {
        let (d, _) = mirrored(left);
        review_462630_bounds(
            &review_597368_resize(&d, mirror_start, mirror_raw, Mods::default(), false, view),
            Rect::new(400.0, 100.0, 400.5, 200.0),
        );
        review_462630_bounds(
            &review_597368_resize(&d, mirror_start, Point::new(402.0, 150.0), Mods::default(), true, view),
            Rect::new(400.0, 100.0, 402.0, 200.0),
        );
    }

    // REGRESSION1: the nearer feasible interval begins at width1, not the far
    // collinear query endpoint4.5 chosen after rejecting the midpoint.
    let v = review_597368_resize(&side, start, raw, Mods::default(), true, view);
    review_462630_bounds(&v, Rect::new(0.0, 0.0, 1.0, 100.0));
    assert_eq!(v["x"], 0.0, "the opposite side stays fixed");
    assert!(v["width"].as_f64().unwrap() >= 1.0 && v["height"].as_f64().unwrap() >= 1.0);

    // REGRESSION2: proportional Right changes height even though that grabbed
    // coordinate stays at0.5; the1pt short side requires factor>=1, hence width1000.
    let (v, guides) = review_597569_resize_guides(&wide, wide_start, Point::new(999.5, 0.5), shift, true, wide_view);
    assert!(
        guides.iter().any(|o| matches!(o, Overlay::Label { p, text, .. } if text == "stroke edge" && p.distance(Point::new(1000.0, 0.5)) <= 1e-6)),
        "REGRESSION: the proportional minimum must retain its actual painted contact, not merely clamp to identical bounds: {guides:?}"
    );
    review_462630_bounds(&v, Rect::new(0.0, 0.0, 1000.0, 1.0));
    assert!(v["width"].as_f64().unwrap() >= 1000.0 && v["height"].as_f64().unwrap() >= 1.0);
    assert_eq!(v["x"], 0.0, "the proportional side keeps its opposite x origin");
    let center_y = v["y"].as_f64().unwrap() + v["height"].as_f64().unwrap() / 2.0;
    assert!((center_y - 0.5).abs() < 1e-12, "the inactive grabbed coordinate stays at its pivot: {v}");

    // REGRESSION3: after Left crosses its fixed Right origin, the carried point
    // uses the actual RIGHT bleed7; old LEFT bleed2/0 must not replace that offset.
    for left in [2.0, 0.0] {
        let (d, stroke) = mirrored(left);
        let v = review_597368_resize(&d, mirror_start, mirror_raw, Mods::default(), true, view);
        review_462630_bounds(&v, Rect::new(400.0, 100.0, 401.0, 200.0));
        let contact = Point::new(v["x"].as_f64().unwrap() + v["width"].as_f64().unwrap() + 7.0, 150.0);
        assert!(contact.distance(Point::new(408.0, 150.0)) <= 1e-6, "actual mirrored bleed contact: {contact:?}");
        review_597368_painted(&d, stroke, contact, 4.0);
        assert_eq!(v["x"], 400.0, "the opposite side stays fixed after mirroring");
        assert!(v["width"].as_f64().unwrap() >= 1.0 && v["height"].as_f64().unwrap() >= 1.0);
    }
}
