//! Variables (data merge): named values bound to objects, applied per dataset.
//!
//! One template document produces many variants: bind a type object to a `text`
//! variable, anything to a `visibility` variable, a linked image to an `image`
//! variable, or a graph root to a `graph` variable (CSV data), put a row of values in
//! each dataset, and `dataset.select` swaps the whole document to that row in one undo
//! step. `dataset.import` / `dataset.export` move those rows as CSV (header: `name`
//! then variable names). Agents reach every command through `run_command`, like all
//! engine commands.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use vectorcraft_doc::{DataSet, DataValue, NodeId, NodeKind, Variable, VariableKind, Variables};

use super::edit::selected_roots;
use super::fileio::{absolute_path, read_file_head, write_file};
use super::textedit::set_plain_text;
use super::*;

/// The longest variable or dataset name, in characters (names arrive from MCP and files).
const MAX_NAME: usize = 255;
/// Cap on a graph variable's CSV (MCP and files are untrusted).
const MAX_GRAPH_CSV: usize = 256 * 1024;

/// Caps on a datasets CSV (paths and MCP payloads are untrusted).
const MAX_CSV_BYTES: usize = 1_048_576;
const MAX_CSV_ROWS: usize = 1_000;
const MAX_CSV_COLS: usize = 256;

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

/// Refuse what a variable of `kind` can't drive: art that is locked or on a hidden or locked
/// layer is not the document's to rewrite; a text variable only drives type; an image variable
/// only drives an image object.
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
    Ok(json!({"variables": variables}))
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
/// variable the row holds. A variable whose objects report no value (locked, inside a hidden or
/// locked layer or group, or gone) keeps the row's value. With `all`, every other variable the
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
    let (applied, skipped, mut typed) = s.edit("Apply Data Set", |d, sel| {
        let ds = d.variables.dataset(name).cloned().ok_or_else(|| bad(C, format!("no dataset named `{name}`")))?;
        // One object may be driven by several variables; walk them in definition order, so two
        // text variables on the same object land the later one's value and both are counted.
        let mut pairs: Vec<(NodeId, String)> = Vec::new();
        for v in &d.variables.variables {
            pairs.extend(d.variables.objects_of(&v.name).into_iter().map(|id| (id, v.name.clone())));
        }
        // The row's values go in three passes: the visibility values that show an object,
        // outermost object first; the other values, in definition order; the visibility values
        // that hide an object, innermost object first. An object inside a group is then rewritten
        // after the row shows the group and before the row hides it. An object with several
        // visibility variables goes in the pass of the last one's value, and each pass keeps
        // definition order otherwise.
        let mut shows = std::collections::HashMap::new();
        for (id, var) in &pairs {
            let kind = d.variables.variable(var).map(|v| v.kind);
            if let (Some(VariableKind::Visibility), Some(DataValue::Visible(show))) = (kind, ds.values.get(var)) {
                shows.insert(*id, *show);
            }
        }
        let depths = d.index_paths(shows.keys().copied());
        pairs.sort_by_cached_key(|(id, var)| {
            let depth = depths.get(id).map_or(0, Vec::len);
            match (d.variables.variable(var).map(|v| v.kind), shows.get(id)) {
                (Some(VariableKind::Visibility), Some(true)) => (0, depth),
                (Some(VariableKind::Visibility), Some(false)) => (2, usize::MAX - depth),
                _ => (1, 0),
            }
        });
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
            // Locked art, art on a locked or hidden layer, and text whose object is gone, can't
            // be rewritten: skipped and counted, like the rest. A hidden object is rewritten: a
            // row that hid it shows it again.
            if !d.variable_target(id) {
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
                (VariableKind::Image, DataValue::Image(path)) => links::set_image_from_path(d, id, path).is_ok(),
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
        Ok((applied, skipped, typed))
    })?;
    // Two text variables on one object rewrite it twice; it is named once.
    typed.sort_unstable();
    typed.dedup();
    Ok(super::typecmd::with_overflow(s, &typed, json!({"name": name, "applied": applied, "skipped": skipped})))
}

fn dataset_select(s: &mut Session, p: &Value) -> Result<Value> {
    apply_dataset(s, &var_name(p, "dataset.select")?)
}

