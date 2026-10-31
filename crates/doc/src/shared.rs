//! [`Shared`]: a value shared between copies of a node until one of them changes it.

use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A copy-on-write value: cloning shares it, and the first change through `DerefMut` makes the
/// changed copy its own (`Arc::make_mut`). A node moved or recoloured keeps sharing what it didn't
/// change with the version the undo history holds. Reads, writes, equality and the saved form are
/// exactly those of the value itself.
#[derive(Clone, Default)]
pub struct Shared<T>(Arc<T>);

impl<T> Shared<T> {
    pub fn new(v: T) -> Self {
        Shared(Arc::new(v))
    }
    /// Whether `a` and `b` share one value (an unchanged copy).
    pub fn ptr_eq(a: &Self, b: &Self) -> bool {
        Arc::ptr_eq(&a.0, &b.0)
    }
    /// A weak reference to the value: it keeps the allocation, so the value's address names it
    /// while the reference is kept, but not the value.
    pub fn downgrade(this: &Self) -> std::sync::Weak<T> {
        Arc::downgrade(&this.0)
    }
}

impl<T> std::ops::Deref for Shared<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: Clone> std::ops::DerefMut for Shared<T> {
    fn deref_mut(&mut self) -> &mut T {
        Arc::make_mut(&mut self.0)
    }
}

impl<T: Clone> Shared<T> {
    /// The value, taken out (copied only when another copy still shares it).
    pub fn into_inner(self) -> T {
        Arc::unwrap_or_clone(self.0)
    }
}

/// `for x in std::mem::take(&mut doc.swatches)` takes the value, as it would the value itself.
impl<T: Clone + IntoIterator> IntoIterator for Shared<T> {
    type Item = T::Item;
    type IntoIter = T::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        self.into_inner().into_iter()
    }
}

/// `for x in &doc.swatches` iterates the shared value, as it would the value itself.
impl<'a, T> IntoIterator for &'a Shared<T>
where
    &'a T: IntoIterator,
{
    type Item = <&'a T as IntoIterator>::Item;
    type IntoIter = <&'a T as IntoIterator>::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        (&*self.0).into_iter()
    }
}

/// `for x in &mut doc.swatches` makes the value this copy's own first, as any change does.
impl<'a, T: Clone> IntoIterator for &'a mut Shared<T>
where
    &'a mut T: IntoIterator,
{
    type Item = <&'a mut T as IntoIterator>::Item;
    type IntoIter = <&'a mut T as IntoIterator>::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        Arc::make_mut(&mut self.0).into_iter()
    }
}

impl<T> From<T> for Shared<T> {
    fn from(v: T) -> Self {
        Shared::new(v)
    }
}

impl<T: PartialEq> PartialEq for Shared<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || *self.0 == *other.0
    }
}

/// A shared value equals the plain value it holds (`node.appearance == style.appearance`).
impl<T: PartialEq> PartialEq<T> for Shared<T> {
    fn eq(&self, other: &T) -> bool {
        *self.0 == *other
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for Shared<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl<T: Serialize> Serialize for Shared<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(s)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Shared<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        T::deserialize(d).map(Shared::new)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::text::TextStyleDef;
    use crate::variables::{Variable, VariableKind};
    use crate::{Document, NodeId, SavedSelection};

    /// A document with every shared resource set, changed only through `push`/`insert` (as code
    /// written for the plain values does).
    fn doc_with_resources() -> Document {
        let mut d = Document::new(100.0, 100.0);
        let swatch = d.swatches[1].clone();
        d.swatches.push(swatch);
        d.swatch_groups.truncate(1);
        d.graphic_styles.truncate(2);
        d.char_styles.push(TextStyleDef { name: "Emphasis".into(), attrs: json!({"size": 14}).as_object().cloned().unwrap_or_default() });
        d.para_styles.push(TextStyleDef { name: "Body".into(), attrs: Default::default() });
        d.saved_selections.push(SavedSelection { name: "Saved".into(), objects: vec![NodeId(1)] });
        d.variables.variables.push(Variable { name: "Name".into(), kind: VariableKind::Text });
        d.variables.bindings.entry(NodeId(1)).or_default().insert("Name".into());
        d.unknown.insert("foreign".into(), json!({"keep": [1, 2]}));
        d
    }

    /// Shared document resources save as the values themselves: the saved form is unchanged.
    #[test]
    fn shared_resources_save_and_load_as_their_values() {
        let d = doc_with_resources();
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["swatches"], serde_json::to_value(&*d.swatches).unwrap());
        assert_eq!(v["swatch_groups"], serde_json::to_value(&*d.swatch_groups).unwrap());
        assert_eq!(v["graphic_styles"], serde_json::to_value(&*d.graphic_styles).unwrap());
        assert_eq!(v["char_styles"], json!([{"name": "Emphasis", "attrs": {"size": 14}}]));
        assert_eq!(v["para_styles"], json!([{"name": "Body", "attrs": {}}]));
        assert_eq!(v["saved_selections"], json!([{"name": "Saved", "objects": [1]}]));
        assert_eq!(v["variables"], json!({"variables": [{"name": "Name", "kind": "text"}], "bindings": {"1": ["Name"]}}));
        assert_eq!(v["unknown"], json!({"foreign": {"keep": [1, 2]}}));
        let text = serde_json::to_string(&d).unwrap();
        let back: Document = serde_json::from_str(&text).unwrap();
        assert_eq!(back, d);
        assert_eq!(serde_json::to_string(&back).unwrap(), text, "saving again gives the same bytes");
        // Empty ones are left out, as before.
        let v = serde_json::to_value(Document::new(10.0, 10.0)).unwrap();
        for k in ["char_styles", "para_styles", "saved_selections", "variables", "unknown"] {
            assert!(v.get(k).is_none(), "{k} is left out when empty");
        }
    }

    /// A copy of a document shares its resources until one copy changes one of them.
    #[test]
    fn copies_share_resources_until_changed() {
        let a = doc_with_resources();
        let mut b = a.clone();
        assert!(super::Shared::ptr_eq(&a.swatches, &b.swatches) && super::Shared::ptr_eq(&a.unknown, &b.unknown));
        b.swatches.pop();
        assert!(!super::Shared::ptr_eq(&a.swatches, &b.swatches));
        assert_eq!(a.swatches.len(), b.swatches.len() + 1, "the other copy keeps its own");
        assert!(super::Shared::ptr_eq(&a.graphic_styles, &b.graphic_styles), "the rest stays shared");
    }
}
