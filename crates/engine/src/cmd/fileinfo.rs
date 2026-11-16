//! File → File Info (`file.info`): the document title and its descriptive metadata (author,
//! description, keywords, rating, copyright), with the read-only created and modified dates. The
//! title is the one the file the document was opened from gave itself ([`DocMetadata::title`]),
//! else the document's name. The SVG (with `metadata`), PDF and PNG writers carry it, and a PDF or
//! `.ai` opens with the File Info it gives ([`read_pdf`]).
//!
//! Metadata templates: `file.info.saveTemplate` writes the File Info as an XMP file
//! ([`vectorcraft_pdf::metadata_template`]), `file.info.applyTemplate` merges one into the File
//! Info, and `file.info.templates` lists those in the metadata templates folder.

use serde_json::{Map, Value, json};
use vectorcraft_doc::metadata::{MAX_RATING, iso8601};
use vectorcraft_doc::{ColorMode, CopyrightStatus, DocMetadata, Document};

use super::{Result, Session, bad, switch_or, text_param};

const C: &str = "file.info";
const SAVE: &str = "file.info.saveTemplate";
const APPLY: &str = "file.info.applyTemplate";
const LIST: &str = "file.info.templates";
/// The extension of a metadata template file.
const TEMPLATE_EXT: &str = "xmp";
/// The most templates `file.info.templates` lists.
const MAX_LISTED: usize = 1000;
/// The largest metadata template `file.info.saveTemplate` writes and `file.info.applyTemplate`
/// reads, in bytes (8 MB). The largest template File Info's fields can give is under 4 MB.
const MAX_TEMPLATE_FILE: usize = vectorcraft_pdf::MAX_TEMPLATE_BYTES;
/// Longest text field, in characters.
const MAX_TEXT: usize = 65_536;
/// Longest keyword, in characters.
const MAX_KEYWORD: usize = 256;
const MAX_KEYWORDS: usize = 1000;
/// Values `file.info` reports but doesn't take (fed back, they are ignored).
const READ_ONLY: [&str; 6] = ["created", "modified", "colorMode", "units", "artboards", "objects"];

/// What `file.info` reports (and takes back).
pub(crate) fn info_json(d: &Document) -> Value {
    info_json_with(d, d.info_title(), &d.metadata)
}

/// [`info_json`] with the title `title` and the metadata `m` in place of the document's.
fn info_json_with(d: &Document, title: &str, m: &DocMetadata) -> Value {
    json!({
        "title": title,
        "author": m.author,
        "authorTitle": m.author_title,
        "description": m.description,
        "keywords": m.keywords,
        "rating": m.rating,
        "copyrightStatus": m.copyright_status.id(),
        "copyrightNotice": m.copyright_notice,
        "copyrightUrl": m.copyright_url,
        "created": m.created.map(iso8601),
        "modified": m.modified.map(iso8601),
        "colorMode": match d.color_mode { ColorMode::Rgb => "rgb", ColorMode::Cmyk => "cmyk" },
        "units": d.units.label(),
        "artboards": d.artboards.len(),
        "objects": d.node_count(),
    })
}

pub(crate) fn file_info(s: &mut Session, p: &Value) -> Result<Value> {
    let empty = Map::new();
    let o = match p {
        Value::Null => &empty,
        Value::Object(o) => o,
        _ => return Err(bad(C, "params must be an object")),
    };
    let (title, m) = edited(s, Some(o))?;
    commit(s, title, m)?;
    Ok(info_json(&s.doc()?.doc))
}

/// The active document's File Info title and metadata with `fields` (as `file.info` takes them)
/// applied.
fn edited(s: &Session, fields: Option<&Map<String, Value>>) -> Result<(String, DocMetadata)> {
    let d = &s.doc()?.doc;
    let (mut title, mut m) = (d.info_title().to_string(), d.metadata.clone());
    for (k, v) in fields.into_iter().flatten() {
        apply(&mut title, &mut m, k, v)?;
    }
    Ok((title, m))
}

/// Make `title` and `m` the active document's File Info in one undo step; nothing is recorded when
/// they are what it has. Whether anything changed.
fn commit(s: &mut Session, title: String, mut m: DocMetadata) -> Result<bool> {
    let d = &s.doc()?.doc;
    // A title File Info changes becomes the document's own, as it always has (the tab and Save As
    // offer it where the document has no file), and replaces the one its file gave.
    let retitled = title != d.info_title();
    if retitled {
        m.title = None;
    }
    if !retitled && m == d.metadata {
        return Ok(false);
    }
    s.edit("File Info", |d, _| {
        if retitled {
            d.title = title;
        }
        d.metadata = m;
        Ok(())
    })?;
    Ok(true)
}

