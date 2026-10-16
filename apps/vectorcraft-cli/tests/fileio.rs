//! The CLI opens every readable format (PDF, .ai, images) through the engine's `document.open`.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::Command;

use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_vectorcraft-cli");

fn tmp(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("vectorcraft-cli-fileio-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
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
