//! CSS. SVG export's style attributes and sheets, and the CSS web pages style objects with
//! ([`css_rules`]: the CSS Properties panel and its exports), write each property they share in
//! the same way: the declarations of transparency, fonts and type spacing come from the functions
//! here, colours are `#rrggbb` (or `rgba()` where CSS has no separate opacity), gradients take the
//! stops SVG export writes, and numbers go through [`fmt_num`].

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use vectorcraft_color::{BlendMode, Color, GradientKind, GradientPaint, Paint};
use vectorcraft_doc::{AppearanceItem, CharStyle, Document, Justify, LineCap, LiveShape, Node, NodeId, NodeKind, StrokeAlign, StrokeLayer, TextKind};
use vectorcraft_effects::RasterFx;
use vectorcraft_geom::{Affine, PathData, Point, Rect};

use crate::export::sanitize_id;
use crate::fmt_num;

/// CSS declarations, in order: (property, value).
pub(crate) type Props = Vec<(&'static str, String)>;

/// A declaration block: `property:value;property:value`.
pub(crate) fn declarations<'a>(props: impl IntoIterator<Item = &'a (&'static str, String)>) -> String {
    props.into_iter().map(|(k, v)| format!("{k}:{v}")).collect::<Vec<_>>().join(";")
}

/// A CSS string.
pub(crate) fn css_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

pub(crate) fn blend_css(b: BlendMode) -> &'static str {
    match b {
        BlendMode::Normal => "normal",
        BlendMode::Darken => "darken",
        BlendMode::Multiply => "multiply",
        BlendMode::ColorBurn => "color-burn",
        BlendMode::Lighten => "lighten",
        BlendMode::Screen => "screen",
        BlendMode::ColorDodge => "color-dodge",
        BlendMode::Overlay => "overlay",
        BlendMode::SoftLight => "soft-light",
        BlendMode::HardLight => "hard-light",
        BlendMode::Difference => "difference",
        BlendMode::Exclusion => "exclusion",
        BlendMode::Hue => "hue",
        BlendMode::Saturation => "saturation",
        BlendMode::Color => "color",
        BlendMode::Luminosity => "luminosity",
    }
}

/// An object's opacity, blend mode and isolation.
pub(crate) fn transparency(n: &Node) -> Props {
    let mut p = Props::new();
    if n.opacity < 1.0 {
        p.push(("opacity", fmt_num(n.opacity as f64, 3)));
    }
    if n.blend != BlendMode::Normal {
        p.push(("mix-blend-mode", blend_css(n.blend).into()));
    }
    if n.isolate {
        p.push(("isolation", "isolate".into()));
    }
    p
}

/// The weight and italic of the face style `st` asks for: what embedded fonts' `@font-face`
/// rules and the characters using them say, so each face is told apart (Semibold from Bold).
pub(crate) fn font_descriptor(st: &CharStyle) -> (u16, bool) {
    let fs = st.font_style.to_ascii_lowercase();
    (vectorcraft_text::style_weight(&st.font_style).round().clamp(1.0, 1000.0) as u16, fs.contains("italic") || fs.contains("oblique"))
}

/// The font of character style `st`: family, size (the vertical scale; writers stretch the
/// horizontal one), weight and style, lengths written by `len`. `numeric_weight`: the face's
/// weight as a number (embedded faces are told apart by it), else `bold` for Bold and the number
/// for the other weights (Semibold 600, Light 300…), so they don't come back Bold or Regular.
pub(crate) fn font_props(st: &CharStyle, len: &dyn Fn(f64) -> String, numeric_weight: bool) -> Props {
    let mut p = Props::new();
    let fam = if st.font_family.contains(|c: char| c.is_whitespace() || c == ',') { format!("'{}'", st.font_family) } else { st.font_family.clone() };
    p.push(("font-family", fam));
    p.push(("font-size", len(st.size * st.v_scale / 100.0)));
    let (weight, italic) = font_descriptor(st);
    match weight {
        400 => {}
        700 if !numeric_weight => p.push(("font-weight", "bold".into())),
        w => p.push(("font-weight", w.to_string())),
    }
    if italic {
        p.push(("font-style", "italic".into()));
    }
    p
}

