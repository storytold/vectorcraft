//! Include Variable Data: the document's text variables, its data sets and the object each text
//! variable is bound to, written as a `variableSets` element in the layout Load Variable Library
//! reads (`variableSets` › `variableSet` › `variables` of `variable` elements, and
//! `sampleDataSets` of `sampleDataSet` elements). A variable's bound object is named by its
//! element's id in a `docRef` attribute, `docRef="id('…')"`.

use std::collections::HashSet;

use vectorcraft_doc::{DataValue, Document, NodeId, NodeKind, VariableKind};

use super::Writer;
use super::output::{valid_xml_value, xml_attr_escape, xml_ncname, xml_text};

/// The namespace of a variable library's elements.
const NS_VARIABLES: &str = "http://ns.adobe.com/Variables/1.0/"; // brand-ok: the namespace variable libraries use

/// The default namespace inside `sampleDataSets`, which the elements holding values are in.
const NS_CUSTOM: &str = "http://ns.adobe.com/GenericCustomNamespace/1.0/"; // brand-ok: the namespace variable libraries use

/// The `category` of a text variable.
const NS_FLOWS: &str = "http://ns.adobe.com/Flows/1.0/"; // brand-ok: the namespace variable libraries use

/// The most variable data a file holds: the largest variable library Load Variable Library reads.
const MAX_VARIABLE_DATA: usize = 8 << 20;

/// The most variables a variable library Load Variable Library reads declares.
const MAX_VARIABLES: usize = 256;

/// The most data sets a variable library Load Variable Library reads holds.
const MAX_DATASETS: usize = 1_000;

/// The most XML nodes (elements and text) a variable library Load Variable Library reads holds.
const MAX_NODES: usize = 2_000_000;

/// The text variables of `doc` that a variable library can hold: those whose names can name an
/// element, the first [`MAX_VARIABLES`] of them.
fn written_variables(doc: &Document) -> Vec<&str> {
    nameable_variables(doc).take(MAX_VARIABLES).collect()
}

/// The text variables of `doc` whose names can name an element.
fn nameable_variables(doc: &Document) -> impl Iterator<Item = &str> {
    doc.variables.variables.iter().filter(|v| v.kind == VariableKind::Text && xml_ncname(&v.name)).map(|v| v.name.as_str())
}

/// The object each text variable of `doc` that a library can hold names: the first of its bound
/// objects (by object id) that is a type object and that no variable before it names.
fn bound_objects(doc: &Document) -> Vec<(&str, NodeId)> {
    let mut taken = HashSet::new();
    written_variables(doc)
        .into_iter()
        .filter_map(|name| {
            let id = doc
                .variables
                .objects_of(name)
                .into_iter()
                .find(|id| !taken.contains(id) && doc.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Text(_))))?;
            taken.insert(id);
            Some((name, id))
        })
        .collect()
}

