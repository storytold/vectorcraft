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

/// Run one call; returns a failure description or None.
fn probe(fx: Fixture, id: &str, params: &Value) -> Option<String> {
    let mut s = fx.session();
    let r = catch_quiet(|| s.execute(id, params));
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
        match catch_quiet(|| s.execute(c.id, &json!({}))) {
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

/// Known bugs (see the `#[ignore]` repro tests below): (command, param key) pairs skipped by the
/// junk sweep so the rest of the sweep stays meaningful.
const KNOWN_BUGS: &[(&str, &str)] = &[
    ("object.scale", "sx"),
    ("object.transformEach", "scaleH"),
    ("object.transformEach", "scaleV"),
    ("object.distributeSpacing", "spacing"),
    ("artboard.setProps", "x"),
    ("artboard.setProps", "y"),
    ("artboard.setProps", "width"),
    ("artboard.setProps", "height"),
];

fn known_bug(id: &str, p: &Value) -> bool {
    KNOWN_BUGS.iter().any(|(c, k)| *c == id && p.get(*k).is_some())
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
        if s.execute(id, &p).is_ok() {
            vectorcraft_testkit::invariants::check_native_roundtrip(&s.doc().unwrap().doc).unwrap_or_else(|e| panic!("{id} {p}: {e}"));
        }
    }
}

#[test]
fn bug_artboard_huge_props_break_svg() {
    for key in ["x", "y", "width", "height"] {
        let mut s = Fixture::Single.session();
        let p = json!({"index": 0, key: 1e308});
        if s.execute("artboard.setProps", &p).is_ok() {
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
                if known_bug(c.id, p) {
                    continue;
                }
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
        ("effect.apply", json!({"effect": "pixelate.colorHalftone", "params": {"maxRadius": 1e308, "channel1": "1e999", "channel4": null}})),
        ("effect.apply", json!({"effect": "pixelate.crystallize", "params": {"cellSize": -1e308}})),
        ("effect.apply", json!({"effect": "pixelate.mezzotint", "params": {"type": 5}})),
        ("effect.apply", json!({"effect": "pixelate.pointillize", "params": {"cellSize": "NaN"}})),
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
        ("effect.apply", json!({"effect": "no.such.effect"})),
        ("effect.remove", json!({"index": 99})),
        ("effect.setParams", json!({"index": 0, "params": null})),
        ("object.path.offsetPath", json!({"offset": 1e308})),
        ("object.path.offsetPath", json!({"offset": -1e308})),
        ("object.path.simplify", json!({"tolerance": -1, "cornerAngle": 1e308})),
        ("object.path.splitIntoGrid", json!({"rows": 500, "columns": 1, "gutter": 1e308})),
        ("object.path.splitIntoGrid", json!({"rows": 0.4})),
        ("object.setBounds", json!({"width": 0, "height": 0})),
        ("object.setBounds", json!({"width": -10, "height": 1e308, "reference": 99})),
        ("object.distributeSpacing", json!({"axis": "horizontal", "spacing": -1e308})),
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
        ("object.blend.spine.moveAnchor", json!({"id": 2, "anchor": u64::MAX, "x": 1e308, "y": -1e308, "handle": "sideways"})),
        ("object.blend.spine.addAnchor", json!({"id": u64::MAX, "x": 0, "y": 0})),
        ("object.blend.spine.removeAnchor", json!({"anchor": -1})),
        ("object.puppetWarp", json!({"rest": true, "pins": [], "moved": []})),
        ("object.puppetWarp", json!({"rest": true, "pins": [[1e308, 0]], "moved": [[0, 0]], "expand": -5})),
        ("object.puppetWarp", json!({"rest": true, "ids": [u64::MAX], "pins": [[0, 0]], "moved": [[0, 0]]})),
        ("object.puppetWarp.pins", json!({"ids": [0, 1, 2], "expand": 1e308})),
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
            let made = s.execute("text.create", &json!({"x": 10, "y": 10, "size": size, "text": story, "area": {"width": w, "height": h}}));
            let Ok(made) = made else { continue };
            let id = made["id"].as_u64().unwrap_or(0);
            let r = catch_quiet(|| s.execute("text.areaOptions", p));
            match r {
                Err(m) => failures.push(format!("PANIC text.areaOptions {p} [{size} pt, {w}×{h}]: {m}")),
                Ok(_) => {
                    let typed = catch_quiet(|| s.execute("text.editRange", &json!({"id": id, "start": 0, "insert": "More words. "})));
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
            let Ok(_) = s.execute("shape.polygon", &json!({"cx": 100, "cy": 100, "radius": 40, "sides": 7, "rotation": 10})) else { continue };
            if let Some((sx, sy)) = scale {
                let _ = s.execute("object.scale", &json!({"sx": sx, "sy": sy}));
            }
            match catch_quiet(|| s.execute("object.setLiveShape", p)) {
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
                if known_bug(c.id, &p) {
                    continue;
                }
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
            let _ = s.execute("edit.undo", &json!({}));
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
                let r = catch_quiet(|| s.execute(id, &p));
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
                        let _ = s.execute("object.envelope.editContents", &json!({"editing": true}));
                    }
                    let r = catch_quiet(|| s.execute(id, &p));
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
        match catch_quiet(|| s.execute(id, p)) {
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