/// The spacing, kerning, OpenType features and decorations of character style `st`, lengths
/// written by `len`.
pub(crate) fn type_props(st: &CharStyle, len: &dyn Fn(f64) -> String) -> Props {
    let mut p = Props::new();
    // Tracking and manual kerning both add space after every character.
    let spacing = st.tracking + st.kerning.unwrap_or(0.0);
    if spacing != 0.0 {
        p.push(("letter-spacing", len(spacing / 1000.0 * st.size)));
    }
    if st.kerning.is_some() {
        // Manual kerning replaces the font's pair kerning.
        p.push(("font-kerning", "none".into()));
    }
    if !st.features.is_empty() {
        let v: Vec<String> = st
            .features
            .iter()
            .map(|t| match t.strip_prefix('-') {
                Some(off) => format!("\"{off}\" 0"),
                None => format!("\"{t}\" 1"),
            })
            .collect();
        p.push(("font-feature-settings", v.join(", ")));
    }
    match (st.underline, st.strikethrough) {
        (true, true) => p.push(("text-decoration", "underline line-through".into())),
        (true, false) => p.push(("text-decoration", "underline".into())),
        (false, true) => p.push(("text-decoration", "line-through".into())),
        _ => {}
    }
    p
}

/// A colour at `opacity`: `#rrggbb` when opaque, else `rgba()`.
fn color_css(c: &Color, opacity: f32) -> String {
    if opacity >= 1.0 {
        return c.to_hex();
    }
    let [r, g, b, _] = c.to_rgba8(1.0);
    format!("rgba({r}, {g}, {b}, {})", fmt_num(opacity.clamp(0.0, 1.0) as f64, 3))
}

// ---------- CSS for objects ----------

/// The units of the lengths [`css_rules`] writes (CSS Properties → Units). One CSS pixel is one
/// point, as in SVG export; the other units keep the document's physical sizes (72 points to the
/// inch).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CssUnits {
    #[default]
    Px,
    Pt,
    Mm,
    Cm,
    In,
}

impl CssUnits {
    pub const ALL: [CssUnits; 5] = [CssUnits::Px, CssUnits::Pt, CssUnits::Mm, CssUnits::Cm, CssUnits::In];

    /// The unit's CSS name.
    pub fn name(self) -> &'static str {
        match self {
            CssUnits::Px => "px",
            CssUnits::Pt => "pt",
            CssUnits::Mm => "mm",
            CssUnits::Cm => "cm",
            CssUnits::In => "in",
        }
    }

    /// `points` in these units (`0` without a unit).
    fn len(self, points: f64) -> String {
        let per_point = match self {
            CssUnits::Px | CssUnits::Pt => 1.0,
            CssUnits::Mm => 25.4 / 72.0,
            CssUnits::Cm => 2.54 / 72.0,
            CssUnits::In => 1.0 / 72.0,
        };
        match fmt_num(points * per_point, 3) {
            zero if zero == "0" => zero,
            v => format!("{v}{}", self.name()),
        }
    }
}

/// What [`css_rules`] writes (CSS Properties → Export Options). Deserializes from camelCase JSON
/// with every field optional.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CssOptions {
    pub units: CssUnits,
    /// `position: absolute` with `left` and `top` from the top-left corner of the object's
    /// artboard (the one its centre is on, else the first).
    pub position: bool,
    /// `width` and `height`.
    pub dimensions: bool,
    /// Rules for unnamed objects too, named after their kind (`.rectangle`); else unnamed objects
    /// are left out ([`CssSheet::skipped`]).
    pub unnamed: bool,
    /// Art CSS can't describe ([`CssRule::unsupported`]) as a `background-image` of its own
    /// picture ([`CssRule::image`]); else it is described as the box it sits in.
    pub rasterize: bool,
}

