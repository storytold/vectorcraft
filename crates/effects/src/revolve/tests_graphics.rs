use serde_json::json;
use vectorcraft_doc::Symbol;
use vectorcraft_doc::color::Color;
use vectorcraft_geom::{BezPath, shapes};

use super::*;
use crate::solids::three_d_params_as;

fn bar(r: Rect, color: Color) -> Node {
    Node::path(NodeId(0), shapes::rectangle(r), Appearance::basic(Paint::solid(color), Paint::None, 0.0))
}

/// A document with symbols "Red" and "Blue" (10 × 20 bars) and "Wide" (a red 300 × 10 bar).
fn document() -> Document {
    let mut d = Document::new(1000.0, 1000.0);
    for (name, r, color) in [
        ("Red", Rect::new(0.0, 0.0, 10.0, 20.0), Color::rgb(1.0, 0.0, 0.0)),
        ("Blue", Rect::new(0.0, 0.0, 10.0, 20.0), Color::rgb(0.0, 0.0, 1.0)),
        ("Wide", Rect::new(0.0, 0.0, 300.0, 10.0), Color::rgb(1.0, 0.0, 0.0)),
        ("Flat", Rect::new(0.0, 0.0, 10.0, 2.0), Color::rgb(1.0, 0.0, 0.0)),
    ] {
        d.symbols.push(Symbol { dynamic: false, name: name.into(), art: Arc::new(bar(r, color)) });
    }
    d
}

/// A gray open profile through `points` (its first and last on the axis at x 0), revolved with
/// `params` over a view straight on without shading.
fn revolved(points: &[(f64, f64)], params: Value) -> Node {
    let mut b = BezPath::new();
    b.move_to(points[0]);
    points[1..].iter().for_each(|p| b.line_to(*p));
    let mut p = json!({"rotationX": 0.0, "rotationY": 0.0, "rotationZ": 0.0, "shade": false});
    p.as_object_mut().unwrap().extend(params.as_object().unwrap().clone());
    let mut n = Node::path(NodeId(7), PathData::from_bezpath(&b), Appearance::basic(Paint::solid(Color::rgb(0.5, 0.5, 0.5)), Paint::None, 0.0));
    n.appearance.effects.push(crate::new_effect(REVOLVE, &p).unwrap());
    n
}

/// A cylinder of radius 50 about x 0, from y 0 to 200.
const CYLINDER: [(f64, f64); 4] = [(0.0, 0.0), (50.0, 0.0), (50.0, 200.0), (0.0, 200.0)];

/// A profile that narrows from 50 to 20 from the axis at y 100 and widens again.
const NECK: [(f64, f64); 5] = [(0.0, 0.0), (50.0, 0.0), (20.0, 100.0), (50.0, 200.0), (0.0, 200.0)];

/// The red channel of each color `paint` draws when it is a shade of pure red or pure blue (`blue`):
/// a solid color or the stops of a gradient. `None` for any other paint.
fn shade_of(paint: &Paint, blue: bool) -> Option<Vec<f32>> {
    let hue = |c: &Color| {
        let [r, g, b] = c.to_rgb();
        let (on, off) = if blue { (b, r) } else { (r, b) };
        (on > 0.02 && off < 0.01 && g < 0.01).then_some(on)
    };
    match paint {
        Paint::Solid { .. } => Some(vec![hue(&paint.color()?)?]),
        Paint::Gradient(g) => g.gradient.stops.iter().map(|s| hue(&s.color)).collect(),
        _ => None,
    }
}

/// The paths in `art` filled in a shade of red (or blue), with their shades.
fn parts(art: &Node, blue: bool) -> Vec<(PathData, Vec<f32>)> {
    let mut out = vec![];
    art.walk(&mut |n| {
        let Some(path) = n.path_data() else { return };
        let fill = n.appearance.items.iter().find_map(|i| match i {
            AppearanceItem::Fill(f) => shade_of(&f.paint, blue),
            AppearanceItem::Stroke(_) => None,
        });
        if let Some(shades) = fill {
            out.push((path.clone(), shades));
        }
    });
    out
}

fn reach(parts: &[(PathData, Vec<f32>)]) -> Rect {
    parts.iter().filter_map(|(p, _)| p.bounds()).reduce(|a, b| a.union(b)).unwrap()
}

