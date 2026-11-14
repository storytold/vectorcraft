#![allow(clippy::unwrap_used, clippy::panic)]
use serde_json::{Value, json};
use vectorcraft_doc::NodeId;

use crate::Session;

/// A 100 × 100 square with `effect`, and a 10 × 20 red rectangle beside it kept as graphic "Mark".
fn setup(effect: &str) -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width":400,"height":400})).unwrap();
    let mark = s.execute("path.create", &json!({"d":"M300 300L310 300L310 320L300 320Z"})).unwrap()["id"].clone();
    s.execute("paint.setFill", &json!({"color":"#ff0000"})).unwrap();
    let made = s.execute("threeD.addGraphic", &json!({"name": "Mark", "ids": [mark]})).unwrap();
    assert_eq!(made["name"], "Mark");
    let id = NodeId(s.execute("path.create", &json!({"d":"M100 100L200 100L200 200L100 200Z"})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"color":"#338ad6"})).unwrap();
    s.execute("effect.apply", &json!({"effect":effect,"item":null,"params":{"rotationX":0.0,"rotationY":0.0,"rotationZ":0.0}})).unwrap();
    (s, id)
}

fn graphics(s: &Session, id: NodeId) -> Value {
    s.doc().unwrap().doc.node(id).unwrap().appearance.effects[0].params["graphics"].clone()
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

#[test]
fn add_as_single_graphic_keeps_the_art_and_makes_a_symbol() {
    let (s, _) = setup("threeD.extrude");
    let d = &s.doc().unwrap().doc;
    assert!(d.symbols.iter().any(|sym| sym.name == "Mark"));
    // The art stays a plain path: no instance replaced it.
    let mut instances = 0;
    for l in &d.layers {
        l.walk(&mut |n| instances += usize::from(matches!(n.kind, vectorcraft_doc::NodeKind::SymbolInstance { .. })));
    }
    assert_eq!(instances, 0);
    // The 10 × 20 rectangle and its 1 pt stroke.
    assert_eq!(vectorcraft_render::effects::graphic_bounds(d, "Mark").map(|r| (r.width(), r.height())), Some((11.0, 21.0)));
}

#[test]
fn map_graphic_adds_to_the_selected_object_in_one_undo_step_and_undoes() {
    let (mut s, id) = setup("threeD.extrude");
    let before = s.doc().unwrap().doc.clone();
    let history = undo_len(&s);
    let r = s.execute("threeD.mapGraphic", &json!({"symbol": "Mark", "x": 5.0, "rotation": 30.0})).unwrap();
    assert_eq!(r["ids"], json!([id.0]));
    assert_eq!(graphics(&s, id), json!([{"symbol": "Mark", "face": "front", "x": 5.0, "y": 0.0, "scale": 100.0, "rotation": 30.0}]));
    assert_eq!(undo_len(&s), history + 1);
    s.execute("threeD.mapGraphic", &json!({"symbol": "Mark", "face": "back"})).unwrap();
    assert_eq!(graphics(&s, id).as_array().unwrap().len(), 2);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc.layers, before.layers);
}

#[test]
fn a_large_graphic_starts_inside_its_object() {
    let (mut s, id) = setup("threeD.rotate");
    let big = s.execute("path.create", &json!({"d":"M0 0L400 0L400 200L0 200Z"})).unwrap()["id"].clone();
    s.execute("threeD.addGraphic", &json!({"name": "Big", "ids": [big]})).unwrap();
    s.execute("threeD.mapGraphic", &json!({"symbol": "Big", "ids": [id.0]})).unwrap();
    // 401 pt wide with its stroke, in a 100 pt square.
    let scale = graphics(&s, id)[0]["scale"].as_f64().unwrap();
    assert!((scale - 100.0 * 100.0 / 401.0).abs() < 1e-9, "{scale}");
}

