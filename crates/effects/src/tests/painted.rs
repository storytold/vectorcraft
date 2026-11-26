//! Live-effect plug-ins that paint what they return, run on each fill a painted effect returned.

use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Effect, Node, NodeId, NodeKind};
use vectorcraft_geom::{Anchor, PathData, Point, Rect, SubPath, shapes};
use vectorcraft_plugins::wat as tpl;

use crate::geometry::Budget;
use crate::reshape;
use crate::reshape::{DRESSED, HELD_PEAK, PLUGIN_RUNS};

/// Installs live-effect plug-in `id`, which paints what it returns, replacing one with the same id:
/// its effect returns `output` whatever it is given.
fn install_painting(id: &str, output: &serde_json::Value) {
    let manifest = format!(r#"{{"id":"{id}","name":"Test {id}","version":"1","kind":"effect","api":2,"paintOutput":true,"params":{{}}}}"#);
    let module = wat::parse_str(tpl::returning(&manifest, &output.to_string())).unwrap();
    vectorcraft_plugins::registry::install_bytes(&module).unwrap();
}

/// An open path of `curves` tall curves, each drawn with about 5,200 lines.
fn tall_curves(curves: u32) -> PathData {
    let h = 3_000_000.0;
    let anchors = (0..=curves)
        .map(|i| {
            let x = 40.0 + f64::from(i);
            Anchor::with_handles(Point::new(x, 0.0), Point::new(x, -h), Point::new(x, h))
        })
        .collect();
    PathData::single(SubPath::new(anchors, false))
}

/// Installs live-effect plug-in `id`, which reshapes the path it is given: its effect returns
/// `output` whatever it is given.
fn install_reshaping(id: &str, output: &serde_json::Value) {
    let manifest = format!(r#"{{"id":"{id}","name":"Test {id}","version":"1","kind":"effect","params":{{}}}}"#);
    let module = wat::parse_str(tpl::returning(&manifest, &output.to_string())).unwrap();
    vectorcraft_plugins::registry::install_bytes(&module).unwrap();
}

/// `curves` curves from `x`, each `h` tall at its handles.
fn curves_at(x: f64, curves: u32, h: f64) -> SubPath {
    let anchors = (0..=curves)
        .map(|i| {
            let x = x + f64::from(i);
            Anchor::with_handles(Point::new(x, 0.0), Point::new(x, -h), Point::new(x, h))
        })
        .collect();
    SubPath::new(anchors, false)
}

/// An open path, `dx` to the right of 40, drawn with about `lines` lines: tall curves and one
/// curve that makes up the rest.
fn drawn_with(lines: usize, dx: f64) -> PathData {
    let h = 3_000_000.0;
    let per = Budget::of(&PathData::single(curves_at(0.0, 1, h))).lines;
    let tall = u32::try_from(lines / per).unwrap();
    let rest = lines % per;
    let mut subpaths = vec![curves_at(40.0 + dx, tall, h)];
    if rest > 0 {
        // A curve `h` tall at its handles is drawn with about √(9h) lines.
        let rest = rest as f64;
        subpaths.push(curves_at(40.0 + dx + f64::from(tall) + 2.0, 1, rest * rest / 9.0));
    }
    PathData::new(subpaths)
}

/// Stacks a painted effect that returns `first` on a path drawn with about `lines` lines with five
/// later effects on a square's fill, the `k`th returning that path `k` to the right (`later`
/// installs each). Each piece's share of the budget is counted once, so every effect fits, and the
/// art ends on the last effect's path, painted by `paints`.
fn later_effects_count_each_piece_once(name: &str, lines: usize, first: &serde_json::Value, later: fn(&str, &serde_json::Value), paints: &[Paint]) {
    let near = drawn_with(lines, 0.0);
    let drawn = Budget::of(&near).lines;
    assert!(drawn.abs_diff(lines) <= 2, "the path is drawn with {drawn} lines");
    let first_id = format!("test.effects.{name}.first");
    let mut first = first.clone();
    first["objects"][0]["path"] = serde_json::to_value(&near).unwrap();
    install_painting(&first_id, &first);
    let mut ids = vec![format!("plugin.{first_id}")];
    for k in 1..=5u8 {
        let id = format!("test.effects.{name}.later{k}");
        later(&id, &json!({"objects": [{"path": drawn_with(lines, f64::from(k))}]}));
        ids.push(format!("plugin.{id}"));
    }
    let mut n =
        Node::path(NodeId(9), shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    let fill = n.appearance.items.iter().position(|i| i.is_fill()).unwrap();
    for id in ids {
        n.appearance.items[fill].effects_mut().push(Effect { id, params: json!({}), visible: true });
    }
    let art = reshape(None, &n, None).unwrap();
    let mut paths = vec![];
    // The path is a compound path of two (the tall curves and the one that makes up the rest).
    art.walk(&mut |c| {
        let bounds = match &c.kind {
            NodeKind::Compound { children, .. } => children.iter().filter_map(|k| k.path_data().and_then(PathData::bounds)).reduce(|a, b| a.union(b)),
            _ => c.path_data().and_then(PathData::bounds),
        };
        let paints: Vec<Paint> = c.appearance.items.iter().map(|i| i.paint().clone()).filter(|p| !p.is_none()).collect();
        if bounds.is_some() && !paints.is_empty() {
            paths.push((bounds, paints));
        }
    });
    assert_eq!(paths, [(drawn_with(lines, 5.0).bounds(), paints.to_vec())]);
}

/// Later geometry effects on the two fills a painted effect returned run on each fill, and the
/// path they both stay on counts its piece's share of the budget once: each fill's result, drawn
/// with about 200 lines fewer than half the budget allows, fits at every stage.
#[test]
fn later_geometry_effects_on_two_fills_count_the_piece_once() {
    let colors = [Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::solid(Color::rgb(0.0, 1.0, 0.0))];
    let fills: Vec<_> = colors.iter().map(|p| serde_json::to_value(p).unwrap()).collect();
    let lines = Budget::EFFECTS.lines / 2 - 200;
    later_effects_count_each_piece_once("twofills", lines, &json!({"objects": [{"fills": fills}]}), install_reshaping, &colors);
}

/// Later painted effects on a painted effect's piece count the piece's share of the budget once:
/// each run's pieces take the place of the piece they were made of, so a result drawn with about
/// 300 lines fewer than the budget allows fits at every stage.
#[test]
fn later_painted_effects_on_a_painted_piece_count_it_once() {
    let lines = Budget::EFFECTS.lines - 300;
    later_effects_count_each_piece_once("painted", lines, &json!({"objects": [{}]}), install_painting, &[Paint::solid(Color::BLACK)]);
}

/// A path's bounds and the paints of its fills and strokes.
type PaintedPath = (Option<Rect>, Vec<Paint>);

/// The paths of the art a painted effect `name` that returns `square` with eight fills makes, each
/// path with its paints, when a later effect returns `first` on the first fill and `rest` on the
/// others, painting what it returns when `paints`; and the eight paints. Checks that the plug-in
/// results kept from one fill to the next never cost more than the effects' budget together.
fn eight_fills_then(name: &str, square: &PathData, paints: bool, results: (&PathData, &PathData)) -> (Vec<PaintedPath>, Vec<Paint>) {
    let (paths, colors, _) = fills_then(name, 8, square, paints, results);
    (paths, colors)
}

/// [`eight_fills_then`] with `n` fills, and how many times the later effect ran on a fill.
fn fills_then(name: &str, n: u8, square: &PathData, paints: bool, (first, rest): (&PathData, &PathData)) -> (Vec<PaintedPath>, Vec<Paint>, usize) {
    let colors: Vec<Paint> = (0..n).map(|i| Paint::solid(Color::rgb(f32::from(i) / f32::from(n), 0.0, 0.0))).collect();
    let fills: Vec<_> = colors.iter().map(|p| serde_json::to_value(p).unwrap()).collect();
    let fills_id = format!("test.effects.{name}.fills");
    install_painting(&fills_id, &json!({"objects": [{"path": square, "fills": fills}]}));
    let each_id = format!("test.effects.{name}.each");
    let declared = if paints { r#","api":2,"paintOutput":true"# } else { "" };
    let manifest = format!(r#"{{"id":"{each_id}","name":"Test {each_id}","version":"1","kind":"effect"{declared},"params":{{}}}}"#);
    let (on_first, on_rest) = (json!({"objects": [{"path": first}]}).to_string(), json!({"objects": [{"path": rest}]}).to_string());
    let module = wat::parse_str(tpl::when_given_else(&manifest, tpl::Given::Params, r#""item":{"fill":0}"#, &on_first, &on_rest)).unwrap();
    vectorcraft_plugins::registry::install_bytes(&module).unwrap();
    let mut n =
        Node::path(NodeId(9), shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    let fill = n.appearance.items.iter().position(|i| i.is_fill()).unwrap();
    for id in [fills_id, each_id] {
        n.appearance.items[fill].effects_mut().push(Effect { id: format!("plugin.{id}"), params: json!({}), visible: true });
    }
    HELD_PEAK.with(|p| p.set(Budget { anchors: 0, lines: 0 }));
    PLUGIN_RUNS.with(|r| r.set(0));
    let art = reshape(None, &n, None).unwrap();
    let runs = PLUGIN_RUNS.with(std::cell::Cell::get);
    let mut paths = vec![];
    art.walk(&mut |c| {
        if let Some(p) = c.path_data() {
            let paints: Vec<Paint> = c.appearance.items.iter().map(|i| i.paint().clone()).filter(|p| !p.is_none()).collect();
            if !paints.is_empty() {
                paths.push((p.bounds(), paints));
            }
        }
    });
    let peak = HELD_PEAK.with(std::cell::Cell::get);
    assert!(Budget::EFFECTS.fits(peak), "the results kept cost {peak:?}");
    (paths, colors, runs)
}

/// A later painted effect's results on eight fills, each costing more than half the effects'
/// budget and the first fill's on another path than the rest: only the first fill's result fits,
/// the other fills stay together on the square, and the results kept never cost more than the
/// budget together.
#[test]
fn painted_results_that_do_not_fit_are_not_kept() {
    let square = shapes::rectangle(Rect::new(40.0, 40.0, 60.0, 60.0));
    let (large, other) = (tall_curves(900), PathData::single(curves_at(3_000.0, 900, 3_000_000.0)));
    let (paths, colors) = eight_fills_then("held", &square, true, (&large, &other));
    assert_eq!(paths, [(large.bounds(), colors[..1].to_vec()), (square.bounds(), colors[1..].to_vec())]);
}

/// A later effect that reshapes each of eight painted fills to the same path, drawn with about a
/// fifth of the effects' budget: the fills share that one path, and the results that move onto it
/// are not kept as well, so what is kept never costs more than the budget.
#[test]
fn results_on_one_path_are_not_kept_for_each_fill() {
    let square = shapes::rectangle(Rect::new(40.0, 40.0, 60.0, 60.0));
    let same = tall_curves(300);
    let (paths, colors) = eight_fills_then("merged", &square, false, (&same, &same));
    assert_eq!(paths, [(same.bounds(), colors)]);
}

/// A later effect that reshapes each of eight painted fills to the same path, drawn with more than
/// half the effects' budget: the path costs the art once, so every fill moves onto it, although a
/// second copy of it would not fit.
#[test]
fn results_on_one_path_share_it_near_the_budget() {
    let square = shapes::rectangle(Rect::new(40.0, 40.0, 60.0, 60.0));
    let large = tall_curves(900);
    let (paths, colors) = eight_fills_then("shared", &square, false, (&large, &large));
    assert_eq!(paths, [(large.bounds(), colors)]);
}

/// An open path of tall curves `dx` to the right of 40, drawn with about `share` of the effects'
/// budget's lines.
fn drawn_share(share: f64, dx: f64) -> PathData {
    let h = 3_000_000.0;
    let per = Budget::of(&PathData::single(curves_at(0.0, 1, h))).lines as f64;
    let curves = (Budget::EFFECTS.lines as f64 * share / per) as u32;
    PathData::single(curves_at(40.0 + dx, curves, h))
}

/// A later painted effect runs once on each of three fills although the results are placed twice:
/// a result that fits both placings is kept between them, and one that did not fit in the first is
/// not run again when it does not fit the second either. The first fill's result (a tenth of the
/// budget) and the second's (half of it) fit, the third's (half of it) does not, and the square
/// (45%) left for the third fill then has no room; in the second placing the second fill's result
/// does not fit beside the square kept for the fills after it.
#[test]
fn a_later_plug_in_runs_once_on_each_fill_across_both_placings() {
    let (square, first, rest) = (drawn_share(0.45, 0.0), drawn_share(0.1, 2_000.0), drawn_share(0.5, 4_000.0));
    let (paths, colors, runs) = fills_then("once", 3, &square, true, (&first, &rest));
    assert_eq!(paths, [(first.bounds(), colors[..1].to_vec()), (square.bounds(), colors[1..].to_vec())]);
    assert_eq!(runs, 3, "the later effect ran {runs} times on three fills");
}

/// A built-in geometry effect on two strokes that a painted effect returned, which doubles the lines
/// of a path drawn with 30% of the budget: the second stroke's result does not fit beside the
/// first's, but it is the same path, so it is built again with all the room the art leaves and
/// both strokes are drawn on that one path.
#[test]
fn a_built_in_result_on_the_path_before_it_is_taken_near_the_budget() {
    let square = drawn_share(0.3, 0.0);
    let transform = json!({"scaleV": 400});
    let tall = crate::distort::transform_within(&square, square.bounds().unwrap(), &transform, 0, usize::MAX);
    let (s, t) = (Budget::of(&square).lines, Budget::of(&tall).lines);
    let lines = Budget::EFFECTS.lines;
    assert!(t > lines / 2 + lines / 20 && t < lines - lines / 20 && s + t < lines, "the paths are drawn with {s} and {t} lines");
    let colors = [Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::solid(Color::rgb(0.0, 1.0, 0.0))];
    let strokes: Vec<_> = colors.iter().map(|p| json!({"paint": p, "width": 1})).collect();
    let id = "test.effects.builtin.strokes";
    install_painting(id, &json!({"objects": [{"path": square, "strokes": strokes}]}));
    let mut n =
        Node::path(NodeId(9), shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)), Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0));
    let stroke = n.appearance.items.iter().position(|i| !i.is_fill()).unwrap();
    n.appearance.items[stroke].effects_mut().push(Effect { id: format!("plugin.{id}"), params: json!({}), visible: true });
    n.appearance.items[stroke].effects_mut().push(Effect { id: "distort.transform".into(), params: transform, visible: true });
    let art = reshape(None, &n, None).unwrap();
    let mut paths = vec![];
    art.walk(&mut |c| {
        if let Some(p) = c.path_data() {
            let paints: Vec<Paint> = c.appearance.items.iter().map(|i| i.paint().clone()).filter(|p| !p.is_none()).collect();
            if !paints.is_empty() {
                paths.push((p.bounds(), paints));
            }
        }
    });
    // One path, scaled about a reference point of the piece's own, painted by both strokes.
    assert_eq!(paths.len(), 1, "{paths:?}");
    let (bounds, paints) = &paths[0];
    assert_eq!(paints, &colors.to_vec());
    let height = |b: Option<Rect>| b.map_or(0.0, |b| b.height());
    assert!((height(*bounds) - height(tall.bounds())).abs() < 1.0, "{bounds:?} is not {:?} tall", tall.bounds());
}

/// The host takes a built-in effect with parameters it has, of their type and in range, on a piece,
/// and a raster effect on a fill or stroke; it refuses a plug-in effect, a 3D effect, a Pathfinder
/// effect (a piece is one path, with nothing to combine), an unknown effect or parameter, a value
/// of another type or out of range, and a geometry effect on a fill or stroke.
#[test]
fn returned_effects_refuse_nested_fields_as_top_level_parameters() {
    for key in ["active", "curve", "swatch"] {
        let mut params = serde_json::Map::new();
        params.insert(key.into(), json!(false));
        let effect = Effect { id: "addon.color.duotone".into(), params: params.into(), visible: true };
        assert!(crate::check_returned_effect(&effect, false).is_err(), "{key}");
    }
    let valid = Effect { id: "addon.color.duotone".into(), params: json!({"tones": []}), visible: true };
    assert_eq!(crate::check_returned_effect(&valid, false), Ok(()));
}

#[test]
fn returned_duotone_effects_validate_each_tone() {
    let check = |params| crate::check_returned_effect(&Effect { id: "addon.color.duotone".into(), params, visible: true }, false);
    let mut accepted = vec![];
    for tone in [
        json!(false),
        json!({"color": "not-a-color"}),
        json!({"curve": "not-a-curve"}),
        json!({"curve": [[0, 0], [100, 101]]}),
        json!({"curve": [[0, 0, 0]]}),
        json!({"active": "false"}),
        json!({"swatch": false}),
        json!({"unknown": 1}),
        json!({"curve": vec![[0, 0]; 65]}),
    ] {
        let params = json!({"tones": [tone]});
        if check(params.clone()).is_ok() {
            accepted.push(params);
        }
    }
    assert!(accepted.is_empty(), "accepted invalid tones: {accepted:?}");
    for params in [
        json!({"tones": []}),
        json!({"tones": [{}]}),
        json!({"tones": [{"color": "#123456", "curve": "0,0 100,100", "active": false, "swatch": "Ink"}]}),
        json!({"tones": [{"color": [0.1, 0.2, 0.3], "curve": [[0, 0], [100, 100]]}]}),
    ] {
        assert_eq!(check(params.clone()), Ok(()), "{params}");
    }
}

#[test]
fn returned_svg_filters_remain_separate_on_transform_copies() {
    let path = shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0));
    let xml = r##"<filter id="f" x="0" y="0" width="100%" height="100%"><feFlood flood-color="#ff0000"/></filter>"##;
    let filter = Effect { id: crate::SVG_FILTER.into(), params: json!({"xml": xml}), visible: true };
    assert_eq!(crate::check_returned_effect(&filter, false), Ok(()));
    let id = "test.effects.returned.svgcopies";
    let manifest = format!(r#"{{"id":"{id}","name":"SVG copies","kind":"effect","api":3,"paintOutput":true,"appearanceOutput":true,"params":{{}}}}"#);
    let output = json!({"objects": [{"path": path, "effects": [{"id": crate::SVG_FILTER, "params": filter.params}]}]});
    let module = wat::parse_str(tpl::returning(&manifest, &output.to_string())).unwrap();
    vectorcraft_plugins::registry::install_bytes(&module).unwrap();
    for row in [false, true] {
        let mut source = Node::path(NodeId(9), path.clone(), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
        let effects = vec![
            Effect { id: format!("plugin.{id}"), params: json!({}), visible: true },
            Effect { id: "distort.transform".into(), params: json!({"copies": 1, "moveH": 40.0}), visible: true },
        ];
        if row {
            source.appearance.items.first_mut().unwrap().effects_mut().extend(effects);
        } else {
            source.appearance.effects = effects;
        }
        let art = reshape(None, &source, None).unwrap();
        assert_eq!(vectorcraft_plugins::effect::last_error(id), None);
        let mut filtered = vec![];
        art.walk(&mut |node| {
            if node.appearance.effects.iter().any(|effect| effect.id == crate::SVG_FILTER) {
                filtered.push(node.geometric_bounds().unwrap());
            }
        });
        assert_eq!(filtered.len(), 2, "row={row}: {filtered:?}");
        assert!(filtered.iter().all(|bounds| bounds.width() == 20.0), "row={row}: {filtered:?}");
    }
    vectorcraft_plugins::registry::remove(id);
}

#[test]
fn row_transforms_include_the_dressed_pieces_enclosing_appearance() {
    let path = shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0));
    let id = "test.effects.returned.dressedgroup";
    let manifest =
        format!(r#"{{"id":"{id}","name":"Dressed group","kind":"effect","api":3,"paintOutput":true,"appearanceOutput":true,"params":{{}}}}"#);
    let output = json!({"objects": [{"path": path, "opacity": 0.5, "effects": [
        {"id": "path.offsetPath", "params": {"offset": 3.0}},
        {"id": "stylize.dropShadow", "params": {"x": 3.0, "y": 4.0}}
    ]}]});
    let module = wat::parse_str(tpl::returning(&manifest, &output.to_string())).unwrap();
    vectorcraft_plugins::registry::install_bytes(&module).unwrap();
    for stroke in [false, true] {
        for copies in [1, 0] {
            let appearance = if stroke {
                Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 2.0)
            } else {
                Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0)
            };
            let mut source = Node::path(NodeId(9), path.clone(), appearance);
            source.appearance.items.iter_mut().find(|row| row.is_fill() != stroke).unwrap().effects_mut().extend([
                Effect { id: format!("plugin.{id}"), params: json!({}), visible: true },
                Effect { id: "distort.transform".into(), params: json!({"copies": copies, "rotate": 90.0}), visible: true },
            ]);
            let art = reshape(None, &source, None).unwrap();
            assert_eq!(vectorcraft_plugins::effect::last_error(id), None);
            let mut opacities = 0;
            let mut shadows = vec![];
            art.walk(&mut |node| {
                opacities += usize::from(node.opacity == 0.5);
                shadows.extend(node.appearance.effects.iter().filter(|effect| effect.id == "stylize.dropShadow").map(|effect| effect.params.clone()));
            });
            assert_eq!(opacities, copies + 1, "stroke={stroke} copies={copies}");
            assert_eq!(shadows.len(), copies + 1);
            assert!(shadows.iter().any(|params| *params != json!({"x": 3.0, "y": 4.0})), "{shadows:?}");
        }
    }
    vectorcraft_plugins::registry::remove(id);
}

#[test]
fn copied_dressed_pieces_turn_the_source_rows_shadow() {
    let path = shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0));
    let id = "test.effects.returned.dressedrowshadow";
    let manifest =
        format!(r#"{{"id":"{id}","name":"Dressed row shadow","kind":"effect","api":3,"paintOutput":true,"appearanceOutput":true,"params":{{}}}}"#);
    let output = json!({"objects": [{"path": path, "opacity": 0.5, "effects": [{"id": "path.offsetPath", "params": {"offset": 3.0}}]}]});
    let module = wat::parse_str(tpl::returning(&manifest, &output.to_string())).unwrap();
    vectorcraft_plugins::registry::install_bytes(&module).unwrap();
    let mut source = Node::path(NodeId(9), path, Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    source.appearance.items.iter_mut().find(|row| row.is_fill()).unwrap().effects_mut().extend([
        Effect { id: "stylize.dropShadow".into(), params: json!({"x": 11.0, "y": 22.0}), visible: true },
        Effect { id: format!("plugin.{id}"), params: json!({}), visible: true },
        Effect { id: "distort.transform".into(), params: json!({"copies": 1, "rotate": 90.0}), visible: true },
    ]);
    let art = reshape(None, &source, None).unwrap();
    assert_eq!(vectorcraft_plugins::effect::last_error(id), None);
    let mut shadows = vec![];
    art.walk(&mut |node| {
        for item in &node.appearance.items {
            shadows.extend(item.effects().iter().filter(|effect| effect.id == "stylize.dropShadow").map(|effect| effect.params.clone()));
        }
    });
    assert_eq!(shadows.len(), 2);
    assert!(shadows.iter().any(|params| *params != json!({"x": 11.0, "y": 22.0})), "{shadows:?}");
    vectorcraft_plugins::registry::remove(id);
}

#[test]
fn returned_effects_are_built_in_effects_with_their_own_parameters() {
    let e = |id: &str, params: serde_json::Value| Effect { id: id.into(), params, visible: true };
    let check = crate::check_returned_effect;
    assert_eq!(check(&e("path.offsetPath", json!({"offset": 4.0})), false), Ok(()));
    assert_eq!(check(&e("path.offsetPath", serde_json::Value::Null), false), Ok(()));
    assert_eq!(check(&e("stylize.dropShadow", json!({"x": 2.0})), true), Ok(()));
    assert_eq!(check(&e("brushStrokes.accentedEdges", json!({"edgeWidth": 14.0})), false), Ok(()));
    assert_eq!(check(&e("distort.transform", json!({"copies": 1000})), false), Ok(()));
    // A parameter documented right after a comma has its range checked too.
    assert_eq!(check(&e("artistic.paintDaubs", json!({"sharpness": 25})), false), Ok(()));
    // Each documented choice and each end of a documented range passes.
    for (id, params) in [
        ("distort.roughen", json!({"points": "smooth"})),
        ("distort.roughen", json!({"points": "corner"})),
        ("distort.zigZag", json!({"points": "corner"})),
        ("distort.puckerBloat", json!({"amount": -200})),
        ("distort.puckerBloat", json!({"amount": 200})),
        ("adjust.temperatureTint", json!({"temperature": -100, "tint": 100})),
        ("stylize.dropShadow", json!({"mode": "color-burn"})),
        ("stylize.outerGlow", json!({"mode": "Normal"})),
        // A `color` is also taken as [r, g, b] 0..1 or a color object, as the effects draw it.
        ("stylize.dropShadow", json!({"color": [1.0, 0.0, 0.0]})),
        ("stylize.dropShadow", json!({"color": [1, 0, 0]})),
        ("stylize.outerGlow", json!({"color": {"model": "gray", "k": 0.5}})),
        ("stylize.innerGlow", json!({"color": {"model": "cmyk", "c": 0, "m": 1, "y": 0, "k": 0}})),
        ("stylize.innerGlow", json!({"color": {"model": "lab", "l": 100, "a": -128, "b": 127}})),
        // Curves' `points` is also taken as [[x, y], …], and a Transform's whole numbers as such.
        ("adjust.curves", json!({"points": [[0, 255], [255, 0]]})),
        ("adjust.curves", json!({"points": "0,0; 64,32  255,255"})),
        ("distort.transform", json!({"copies": 2, "reference": 0.0})),
    ] {
        assert_eq!(check(&e(id, params.clone()), false), Ok(()), "{id} {params}");
    }
    // Every effect's own defaults pass the check, so each documented range holds its default.
    for info in crate::effect_catalog().into_iter().filter(|i| !crate::is_three_d(i.id) && !crate::is_pathfinder(i.id)) {
        assert_eq!(check(&e(info.id, info.defaults.clone()), false), Ok(()), "{}", info.id);
    }
    for (effect, row) in [
        (e("distort.transform", json!({"copies": 1001})), false),
        (e("path.offsetPath", json!({"offset": null})), false),
        (e("plugin.test.effects.self", json!({})), false),
        (e("threeD.extrude", json!({})), false),
        (e("pathfinder.add", serde_json::Value::Null), false),
        (e("pathfinder.divide", json!({"precision": 1.0})), false),
        (e("stylize.nope", json!({})), false),
        (e("path.offsetPath", json!({"inset": 4.0})), false),
        (e("path.offsetPath", json!({"offset": "4"})), false),
        (e("brushStrokes.accentedEdges", json!({"edgeWidth": 15.0})), false),
        (e("artistic.paintDaubs", json!({"sharpness": 41})), false),
        (e("path.offsetPath", json!([4.0])), false),
        (e("path.offsetPath", json!({"offset": 4.0})), true),
    ] {
        assert!(check(&effect, row).is_err(), "{effect:?} on a fill or stroke: {row}");
    }
    let why = check(&e("pathfinder.add", serde_json::Value::Null), false).unwrap_err();
    assert!(why.contains("Pathfinder effects are not allowed"), "{why}");
    // The values checked for one effect are refused with the parameter named.
    for (id, params, key) in [
        (crate::SVG_FILTER, json!({"axes": [1, 0, 0, 0]}), "axes"),
        (crate::SVG_FILTER, json!({"scale": 0.0}), "scale"),
        (crate::SVG_FILTER, json!({"origin": [1.0]}), "origin"),
        (crate::SVG_FILTER, json!({"name": "f"}), "name"),
        ("warp.arc", json!({"version": 99}), "version"),
        ("stylize.dropShadow", json!({"color": "#zz0000"}), "color"),
        ("stylize.dropShadow", json!({"color": [1.0, "0", 0.0]}), "color"),
        ("stylize.innerGlow", json!({"color": {"model": "rgb"}}), "color"),
        ("stylize.outerGlow", json!({"color": true}), "color"),
        // Color components are from 0 to 1, Lab's lightness from 0 to 100.
        ("stylize.dropShadow", json!({"color": [2, -1, 0]}), "color"),
        ("stylize.dropShadow", json!({"color": [0.5, 0.5, 1.01]}), "color"),
        ("stylize.outerGlow", json!({"color": {"model": "rgb", "r": 2.0, "g": 0.0, "b": 0.0}}), "color"),
        ("stylize.innerGlow", json!({"color": {"model": "gray", "k": -0.5}}), "color"),
        ("stylize.innerGlow", json!({"color": {"model": "lab", "l": 101, "a": 0, "b": 0}}), "color"),
        ("adjust.curves", json!({"points": [[0, 255], [255]]}), "points"),
        ("adjust.curves", json!({"points": ["0,0", "255,255"]}), "points"),
        // Curves' points are read as "x,y" or [x, y] pairs, each number from 0 to 255.
        ("adjust.curves", json!({"points": "not-points"}), "points"),
        ("adjust.curves", json!({"points": "0,0 255,-1"}), "points"),
        ("adjust.curves", json!({"points": [[0, 300], [255, 0]]}), "points"),
        ("distort.transform", json!({"copies": 1.5}), "copies"),
        ("distort.transform", json!({"reference": 4.5}), "reference"),
        ("adjust.levels", json!({"channel": "cyan"}), "channel"),
        ("blur.radial", json!({"centerX": 1.5}), "centerX"),
        ("blur.gaussian", json!({"linear": 1}), "linear"),
        ("pixelate.colorHalftone", json!({"channel1": 400.0}), "channel1"),
        ("distort.twist", json!({"angle": 4000.0}), "angle"),
        ("distort.freeDistort", json!({"corners": []}), "corners"),
        ("distort.freeDistort", json!({"corners": [[0, 0], [1, 0], [1, 1], [0]]}), "corners"),
        ("distort.roughen", json!({"points": "invalid"}), "points"),
        ("distort.zigZag", json!({"points": "Corner"}), "points"),
        ("path.offsetPath", json!({"joins": "square"}), "joins"),
        (crate::RASTERIZE, json!({"colorModel": "cmyk"}), "colorModel"),
        (crate::CROP_MARKS, json!({"style": 1}), "style"),
        ("distort.puckerBloat", json!({"amount": -201}), "amount"),
        ("distort.puckerBloat", json!({"amount": 201}), "amount"),
        ("adjust.temperatureTint", json!({"temperature": 101}), "temperature"),
        ("adjust.temperatureTint", json!({"tint": -101}), "tint"),
        ("stylize.dropShadow", json!({"mode": "not-a-mode"}), "mode"),
        ("stylize.innerGlow", json!({"mode": "???"}), "mode"),
    ] {
        let why = check(&e(id, params.clone()), false).expect_err(&format!("{id} {params}"));
        assert!(why.contains(key), "{id} {params}: {why}");
    }
}

/// A painted effect on a fill whose piece fits the budget, but whose piece's own Transform copy
/// would not fit beside the source: the fill is drawn on the source's path, and nothing is drawn
/// where the piece would be.
#[test]
fn a_piece_s_own_geometry_effects_keep_to_the_budget() {
    let id = "test.effects.look.budget";
    let source = drawn_share(0.45, 0.0);
    let piece = json!({"path": drawn_share(0.3, 2_000.0), "effects": [{"id": "distort.transform", "params": {"copies": 1, "moveH": 10.0}}]});
    let manifest = format!(
        r#"{{"id":"{id}","name":"Test {id}","version":"1","kind":"effect","api":3,"paintOutput":true,"appearanceOutput":true,"params":{{}}}}"#
    );
    let module = wat::parse_str(tpl::returning(&manifest, &json!({ "objects": [piece] }).to_string())).unwrap();
    vectorcraft_plugins::registry::install_bytes(&module).unwrap();
    let mut n = Node::path(NodeId(9), source.clone(), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    let fill = n.appearance.items.iter().position(|i| i.is_fill()).unwrap();
    n.appearance.items[fill].effects_mut().push(Effect { id: format!("plugin.{id}"), params: json!({}), visible: true });
    let art = reshape(None, &n, None).unwrap();
    let mut bounds = vec![];
    art.walk(&mut |c| {
        if let Some(b) = c.path_data().and_then(PathData::bounds) {
            bounds.push(b);
        }
    });
    assert!(!bounds.is_empty() && bounds.iter().all(|b| b.x1 < 2_000.0), "{bounds:?}");
    vectorcraft_plugins::registry::remove(id);
}

/// Where [`many_heavy_pieces`] puts its effect.
#[derive(Clone, Copy, Debug)]
enum HeavyOn {
    /// The object's own effects.
    Object,
    /// The square's fill, as its only effect.
    Fill,
    /// The square's fill, after a painted effect that returns the square.
    LaterOnFill,
}

/// Ten painted pieces, each drawn with 4% of the effects' budget, each with a Transform effect that
/// makes 14 copies of it (60% of the budget per piece), from an effect on a square `on`: they would
/// pass the budget together, so the square is drawn as it was. The pieces' own geometry effects keep
/// to what the pieces before them left: the art reshaped for them costs less than the budget
/// together, and so does the art drawn.
fn many_heavy_pieces(on: HeavyOn) {
    let id = format!("test.effects.look.heavy{on:?}").to_lowercase();
    let pieces: Vec<_> = (0..10)
        .map(|k| json!({"path": drawn_share(0.04, 1_000.0 * f64::from(k)), "effects": [{"id": "distort.transform", "params": {"copies": 14, "moveH": 10.0}}]}))
        .collect();
    let manifest = format!(
        r#"{{"id":"{id}","name":"Test {id}","version":"1","kind":"effect","api":3,"paintOutput":true,"appearanceOutput":true,"params":{{}}}}"#
    );
    let module = wat::parse_str(tpl::returning(&manifest, &json!({ "objects": pieces }).to_string())).unwrap();
    vectorcraft_plugins::registry::install_bytes(&module).unwrap();
    let square = shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0));
    let first = format!("{id}.first");
    install_painting(&first, &json!({"objects": [{"path": square}]}));
    let mut n = Node::path(NodeId(9), square.clone(), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    let heavy = Effect { id: format!("plugin.{id}"), params: json!({}), visible: true };
    let fill = n.appearance.items.iter().position(|i| i.is_fill()).unwrap();
    match on {
        HeavyOn::Object => n.appearance.effects.push(heavy),
        HeavyOn::Fill => n.appearance.items[fill].effects_mut().push(heavy),
        HeavyOn::LaterOnFill => {
            n.appearance.items[fill].effects_mut().push(Effect { id: format!("plugin.{first}"), params: json!({}), visible: true });
            n.appearance.items[fill].effects_mut().push(heavy);
        }
    }
    DRESSED.with(|d| d.set(Budget { anchors: 0, lines: 0 }));
    let art = reshape(None, &n, None).unwrap();
    let dressed = DRESSED.with(std::cell::Cell::get);
    assert!(Budget::EFFECTS.fits(dressed), "{on:?}: the pieces' own effects made {dressed:?}");
    let (mut bounds, mut drawn) = (vec![], Budget { anchors: 0, lines: 0 });
    art.walk(&mut |c| {
        if let Some(p) = c.path_data() {
            bounds.push(p.bounds());
            drawn = drawn.plus(Budget::of(p));
        }
    });
    // On a fill, the fill and the stroke are paths of their own.
    assert!(!bounds.is_empty() && bounds.iter().all(|b| *b == square.bounds()), "{on:?}: {bounds:?}");
    assert!(Budget::EFFECTS.fits(drawn), "{on:?}: the art costs {drawn:?}");
    vectorcraft_plugins::registry::remove(&id);
    vectorcraft_plugins::registry::remove(&first);
}

/// [`many_heavy_pieces`] from the object's own effect.
#[test]
fn heavy_pieces_of_an_object_s_effect_stop_within_the_budget() {
    many_heavy_pieces(HeavyOn::Object);
}

/// [`many_heavy_pieces`] from an effect on a fill.
#[test]
fn heavy_pieces_of_a_fill_s_effect_stop_within_the_budget() {
    many_heavy_pieces(HeavyOn::Fill);
}

/// [`many_heavy_pieces`] from a later effect on a fill.
#[test]
fn heavy_pieces_of_a_later_effect_on_a_fill_stop_within_the_budget() {
    many_heavy_pieces(HeavyOn::LaterOnFill);
}

/// A piece's own geometry effect reshapes it before the source's later effects run, and those run
/// on it once: the square at 120, 40 grown by a 10 pt Offset Path, then moved 100 pt right by the
/// source's Transform effect. The piece's opacity and blending mode stay on the path it becomes.
#[test]
fn later_source_effects_run_after_generated_effects_once() {
    let id = "test.effects.look.offset";
    let piece = json!({"path": shapes::rectangle(Rect::new(120.0, 40.0, 140.0, 60.0)), "opacity": 0.5, "blend": "Multiply",
        "effects": [{"id": "path.offsetPath", "params": {"offset": 10.0}}]});
    let manifest = format!(
        r#"{{"id":"{id}","name":"Test {id}","version":"1","kind":"effect","api":3,"paintOutput":true,"appearanceOutput":true,"params":{{}}}}"#
    );
    let module = wat::parse_str(tpl::returning(&manifest, &json!({ "objects": [piece] }).to_string())).unwrap();
    vectorcraft_plugins::registry::install_bytes(&module).unwrap();
    let mut n =
        Node::path(NodeId(9), shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    n.appearance.effects.push(Effect { id: format!("plugin.{id}"), params: json!({}), visible: true });
    n.appearance.effects.push(Effect { id: "distort.transform".into(), params: json!({"moveH": 100.0}), visible: true });
    let art = reshape(None, &n, None).unwrap();
    let (mut paths, mut looks) = (vec![], vec![]);
    art.walk(&mut |c| {
        if let Some(p) = c.path_data() {
            paths.push(p.bounds());
        }
        if c.opacity != 1.0 || c.blend != vectorcraft_doc::color::BlendMode::Normal {
            looks.push((c.opacity, c.blend));
        }
    });
    assert_eq!(paths.len(), 1, "{paths:?}");
    // Once, on the piece and what holds it.
    assert_eq!(looks, [(0.5, vectorcraft_doc::color::BlendMode::Multiply)]);
    let b = paths[0].unwrap();
    assert!((b.x0 - 210.0).abs() < 0.5 && (b.x1 - 250.0).abs() < 0.5 && (b.y0 - 30.0).abs() < 0.5, "{b:?}");
    vectorcraft_plugins::registry::remove(id);
}

/// A piece's look reshapes it by its own geometry effects alone: the Transform effect of the fill
/// it carries stays on the fill for its own stage and does not move the piece now.
#[test]
fn a_piece_s_look_runs_only_its_own_geometry_effects() {
    let mut n =
        Node::path(NodeId(9), shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    let fill = n.appearance.items.iter().position(|i| i.is_fill()).unwrap();
    n.appearance.items[fill].effects_mut().push(Effect { id: "distort.transform".into(), params: json!({"moveH": 100.0}), visible: true });
    let look = vectorcraft_plugins::painted::Look {
        effects: vec![Effect { id: "path.offsetPath".into(), params: json!({"offset": 10.0}), visible: true }],
        ..Default::default()
    };
    crate::reshape::dress_piece(&mut n, &look, Budget::EFFECTS, None).unwrap();
    let mut paths = vec![];
    n.walk(&mut |c| {
        if let Some(b) = c.path_data().and_then(PathData::bounds) {
            paths.push((b, c.appearance.items.iter().map(|i| i.effects().len()).sum::<usize>()));
        }
    });
    assert_eq!(paths.len(), 1, "{paths:?}");
    let (b, kept) = paths[0];
    assert!((b.x0 + 10.0).abs() < 0.5 && (b.x1 - 30.0).abs() < 0.5, "{b:?}");
    assert_eq!(kept, 1, "the fill keeps its Transform effect");
}

/// A piece's blending mode on an object that blends in the same mode keeps both: the object
/// becomes a group in its mode around the piece in the piece's.
#[test]
fn a_piece_s_blending_mode_nests_in_the_object_s_same_mode() {
    use vectorcraft_doc::color::BlendMode;
    let mut n =
        Node::path(NodeId(9), shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    n.blend = BlendMode::Multiply;
    let look = vectorcraft_plugins::painted::Look { blend: Some(BlendMode::Multiply), ..Default::default() };
    crate::reshape::dress_piece(&mut n, &look, Budget::EFFECTS, None).unwrap();
    let mut blends = vec![];
    n.walk(&mut |c| blends.push(c.blend));
    assert_eq!(blends, [BlendMode::Multiply, BlendMode::Multiply]);
}

/// A piece with an opacity, a blending mode and a geometry effect draws them once: the group the
/// reshaped piece becomes carries them, and the path inside it draws at full opacity in Normal.
#[test]
fn a_reshaped_piece_s_opacity_and_blending_mode_apply_once() {
    use vectorcraft_doc::color::BlendMode;
    let mut n =
        Node::path(NodeId(9), shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    let look = vectorcraft_plugins::painted::Look {
        opacity: Some(0.5),
        blend: Some(BlendMode::Screen),
        effects: vec![Effect { id: "path.offsetPath".into(), params: json!({"offset": 10.0}), visible: true }],
    };
    crate::reshape::dress_piece(&mut n, &look, Budget::EFFECTS, None).unwrap();
    let (mut opacity, mut blends) = (1.0f32, vec![]);
    n.walk(&mut |c| {
        opacity *= c.opacity;
        if c.blend != BlendMode::Normal {
            blends.push(c.blend);
        }
    });
    assert!((opacity - 0.5).abs() < 1e-6, "{opacity}");
    assert_eq!(blends, [BlendMode::Screen]);
}

/// A piece's raster effect listed before its Transform effect follows the Transform, as an
/// object's does: a Drop Shadow before a scale of 200% keeps twice its offset. A hidden record
/// stays as it is.
#[test]
fn a_piece_s_raster_effects_follow_its_transform_effect() {
    let mut n =
        Node::path(NodeId(9), shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    let hidden = Effect { id: "path.offsetPath".into(), params: json!({"offset": 10.0}), visible: false };
    let look = vectorcraft_plugins::painted::Look {
        effects: vec![
            Effect { id: "stylize.dropShadow".into(), params: json!({"x": 3.0, "y": 4.0}), visible: true },
            hidden.clone(),
            Effect { id: "distort.transform".into(), params: json!({"scaleH": 200, "scaleV": 200}), visible: true },
        ],
        ..Default::default()
    };
    crate::reshape::dress_piece(&mut n, &look, Budget::EFFECTS, None).unwrap();
    let mut kept = vec![];
    n.walk(&mut |c| kept.extend(c.appearance.effects.iter().cloned()));
    let shadow = kept.iter().find(|e| e.id == "stylize.dropShadow").expect("the piece keeps its Drop Shadow");
    let at = |k: &str| shadow.params.get(k).and_then(serde_json::Value::as_f64).unwrap_or(f64::NAN);
    assert!((at("x") - 6.0).abs() < 1e-9 && (at("y") - 8.0).abs() < 1e-9, "{shadow:?}");
    assert_eq!(kept.iter().filter(|e| **e == hidden).count(), 1, "{kept:?}");
    assert!(kept.iter().all(|e| e.id != "distort.transform"), "{kept:?}");
}

/// A piece's Transform effect with no copies and Scale Strokes & Effects on moves the piece: no
/// copies were asked for, so none are missing.
#[test]
fn a_piece_s_transform_without_copies_scaling_strokes_moves_it() {
    let mut n =
        Node::path(NodeId(9), shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    let look = vectorcraft_plugins::painted::Look {
        effects: vec![Effect { id: "distort.transform".into(), params: json!({"moveH": 30.0, "copies": 0, "strokes": true}), visible: true }],
        ..Default::default()
    };
    crate::reshape::dress_piece(&mut n, &look, Budget::EFFECTS, None).expect("the piece is dressed");
    let mut paths = vec![];
    n.walk(&mut |c| paths.extend(c.path_data().and_then(PathData::bounds)));
    assert_eq!(paths.len(), 1, "{paths:?}");
    assert!((paths[0].x0 - 30.0).abs() < 0.5 && (paths[0].x1 - 50.0).abs() < 0.5, "{paths:?}");
}

#[test]
fn a_piece_s_copies_keep_separate_rotated_and_scaled_shadows() {
    for transform in [json!({"copies": 1, "moveH": 40.0, "rotate": 90.0}), json!({"copies": 1, "moveH": 40.0, "scaleH": 200, "scaleV": 200})] {
        let mut source = Node::path(
            NodeId(9),
            shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)),
            Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
        );
        let effects = vec![
            Effect { id: "stylize.dropShadow".into(), params: json!({"x": 3.0, "y": 4.0, "blur": 2.0}), visible: true },
            Effect { id: "distort.transform".into(), params: transform, visible: true },
        ];
        let mut ordinary = source.clone();
        ordinary.appearance.effects = effects.clone();
        let expected = reshape(None, &ordinary, None).unwrap();
        let shadows = |n: &Node| {
            let mut values = vec![];
            n.walk(&mut |c| {
                values.extend(c.appearance.effects.iter().filter(|e| e.id == "stylize.dropShadow").map(|e| e.params.to_string()));
            });
            values.sort();
            values
        };
        let expected = shadows(&expected);
        assert_eq!(expected.len(), 2);
        assert_ne!(expected[0], expected[1]);
        let look = vectorcraft_plugins::painted::Look { effects, ..Default::default() };
        crate::reshape::dress_piece(&mut source, &look, Budget::EFFECTS, None).unwrap();
        assert_eq!(shadows(&source), expected);
    }
}

#[test]
fn a_piece_s_copies_transform_raster_effects_on_its_fill() {
    for transform in [json!({"copies": 1, "moveH": 40.0, "rotate": 90.0}), json!({"copies": 1, "moveH": 40.0, "scaleH": 200, "scaleV": 200})] {
        let mut source = Node::path(
            NodeId(9),
            shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)),
            Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
        );
        source.appearance.items.iter_mut().find(|row| row.is_fill()).unwrap().effects_mut().push(Effect {
            id: "stylize.dropShadow".into(),
            params: json!({"x": 3.0, "y": 4.0, "blur": 2.0}),
            visible: true,
        });
        let effects = vec![Effect { id: "distort.transform".into(), params: transform, visible: true }];
        let mut ordinary = source.clone();
        ordinary.appearance.effects = effects.clone();
        let expected = reshape(None, &ordinary, None).unwrap();
        let shadows = |n: &Node| {
            let mut values = vec![];
            n.walk(&mut |c| {
                for row in &c.appearance.items {
                    values.extend(row.effects().iter().filter(|e| e.id == "stylize.dropShadow").map(|e| e.params.to_string()));
                }
                values.extend(c.appearance.effects.iter().filter(|e| e.id == "stylize.dropShadow").map(|e| e.params.to_string()));
            });
            values.sort();
            values
        };
        let expected = shadows(&expected);
        assert_eq!(expected.len(), 2);
        assert_ne!(expected[0], expected[1]);
        let look = vectorcraft_plugins::painted::Look { effects, ..Default::default() };
        crate::reshape::dress_piece(&mut source, &look, Budget::EFFECTS, None).unwrap();
        assert_eq!(shadows(&source), expected);
    }
}

#[test]
fn copied_piece_effects_share_the_remaining_geometry_budget() {
    let shadow = Effect { id: "stylize.dropShadow".into(), params: json!({"x": 3.0, "y": 4.0}), visible: true };
    let offset = Effect { id: "path.offsetPath".into(), params: json!({"offset": 2.0}), visible: true };
    let copies = Effect { id: "distort.transform".into(), params: json!({"copies": 2, "moveH": 40.0}), visible: true };
    for effects in [
        vec![offset.clone(), shadow.clone(), copies.clone()],
        vec![shadow.clone(), copies.clone(), offset],
        vec![shadow.clone(), copies.clone(), shadow, copies],
    ] {
        let source = Node::path(
            NodeId(9),
            shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)),
            Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
        );
        let look = vectorcraft_plugins::painted::Look { effects, ..Default::default() };
        let cost = crate::reshape::dress_piece(&mut source.clone(), &look, Budget::EFFECTS, None).unwrap();
        assert!(cost.anchors > 0);
        assert!(crate::reshape::dress_piece(&mut source.clone(), &look, cost, None).is_some());
        let short = Budget { anchors: cost.anchors - 1, lines: cost.lines };
        assert!(crate::reshape::dress_piece(&mut source.clone(), &look, short, None).is_none());
    }
}

#[test]
fn copied_pieces_reserve_their_shares_before_later_effects() {
    let path = shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0));
    let mut source = Node::path(NodeId(9), path.clone(), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    let look = vectorcraft_plugins::painted::Look {
        effects: vec![
            Effect { id: "stylize.dropShadow".into(), params: json!({"x": 3.0, "y": 4.0}), visible: true },
            Effect { id: "distort.transform".into(), params: json!({"copies": 1, "moveH": 40.0}), visible: true },
        ],
        ..Default::default()
    };
    assert!(crate::reshape::dress_piece(&mut source, &look, Budget::of(&path), None).is_none());
}

#[test]
fn returned_fill_shadows_follow_later_copying_transforms() {
    let path = shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0));
    let id = "test.effects.returned.fill.shadow";
    let manifest =
        format!(r#"{{"id":"{id}","name":"Returned shadows","kind":"effect","api":3,"paintOutput":true,"appearanceOutput":true,"params":{{}}}}"#);
    let output = json!({"objects": [{"path": path, "fills": [{"paint": Paint::solid(Color::BLACK), "effects": [{"id": "stylize.dropShadow", "params": {"x": 3.0, "y": 4.0, "blur": 2.0}}]}]}]});
    let module = wat::parse_str(tpl::returning(&manifest, &output.to_string())).unwrap();
    vectorcraft_plugins::registry::install_bytes(&module).unwrap();
    for transform in [json!({"copies": 1, "moveH": 40.0, "rotate": 90.0}), json!({"copies": 1, "moveH": 40.0, "scaleH": 200, "scaleV": 200})] {
        let mut source = Node::path(NodeId(9), path.clone(), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
        source.appearance.effects = vec![
            Effect { id: format!("plugin.{id}"), params: json!({}), visible: true },
            Effect { id: "distort.transform".into(), params: transform, visible: true },
        ];
        let art = reshape(None, &source, None).unwrap();
        assert_eq!(vectorcraft_plugins::effect::last_error(id), None);
        let mut shadows = vec![];
        art.walk(&mut |node| {
            for row in &node.appearance.items {
                shadows.extend(row.effects().iter().filter(|effect| effect.id == "stylize.dropShadow").map(|effect| effect.params.to_string()));
            }
            shadows.extend(node.appearance.effects.iter().filter(|effect| effect.id == "stylize.dropShadow").map(|effect| effect.params.to_string()));
        });
        assert_eq!(shadows.len(), 2, "{shadows:?}");
        assert_ne!(shadows[0], shadows[1]);
    }
}

#[test]
fn copied_piece_shadows_leave_later_fill_geometry_pending() {
    for transform in [json!({"copies": 1, "moveH": 40.0, "rotate": 90.0}), json!({"copies": 1, "moveH": 40.0, "scaleH": 200, "scaleV": 200})] {
        let mut source = Node::path(
            NodeId(9),
            shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)),
            Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
        );
        source.appearance.items.iter_mut().find(|item| item.is_fill()).unwrap().effects_mut().push(Effect {
            id: "path.offsetPath".into(),
            params: json!({"offset": 10.0}),
            visible: true,
        });
        let look = vectorcraft_plugins::painted::Look {
            effects: vec![
                Effect { id: "stylize.dropShadow".into(), params: json!({"x": 3.0, "y": 4.0}), visible: true },
                Effect { id: "distort.transform".into(), params: transform, visible: true },
            ],
            ..Default::default()
        };
        crate::reshape::dress_piece(&mut source, &look, Budget::EFFECTS, None).unwrap();
        let mut shadows = vec![];
        let mut offsets = vec![];
        source.walk(&mut |node| {
            shadows.extend(node.appearance.effects.iter().filter(|effect| effect.id == "stylize.dropShadow").map(|effect| effect.params.to_string()));
            for item in &node.appearance.items {
                offsets.extend(item.effects().iter().filter(|effect| effect.id == "path.offsetPath").map(|effect| effect.params.clone()));
            }
        });
        assert_eq!(shadows.len(), 2, "{shadows:?}");
        assert_ne!(shadows[0], shadows[1]);
        assert_eq!(offsets, [json!({"offset": 10.0}), json!({"offset": 10.0})]);
    }
}