/// The optional object `p` gives for `key`.
fn fields_param<'a>(p: &'a Value, key: &str, c: &str) -> Result<Option<&'a Map<String, Value>>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(o)) => Ok(Some(o)),
        Some(_) => Err(bad(c, format!("`{key}` must be an object"))),
    }
}

impl Session {
    /// Have `file.info.saveTemplate`, `file.info.applyTemplate` and `file.info.templates` keep the
    /// metadata templates in the folder `dir`, which a save makes when it is missing.
    pub fn attach_metadata_templates_dir(&mut self, dir: std::path::PathBuf) {
        self.metadata_templates_dir = Some(dir);
    }

    /// The metadata templates folder ([`Session::attach_metadata_templates_dir`]); none on the web,
    /// which has no folders.
    pub fn metadata_templates_dir(&self) -> Option<&std::path::Path> {
        self.metadata_templates_dir.as_deref().filter(|_| !cfg!(target_arch = "wasm32"))
    }
}

/// The metadata template of the File Info `title` and `m`: each field that has a value, the title
/// only where it is one of its own ([`super::fileio::is_own_title`]), the rating only from 1 up.
fn template_of(title: &str, m: &DocMetadata) -> vectorcraft_pdf::FileInfo {
    let text = |s: &str| (!s.trim().is_empty()).then(|| s.to_string());
    vectorcraft_pdf::FileInfo {
        title: super::fileio::is_own_title(title).then(|| title.to_string()),
        author: text(&m.author),
        author_title: text(&m.author_title),
        description: text(&m.description),
        keywords: (!m.keywords.is_empty()).then(|| m.keywords.clone()),
        rating: (m.rating > 0).then_some(m.rating),
        copyright_notice: text(&m.copyright_notice),
        copyright_url: text(&m.copyright_url),
        copyrighted: match m.copyright_status {
            CopyrightStatus::Copyrighted => Some(true),
            CopyrightStatus::PublicDomain => Some(false),
            CopyrightStatus::Unknown => None,
        },
        ..Default::default()
    }
}

/// The path of the template `name` in the folder `dir`: `name` made a safe file name, with
/// [`TEMPLATE_EXT`].
fn template_path(dir: &std::path::Path, name: &str, c: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(bad(c, "the template name can't be empty"));
    }
    Ok(dir.join(super::swatchlib::library_file_name(name, TEMPLATE_EXT)).to_string_lossy().into_owned())
}

/// The error for a template over [`MAX_TEMPLATE_FILE`].
fn too_big(c: &str) -> crate::EngineError {
    bad(c, format!("the file is larger than {} MB", MAX_TEMPLATE_FILE >> 20))
}

/// File Info › Save Metadata Template: `file.info.saveTemplate {name?, path?, fields?}`.
pub(crate) fn save_template(s: &mut Session, p: &Value) -> Result<Value> {
    let name = text_param(p, "name", SAVE)?;
    let path = text_param(p, "path", SAVE)?;
    let (title, m) = edited(s, fields_param(p, "fields", SAVE)?)?;
    let xmp = vectorcraft_pdf::metadata_template(&template_of(&title, &m));
    if xmp.len() > MAX_TEMPLATE_FILE {
        return Err(too_big(SAVE));
    }
    match (name, path) {
        (Some(_), Some(_)) => Err(bad(SAVE, "give `name` or `path`, not both")),
        (Some(name), None) => {
            let dir = s.metadata_templates_dir().ok_or_else(|| bad(SAVE, "no metadata templates folder here"))?.to_path_buf();
            let path = template_path(&dir, name, SAVE)?;
            // The name is the file name as it is, so two names never share a file.
            let listed = std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned());
            if listed.as_deref() != Some(name.trim()) {
                return Err(bad(SAVE, "the template name isn't usable as a file name"));
            }
            // Only a file in the folder itself is replaced: a link there is not followed.
            if std::fs::symlink_metadata(&path).is_ok_and(|m| !m.is_file()) {
                return Err(crate::EngineError::Other(format!("{path}: not a file")));
            }
            let dir = dir.to_string_lossy().into_owned();
            // The app's own folder, under a name checked above: no root applies.
            crate::file_access::unconfined(|| super::fileio::create_dir(&dir).and_then(|()| super::fileio::write_file(&path, xmp.as_bytes())))?;
            Ok(json!({ "name": listed, "path": super::fileio::shown_path(&path) }))
        }
        (None, Some(path)) => {
            super::fileio::write_file(path, xmp.as_bytes())?;
            Ok(json!({ "path": path }))
        }
        (None, None) => Ok(json!({ "data": xmp })),
    }
}