#[test]
fn map_graphic_puts_a_graphic_on_the_side_of_a_revolve() {
    let (mut s, id) = setup("threeD.revolve");
    let plain = vectorcraft_render::effects::revolve_art_in(Some(&s.doc().unwrap().doc), s.doc().unwrap().doc.node(id).unwrap());
    let history = undo_len(&s);
    s.execute("threeD.mapGraphic", &json!({"symbol": "Mark", "ids": [id.0]})).unwrap();
    assert_eq!(graphics(&s, id), json!([{"symbol": "Mark", "face": "front", "x": 0.0, "y": 0.0, "scale": 100.0, "rotation": 0.0}]));
    assert_eq!(undo_len(&s), history + 1);
    let d = &s.doc().unwrap().doc;
    assert_ne!(vectorcraft_render::effects::revolve_art_in(Some(d), d.node(id).unwrap()), plain, "the graphic is drawn");
    // A large graphic starts inside the half of the side seen from the front: 100π around the
    // axis (the square's far edge is 100 from it) and 300 along the profile (its edge on the axis
    // sweeps nothing).
    let big = s.execute("path.create", &json!({"d":"M0 0L400 0L400 200L0 200Z"})).unwrap()["id"].clone();
    s.execute("threeD.addGraphic", &json!({"name": "Big", "ids": [big]})).unwrap();
    s.execute("threeD.mapGraphic", &json!({"symbol": "Big", "ids": [id.0]})).unwrap();
    let scale = graphics(&s, id)[1]["scale"].as_f64().unwrap();
    assert!((scale - 100.0 * 100.0 * std::f64::consts::PI / 401.0).abs() < 1e-6, "{scale}");
    // The side is its only face.
    let history = undo_len(&s);
    let e = s.execute("threeD.mapGraphic", &json!({"symbol": "Mark", "face": "back", "ids": [id.0]})).unwrap_err().to_string();
    assert!(e.contains("side"), "{e}");
    assert_eq!(undo_len(&s), history);
}

#[test]
fn map_graphic_refuses_what_cannot_take_it_and_changes_nothing() {
    let (mut s, id) = setup("threeD.inflate");
    let history = undo_len(&s);
    let e = s.execute("threeD.mapGraphic", &json!({"symbol": "Mark", "ids": [id.0]})).unwrap_err().to_string();
    assert!(e.contains("Plane, Extrude and Revolve"), "{e}");
    let (mut s, id) = setup("threeD.rotate");
    for (p, says) in [
        (json!({"symbol": "Gone"}), "no symbol named"),
        (json!({"face": "front"}), "needs `symbol`"),
        (json!({"symbol": "Mark", "face": "back"}), "one face"),
        (json!({"symbol": "Mark", "scale": 0.0}), "scale"),
        (json!({"symbol": "Mark", "tile": true}), "`tile`"),
        (json!({"symbol": "Mark", "item": 0}), "`item` needs `index`"),
        (json!({"symbol": "Mark", "item": null}), "`item` needs `index`"),
    ] {
        let mut p = p;
        p["ids"] = json!([id.0]);
        let e = s.execute("threeD.mapGraphic", &p).unwrap_err().to_string();
        assert!(e.contains(says), "{p}: {e}");
    }
    assert_eq!(undo_len(&s), history);
    // An object without a 3D effect.
    let plain = s.execute("path.create", &json!({"d":"M0 0L10 0L10 10Z"})).unwrap()["id"].clone();
    let e = s.execute("threeD.mapGraphic", &json!({"symbol": "Mark", "ids": [plain]})).unwrap_err().to_string();
    assert!(e.contains("3D effect"), "{e}");
    // A hollow Extrude & Bevel has no cap to show a new graphic on.
    let (mut s, id) = setup("threeD.extrude");
    s.execute("effect.setParams", &json!({"index": 0, "item": null, "params": {"cap": false}})).unwrap();
    let history = undo_len(&s);
    let e = s.execute("threeD.mapGraphic", &json!({"symbol": "Mark", "ids": [id.0]})).unwrap_err().to_string();
    assert!(e.contains("turn on Cap"), "{e}");
    assert_eq!(undo_len(&s), history);
    // With a depth of 0 there is no back.
    s.execute("effect.setParams", &json!({"index": 0, "item": null, "params": {"cap": true, "depth": 0.0}})).unwrap();
    let history = undo_len(&s);
    let e = s.execute("threeD.mapGraphic", &json!({"symbol": "Mark", "face": "back", "ids": [id.0]})).unwrap_err().to_string();
    assert!(e.contains("one face"), "{e}");
    assert_eq!(undo_len(&s), history);
    assert!(s.execute("threeD.mapGraphic", &json!({"symbol": "Mark", "ids": [id.0]})).is_ok());
}

