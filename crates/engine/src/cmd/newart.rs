//! The new-art template: what the next object drawn looks like.
//!
//! The fill and stroke proxies (`Session::paint`) give new art its paints and stroke weight; the
//! template ([`PaintDefaults::appearance`]) gives it the rest: the Stroke panel's options (set by
//! `stroke.set` whatever is selected), a graphic style clicked with nothing selected (more fills
//! and strokes, effects, opacity and blend mode, and the link to the style) or, with the Appearance
//! panel's New Art Has Basic Appearance off, the whole appearance of the last selection. With the
//! option on (the default) an inherited appearance gives new art its top stroke's options only,
//! and a change of selection loads only the proxies. `paint.default` resets it, and
//! `appearance.setNewArt` sets its fill, stroke and stroke options whatever is selected.
//!
//! A document whose Default Graphic Style no longer has the shipped look (a New Document profile
//! redefined it) gives the reset that style's fills, strokes and effects instead, and `file.new`
//! resets new art when it opens such a document.

use std::borrow::Cow;

use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, ColorMode, DEFAULT_GRAPHIC_STYLE, Document, FillLayer, GraphicStyle, Node, NodeKind, StrokeLayer};
use vectorcraft_tools::PaintDefaults;

use super::gradient::unplaced;
use super::{CommandSpec, Result, Session, always, bad, cmd, switch_or};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "appearance.setNewArtBasic",
            "New Art Has Basic Appearance",
            ["Window", "Appearance"],
            None,
            "{on?: bool (omitted: toggle)} the Appearance panel option (the preference newArtBasic, on by default): on, new art takes one fill and stroke (with the Stroke panel's options), the fill, stroke and weight the selection shares when it last changed; off, the whole appearance of the last selected object (every fill, stroke and effect, opacity and blend mode) → {on}",
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
        cmd!(
            "appearance.setNewArt",
            "Set New Art Appearance",
            [],
            None,
            "{fill?: {color?, none?, swatch?, tint?, gradient?, keepModel?} (a paint as paint.setFill takes it), stroke?: (a paint, as fill), weight?: 0..1000 pt, cap?: butt|round|square, join?: miter|round|bevel, miterLimit?: 1..500, dash?: [lengths, each 0..1000000 pt, at most 64, not all 0]|null (null: a solid line), dashOffset?: -1000000..1000000 pt (with a dash pattern), fillOverprint?: bool, strokeOverprint?: bool} set the fill, stroke and stroke options the next object drawn gets, whatever is selected: the selection and its art, the active proxy and the recent colors stay as they are. A stroke painted on new art without one gets a 1 pt weight unless `weight` is given. A bad value is an error with nothing changed, and so is any change while New Art Has Basic Appearance is off and an object is selected (new art takes that object's appearance then) → {changed: whether the next object drawn looks different}",
            always,
            set_new_art
        ),
    ]
}

/// The most dash and gap lengths `appearance.setNewArt` takes.
const MAX_DASHES: usize = 64;
/// The largest dash, gap or dash offset `appearance.setNewArt` takes, in points.
const MAX_DASH_LENGTH: f64 = 1_000_000.0;

