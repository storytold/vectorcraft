//! Document nodes.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use vectorcraft_color::BlendMode;
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, Point, Rect, shapes};

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
        /// Untransformed width/height.
        w: f64,
        h: f64,
        /// Corner radii: top-left, top-right, bottom-right, bottom-left.
        radii: [f64; 4],
        /// Maps the untransformed shape (origin at its top-left) into the document.
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
    },
    Line {
        a: Point,
        b: Point,
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
        }
    }
    /// Regenerate the path.
    pub fn to_path(&self) -> PathData {
        match self {
            LiveShape::Rectangle { w, h, radii, xf } => shapes::rounded_rectangle_each(Rect::new(0.0, 0.0, *w, *h), *radii).transformed(*xf),
            LiveShape::Ellipse { w, h, xf, .. } => shapes::ellipse(Rect::new(0.0, 0.0, *w, *h)).transformed(*xf),
            LiveShape::Polygon { radius, sides, xf } => shapes::polygon(Point::ZERO, *radius, *sides, 0.0).transformed(*xf),
            LiveShape::Line { a, b } => shapes::line(*a, *b),
        }
    }
    /// A live rectangle's corner (0 = top-left … 3 = bottom-left, in its own frame) of each anchor
    /// of [`Self::to_path`]; `None` for other shapes.
    pub fn rect_corners(&self) -> Option<Vec<usize>> {
        match self {
            LiveShape::Rectangle { w, h, radii, .. } => Some(shapes::rounded_rectangle_corners(Rect::new(0.0, 0.0, *w, *h), *radii)),
            _ => None,
        }
    }
    /// The corners of a live rectangle that `anchors` (direct-selected anchors of its path) sit
    /// on, in order; `None` for other shapes or when no anchor is picked.
    pub fn rect_corners_of(&self, anchors: Option<&std::collections::BTreeSet<(usize, usize)>>) -> Option<Vec<usize>> {
        let map = self.rect_corners()?;
        let mut out: Vec<usize> = anchors?.iter().filter(|(si, _)| *si == 0).filter_map(|(_, ai)| map.get(*ai).copied()).collect();
        out.sort_unstable();
        out.dedup();
        (!out.is_empty()).then_some(out)
    }
    pub fn transform(&mut self, a: Affine) {
        match self {
            LiveShape::Rectangle { xf, .. } | LiveShape::Ellipse { xf, .. } | LiveShape::Polygon { xf, .. } => *xf = a * *xf,
            LiveShape::Line { a: p, b } => {
                *p = a * *p;
                *b = a * *b;
            }
        }
    }
    /// Divide live corner radii by `k`, the mean scale of a transform just applied, so the corners
    /// keep their size (Scale Corners off). False when nothing changed.
    pub fn keep_corners(&mut self, k: f64) -> bool {
        match self {
            LiveShape::Rectangle { radii, .. } if radii.iter().any(|r| *r > 0.0) && k > 1e-12 => {
                radii.iter_mut().for_each(|r| *r /= k);
                true
            }
            _ => false,
        }
    }
    /// Rotation angle of the live shape in degrees (shown in the Properties panel).
    pub fn angle_deg(&self) -> f64 {
        match self {
            LiveShape::Rectangle { xf, .. } | LiveShape::Ellipse { xf, .. } | LiveShape::Polygon { xf, .. } => {
                let c = xf.as_coeffs();
                c[1].atan2(c[0]).to_degrees()
            }
            LiveShape::Line { a, b } => (b.y - a.y).atan2(b.x - a.x).to_degrees(),
        }
    }
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
    /// Image Trace object: `{preset, params}` it was traced with (the Image Trace panel shows them).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<Box<serde_json::Value>>,
    /// Object → Text Wrap: area type below this object (in the same layer) flows around it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap: Option<crate::text::TextWrap>,
    /// Graph object: the group's children are generated from this spec (Object → Graph).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph: Option<Box<crate::graph::GraphSpec>>,
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
        let mut n = Self::new(id, NodeKind::Layer { color, template: false, printable: true, children: vec![], clip: false });
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
    pub fn is_layer(&self) -> bool {
        matches!(self.kind, NodeKind::Layer { .. })
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
                children.iter().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, c.visual_bounds())).map(|b| b.inflate(o, o))
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
        match &mut self.kind {
            NodeKind::Path { path, live, .. } => {
                path.transform(a);
                if let Some(l) = live {
                    l.transform(a);
                    if sc.keep_corners
                        && let Some(k) = k
                        && l.keep_corners(k)
                    {
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
                for c in children.iter().filter(|c| c.visible) {
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
        matches!(&self.kind, NodeKind::Path { live: Some(l), .. } if !matches!(l, LiveShape::Line { .. }))
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
        let l = LiveShape::Rectangle { w: 10.0, h: 20.0, radii: [0.0; 4], xf: Affine::translate((5.0, 5.0)) };
        assert_eq!(l.to_path().bounds(), Some(Rect::new(5.0, 5.0, 15.0, 25.0)));
        assert_eq!(l.label(), "Rectangle");
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
