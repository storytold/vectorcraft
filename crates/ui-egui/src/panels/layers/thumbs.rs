use craft::layers::ui::thumbs::{LayerThumb, LayerThumbKind}; // PLAN: assumed path of craft-layers' thumbnail key
use craft::ui::primitives::textures::{LoadFailure, LoadedPicture, Revision, ThumbRender, ThumbRequest, ThumbSource, ThumbnailOptions, Thumbnails};
use vectorcraft_doc::{NodeKind, TextKind};
use vectorcraft_geom::PathData;

use super::model::RowId; // PLAN: assumed document-panels-w7's `RowId::{Node(NodeId), Guide(usize), Spine(NodeId)}`
use super::{Arc, Document, Node, NodeId};

/// Node `id` of `doc`, as the `Arc` its parent holds.
pub(crate) fn node_arc(doc: &Document, id: NodeId) -> Option<Arc<Node>> {
    let path = doc.index_path(id)?;
    let (first, rest) = path.split_first()?;
    let mut cur = doc.layers.get(*first)?;
    for &i in rest {
        cur = cur.children()?.get(i)?;
    }
    Some(cur.clone())
}

/// A node of the Links panel's rows: the document and the node, nested ones included.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct NodeThumb {
    /// The document's uid.
    pub uid: u64,
    /// The node the row shows.
    pub node: NodeId,
}

/// The thumbnails of one document's Layers panel, one store for each document: node ids repeat across
/// documents, and a key shared by two would show one document's picture for the other's row.
pub(crate) fn layer_thumbnails(ctx: &egui::Context, uid: u64) -> Thumbnails<LayerThumb<RowId>> {
    Thumbnails::in_context(ctx, ("vc-layer-thumbs", uid), ThumbnailOptions::new())
}

/// The thumbnails of the Links panel's rows, for every document.
pub(crate) fn link_thumbnails(ctx: &egui::Context) -> Thumbnails<NodeThumb> {
    Thumbnails::in_context(ctx, "vc-link-thumbs", ThumbnailOptions::new())
}

/// Forget every thumbnail of document `uid`'s Layers panel, when it closes.
pub(crate) fn forget(ctx: &egui::Context, uid: u64) {
    layer_thumbnails(ctx, uid).retain(ctx, |_| false);
}

/// Keep the Links panel's thumbnails of the documents in `uids`, the open ones.
pub(crate) fn retain_open(ctx: &egui::Context, uids: &[u64]) {
    link_thumbnails(ctx).retain(ctx, |key| uids.contains(&key.uid));
}

/// The version of `node`'s picture: the node's identity, since an unchanged node keeps its `Arc`
/// allocation and an edited one gets another.
fn node_revision(node: &Arc<Node>, request: &ThumbRequest) -> Revision {
    Revision::of(&(Arc::as_ptr(node) as usize, request.side))
}

/// Where the Layers panel's thumbnails come from: the rows of document `doc`.
pub(crate) struct LayerThumbs<'a> {
    /// The document the rows are of.
    pub doc: &'a Arc<Document>,
}

// PLAN: the sheet's `tree` field is left out: `RowId` already says which rows have no picture (guides and spines).
impl LayerThumbs<'_> {
    /// The node of the row `key` names, when it has a picture of its own to show.
    fn node(&self, key: &LayerThumb<RowId>) -> Option<Arc<Node>> {
        if key.kind != LayerThumbKind::Pixels {
            return None;
        }
        match &key.layer {
            RowId::Node(id) => node_arc(self.doc, *id),
            _ => None,
        }
    }
}

impl ThumbSource<LayerThumb<RowId>> for LayerThumbs<'_> {
    fn revision(&self, key: &LayerThumb<RowId>, request: &ThumbRequest) -> Revision {
        self.node(key).map_or_else(Revision::default, |n| node_revision(&n, request))
    }

    fn render(&self, key: &LayerThumb<RowId>, request: &ThumbRequest) -> ThumbRender {
        match self.node(key) {
            Some(n) => {
                let light = light_row(&n, None, self.doc);
                render(n, self.doc.clone(), request, light)
            }
            None => ThumbRender::Blank,
        }
    }
}

/// Where the Links panel's thumbnails come from: the nodes of document `doc`, found by `lookup`
/// (nested nodes are not in the document's own tree).
pub(crate) struct NodeThumbs<'a, F: Fn(NodeId) -> Option<Arc<Node>>> {
    doc: &'a Arc<Document>,
    lookup: F,
}

