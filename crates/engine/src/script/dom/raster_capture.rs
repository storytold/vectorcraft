//! Rasterizing art and capturing it as an image in the script object model: `Document.rasterize`,
//! `Document.imageCapture` and their option classes, as the help pages
//! <https://ai-scripting.docsforadobe.dev/jsobjref/Document/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/RasterizeOptions/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/ImageCaptureOptions/> and
//! <https://ai-scripting.docsforadobe.dev/jsobjref/scripting-constants/> describe.
//!
//! `rasterize` runs `object.rasterize` with the source art's ids, and `imageCapture` runs
//! `document.export` of an area of the page, inside the run's transaction. Every option is given to
//! the command, at the help page's default when the script leaves it out, so the document's raster
//! effects settings do not change the result.
//!
//! VectorCraft's own rules, which the help pages do not state:
//! - `clipBounds` is `[left, top, right, bottom]` in the run's script coordinates, as
//!   `geometricBounds` gives them; the right edge must be right of the left one and the bottom edge
//!   below the top one, and the numbers are never swapped.
//! - `rasterize` takes one page item or a list of at most [`super::MAX_LISTED`] of them, all of
//!   this document, none listed twice, inside another listed one or a path of a compound path, and
//!   none locked or in a locked layer or group. It keeps the selection, less the art it replaces,
//!   and gives the new `RasterItem`: with `clippingMask`, the image inside its clip group.
//!   `transparency` wins over `backgroundBlack`. `convertSpotColors`, `convertTextToOutlines` and
//!   `includeLayers` true throw an error that says they are not available yet.
//! - `imageCapture` writes a PNG, and the file's name must end with `.png`. Without `clipBounds` it
//!   captures the bounds of all the visible art, on the artboards or off them. With
//!   `transparency` the image keeps its transparency; otherwise it is drawn on `matteColor` with
//!   `matte`, and on white without. `matteColor` is a record with `red`, `green` and `blue` from 0
//!   to 255. The file is written only where the script's `File` objects may write, and never over
//!   an open document's file or a file one of them links.
//! - An image of more than 64 megapixels or 32,768 pixels a side throws an error before anything is
//!   drawn; the resolution is never lowered to make it fit.
//! - `windowCapture` throws an error that says it is not available yet.

use std::rc::Rc;

use serde_json::{Map, Value, json};
use vectorcraft_geom::Rect;
use vectorcraft_js::{Data, HostClass, HostError, HostFunction, HostMember};

use super::{
    Answer, Doc, Host, Link, MAX_LISTED, STALE_ITEM, arg, boolean, command, coordinate, current, frame, given, item_of, item_ref, later, method,
    named, num, on_doc, text, this_doc, with_doc,
};

/// The lowest resolution the option classes take, in pixels per inch.
pub const MIN_RESOLUTION: f64 = 72.0;
/// The highest resolution the option classes take, in pixels per inch.
pub const MAX_RESOLUTION: f64 = 2400.0;
/// The most room `RasterizeOptions.padding` adds around the art, in points.
pub const MAX_PADDING: f64 = 1000.0;
/// The error of a source art object from another document.
pub const OTHER_DOCUMENT: &str = "The source art must be in this document";

/// The enumerations these members use, as `dom`'s are made.
pub(super) const ENUMS: &[(&str, &[&str])] =
    &[("AntiAliasingMethod", &["ARTOPTIMIZED", "None", "TYPEOPTIMIZED"]), ("RasterizationColorModel", &["BITMAP", "DEFAULTCOLORMODEL", "GRAYSCALE"])];

/// The member of `enumeration` that `d` names (`"Enumeration.MEMBER"`).
fn member<'a>(d: &'a Data, enumeration: &str, what: &str) -> std::result::Result<&'a str, HostError> {
    let members = ENUMS.iter().find(|(n, _)| *n == enumeration).map_or(&[][..], |(_, m)| *m);
    let bad = || HostError::Type(format!("{what} must be a {enumeration} value"));
    let Data::String(s) = d else { return Err(bad()) };
    let m = s.strip_prefix(enumeration).and_then(|r| r.strip_prefix('.')).ok_or_else(bad)?;
    if members.contains(&m) { Ok(m) } else { Err(bad()) }
}

