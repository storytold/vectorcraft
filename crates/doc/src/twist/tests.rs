use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_geom::{Point, Rect, shapes};

use super::*;
use crate::shaper::{FaceEdit, FaceSelector, ShaperSpec};
use crate::{GraphicStyle, OpacityMask, PatternDef, Symbol};

fn fx(id: &str, params: Value) -> Effect {
    Effect { id: id.into(), params, visible: true }
}

/// An appearance with a Twist of `params` on the object and on its fill, and a Roughen.
fn twisted_appearance(params: &Value) -> Appearance {
    let mut a = Appearance::default_art();
    a.effects.push(fx(TWIST, params.clone()));
    a.effects.push(fx("distort.roughen", json!({"size": 5})));
    a.items[0].effects_mut().push(fx(TWIST, params.clone()));
    a
}

fn twisted(d: &mut Document, params: &Value) -> Node {
    Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), twisted_appearance(params))
}

/// A version 1 document with Twist effects of `params` everywhere one is stored: on a path, in a
/// group, in an opacity mask's art, in a Shaper edit, in a symbol, a pattern swatch and a graphic
/// style, and `brush` in a brush definition. Its first layer holds the art; its last has no
/// Twist.
fn document(params: &Value, brush: &Value) -> Document {
    let mut d = Document::new(100.0, 100.0);
    let l = d.layers[0].id;
    let path = twisted(&mut d, params);
    d.insert(Some(l), 0, path).unwrap();
    let inner = twisted(&mut d, params);
    let group = Node::group(d.alloc_id(), vec![Arc::new(inner)]);
    d.insert(Some(l), 1, group).unwrap();
    let mask_art = twisted(&mut d, params);
    let mut masked = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 5.0, 5.0)), Appearance::default_art());
    masked.extras_mut().mask = Some(Box::new(OpacityMask::new(mask_art, true)));
    d.insert(Some(l), 2, masked).unwrap();
    let mut shaper = Node::group(d.alloc_id(), vec![]);
    let face = FaceEdit {
        selector: FaceSelector { sources: vec![0], anchor: Point::ZERO },
        erase: false,
        erase_edges: false,
        paint_source: None,
        merge: None,
        paint: Some(twisted_appearance(params)),
    };
    shaper.extras_mut().shaper = Some(Box::new(ShaperSpec { faces: vec![face], edges: vec![] }));
    d.insert(Some(l), 3, shaper).unwrap();
    let art = twisted(&mut d, params);
    d.symbols.push(Symbol { name: "S".into(), art: Arc::new(art), dynamic: false });
    let art = twisted(&mut d, params);
    d.patterns.push(PatternDef::new("P", vec![Arc::new(art)]));
    d.graphic_styles.push(GraphicStyle::new("G", twisted_appearance(params)));
    d.unknown.insert(BRUSHES_KEY.into(), json!([{"name": "B", "art": {"id": 99, "appearance": {"effects": [brush]}}}]));
    d.add_layer(None);
    d.version = 1;
    d
}

/// The angle each Twist effect of the saved document draws with, in the order the JSON lists them.
fn angles(d: &Document) -> Vec<f64> {
    fn walk(v: &Value, out: &mut Vec<f64>) {
        match v {
            Value::Object(m) => {
                if is_twist_json(m) {
                    out.push(angle(m.get("params").unwrap_or(&Value::Null)));
                }
                m.values().for_each(|c| walk(c, out));
            }
            Value::Array(a) => a.iter().for_each(|c| walk(c, out)),
            _ => {}
        }
    }
    let mut out = vec![];
    walk(&serde_json::to_value(d).unwrap(), &mut out);
    out
}

