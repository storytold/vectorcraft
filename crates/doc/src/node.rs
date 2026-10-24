//! Document nodes.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_geom::corners::cut_corners;
use vectorcraft_geom::shapes::{self, CornerKind};
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, Point, Rect, Shape};

use crate::appearance::Appearance;
use crate::live::{BlendSpec, EnvelopeKind, GradientMesh, Outliner};
use crate::pattern::RepeatSpec;
use crate::symbols::{SYMBOL_SQUARE, SymbolBoxes};
use crate::text::TextObject;

/// Stable per-document object id. Never reused.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(pub u64);

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Layer colours used for selection highlighting (index into [`LAYER_COLORS`]) or a custom RGB.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayerColor {
    Preset(u8),
    Custom([u8; 3]),
}

/// Layer highlight colours, assigned in sequence to new layers. First is Illustrator-like light blue.
pub const LAYER_COLORS: [(&str, [u8; 3]); 27] = [
    ("Light Blue", [0x4f, 0x80, 0xff]),
    ("Red", [0xff, 0x4f, 0x4f]),
    ("Green", [0x4f, 0xff, 0x4f]),
    ("Blue", [0x4f, 0x4f, 0xff]),
    ("Yellow", [0xff, 0xff, 0x4f]),
    ("Magenta", [0xff, 0x4f, 0xff]),
    ("Cyan", [0x4f, 0xff, 0xff]),
    ("Gray", [0x80, 0x80, 0x80]),
    ("Black", [0x00, 0x00, 0x00]),
    ("Orange", [0xff, 0x66, 0x00]),
    ("Dark Green", [0x00, 0x80, 0x00]),
    ("Teal", [0x00, 0x80, 0x80]),
    ("Tan", [0xcc, 0x99, 0x66]),
    ("Brown", [0x99, 0x33, 0x00]),
    ("Violet", [0x99, 0x33, 0xff]),
    ("Gold", [0xff, 0x99, 0x00]),
    ("Dark Blue", [0x00, 0x00, 0x80]),
    ("Pink", [0xff, 0x99, 0xcc]),
    ("Lavender", [0x99, 0x99, 0xff]),
    ("Brick Red", [0x99, 0x00, 0x00]),
    ("Olive Green", [0x66, 0x66, 0x00]),
    ("Peach", [0xff, 0x99, 0x99]),
    ("Burgundy", [0x99, 0x00, 0x33]),
    ("Grass Green", [0x99, 0xcc, 0x00]),
    ("Ochre", [0x99, 0x66, 0x00]),
    ("Purple", [0x66, 0x00, 0x99]),
    ("Light Gray", [0xbb, 0xbb, 0xbb]),
];

impl LayerColor {
    pub fn rgb(self) -> [u8; 3] {
        match self {
            LayerColor::Preset(i) => LAYER_COLORS[i as usize % LAYER_COLORS.len()].1,
            LayerColor::Custom(c) => c,
        }
    }
}

/// Live shape parameters (Illustrator's "Live Shapes"). The node's path is regenerated from these
/// while the shape stays live; editing anchors directly converts it to a plain path.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "lowercase")]
pub enum LiveShape {
    Rectangle {
        /// Untransformed width/height. Transforms fold their scale into these (see
        /// [`LiveShape::transform`]), so they are document lengths unless `xf` shears.
        w: f64,
        h: f64,
        /// Corner radii: top-left, top-right, bottom-right, bottom-left. In the same units as
        /// `w`/`h`: the corners are circular arcs in the document whenever `xf` doesn't scale.
        radii: [f64; 4],
        /// Corner kinds, in the same order (Live Corners' corner types).
        #[serde(default, skip_serializing_if = "all_round")]
        kinds: [CornerKind; 4],
        /// When true, `radii` are fractions in 0..1 of each corner's largest possible radius.
        #[serde(default, skip_serializing_if = "is_false")]
        radii_relative: bool,
        /// Maps the untransformed shape (origin at its top-left) into the document. A rotation
        /// or reflection plus a move, unless a shear (or a file saved before transforms folded
        /// their scale into `w`/`h`) put more in it.
        xf: Affine,
    },
    Ellipse {
        w: f64,
        h: f64,
        /// Pie start/end angles in degrees (0/360 = full ellipse).
        pie: (f64, f64),
        xf: Affine,
    },
    Polygon {
        radius: f64,
        /// 3–1000, as [`LiveShape::set_sides`] gives it; a file's count is clamped when read.
        #[serde(deserialize_with = "de_sides")]
        sides: u32,
        xf: Affine,
        /// Corner radii in document units, by vertex from the first one clockwise (none: sharp).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        radii: Vec<f64>,
        /// Corner kinds, in the same order (none: round).
        #[serde(default, skip_serializing_if = "all_round")]
        kinds: Vec<CornerKind>,
        #[serde(default, skip_serializing_if = "is_false")]
        radii_relative: bool,
    },
    Line {
        a: Point,
        b: Point,
    },
    /// Any other path whose corners Live Corners cut (a star, a pen path): it keeps its uncut
    /// outline so the corners stay editable. Without a cut corner it is a plain path again.
    Path {
        /// The path with its corners uncut, in the document.
        base: PathData,
        /// Corner radii in document units, by anchor index of `base` (counting every subpath's
        /// anchors in order; missing: sharp).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        radii: Vec<f64>,
        /// Corner kinds, in the same order (missing: round).
        #[serde(default, skip_serializing_if = "all_round")]
        kinds: Vec<CornerKind>,
        #[serde(default, skip_serializing_if = "is_false")]
        radii_relative: bool,
    },
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl LiveShape {
    pub fn radii_relative(&self) -> bool {
        match self {
            LiveShape::Rectangle { radii_relative, .. } | LiveShape::Polygon { radii_relative, .. } | LiveShape::Path { radii_relative, .. } => {
                *radii_relative
            }
            LiveShape::Ellipse { .. } | LiveShape::Line { .. } => false,
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            LiveShape::Rectangle { radii, .. } if radii.iter().any(|r| *r > 0.0) => "Rounded Rectangle",
            LiveShape::Rectangle { .. } => "Rectangle",
            LiveShape::Ellipse { .. } => "Ellipse",
            LiveShape::Polygon { .. } => "Polygon",
            LiveShape::Line { .. } => "Line",
            LiveShape::Path { .. } => "Path",
        }
    }
    /// Regenerate the path.
    pub fn to_path(&self) -> PathData {
        match self {
            LiveShape::Rectangle { w, h, radii, kinds, radii_relative, xf } => {
                let base = shapes::rectangle(Rect::new(0.0, 0.0, *w, *h));
                let resolved = crate::corners::resolve_stored_radii(&base, radii, *radii_relative);
                let radii = std::array::from_fn(|i| resolved.get(i).copied().unwrap_or(0.0));
                shapes::rectangle_with_corners(Rect::new(0.0, 0.0, *w, *h), radii, *kinds).transformed(*xf)
            }
            LiveShape::Ellipse { w, h, pie, xf } => shapes::ellipse_pie(Rect::new(0.0, 0.0, *w, *h), pie.0, pie.1).transformed(*xf),
            LiveShape::Polygon { radii, kinds, radii_relative, .. } => {
                let outline = self.polygon_outline();
                let radii = crate::corners::resolve_stored_radii(&outline, radii, *radii_relative);
                cut_corners(&outline, &radii, kinds).0
            }
            LiveShape::Line { a, b } => shapes::line(*a, *b),
            LiveShape::Path { base, radii, kinds, radii_relative } => {
                let radii = crate::corners::resolve_stored_radii(base, radii, *radii_relative);
                cut_corners(base, &radii, kinds).0
            }
        }
    }
    /// A polygon's outline with its corners uncut, in the document (empty for other shapes).
    pub fn polygon_outline(&self) -> PathData {
        match self {
            LiveShape::Polygon { radius, sides, xf, .. } => shapes::polygon(Point::ZERO, *radius, *sides, 0.0).transformed(*xf),
            _ => PathData::default(),
        }
    }
    /// Give a polygon `n` sides (3–1000). Its corners keep the radius and kind they shared; else
    /// the corners past the old last one are sharp.
    pub fn set_sides(&mut self, n: u64) {
        let LiveShape::Polygon { sides, radii, kinds, .. } = self else { return };
        *sides = n.clamp(3, 1000) as u32;
        fn spread<T: Copy + PartialEq + Default>(v: &mut Vec<T>, n: usize) {
            match v.first().copied() {
                Some(x) if v.iter().all(|y| *y == x) => *v = vec![x; n],
                Some(_) => v.resize(n, T::default()),
                None => {}
            }
        }
        spread(radii, *sides as usize);
        spread(kinds, *sides as usize);
    }
    /// A polygon's radius in the document, centre to vertex: its own radius by the mean scale of
    /// `xf` (exact while its sides are equal). None for other shapes.
    pub fn polygon_radius(&self) -> Option<f64> {
        let LiveShape::Polygon { radius, xf, .. } = self else { return None };
        Some(radius * xf.determinant().abs().sqrt())
    }
    /// A polygon's angle: the counterclockwise degrees in [0, 360) its first vertex is turned by
    /// from straight up (the Rotate field's sense). None for other shapes.
    pub fn polygon_angle(&self) -> Option<f64> {
        let LiveShape::Polygon { .. } = self else { return None };
        // No −0° or 360° from rounding: a polygon drawn upright reads 0°.
        let a = (-self.angle_deg()).rem_euclid(360.0);
        Some(if a > 360.0 - 1e-9 { 0.0 } else { a + 0.0 })
    }
    /// Whether a polygon's sides are all as long: `xf` scales it evenly and doesn't shear it. True
    /// for other shapes.
    pub fn polygon_sides_equal(&self) -> bool {
        let LiveShape::Polygon { xf, .. } = self else { return true };
        let [a, b, c, d, _, _] = xf.as_coeffs();
        let (sx, sy) = (a.hypot(b), c.hypot(d));
        (sx - sy).abs() <= 1e-9 * sx.max(sy) && (a * c + b * d).abs() <= 1e-9 * sx * sy
    }
    /// Give a polygon the document radius `r` (centre to vertex, see [`LiveShape::polygon_radius`]).
    pub fn set_polygon_radius(&mut self, r: f64) {
        let LiveShape::Polygon { radius, xf, .. } = self else { return };
        let k = xf.determinant().abs().sqrt();
        if k > 1e-12 && r.is_finite() {
            *radius = r.max(0.0) / k;
        }
    }
    /// The document radius that makes a polygon's sides `side` long (with its sides equal).
    pub fn polygon_radius_for_side(&self, side: f64) -> Option<f64> {
        let LiveShape::Polygon { sides, .. } = self else { return None };
        Some(side / (2.0 * (std::f64::consts::PI / f64::from((*sides).max(3))).sin()))
    }
    /// Turn a polygon about its centre to `angle` (degrees, as [`LiveShape::polygon_angle`]).
    pub fn set_polygon_angle(&mut self, angle: f64) {
        let Some(now) = self.polygon_angle() else { return };
        if let LiveShape::Polygon { xf, .. } = self
            && angle.is_finite()
        {
            let centre = *xf * Point::ZERO;
            *xf = Affine::rotate_about((now - angle).to_radians(), centre) * *xf;
        }
    }
    /// Make a polygon's sides equal (Make Sides Equal): drop the uneven scale and the shear from
    /// `xf`, keeping its centre, angle, reflection and [`LiveShape::polygon_radius`].
    pub fn make_sides_equal(&mut self) {
        let Some(r) = self.polygon_radius() else { return };
        let LiveShape::Polygon { radius, xf, .. } = self else { return };
        let [a, b, _, _, e, f] = xf.as_coeffs();
        let flip = if xf.determinant() < 0.0 { Affine::FLIP_Y } else { Affine::IDENTITY };
        *xf = Affine::translate((e, f)) * Affine::rotate(b.atan2(a)) * flip;
        *radius = r;
    }
    /// Apply `a` to the shape. True when the shape is no longer `a` applied to the old path, so
    /// the caller must regenerate the path with [`LiveShape::to_path`].
    ///
    /// A rectangle keeps round corners whatever its proportions: when `a` scales it along its own
    /// sides (any rotation or reflection, no shear), the scale goes into `w`/`h` and `xf` keeps
    /// only the rotation, reflection and position. The radii scale by the mean scale (the square
    /// root of the determinant), which [`LiveShape::keep_corners`] undoes for Scale Corners off.
    /// A shear can't keep circular corners: `xf` takes it, as do moves (so files saved with a
    /// scale in `xf` keep their exact geometry until they're next transformed). A polygon's or a
    /// path's corner radii are document lengths: they scale by the mean scale and the corners are
    /// cut again, circular whatever `a` does.
    pub fn transform(&mut self, a: Affine) -> bool {
        match self {
            LiveShape::Rectangle { xf, .. } => {
                *xf = a * *xf;
                !moves_only(a) && self.fold_scale()
            }
            LiveShape::Ellipse { xf, .. } => {
                *xf = a * *xf;
                false
            }
            LiveShape::Polygon { xf, radii, radii_relative, .. } => {
                *xf = a * *xf;
                scale_radii(radii, a, *radii_relative)
            }
            LiveShape::Path { base, radii, radii_relative, .. } => {
                base.transform(a);
                scale_radii(radii, a, *radii_relative)
            }
            LiveShape::Line { a: p, b } => {
                *p = a * *p;
                *b = a * *b;
                false
            }
        }
    }
    /// Move the scale of a rectangle's `xf` into `w`/`h` (and its radii, by the mean scale), so
    /// `xf` keeps only a rotation or reflection plus a move and the corners are circular in the
    /// document. False (and nothing changes) when there is no scale to move, when `xf` shears or
    /// isn't finite, and for other shapes. Files saved before transforms did this (#291) carry an
    /// uneven scale in `xf`: their corners are elliptical until this runs.
    pub fn fold_scale(&mut self) -> bool {
        let LiveShape::Rectangle { w, h, radii, radii_relative, xf, .. } = self else { return false };
        let [m0, m1, m2, m3, m4, m5] = xf.as_coeffs();
        let (sx, sy) = (m0.hypot(m1), m2.hypot(m3));
        let orthogonal = sx > 1e-12 && sy > 1e-12 && (m0 * m2 + m1 * m3).abs() <= 1e-9 * sx * sy;
        let finite = xf.as_coeffs().iter().all(|c| c.is_finite()) && w.is_finite() && h.is_finite();
        let unscaled = (sx - 1.0).abs() < 1e-12 && (sy - 1.0).abs() < 1e-12;
        if !orthogonal || !finite || unscaled {
            return false;
        }
        let k = (sx * sy).sqrt();
        if !*radii_relative {
            radii.iter_mut().for_each(|r| *r *= k);
        }
        *w *= sx;
        *h *= sy;
        *xf = Affine::new([m0 / sx, m1 / sx, m2 / sy, m3 / sy, m4, m5]);
        true
    }
    /// This shape with [`LiveShape::fold_scale`] applied: a rectangle's size and radii in document
    /// units (what Live Corners show and edit).
    pub fn folded(&self) -> Self {
        let mut s = self.clone();
        s.fold_scale();
        s
    }
    /// Divide live corner radii by `k`, the mean scale of a transform just applied, so the corners
    /// keep their size (Scale Corners off). False when nothing changed.
    pub fn keep_corners(&mut self, k: f64) -> bool {
        let relative = self.radii_relative();
        let radii: &mut [f64] = match self {
            LiveShape::Rectangle { radii, .. } => radii,
            LiveShape::Polygon { radii, .. } | LiveShape::Path { radii, .. } => radii,
            LiveShape::Ellipse { .. } | LiveShape::Line { .. } => return false,
        };
        if relative || !radii.iter().any(|r| *r > 0.0) || k <= 1e-12 {
            return false;
        }
        radii.iter_mut().for_each(|r| *r /= k);
        true
    }
    /// Rotation angle of the live shape in degrees, clockwise on the page (y points down).
    pub fn angle_deg(&self) -> f64 {
        match self {
            LiveShape::Rectangle { xf, .. } | LiveShape::Ellipse { xf, .. } | LiveShape::Polygon { xf, .. } => {
                let c = xf.as_coeffs();
                c[1].atan2(c[0]).to_degrees()
            }
            LiveShape::Line { a, b } => (b.y - a.y).atan2(b.x - a.x).to_degrees(),
            LiveShape::Path { .. } => 0.0,
        }
    }
}

fn all_round(kinds: &[CornerKind]) -> bool {
    kinds.iter().all(|k| *k == CornerKind::Round)
}

/// A polygon's side count read from a file, clamped as the commands clamp it: the outline and its
/// corner lists hold one entry per side, so a huge count would allocate gigabytes.
fn de_sides<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    u32::deserialize(d).map(|n| n.clamp(3, 1000))
}

