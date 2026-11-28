//! Edit › Find and Replace…: the framework's Find and Replace dialog on the dialog host, with the
//! Find what and Replace with fields, Match Case, Find Whole Word, Search Backwards, Check Hidden
//! Layers and Check Locked Layers, and the Find Next, Replace, Replace & Find, Replace All and Close
//! buttons, as "Find and replace text" in the public
//! [reference](https://helpx.adobe.com/pdf/illustrator_reference.pdf) describes them. The dialog
//! stays open: Find Next goes to the next match (`edit.findNext`), Replace changes that match
//! (`edit.replaceMatch`), Replace & Find changes it and goes to the next, and Replace All changes
//! every match (`edit.findReplace`). Each Replace is one undo step, and so is Replace All.
//!
//! These rules are VectorCraft's own. A match is shown under the fields in its line of text with its
//! place among the matches, and its type object is selected, unless it is on a hidden or locked
//! layer. Replace and Replace & Find wait for a match that Find Next reached with the Find text and
//! the options as they are now. Edit › Find Next goes on from the last search while the dialog is
//! closed, and opens the dialog when there has been none.
//!
//! The walk lives in `UiState::last_find`: the Find text and the switches of the last request, its
//! `replace` text and `backwards`, the place it reached in `from` (`{id, start, end}`), the drawing's
//! UID and Revert counter in `doc`, and the match in `found` (`{index, count}`). Changing the
//! loaded drawing, Find text, or switches restarts the search. [`pump`] runs the requests each frame.

use craft::text::find::FindOptions;
use craft::text::ui::find::{FindCheck, FindReplaceDialog, FindRequest, FindTab, ReplaceButtons};
use craft::ui::dialog::StackDialog;
use serde_json::{Map, Value, json};
use vectorcraft_doc::{NodeId, NodeKind};

use crate::VectorcraftApp;

/// The command of its menu item, which Replace All runs.
pub const COMMAND: &str = "edit.findReplace";

/// The command Find runs, and Edit › Find Next.
pub const NEXT: &str = "edit.findNext";

/// The command Replace runs.
const REPLACE: &str = "edit.replaceMatch";

/// The switches that decide what a match is and which type is searched.
const SWITCHES: [&str; 4] = ["matchCase", "wholeWord", "hiddenLayers", "lockedLayers"];

/// The characters of a match's line shown on either side of it.
const CONTEXT: usize = 24;

/// What the dialog asks for in one frame: the query (the Find text and the [`SWITCHES`]), the
/// Replace with text and the search direction.
struct Form {
    query: Value,
    replace: String,
    backwards: bool,
}

/// The params of the Edit menu's Find and Replace… item: a `find` of null, which gives the dialog
/// no field, so that it opens on the last search.
pub fn menu_params() -> Value {
    json!({"find": null})
}

/// The active drawing and its loaded generation.
fn active_drawing(app: &VectorcraftApp) -> Option<(u64, u64)> {
    app.session.active().map(|d| (d.uid, d.reverts))
}

fn saved_drawing(last: &Map<String, Value>) -> Option<(u64, u64)> {
    let [uid, reverts] = last.get("doc")?.as_array()?.as_slice() else { return None };
    Some((uid.as_u64()?, reverts.as_u64()?))
}

fn same_drawing(last: &Map<String, Value>, drawing: (u64, u64)) -> bool {
    saved_drawing(last) == Some(drawing)
}

/// The match found belongs to an open dialog: the last search keeps its place, not the match.
fn forget_found(app: &mut VectorcraftApp) {
    if let Some(last) = app.ui.last_find.as_mut().and_then(Value::as_object_mut) {
        last.remove("found");
    }
}

