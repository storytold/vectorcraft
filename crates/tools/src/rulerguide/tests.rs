use super::*;
use crate::testutil::*;
use crate::{Mods, ScreenFrame};
use vectorcraft_doc::{Guide, Selection};
use vectorcraft_geom::Vec2;

fn ev(kind: PointerKind, x: f64, y: f64, mods: Mods) -> PointerEvent {
    PointerEvent::new(kind, x, y).with_mods(mods)
}

fn guides_doc() -> vectorcraft_doc::Document {
    let (mut d, _) = doc_with_rect();
    d.guides = vec![Guide::new(true, 300.0), Guide::new(false, 400.0)];
    d
}

#[test]
fn picks_the_guide_within_the_tolerance_unless_hidden_or_locked() {
    let (d, s, p) = (guides_doc(), Selection::default(), paint());
    let c = cx(&d, &s, &p);
    assert_eq!(guide_at(&c, Point::new(302.0, 50.0)), Some(0));
    assert_eq!(guide_at(&c, Point::new(50.0, 398.0)), Some(1));
    // Where they cross, the nearer one.
    assert_eq!(guide_at(&c, Point::new(301.0, 398.0)), Some(0));
    assert_eq!(guide_at(&c, Point::new(305.0, 50.0)), None, "beyond the 3 px tolerance");
    let hidden = ToolContext { guides: false, ..cx(&d, &s, &p) };
    assert_eq!(guide_at(&hidden, Point::new(300.0, 50.0)), None);
}

#[test]
fn a_drag_moves_the_selected_guides_in_one_step() {
    let d = guides_doc();
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    let mut g = GuideEdit::default();
    let m = Mods::default();
    assert_eq!(g.press(&c, &ev(PointerKind::Down, 300.0, 50.0, m)), Some(vec![Action::Exec("guide.select".into(), json!({"indexes": [0]}))]));
    // The tool sees the guide selected from now on.
    let mut s = s;
    s.set_guides([0]);
    let c = cx(&d, &s, &p);
    let a = g.pointer(&c, &ev(PointerKind::Drag, 340.0, 60.0, m)).unwrap();
    assert_eq!(a[0], Action::Begin("Move Guide".into()));
    assert_eq!(a[1], Action::Preview("guide.move".into(), json!({"dx": 40.0, "dy": 10.0, "copy": false})));
    assert!(matches!(g.overlays(&c).last(), Some(Overlay::Measure { text, .. }) if text == "X: 340.00 pt"));
    assert_eq!(g.cursor(&c, Point::ZERO), Some(Cursor::ResizeH));
    assert_eq!(g.pointer(&c, &ev(PointerKind::Up, 340.0, 60.0, m)), Some(vec![Action::Commit]));
    assert!(!g.busy() && g.pointer(&c, &ev(PointerKind::Drag, 0.0, 0.0, m)).is_none());
}

#[test]
fn the_dragged_guide_snaps_to_art_ruler_ticks_and_the_grid() {
    let d = guides_doc();
    let mut s = Selection::default();
    s.set_guides([0]);
    let p = paint();
    let drag_to = |c: &ToolContext, x: f64, mods: Mods| {
        let mut g = GuideEdit::default();
        g.press(c, &ev(PointerKind::Down, 300.0, 50.0, Mods::default()));
        let a = g.pointer(c, &ev(PointerKind::Drag, x, 50.0, mods)).unwrap();
        let Some(Action::Preview(_, v)) = a.last() else { panic!("{a:?}") };
        (300.0 + v["dx"].as_f64().unwrap(), g.overlays(c))
    };
    // Smart Guides: onto the rect's right edge (x = 200), labelled.
    let (x, ov) = drag_to(&cx(&d, &s, &p), 203.0, Mods::default());
    assert_eq!(x, 200.0);
    assert!(matches!(&ov[0], Overlay::Label { .. }));
    let off = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    assert_eq!(drag_to(&off, 203.0, Mods::default()).0, 203.0);
    // Shift: the ruler's ticks (labels every 50 pt at 100 %, ticks every 5 pt).
    assert_eq!(drag_to(&off, 223.0, Mods { shift: true, ..Default::default() }).0, 225.0);
    let zoomed = ToolContext { zoom: 5.0, ..cx(&d, &s, &p) };
    assert_eq!(drag_to(&zoomed, 223.4, Mods { shift: true, ..Default::default() }).0, 223.0, "every point at 500 %");
    let grid = ToolContext { snap_to_grid: true, ..cx(&d, &s, &p) };
    assert_eq!(drag_to(&grid, 226.0, Mods::default()).0, 225.0, "the grid's 9 pt subdivisions");
}

