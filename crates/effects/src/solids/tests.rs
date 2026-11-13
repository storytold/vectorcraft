use serde_json::json;
use vectorcraft_doc::{Document, NodeId, StrokeLayer};
use vectorcraft_geom::{Affine, BezPath, Point, Shape};

use super::*;

const RED: Color = Color::Rgb { r: 0.8, g: 0.2, b: 0.1 };

fn square_node() -> Node {
    let mut b = BezPath::new();
    b.move_to((100.0, 100.0));
    b.line_to((200.0, 100.0));
    b.line_to((200.0, 200.0));
    b.line_to((100.0, 200.0));
    b.close_path();
    Node::path(NodeId(1), PathData::from_bezpath(&b), Appearance::basic(Paint::solid(RED), Paint::solid(Color::BLACK), 4.0))
}

fn fx(id: &str, params: Value) -> Effect {
    crate::new_effect(id, &params).unwrap()
}

/// The shaded faces of every surface of `art`, bottom first.
fn faces(art: &Node) -> Vec<&Node> {
    art.children().unwrap().iter().flat_map(|s| s.children().unwrap().iter().map(|f| &**f)).collect()
}

fn fill_rgb(n: &Node) -> [f32; 3] {
    match &n.appearance.items[0] {
        AppearanceItem::Fill(f) => match &f.paint {
            Paint::Gradient(g) => g.gradient.sample(0.5).0.to_rgb(),
            p => p.color().unwrap().to_rgb(),
        },
        _ => panic!("a fill first"),
    }
}

#[test]
fn every_solid_draws_shaded_faces_and_leaves_the_source_editable() {
    for id in SOLIDS {
        let mut n = square_node();
        let source = n.path_data().unwrap().clone();
        n.appearance.effects.push(fx(id, json!({})));
        let art = solid_art(&n).unwrap_or_else(|| panic!("{id}"));
        assert_eq!(n.path_data(), Some(&source), "{id}");
        assert!(!faces(&art).is_empty(), "{id}");
        // The faces are the fill's red or the stroke's black, shaded: never brighter, never another
        // hue. The classic Plastic surface, its default, also lightens a face toward white, which
        // scales every channel's distance from white by the same factor and so keeps r - g to
        // r - b at 6 to 7.
        for f in faces(&art) {
            let [r, g, b] = fill_rgb(f);
            let shaded = r <= 0.8 + 1e-6 && (g * 4.0 - r).abs() < 1e-4 && (b * 8.0 - r).abs() < 1e-4;
            let highlighted =
                id == CLASSIC_EXTRUDE && [r, g, b].iter().all(|c| (0.0..=1.0).contains(c)) && ((r - g) * 7.0 - (r - b) * 6.0).abs() < 1e-4;
            assert!(shaded || highlighted || id == ROTATE, "{id}: {r} {g} {b}");
        }
        // The art follows the source when it moves.
        let before = art.visual_bounds().unwrap();
        n.transform(Affine::translate((50.0, -30.0)), false);
        let after = solid_art(&n).unwrap().visual_bounds().unwrap();
        assert!((after.x0 - before.x0 - 50.0).abs() < 1e-6 && (after.y0 - before.y0 + 30.0).abs() < 1e-6, "{id}");
        // Hidden, the effect draws nothing of its own.
        n.appearance.effects[0].visible = false;
        assert!(solid_art(&n).is_none(), "{id}");
    }
}

#[test]
fn rotate_turns_every_fill_and_stroke_and_the_solids_use_the_top_fill() {
    let mut n = square_node();
    n.appearance.effects.push(fx(ROTATE, json!({})));
    let art = solid_art(&n).unwrap();
    // The fill's plane and the stroke's outline, each in its own paint.
    let surfaces = art.children().unwrap();
    assert_eq!(surfaces.len(), 2);
    let paint = |s: &Node| fill_rgb(&s.children().unwrap()[0]);
    assert!(paint(&surfaces[0])[0] > paint(&surfaces[0])[1], "the red fill below");
    assert!(paint(&surfaces[1]).iter().all(|c| *c < 0.05), "the black stroke above");
    // Turned 60° about y with no perspective, the stroke's outline spans half its width plus the
    // stroke's own outset, foreshortened as well.
    n.appearance.effects[0].params = json!({"rotationX": 0, "rotationY": 60, "rotationZ": 0});
    let b = solid_art(&n).unwrap().children().unwrap()[1].visual_bounds().unwrap();
    assert!((b.width() - 104.0 * 0.5 - 0.75).abs() < 1e-6, "{b:?}");
    // Extrude & Bevel draws the fill's solid and the stroke's, both opaque, as one surface.
    let mut n = square_node();
    n.appearance.effects.push(fx(EXTRUDE, json!({})));
    assert_eq!(solid_art(&n).unwrap().children().unwrap().len(), 1);
}

const BLUE: Color = Color::Rgb { r: 0.1, g: 0.3, b: 0.9 };

/// `b` with a red fill and a 6 pt blue stroke above it, on the path.
fn stroked(b: &BezPath) -> Node {
    Node::path(NodeId(1), PathData::from_bezpath(b), Appearance::basic(Paint::solid(RED), Paint::solid(BLUE), 6.0))
}

/// A `width` × 100 pt rectangle at (100, 100), stroked.
fn stroked_rect(width: f64) -> Node {
    let mut b = BezPath::new();
    b.move_to((100.0, 100.0));
    b.line_to((100.0 + width, 100.0));
    b.line_to((100.0 + width, 200.0));
    b.line_to((100.0, 200.0));
    b.close_path();
    stroked(&b)
}

/// Is `face` in the stroke's blue (rather than the fill's red)?
fn blue(face: &Node) -> bool {
    let [r, _, b] = fill_rgb(face);
    b > r
}

/// The bounds of the faces' own paths (without their seam strokes).
fn extent(faces: &[&Node]) -> Rect {
    faces.iter().filter_map(|f| f.path_data()?.bounds()).reduce(|a, b| a.union(b)).unwrap()
}

fn assert_rect(r: Rect, x0: f64, y0: f64, x1: f64, y1: f64) {
    assert!([r.x0 - x0, r.y0 - y0, r.x1 - x1, r.y1 - y1].iter().all(|d| d.abs() < 1e-6), "{r:?}");
}

/// Extrude & Bevel and Inflate build a solid from each stroke's outline as well as from the path
/// for the fill: seen straight on, the stroke's faces reach 3 pt (half its width) outside the
/// fill's, and they are drawn in the stroke's paint.
#[test]
fn a_stroke_builds_a_solid_from_its_outline_beside_the_fill() {
    for id in [EXTRUDE, INFLATE] {
        let mut n = stroked_rect(100.0);
        n.appearance.effects.push(fx(id, json!({"rotationX": 0, "rotationY": 0, "rotationZ": 0})));
        let art = solid_art(&n).unwrap();
        let (strokes, fills): (Vec<&Node>, Vec<&Node>) = faces(&art).into_iter().partition(|f| blue(f));
        assert!(!strokes.is_empty(), "{id}: the stroke draws");
        assert_rect(extent(&fills), 100.0, 100.0, 200.0, 200.0);
        assert_rect(extent(&strokes), 97.0, 97.0, 203.0, 203.0);
        // Its own blue, shaded: never brighter, never another hue.
        for f in &strokes {
            let [r, g, b] = fill_rgb(f);
            assert!(b <= 0.9 + 1e-6 && (r * 9.0 - b).abs() < 1e-4 && (g * 3.0 - b).abs() < 1e-4, "{id}: {r} {g} {b}");
        }
    }
}

/// Turned, the extrusion's walls show in both paints, and the front caps, which nothing of either
/// solid can hide, are drawn last: the fill's, then the stroke's above it, equally lit. Without
/// depth the two flat shapes are drawn in the same order. A stroke below the fill has its cap
/// drawn below the fill's.
#[test]
fn the_front_caps_are_drawn_last_in_the_order_of_the_appearance() {
    // A triangle wider than deep, so the near ends of the walls lie nearer than the middle of the
    // caps; the mean of its outline's points is not the mean of the stroke's.
    let mut b = BezPath::new();
    b.move_to((100.0, 100.0));
    b.line_to((400.0, 100.0));
    b.line_to((100.0, 200.0));
    b.close_path();
    for params in [json!({}), json!({"depth": 120, "bevel": "rounded"}), json!({"depth": 0})] {
        let mut n = stroked(&b);
        n.appearance.effects.push(fx(EXTRUDE, params.clone()));
        n.appearance.items.reverse();
        let art = solid_art(&n).unwrap();
        let [.., stroke_cap, fill_cap] = faces(&art)[..] else { panic!("{params}: two caps") };
        assert!(blue(stroke_cap) && !blue(fill_cap), "{params}: the stroke below");
        n.appearance.items.reverse();
        let art = solid_art(&n).unwrap();
        let faces = faces(&art);
        let flat = params["depth"] == json!(0);
        assert!(if flat { faces.len() == 2 } else { faces.len() > 4 }, "{params}: {} faces", faces.len());
        let [.., fill_cap, stroke_cap] = faces.as_slice() else { panic!("{params}: two caps") };
        assert!(!blue(fill_cap) && blue(stroke_cap), "{params}");
        // The stroke's cap is a ring: its outline and the hole the fill shows through.
        assert_eq!(stroke_cap.path_data().unwrap().subpaths.len(), 2, "{params}");
        assert_eq!(fill_cap.path_data().unwrap().subpaths.len(), 1, "{params}");
        let shade = |f: &Node, i: usize, of: f32| fill_rgb(f)[i] / of;
        assert!((shade(fill_cap, 0, 0.8) - shade(stroke_cap, 2, 0.9)).abs() < 1e-5, "{params}: one plane, one light");
        if !flat {
            assert!(faces.iter().filter(|f| blue(f)).count() > 2 && faces.iter().filter(|f| !blue(f)).count() > 1, "{params}: walls of both");
        }
    }
}

/// A C, open to the right: two concave corners where its walls meet the walls of the notch.
fn letter_c() -> BezPath {
    let mut b = BezPath::new();
    b.move_to((100.0, 100.0));
    b.line_to((200.0, 100.0));
    b.line_to((200.0, 130.0));
    b.line_to((130.0, 130.0));
    b.line_to((130.0, 170.0));
    b.line_to((200.0, 170.0));
    b.line_to((200.0, 200.0));
    b.line_to((100.0, 200.0));
    b.close_path();
    b
}

/// The pairs (blue, red) of `walls`, by index, where the blue face is drawn first and the two
/// faces' bounds overlap.
fn red_after_blue(walls: &[&Node]) -> Vec<(usize, usize)> {
    let bounds: Vec<Rect> = walls.iter().map(|f| f.path_data().and_then(PathData::bounds).unwrap()).collect();
    let overlap = |a: &Rect, b: &Rect| a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1;
    let mut out = vec![];
    for (i, a) in walls.iter().enumerate() {
        for (j, b) in walls.iter().enumerate().skip(i + 1) {
            if blue(a) && !blue(b) && overlap(&bounds[i], &bounds[j]) {
                out.push((i, j));
            }
        }
    }
    out
}

/// An opaque stroke's solid holds the fill's edge, so the fill's walls are drawn before every
/// face of the stroke they overlap, and the stroke's walls cover them at the concave corners of a
/// turned C, in Extrude & Bevel and Extrude & Bevel (Classic). The front caps stay last, the
/// fill's below the stroke's.
#[test]
fn a_fills_walls_are_drawn_before_the_stroke_faces_over_them() {
    let views = [json!({}), json!({"rotationX": -20, "rotationY": -40, "rotationZ": 0}), json!({"rotationX": 25, "rotationY": -55, "rotationZ": 10})];
    for id in [EXTRUDE, CLASSIC_EXTRUDE] {
        for view in &views {
            let mut params = view.clone();
            params["depth"] = json!(60);
            let mut n = stroked(&letter_c());
            n.appearance.effects.push(fx(id, params));
            let art = solid_art(&n).unwrap();
            let faces = faces(&art);
            let [walls @ .., fill_cap, stroke_cap] = faces.as_slice() else { panic!("{id} {view}: the caps") };
            assert!(!blue(fill_cap) && blue(stroke_cap), "{id} {view}: the caps last");
            assert!(walls.iter().any(|f| !blue(f)) && walls.iter().any(|f| blue(f)), "{id} {view}: walls of both");
            let wrong = red_after_blue(walls);
            assert!(wrong.is_empty(), "{id} {view}: fill walls after the stroke faces over them: {wrong:?} of {}", walls.len());
        }
    }
    // Strokes aligned inside or outside keep the fill's walls, in that order. A dashed stroke
    // leaves the fill's walls in the gaps between its dashes, and they keep their depths.
    use vectorcraft_doc::StrokeAlign;
    for (dash, align) in [(true, StrokeAlign::Center), (false, StrokeAlign::Inside), (false, StrokeAlign::Outside)] {
        let mut n = stroked(&letter_c());
        if let Some(AppearanceItem::Stroke(s)) = n.appearance.items.iter_mut().find(|i| matches!(i, AppearanceItem::Stroke(_))) {
            s.dash = dash.then(|| vectorcraft_doc::Dash { pattern: vec![12.0, 12.0], offset: 0.0, align_corners: false });
            s.align = align;
        }
        n.appearance.effects.push(fx(EXTRUDE, views[1].clone()));
        let art = solid_art(&n).unwrap();
        let faces = faces(&art);
        let [walls @ .., _, _] = faces.as_slice() else { panic!("{align:?}: the caps") };
        assert!(walls.iter().any(|f| !blue(f)), "{align:?}: the fill's walls");
        if !dash {
            assert!(red_after_blue(walls).is_empty(), "{align:?}");
        }
    }
}

/// The faces of the solids effect `id` with `params` builds for `n`, in the order they are drawn,
/// as [`surfaces_for`] orders them, each with the index of the part ([`solid_parts`]) it was built
/// from and whether it is a stroke's.
fn drawn_parts(n: &Node, id: &str, params: &Value) -> Vec<(Facet, usize, bool)> {
    let (path, rule) = profile(n).unwrap();
    let shapes = [Profile { path, rule, items: solid_paints(n), opacity: 1.0, scopes: vec![] }];
    let p = crate::merged_params(id, params);
    let (parts, bounds) = solid_parts(&shapes, union_bounds(&shapes).unwrap());
    let mut solids = vec![];
    for (part, ((item, _, _, _, _, member), f)) in parts.iter().zip(built_solids(id, &p, &parts, bounds, MAX_OBJECT_FACES)).enumerate() {
        let Some(f) = f.unwrap() else { continue };
        let closed = holds_edge(id, &shapes[*member].path);
        solids.push((Paints::of(item, closed), *member, f, part, matches!(item, AppearanceItem::Stroke(_))));
    }
    let mut cuts: Vec<(Paints, usize, &mut Vec<Facet>)> = solids.iter_mut().map(|(paints, member, f, _, _)| (*paints, *member, f)).collect();
    cut_stroke_faces(&mut cuts, MAX_OBJECT_FACES);
    let (mut placed, mut all) = (vec![], vec![]);
    for (paints, member, f, part, stroke) in solids {
        for facet in f {
            placed.push(Placed::of(&facet, member, paints));
            all.push((facet, part, stroke));
        }
    }
    let sealed = match id {
        INFLATE => inflate_options(&p).cap,
        CLASSIC_EXTRUDE => classic_extrude_options(&p).cap,
        _ => extrude_options(&p).cap,
    };
    draw_order(&placed, sealed).into_iter().map(|k| all[k].clone()).collect()
}

