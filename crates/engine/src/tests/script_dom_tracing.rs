//! Tracing in the script object model: `trace()` on placed and raster items, the `PluginItem`, its
//! `TracingObject` and its `TracingOptions`, and the checks on what a script gives them.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::*;

/// A fresh folder for test `name`, with a `main` folder for the script, by its real path.
fn folder(name: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("vc-dom-tracing-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("main")).unwrap();
    std::fs::canonicalize(&base).unwrap()
}

/// A 60 × 60 PNG of a black disc on white.
fn disc_png() -> Vec<u8> {
    vectorcraft_trace::Raster::from_fn(60, 60, |x, y| {
        let (dx, dy) = (f64::from(x) + 0.5 - 30.0, f64::from(y) + 0.5 - 30.0);
        if dx * dx + dy * dy <= 400.0 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }
    })
    .encode_png()
}

/// A session with one 400 × 300 pt document holding the disc image, linked (`link`) or embedded.
fn session(base: &Path, link: bool) -> (Session, NodeId) {
    let png = base.join("main/disc.png");
    std::fs::write(&png, disc_png()).unwrap();
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let r = s.execute("file.place", &json!({"path": png.to_string_lossy(), "link": link})).unwrap();
    let id = NodeId(r["ids"][0].as_u64().unwrap());
    (s, id)
}

/// Runs `text` as `main/main.jsx` in `base`.
fn run(s: &mut Session, base: &Path, text: &str) -> Result<Value> {
    let main = base.join("main/main.jsx");
    std::fs::write(&main, text).unwrap();
    s.execute("script.run", &json!({"path": main.to_string_lossy()}))
}

fn log(s: &mut Session, base: &Path, text: &str) -> Vec<Value> {
    let r = run(s, base, text).unwrap_or_else(|e| panic!("{text}: {e}"));
    r["log"].as_array().cloned().unwrap_or_default()
}

fn undo_steps(s: &Session) -> usize {
    s.active().unwrap().history.undo.len()
}

/// The Image Trace objects of the active document.
fn traces(s: &Session) -> Vec<NodeId> {
    let mut out = vec![];
    s.active().unwrap().doc.walk(|n| {
        if crate::cmd::is_trace_group(n) {
            out.push(n.id);
        }
    });
    out
}

/// The `trace` settings of the Image Trace object `id`.
fn trace_of(s: &Session, id: NodeId) -> Value {
    s.active().unwrap().doc.node(id).and_then(|n| n.extras().trace.as_deref().cloned()).unwrap_or(Value::Null)
}

/// Every top-level object of the active document's first layer.
fn top(s: &Session) -> Vec<NodeId> {
    let d = &s.active().unwrap().doc;
    d.children(Some(d.layers[0].id)).map(|c| c.iter().map(|n| n.id).collect()).unwrap_or_default()
}

/// Traces with the Default preset as a script does, which the script's counts should match: its
/// `paths` and `anchors`, and the different paints of the result.
fn command_counts(base: &Path, link: bool) -> (u64, u64, u64) {
    let (mut s, id) = session(base, link);
    let r = s.execute("imageTrace.make", &json!({"id": id.0})).unwrap();
    (r["paths"].as_u64().unwrap(), r["anchors"].as_u64().unwrap(), r["colors"].as_u64().unwrap())
}