/// #451: a guide dragged out of a ruler lands on the art's corners, side midpoints (its
/// centre's line) and anchors, and on the artboard's edges and centre, labelled.
#[test]
fn a_new_guide_snaps_to_the_art_and_the_artboard() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    let m = Mods::default();
    let drag = |vertical: bool, v: f64| {
        let mut g = NewGuide::new(&c, vertical, None);
        let at = if vertical { Point::new(v, 50.0) } else { Point::new(50.0, v) };
        let a = g.pointer(&c, &ev(PointerKind::Drag, at.x, at.y, m), true);
        let Some(Action::Preview(cmd, p)) = a.last() else { panic!("{a:?}") };
        assert_eq!((a[0].clone(), cmd.as_str(), &p["vertical"]), (Action::Begin("New Guide".into()), "guide.add", &json!(vertical)));
        let label = g.overlays(&c).iter().find_map(|o| match o {
            Overlay::Label { text, .. } => Some(text.clone()),
            _ => None,
        });
        (p["pos"].as_f64().unwrap(), label)
    };
    assert_eq!(drag(true, 152.0), (150.0, Some("center".into())), "the line through the top and bottom midpoints");
    assert_eq!(drag(false, 103.0), (100.0, Some("path".into())), "the top edge, through two corners");
    assert_eq!(drag(true, 497.0), (500.0, Some("artboard".into())), "the artboard's right edge");
    assert_eq!(drag(false, 248.0), (250.0, Some("center".into())), "the artboard's centre");
    assert_eq!(drag(true, 320.0), (320.0, None), "nothing within reach");
}

/// Preferences › Smart Guides (#394) reach a guide dragged out of a ruler: Snapping
/// Tolerance sets how far a target pulls it, Anchor/Path Labels and Measurement Labels what
/// shows by it.
#[test]
fn a_new_guide_follows_the_smart_guide_preferences() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let drag = |c: &ToolContext| {
        let mut g = NewGuide::new(c, true, None);
        let a = g.pointer(c, &ev(PointerKind::Drag, 497.0, 50.0, Mods::default()), true);
        let Some(Action::Preview(_, p)) = a.last() else { panic!("{a:?}") };
        (p["pos"].as_f64().unwrap(), g.overlays(c))
    };
    let (at, ov) = drag(&ToolContext { snapping_tolerance: 2.0, ..cx(&d, &s, &p) });
    assert_eq!((at, ov.len()), (497.0, 1), "3 px off is beyond 2: only the readout {ov:?}");
    let (at, ov) = drag(&ToolContext { anchor_path_labels: false, measurement_labels: false, ..cx(&d, &s, &p) });
    assert_eq!((at, ov), (500.0, vec![]), "onto the artboard's edge, nothing said");
}

#[test]
fn a_new_guide_is_made_on_release_over_the_canvas_only() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let c = ToolContext { smart_guides: false, snap_to_point: false, ..cx(&d, &s, &p) };
    let m = Mods::default();
    let mut g = NewGuide::new(&c, false, Some(0));
    let preview = |pos: f64| Action::Preview("guide.add".into(), json!({"vertical": false, "pos": pos, "artboard": 0}));
    // Off the canvas (still over the ruler): nothing yet.
    assert_eq!(g.pointer(&c, &ev(PointerKind::Drag, 40.0, -5.0, m), false), vec![]);
    assert_eq!(g.pointer(&c, &ev(PointerKind::Drag, 40.0, 30.0, m), true), vec![Action::Begin("New Guide".into()), preview(30.0)]);
    assert!(matches!(g.overlays(&c).last(), Some(Overlay::Measure { text, .. }) if text == "Y: 30.00 pt"));
    // The same place again: no new preview.
    assert_eq!(g.pointer(&c, &ev(PointerKind::Drag, 90.0, 30.0, m), true), vec![]);
    // Back onto the ruler it goes, and comes back over the canvas.
    assert_eq!(g.pointer(&c, &ev(PointerKind::Drag, 40.0, -5.0, m), false), vec![Action::Cancel]);
    assert!(g.overlays(&c).is_empty());
    assert_eq!(g.pointer(&c, &ev(PointerKind::Drag, 40.0, 60.0, m), true), vec![Action::Begin("New Guide".into()), preview(60.0)]);
    // Shift: the ruler's ticks.
    assert_eq!(g.pointer(&c, &ev(PointerKind::Drag, 40.0, 63.0, Mods { shift: true, ..m }), true), vec![preview(65.0)]);
    assert_eq!(g.pointer(&c, &ev(PointerKind::Up, 40.0, 65.0, m), true), vec![Action::Commit]);
    // Released off the canvas: none.
    let mut g = NewGuide::new(&c, true, None);
    g.pointer(&c, &ev(PointerKind::Drag, 30.0, 40.0, m), true);
    assert_eq!(g.pointer(&c, &ev(PointerKind::Up, -5.0, 40.0, m), false), vec![Action::Cancel]);
}

