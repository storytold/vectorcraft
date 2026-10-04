//! The appearance model: a stack of fills and strokes, each with its own opacity, blend mode and
//! effects, plus object-level effects.

use serde::{Deserialize, Serialize};
use vectorcraft_color::{BlendMode, Color, Paint};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StrokeAlign {
    #[default]
    Center,
    Inside,
    Outside,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Dash {
    /// Dash, gap, dash, gap… (up to 6 values in the Stroke panel).
    pub pattern: Vec<f64>,
    #[serde(default)]
    pub offset: f64,
    /// "Aligns dashes to corners and path ends, adjusting lengths to fit".
    #[serde(default)]
    pub align_corners: bool,
}

impl Dash {
    /// Whether the pattern draws dashes. As in PDF (`d` operator) and SVG (`stroke-dasharray`), a
    /// pattern with a negative or non-finite value, or whose values are all zero, is invalid and the
    /// stroke is drawn solid.
    pub fn is_dashed(&self) -> bool {
        self.pattern.iter().all(|v| v.is_finite() && *v >= 0.0) && self.pattern.iter().any(|v| *v > 0.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Arrowhead {
    Triangle,
    TriangleOpen,
    Circle,
    CircleOpen,
    Square,
    SquareOpen,
    Bar,
    Diamond,
    Arrow,
    ArrowOpen,
}

impl Arrowhead {
    pub const ALL: [Arrowhead; 10] = [
        Arrowhead::Arrow,
        Arrowhead::ArrowOpen,
        Arrowhead::Triangle,
        Arrowhead::TriangleOpen,
        Arrowhead::Circle,
        Arrowhead::CircleOpen,
        Arrowhead::Square,
        Arrowhead::SquareOpen,
        Arrowhead::Diamond,
        Arrowhead::Bar,
    ];
}

/// Where an arrowhead sits relative to the end of its path. In both modes the stroke stops under
/// the head, so the line never shows through a hollow head or past its tip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ArrowAlign {
    /// The tip extends past the end point (the path keeps its length).
    #[default]
    Extend,
    /// The tip sits on the end point (the stroke is shortened by the head).
    Tip,
}

/// Variable-width profile: (position 0..1 along the path, left width factor, right width factor).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WidthProfile {
    pub points: Vec<(f64, f64, f64)>,
}

/// A built-in width profile (the Stroke panel's Profile list).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProfilePreset {
    /// Stable id used by `stroke.set {profile}`.
    pub id: &'static str,
    /// Menu label.
    pub label: &'static str,
    /// (t, left, right) width points.
    pub points: &'static [(f64, f64, f64)],
}

impl WidthProfile {
    /// Width factor at `t` (average of both sides), linear between points.
    pub fn at(&self, t: f64) -> (f64, f64) {
        let p = &self.points;
        if p.is_empty() {
            return (1.0, 1.0);
        }
        if t <= p[0].0 {
            return (p[0].1, p[0].2);
        }
        for w in p.windows(2) {
            if t <= w[1].0 {
                let u = (t - w[0].0) / (w[1].0 - w[0].0).max(1e-9);
                return (w[0].1 + (w[1].1 - w[0].1) * u, w[0].2 + (w[1].2 - w[0].2) * u);
            }
        }
        p.last().map_or((1.0, 1.0), |l| (l.1, l.2))
    }
    /// The built-in profiles, in menu order. "uniform" is the plain stroke (no profile).
    pub const PRESETS: [ProfilePreset; 4] = [
        ProfilePreset { id: "uniform", label: "Uniform", points: &[(0.0, 1.0, 1.0), (1.0, 1.0, 1.0)] },
        ProfilePreset { id: "lens", label: "Lens", points: &[(0.0, 0.0, 0.0), (0.5, 1.0, 1.0), (1.0, 0.0, 0.0)] },
        ProfilePreset { id: "taperStart", label: "Taper Start", points: &[(0.0, 0.0, 0.0), (1.0, 1.0, 1.0)] },
        ProfilePreset { id: "taperEnd", label: "Taper End", points: &[(0.0, 1.0, 1.0), (1.0, 0.0, 0.0)] },
    ];
    /// The built-in profile with this id.
    pub fn preset(id: &str) -> Option<Self> {
        Self::PRESETS.iter().find(|p| p.id == id).map(|p| Self { points: p.points.to_vec() })
    }
    /// The id of the built-in profile these points match, if any.
    pub fn preset_id(&self) -> Option<&'static str> {
        Self::PRESETS.iter().find(|p| p.points == self.points.as_slice()).map(|p| p.id)
    }
    /// The id of a stroke's profile: "uniform" without one, "custom" when it matches no preset.
    pub fn id_of(p: Option<&Self>) -> &'static str {
        p.map_or(Some("uniform"), Self::preset_id).unwrap_or("custom")
    }
    /// The lens profile (thin ends, full width in the middle).
    pub fn lens() -> Self {
        Self::preset("lens").unwrap_or_else(|| Self { points: vec![] })
    }
    pub fn taper_end() -> Self {
        Self::preset("taperEnd").unwrap_or_else(|| Self { points: vec![] })
    }
    pub fn taper_start() -> Self {
        Self::preset("taperStart").unwrap_or_else(|| Self { points: vec![] })
    }
}

