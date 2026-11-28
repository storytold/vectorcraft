//! Slices (Object → Slice): rectangles that cut the artwork into the pieces web output saves.
//!
//! User slices are rectangles of their own ([`Document::slices`]); object slices follow the
//! bounds of an object with [`Node::slice`] options; auto slices fill the rest of the slice region
//! (the artboards with Clip to Artboard on, else the art and the slices) and are never stored.
//! User slice ids come from the document's id counter, so a slice id never names an object and
//! one id list can hold both kinds (an object slice goes by its object's id).
//!
//! Slices are numbered left to right, top to bottom, auto slices included.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use vectorcraft_geom::Rect;

use crate::{Document, Node, NodeId};

/// Below this a slice edge or size is no edge or size (points).
const EPS: f64 = 1e-6;

/// Enums whose values have a stable key (params, JSON) and a label (dialogs).
macro_rules! keyed {
    ($(#[$m:meta])* $name:ident { $($(#[$vm:meta])* $v:ident = ($key:literal, $label:literal)),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub enum $name {
            $($(#[$vm])* $v,)+
        }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$v),+];
            /// The value params and JSON use (`noImage`).
            pub fn key(self) -> &'static str {
                match self {
                    $($name::$v => $key,)+
                }
            }
            /// The dialog label (`No Image`).
            pub fn label(self) -> &'static str {
                match self {
                    $($name::$v => $label,)+
                }
            }
            /// The value `s` names: its key or label in any case.
            pub fn parse(s: &str) -> Option<Self> {
                let s = s.trim();
                Self::ALL.iter().copied().find(|v| v.key().eq_ignore_ascii_case(s) || v.label().eq_ignore_ascii_case(s))
            }
        }
    };
}

keyed!(
    /// Slice Options → Slice Type: what web output puts in the slice's cell.
    SliceKind {
        /// The artwork as an image (with a link, alt text and a status message).
        #[default]
        Image = ("image", "Image"),
        /// An empty cell showing `text` (HTML allowed) on the background.
        NoImage = ("noImage", "No Image"),
        /// The text of the sliced type object as HTML (object slices of type only).
        HtmlText = ("htmlText", "HTML Text"),
    }
);

keyed!(
    /// Horizontal alignment of a No Image / HTML Text cell's text.
    CellAlign {
        #[default]
        Default = ("default", "Default"),
        Left = ("left", "Left"),
        Center = ("center", "Center"),
        Right = ("right", "Right"),
    }
);

keyed!(
    /// Vertical alignment of a No Image / HTML Text cell's text.
    CellVAlign {
        #[default]
        Default = ("default", "Default"),
        Top = ("top", "Top"),
        Middle = ("middle", "Middle"),
        Baseline = ("baseline", "Baseline"),
        Bottom = ("bottom", "Bottom"),
    }
);

/// Slice Options: how web output writes a slice.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SliceOptions {
    #[serde(skip_serializing_if = "crate::skip::is_default")]
    pub kind: SliceKind,
    /// The slice's file name in web output; empty: `<document>_<number>` ([`Document::slice_name`]).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Image: the link the slice's image opens.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub url: String,
    /// Image: the frame the link opens in (`_blank`, `_self`, `_parent`, `_top` or a name).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub target: String,
    /// Image: the browser's status message over the slice.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub message: String,
    /// Image: the alternative text.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub alt: String,
    /// No Image: the text the cell shows (HTML allowed).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// The cell's background: empty (none), `matte` or `#rrggbb`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub background: String,
    #[serde(skip_serializing_if = "crate::skip::is_default")]
    pub h_align: CellAlign,
    #[serde(skip_serializing_if = "crate::skip::is_default")]
    pub v_align: CellVAlign,
}

/// A user slice: a rectangle drawn with the Slice tool or made by an Object → Slice command.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Slice {
    /// From the document's id counter (never an object's id).
    pub id: NodeId,
    pub rect: Rect,
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub options: SliceOptions,
}

/// Where a slice comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SliceSource {
    /// A [`Slice`] of the document.
    User,
    /// The bounds of an object with slice options.
    Object,
    /// Generated over what no other slice covers.
    Auto,
}