#[test]
fn new_ruler_guide_alt_switches_axis_and_uses_the_release_position() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let c = ToolContext { smart_guides: false, snap_to_point: false, ..cx(&d, &s, &p) };
    for source_vertical in [false, true] {
        let mut g = NewGuide::new(&c, source_vertical, Some(0));
        for (i, alt) in [false, true, false, true].into_iter().enumerate() {
            let vertical = source_vertical != alt;
            let at = if vertical { 80.0 } else { 120.0 };
            let mut expected = vec![];
            if i == 0 {
                expected.push(Action::Begin("New Guide".into()));
            }
            expected.push(Action::Preview("guide.add".into(), json!({"vertical": vertical, "pos": at, "artboard": 0})));
            assert_eq!(g.pointer(&c, &ev(PointerKind::Drag, 80.0, 120.0, Mods { alt, ..Default::default() }), true), expected);
            let label = format!("{}: {at:.2} pt", if vertical { "X" } else { "Y" });
            assert!(matches!(g.overlays(&c).last(), Some(Overlay::Measure { text, .. }) if text == &label));
        }
        // Releasing at a new position with Alt released uses both final coordinates and modifiers.
        let at = if source_vertical { 190.0 } else { 240.0 };
        assert_eq!(
            g.pointer(&c, &ev(PointerKind::Up, 190.0, 240.0, Mods::default()), true),
            vec![Action::Preview("guide.add".into(), json!({"vertical": source_vertical, "pos": at, "artboard": 0})), Action::Commit]
        );
        assert!(g.overlays(&c).is_empty());
    }
}

#[test]
fn new_ruler_guide_alt_switches_even_when_coordinates_are_equal() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let c = ToolContext { smart_guides: false, snap_to_point: false, ..cx(&d, &s, &p) };
    for source_vertical in [false, true] {
        let mut g = NewGuide::new(&c, source_vertical, None);
        g.pointer(&c, &ev(PointerKind::Drag, 80.0, 80.0, Mods::default()), true);
        for alt in [true, false] {
            assert_eq!(
                g.pointer(&c, &ev(PointerKind::Drag, 80.0, 80.0, Mods { alt, ..Default::default() }), true),
                vec![Action::Preview("guide.add".into(), json!({"vertical": source_vertical != alt, "pos": 80.0}))]
            );
        }
    }
}

#[test]
fn new_ruler_guide_alt_snaps_on_the_effective_axis() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let off = ToolContext { smart_guides: false, snap_to_point: false, ..cx(&d, &s, &p) };
    let grid = ToolContext { snap_to_grid: true, ..off };
    let pixel = ToolContext { snap_to_pixel: true, ..off };
    let smart = cx(&d, &s, &p);
    for (c, shift, x, y, expected_x, expected_y) in [
        (&off, true, 223.0, 237.0, 225.0, 235.0),
        (&grid, false, 226.0, 237.0, 225.0, 234.0),
        (&pixel, false, 226.4, 237.6, 226.0, 238.0),
        (&smart, false, 203.0, 102.0, 200.0, 100.0),
    ] {
        for source_vertical in [false, true] {
            let mut g = NewGuide::new(c, source_vertical, None);
            let actions = g.pointer(c, &ev(PointerKind::Drag, x, y, Mods { alt: true, shift, ..Default::default() }), true);
            let expected = if source_vertical { expected_y } else { expected_x };
            assert_eq!(actions.last(), Some(&Action::Preview("guide.add".into(), json!({"vertical": !source_vertical, "pos": expected}))));
        }
    }
}