/// How `file.info.applyTemplate` merges a template's fields into the File Info.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Merge {
    /// Clear the fields a template can hold, then take the template's.
    Replace,
    /// Keep the fields, and take each one the template gives.
    Keep,
    /// Keep the fields: add the template's keywords, and take its other fields only where the File
    /// Info has none.
    Append,
}

/// File Info › Import: `file.info.applyTemplate {path?|name?|data?, mode, apply?, base?}`.
pub(crate) fn apply_template(s: &mut Session, p: &Value) -> Result<Value> {
    let mode = match text_param(p, "mode", APPLY)? {
        Some("replace") => Merge::Replace,
        Some("keep") => Merge::Keep,
        Some("append") => Merge::Append,
        _ => return Err(bad(APPLY, "mode must be replace, keep or append")),
    };
    let apply = switch_or(p, "apply", true, APPLY)?;
    let base = fields_param(p, "base", APPLY)?;
    let bytes = template_bytes(s, p)?;
    let t = vectorcraft_pdf::read_template(&bytes).ok_or_else(|| bad(APPLY, "the file is not an XMP metadata template"))?;
    let (mut title, mut m) = edited(s, base)?;
    merge(&mut title, &mut m, t, mode);
    if !apply {
        return Ok(info_json_with(&s.doc()?.doc, &title, &m));
    }
    let changed = commit(s, title, m)?;
    let mut out = info_json(&s.doc()?.doc);
    out["changed"] = json!(changed);
    Ok(out)
}

/// The bytes of the template `p` names: `path`, `name` (in the metadata templates folder) or
/// `data`, exactly one, at most [`MAX_TEMPLATE_FILE`] long.
fn template_bytes(s: &Session, p: &Value) -> Result<Vec<u8>> {
    let given = [text_param(p, "path", APPLY)?, text_param(p, "name", APPLY)?, text_param(p, "data", APPLY)?];
    // At most one byte past the limit is read, so a huge file is never read whole.
    let read = |path: &str| super::fileio::read_file_head(path, MAX_TEMPLATE_FILE + 1);
    let bytes = match given {
        [Some(path), None, None] => read(path)?,
        [None, Some(name), None] => {
            let dir = s.metadata_templates_dir().ok_or_else(|| bad(APPLY, "no metadata templates folder here"))?;
            let mut path = template_path(dir, name, APPLY)?;
            // A template `file.info.templates` lists: by its `file` first, then by its `name` with
            // the extension in lowercase, then in other letter case. Only files in the folder itself
            // are listed: a link there is not followed.
            let name = name.trim();
            let own = format!("{name}.{TEMPLATE_EXT}");
            let listed = template_files(dir)
                .filter(|f| f == name || f.rsplit_once('.').is_some_and(|(stem, _)| stem == name))
                .min_by_key(|f| (f != name, *f != own));
            if let Some(file) = listed {
                path = dir.join(file).to_string_lossy().into_owned();
            } else if std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy()).as_deref() != Some(name) {
                // A name that is not a file name as it is never opens another template.
                return Err(bad(APPLY, "the template name isn't usable as a file name"));
            }
            let plain_file = |path: &str| std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file());
            if std::fs::symlink_metadata(&path).is_ok() && !plain_file(&path) {
                return Err(crate::EngineError::Other(format!("{path}: not a file")));
            }
            // The app's own folder: no root applies.
            crate::file_access::unconfined(|| read(&path))?
        }
        [None, None, Some(data)] if data.len() > MAX_TEMPLATE_FILE => return Err(too_big(APPLY)),
        [None, None, Some(data)] => data.as_bytes().to_vec(),
        _ => return Err(bad(APPLY, "give one of `path`, `name` and `data`")),
    };
    if bytes.len() > MAX_TEMPLATE_FILE {
        return Err(too_big(APPLY));
    }
    Ok(bytes)
}

