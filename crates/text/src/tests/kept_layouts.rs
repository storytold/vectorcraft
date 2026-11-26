//! Layouts kept for the first frame: found only for an equal text object at the same stamp, taken
//! once, and limited in bytes.

use kurbo::{Affine, Point};
use vectorcraft_doc::color::{Color, Gradient, GradientPaint, Paint};
use vectorcraft_doc::{CharStyle, Dash, RubySpan, TabAlign, TabStop, TextKind, TextObject, TextWrap, WrapShape};
use vectorcraft_geom::PathData;

use crate::kept::test_shelf::{FRAMES, PER_HASH, TestShelf, bytes_of, stamp};
use crate::{FontDb, ListMarker, TextLayout, layout};

const BUDGET: usize = 1 << 30;

fn object(text: &str) -> TextObject {
    TextObject::point(Point::ZERO, text, CharStyle::default())
}

fn laid_out(t: &TextObject) -> TextLayout {
    layout(FontDb::global(), t)
}

#[test]
fn a_kept_layout_is_taken_once() {
    let at = stamp(1, true, 1);
    let t = object("Type");
    let mut shelf = TestShelf::default();
    shelf.keep(&t, at, at, laid_out(&t), BUDGET);
    let taken = shelf.take(&t.clone(), at).unwrap();
    assert_eq!(taken.glyphs.len(), 4);
    assert!(shelf.take(&t, at).is_none());
    assert_eq!(shelf.bytes(), 0);
}

#[test]
fn equal_objects_take_a_layout_each() {
    let at = stamp(1, true, 1);
    let t = object("Type");
    let mut shelf = TestShelf::default();
    shelf.keep(&t, at, at, laid_out(&t), BUDGET);
    shelf.keep(&t, at, at, laid_out(&t), BUDGET);
    assert!(shelf.take(&t, at).is_some() && shelf.take(&t, at).is_some());
    assert!(shelf.take(&t, at).is_none());
}

#[test]
fn a_changed_object_does_not_find_the_layout() {
    let at = stamp(1, true, 1);
    let t = object("Type");
    fn frame(w: f64) -> TextKind {
        TextKind::Area { frame: PathData::from_bezpath(&kurbo::Shape::to_path(&kurbo::Rect::new(0.0, 0.0, w, 50.0), 0.1)) }
    }
    let changes: [fn(&mut TextObject); 11] = [
        |t| t.runs[0].text = "Typo".into(),
        |t| t.runs[0].style.size += 1.0,
        |t| t.runs[0].style.font_family = "Inter".into(),
        |t| t.runs[0].style.font_style = "Bold".into(),
        |t| t.runs[0].style.language = Some("tr".into()),
        |t| t.xf = Affine::translate((1.0, 0.0)),
        |t| t.vertical = true,
        |t| t.para.first_line_indent = 3.0,
        |t| t.first_para_continues = true,
        |t| t.area.columns = 2,
        |t| t.kind = frame(100.0),
    ];
    for (i, change) in changes.iter().enumerate() {
        let mut shelf = TestShelf::default();
        shelf.keep(&t, at, at, laid_out(&t), BUDGET);
        let mut other = t.clone();
        change(&mut other);
        assert!(shelf.take(&other, at).is_none(), "change {i}");
        assert!(shelf.take(&t, at).is_some(), "change {i}: the object itself still finds it");
    }
    // Two frame shapes of the same size, each a text object of its own.
    let mut area = t.clone();
    area.kind = frame(100.0);
    let mut wider = t.clone();
    wider.kind = frame(120.0);
    let mut shelf = TestShelf::default();
    shelf.keep(&area, at, at, laid_out(&area), BUDGET);
    assert!(shelf.take(&wider, at).is_none());
    assert!(shelf.take(&area, at).is_some());
}

#[test]
fn a_changed_stamp_drops_the_layouts() {
    let t = object("Type");
    let at = stamp(1, true, 1);
    for later in [stamp(2, true, 1), stamp(1, false, 1), stamp(1, true, 2)] {
        let mut shelf = TestShelf::default();
        shelf.keep(&t, at, at, laid_out(&t), BUDGET);
        assert!(shelf.take(&t, later).is_none(), "{later:?}");
        assert_eq!(shelf.bytes(), 0, "{later:?}");
        assert!(shelf.take(&t, at).is_none(), "{later:?}: gone for good");
    }
}

#[test]
fn a_layout_made_while_the_stamp_changed_is_not_kept() {
    let t = object("Type");
    let mut shelf = TestShelf::default();
    shelf.keep(&t, stamp(1, true, 1), stamp(2, true, 1), laid_out(&t), BUDGET);
    assert_eq!(shelf.bytes(), 0);
    assert!(shelf.take(&t, stamp(1, true, 1)).is_none() && shelf.take(&t, stamp(2, true, 1)).is_none());
}

#[test]
fn keeping_at_a_new_stamp_lets_the_older_layouts_go() {
    let (old, new) = (stamp(1, true, 1), stamp(2, true, 1));
    let (a, b) = (object("Old"), object("New"));
    let mut shelf = TestShelf::default();
    shelf.keep(&a, old, old, laid_out(&a), BUDGET);
    shelf.keep(&b, new, new, laid_out(&b), BUDGET);
    assert!(shelf.take(&a, new).is_none());
    assert!(shelf.take(&b, new).is_some());
}

