use super::*;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Artboard {
    pub id: u32,
    pub name: String,
    /// Artboard rectangle in document points.
    pub rect: Rect,
    /// Locked (the Artboards panel's lock column): the Artboard tool and the artboard commands
    /// don't move, resize or delete it, Rearrange leaves it where it is, and the art on it can't be
    /// selected or edited ([`Document::on_locked_artboard`]), except new art while it stays
    /// selected ([`Document::new_art`]).
    #[serde(default, skip_serializing_if = "skip::is_default")]
    pub locked: bool,
    /// Show Center Mark (Artboard Options): the canvas draws a mark at the artboard's center.
    /// Exports and print leave it out. Always written, as before it could be set.
    #[serde(default)]
    pub show_center_mark: bool,
    /// Show Cross Hairs (Artboard Options): the canvas draws a line from the middle of each side
    /// through the center. Exports and print leave them out. Always written, as before it could
    /// be set.
    #[serde(default)]
    pub show_cross_hairs: bool,
    /// Show Video Safe Areas (Artboard Options): the canvas draws the artboard's action-safe and
    /// title-safe areas ([`Artboard::video_safe_areas`]). Exports and print leave them out. Written
    /// only when on, so a document that never shows them saves as before.
    #[serde(default, skip_serializing_if = "skip::is_default")]
    pub show_video_safe_areas: bool,
    /// Video Ruler Pixel Aspect Ratio (Artboard Options): the width of one video pixel over its
    /// height, which View › Rulers › Show Video Rulers counts the artboard in: a pixel is this many
    /// points wide and one point tall. 1 (square pixels) unless set, and written only when it isn't
    /// 1; always within [`VIDEO_PIXEL_ASPECT_RANGE`]: the commands refuse others, and a file's is
    /// brought into it as it opens.
    #[serde(default = "square_pixels", deserialize_with = "pixel_aspect_within_range", skip_serializing_if = "is_square_pixels")]
    pub video_ruler_pixel_aspect_ratio: f64,
    /// Background color (Properties › Artboards › Fill, Artboard Options). The canvas draws the
    /// artboard's paper in it, multiplied with the Simulate Colored Paper color while that is on;
    /// without one the paper is white, or the Simulate Colored Paper color. Exports and print leave
    /// it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<vectorcraft_color::Color>,
}

/// The Video Ruler Pixel Aspect Ratios an artboard takes (Artboard Options): from a pixel ten times
/// as tall as it is wide to one ten times as wide as it is tall, which holds the anamorphic and
/// standard-definition ratios video uses (0.9, 1.2, 1.33, 2) with room to spare.
pub const VIDEO_PIXEL_ASPECT_RANGE: std::ops::RangeInclusive<f64> = 0.1..=10.0;

/// How far in from each edge of an artboard the video safe areas lie, as a share of its width
/// (left and right) and of its height (top and bottom): the action-safe area 3.5% and the
/// title-safe (graphics-safe) area 5%, as EBU R 95 sets them for 16:9 television
/// (https://tech.ebu.ch/docs/r/r095.pdf, note 5).
pub const VIDEO_SAFE_INSETS: [f64; 2] = [0.035, 0.05];

fn square_pixels() -> f64 {
    1.0
}

fn is_square_pixels(v: &f64) -> bool {
    *v == 1.0
}

/// A file's Video Ruler Pixel Aspect Ratio, brought into [`VIDEO_PIXEL_ASPECT_RANGE`]: one that
/// isn't a number (NaN, infinite) reads as square pixels.
fn pixel_aspect_within_range<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    let v = f64::deserialize(d)?;
    let (lo, hi) = (*VIDEO_PIXEL_ASPECT_RANGE.start(), *VIDEO_PIXEL_ASPECT_RANGE.end());
    Ok(if v.is_finite() { v.clamp(lo, hi) } else { 1.0 })
}

impl Artboard {
    /// The action-safe and the title-safe area of the artboard, in that order, each inset from its
    /// edges by the share of [`VIDEO_SAFE_INSETS`] (Show Video Safe Areas).
    pub fn video_safe_areas(&self) -> [Rect; 2] {
        let r = self.rect;
        VIDEO_SAFE_INSETS.map(|k| r.inflate(-k * r.width(), -k * r.height()))
    }
}

/// A document's locked artboards ([`Document::locked_artboards`]).
pub struct LockedArtboards<'a> {
    pub(crate) rects: Vec<Rect>,
    pub(crate) symbols: SymbolBoxes<'a>,
    /// The new art, which isn't on them ([`Document::new_art`]).
    pub(crate) new_art: Arc<std::collections::HashSet<NodeId>>,
    /// The objects of pattern editing mode's layer ([`Document::pattern_edit`]), which are on no
    /// artboard.
    pub(crate) pattern_art: std::collections::HashSet<NodeId>,
    /// What [`LockedArtboards::overlaps`] found for each object it measured: the members of a
    /// group are each answered with one measurement of the group.
    pub(crate) held: std::cell::RefCell<std::collections::HashMap<NodeId, bool>>,
}

impl LockedArtboards<'_> {
    /// No artboard is locked.
    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }
    /// Is `n`, an object of a layer or sublayer, on a locked artboard: shown, not a guide, and its
    /// visual bounds (with its strokes, a symbol instance by its art) overlapping the artboard? An
    /// object counts for every artboard it overlaps, as Fit to Artwork Bounds counts it. A layer or
    /// sublayer is on none, nor is the art of pattern editing mode ([`Document::pattern_edit`]) or
    /// new art while it stays selected ([`Document::new_art`]).
    pub fn holds(&self, n: &Node) -> bool {
        !self.new_art.contains(&n.id) && self.overlaps(n)
    }
    /// [`LockedArtboards::holds`] for new art too: is `n` on a locked artboard, whether or not it
    /// is new art?
    pub fn overlaps(&self, n: &Node) -> bool {
        if self.rects.is_empty()
            || !n.visible
            || n.is_layer()
            || matches!(n.kind, NodeKind::Path { guide: true, .. })
            || self.pattern_art.contains(&n.id)
        {
            return false;
        }
        if let Some(held) = self.held.borrow().get(&n.id) {
            return *held;
        }
        let held =
            n.visual_bounds_with(&self.symbols).is_some_and(|b| self.rects.iter().any(|r| b.x0 < r.x1 && b.x1 > r.x0 && b.y0 < r.y1 && b.y1 > r.y0));
        self.held.borrow_mut().insert(n.id, held);
        held
    }
    /// Is node `id` on a locked artboard: is the object of a layer or sublayer that holds it (`id`
    /// itself when it is one, a group counting as one object) on one ([`LockedArtboards::holds`])?
    pub fn holds_id(&self, id: NodeId) -> bool {
        !self.rects.is_empty() && self.symbols.document().layer_object(id).is_some_and(|n| self.holds(n))
    }
}

/// [`Document::retired_artboard_id`]: not document content, so two documents are equal whatever it
/// holds. A clone shares the value with the document it was cloned from.
#[derive(Clone, Debug, Default)]
pub struct RetiredArtboardId(pub(crate) Arc<std::sync::atomic::AtomicU32>);

impl RetiredArtboardId {
    pub fn get(&self) -> u32 {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
    /// Raise the value to `id` when `id` is higher.
    pub fn raise(&self, id: u32) {
        self.0.fetch_max(id, std::sync::atomic::Ordering::Relaxed);
    }
}

impl PartialEq for RetiredArtboardId {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
