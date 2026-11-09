//! Drawing again only what an edit changed gives the pixels of drawing it all again.

use crate::*;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::Appearance;
use vectorcraft_geom::{Anchor, PathData, Point, SubPath, Vec2, shapes};

const W: u32 = 160;
const H: u32 = 120;

/// A view that doesn't line the art up with the pixels, so the edges are anti-aliased.
fn view() -> Affine {
    Affine::translate((3.3, 2.7)) * Affine::scale(1.37)
}

fn rect(d: &mut Document, r: Rect, fill: Color) -> Node {
    let id = d.alloc_id();
    Node::path(id, shapes::rectangle(r), Appearance::basic(Paint::solid(fill), Paint::solid(Color::BLACK), 2.0))
}

fn shadow() -> vectorcraft_doc::Effect {
    vectorcraft_doc::Effect {
        id: "stylize.dropShadow".into(),
        params: serde_json::json!({"x": 6.0, "y": 6.0, "blur": 4.0, "opacity": 75.0}),
        visible: true,
    }
}

/// A layer with a rectangle, a group at half opacity holding two, a clip group, and one with a
/// drop shadow; the ids of the loose rectangle, a member of each group, and the shadowed one.
fn document() -> (Document, [NodeId; 5]) {
    let mut d = Document::new(110.0, 80.0);
    let l = d.layers[0].id;
    let loose = rect(&mut d, Rect::new(5.0, 5.0, 25.0, 20.0), Color::rgb(1.0, 0.0, 0.0));
    let loose = d.insert(Some(l), 0, loose).unwrap();
    let (a, b) = (
        rect(&mut d, Rect::new(30.0, 5.0, 50.0, 25.0), Color::rgb(0.0, 0.6, 0.0)),
        rect(&mut d, Rect::new(40.0, 15.0, 60.0, 35.0), Color::rgb(0.0, 0.0, 1.0)),
    );
    let mut group = Node::new(d.alloc_id(), NodeKind::Group { children: vec![Arc::new(a), Arc::new(b)], clip: false });
    group.opacity = 0.5;
    let member = group.children().unwrap()[1].id;
    d.insert(Some(l), 1, group).unwrap();
    let (mask, art) =
        (rect(&mut d, Rect::new(5.0, 40.0, 30.0, 60.0), Color::WHITE), rect(&mut d, Rect::new(0.0, 45.0, 40.0, 55.0), Color::rgb(0.8, 0.4, 0.0)));
    let clipped = art.id;
    let clip = Node::new(d.alloc_id(), NodeKind::Group { children: vec![Arc::new(mask), Arc::new(art)], clip: true });
    d.insert(Some(l), 2, clip).unwrap();
    let mut shadowed = rect(&mut d, Rect::new(60.0, 45.0, 80.0, 60.0), Color::rgb(0.9, 0.9, 0.2));
    shadowed.appearance.effects.push(shadow());
    let shadowed = d.insert(Some(l), 3, shadowed).unwrap();
    let other = rect(&mut d, Rect::new(85.0, 10.0, 105.0, 30.0), Color::rgb(0.5, 0.0, 0.5));
    let other = d.insert(Some(l), 4, other).unwrap();
    (d, [loose, member, clipped, shadowed, other])
}

/// Draws `new` again only where it changed from `old`, over `old` drawn whole, and checks that
/// this is `new` drawn whole; returns what changed. Measured afresh ([`changed`]) and by the
/// renderer that drew `old`, which then draws the part ([`Renderer::changed`]): both must hold.
fn redraw(old: &Document, new: &Document) -> Changed {
    let opts = RenderOptions::default();
    let after = Renderer::new().render(new, W, H, view(), &opts);
    let change = changed(old, new);
    let mut fresh = Renderer::new();
    let before = fresh.render(old, W, H, view(), &opts);
    patched(&mut fresh, &before, new, change, &after, &opts);
    let mut r = Renderer::new();
    let before = r.render(old, W, H, view(), &opts);
    let measured = r.changed(old, new, &opts);
    patched(&mut r, &before, new, measured, &after, &opts);
    change
}

/// `before` with `new` drawn again by `r` with `opts` only where `change` says, checked against
/// `after`.
fn patched(r: &mut Renderer, before: &Rendered, new: &Document, change: Changed, after: &Rendered, opts: &RenderOptions) {
    let mut patched = Rendered { width: W, height: H, pixels: before.pixels.clone() };
    match change {
        Changed::Everything => patched.pixels = after.pixels.clone(),
        Changed::Nothing => {}
        Changed::Area(area) => {
            if let Some([x0, y0, x1, y1]) = view_pixels(area, view(), W, H) {
                let part = r.render(new, x1 - x0, y1 - y0, Affine::translate((-f64::from(x0), -f64::from(y0))) * view(), opts);
                for y in 0..y1 - y0 {
                    let (from, to) = ((y * (x1 - x0) * 4) as usize, (((y0 + y) * W + x0) * 4) as usize);
                    let n = ((x1 - x0) * 4) as usize;
                    patched.pixels[to..to + n].copy_from_slice(&part.pixels[from..from + n]);
                }
            }
        }
    }
    assert!(patched.pixels == after.pixels, "{change:?}: drawn again only there, it differs from drawing it all");
}

