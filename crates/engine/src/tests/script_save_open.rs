//! Saving, exporting and opening documents from a script: `Document.saveAs`, `Document.exportFile`,
//! `app.open`, their option classes and `app.userInteractionLevel`, and the file grants each path
//! goes through.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::*;

const DENIED: &str = "Script file access denied";

/// A fresh folder for test `name`, by its real path, with `main` for the script.
fn folder(name: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("vc-script-save-{name}-{}", std::process::id()));
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

#[test]
fn save_as_export_file_and_open_write_and_read_where_the_script_may() {
    let base = folder("happy");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    let d = quoted(&dir);
    let out = logged(
        &mut s,
        &base,
        &format!(
            "Folder.userData; var dir = {d}, d = app.documents.add(DocumentColorSpace.RGB, 200, 100);\n\
             vc.run('shape.rectangle', {{x: 10, y: 10, width: 50, height: 20}});\n\
             d.saveAs(new File(dir + '/a.svg'));\n\
             var o = new PDFSaveOptions(); vc.log([o.typename, o.compatibility, o.preserveEditability]);\n\
             o.compatibility = PDFCompatibility.ACROBAT7; o.viewAfterSaving = false; o.pDFXStandardDescription = undefined;\n\
             o.colorCompression = 'CompressionQuality.None';\n\
             d.saveAs(new File(dir + '/b.pdf'), o); vc.log(d.name);\n\
             var p = new ExportOptionsPNG24(); p.horizontalScale = p.verticalScale = 200; p.artBoardClipping = true;\n\
             d.exportFile(new File(dir + '/c'), ExportType.PNG24, p);\n\
             d.exportFile(new File(dir + '/e.JPG'), ExportType.JPEG);\n\
             var v = new ExportOptionsSVG(); v.compressed = true; v.coordinatePrecision = 2;\n\
             d.exportFile(new File(dir + '/f'), ExportType.SVG, v); vc.log(d.name);\n\
             var g = app.open(new File(dir + '/a.svg'));\n\
             vc.log([g.typename, app.documents.length, app.activeDocument.name === g.name, g.name]);\n\
             var u = [app.userInteractionLevel]; app.userInteractionLevel = UserInteractionLevel.DONTDISPLAYALERTS; u.push(app.userInteractionLevel); vc.log(u);"
        ),
    );
    assert_eq!(out[0], json!(["PDFSaveOptions", "PDFCompatibility.ACROBAT5", true]));
    assert_eq!(out[1], json!("b.pdf"), "saveAs gives the document the new file");
    assert_eq!(out[2], json!("b.pdf"), "exportFile leaves the document's file as it is");
    assert_eq!(out[3], json!(["Document", 2, true, "a.svg"]));
    assert_eq!(out[4], json!(["UserInteractionLevel.DISPLAYALERTS", "UserInteractionLevel.DONTDISPLAYALERTS"]));
    assert!(std::fs::read_to_string(dir.join("a.svg")).unwrap().contains("<svg"));
    let pdf = std::fs::read(dir.join("b.pdf")).unwrap();
    assert!(pdf.starts_with(b"%PDF-1.6"), "{:?}", String::from_utf8_lossy(&pdf[..8]));
    let png = image::open(dir.join("c.png")).unwrap();
    assert_eq!((png.width(), png.height()), (400, 200), "the artboard at 200%");
    assert!(dir.join("e.JPG").is_file() && !dir.join("e.JPG.jpg").exists(), "a name that ends with the extension keeps it");
    assert!(std::fs::read(dir.join("f.svgz")).unwrap().starts_with(&[0x1f, 0x8b]));
    assert_eq!(s.documents().len(), 2);
    let _ = std::fs::remove_dir_all(base);
}

/// Without options, a save or export writes what an untouched new options object of its class gives.
#[test]
fn omitted_options_are_a_new_options_objects_defaults() {
    let base = folder("defaults");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    let d = quoted(&dir);
    logged(
        &mut s,
        &base,
        &format!(
            "Folder.userData; var dir = {d}, d = app.documents.add(DocumentColorSpace.RGB, 200, 100);\n\
             vc.run('shape.rectangle', {{x: 10, y: 10, width: 50, height: 20}});\n\
             d.exportFile(new File(dir + '/j1'), ExportType.JPEG);\n\
             d.exportFile(new File(dir + '/j2'), ExportType.JPEG, new ExportOptionsJPEG());\n\
             d.exportFile(new File(dir + '/p1'), ExportType.PNG24);\n\
             d.exportFile(new File(dir + '/p2'), ExportType.PNG24, new ExportOptionsPNG24());\n\
             var j3 = new ExportOptionsJPEG(); j3.matteColor = {{red: 255, green: 255, blue: 255}};\n\
             d.exportFile(new File(dir + '/j3'), ExportType.JPEG, j3);\n\
             var p3 = new ExportOptionsPNG24(); p3.matteColor = {{typename: 'RGBColor', red: 255, green: 255, blue: 255}};\n\
             d.exportFile(new File(dir + '/p3'), ExportType.PNG24, p3);\n\
             d.exportFile(new File(dir + '/s1'), ExportType.SVG);\n\
             d.exportFile(new File(dir + '/s2'), ExportType.SVG, new ExportOptionsSVG());\n\
             d.saveAs(new File(dir + '/a1.pdf'));\n\
             d.saveAs(new File(dir + '/a2.pdf'), new PDFSaveOptions());"
        ),
    );
    for (one, two) in [("j1.jpg", "j2.jpg"), ("j1.jpg", "j3.jpg"), ("p1.png", "p2.png"), ("p1.png", "p3.png"), ("s1.svg", "s2.svg")] {
        assert!(std::fs::read(dir.join(one)).unwrap() == std::fs::read(dir.join(two)).unwrap(), "{one} and {two} differ");
    }
    for name in ["a1.pdf", "a2.pdf"] {
        let pdf = std::fs::read(dir.join(name)).unwrap();
        assert!(pdf.starts_with(b"%PDF-1.4"), "{name}: PDFCompatibility.ACROBAT5 is PDF 1.4, {:?}", String::from_utf8_lossy(&pdf[..8]));
    }
    let _ = std::fs::remove_dir_all(base);
}

/// A raster export at 100% has one pixel per point, whatever the document's raster effects
/// resolution.
#[test]
fn a_raster_export_at_100_percent_has_a_pixel_per_point() {
    let base = folder("rasterscale");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    s.execute("document.rasterEffectsSettings", &json!({"resolution": 300})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 20})).unwrap();
    let d = quoted(&dir);
    logged(
        &mut s,
        &base,
        &format!(
            "Folder.userData; var dir = {d}, d = app.activeDocument, o = new ExportOptionsPNG24(); o.artBoardClipping = true;\n\
             d.exportFile(new File(dir + '/a'), ExportType.PNG24, o);\n\
             var j = new ExportOptionsJPEG(); j.artBoardClipping = true; d.exportFile(new File(dir + '/b'), ExportType.JPEG, j);"
        ),
    );
    for name in ["a.png", "b.jpg"] {
        let image = image::open(dir.join(name)).unwrap();
        assert_eq!((image.width(), image.height()), (200, 100), "{name}");
    }
    let _ = std::fs::remove_dir_all(base);
}