/// Is `p` inside the polygon of `f` (nonzero winding)?
fn facet_contains(f: &Facet, p: Point) -> bool {
    let mut winding = 0i32;
    for c in &f.contours {
        for (a, b) in c.iter().zip(c.iter().cycle().skip(1)).take(c.len()) {
            let cross = (b.x - a.x) * (p.y - a.y) - (p.x - a.x) * (b.y - a.y);
            if a.y <= p.y && b.y > p.y && cross > 0.0 {
                winding += 1;
            } else if a.y > p.y && b.y <= p.y && cross < 0.0 {
                winding -= 1;
            }
        }
    }
    winding != 0
}

/// The points of a 1 pt grid over `faces` (drawn in that order, each with whether it is a stroke's)
/// where the face drawn on top is the fill's while a face of the stroke turned toward the viewer is
/// nearer the viewer there by more than 0.5 pt, or the other way round. Points within 0.3 pt of an
/// edge are left out.
fn wrong_on_top(faces: &[(Facet, bool)]) -> Vec<Point> {
    let polygons: Vec<&Facet> = faces.iter().map(|(f, _)| f).collect();
    let extent =
        polygons.iter().flat_map(|f| f.contours.iter().flatten()).fold(Rect::new(f64::MAX, f64::MAX, f64::MIN, f64::MIN), |r, p| r.union_pt(*p));
    let mut wrong = vec![];
    let mut y = extent.y0.floor() + 0.5;
    while y < extent.y1 {
        let mut x = extent.x0.floor() + 0.5;
        while x < extent.x1 {
            let p = Point::new(x, y);
            let well_inside = |f: &Facet| {
                [(-0.3, -0.3), (0.3, -0.3), (-0.3, 0.3), (0.3, 0.3), (0.0, 0.0)].iter().all(|(dx, dy)| facet_contains(f, Point::new(x + dx, y + dy)))
            };
            let top = faces.iter().rposition(|(f, _)| facet_contains(f, p));
            if let Some((f, stroke)) = top.and_then(|k| faces.get(k)).filter(|(f, _)| !f.away && well_inside(f)) {
                let nearest = |of_stroke: bool| {
                    faces.iter().filter(|(g, s)| *s == of_stroke && !g.away && well_inside(g)).filter_map(|(g, _)| g.depth_at(p)).reduce(f64::max)
                };
                if let (Some(ds), Some(df)) = (nearest(true), nearest(false))
                    && (ds - df).abs() > 0.5
                    && (ds > df) != *stroke
                    && f.depth_at(p).is_some()
                {
                    wrong.push(p);
                }
            }
            x += 1.0;
        }
        y += 1.0;
    }
    wrong
}

/// The points of a 1 pt grid over `faces` (drawn in that order) where the face drawn on top is
/// farther from the viewer than another face turned toward the viewer, of any part, by more than
/// 0.5 pt. Points within 0.3 pt of an edge, and faces without a plane, are left out.
fn misdrawn(faces: &[(Facet, usize, bool)]) -> Vec<Point> {
    let extent =
        faces.iter().flat_map(|(f, ..)| f.contours.iter().flatten()).fold(Rect::new(f64::MAX, f64::MAX, f64::MIN, f64::MIN), |r, p| r.union_pt(*p));
    let mut wrong = vec![];
    let mut y = extent.y0.floor() + 0.5;
    while y < extent.y1 {
        let mut x = extent.x0.floor() + 0.5;
        while x < extent.x1 {
            let p = Point::new(x, y);
            let well_inside = |f: &Facet| {
                [(-0.3, -0.3), (0.3, -0.3), (-0.3, 0.3), (0.3, 0.3), (0.0, 0.0)].iter().all(|(dx, dy)| facet_contains(f, Point::new(x + dx, y + dy)))
            };
            let top = faces.iter().rposition(|(f, ..)| facet_contains(f, p)).and_then(|k| faces.get(k)).filter(|(f, ..)| !f.away && well_inside(f));
            if let Some(on_top) = top.and_then(|(f, ..)| f.depth_at(p)) {
                let nearest = faces.iter().filter(|(g, ..)| !g.away && well_inside(g)).filter_map(|(g, ..)| g.depth_at(p)).reduce(f64::max);
                if nearest.is_some_and(|d| d - on_top > 0.5) {
                    wrong.push(p);
                }
            }
            x += 1.0;
        }
        y += 1.0;
    }
    wrong
}

/// The faces of a shape's fill and strokes are drawn in the order of their depths where they
/// overlap: on a turned C, with or without perspective, under a stroke that does not cover the
/// fill's whole edge (dashed, with a width profile, or with a brush), under a plain stroke and
/// under two strokes whose faces pass through each other, no point shows a face farther from the
/// viewer than another, in Extrude & Bevel and Extrude & Bevel (Classic), and only a few in
/// Inflate, where curved puffs cross.
#[test]
fn fill_walls_and_strokes_that_leave_gaps_draw_in_the_order_of_their_depths_where_they_overlap() {
    use vectorcraft_doc::{Dash, WidthProfile};
    let views = [
        json!({"rotationX": -20, "rotationY": -40, "rotationZ": 0}),
        json!({"rotationX": 25, "rotationY": -55, "rotationZ": 10}),
        json!({"rotationX": -30, "rotationY": 50, "rotationZ": 5, "perspectiveAngle": 40}),
        json!({"rotationX": -30, "rotationY": 20, "rotationZ": 5, "perspectiveAngle": 40}),
    ];
    type Style<'a> = (&'a str, &'a dyn Fn(&mut Node));
    let first = |n: &mut Node, set: &dyn Fn(&mut StrokeLayer)| {
        if let Some(AppearanceItem::Stroke(s)) = n.appearance.items.iter_mut().find(|i| matches!(i, AppearanceItem::Stroke(_))) {
            set(s);
        }
    };
    let dash = |pattern: Vec<f64>, offset: f64| Some(Dash { pattern, offset, align_corners: false });
    let styles: [Style; 6] = [
        ("plain", &|_| {}),
        ("dashed", &|n| first(n, &|s| s.dash = dash(vec![12.0, 12.0], 0.0))),
        ("profiled", &|n| first(n, &|s| s.profile = Some(WidthProfile::lens()))),
        ("brushed", &|n| first(n, &|s| s.brush = Some("brush".into()))),
        ("dashed with perspective", &|n| first(n, &|s| s.dash = dash(vec![20.0, 6.0], 3.0))),
        ("two strokes", &|n| {
            first(n, &|s| s.dash = dash(vec![12.0, 12.0], 0.0));
            let mut wide = StrokeLayer::new(Paint::solid(RED), 14.0);
            wide.profile = Some(WidthProfile::lens());
            n.appearance.items.push(AppearanceItem::Stroke(wide));
        }),
    ];
    for id in [EXTRUDE, CLASSIC_EXTRUDE, INFLATE] {
        for (v, view) in views.iter().enumerate() {
            for (style, set) in &styles {
                // The last two views, for the plain and the two strokes only.
                if v >= 2 && !["plain", "two strokes"].contains(style) {
                    continue;
                }
                let mut params = view.clone();
                params["depth"] = json!(60);
                if style.contains("perspective") {
                    params["perspectiveAngle"] = json!(40);
                }
                let mut n = stroked(&letter_c());
                set(&mut n);
                let parts = drawn_parts(&n, id, &params);
                let faces: Vec<(Facet, bool)> = parts.iter().map(|(f, _, s)| (f.clone(), *s)).collect();
                assert!(faces.iter().any(|(_, s)| *s) && faces.iter().any(|(_, s)| !*s), "{id} {view} {style}: faces of both");
                // Inflate's curved puffs cross each other along curves its flat facets only follow
                // roughly: a few points next to those crossings may still show the farther face.
                let allowed = if id == INFLATE { 5 } else { 0 };
                let wrong = wrong_on_top(&faces);
                assert!(wrong.len() <= allowed, "{id} {view} {style}: {} points drawn in the wrong paint, first {:?}", wrong.len(), wrong.first());
                let wrong = misdrawn(&parts);
                assert!(wrong.len() <= allowed, "{id} {view} {style}: {} points show a farther face, first {:?}", wrong.len(), wrong.first());
            }
        }
    }
}

/// The paths of `art`'s faces in drawing order, leaving out the graphics (clip groups).
fn face_paths(art: &Node) -> Vec<PathData> {
    fn walk(n: &Node, out: &mut Vec<PathData>) {
        match &n.kind {
            NodeKind::Group { clip: true, .. } => {}
            NodeKind::Group { children, .. } => children.iter().for_each(|c| walk(c, out)),
            _ => out.extend(n.path_data().cloned()),
        }
    }
    let mut out = vec![];
    walk(art, &mut out);
    out
}

/// A member of a 3D group with an opacity mask or a blending mode of its own draws its fill's and
/// its strokes' faces in one order inside that mask, the order the same member draws them in
/// without it, its dashes cut where they pass through the fill's walls. A member whose stroke is
/// translucent still draws each surface whole.
#[test]
fn a_masked_members_fill_and_stroke_faces_go_in_one_order() {
    let mut member = stroked(&letter_c());
    if let Some(AppearanceItem::Stroke(s)) = member.appearance.items.iter_mut().find(|i| matches!(i, AppearanceItem::Stroke(_))) {
        s.dash = Some(vectorcraft_doc::Dash { pattern: vec![12.0, 12.0], offset: 0.0, align_corners: false });
    }
    let solid = |member: Node| {
        let mut g = Node::group(NodeId(2), vec![Arc::new(member)]);
        g.appearance.effects.push(fx(EXTRUDE, json!({"rotationX": -20, "rotationY": -40, "rotationZ": 0, "depth": 60})));
        solid_art(&g).unwrap()
    };
    let cover = Node::path(
        NodeId(900),
        vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 400.0, 400.0)),
        Appearance::basic(Paint::solid(Color::WHITE), Paint::None, 0.0),
    );
    let mut masked = member.clone();
    masked.extras_mut().mask = Some(Box::new(OpacityMask::new(cover, true)));
    let mut multiplied = member.clone();
    multiplied.blend = BlendMode::Multiply;
    let unmasked = face_paths(&solid(member.clone()));
    for (what, scoped) in [("masked", masked.clone()), ("multiplied", multiplied)] {
        let art = solid(scoped);
        let mut scopes = 0;
        art.walk(&mut |n| scopes += usize::from(n.extras().mask.is_some() || n.blend == BlendMode::Multiply));
        assert!(scopes > 0, "{what}: drawn in its scope");
        assert_eq!(face_paths(&art), unmasked, "{what}: the faces in the member's one order");
    }
    // A translucent stroke keeps its surface whole: the fill's faces, then the stroke's.
    if let Some(AppearanceItem::Stroke(s)) = masked.appearance.items.iter_mut().find(|i| matches!(i, AppearanceItem::Stroke(_))) {
        s.opacity = 0.5;
    }
    let art = solid(masked);
    let mut groups = vec![];
    art.walk(&mut |n| {
        if n.opacity == 0.5 {
            groups.push(n.children().map_or(0, Vec::len));
        }
    });
    assert_eq!(groups.len(), 1, "the stroke's surface is one translucent group");
}

/// Under a light that is not white, Inflate in gradients, whose faces are drawn one by one, draws the
/// fill's and the dashed stroke's faces in one order, as it does under a white light. A solid color,
/// drawn as one composite of each surface's facets, still draws each surface whole.
#[test]
fn a_colored_inflate_in_gradients_draws_its_faces_in_one_order() {
    use vectorcraft_doc::color::{Gradient, GradientKind, GradientPaint, GradientStop};
    let gradient = |c: Color| {
        Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(
            GradientKind::Linear,
            vec![GradientStop::new(0.0, c), GradientStop::new(1.0, Color::WHITE)],
        ))))
    };
    let mut n = stroked(&letter_c());
    for item in &mut n.appearance.items {
        match item {
            AppearanceItem::Fill(f) => f.paint = gradient(RED),
            AppearanceItem::Stroke(s) => {
                s.paint = gradient(BLUE);
                s.dash = Some(vectorcraft_doc::Dash { pattern: vec![12.0, 12.0], offset: 0.0, align_corners: false });
            }
        }
    }
    // How many surfaces the art draws, and how many faces the first holds.
    let drawn = |n: &Node, lights: Option<Value>| {
        let mut n = n.clone();
        let mut p = json!({"rotationX": -20, "rotationY": -40, "rotationZ": 0, "depth": 60});
        if let Some(lights) = lights {
            p["lights"] = lights;
        }
        n.appearance.effects.push(fx(INFLATE, p));
        let art = solid_art(&n).unwrap();
        let surfaces = art.children().unwrap();
        (surfaces.len(), surfaces[0].children().map_or(0, Vec::len))
    };
    let colored = json!([{"color": "#ff8080"}]);
    let white = drawn(&n, None);
    assert_eq!(white.0, 1, "one order");
    let cut = drawn_parts(&n, INFLATE, &json!({"rotationX": -20, "rotationY": -40, "rotationZ": 0, "depth": 60})).len();
    assert_eq!(white.1, cut, "the white light's faces, cut");
    assert_eq!(drawn(&n, Some(colored.clone())).0, 1, "one order under the colored light");
    // A solid color under the colored light: a composite for each of the fill and the stroke.
    let solid = stroked(&letter_c());
    assert_eq!(drawn(&solid, Some(colored)).0, 2);
}