/// With Smart Guides off, Snap to Point pulls a dragged guide onto the anchors (within its
/// distance), not the edges' midpoints.
#[test]
fn with_smart_guides_off_snap_to_point_pulls_guides_onto_anchors() {
    let d = guides_doc();
    let mut s = Selection::default();
    s.set_guides([0]);
    let p = paint();
    let off = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let moved_to = |c: &ToolContext, x: f64| {
        let mut g = GuideEdit::default();
        g.press(c, &ev(PointerKind::Down, 300.0, 50.0, Mods::default()));
        let a = g.pointer(c, &ev(PointerKind::Drag, x, 50.0, Mods::default())).unwrap();
        let Some(Action::Preview(_, v)) = a.last() else { panic!("{a:?}") };
        300.0 + v["dx"].as_f64().unwrap()
    };
    assert_eq!(moved_to(&off, 201.5), 200.0);
    assert_eq!(moved_to(&off, 151.0), 151.0, "the centre is no anchor");
    assert_eq!(moved_to(&off, 203.0), 203.0, "beyond the 2 px Snap to Point distance");
    let neither = ToolContext { snap_to_point: false, ..off };
    assert_eq!(moved_to(&neither, 201.5), 201.5);
    let mut g = NewGuide::new(&off, true, None);
    let a = g.pointer(&off, &ev(PointerKind::Drag, 98.5, 20.0, Mods::default()), true);
    assert_eq!(a.last(), Some(&Action::Preview("guide.add".into(), json!({"vertical": true, "pos": 100.0}))));
}

/// #451: an artboard guide runs across its artboard only: it is picked there, and pulls a
/// point with Snap to Point there.
#[test]
fn an_artboard_guide_is_picked_and_snapped_to_across_its_artboard_only() {
    let (mut d, _) = doc_with_rect();
    let id = d.next_artboard_id();
    d.artboards.push(vectorcraft_doc::Artboard {
        id,
        name: "B".into(),
        rect: vectorcraft_geom::Rect::new(600.0, 0.0, 900.0, 300.0),
        locked: false,
        show_center_mark: false,
        show_cross_hairs: false,
        show_video_safe_areas: false,
        video_ruler_pixel_aspect_ratio: 1.0,
        color: None,
    });
    d.guides = vec![Guide { artboard: Some(id), ..Guide::new(false, 150.0) }];
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    assert_eq!(guide_at(&c, Point::new(700.0, 151.0)), Some(0));
    assert_eq!(guide_at(&c, Point::new(602.0 - 4.0, 151.0)), Some(0), "within the tolerance of its end");
    assert_eq!(guide_at(&c, Point::new(300.0, 151.0)), None, "not across the first artboard");
    let t = Targets::snap_to_point(&ToolContext { smart_guides: false, ..c }, &[]).unwrap();
    assert_eq!(t.snap_point(Point::new(700.0, 151.5), 2.0).0, Point::new(700.0, 150.0));
    assert_eq!(t.snap_point(Point::new(300.0, 151.5), 2.0).0, Point::new(300.0, 151.5));
}

#[test]
fn shift_click_toggles_and_alt_drag_copies() {
    let d = guides_doc();
    let mut s = Selection::default();
    s.set_guides([0, 1]);
    let p = paint();
    let c = cx(&d, &s, &p);
    let (shift, alt) = (Mods { shift: true, ..Default::default() }, Mods { alt: true, ..Default::default() });
    let mut g = GuideEdit::default();
    assert_eq!(g.press(&c, &ev(PointerKind::Down, 50.0, 400.0, shift)), Some(vec![]));
    let deselect = Action::Exec("guide.select".into(), json!({"indexes": [1], "toggle": true}));
    assert_eq!(g.pointer(&c, &ev(PointerKind::Up, 50.0, 400.0, shift)), Some(vec![deselect]));
    g.press(&c, &ev(PointerKind::Down, 50.0, 400.0, alt));
    let a = g.pointer(&c, &ev(PointerKind::Drag, 50.0, 420.0, alt)).unwrap();
    assert_eq!(a[0], Action::Begin("Copy Guide".into()));
    assert_eq!(a[1], Action::Preview("guide.move".into(), json!({"dx": 0.0, "dy": 20.0, "copy": true})));
}

