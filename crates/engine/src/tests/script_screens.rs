//! Exporting for screens from a script: `Document.exportForScreens`, `ExportForScreensItemToExport`,
//! the option classes and the file grants the folder goes through.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::*;

const DENIED: &str = "Script file access denied";

#[test]
fn screen_width_and_height_reject_effective_scales_outside_the_encoder_range() {
    for mode in ["SCALEBYWIDTH", "SCALEBYHEIGHT"] {
        for pixels in [65.0, 0.001] {
            let base = vectorcraft_testkit::TempDir::new("screen-size-limit");
            std::fs::create_dir_all(base.join("main")).unwrap();
            let mut s = session(&base);
            let dir = data_folder(&mut s, &base);
            let output = dir.join("scaled");
            let source = format!(
                "Folder.userData; var d = app.documents.add(DocumentColorSpace.RGB, 1, 1); \
                 var o = new ExportForScreensOptionsPNG24(); \
                 o.scaleType = ExportForScreensScaleType.{mode}; o.scaleTypeValue = {pixels}; \
                 d.exportForScreens(new Folder({}), ExportForScreensType.SE_PNG24, o);",
                quoted(&output)
            );
            let error = failure(&mut s, &base, &source);
            assert!(error.contains("scale"), "{mode}: {pixels}: {error}");
            assert!(!output.exists(), "{mode}: {pixels}");
        }
    }
}

#[test]
fn screen_png8_without_matting_keeps_transparent_edge_colors() {
    let base = vectorcraft_testkit::TempDir::new("screen-png8-unmatted");
    std::fs::create_dir_all(base.join("main")).unwrap();
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    logged(
        &mut s,
        &base,
        &format!(
            "Folder.userData; var d = app.documents.add(DocumentColorSpace.RGB, 20, 20); \
         vc.run('shape.rectangle', {{x: 0, y: 0, width: 20, height: 20}}); \
         vc.run('paint.setFill', {{color: '#ff0000'}}); d.pageItems[0].opacity = 50; \
         var o = new ExportForScreensOptionsPNG8(); o.matte = false; \
         d.exportForScreens(new Folder({}), ExportForScreensType.SE_PNG8, o);",
            quoted(&dir.join("png8"))
        ),
    );
    let output = dir.join("png8");
    let files = names(&output);
    assert_eq!(files.len(), 1);
    let rgba = image::open(output.join(&files[0])).unwrap().to_rgba8();
    let pixel = rgba.get_pixel(5, 5).0;
    assert!(pixel[0] > 200 && pixel[1] < 20 && pixel[2] < 20, "{pixel:?}");
}

#[test]
fn linked_screen_svg_images_are_rejected_before_output_is_created() {
    let base = vectorcraft_testkit::TempDir::new("screen-svg-linked");
    std::fs::create_dir_all(base.join("main")).unwrap();
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    s.execute("file.new", &json!({"width": 20, "height": 20})).unwrap();
    let output = dir.join("svg");
    let error = failure(
        &mut s,
        &base,
        &format!(
            "Folder.userData; var o = new ExportForScreensOptionsWebOptimizedSVG(); \
         o.rasterImageLocation = RasterImageLocation.LINK; \
         app.activeDocument.exportForScreens(new Folder({}), ExportForScreensType.SE_SVG, o);",
            quoted(&output)
        ),
    );
    assert!(error.contains("not available yet"), "{error}");
    assert!(!output.exists());
}

