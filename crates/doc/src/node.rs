//! Document nodes.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use vectorcraft_color::BlendMode;
use vectorcraft_geom::corners::cut_corners;
use vectorcraft_geom::shapes::{self, CornerKind};
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, Point, Rect};

use crate::appearance::Appearance;
use crate::live::{BlendSpec, EnvelopeKind, GradientMesh, Outliner};
use crate::pattern::RepeatSpec;
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
        sides: u32,
        xf: Affine,
        /// Corner radii in document units, by vertex from the first one clockwise (none: sharp).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        radii: Vec<f64>,
        /// Corner kinds, in the same order (none: round).
        #[serde(default, skip_serializing_if = "all_round")]
        kinds: Vec<CornerKind>,
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
    },
}

impl LiveShape {
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
            LiveShape::Rectangle { w, h, radii, kinds, xf } => {
                shapes::rectangle_with_corners(Rect::new(0.0, 0.0, *w, *h), *radii, *kinds).transformed(*xf)
            }
            LiveShape::Ellipse { w, h, pie, xf } => shapes::ellipse_pie(Rect::new(0.0, 0.0, *w, *h), pie.0, pie.1).transformed(*xf),
            LiveShape::Polygon { radii, kinds, .. } => cut_corners(&self.polygon_outline(), radii, kinds).0,
            LiveShape::Line { a, b } => shapes::line(*a, *b),
            LiveShape::Path { base, radii, kinds } => cut_corners(base, radii, kinds).0,
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
            LiveShape::Polygon { xf, radii, .. } => {
                *xf = a * *xf;
                scale_radii(radii, a)
            }
            LiveShape::Path { base, radii, .. } => {
                base.transform(a);
                scale_radii(radii, a)
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
        let LiveShape::Rectangle { w, h, radii, xf, .. } = self else { return false };
        let [m0, m1, m2, m3, m4, m5] = xf.as_coeffs();
        let (sx, sy) = (m0.hypot(m1), m2.hypot(m3));
        let orthogonal = sx > 1e-12 && sy > 1e-12 && (m0 * m2 + m1 * m3).abs() <= 1e-9 * sx * sy;
        let finite = xf.as_coeffs().iter().all(|c| c.is_finite()) && w.is_finite() && h.is_finite();
        let unscaled = (sx - 1.0).abs() < 1e-12 && (sy - 1.0).abs() < 1e-12;
        if !orthogonal || !finite || unscaled {
            return false;
        }
        let k = (sx * sy).sqrt();
        radii.iter_mut().for_each(|r| *r *= k);
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
        let radii: &mut [f64] = match self {
            LiveShape::Rectangle { radii, .. } => radii,
            LiveShape::Polygon { radii, .. } | LiveShape::Path { radii, .. } => radii,
            LiveShape::Ellipse { .. } | LiveShape::Line { .. } => return false,
        };
        if !radii.iter().any(|r| *r > 0.0) || k <= 1e-12 {
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

/// Whether `a` only moves (no rotation, scale, shear or reflection).
fn moves_only(a: Affine) -> bool {
    let [a0, a1, a2, a3, _, _] = a.as_coeffs();
    (a0 - 1.0).abs() < 1e-12 && a1.abs() < 1e-12 && a2.abs() < 1e-12 && (a3 - 1.0).abs() < 1e-12
}

/// Scale corner radii in document units by the mean scale of `a`, applied to their path. True
/// when the path must be cut again: some corner is cut and `a` does more than move it (an
/// uneven scale would otherwise make the cuts elliptical).
fn scale_radii(radii: &mut [f64], a: Affine) -> bool {
    if moves_only(a) || !radii.iter().any(|r| *r > 0.0) {
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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        live: Option<LiveShape>,
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
    Image(ImageObject),
    SymbolInstance {
        symbol: String,
        xf: Affine,
    },
    /// Live blend: the key objects (paint order) plus spacing/orientation/spine; the intermediate
    /// steps are evaluated on demand (`live::blend_expand`).
    Blend {
        children: Vec<Arc<Node>>,
        #[serde(default)]
        spec: BlendSpec,
    },
    /// Live envelope distortion of `content`.
    Envelope {
        content: Vec<Arc<Node>>,
        kind: EnvelopeKind,
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
        frame: Affine,
    },
    /// Gradient mesh.
    Mesh(GradientMesh),
    /// Live Repeat (radial / grid / mirror) of source art.
    Repeat(RepeatSpec),
    /// A placed document: an artboard of another VectorCraft file, linked and locked (see
    /// [`crate::placed_document`]).
    PlacedDocument(Box<crate::placed_document::PlacedDocument>),
}

fn yes() -> bool {
    true
}
fn one() -> f32 {
    1.0
}

/// One object in the tree. Children are `Arc`s so documents share structure between undo states.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default = "yes", skip_serializing_if = "crate::skip::is_true")]
    pub visible: bool,
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub locked: bool,
    #[serde(default = "one", skip_serializing_if = "crate::skip::is_one")]
    pub opacity: f32,
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub blend: BlendMode,
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub isolate: bool,
    /// Knockout Group: whether a container's children knock each other out (see [`Knockout`]).
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub knockout: Knockout,
    /// Opacity & Mask Define Knockout Shape: inside a knockout group, this object's opacity and
    /// opacity mask scale how much of the objects below it in the group it knocks out.
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub knockout_shape: bool,
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub appearance: Appearance,
    /// Opacity mask (Transparency panel). Its art lives here, outside the layer tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<Box<OpacityMask>>,
    /// Image Trace object: `{preset, params, view?}` it was traced with (the Image Trace panel shows
    /// them; `view` is its [`crate::TraceView`] id, absent for the tracing result).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<Box<serde_json::Value>>,
    /// Object → Text Wrap: area type below this object (in the same layer) flows around it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap: Option<crate::text::TextWrap>,
    /// Graph object: the group's children are generated from this spec (Object → Graph).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph: Option<Box<crate::graph::GraphSpec>>,
    /// Index of the graph series this generated group belongs to. Axes, the legend and other
    /// children leave it unset, so a series named "Legend" or "Axes" is not mistaken for them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_index: Option<u32>,
    /// Editable Shaper composition; the original art is retained in its source child.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shaper: Option<Box<crate::shaper::ShaperSpec>>,
    pub kind: NodeKind,
    /// The [`crate::GraphicStyle::id`] last applied to this object. It stays linked while it keeps
    /// that style's look: editing its appearance or transparency breaks the link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graphic_style: Option<u32>,
    /// Attributes panel: centre point display, image map, URL and note (`None`: all defaults).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attrs: Option<Box<ObjectAttributes>>,
    /// Object → Slice → Make: the object is an object slice (its slice follows its bounds) with
    /// these options.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slice: Option<Box<crate::SliceOptions>>,
    /// The angle of the object's own axes, counter-clockwise degrees (0: square to the page): its
    /// bounding box and handles stand at this angle. Transforms turn it with the object (see
    /// [`crate::orient`]); Reset Bounding Box sets it back to 0.
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub bbox_angle: f64,
    /// Object › Perspective: the perspective grid plane the object is attached to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perspective: Option<Box<crate::PerspectiveAttachment>>,
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
            appearance: Appearance::default(),
            mask: None,
            trace: None,
            wrap: None,
            graph: None,
            series_index: None,
            shaper: None,
            kind,
            graphic_style: None,
            attrs: None,
            slice: None,
            bbox_angle: 0.0,
            perspective: None,
        }
    }
    pub fn path(id: NodeId, path: PathData, appearance: Appearance) -> Self {
        let mut n = Self::new(id, NodeKind::Path { path, rule: FillRule::NonZero, live: None, clipping: false, guide: false });
        n.appearance = appearance;
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
            | NodeKind::Repeat(RepeatSpec { source: children, .. }) => Some(children),
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
            | NodeKind::Repeat(RepeatSpec { source: children, .. }) => Some(children),
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
            && self.mask.is_none()
    }
    /// Kind name as the Layers panel / Properties panel shows it.
    pub fn kind_label(&self) -> &'static str {
        match &self.kind {
            NodeKind::Layer { .. } => "Layer",
            NodeKind::Group { clip: true, .. } => "Clip Group",
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
    /// ([`Appearance::outset`]) instead of measuring along their shape.
    pub fn reach_bounds(&self) -> Option<Rect> {
        match &self.kind {
            NodeKind::Path { .. } | NodeKind::Compound { .. } => {
                let o = self.appearance.outset();
                self.geometric_bounds().map(|b| b.inflate(o, o))
            }
            _ => self.visual_bounds(),
        }
    }
    /// Geometric bounds (no stroke), recursively. Clip groups are bounded by their clip path.
    pub fn geometric_bounds(&self) -> Option<Rect> {
        match &self.kind {
            NodeKind::Path { path, .. } => path.bounds(),
            NodeKind::Group { children, clip: true } | NodeKind::Layer { children, clip: true, .. } => {
                children.first().and_then(|c| c.geometric_bounds())
            }
            NodeKind::Layer { children, .. } | NodeKind::Group { children, .. } | NodeKind::Compound { children, .. } => children
                .iter()
                .skip(usize::from(self.shaper.is_some()))
                .filter(|c| c.visible || !matches!(self.kind, NodeKind::Layer { .. }))
                .fold(None, |acc, c| vectorcraft_geom::union_opt(acc, c.geometric_bounds())),
            NodeKind::Text(t) => self.projected(t.bounds()),
            NodeKind::Image(im) => Some(im.xf.transform_rect_bbox(Rect::new(0.0, 0.0, im.width as f64, im.height as f64))),
            NodeKind::SymbolInstance { xf, .. } => self.projected(Some(xf.transform_rect_bbox(Rect::new(-10.0, -10.0, 10.0, 10.0)))),
            NodeKind::Blend { children, spec } => {
                let b = crate::live::nodes_bounds(children);
                vectorcraft_geom::union_opt(b, spec.spine.as_ref().and_then(|s| s.bounds()))
            }
            NodeKind::Envelope { content, kind, frame, .. } => crate::live::envelope_bounds(content, kind, *frame),
            NodeKind::Mesh(m) => m.bounds(),
            NodeKind::Repeat(r) => r.bounds(),
            NodeKind::PlacedDocument(p) => Some(p.bounds()),
        }
    }
    /// Visual bounds: what the object paints, its strokes included (paths and compound paths
    /// measure theirs along their own shape, see [`Appearance::stroked_bounds`]).
    pub fn visual_bounds(&self) -> Option<Rect> {
        match &self.kind {
            NodeKind::Group { children, clip: true } | NodeKind::Layer { children, clip: true, .. } => {
                // A clipping path that paints (see `clip_paint`) strokes over the clip edge.
                let clip = children.first()?;
                if matches!(clip.kind, NodeKind::Path { .. } | NodeKind::Compound { .. } | NodeKind::Text(_)) {
                    clip.visual_bounds()
                } else {
                    clip.geometric_bounds()
                }
            }
            NodeKind::Layer { children, .. } | NodeKind::Group { children, .. } => {
                // The container's own strokes paint around its members.
                let o = self.appearance.outset();
                children
                    .iter()
                    .skip(usize::from(self.shaper.is_some()))
                    .fold(None, |acc, c| vectorcraft_geom::union_opt(acc, c.visual_bounds()))
                    .map(|b| b.inflate(o, o))
            }
            NodeKind::Blend { children, spec } => {
                let b = children.iter().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, c.visual_bounds()));
                let o = crate::live::max_outset(children);
                vectorcraft_geom::union_opt(b, spec.spine.as_ref().and_then(|s| s.bounds()).map(|r| r.inflate(o, o)))
            }
            NodeKind::Envelope { content, .. } | NodeKind::Repeat(RepeatSpec { source: content, .. }) => {
                let o = crate::live::max_outset(content);
                self.geometric_bounds().map(|b| b.inflate(o, o))
            }
            NodeKind::Path { .. } | NodeKind::Compound { .. } => {
                let b = self.geometric_bounds()?;
                Some(self.appearance.stroked_bounds(b, || self.stroke_path().unwrap_or_default()))
            }
            _ => {
                let o = self.appearance.outset();
                self.geometric_bounds().map(|b| b.inflate(o, o))
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
        self.appearance.transform_gradients(a);
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
            NodeKind::Layer { children, .. } | NodeKind::Group { children, .. } | NodeKind::Compound { children, .. } => {
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
                *frame = crate::live::envelope_frame(a * *frame);
                match kind {
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
        if let Some(m) = &mut self.mask
            && m.linked
        {
            Arc::make_mut(&mut m.art).transform_scaled(a, sc);
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
                for c in children.iter().skip(usize::from(self.shaper.is_some())).filter(|c| c.visible) {
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
            NodeKind::Blend { .. } | NodeKind::Envelope { .. } | NodeKind::Mesh(_) | NodeKind::Repeat(_) => {
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
    /// "Contents" for a group or layer (its members), "Characters" for type; `None` for objects
    /// whose fills and strokes paint their own geometry.
    pub fn contents_label(&self) -> Option<&'static str> {
        match self.kind {
            NodeKind::Group { .. } | NodeKind::Layer { .. } => Some("Contents"),
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
}

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
        self.attrs.as_deref().map(|a| a.url.as_str()).filter(|u| !u.is_empty())
    }

    /// Whether the canvas shows this object's centre point when it is selected: as set in the
    /// Attributes panel, else [`Self::shows_center_by_default`].
    pub fn shows_center(&self) -> bool {
        self.attrs.as_deref().and_then(|a| a.show_center).unwrap_or_else(|| self.shows_center_by_default())
    }

    /// Shapes drawn with the shape tools (live rectangles, ellipses and polygons) show their centre.
    pub fn shows_center_by_default(&self) -> bool {
        matches!(&self.kind, NodeKind::Path { live: Some(l), .. } if !matches!(l, LiveShape::Line { .. } | LiveShape::Path { .. }))
    }

    /// Change this object's attributes with `f`; all-default attributes are dropped.
    pub fn edit_attrs<R>(&mut self, f: impl FnOnce(&mut ObjectAttributes) -> R) -> R {
        let mut a = self.attrs.take().unwrap_or_default();
        let r = f(&mut a);
        self.attrs = (*a != ObjectAttributes::default()).then_some(a);
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
                || self.mask.as_ref().is_some_and(|m| !m.disabled)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_live_shape_roundtrip() {
        let l = LiveShape::Rectangle { w: 10.0, h: 20.0, radii: [0.0; 4], kinds: Default::default(), xf: Affine::translate((5.0, 5.0)) };
        assert_eq!(l.to_path().bounds(), Some(Rect::new(5.0, 5.0, 15.0, 25.0)));
        assert_eq!(l.label(), "Rectangle");
    }

    #[test]
    fn corner_kinds_save_only_when_some_corner_isnt_round() {
        let mut l = LiveShape::Rectangle { w: 100.0, h: 50.0, radii: [0.0, 10.0, 0.0, 0.0], kinds: Default::default(), xf: Affine::IDENTITY };
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
        assert_eq!(p, LiveShape::Polygon { radius: 10.0, sides: 5, xf: Affine::IDENTITY, radii: vec![], kinds: vec![] });
        let star = shapes::star(Point::new(0.0, 0.0), 20.0, 10.0, 5, 0.0);
        let path = LiveShape::Path { base: star, radii: vec![2.0; 10], kinds: vec![] };
        let json = serde_json::to_value(&path).unwrap();
        assert_eq!((json["shape"].as_str(), json.get("kinds")), (Some("path"), None));
        assert_eq!(serde_json::from_value::<LiveShape>(json).unwrap(), path);
        assert_eq!(path.label(), "Path");
    }

    #[test]
    fn a_polygon_reports_and_takes_its_radius_angle_and_equal_sides() {
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        // A hexagon of radius 10 at (50, 50), drawn 2× and turned 30° clockwise on the page.
        let xf = Affine::translate((50.0, 50.0)) * Affine::rotate(30f64.to_radians()) * Affine::scale(2.0);
        let mut p = LiveShape::Polygon { radius: 10.0, sides: 6, xf, radii: vec![], kinds: vec![] };
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
        let live = LiveShape::Rectangle { w, h, radii: [r; 4], kinds: Default::default(), xf };
        let mut n = Node::path(NodeId(1), live.to_path(), Appearance::default_art());
        if let NodeKind::Path { live: slot, .. } = &mut n.kind {
            *slot = Some(live);
        }
        n
    }

    fn live_of(n: &Node) -> (f64, f64, [f64; 4], Affine) {
        let NodeKind::Path { live: Some(LiveShape::Rectangle { w, h, radii, xf, .. }), .. } = &n.kind else { panic!("not a live rectangle") };
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
            |n| n.mask = Some(Box::new(OpacityMask::new(n.clone(), true))),
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
}
