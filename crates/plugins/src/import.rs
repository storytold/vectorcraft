//! Importer plug-ins: a plug-in of kind `importer` reads files in a format of its own, such as a
//! pen plotter's or a cutting machine's, into a new document.
//!
//! File › Open opens a file with an importer only when no built-in format reads it: the built-in
//! formats are found by the file's content first and then by its extension, and an importer is
//! picked by an extension it lists. When several importers list the extension, the one whose id
//! sorts first opens it ([`for_extension`]). File › Open With and `document.open {importer}` pick an
//! importer by its format id (`plugin.<id>`), also for an extension a built-in format reads.
//!
//! The host hands the importer the file's bytes as they are, at most [`MAX_INPUT_BYTES`] of them,
//! and reads back the objects of the new document in the same form filters return them
//! ([`crate::objects`]): `{"objects": [...], "warnings"?: ["…"]}`, or `{"error": "…"}` to decline.
//! An import is one run the user asked for, so it gets an export's budget ([`crate::export::FUEL`]
//! and [`crate::export::WALL_TIME_MS`]), and Cancel stops it between two slices of the budget
//! ([`Error::Cancelled`]).

use std::sync::Arc;

use serde_json::{Value, json};

use crate::objects::{OutObject, declined, printable};
use crate::{Error, Kind, Limits, Plugin, Result, registry};

/// The largest file an importer is given, in bytes: 128 MiB, a quarter of a plug-in's memory
/// limit, so the file and the objects made from it fit beside each other.
pub const MAX_INPUT_BYTES: usize = 128 << 20;

/// The keys an importer's output may have.
const OUTPUT_KEYS: &[&str] = &["objects", "warnings", "error"];

/// The installed importers, by plug-in id.
pub fn installed() -> Vec<Arc<Plugin>> {
    registry::list().into_iter().filter(|p| p.manifest().kind == Kind::Importer).collect()
}

/// The installed importer of format id `format_id` (`plugin.<id>`).
pub fn by_format(format_id: &str) -> Option<Arc<Plugin>> {
    registry::get(crate::export::plugin_id(format_id)?).filter(|p| p.manifest().kind == Kind::Importer)
}

/// The installed importer `importer` names: its format id (`plugin.<id>`) or its plug-in id. A
/// plug-in of another kind, and one that isn't installed, are errors that say so.
pub fn named(importer: &str) -> Result<Arc<Plugin>> {
    let id = crate::export::plugin_id(importer).unwrap_or(importer);
    let plugin = registry::get(id).ok_or_else(|| Error::NotFound(id.into()))?;
    match plugin.manifest().kind {
        Kind::Importer => Ok(plugin),
        _ => Err(not_importer(plugin.manifest())),
    }
}

/// The error of plug-in `m`, which is not an importer, asked to import.
fn not_importer(m: &crate::Manifest) -> Error {
    Error::Params(format!("`{}` is a plug-in of kind `{}`, not an importer", m.id, m.kind.name()))
}

/// The installed importers that list extension `ext` (any case, leading dot allowed), by plug-in
/// id: the first is the one File › Open uses.
pub fn for_extension(ext: &str) -> Vec<Arc<Plugin>> {
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    installed().into_iter().filter(|p| p.manifest().extensions.contains(&ext)).collect()
}

/// What an importer returned.
#[derive(Clone, Debug, PartialEq)]
pub struct Imported {
    /// The objects of the new document, in paint order (bottom first), checked as a filter's are.
    pub objects: Vec<OutObject>,
    /// What the import left out or changed, for the user (at most
    /// [`crate::export::MAX_WARNINGS`]).
    pub warnings: Vec<String>,
}

/// The file an import reads, as the importer is told in `_context`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source<'a> {
    /// Its extension, without the dot: one the importer lists.
    pub extension: &'a str,
    /// Its name without the folder and extension.
    pub name: &'a str,
}

/// The error of a file larger than [`MAX_INPUT_BYTES`], which no importer is given.
pub fn too_large() -> Error {
    Error::Io(format!("the file is larger than the {MAX_INPUT_BYTES} bytes an importer reads"))
}

/// The limits of an import by `plugin`: its own, with the budgets of an export.
pub fn limits(plugin: &Plugin) -> Limits {
    crate::export::limits(plugin)
}

/// Runs importer `plugin` on `file`, the bytes of the file being opened, with `params` (checked
/// against its manifest: missing ones take their defaults) for the file `source`.
pub fn run(plugin: &Plugin, file: &[u8], params: &Value, source: Source) -> Result<Imported> {
    let m = plugin.manifest();
    if m.kind != Kind::Importer {
        return Err(not_importer(m));
    }
    if file.len() > MAX_INPUT_BYTES {
        return Err(too_large());
    }
    let mut params = Value::Object(m.resolve_params(params)?);
    params["_context"] = json!({"mode": "import", "extension": source.extension, "name": source.name});
    let params = serde_json::to_vec(&params).map_err(|e| Error::Params(e.to_string()))?;
    decode(&plugin.run_limited(file, &params, &limits(plugin))?)
}

