//! The appearance model: a stack of fills and strokes, each with its own opacity, blend mode and
//! effects, plus object-level effects.

use serde::{Deserialize, Serialize};
use vectorcraft_color::{BlendMode, Color, Paint};

mod layers;
mod profile;
mod stroke_style;

pub(crate) use layers::HEAD_REACH;
pub use layers::{Effect, FillLayer, StrokeGradientMode, StrokeLayer, stroke_paint_bounds};
pub use profile::{PressureProfile, ProfilePreset, SavedProfile, WidthProfile};
pub use stroke_style::{ArrowAlign, Arrowhead, Dash, LineCap, LineJoin, StrokeAlign};

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
    pub fn paint_mut(&mut self) -> &mut Paint {
        match self {
            AppearanceItem::Fill(f) => &mut f.paint,
            AppearanceItem::Stroke(s) => &mut s.paint,
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
    /// Groups, layers, type and images: how many items paint below the object's contents (its
    /// members, type's characters, or an image's pixels), the slot of the Appearance panel's
    /// Contents (Characters) row. `None`: every item paints above the contents (see
    /// [`Self::contents_at`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contents_index: Option<usize>,
}

impl Appearance {
    /// Illustrator's "basic appearance": one fill below one stroke.
    pub fn basic(fill: Paint, stroke: Paint, width: f64) -> Self {
        Self { items: vec![AppearanceItem::Fill(FillLayer::new(fill)), AppearanceItem::Stroke(StrokeLayer::new(stroke, width))], ..Self::default() }
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
    /// This appearance for other art: the pen pressure its strokes were drawn with stays with them.
    pub fn without_pressure(mut self) -> Self {
        for it in &mut self.items {
            if let AppearanceItem::Stroke(s) = it {
                s.pressure = None;
            }
        }
        self
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
            None => self.insert_item(0, AppearanceItem::Fill(FillLayer::new(p))),
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
            && self.contents_index.is_none()
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
    /// The visible strokes that paint something.
    pub fn painted_strokes(&self) -> impl Iterator<Item = &StrokeLayer> {
        self.items.iter().filter_map(|i| match i {
            AppearanceItem::Stroke(s) if s.visible && !s.paint.is_none() => Some(s),
            _ => None,
        })
    }
    /// Largest distance the painted area can extend beyond any geometry (see
    /// [`StrokeLayer::reach`]); paths measure their own with [`Self::stroked_bounds`].
    pub fn outset(&self) -> f64 {
        self.painted_strokes().map(StrokeLayer::reach).fold(0.0, f64::max)
    }
    /// Scale stroke weights, dash lengths and dash offsets (Scale Strokes & Effects).
    pub fn scale_strokes(&mut self, s: f64) {
        for i in &mut self.items {
            if let AppearanceItem::Stroke(st) = i {
                st.width *= s;
                if let Some(d) = &mut st.dash {
                    d.scale(s);
                }
            }
        }
    }
    /// Scale the distance parameters of every effect (the object's and each fill's and stroke's)
    /// by `s` with `scale` (the effects catalogue knows which parameters are distances).
    pub fn scale_effects(&mut self, s: f64, scale: fn(&mut Effect, f64)) {
        let items = self.items.iter_mut().flat_map(|i| i.effects_mut().iter_mut());
        for e in self.effects.iter_mut().chain(items) {
            scale(e, s);
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
    pub(crate) fn gradients(&self) -> impl Iterator<Item = &vectorcraft_color::GradientPaint> {
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
                && ga.focal_point().distance(go.focal_point()) <= 1e-6
            {
                *ga = *go;
            }
            if let (Some(fa), Some(fo)) = (&mut g.freeform, &o.freeform)
                && fa.points.len() == fo.points.len()
                && fa.points.iter().zip(&fo.points).all(|(p, q)| p.at.distance(q.at) <= 1e-6)
            {
                fa.points.iter_mut().zip(&fo.points).for_each(|(p, q)| p.at = q.at);
            }
        }
        a == *other
    }
    /// Does a fill or stroke carry a gradient whose placement `placed` is (some: placed in document
    /// space; none: fitted to the bounds on each render)?
    fn has_gradient(&self, placed: bool) -> bool {
        self.items.iter().any(|i| match i {
            AppearanceItem::Fill(FillLayer { paint: Paint::Gradient(g), .. })
            | AppearanceItem::Stroke(StrokeLayer { paint: Paint::Gradient(g), .. }) => g.is_placed() == placed,
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
    /// Envelope Options → Distort Linear Gradients: bend the linear gradients of an object whose
    /// geometric bounds are `bounds` with a warp, given its local affine approximation at a point
    /// (taken at the middle of each gradient's vector). Unplaced ones are fixed to their fit first;
    /// other gradients stay as they are.
    pub fn warp_linear_gradients(&mut self, bounds: vectorcraft_geom::Rect, near: &dyn Fn(vectorcraft_geom::Point) -> vectorcraft_geom::Affine) {
        for i in &mut self.items {
            let (g, b) = match i {
                AppearanceItem::Fill(FillLayer { paint: Paint::Gradient(g), .. }) => (g, bounds),
                AppearanceItem::Stroke(s) => {
                    let b = s.paint_bounds(bounds);
                    let Paint::Gradient(g) = &mut s.paint else { continue };
                    (g, b)
                }
                _ => continue,
            };
            if g.gradient.kind != vectorcraft_color::GradientKind::Linear {
                continue;
            }
            g.pin(b);
            if let Some(geom) = g.geom {
                g.transform(near(geom.start.midpoint(geom.end)));
            }
        }
    }
    /// Map placed gradients through a warp, given its local affine approximation at a point (taken
    /// at each gradient's centre: the start of a radial, the middle of a linear vector). Freeform
    /// points each follow the warp at their own position.
    pub fn warp_gradients(&mut self, near: &dyn Fn(vectorcraft_geom::Point) -> vectorcraft_geom::Affine) {
        for g in self.gradients_mut() {
            let freeform = g.freeform.take();
            if let Some(geom) = g.geom {
                let c = if g.gradient.kind == vectorcraft_color::GradientKind::Radial { geom.start } else { geom.start.midpoint(geom.end) };
                g.transform(near(c));
            }
            g.freeform = freeform.map(|mut f| {
                f.map_points(|p| near(p) * p);
                f
            });
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

/// The contents slot: where a group's or layer's members (type: its characters) paint among the
/// object's own fills and strokes.
impl Appearance {
    /// How many items paint below the contents (the Contents/Characters row sits above them).
    pub fn contents_at(&self) -> usize {
        self.contents_index.unwrap_or(0).min(self.items.len())
    }
    /// Put the contents above the bottom `k` items (clamped; 0, all items above, is stored as `None`).
    pub fn set_contents_at(&mut self, k: usize) {
        let k = k.min(self.items.len());
        self.contents_index = (k > 0).then_some(k);
    }
    /// The items painted below the contents and those painted above them.
    pub fn split_contents(&self) -> (&[AppearanceItem], &[AppearanceItem]) {
        self.items.split_at(self.contents_at())
    }
    /// Insert `item` at paint-order index `at` (clamped). The contents stay between the same items;
    /// an item landing right above them joins the item below it (below the contents if that one is).
    pub fn insert_item(&mut self, at: usize, item: AppearanceItem) {
        let k = self.contents_at();
        let at = at.min(self.items.len());
        self.items.insert(at, item);
        if at < k || (at == k && k > 0) {
            self.set_contents_at(k + 1);
        }
    }
    /// Remove item `i` (if it exists), keeping the contents between the same items.
    pub fn remove_item(&mut self, i: usize) -> Option<AppearanceItem> {
        let k = self.contents_at();
        (i < self.items.len()).then(|| {
            let it = self.items.remove(i);
            if i < k {
                self.set_contents_at(k - 1);
            }
            it
        })
    }
    /// Move item `from` to paint-order index `to` (clamped) and return where it landed, or `None`
    /// when there is no item `from`. The contents stay between the same other items, and the moved
    /// item keeps its side of them when it lands right next to them.
    pub fn move_item(&mut self, from: usize, to: usize) -> Option<usize> {
        let k = self.contents_at();
        let below = from < k;
        let it = self.remove_item(from)?;
        let others_below = self.contents_at();
        let to = to.min(self.items.len());
        self.items.insert(to, it);
        let now_below = if below { to <= others_below } else { to < others_below };
        self.set_contents_at(others_below + usize::from(now_below));
        Some(to)
    }
}

/// The most fills, strokes or effects a stack may have to be matched against another's
/// ([`align`]); a larger stack shares nothing with the others.
const MAX_ALIGNED: usize = 1024;

/// Where each entry of `first` is in `other`: the entries equal in both (`same`), matched in their
/// stacking order (a longest common subsequence, so the matches never cross), by index into
/// `other`; `None` where `other` has no equal entry left for it.
pub fn align<T>(first: &[T], other: &[T], same: impl Fn(&T, &T) -> bool) -> Vec<Option<usize>> {
    let (n, m) = (first.len(), other.len());
    let mut out = vec![None; n];
    if n == 0 || m == 0 || n > MAX_ALIGNED || m > MAX_ALIGNED {
        return out;
    }
    // `len[i][j]`: the longest match of `first[i..]` and `other[j..]`.
    let w = m + 1;
    let mut len = vec![0u16; (n + 1) * w];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            let here = match (first.get(i), other.get(j)) {
                (Some(a), Some(b)) if same(a, b) => len.get((i + 1) * w + j + 1).copied().unwrap_or(0).saturating_add(1),
                _ => len.get((i + 1) * w + j).copied().unwrap_or(0).max(len.get(i * w + j + 1).copied().unwrap_or(0)),
            };
            if let Some(c) = len.get_mut(i * w + j) {
                *c = here;
            }
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        let at = |a: usize, b: usize| len.get(a * w + b).copied().unwrap_or(0);
        match (first.get(i), other.get(j)) {
            (Some(a), Some(b)) if same(a, b) && at(i, j) == at(i + 1, j + 1).saturating_add(1) => {
                if let Some(o) = out.get_mut(i) {
                    *o = Some(j);
                }
                i += 1;
                j += 1;
            }
            _ if at(i + 1, j) >= at(i, j + 1) => i += 1,
            _ => j += 1,
        }
    }
    out
}

/// What several objects' appearances share, as the Appearance panel lists a mixed selection: the
/// fills and strokes, and the object-level effects, that every one of them has an equal copy of, in
/// stacking order ([`align`]), each with where it is in every object's own stack.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SharedStack {
    /// Each shared fill or stroke, in the first object's paint order: its index in each object's
    /// `items` (the first object's first).
    pub items: Vec<Vec<usize>>,
    /// Each shared object-level effect: its index in each object's `effects`.
    pub effects: Vec<Vec<usize>>,
}

impl SharedStack {
    /// The rows `appearances` share (see [`SharedStack`]). One appearance shares all of itself.
    pub fn of(appearances: &[&Appearance]) -> Self {
        let Some((first, rest)) = appearances.split_first() else { return Self::default() };
        fn shared<T>(first: &[T], rest: Vec<&[T]>, same: impl Fn(&T, &T) -> bool + Copy) -> Vec<Vec<usize>> {
            let maps: Vec<Vec<Option<usize>>> = rest.iter().map(|o| align(first, o, same)).collect();
            (0..first.len())
                .filter_map(|i| {
                    let mut at = vec![i];
                    for m in &maps {
                        at.push((*m.get(i)?)?);
                    }
                    Some(at)
                })
                .collect()
        }
        Self {
            items: shared(&first.items, rest.iter().map(|a| a.items.as_slice()).collect(), |a, b| a == b),
            effects: shared(&first.effects, rest.iter().map(|a| a.effects.as_slice()).collect(), |a, b| a == b),
        }
    }

    /// Where the first object's fill or stroke `first` is in object `k`'s stack, when it is shared.
    pub fn item_in(&self, first: usize, k: usize) -> Option<usize> {
        self.items.iter().find(|at| at.first() == Some(&first))?.get(k).copied()
    }

    /// Where the first object's object-level effect `first` is in object `k`'s effects, when shared.
    pub fn effect_in(&self, first: usize, k: usize) -> Option<usize> {
        self.effects.iter().find(|at| at.first() == Some(&first))?.get(k).copied()
    }

    /// Is the first object's fill or stroke `i` shared?
    pub fn has_item(&self, i: usize) -> bool {
        self.items.iter().any(|at| at.first() == Some(&i))
    }

    /// Is the first object's object-level effect `k` shared?
    pub fn has_effect(&self, k: usize) -> bool {
        self.effects.iter().any(|at| at.first() == Some(&k))
    }
}

#[cfg(test)]
mod tests;
