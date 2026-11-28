use egui::Color32;
use vectorcraft_doc::{Node, NodeKind, SymbolBoxes};
use vectorcraft_geom::{BezPath, Point};

use super::VectorcraftApp;

pub(super) fn c32(rgb: [u8; 3]) -> Color32 {
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

/// What the view hides among a selected object's members, which the selection marks leave out as
/// the canvas does ([`Node::hidden_by_view`]): template layers while View › Hide Template is on,
/// and paths made into guides while the guides are hidden. The default hides nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ViewHides {
    pub(super) templates: bool,
    pub(super) guides: bool,
}

impl ViewHides {
    /// What `app`'s view hides.
    pub(super) fn of(app: &VectorcraftApp) -> Self {
        Self { templates: !app.ui.view.templates, guides: !app.ui.view.guides_shown() }
    }

    /// [`Node::drawn_children`] of `n` without the members the view leaves out. Every path of a
    /// compound path stays, since the compound path draws all of them, guides or not.
    fn children(self, n: &Node) -> impl Iterator<Item = &std::sync::Arc<Node>> {
        let hide_guides = self.guides && !matches!(n.kind, NodeKind::Compound { .. });
        n.drawn_children().filter(move |c| !c.hidden_by_view(self.templates, hide_guides))
    }
}

/// Outline of a node for highlighting (paths, compound children, area type frames, image bounds,
/// the box as drawn of type in perspective and of symbol instances, whose art `symbols` measures
/// ([`Node::drawn_corners`]); a clip group's clipping path only, as [`walk_drawn`] visits it,
/// without the members `hides` leaves out).
pub(super) fn node_outline(n: &Node, symbols: &SymbolBoxes, hides: ViewHides) -> BezPath {
    let mut bp = BezPath::new();
    walk_drawn(n, hides, &mut |c| match &c.kind {
        NodeKind::Path { path, .. } => bp.extend(path.to_bezpath()),
        // Area type shows its frame: the type area Direct Selection reshapes; type on a path its
        // path, which the selection tools' brackets sit on; point type shows its baseline(s).
        NodeKind::Text(t) => {
            if c.extras().perspective.is_none() {
                if let Some(path) = t.area_frame().or_else(|| t.type_path()) {
                    bp.extend(path.to_bezpath());
                } else if matches!(t.kind, vectorcraft_doc::TextKind::Point) {
                    for (a, b) in t.baselines() {
                        bp.move_to(t.xf * a);
                        bp.line_to(t.xf * b);
                    }
                }
            } else if let Some(q) = c.drawn_corners(symbols) {
                corners(&mut bp, q);
            }
        }
        // An image shows its frame, turned or distorted with it.
        NodeKind::Image(_) => {
            if let Some(frame) = c.image_frame() {
                bp.extend(frame);
            }
        }
        // A symbol instance shows its art's box, turned, skewed or in perspective with it.
        NodeKind::SymbolInstance { .. } => {
            if let Some(q) = c.drawn_corners(symbols) {
                corners(&mut bp, q);
            }
        }
        // A placed document shows its box, turned with it.
        NodeKind::PlacedDocument(p) => {
            let [a, b, c, d] = p.corners();
            bp.move_to(a);
            [b, c, d].into_iter().for_each(|q| bp.line_to(q));
            bp.close_path();
        }
        // A gradient mesh shows its mesh lines (#1111).
        NodeKind::Mesh(m) => bp.extend(m.lines().to_bezpath()),
        // An envelope shows its mesh (or top object), not its content.
        NodeKind::Envelope { .. } => {
            if let Some((lines, _)) = vectorcraft_doc::live::envelope_overlay(c) {
                bp.extend(lines.to_bezpath());
            }
        }
        _ => {}
    });
    bp
}

/// Whether `n` is a linked image or a placed document, which the selection marks cross.
fn is_linked(n: &Node) -> bool {
    match &n.kind {
        NodeKind::Image(im) => im.link.is_some(),
        NodeKind::PlacedDocument(_) => true,
        _ => false,
    }
}