/// A live effect in an appearance stack. Parameters are interpreted by `vectorcraft-effects`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Effect {
    /// Stable id, e.g. `stylize.dropShadow`, `distort.roughen`, `path.offsetPath`, `warp.arc`.
    pub id: String,
    #[serde(default)]
    pub params: serde_json::Value,
    #[serde(default = "yes")]
    pub visible: bool,
}

fn yes() -> bool {
    true
}
fn one() -> f32 {
    1.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FillLayer {
    pub paint: Paint,
    #[serde(default = "one", skip_serializing_if = "crate::skip::is_one")]
    pub opacity: f32,
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub blend: BlendMode,
    #[serde(default = "yes", skip_serializing_if = "crate::skip::is_true")]
    pub visible: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
    /// Overprint Fill: on press the fill's inks print over the inks below instead of knocking
    /// them out (Overprint Preview and Separations Preview show it).
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub overprint: bool,
}

impl FillLayer {
    pub fn new(paint: Paint) -> Self {
        Self { paint, opacity: 1.0, blend: BlendMode::Normal, visible: true, effects: vec![], overprint: false }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrokeLayer {
    pub paint: Paint,
    /// Weight in points.
    pub width: f64,
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub cap: LineCap,
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub join: LineJoin,
    #[serde(default = "ten", skip_serializing_if = "is_ten")]
    pub miter_limit: f64,
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub align: StrokeAlign,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dash: Option<Dash>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_arrow: Option<Arrowhead>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_arrow: Option<Arrowhead>,
    /// Arrowhead scale in percent (start, end).
    #[serde(default = "hundreds", skip_serializing_if = "is_hundreds")]
    pub arrow_scale: (f64, f64),
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<WidthProfile>,
    /// Brush applied to the stroke (by brush name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brush: Option<String>,
    #[serde(default = "one", skip_serializing_if = "crate::skip::is_one")]
    pub opacity: f32,
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub blend: BlendMode,
    #[serde(default = "yes", skip_serializing_if = "crate::skip::is_true")]
    pub visible: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
    /// Arrowhead placement at both ends.
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub arrow_align: ArrowAlign,
    /// Overprint Stroke (see [`FillLayer::overprint`]).
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub overprint: bool,
}

fn ten() -> f64 {
    10.0
}
fn hundreds() -> (f64, f64) {
    (100.0, 100.0)
}
fn is_ten(v: &f64) -> bool {
    *v == 10.0
}
fn is_hundreds(v: &(f64, f64)) -> bool {
    *v == (100.0, 100.0)
}

impl StrokeLayer {
    pub fn new(paint: Paint, width: f64) -> Self {
        Self {
            paint,
            width,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter_limit: 10.0,
            align: StrokeAlign::Center,
            dash: None,
            start_arrow: None,
            end_arrow: None,
            arrow_scale: (100.0, 100.0),
            profile: None,
            brush: None,
            opacity: 1.0,
            blend: BlendMode::Normal,
            visible: true,
            effects: vec![],
            arrow_align: ArrowAlign::Extend,
            overprint: false,
        }
    }
    /// Weight of the start (`end == false`) or end arrowhead: stroke weight × its scale, at least
    /// a quarter point. A head of weight `hw` is `4·hw` long and wide.
    pub fn arrow_weight(&self, end: bool) -> f64 {
        let pct = if end { self.arrow_scale.1 } else { self.arrow_scale.0 };
        (self.width * pct / 100.0).max(0.25)
    }
    /// How far the arrowheads can reach from the path's end points (0 without heads): the
    /// head's diagonal, or with [`ArrowAlign::Extend`] its length plus the cap past the end.
    pub fn arrow_reach(&self) -> f64 {
        let reach = |head: Option<Arrowhead>, end: bool| head.map_or(0.0, |_| 4.5 * self.arrow_weight(end) + self.width / 2.0);
        reach(self.start_arrow, false).max(reach(self.end_arrow, true))
    }
    /// The box an unplaced gradient on this stroke fits to (see [`stroke_paint_bounds`]).
    pub fn paint_bounds(&self, geometric: vectorcraft_geom::Rect) -> vectorcraft_geom::Rect {
        stroke_paint_bounds(geometric, self.width)
    }
}

/// The box an unplaced gradient on a stroke of `width` fits to: the geometric bounds grown by half
/// the weight (object strokes and type strokes alike, on screen and in exports).
pub fn stroke_paint_bounds(geometric: vectorcraft_geom::Rect, width: f64) -> vectorcraft_geom::Rect {
    geometric.inflate(width / 2.0, width / 2.0)
}

/// One entry of the appearance stack.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum AppearanceItem {
    Fill(FillLayer),
    Stroke(StrokeLayer),
}

impl AppearanceItem {
    pub fn is_fill(&self) -> bool {
        matches!(self, AppearanceItem::Fill(_))
    }
    /// `"fill"` or `"stroke"` (the serialized `kind`).
    pub fn kind_name(&self) -> &'static str {
        if self.is_fill() { "fill" } else { "stroke" }
    }
    pub fn paint(&self) -> &Paint {
        match self {
            AppearanceItem::Fill(f) => &f.paint,
            AppearanceItem::Stroke(s) => &s.paint,
        }
    }
    pub fn visible(&self) -> bool {
        match self {
            AppearanceItem::Fill(f) => f.visible,
            AppearanceItem::Stroke(s) => s.visible,
        }
    }
    pub fn opacity(&self) -> f32 {
        match self {
            AppearanceItem::Fill(f) => f.opacity,
            AppearanceItem::Stroke(s) => s.opacity,
        }
    }
    pub fn blend(&self) -> BlendMode {
        match self {
            AppearanceItem::Fill(f) => f.blend,
            AppearanceItem::Stroke(s) => s.blend,
        }
    }
    /// The item's own live effects (applied to this fill or stroke only).
    pub fn effects(&self) -> &Vec<Effect> {
        match self {
            AppearanceItem::Fill(f) => &f.effects,
            AppearanceItem::Stroke(s) => &s.effects,
        }
    }
    pub fn effects_mut(&mut self) -> &mut Vec<Effect> {
        match self {
            AppearanceItem::Fill(f) => &mut f.effects,
            AppearanceItem::Stroke(s) => &mut s.effects,
        }
    }
    /// Whether this fill or stroke overprints.
    pub fn overprint(&self) -> bool {
        match self {
            AppearanceItem::Fill(f) => f.overprint,
            AppearanceItem::Stroke(s) => s.overprint,
        }
    }
    pub fn overprint_mut(&mut self) -> &mut bool {
        match self {
            AppearanceItem::Fill(f) => &mut f.overprint,
            AppearanceItem::Stroke(s) => &mut s.overprint,
        }
    }
}

/// Appearance attributes. `items` is in paint order: `items[0]` is painted first (the bottom of the
/// Appearance panel list).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Appearance {
    #[serde(default)]
    pub items: Vec<AppearanceItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
}