fn area(c: Changed) -> Rect {
    match c {
        Changed::Area(a) => a,
        other => panic!("an area: {other:?}"),
    }
}

#[test]
fn moving_an_object_changes_where_it_was_and_is() {
    let (old, [loose, ..]) = document();
    let mut new = old.clone();
    new.node_mut(loose).unwrap().transform(Affine::translate((7.0, 3.0)), false);
    let a = area(redraw(&old, &new));
    assert!(a.contains(Point::new(6.0, 6.0)) && a.contains(Point::new(31.0, 22.0)), "before and after: {a:?}");
    assert!(a.x1 < 35.0, "not the objects beside it: {a:?}");
}

#[test]
fn recolouring_a_member_of_a_translucent_group_changes_only_the_member() {
    let (old, [_, member, ..]) = document();
    let mut new = old.clone();
    new.node_mut(member).unwrap().appearance.fill_mut().unwrap().paint = Paint::solid(Color::rgb(1.0, 0.5, 0.0));
    let a = area(redraw(&old, &new));
    assert!(a.x0 > 35.0, "not the group's other member: {a:?}");
}

#[test]
fn moving_clipped_art_changes_its_whole_clip_group() {
    let (old, [_, _, clipped, ..]) = document();
    let mut new = old.clone();
    new.node_mut(clipped).unwrap().transform(Affine::translate((0.0, 4.0)), false);
    redraw(&old, &new);
}

#[test]
fn moving_a_shadowed_object_changes_where_its_shadow_falls() {
    let (old, [.., shadowed, _]) = document();
    let mut new = old.clone();
    new.node_mut(shadowed).unwrap().transform(Affine::translate((-5.0, 5.0)), false);
    let a = area(redraw(&old, &new));
    assert!(a.x1 > 86.0 && a.y1 > 70.0, "the shadow, before and after: {a:?}");
}

#[test]
fn adding_removing_and_restacking_change_those_objects() {
    let (old, [loose, _, _, _, other]) = document();
    let mut added = old.clone();
    let n = rect(&mut added, Rect::new(70.0, 5.0, 90.0, 15.0), Color::BLACK);
    let l = added.layers[0].id;
    added.insert(Some(l), 5, n).unwrap();
    redraw(&old, &added);
    let mut removed = old.clone();
    removed.remove(loose).unwrap();
    redraw(&old, &removed);
    let mut restacked = old.clone();
    let n = restacked.remove(other).unwrap();
    restacked.insert(Some(l), 0, (*n).clone()).unwrap();
    redraw(&old, &restacked);
}

/// A blur too wide for a raster of all of it is cropped to the frame. A part of the frame drawn
/// again, during a drag too, draws it from that raster: the same pixels, and no preview at the
/// drag's lower resolution.
#[test]
fn a_part_of_a_frame_draws_a_cropped_blur_from_the_frames_raster() {
    let (mut d, _) = document();
    let mut back = rect(&mut d, Rect::new(0.0, 0.0, 110.0, 80.0), Color::rgb(0.1, 0.3, 0.9));
    back.appearance.effects.push(vectorcraft_doc::Effect { id: "blur.gaussian".into(), params: serde_json::json!({"radius": 500.0}), visible: true });
    let l = d.layers[0].id;
    d.insert(Some(l), 0, back).unwrap();
    let view = Affine::scale(2.0) * Affine::translate((-20.0, -15.0));
    let mut r = Renderer::new();
    let whole = r.render(&d, W, H, view, &RenderOptions::default());
    let [x0, y0, x1, y1] = [40, 30, 100, 80];
    let dragging = RenderOptions { interacting: true, dragging: true, ..Default::default() };
    let part = r.render(&d, x1 - x0, y1 - y0, Affine::translate((-f64::from(x0), -f64::from(y0))) * view, &dragging);
    assert_eq!(r.stats.previewed, 0, "drawn from the frame's raster");
    for y in 0..y1 - y0 {
        for x in 0..x1 - x0 {
            assert_eq!(part.pixel(x, y), whole.pixel(x0 + x, y0 + y), "at {x}, {y}");
        }
    }
}

