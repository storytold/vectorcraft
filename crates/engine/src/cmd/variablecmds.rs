//! Variables (data merge): named values bound to objects, applied per dataset.
//!
//! One template document produces many variants: bind a type object to a `text`
//! variable, anything to a `visibility` variable, a linked image to an `image`
//! variable, or a graph root to a `graph` variable (CSV data), put a row of values in
//! each dataset, and `dataset.select` swaps the whole document to that row in one undo
//! step. `dataset.import` / `dataset.export` move those rows as CSV (header: `name`
//! then variable names); import also reads tab-separated files, files without the
//! `name` column, and header cells that define new variables (`#` for visibility, `@` for
//! image, no prefix for text). `variable.loadLibrary` replaces the variables and data sets with the
//! text variables and data sets of a variable library, the XML file of the reference app's Save
//! Variable Library. Agents reach every command through `run_command`, like all engine commands.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use vectorcraft_doc::{DataSet, DataValue, NodeId, NodeKind, Variable, VariableKind, Variables};

use super::edit::selected_roots;
use super::fileio::{absolute_path, read_file_head, write_file};
use super::textedit::set_plain_text;
use super::{CommandSpec, EngineError, Result, Session, bad, bool_or, checked_ids_or_selection, cmd, has_doc, has_selection, links, str_param};

mod csv;
mod library;

use csv::{MAX_GRAPH_CSV, dataset_export, dataset_import};
use library::load_library;

/// The longest variable or dataset name, in characters (names arrive from MCP and files).
const MAX_NAME: usize = 255;

/// Lock Variables refuses New Variable, Delete Variable, Rename Variable (Variable Options),
/// Make … Dynamic, an import whose header would define a variable and a Load Variable Library
/// that would delete or define one; binding, unbinding and the other dataset commands still run.
fn variables_unlocked(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    if s.active().is_some_and(|st| st.doc.variables.locked) { Err("variables are locked".into()) } else { Ok(()) }
}

/// Make … Dynamic needs a selection, and defines a variable, which Lock Variables refuses.
fn selection_and_unlocked(s: &Session) -> std::result::Result<(), String> {
    has_selection(s)?;
    variables_unlocked(s)
}

fn var_name(p: &Value, cmd: &str) -> Result<String> {
    named(str_param(p, "name"), "name", cmd)
}

/// `key`'s value, as a name: trimmed, not empty and at most [`MAX_NAME`] characters.
fn named(v: Option<&str>, key: &str, cmd: &str) -> Result<String> {
    match v.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) if v.chars().count() > MAX_NAME => Err(bad(cmd, format!("`{key}` is longer than {MAX_NAME} characters"))),
        Some(v) => Ok(v.to_string()),
        None => Err(bad(cmd, format!("missing `{key}`"))),
    }
}

/// `key`'s value as a name, or `None` when it is missing or empty.
fn optional_name(p: &Value, key: &str, cmd: &str) -> Result<Option<String>> {
    match str_param(p, key).map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) => named(Some(v), key, cmd).map(Some),
        None => Ok(None),
    }
}

/// The `names` of a delete, or without them `fallback` (what the panel acts on) as the one name.
fn names_param(p: &Value, cmd: &str, fallback: Option<String>, nothing: &str) -> Result<Vec<String>> {
    let Some(given) = p.get("names") else {
        return fallback.map(|n| vec![n]).ok_or_else(|| bad(cmd, nothing.to_string()));
    };
    match given.as_array() {
        Some(names) if !names.is_empty() => names.iter().map(|n| named(n.as_str(), "names", cmd)).collect(),
        _ => Err(bad(cmd, "`names` must be a list of names")),
    }
}

