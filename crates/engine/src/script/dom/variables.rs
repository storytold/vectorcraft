//! Variables and data sets in the script object model: `Document.variables`, `dataSets`,
//! `activeDataset`, `variablesLocked`, `importVariables` and `exportVariables`, the `Variables`,
//! `Variable`, `Datasets` and `Dataset` classes, and the variables page items are bound to, as the
//! help pages <https://ai-scripting.docsforadobe.dev/jsobjref/Variables/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/Variable/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/Datasets/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/Dataset/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/Document/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/PageItem/> and
//! <https://ai-scripting.docsforadobe.dev/jsobjref/TextFrameItem/> describe them. Every change runs
//! one of the `variable.*` and `dataset.*` commands.
//!
//! VectorCraft's own rules, which the help pages do not state:
//! - A `Variable` or a `Dataset` names its variable or data set by name. A rename or kind change
//!   made through the object keeps the object working; once the name leaves the document any other
//!   way (a delete, a rename by another command, an undo, a kind change by another command, or an
//!   import that replaces the data sets), the object throws [`STALE_VARIABLE`] or
//!   [`STALE_DATASET`] on every member, also after the name comes back. A run holds at most
//!   [`MAX_HANDLES`] of each.
//! - Collections list variables and data sets in the order the document holds them, counted from
//!   0. An index out of range gives `undefined`; `index` and `getByName` throw `Not found`.
//! - `Variables.add()` defines a text variable named `Variable1`, `Variable2` and so on, the first
//!   name that is free. `Datasets.add()` captures what the bound objects hold now as a new data
//!   set named `Data Set 1` and so on, which becomes the active one.
//! - Kinds are the strings `VariableKind.TEXTUAL`, `VariableKind.VISIBILITY`, `VariableKind.IMAGE`,
//!   `VariableKind.GRAPH` and `VariableKind.Unknown`; `Unknown` is the kind of a variable a file
//!   declared without one VectorCraft knows, and cannot be given.
//! - `Variable.pageItems` is a new array of the objects bound to the variable, in id order.
//! - `activeDataset` is `null` with no active data set; assigning a `Dataset` of the document shows
//!   its values in the art, as `display()` does.
//! - `visibilityVariable` and `contentVariable` give the first variable of their kind bound to the
//!   object, in the order the variables are defined, or `null`. Assigning a `Variable` binds the
//!   object to it and unbinds the object's other variables of that kind; `null` unbinds them all.
//!   An embedded image (`RasterItem`) cannot take an image variable.
//! - `importVariables` and `exportVariables` take a `File`, not a path. The file must be one the
//!   script may read, or write, as its `File` objects may: the files of the open documents do not
//!   count.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use serde_json::json;
use vectorcraft_js::{Data, HostClass, HostError, HostItem, MemberInfo, Realm, RunError, file_ref_path, host_ref};

use super::{
    Answer, Described, Doc, Host, Link, MAX_LISTED, MAX_TEXT_BYTES, NOT_FOUND, arg, class, command, doc_of, doc_ref, edit_item, find_doc, float,
    function, get, handle_of, item_of, item_ref, method, named, num, on_doc, property, returns, text, typed, typename, whole, with_doc,
};
use crate::doc::{DataSet, Node, Variable, VariableKind};
use crate::file_access::Access;

/// The error for a `Variable` whose variable left the document, or that came from an earlier run.
pub const STALE_VARIABLE: &str = "Stale variable";
/// The error for a `Dataset` whose data set left the document, or that came from an earlier run.
pub const STALE_DATASET: &str = "Stale data set";
/// The most `Variable` objects, and the most `Dataset` objects, one run gives out.
pub const MAX_HANDLES: usize = 10_000;

/// The kinds, each with the name of its `VariableKind` member.
const KINDS: [(VariableKind, &str); 5] = [
    (VariableKind::Text, "TEXTUAL"),
    (VariableKind::Visibility, "VISIBILITY"),
    (VariableKind::Image, "IMAGE"),
    (VariableKind::Graph, "GRAPH"),
    (VariableKind::Unknown, "Unknown"),
];

/// What a `Variable` or `Dataset` object names: the document's uid, the name, the kind of a
/// variable (`None` for a data set) and whether it still names it.
struct Entry {
    uid: u64,
    name: String,
    kind: Option<VariableKind>,
    alive: bool,
}

/// The objects of one class a run gave out: each one's handle is its index here.
#[derive(Default)]
struct Table {
    entries: Vec<Entry>,
    /// The entry that names each document's name, while it does.
    live: HashMap<(u64, String), usize>,
}