/// Type on a path whose path a script reshapes: its letters stand above the path and run past
/// where it ends, and they change where they were and where they are, outside the path's box.
#[test]
fn reshaping_the_path_of_type_on_a_path_changes_where_its_letters_were_and_are() {
    let mut old = Document::new(110.0, 80.0);
    let l = old.layers[0].id;
    let on = |y: f64| {
        let mut t = vectorcraft_doc::text::TextObject::point(
            Point::ZERO,
            "Text along a path",
            vectorcraft_doc::text::CharStyle { size: 14.0, ..Default::default() },
        );
        let line = kurbo::Line::new((10.0, 60.0), (90.0, y)).to_path(0.1);
        t.kind = vectorcraft_doc::TextKind::OnPath { path: vectorcraft_geom::PathData::from_bezpath(&line), start: 0.0, end: None };
        t.xf = Affine::IDENTITY;
        // The engine lays out type that changed ([`vectorcraft_doc::TextObject::cached_bounds`]).
        t.cached_bounds = Some(vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t).bounds);
        t
    };
    let text = old.insert(Some(l), 0, Node::new(NodeId(0), NodeKind::Text(Box::new(on(55.0))))).unwrap();
    let mut new = old.clone();
    if let Some(NodeKind::Text(t)) = new.node_mut(text).map(|n| &mut n.kind) {
        **t = on(25.0);
    }
    redraw(&old, &new);
}

/// Stroked type with sharp corners: the miter spikes of its characters' strokes reach past half
/// the weight, and change where they were and where they are.
#[test]
fn moving_type_with_mitred_strokes_changes_where_its_spikes_reach() {
    let mut old = Document::new(110.0, 80.0);
    let l = old.layers[0].id;
    let style = vectorcraft_doc::text::CharStyle {
        size: 30.0,
        stroke: Paint::solid(Color::BLACK),
        stroke_width: 10.0,
        stroke_join: vectorcraft_doc::LineJoin::Miter,
        stroke_miter_limit: 10.0,
        ..Default::default()
    };
    let mut t = vectorcraft_doc::text::TextObject::point(Point::new(10.0, 50.0), "VAWV", style);
    t.cached_bounds = Some(vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t).bounds);
    let text = old.insert(Some(l), 0, Node::new(NodeId(0), NodeKind::Text(Box::new(t)))).unwrap();
    let mut new = old.clone();
    new.node_mut(text).unwrap().transform(Affine::translate((9.0, 7.0)), false);
    redraw(&old, &new);
}

#[test]
fn an_unchanged_document_changes_nothing_and_a_new_symbol_everything() {
    let (old, _) = document();
    assert_eq!(redraw(&old, &old.clone()), Changed::Nothing);
    let mut new = old.clone();
    let art = Arc::new(rect(&mut new, Rect::new(0.0, 0.0, 5.0, 5.0), Color::BLACK));
    new.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Dot".into(), art });
    assert_eq!(redraw(&old, &new), Changed::Everything);
}

/// While a pattern is edited its art also draws in the copies round it: an edit of it changes
/// everything.
#[test]
fn an_edit_while_a_pattern_is_edited_changes_everything() {
    let (mut old, [loose, ..]) = document();
    let l = old.layers[0].id;
    old.pattern_edit = Some(vectorcraft_doc::PatternEdit { pattern: "Tile".into(), layer: l, original: None });
    let mut new = old.clone();
    new.node_mut(loose).unwrap().transform(Affine::translate((7.0, 3.0)), false);
    assert_eq!(changed(&old, &new), Changed::Everything);
}

/// Type stretched more on one axis than its area grows: its characters' strokes stretch with it.
#[test]
fn moving_stretched_type_changes_where_its_stretched_strokes_reach() {
    let mut old = Document::new(110.0, 80.0);
    let l = old.layers[0].id;
    let style = vectorcraft_doc::text::CharStyle {
        size: 4.0,
        stroke: Paint::solid(Color::BLACK),
        stroke_width: 1.0,
        stroke_join: vectorcraft_doc::LineJoin::Miter,
        stroke_miter_limit: 10.0,
        ..Default::default()
    };
    let mut t = vectorcraft_doc::text::TextObject::point(Point::ZERO, "VAWV", style);
    t.cached_bounds = Some(vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t).bounds);
    // Its area stays, so its scale factor is 1, while it grows ten times as wide.
    t.xf = Affine::translate((10.0, 40.0)) * Affine::scale_non_uniform(10.0, 0.1);
    let text = old.insert(Some(l), 0, Node::new(NodeId(0), NodeKind::Text(Box::new(t)))).unwrap();
    let mut new = old.clone();
    new.node_mut(text).unwrap().transform(Affine::translate((9.0, 7.0)), false);
    area(redraw(&old, &new));
}

/// `n` anchors round an ellipse at (55, 40) with radii `rx` and `ry` and a ripple, every third a
/// corner and the others smooth with handles along the ellipse.
fn wavy(n: usize, rx: f64, ry: f64) -> SubPath {
    let anchors = (0..n)
        .map(|i| {
            let t = i as f64 / n as f64 * std::f64::consts::TAU;
            let k = 1.0 + 0.06 * (t * 12.0).sin();
            let p = Point::new(55.0 + rx * k * t.cos(), 40.0 + ry * k * t.sin());
            if i % 3 == 0 {
                return Anchor::corner(p);
            }
            let along = Vec2::new(-rx * t.sin(), ry * t.cos()) * (0.4 / n as f64);
            Anchor { h_in: p - along, h_out: p + along, ..Anchor::corner(p) }
        })
        .collect();
    SubPath::new(anchors, true)
}