/// Merge the template `t` into the File Info `title` and `m` as `mode` says. Each field goes in as
/// `file.info` would take it; a value it would refuse is left out.
fn merge(title: &mut String, m: &mut DocMetadata, t: vectorcraft_pdf::FileInfo, mode: Merge) {
    if mode == Merge::Replace {
        // The title is never cleared: a document always has one.
        m.author.clear();
        m.author_title.clear();
        m.description.clear();
        m.keywords.clear();
        m.rating = 0;
        m.copyright_status = CopyrightStatus::Unknown;
        m.copyright_notice.clear();
        m.copyright_url.clear();
    }
    let status = t.copyrighted.map(|c| if c { CopyrightStatus::Copyrighted } else { CopyrightStatus::PublicDomain });
    let fields = [
        ("title", t.title.map(Value::from)),
        ("author", t.author.map(Value::from)),
        ("authorTitle", t.author_title.map(Value::from)),
        ("description", t.description.map(Value::from)),
        ("keywords", t.keywords.map(Value::from)),
        ("rating", t.rating.map(Value::from)),
        ("copyrightStatus", status.map(|s| Value::from(s.id()))),
        ("copyrightNotice", t.copyright_notice.map(Value::from)),
        ("copyrightUrl", t.copyright_url.map(Value::from)),
    ];
    for (k, v) in fields {
        let Some(mut v) = v else { continue };
        if mode == Merge::Append {
            let empty = match k {
                "title" => !super::fileio::is_own_title(title),
                "author" => m.author.is_empty(),
                "authorTitle" => m.author_title.is_empty(),
                "description" => m.description.is_empty(),
                "rating" => m.rating == 0,
                "copyrightStatus" => m.copyright_status == CopyrightStatus::Unknown,
                "copyrightNotice" => m.copyright_notice.is_empty(),
                "copyrightUrl" => m.copyright_url.is_empty(),
                _ => true,
            };
            if !empty {
                continue;
            }
            if k == "keywords" {
                let mut all: Vec<Value> = m.keywords.iter().cloned().map(Value::from).collect();
                all.extend(v.as_array().cloned().unwrap_or_default());
                v = Value::from(all);
            }
        }
        // `apply` changes the field only when it takes the value.
        let _ = apply(title, m, k, &v);
    }
}

/// The file names of the templates in the folder `dir`: its files with a name before the
/// [`TEMPLATE_EXT`] extension, in any case. The app's own folder, read directly; a folder not made
/// yet holds none.
fn template_files(dir: &std::path::Path) -> impl Iterator<Item = String> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|file| file.rsplit_once('.').is_some_and(|(stem, ext)| ext.eq_ignore_ascii_case(TEMPLATE_EXT) && !stem.is_empty()))
}

/// `file.info.templates {create?}`: the metadata templates in the folder, by name.
pub(crate) fn templates(s: &mut Session, p: &Value) -> Result<Value> {
    let create = switch_or(p, "create", false, LIST)?;
    let Some(dir) = s.metadata_templates_dir().map(|d| d.to_string_lossy().into_owned()) else {
        return Ok(json!({ "templates": [] }));
    };
    if create {
        // The app's own folder: no root applies.
        crate::file_access::unconfined(|| super::fileio::create_dir(&dir))?;
    }
    // The first [`MAX_LISTED`] by name, with no more than that many held at once.
    let mut files = std::collections::BTreeSet::new();
    for file in template_files(std::path::Path::new(&dir)) {
        files.insert(file);
        if files.len() > MAX_LISTED {
            files.pop_last();
        }
    }
    let list: Vec<Value> = files
        .iter()
        .filter_map(|file| {
            let (stem, _) = file.rsplit_once('.')?;
            let path = std::path::Path::new(&dir).join(file).to_string_lossy().into_owned();
            Some(json!({ "name": stem, "file": file, "path": super::fileio::shown_path(&path) }))
        })
        .collect();
    let mut out = json!({ "templates": list });
    if super::fileio::shown_path(&dir) == dir.as_str() {
        out["folder"] = json!(dir);
    }
    Ok(out)
}

/// The File Info a PDF or PDF-compatible `.ai` gives ([`vectorcraft_pdf::file_info`]) into `m`: each
/// field that `file.info` would take (a value it refuses is left out), its title ([`imported_title`])
/// as the File Info title, and the created and modified dates.
pub(crate) fn read_pdf(m: &mut DocMetadata, f: vectorcraft_pdf::FileInfo) {
    let status = f.copyrighted.map(|c| if c { CopyrightStatus::Copyrighted } else { CopyrightStatus::PublicDomain });
    let fields = [
        ("author", f.author.map(Value::from)),
        ("authorTitle", f.author_title.map(Value::from)),
        ("description", f.description.map(Value::from)),
        ("keywords", f.keywords.map(Value::from)),
        ("rating", f.rating.map(Value::from)),
        ("copyrightStatus", status.map(|s| Value::from(s.id()))),
        ("copyrightNotice", f.copyright_notice.map(Value::from)),
        ("copyrightUrl", f.copyright_url.map(Value::from)),
    ];
    // `apply` changes the field only when it takes the value.
    let mut title = String::new();
    for (k, v) in fields {
        if let Some(v) = v {
            let _ = apply(&mut title, m, k, &v);
        }
    }
    m.title = f.title.as_deref().and_then(imported_title);
    m.created = f.created.or(m.created);
    m.modified = f.modified.or(m.modified);
}