/// The lines that cross each linked image and placed document of `n` that [`walk_drawn`] visits,
/// from corner to corner of its frame or box (#9080), without the members `hides` leaves out. The
/// selection marks draw them; the hover highlight does not.
pub(super) fn linked_crosses(n: &Node, hides: ViewHides) -> BezPath {
    let mut bp = BezPath::new();
    walk_drawn(n, hides, &mut |c| match &c.kind {
        NodeKind::Image(_) if is_linked(c) => {
            if let Some(cross) = c.image_cross() {
                bp.extend(cross);
            }
        }
        NodeKind::PlacedDocument(p) => {
            let [tl, tr, br, bl] = p.corners();
            bp.move_to(tl);
            bp.line_to(br);
            bp.move_to(tr);
            bp.line_to(bl);
        }
        _ => {}
    });
    bp
}

/// Adds the closed outline through the four corners `q` to `bp`.
fn corners(bp: &mut BezPath, q: [Point; 4]) {
    let [a, b, c, d] = q;
    bp.move_to(a);
    [b, c, d].into_iter().for_each(|p| bp.line_to(p));
    bp.close_path();
}

/// The most points ([`overlay_points`]) the selection's outlines and anchors, and the hover
/// highlight, are drawn for: an Image Trace of a photo selects hundreds of thousands of paths at
/// once, and selecting all of an opened PDF can select as many type objects, images and gradient
/// mesh pieces. Their outlines would make a mesh larger than the GPU takes in one buffer (#525,
/// #2460).
pub(super) const OVERLAY_MAX_ANCHORS: usize = 100_000;

/// Whether `nodes` have more than [`OVERLAY_MAX_ANCHORS`] points to outline ([`overlay_points`],
/// without the members `hides` leaves out; counting stops there).
pub(super) fn too_many_anchors<'a>(nodes: impl IntoIterator<Item = &'a Node>, hides: ViewHides) -> bool {
    over_limit(nodes, false, hides)
}

/// Whether `nodes` have more than [`OVERLAY_MAX_ANCHORS`] points to outline and to cross where they
/// are linked images or placed documents ([`linked_crosses`]), as the selection marks draw them
/// without the members `hides` leaves out.
pub(super) fn too_many_marks<'a>(nodes: impl IntoIterator<Item = &'a Node>, hides: ViewHides) -> bool {
    over_limit(nodes, true, hides)
}

/// Whether `nodes` have more than [`OVERLAY_MAX_ANCHORS`] points ([`overlay_points`], with the
/// ends of the crossing lines when `crosses`) in what [`walk_drawn`] visits; counting stops there.
fn over_limit<'a>(nodes: impl IntoIterator<Item = &'a Node>, crosses: bool, hides: ViewHides) -> bool {
    fn over(n: &Node, left: &mut usize, crosses: bool, hides: ViewHides) -> bool {
        let ends = if crosses && is_linked(n) { 4 } else { 0 };
        match left.checked_sub(overlay_points(n) + ends) {
            Some(l) => *left = l,
            None => return true,
        }
        // A clip group counts only its clipping path, the one child `walk_drawn` visits.
        if n.clips() {
            return n.children().and_then(|children| children.first()).is_some_and(|clip| over(clip, left, crosses, hides));
        }
        !matches!(n.kind, NodeKind::Envelope { .. }) && hides.children(n).any(|c| over(c, left, crosses, hides))
    }
    let mut left = OVERLAY_MAX_ANCHORS;
    nodes.into_iter().any(|n| over(n, &mut left, crosses, hides))
}

