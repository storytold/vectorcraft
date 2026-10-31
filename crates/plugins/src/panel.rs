//! Panel plug-ins: a plug-in of kind `panel` has a panel of its own. It declares what the panel
//! shows, and answers each event of the panel (it opened, a button was clicked, a field was set, a
//! file was picked) with what the panel shows now, what it keeps until its next event and the host
//! commands to run.
//!
//! A plug-in still keeps nothing between runs: each event runs a fresh instance, as a filter's run
//! does. The host keeps the panel's `state` (at most [`MAX_STATE`] bytes of JSON) and hands it back
//! with the next event, as it keeps a filter's `documentData`. The commands are the host's own
//! (`shape.rectangle`, `object.group`…): which of them a panel may run is the host's to decide, so
//! this crate leaves them unchecked.

use serde_json::{Value, json};

use crate::objects::declined;
use crate::{Error, Kind, Plugin, Result};

/// Largest state a panel keeps between its events, in bytes of JSON.
pub const MAX_STATE: usize = 4 << 20;

/// Most commands one event runs.
pub const MAX_COMMANDS: usize = 256;

/// Longest field id an event names, in bytes.
pub const MAX_TARGET: usize = 64;

/// Largest file a `file` event hands a panel, in bytes.
pub const MAX_FILE: usize = 4 << 20;

/// Longest file name a `file` event gives, in bytes of UTF-8 (a name of 255 characters, as file
/// systems allow, takes up to 765 of them).
pub const MAX_FILE_NAME: usize = 1024;

/// What happened in a panel.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The panel opened: the plug-in says what it shows.
    Open,
    /// Button `target` was clicked.
    Click { target: String },
    /// Field `target` was set to `value`.
    Change { target: String, value: Value },
    /// A file was picked with file field `target`: its `name`, without its folder, and its bytes,
    /// base64 encoded. The host reads the file and checks the base64 (this crate has no base64 of
    /// its own); the plug-in never learns where the file is.
    File { target: String, name: String, data_base64: String },
}

impl Event {
    /// The event `v` describes: `{"type": "open"}`, `{"type": "click", "target"}`,
    /// `{"type": "change", "target", "value"}` or `{"type": "file", "target", "name", "dataBase64"}`.
    pub fn parse(v: &Value) -> std::result::Result<Event, String> {
        let target = || match v.get("target").and_then(Value::as_str) {
            Some(t) if !t.is_empty() && t.len() <= MAX_TARGET => Ok(t.to_string()),
            _ => Err(format!("`event.target` must name the field, in 1-{MAX_TARGET} bytes")),
        };
        match v.get("type").and_then(Value::as_str) {
            Some("open") => Ok(Event::Open),
            Some("click") => Ok(Event::Click { target: target()? }),
            Some("change") => Ok(Event::Change { target: target()?, value: v.get("value").cloned().unwrap_or(Value::Null) }),
            Some("file") => {
                let target = target()?;
                let name = match v.get("name").and_then(Value::as_str) {
                    Some(n) if !n.is_empty() && n.len() <= MAX_FILE_NAME && !n.contains(['/', '\\', '\0']) => n.to_string(),
                    _ => return Err(format!("`event.name` must be the file's name, without its folder, in 1-{MAX_FILE_NAME} bytes")),
                };
                // Four characters for every three bytes, the last group padded.
                let data_base64 = match v.get("dataBase64").and_then(Value::as_str) {
                    Some(d) if d.len() <= MAX_FILE.div_ceil(3) * 4 => d.to_string(),
                    Some(_) => return Err(format!("the file is larger than {MAX_FILE} bytes")),
                    None => return Err("`event.dataBase64` must be the file's bytes, base64 encoded".into()),
                };
                Ok(Event::File { target, name, data_base64 })
            }
            _ => Err("`event.type` must be \"open\", \"click\", \"change\" or \"file\"".into()),
        }
    }

    /// The event as the plug-in is given it.
    pub fn to_json(&self) -> Value {
        match self {
            Event::Open => json!({"type": "open"}),
            Event::Click { target } => json!({"type": "click", "target": target}),
            Event::Change { target, value } => json!({"type": "change", "target": target, "value": value}),
            Event::File { target, name, data_base64 } => json!({"type": "file", "target": target, "name": name, "dataBase64": data_base64}),
        }
    }
}