impl Writer<'_> {
    /// With Include Variable Data, give each object a text variable names an id when its name gave
    /// it none (an unnamed object, or Minimal object ids). Called after the named objects' ids are
    /// taken.
    pub(super) fn assign_bound_ids(&mut self) {
        if !self.opts.variable_data {
            return;
        }
        let doc = self.doc;
        for (name, id) in bound_objects(doc) {
            if !self.names.contains_key(&id) {
                let cand = self.unique_id(&super::sanitize_id(name));
                self.names.insert(id, cand);
            }
            self.bound.insert(id, Bound::Pending);
        }
    }

    /// Note that the object `id` was written with the element id `element`, when a variable names
    /// it (the first element written for it).
    pub(super) fn note_bound(&mut self, id: NodeId, element: &str) {
        if let Some(slot) = self.bound.get_mut(&id)
            && *slot == Bound::Pending
        {
            *slot = Bound::Written(element.to_string());
        }
    }

    /// Note that the type object `id` was written as outlines, which open as art rather than type.
    pub(super) fn note_outlined(&mut self, id: NodeId) {
        if let Some(slot) = self.bound.get_mut(&id) {
            *slot = Bound::Outlined;
        }
    }

    /// The `variableSets` element, indented one level, or nothing when the option is off or the
    /// document has no text variable a library can hold. Variables of other kinds, names that
    /// can't name an element, and data sets whose names are blank, have white space at either end,
    /// hold characters XML can't hold or repeat a name are left out, each kind with a warning. A
    /// variable whose object this file doesn't hold, or holds as outlines, is written without a
    /// `docRef` (outlines with a warning). Characters XML can't hold are left out of the values,
    /// with a warning. Variable data larger than Load Variable Library reads
    /// ([`MAX_VARIABLE_DATA`]) is left out with a warning.
    pub(super) fn variable_sets(&mut self) -> String {
        let doc = self.doc;
        let vars = &doc.variables;
        if !self.opts.variable_data || vars.variables.is_empty() {
            return String::new();
        }
        if vars.variables.iter().any(|v| v.kind != VariableKind::Text) {
            self.warn("Variables that aren't text variables were left out of the SVG's variable data");
        }
        if vars.variables.iter().any(|v| v.kind == VariableKind::Text && !xml_ncname(&v.name)) {
            self.warn("Text variables whose names can't name an XML element were left out of the SVG's variable data");
        }
        let names = written_variables(doc);
        if names.is_empty() {
            return String::new();
        }
        if nameable_variables(doc).nth(MAX_VARIABLES).is_some() {
            self.warn("Text variables past the first 256, which Load Variable Library reads, were left out of the SVG's variable data");
        }
        // XML names can't be written as character references.
        self.non_latin1_names |= names.iter().any(|n| n.chars().any(|c| u32::from(c) > 0xff));
        let bound = bound_objects(doc);
        let slots: Vec<Option<&Bound>> =
            names.iter().map(|name| bound.iter().find(|(n, _)| n == name).and_then(|(_, id)| self.bound.get(id))).collect();
        let outlined = slots.contains(&Some(&Bound::Outlined));
        let refs: Vec<Option<String>> = slots
            .into_iter()
            .map(|s| match s {
                Some(Bound::Written(e)) => Some(e.clone()),
                _ => None,
            })
            .collect();
        if outlined {
            self.warn("Type written as outlines opens as art: its text variables were written unbound in the SVG's variable data");
        }
        let mut seen = HashSet::new();
        let mut datasets: Vec<_> = vars
            .datasets
            .iter()
            .filter(|ds| !ds.name.is_empty() && ds.name.trim() == ds.name && valid_xml_value(&ds.name) && seen.insert(ds.name.as_str()))
            .collect();
        if datasets.len() < vars.datasets.len() {
            self.warn(
                "Data sets whose names are blank, start or end with white space, hold characters XML can't hold or repeat a name were left out of the SVG's variable data",
            );
        }
        if datasets.len() > MAX_DATASETS {
            datasets.truncate(MAX_DATASETS);
            self.warn("Data sets past the first 1,000, which Load Variable Library reads, were left out of the SVG's variable data");
        }
        let values = || datasets.iter().flat_map(|ds| names.iter().filter_map(|n| ds.values.get(*n)));
        if values().any(|v| matches!(v, DataValue::Text(t) if !valid_xml_value(t))) {
            self.warn("Characters XML can't hold were left out of data set values in the SVG's variable data");
        }
        let nl = if self.opts.minify { "" } else { "\n" };
        let mut out = String::new();
        // The XML nodes the reader makes: each start tag an element, the white space before each
        // line a text node unless minified, and each paragraph's text another.
        let mut nodes = 0usize;
        let mut line = |depth: usize, s: &str| {
            nodes += usize::from(!s.starts_with("</")) + usize::from(!self.opts.minify) + usize::from(s.starts_with("<p>") && s != "<p></p>");
            out.push_str(&self.indent(depth));
            out.push_str(s);
            out.push_str(nl);
        };
        line(1, &format!("<variableSets xmlns=\"{NS_VARIABLES}\">"));
        line(2, "<variableSet varSetName=\"binding1\" locked=\"none\">");
        line(3, "<variables>");
        for (name, element) in names.iter().zip(&refs) {
            let doc_ref = element.as_ref().map_or_else(String::new, |e| format!(" docRef=\"id('{}')\"", xml_attr_escape(e)));
            line(4, &format!("<variable varName=\"{name}\" trait=\"textcontent\" category=\"{NS_FLOWS}\"{doc_ref}></variable>"));
        }
        line(3, "</variables>");
        line(3, &format!("<v:sampleDataSets xmlns:v=\"{NS_VARIABLES}\" xmlns=\"{NS_CUSTOM}\">"));
        for ds in datasets {
            line(4, &format!("<v:sampleDataSet dataSetName=\"{}\">", xml_attr_escape(&ds.name)));
            for name in &names {
                let Some(DataValue::Text(t)) = ds.values.get(*name) else { continue };
                line(5, &format!("<{name}>"));
                // One paragraph per line; a carriage return as a reference, which reads back as itself.
                for para in t.split('\n') {
                    line(6, &format!("<p>{}</p>", xml_text(para).replace('\r', "&#13;")));
                }
                line(5, &format!("</{name}>"));
            }
            line(4, "</v:sampleDataSet>");
        }
        line(3, "</v:sampleDataSets>");
        line(2, "</variableSet>");
        line(1, "</variableSets>");
        // Measured as written: a Latin-1 file holds other characters as references.
        let mut written = std::borrow::Cow::Borrowed(out.as_str());
        if self.opts.encoding == crate::Encoding::Latin1 {
            super::latin1_refs(written.to_mut());
        }
        if written.len() > MAX_VARIABLE_DATA {
            self.warn("The variable data is larger than 8 MiB, which Load Variable Library reads, and was left out of the SVG");
            return String::new();
        }
        if nodes > MAX_NODES {
            self.warn("The variable data holds more than 2,000,000 XML nodes, which Load Variable Library reads, and was left out of the SVG");
            return String::new();
        }
        out
    }
}

/// What an object a text variable names became in the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Bound {
    /// Not written yet.
    Pending,
    /// Written as an element with this id.
    Written(String),
    /// Written as outlines, which open as art.
    Outlined,
}