fn near(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

#[test]
fn a_graphic_wraps_around_the_side_and_narrows_toward_the_silhouette() {
    let d = document();
    let n = revolved(&CYLINDER, json!({"graphics": [{"symbol": "Red"}, {"symbol": "Blue", "x": 60.0}]}));
    let art = revolve_art_in(Some(&d), &n).unwrap();
    // At the middle of the side, facing the viewer, the bar keeps its size: 10 wide at the axis's
    // line, from 10 above the middle of the profile to 10 below.
    let front = reach(&parts(&art, false));
    let half = 50.0 * (5.0f64 / 50.0).sin();
    assert!(near(front.x0, -half, 0.05) && near(front.x1, half, 0.05), "{front:?}");
    assert!(near(front.y0, 90.0, 1e-6) && near(front.y1, 110.0, 1e-6), "{front:?}");
    // 60 to its right the side has turned 1.1 to 1.3 radians away: the same bar draws less than
    // half as wide, at the sine of those angles.
    let turned = reach(&parts(&art, true));
    assert!(near(turned.x0, 50.0 * 1.1f64.sin(), 0.15) && near(turned.x1, 50.0 * 1.3f64.sin(), 0.15), "{turned:?}");
    assert!(turned.width() < front.width() / 2.0);
}

#[test]
fn the_part_past_the_silhouette_and_on_the_back_is_not_drawn() {
    let d = document();
    // A bar wider than the half of the side seen from the front reaches the silhouette and no
    // further.
    let n = revolved(&CYLINDER, json!({"graphics": [{"symbol": "Wide"}]}));
    let wide = reach(&parts(&revolve_art_in(Some(&d), &n).unwrap(), false));
    assert!(wide.x0 > -50.0 - 1e-6 && wide.x1 < 50.0 + 1e-6, "{wide:?}");
    assert!(wide.x0 < -49.5 && wide.x1 > 49.5, "{wide:?}");
    // On the back, half a turn around the axis, nothing is drawn.
    let n = revolved(&CYLINDER, json!({"graphics": [{"symbol": "Red", "x": 150.0}]}));
    assert!(parts(&revolve_art_in(Some(&d), &n).unwrap(), false).is_empty());
    // Turned half a turn, the back faces the viewer.
    let n = revolved(&CYLINDER, json!({"rotationY": 180.0, "graphics": [{"symbol": "Red", "x": 150.0}]}));
    assert!(!parts(&revolve_art_in(Some(&d), &n).unwrap(), false).is_empty());
}

#[test]
fn a_graphic_follows_the_profile_and_keeps_its_size_where_it_comes_nearer_the_axis() {
    let d = document();
    // A 10 wide bar keeps its length around the axis at every distance from it: at the neck, 20
    // from the axis, it spans 0.5 radians; half way up the slanted side (52.2 along it, 35 from
    // the axis, at y 50), 2/7 of a radian. Straight on, both draw nearly 10 wide.
    let slant = (30.0f64 * 30.0 + 100.0 * 100.0).sqrt();
    let n = revolved(&NECK, json!({"graphics": [{"symbol": "Flat"}, {"symbol": "Flat", "y": -slant / 2.0}]}));
    let art = revolve_art_in(Some(&d), &n).unwrap();
    let all = parts(&art, false);
    let neck: Vec<_> = all.iter().filter(|(p, _)| p.bounds().unwrap().center().y > 75.0).cloned().collect();
    let up: Vec<_> = all.iter().filter(|(p, _)| p.bounds().unwrap().center().y <= 75.0).cloned().collect();
    let (neck, up) = (reach(&neck), reach(&up));
    // Each bar is 2 tall, so its ends are up to 0.3 farther from the axis than its middle.
    assert!(near(neck.width(), 2.0 * 20.0 * 0.25f64.sin(), 0.15), "{neck:?}");
    assert!(near(neck.center().y, 100.0, 0.05), "{neck:?}");
    assert!(near(up.width(), 2.0 * 35.0 * (5.0f64 / 35.0).sin(), 0.15), "{up:?}");
    assert!(near(up.center().y, 50.0, 0.05), "{up:?}");
    // At the neck a bar 25 to the right lies 1 to 1.5 radians around, near the silhouette; one 40
    // to the right lies 1.75 to 2.25 radians around, past the silhouette, and is not drawn.
    let n = revolved(&NECK, json!({"graphics": [{"symbol": "Flat", "x": 25.0}]}));
    let side = reach(&parts(&revolve_art_in(Some(&d), &n).unwrap(), false));
    assert!(near(side.x0, 20.0 * 1.0f64.sin(), 0.3) && near(side.x1, 20.0 * 1.5f64.sin(), 0.3), "{side:?}");
    let n = revolved(&NECK, json!({"graphics": [{"symbol": "Flat", "x": 40.0}]}));
    assert!(parts(&revolve_art_in(Some(&d), &n).unwrap(), false).is_empty());
}

#[test]
fn with_the_axis_on_the_right_a_graphic_starts_on_the_front() {
    let d = document();
    // The cylinder of radius 50 about x 0, swept from its right edge.
    let profile = [(0.0, 0.0), (-50.0, 0.0), (-50.0, 200.0), (0.0, 200.0)];
    let n = revolved(&profile, json!({"edge": "right", "graphics": [{"symbol": "Red"}]}));
    let front = reach(&parts(&revolve_art_in(Some(&d), &n).unwrap(), false));
    let half = 50.0 * (5.0f64 / 50.0).sin();
    assert!(near(front.x0, -half, 0.05) && near(front.x1, half, 0.05), "{front:?}");
    assert!(near(front.y0, 90.0, 1e-6) && near(front.y1, 110.0, 1e-6), "{front:?}");
    // x increases to the viewer's right here too.
    let n = revolved(&profile, json!({"edge": "right", "graphics": [{"symbol": "Red", "x": 25.0}]}));
    let right = reach(&parts(&revolve_art_in(Some(&d), &n).unwrap(), false));
    assert!(near(right.center().x, 50.0 * 0.5f64.sin(), 0.3), "{right:?}");
}

#[test]
fn a_graphic_is_shaded_with_the_surface() {
    let d = document();
    // The light is on the left: the bar left of the middle is lighter than the one on its right,
    // and neither keeps its unlit red.
    let n = revolved(&CYLINDER, json!({"shade": true, "graphics": [{"symbol": "Red", "x": -30.0}, {"symbol": "Red", "x": 30.0}]}));
    let all = parts(&revolve_art_in(Some(&d), &n).unwrap(), false);
    let mean = |left: bool| {
        let shades: Vec<f32> =
            all.iter().filter(|(p, _)| (p.bounds().unwrap().center().x < 0.0) == left).flat_map(|(_, s)| s.iter().copied()).collect();
        shades.iter().sum::<f32>() / shades.len() as f32
    };
    assert!(mean(true) > mean(false) + 0.05, "{} {}", mean(true), mean(false));
    assert!(all.iter().all(|(_, s)| s.iter().all(|r| *r < 0.999)));
}

#[test]
fn overlapping_graphics_keep_their_order_on_every_face() {
    let d = document();
    // The wide red bar, listed second, reaches across the narrow blue one, listed first. Shaded,
    // each face is one triangle.
    let n = revolved(&CYLINDER, json!({"shade": true, "graphics": [{"symbol": "Blue", "x": 30.0}, {"symbol": "Wide"}]}));
    let art = revolve_art_in(Some(&d), &n).unwrap();
    let mut both = 0;
    art.walk(&mut |g| {
        // A face with parts of graphics: the face's path, then the parts' paths.
        let NodeKind::Group { children, clip: false } = &g.kind else { return };
        if !children.iter().all(|c| matches!(c.kind, NodeKind::Path { .. })) {
            return;
        }
        let colors: Vec<bool> = children
            .iter()
            .filter_map(|c| {
                let blue = !parts(c, true).is_empty();
                (blue || !parts(c, false).is_empty()).then_some(blue)
            })
            .collect();
        if colors.contains(&true) && colors.contains(&false) {
            both += 1;
            assert!(colors.iter().position(|b| !b) > colors.iter().position(|b| *b), "{colors:?}");
        }
    });
    assert!(both > 0);
}

#[test]
fn a_graphic_past_the_end_reaches_the_curved_rim_and_no_further() {
    let d = document();
    // A tube 50 from the axis with a cap at each end, in 10 segments, tilted to show its top: the
    // bottom rim is an arc on the page whose lowest point, in front, lies between two corners of
    // the faces. The wide bar runs 5 past the bottom of the profile.
    for shade in [true, false] {
        let mut b = BezPath::new();
        b.move_to((0.0, 0.0));
        b.line_to((0.0, 200.0));
        let p = json!({"offset": 50.0, "segments": 10, "rotationX": -30.0, "rotationY": 0.0, "rotationZ": 0.0, "shade": shade, "graphics": [{"symbol": "Wide", "y": 100.0}]});
        let mut n = Node::path(NodeId(7), PathData::from_bezpath(&b), Appearance::basic(Paint::solid(Color::rgb(0.5, 0.5, 0.5)), Paint::None, 0.0));
        n.appearance.effects.push(crate::new_effect(REVOLVE, &p).unwrap());
        let art = revolve_art_in(Some(&d), &n).unwrap();
        let mut lowest = f64::NEG_INFINITY;
        art.walk(&mut |c| {
            if let Some(b) = c.path_data().and_then(PathData::bounds) {
                lowest = lowest.max(b.y1);
            }
        });
        let bar = reach(&parts(&art, false));
        assert!(near(bar.y1, lowest, 0.05), "shade {shade}: {bar:?} {lowest}");
        // A thin bar wholly past the end, 0.35 to 0.85 below it in front, lies between the faces'
        // straight lower edge and the rim's arc there: it is drawn, inside the rim.
        let mut thin = n.clone();
        let p = json!({"offset": 50.0, "segments": 10, "rotationX": -30.0, "rotationY": 0.0, "rotationZ": 0.0, "shade": shade, "graphics": [{"symbol": "Flat", "y": 100.6, "scale": 25.0}]});
        thin.appearance.effects = vec![crate::new_effect(REVOLVE, &p).unwrap()];
        let past = parts(&revolve_art_in(Some(&d), &thin).unwrap(), false);
        assert!(!past.is_empty(), "shade {shade}");
        assert!(reach(&past).y1 <= lowest + 1e-6, "shade {shade}: {:?} {lowest}", reach(&past));
    }
}

#[test]
fn under_several_lights_a_graphic_takes_each_faces_own_lit_art() {
    let d = document();
    let lights = json!([{"color": "#ff8080", "rotation": 120.0}, {"color": "#8080ff", "rotation": -30.0}]);
    let clips = |n: &Node| {
        let mut count = 0;
        revolve_art_in(Some(&d), n).unwrap().walk(&mut |c| count += usize::from(matches!(c.kind, NodeKind::Group { clip: true, .. })));
        count
    };
    let plain = revolved(&CYLINDER, json!({"shade": true, "lights": lights}));
    let mapped = revolved(&CYLINDER, json!({"shade": true, "lights": lights, "graphics": [{"symbol": "Red"}]}));
    // Each part of the bar is the face's lit art clipped to the part: one clip more for the
    // part, one for the face's own art inside it.
    assert!(clips(&mapped) > clips(&plain), "{} {}", clips(&mapped), clips(&plain));
    // A lit face draws its paint under the face's light: a layer of the light's three channels,
    // multiplied over the paint. Every part of the bar takes such a layer of its own, so the bar
    // is shaded with each face's colored light; a bar drawn in flat red would add none.
    let lit = |n: &Node| {
        let mut count = 0;
        revolve_art_in(Some(&d), n).unwrap().walk(&mut |c| count += usize::from(c.blend == vectorcraft_doc::color::BlendMode::Multiply));
        count
    };
    assert!(lit(&mapped) > lit(&plain), "{} {}", lit(&mapped), lit(&plain));
    // An opaque part also draws its lit art clipped to the band of its seam stroke; a translucent
    // part, whose overlap would show, does not.
    let mut d = d.clone();
    let mut clear = bar(Rect::new(0.0, 0.0, 10.0, 20.0), Color::rgb(1.0, 0.0, 0.0));
    if let Some(AppearanceItem::Fill(f)) = clear.appearance.items.first_mut() {
        f.opacity = 0.5;
    }
    d.symbols.push(Symbol { dynamic: false, name: "Clear".into(), art: Arc::new(clear) });
    let clips = |n: &Node| {
        let mut count = 0;
        revolve_art_in(Some(&d), n).unwrap().walk(&mut |c| count += usize::from(matches!(c.kind, NodeKind::Group { clip: true, .. })));
        count
    };
    let translucent = revolved(&CYLINDER, json!({"shade": true, "lights": lights, "graphics": [{"symbol": "Clear"}]}));
    let (base, opaque, translucent) = (clips(&plain), clips(&mapped), clips(&translucent));
    assert!(translucent > base && opaque - base == 2 * (translucent - base), "{base} {opaque} {translucent}");
}

#[test]
fn graphics_past_the_point_limit_are_refused() {
    let mut d = document();
    let dots: Vec<Arc<Node>> = (0..2000)
        .map(|i| {
            let c = Point::new(f64::from(i % 50) * 12.0, f64::from(i / 50) * 12.0);
            Arc::new(Node::path(
                NodeId(0),
                shapes::ellipse(Rect::new(c.x, c.y, c.x + 10.0, c.y + 10.0)),
                Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
            ))
        })
        .collect();
    d.symbols.push(Symbol { dynamic: false, name: "Dots".into(), art: Arc::new(Node::group(NodeId(0), dots)) });
    let n = revolved(&CYLINDER, json!({}));
    let p = |scale: f64| json!({"graphics": [{"symbol": "Dots", "scale": scale}]});
    assert!(validate_revolve_in(Some(&d), &n, &p(100.0)).is_ok());
    // At 100 times the size each dot is cut into hundreds of straight pieces.
    let e = validate_revolve_in(Some(&d), &n, &p(10_000.0)).unwrap_err();
    assert!(e.contains("too complex"), "{e}");
}

#[test]
fn graphics_cut_into_too_many_parts_are_refused_on_the_object_and_on_a_fill() {
    let mut d = document();
    // 4,000 thin bars across the side: few points each, but each is cut at every face it crosses.
    let bars: Vec<Arc<Node>> =
        (0..4000).map(|i| Arc::new(bar(Rect::new(0.0, f64::from(i) * 0.05, 300.0, f64::from(i) * 0.05 + 0.02), Color::BLACK))).collect();
    d.symbols.push(Symbol { dynamic: false, name: "Bars".into(), art: Arc::new(Node::group(NodeId(0), bars)) });
    let p = json!({"segments": 128, "graphics": [{"symbol": "Bars"}]});
    let n = revolved(&CYLINDER, json!({}));
    let e = validate_revolve_in(Some(&d), &n, &p).unwrap_err();
    assert!(e.contains("too complex for this surface"), "{e}");
    assert!(validate_revolve_in(Some(&d), &n, &json!({"graphics": [{"symbol": "Red"}]})).is_ok());
    // The same on a Revolve on the object's fill.
    let mut filled = n.clone();
    let fx = filled.appearance.effects.remove(0);
    if let Some(AppearanceItem::Fill(f)) = filled.appearance.items.first_mut() {
        f.effects.push(fx);
    }
    let e = crate::solids::validate_three_d_at_in(Some(&d), &filled, REVOLVE, &p, (Some(0), 0)).unwrap_err();
    assert!(e.contains("too complex for this surface"), "{e}");
}

#[test]
fn without_the_document_nothing_is_drawn_and_expand_keeps_the_graphics() {
    let d = document();
    let n = revolved(&CYLINDER, json!({"graphics": [{"symbol": "Red"}]}));
    assert!(parts(&revolve_art(&n).unwrap(), false).is_empty());
    let shown = parts(&revolve_art_in(Some(&d), &n).unwrap(), false);
    let expanded = parts(&expand_revolve_art_in(Some(&d), &n).unwrap(), false);
    assert!(!shown.is_empty());
    assert_eq!(shown, expanded);
    // In a group the graphics go once, on the side of the member with the longest profile.
    let mut member = n.clone();
    member.appearance.effects.clear();
    let mut short = member.clone();
    if let NodeKind::Path { path, .. } = &mut short.kind {
        *path = PathData::from_bezpath(&{
            let mut b = BezPath::new();
            b.move_to((0.0, 80.0));
            b.line_to((30.0, 80.0));
            b.line_to((30.0, 120.0));
            b.line_to((0.0, 120.0));
            b
        });
    }
    let mut group = Node::group(NodeId(9), vec![Arc::new(short), Arc::new(member)]);
    group.appearance.effects = n.appearance.effects.clone();
    assert_eq!(parts(&revolve_art_in(Some(&d), &group).unwrap(), false).len(), shown.len());
}

#[test]
fn revolve_takes_graphics_on_its_side_only() {
    let front = json!({"graphics": [{"symbol": "Red"}]});
    let back = json!({"graphics": [{"symbol": "Red", "face": "back"}]});
    assert!(crate::solids::graphics::validate(REVOLVE, &front).is_ok());
    assert!(crate::solids::graphics::validate(REVOLVE, &back).unwrap_err().contains("side"));
    assert!(crate::solids::graphics::validate(CLASSIC_REVOLVE, &front).is_err());
    let n = revolved(&CYLINDER, json!({}));
    assert!(validate_revolve(&n, &back).is_err());
    // Changing the type keeps the front's graphics on Revolve's side and drops the back's.
    let both = json!({"graphics": [{"symbol": "Red"}, {"symbol": "Blue", "face": "back"}]});
    let kept = three_d_params_as(crate::EXTRUDE, &both, REVOLVE);
    assert_eq!(kept["graphics"].as_array().map(Vec::len), Some(1));
    assert_eq!(kept["graphics"][0]["symbol"], "Red");
    assert_eq!(three_d_params_as(REVOLVE, &front, crate::INFLATE).get("graphics"), None);
}

#[test]
fn revolve_classic_draws_no_graphics_its_parameters_carry() {
    let d = document();
    let mut n = revolved(&CYLINDER, json!({"graphics": [{"symbol": "Red"}]}));
    assert!(!parts(&revolve_art_in(Some(&d), &n).unwrap(), false).is_empty());
    // A file can hold graphics on Revolve (Classic), which commands refuse: Revolve (Classic)
    // draws its classic surface, without them.
    if let Some(fx) = n.appearance.effects.first_mut() {
        fx.id = CLASSIC_REVOLVE.into();
    }
    assert!(parts(&revolve_art_in(Some(&d), &n).unwrap(), false).is_empty());
}

#[test]
fn a_group_with_no_member_to_carry_graphics_still_checks_their_art() {
    let mut d = document();
    let dots: Vec<Arc<Node>> = (0..2000)
        .map(|i| {
            let c = Point::new(f64::from(i % 50) * 12.0, f64::from(i / 50) * 12.0);
            Arc::new(Node::path(
                NodeId(0),
                shapes::ellipse(Rect::new(c.x, c.y, c.x + 10.0, c.y + 10.0)),
                Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
            ))
        })
        .collect();
    d.symbols.push(Symbol { dynamic: false, name: "Dots".into(), art: Arc::new(Node::group(NodeId(0), dots)) });
    let n = revolved(&CYLINDER, json!({}));
    let mut member = n.clone();
    member.appearance.effects.clear();
    member.appearance.items.clear();
    let mut group = Node::group(NodeId(9), vec![Arc::new(member)]);
    group.appearance.effects = n.appearance.effects.clone();
    // No member has a fill or stroke, so none carries the graphics, but art past the limits is
    // refused as it is on a group whose member carries them.
    let e = validate_in(Some(&d), &group, REVOLVE, &json!({"graphics": [{"symbol": "Dots", "scale": 10_000.0}]})).unwrap_err();
    assert!(e.contains("too complex"), "{e}");
    assert!(validate_in(Some(&d), &group, REVOLVE, &json!({"graphics": [{"symbol": "Red"}]})).is_ok());
}

#[test]
fn a_full_turn_draws_a_graphic_on_either_side_of_the_cut_at_the_back() {
    let mut d = document();
    d.symbols.push(Symbol { dynamic: false, name: "Thin".into(), art: Arc::new(bar(Rect::new(0.0, 0.0, 2.0, 20.0), Color::rgb(1.0, 0.0, 0.0))) });
    // Half a turn around a radius of 50 is about 157 to each side of the middle: turned to face
    // the viewer, a graphic just inside either end is drawn.
    for x in [155.0, -155.0] {
        let n = revolved(&CYLINDER, json!({"rotationY": 180.0, "graphics": [{"symbol": "Thin", "x": x}]}));
        assert!(!parts(&revolve_art_in(Some(&d), &n).unwrap(), false).is_empty(), "x {x}");
    }
}

/// A gray cylinder of radius 50 whose side is 200 edges along the profile, from y 0 to 200.
fn ribbed() -> Vec<(f64, f64)> {
    let mut points = vec![(0.0, 0.0)];
    points.extend((0..=200).map(|k| (50.0, f64::from(k))));
    points.push((0.0, 200.0));
    points
}

/// A symbol `name` of one red path whose contours are `contours`, each a polygon through its
/// points.
fn contours_symbol(name: &str, contours: &[Vec<Point>]) -> Symbol {
    let mut b = BezPath::new();
    for contour in contours {
        b.move_to(contour[0]);
        contour[1..].iter().for_each(|p| b.line_to(*p));
        b.close_path();
    }
    let art = Node::path(NodeId(0), PathData::from_bezpath(&b), Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0));
    Symbol { dynamic: false, name: name.into(), art: Arc::new(art) }
}

