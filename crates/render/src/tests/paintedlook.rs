//! A live-effect plug-in that declares `appearanceOutput` gives each piece it returns an opacity, a
//! blending mode and effects, and the canvas draws the piece with them. A refused look (an unknown
//! blending mode, a plug-in effect) leaves the source as it is.

use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Effect, Node};
use vectorcraft_geom::{Affine, PathData, Point, Rect, SubPath, shapes};

use crate::{RenderOptions, Renderer};

/// A 20 × 20 square at `x`, 40.
fn square_at(x: f64) -> PathData {
    PathData::single(SubPath::polyline(&[Point::new(x, 40.0), Point::new(x + 20.0, 40.0), Point::new(x + 20.0, 60.0), Point::new(x, 60.0)], true))
}

/// Installs live-effect plug-in `id`, which declares `appearanceOutput` and returns one piece: the
/// square at 120, 40 filled red, with `look`'s keys added to it.
fn install(id: &str, look: serde_json::Value) {
    use vectorcraft_plugins::wat as tpl;
    let red = serde_json::to_value(Paint::solid(Color::rgb(1.0, 0.0, 0.0))).unwrap();
    let mut piece = json!({"path": square_at(120.0), "fills": [red]});
    if let (Some(p), Some(l)) = (piece.as_object_mut(), look.as_object()) {
        p.extend(l.clone());
    }
    let output = json!({ "objects": [piece] }).to_string();
    let manifest = format!(
        r#"{{"id":"{id}","name":"Test {id}","version":"1","kind":"effect","api":3,"paintOutput":true,"appearanceOutput":true,"params":{{}}}}"#
    );
    let module = wat::parse_str(tpl::returning(&manifest, &output)).unwrap();
    vectorcraft_plugins::registry::install_bytes(&module).unwrap();
}

/// A document with a blue rectangle over all of it and, above it, the square at `x`, 40 filled
/// green with live-effect plug-in `plugin` as its own effect.
fn document(plugin: &str, x: f64) -> Document {
    let mut d = Document::new(300.0, 100.0);
    let l = d.layers[0].id;
    let blue = Node::path(
        d.alloc_id(),
        shapes::rectangle(Rect::new(0.0, 0.0, 300.0, 100.0)),
        Appearance::basic(Paint::solid(Color::rgb(0.0, 0.0, 1.0)), Paint::None, 0.0),
    );
    d.insert(Some(l), 0, blue).unwrap();
    let mut ap = Appearance::basic(Paint::solid(Color::rgb(0.0, 1.0, 0.0)), Paint::None, 0.0);
    ap.effects = vec![Effect { id: format!("plugin.{plugin}"), params: json!({}), visible: true }];
    let n = Node::path(d.alloc_id(), square_at(x), ap);
    d.insert(Some(l), usize::MAX, n).unwrap();
    d
}

/// The color a new renderer draws `d` with at (`x`, `y`).
fn pixel(d: &Document, x: f64, y: f64) -> [u8; 4] {
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    Renderer::new().render(d, 1, 1, Affine::translate((-x, -y)), &opts).pixel(0, 0)
}

fn near(got: [u8; 4], want: [u8; 3]) -> bool {
    got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 8)
}

/// A piece at 50% opacity with Multiply draws red multiplied with the blue under it (black) at
/// half strength: half blue. Moved, the source runs the plug-in again, and the new piece draws the
/// same way. Without the look the piece is plain red.
#[test]
fn a_piece_s_opacity_and_blending_mode_draw_and_survive_a_rerun() {
    const ID: &str = "test.render.paintedlook.multiply";
    install(ID, json!({"opacity": 0.5, "blend": "Multiply"}));
    for x in [0.0, 2.0] {
        let d = document(ID, x);
        let p = pixel(&d, 130.0, 50.0);
        assert!(near(p, [0, 0, 128]), "source at {x}: {p:?}");
        assert!(near(pixel(&d, 10.0, 50.0), [0, 0, 255]), "the source is not drawn");
    }
    assert_eq!(vectorcraft_plugins::effect::last_error(ID), None);
    install(ID, json!({}));
    assert!(near(pixel(&document(ID, 4.0), 130.0, 50.0), [255, 0, 0]));
    vectorcraft_plugins::registry::remove(ID);
}

/// A piece's geometry effect reshapes it: Offset Path by 10 pt draws red 5 pt left of the square.
#[test]
fn a_piece_s_geometry_effect_draws_on_it() {
    const ID: &str = "test.render.paintedlook.offset";
    install(ID, json!({"effects": [{"id": "path.offsetPath", "params": {"offset": 10.0}}]}));
    let d = document(ID, 0.0);
    assert!(near(pixel(&d, 115.0, 50.0), [255, 0, 0]), "{:?}", pixel(&d, 115.0, 50.0));
    assert!(near(pixel(&d, 130.0, 50.0), [255, 0, 0]));
    vectorcraft_plugins::registry::remove(ID);
}

/// An unknown blending mode, a plug-in effect (this one itself) and too many effects refuse the
/// whole result: the source draws as it is, green at its own place, with no piece, and the plug-in
/// reports the field.
#[test]
fn a_refused_look_leaves_the_source_unchanged() {
    for (name, look, field) in [
        ("blend", json!({"blend": "multiply"}), "objects[0].blend"),
        ("self", json!({"effects": [{"id": "plugin.test.render.paintedlook.self"}]}), "objects[0].effects[0]"),
        ("many", json!({"effects": vec![json!({"id": "stylize.dropShadow"}); 17]}), "objects[0].effects"),
    ] {
        let id = format!("test.render.paintedlook.{name}");
        install(&id, look);
        let d = document(&id, 0.0);
        assert!(near(pixel(&d, 10.0, 50.0), [0, 255, 0]), "{name}: the source draws");
        assert!(near(pixel(&d, 130.0, 50.0), [0, 0, 255]), "{name}: no piece");
        let e = vectorcraft_plugins::effect::last_error(&id).unwrap_or_default();
        assert!(e.contains(field), "{name}: {e}");
        vectorcraft_plugins::registry::remove(&id);
    }
}
