//! The PDF export warnings for raster effects whose images the 64 million pixel cap rendered below
//! the document's raster effects resolution, one per object of the document.

use vectorcraft_doc::{Document, Node, NodeId, NodeKind};

use super::Which;

pub(super) struct FlattenReport {
    pub which: Which,
    requested_ppi: f64,
    reductions: Vec<Reduction>,
    /// Ids from here on are the flattening's own (split appearance items, groups under shadows).
    first_new_id: u64,
}

struct Reduction {
    id: NodeId,
    name: Option<String>,
    achieved_ppi: f64,
}

impl FlattenReport {
    pub fn new(which: Which, doc: &Document) -> Self {
        Self { which, requested_ppi: super::effects_scale(doc) * 72.0, reductions: vec![], first_new_id: doc.peek_next_id() }
    }

    /// Note `image` (made for the raster effects of `original`) when the cap lowered its resolution.
    pub fn record(&mut self, original: &Node, image: &Node) {
        if let NodeKind::Image(image) = &image.kind {
            let [pixel_size, ..] = image.xf.as_coeffs();
            let achieved_ppi = 72.0 / pixel_size;
            if achieved_ppi + 1e-6 < self.requested_ppi {
                self.reductions.push(Reduction { id: original.id, name: original.name.clone(), achieved_ppi });
            }
        }
    }

    pub fn checkpoint(&self) -> usize {
        self.reductions.len()
    }

    /// Give the reductions recorded since `checkpoint` on pieces the flattening made to `original`,
    /// a document object (the innermost one, as the walk returns from it).
    pub fn attribute_since(&mut self, checkpoint: usize, original: &Node) {
        let first_new_id = self.first_new_id;
        if original.id.0 >= first_new_id {
            return;
        }
        for reduction in self.reductions.iter_mut().skip(checkpoint) {
            if reduction.id.0 >= first_new_id {
                reduction.id = original.id;
                reduction.name.clone_from(&original.name);
            }
        }
    }

    pub fn warnings(self) -> Vec<String> {
        let mut lowest: Vec<Reduction> = vec![];
        for reduction in self.reductions {
            if let Some(existing) = lowest.iter_mut().find(|r| r.id == reduction.id) {
                existing.achieved_ppi = existing.achieved_ppi.min(reduction.achieved_ppi);
            } else {
                lowest.push(reduction);
            }
        }
        lowest
            .into_iter()
            .map(|r| {
                let name = r.name.map(|name| format!(" ({name})")).unwrap_or_default();
                format!(
                    "raster effects on object {}{name}: requested {:.1} ppi, achieved {:.1} ppi due to the 64 million pixel cap",
                    r.id.0, self.requested_ppi, r.achieved_ppi
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_doc::ImageObject;
    use vectorcraft_geom::Affine;

    fn image(ppi: f64) -> Node {
        Node::new(
            NodeId(999),
            NodeKind::Image(Box::new(ImageObject {
                key: "effect".into(),
                width: 8000,
                height: 8000,
                xf: Affine::scale(72.0 / ppi),
                link: None,
                placement: Default::default(),
                pixels: None,
            })),
        )
    }

    #[test]
    fn lowest_resolution_is_reported_when_multiple_images_belong_to_one_object() {
        let mut doc = Document::new(100.0, 100.0);
        doc.raster_effects_ppi = 2400.0;
        let original = &doc.layers[0];
        let mut report = FlattenReport::new(Which::All, &doc);

        for ppi in [1800.0, 1200.0, 1500.0] {
            report.record(original, &image(ppi));
        }

        let warnings = report.warnings();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("achieved 1200.0 ppi"), "{:?}", warnings);
    }

    #[test]
    fn parent_identity_is_reported_when_generated_pieces_are_attributed() {
        let doc = Document::new(100.0, 100.0);
        let original = &doc.layers[0];
        let generated = Node::group(NodeId(998), vec![]);
        let mut report = FlattenReport::new(Which::All, &doc);

        report.record(&generated, &image(36.0));
        report.attribute_since(0, original);

        let warnings = report.warnings();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains(&format!("object {}", original.id.0)), "{:?}", warnings);
    }
}