impl Table {
    /// The entry for `name` of the document `uid`, made when there is none.
    fn slot(&mut self, uid: u64, name: &str, kind: Option<VariableKind>, what: &str) -> std::result::Result<usize, HostError> {
        if let Some(&at) = self.live.get(&(uid, name.to_string()))
            && self.entries.get(at).is_some_and(|e| e.alive && e.kind == kind)
        {
            return Ok(at);
        }
        if self.entries.len() >= MAX_HANDLES {
            return Err(HostError::Range(format!("a script can hold at most {MAX_HANDLES} {what} objects in one run")));
        }
        self.entries.push(Entry { uid, name: name.to_string(), kind, alive: true });
        let at = self.entries.len() - 1;
        self.live.insert((uid, name.to_string()), at);
        Ok(at)
    }

    /// The entry `at` no longer names anything.
    fn kill(&mut self, at: usize) {
        if let Some(e) = self.entries.get_mut(at) {
            e.alive = false;
            let k = (e.uid, std::mem::take(&mut e.name));
            if self.live.get(&k) == Some(&at) {
                self.live.remove(&k);
            }
        }
    }

    /// The entry `at` names `name`, of `kind`, from now on: a rename made through it.
    fn rename(&mut self, at: usize, name: &str, kind: Option<VariableKind>) {
        let Some(uid) = self.entries.get(at).map(|e| e.uid) else { return };
        self.kill(at);
        if let Some(e) = self.entries.get_mut(at) {
            *e = Entry { uid, name: name.to_string(), kind, alive: true };
        }
        self.live.insert((uid, name.to_string()), at);
    }

    /// Kills each live entry `keep` refuses.
    fn sweep(&mut self, keep: impl Fn(&Entry) -> bool) {
        let gone: Vec<usize> = self.live.values().copied().filter(|at| self.entries.get(*at).is_none_or(|e| !keep(e))).collect();
        for at in gone {
            self.kill(at);
        }
    }
}

/// The `Variable` and `Dataset` objects of a run, and each document's revision when they were last
/// checked against it.
#[derive(Default)]
pub(super) struct Handles {
    vars: RefCell<Table>,
    sets: RefCell<Table>,
    seen: RefCell<Vec<(u64, u64)>>,
}

/// The data sets of the active document before a command that may replace them (an import), while
/// the run holds a `Dataset`; `None` for any other command.
pub(super) fn before_command(h: &Host, id: &str) -> Option<(u64, Vec<DataSet>)> {
    if !matches!(id, "dataset.import" | "variable.loadLibrary" | "variable.import") || h.dom.vars.sets.borrow().live.is_empty() {
        return None;
    }
    let s = h.session.try_borrow().ok()?;
    s.active().map(|st| (st.uid, st.doc.variables.datasets.clone()))
}

/// After each command a script runs: the objects whose variable or data set it removed stop
/// working, so one made again under the same name does not bring them back. An import that leaves
/// its document with other data sets than `before` ([`before_command`]) holds stops every `Dataset`
/// of that document, also when the new data sets reuse the names; an import that failed, was put
/// back or left the data sets as they were stops none.
pub(super) fn after_command(h: &Host, before: Option<(u64, Vec<DataSet>)>) {
    let hs = &h.dom.vars;
    if hs.vars.borrow().live.is_empty() && hs.sets.borrow().live.is_empty() {
        return;
    }
    let Ok(s) = h.session.try_borrow() else { return };
    let replaced = before.and_then(|(uid, sets)| s.documents().iter().find(|d| d.uid == uid && d.doc.variables.datasets != sets).map(|d| d.uid));
    let now: Vec<(u64, u64)> = s.documents().iter().map(|d| (d.uid, d.revision)).collect();
    let changed: HashSet<u64> = {
        let seen = hs.seen.borrow();
        now.iter().filter(|n| !seen.contains(n)).chain(seen.iter().filter(|o| !now.iter().any(|n| n.0 == o.0))).map(|(uid, _)| *uid).collect()
    };
    *hs.seen.borrow_mut() = now;
    if changed.is_empty() && replaced.is_none() {
        return;
    }
    let mut kinds: HashMap<u64, HashMap<&str, VariableKind>> = HashMap::new();
    let mut sets: HashMap<u64, HashSet<&str>> = HashMap::new();
    for st in s.documents().iter().filter(|d| changed.contains(&d.uid)) {
        let vars = &st.doc.variables;
        kinds.insert(st.uid, vars.variables.iter().map(|v| (v.name.as_str(), v.kind)).collect());
        sets.insert(st.uid, vars.datasets.iter().map(|d| d.name.as_str()).collect());
    }
    hs.vars.borrow_mut().sweep(|e| !changed.contains(&e.uid) || kinds.get(&e.uid).and_then(|k| k.get(e.name.as_str())).copied() == e.kind);
    hs.sets
        .borrow_mut()
        .sweep(|e| Some(e.uid) != replaced && (!changed.contains(&e.uid) || sets.get(&e.uid).is_some_and(|n| n.contains(e.name.as_str()))));
}