fn apply(title: &mut String, m: &mut DocMetadata, k: &str, v: &Value) -> Result<()> {
    match k {
        "title" => {
            *title = text(k, v, false)?;
            if title.is_empty() {
                return Err(bad(C, "title can't be empty"));
            }
        }
        "author" => m.author = text(k, v, false)?,
        "authorTitle" => m.author_title = text(k, v, false)?,
        "description" => m.description = text(k, v, true)?,
        "keywords" => m.keywords = keywords(v)?,
        "rating" => {
            m.rating = v
                .as_u64()
                .filter(|r| *r <= u64::from(MAX_RATING))
                .and_then(|r| u8::try_from(r).ok())
                .ok_or_else(|| bad(C, format!("rating must be a whole number from 0 to {MAX_RATING}")))?
        }
        "copyrightStatus" => {
            m.copyright_status =
                v.as_str().and_then(CopyrightStatus::parse).ok_or_else(|| bad(C, "copyrightStatus must be unknown, copyrighted or publicDomain"))?
        }
        "copyrightNotice" => m.copyright_notice = text(k, v, true)?,
        "copyrightUrl" => {
            let url = text(k, v, false)?;
            if url.chars().any(char::is_whitespace) || url.chars().count() > 2048 {
                return Err(bad(C, "copyrightUrl must be a URL (no spaces, at most 2048 characters)"));
            }
            m.copyright_url = url;
        }
        k if READ_ONLY.contains(&k) => {}
        _ => return Err(bad(C, format!("unknown File Info field `{k}`"))),
    }
    Ok(())
}

/// A trimmed text value (`null` clears it); line breaks only where `multiline`, no other control
/// characters.
fn text(k: &str, v: &Value, multiline: bool) -> Result<String> {
    let s = match v {
        Value::Null => "",
        Value::String(s) => s.trim(),
        _ => return Err(bad(C, format!("{k} must be text"))),
    };
    if s.chars().count() > MAX_TEXT {
        return Err(bad(C, format!("{k} is too long (at most {MAX_TEXT} characters)")));
    }
    if s.chars().any(|c| c.is_control() && !(multiline && matches!(c, '\n' | '\r' | '\t'))) {
        return Err(if multiline {
            bad(C, format!("{k} has control characters"))
        } else {
            bad(C, format!("{k} has control characters or line breaks"))
        });
    }
    Ok(s.to_string())
}

/// The File Info title `raw`, read from a file, makes: runs of white space (line breaks too)
/// become single spaces, other control characters are dropped, at most [`MAX_TEXT`] characters
/// are kept, as `file.info` takes a title. `None` when nothing is left, or when it is the
/// Untitled-N name New gives (what a document that never had a title writes to its files).
pub(crate) fn imported_title(raw: &str) -> Option<String> {
    let words: Vec<String> =
        raw.split_whitespace().map(|w| w.chars().filter(|c| !c.is_control()).collect::<String>()).filter(|w| !w.is_empty()).collect();
    let title: String = words.join(" ").chars().take(MAX_TEXT).collect();
    let title = title.trim_end();
    super::fileio::is_own_title(title).then(|| title.to_string())
}

/// Keywords from a list or a comma- or semicolon-separated string: trimmed, each once (ignoring
/// case), empty ones dropped.
fn keywords(v: &Value) -> Result<Vec<String>> {
    let parts: Vec<String> = match v {
        Value::Null => vec![],
        Value::String(s) => s.split([',', ';']).map(str::to_string).collect(),
        Value::Array(a) => a.iter().map(|x| text("keywords", x, false)).collect::<Result<_>>()?,
        _ => return Err(bad(C, "keywords must be a list of words or a comma-separated string")),
    };
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<String> = vec![];
    for k in parts.iter().map(|k| k.trim()).filter(|k| !k.is_empty()) {
        if k.chars().count() > MAX_KEYWORD || k.chars().any(char::is_control) {
            return Err(bad(C, format!("keywords must be at most {MAX_KEYWORD} characters, without control characters")));
        }
        if seen.insert(k.to_lowercase()) {
            out.push(k.to_string());
        }
        if out.len() > MAX_KEYWORDS {
            return Err(bad(C, format!("at most {MAX_KEYWORDS} keywords")));
        }
    }
    Ok(out)
}