/// The value of the DXF header variable `name` (the group after its name).
fn dxf_header(dxf: &str, name: &str) -> Option<String> {
    let lines: Vec<&str> = dxf.lines().map(str::trim).collect();
    lines.iter().position(|l| *l == name).and_then(|i| lines.get(i + 2)).map(|v| v.to_string())
}

/// The largest x coordinate (group code 10) in the entities of a DXF.
fn dxf_widest(dxf: &str) -> f64 {
    dxf_xs(dxf).into_iter().fold(f64::MIN, f64::max)
}

/// The smallest x coordinate (group code 10) in the entities of a DXF.
fn dxf_leftmost(dxf: &str) -> f64 {
    dxf_xs(dxf).into_iter().fold(f64::MAX, f64::min)
}

/// The x coordinates (group code 10) of the polyline vertices in a DXF. A hatch's code 10 is its
/// elevation point, so hatches are left out.
fn dxf_xs(dxf: &str) -> Vec<f64> {
    let lines: Vec<&str> = dxf.lines().map(str::trim).collect();
    // The section's name is a value; the group codes start on the line after it.
    let start = lines.iter().position(|l| *l == "ENTITIES").map_or(0, |i| i + 1);
    let mut entity = "";
    let mut xs = Vec::new();
    for p in lines[start..].chunks(2).filter(|p| p.len() == 2) {
        match p[0] {
            "0" => entity = p[1],
            "10" if entity == "LWPOLYLINE" => xs.extend(p[1].parse::<f64>().ok()),
            _ => {}
        }
    }
    xs
}

/// `exportFile` with `ExportType.AutoCAD` writes a DXF with the `ExportOptionsAutoCAD` options, and
/// DWG, the help page's default format, throws without writing a file.
#[test]
fn an_autocad_export_writes_a_dxf_with_its_options() {
    let base = folder("autocad");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    let d = quoted(&dir);
    let out = logged(
        &mut s,
        &base,
        &format!(
            "Folder.userData; var dir = {d}, d = app.documents.add(DocumentColorSpace.RGB, 200, 100);\n\
             vc.run('shape.rectangle', {{x: 10, y: 10, width: 50, height: 20}});\n\
             var o = new ExportOptionsAutoCAD(); vc.log([o.typename, o.exportFileFormat, o.version, ExportType.AUTOCAD === ExportType.AutoCAD]);\n\
             try {{ d.exportFile(new File(dir + '/w'), ExportType.AutoCAD, o); }} catch (e) {{ vc.log(String(e)); }}\n\
             o.exportFileFormat = AutoCADExportFileFormat.DXF; o.version = AutoCADCompatibility.AutoCADRelease15;\n\
             o.unit = AutoCADUnit.Inches; o.unitScaleRatio = 25.4; o.colors = AutoCADColors.Max256Colors; o.rasterFormat = AutoCADRasterFormat.JPEG;\n\
             d.exportFile(new File(dir + '/a'), ExportType.AutoCAD, o);\n\
             o.unitScaleRatio = 1; d.exportFile(new File(dir + '/b.DXF'), ExportType.AUTOCAD, o);\n\
             o.unit = AutoCADUnit.Picas; d.exportFile(new File(dir + '/p'), ExportType.AutoCAD, o);\n\
             var u = new ExportOptionsAutoCAD(); u.exportFileFormat = AutoCADExportFileFormat.DXF; u.version = undefined; d.exportFile(new File(dir + '/u'), ExportType.AutoCAD, u);\n\
             d.exportFile(new File(dir + '/c'), ExportType.AutoCAD, {{typename: 'ExportOptionsAutoCAD', exportFileFormat: AutoCADExportFileFormat.DXF, version: undefined}});"
        ),
    );
    assert_eq!(out[0], json!(["ExportOptionsAutoCAD", "AutoCADExportFileFormat.DWG", "AutoCADCompatibility.AutoCADRelease24", true]));
    assert!(out[1].as_str().is_some_and(|e| e.contains("DWG is not available")), "{}", out[1]);
    assert!(!dir.join("w.dxf").exists() && !dir.join("w.dwg").exists() && !dir.join("w").exists(), "DWG writes nothing");
    let a = std::fs::read_to_string(dir.join("a.dxf")).unwrap();
    let b = std::fs::read_to_string(dir.join("b.DXF")).unwrap();
    assert!(!dir.join("b.DXF.dxf").exists(), "a name that ends with the extension keeps it");
    assert_eq!(dxf_header(&a, "$ACADVER").as_deref(), Some("AC1015"), "AutoCADRelease15 is the 2000 format");
    assert_eq!(dxf_header(&b, "$INSUNITS").as_deref(), Some("1"), "inches, one drawing unit each");
    assert!(dxf_widest(&a) > dxf_widest(&b) * 20.0, "unitScaleRatio scales the drawing: {} and {}", dxf_widest(&a), dxf_widest(&b));
    for name in ["c.dxf", "u.dxf"] {
        let dxf = std::fs::read_to_string(dir.join(name)).unwrap();
        assert_eq!(dxf_header(&dxf, "$ACADVER").as_deref(), Some("AC1024"), "{name}: an option left out or undefined is AutoCADRelease24");
        // The rectangle is 10 pt (3.5 mm) from the artboard's left edge.
        assert!(dxf_leftmost(&dxf).abs() < 1.0, "{name}: the origin is at the visible art's bounds: {}", dxf_leftmost(&dxf));
    }
    let p = std::fs::read_to_string(dir.join("p.dxf")).unwrap();
    assert!(dxf_widest(&p) > dxf_widest(&b) * 5.0, "a pica is a sixth of an inch: {} and {}", dxf_widest(&p), dxf_widest(&b));
    let _ = std::fs::remove_dir_all(base);
}

/// With `pDFPreset` named, the options left at a new object's defaults keep the preset's values, and
/// an option the script changes applies over the preset.
#[test]
fn a_pdf_preset_keeps_its_settings_unless_an_option_changes() {
    let base = folder("preset");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    let d = quoted(&dir);
    logged(
        &mut s,
        &base,
        &format!(
            "Folder.userData; var dir = {d}, d = app.documents.add(DocumentColorSpace.RGB, 200, 100);\n\
             vc.run('shape.rectangle', {{x: 10, y: 10, width: 50, height: 20}});\n\
             var o = new PDFSaveOptions(); o.pDFPreset = 'High Quality Print'; d.saveAs(new File(dir + '/a.pdf'), o);\n\
             o.compatibility = PDFCompatibility.ACROBAT8; d.saveAs(new File(dir + '/b.pdf'), o);"
        ),
    );
    for (name, version) in [("a.pdf", b"%PDF-1.6"), ("b.pdf", b"%PDF-1.7")] {
        let pdf = std::fs::read(dir.join(name)).unwrap();
        assert!(pdf.starts_with(version), "{name}: {:?}", String::from_utf8_lossy(&pdf[..8]));
    }
    let _ = std::fs::remove_dir_all(base);
}

