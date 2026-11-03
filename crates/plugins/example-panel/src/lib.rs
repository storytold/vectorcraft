//! CSV Rows: an example VectorCraft panel plug-in (plug-in ABI v1, see `docs/plugins.md`).
//!
//! Its panel loads a CSV file picked with its file field into the document's data sets
//! (`dataset.import`), places a text frame bound to a variable for each of the file's columns
//! (`text.create`, `variable.bind`), and steps the document through the rows (`dataset.next`,
//! `dataset.prev`), one undo step per click. The document keeps the rows and which one is applied;
//! the plug-in keeps only the file's name and its columns in the `state` the host keeps for it, so
//! an undo never leaves the panel telling of a row the document no longer shows. Only `serde_json`
//! is used: no imports, no WASI.

use serde_json::{Value, json};

const MANIFEST: &str = r#"{
  "id": "org.vectorcraft.example.csv-rows",
  "name": "CSV Rows",
  "version": "1.0.0",
  "kind": "panel",
  "author": "VectorCraft contributors",
  "description": "Steps text bound to the columns of a CSV file through its rows; an example of a panel plug-in."
}"#;

/// Most columns Place Text makes a text frame for: two commands each, and an event runs at most
/// 256.
const MAX_COLUMNS: usize = 32;

#[unsafe(no_mangle)]
pub extern "C" fn vc_abi_version() -> u32 {
    1
}

/// `len << 32 | ptr` of the UTF-8 manifest JSON.
#[unsafe(no_mangle)]
pub extern "C" fn vc_manifest() -> u64 {
    pack(MANIFEST.as_bytes())
}

/// A block of `size` bytes the host writes the input and the parameters into. Never freed: every
/// event gets a fresh instance.
#[unsafe(no_mangle)]
pub extern "C" fn vc_alloc(size: u32) -> u32 {
    let mut block = Vec::<u8>::with_capacity(size as usize);
    let ptr = block.as_mut_ptr();
    std::mem::forget(block);
    ptr as u32
}

/// Answers one event of the panel: `{"event", "state", "values", "objects"}` in, `{"panel",
/// "state", "commands"}` out, or `{"error"}`, which the host shows under the panel.
///
/// # Safety
/// The host guarantees the input block came from `vc_alloc` and holds `input_len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vc_run(input: *const u8, input_len: u32, _params: *const u8, _params_len: u32) -> i64 {
    // SAFETY: see the function's contract.
    let input = unsafe { std::slice::from_raw_parts(input, input_len as usize) };
    let answer = match serde_json::from_slice::<Value>(input) {
        Ok(input) => answer(&input).unwrap_or_else(|e| json!({"error": e})),
        Err(_) => json!({"error": "the panel's input isn't JSON"}),
    };
    output(&answer)
}

/// What the panel shows, keeps and runs after the event `input` hands it.
fn answer(input: &Value) -> Result<Value, String> {
    let event = &input["event"];
    let mut state = if input["state"].is_object() { input["state"].clone() } else { json!({}) };
    let mut commands = Vec::new();
    match (event["type"].as_str(), event["target"].as_str()) {
        (Some("file"), Some("csv")) => {
            let data = event["dataBase64"].as_str().unwrap_or_default();
            let bytes = base64_decode(data).ok_or("the file's bytes didn't decode")?;
            // The host reads the rows, names them as Import Data Sets does, and keeps them in the
            // document. When the import fails, the host keeps the state as it was.
            commands.push(json!({"command": "dataset.import", "params": {"dataBase64": data}}));
            state = json!({"file": event["name"], "columns": columns(&String::from_utf8_lossy(&bytes))});
        }
        (Some("click"), Some("place")) => {
            for (i, column) in columns_of(&state).iter().take(MAX_COLUMNS).enumerate() {
                // The new text frame is selected, so the bind that follows acts on it.
                commands.push(json!({"command": "text.create", "params": {"x": 72, "y": 96 + 36 * i, "text": column, "size": 24}}));
                commands.push(json!({"command": "variable.bind", "params": {"variable": column}}));
            }
        }
        (Some("click"), Some("previous")) => commands.push(json!({"command": "dataset.prev"})),
        (Some("click"), Some("next")) => commands.push(json!({"command": "dataset.next"})),
        _ => {}
    }
    Ok(json!({"panel": panel(&state), "state": state, "commands": commands}))
}

/// The panel's items for what it keeps.
fn panel(state: &Value) -> Value {
    let columns = columns_of(state);
    let file = state["file"].as_str();
    let status = match file {
        Some(file) if columns.is_empty() => format!("{file}: no text columns"),
        Some(file) => format!("{file}: {}", columns.join(", ")),
        None => "Load a CSV whose first row names its columns.".to_string(),
    };
    json!({"items": [
        {"type": "file", "id": "csv", "label": "Load CSV…", "extensions": ["csv", "tsv", "txt"]},
        {"type": "text", "text": status},
        {"type": "button", "id": "place", "label": "Place Text for Each Column", "enabled": !columns.is_empty()},
        {"type": "separator"},
        {"type": "row", "items": [
            {"type": "button", "id": "previous", "label": "◀ Previous Row", "enabled": file.is_some()},
            {"type": "button", "id": "next", "label": "Next Row ▶", "enabled": file.is_some()},
        ]},
    ]})
}

/// The columns the panel keeps.
fn columns_of(state: &Value) -> Vec<String> {
    state["columns"].as_array().map(|c| c.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

/// The text variables the header of CSV `text` names, roughly as Import Data Sets reads them: the
/// cells of its first line that isn't blank, split at tabs when it has one and else at commas,
/// unquoted and trimmed. Left out: a first cell `name` (it names the rows), empty cells, and
/// cells starting with `#`, `@` or `%` (visibility, image and graph variables).
fn columns(text: &str) -> Vec<String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let Some(line) = text.lines().find(|l| !l.trim().is_empty()) else { return Vec::new() };
    let sep = if line.contains('\t') { '\t' } else { ',' };
    let (mut cells, mut cell, mut quoted) = (Vec::new(), String::new(), false);
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                chars.next();
                cell.push('"');
            }
            '"' => quoted = !quoted,
            c if c == sep && !quoted => cells.push(std::mem::take(&mut cell)),
            c => cell.push(c),
        }
    }
    cells.push(cell);
    let named = cells.first().is_some_and(|c| c.trim().eq_ignore_ascii_case("name"));
    cells.iter().skip(usize::from(named)).map(|c| c.trim().to_string()).filter(|c| !c.is_empty() && !c.starts_with(['#', '@', '%'])).collect()
}

/// Standard base64 with padding, as the host encodes a file; `None` for anything else.
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    fn value(c: u8) -> Option<u32> {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        Some(u32::from(v))
    }
    let s = s.as_bytes();
    if !s.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    for chunk in s.chunks(4) {
        let pad = chunk.iter().rev().take_while(|&&c| c == b'=').count();
        if pad > 2 {
            return None;
        }
        let mut n = 0u32;
        for &c in &chunk[..4 - pad] {
            n = n << 6 | value(c)?;
        }
        n <<= 6 * pad as u32;
        out.extend_from_slice(&[(n >> 16) as u8, (n >> 8) as u8, n as u8][..3 - pad]);
    }
    Some(out)
}

/// Hands `v` to the host as JSON.
fn output(v: &Value) -> i64 {
    let Ok(bytes) = serde_json::to_vec(v) else { return -1 };
    pack(bytes.leak()) as i64
}

fn pack(bytes: &[u8]) -> u64 {
    ((bytes.len() as u64) << 32) | bytes.as_ptr() as u64
}