/// Open Find and Replace on the last search when there has been one, with the fields that
/// `params` gives in its place. A null value, such as the menu item's `find` ([`menu_params`]),
/// gives none. A field given a value of another type is an error, and the dialog does not open.
pub fn open(app: &mut VectorcraftApp, params: &Map<String, Value>) -> Result<Value, String> {
    let mut fields = Map::new();
    fields.insert("find".into(), json!(""));
    fields.insert("replace".into(), json!(""));
    for key in SWITCHES.into_iter().chain(["backwards"]) {
        fields.insert(key.into(), json!(false));
    }
    check_types(params, true)?;
    let last = app.ui.last_find.as_ref().and_then(Value::as_object).cloned().unwrap_or_default();
    for (k, v) in last.iter().chain(params) {
        // Only the dialog's own fields, each of its own type.
        if fields.get(k).is_some_and(|f| f.is_string() == v.is_string() && f.is_boolean() == v.is_boolean()) {
            fields.insert(k.clone(), v.clone());
        }
    }
    let text = |key: &str| fields.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let flag = |key: &str| fields.get(key).and_then(Value::as_bool).unwrap_or(false);
    // The framework's dialog is modeless; the document stays editable under it.
    let dialog = FindReplaceDialog::new(text("find"), FindTab::Replace)
        .word(false)
        .replace_with(text("replace"))
        .options(FindOptions { match_case: flag("matchCase"), whole_word: flag("wholeWord"), ..Default::default() })
        .checks(vec![
            FindCheck::new("backwards", tl!("Search Backwards"), flag("backwards")),
            FindCheck::new("hiddenLayers", tl!("Check Hidden Layers"), flag("hiddenLayers")),
            FindCheck::new("lockedLayers", tl!("Check Locked Layers"), flag("lockedLayers")),
        ])
        .replace_buttons(ReplaceButtons::Separate);
    let dialogs = app.dialogs().ok_or("no window")?;
    forget_found(app);
    dialogs.with(|host| host.open(dialog));
    Ok(Value::Null)
}

/// The first of the dialog's fields in `fields` that holds a value of another type than its own,
/// as an error: `find` and `replace` are strings, and `backwards` and the [`SWITCHES`] are true or
/// false. A null value is no error when `null_gives_none` is true.
fn check_types(fields: &Map<String, Value>, null_gives_none: bool) -> Result<(), String> {
    for (k, v) in fields.iter().filter(|(_, v)| !(null_gives_none && v.is_null())) {
        if matches!(k.as_str(), "find" | "replace") && !v.is_string() {
            return Err(format!("`{k}` must be a string"));
        }
        if (k == "backwards" || SWITCHES.contains(&k.as_str())) && !v.is_boolean() {
            return Err(format!("`{k}` must be true or false"));
        }
    }
    Ok(())
}

/// Edit › Find Next with no text to find: the next match of the last search, from the place it
/// reached, with the options the dialog had. With no search yet, it opens the dialog.
pub fn find_next(app: &mut VectorcraftApp) -> Result<Value, String> {
    let Some(mut last) = app.ui.last_find.clone().filter(|l| l.get("find").and_then(Value::as_str).is_some_and(|f| !f.is_empty())) else {
        return open(app, &Map::new());
    };
    forget_found(app);
    let drawing = active_drawing(app);
    if let Some(o) = last.as_object_mut() {
        let saved = saved_drawing(o);
        o.remove("replace");
        o.remove("found");
        o.remove("doc");
        if saved != drawing || drawing.is_none() {
            let restored = saved.zip(drawing).is_some_and(|((old_uid, _), (uid, _))| old_uid == uid);
            if restored {
                // An explicit start ignores the whole text frame currently selected.
                o.insert("from".into(), Value::Null);
            } else {
                o.remove("from");
            }
        }
    }
    let r = app.run(NEXT, last)?;
    if r["id"].is_null() {
        app.status("No matches found");
        return Ok(r);
    }
    if let Some(l) = app.ui.last_find.as_mut() {
        l["from"] = json!({ "id": r["id"], "start": r["start"], "end": r["end"] });
        l["doc"] = json!(drawing);
    }
    Ok(r)
}

/// The Find text and the switches of [`SWITCHES`]: what a match is, as params, from the dialog's
/// fields or from the last search.
fn query_in(fields: &Map<String, Value>) -> Value {
    let mut q = json!({ "find": fields.get("find").and_then(Value::as_str).unwrap_or_default() });
    for key in SWITCHES {
        q[key] = json!(fields.get(key).and_then(Value::as_bool).unwrap_or(false));
    }
    q
}

