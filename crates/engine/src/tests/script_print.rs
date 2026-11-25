//! Printing from a script: `Document.print` with `PrintOptions` and its option classes,
//! `app.printPresetsList`, `app.PPDFileList`, and the checks every option and the file go through.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::script::printing::{MAX_PRINTER_NAME_BYTES, MAX_PRINTERS, PreparedPrint, Printers};
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
    assert!(ps.starts_with("%!PS") && !ps.contains("%%LanguageLevel: 2"), "a .ps name alone: {}", &ps[..ps.len().min(200)]);
    let ps = std::fs::read_to_string(dir.join("level2.prn")).unwrap();
    assert!(ps.starts_with("%!PS") && ps.contains("%%LanguageLevel: 2"), "a level alone: {}", &ps[..ps.len().min(200)]);
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

/// Without the printers a frontend lists for scripts (a run outside the app, the web), printing to
/// a printer and `app.printerList` throw, and nothing is prepared.
#[test]
fn print_without_a_file_does_not_print() {
    let base = folder("no-file");
    let mut s = session(&base);
    for case in [
        "app.activeDocument.print();",
        "app.activeDocument.print(new PrintOptions());",
        "var o = new PrintOptions(); o.jobOptions.copies = 2; app.activeDocument.print(o);",
        "var o = new PrintOptions(); o.printerName = 'Office'; app.activeDocument.print(o);",
    ] {
        let e = failure(&mut s, &base, case);
        assert!(e.contains("Printing to a printer is not available here"), "{case}: {e}");
    }
    let e = failure(&mut s, &base, "app.printerList;");
    assert!(e.contains("Application.printerList is not available here"), "{e}");
    assert!(!s.script_prints_waiting());
    let _ = std::fs::remove_dir_all(base);
}

/// The printers a frontend lists, `Office` the default, with a name a shell or a file system would
/// read as more than a name.
const ODD_NAME: &str = "Lab; rm -rf / & \\\\srv\\q $(x) ../p";

fn printers() -> Printers {
    Printers::new([("Office".to_string(), true), (ODD_NAME.to_string(), false)], false)
}

/// The PDF pages a prepared job holds.
fn job_pages(job: &PreparedPrint) -> usize {
    vectorcraft_pdf::info(&job.pdf, None).unwrap().pages.len()
}

/// `app.printerList` gives a `Printer` for each printer the frontend listed, at most 256 of them,
/// without names that are empty, too long, hold a control character or repeat; the list is the
/// run's own, needs an open document, and its objects go stale after the run.
#[test]
fn printer_list_is_the_bounded_list_the_frontend_gave() {
    let base = folder("printer-list");
    let mut s = session(&base);
    let mut list: Vec<(String, bool)> = (0..300).map(|i| (format!("Printer {i}"), i == 1)).collect();
    list.insert(0, ("x".repeat(MAX_PRINTER_NAME_BYTES + 1), true));
    list.insert(1, ("Bad\nName".into(), false));
    list.insert(2, (String::new(), false));
    list.insert(3, ("Printer 5".into(), false));
    let snapshot = Printers::new(list, false);
    assert_eq!(snapshot.names().len(), MAX_PRINTERS);
    assert_eq!(snapshot.default_name(), Some("Printer 1"));
    // A default listed after the first 256 takes the last place, and a repeated name keeps a later
    // default mark without a second entry.
    let late = Printers::new((0..300).map(|i| (format!("Q{i}"), i == 280)), false);
    let names = late.names();
    assert_eq!((names.len(), late.default_name(), names[254].as_str(), names[255].as_str()), (MAX_PRINTERS, Some("Q280"), "Q254", "Q280"));
    let repeat = Printers::new([("A".to_string(), false), ("B".to_string(), false), ("A".to_string(), true)], false);
    assert_eq!((repeat.names(), repeat.default_name()), (&["A".to_string(), "B".to_string()][..], Some("A")));
    s.script_printers = Some(snapshot);
    let out = logged(
        &mut s,
        &base,
        "#targetengine keep\nvar l = app.printerList; held = l[0];\n\
         vc.log([l.length, l[0].name, l[0].typename, l[1].name, l[255].name]);\n\
         try { l[0].name = 'Other'; } catch (e) {} vc.log(l[0].name);\n\
         try { l[0].printerInfo; } catch (e) { vc.log(e.message); }",
    );
    assert_eq!(out[0], json!([256, "Printer 5", "Printer", "Printer 0", "Printer 255"]));
    assert_eq!(out[1], json!("Printer 5"), "name is read-only");
    assert_eq!(out[2], json!("Printer.printerInfo is not available yet"));
    // The next run has no list unless the frontend gives one again.
    let e = failure(&mut s, &base, "app.printerList;");
    assert!(e.contains("Application.printerList is not available here"), "{e}");
    s.script_printers = Some(printers());
    let e = failure(&mut s, &base, "#targetengine keep\nheld.name;");
    assert!(e.contains("Stale printer"), "{e}");
    s.script_printers = Some(printers());
    assert_eq!(logged(&mut s, &base, "vc.log(app.printerList[1].name);")[0], json!(ODD_NAME));
    // With no document open there is no list.
    let mut empty = Session::new();
    empty.attach_script_data(base.join("settings/Script Data"));
    empty.script_printers = Some(printers());
    let e = failure(&mut empty, &base, "app.printerList;");
    assert!(e.contains("Application.printerList needs an open document"), "{e}");
    let _ = std::fs::remove_dir_all(base);
}