#[test]
fn returned_piece_shadows_follow_later_row_copying_transforms() {
    let path = shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0));
    let id = "test.effects.returned.row.ownshadow";
    let manifest =
        format!(r#"{{"id":"{id}","name":"Returned row shadows","kind":"effect","api":3,"paintOutput":true,"appearanceOutput":true,"params":{{}}}}"#);
    let output = json!({"objects": [{"path": path, "effects": [{"id": "stylize.dropShadow", "params": {"x": 3.0, "y": 4.0}}]}]});
    let module = wat::parse_str(tpl::returning(&manifest, &output.to_string())).unwrap();
    vectorcraft_plugins::registry::install_bytes(&module).unwrap();
    for stroke in [false, true] {
        for transform in [
            json!({"copies": 1, "moveH": 40.0, "rotate": 90.0}),
            json!({"copies": 1, "moveH": 40.0, "scaleH": 200, "scaleV": 200}),
            json!({"copies": 0, "moveH": 40.0, "rotate": 90.0}),
            json!({"copies": 0, "moveH": 40.0, "scaleH": 200, "scaleV": 200}),
        ] {
            let copies = transform["copies"].as_u64().unwrap() as usize;
            let mut source = Node::path(NodeId(9), path.clone(), Appearance::basic(Paint::solid(Color::BLACK), Paint::solid(Color::BLACK), 2.0));
            let row = source.appearance.items.iter_mut().find(|item| item.is_fill() != stroke).unwrap();
            row.effects_mut().extend([
                Effect { id: format!("plugin.{id}"), params: json!({}), visible: true },
                Effect { id: "distort.transform".into(), params: transform, visible: true },
            ]);
            let art = reshape(None, &source, None).unwrap();
            assert_eq!(vectorcraft_plugins::effect::last_error(id), None);
            let mut shadows = vec![];
            art.walk(&mut |node| {
                shadows
                    .extend(node.appearance.effects.iter().filter(|effect| effect.id == "stylize.dropShadow").map(|effect| effect.params.to_string()))
            });
            assert_eq!(shadows.len(), copies + 1, "stroke={stroke}: {shadows:?}");
            if copies > 0 {
                assert_ne!(shadows[0], shadows[1], "stroke={stroke}");
            } else {
                assert_ne!(shadows[0], json!({"x": 3.0, "y": 4.0}).to_string(), "stroke={stroke}");
            }
        }
    }
    vectorcraft_plugins::registry::remove(id);
}