/// `appearance.setNewArt`: new art's fill, stroke and stroke options, every value checked before
/// anything changes.
fn set_new_art(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "appearance.setNewArt";
    let paint = |key: &str, fill: bool| -> Result<Option<Paint>> {
        let Some(v) = p.get(key) else { return Ok(None) };
        if !v.is_object() {
            return Err(bad(C, format!("{key} must be a paint object (color, none, swatch or gradient), got {v}")));
        }
        if v.get("tint").is_some() && v.get("swatch").is_none() {
            return Err(bad(C, format!("{key}: `tint` goes with a global or spot `swatch`")));
        }
        let given = super::paint::paint_from(s, v)?.ok_or_else(|| bad(C, format!("{key}: give color, none, swatch or gradient")))?;
        if !fill && matches!(&given, Paint::Gradient(g) if g.gradient.dither) && v.get("swatch").and_then(Value::as_str).is_none() {
            return Err(bad(C, "`dither` applies to gradient fills"));
        }
        Ok(Some(unplaced(&given)))
    };
    let (fill, stroke) = (paint("fill", true)?, paint("stroke", false)?);
    let number = |key: &str, min: f64, max: f64| -> Result<()> {
        match p.get(key) {
            None => Ok(()),
            Some(v) if v.as_f64().is_some_and(|n| n.is_finite() && (min..=max).contains(&n)) => Ok(()),
            Some(v) => Err(bad(C, format!("{key} must be a number from {min} to {max}, got {v}"))),
        }
    };
    number("weight", 0.0, 1000.0)?;
    number("miterLimit", 1.0, 500.0)?;
    number("dashOffset", -MAX_DASH_LENGTH, MAX_DASH_LENGTH)?;
    for key in ["cap", "join"] {
        if p.get(key).is_some_and(|v| !v.is_string()) {
            return Err(bad(C, format!("{key} must be a name")));
        }
    }
    match p.get("dash") {
        None | Some(Value::Null) => {}
        Some(Value::Array(a)) if a.len() > MAX_DASHES => return Err(bad(C, format!("dash takes at most {MAX_DASHES} lengths"))),
        Some(Value::Array(a)) => {
            if !a.iter().all(|v| v.as_f64().is_some_and(|n| n.is_finite() && (0.0..=MAX_DASH_LENGTH).contains(&n))) {
                return Err(bad(C, format!("each dash length must be a number from 0 to {MAX_DASH_LENGTH}")));
            }
            if !a.is_empty() && a.iter().all(|v| v.as_f64() == Some(0.0)) {
                return Err(bad(C, "dash needs a length that is not 0"));
            }
        }
        Some(v) => return Err(bad(C, format!("dash must be a list of lengths or null, got {v}"))),
    }
    let flag = |key: &str| -> Result<Option<bool>> {
        p.get(key).map(|v| v.as_bool().ok_or_else(|| bad(C, format!("{key} must be true or false, got {v}")))).transpose()
    };
    let (fill_overprint, stroke_overprint) = (flag("fillOverprint")?, flag("strokeOverprint")?);
    let options: serde_json::Map<String, Value> =
        ["weight", "cap", "join", "miterLimit", "dash", "dashOffset"].iter().filter_map(|k| Some((k.to_string(), p.get(*k)?.clone()))).collect();
    let change = super::stroke::StrokeChange::parse(&Value::Object(options), C)?;
    let dashed = match p.get("dash") {
        Some(Value::Array(a)) => !a.is_empty(),
        Some(_) => false,
        None => s.new_art().stroke().is_some_and(|st| st.dash.is_some()),
    };
    if p.get("dashOffset").and_then(Value::as_f64).is_some_and(|o| o != 0.0) && !dashed {
        return Err(bad(C, "dashOffset needs a dash pattern"));
    }
    let nothing = fill.is_none() && stroke.is_none() && change.is_empty() && fill_overprint.is_none() && stroke_overprint.is_none();
    if !nothing && !s.prefs.new_art_basic && !s.bucket_paints() && s.active().is_some_and(|st| !st.selection.subjects().is_empty()) {
        return Err(bad(
            C,
            "with New Art Has Basic Appearance off, new art takes the selected object's appearance: deselect it or turn the option on",
        ));
    }
    let look = |s: &Session| {
        let (opacity, blend, style) = s.new_art_transparency();
        (s.new_art(), opacity, blend, style.map(str::to_string))
    };
    let (before, was) = (s.paint.clone(), look(s));
    if let Some(f) = fill {
        s.paint.fill = f;
    }
    if let Some(st) = stroke {
        if !st.is_none() && s.paint.stroke.is_none() && s.paint.stroke_width == 0.0 {
            s.paint.stroke_width = 1.0;
        }
        s.paint.stroke = st;
    }
    if let Some(w) = change.weight {
        s.paint.stroke_width = w;
    }
    if !change.is_empty()
        && let Some(st) = s.new_art_stroke_mut()
    {
        change.apply(st);
    }
    if let Some(on) = fill_overprint {
        let t = s.new_art_template_mut();
        if t.fill().is_none() {
            t.items.insert(0, AppearanceItem::Fill(FillLayer::new(Paint::None)));
        }
        if let Some(f) = t.fill_mut() {
            f.overprint = on;
        }
    }
    if let Some(on) = stroke_overprint
        && let Some(st) = s.new_art_stroke_mut()
    {
        st.overprint = on;
    }
    let changed = look(s) != was;
    if !changed {
        s.paint = before;
    }
    Ok(json!({ "changed": changed }))
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
    /// the first selected object (so the last selection's once nothing is selected). Not while the
    /// Live Paint Bucket is the active tool: the fill and stroke are the colors it paints with.
    pub(crate) fn inherit_new_art(&mut self) {
        if self.prefs.new_art_basic || self.bucket_paints() {
            return;
        }
        let Some(st) = self.active() else { return };
        let Some(n) = st.selection.subjects().first().and_then(|id| st.doc.node(*id)) else { return };
        let style = super::style::linked_style(&st.doc, n).map(|g| g.name.clone());
        let (ap, opacity, blend) = (super::style::captured(n), n.opacity, n.blend);
        self.take_new_art(ap, opacity, blend, style, true);
    }

    /// After a command changed the selection, with New Art Has Basic Appearance on: the fill and
    /// stroke proxies and the weight take what the selection shows in them, so new art looks like
    /// it. A paint the selected objects don't share (a "?" proxy) stays as it was, and art without
    /// paint (a placed image that isn't colorized, whose None would make new art invisible) changes
    /// nothing. The template is left alone, and without a stroke to show the weight stays too.
    pub(crate) fn load_selection_paints(&mut self) {
        if !self.prefs.new_art_basic {
            return;
        }
        // A Live Paint group with edges and no faces shows only a stroke.
        let has_paint = [false, true].into_iter().any(|stroke| {
            self.proxy_source(stroke)
                .is_some_and(|(n, _)| matches!(n.kind, NodeKind::Text(_)) || !n.appearance.items.is_empty() || n.extras().colorize.is_some())
        });
        if !has_paint {
            return;
        }
        let (fill_mixed, stroke_mixed) = self.proxy_mixed();
        let (fill, stroke) = self.proxy_paints();
        let weight_mixed = self.active().is_some_and(|st| st.stroke_mixed().weight);
        let weight = self.shown_stroke().filter(|st| st.paint != Paint::None && !weight_mixed).map(|st| st.width);
        if !fill_mixed {
            self.paint.fill = unplaced(&fill);
        }
        if !stroke_mixed {
            self.paint.stroke = unplaced(&stroke);
        }
        if let Some(width) = weight {
            self.paint.stroke_width = width;
        }
    }
}

