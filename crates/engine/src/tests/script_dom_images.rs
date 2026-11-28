//! Placed and raster items in the script object model: placing a file with `PlacedItems.add()`,
//! relinking, embedding, the encoded image's header, and the file checks.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::*;

/// A fresh folder for test `name`, with a `main` folder for the script and an `outside` folder the
/// script may not read, by its real path.
fn folder(name: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("vc-dom-images-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("main")).unwrap();
    std::fs::create_dir_all(base.join("outside")).unwrap();
    std::fs::canonicalize(&base).unwrap()
}

/// A `w`×`h` 8-bit gray PNG.
fn gray_png(w: u32, h: u32) -> Vec<u8> {
    let raw = vec![128u8; (w * h) as usize];
    let img = craft_image_codecs::Image::from_raw(w, h, craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8, raw).unwrap();
    craft_image_codecs::encode(&img, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default()).unwrap()
}

fn js_path(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "\\\\").replace('\'', "\\'")
}

/// A session with one 400 × 300 pt document.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
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

fn failure(s: &mut Session, base: &Path, text: &str) -> String {
    run(s, base, text).map(|r| panic!("{text} ran: {r}")).unwrap_err().to_string()
}

fn undo_steps(s: &Session) -> usize {
    s.active().unwrap().history.undo.len()
}

/// The images of the active document: (id, linked path).
fn images(s: &Session) -> Vec<(NodeId, Option<String>)> {
    let mut out = vec![];
    s.active().unwrap().doc.walk(|n| {
        if let NodeKind::Image(im) = &n.kind {
            out.push((n.id, im.link.as_ref().map(|l| l.path.clone())));
        }
    });
    out
}

/// Places `path` linked (`link`) with the engine's own command, outside any script.
fn place(s: &mut Session, path: &Path, link: bool) -> NodeId {
    let r = s.execute("file.place", &json!({"path": path.to_string_lossy(), "link": link})).unwrap();
    NodeId(r["ids"][0].as_u64().unwrap())
}

