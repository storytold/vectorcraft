//! Live Corners on any path: the corners of its uncut outline (a live rectangle's or polygon's, a
//! live path's, or a plain path's own), each one's radius and kind, and the anchors of the path as
//! drawn that belong to each. [`set_corners`] edits them.

use std::borrow::Cow;
use std::collections::BTreeSet;

use vectorcraft_geom::corners::{Corner, cut_corners, fitted_radii, path_corners};
use vectorcraft_geom::shapes::{self, CornerKind};
use vectorcraft_geom::{Affine, PathData, Point, Rect};

use crate::node::{LiveShape, Node, NodeKind};
use crate::selection::AnchorRef;

/// Stored corner radii as document lengths for cutting and display (fractions when `relative`).
pub fn resolve_stored_radii(base: &PathData, stored: &[f64], relative: bool) -> Vec<f64> {
    if !relative {
        return stored.to_vec();
    }
    let corners = path_corners(base);
    let mut out = stored.to_vec();
    out.resize(base.anchor_count(), 0.0);
    for c in corners {
        if let Some(r) = out.get_mut(c.index) {
            *r = (*r).clamp(0.0, 1.0) * c.max_radius();
        }
    }
    out
}

/// Turn stored absolute radii into fractions of each corner's maximum, or the other way.
pub fn convert_radii_relative(live: &mut LiveShape, relative: bool) {
    if live.radii_relative() == relative {
        return;
    }
    let was_relative = live.radii_relative();
    let base = match live {
        LiveShape::Rectangle { w, h, .. } => shapes::rectangle(Rect::new(0.0, 0.0, *w, *h)),
        LiveShape::Polygon { .. } => live.polygon_outline(),
        LiveShape::Path { base, .. } => base.clone(),
        LiveShape::Ellipse { .. } | LiveShape::Line { .. } => return,
    };
    let corners = path_corners(&base);
    let radii: &mut [f64] = match live {
        LiveShape::Rectangle { radii, .. } => radii,
        LiveShape::Polygon { radii, .. } => radii,
        LiveShape::Path { radii, .. } => radii,
        LiveShape::Ellipse { .. } | LiveShape::Line { .. } => return,
    };
    for c in corners {
        if let Some(r) = radii.get(c.index) {
            let max = c.max_radius();
            radii[c.index] = if relative {
                if max > 1e-12 && !was_relative { (*r / max).clamp(0.0, 1.0) } else { *r }
            } else if was_relative {
                (*r).clamp(0.0, 1.0) * max
            } else {
                *r
            };
        }
    }
    match live {
        LiveShape::Rectangle { radii_relative: rel, .. }
        | LiveShape::Polygon { radii_relative: rel, .. }
        | LiveShape::Path { radii_relative: rel, .. } => {
            *rel = relative;
        }
        LiveShape::Ellipse { .. } | LiveShape::Line { .. } => {}
    }
}

/// The Live Corners of a path.
#[derive(Clone, Debug)]
pub struct LiveCorners<'a> {
    /// The path with its corners uncut, in the corners' own space: a live rectangle's (its size
    /// and radii in document units, see [`LiveShape::folded`]), else the document's.
    pub base: Cow<'a, PathData>,
    /// Maps `base` into the document.
    pub xf: Affine,
    /// The corners that can be cut, in anchor order ([`path_corners`] of `base`).
    pub corners: Vec<Corner>,
    radii: Vec<f64>,
    /// The radius each of `corners` is drawn with ([`fitted_radii`]).
    drawn: Vec<f64>,
    kinds: Vec<CornerKind>,
    /// Stored radii are fractions of each corner's maximum, not document lengths.
    relative: bool,
    /// For each anchor of the path as drawn (per subpath), the anchor of `base` it comes from.
    sources: Vec<Vec<usize>>,
}

