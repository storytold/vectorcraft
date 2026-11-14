//! The art of the stored brushes (a scatter or art brush's art, a pattern brush's tiles) as stored
//! art of the document ([`Document::mounted_art`]): set on the document while a command looks it up
//! by id or changes it, then written back into the library entries whose art changed. The Links
//! panel finds the images and placed documents in that art this way.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeKind};

use crate::{Brush, BrushKind, DOC_KEY};

/// The brushes of a document's stored library whose art holds images or placed documents.
#[derive(Default)]
pub struct StoredArt {
    entries: Vec<Entry>,
    /// How many entries the stored library had when it was read.
    read_entries: usize,
}

/// A library entry whose brush's art holds images or placed documents.
struct Entry {
    /// Its index in the stored library.
    slot: usize,
    /// The entry as it was read.
    raw: Value,
    brush: Brush,
    /// The brush's art ([`Brush::art`]), each root with the `owner` the Links panel names for it.
    art: Vec<(Arc<Node>, Value)>,
}

/// What names each root of [`Brush::art`] among the brush's art: a pattern brush's tile, in the
/// same order; nothing for the one root of a scatter or art brush.
fn tiles(b: &Brush) -> Vec<Option<&'static str>> {
    match &b.kind {
        BrushKind::Pattern(p) => std::iter::once(Some("side"))
            .chain(
                [
                    (p.outer_corner.is_some(), "outerCorner"),
                    (p.inner_corner.is_some(), "innerCorner"),
                    (p.start.is_some(), "start"),
                    (p.end.is_some(), "end"),
                ]
                .into_iter()
                .filter(|(held, _)| *held)
                .map(|(_, tile)| Some(tile)),
            )
            .collect(),
        _ => vec![None],
    }
}

/// Whether `n` or its subtree, opacity-mask art included, holds an image or a placed document.
pub(crate) fn holds_files(n: &Node) -> bool {
    let mut found = false;
    vectorcraft_doc::swatches::walk_with_masks(n, &mut |c| found |= matches!(c.kind, NodeKind::Image(_) | NodeKind::PlacedDocument(_)));
    found
}

impl StoredArt {
    /// The brushes of `doc`'s stored library whose art holds an image or a placed document (opacity
    /// masks included). An entry that does not read as a brush is left out, and so is left as it
    /// is. A root's owner is
    /// `{kind: "brush", name, index, tile?}`, with the name and index the brush has in the library as
    /// the Brushes panel lists it ([`crate::library`], a repeated name numbered).
    pub fn read(doc: &Document) -> Self {
        let Some(stored) = doc.unknown.get(DOC_KEY) else { return Self::default() };
        let Some(list) = stored.as_array() else { return Self::default() };
        let listed = crate::parse_library(stored);
        let parsed = list.iter().enumerate().filter_map(|(slot, raw)| Brush::from_json(raw.clone()).ok().map(|b| (slot, raw, b)));
        let mut entries = vec![];
        for (index, (slot, raw, brush)) in parsed.enumerate() {
            if !brush.art().into_iter().any(holds_files) {
                continue;
            }
            let name = listed.get(index).map_or(brush.name.as_str(), |b| b.name.as_str());
            let art: Vec<(Arc<Node>, Value)> = brush
                .art()
                .into_iter()
                .zip(tiles(&brush))
                .map(|(n, tile)| {
                    let mut owner = json!({ "kind": "brush", "name": name, "index": index });
                    if let Some(tile) = tile {
                        owner["tile"] = json!(tile);
                    }
                    (Arc::new(n.clone()), owner)
                })
                .collect();
            if !art.is_empty() {
                entries.push(Entry { slot, raw: raw.clone(), brush, art });
            }
        }
        Self { entries, read_entries: list.len() }
    }

    /// Gives the objects of the stored brushes' art that holds images or placed documents ids of
    /// their own in `doc`'s library, as [`Self::mount`] gives them, and writes the brushes whose
    /// ids changed back. Does nothing while art is set on `doc` ([`Document::mounted_art`]).
    pub fn persist_ids(doc: &mut Document) {
        if !doc.mounted_art.is_empty() {
            return;
        }
        let art = Self::read(doc);
        if !art.is_empty() {
            art.mount(doc);
            art.write_entries(doc);
        }
    }

    /// No stored brush's art holds an image or a placed document.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Sets the art on `doc` as [`Document::mounted_art`], each object with an id of its own
    /// ([`Document::number_mounted_art`]: an id another stored object has, or none, is replaced, and
    /// the id counter stands above them all).
    pub fn mount(&self, doc: &mut Document) {
        doc.mounted_art = self.entries.iter().flat_map(|e| e.art.iter().cloned()).collect();
        doc.number_mounted_art();
    }

    /// Takes [`Document::mounted_art`] off `doc` and writes each brush whose art changed (new ids
    /// included) back into its library entry: the entry at its index when that entry is as it was
    /// read or reads as the brush as it was apart from its name, else the first entry that reads as
    /// the brush as it was, else the first that does apart from its name. An entry found apart from
    /// its name keeps that name (the library was written again meanwhile, as placing a document's
    /// brushes into it does, numbering a repeated name). The other entries are left as
    /// they are. Then the art of brushes added meanwhile gets ids of its own
    /// ([`Self::persist_ids`]).
    pub fn write_back(self, doc: &mut Document) {
        let read_entries = self.read_entries;
        self.write_entries(doc);
        if doc.unknown.get(DOC_KEY).and_then(Value::as_array).map(Vec::len) != Some(read_entries) {
            Self::persist_ids(doc);
        }
    }

    /// [`Self::write_back`] without giving ids to the art of brushes added meanwhile.
    fn write_entries(self, doc: &mut Document) {
        let mut mounted = std::mem::take(&mut doc.mounted_art).into_iter();
        let mut changed = vec![];
        for e in self.entries {
            let now: Vec<Arc<Node>> = mounted.by_ref().take(e.art.len()).map(|(n, _)| n).collect();
            if now.len() != e.art.len() || now.iter().zip(&e.art).all(|(a, (b, _))| Arc::ptr_eq(a, b)) {
                continue;
            }
            let mut brush = e.brush.clone();
            for (slot, art) in brush.art_mut().into_iter().zip(now) {
                *slot = Arc::unwrap_or_clone(art);
            }
            changed.push((e.slot, e.raw, e.brush, brush));
        }
        let Some(list) = doc.unknown.get_mut(DOC_KEY).and_then(Value::as_array_mut) else { return };
        for (slot, raw, was, mut brush) in changed {
            // The entry's name when it reads as `was` apart from its name.
            let name_of = |entry: &Value| {
                Brush::from_json(entry.clone()).ok().and_then(|mut b| {
                    let name = std::mem::replace(&mut b.name, was.name.clone());
                    (b == was).then_some(name)
                })
            };
            let at = match list.get(slot) {
                Some(entry) if *entry == raw || name_of(entry).is_some() => Some(slot),
                _ => list.iter().position(|e| name_of(e).as_ref() == Some(&was.name)).or_else(|| list.iter().position(|e| name_of(e).is_some())),
            };
            let Some(entry) = at.and_then(|i| list.get_mut(i)) else { continue };
            if let Some(name) = name_of(entry) {
                brush.name = name;
            }
            if let Ok(v) = serde_json::to_value(&brush) {
                *entry = v;
            }
        }
    }
}