impl SliceSource {
    pub fn key(self) -> &'static str {
        match self {
            SliceSource::User => "user",
            SliceSource::Object => "object",
            SliceSource::Auto => "auto",
        }
    }
}

/// One slice as laid out: its id (none for auto slices), source, rectangle (clipped to the
/// artboards with Clip to Artboard on) and number (1-based).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliceArea {
    pub id: Option<NodeId>,
    pub source: SliceSource,
    pub rect: Rect,
    pub number: usize,
}

/// A rectangle with a size and finite coordinates.
pub fn valid_rect(r: Rect) -> bool {
    [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()) && r.width() > EPS && r.height() > EPS
}

impl Document {
    /// The user slice `id`.
    pub fn slice(&self, id: NodeId) -> Option<&Slice> {
        self.slices.iter().find(|s| s.id == id)
    }
    pub fn slice_mut(&mut self, id: NodeId) -> Option<&mut Slice> {
        self.slices.iter_mut().find(|s| s.id == id)
    }
    /// Is `id` a user slice or an object with slice options?
    pub fn is_slice(&self, id: NodeId) -> bool {
        self.slice(id).is_some() || self.node(id).is_some_and(|n| n.extras().slice.is_some())
    }
    /// The options of slice `id` (user or object).
    pub fn slice_options(&self, id: NodeId) -> Option<&SliceOptions> {
        match self.slice(id) {
            Some(s) => Some(&s.options),
            None => self.node(id)?.extras().slice.as_deref(),
        }
    }
    /// The rectangle of slice `id` before clipping: a user slice's own, an object slice's object's
    /// visual bounds.
    pub fn slice_bounds(&self, id: NodeId) -> Option<Rect> {
        match self.slice(id) {
            Some(s) => Some(s.rect),
            None => self.node(id).filter(|n| n.extras().slice.is_some())?.visual_bounds(),
        }
    }
    /// The visible objects with slice options and their visual bounds, in paint order.
    pub fn object_slices(&self) -> Vec<(NodeId, Rect)> {
        fn walk(nodes: &[Arc<Node>], out: &mut Vec<(NodeId, Rect)>) {
            for n in nodes.iter().filter(|n| n.visible) {
                if n.extras().slice.is_some()
                    && let Some(b) = n.visual_bounds()
                {
                    out.push((n.id, b));
                }
                if let Some(ch) = n.children() {
                    walk(ch, out);
                }
            }
        }
        let mut out = vec![];
        walk(&self.layers, &mut out);
        out
    }
    /// Every slice id: the user slices, then the object slices.
    pub fn slice_ids(&self) -> Vec<NodeId> {
        self.slices.iter().map(|s| s.id).chain(self.object_slices().into_iter().map(|(id, _)| id)).collect()
    }
    /// The box Clip to Artboard clips slices to: the artboards' bounds.
    pub fn slice_clip(&self) -> Option<Rect> {
        self.artboards.iter().map(|a| a.rect).reduce(|a, b| a.union(b))
    }
    /// The user and object slices with their rectangles as laid out (clipped to [`Self::slice_clip`]
    /// when Clip to Artboard is on; one wholly outside stays as it is).
    fn cut_slices(&self) -> Vec<(NodeId, SliceSource, Rect)> {
        let clip = self.slices_clip_to_artboard.then(|| self.slice_clip()).flatten();
        let clipped = |r: Rect| clip.map(|c| c.intersect(r)).filter(|c| valid_rect(*c)).unwrap_or(r);
        let users = self.slices.iter().map(|s| (s.id, SliceSource::User, s.rect));
        let objects = self.object_slices().into_iter().map(|(id, r)| (id, SliceSource::Object, r));
        users.chain(objects).map(|(id, src, r)| (id, src, clipped(r))).collect()
    }
    /// The slices as laid out, numbered left to right, top to bottom: the user and object slices
    /// and the auto slices filling the rest of the region (the artboards with Clip to Artboard on,
    /// else the visible art and the slices). Empty while the document has no user or object slice.
    pub fn slice_layout(&self) -> Vec<SliceArea> {
        let cut = self.cut_slices();
        if cut.is_empty() {
            return vec![];
        }
        let region = if self.slices_clip_to_artboard { self.slice_clip() } else { None }
            .or_else(|| cut.iter().fold(self.art_bounds(), |acc, (_, _, r)| vectorcraft_geom::union_opt(acc, Some(*r))));
        let rects: Vec<Rect> = cut.iter().map(|(_, _, r)| *r).collect();
        let autos = region.map(|g| auto_slices(g, &rects)).unwrap_or_default();
        let mut out: Vec<SliceArea> = cut
            .into_iter()
            .map(|(id, source, rect)| SliceArea { id: Some(id), source, rect, number: 0 })
            .chain(autos.into_iter().map(|rect| SliceArea { id: None, source: SliceSource::Auto, rect, number: 0 }))
            .collect();
        out.sort_by(|a, b| a.rect.y0.total_cmp(&b.rect.y0).then(a.rect.x0.total_cmp(&b.rect.x0)));
        for (i, a) in out.iter_mut().enumerate() {
            a.number = i + 1;
        }
        out
    }
    /// The name web output gives slice `area`: its options' name, else `<document>_<number>`
    /// (two digits at least, `Untitled-1_03`).
    pub fn slice_name(&self, area: &SliceArea) -> String {
        match area.id.and_then(|id| self.slice_options(id)).filter(|o| !o.name.is_empty()) {
            Some(o) => o.name.clone(),
            None => {
                let stem = self.title.rsplit_once('.').map_or(self.title.as_str(), |(s, _)| s);
                format!("{stem}_{:02}", area.number)
            }
        }
    }
}

