//! Asset Export (Window → Asset Export, Object → Collect for Export): pieces of art collected to
//! export on their own, at the export settings Export for Screens shares.
//!
//! An asset names objects rather than copying them: it follows edits (a moved object moves the
//! crop, a recoloured one exports recoloured) and lets go of objects that are deleted
//! ([`Document::prune_assets`]; an asset left with none goes too). Asset ids come from the
//! document's id counter, so they never repeat within a document.

use serde::{Deserialize, Serialize};

use crate::{Document, NodeId};

/// One asset: what it is called (its files' names) and the objects it exports, cropped to their
/// visual bounds, or for a 3D asset its object's solids as a mesh file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportAsset {
    pub id: u64,
    pub name: String,
    pub nodes: Vec<NodeId>,
    /// A 3D asset's mesh format (`usda`, `usdz`, `gltf` or `obj`): its object exports as a 3D
    /// file. `None` for art exported as pictures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_3d: Option<String>,
}

/// What the assets are named when they aren't named after their object: `Asset 1`, `Asset 2`…
const ASSET: &str = "Asset ";

impl Document {
    /// Asset `id`.
    pub fn asset(&self, id: u64) -> Option<&ExportAsset> {
        self.assets.iter().find(|a| a.id == id)
    }

    /// The name of the next unnamed asset: one more than the highest `Asset N` so far.
    pub fn next_asset_name(&self) -> String {
        let n = self
            .assets
            .iter()
            .filter_map(|a| {
                let digits = a.name.strip_prefix(ASSET)?;
                // `parse` alone would accept a leading `+`.
                digits.bytes().all(|b| b.is_ascii_digit()).then(|| digits.parse::<u64>().ok()).flatten()
            })
            .max()
            .unwrap_or(0);
        format!("{ASSET}{}", n.saturating_add(1))
    }

    /// Forget the objects of assets that are no longer in the document, and the assets left with
    /// none (after every edit).
    pub fn prune_assets(&mut self) {
        if self.assets.iter().all(|a| a.nodes.iter().all(|id| self.node(*id).is_some())) {
            return;
        }
        let mut assets = std::mem::take(&mut self.assets);
        for a in &mut assets {
            a.nodes.retain(|id| self.node(*id).is_some());
        }
        assets.retain(|a| !a.nodes.is_empty());
        self.assets = assets;
    }

    /// `ids` that are in the document, back to front.
    pub fn paint_order(&self, ids: impl IntoIterator<Item = NodeId>) -> Vec<NodeId> {
        let ids: Vec<NodeId> = ids.into_iter().collect();
        let paths = self.index_paths(ids.iter().copied());
        let mut v: Vec<(Vec<usize>, NodeId)> = ids.into_iter().filter_map(|id| paths.get(&id).map(|p| (p.clone(), id))).collect();
        v.sort();
        v.into_iter().map(|(_, id)| id).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name_after(names: &[&str]) -> String {
        let mut doc = Document::new(100.0, 100.0);
        for (i, n) in names.iter().enumerate() {
            doc.assets.push(ExportAsset { id: i as u64 + 1, name: (*n).to_string(), nodes: vec![], format_3d: None });
        }
        doc.next_asset_name()
    }

    #[test]
    fn next_asset_name_ignores_non_digit_suffixes() {
        assert_eq!(name_after(&["Asset +5"]), "Asset 1");
        assert_eq!(name_after(&["Asset ", "Asset 5x"]), "Asset 1");
        assert_eq!(name_after(&["Asset 5"]), "Asset 6");
    }
}