/// A finite number from `lo` to `hi`.
fn number_in(d: &Data, what: &str, lo: f64, hi: f64) -> std::result::Result<f64, HostError> {
    match d {
        Data::Number(n) if n.is_finite() => {
            if (lo..=hi).contains(n) {
                Ok(*n)
            } else {
                Err(HostError::Range(format!("{what} must be from {lo} to {hi}")))
            }
        }
        _ => Err(HostError::Type(format!("{what} must be a finite number"))),
    }
}

/// The fields of the options object `d` of `class`, or `None` when the script gave none.
fn options<'a>(d: &'a Data, class: &str) -> std::result::Result<Option<&'a [(String, Data)]>, HostError> {
    match given(d) {
        None | Some(Data::Null) => Ok(None),
        Some(Data::Object(items)) if items.iter().any(|(k, v)| k == "typename" && matches!(v, Data::String(s) if s == class)) => Ok(Some(items)),
        Some(_) => Err(HostError::Type(format!("the options must be a {class} object"))),
    }
}

/// `clipBounds`, `[left, top, right, bottom]` in the run's script coordinates, as a rectangle in
/// document coordinates; `None` when the script gave none.
fn clip_bounds(h: &Host, doc: Doc, d: &Data) -> std::result::Result<Option<Rect>, HostError> {
    let a = match given(d) {
        None | Some(Data::Null) => return Ok(None),
        Some(Data::Array(a)) if a.len() == 4 => a,
        Some(_) => return Err(HostError::Type("clipBounds must be an array of four numbers [left, top, right, bottom]".into())),
    };
    let mut n = [0.0; 4];
    for (o, d) in n.iter_mut().zip(a) {
        *o = coordinate(d, "clipBounds")?;
    }
    let f = with_doc(h, doc, |_, st| frame(st))?;
    let c = h.dom.coords.get();
    let (p0, p1) = (f.to_document(c, [n[0], n[1]]), f.to_document(c, [n[2], n[3]]));
    if p1.x <= p0.x || p1.y <= p0.y {
        return Err(HostError::Range("clipBounds must have its right edge right of its left edge and its bottom edge below its top edge".into()));
    }
    Ok(Some(Rect::new(p0.x, p0.y, p1.x, p1.y)))
}

/// Throws when an image of `region` at `ppi` would be more than 64 megapixels or longer than the
/// renderer draws on a side, before anything is drawn.
fn check_size(region: Rect, ppi: f64) -> std::result::Result<(), HostError> {
    let scale = ppi / 72.0;
    let (w, h) = ((region.width() * scale).round().max(1.0), (region.height() * scale).round().max(1.0));
    let side = f64::from(vectorcraft_render::MAX_RASTER_SIDE);
    if w * h > crate::cmd::menucmds::MAX_PIXELS || w > side || h > side {
        return Err(HostError::Range(format!(
            "the image would be {w} × {h} pixels; images are limited to 64 megapixels and {side} pixels a side: lower the resolution or the bounds"
        )));
    }
    Ok(())
}

// ---- Rasterize ----

