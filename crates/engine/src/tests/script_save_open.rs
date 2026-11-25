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
             d.exportFile(new File(dir + '/s1'), ExportType.SVG);\n\
             d.exportFile(new File(dir + '/s2'), ExportType.SVG, new ExportOptionsSVG());\n\
             d.saveAs(new File(dir + '/a1.pdf'));\n\
             d.saveAs(new File(dir + '/a2.pdf'), new PDFSaveOptions());"
        ),
    );
    for (one, two) in [("j1.jpg", "j2.jpg"), ("p1.png", "p2.png"), ("s1.svg", "s2.svg")] {
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
    let lines: Vec<&str> = dxf.lines().map(str::trim).collect();
    // The section's name is a value; the group codes start on the line after it.
    let start = lines.iter().position(|l| *l == "ENTITIES").map_or(0, |i| i + 1);
    lines[start..].chunks(2).filter(|p| p.len() == 2 && p[0] == "10").filter_map(|p| p[1].parse::<f64>().ok()).fold(f64::MIN, f64::max)
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
    }
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
/// still writes none of them after the refused one.
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
    let gate = ScriptGate { read: ScriptScope::default(), write: ScriptScope::of_folder(&dir), documents: vec![], held: vec![], also: None };
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
    assert!(r.is_err(), "{r:?}");
    let refused = refused.unwrap();
    assert_eq!(written, std::slice::from_ref(&refused), "only the refused file went to the writer");
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