/// Whether `a` only moves (no rotation, scale, shear or reflection).
fn moves_only(a: Affine) -> bool {
    let [a0, a1, a2, a3, _, _] = a.as_coeffs();
    (a0 - 1.0).abs() < 1e-12 && a1.abs() < 1e-12 && a2.abs() < 1e-12 && (a3 - 1.0).abs() < 1e-12
}

/// Scale corner radii in document units by the mean scale of `a`, applied to their path. True
/// when the path must be cut again: some corner is cut and `a` does more than move it (an
/// uneven scale would otherwise make the cuts elliptical).
fn scale_radii(radii: &mut [f64], a: Affine, relative: bool) -> bool {
    if relative || moves_only(a) || !radii.iter().any(|r| *r > 0.0) {
        return false;
    }
    let k = a.determinant().abs().sqrt();
    radii.iter_mut().for_each(|r| *r *= k);
    true
}

/// What a transform scales besides geometry, by its mean scale (the square root of its
/// determinant). `Scaling::default()` (and `false`) scales nothing else; `true` scales stroke
/// weights and dashes only. Transforms the user asks for take theirs from Scale Strokes & Effects
/// and Scale Corners.
#[derive(Clone, Copy, Debug, Default)]
pub struct Scaling {
    /// Stroke weights, dash lengths and dash offsets scale.
    pub strokes: bool,
    /// With `strokes`: scales an effect's distance parameters (the effects catalogue's).
    pub effects: Option<fn(&mut crate::Effect, f64)>,
    /// Type keeps its character strokes' weight (they otherwise scale with the type).
    pub keep_type_strokes: bool,
    /// Live corner radii keep their size.
    pub keep_corners: bool,
    /// Pattern fills and strokes move with the art (Transform Patterns; General › Transform
    /// Pattern Tiles is its default): their tiles transform too, instead of staying put.
    pub patterns: bool,
}

impl Scaling {
    /// The mean scale of `a` when it scales (isn't 1): what strokes, effects and corners scale by.
    pub fn factor(a: Affine) -> Option<f64> {
        let k = a.determinant().abs().sqrt();
        ((k - 1.0).abs() > 1e-9).then_some(k)
    }
}

impl From<bool> for Scaling {
    fn from(strokes: bool) -> Self {
        Self { strokes, ..Self::default() }
    }
}

/// Embedded or linked raster image.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageObject {
    /// Key into [`crate::Document::images`].
    pub key: String,
    pub width: u32,
    pub height: u32,
    /// Maps pixel space (0..w, 0..h) into the document.
    pub xf: Affine,
    /// The file a linked image shows (File → Place with Link); `None`: embedded. Files saved
    /// before links had details hold just the path.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::links::de_link")]
    pub link: Option<crate::LinkInfo>,
    /// Links panel → Placement Options: how a file read again takes this image's place.
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub placement: crate::links::PlacementOptions,
    /// The pixels of an image in art made outside a document (type's outlines, a glyph's art),
    /// which has no [`crate::Document::images`] to keep them in. A document takes them in
    /// ([`crate::Document::adopt_pixels`]): no image in one holds its own.
    #[serde(skip)]
    pub pixels: Option<Arc<crate::ImageBlob>>,
}

/// Written with its `type` first, then its fields (an internally tagged enum); read by
/// [`kind_de`], which takes the fields straight from the file when `type` comes first.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum NodeKind {
    /// A top-level layer or sublayer.
    Layer {
        color: LayerColor,
        #[serde(default)]
        template: bool,
        #[serde(default = "yes")]
        printable: bool,
        children: Vec<Arc<Node>>,
        /// Layer clipping mask: the first (bottom-most) child clips the others, as in a clip group.
        #[serde(default, skip_serializing_if = "crate::skip::is_default")]
        clip: bool,
        /// Layer Options → Preview: off, the layer's art is drawn (and clicked) in outline on screen
        /// (Ctrl-click its eye in the Layers panel).
        #[serde(default = "yes", skip_serializing_if = "crate::skip::is_true")]
        preview: bool,
        /// Layer Options → Dim Images to: on screen, images on the layer show faded to this
        /// percentage (0–100); `None`: not dimmed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dim_images: Option<u8>,
    },
    /// A group. With `clip`, the first (bottom-most) child is the clipping path.
    Group {
        children: Vec<Arc<Node>>,
        #[serde(default)]
        clip: bool,
    },
    Path {
        path: PathData,
        #[serde(default, skip_serializing_if = "crate::skip::is_default")]
        rule: FillRule,
        /// Boxed: few paths are live shapes, and the shape is larger than the rest of the variant.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        live: Option<Box<LiveShape>>,
        /// This path acts as a clipping path inside a clip group.
        #[serde(default, skip_serializing_if = "crate::skip::is_default")]
        clipping: bool,
        /// Guide (non-printing) path.
        #[serde(default, skip_serializing_if = "crate::skip::is_default")]
        guide: bool,
    },
    /// Compound path: children are paths painted as one with the compound's appearance.
    Compound {
        children: Vec<Arc<Node>>,
        #[serde(default)]
        rule: FillRule,
    },
    Text(Box<TextObject>),
    /// Boxed, as the other large kinds: a node is as large as its largest kind.
    Image(Box<ImageObject>),
    SymbolInstance {
        symbol: String,
        xf: Affine,
    },
    /// Live blend: the key objects (paint order) plus spacing/orientation/spine; the intermediate
    /// steps are evaluated on demand (`live::blend_expand`).
    Blend {
        children: Vec<Arc<Node>>,
        #[serde(default)]
        spec: Box<BlendSpec>,
    },
    /// Live envelope distortion of `content`.
    Envelope {
        content: Vec<Arc<Node>>,
        kind: Box<EnvelopeKind>,
        /// Envelope Options → Fidelity (0–100).
        #[serde(default = "crate::live::default_fidelity")]
        fidelity: f64,
        /// Edit Contents mode (the content, not the envelope, is edited).
        #[serde(default)]
        editing: bool,
        /// Envelope Options besides Fidelity.
        #[serde(default, skip_serializing_if = "crate::skip::is_default")]
        options: crate::live::EnvelopeOptions,
        /// The envelope's own axes (→ the document): the transforms it took since it was made that
        /// the page axes can't stand for (rotation, shear, reflection). The content maps from its
        /// bounds in this frame, and a warp bends along it.
        #[serde(default, skip_serializing_if = "crate::skip::is_default")]
        frame: Box<Affine>,
    },
    /// Gradient mesh.
    Mesh(GradientMesh),
    /// Live Repeat (radial / grid / mirror) of source art.
    Repeat(RepeatSpec),
    /// A placed document: an artboard of another VectorCraft file, or a page of a PDF or `.ai`
    /// file, linked and locked (see [`crate::placed_document`]).
    PlacedDocument(Box<crate::placed_document::PlacedDocument>),
    /// Object › Intertwine: the objects keep their own stacking order, except where one of them
    /// is drawn over the others: the area [`NodeExtras::intertwine`] of that member (see
    /// [`crate::intertwine`]).
    Intertwine {
        children: Vec<Arc<Node>>,
    },
}

fn yes() -> bool {
    true
}
fn one() -> f32 {
    1.0
}

/// One object in the tree. Children are `Arc`s so documents share structure between undo states.
///
/// What most objects have is kept inline; what few have is in [`NodeExtras`], allocated only for
/// the objects that have any of it: every undo step of an edit copies the nodes it changed, so a
/// small node is a cheap step. Files keep the flat field list either way: a node is written field
/// by field from where each lives, and read through a flat `NodeWire`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(from = "NodeWire")]
pub struct Node {
    pub id: NodeId,
    pub name: Option<String>,
    pub visible: bool,
    pub locked: bool,
    pub opacity: f32,
    pub blend: BlendMode,
    pub isolate: bool,
    /// Knockout Group: whether a container's children knock each other out (see [`Knockout`]).
    pub knockout: Knockout,
    /// Opacity & Mask Define Knockout Shape: inside a knockout group, this object's opacity and
    /// opacity mask scale how much of the objects below it in the group it knocks out.
    pub knockout_shape: bool,
    pub appearance: crate::Shared<Appearance>,
    pub kind: NodeKind,
    /// The angle of the object's own axes, counter-clockwise degrees (0: square to the page): its
    /// bounding box and handles stand at this angle. Transforms turn it with the object (see
    /// [`crate::orient`]); Reset Bounding Box sets it back to 0.
    pub bbox_angle: f64,
    /// The rarely set parts, read with [`Node::extras`] and changed with [`Node::extras_mut`]
    /// (public so that `Node { opacity: 1.0, ..node }` works).
    pub extras: Extras,
}

/// The parts of a [`Node`] that few objects have: kept out of line, so that the nodes without
/// them stay small ([`Node::extras`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeExtras {
    /// Opacity mask (Transparency panel). Its art lives here, outside the layer tree.
    pub mask: Option<Box<OpacityMask>>,
    /// Image Trace object: `{preset, params, view?}` it was traced with (the Image Trace panel shows
    /// them; `view` is its [`crate::TraceView`] id, absent for the tracing result).
    pub trace: Option<Box<serde_json::Value>>,
    /// Object → Text Wrap: area type below this object (in the same layer) flows around it.
    pub wrap: Option<crate::text::TextWrap>,
    /// Graph object: the group's children are generated from this spec (Object → Graph).
    pub graph: Option<Box<crate::graph::GraphSpec>>,
    /// Index of the graph series this generated group belongs to. Axes, the legend and other
    /// children leave it unset, so a series named "Legend" or "Axes" is not mistaken for them.
    pub series_index: Option<u32>,
    /// The group holds the glyph outlines of type. Geometry effects that reshape it move and scale
    /// the placed gradients in it with the outlines.
    pub type_outlines: bool,
    /// Editable Shaper composition; the original art is retained in its source child.
    pub shaper: Option<Box<crate::shaper::ShaperSpec>>,
    /// The [`crate::GraphicStyle::id`] last applied to this object. It stays linked while it keeps
    /// that style's look: editing its appearance or transparency breaks the link.
    pub graphic_style: Option<u32>,
    /// Attributes panel: centre point display, image map, URL and note (`None`: all defaults).
    pub attrs: Option<Box<ObjectAttributes>>,
    /// Object → Slice → Make: the object is an object slice (its slice follows its bounds) with
    /// these options.
    pub slice: Option<Box<crate::SliceOptions>>,
    /// Object › Perspective: the perspective grid plane the object is attached to.
    pub perspective: Option<Box<crate::PerspectiveAttachment>>,
    /// Dimension object: the live measurement it draws; its ordinary group children hold its
    /// exportable appearance (Object › Dimensions, the Dimension tool).
    pub dimension: Option<Box<crate::dimension::Dimension>>,
    /// Object › Intertwine: as a member of an Intertwine group, the area (document coordinates,
    /// filled non-zero) where this object is drawn over the group's other members.
    pub intertwine: Option<Box<PathData>>,
    /// Live Paint group: its Gap Options (Object › Live Paint › Gap Options; `None`: the defaults).
    pub live_paint_gaps: Option<Box<crate::live_paint::GapOptions>>,
    /// Object › Objects on Path: the group's path and the objects attached to it (see
    /// [`crate::objects_on_path`]).
    pub objects_on_path: Option<Box<crate::objects_on_path::ObjectsOnPath>>,
}

/// No extras: what [`Node::extras`] gives for a node without any.
static NO_EXTRAS: NodeExtras = NodeExtras {
    mask: None,
    trace: None,
    wrap: None,
    graph: None,
    series_index: None,
    type_outlines: false,
    shaper: None,
    graphic_style: None,
    attrs: None,
    slice: None,
    perspective: None,
    dimension: None,
    intertwine: None,
    live_paint_gaps: None,
    objects_on_path: None,
};