/// The seven-pointed star {7/3} round (55, 40), each edge split by 39 anchors on it: its edges
/// cross, so parts of it are covered more than once.
fn star() -> SubPath {
    let tip = |i: usize| {
        let t = (i * 3 % 7) as f64 / 7.0 * std::f64::consts::TAU;
        Point::new(55.0 + 45.0 * t.sin(), 40.0 - 35.0 * t.cos())
    };
    let anchors = (0..7).flat_map(|i| (0..40).map(move |j| Anchor::corner(tip(i).lerp(tip(i + 1), j as f64 / 40.0)))).collect();
    SubPath::new(anchors, true)
}

/// A document with path `sub` filled with `rule`, stroked `stroke` points wide when that is more
/// than 0, and its id.
fn long_path(sub: SubPath, rule: FillRule, stroke: f64) -> (Document, NodeId) {
    let mut d = Document::new(110.0, 80.0);
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let line = if stroke > 0.0 { Paint::solid(Color::BLACK) } else { Paint::None };
    let mut n = Node::path(id, PathData::single(sub), Appearance::basic(Paint::solid(Color::rgb(0.1, 0.5, 0.8)), line, stroke));
    if let NodeKind::Path { rule: r, .. } = &mut n.kind {
        *r = rule;
    }
    let id = d.insert(Some(l), 0, n).unwrap();
    (d, id)
}

/// `d` with anchor `i` of the first subpath of path `id` and its handles moved by `by`.
fn move_anchor(d: &Document, id: NodeId, i: usize, by: Vec2) -> Document {
    let mut d = d.clone();
    if let Some(NodeKind::Path { path, .. }) = d.node_mut(id).map(|n| &mut n.kind) {
        let a = &mut path.subpaths[0].anchors[i];
        (a.p, a.h_in, a.h_out) = (a.p + by, a.h_in + by, a.h_out + by);
    }
    d
}

/// Moving one anchor of a long filled path changes the fill only round the segments at that
/// anchor, with either fill rule, a smooth or a corner anchor, and a closing segment.
#[test]
fn moving_an_anchor_of_a_long_path_changes_only_round_its_segments() {
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        let (old, id) = long_path(wavy(2000, 45.0, 32.0), rule, 0.0);
        for i in [0, 700, 1001] {
            let a = area(redraw(&old, &move_anchor(&old, id, i, Vec2::new(4.0, -3.0))));
            assert!(a.width() < 8.0 && a.height() < 8.0, "{rule:?}, anchor {i}: {a:?}");
        }
    }
}

/// The fill of an open path closes it with a line from its last anchor to its first: moving an
/// end changes where that line was and is too.
#[test]
fn moving_an_end_of_an_open_filled_path_changes_the_line_that_closes_its_fill() {
    let mut half = wavy(3000, 45.0, 32.0);
    half.anchors.truncate(1500);
    half.closed = false;
    let (old, id) = long_path(half, FillRule::NonZero, 0.0);
    for i in [0, 1499] {
        redraw(&old, &move_anchor(&old, id, i, Vec2::new(0.0, 12.0)));
    }
}

/// Where a path covers its middle twice, the two fill rules draw it differently: an anchor moved
/// across that part changes only round its segments.
#[test]
fn moving_an_anchor_where_a_path_crosses_itself_changes_only_round_its_segments() {
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        let (old, id) = long_path(star(), rule, 0.0);
        let a = area(redraw(&old, &move_anchor(&old, id, 20, Vec2::new(-6.0, 5.0))));
        assert!(a.width() < 12.0 && a.height() < 12.0, "{rule:?}: {a:?}");
    }
}

/// A stroke with miter joints changes round the moved anchor's segments and as far as its spikes
/// reach from them, and the open end of a stroke with square caps as far as its cap's corners.
#[test]
fn moving_an_anchor_of_a_stroked_path_changes_as_far_as_its_joints_reach() {
    let (old, id) = long_path(star(), FillRule::NonZero, 1.5);
    let a = area(redraw(&old, &move_anchor(&old, id, 60, Vec2::new(3.0, 4.0))));
    assert!(a.width() < 30.0, "{a:?}");
    let mut open = star();
    open.closed = false;
    let (old, id) = long_path(open, FillRule::NonZero, 3.0);
    let mut sq = old.clone();
    if let Some(AppearanceItem::Stroke(s)) = sq.node_mut(id).unwrap().appearance.items.get_mut(1) {
        s.cap = vectorcraft_doc::LineCap::Square;
    }
    let a = area(redraw(&sq, &move_anchor(&sq, id, 279, Vec2::new(5.0, 2.0))));
    assert!(a.width() < 50.0, "{a:?}");
}

