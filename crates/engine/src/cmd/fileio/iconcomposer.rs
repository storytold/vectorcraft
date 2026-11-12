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

/// Most layer files one export writes: a bound on the work one request can ask for.
const MAX_FILES: usize = 10_000;

/// Most bytes of layer files one export holds in memory when it returns them (in the reply or as
/// one ZIP) rather than writing them to a folder, where each file is written as it is made.
const MAX_HELD_BYTES: usize = 256 << 20;

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

/// One file the export makes: its path inside the export's folder and its bytes.
struct LayerFile {
    path: String,
    bytes: Vec<u8>,
}

/// Where each layer file goes as soon as it is encoded.
enum Sink<'a> {
    /// Written into a new folder in `parent`, made at the first file, with each artboard's
    /// sub-folder made at its first file. An export that fails removes what it wrote
    /// ([`Sink::abandon`]).
    Folder { parent: &'a str, name: &'a str, folder: Option<String>, subs: Vec<String>, written: Vec<String> },
    /// Held for the reply, at most [`MAX_HELD_BYTES`] in all.
    Held { files: Vec<LayerFile>, bytes: usize },
}

impl Sink<'_> {
    fn put(&mut self, sub: Option<&str>, file: LayerFile) -> Result<()> {
        match self {
            Sink::Folder { parent, name, folder, subs, written } => {
                if folder.is_none() {
                    create_dir(parent)?;
                    *folder = Some(new_folder(parent, name)?);
                }
                let Some(folder) = folder.as_deref() else { return Err(bad(C, "the export folder could not be made")) };
                if let Some(sub) = sub.filter(|s| !subs.iter().any(|d| d == s)) {
                    create_dir(&format!("{folder}/{sub}"))?;
                    subs.push(sub.to_string());
                }
                let path = format!("{folder}/{}", file.path);
                write_file(&path, &file.bytes)?;
                written.push(path);
            }
            Sink::Held { files, bytes } => {
                *bytes = bytes.saturating_add(file.bytes.len());
                if *bytes > MAX_HELD_BYTES {
                    return Err(bad(C, format!("the layer files come to over {} MB: export them to a folder", MAX_HELD_BYTES >> 20)));
                }
                files.push(file);
            }
        }
        Ok(())
    }

    /// Undo a failed export: remove the files it wrote, then its sub-folders and folder when they
    /// are empty. Only what this export made is touched.
    fn abandon(&self) {
        if let Sink::Folder { folder: Some(folder), subs, written, .. } = self {
            for path in written.iter().rev() {
                super::remove_made(path);
            }
            for sub in subs.iter().rev() {
                super::remove_made(&format!("{folder}/{sub}"));
            }
            super::remove_made(folder);
        }
    }
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
    let mut boards = o.boards.resolve(n).map_err(|e| bad(C, e))?.unwrap_or_else(|| (0..n).collect());
    // An artboard picked twice is exported once, where it was first picked: twice would give two
    // files of one name.
    let mut seen = std::collections::HashSet::new();
    boards.retain(|b| seen.insert(*b));
    if boards.is_empty() {
        return Err(bad(C, "the document has no artboard"));
    }
    // A sub-folder per artboard when the document has several, as Icon Composer takes one icon's
    // layers at a time. Names that differ only in case are one folder on some file systems.
    let subfolders: Option<Vec<String>> = (n > 1).then(|| {
        let names = boards.iter().map(|b| safe_name(doc.artboards.get(*b).map_or("", |a| a.name.as_str()), &format!("Artboard {}", b + 1)));
        distinct_names_by(names, str::to_lowercase)
    });
    let folder = o.folder.as_deref().map(str::trim).filter(|f| !f.is_empty());
    let mut sink = match folder {
        Some(parent) => Sink::Folder { parent, name: &name, folder: None, subs: vec![], written: vec![] },
        None => Sink::Held { files: vec![], bytes: 0 },
    };
    let warnings = match layer_files(doc, &boards, subfolders.as_deref(), format, &mut sink) {
        Ok(w) => w,
        Err(e) => {
            sink.abandon();
            return Err(e);
        }
    };
    match sink {
        Sink::Folder { folder, written, .. } => Ok(json!({ "folder": folder, "files": written, "warnings": warnings })),
        Sink::Held { files, .. } if o.zip => {
            let names: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
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
        Sink::Held { files, .. } => {
            let files: Vec<Value> =
                files.iter().map(|f| json!({ "name": f.path, "dataBase64": vectorcraft_format::base64_encode(&f.bytes) })).collect();
            Ok(json!({ "folder": name, "files": files, "warnings": warnings }))
        }
    }
}