/// A node's [`NodeExtras`], if it has any ([`Node::extras`]). Extras that are all unset equal
/// none.
#[derive(Clone, Debug, Default)]
pub struct Extras(Option<Box<NodeExtras>>);

impl Extras {
    fn get(&self) -> &NodeExtras {
        self.0.as_deref().unwrap_or(&NO_EXTRAS)
    }
}

impl PartialEq for Extras {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => a == b,
            _ => self.get() == other.get(),
        }
    }
}

impl Node {
    /// The rarely set parts of this node (all unset when it has none).
    #[inline]
    pub fn extras(&self) -> &NodeExtras {
        self.extras.get()
    }
    /// The rarely set parts of this node, to change: when the change is done (the guard dropped),
    /// extras that were set are allocated and extras left all unset are let go. Setting one to
    /// `None` on a node without extras allocates nothing.
    #[inline]
    pub fn extras_mut(&mut self) -> ExtrasMut<'_> {
        ExtrasMut { slot: &mut self.extras.0, local: NodeExtras::default() }
    }
    /// The rarely set parts of this node, if it has any: to change what is there in place.
    #[inline]
    pub fn extras_if_any(&mut self) -> Option<&mut NodeExtras> {
        self.extras.0.as_deref_mut()
    }
    /// The live shape of a path object, if it is one.
    pub fn live_shape(&self) -> Option<&LiveShape> {
        match &self.kind {
            NodeKind::Path { live, .. } => live.as_deref(),
            _ => None,
        }
    }
    /// The live shape of a path object, if it is one, to change.
    pub fn live_shape_mut(&mut self) -> Option<&mut LiveShape> {
        match &mut self.kind {
            NodeKind::Path { live, .. } => live.as_deref_mut(),
            _ => None,
        }
    }
    /// This node with opacity mask `mask` (or none).
    pub fn with_mask(mut self, mask: Option<Box<OpacityMask>>) -> Self {
        self.extras_mut().mask = mask;
        self
    }
    /// The extras, if this node has them allocated (for [`crate::mem`]'s estimate).
    pub(crate) fn extras_box(&self) -> Option<&NodeExtras> {
        self.extras.0.as_deref()
    }
}

/// [`Node::extras_mut`]: the node's extras while they change.
pub struct ExtrasMut<'a> {
    slot: &'a mut Option<Box<NodeExtras>>,
    /// Stands for the extras of a node without any, boxed into it when dropped if one was set.
    local: NodeExtras,
}

impl std::ops::Deref for ExtrasMut<'_> {
    type Target = NodeExtras;
    fn deref(&self) -> &NodeExtras {
        self.slot.as_deref().unwrap_or(&self.local)
    }
}

impl std::ops::DerefMut for ExtrasMut<'_> {
    fn deref_mut(&mut self) -> &mut NodeExtras {
        match self.slot.as_deref_mut() {
            Some(e) => e,
            None => &mut self.local,
        }
    }
}

impl Drop for ExtrasMut<'_> {
    fn drop(&mut self) {
        match self.slot.as_deref() {
            Some(e) if *e == NO_EXTRAS => *self.slot = None,
            Some(_) => {}
            None if self.local != NO_EXTRAS => *self.slot = Some(Box::new(std::mem::take(&mut self.local))),
            None => {}
        }
    }
}

/// How a [`Node`] is written: the flat field list of the file format, in its order, borrowed from
/// the node and its extras (nothing is copied). The node reads back through `NodeWire`.
impl Serialize for Node {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use crate::skip::{is_default, is_one, is_true};
        use serde::ser::SerializeStruct;
        let e = self.extras();
        // As the derive did: the fields written, then each field written or skipped in order.
        let skips = [
            false,
            self.name.is_none(),
            is_true(&self.visible),
            is_default(&self.locked),
            is_one(&self.opacity),
            is_default(&self.blend),
            is_default(&self.isolate),
            is_default(&self.knockout),
            is_default(&self.knockout_shape),
            is_default(&self.appearance),
            e.mask.is_none(),
            e.trace.is_none(),
            e.wrap.is_none(),
            e.graph.is_none(),
            e.series_index.is_none(),
            is_default(&e.type_outlines),
            e.shaper.is_none(),
            false,
            e.graphic_style.is_none(),
            e.attrs.is_none(),
            e.slice.is_none(),
            is_default(&self.bbox_angle),
            e.perspective.is_none(),
            e.dimension.is_none(),
            e.intertwine.is_none(),
            e.live_paint_gaps.is_none(),
            e.objects_on_path.is_none(),
        ];
        let mut st = s.serialize_struct("Node", skips.iter().filter(|s| !**s).count())?;
        let mut skip = skips.into_iter();
        macro_rules! field {
            ($name:literal, $v:expr) => {
                if skip.next().unwrap_or(false) {
                    st.skip_field($name)?;
                } else {
                    st.serialize_field($name, $v)?;
                }
            };
        }
        field!("id", &self.id);
        field!("name", &self.name);
        field!("visible", &self.visible);
        field!("locked", &self.locked);
        field!("opacity", &self.opacity);
        field!("blend", &self.blend);
        field!("isolate", &self.isolate);
        field!("knockout", &self.knockout);
        field!("knockout_shape", &self.knockout_shape);
        field!("appearance", &self.appearance);
        field!("mask", &e.mask);
        field!("trace", &e.trace);
        field!("wrap", &e.wrap);
        field!("graph", &e.graph);
        field!("series_index", &e.series_index);
        field!("type_outlines", &e.type_outlines);
        field!("shaper", &e.shaper);
        field!("kind", &self.kind);
        field!("graphic_style", &e.graphic_style);
        field!("attrs", &e.attrs);
        field!("slice", &e.slice);
        field!("bbox_angle", &self.bbox_angle);
        field!("perspective", &e.perspective);
        field!("dimension", &e.dimension);
        field!("intertwine", &e.intertwine);
        field!("live_paint_gaps", &e.live_paint_gaps);
        field!("objects_on_path", &e.objects_on_path);
        st.end()
    }
}

/// How a [`Node`] is read: the flat field list of the file format. Fields move into the node
/// (and its extras, allocated only when one is set) without a copy of what they own.
#[derive(Deserialize)]
#[serde(rename = "Node")]
struct NodeWire {
    id: NodeId,
    #[serde(default)]
    name: Option<String>,
    #[serde(default = "yes")]
    visible: bool,
    #[serde(default)]
    locked: bool,
    #[serde(default = "one")]
    opacity: f32,
    #[serde(default)]
    blend: BlendMode,
    #[serde(default)]
    isolate: bool,
    #[serde(default)]
    knockout: Knockout,
    #[serde(default)]
    knockout_shape: bool,
    #[serde(default)]
    appearance: crate::Shared<Appearance>,
    #[serde(default)]
    mask: Option<Box<OpacityMask>>,
    #[serde(default)]
    trace: Option<Box<serde_json::Value>>,
    #[serde(default)]
    wrap: Option<crate::text::TextWrap>,
    #[serde(default)]
    graph: Option<Box<crate::graph::GraphSpec>>,
    #[serde(default)]
    series_index: Option<u32>,
    #[serde(default)]
    type_outlines: bool,
    #[serde(default)]
    shaper: Option<Box<crate::shaper::ShaperSpec>>,
    kind: NodeKind,
    #[serde(default)]
    graphic_style: Option<u32>,
    #[serde(default)]
    attrs: Option<Box<ObjectAttributes>>,
    #[serde(default)]
    slice: Option<Box<crate::SliceOptions>>,
    #[serde(default)]
    bbox_angle: f64,
    #[serde(default)]
    perspective: Option<Box<crate::PerspectiveAttachment>>,
    #[serde(default)]
    dimension: Option<Box<crate::dimension::Dimension>>,
    #[serde(default)]
    intertwine: Option<Box<PathData>>,
    #[serde(default)]
    live_paint_gaps: Option<Box<crate::live_paint::GapOptions>>,
    #[serde(default)]
    objects_on_path: Option<Box<crate::objects_on_path::ObjectsOnPath>>,
}

impl From<NodeWire> for Node {
    fn from(w: NodeWire) -> Self {
        let extras = NodeExtras {
            mask: w.mask,
            trace: w.trace,
            wrap: w.wrap,
            graph: w.graph,
            series_index: w.series_index,
            type_outlines: w.type_outlines,
            shaper: w.shaper,
            graphic_style: w.graphic_style,
            attrs: w.attrs,
            slice: w.slice,
            perspective: w.perspective,
            dimension: w.dimension,
            intertwine: w.intertwine,
            live_paint_gaps: w.live_paint_gaps,
            objects_on_path: w.objects_on_path,
        };
        Node {
            id: w.id,
            name: w.name,
            visible: w.visible,
            locked: w.locked,
            opacity: w.opacity,
            blend: w.blend,
            isolate: w.isolate,
            knockout: w.knockout,
            knockout_shape: w.knockout_shape,
            appearance: w.appearance,
            kind: w.kind,
            bbox_angle: w.bbox_angle,
            extras: Extras((extras != NO_EXTRAS).then(|| Box::new(extras))),
        }
    }
}

/// Opacity mask: the luminance of the mask art sets the object's opacity (white = opaque).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpacityMask {
    /// Mask art in document coordinates.
    pub art: Arc<Node>,
    /// Clip: outside the mask art is hidden. Off: outside the mask art stays visible.
    #[serde(default = "yes")]
    pub clip: bool,
    /// Invert the mask's luminance.
    #[serde(default)]
    pub invert: bool,
    /// Disabled masks are kept but not applied.
    #[serde(default)]
    pub disabled: bool,
    /// Linked masks move with the object.
    #[serde(default = "yes")]
    pub linked: bool,
}

impl OpacityMask {
    pub fn new(art: Node, clip: bool) -> Self {
        Self { art: Arc::new(art), clip, invert: false, disabled: false, linked: true }
    }
}

/// Every paint of `n` (and its children) white, opacities kept: an alpha mask's art as an opacity
/// mask's, whose luminance over the black outside the art is then the art's alpha.
pub fn whiten(n: &mut Node) {
    let white = |p: &mut Paint| match p {
        Paint::Solid { color, swatch, tint } => {
            (*color, *swatch, *tint) = (Color::WHITE, None, 1.0);
        }
        Paint::Gradient(g) => {
            for s in &mut g.gradient.stops {
                (s.color, s.swatch, s.tint) = (Color::WHITE, None, 1.0);
            }
        }
        Paint::None | Paint::Pattern { .. } => {}
    };
    for item in &mut n.appearance.items {
        white(item.paint_mut());
    }
    match &mut n.kind {
        NodeKind::Mesh(m) => m.points.iter_mut().for_each(|p| p.color = Color::WHITE),
        NodeKind::Text(t) => t.runs.iter_mut().for_each(|r| {
            white(&mut r.style.fill);
            white(&mut r.style.stroke);
        }),
        _ => {}
    }
    if let Some(ch) = n.children_mut() {
        for c in ch {
            whiten(Arc::make_mut(c));
        }
    }
}