/// A compound path with a hole changes only round the moved anchor of the member it moved in.
#[test]
fn moving_an_anchor_of_a_compound_path_changes_only_round_its_segments() {
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        let mut old = Document::new(110.0, 80.0);
        let l = old.layers[0].id;
        let (outer, inner) = (old.alloc_id(), old.alloc_id());
        let mut hole = wavy(600, 25.0, 15.0);
        if rule == FillRule::NonZero {
            // Wound the other way, so it is a hole by either rule.
            hole.anchors.reverse();
            for a in &mut hole.anchors {
                std::mem::swap(&mut a.h_in, &mut a.h_out);
            }
        }
        let members = vec![
            Arc::new(Node::path(outer, PathData::single(wavy(1500, 45.0, 32.0)), Appearance::default())),
            Arc::new(Node::path(inner, PathData::single(hole), Appearance::default())),
        ];
        let mut compound = Node::new(old.alloc_id(), NodeKind::Compound { children: members, rule });
        compound.appearance = Appearance::basic(Paint::solid(Color::rgb(0.8, 0.3, 0.1)), Paint::None, 0.0).into();
        old.insert(Some(l), 0, compound).unwrap();
        let new = move_anchor(&old, inner, 150, Vec2::new(3.0, 3.0));
        let a = area(redraw(&old, &new));
        assert!(a.width() < 8.0 && a.height() < 8.0, "{rule:?}: {a:?}");
    }
}

/// A gradient fill and a dashed stroke are laid out by the whole path: moving one anchor changes
/// all of it, as does adding an anchor.
#[test]
fn a_path_painted_by_its_whole_shape_changes_whole_when_an_anchor_moves() {
    let (plain, id) = long_path(wavy(400, 45.0, 32.0), FillRule::NonZero, 1.0);
    let mut graded = plain.clone();
    let stops = vec![vectorcraft_color::GradientStop::new(0.0, Color::BLACK), vectorcraft_color::GradientStop::new(1.0, Color::WHITE)];
    graded.node_mut(id).unwrap().appearance.fill_mut().unwrap().paint = Paint::Gradient(Box::new(vectorcraft_color::GradientPaint::new(
        vectorcraft_color::Gradient::new(vectorcraft_color::GradientKind::Linear, stops),
    )));
    let mut dashed = plain.clone();
    if let Some(AppearanceItem::Stroke(s)) = dashed.node_mut(id).unwrap().appearance.items.get_mut(1) {
        s.dash = Some(vectorcraft_doc::Dash { pattern: vec![3.0, 2.0], ..Default::default() });
    }
    for old in [graded, dashed] {
        let a = area(redraw(&old, &move_anchor(&old, id, 100, Vec2::new(2.0, 2.0))));
        assert!(a.width() > 80.0 && a.height() > 60.0, "{a:?}");
    }
    let mut added = plain.clone();
    if let Some(NodeKind::Path { path, .. }) = added.node_mut(id).map(|n| &mut n.kind) {
        path.subpaths[0].anchors.insert(10, Anchor::corner(Point::new(50.0, 40.0)));
    }
    let a = area(redraw(&plain, &added));
    assert!(a.width() > 80.0 && a.height() > 60.0, "{a:?}");
}

/// Changing a 3D effect's parameters on `make`'s object, from `before` to `after`.
fn three_d_change(make: impl Fn(&mut Document) -> Node, id: &str, before: serde_json::Value, after: serde_json::Value) -> Changed {
    let effect = |params: serde_json::Value| vectorcraft_doc::Effect { id: id.into(), params, visible: true };
    let mut old = Document::new(110.0, 80.0);
    let l = old.layers[0].id;
    let mut n = make(&mut old);
    n.appearance.effects.push(effect(before));
    let n = old.insert(Some(l), 0, n).unwrap();
    let mut new = old.clone();
    new.node_mut(n).unwrap().appearance.effects[0] = effect(after);
    redraw(&old, &new)
}