#[test]
fn set_params_edits_graphics_and_checks_them() {
    let (mut s, id) = setup("threeD.extrude");
    s.execute("threeD.mapGraphic", &json!({"symbol": "Mark"})).unwrap();
    let moved = json!([{"symbol": "Mark", "face": "front", "x": -12.0, "y": 8.0, "scale": 150.0, "rotation": 0.0}]);
    s.execute("effect.setParams", &json!({"ids": [id.0], "index": 0, "item": null, "params": {"graphics": moved}})).unwrap();
    assert_eq!(graphics(&s, id), moved);
    let bad = json!({"ids": [id.0], "index": 0, "item": null, "params": {"graphics": [{"symbol": "Mark", "face": "top"}]}});
    assert!(s.execute("effect.setParams", &bad).is_err());
    let null = json!({"ids": [id.0], "index": 0, "item": null, "params": {"graphics": null}});
    assert!(s.execute("effect.setParams", &null).is_err());
    assert_eq!(graphics(&s, id), moved);
    // Extrude to Plane keeps the graphics; to Inflate drops them.
    s.execute("effect.setParams", &json!({"ids": [id.0], "index": 0, "item": null, "threeDType": "plane"})).unwrap();
    assert_eq!(graphics(&s, id), moved);
    s.execute("effect.setParams", &json!({"ids": [id.0], "index": 0, "item": null, "threeDType": "inflate"})).unwrap();
    assert_eq!(graphics(&s, id), Value::Null);
}

#[test]
fn effect_commands_refuse_an_unknown_symbol_but_keep_a_deleted_ones_graphic() {
    let (mut s, id) = setup("threeD.extrude");
    let unknown = json!([{"symbol": "Nowhere"}]);
    let history = undo_len(&s);
    assert!(s.execute("effect.setParams", &json!({"ids": [id.0], "index": 0, "item": null, "params": {"graphics": unknown}})).is_err());
    s.execute("path.create", &json!({"d":"M0 0L50 0L50 50L0 50Z"})).unwrap();
    let created = undo_len(&s);
    assert!(s.execute("effect.apply", &json!({"effect": "threeD.extrude", "item": null, "params": {"graphics": unknown}})).is_err());
    assert_eq!((history + 1, undo_len(&s)), (created, created));
    // A graphic whose symbol was deleted stays while the effect's other settings change.
    s.execute("threeD.mapGraphic", &json!({"symbol": "Mark", "ids": [id.0]})).unwrap();
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).symbols.retain(|sym| sym.name != "Mark");
    s.execute("effect.setParams", &json!({"ids": [id.0], "index": 0, "item": null, "params": {"depth": 30.0}})).unwrap();
    assert_eq!(graphics(&s, id)[0]["symbol"], "Mark");
}