/// What a panel is given with an event.
#[derive(Clone, Debug, PartialEq)]
pub struct Given<'a> {
    pub event: &'a Event,
    /// What it kept at its last event (`null` before it keeps anything).
    pub state: &'a Value,
    /// The panel's fields as they are, by field id.
    pub values: &'a Value,
    /// The selected paths and compound paths, as a filter is given them
    /// ([`crate::objects::encode_objects`]).
    pub objects: Vec<Value>,
    /// What it keeps with the active document (`null` without one, or when it keeps nothing).
    pub document_data: Value,
}

/// A host command a panel asks to run.
#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    pub command: String,
    pub params: Value,
}

/// What a panel answered.
#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    /// What the panel shows now, which the host draws.
    pub panel: Value,
    /// What it keeps until its next event: `None` keeps what it kept, `Some(null)` forgets it.
    pub state: Option<Value>,
    /// The host commands to run, in order.
    pub commands: Vec<Command>,
}

/// Runs panel `plugin` on the event `given` describes.
pub fn run(plugin: &Plugin, given: Given) -> Result<Answer> {
    let m = plugin.manifest();
    if m.kind != Kind::Panel {
        return Err(Error::Params(format!("`{}` is a {}, not a panel", m.id, m.kind.name())));
    }
    let input = json!({"event": given.event.to_json(), "state": given.state, "values": given.values, "objects": given.objects});
    let input = serde_json::to_vec(&input).map_err(|e| Error::Params(e.to_string()))?;
    let mut params = Value::Object(m.resolve_params(&Value::Null)?);
    params["_context"] = json!({"mode": "panel", "documentData": given.document_data});
    let params = serde_json::to_vec(&params).map_err(|e| Error::Params(e.to_string()))?;
    decode(&plugin.run(&input, &params)?)
}

/// A panel's output: `{"panel": {…}, "state"?: …, "commands"?: [{"command", "params"?}]}`, or
/// `{"error": "…"}` to decline with a message for the user.
pub fn decode(bytes: &[u8]) -> Result<Answer> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| Error::Output(format!("not JSON: {e}")))?;
    if let Some(e) = declined(&v) {
        return Err(e);
    }
    let panel = v.get("panel").filter(|p| p.is_object()).cloned().ok_or_else(|| Error::Output("expected {\"panel\": {…}}".into()))?;
    let state = v.get("state").cloned();
    if let Some(s) = &state {
        let size = serde_json::to_vec(s).map_or(usize::MAX, |b| b.len());
        if size > MAX_STATE {
            return Err(Error::Output(format!("`state` is {size} bytes of JSON; at most {MAX_STATE}")));
        }
    }
    let commands = match v.get("commands") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(list)) if list.len() <= MAX_COMMANDS => list.iter().enumerate().map(|(i, c)| command(i, c)).collect::<Result<_>>()?,
        Some(Value::Array(_)) => return Err(Error::Output(format!("at most {MAX_COMMANDS} `commands`"))),
        Some(_) => return Err(Error::Output("`commands` must be a list".into())),
    };
    Ok(Answer { panel, state, commands })
}