/// Type and a group draw a 3D effect as solids that reach past their letters and members: a
/// shallower or turned solid changes where the deeper one drew too.
#[test]
fn a_3d_effect_changed_on_type_or_a_group_changes_where_its_solid_was() {
    use vectorcraft_doc::{CharStyle, TextObject};
    let blue = Color::rgb(0.1, 0.2, 0.6);
    let type_k = move |d: &mut Document| {
        let style = CharStyle { size: 30.0, fill: Paint::solid(blue), ..Default::default() };
        Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(40.0, 50.0), "K", style))))
    };
    let group = move |d: &mut Document| {
        let (a, b) = (rect(d, Rect::new(40.0, 30.0, 50.0, 50.0), blue), rect(d, Rect::new(52.0, 30.0, 60.0, 50.0), blue));
        Node::group(d.alloc_id(), vec![Arc::new(a), Arc::new(b)])
    };
    let at = |depth: f64| serde_json::json!({"depth": depth, "position": "isometricTop"});
    for make in [&type_k as &dyn Fn(&mut Document) -> Node, &group] {
        for (id, before, after) in [
            ("threeD.extrude", at(60.0), at(5.0)),
            ("threeD.inflate", at(40.0), at(2.0)),
            ("threeD.rotate", serde_json::json!({"rotationX": 0.0}), serde_json::json!({"rotationX": 80.0})),
        ] {
            // `redraw` holds the pixels drawn again only where it changed to drawing it all, and
            // only an area of the document is drawn again.
            area(three_d_change(make, id, before, after));
        }
    }
    // Type on a path, a group with a hidden member, and a group in a group.
    let on_path = move |d: &mut Document| {
        let style = CharStyle { size: 14.0, fill: Paint::solid(blue), ..Default::default() };
        let mut t = TextObject::point(Point::ZERO, "Along", style);
        let line = kurbo::Line::new((30.0, 55.0), (80.0, 35.0)).to_path(0.1);
        t.kind = vectorcraft_doc::TextKind::OnPath { path: vectorcraft_geom::PathData::from_bezpath(&line), start: 0.0, end: None };
        t.xf = Affine::IDENTITY;
        t.cached_bounds = Some(vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t).bounds);
        Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)))
    };
    let hiding = move |d: &mut Document| {
        let mut hidden = rect(d, Rect::new(10.0, 10.0, 20.0, 20.0), blue);
        hidden.visible = false;
        let shown = rect(d, Rect::new(40.0, 30.0, 60.0, 50.0), blue);
        Node::group(d.alloc_id(), vec![Arc::new(hidden), Arc::new(shown)])
    };
    let nested = move |d: &mut Document| {
        let inner = group(d);
        Node::group(d.alloc_id(), vec![Arc::new(inner)])
    };
    for make in [&on_path as &dyn Fn(&mut Document) -> Node, &hiding, &nested] {
        area(three_d_change(make, "threeD.extrude", at(60.0), at(5.0)));
    }
    // A revolved path, and revolved type, which Revolve leaves as it is.
    let path = move |d: &mut Document| rect(d, Rect::new(40.0, 30.0, 60.0, 50.0), blue);
    for make in [&path as &dyn Fn(&mut Document) -> Node, &type_k] {
        area(three_d_change(make, "threeD.revolve", serde_json::json!({"offset": 30.0}), serde_json::json!({"offset": 0.0})));
    }
}

/// A renderer that drew a document measures what an edit changed through what it keeps
/// ([`Renderer::changed`]): the same area as [`changed`], and the 3D art it made to measure the
/// edit it draws the edit with, making none again. [`changed`] makes its own, and the renderer
/// makes the art again to draw it.
#[test]
fn a_renderer_measures_an_edit_with_the_3d_art_it_draws() {
    use vectorcraft_doc::{CharStyle, TextObject};
    let mut old = Document::new(110.0, 80.0);
    let style = CharStyle { size: 30.0, fill: Paint::solid(Color::rgb(0.1, 0.2, 0.6)), ..Default::default() };
    let mut n = Node::new(old.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(40.0, 50.0), "K", style))));
    let extrude = |depth: f64| vectorcraft_doc::Effect {
        id: "threeD.extrude".into(),
        params: serde_json::json!({"depth": depth, "position": "isometricTop"}),
        visible: true,
    };
    n.appearance.effects.push(extrude(60.0));
    let l = old.layers[0].id;
    let id = old.insert(Some(l), 0, n).unwrap();
    let mut new = old.clone();
    new.node_mut(id).unwrap().appearance.effects[0] = extrude(5.0);
    let opts = RenderOptions::default();
    let _fonts = crate::tests_fontchange::fonts_steady();
    let (measured, built, apart) = crate::tests_fontchange::with_fonts_unchanged(|| {
        let mut r = Renderer::new();
        r.render(&old, W, H, view(), &opts);
        let measured = area(r.changed(&old, &new, &opts));
        r.render(&new, W, H, view(), &opts);
        let mut fresh = Renderer::new();
        fresh.render(&old, W, H, view(), &opts);
        let _ = changed(&old, &new);
        fresh.render(&new, W, H, view(), &opts);
        (measured, r.stats.three_d_built, fresh.stats.three_d_built)
    });
    assert_eq!(measured, area(changed(&old, &new)), "as changed measures it");
    assert_eq!(built, 0, "drawn with the art made to measure it");
    assert_eq!(apart, 1, "changed keeps nothing for the renderer");
}

