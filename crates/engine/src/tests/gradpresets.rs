//! The Gradient panel's presets (`gradient.presets.*`): the built-in gradient libraries by theme,
//! applied in place to the gradient behind a proxy, and gradients saved beside them with the
//! preferences.

use serde_json::{Value, json};
use vectorcraft_color::{Color, GradientKind, GradientMethod, Paint};

use crate::*;

/// A document with a selected rectangle filled with `fill` (`paint.setFill` params).
fn session_with(fill: Value) -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
    let r = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 50})).unwrap();
    s.execute("paint.setFill", &fill).unwrap();
    (s, NodeId(r["id"].as_u64().unwrap()))
}

fn paint(s: &Session, id: NodeId, stroke: bool) -> vectorcraft_color::GradientPaint {
    let n = s.doc().unwrap().doc.node(id).unwrap();
    match n.appearance.paint_at(None, !stroke).unwrap().clone() {
        Paint::Gradient(g) => *g,
        p => panic!("expected a gradient, got {p:?}"),
    }
}

fn names(list: &Value) -> Vec<String> {
    list["presets"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap().to_string()).collect()
}

#[test]
fn built_in_presets_are_the_gradient_libraries_by_theme_with_unique_names() {
    let mut s = Session::new();
    let list = s.execute("gradient.presets.list", &json!({})).unwrap();
    let rows = list["presets"].as_array().unwrap();
    assert!(rows.len() > 40, "{} presets", rows.len());
    assert!(rows.iter().all(|r| r["builtIn"] == true && r["gradient"]["stops"].as_array().is_some_and(|a| a.len() >= 2)));
    // Grouped by library, the gradient libraries first, then the swatch libraries holding gradients.
    let mut libraries: Vec<&str> = rows.iter().map(|r| r["library"].as_str().unwrap()).collect();
    libraries.dedup();
    assert_eq!(libraries, ["Spectrum Gradients", "Sky Gradients", "Neutral Gradients", "Duotone Gradients", "Radial Glows", "Metallic Gradients"]);
    let mut seen = std::collections::HashSet::new();
    assert!(names(&list).iter().all(|n| seen.insert(n.to_lowercase())), "a name names one preset");
    let vignette = rows.iter().find(|r| r["name"] == "Vignette").unwrap();
    assert_eq!((vignette["library"].as_str(), vignette["gradient"]["kind"].as_str()), (Some("Neutral Gradients"), Some("radial")));
}

#[test]
fn applying_a_preset_edits_the_gradient_in_place_as_one_undo_step() {
    let (mut s, id) = session_with(json!({"gradient": {"angle": 30}}));
    s.execute("paint.editGradient", &json!({"dither": true})).unwrap();
    let before = paint(&s, id, false);
    let undo = s.doc().unwrap().history.undo.len();
    let r = s.execute("gradient.presets.apply", &json!({"name": "vignette"})).unwrap();
    assert_eq!((r["name"].as_str(), r["ids"].clone()), (Some("Vignette"), json!([id.0])));
    let g = paint(&s, id, false);
    let preset = s.gradient_preset("Vignette").unwrap().gradient;
    assert_eq!((g.gradient.kind, &g.gradient.stops), (GradientKind::Radial, &preset.stops));
    assert!(!g.gradient.dither, "the fill takes the preset's dither");
    assert_eq!(g.angle, before.angle, "the placement stays");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    assert_eq!(s.gradient_preset_name(&g.gradient).as_deref(), Some("Vignette"), "the gradient is that preset now");
    assert_eq!(s.gradient_preset_name(&before.gradient), None);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(paint(&s, id, false), before);
}

#[test]
fn a_preset_applies_to_the_stroke_and_to_a_solid_paint() {
    let (mut s, id) = session_with(json!({"color": "#336699"}));
    s.execute("paint.setStroke", &json!({"color": "#000000", "focus": false})).unwrap();
    s.execute("gradient.presets.apply", &json!({"name": "Gold"})).unwrap();
    let gold = s.gradient_preset("Gold").unwrap();
    assert_eq!((gold.library, paint(&s, id, false).gradient.stops), (Some("Metallic Gradients"), gold.gradient.stops.clone()));
    // With the Stroke proxy in front (or `stroke: true`) the stroke takes it; dither isn't sent.
    s.execute("paint.toggleActive", &json!({"fill": false})).unwrap();
    s.execute("gradient.presets.apply", &json!({"name": "Dawn"})).unwrap();
    assert_eq!(paint(&s, id, true).gradient.stops, s.gradient_preset("Dawn").unwrap().gradient.stops);
    assert_eq!(paint(&s, id, false).gradient.stops, gold.gradient.stops, "the fill keeps its gradient");
    for bad in [json!({}), json!({"name": 3}), json!({"name": "No Such Preset"})] {
        assert!(s.execute("gradient.presets.apply", &bad).is_err(), "{bad}");
    }
}