/// Command `i` of a panel's output: `{"command": "<id>", "params"?: {…}}`.
fn command(i: usize, c: &Value) -> Result<Command> {
    let bad = |why: &str| Error::Output(format!("commands[{i}]: {why}"));
    let id = c.get("command").and_then(Value::as_str).filter(|id| !id.is_empty()).ok_or_else(|| bad("expected {\"command\": \"<id>\"}"))?;
    let params = match c.get("params") {
        None | Some(Value::Null) => json!({}),
        Some(p @ Value::Object(_)) => p.clone(),
        Some(_) => return Err(bad("`params` must be an object")),
    };
    Ok(Command { command: id.to_string(), params })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_are_open_click_or_change() {
        assert_eq!(Event::parse(&json!({"type": "open"})), Ok(Event::Open));
        assert_eq!(Event::parse(&json!({"type": "click", "target": "add"})), Ok(Event::Click { target: "add".into() }));
        let change = Event::parse(&json!({"type": "change", "target": "width", "value": 40})).unwrap();
        assert_eq!(change, Event::Change { target: "width".into(), value: json!(40) });
        assert_eq!(change.to_json(), json!({"type": "change", "target": "width", "value": 40}));
        let long = "x".repeat(MAX_TARGET + 1);
        for bad in [
            json!(null),
            json!({}),
            json!({"type": "drag"}),
            json!({"type": "click"}),
            json!({"type": "click", "target": ""}),
            json!({"type": "change", "target": long}),
        ] {
            assert!(Event::parse(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_file_event_gives_the_name_and_the_bytes() {
        let v = json!({"type": "file", "target": "pick", "name": "data.csv", "dataBase64": "YSxi"});
        let file = Event::parse(&v).unwrap();
        assert_eq!(file, Event::File { target: "pick".into(), name: "data.csv".into(), data_base64: "YSxi".into() });
        assert_eq!(file.to_json(), v);
        // An empty file is a file.
        assert!(Event::parse(&json!({"type": "file", "target": "pick", "name": "empty.txt", "dataBase64": ""})).is_ok());
        let big = "A".repeat(MAX_FILE.div_ceil(3) * 4 + 4);
        let e = Event::parse(&json!({"type": "file", "target": "pick", "name": "big.bin", "dataBase64": big})).unwrap_err();
        assert!(e.contains("larger than"), "{e}");
        for bad in [
            json!({"type": "file", "name": "a.csv", "dataBase64": ""}),
            json!({"type": "file", "target": "pick", "dataBase64": ""}),
            json!({"type": "file", "target": "pick", "name": "", "dataBase64": ""}),
            json!({"type": "file", "target": "pick", "name": "/Users/me/a.csv", "dataBase64": ""}),
            json!({"type": "file", "target": "pick", "name": "..\\a.csv", "dataBase64": ""}),
            json!({"type": "file", "target": "pick", "name": "x".repeat(MAX_FILE_NAME + 1), "dataBase64": ""}),
            json!({"type": "file", "target": "pick", "name": "a.csv"}),
            json!({"type": "file", "target": "pick", "name": "a.csv", "dataBase64": [1, 2]}),
        ] {
            assert!(Event::parse(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn output_is_the_panel_its_state_and_commands_or_a_decline() {
        let out = decode(br#"{"panel": {"items": []}, "state": {"n": 1}, "commands": [{"command": "shape.rectangle", "params": {"width": 5}}, {"command": "object.group"}]}"#).unwrap();
        assert_eq!(out.panel, json!({"items": []}));
        assert_eq!(out.state, Some(json!({"n": 1})));
        assert_eq!(
            out.commands,
            vec![
                Command { command: "shape.rectangle".into(), params: json!({"width": 5}) },
                Command { command: "object.group".into(), params: json!({}) },
            ]
        );
        // No state keeps the last one; null forgets it.
        let out = decode(br#"{"panel": {}}"#).unwrap();
        assert_eq!((out.state, out.commands), (None, vec![]));
        assert_eq!(decode(br#"{"panel": {}, "state": null}"#).unwrap().state, Some(Value::Null));
        assert_eq!(decode(br#"{"error": "No\nselection."}"#), Err(Error::Failed("Noselection.".into())));
        for bad in [
            &b"not json"[..],
            b"[]",
            b"{}",
            br#"{"panel": []}"#,
            br#"{"panel": {}, "commands": {}}"#,
            br#"{"panel": {}, "commands": [{}]}"#,
            br#"{"panel": {}, "commands": [{"command": ""}]}"#,
            br#"{"panel": {}, "commands": [{"command": "x", "params": [1]}]}"#,
        ] {
            assert!(matches!(decode(bad), Err(Error::Output(_))), "{}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn state_and_commands_are_capped() {
        let big = "x".repeat(MAX_STATE);
        assert!(matches!(decode(json!({"panel": {}, "state": big}).to_string().as_bytes()), Err(Error::Output(_))));
        let many: Vec<Value> = (0..=MAX_COMMANDS).map(|_| json!({"command": "select.none"})).collect();
        assert!(matches!(decode(json!({"panel": {}, "commands": many}).to_string().as_bytes()), Err(Error::Output(_))));
    }
}