impl Default for CssOptions {
    fn default() -> Self {
        Self { units: CssUnits::Px, position: false, dimensions: true, unnamed: true, rasterize: false }
    }
}

/// The CSS rule of one object.
#[derive(Clone, Debug, PartialEq)]
pub struct CssRule {
    pub id: NodeId,
    /// `.name`: the object's name made a class name, else (unnamed) its kind's, unique in the sheet.
    pub selector: String,
    pub props: Vec<(&'static str, String)>,
    /// Why CSS can't describe the object exactly (other shapes, images, patterns, live effects…).
    pub unsupported: Option<&'static str>,
    /// With [`CssOptions::rasterize`], the file name of the PNG `background-image` refers to for
    /// unsupported art: the writer of the CSS file writes the object's picture under it (its
    /// visual bounds at 1 px per point, which `left`/`top`/`width`/`height` then describe).
    pub image: Option<String>,
}

impl CssRule {
    /// The rule as written: `.name {` and one declaration a line.
    pub fn text(&self) -> String {
        let mut s = format!("{} {{\n", self.selector);
        for (k, v) in &self.props {
            s.push_str(&format!("  {k}: {v};\n"));
        }
        s.push('}');
        s
    }
}

/// The CSS of a set of objects.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CssSheet {
    pub rules: Vec<CssRule>,
    /// Unnamed objects left out (without [`CssOptions::unnamed`]).
    pub skipped: usize,
    /// What the rules leave out or approximate (once each).
    pub warnings: Vec<String>,
}

impl CssSheet {
    /// The style sheet: the rules, a blank line between them.
    pub fn text(&self) -> String {
        self.rules.iter().map(CssRule::text).collect::<Vec<_>>().join("\n\n")
    }
}

/// The CSS of objects `ids` of `doc` (back to front) as web pages style elements: one rule per
/// object. Layers and unnamed groups without transparency or effects stand for the objects in
/// them; hidden objects, guides and template layers are left out.
///
/// A rectangle, rounded rectangle or ellipse gives its fill as `background-color` (gradients as
/// `linear-gradient()`/`radial-gradient()`), its stroke as `border` and its corners as
/// `border-radius`; type gives its first character style's font, colour and spacing and its
/// paragraph's alignment; shadows and glows give `box-shadow` (`text-shadow`), a Gaussian blur
/// `filter: blur()`. Other art is described as its box (see [`CssOptions::rasterize`]).
pub fn css_rules(doc: &Document, ids: &[NodeId], opts: &CssOptions) -> CssSheet {
    let mut w = Rules { doc, opts, sheet: CssSheet::default(), seen: HashSet::new(), names: HashSet::new() };
    for n in ids.iter().filter_map(|id| doc.node(*id)) {
        w.object(n);
    }
    w.sheet
}

struct Rules<'a> {
    doc: &'a Document,
    opts: &'a CssOptions,
    sheet: CssSheet,
    /// Objects written (an object and a group holding it may both be asked for).
    seen: HashSet<NodeId>,
    /// Class names given, lower case.
    names: HashSet<String>,
}

/// The CSS of one object's art, and why it is approximate.
#[derive(Default)]
struct Art {
    props: Props,
    unsupported: Option<&'static str>,
}

impl Art {
    /// Note that CSS can't describe the art exactly (the first reason stays).
    fn unsupported(&mut self, why: &'static str) {
        self.unsupported.get_or_insert(why);
    }
}

