//! Edit › Find and Replace…: the Find and Replace With fields, Match Case, Find Whole Word, Search
//! Backwards, Check Hidden Layers and Check Locked Layers, and the Find, Replace, Replace & Find,
//! Replace All and Done buttons, as "Find and replace text" in the public
//! [reference](https://helpx.adobe.com/pdf/illustrator_reference.pdf) describes them. The dialog
//! stays open: Find goes to the next match (`edit.findNext`), Replace changes that match
//! (`edit.replaceMatch`), Replace & Find changes it and goes to the next, and Replace All changes
//! every match (`edit.findReplace`). Each Replace is one undo step, and so is Replace All.
//!
//! These rules are VectorCraft's own. A match is shown in the dialog in its line of text with its
//! place among the matches, and its type object is selected, unless it is on a hidden or locked
//! layer. Enter runs Find. Replace and Replace & Find wait for a match that Find reached with the
//! Find text and the options as they are now. Edit › Find Next goes on from the last search while
//! the dialog is closed, and opens the dialog when there has been none.
//!
//! Fields: `find`, `replace`, `matchCase`, `wholeWord`, `backwards`, `hiddenLayers`,
//! `lockedLayers` and `action` (what `ui.dialog.confirm` does: `find`, `replace`, `replaceFind`,
//! `replaceAll` or `done`; a button sets it and confirms). The dialog keeps the place the search
//! reached in `__at`, the `uid` of its document in `__doc` and the Find text and switches it was
//! reached with in `__atQuery`, the match found in `__found` and the line under the fields in
//! `__note`. A place in a document other than the active one, or reached with other Find text or
//! switches, is not used: the search starts again.

use serde_json::{Map, Value, json};
use vectorcraft_doc::{NodeId, NodeKind};

use super::{DialogAction, DialogSpec, form};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Find and Replace.
pub const KIND: &str = "findReplace";

/// The command of its menu item, which Replace All runs.
pub const COMMAND: &str = "edit.findReplace";

/// The command Find runs, and Edit › Find Next.
pub const NEXT: &str = "edit.findNext";

/// The command Replace runs.
const REPLACE: &str = "edit.replaceMatch";

/// The switches that decide what a match is and which type is searched.
const SWITCHES: [&str; 4] = ["matchCase", "wholeWord", "hiddenLayers", "lockedLayers"];

/// The place the search reached: `{id, start, end}`, a match or the text that replaced one.
const AT: &str = "__at";
/// The match Find reached, which Replace changes: `{index, count, query}`, with `query` the Find
/// text and the switches it was found with.
const FOUND: &str = "__found";
/// The line under the fields, in English: no matches, or how many Replace All replaced.
const NOTE: &str = "__note";

/// The characters of a match's line shown on either side of it.
const CONTEXT: usize = 24;

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Find and Replace").into(),
    body_with_actions: Some(body),
    confirm,
    ok: None,
    min_width: 480.0,
    ..DialogSpec::FORM
};

/// The document a place in [`AT`] or the last search's `from` belongs to: its `uid`.
const DOC: &str = "__doc";
/// The Find text and switches the place in [`AT`] was reached with ([`query`]).
const AT_QUERY: &str = "__atQuery";

/// The params of the Edit menu's Find and Replace… item: a `find` of null, which gives the dialog
/// no field, so that it opens on the last search.
pub fn menu_params() -> Value {
    json!({"find": null})
}

/// The `uid` of the active document.
fn active_uid(app: &VectorcraftApp) -> Option<u64> {
    app.session.active().map(|d| d.uid)
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
    // The place the last search reached, while its document is the active one.
    let uid = active_uid(app);
    if let Some(at) = last.get("from").filter(|v| v.is_object() && uid.is_some() && last.get("doc").and_then(Value::as_u64) == uid) {
        fields.insert(AT.into(), at.clone());
        fields.insert(DOC.into(), json!(uid));
        fields.insert(AT_QUERY.into(), query_in(&last));
    }
    fields.insert("action".into(), json!("find"));
    fields.insert("__command".into(), json!(COMMAND));
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields });
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
    let uid = active_uid(app);
    if let Some(o) = last.as_object_mut() {
        o.remove("replace");
        // A place in another document is no place in this one.
        if uid.is_none() || o.remove("doc").and_then(|d| d.as_u64()) != uid {
            o.remove("from");
        }
    }
    let r = app.run(NEXT, last)?;
    if r["id"].is_null() {
        app.status("No matches found");
        return Ok(r);
    }
    if let Some(l) = app.ui.last_find.as_mut() {
        l["from"] = json!({ "id": r["id"], "start": r["start"], "end": r["end"] });
        l["doc"] = json!(uid);
    }
    Ok(r)
}

