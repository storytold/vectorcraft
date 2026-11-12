//! Revolve (Classic): the surface Revolve sweeps from a path, drawn with the classic surfaces of
//! Extrude & Bevel (Classic) (Wireframe, No Shading, Diffuse and Plastic), their highlight, blend
//! steps and shading color.
use serde_json::Value;
use vectorcraft_three_d::Revolve;

pub const CLASSIC_REVOLVE: &str = "threeD.classicRevolve";

/// The numeric parameters: (key, lowest, highest, whole numbers only).
const RANGES: [(&str, f64, f64, bool); 13] = [
    ("angle", 0.0, 360.0, false),
    ("offset", 0.0, 1000.0, false),
    ("rotationX", -180.0, 180.0, false),
    ("rotationY", -180.0, 180.0, false),
    ("rotationZ", -180.0, 180.0, false),
    ("perspectiveAngle", 0.0, 160.0, false),
    ("lightRotation", -180.0, 180.0, false),
    ("lightHeight", 0.0, 90.0, false),
    ("lightIntensity", 0.0, 100.0, false),
    ("ambient", 0.0, 100.0, false),
    ("highlightIntensity", 0.0, 100.0, false),
    ("highlightSize", 0.0, 100.0, false),
    ("blendSteps", 1.0, 256.0, true),
];

const FLAGS: [&str; 2] = ["cap", "lightBehind"];

/// The surface Revolve (Classic)'s parameters `p` sweep: Revolve's angle, cap, offset, edge,
/// rotation and perspective, built unshaded, since the classic surfaces light its faces.
pub(crate) fn options(p: &Value) -> Revolve {
    Revolve { shade: false, smooth_shading: false, lights: None, light_color: [1.0; 3], ..super::revolve_options(p) }
}