/// Refuse what a variable of `kind` can't be bound to: art that is locked or on a hidden or locked
/// layer; a text variable only drives type; an image variable only drives an image object.
fn check_bindable(doc: &vectorcraft_doc::Document, ids: &[NodeId], kind: VariableKind, cmd: &str) -> Result<()> {
    for id in ids {
        match doc.node(*id) {
            None => return Err(EngineError::NoNode(*id)),
            Some(_) if !doc.variable_target(*id) => return Err(bad(cmd, not_a_target(doc, *id))),
            Some(n) if kind == VariableKind::Text && !matches!(n.kind, NodeKind::Text(_)) => {
                return Err(bad(cmd, format!("node {} is not type", id.0)));
            }
            Some(n) if kind == VariableKind::Image && !matches!(n.kind, NodeKind::Image(_)) => {
                return Err(bad(cmd, format!("node {} is not an image", id.0)));
            }
            Some(n) if kind == VariableKind::Graph && n.extras().graph.is_none() => {
                return Err(bad(cmd, format!("node {} is not a graph", id.0)));
            }
            Some(_) => {}
        }
    }
    Ok(())
}

/// Why `variable_target` refuses `id`: the object is locked, a group or layer that holds it is
/// locked, or one that holds it is hidden, checked in that order.
fn not_a_target(doc: &vectorcraft_doc::Document, id: NodeId) -> String {
    let holders: Vec<&vectorcraft_doc::Node> =
        doc.ancestry(id).unwrap_or_default().into_iter().filter(|i| *i != id).filter_map(|i| doc.node(i)).collect();
    if doc.node(id).is_some_and(|n| n.locked) {
        format!("node {} is locked", id.0)
    } else if holders.iter().any(|n| n.locked) {
        format!("node {} is in a locked group or layer", id.0)
    } else {
        format!("node {} is in a hidden group or layer", id.0)
    }
}

fn dataset_value(cmd: &str, var: &str, kind: VariableKind, v: &Value) -> Result<DataValue> {
    match kind {
        VariableKind::Text => v.as_str().map(|t| DataValue::Text(t.to_string())),
        VariableKind::Visibility => v.as_bool().map(DataValue::Visible),
        // An image value is a path string; empty is not a file to open.
        VariableKind::Image => v.as_str().map(str::trim).filter(|t| !t.is_empty()).map(|t| DataValue::Image(t.to_string())),
        VariableKind::Graph => v
            .as_str()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .and_then(|t| if t.len() > MAX_GRAPH_CSV { None } else { Some(DataValue::Graph(t.to_string())) }),
    }
    .ok_or_else(|| bad(cmd, format!("value for `{var}` must match its variable kind")))
}

/// The `values` object of `dataset.new` / `dataset.set`, each entry checked against its
/// variable's kind. Every name must be a variable the document defines: a value nothing
/// reads is a typo, not a value to keep.
fn values_param(vars: &Variables, p: &Value, cmd: &str) -> Result<BTreeMap<String, DataValue>> {
    let mut out = BTreeMap::new();
    let Some(given) = p.get("values") else { return Ok(out) };
    let Some(obj) = given.as_object() else { return Err(bad(cmd, "`values` must be an object")) };
    for (var, v) in obj {
        let kind = vars.variable(var).map(|d| d.kind).ok_or_else(|| bad(cmd, format!("no variable named `{var}`")))?;
        out.insert(var.clone(), dataset_value(cmd, var, kind, v)?);
    }
    Ok(out)
}

fn define(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "variable.define";
    let name = var_name(p, C)?;
    let kind = str_param(p, "kind").and_then(VariableKind::parse).ok_or_else(|| bad(C, "`kind` must be `text`, `visibility`, `image` or `graph`"))?;
    if s.doc()?.doc.variables.variable(&name).is_some() {
        return Err(bad(C, format!("variable `{name}` is already defined")));
    }
    s.edit("New Variable", |d, _| {
        d.variables.variables.push(Variable { name: name.clone(), kind });
        Ok(())
    })?;
    Ok(json!({"name": name, "kind": kind.label()}))
}

fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "variable.rename";
    let from = var_name(p, C)?;
    let to = named(p.get("newName").and_then(Value::as_str), "newName", C)?;
    let st = s.doc()?;
    if st.doc.variables.variable(&to).is_some() {
        return Err(bad(C, format!("variable `{to}` already exists")));
    }
    s.edit("Rename Variable", |d, _| {
        let v = d.variables.variables.iter_mut().find(|v| v.name == from).ok_or_else(|| bad(C, format!("no variable named `{from}`")))?;
        v.name = to.clone();
        for bound in d.variables.bindings.values_mut() {
            if bound.remove(&from) {
                bound.insert(to.clone());
            }
        }
        for ds in &mut d.variables.datasets {
            if let Some(value) = ds.values.remove(&from) {
                ds.values.insert(to.clone(), value);
            }
        }
        Ok(())
    })?;
    // The highlight names a variable, so it follows the rename rather than going stale.
    let st = s.doc_mut()?;
    if st.variables_highlight.as_deref() == Some(from.as_str()) {
        st.variables_highlight = Some(to.clone());
    }
    st.revision += 1;
    Ok(json!({"from": from, "name": to}))
}

/// Make Text / Visibility / Image / Graph Dynamic: define a variable of that kind and bind the
/// selection to it in one step, naming it after what it drives — the panel's buttons.
fn make_dynamic(s: &mut Session, kind: VariableKind, stem: &str) -> Result<Value> {
    // Errors name the command that ran: `variable.make{stem}Dynamic`.
    let c = &format!("variable.make{stem}Dynamic");
    let ids = selected_roots(s)?;
    if ids.is_empty() {
        return Err(bad(c, "select the objects to bind"));
    }
    let doc = &s.doc()?.doc;
    check_bindable(doc, &ids, kind, c)?;
    // Named after the first object it drives, as the reference panel does; a second variable
    // over the same art takes a number rather than colliding.
    let shown = ids.first().and_then(|id| doc.node(*id)).map(|n| n.display_name()).unwrap_or_default();
    let base: String = match shown.trim() {
        "" => stem.to_string(),
        b => b.chars().take(MAX_NAME - 8).collect(),
    };
    let mut name = base.clone();
    let mut n = 2;
    while doc.variables.variable(&name).is_some() {
        name = format!("{base} {n}");
        n += 1;
    }
    s.edit("Make Dynamic", |d, _| {
        d.variables.variables.push(Variable { name: name.clone(), kind });
        for id in &ids {
            d.variables.bind(*id, &name);
        }
        Ok(())
    })?;
    Ok(json!({"name": name, "kind": kind.label(), "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()}))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    // Without `names`, the highlighted variable: what the panel's bin and the menu item delete.
    let highlighted = s.doc()?.variables_highlight.clone();
    let names = names_param(p, "variable.delete", highlighted, "no variable highlighted")?;
    let n = s.edit("Delete Variables", |d, _| Ok(d.variables.prune(&names)))?;
    // A highlight of a variable that is gone highlights nothing.
    let st = s.doc_mut()?;
    if st.variables_highlight.as_ref().is_some_and(|h| st.doc.variables.variable(h).is_none()) {
        st.variables_highlight = None;
    }
    st.revision += 1;
    Ok(json!({"deleted": n}))
}

fn list(s: &mut Session, _p: &Value) -> Result<Value> {
    let st = s.doc()?;
    let variables: Vec<Value> = st
        .doc
        .variables
        .variables
        .iter()
        .map(|v| {
            let bindings = st.doc.variables.objects_of(&v.name).len();
            json!({"name": v.name, "kind": v.kind.label(), "bindings": bindings})
        })
        .collect();
    Ok(json!({"variables": variables, "locked": st.doc.variables.locked}))
}

/// Lock Variables: without `locked` it toggles. Setting the state the document already has
/// does not edit the document or add an undo step.
fn lock(s: &mut Session, p: &Value) -> Result<Value> {
    let was = s.doc()?.doc.variables.locked;
    let locked = bool_or(p, "locked", !was);
    if locked != was {
        s.edit(if locked { "Lock Variables" } else { "Unlock Variables" }, |d, _| {
            d.variables.locked = locked;
            Ok(())
        })?;
    }
    Ok(json!({"locked": locked}))
}

fn bind(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "variable.bind";
    let name = named(str_param(p, "variable"), "variable", C)?;
    let ids = checked_ids_or_selection(s, p, C)?;
    if ids.is_empty() {
        return Err(bad(C, "nothing to bind"));
    }
    let doc = &s.doc()?.doc;
    let kind = doc.variables.variable(&name).map(|v| v.kind).ok_or_else(|| bad(C, format!("no variable named `{name}`")))?;
    check_bindable(doc, &ids, kind, C)?;
    // Binding again is not a second binding, and says so rather than counting as a change.
    let already: Vec<u64> = ids.iter().filter(|id| doc.variables.objects_of(&name).contains(id)).map(|id| id.0).collect();
    let n = s.edit("Bind Variable", |d, _| Ok(ids.iter().filter(|id| d.variables.bind(**id, &name)).count()))?;
    Ok(json!({"variable": name, "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "bound": n, "already": already}))
}

fn unbind(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "variable.unbind";
    let ids = checked_ids_or_selection(s, p, C)?;
    let name = defined_variable(s, p, C)?;
    let n = s.edit("Unbind Variable", |d, _| {
        let mut n = 0;
        for id in &ids {
            // With `variable` only that one binding goes; without it, every one the object has.
            match &name {
                Some(v) => n += usize::from(d.variables.unbind(*id, v)),
                None => n += usize::from(d.variables.bindings.remove(id).is_some()),
            }
        }
        Ok(n)
    })?;
    Ok(json!({"unbound": n}))
}

/// The `variable` a command names, which must be defined; `None` when it names none.
fn defined_variable(s: &Session, p: &Value, cmd: &str) -> Result<Option<String>> {
    let name = optional_name(p, "variable", cmd)?;
    if let Some(v) = &name
        && s.doc()?.doc.variables.variable(v).is_none()
    {
        return Err(bad(cmd, format!("no variable named `{v}`")));
    }
    Ok(name)
}

/// Highlight one Variables panel row: what the panel's Delete, Options… and Select Bound
/// Object act on. Panel state, not art selection: not saved, not an undo step (an empty
/// or missing `variable` clears it), like `layer.highlight`.
fn highlight(s: &mut Session, p: &Value) -> Result<Value> {
    let name = defined_variable(s, p, "variable.highlight")?;
    let st = s.doc_mut()?;
    st.variables_highlight = name.clone();
    st.revision += 1;
    Ok(json!({"variable": name}))
}

fn dataset_new(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "dataset.new";
    let name = var_name(p, C)?;
    let st = s.doc()?;
    // A dataset and a variable are separate namespaces, so a dataset may share a variable's
    // name; it is its own row of values either way.
    let values = values_param(&st.doc.variables, p, C)?;
    if st.doc.variables.dataset(&name).is_some() {
        return Err(bad(C, format!("dataset `{name}` already exists")));
    }
    s.edit("New Data Set", |d, _| {
        d.variables.datasets.push(DataSet { name: name.clone(), values: values.clone() });
        Ok(())
    })?;
    Ok(json!({"name": name}))
}

/// Replace a dataset's values. A variable left out of `values` loses its value in this row,
/// so the dataset ends up holding exactly what the call asked for.
fn dataset_set(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "dataset.set";
    let name = var_name(p, C)?;
    let values = values_param(&s.doc()?.doc.variables, p, C)?;
    let (set, removed) = s.edit("Edit Data Set", |d, _| {
        let ds = d.variables.datasets.iter_mut().find(|x| x.name == name).ok_or_else(|| bad(C, format!("no dataset named `{name}`")))?;
        let removed: Vec<String> = ds.values.keys().filter(|k| !values.contains_key(*k)).cloned().collect();
        ds.values = values.clone();
        Ok((values.len(), removed))
    })?;
    Ok(json!({"name": name, "values": set, "removed": removed}))
}

fn dataset_rename(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "dataset.rename";
    let from = var_name(p, C)?;
    let to = named(p.get("newName").and_then(Value::as_str), "newName", C)?;
    let st = s.doc()?;
    if st.doc.variables.dataset(&to).is_some() {
        return Err(bad(C, format!("dataset `{to}` already exists")));
    }
    s.edit("Rename Data Set", |d, _| {
        let ds = d.variables.datasets.iter_mut().find(|d| d.name == from).ok_or_else(|| bad(C, format!("no dataset named `{from}`")))?;
        ds.name = to.clone();
        if d.variables.active_dataset.as_deref() == Some(from.as_str()) {
            d.variables.active_dataset = Some(to.clone());
        }
        Ok(())
    })?;
    Ok(json!({"from": from, "name": to}))
}

/// Capture Data Set: record what the bound objects hold right now as a new row. This, rather
/// than typing values, is how rows are made — change the art, then capture it.
fn dataset_capture(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "dataset.capture";
    let st = s.doc()?;
    let values = st.doc.captured_values();
    if values.is_empty() {
        return Err(bad(C, "bind something to a variable first"));
    }
    let name = match optional_name(p, "name", C)? {
        Some(n) => n,
        // "Data Set 1", the first free number, as the reference panel's rows are named.
        None => (1..).map(|i| format!("Data Set {i}")).find(|n| st.doc.variables.dataset(n).is_none()).unwrap_or_default(),
    };
    if s.doc()?.doc.variables.dataset(&name).is_some() {
        return Err(bad(C, format!("dataset `{name}` already exists")));
    }
    s.edit("Capture Data Set", |d, _| {
        d.variables.datasets.push(DataSet { name: name.clone(), values: values.clone() });
        d.variables.active_dataset = Some(name.clone());
        Ok(())
    })?;
    Ok(json!({"name": name, "values": values.len()}))
}

/// Update Data Set: write what the bound objects hold now into the row that is active, for each
/// variable the row holds. A variable whose objects report no value (locked, inside a locked
/// layer or group, or gone) keeps the row's value. With `all`, every other variable the
/// objects report a value for is added to the row. Without an active row there is nothing to
/// update, so say so rather than picking one.
fn dataset_update(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "dataset.update";
    let all = bool_or(p, "all", false);
    let values = s.doc()?.doc.captured_values();
    let Some(name) = s.doc()?.doc.variables.active_dataset.clone() else {
        return Err(bad(C, "no active data set"));
    };
    if values.is_empty() {
        return Err(bad(C, "bind something to a variable first"));
    }
    let (written, left) = s.edit("Update Data Set", |d, _| {
        let ds = d.variables.datasets.iter_mut().find(|d| d.name == name).ok_or_else(|| bad(C, format!("no dataset named `{name}`")))?;
        let mut written = BTreeSet::new();
        for (var, value) in &values {
            if all || ds.values.contains_key(var) {
                ds.values.insert(var.clone(), value.clone());
                written.insert(var.as_str());
            }
        }
        let left: Vec<String> = d.variables.variables.iter().filter(|v| !written.contains(v.name.as_str())).map(|v| v.name.clone()).collect();
        Ok((written.len(), left))
    })?;
    Ok(json!({"name": name, "values": written, "left": left}))
}

fn dataset_delete(s: &mut Session, p: &Value) -> Result<Value> {
    // Without `names`, the active dataset: what the panel's Delete Data Set deletes.
    let active = s.doc()?.doc.variables.active_dataset.clone();
    let names = names_param(p, "dataset.delete", active, "no active data set")?;
    let n = s.edit("Delete Data Sets", |d, _| {
        let before = d.variables.datasets.len();
        d.variables.datasets.retain(|d| !names.contains(&d.name));
        // The active dataset's own name is the one that goes, not a variable's.
        if d.variables.active_dataset.as_ref().is_some_and(|a| names.contains(a)) {
            d.variables.active_dataset = None;
        }
        Ok(before - d.variables.datasets.len())
    })?;
    Ok(json!({"deleted": n}))
}

fn dataset_list(s: &mut Session, _p: &Value) -> Result<Value> {
    let st = s.doc()?;
    let current = st.doc.captured_values();
    let datasets: Vec<Value> = st
        .doc
        .variables
        .datasets
        .iter()
        // `matches` says whether the art still holds what the row says, which is what the
        // panel shows in italics.
        .map(|d| json!({"name": d.name, "values": d.values, "matches": d.matches(&current)}))
        .collect();
    Ok(json!({"datasets": datasets, "active": st.doc.variables.active_dataset}))
}

fn apply_dataset(s: &mut Session, name: &str) -> Result<Value> {
    const C: &str = "dataset.select";
    let doc_path = s.doc()?.path.clone();
    let (applied, skipped, mut typed, bound) = s.edit("Apply Data Set", |d, sel| {
        let ds = d.variables.dataset(name).cloned().ok_or_else(|| bad(C, format!("no dataset named `{name}`")))?;
        // One object may be driven by several variables; walk them in definition order, so two
        // text variables on the same object land the later one's value and both are counted.
        let mut pairs: Vec<(NodeId, String)> = Vec::new();
        for v in &d.variables.variables {
            pairs.extend(d.variables.objects_of(&v.name).into_iter().map(|id| (id, v.name.clone())));
        }
        // Whether an object is bound to a variable that this row has a value for (the reply's `warning`).
        let bound = pairs.iter().any(|(_, var)| ds.values.contains_key(var));
        let mut applied = 0usize;
        let mut skipped = 0usize;
        // Collects the type objects whose text the row rewrote, for the reply's `overflow`.
        let mut typed = Vec::new();
        for (id, var) in pairs {
            // A binding whose variable is gone, whose value this row doesn't carry, or whose
            // value doesn't fit the variable's kind, is left alone and counted, never
            // silently reinterpreted.
            let (kind, value) = match (d.variables.variable(&var).map(|v| v.kind), ds.values.get(&var)) {
                (Some(kind), Some(value)) if value.fits(kind) => (kind, value.clone()),
                _ => {
                    skipped += 1;
                    continue;
                }
            };
            // Locked art, art in a locked layer or group, and text whose object is gone, can't be
            // rewritten: skipped and counted, like the rest. Hidden art is rewritten, in a hidden
            // layer or group too, and stays hidden unless a visibility value shows it.
            if !d.dataset_target(id) {
                skipped += 1;
                continue;
            }
            let done = match (kind, &value) {
                // The Type tool's own whole-range replace, so a dataset's text is styled and
                // laid out exactly as typing it would be.
                (VariableKind::Text, DataValue::Text(text)) => set_plain_text(d, id, text).is_ok_and(|()| {
                    typed.push(id);
                    true
                }),
                (VariableKind::Visibility, DataValue::Visible(show)) => d.node_mut(id).is_some_and(|n| {
                    n.visible = *show;
                    true
                }),
                // Swap the linked file; keep the object's bounds. A missing path skips.
                (VariableKind::Image, DataValue::Image(path)) => links::set_image_from_path(d, id, path, doc_path.as_deref()).is_ok(),
                (VariableKind::Graph, DataValue::Graph(csv)) => super::graph::apply_csv(d, id, csv).is_ok(),
                _ => false,
            };
            if done {
                applied += 1;
            } else {
                skipped += 1;
            }
        }
        d.variables.active_dataset = Some(name.to_string());
        // Every selected object left hidden or locked, such as one the row hid, is deselected.
        sel.deselect_uneditable(d);
        Ok((applied, skipped, typed, bound))
    })?;
    // Two text variables on one object rewrite it twice; it is named once.
    typed.sort_unstable();
    typed.dedup();
    let mut out = super::typecmd::with_overflow(s, &typed, json!({"name": name, "applied": applied, "skipped": skipped}));
    // A row with no value for any bound variable applies nothing. The reply then carries a
    // `warning`, which the app shows in its status bar.
    if !bound {
        out["warning"] = json!(format!("nothing in data set {name} was applied: no object is bound to a variable it has a value for"));
    }
    Ok(out)
}

fn dataset_select(s: &mut Session, p: &Value) -> Result<Value> {
    apply_dataset(s, &var_name(p, "dataset.select")?)
}

fn dataset_step(s: &mut Session, cmd: &str, forward: bool) -> Result<Value> {
    let st = s.doc()?;
    let n = st.doc.variables.datasets.len();
    if n == 0 {
        return Err(bad(cmd, "no datasets"));
    }
    let at = st.doc.variables.active_dataset.as_deref().and_then(|a| st.doc.variables.datasets.iter().position(|d| d.name == a));
    let next = match (at, forward) {
        (Some(i), true) => (i + 1) % n,
        (Some(i), false) => (i + n - 1) % n,
        (None, true) => 0,
        (None, false) => n - 1,
    };
    let name = st.doc.variables.datasets.get(next).map(|d| d.name.clone()).ok_or_else(|| bad(cmd, "no datasets"))?;
    apply_dataset(s, &name)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "variable.define",
            "New Variable…",
            ["Window", "Variables"],
            None,
            "{name, kind: text|visibility|image|graph} define a variable (graph: CSV data on a graph root); refused while the variables are locked",
            variables_unlocked,
            define
        ),
        cmd!(
            "variable.delete",
            "Delete Variable",
            ["Window", "Variables"],
            None,
            "{names?} delete variables with their bindings and values (default: the highlighted one); refused while the variables are locked",
            variables_unlocked,
            delete
        ),
        cmd!(
            "variable.rename",
            "Rename Variable",
            ["Window", "Variables"],
            None,
            "{name, newName} rename a variable, with its bindings and its values in every dataset; refused while the variables are locked",
            variables_unlocked,
            rename
        ),
        cmd!(query "variable.list", "List Variables", [], None, "{} → variables with kinds and binding counts, and whether they are locked → {variables, locked}", has_doc, list),
        cmd!(
            "variable.makeTextDynamic",
            "Make Text Dynamic",
            ["Window", "Variables"],
            None,
            "{} define a text variable over the selection, named after it and bound to it; refused while the variables are locked",
            selection_and_unlocked,
            |s, _| make_dynamic(s, VariableKind::Text, "Text")
        ),
        cmd!(
            "variable.makeVisibilityDynamic",
            "Make Visibility Dynamic",
            ["Window", "Variables"],
            None,
            "{} define a visibility variable over the selection, named after it and bound to it; refused while the variables are locked",
            selection_and_unlocked,
            |s, _| make_dynamic(s, VariableKind::Visibility, "Visibility")
        ),
        cmd!(
            "variable.makeImageDynamic",
            "Make Image Dynamic",
            ["Window", "Variables"],
            None,
            "{} define an image variable over the selection, named after it and bound to it; refused while the variables are locked",
            selection_and_unlocked,
            |s, _| make_dynamic(s, VariableKind::Image, "Image")
        ),
        cmd!(
            "variable.makeGraphDynamic",
            "Make Graph Dynamic",
            ["Window", "Variables"],
            None,
            "{} define a graph variable over the selected graph root(s), named after it and bound to it; each dataset row supplies CSV as graph.setData; refused while the variables are locked",
            selection_and_unlocked,
            |s, _| make_dynamic(s, VariableKind::Graph, "Graph")
        ),
        cmd!(
            "variable.bind",
            "Bind Variable",
            ["Window", "Variables"],
            None,
            "{variable, ids?} bind objects (default: the selection) to a variable; text needs type, image needs an image object, graph needs a graph root, locked art is refused. An object may hold several variables, and binding it to one it already has changes nothing → {bound, already}",
            has_doc,
            bind
        ),
        cmd!(
            "variable.unbind",
            "Unbind Variable",
            ["Window", "Variables"],
            None,
            "{ids?, variable?} drop bindings (default: the selection): `variable` drops that one, else every variable the object has",
            has_doc,
            unbind
        ),
        cmd!(
            "variable.lock",
            "Lock Variables",
            ["Window", "Variables"],
            None,
            "{locked?: bool} lock or unlock the variables (default: toggle); saved with the document; a change is one undo step. While locked, variable.define, variable.delete, variable.rename, variable.make…Dynamic and a variable.loadLibrary that would delete or define a variable are refused; binding, unbinding and the dataset commands still run → {locked}",
            has_doc,
            lock
        ),
        cmd!(
            "variable.highlight",
            "Highlight Variable",
            [],
            None,
            "{variable?} highlight one Variables panel row (a click); what the panel's Delete, Options… and Select Bound Object act on. An empty or missing `variable` clears it. Not an undo step → {variable}",
            has_doc,
            highlight
        ),
        cmd!("dataset.new", "New Data Set…", ["Window", "Variables"], None, "{name, values?: {variable: value}} add a dataset", has_doc, dataset_new),
        cmd!(
            "dataset.set",
            "Edit Data Set…",
            ["Window", "Variables"],
            None,
            "{name, values?: {variable: value}} replace a dataset's values; a variable left out loses its value in this row → {values, removed}",
            has_doc,
            dataset_set
        ),
        cmd!("dataset.rename", "Rename Data Set", ["Window", "Variables"], None, "{name, newName} rename a dataset", has_doc, dataset_rename),
        cmd!(
            "dataset.capture",
            "Capture Data Set",
            ["Window", "Variables"],
            None,
            "{name?} record what the bound objects hold right now as a new dataset (Data Set 1…), and make it the active one",
            has_doc,
            dataset_capture
        ),
        cmd!(
            "dataset.update",
            "Update Data Set",
            ["Window", "Variables"],
            None,
            "{all?: bool} write what the bound objects hold now into the active dataset, for each variable the dataset holds; a variable whose objects report no value (locked, inside a locked layer or group, or gone) keeps the dataset's value. `all: true` also adds every other variable the objects report a value for; `left` names the variables it didn't write → {values, left}",
            has_doc,
            dataset_update
        ),
        cmd!(
            "dataset.delete",
            "Delete Data Set",
            ["Window", "Variables"],
            None,
            "{names?} delete datasets (default: the active one)",
            has_doc,
            dataset_delete
        ),
        cmd!(
            query "dataset.list",
            "List Data Sets",
            [],
            None,
            "{} → datasets with their values, the active one, and which rows the art still matches",
            has_doc,
            dataset_list
        ),
        cmd!(
            "dataset.select",
            "Select Data Set",
            ["Window", "Variables"],
            None,
            "{name} apply a dataset in one undo step, deselecting every selected object left hidden or locked (the key object too) → {applied, skipped, overflow?: [id…] (area type and type on a path whose text it rewrote and that no longer fits; left out when all fits), warning? (when no object is bound to a variable that the dataset has a value for, so nothing was applied)}",
            has_doc,
            dataset_select
        ),
        cmd!("dataset.next", "Next Data Set", ["Window", "Variables"], None, "{} apply the next dataset, wrapping", has_doc, |s, _| dataset_step(
            s,
            "dataset.next",
            true
        )),
        cmd!("dataset.prev", "Previous Data Set", ["Window", "Variables"], None, "{} apply the previous dataset, wrapping", has_doc, |s, _| {
            dataset_step(s, "dataset.prev", false)
        }),
        cmd!(
            "dataset.import",
            "Import Data Sets…",
            ["Window", "Variables"],
            None,
            "{path? | data? | dataBase64? | csv?} load datasets from CSV, comma- or tab-separated (header: name, then variable names; one row per dataset; without the name column every header is a variable and the rows take the first free names Data Set 1, 2…). A header that names no variable defines one, unbound: `#Name` defines a visibility variable, `@Name` defines an image variable, and a header without a prefix defines a text variable. A prefixed header whose name is a variable of another kind, and a `%` header that isn't a variable's exact name (graph data files), are refused, and so is a header that would define a variable while the variables are locked. Rows whose name already exists are replaced → {imported, updated, names, defined}",
            has_doc,
            dataset_import
        ),
        cmd!(
            query "dataset.export",
            "Export Data Sets…",
            ["Window", "Variables"],
            None,
            "{path?} write every dataset as CSV (header: name, then variables); without path → {csv, datasets}",
            has_doc,
            dataset_export
        ),
        cmd!(
            "variable.loadLibrary",
            "Load Variable Library…",
            ["Window", "Variables"],
            None,
            "{path? | xml? | dataBase64?} replace the document's variables and data sets with a variable library's (XML: `variableSets` › `variableSet` › `variables` of `variable` elements with `varName` and `trait`, and `sampleDataSets` of `sampleDataSet` elements with `dataSetName`) in one undo step. Every variable of the document is deleted with its bindings and values, and every data set is deleted; then variables whose `trait` is `textcontent` are defined as text variables, unbound, and each data set takes a value for a variable from the `p` elements of the element named after it, one paragraph each. Variables of other kinds are left out and named in `skipped` and `warning`. Refused while the variables are locked when it would delete or define a variable → {variables, deleted, deletedDataSets, names, skipped, warning?}",
            has_doc,
            load_library
        ),
    ]
}