/// `object.rasterize`'s params for the `RasterizeOptions` fields `items` (`None`: the defaults),
/// with every option given.
fn rasterize_params(items: Option<&[(String, Data)]>) -> std::result::Result<Map<String, Value>, HostError> {
    let (mut anti_alias, mut black, mut clear, mut clip, mut model, mut padding, mut ppi) = ("art", false, false, false, "document", 0.0, 300.0);
    for (k, v) in items.unwrap_or_default() {
        if matches!(v, Data::Undefined) {
            continue;
        }
        let what = format!("RasterizeOptions.{k}");
        match k.as_str() {
            "antiAliasingMethod" => {
                anti_alias = match member(v, "AntiAliasingMethod", &what)? {
                    "None" => "none",
                    "TYPEOPTIMIZED" => "type",
                    _ => "art",
                }
            }
            "backgroundBlack" => black = boolean(v, &what)?,
            "transparency" => clear = boolean(v, &what)?,
            "clippingMask" => clip = boolean(v, &what)?,
            "colorModel" => {
                model = match member(v, "RasterizationColorModel", &what)? {
                    "BITMAP" => "bitmap",
                    "GRAYSCALE" => "grayscale",
                    _ => "document",
                }
            }
            "padding" => padding = number_in(v, &what, 0.0, MAX_PADDING)?,
            "resolution" => ppi = number_in(v, &what, MIN_RESOLUTION, MAX_RESOLUTION)?,
            "convertSpotColors" | "convertTextToOutlines" | "includeLayers" if boolean(v, &what)? => {
                return Err(HostError::Error(format!("{what} is not available yet")));
            }
            _ => {}
        }
    }
    let background = if clear {
        "transparent"
    } else if black {
        "black"
    } else {
        "white"
    };
    let mut p = Map::new();
    p.insert("ppi".into(), json!(ppi));
    p.insert("background".into(), json!(background));
    p.insert("antiAlias".into(), json!(anti_alias));
    p.insert("padding".into(), json!(padding));
    p.insert("colorModel".into(), json!(model));
    p.insert("clippingMask".into(), json!(clip));
    Ok(p)
}

/// The ids of the source art `d` names for `rasterize` in the document `doc`: one page item, or a
/// list of them, none inside another listed one. The command checks that none is listed twice or
/// locked.
fn sources(h: &Host, doc: Doc, d: &Data) -> std::result::Result<Vec<u64>, HostError> {
    let list: Vec<&Data> = match d {
        Data::Array(a) if a.len() > MAX_LISTED => return Err(HostError::Range(format!("rasterize takes at most {MAX_LISTED} objects"))),
        Data::Array(a) => a.iter().collect(),
        Data::Undefined | Data::Null => return Err(HostError::Type("rasterize needs the source art, a page item or a list of them".into())),
        d => vec![d],
    };
    if list.is_empty() {
        return Err(HostError::Error("rasterize needs at least one page item".into()));
    }
    h.charge_call(16 * list.len() as u64)?;
    let mut ids = Vec::with_capacity(list.len());
    for d in list {
        let item = item_of(h, d)?;
        if item.doc.uid != doc.uid {
            return Err(HostError::Error(OTHER_DOCUMENT.into()));
        }
        ids.push(item.id.0);
    }
    let listed: std::collections::HashSet<u64> = ids.iter().copied().collect();
    let inside = with_doc(h, doc, |_, st| {
        ids.iter().find_map(|id| {
            let up = st.doc.ancestry(crate::NodeId(*id))?;
            up.iter().any(|a| a.0 != *id && listed.contains(&a.0)).then_some(())
        })
    });
    if inside.is_some() {
        return Err(HostError::Error("a source page item is inside another one, which is listed too".into()));
    }
    Ok(ids)
}

/// `document.rasterize(sourceArt, clipBounds?, options?)`: `object.rasterize` of the source art,
/// which the image replaces in one undo step. → the new `RasterItem`.
fn doc_rasterize(h: &Host, a: &[Data]) -> Answer {
    let doc = this_doc(h, a)?;
    let ids = sources(h, doc, arg(a, 1))?;
    let clip = clip_bounds(h, doc, arg(a, 2))?;
    let mut p = rasterize_params(options(arg(a, 3), "RasterizeOptions")?)?;
    p.insert("ids".into(), json!(ids));
    if let Some(r) = clip {
        p.insert("clipBounds".into(), json!([r.x0, r.y0, r.x1, r.y1]));
    }
    let r = on_doc(h, doc.uid, true, |h| command(h, "object.rasterize", &Value::Object(p)))?;
    let image = match &r {
        Data::Object(o) => o.iter().find_map(|(k, v)| match (k.as_str(), v) {
            ("imageId", Data::Number(n)) => super::whole(*n),
            _ => None,
        }),
        _ => None,
    };
    let image = image.ok_or_else(|| HostError::Error("the image was not made".into()))?;
    let s = h.session.borrow();
    let st = s.documents().iter().find(|d| d.uid == doc.uid).ok_or_else(|| HostError::Error(STALE_ITEM.into()))?;
    Ok(item_ref(h, st, crate::NodeId(image)))
}