/// A save that writes several files, such as an EPS with a file per artboard, writes none of them
/// when one may not be written.
#[test]
fn a_save_writes_nothing_when_one_of_its_files_is_refused() {
    let base = folder("saveboards");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    s.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 2})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 20})).unwrap();
    let save = format!("Folder.userData; vc.run('file.saveAs', {{path: {}, format: 'eps', useArtboards: true}});", quoted(&dir.join("a.eps")));
    logged(&mut s, &base, &save);
    let central = dir.join("a.eps");
    let mut boards: Vec<PathBuf> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| *p != central).collect();
    boards.sort();
    assert!(central.is_file() && boards.len() == 2, "the central file and a file per artboard: {boards:?}");
    let board = boards.pop().unwrap();
    std::fs::remove_file(&central).unwrap();
    let mut perms = std::fs::metadata(&board).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&board, perms.clone()).unwrap();
    let e = failure(&mut s, &base, &save);
    assert!(e.contains(DENIED), "{e}");
    assert!(!central.exists(), "the central file is not written when an artboard's file may not be");
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(&board, perms).unwrap();
    let _ = std::fs::remove_dir_all(base);
}

/// When the writer a save is given does not check the file access itself, a save of several files
/// whose last file is refused hands none of them to the writer, the refused one included.
#[test]
fn a_refused_file_stops_a_save_whose_writer_does_not_check() {
    use crate::cmd::fileio::{SaveMode, save_job, save_plan};
    use crate::file_access::{ScriptGate, ScriptScope, with_script_gate};
    let base = folder("unchecked");
    let dir = base.join("outside");
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 2})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 20})).unwrap();
    let params = json!({"path": dir.join("a.eps").to_string_lossy(), "format": "eps", "useArtboards": true});
    let plan = save_plan(&s, SaveMode::SaveAs, &params).unwrap();
    let job = save_job(&mut s, plan).unwrap();
    let gate = ScriptGate {
        read: ScriptScope::default(),
        write: ScriptScope::of_folder(&dir),
        documents: vec![],
        held: vec![],
        linked: Default::default(),
        also: None,
    };
    let (mut refused, mut written) = (None, vec![]);
    let make_read_only = |paths: &[&str]| {
        let last = paths.last().map(|p| p.to_string()).unwrap();
        std::fs::write(&last, b"").unwrap();
        let mut perms = std::fs::metadata(&last).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&last, perms).unwrap();
        refused = Some(last);
        None
    };
    let r = with_script_gate(gate, || {
        job.write_unless(make_read_only, |p, _| {
            written.push(p.to_string());
            Ok(())
        })
    });
    let e = r.unwrap_err().to_string();
    let refused = refused.unwrap();
    assert!(e.contains(DENIED) && e.contains(&refused), "{e}");
    assert!(written.is_empty(), "no file went to the writer: {written:?}");
    let mut perms = std::fs::metadata(&refused).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(&refused, perms).unwrap();
    let _ = std::fs::remove_dir_all(base);
}

/// Every path goes through the script's file grants, as a `File` opened for reading or writing.
#[test]
fn save_as_export_file_and_open_refuse_paths_outside_the_grants() {
    let base = folder("refused");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    std::fs::write(base.join("outside/secret.svg"), "<svg xmlns='http://www.w3.org/2000/svg'/>").unwrap();
    std::fs::write(base.join("settings/Script Grants.json"), "{}").unwrap();
    std::fs::write(dir.join("kept.svg"), "kept").unwrap();
    let mut perms = std::fs::metadata(dir.join("kept.svg")).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(dir.join("kept.svg"), perms).unwrap();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let undo = s.active().unwrap().history.undo.len();
    let outside = quoted(&base.join("outside/x.svg"));
    let grants = quoted(&base.join("settings/Script Grants.json"));
    let kept = quoted(&dir.join("kept.svg"));
    let secret = quoted(&base.join("outside/secret.svg"));
    let cases = [
        format!("app.activeDocument.saveAs(new File({outside}));"),
        format!("app.activeDocument.exportFile(new File({outside}), ExportType.PNG24);"),
        format!("app.activeDocument.saveAs(new File({grants}));"),
        format!("app.open(new File({secret}));"),
        format!("app.userInteractionLevel = UserInteractionLevel.DONTDISPLAYALERTS; app.activeDocument.saveAs(new File({outside}));"),
    ];
    for case in &cases {
        let e = failure(&mut s, &base, &format!("Folder.userData; {case}"));
        assert!(e.contains(DENIED), "{case}: {e}");
    }
    let e = failure(&mut s, &base, &format!("Folder.userData; app.activeDocument.saveAs(new File({kept}));"));
    assert!(e.contains("Script file is read-only"), "{e}");
    let e = failure(&mut s, &base, &format!("app.activeDocument.saveAs(new File({}));", quoted(&dir)));
    assert!(e.contains("Script file") && !e.contains("ran"), "a folder is not saved over: {e}");
    assert_eq!(std::fs::read_to_string(dir.join("kept.svg")).unwrap(), "kept");
    assert_eq!(std::fs::read_to_string(base.join("settings/Script Grants.json")).unwrap(), "{}");
    assert!(!base.join("outside/x.svg").exists() && !base.join("outside/x.svg.png").exists());
    assert_eq!(s.documents().len(), 1);
    assert_eq!(s.active().unwrap().path, None, "a refused save leaves the document's file as it is");
    assert_eq!(s.active().unwrap().history.undo.len(), undo, "and adds no undo step");
    let mut perms = std::fs::metadata(dir.join("kept.svg")).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(dir.join("kept.svg"), perms).unwrap();
    let _ = std::fs::remove_dir_all(base);
}

/// The file of an open document is not written or read through a script method unless the
/// script's grants take it, though `vc.run`'s commands may use it.
#[test]
fn an_open_documents_file_is_not_saved_over_without_a_grant() {
    let base = folder("opendoc");
    let mut s = session(&base);
    let path = base.join("outside/open.svg");
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s.execute("file.saveAs", &json!({"path": path.to_string_lossy()})).unwrap();
    let before = std::fs::read(&path).unwrap();
    let e = failure(&mut s, &base, &format!("app.activeDocument.saveAs(new File({}));", quoted(&path)));
    assert!(e.contains(DENIED), "{e}");
    let e = failure(&mut s, &base, &format!("app.activeDocument.exportFile(new File({}), ExportType.SVG);", quoted(&path)));
    assert!(e.contains(DENIED), "{e}");
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let _ = std::fs::remove_dir_all(base);
}

/// A save or export to a file a `File` object holds open for writing throws, so the object's bytes
/// written at the end of the run do not replace the saved document.
#[test]
fn a_file_open_for_writing_is_not_saved_over() {
    let base = folder("openhandle");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    for (name, call) in
        [("a.svg", "app.activeDocument.saveAs(new File(p));"), ("b.png", "app.activeDocument.exportFile(new File(p), ExportType.PNG24);")]
    {
        let p = quoted(&dir.join(name));
        let log = logged(
            &mut s,
            &base,
            &format!(
                "Folder.userData; var p = {p}, w = new File(p); w.open('w'); w.write('pending');\ntry {{ {call} vc.log('ran'); }} catch (e) {{ vc.log(e.message); }}"
            ),
        );
        assert!(log[0].as_str().unwrap().contains(crate::script::files::OPEN_FOR_WRITING), "{name}: {log}");
        assert_eq!(std::fs::read_to_string(dir.join(name)).unwrap(), "pending", "{name}: the File's own bytes");
        assert_eq!(s.active().unwrap().path, None, "{name}: the document's file is unchanged");
    }
    let _ = std::fs::remove_dir_all(base);
}