impl Rules<'_> {
    fn len(&self, points: f64) -> String {
        self.opts.units.len(points)
    }

    fn warn(&mut self, w: &str) {
        if !self.sheet.warnings.iter().any(|x| x == w) {
            self.sheet.warnings.push(w.to_string());
        }
    }

    fn object(&mut self, n: &Node) {
        if !n.visible {
            return;
        }
        let plain_group = n.name.is_none() && transparency(n).is_empty() && !n.appearance.effects.iter().any(|e| e.visible) && n.mask.is_none();
        match &n.kind {
            NodeKind::Layer { template: true, .. } | NodeKind::Path { guide: true, .. } => {}
            NodeKind::Layer { children, clip: false, .. } => children.iter().for_each(|c| self.object(c)),
            NodeKind::Group { children, clip: false } if plain_group => children.iter().for_each(|c| self.object(c)),
            _ => self.rule(n),
        }
    }

    /// A class name for `name` (an object's, or its kind's), unique in the sheet.
    fn class_name(&mut self, name: &str) -> String {
        let base = sanitize_id(name).replace('.', "_");
        let mut cand = base.clone();
        for i in 2.. {
            if self.names.insert(cand.to_lowercase()) {
                break;
            }
            cand = format!("{base}-{i}");
        }
        cand
    }

    fn rule(&mut self, n: &Node) {
        if !self.seen.insert(n.id) {
            return;
        }
        let name = match n.name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(name) => name.to_string(),
            None if self.opts.unnamed => n.kind_label().to_lowercase().replace(' ', "-"),
            None => {
                self.sheet.skipped += 1;
                return;
            }
        };
        let class = self.class_name(&name);
        let art = self.art(n);
        let image = art.unsupported.filter(|_| self.opts.rasterize).map(|_| format!("{class}.png"));
        // A picture covers what the object paints.
        let bounds = if image.is_some() { n.visual_bounds() } else { n.geometric_bounds() };
        let mut props = self.box_props(bounds);
        match &image {
            Some(file) => {
                props.push(("background-image", format!("url({file})")));
                // The picture has the object's opacity; its blend mode stays a property.
                props.extend(transparency(n).into_iter().filter(|(k, _)| *k != "opacity"));
            }
            None => {
                props.extend(art.props);
                props.extend(transparency(n));
                if art.unsupported.is_some() {
                    self.warn("art CSS can't describe exactly is written as its box (rasterize writes it as a picture)");
                }
            }
        }
        self.sheet.rules.push(CssRule { id: n.id, selector: format!(".{class}"), props, unsupported: art.unsupported, image });
    }

    /// Position and dimensions of a box (as the options ask), from its artboard's top-left.
    fn box_props(&self, b: Option<Rect>) -> Props {
        let mut p = Props::new();
        let Some(b) = b else { return p };
        if self.opts.position {
            let c = b.center();
            let ab = self.doc.artboards.iter().find(|a| a.rect.contains(c)).or_else(|| self.doc.artboards.first());
            let origin = ab.map_or(Point::ZERO, |a| Point::new(a.rect.x0, a.rect.y0));
            p.push(("position", "absolute".into()));
            p.push(("left", self.len(b.x0 - origin.x)));
            p.push(("top", self.len(b.y0 - origin.y)));
        }
        if self.opts.dimensions {
            p.push(("width", self.len(b.width())));
            p.push(("height", self.len(b.height())));
        }
        p
    }

    /// The art of `n` as CSS: paints, corners, type and effects.
    fn art(&mut self, n: &Node) -> Art {
        let mut art = Art::default();
        let mut radius = None;
        let text = match &n.kind {
            NodeKind::Path { path, live, .. } => {
                match corners(path, live.as_ref()) {
                    Some(Corners::Square) => {}
                    Some(Corners::Round(r)) => radius = Some(r.iter().map(|r| self.len(*r)).collect::<Vec<_>>().join(" ")),
                    Some(Corners::Ellipse) => radius = Some("50%".into()),
                    None => art.unsupported("a shape other than a rectangle or an ellipse"),
                }
                false
            }
            NodeKind::Compound { .. } => {
                art.unsupported("a compound path");
                false
            }
            NodeKind::Text(t) => {
                self.type_art(n, t, &mut art);
                true
            }
            _ => {
                art.unsupported("art other than shapes and type");
                false
            }
        };
        if !text {
            self.paints(n, &mut art);
        }
        art.props.extend(radius.map(|r| ("border-radius", r)));
        self.effects(n, text, &mut art);
        if n.mask.as_ref().is_some_and(|m| !m.disabled) {
            art.unsupported("an opacity mask");
        }
        art
    }

    /// A shape's fill (`background-*`) and stroke (`border*`).
    fn paints(&mut self, n: &Node, art: &mut Art) {
        let bounds = n.geometric_bounds().unwrap_or_default();
        let shown = |i: &&AppearanceItem| i.visible() && !i.paint().is_none() && !matches!(i, AppearanceItem::Stroke(s) if s.width <= 0.0);
        let items: Vec<&AppearanceItem> = n.appearance.items.iter().filter(shown).collect();
        let fills: Vec<_> = items.iter().filter_map(|i| if let AppearanceItem::Fill(f) = i { Some(f) } else { None }).collect();
        let strokes: Vec<_> = items.iter().filter_map(|i| if let AppearanceItem::Stroke(s) = i { Some(s) } else { None }).collect();
        if fills.len() > 1 || strokes.len() > 1 {
            art.unsupported("several fills or strokes");
        }
        if items.iter().any(|i| i.blend() != BlendMode::Normal || i.effects().iter().any(|e| e.visible)) {
            art.unsupported("blend modes or effects on a fill or stroke");
        }
        // The topmost of each, as the canvas shows it on top.
        if let Some(f) = fills.last() {
            match &f.paint {
                Paint::Solid { color, .. } => art.props.push(("background-color", color_css(color, f.opacity))),
                Paint::Gradient(g) => match self.gradient(g, bounds, f.opacity) {
                    Some(css) => art.props.push(("background-image", css)),
                    None => art.unsupported("a freeform gradient"),
                },
                Paint::Pattern { .. } => art.unsupported("a pattern"),
                Paint::None => {}
            }
        }
        if let Some(s) = strokes.last() {
            self.border(s, bounds, art);
        }
    }

    /// A stroke as a border: weight, style (dashed; dotted for round dots) and colour.
    fn border(&mut self, s: &StrokeLayer, bounds: Rect, art: &mut Art) {
        // Aligned strokes still draw as borders; arrowheads, profiles, brushes and fitted dashes don't.
        if !vectorcraft_effects::stroke::is_plain(&StrokeLayer { align: StrokeAlign::Center, ..s.clone() }) {
            art.unsupported("arrowheads, width profiles, brushes or dashes fitted to corners");
        }
        let style = match &s.dash {
            Some(d) if d.is_dashed() && s.cap == LineCap::Round && d.pattern.first() == Some(&0.0) => "dotted",
            Some(d) if d.is_dashed() => "dashed",
            _ => "solid",
        };
        let width = self.len(s.width);
        match &s.paint {
            Paint::Solid { color, .. } => art.props.push(("border", format!("{width} {style} {}", color_css(color, s.opacity)))),
            Paint::Gradient(g) => match self.gradient(g, s.paint_bounds(bounds), s.opacity) {
                Some(css) => {
                    art.props.push(("border", format!("{width} {style}")));
                    art.props.push(("border-image", format!("{css} 1")));
                }
                None => art.unsupported("a freeform gradient"),
            },
            Paint::Pattern { .. } => art.unsupported("a pattern"),
            Paint::None => {}
        }
    }

    /// A linear or radial gradient over `b` at `opacity`, with the stops SVG export writes
    /// (midpoints as stops of their own). `None` for freeform gradients.
    fn gradient(&mut self, g: &GradientPaint, b: Rect, opacity: f32) -> Option<String> {
        let geom = g.resolve(b);
        let stops = |pos: &dyn Fn(f32) -> f64| {
            let stops: Vec<String> =
                g.gradient.expanded().map(|(t, c, o, _)| format!("{} {}%", color_css(&c, o * opacity), fmt_num(pos(t) * 100.0, 2))).collect();
            stops.join(", ")
        };
        match g.gradient.kind {
            GradientKind::Linear => {
                // CSS runs the gradient along a line through the box's centre at its angle (0deg up,
                // 90deg right), long enough for the corners: the stops are placed on it.
                let v = geom.end - geom.start;
                let length = v.hypot();
                let (dir, angle) = if length > 1e-9 {
                    (v / length, v.x.atan2(-v.y).to_degrees().rem_euclid(360.0))
                } else {
                    (vectorcraft_geom::Vec2::new(1.0, 0.0), 90.0)
                };
                let line = (b.width() * dir.x).abs() + (b.height() * dir.y).abs();
                let from = (geom.start - b.center()).dot(dir);
                let pos = |t: f32| if line > 1e-9 { (from + t as f64 * length) / line + 0.5 } else { t as f64 };
                Some(format!("linear-gradient({}deg, {})", fmt_num(angle, 2), stops(&pos)))
            }
            GradientKind::Radial => {
                let r = (geom.end - geom.start).hypot();
                if geom.focal.is_some() {
                    self.warn("radial gradients start at their centre (an off-centre focal point is left out)");
                }
                let shape = if (geom.aspect - 1.0).abs() < 1e-9 {
                    format!("circle {}", self.len(r))
                } else {
                    format!("ellipse {} {}", self.len(r), self.len(r * geom.aspect))
                };
                let at = format!("at {} {}", self.len(geom.start.x - b.x0), self.len(geom.start.y - b.y0));
                Some(format!("radial-gradient({shape} {at}, {})", stops(&|t| t as f64)))
            }
            GradientKind::Freeform => None,
        }
    }

    /// Type: its first character style's font, colour and spacing, its paragraph's alignment.
    fn type_art(&mut self, n: &Node, t: &vectorcraft_doc::TextObject, art: &mut Art) {
        if matches!(t.kind, TextKind::OnPath { .. }) {
            art.unsupported("type on a path");
        }
        if n.appearance.items.iter().any(|i| i.visible() && !i.paint().is_none()) {
            art.unsupported("fills or strokes on the type object");
        }
        let st = t.first_style();
        if t.runs.iter().any(|r| r.style != st) {
            self.warn("type with several character styles is written in its first one");
        }
        let len = |v: f64| self.len(v);
        art.props.extend(font_props(&st, &len, false));
        match &st.fill {
            Paint::Solid { color, .. } => art.props.push(("color", color.to_hex())),
            Paint::None => art.props.push(("color", "transparent".into())),
            _ => art.unsupported("type painted with a gradient or a pattern"),
        }
        if st.has_stroke() {
            match &st.stroke {
                Paint::Solid { color, .. } => art.props.push(("-webkit-text-stroke", format!("{} {}", self.len(st.stroke_width), color.to_hex()))),
                _ => art.unsupported("type painted with a gradient or a pattern"),
            }
        }
        art.props.extend(type_props(&st, &len));
        if let Some(l) = st.leading {
            art.props.push(("line-height", self.len(l)));
        }
        let align = match t.para.justify {
            Justify::Left => None,
            Justify::Center => Some("center"),
            Justify::Right => Some("right"),
            Justify::JustifyLeft | Justify::JustifyCenter | Justify::JustifyRight | Justify::JustifyAll => Some("justify"),
        };
        art.props.extend(align.map(|a| ("text-align", a.to_string())));
        if st.all_caps {
            art.props.push(("text-transform", "uppercase".into()));
        }
    }

    /// Shadows and glows as `box-shadow` (type: `text-shadow`), a Gaussian blur as `filter`.
    fn effects(&mut self, n: &Node, text: bool, art: &mut Art) {
        let fx = &n.appearance.effects;
        if fx.iter().any(|e| e.visible && !vectorcraft_effects::is_raster(&e.id)) {
            art.unsupported("live effects");
        }
        let mut shadows = vec![];
        let mut filter = None;
        for f in vectorcraft_effects::raster_effects(fx) {
            match f {
                RasterFx::DropShadow { opacity, dx, dy, blur, color, .. } => {
                    shadows.push(format!("{} {} {} {}", self.len(dx), self.len(dy), self.len(blur), color_css(&color, opacity)))
                }
                RasterFx::OuterGlow { opacity, blur, color, .. } => shadows.push(format!("0 0 {} {}", self.len(blur), color_css(&color, opacity))),
                RasterFx::InnerGlow { opacity, blur, color, center: false, .. } if !text => {
                    shadows.push(format!("inset 0 0 {} {}", self.len(blur), color_css(&color, opacity)))
                }
                RasterFx::GaussianBlur { radius } => filter = Some(format!("blur({})", self.len(radius / 2.0))),
                RasterFx::InnerGlow { .. } | RasterFx::Feather { .. } => art.unsupported("inner glows or feathering"),
            }
        }
        if !shadows.is_empty() {
            art.props.push((if text { "text-shadow" } else { "box-shadow" }, shadows.join(", ")));
        }
        art.props.extend(filter.map(|f| ("filter", f)));
    }
}

