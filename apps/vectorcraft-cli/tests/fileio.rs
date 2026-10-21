//! The CLI opens every readable format (PDF, .ai, images) through the engine's `document.open`.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::Command;

use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_vectorcraft-cli");

/// Returns the path of `name` in `vectorcraft-cli-fileio-<pid>` in the temp folder. The first call
/// removes that folder and creates it again empty.
fn tmp(name: &str) -> String {
    static DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    let dir = DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("vectorcraft-cli-fileio-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    });
    dir.join(name).to_string_lossy().to_string()
}

fn ok(args: &[&str]) -> String {
    let out = Command::new(BIN).args(args).output().unwrap();
    assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

/// `run` a new `width`×`height` document with a rectangle and export it to `out`.
fn make(out: &str, width: u32, height: u32) {
    ok(&[
        "run",
        "--cmd",
        "file.new",
        "--params",
        &format!(r#"{{"width":{width},"height":{height},"artboards":2}}"#),
        "--cmd",
        "shape.rectangle",
        "--params",
        r#"{"x":1,"y":1,"width":2,"height":1}"#,
        "--export",
        out,
    ]);
}

fn info(file: &str) -> Value {
    serde_json::from_str(&ok(&["info", file])).unwrap()
}

#[test]
fn convert_pdf_to_svg_and_info_on_ai() {
    let pdf = tmp("in.pdf");
    make(&pdf, 120, 80);
    let svg = tmp("out.svg");
    ok(&["convert", &pdf, &svg]);
    assert!(std::fs::read_to_string(&svg).unwrap().contains("<svg"));

    let ai = tmp("in.ai");
    std::fs::copy(&pdf, &ai).unwrap();
    let v = info(&ai);
    assert_eq!(v["artboards"].as_array().unwrap().len(), 2, "{v}");
    assert_eq!(v["artboards"][0]["rect"][2], 120.0);
    // `run --in` takes it too.
    ok(&["run", "--in", &ai, "--cmd", "document.inspect"]);
    // A PDF keeps every artboard unless --artboard / --range names some.
    let all = tmp("all.pdf");
    ok(&["convert", &ai, &all]);
    assert_eq!(info(&all)["artboards"].as_array().unwrap().len(), 2);
    let second = tmp("second.pdf");
    ok(&["convert", &ai, &second, "--range", "2"]);
    assert_eq!(info(&second)["artboards"].as_array().unwrap().len(), 1);
}

#[test]
fn images_open_at_their_pixel_size() {
    let webp = tmp("tiny.webp");
    make(&webp, 3, 2);
    let v = info(&webp);
    assert_eq!(v["artboards"][0]["rect"], serde_json::json!([0.0, 0.0, 3.0, 2.0]), "{v}");
    assert_eq!(v["kinds"]["Image"], 1, "{v}");
    let png = tmp("tiny.png");
    ok(&["convert", &webp, &png, "--scale", "2"]);
    assert_eq!(&std::fs::read(&png).unwrap()[16..24], [0, 0, 0, 6, 0, 0, 0, 4], "IHDR 6×4");
}

#[test]
fn raster_exports_use_document_resolution_unless_overridden() {
    let source = tmp("resolution.vectorcraft");
    ok(&["run", "--cmd", "file.new", "--params", r#"{"width":72,"height":36,"rasterEffectsPpi":300}"#, "--export", &source]);
    for (flags, width, height) in [(vec![], 300u32, 150u32), (vec!["--scale", "1"], 72, 36), (vec!["--ppi", "150", "--scale", "2"], 150, 75)] {
        for mode in ["run", "convert"] {
            for format in ["png", "psd"] {
                let path = tmp(&format!("resolution-{mode}-{width}.{format}"));
                let mut args = if mode == "run" { vec!["run", "--in", &source, "--export", &path] } else { vec!["convert", &source, &path] };
                args.extend_from_slice(&flags);
                ok(&args);
                let bytes = std::fs::read(&path).unwrap();
                let dimensions = if format == "png" { &bytes[16..24] } else { &bytes[14..22] };
                let expected =
                    if format == "png" { [width.to_be_bytes(), height.to_be_bytes()] } else { [height.to_be_bytes(), width.to_be_bytes()] };
                assert_eq!(dimensions, expected.concat(), "{args:?}");
            }
        }
    }
}

#[test]
fn raster_export_rejects_invalid_ppi() {
    for ppi in ["0", "-150", "NaN", "inf", "high"] {
        for args in [vec!["run", "--export", "unused.png", "--ppi", ppi], vec!["convert", "unused.svg", "unused.png", "--ppi", ppi]] {
            let out = Command::new(BIN).args(&args).output().unwrap();
            assert!(!out.status.success(), "{args:?}");
            assert!(String::from_utf8_lossy(&out.stderr).contains("positive finite number"), "{args:?}: {out:?}");
        }
    }
}

#[test]
fn help_lists_readable_formats() {
    let out = ok(&["--help"]);
    assert!(out.contains("Readable formats: .vectorcraft") && out.contains(".ait") && out.contains(".webp"), "{out}");
}

#[test]
fn help_lists_perf_deep() {
    let out = ok(&["--help"]);
    assert!(
        out.contains("vectorcraft-cli perf [--paths N] [--deep]") && out.contains("--deep also times") && out.contains("with the peak so far on"),
        "{out}"
    );
}

/// `info` reports the import warnings: an EPS read only up to an error names it.
#[test]
fn info_reports_why_an_eps_read_partly() {
    let eps = tmp("partial.eps");
    std::fs::write(&eps, "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n0 0 10 10 rectfill frobnicate\n%%EOF\n").unwrap();
    let v = info(&eps);
    let w = v["warnings"][0].as_str().unwrap_or_default();
    assert!(w.contains("`frobnicate`"), "{v}");
    assert_eq!(v["kinds"]["Path"], 1, "{v}");
}

/// #1604: convert reports what reading the input left out (here a missing linked image) as
/// importWarnings, next to the export's own warnings.
#[test]
fn convert_reports_import_warnings() {
    let svg = tmp("missing-link.svg");
    std::fs::write(
        &svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40"><image width="40" height="40" href="nowhere-to-be-found.png"/></svg>"#,
    )
    .unwrap();
    let png = tmp("missing-link.png");
    let out: Value = serde_json::from_str(ok(&["convert", &svg, &png]).trim()).unwrap();
    let w = out["importWarnings"].as_array().unwrap_or_else(|| panic!("{out}"));
    assert!(w.iter().any(|w| w.as_str().unwrap().contains("nowhere-to-be-found.png")), "{out}");
}

#[test]
fn convert_writes_legacy_ai_with_its_options() {
    // Two artboards, a shown rectangle and a hidden one.
    let src = tmp("legacy.vectorcraft");
    ok(&[
        "run",
        "--cmd",
        "file.new",
        "--params",
        r#"{"width":120,"height":80,"artboards":2}"#,
        "--cmd",
        "shape.rectangle",
        "--params",
        r#"{"x":10,"y":10,"width":20,"height":10}"#,
        "--cmd",
        "shape.rectangle",
        "--params",
        r#"{"x":40,"y":10,"width":20,"height":10}"#,
        "--cmd",
        "object.hide",
        "--export",
        &src,
    ]);
    // One file per artboard, named after it; hidden objects in a hidden layer by default.
    let out = tmp("legacy.ai");
    let r: Value = serde_json::from_str(&ok(&["convert", &src, &out, "--ai-version", "7"])).unwrap();
    let files: Vec<&str> = r["files"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert_eq!(files.len(), 2, "{r}");
    assert!(files.iter().all(|f| std::fs::read(f).unwrap().starts_with(b"%!PS-Adobe-3.0")), "{r}");
    assert!(r["warnings"].to_string().contains("hidden layer"), "{r}");
    // One artboard is the file itself; the hidden rectangle left out.
    let one = tmp("legacy-one.ai");
    let r: Value = serde_json::from_str(&ok(&[
        "convert",
        &src,
        &one,
        "--ai-version",
        "7",
        "--range",
        "1",
        "--hidden-objects",
        "omit",
        "--transparent-images",
        "omit",
    ]))
    .unwrap();
    assert!(r["warnings"].to_string().contains("hidden objects are left out"), "{r}");
    assert!(std::fs::read(&one).unwrap().starts_with(b"%!PS-Adobe-3.0"));
    // A value the engine doesn't take is an error.
    let bad = Command::new(BIN).args(["convert", &src, &tmp("bad.ai"), "--ai-version", "7", "--hidden-objects", "behind"]).output().unwrap();
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("hiddenObjects"), "{}", String::from_utf8_lossy(&bad.stderr));
}

/// convert and `run --export` write a `.ait` as Save as Template does: a PDF-compatible template of
/// every artboard that opens as a new untitled document.
#[test]
fn convert_and_run_write_ait_templates() {
    let pdf = tmp("template-in.pdf");
    make(&pdf, 120, 80);
    let (converted, exported) = (tmp("converted.ait"), tmp("exported.ait"));
    let r: Value = serde_json::from_str(&ok(&["convert", &pdf, &converted])).unwrap();
    assert_eq!((&r["format"], &r["path"]), (&Value::from("ait"), &Value::from(converted.as_str())), "{r}");
    let r: Value = serde_json::from_str(ok(&["run", "--in", &pdf, "--export", &exported]).lines().nth(1).unwrap()).unwrap();
    assert_eq!(r["result"]["format"], "ait", "{r}");
    for ait in [&converted, &exported] {
        assert!(std::fs::read(ait).unwrap().starts_with(b"%PDF"), "{ait}");
        let opened: Value = serde_json::from_str(ok(&["run", "--in", ait]).lines().next().unwrap()).unwrap();
        let opened = &opened["result"];
        assert_eq!((&opened["format"], &opened["restored"]), (&Value::from("ait"), &Value::from(true)), "{opened}");
        assert!(opened["title"].as_str().is_some_and(|t| t.starts_with("Untitled-")), "{opened}");
        assert_eq!(info(ait)["artboards"].as_array().unwrap().len(), 2, "{ait}");
    }
    // A template holds every artboard: one can't be picked.
    let one = Command::new(BIN).args(["convert", &pdf, &tmp("one.ait"), "--artboard", "0"]).output().unwrap();
    assert!(!one.status.success());
    assert!(String::from_utf8_lossy(&one.stderr).contains("every artboard"), "{}", String::from_utf8_lossy(&one.stderr));
}