/// A file `open` reads besides the one it names, here an SVG's linked image, is read only where
/// the script's grants allow, though it is an open document's file.
#[test]
fn open_reads_linked_files_only_where_the_script_may() {
    let base = folder("linked");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    let linked = base.join("outside/linked.svg");
    std::fs::write(&linked, "<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'><rect width='4' height='4'/></svg>").unwrap();
    s.execute("document.open", &json!({"path": linked.to_string_lossy()})).unwrap();
    assert!(s.active().unwrap().path.is_some(), "the linked file is an open document's file");
    let href = linked.to_string_lossy().replace('\\', "/").replace('&', "&amp;").replace('<', "&lt;").replace('\'', "&apos;");
    std::fs::write(
        dir.join("page.svg"),
        format!("<svg xmlns='http://www.w3.org/2000/svg' xmlns:xlink='http://www.w3.org/1999/xlink' width='50' height='50'><image x='0' y='0' width='10' height='10' xlink:href='{href}'/></svg>"),
    )
    .unwrap();
    logged(&mut s, &base, &format!("Folder.userData; app.open(new File({}));", quoted(&dir.join("page.svg"))));
    assert_eq!(s.documents().len(), 2);
    let ims = |s: &Session| {
        let mut out = vec![];
        s.active().unwrap().doc.walk(|n| {
            if let NodeKind::Image(im) = &n.kind {
                out.push((im.width, im.height, im.link.as_ref().map(|l| (l.modified.is_some(), l.hash.is_some()))));
            }
        });
        out
    };
    let kinds = |s: &Session| {
        let mut out = vec![];
        s.active().unwrap().doc.walk(|n| out.push(format!("{:?}", n.kind).chars().take(12).collect::<String>()));
        out
    };
    assert_eq!(ims(&s), vec![(64, 64, Some((false, false)))], "the linked file is a placeholder: {:?}", kinds(&s));
    // Opened by the app, a copy of the file reads its image.
    std::fs::copy(dir.join("page.svg"), dir.join("copy.svg")).unwrap();
    s.execute("document.open", &json!({"path": dir.join("copy.svg").to_string_lossy()})).unwrap();
    assert!(ims(&s).is_empty() && kinds(&s).iter().any(|k| k.starts_with("Path")), "{:?}", kinds(&s));
    let _ = std::fs::remove_dir_all(base);
}

/// An SVG export with `embedRasterImages` false writes the document's embedded image beside the
/// SVG, and when that file may not be written, the SVG is not written either.
#[test]
fn an_svg_export_writes_nothing_when_one_of_its_files_is_refused() {
    let base = folder("svgimages");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    let pic = base.join("outside/pic.png");
    std::fs::write(&pic, super::links::png(2, 2, [10, 200, 30])).unwrap();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s.execute("file.place", &json!({"path": pic.to_string_lossy(), "link": false})).unwrap();
    let export =
        format!("Folder.userData; app.activeDocument.exportFile(new File({}), ExportType.SVG, new ExportOptionsSVG());", quoted(&dir.join("a.svg")));
    logged(&mut s, &base, &export);
    let svg = dir.join("a.svg");
    let mut images: Vec<PathBuf> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| *p != svg).collect();
    assert_eq!(images.len(), 1, "the embedded image is written beside the SVG: {images:?}");
    let image = images.remove(0);
    std::fs::remove_file(&svg).unwrap();
    let mut perms = std::fs::metadata(&image).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&image, perms.clone()).unwrap();
    let e = failure(&mut s, &base, &export);
    assert!(e.contains(DENIED), "{e}");
    assert!(!svg.exists(), "the SVG is not written when its image may not be");
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(&image, perms).unwrap();
    let _ = std::fs::remove_dir_all(base);
}

/// An export does not write a file it writes besides the one named, here an SVG's image, while a
/// `File` object holds it open for writing, and so writes nothing.
#[test]
fn an_svg_export_does_not_write_an_image_a_file_holds_open() {
    let base = folder("svgheld");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    let pic = base.join("outside/pic.png");
    std::fs::write(&pic, super::links::png(2, 2, [10, 200, 30])).unwrap();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s.execute("file.place", &json!({"path": pic.to_string_lossy(), "link": false})).unwrap();
    let svg = dir.join("a.svg");
    let export = format!("app.activeDocument.exportFile(new File({}), ExportType.SVG, new ExportOptionsSVG());", quoted(&svg));
    logged(&mut s, &base, &format!("Folder.userData; {export}"));
    let mut images: Vec<PathBuf> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| *p != svg).collect();
    assert_eq!(images.len(), 1, "the embedded image is written beside the SVG: {images:?}");
    let image = images.remove(0);
    std::fs::remove_file(&svg).unwrap();
    let log = logged(
        &mut s,
        &base,
        &format!(
            "Folder.userData; var w = new File({}); w.open('w'); w.write('pending');\ntry {{ {export} vc.log('ran'); }} catch (e) {{ vc.log(e.message); }}",
            quoted(&image)
        ),
    );
    assert!(log[0].as_str().unwrap().contains(DENIED), "{log}");
    assert!(!svg.exists(), "the SVG is not written when its image may not be");
    assert_eq!(std::fs::read_to_string(&image).unwrap(), "pending", "the File's own bytes");
    let _ = std::fs::remove_dir_all(base);
}

/// A link inside a granted folder that leads out of it is followed and refused.
#[cfg(unix)]
#[test]
fn a_link_out_of_the_granted_folder_is_refused() {
    let base = folder("link");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    std::os::unix::fs::symlink(base.join("outside"), dir.join("away")).unwrap();
    std::os::unix::fs::symlink(base.join("outside/target.svg"), dir.join("file.svg")).unwrap();
    std::fs::write(base.join("outside/read.svg"), "<svg xmlns='http://www.w3.org/2000/svg'/>").unwrap();
    std::os::unix::fs::symlink(base.join("outside/read.svg"), dir.join("read.svg")).unwrap();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    for name in ["away/x.svg", "file.svg"] {
        let e = failure(&mut s, &base, &format!("app.activeDocument.saveAs(new File({}));", quoted(&dir.join(name))));
        assert!(e.contains(DENIED), "{name}: {e}");
    }
    let e = failure(&mut s, &base, &format!("app.open(new File({}));", quoted(&dir.join("read.svg"))));
    assert!(e.contains(DENIED), "{e}");
    assert!(std::fs::read_dir(base.join("outside")).unwrap().count() == 1, "nothing is written outside");
    assert_eq!(s.documents().len(), 1);
    let _ = std::fs::remove_dir_all(base);
}