// ---- Image capture ----

/// A channel of `matteColor`: a finite number from 0 to 255.
fn channel(items: &[(String, Data)], key: &str) -> std::result::Result<u8, HostError> {
    let d = items.iter().find(|(k, _)| k == key).map_or(&Data::Undefined, |(_, v)| v);
    let n = number_in(d, &format!("ImageCaptureOptions.matteColor.{key}"), 0.0, 255.0)?;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let byte = n.round() as u8;
    Ok(byte)
}

/// The capture's resolution and `document.export`'s `background` and `antiAlias` for the
/// `ImageCaptureOptions` fields `items` (`None`: the defaults).
fn capture_settings(items: Option<&[(String, Data)]>) -> std::result::Result<(f64, String, &'static str), HostError> {
    let (mut anti_alias, mut matte, mut clear, mut ppi) = (false, false, false, 150.0);
    let mut color = [255u8; 3];
    for (k, v) in items.unwrap_or_default() {
        if matches!(v, Data::Undefined) {
            continue;
        }
        let what = format!("ImageCaptureOptions.{k}");
        match k.as_str() {
            "antiAliasing" => anti_alias = boolean(v, &what)?,
            "matte" => matte = boolean(v, &what)?,
            "transparency" => clear = boolean(v, &what)?,
            "resolution" => ppi = number_in(v, &what, MIN_RESOLUTION, MAX_RESOLUTION)?,
            "matteColor" => {
                let Data::Object(c) = v else { return Err(HostError::Type(format!("{what} must be an RGBColor"))) };
                color = [channel(c, "red")?, channel(c, "green")?, channel(c, "blue")?];
            }
            _ => {}
        }
    }
    let background = if clear {
        "transparent".to_string()
    } else if matte {
        format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2])
    } else {
        "white".to_string()
    };
    Ok((ppi, background, if anti_alias { "art" } else { "none" }))
}

/// The files a capture must not write over: the open documents' files and the files their linked
/// images and placed documents show, wherever those are stored (opacity masks, symbols, patterns
/// and brushes included), with a DCS image's plate files, also when a file is missing, in the form
/// [`placed`] gives. A link's places are the ones opening the document looks at: its stored path
/// from the document's folder and its relative path from the folder of the file the document was
/// read from (a template opened as an untitled document has no file of its own), with a DCS
/// image's plate files beside each. The documents the interface has open while the run's slice
/// runs, such as one opened while the run was paused, count too.
fn sources_on_disk(h: &Host) -> Vec<Placed> {
    let s = h.session.borrow();
    let interface = h.interface_docs.borrow();
    let mut paths: Vec<String> = Vec::new();
    for st in s.documents().iter().chain(interface.iter()) {
        paths.extend(st.path.clone());
        paths.extend(crate::cmd::links::link_read_places(&st.doc, crate::cmd::links::LinkBase::of(st)));
    }
    paths.iter().filter_map(|p| placed(std::path::Path::new(p))).collect()
}

/// A file as its folder's real path and its name.
type Placed = (std::path::PathBuf, std::ffi::OsString);

/// `p` as its folder's real path and its file name: from its real path with links followed, or,
/// for a file that is not there, from its folder's real path. `None` when neither the file nor its
/// folder is there.
fn placed(p: &std::path::Path) -> Option<Placed> {
    let real = std::fs::canonicalize(p).ok().or_else(|| Some(std::fs::canonicalize(p.parent()?).ok()?.join(p.file_name()?)))?;
    Some((real.parent()?.to_path_buf(), real.file_name()?.to_os_string()))
}

