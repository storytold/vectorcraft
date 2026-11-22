//! A live-effect plug-in before a fill's or stroke's own 3D effect is told which fill or stroke it
//! is on, in the drawn art, the exported meshes and the wireframe.

use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Effect, FillLayer, Node, NodeId};
use vectorcraft_geom::{PathData, Rect, shapes};
use vectorcraft_plugins::wat as tpl;

use crate::{EXTRUDE, solid_art, three_d_meshes_in, three_d_wireframe_in};

/// Widths the plug-in's squares are drawn at: `LARGE` when it is told it is on the item it looks
/// for, `SMALL` otherwise.
const LARGE: f64 = 600.0;
const SMALL: f64 = 6.0;

/// A rectangle with a fill, a 1 pt stroke and a second fill on top. The item at `at` (an index in
/// its appearance items) has, in its own effects, live-effect plug-in `name` and then Extrude &
/// Bevel. The plug-in returns a `LARGE` square when it is given `item`, and a `SMALL` one otherwise.
fn plugin_then_extrude(name: &str, at: usize, item: &str) -> Node {
    let square = |w: f64| shapes::rectangle(Rect::new(0.0, 0.0, w, w));
    let id = format!("test.effects.solidcontext.{name}");
    let manifest = format!(r#"{{"id":"{id}","name":"Test {id}","version":"1","kind":"effect","params":{{}}}}"#);
    let (large, small) = (json!({"objects": [{"path": square(LARGE)}]}).to_string(), json!({"objects": [{"path": square(SMALL)}]}).to_string());
    let module = wat::parse_str(tpl::when_given_else(&manifest, tpl::Given::Params, item, &large, &small)).unwrap();
    vectorcraft_plugins::registry::install_bytes(&module).unwrap();
    let mut appearance = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::solid(Color::BLACK), 1.0);
    appearance.items.push(AppearanceItem::Fill(FillLayer::new(Paint::solid(Color::rgb(0.0, 0.0, 1.0)))));
    let effects = appearance.items[at].effects_mut();
    effects.push(Effect { id: format!("plugin.{id}"), params: json!({}), visible: true });
    effects.push(crate::new_effect(EXTRUDE, &json!({})).unwrap());
    Node::path(NodeId(7), shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)), appearance)
}

/// The width of the union of `paths`' bounds.
fn width(paths: impl IntoIterator<Item = PathData>) -> f64 {
    paths.into_iter().filter_map(|p| p.bounds()).reduce(|a, b| a.union(b)).map_or(0.0, |b| b.width())
}

/// Whether each of the art, the meshes and the wireframe `n`'s 3D effect draws is wider than half
/// of `LARGE`, with the widths.
fn drawn_large(n: &Node) -> ([bool; 3], [f64; 3]) {
    let art = solid_art(n).unwrap();
    let mut paths = vec![];
    art.walk(&mut |c| paths.extend(c.path_data().cloned()));
    let meshes = three_d_meshes_in(None, n).unwrap();
    let xs = meshes.iter().flat_map(|m| m.mesh.positions.iter().map(|p| p[0]));
    let mesh_width = xs.clone().fold(f64::NEG_INFINITY, f64::max) - xs.fold(f64::INFINITY, f64::min);
    let widths = [width(paths), mesh_width, width(three_d_wireframe_in(None, n).unwrap())];
    (widths.map(|w| w > LARGE / 2.0), widths)
}

/// A plug-in before the second fill's Extrude & Bevel is given `{"fill": 1}`, not `"object"`, so
/// the art, meshes and wireframe are all built from the square it returns for that fill.
#[test]
fn a_plugin_before_a_fills_extrude_is_told_its_fill() {
    let (large, widths) = drawn_large(&plugin_then_extrude("fill", 2, r#""item":{"fill":1}"#));
    assert_eq!(large, [true; 3], "the art, meshes and wireframe are {widths:?} wide");
}

/// A plug-in before the stroke's Extrude & Bevel is given `{"stroke": 0}`.
#[test]
fn a_plugin_before_a_strokes_extrude_is_told_its_stroke() {
    let (large, widths) = drawn_large(&plugin_then_extrude("stroke", 1, r#""item":{"stroke":0}"#));
    assert_eq!(large, [true; 3], "the art, meshes and wireframe are {widths:?} wide");
}

/// A plug-in on a fill is not told it is on the object: the square it returns otherwise is the one
/// extruded.
#[test]
fn a_plugin_on_a_fill_is_not_told_it_is_on_the_object() {
    let (large, widths) = drawn_large(&plugin_then_extrude("object", 2, r#""item":"object""#));
    assert_eq!(large, [false; 3], "the art, meshes and wireframe are {widths:?} wide");
}