fn kind_text(k: VariableKind) -> Data {
    let name = KINDS.iter().find(|(kind, _)| *kind == k).map_or("Unknown", |(_, n)| n);
    text(format!("VariableKind.{name}"))
}

fn kind_of(d: &Data) -> std::result::Result<VariableKind, HostError> {
    let found = match d {
        Data::String(s) => s.strip_prefix("VariableKind.").and_then(|m| KINDS.iter().find(|(_, n)| *n == m)).map(|(k, _)| *k),
        _ => None,
    };
    match found {
        Some(VariableKind::Unknown) => Err(HostError::Error("a variable cannot be given the kind VariableKind.Unknown".into())),
        Some(k) => Ok(k),
        None => Err(HostError::Type("kind must be a VariableKind value".into())),
    }
}

/// A name a script gives: a string of at most [`MAX_TEXT_BYTES`] bytes.
fn name_arg<'a>(d: &'a Data, what: &str) -> std::result::Result<&'a str, HostError> {
    let Data::String(s) = d else { return Err(HostError::Type(format!("{what} must be a string"))) };
    if s.len() > MAX_TEXT_BYTES {
        return Err(HostError::Range(format!("the {what} is too long")));
    }
    Ok(s)
}

/// The `name` field of a command's result.
fn result_name(d: &Data) -> Option<String> {
    let Data::Object(o) = d else { return None };
    o.iter().find(|(k, _)| k == "name").and_then(|(_, v)| if let Data::String(s) = v { Some(s.clone()) } else { None })
}

fn var_ref(h: &Host, uid: u64, v: &Variable) -> Answer {
    let at = h.dom.vars.vars.borrow_mut().slot(uid, &v.name, Some(v.kind), "Variable")?;
    Ok(host_ref("Variable", &[float(h.run), float(at as u64)]))
}

fn set_ref(h: &Host, uid: u64, name: &str) -> Answer {
    let at = h.dom.vars.sets.borrow_mut().slot(uid, name, None, "Dataset")?;
    Ok(host_ref("Dataset", &[float(h.run), float(at as u64)]))
}

/// A variable or data set an object names: its document, its handle's index and its name.
struct Named {
    doc: Doc,
    at: usize,
    name: String,
    kind: Option<VariableKind>,
}

/// What the `Variable` (`variable`) or `Dataset` object `d` names, which must still be in its
/// document.
fn named_of(h: &Host, d: &Data, variable: bool) -> std::result::Result<Named, HostError> {
    let (class, stale) = if variable { ("Variable", STALE_VARIABLE) } else { ("Dataset", STALE_DATASET) };
    let Some(handle) = handle_of(d, class) else { return Err(HostError::Type(format!("this is not a {class}"))) };
    let &[run, at] = handle.as_slice() else { return Err(HostError::Type(format!("this is not a {class}"))) };
    let stale = || HostError::Error(stale.into());
    if whole(run) != Some(h.run) {
        return Err(stale());
    }
    let at = whole(at).and_then(|a| usize::try_from(a).ok()).ok_or_else(stale)?;
    let table = if variable { &h.dom.vars.vars } else { &h.dom.vars.sets };
    let (uid, name, kind) = {
        let t = table.borrow();
        let e = t.entries.get(at).filter(|e| e.alive).ok_or_else(stale)?;
        (e.uid, e.name.clone(), e.kind)
    };
    let doc = find_doc(h, float(h.run), float(uid));
    let there = doc.is_some_and(|doc| {
        with_doc(h, doc, |_, st| {
            let vars = &st.doc.variables;
            if variable { vars.variable(&name).is_some_and(|v| Some(v.kind) == kind) } else { vars.dataset(&name).is_some() }
        })
    });
    match doc {
        Some(doc) if there => Ok(Named { doc, at, name, kind }),
        _ => {
            table.borrow_mut().kill(at);
            Err(stale())
        }
    }
}

fn this_var(h: &Host, a: &[Data]) -> std::result::Result<Named, HostError> {
    named_of(h, arg(a, 0), true)
}

fn this_set(h: &Host, a: &[Data]) -> std::result::Result<Named, HostError> {
    named_of(h, arg(a, 0), false)
}

/// The index argument of a collection's indexer: a whole number, or `None`.
fn index_arg(d: &Data) -> Option<usize> {
    match d {
        Data::Number(n) => whole(*n).and_then(|i| usize::try_from(i).ok()),
        _ => None,
    }
}

// ---- Document ----

fn this_doc(h: &Host, a: &[Data]) -> std::result::Result<Doc, HostError> {
    doc_of(h, arg(a, 0), "Document")
}