#[test]
fn the_budget_lets_the_oldest_layouts_go() {
    let at = stamp(1, true, 1);
    let (a, b, c) = (object("Abc"), object("Def"), object("Ghi"));
    let (la, lb, lc) = (laid_out(&a), laid_out(&b), laid_out(&c));
    let budget = bytes_of(&b, &lb) + bytes_of(&c, &lc);
    assert!(bytes_of(&a, &la) + budget > budget);
    let mut shelf = TestShelf::default();
    shelf.keep(&a, at, at, la, budget);
    shelf.keep(&b, at, at, lb, budget);
    shelf.keep(&c, at, at, lc, budget);
    assert!(shelf.bytes() <= budget);
    assert!(shelf.take(&a, at).is_none(), "the oldest went");
    assert!(shelf.take(&b, at).is_some() && shelf.take(&c, at).is_some());
    assert_eq!(shelf.bytes(), 0);
}

#[test]
fn a_layout_larger_than_the_budget_is_not_kept_and_lets_nothing_go() {
    let at = stamp(1, true, 1);
    let (small, large) = (object("A"), object(&"Long text ".repeat(50)));
    let (ls, ll) = (laid_out(&small), laid_out(&large));
    let budget = bytes_of(&small, &ls);
    assert!(bytes_of(&large, &ll) > budget);
    let mut shelf = TestShelf::default();
    shelf.keep(&small, at, at, ls, budget);
    shelf.keep(&large, at, at, ll, budget);
    assert!(shelf.take(&large, at).is_none());
    assert!(shelf.take(&small, at).is_some());
}

#[test]
fn a_frame_that_took_a_layout_lets_the_others_go() {
    let at = stamp(1, true, 1);
    let (a, b) = (object("Seen"), object("Out of view"));
    let mut shelf = TestShelf::default();
    shelf.keep(&a, at, at, laid_out(&a), BUDGET);
    shelf.keep(&b, at, at, laid_out(&b), BUDGET);
    shelf.frame_done();
    assert!(shelf.bytes() > 0, "a frame that took none lets none go");
    assert!(shelf.take(&a, at).is_some());
    shelf.frame_done();
    assert_eq!(shelf.bytes(), 0);
    assert!(shelf.take(&b, at).is_none());
}

#[test]
fn a_layout_with_color_art_is_not_kept() {
    let at = stamp(1, true, 1);
    let t = object("Type");
    let mut l = laid_out(&t);
    l.glyphs[0].colour = Some(std::sync::Arc::new(vectorcraft_doc::Node::group(vectorcraft_doc::NodeId(1), vec![])));
    let mut shelf = TestShelf::default();
    shelf.keep(&t, at, at, l, BUDGET);
    assert_eq!(shelf.bytes(), 0);
    assert!(shelf.take(&t, at).is_none());
}

#[test]
fn an_object_with_a_gradient_is_not_kept() {
    let at = stamp(1, true, 1);
    let gradient = Paint::Gradient(Box::new(GradientPaint::new(Gradient::default())));
    for stroke in [false, true] {
        let mut t = object("Type");
        if stroke {
            t.runs[0].style.stroke = gradient.clone();
        } else {
            t.runs[0].style.fill = gradient.clone();
        }
        let mut shelf = TestShelf::default();
        shelf.keep(&t, at, at, laid_out(&t), BUDGET);
        assert_eq!(shelf.bytes(), 0, "stroke: {stroke}");
    }
}

/// Objects that differ only past the hash stay few enough to compare.
#[test]
fn one_hash_holds_a_limited_number_of_layouts() {
    let at = stamp(1, true, 1);
    let objects: Vec<TextObject> = (0..PER_HASH + 2)
        .map(|i| {
            let mut t = object("Type");
            t.para.first_line_indent = i as f64;
            t
        })
        .collect();
    let mut shelf = TestShelf::default();
    for t in &objects {
        shelf.keep(t, at, at, laid_out(t), BUDGET);
    }
    let taken = objects.iter().filter(|t| shelf.take(t, at).is_some()).count();
    assert_eq!(taken, PER_HASH);
    assert_eq!(shelf.bytes(), 0);
}

#[test]
fn a_frame_still_being_drawn_keeps_the_layouts_it_may_take() {
    let at = stamp(1, true, 1);
    let (a, b) = (object("Canvas"), object("Other pane"));
    let mut shelf = TestShelf::default();
    shelf.keep(&a, at, at, laid_out(&a), BUDGET);
    shelf.keep(&b, at, at, laid_out(&b), BUDGET);
    shelf.frame_begin();
    shelf.frame_begin();
    assert!(shelf.take(&a, at).is_some());
    shelf.frame_done();
    assert!(shelf.take(&b, at).is_some(), "the other frame takes its layout after the first ends");
    shelf.keep(&a, at, at, laid_out(&a), BUDGET);
    shelf.frame_done();
    assert_eq!(shelf.bytes(), 0, "the last frame to end lets the rest go");
}