/// #2143: a click on a guide selected with objects makes it the key; on the key guide again,
/// no key. Alone (no objects), with Shift, or dragged, it doesn't.
#[test]
fn a_click_on_a_guide_selected_with_objects_makes_it_the_key() {
    // The same document as doc_with_rect's, with the guides.
    let (d, rect) = (guides_doc(), doc_with_rect().1);
    let p = paint();
    let click = |s: &Selection, m: Mods| {
        let c = cx(&d, s, &p);
        let mut g = GuideEdit::default();
        let mut out = g.press(&c, &ev(PointerKind::Down, 302.0, 50.0, m)).unwrap();
        out.extend(g.pointer(&c, &ev(PointerKind::Up, 302.0, 50.0, m)).unwrap());
        out
    };
    let mut s = Selection::default();
    s.set([rect]);
    s.toggle_guide(0);
    let make = Action::Exec("select.key".into(), json!({"guide": 0}));
    assert_eq!(click(&s, Mods::default()), vec![make]);
    s.key_guide = Some(0);
    assert_eq!(click(&s, Mods::default()), vec![Action::Exec("select.key".into(), json!({}))], "the key guide again");
    let shift = Mods { shift: true, ..Default::default() };
    assert_eq!(click(&s, shift), vec![Action::Exec("guide.select".into(), json!({"indexes": [0], "toggle": true}))]);
    s.set_guides([0]);
    assert_eq!(click(&s, Mods::default()), vec![], "a guide alone has nothing to align");
    // Not selected yet: the first click selects it.
    s.set([rect]);
    assert_eq!(click(&s, Mods::default()), vec![Action::Exec("guide.select".into(), json!({"indexes": [0]}))]);
    // A drag moves it.
    s.toggle_guide(0);
    let c = cx(&d, &s, &p);
    let mut g = GuideEdit::default();
    g.press(&c, &ev(PointerKind::Down, 302.0, 50.0, Mods::default()));
    g.pointer(&c, &ev(PointerKind::Drag, 340.0, 50.0, Mods::default()));
    assert_eq!(g.pointer(&c, &ev(PointerKind::Up, 340.0, 50.0, Mods::default())), Some(vec![Action::Commit]));
}

#[test]
fn dropped_on_its_ruler_the_guide_is_deleted() {
    let d = guides_doc();
    let p = paint();
    let mut s = Selection::default();
    s.set_guides([1]);
    // The window shows the document from (0, 0) at 100 %: above y = 0 is the top ruler.
    let screen = ScreenFrame { origin: Point::ZERO, right: Vec2::new(1.0, 0.0), down: Vec2::new(0.0, 1.0), size: (800.0, 600.0) };
    let c = ToolContext { screen: Some(screen), ..cx(&d, &s, &p) };
    let m = Mods::default();
    let gesture = |to: Point, mods: Mods| {
        let mut g = GuideEdit::default();
        g.press(&c, &ev(PointerKind::Down, 50.0, 400.0, m));
        g.pointer(&c, &ev(PointerKind::Drag, to.x, to.y, mods));
        g.pointer(&c, &ev(PointerKind::Up, to.x, to.y, mods)).unwrap()
    };
    assert_eq!(gesture(Point::new(50.0, -6.0), m), vec![Action::Cancel, Action::Exec("guide.remove".into(), json!({"index": 1}))]);
    assert_eq!(gesture(Point::new(50.0, -6.0), Mods { alt: true, ..m }), vec![Action::Cancel], "a copy is just not made");
    // Off the left side: not its ruler.
    assert_eq!(gesture(Point::new(-6.0, 300.0), m), vec![Action::Commit]);
    // Headless (no window): no ruler to drop it on.
    let mut g = GuideEdit::default();
    let c = cx(&d, &s, &p);
    g.press(&c, &ev(PointerKind::Down, 50.0, 400.0, m));
    g.pointer(&c, &ev(PointerKind::Drag, 50.0, -6.0, m));
    assert_eq!(g.pointer(&c, &ev(PointerKind::Up, 50.0, -6.0, m)), Some(vec![Action::Commit]));
}

// Tick positions are explicit fixtures in each unit, independent of the snap implementation.
fn origin_tick_cases() -> Vec<(vectorcraft_doc::Unit, f64, Point, Point, Point, f64)> {
    use vectorcraft_doc::Unit;
    let mm = |v| Unit::Millimeters.to_pt(v);
    let inch = |v| Unit::Inches.to_pt(v);
    vec![
        (Unit::Points, 1.0, Point::new(17.0, 23.0), Point::new(223.0, 237.0), Point::new(222.0, 238.0), 5.0),
        (Unit::Points, 1.0, Point::new(-17.0, -23.0), Point::new(223.0, 237.0), Point::new(223.0, 237.0), 5.0),
        (Unit::Points, 5.0, Point::new(17.25, 23.75), Point::new(223.4, 237.2), Point::new(223.25, 236.75), 1.0),
        (Unit::Millimeters, 1.0, Point::new(mm(1.0), mm(-1.0)), Point::new(mm(78.7), mm(83.6)), Point::new(mm(78.5), mm(84.0)), mm(2.5)),
        (
            Unit::Inches,
            2.0,
            Point::new(inch(0.01), inch(0.03)),
            Point::new(inch(3.07), inch(3.27)),
            // An inch ruler's ticks are binary fractions, so at zoom 2 they are 1/16 in
            // apart, not the 0.05 in a decimal ladder gave (#2827).
            Point::new(inch(3.0725), inch(3.28)),
            inch(0.0625),
        ),
    ]
}

