//! Clip groups clip by any clipping path (M3.26): compound holes, even-odd fills, the union of a
//! group's members, text outlines; with nothing to clip by the clipped art is hidden.

use crate::*;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, CharStyle};
use vectorcraft_geom::{PathData, Point, SubPath, shapes};

const BLACK: [u8; 4] = [0, 0, 0, 255];
const CLEAR: [u8; 4] = [0, 0, 0, 0];

fn path(d: &mut Document, p: PathData, rule: FillRule) -> Arc<Node> {
    let mut n = Node::path(d.alloc_id(), p, Appearance::basic(Paint::None, Paint::None, 0.0));
    if let NodeKind::Path { rule: r, .. } = &mut n.kind {
        *r = rule;
    }
    Arc::new(n)
}

fn rect(r: Rect) -> PathData {
    shapes::rectangle(r)
}

/// `clip` clipping a black square over the whole 100×100 canvas, rendered transparent.
fn clipped(mut d: Document, clip: Arc<Node>) -> Rendered {
    let art = Node::path(d.alloc_id(), rect(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    let g = Node::new(d.alloc_id(), NodeKind::Group { children: vec![clip, Arc::new(art)], clip: true });
    let l = d.layers[0].id;
    d.insert(Some(l), 0, g).unwrap();
    Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &RenderOptions::default())
}

#[test]
fn compound_clip_keeps_its_hole() {
    let mut d = Document::new(100.0, 100.0);
    let (outer, inner) = (rect(Rect::new(10.0, 10.0, 90.0, 90.0)), rect(Rect::new(35.0, 35.0, 65.0, 65.0)));
    let children = vec![path(&mut d, outer, FillRule::NonZero), path(&mut d, inner, FillRule::NonZero)];
    let donut = Arc::new(Node::new(d.alloc_id(), NodeKind::Compound { children, rule: FillRule::EvenOdd }));
    let r = clipped(d, donut);
    assert_eq!(r.pixel(20, 50), BLACK, "the ring");
    assert_eq!(r.pixel(50, 50), CLEAR, "the hole stays transparent");
    assert_eq!(r.pixel(5, 5), CLEAR);
}

#[test]
fn even_odd_star_clips_with_an_empty_centre() {
    let mut d = Document::new(100.0, 100.0);
    // A pentagram drawn in one stroke: its centre pentagon is outside under even-odd.
    let pts: Vec<Point> = (0..5)
        .map(|k| {
            let a = (k * 2 % 5) as f64 * std::f64::consts::TAU / 5.0 - std::f64::consts::FRAC_PI_2;
            Point::new(50.0 + 48.0 * a.cos(), 52.0 + 48.0 * a.sin())
        })
        .collect();
    let star = path(&mut d, PathData::single(SubPath::polyline(&pts, true)), FillRule::EvenOdd);
    let r = clipped(d, star);
    assert_eq!(r.pixel(50, 52), CLEAR, "the centre is empty");
    assert_eq!(r.pixel(50, 12), BLACK, "a point");
}

#[test]
fn group_clip_is_the_union_of_its_members() {
    let mut d = Document::new(100.0, 100.0);
    // Opposite windings: a plain concatenation would cancel out where they overlap.
    let mut b = rect(Rect::new(40.0, 0.0, 100.0, 50.0));
    b.reverse();
    let members = vec![path(&mut d, rect(Rect::new(0.0, 0.0, 60.0, 50.0)), FillRule::NonZero), path(&mut d, b, FillRule::NonZero)];
    let g = Arc::new(Node::group(d.alloc_id(), members));
    let r = clipped(d, g);
    for x in [20, 50, 80] {
        assert_eq!(r.pixel(x, 25), BLACK, "x = {x}");
    }
    assert_eq!(r.pixel(50, 75), CLEAR);
}

#[test]
fn text_clips_by_its_glyph_outlines() {
    let mut d = Document::new(100.0, 100.0);
    let t = vectorcraft_doc::TextObject::point(Point::new(5.0, 80.0), "O", CharStyle { size: 90.0, ..CharStyle::default() });
    let text = Arc::new(Node::new(d.alloc_id(), NodeKind::Text(Box::new(t))));
    let b = text.geometric_bounds().unwrap();
    let r = clipped(d, text);
    let cy = ((b.y0 + b.y1) / 2.0) as u32;
    // Across the middle of the "O": clear, ring, the counter (clear), ring, clear.
    let row: Vec<bool> = (0..100).map(|x| r.pixel(x, cy)[3] > 128).collect();
    let runs = row.windows(2).filter(|w| w[0] != w[1]).count();
    assert_eq!(runs, 4, "two ring crossings with the counter between: {row:?}");
}

#[test]
fn nothing_to_clip_by_hides_the_art() {
    let mut d = Document::new(100.0, 100.0);
    let empty = Arc::new(Node::group(d.alloc_id(), vec![]));
    let r = clipped(d, empty);
    assert_eq!(r.pixel(50, 50), CLEAR);
}

#[test]
fn the_clipping_paths_geometry_effects_shape_the_clip() {
    // A Transform effect moving the left half 50 pt right: the right half shows instead.
    let mut d = Document::new(100.0, 100.0);
    let mut clip = Node::path(d.alloc_id(), rect(Rect::new(0.0, 0.0, 50.0, 100.0)), Appearance::basic(Paint::None, Paint::None, 0.0));
    clip.appearance.effects.push(vectorcraft_doc::Effect { id: "distort.transform".into(), params: serde_json::json!({"moveH": 50}), visible: true });
    let r = clipped(d, Arc::new(clip));
    assert_eq!(r.pixel(25, 50), CLEAR, "the clipping path's own place");
    assert_eq!(r.pixel(75, 50), BLACK, "where its effect moves it");
}

#[test]
fn a_view_of_only_the_moved_clip_draws_the_clipped_art() {
    // The clipping path covers x 0 to 50 and its Transform effect moves it to x 50 to 100; the
    // view shows x 60 to 100, past the clipping path's own shape.
    let mut d = Document::new(100.0, 100.0);
    let mut clip = Node::path(d.alloc_id(), rect(Rect::new(0.0, 0.0, 50.0, 100.0)), Appearance::basic(Paint::None, Paint::None, 0.0));
    clip.appearance.effects.push(vectorcraft_doc::Effect { id: "distort.transform".into(), params: serde_json::json!({"moveH": 50}), visible: true });
    let art = Node::path(d.alloc_id(), rect(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    let members = vec![Arc::new(clip), Arc::new(art)];
    let g = Node::new(d.alloc_id(), NodeKind::Group { children: members.clone(), clip: true });
    // The same in a sublayer that clips by its first object.
    let mut sublayer = (*d.layers[0]).clone();
    sublayer.id = d.alloc_id();
    if let NodeKind::Layer { children, clip, .. } = &mut sublayer.kind {
        (*children, *clip) = (members, true);
    }
    for clipping in [g, sublayer] {
        let mut d = d.clone();
        let l = d.layers[0].id;
        d.insert(Some(l), 0, clipping).unwrap();
        let r = Renderer::new().render(&d, 40, 100, Affine::translate((-60.0, 0.0)), &RenderOptions::default());
        assert_eq!(r.pixel(20, 50), BLACK, "x 80, inside the moved clip");
    }
}

#[test]
fn clip_regions_are_cached_per_clipping_path() {
    let mut d = Document::new(100.0, 100.0);
    let clip = path(&mut d, rect(Rect::new(0.0, 0.0, 50.0, 100.0)), FillRule::NonZero);
    let mut r = Renderer::new();
    let first = r.clip_of(&clip).unwrap();
    assert!(Arc::ptr_eq(&first, &r.clip_of(&clip).unwrap()));
    assert_eq!(first.1, FillRule::NonZero);
}