/// Check Revolve (Classic)'s parameters `p` (its defaults filled in) beyond the checks it shares
/// with Revolve: every key is one of its parameters, numbers are in range, flags are true or false,
/// and the surface and shading color are named.
pub(crate) fn validate(p: &Value) -> Result<(), String> {
    crate::solids::classic::validate_classic(CLASSIC_REVOLVE, p, &RANGES, &FLAGS)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_doc::color::{Color, Paint};
    use vectorcraft_doc::{Appearance, AppearanceItem, Node, NodeId};
    use vectorcraft_geom::{BezPath, PathData};

    use super::*;

    const BLUE: Color = Color::Rgb { r: 0.2, g: 0.4, b: 0.8 };

    /// Half a vase's profile, open, filled blue.
    fn profile() -> Node {
        let mut b = BezPath::new();
        b.move_to((100.0, 40.0));
        b.line_to((130.0, 60.0));
        b.line_to((130.0, 140.0));
        b.line_to((100.0, 160.0));
        Node::path(NodeId(1), PathData::from_bezpath(&b), Appearance::basic(Paint::solid(BLUE), Paint::None, 0.0))
    }

    fn revolved(params: Value) -> Node {
        let mut n = profile();
        n.appearance.effects.push(crate::new_effect(CLASSIC_REVOLVE, &params).unwrap());
        crate::revolve_art(&n).unwrap()
    }

    /// The faces of the surface of `art`, back to front.
    fn faces(art: &Node) -> Vec<&Node> {
        art.children().unwrap().iter().flat_map(|s| s.children().unwrap().iter().map(|f| &**f)).collect()
    }

    fn fill(face: &Node) -> Option<Color> {
        face.appearance.items.iter().find_map(|i| match i {
            AppearanceItem::Fill(f) => f.paint.color(),
            AppearanceItem::Stroke(_) => None,
        })
    }

    fn facets(params: &Value) -> usize {
        let n = profile();
        let path = n.path_data().unwrap();
        let p = crate::merged_params(CLASSIC_REVOLVE, params);
        vectorcraft_three_d::revolve_facets(path, path.bounds().unwrap(), options(&p)).unwrap().len()
    }

    #[test]
    fn plastic_shades_every_face_of_the_swept_surface_in_the_fill() {
        let art = revolved(json!({}));
        let drawn = faces(&art);
        assert_eq!(drawn.len(), facets(&json!({})), "one face for each facet of the surface");
        let colors: Vec<[f32; 3]> = drawn.iter().map(|f| fill(f).unwrap().to_rgb()).collect();
        // Lit from one side, the faces take more than one tone, and none is another hue: a face is
        // the blue darkened toward black, lightened toward white, or both.
        let distinct = colors.iter().filter(|c| colors.iter().any(|d| (c[0] - d[0]).abs() > 0.02)).count();
        assert!(distinct > 0, "{colors:?}");
        // Diffuse has no highlight, so every face is the blue darkened toward black.
        let art = revolved(json!({"surface": "diffuse"}));
        for face in faces(&art) {
            let [r, g, b] = fill(face).unwrap().to_rgb();
            assert!(b <= 0.8 + 1e-6 && (g * 2.0 - b).abs() < 1e-4 && (r * 4.0 - b).abs() < 1e-4, "{r} {g} {b}");
        }
    }

    #[test]
    fn wireframe_strokes_the_edges_of_every_face_and_no_shading_keeps_the_paint() {
        let art = revolved(json!({"surface": "wireframe"}));
        let faces_drawn = faces(&art);
        assert_eq!(faces_drawn.len(), facets(&json!({"surface": "wireframe"})));
        for face in faces_drawn {
            match face.appearance.items.as_slice() {
                [AppearanceItem::Stroke(s)] => assert_eq!((s.paint.color(), s.width), (Some(BLUE), 0.5)),
                items => panic!("a wireframe face is one stroke: {items:?}"),
            }
        }
        for face in faces(&revolved(json!({"surface": "none"}))) {
            assert_eq!(fill(face), Some(BLUE));
        }
    }

    #[test]
    fn angle_offset_edge_and_cap_shape_the_surface() {
        let full = revolved(json!({"rotationX": 0, "rotationY": 0, "rotationZ": 0})).visual_bounds().unwrap();
        // Seen from the front, the full turn is twice the profile's width across, about its left edge.
        assert!((full.width() - 60.0).abs() < 2.0, "{full:?}");
        let ring = revolved(json!({"rotationX": 0, "rotationY": 0, "rotationZ": 0, "offset": 20})).visual_bounds().unwrap();
        assert!((ring.width() - 100.0).abs() < 2.0, "{ring:?}");
        // About its right edge the axis is at x 130, so the surface reaches x 160.
        let right = revolved(json!({"rotationX": 0, "rotationY": 0, "rotationZ": 0, "edge": "right"})).visual_bounds().unwrap();
        assert!((right.x1 - 160.0).abs() < 2.0, "{right:?}");
        assert!(facets(&json!({"angle": 90})) < facets(&json!({})));
        // The profile's ends lie on the axis, so they close nothing until an offset moves them off it.
        assert!(facets(&json!({"cap": false, "offset": 10})) < facets(&json!({"offset": 10})), "hollow leaves out the end caps");
    }

    #[test]
    fn a_group_revolves_its_members_about_one_shared_axis() {
        let mut b = BezPath::new();
        b.move_to((150.0, 60.0));
        b.line_to((170.0, 60.0));
        b.line_to((170.0, 140.0));
        let outer = Node::path(NodeId(2), PathData::from_bezpath(&b), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
        let mut g = Node::group(NodeId(3), vec![std::sync::Arc::new(profile()), std::sync::Arc::new(outer)]);
        g.appearance.effects.push(crate::new_effect(CLASSIC_REVOLVE, &json!({"rotationX": 0, "rotationY": 0, "rotationZ": 0})).unwrap());
        let art = crate::revolve_art(&g).unwrap();
        // The axis is at the left edge of both members together, x 100, so the outer one reaches 70
        // out on either side of it.
        let bounds = art.visual_bounds().unwrap();
        assert!((bounds.x0 - 30.0).abs() < 2.0 && (bounds.x1 - 170.0).abs() < 2.0, "{bounds:?}");
        let mut colors = vec![];
        art.walk(&mut |n| colors.extend(fill(n).map(|c| c.to_rgb())));
        assert!(colors.iter().any(|c| c[2] > c[0]) && colors.iter().any(|c| c[0] == c[2]), "both members' paints are drawn");
    }

    /// A group member in a gradient with transparent stops keeps the shared back-to-front order of
    /// every member's faces: its faces go in runs that carry its opacity mask, and the other
    /// member's faces are drawn between them where they are in front of some and behind others.
    #[test]
    fn a_member_in_a_transparent_gradient_keeps_the_shared_face_order() {
        use vectorcraft_doc::color::{Gradient, GradientKind, GradientPaint, GradientStop};
        let art = |end_opacity: f32| {
            let mut end = GradientStop::new(1.0, Color::BLACK);
            end.opacity = end_opacity;
            let gradient = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, vec![GradientStop::new(0.0, BLUE), end]))));
            let mut b = BezPath::new();
            b.move_to((150.0, 60.0));
            b.line_to((170.0, 60.0));
            b.line_to((170.0, 140.0));
            let outer = Node::path(NodeId(2), PathData::from_bezpath(&b), Appearance::basic(gradient, Paint::None, 0.0));
            let mut g = Node::group(NodeId(3), vec![std::sync::Arc::new(outer), std::sync::Arc::new(profile())]);
            g.appearance.effects.push(crate::new_effect(CLASSIC_REVOLVE, &json!({})).unwrap());
            crate::revolve_art(&g).unwrap()
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
        assert!(!order(&art(1.0)).contains('M'), "an opaque gradient needs no mask");
        let masked = order(&art(0.5));
        let (first, last) = (masked.find('M').unwrap(), masked.rfind('M').unwrap());
        assert!(masked.get(first..last).is_some_and(|between| between.contains('F')), "faces between the member's runs: {masked}");
    }

    #[test]
    fn bad_parameters_are_refused() {
        let check = |p: Value| crate::validate_three_d(&profile(), CLASSIC_REVOLVE, &crate::merged_params(CLASSIC_REVOLVE, &p));
        assert_eq!(check(json!({})), Ok(()));
        for (p, error) in [
            (json!({"segments": 32}), "unknown parameter `segments`"),
            (json!({"angle": 400}), "`angle` must be between 0 and 360"),
            (json!({"offset": 1001}), "`offset` must be between 0 and 1000"),
            (json!({"rotationY": -181}), "`rotationY` must be between -180 and 180"),
            (json!({"blendSteps": 2.5}), "`blendSteps` must be a whole number"),
            (json!({"lightBehind": 1}), "`lightBehind` must be true or false"),
            (json!({"surface": "glossy"}), "`surface` must be one of"),
            (json!({"shadingColor": "white"}), "`shadingColor` must be one of"),
            (json!({"shadingCustom": "red"}), "`shadingCustom` must be a #rrggbb color"),
            (json!({"edge": "middle"}), "Revolve edge must be left or right"),
            (json!({"cap": "yes"}), "Revolve cap must be a boolean"),
        ] {
            let e = check(p.clone()).unwrap_err();
            assert!(e.contains(error), "{p}: {e}");
        }
    }
}
