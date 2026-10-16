//! Variables (data merge): named values bound to objects, with datasets that apply
//! them all at once. One template document produces many variants (badges, price
//! lists, localized copies) by selecting a dataset instead of editing each object.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{Document, NodeId, NodeKind};

/// What a variable drives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VariableKind {
    /// A type object's characters.
    #[default]
    Text,
    /// Whether bound objects show.
    Visibility,
    /// A linked image's file path (the pixels swap; the object's bounds stay).
    Image,
    /// A graph object's data (CSV of categories and series, as Graph Data uses).
    Graph,
}

impl VariableKind {
    /// Parse the `kind` of `variable.define`: `text`, `visibility` (`visible` is accepted
    /// as the same thing, as the panel and the docs call it both), `image` or `graph`.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "text" => Some(Self::Text),
            "visibility" | "visible" => Some(Self::Visibility),
            "image" => Some(Self::Image),
            "graph" => Some(Self::Graph),
            _ => None,
        }
    }

    /// The kind's own name, as `variable.define` takes it and `variable.list` reports it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Visibility => "visibility",
            Self::Image => "image",
            Self::Graph => "graph",
        }
    }
}

/// One named variable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Variable {
    pub name: String,
    pub kind: VariableKind,
}

/// One value a dataset gives a variable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DataValue {
    Text(String),
    Visible(bool),
    /// Absolute path of the image file an image variable should show.
    Image(String),
    /// CSV of a graph's categories and series (same shape as Graph Data / `graph.setData`).
    Graph(String),
}

impl DataValue {
    /// Whether this value fits the variable's kind (a dataset may carry values for
    /// variables of either kind; each binding takes what fits it).
    pub fn fits(&self, kind: VariableKind) -> bool {
        matches!(
            (self, kind),
            (Self::Text(_), VariableKind::Text)
                | (Self::Visible(_), VariableKind::Visibility)
                | (Self::Image(_), VariableKind::Image)
                | (Self::Graph(_), VariableKind::Graph)
        )
    }
}

/// One named row of values, applied with `dataset.select`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DataSet {
    pub name: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, DataValue>,
}

impl DataSet {
    /// Whether the art, as [`Document::captured_values`] read it, still holds what this row
    /// says. A value the art can't report (its object is locked or gone) isn't drift: the row
    /// keeps it, and applying it again would skip it too.
    pub fn matches(&self, current: &BTreeMap<String, DataValue>) -> bool {
        self.values.iter().all(|(k, v)| current.get(k).is_none_or(|c| c == v))
    }
}

/// The variable state of a document: definitions, datasets, bindings
/// (object id → the variables bound to it) and which dataset is active.
///
/// An object may be driven by several variables — the reference app binds the selected object
/// to the selected variable, one click at a time, and never says a second binding replaces the
/// first — so a binding is a set. `dataset.select` applies the text values in the order the
/// variables are defined, which is what makes two text variables on one object deterministic:
/// the last one wins, and `applied` counts both. Of two visibility variables on one object, the
/// last one defined wins too.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Variables {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<Variable>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub datasets: Vec<DataSet>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bindings: BTreeMap<NodeId, BTreeSet<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_dataset: Option<String>,
}

impl Variables {
    /// The variable `name`, if the document defines it.
    pub fn variable(&self, name: &str) -> Option<&Variable> {
        self.variables.iter().find(|v| v.name == name)
    }

    /// The dataset `name`, if the document has it.
    pub fn dataset(&self, name: &str) -> Option<&DataSet> {
        self.datasets.iter().find(|d| d.name == name)
    }

    /// Bind `id` to the variable `name`. True when it was not bound to it already.
    pub fn bind(&mut self, id: NodeId, name: &str) -> bool {
        self.bindings.entry(id).or_default().insert(name.to_string())
    }

    /// Drop `id`'s binding to `name` (and the binding itself when that was its last variable).
    /// True when there was one.
    pub fn unbind(&mut self, id: NodeId, name: &str) -> bool {
        let Some(bound) = self.bindings.get_mut(&id) else { return false };
        let dropped = bound.remove(name);
        if bound.is_empty() {
            self.bindings.remove(&id);
        }
        dropped
    }

