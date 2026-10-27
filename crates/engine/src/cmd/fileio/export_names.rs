use super::access::write_file;
use super::*;

/// File-name parts for artboards `boards` of `doc`: the artboard's name with unsafe characters as
/// `-`, `Artboard-N` when unnamed, and `-2`, `-3`… after a name already used (compared without
/// case: `Icon` and `icon` are one file on most desktop file systems).
pub fn artboard_file_names(doc: &vectorcraft_doc::Document, boards: &[usize]) -> Vec<String> {
    unique_file_names(boards.iter().map(|&b| (doc.artboards.get(b).map_or("", |a| a.name.as_str()), format!("Artboard-{}", b + 1))))
}

/// File-name parts for `(name, fallback)` pairs (artboards, assets): the name with unsafe
/// characters as `-`, the fallback when it is empty, and `-2`, `-3`… after a name already used
/// (compared without case).
pub(crate) fn unique_file_names<'a>(names: impl IntoIterator<Item = (&'a str, String)>) -> Vec<String> {
    distinct_names(names.into_iter().map(|(name, fallback)| {
        let base: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
        if base.is_empty() { fallback } else { base }
    }))
}

/// `names` with `-2`, `-3`… after a name already used (compared without case).
pub(super) fn distinct_names(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut taken = std::collections::HashSet::new();
    // The next number to try for each base: a name repeated many times doesn't search the numbers
    // taken before it.
    let mut next: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    names
        .into_iter()
        .map(|base| {
            let mut name = base.clone();
            while !taken.insert(name.to_lowercase()) {
                let n = next.entry(base.to_lowercase()).or_insert(1);
                *n += 1;
                name = format!("{base}-{n}");
            }
            name
        })
        .collect()
}

/// Write an export's files for the destination `path` → `{path, bytes, files?, linked?, …extra}`
/// (`files`: every file when there are several; `linked`: the images it links to); with no path
/// → the same with `dataBase64` and `{name, dataBase64}` rows, named after `name`.
pub(super) fn write_encoded(path: Option<&str>, name: &str, doc: &vectorcraft_doc::Document, enc: &Encoded, extra: Value) -> Result<Value> {
    let files = enc.named(doc, path.unwrap_or(name));
    let (main, linked) = files.split_at(enc.files.len());
    let row = |(name, bytes): &(String, std::borrow::Cow<[u8]>)| match path {
        Some(_) => json!(name),
        None => json!({ "name": name, "dataBase64": vectorcraft_format::base64_encode(bytes) }),
    };
    if path.is_some() {
        for (p, bytes) in &files {
            write_file(p, bytes)?;
        }
    }
    let Some((first, bytes)) = main.first() else { return Err(EngineError::Other("the export wrote no file".into())) };
    let mut out = match path {
        Some(_) => json!({ "path": first, "bytes": bytes.len() }),
        None => json!({ "dataBase64": vectorcraft_format::base64_encode(bytes), "bytes": bytes.len() }),
    };
    if main.len() > 1 {
        out["files"] = main.iter().map(row).collect();
    }
    if !linked.is_empty() {
        out["linked"] = linked.iter().map(row).collect();
    }
    Ok(merge(out, extra))
}

/// `a` with the fields of `b`.
pub(crate) fn merge(mut a: Value, b: Value) -> Value {
    if let (Some(a), Value::Object(b)) = (a.as_object_mut(), b) {
        a.extend(b);
    }
    a
}

/// Appearance of Black › Printing / Exporting (#394): Output All Blacks as Rich Black makes RGB
/// raster exports draw pure CMYK black as RGB black (`richBlack` on the encoder params).
pub(crate) fn with_black_output(s: &Session, p: &Value) -> Value {
    if s.prefs.black_output == "rich" { merge(p.clone(), json!({ "richBlack": true })) } else { p.clone() }
}

/// A file name for bytes handed back without a path: the document's title with `ext`.
pub(crate) fn default_name(doc: &vectorcraft_doc::Document, ext: &str) -> String {
    let stem = std::path::Path::new(&doc.title).file_stem().map(|s| s.to_string_lossy().into_owned()).filter(|s| !s.is_empty());
    format!("{}.{ext}", stem.as_deref().unwrap_or("Untitled"))
}

/// Write `bytes` to `path` → `{path, bytes, …extra}`; with no path → `{dataBase64, bytes, …extra}`.
pub(crate) fn write_or_return(path: Option<&str>, bytes: &[u8], extra: Value) -> Result<Value> {
    let out = match path {
        Some(path) => {
            write_file(path, bytes)?;
            json!({ "path": path, "bytes": bytes.len() })
        }
        None => json!({ "dataBase64": vectorcraft_format::base64_encode(bytes), "bytes": bytes.len() }),
    };
    Ok(merge(out, extra))
}