fn doc_variables(h: &Host, a: &[Data]) -> Answer {
    let doc = this_doc(h, a)?;
    Ok(host_ref("Variables", &[float(h.run), float(doc.uid)]))
}

fn doc_datasets(h: &Host, a: &[Data]) -> Answer {
    let doc = this_doc(h, a)?;
    Ok(host_ref("Datasets", &[float(h.run), float(doc.uid)]))
}

fn doc_locked(h: &Host, a: &[Data]) -> Answer {
    let doc = this_doc(h, a)?;
    Ok(Data::Bool(with_doc(h, doc, |_, st| st.doc.variables.locked)))
}

/// `variablesLocked = flag`: `variable.lock`, with no command when it is already so.
fn doc_set_locked(h: &Host, a: &[Data]) -> Answer {
    let doc = this_doc(h, a)?;
    let Data::Bool(locked) = arg(a, 1) else { return Err(HostError::Type("variablesLocked must be true or false".into())) };
    if with_doc(h, doc, |_, st| st.doc.variables.locked) != *locked {
        on_doc(h, doc.uid, false, |h| command(h, "variable.lock", &json!({"locked": locked})))?;
    }
    Ok(Data::Undefined)
}

fn doc_active_dataset(h: &Host, a: &[Data]) -> Answer {
    let doc = this_doc(h, a)?;
    match with_doc(h, doc, |_, st| st.doc.variables.active_dataset.clone()) {
        Some(name) => set_ref(h, doc.uid, &name),
        None => Ok(Data::Null),
    }
}

/// Shows the data set `set` in the art with `dataset.select`, with no command when it is the active
/// one and the art already shows it.
fn display(h: &Host, set: &Named) -> Answer {
    let shown = with_doc(h, set.doc, |_, st| {
        let vars = &st.doc.variables;
        vars.active_dataset.as_deref() == Some(set.name.as_str()) && vars.dataset(&set.name).is_some_and(|d| d.matches(&st.doc.captured_values()))
    });
    if !shown {
        on_doc(h, set.doc.uid, false, |h| command(h, "dataset.select", &json!({"name": set.name})))?;
    }
    Ok(Data::Undefined)
}

fn doc_set_active_dataset(h: &Host, a: &[Data]) -> Answer {
    let doc = this_doc(h, a)?;
    if handle_of(arg(a, 1), "Dataset").is_none() {
        return Err(HostError::Type("activeDataset must be a Dataset".into()));
    }
    let set = named_of(h, arg(a, 1), false)?;
    if set.doc.uid != doc.uid {
        return Err(HostError::Error("the data set is in another document".into()));
    }
    display(h, &set)
}

/// The path of the `File` argument of `what`, which must be a file the script may reach for
/// `access` through its own `File` objects.
fn file_arg(h: &Host, a: &[Data], what: &str, access: Access) -> std::result::Result<String, HostError> {
    let Some(path) = file_ref_path(arg(a, 1)) else { return Err(HostError::Type(format!("{what} needs a File"))) };
    if path.chars().count() > super::super::files::MAX_PATH_CHARS {
        return Err(HostError::Range(format!("{what}: the path is too long")));
    }
    let gate = h.disk.borrow().gate(Vec::new(), crate::file_access::LinkedFiles::default());
    gate.check(path, access).map_err(HostError::Error)?;
    Ok(path.to_string())
}

/// `importVariables(file)`: `variable.loadLibrary` with the file, which replaces the document's
/// variables and data sets with the library's.
fn doc_import(h: &Host, a: &[Data]) -> Answer {
    let doc = this_doc(h, a)?;
    let path = file_arg(h, a, "importVariables", Access::Read)?;
    on_doc(h, doc.uid, false, |h| command(h, "variable.loadLibrary", &json!({"path": path})))?;
    Ok(Data::Undefined)
}

/// `exportVariables(file)`: `variable.saveLibrary` into the file.
fn doc_export(h: &Host, a: &[Data]) -> Answer {
    let doc = this_doc(h, a)?;
    let path = file_arg(h, a, "exportVariables", Access::Write)?;
    on_doc(h, doc.uid, false, |h| command(h, "variable.saveLibrary", &json!({"path": path})))?;
    Ok(Data::Undefined)
}

// ---- Variables ----

fn vars_doc(h: &Host, a: &[Data]) -> std::result::Result<Doc, HostError> {
    doc_of(h, arg(a, 0), "Variables")
}

fn vars_typename(h: &Host, a: &[Data]) -> Answer {
    vars_doc(h, a)?;
    Ok(text("Variables"))
}

fn vars_length(h: &Host, a: &[Data]) -> Answer {
    let doc = vars_doc(h, a)?;
    Ok(num(float(with_doc(h, doc, |_, st| st.doc.variables.variables.len()) as u64)))
}