/// `print` without a file prepares a print-ready PDF for the printer it names, or the default one,
/// and sends nothing: the session holds the jobs once the run's changes are kept, for the frontend
/// to take. A name is matched exactly, whatever characters it holds.
#[test]
fn print_to_a_printer_holds_the_job_until_the_run_ends() {
    let base = folder("prepared");
    let mut s = session(&base);
    s.script_printers = Some(printers());
    let name = serde_json::to_string(ODD_NAME).unwrap();
    let report = run(
        &mut s,
        &base,
        &format!(
            "var o = new PrintOptions(); o.printerName = {name}; o.jobOptions.copies = 2; o.jobOptions.collate = true;\n\
             app.activeDocument.print(o);\n\
             vc.run('shape.rectangle', {{x: 150, y: 10, width: 20, height: 20}});\n\
             app.activeDocument.print();"
        ),
    )
    .unwrap();
    assert_eq!(report["prints"], 2, "{report}");
    assert!(s.script_prints_waiting());
    let title = s.active().unwrap().title();
    let jobs = s.take_script_prints();
    assert!(!s.script_prints_waiting());
    assert_eq!(jobs.len(), 2);
    assert_eq!((jobs[0].printer.as_str(), jobs[0].sheets, job_pages(&jobs[0])), (ODD_NAME, 4, 4), "two artboards, two copies");
    assert_eq!((jobs[1].printer.as_str(), jobs[1].sheets, job_pages(&jobs[1])), ("Office", 2, 2), "the default printer");
    assert!(jobs.iter().all(|j| j.title == title && j.pdf.starts_with(b"%PDF")));
    // Each job holds the document as it was when print was called: one copy of the second job has
    // the rectangle the script added after the first job, and the first job's two copies do not.
    let paths = |job: &PreparedPrint| {
        let mut n = 0;
        vectorcraft_pdf::import(&job.pdf).unwrap().walk(|c| n += usize::from(matches!(c.kind, vectorcraft_doc::NodeKind::Path { .. })));
        n
    };
    assert_eq!(paths(&jobs[0]) % 2, 0, "two equal copies");
    assert_eq!(paths(&jobs[0]) / 2 + 1, paths(&jobs[1]), "the second job has the added rectangle");
    assert!(s.take_script_prints().is_empty(), "a job is taken once");
    let _ = std::fs::remove_dir_all(base);
}

/// A run that throws, runs out of fuel or ends at a failed command keeps no job.
#[test]
fn a_run_that_fails_keeps_no_print_job() {
    let base = folder("rollback");
    let mut s = session(&base);
    let objects = s.active().unwrap().doc.layers[0].children().map_or(0, Vec::len);
    for case in [
        "app.activeDocument.print(); throw new Error('late');",
        "app.activeDocument.print(); vc.run('no.such.command', {});",
        "app.activeDocument.print(); app.activeDocument.print(); app.activeDocument.print(); app.activeDocument.print(); app.activeDocument.print();",
    ] {
        s.script_printers = Some(printers());
        failure(&mut s, &base, case);
        assert!(!s.script_prints_waiting(), "{case}");
    }
    s.script_printers = Some(printers());
    let stopped = s.execute("script.run", &json!({"source": "app.activeDocument.print(); while (true) {}", "name": "t.js", "maxFuel": 2_000_000}));
    assert!(stopped.is_err());
    assert!(s.take_script_prints().is_empty());
    assert_eq!(s.active().unwrap().doc.layers[0].children().map_or(0, Vec::len), objects);
    let _ = std::fs::remove_dir_all(base);
}