impl Node {
    pub fn new(id: NodeId, kind: NodeKind) -> Self {
        Self {
            id,
            name: None,
            visible: true,
            locked: false,
            opacity: 1.0,
            blend: BlendMode::Normal,
            isolate: false,
            knockout: Knockout::Neutral,
            knockout_shape: false,
            appearance: Appearance::default().into(),
            kind,
            bbox_angle: 0.0,
            extras: Extras::default(),
        }
    }
    pub fn path(id: NodeId, path: PathData, appearance: Appearance) -> Self {
        let mut n = Self::new(id, NodeKind::Path { path, rule: FillRule::NonZero, live: None, clipping: false, guide: false });
        n.appearance = appearance.into();
        n
    }
    pub fn group(id: NodeId, children: Vec<Arc<Node>>) -> Self {
        Self::new(id, NodeKind::Group { children, clip: false })
    }
    pub fn layer(id: NodeId, name: &str, color: LayerColor) -> Self {
        let mut n = Self::new(
            id,
            NodeKind::Layer { color, template: false, printable: true, children: vec![], clip: false, preview: true, dim_images: None },
        );
        n.name = Some(name.to_string());
        n
    }
    pub fn children(&self) -> Option<&Vec<Arc<Node>>> {
        match &self.kind {
            NodeKind::Layer { children, .. }
            | NodeKind::Group { children, .. }
            | NodeKind::Compound { children, .. }
            | NodeKind::Blend { children, .. }
            | NodeKind::Envelope { content: children, .. }
            | NodeKind::Repeat(RepeatSpec { source: children, .. })
            | NodeKind::Intertwine { children } => Some(children),
            _ => None,
        }
    }
    pub fn children_mut(&mut self) -> Option<&mut Vec<Arc<Node>>> {
        match &mut self.kind {
            NodeKind::Layer { children, .. }
            | NodeKind::Group { children, .. }
            | NodeKind::Compound { children, .. }
            | NodeKind::Blend { children, .. }
            | NodeKind::Envelope { content: children, .. }
            | NodeKind::Repeat(RepeatSpec { source: children, .. })
            | NodeKind::Intertwine { children } => Some(children),
            _ => None,
        }
    }
    /// Does this object or layer move with its artboard? Locked and hidden ones only with
    /// `locked_and_hidden` (Selection & Anchor Display › Move Locked and Hidden Artwork with
    /// Artboard).
    pub fn rides_with_artboard(&self, locked_and_hidden: bool) -> bool {
        locked_and_hidden || (!self.locked && self.visible)
    }
    pub fn is_layer(&self) -> bool {
        matches!(self.kind, NodeKind::Layer { .. })
    }
    /// A template layer (Layer Options → Template).
    pub fn is_template(&self) -> bool {
        matches!(self.kind, NodeKind::Layer { template: true, .. })
    }
    /// The art objects of this container: its children, looking through sublayers, which are not
    /// objects (bottom first). With `editable`, only the visible, unlocked objects in visible,
    /// unlocked, non-template sublayers.
    pub fn layer_art(&self, editable: bool) -> Vec<NodeId> {
        fn visit(n: &Node, editable: bool, out: &mut Vec<NodeId>) {
            for c in n.children().into_iter().flatten() {
                if editable && (!c.visible || c.locked || c.is_template()) {
                    continue;
                }
                if c.is_layer() {
                    visit(c, editable, out);
                } else {
                    out.push(c.id);
                }
            }
        }
        let mut out = vec![];
        visit(self, editable, &mut out);
        out
    }
    pub fn is_container(&self) -> bool {
        self.children().is_some()
    }
    /// Full opacity, Normal blending, no isolation, knockout or opacity mask (the Layers panel
    /// fills an object's target circle otherwise).
    pub fn has_default_transparency(&self) -> bool {
        self.opacity >= 1.0
            && self.blend == BlendMode::Normal
            && !self.isolate
            && self.knockout == Knockout::Neutral
            && !self.knockout_shape
            && self.extras().mask.is_none()
    }
    /// Kind name as the Layers panel / Properties panel shows it.
    pub fn kind_label(&self) -> &'static str {
        match &self.kind {
            NodeKind::Layer { .. } => "Layer",
            NodeKind::Group { clip: true, .. } => "Clip Group",
            NodeKind::Group { .. } if self.extras().objects_on_path.is_some() => "Objects on Path",
            NodeKind::Group { .. } => "Group",
            NodeKind::Path { live: Some(l), .. } => l.label(),
            NodeKind::Path { guide: true, .. } => "Guide",
            NodeKind::Path { .. } => "Path",
            NodeKind::Compound { .. } => "Compound Path",
            NodeKind::Text(_) => "Type",
            NodeKind::Image(_) => "Image",
            NodeKind::SymbolInstance { .. } => "Symbol",
            NodeKind::Blend { .. } => "Blend",
            NodeKind::Envelope { .. } => "Envelope",
            NodeKind::Mesh(_) => "Mesh",
            NodeKind::Repeat(r) => r.kind.label(),
            NodeKind::PlacedDocument(_) => "Placed Document",
            NodeKind::Intertwine { .. } => "Intertwine",
        }
    }
    /// Name shown in the Layers panel: explicit name or `<Kind>`.
    pub fn display_name(&self) -> String {
        if let Some(n) = &self.name {
            return n.clone();
        }
        match &self.kind {
            NodeKind::Text(t) => {
                let s: String = t.plain_text().chars().take(32).collect();
                if s.is_empty() { "<Text>".into() } else { s }
            }
            _ => format!("<{}>", self.kind_label()),
        }
    }

    /// [`Self::display_name`], or its XML id when Preferences › Units › Identify Objects By is XML ID.
    pub fn identify_label(&self, xml_id: bool) -> String {
        let name = self.display_name();
        if xml_id { crate::xml_id::from_object_name(&name) } else { name }
    }
    /// Path data for path nodes.
    pub fn path_data(&self) -> Option<&PathData> {
        match &self.kind {
            NodeKind::Path { path, .. } => Some(path),
            _ => None,
        }
    }
    pub fn path_data_mut(&mut self) -> Option<&mut PathData> {
        match &mut self.kind {
            NodeKind::Path { path, .. } => Some(path),
            _ => None,
        }
    }
    /// The path a path's or compound path's own strokes follow (a compound's members' together).
    pub fn stroke_path(&self) -> Option<vectorcraft_geom::BezPath> {
        match &self.kind {
            NodeKind::Path { path, .. } => Some(path.to_bezpath()),
            NodeKind::Compound { children, .. } => {
                let mut bp = vectorcraft_geom::BezPath::new();
                children.iter().filter_map(|c| c.path_data()).for_each(|pd| bp.extend(pd.to_bezpath()));
                Some(bp)
            }
            _ => None,
        }
    }
    /// A quick box round everything the object can paint, for culling clicks: paths and compound
    /// paths grow their geometric bounds by the farthest their strokes can reach from any path
    /// ([`Appearance::outset`]) instead of measuring along their shape. An image's box takes in
    /// the shapes its own fills and strokes paint ([`crate::hit::set_image_paint`]). Groups, layers
    /// without a clipping mask and Intertwine groups take in their members' boxes, grown by their
    /// own strokes. Symbol instances are measured as in [`Node::visual_bounds`].
    pub fn reach_bounds(&self) -> Option<Rect> {
        self.reach(None)
    }
    /// [`Node::reach_bounds`] with symbol instances measured by their symbol's art box from
    /// `symbols`. An instance that only moves its art is measured as the art's visual box moved,
    /// and any other instance is measured as its geometric box ([`Node::geometric_bounds_with`])
    /// grown by the farthest a stroke in the art can reach.
    pub fn reach_bounds_with(&self, symbols: &SymbolBoxes) -> Option<Rect> {
        symbols.quickly(|| self.reach(Some(symbols)))
    }
    /// The box of [`Node::reach_bounds`], or of [`Node::reach_bounds_with`] when `symbols` is given.
    pub(crate) fn reach(&self, symbols: Option<&SymbolBoxes>) -> Option<Rect> {
        match &self.kind {
            NodeKind::Path { .. } | NodeKind::Compound { .. } => {
                let o = self.appearance.outset();
                self.geometric(symbols).map(|b| b.inflate(o, o))
            }
            // The members' reach, grown by the container's own strokes.
            NodeKind::Layer { children, clip: false, .. } | NodeKind::Group { children, clip: false } => {
                let o = self.appearance.outset();
                children
                    .iter()
                    .skip(usize::from(self.extras().shaper.is_some()))
                    .fold(None, |acc, c| vectorcraft_geom::union_opt(acc, c.reach(symbols)))
                    .map(|b| b.inflate(o, o))
            }
            NodeKind::Intertwine { children } => children.iter().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, c.reach(symbols))),
            // The frame and each shape the image's own fills and strokes paint, a stroke's grown by
            // the farthest it can reach from its shape.
            NodeKind::Image(_) => crate::hit::image_paint(self).iter().fold(self.geometric(symbols), |acc, (bp, _, item)| {
                let o = if let crate::AppearanceItem::Stroke(s) = item { s.reach() } else { 0.0 };
                vectorcraft_geom::union_opt(acc, (!bp.elements().is_empty()).then(|| bp.bounding_box().inflate(o, o)))
            }),
            _ => self.visual(symbols),
        }
    }
    /// Geometric bounds (no stroke), recursively. Clip groups are bounded by their clip path. A
    /// symbol instance is measured as its transform applied to the 20 × 20 square around the origin
    /// that symbols made in the app keep their art in ([`Node::geometric_bounds_with`] measures
    /// its art).
    pub fn geometric_bounds(&self) -> Option<Rect> {
        self.geometric(None)
    }
    /// [`Node::geometric_bounds`] with each symbol instance measured as its transform applied to
    /// its symbol's art box from `symbols`. An instance is measured as the square when its symbol
    /// is missing, when its symbol or a symbol nested in its art holds an instance of itself, and
    /// when symbols nest 32 deep in its art.
    pub fn geometric_bounds_with(&self, symbols: &SymbolBoxes) -> Option<Rect> {
        self.geometric(Some(symbols))
    }
    /// Children that contribute to a container's editing bounds and selection highlights. A live trace retains a
    /// hidden bitmap for re-tracing/release; only source-image views measure that bitmap.
    pub fn editing_children(&self) -> &[Arc<Node>] {
        let children = self.children().map(Vec::as_slice).unwrap_or_default();
        let trace_source = self.extras().trace.is_some()
            && !self.trace_view().shows_source()
            && children.first().is_some_and(|n| !n.visible && matches!(n.kind, NodeKind::Image(_)));
        let skip = usize::from(self.extras().shaper.is_some() || trace_source);
        children.get(skip..).unwrap_or_default()
    }

    /// The box of [`Node::geometric_bounds`], or of [`Node::geometric_bounds_with`] when `symbols`
    /// is given.
    pub(crate) fn geometric(&self, symbols: Option<&SymbolBoxes>) -> Option<Rect> {
        let union = |nodes: &[Arc<Node>]| nodes.iter().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, c.geometric(symbols)));
        match &self.kind {
            NodeKind::Path { path, .. } => path.bounds(),
            NodeKind::Group { children, clip: true } | NodeKind::Layer { children, clip: true, .. } => {
                children.first().and_then(|c| c.geometric(symbols))
            }
            NodeKind::Layer { .. } | NodeKind::Group { .. } | NodeKind::Compound { .. } => self
                .editing_children()
                .iter()
                .filter(|c| c.visible || !matches!(self.kind, NodeKind::Layer { .. }))
                .fold(None, |acc, c| vectorcraft_geom::union_opt(acc, c.geometric(symbols))),
            NodeKind::Text(t) => self.projected(t.bounds()),
            NodeKind::Image(im) => Some(im.xf.transform_rect_bbox(Rect::new(0.0, 0.0, im.width as f64, im.height as f64))),
            NodeKind::SymbolInstance { symbol, xf } => {
                let art = symbols.and_then(|s| s.art(symbol)).map_or(SYMBOL_SQUARE, |a| a.geometric);
                self.projected(Some(xf.transform_rect_bbox(art)))
            }
            NodeKind::Blend { children, spec } => vectorcraft_geom::union_opt(union(children), spec.spine.as_ref().and_then(|s| s.bounds())),
            NodeKind::Envelope { content, kind, frame, .. } => crate::live::envelope_bounds(content, kind, **frame),
            NodeKind::Mesh(m) => m.bounds(),
            // Each copy is measured where the repeat draws it: `RepeatSpec::transforms` lays the
            // copies out by the source's bounds measured without the document.
            NodeKind::Repeat(r) => r.copies_bounds(union(&r.source)?, symbols.map(|s| s.document())),
            NodeKind::PlacedDocument(p) => Some(p.bounds()),
            // The members drawn over the others are drawn where they are.
            NodeKind::Intertwine { children } => union(children),
        }
    }
    /// Visual bounds: what the object paints, its strokes included (paths and compound paths
    /// measure theirs along their own shape, see [`Appearance::stroked_bounds`]). An image is
    /// measured by its frame. A symbol instance is measured as in [`Node::geometric_bounds`],
    /// grown by its own strokes' reach ([`Node::visual_bounds_with`] measures its art).
    pub fn visual_bounds(&self) -> Option<Rect> {
        self.visual(None)
    }
    /// [`Node::visual_bounds`] with symbol instances measured by their symbol's art from
    /// `symbols`, the strokes in it included. An instance that only moves its art is measured as
    /// the art's visual box moved. Any other instance is measured as its art transformed by it with
    /// the strokes at their own weight, as the renderer draws it, and never smaller than its
    /// geometric box ([`Node::geometric_bounds_with`]). Instances inside symbol art are measured as
    /// in [`Node::reach_bounds_with`]. In boxes made by [`SymbolBoxes::with_brush_art`], the box of
    /// an instance whose art holds brushed strokes grows by as far as their brush art can paint. In
    /// boxes made by [`SymbolBoxes::without_art_strokes`], an instance is measured as its art's
    /// geometric box placed by its transform, grown by its own strokes.
    pub fn visual_bounds_with(&self, symbols: &SymbolBoxes) -> Option<Rect> {
        self.visual(Some(symbols))
    }
    /// The box of [`Node::visual_bounds`], or of [`Node::visual_bounds_with`] when `symbols` is
    /// given.
    pub(crate) fn visual(&self, symbols: Option<&SymbolBoxes>) -> Option<Rect> {
        match &self.kind {
            NodeKind::Group { children, clip: true } | NodeKind::Layer { children, clip: true, .. } => {
                // A clipping path that paints (see `clip_paint`) strokes over the clip edge.
                let clip = children.first()?;
                if matches!(clip.kind, NodeKind::Path { .. } | NodeKind::Compound { .. } | NodeKind::Text(_)) {
                    clip.visual(symbols)
                } else {
                    clip.geometric(symbols)
                }
            }
            NodeKind::Intertwine { children } => children.iter().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, c.visual(symbols))),
            NodeKind::Layer { .. } | NodeKind::Group { .. } => {
                // The container's own strokes paint around its members.
                let o = self.appearance.outset();
                self.editing_children().iter().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, c.visual(symbols))).map(|b| b.inflate(o, o))
            }
            NodeKind::Blend { children, spec } => {
                let b = children.iter().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, c.visual(symbols)));
                let o = crate::live::max_outset(children, symbols);
                vectorcraft_geom::union_opt(b, spec.spine.as_ref().and_then(|s| s.bounds()).map(|r| r.inflate(o, o)))
            }
            NodeKind::Envelope { content, .. } | NodeKind::Repeat(RepeatSpec { source: content, .. }) => {
                let o = crate::live::max_outset(content, symbols);
                self.geometric(symbols).map(|b| b.inflate(o, o))
            }
            NodeKind::Path { .. } | NodeKind::Compound { .. } => {
                let b = self.geometric(symbols)?;
                Some(self.appearance.stroked_bounds(b, || self.stroke_path().unwrap_or_default()))
            }
            NodeKind::SymbolInstance { symbol, xf } => {
                let b = match symbols.and_then(|s| Some((s, s.art(symbol)?))) {
                    Some((s, art)) if !s.art_strokes() => xf.transform_rect_bbox(art.geometric),
                    Some((_, art)) if moves_only(*xf) => xf.transform_rect_bbox(art.visual),
                    Some((s, art)) => {
                        let placed = xf.transform_rect_bbox(art.geometric);
                        match s.placed_visual(symbol, *xf) {
                            Some(v) => v.union(placed),
                            None => placed.inflate(art.reach, art.reach),
                        }
                    }
                    None => xf.transform_rect_bbox(SYMBOL_SQUARE),
                };
                let o = self.appearance.outset() + symbols.map_or(0.0, |s| s.brush_pad(symbol));
                self.projected(Some(b)).map(|b| b.inflate(o, o))
            }
            NodeKind::Text(t) => {
                // The characters' strokes paint around the glyphs.
                let o = self.appearance.outset().max(t.char_stroke_outset());
                self.geometric(symbols).map(|b| b.inflate(o, o))
            }
            // An image is measured by its frame, without its own fills and strokes.
            NodeKind::Image(_) => self.geometric(symbols),
            _ => {
                let o = self.appearance.outset();
                self.geometric(symbols).map(|b| b.inflate(o, o))
            }
        }
    }
    /// Apply an affine transform to the geometry (and gradients) of this node and its descendants.
    /// `scaling` says what else scales by the transform's mean scale (a bool: stroke weights
    /// only, see [`Scaling`]).
    pub fn transform(&mut self, a: Affine, scaling: impl Into<Scaling>) {
        self.transform_scaled(a, &scaling.into());
    }
    fn transform_scaled(&mut self, a: Affine, sc: &Scaling) {
        // A dimension's measurement moves with it.
        if let Some(e) = self.extras_if_any()
            && let Some(dimension) = &mut e.dimension
        {
            dimension.transform(a);
        }
        // The frames of the objects attached to a path move with them.
        if let Some(e) = self.extras_if_any()
            && let Some(on_path) = &mut e.objects_on_path
        {
            on_path.transform(a);
        }
        if !self.is_layer() {
            self.bbox_angle = crate::orient::transformed_angle(self.bbox_angle, a);
        }
        // Type and symbols in perspective keep looking the same, moved by `a`.
        self.transform_projection(a);
        // Refitting an unplaced gradient only reproduces moves and uniform scales.
        if !keeps_gradient_fit(a) {
            self.pin_gradients();
        }
        let k = Scaling::factor(a);
        if let Some(k) = k
            && sc.strokes
        {
            self.appearance.scale_strokes(k);
            if let Some(f) = sc.effects {
                self.appearance.scale_effects(k, f);
            }
        }
        // Only through `DerefMut` when there is something to change: the appearance is shared
        // copy-on-write (`Shared`), and a move must not copy what it leaves as it is.
        if self.appearance.gradients().next().is_some() {
            self.appearance.transform_gradients(a);
        }
        if sc.patterns {
            crate::pattern::transform_pattern_paints(self, a);
        }
        match &mut self.kind {
            NodeKind::Path { path, live, .. } => {
                path.transform(a);
                if let Some(l) = live {
                    // A rectangle scaled unevenly keeps circular corners: its path is regenerated.
                    let reshaped = l.transform(a);
                    let kept = sc.keep_corners && k.is_some_and(|k| l.keep_corners(k));
                    if reshaped || kept {
                        *path = l.to_path();
                    }
                }
            }
            NodeKind::Layer { children, .. }
            | NodeKind::Group { children, .. }
            | NodeKind::Compound { children, .. }
            | NodeKind::Intertwine { children } => {
                for c in children.iter_mut() {
                    Arc::make_mut(c).transform_scaled(a, sc);
                }
            }
            NodeKind::Text(t) => {
                t.transform(a);
                // Character strokes scale with the type: undo that when strokes keep their weight.
                if sc.keep_type_strokes
                    && let Some(k) = k.filter(|k| *k > 1e-12)
                {
                    t.scale_char_strokes(1.0 / k);
                }
            }
            NodeKind::Image(im) => im.xf = a * im.xf,
            NodeKind::PlacedDocument(p) => p.xf = a * p.xf,
            NodeKind::SymbolInstance { xf, .. } => *xf = a * *xf,
            NodeKind::Blend { children, spec } => {
                for c in children.iter_mut() {
                    Arc::make_mut(c).transform_scaled(a, sc);
                }
                if let Some(s) = &mut spec.spine {
                    s.transform(a);
                }
            }
            NodeKind::Envelope { content, kind, frame, .. } => {
                for c in content.iter_mut() {
                    Arc::make_mut(c).transform_scaled(a, sc);
                }
                **frame = crate::live::envelope_frame(a * **frame);
                match &mut **kind {
                    EnvelopeKind::Mesh { points, handles, .. } => {
                        for p in points.iter_mut() {
                            *p = a * *p;
                        }
                        // Handles are offsets: they take the linear part.
                        let [m0, m1, m2, m3, _, _] = a.as_coeffs();
                        let lin = Affine::new([m0, m1, m2, m3, 0.0, 0.0]);
                        for h in handles.iter_mut().flatten() {
                            *h = (lin * h.to_point()).to_vec2();
                        }
                    }
                    EnvelopeKind::TopObject { path } => path.transform(a),
                    EnvelopeKind::Warp { .. } => {}
                }
            }
            NodeKind::Mesh(m) => m.transform(a),
            NodeKind::Repeat(r) => r.transform(a, *sc),
        }
        if let Some(m) = self.extras_if_any().and_then(|e| e.mask.as_mut())
            && m.linked
        {
            Arc::make_mut(&mut m.art).transform_scaled(a, sc);
        }
        // The area where an Intertwine member is drawn over the others moves with it.
        if let Some(area) = self.extras_if_any().and_then(|e| e.intertwine.as_mut()) {
            area.transform(a);
        }
    }
    /// Visit this node and all descendants depth first (paint order).
    pub fn walk<'a>(&'a self, f: &mut impl FnMut(&'a Node)) {
        f(self);
        if let Some(ch) = self.children() {
            for c in ch {
                c.walk(f);
            }
        }
    }
    /// Count of nodes in this subtree.
    pub fn count(&self) -> usize {
        let mut n = 0;
        self.walk(&mut |_| n += 1);
        n
    }
    /// [`Self::count`], with the art of the opacity masks in the subtree (and theirs): what copying
    /// and transforming the subtree makes, as [`Self::walk`] doesn't go into masks.
    pub fn count_with_masks(&self) -> usize {
        let mut n = 0;
        self.walk(&mut |c| {
            n += 1;
            if let Some(m) = &c.extras().mask {
                n += m.art.count_with_masks();
            }
        });
        n
    }
    /// Fix this node's unplaced gradients to their fit on its current bounds (before a transform
    /// or warp that refitting wouldn't reproduce). Descendants pin their own when transformed.
    pub fn pin_gradients(&mut self) {
        if self.appearance.has_unplaced_gradient()
            && let Some(b) = self.geometric_bounds()
        {
            self.appearance.pin_gradients(b);
        }
    }
    /// The paint behind the Fill (or Stroke) proxy, the map from its space to the document and
    /// the box (in that space) an unplaced gradient fits. `item` (the Appearance panel's active
    /// item) stands in for the topmost fill or stroke when it is of the proxy's kind. Otherwise
    /// type objects paint their runs (the first run speaks for all) in text space, fitted to the
    /// layout bounds; everything else paints its fill or stroke in the document, fitted to the
    /// geometric bounds. Strokes fit the box grown by half their weight.
    pub fn proxy_paint(&self, stroke: bool, item: Option<usize>) -> Option<(&vectorcraft_color::Paint, Affine, Rect)> {
        let item = self.appearance.item_of_kind(item, !stroke);
        if item.is_none()
            && let NodeKind::Text(t) = &self.kind
        {
            let st = &t.runs.first()?.style;
            let b = t.local_bounds();
            return Some(if stroke { (&st.stroke, t.xf, crate::appearance::stroke_paint_bounds(b, st.stroke_width)) } else { (&st.fill, t.xf, b) });
        }
        // An object without bounds (an empty path) still shows its paints, fitted to an empty box.
        let b = self.geometric_bounds().unwrap_or(Rect::ZERO);
        if stroke {
            self.appearance.stroke_at(item).map(|s| (&s.paint, Affine::IDENTITY, s.paint_bounds(b)))
        } else {
            self.appearance.fill_at(item).map(|f| (&f.paint, Affine::IDENTITY, b))
        }
    }
    /// The gradient behind the Fill (or Stroke) proxy and its placement in document coordinates
    /// (see [`Node::proxy_paint`]): what the gradient annotator shows and edits.
    pub fn proxy_gradient(&self, stroke: bool, item: Option<usize>) -> Option<(&vectorcraft_color::GradientPaint, vectorcraft_color::GradientGeom)> {
        let (paint, to_doc, b) = self.proxy_paint(stroke, item)?;
        let vectorcraft_color::Paint::Gradient(g) = paint else { return None };
        let mut geom = g.resolve(b);
        if to_doc != Affine::IDENTITY {
            geom.transform(to_doc, g.gradient.kind);
        }
        Some((g, geom))
    }
    /// The freeform gradient behind the Fill (or Stroke) proxy, its points in document coordinates
    /// (the automatic ones while unplaced), and the length its spreads are fractions of there
    /// (see [`Node::proxy_paint`]): what the Gradient tool's freeform annotator shows and edits.
    pub fn proxy_freeform(&self, stroke: bool, item: Option<usize>) -> Option<(vectorcraft_color::Freeform, f64)> {
        let (paint, to_doc, b) = self.proxy_paint(stroke, item)?;
        let vectorcraft_color::Paint::Gradient(g) = paint else { return None };
        if g.gradient.kind != vectorcraft_color::GradientKind::Freeform {
            return None;
        }
        let mut f = g.freeform_on(b).into_owned();
        let mut scale = vectorcraft_color::freeform::spread_scale(b);
        if to_doc != Affine::IDENTITY {
            f.transform(to_doc);
            scale *= to_doc.determinant().abs().sqrt();
        }
        Some((f, scale))
    }
}