/// The Find text and the switches of [`SWITCHES`]: what a match is, as params.
fn query(d: &Dialog) -> Value {
    query_in(&d.fields)
}

/// [`query`] of the dialog fields, or of the last search, in `fields`.
fn query_in(fields: &Map<String, Value>) -> Value {
    let mut q = json!({ "find": fields.get("find").and_then(Value::as_str).unwrap_or_default() });
    for key in SWITCHES {
        q[key] = json!(fields.get(key).and_then(Value::as_bool).unwrap_or(false));
    }
    q
}

/// The place the search reached, `{id, start, end}`, while its document is the active one and
/// the Find text and the switches are those it was reached with.
fn place<'a>(app: &VectorcraftApp, d: &'a Dialog) -> Option<&'a Value> {
    let uid = active_uid(app)?;
    let same = d.fields.get(DOC).and_then(Value::as_u64) == Some(uid) && d.fields.get(AT_QUERY) == Some(&query(d));
    same.then(|| d.fields.get(AT)).flatten().filter(|at| at.is_object())
}

/// Keep `at`, a place in the active document, as the place the search reached.
fn set_place(app: &VectorcraftApp, d: &mut Dialog, at: Value) {
    d.fields.insert(AT.into(), at);
    d.fields.insert(DOC.into(), json!(active_uid(app)));
    d.fields.insert(AT_QUERY.into(), query(d));
}

/// The match Replace changes, as `{id, start, end}`: the one Find reached in the active document,
/// while the Find text and the switches are as they were when it was found.
fn current_match<'a>(app: &VectorcraftApp, d: &'a Dialog) -> Option<&'a Value> {
    let found = d.fields.get(FOUND)?;
    (found.get("query") == Some(&query(d))).then(|| place(app, d)).flatten()
}

/// Keep what the dialog searched for and the place it reached, for Edit › Find Next and for the
/// next time the dialog opens.
fn remember(app: &mut VectorcraftApp, d: &Dialog) {
    let mut last = query(d);
    last["replace"] = json!(d.str("replace"));
    last["backwards"] = json!(d.bool("backwards"));
    if let Some(at) = place(app, d) {
        last["from"] = at.clone();
        last["doc"] = json!(active_uid(app));
    }
    app.ui.last_find = Some(last);
}