impl<'a, F: Fn(NodeId) -> Option<Arc<Node>>> NodeThumbs<'a, F> {
    /// The source of the rows of `doc`.
    pub(crate) fn new(doc: &'a Arc<Document>, lookup: F) -> Self {
        Self { doc, lookup }
    }
}

impl<F: Fn(NodeId) -> Option<Arc<Node>>> ThumbSource<NodeThumb> for NodeThumbs<'_, F> {
    fn revision(&self, key: &NodeThumb, request: &ThumbRequest) -> Revision {
        (self.lookup)(key.node).map_or_else(Revision::default, |n| node_revision(&n, request))
    }

    fn render(&self, key: &NodeThumb, request: &ThumbRequest) -> ThumbRender {
        match (self.lookup)(key.node) {
            Some(n) => {
                let light = light_row(&n, None, self.doc);
                render(n, self.doc.clone(), request, light)
            }
            None => ThumbRender::Blank,
        }
    }
}

/// The thumbnail of `node` (of `doc`) for `request`: made here and now when `light`, else as a job
/// for a lane that stops when cancelled. A node that draws nothing is `Blank`.
pub(crate) fn render(node: Arc<Node>, doc: Arc<Document>, request: &ThumbRequest, light: bool) -> ThumbRender {
    let side = request.side;
    if light {
        thread_local! {
            static RENDERER: std::cell::RefCell<vectorcraft_render::Renderer> = std::cell::RefCell::new(vectorcraft_render::Renderer::new());
        }
        return match RENDERER.with(|r| render_thumb(&mut r.borrow_mut(), &doc, &node, side)) {
            Some(img) => ThumbRender::Ready(crate::canvas::color_image(&img)),
            None => ThumbRender::Blank,
        };
    }
    ThumbRender::Later(Box::new(move |cancel| {
        let cancelled = || LoadFailure::Message("cancelled".to_owned());
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        let img = render_thumb(&mut vectorcraft_render::Renderer::new(), &doc, &node, side);
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        Ok(img.map_or(LoadedPicture::Blank, |img| LoadedPicture::Image(crate::canvas::color_image(&img))))
    }))
}

/// A node with more objects than this inside has its first thumbnail made on a lane.
const LIGHT_THUMB_NODES: usize = 200;

/// A node whose paths have more anchors than this in all has its first thumbnail made on a lane.
/// Type's glyphs, mesh points and image pixels count against it too, each as the anchors
/// that take about as long to render: [`GLYPH_ANCHORS`], [`MESH_POINT_ANCHORS`] and
/// [`IMAGE_PIXELS_PER_ANCHOR`].
const LIGHT_THUMB_ANCHORS: usize = 2_000;

/// Anchors one glyph of type counts as.
const GLYPH_ANCHORS: usize = 4;

/// Anchors one point of a gradient mesh counts as.
const MESH_POINT_ANCHORS: usize = 1;

/// Pixels of an image that count as one anchor.
const IMAGE_PIXELS_PER_ANCHOR: usize = 100;

/// The most symbols and placed documents counted inside one another: a node holding them deeper
/// (a symbol whose art holds an instance of itself, for one) has its first thumbnail made on a
/// lane.
const MAX_THUMB_NESTING: usize = 8;

/// The most steps counting a node takes: each object, subpath, text run and symbol looked at is
/// one step, and so is every 64 bytes of a symbol or placed file name looked up. A node that
/// needs more has its thumbnail made on a lane.
const THUMB_COUNT_STEPS: usize = 20_000;

/// How row thumbnails render: images far smaller than they are sample their mip levels.
pub(crate) fn thumb_options() -> vectorcraft_render::RenderOptions {
    vectorcraft_render::RenderOptions { image_mips: true, ..Default::default() }
}

/// Has `n` at most `nodes` objects inside it, and at most `anchors` anchors in its paths and the
/// paths inside it, the frames of area type and the paths of type on a path included (counting
/// stops at either limit)? With `masks`, the art of the opacity masks that are not disabled, and
/// the objects and anchors inside it, count as well. With `doc` (the document `n` is in), type's
/// glyphs, mesh points and image pixels count as anchors too ([`LIGHT_THUMB_ANCHORS`]), and a
/// symbol instance and a placed document count the objects and anchors of the art they show: the
/// symbol's art in `doc`, and the placed file's art when it has been read (a placed document whose
/// file has not been read, or art nested deeper than [`MAX_THUMB_NESTING`], is not small). A node
/// that takes more than [`THUMB_COUNT_STEPS`] steps to count is not small.
fn small_node(n: &Node, nodes: usize, anchors: usize, masks: bool, doc: Option<&Document>) -> bool {
    weigh(n, nodes, anchors, masks, doc).0
}

