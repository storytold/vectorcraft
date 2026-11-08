//! What plug-ins keep with a document: settings that belong to the whole document rather than to
//! one object, such as the pens a plotter holds or a machine's profile.
//!
//! They are the document's foreign data ([`Document::unknown`], under [`PLUGIN_DATA`]), as the
//! perspective grid and the brushes are, so the native file has no section of its own for them
//! and a version from before them keeps them when it saves. The host never reads them: a filter
//! plug-in writes its own and reads only its own back, and an exporter, which is given the whole
//! document, can read them all (`vectorcraft-plugins`, which caps what one plug-in keeps).

use serde_json::{Map, Value};

use crate::Document;

/// `Document::unknown` key: `{"<plug-in id>": <what it keeps>}`.
pub const PLUGIN_DATA: &str = "pluginData";

impl Document {
    /// What plug-in `id` keeps with the document, if anything.
    pub fn plugin_data(&self, id: &str) -> Option<&Value> {
        self.unknown.get(PLUGIN_DATA).and_then(|m| m.get(id))
    }

    /// Keeps `data` for plug-in `id` in place of what it kept; `None` forgets it, and the key goes
    /// once no plug-in keeps anything. A value under the key that isn't a map of plug-in ids (a
    /// damaged file) is replaced when a plug-in keeps something.
    pub fn set_plugin_data(&mut self, id: &str, data: Option<Value>) {
        let Some(data) = data else {
            // Nothing to forget: don't copy the foreign data the undo history shares.
            if self.plugin_data(id).is_none() {
                return;
            }
            let Some(m) = self.unknown.get_mut(PLUGIN_DATA).and_then(Value::as_object_mut) else { return };
            m.remove(id);
            if m.is_empty() {
                self.unknown.remove(PLUGIN_DATA);
            }
            return;
        };
        let kept = self.unknown.entry(PLUGIN_DATA.to_string()).or_insert_with(|| Value::Object(Map::new()));
        if !kept.is_object() {
            *kept = Value::Object(Map::new());
        }
        if let Some(m) = kept.as_object_mut() {
            m.insert(id.to_string(), data);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// Each plug-in has its own entry, replaced when it keeps something else; forgetting the last
    /// one removes the key, and forgetting what isn't kept copies nothing.
    #[test]
    fn each_plug_in_keeps_its_own_data() {
        let mut d = Document::new(10.0, 10.0);
        assert_eq!(d.plugin_data("a"), None);
        d.set_plugin_data("a", Some(json!({"pens": 2})));
        d.set_plugin_data("b", Some(json!([1, 2])));
        d.set_plugin_data("a", Some(json!({"pens": 3})));
        assert_eq!((d.plugin_data("a"), d.plugin_data("b")), (Some(&json!({"pens": 3})), Some(&json!([1, 2]))));
        d.set_plugin_data("a", None);
        assert_eq!(d.unknown[PLUGIN_DATA], json!({"b": [1, 2]}));
        let before = d.clone();
        d.set_plugin_data("a", None);
        assert!(crate::Shared::ptr_eq(&before.unknown, &d.unknown), "still shared with the earlier version");
        d.set_plugin_data("b", None);
        assert!(!d.unknown.contains_key(PLUGIN_DATA));
    }

    /// A damaged file's value under the key holds nothing for any plug-in, stays while there is
    /// nothing to forget, and is replaced when a plug-in keeps something.
    #[test]
    fn a_damaged_value_holds_nothing() {
        let mut d = Document::new(10.0, 10.0);
        d.unknown.insert(PLUGIN_DATA.into(), json!(5));
        assert_eq!(d.plugin_data("a"), None);
        d.set_plugin_data("a", None);
        assert_eq!(d.unknown[PLUGIN_DATA], json!(5));
        d.set_plugin_data("a", Some(json!(true)));
        assert_eq!(d.unknown[PLUGIN_DATA], json!({"a": true}));
    }
}