/// Find: the next match after the place the search reached, kept as that place and as the match
/// Replace changes. With no match, the note says so.
fn find(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<Value, String> {
    let mut p = query(d);
    p["backwards"] = json!(d.bool("backwards"));
    // With no place, the search starts at the first match, not after the selected type object.
    p["from"] = place(app, d).cloned().unwrap_or(Value::Null);
    d.fields.remove(FOUND);
    d.fields.remove(NOTE);
    let r = app.run(NEXT, p)?;
    if r["id"].is_null() {
        app.status("No matches found");
        d.fields.insert(NOTE.into(), json!(app.ui.status));
        return Ok(r);
    }
    set_place(app, d, json!({ "id": r["id"], "start": r["start"], "end": r["end"] }));
    d.fields.insert(FOUND.into(), json!({ "index": r["index"], "count": r["count"], "query": query(d) }));
    Ok(r)
}

/// Replace: the match Find reached becomes the Replace With text, and the search goes on from
/// it. With no such match, it finds one.
fn replace(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<Value, String> {
    let Some(at) = current_match(app, d).cloned() else { return find(app, d) };
    let mut p = query(d);
    for key in ["hiddenLayers", "lockedLayers"] {
        if let Some(o) = p.as_object_mut() {
            o.remove(key);
        }
    }
    p["replace"] = json!(d.str("replace"));
    for key in ["id", "start", "end"] {
        p[key] = at[key].clone();
    }
    d.fields.remove(FOUND);
    d.fields.remove(NOTE);
    let r = app.run(REPLACE, p)?;
    set_place(app, d, json!({ "id": r["id"], "start": r["start"], "end": r["end"] }));
    Ok(r)
}

/// Replace All: every match becomes the Replace With text, and the note says how many.
fn replace_all(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<Value, String> {
    let mut p = query(d);
    p["replace"] = json!(d.str("replace"));
    d.fields.remove(FOUND);
    d.fields.remove(NOTE);
    let r = app.run(COMMAND, p)?;
    // The matches that changed: those that already read as the Replace With text do not count.
    match (r["count"].as_u64().unwrap_or(0), r["replaced"].as_u64().unwrap_or(0)) {
        (0, _) => app.status("No matches found"),
        (_, n) => app.status(format!("Matches replaced: {n}")),
    }
    d.fields.insert(NOTE.into(), json!(app.ui.status));
    Ok(r)
}

/// A button or `ui.dialog.confirm`: run the `action` and keep the dialog open, or close it for
/// `done`. What a command refuses is the note, and the dialog stays open. A field set to a value
/// of another type than its own ([`check_types`]) runs no action: the error is the note, shown
/// under the match found, which Replace still changes once the field is set right. So does an
/// `action` that is not a string.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut d = d.clone();
    let action = d.fields.insert("action".into(), json!("find"));
    let checked = match &action {
        Some(a) if !a.is_string() => Err("`action` must be a string".to_string()),
        _ => check_types(&d.fields, false),
    };
    if let Err(e) = checked {
        d.fields.insert(NOTE.into(), json!(e));
        app.ui.dialog = Some(d);
        return Err(e);
    }
    let r = match action.as_ref().and_then(Value::as_str).unwrap_or_default() {
        "done" => {
            remember(app, &d);
            app.ui.dialog = None;
            return Ok(Value::Null);
        }
        "" | "find" => find(app, &mut d),
        "replace" => replace(app, &mut d),
        "replaceFind" => match current_match(app, &d) {
            Some(_) => replace(app, &mut d).and_then(|_| find(app, &mut d)),
            None => find(app, &mut d),
        },
        "replaceAll" => replace_all(app, &mut d),
        _ => Err("no such action".to_string()),
    };
    if let Err(e) = &r {
        d.fields.insert(NOTE.into(), json!(e));
    }
    remember(app, &d);
    app.ui.dialog = Some(d);
    r
}

/// The match Find reached as it reads in its text: (the text before it on its line, the match,
/// the text after it), each at most [`CONTEXT`] characters, and its place among the matches.
fn shown_match(app: &VectorcraftApp, d: &Dialog) -> Option<([String; 3], String)> {
    let at = current_match(app, d)?;
    let found = d.fields.get(FOUND)?;
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
    let before = format!("{lead}{}", before.chars().skip(skip).collect::<String>());
    let after = format!("{}{tail}", after.chars().take(CONTEXT).collect::<String>());
    Some(([before, hit.to_string(), after], format!("{} / {}", found["index"], found["count"])))
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> DialogAction {
    let can_find = !d.str("find").is_empty();
    let found = current_match(app, d).is_some();
    let mut action = None;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            widgets::dim_label(ui, tl!("Find:"));
            form::text(ui, d, "find", 320.0);
            ui.add_space(6.0);
            widgets::dim_label(ui, tl!("Replace With:"));
            form::text(ui, d, "replace", 320.0);
            ui.add_space(10.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    form::check(ui, d, "matchCase", "Match Case");
                    form::check(ui, d, "wholeWord", "Find Whole Word");
                    form::check(ui, d, "backwards", "Search Backwards");
                });
                ui.add_space(16.0);
                ui.vertical(|ui| {
                    form::check(ui, d, "hiddenLayers", "Check Hidden Layers");
                    form::check(ui, d, "lockedLayers", "Check Locked Layers");
                });
            });
        });
        ui.add_space(16.0);
        ui.vertical(|ui| {
            let mut button = |ui: &mut egui::Ui, label: &str, enabled: bool, name: &'static str| {
                if ui.add_enabled_ui(enabled, |ui| widgets::flat_button(ui, label, 130.0).clicked()).inner {
                    action = Some(name);
                }
                ui.add_space(6.0);
            };
            button(ui, if found { tl!("Find Next") } else { tl!("Find") }, can_find, "find");
            button(ui, tl!("Replace"), can_find && found, "replace");
            button(ui, tl!("Replace & Find"), can_find && found, "replaceFind");
            button(ui, tl!("Replace All"), can_find, "replaceAll");
            button(ui, tl!("Done"), true, "done");
        });
    });
    ui.add_space(8.0);
    if let Some(([before, hit, after], place)) = shown_match(app, d) {
        let t = crate::theme::Tokens::get(ui.ctx());
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            widgets::dim_name(ui, &format!("{place}   {before}"));
            ui.colored_label(t.text_strong, egui::RichText::new(hit).strong().underline());
            widgets::dim_name(ui, &after);
        });
    }
    // Find and Replace remove the note, so it shows under a match only when an action was refused.
    if let Some(note) = d.fields.get(NOTE).and_then(Value::as_str) {
        widgets::dim_name(ui, &crate::i18n::message(crate::i18n::current(), note));
    }
    // Enter finds, as the Find button does.
    if action.is_none() && can_find && crate::shortcuts::pressed_anew(ui.ctx(), egui::Key::Enter) {
        action = Some("find");
    }
    match action {
        Some(name) => {
            d.fields.insert("action".into(), json!(name));
            DialogAction::Confirm
        }
        None => DialogAction::None,
    }
}

#[cfg(test)]
mod tests;
