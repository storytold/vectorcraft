//! File › Export › Export for Icon Composer… (`document.exportForIconComposer`): each visible
//! top-level layer of each chosen artboard as a file of its own, SVG or PNG, all the size of the
//! artboard, ready to bring into Icon Composer as the layers of an app icon.
//!
//! Icon Composer builds an icon from layers given as separate SVG or PNG files on one canvas
//! (<https://developer.apple.com/icon-composer/>).
//! This export gives it that: every file keeps the artboard's bounds, so the layers line up.
//!
//! The files go in a folder named after the document; when that folder exists already, a new one is
//! made with the date and time after the name (`Icon 2026-11-12 09.30.00`, UTC). A document of
//! several artboards gets a sub-folder per chosen artboard, named after it. Each file is named
//! `<n>.<layer name>.<svg|png>` in lower case, `n` counting the exported layers of that artboard
//! from the back (1) to the front, so the files sort in stacking order. A layer is left out when it
//! is hidden, a template, or has no visible art that overlaps the artboard; the objects of a layer
//! that lie wholly off the artboard are left out of its file.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node};
use vectorcraft_geom::Rect;

use super::super::{Result, Session, bad};
use super::encode::with_single_artboard;
use super::screens::safe_file_name;
use super::{ArtboardPick, create_dir, distinct_names_by, encode_with_warnings, file_stem, write_file};

pub(super) const C: &str = "document.exportForIconComposer";

/// Most layer files one export writes: a bound on the work and memory one request can ask for.
const MAX_FILES: usize = 10_000;

/// The params of `document.exportForIconComposer`.
#[derive(Default, Deserialize)]
#[serde(default)]
struct Params {
    #[serde(flatten)]
    boards: ArtboardPick,
    folder: Option<String>,
    format: Option<String>,
    zip: bool,
}

/// One file the export writes: its path inside the export's folder and its bytes.
struct LayerFile {
    path: String,
    bytes: Vec<u8>,
}

/// `document.exportForIconComposer`.
pub(super) fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let o: Params = if p.is_object() { Params::deserialize(p).map_err(|e| bad(C, e.to_string()))? } else { Params::default() };
    let format = match o.format.as_deref().map(str::to_ascii_lowercase).as_deref() {
        None | Some("svg") => "svg",
        Some("png") => "png",
        Some(f) => return Err(bad(C, format!("format `{f}`: svg or png"))),
    };
    let st = s.doc()?;
    let doc = &st.doc;
    let name = folder_name(st.path.as_deref().unwrap_or(&doc.title));
    let n = doc.artboards.len();
    let boards = o.boards.resolve(n).map_err(|e| bad(C, e))?.unwrap_or_else(|| (0..n).collect());
    if boards.is_empty() {
        return Err(bad(C, "the document has no artboard"));
    }
    // A sub-folder per artboard when the document has several, as Icon Composer takes one icon's
    // layers at a time. Names that differ only in case are one folder on some file systems.
    let subfolders: Option<Vec<String>> = (n > 1).then(|| {
        let names = boards.iter().map(|b| safe_name(doc.artboards.get(*b).map_or("", |a| a.name.as_str()), &format!("Artboard {}", b + 1)));
        distinct_names_by(names, str::to_lowercase)
    });
    let (files, warnings) = layer_files(doc, &boards, subfolders.as_deref(), format)?;
    let names: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    match o.folder.as_deref().map(str::trim).filter(|f| !f.is_empty()) {
        Some(parent) => {
            create_dir(parent)?;
            let folder = new_folder(parent, &name)?;
            for sub in subfolders.iter().flatten() {
                create_dir(&format!("{folder}/{sub}"))?;
            }
            let mut written = vec![];
            for f in &files {
                let path = format!("{folder}/{}", f.path);
                write_file(&path, &f.bytes)?;
                written.push(path);
            }
            Ok(json!({ "folder": folder, "files": written, "warnings": warnings }))
        }
        None if o.zip => {
            let entries: Vec<(String, &[u8])> = files.iter().map(|f| (format!("{name}/{}", f.path), f.bytes.as_slice())).collect();
            let zip = super::zip::deflate(&entries, super::zip::FIXED_TIME, vectorcraft_doc::cancel::is_cancelled);
            vectorcraft_doc::cancel::check()?;
            let zip = zip.map_err(|e| bad(C, e))?;
            Ok(json!({
                "name": format!("{name}.zip"),
                "dataBase64": vectorcraft_format::base64_encode(&zip),
                "bytes": zip.len(),
                "files": names,
                "warnings": warnings,
            }))
        }
        None => {
            let files: Vec<Value> =
                files.iter().map(|f| json!({ "name": f.path, "dataBase64": vectorcraft_format::base64_encode(&f.bytes) })).collect();
            Ok(json!({ "folder": name, "files": files, "warnings": warnings }))
        }
    }
}