fn vars_parent(h: &Host, a: &[Data]) -> Answer {
    let doc = vars_doc(h, a)?;
    Ok(doc_ref(h, doc.uid))
}

fn vars_at(h: &Host, a: &[Data]) -> Answer {
    let doc = vars_doc(h, a)?;
    let Some(i) = index_arg(arg(a, 1)) else { return Ok(Data::Undefined) };
    match with_doc(h, doc, |_, st| st.doc.variables.variables.get(i).cloned()) {
        Some(v) => var_ref(h, doc.uid, &v),
        None => Ok(Data::Undefined),
    }
}

fn vars_by_name(h: &Host, a: &[Data]) -> Answer {
    let doc = vars_doc(h, a)?;
    let name = name_arg(arg(a, 1), "name")?;
    let found = with_doc(h, doc, |_, st| {
        h.charge_call(st.doc.variables.variables.len() as u64)?;
        Ok::<_, HostError>(st.doc.variables.variable(name).cloned())
    })?;
    found.map_or_else(|| Err(HostError::Error(NOT_FOUND.into())), |v| var_ref(h, doc.uid, &v))
}

fn vars_index(h: &Host, a: &[Data]) -> Answer {
    match arg(a, 1) {
        Data::String(_) => vars_by_name(h, a),
        Data::Number(_) => match vars_at(h, a)? {
            Data::Undefined => Err(HostError::Error(NOT_FOUND.into())),
            d => Ok(d),
        },
        _ => Err(HostError::Type("give a name or an index".into())),
    }
}

/// `variables.add()`: `variable.define` of a text variable named `Variable1`, `Variable2`… (the
/// first free name). → the new variable.
fn vars_add(h: &Host, a: &[Data]) -> Answer {
    let doc = vars_doc(h, a)?;
    let name = with_doc(h, doc, |_, st| {
        let vars = &st.doc.variables;
        (1..=vars.variables.len() + 1).map(|i| format!("Variable{i}")).find(|n| vars.variable(n).is_none()).unwrap_or_default()
    });
    on_doc(h, doc.uid, false, |h| command(h, "variable.define", &json!({"name": name, "kind": "text"})))?;
    var_ref(h, doc.uid, &Variable { name, kind: VariableKind::Text })
}

/// `variables.removeAll()`: one `variable.delete` of every variable, with no command when there is
/// none.
fn vars_remove_all(h: &Host, a: &[Data]) -> Answer {
    let doc = vars_doc(h, a)?;
    let names: Vec<String> = with_doc(h, doc, |_, st| st.doc.variables.variables.iter().map(|v| v.name.clone()).collect());
    if !names.is_empty() {
        on_doc(h, doc.uid, false, |h| command(h, "variable.delete", &json!({"names": names})))?;
    }
    Ok(Data::Undefined)
}

// ---- Variable ----

fn var_typename(h: &Host, a: &[Data]) -> Answer {
    this_var(h, a)?;
    Ok(text("Variable"))
}

fn var_parent(h: &Host, a: &[Data]) -> Answer {
    let v = this_var(h, a)?;
    Ok(doc_ref(h, v.doc.uid))
}

fn var_name(h: &Host, a: &[Data]) -> Answer {
    Ok(text(this_var(h, a)?.name))
}

/// `name = text`: `variable.rename`, which trims it, with no command when the name stays.
fn var_set_name(h: &Host, a: &[Data]) -> Answer {
    let v = this_var(h, a)?;
    let to = name_arg(arg(a, 1), "name")?;
    if to.trim() == v.name {
        return Ok(Data::Undefined);
    }
    let r = on_doc(h, v.doc.uid, false, |h| command(h, "variable.rename", &json!({"name": v.name, "newName": to})))?;
    let name = result_name(&r).unwrap_or_else(|| to.trim().to_string());
    h.dom.vars.vars.borrow_mut().rename(v.at, &name, v.kind);
    Ok(Data::Undefined)
}

fn var_kind(h: &Host, a: &[Data]) -> Answer {
    let v = this_var(h, a)?;
    Ok(kind_text(v.kind.unwrap_or(VariableKind::Unknown)))
}

/// `kind = VariableKind.…`: `variable.rename` with the kind, which unbinds the objects that cannot
/// hold it and drops the variable's values, with no command when the kind stays.
fn var_set_kind(h: &Host, a: &[Data]) -> Answer {
    let v = this_var(h, a)?;
    let kind = kind_of(arg(a, 1))?;
    if Some(kind) == v.kind {
        return Ok(Data::Undefined);
    }
    on_doc(h, v.doc.uid, false, |h| command(h, "variable.rename", &json!({"name": v.name, "kind": kind.label()})))?;
    h.dom.vars.vars.borrow_mut().rename(v.at, &v.name, Some(kind));
    Ok(Data::Undefined)
}

