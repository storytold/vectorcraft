//! The variable library: the document's variables, data sets and the objects bound to them, as XML
//! nodes of the `/Document` dictionary (see [`crate::ai_editing`] for the form).

use std::collections::HashSet;
use std::fmt::Write;

use vectorcraft_doc::{DataValue, Node, NodeId, NodeKind, VariableKind};

use super::Writer;
use crate::ps;

pub(in crate::ai_editing) const BINDINGS: &str = "variable bindings to layers, to type written as outlines, to graphs, and to linked images written as other objects are left out of the editing data for other apps";
pub(in crate::ai_editing) const VALUES: &str = "data set values of graphs, values that don't fit their variable, linked files that aren't absolute paths or that would read back as other paths, and values of variables whose names have a colon are left out of the editing data for other apps";
pub(in crate::ai_editing) const ACTIVE: &str =
    "the data set shown is left out of the editing data for other apps when it isn't one of the data sets written";

pub(in crate::ai_editing) const LIMITS: &str = "variables and data sets whose names are empty, repeated, longer than 255 characters or start or end with white space, variables past 256, data sets past 1,000, bindings past 100,000, bindings to objects whose XML ids are longer than 255 characters, values past 200,000 text lines and values in all, and names and values past 8,388,608 bytes in all are left out of the editing data for other apps";

/// The most bytes of a string literal on one line; a longer string is written across lines.
const LITERAL: usize = 64;
/// The reader's caps on a variable library: the variables, bindings and data sets it keeps, the
/// characters of a name and of an object's XML id, and the bytes of the names (a binding's variable name and
/// object id) and values it keeps.
const MAX_VARIABLES: usize = 256;
const MAX_BINDINGS: usize = 100_000;
const MAX_DATASETS: usize = 1_000;
const MAX_ID: usize = 255;
const MAX_BYTES: usize = 8 << 20;
/// The most XML nodes the data sets' values are written with: a value's element, and a text value's
/// `p` element and text for each line.
const MAX_VALUE_NODES: usize = 200_000;

/// The bytes of names and values written so far, as the reader counts them, the XML nodes of the
/// values, and whether anything was left out past the caps.
#[derive(Default)]
struct Budget {
    bytes: usize,
    nodes: usize,
    past: bool,
}

impl Budget {
    /// Whether `len` more bytes fit in [`MAX_BYTES`] (they are then counted).
    fn keep(&mut self, len: usize) -> bool {
        self.keep_value(len, 0)
    }

    /// Whether a value of `len` bytes written with `nodes` XML nodes fits in [`MAX_BYTES`] and
    /// [`MAX_VALUE_NODES`] (it is then counted).
    fn keep_value(&mut self, len: usize, nodes: usize) -> bool {
        let fits = self.bytes.saturating_add(len) <= MAX_BYTES && self.nodes.saturating_add(nodes) <= MAX_VALUE_NODES;
        if fits {
            self.bytes += len;
            self.nodes += nodes;
        }
        fits
    }
}

