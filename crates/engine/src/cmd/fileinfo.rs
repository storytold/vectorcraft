//! File → File Info (`file.info`): the document title and its descriptive metadata (author,
//! description, keywords, rating, copyright), with the read-only created and modified dates. The
//! SVG (with `metadata`), PDF and PNG writers carry it, and a PDF or `.ai` opens with the File Info
//! it gives ([`read_pdf`]).

use serde_json::{Map, Value, json};
use vectorcraft_doc::metadata::{MAX_RATING, iso8601};
use vectorcraft_doc::{ColorMode, CopyrightStatus, DocMetadata, Document};

use super::*;

const C: &str = "file.info";
/// Longest text field, in characters.
const MAX_TEXT: usize = 65_536;
/// Longest keyword, in characters.
const MAX_KEYWORD: usize = 256;
const MAX_KEYWORDS: usize = 1000;
/// Values `file.info` reports but doesn't take (fed back, they are ignored).
const READ_ONLY: [&str; 6] = ["created", "modified", "colorMode", "units", "artboards", "objects"];

/// What `file.info` reports (and takes back).
pub(crate) fn info_json(d: &Document) -> Value {
    let m = &d.metadata;
    json!({
        "title": d.title,
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
    let d = &s.doc()?.doc;
    let (mut title, mut m) = (d.title.clone(), d.metadata.clone());
    for (k, v) in o {
        apply(&mut title, &mut m, k, v)?;
    }
    let d = &s.doc()?.doc;
    if title != d.title || m != d.metadata {
        s.edit("File Info", |d, _| {
            d.title = title;
            d.metadata = m;
            Ok(())
        })?;
    }
    Ok(info_json(&s.doc()?.doc))
}

/// The File Info a PDF or PDF-compatible `.ai` gives ([`vectorcraft_pdf::file_info`]) into `m`: each
/// field that `file.info` would take (a value it refuses is left out), and the created and
/// modified dates.
pub(crate) fn read_pdf(m: &mut DocMetadata, f: vectorcraft_pdf::FileInfo) {
    let status = f.copyrighted.map(|c| if c { CopyrightStatus::Copyrighted } else { CopyrightStatus::PublicDomain });
    let fields = [
        ("author", f.author.map(Value::from)),
        ("description", f.description.map(Value::from)),
        ("keywords", f.keywords.map(Value::from)),
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
        return Err(bad(C, format!("{k} has control characters{}", if multiline { "" } else { " or line breaks" })));
    }
    Ok(s.to_string())
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