#[test]
fn saving_listing_applying_and_deleting_a_preset() {
    let stops = json!([{"offset": 0, "color": "#ff0000"}, {"offset": 0.4, "color": "#00ff00", "opacity": 0.5, "midpoint": 0.3}, {"offset": 1, "color": "#0000ff"}]);
    let (mut s, id) = session_with(json!({"gradient": {"kind": "radial", "stops": stops, "method": "perceptual"}}));
    // A stop linked to a document swatch is saved with its colour, unlinked.
    s.execute("swatch.new", &json!({"name": "Brand", "color": "#123456", "global": true})).unwrap();
    s.execute("gradient.selectStop", &json!({"index": 1})).unwrap();
    let mut linked = paint(&s, id, false).gradient.stops.clone();
    linked[1].set_color(Color::rgb8(0x12, 0x34, 0x56), Some(("Brand".into(), 1.0)));
    s.execute("paint.editGradient", &json!({"stops": vectorcraft_tools::params::stops_json(&linked)})).unwrap();
    assert_eq!(paint(&s, id, false).gradient.stops[1].swatch.as_deref(), Some("Brand"));
    assert_eq!(s.next_gradient_preset_name(), "Gradient Preset 1");
    assert_eq!(s.execute("gradient.presets.save", &json!({})).unwrap()["name"], "Gradient Preset 1");
    let saved = &s.prefs.gradient_presets[0].gradient;
    assert_eq!((saved.kind, saved.method, saved.stops.len()), (GradientKind::Radial, GradientMethod::Perceptual, 3));
    assert_eq!((saved.stops[1].swatch.as_ref(), saved.stops[1].color), (None, Color::rgb8(0x12, 0x34, 0x56)));
    assert_eq!(s.active_saved_gradient_preset(), Some("Gradient Preset 1"), "the shown gradient is that preset");
    // Listed after the built-in ones, without a library.
    let list = s.execute("gradient.presets.list", &json!({})).unwrap();
    let last = list["presets"].as_array().unwrap().last().unwrap().clone();
    assert_eq!((last["name"].as_str(), last["builtIn"].as_bool(), last["library"].clone()), (Some("Gradient Preset 1"), Some(false), Value::Null));
    // Names are unique, any case, and never a built-in preset's.
    for (name, why) in [("gradient preset 1", "taken"), ("Sunset", "built in"), ("  ", "empty")] {
        assert!(s.execute("gradient.presets.save", &json!({"name": name})).is_err(), "{why}");
    }
    s.execute("paint.editGradient", &json!({"reverse": true})).unwrap();
    assert_eq!(s.execute("gradient.presets.save", &json!({"name": " Mine "})).unwrap()["name"], "Mine");
    // A saved preset applies to other art like a built-in one.
    let r = s.execute("shape.ellipse", &json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap();
    let other = NodeId(r["id"].as_u64().unwrap());
    s.execute("gradient.presets.apply", &json!({"name": "Mine"})).unwrap();
    assert_eq!(paint(&s, other, false).gradient, s.prefs.gradient_presets[1].gradient);
    // Delete: built-in ones stay; without a name, the saved preset the active proxy shows.
    assert!(s.execute("gradient.presets.delete", &json!({"name": "Sunset"})).is_err());
    assert!(s.execute("gradient.presets.delete", &json!({"name": "Nothing"})).is_err());
    assert_eq!(s.execute("gradient.presets.delete", &json!({})).unwrap()["deleted"], "Mine");
    assert!(s.execute("gradient.presets.delete", &json!({})).is_err(), "the ellipse's gradient is no saved preset now");
    assert_eq!(s.execute("gradient.presets.delete", &json!({"name": "GRADIENT PRESET 1"})).unwrap()["deleted"], "Gradient Preset 1");
    assert!((crate::find_command("gradient.presets.delete").unwrap().enabled)(&s).is_err(), "nothing left to delete");
}

#[test]
fn saving_needs_a_gradient_behind_the_proxy() {
    let (mut s, _) = session_with(json!({"color": "#336699"}));
    assert!((crate::find_command("gradient.presets.save").unwrap().enabled)(&s).is_err());
    assert!(s.execute("gradient.presets.save", &json!({})).is_err());
    // A gradient stroke is saved with the Stroke proxy in front.
    s.execute("paint.setStroke", &json!({"gradient": {"kind": "radial"}, "focus": false})).unwrap();
    assert!(s.execute("gradient.presets.save", &json!({})).is_err(), "the Fill is in front");
    s.execute("paint.toggleActive", &json!({"fill": false})).unwrap();
    assert_eq!(s.execute("gradient.presets.save", &json!({"name": "Ring"})).unwrap()["name"], "Ring");
    assert_eq!(s.prefs.gradient_presets[0].gradient.kind, GradientKind::Radial);
    assert!(s.execute("gradient.presets.save", &json!({"name": 7})).is_err(), "a name is text");
}

/// Save Gradient Preset is enabled exactly when saving works, with and without a gradient behind
/// the active proxy: the defaults with nothing selected, a fill or a stroke, a group's first
/// painted member, type and an Appearance panel row.
#[test]
fn the_save_check_matches_saving_with_and_without_a_gradient() {
    /// What the case is, how its session is made, and whether a gradient is behind the proxy.
    type Case = (&'static str, fn() -> Session, bool);
    let cases: [Case; 9] = [
        (
            "default solid paint",
            || {
                let mut s = Session::new();
                s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
                s
            },
            false,
        ),
        (
            "default gradient paint",
            || {
                let (mut s, _) = session_with(json!({"gradient": {}}));
                s.execute("select.none", &json!({})).unwrap();
                s
            },
            true,
        ),
        ("solid fill", || session_with(json!({"color": "#336699"})).0, false),
        ("gradient fill", || session_with(json!({"gradient": {"kind": "radial"}})).0, true),
        (
            "gradient fill, Stroke proxy in front",
            || {
                let (mut s, _) = session_with(json!({"gradient": {}}));
                s.execute("paint.toggleActive", &json!({"fill": false})).unwrap();
                s
            },
            false,
        ),
        (
            "gradient stroke, Stroke proxy in front",
            || {
                let (mut s, _) = session_with(json!({"color": "#336699"}));
                s.execute("paint.setStroke", &json!({"gradient": {}, "focus": false})).unwrap();
                s.execute("paint.toggleActive", &json!({"fill": false})).unwrap();
                s
            },
            true,
        ),
        (
            "group of a gradient and a solid",
            || {
                let (mut s, a) = session_with(json!({"gradient": {}}));
                let b = NodeId(s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap());
                s.execute("paint.setFill", &json!({"color": "#336699"})).unwrap();
                s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
                s.execute("object.group", &json!({})).unwrap();
                s
            },
            true,
        ),
        (
            "type with a gradient fill",
            || {
                let mut s = Session::new();
                s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
                s.execute("text.create", &json!({"x": 10, "y": 100, "text": "type"})).unwrap();
                s.execute("paint.setFill", &json!({"gradient": {}})).unwrap();
                s
            },
            true,
        ),
        (
            "a solid fill row active under a gradient fill",
            || {
                let (mut s, _) = session_with(json!({"color": "#336699"}));
                s.execute("appearance.addFill", &json!({})).unwrap();
                s.execute("paint.setFill", &json!({"gradient": {}, "item": null})).unwrap();
                let items = &s.doc().unwrap().doc.node(s.doc().unwrap().selection.objects[0]).unwrap().appearance.items;
                let solid = items
                    .iter()
                    .position(|i| matches!(i, vectorcraft_doc::AppearanceItem::Fill(f) if matches!(f.paint, Paint::Solid { .. })))
                    .unwrap();
                s.execute("appearance.setActiveItem", &json!({"index": solid})).unwrap();
                s
            },
            false,
        ),
    ];
    for (why, make, gradient) in cases {
        let mut s = make();
        let enabled = (crate::find_command("gradient.presets.save").unwrap().enabled)(&s).is_ok();
        assert_eq!((enabled, s.execute("gradient.presets.save", &json!({})).is_ok()), (gradient, gradient), "{why}");
    }
}

/// [`Node::proxy_paint_only`] picks the paint [`Node::proxy_paint`] does, for fills and strokes,
/// with and without an Appearance panel row, on paths with several fills, type, an empty path
/// and a colorized object.
#[test]
fn the_unmeasured_proxy_paint_is_the_measured_one() {
    let (mut s, id) = session_with(json!({"gradient": {}}));
    s.execute("appearance.addFill", &json!({})).unwrap();
    s.execute("appearance.addStroke", &json!({})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#ff0000"})).unwrap();
    let plain = NodeId(s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap());
    let colorized = NodeId(s.execute("shape.rectangle", &json!({"x": 40, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap());
    s.edit("Colorize", |d, _| {
        d.node_mut(colorized).unwrap().extras_mut().colorize = Some(Box::new(Paint::solid(Color::rgb8(255, 0, 0))));
        Ok(())
    })
    .unwrap();
    let text = NodeId(s.execute("text.create", &json!({"x": 10, "y": 200, "text": "type"})).unwrap()["id"].as_u64().unwrap());
    let d = &s.doc().unwrap().doc;
    let mut nodes: Vec<vectorcraft_doc::Node> = [id, plain, colorized, text].iter().map(|i| d.node(*i).unwrap().clone()).collect();
    nodes.push(vectorcraft_doc::Node::path(NodeId(9999), Default::default(), (*d.node(plain).unwrap().appearance).clone()));
    let mut runless = d.node(text).unwrap().clone();
    if let vectorcraft_doc::NodeKind::Text(t) = &mut runless.kind {
        t.runs.clear();
    }
    nodes.push(runless);
    for n in &nodes {
        for stroke in [false, true] {
            for item in [None, Some(0), Some(1), Some(2), Some(3), Some(9)] {
                assert_eq!(n.proxy_paint_only(stroke, item), n.proxy_paint(stroke, item).map(|p| p.0), "{:?} stroke {stroke} item {item:?}", n.id);
            }
        }
    }
}

/// On objects made from a fixed seed (paths, an empty path, type with and without runs, each
/// with zero to four fills and strokes of none, a solid color or a gradient, some colorized),
/// [`Node::proxy_paint_only`] picks the paint [`Node::proxy_paint`] does for every row.
#[test]
fn the_unmeasured_proxy_paint_is_the_measured_one_on_varied_objects() {
    let (mut s, rect) = session_with(json!({"gradient": {}}));
    let gradient = s.doc().unwrap().doc.node(rect).unwrap().appearance.paint_at(None, true).unwrap().clone();
    let text = NodeId(s.execute("text.create", &json!({"x": 10, "y": 200, "text": "type"})).unwrap()["id"].as_u64().unwrap());
    let d = &s.doc().unwrap().doc;
    let mut runless = d.node(text).unwrap().clone();
    if let vectorcraft_doc::NodeKind::Text(t) = &mut runless.kind {
        t.runs.clear();
    }
    let empty = vectorcraft_doc::Node::path(NodeId(9999), Default::default(), Default::default());
    let bases = [d.node(rect).unwrap().clone(), empty, d.node(text).unwrap().clone(), runless];
    let mut seed: u64 = 0x9a1d_5eed_c0de;
    let mut next = |n: u64| {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) % n
    };
    let (mut checked, mut some) = (0, 0);
    for _ in 0..200 {
        let mut n = bases[next(4) as usize].clone();
        let paints = [Paint::None, Paint::solid(Color::rgb8(next(256) as u8, 0, 0)), gradient.clone()];
        let mut look = vectorcraft_doc::Appearance::default();
        for _ in 0..next(5) {
            let p = paints[next(3) as usize].clone();
            look.items.push(if next(2) == 0 {
                vectorcraft_doc::AppearanceItem::Fill(vectorcraft_doc::FillLayer::new(p))
            } else {
                vectorcraft_doc::AppearanceItem::Stroke(vectorcraft_doc::StrokeLayer::new(p, 1.0 + next(5) as f64))
            });
        }
        n.appearance = look.into();
        if next(4) == 0 {
            n.extras_mut().colorize = Some(Box::new(paints[next(3) as usize].clone()));
        }
        for stroke in [false, true] {
            for item in [None, Some(0), Some(1), Some(2), Some(3), Some(4)] {
                let measured = n.proxy_paint(stroke, item).map(|p| p.0);
                assert_eq!(n.proxy_paint_only(stroke, item), measured, "{n:?} stroke {stroke} item {item:?}");
                checked += 1;
                some += usize::from(measured.is_some());
            }
        }
    }
    assert!(some > checked / 4 && some < checked, "both answers come up: {some} of {checked} found a paint");
}

#[test]
fn saved_presets_round_trip_with_the_preferences() {
    let (mut s, _) = session_with(json!({"gradient": {}}));
    s.execute("gradient.presets.save", &json!({"name": "Mine"})).unwrap();
    let json = s.prefs.to_json();
    assert_eq!(json["gradientPresets"][0]["name"], "Mine");
    let back: Prefs = serde_json::from_value(json).unwrap();
    assert_eq!(back, s.prefs);
    // Without saved presets nothing is written, and older preference files load with none.
    assert!(Prefs::default().to_json().get("gradientPresets").is_none());
    let old: Prefs = serde_json::from_value(json!({"keyboardIncrement": 2})).unwrap();
    assert!(old.gradient_presets.is_empty());
    // Resetting the preferences keeps them.
    s.execute("prefs.reset", &json!({})).unwrap();
    assert_eq!(s.prefs.gradient_presets.len(), 1);
    // A restarted session (preferences applied from the saved file) lists them again.
    let mut fresh = Session::new();
    fresh.apply_prefs(back);
    assert!(fresh.gradient_preset("mine").is_some_and(|e| e.library.is_none()));
}
