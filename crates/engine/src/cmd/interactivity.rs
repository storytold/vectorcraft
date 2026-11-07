//! Window → SVG Interactivity: the JavaScript an object runs on each event in web output
//! (`svgInteractivity.setEvent`, `removeEvent`, `clearEvents`) and the JavaScript files the
//! document links (`svgInteractivity.addFile`, `removeFile`). SVG export writes the events as
//! event attributes (`onclick="…"`) and the files as `<script xlink:href>` elements; SVG import
//! reads both back as text. The panel reads everything with `svgInteractivity.info`.

use serde_json::{Value, json};
use vectorcraft_doc::{Document, MAX_SVG_SCRIPTS, NodeId, SVG_EVENTS};

use super::overprint::edit_counted;
use super::{CommandSpec, Result, Session, bad, cmd, has_doc, str_param, targets};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "svgInteractivity.info",
            "SVG Interactivity",
            [],
            None,
            "{ids?|id?} → {ids, events: [{event, script}…] in the panel's order | null (the objects' events differ), files: [the linked JavaScript files…], eventNames: [the events the panel lists…]} the SVG Interactivity panel's values for ids or the selection",
            has_doc,
            info
        ),
        cmd!(
            "svgInteractivity.setEvent",
            "Add Event",
            [],
            None,
            "{event: \"onclick\"|\"onmouseover\"|… (svgInteractivity.info's eventNames), script: string (the JavaScript; \"\" removes the event), ids?|id?} set the JavaScript an event runs on ids or the selection, replacing the event's script where it has one, as one undo step; SVG export writes it as the event attribute → {changed: objects}",
            has_doc,
            set_event
        ),
        cmd!(
            "svgInteractivity.removeEvent",
            "Remove Selected Entry",
            [],
            None,
            "{event, ids?|id?} remove an event and its JavaScript from ids or the selection, as one undo step → {changed: objects}",
            has_doc,
            remove_event
        ),
        cmd!(
            "svgInteractivity.clearEvents",
            "Clear Events",
            [],
            None,
            "{ids?|id?} remove every event from ids or the selection, as one undo step → {changed: objects}",
            has_doc,
            clear_events
        ),
        cmd!(
            "svgInteractivity.addFile",
            "Add JavaScript File",
            [],
            None,
            "{file: path or URL (written as given, without surrounding spaces)} link a JavaScript file to the document, after the others: SVG export writes a <script xlink:href> for each linked file. A file already linked stays where it is; a document links at most 64 files → {files}",
            has_doc,
            add_file
        ),
        cmd!(
            "svgInteractivity.removeFile",
            "Remove JavaScript File",
            [],
            None,
            "{file} unlink a JavaScript file from the document → {files}",
            has_doc,
            remove_file
        ),
    ]
}

/// `p`'s `event`, one of [`SVG_EVENTS`] (any case).
fn event_param(p: &Value, cmd: &str) -> Result<&'static str> {
    let e = str_param(p, "event").ok_or_else(|| bad(cmd, "give `event`, such as onclick"))?;
    SVG_EVENTS
        .iter()
        .find(|n| n.eq_ignore_ascii_case(e.trim()))
        .copied()
        .ok_or_else(|| bad(cmd, format!("`event` must be one of {}", SVG_EVENTS.join(", "))))
}

/// The events of object `id` in `d` (none for a missing object).
fn events_of(d: &Document, id: NodeId) -> Vec<(String, String)> {
    d.node(id).and_then(|n| n.extras().attrs.as_deref()).map(|a| a.events.clone()).unwrap_or_default()
}

/// Change the events of `ids` (or the selection) with `f`, as one undo step named `label`; an
/// object whose events `f` leaves as they were isn't changed → `{changed: objects}`.
fn edit_events(s: &mut Session, p: &Value, cmd: &str, label: &str, f: impl Fn(&mut Vec<(String, String)>)) -> Result<Value> {
    let ids = targets(s, p)?;
    if ids.is_empty() {
        return Err(bad(cmd, "select the objects (or give their ids)"));
    }
    let changed = edit_counted(s, label, |d| {
        let mut changed = 0;
        for id in &ids {
            let mut events = events_of(d, *id);
            let before = events.clone();
            f(&mut events);
            if events != before
                && let Some(n) = d.node_mut(*id)
            {
                n.edit_attrs(|a| a.events = events);
                changed += 1;
            }
        }
        Ok(changed)
    })?;
    Ok(json!({ "changed": changed }))
}

fn info(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = targets(s, p)?;
    let d = &s.doc()?.doc;
    let lists: Vec<Vec<(String, String)>> = ids.iter().map(|id| events_of(d, *id)).collect();
    let events = lists
        .first()
        .filter(|first| lists.iter().all(|l| l == *first))
        .map(|l| l.iter().map(|(event, script)| json!({ "event": event, "script": script })).collect::<Vec<_>>());
    Ok(json!({
        "ids": ids.iter().map(|id| id.0).collect::<Vec<_>>(),
        "events": events,
        "files": d.svg_scripts,
        "eventNames": SVG_EVENTS,
    }))
}

fn set_event(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "svgInteractivity.setEvent";
    let event = event_param(p, C)?;
    let script = p.get("script").map(|v| v.as_str().ok_or_else(|| bad(C, "`script` must be a string"))).transpose()?;
    let script = script.ok_or_else(|| bad(C, "give `script`, the JavaScript the event runs (\"\" removes the event)"))?.trim().to_string();
    edit_events(s, p, C, "Add Event", |events| {
        if script.is_empty() {
            events.retain(|(e, _)| e != event);
        } else if let Some((_, js)) = events.iter_mut().find(|(e, _)| e == event) {
            js.clone_from(&script);
        } else {
            events.push((event.to_string(), script.clone()));
        }
    })
}

fn remove_event(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "svgInteractivity.removeEvent";
    let event = event_param(p, C)?;
    edit_events(s, p, C, "Remove Selected Entry", |events| events.retain(|(e, _)| e != event))
}

fn clear_events(s: &mut Session, p: &Value) -> Result<Value> {
    edit_events(s, p, "svgInteractivity.clearEvents", "Clear Events", Vec::clear)
}

/// `p`'s `file`, trimmed and not empty.
fn file_param(p: &Value, cmd: &str) -> Result<String> {
    let f = str_param(p, "file").map(str::trim).filter(|f| !f.is_empty());
    f.map(str::to_string).ok_or_else(|| bad(cmd, "give `file`, the JavaScript file's path or URL"))
}

fn files(s: &Session) -> Result<Value> {
    Ok(json!({ "files": s.doc()?.doc.svg_scripts }))
}

fn add_file(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "svgInteractivity.addFile";
    let file = file_param(p, C)?;
    let linked = &s.doc()?.doc.svg_scripts;
    if !linked.contains(&file) {
        if linked.len() >= MAX_SVG_SCRIPTS {
            return Err(bad(C, format!("a document links at most {MAX_SVG_SCRIPTS} JavaScript files")));
        }
        s.edit("Add JavaScript File", |d, _| {
            d.svg_scripts.push(file);
            Ok(())
        })?;
    }
    files(s)
}

fn remove_file(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "svgInteractivity.removeFile";
    let file = file_param(p, C)?;
    if !s.doc()?.doc.svg_scripts.contains(&file) {
        return Err(bad(C, format!("{file:?} isn't linked")));
    }
    s.edit("Remove JavaScript File", |d, _| {
        d.svg_scripts.retain(|f| *f != file);
        Ok(())
    })?;
    files(s)
}