fn guide_preview_position(actions: &[Action], vertical: bool, from: Option<f64>) -> f64 {
    let Some(Action::Preview(command, p)) = actions.iter().find(|a| matches!(a, Action::Preview(_, _))) else {
        panic!("missing guide preview: {actions:?}");
    };
    if let Some(from) = from {
        assert_eq!(command, "guide.move");
        from + p[if vertical { "dx" } else { "dy" }].as_f64().unwrap()
    } else {
        assert_eq!(command, "guide.add");
        assert_eq!(p["vertical"], json!(vertical));
        p["pos"].as_f64().unwrap()
    }
}

#[test]
fn ruler_guide_shift_new_uses_document_origin() {
    for (unit, zoom, origin, pointer, expected, tick) in origin_tick_cases() {
        let (mut doc, _) = doc_with_rect();
        doc.ruler_origin = origin;
        doc.rulers = vectorcraft_doc::RulerMode::Global;
        let (selection, paint) = (Selection::default(), paint());
        let c = ToolContext { unit, zoom, smart_guides: false, snap_to_point: false, ..cx(&doc, &selection, &paint) };
        for vertical in [false, true] {
            let mut guide = NewGuide::new(&c, vertical, None);
            let mods = Mods { shift: true, ..Default::default() };
            let actions = guide.pointer(&c, &ev(PointerKind::Drag, pointer.x, pointer.y, mods), true);
            assert_eq!(actions.first(), Some(&Action::Begin("New Guide".into())));
            let want = if vertical { expected.x } else { expected.y };
            let actual = guide_preview_position(&actions, vertical, None);
            assert!((actual - want).abs() < 1e-8, "{unit:?} {origin:?} vertical={vertical} actual={actual} want={want}");
            assert!(guide.pointer(&c, &ev(PointerKind::Drag, pointer.x, pointer.y, mods), true).is_empty());
            // Release updates to its own tick rather than committing a stale preview.
            let actions = guide.pointer(&c, &ev(PointerKind::Up, pointer.x + tick * 2.0, pointer.y + tick * 2.0, mods), true);
            assert!((guide_preview_position(&actions, vertical, None) - (want + tick * 2.0)).abs() < 1e-8);
            assert_eq!(actions.last(), Some(&Action::Commit));
            assert!(guide.overlays(&c).is_empty());
        }
    }
}

#[test]
fn ruler_guide_shift_move_uses_document_origin() {
    for (unit, zoom, origin, pointer, expected, _) in origin_tick_cases() {
        let mut doc = guides_doc();
        doc.ruler_origin = origin;
        doc.rulers = vectorcraft_doc::RulerMode::Global;
        let paint = paint();
        for (index, vertical, from) in [(0, true, 300.0), (1, false, 400.0)] {
            let mut selection = Selection::default();
            selection.set_guides([index]);
            let c = ToolContext { unit, zoom, smart_guides: false, snap_to_point: false, ..cx(&doc, &selection, &paint) };
            for copy in [false, true] {
                let mut guide = GuideEdit::default();
                let (start, to, want) = if vertical {
                    (Point::new(from, 50.0), Point::new(pointer.x, 50.0), expected.x)
                } else {
                    (Point::new(50.0, from), Point::new(50.0, pointer.y), expected.y)
                };
                assert!(guide.press(&c, &ev(PointerKind::Down, start.x, start.y, Mods::default())).unwrap().is_empty());
                let mods = Mods { shift: true, alt: copy, ..Default::default() };
                let actions = guide.pointer(&c, &ev(PointerKind::Drag, to.x, to.y, mods)).unwrap();
                assert_eq!(actions.first(), Some(&Action::Begin(if copy { "Copy Guide" } else { "Move Guide" }.into())));
                let actual = guide_preview_position(&actions, vertical, Some(from));
                assert!((actual - want).abs() < 1e-8, "{unit:?} {origin:?} vertical={vertical} copy={copy} actual={actual} want={want}");
                assert!(matches!(actions.last(), Some(Action::Preview(_, p)) if p["copy"] == json!(copy)));
                assert_eq!(guide.pointer(&c, &ev(PointerKind::Up, to.x, to.y, mods)), Some(vec![Action::Commit]));
                assert!(!guide.busy());
            }
        }
    }
}