/// A face of a stroke that leaves gaps passing through the plane of its fill's wall where they
/// overlap is cut along the line where their depths meet, into the piece in front of the wall and
/// the piece behind it; a face on one side of the wall, another shape's stroke, a covering stroke
/// and a cut past the room left are left whole.
#[test]
fn a_stroke_face_through_a_fill_wall_is_cut_where_their_depths_meet() {
    let facet = |x: f64, normal: [f64; 3], offset: f64| Facet {
        contours: vec![vec![Point::new(x, 0.0), Point::new(x + 10.0, 0.0), Point::new(x + 10.0, 10.0), Point::new(x, 10.0)]],
        depth: 0.0,
        brightness: 1.0,
        illumination: [1.0; 3],
        normal,
        rgb_gradient: None,
        outline: None,
        side: true,
        away: false,
        end: None,
        plane: Some(DepthPlane { normal, offset, center: Point::ZERO, eye: None }),
    };
    let wall = facet(0.0, [0.0, 0.0, 1.0], 0.0);
    // z = x - 8: behind the wall left of x = 8, in front of it right of it.
    let r = std::f64::consts::FRAC_1_SQRT_2;
    let mut through = facet(5.0, [-r, 0.0, r], -8.0 * r);
    // Its curved outline: the top edge bows up to y = -2.
    let mut curved = BezPath::new();
    curved.move_to((5.0, 0.0));
    curved.curve_to((8.0, -8.0 / 3.0), (12.0, -8.0 / 3.0), (15.0, 0.0));
    curved.line_to((15.0, 10.0));
    curved.line_to((5.0, 10.0));
    curved.close_path();
    through.outline = Some(PathData::from_bezpath(&curved));
    let extent = |f: &Facet| polygon_bounds(&f.contours[0]);
    let cut_with = |paints: Paints, member: usize, room: usize| {
        let (mut fill, mut stroke) = (vec![wall.clone()], vec![through.clone()]);
        let mut solids: Vec<(Paints, usize, &mut Vec<Facet>)> = vec![(Paints::Fill, 0, &mut fill), (paints, member, &mut stroke)];
        cut_stroke_faces(&mut solids, room);
        stroke
    };
    let pieces = cut_with(Paints::Other, 0, 10);
    assert_eq!(pieces.len(), 2);
    let (near, far) = (extent(&pieces[0]), extent(&pieces[1]));
    assert!((near.x0 - 8.0).abs() < 1e-9 && near.x1 == 15.0 && far.x0 == 5.0 && (far.x1 - 8.0).abs() < 1e-9, "{near:?} {far:?}");
    assert!(pieces[0].depth > 0.0 && pieces[1].depth < 0.0);
    // Each piece keeps its side of the curved outline, the bowed edge included.
    let curve = |f: &Facet| f.outline.as_ref().and_then(PathData::bounds).unwrap();
    let (near, far) = (curve(&pieces[0]), curve(&pieces[1]));
    assert!((near.x0 - 8.0).abs() < 1e-6 && (near.x1 - 15.0).abs() < 1e-6 && near.y0 < -1.5, "{near:?}");
    assert!((far.x0 - 5.0).abs() < 1e-6 && (far.x1 - 8.0).abs() < 1e-6 && far.y0 < -1.5, "{far:?}");
    let whole = std::slice::from_ref(&through);
    assert_eq!(cut_with(Paints::Other, 1, 10), whole, "another shape's stroke");
    assert_eq!(cut_with(Paints::Stroke, 0, 10), whole, "a covering stroke");
    assert_eq!(cut_with(Paints::Other, 0, 0), whole, "no room");
    let (mut fill, mut stroke) = (vec![wall.clone()], vec![facet(5.0, [0.0, 0.0, 1.0], 1.0)]);
    cut_stroke_faces(&mut [(Paints::Fill, 0, &mut fill), (Paints::Other, 0, &mut stroke)], 10);
    assert_eq!(stroke.len(), 1, "in front of the wall wherever they overlap");
    // Of two of a shape's strokes whose faces pass through each other, the later stroke's face is
    // cut against the earlier one's; a covering stroke's face is never cut, and the other stroke's
    // face is cut against it. Two shapes' strokes are left whole.
    for (case, (first, second, cut_first)) in
        [(Paints::Other, Paints::Other, false), (Paints::Stroke, Paints::Other, false), (Paints::Other, Paints::Stroke, true)].into_iter().enumerate()
    {
        let (mut a, mut b) = (vec![wall.clone()], vec![through.clone()]);
        cut_stroke_faces(&mut [(first, 0, &mut a), (second, 0, &mut b)], 10);
        assert_eq!((a.len(), b.len()), if cut_first { (2, 1) } else { (1, 2) }, "case {case}");
    }
    let (mut a, mut b) = (vec![wall.clone()], vec![through.clone()]);
    cut_stroke_faces(&mut [(Paints::Other, 0, &mut a), (Paints::Other, 1, &mut b)], 10);
    assert_eq!((a.len(), b.len()), (1, 1), "two shapes");
}

/// A small polygon far from the origin keeps its area and centroid.
#[test]
fn a_small_polygon_far_away_has_a_centroid() {
    let far = 100_000_000.0;
    let square = [Point::new(far, far), Point::new(far + 1.0, far), Point::new(far + 1.0, far + 1.0), Point::new(far, far + 1.0)];
    let c = centroid(&square).unwrap();
    assert!((c.x - far - 0.5).abs() < 1e-6 && (c.y - far - 0.5).abs() < 1e-6, "{c:?}");
    assert!(centroid(&[]).is_none() && centroid(&square[..2]).is_none());
}

/// Cutting adds no more faces than the object's face limit and the paint-copy allowance leave room
/// for, so an effect that draws with its faces uncut still draws.
#[test]
fn cutting_stays_within_the_face_limit_and_the_paint_allowance() {
    let items = stroked(&letter_c()).appearance.items.clone();
    let stroke = items.iter().find(|i| matches!(i, AppearanceItem::Stroke(_))).unwrap();
    let mut one = 0usize;
    crate::revolve::paint_copies_add(&mut one, 6, paint_ref(stroke));
    let max = crate::revolve::MAX_PAINT_COPIES;
    assert_eq!(cut_room(items.iter(), 10, 0), (MAX_OBJECT_FACES - 10).min(max / one));
    assert_eq!(cut_room(items.iter(), MAX_OBJECT_FACES + 5, 0), 0);
    assert_eq!(cut_room(items.iter(), 10, max - 3 * one), 3);
    assert_eq!(cut_room(items.iter(), 10, max + 1), 0);
}

/// Where a shape's fill wall and a face of its stroke that leaves gaps overlap, the one nearer the
/// viewer there is drawn after the other, whichever their depths put first; a stroke face that
/// overlaps no fill wall keeps its place.
#[test]
fn local_order_puts_overlapping_fill_walls_and_stroke_faces_in_depth_order() {
    let square = |x: f64| vec![Point::new(x, 0.0), Point::new(x + 10.0, 0.0), Point::new(x + 10.0, 10.0), Point::new(x, 10.0)];
    let at = |depth: f64, x: f64, z: f64, paints: Paints| Placed {
        depth,
        side: true,
        away: false,
        bounds: Rect::new(x, 0.0, x + 10.0, 10.0),
        member: 0,
        paints,
        outline: square(x),
        plane: Some(DepthPlane { normal: [0.0, 0.0, 1.0], offset: z, center: Point::ZERO, eye: None }),
    };
    // 0 a fill wall at z 0; 1 a stroke face over half of it, nearer, sorted before it; 2 a stroke
    // face elsewhere; 3 a stroke face over it, farther, sorted after it.
    let faces =
        [at(5.0, 0.0, 0.0, Paints::Fill), at(2.0, 5.0, 1.0, Paints::Other), at(1.0, 20.0, 1.0, Paints::Other), at(6.0, 2.0, -1.0, Paints::Other)];
    assert_eq!(draw_order(&faces, true), [2, 3, 0, 1]);
    assert_eq!(draw_order(&faces, false), [2, 3, 0, 1]);
    // A covering stroke's faces keep the wall's own rule.
    let covering: Vec<Placed> = faces
        .iter()
        .map(|f| Placed { paints: if f.paints == Paints::Other { Paints::Stroke } else { f.paints }, outline: vec![], plane: None, ..*f })
        .collect();
    assert_eq!(draw_order(&covering, false), [2, 1, 0, 3]);
    // Another shape's stroke, and a stroke face at the same depth where they overlap, move nothing.
    let other_shape: Vec<Placed> =
        faces[..3].iter().map(|f| Placed { member: usize::from(f.paints == Paints::Other), outline: f.outline.clone(), ..*f }).collect();
    assert_eq!(draw_order(&other_shape, true), [2, 1, 0]);
    let level = [at(5.0, 0.0, 0.0, Paints::Fill), at(2.0, 5.0, 0.0, Paints::Other)];
    assert_eq!(draw_order(&level, true), [1, 0]);
    // A stroke face moved after its fill's wall does not pass a nearer face of another shape that
    // it overlaps: 0 a stroke face, 1 another shape's face in front of both, 2 the fill's wall.
    let in_front = Placed { member: 1, ..at(2.0, 5.0, 2.0, Paints::Fill) };
    let shapes = [at(1.0, 5.0, 1.0, Paints::Other), in_front, at(3.0, 0.0, 0.0, Paints::Fill)];
    assert_eq!(draw_order(&shapes, true), [2, 0, 1]);
    // A shape with a covering stroke as well keeps its fill wall before the covering stroke's face,
    // even where that face is the farther: 0 the fill's wall, 1 the covering stroke's face behind
    // it, 2 the other stroke's face in front.
    let both = [at(3.0, 0.0, 0.0, Paints::Fill), at(3.0, 0.0, -1.0, Paints::Stroke), at(2.0, 5.0, 1.0, Paints::Other)];
    assert_eq!(draw_order(&both, true), [0, 1, 2]);
    // Faces of a shape's strokes go in the order of their depths where they overlap too, with no
    // fill face among them: 1 is nearer than 0 there, though sorted before it.
    for (case, (back, front)) in
        [(Paints::Other, Paints::Other), (Paints::Stroke, Paints::Other), (Paints::Stroke, Paints::Stroke)].into_iter().enumerate()
    {
        let strokes = [at(5.0, 0.0, 0.0, back), at(2.0, 5.0, 1.0, front)];
        assert_eq!(draw_order(&strokes, true), [0, 1], "case {case}");
    }
}

/// A cycle of overlapping stroke faces is broken at the face that comes first by depth, and a cap
/// drawn last stays after every wall: 0 at x 0 to 10, flat at z 0; 1 at x 4 to 14, z = x - 8; 2 at
/// x 8 to 18, flat at z 1. Where they overlap, 0 is nearer than 1, 1 nearer than 2 and 2 nearer
/// than 0. 3 is the cap.
#[test]
fn a_depth_cycle_keeps_the_cap_last() {
    let square = |x: f64| vec![Point::new(x, 0.0), Point::new(x + 10.0, 0.0), Point::new(x + 10.0, 10.0), Point::new(x, 10.0)];
    let at = |depth: f64, x: f64, normal: [f64; 3], offset: f64| Placed {
        depth,
        side: true,
        away: false,
        bounds: Rect::new(x, 0.0, x + 10.0, 10.0),
        member: 0,
        paints: Paints::Other,
        outline: square(x),
        plane: Some(DepthPlane { normal, offset, center: Point::ZERO, eye: None }),
    };
    let cap = Placed {
        depth: f64::MAX,
        side: false,
        away: false,
        bounds: Rect::new(0.0, 0.0, 18.0, 10.0),
        member: 0,
        paints: Paints::Other,
        outline: vec![],
        plane: None,
    };
    let faces = [at(1.0, 0.0, [0.0, 0.0, 1.0], 0.0), at(2.0, 4.0, [-1.0, 0.0, 1.0], -8.0), at(3.0, 8.0, [0.0, 0.0, 1.0], 1.0), cap];
    assert_eq!(draw_order(&faces, true), [0, 2, 1, 3]);
}

/// The order keeps depths except where a shape's fill wall turned toward the viewer overlaps a
/// face of the same shape's stroke drawn before it: the wall then goes just before that face. A
/// cap, a face turned away, another shape's stroke, a stroke face the wall does not overlap and an
/// effect without caps keep the order of the depths.
#[test]
fn the_draw_order_moves_a_fill_wall_before_the_stroke_faces_it_overlaps() {
    let at = |depth: f64, x: f64, member: usize, paints: Paints, side: bool, away: bool| Placed {
        depth,
        side,
        away,
        bounds: Rect::new(x, 0.0, x + 10.0, 10.0),
        member,
        paints,
        outline: vec![],
        plane: None,
    };
    // 0 the fill's wall; 1 the stroke's wall over it; 2 another shape's stroke over it; 3 a stroke
    // wall elsewhere; 4 a stroke face turned away; 5 a fill cap; 6 the stroke's cap, nearer; 7 a
    // fill wall at the stroke wall's depth.
    let faces = [
        at(5.0, 0.0, 0, Paints::Fill, true, false),
        at(3.0, 5.0, 0, Paints::Stroke, true, false),
        at(2.0, 5.0, 1, Paints::Stroke, true, false),
        at(1.0, 50.0, 0, Paints::Stroke, true, false),
        at(0.0, 0.0, 0, Paints::Stroke, true, true),
        at(4.0, 0.0, 0, Paints::Fill, false, false),
        at(6.0, 0.0, 0, Paints::Stroke, false, false),
        at(3.0, 0.0, 0, Paints::Fill, true, false),
    ];
    assert_eq!(draw_order(&faces, true), [4, 3, 2, 0, 7, 1, 5, 6]);
    assert_eq!(draw_order(&faces, false), [4, 3, 2, 1, 7, 5, 0, 6]);
    // A shape with no stroke, and a stroke without a width, move nothing.
    // Only a stroke whose solid hides the fill's whole edge counts: an opaque, plain stroke with a
    // width on a closed shape.
    let stroke = |f: &dyn Fn(&mut StrokeLayer)| {
        let mut s = StrokeLayer::new(Paint::solid(BLUE), 6.0);
        f(&mut s);
        AppearanceItem::Stroke(s)
    };
    assert!(Paints::of(&stroke(&|_| {}), true) == Paints::Stroke);
    assert!(Paints::of(&stroke(&|_| {}), false) == Paints::Other, "an open shape");
    let closed = profile(&stroked(&letter_c())).unwrap().0;
    assert!(holds_edge(EXTRUDE, &closed) && holds_edge(CLASSIC_EXTRUDE, &closed));
    assert!(!holds_edge(INFLATE, &closed), "an inflated fill can rise in front of its stroke");
    assert!(Paints::of(&stroke(&|s| s.width = 0.0), true) == Paints::Other, "no width");
    let dash = vectorcraft_doc::Dash { pattern: vec![4.0, 4.0], offset: 0.0, align_corners: false };
    assert!(Paints::of(&stroke(&|s| s.dash = Some(dash.clone())), true) == Paints::Other, "dashes");
    assert!(Paints::of(&stroke(&|s| s.brush = Some("brush".into())), true) == Paints::Other);
    assert!(Paints::of(&stroke(&|s| s.paint = Paint::None), true) == Paints::Other);
    assert!(Paints::of(&stroke(&|s| s.paint = Paint::Pattern { pattern: "p".into(), xf: Affine::IDENTITY }), true) == Paints::Other, "a pattern");
    use vectorcraft_doc::color::{Gradient, GradientKind, GradientPaint, GradientStop};
    let gradient = |opacity: f32| {
        let mut end = GradientStop::new(1.0, RED);
        end.opacity = opacity;
        Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, vec![GradientStop::new(0.0, BLUE), end]))))
    };
    assert!(Paints::of(&stroke(&|s| s.paint = gradient(1.0)), true) == Paints::Stroke, "an opaque gradient");
    assert!(Paints::of(&stroke(&|s| s.paint = gradient(0.5)), true) == Paints::Other, "a gradient with a transparent stop");
    let other = |p: Paints| if p == Paints::Stroke { Paints::Other } else { p };
    let unstroked: Vec<Placed> = faces.iter().map(|f| at(f.depth, f.bounds.x0, f.member, other(f.paints), f.side, f.away)).collect();
    assert_eq!(draw_order(&unstroked, true), draw_order(&faces, false));
}