fn set_basic(s: &mut Session, p: &Value) -> Result<Value> {
    let on = switch_or(p, "on", !s.prefs.new_art_basic, "appearance.setNewArtBasic")?;
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

/// The defaults for new art reset (`paint.default`): white fill, 1 pt black stroke, no template;
/// in a document whose Default Graphic Style was redefined, that style's fills, strokes and effects
/// at full opacity, not linked to the style.
pub(crate) fn reset(s: &mut Session) {
    s.paint = PaintDefaults::default();
    let style = s.doc().ok().and_then(|st| redefined_default_style(&st.doc)).cloned();
    if let Some(g) = style {
        let ap = if g.unit_box { g.appearance } else { unplace_all(g.appearance) };
        s.take_new_art(ap, 1.0, BlendMode::Normal, None, false);
    }
}

/// `d`'s Default Graphic Style when its fills, strokes, effects or transparency differ from the
/// shipped style's (white fill, 1 pt black stroke), the shipped colors converted to `d`'s color mode
/// as a New Document profile's are.
pub(crate) fn redefined_default_style(d: &Document) -> Option<&GraphicStyle> {
    let g = d.graphic_styles.iter().find(|g| g.name == DEFAULT_GRAPHIC_STYLE)?;
    let same =
        |o: &GraphicStyle| (&o.appearance, o.opacity, o.blend, o.isolate, o.knockout) == (&g.appearance, g.opacity, g.blend, g.isolate, g.knockout);
    if same(&GraphicStyle::new(DEFAULT_GRAPHIC_STYLE, Appearance::default_art())) {
        return None;
    }
    if d.color_mode != ColorMode::Rgb {
        let mut shipped = Document::new_with_mode(1.0, 1.0, ColorMode::Rgb);
        shipped.graphic_styles.retain(|o| o.name == DEFAULT_GRAPHIC_STYLE);
        super::colormgmt::set_color_mode(&mut shipped, d.color_mode, true, None, super::colormgmt::Grays::Profile);
        if shipped.graphic_styles.first().is_some_and(same) {
            return None;
        }
    }
    Some(g)
}