/// Bad arguments throw before anything is written or opened.
#[test]
fn bad_arguments_are_refused_with_an_error() {
    let base = folder("bad");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let f = format!("new File({})", quoted(&dir.join("x.svg")));
    let huge = "a".repeat(5000);
    let cases: Vec<(String, &str)> = vec![
        ("app.activeDocument.saveAs('x.svg');".into(), "saveAs needs a File"),
        (format!("app.activeDocument.saveAs({{'\\u0000fileRef': {}}});", quoted(&dir.join("x.svg"))), "is not plain data"),
        (format!("app.activeDocument.saveAs({{'\\u0000folderRef': {}}});", quoted(&dir)), "is not plain data"),
        (format!("app.activeDocument.saveAs({f}, {{typename: 'Nope'}});"), "saveAs options must be"),
        (format!("app.activeDocument.saveAs({f}, new ExportOptionsSVG());"), "saveAs options must be"),
        (format!("var o = new IllustratorSaveOptions(); o.compressed = 'yes'; app.activeDocument.saveAs({f}, o);"), "must be true or false"),
        (
            format!("var o = new IllustratorSaveOptions(); o.compatibility = Compatibility.ILLUSTRATOR8; app.activeDocument.saveAs({f}, o);"),
            "is not available yet",
        ),
        (
            format!("var o = new PDFSaveOptions(); o.trimMarks = true; app.activeDocument.saveAs({f}, o);"),
            "PDFSaveOptions.trimMarks is not available yet",
        ),
        (
            format!("var o = new PDFSaveOptions(); o.colorCompression = 'Unrelated.None'; app.activeDocument.saveAs({f}, o);"),
            "PDFSaveOptions.colorCompression is not available yet",
        ),
        (format!("var o = new PDFSaveOptions(); o.pDFPreset = '{huge}'; app.activeDocument.saveAs({f}, o);"), "too long"),
        (format!("var o = new EPSSaveOptions(); o.postScript = 'LEVEL2'; app.activeDocument.saveAs({f}, o);"), "EPSPostScriptLevelEnum"),
        (format!("app.activeDocument.exportFile({f}, 'PNG24');"), "ExportType"),
        (format!("app.activeDocument.exportFile({f}, ExportType.FLASH);"), "is not available yet"),
        (
            format!(
                "var p = new ExportOptionsPNG24(); p.matteColor = {{red: 255, green: 0, blue: 255}}; app.activeDocument.exportFile({f}, ExportType.PNG24, p);"
            ),
            "ExportOptionsPNG24.matteColor is not available yet",
        ),
        (format!("app.activeDocument.exportFile({f}, ExportType.PNG24, new ExportOptionsJPEG());"), "ExportOptionsPNG24"),
        (
            format!(
                "var p = new ExportOptionsPNG24(); p.horizontalScale = p.verticalScale = 1e9; app.activeDocument.exportFile({f}, ExportType.PNG24, p);"
            ),
            "at most",
        ),
        (
            format!(
                "var p = new ExportOptionsPNG24(); p.horizontalScale = p.verticalScale = 0.5; app.activeDocument.exportFile({f}, ExportType.PNG24, p);"
            ),
            "at least 1",
        ),
        (
            format!("var p = new ExportOptionsPNG24(); p.horizontalScale = 50; app.activeDocument.exportFile({f}, ExportType.PNG24, p);"),
            "not available yet",
        ),
        (
            format!("var p = new ExportOptionsJPEG(); p.qualitySetting = 101; app.activeDocument.exportFile({f}, ExportType.JPEG, p);"),
            "from 0 to 100",
        ),
        (format!("app.activeDocument.exportFile({f}, ExportType.GIF, {{}});"), "not available yet"),
        (format!("app.activeDocument.exportFile({f}, ExportType.AutoCAD);"), "DWG is not available"),
        (
            format!("var o = new ExportOptionsAutoCAD(); o.exportFileFormat = undefined; app.activeDocument.exportFile({f}, ExportType.AUTOCAD, o);"),
            "DWG is not available",
        ),
        (
            format!(
                "var o = new ExportOptionsAutoCAD(); o.exportFileFormat = AutoCADExportFileFormat.DXF; o.unitScaleRatio = 2000; app.activeDocument.exportFile({f}, ExportType.AutoCAD, o);"
            ),
            "at most 1000",
        ),
        (
            format!(
                "var o = new ExportOptionsAutoCAD(); o.exportFileFormat = AutoCADExportFileFormat.DXF; o.version = 'AutoCADCompatibility.AutoCADRelease99'; app.activeDocument.exportFile({f}, ExportType.AutoCAD, o);"
            ),
            "AutoCADCompatibility",
        ),
        (
            format!(
                "var o = new ExportOptionsAutoCAD(); o.exportFileFormat = AutoCADExportFileFormat.DXF; o.colors = 256; app.activeDocument.exportFile({f}, ExportType.AutoCAD, o);"
            ),
            "AutoCADColors",
        ),
        (
            format!(
                "var o = new ExportOptionsAutoCAD(); o.exportFileFormat = AutoCADExportFileFormat.DXF; o.unitScale = 2; app.activeDocument.exportFile({f}, ExportType.AutoCAD, o);"
            ),
            "ExportOptionsAutoCAD has no option unitScale",
        ),
        (
            format!(
                "var o = new ExportOptionsAutoCAD(); o.exportFileFormat = AutoCADExportFileFormat.DXF; o.version = AutoCADCompatibility.AutoCADRelease14; o.colors = AutoCADColors.TrueColors; app.activeDocument.exportFile({f}, ExportType.AutoCAD, o);"
            ),
            "TrueColors needs AutoCADRelease18",
        ),
        (format!("app.open({f}, 'RGB');"), "DocumentColorSpace"),
        (format!("app.open({f}, undefined, {{pageToOpen: 0}});"), "from 1"),
        (format!("app.open({f}, undefined, {{pDFCropToBox: 'art'}});"), "PDFBoxType"),
        (format!("app.open({f}, undefined, 5);"), "PDFFileOptions"),
        (format!("var o = new PDFSaveOptions(); o.trimMark = true; app.activeDocument.saveAs({f}, o);"), "PDFSaveOptions has no option trimMark"),
        (
            format!("var p = new ExportOptionsPNG24(); p.transparancy = false; app.activeDocument.exportFile({f}, ExportType.PNG24, p);"),
            "ExportOptionsPNG24 has no option transparancy",
        ),
        (format!("app.open({f}, undefined, {{pageToOpne: 2}});"), "PDFFileOptions has no option pageToOpne"),
        ("app.userInteractionLevel = 'quiet';".into(), "UserInteractionLevel"),
        (format!("app.open({});", quoted(&dir.join("x.svg"))), "open needs a File"),
    ];
    for (case, wanted) in &cases {
        let e = failure(&mut s, &base, &format!("Folder.userData; {case}"));
        assert!(e.contains(wanted), "{case}: {e}");
    }
    let e = failure(&mut s, &base, &format!("Folder.userData; app.open(new File({}));", quoted(&dir.join("missing.svg"))));
    assert!(e.contains(crate::script::files::NOT_FOUND), "{e}");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "nothing was written");
    assert_eq!(s.documents().len(), 1);
    assert_eq!(s.active().unwrap().path, None);
    let _ = std::fs::remove_dir_all(base);
}