impl Appearance {
    /// Illustrator's "basic appearance": one fill below one stroke.
    pub fn basic(fill: Paint, stroke: Paint, width: f64) -> Self {
        Self { items: vec![AppearanceItem::Fill(FillLayer::new(fill)), AppearanceItem::Stroke(StrokeLayer::new(stroke, width))], effects: vec![] }
    }
    /// White fill, 1 pt black stroke (the default for new art).
    pub fn default_art() -> Self {
        Self::basic(Paint::solid(Color::WHITE), Paint::solid(Color::BLACK), 1.0)
    }
    /// The topmost fill (what the Fill proxy shows).
    pub fn fill(&self) -> Option<&FillLayer> {
        self.items.iter().rev().find_map(|i| if let AppearanceItem::Fill(f) = i { Some(f) } else { None })
    }
    pub fn fill_mut(&mut self) -> Option<&mut FillLayer> {
        self.items.iter_mut().rev().find_map(|i| if let AppearanceItem::Fill(f) = i { Some(f) } else { None })
    }
    /// The topmost stroke (what the Stroke proxy shows).
    pub fn stroke(&self) -> Option<&StrokeLayer> {
        self.items.iter().rev().find_map(|i| if let AppearanceItem::Stroke(s) = i { Some(s) } else { None })
    }
    pub fn stroke_mut(&mut self) -> Option<&mut StrokeLayer> {
        self.items.iter_mut().rev().find_map(|i| if let AppearanceItem::Stroke(s) = i { Some(s) } else { None })
    }
    pub fn fill_paint(&self) -> Paint {
        self.fill().map(|f| f.paint.clone()).unwrap_or(Paint::None)
    }
    pub fn stroke_paint(&self) -> Paint {
        self.stroke().map(|s| s.paint.clone()).unwrap_or(Paint::None)
    }
    /// Set the top fill's paint, creating a fill if there is none.
    pub fn set_fill(&mut self, p: Paint) {
        match self.fill_mut() {
            Some(f) => f.paint = p,
            None => self.items.insert(0, AppearanceItem::Fill(FillLayer::new(p))),
        }
    }
    /// Set the top stroke's paint, creating a 1 pt stroke if there is none.
    pub fn set_stroke(&mut self, p: Paint) {
        match self.stroke_mut() {
            Some(s) => s.paint = p,
            None => self.items.push(AppearanceItem::Stroke(StrokeLayer::new(p, 1.0))),
        }
    }
    pub fn stroke_width(&self) -> f64 {
        self.stroke().filter(|s| !s.paint.is_none()).map(|s| s.width).unwrap_or(0.0)
    }
    /// Is this a basic appearance: at most one fill and one stroke, none hidden or with its own
    /// opacity, blend mode or effects, and no object effects?
    pub fn is_basic(&self) -> bool {
        let count = |fill: bool| self.items.iter().filter(|i| i.is_fill() == fill).count();
        self.effects.is_empty()
            && count(true) <= 1
            && count(false) <= 1
            && self.items.iter().all(|i| i.visible() && i.effects().is_empty() && i.opacity() == 1.0 && i.blend() == BlendMode::Normal)
    }
    /// Fill item `index`, or the topmost fill for `None`. `None` when that item is not a fill.
    pub fn fill_at(&self, index: Option<usize>) -> Option<&FillLayer> {
        match index {
            None => self.fill(),
            Some(i) => match self.items.get(i)? {
                AppearanceItem::Fill(f) => Some(f),
                AppearanceItem::Stroke(_) => None,
            },
        }
    }
    pub fn fill_at_mut(&mut self, index: Option<usize>) -> Option<&mut FillLayer> {
        match index {
            None => self.fill_mut(),
            Some(i) => match self.items.get_mut(i)? {
                AppearanceItem::Fill(f) => Some(f),
                AppearanceItem::Stroke(_) => None,
            },
        }
    }
    /// Stroke item `index`, or the topmost stroke for `None`. `None` when that item is not a stroke.
    pub fn stroke_at(&self, index: Option<usize>) -> Option<&StrokeLayer> {
        match index {
            None => self.stroke(),
            Some(i) => match self.items.get(i)? {
                AppearanceItem::Stroke(s) => Some(s),
                AppearanceItem::Fill(_) => None,
            },
        }
    }
    pub fn stroke_at_mut(&mut self, index: Option<usize>) -> Option<&mut StrokeLayer> {
        match index {
            None => self.stroke_mut(),
            Some(i) => match self.items.get_mut(i)? {
                AppearanceItem::Stroke(s) => Some(s),
                AppearanceItem::Fill(_) => None,
            },
        }
    }
    /// `index` when it names a fill (`fill`) or stroke item, else `None` (the topmost one): the
    /// item a fill or stroke edit changes while item `index` is the Appearance panel's target.
    pub fn item_of_kind(&self, index: Option<usize>, fill: bool) -> Option<usize> {
        index.filter(|i| self.items.get(*i).is_some_and(|it| it.is_fill() == fill))
    }
    /// The fill the Fill proxy shows while item `index` is targeted: that item when it is a fill,
    /// else the topmost fill.
    pub fn fill_for(&self, index: Option<usize>) -> Option<&FillLayer> {
        self.fill_at(self.item_of_kind(index, true))
    }
    /// The stroke the Stroke proxy and panel show while item `index` is targeted.
    pub fn stroke_for(&self, index: Option<usize>) -> Option<&StrokeLayer> {
        self.stroke_at(self.item_of_kind(index, false))
    }
    /// Set the paint of fill item `index` (`None`: the top fill, created if missing). False when
    /// `index` is not a fill.
    pub fn set_fill_at(&mut self, index: Option<usize>, p: Paint) -> bool {
        if index.is_none() {
            self.set_fill(p);
            return true;
        }
        self.fill_at_mut(index).map(|f| f.paint = p).is_some()
    }
    /// Set the paint of stroke item `index` (`None`: the top stroke, created if missing). False
    /// when `index` is not a stroke.
    pub fn set_stroke_at(&mut self, index: Option<usize>, p: Paint) -> bool {
        if index.is_none() {
            self.set_stroke(p);
            return true;
        }
        self.stroke_at_mut(index).map(|s| s.paint = p).is_some()
    }
    /// The paint of fill (`fill`) or stroke item `index` (`None`: the topmost one).
    pub fn paint_at(&self, index: Option<usize>, fill: bool) -> Option<&Paint> {
        if fill { self.fill_at(index).map(|f| &f.paint) } else { self.stroke_at(index).map(|s| &s.paint) }
    }
    /// [`Self::set_fill_at`] or [`Self::set_stroke_at`].
    pub fn set_paint_at(&mut self, index: Option<usize>, fill: bool, p: Paint) -> bool {
        if fill { self.set_fill_at(index, p) } else { self.set_stroke_at(index, p) }
    }
    /// The effects of item `index`, or the object-level effects for `None`.
    pub fn effects_at(&self, index: Option<usize>) -> Option<&Vec<Effect>> {
        match index {
            None => Some(&self.effects),
            Some(i) => self.items.get(i).map(AppearanceItem::effects),
        }
    }
    pub fn effects_mut(&mut self, index: Option<usize>) -> Option<&mut Vec<Effect>> {
        match index {
            None => Some(&mut self.effects),
            Some(i) => self.items.get_mut(i).map(AppearanceItem::effects_mut),
        }
    }
    /// Largest distance the painted area extends beyond the geometry (for visual bounds).
    pub fn outset(&self) -> f64 {
        self.items
            .iter()
            .filter_map(|i| match i {
                AppearanceItem::Stroke(s) if s.visible && !s.paint.is_none() => Some(
                    match s.align {
                        StrokeAlign::Center => s.width / 2.0 * if s.join == LineJoin::Miter { s.miter_limit.min(4.0) } else { 1.0 },
                        StrokeAlign::Outside => s.width,
                        StrokeAlign::Inside => 0.0,
                    }
                    .max(s.arrow_reach()),
                ),
                _ => None,
            })
            .fold(0.0, f64::max)
    }
    /// Scale stroke weights (Scale Strokes & Effects).
    pub fn scale_strokes(&mut self, s: f64) {
        for i in &mut self.items {
            if let AppearanceItem::Stroke(st) = i {
                st.width *= s;
                if let Some(d) = &mut st.dash {
                    for v in &mut d.pattern {
                        *v *= s;
                    }
                }
            }
        }
    }
    /// Gradient paints of the fills and strokes.
    fn gradients_mut(&mut self) -> impl Iterator<Item = &mut vectorcraft_color::GradientPaint> {
        self.items.iter_mut().filter_map(|i| match i {
            AppearanceItem::Fill(FillLayer { paint: Paint::Gradient(g), .. })
            | AppearanceItem::Stroke(StrokeLayer { paint: Paint::Gradient(g), .. }) => Some(&mut **g),
            _ => None,
        })
    }
    /// Gradient paints of the fills and strokes.
    fn gradients(&self) -> impl Iterator<Item = &vectorcraft_color::GradientPaint> {
        self.items.iter().filter_map(|i| match i {
            AppearanceItem::Fill(FillLayer { paint: Paint::Gradient(g), .. })
            | AppearanceItem::Stroke(StrokeLayer { paint: Paint::Gradient(g), .. }) => Some(&**g),
            _ => None,
        })
    }
    /// Is `self` equal to `other`, placed gradients within rounding (1e-6 pt) of each other, as
    /// moving them between boxes leaves them?
    pub fn approx_eq(&self, other: &Appearance) -> bool {
        if self == other {
            return true;
        }
        let mut a = self.clone();
        for (g, o) in a.gradients_mut().zip(other.gradients()) {
            if let (Some(ga), Some(go)) = (&mut g.geom, &o.geom)
                && ga.start.distance(go.start) <= 1e-6
                && ga.end.distance(go.end) <= 1e-6
                && (ga.aspect - go.aspect).abs() <= 1e-6
            {
                *ga = *go;
            }
        }
        a == *other
    }
    /// Does a fill or stroke carry a gradient whose placement `placed` is (some: placed in document
    /// space; none: fitted to the bounds on each render)?
    fn has_gradient(&self, placed: bool) -> bool {
        self.items.iter().any(|i| match i {
            AppearanceItem::Fill(FillLayer { paint: Paint::Gradient(g), .. })
            | AppearanceItem::Stroke(StrokeLayer { paint: Paint::Gradient(g), .. }) => g.geom.is_some() == placed,
            _ => false,
        })
    }
    /// Does a fill or stroke carry a gradient that is fitted to the bounds on each render?
    pub fn has_unplaced_gradient(&self) -> bool {
        self.has_gradient(false)
    }
    /// Does a fill or stroke carry a placed gradient (start and end in document space)?
    pub fn has_placed_gradient(&self) -> bool {
        self.has_gradient(true)
    }
    /// Fix unplaced gradients to their fit on `bounds` (the object's geometric bounds; strokes fit
    /// the stroke-inflated box), so they can follow transforms that refitting wouldn't reproduce.
    pub fn pin_gradients(&mut self, bounds: vectorcraft_geom::Rect) {
        for i in &mut self.items {
            match i {
                AppearanceItem::Fill(FillLayer { paint: Paint::Gradient(g), .. }) => g.pin(bounds),
                AppearanceItem::Stroke(s) => {
                    let b = s.paint_bounds(bounds);
                    if let Paint::Gradient(g) = &mut s.paint {
                        g.pin(b);
                    }
                }
                _ => {}
            }
        }
    }
    /// Move placed gradients from an object whose geometric bounds are `from` to one whose bounds
    /// are `to`, keeping them at the same place relative to the box (strokes: relative to their
    /// stroke-inflated boxes, as they fit).
    pub fn rebase_gradients(&mut self, from: vectorcraft_geom::Rect, to: vectorcraft_geom::Rect) {
        for i in &mut self.items {
            match i {
                AppearanceItem::Fill(FillLayer { paint: Paint::Gradient(g), .. }) => g.rebase(from, to),
                AppearanceItem::Stroke(s) => {
                    let (f, t) = (s.paint_bounds(from), s.paint_bounds(to));
                    if let Paint::Gradient(g) = &mut s.paint {
                        g.rebase(f, t);
                    }
                }
                _ => {}
            }
        }
    }
    /// Map placed gradients through `a`.
    pub fn transform_gradients(&mut self, a: vectorcraft_geom::Affine) {
        for g in self.gradients_mut() {
            g.transform(a);
        }
    }
    /// Map placed gradients through a warp, given its local affine approximation at a point (taken
    /// at each gradient's centre: the start of a radial, the middle of a linear vector).
    pub fn warp_gradients(&mut self, near: &dyn Fn(vectorcraft_geom::Point) -> vectorcraft_geom::Affine) {
        for g in self.gradients_mut() {
            if let Some(geom) = g.geom {
                let c = if g.gradient.kind == vectorcraft_color::GradientKind::Radial { geom.start } else { geom.start.midpoint(geom.end) };
                g.transform(near(c));
            }
        }
    }
}