#[test]
fn contours_away_from_a_face_are_not_cut_at_it() {
    // 2,000 small squares in two clusters on the back, at opposite corners of the side: the path's
    // bounds reach across every face, but no square reaches a face that is drawn.
    let squares: Vec<Vec<Point>> = (0..2000)
        .map(|i| {
            let (x, y) = if i < 1000 { (0.0, 0.0) } else { (290.0, 190.0) };
            let (x, y) = (x + f64::from(i % 10) * 0.1, y + f64::from(i % 1000 / 10) * 0.1);
            vec![Point::new(x, y), Point::new(x + 0.05, y), Point::new(x + 0.05, y + 0.05), Point::new(x, y + 0.05)]
        })
        .collect();
    let mut d = document();
    d.symbols.push(contours_symbol("Clusters", &squares));
    let n = revolved(&ribbed(), json!({}));
    let p = json!({"graphics": [{"symbol": "Clusters"}]});
    assert_eq!(validate_revolve_in(Some(&d), &n, &p), Ok(()));
}

#[test]
fn a_contour_that_reaches_every_face_and_draws_on_none_is_refused() {
    // One contour of 10,000 points: a thin frame around the whole side, past its ends and on the
    // back, so it is cut at every face and keeps nothing.
    let edge = |from: Point, to: Point| (0..2500).map(move |k| from.lerp(to, f64::from(k) / 2500.0));
    let (outer, inner) = ([(0.0, 0.0), (400.0, 0.0), (400.0, 400.0), (0.0, 400.0)], [(1.0, 1.0), (1.0, 399.0), (399.0, 399.0), (399.0, 1.0)]);
    let ring = |corners: [(f64, f64); 4]| (0..4).flat_map(move |k| edge(corners[k].into(), corners[(k + 1) % 4].into())).collect::<Vec<Point>>();
    let mut frame = ring(outer);
    frame.extend(ring(inner));
    let mut d = document();
    d.symbols.push(contours_symbol("Frame", &[frame]));
    let n = revolved(&ribbed(), json!({}));
    let e = validate_revolve_in(Some(&d), &n, &json!({"graphics": [{"symbol": "Frame"}]})).unwrap_err();
    assert!(e.contains("too complex for this surface"), "{e}");
    assert!(parts(&revolve_art_in(Some(&d), &n).unwrap(), false).is_empty());
}