/// A volume root the person picked is a folder the export may write into: the export goes on to
/// the command, which here refuses an asset the document does not have, and writes nothing.
#[cfg(unix)]
#[test]
fn a_picked_volume_root_is_a_folder_to_export_into() {
    let base = vectorcraft_testkit::TempDir::new("screen-root");
    std::fs::create_dir_all(base.join("main")).unwrap();
    let mut s = session(&base);
    s.execute("file.new", &json!({"width": 20, "height": 20})).unwrap();
    s.script_frames = true;
    let main = base.join("main/main.jsx");
    let script = "var p = Folder.selectDialog('p'); var i = new ExportForScreensItemToExport(); i.artboards = ''; i.assets = [987654321];\n\
                  try { app.activeDocument.exportForScreens(p, ExportForScreensType.SE_PNG24, undefined, i); vc.log('exported'); } catch (e) { vc.log(String(e)); }";
    std::fs::write(&main, script).unwrap();
    assert_eq!(s.execute("script.run", &json!({"path": main.to_string_lossy()})).unwrap()["running"], true);
    let mut picked = false;
    let mut report = None;
    for _ in 0..10_000 {
        if !picked && s.script_picker().is_some() {
            picked = true;
            assert!(s.script_pick(crate::script::files::PickAnswer::Allow { paths: vec!["/".into()], always: false }));
        }
        if let Some(r) = s.script_pump() {
            report = Some(r.unwrap());
            break;
        }
    }
    let report = report.unwrap_or_else(|| panic!("the run neither ended nor waited"));
    assert!(picked, "{report}");
    let said = report["log"][0].as_str().unwrap_or_default().to_string();
    assert!(!said.contains(DENIED) && !said.contains("exported") && said.contains("987654321"), "{said}");
}

