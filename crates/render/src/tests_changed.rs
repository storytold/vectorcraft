//! Drawing again only what an edit changed gives the pixels of drawing it all again.

use super::*;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::Appearance;
use vectorcraft_geom::{Point, shapes};

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
/// this is `new` drawn whole; returns what changed.
fn redraw(old: &Document, new: &Document) -> Changed {
    let mut r = Renderer::new();
    let opts = RenderOptions::default();
    let before = r.render(old, W, H, view(), &opts);
    let after = r.render(new, W, H, view(), &opts);
    let change = changed(old, new);
    let mut patched = Rendered { width: W, height: H, pixels: before.pixels.clone() };
    match change {
        Changed::Everything => patched.pixels = after.pixels.clone(),
        Changed::Nothing => {}
        Changed::Area(area) => {
            if let Some([x0, y0, x1, y1]) = view_pixels(area, view(), W, H) {
                let part = r.render(new, x1 - x0, y1 - y0, Affine::translate((-f64::from(x0), -f64::from(y0))) * view(), &opts);
                for y in 0..y1 - y0 {
                    let (from, to) = ((y * (x1 - x0) * 4) as usize, (((y0 + y) * W + x0) * 4) as usize);
                    let n = ((x1 - x0) * 4) as usize;
                    patched.pixels[to..to + n].copy_from_slice(&part.pixels[from..from + n]);
                }
            }
        }
    }
    assert!(patched.pixels == after.pixels, "{change:?}: drawn again only there, it differs from drawing it all");
    change
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
    new.symbols.push(vectorcraft_doc::Symbol { name: "Dot".into(), art });
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