fn var_page_items(h: &Host, a: &[Data]) -> Answer {
    let v = this_var(h, a)?;
    with_doc(h, v.doc, |_, st| {
        let ids: Vec<crate::NodeId> =
            st.doc.variables.objects_of(&v.name).into_iter().filter(|i| st.doc.node(*i).and_then(typename).is_some()).collect();
        if ids.len() > MAX_LISTED {
            return Err(HostError::Range(format!("the variable is bound to more than {MAX_LISTED} objects")));
        }
        h.charge_call(16 * ids.len() as u64)?;
        Ok(Data::Array(ids.iter().map(|i| item_ref(h, st, *i)).collect()))
    })
}

/// `variable.remove()`: `variable.delete` of the variable, with its bindings and values.
fn var_remove(h: &Host, a: &[Data]) -> Answer {
    let v = this_var(h, a)?;
    on_doc(h, v.doc.uid, false, |h| command(h, "variable.delete", &json!({"names": [v.name]})))?;
    h.dom.vars.vars.borrow_mut().kill(v.at);
    Ok(Data::Undefined)
}

// ---- Datasets ----

fn sets_doc(h: &Host, a: &[Data]) -> std::result::Result<Doc, HostError> {
    doc_of(h, arg(a, 0), "Datasets")
}

fn sets_typename(h: &Host, a: &[Data]) -> Answer {
    sets_doc(h, a)?;
    Ok(text("Datasets"))
}

fn sets_length(h: &Host, a: &[Data]) -> Answer {
    let doc = sets_doc(h, a)?;
    Ok(num(float(with_doc(h, doc, |_, st| st.doc.variables.datasets.len()) as u64)))
}

fn sets_parent(h: &Host, a: &[Data]) -> Answer {
    let doc = sets_doc(h, a)?;
    Ok(doc_ref(h, doc.uid))
}

fn sets_at(h: &Host, a: &[Data]) -> Answer {
    let doc = sets_doc(h, a)?;
    let Some(i) = index_arg(arg(a, 1)) else { return Ok(Data::Undefined) };
    match with_doc(h, doc, |_, st| st.doc.variables.datasets.get(i).map(|d| d.name.clone())) {
        Some(name) => set_ref(h, doc.uid, &name),
        None => Ok(Data::Undefined),
    }
}

fn sets_by_name(h: &Host, a: &[Data]) -> Answer {
    let doc = sets_doc(h, a)?;
    let name = name_arg(arg(a, 1), "name")?;
    let found = with_doc(h, doc, |_, st| {
        h.charge_call(st.doc.variables.datasets.len() as u64)?;
        Ok::<_, HostError>(st.doc.variables.dataset(name).map(|d| d.name.clone()))
    })?;
    found.map_or_else(|| Err(HostError::Error(NOT_FOUND.into())), |n| set_ref(h, doc.uid, &n))
}

fn sets_index(h: &Host, a: &[Data]) -> Answer {
    match arg(a, 1) {
        Data::String(_) => sets_by_name(h, a),
        Data::Number(_) => match sets_at(h, a)? {
            Data::Undefined => Err(HostError::Error(NOT_FOUND.into())),
            d => Ok(d),
        },
        _ => Err(HostError::Type("give a name or an index".into())),
    }
}

/// `dataSets.add()`: `dataset.capture`, which records what the bound objects hold now as a new
/// data set and makes it the active one. → the new data set.
fn sets_add(h: &Host, a: &[Data]) -> Answer {
    let doc = sets_doc(h, a)?;
    let r = on_doc(h, doc.uid, false, |h| command(h, "dataset.capture", &json!({})))?;
    let name = result_name(&r).ok_or_else(|| HostError::Error("the data set was not made".into()))?;
    set_ref(h, doc.uid, &name)
}

/// `dataSets.removeAll()`: one `dataset.delete` of every data set, with no command when there is
/// none.
fn sets_remove_all(h: &Host, a: &[Data]) -> Answer {
    let doc = sets_doc(h, a)?;
    let names: Vec<String> = with_doc(h, doc, |_, st| st.doc.variables.datasets.iter().map(|d| d.name.clone()).collect());
    if !names.is_empty() {
        on_doc(h, doc.uid, false, |h| command(h, "dataset.delete", &json!({"names": names})))?;
    }
    Ok(Data::Undefined)
}

// ---- Dataset ----

fn set_typename(h: &Host, a: &[Data]) -> Answer {
    this_set(h, a)?;
    Ok(text("Dataset"))
}

fn set_parent(h: &Host, a: &[Data]) -> Answer {
    let d = this_set(h, a)?;
    Ok(doc_ref(h, d.doc.uid))
}