impl Appearance {
    /// Is any fill, stroke or effect (the object's or an item's) hidden?
    pub fn has_hidden(&self) -> bool {
        let hidden = |fx: &[Effect]| fx.iter().any(|e| !e.visible);
        hidden(&self.effects) || self.items.iter().any(|i| !i.visible() || hidden(i.effects()))
    }

    /// Make every hidden fill, stroke and effect visible (Show All Hidden Attributes). Returns
    /// whether anything was hidden.
    pub fn show_all(&mut self) -> bool {
        let had = self.has_hidden();
        let show = |fx: &mut Vec<Effect>| fx.iter_mut().for_each(|e| e.visible = true);
        show(&mut self.effects);
        for i in &mut self.items {
            match i {
                AppearanceItem::Fill(f) => f.visible = true,
                AppearanceItem::Stroke(s) => s.visible = true,
            }
            show(i.effects_mut());
        }
        had
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_width_profiles_are_built_in() {
        // `lens()` and friends fall back to uniform only if a preset id is mistyped.
        for p in [WidthProfile::lens(), WidthProfile::taper_end(), WidthProfile::taper_start()] {
            assert!(p.preset_id().is_some(), "{p:?}");
        }
    }

    #[test]
    fn hidden_attributes_show_again() {
        let mut a = Appearance::default_art();
        assert!(!a.has_hidden() && !a.show_all());
        a.stroke_mut().unwrap().visible = false;
        a.items[0].effects_mut().push(Effect { id: "distort.roughen".into(), params: serde_json::Value::Null, visible: false });
        assert!(a.has_hidden());
        assert!(a.show_all());
        assert!(!a.has_hidden() && a.items.iter().all(AppearanceItem::visible) && a.items[0].effects()[0].visible);
    }

    #[test]
    fn basic_appearance() {
        let a = Appearance::default_art();
        assert!(a.is_basic());
        assert_eq!(a.fill_paint(), Paint::solid(Color::WHITE));
        assert_eq!(a.stroke_width(), 1.0);
    }

    #[test]
    fn rebase_moves_fills_by_the_box_and_strokes_by_their_inflated_box() {
        use vectorcraft_color::{Gradient, GradientGeom, GradientPaint};
        use vectorcraft_geom::{Point, Rect};
        let placed = |x: f64| {
            let mut g = GradientPaint::new(Gradient::default());
            g.geom = Some(GradientGeom { start: Point::new(x, 0.0), end: Point::new(x + 10.0, 0.0), aspect: 1.0 });
            Paint::Gradient(Box::new(g))
        };
        let mut a = Appearance::basic(placed(0.0), placed(-5.0), 10.0);
        a.rebase_gradients(Rect::new(0.0, 0.0, 100.0, 100.0), Rect::new(100.0, 0.0, 300.0, 100.0));
        let start = |p: Paint| match p {
            Paint::Gradient(g) => g.geom.unwrap().start,
            _ => panic!("not a gradient"),
        };
        // The fill's start stays on the left edge; the stroke's on its inflated box's (−5 → 95).
        assert_eq!((start(a.fill_paint()), start(a.stroke_paint())), (Point::new(100.0, 0.0), Point::new(95.0, 0.0)));
    }

    #[test]
    fn items_by_index() {
        let mut a = Appearance::default_art();
        a.items.push(AppearanceItem::Fill(FillLayer::new(Paint::None)));
        // [Fill white, Stroke black, Fill none]
        assert_eq!(a.fill_at(None).unwrap().paint, Paint::None);
        assert_eq!(a.fill_at(Some(0)).unwrap().paint, Paint::solid(Color::WHITE));
        assert!(a.fill_at(Some(1)).is_none() && a.stroke_at(Some(0)).is_none() && a.fill_at(Some(9)).is_none());
        assert_eq!(a.item_of_kind(Some(1), false), Some(1));
        assert_eq!(a.item_of_kind(Some(1), true), None);
        assert_eq!(a.fill_for(Some(1)).unwrap().paint, Paint::None);
        assert!(a.set_fill_at(Some(0), Paint::solid(Color::BLACK)));
        assert!(!a.set_stroke_at(Some(0), Paint::None));
        assert_eq!(a.fill_at(Some(0)).unwrap().paint, Paint::solid(Color::BLACK));
        a.effects_mut(Some(1)).unwrap().push(Effect { id: "distort.roughen".into(), params: serde_json::Value::Null, visible: true });
        assert_eq!(a.effects_at(Some(1)).unwrap().len(), 1);
        assert!(a.effects_at(None).unwrap().is_empty() && a.effects_mut(Some(3)).is_none());
        assert_eq!((a.items[1].kind_name(), a.items[2].kind_name()), ("stroke", "fill"));
    }

    #[test]
    fn basic_means_one_plain_fill_and_stroke() {
        let stroke = || AppearanceItem::Stroke(StrokeLayer::new(Paint::solid(Color::BLACK), 1.0));
        let mut a = Appearance { items: vec![stroke(), stroke()], effects: vec![] };
        assert!(!a.is_basic(), "two strokes");
        a.items.pop();
        assert!(a.is_basic() && Appearance::default().is_basic());
        let mut b = Appearance::default_art();
        b.stroke_mut().unwrap().visible = false;
        assert!(!b.is_basic(), "a hidden stroke");
        let mut c = Appearance::default_art();
        c.fill_mut().unwrap().opacity = 0.5;
        assert!(!c.is_basic(), "fill opacity");
        let mut d = Appearance::default_art();
        d.items.push(AppearanceItem::Fill(FillLayer::new(Paint::None)));
        assert!(!d.is_basic(), "two fills");
        let mut e = Appearance::default_art();
        e.effects.push(Effect { id: "distort.roughen".into(), params: serde_json::Value::Null, visible: true });
        assert!(!e.is_basic(), "an effect");
    }

    #[test]
    fn set_creates_missing() {
        let mut a = Appearance::default();
        a.set_fill(Paint::solid(Color::BLACK));
        a.set_stroke(Paint::solid(Color::WHITE));
        assert_eq!(a.items.len(), 2);
        assert!(matches!(a.items[0], AppearanceItem::Fill(_)));
    }

    #[test]
    fn profile_interp() {
        let p = WidthProfile::lens();
        assert_eq!(p.at(0.5), (1.0, 1.0));
        assert_eq!(p.at(0.25), (0.5, 0.5));
        assert_eq!(p.at(2.0), (0.0, 0.0));
    }

    #[test]
    fn profile_presets_round_trip_their_ids() {
        for p in WidthProfile::PRESETS {
            let prof = WidthProfile::preset(p.id).unwrap();
            assert_eq!(prof.preset_id(), Some(p.id));
            assert_eq!(WidthProfile::id_of(Some(&prof)), p.id);
        }
        assert_eq!(WidthProfile::id_of(None), "uniform");
        assert_eq!(WidthProfile::id_of(Some(&WidthProfile { points: vec![(0.0, 0.3, 0.3)] })), "custom");
        assert!(WidthProfile::preset("nope").is_none());
        assert_eq!(WidthProfile::lens().points, vec![(0.0, 0.0, 0.0), (0.5, 1.0, 1.0), (1.0, 0.0, 0.0)]);
    }

    #[test]
    fn arrow_align_defaults_to_extend_and_round_trips() {
        let mut st = StrokeLayer::new(Paint::solid(Color::BLACK), 2.0);
        assert_eq!(st.arrow_align, ArrowAlign::Extend);
        // The default is not written, so older readers see the same JSON as before.
        assert!(!serde_json::to_string(&st).unwrap().contains("arrow_align"));
        let old: StrokeLayer = serde_json::from_str(r#"{"paint":{"type":"none"},"width":2.0}"#).unwrap();
        assert_eq!(old.arrow_align, ArrowAlign::Extend);
        st.arrow_align = ArrowAlign::Tip;
        let back: StrokeLayer = serde_json::from_str(&serde_json::to_string(&st).unwrap()).unwrap();
        assert_eq!(back, st);
    }

    #[test]
    fn outset_covers_arrowheads() {
        let mut a = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 4.0);
        a.stroke_mut().unwrap().join = LineJoin::Round;
        assert_eq!(a.outset(), 2.0);
        let st = a.stroke_mut().unwrap();
        st.end_arrow = Some(Arrowhead::Triangle);
        st.arrow_scale = (100.0, 200.0);
        assert_eq!(st.arrow_weight(false), 4.0);
        assert_eq!(st.arrow_weight(true), 8.0);
        // A 32 pt head whose tip sits up to its length (plus the cap) past the end point.
        assert_eq!(a.outset(), 4.5 * 8.0 + 2.0);
        a.stroke_mut().unwrap().width = 0.01;
        assert_eq!(a.stroke().unwrap().arrow_weight(true), 0.25, "heads keep a minimum size");
    }

    #[test]
    fn outset_depends_on_align() {
        let mut a = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 10.0);
        a.stroke_mut().unwrap().join = LineJoin::Round;
        assert_eq!(a.outset(), 5.0);
        a.stroke_mut().unwrap().align = StrokeAlign::Outside;
        assert_eq!(a.outset(), 10.0);
        a.stroke_mut().unwrap().align = StrokeAlign::Inside;
        assert_eq!(a.outset(), 0.0);
    }
}