/// Unites filled regions (each under its own fill rule) into one path filled non-zero. Booleans
/// live above this crate (`vectorcraft-pathops`), so callers supply it.
pub type Uniter<'a> = &'a dyn Fn(&[(BezPath, FillRule)]) -> BezPath;

impl Node {
    /// The filled regions this object clips to as the clipping path of a clip group, in document
    /// space, each with its fill rule: a path, a compound path (with its holes), an image's frame,
    /// text outlined by `text` (glyph outlines need the font engine, above this crate), the visible
    /// members of a group (their union) and the clipping path of a nested clip group. Live objects
    /// are evaluated first. Guides and symbol instances add nothing.
    pub fn clip_shapes(&self, text: Outliner) -> Vec<(BezPath, FillRule)> {
        let mut out = vec![];
        self.push_clip_shapes(text, &mut out);
        out
    }

    fn push_clip_shapes(&self, text: Outliner, out: &mut Vec<(BezPath, FillRule)>) {
        match &self.kind {
            NodeKind::Path { guide: true, .. } | NodeKind::SymbolInstance { .. } => {}
            NodeKind::Path { path, rule, .. } => out.push((path.to_bezpath(), *rule)),
            NodeKind::Compound { children, rule } => {
                let mut bp = BezPath::new();
                for p in children.iter().filter_map(|c| c.path_data()) {
                    bp.extend(p.to_bezpath());
                }
                out.push((bp, *rule));
            }
            NodeKind::Group { children, clip: true } => {
                if let Some(c) = children.first() {
                    c.push_clip_shapes(text, out);
                }
            }
            NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => {
                for c in children.iter().skip(usize::from(self.extras().shaper.is_some())).filter(|c| c.visible) {
                    c.push_clip_shapes(text, out);
                }
            }
            NodeKind::Text(_) => {
                if let Some(o) = text.and_then(|f| f(self)) {
                    o.push_clip_shapes(None, out);
                }
            }
            NodeKind::Image(im) => {
                let frame = shapes::rectangle(Rect::new(0.0, 0.0, im.width as f64, im.height as f64)).transformed(im.xf);
                out.push((frame.to_bezpath(), FillRule::NonZero));
            }
            NodeKind::PlacedDocument(p) => {
                let frame = shapes::rectangle(p.natural()).transformed(p.xf);
                out.push((frame.to_bezpath(), FillRule::NonZero));
            }
            NodeKind::Blend { .. } | NodeKind::Envelope { .. } | NodeKind::Mesh(_) | NodeKind::Repeat(_) | NodeKind::Intertwine { .. } => {
                crate::live::expand_deep(self, text).push_clip_shapes(text, out);
            }
        }
    }

    /// The region this object clips to as one path and fill rule ([`Self::clip_shapes`]), shared
    /// by the renderer and the SVG and PDF writers so every output clips alike. One shape keeps its
    /// own rule; several are united by `unite` (filled non-zero). `None` when there is nothing to
    /// clip by: the clipped art is then hidden.
    pub fn clip_outline(&self, text: Outliner, unite: Uniter) -> Option<(BezPath, FillRule)> {
        let mut shapes = self.clip_shapes(text);
        match shapes.len() {
            0 => None,
            1 => shapes.pop(),
            _ => Some((unite(&shapes), FillRule::NonZero)),
        }
    }

    /// What this object paints as the clipping path of a clip group, shared by the renderer and
    /// the SVG and PDF writers: its fills, painted behind the clipped art, and its strokes, painted
    /// over it (unclipped). Each part is this object keeping only those appearance items (type:
    /// also only its characters' fills or strokes); `None` where it paints nothing, as right after
    /// Make Clipping Mask. Only paths, compound paths and type paint as clipping paths.
    pub fn clip_paint(&self) -> ClipPaint {
        use crate::appearance::AppearanceItem;
        let runs = match &self.kind {
            NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } => None,
            NodeKind::Text(t) => Some(&t.runs),
            _ => return ClipPaint::default(),
        };
        let paints = |fill: bool| {
            let item = |i: &AppearanceItem| match i {
                AppearanceItem::Fill(f) => fill && f.visible && !f.paint.is_none(),
                AppearanceItem::Stroke(s) => !fill && s.visible && !s.paint.is_none() && s.width > 0.0,
            };
            let run = |r: &crate::text::TextRun| if fill { !r.style.fill.is_none() } else { !r.style.stroke.is_none() && r.style.stroke_width > 0.0 };
            self.appearance.items.iter().any(item) || runs.is_some_and(|rs| rs.iter().any(run))
        };
        let part = |fill: bool| {
            paints(fill).then(|| {
                let mut n = self.clone();
                for i in (0..n.appearance.items.len()).rev() {
                    if matches!(n.appearance.items[i], AppearanceItem::Fill(_)) != fill {
                        n.appearance.remove_item(i);
                    }
                }
                if let NodeKind::Text(t) = &mut n.kind {
                    for r in &mut t.runs {
                        *(if fill { &mut r.style.stroke } else { &mut r.style.fill }) = vectorcraft_color::Paint::None;
                    }
                }
                n
            })
        };
        ClipPaint { fill: part(true), stroke: part(false) }
    }
}

/// What a clipping path paints ([`Node::clip_paint`]).
#[derive(Clone, Debug, Default)]
pub struct ClipPaint {
    /// The clipping path with only its fills: painted behind the clipped art (inside the clip).
    pub fill: Option<Node>,
    /// The clipping path with only its strokes: painted over the clipped art, not clipped.
    pub stroke: Option<Node>,
}

/// Knockout Group state of a container (the Transparency panel's three-state checkbox). In a
/// knockout group each child composites against the group's backdrop, so it hides the children
/// below it instead of showing them through its transparency. Neutral passes the enclosing
/// group's setting through to the children. Files from before the three states stored a bool:
/// `true` loads as On, `false` as Neutral.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Knockout {
    #[default]
    Neutral,
    Off,
    On,
}