/// Only a `File` the `File` class made names a file. An object made from `File.prototype` and a
/// plain object throw a `TypeError`, and a `File` whose hidden path property a script set keeps its
/// own path, so nothing is written or opened at the path the script put there.
#[test]
fn a_forged_file_is_refused() {
    let base = folder("forged");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    std::fs::write(dir.join("real.svg"), "<svg xmlns='http://www.w3.org/2000/svg'/>").unwrap();
    std::fs::write(base.join("outside/secret.svg"), "<svg xmlns='http://www.w3.org/2000/svg'/>").unwrap();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let (x, real) = (quoted(&dir.join("x.svg")), quoted(&dir.join("real.svg")));
    let (out, secret) = (quoted(&base.join("outside/x.svg")), quoted(&base.join("outside/secret.svg")));
    let calls = |f: &str, read: &str| {
        [format!("app.activeDocument.saveAs({f});"), format!("app.activeDocument.exportFile({f}, ExportType.SVG);"), format!("app.open({read});")]
    };
    for made in ["Object.create(File.prototype)", "{}"] {
        let make = |p: &str| format!("(function () {{ var f = {made}; f.__vcPath = {p}; return f; }})()");
        for case in calls(&make(&x), &make(&real)) {
            let text = format!("Folder.userData; try {{ {case} vc.log('ran'); }} catch (e) {{ vc.log([e.name, e.message]); }}");
            let log = logged(&mut s, &base, &text);
            assert_eq!(log[0][0], json!("TypeError"), "{case}: {log}");
            assert!(log[0][1].as_str().unwrap().contains("needs a File"), "{case}: {log}");
        }
    }
    let moved = |from: &str, to: &str| format!("(function () {{ var f = new File({from}); f.__vcPath = {to}; return f; }})()");
    for case in calls(&moved(&out, &x), &moved(&secret, &real)) {
        let e = failure(&mut s, &base, &format!("Folder.userData; {case}"));
        assert!(e.contains(DENIED), "{case}: {e}");
    }
    assert!(!dir.join("x.svg").exists() && !dir.join("x.svg.svg").exists(), "nothing is written at the forged path");
    assert!(std::fs::read_dir(base.join("outside")).unwrap().count() == 1);
    assert_eq!(s.documents().len(), 1, "nothing is opened");
    assert_eq!(s.active().unwrap().path, None);
    let _ = std::fs::remove_dir_all(base);
}

/// A document from an earlier run is stale, and saving it throws.
#[test]
fn a_stale_document_is_not_saved() {
    let base = folder("stale");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    run(&mut s, &base, "#targetengine keep\nvar held = app.activeDocument;").unwrap();
    let e = failure(&mut s, &base, &format!("#targetengine keep\nFolder.userData; held.saveAs(new File({}));", quoted(&dir.join("x.svg"))));
    assert!(e.contains("Stale document"), "{e}");
    assert!(!dir.join("x.svg").exists());
    let _ = std::fs::remove_dir_all(base);
}

/// `open` applies the page and crop box of its options to a PDF.
#[test]
fn open_takes_the_page_and_crop_box_of_its_options() {
    let base = folder("pdf");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    s.execute("file.new", &json!({"width": 100, "height": 50, "artboards": 2})).unwrap();
    let pdf = dir.join("two.pdf");
    s.execute("file.saveAs", &json!({"path": pdf.to_string_lossy(), "format": "pdf"})).unwrap();
    let out = logged(
        &mut s,
        &base,
        &format!(
            "Folder.userData; var g = app.open(new File({}), DocumentColorSpace.CMYK, {{pageToOpen: 2, pDFCropToBox: PDFBoxType.PDFMEDIABOX, typename: 'PDFFileOptions', later: undefined}});\n\
             vc.log(g.documentColorSpace);",
            quoted(&pdf)
        ),
    );
    assert_eq!(out, json!(["DocumentColorSpace.CMYK"]));
    let e = failure(&mut s, &base, &format!("Folder.userData; app.open(new File({}), undefined, {{pageToOpen: 3}});", quoted(&pdf)));
    assert!(!e.is_empty(), "a page the file does not have");
    assert_eq!(s.documents().len(), 2);
    assert_eq!(s.active().unwrap().doc.artboards.len(), 1, "one page");
    // Options left undefined are left out: the file opens as it does without options.
    let undefined = format!("Folder.userData; app.open(new File({}), undefined, {{pageToOpen: undefined, pDFCropToBox: undefined}});", quoted(&pdf));
    logged(&mut s, &base, &undefined);
    assert_eq!(s.documents().len(), 3);
    assert_eq!(s.active().unwrap().doc.artboards.len(), 2, "both pages");
    let _ = std::fs::remove_dir_all(base);
}

/// `fullName` and `path` name the document's file and folder once it has one, `saved` follows its
/// changes, and `close` with `SaveOptions.DONOTSAVECHANGES` closes it.
#[test]
fn full_name_path_saved_and_close_follow_the_documents_file() {
    let base = folder("fullname");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    let d = quoted(&dir);
    let out = logged(
        &mut s,
        &base,
        &format!(
            "Folder.userData; var dir = {d}, d = app.documents.add(DocumentColorSpace.RGB, 200, 100);\n\
             vc.log([d.fullName instanceof File, d.fullName.name === d.name, d.fullName.fsName === new File(d.name).fsName, d.path === '', d.saved]);\n\
             vc.run('shape.rectangle', {{x: 10, y: 10, width: 50, height: 20}}); vc.log(d.saved);\n\
             d.saveAs(new File(dir + '/a.svg'));\n\
             vc.log([d.fullName instanceof File, d.fullName.fsName, d.fullName.name, d.path instanceof Folder, d.path.fsName, d.saved]);\n\
             vc.run('shape.rectangle', {{x: 20, y: 20, width: 5, height: 5}}); vc.log(d.saved);\n\
             var e = ''; try {{ d.close(); }} catch (x) {{ e = String(x); }} vc.log([e, app.documents.length]);\n\
             var before = new File(dir + '/a.svg').length; d.close(SaveOptions.SAVECHANGES);\n\
             vc.log([app.documents.length, new File(dir + '/a.svg').length > before]);\n\
             var n = app.documents.add(DocumentColorSpace.RGB, 200, 100); vc.run('shape.rectangle', {{x: 10, y: 10, width: 5, height: 5}});\n\
             e = ''; try {{ n.close(SaveOptions.SAVECHANGES); }} catch (x) {{ e = String(x); }} vc.log([e, app.documents.length]);\n\
             n.close(SaveOptions.DONOTSAVECHANGES); vc.log(app.documents.length);"
        ),
    );
    assert_eq!(out[0], json!([true, true, true, true, true]), "an untouched document that has no file");
    assert_eq!(out[1], json!(false), "an edit leaves changes to save");
    let file = dir.join("a.svg");
    assert_eq!(
        out[2],
        json!([true, file.to_string_lossy(), "a.svg", true, dir.to_string_lossy(), true]),
        "after saveAs the document's file and folder"
    );
    assert_eq!(out[3], json!(false));
    assert!(out[4][0].as_str().unwrap().contains("unsaved changes"), "{}", out[4]);
    assert_eq!(out[4][1], json!(1), "without SaveOptions the document stays open");
    assert_eq!(out[5], json!([0, true]), "SAVECHANGES saves the edit to the document's file and closes it");
    assert!(out[6][0].as_str().unwrap().contains("no file to save"), "{}", out[6]);
    assert_eq!(out[6][1], json!(1), "SAVECHANGES without a file leaves the document open");
    assert_eq!(out[7], json!(0));
    assert!(s.documents().is_empty());
    let e = failure(&mut s, &base, "app.documents.add(); app.activeDocument.close('DONOTSAVECHANGES');");
    assert!(e.contains("SaveOptions"), "{e}");
    let _ = std::fs::remove_dir_all(base);
}