#[test]
fn a_version_1_document_from_before_0_21_has_its_twist_angles_negated() {
    let mut d = document(&json!({"angle": 30}), &json!({"id": TWIST}));
    let before = angles(&d);
    // Two per path, shaper edit and style; one in the brush.
    assert_eq!(before.len(), 15, "{before:?}");
    assert_eq!(before.iter().filter(|&&a| a == DEFAULT_ANGLE).count(), 1, "{before:?}");
    d.migrate_twist_angles(|| false);
    assert_eq!(d.version, 1, "the caller sets the version");
    assert_eq!(angles(&d), before.iter().map(|a| -a).collect::<Vec<_>>());
    let roughen = serde_json::to_string(&d).unwrap().matches(r#""id":"distort.roughen","params":{"size":5}"#).count();
    assert_eq!(roughen, 7, "other effects are left alone");
}

#[test]
fn a_document_that_turned_clockwise_is_left_as_it_is() {
    let mut d = document(&json!({"angle": 30}), &json!({"id": TWIST, "params": {"angle": 30}}));
    let before = d.clone();
    d.migrate_twist_angles(|| true);
    assert_eq!(d, before, "version 1, written by 0.21.0 or later");
    d.version = 2;
    let before = d.clone();
    d.migrate_twist_angles(|| panic!("a version 2 document's writer is not asked about"));
    assert_eq!(d, before, "version 2");
}

#[test]
fn the_writer_is_asked_about_only_when_a_twist_is_stored() {
    let mut d = Document::new(10.0, 10.0);
    d.version = 1;
    let before = d.clone();
    d.migrate_twist_angles(|| panic!("a document without a Twist needs no answer"));
    assert_eq!(d, before);
}

#[test]
fn angles_are_read_as_the_effect_reads_them() {
    let read = |params: Value| {
        let mut e = fx(TWIST, params);
        negate_params(&mut e.params);
        e.params
    };
    assert_eq!(read(json!({"angle": 30, "x": 1})), json!({"angle": -30.0, "x": 1}));
    assert_eq!(read(json!({"angle": "45 deg"})), json!({"angle": -45.0}));
    assert_eq!(read(json!({"angle": true})), json!({"angle": -1.0}));
    assert_eq!(read(json!({})), json!({"angle": -10.0}));
    assert_eq!(read(Value::Null), json!({"angle": -10.0}));
    let zero = read(json!({"angle": 0}));
    assert!(zero["angle"].as_f64().is_some_and(|a| a == 0.0 && a.is_sign_positive()), "{zero}");
}

#[test]
fn only_the_art_holding_a_twist_is_copied() {
    let mut d = document(&json!({"angle": 30}), &json!({"id": TWIST}));
    let plain = d.layers.last().cloned().unwrap();
    let twisted = d.layers[0].clone();
    d.migrate_twist_angles(|| false);
    assert!(Arc::ptr_eq(d.layers.last().unwrap(), &plain), "a layer without a Twist is not copied");
    assert!(!Arc::ptr_eq(&d.layers[0], &twisted));
}

#[test]
fn storing_for_an_earlier_app_and_reading_back_round_trips() {
    let original = document(&json!({"angle": 30.0}), &json!({"id": TWIST, "params": {"angle": -12.5}}));
    let mut d = original.clone();
    d.migrate_twist_angles(|| false);
    d.version = 2;
    let current = d.clone();
    assert_eq!(angles(&current), angles(&original).iter().map(|a| -a).collect::<Vec<_>>());
    d.store_legacy_twist_angles();
    assert_eq!(d, original);
    d.migrate_twist_angles(|| false);
    d.version = 2;
    assert_eq!(d, current);
    // Another conversion for the same save may have marked the document version 1 already.
    d.version = 1;
    d.store_legacy_twist_angles();
    assert_eq!(d, original);
}

#[test]
fn a_version_1_style_library_has_its_twist_angles_negated() {
    let lib = StyleLibrary { name: "L".into(), styles: vec![GraphicStyle::new("G", twisted_appearance(&json!({"angle": 30.0})))], patterns: vec![] };
    let text = crate::style_libs::write(&lib);
    assert!(text.contains(r#""version": 2"#), "{text}");
    assert_eq!(crate::style_libs::read(&text, "x").unwrap(), lib, "a version 2 file reads as written");
    let old = crate::style_libs::read(&text.replacen(r#""version": 2"#, r#""version": 1"#, 1), "x").unwrap();
    let fx = &old.styles[0].appearance.effects;
    assert_eq!(fx[0].params, json!({"angle": -30.0}));
    assert_eq!(fx[1], lib.styles[0].appearance.effects[1]);
    assert_eq!(old.styles[0].appearance.items[0].effects()[0].params, json!({"angle": -30.0}));
}

#[test]
fn releases_from_0_21_turned_twist_clockwise() {
    for g in ["VectorCraft 0.21.0", "VectorCraft 0.21.4", "VectorCraft 0.22.0-dev", "VectorCraft 1.0.0"] {
        assert!(written_clockwise(Some(g)), "{g}");
    }
    for g in ["VectorCraft 0.20.0", "VectorCraft 0.9.12", "DrawCraft 0.1.0", "VectorCraft", "VectorCraft x.y", "vectorcraft 0.21.0"] {
        assert!(!written_clockwise(Some(g)), "{g}");
    }
    assert!(!written_clockwise(None));
}