/// A red linear gradient of `stops` stops, each with `opacity`.
fn ramp(stops: usize, opacity: f32) -> Paint {
    use vectorcraft_doc::color::{Gradient, GradientKind, GradientPaint, GradientStop};
    let stops = (0..stops)
        .map(|i| {
            let mut stop = GradientStop::new(i as f32 / stops as f32, Color::rgb(1.0, 0.0, 0.0));
            stop.opacity = opacity;
            stop
        })
        .collect();
    Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, stops))))
}

#[test]
fn the_copies_of_a_graphics_paint_have_a_budget_of_their_own() {
    let mut d = document();
    for (name, opacity) in [("Ramp", 1.0), ("Clear ramp", 0.5)] {
        let art =
            Node::path(NodeId(0), shapes::rectangle(Rect::new(0.0, 0.0, 300.0, 10.0)), Appearance::basic(ramp(4096, opacity), Paint::None, 0.0));
        d.symbols.push(Symbol { dynamic: false, name: name.into(), art: Arc::new(art) });
    }
    let p = json!({"graphics": [{"symbol": "Ramp"}]});
    let n = revolved(&CYLINDER, json!({}));
    let NodeKind::Path { path, .. } = &n.kind else { panic!("a path") };
    let faces = vectorcraft_three_d::revolve(path, path.bounds().unwrap(), options_of(REVOLVE, &p)).unwrap();
    let parts_of = |symbol: &str, copies: &mut usize| {
        let graphics = crate::solids::mapped_graphics(&json!({"graphics": [{"symbol": symbol}]}));
        crate::solids::graphics::surface_parts(Some(&d), NodeId(0), &graphics, &faces, &|_, c| Paint::solid(c), copies).map(|p| p.concat().len())
    };
    // Each opaque part copies the paint into its fill and its seam stroke; a part with
    // transparent stops takes no seam stroke and copies it once.
    let (opaque, clear) = (paint_bytes(&ramp(4096, 1.0)), paint_bytes(&ramp(4096, 0.5)));
    let mut used = 0;
    let count = parts_of("Ramp", &mut used).unwrap();
    assert!(count >= 2, "the graphic draws on several faces");
    assert_eq!(used, 2 * count * opaque);
    let mut used = 0;
    assert_eq!(parts_of("Clear ramp", &mut used), Some(count));
    assert_eq!(used, count * clear);
    // Past the budget the graphic is refused.
    assert_eq!(parts_of("Ramp", &mut (MAX_PAINT_COPIES - opaque)), None);
    // A surface whose own gradient takes nearly the whole budget for surfaces still takes the
    // graphic, on the object, on one fill of two and on a member of a group.
    let stop = paint_bytes(&ramp(2, 1.0)) - paint_bytes(&ramp(1, 1.0));
    let base = paint_bytes(&ramp(1, 1.0)) - stop;
    let gray = Paint::solid(Color::rgb(0.5, 0.5, 0.5));
    let full = ramp(((MAX_PAINT_COPIES - paint_copies(faces.len(), &gray)) / (faces.len() * 4) - base) / stop, 1.0);
    assert!(MAX_PAINT_COPIES - paint_copies(faces.len(), &gray) - paint_copies(faces.len(), &full) < opaque);
    let mut object = n.clone();
    if let Some(AppearanceItem::Fill(f)) = object.appearance.items.first_mut() {
        f.paint = ramp(((MAX_PAINT_COPIES - opaque) / (faces.len() * 4) - base) / stop, 1.0);
    }
    assert_eq!(validate_revolve_in(Some(&d), &object, &p), Ok(()));
    let fill = |paint: Paint, params: Value| {
        let mut fill = FillLayer::new(paint);
        fill.effects.push(crate::new_effect(REVOLVE, &params).unwrap());
        AppearanceItem::Fill(fill)
    };
    let mut two = n.clone();
    two.appearance = Appearance { items: vec![fill(gray.clone(), p.clone()), fill(full.clone(), json!({}))], ..Default::default() }.into();
    assert_eq!(crate::solids::validate_three_d_at_in(Some(&d), &two, REVOLVE, &json!({}), (Some(1), 0)), Ok(()));
    let mut member = n.clone();
    member.appearance.effects.clear();
    let mut second = member.clone();
    if let Some(AppearanceItem::Fill(f)) = second.appearance.items.first_mut() {
        f.paint = full;
    }
    let mut group = Node::group(NodeId(9), vec![Arc::new(member), Arc::new(second)]);
    group.appearance.effects = n.appearance.effects.clone();
    assert_eq!(validate_in(Some(&d), &group, REVOLVE, &p), Ok(()));
}