/// [`small_node`], and the steps counting took.
fn weigh(n: &Node, nodes: usize, anchors: usize, masks: bool, doc: Option<&Document>) -> (bool, usize) {
    /// What may still be counted.
    struct Left {
        nodes: usize,
        anchors: usize,
        steps: usize,
        masks: bool,
    }
    impl Left {
        /// Count `anchors` more: false past the limit.
        fn take(&mut self, anchors: usize) -> bool {
            match self.anchors.checked_sub(anchors) {
                Some(left) => {
                    self.anchors = left;
                    true
                }
                None => false,
            }
        }
        /// Take `steps` more: false past the limit.
        fn step(&mut self, steps: usize) -> bool {
            match self.steps.checked_sub(steps) {
                Some(left) => {
                    self.steps = left;
                    true
                }
                None => false,
            }
        }
        /// Look up `name`: false past the step limit.
        fn look_up(&mut self, name: &str) -> bool {
            self.step(1 + name.len() / 64)
        }
        /// Count one more object: false past the limit.
        fn object(&mut self) -> bool {
            match self.nodes.checked_sub(1) {
                Some(left) => {
                    self.nodes = left;
                    true
                }
                None => false,
            }
        }
        /// Count the anchors of `p`, a step for each subpath.
        fn path(&mut self, p: &PathData) -> bool {
            p.subpaths.iter().all(|s| self.step(1) && self.take(s.anchors.len()))
        }
    }
    /// Count `art`, drawn in place of an object (`doc` its document), one level deeper.
    fn inside(art: &Node, doc: &Document, left: &mut Left, depth: usize) -> bool {
        depth < MAX_THUMB_NESTING && left.object() && count(art, Some(doc), left, depth + 1)
    }
    /// Count symbol `name` of `doc` (one that doesn't exist draws nothing).
    fn symbol(name: &str, doc: &Document, left: &mut Left, depth: usize) -> bool {
        for s in &doc.symbols {
            if !left.look_up(name) {
                return false;
            }
            if s.name == name {
                return inside(&s.art, doc, left, depth);
            }
        }
        true
    }
    /// Count what `n` draws besides its children.
    fn own(n: &Node, doc: Option<&Document>, left: &mut Left, depth: usize) -> bool {
        let path = match &n.kind {
            NodeKind::Text(t) => match &t.kind {
                TextKind::Area { frame: p } | TextKind::OnPath { path: p, .. } => Some(p),
                TextKind::Point => None,
            },
            _ => n.path_data(),
        };
        if !path.is_none_or(|p| left.path(p)) {
            return false;
        }
        let Some(doc) = doc else { return true };
        match &n.kind {
            // Glyphs are counted only up to one past what the anchors left allow.
            NodeKind::Text(t) => t.runs.iter().all(|r| {
                let glyphs = r.text.chars().take(left.anchors / GLYPH_ANCHORS + 1).count();
                left.step(1)
                    && left.take(glyphs.saturating_mul(GLYPH_ANCHORS))
                    && r.inline.as_ref().is_none_or(|a| symbol(&a.symbol, doc, left, depth))
            }),
            NodeKind::SymbolInstance { symbol: name, .. } => symbol(name, doc, left, depth),
            NodeKind::Mesh(m) => left.take(m.points.len().saturating_mul(MESH_POINT_ANCHORS)),
            NodeKind::Image(im) => left.take((im.width as usize).saturating_mul(im.height as usize) / IMAGE_PIXELS_PER_ANCHOR),
            NodeKind::PlacedDocument(p) => {
                left.look_up(&p.key) && vectorcraft_doc::placed_document::read_cached(p).is_some_and(|e| inside(&e.art, &e.doc, left, depth))
            }
            _ => true,
        }
    }
    fn count(n: &Node, doc: Option<&Document>, left: &mut Left, depth: usize) -> bool {
        if !(left.step(1) && own(n, doc, left, depth)) {
            return false;
        }
        let mask = n.extras().mask.as_deref().filter(|m| left.masks && !m.disabled).map(|m| &m.art);
        // A layer's ruler guides draw nothing in its thumbnail and use no budget.
        n.children().into_iter().flatten().filter(|c| !c.is_ruler_guide()).chain(mask).all(|c| left.object() && count(c, doc, left, depth))
    }
    let mut left = Left { nodes, anchors, steps: THUMB_COUNT_STEPS, masks };
    let small = count(n, doc, &mut left, 0);
    (small, THUMB_COUNT_STEPS - left.steps)
}