/// Bytes of a datasets CSV from `path`, `data`, `dataBase64` or `csv`.
fn datasets_csv_bytes(p: &Value, cmd: &str) -> Result<Vec<u8>> {
    let bytes = match (str_param(p, "path"), str_param(p, "data"), str_param(p, "dataBase64"), str_param(p, "csv")) {
        // One byte past the cap is enough to refuse the file below.
        (Some(path), ..) => read_file_head(&absolute_path(path), MAX_CSV_BYTES + 1)?,
        (None, Some(text), ..) => text.as_bytes().to_vec(),
        (None, None, Some(b64), _) => vectorcraft_format::base64_decode(b64).ok_or_else(|| bad(cmd, "bad dataBase64"))?,
        (None, None, None, Some(csv)) => csv.as_bytes().to_vec(),
        _ => return Err(bad(cmd, "give `path`, `data`, `dataBase64` or `csv`")),
    };
    if bytes.len() > MAX_CSV_BYTES {
        return Err(bad(cmd, format!("CSV is larger than {MAX_CSV_BYTES} bytes")));
    }
    Ok(bytes)
}

/// Parse a datasets CSV into rows of cells (no header interpretation yet): commas between cells,
/// `"` quotes a cell (doubled inside), which may then hold commas and line breaks, as
/// [`csv_escape`] writes them. An unquoted cell is trimmed. A quoted cell keeps what its quotes
/// hold as written, spaces included, and drops white space after the closing quote at the end
/// of the cell. Blank lines are skipped.
fn parse_datasets_csv(text: &str, cmd: &str) -> Result<Vec<Vec<String>>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let (mut rows, mut row, mut cur) = (Vec::new(), Vec::<String>::new(), String::new());
    // `closed`: where the closing quote of a quoted cell falls in `cur`.
    let (mut in_quotes, mut closed, mut line) = (false, None, 1usize);
    let mut chars = text.chars().peekable();
    loop {
        let c = chars.next();
        match c {
            Some('"') if in_quotes => {
                if chars.peek() == Some(&'"') {
                    cur.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                    closed = Some(cur.len());
                }
            }
            Some('"') if cur.is_empty() => in_quotes = true,
            Some(',') if !in_quotes => row.push(end_csv_cell(&mut cur, &mut closed)),
            Some('\r') if !in_quotes => {}
            Some('\n') | None if !in_quotes => {
                row.push(end_csv_cell(&mut cur, &mut closed));
                if row.iter().any(|c| !c.trim().is_empty()) {
                    rows.push(std::mem::take(&mut row));
                    // +1 for the header.
                    if rows.len() > MAX_CSV_ROWS + 1 {
                        return Err(bad(cmd, format!("more than {MAX_CSV_ROWS} data rows")));
                    }
                }
                row.clear();
                if c.is_none() {
                    return Ok(rows);
                }
                line += 1;
            }
            None => return Err(bad(cmd, format!("line {line}: unclosed quote"))),
            Some(c) => {
                line += usize::from(c == '\n');
                cur.push(c);
            }
        }
        if row.len() > MAX_CSV_COLS {
            return Err(bad(cmd, format!("line {line}: more than {MAX_CSV_COLS} columns")));
        }
    }
}

/// Take the cell read so far out of `cur`. Without a closing quote (`closed` is `None`) the cell
/// is trimmed; with one, the text before it is kept as written and the text after it is
/// trimmed at the end.
fn end_csv_cell(cur: &mut String, closed: &mut Option<usize>) -> String {
    let cell = std::mem::take(cur);
    match closed.take() {
        Some(at) => match cell.split_at_checked(at) {
            Some((quoted, after)) => format!("{quoted}{}", after.trim_end()),
            None => cell,
        },
        None => cell.trim().to_string(),
    }
}

fn csv_escape(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) || s != s.trim() { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

/// A visibility cell: true/false, 1/0, yes/no (ASCII, case-insensitive).
fn parse_visible_cell(s: &str, var: &str, cmd: &str) -> Result<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => Err(bad(cmd, format!("value for `{var}` must be true or false"))),
    }
}

