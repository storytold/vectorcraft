use super::layout::{master_instances, master_prefix, page_masters};
use super::master_copies::{CopyCost, MAX_MASTER_COPIES, MasterPart, copy_cost};
use super::nodes::rect_of;
use super::*;

impl Builder {
    pub(super) fn new(k: f64) -> Self {
        Self {
            doc: Document::new(0.0, 0.0),
            warnings: Vec::new(),
            k,
            layer: 0,
            embedded: 0,
            masters: Vec::new(),
            cmyk_profile: None,
            cmyk_lut: None,
            working_cmyk: None,
            overlay_work: vectorcraft_affinity::overlay::MAX_WORK,
            copy_left: MAX_MASTER_COPIES,
            master_parts: std::collections::HashMap::new(),
        }
    }

    pub(super) fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    pub(super) fn document(&mut self, d: &af::Document) -> Result<()> {
        self.cmyk_profile = d.cmyk_profile.clone();
        // Spreads (Publisher pages) sit side by side, 36 pt apart; a single spread keeps its origin.
        let mut layers = Vec::new();
        let mut artboards = Vec::new();
        let mut x = 0.0;
        for (i, spread) in d.spreads.iter().enumerate() {
            let shift = if i == 0 { 0.0 } else { x - spread.bounds.x0 * self.k };
            let to_doc = Affine::translate((shift, 0.0)) * Affine::scale(self.k);
            let before = artboards.len();
            let mut loose: Vec<Arc<Node>> = Vec::new();
            for n in &spread.nodes {
                let top = match &n.kind {
                    Kind::Layer => self.node(n, to_doc, &mut artboards, true, 0),
                    Kind::Artboard { .. } => {
                        self.node(n, to_doc, &mut artboards, true, 0).map(|art| self.layer_of(&n.name, art, n.visible, n.locked))
                    }
                    _ => {
                        if let Some(art) = self.node(n, to_doc, &mut artboards, false, 0) {
                            loose.push(art);
                        }
                        continue;
                    }
                };
                if !loose.is_empty() {
                    layers.push(self.layer_with(None, std::mem::take(&mut loose)));
                }
                layers.extend(top);
            }
            if !loose.is_empty() {
                layers.push(self.layer_with(None, loose));
            }
            self.master_pages(d, spread, to_doc);
            // Without artboards, each page (or the spread) becomes one.
            if artboards.len() == before {
                let rects = if spread.pages.is_empty() { vec![spread.bounds] } else { spread.pages.clone() };
                for r in rects {
                    let name = format!("{} {}", if d.spreads.len() > 1 || spread.pages.len() > 1 { "Page" } else { "Artboard" }, artboards.len() + 1);
                    artboards.push((name, to_doc.transform_rect_bbox(rect_of(r))));
                }
            }
            // (Earlier spreads' artboards are all left of `x` already.)
            x = artboards.get(before..).unwrap_or_default().iter().map(|(_, r)| r.x1).fold(x, f64::max) + 36.0;
            if spread.transparent && i == 0 {
                self.warn("transparent page backgrounds show as white artboards");
            }
        }
        self.doc.artboards = artboards
            .into_iter()
            .enumerate()
            .filter(|(_, (_, r))| r.width() > 0.0 && r.height() > 0.0)
            .map(|(i, (name, rect))| Artboard {
                id: i as u32 + 1,
                name,
                rect,
                locked: false,
                show_center_mark: false,
                show_cross_hairs: false,
                show_video_safe_areas: false,
                video_ruler_pixel_aspect_ratio: 1.0,
                color: None,
            })
            .collect();
        if self.doc.artboards.is_empty() {
            return Err(err("the Affinity document has no page with a size"));
        }
        // Master pages are behind everything on the pages that show them.
        let mut masters = std::mem::take(&mut self.masters);
        masters.sort_by_key(|(m, _)| *m);
        let mut bottom = Vec::new();
        for (m, art) in masters {
            let mut l = (*self.layer_with(Some(&format!("Master {}", master_prefix(m))), art)).clone();
            l.locked = true;
            bottom.push(Arc::new(l));
        }
        // Bottom to top, as Affinity lists them.
        bottom.extend(layers);
        self.doc.layers = bottom;
        if self.doc.layers.is_empty() {
            let empty = self.layer_with(None, Vec::new());
            self.doc.layers.push(empty);
        }
        Ok(())
    }

