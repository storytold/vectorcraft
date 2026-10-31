//! The new-art template: what the next object drawn looks like.
//!
//! The fill and stroke proxies (`Session::paint`) give new art its paints and stroke weight; the
//! template ([`PaintDefaults::appearance`]) gives it the rest: the Stroke panel's options (set by
//! `stroke.set` whatever is selected), a graphic style clicked with nothing selected (more fills
//! and strokes, effects, opacity and blend mode, and the link to the style) or, with the Appearance
//! panel's New Art Has Basic Appearance off, the whole appearance of the last selection. With the
//! option on (the default) an inherited appearance gives new art its top stroke's options only.
//! `paint.default` resets it.

use std::borrow::Cow;

use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, FillLayer, GraphicStyle, Node, NodeKind, StrokeLayer};
use vectorcraft_tools::PaintDefaults;

use super::gradient::unplaced;
use super::{CommandSpec, Result, Session, always, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "appearance.setNewArtBasic",
            "New Art Has Basic Appearance",
            ["Window", "Appearance"],
            None,
            "{on?: bool (omitted: toggle)} the Appearance panel option (the preference newArtBasic, on by default): on, new art takes one fill and stroke (with the Stroke panel's options); off, the whole appearance of the last selected object (every fill, stroke and effect, opacity and blend mode) → {on}",
            always,
            set_basic
        ),
        cmd!(
            query "appearance.newArt",
            "New Art Appearance",
            [],
            None,
            "{} what the next object drawn gets → {basic: New Art Has Basic Appearance, appearance: {items, effects} (the model's JSON; placed gradients relative to the unit box), opacity: 0..100, blend, graphicStyle: the style it is linked to or null, inherited: taken from the last selection}",
            always,
            describe
        ),
    ]
}

/// `t` reduced to a basic appearance: an empty fill under its top stroke (options kept, without
/// its own transparency or effects).
fn basic_of(t: &Appearance) -> Appearance {
    let mut items = vec![AppearanceItem::Fill(FillLayer::new(Paint::None))];
    if let Some(st) = t.stroke() {
        items.push(AppearanceItem::Stroke(StrokeLayer { opacity: 1.0, blend: BlendMode::Normal, visible: true, effects: vec![], ..st.clone() }));
    }
    Appearance { items, ..Default::default() }
}

/// Paint `p` on the top fill (`fill`) or stroke of `ap`, creating it unless `p` is None. A paint
/// that only differs in where its gradient is placed keeps its place.
fn repaint(ap: &mut Appearance, fill: bool, p: Paint) {
    match ap.paint_at(None, fill) {
        Some(cur) if unplaced(cur) == p => {}
        Some(_) => {
            ap.set_paint_at(None, fill, p);
        }
        None if p.is_none() => {}
        None if fill => ap.set_fill(p),
        None => ap.set_stroke(p),
    }
}

/// `ap` with every gradient fitted to the art it lands on.
fn unplace_all(mut ap: Appearance) -> Appearance {
    for it in &mut ap.items {
        let p = it.paint_mut();
        *p = unplaced(p);
    }
    ap
}

/// How a new object looks ([`Session::new_art_look`]): its appearance and transparency and the
/// graphic style it is linked to.
pub(crate) struct NewArt {
    pub(crate) appearance: Appearance,
    opacity: f32,
    blend: BlendMode,
    style: Option<String>,
    /// From the template: placed gradients are relative to the unit box.
    unit_box: bool,
}

impl NewArt {
    /// A plain look: `appearance`, full opacity, no link.
    pub(crate) fn plain(appearance: Appearance) -> Self {
        Self { appearance, opacity: 1.0, blend: BlendMode::Normal, style: None, unit_box: false }
    }

    /// Give new object `n` (in `d`) this look: its placed gradients land relative to its bounds, and
    /// it links to the style (type keeps its own transparency).
    pub(crate) fn apply(&self, d: &mut vectorcraft_doc::Document, n: &mut Node) {
        n.appearance = self.appearance.clone().into();
        if self.unit_box
            && n.appearance.has_placed_gradient()
            && let Some(b) = n.geometric_bounds()
        {
            n.appearance.rebase_gradients(GraphicStyle::UNIT_BOX, b);
        }
        if matches!(n.kind, NodeKind::Text(_)) {
            return;
        }
        (n.opacity, n.blend) = (self.opacity, self.blend);
        n.extras_mut().graphic_style = self.style.as_deref().and_then(|name| d.graphic_style_index(name)).map(|i| d.graphic_style_id(i));
    }
}