#[test]
fn placed_add_file_links_one_image() {
    let base = folder("add");
    let png = base.join("main/a.png");
    std::fs::write(&png, gray_png(40, 20)).unwrap();
    let mut s = session();
    let out = log(
        &mut s,
        &base,
        &format!(
            "var d = app.activeDocument, p = d.placedItems.add(); vc.log(p.file); vc.log(d.placedItems.length);\
             var m = ''; try {{ p.width; }} catch (e) {{ m = e.message; }} vc.log(m);\
             p.file = new File('{}'); vc.log(d.placedItems.length); vc.log(p.typename); vc.log(p.file.name);\
             vc.log(p.width); vc.log(d.placedItems[0].file.name); vc.log(d.rasterItems.length);\
             try {{ p.file = new File('{}'); }} catch (e) {{ m = e.message; }} vc.log(m);",
            js_path(&png),
            js_path(&png)
        ),
    );
    assert_eq!(out[..3], [json!(null), json!(0), json!("PlacedItem.file is required")]);
    assert_eq!(out[3..9], [json!(1), json!("PlacedItem"), json!("a.png"), json!(40), json!("a.png"), json!(0)]);
    assert_eq!(out[9], json!("PlacedItem.file is read-only: use relink"));
    let imgs = images(&s);
    assert_eq!(imgs.len(), 1);
    assert_eq!(imgs[0].1.as_deref().map(Path::new), Some(png.as_path()));
    assert_eq!(undo_steps(&s), 1);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn placed_file_refuses_bad_values_without_changing_the_document() {
    let base = folder("bad");
    std::fs::write(base.join("main/notes.txt"), "hello").unwrap();
    std::fs::write(base.join("outside/b.png"), gray_png(4, 4)).unwrap();
    let big = std::fs::File::create(base.join("main/big.png")).unwrap();
    big.set_len((256 << 20) + 1).unwrap();
    drop(big);
    let mut s = session();
    let cases = [
        ("'a.png'", "must be a File"),
        ("{}", "must be a File"),
        (&format!("new File('{}')", js_path(&base.join("main/notes.txt"))), "not an image"),
        (&format!("new File('{}')", js_path(&base.join("outside/b.png"))), "Script file access denied"),
        (&format!("new File('{}')", js_path(&base.join("main/big.png"))), "larger than 256 MiB"),
        (&format!("new File('{}')", js_path(&base.join("main/none.png"))), "does not exist"),
        (&format!("new Folder('{}')", js_path(&base.join("main"))), "must be a File"),
    ];
    for (value, message) in cases {
        let e = failure(&mut s, &base, &format!("app.activeDocument.placedItems.add().file = {value};"));
        assert!(e.contains(message), "{value}: {e}");
        assert!(!e.contains("outside"), "{value}: {e}");
        assert!(images(&s).is_empty(), "{value}");
        assert_eq!(undo_steps(&s), 0, "{value}");
    }
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn a_plain_object_shaped_like_a_file_is_not_one() {
    let base = folder("forged");
    std::fs::write(base.join("outside/b.png"), gray_png(4, 4)).unwrap();
    let mut s = session();
    let forged = js_path(&base.join("outside/b.png"));
    let e = failure(&mut s, &base, &format!("var f = {{}}; f['\\u0000fileRef'] = '{forged}'; app.activeDocument.placedItems.add().file = f;"));
    assert!(e.contains("not plain data"), "{e}");
    assert!(images(&s).is_empty());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn a_forged_or_altered_file_does_not_change_the_path() {
    let base = folder("forged-proto");
    let (a, b, c) = (base.join("main/a.png"), base.join("main/b.png"), base.join("outside/c.png"));
    std::fs::write(&a, gray_png(10, 10)).unwrap();
    std::fs::write(&b, gray_png(20, 10)).unwrap();
    std::fs::write(&c, gray_png(30, 10)).unwrap();
    let mut s = session();
    let forged = js_path(&c);
    let make = format!(
        "var f = Object.create(File.prototype); f.__vcPath = '{forged}';\
         Object.defineProperty(f, 'fsName', {{value: '{forged}', enumerable: true}});\
         Object.defineProperty(f, 'path', {{value: '{forged}', enumerable: true}});"
    );
    for target in ["app.activeDocument.placedItems.add().file = f;", "app.activeDocument.placedItems[0].relink(f);"] {
        if target.contains("relink") {
            place(&mut s, &a, true);
        }
        let e = failure(&mut s, &base, &format!("{make} {target}"));
        assert!(e.contains("must be a File") && !e.contains("c.png"), "{target}: {e}");
    }
    let id = images(&s)[0].0;
    assert_eq!(images(&s), vec![(id, Some(a.to_string_lossy().into_owned()))]);
    let alter = format!(
        "f.__vcPath = '{forged}'; Object.defineProperty(f, '__vcPath', {{value: '{forged}'}});\
         Object.defineProperty(f, 'fsName', {{value: '{forged}', enumerable: true}});"
    );
    let out = log(
        &mut s,
        &base,
        &format!(
            "var f = new File('{}'); {alter} var p = app.activeDocument.placedItems.add(); p.file = f; vc.log(p.file.name);\
             f = new File('{}'); {alter} p.relink(f); vc.log(p.file.name);",
            js_path(&b),
            js_path(&a)
        ),
    );
    assert_eq!(out, vec![json!("b.png"), json!("a.png")]);
    let paths: Vec<_> = images(&s).into_iter().map(|(_, p)| p).collect();
    assert_eq!(paths.len(), 2);
    assert!(paths.iter().all(|p| p.as_deref().map(Path::new) == Some(a.as_path())), "{paths:?}");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn a_placed_items_add_result_is_a_relative_object() {
    let base = folder("relative");
    let (a, b) = (base.join("main/a.png"), base.join("main/b.png"));
    std::fs::write(&a, gray_png(10, 10)).unwrap();
    std::fs::write(&b, gray_png(20, 10)).unwrap();
    let mut s = session();
    let first = place(&mut s, &a, true);
    let out = log(
        &mut s,
        &base,
        &format!(
            "var d = app.activeDocument, q = d.placedItems[0], p = d.placedItems.add(), m = '';\
             try {{ q.move(p, ElementPlacement.PLACEBEFORE); }} catch (e) {{ m = e.message; }} vc.log(m);\
             p.file = new File('{}'); q.move(p, ElementPlacement.PLACEBEFORE);",
            js_path(&b)
        ),
    );
    assert_eq!(out, vec![json!("PlacedItem.file is required")]);
    let order: Vec<NodeId> = images(&s).into_iter().map(|(id, _)| id).collect();
    assert_eq!(order.len(), 2);
    assert_eq!(order[1], first, "{order:?}");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn a_file_that_is_not_an_image_is_refused_before_placing() {
    let base = folder("text");
    let notes = base.join("main/notes.txt");
    std::fs::write(&notes, "hello").unwrap();
    let mut s = session();
    let out = log(
        &mut s,
        &base,
        &format!("try {{ app.activeDocument.placedItems.add().file = new File('{}'); }} catch (e) {{ vc.log(e.message); }}", js_path(&notes)),
    );
    assert_eq!(out, vec![json!("The file is not an image")]);
    assert_eq!(s.active().unwrap().doc.layers[0].children().map_or(0, |c| c.len()), 0);
    assert_eq!(undo_steps(&s), 0);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn placed_items_add_is_bounded() {
    let base = folder("pending");
    let mut s = session();
    let e = failure(&mut s, &base, "var p = app.activeDocument.placedItems; for (var i = 0; i < 1025; i++) p.add();");
    assert!(e.contains("at most 1024"), "{e}");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn relink_denied_path_preserves_old_link() {
    let base = folder("relink");
    let (a, b, c) = (base.join("main/a.png"), base.join("main/b.png"), base.join("outside/c.png"));
    std::fs::write(&a, gray_png(10, 10)).unwrap();
    std::fs::write(&b, gray_png(20, 10)).unwrap();
    std::fs::write(&c, gray_png(30, 10)).unwrap();
    let mut s = session();
    let id = place(&mut s, &a, true);
    let e = failure(&mut s, &base, &format!("app.activeDocument.placedItems[0].relink(new File('{}'));", js_path(&c)));
    assert!(e.contains("Script file access denied") && !e.contains("c.png"), "{e}");
    assert_eq!(images(&s), vec![(id, Some(a.to_string_lossy().into_owned()))]);
    let before = undo_steps(&s);
    let out = log(&mut s, &base, &format!("var p = app.activeDocument.placedItems[0]; p.relink(new File('{}')); vc.log(p.file.name);", js_path(&b)));
    assert_eq!(out, vec![json!("b.png")]);
    assert_eq!(images(&s), vec![(id, Some(b.to_string_lossy().into_owned()))]);
    assert_eq!(undo_steps(&s), before + 1);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn embed_reclassifies_and_stales_wrapper() {
    let base = folder("embed");
    let a = base.join("main/a.png");
    std::fs::write(&a, gray_png(10, 10)).unwrap();
    let mut s = session();
    let id = place(&mut s, &a, true);
    let out = log(
        &mut s,
        &base,
        "var d = app.activeDocument, p = d.placedItems[0]; p.embed(); var m = '';\
         try { p.name; } catch (e) { m = e.message; } vc.log(m);\
         vc.log(d.placedItems.length); var r = d.rasterItems[0]; vc.log(r.typename);\
         vc.log(r.embedded); vc.log(r.file); vc.log(r.status === RasterLinkState.NODATA); r.embedded = true;\
         try { r.embedded = false; } catch (e) { m = e.message; } vc.log(m);",
    );
    assert_eq!(out[..6], [json!("Stale page item"), json!(0), json!("RasterItem"), json!(true), json!(null), json!(true)]);
    assert!(out[6].as_str().unwrap().contains("cannot be linked"), "{out:?}");
    assert_eq!(images(&s), vec![(id, None)]);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn embed_gives_raster_item_a_new_object() {
    let base = folder("embed-new");
    let a = base.join("main/a.png");
    std::fs::write(&a, gray_png(10, 10)).unwrap();
    let mut s = session();
    place(&mut s, &a, true);
    let out = log(
        &mut s,
        &base,
        "var d = app.activeDocument, p = d.placedItems[0], q = d.pageItems[0]; vc.log(p === q); p.embed();\
         var r = d.rasterItems[0]; vc.log(p === r); vc.log(r.typename); vc.log(d.pageItems[0] === r); var m = '';\
         try { p.name; } catch (e) { m = e.message; } vc.log(m);",
    );
    assert_eq!(out, vec![json!(true), json!(false), json!("RasterItem"), json!(true), json!("Stale page item")]);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn missing_link_embed_reports_error() {
    let base = folder("missing");
    let a = base.join("main/a.png");
    std::fs::write(&a, gray_png(10, 10)).unwrap();
    let mut s = session();
    let id = place(&mut s, &a, true);
    std::fs::remove_file(&a).unwrap();
    let before = undo_steps(&s);
    let e = failure(&mut s, &base, "app.activeDocument.placedItems[0].embed();");
    assert!(e.contains("Linked image is missing"), "{e}");
    assert_eq!(images(&s), vec![(id, Some(a.to_string_lossy().into_owned()))]);
    assert_eq!(undo_steps(&s), before);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn raster_metadata_uses_encoded_header_not_rgb_fallback() {
    let base = folder("meta");
    let (g, c) = (base.join("main/g.png"), base.join("main/c.png"));
    std::fs::write(&g, gray_png(6, 4)).unwrap();
    std::fs::write(&c, super::links::png(3, 2, [10, 20, 30])).unwrap();
    let mut s = session();
    place(&mut s, &g, false);
    place(&mut s, &c, false);
    let out = log(
        &mut s,
        &base,
        "var r = app.activeDocument.rasterItems, t = [];\
         for (var i = 0; i < r.length; i++) t.push([r[i].imageColorSpace, r[i].channels, r[i].bitsPerChannel, r[i].transparent, r[i].colorizedGrayscale]);\
         vc.log(t); vc.log(r[1].boundingBox); vc.log(r[0].imageColorSpace === ImageColorSpace.RGB); var m = r[0].matrix; vc.log([m.typename, m.mValueA, m.mValueD]);",
    );
    // The collection lists the images topmost first, as `pageItems` does: the color image placed last comes first.
    assert_eq!(out[0], json!([["ImageColorSpace.RGB", 4, 8, true, false], ["ImageColorSpace.Grayscale", 1, 8, false, false]]));
    assert_eq!(out[1], json!([0, 4, 6, 0]));
    assert_eq!(out[2], json!(true));
    let m = out[3].as_array().unwrap();
    assert_eq!(m[0], json!("Matrix"));
    assert!(m[1].as_f64().unwrap() > 0.0 && m[2].as_f64().unwrap() < 0.0, "{m:?}");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn image_matrix_assignment_transforms_the_image() {
    let base = folder("matrix");
    let g = base.join("main/g.png");
    std::fs::write(&g, gray_png(10, 20)).unwrap();
    let mut s = session();
    let id = place(&mut s, &g, false);
    let before = undo_steps(&s);
    let out = log(
        &mut s,
        &base,
        "var r = app.activeDocument.rasterItems[0], m = r.matrix;\
         m.mValueA = 0; m.mValueB = 2; m.mValueC = -3; m.mValueD = 0; m.mValueTX = 50; m.mValueTY = 60; r.matrix = m;\
         var n = r.matrix; vc.log([n.mValueA, n.mValueB, n.mValueC, n.mValueD, n.mValueTX, n.mValueTY]); r.matrix = n;",
    );
    let got: Vec<f64> = out[0].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
    for (g, w) in got.iter().zip([0.0, 2.0, -3.0, 0.0, 50.0, 60.0]) {
        assert!((g - w).abs() < 1e-6, "{got:?}");
    }
    let xf = match &s.active().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Image(im) => im.xf.as_coeffs(),
        _ => panic!("not an image"),
    };
    assert!((xf[1] + 2.0).abs() < 1e-6 && (xf[2] + 3.0).abs() < 1e-6, "{xf:?}");
    assert_eq!(undo_steps(&s), before + 1);
    log(&mut s, &base, "var r = app.activeDocument.rasterItems[0]; r.matrix = r.matrix;");
    assert_eq!(undo_steps(&s), before + 1);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn image_matrix_refuses_bad_values_without_changing_the_image() {
    let base = folder("matrix-bad");
    let g = base.join("main/g.png");
    std::fs::write(&g, gray_png(10, 20)).unwrap();
    let mut s = session();
    let id = place(&mut s, &g, true);
    let xf = |s: &Session| match &s.active().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Image(im) => im.xf,
        _ => panic!("not an image"),
    };
    let (was, before) = (xf(&s), undo_steps(&s));
    let cases = [
        ("m.mValueA = NaN;", "mValueA is not a finite number"),
        ("m.mValueTY = -Infinity;", "mValueTY is not a finite number"),
        ("m.mValueB = '1';", "finite mValueB"),
        ("delete m.mValueTX;", "finite mValueTX"),
        ("m = 5;", "finite mValueA"),
        ("m.mValueA = 0; m.mValueD = 0;", "no area"),
        ("m.mValueA = 1e300;", "outside the canvas"),
        ("m.mValueTX = 9e6;", "outside the canvas"),
        ("m.mValueA = 1e6;", "outside the canvas"),
    ];
    for (change, message) in cases {
        let e = failure(&mut s, &base, &format!("var p = app.activeDocument.placedItems[0], m = p.matrix; {change} p.matrix = m;"));
        assert!(e.contains(message), "{change}: {e}");
        assert_eq!(xf(&s), was, "{change}");
        assert_eq!(undo_steps(&s), before, "{change}");
    }
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn image_file_getter_hides_out_of_scope_link() {
    let base = folder("hide");
    let b = base.join("outside/secret.png");
    std::fs::write(&b, gray_png(4, 4)).unwrap();
    let mut s = session();
    place(&mut s, &b, true);
    let e = failure(&mut s, &base, "vc.log(app.activeDocument.placedItems[0].file.fsName);");
    assert!(e.contains("Script file access denied") && !e.contains("secret"), "{e}");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn image_file_getter_hides_a_link_to_an_open_document_out_of_scope() {
    let base = folder("hidedoc");
    let b = base.join("outside/secret.png");
    std::fs::write(&b, gray_png(4, 4)).unwrap();
    let mut s = session();
    place(&mut s, &b, true);
    // The linked file is also the file of an open document, which the script's commands may read.
    s.doc_mut().unwrap().path = Some(b.to_string_lossy().into_owned());
    let e = failure(&mut s, &base, "vc.log(app.activeDocument.placedItems[0].file.fsName);");
    assert!(e.contains("Script file access denied") && !e.contains("secret"), "{e}");
    let _ = std::fs::remove_dir_all(base);
}

/// Assigning `file` or `matrix` checks the item before the value: an `add()` result with no file
/// throws `PlacedItem.file is required`, and one that was embedded throws `Stale page item`.
#[test]
fn assignments_check_the_item_before_the_value() {
    let base = folder("receiver");
    let a = base.join("main/a.png");
    std::fs::write(&a, gray_png(4, 4)).unwrap();
    let mut s = session();
    let out = log(
        &mut s,
        &base,
        &format!(
            "var d = app.activeDocument, n = d.placedItems.add(), p = d.placedItems.add(), m = [];\
             try {{ n.matrix = null; }} catch (e) {{ m.push(e.message); }}\
             p.file = new File('{0}'); p.embed();\
             try {{ p.file = new File('{0}'); }} catch (e) {{ m.push(e.message); }}\
             try {{ p.matrix = null; }} catch (e) {{ m.push(e.message); }}\
             try {{ d.rasterItems[0].matrix = null; }} catch (e) {{ m.push(e.name); }}\
             vc.log(m);",
            js_path(&a)
        ),
    );
    assert_eq!(out, vec![json!(["PlacedItem.file is required", "Stale page item", "Stale page item", "TypeError"])]);
    let _ = std::fs::remove_dir_all(base);
}

/// `removeAll()` deletes no image when one is on a locked artboard, in both collections.
#[test]
fn remove_all_keeps_images_on_a_locked_artboard() {
    let base = folder("locked-board");
    let a = base.join("main/a.png");
    std::fs::write(&a, gray_png(4, 4)).unwrap();
    let mut s = session();
    let linked = place(&mut s, &a, true);
    let embedded = place(&mut s, &a, false);
    let lock = |s: &mut Session, locked: bool| {
        s.edit("Lock Artboard", |d, _| {
            d.artboards[0].locked = locked;
            Ok(())
        })
        .unwrap();
    };
    lock(&mut s, true);
    let steps = undo_steps(&s);
    let e = failure(&mut s, &base, "app.activeDocument.placedItems.removeAll();");
    assert!(e.contains(&format!("cannot delete objects on a locked artboard: {}", linked.0)), "{e}");
    let e = failure(&mut s, &base, "app.activeDocument.rasterItems.removeAll();");
    assert!(e.contains(&format!("cannot delete objects on a locked artboard: {}", embedded.0)), "{e}");
    assert_eq!(images(&s).len(), 2);
    assert_eq!(undo_steps(&s), steps);
    lock(&mut s, false);
    let out = log(&mut s, &base, "var d = app.activeDocument; d.placedItems.removeAll(); d.rasterItems.removeAll(); vc.log(d.pageItems.length);");
    assert_eq!(out, vec![json!(0)]);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn stale_and_foreign_placed_items_are_refused() {
    let base = folder("stale");
    let a = base.join("main/a.png");
    std::fs::write(&a, gray_png(4, 4)).unwrap();
    let mut s = session();
    let id = place(&mut s, &a, true);
    let out = log(
        &mut s,
        &base,
        "var p = app.activeDocument.placedItems[0], m = []; p.remove();\
         try { p.relink(null); } catch (e) { m.push(e.message); }\
         try { app.activeDocument.placedItems.add().file; RasterItem.prototype.embedded.call(1); } catch (e) { m.push(e.name); }\
         try { PlacedItem.prototype.embed.call(app.activeDocument); } catch (e) { m.push(e.message); }\
         vc.log(m);",
    );
    assert_eq!(out, vec![json!(["Stale page item", "TypeError", "this is not a PlacedItem"])]);
    assert!(s.active().unwrap().doc.node(id).is_none());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn members_not_available_yet_check_the_image_first() {
    let base = folder("later");
    let a = base.join("main/a.png");
    std::fs::write(&a, gray_png(4, 4)).unwrap();
    let mut s = session();
    place(&mut s, &a, true);
    let out = log(
        &mut s,
        &base,
        "var d = app.activeDocument, p = d.placedItems[0], n = d.placedItems.add(), m = [];\
         try { n.contentVariable; } catch (e) { m.push(e.message); }\
         p.embed(); try { p.contentVariable; } catch (e) { m.push(e.message); }\
         var r = d.rasterItems[0]; try { r.colorize; } catch (e) { m.push(e.message); }\
         r.remove(); try { r.overprint; } catch (e) { m.push(e.message); }\
         vc.log(m);",
    );
    assert_eq!(out, vec![json!(["PlacedItem.file is required", "Stale page item", "RasterItem.colorize is not available yet", "Stale page item"])]);
    let _ = std::fs::remove_dir_all(base);
}

/// The `file` of a `PlacedItems.add()` result with no file is null while its document is open, and
/// the item is stale once that document is closed.
#[test]
fn a_pending_placed_items_file_is_stale_once_its_document_closes() {
    let base = folder("pending-closed");
    let mut s = session();
    let out = log(
        &mut s,
        &base,
        "var p = app.activeDocument.placedItems.add(); vc.log(p.file); vc.run('file.close', {});\
         var m = ''; try { vc.log(p.file); } catch (e) { m = e.message; } vc.log(m);",
    );
    assert_eq!(out, vec![json!(null), json!("Stale page item")]);
    let _ = std::fs::remove_dir_all(base);
}

/// Assigning `file` binds the placed item to the image `file.place` made, by its exact id, also
/// for an id above 2^53, which a script number does not hold exactly.
#[test]
fn a_placed_items_file_binds_the_exact_id_of_the_image_placed() {
    let base = folder("big-id");
    let png = base.join("main/a.png");
    std::fs::write(&png, gray_png(4, 4)).unwrap();
    let mut s = session();
    let big = (1u64 << 53) + 1;
    s.active_mut().unwrap().change_unsaved(|d| std::sync::Arc::make_mut(d).raise_next_id(big));
    let out = log(
        &mut s,
        &base,
        &format!("var p = app.activeDocument.placedItems.add(); p.file = new File('{}'); vc.log(p.file.name); vc.log(p.width);", js_path(&png)),
    );
    assert_eq!(out, vec![json!("a.png"), json!(4)]);
    let imgs = images(&s);
    assert_eq!(imgs.len(), 1, "{imgs:?}");
    assert_eq!(imgs[0].0, NodeId(big));
    let _ = std::fs::remove_dir_all(base);
}

/// A `PlacedItem` that was embedded is stale as the relative object of `move`: the raster item
/// it became is named by a fresh wrapper.
#[test]
fn a_stale_placed_item_is_refused_as_the_relative_object_of_move() {
    let base = folder("stale-relative");
    let a = base.join("main/a.png");
    std::fs::write(&a, gray_png(4, 4)).unwrap();
    let mut s = session();
    place(&mut s, &a, true);
    let rect = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    let out = log(
        &mut s,
        &base,
        "var d = app.activeDocument, p = d.placedItems[0], x = null, m = '';\
         for (var i = 0; i < d.pageItems.length; i++) { if (d.pageItems[i].typename == 'PathItem') x = d.pageItems[i]; }\
         p.embed(); try { x.move(p, ElementPlacement.PLACEAFTER); } catch (e) { m = e.message; } vc.log(m);\
         x.move(d.rasterItems[0], ElementPlacement.PLACEAFTER); vc.log([d.pageItems[0].typename, d.pageItems[1].typename]);",
    );
    assert_eq!(out, vec![json!("Stale page item"), json!(["RasterItem", "PathItem"])]);
    assert!(s.active().unwrap().doc.node(NodeId(rect)).is_some());
    let _ = std::fs::remove_dir_all(base);
}