#[test]
fn ruler_guide_shift_zero_and_invalid_origins_use_zero() {
    // One bad axis invalidates the entire origin, matching the app's ruler display.
    for origin in [
        Point::ZERO,
        Point::new(f64::NAN, 23.0),
        Point::new(17.0, f64::NAN),
        Point::new(f64::INFINITY, 23.0),
        Point::new(17.0, f64::NEG_INFINITY),
        Point::new(4_000_001.0, 23.0),
        Point::new(17.0, -4_000_001.0),
    ] {
        let mut doc = guides_doc();
        doc.ruler_origin = origin;
        doc.rulers = vectorcraft_doc::RulerMode::Global;
        let paint = paint();
        for (index, vertical, from, want) in [(0, true, 300.0, 225.0), (1, false, 400.0, 235.0)] {
            let mut selection = Selection::default();
            selection.set_guides([index]);
            let c = ToolContext { smart_guides: false, snap_to_point: false, ..cx(&doc, &selection, &paint) };
            let mods = Mods { shift: true, ..Default::default() };
            let mut new = NewGuide::new(&c, vertical, None);
            let actions = new.pointer(&c, &ev(PointerKind::Drag, 223.0, 237.0, mods), true);
            assert_eq!(guide_preview_position(&actions, vertical, None), want);
            let mut moved = GuideEdit::default();
            let (start, to) = if vertical { ((from, 50.0), (223.0, 50.0)) } else { ((50.0, from), (50.0, 237.0)) };
            moved.press(&c, &ev(PointerKind::Down, start.0, start.1, Mods::default())).unwrap();
            let actions = moved.pointer(&c, &ev(PointerKind::Drag, to.0, to.1, mods)).unwrap();
            assert_eq!(guide_preview_position(&actions, vertical, Some(from)), want);
        }
    }
}

#[test]
fn ruler_guide_origin_leaves_other_snap_modes_unchanged() {
    for origin in [Point::ZERO, Point::new(17.0, 23.0), Point::new(-17.0, -23.0)] {
        let mut doc = guides_doc();
        doc.ruler_origin = origin;
        doc.rulers = vectorcraft_doc::RulerMode::Global;
        let paint = paint();
        for (index, vertical, from) in [(0, true, 300.0), (1, false, 400.0)] {
            let mut selection = Selection::default();
            selection.set_guides([index]);
            let off = ToolContext { smart_guides: false, snap_to_point: false, ..cx(&doc, &selection, &paint) };
            for (c, pointer, expected) in [
                (ToolContext { ..off }, Point::new(226.4, 237.6), Point::new(226.4, 237.6)),
                (ToolContext { snap_to_pixel: true, ..off }, Point::new(226.4, 237.6), Point::new(226.0, 238.0)),
                (ToolContext { snap_to_grid: true, ..off }, Point::new(226.4, 237.6), Point::new(225.0, 234.0)),
                (ToolContext { smart_guides: true, ..off }, Point::new(203.0, 102.0), Point::new(200.0, 100.0)),
                (ToolContext { snap_to_point: true, ..off }, Point::new(201.0, 102.0), Point::new(200.0, 100.0)),
            ] {
                let want = if vertical { expected.x } else { expected.y };
                let mut new = NewGuide::new(&c, vertical, None);
                let actions = new.pointer(&c, &ev(PointerKind::Drag, pointer.x, pointer.y, Mods::default()), true);
                assert!((guide_preview_position(&actions, vertical, None) - want).abs() < 1e-8);
                let mut moved = GuideEdit::default();
                let (start, to) = if vertical { ((from, pointer.y), (pointer.x, pointer.y)) } else { ((pointer.x, from), (pointer.x, pointer.y)) };
                moved.press(&c, &ev(PointerKind::Down, start.0, start.1, Mods::default())).unwrap();
                let actions = moved.pointer(&c, &ev(PointerKind::Drag, to.0, to.1, Mods::default())).unwrap();
                assert!((guide_preview_position(&actions, vertical, Some(from)) - want).abs() < 1e-8);
            }
        }
    }
}