/// Whether each part of `art` filled with a gradient has a stroke over its seams.
fn gradient_seams(art: &Node) -> Vec<bool> {
    let mut out = vec![];
    art.walk(&mut |n| {
        let items = &n.appearance.items;
        let red = |p: &Paint| matches!(p, Paint::Gradient(_)) && shade_of(p, false).is_some();
        if n.path_data().is_some() && matches!(items.first(), Some(AppearanceItem::Fill(f)) if red(&f.paint)) {
            out.push(items.iter().any(|i| matches!(i, AppearanceItem::Stroke(_))));
        }
    });
    out
}

#[test]
fn a_gradient_with_transparent_stops_takes_no_stroke_over_its_seams() {
    let mut d = document();
    for (name, opacity) in [("Opaque", 1.0), ("Clear", 0.5)] {
        let art = Node::path(NodeId(0), shapes::rectangle(Rect::new(0.0, 0.0, 60.0, 10.0)), Appearance::basic(ramp(2, opacity), Paint::None, 0.0));
        d.symbols.push(Symbol { dynamic: false, name: name.into(), art: Arc::new(art) });
    }
    let seams = |symbol: &str| gradient_seams(&revolve_art_in(Some(&d), &revolved(&CYLINDER, json!({"graphics": [{"symbol": symbol}]}))).unwrap());
    let opaque = seams("Opaque");
    assert!(!opaque.is_empty() && opaque.iter().all(|s| *s), "{opaque:?}");
    let clear = seams("Clear");
    assert!(!clear.is_empty() && clear.iter().all(|s| !*s), "{clear:?}");
}