fn set_name(h: &Host, a: &[Data]) -> Answer {
    Ok(text(this_set(h, a)?.name))
}

/// `name = text`: `dataset.rename`, which trims it, with no command when the name stays.
fn set_set_name(h: &Host, a: &[Data]) -> Answer {
    let d = this_set(h, a)?;
    let to = name_arg(arg(a, 1), "name")?;
    if to.trim() == d.name {
        return Ok(Data::Undefined);
    }
    let r = on_doc(h, d.doc.uid, false, |h| command(h, "dataset.rename", &json!({"name": d.name, "newName": to})))?;
    let name = result_name(&r).unwrap_or_else(|| to.trim().to_string());
    h.dom.vars.sets.borrow_mut().rename(d.at, &name, None);
    Ok(Data::Undefined)
}

fn set_display(h: &Host, a: &[Data]) -> Answer {
    let d = this_set(h, a)?;
    display(h, &d)
}

/// `dataset.update()`: `dataset.update` of this data set, also when another one is active, with no
/// command when it already holds what the bound objects hold.
fn set_update(h: &Host, a: &[Data]) -> Answer {
    let d = this_set(h, a)?;
    let same = with_doc(h, d.doc, |_, st| {
        let doc = &st.doc;
        let bound = doc.variables.bindings.iter().any(|(id, names)| !names.is_empty() && doc.node(*id).is_some());
        bound && doc.variables.dataset(&d.name).is_some_and(|ds| ds.matches(&doc.captured_values()))
    });
    if !same {
        on_doc(h, d.doc.uid, false, |h| command(h, "dataset.update", &json!({"name": d.name})))?;
    }
    Ok(Data::Undefined)
}

/// `dataset.remove()`: `dataset.delete` of this data set.
fn set_remove(h: &Host, a: &[Data]) -> Answer {
    let d = this_set(h, a)?;
    on_doc(h, d.doc.uid, false, |h| command(h, "dataset.delete", &json!({"names": [d.name]})))?;
    h.dom.vars.sets.borrow_mut().kill(d.at);
    Ok(Data::Undefined)
}

// ---- Page items ----

/// The kind of variable a binding slot takes for the object `n`: visibility, or what drives its
/// content.
fn slot_kind(n: &Node, content: bool) -> std::result::Result<VariableKind, HostError> {
    if !content {
        return Ok(VariableKind::Visibility);
    }
    crate::cmd::content_kind(n).ok_or_else(|| HostError::Error("the object has no content a variable can drive".into()))
}

/// `visibilityVariable` (`content` false) or `contentVariable`: the first variable of the slot's
/// kind bound to the object, in the order the variables are defined, or `null`.
fn binding(h: &Host, a: &[Data], content: bool) -> Answer {
    let item = item_of(h, arg(a, 0))?;
    let found = with_doc(h, item.doc, |_, st| {
        let n = st.doc.node(item.id).ok_or_else(|| HostError::Error(super::STALE_ITEM.into()))?;
        let kind = slot_kind(n, content)?;
        let vars = &st.doc.variables;
        let bound = vars.bindings.get(&item.id);
        Ok::<_, HostError>(vars.variables.iter().find(|v| v.kind == kind && bound.is_some_and(|b| b.contains(&v.name))).cloned())
    })?;
    match found {
        Some(v) => var_ref(h, item.doc.uid, &v),
        None => Ok(Data::Null),
    }
}

/// Assigning `visibilityVariable` or `contentVariable`: `variable.setBinding` with a `Variable` of
/// the object's document, or with none for `null`. No command runs when the object already has just
/// that binding of the slot's kind.
fn set_binding(h: &Host, a: &[Data], content: bool) -> Answer {
    let item = item_of(h, arg(a, 0))?;
    let variable = match arg(a, 1) {
        Data::Null => None,
        d if handle_of(d, "Variable").is_some() => {
            let v = named_of(h, d, true)?;
            if v.doc.uid != item.doc.uid {
                return Err(HostError::Error("the variable is in another document".into()));
            }
            Some(v.name)
        }
        _ => return Err(HostError::Type("give a Variable or null".into())),
    };
    let same = with_doc(h, item.doc, |_, st| {
        let n = st.doc.node(item.id).ok_or_else(|| HostError::Error(super::STALE_ITEM.into()))?;
        let kind = slot_kind(n, content)?;
        let vars = &st.doc.variables;
        let bound = vars.bindings.get(&item.id);
        let held: Vec<&str> =
            vars.variables.iter().filter(|v| v.kind == kind && bound.is_some_and(|b| b.contains(&v.name))).map(|v| v.name.as_str()).collect();
        Ok::<_, HostError>(held == variable.as_deref().into_iter().collect::<Vec<_>>())
    })?;
    if !same {
        let slot = if content { "content" } else { "visibility" };
        edit_item(h, item, "variable.setBinding", &json!({"id": item.id.0, "slot": slot, "variable": variable}))?;
    }
    Ok(Data::Undefined)
}

