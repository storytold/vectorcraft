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