    /// The objects bound to the variable `name`, in document order.
    pub fn objects_of(&self, name: &str) -> Vec<NodeId> {
        let mut ids: Vec<NodeId> = self.bindings.iter().filter(|(_, v)| v.contains(name)).map(|(id, _)| *id).collect();
        ids.sort_unstable();
        ids
    }

    /// Drop the variables `names`, the dataset values that name them and the bindings to
    /// them. Datasets named here are left alone: their names are their own namespace (the
    /// active one is dropped by whoever deletes the dataset).
    pub fn prune(&mut self, names: &[String]) -> usize {
        // A set, so a long list of names costs a lookup per value rather than a scan.
        let names: BTreeSet<&str> = names.iter().map(String::as_str).collect();
        let before = self.variables.len();
        self.variables.retain(|v| !names.contains(v.name.as_str()));
        for dataset in &mut self.datasets {
            dataset.values.retain(|k, _| !names.contains(k.as_str()));
        }
        for bound in self.bindings.values_mut() {
            bound.retain(|v| !names.contains(v.as_str()));
        }
        self.bindings.retain(|_, v| !v.is_empty());
        before - self.variables.len()
    }
}

impl Document {
    /// Forget the bindings of objects that are no longer in the document (after every edit),
    /// like [`Document::prune_assets`] does for Asset Export's. A binding outliving its object
    /// would count as a skip in every dataset application and grow the saved file forever.
    pub fn prune_variable_bindings(&mut self) {
        if self.variables.bindings.keys().all(|id| self.node(*id).is_some()) {
            return;
        }
        let mut bindings = std::mem::take(&mut self.variables.bindings);
        bindings.retain(|id, _| self.node(*id).is_some());
        self.variables.bindings = bindings;
    }

    /// Whether a variable may read and rewrite the object `id`: it is in the document, neither
    /// it nor anything above it is locked, and nothing above it is hidden. Its own visibility
    /// doesn't count, as that is what a visibility variable drives: a row that hid it must be
    /// able to show it again, and a capture must record it hidden.
    pub fn variable_target(&self, id: NodeId) -> bool {
        self.ancestry(id).is_some_and(|a| a.iter().all(|i| self.node(*i).is_some_and(|n| !n.locked && (n.visible || *i == id))))
    }

    /// What the object `id` holds for the variable of `kind`: its characters, whether it shows,
    /// or a linked image's path. `None` when the object is gone, is not of that kind, has no
    /// link path yet (an embedded image), or is locked or on a hidden or locked layer (the
    /// document's value is whatever it was left at).
    pub fn variable_value(&self, id: NodeId, kind: VariableKind) -> Option<DataValue> {
        if !self.variable_target(id) {
            return None;
        }
        let node = self.node(id)?;
        match (kind, &node.kind) {
            (VariableKind::Text, NodeKind::Text(t)) => Some(DataValue::Text(t.plain_text())),
            (VariableKind::Visibility, _) => Some(DataValue::Visible(node.visible)),
            (VariableKind::Image, NodeKind::Image(im)) => im.link.as_ref().map(|l| DataValue::Image(l.path.clone())),
            (VariableKind::Graph, _) => node.extras().graph.as_ref().map(|g| DataValue::Graph(g.to_csv())),
            (VariableKind::Text | VariableKind::Image, _) => None,
        }
    }

    /// The values a dataset captured right now: what each bound object holds, in the order the
    /// variables are defined. `dataset.capture` writes these into a new row and
    /// `dataset.update` into the active one, for the variables that row holds (`all: true` adds
    /// the rest); the Variables panel compares the active row with them to tell whether the art
    /// still matches what it says.
    pub fn captured_values(&self) -> BTreeMap<String, DataValue> {
        let mut out = BTreeMap::new();
        for v in &self.variables.variables {
            // An object driven by several variables of the same kind reads from the first one
            // bound, so a capture is one value per variable and applies back the same way.
            let value = self.variables.objects_of(&v.name).into_iter().find_map(|id| self.variable_value(id, v.kind));
            if let Some(value) = value {
                out.insert(v.name.clone(), value);
            }
        }
        out
    }
}