#[test]
fn ruler_guide_origin_readouts_are_relative_for_new_and_moved_guides() {
    for (origin, expected) in [
        (Point::new(17.0, 23.0), Point::new(205.0, 215.0)),
        (Point::new(-17.0, -23.0), Point::new(239.0, 261.0)),
        (Point::ZERO, Point::new(222.0, 238.0)),
        (Point::new(f64::NAN, 23.0), Point::new(222.0, 238.0)),
        (Point::new(17.0, 4_000_001.0), Point::new(222.0, 238.0)),
    ] {
        let mut doc = guides_doc();
        doc.ruler_origin = origin;
        doc.rulers = vectorcraft_doc::RulerMode::Global;
        let paint = paint();
        for unit in [vectorcraft_doc::Unit::Points, vectorcraft_doc::Unit::Millimeters] {
            for (index, vertical, from) in [(0, true, 300.0), (1, false, 400.0)] {
                let mut selection = Selection::default();
                selection.set_guides([index]);
                let c = ToolContext { unit, smart_guides: false, snap_to_point: false, ..cx(&doc, &selection, &paint) };
                let want = format!("{}: {}", if vertical { "X" } else { "Y" }, c.len(if vertical { expected.x } else { expected.y }));
                let check = |overlays: Vec<Overlay>| {
                    assert!(
                        matches!(overlays.last(), Some(Overlay::Measure { text, .. }) if text == &want),
                        "{origin:?} {unit:?}: {overlays:?}, want {want}"
                    );
                };
                let mut new = NewGuide::new(&c, vertical, None);
                let actions = new.pointer(&c, &ev(PointerKind::Drag, 222.0, 238.0, Mods::default()), true);
                assert_eq!(
                    guide_preview_position(&actions, vertical, None),
                    if vertical { 222.0 } else { 238.0 },
                    "commands keep absolute coordinates"
                );
                check(new.overlays(&c));
                for copy in [false, true] {
                    let mut moved = GuideEdit::default();
                    let (start, to) = if vertical { ((from, 50.0), (222.0, 50.0)) } else { ((50.0, from), (50.0, 238.0)) };
                    moved.press(&c, &ev(PointerKind::Down, start.0, start.1, Mods::default())).unwrap();
                    moved.pointer(&c, &ev(PointerKind::Drag, to.0, to.1, Mods { alt: copy, ..Default::default() })).unwrap();
                    check(moved.overlays(&c));
                }
                let quiet = ToolContext { measurement_labels: false, ..c };
                assert!(new.overlays(&quiet).is_empty(), "measurement-label preference is preserved");
            }
        }
    }
}

#[test]
fn ruler_guide_readouts_count_from_the_active_artboard_with_artboard_rulers() {
    let mut doc = guides_doc();
    doc.artboards.push(vectorcraft_doc::Artboard {
        id: 2,
        name: "Artboard 2".into(),
        rect: Rect::new(150.0, 200.0, 350.0, 400.0),
        locked: false,
        show_center_mark: false,
        show_cross_hairs: false,
        show_video_safe_areas: false,
        video_ruler_pixel_aspect_ratio: 1.0,
        color: None,
    });
    doc.ruler_origin = Point::new(5.0, 5.0);
    let (paint, selection) = (paint(), Selection::default());
    for (rulers, active, zero) in [
        (vectorcraft_doc::RulerMode::Artboard, 1, Point::new(150.0, 200.0)),
        (vectorcraft_doc::RulerMode::Artboard, 0, Point::new(doc.artboards[0].rect.x0, doc.artboards[0].rect.y0)),
        (vectorcraft_doc::RulerMode::Global, 1, Point::new(5.0, 5.0)),
    ] {
        doc.rulers = rulers;
        let c = ToolContext { ruler_origin: doc.ruler_origin_for(active), smart_guides: false, snap_to_point: false, ..cx(&doc, &selection, &paint) };
        assert_eq!(c.ruler_origin, zero, "{rulers:?} {active}");
        for vertical in [true, false] {
            let mut new = NewGuide::new(&c, vertical, None);
            new.pointer(&c, &ev(PointerKind::Drag, 222.0, 238.0, Mods::default()), true);
            let want = if vertical { format!("X: {}", c.len(222.0 - zero.x)) } else { format!("Y: {}", c.len(238.0 - zero.y)) };
            assert!(matches!(new.overlays(&c).last(), Some(Overlay::Measure { text, .. }) if text == &want), "{rulers:?} {active}: want {want}");
            // Shift puts the guide on a ruler tick counted from the same zero point.
            let snapped = new.pointer(&c, &ev(PointerKind::Drag, 222.0, 238.0, Mods { shift: true, ..Default::default() }), true);
            let at = guide_preview_position(&snapped, vertical, None);
            let from = if vertical { zero.x } else { zero.y };
            assert_eq!(at, from + c.unit.snap_to_ruler_tick(if vertical { 222.0 } else { 238.0 } - from, c.zoom), "{rulers:?} {active}");
        }
    }
}