#[test]
fn frames_that_take_nothing_let_the_layouts_go() {
    let at = stamp(1, true, 1);
    let t = object("Out of view");
    let mut shelf = TestShelf::default();
    shelf.keep(&t, at, at, laid_out(&t), BUDGET);
    for _ in 1..FRAMES {
        shelf.frame_done();
    }
    assert!(shelf.bytes() > 0, "the last frame has not ended");
    // Another open starts the count again.
    shelf.keep(&t, at, at, laid_out(&t), BUDGET);
    shelf.frame_done();
    assert!(shelf.bytes() > 0, "a keep starts the count again");
    for _ in 1..FRAMES {
        shelf.frame_done();
    }
    assert_eq!(shelf.bytes(), 0);
    assert!(shelf.take(&t, at).is_none());
}

#[test]
fn the_budget_counts_frames_paths_and_markers() {
    let t = object("Type");
    let mut l = laid_out(&t);
    let plain = bytes_of(&t, &l);
    l.frames = vec![kurbo::Rect::ZERO; 1000];
    let with_frames = bytes_of(&t, &l);
    l.markers = vec![ListMarker { glyphs: 0..0, tab: 0, text: "x".repeat(1000) }];
    let with_markers = bytes_of(&t, &l);
    let mut area = t.clone();
    let wide = kurbo::BezPath::from_vec(
        std::iter::once(kurbo::PathEl::MoveTo(kurbo::Point::ZERO))
            .chain((1..1000).map(|i| kurbo::PathEl::LineTo(kurbo::Point::new(f64::from(i), 0.0))))
            .collect(),
    );
    area.kind = TextKind::Area { frame: PathData::from_bezpath(&wide) };
    let mut wrapped = t.clone();
    wrapped.wrap = vec![WrapShape { path: PathData::from_bezpath(&wide), wrap: TextWrap::default() }];
    let mut paragraphs = t.clone();
    paragraphs.paras = vec![paragraphs.para.clone(); 100];
    let mut styled = t.clone();
    styled.runs[0].style.font_family = "x".repeat(1000);
    let mut featured = t.clone();
    featured.runs[0].style.features = vec!["x".repeat(1000)];
    let mut swatched = t.clone();
    swatched.runs[0].style.fill = Paint::Solid { color: Color::BLACK, swatch: Some("x".repeat(1000)), tint: 1.0 };
    let mut dashed = t.clone();
    dashed.runs[0].style.stroke_dash = Some(Dash { pattern: vec![1.0; 1000], ..Dash::default() });
    let mut tabbed = t.clone();
    tabbed.para.tabs = vec![TabStop { position: 10.0, align: TabAlign::default(), leader: "x".repeat(1000), align_on: '.' }];
    let mut with_ruby = t.clone();
    with_ruby.ruby = vec![RubySpan::new(0, 1, "x".repeat(1000))];
    let mut with_carets = laid_out(&t);
    let no_carets = bytes_of(&t, &with_carets);
    with_carets.glyphs[0].carets = vec![(0, 0.0); 1000];
    let anchors = 999 * size_of::<vectorcraft_geom::Anchor>();
    let counted = [
        ("frames", with_frames >= plain + 1000 * size_of::<kurbo::Rect>()),
        ("markers", with_markers >= with_frames + 1000),
        ("the frame's path", bytes_of(&area, &l) >= with_markers + anchors),
        ("the wrap's path", bytes_of(&wrapped, &l) >= with_markers + anchors),
        ("paragraphs", bytes_of(&paragraphs, &l) >= with_markers + 100 * size_of::<vectorcraft_doc::ParaStyle>()),
        ("a run's family", bytes_of(&styled, &l) >= with_markers + 900),
        ("a run's features", bytes_of(&featured, &l) >= with_markers + 1000),
        ("a swatch's name", bytes_of(&swatched, &l) >= with_markers + 1000),
        ("a stroke's dashes", bytes_of(&dashed, &l) >= with_markers + 1000 * size_of::<f64>()),
        ("a tab's leader", bytes_of(&tabbed, &l) >= with_markers + 1000),
        ("ruby text", bytes_of(&with_ruby, &l) >= with_markers + 1000),
        ("carets", bytes_of(&t, &with_carets) >= no_carets + 1000 * size_of::<(usize, f64)>()),
    ];
    let missed: Vec<&str> = counted.iter().filter(|(_, ok)| !ok).map(|(name, _)| *name).collect();
    assert!(missed.is_empty(), "not counted: {missed:?}");
}

#[test]
fn a_taken_layout_stays_within_the_budget_after_many_keeps() {
    // Taken layouts leave nothing behind that a later keep has to count.
    let at = stamp(1, true, 1);
    let t = object("Type");
    let budget = bytes_of(&t, &laid_out(&t));
    let mut shelf = TestShelf::default();
    for _ in 0..50 {
        shelf.keep(&t, at, at, laid_out(&t), budget);
        assert!(shelf.take(&t, at).is_some());
        assert_eq!(shelf.bytes(), 0);
    }
}