/// Whether two [`placed`] files are one file: the same folder, and the same name, or names that
/// differ only in letter case in a folder whose volume ignores case.
fn same_file(a: &Placed, b: &Placed) -> bool {
    same_folder(&a.0, &b.0) && (a.1 == b.1 || (a.1.to_string_lossy().to_lowercase() == b.1.to_string_lossy().to_lowercase() && ignores_case(&a.0)))
}

/// Whether two real paths are one file or folder: the same path, or, on Unix, the same device and
/// file number, so a path spelled in another letter case on a volume that ignores case still
/// matches. On Windows a real path carries the letter case stored on disk.
fn same_folder(a: &std::path::Path, b: &std::path::Path) -> bool {
    if a == b {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(x), Ok(y)) = (std::fs::metadata(a), std::fs::metadata(b)) {
            return x.dev() == y.dev() && x.ino() == y.ino();
        }
    }
    false
}

/// Whether `other`, a spelling of `entry` in other letter case, finds `entry` itself and not a
/// symbolic link or another file.
fn finds_same(entry: &std::path::Path, other: &std::path::Path) -> bool {
    if !std::fs::symlink_metadata(other).is_ok_and(|m| !m.file_type().is_symlink()) {
        return false;
    }
    match (std::fs::canonicalize(entry), std::fs::canonicalize(other)) {
        (Ok(a), Ok(b)) => same_folder(&a, &b),
        _ => false,
    }
}

/// `name` with the letter case of every ASCII letter swapped, and other characters as they are.
fn swapped_case(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_lowercase() { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() }).collect()
}

/// Whether file names in the real folder `folder` ignore letter case. Names keep case when two of
/// the folder's first 256 entries differ only in the case of ASCII letters. Otherwise the first of
/// them whose name has an ASCII letter and that is not a symbolic link is looked up with the case
/// of those letters swapped: names ignore case when that finds the same entry. With no such entry
/// on macOS, where a whole volume either ignores or keeps case, the nearest folder at or above
/// `folder` whose name has an ASCII letter, with its parent on the same volume, is looked up the
/// same way in its parent. When neither tells, names are taken to ignore case.
fn ignores_case(folder: &std::path::Path) -> bool {
    let entries: Vec<std::fs::DirEntry> = std::fs::read_dir(folder).into_iter().flatten().flatten().take(256).collect();
    let mut folded = std::collections::HashSet::new();
    let names: std::collections::HashSet<String> = entries.iter().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    if !names.iter().all(|n| folded.insert(n.to_ascii_lowercase())) {
        return false;
    }
    for entry in &entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let swapped = swapped_case(&name);
        if swapped != name && entry.file_type().is_ok_and(|t| !t.is_symlink()) {
            return finds_same(&entry.path(), &folder.join(swapped));
        }
    }
    #[cfg(target_os = "macos")]
    for dir in folder.ancestors() {
        use std::os::unix::fs::MetadataExt;
        let (Some(parent), Some(name)) = (dir.parent(), dir.file_name()) else { break };
        let device = |p: &std::path::Path| std::fs::metadata(p).map(|m| m.dev()).ok();
        let here = device(folder);
        if here.is_none() || device(dir) != here || device(parent) != here {
            break;
        }
        let name = name.to_string_lossy();
        let swapped = swapped_case(&name);
        if swapped != name {
            return finds_same(dir, &parent.join(swapped));
        }
    }
    true
}

