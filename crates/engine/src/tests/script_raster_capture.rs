//! Rasterizing and capturing art from a script: `Document.rasterize`, `Document.imageCapture`,
//! `RasterizeOptions`, `ImageCaptureOptions` and `object.rasterize` with `ids` and `clipBounds`.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::*;

fn run(s: &mut Session, source: &str) -> Result<Value> {
    s.execute("script.run", &json!({"source": source, "name": "t.js"}))
}

fn log(s: &mut Session, source: &str) -> Vec<Value> {
    let r = run(s, source).unwrap_or_else(|e| panic!("{source}: {e}"));
    r["log"].as_array().cloned().unwrap_or_default()
}

fn failure(s: &mut Session, source: &str) -> String {
    run(s, source).map(|r| panic!("{source} ran: {r}")).unwrap_err().to_string()
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap()["id"].as_u64().unwrap())
}

/// Fills the objects `ids` with `color` and takes their strokes off.
fn paint(s: &mut Session, ids: &[NodeId], color: &str) {
    let ids: Vec<u64> = ids.iter().map(|i| i.0).collect();
    s.execute("paint.setFill", &json!({"ids": ids, "color": color})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": ids, "none": true})).unwrap();
}

/// A 400 × 300 pt document with two rectangles without strokes: `a` at (10, 20) 30 × 40 pt, `b` at
/// (100, 100) 50 × 50 pt.
fn doc() -> (Session, NodeId, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let a = rect(&mut s, 10.0, 20.0, 30.0, 40.0);
    let b = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    paint(&mut s, &[a, b], "#ff0000");
    s.execute("select.none", &json!({})).unwrap();
    (s, a, b)
}

fn undo_steps(s: &Session) -> usize {
    s.active().unwrap().history.undo.len()
}

/// The size of each embedded image of the active document, in pixels.
fn images(s: &Session) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    s.active().unwrap().doc.walk(|n| {
        if let NodeKind::Image(i) = &n.kind {
            out.push((i.width, i.height));
        }
    });
    out
}

/// A script that selects `b` and rasterizes `a` (the second and first page items: the list starts
/// with the topmost object).
const RASTERIZE_A: &str = "var d = app.activeDocument, a = d.pageItems[1], b = d.pageItems[0]; b.name = 'b'; d.selection = [b];";

#[test]
fn script_rasterize_explicit_sources_preserves_selection() {
    let (mut s, a, b) = doc();
    let undo = undo_steps(&s);
    let out = log(
        &mut s,
        &format!(
            "{RASTERIZE_A} var r = d.rasterize(a); vc.log(r.typename); vc.log(d.selection.length); vc.log(d.selection[0].name === 'b');\
             try {{ a.name; }} catch (e) {{ vc.log(e.message); }} vc.log(d.pageItems.length);"
        ),
    );
    assert_eq!(out, [json!("RasterItem"), json!(1), json!(true), json!("Stale page item"), json!(2)]);
    let st = s.active().unwrap();
    assert!(st.doc.node(a).is_none(), "the source art is replaced");
    assert_eq!(st.selection.objects, vec![b], "the selection stays as it was");
    assert_eq!(undo_steps(&s), undo + 1, "one undo step");
}

#[test]
fn script_rasterize_defaults_override_document_raster_settings() {
    let (mut s, ..) = doc();
    s.execute("document.rasterEffectsSettings", &json!({"resolution": 72, "background": "transparent"})).unwrap();
    log(&mut s, &format!("{RASTERIZE_A} d.rasterize(a);"));
    // 30 × 40 pt at 300 ppi, snapped out to whole pixels (at 72 ppi it would be about 30 × 40).
    let [(w, h)] = images(&s)[..] else { panic!("one image: {:?}", images(&s)) };
    assert!((125..=126).contains(&w) && (167..=168).contains(&h), "300 ppi, the help page's default, not the document's 72: {w} × {h}");
    let (mut s, ..) = doc();
    log(&mut s, &format!("{RASTERIZE_A} var o = new RasterizeOptions(); o.resolution = 144; o.padding = 5; d.rasterize(a, undefined, o);"));
    assert_eq!(images(&s), vec![(80, 100)], "40 × 50 pt with the padding, at 144 ppi");
}