/// Is row `n` (an object of `doc`) light enough to make its thumbnail on the UI thread? `quick`:
/// whether its last thumbnail was quick to render, if it has one.
fn light_row(n: &Node, quick: Option<bool>, doc: &Document) -> bool {
    match quick {
        Some(quick) => quick && small_node(n, THUMB_COUNT_STEPS, LIGHT_THUMB_ANCHORS, true, Some(doc)),
        None => small_node(n, LIGHT_THUMB_NODES, LIGHT_THUMB_ANCHORS, true, Some(doc)),
    }
}

/// The thumbnail of `n` (`px` pixels square) as `r` renders it, on the UI thread or a lane.
fn render_thumb(r: &mut vectorcraft_render::Renderer, doc: &Document, n: &Node, px: u32) -> Option<vectorcraft_render::Rendered> {
    r.render_node_thumbnail_with(doc, n, px, None, &thumb_options())
}

#[cfg(test)]
mod tests {
    use craft::host::queue::Cancel; // PLAN: assumed path of craft-host's `Cancel`
    use vectorcraft_color::{Color, Paint};
    use vectorcraft_doc::{Appearance, CharStyle, LayerColor, NodeKind, OpacityMask, TextKind, TextObject};
    use vectorcraft_geom::{Affine, Point, SubPath};

    use super::*;

    /// An open zigzag of `anchors` anchors.
    fn line(anchors: usize) -> PathData {
        let pts: Vec<Point> = (0..anchors).map(|i| Point::new(i as f64 * 0.01, (i % 2) as f64 * 50.0)).collect();
        PathData::single(SubPath::polyline(&pts, false))
    }