/// A fresh folder for test `name`, by its real path, with `main` for the script.
fn folder(name: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("vc-script-screens-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("main")).unwrap();
    std::fs::create_dir_all(base.join("outside")).unwrap();
    std::fs::canonicalize(&base).unwrap()
}

/// A session whose scripts keep their data folders in `base/settings/Script Data`.
fn session(base: &Path) -> Session {
    let mut s = Session::new();
    s.attach_script_data(base.join("settings/Script Data"));
    s
}

/// Runs `main/main.jsx` in `base` with the text `text` in `s`.
fn run(s: &mut Session, base: &Path, text: &str) -> Result<Value> {
    let main = base.join("main/main.jsx");
    std::fs::write(&main, text.as_bytes()).unwrap();
    s.execute("script.run", &json!({"path": main.to_string_lossy()}))
}

fn logged(s: &mut Session, base: &Path, text: &str) -> Value {
    run(s, base, text).unwrap_or_else(|e| panic!("{text}: {e}"))["log"].clone()
}

fn failure(s: &mut Session, base: &Path, text: &str) -> String {
    run(s, base, text).map(|r| panic!("{text} ran: {r}")).unwrap_err().to_string()
}

/// The script's data folder, where it may write.
fn data_folder(s: &mut Session, base: &Path) -> PathBuf {
    PathBuf::from(logged(s, base, "vc.log(Folder.userData.fsName);")[0].as_str().unwrap())
}

fn quoted(p: &Path) -> String {
    serde_json::to_string(&p.to_string_lossy()).unwrap()
}

/// The names of the files in `dir`, sorted; none when it is not there.
fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> =
        std::fs::read_dir(dir).map(|r| r.map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    v.sort();
    v
}

/// A script that makes a document of two artboards with a rectangle on the first, in `dir`.
fn two_artboards(dir: &Path) -> String {
    format!(
        "Folder.userData; var dir = {}, d = app.documents.add(DocumentColorSpace.RGB, 200, 100);\n\
         vc.run('artboard.new', {{x: 300, y: 0, width: 60, height: 80}});\n\
         vc.run('shape.rectangle', {{x: 10, y: 10, width: 50, height: 20}});\n",
        quoted(dir)
    )
}

/// Each export type writes its format into the folder, a file per artboard of the item's range, at
/// the size the options' scale type gives.
#[test]
fn export_for_screens_writes_each_format_into_the_folder() {
    let base = folder("formats");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    let out = logged(
        &mut s,
        &base,
        &format!(
            "{}var one = new ExportForScreensItemToExport(); one.artboards = '1';\n\
             vc.log([one.typename, new ExportForScreensItemToExport().artboards, new ExportForScreensOptionsJPEG().scaleTypeValue]);\n\
             d.exportForScreens(new Folder(dir + '/jpg'), ExportForScreensType.SE_JPEG100, undefined, one);\n\
             var p = new ExportForScreensOptionsPNG24(); p.scaleType = ExportForScreensScaleType.SCALEBYFACTOR; p.scaleTypeValue = 2;\n\
             d.exportForScreens(new Folder(dir + '/png'), ExportForScreensType.SE_PNG24, p);\n\
             var w = new ExportForScreensOptionsPNG8(); w.scaleType = ExportForScreensScaleType.SCALEBYWIDTH; w.scaleTypeValue = 50;\n\
             d.exportForScreens(new Folder(dir + '/png8'), ExportForScreensType.SE_PNG8, w, one);\n\
             var r = new ExportForScreensOptionsJPEG(); r.scaleType = ExportForScreensScaleType.SCALEBYRESOLUTION; r.scaleTypeValue = 144;\n\
             d.exportForScreens(new Folder(dir + '/ppi'), ExportForScreensType.SE_JPEG20, r, one);\n\
             d.exportForScreens(new Folder(dir + '/svg'), ExportForScreensType.SE_SVG);\n\
             d.exportForScreens(new Folder(dir + '/pdf'), ExportForScreensType.SE_PDF, new ExportForScreensPDFOptions(), one);",
            two_artboards(&dir)
        ),
    );
    assert_eq!(out[0], json!(["ExportForScreensItemToExport", "all", 1]));
    let size = |sub: &str| {
        let files = names(&dir.join(sub));
        let image = image::open(dir.join(sub).join(&files[0])).unwrap();
        (files, image.width(), image.height())
    };
    let (jpg, w, h) = size("jpg");
    assert!(jpg.len() == 1 && jpg[0].ends_with(".jpg"), "one artboard: {jpg:?}");
    assert_eq!((w, h), (200, 100), "a pixel per point at 1x");
    let (png, w, h) = size("png");
    assert!(png.len() == 2 && png.iter().all(|n| n.ends_with("@2x.png")), "both artboards at 2x: {png:?}");
    assert_eq!((w, h), (400, 200), "{png:?}");
    let (png8, w, h) = size("png8");
    assert_eq!(png8.len(), 1, "{png8:?}");
    assert_eq!((w, h), (50, 25), "50 pixels wide, in proportion");
    let (_, w, h) = size("ppi");
    assert_eq!((w, h), (400, 200), "144 ppi is two pixels per point");
    let svg = names(&dir.join("svg"));
    assert!(svg.len() == 2 && svg.iter().all(|n| n.ends_with(".svg")), "{svg:?}");
    let pdf = names(&dir.join("pdf"));
    assert_eq!(pdf.len(), 1, "{pdf:?}");
    assert!(std::fs::read(dir.join("pdf").join(&pdf[0])).unwrap().starts_with(b"%PDF"));
    assert_eq!(s.active().unwrap().path, None, "the export leaves the document's file as it is");
    let _ = std::fs::remove_dir_all(base);
}

/// Without options, an export writes what an untouched new options object of its class gives.
#[test]
fn omitted_options_are_a_new_options_objects_defaults() {
    let base = folder("defaults");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    let types = [
        ("SE_JPEG80", "ExportForScreensOptionsJPEG"),
        ("SE_PNG24", "ExportForScreensOptionsPNG24"),
        ("SE_PNG8", "ExportForScreensOptionsPNG8"),
        ("SE_SVG", "ExportForScreensOptionsWebOptimizedSVG"),
    ];
    let calls: String = types
        .iter()
        .map(|(t, c)| {
            format!(
                "d.exportForScreens(new Folder(dir + '/{t}-a'), ExportForScreensType.{t});\n\
                 d.exportForScreens(new Folder(dir + '/{t}-b'), ExportForScreensType.{t}, new {c}());\n"
            )
        })
        .collect();
    logged(&mut s, &base, &format!("{}{calls}", two_artboards(&dir)));
    for (t, _) in types {
        let (a, b) = (dir.join(format!("{t}-a")), dir.join(format!("{t}-b")));
        assert!(names(&a) == names(&b) && names(&a).len() == 2, "{t}: {:?} {:?}", names(&a), names(&b));
        for n in names(&a) {
            assert!(std::fs::read(a.join(&n)).unwrap() == std::fs::read(b.join(&n)).unwrap(), "{t}: {n} differs");
        }
    }
    let _ = std::fs::remove_dir_all(base);
}

/// `itemToExport` exports the artboards of its range, its assets and the whole document, each that
/// it names; one that names nothing throws and writes nothing.
#[test]
fn item_to_export_picks_artboards_assets_and_the_document() {
    let base = folder("items");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    logged(
        &mut s,
        &base,
        &format!(
            "{}var ids = vc.run('assets.add', {{name: 'Logo', multiple: false}}).assets;\n\
             var a = new ExportForScreensItemToExport(); a.artboards = ''; a.assets = ids;\n\
             d.exportForScreens(new Folder(dir + '/assets'), ExportForScreensType.SE_SVG, undefined, a);\n\
             var whole = new ExportForScreensItemToExport(); whole.artboards = ''; whole.document = true;\n\
             d.exportForScreens(new Folder(dir + '/document'), ExportForScreensType.SE_PNG24, undefined, whole);\n\
             var both = new ExportForScreensItemToExport(); both.assets = ids; both.artboards = '2';\n\
             d.exportForScreens(new Folder(dir + '/both'), ExportForScreensType.SE_SVG, undefined, both);",
            two_artboards(&dir)
        ),
    );
    assert_eq!(names(&dir.join("assets")), ["Logo.svg"]);
    let whole = names(&dir.join("document"));
    assert_eq!(whole.len(), 1, "{whole:?}");
    let image = image::open(dir.join("document").join(&whole[0])).unwrap();
    let (w, h) = (image.width(), image.height());
    assert!((50..60).contains(&w) && (20..30).contains(&h), "the bounds of the document's art, not an artboard: {w}x{h}");
    let both = names(&dir.join("both"));
    assert!(both.len() == 2 && both.contains(&"Logo.svg".to_string()), "artboard 2 and the asset: {both:?}");
    for empty in ["''", "' '"] {
        let e = failure(
            &mut s,
            &base,
            &format!(
                "Folder.userData; var i = new ExportForScreensItemToExport(); i.artboards = {empty};\n\
                 app.activeDocument.exportForScreens(new Folder({} + '/none'), ExportForScreensType.SE_PNG24, undefined, i);",
                quoted(&dir)
            ),
        );
        assert!(e.contains("nothing to export"), "{empty}: {e}");
    }
    assert!(!dir.join("none").exists(), "an empty item writes nothing");
    let _ = std::fs::remove_dir_all(base);
}

/// An asset named as an artboard of the item's range gets a file of its own: the artboard's file
/// keeps its name and the asset's is numbered, in place of writing over it.
#[test]
fn an_asset_named_as_an_artboard_does_not_overwrite_its_file() {
    let base = folder("clash");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    logged(
        &mut s,
        &base,
        &format!(
            "{}vc.run('artboard.setProps', {{artboards: [{{index: 0, name: 'Logo'}}]}});\n\
             var ids = vc.run('assets.add', {{name: 'Logo', multiple: false}}).assets;\n\
             var both = new ExportForScreensItemToExport(); both.artboards = '1'; both.assets = ids;\n\
             d.exportForScreens(new Folder(dir + '/both'), ExportForScreensType.SE_SVG, undefined, both);",
            two_artboards(&dir)
        ),
    );
    assert_eq!(names(&dir.join("both")), ["Logo-2.svg", "Logo.svg"]);
    let _ = std::fs::remove_dir_all(base);
}

/// The prefix comes before each file's name, and a path character in it does not make a folder or
/// reach out of the export folder.
#[test]
fn a_file_name_prefix_stays_in_the_folder() {
    let base = folder("prefix");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    logged(
        &mut s,
        &base,
        &format!(
            "{}d.exportForScreens(new Folder(dir + '/out'), ExportForScreensType.SE_SVG, undefined, undefined, 'web-');\n\
             d.exportForScreens(new Folder(dir + '/up'), ExportForScreensType.SE_SVG, undefined, undefined, '../x/');",
            two_artboards(&dir)
        ),
    );
    let out = names(&dir.join("out"));
    assert!(out.len() == 2 && out.iter().all(|n| n.starts_with("web-")), "{out:?}");
    let up = names(&dir.join("up"));
    assert!(up.len() == 2 && up.iter().all(|n| n.starts_with("..-x-")), "{up:?}");
    assert!(std::fs::read_dir(dir.join("up")).unwrap().all(|e| e.unwrap().file_type().unwrap().is_file()), "no folder was made");
    assert!(!dir.join("x").exists(), "nothing is written beside the folder");
    let _ = std::fs::remove_dir_all(base);
}

/// The folder goes through the script's file grants: one outside them, and a file given as the
/// folder, throw before anything is written.
#[test]
fn an_unwritable_folder_is_refused() {
    let base = folder("refused");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    std::fs::write(dir.join("taken"), "kept").unwrap();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 20})).unwrap();
    let outside = quoted(&base.join("outside/screens"));
    let e = failure(
        &mut s,
        &base,
        &format!("Folder.userData; app.activeDocument.exportForScreens(new Folder({outside}), ExportForScreensType.SE_PNG24);"),
    );
    assert!(e.contains(DENIED), "{e}");
    assert!(!base.join("outside/screens").exists());
    let taken = quoted(&dir.join("taken"));
    let e =
        failure(&mut s, &base, &format!("Folder.userData; app.activeDocument.exportForScreens(new Folder({taken}), ExportForScreensType.SE_PNG24);"));
    assert!(e.contains(crate::script::files::NOT_FOLDER), "{e}");
    assert_eq!(std::fs::read_to_string(dir.join("taken")).unwrap(), "kept");
    assert_eq!(names(&base.join("outside")), Vec::<String>::new());
    let _ = std::fs::remove_dir_all(base);
}