    /// The master pages `spread`'s pages show, placed on those pages (`to_doc` places the spread).
    /// Each master page shows only the objects on it, moved from its page to the one showing it.
    fn master_pages(&mut self, d: &af::Document, spread: &af::Spread, to_doc: Affine) {
        let instances = master_instances(&spread.nodes);
        if instances.is_empty() {
            return;
        }
        let owners = page_masters(&instances, spread.pages.len());
        for (pi, page) in spread.pages.iter().enumerate() {
            let Some(&Some((master, first, master_page))) = owners.get(pi) else { continue };
            let Some(m) = d.masters.get(master) else { continue };
            let index = master_page.saturating_add(pi.saturating_sub(first));
            let r = m.pages.get(index).or_else(|| m.pages.last()).copied().unwrap_or(m.bounds);
            let parts = self.master_parts(master, m);
            let on_page = |b: &Option<af::Rect>| b.is_some_and(|b| b.x1 > r.x0 && b.x0 < r.x1 && b.y1 > r.y0 && b.y0 < r.y1);
            // Looking through the master's objects takes one each, copying them what they hold.
            let looked = CopyCost { work: parts.len() as u64, pixels: 0 };
            let cost = parts.iter().filter(|p| on_page(&p.bounds)).fold(looked, |c, p| c.plus(p.cost));
            let Some(left) = self.copy_left.minus(cost) else {
                self.warn("master pages shown on more pages than one import copies them to (left out on the rest)");
                return;
            };
            self.copy_left = left;
            let place = to_doc * Affine::translate((page.x0 - r.x0, page.y0 - r.y0));
            let mut art = Vec::new();
            let mut boards = Vec::new();
            self.embedded += 1;
            let mut i = 0;
            while let Some(p) = parts.get(i).copied() {
                let Some(n) = m.nodes.get(p.node) else {
                    i += 1;
                    continue;
                };
                if p.child.is_none() {
                    if on_page(&p.bounds) {
                        art.extend(self.node(n, place, &mut boards, false, 0));
                    }
                    i += 1;
                    continue;
                }
                // A layer of the master spread: just its objects on this page.
                let mut kids: Vec<Arc<Node>> = Vec::new();
                while let Some(q) = parts.get(i).copied().filter(|q| q.node == p.node) {
                    if on_page(&q.bounds)
                        && let Some(c) = q.child.and_then(|c| n.children.get(c))
                    {
                        kids.extend(self.node(c, place, &mut boards, false, 1));
                    }
                    i += 1;
                }
                if !kids.is_empty() {
                    let mut g = self.base(n, NodeKind::Group { children: kids, clip: false });
                    g.isolate = n.blend != Blend::PassThrough;
                    art.push(Arc::new(g));
                }
            }
            self.embedded -= 1;
            if art.is_empty() {
                continue;
            }
            let mut g = Node::group(self.doc.alloc_id(), art);
            g.name = Some(format!("Master {} (page {})", master_prefix(master), pi + 1));
            match self.masters.iter_mut().find(|(i, _)| *i == master) {
                Some((_, v)) => v.push(Arc::new(g)),
                None => self.masters.push((master, vec![Arc::new(g)])),
            }
        }
    }

    /// The objects of master spread `m` (number `index`) as its pages show them: its layers'
    /// objects one by one and its other top-level objects, with their bounds and costs.
    fn master_parts(&mut self, index: usize, m: &af::Spread) -> Arc<[MasterPart]> {
        if let Some(parts) = self.master_parts.get(&index) {
            return parts.clone();
        }
        if !master_instances(&m.nodes).is_empty() {
            self.warn("master pages based on other master pages (only their own objects are used)");
        }
        let part = |node: usize, child: Option<usize>, n: &af::Node| MasterPart {
            node,
            child,
            bounds: af::nodes_bounds(std::slice::from_ref(n)),
            cost: copy_cost(n, 0),
        };
        let mut parts = Vec::new();
        for (i, n) in m.nodes.iter().enumerate() {
            if matches!(n.kind, Kind::Layer) {
                parts.extend(n.children.iter().enumerate().map(|(c, child)| part(i, Some(c), child)));
            } else {
                parts.push(part(i, None, n));
            }
        }
        let parts: Arc<[MasterPart]> = parts.into();
        self.master_parts.insert(index, parts.clone());
        parts
    }
}