/// Encode the layer files of every artboard in `boards` (sub-folder `subfolders[i]` for the `i`th),
/// each into `sink` as it is made → the encoders' warnings, each once.
fn layer_files(doc: &Document, boards: &[usize], subfolders: Option<&[String]>, format: &str, sink: &mut Sink) -> Result<Vec<String>> {
    // No background color; PNG at one pixel per point (72 ppi), not the document's raster effects
    // resolution, so a 1024 pt artboard gives the 1024 px canvas Icon Composer works on.
    let options = if format == "png" { json!({ "includeArtboardFill": false, "ppi": 72 }) } else { json!({ "includeArtboardFill": false }) };
    let mut count = 0usize;
    let mut warnings: Vec<String> = vec![];
    for (i, b) in boards.iter().enumerate() {
        let Some(artboard) = doc.artboards.get(*b) else { continue };
        let sub = subfolders.and_then(|s| s.get(i));
        let mut number = 0usize;
        // Bottom first: the back layer is 1.
        for layer in doc.layers.iter().filter(|l| l.is_layer() && l.visible && !l.is_template()) {
            vectorcraft_doc::cancel::check()?;
            let Some(d) = layer_doc(doc, layer, artboard.rect, &artboard.name) else { continue };
            if count >= MAX_FILES {
                return Err(bad(C, format!("more than {MAX_FILES} layer files: export fewer artboards at a time")));
            }
            number += 1;
            count += 1;
            let (bytes, notes) = encode_with_warnings(&d, format, &options)?;
            for w in notes {
                if !warnings.contains(&w) {
                    warnings.push(w);
                }
            }
            let file = format!("{number}.{}.{format}", safe_name(&layer.display_name(), "Layer")).to_lowercase();
            let path = sub.map_or_else(|| file.clone(), |s| format!("{s}/{file}"));
            sink.put(sub.map(String::as_str), LayerFile { path, bytes })?;
        }
    }
    if count == 0 {
        return Err(bad(C, "nothing to export: no visible layer has art on the chosen artboards"));
    }
    Ok(warnings)
}

/// `doc` with `layer` alone and one artboard, `rect` (named `name`), holding only the layer's
/// objects whose drawing touches it; `None` when none of its visible art overlaps `rect`.
fn layer_doc(doc: &Document, layer: &Arc<Node>, rect: Rect, name: &str) -> Option<Document> {
    let mut d = doc.clone();
    d.layers = vec![layer.clone()];
    let mut d = with_single_artboard(d, rect, name);
    let mut kept = d.layers.clone();
    {
        let symbols = d.symbol_boxes();
        for l in &mut kept {
            keep_drawn_on(&symbols, l, rect);
        }
    }
    d.layers = kept;
    d.last_view = None;
    d.layers_open = None;
    vectorcraft_render::encode::art_bounds_on(&d, rect)?;
    Some(d)
}

/// Keep in layer `n` (and its sublayers) only the objects whose drawing touches `r`, effects
/// included (a shadow cast onto the artboard by a shape beside it keeps the shape), and those
/// that can't be measured (type in a missing font): dropping them could lose art.
fn keep_drawn_on(symbols: &vectorcraft_doc::SymbolBoxes, n: &mut Arc<Node>, r: Rect) {
    let touches = |b: Rect| b.x0 <= r.x1 && b.x1 >= r.x0 && b.y0 <= r.y1 && b.y1 >= r.y0;
    if let Some(children) = Arc::make_mut(n).children_mut() {
        children.retain(|c| c.is_layer() || vectorcraft_render::encode::drawn_bounds(symbols, c).or_else(|| c.visual_bounds()).is_none_or(touches));
        for c in children.iter_mut().filter(|c| c.is_layer()) {
            keep_drawn_on(symbols, c, r);
        }
    }
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::super::tests_iconcomposer::TempFolder;
    use super::*;

    #[test]
    fn a_failed_export_removes_what_it_wrote_and_nothing_else() {
        let dir = TempFolder::new("abandon");
        let parent = dir.0.to_string_lossy().to_string();
        let other = dir.0.join("other.txt");
        std::fs::write(&other, b"keep").unwrap();
        let mut sink = Sink::Folder { parent: &parent, name: "Icon", folder: None, subs: vec![], written: vec![] };
        sink.put(Some("A"), LayerFile { path: "A/1.back.svg".into(), bytes: b"<svg/>".to_vec() }).unwrap();
        sink.put(Some("B"), LayerFile { path: "B/1.back.svg".into(), bytes: b"<svg/>".to_vec() }).unwrap();
        assert!(dir.0.join("Icon/B/1.back.svg").is_file());
        sink.abandon();
        let left: Vec<_> = std::fs::read_dir(&dir.0).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, ["other.txt"], "the export's folder is gone; the file beside it stays");
    }
}
