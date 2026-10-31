//! Every-command sweep: each registered command, in three fixture states, with `{}` and with fuzzed
//! junk params, must return Ok or Err (never panic), never leave an interaction open, and never
//! corrupt the tree.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use vectorcraft_engine::{Session, command_specs};
use vectorcraft_testkit::catch_quiet;
use vectorcraft_testkit::fixtures::Fixture;
use vectorcraft_testkit::invariants::{check_all, check_session, doc_json};
use vectorcraft_testkit::strategies::{junk_params, junk_values, param_keys};

/// Commands that replace/close the active document or touch the filesystem are fine to call, but a
/// few need their paths redirected into a temp dir (handled by `junk_params`).
fn safe_path() -> String {
    vectorcraft_testkit::temp_dir("sweep").join("out.bin").to_string_lossy().to_string()
}

/// `r`, after failing the case if the command panicked. The engine's guard reports a panic as
/// `EngineError::Internal`.
fn no_panic(r: Result<Value, vectorcraft_engine::EngineError>) -> Result<Value, vectorcraft_engine::EngineError> {
    if let Err(vectorcraft_engine::EngineError::Internal { cmd, msg }) = &r {
        panic!("{cmd} panicked: {msg}");
    }
    r
}

/// Run one call; returns a failure description or None.
fn probe(fx: Fixture, id: &str, params: &Value) -> Option<String> {
    let mut s = fx.session();
    let r = catch_quiet(|| no_panic(s.execute(id, params)));
    match r {
        Err(msg) => Some(format!("PANIC {id} {params} [{fx:?}]: {msg}")),
        Ok(_) => {
            if s.in_interaction() {
                return Some(format!("{id} {params} [{fx:?}]: left an interaction open"));
            }
            check_session(&s).err().map(|e| format!("{id} {params} [{fx:?}]: {e}"))
        }
    }
}

#[test]
fn registry_is_well_formed() {
    let specs = command_specs();
    assert!(specs.len() > 150, "only {} commands", specs.len());
    let mut ids = std::collections::HashSet::new();
    for c in specs {
        assert!(ids.insert(c.id), "duplicate command id {}", c.id);
        assert!(!c.label.is_empty(), "{} has no label", c.id);
        assert!(c.params.starts_with('{') || c.params.starts_with("same as"), "{}: params doc `{}`", c.id, c.params);
        assert!(c.id.contains('.'), "{}: ids are namespaced", c.id);
    }
    // Every command reports enablement without a document.
    let s = Session::new();
    for info in s.commands() {
        assert_eq!(info.enabled, info.disabled_reason.is_none(), "{}", info.id);
    }
}

