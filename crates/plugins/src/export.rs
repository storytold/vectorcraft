//! Exporter plug-ins: a plug-in of kind `exporter` writes the document in a file format of its
//! own, such as a pen plotter's or a cutting machine's.
//!
//! File › Export › Export As lists each installed exporter after the built-in formats, under its
//! name, and `document.export` writes it by its format id (`plugin.<id>`) or by one of the
//! extensions its manifest lists. A built-in format keeps its extension: an exporter that lists
//! `svg` too is picked by its format id. The host hands the exporter the document as the native
//! file's JSON (`vectorcraft_format::json_part`: images of `INLINE_MAX` bytes or more without
//! their data) and writes the bytes it returns.
//!
//! An export is one run the user asked for, often of a whole document, so it gets a larger budget
//! than a filter: [`FUEL`] instructions, plus [`Limits::fuel_per_byte`] for each byte of input,
//! and [`WALL_TIME_MS`]. It runs where the export runs: with Background Export on the export's
//! worker, whose Cancel stops it between two slices of the budget ([`Error::Cancelled`]).

use std::sync::Arc;

use serde_json::{Value, json};

use crate::objects::{declined, printable};
use crate::{Error, Kind, Limits, Plugin, Result, registry};

/// Instructions an export may use, besides [`Limits::fuel_per_byte`] for each byte of the
/// document: 200 times a filter's base budget.
pub const FUEL: u64 = 10_000_000_000;

/// Wall-clock budget of one export, in milliseconds (native builds; the web build has no clock,
/// and the instruction budget alone bounds it there).
pub const WALL_TIME_MS: u64 = 300_000;

/// Most warnings an export reports.
pub const MAX_WARNINGS: usize = 100;

/// Longest warning, in characters.
pub const MAX_WARNING_CHARS: usize = 500;

/// The format id of exporter `plugin_id`: `plugin.<id>`.
pub fn format_id(plugin_id: &str) -> String {
    format!("{}{plugin_id}", crate::PREFIX)
}

/// The plug-in id in format id `format_id`.
pub fn plugin_id(format_id: &str) -> Option<&str> {
    format_id.strip_prefix(crate::PREFIX)
}

/// The installed exporters, by plug-in id.
pub fn installed() -> Vec<Arc<Plugin>> {
    registry::list().into_iter().filter(|p| p.manifest().kind == Kind::Exporter).collect()
}

/// The installed exporter of format id `format_id`.
pub fn by_format(format_id: &str) -> Option<Arc<Plugin>> {
    registry::get(plugin_id(format_id)?).filter(|p| p.manifest().kind == Kind::Exporter)
}

/// What an exporter returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exported {
    /// The file, base64 encoded, as the plug-in returned it (the host decodes it: this crate has
    /// no base64 of its own).
    pub data_base64: String,
    /// What the export left out or changed, for the user (at most [`MAX_WARNINGS`]).
    pub warnings: Vec<String>,
}

/// The file an export writes, as the exporter is told in `_context`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target<'a> {
    /// Its extension, without the dot: one the exporter lists.
    pub extension: &'a str,
    /// Its name without the extension (some formats keep a name in their header).
    pub name: &'a str,
}

/// The limits of an export by `plugin`: its own, with the budgets of an export.
pub fn limits(plugin: &Plugin) -> Limits {
    Limits { fuel_base: FUEL, wall_time_ms: WALL_TIME_MS, ..plugin.limits().clone() }
}

/// Runs exporter `plugin` on `document`, the native file's JSON, with `params` (checked against
/// its manifest: missing ones take their defaults) for the file `target`.
pub fn run(plugin: &Plugin, document: &[u8], params: &Value, target: Target) -> Result<Exported> {
    let m = plugin.manifest();
    if m.kind != Kind::Exporter {
        return Err(Error::Params(format!("`{}` is a {}, not an exporter", m.id, m.kind.name())));
    }
    let mut params = Value::Object(m.resolve_params(params)?);
    params["_context"] = json!({"mode": "export", "extension": target.extension, "name": target.name});
    let params = serde_json::to_vec(&params).map_err(|e| Error::Params(e.to_string()))?;
    decode(&plugin.run_limited(document, &params, &limits(plugin))?)
}

/// An exporter's output: `{"dataBase64": "…", "warnings"?: ["…"]}`, or `{"error": "…"}` to decline
/// with a message for the user.
pub fn decode(bytes: &[u8]) -> Result<Exported> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| Error::Output(format!("not JSON: {e}")))?;
    if let Some(e) = declined(&v) {
        return Err(e);
    }
    let data = v.get("dataBase64").and_then(Value::as_str).ok_or_else(|| Error::Output("expected {\"dataBase64\": \"…\"}".into()))?;
    let warnings = match v.get("warnings") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(w)) => w.iter().take(MAX_WARNINGS).map(|w| printable(w, MAX_WARNING_CHARS)).collect(),
        Some(_) => return Err(Error::Output("`warnings` must be a list of strings".into())),
    };
    Ok(Exported { data_base64: data.to_string(), warnings })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_ids_name_the_plug_in() {
        assert_eq!(format_id("org.example.plotter"), "plugin.org.example.plotter");
        assert_eq!(plugin_id("plugin.org.example.plotter"), Some("org.example.plotter"));
        assert_eq!(plugin_id("png"), None);
    }

    #[test]
    fn output_is_the_data_and_the_warnings_or_a_decline() {
        let out = decode(br#"{"dataBase64": "SGk=", "warnings": ["a", 2]}"#).unwrap();
        assert_eq!(out, Exported { data_base64: "SGk=".into(), warnings: vec!["a".into(), "2".into()] });
        assert_eq!(decode(br#"{"dataBase64": ""}"#).unwrap().warnings, Vec::<String>::new());
        assert_eq!(decode(br#"{"error": "Nothing to\nwrite."}"#), Err(Error::Failed("Nothing towrite.".into())));
        for bad in [&b"[]"[..], b"{}", b"not json", br#"{"dataBase64": 1}"#, br#"{"dataBase64": "", "warnings": "x"}"#] {
            assert!(matches!(decode(bad), Err(Error::Output(_))), "{}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn warnings_are_capped_in_number_and_length() {
        let many: Vec<String> = (0..MAX_WARNINGS + 5).map(|i| format!("w{i}")).collect();
        let long = "x".repeat(MAX_WARNING_CHARS + 1);
        let out = decode(json!({"dataBase64": "", "warnings": many}).to_string().as_bytes()).unwrap();
        assert_eq!(out.warnings.len(), MAX_WARNINGS);
        let out = decode(json!({"dataBase64": "", "warnings": [long]}).to_string().as_bytes()).unwrap();
        assert_eq!(out.warnings.first().map(|w| w.chars().count()), Some(MAX_WARNING_CHARS));
    }
}