#[test]
fn raster_trace_returns_plugin_with_counts() {
    let base = folder("raster");
    let (mut s, image) = session(&base, false);
    let ppi = s.execute("image.info", &json!({"id": image.0})).unwrap()["ppi"][0].clone();
    let out = log(
        &mut s,
        &base,
        "var d = app.activeDocument, r = d.rasterItems[0]; r.name = 'src'; var p = r.trace(), t = p.tracing;\
         vc.log(p.typename); vc.log(p.isTracing); vc.log(t.typename); vc.log(t.sourceArt.name); vc.log(r.typename);\
         vc.log(t.pathCount); vc.log(t.anchorCount); vc.log(t.usedColorCount); vc.log(t.imageResolution);\
         vc.log(p.tracing.pathCount === t.pathCount); vc.log(t.parent.isTracing); vc.log(t.tracingOptions.typename);\
         vc.log(t.tracingOptions.parent.typename); vc.log(d.pageItems[0].typename); vc.log(t.tracingOptions.preset);",
    );
    assert_eq!(out[..5], [json!("PluginItem"), json!(true), json!("TracingObject"), json!("src"), json!("RasterItem")]);
    let (paths, anchors, colors) = command_counts(&base, false);
    assert!(paths > 0 && anchors > 0, "{paths} {anchors}");
    assert_eq!(out[5], json!(paths));
    assert_eq!(out[6], json!(anchors));
    let used = out[7].as_u64().unwrap();
    assert!(used >= 1 && used <= colors, "{used} of {colors}");
    assert_eq!(out[8].as_f64(), ppi.as_f64());
    assert_eq!(out[9..], [json!(true), json!(true), json!("TracingOptions"), json!("TracingObject"), json!("PluginItem"), json!("Default")]);
    let groups = traces(&s);
    assert_eq!(groups.len(), 1);
    assert_eq!(top(&s), groups);
    assert_eq!(undo_steps(&s), 2, "the place and the run");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn trace_image_resolution_is_one_number_or_none() {
    let base = folder("resolution");
    let (mut s, image) = session(&base, false);
    let ppi = s.execute("image.info", &json!({"id": image.0})).unwrap()["ppi"][0].as_f64().unwrap();
    let out = log(
        &mut s,
        &base,
        "var r = app.activeDocument.rasterItems[0]; r.width = r.width * 2; r.height = r.height * 2;\
         var t = r.trace().tracing; vc.log(t.imageResolution); t.releaseTracing(); r.width = r.width * 2;\
         var m = ''; try { r.trace().tracing.imageResolution; } catch (e) { m = e.message; } vc.log(m);",
    );
    assert!((out[0].as_f64().unwrap() - ppi / 2.0).abs() < 1e-9, "{out:?}");
    assert_eq!(out[1], json!("the source image has no single resolution"));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn placed_trace_returns_plugin_with_source_art() {
    let base = folder("placed");
    let (mut s, image) = session(&base, true);
    let out = log(
        &mut s,
        &base,
        "var d = app.activeDocument, pl = d.placedItems[0]; pl.name = 'src'; var p = pl.trace();\
         vc.log(p.typename); vc.log(p.tracing.sourceArt.name); vc.log(pl.typename); vc.log(pl.file.name);\
         var m = ''; try { pl.trace(); } catch (e) { m = e.message; } vc.log(m);",
    );
    assert_eq!(out, [json!("PluginItem"), json!("src"), json!("PlacedItem"), json!("disc.png"), json!("The image is the source art of a tracing")]);
    let groups = traces(&s);
    assert_eq!(groups.len(), 1);
    let d = &s.active().unwrap().doc;
    assert_eq!(d.parent_of(image), Some(groups[0]), "the image is the tracing's source");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn trace_checks_the_image_first() {
    let base = folder("unbound");
    let (mut s, image) = session(&base, true);
    let out = log(
        &mut s,
        &base,
        "var d = app.activeDocument, p = d.placedItems[0], n = d.placedItems.add(), m = [];\
         try { n.trace(); } catch (e) { m.push(e.message); } p.embed(); try { p.trace(); } catch (e) { m.push(e.message); }\
         try { PlacedItem.prototype.trace.call(d.rasterItems[0]); } catch (e) { m.push(e.message); } vc.log(m);",
    );
    assert_eq!(out, [json!(["PlacedItem.file is required", "Stale page item", "this is not a PlacedItem"])]);
    assert!(traces(&s).is_empty());
    assert!(s.active().unwrap().doc.node(image).is_some());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn trace_missing_link_refuses_preview() {
    let base = folder("missing");
    let (mut s, image) = session(&base, true);
    std::fs::remove_file(base.join("main/disc.png")).unwrap();
    let out = log(&mut s, &base, "var m = ''; try { app.activeDocument.placedItems[0].trace(); } catch (e) { m = e.message; } vc.log(m);");
    assert_eq!(out, [json!("Linked image is missing")]);
    assert!(traces(&s).is_empty());
    assert_eq!(top(&s), [image]);
    assert_eq!(undo_steps(&s), 1);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn trace_release_restores_source_kind() {
    let base = folder("release");
    let (mut s, image) = session(&base, false);
    let out = log(
        &mut s,
        &base,
        "var r = app.activeDocument.rasterItems[0]; r.name = 'src';\
         var p = r.trace(), t = p.tracing, o = t.tracingOptions, back = t.releaseTracing();\
         vc.log(back.name); vc.log(back.typename); vc.log(r.hidden);\
         var m = []; try { p.name; } catch (e) { m.push(e.message); } try { t.pathCount; } catch (e) { m.push(e.message); }\
         try { o.threshold; } catch (e) { m.push(e.message); } vc.log(m);",
    );
    assert_eq!(out[..3], [json!("src"), json!("RasterItem"), json!(false)]);
    assert_eq!(out[3], json!(["Stale page item", "Stale page item", "Stale page item"]));
    assert!(traces(&s).is_empty());
    assert_eq!(top(&s), [image]);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn trace_expand_returns_group_and_stales_plugin() {
    let base = folder("expand");
    let (mut s, image) = session(&base, false);
    let out = log(
        &mut s,
        &base,
        "var r = app.activeDocument.rasterItems[0], p = r.trace(), t = p.tracing, m = [];\
         try { t.expandTracing(true); } catch (e) { m.push(e.message); }\
         try { t.expandTracing('yes'); } catch (e) { m.push(e.name); }\
         vc.log(m); vc.log(p.isTracing);\
         var g = t.expandTracing(); vc.log(g.typename); vc.log(p === g); m = [];\
         try { p.isTracing; } catch (e) { m.push(e.message); } try { t.anchorCount; } catch (e) { m.push(e.message); }\
         try { r.name; } catch (e) { m.push(e.message); } vc.log(m); vc.log(app.activeDocument.pageItems[0] === g);",
    );
    assert_eq!(out[0], json!(["TracingObject.expandTracing(true) is not implemented", "TypeError"]));
    assert_eq!(out[1..4], [json!(true), json!("GroupItem"), json!(false)], "the group is a new object");
    assert_eq!(out[4], json!(["Stale page item", "Stale page item", "Stale page item"]));
    assert_eq!(out[5], json!(true), "the group read again is the object expandTracing gave");
    assert!(traces(&s).is_empty());
    let d = &s.active().unwrap().doc;
    assert!(d.node(image).is_none(), "the expanded group keeps no source image");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn trace_options_retrace_preserves_source_and_view() {
    let base = folder("options");
    let (mut s, image) = session(&base, false);
    let defaults = serde_json::to_value(vectorcraft_trace::TraceParams::default()).unwrap();
    let out = log(
        &mut s,
        &base,
        "var r = app.activeDocument.rasterItems[0]; r.name = 'src'; var p = r.trace(), t = p.tracing, o = t.tracingOptions;\
         vc.log([o.tracingMode, o.threshold, o.fills, o.strokes, o.maxColors, o.palette, o.ignoreWhite, o.minArea]);\
         vc.log([o.viewRaster, o.viewVector]);\
         o.viewVector = ViewVectorType.TRACINGVIEWVECTOROUTLINES; o.viewRaster = ViewRasterType.TRACINGVIEWRASTERORIGINALIMAGE;\
         o.threshold = 200; o.minArea = 4;\
         vc.log([o.threshold, o.minArea, o.preset, o.viewRaster, o.viewVector]);\
         vc.log(t.sourceArt.name); vc.log(r.name); vc.log(p.isTracing); vc.log(p.tracing.tracingOptions.threshold); vc.log(t.pathCount > 0);",
    );
    let mode = |m: &str| json!(format!("TracingModeType.TRACINGMODE{m}"));
    let expected_mode = match defaults["mode"].as_str() {
        Some("blackAndWhite") => mode("BLACKANDWHITE"),
        Some("grayscale") => mode("GRAY"),
        _ => mode("COLOR"),
    };
    assert_eq!(
        out[0],
        json!([
            expected_mode,
            defaults["threshold"],
            defaults["fills"],
            defaults["strokes"],
            defaults["colors"],
            "Limited",
            defaults["ignoreWhite"],
            defaults["noise"]
        ])
    );
    assert_eq!(out[1], json!(["ViewRasterType.TRACINGVIEWRASTERNOIMAGE", "ViewVectorType.TRACINGVIEWVECTORTRACINGRESULT"]));
    assert_eq!(out[2], json!([200, 4, "Custom", "ViewRasterType.TRACINGVIEWRASTERORIGINALIMAGE", "ViewVectorType.TRACINGVIEWVECTOROUTLINES"]));
    assert_eq!(out[3..], [json!("src"), json!("src"), json!(true), json!(200), json!(true)]);
    let groups = traces(&s);
    assert_eq!(groups.len(), 1);
    let t = trace_of(&s, groups[0]);
    assert_eq!(
        (t["params"]["threshold"].clone(), t["params"]["noise"].clone(), t["view"].clone()),
        (json!(200), json!(4), json!("outlinesWithSourceImage"))
    );
    assert_eq!(s.active().unwrap().doc.parent_of(image), Some(groups[0]));

    // Assigning the values the options have already changes nothing.
    let before = (undo_steps(&s), traces(&s));
    let out = log(
        &mut s,
        &base,
        "var o = app.activeDocument.pageItems[0].tracing.tracingOptions; o.threshold = 200; o.minArea = 4; o.fills = true;\
         o.viewVector = ViewVectorType.TRACINGVIEWVECTOROUTLINES; vc.log(o.threshold);",
    );
    assert_eq!(out, [json!(200)]);
    assert_eq!((undo_steps(&s), traces(&s)), before);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn trace_invalid_option_is_atomic() {
    let base = folder("invalid");
    let (mut s, _) = session(&base, false);
    log(&mut s, &base, "app.activeDocument.rasterItems[0].trace();");
    let before = (undo_steps(&s), traces(&s), trace_of(&s, traces(&s)[0]));
    let cases = [
        ("o.threshold = 'x'", "TypeError"),
        ("o.threshold = 256", "TracingOptions.threshold is out of range"),
        ("o.threshold = -1", "TracingOptions.threshold is out of range"),
        ("o.threshold = 1.5", "TracingOptions.threshold is out of range"),
        ("o.threshold = NaN", "TypeError"),
        ("o.maxColors = 1", "TracingOptions.maxColors is out of range"),
        ("o.maxColors = 257", "TracingOptions.maxColors is out of range"),
        ("o.minArea = 1e12", "TracingOptions.minArea is out of range"),
        ("o.fills = 1", "TypeError"),
        ("o.fills = false", "TracingOptions requires fills or strokes"),
        ("o.tracingMode = 'color'", "TypeError"),
        ("o.palette = 'Spectrum'", "TracingOptions.palette has an invalid value"),
        ("o.palette = 3", "TypeError"),
        ("o.viewRaster = ViewRasterType.TRACINGVIEWRASTERADJUSTEDIMAGE", "TracingOptions.viewRaster has an invalid value"),
        ("o.viewRaster = ViewRasterType.TRACINGVIEWRASTERORIGINALIMAGE", "TracingOptions.viewRaster has an invalid value"),
        ("o.viewVector = 'outlines'", "TypeError"),
        ("o.pathFitting = 3", "TracingOptions.pathFitting is not implemented"),
        ("o.cornerAngle", "TracingOptions.cornerAngle is not implemented"),
        ("o.resampleResolution = 72", "TracingOptions.resampleResolution is not implemented"),
        ("t.areaCount", "TracingObject.areaCount is not implemented"),
        ("o.loadFromPreset('No Such Preset')", "no built-in or saved preset has that name"),
        ("o.loadFromPreset('../Default')", "cannot hold"),
        ("o.loadFromPreset('C:\\\\x')", "cannot hold"),
        ("o.loadFromPreset('a\\u0000b')", "cannot hold"),
        ("o.loadFromPreset(new Array(300).join('x'))", "at most 256 bytes"),
        ("o.loadFromPreset('  ')", "the preset name is empty"),
        ("o.loadFromPreset(5)", "TypeError"),
        ("o.storeToPreset('Mine')", "a script cannot run `imageTrace.presets.save`"),
        ("o.storeToPreset('../Mine')", "cannot hold"),
    ];
    let mut wrong = vec![];
    for (code, message) in cases {
        let out = log(
            &mut s,
            &base,
            &format!(
                "var t = app.activeDocument.pageItems[0].tracing, o = t.tracingOptions, m = 'none';\
                 try {{ {code}; }} catch (e) {{ m = e.name + ': ' + e.message; }} vc.log(m); vc.log(o.threshold);"
            ),
        );
        let m = out[0].as_str().unwrap_or_default().to_string();
        let now = (undo_steps(&s), traces(&s), trace_of(&s, traces(&s)[0]));
        if !m.contains(message) || out[1] != before.2["params"]["threshold"] || now != before {
            wrong.push(format!("{code}: {m}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
    assert!(s.prefs.trace_presets.is_empty(), "no preset was saved");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn trace_presets_load_and_locked_store() {
    let base = folder("presets");
    let (mut s, _) = session(&base, false);
    let silhouettes = serde_json::to_value(vectorcraft_trace::preset("Silhouettes").unwrap()).unwrap();
    let out = log(
        &mut s,
        &base,
        "var p = app.activeDocument.rasterItems[0].trace(), o = p.tracing.tracingOptions;\
         vc.log(o.loadFromPreset('Silhouettes')); vc.log(o.preset); vc.log(o.threshold);\
         vc.log(o.storeToPreset('Default')); vc.log(o.storeToPreset('custom')); vc.log(o.storeToPreset('Silhouettes'));",
    );
    assert_eq!(out, [json!(true), json!("Silhouettes"), silhouettes["threshold"].clone(), json!(false), json!(false), json!(false)]);
    let t = trace_of(&s, traces(&s)[0]);
    assert_eq!(t["params"], silhouettes);

    // Loading the preset the tracing has already traces nothing again.
    let before = (undo_steps(&s), traces(&s));
    let out = log(&mut s, &base, "vc.log(app.activeDocument.pageItems[0].tracing.tracingOptions.loadFromPreset('silhouettes'));");
    assert_eq!(out, [json!(true)]);
    assert_eq!((undo_steps(&s), traces(&s)), before);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn trace_external_retrace_undo_and_source_delete() {
    let base = folder("stale");
    let (mut s, image) = session(&base, false);
    let threshold = serde_json::to_value(vectorcraft_trace::TraceParams::default()).unwrap()["threshold"].clone();
    let out = log(
        &mut s,
        &base,
        &format!(
            "var d = app.activeDocument, m = [];\
             var p = d.rasterItems[0].trace(), t = p.tracing; d.selection = p; vc.run('imageTrace.make', {{}});\
             try {{ t.pathCount; }} catch (e) {{ m.push(e.message); }} try {{ p.isTracing; }} catch (e) {{ m.push(e.message); }}\
             var q = d.pageItems[0], u = q.tracing, o = u.tracingOptions; o.threshold = 90; vc.log(o.threshold); vc.log(q.isTracing);\
             vc.run('edit.undo'); vc.log(q.isTracing); vc.log(o.threshold); vc.log(u.pathCount > 0);\
             var r = ''; try {{ u.sourceArt.remove(); }} catch (e) {{ r = e.message; }} vc.log(r); vc.log(u.pathCount > 0);\
             vc.run('edit.clear', {{ids: [{}]}});\
             try {{ u.pathCount; }} catch (e) {{ m.push(e.message); }} try {{ q.isTracing; }} catch (e) {{ m.push(e.message); }}\
             vc.log(d.pageItems[0].typename); vc.log(m);",
            image.0
        ),
    );
    assert_eq!(out[..5], [json!(90), json!(true), json!(true), threshold, json!(true)], "the undo puts the first tracing back");
    assert!(out[5].as_str().is_some_and(|m| m.starts_with("cannot delete hidden or locked objects")), "{:?}", out[5]);
    assert_eq!(out[6], json!(true), "the refused remove leaves the tracing");
    assert_eq!(out[7], json!("GroupItem"), "a group without its source image is no longer a tracing");
    assert_eq!(out[8], json!(vec!["Stale page item"; 4]));
    let _ = std::fs::remove_dir_all(base);
}

/// After two option changes, an Undo puts back the tracing the first change made, and the objects
/// of the first tracing follow it there.
#[test]
fn trace_undo_of_the_second_retrace_reaches_the_first() {
    let base = folder("chain");
    let (mut s, _) = session(&base, false);
    let out = log(
        &mut s,
        &base,
        "var p = app.activeDocument.rasterItems[0].trace(), o = p.tracing.tracingOptions;\
         o.threshold = 90; o.threshold = 60; vc.log(o.threshold); vc.run('edit.undo');\
         vc.log(o.threshold); vc.log(p.isTracing); vc.log(p.tracing.pathCount > 0);",
    );
    assert_eq!(out, [json!(60), json!(90), json!(true), json!(true)]);
    let groups = traces(&s);
    assert_eq!(groups.len(), 1);
    assert_eq!(trace_of(&s, groups[0])["params"]["threshold"], json!(90));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn trace_deleted_plugin_is_stale() {
    let base = folder("deleted");
    let (mut s, _) = session(&base, false);
    let out = log(
        &mut s,
        &base,
        "var p = app.activeDocument.rasterItems[0].trace(), t = p.tracing, o = t.tracingOptions, m = []; p.remove();\
         try { p.name; } catch (e) { m.push(e.message); } try { t.sourceArt; } catch (e) { m.push(e.message); }\
         try { o.threshold = 10; } catch (e) { m.push(e.message); } try { t.releaseTracing(); } catch (e) { m.push(e.message); }\
         try { o.pathFitting; } catch (e) { m.push(e.message); } try { o.cornerAngle = 1; } catch (e) { m.push(e.message); } vc.log(m);",
    );
    assert_eq!(out, [json!(vec!["Stale page item"; 6])]);
    assert!(traces(&s).is_empty());
    let _ = std::fs::remove_dir_all(base);
}

/// A script's trace of an image past the tracer's source limit (`vectorcraft_trace::MAX_SOURCE_PIXELS`,
/// 192 megapixels) is refused from the image's header before its pixels are decoded, and the run
/// leaves the document as it was.
#[test]
fn trace_large_image_is_bounded() {
    let base = folder("large");
    let png = base.join("main/wall.png");
    std::fs::write(&png, super::linkspanel::png_header(30_000, 30_000)).unwrap();
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let r = s.execute("file.place", &json!({"path": png.to_string_lossy(), "link": false})).unwrap();
    let image = NodeId(r["ids"][0].as_u64().unwrap());
    let steps = undo_steps(&s);
    let out = log(&mut s, &base, "var m = ''; try { app.activeDocument.rasterItems[0].trace(); } catch (e) { m = e.message; } vc.log(m);");
    let m = out[0].as_str().unwrap_or_default();
    assert!(m.contains("30000 × 30000 pixels") && m.contains("(192 megapixels)"), "{m}");
    assert!(traces(&s).is_empty());
    assert_eq!(top(&s), [image]);
    assert_eq!(undo_steps(&s), steps);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn trace_remove_all_takes_the_tracing() {
    let base = folder("remove-all");
    let (mut s, _) = session(&base, false);
    let out = log(&mut s, &base, "var d = app.activeDocument; d.rasterItems[0].trace(); d.pageItems.removeAll(); vc.log(d.pageItems.length);");
    assert_eq!(out, [json!(0)]);
    assert!(traces(&s).is_empty());
    assert!(top(&s).is_empty());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn trace_stop_rolls_back_long_work() {
    let base = folder("stop");
    let (mut s, image) = session(&base, false);
    let before = top(&s);
    s.script_in_run = true;
    let stop = s.script_interrupt();
    stop.set();
    let r = s.execute("imageTrace.make", &json!({"id": image.0}));
    stop.clear();
    s.script_in_run = false;
    assert!(matches!(r, Err(EngineError::Cancelled)), "{r:?}");
    assert_eq!(top(&s), before);
    assert!(traces(&s).is_empty());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn image_trace_refuses_the_source_image_of_a_trace() {
    let base = folder("source");
    let (mut s, image) = session(&base, false);
    let group = NodeId(s.execute("imageTrace.make", &json!({"id": image.0})).unwrap()["id"].as_u64().unwrap());
    let e = s.execute("imageTrace.make", &json!({"id": image.0})).unwrap_err().to_string();
    assert!(e.contains("source image of an Image Trace object"), "{e}");
    assert_eq!(traces(&s), [group]);
    assert_eq!(s.active().unwrap().doc.parent_of(image), Some(group));

    // With another image named after the source image, that other image is traced.
    let png = base.join("main/disc.png");
    let other = s.execute("file.place", &json!({"path": png.to_string_lossy(), "link": false})).unwrap();
    let other = NodeId(other["ids"][0].as_u64().unwrap());
    let r = s.execute("imageTrace.make", &json!({"ids": [image.0, other.0]})).unwrap();
    let second = NodeId(r["id"].as_u64().unwrap());
    let d = &s.active().unwrap().doc;
    assert_eq!(d.parent_of(other), Some(second));
    assert_eq!(d.parent_of(image), Some(group));
    assert_eq!(traces(&s).len(), 2);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn trace_of_another_image_inside_a_tracing() {
    let base = folder("inside");
    let (mut s, image) = session(&base, false);
    let png = base.join("main/disc.png");
    let other = s.execute("file.place", &json!({"path": png.to_string_lossy(), "link": false})).unwrap();
    let other = NodeId(other["ids"][0].as_u64().unwrap());
    let out = log(
        &mut s,
        &base,
        "var d = app.activeDocument, p = d.rasterItems[1].trace(), r = d.rasterItems[0]; r.move(p, ElementPlacement.PLACEATBEGINNING);\
         var q = r.trace(); vc.log(q.typename); vc.log(q.tracing.sourceArt === r); vc.log(p.tracing.pathCount > 0);",
    );
    assert_eq!(out, [json!("PluginItem"), json!(true), json!(true)]);
    let groups = traces(&s);
    assert_eq!(groups.len(), 2);
    let d = &s.active().unwrap().doc;
    let (a, b) = (d.parent_of(image).unwrap(), d.parent_of(other).unwrap());
    assert!(groups.contains(&a) && groups.contains(&b) && a != b, "each image is the source of its own tracing");
    assert!(d.parent_of(a) == Some(b) || d.parent_of(b) == Some(a), "one tracing is inside the other");
    let _ = std::fs::remove_dir_all(base);
}

/// An object moved or duplicated to the end of a tracing goes directly above its source image,
/// which stays the tracing's first child.
#[test]
fn an_object_placed_at_the_end_of_a_tracing_stays_above_its_source() {
    let base = folder("at-end");
    let (mut s, _) = session(&base, false);
    let png = base.join("main/disc.png");
    s.execute("file.place", &json!({"path": png.to_string_lossy(), "link": false})).unwrap();
    let out = log(
        &mut s,
        &base,
        "var d = app.activeDocument, a = d.rasterItems[1]; a.name = 'src'; var p = a.trace(), r = d.rasterItems[0]; r.name = 'moved';\
         r.duplicate(p, ElementPlacement.PLACEATEND); r.move(p, ElementPlacement.PLACEATEND);\
         var s = p.tracing.sourceArt; s.hidden = false; s.move(p, ElementPlacement.PLACEATEND);\
         vc.log(p.tracing.sourceArt.name); vc.log(d.rasterItems.length); vc.log(p.isTracing);",
    );
    assert_eq!(out, [json!("src"), json!(2), json!(true)]);
    let groups = traces(&s);
    assert_eq!(groups.len(), 1);
    let d = &s.active().unwrap().doc;
    let names: Vec<Option<String>> = d.children(Some(groups[0])).unwrap().iter().map(|n| n.name.clone()).collect();
    let name = |n: &str| Some(n.to_string());
    assert_eq!(names[..3], [name("src"), name("moved"), name("moved")], "the source stays first: {names:?}");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn trace_source_art_is_not_listed_with_the_images() {
    let base = folder("lists");
    let (mut s, _) = session(&base, false);
    let png = base.join("main/disc.png");
    s.execute("file.place", &json!({"path": png.to_string_lossy(), "link": false})).unwrap();
    let out = log(
        &mut s,
        &base,
        "var d = app.activeDocument; d.rasterItems[0].trace(); var n = d.rasterItems.length, k = 0;\
         for (var i = 0; i < d.pageItems.length; i++) { if (d.pageItems[i].typename === 'RasterItem') k++; }\
         d.rasterItems.removeAll(); vc.log(n); vc.log(k); vc.log(d.rasterItems.length); vc.log(d.pageItems[0].typename);",
    );
    assert_eq!(out, [json!(1), json!(1), json!(0), json!("PluginItem")]);
    assert_eq!(traces(&s).len(), 1, "removing the raster items leaves the tracing");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn trace_plugin_keeps_its_name() {
    let base = folder("renamed");
    let (mut s, _) = session(&base, false);
    let out = log(
        &mut s,
        &base,
        "var d = app.activeDocument, p = d.rasterItems[0].trace(), t = p.tracing, m = '', n = p.name; p.name = n;\
         try { p.name = 'logo'; } catch (e) { m = e.message; } vc.log(m); vc.log(p.name === n); vc.log(p.typename); vc.log(t.pathCount > 0);",
    );
    assert_eq!(out, [json!("The name of a tracing cannot be changed"), json!(true), json!("PluginItem"), json!(true)]);
    assert_eq!(traces(&s).len(), 1);
    assert_eq!(undo_steps(&s), 2, "the place and the run");
    let _ = std::fs::remove_dir_all(base);
}