fn cell_to_value(kind: VariableKind, cell: &str, var: &str, cmd: &str) -> Result<Option<DataValue>> {
    // A text cell is not trimmed here (the parser trims unquoted cells); image, graph and
    // visibility cells are trimmed.
    let t = if kind == VariableKind::Text { cell } else { cell.trim() };
    if t.is_empty() {
        return Ok(None);
    }
    Ok(Some(match kind {
        VariableKind::Text => DataValue::Text(t.to_string()),
        VariableKind::Image => DataValue::Image(t.to_string()),
        VariableKind::Graph if t.len() <= MAX_GRAPH_CSV => DataValue::Graph(t.to_string()),
        VariableKind::Graph => return Err(bad(cmd, format!("value for `{var}` is longer than {MAX_GRAPH_CSV} bytes"))),
        VariableKind::Visibility => DataValue::Visible(parse_visible_cell(t, var, cmd)?),
    }))
}

fn value_to_cell(v: &DataValue) -> String {
    match v {
        DataValue::Text(t) | DataValue::Image(t) | DataValue::Graph(t) => t.clone(),
        DataValue::Visible(b) => if *b { "true" } else { "false" }.to_string(),
    }
}

/// Build the CSV of every dataset (header `name` then variables in definition order).
fn datasets_to_csv(vars: &Variables) -> String {
    let mut out = String::from("name");
    for v in &vars.variables {
        out.push(',');
        out.push_str(&csv_escape(&v.name));
    }
    out.push('\n');
    for ds in &vars.datasets {
        out.push_str(&csv_escape(&ds.name));
        for v in &vars.variables {
            out.push(',');
            if let Some(val) = ds.values.get(&v.name) {
                out.push_str(&csv_escape(&value_to_cell(val)));
            }
        }
        out.push('\n');
    }
    out
}

/// Import datasets from CSV: first column `name`, remaining headers are variable names.
/// Existing rows of the same name are replaced; new names are appended. One undo step.
fn dataset_import(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "dataset.import";
    let bytes = datasets_csv_bytes(p, C)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| bad(C, "CSV is not UTF-8"))?;
    let rows = parse_datasets_csv(text, C)?;
    let Some(header) = rows.first() else {
        return Err(bad(C, "CSV is empty"));
    };
    if header.first().map(|h| h.trim().eq_ignore_ascii_case("name")) != Some(true) {
        return Err(bad(C, "first header cell must be `name`"));
    }
    let (kinds, existing_names) = {
        let st = s.doc()?;
        let kinds: BTreeMap<String, VariableKind> = st.doc.variables.variables.iter().map(|v| (v.name.clone(), v.kind)).collect();
        let existing: Vec<String> = st.doc.variables.datasets.iter().map(|d| d.name.clone()).collect();
        (kinds, existing)
    };
    let mut columns: Vec<(usize, String, VariableKind)> = Vec::new();
    for (i, h) in header.iter().enumerate().skip(1) {
        let name = h.trim();
        if name.is_empty() {
            return Err(bad(C, format!("empty header in column {}", i + 1)));
        }
        let kind = kinds.get(name).copied().ok_or_else(|| bad(C, format!("no variable named `{name}`")))?;
        columns.push((i, name.to_string(), kind));
    }
    if rows.len() == 1 {
        return Err(bad(C, "CSV has a header but no data rows"));
    }
    let mut parsed: Vec<DataSet> = Vec::new();
    for (r_i, row) in rows.iter().enumerate().skip(1) {
        let line = r_i + 1;
        let raw_name = row.first().map(|c| c.trim()).unwrap_or("");
        let name = if raw_name.is_empty() {
            // Same numbering as Capture Data Set when a row leaves the name blank.
            (1..)
                .map(|n| format!("Data Set {n}"))
                .find(|n| parsed.iter().all(|d| d.name != *n) && !existing_names.iter().any(|e| e == n))
                .unwrap_or_else(|| format!("Data Set {line}"))
        } else {
            named(Some(raw_name), "name", C)?
        };
        if parsed.iter().any(|d| d.name == name) {
            return Err(bad(C, format!("line {line}: dataset `{name}` appears twice")));
        }
        let mut values = BTreeMap::new();
        for &(col, ref var, kind) in &columns {
            let cell = row.get(col).map(String::as_str).unwrap_or("");
            match cell_to_value(kind, cell, var, C) {
                Ok(Some(v)) => {
                    values.insert(var.clone(), v);
                }
                Ok(None) => {}
                Err(_) => return Err(bad(C, format!("line {line}: value for `{var}` must match its variable kind"))),
            }
        }
        parsed.push(DataSet { name, values });
    }
    let names: Vec<String> = parsed.iter().map(|d| d.name.clone()).collect();
    let (imported, updated) = s.edit("Import Data Sets", |d, _| {
        let mut imported = 0usize;
        let mut updated = 0usize;
        for row in parsed {
            if let Some(existing) = d.variables.datasets.iter_mut().find(|x| x.name == row.name) {
                *existing = row;
                updated += 1;
            } else {
                d.variables.datasets.push(row);
                imported += 1;
            }
        }
        Ok((imported, updated))
    })?;
    Ok(json!({"imported": imported, "updated": updated, "names": names}))
}