/// The value of an XML node's entry, with the type it is written with.
#[derive(Clone, Copy)]
enum Value<'a> {
    /// A `/String`.
    Str(&'a str),
    /// A `/UnicodeString`.
    Unicode(&'a str),
    /// A `/Bool`.
    Bool(bool),
    /// An object's XML id, `/XMLUIDREF`.
    Uid(&'a str),
}

/// `s` as string literals on `%_` lines, at most [`LITERAL`] bytes each, the first line's `%_`
/// included and the last line left open for what follows it.
fn literals(s: &str) -> String {
    let mut parts = vec![];
    let mut start = 0;
    for (at, c) in s.char_indices() {
        if at + c.len_utf8() - start > LITERAL {
            parts.push(&s[start..at]);
            start = at;
        }
    }
    parts.push(&s[start..]);
    parts.iter().map(|p| format!("%_{}", ps::bytes(p.as_bytes()))).collect::<Vec<_>>().join("\n")
}

/// `v` written before an entry's key, from the start of its line.
fn value(v: Value) -> String {
    match v {
        Value::Str(s) => format!("{} /String", literals(s)),
        Value::Unicode(s) => format!("{} /UnicodeString", literals(s)),
        Value::Bool(b) => format!("%_{} /Bool", u8::from(b)),
        Value::Uid(id) => format!("%_/XMLUIDREF : {} ;", ps::bytes(id.as_bytes())),
    }
}

/// An XML node (`/XMLNode`) up to the `;` that closes it: its name, its value, its type `kind` (1 an
/// element, 2 an attribute, 3 text, 9 the document), its children (an array, each closed as an item)
/// and its attribute nodes (a dictionary, each under its name).
fn node(kind: u8, name: &str, v: Value, attributes: &[(&str, Value)], children: &[String]) -> String {
    let mut s = format!(
        "%_/XMLNode :\n{} (xmlnode-nodename) ,\n{} (xmlnode-nodevalue) ,\n%_{kind} /Int (xmlnode-nodetype) ,\n%_/Array :\n",
        value(Value::Str(name)),
        value(v)
    );
    for c in children {
        s.push_str(c);
        s.push_str("%_; ,\n");
    }
    s.push_str("%_; (xmlnode-children) ,\n%_/Dictionary :\n");
    for (k, v) in attributes {
        s.push_str(&node(2, k, *v, &[], &[]));
        let _ = writeln!(s, "%_; {} ,", ps::bytes(k.as_bytes()));
    }
    s.push_str("%_; (xmlnode-attributes) ,\n");
    s
}

/// An element `name` with `attributes` and `children`, up to the `;` that closes it.
fn element(name: &str, attributes: &[(&str, Value)], children: &[String]) -> String {
    node(1, name, Value::Str(""), attributes, children)
}

/// A text node (`#text`) holding `v`, up to the `;` that closes it.
fn text(v: Value) -> String {
    node(3, "#text", v, &[], &[])
}

/// A linked file's path as a data set holds it: a path from the root (`/…`) as a `file://` URL
/// (`file:///` and the path, with `%`, `#`, `?`, white space, control characters and bytes outside ASCII
/// escaped as `%XX`), another absolute path (a drive's, or a network path) as it is. `None` for a
/// path that isn't absolute, for the root alone (`/`), for a path from the root whose first part
/// starts like a drive (`/C:…`, which the reader reads as the drive's path), and for one written as
/// it is with white space at either end.
fn file_value(path: &str) -> Option<String> {
    // A drive's path from its root (`C:\…`, `C:/…`); `C:name` is relative to the drive's folder.
    let drive = matches!(path.as_bytes(), [l, b':', b'\\' | b'/', ..] if l.is_ascii_alphabetic());
    if path.starts_with('/') && !path.starts_with("//") {
        let rest = &path.as_bytes()[1..];
        if rest.is_empty() || matches!(rest, [l, b':', ..] if l.is_ascii_alphabetic()) {
            return None;
        }
        let mut url = String::from("file:///");
        for &b in path.as_bytes() {
            if matches!(b, b'%' | b'#' | b'?') || !(0x21..=0x7e).contains(&b) {
                let _ = write!(url, "%{b:02X}");
            } else {
                url.push(char::from(b));
            }
        }
        return Some(url);
    }
    // The reader takes white space off both ends of a path written as it is.
    let trimmed = path.trim_matches([' ', '\t', '\n', '\r']) == path;
    (trimmed && path.len() > 1 && (path.starts_with('/') || path.starts_with("\\\\") || drive)).then(|| path.to_string())
}

/// Whether the reader reads `name` as a variable's or a data set's name as it is: not empty, at most
/// [`MAX_ID`] characters, and without white space at either end.
fn readable_name(name: &str) -> bool {
    !name.is_empty() && name.trim() == name && name.chars().count() <= MAX_ID
}

impl Writer<'_> {
    /// The variables `n` is bound to that its XML id is written for: visibility variables, image
    /// variables when `n` was written as a placed file (`placed`, see [`Self::placed`]), and text
    /// variables when `n` was written as type (`typed`, see [`Self::typed`]). Only objects of the
    /// document's own tree are bound, not copies of them (symbol art, live objects' art, opacity
    /// masks' art, pattern tiles).
    /// The bindings left out are reported by [`Self::variable_library`].
    pub(super) fn bound_variables(&self, n: &Node, placed: bool, typed: bool) -> Vec<String> {
        let doc = self.doc;
        let Some(names) = doc.variables.bindings.get(&n.id) else { return vec![] };
        if self.nest > 0 || self.hidden > 0 || !doc.node(n.id).is_some_and(|d| std::ptr::eq(d, n)) {
            return vec![];
        }
        let fits = |name: &&String| match doc.variables.variable(name).map(|v| v.kind) {
            Some(VariableKind::Visibility) => true,
            Some(VariableKind::Image) => placed && matches!(n.kind, NodeKind::Image(_)),
            Some(VariableKind::Text) => typed && matches!(n.kind, NodeKind::Text(_)),
            _ => false,
        };
        names.iter().filter(fits).cloned().collect()
    }

    /// The `#document` entry of the `/Document` dictionary's `/Recorded` part that holds the
    /// variable library, on `%_` lines: nothing when the document has no variables or data sets.
    pub(in crate::ai_editing) fn variable_library(&mut self) -> String {
        let doc = self.doc;
        let vars = &doc.variables;
        if vars.variables.is_empty() && vars.datasets.is_empty() && !vars.locked {
            return String::new();
        }
        // Bindings not written: to layers, to type written as outlines, to graphs, and to objects
        // not written as themselves. Bindings to objects no longer in the document have nothing to
        // write.
        let pairs: HashSet<(NodeId, &str)> = self.bound.iter().flat_map(|(n, _, names)| names.iter().map(|name| (*n, name.as_str()))).collect();
        let written = |id: &NodeId, name: &String| pairs.contains(&(*id, name.as_str()));
        let kind = |name: &str| vars.variable(name).map(|v| v.kind);
        let missing = vars.bindings.iter().any(|(id, names)| {
            doc.node(*id).is_some() && names.iter().any(|name| kind(name).is_some_and(|k| k != VariableKind::Unknown) && !written(id, name))
        });
        if missing {
            self.warn(BINDINGS);
        }
        // What the reader keeps: names it doesn't read, and names, bindings and values past its
        // caps, are left out.
        let mut budget = Budget::default();
        let mut kept: Vec<&vectorcraft_doc::Variable> = vec![];
        for v in &vars.variables {
            let new = readable_name(&v.name) && !kept.iter().any(|k| k.name == v.name);
            if new && kept.len() < MAX_VARIABLES && budget.keep(v.name.len()) {
                kept.push(v);
            } else {
                budget.past = true;
            }
        }
        let mut declared = vec![];
        let mut bindings = 0usize;
        for v in &kept {
            let mut attributes = vec![];
            let kind = match v.kind {
                VariableKind::Text => Some(("textcontent", "&ns_flows;")),
                VariableKind::Visibility => Some(("visibility", "&ns_vars;")),
                VariableKind::Image => Some(("fileref", "&ns_vars;")),
                VariableKind::Graph => Some(("graphdata", "&ns_vars;")),
                VariableKind::Unknown => None,
            };
            if let Some((t, category)) = kind {
                attributes.push(("trait", Value::Str(t)));
                attributes.push(("category", Value::Str(category)));
            }
            attributes.push(("varName", Value::Unicode(&v.name)));
            // A variable bound to several objects is declared once for each.
            let objects = self.bound.iter().filter(|(_, _, names)| names.contains(&v.name)).map(|(_, id, _)| id.as_str());
            let mut written = 0;
            for id in objects {
                if bindings >= MAX_BINDINGS || id.chars().count() > MAX_ID || !budget.keep(v.name.len().saturating_add(id.len())) {
                    budget.past = true;
                    continue;
                }
                bindings += 1;
                written += 1;
                let mut with = attributes.clone();
                with.push(("docRef", Value::Uid(id)));
                declared.push(element("variable", &with, &[]));
            }
            if written == 0 {
                declared.push(element("variable", &attributes, &[]));
            }
        }
        let mut left_out = false;
        let mut rows = vec![];
        let mut row_names: Vec<&str> = vec![];
        for ds in &vars.datasets {
            let new = readable_name(&ds.name) && !row_names.contains(&ds.name.as_str());
            if !new || rows.len() >= MAX_DATASETS || !budget.keep(ds.name.len()) {
                budget.past = true;
                continue;
            }
            let mut values = vec![];
            for (name, v) in &ds.values {
                let Some(kind) = kept.iter().find(|k| &k.name == name).map(|k| k.kind) else {
                    // A value of a variable left out, or of none.
                    left_out |= vars.variable(name).is_none();
                    continue;
                };
                // The value's bytes as the reader counts them, and the nodes it is written with.
                let (len, nodes) = match (v, kind) {
                    _ if name.contains(':') => {
                        left_out = true;
                        continue;
                    }
                    (DataValue::Text(t), VariableKind::Text) => {
                        let lines = t.bytes().filter(|b| *b == b'\n').count() + 1;
                        (t.len(), 1 + 2 * lines)
                    }
                    (DataValue::Visible(_), VariableKind::Visibility) => (1, 2),
                    (DataValue::Image(path), VariableKind::Image) if file_value(path).is_some() => (path.len(), 2),
                    _ => {
                        left_out = true;
                        continue;
                    }
                };
                if !budget.keep_value(len, nodes) {
                    budget.past = true;
                    continue;
                }
                let children = match v {
                    DataValue::Text(t) => t
                        .split('\n')
                        .map(|line| {
                            let words = if line.is_empty() { vec![] } else { vec![text(Value::Unicode(line))] };
                            element("p", &[], &words)
                        })
                        .collect(),
                    DataValue::Visible(show) => vec![text(Value::Bool(*show))],
                    DataValue::Image(path) => file_value(path).map(|f| vec![text(Value::Unicode(&f))]).unwrap_or_default(),
                    DataValue::Graph(_) => vec![],
                };
                values.push(element(name, &[], &children));
            }
            rows.push(element("v:sampleDataSet", &[("dataSetName", Value::Unicode(&ds.name))], &values));
            row_names.push(&ds.name);
        }
        if left_out {
            self.warn(VALUES);
        }
        if budget.past {
            self.warn(LIMITS);
        }
        let mut attributes = vec![];
        // The data set shown, when a data set of that exact name is written.
        if let Some(active) = &vars.active_dataset {
            if row_names.contains(&active.as_str()) {
                attributes.push(("i:activeDataSet", Value::Unicode(active)));
            } else {
                self.warn(ACTIVE);
            }
        }
        attributes.push(("xmlns", Value::Str("&ns_custom;")));
        attributes.push(("xmlns:v", Value::Str("&ns_vars;")));
        let data_sets = element("v:sampleDataSets", &attributes, &rows);
        let locked = if vars.locked { "variableAndTypes" } else { "none" };
        let set = element(
            "variableSet",
            &[("locked", Value::Str(locked)), ("varSetName", Value::Str("binding1"))],
            &[element("variables", &[], &declared), data_sets],
        );
        let sets = element("variableSets", &[("xmlns", Value::Str("&ns_vars;"))], &[set]);
        let document = node(9, "#document", Value::Str(""), &[], &[element("metadata", &[], &[sets])]);
        format!("{document}%_; (#document) ,\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_from_the_root_are_file_urls() {
        assert_eq!(file_value("/Users/a b/100%.png").as_deref(), Some("file:////Users/a%20b/100%25.png"));
        assert_eq!(file_value("/tmp/é.png").as_deref(), Some("file:////tmp/%C3%A9.png"));
        assert_eq!(file_value("C:\\Art\\pic.png").as_deref(), Some("C:\\Art\\pic.png"));
        assert_eq!(file_value("\\\\server\\share\\pic.png").as_deref(), Some("\\\\server\\share\\pic.png"));
        assert_eq!(file_value("//server/share/pic.png").as_deref(), Some("//server/share/pic.png"));
        assert_eq!(file_value("/a/#1?.png").as_deref(), Some("file:////a/%231%3F.png"));
        assert_eq!(file_value("pic.png"), None);
        assert_eq!(file_value("C:pic.png"), None);
        assert_eq!(file_value("C:\\Art\\pic.png "), None);
        assert_eq!(file_value("/tmp/pic.png ").as_deref(), Some("file:////tmp/pic.png%20"));
    }

    #[test]
    fn paths_the_reader_reads_as_other_paths_are_left_out() {
        assert_eq!(file_value("/"), None);
        assert_eq!(file_value("/a:b/x.png"), None);
        assert_eq!(file_value("/C:/x.png"), None);
        assert_eq!(file_value("/1:b/x.png").as_deref(), Some("file:////1:b/x.png"));
        assert_eq!(file_value("/ab:c/x.png").as_deref(), Some("file:////ab:c/x.png"));
    }

    #[test]
    fn long_strings_are_written_across_lines() {
        // 140 bytes: two lines of 64 bytes and one of 12.
        let s = "é".repeat(70);
        let lines = literals(&s);
        assert_eq!(lines.lines().count(), 3);
        assert!(lines.lines().all(|l| l.starts_with("%_(")));
    }
}