/// A stroked open path builds a ribbon, the stroke's outline, not the area the path would enclose
/// if it were closed: for an L of two 100 pt legs, a 6 pt band with butt ends and a mitered corner.
#[test]
fn a_stroked_open_path_builds_a_ribbon() {
    let mut b = BezPath::new();
    b.move_to((100.0, 100.0));
    b.line_to((200.0, 100.0));
    b.line_to((200.0, 200.0));
    for id in [EXTRUDE, INFLATE] {
        let mut n = Node::path(NodeId(1), PathData::from_bezpath(&b), Appearance::basic(Paint::None, Paint::solid(BLUE), 6.0));
        n.appearance.effects.push(fx(id, json!({"rotationX": 0, "rotationY": 0, "rotationZ": 0, "volume": 0})));
        let art = solid_art(&n).unwrap();
        let faces = faces(&art);
        assert!(faces.iter().all(|f| blue(f)), "{id}");
        assert_rect(extent(&faces), 100.0, 97.0, 203.0, 200.0);
        let area: f64 = faces.iter().map(|f| f.path_data().unwrap().to_bezpath().area().abs()).sum();
        // 6 × 200 less the 3 × 3 the legs share, plus the 3 × 3 miter.
        assert!((area - 1200.0).abs() < 1e-6, "{id}: {area}");
    }
}

/// A stroke whose outline takes more points than a solid takes is left out, and the fill's solid
/// still draws.
#[test]
fn a_stroke_past_the_limits_is_left_out() {
    // 40,000 corners: within the 65,536 points of the fill's solid, but not the stroke's outline.
    let points: Vec<Point> = (0..40_000)
        .map(|k| {
            let (s, c) = (k as f64 * std::f64::consts::TAU / 40_000.0).sin_cos();
            let r = if k % 2 == 0 { 10_000.0 } else { 9_999.0 };
            Point::new(20_000.0 + r * c, 20_000.0 + r * s)
        })
        .collect();
    let path = PathData::single(vectorcraft_geom::SubPath::polyline(&points, true));
    for id in [EXTRUDE, INFLATE] {
        let mut n = Node::path(NodeId(1), path.clone(), Appearance::basic(Paint::solid(RED), Paint::solid(BLUE), 2.0));
        n.appearance.effects.push(fx(id, json!({})));
        let art = solid_art(&n).unwrap_or_else(|| panic!("{id}: the fill's solid draws"));
        assert!(!faces(&art).is_empty() && faces(&art).iter().all(|f| !blue(f)), "{id}");
    }
}

/// Each solid [`built_solids`] gives for effect `id` on `n` within `limit` faces together, as its
/// face count (`None` for a stroke left out) or the error, every result the iterator gives.
fn solid_faces(id: &str, n: &Node, limit: usize) -> Vec<Result<Option<usize>, String>> {
    let shapes = [Profile { path: profile(n).unwrap().0, rule: FillRule::NonZero, items: solid_paints(n), opacity: 1.0, scopes: vec![] }];
    let p = crate::merged_params(id, &json!({}));
    let (parts, bounds) = solid_parts(&shapes, union_bounds(&shapes).unwrap());
    built_solids(id, &p, &parts, bounds, limit).map(|r| r.map(|f| f.map(|f| f.len()))).collect()
}

/// The solids of one object count their faces together: at the limit they all build, one face past
/// it is an error, and the solids after the one that passes it are never built.
#[test]
fn an_objects_solids_count_their_faces_together() {
    for id in [EXTRUDE, CLASSIC_EXTRUDE, INFLATE] {
        let mut n = square_node();
        // A second stroke, so a solid follows the one that passes the limit.
        let stroke = n.appearance.items[1].clone();
        n.appearance.items.push(stroke);
        let counts: Vec<usize> = solid_faces(id, &n, MAX_OBJECT_FACES).into_iter().map(|c| c.unwrap().unwrap()).collect();
        assert!(counts.len() == 3 && counts.iter().all(|c| *c > 0), "{id}: {counts:?}");
        let total: usize = counts.iter().sum();
        assert!(solid_faces(id, &n, total).iter().all(Result::is_ok), "{id}: at the limit");
        let past = solid_faces(id, &n, total - 1);
        assert_eq!(past.last(), Some(&Err(vectorcraft_three_d::TOO_COMPLEX_SOLID.to_string())), "{id}");
        SOLIDS_BUILT.with(|b| b.set(0));
        let past = solid_faces(id, &n, counts[0] + counts[1] - 1);
        assert!(past.len() == 2 && past[1].is_err(), "{id}: {past:?}");
        assert_eq!(SOLIDS_BUILT.with(std::cell::Cell::get), 2, "{id}: the second stroke's solid is never built");
    }
}

/// A convex polygon of `corners` corners about (20,000, 20,000), filled red, with no stroke.
fn polygon_node(id: u64, corners: usize) -> Node {
    let points: Vec<Point> = (0..corners)
        .map(|k| {
            let (s, c) = (k as f64 * std::f64::consts::TAU / corners as f64).sin_cos();
            Point::new(20_000.0 + 10_000.0 * c, 20_000.0 + 10_000.0 * s)
        })
        .collect();
    let path = PathData::single(vectorcraft_geom::SubPath::polyline(&points, true));
    Node::path(NodeId(id), path, Appearance::basic(Paint::solid(RED), Paint::None, 0.0))
}

/// A group whose members' solids each fit, but not all of them together, draws its source art, and
/// the commands that check the effect report the error, as do its mesh and its wireframe; one
/// member fewer, within the limit, draws.
#[test]
fn a_groups_solids_together_past_the_limit_draw_the_source() {
    let member = polygon_node(1, 16_384);
    let mut alone = member.clone();
    alone.appearance.effects.push(fx(EXTRUDE, json!({})));
    let [Ok(Some(each))] = solid_faces(EXTRUDE, &alone, MAX_OBJECT_FACES)[..] else { panic!("one solid") };
    // Members stacked in one place each build the same solid.
    let fits = MAX_OBJECT_FACES / each;
    let group = |count: usize| {
        let members = (0..count)
            .map(|k| {
                let mut m = member.clone();
                m.id = NodeId(k as u64 + 1);
                Arc::new(m)
            })
            .collect();
        let mut g = Node::group(NodeId(9999), members);
        g.appearance.effects.push(fx(EXTRUDE, json!({})));
        g
    };
    let within = group(fits);
    assert!(validate_solid(&within, EXTRUDE, &json!({})).is_ok(), "{fits} × {each} faces");
    assert!(solid_art(&within).is_some());
    let past = group(fits + 1);
    assert_eq!(validate_solid(&past, EXTRUDE, &json!({})), Err(vectorcraft_three_d::TOO_COMPLEX_SOLID.to_string()));
    assert!(solid_art(&past).is_none(), "the source art draws");
    assert_eq!(three_d_meshes(&past), Err(vectorcraft_three_d::TOO_COMPLEX_SOLID.to_string()));
    assert_eq!(three_d_wireframe_in(None, &past), Err(vectorcraft_three_d::TOO_COMPLEX_SOLID.to_string()));
}

/// A fill and strokes whose solids each fit, but not all of them together, draw the source art,
/// and the strokes after the one past the limit are not left out to make room.
#[test]
fn a_fill_and_strokes_together_past_the_limit_draw_the_source() {
    let mut n = polygon_node(1, 16_384);
    n.appearance.effects.push(fx(EXTRUDE, json!({})));
    let stroke = AppearanceItem::Stroke(StrokeLayer::new(Paint::solid(BLUE), 20.0));
    n.appearance.items.push(stroke.clone());
    // Strokes alike each build the same solid.
    let [Ok(Some(fill)), Ok(Some(each))] = solid_faces(EXTRUDE, &n, MAX_OBJECT_FACES)[..] else { panic!("a fill's solid and a stroke's") };
    let fits = (MAX_OBJECT_FACES - fill) / each;
    assert!(fits >= 2, "{fill} + {fits} × {each}");
    n.appearance.items.extend(std::iter::repeat_n(stroke, fits - 1));
    assert!(validate_solid(&n, EXTRUDE, &json!({})).is_ok(), "{fill} + {fits} × {each}");
    let art = solid_art(&n).unwrap();
    assert!(faces(&art).iter().any(|f| blue(f)));
    n.appearance.items.push(AppearanceItem::Stroke(StrokeLayer::new(Paint::solid(BLUE), 20.0)));
    assert_eq!(validate_solid(&n, EXTRUDE, &json!({})), Err(vectorcraft_three_d::TOO_COMPLEX_SOLID.to_string()));
    assert!(solid_art(&n).is_none(), "the source art draws");
}

/// The check builds the solids from the shapes the art draws them from: after the geometry effects
/// before the 3D effect, on an object, on its fill and on a group. Copies a Transform effect before
/// it makes that take the solids past the limit fail the check, as the art then draws the source.
#[test]
fn the_check_counts_the_faces_after_the_geometry_effects_before() {
    let mut alone = polygon_node(1, 16_384);
    alone.appearance.effects.push(fx(EXTRUDE, json!({})));
    let [Ok(Some(each))] = solid_faces(EXTRUDE, &alone, MAX_OBJECT_FACES)[..] else { panic!("one solid") };
    // Side by side, each copy builds the same solid.
    let copies = |k: usize| fx("distort.transform", json!({"moveH": 30_000.0, "copies": k}));
    for (k, fits) in [(1, true), (MAX_OBJECT_FACES / each, false)] {
        let mut n = polygon_node(1, 16_384);
        n.appearance.effects = vec![copies(k), fx(EXTRUDE, json!({}))];
        assert_eq!(validate_solid(&n, EXTRUDE, &json!({})).is_ok(), fits, "on the object, {k} copies");
        assert_eq!(solid_art(&n).is_some(), fits, "on the object, {k} copies");
        let mut f = polygon_node(1, 16_384);
        f.appearance.items[0].effects_mut().extend([copies(k), fx(EXTRUDE, json!({}))]);
        assert_eq!(validate_three_d_at(&f, EXTRUDE, &json!({}), (Some(0), 1)).is_ok(), fits, "on the fill, {k} copies");
        assert_eq!(solid_art(&f).is_some(), fits, "on the fill, {k} copies");
        let mut g = Node::group(NodeId(9999), vec![Arc::new(polygon_node(1, 16_384))]);
        g.appearance.effects = vec![copies(k), fx(EXTRUDE, json!({}))];
        assert_eq!(validate_solid(&g, EXTRUDE, &json!({})).is_ok(), fits, "on a group, {k} copies");
        assert_eq!(solid_art(&g).is_some(), fits, "on a group, {k} copies");
        // A new effect goes after the copies.
        let mut new = polygon_node(1, 16_384);
        new.appearance.effects = vec![copies(k)];
        assert_eq!(validate_three_d_at(&new, EXTRUDE, &json!({}), (None, 1)).is_ok(), fits, "a new effect, {k} copies");
    }
    // A hidden effect checked at its own index counts only the effects before it: copies after it
    // copy the drawn art once it is shown again.
    let mut hidden = polygon_node(1, 16_384);
    let mut extrude = fx(EXTRUDE, json!({}));
    extrude.visible = false;
    hidden.appearance.effects = vec![extrude, copies(MAX_OBJECT_FACES / each)];
    assert!(validate_three_d_at(&hidden, EXTRUDE, &json!({}), (None, 0)).is_ok());
    hidden.appearance.effects[0].visible = true;
    assert!(solid_art(&hidden).is_some());
}

/// At no volume Inflate's front is flat: turned, the fill's cells are all drawn before the stroke's,
/// whatever their depths, as the extrusion's caps are. When the fill puffs, a stroke too narrow to
/// puff on the grid stays flat and keeps its depths: the fill's puff is drawn over its far side.
#[test]
fn a_flat_inflate_draws_the_strokes_front_above_the_fills() {
    // On the 100 pt square's grid (cells 101 / 48 pt apart from 99.5) no grid point falls inside a
    // 1 pt stroke, so the stroke stays flat.
    for (volume, width, length) in [(0.0, 6.0, 300.0), (50.0, 1.0, 100.0)] {
        let mut n = stroked_rect(length);
        if let Some(AppearanceItem::Stroke(s)) = n.appearance.items.iter_mut().find(|i| matches!(i, AppearanceItem::Stroke(_))) {
            s.width = width;
        }
        n.appearance.effects.push(fx(INFLATE, json!({ "volume": volume })));
        let art = solid_art(&n).unwrap();
        let faces = faces(&art);
        let first_blue = faces.iter().position(|f| blue(f)).unwrap();
        let last_red = faces.iter().rposition(|f| !blue(f)).unwrap();
        assert_eq!(last_red < first_blue, volume == 0.0, "{volume}: red at {last_red}, blue from {first_blue} of {}", faces.len());
    }
}

/// With Perspective one eye sees the fill's puff and the stroke's, however much higher one rises:
/// seen from behind, the back of a stroke aligned outside meets the fill's back exactly.
#[test]
fn inflate_sees_the_fill_and_the_stroke_from_one_eye() {
    let mut n = stroked_rect(100.0);
    if let Some(AppearanceItem::Stroke(s)) = n.appearance.items.iter_mut().find(|i| matches!(i, AppearanceItem::Stroke(_))) {
        s.align = vectorcraft_doc::StrokeAlign::Outside;
    }
    let params = json!({"volume": 100, "depth": 100, "perspectiveAngle": 120, "rotationX": 15, "rotationY": 160, "rotationZ": 0});
    n.appearance.effects.push(fx(INFLATE, params));
    let art = solid_art(&n).unwrap();
    let [.., fill_back, stroke_back] = faces(&art)[..] else { panic!("two backs") };
    assert!(!blue(fill_back) && blue(stroke_back));
    let fill = fill_back.path_data().unwrap().bounds().unwrap();
    // The ring's inner contour: the smaller of its two.
    let rims = stroke_back.path_data().unwrap().subpaths.iter().filter_map(|s| PathData::new(vec![s.clone()]).bounds());
    let hole = rims.reduce(|a, b| a.intersect(b)).unwrap();
    assert_rect(hole, fill.x0, fill.y0, fill.x1, fill.y1);
}