#[test]
fn a_symbol_a_graphic_uses_is_used_and_follows_a_rename() {
    let (mut s, id) = setup("threeD.extrude");
    s.execute("threeD.mapGraphic", &json!({"symbol": "Mark"})).unwrap();
    let unused = s.execute("symbol.unused", &json!({})).unwrap();
    assert!(!unused["names"].as_array().unwrap().iter().any(|n| n == "Mark"), "{unused}");
    // A 3D object with the graphic kept in another symbol's art follows the rename too.
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    s.execute("edit.paste", &json!({})).unwrap();
    s.execute("symbol.new", &json!({"name": "Holder"})).unwrap();
    s.execute("symbol.options", &json!({"name": "Mark", "newName": "Label"})).unwrap();
    assert_eq!(graphics(&s, id)[0]["symbol"], "Label");
    let d = &s.doc().unwrap().doc;
    let holder = d.symbols.iter().find(|sym| sym.name == "Holder").unwrap();
    let mut held = vec![];
    holder.art.walk(&mut |n| held.extend(vectorcraft_render::effects::graphic_symbols(n)));
    assert_eq!(held, vec!["Label".to_string()]);
}

#[test]
fn a_rename_reaches_graphics_in_graph_designs_and_opacity_masks() {
    let (mut s, id) = setup("threeD.extrude");
    s.execute("threeD.mapGraphic", &json!({"symbol": "Mark"})).unwrap();
    let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let held = d.node(id).unwrap().clone();
    d.graph_designs.push(vectorcraft_doc::GraphDesign { name: "Bars".into(), art: std::sync::Arc::new(held.clone()) });
    d.graphic_styles.push(vectorcraft_doc::GraphicStyle::new("Boxed", (*held.appearance).clone()));
    let mut lib = vectorcraft_brush::library(d);
    let scatter = lib.iter_mut().find(|b| !b.art().is_empty()).unwrap();
    *scatter.art_mut()[0] = held.clone();
    vectorcraft_brush::store(d, &lib);
    d.node_mut(id).unwrap().extras_mut().mask = Some(Box::new(vectorcraft_doc::OpacityMask::new(held, true)));
    s.execute("symbol.options", &json!({"name": "Mark", "newName": "Label"})).unwrap();
    let d = &s.doc().unwrap().doc;
    let style = d.graphic_styles.iter().find(|g| g.name == "Boxed").unwrap();
    assert_eq!(vectorcraft_render::effects::appearance_graphic_symbols(&style.appearance), vec!["Label".to_string()]);
    let brush: Vec<String> =
        vectorcraft_brush::library(d).iter().flat_map(|b| b.art()).flat_map(vectorcraft_render::effects::graphic_symbols).collect();
    assert_eq!(brush, vec!["Label".to_string()]);
    let names = |n: &vectorcraft_doc::Node| {
        let mut out = vec![];
        n.walk(&mut |c| out.extend(vectorcraft_render::effects::graphic_symbols(c)));
        out
    };
    assert_eq!(names(&d.graph_designs[0].art), vec!["Label".to_string()]);
    let mask = d.node(id).unwrap().extras().mask.as_deref().map(|m| names(&m.art));
    assert_eq!(mask, Some(vec!["Label".to_string()]));
}

#[test]
fn pasted_into_another_document_a_graphic_brings_its_symbol() {
    let (mut s, id) = setup("threeD.extrude");
    s.execute("threeD.mapGraphic", &json!({"symbol": "Mark"})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    // The symbol the pasted object's graphic names, and the size of that symbol's art.
    let pasted = |s: &mut Session| {
        s.execute("edit.paste", &json!({})).unwrap();
        let st = s.doc().unwrap();
        let n = st.doc.node(st.selection.objects[0]).unwrap();
        let name = n.appearance.effects[0].params["graphics"][0]["symbol"].as_str().unwrap().to_string();
        let size = vectorcraft_render::effects::graphic_bounds(&st.doc, &name).map(|r| (r.width(), r.height()));
        (name, size)
    };
    // A document without the symbol gets it.
    s.execute("file.new", &json!({"width":400,"height":400})).unwrap();
    assert_eq!(pasted(&mut s), ("Mark".to_string(), Some((11.0, 21.0))));
    // A document whose symbol "Mark" is other art gets the copied symbol under a free name.
    s.execute("file.new", &json!({"width":400,"height":400})).unwrap();
    let other = s.execute("path.create", &json!({"d":"M0 0L50 0L50 50L0 50Z"})).unwrap()["id"].clone();
    s.execute("threeD.addGraphic", &json!({"name": "Mark", "ids": [other]})).unwrap();
    let (name, size) = pasted(&mut s);
    assert_ne!(name, "Mark");
    assert_eq!(size, Some((11.0, 21.0)));
}

#[test]
fn a_symbol_only_a_graphic_style_uses_is_used() {
    let (mut s, id) = setup("threeD.extrude");
    s.execute("threeD.mapGraphic", &json!({"symbol": "Mark"})).unwrap();
    let appearance = (*s.doc().unwrap().doc.node(id).unwrap().appearance).clone();
    s.execute("effect.setParams", &json!({"index": 0, "item": null, "params": {"graphics": []}})).unwrap();
    let unused = |s: &mut Session| s.execute("symbol.unused", &json!({})).unwrap()["names"].clone();
    assert!(unused(&mut s).as_array().unwrap().iter().any(|n| n == "Mark"));
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).graphic_styles.push(vectorcraft_doc::GraphicStyle::new("Boxed", appearance));
    assert!(!unused(&mut s).as_array().unwrap().iter().any(|n| n == "Mark"));
}