/// An Image Trace object shown with its source image (`view`): its hidden source, a red image at
/// the left, and two traced shapes, at the right; the object's id, and the ids of the source and
/// of the second shape.
fn traced(view: &str) -> (Document, NodeId, NodeId, NodeId) {
    let mut d = Document::new(W as f64, H as f64);
    let mut png = vec![];
    let red = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255]));
    red.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    d.images.insert("red".into(), vectorcraft_doc::ImageBlob::new("image/png", png));
    let source = red_image(&mut d, 10.0);
    let a = rect(&mut d, Rect::new(60.0, 10.0, 70.0, 20.0), Color::rgb(0.0, 0.5, 0.0));
    let b = rect(&mut d, Rect::new(80.0, 10.0, 90.0, 20.0), Color::rgb(1.0, 0.5, 0.0));
    let ids = (source.id, b.id);
    let mut g = Node::group(d.alloc_id(), vec![Arc::new(source), Arc::new(a), Arc::new(b)]);
    g.extras_mut().trace = Some(Box::new(serde_json::json!({ "view": view })));
    let l = d.layers[0].id;
    let g = d.insert(Some(l), 0, g).unwrap();
    (d, g, ids.0, ids.1)
}

/// A hidden 30 pt square of the red image at (`x`, 10), as an Image Trace object's source.
fn red_image(d: &mut Document, x: f64) -> Node {
    let xf = Affine::translate((x, 10.0)) * Affine::scale(30.0);
    let im = vectorcraft_doc::ImageObject { key: "red".into(), width: 1, height: 1, xf, link: None, placement: Default::default(), pixels: None };
    let mut source = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
    source.visible = false;
    source
}

/// Draws `new` again only where the renderer that drew `old` on screen, its Image Trace objects in
/// their views, measures that it changed, and checks that this is `new` drawn whole.
fn redraw_traced(old: &Document, new: &Document) -> Changed {
    let opts = RenderOptions { trace_views: true, ..Default::default() };
    let after = Renderer::new().render(new, W, H, view(), &opts);
    let mut r = Renderer::new();
    let before = r.render(old, W, H, view(), &opts);
    let red = |r: &Rendered| r.pixels.chunks(4).any(|p| p[0] > 200 && p[1] < 60);
    assert!(red(&before) || red(&after), "a source shows");
    let measured = r.changed(old, new, &opts);
    patched(&mut r, &before, new, measured, &after, &opts);
    measured
}

#[test]
fn moving_a_trace_objects_source_changes_where_it_shows() {
    for view in ["sourceImage", "outlinesWithSourceImage"] {
        let (old, _, source, _) = traced(view);
        let mut new = old.clone();
        new.node_mut(source).unwrap().transform(Affine::translate((5.0, 30.0)), false);
        let a = area(redraw_traced(&old, &new));
        assert!(a.contains(Point::new(12.0, 12.0)) && a.contains(Point::new(44.0, 69.0)), "{view}: before and after: {a:?}");
    }
}

/// A trace object's source that is first again after an edit (undoing Send to Back on a traced
/// shape), or a new source in its place, draws where it shows.
#[test]
fn a_trace_objects_new_first_member_is_its_source() {
    let (back, g, source, shape) = traced("sourceImage");
    // The shape sent to the back, before the source.
    let mut sent = back.clone();
    let children = sent.node_mut(g).unwrap().children_mut().unwrap();
    let at = children.iter().position(|c| c.id == shape).unwrap();
    let moved = children.remove(at);
    children.insert(0, moved);
    redraw_traced(&sent, &back);
    // A new source image, with an id of its own.
    let mut replaced = back.clone();
    let new_source = red_image(&mut replaced, 55.0);
    let children = replaced.node_mut(g).unwrap().children_mut().unwrap();
    assert_eq!(children[0].id, source);
    children[0] = Arc::new(new_source);
    redraw_traced(&back, &replaced);
}

#[test]
fn recolouring_a_traced_shape_changes_only_the_shape() {
    let (old, _, _, shape) = traced("outlinesWithSourceImage");
    let mut new = old.clone();
    new.node_mut(shape).unwrap().appearance.fill_mut().unwrap().paint = Paint::solid(Color::rgb(0.0, 0.0, 1.0));
    // The shape, half its 2 pt stroke and a pixel's anti-aliasing, at most: measured by the
    // renderer that drew the trace object in its view, and by the free measure.
    let opts = RenderOptions { trace_views: true, ..Default::default() };
    let mut r = Renderer::new();
    r.render(&old, W, H, view(), &opts);
    let measured = area(r.changed(&old, &new, &opts));
    let shape = Rect::new(80.0, 10.0, 90.0, 20.0).inflate(2.0, 2.0);
    for (what, a) in [("measured", measured), ("free", area(redraw(&old, &new)))] {
        assert!(a.union(shape) == shape, "{what}: only the shape: {a:?}");
    }
}