impl Knockout {
    pub const ALL: [Knockout; 3] = [Knockout::Neutral, Knockout::Off, Knockout::On];

    /// Name in files and command parameters.
    pub fn label(self) -> &'static str {
        match self {
            Knockout::Neutral => "neutral",
            Knockout::Off => "off",
            Knockout::On => "on",
        }
    }
    /// A name ([`Self::label`], any case) or a bool as older files and commands wrote it.
    pub fn from_value(v: &serde_json::Value) -> Option<Self> {
        match v {
            serde_json::Value::Bool(b) => Some(Self::from(*b)),
            serde_json::Value::String(s) => Self::ALL.into_iter().find(|k| k.label().eq_ignore_ascii_case(s.trim())),
            _ => None,
        }
    }
    /// The state a click on the checkbox moves to: on → neutral → off → on.
    pub fn cycle(self) -> Self {
        match self {
            Knockout::On => Knockout::Neutral,
            Knockout::Neutral => Knockout::Off,
            Knockout::Off => Knockout::On,
        }
    }
    /// Whether children of a group with this state knock each other out, inside a group (or page)
    /// where that is `enclosing`.
    pub fn resolve(self, enclosing: bool) -> bool {
        match self {
            Knockout::Neutral => enclosing,
            Knockout::Off => false,
            Knockout::On => true,
        }
    }
}

impl From<bool> for Knockout {
    fn from(b: bool) -> Self {
        if b { Knockout::On } else { Knockout::Neutral }
    }
}

impl Serialize for Knockout {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.label())
    }
}

impl<'de> Deserialize<'de> for Knockout {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        Self::from_value(&v).ok_or_else(|| serde::de::Error::custom(format!("invalid knockout {v}")))
    }
}

impl Node {
    /// Whether the children of this group or layer knock each other out, inside a group (or page)
    /// where that is `enclosing`.
    pub fn knocks_out(&self, enclosing: bool) -> bool {
        matches!(self.kind, NodeKind::Group { .. } | NodeKind::Layer { .. }) && self.knockout.resolve(enclosing)
    }

    /// A plain group or layer that is neutral to knockout and has no transparency or appearance
    /// of its own: inside a knockout group its children composite as the group's own children.
    pub fn passes_knockout_through(&self) -> bool {
        matches!(self.kind, NodeKind::Group { clip: false, .. } | NodeKind::Layer { template: false, clip: false, .. })
            && self.has_default_transparency()
            && self.appearance.items.is_empty()
            && self.appearance.effects.is_empty()
    }

    /// The visible elements of a knockout group made of `children` (bottom first): neutral plain
    /// groups ([`Self::passes_knockout_through`]) contribute their children instead of themselves.
    pub fn knockout_elements(children: &[Arc<Node>]) -> Vec<&Arc<Node>> {
        fn push<'a>(children: &'a [Arc<Node>], out: &mut Vec<&'a Arc<Node>>) {
            for c in children.iter().filter(|c| c.visible) {
                match c.children() {
                    Some(ch) if c.passes_knockout_through() => push(ch, out),
                    _ => out.push(c),
                }
            }
        }
        let mut out = Vec::with_capacity(children.len());
        push(children, &mut out);
        out
    }

    /// Whether this is a clip group or a layer with a clipping mask: its first child clips the rest.
    pub fn clips(&self) -> bool {
        matches!(self.kind, NodeKind::Group { clip: true, .. } | NodeKind::Layer { clip: true, .. })
    }

    /// Make this group or layer clip its children by its first child, or stop it.
    pub fn set_clips(&mut self, on: bool) {
        if let NodeKind::Group { clip, .. } | NodeKind::Layer { clip, .. } = &mut self.kind {
            *clip = on;
        }
    }

    /// Whether blending inside this object reaches the art below it unless the object isolates
    /// it: a blend mode other than Normal on a child (or, in a leaf, on a fill or stroke),
    /// directly or through children that don't isolate their own blending.
    pub fn blends_through(&self) -> bool {
        self.blends_through_with(&mut |c| c.blends_through())
    }

    /// [`Self::blends_through`], with `inner` answering it for the children (e.g. from a cache).
    pub fn blends_through_with(&self, inner: &mut dyn FnMut(&Arc<Node>) -> bool) -> bool {
        match self.children() {
            Some(ch) if !matches!(self.kind, NodeKind::Compound { .. }) => Self::children_blend(ch, inner),
            _ => self.appearance.items.iter().any(|i| i.visible() && i.blend() != BlendMode::Normal),
        }
    }

    /// [`Self::blends_through`] of a group made of `children` (`inner` answers it for each child).
    pub fn children_blend(children: &[Arc<Node>], inner: &mut dyn FnMut(&Arc<Node>) -> bool) -> bool {
        children.iter().any(|c| c.visible && (c.blend != BlendMode::Normal || (!c.isolate && inner(c))))
    }
}

impl Node {
    /// The Appearance panel's row for what this object's own fills and strokes paint around:
    /// "Contents" for a group or layer (its members) and for an image (its pixels), "Characters"
    /// for type; `None` for objects whose fills and strokes paint their own geometry.
    pub fn contents_label(&self) -> Option<&'static str> {
        match self.kind {
            NodeKind::Group { .. } | NodeKind::Layer { .. } | NodeKind::Image(_) => Some("Contents"),
            NodeKind::Text(_) => Some("Characters"),
            _ => None,
        }
    }
}

/// Window → Attributes: an object's settings that don't change how it prints.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectAttributes {
    /// Show Center / Don't Show Center (`None`: the object kind's default, [`Node::shows_center`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_center: Option<bool>,
    /// Image Map: the clickable area the URL covers in web output.
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub image_map: ImageMap,
    /// URL the object links to (SVG `<a href>`; empty: none).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    /// The note shown in the Attributes panel.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// The object's own data, in order: SVG's `data-*` attributes (`data-pivot="100,180"` is
    /// `("pivot", "100,180")`), kept from import to export (`object.setProps {data}`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub data: Vec<(String, String)>,
    /// Window › SVG Interactivity: the JavaScript each event runs, in the panel's order
    /// (`("onclick", "alert(1)")`, one entry per event of [`SVG_EVENTS`]). SVG export writes them as
    /// the element's event attributes; nothing in VectorCraft runs them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<(String, String)>,
    /// Attributes in other applications' XML namespaces, in order, kept from SVG import to SVG
    /// export: what another tool notes on an object (a plotter's pen, a cutter's passes) survives
    /// a round trip through VectorCraft. Nothing in VectorCraft reads them. They are the object's,
    /// as `data` is, so copies, duplicates and pastes keep them and a deleted object takes them
    /// with it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub foreign: Vec<ForeignAttribute>,
}

/// An attribute in another application's XML namespace: `plotter:pen="2"` with `xmlns:plotter=
/// "https://example.org/plotter"` is `{ns: "https://example.org/plotter", prefix: "plotter", name:
/// "pen", value: "2"}`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForeignAttribute {
    /// The namespace's URI, which names it.
    pub ns: String,
    /// The prefix the file gave the namespace, which export reuses where it can.
    pub prefix: String,
    /// The attribute's local name.
    pub name: String,
    /// The attribute's value, as the file had it.
    pub value: String,
}

/// The events the SVG Interactivity panel attaches JavaScript to, by the names of the SVG event
/// attributes that run it.
pub const SVG_EVENTS: [&str; 19] = [
    "onfocusin",
    "onfocusout",
    "onactivate",
    "onmousedown",
    "onmouseup",
    "onclick",
    "onmouseover",
    "onmousemove",
    "onmouseout",
    "onkeydown",
    "onkeypress",
    "onkeyup",
    "onload",
    "onerror",
    "onabort",
    "onunload",
    "onzoom",
    "onresize",
    "onscroll",
];

/// The most JavaScript files a document links (`Document::svg_scripts`): SVG import keeps the
/// first ones and warns about the rest, and `svgInteractivity.addFile` refuses another.
pub const MAX_SVG_SCRIPTS: usize = 64;

/// The Attributes panel's Image Map shapes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageMap {
    #[default]
    None,
    Rectangle,
    Polygon,
}

impl ImageMap {
    pub const ALL: [ImageMap; 3] = [ImageMap::None, ImageMap::Rectangle, ImageMap::Polygon];

    pub fn label(self) -> &'static str {
        match self {
            ImageMap::None => "None",
            ImageMap::Rectangle => "Rectangle",
            ImageMap::Polygon => "Polygon",
        }
    }

    /// Parse a label or serialized name, ignoring case.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.label().eq_ignore_ascii_case(s))
    }
}

impl Node {
    /// The URL this object links to, if any.
    pub fn url(&self) -> Option<&str> {
        self.extras().attrs.as_deref().map(|a| a.url.as_str()).filter(|u| !u.is_empty())
    }

    /// Whether the canvas shows this object's centre point when it is selected: as set in the
    /// Attributes panel, else [`Self::shows_center_by_default`].
    pub fn shows_center(&self) -> bool {
        self.extras().attrs.as_deref().and_then(|a| a.show_center).unwrap_or_else(|| self.shows_center_by_default())
    }

    /// Shapes drawn with the shape tools (live rectangles, ellipses and polygons) show their centre.
    pub fn shows_center_by_default(&self) -> bool {
        matches!(&self.kind, NodeKind::Path { live: Some(l), .. } if !matches!(**l, LiveShape::Line { .. } | LiveShape::Path { .. }))
    }

    /// Change this object's attributes with `f`; all-default attributes are dropped.
    pub fn edit_attrs<R>(&mut self, f: impl FnOnce(&mut ObjectAttributes) -> R) -> R {
        let mut a = self.extras_mut().attrs.take().unwrap_or_default();
        let r = f(&mut a);
        self.extras_mut().attrs = (*a != ObjectAttributes::default()).then_some(a);
        r
    }
}

impl Node {
    /// Whether this object shows any transparency, itself or anything inside it: object, fill or
    /// stroke opacity below 100%, a blend mode, an opacity mask, effects or transparent gradient
    /// stops. Hidden objects show none.
    pub fn shows_transparency(&self) -> bool {
        use vectorcraft_color::Paint;
        let see_through = |p: &Paint| matches!(p, Paint::Gradient(g) if g.gradient.stops.iter().any(|s| s.opacity < 1.0));
        let effects = |fx: &[crate::Effect]| fx.iter().any(|e| e.visible);
        self.visible
            && (self.opacity < 1.0
                || self.blend != BlendMode::Normal
                || self.extras().mask.as_ref().is_some_and(|m| !m.disabled)
                || effects(&self.appearance.effects)
                || self
                    .appearance
                    .items
                    .iter()
                    .any(|i| i.visible() && (i.opacity() < 1.0 || i.blend() != BlendMode::Normal || see_through(i.paint()) || effects(i.effects())))
                || self.children().is_some_and(|ch| ch.iter().any(|c| c.shows_transparency())))
    }
}

/// Is `a` a move plus a positive uniform scale (after which a refit gradient still matches)?
fn keeps_gradient_fit(a: Affine) -> bool {
    let [m0, m1, m2, m3, _, _] = a.as_coeffs();
    let eps = 1e-12 * m0.abs().max(1.0);
    m0 > 0.0 && m1.abs() <= eps && m2.abs() <= eps && (m0 - m3).abs() <= eps
}

impl Node {
    /// Whether a document point lies in this object's filled regions ([`Node::clip_shapes`]; true
    /// everywhere for objects without them, such as type, whose boxes stand in).
    pub fn contains_fn(&self) -> impl Fn(Point) -> bool + use<> {
        let shapes = self.clip_shapes(None);
        move |p| shapes.is_empty() || shapes.iter().any(|(bp, rule)| vectorcraft_geom::hit::fill_contains(bp, *rule, p))
    }
}

/// Reading a [`NodeKind`]. Derived, an internally tagged enum buffers each kind (with every
/// object inside a group, again at each level of nesting) before reading it, since `type` may come
/// anywhere among its fields. Files have it first (it is written first): then the kind is read
/// straight from the file. Otherwise (hand-written files, values whose keys are sorted) the
/// fields before `type` are buffered, and only those.
mod kind_de {
    use std::fmt;
    use std::sync::Arc;

    use serde::de::value::{MapAccessDeserializer, StringDeserializer};
    use serde::de::{self, DeserializeSeed, Deserializer, EnumAccess, IntoDeserializer, MapAccess, VariantAccess, Visitor};
    use serde::{Deserialize, forward_to_deserialize_any};
    use vectorcraft_geom::{Affine, FillRule, PathData};

    use super::{LayerColor, LiveShape, Node, NodeKind, yes};
    use crate::live::{BlendSpec, EnvelopeKind, GradientMesh};
    use crate::pattern::RepeatSpec;
    use crate::text::TextObject;

    /// The kinds and their fields as [`NodeKind`] has them, with the defaults its field attributes
    /// note (the compiler checks that every field is here), read externally tagged: the kind from
    /// `type`, its fields from the rest of the object ([`Tagged`]).
    #[derive(Deserialize)]
    #[serde(remote = "NodeKind", rename_all = "lowercase")]
    enum KindDef {
        Layer {
            color: LayerColor,
            #[serde(default)]
            template: bool,
            #[serde(default = "yes")]
            printable: bool,
            children: Vec<Arc<Node>>,
            #[serde(default)]
            clip: bool,
            #[serde(default = "yes")]
            preview: bool,
            #[serde(default)]
            dim_images: Option<u8>,
        },
        Group {
            children: Vec<Arc<Node>>,
            #[serde(default)]
            clip: bool,
        },
        Path {
            path: PathData,
            #[serde(default)]
            rule: FillRule,
            #[serde(default)]
            live: Option<Box<LiveShape>>,
            #[serde(default)]
            clipping: bool,
            #[serde(default)]
            guide: bool,
        },
        Compound {
            children: Vec<Arc<Node>>,
            #[serde(default)]
            rule: FillRule,
        },
        Text(Box<TextObject>),
        Image(Box<super::ImageObject>),
        SymbolInstance {
            symbol: String,
            xf: Affine,
        },
        Blend {
            children: Vec<Arc<Node>>,
            #[serde(default)]
            spec: Box<BlendSpec>,
        },
        Envelope {
            content: Vec<Arc<Node>>,
            kind: Box<EnvelopeKind>,
            #[serde(default = "crate::live::default_fidelity")]
            fidelity: f64,
            #[serde(default)]
            editing: bool,
            #[serde(default)]
            options: crate::live::EnvelopeOptions,
            #[serde(default)]
            frame: Box<Affine>,
        },
        Mesh(GradientMesh),
        Repeat(RepeatSpec),
        PlacedDocument(Box<crate::placed_document::PlacedDocument>),
        Intertwine {
            children: Vec<Arc<Node>>,
        },
    }