impl<'a> LiveCorners<'a> {
    /// The Live Corners of `path` and its live shape; `None` for shapes without corners (ellipses,
    /// lines).
    pub fn new(path: &'a PathData, live: Option<&'a LiveShape>) -> Option<Self> {
        let (base, xf, stored, kinds, relative) = match live {
            None => (Cow::Borrowed(path), Affine::IDENTITY, vec![], vec![], false),
            Some(l @ LiveShape::Rectangle { .. }) => {
                let LiveShape::Rectangle { w, h, radii, kinds, radii_relative, xf } = l.folded() else { return None };
                (Cow::Owned(shapes::rectangle(Rect::new(0.0, 0.0, w, h))), xf, radii.to_vec(), kinds.to_vec(), radii_relative)
            }
            Some(l @ LiveShape::Polygon { radii, kinds, radii_relative, .. }) => {
                (Cow::Owned(l.polygon_outline()), Affine::IDENTITY, radii.clone(), kinds.clone(), *radii_relative)
            }
            Some(LiveShape::Path { base, radii, kinds, radii_relative }) => {
                (Cow::Borrowed(base), Affine::IDENTITY, radii.clone(), kinds.clone(), *radii_relative)
            }
            Some(LiveShape::Ellipse { .. } | LiveShape::Line { .. }) => return None,
        };
        let radii = resolve_stored_radii(&base, &stored, relative);
        let sources = if radii.iter().any(|r| *r > 0.0) {
            cut_corners(&base, &radii, &kinds).1
        } else {
            let mut first = 0;
            base.subpaths
                .iter()
                .map(|sp| {
                    first += sp.anchors.len();
                    (first - sp.anchors.len()..first).collect()
                })
                .collect()
        };
        let corners = path_corners(&base);
        let drawn = fitted_radii(&corners, &radii);
        Some(Self { corners, base, xf, radii: stored, drawn, kinds, relative, sources })
    }

    /// The Live Corners of a path object.
    pub fn of(node: &'a Node) -> Option<Self> {
        let NodeKind::Path { path, live, .. } = &node.kind else { return None };
        Self::new(path, live.as_deref())
    }

    /// The corner at anchor `index` of `base`.
    pub fn corner(&self, index: usize) -> Option<&Corner> {
        self.corners.binary_search_by_key(&index, |c| c.index).ok().and_then(|i| self.corners.get(i))
    }

    /// Whether corner radii are stored as fractions of each corner's maximum.
    pub fn radii_relative(&self) -> bool {
        self.relative
    }

    /// The radius corner `index` is stored at: document lengths, or a fraction in 0..1 when
    /// [`LiveCorners::radii_relative`].
    pub fn radius(&self, index: usize) -> f64 {
        self.radii.get(index).copied().unwrap_or(0.0)
    }

    /// The radius corner `index` is drawn with, in document units: its own, no larger than its
    /// neighbours leave room for (0 for an anchor that is no corner).
    pub fn drawn_radius(&self, index: usize) -> f64 {
        self.corners.binary_search_by_key(&index, |c| c.index).ok().and_then(|i| self.drawn.get(i)).copied().unwrap_or(0.0)
    }
    /// The kind corner `index` is cut with.
    pub fn kind(&self, index: usize) -> CornerKind {
        self.kinds.get(index).copied().unwrap_or_default()
    }

    /// Every corner.
    pub fn all(&self) -> BTreeSet<usize> {
        self.corners.iter().map(|c| c.index).collect()
    }

    /// The corners holding one of `anchors` (of the path as drawn).
    pub fn corners_of(&self, anchors: &BTreeSet<AnchorRef>) -> BTreeSet<usize> {
        anchors.iter().filter_map(|(si, ai)| self.sources.get(*si)?.get(*ai).copied()).filter(|k| self.corner(*k).is_some()).collect()
    }

    /// The corners Live Corners edit for a selection: those holding a selected anchor when the
    /// path is partly selected (Direct Selection), else every corner.
    pub fn picked(&self, partial: Option<&BTreeSet<AnchorRef>>) -> BTreeSet<usize> {
        let picked = partial.map(|a| self.corners_of(a)).unwrap_or_default();
        if picked.is_empty() { self.all() } else { picked }
    }

    /// The anchors of the path as drawn that belong to `corners` (to keep them selected as the
    /// path changes).
    pub fn anchors_of(&self, corners: &BTreeSet<usize>) -> BTreeSet<AnchorRef> {
        let mut out = BTreeSet::new();
        for (si, from) in self.sources.iter().enumerate() {
            out.extend(from.iter().enumerate().filter(|(_, k)| corners.contains(k)).map(|(ai, _)| (si, ai)));
        }
        out
    }