#[test]
fn returned_piece_shadows_follow_later_source_copying_transforms() {
    let path = shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0));
    let id = "test.effects.returned.own.shadow";
    let manifest =
        format!(r#"{{"id":"{id}","name":"Returned own shadows","kind":"effect","api":3,"paintOutput":true,"appearanceOutput":true,"params":{{}}}}"#);
    let output = json!({"objects": [{"path": path, "fills": [Paint::solid(Color::BLACK)], "effects": [{"id": "stylize.dropShadow", "params": {"x": 3.0, "y": 4.0}}]}]});
    let module = wat::parse_str(tpl::returning(&manifest, &output.to_string())).unwrap();
    vectorcraft_plugins::registry::install_bytes(&module).unwrap();
    for transform in [json!({"copies": 1, "moveH": 40.0, "rotate": 90.0}), json!({"copies": 1, "moveH": 40.0, "scaleH": 200, "scaleV": 200})] {
        let mut source = Node::path(NodeId(9), path.clone(), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
        source.appearance.effects = vec![
            Effect { id: format!("plugin.{id}"), params: json!({}), visible: true },
            Effect { id: "distort.transform".into(), params: transform, visible: true },
        ];
        let art = reshape(None, &source, None).unwrap();
        assert_eq!(vectorcraft_plugins::effect::last_error(id), None);
        let mut shadows = vec![];
        art.walk(&mut |node| {
            shadows.extend(node.appearance.effects.iter().filter(|effect| effect.id == "stylize.dropShadow").map(|effect| effect.params.to_string()));
        });
        assert_eq!(shadows.len(), 2, "{shadows:?}");
        assert_ne!(shadows[0], shadows[1]);
    }
}

#[test]
fn returned_piece_row_copies_charge_the_transform_once() {
    let path = drawn_share(0.3, 0.0);
    let cost = Budget::of(&path);
    assert!(Budget::EFFECTS.fits(cost.plus(cost)));
    let id = "test.effects.returned.copycharge";
    let manifest =
        format!(r#"{{"id":"{id}","name":"Copy charge","kind":"effect","api":3,"paintOutput":true,"appearanceOutput":true,"params":{{}}}}"#);
    let module = wat::parse_str(tpl::returning(&manifest, &json!({"objects": [{"path": path, "opacity": 0.5}]}).to_string())).unwrap();
    vectorcraft_plugins::registry::install_bytes(&module).unwrap();
    let mut source =
        Node::path(NodeId(9), shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    source.appearance.items.iter_mut().find(|row| row.is_fill()).unwrap().effects_mut().extend([
        Effect { id: format!("plugin.{id}"), params: json!({}), visible: true },
        Effect { id: "distort.transform".into(), params: json!({"copies": 1, "moveH": 40.0}), visible: true },
    ]);
    let art = reshape(None, &source, None).unwrap();
    assert_eq!(vectorcraft_plugins::effect::last_error(id), None);
    let mut paths = 0;
    art.walk(&mut |node| {
        if node.path_data().is_some() && node.appearance.items.iter().any(crate::paints) {
            paths += 1;
        }
    });
    assert_eq!(paths, 2);
    vectorcraft_plugins::registry::remove(id);
}

#[test]
fn a_lifted_row_shadow_follows_the_later_rotation() {
    let path = shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0));
    let id = "test.effects.returned.liftedrotation";
    let manifest =
        format!(r#"{{"id":"{id}","name":"Lifted rotation","kind":"effect","api":3,"paintOutput":true,"appearanceOutput":true,"params":{{}}}}"#);
    let output = json!({"objects": [{"path": path, "opacity": 0.5}]});
    let module = wat::parse_str(tpl::returning(&manifest, &output.to_string())).unwrap();
    vectorcraft_plugins::registry::install_bytes(&module).unwrap();
    let mut source = Node::path(NodeId(9), path, Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    source.appearance.items.iter_mut().find(|row| row.is_fill()).unwrap().effects_mut().extend([
        Effect { id: format!("plugin.{id}"), params: json!({}), visible: true },
        Effect { id: "distort.transform".into(), params: json!({"copies": 1, "moveH": 40.0}), visible: true },
        Effect { id: "stylize.dropShadow".into(), params: json!({"x": 3.0, "y": 4.0}), visible: true },
        Effect { id: "distort.transform".into(), params: json!({"rotate": 90.0}), visible: true },
    ]);
    let art = reshape(None, &source, None).unwrap();
    let mut shadows = vec![];
    art.walk(&mut |node| {
        shadows.extend(node.appearance.effects.iter().filter(|effect| effect.id == "stylize.dropShadow").map(|effect| effect.params.clone()));
    });
    assert_eq!(shadows.len(), 1);
    assert_ne!(shadows[0], json!({"x": 3.0, "y": 4.0}));
    vectorcraft_plugins::registry::remove(id);
}

#[test]
fn successive_row_copies_keep_later_shadows_in_order() {
    let path = shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0));
    let id = "test.effects.returned.liftedorder";
    let manifest =
        format!(r#"{{"id":"{id}","name":"Lifted order","kind":"effect","api":3,"paintOutput":true,"appearanceOutput":true,"params":{{}}}}"#);
    let output = json!({"objects": [{"path": path, "opacity": 0.5}]});
    let module = wat::parse_str(tpl::returning(&manifest, &output.to_string())).unwrap();
    vectorcraft_plugins::registry::install_bytes(&module).unwrap();
    let mut source = Node::path(NodeId(9), path, Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    source.appearance.items.iter_mut().find(|row| row.is_fill()).unwrap().effects_mut().extend([
        Effect { id: format!("plugin.{id}"), params: json!({}), visible: true },
        Effect { id: "distort.transform".into(), params: json!({"copies": 1, "moveH": 40.0}), visible: true },
        Effect { id: "stylize.dropShadow".into(), params: json!({"x": 3.0, "y": 4.0}), visible: true },
        Effect { id: "distort.transform".into(), params: json!({"copies": 1, "moveV": 40.0}), visible: true },
        Effect { id: "stylize.dropShadow".into(), params: json!({"x": 7.0, "y": 8.0}), visible: true },
    ]);
    let art = reshape(None, &source, None).unwrap();
    let mut shadows = vec![];
    art.walk(&mut |node| {
        let effects: Vec<_> =
            node.appearance.effects.iter().filter(|effect| effect.id == "stylize.dropShadow").map(|effect| effect.params.clone()).collect();
        if !effects.is_empty() {
            shadows.push(effects);
        }
    });
    assert_eq!(shadows, vec![vec![json!({"x": 3.0, "y": 4.0}), json!({"x": 7.0, "y": 8.0})]]);
    vectorcraft_plugins::registry::remove(id);
}

/// The same on a fill: the plug-in and the Transform effect after it are the fill's. The piece's
/// Offset Path runs on it alone, so the plug-in runs once and the square moves once.
#[test]
fn a_fill_s_later_effects_run_after_its_piece_s_effects_once() {
    let id = "test.effects.look.rowoffset";
    let piece = json!({"path": shapes::rectangle(Rect::new(120.0, 40.0, 140.0, 60.0)), "opacity": 0.5,
        "effects": [{"id": "path.offsetPath", "params": {"offset": 10.0}}]});
    let manifest = format!(
        r#"{{"id":"{id}","name":"Test {id}","version":"1","kind":"effect","api":3,"paintOutput":true,"appearanceOutput":true,"params":{{}}}}"#
    );
    let module = wat::parse_str(tpl::returning(&manifest, &json!({ "objects": [piece] }).to_string())).unwrap();
    vectorcraft_plugins::registry::install_bytes(&module).unwrap();
    let mut n =
        Node::path(NodeId(9), shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    let fill = n.appearance.items.iter().position(|i| i.is_fill()).unwrap();
    let fx = n.appearance.items[fill].effects_mut();
    fx.push(Effect { id: format!("plugin.{id}"), params: json!({}), visible: true });
    fx.push(Effect { id: "distort.transform".into(), params: json!({"moveH": 100.0}), visible: true });
    let art = reshape(None, &n, None).unwrap();
    let mut paths = vec![];
    art.walk(&mut |c| {
        if let Some(b) = c.path_data().and_then(PathData::bounds)
            && c.appearance.items.iter().any(|i| i.is_fill() && crate::paints(i))
        {
            paths.push(b);
        }
    });
    assert_eq!(paths.len(), 1, "{paths:?}");
    let b = paths[0];
    assert!((b.x0 - 210.0).abs() < 0.5 && (b.x1 - 250.0).abs() < 0.5 && (b.y0 - 30.0).abs() < 0.5, "{b:?}");
    vectorcraft_plugins::registry::remove(id);
}