    /// An open stroked path `id` of `anchors` anchors.
    fn path(id: u64, anchors: usize) -> Node {
        Node::path(NodeId(id), line(anchors), Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0))
    }

    /// Area type `id` whose frame has `anchors` anchors.
    fn area_type(id: u64, anchors: usize) -> Node {
        typed(id, TextKind::Area { frame: line(anchors) })
    }

    /// Type on a path `id` whose path has `anchors` anchors.
    fn path_type(id: u64, anchors: usize) -> Node {
        typed(id, TextKind::OnPath { path: line(anchors), start: 0.0, end: None })
    }

    fn typed(id: u64, kind: TextKind) -> Node {
        let mut t = TextObject::point(Point::ZERO, "Type", CharStyle::default());
        t.kind = kind;
        Node::new(NodeId(id), NodeKind::Text(Box::new(t)))
    }

    fn rect(id: u64) -> Node {
        let r = vectorcraft_geom::rect_from_points(Point::new(0.0, 0.0), Point::new(10.0, 10.0));
        Node::path(NodeId(id), vectorcraft_geom::shapes::rectangle(r), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 1.0))
    }

    fn small(n: &Node) -> bool {
        small_node(n, LIGHT_THUMB_NODES, LIGHT_THUMB_ANCHORS, true, None)
    }

    /// Is `n`, an object of `doc`, small?
    fn small_in(doc: &Document, n: &Node) -> bool {
        small_node(n, LIGHT_THUMB_NODES, LIGHT_THUMB_ANCHORS, true, Some(doc))
    }

    /// `n` with an opacity mask of `art`, disabled or not.
    fn masked(mut n: Node, art: Node, disabled: bool) -> Node {
        let mut m = OpacityMask::new(art, true);
        m.disabled = disabled;
        n.extras_mut().mask = Some(Box::new(m));
        n
    }

    #[test]
    fn the_anchors_of_an_opacity_mask_count_unless_it_is_disabled() {
        let long = || path(2, LIGHT_THUMB_ANCHORS + 1);
        assert!(!small(&masked(rect(1), long(), false)), "a small object with a long mask");
        assert!(small(&masked(rect(1), long(), true)), "a small object with a long mask that is disabled");
        let inner = Node::group(NodeId(3), vec![Arc::new(long())]);
        assert!(!small(&masked(rect(1), inner, false)), "a mask holding a long path");
        let child = Node::group(NodeId(1), vec![Arc::new(masked(rect(4), long(), false))]);
        assert!(!small(&child), "a group holding an object with a long mask");
        assert!(small_node(&masked(rect(1), long(), false), usize::MAX, LIGHT_THUMB_ANCHORS, false, None), "masks left out");
    }

    #[test]
    fn the_first_thumbnail_goes_to_the_worker_by_anchors_as_well_as_objects() {
        assert!(!small(&path(1, LIGHT_THUMB_ANCHORS + 1)), "one long path");
        assert!(small(&path(1, LIGHT_THUMB_ANCHORS)), "a path of as many anchors as the limit");
        let group = |n: usize| Node::group(NodeId(1), (0..n).map(|i| Arc::new(rect(i as u64 + 2))).collect());
        assert!(small(&group(LIGHT_THUMB_NODES)), "many small objects");
        assert!(!small(&group(LIGHT_THUMB_NODES + 1)), "more objects than the limit");
        assert!(small(&Node::layer(NodeId(1), "Layer 1", LayerColor::Preset(0))), "an empty layer");
        let inner = Node::group(NodeId(2), vec![Arc::new(path(3, LIGHT_THUMB_ANCHORS + 1))]);
        assert!(!small(&Node::group(NodeId(1), vec![Arc::new(rect(4)), Arc::new(inner)])), "a group holding a long path");
        let halves = Node::group(NodeId(1), vec![Arc::new(path(2, LIGHT_THUMB_ANCHORS / 2)), Arc::new(path(3, LIGHT_THUMB_ANCHORS / 2 + 1))]);
        assert!(!small(&halves), "paths whose anchors together pass the limit");
        assert!(small(&area_type(1, LIGHT_THUMB_ANCHORS)), "area type whose frame has as many anchors as the limit");
        assert!(!small(&area_type(1, LIGHT_THUMB_ANCHORS + 1)), "area type with a long frame");
        assert!(small(&path_type(1, LIGHT_THUMB_ANCHORS)), "type on a path of as many anchors as the limit");
        assert!(!small(&path_type(1, LIGHT_THUMB_ANCHORS + 1)), "type on a long path");
        let both = Node::group(NodeId(1), vec![Arc::new(path(2, LIGHT_THUMB_ANCHORS / 2)), Arc::new(area_type(3, LIGHT_THUMB_ANCHORS / 2 + 1))]);
        assert!(!small(&both), "a path and an area frame whose anchors together pass the limit");
    }

    /// Point type `id` of `glyphs` glyphs.
    fn glyphs(id: u64, glyphs: usize) -> Node {
        let t = TextObject::point(Point::ZERO, &"a".repeat(glyphs), CharStyle::default());
        Node::new(NodeId(id), NodeKind::Text(Box::new(t)))
    }

    /// A gradient mesh `id` of `rows` x `rows` patches.
    fn mesh(id: u64, rows: u32) -> Node {
        let m = vectorcraft_doc::GradientMesh::from_surface(rows, rows, &|u, v| Point::new(u * 100.0, v * 100.0), &|_, _| Color::BLACK);
        Node::new(NodeId(id), NodeKind::Mesh(m))
    }

    /// An image `id` of `w` x `h` pixels.
    fn image(id: u64, w: u32, h: u32) -> Node {
        let im = vectorcraft_doc::ImageObject {
            key: "im".into(),
            width: w,
            height: h,
            xf: Affine::IDENTITY,
            link: None,
            placement: Default::default(),
            pixels: None,
        };
        Node::new(NodeId(id), NodeKind::Image(Box::new(im)))
    }

    /// An instance `id` of symbol `name`.
    fn instance(id: u64, name: &str) -> Node {
        Node::new(NodeId(id), NodeKind::SymbolInstance { symbol: name.into(), xf: Affine::IDENTITY })
    }

    /// A document whose symbol `name` is `art`.
    fn with_symbol(name: &str, art: Node) -> Document {
        let mut doc = Document::new(200.0, 200.0);
        doc.symbols.push(vectorcraft_doc::Symbol { name: name.into(), art: Arc::new(art), dynamic: false });
        doc
    }

    /// A placed document `id` showing file `key`, read already when `art` is given.
    fn placed(id: u64, key: &str, art: Option<Node>) -> Node {
        let p = vectorcraft_doc::PlacedDocument {
            link: vectorcraft_doc::LinkInfo::new("/art/placed.vectorcraft"),
            key: key.into(),
            bounding: false,
            page_box: Default::default(),
            width: 100.0,
            height: 100.0,
            xf: Affine::IDENTITY,
            placement: Default::default(),
        };
        if let Some(art) = art {
            let frame = vectorcraft_geom::Rect::new(0.0, 0.0, 100.0, 100.0);
            vectorcraft_doc::placed_document::prime(&p, Arc::new(key.as_bytes().to_vec()), (Document::new(100.0, 100.0), art, frame));
        }
        Node::new(NodeId(id), NodeKind::PlacedDocument(Box::new(p)))
    }

    #[test]
    fn type_meshes_and_images_count_their_work_as_anchors() {
        let doc = Document::new(200.0, 200.0);
        let limit = LIGHT_THUMB_ANCHORS / GLYPH_ANCHORS;
        assert!(small_in(&doc, &glyphs(1, limit)), "type of as many glyphs as the limit");
        assert!(!small_in(&doc, &glyphs(1, limit + 1)), "type of more glyphs");
        assert!(!small_in(&doc, &glyphs(1, 5_000)), "long type");
        let beside = Node::group(NodeId(1), vec![Arc::new(glyphs(2, limit / 2)), Arc::new(path(3, LIGHT_THUMB_ANCHORS / 2 + 1))]);
        assert!(!small_in(&doc, &beside), "type and a path that together pass the limit");
        // (rows + 1)² points: 1,936 and 2,025.
        assert!(small_in(&doc, &mesh(1, 43)), "a small mesh");
        assert!(!small_in(&doc, &mesh(1, 44)), "a mesh of more points");
        let px = (LIGHT_THUMB_ANCHORS * IMAGE_PIXELS_PER_ANCHOR) as u32;
        assert!(small_in(&doc, &image(1, px / 100, 100)), "an image of as many pixels as the limit");
        assert!(!small_in(&doc, &image(1, px / 100, 101)), "a larger image");
        assert!(!small_in(&doc, &image(1, 4_000, 3_000)), "a photo");
        // Without the document only paths and frames count.
        assert!(small(&glyphs(1, 5_000)) && small(&mesh(1, 100)) && small(&image(1, 4_000, 3_000)));
    }

    #[test]
    fn a_symbol_instance_and_a_placed_document_count_the_art_they_show() {
        let doc = with_symbol("Long", path(10, LIGHT_THUMB_ANCHORS + 1));
        assert!(!small_in(&doc, &instance(1, "Long")), "an instance of a symbol of a long path");
        assert!(small(&instance(1, "Long")), "without the document");
        assert!(small_in(&doc, &instance(1, "Missing")), "an instance of a symbol that doesn't exist");
        let doc = with_symbol("Short", path(10, LIGHT_THUMB_ANCHORS));
        assert!(small_in(&doc, &instance(1, "Short")), "an instance of a symbol of a path at the limit");
        let two = Node::group(NodeId(1), vec![Arc::new(instance(2, "Short")), Arc::new(instance(3, "Short"))]);
        assert!(!small_in(&doc, &two), "two instances whose art together passes the limit");
        let many = with_symbol("Group", Node::group(NodeId(10), (0..LIGHT_THUMB_NODES).map(|i| Arc::new(rect(i as u64 + 11))).collect()));
        assert!(!small_in(&many, &instance(1, "Group")), "a symbol of more objects than the limit");
        // Inline graphics in type draw their symbol too.
        let mut t = TextObject::point(Point::ZERO, "a", CharStyle::default());
        t.runs[0].inline = Some(vectorcraft_doc::InlineArt { symbol: "Long".into(), scale: 1.0, baseline_shift: 0.0, bounds: None });
        let long = with_symbol("Long", path(10, LIGHT_THUMB_ANCHORS + 1));
        assert!(!small_in(&long, &Node::new(NodeId(1), NodeKind::Text(Box::new(t)))), "type showing a long symbol inline");

        let doc = Document::new(200.0, 200.0);
        assert!(small_in(&doc, &placed(1, "thumbs-small-placed", Some(path(10, LIGHT_THUMB_ANCHORS)))), "a placed file of a short path");
        assert!(!small_in(&doc, &placed(1, "thumbs-long-placed", Some(path(10, LIGHT_THUMB_ANCHORS + 1)))), "a placed file of a long path");
        assert!(!small_in(&doc, &placed(1, "thumbs-unread-placed", None)), "a placed file not read yet");
    }

    #[test]
    fn a_symbol_that_holds_itself_is_not_small_and_is_counted_to_an_end() {
        let doc = with_symbol("Loop", Node::group(NodeId(10), vec![Arc::new(rect(11)), Arc::new(instance(12, "Loop"))]));
        assert!(!small_in(&doc, &instance(1, "Loop")));
        assert!(!small_node(&instance(1, "Loop"), usize::MAX, usize::MAX, true, Some(&doc)), "without limits on objects and anchors");
        // Nested as deep as counted, the art is small.
        let mut doc = with_symbol("S0", rect(10));
        for i in 1..MAX_THUMB_NESTING {
            let art = Node::group(NodeId(10 + i as u64), vec![Arc::new(instance(100 + i as u64, &format!("S{}", i - 1)))]);
            doc.symbols.push(vectorcraft_doc::Symbol { name: format!("S{i}"), art: Arc::new(art), dynamic: false });
        }
        assert!(small_in(&doc, &instance(1, &format!("S{}", MAX_THUMB_NESTING - 1))), "symbols as deep as counted");
        let art = Node::group(NodeId(9), vec![Arc::new(instance(99, &format!("S{}", MAX_THUMB_NESTING - 1)))]);
        doc.symbols.push(vectorcraft_doc::Symbol { name: "Deeper".into(), art: Arc::new(art), dynamic: false });
        assert!(!small_in(&doc, &instance(1, "Deeper")), "one level deeper");
    }

    /// A document of symbols `L0` to `L{levels}`: `L0` is an empty group, and each one above it a
    /// group of `fan` instances of the one below.
    fn fan_out(levels: usize, fan: usize) -> Document {
        let mut doc = with_symbol("L0", Node::group(NodeId(10), vec![]));
        let mut id = 100;
        for level in 1..=levels {
            let below = format!("L{}", level - 1);
            let kids = (0..fan)
                .map(|_| {
                    id += 1;
                    Arc::new(instance(id, &below))
                })
                .collect();
            id += 1;
            doc.symbols.push(vectorcraft_doc::Symbol { name: format!("L{level}"), art: Arc::new(Node::group(NodeId(id), kids)), dynamic: false });
        }
        doc
    }

    #[test]
    fn symbols_that_each_hold_many_instances_are_counted_within_the_step_limit() {
        // 40 instances on each of 4 levels: 40⁴ (2,560,000) empty groups, and no anchors.
        let doc = fan_out(4, 40);
        let top = instance(1, "L4");
        assert!(!light_row(&top, Some(true), &doc), "a row drawn before, quick to render");
        assert!(!light_row(&top, None, &doc), "a row not drawn yet");
        let (small, steps) = weigh(&top, THUMB_COUNT_STEPS, usize::MAX, true, Some(&doc));
        assert!(!small && steps <= THUMB_COUNT_STEPS, "{steps} steps");
        let (small, steps) = weigh(&top, usize::MAX, usize::MAX, true, Some(&doc));
        assert!(!small && steps <= THUMB_COUNT_STEPS, "{steps} steps without a limit on objects");
        // 5 instances on each of 3 levels stay light once drawn quickly.
        let doc = fan_out(3, 5);
        assert!(light_row(&instance(1, "L3"), Some(true), &doc));
        assert!(!light_row(&instance(1, "L3"), Some(false), &doc), "drawn slowly");
    }

    #[test]
    fn empty_subpaths_text_runs_and_symbol_lookups_take_steps() {
        let doc = Document::new(200.0, 200.0);
        let style = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
        let empty = Node::path(NodeId(1), PathData::new(vec![SubPath::default(); THUMB_COUNT_STEPS]), style.clone());
        assert!(!small_in(&doc, &empty), "a path of many empty subpaths");
        let fewer = Node::path(NodeId(1), PathData::new(vec![SubPath::default(); 100]), style);
        assert!(small_in(&doc, &fewer), "a path of a few");
        let mut t = TextObject::point(Point::ZERO, "", CharStyle::default());
        t.runs = vec![vectorcraft_doc::TextRun::new("", CharStyle::default()); THUMB_COUNT_STEPS];
        assert!(!small_in(&doc, &Node::new(NodeId(2), NodeKind::Text(Box::new(t)))), "type of many empty runs");
        let mut doc = with_symbol("S", rect(10));
        for i in 0..THUMB_COUNT_STEPS {
            doc.symbols.push(vectorcraft_doc::Symbol { name: format!("Other {i}"), art: Arc::new(rect(11)), dynamic: false });
        }
        assert!(small_in(&doc, &instance(1, "S")), "the first of many symbols");
        assert!(!small_in(&doc, &instance(1, &format!("Other {}", THUMB_COUNT_STEPS - 1))), "the last of many symbols");
    }

    /// A document with `n` on its layer, and `n` as the layer holds it.
    fn doc_with(n: impl FnOnce(NodeId) -> Node) -> (Arc<Document>, Arc<Node>) {
        let mut doc = Document::new(200.0, 200.0);
        let layer = doc.layers.first().map(|l| l.id);
        let id = doc.alloc_id();
        doc.insert(layer, usize::MAX, n(id)).unwrap();
        let doc = Arc::new(doc);
        let node = node_arc(&doc, id).unwrap();
        (doc, node)
    }

    /// `n` of `doc` as `render` answers for a 24-point cell, weighed as a row not drawn before.
    fn rendered(doc: &Arc<Document>, n: &Arc<Node>, request: &ThumbRequest) -> ThumbRender {
        render(n.clone(), doc.clone(), request, light_row(n, None, doc))
    }

    /// `render` gives a heavy node as a job, whose pixels are those `render_thumb` makes here.
    fn assert_job_matches_a_render_made_here(doc: &Arc<Document>, n: &Arc<Node>) {
        let request = ThumbRequest::for_cell(24.0, 1.0);
        let ThumbRender::Later(job) = rendered(doc, n, &request) else { panic!("a heavy node is made on a lane") };
        let Ok(LoadedPicture::Image(there)) = job(&Cancel::new()) else { panic!("the job draws the node") };
        let here = render_thumb(&mut vectorcraft_render::Renderer::new(), doc, n, request.side).expect("this thread draws the node");
        let here = crate::canvas::color_image(&here);
        assert!(here.pixels.iter().any(|p| p.a() > 0), "the thumbnail shows the art");
        assert_eq!((there.size, &there.pixels), (here.size, &here.pixels), "the same pixels");
    }

    #[test]
    fn a_long_paths_thumbnail_is_made_on_the_worker_and_matches_one_made_here() {
        let (doc, n) = doc_with(|id| path(id.0, 50_000));
        assert_job_matches_a_render_made_here(&doc, &n);
        // A small object's thumbnail is made on this thread, at once.
        let (doc, n) = doc_with(|id| rect(id.0));
        let request = ThumbRequest::for_cell(24.0, 1.0);
        assert!(matches!(rendered(&doc, &n, &request), ThumbRender::Ready(_)));
        // A cancelled job gives no picture.
        let (doc, n) = doc_with(|id| path(id.0, 50_000));
        let ThumbRender::Later(job) = rendered(&doc, &n, &request) else { panic!("a long path is made on a lane") };
        let cancel = Cancel::new();
        cancel.cancel();
        assert!(job(&cancel).is_err());
    }

    #[test]
    fn an_instance_of_a_long_symbols_thumbnail_is_made_on_the_worker_and_matches_one_made_here() {
        let mut doc = with_symbol("Long", path(900, 50_000));
        let layer = doc.layers.first().map(|l| l.id);
        let id = doc.alloc_id();
        doc.insert(layer, usize::MAX, instance(id.0, "Long")).unwrap();
        let doc = Arc::new(doc);
        let n = node_arc(&doc, id).unwrap();
        assert_job_matches_a_render_made_here(&doc, &n);
    }

    #[test]
    fn a_row_whose_object_becomes_a_long_path_is_drawn_again_on_the_worker() {
        let request = ThumbRequest::for_cell(24.0, 1.0);
        let (doc, small_node) = doc_with(|id| rect(id.0));
        let (long_doc, long_node) = doc_with(|id| path(id.0, 50_000));
        let key = NodeThumb { uid: 1, node: small_node.id };
        let before = NodeThumbs::new(&doc, |_| Some(small_node.clone()));
        let after = NodeThumbs::new(&long_doc, |_| Some(long_node.clone()));
        assert_ne!(before.revision(&key, &request), after.revision(&key, &request), "a changed node is a new picture");
        assert!(matches!(before.render(&key, &request), ThumbRender::Ready(_)));
        assert!(matches!(after.render(&key, &request), ThumbRender::Later(_)), "and a long path is made on a lane");
    }
}
