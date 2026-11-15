//! The Symbols panel's Delete Symbol question ([`ask`]). A symbol no instance uses gets "Delete the
//! selected symbol?" with Yes and No ([`KIND`]); a symbol with placed instances gets Expand
//! Instances, Delete Instances and Cancel ([`USED`]).
//!
//! Fields: `name` (the symbol) and `doc` (the [`vectorcraft_engine::DocState::uid`] of the document
//! it was asked in). Yes or either Instances button deletes the symbol only while that document is
//! the active one and still has the symbol; an unused symbol that has gained instances since asks
//! the in-use question instead.

use serde_json::{Value, json};

use super::{DialogResult, DialogSpec};
use crate::VectorcraftApp;
use crate::state::Dialog;

/// The dialog kind of the question for a symbol no instance uses.
pub const KIND: &str = "deleteSymbol";

/// The dialog kind of the question for a symbol with placed instances.
pub const USED: &str = "deleteUsedSymbol";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |d| {
        if d.fields.get("names").and_then(Value::as_array).is_some_and(|n| n.len() > 1) {
            tl!("Delete the selected symbols?")
        } else {
            tl!("Delete the selected symbol?")
        }
        .into()
    },
    body: |_, _, _| false,
    confirm: confirm_unused,
    ok: Some("Yes"),
    cancel: Some("No"),
    max_width: Some(420.0),
    ..DialogSpec::FORM
};

pub(super) const USED_SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("One or more of the symbols are in use and cannot be deleted until their instances are expanded or deleted.").into(),
    body: |_, _, _| false,
    confirm: confirm_used,
    ok: Some("Expand Instances"),
    discard: Some("Delete Instances"),
    max_width: Some(420.0),
    ..DialogSpec::FORM
};

/// Delete Symbol for symbol `name` of the active document. A symbol with instances always asks
/// what becomes of them; an unused one is deleted at once with `now` (Alt-click on the panel's
/// button) and asked about otherwise.
pub(crate) fn ask(app: &mut VectorcraftApp, name: &str, now: bool) {
    let Some(uid) = app.session.active().map(|st| st.uid) else { return };
    match instances(app, uid, name) {
        None => {}
        Some(0) if now => {
            if let Err(e) = app.run("symbol.delete", json!({ "name": name })) {
                app.status(e);
            }
        }
        Some(n) => open(app, if n == 0 { KIND } else { USED }, name, uid),
    }
}

/// Delete Symbol for the unused symbols `names` that Select All Unused picked (#4918): asked once
/// ("Delete the selected symbols?", or the singular for one), or at once with `now`. Only those still
/// unused then (`symbol.unused`: brush art, patterns and nested instances count) go, in one undo step.
pub(crate) fn ask_many(app: &mut VectorcraftApp, names: &[String], now: bool) {
    let Some(uid) = app.session.active().map(|st| st.uid) else { return };
    if now {
        // An error already went to the status bar.
        let _ = delete_unused(app, uid, names);
    } else {
        app.ui.dialog = Some(Dialog::new(KIND, json!({ "names": names, "doc": uid })));
    }
}

/// Deletes those of `names` that are still unused in document `uid` while it is the active one.
fn delete_unused(app: &mut VectorcraftApp, uid: u64, names: &[String]) -> DialogResult {
    if app.session.active().map(|st| st.uid) != Some(uid) {
        return Ok(Value::Null);
    }
    let unused: Vec<String> =
        app.run("symbol.unused", json!({})).ok().and_then(|r| serde_json::from_value(r["names"].clone()).ok()).unwrap_or_default();
    let names: Vec<&String> = names.iter().filter(|n| unused.contains(n)).collect();
    if names.is_empty() {
        return Ok(Value::Null);
    }
    let r = app.run("symbol.delete", json!({ "names": names }));
    if let Err(e) = &r {
        app.status(e.clone());
    }
    r
}

fn open(app: &mut VectorcraftApp, kind: &str, name: &str, uid: u64) {
    app.ui.dialog = Some(Dialog::new(kind, json!({ "name": name, "doc": uid })));
}

/// How many instances symbol `name` has, while the document `uid` is the active one and has the
/// symbol; `None` otherwise.
fn instances(app: &mut VectorcraftApp, uid: u64, name: &str) -> Option<u64> {
    if app.session.active().map(|st| st.uid) != Some(uid) {
        return None;
    }
    let list = app.run("symbol.list", json!({})).ok()?;
    list["symbols"].as_array()?.iter().find(|s| s["name"].as_str() == Some(name)).map(|s| s["instances"].as_u64().unwrap_or(0))
}

/// The document and symbol `d` asks about.
fn target(d: &Dialog) -> (u64, String) {
    (d.fields.get("doc").and_then(Value::as_u64).unwrap_or(0), d.str("name"))
}

/// Yes: deletes the symbol, or asks the in-use question when it has gained instances.
fn confirm_unused(app: &mut VectorcraftApp, d: &Dialog) -> DialogResult {
    app.ui.dialog = None;
    let (uid, name) = target(d);
    if let Some(names) = d.fields.get("names").and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok()) {
        return delete_unused(app, uid, &names);
    }
    match instances(app, uid, &name) {
        None => Ok(Value::Null),
        Some(0) => app.run("symbol.delete", json!({ "name": name })),
        Some(_) => {
            open(app, USED, &name, uid);
            Ok(Value::Null)
        }
    }
}

/// Expand Instances (OK) turns the instances into ordinary art; Delete Instances (`discard`)
/// removes them. Either one deletes the symbol in one undo step.
fn confirm_used(app: &mut VectorcraftApp, d: &Dialog) -> DialogResult {
    app.ui.dialog = None;
    let (uid, name) = target(d);
    if instances(app, uid, &name).is_none() {
        return Ok(Value::Null);
    }
    app.run("symbol.delete", json!({ "name": name, "expandInstances": !d.bool("discard") }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The question for the symbols Select All Unused picked is plural (#4918).
    #[test]
    fn several_symbols_get_the_plural_question() {
        let one = Dialog::new(KIND, json!({"name": "A", "doc": 1}));
        let many = Dialog::new(KIND, json!({"names": ["A", "B"], "doc": 1}));
        assert_eq!((SPEC.heading)(&one), "Delete the selected symbol?");
        assert_eq!((SPEC.heading)(&many), "Delete the selected symbols?");
        let picked_one = Dialog::new(KIND, json!({"names": ["A"], "doc": 1}));
        assert_eq!((SPEC.heading)(&picked_one), "Delete the selected symbol?");
    }
}