#[test]
fn every_command_with_empty_params_in_every_fixture() {
    let mut failures = vec![];
    for fx in Fixture::ALL {
        for c in command_specs() {
            if let Some(f) = probe(fx, c.id, &json!({})) {
                failures.push(f);
            }
        }
    }
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn every_command_without_a_document() {
    let mut failures = vec![];
    for c in command_specs() {
        let mut s = Session::new();
        match catch_quiet(|| no_panic(s.execute(c.id, &json!({})))) {
            Err(p) => failures.push(format!("PANIC {} with no document: {p}", c.id)),
            // Anything that needs a document must say so via enablement, not fail later.
            Ok(Err(vectorcraft_engine::EngineError::NoDocument)) => failures.push(format!("{}: enabled without a document but needs one", c.id)),
            Ok(_) => {}
        }
        if s.in_interaction() {
            failures.push(format!("{}: interaction left open", c.id));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Regression: huge column counts used to panic with `capacity overflow`; in every layout and
/// order (#681), with huge and negative spacing too.
#[test]
fn artboard_rearrange_huge_columns() {
    for fx in Fixture::ALL {
        for layout in ["gridByRow", "gridByColumn", "row", "column"] {
            for order in ["leftToRight", "rightToLeft"] {
                for (cols, spacing) in [(json!(u64::MAX), json!(1e308)), (json!(1e308), json!(-1e308)), (json!(0), json!(0)), (json!(-5), json!(20))]
                {
                    let p = json!({ "layout": layout, "order": order, "columns": cols, "spacing": spacing });
                    assert_eq!(probe(fx, "artboard.rearrange", &p), None);
                }
            }
        }
    }
}

#[test]
fn bug_huge_transform_makes_document_unloadable() {
    for (id, p) in [
        ("object.scale", json!({"sx": 1e308})),
        ("object.transformEach", json!({"scaleH": 1e308})),
        ("object.distributeSpacing", json!({"spacing": 1e308})),
    ] {
        let mut s = Fixture::Multi.session();
        if no_panic(s.execute(id, &p)).is_ok() {
            vectorcraft_testkit::invariants::check_native_roundtrip(&s.doc().unwrap().doc).unwrap_or_else(|e| panic!("{id} {p}: {e}"));
        }
    }
}

/// Distribute with a huge spacing from each of the three selected objects as the key object leaves
/// a document that saves and opens again.
#[test]
fn distribute_with_a_huge_spacing_from_the_key_object() {
    for key in 0..3 {
        for p in [json!({"horizontal": "left", "spacing": 1e308}), json!({"vertical": "bottom", "spacing": -1e308})] {
            let mut s = Fixture::Multi.session();
            let id = s.doc().unwrap().selection.objects[key];
            s.execute("select.key", &json!({"id": id.0})).unwrap();
            if no_panic(s.execute("object.distribute", &p)).is_ok() {
                check_session(&s).unwrap_or_else(|e| panic!("key {key}, {p}: {e}"));
                vectorcraft_testkit::invariants::check_native_roundtrip(&s.doc().unwrap().doc).unwrap_or_else(|e| panic!("key {key}, {p}: {e}"));
            }
        }
    }
}

#[test]
fn distribute_with_a_huge_spacing_from_a_key_guide() {
    for (vertical, pos) in [(true, 50.0), (true, 10000.0), (false, -10000.0)] {
        for p in [json!({"horizontal": "left", "spacing": 1e308}), json!({"vertical": "bottom", "spacing": -1e308})] {
            let mut s = Fixture::Multi.session();
            s.execute("guide.add", &json!({"vertical": vertical, "pos": pos})).unwrap();
            s.execute("guide.select", &json!({"indexes": [0], "toggle": true})).unwrap();
            s.execute("select.key", &json!({"guide": 0})).unwrap();
            if no_panic(s.execute("object.distribute", &p)).is_ok() {
                check_session(&s).unwrap_or_else(|e| panic!("guide at {pos}, {p}: {e}"));
                vectorcraft_testkit::invariants::check_native_roundtrip(&s.doc().unwrap().doc).unwrap_or_else(|e| panic!("guide at {pos}, {p}: {e}"));
            }
        }
    }
}

#[test]
fn distribute_guides_selected_alone_with_a_huge_spacing() {
    for (cmd, p) in [
        ("object.distributeSpacing", json!({"axis": "horizontal", "spacing": 1e308})),
        ("object.distributeSpacing", json!({"axis": "vertical", "spacing": -1e308})),
        ("object.distribute", json!({"horizontal": "center", "to": "artboard", "artboard": 99})),
    ] {
        let mut s = Fixture::Empty.session();
        for (v, pos) in [(true, 10.0), (true, 20.0), (true, 9000.0), (false, -9000.0), (false, 0.0)] {
            s.execute("guide.add", &json!({"vertical": v, "pos": pos})).unwrap();
        }
        s.execute("guide.select", &json!({"indexes": [0, 1, 2, 3, 4]})).unwrap();
        if no_panic(s.execute(cmd, &p)).is_ok() {
            check_session(&s).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"));
            vectorcraft_testkit::invariants::check_native_roundtrip(&s.doc().unwrap().doc).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"));
        }
    }
}

#[test]
fn bug_artboard_huge_props_break_svg() {
    for key in ["x", "y", "width", "height"] {
        let mut s = Fixture::Single.session();
        let p = json!({"index": 0, key: 1e308});
        if no_panic(s.execute("artboard.setProps", &p)).is_ok() {
            vectorcraft_testkit::invariants::check_svg_roundtrip(&s.doc().unwrap().doc).unwrap_or_else(|e| panic!("{p}: {}", &e[..e.len().min(200)]));
        }
    }
}

/// Commands too slow to fuzz exhaustively get a reduced junk set (they still run).
fn heavy(id: &str) -> bool {
    id.starts_with("document.serialize") || id.starts_with("document.export") || id == "document.open"
}

#[test]
fn every_command_with_junk_params() {
    let path = safe_path();
    let mut failures = vec![];
    let mut calls = 0usize;
    for fx in Fixture::ALL {
        for c in command_specs() {
            let mut cases = junk_params(c.params, &path);
            if heavy(c.id) {
                cases.truncate(12);
            }
            for p in &cases {
                calls += 1;
                if let Some(f) = probe(fx, c.id, p) {
                    failures.push(f);
                }
            }
        }
    }
    assert!(calls > 5_000, "only {calls} calls");
    failures.sort();
    failures.dedup();
    assert!(
        failures.is_empty(),
        "{} failures (of {calls} calls):\n{}",
        failures.len(),
        failures.iter().take(60).cloned().collect::<Vec<_>>().join("\n")
    );
}

/// Junk inside the obvious structured params: ids arrays, anchors, matrices, colours, stops.
#[test]
fn structured_junk() {
    let cases: Vec<(&str, Value)> = vec![
        ("select.set", json!({"ids": [0, u64::MAX, -1, "x", null, 1.5]})),
        ("document.export", json!({"format": "png", "rect": [1e308, -1e308, 1e308, 1e308]})),
        ("document.export", json!({"format": "png", "rect": [0, 0, 1e300, 1e300], "scale": 64})),
        ("document.export", json!({"format": "tiff", "rect": [null, "x", -1, 0.0000001], "useArtboards": true})),
        ("document.export", json!({"format": "bmp", "rect": {"x": 1}, "artboard": 3})),
        ("document.export", json!({"format": "svg", "rect": [0, 0, 10, 10]})),
        ("select.add", json!({"ids": [[1], {"a": 1}]})),
        ("select.anchors", json!({"id": 2, "anchors": [[99, 99], [-1, 0], ["a", "b"]], "mode": "set"})),
        ("select.anchorsMany", json!({"items": [{"id": 2, "anchors": [[0, 1000]]}, null, 5]})),
        ("select.anchorsMany", json!({"items": [{"id": 2, "anchors": [[0, 1000], [0, 0]]}, {"id": u64::MAX}], "mode": "toggle"})),
        ("select.anchorsMany", json!({"items": [{"id": 2, "anchors": [[0, 0]]}], "mode": "subtract"})),
        ("select.anchorsMany", json!({"items": [], "mode": 7})),
        ("select.toggle", json!({"ids": [2, u64::MAX, "x"]})),
        ("object.transform", json!({"matrix": [0, 0, 0, 0, 0, 0]})),
        ("object.transform", json!({"matrix": [1e308, 1e308, 1e308, 1e308, 1e308, 1e308]})),
        ("object.transform", json!({"matrix": [1, 2, 3]})),
        ("object.scale", json!({"sx": 0, "sy": 0})),
        ("object.scale", json!({"sx": -100, "sy": 1e308})),
        ("object.rotate", json!({"angle": 1e308, "origin": [1e308, -1e308]})),
        ("object.distort", json!({"corners": [[0, 0], [0, 0], [0, 0], [0, 0]]})),
        ("object.distort", json!({"corners": [[0, 0]], "from": [0, 0, 0, 0]})),
        ("path.create", json!({"anchors": []})),
        ("path.create", json!({"anchors": [{"x": 1e308, "y": -1e308}, {"x": 0, "y": 0, "in": [1e308, 1e308]}], "closed": true})),
        ("path.create", json!({"d": "M 0 0 L"})),
        ("path.create", json!({"d": "M0 0 C 1e308 1e308 -1e308 -1e308 0 0 Z"})),
        ("path.create", json!({"d": "Z Z Z m 1 1 z"})),
        ("path.setAnchors", json!({"id": 2, "subpaths": [{"anchors": [], "closed": true}]})),
        ("path.setAnchors", json!({"id": 1, "subpaths": [{"anchors": [{"x": 0, "y": 0}], "closed": true}]})),
        ("path.insertAnchor", json!({"id": 2, "subpath": 0, "segment": 99, "t": 5})),
        ("path.insertAnchor", json!({"id": 2, "subpath": 7, "segment": 0, "t": -1})),
        ("path.setHandle", json!({"id": 2, "subpath": 0, "anchor": 100, "which": "in", "x": 0, "y": 0})),
        ("path.moveAnchor", json!({"id": 2, "subpath": 0, "anchor": 100, "x": 0, "y": 0})),
        ("path.moveAnchor", json!({"id": 2, "subpath": 0, "anchor": 0, "x": 1e308, "y": -1e308})),
        ("path.removeAnchor", json!({"id": 2, "subpath": 0, "anchor": 0})),
        ("path.split", json!({"id": 2, "subpath": 0, "segment": 1000, "t": 0.5})),
        ("path.reshapeSegment", json!({"id": 2, "subpath": 0, "segment": 0, "t": 2.0, "dx": 1e308, "dy": 0})),
        ("path.freehand", json!({"points": [[0, 0]]})),
        ("path.freehand", json!({"points": [[0, 0], [0, 0], [0, 0], [0, 0]]})),
        ("path.freehand", json!({"points": [[1e308, 1e308], [-1e308, 0]], "fidelity": 0})),
        ("path.freehand", json!({"points": [[0, 0, -5], [10, 0, 1e308], [20, 0, "x"], [30, 0, null], [40, 0, 0.5]], "style": "brush"})),
        ("path.freehand", json!({"points": [[0, 0, 0.2], [0, 0, 0.9], [0, 0, 0.1]]})),
        (
            "path.freehand",
            json!({"points": [{"x": 0, "y": 0, "pressure": 2}, {"x": 1e308, "y": 0, "pressure": -1}], "extend": {"id": 2, "end": "start"}}),
        ),
        ("brush.freehand", json!({"points": [[0, 0, 0], [50, 50, 1]], "brush": "3 pt. Round"})),
        ("brush.options", json!({"name": "3 pt. Round", "params": {"modes": ["pressure", "random", "nope"]}})),
        ("brush.options", json!({"name": "3 pt. Round", "params": {"modes": "pressure", "variation": [1e308, -1e308, 1e308]}})),
        (
            "brush.options",
            json!({"name": "3 pt. Round", "params": {"modes": ["pressure", "pressure", "pressure"], "variation": [1e308, 1e308, 1e308], "size": 1e308, "angle": -1e308}}),
        ),
        (
            "brush.options",
            json!({"name": "Confetti", "params": {"size": [1e308, -1e308], "spacing": [0, 1e308], "scatter": [-1e308, 1e308], "rotation": [1e308, -1e308], "modes": ["pressure", "random", "fixed", "pressure"]}}),
        ),
        ("brush.options", json!({"name": "Dots", "params": {"modes": ["fixed", "tilt", "fixed", "fixed"], "size": "x"}})),
        ("brush.options", json!({"name": "Dots", "params": {"modes": ["fixed"], "colorization": {"method": "sparkle"}}})),
        (
            "brush.options",
            json!({"name": "Dots", "params": {"colorization": {"method": "hueShift", "key": {"model": "rgb", "r": 1e308, "g": -1e308, "b": 0}}}}),
        ),
        ("brush.options", json!({"name": "Arrow", "params": {"width": 1e308, "scale": {"mode": "betweenGuides", "start": 1e308, "end": -1e308}}})),
        ("brush.options", json!({"name": "Arrow", "params": {"width": -5, "direction": "diagonal", "flip_along": "yes"}})),
        ("brush.options", json!({"name": "Chain", "params": {"scale": 1e308, "spacing": -1e308, "fit": "approximate", "flip_across": true}})),
        ("brush.options", json!({"name": "Chain", "params": {"scale": 0, "spacing": 1e308, "fit": "nope", "side": null}})),
        ("path.curvature", json!({"points": [{"x": 0, "y": 0}]})),
        ("path.curvatureEdit", json!({"id": 2, "op": "move", "anchor": 1000, "x": 0, "y": 0})),
        ("path.curvatureEdit", json!({"id": 2, "op": "move", "anchor": 0, "x": 1e308, "y": -1e308})),
        ("path.curvatureEdit", json!({"id": 2, "subpath": 7, "op": "extend", "x": 1e308, "y": -1e308, "from": "smooth"})),
        ("path.curvatureEdit", json!({"id": 2, "op": "insert", "segment": 99, "t": -5, "x": 1})),
        ("path.curvatureEdit", json!({"id": 2, "op": "close", "end": "start", "from": "corner"})),
        ("path.knife", json!({"points": [[0, 0], [1e9, 1e9]]})),
        ("path.eraseRegion", json!({"points": [[100, 80]], "size": 0})),
        ("path.eraseRegion", json!({"points": [[100, 80], [101, 80]], "size": 1e9})),
        ("path.blob", json!({"points": [[0, 0], [0, 0]], "size": -5})),
        ("path.smoothRegion", json!({"points": [[100, 80], [120, 90]], "radius": 1e308})),
        ("paint.setFill", json!({"gradient": {"stops": []}})),
        ("paint.setFill", json!({"gradient": {"stops": [{"offset": 0.5, "color": "#ff0000"}]}})),
        (
            "paint.setFill",
            json!({"gradient": {"kind": "radial", "stops": [{"offset": -5, "color": [1, 0, 0]}, {"offset": 99, "color": {"gray": 50}}], "angle": 1e308}}),
        ),
        ("paint.setFill", json!({"color": [1e308, -1e308, 2]})),
        ("paint.setFill", json!({"color": {"c": 200, "m": -1, "y": 0, "k": 1e9}})),
        ("paint.setGradientGeom", json!({"start": [0, 0], "end": [0, 0]})),
        ("stroke.set", json!({"weight": -5, "dash": [0, 0, 0], "miterLimit": -1})),
        ("stroke.set", json!({"weight": 1e308, "dash": [-1, -2], "dashOffset": 1e308})),
        ("stroke.set", json!({"dash": [1e-12]})),
        ("document.inspect", json!({"ids": [u64::MAX, 0, 1e308], "kind": "", "limit": u64::MAX, "offset": u64::MAX})),
        ("document.inspect", json!({"ids": null, "kind": {"a": 1}, "limit": -1, "offset": "x", "depth": u64::MAX})),
        ("transparency.set", json!({"opacity": -50})),
        ("transparency.set", json!({"opacity": 1e308, "blend": "NoSuchBlend"})),
        ("perspective.move", json!({"from": [0, 0], "to": [1e308, -1e308], "perpendicular": true, "copy": true, "plane": "ground"})),
        ("perspective.move", json!({"from": [f64::MAX, 0], "to": [0, 0], "plane": "left"})),
        ("perspective.transform", json!({"matrix": [1e308, 0, 0, 1e308, 0, 0], "plane": "right"})),
        ("perspective.transform", json!({"matrix": [1, 0, 0, 1, 0, 0], "depth": -1e9, "plane": "left", "copy": true})),
        ("perspective.transform", json!({"matrix": [1, 0, 0, 1, 1e300, 0], "plane": "left"})),
        ("perspective.nudge", json!({"dx": 1e308, "dy": -1e308, "big": true, "copy": true})),
        ("perspective.plane.move", json!({"plane": "ground", "offset": 1e308, "objects": "copy"})),
        ("perspective.plane.move", json!({"plane": "left", "by": -1e7, "objects": "move"})),
        ("perspective.plane.move", json!({"plane": "none", "offset": 0})),
        ("perspective.plane.matchObject", json!({"id": u64::MAX})),
        ("perspective.grid.set", json!({"leftOffset": -1e12, "reproject": true})),
        ("perspective.grid.set", json!({"vpRight": -1e6, "reproject": true})),
        ("perspective.editText", json!({"id": u64::MAX})),
        (
            "perspective.draw",
            json!({"command": "shape.rectangle", "params": {"x": 1, "y": 1, "width": 10, "height": 10}, "at": [1e308, -1e308], "plane": "left"}),
        ),
        ("perspective.draw", json!({"command": "shape.flare", "params": {"cx": 0, "cy": 0}, "plane": "ground"})),
        ("object.setProps", json!({"opacity": -1, "blend": 5})),
        ("effect.apply", json!({"effect": "distort.roughen", "params": {"size": 100, "detail": 100, "relative": false}})),
        ("effect.apply", json!({"effect": "distort.zigZag", "params": {"size": 1e308, "ridges": 100}})),
        ("effect.apply", json!({"effect": "distort.transform", "params": {"copies": 1e308}})),
        ("effect.apply", json!({"effect": "stylize.dropShadow", "params": {"blur": 1e308, "opacity": -1}})),
        ("effect.apply", json!({"effect": "blur.radial", "params": {"amount": 1e308, "method": 5, "quality": "best"}})),
        ("effect.apply", json!({"effect": "blur.smart", "params": {"radius": 1e308, "threshold": -1e308, "quality": null}})),
        ("effect.apply", json!({"effect": "sharpen.unsharpMask", "params": {"amount": -1, "radius": "1e999", "threshold": 1e308}})),
        (
            "effect.apply",
            json!({"effect": "brushStrokes.accentedEdges", "params": {"edgeWidth": 1e308, "edgeBrightness": "NaN", "smoothness": -1e308}}),
        ),
        (
            "effect.apply",
            json!({"effect": "brushStrokes.angledStrokes", "params": {"directionBalance": [1], "strokeLength": 1e308, "sharpness": null}}),
        ),
        ("effect.apply", json!({"effect": "brushStrokes.crosshatch", "params": {"strokeLength": "1e999", "sharpness": 1e308, "strength": -1e308}})),
        ("effect.apply", json!({"effect": "brushStrokes.darkStrokes", "params": {"balance": -1e308, "blackIntensity": {}, "whiteIntensity": 1e308}})),
        (
            "effect.apply",
            json!({"effect": "brushStrokes.inkOutlines", "params": {"strokeLength": -1, "darkIntensity": 1e308, "lightIntensity": "x"}}),
        ),
        ("effect.apply", json!({"effect": "brushStrokes.spatter", "params": {"sprayRadius": 1e308, "smoothness": "NaN"}})),
        (
            "effect.apply",
            json!({"effect": "brushStrokes.sprayedStrokes", "params": {"strokeLength": 1e308, "sprayRadius": -1e308, "strokeDirection": 3}}),
        ),
        ("effect.apply", json!({"effect": "brushStrokes.sumiE", "params": {"strokeWidth": 0, "strokePressure": 1e308, "contrast": [1]}})),
        ("effect.apply", json!({"effect": "pixelate.colorHalftone", "params": {"maxRadius": 1e308, "channel1": "1e999", "channel4": null}})),
        ("effect.apply", json!({"effect": "pixelate.crystallize", "params": {"cellSize": -1e308}})),
        ("effect.apply", json!({"effect": "pixelate.mezzotint", "params": {"type": 5}})),
        ("effect.apply", json!({"effect": "pixelate.pointillize", "params": {"cellSize": "NaN"}})),
        ("effect.apply", json!({"effect": "artistic.smudgeStick", "params": {"strokeLength": 1e308, "highlightArea": "NaN", "intensity": [1]}})),
        ("effect.apply", json!({"effect": "artistic.sponge", "params": {"brushSize": -1e308, "definition": 1e308, "smoothness": null}})),
        ("effect.apply", json!({"effect": "artistic.watercolor", "params": {"brushDetail": "1e999", "shadowIntensity": -1, "texture": 1e308}})),
        ("effect.apply", json!({"effect": "artistic.cutout", "params": {"levels": 1e308, "edgeSimplicity": "NaN", "edgeFidelity": -1e308}})),
        ("effect.apply", json!({"effect": "artistic.dryBrush", "params": {"brushSize": -1e308, "brushDetail": "NaN", "texture": 1e308}})),
        ("effect.apply", json!({"effect": "artistic.neonGlow", "params": {"glowSize": -1e308, "glowBrightness": "1e999", "color": [1]}})),
        ("effect.apply", json!({"effect": "artistic.posterEdges", "params": {"edgeThickness": 1e308, "edgeIntensity": [], "posterization": -7}})),
        (
            "effect.apply",
            json!({"effect": "artistic.roughPastels", "params": {"strokeLength": 1e308, "strokeDetail": "NaN", "texture": -1e308, "scaling": "bad", "lightDirection": "nope"}}),
        ),
        ("effect.apply", json!({"effect": "sketch.chrome", "params": {"detail": -1e308, "smoothness": "NaN"}})),
        (
            "effect.apply",
            json!({"effect": "sketch.conteCrayon", "params": {"foregroundLevel": "NaN", "backgroundLevel": 1e308, "texture": [1], "scaling": -1e308, "relief": "1e999", "lightDirection": 3, "invert": "yes"}}),
        ),
        ("effect.apply", json!({"effect": "sketch.graphicPen", "params": {"strokeLength": -1e308, "lightDarkBalance": "NaN", "strokeDirection": 3}})),
        ("effect.apply", json!({"effect": "sketch.halftonePattern", "params": {"size": -1e308, "contrast": "NaN", "patternType": 3}})),
        ("effect.apply", json!({"effect": "sketch.basRelief", "params": {"detail": "NaN", "smoothness": 1e308, "lightDirection": 3}})),
        ("effect.apply", json!({"effect": "sketch.charcoal", "params": {"thickness": 1e308, "detail": -1e308, "lightDarkBalance": [1]}})),
        ("effect.apply", json!({"effect": "sketch.photocopy", "params": {"detail": "1e999", "darkness": null}})),
        ("effect.apply", json!({"effect": "sketch.stamp", "params": {"lightDarkBalance": -1e308, "smoothness": "NaN"}})),
        ("effect.apply", json!({"effect": "sketch.reticulation", "params": {"density": "NaN", "foregroundLevel": -1e308, "backgroundLevel": 1e308}})),
        ("effect.apply", json!({"effect": "texture.craquelure", "params": {"crackSpacing": -1e308, "crackDepth": "NaN", "crackBrightness": 1e308}})),
        ("effect.apply", json!({"effect": "texture.grain", "params": {"intensity": 1e308, "contrast": [1], "grainType": 7}})),
        ("effect.apply", json!({"effect": "texture.mosaicTiles", "params": {"tileSize": "1e999", "groutWidth": -1, "lightenGrout": null}})),
        ("effect.apply", json!({"effect": "texture.patchwork", "params": {"squareSize": 1e308, "relief": -1e308}})),
        ("effect.apply", json!({"effect": "texture.stainedGlass", "params": {"cellSize": 0, "borderThickness": 1e308, "lightIntensity": "x"}})),
        (
            "effect.apply",
            json!({"effect": "texture.texturizer", "params": {"texture": {}, "scaling": -1e308, "relief": 1e308, "lightDirection": 3, "invert": "yes"}}),
        ),
        ("effect.apply", json!({"effect": "video.deinterlace", "params": {"eliminate": 7, "create": ["x"]}})),
        ("effect.apply", json!({"effect": "video.ntscColors", "params": {"junk": 1e308}})),
        ("effect.apply", json!({"effect": "distort.diffuseGlow", "params": {"graininess": "x", "glowAmount": 1e308, "clearAmount": -1e308}})),
        ("effect.apply", json!({"effect": "distort.glass", "params": {"distortion": 1e308, "smoothness": "NaN", "texture": 3, "scaling": -1}})),
        ("effect.apply", json!({"effect": "distort.oceanRipple", "params": {"rippleSize": [1], "rippleMagnitude": 1e308}})),
        ("effect.apply", json!({"effect": "addon.color.halftone", "params": {"frequency": 1e308, "angle": -1e308, "shape": "star", "mode": "rgb"}})),
        ("effect.apply", json!({"effect": "addon.color.halftone", "params": {"frequency": 300, "invert": true, "clip": false, "fromArt": true}})),
        ("effect.apply", json!({"effect": "addon.color.halftone", "params": {"frequency": -1e308, "mode": "cmyk", "color": "#123456", "target": 3}})),
        ("effect.apply", json!({"effect": "addon.color.duotone", "params": {"tones": "x", "skipSpot": "yes"}})),
        (
            "effect.apply",
            json!({"effect": "addon.color.duotone", "params": {"tones": [1, null, {"color": [1e308, -1e308, "x"], "curve": [[1e308, -1e308], ["a"]]}]}}),
        ),
        (
            "effect.apply",
            json!({"effect": "addon.color.duotone", "params": {"tones": [{"swatch": 3, "curve": "1e999,5 x,y", "active": "no"}, {"color": {"model": "lab", "l": 1e30, "a": -1e30, "b": 0}}, {}, {}, {}, {}]}}),
        ),
        (
            "addon.color.filter",
            json!({"effect": "addon.color.duotone", "params": {"tones": [{"color": "#ff0000", "swatch": "[Registration]"}], "skipSpot": true}}),
        ),
        ("effect.apply", json!({"effect": "addon.path.dashes", "params": {"dash1": 1e308, "gap1": 1e308, "offset": -1e308, "alignCorners": true}})),
        ("effect.apply", json!({"effect": "addon.path.dashes", "params": {"dash1": -1e308, "gap1": 1e-308, "dash5": 1e-9, "gap5": 0}})),
        ("effect.apply", json!({"effect": "addon.path.addAnchors", "params": {"target": "x", "spacing": "x", "count": 1e308, "maxCount": 1e308}})),
        ("effect.apply", json!({"effect": "addon.path.addAnchors", "params": {"spacing": "distance", "distance": 1e-308, "maxCount": -1e308}})),
        ("effect.apply", json!({"effect": "addon.path.extend", "params": {"startLength": 1e308, "endLength": -1e308, "startPercent": -1e308}})),
        ("effect.apply", json!({"effect": "addon.path.extend", "params": {"endPercent": 1e308, "endShape": "arc", "startShape": "x"}})),
        ("effect.apply", json!({"effect": "addon.path.close", "params": {"mode": "x"}})),
        ("effect.apply", json!({"effect": "addon.path.smooth", "params": {"ratio": 1e308}})),
        ("effect.apply", json!({"effect": "addon.path.retract", "params": {"handles": "shorter", "length": 1e308}})),
        ("effect.apply", json!({"effect": "addon.path.reverse"})),
        ("effect.apply", json!({"effect": "addon.path.simplify", "params": {"tolerance": 1e308, "protectCorners": false}})),
        ("effect.apply", json!({"effect": "addon.path.split", "params": {"anchors": "x"}})),
        ("addon.color.printFixes", json!({"inkLimit": 1e308, "overprintBlack": -1e308})),
        ("addon.color.printFixes", json!({"inkLimit": 1, "overprintBlack": 1, "removeWhiteOverprint": true, "scope": "document", "textOnly": true})),
        ("addon.color.printFixes", json!({"inkLimit": [300], "scope": 3, "ids": "x"})),
        ("effect.apply", json!({"effect": "no.such.effect"})),
        ("effect.remove", json!({"index": 99})),
        ("effect.setParams", json!({"index": 0, "params": null})),
        ("object.path.offsetPath", json!({"offset": 1e308})),
        ("object.path.offsetPath", json!({"offset": -1e308})),
        ("object.path.simplify", json!({"tolerance": -1, "cornerAngle": 1e308})),
        ("object.path.simplify", json!({"level": 1e308, "tolerance": "x"})),
        ("object.path.simplify", json!({"level": -1e308, "auto": "yes"})),
        ("object.path.simplify", json!({"auto": true, "level": [1]})),
        (
            "shape.rectangularGrid",
            json!({"x": 0, "y": 0, "width": 1e308, "height": -1e308, "rows": 999, "columns": u64::MAX, "rowSkew": 1e308, "columnSkew": -1e308, "frame": false, "fill": true}),
        ),
        ("shape.rectangularGrid", json!({"x": 0, "y": 0, "width": 0, "height": 0, "rowSkew": "x", "frame": 3})),
        (
            "shape.polarGrid",
            json!({"x": 0, "y": 0, "width": 1e308, "height": 1e-308, "concentric": 999, "radial": 999, "concentricSkew": -1e308, "radialSkew": 1e308, "compound": true, "fill": true}),
        ),
        (
            "shape.polarGrid",
            json!({"x": 0, "y": 0, "width": 10, "height": 10, "concentric": -1, "radial": 0.5, "radialSkew": null, "compound": "yes"}),
        ),
        ("object.path.splitIntoGrid", json!({"rows": 500, "columns": 1, "gutter": 1e308})),
        ("object.path.splitIntoGrid", json!({"rows": 0.4})),
        ("object.setBounds", json!({"width": 0, "height": 0})),
        ("object.setBounds", json!({"width": -10, "height": 1e308, "reference": 99})),
        ("object.distributeSpacing", json!({"axis": "horizontal", "spacing": -1e308})),
        ("object.distributeSpacing", json!({"axis": "vertical", "spacing": 0})),
        ("object.align", json!({"horizontal": "center", "vertical": "center", "to": "artboard", "bounds": "preview"})),
        ("object.distribute", json!({"vertical": "center", "bounds": "geometric"})),
        ("object.distribute", json!({"horizontal": "right", "to": "artboard", "artboard": u64::MAX, "bounds": "preview"})),
        ("object.distributeSpacing", json!({"axis": "vertical", "to": "artboard", "artboard": -1, "spacing": null})),
        ("artboard.setProps", json!({"index": 0, "width": -1, "height": 0})),
        ("artboard.setProps", json!({"index": 0, "width": 1e300, "height": 1e-300, "scaleArt": true, "strokes": true, "corners": true})),
        ("artboard.setProps", json!({"index": 0, "x": -1e308, "width": 0, "height": 1, "scaleArt": true, "patterns": true})),
        ("artboard.new", json!({"width": 0, "height": -1})),
        ("artboard.delete", json!({"index": 0})),
        ("artboard.move", json!({"index": 0, "dx": 1e308, "dy": 0, "moveArt": true})),
        ("node.move", json!({"id": 1, "parent": 2, "index": 0})),
        ("node.move", json!({"id": 2, "parent": 2, "index": 0})),
        ("node.move", json!({"id": 1, "parent": null, "index": 1000})),
        ("layer.setProps", json!({"id": 1, "color": 1000})),
        ("layer.newSublayer", json!({"parent": 2})),
        ("text.create", json!({"x": 0, "y": 0, "text": "\u{0}\u{FFFF}\u{1F600}", "size": -1})),
        ("text.create", json!({"x": 0, "y": 0, "text": "a", "size": 1e308, "area": {"width": -1, "height": 0}})),
        ("text.setStyle", json!({"size": 0, "leading": -1, "tracking": 1e308})),
        ("text.setStyle", json!({"justify": "center", "start": u64::MAX, "end": 0})),
        ("text.setStyle", json!({"justify": "right", "start": -1, "end": "x"})),
        ("text.setFormat", json!({"spaceBefore": 1e308, "leftIndent": -1e308, "start": 3, "end": u64::MAX})),
        ("text.setFormat", json!({"hyphenate": true, "underline": true, "start": 1.5, "end": null})),
        ("text.setFormat", json!({"ids": [u64::MAX, 2], "firstLineIndent": 5, "start": 0, "end": 0})),
        ("text.tabs.set", json!({"stops": [{"position": 10}], "start": u64::MAX, "end": u64::MAX})),
        ("text.tabs.get", json!({"start": -5})),
        ("paraStyle.apply", json!({"name": "[Normal Paragraph Style]", "id": 2, "start": u64::MAX, "end": 1})),
        ("paraStyle.new", json!({"id": 2, "start": u64::MAX})),
        ("document.setUnits", json!({"units": "Parsecs"})),
        ("document.open", json!({"name": "x.svg", "dataBase64": "!!!"})),
        ("document.open", json!({"name": "x.svg", "dataBase64": "PHN2Zz4="})),
        ("document.open", json!({"name": "x.pdf", "dataBase64": "JVBERi0xLjQK"})),
        ("document.open", json!({"name": "x.vectorcraft", "dataBase64": "e30="})),
        ("document.activate", json!({"index": 99})),
        ("file.close", json!({"index": 99})),
        ("file.new", json!({"width": -1, "height": 0, "artboards": 1000000})),
        ("command.batch", json!({"commands": [{"command": 5}, {"params": 3}]})),
        ("command.batch", json!({"commands": "nope"})),
        ("object.puppetWarp", json!({"pins": [[0, 0]], "moved": [[0, 0]], "angles": [1e308]})),
        ("object.puppetWarp", json!({"pins": [[0, 0], [5, 5]], "moved": [[0, 0], [1e9, 1e9]], "angles": [-1e308, "x"]})),
        ("object.liquify", json!({"tool": "pucker", "points": [[0, 0, 1e308], [1, 1, -5], [2, 2, "x"]], "usePressure": true})),
        ("object.liquify", json!({"tool": "twirl", "points": vec![json!([50, 50, 0.5]); 400], "usePressure": "yes", "simplifyOn": 3})),
        ("object.liquify", json!({"tool": "wrinkle", "points": [[0, 0], [1e4, 1e4]], "width": 1e308, "complexity": -1, "affectIn": null})),
        ("object.blend.make", json!({"ids": [1, 2, 3], "starts": [u64::MAX, -1, "x", null, 1e308]})),
        ("object.blend.make", json!({"ids": [2, 2], "starts": {"a": 1}, "steps": 1e308})),
        ("object.blend.options", json!({"spacing": "distance", "value": -1e308, "orientation": 5})),
        ("object.blend.options", json!({"ramp": 1e308, "colorShift": -1e308, "easingType": [1]})),
        ("object.blend.make", json!({"ids": [1, 2], "ramp": f64::NAN, "colorShift": [1]})),
        ("object.blend.spine.moveAnchor", json!({"id": 2, "anchor": u64::MAX, "x": 1e308, "y": -1e308, "handle": "sideways"})),
        ("object.blend.spine.addAnchor", json!({"id": u64::MAX, "x": 0, "y": 0})),
        ("object.blend.spine.reshapeSegment", json!({"id": 2, "segment": u64::MAX, "t": f64::NAN, "dx": 1e308, "dy": -1e308})),
        ("object.blend.spine.removeAnchor", json!({"anchor": -1})),
        ("object.puppetWarp", json!({"rest": true, "pins": [], "moved": []})),
        ("object.puppetWarp", json!({"rest": true, "pins": [[1e308, 0]], "moved": [[0, 0]], "expand": -5})),
        ("object.puppetWarp", json!({"rest": true, "ids": [u64::MAX], "pins": [[0, 0]], "moved": [[0, 0]]})),
        ("object.puppetWarp.pins", json!({"ids": [0, 1, 2], "expand": 1e308})),
        ("symbol.new", json!({"name": "J", "nineSlice": true, "nineSliceGuides": [1e308, -1e308, f64::NAN, 0]})),
        ("symbol.new", json!({"name": "J", "nineSlice": "yes", "nineSliceGuides": [0.6, 0.1, 0.6, 0.1]})),
        ("symbol.new", json!({"name": "J", "nineSliceGuides": [0.1, 0.2]})),
        ("symbol.new", json!({"name": "J", "nineSliceGuides": {"left": 0.1}})),
        ("symbol.new", json!({"name": "J", "nineSlice": true, "nineSliceGuides": ["a", null, [1], {}]})),
        ("symbol.options", json!({"nineSlice": true, "nineSliceGuides": [0.5, 0.5, 0.5, 0.5]})),
        ("symbol.options", json!({"name": "Dot", "nineSlice": true, "nineSliceGuides": [1e308, -1e308, 0, 0]})),
        ("symbol.options", json!({"nineSlice": 7, "nineSliceGuides": []})),
        ("symbol.options", json!({"nineSlice": false, "nineSliceGuides": null})),
        // Shape Builder Gap Detection: the `gap` param and the tool options.
        ("shapeBuilder.merge", json!({"ids": [2, 3], "points": [[100, 80]], "gap": 1e308})),
        ("shapeBuilder.merge", json!({"ids": [2, 3], "points": [[100, 80], [120, 90]], "gap": -1e308})),
        ("shapeBuilder.merge", json!({"ids": [2, 3], "points": [[100, 80]], "gap": f64::MAX})),
        ("shapeBuilder.merge", json!({"ids": [2, 3], "points": [[100, 80]], "gap": "NaN"})),
        ("shapeBuilder.merge", json!({"ids": [2, 3], "points": [[100, 80]], "gap": "1e999"})),
        ("shapeBuilder.merge", json!({"ids": [2, 3], "points": [[100, 80]], "gap": null})),
        ("shapeBuilder.merge", json!({"points": [[100, 80]], "gap": [1, 2]})),
        ("shapeBuilder.merge", json!({"points": [[100, 80]], "gap": {"a": 1}, "erase": true})),
        ("shapeBuilder.merge", json!({"points": [[100, 80]], "gap": 1e-300, "erase": true, "tolerance": 1e308})),
        ("shapeBuilder.regions", json!({"gap": 1e308})),
        ("shapeBuilder.regions", json!({"ids": [0, 1, 2, 3], "gap": -1})),
        ("shapeBuilder.regions", json!({"ids": [2, 3], "gap": "x"})),
        ("shapeBuilder.regions", json!({"gap": true})),
        ("shapeBuilder.regions", json!({"gap": 12, "ids": [u64::MAX]})),
        // Path Precision Tools: the Parametric Shape Tool.
        ("addon.pathtools.shape.create", json!({"center": [1e308, -1e308], "width": 1e308, "count": 1e308})),
        (
            "addon.pathtools.shape.create",
            json!({"center": [0, 0], "kind": "gear", "count": 1000, "a": 0.9, "radius": 16384, "width": 16384, "height": 0.01}),
        ),
        ("addon.pathtools.shape.create", json!({"center": [0, 0], "kind": "star", "count": 1000, "a": 0.01, "radius": 1e-300})),
        ("addon.pathtools.shape.create", json!({"center": ["x", null], "kind": ["star"], "rotation": f64::NAN})),
        ("addon.pathtools.shape.create", json!({"center": [5, 5], "kind": "bubble", "a": -1e308, "b": 1e308, "rotation": 1e308})),
        ("addon.pathtools.shape.set", json!({"id": 2, "count": 1000, "kind": "gear"})),
        ("addon.pathtools.shape.set", json!({"id": u64::MAX, "width": -1})),
        ("addon.pathtools.shape.set", json!({"id": 2, "center": [1, 2, 3], "radius": "big"})),
        ("addon.pathtools.shape.convert", json!({"ids": [0, 1, 2, 3, u64::MAX]})),
        ("addon.pathtools.shape.convert", json!({"ids": "all"})),
        // Path Precision Tools: the Corner Tool and Slide Anchor Along Path.
        ("addon.pathtools.cornerSet", json!({"ids": [2], "radius": 1e6, "kind": "chamfer"})),
        ("addon.pathtools.cornerSet", json!({"ids": [2, 3], "anchors": [0, 1, u64::MAX], "radius": 5})),
        ("addon.pathtools.cornerSet", json!({"items": [{"id": 2, "anchors": [0]}, {"id": u64::MAX}], "radius": 1e-300})),
        ("addon.pathtools.cornerSet", json!({"items": [1, null, {"anchors": "x"}], "radius": 1})),
        ("addon.pathtools.cornerSet", json!({"ids": [2], "radius": 1e308, "kind": "spiky"})),
        ("addon.pathtools.cornerSet", json!({"items": "all", "radius": f64::NAN, "anchors": [-1, 1.5, "x"]})),
        ("addon.pathtools.cornerRemove", json!({"ids": [0, 1, 2, 3, u64::MAX]})),
        ("addon.pathtools.cornerRemove", json!({"items": [{"id": 2, "anchors": [u64::MAX]}]})),
        ("addon.pathtools.slideAnchor", json!({"id": 2, "anchor": 1, "t": 0.98})),
        ("addon.pathtools.slideAnchor", json!({"id": 2, "subpath": u64::MAX, "anchor": 0, "t": -0.5})),
        ("addon.pathtools.slideAnchor", json!({"id": 2, "anchor": u64::MAX, "t": 1e308})),
        ("addon.pathtools.slideAnchor", json!({"id": u64::MAX, "anchor": "x", "t": "y"})),
        // Path Precision Tools: the Shared Edge Merge Tool and Keep Shared Points Together.
        ("addon.pathtools.mergeEdge", json!({"a": 2, "b": 3, "at": [0, 0]})),
        ("addon.pathtools.mergeEdge", json!({"a": 2, "b": 3, "at": [10, 10], "diameter": 200, "tolerance": 3, "ends": true})),
        ("addon.pathtools.mergeEdge", json!({"a": 2, "b": 2, "at": [0, 0]})),
        ("addon.pathtools.mergeEdge", json!({"a": 2, "b": u64::MAX, "at": [1e308, -1e308]})),
        ("addon.pathtools.mergeEdge", json!({"a": 2, "b": 3, "at": [1, 2, 3], "diameter": 1e308, "tolerance": -1, "follow": "up"})),
        ("addon.pathtools.mergeEdge", json!({"a": "x", "b": null, "at": "y", "follow": 3, "ends": "z"})),
        ("addon.pathtools.movePointsTogether", json!({"enabled": "maybe"})),
        ("addon.pathtools.movePointsTogether", json!({"enabled": true})),
        ("addon.pathtools.slideAnchor", json!({"id": 2, "anchor": 1, "t": 0.5})),
        ("addon.pathtools.movePointsTogether", json!({"enabled": false})),
        // Smart Pen add-on.
        ("addon.pen.append", json!({"id": 2, "x": 1e308, "y": 0, "in": [1e308, -1e308]})),
        ("addon.pen.append", json!({"id": 2, "subpath": u64::MAX, "x": 0, "y": 0, "segment": "curved"})),
        ("addon.pen.append", json!({"id": 2, "x": 9.9e8, "y": -9.9e8, "in": [-9.9e8, 9.9e8], "out": [9.9e8, 9.9e8], "segment": "curved"})),
        ("addon.pen.close", json!({"id": 2, "subpath": u64::MAX, "segment": "curved"})),
        ("addon.pen.close", json!({"id": 3, "segment": "straight"})),
        ("addon.pen.segment", json!({"id": 2, "segment": u64::MAX, "curved": true})),
        ("addon.pen.segment", json!({"id": 2, "subpath": 0, "segment": 0, "curved": "yes"})),
        ("addon.pen.removePoint", json!({"id": 2, "anchor": u64::MAX, "plain": true})),
        ("addon.pen.removePoint", json!({"id": 2, "anchor": 0})),
        ("addon.pen.togglePoint", json!({"id": 2, "anchor": 0, "ratio": 1e308})),
        ("addon.pen.togglePoint", json!({"id": 2, "subpath": 0, "anchor": 1})),
        ("addon.pen.reshape", json!({"id": 2, "segment": 0, "t": 0.0, "dx": 9.9e8, "dy": -9.9e8, "mode": "keepAngles"})),
        ("addon.pen.reshape", json!({"id": 2, "segment": 0, "t": "x", "dx": 1, "dy": 1, "mode": "symmetric"})),
        ("addon.pen.nextSegment", json!({"curved": "yes"})),
        ("addon.pen.closeJoin", json!({"ids": [u64::MAX]})),
        ("addon.cleanup.breakText", json!({"by": "glyphs", "ids": [0, 1, 2, 3, 4, 5, 6, 7, 8, u64::MAX]})),
        ("addon.cleanup.breakText", json!({"by": ["lines"]})),
        ("addon.cleanup.breakText", json!({"by": "paragraphs"})),
        ("addon.cleanup.breakText", json!({"by": "words", "ids": "all"})),
        ("addon.cleanup.combineText", json!({"ids": [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]})),
        ("addon.cleanup.combineText", json!({"ids": [-1, 1e308]})),
        ("addon.cleanup.removeBreaks", json!({"kind": "paragraph", "ids": [1, 2, 3, 4, 5, 6, 7, 8, u64::MAX]})),
        ("addon.cleanup.removeBreaks", json!({"kind": 3})),
        ("addon.cleanup.removeBreaks", json!({"kind": "line", "ids": null})),
        ("addon.cleanup.splitText", json!({"at": 1e308})),
        ("addon.cleanup.alignInPlace", json!({"align": "right", "ids": [1, 2, 3, 4, 5, 6, 7, 8]})),
        ("addon.cleanup.alignInPlace", json!({"align": null, "ids": [-1]})),
        ("addon.cleanup.resetTextTransform", json!({"scale": "yes"})),
        ("addon.cleanup.resetTextTransform", json!({"scale": true, "ids": [1, 2, 3, 4, 5, 6, 7, 8]})),
        ("addon.cleanup.replaceFonts", json!({"font": ""})),
        ("addon.cleanup.replaceFonts", json!({"font": ["Source Sans 3"]})),
        ("addon.cleanup.replaceFonts", json!({"font": "\u{0}"})),
    ];
    let mut failures = vec![];
    for fx in Fixture::ALL {
        for (id, p) in &cases {
            if let Some(f) = probe(fx, id, p) {
                failures.push(f);
            }
        }
    }
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}

/// Artwork Cleanup's text tools while a Type tool edits type with junk ranges (past the end,
/// reversed, inside a character): never a panic, and the document stays sound.
#[test]
fn cleanup_text_tools_with_junk_editing_ranges() {
    let ranges = [(0, u64::MAX / 4), (u64::MAX, 1), (5, 5), (1, 2), (u64::MAX / 4, u64::MAX / 4)];
    let mut failures = vec![];
    for fx in Fixture::ALL {
        let mut texts = vec![];
        for l in &fx.session().doc().unwrap().doc.layers {
            l.walk(&mut |n| {
                if matches!(n.kind, vectorcraft_doc::NodeKind::Text(_)) {
                    texts.push(n.id.0);
                }
            });
        }
        for id in texts {
            for (anchor, caret) in ranges {
                for (cmd, p) in [("addon.cleanup.splitText", json!({})), ("addon.cleanup.removeBreaks", json!({"kind": "paragraph"}))] {
                    let mut s = fx.session();
                    let r = catch_quiet(|| {
                        let _ = s.execute("select.set", &json!({"ids": [id]}));
                        let _ = s.select_tool("type", Default::default());
                        s.set_tool_option("textSelection", &json!({"editing": id, "anchor": anchor, "caret": caret}));
                        no_panic(s.execute(cmd, &p))
                    });
                    if let Err(msg) = r {
                        failures.push(format!("PANIC {cmd} {id} {anchor}..{caret} [{fx:?}]: {msg}"));
                    } else if let Err(e) = check_session(&s) {
                        failures.push(format!("{cmd} {id} {anchor}..{caret} [{fx:?}]: {e}"));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}

/// Shape Builder Tool Options (Gap Detection) with junk values: never a panic, and the tool still
/// answers with options and works on a drag afterwards.
#[test]
fn shape_builder_options_junk() {
    let cases = [
        json!({"values": {"gapDetection": true, "gapLength": 1e308}}),
        json!({"values": {"gapDetection": "yes", "gapLength": -1e308}}),
        json!({"values": {"gapDetection": null, "gapLength": "NaN"}}),
        json!({"values": {"gapDetection": [true], "gapLength": [1, 2]}}),
        json!({"values": {"gapDetection": true, "gapLength": "1e999"}}),
        json!({"values": {"gapLength": {"a": 1}, "bogus": 1}}),
        json!({"key": "gapLength", "value": f64::MAX}),
        json!({"key": "gapLength", "value": f64::MIN_POSITIVE}),
        json!({"key": "gapDetection"}),
        json!({"key": "gapDetection", "value": 1}),
        json!({"values": [1, 2]}),
        json!({"values": "x"}),
    ];
    let mut failures = vec![];
    for fx in Fixture::ALL {
        for p in &cases {
            let mut s = fx.session();
            let r = catch_quiet(|| {
                let _ = s.select_tool("shapeBuilder", Default::default());
                let _ = s.set_tool_option_cmd(p);
                let _ = s.set_tool_option_cmd(&json!({"values": {"gapDetection": true}}));
                no_panic(s.execute("shapeBuilder.regions", &json!({})))
            });
            if let Err(msg) = r {
                failures.push(format!("PANIC {p} [{fx:?}]: {msg}"));
            } else if let Err(e) = check_session(&s) {
                failures.push(format!("{p} [{fx:?}]: {e}"));
            }
        }
    }
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}

/// Area Type Options with junk fit values, on selected area type (the fixtures have none), with
/// text that overflows: never a panic, the document stays sound, and editing still works.
#[test]
fn area_options_fit_junk() {
    let cases = [
        json!({"fit": "shrinkText", "fitMinPercent": 1e308}),
        json!({"fit": "shrinkText", "fitMinPercent": -1e308}),
        json!({"fit": "shrinkText", "fitMinPercent": "x"}),
        json!({"fit": {"shrinkText": {"minPercent": -5}}}),
        json!({"fit": {"shrinkText": {"minPercent": "a"}}}),
        json!({"fit": {"shrinkText": null}}),
        json!({"fit": {"autoHeight": 1}}),
        json!({"fit": {"bogus": {}}}),
        json!({"fit": [1, 2]}),
        json!({"fit": ""}),
        json!({"fit": "AUTO-HEIGHT", "height": 1e308, "width": -1e308}),
        json!({"fit": "autoHeight", "columns": u64::MAX, "gutter": 1e308, "inset": 1e308}),
        json!({"fit": "autoHeight", "rows": 5, "inset": -1}),
        json!({"fit": "shrinkText", "columns": 100, "gutter": 0, "inset": 10000}),
        json!({"fitMinPercent": 50}),
        json!({"fit": "none", "fitMinPercent": 1e-308}),
    ];
    let story = "Words that overflow a small frame. ".repeat(40);
    let mut failures = vec![];
    for (size, (w, h)) in [(12.0, (120.0, 40.0)), (0.1, (1.0, 1.0)), (1296.0, (100_000.0, 1.0))] {
        for p in &cases {
            let mut s = Fixture::Multi.session();
            let made = no_panic(s.execute("text.create", &json!({"x": 10, "y": 10, "size": size, "text": story, "area": {"width": w, "height": h}})));
            let Ok(made) = made else { continue };
            let id = made["id"].as_u64().unwrap_or(0);
            let r = catch_quiet(|| no_panic(s.execute("text.areaOptions", p)));
            match r {
                Err(m) => failures.push(format!("PANIC text.areaOptions {p} [{size} pt, {w}×{h}]: {m}")),
                Ok(_) => {
                    let typed = catch_quiet(|| no_panic(s.execute("text.editRange", &json!({"id": id, "start": 0, "insert": "More words. "}))));
                    if typed.is_err() {
                        failures.push(format!("PANIC typing after text.areaOptions {p}"));
                    }
                    if let Err(e) = check_session(&s) {
                        failures.push(format!("text.areaOptions {p} [{size} pt]: {e}"));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}

/// A live polygon's Polygon Properties with junk values (the fixtures have no polygon), drawn
/// plain, scaled unevenly and flattened to nothing: never a panic, and the document stays sound.
#[test]
fn polygon_properties_junk() {
    let mut cases: Vec<Value> = vec![json!({"makeSidesEqual": true}), json!({"sides": u64::MAX, "sideLength": 1e-300, "makeSidesEqual": true})];
    for key in ["polygonRadius", "sideLength", "polygonAngle"] {
        for v in junk_values() {
            cases.push(json!({ key: v }));
        }
    }
    let mut failures = vec![];
    for scale in [None, Some((300.0, 20.0)), Some((1e-300, 100.0))] {
        for p in &cases {
            let mut s = Fixture::Multi.session();
            let Ok(_) = no_panic(s.execute("shape.polygon", &json!({"cx": 100, "cy": 100, "radius": 40, "sides": 7, "rotation": 10}))) else {
                continue;
            };
            if let Some((sx, sy)) = scale {
                let _ = no_panic(s.execute("object.scale", &json!({"sx": sx, "sy": sy})));
            }
            match catch_quiet(|| no_panic(s.execute("object.setLiveShape", p))) {
                Err(m) => failures.push(format!("PANIC object.setLiveShape {p} [{scale:?}]: {m}")),
                Ok(_) => {
                    if let Err(e) = catch_quiet(|| check_all(&mut s)).unwrap_or_else(|m| Err(format!("panic in checks: {m}"))) {
                        failures.push(format!("object.setLiveShape {p} [{scale:?}]: {e}"));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}

/// After a successful fuzzed call, the document still round-trips and exports.
#[test]
fn fuzzed_calls_keep_documents_serializable() {
    let path = safe_path();
    let mut failures = vec![];
    for c in command_specs() {
        if heavy(c.id) || c.id.starts_with("file.") {
            continue;
        }
        for v in junk_values().iter().take(8) {
            for k in param_keys(c.params) {
                let k = if k == "path" { continue } else { k };
                let mut s = Fixture::Multi.session();
                let p = json!({ k.clone(): v });
                let ok = catch_quiet(|| s.execute(c.id, &p)).map(|r| r.is_ok()).unwrap_or(false);
                if ok && let Err(e) = catch_quiet(|| check_all(&mut s)).unwrap_or_else(|m| Err(format!("panic in checks: {m}"))) {
                    failures.push(format!("{} {p}: {e}", c.id));
                }
            }
        }
    }
    let _ = path;
    failures.sort();
    failures.dedup();
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.iter().take(40).cloned().collect::<Vec<_>>().join("\n"));
}

/// Undo after any successful command in the Multi fixture restores the exact document.
#[test]
fn every_command_undoes_exactly() {
    let mut failures = vec![];
    for c in command_specs() {
        if matches!(c.id, "edit.undo" | "edit.redo") || c.id.starts_with("file.") || c.id.starts_with("document.") {
            continue;
        }
        let mut s = Fixture::Multi.session();
        let before = doc_json(&s.doc().unwrap().doc);
        let depth = s.doc().unwrap().history.undo.len();
        let Ok(Ok(_)) = catch_quiet(|| s.execute(c.id, &json!({}))) else { continue };
        if s.doc().unwrap().history.undo.len() > depth {
            let _ = no_panic(s.execute("edit.undo", &json!({})));
            let after = doc_json(&s.doc().unwrap().doc);
            if after != before {
                failures.push(format!("{}: {}", c.id, vectorcraft_testkit::invariants::first_diff(&before, &after, "$")));
            }
        } else if doc_json(&s.doc().unwrap().doc) != before && !c.params.contains("View state: not an undo step") {
            failures.push(format!("{}: changed the document without an undo step", c.id));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Data Recovery reads whatever its store holds: damaged copies and details fail cleanly, and junk
/// params never panic.
#[test]
fn recovery_commands_with_a_damaged_store_and_junk_params() {
    use std::sync::Arc;
    use vectorcraft_engine::cmd::recovery::{MemoryStore, RecoveryStore};
    let store = Arc::new(MemoryStore::default());
    for (name, bytes) in [
        ("1-1/bad-1.vectorcraft", &b"{not json"[..]),
        ("1-1/bad-1.json", b"[1, 2"),
        ("1-1/gz-1.vectorcraft", &[0x1f, 0x8b, 0, 1, 2][..]),
        ("1-1/meta-only.json", br#"{"title": 5, "path": [], "saved": "x"}"#),
        ("2-1/heartbeat", b"not a number"),
        ("2-1/x.vectorcraft", b"{}"),
        ("../escape.vectorcraft", b"{}"),
        ("no-area.vectorcraft", b"{}"),
        ("/.vectorcraft", b"{}"),
    ] {
        store.write(name, bytes).unwrap();
    }
    let path = safe_path();
    for fx in Fixture::ALL {
        for id in ["file.recovery.save", "file.recovery.list", "file.recovery.restore", "file.recovery.discard"] {
            let spec = command_specs().iter().find(|c| c.id == id).unwrap();
            for p in std::iter::once(json!({})).chain(junk_params(spec.params, &path)) {
                let mut s = fx.session();
                s.recovery.set_store(store.clone());
                let r = catch_quiet(|| no_panic(s.execute(id, &p)));
                assert!(r.is_ok(), "PANIC {id} {p} [{fx:?}]");
                check_session(&s).unwrap_or_else(|e| panic!("{id} {p} [{fx:?}]: {e}"));
            }
        }
    }
}

/// Envelope Distort's commands on warp, mesh and top-object envelopes (selected, and with their
/// content selected in Edit Contents) with `{}` and junk params: never a panic, an open
/// interaction or a broken tree.
#[test]
fn envelope_commands_with_junk_params_on_envelopes() {
    let path = safe_path();
    let makes: [(&str, Value); 3] = [
        ("object.envelope.makeWithWarp", json!({"style": "twist", "bend": 80})),
        ("object.envelope.makeWithMesh", json!({"rows": 2, "cols": 3})),
        ("object.envelope.makeWithTopObject", json!({})),
    ];
    let mut failures = vec![];
    for (make, mp) in &makes {
        for editing in [false, true] {
            let ids: Vec<&str> =
                command_specs().iter().map(|c| c.id).filter(|id| id.starts_with("object.envelope.") || id.starts_with("object.mesh.")).collect();
            for id in ids {
                let spec = command_specs().iter().find(|c| c.id == id).unwrap();
                for p in std::iter::once(json!({})).chain(junk_params(spec.params, &path)) {
                    let mut s = Fixture::Multi.session();
                    if s.execute(make, mp).is_err() {
                        failures.push(format!("{make} failed on the Multi fixture"));
                        continue;
                    }
                    if editing {
                        let _ = no_panic(s.execute("object.envelope.editContents", &json!({"editing": true})));
                    }
                    let r = catch_quiet(|| no_panic(s.execute(id, &p)));
                    if r.is_err() {
                        failures.push(format!("PANIC {id} {p} [{make}, editing {editing}]"));
                    } else if s.in_interaction() {
                        failures.push(format!("{id} {p} [{make}]: left an interaction open"));
                    } else if let Err(e) = check_session(&s) {
                        failures.push(format!("{id} {p} [{make}]: {e}"));
                    }
                }
            }
        }
    }
    failures.sort();
    failures.dedup();
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.iter().take(40).cloned().collect::<Vec<_>>().join("\n"));
}

/// Inline graphics in text (a symbol set in a run of type): text commands with `{}`, junk and
/// structured junk (bad symbol names, offsets inside or past the graphic's character, malformed
/// inline runs) never panic, leave an interaction open or break the tree, and the document still
/// round-trips and exports.
#[test]
fn text_commands_with_junk_params_on_inline_graphics() {
    use vectorcraft_testkit::fixtures::{ellipse, exec, id_of, select};
    let path = safe_path();
    let setup = || {
        let mut s = Fixture::Multi.session();
        let e = ellipse(&mut s, 400.0, 300.0, 20.0, 20.0);
        select(&mut s, &[e]);
        exec(&mut s, "symbol.new", json!({"name": "Dot"}));
        let t = id_of(&exec(&mut s, "text.create", json!({"x": 20, "y": 200, "text": "ab cd", "size": 20})));
        exec(&mut s, "text.insertInline", json!({"id": t.0, "at": 2, "symbol": "Dot", "scale": 2}));
        select(&mut s, &[t]);
        (s, t)
    };
    let (_, t) = setup();
    let t = t.0;
    let style = json!({"font_family": "Source Sans 3", "size": 12, "fill": {"type": "none"}});
    let structured = [
        ("text.insertInline", json!({"id": t, "at": u64::MAX, "symbol": "Dot", "scale": 100})),
        ("text.insertInline", json!({"id": t, "at": 3, "symbol": "Dot", "scale": 1e-300, "shift": -1e5})),
        ("text.insertInline", json!({"id": t, "at": 3, "symbol": "\u{0}", "scale": 1})),
        ("text.insertInline", json!({"id": t, "at": 3, "symbol": "", "scale": f64::MAX})),
        ("text.insertInline", json!({"id": u64::MAX, "symbol": "Dot"})),
        ("text.insertInline", json!({"id": t, "at": "3", "symbol": ["Dot"], "scale": "big", "shift": null})),
        ("text.editRange", json!({"id": t, "start": 3, "end": 4, "insert": "x"})),
        ("text.editRange", json!({"id": t, "start": 4, "end": 3})),
        (
            "text.editRange",
            json!({"id": t, "start": 0, "end": 0, "runs": [{"text": "\u{FFFC}\u{FFFC}x", "style": style, "inline": {"symbol": "Dot", "scale": 1e308, "baseline_shift": -1e308}}]}),
        ),
        ("text.editRange", json!({"id": t, "start": 0, "end": 0, "runs": [{"text": "\u{FFFC}", "style": style, "inline": {"symbol": "Missing"}}]})),
        ("text.editRange", json!({"id": t, "start": 0, "end": 0, "runs": [{"text": "", "style": style, "inline": {"symbol": "Dot", "scale": -5}}]})),
        ("text.editRange", json!({"id": t, "runs": [{"text": "\u{FFFC}", "style": style, "inline": {"symbol": 5}}]})),
        ("text.setRangeStyle", json!({"id": t, "start": 3, "end": 4, "size": 1e308, "tracking": -1e308})),
        ("text.getRange", json!({"id": t, "start": 3, "end": 4})),
        ("type.insert", json!({"text": "\u{FFFC}"})),
        ("type.changeCase", json!({"case": "upper"})),
        ("type.smartPunctuation", json!({"quotes": true, "dashes": true, "ellipsis": true, "scope": "selection"})),
        ("type.convertToAreaType", json!({})),
        ("type.createOutlines", json!({})),
        ("symbol.delete", json!({"name": "Dot"})),
        ("symbol.delete", json!({"name": "Dot", "expandInstances": false})),
        ("symbol.update", json!({"name": "Dot"})),
    ];
    let mut failures = vec![];
    let mut run = |id: &str, p: &Value| {
        let (mut s, _) = setup();
        match catch_quiet(|| no_panic(s.execute(id, p))) {
            Err(msg) => failures.push(format!("PANIC {id} {p}: {msg}")),
            Ok(r) => {
                if s.in_interaction() {
                    failures.push(format!("{id} {p}: left an interaction open"));
                } else if let Err(e) = check_session(&s) {
                    failures.push(format!("{id} {p}: {e}"));
                } else if r.is_ok()
                    && let Err(e) = catch_quiet(|| check_all(&mut s)).unwrap_or_else(|m| Err(format!("panic in checks: {m}")))
                {
                    failures.push(format!("{id} {p}: {e}"));
                }
            }
        }
    };
    for (id, p) in &structured {
        run(id, p);
    }
    for id in ["text.insertInline", "text.editRange", "text.getRange", "text.setRangeStyle", "type.insert"] {
        let spec = command_specs().iter().find(|c| c.id == id).unwrap();
        for p in std::iter::once(json!({})).chain(junk_params(spec.params, &path)) {
            run(id, &p);
        }
    }
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}

/// Create Gradient Mesh on embedded, linked and broken images (bad pixels, zero size, a NaN
/// transform) with junk rows and columns: never a panic or a broken tree.
#[test]
fn gradient_mesh_from_images_with_junk_params() {
    use vectorcraft_doc::{ImageBlob, ImageObject, LinkInfo, Node, NodeKind};
    use vectorcraft_geom::Affine;
    let img = image::RgbaImage::from_fn(5, 3, |x, y| image::Rgba([x as u8 * 50, y as u8 * 90, 7, if x == 2 { 0 } else { 255 }]));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let params = [
        json!({}),
        json!({"rows": 1, "cols": 1}),
        json!({"rows": 50, "cols": 50}),
        json!({"rows": 0, "cols": 3}),
        json!({"rows": 51, "cols": 3}),
        json!({"rows": -1, "cols": 1e308}),
        json!({"rows": "x", "columns": null, "appearance": 5}),
        json!({"rows": 7, "cols": 9, "appearance": "edge", "highlight": f64::MAX}),
        json!({"at": [2.5, 1.0]}),
        json!({"at": [1e308, -1e308]}),
        json!({"ids": [u64::MAX]}),
    ];
    // (stored bytes, width, height, transform, linked)
    let variants: [(Vec<u8>, u32, u32, Affine, bool); 6] = [
        (png.clone(), 5, 3, Affine::scale(4.0), false),
        (png.clone(), 5, 3, Affine::scale(4.0), true),
        (png.clone(), 0, 0, Affine::scale(4.0), false),
        (png.clone(), 5, 3, Affine::new([f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0]), false),
        (png[..png.len() / 2].to_vec(), 5, 3, Affine::IDENTITY, false),
        (png.clone(), u32::MAX, u32::MAX, Affine::scale(1e300), false),
    ];
    let mut failures = vec![];
    for (i, (bytes, w, h, xf, linked)) in variants.iter().enumerate() {
        for p in &params {
            let mut s = Fixture::Empty.session();
            let placed = s.edit("Place", |d, sel| {
                d.images.insert("pic".into(), ImageBlob::png(bytes.clone()));
                let id = d.alloc_id();
                let link = linked.then(|| LinkInfo { path: "/nonexistent/pic.png".into(), ..Default::default() });
                let im = ImageObject { key: "pic".into(), width: *w, height: *h, xf: *xf, link, placement: Default::default(), pixels: None };
                d.insert(d.default_layer(), 0, Node::new(id, NodeKind::Image(Box::new(im))))?;
                sel.set([id]);
                Ok(())
            });
            if placed.is_err() {
                // Insert already refuses a NaN or out-of-range transform.
                assert!(i >= 3, "variant {i}: {placed:?}");
                continue;
            }
            let r = catch_quiet(|| no_panic(s.execute("object.mesh.create", p)));
            if r.is_err() {
                failures.push(format!("PANIC variant {i} {p}"));
            } else if let Err(e) = check_session(&s) {
                failures.push(format!("variant {i} {p}: {e}"));
            }
        }
    }
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}