/// How a box's corners round.
enum Corners {
    Square,
    /// Top-left, top-right, bottom-right, bottom-left (one value when all are the same).
    Round(Vec<f64>),
    Ellipse,
}

/// The corners of a path that is an upright rectangle (a live one keeps its radii) or a whole,
/// upright live ellipse; `None` for other shapes.
fn corners(path: &PathData, live: Option<&LiveShape>) -> Option<Corners> {
    let upright = |xf: &Affine| {
        let [_, b, c, _, _, _] = xf.as_coeffs();
        b.abs() < 1e-9 && c.abs() < 1e-9
    };
    match live {
        Some(LiveShape::Rectangle { radii, xf, .. }) if upright(xf) => {
            let [a, _, _, d, _, _] = xf.as_coeffs();
            let s = (a * d).abs().sqrt();
            if radii.iter().all(|r| *r <= 0.0) {
                Some(Corners::Square)
            } else if radii.iter().all(|r| (r - radii[0]).abs() < 1e-9) {
                Some(Corners::Round(vec![radii[0] * s]))
            } else {
                Some(Corners::Round(radii.iter().map(|r| r.max(0.0) * s).collect()))
            }
        }
        Some(LiveShape::Ellipse { pie, xf, .. }) if upright(xf) => {
            let sweep = (pie.1 - pie.0).abs();
            (sweep < 1e-6 || (sweep - 360.0).abs() < 1e-6).then_some(Corners::Ellipse)
        }
        Some(LiveShape::Rectangle { .. } | LiveShape::Ellipse { .. } | LiveShape::Polygon { .. } | LiveShape::Line { .. }) => None,
        None => is_upright_rectangle(path).then_some(Corners::Square),
    }
}

/// One closed subpath of four straight sides along the axes.
fn is_upright_rectangle(path: &PathData) -> bool {
    let ([sp], Some(b)) = (path.subpaths.as_slice(), path.bounds()) else { return false };
    let corner = |p: Point| ((p.x - b.x0).abs() < 1e-6 || (p.x - b.x1).abs() < 1e-6) && ((p.y - b.y0).abs() < 1e-6 || (p.y - b.y1).abs() < 1e-6);
    sp.closed
        && sp.anchors.len() == 4
        && (0..sp.segment_count()).all(|i| sp.segment_is_line(i))
        && sp.anchors.iter().all(|a| corner(a.p))
        && b.width() > 0.0
        && b.height() > 0.0
}