/// Only a `Folder` the `Folder` class made (or a `File`) names the folder: an object made from
/// `Folder.prototype` throws a `TypeError`, and a `Folder` whose hidden path property a script set
/// keeps its own path.
#[test]
fn a_forged_folder_is_refused() {
    let base = folder("forged");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 20})).unwrap();
    let (inside, outside) = (quoted(&dir.join("in")), quoted(&base.join("outside/x")));
    let call = |f: &str| format!("Folder.userData; app.activeDocument.exportForScreens({f}, ExportForScreensType.SE_SVG);");
    let made = format!("(function () {{ var f = Object.create(Folder.prototype); f.__vcPath = {inside}; return f; }})()");
    let e = failure(&mut s, &base, &call(&made));
    assert!(e.contains("TypeError") && e.contains("needs a Folder"), "{e}");
    let moved = format!("(function () {{ var f = new Folder({outside}); f.__vcPath = {inside}; return f; }})()");
    let e = failure(&mut s, &base, &call(&moved));
    assert!(e.contains(DENIED), "{e}");
    assert!(!dir.join("in").exists() && !base.join("outside/x").exists(), "nothing is written");
    logged(&mut s, &base, &call(&format!("new File({inside})")));
    assert_eq!(names(&dir.join("in")).len(), 1, "a File names the folder at its path");
    let _ = std::fs::remove_dir_all(base);
}