/// Encode the layer files of every artboard in `boards` (sub-folder `subfolders[i]` for the `i`th)
/// → the files and the encoders' warnings, each once.
fn layer_files(doc: &Document, boards: &[usize], subfolders: Option<&[String]>, format: &str) -> Result<(Vec<LayerFile>, Vec<String>)> {
    // No background color; PNG at one pixel per point (72 ppi), not the document's raster effects
    // resolution, so a 1024 pt artboard gives the 1024 px canvas Icon Composer works on.
    let options = if format == "png" { json!({ "includeArtboardFill": false, "ppi": 72 }) } else { json!({ "includeArtboardFill": false }) };
    let mut files = vec![];
    let mut warnings: Vec<String> = vec![];
    for (i, b) in boards.iter().enumerate() {
        let Some(artboard) = doc.artboards.get(*b) else { continue };
        let sub = subfolders.and_then(|s| s.get(i));
        let mut number = 0usize;
        // Bottom first: the back layer is 1.
        for layer in doc.layers.iter().filter(|l| l.is_layer() && l.visible && !l.is_template()) {
            vectorcraft_doc::cancel::check()?;
            let Some(d) = layer_doc(doc, layer, artboard.rect, &artboard.name) else { continue };
            if files.len() >= MAX_FILES {
                return Err(bad(C, format!("more than {MAX_FILES} layer files: export fewer artboards at a time")));
            }
            number += 1;
            let (bytes, notes) = encode_with_warnings(&d, format, &options)?;
            for w in notes {
                if !warnings.contains(&w) {
                    warnings.push(w);
                }
            }
            let file = format!("{number}.{}.{format}", safe_name(&layer.display_name(), "Layer")).to_lowercase();
            let path = sub.map_or_else(|| file.clone(), |s| format!("{s}/{file}"));
            files.push(LayerFile { path, bytes });
        }
    }
    if files.is_empty() {
        return Err(bad(C, "nothing to export: no visible layer has art on the chosen artboards"));
    }
    Ok((files, warnings))
}

/// `doc` with `layer` alone and one artboard, `rect` (named `name`), holding only the layer's
/// objects that touch it; `None` when none of its visible art overlaps `rect`.
fn layer_doc(doc: &Document, layer: &Arc<Node>, rect: Rect, name: &str) -> Option<Document> {
    let mut d = doc.clone();
    d.layers = vec![layer.clone()];
    let mut d = with_single_artboard(d, rect, name);
    super::save::keep_art_on(&mut d, rect);
    vectorcraft_render::encode::art_bounds_on(&d, rect)?;
    Some(d)
}

/// The export folder's name: the document's file name without its extension, else its title.
fn folder_name(title: &str) -> String {
    safe_name(&file_stem(title), "Untitled")
}

/// `name` safe as one file or folder name on every platform ([`safe_file_name`]), without the
/// leading periods that hide a file on macOS and Linux; `fallback` when nothing is left.
fn safe_name(name: &str, fallback: &str) -> String {
    let n = safe_file_name(name.trim().trim_start_matches('.').trim_start());
    if n.is_empty() { fallback.to_string() } else { n }
}

/// Make a new folder named `name` in `parent` → its path. When `name` is taken, the folder is named
/// with the date and time after it, and `-2`, `-3`… after that when that is taken too.
fn new_folder(parent: &str, name: &str) -> Result<String> {
    let first = format!("{parent}/{name}");
    if super::create_new_dir(&first)? {
        return Ok(first);
    }
    let [y, mo, d, h, mi, s] = vectorcraft_doc::metadata::output_time().map_or([1970, 1, 1, 0, 0, 0], vectorcraft_doc::metadata::civil);
    let stamped = format!("{name} {y:04}-{mo:02}-{d:02} {h:02}.{mi:02}.{s:02}");
    for i in 1..=100u32 {
        let path = if i == 1 { format!("{parent}/{stamped}") } else { format!("{parent}/{stamped}-{i}") };
        if super::create_new_dir(&path)? {
            return Ok(path);
        }
    }
    Err(bad(C, format!("{parent}: could not make a new folder for `{name}`")))
}