impl Session {
    /// The template as new art takes it now (an inherited one reduced to basic while New Art Has
    /// Basic Appearance is on).
    fn template(&self) -> Option<Cow<'_, Appearance>> {
        let t = self.paint.appearance.as_ref()?;
        Some(if self.basic_only() { Cow::Owned(basic_of(t)) } else { Cow::Borrowed(t) })
    }

    /// Does an inherited template give new art only its basic fill and stroke?
    fn basic_only(&self) -> bool {
        self.paint.inherited && self.prefs.new_art_basic
    }

    /// The look of new art painted with `fill` and `stroke` (weight `width`) on its top fill and
    /// stroke: the template's, else one plain fill and stroke.
    pub(crate) fn new_art_look(&self, fill: Paint, stroke: Paint, width: f64) -> NewArt {
        let Some(t) = self.template() else { return NewArt::plain(Appearance::basic(fill, stroke, width)) };
        let mut appearance = t.into_owned();
        repaint(&mut appearance, true, fill);
        repaint(&mut appearance, false, stroke);
        if let Some(st) = appearance.stroke_mut() {
            st.width = width;
        }
        let (opacity, blend, style) = self.new_art_transparency();
        NewArt { appearance, opacity, blend, style: style.map(str::to_string), unit_box: true }
    }

    /// The appearance of the next object drawn (the Appearance panel shows it with nothing
    /// selected). Placed gradients are relative to the unit box.
    pub fn new_art(&self) -> Appearance {
        self.new_art_look(self.paint.fill.clone(), self.paint.stroke.clone(), self.paint.stroke_width).appearance
    }

    /// The opacity, blend mode and graphic style (name) of the next object drawn.
    pub fn new_art_transparency(&self) -> (f32, BlendMode, Option<&str>) {
        if self.basic_only() { (1.0, BlendMode::Normal, None) } else { (self.paint.opacity, self.paint.blend, self.paint.style.as_deref()) }
    }

    /// The stroke of the next object drawn (the Stroke panel shows it with nothing selected).
    pub fn new_art_stroke(&self) -> StrokeLayer {
        let template = self.paint.appearance.as_ref().and_then(Appearance::stroke);
        let mut st = template.cloned().unwrap_or_else(|| StrokeLayer::new(Paint::None, 0.0));
        (st.paint, st.width) = (self.paint.stroke.clone(), self.paint.stroke_width);
        st
    }

    /// The template for an edit (`stroke.set`): it becomes the user's own (an inherited one as new
    /// art takes it now), with a stroke to edit.
    fn new_art_template_mut(&mut self) -> &mut Appearance {
        let own = self.template().map(Cow::into_owned);
        if self.basic_only() {
            (self.paint.opacity, self.paint.blend, self.paint.style) = (1.0, BlendMode::Normal, None);
        }
        self.paint.inherited = false;
        let t = self.paint.appearance.insert(own.unwrap_or_else(|| Appearance::basic(Paint::None, Paint::None, 0.0)));
        if t.stroke().is_none() {
            t.set_stroke(Paint::None);
        }
        t
    }

    /// The template's top stroke for an edit (see [`Session::new_art_template_mut`], which always
    /// gives it one).
    pub(crate) fn new_art_stroke_mut(&mut self) -> Option<&mut StrokeLayer> {
        self.new_art_template_mut().stroke_mut()
    }

    /// A graphic style clicked with nothing selected: new art takes it (`add`: its fills, strokes
    /// and effects on top of the template's, unlinked).
    pub(crate) fn new_art_style(&mut self, g: &GraphicStyle, add: bool) {
        let ap = if g.unit_box { g.appearance.clone() } else { unplace_all(g.appearance.clone()) };
        if add {
            let t = self.new_art_template_mut();
            t.items.extend(ap.items);
            t.effects.extend(ap.effects);
            self.paint.style = None;
            return;
        }
        self.take_new_art(ap, g.opacity, g.blend, Some(g.name.clone()), false);
    }

    /// Make `ap` (placed gradients relative to the unit box) and this transparency the template,
    /// and its top fill, stroke and weight the proxies.
    fn take_new_art(&mut self, ap: Appearance, opacity: f32, blend: BlendMode, style: Option<String>, inherited: bool) {
        let p = &mut self.paint;
        (p.fill, p.stroke) = (unplaced(&ap.fill_paint()), unplaced(&ap.stroke_paint()));
        if let Some(st) = ap.stroke() {
            p.stroke_width = st.width;
        }
        (p.appearance, p.opacity, p.blend, p.style, p.inherited) = (Some(ap), opacity, blend, style, inherited);
    }

    /// After each command: with New Art Has Basic Appearance off, new art takes the appearance of
    /// the first selected object (so the last selection's once nothing is selected).
    pub(crate) fn inherit_new_art(&mut self) {
        if self.prefs.new_art_basic {
            return;
        }
        let Some(st) = self.active() else { return };
        let Some(n) = st.selection.subjects().first().and_then(|id| st.doc.node(*id)) else { return };
        let style = super::style::linked_style(&st.doc, n).map(|g| g.name.clone());
        let (ap, opacity, blend) = (super::style::captured(n), n.opacity, n.blend);
        self.take_new_art(ap, opacity, blend, style, true);
    }
}

fn set_basic(s: &mut Session, p: &Value) -> Result<Value> {
    let on = p.get("on").and_then(Value::as_bool).unwrap_or(!s.prefs.new_art_basic);
    s.prefs.new_art_basic = on;
    Ok(json!({ "on": on }))
}

fn describe(s: &mut Session, _: &Value) -> Result<Value> {
    let (opacity, blend, style) = s.new_art_transparency();
    Ok(json!({
        "basic": s.prefs.new_art_basic,
        "appearance": s.new_art(),
        "opacity": (opacity * 100.0).round(),
        "blend": blend.label(),
        "graphicStyle": style,
        "inherited": s.paint.inherited,
    }))
}

/// The defaults for new art reset (`paint.default`): white fill, 1 pt black stroke, no template.
pub(crate) fn reset(s: &mut Session) {
    s.paint = PaintDefaults::default();
}