/// The auto slices of `region`: rectangles covering what none of `cut` covers, without
/// overlapping each other or `cut`. The region is cut into rows at every slice's top and bottom;
/// each row's uncovered spans become slices, and a span continuing the one above it (same left and
/// right) extends it.
pub fn auto_slices(region: Rect, cut: &[Rect]) -> Vec<Rect> {
    if !valid_rect(region) {
        return vec![];
    }
    let inside: Vec<Rect> = cut.iter().map(|r| region.intersect(*r)).filter(|r| valid_rect(*r)).collect();
    // Every band scans every cut: past a bound the layout is refused, not truncated.
    if inside.len() > MAX_AUTO_CUTS {
        return vec![];
    }
    let mut ys: Vec<f64> = inside.iter().flat_map(|r| [r.y0, r.y1]).chain([region.y0, region.y1]).collect();
    ys.sort_by(f64::total_cmp);
    ys.dedup_by(|a, b| (*a - *b).abs() <= EPS);
    let (mut open, mut done): (Vec<Rect>, Vec<Rect>) = (vec![], vec![]);
    for band in ys.windows(2) {
        let [y0, y1] = [band[0], band[1]];
        let mid = (y0 + y1) / 2.0;
        let mut spans: Vec<(f64, f64)> = inside.iter().filter(|r| r.y0 < mid && mid < r.y1).map(|r| (r.x0, r.x1)).collect();
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        // The uncovered spans of this row.
        let mut free = vec![];
        let mut x = region.x0;
        for (a, b) in spans {
            if a - x > EPS {
                free.push((x, a));
            }
            x = x.max(b);
        }
        if region.x1 - x > EPS {
            free.push((x, region.x1));
        }
        // Spans continuing an open slice extend it; the others close.
        let mut next = Vec::with_capacity(free.len());
        for (a, b) in free {
            if done.len() + open.len() + next.len() >= MAX_AUTO_SLICES {
                break;
            }
            match open.iter().position(|r: &Rect| (r.x0 - a).abs() <= EPS && (r.x1 - b).abs() <= EPS) {
                Some(i) => {
                    let mut r = open.swap_remove(i);
                    r.y1 = y1;
                    next.push(r);
                }
                None => next.push(Rect::new(a, y0, b, y1)),
            }
        }
        done.append(&mut open);
        open = next;
        // Crossing strips make a cell of every crossing: the layout stops, short, past a bound.
        if done.len() + open.len() >= MAX_AUTO_SLICES {
            break;
        }
    }
    done.append(&mut open);
    done.truncate(MAX_AUTO_SLICES);
    done
}

/// The most slices automatic layout makes.
const MAX_AUTO_SLICES: usize = 10_000;

/// The most cut rectangles automatic layout works through.
const MAX_AUTO_CUTS: usize = 2_000;