/// What the open dialog asks for now.
fn form(d: &FindReplaceDialog) -> Form {
    let fields = d.json_fields().unwrap_or(Value::Null);
    let text = |key: &str| fields.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let on = |key: &str| fields.get(key).and_then(Value::as_bool).unwrap_or(false);
    let check = |key: &str| d.checked(key).unwrap_or(false);
    let mut shown = Map::new();
    shown.insert("find".into(), json!(text("find")));
    for key in ["matchCase", "wholeWord"] {
        shown.insert(key.into(), json!(on(key)));
    }
    for key in ["hiddenLayers", "lockedLayers"] {
        shown.insert(key.into(), json!(check(key)));
    }
    Form { query: query_in(&shown), replace: text("replace"), backwards: check("backwards") }
}

/// The place the last search reached, `{id, start, end}`, while its document is the active one and
/// the Find text and the switches are `query`, those it was reached with.
fn place<'a>(app: &VectorcraftApp, last: &'a Value, query: &Value) -> Option<&'a Value> {
    let drawing = active_drawing(app)?;
    let last = last.as_object()?;
    let same = same_drawing(last, drawing) && query_in(last) == *query;
    same.then(|| last.get("from")).flatten().filter(|at| at.is_object())
}

/// The match Replace changes, as `{id, start, end}`: the one Find reached, in the active document,
/// while the Find text and the switches are as they were when it was found.
fn current_match<'a>(app: &VectorcraftApp, last: &'a Value, query: &Value) -> Option<&'a Value> {
    last.get("found")?;
    place(app, last, query)
}

/// The place the last search reached under the form's query, to keep when a request moves none.
fn kept_place(app: &VectorcraftApp, form: &Form) -> Option<Value> {
    app.ui.last_find.as_ref().and_then(|last| place(app, last, &form.query)).cloned()
}

/// Keep what a request searched for, the place it reached and the match it found, for Edit › Find
/// Next and for the next time the dialog opens.
fn remember(app: &mut VectorcraftApp, form: &Form, at: Option<Value>, found: Option<Value>) {
    let mut last = form.query.clone();
    last["replace"] = json!(form.replace);
    last["backwards"] = json!(form.backwards);
    if let Some(at) = at {
        last["from"] = at;
        last["doc"] = json!(active_drawing(app));
    }
    if let Some(found) = found {
        last["found"] = found;
    }
    app.ui.last_find = Some(last);
}

/// The note in the current language.
fn said(note: &str) -> String {
    crate::i18n::message(crate::i18n::current(), note).to_string()
}

/// Find: the next match after the place the search reached, kept as that place and as the match
/// Replace changes → the match line, or "No matches found".
fn find(app: &mut VectorcraftApp, form: &Form) -> Result<String, String> {
    let mut p = form.query.clone();
    p["backwards"] = json!(form.backwards);
    // With no place, the search starts at the first match, not after the selected type object.
    p["from"] = kept_place(app, form).unwrap_or(Value::Null);
    let r = app.run(NEXT, p)?;
    if r["id"].is_null() {
        app.status("No matches found");
        let at = kept_place(app, form);
        remember(app, form, at, None);
        return Ok(said("No matches found"));
    }
    let at = json!({ "id": r["id"], "start": r["start"], "end": r["end"] });
    let found = json!({ "index": r["index"], "count": r["count"] });
    let line = shown_match(app, &at, &found).unwrap_or_default();
    remember(app, form, Some(at), Some(found));
    Ok(line)
}

/// Replace: the match Find reached becomes the Replace with text, and the search goes on from it
/// when `find_next`. With no such match, nothing changes.
fn replace(app: &mut VectorcraftApp, form: &Form, find_next: bool) -> Result<String, String> {
    let Some(at) = app.ui.last_find.as_ref().and_then(|last| current_match(app, last, &form.query)).cloned() else {
        return Ok(String::new());
    };
    let mut p = form.query.clone();
    for key in ["hiddenLayers", "lockedLayers"] {
        if let Some(o) = p.as_object_mut() {
            o.remove(key);
        }
    }
    p["replace"] = json!(form.replace);
    for key in ["id", "start", "end"] {
        p[key] = at[key].clone();
    }
    let r = app.run(REPLACE, p)?;
    remember(app, form, Some(json!({ "id": r["id"], "start": r["start"], "end": r["end"] })), None);
    if find_next { find(app, form) } else { Ok(String::new()) }
}

