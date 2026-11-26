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

/// The output of a plug-in that returns one piece: the square at `x`, 40 filled red, with `look`'s
/// keys added to it.
fn output(x: f64, look: &serde_json::Value) -> String {
    let red = serde_json::to_value(Paint::solid(Color::rgb(1.0, 0.0, 0.0))).unwrap();
    let mut piece = json!({"path": square_at(x), "fills": [red]});
    if let (Some(p), Some(l)) = (piece.as_object_mut(), look.as_object()) {
        p.extend(l.clone());
    }
    json!({ "objects": [piece] }).to_string()
}

/// The manifest of live-effect plug-in `id`, which declares `appearanceOutput`.
fn manifest(id: &str) -> String {
    format!(r#"{{"id":"{id}","name":"Test {id}","version":"1","kind":"effect","api":3,"paintOutput":true,"appearanceOutput":true,"params":{{}}}}"#)
}

/// Installs live-effect plug-in `id`, which returns one piece: the square at 120, 40 filled red,
/// with `look`'s keys added to it.
fn install(id: &str, look: serde_json::Value) {
    let module = wat::parse_str(vectorcraft_plugins::wat::returning(&manifest(id), &output(120.0, &look))).unwrap();
    vectorcraft_plugins::registry::install_bytes(&module).unwrap();
}

/// Installs live-effect plug-in `id` as [`install`] does, except that it returns the square at 160,
/// 40 when its input holds the text `moved`.
fn install_moving(id: &str, look: serde_json::Value, moved: &str) {
    use vectorcraft_plugins::wat as tpl;
    let module = tpl::when_given_else(&manifest(id), tpl::Given::Input, moved, &output(160.0, &look), &output(120.0, &look));
    vectorcraft_plugins::registry::install_bytes(&wat::parse_str(module).unwrap()).unwrap();
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
    pixel_with(&mut Renderer::new(), d, x, y)
}

/// The color renderer `r` draws `d` with at (`x`, `y`).
fn pixel_with(r: &mut Renderer, d: &Document, x: f64, y: f64) -> [u8; 4] {
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    r.render(d, 1, 1, Affine::translate((-x, -y)), &opts).pixel(0, 0)
}

fn near(got: [u8; 4], want: [u8; 3]) -> bool {
    got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 8)
}

/// A piece at 50% opacity with Multiply draws red multiplied with the blue under it (black) at
/// half strength: half blue. Moved in the same document and drawn by the same renderer, the source
/// runs the plug-in again, which now returns the square at 160, 40, and the new piece draws the
/// same way. Without the look the piece is plain red.
#[test]
fn a_piece_s_opacity_and_blending_mode_draw_and_survive_a_rerun() {
    const ID: &str = "test.render.paintedlook.multiply";
    install_moving(ID, json!({"opacity": 0.5, "blend": "Multiply"}), "33.25");
    let mut d = document(ID, 0.0);
    let source = d.layers[0].children().and_then(|c| c.last()).map(|n| n.id).unwrap();
    let mut r = Renderer::new();
    let p = pixel_with(&mut r, &d, 130.0, 50.0);
    assert!(near(p, [0, 0, 128]), "before the move: {p:?}");
    assert!(near(pixel_with(&mut r, &d, 10.0, 50.0), [0, 0, 255]), "the source is not drawn");
    d.node_mut(source).unwrap().transform(Affine::translate((33.25, 0.0)), false);
    let p = pixel_with(&mut r, &d, 170.0, 50.0);
    assert!(near(p, [0, 0, 128]), "after the move: {p:?}");
    assert!(near(pixel_with(&mut r, &d, 130.0, 50.0), [0, 0, 255]), "the earlier piece is not drawn");
    assert!(near(pixel_with(&mut r, &d, 43.0, 50.0), [0, 0, 255]), "the moved source is not drawn");
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

#[test]
fn later_painted_pieces_keep_the_earlier_piece_transparency_as_a_group() {
    for stage in ["object", "fill", "stroke"] {
        let first = format!("test.render.paintedlook.transparency.{stage}");
        let second = format!("test.render.paintedlook.overlapping.{stage}");
        let value: serde_json::Value = serde_json::from_str(&output(120.0, &json!({}))).unwrap();
        let mut piece = value["objects"][0].clone();
        if stage == "stroke" {
            let fields = piece.as_object_mut().unwrap();
            let red = fields.remove("fills").unwrap()[0].clone();
            fields.insert("strokes".into(), json!([{"paint": red, "width": 4.0}]));
        }
        let mut transparent = piece.clone();
        transparent["opacity"] = json!(0.5);
        transparent["blend"] = json!("Multiply");
        let first_output = json!({"objects": [transparent]}).to_string();
        let module = wat::parse_str(vectorcraft_plugins::wat::returning(&manifest(&first), &first_output)).unwrap();
        vectorcraft_plugins::registry::install_bytes(&module).unwrap();
        let result = json!({"objects": [piece.clone(), piece]}).to_string();
        let module = wat::parse_str(vectorcraft_plugins::wat::returning(&manifest(&second), &result)).unwrap();
        vectorcraft_plugins::registry::install_bytes(&module).unwrap();
        let mut d = document(&first, 0.0);
        let source = d.layers[0].children().unwrap().last().unwrap().id;
        let n = d.node_mut(source).unwrap();
        let mut effects = std::mem::take(&mut n.appearance.effects);
        effects.push(Effect { id: format!("plugin.{second}"), params: json!({}), visible: true });
        if stage == "object" {
            n.appearance.effects = effects;
        } else {
            if stage == "stroke" {
                n.appearance = Appearance::basic(Paint::None, Paint::solid(Color::rgb(0.0, 1.0, 0.0)), 4.0).into();
            }
            let row = n.appearance.items.iter_mut().find(|row| if stage == "fill" { row.is_fill() } else { !row.is_fill() }).unwrap();
            row.effects_mut().extend(effects);
        }
        let got = pixel(&d, if stage == "stroke" { 120.0 } else { 130.0 }, 50.0);
        assert!(near(got, [0, 0, 128]), "{stage}: {got:?}");
        assert_eq!(vectorcraft_plugins::effect::last_error(&first), None);
        assert_eq!(vectorcraft_plugins::effect::last_error(&second), None);
        vectorcraft_plugins::registry::remove(&first);
        vectorcraft_plugins::registry::remove(&second);
    }
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
