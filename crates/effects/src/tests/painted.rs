//! Live-effect plug-ins that paint what they return, run on each fill a painted effect returned.

use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Effect, Node, NodeId, NodeKind};
use vectorcraft_geom::{Anchor, PathData, Point, Rect, SubPath, shapes};
use vectorcraft_plugins::wat as tpl;

use crate::geometry::Budget;
use crate::reshape;
use crate::reshape::{HELD_PEAK, PLUGIN_RUNS};

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