/// A name that is not exactly a listed printer's, a list with no default, PostScript, more jobs or
/// sheets than a run may prepare and jobs an earlier run left waiting all throw, and nothing is
/// prepared; a file wins over a printer name.
#[test]
fn print_to_a_printer_refuses_what_it_cannot_send() {
    let base = folder("refuse");
    let mut s = session(&base);
    let dir = data_folder(&mut s, &base);
    let cases = [
        ("o.printerName = 'office';", "PrintOptions.printerName names no installed printer: office"),
        ("o.printerName = 'Offic';", "names no installed printer: Offic"),
        ("o.printerName = ' Office';", "names no installed printer:  Office"),
        ("o.printerName = 'Office\\n';", "names no installed printer"),
        ("o.postScriptOptions.postScriptLevel = PrinterPostScriptLevelEnum.PSLEVEL2;", "PostScript prints only to a file"),
        ("o.jobOptions.copies = 51;", "at most 100 sheets in a run; this job has 102"),
        ("for (var i = 0; i < 4; i++) app.activeDocument.print(o);", "at most 4 jobs to a printer in a run"),
        ("o.printPreset = 'Nope';", "printPreset names no print preset"),
    ];
    for (case, part) in cases {
        s.script_printers = Some(printers());
        let e = failure(&mut s, &base, &format!("var o = new PrintOptions(); {case} app.activeDocument.print(o);"));
        assert!(e.contains(part), "{case}: {e}");
        assert!(!s.script_prints_waiting(), "{case}");
    }
    // A job past the sheet limit is refused before it is composed.
    s.script_printers = Some(printers());
    failure(&mut s, &base, "var o = new PrintOptions(); o.jobOptions.copies = 999; app.activeDocument.print(o);");
    let report = s.script_report().unwrap();
    let ran: Vec<&str> = report["slowestCommands"].as_array().unwrap().iter().filter_map(|c| c["id"].as_str()).collect();
    assert_eq!(ran, ["file.print"], "{report}");
    s.script_printers = Some(Printers::new([("Office".to_string(), false)], false));
    let e = failure(&mut s, &base, "app.activeDocument.print();");
    assert!(e.contains("There is no default printer: set PrintOptions.printerName"), "{e}");
    // Jobs an earlier run left waiting: the next run cannot print to a printer until they are taken.
    s.script_printers = Some(printers());
    logged(&mut s, &base, "app.activeDocument.print();");
    s.script_printers = Some(printers());
    let e = failure(&mut s, &base, "app.activeDocument.print();");
    assert!(e.contains("A print job from an earlier script still waits for approval"), "{e}");
    assert_eq!(s.take_script_prints().len(), 1, "the earlier job is kept");
    s.script_printers = Some(printers());
    logged(&mut s, &base, "app.activeDocument.print();");
    assert_eq!(s.take_script_prints().len(), 1);
    // A file wins over the printer name.
    s.script_printers = Some(printers());
    let file = quoted(&dir.join("f.pdf"));
    logged(
        &mut s,
        &base,
        &format!(
            "Folder.userData; var o = new PrintOptions(); o.printerName = 'Nowhere'; o.jobOptions.file = new File({file}); app.activeDocument.print(o);"
        ),
    );
    assert!(dir.join("f.pdf").exists());
    assert!(!s.script_prints_waiting());
    let _ = std::fs::remove_dir_all(base);
}

/// The pages of a job for a printer are counted from the art as it prints, artboard backgrounds
/// included, as `file.print` lays it out: an artboard that holds only its background prints with
/// `skipBlank`, and counts toward the sheet limit.
#[test]
fn print_to_a_printer_counts_the_pages_of_the_art_as_it_prints() {
    let base = folder("backgrounds");
    let mut s = session(&base);
    s.execute("artboard.setProps", &json!({"index": 1, "color": "#ff0000"})).unwrap();
    s.execute("print.setup", &json!({"settings": {"skipBlank": true, "artboards": "range", "range": "2"}})).unwrap();
    s.script_printers = Some(printers());
    logged(&mut s, &base, "app.activeDocument.print();");
    let jobs = s.take_script_prints();
    assert_eq!(jobs.iter().map(|j| (j.sheets, job_pages(j))).collect::<Vec<_>>(), [(1, 1)], "the background prints");
    s.execute("print.setup", &json!({"settings": {"artboards": "all"}})).unwrap();
    s.script_printers = Some(printers());
    let e = failure(&mut s, &base, "var o = new PrintOptions(); o.jobOptions.copies = 51; app.activeDocument.print(o);");
    assert!(e.contains("this job has 102"), "both artboards print: {e}");
    assert!(!s.script_prints_waiting());
    let _ = std::fs::remove_dir_all(base);
}

/// `file.print`'s `maxPages` composes nothing for a longer job, and only after the other params are
/// checked.
#[test]
fn file_print_max_pages_composes_only_what_fits() {
    let base = folder("max-pages");
    let mut s = session(&base);
    let r = s.execute("file.print", &json!({"maxPages": 5, "settings": {"copies": 3}})).unwrap();
    assert_eq!((&r["pages"], r.get("dataBase64").is_none()), (&json!(6), true), "{r}");
    let r = s.execute("file.print", &json!({"maxPages": 6, "settings": {"copies": 3}})).unwrap();
    assert_eq!((&r["pages"], r["dataBase64"].is_string()), (&json!(6), true));
    for bad in [
        json!({"maxPages": 0, "format": "postscript", "level": 1}),
        json!({"maxPages": 0, "settings": {"advanced": {"flattenerPreset": "nope"}}}),
        json!({"maxPages": -1}),
        json!({"maxPages": "2"}),
        json!({"maxPages": null}),
    ] {
        assert!(s.execute("file.print", &bad).is_err(), "{bad}");
    }
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