/// A stroke with its own transparency or blending is a surface of its own above the fill's, so its
/// opacity applies to the stroke as a whole.
#[test]
fn a_translucent_stroke_is_a_surface_of_its_own() {
    for (opacity, blend) in [(0.5, BlendMode::Normal), (1.0, BlendMode::Multiply)] {
        let mut n = stroked_rect(100.0);
        if let Some(AppearanceItem::Stroke(s)) = n.appearance.items.iter_mut().find(|i| matches!(i, AppearanceItem::Stroke(_))) {
            s.opacity = opacity;
            s.blend = blend;
        }
        n.appearance.effects.push(fx(EXTRUDE, json!({})));
        let art = solid_art(&n).unwrap();
        let surfaces = art.children().unwrap();
        assert_eq!(surfaces.len(), 2);
        assert_eq!((surfaces[0].opacity, surfaces[0].blend), (1.0, BlendMode::Normal));
        assert_eq!((surfaces[1].opacity, surfaces[1].blend), (opacity, blend));
        assert!(surfaces[0].children().unwrap().iter().all(|f| !blue(f)) && surfaces[1].children().unwrap().iter().all(|f| blue(f)));
    }
}

/// With a light that is not white, a fill in a gradient with transparent stops is drawn with an
/// opacity mask: its faces still take their places in the one order of the fill's and the
/// stroke's faces, in runs that each carry the mask. Under a white light the faces carry the
/// gradient itself and no mask.
#[test]
fn a_masked_surface_under_a_colored_light_keeps_the_shared_face_order() {
    use vectorcraft_doc::color::{Gradient, GradientKind, GradientPaint, GradientStop};
    let mut half = GradientStop::new(1.0, RED);
    half.opacity = 0.5;
    let gradient = GradientPaint::new(Gradient::new(GradientKind::Linear, vec![GradientStop::new(0.0, RED), half]));
    for (lights, masked) in [(json!({"lights": [{"color": "#ff8080"}]}), true), (json!({}), false)] {
        let mut n = stroked_rect(100.0);
        if let Some(AppearanceItem::Fill(f)) = n.appearance.items.iter_mut().find(|i| matches!(i, AppearanceItem::Fill(_))) {
            f.paint = Paint::Gradient(Box::new(gradient.clone()));
        }
        n.appearance.effects.push(fx(EXTRUDE, lights.clone()));
        let art = solid_art(&n).unwrap();
        let drawn = art.children().unwrap();
        assert_eq!(drawn.len(), 1, "{lights}: one order of faces");
        let runs: Vec<&Arc<Node>> = drawn[0].children().unwrap().iter().filter(|c| c.extras().mask.is_some()).collect();
        assert_eq!(!runs.is_empty(), masked, "{lights}");
        assert!(runs.iter().all(|r| r.children().unwrap().iter().all(|f| !blue(f))), "{lights}: the mask stays on the fill's faces");
    }
}

#[test]
fn transparency_stays_on_the_whole_surface_and_items_take_their_own_effect() {
    let mut n = square_node();
    n.opacity = 0.5;
    if let AppearanceItem::Fill(f) = &mut n.appearance.items[0] {
        f.opacity = 0.4;
        f.effects.push(fx(INFLATE, json!({})));
    }
    let art = solid_art(&n).unwrap();
    assert_eq!(art.opacity, 0.5);
    let surfaces = art.children().unwrap();
    // The fill inflated, the stroke drawn flat as it was.
    assert_eq!(surfaces.len(), 2);
    assert_eq!(surfaces[0].opacity, 0.4);
    assert!(surfaces[0].children().unwrap().iter().all(|c| c.opacity == 1.0));
    assert!(matches!(surfaces[1].kind, NodeKind::Path { .. }));
}

/// Two nested squares drawn in the same direction, so only the even-odd rule leaves a hole.
fn nested_squares() -> [PathData; 2] {
    [(100.0, 200.0), (130.0, 170.0)].map(|(lo, hi)| {
        let mut b = BezPath::new();
        b.move_to((lo, lo));
        b.line_to((hi, lo));
        b.line_to((hi, hi));
        b.line_to((lo, hi));
        b.close_path();
        PathData::from_bezpath(&b)
    })
}

/// The nested squares as an even-odd compound path and as one even-odd path, painted by `items`.
fn even_odd_shapes(items: Vec<AppearanceItem>) -> [Node; 2] {
    let [outer, inner] = nested_squares();
    let appearance = Appearance { items, ..Default::default() };
    let children = vec![
        Arc::new(Node::path(NodeId(2), outer.clone(), Appearance::default())),
        Arc::new(Node::path(NodeId(3), inner.clone(), Appearance::default())),
    ];
    let mut compound = Node::new(NodeId(1), NodeKind::Compound { children, rule: FillRule::EvenOdd });
    compound.appearance = appearance.clone().into();
    let both = PathData::new(outer.subpaths.into_iter().chain(inner.subpaths).collect());
    [compound, crate::group::shape_node(NodeId(1), both, FillRule::EvenOdd, appearance)]
}

/// Check that `art` draws `item` as one flat child: an even-odd path with a hole in the middle,
/// below the 3D art after it.
fn assert_flat_item_keeps_the_hole(art: &Node, item: &AppearanceItem, what: &str) {
    use vectorcraft_geom::hit::fill_contains;
    let children = art.children().unwrap();
    let flat: Vec<usize> =
        (0..children.len()).filter(|&i| children[i].appearance.items == [item.clone()] && children[i].path_data().is_some()).collect();
    let [index] = flat[..] else { panic!("{what}: one flat child, got {flat:?}") };
    let NodeKind::Path { path, rule, .. } = &children[index].kind else { panic!("{what}: a path") };
    assert_eq!(*rule, FillRule::EvenOdd, "{what}");
    let bez = path.to_bezpath();
    assert!(fill_contains(&bez, *rule, Point::new(110.0, 150.0)), "{what}: the ring is filled");
    assert!(!fill_contains(&bez, *rule, Point::new(150.0, 150.0)), "{what}: the hole stays open");
    assert!(index + 1 < children.len(), "{what}: the 3D art is drawn above");
}

#[test]
fn a_fill_or_stroke_without_the_effect_keeps_the_even_odd_hole() {
    let red = AppearanceItem::Fill(vectorcraft_doc::FillLayer::new(Paint::solid(RED)));
    let blue = AppearanceItem::Fill(vectorcraft_doc::FillLayer::new(Paint::solid(BLUE)));
    let stroke = AppearanceItem::Stroke(StrokeLayer::new(Paint::solid(Color::BLACK), 4.0));
    for id in SOLIDS {
        // The top fill and then the top stroke take the effect; the fill below stays flat.
        for (top, on) in [(&red, "fill"), (&stroke, "stroke")] {
            let mut lifted = top.clone();
            lifted.effects_mut().push(fx(id, json!({})));
            for (n, shape) in even_odd_shapes(vec![blue.clone(), lifted]).into_iter().zip(["compound path", "path"]) {
                let what = format!("{id} on the top {on} of a {shape}");
                let art = solid_art(&n).unwrap_or_else(|| panic!("{what}"));
                assert_flat_item_keeps_the_hole(&art, &blue, &what);
            }
        }
    }
}

#[test]
fn export_and_expand_bake_faces_with_their_own_ids() {
    for id in SOLIDS {
        let mut d = Document::new(400.0, 400.0);
        let mut n = square_node();
        n.id = d.alloc_id();
        n.appearance.effects.push(fx(id, json!({"bevel": "rounded"})));
        assert_eq!(crate::bake_appearance(None, &n), solid_art(&n), "{id}");
        let mut stroke = |_: &mut Document, _: &PathData, _: FillRule, _: &StrokeLayer| None;
        let art = crate::expand_leaf(&mut d, &n, &mut stroke).unwrap();
        let mut ids = std::collections::HashSet::new();
        fn visit(n: &Node, ids: &mut std::collections::HashSet<NodeId>) {
            assert!(ids.insert(n.id), "{:?} twice", n.id);
            assert!(!has_solid(n));
            for c in n.children().into_iter().flatten() {
                visit(c, ids);
            }
        }
        visit(&art, &mut ids);
        assert!(ids.len() > 2, "{id}");
    }
}

/// A twisted, tapered, hollow Inflate (both sides) exports and expands like the others: faces with
/// their own ids, no 3D effect left on them.
#[test]
fn a_twisted_tapered_hollow_inflate_exports_and_expands() {
    let mut d = Document::new(400.0, 400.0);
    let mut n = square_node();
    n.id = d.alloc_id();
    let params = json!({"twist": 120, "taper": 50, "cap": false, "bothSides": true, "rotationX": -20, "rotationY": 35});
    n.appearance.effects.push(fx(INFLATE, params));
    let art = solid_art(&n).unwrap();
    assert_eq!(crate::bake_appearance(None, &n), Some(art.clone()));
    assert!(faces(&art).len() > 20, "{} faces", faces(&art).len());
    let mut stroke = |_: &mut Document, _: &PathData, _: FillRule, _: &StrokeLayer| None;
    let expanded = crate::expand_leaf(&mut d, &n, &mut stroke).unwrap();
    let mut ids = std::collections::HashSet::new();
    fn visit(n: &Node, ids: &mut std::collections::HashSet<NodeId>) {
        assert!(ids.insert(n.id), "{:?} twice", n.id);
        assert!(!has_solid(n));
        for c in n.children().into_iter().flatten() {
            visit(c, ids);
        }
    }
    visit(&expanded, &mut ids);
    assert!(ids.len() > 20, "{} nodes", ids.len());
}

/// Files and scripts from before the Perspective angle give `perspective` as a 0–100 % strength:
/// each 3D effect draws it exactly as the angle that stands for it, an angle given too wins, and
/// the current parameters carry only the angle.
#[test]
fn an_earlier_perspective_strength_draws_as_its_angle() {
    let art = |n: &Node, id: &str| if id == crate::REVOLVE { crate::revolve_art(n) } else { solid_art(n) };
    for id in SOLIDS.iter().copied().chain([crate::REVOLVE]) {
        let view = |extra: Value| {
            let mut p = json!({"rotationX": -25, "rotationY": 35, "rotationZ": 0, "depth": 80});
            p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            p
        };
        let with = |params: Value| {
            let mut n = square_node();
            // As read from a file: the stored parameters, not a fresh effect's.
            n.appearance.effects.push(Effect { id: id.into(), params, visible: true });
            art(&n, id).unwrap().visual_bounds().unwrap()
        };
        let angle = vectorcraft_three_d::perspective_from_strength(0.6);
        let old = with(view(json!({"perspective": 60})));
        let new = with(view(json!({"perspectiveAngle": angle})));
        let none = with(view(json!({})));
        for (a, b) in [(old.x0, new.x0), (old.y0, new.y0), (old.x1, new.x1), (old.y1, new.y1)] {
            assert!((a - b).abs() < 1e-9, "{id}: {old:?} against {new:?}, none {none:?}");
        }
        assert!((old.width() - none.width()).abs() > 1.0 || (old.height() - none.height()).abs() > 1.0, "{id}: perspective shows");
        assert_eq!(with(view(json!({"perspective": 60, "perspectiveAngle": 0}))), none, "{id}: the angle wins");
        let merged = crate::merged_params(id, &view(json!({"perspective": 60})));
        assert!(merged.get("perspective").is_none() && (merged["perspectiveAngle"].as_f64().unwrap() - angle).abs() < 1e-12, "{id}");
    }
}

/// Effects from before Rotation and Height set the light by azimuth and elevation: each 3D effect
/// shades its faces exactly as with the Rotation, Height and side that stand for it.
#[test]
fn an_earlier_light_direction_shades_as_its_rotation_and_height() {
    fn colours(n: &Node, out: &mut Vec<[f32; 3]>) {
        if let Some(AppearanceItem::Fill(f)) = n.appearance.items.first() {
            if let Paint::Gradient(g) = &f.paint {
                out.extend(g.gradient.stops.iter().map(|s| s.color.to_rgb()));
            } else if let Some(c) = f.paint.color() {
                out.push(c.to_rgb());
            }
        }
        for c in n.children().into_iter().flatten() {
            colours(c, out);
        }
    }
    for id in SOLIDS.iter().copied().chain([crate::REVOLVE]) {
        for (azimuth, elevation) in [(-45.0, 45.0), (150.0, -30.0), (60.0, 80.0), (-120.0, 10.0)] {
            let with = |light: Value| {
                let mut p = json!({"rotationX": -25, "rotationY": 35, "rotationZ": 0, "depth": 60, "lightSoftness": 0});
                p.as_object_mut().unwrap().extend(light.as_object().unwrap().clone());
                let mut n = square_node();
                n.appearance.effects.push(Effect { id: id.into(), params: p, visible: true });
                let art = if id == crate::REVOLVE { crate::revolve_art(&n) } else { solid_art(&n) }.unwrap();
                let mut out = vec![];
                colours(&art, &mut out);
                out
            };
            let (r, h, behind) = vectorcraft_three_d::light_from_azimuth_elevation(azimuth, elevation);
            let old = with(json!({"lightAzimuth": azimuth, "lightElevation": elevation}));
            let new = with(json!({"lightRotation": r, "lightHeight": h, "lightBehind": behind}));
            assert!(!old.is_empty() && old.len() == new.len(), "{id}");
            for (a, b) in old.iter().zip(&new) {
                assert!(a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-6), "{id} {azimuth} {elevation}: {a:?} against {b:?}");
            }
        }
        let merged = crate::merged_params(id, &json!({"lightAzimuth": 90, "lightElevation": 0}));
        assert!(
            merged.get("lightAzimuth").is_none() && merged["lightHeight"].as_f64().unwrap().abs() < 1e-9 && merged["lightBehind"] == json!(false),
            "{id}: {merged}"
        );
    }
}

#[test]
fn effects_before_it_shape_the_solid_and_effects_after_it_bend_the_result() {
    let mut n = square_node();
    n.appearance.effects.push(fx("path.offsetPath", json!({"offset": 20})));
    n.appearance.effects.push(fx(EXTRUDE, json!({"rotationX": 0, "rotationY": 0, "rotationZ": 0, "perspectiveAngle": 0})));
    // Straight on, the extrusion shows its front: the offset square and its 4 pt stroke.
    let b = solid_art(&n).unwrap().visual_bounds().unwrap();
    assert!((b.width() - 144.0 - 0.75).abs() < 1e-6, "{b:?}");
    n.appearance.effects.push(fx("distort.twist", json!({"angle": 30})));
    let art = solid_art(&n).unwrap();
    assert!(art.appearance.effects.iter().any(|e| e.id == "distort.twist"), "applied to the result as a whole");
    assert!(art.appearance.effects.iter().all(|e| e.id != "path.offsetPath" && !is_three_d(&e.id)));
}