/// Bad arguments throw before anything is written.
#[test]
fn bad_arguments_are_refused_with_an_error() {
    let base = folder("bad");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let f = format!("new Folder({})", quoted(&dir.join("x")));
    let e = "app.activeDocument.exportForScreens";
    let long = "a".repeat(300);
    let cases: Vec<(String, &str)> = vec![
        (format!("{e}('x', ExportForScreensType.SE_PNG24);"), "needs a Folder"),
        (format!("{e}({f}, 'SE_PNG24');"), "ExportForScreensType"),
        (format!("{e}({f}, ExportType.PNG24);"), "ExportForScreensType"),
        (format!("{e}({f}, ExportForScreensType.SE_PNG24, new ExportForScreensOptionsJPEG());"), "ExportForScreensOptionsPNG24"),
        (
            format!("{e}({f}, ExportForScreensType.SE_SVG, {{typename: 'ExportForScreensOptionsWebOptimizedSVG', svgMinified: true}});"),
            "has no option svgMinified",
        ),
        (format!("var o = new ExportForScreensOptionsJPEG(); o.progressiveScan = 6; {e}({f}, ExportForScreensType.SE_JPEG50, o);"), "from 3 to 5"),
        (format!("var o = new ExportForScreensOptionsPNG8(); o.colorCount = 1; {e}({f}, ExportForScreensType.SE_PNG8, o);"), "from 2 to 256"),
        (format!("var o = new ExportForScreensOptionsPNG8(); o.matteColor = 1; {e}({f}, ExportForScreensType.SE_PNG8, o);"), "not available yet"),
        (format!("var o = new ExportForScreensOptionsPNG24(); o.scaleTypeValue = 0; {e}({f}, ExportForScreensType.SE_PNG24, o);"), "above 0"),
        (
            format!("var o = new ExportForScreensOptionsPNG24(); o.scaleTypeValue = 100; {e}({f}, ExportForScreensType.SE_PNG24, o);"),
            "from 0.01 to 64",
        ),
        (
            format!(
                "var o = new ExportForScreensOptionsPNG24(); o.scaleType = ExportForScreensScaleType.SCALEBYRESOLUTION; o.scaleTypeValue = 5000; {e}({f}, ExportForScreensType.SE_PNG24, o);"
            ),
            "from 0.72 to 4608",
        ),
        (
            format!("var o = new ExportForScreensOptionsPNG24(); o.scaleType = 'SCALEBYWIDTH'; {e}({f}, ExportForScreensType.SE_PNG24, o);"),
            "ExportForScreensScaleType",
        ),
        (
            format!("var o = new ExportForScreensOptionsPNG24(); o.antiAliasing = true; {e}({f}, ExportForScreensType.SE_PNG24, o);"),
            "AntiAliasingMethod",
        ),
        (
            format!("var o = new ExportForScreensOptionsWebOptimizedSVG(); o.svgId = SVGFontType.SVGFONT; {e}({f}, ExportForScreensType.SE_SVG, o);"),
            "SVGIdType",
        ),
        (format!("var o = new ExportForScreensPDFOptions(); o.pdfPreset = '{long}'; {e}({f}, ExportForScreensType.SE_PDF, o);"), "255 bytes"),
        (format!("{e}({f}, ExportForScreensType.SE_PNG24, undefined, {{artboards: '1'}});"), "ExportForScreensItemToExport"),
        (format!("var i = new ExportForScreensItemToExport(); i.artboards = '2-1'; {e}({f}, ExportForScreensType.SE_PNG24, undefined, i);"), "range"),
        (
            format!("var i = new ExportForScreensItemToExport(); i.artboards = 0; {e}({f}, ExportForScreensType.SE_PNG24, undefined, i);"),
            "whole number from 1",
        ),
        (
            format!("var i = new ExportForScreensItemToExport(); i.assets = 3; {e}({f}, ExportForScreensType.SE_PNG24, undefined, i);"),
            "array of asset ids",
        ),
        (
            format!("var i = new ExportForScreensItemToExport(); i.assets = [1.5]; {e}({f}, ExportForScreensType.SE_PNG24, undefined, i);"),
            "not an asset id",
        ),
        (
            format!("var i = new ExportForScreensItemToExport(); i.document = 'yes'; {e}({f}, ExportForScreensType.SE_PNG24, undefined, i);"),
            "true or false",
        ),
        (
            format!("var i = new ExportForScreensItemToExport(); i.artboard = '1'; {e}({f}, ExportForScreensType.SE_PNG24, undefined, i);"),
            "has no option artboard",
        ),
        (format!("{e}({f}, ExportForScreensType.SE_PNG24, undefined, undefined, 5);"), "fileNamePrefix must be a string"),
        (format!("var i = new ExportForScreensItemToExport(); i.artboards = '3'; {e}({f}, ExportForScreensType.SE_PNG24, undefined, i);"), "range"),
    ];
    for (case, wanted) in &cases {
        let e = failure(&mut s, &base, &format!("Folder.userData; {case}"));
        assert!(e.contains(wanted), "{case}: {e}");
    }
    assert!(!dir.join("x").exists(), "nothing was written");
    let _ = std::fs::remove_dir_all(base);
}