/// Replace All: every match becomes the Replace with text → how many.
fn replace_all(app: &mut VectorcraftApp, form: &Form) -> Result<String, String> {
    let mut p = form.query.clone();
    p["replace"] = json!(form.replace);
    let r = app.run(COMMAND, p)?;
    // The matches that changed: those that already read as the Replace with text do not count.
    let note = match (r["count"].as_u64().unwrap_or(0), r["replaced"].as_u64().unwrap_or(0)) {
        (0, _) => "No matches found".to_string(),
        (_, n) => format!("Matches replaced: {n}"),
    };
    app.status(note.clone());
    let at = kept_place(app, form);
    remember(app, form, at, None);
    Ok(said(&note))
}

/// The match Find reached as it reads in its text: its place among the matches, then the text
/// before it on its line, the match and the text after it, each at most [`CONTEXT`] characters.
fn shown_match(app: &VectorcraftApp, at: &Value, found: &Value) -> Option<String> {
    let whole = |v: &Value| v.as_u64().and_then(|n| usize::try_from(n).ok());
    let (id, start, end) = (at["id"].as_u64()?, whole(&at["start"])?, whole(&at["end"])?);
    let NodeKind::Text(t) = &app.session.active()?.doc.node(NodeId(id))?.kind else { return None };
    let text = t.plain_text();
    let (before, hit, after) = (text.get(..start)?, text.get(start..end)?, text.get(end..)?);
    let before = before.rsplit(['\n', '\r']).next().unwrap_or_default();
    let after = after.split(['\n', '\r']).next().unwrap_or_default();
    let skip = before.chars().count().saturating_sub(CONTEXT);
    let lead = if skip > 0 { "…" } else { "" };
    let tail = if after.chars().count() > CONTEXT { "…" } else { "" };
    let before: String = before.chars().skip(skip).collect();
    let after: String = after.chars().take(CONTEXT).collect();
    Some(format!("{} / {}   {lead}{before}{hit}{after}{tail}", found["index"], found["count"]))
}

/// Refreshes the match line when a drawing or query change makes its match ineligible or current again.
fn refresh_match(app: &mut VectorcraftApp, query: &Value) -> (bool, Option<String>) {
    let Some(last) = app.ui.last_find.as_ref() else { return (false, None) };
    let Some(found) = last.get("found") else { return (false, None) };
    let at = place(app, last, query);
    let matched = at.is_some();
    if found["visible"].as_bool().unwrap_or(true) == matched {
        return (matched, None);
    }
    let message = at.and_then(|at| shown_match(app, at, found)).unwrap_or_default();
    if let Some(found) = app.ui.last_find.as_mut().and_then(|last| last.get_mut("found")) {
        found["visible"] = json!(matched);
    }
    (matched, Some(message))
}

/// Runs the request the dialog made this frame, then tells the dialog its message and whether a
/// match is current. The match is evaluated in every pump, so a changed Find text or switch
/// disables Replace and Replace & Find at once; a pass that has no request only refreshes it.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    // `app.run` must not run inside the host's lock, so the request is taken first.
    let Some((request, form)) = dialogs.with(|host| host.get_mut::<FindReplaceDialog>().map(|d| (d.take_request(), form(d)))) else { return };
    let message = request.and_then(|request| {
        let ran = match request {
            FindRequest::FindNext { .. } => find(app, &form),
            FindRequest::Replace { find_next, .. } => replace(app, &form, find_next),
            FindRequest::ReplaceAll { .. } => replace_all(app, &form),
            _ => return None,
        };
        Some(ran.unwrap_or_else(|e| said(&e)))
    });
    let (matched, match_message) = refresh_match(app, &form.query);
    dialogs.with(|host| {
        if let Some(d) = host.get_mut::<FindReplaceDialog>() {
            if let Some(message) = message.or(match_message) {
                d.set_message(message);
            }
            d.set_matched(matched);
        }
    });
}

#[cfg(test)]
mod tests;