fn item_visibility(h: &Host, a: &[Data]) -> Answer {
    binding(h, a, false)
}

fn item_set_visibility(h: &Host, a: &[Data]) -> Answer {
    set_binding(h, a, false)
}

fn item_content(h: &Host, a: &[Data]) -> Answer {
    binding(h, a, true)
}

fn item_set_content(h: &Host, a: &[Data]) -> Answer {
    set_binding(h, a, true)
}

// ---- Registration ----

/// The members this module adds to `Document`.
pub(super) fn document_members(l: &Link) -> Vec<Described> {
    named(vec![
        ("variables", get(l, doc_variables), MemberInfo { collection: true, ..typed("Variables") }),
        ("dataSets", get(l, doc_datasets), MemberInfo { collection: true, ..typed("Datasets") }),
        ("activeDataset", property(l, doc_active_dataset, doc_set_active_dataset), typed("Dataset")),
        ("variablesLocked", property(l, doc_locked, doc_set_locked), typed("boolean")),
        ("importVariables", method(l, doc_import), returns("undefined", &[("fileSpec", "File")])),
        ("exportVariables", method(l, doc_export), returns("undefined", &[("fileSpec", "File")])),
    ])
}

/// The members this module adds to `PageItem`.
pub(super) fn page_item_members(l: &Link) -> Vec<Described> {
    named(vec![("visibilityVariable", property(l, item_visibility, item_set_visibility), typed("Variable"))])
}

/// The members this module adds to the page item class `class`.
pub(super) fn item_members(l: &Link, class: &str) -> Vec<Described> {
    match class {
        "TextFrame" | "PlacedItem" | "RasterItem" | "GraphItem" => {
            named(vec![("contentVariable", property(l, item_content, item_set_content), typed("Variable"))])
        }
        _ => Vec::new(),
    }
}

/// The classes this module adds.
pub(super) fn classes(l: &Link) -> Vec<HostClass> {
    vec![
        class(
            "Variables",
            None,
            named(vec![
                ("typename", get(l, vars_typename), typed("string")),
                ("length", get(l, vars_length), typed("number")),
                ("parent", get(l, vars_parent), typed("Document")),
                ("index", method(l, vars_index), returns("Variable", &[("itemKey", "*")])),
                ("getByName", method(l, vars_by_name), returns("Variable", &[("name", "string")])),
                ("add", method(l, vars_add), returns("Variable", &[])),
                ("removeAll", method(l, vars_remove_all), returns("undefined", &[])),
            ]),
            Some(function(l, vars_at)),
        ),
        class(
            "Variable",
            None,
            named(vec![
                ("typename", get(l, var_typename), typed("string")),
                ("parent", get(l, var_parent), typed("Document")),
                ("name", property(l, var_name, var_set_name), typed("string")),
                ("kind", property(l, var_kind, var_set_kind), typed("VariableKind")),
                ("pageItems", get(l, var_page_items), typed("Array")),
                ("remove", method(l, var_remove), returns("undefined", &[])),
            ]),
            None,
        ),
        class(
            "Datasets",
            None,
            named(vec![
                ("typename", get(l, sets_typename), typed("string")),
                ("length", get(l, sets_length), typed("number")),
                ("parent", get(l, sets_parent), typed("Document")),
                ("index", method(l, sets_index), returns("Dataset", &[("itemKey", "*")])),
                ("getByName", method(l, sets_by_name), returns("Dataset", &[("name", "string")])),
                ("add", method(l, sets_add), returns("Dataset", &[])),
                ("removeAll", method(l, sets_remove_all), returns("undefined", &[])),
            ]),
            Some(function(l, sets_at)),
        ),
        class(
            "Dataset",
            None,
            named(vec![
                ("typename", get(l, set_typename), typed("string")),
                ("parent", get(l, set_parent), typed("Document")),
                ("name", property(l, set_name, set_set_name), typed("string")),
                ("display", method(l, set_display), returns("undefined", &[])),
                ("update", method(l, set_update), returns("undefined", &[])),
                ("remove", method(l, set_remove), returns("undefined", &[])),
            ]),
            None,
        ),
    ]
}

/// Adds `VariableKind`, a frozen object whose members are `"VariableKind.MEMBER"` strings.
pub(super) fn define_enums(realm: &mut Realm) -> std::result::Result<(), RunError> {
    let items = KINDS.iter().map(|(_, m)| (m.to_string(), HostItem::Value(text(format!("VariableKind.{m}"))))).collect();
    realm.define_host_object("VariableKind", items)
}