#[test]
fn script_rasterize_options_have_the_help_page_defaults() {
    let (mut s, ..) = doc();
    let out = log(
        &mut s,
        "var o = new RasterizeOptions(); vc.log([o.typename, o.antiAliasingMethod, o.backgroundBlack, o.clippingMask, o.colorModel, \
         o.convertSpotColors, o.convertTextToOutlines, o.includeLayers, o.padding, o.resolution, o.transparency]);\
         var c = new ImageCaptureOptions(); vc.log([c.typename, c.antiAliasing, c.matte, c.matteColor.red, c.matteColor.green, \
         c.matteColor.blue, c.resolution, c.transparency]); vc.log([AntiAliasingMethod.TYPEOPTIMIZED, RasterizationColorModel.BITMAP]);",
    );
    let rasterize = json!([
        "RasterizeOptions",
        "AntiAliasingMethod.ARTOPTIMIZED",
        false,
        false,
        "RasterizationColorModel.DEFAULTCOLORMODEL",
        false,
        false,
        false,
        0,
        300,
        false
    ]);
    assert_eq!(out[0], rasterize);
    assert_eq!(out[1], json!(["ImageCaptureOptions", false, false, 255, 255, 255, 150, false]));
    assert_eq!(out[2], json!(["AntiAliasingMethod.TYPEOPTIMIZED", "RasterizationColorModel.BITMAP"]));
}

#[test]
fn script_rasterize_clip_mask_returns_image_child() {
    let (mut s, ..) = doc();
    let out = log(
        &mut s,
        &format!(
            "{RASTERIZE_A} var o = new RasterizeOptions(); o.clippingMask = true; var r = d.rasterize([a], null, o); vc.log([r.typename, r.parent.typename]);"
        ),
    );
    assert_eq!(out, [json!(["RasterItem", "GroupItem"])]);
}

#[test]
fn script_rasterize_clip_bounds_cut_the_image() {
    let (mut s, ..) = doc();
    // The left half of `a`: x 10 to 25, and its whole height (y 280 to 240, Y up).
    log(&mut s, &format!("{RASTERIZE_A} var o = new RasterizeOptions(); o.resolution = 72; d.rasterize(a, [10, 280, 25, 240], o);"));
    assert_eq!(images(&s), vec![(15, 40)]);
}

#[test]
fn script_rasterize_disposes_several_sources_in_one_step() {
    let (mut s, a, b) = doc();
    let undo = undo_steps(&s);
    let out = log(&mut s, "var d = app.activeDocument; var r = d.rasterize([d.pageItems[1], d.pageItems[0]]); vc.log(d.pageItems.length);");
    assert_eq!(out, [json!(1)]);
    let st = s.active().unwrap();
    assert!(st.doc.node(a).is_none() && st.doc.node(b).is_none());
    assert_eq!(undo_steps(&s), undo + 1);
}

/// Each bad call throws and leaves the document as it was, with no undo step.
fn refused(s: &mut Session, source: &str, message: &str) {
    let before = s.active().unwrap().doc.clone();
    let undo = undo_steps(s);
    let e = failure(s, source);
    assert!(e.contains(message), "{source}: {e}");
    assert!(std::sync::Arc::ptr_eq(&before, &s.active().unwrap().doc) || *before == *s.active().unwrap().doc, "{source} changed the document");
    assert_eq!(undo_steps(s), undo, "{source} added an undo step");
}

#[test]
fn script_rasterize_rejects_other_documents_art() {
    let (mut s, ..) = doc();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    refused(&mut s, "app.documents[0].rasterize(app.documents[1].pageItems[0]);", "The source art must be in this document");
}

#[test]
fn script_rasterize_rejects_listed_twice() {
    let (mut s, ..) = doc();
    refused(&mut s, "var d = app.activeDocument; d.rasterize([d.pageItems[0], d.pageItems[0]]);", "listed twice");
}

#[test]
fn script_rasterize_rejects_locked_art() {
    let (mut s, ..) = doc();
    log(&mut s, "app.activeDocument.pageItems[0].locked = true;");
    refused(&mut s, "var d = app.activeDocument; d.rasterize(d.pageItems[0]);", "locked");
}

#[test]
fn script_rasterize_rejects_art_inside_listed_art() {
    let (mut s, a, b) = doc();
    s.execute("object.group", &json!({"ids": [a.0, b.0]})).unwrap();
    refused(&mut s, "var d = app.activeDocument; d.rasterize([d.pageItems[0], d.pageItems[1]]);", "which is listed too");
}

