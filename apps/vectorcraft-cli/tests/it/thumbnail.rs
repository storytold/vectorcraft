//! `thumbnail` and the Linux `.thumbnailer` entry that runs it for file managers.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const THUMBNAILER: &str = include_str!("../../../../packaging/linux/ai.storyteller.vectorcraft.thumbnailer");
const MIME: &str = include_str!("../../../../packaging/linux/ai.storyteller.vectorcraft.mime.xml");
const PACKAGE: &str = include_str!("../../../../packaging/linux/package.sh");

/// Returns a new empty folder in the temp folder, removed with everything in it when the guard drops.
fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn cli(args: &[&str]) -> Output {
    Command::new(crate::bin()).args(args).output().unwrap()
}

/// Saves a `width`×`height` document with a rectangle on its artboard as `name` in `dir` and returns its path.
fn document(dir: &Path, name: &str, width: u32, height: u32) -> PathBuf {
    save(dir, name, width, height, &["--cmd", "shape.rectangle", "--params", r#"{"x":10,"y":10,"width":40,"height":20}"#])
}

/// Saves a new `width`×`height` document as `name` in `dir` after the `run` arguments `steps`.
fn save(dir: &Path, name: &str, width: u32, height: u32, steps: &[&str]) -> PathBuf {
    let path = dir.join(name);
    let size = format!(r#"{{"width":{width},"height":{height}}}"#);
    let mut args = vec!["run", "--cmd", "file.new", "--params", &size];
    args.extend_from_slice(steps);
    args.extend(["--export", path.to_str().unwrap()]);
    let out = cli(&args);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    path
}

/// The width and height in a PNG's IHDR chunk, after checking that it is the 8-bit, non-interlaced
/// RGBA a thumbnail must be (<https://specifications.freedesktop.org/thumbnail-spec/latest/creation.html>).
fn png_size(png: &[u8]) -> (u32, u32) {
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n") && &png[12..16] == b"IHDR", "not a PNG");
    assert_eq!((png[24], png[25], png[28]), (8, 6, 0), "bit depth, color type (RGBA) and interlacing");
    let be = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    (be(&png[16..20]), be(&png[20..24]))
}

/// The value of `key` in the entry's `[Thumbnailer Entry]` group.
fn entry(key: &str) -> &'static str {
    assert_eq!(THUMBNAILER.lines().next(), Some("[Thumbnailer Entry]"));
    THUMBNAILER.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('=')).unwrap_or_else(|| panic!("no {key} key"))
}

/// The file manager's command for the entry: `Exec` split at spaces, `%s` the size, `%i` the
/// input path and `%o` the output path (the codes the thumbnail factories expand).
fn exec(size: u32, input: &Path, output: &Path) -> Vec<String> {
    entry("Exec")
        .split(' ')
        .map(|arg| match arg {
            "%s" => size.to_string(),
            "%i" => input.to_string_lossy().into_owned(),
            "%o" => output.to_string_lossy().into_owned(),
            other => {
                assert!(!other.contains('%'), "a field code the CLI doesn't take: {other}");
                other.to_string()
            }
        })
        .collect()
}

/// The entry runs the CLI with the size, the document and the PNG to write, as a file manager
/// does: with no environment, and the PNG is the first artboard at that size, drawn by the
/// renderer that embeds a native file's preview.
#[test]
fn the_thumbnailer_entry_writes_the_first_artboard() {
    let dir = tmp();
    let doc = document(dir.path(), "wide.vectorcraft", 200, 100);
    let out = dir.path().join("wide.png");
    let command = exec(128, &doc, &out);
    assert_eq!(command[0], entry("TryExec"));
    assert_eq!(command[0], "vectorcraft-cli");
    let run = Command::new(crate::bin()).args(&command[1..]).env_clear().output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let png = std::fs::read(&out).unwrap();
    assert_eq!(png_size(&png), (128, 64));
    let saved = vectorcraft_format::load(&std::fs::read(&doc).unwrap()).unwrap();
    assert_eq!(Some(png), vectorcraft_render::encode::first_artboard_png(&saved, 128).unwrap(), "the renderer's picture");
}

/// Each type the entry lists is a native document type the app's MIME package declares, and the
/// packages install the entry where the thumbnail factories look.
#[test]
fn the_thumbnailer_entry_covers_the_native_types() {
    let types: Vec<&str> = entry("MimeType").split(';').filter(|t| !t.is_empty()).collect();
    assert_eq!(types, ["application/x-vectorcraft", "application/x-vectorcraft-template"]);
    for t in types {
        assert!(MIME.contains(&format!("<mime-type type=\"{t}\">")), "the MIME package doesn't declare {t}");
    }
    assert!(
        PACKAGE.contains(r#"install -Dm644 "$HERE/$APP_ID.thumbnailer" "$STAGE/usr/share/thumbnailers/$APP_ID.thumbnailer""#),
        "package.sh doesn't install the entry"
    );
}

/// The size defaults to 256 pixels on the longer side, and a tall artboard is fitted by its height.
#[test]
fn thumbnail_fits_the_longer_side() {
    let dir = tmp();
    let doc = document(dir.path(), "tall.vectorcraft", 100, 400);
    let out = dir.path().join("tall.png");
    let run = cli(&["thumbnail", doc.to_str().unwrap(), out.to_str().unwrap()]);
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert_eq!(png_size(&std::fs::read(&out).unwrap()), (64, 256));
}

/// A size outside 1 to 1024 pixels, a missing file name and an unknown option are errors, and no
/// PNG is written.
#[test]
fn thumbnail_refuses_bad_arguments() {
    let dir = tmp();
    let doc = document(dir.path(), "args.vectorcraft", 100, 100);
    let out = dir.path().join("args.png");
    let (d, o) = (doc.to_str().unwrap(), out.to_str().unwrap());
    for (args, says) in [
        (vec!["thumbnail", "--size", "0", d, o], "--size"),
        (vec!["thumbnail", "--size", "1025", d, o], "--size"),
        (vec!["thumbnail", "--size", "big", d, o], "--size"),
        (vec!["thumbnail", "--size"], "--size"),
        (vec!["thumbnail", d], "IN and OUT"),
        (vec!["thumbnail", "--quality", "9", d, o], "--quality"),
    ] {
        let run = cli(&args);
        assert!(!run.status.success(), "{args:?}");
        assert!(String::from_utf8_lossy(&run.stderr).contains(says), "{args:?}: {}", String::from_utf8_lossy(&run.stderr));
        assert!(!out.exists(), "{args:?} wrote a PNG");
    }
}

/// A file that isn't a document, a damaged one, one larger than the CLI opens, a missing one and
/// an empty document get an error and no PNG, so the file manager shows the file's icon.
#[test]
fn thumbnail_writes_nothing_for_files_it_cannot_read() {
    let dir = tmp();
    let text = dir.path().join("notes.vectorcraft");
    std::fs::write(&text, b"not a document").unwrap();
    let saved = std::fs::read(document(dir.path(), "whole.vectorcraft", 100, 100)).unwrap();
    let damaged = dir.path().join("damaged.vectorcraft");
    std::fs::write(&damaged, &saved[..saved.len() / 2]).unwrap();
    // Sparse where the file system allows it: its size is checked before anything is read.
    let large = dir.path().join("large.vectorcraft");
    std::fs::File::create(&large).unwrap().set_len((256 << 20) + 1).unwrap();
    let missing = dir.path().join("missing.vectorcraft");
    let empty = save(dir.path(), "empty.vectorcraft", 100, 100, &[]);
    for (input, says) in
        [(&text, None), (&damaged, None), (&large, Some("larger than 256 MiB")), (&missing, None), (&empty, Some("the document shows nothing"))]
    {
        let out = dir.path().join("unreadable.png");
        let run = cli(&["thumbnail", input.to_str().unwrap(), out.to_str().unwrap()]);
        assert!(!run.status.success(), "{input:?}");
        let err = String::from_utf8_lossy(&run.stderr);
        assert!(err.contains(input.to_str().unwrap()), "{input:?}: {err}");
        assert!(says.is_none_or(|s| err.contains(s)), "{input:?}: {err}");
        assert!(!out.exists(), "{input:?} wrote a PNG");
    }
}