    impl<'de> Deserialize<'de> for NodeKind {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            d.deserialize_map(KindVisitor)
        }
    }

    struct KindVisitor;

    impl<'de> Visitor<'de> for KindVisitor {
        type Value = NodeKind;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("an object kind (a map with `type`)")
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<NodeKind, A::Error> {
            let mut before = vec![];
            while let Some(key) = map.next_key::<String>()? {
                if key == "type" {
                    let tag: String = map.next_value()?;
                    return KindDef::deserialize(Tagged { tag, map: Buffered { before: before.into_iter(), value: None, map } });
                }
                before.push((key, map.next_value::<serde_json::Value>()?));
            }
            Err(de::Error::missing_field("type"))
        }
    }

    /// The fields read before `type` (buffered), then the rest of the object.
    struct Buffered<A> {
        before: std::vec::IntoIter<(String, serde_json::Value)>,
        value: Option<serde_json::Value>,
        map: A,
    }

    impl<'de, A: MapAccess<'de>> MapAccess<'de> for Buffered<A> {
        type Error = A::Error;

        fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, A::Error> {
            match self.before.next() {
                Some((key, value)) => {
                    self.value = Some(value);
                    seed.deserialize(StringDeserializer::<A::Error>::new(key)).map(Some)
                }
                None => self.map.next_key_seed(seed),
            }
        }

        fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, A::Error> {
            match self.value.take() {
                Some(value) => seed.deserialize(value).map_err(de::Error::custom),
                None => self.map.next_value_seed(seed),
            }
        }
    }

    /// An object whose `type` was read: the kind is `tag`, its fields are the rest of `map`.
    struct Tagged<A> {
        tag: String,
        map: A,
    }

    impl<'de, A: MapAccess<'de>> Deserializer<'de> for Tagged<A> {
        type Error = A::Error;

        fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, A::Error> {
            visitor.visit_enum(self)
        }

        forward_to_deserialize_any! {
            bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf
            option unit unit_struct newtype_struct seq tuple tuple_struct map struct enum
            identifier ignored_any
        }
    }

    impl<'de, A: MapAccess<'de>> EnumAccess<'de> for Tagged<A> {
        type Error = A::Error;
        type Variant = Fields<A>;

        fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, Fields<A>), A::Error> {
            let kind = seed.deserialize(self.tag.into_deserializer())?;
            Ok((kind, Fields(self.map)))
        }
    }

    /// A kind's fields: the rest of the object.
    struct Fields<A>(A);

    impl<'de, A: MapAccess<'de>> VariantAccess<'de> for Fields<A> {
        type Error = A::Error;

        fn unit_variant(self) -> Result<(), A::Error> {
            Err(de::Error::invalid_type(de::Unexpected::UnitVariant, &"an object kind with fields"))
        }

        fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, A::Error> {
            seed.deserialize(MapAccessDeserializer::new(self.0))
        }

        fn tuple_variant<V: Visitor<'de>>(self, _: usize, _: V) -> Result<V::Value, A::Error> {
            Err(de::Error::invalid_type(de::Unexpected::TupleVariant, &"an object kind with fields"))
        }

        fn struct_variant<V: Visitor<'de>>(self, _: &'static [&'static str], visitor: V) -> Result<V::Value, A::Error> {
            visitor.visit_map(self.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A node writes its rarely set parts among the others, in the file's field order, and reads
    /// them back into its extras; a node without any keeps none.
    #[test]
    fn extras_are_written_flat_in_order() {
        let mut n = Node::path(NodeId(7), shapes::rectangle(Rect::new(0.0, 0.0, 4.0, 2.0)), Appearance::default_art());
        n.name = Some("n".into());
        n.knockout_shape = true;
        n.bbox_angle = 30.0;
        {
            let mut e = n.extras_mut();
            e.mask = Some(Box::new(OpacityMask::new(Node::group(NodeId(8), vec![]), true)));
            e.trace = Some(Box::new(serde_json::json!({"preset": "p"})));
            e.series_index = Some(2);
            e.type_outlines = true;
            e.graphic_style = Some(3);
            e.attrs = Some(Box::default());
            e.slice = Some(Box::default());
            e.perspective = Some(Box::new(crate::PerspectiveAttachment::new("left", 1.0)));
            e.live_paint_gaps = Some(Box::default());
        }
        /// The top-level keys of a JSON object, in the order written.
        struct Keys(Vec<String>);
        impl<'de> Deserialize<'de> for Keys {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                struct V;
                impl<'de> serde::de::Visitor<'de> for V {
                    type Value = Keys;
                    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                        f.write_str("an object")
                    }
                    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut a: A) -> Result<Keys, A::Error> {
                        let mut out = vec![];
                        while let Some(k) = a.next_key::<String>()? {
                            a.next_value::<serde::de::IgnoredAny>()?;
                            out.push(k);
                        }
                        Ok(Keys(out))
                    }
                }
                d.deserialize_map(V)
            }
        }
        let text = serde_json::to_string(&n).unwrap();
        let keys = serde_json::from_str::<Keys>(&text).unwrap().0;
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let order = [
            "id",
            "name",
            "knockout_shape",
            "appearance",
            "mask",
            "trace",
            "series_index",
            "type_outlines",
            "kind",
            "graphic_style",
            "attrs",
            "slice",
            "bbox_angle",
            "perspective",
            "live_paint_gaps",
        ];
        assert_eq!(keys, order);
        assert_eq!(serde_json::from_value::<Node>(v).unwrap(), n);
        let plain = Node::path(NodeId(9), shapes::rectangle(Rect::new(0.0, 0.0, 4.0, 2.0)), Appearance::default_art());
        let back: Node = serde_json::from_value(serde_json::to_value(&plain).unwrap()).unwrap();
        assert!(back.extras_box().is_none());
        // Clearing the last extra lets them go; clearing one a node doesn't have allocates none.
        n.extras_mut().mask = None;
        assert!(n.extras_box().is_some());
        *n.extras_mut() = NodeExtras::default();
        assert!(n.extras_box().is_none());
        n.extras_mut().mask = None;
        assert!(n.extras_box().is_none());
        assert_eq!(n.extras(), &NodeExtras::default());
    }

    #[test]
    fn rect_live_shape_roundtrip() {
        let l = LiveShape::Rectangle {
            w: 10.0,
            h: 20.0,
            radii: [0.0; 4],
            kinds: Default::default(),
            radii_relative: false,
            xf: Affine::translate((5.0, 5.0)),
        };
        assert_eq!(l.to_path().bounds(), Some(Rect::new(5.0, 5.0, 15.0, 25.0)));
        assert_eq!(l.label(), "Rectangle");
    }

    #[test]
    fn corner_kinds_save_only_when_some_corner_isnt_round() {
        let mut l = LiveShape::Rectangle {
            w: 100.0,
            h: 50.0,
            radii: [0.0, 10.0, 0.0, 0.0],
            kinds: Default::default(),
            radii_relative: false,
            xf: Affine::IDENTITY,
        };
        // Older files read as round.
        let json = serde_json::to_value(&l).unwrap();
        assert!(json.get("kinds").is_none(), "{json}");
        assert_eq!(serde_json::from_value::<LiveShape>(json).unwrap(), l);
        if let LiveShape::Rectangle { kinds, .. } = &mut l {
            kinds[1] = CornerKind::Chamfer;
        }
        let json = serde_json::to_value(&l).unwrap();
        assert_eq!(json["kinds"], serde_json::json!(["round", "chamfer", "round", "round"]));
        assert_eq!(serde_json::from_value::<LiveShape>(json).unwrap(), l);
        // Polygons saved before Live Corners cut theirs read as sharp; a live path round-trips.
        let p: LiveShape =
            serde_json::from_value(serde_json::json!({"shape": "polygon", "radius": 10.0, "sides": 5, "xf": [1, 0, 0, 1, 0, 0]})).unwrap();
        assert_eq!(p, LiveShape::Polygon { radius: 10.0, sides: 5, xf: Affine::IDENTITY, radii: vec![], kinds: vec![], radii_relative: false });
        let star = shapes::star(Point::new(0.0, 0.0), 20.0, 10.0, 5, 0.0);
        let path = LiveShape::Path { base: star, radii: vec![2.0; 10], kinds: vec![], radii_relative: false };
        let json = serde_json::to_value(&path).unwrap();
        assert_eq!((json["shape"].as_str(), json.get("kinds")), (Some("path"), None));
        assert_eq!(serde_json::from_value::<LiveShape>(json).unwrap(), path);
        assert_eq!(path.label(), "Path");
    }

    /// A polygon read from a file has 3 to 1000 sides, as the commands give it: selecting one
    /// with four billion made its outline and corner lists allocate tens of gigabytes.
    #[test]
    fn a_polygon_from_a_file_has_3_to_1000_sides() {
        let read = |sides: u32| {
            let json = serde_json::json!({"shape": "polygon", "radius": 10.0, "sides": sides, "xf": [1, 0, 0, 1, 0, 0]});
            serde_json::from_value::<LiveShape>(json).unwrap()
        };
        let sides_of = |l: &LiveShape| match l {
            LiveShape::Polygon { sides, .. } => *sides,
            _ => panic!("not a polygon: {l:?}"),
        };
        let huge = read(4_000_000_000);
        assert_eq!([0, 5, 1000].map(|n| sides_of(&read(n))), [3, 5, 1000]);
        assert_eq!(sides_of(&huge), 1000);
        assert_eq!(huge.polygon_outline().anchor_count(), 1000);
    }

    #[test]
    fn a_polygon_reports_and_takes_its_radius_angle_and_equal_sides() {
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        // A hexagon of radius 10 at (50, 50), drawn 2× and turned 30° clockwise on the page.
        let xf = Affine::translate((50.0, 50.0)) * Affine::rotate(30f64.to_radians()) * Affine::scale(2.0);
        let mut p = LiveShape::Polygon { radius: 10.0, sides: 6, xf, radii: vec![], kinds: vec![], radii_relative: false };
        assert!(close(p.polygon_radius().unwrap(), 20.0) && close(p.polygon_angle().unwrap(), 330.0) && p.polygon_sides_equal());
        p.set_polygon_angle(45.0);
        assert!(close(p.polygon_angle().unwrap(), 45.0));
        let centre = p.to_path().bounds().unwrap().center();
        assert!(close(centre.x, 50.0) && close(centre.y, 50.0), "turned about its centre: {centre:?}");
        p.set_polygon_radius(30.0);
        assert!(close(p.polygon_radius().unwrap(), 30.0));
        // A hexagon's side is as long as its radius.
        assert!(close(p.polygon_radius_for_side(12.0).unwrap(), 12.0));
        // Stretched, its sides differ; Make Sides Equal keeps its centre, angle and mean radius.
        p.transform(Affine::scale_non_uniform(2.0, 1.0));
        assert!(!p.polygon_sides_equal());
        let (r, a) = (p.polygon_radius().unwrap(), p.polygon_angle().unwrap());
        p.make_sides_equal();
        assert!(p.polygon_sides_equal() && close(p.polygon_radius().unwrap(), r) && close(p.polygon_angle().unwrap(), a));
        let LiveShape::Polygon { xf, .. } = &p else { panic!("polygon") };
        assert_eq!(*xf * Point::ZERO, Point::new(100.0, 50.0));
        // Other shapes have none of it.
        let e = LiveShape::Ellipse { w: 1.0, h: 1.0, pie: (0.0, 360.0), xf: Affine::IDENTITY };
        assert_eq!((e.polygon_radius(), e.polygon_angle()), (None, None));
    }

    /// A node holding a live rectangle (`w` × `h`, all radii `r`, placed by `xf`) and its path.
    fn live_rect(w: f64, h: f64, r: f64, xf: Affine) -> Node {
        let live = LiveShape::Rectangle { w, h, radii: [r; 4], kinds: Default::default(), radii_relative: false, xf };
        let mut n = Node::path(NodeId(1), live.to_path(), Appearance::default_art());
        if let NodeKind::Path { live: slot, .. } = &mut n.kind {
            *slot = Some(Box::new(live));
        }
        n
    }

    fn live_of(n: &Node) -> (f64, f64, [f64; 4], Affine) {
        let NodeKind::Path { live: Some(l), .. } = &n.kind else { panic!("not a live shape") };
        let LiveShape::Rectangle { w, h, radii, xf, .. } = &**l else { panic!("not a live rectangle") };
        (*w, *h, *radii, *xf)
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn near_xf(a: Affine, b: Affine) -> bool {
        a.as_coeffs().iter().zip(b.as_coeffs()).all(|(x, y)| near(*x, y))
    }

    /// Size and radius of a live rectangle, within rounding.
    fn assert_live(n: &Node, w: f64, h: f64, r: f64) {
        let (lw, lh, radii, _) = live_of(n);
        assert!(near(lw, w) && near(lh, h) && radii.iter().all(|x| near(*x, r)), "{lw} × {lh} r {radii:?}, want {w} × {h} r {r}");
    }

    /// Each corner arc of a rounded rectangle's path, mapped by `back` (to undo a rotation),
    /// spans `r` along x and y: the corner is a circular arc of radius `r`.
    fn assert_round_corners(n: &Node, back: Affine, r: f64) {
        let NodeKind::Path { path, .. } = &n.kind else { panic!("not a path") };
        let a: Vec<Point> = path.subpaths[0].anchors.iter().map(|a| back * a.p).collect();
        assert_eq!(a.len(), 8, "a rounded rectangle");
        // Anchors 1→2, 3→4, 5→6 and 7→0 are the arcs.
        for (i, j) in [(1, 2), (3, 4), (5, 6), (7, 0)] {
            let (x, y) = ((a[j].x - a[i].x).abs(), (a[j].y - a[i].y).abs());
            assert!(near(x, r) && near(y, r), "corner spans {x} × {y}, want {r} × {r}");
        }
    }

    const KEEP_CORNERS: Scaling = Scaling { strokes: false, effects: None, keep_type_strokes: false, keep_corners: true, patterns: false };

    #[test]
    fn uneven_scales_keep_live_corners_circular() {
        // A 100 pt rounded square at (10, 10) stretched to 300 × 100 (#291).
        let mut n = live_rect(100.0, 100.0, 20.0, Affine::translate((10.0, 10.0)));
        let stretch = Affine::translate((10.0, 10.0)) * Affine::scale_non_uniform(3.0, 1.0) * Affine::translate((-10.0, -10.0));
        n.transform(stretch, KEEP_CORNERS);
        assert_live(&n, 300.0, 100.0, 20.0);
        assert!(near_xf(live_of(&n).3, Affine::translate((10.0, 10.0))), "the scale went into the size");
        let b = n.geometric_bounds().unwrap();
        assert!(near(b.x0, 10.0) && near(b.y0, 10.0) && near(b.x1, 310.0) && near(b.y1, 110.0), "{b:?}");
        assert_round_corners(&n, Affine::IDENTITY, 20.0);
        // Scale Corners on: the radius scales by the mean scale, still circular.
        let mut n = live_rect(100.0, 100.0, 20.0, Affine::IDENTITY);
        n.transform(Affine::scale_non_uniform(4.0, 1.0), Scaling::default());
        assert_live(&n, 400.0, 100.0, 40.0);
        assert_round_corners(&n, Affine::IDENTITY, 40.0);
        // Squashed below the radius, the short sides are fully round; stretched back, it returns.
        n.transform(Affine::scale_non_uniform(1.0, 0.5), KEEP_CORNERS);
        assert_round_corners(&n, Affine::IDENTITY, 25.0);
        n.transform(Affine::scale_non_uniform(1.0, 2.0), KEEP_CORNERS);
        assert_round_corners(&n, Affine::IDENTITY, 40.0);
    }

    #[test]
    fn rotated_and_reflected_live_rectangles_scale_along_their_own_sides() {
        let place = Affine::translate((200.0, 100.0)) * Affine::rotate(30f64.to_radians());
        let mut n = live_rect(100.0, 50.0, 10.0, place);
        // Stretched along its own width, as a bounding-box drag of a rotated shape does.
        n.transform(place * Affine::scale_non_uniform(2.0, 1.0) * place.inverse(), KEEP_CORNERS);
        assert_live(&n, 200.0, 50.0, 10.0);
        assert!(near_xf(live_of(&n).3, place));
        assert_round_corners(&n, place.inverse(), 10.0);
        // A reflection stays in `xf`; the corners stay as they were.
        n.transform(Affine::scale_non_uniform(-1.0, 1.0), KEEP_CORNERS);
        assert_live(&n, 200.0, 50.0, 10.0);
        let xf = live_of(&n).3;
        assert!(near(xf.determinant(), -1.0));
        assert_round_corners(&n, xf.inverse(), 10.0);
    }

    #[test]
    fn shears_and_moves_keep_the_old_transform() {
        // Scaling a rotated rectangle along the page axes shears it: `xf` takes it all, as before.
        let rot = Affine::rotate(30f64.to_radians());
        let mut n = live_rect(100.0, 50.0, 10.0, rot);
        let before = n.clone();
        let a = Affine::scale_non_uniform(2.0, 1.0);
        n.transform(a, Scaling::default());
        assert_eq!(live_of(&n), (100.0, 50.0, [10.0; 4], a * rot));
        let (NodeKind::Path { path, .. }, NodeKind::Path { path: old, .. }) = (&n.kind, &before.kind) else { panic!() };
        assert_eq!(*path, old.transformed(a));
        // A file saved with an uneven scale in `xf` keeps its exact geometry when moved...
        let legacy = Affine::translate((5.0, 5.0)) * Affine::scale_non_uniform(2.0, 1.0);
        let mut n = live_rect(100.0, 100.0, 20.0, legacy);
        n.transform(Affine::translate((10.0, 0.0)), KEEP_CORNERS);
        assert_eq!(live_of(&n), (100.0, 100.0, [20.0; 4], Affine::translate((10.0, 0.0)) * legacy));
        // ...and once scaled, has document units and circular corners of its mean radius.
        n.transform(Affine::scale(2.0), Scaling::default());
        assert_live(&n, 400.0, 200.0, 20.0 * 2f64.sqrt() * 2.0);
        assert!(near(live_of(&n).3.determinant(), 1.0));
        assert_round_corners(&n, Affine::IDENTITY, 20.0 * 2f64.sqrt() * 2.0);
    }

    /// A rectangle saved with an uneven scale in `xf` (before #291) has elliptical corners; Live
    /// Corners see and edit it in document units, with circular corners (#442).
    #[test]
    fn a_scale_left_in_the_transform_folds_into_the_size() {
        let legacy = Affine::translate((5.0, 5.0)) * Affine::scale_non_uniform(4.0, 1.0);
        let n = live_rect(50.0, 80.0, 20.0, legacy);
        let NodeKind::Path { live: Some(live), .. } = &n.kind else { panic!("not live") };
        let folded = live.folded();
        assert_eq!(live_of(&n).3, legacy, "folded() leaves the shape alone");
        let LiveShape::Rectangle { w, h, radii, xf, .. } = &folded else { panic!("not a rectangle") };
        // Drawn 200 × 80 with 80 × 20 corners; folded, the corners are circles of the mean radius.
        assert!(near(*w, 200.0) && near(*h, 80.0) && radii.iter().all(|r| near(*r, 40.0)), "{w} × {h} r {radii:?}");
        assert!(near_xf(*xf, Affine::translate((5.0, 5.0))));
        let (a, b) = (folded.to_path().bounds().unwrap(), live.to_path().bounds().unwrap());
        assert!(near(a.x0, b.x0) && near(a.y0, b.y0) && near(a.x1, b.x1) && near(a.y1, b.y1), "same place and size: {a:?} {b:?}");
        let corners = crate::LiveCorners::of(&n).unwrap();
        assert_eq!(corners.style(&corners.all()).0, Some(40.0), "the radius in document units");
        // Nothing to fold: a moved, rotated or reflected rectangle and a sheared one stay as they are.
        let place = Affine::translate((5.0, 5.0)) * Affine::rotate(0.5) * Affine::scale_non_uniform(-1.0, 1.0);
        let shear = Affine::new([1.0, 0.0, 0.5, 1.0, 0.0, 0.0]);
        for xf in [place, shear] {
            let mut l = live_rect(100.0, 160.0, 20.0, xf);
            let NodeKind::Path { live: Some(live), .. } = &mut l.kind else { panic!("not live") };
            assert!(!live.fold_scale());
            assert_eq!(live_of(&l), (100.0, 160.0, [20.0; 4], xf));
        }
    }

    #[test]
    fn default_transparency() {
        let p = || Node::path(NodeId(1), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art());
        assert!(p().has_default_transparency());
        let tweaks: [fn(&mut Node); 7] = [
            |n| n.opacity = 0.5,
            |n| n.blend = BlendMode::Multiply,
            |n| n.isolate = true,
            |n| n.knockout = Knockout::On,
            |n| n.knockout = Knockout::Off,
            |n| n.knockout_shape = true,
            |n| n.extras_mut().mask = Some(Box::new(OpacityMask::new(n.clone(), true))),
        ];
        for tweak in tweaks {
            let mut n = p();
            tweak(&mut n);
            assert!(!n.has_default_transparency(), "{n:?}");
        }
    }

    #[test]
    fn transform_group_recurses() {
        let p = Node::path(NodeId(2), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art());
        let mut g = Node::group(NodeId(1), vec![Arc::new(p)]);
        g.transform(Affine::scale(2.0), true);
        assert_eq!(g.geometric_bounds(), Some(Rect::new(0.0, 0.0, 20.0, 20.0)));
        let child = &g.children().unwrap()[0];
        assert_eq!(child.appearance.stroke_width(), 2.0);
    }

    #[test]
    fn unplaced_gradients_pin_only_when_a_refit_would_differ() {
        use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, Paint};
        let grad = || Paint::Gradient(Box::new(GradientPaint::new(Gradient::default())));
        let fresh = || Node::path(NodeId(2), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 50.0)), Appearance::basic(grad(), grad(), 2.0));
        let geoms = |n: &Node| match (n.appearance.fill_paint(), n.appearance.stroke_paint()) {
            (Paint::Gradient(f), Paint::Gradient(s)) => (f.geom, s.geom),
            _ => panic!("gradient fill and stroke"),
        };
        let mut n = fresh();
        n.transform(Affine::translate((10.0, 5.0)) * Affine::scale(2.0), true);
        assert_eq!(geoms(&n), (None, None), "moves and uniform scales keep refitting");
        let mut n = fresh();
        n.transform(Affine::rotate(std::f64::consts::FRAC_PI_2), false);
        let (f, s) = geoms(&n);
        // Fitted on the old bounds (strokes on the stroke-inflated box), then rotated: vertical.
        let mut want = GradientGeom::fit(GradientKind::Linear, Rect::new(0.0, 0.0, 100.0, 50.0), 0.0);
        want.transform(Affine::rotate(std::f64::consts::FRAC_PI_2), GradientKind::Linear);
        let f = f.unwrap();
        assert!(f.start.distance(want.start) < 1e-9 && f.end.distance(want.end) < 1e-9, "{f:?}");
        assert!((s.unwrap().length() - 102.0).abs() < 1e-9);
    }

    #[test]
    fn display_names() {
        let p = Node::path(NodeId(2), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art());
        assert_eq!(p.display_name(), "<Path>");
        let l = Node::layer(NodeId(1), "Layer 1", LayerColor::Preset(0));
        assert_eq!(l.display_name(), "Layer 1");
    }

    #[test]
    fn visual_bounds_include_stroke() {
        let mut p = Node::path(NodeId(2), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art());
        p.appearance.stroke_mut().unwrap().join = crate::appearance::LineJoin::Round;
        assert_eq!(p.visual_bounds(), Some(Rect::new(-0.5, -0.5, 10.5, 10.5)));
    }

    /// Type's visual bounds grow by how far its character strokes paint past the glyphs: half the
    /// weight centred, the whole weight outside, nothing inside; scaled with the type.
    #[test]
    fn type_visual_bounds_grow_with_the_character_stroke() {
        use crate::appearance::StrokeAlign;
        use crate::text::{CharStyle, TextObject};
        let style = CharStyle { stroke: vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK), stroke_width: 4.0, ..Default::default() };
        let mut t = TextObject::point(vectorcraft_geom::Point::new(0.0, 0.0), "Ab", style);
        t.xf = Affine::scale(2.0) * t.xf;
        let node = |t: &TextObject| Node::new(NodeId(1), NodeKind::Text(Box::new(t.clone())));
        let g = node(&t).geometric_bounds().unwrap();
        for (align, grow) in [(StrokeAlign::Center, 4.0), (StrokeAlign::Outside, 8.0), (StrokeAlign::Inside, 0.0)] {
            t.runs[0].style.stroke_align = align;
            assert_eq!(node(&t).visual_bounds(), Some(g.inflate(grow, grow)), "{align:?}");
        }
        t.runs[0].style.stroke = vectorcraft_color::Paint::None;
        assert_eq!(node(&t).visual_bounds(), Some(g), "no stroke, no growth");
    }

    /// A group holding a path, a compound, an image and a nested group, as a file writes it.
    fn nested_group() -> Node {
        let path = |id| Node::path(NodeId(id), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art());
        let image =
            ImageObject { key: "k".into(), width: 2, height: 3, xf: Affine::IDENTITY, link: None, placement: Default::default(), pixels: None };
        let inner = Node::new(NodeId(5), NodeKind::Group { children: vec![Arc::new(path(6))], clip: true });
        let compound = Node::new(NodeId(3), NodeKind::Compound { children: vec![Arc::new(path(4))], rule: FillRule::EvenOdd });
        let kids = vec![Arc::new(path(2)), Arc::new(compound), Arc::new(Node::new(NodeId(7), NodeKind::Image(Box::new(image)))), Arc::new(inner)];
        Node::new(NodeId(1), NodeKind::Group { children: kids, clip: false })
    }

    #[test]
    fn kinds_read_back_with_type_first_or_anywhere() {
        let n = nested_group();
        let json = serde_json::to_string(&n).unwrap();
        assert!(json.contains(r#""kind":{"type":"group","children":["#), "written with `type` first: {json}");
        assert_eq!(serde_json::from_str::<Node>(&json).unwrap(), n);
        // Through a JSON value, whose keys are sorted: `type` comes after `children`.
        let v = serde_json::to_value(&n).unwrap();
        assert_eq!(serde_json::from_value::<Node>(v.clone()).unwrap(), n);
        assert_eq!(serde_json::from_str::<Node>(&v.to_string()).unwrap(), n);
        // `type` last, in the middle, and keys a newer version added (ignored, as before).
        let kind = r#"{"children":[],"x":{"deep":[1,2]},"clip":true,"type":"group"}"#;
        assert_eq!(serde_json::from_str::<NodeKind>(kind).unwrap(), NodeKind::Group { children: vec![], clip: true });
        let kind = r#"{"symbol":"S","type":"symbolinstance","later":1,"xf":[1,0,0,1,5,6]}"#;
        assert_eq!(
            serde_json::from_str::<NodeKind>(kind).unwrap(),
            NodeKind::SymbolInstance { symbol: "S".into(), xf: Affine::translate((5.0, 6.0)) }
        );
        let kind = r#"{"width":2,"type":"image","key":"k","height":3,"xf":[1,0,0,1,0,0]}"#;
        assert!(matches!(serde_json::from_str::<NodeKind>(kind).unwrap(), NodeKind::Image(i) if i.key == "k" && i.height == 3));
        // Defaults as before.
        let NodeKind::Layer { printable, preview, template, .. } =
            serde_json::from_str(r#"{"type":"layer","color":{"Preset":0},"children":[]}"#).unwrap()
        else {
            panic!("a layer")
        };
        assert!(printable && preview && !template);
    }

    #[test]
    fn bad_kinds_are_errors() {
        for bad in [r#"{"children":[]}"#, r#"{"type":"nothing"}"#, r#"{"type":"group"}"#, r#"{"type":3,"children":[]}"#, r#"[]"#, r#""group""#] {
            assert!(serde_json::from_str::<NodeKind>(bad).is_err(), "{bad}");
        }
    }
}