#[test]
fn parameters_are_validated_and_lengths_scale_with_the_object() {
    let n = square_node();
    assert!(validate_solid(&n, EXTRUDE, &json!({"bevel": "rounded", "depth": 80})).is_ok());
    assert!(validate_solid(&n, EXTRUDE, &json!({"bevel": "fancy"})).unwrap_err().contains("bevel must be"));
    let empty = Node::group(NodeId(2), vec![]);
    assert!(validate_solid(&empty, INFLATE, &json!({})).unwrap_err().contains("requires a group holding a shape"));
    for (id, key) in [(EXTRUDE, "depth"), (EXTRUDE, "bevelWidth"), (EXTRUDE, "bevelHeight"), (INFLATE, "depth")] {
        assert!(crate::is_length(id, key, false), "{id} {key}");
    }
    assert!(!crate::is_length(EXTRUDE, "twist", false));
    for id in SOLIDS {
        assert!(!crate::is_geometry(id) && is_three_d(id));
        assert!(crate::default_params(id).is_some());
    }
    // Options read from parameters, with the documented ranges' defaults.
    let o = extrude_options(&json!({"taper": 40, "bevel": "step", "bevelSpace": 30, "bevelRepeat": 99}));
    assert_eq!(o.taper, 0.4);
    let b = o.bevel.unwrap();
    assert_eq!((b.shape, b.space, b.repeat), (BevelShape::Step, 0.3, 32));
    assert!(extrude_options(&json!({})).bevel.is_none());
}

/// `bevel` custom draws the points of `bevelProfile` in both Extrude & Bevels, with the other bevel
/// settings; bad points, a custom bevel without points and a name that is not a string are refused.
#[test]
fn custom_bevels_draw_their_points_and_bad_points_are_refused() {
    let n = square_node();
    let notch = json!([[0, 0], [0.2, 0.6], [0.7, 0.4], [1, 1]]);
    let custom = json!({"bevel": "custom", "bevelName": "Notch", "bevelProfile": notch, "bevelWidth": 10, "bevelRepeat": 2, "bevelBothSides": true});
    let b = extrude_options(&custom).bevel.unwrap();
    assert!(matches!(b.shape, BevelShape::Custom(_)));
    assert_eq!((b.width, b.repeat, b.both_sides), (10.0, 2, true));
    let points: Vec<(f64, f64)> = match b.shape {
        BevelShape::Custom(p) => p.points().collect(),
        _ => vec![],
    };
    assert_eq!(points.len(), 4);
    assert!((points[2].0 - 0.7).abs() < 1e-4 && (points[2].1 - 0.4).abs() < 1e-4, "{points:?}");
    assert!(matches!(classic_extrude_options(&custom).bevel.map(|b| b.shape), Some(BevelShape::Custom(_))));
    for id in [EXTRUDE, CLASSIC_EXTRUDE] {
        let classic_params = json!({"bevel": "classic", "bevelHeight": 10});
        let custom_params = json!({"bevel": "custom", "bevelProfile": notch, "bevelHeight": 10, "bevelName": "Notch"});
        assert!(validate_solid(&n, id, &custom_params).is_ok(), "{id}");
        let art = |p: &Value| {
            let mut n = square_node();
            n.appearance.effects.push(fx(id, p.clone()));
            solid_art(&n).unwrap()
        };
        assert_ne!(art(&classic_params), art(&custom_params), "{id}");
        for (bad, err) in [
            (json!({"bevel": "custom"}), "bevel custom needs bevelProfile"),
            (json!({"bevel": "custom", "bevelProfile": [[0, 0]]}), "bevelProfile must be 2 to 64"),
            (json!({"bevel": "custom", "bevelProfile": [[0.1, 0], [1, 1]]}), "bevelProfile must be 2 to 64"),
            (json!({"bevel": "custom", "bevelProfile": [[0, 0], [2, 0.5], [1, 1]]}), "bevelProfile must be 2 to 64"),
            (json!({"bevel": "custom", "bevelProfile": [[0, 0], ["a", 0.5], [1, 1]]}), "bevelProfile must be 2 to 64"),
            (json!({"bevel": "custom", "bevelProfile": "0,0 1,1"}), "bevelProfile must be 2 to 64"),
            (json!({"bevel": "custom", "bevelProfile": notch, "bevelName": 3}), "bevelName must be a string"),
        ] {
            let e = validate_solid(&n, id, &bad).unwrap_err();
            assert!(e.contains(err), "{id} {bad}: {e}");
        }
    }
    // A loaded file whose points do not make a profile draws no bevel.
    assert!(extrude_options(&json!({"bevel": "custom", "bevelProfile": [[0, 0]]})).bevel.is_none());
    // Changing the type keeps the custom bevel where the new effect takes a bevel, and drops it
    // where it does not.
    let classic = three_d_params_as(EXTRUDE, &custom, CLASSIC_EXTRUDE);
    assert_eq!((&classic["bevel"], &classic[BEVEL_NAME], &classic[BEVEL_PROFILE]), (&json!("custom"), &json!("Notch"), &notch));
    assert!(validate_solid(&n, CLASSIC_EXTRUDE, &classic).is_ok());
    assert_eq!(three_d_params_as(CLASSIC_EXTRUDE, &classic, EXTRUDE)[BEVEL_PROFILE], notch);
    let inflate = three_d_params_as(EXTRUDE, &custom, INFLATE);
    assert!(inflate.get(BEVEL_PROFILE).is_none() && inflate.get(BEVEL_NAME).is_none());
}

#[test]
fn inflate_reads_twist_taper_and_cap() {
    let o = inflate_options(&json!({"twist": 90, "taper": 40, "cap": false}));
    assert_eq!((o.twist, o.taper, o.cap), (90.0, 0.4, false));
    let d = inflate_options(&json!({}));
    assert_eq!((d.twist, d.taper, d.cap), (0.0, 1.0, true));
    assert_eq!(crate::default_params(INFLATE).unwrap()["cap"], json!(true));
}

/// Inflate under the legacy white light keeps the polygon faces it had before the colored-light
/// batching (each face a path of its own, not a channel group) and its seam's 0.05–0.75 pt range;
/// its seams' joins are round, as every 3D seam's.
#[test]
fn legacy_white_inflate_keeps_polygon_faces_and_its_seam_range() {
    let mut n = square_node();
    n.appearance.effects.push(fx(INFLATE, json!({})));
    let art = solid_art(&n).unwrap();
    let original = faces(&art);
    assert!(original.len() > 1000);
    assert!(original.iter().any(|f| f.path_data().unwrap().subpaths.iter().any(|p| p.anchors.len() > 3)), "legacy cells remain polygons");
    for face in original {
        assert!(matches!(face.kind, NodeKind::Path { .. }));
        let AppearanceItem::Stroke(stroke) = &face.appearance.items[1] else { panic!("seam") };
        assert!((0.05..=0.75).contains(&stroke.width));
        // Round, as every seam is: a facet seen almost edge-on is a sliver, and a mitered seam
        // reached past the solid as a thorn.
        assert_eq!(stroke.join, vectorcraft_doc::LineJoin::Round);
    }
    // A tiny shape exercises the lower bound, which otherwise disappears at grazing angles.
    n.transform(Affine::scale(0.001), false);
    let art = solid_art(&n).unwrap();
    assert!(faces(&art).iter().any(|face| matches!(&face.appearance.items[1], AppearanceItem::Stroke(s) if s.width == 0.05)));
    // An explicit white light overrides a stale legacy color without switching shading paths.
    n.appearance.effects[0].params = json!({"lightColor":"#ff0000","lights":[{}]});
    let art = solid_art(&n).unwrap();
    assert!(faces(&art).iter().all(|face| matches!(face.kind, NodeKind::Path { .. })));
}

#[test]
fn colored_solid_inflate_batches_channels_and_clips_once_per_surface() {
    let mut n = square_node();
    n.appearance.effects.push(fx(INFLATE, json!({"lights":[{"color":"#ff0000"},{"color":"#0000ff"}]})));
    let art = solid_art(&n).unwrap();
    fn count(n: &Node) -> (usize, usize, usize) {
        let (mut groups, mut clips, mut paths) = match &n.kind {
            NodeKind::Group { clip, .. } => (1, usize::from(*clip), 0),
            NodeKind::Path { .. } => (0, 0, 1),
            _ => (0, 0, 0),
        };
        for child in n.children().into_iter().flatten() {
            let (g, c, p) = count(child);
            groups += g;
            clips += c;
            paths += p;
        }
        (groups, clips, paths)
    }
    let (groups, clips, paths) = count(&art);
    assert!(paths > 1000, "exercise a tessellated surface");
    // The fill's solid and the stroke's, each one composite with its own clip.
    let surfaces = art.children().unwrap().len();
    assert_eq!(surfaces, 2);
    assert_eq!(clips, surfaces);
    assert_eq!(groups, 1 + 6 * surfaces, "group count must not grow with triangle count");
}

/// `square_node` moved right by `dx`, with id `id` and fill `fill`.
fn square_at(id: u64, dx: f64, fill: Color) -> Node {
    let mut n = square_node();
    n.id = NodeId(id);
    n.transform(Affine::translate((dx, 0.0)), false);
    n.appearance.items = Appearance::basic(Paint::solid(fill), Paint::None, 0.0).items;
    n
}

#[test]
fn a_3d_effect_on_a_group_turns_every_member_about_one_shared_axis() {
    // Two squares side by side, 100 pt apart, in a group: 100–200 and 300–400 across.
    let group = |id: &str, params: Value| {
        let mut g = Node::group(NodeId(9), vec![Arc::new(square_at(1, 0.0, RED)), Arc::new(square_at(2, 200.0, BLUE))]);
        g.appearance.effects.push(fx(id, params));
        g
    };
    let turned = json!({"rotationX": 0, "rotationY": 60, "rotationZ": 0});
    for id in SOLIDS {
        let g = group(id, json!({}));
        assert!(validate_solid(&g, id, &json!({})).is_ok(), "{id}");
        let art = solid_art(&g).unwrap_or_else(|| panic!("{id}"));
        assert_eq!(art.id, NodeId(9), "{id}: the art is the group's");
        assert!(art.appearance.effects.iter().all(|e| !is_three_d(&e.id)), "{id}");
        // Both members draw, each in its own fill.
        let mut colours = vec![];
        art.walk(&mut |c| {
            if c.path_data().is_some() {
                colours.push(fill_rgb(c));
            }
        });
        assert!(colours.iter().any(|c| c[2] > c[0]) && colours.iter().any(|c| c[0] > c[2]), "{id}: red and blue faces");
    }
    // Turned 60° about y, the members turn together about the group's centre (x = 250): the
    // pair spans half its width; turned each about its own centre they would stay 100 pt apart.
    let b = solid_art(&group(ROTATE, turned.clone())).unwrap().visual_bounds().unwrap();
    assert!((b.center().x - 250.0).abs() < 1e-3, "{b:?}");
    assert!((b.width() - 300.0 * 0.5).abs() < 1.0, "{b:?}");
    let mut alone = square_at(2, 200.0, BLUE);
    alone.appearance.effects.push(fx(ROTATE, turned));
    let own = solid_art(&alone).unwrap().visual_bounds().unwrap();
    assert!((own.center().x - 350.0).abs() < 1e-3, "{own:?}");
    // Hidden members and guides are left out; a group holding what can't become a solid is
    // refused with what to do.
    let mut g = group(EXTRUDE, json!({}));
    let mut woven = Node::group(NodeId(5), vec![Arc::new(square_at(6, 0.0, RED))]);
    woven.kind = NodeKind::Intertwine { children: woven.children().unwrap().clone() };
    if let NodeKind::Group { children, .. } = &mut g.kind {
        children.push(Arc::new(woven));
    }
    let err = validate_solid(&g, EXTRUDE, &json!({})).unwrap_err();
    assert!(err.contains("release or expand its other objects"), "{err}");
    assert!(err.contains("type, groups, clipping groups, blends and symbol instances"), "{err}");
    assert!(solid_art(&g).is_none(), "drawn as the group it is");
}

/// The red square without its stroke, with Extrude & Bevel (Classic) and `params`.
fn classic(params: Value) -> Node {
    let mut n = square_node();
    n.appearance.items.retain(|i| matches!(i, AppearanceItem::Fill(_)));
    n.appearance.effects.push(fx(CLASSIC_EXTRUDE, params));
    n
}

fn classic_faces(params: Value) -> Vec<Node> {
    let art = solid_art(&classic(params)).unwrap();
    faces(&art).into_iter().cloned().collect()
}

fn close3(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-4)
}

#[test]
fn classic_surfaces_draw_edges_flat_paint_soft_light_and_a_highlight() {
    // Wireframe: the edges of every face, the hidden ones too, in the paint; no fills.
    let wire = classic_faces(json!({"surface": "wireframe"}));
    let plastic = classic_faces(json!({}));
    assert_eq!(plastic.len(), 3, "a box shows three faces");
    assert_eq!(wire.len(), 6, "and has six");
    for f in &wire {
        let [AppearanceItem::Stroke(s)] = &f.appearance.items[..] else { panic!("a stroke alone: {:?}", f.appearance.items) };
        assert_eq!(s.paint, Paint::solid(RED));
    }
    // No Shading: every face in the art's own paint, a CMYK color kept as it is.
    assert!(classic_faces(json!({"surface": "none"})).iter().all(|f| close3(fill_rgb(f), RED.to_rgb())));
    let cmyk = Paint::solid(Color::Cmyk { c: 0.1, m: 0.9, y: 0.8, k: 0.0 });
    let mut n = classic(json!({"surface": "none"}));
    if let Some(AppearanceItem::Fill(f)) = n.appearance.items.first_mut() {
        f.paint = cmyk.clone();
    }
    for f in faces(&solid_art(&n).unwrap()) {
        let AppearanceItem::Fill(fill) = &f.appearance.items[0] else { panic!("a fill first") };
        assert_eq!(fill.paint, cmyk);
    }
    // Diffuse: the red darkened toward black by how far each face turns from the light, so the
    // faces differ but keep their hue.
    let diffuse: Vec<[f32; 3]> = classic_faces(json!({"surface": "diffuse"})).iter().map(fill_rgb).collect();
    assert!(diffuse.iter().all(|[r, g, b]| *r <= 0.8 + 1e-6 && (g * 4.0 - r).abs() < 1e-4 && (b * 8.0 - r).abs() < 1e-4), "{diffuse:?}");
    assert!(diffuse.windows(2).any(|w| !close3(w[0], w[1])), "{diffuse:?}");
    // Plastic: the same faces, one of them lightened toward white by the highlight.
    let shiny: Vec<[f32; 3]> =
        classic_faces(json!({"highlightIntensity": 100, "highlightSize": 100, "lightHeight": 60})).iter().map(fill_rgb).collect();
    let soft: Vec<[f32; 3]> = classic_faces(json!({"surface": "diffuse", "lightHeight": 60})).iter().map(fill_rgb).collect();
    assert!(shiny.iter().zip(&soft).any(|(s, d)| s[1] > d[1] + 0.01), "{shiny:?} {soft:?}");
    assert!(shiny.iter().zip(&soft).all(|(s, d)| s[1] >= d[1] - 1e-4));
    // Without highlight, Plastic shades as Diffuse.
    let dull: Vec<[f32; 3]> = classic_faces(json!({"highlightIntensity": 0, "lightHeight": 60})).iter().map(fill_rgb).collect();
    assert!(dull.iter().zip(&soft).all(|(a, b)| close3(*a, *b)));
}