#[test]
fn add_graphic_refuses_bad_parameters_and_changes_nothing() {
    let (mut s, _) = setup("threeD.extrude");
    let art = s.execute("path.create", &json!({"d":"M0 0L10 0L10 10Z"})).unwrap()["id"].clone();
    let (symbols, history) = (s.doc().unwrap().doc.symbols.len(), undo_len(&s));
    for p in [
        json!({"name": 5, "ids": [art]}),
        json!({"name": null, "ids": [art]}),
        json!({"name": "", "ids": [art]}),
        json!({"name": "  ", "ids": [art]}),
        json!({"tile": true, "ids": [art]}),
    ] {
        assert!(s.execute("threeD.addGraphic", &p).is_err(), "{p}");
    }
    assert_eq!((s.doc().unwrap().doc.symbols.len(), undo_len(&s)), (symbols, history));
}

#[test]
fn map_graphic_refuses_a_selection_with_an_object_without_a_3d_effect() {
    let (mut s, id) = setup("threeD.extrude");
    let plain = s.execute("path.create", &json!({"d":"M0 0L10 0L10 10Z"})).unwrap()["id"].as_u64().unwrap();
    let history = undo_len(&s);
    let e = s.execute("threeD.mapGraphic", &json!({"symbol": "Mark", "ids": [id.0, plain]})).unwrap_err().to_string();
    assert!(e.contains(&format!("object {plain} needs a 3D effect")), "{e}");
    assert_eq!(undo_len(&s), history);
    assert_eq!(graphics(&s, id).as_array().map_or(0, Vec::len), 0);
}

#[test]
fn the_graphic_is_drawn_on_the_object() {
    let (mut s, id) = setup("threeD.extrude");
    s.execute("threeD.mapGraphic", &json!({"symbol": "Mark"})).unwrap();
    let d = &s.doc().unwrap().doc;
    let art = vectorcraft_render::effects::solid_art_in(Some(d), d.node(id).unwrap()).unwrap();
    let mut red = None;
    art.walk(&mut |n| {
        if let (Some(p), Some(vectorcraft_doc::AppearanceItem::Fill(f))) = (n.path_data(), n.appearance.items.first())
            && f.paint.color().is_some_and(|c| c.to_rgb()[0] > 0.3 && c.to_rgb()[1] < 0.1)
        {
            red = p.bounds();
        }
    });
    // Seen straight on, the 10 × 20 graphic sits at the center of the square.
    let r = red.unwrap();
    assert!(
        (r.center().x - 150.0).abs() < 1e-6
            && (r.center().y - 150.0).abs() < 1e-6
            && (r.width() - 10.0).abs() < 1e-6
            && (r.height() - 20.0).abs() < 1e-6,
        "{r:?}"
    );
}