/// The points counted for `n` itself, without its children, toward [`OVERLAY_MAX_ANCHORS`]: a
/// path's anchors; the anchors of area type's frame or of type on a path's path; the four corners
/// of the box that area type or type on a path in perspective, images, symbol instances and
/// placed documents show; six for point type, its box's corners and the ends of the baseline
/// marker a selected one shows; and the points and the ends of the mesh lines of a gradient mesh
/// or an envelope.
fn overlay_points(n: &Node) -> usize {
    match &n.kind {
        NodeKind::Path { path, .. } => path.anchor_count(),
        NodeKind::Text(t) => match &t.kind {
            vectorcraft_doc::TextKind::Point => 6,
            _ if n.extras().perspective.is_some() => 4,
            vectorcraft_doc::TextKind::Area { frame } => frame.anchor_count(),
            vectorcraft_doc::TextKind::OnPath { path, .. } => path.anchor_count(),
        },
        NodeKind::Image(_) | NodeKind::SymbolInstance { .. } | NodeKind::PlacedDocument(_) => 4,
        NodeKind::Mesh(m) => m.lines().anchor_count() + m.points.len(),
        NodeKind::Envelope { .. } => {
            vectorcraft_doc::live::envelope_overlay(n).map_or(0, |(lines, grid)| lines.anchor_count() + grid.map_or(0, |g| g.points.len()))
        }
        _ => 0,
    }
}

/// [`Node::walk`] over what a selection highlight shows: an envelope's content is left out (the
/// envelope shows its mesh instead), a clip group or clipping layer shows only its clipping path,
/// its first child, not the art the path clips (#2422), and hidden members
/// ([`Node::drawn_children`]) and the members `hides` leaves out are left out.
pub(super) fn walk_drawn<'a>(n: &'a Node, hides: ViewHides, f: &mut impl FnMut(&'a Node)) {
    f(n);
    if matches!(n.kind, NodeKind::Envelope { .. }) {
        return;
    }
    if n.clips() {
        if let Some(clip) = n.children().and_then(|children| children.first()) {
            walk_drawn(clip, hides, f);
        }
        return;
    }
    for c in hides.children(n) {
        walk_drawn(c, hides, f);
    }
}

/// The topmost editable object under document point `p` at `zoom`, as what is drawn there
/// ([`hit_with`]): a swatch or a symbol dropped on art.
pub(super) fn hit_at(app: &VectorcraftApp, p: Point, zoom: f64) -> Option<vectorcraft_doc::hit::Hit> {
    hit_with(app, p, zoom, vectorcraft_doc::hit::hit_test)
}

/// The topmost editable object under document point `p` at `zoom`, as the selection tools pick
/// it: [`hit_at`], and a clipping path on its outline before the art it clips
/// ([`vectorcraft_doc::hit::HitOptions::clip_outlines`]), and an unfilled closed path inside where
/// nothing else is hit ([`vectorcraft_doc::hit::HitOptions::unfilled_interiors`]). Object
/// Highlighting and a right-click's selection show and take what a click of those tools would.
pub(super) fn selection_hit_at(app: &VectorcraftApp, p: Point, zoom: f64) -> Option<vectorcraft_doc::hit::Hit> {
    hit_with(app, p, zoom, |doc, p, opt| {
        vectorcraft_doc::hit::hit_test(doc, p, vectorcraft_doc::hit::HitOptions { clip_outlines: true, unfilled_interiors: true, ..opt })
    })
}

/// `test` on the active document at document point `p`, with the options the selection tools hit
/// objects with at `zoom` (Selection & Anchor Display › Tolerance and Object Selection by Path Only,
/// Type › Type Object Selection by Path Only, isolation mode).
pub(super) fn hit_with<T>(
    app: &VectorcraftApp,
    p: Point,
    zoom: f64,
    test: impl FnOnce(&vectorcraft_doc::Document, Point, vectorcraft_doc::hit::HitOptions) -> Option<T>,
) -> Option<T> {
    let prefs = &app.session.prefs;
    let opt = vectorcraft_doc::hit::HitOptions {
        tol: prefs.selection_tolerance / zoom,
        outline: app.outline_view(),
        path_only: prefs.object_selection_by_path_only,
        type_path_only: prefs.type_selection_by_path_only,
        scope: app.session.active().and_then(|st| st.isolation),
        skip_guides: !app.ui.view.guides_shown(),
        ..Default::default()
    };
    test(&app.session.active()?.doc, p, opt)
}