/// Export every dataset as CSV (`name` then variables). With `path`, write the file; else return `{csv}`.
fn dataset_export(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "dataset.export";
    let st = s.doc()?;
    if st.doc.variables.variables.is_empty() {
        return Err(bad(C, "define a variable first"));
    }
    let csv = datasets_to_csv(&st.doc.variables);
    if csv.len() > MAX_CSV_BYTES {
        return Err(bad(C, format!("CSV would be larger than {MAX_CSV_BYTES} bytes")));
    }
    match str_param(p, "path") {
        Some(path) => {
            let path = absolute_path(path);
            write_file(&path, csv.as_bytes())?;
            Ok(json!({"path": path, "datasets": st.doc.variables.datasets.len()}))
        }
        None => Ok(json!({"csv": csv, "datasets": st.doc.variables.datasets.len()})),
    }
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
            "{name, kind: text|visibility|image|graph} define a variable (graph: CSV data on a graph root)",
            has_doc,
            define
        ),
        cmd!(
            "variable.delete",
            "Delete Variable",
            ["Window", "Variables"],
            None,
            "{names?} delete variables with their bindings and values (default: the highlighted one)",
            has_doc,
            delete
        ),
        cmd!(
            "variable.rename",
            "Rename Variable",
            ["Window", "Variables"],
            None,
            "{name, newName} rename a variable, with its bindings and its values in every dataset",
            has_doc,
            rename
        ),
        cmd!(query "variable.list", "List Variables", [], None, "{} → variables with kinds and binding counts", has_doc, list),
        cmd!(
            "variable.makeTextDynamic",
            "Make Text Dynamic",
            ["Window", "Variables"],
            None,
            "{} define a text variable over the selection, named after it and bound to it",
            has_selection,
            |s, _| make_dynamic(s, VariableKind::Text, "Text")
        ),
        cmd!(
            "variable.makeVisibilityDynamic",
            "Make Visibility Dynamic",
            ["Window", "Variables"],
            None,
            "{} define a visibility variable over the selection, named after it and bound to it",
            has_selection,
            |s, _| make_dynamic(s, VariableKind::Visibility, "Visibility")
        ),
        cmd!(
            "variable.makeImageDynamic",
            "Make Image Dynamic",
            ["Window", "Variables"],
            None,
            "{} define an image variable over the selection, named after it and bound to it",
            has_selection,
            |s, _| make_dynamic(s, VariableKind::Image, "Image")
        ),
        cmd!(
            "variable.makeGraphDynamic",
            "Make Graph Dynamic",
            ["Window", "Variables"],
            None,
            "{} define a graph variable over the selected graph root(s), named after it and bound to it; each dataset row supplies CSV as graph.setData",
            has_selection,
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
            "{all?: bool} write what the bound objects hold now into the active dataset, for each variable the dataset holds; a variable whose objects report no value (locked, inside a hidden or locked layer or group, or gone) keeps the dataset's value. `all: true` also adds every other variable the objects report a value for; `left` names the variables it didn't write → {values, left}",
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
            "{name} apply a dataset in one undo step, deselecting every selected object left hidden or locked (the key object too) → {applied, skipped, overflow?: [id…] (area type and type on a path whose text it rewrote and that no longer fits; left out when all fits)}",
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
            "{path? | data? | dataBase64? | csv?} load datasets from CSV (header: name, then variable names; one row per dataset). Rows whose name already exists are replaced → {imported, updated, names}",
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
    ]
}