/// Templates and guides that the view hides draw nothing: an edit to them changes nothing.
#[test]
fn an_edit_to_what_the_view_hides_changes_nothing() {
    let mut old = Document::new(W as f64, H as f64);
    let template = Node::layer(old.alloc_id(), "Template", vectorcraft_doc::LayerColor::Preset(1));
    let template = old.insert(None, usize::MAX, template).unwrap();
    if let Some(NodeKind::Layer { template: t, .. }) = old.node_mut(template).map(|n| &mut n.kind) {
        *t = true;
    }
    let traced = rect(&mut old, Rect::new(10.0, 10.0, 40.0, 40.0), Color::rgb(0.5, 0.5, 0.5));
    let traced = old.insert(Some(template), 0, traced).unwrap();
    let mut guide = rect(&mut old, Rect::new(60.0, 10.0, 90.0, 40.0), Color::BLACK);
    if let NodeKind::Path { guide: g, .. } = &mut guide.kind {
        *g = true;
    }
    let l = old.layers[0].id;
    let guide = old.insert(Some(l), 0, guide).unwrap();
    let mut new = old.clone();
    for id in [traced, guide] {
        new.node_mut(id).unwrap().transform(Affine::translate((5.0, 5.0)), false);
    }
    let opts = RenderOptions { hide_templates: true, hide_guides: true, ..Default::default() };
    let mut r = Renderer::new();
    r.render(&old, W, H, view(), &opts);
    assert_eq!(r.changed(&old, &new, &opts), Changed::Nothing);
    assert!(matches!(changed(&old, &new), Changed::Area(_)), "shown, they change");
}

/// Overprint Preview and separations draw objects through the inks they print, not through the
/// art a renderer keeps for them: a renderer measures an edit there holding none of the 3D art it
/// made to measure it, and as [`changed`] does.
#[test]
fn an_edit_measured_under_overprint_holds_no_art() {
    use vectorcraft_doc::{CharStyle, TextObject};
    let mut old = Document::new(110.0, 80.0);
    let style = CharStyle { size: 30.0, fill: Paint::solid(Color::rgb(0.1, 0.2, 0.6)), ..Default::default() };
    let mut n = Node::new(old.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(40.0, 50.0), "K", style))));
    let extrude = |depth: f64| vectorcraft_doc::Effect {
        id: "threeD.extrude".into(),
        params: serde_json::json!({"depth": depth, "position": "isometricTop"}),
        visible: true,
    };
    n.appearance.effects.push(extrude(60.0));
    let l = old.layers[0].id;
    let id = old.insert(Some(l), 0, n).unwrap();
    let mut new = old.clone();
    new.node_mut(id).unwrap().appearance.effects[0] = extrude(5.0);
    let _fonts = crate::tests_fontchange::fonts_steady();
    let held = |opts: &RenderOptions| {
        crate::tests_fontchange::with_fonts_unchanged(|| {
            let mut r = Renderer::new();
            r.render(&old, W, H, view(), opts);
            (area(r.changed(&old, &new, opts)), r.measured_bytes())
        })
    };
    let (_, on_screen) = held(&RenderOptions::default());
    assert!(on_screen > 0, "the screen view holds the art it measured with");
    let (measured, overprinted) = held(&RenderOptions { overprint_preview: true, ..Default::default() });
    assert_eq!(overprinted, 0);
    // A consistency check: the area itself doesn't tell the branch taken.
    assert_eq!(measured, area(changed(&old, &new)), "as changed measures it");
}

/// Symbol editing mode on screen doesn't draw the instance it was entered from: an edit to that
/// instance changes nothing there, and changes where it draws in an export.
#[test]
fn an_edit_to_the_instance_symbol_editing_came_from_changes_nothing_on_screen() {
    use vectorcraft_doc::{Symbol, SymbolEdit};
    let mut old = Document::new(W as f64, H as f64);
    let art = rect(&mut old, Rect::new(20.0, 20.0, 50.0, 50.0), Color::rgb(0.0, 0.5, 0.0));
    old.symbols.push(Symbol { dynamic: false, name: "Square".into(), art: Arc::new(art.clone()) });
    let inst = Node::new(old.alloc_id(), NodeKind::SymbolInstance { symbol: "Square".into(), xf: Affine::IDENTITY });
    let l = old.layers[0].id;
    let inst = old.insert(Some(l), 0, inst).unwrap();
    let layer = old.add_layer(Some("Square"));
    let copy = old.reid(&art);
    old.insert(Some(layer), usize::MAX, copy).unwrap();
    old.symbol_edit = Some(SymbolEdit {
        symbol: "Square".into(),
        layer,
        instance: Some(inst),
        xf: Affine::IDENTITY,
        group: false,
        changed: false,
        back_isolation: None,
        back_layer: None,
    });
    let mut new = old.clone();
    new.node_mut(inst).unwrap().transform(Affine::translate((30.0, 30.0)), false);
    let screen = RenderOptions { isolated: Some(layer), ..Default::default() };
    let mut r = Renderer::new();
    r.render(&old, W, H, view(), &screen);
    assert_eq!(r.changed(&old, &new, &screen), Changed::Nothing);
    let export = RenderOptions::default();
    let mut r = Renderer::new();
    r.render(&old, W, H, view(), &export);
    assert!(matches!(r.changed(&old, &new, &export), Changed::Area(_)), "an export draws it");
}