#[test]
fn script_rasterize_rejects_a_compound_path_member() {
    let (mut s, a, b) = doc();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    s.execute("object.compoundPath.make", &json!({})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    // The page items are the compound path, then its two paths.
    refused(&mut s, "var d = app.activeDocument; d.rasterize(d.pageItems[1]);", "part of a compound path");
    let out = log(&mut s, "var d = app.activeDocument; vc.log(d.rasterize(d.pageItems[0]).typename);");
    assert_eq!(out, [json!("RasterItem")], "the compound path itself is rasterized");
}

#[test]
fn script_rasterize_rejects_stale_and_missing_sources() {
    let (mut s, ..) = doc();
    refused(&mut s, "var d = app.activeDocument, a = d.pageItems[0]; a.remove(); d.rasterize(a);", "Stale page item");
    refused(&mut s, "app.activeDocument.rasterize([]);", "at least one page item");
    refused(&mut s, "app.activeDocument.rasterize();", "needs the source art");
    refused(&mut s, "app.activeDocument.rasterize(5);", "not a page item");
    let many = "var d = app.activeDocument, l = []; for (var i = 0; i < 10001; i++) l.push(d.pageItems[0]); d.rasterize(l);";
    refused(&mut s, many, "at most 10000 objects");
}

#[test]
fn script_rasterize_rejects_bad_options() {
    let (mut s, ..) = doc();
    let cases = [
        ("var o = new RasterizeOptions(); o.resolution = 71;", "from 72 to 2400"),
        ("var o = new RasterizeOptions(); o.resolution = 2401;", "from 72 to 2400"),
        ("var o = new RasterizeOptions(); o.resolution = NaN;", "finite number"),
        ("var o = new RasterizeOptions(); o.resolution = Infinity;", "finite number"),
        ("var o = new RasterizeOptions(); o.padding = 1001;", "from 0 to 1000"),
        ("var o = new RasterizeOptions(); o.padding = -Infinity;", "finite number"),
        ("var o = new RasterizeOptions(); o.antiAliasingMethod = 'smooth';", "AntiAliasingMethod value"),
        ("var o = new RasterizeOptions(); o.colorModel = AntiAliasingMethod.None;", "RasterizationColorModel value"),
        ("var o = new RasterizeOptions(); o.clippingMask = 1;", "true or false"),
        ("var o = new RasterizeOptions(); o.convertTextToOutlines = true;", "not available yet"),
        ("var o = new RasterizeOptions(); o.includeLayers = true;", "not available yet"),
        ("var o = new RasterizeOptions(); o.convertSpotColors = true;", "not available yet"),
        ("var o = new ImageCaptureOptions();", "RasterizeOptions object"),
        ("var o = {resolution: 300};", "RasterizeOptions object"),
    ];
    for (options, message) in cases {
        refused(&mut s, &format!("var d = app.activeDocument; {options} d.rasterize(d.pageItems[0], undefined, o);"), message);
    }
}

#[test]
fn script_rasterize_rejects_bad_clip_bounds() {
    let (mut s, ..) = doc();
    let cases = [
        ("[25, 280, 10, 240]", "right edge right of its left edge"),
        ("[10, 240, 25, 280]", "bottom edge below its top edge"),
        ("[10, 280, 25]", "four numbers"),
        ("[10, 280, 25, NaN]", "finite number"),
        ("[10, 280, 25, 1e300]", "outside the canvas"),
        ("[300, 20, 350, 10]", "does not overlap the art"),
        ("'all'", "four numbers"),
    ];
    for (bounds, message) in cases {
        refused(&mut s, &format!("var d = app.activeDocument; d.rasterize(d.pageItems[1], {bounds});"), message);
    }
    // `a` spans x 10 to 40: a rectangle at x 42 to 48 reaches only the 10 pt padding around it.
    refused(
        &mut s,
        "var d = app.activeDocument, o = new RasterizeOptions(); o.padding = 10; d.rasterize(d.pageItems[1], [42, 280, 48, 240], o);",
        "does not overlap the art",
    );
}

#[test]
fn script_rasterize_rejects_an_image_past_the_size_limit() {
    let (mut s, ..) = doc();
    let source = "var d = app.activeDocument, b = d.pageItems[0]; b.width = 3000; b.height = 3000; var o = new RasterizeOptions(); o.resolution = 2400; d.rasterize(b, undefined, o);";
    let e = failure(&mut s, source);
    assert!(e.contains("too large"), "{e}");
    assert_eq!(images(&s), Vec::<(u32, u32)>::new());
}

#[test]
fn object_rasterize_with_ids_keeps_the_selection_and_names_the_image() {
    let (mut s, a, b) = doc();
    s.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    let r = s.execute("object.rasterize", &json!({"ids": [a.0], "ppi": 72, "clippingMask": true})).unwrap();
    let st = s.active().unwrap();
    assert_eq!(st.selection.objects, vec![b]);
    let (id, image) = (NodeId(r["id"].as_u64().unwrap()), NodeId(r["imageId"].as_u64().unwrap()));
    assert!(matches!(st.doc.node(id).unwrap().kind, NodeKind::Group { .. }));
    assert!(matches!(st.doc.node(image).unwrap().kind, NodeKind::Image(_)));
    assert_eq!(st.doc.parent_of(image), Some(id));
    let r = s.execute("object.rasterize", &json!({"ppi": 72})).unwrap();
    assert_eq!(r["id"], r["imageId"], "without a clip group the image is the new object");
    assert_eq!(s.active().unwrap().selection.objects, vec![NodeId(r["id"].as_u64().unwrap())], "the selection is rasterized as before");
}

// ---- imageCapture ----

const DENIED: &str = "Script file access denied";

/// A fresh folder for test `name`, by its real path, with `main` for the script.
fn folder(name: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("vc-script-capture-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("main")).unwrap();
    std::fs::create_dir_all(base.join("outside")).unwrap();
    std::fs::canonicalize(&base).unwrap()
}

/// [`doc`] in a session whose scripts keep their data folders in `base`, and the data folder, where
/// scripts may write.
fn capture_doc(base: &Path) -> (Session, PathBuf) {
    let (mut s, ..) = doc();
    s.attach_script_data(base.join("settings/Script Data"));
    let dir = PathBuf::from(script(&mut s, base, "vc.log(Folder.userData.fsName);")[0].as_str().unwrap());
    (s, dir)
}

/// Runs `main/main.jsx` in `base` with the text `text`.
fn run_file(s: &mut Session, base: &Path, text: &str) -> Result<Value> {
    let main = base.join("main/main.jsx");
    // Reading `Folder.userData` makes the data folder, where the script may write.
    std::fs::write(&main, format!("Folder.userData; {text}").as_bytes()).unwrap();
    s.execute("script.run", &json!({"path": main.to_string_lossy()}))
}

fn script(s: &mut Session, base: &Path, text: &str) -> Vec<Value> {
    run_file(s, base, text).unwrap_or_else(|e| panic!("{text}: {e}"))["log"].as_array().cloned().unwrap_or_default()
}

fn file_failure(s: &mut Session, base: &Path, text: &str) -> String {
    run_file(s, base, text).map(|r| panic!("{text} ran: {r}")).unwrap_err().to_string()
}

fn quoted(p: &Path) -> String {
    serde_json::to_string(&p.to_string_lossy()).unwrap()
}

/// The PNG at `path`, decoded by the `image` crate, as RGBA.
fn png(path: &Path) -> image::RgbaImage {
    let bytes = std::fs::read(path).unwrap();
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"), "a PNG signature");
    image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).unwrap().to_rgba8()
}

#[test]
fn image_capture_whole_artwork_including_pasteboard() {
    let base = folder("whole");
    let (mut s, dir) = capture_doc(&base);
    // A third rectangle off the artboard, left of it: the art spans x -90 to 150 and y 20 to 150.
    let c = rect(&mut s, -90.0, 30.0, 20.0, 20.0);
    paint(&mut s, &[c], "#0000ff");
    // The capture's own 150 ppi applies, not the document's raster effects resolution.
    s.execute("document.rasterEffectsSettings", &json!({"resolution": 300})).unwrap();
    let undo = undo_steps(&s);
    let out = script(
        &mut s,
        &base,
        &format!("var r = app.activeDocument.imageCapture(new File({})); vc.log(r === undefined);", quoted(&dir.join("all.png"))),
    );
    assert_eq!(out, [json!(true)]);
    let img = png(&dir.join("all.png"));
    assert_eq!((img.width(), img.height()), (500, 271), "240 × 130 pt at 150 ppi");
    assert_eq!(img.get_pixel(5, 30).0, [0, 0, 255, 255], "the art off the artboard is there");
    assert_eq!(img.get_pixel(499, 0).0, [255, 255, 255, 255], "white where there is no art");
    assert_eq!(undo_steps(&s), undo, "a capture changes nothing in the document");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn image_capture_exact_clip_bounds_and_matte() {
    let base = folder("clip");
    let (mut s, dir) = capture_doc(&base);
    // x 0 to 50 and y 300 to 250 (Y up): the top-left corner of the artboard, with `a` in it.
    let f = |n: &str| quoted(&dir.join(n));
    let source = format!(
        "var d = app.activeDocument, o = new ImageCaptureOptions(); o.resolution = 72;\
         d.imageCapture(new File({}), [0, 300, 50, 250], o);\
         o.matte = true; o.matteColor = {{red: 0, green: 128, blue: 0}}; d.imageCapture(new File({}), [0, 300, 50, 250], o);\
         o.transparency = true; d.imageCapture(new File({}), [0, 300, 50, 250], o);",
        f("white.png"),
        f("matte.png"),
        f("clear.png")
    );
    script(&mut s, &base, &source);
    for (name, blank) in [("white.png", [255, 255, 255, 255]), ("matte.png", [0, 128, 0, 255]), ("clear.png", [0, 0, 0, 0])] {
        let img = png(&dir.join(name));
        assert_eq!((img.width(), img.height()), (50, 50), "{name}");
        assert_eq!(img.get_pixel(2, 2).0, blank, "{name}: the area without art");
        assert_eq!(img.get_pixel(20, 30).0, [255, 0, 0, 255], "{name}: the art");
    }
    let _ = std::fs::remove_dir_all(base);
}

/// Bounds that miss every object still give an image of that area, blank or transparent.
#[test]
fn image_capture_of_bounds_without_art_is_blank() {
    let base = folder("empty");
    let (mut s, dir) = capture_doc(&base);
    // x 300 to 350 and y 100 to 50 (Y up): below and right of both rectangles.
    let f = |n: &str| quoted(&dir.join(n));
    let source = format!(
        "var d = app.activeDocument, o = new ImageCaptureOptions(); o.resolution = 72;\
         d.imageCapture(new File({}), [300, 100, 350, 50], o);\
         o.transparency = true; d.imageCapture(new File({}), [300, 100, 350, 50], o);",
        f("blank.png"),
        f("clear.png")
    );
    let undo = undo_steps(&s);
    script(&mut s, &base, &source);
    for (name, blank) in [("blank.png", [255, 255, 255, 255]), ("clear.png", [0, 0, 0, 0])] {
        let img = png(&dir.join(name));
        assert_eq!((img.width(), img.height()), (50, 50), "{name}");
        assert!(img.pixels().all(|p| p.0 == blank), "{name}: every pixel is {blank:?}");
    }
    assert_eq!(undo_steps(&s), undo, "a capture changes nothing in the document");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn capture_rejects_64mp_and_side_limit_before_render() {
    let base = folder("huge");
    let (mut s, dir) = capture_doc(&base);
    let target = dir.join("huge.png");
    // 4000 × 4000 pt at 2400 ppi is far past 64 megapixels; 40,000 × 1 pt at 72 ppi is past 32,768 a side.
    let cases = [
        format!(
            "var o = new ImageCaptureOptions(); o.resolution = 2400; app.activeDocument.imageCapture(new File({}), [0, 4000, 4000, 0], o);",
            quoted(&target)
        ),
        format!(
            "var o = new ImageCaptureOptions(); o.resolution = 72; app.activeDocument.imageCapture(new File({}), [0, 1, 40000, 0], o);",
            quoted(&target)
        ),
    ];
    for case in &cases {
        let e = file_failure(&mut s, &base, case);
        assert!(e.contains("64 megapixels"), "{case}: {e}");
    }
    assert!(!target.exists());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn capture_rejects_bad_options_and_names() {
    let base = folder("bad");
    let (mut s, dir) = capture_doc(&base);
    let png = quoted(&dir.join("x.png"));
    let cases = [
        (
            format!("var o = new ImageCaptureOptions(); o.resolution = 50; app.activeDocument.imageCapture(new File({png}), undefined, o);"),
            "from 72 to 2400",
        ),
        (
            format!("var o = new ImageCaptureOptions(); o.resolution = Infinity; app.activeDocument.imageCapture(new File({png}), undefined, o);"),
            "finite number",
        ),
        (
            format!(
                "var o = new ImageCaptureOptions(); o.matteColor = {{red: NaN, green: 0, blue: 0}}; app.activeDocument.imageCapture(new File({png}), undefined, o);"
            ),
            "finite number",
        ),
        (format!("app.activeDocument.imageCapture(new File({png}), [0, 300, Infinity, 250]);"), "finite number"),
        (
            format!("var o = new ImageCaptureOptions(); o.matte = 'yes'; app.activeDocument.imageCapture(new File({png}), undefined, o);"),
            "true or false",
        ),
        (
            format!(
                "var o = new ImageCaptureOptions(); o.matteColor = {{red: 300, green: 0, blue: 0}}; app.activeDocument.imageCapture(new File({png}), undefined, o);"
            ),
            "from 0 to 255",
        ),
        (
            format!("var o = new ImageCaptureOptions(); o.matteColor = 'red'; app.activeDocument.imageCapture(new File({png}), undefined, o);"),
            "RGBColor",
        ),
        (format!("app.activeDocument.imageCapture(new File({png}), undefined, new RasterizeOptions());"), "ImageCaptureOptions object"),
        (format!("app.activeDocument.imageCapture(new File({}));", quoted(&dir.join("x.tif"))), "ends with .png"),
        (format!("app.activeDocument.imageCapture(new File({}));", quoted(&dir.join(".png"))), "ends with .png"),
        ("app.activeDocument.imageCapture('x.png');".to_string(), "needs a File"),
        (format!("app.activeDocument.imageCapture(new File({png}), [0, 0, 0, 0]);"), "right edge right of its left edge"),
        (format!("app.activeDocument.windowCapture(new File({}), [100, 100]);", quoted(&dir.join("w.tif"))), "not available yet"),
    ];
    for (case, message) in &cases {
        let e = file_failure(&mut s, &base, &format!("Folder.userData; {case}"));
        assert!(e.contains(message), "{case}: {e}");
    }
    assert_eq!(std::fs::read_dir(&dir).unwrap().filter(|e| e.as_ref().is_ok_and(|e| e.path().extension().is_some())).count(), 0, "nothing written");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn capture_denied_or_source_path_writes_nothing() {
    let base = folder("denied");
    let (mut s, dir) = capture_doc(&base);
    let outside = base.join("outside/x.png");
    let e = file_failure(&mut s, &base, &format!("Folder.userData; app.activeDocument.imageCapture(new File({}));", quoted(&outside)));
    assert!(e.contains(DENIED), "{e}");
    assert!(!outside.exists());
    // An open document whose file is named .png in the folder the script may write: its source.
    let source = dir.join("source.png");
    std::fs::write(&source, b"kept").unwrap();
    s.active_mut().unwrap().path = Some(source.to_string_lossy().into_owned());
    let e = file_failure(&mut s, &base, &format!("Folder.userData; app.activeDocument.imageCapture(new File({}));", quoted(&source)));
    assert!(e.contains("cannot write over an open document"), "{e}");
    assert_eq!(std::fs::read(&source).unwrap(), b"kept");
    // An image linked by a path relative to the document's folder, not to the working directory.
    let linked = dir.join("linked.png");
    std::fs::write(&linked, b"kept").unwrap();
    s.edit("Place", |d, _| {
        let id = d.alloc_id();
        let link = Some(vectorcraft_doc::LinkInfo { path: "linked.png".into(), relative: Some("linked.png".into()), ..Default::default() });
        let im = vectorcraft_doc::ImageObject {
            key: "pic".into(),
            width: 1,
            height: 1,
            xf: vectorcraft_geom::Affine::IDENTITY,
            link,
            placement: Default::default(),
            pixels: None,
        };
        d.insert(d.default_layer(), 0, vectorcraft_doc::Node::new(id, NodeKind::Image(Box::new(im))))?;
        Ok(())
    })
    .unwrap();
    let e =
        file_failure(&mut s, &base, &format!("Folder.userData; app.activeDocument.imageCapture(new File({}), [0, 300, 50, 250]);", quoted(&linked)));
    assert!(e.contains("cannot write over an open document"), "{e}");
    assert_eq!(std::fs::read(&linked).unwrap(), b"kept");
    // An image linked only from a symbol's art, outside the layers.
    let in_symbol = dir.join("in-symbol.png");
    std::fs::write(&in_symbol, b"kept").unwrap();
    s.edit("Symbol", |d, _| {
        let id = d.alloc_id();
        let link = Some(vectorcraft_doc::LinkInfo { path: in_symbol.to_string_lossy().into_owned(), ..Default::default() });
        let im = vectorcraft_doc::ImageObject {
            key: "symbol-pic".into(),
            width: 1,
            height: 1,
            xf: vectorcraft_geom::Affine::IDENTITY,
            link,
            placement: Default::default(),
            pixels: None,
        };
        let art = vectorcraft_doc::Node::new(id, NodeKind::Image(Box::new(im)));
        d.symbols.push(vectorcraft_doc::Symbol { name: "Mark".into(), art: std::sync::Arc::new(art), dynamic: false });
        Ok(())
    })
    .unwrap();
    let e = file_failure(
        &mut s,
        &base,
        &format!("Folder.userData; app.activeDocument.imageCapture(new File({}), [0, 300, 50, 250]);", quoted(&in_symbol)),
    );
    assert!(e.contains("cannot write over an open document"), "{e}");
    assert_eq!(std::fs::read(&in_symbol).unwrap(), b"kept");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn capture_to_a_missing_links_path_writes_nothing() {
    let base = folder("missing-link");
    let (mut s, dir) = capture_doc(&base);
    s.active_mut().unwrap().path = Some(dir.join("doc.vectorcraft").to_string_lossy().into_owned());
    // An image linked by a path next to the document whose file is not there.
    s.edit("Place", |d, _| {
        let id = d.alloc_id();
        let link = Some(vectorcraft_doc::LinkInfo { path: "gone.png".into(), relative: Some("gone.png".into()), ..Default::default() });
        let im = vectorcraft_doc::ImageObject {
            key: "gone".into(),
            width: 1,
            height: 1,
            xf: vectorcraft_geom::Affine::IDENTITY,
            link,
            placement: Default::default(),
            pixels: None,
        };
        d.insert(d.default_layer(), 0, vectorcraft_doc::Node::new(id, NodeKind::Image(Box::new(im))))?;
        Ok(())
    })
    .unwrap();
    let gone = dir.join("gone.png");
    assert!(!gone.exists());
    let e =
        file_failure(&mut s, &base, &format!("Folder.userData; app.activeDocument.imageCapture(new File({}), [0, 300, 50, 250]);", quoted(&gone)));
    assert!(e.contains("cannot write over an open document"), "{e}");
    assert!(!gone.exists(), "nothing written");
    // The same name in other letter case is the linked file only where the volume ignores case.
    std::fs::write(dir.join("probe.txt"), b"x").unwrap();
    let volume_ignores_case = dir.join("PROBE.TXT").exists();
    let upper = dir.join("GONE.png");
    let text = format!("Folder.userData; app.activeDocument.imageCapture(new File({}), [0, 300, 50, 250]);", quoted(&upper));
    if volume_ignores_case {
        assert!(file_failure(&mut s, &base, &text).contains("cannot write over an open document"));
        assert!(!gone.exists(), "nothing written");
    } else {
        script(&mut s, &base, &text);
        assert!(upper.exists() && !gone.exists(), "a different file on a volume that keeps case");
        // Two names for one file that differ only in case show that the folder keeps case.
        let sub = dir.join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("probe.txt"), b"x").unwrap();
        std::fs::hard_link(sub.join("probe.txt"), sub.join("PROBE.TXT")).unwrap();
        s.edit("Place", |d, _| {
            let id = d.alloc_id();
            let link = Some(vectorcraft_doc::LinkInfo { path: "sub/gone.png".into(), relative: Some("sub/gone.png".into()), ..Default::default() });
            let im = vectorcraft_doc::ImageObject {
                key: "sub-gone".into(),
                width: 1,
                height: 1,
                xf: vectorcraft_geom::Affine::IDENTITY,
                link,
                placement: Default::default(),
                pixels: None,
            };
            d.insert(d.default_layer(), 0, vectorcraft_doc::Node::new(id, NodeKind::Image(Box::new(im))))?;
            Ok(())
        })
        .unwrap();
        let sub_upper = sub.join("GONE.png");
        script(&mut s, &base, &format!("Folder.userData; app.activeDocument.imageCapture(new File({}), [0, 300, 50, 250]);", quoted(&sub_upper)));
        assert!(sub_upper.exists() && !sub.join("gone.png").exists());
    }
    let _ = std::fs::remove_dir_all(base);
}

/// Places a linked image of the key `key` with `link` on the active document's default layer.
fn place_linked(s: &mut Session, key: &str, link: vectorcraft_doc::LinkInfo) {
    s.edit("Place", |d, _| {
        let id = d.alloc_id();
        let im = vectorcraft_doc::ImageObject {
            key: key.into(),
            width: 1,
            height: 1,
            xf: vectorcraft_geom::Affine::IDENTITY,
            link: Some(link),
            placement: Default::default(),
            pixels: None,
        };
        d.insert(d.default_layer(), 0, vectorcraft_doc::Node::new(id, NodeKind::Image(Box::new(im))))?;
        Ok(())
    })
    .unwrap();
}

/// An untitled document opened from a template finds a link by its relative path from the
/// template's folder, so a capture there is refused, also when the file is missing.
#[test]
fn capture_to_a_templates_relative_link_writes_nothing() {
    let base = folder("template-link");
    let (mut s, dir) = capture_doc(&base);
    let st = s.active_mut().unwrap();
    st.path = None;
    st.links_from = Some(dir.join("t.vctemplate").to_string_lossy().into_owned());
    let stale = base.join("outside/gone/pic.png").to_string_lossy().into_owned();
    place_linked(&mut s, "pic", vectorcraft_doc::LinkInfo { path: stale, relative: Some("pic.png".into()), ..Default::default() });
    let target = dir.join("pic.png");
    let e = file_failure(&mut s, &base, &format!("app.activeDocument.imageCapture(new File({}), [0, 300, 50, 250]);", quoted(&target)));
    assert!(e.contains("cannot write over an open document"), "{e}");
    assert!(!target.exists(), "nothing written");
    let _ = std::fs::remove_dir_all(base);
}

/// Two images that share a stale path and a content but keep different relative paths are each
/// looked for at their own relative path, so a capture over either one's file is refused.
#[test]
fn capture_over_either_relative_path_of_one_linked_content_writes_nothing() {
    let base = folder("two-relatives");
    let (mut s, dir) = capture_doc(&base);
    s.active_mut().unwrap().path = Some(dir.join("doc.vectorcraft").to_string_lossy().into_owned());
    let stale = base.join("outside/gone/pic.png").to_string_lossy().into_owned();
    for relative in ["a.png", "b.png"] {
        place_linked(&mut s, "pic", vectorcraft_doc::LinkInfo { path: stale.clone(), relative: Some(relative.into()), ..Default::default() });
    }
    for name in ["a.png", "b.png"] {
        let target = dir.join(name);
        let e = file_failure(&mut s, &base, &format!("app.activeDocument.imageCapture(new File({}), [0, 300, 50, 250]);", quoted(&target)));
        assert!(e.contains("cannot write over an open document"), "{name}: {e}");
        assert!(!target.exists(), "{name}: nothing written");
    }
    let _ = std::fs::remove_dir_all(base);
}

/// A DCS image whose stored path is stale reads its plate files beside the main file its relative
/// path finds, so a capture over one of those is refused.
#[test]
fn capture_over_a_plate_beside_a_found_dcs_file_writes_nothing() {
    use vectorcraft_doc::separated::{InkPlate, PlateFile, Separated};
    let base = folder("dcs-plate");
    let (mut s, dir) = capture_doc(&base);
    s.active_mut().unwrap().path = Some(dir.join("doc.vectorcraft").to_string_lossy().into_owned());
    std::fs::write(dir.join("main.eps"), b"main").unwrap();
    let plate = dir.join("cyan.png");
    std::fs::write(&plate, b"kept").unwrap();
    let separated = Separated::new(1, 1, vec![InkPlate { name: "Cyan".into(), cmyk: [1.0, 0.0, 0.0, 0.0], data: vec![255] }]).unwrap();
    let files = [PlateFile { name: "cyan.png".into(), size: 4, modified: 0 }];
    let blob = vectorcraft_doc::ImageBlob::separated_tiff(&separated, None, &files).unwrap();
    s.edit("Blob", |d, _| {
        d.images.insert("dcs".into(), blob);
        Ok(())
    })
    .unwrap();
    let stale = base.join("outside/gone/main.eps").to_string_lossy().into_owned();
    place_linked(&mut s, "dcs", vectorcraft_doc::LinkInfo { path: stale, relative: Some("main.eps".into()), ..Default::default() });
    let e = file_failure(&mut s, &base, &format!("app.activeDocument.imageCapture(new File({}), [0, 300, 50, 250]);", quoted(&plate)));
    assert!(e.contains("cannot write over an open document"), "{e}");
    assert_eq!(std::fs::read(&plate).unwrap(), b"kept");
    let _ = std::fs::remove_dir_all(base);
}

/// A document the interface opens while the run is paused is open too: a capture over its file is
/// refused.
#[test]
fn capture_over_a_document_opened_while_paused_writes_nothing() {
    let base = folder("paused-open");
    let (mut s, dir) = capture_doc(&base);
    let target = dir.join("opened.png");
    let main = base.join("main/main.jsx");
    let text = format!(
        "Folder.userData; for (var i = 0; i < 2000000; i++) {{}}\
         try {{ app.activeDocument.imageCapture(new File({}), [0, 300, 50, 250]); vc.log('written'); }} catch (e) {{ vc.log(e.message); }}",
        quoted(&target)
    );
    std::fs::write(&main, text.as_bytes()).unwrap();
    s.script_frames = true;
    assert_eq!(s.execute("script.run", &json!({"path": main.to_string_lossy()})).unwrap()["running"], true);
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"/>"#;
    crate::cmd::fileio::open_bytes(&mut s, "opened.svg", svg, Some(target.to_string_lossy().into_owned())).unwrap();
    std::fs::write(&target, b"kept").unwrap();
    let r = loop {
        if let Some(r) = s.script_pump() {
            break r.unwrap();
        }
    };
    s.script_frames = false;
    let out = r["log"].as_array().cloned().unwrap_or_default();
    assert!(out.len() == 1 && out[0].as_str().is_some_and(|m| m.contains("cannot write over an open document")), "{out:?}");
    assert_eq!(std::fs::read(&target).unwrap(), b"kept");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn capture_written_before_a_failure_is_reported() {
    let base = folder("report");
    let (mut s, dir) = capture_doc(&base);
    let target = dir.join("kept.png");
    let r = run_file(&mut s, &base, &format!("app.activeDocument.imageCapture(new File({})); throw new Error('later');", quoted(&target)));
    let e = r.unwrap_err().to_string();
    assert!(e.contains("later") && e.contains("kept.png"), "the failure names the file it wrote: {e}");
    assert!(target.is_file());
    let _ = std::fs::remove_dir_all(base);
}