    /// For each anchor of the path as drawn (per subpath), the anchor of `base` it comes from.
    pub fn sources(&self) -> &[Vec<usize>] {
        &self.sources
    }

    /// The radius and kind `corners` share (each `None` when they differ, or for no corners).
    /// Radius is what was set ([`LiveCorners::radius`]: document lengths, or a fraction in 0..1
    /// when [`LiveCorners::radii_relative`]), not the fitted draw size ([`LiveCorners::drawn_radius`]).
    pub fn style(&self, corners: &BTreeSet<usize>) -> (Option<f64>, Option<CornerKind>) {
        fn shared<T: Copy>(mut it: impl Iterator<Item = T>, same: impl Fn(T, T) -> bool) -> Option<T> {
            let first = it.next()?;
            it.all(|x| same(x, first)).then_some(first)
        }
        let radius = shared(corners.iter().map(|k| self.radius(*k)), |a, b| (a - b).abs() < 1e-9);
        (radius, shared(corners.iter().map(|k| self.kind(*k)), |a, b| a == b))
    }
}

/// Set `radius` (document units; negative: 0) and/or `kind` on the `corners` (anchor indices of
/// the uncut outline, see [`LiveCorners`]) of a path and its live shape, then cut the path again.
/// A path that isn't a live shape keeps its uncut outline as a [`LiveShape::Path`], which is a
/// plain path again once no corner is cut. Ellipses and lines have no corners: they stay as they
/// are.
pub fn set_corners(path: &mut PathData, live: &mut Option<Box<LiveShape>>, corners: &BTreeSet<usize>, radius: Option<f64>, kind: Option<CornerKind>) {
    fn edit(
        base: &PathData,
        relative: bool,
        radii: &mut [f64],
        kinds: &mut [CornerKind],
        corners: &BTreeSet<usize>,
        radius: Option<f64>,
        kind: Option<CornerKind>,
    ) {
        let corners_on_base = path_corners(base);
        for k in corners {
            if let (Some(r), Some(slot)) = (radius, radii.get_mut(*k)) {
                *slot = if relative {
                    let max = corners_on_base.iter().find(|c| c.index == *k).map(|c| c.max_radius()).unwrap_or(0.0);
                    if max > 1e-12 { (r / max).clamp(0.0, 1.0) } else { 0.0 }
                } else {
                    r.max(0.0)
                };
            }
            if let (Some(kind), Some(slot)) = (kind, kinds.get_mut(*k)) {
                *slot = kind;
            }
        }
    }
    fn trim_vec_radii(radii: &mut Vec<f64>, kinds: &mut Vec<CornerKind>) {
        if radii.iter().all(|r| *r <= 0.0) {
            radii.clear();
        }
        if kinds.iter().all(|k| *k == CornerKind::Round) {
            kinds.clear();
        }
    }
    let l = live.get_or_insert_with(|| Box::new(LiveShape::Path { base: path.clone(), radii: vec![], kinds: vec![], radii_relative: false }));
    // Radii are document lengths and corners circular, also on a rectangle from a file that kept
    // an uneven scale in its transform (#442).
    l.fold_scale();
    let relative = l.radii_relative();
    match &mut **l {
        LiveShape::Rectangle { w, h, radii, kinds, .. } => {
            let base = shapes::rectangle(Rect::new(0.0, 0.0, *w, *h));
            edit(&base, relative, radii, kinds, corners, radius, kind);
        }
        LiveShape::Polygon { radius: poly_r, sides, xf, radii, kinds, .. } => {
            let outline = shapes::polygon(Point::ZERO, *poly_r, *sides, 0.0).transformed(*xf);
            let n = *sides as usize;
            radii.resize(n, 0.0);
            kinds.resize(n, CornerKind::Round);
            edit(&outline, relative, radii, kinds, corners, radius, kind);
            trim_vec_radii(radii, kinds);
        }
        LiveShape::Path { base, radii, kinds, .. } => {
            let n = base.anchor_count();
            radii.resize(n, 0.0);
            kinds.resize(n, CornerKind::Round);
            edit(base, relative, radii, kinds, corners, radius, kind);
            trim_vec_radii(radii, kinds);
        }
        LiveShape::Ellipse { .. } | LiveShape::Line { .. } => return,
    }
    match live.as_deref_mut() {
        Some(LiveShape::Path { base, radii, kinds, .. }) if radii.is_empty() && kinds.is_empty() => {
            *path = std::mem::take(base);
            *live = None;
        }
        Some(l) => *path = l.to_path(),
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_geom::{Point, SubPath};

    fn set(v: &[usize]) -> BTreeSet<usize> {
        v.iter().copied().collect()
    }

    fn anchors(v: &[usize]) -> BTreeSet<AnchorRef> {
        v.iter().map(|ai| (0, *ai)).collect()
    }

    #[test]
    fn rectangle_corners_map_to_their_anchors() {
        let l = LiveShape::Rectangle {
            w: 100.0,
            h: 50.0,
            radii: [0.0, 10.0, 0.0, 0.0],
            kinds: Default::default(),
            radii_relative: false,
            xf: Affine::IDENTITY,
        };
        let path = l.to_path();
        let c = LiveCorners::new(&path, Some(&l)).unwrap();
        // Anchors: top-left, the top-right corner's two, bottom-right, bottom-left.
        assert_eq!(c.sources(), [vec![0, 1, 1, 2, 3]]);
        assert_eq!(path.anchor_count(), 5);
        assert_eq!(c.corners_of(&anchors(&[2, 4])), set(&[1, 3]));
        assert!(c.corners_of(&anchors(&[9])).is_empty(), "out of range");
        assert_eq!(c.anchors_of(&set(&[1, 3])), anchors(&[1, 2, 4]));
        // Partly selected: the corners of the selected anchors; wholly (or no anchor): all four.
        assert_eq!(c.picked(Some(&anchors(&[0]))), set(&[0]));
        assert_eq!(c.picked(None), set(&[0, 1, 2, 3]));
        assert_eq!(c.picked(Some(&anchors(&[]))), set(&[0, 1, 2, 3]));
        assert_eq!(c.style(&set(&[1])), (Some(10.0), Some(CornerKind::Round)));
        assert_eq!(c.style(&set(&[0, 1])), (None, Some(CornerKind::Round)));
        assert_eq!(c.style(&set(&[0, 2, 3])), (Some(0.0), Some(CornerKind::Round)));
        let e = LiveShape::Ellipse { w: 1.0, h: 1.0, pie: (0.0, 360.0), xf: Affine::IDENTITY };
        assert!(LiveCorners::new(&e.to_path(), Some(&e)).is_none());
    }

    /// #511: a plain path (a star) rounds its corners and keeps them editable; with none cut it
    /// is plain again.
    #[test]
    fn a_plain_path_keeps_its_cut_corners_live() {
        let star = shapes::star(Point::new(100.0, 100.0), 50.0, 25.0, 5, 0.0);
        let (mut path, mut live) = (star.clone(), None);
        set_corners(&mut path, &mut live, &set(&[0, 2]), Some(5.0), None);
        let Some(LiveShape::Path { base, radii, kinds, .. }) = live.as_deref() else { panic!("live corners: {live:?}") };
        assert_eq!((base, radii.len(), kinds.len()), (&star, 10, 0));
        assert_eq!(path.anchor_count(), 12, "two corners cut");
        let c = LiveCorners::new(&path, live.as_deref()).unwrap();
        assert_eq!(c.style(&set(&[0, 2])), (Some(5.0), Some(CornerKind::Round)));
        assert_eq!(c.corners_of(&anchors(&[0, 11])), set(&[0]), "the first corner's cut ends the path");
        // Kinds alone keep it live; squaring every corner makes it the plain star again.
        set_corners(&mut path, &mut live, &set(&[0, 2]), Some(0.0), Some(CornerKind::Chamfer));
        assert!(matches!(live.as_deref(), Some(LiveShape::Path { radii, .. }) if radii.is_empty()));
        set_corners(&mut path, &mut live, &set(&[0, 2]), None, Some(CornerKind::Round));
        assert_eq!((path, live), (star, None));
    }

    /// A polygon's corners are cut in the document and survive a change of sides when shared.
    #[test]
    fn polygon_corners_follow_their_sides() {
        let mut l =
            LiveShape::Polygon { radius: 50.0, sides: 6, xf: Affine::translate((100.0, 100.0)), radii: vec![], kinds: vec![], radii_relative: false };
        let (mut path, mut live) = (l.to_path(), Some(Box::new(l.clone())));
        set_corners(&mut path, &mut live, &set(&[0, 1, 2, 3, 4, 5]), Some(8.0), None);
        assert_eq!(path.anchor_count(), 12);
        l = *live.clone().unwrap();
        l.set_sides(8);
        assert!(matches!(&l, LiveShape::Polygon { radii, .. } if *radii == vec![8.0; 8]));
        assert_eq!(l.to_path().anchor_count(), 16);
        // Mixed radii: the new corners are sharp.
        set_corners(&mut path, &mut live, &set(&[1]), Some(3.0), None);
        let mut l = *live.unwrap();
        l.set_sides(7);
        assert!(matches!(&l, LiveShape::Polygon { radii, .. } if *radii == [8.0, 3.0, 8.0, 8.0, 8.0, 8.0, 0.0]));
        // An uneven scale keeps the cuts circular: the radius scales by the mean scale.
        let mut p = l.clone();
        assert!(p.transform(Affine::scale_non_uniform(4.0, 1.0)));
        assert!(matches!(&p, LiveShape::Polygon { radii, .. } if (radii[0] - 16.0).abs() < 1e-9));
        assert!(!p.transform(Affine::translate((5.0, 0.0))), "a move needs no new cuts");
    }

    #[test]
    fn relative_radii_scale_with_rectangle_size() {
        let l = LiveShape::Rectangle {
            w: 100.0,
            h: 100.0,
            radii: [0.25, 0.25, 0.25, 0.25],
            kinds: Default::default(),
            radii_relative: true,
            xf: Affine::IDENTITY,
        };
        let path = l.to_path();
        let c = LiveCorners::new(&path, Some(&l)).unwrap();
        let r0 = c.drawn_radius(0);
        let l2 = LiveShape::Rectangle {
            w: 200.0,
            h: 200.0,
            radii: [0.25, 0.25, 0.25, 0.25],
            kinds: Default::default(),
            radii_relative: true,
            xf: Affine::IDENTITY,
        };
        let path2 = l2.to_path();
        let c2 = LiveCorners::new(&path2, Some(&l2)).unwrap();
        assert!((c2.drawn_radius(0) / r0 - 2.0).abs() < 0.05, "{r0} → {}", c2.drawn_radius(0));
    }

    #[test]
    fn convert_between_relative_and_absolute_radii() {
        let mut l = LiveShape::Rectangle {
            w: 100.0,
            h: 50.0,
            radii: [10.0, 0.0, 0.0, 0.0],
            kinds: Default::default(),
            radii_relative: false,
            xf: Affine::IDENTITY,
        };
        let before = LiveCorners::new(&l.to_path(), Some(&l)).unwrap().drawn_radius(0);
        convert_radii_relative(&mut l, true);
        assert!(l.radii_relative());
        let mid = LiveCorners::new(&l.to_path(), Some(&l)).unwrap().drawn_radius(0);
        assert!((mid - before).abs() < 0.5);
        convert_radii_relative(&mut l, false);
        assert!(!l.radii_relative());
        let after = LiveCorners::new(&l.to_path(), Some(&l)).unwrap().drawn_radius(0);
        assert!((after - before).abs() < 0.5);
    }

    #[test]
    fn open_paths_and_curves_keep_their_ends_and_smooth_anchors() {
        let pen = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 80.0)], false));
        let c = LiveCorners::new(&pen, None).unwrap();
        assert_eq!(c.all(), set(&[1]));
        assert_eq!(c.picked(Some(&anchors(&[0]))), set(&[1]), "an end is no corner: every corner");
        let (mut path, mut live) = (pen.clone(), None);
        set_corners(&mut path, &mut live, &c.all(), Some(10.0), None);
        assert_eq!(path.anchor_count(), 4);
        assert_eq!(path.subpaths[0].anchors[0].p, Point::new(0.0, 0.0));
        assert_eq!(path.subpaths[0].anchors[3].p, Point::new(100.0, 80.0));
    }
}