/// A document whose file was saved under a relative name is in the working folder the file was
/// written in.
#[test]
fn path_of_a_file_saved_under_a_relative_name_is_the_working_folder() {
    let base = folder("relative");
    let mut s = session(&base);
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s.active_mut().unwrap().path = Some("drawing.svg".into());
    let out = logged(
        &mut s,
        &base,
        "var d = app.activeDocument; vc.log([d.path instanceof Folder, d.path === '' ? '' : d.path.fsName, d.fullName.fsName]);",
    );
    let cwd = std::env::current_dir().unwrap();
    assert_eq!(out[0][0], json!(true), "{out}");
    let same = |shown: &str, want: &Path| {
        let shown = PathBuf::from(shown);
        shown == want || std::fs::canonicalize(&shown).ok().zip(std::fs::canonicalize(want).ok()).is_some_and(|(a, b)| a == b)
    };
    assert!(same(out[0][1].as_str().unwrap(), &cwd), "{out} is not in {cwd:?}");
    let file = out[0][2].as_str().unwrap();
    assert!(Path::new(file).is_absolute() && Path::new(file).file_name() == Some("drawing.svg".as_ref()), "{file}");
    assert!(same(&Path::new(file).parent().unwrap().to_string_lossy(), &cwd), "{file} is not in {cwd:?}");
    let _ = std::fs::remove_dir_all(base);
}

/// The warnings a save gives, such as editing data left out of a PDF of one artboard of several, go
/// to the Script Console.
#[test]
fn a_saves_warnings_go_to_the_console() {
    let base = folder("warnings");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    s.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 2})).unwrap();
    let save = format!(
        "Folder.userData; var o = new PDFSaveOptions(); o.preserveEditability = true; o.artboardRange = '1';\n\
         app.activeDocument.saveAs(new File({}), o);",
        quoted(&dir.join("one.pdf"))
    );
    let r = run(&mut s, &base, &save).unwrap();
    let lines = r["console"]["lines"].as_array().unwrap();
    assert!(lines.iter().any(|l| l.as_str().is_some_and(|l| l.starts_with("saveAs: "))), "{lines:?}");
    let _ = std::fs::remove_dir_all(base);
}

/// A document without unsaved changes closes with `SaveOptions.SAVECHANGES` and with no argument.
#[test]
fn a_document_without_unsaved_changes_closes_with_any_save_options() {
    let base = folder("closeclean");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    let d = quoted(&dir);
    let out = logged(
        &mut s,
        &base,
        &format!(
            "Folder.userData; var dir = {d}, d = app.documents.add(DocumentColorSpace.RGB, 200, 100);\n\
             vc.run('shape.rectangle', {{x: 10, y: 10, width: 50, height: 20}}); d.saveAs(new File(dir + '/a.svg'));\n\
             var e = app.documents.add(DocumentColorSpace.RGB, 200, 100);\n\
             vc.log([d.saved, e.saved, app.documents.length]);\n\
             d.close(SaveOptions.SAVECHANGES); vc.log(app.documents.length);\n\
             e.close(); vc.log(app.documents.length);"
        ),
    );
    assert_eq!(out, json!([[true, true, 2], 1, 0]));
    let _ = std::fs::remove_dir_all(base);
}

/// `close(SaveOptions.SAVECHANGES)` saves in the format the document was last saved in, not the
/// one its file's extension names.
#[test]
fn closing_with_save_changes_keeps_the_documents_format() {
    let base = folder("closeformat");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    let d = quoted(&dir);
    logged(
        &mut s,
        &base,
        &format!(
            "Folder.userData; var d = app.documents.add(DocumentColorSpace.RGB, 200, 100);\n\
             vc.run('shape.rectangle', {{x: 10, y: 10, width: 50, height: 20}}); d.saveAs(new File({d} + '/p.svg'), new PDFSaveOptions());\n\
             vc.run('shape.rectangle', {{x: 20, y: 20, width: 5, height: 5}}); d.close(SaveOptions.SAVECHANGES); vc.log(app.documents.length);"
        ),
    );
    let bytes = std::fs::read(dir.join("p.svg")).unwrap_or_default();
    assert!(bytes.starts_with(b"%PDF"), "the save on close keeps the PDF format");
    assert!(s.documents().is_empty());
    let _ = std::fs::remove_dir_all(base);
}

/// Setting `Document.saved` records the document's saved state without saving it: `true` lets
/// `close()` close a document with edits, and `false` makes an untouched one ask.
#[test]
fn setting_saved_records_the_saved_state() {
    let base = folder("setsaved");
    let mut s = session(&base);
    data_folder(&mut s, &base);
    let out = logged(
        &mut s,
        &base,
        "var d = app.documents.add(DocumentColorSpace.RGB, 200, 100);\n\
         vc.run('shape.rectangle', {x: 10, y: 10, width: 50, height: 20});\n\
         vc.log(d.saved); d.saved = true; vc.log(d.saved); d.close(); vc.log(app.documents.length);\n\
         var e = app.documents.add(DocumentColorSpace.RGB, 200, 100); vc.log(e.saved); e.saved = false; vc.log(e.saved);\n\
         var m = ''; try { e.close(); } catch (x) { m = String(x); } vc.log([m, app.documents.length]);\n\
         m = ''; try { e.saved = 'yes'; } catch (x) { m = String(x); } vc.log(m);",
    );
    assert_eq!(out[0], json!(false), "an edit leaves changes to save");
    assert_eq!(out[1], json!(true));
    assert_eq!(out[2], json!(0), "close() closes a document set as saved");
    assert_eq!(out[3], json!(true));
    assert_eq!(out[4], json!(false));
    assert!(out[5][0].as_str().unwrap().contains("unsaved changes"), "{}", out[5]);
    assert_eq!(out[5][1], json!(1), "a document set as unsaved stays open");
    assert!(out[6].as_str().unwrap().contains("true or false"), "{}", out[6]);
    assert!(s.active().unwrap().is_dirty(), "the state stays after the run");
    let _ = std::fs::remove_dir_all(base);
}

/// With automation read roots that leave out the document's file, `fullName` and `path` throw
/// instead of naming another file.
#[test]
fn full_name_and_path_leave_out_a_file_outside_the_read_roots() {
    let base = folder("fullnameroots");
    let mut s = session(&base);
    data_folder(&mut s, &base);
    let file = base.join("outside/hidden.svg");
    s.execute("file.new", &json!({})).unwrap();
    s.execute("document.save", &json!({"path": file.to_string_lossy()})).unwrap();
    let query = "var d = app.activeDocument; vc.log([d.fullName.name, d.fullName.fsName, d.path === '' ? '' : d.path.fsName]);";
    let out = logged(&mut s, &base, query);
    let outside = base.join("outside");
    assert_eq!(out[0], json!(["hidden.svg", file.to_string_lossy(), outside.to_string_lossy()]), "no read roots: the whole path");
    let roots = crate::file_access::AutomationRoots::new(&[&base.join("main"), &base.join("settings")], &[&base.join("settings")]).unwrap().unwrap();
    let hidden = "var d = app.activeDocument, r = [];\n\
                  try { r.push(d.fullName.fsName); } catch (e) { r.push(e.message); }\n\
                  try { r.push(String(d.path)); } catch (e) { r.push(e.message); }\n\
                  vc.log(r);";
    let out = crate::file_access::confine(Some(&roots), || logged(&mut s, &base, hidden));
    for m in out[0].as_array().unwrap() {
        let m = m.as_str().unwrap();
        assert!(m.starts_with(DENIED) && m.ends_with("hidden.svg"), "{m}");
        assert!(!m.contains("outside"), "the folder outside the read roots is not named: {m}");
    }
    let before = std::fs::read(&file).unwrap();
    let e = failure(
        &mut s,
        &base,
        "vc.run('shape.rectangle', {x: 10, y: 10, width: 50, height: 20}); app.activeDocument.close(SaveOptions.SAVECHANGES);",
    );
    assert!(e.contains(DENIED) && !e.contains("outside"), "SAVECHANGES does not write a file the script may not: {e}");
    assert_eq!(std::fs::read(&file).unwrap(), before, "the file is left as it was");
    assert_eq!(s.documents().len(), 1, "the document stays open");
    let _ = std::fs::remove_dir_all(base);
}