/// An importer's output: `{"objects": [...], "warnings"?: ["…"]}`, or `{"error": "…"}` to decline
/// with a message for the user. The objects are new ones (no `id`), checked as a filter's are; an
/// output without any, or with keys of its own, is refused.
pub fn decode(bytes: &[u8]) -> Result<Imported> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| Error::Output(format!("not JSON: {e}")))?;
    if let Some(e) = declined(&v) {
        return Err(e);
    }
    if let Some(key) = v.as_object().and_then(|o| o.keys().find(|k| !OUTPUT_KEYS.contains(&k.as_str()))) {
        let key = printable(&json!(key), 64);
        return Err(Error::Output(format!("an importer returns `objects` and `warnings`, not `{key}`")));
    }
    let objects = crate::objects::decode(bytes)?;
    if objects.is_empty() {
        return Err(Error::Output("the importer returned no objects".into()));
    }
    let warnings = match v.get("warnings") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(w)) => w.iter().take(crate::export::MAX_WARNINGS).map(|w| printable(w, crate::export::MAX_WARNING_CHARS)).collect(),
        Some(_) => return Err(Error::Output("`warnings` must be a list of strings".into())),
    };
    Ok(Imported { objects, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQUARE: &str = r#"{"type":"path","path":{"subpaths":[{"closed":true,"anchors":[{"p":[0,0]},{"p":[10,0]},{"p":[10,10]}]}]}}"#;

    #[test]
    fn output_is_the_objects_and_the_warnings_or_a_decline() {
        let out = decode(format!(r#"{{"objects":[{SQUARE}],"warnings":["a",2]}}"#).as_bytes()).unwrap();
        assert_eq!((out.objects.len(), out.warnings), (1, vec!["a".to_string(), "2".to_string()]));
        assert!(out.objects.iter().all(|o| o.id.is_none() && o.path.is_some()));
        assert_eq!(decode(format!(r#"{{"objects":[{SQUARE}]}}"#).as_bytes()).unwrap().warnings, Vec::<String>::new());
        assert_eq!(decode(br#"{"error": "Not a\nplot file."}"#), Err(Error::Failed("Not aplot file.".into())));
        // A decline is reported as one, also with keys the host doesn't read beside it.
        assert_eq!(decode(br#"{"error": "No.", "layers": []}"#), Err(Error::Failed("No.".into())));
    }

    /// Nothing an importer returns reaches a document unchecked: malformed JSON, no objects, keys
    /// the host doesn't read, bad warnings and bad objects (coordinates out of range, unknown paints)
    /// are all errors.
    #[test]
    fn bad_output_is_refused() {
        let far = r#"{"objects":[{"path":{"subpaths":[{"closed":false,"anchors":[{"p":[0,0]},{"p":[1e9,0]}]}]}}]}"#;
        for (bad, says) in [
            (&b"[]"[..], "objects"),
            (b"{}", "objects"),
            (b"not json", "not JSON"),
            (br#"{"objects":[]}"#, "no objects"),
            (br#"{"objects":{}}"#, "objects"),
            (br#"{"objects":[1]}"#, "JSON object"),
            (far.as_bytes(), "out of range"),
            (br#"{"objects":[{"path":{"subpaths":[]},"fills":[{"type":"hatch"}]}]}"#, "objects[0]"),
            (br#"{"objects":[],"artboard":[0,0,1,1]}"#, "not `artboard`"),
        ] {
            let e = decode(bad).unwrap_err().to_string();
            assert!(e.contains(says), "{}: {e}", String::from_utf8_lossy(bad));
        }
        let warns = format!(r#"{{"objects":[{SQUARE}],"warnings":"x"}}"#);
        assert!(decode(warns.as_bytes()).unwrap_err().to_string().contains("`warnings`"));
    }

    #[test]
    fn warnings_are_capped_in_number_and_length() {
        let many: Vec<String> = (0..crate::export::MAX_WARNINGS + 5).map(|i| format!("w{i}")).collect();
        let square: Value = serde_json::from_str(SQUARE).unwrap();
        let out = decode(json!({"objects": [square], "warnings": many}).to_string().as_bytes()).unwrap();
        assert_eq!(out.warnings.len(), crate::export::MAX_WARNINGS);
        let long = "x".repeat(crate::export::MAX_WARNING_CHARS + 1);
        let out = decode(json!({"objects": [square], "warnings": [long]}).to_string().as_bytes()).unwrap();
        assert_eq!(out.warnings.first().map(|w| w.chars().count()), Some(crate::export::MAX_WARNING_CHARS));
    }
}