#[test]
fn classic_shading_color_blend_steps_and_lights() {
    let base = json!({"surface": "diffuse", "ambient": 20});
    let with = |extra: Value| {
        let mut p = base.clone();
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        classic_faces(p).iter().map(fill_rgb).collect::<Vec<_>>()
    };
    // A custom shading color darkens toward that color; None keeps the paint.
    assert!(with(json!({"shadingColor": "custom", "shadingCustom": "#0000ff"})).iter().any(|c| c[2] > c[0] / 8.0 + 0.01));
    assert!(with(json!({"shadingColor": "none"})).iter().all(|c| close3(*c, RED.to_rgb())));
    // One blend step: every face fully lit or fully in shadow.
    let stepped = with(json!({"blendSteps": 1, "ambient": 0}));
    assert!(stepped.iter().all(|c| close3(*c, RED.to_rgb()) || close3(*c, [0.0; 3])), "{stepped:?}");
    // No light: the ambient share of the red. Two lights add up.
    let dark = with(json!({"lightIntensity": 0, "blendSteps": 256}));
    assert!(dark.iter().all(|c| (c[0] - 0.8 * 0.2).abs() < 0.01), "{dark:?}");
    let one = with(json!({"lights": [{"intensity": 30}], "blendSteps": 256}));
    let two = with(json!({"lights": [{"intensity": 30}, {"intensity": 30}], "blendSteps": 256}));
    assert!(one.iter().zip(&two).all(|(a, b)| b[0] >= a[0] - 1e-4) && one.iter().zip(&two).any(|(a, b)| b[0] > a[0] + 0.01));
    // Draw Hidden Faces keeps the faces turned away, drawn first.
    assert_eq!(with(json!({"drawHiddenFaces": true})).len(), 6);
}

#[test]
fn classic_bevel_height_extent_and_cap_shape_the_solid() {
    let bounds = |p: Value| solid_art(&classic(p)).unwrap().visual_bounds().unwrap();
    let front = json!({"rotationX": 0, "rotationY": 0, "rotationZ": 0, "bevel": "classic", "bevelHeight": 10});
    let out = bounds(front.clone());
    let mut inside = front.clone();
    inside["bevelInside"] = json!(true);
    // Straight on, a bevel added around the shape widens it; one carved into it keeps its size.
    assert!(out.width() > 100.0 + 15.0, "{out:?}");
    let carved = bounds(inside);
    assert!((carved.width() - 100.0).abs() < 2.0, "{carved:?}");
    // Hollow: no caps, so the walls seen from inside show too.
    assert!(classic_faces(json!({"cap": false})).len() > classic_faces(json!({})).len());
    // The depth sets how far back the solid goes.
    let side = json!({"rotationX": 0, "rotationY": 90, "rotationZ": 0});
    let mut deep = side.clone();
    deep["depth"] = json!(120);
    assert!((bounds(deep).width() - 120.0).abs() < 1.0);
}

/// A gradient fill with a transparent stop is shaded opaque and masked by its own transparency, so
/// the shading does not cover the transparent part; an opaque gradient needs no mask.
#[test]
fn classic_shading_keeps_a_gradients_transparency() {
    use vectorcraft_doc::color::{Gradient, GradientKind, GradientPaint, GradientStop};
    for (end, masked) in [(0.25, true), (1.0, false)] {
        let mut stop = GradientStop::new(1.0, RED);
        stop.opacity = end;
        let gradient = GradientPaint::new(Gradient::new(GradientKind::Linear, vec![GradientStop::new(0.0, RED), stop]));
        let mut n = classic(json!({}));
        if let Some(AppearanceItem::Fill(f)) = n.appearance.items.first_mut() {
            f.paint = Paint::Gradient(Box::new(gradient));
        }
        let art = solid_art(&n).unwrap();
        let surface = &art.children().unwrap()[0];
        let mut has_mask = false;
        surface.walk(&mut |c| has_mask |= c.extras().mask.is_some());
        assert_eq!(has_mask, masked, "{end}");
        let mut opaque = true;
        surface.walk(&mut |c| {
            for item in &c.appearance.items {
                if let AppearanceItem::Fill(f) = item
                    && let Paint::Gradient(g) = &f.paint
                {
                    opaque &= g.gradient.stops.iter().all(|s| s.opacity == 1.0);
                }
            }
        });
        assert!(opaque, "{end}: the faces are drawn opaque");
    }
}

/// A group member in a gradient with transparent stops keeps the shared back-to-front order of
/// every member's faces: its faces go in runs that carry its opacity mask, and the other member's
/// faces are drawn between them where they are in front of some and behind others. Here a square
/// frame in the gradient holds a square in its hole: the frame's inner walls are behind the square
/// and its front cap is in front of the square's walls.
#[test]
fn a_member_in_a_transparent_gradient_keeps_the_shared_face_order() {
    use vectorcraft_doc::color::{Gradient, GradientKind, GradientPaint, GradientStop};
    let square = |b: &mut BezPath, x0: f64, x1: f64, clockwise: bool| {
        let corners = [(x0, x0), (x1, x0), (x1, x1), (x0, x1)];
        let mut points = corners.to_vec();
        if !clockwise {
            points.reverse();
        }
        b.move_to(points[0]);
        for p in &points[1..] {
            b.line_to(*p);
        }
        b.close_path();
    };
    let art = |id: &str, params: &Value, end_opacity: f32| {
        let mut end = GradientStop::new(1.0, Color::BLACK);
        end.opacity = end_opacity;
        let gradient = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, vec![GradientStop::new(0.0, RED), end]))));
        let mut b = BezPath::new();
        square(&mut b, 100.0, 300.0, true);
        square(&mut b, 150.0, 250.0, false);
        let frame = Node::path(NodeId(1), PathData::from_bezpath(&b), Appearance::basic(gradient, Paint::None, 0.0));
        let mut b = BezPath::new();
        square(&mut b, 185.0, 215.0, true);
        let inner = Node::path(NodeId(2), PathData::from_bezpath(&b), Appearance::basic(Paint::solid(BLUE), Paint::None, 0.0));
        let mut g = Node::group(NodeId(3), vec![Arc::new(frame), Arc::new(inner)]);
        g.appearance.effects.push(fx(id, params.clone()));
        solid_art(&g).unwrap_or_else(|| panic!("{id}"))
    };
    // The drawing order: `M` for a group with an opacity mask, `F` for a face outside one.
    fn visit(n: &Node, order: &mut String) {
        if n.extras().mask.is_some() {
            order.push('M');
            return;
        }
        if n.path_data().is_some() {
            order.push('F');
        }
        for c in n.children().into_iter().flatten() {
            visit(c, order);
        }
    }
    let order = |art: &Node| {
        let mut order = String::new();
        visit(art, &mut order);
        order
    };
    for (id, params) in [(CLASSIC_EXTRUDE, json!({})), (EXTRUDE, json!({"lights": [{"color": "#ff8080"}]}))] {
        assert!(!order(&art(id, &params, 1.0)).contains('M'), "{id}: an opaque gradient needs no mask");
        let masked = order(&art(id, &params, 0.5));
        let (first, last) = (masked.find('M').unwrap(), masked.rfind('M').unwrap());
        assert!(masked.get(first..last).is_some_and(|between| between.contains('F')), "{id}: faces between the member's runs: {masked}");
    }
}

#[test]
fn classic_parameters_are_checked() {
    let n = classic(json!({}));
    let check = |p: Value| validate_solid(&n, CLASSIC_EXTRUDE, &crate::merged_params(CLASSIC_EXTRUDE, &p));
    assert!(check(json!({})).is_ok());
    assert!(check(json!({"surface": "wireframe", "shadingColor": "custom", "shadingCustom": "#12aBcd", "blendSteps": 256, "bevel": "ogee"})).is_ok());
    for (p, says) in [
        (json!({"surface": "glossy"}), "`surface` must be one of wireframe, none, diffuse, plastic"),
        (json!({"shadingColor": "red"}), "`shadingColor` must be one of black, custom, none"),
        (json!({"shadingCustom": "blue"}), "`shadingCustom` must be a #rrggbb color"),
        (json!({"shadingCustom": "#fff"}), "`shadingCustom` must be a #rrggbb color"),
        (json!({"depth": 2001}), "`depth` must be between 0 and 2000 (got 2001)"),
        (json!({"bevelHeight": 0}), "`bevelHeight` must be between 1 and 100 (got 0)"),
        (json!({"ambient": -1}), "`ambient` must be between 0 and 100 (got -1)"),
        (json!({"blendSteps": 2.5}), "`blendSteps` must be a whole number between 1 and 256 (got 2.5)"),
        (json!({"blendSteps": 0}), "`blendSteps` must be a whole number between 1 and 256 (got 0)"),
        (json!({"highlightSize": "big"}), "`highlightSize` must be a number"),
        (json!({"cap": "yes"}), "`cap` must be true or false"),
        (json!({"drawHiddenFaces": 1}), "`drawHiddenFaces` must be true or false"),
        (json!({"bevel": "fancy"}), "bevel must be none or one of"),
        (json!({"twist": 10}), "unknown parameter `twist`"),
        (json!({"lights": []}), "lights must contain one to ten lights"),
    ] {
        let err = check(p.clone()).unwrap_err();
        assert!(err.starts_with(says), "{p}: {err}");
    }
}

/// The red square, stroked black, with Rotate (Classic) and `params`: its surfaces, the fill's
/// plane first, then the stroke's outline.
fn classic_rotate(params: Value) -> Vec<Node> {
    let mut n = square_node();
    n.appearance.effects.push(fx(CLASSIC_ROTATE, params));
    solid_art(&n).unwrap().children().unwrap().iter().map(|s| (**s).clone()).collect()
}

#[test]
fn classic_rotate_turns_every_fill_and_stroke_as_one_plane() {
    let front = json!({"rotationX": 0, "rotationY": 0, "rotationZ": 0});
    let surfaces = classic_rotate(front.clone());
    assert_eq!(surfaces.len(), 2, "the fill's plane and the stroke's outline");
    assert!(surfaces.iter().all(|s| s.children().unwrap().len() == 1), "each is one flat face");
    let fill = surfaces[0].visual_bounds().unwrap();
    assert!((fill.width() - 100.0).abs() < 1.0 && (fill.height() - 100.0).abs() < 1.0, "{fill:?}");
    // Turned 60° about the vertical axis, the square shows half its width and all its height,
    // about the center of its bounds.
    let turned = classic_rotate(json!({"rotationX": 0, "rotationY": 60, "rotationZ": 0}))[0].visual_bounds().unwrap();
    assert!((turned.width() - 50.0).abs() < 1.0 && (turned.height() - 100.0).abs() < 1.0, "{turned:?}");
    assert!((turned.center().x - 150.0).abs() < 1.0, "{turned:?}");
    // Turned past 90°, the back of the plane is drawn.
    let back = classic_rotate(json!({"rotationX": 0, "rotationY": 150, "rotationZ": 0}));
    assert!(back[0].visual_bounds().is_some_and(|b| (b.width() - 100.0 * 30f64.to_radians().cos()).abs() < 1.0));
}

#[test]
fn classic_rotate_shades_with_no_shading_or_diffuse() {
    let rgb = |p: Value| fill_rgb(&classic_rotate(p)[0].children().unwrap()[0]);
    // No Shading keeps the paint, the color itself kept.
    let mut n = square_node();
    n.appearance.effects.push(fx(CLASSIC_ROTATE, json!({"surface": "none"})));
    let art = solid_art(&n).unwrap();
    let AppearanceItem::Fill(f) = &faces(&art)[0].appearance.items[0] else { panic!("a fill first") };
    assert_eq!(f.paint, Paint::solid(RED));
    // Diffuse: straight on with the light in front, the face is fully lit; turned away from the
    // light, it is the red darkened toward black.
    let lit = json!({"rotationX": 0, "rotationY": 0, "rotationZ": 0, "lightHeight": 90});
    assert!(close3(rgb(lit), RED.to_rgb()));
    let [r, g, b] = rgb(json!({"rotationX": 0, "rotationY": 70, "rotationZ": 0, "lightHeight": 90}));
    assert!(r < 0.8 - 0.05 && (g * 4.0 - r).abs() < 1e-4 && (b * 8.0 - r).abs() < 1e-4, "{r} {g} {b}");
    // No light: the ambient share of the red. A custom shading color darkens toward that color;
    // None keeps the paint.
    let dark = rgb(json!({"lightIntensity": 0, "ambient": 20, "blendSteps": 256}));
    assert!((dark[0] - 0.8 * 0.2).abs() < 0.01, "{dark:?}");
    let blue = rgb(json!({"lightIntensity": 0, "ambient": 20, "shadingColor": "custom", "shadingCustom": "#0000ff"}));
    assert!(blue[2] > blue[0] / 8.0 + 0.01, "{blue:?}");
    assert!(close3(rgb(json!({"lightIntensity": 0, "shadingColor": "none"})), RED.to_rgb()));
}