/// A `pDFPreset` in square brackets names the built-in preset of the name inside them, and a name
/// that is not a preset either way is refused.
#[test]
fn a_bracketed_pdf_preset_names_the_built_in_preset() {
    let base = folder("brackets");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    let d = quoted(&dir);
    logged(
        &mut s,
        &base,
        &format!(
            "Folder.userData; var dir = {d}, d = app.documents.add(DocumentColorSpace.RGB, 200, 100);\n\
             vc.run('shape.rectangle', {{x: 10, y: 10, width: 50, height: 20}});\n\
             var o = new PDFSaveOptions(); o.pDFPreset = '[Smallest File Size]'; d.saveAs(new File(dir + '/small.pdf'), o);\n\
             o.pDFPreset = '[Illustrator Default]'; d.saveAs(new File(dir + '/default.pdf'), o);\n\
             o.pDFPreset = '[PDF/X-4:2008]'; d.saveAs(new File(dir + '/x4.pdf'), o);"
        ),
    );
    let small = std::fs::read(dir.join("small.pdf")).unwrap();
    assert!(String::from_utf8_lossy(&small[..200.min(small.len())]).contains("/Linearized 1"), "Smallest File Size writes a linearized file");
    assert!(dir.join("default.pdf").is_file());
    let x4 = String::from_utf8_lossy(&std::fs::read(dir.join("x4.pdf")).unwrap()).into_owned();
    assert!(x4.contains("PDF/X-4"), "the PDF/X-4 preset writes a PDF/X-4 file");
    for bad in ["[Nope]", "[Basic Default]", "[Ilustrator Default]"] {
        let e = failure(
            &mut s,
            &base,
            &format!("Folder.userData; var o = new PDFSaveOptions(); o.pDFPreset = '{bad}'; app.activeDocument.saveAs(new File({d} + '/n.pdf'), o);"),
        );
        assert!(e.contains("unknown PDF preset"), "{bad}: {e}");
    }
    let _ = std::fs::remove_dir_all(base);
}

/// A number given as `artboardRange` is read as `String(number)` reads it.
#[test]
fn a_number_artboard_range_names_that_artboard() {
    let base = folder("rangenumber");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    let d = quoted(&dir);
    logged(
        &mut s,
        &base,
        &format!(
            "Folder.userData; var dir = {d}, d = app.documents.add(DocumentColorSpace.RGB, 200, 100);\n\
             vc.run('artboard.new', {{x: 300, y: 0, width: 60, height: 80}});\n\
             var o = new PDFSaveOptions(); o.artboardRange = 2; d.saveAs(new File(dir + '/two.pdf'), o);"
        ),
    );
    let pdf = vectorcraft_pdf::import(&std::fs::read(dir.join("two.pdf")).unwrap()).unwrap();
    assert_eq!(pdf.artboards.len(), 1, "one page");
    let r = pdf.artboards[0].rect;
    assert!((r.width() - 60.0).abs() < 0.01 && (r.height() - 80.0).abs() < 0.01, "the second artboard: {r:?}");
    let _ = std::fs::remove_dir_all(base);
}

/// With `saveMultipleArtboards`, a `.ai` or EPS save also writes a file for each artboard of
/// `artboardRange`; without it the range is not used, and only a value that is not written as a
/// range throws.
#[test]
fn save_multiple_artboards_writes_a_file_per_artboard() {
    let base = folder("multiple");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    let d = quoted(&dir);
    let names = |sub: &str| {
        let mut v: Vec<String> = std::fs::read_dir(dir.join(sub)).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    };
    logged(
        &mut s,
        &base,
        &format!(
            "Folder.userData; var dir = {d}, d = app.documents.add(DocumentColorSpace.RGB, 200, 100);\n\
             vc.run('artboard.new', {{x: 300, y: 0, width: 60, height: 80}});\n\
             vc.run('shape.rectangle', {{x: 10, y: 10, width: 50, height: 20}});\n\
             var names = ['ai', 'eps', 'aione', 'epsone', 'aioff'];\n\
             for (var i = 0; i < names.length; i++) new Folder(dir + '/' + names[i]).create();\n\
             var a = new IllustratorSaveOptions(); a.saveMultipleArtboards = true; d.saveAs(new File(dir + '/ai/a.ai'), a);\n\
             var e = new EPSSaveOptions(); e.saveMultipleArtboards = true; d.saveAs(new File(dir + '/eps/a.eps'), e);\n\
             a.artboardRange = '2'; d.saveAs(new File(dir + '/aione/a.ai'), a);\n\
             e.artboardRange = 2; d.saveAs(new File(dir + '/epsone/a.eps'), e);\n\
             a.saveMultipleArtboards = false; d.saveAs(new File(dir + '/aioff/a.ai'), a);"
        ),
    );
    assert_eq!(names("ai"), ["a-Artboard-1.ai", "a-Artboard-2.ai", "a.ai"], "the file and one for each of the two artboards");
    assert_eq!(names("eps"), ["a.eps", "a_Artboard-1.eps", "a_Artboard-2.eps"], "the central file and one for each of the two artboards");
    assert_eq!(names("aione"), ["a-Artboard-2.ai", "a.ai"], "the file and one for artboard 2");
    assert_eq!(names("epsone"), ["a.eps"], "one artboard: its file alone");
    let eps = String::from_utf8_lossy(&std::fs::read(dir.join("epsone/a.eps")).unwrap()).into_owned();
    let bbox = eps.lines().find(|l| l.starts_with("%%BoundingBox:")).unwrap_or_default();
    assert_eq!(bbox.split_whitespace().skip(1).collect::<Vec<_>>(), ["0", "0", "60", "80"], "the file holds artboard 2: {bbox}");
    assert_eq!(names("aioff"), ["a.ai"], "without saveMultipleArtboards the range is not used");
    logged(
        &mut s,
        &base,
        &format!(
            "Folder.userData; var a = new IllustratorSaveOptions(); a.artboardRange = 3; \
             app.activeDocument.saveAs(new File({d} + '/aioff/c.ai'), a);"
        ),
    );
    assert!(dir.join("aioff/c.ai").is_file(), "a range past the document's artboards is not used and does not stop the save");
    for bad in ["'abc'", "'0-'", "'3-1'", "'-'", "','", "' , '", "-1", "2.5", "0"] {
        let e = failure(
            &mut s,
            &base,
            &format!(
                "Folder.userData; var a = new IllustratorSaveOptions(); a.artboardRange = {bad}; \
                 app.activeDocument.saveAs(new File({d} + '/aioff/b.ai'), a);"
            ),
        );
        assert!(e.contains("range"), "{bad}: {e}");
    }
    assert!(!dir.join("aioff/b.ai").exists(), "a bad range writes nothing");
    let _ = std::fs::remove_dir_all(base);
}
