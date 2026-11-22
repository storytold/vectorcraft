//! Printing from a script: `Document.print` with `PrintOptions` and its option classes,
//! `app.printPresetsList`, `app.PPDFileList`, and the checks every option and the file go through.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::*;

const DENIED: &str = "Script file access denied";

/// A fresh folder for test `name`, by its real path, with `main` for the script.
fn folder(name: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("vc-script-print-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("main")).unwrap();
    std::fs::create_dir_all(base.join("outside")).unwrap();
    std::fs::canonicalize(&base).unwrap()
}

/// A session whose scripts keep their data folders in `base/settings/Script Data`, with a
/// 300 × 200 document of two artboards holding a rectangle.
fn session(base: &Path) -> Session {
    let mut s = Session::new();
    s.attach_script_data(base.join("settings/Script Data"));
    s.execute("file.new", &json!({"width": 300, "height": 200, "artboards": 2})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 50})).unwrap();
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

fn pages(path: &Path) -> Vec<(f64, f64)> {
    let info = vectorcraft_pdf::info(&std::fs::read(path).unwrap(), None).unwrap();
    info.pages.iter().map(|p| (p.width.round(), p.height.round())).collect()
}

#[test]
fn print_writes_the_file_with_the_assigned_options_over_the_saved_settings() {
    let base = folder("happy");
    let mut s = session(&base);
    s.execute("print.setup", &json!({"settings": {"collate": true, "marks": {"trim": true}}})).unwrap();
    let saved = s.execute("print.setup", &json!({})).unwrap()["settings"].clone();
    let undo = s.active().unwrap().history.undo.len();
    let dir = data_folder(&mut s, &base);
    let d = quoted(&dir);
    let out = logged(
        &mut s,
        &base,
        &format!(
            "Folder.userData; var dir = {d}, doc = app.activeDocument;\n\
             var o = new PrintOptions(), j = new PrintJobOptions();\n\
             vc.log([o.typename, j.typename, j.copies, j.collate, j.printAllArtboards, j.designation, o.coordinateOptions.typename]);\n\
             o.jobOptions = j; j.copies = 2; j.file = new File(dir + '/a.pdf');\n\
             j.copies = 3; vc.log([o.jobOptions.copies, j.collate]);\n\
             doc.print(o);\n\
             var back = o.jobOptions.file; j.file = back; vc.log([back instanceof File, back.name]);\n\
             j.printAllArtboards = false; j.artboardRange = '2'; j.copies = 1; j.file = new File(dir + '/b.pdf');\n\
             o.paperOptions.name = 'a4'; o.coordinateOptions.fitToPage = true; o.coordinateOptions.orientation = PrintOrientation.PORTRAIT;\n\
             o.printerName = ''; o.fontOptions.downloadFonts = PrintFontDownloadMode.DOWNLOADSUBSET;\n\
             doc.print(o);\n\
             j.file = new File(dir + '/c.ps'); doc.print(o);\n\
             o.postScriptOptions.postScriptLevel = PrinterPostScriptLevelEnum.PSLEVEL2; j.file = new File(dir + '/level2.prn');\n\
             doc.print(o);\n\
             o.postScriptOptions.postScriptLevel = PrinterPostScriptLevelEnum.PSLEVEL3; j.file = new File(dir + '/level3.ps');\n\
             doc.print(o);\n\
             vc.log([app.printPresetsList[0], app.PPDFileList.length]);"
        ),
    );
    let expected =
        json!(["PrintOptions", "PrintJobOptions", 1, false, true, "PrintArtworkDesignation.VISIBLEPRINTABLELAYERS", "PrintCoordinateOptions"]);
    assert_eq!(out[0], expected, "a new option object reads as its help page's defaults");
    assert_eq!(out[1], json!([3, false]), "a change after the assignment is read through the PrintOptions");
    assert_eq!(out[2], json!([true, "a.pdf"]), "jobOptions.file reads back as a File");
    assert_eq!(out[3], json!(["[Default]", 0]));
    assert_eq!(pages(&dir.join("a.pdf")).len(), 6, "two artboards, three copies");
    assert_eq!(pages(&dir.join("b.pdf")), vec![(595.0, 842.0)], "artboard 2 only, on upright A4");
    let ps = std::fs::read_to_string(dir.join("c.ps")).unwrap();
    assert!(ps.starts_with("%!PS") && ps.contains("%%LanguageLevel: 2"), "a .ps name alone prints at PSLEVEL2: {}", &ps[..ps.len().min(200)]);
    let ps = std::fs::read_to_string(dir.join("level2.prn")).unwrap();
    assert!(ps.starts_with("%!PS") && ps.contains("%%LanguageLevel: 2"), "a level alone: {}", &ps[..ps.len().min(200)]);
    let ps = std::fs::read_to_string(dir.join("level3.ps")).unwrap();
    assert!(ps.starts_with("%!PS") && ps.contains("%%LanguageLevel: 3"), "an assigned PSLEVEL3: {}", &ps[..ps.len().min(200)]);
    assert_eq!(s.execute("print.setup", &json!({})).unwrap()["settings"], saved, "printing leaves the saved settings as they are");
    assert_eq!(s.active().unwrap().history.undo.len(), undo, "and adds no undo step");
    // The commands stay closed to vc.run, before and after a print.
    for case in [
        "vc.run('file.print', {});",
        "var o = new PrintOptions(); o.jobOptions.file = new File(Folder.userData.fsName + '/d.pdf'); app.activeDocument.print(o); vc.run('file.print', {});",
        "vc.run('print.setup', {settings: {copies: 5}});",
    ] {
        let e = failure(&mut s, &base, case);
        assert!(e.contains("a script cannot run"), "{case}: {e}");
    }
    assert_eq!(s.execute("print.setup", &json!({})).unwrap()["settings"], saved);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn print_without_a_file_does_not_print() {
    let base = folder("no-file");
    let mut s = session(&base);
    for case in [
        "app.activeDocument.print();",
        "app.activeDocument.print(new PrintOptions());",
        "var o = new PrintOptions(); o.jobOptions.copies = 2; app.activeDocument.print(o);",
    ] {
        let e = failure(&mut s, &base, case);
        assert!(e.contains("Printing to a printer is not available to scripts yet"), "{case}: {e}");
    }
    let e = failure(&mut s, &base, "app.printerList;");
    assert!(e.contains("Application.printerList is not available yet"), "{e}");
    let _ = std::fs::remove_dir_all(base);
}

/// Bad values throw when they are assigned, and nothing prints.
#[test]
fn print_options_refuse_bad_values() {
    let base = folder("bad");
    let mut s = session(&base);
    let cases = [
        ("j.copies = 0;", "RangeError", "copies must be a whole number from 1 to 999"),
        ("j.copies = NaN;", "TypeError", "copies"),
        ("j.copies = 1e12;", "RangeError", "copies must be a whole number"),
        ("j.copies = 1.5;", "RangeError", "copies must be a whole number"),
        ("j.copies = '2';", "TypeError", "copies must be a number"),
        ("j.collate = 1;", "TypeError", "collate must be true or false"),
        ("c.horizontalScale = NaN;", "TypeError", "horizontalScale"),
        ("c.horizontalScale = Infinity;", "TypeError", "horizontalScale"),
        ("c.verticalScale = 20000;", "RangeError", "verticalScale must be from 1 to 10000"),
        ("c.verticalScale = 0;", "RangeError", "verticalScale must be from 1 to 10000"),
        ("m.trimMarksWeight = -1;", "RangeError", "trimMarksWeight must be from 0"),
        ("c.orientation = 'PrintOrientation.SIDEWAYS';", "TypeError", "orientation must be a PrintOrientation value"),
        ("c.orientation = 'PrintTiling.TILEFULLPAGES';", "TypeError", "orientation must be a PrintOrientation value"),
        ("j.file = 'C:/out.pdf';", "TypeError", "file must be a File"),
        ("j.file = {};", "TypeError", "file must be a File"),
        ("o.jobOptions = new PrintCoordinateOptions();", "TypeError", "jobOptions must be a PrintJobOptions"),
        ("o.jobOptions = {copies: 2};", "TypeError", "jobOptions must be a PrintJobOptions"),
        ("o.printPreset = new Array(5000).join('x');", "RangeError", "printPreset is too long"),
        ("app.activeDocument.print({jobOptions: j});", "TypeError", "print's options must be a PrintOptions object"),
        ("app.activeDocument.print(j);", "TypeError", "print's options must be a PrintOptions object"),
    ];
    let prefix = "var o = new PrintOptions(), j = o.jobOptions, c = o.coordinateOptions, m = o.pageMarksOptions;";
    for (case, kind, part) in cases {
        let e = failure(&mut s, &base, &format!("{prefix} {case}"));
        assert!(e.contains(kind) && e.contains(part), "{case}: {e}");
    }
    let _ = std::fs::remove_dir_all(base);
}

/// Options that `file.print` cannot honor throw instead of being dropped.
#[test]
fn options_that_are_not_available_throw() {
    let base = folder("later");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    let file = format!("j.file = new File({});", quoted(&dir.join("x.pdf")));
    let cases = [
        ("o.printerName = 'Office';", "PrintOptions.printerName is not available yet"),
        ("o.printerName = '\\\\\\\\server\\\\share';", "PrintOptions.printerName is not available yet"),
        ("o.PPDName = 'Generic';", "PrintOptions.PPDName is not available yet"),
        ("o.PPDName = '/etc/cups/ppd/office.ppd';", "PrintOptions.PPDName is not available yet"),
        ("j.name = 'Job';", "PrintJobOptions.name is not available yet"),
        ("j.printArea = PrintingBounds.ARTWORKBOUNDS;", "PrintingBounds.ARTWORKBOUNDS is not available yet"),
        ("o.colorSeparationOptions.colorSeparationMode = PrintColorSeparationMode.INRIPSEPARATION;", "INRIPSEPARATION is not available yet"),
        ("o.colorManagementOptions.colorProfileMode = PrintColorProfile.PRINTERPROFILE;", "PRINTERPROFILE is not available yet"),
        ("o.colorSeparationOptions.inkList = [];", "PrintColorSeparationOptions.inkList is not available yet"),
        ("o.fontOptions.downloadFonts = PrintFontDownloadMode.DOWNLOADCOMPLETE;", "PrintFontOptions.downloadFonts is not available yet"),
        ("o.flattenerOptions.rasterizationResolution = 600;", "PrintFlattenerOptions.rasterizationResolution is not available yet"),
        ("o.flattenerOptions.rasterizationResolution = NaN;", "PrintFlattenerOptions.rasterizationResolution"),
        ("o.pageMarksOptions.bleedOffsetRect = [0, 0, 0, 0];", "PrintPageMarksOptions.bleedOffsetRect is not available yet"),
        ("o.paperOptions.offset = 10;", "PrintPaperOptions.offset is not available yet"),
        ("o.postScriptOptions.postScriptLevel = PrinterPostScriptLevelEnum.PSLEVEL1;", "PSLEVEL1 is not available yet"),
        ("o.postScriptOptions.binaryPrinting = true;", "PrintPostScriptOptions.binaryPrinting is not available yet"),
        (
            &format!("{file} o.coordinateOptions.horizontalScale = 2000; app.activeDocument.print(o);"),
            "horizontalScale above 1000 is not available yet",
        ),
    ];
    let prefix = "Folder.userData; var o = new PrintOptions(), j = o.jobOptions;";
    for (case, part) in cases {
        let e = failure(&mut s, &base, &format!("{prefix} {case}"));
        assert!(e.contains(part), "{case}: {e}");
    }
    assert!(!dir.join("x.pdf").exists(), "nothing printed");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn print_writes_only_where_the_script_may() {
    let base = folder("refused");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    let grants = base.join("settings/Script Grants.json");
    std::fs::write(&grants, "{}").unwrap();
    for target in [base.join("outside/x.pdf"), grants.clone(), dir.clone()] {
        let case =
            format!("Folder.userData; var o = new PrintOptions(); o.jobOptions.file = new File({}); app.activeDocument.print(o);", quoted(&target));
        let e = failure(&mut s, &base, &case);
        assert!(e.contains("Script file"), "{case}: {e}");
        if target != dir {
            assert!(e.contains(DENIED), "{case}: {e}");
        }
    }
    assert!(!base.join("outside/x.pdf").exists());
    assert_eq!(std::fs::read_to_string(&grants).unwrap(), "{}");
    let _ = std::fs::remove_dir_all(base);
}

/// A print to a file a `File` object holds open for writing throws, so the object's bytes written at
/// the end of the run do not replace the printed file.
#[test]
fn print_does_not_write_a_file_open_for_writing() {
    let base = folder("openhandle");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    let p = quoted(&dir.join("x.pdf"));
    let log = logged(
        &mut s,
        &base,
        &format!(
            "Folder.userData; var w = new File({p}); w.open('w'); w.write('pending');\nvar o = new PrintOptions(); o.jobOptions.file = new File({p});\ntry {{ app.activeDocument.print(o); vc.log('ran'); }} catch (e) {{ vc.log(e.message); }}"
        ),
    );
    assert!(log[0].as_str().unwrap().contains(crate::script::files::OPEN_FOR_WRITING), "{log}");
    assert_eq!(std::fs::read_to_string(dir.join("x.pdf")).unwrap(), "pending", "the File's own bytes");
    let _ = std::fs::remove_dir_all(base);
}

/// One assigned scale factor leaves the other at the document's saved value.
#[test]
fn one_assigned_scale_factor_keeps_the_saved_other() {
    let base = folder("scale");
    let mut s = session(&base);
    let common = json!({"media": "letter", "autoRotate": false, "orientation": "portrait", "artboards": "range", "range": "1"});
    let mut saved = common.clone();
    saved["scale"] = json!({"width": 100, "height": 500, "linked": false});
    s.execute("print.setup", &json!({"settings": saved})).unwrap();
    let dir = data_folder(&mut s, &base);
    let mut expected = common;
    expected["scaling"] = json!("tileFull");
    expected["scale"] = json!({"width": 400, "height": 500, "linked": false});
    let want = s.execute("file.print", &json!({"settings": expected, "path": dir.join("want.pdf").to_string_lossy()})).unwrap();
    logged(
        &mut s,
        &base,
        &format!(
            "Folder.userData; var o = new PrintOptions(); o.jobOptions.file = new File({});\n\
             o.coordinateOptions.horizontalScale = 400; o.coordinateOptions.tiling = PrintTiling.TILEFULLPAGES;\n\
             app.activeDocument.print(o);",
            quoted(&dir.join("got.pdf"))
        ),
    );
    let got = pages(&dir.join("got.pdf"));
    assert_eq!(got.len() as u64, want["pages"].as_u64().unwrap(), "the saved vertical scale of 500% is kept");
    assert!(got.len() >= 4, "1200 by 1000 pt of art tiles over at least four letter pages: {got:?}");
    let _ = std::fs::remove_dir_all(base);
}

/// An assigned `artboardRange` replaces a saved range, an assigned `printArea` prints by artboard over
/// saved settings that ignore the artboards, and two scale factors that differ by any amount print
/// unlinked.
#[test]
fn assigned_job_and_scale_options_apply_over_the_saved_settings() {
    let base = folder("over-saved");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    let print = |s: &mut Session, name: &str, body: &str| {
        let file = quoted(&dir.join(name));
        logged(
            s,
            &base,
            &format!("Folder.userData; var o = new PrintOptions(); o.jobOptions.file = new File({file}); {body} app.activeDocument.print(o);"),
        );
        pages(&dir.join(name)).len()
    };
    s.execute("print.setup", &json!({"settings": {"artboards": "range", "range": "1"}})).unwrap();
    assert_eq!(print(&mut s, "saved.pdf", ""), 1, "the saved range");
    assert_eq!(print(&mut s, "range.pdf", "o.jobOptions.artboardRange = '1-2';"), 2, "the assigned range replaces the saved one");
    s.execute("print.setup", &json!({"settings": {"artboards": "ignore"}})).unwrap();
    assert_eq!(print(&mut s, "ignore.pdf", ""), 1, "all the art as one page");
    assert_eq!(print(&mut s, "area.pdf", "o.jobOptions.printArea = PrintingBounds.ARTBOARDBOUNDS;"), 2, "each artboard prints");
    assert_eq!(print(&mut s, "range-ignored.pdf", "o.jobOptions.artboardRange = '2';"), 1, "a range alone keeps ignoring the artboards");
    let area_range = "o.jobOptions.printArea = PrintingBounds.ARTBOARDBOUNDS; o.jobOptions.artboardRange = '2';";
    assert_eq!(print(&mut s, "area-range.pdf", area_range), 1, "printArea with a range prints the range");
    let scaled = "o.coordinateOptions.horizontalScale = 100; o.coordinateOptions.verticalScale = 100.0000000005;";
    assert!(print(&mut s, "scale.pdf", scaled) >= 1, "factors that differ by a tiny amount print unlinked");
    assert_eq!(s.execute("print.setup", &json!({})).unwrap()["settings"]["artboards"], json!("ignore"), "the saved settings stay");
    let _ = std::fs::remove_dir_all(base);
}

/// The format follows the name `jobOptions.file` gives, not the extension of the file a link in the
/// granted folder leads to.
#[cfg(unix)]
#[test]
fn the_format_follows_the_name_the_script_gives() {
    let base = folder("format-name");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    std::fs::write(dir.join("proof.pdf"), "old").unwrap();
    std::os::unix::fs::symlink(dir.join("proof.pdf"), dir.join("proof.ps")).unwrap();
    logged(
        &mut s,
        &base,
        &format!(
            "Folder.userData; var o = new PrintOptions(); o.jobOptions.file = new File({}); app.activeDocument.print(o);",
            quoted(&dir.join("proof.ps"))
        ),
    );
    let written = std::fs::read(dir.join("proof.pdf")).unwrap();
    assert!(written.starts_with(b"%!PS"), "PostScript for a .ps name: {:?}", String::from_utf8_lossy(&written[..written.len().min(40)]));
    let _ = std::fs::remove_dir_all(base);
}

/// Artboard ranges the document does not hold and preset names that are not presets, such as
/// paths, throw before anything is written.
#[test]
fn print_refuses_ranges_and_preset_names_it_cannot_honor() {
    let base = folder("names");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    let file = format!("j.file = new File({});", quoted(&dir.join("x.pdf")));
    let cases = [
        ("j.printAllArtboards = false; j.artboardRange = '3';", "RangeError", "artboardRange: range `3`"),
        ("j.printAllArtboards = false; j.artboardRange = '0-1';", "RangeError", "artboardRange"),
        ("j.printAllArtboards = false; j.artboardRange = '../../outside';", "RangeError", "artboardRange"),
        ("j.printAllArtboards = false; j.artboardRange = '2-1';", "RangeError", "artboardRange"),
        ("j.artboardRange = '3';", "RangeError", "artboardRange: range `3`"),
        ("o.printPreset = 'Office';", "Error", "printPreset names no print preset: Office"),
        (&format!("o.printPreset = {};", quoted(&base.join("settings/Script Grants.json"))), "Error", "printPreset names no print preset"),
        ("o.printPreset = '../../preset';", "Error", "printPreset names no print preset"),
        ("o.flattenerPreset = '/etc/passwd';", "Error", "flattenerPreset names no flattener preset"),
        (&format!("o.flattenerPreset = {};", quoted(&base.join("outside/f.xml"))), "Error", "flattenerPreset names no flattener preset"),
    ];
    let prefix = format!("Folder.userData; var o = new PrintOptions(), j = o.jobOptions; {file}");
    for (case, kind, part) in cases {
        let e = failure(&mut s, &base, &format!("{prefix} {case} app.activeDocument.print(o);"));
        assert!(e.contains(kind) && e.contains(part), "{case}: {e}");
    }
    assert!(!dir.join("x.pdf").exists(), "nothing printed");
    // The names the lists give print, in any case.
    logged(&mut s, &base, &format!("{prefix} o.printPreset = '[default]'; o.flattenerPreset = 'high resolution'; app.activeDocument.print(o);"));
    assert!(dir.join("x.pdf").exists());
    let _ = std::fs::remove_dir_all(base);
}

/// A color profile, a paper or a preset name that is empty, path-shaped, holds a control character
/// or is not on its list throws before anything is written; a listed name in any case prints.
#[test]
fn print_refuses_profile_paper_and_blank_preset_names() {
    let base = folder("profile-paper");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    let file = format!("j.file = new File({});", quoted(&dir.join("x.pdf")));
    let profile = "PrintColorManagementOptions.name names no color profile";
    let paper = "PrintPaperOptions.name names no paper";
    let cases = [
        ("c.name = '../sRGB IEC61966-2.1';", profile),
        ("c.name = 'C:/Windows/System32/spool/drivers/color/sRGB IEC61966-2.1';", profile),
        ("c.name = '\\\\\\\\server\\\\share\\\\cmyk.icc';", profile),
        ("c.name = 'No Such Profile';", profile),
        ("c.colorProfileMode = PrintColorProfile.SOURCEPROFILE; c.name = '../cmyk.icc';", profile),
        ("c.name = '   ';", "PrintColorManagementOptions.name must name a color profile"),
        ("p.name = '../letter';", paper),
        ("p.name = 'let\\u0007ter';", paper),
        ("p.name = '\\tA4\\n';", paper),
        ("o.printPreset = '[Default]\\n';", "PrintOptions.printPreset names no print preset"),
        ("p.name = 'Office Tray 2';", paper),
        ("p.name = '';", "PrintPaperOptions.name must name a paper"),
        ("p.name = 'a4'; p.width = 300;", "PrintPaperOptions.width and height apply only to custom paper, not a4"),
        ("o.flattenerPreset = '   ';", "PrintOptions.flattenerPreset must name a flattener preset"),
        ("o.flattenerPreset = '';", "PrintOptions.flattenerPreset must name a flattener preset"),
        ("o.printPreset = ' ';", "PrintOptions.printPreset must name a print preset"),
    ];
    let prefix = format!("Folder.userData; var o = new PrintOptions(), j = o.jobOptions, c = o.colorManagementOptions, p = o.paperOptions; {file}");
    for (case, part) in cases {
        let e = failure(&mut s, &base, &format!("{prefix} {case} app.activeDocument.print(o);"));
        assert!(e.contains(part), "{case}: {e}");
    }
    assert!(!dir.join("x.pdf").exists(), "nothing printed");
    // Listed names print in any case and with spaces around them.
    logged(
        &mut s,
        &base,
        &format!(
            "{prefix} c.colorProfileMode = PrintColorProfile.CUSTOMPROFILE; c.name = 'srgb iec61966-2.1'; p.name = ' A4 ';\n\
             o.printPreset = ' [Default] '; o.flattenerPreset = ' High Resolution '; app.activeDocument.print(o);"
        ),
    );
    let sizes = pages(&dir.join("x.pdf"));
    assert!(!sizes.is_empty() && sizes.iter().all(|&(w, h)| [(595.0, 842.0), (842.0, 595.0)].contains(&(w, h))), "A4: {sizes:?}");
    // A custom paper takes the assigned size.
    let custom = format!(
        "Folder.userData; var o = new PrintOptions(), p = o.paperOptions; o.jobOptions.file = new File({});\n\
         p.name = 'Custom'; p.width = 360; p.height = 504; o.coordinateOptions.orientation = PrintOrientation.PORTRAIT;\n\
         app.activeDocument.print(o);",
        quoted(&dir.join("custom.pdf"))
    );
    logged(&mut s, &base, &custom);
    let sizes = pages(&dir.join("custom.pdf"));
    assert!(!sizes.is_empty() && sizes.iter().all(|&size| size == (360.0, 504.0)), "custom: {sizes:?}");
    let _ = std::fs::remove_dir_all(base);
}

/// Each warning `file.print` gives goes to the Script Console.
#[test]
fn print_writes_its_warnings_to_the_script_console() {
    let base = folder("warnings");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    let settings = json!({"color": {"profile": "sRGB IEC61966-2.1"}});
    let want =
        s.execute("file.print", &json!({"format": "postscript", "settings": settings, "path": dir.join("want.ps").to_string_lossy()})).unwrap();
    let warnings: Vec<String> = want["warnings"].as_array().unwrap().iter().map(|w| format!("Document.print: {}", w.as_str().unwrap())).collect();
    assert!(warnings.iter().any(|w| w.contains("printer profile")), "{want}");
    let report = run(
        &mut s,
        &base,
        &format!(
            "Folder.userData; var o = new PrintOptions(); o.jobOptions.file = new File({});\n\
             o.colorManagementOptions.name = 'sRGB IEC61966-2.1'; o.postScriptOptions.postScriptLevel = PrinterPostScriptLevelEnum.PSLEVEL3;\n\
             app.activeDocument.print(o);",
            quoted(&dir.join("got.ps"))
        ),
    )
    .unwrap();
    assert_eq!(report["console"]["lines"], json!(warnings));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn option_objects_are_bounded_and_go_stale_after_their_run() {
    let base = folder("bounds");
    let mut s = session(&base);
    let e = failure(&mut s, &base, "for (var i = 0; i < 1000; i++) new PrintJobOptions();");
    assert!(e.contains("RangeError") && e.contains("at most 256 print option objects"), "{e}");
    let e = failure(&mut s, &base, "var o = new PrintOptions(); for (var i = 0; i < 1000; i++) o.jobOptions = new PrintJobOptions();");
    assert!(e.contains("at most 256"), "{e}");
    // A named engine keeps its globals, but an option object from an earlier run is stale.
    run(&mut s, &base, "#targetengine keep\nvar held = new PrintOptions();").unwrap();
    let e = failure(&mut s, &base, "#targetengine keep\nnew PrintOptions(); held.printPreset;");
    assert!(e.contains("Stale print options"), "{e}");
    let e = failure(&mut s, &base, "#targetengine keep\napp.activeDocument.print(held);");
    assert!(e.contains("Stale print options"), "{e}");
    let e = failure(&mut s, &base, "#targetengine keep\nvar o = new PrintOptions(); o.jobOptions = held.jobOptions;");
    assert!(e.contains("Stale print options"), "{e}");
    let _ = std::fs::remove_dir_all(base);
}