#[test]
fn classic_rotate_parameters_are_checked() {
    let n = square_node();
    let check = |p: Value| validate_solid(&n, CLASSIC_ROTATE, &crate::merged_params(CLASSIC_ROTATE, &p));
    assert!(check(json!({})).is_ok());
    assert!(check(json!({"surface": "none", "shadingColor": "custom", "shadingCustom": "#12aBcd", "blendSteps": 1})).is_ok());
    for (p, says) in [
        (json!({"surface": "plastic"}), "`surface` must be one of none, diffuse"),
        (json!({"surface": "wireframe"}), "`surface` must be one of none, diffuse"),
        (json!({"surface": 2}), "`surface` must be one of"),
        (json!({"shadingColor": "red"}), "`shadingColor` must be one of black, custom, none"),
        (json!({"shadingCustom": "#fff"}), "`shadingCustom` must be a #rrggbb color"),
        (json!({"rotationX": 181}), "`rotationX` must be between -180 and 180 (got 181)"),
        (json!({"perspectiveAngle": 161}), "`perspectiveAngle` must be between 0 and 160 (got 161)"),
        (json!({"lightIntensity": 101}), "`lightIntensity` must be between 0 and 100 (got 101)"),
        (json!({"blendSteps": 0}), "`blendSteps` must be a whole number between 1 and 256 (got 0)"),
        (json!({"lightBehind": 1}), "`lightBehind` must be true or false"),
        (json!({"highlightSize": 50}), "unknown parameter `highlightSize`"),
        (json!({"drawHiddenFaces": true}), "unknown parameter `drawHiddenFaces`"),
        (json!({"shadow": true}), "unknown parameter `shadow`"),
        (json!({"lights": []}), "lights must contain one to ten lights"),
    ] {
        let err = check(p.clone()).unwrap_err();
        assert!(err.starts_with(says), "{p}: {err}");
    }
}

/// Under a white light and under colored ones, in Extrude & Bevel, Inflate, Rotate, Extrude &
/// Bevel (Classic, each of its shading branches) and Rotate (Classic, its No Shading and Diffuse
/// surfaces and a gradient), a face seen almost edge-on (a sharp sliver) has
/// its seam stroke rounded at the corners: every seam's joins are round, and none reaches past its
/// face more than half its width as it is outlined (a mitered seam would reach past the solid as a
/// thorn up to ten times that beyond the face's tip). Each case's view makes some face with a
/// corner a miter would spike from.
#[test]
fn a_faces_seam_stays_within_half_its_width_of_the_face() {
    use vectorcraft_doc::color::{Gradient, GradientKind, GradientPaint, GradientStop};
    let gradient = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(
        GradientKind::Linear,
        vec![GradientStop::new(0.0, RED), GradientStop::new(1.0, BLUE)],
    ))));
    // Does `path` have a convex corner sharper than 45° that a miter with `limit` would spike from
    // (one sharper than 2·asin(1/limit) is bevelled instead, so it can't thorn; a reflex corner's
    // miter points inward)? An open subpath's ends are no corners, and a closing anchor repeating
    // the first is one corner.
    fn acute(path: &PathData, limit: f64) -> bool {
        let floor = 2.0 * (1.0 / limit).asin();
        path.subpaths.iter().any(|sp| {
            let mut pts: Vec<Point> = sp.anchors.iter().map(|a| a.p).collect();
            pts.dedup_by(|a, b| a.distance(*b) < 1e-9);
            if sp.closed && pts.len() > 1 && pts.first().zip(pts.last()).is_some_and(|(a, b)| a.distance(*b) < 1e-9) {
                pts.pop();
            }
            let n = pts.len();
            let corners: Vec<usize> = if sp.closed { (0..n).collect() } else { (1..n.saturating_sub(1)).collect() };
            // Which way the subpath turns as a whole: a convex corner turns the same way.
            let area: f64 = (0..n).map(|k| pts[k].to_vec2().cross(pts[(k + 1) % n].to_vec2())).sum();
            n >= 3
                && corners.into_iter().any(|k| {
                    let (a, b, c) = (pts[(k + n - 1) % n], pts[k], pts[(k + 1) % n]);
                    let (u, v) = (a - b, c - b);
                    let convex = !sp.closed || (b - a).cross(c - b) * area > 0.0;
                    let angle = (u.dot(v) / (u.hypot() * v.hypot())).clamp(-1.0, 1.0).acos();
                    convex && angle > floor && angle < std::f64::consts::FRAC_PI_4
                })
        })
    }
    // Views that make slivers whose corners a miter would spike from, in every case.
    let views = [(-35.0, 25.0), (-85.0, 15.0), (60.0, 60.0)];
    let mut cases = vec![];
    for &(x, y) in &views {
        let view = json!({"rotationX": x, "rotationY": y, "rotationZ": 0, "depth": 60});
        // Rotate (Classic) takes no depth.
        let flat = json!({"rotationX": x, "rotationY": y, "rotationZ": 0});
        let with = |extra: Value| {
            let mut p = view.clone();
            p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            p
        };
        cases.push((EXTRUDE, x, y, with(json!({"taper": 47.5})), None));
        // A gradient under a white light: its paint and the shadow fill over it each have a seam.
        cases.push((EXTRUDE, x, y, with(json!({"taper": 47.5})), Some(gradient.clone())));
        cases.push((INFLATE, x, y, with(json!({})), None));
        // Under colored lights: one, two, and a gradient's paint.
        let one = json!({"taper": 47.5, "lights": [{"color": "#ff8080"}]});
        let two = json!({"taper": 47.5, "lights": [{"color": "#ff8080"}, {"color": "#8080ff", "rotation": -60}]});
        cases.push((EXTRUDE, x, y, with(one.clone()), None));
        cases.push((EXTRUDE, x, y, with(two.clone()), None));
        cases.push((EXTRUDE, x, y, with(one), Some(gradient.clone())));
        cases.push((INFLATE, x, y, with(json!({"lights": [{"color": "#ff8080"}, {"color": "#8080ff", "rotation": -60}]})), None));
        // Rotate turns the art flat: a thin triangle, some corner of which stays sharp enough at
        // every view; under a white light and a colored one.
        cases.push((ROTATE, x, y, with(json!({})), None));
        cases.push((ROTATE, x, y, with(json!({"lights": [{"color": "#ff8080"}]})), None));
        // Classic: shaded, unshaded (No Shading) and a gradient's paint kept under its shading
        // layers each make their seam their own way.
        let bevel = json!({"bevel": "classic", "bevelHeight": 1.5});
        cases.push((CLASSIC_EXTRUDE, x, y, with(bevel.clone()), None));
        cases.push((CLASSIC_EXTRUDE, x, y, with(json!({"bevel": "classic", "bevelHeight": 1.5, "surface": "none"})), None));
        cases.push((CLASSIC_EXTRUDE, x, y, with(bevel), Some(gradient.clone())));
        // Rotate (Classic) draws its plane through Classic's surfaces: No Shading and Diffuse.
        for surface in ["none", "diffuse"] {
            let mut p = flat.clone();
            p["surface"] = json!(surface);
            cases.push((CLASSIC_ROTATE, x, y, p, None));
        }
        let mut p = flat.clone();
        p["surface"] = json!("diffuse");
        cases.push((CLASSIC_ROTATE, x, y, p, Some(gradient.clone())));
    }
    // Classic's own paint for its fill: a swatch in CMYK, which its unshaded faces keep as it is.
    let brick = Paint::Solid { color: Color::Cmyk { c: 0.0, m: 0.75, y: 0.9, k: 0.2 }, swatch: Some("Brick".into()), tint: 1.0 };
    // Rotate (Classic)'s one plane is shaded at some views, not at every one (the light can leave
    // it as its paint), so its Diffuse case is held to a darkened face at one view at least.
    let mut rotate_shaded = false;
    for (id, x, y, params, paint) in cases {
        let mut n = if id == ROTATE || id == CLASSIC_ROTATE {
            let mut b = BezPath::new();
            b.move_to((100.0, 90.0));
            b.line_to((160.0, 110.0));
            b.line_to((100.0, 130.0));
            b.close_path();
            stroked(&b)
        } else {
            stroked_rect(4.0)
        };
        let unshaded = params["surface"] == "none";
        for (k, item) in n.appearance.items.iter_mut().enumerate() {
            match &paint {
                Some(paint) => *item.paint_mut() = paint.clone(),
                None if (id == CLASSIC_EXTRUDE || id == CLASSIC_ROTATE) && k == 0 => *item.paint_mut() = brick.clone(),
                None => {}
            }
        }
        // Each solid on its own (the fill's, then the stroke's), so every check holds for each.
        for keep in 0..n.appearance.items.len() {
            let mut solo = n.clone();
            let item = solo.appearance.items.remove(keep);
            solo.appearance.items = vec![item.clone()];
            let source = item.paint().clone();
            let what = format!("{id} {params} gradient {} view ({x}, {y}) solid {keep}", paint.is_some());
            solo.appearance.effects.push(fx(id, params.clone()));
            let art = solid_art(&solo).unwrap();
            let mut seamed = vec![];
            art.walk(&mut |face| {
                if let Some(path) = face.path_data()
                    && face.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Stroke(_)))
                {
                    seamed.push((path.clone(), face.appearance.items.clone()));
                }
            });
            // Each Classic case (Extrude & Bevel and Rotate) reaches its own branch: an unshaded face
            // keeps the paint itself (its swatch and color space too), a shaded one is the paint
            // darkened, and a gradient's face keeps the gradient.
            if id == CLASSIC_EXTRUDE || id == CLASSIC_ROTATE {
                let fills: Vec<&Paint> = seamed
                    .iter()
                    .filter_map(|(_, items)| match items.first() {
                        Some(AppearanceItem::Fill(f)) => Some(&f.paint),
                        _ => None,
                    })
                    .collect();
                assert!(!fills.is_empty(), "{what}: faces");
                if paint.is_some() || unshaded {
                    assert!(fills.iter().all(|f| **f == source), "{what}: every face keeps its paint as it is");
                } else {
                    let [r, g, b] = source.color().map(|c| c.to_rgb()).unwrap_or([0.0; 3]);
                    let darkened = fills.iter().any(|f| match f {
                        Paint::Solid { color, swatch: None, .. } => {
                            let [fr, fg, fb] = color.to_rgb();
                            fr <= r + 1e-4 && fg <= g + 1e-4 && fb <= b + 1e-4 && fr + fg + fb < r + g + b - 1e-3
                        }
                        _ => false,
                    });
                    if id == CLASSIC_ROTATE {
                        rotate_shaded |= darkened;
                    } else {
                        assert!(darkened, "{what}: some face shaded darker than its paint");
                    }
                }
            }
            // Every seamed fill (the paint's, a shadow's over it) has its seams checked, and on some
            // face a miter would spike from.
            let (mut checked, mut sliver): (usize, Vec<bool>) = (0, vec![]);
            for (path, items) in &seamed {
                let outline = path.to_bezpath();
                let acute_here = items.iter().find_map(|i| match i {
                    AppearanceItem::Stroke(s) => Some(acute(path, s.miter_limit)),
                    _ => None,
                });
                let mut which = 0;
                for item in items {
                    // A seam paints something (a stroke of no width or no paint draws nothing).
                    let AppearanceItem::Stroke(seam) = item else { continue };
                    if seam.width <= 0.0 || matches!(seam.paint, Paint::None) {
                        continue;
                    }
                    if sliver.len() <= which {
                        sliver.resize(which + 1, false);
                    }
                    sliver[which] |= acute_here.unwrap_or(false);
                    which += 1;
                    checked += 1;
                    assert_eq!(seam.join, vectorcraft_doc::LineJoin::Round, "{what}: a seam's joins");
                    // The seam outlined as it is drawn: no point of it farther from the face than
                    // half its width.
                    let drawn = crate::stroke::line_outline(&outline, seam, seam.width, seam.width * 1e-3);
                    let (mut farthest, mut points): (f64, usize) = (0.0, 0);
                    kurbo::flatten(drawn.iter(), seam.width * 1e-4, |el| {
                        if let kurbo::PathEl::MoveTo(p) | kurbo::PathEl::LineTo(p) = el {
                            points += 1;
                            farthest = farthest.max(path.nearest(p).map_or(f64::INFINITY, |n| n.4));
                        }
                    });
                    assert!(points > 0, "{what}: a seam outlined as nothing");
                    assert!(farthest <= seam.width / 2.0 * 1.01, "{what}: a seam {} wide reaches {farthest} from its face", seam.width);
                }
            }
            assert!(checked > 0, "{what}: seams checked");
            for (k, s) in sliver.iter().enumerate() {
                assert!(s, "{what}: no face with an acute corner carries seam {k}");
            }
        }
    }
    assert!(rotate_shaded, "Rotate (Classic), Diffuse: no view shades its plane darker than its paint");
}

/// Revolve (Classic) draws its faces through Classic's surfaces: on the plastic, diffuse and
/// unshaded surfaces, and with a gradient, every face's seam has round joins and reaches no more
/// than half its width past its face as it is outlined (the reach Revolve's overlap test counts on).
#[test]
fn classic_revolves_seams_are_round() {
    use vectorcraft_doc::color::{Gradient, GradientKind, GradientPaint, GradientStop};
    let gradient = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(
        GradientKind::Linear,
        vec![GradientStop::new(0.0, RED), GradientStop::new(1.0, BLUE)],
    ))));
    let mut b = BezPath::new();
    b.move_to((100.0, 40.0));
    b.line_to((130.0, 60.0));
    b.line_to((130.0, 140.0));
    b.line_to((100.0, 160.0));
    let cases = [("plastic", Paint::solid(BLUE)), ("diffuse", Paint::solid(BLUE)), ("none", Paint::solid(BLUE)), ("plastic", gradient)];
    for (surface, paint) in cases {
        let what = format!("{surface} gradient {}", matches!(paint, Paint::Gradient(_)));
        let mut n = Node::path(NodeId(1), PathData::from_bezpath(&b), Appearance::basic(paint, Paint::None, 0.0));
        n.appearance.effects.push(fx(crate::CLASSIC_REVOLVE, json!({"surface": surface})));
        let art = crate::revolve_art(&n).unwrap();
        let mut seams = 0;
        art.walk(&mut |face| {
            let Some(path) = face.path_data() else { return };
            for item in &face.appearance.items {
                let AppearanceItem::Stroke(s) = item else { continue };
                if s.width <= 0.0 {
                    continue;
                }
                seams += 1;
                assert_eq!(s.join, vectorcraft_doc::LineJoin::Round, "{what}: a seam's joins");
                let drawn = crate::stroke::line_outline(&path.to_bezpath(), s, s.width, s.width * 1e-3);
                let mut farthest: f64 = 0.0;
                kurbo::flatten(drawn.iter(), s.width * 1e-4, |el| {
                    if let kurbo::PathEl::MoveTo(p) | kurbo::PathEl::LineTo(p) = el {
                        farthest = farthest.max(path.nearest(p).map_or(f64::INFINITY, |n| n.4));
                    }
                });
                assert!(farthest <= s.width / 2.0 * 1.01, "{what}: a seam {} wide reaches {farthest} from its face", s.width);
            }
        });
        assert!(seams > 0, "{what}: seams checked");
    }
}