/// `document.imageCapture(imageFile, clipBounds?, options?)`: `document.export` of a PNG of the
/// art inside `clipBounds`, or of all the visible art.
fn doc_image_capture(h: &Host, a: &[Data]) -> Answer {
    let doc = this_doc(h, a)?;
    let path = vectorcraft_js::file_ref_path(arg(a, 1)).ok_or_else(|| HostError::Type("imageCapture needs a File".into()))?;
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path).to_lowercase();
    if !name.ends_with(".png") || name.len() <= ".png".len() {
        return Err(HostError::Error("imageCapture writes a PNG file: give it a name that ends with .png".into()));
    }
    let clip = clip_bounds(h, doc, arg(a, 2))?;
    let (ppi, background, anti_alias) = capture_settings(options(arg(a, 3), "ImageCaptureOptions")?)?;
    let region = match clip {
        Some(r) => r,
        None => with_doc(h, doc, |_, st| crate::cmd::fileio::export_art_bounds(&st.doc, "png"))
            .ok_or_else(|| HostError::Error("the document has no visible art to capture".into()))?,
    };
    check_size(region, ppi)?;
    let real = h.disk.borrow().save_target(path).map_err(|m| HostError::Error(format!("{m}: {path}")))?;
    let target = placed(&real);
    if target.is_some_and(|t| sources_on_disk(h).iter().any(|s| same_file(&t, s))) {
        return Err(HostError::Error(format!("imageCapture cannot write over an open document or a file it links: {path}")));
    }
    let p = json!({
        "path": crate::cmd::findfiles::plain(real).to_string_lossy(),
        "format": "png",
        "rect": [region.x0, region.y0, region.width(), region.height()],
        "ppi": ppi,
        "background": background,
        "antiAlias": anti_alias,
        "includeArtboardFill": false,
    });
    h.disk.borrow_mut().exact = true;
    let r = on_doc(h, doc.uid, false, |h| command(h, "document.export", &p));
    h.disk.borrow_mut().exact = false;
    r?;
    Ok(Data::Undefined)
}

/// The members these add to `Document`.
pub(super) fn document_members(l: &Link) -> Vec<(String, HostMember)> {
    let mut members = named(vec![("rasterize", method(l, doc_rasterize)), ("imageCapture", method(l, doc_image_capture))]);
    members.push(later(l, "Document", "windowCapture"));
    members
}

/// The fields a new options object starts with.
type Starts = fn() -> Vec<(&'static str, Data)>;

fn rasterize_starts() -> Vec<(&'static str, Data)> {
    vec![
        ("antiAliasingMethod", text("AntiAliasingMethod.ARTOPTIMIZED")),
        ("backgroundBlack", Data::Bool(false)),
        ("clippingMask", Data::Bool(false)),
        ("colorModel", text("RasterizationColorModel.DEFAULTCOLORMODEL")),
        ("convertSpotColors", Data::Bool(false)),
        ("convertTextToOutlines", Data::Bool(false)),
        ("includeLayers", Data::Bool(false)),
        ("padding", num(0.0)),
        ("resolution", num(300.0)),
        ("transparency", Data::Bool(false)),
    ]
}

fn capture_starts() -> Vec<(&'static str, Data)> {
    let white = Data::Object(vec![
        ("typename".to_string(), text("RGBColor")),
        ("red".to_string(), num(255.0)),
        ("green".to_string(), num(255.0)),
        ("blue".to_string(), num(255.0)),
    ]);
    vec![
        ("antiAliasing", Data::Bool(false)),
        ("matte", Data::Bool(false)),
        ("matteColor", white),
        ("resolution", num(150.0)),
        ("transparency", Data::Bool(false)),
    ]
}

/// The option classes: each `new Name()` gives a record with its `typename` and the defaults its
/// help page gives.
pub(super) fn classes(link: &Link) -> Vec<HostClass> {
    let all: [(&'static str, Starts); 2] = [("RasterizeOptions", rasterize_starts), ("ImageCaptureOptions", capture_starts)];
    all.into_iter()
        .map(|(name, starts)| {
            let l = link.clone();
            let f: HostFunction = Rc::new(move |_: Vec<Data>| {
                current(&l)?.charge_call(64)?;
                let mut items = vec![("typename".to_string(), text(name))];
                items.extend(starts().into_iter().map(|(k, v)| (k.to_string(), v)));
                Ok(Data::Object(items))
            });
            HostClass { name: name.into(), parent: None, members: Vec::new(), index: None, construct: Some(f) }
        })
        .collect()
}
