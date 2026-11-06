//! Art styles: objects with live effects a plain path can't hold, written as their drawn look, the
//! object itself on `%_` lines and `1 (style) XW`, and the styles they name in
//! `%AI9_BeginArtStyles` (see [`crate::ai_editing`] for the form). Each style is read back before it
//! is written ([`crate::import::read_art_style`]: the `.ai` reader's style parser and its reading of
//! a graphic style, with the object's own gradient): one that reads back differently is not
//! written, and the object is written as its drawn look alone.

use std::collections::HashMap;
use std::fmt::Write;
use std::sync::Arc;

use serde_json::Value;
use vectorcraft_color::{BlendMode, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, Effect, FillLayer, Knockout, Node, NodeId, NodeKind, StrokeLayer};
use vectorcraft_geom::{BezPath, FillRule, Rect, Shape};

use super::{Writer, Xy, blend_code, compound_path, hide};
use crate::ai_ops;
use crate::ps;

pub(in crate::ai_editing) const LARGE_CANVAS_STYLES: &str =
    "on a large canvas, objects with live effects are written as their drawn look in the editing data for other apps";
pub(in crate::ai_editing) const GRAPHIC_STYLES: &str = "graphic styles and links to them are left out of the editing data for other apps";

/// Does `doc` have graphic styles other than a new document's, or an object in its layers, opacity
/// masks or symbols linked to a graphic style?
pub(in crate::ai_editing) fn has_graphic_styles(doc: &Document) -> bool {
    fn linked(n: &Node) -> bool {
        n.extras().graphic_style.is_some()
            || n.extras().mask.as_ref().is_some_and(|m| linked(&m.art))
            || n.children().is_some_and(|c| c.iter().any(|m| linked(m)))
    }
    doc.graphic_styles != Document::new(1.0, 1.0).graphic_styles || doc.layers.iter().any(|l| linked(l)) || doc.symbols.iter().any(|s| linked(&s.art))
}

/// The objects of `doc`'s layers that may be written with an art style ([`Writer::styled_object`]),
/// by id: paths and compound paths with points and groups, with live effects, without Opacity &
/// Mask Define Knockout Shape (the reader takes an object's transparency from its look, which
/// doesn't carry it). A group's members are paths, compound paths or such groups, none with effects
/// or anything baking changes.
pub(in crate::ai_editing) fn styled_objects(doc: &Document) -> HashMap<NodeId, Arc<Node>> {
    fn walk(nodes: &[Arc<Node>], out: &mut HashMap<NodeId, Arc<Node>>) {
        for n in nodes {
            if candidate(n) {
                out.insert(n.id, n.clone());
                continue;
            }
            // Baking a container with effects of its own changes its members, which keep their ids:
            // they are written as baking left them.
            if let NodeKind::Layer { children, .. } | NodeKind::Group { children, .. } = &n.kind
                && !bakes_members(n)
            {
                walk(children, out);
            }
        }
    }
    let mut out = HashMap::new();
    walk(&doc.layers, &mut out);
    out
}

/// Does container `n` need baking for what it has itself, whatever its members hold?
fn bakes_members(n: &Node) -> bool {
    let mut own = n.clone();
    if let Some(children) = own.children_mut() {
        children.clear();
    }
    vectorcraft_effects::needs_bake(&own)
}

/// Whether `n` may be written with an art style ([`styled_objects`]).
fn candidate(n: &Node) -> bool {
    if n.appearance.effects.is_empty() || n.knockout_shape {
        return false;
    }
    match &n.kind {
        NodeKind::Path { path, guide: false, clipping: false, .. } => has_points(path),
        NodeKind::Compound { .. } => has_points(&compound_path(n)),
        NodeKind::Group { children, clip: false } => !n.is_compound_shape() && children.iter().all(|c| plain_member(c)),
        _ => false,
    }
}

/// Does `path` write anything ([`Writer::plain_object`] skips subpaths without points)?
fn has_points(path: &vectorcraft_geom::PathData) -> bool {
    path.subpaths.iter().any(|sp| !sp.anchors.is_empty())
}

/// A member of a group written with an art style: written as it is, and read back as it is.
fn plain_member(n: &Node) -> bool {
    let quiet = n.appearance.effects.is_empty() && n.appearance.items.iter().all(|i| i.effects().is_empty()) && !vectorcraft_effects::needs_bake(n);
    quiet
        && match &n.kind {
            NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } => true,
            NodeKind::Group { children, .. } => children.iter().all(|c| plain_member(c)),
            _ => false,
        }
}

/// The art styles objects name, each with its text after its name (`/ActiveStyle :` to `/Def ;`).
#[derive(Default)]
pub(in crate::ai_editing) struct Styles {
    /// Every style, in the order written: its name and text.
    known: Vec<(String, String)>,
    /// The name of each style by its text.
    by_text: HashMap<String, String>,
    next_anon: u64,
}

impl Styles {
    /// The name of an object style of `text`: a style with the same text already written, else a
    /// new one named `Anon` and a number.
    fn object_style(&mut self, text: String) -> String {
        if let Some(name) = self.by_text.get(&text) {
            return name.clone();
        }
        self.next_anon += 1;
        let name = format!("Anon {}", self.next_anon);
        self.by_text.insert(text.clone(), name.clone());
        self.known.push((name.clone(), text));
        name
    }
}

/// One style as the reader's section holds it: `/KnownStyle :`, `(name) /Name ,`, its text.
fn known_style(name: &str, text: &str) -> String {
    format!("/KnownStyle :\n{} /Name ,\n{text}", ps::bytes(name.as_bytes()))
}

/// The number transparency's `Xy` gives the blending mode a live effect's `mode` names (any case,
/// spaces and dashes ignored).
fn mode_code(mode: &str) -> Option<u8> {
    let key: String = mode.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase();
    BlendMode::ALL.iter().find(|m| format!("{m:?}").to_ascii_lowercase() == key).map(|m| blend_code(*m))
}

/// A filter's `Xy` line: `mode opacity isolate knockout shape Xy`.
fn xy_line(s: &mut String, xy: Xy) {
    let knockout = match xy.knockout {
        Knockout::Off => 0,
        Knockout::On => 1,
        Knockout::Neutral => 2,
    };
    let _ = write!(s, "{} ", blend_code(xy.blend));
    ps::push_num(s, f64::from(xy.opacity));
    let _ = writeln!(s, " {} {knockout} {} Xy", u8::from(xy.isolate), u8::from(xy.shape));
}

/// The line style of a paint style that isn't a stroke's.
const NO_LINE: &str = "0 J 0 j 1 w 10 M []0 d\n";

/// A basic filter's start: its name and kind numbers (`1 0` for an effect, `0 0` for the
/// style's own filters), `/Visible` and the dictionary's start.
fn basic(s: &mut String, name: &str, effect: bool, visible: bool) {
    let _ = write!(s, "/BasicFilter :\n{} {} 0 /Filter ,\n{} /Visible ,\n", ps::bytes(name.as_bytes()), u8::from(effect), u8::from(visible));
}

/// The filter that stands for the object's contents in its stack.
const CONDUIT: &str = "/BasicFilter :\n(Conduit Filter) 0 0 /Filter ,\n1 /Visible ,\n;\n /Part ,\n";

/// A basic filter's end.
const END_BASIC: &str = "; /Dict ;\n /Part ,\n";

/// The blend filter of transparency `xy`.
fn blend_filter(s: &mut String, xy: Xy) {
    basic(s, "Blend Style Filter", false, true);
    s.push_str("/Dictionary : /NotRecorded ,\n/BlendStyle : ");
    xy_line(s, xy);
    s.push_str(NO_LINE);
    s.push_str("0 XR\n/Def ; \n (BlendStyle) ,\n");
    s.push_str(END_BASIC);
}

/// A live effect's dictionary entries.
#[derive(Default)]
struct Entries(String);

impl Entries {
    fn real(&mut self, key: &str, v: f64) {
        ps::push_num(&mut self.0, v);
        let _ = writeln!(self.0, " /Real {} ,", ps::bytes(key.as_bytes()));
    }

    fn int(&mut self, key: &str, v: i64) {
        let _ = writeln!(self.0, "{v} /Int {} ,", ps::bytes(key.as_bytes()));
    }

    fn bool(&mut self, key: &str, v: bool) {
        let _ = writeln!(self.0, "{} /Bool {} ,", u8::from(v), ps::bytes(key.as_bytes()));
    }
}

/// `p`'s entry `key` as a number.
fn num(p: &Value, key: &str) -> Option<f64> {
    p.get(key)?.as_f64().filter(|v| v.is_finite())
}

/// `p`'s entry `key` as text, for comparing: lower case, letters and digits only.
fn word(p: &Value, key: &str) -> Option<String> {
    Some(p.get(key)?.as_str()?.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase())
}

/// `p`'s entry `key` as a switch.
fn flag(p: &Value, key: &str) -> Option<bool> {
    p.get(key)?.as_bool()
}

impl Writer<'_> {
    /// What `paint` writes with [`Self::color`] (a color, a spot or global color), as a string.
    fn color_ops(&mut self, paint: &Paint, stroke: bool) -> String {
        let saved = std::mem::take(&mut self.body);
        self.color(paint, stroke);
        std::mem::replace(&mut self.body, saved)
    }

    /// A fill style holding a color, as an effect's entry `key` or a fill's own.
    fn fill_style(&mut self, s: &mut String, color: vectorcraft_color::Color, key: &str) {
        let ops = self.color_ops(&Paint::solid(color), false);
        let _ = write!(s, "/FillStyle : 0 O\n{ops}0 1 0 0 0 Xy\n{NO_LINE}0 XR\n0 1 Xd\n/Def ; \n {} ,\n", ps::bytes(key.as_bytes()));
    }

    /// A fill's basic filter: a color, or a gradient placed on the object's outline (`on`: the
    /// object's own outline and rule), filling by the non-zero rule. `None` for other paint.
    fn fill_filter(&mut self, s: &mut String, f: &FillLayer, visible: bool, on: Option<(&BezPath, FillRule)>) -> Option<()> {
        if f.overprint {
            return None;
        }
        // A color comes before the paint state, a gradient's instance after it.
        let (color, gradient) = match &f.paint {
            Paint::Solid { .. } => (self.color_ops(&f.paint, false), String::new()),
            Paint::Gradient(_) => {
                let (path, _) = on?;
                let Paint::Gradient(placed) = self.placed(&f.paint, path.bounding_box()) else { return None };
                (String::new(), format!("Bb\n{}0 BB\n", self.gradient_bg(&placed, path)?))
            }
            Paint::Pattern { .. } | Paint::None => return None,
        };
        basic(s, "Fill Style Filter", false, visible);
        let _ = write!(
            s,
            "/Dictionary : /NotRecorded ,\n0 /Bool (UseEvenOdd) ,\n/FillStyle : 0 O\n{color}0 1 0 0 0 Xy\n{NO_LINE}0 XR\n0 1 Xd\n{gradient}"
        );
        s.push_str("/Def ; \n (FillStyle) ,\n");
        s.push_str(END_BASIC);
        Some(())
    }

    /// A stroke's basic filter: a color, its line style, and its width profile as the arrays
    /// `BSVariableWidthTValuesArray`, `BSLeftWidthsArray` and `BSRightWidthsArray`. `None` for
    /// other paint.
    fn stroke_filter(&mut self, s: &mut String, st: &StrokeLayer, visible: bool) -> Option<()> {
        if !matches!(st.paint, Paint::Solid { .. }) || st.overprint {
            return None;
        }
        let ops = self.color_ops(&st.paint, true);
        basic(s, "Stroke Style Filter", false, visible);
        s.push_str("2 /FillOrStroke ,\n/Dictionary : /NotRecorded ,\n/StrokeStyle : 0 R\n");
        s.push_str(&ops);
        s.push_str("0 1 0 0 0 Xy\n");
        ai_ops::line_style(s, st);
        s.push_str("0 XR\n1 0 Xd\n/Def ; \n (StrokeStyle) ,\n");
        if let Some(p) = &st.profile {
            let array = |s: &mut String, key: &str, values: &mut dyn Iterator<Item = f64>| {
                s.push_str("/Array :");
                for v in values {
                    s.push(' ');
                    ps::push_num(s, v);
                    s.push_str(" /Real ,");
                }
                let _ = writeln!(s, " ; {} ,", ps::bytes(key.as_bytes()));
            };
            array(s, "BSVariableWidthTValuesArray", &mut p.points.iter().map(|q| q.0));
            array(s, "BSLeftWidthsArray", &mut p.points.iter().map(|q| q.1));
            array(s, "BSRightWidthsArray", &mut p.points.iter().map(|q| q.2));
        }
        s.push_str(END_BASIC);
        Some(())
    }

    /// A gradient instance's focal point (`Bh`, radial gradients only) and its `flag (name) x y
    /// angle length a b c d tx ty 1 Bg` line for placed gradient `g` on `path`, as a fill's style
    /// holds them ([`Self::instance_head`]).
    fn gradient_bg(&mut self, g: &vectorcraft_color::GradientPaint, path: &BezPath) -> Option<String> {
        let saved = std::mem::take(&mut self.body);
        self.instance_head(g, false, path);
        let line = std::mem::replace(&mut self.body, saved);
        (!line.is_empty()).then_some(line)
    }

    /// An appearance item's basic filter. `None` for an item with transparency or effects of its
    /// own, which the reader doesn't read on an object.
    fn item_filter(&mut self, s: &mut String, item: &AppearanceItem, on: Option<(&BezPath, FillRule)>) -> Option<()> {
        if item.opacity() < 1.0 || item.blend() != BlendMode::Normal || !item.effects().is_empty() {
            return None;
        }
        match item {
            AppearanceItem::Fill(f) => self.fill_filter(s, f, item.visible(), on),
            AppearanceItem::Stroke(st) => self.stroke_filter(s, st, item.visible()),
        }
    }

    /// The text of a style of appearance `ap` and transparency `xy` (after its name): the chain
    /// of its stack (the fills and strokes that paint, the top one first, the contents among them
    /// where the appearance puts them), its effects and its blend filter. A gradient fill is
    /// placed on the outline of `on`, and a fill fills by its rule (`None`: the non-zero rule, on no
    /// object). `None` when a fill, stroke or effect isn't one written here.
    pub(super) fn style_text(&mut self, ap: &Appearance, xy: Xy, on: Option<(&BezPath, FillRule)>) -> Option<String> {
        let mut s = String::from(
            "/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
             /CompoundFilter :\n(Stack Style Filter) 0 0 /Filter ,\n1 /Visible ,\n",
        );
        let ci = ap.contents_index.unwrap_or(0).min(ap.items.len());
        let painting: Vec<(usize, &AppearanceItem)> = ap.items.iter().enumerate().filter(|(_, i)| !i.paint().is_none()).collect();
        let below = painting.iter().filter(|(at, _)| *at < ci).count();
        // Listed top first: the contents go above the `below` fills and strokes under them.
        for (written, (_, item)) in painting.iter().rev().enumerate() {
            if painting.len() - written == below {
                s.push_str(CONDUIT);
            }
            self.item_filter(&mut s, item, on)?;
        }
        if below == 0 {
            s.push_str(CONDUIT);
        }
        s.push_str(";\n /Part ,\n");
        for e in &ap.effects {
            s.push_str(&self.effect_filter(e)?);
        }
        blend_filter(&mut s, xy);
        s.push_str(";\n\n/Execution ;\n /Def ;\n");
        Some(s)
    }

    /// A live effect's basic filter, with the settings the reader reads
    /// ([`crate::import`]'s `.ai` reader, `style.rs`), in the units it reads them in. `None` for an
    /// effect, or a setting, that isn't written here.
    fn effect_filter(&mut self, e: &Effect) -> Option<String> {
        let p = vectorcraft_effects::merged_params(&e.id, &e.params);
        let mut d = Entries::default();
        let id = e.id.as_str();
        let name = if id.starts_with("stylize.") { self.stylize_entries(id, &p, &mut d)? } else { Self::path_entries(id, &p, &mut d)? };
        let mut s = String::new();
        basic(&mut s, name, true, e.visible);
        s.push_str("/Dictionary : /NotRecorded ,\n");
        s.push_str(&d.0);
        s.push_str(END_BASIC);
        Some(s)
    }

    /// A Stylize effect's entries ([`Self::effect_filter`]) and its filter's name.
    fn stylize_entries(&mut self, id: &str, p: &Value, d: &mut Entries) -> Option<&'static str> {
        Some(match id {
            "stylize.dropShadow" => {
                d.int("blnd", i64::from(mode_code(p.get("mode")?.as_str()?)?));
                d.real("opac", num(p, "opacity")? / 100.0);
                let darkness = word(p, "shade").as_deref() == Some("darkness");
                if darkness {
                    d.real("dark", num(p, "darkness")?);
                }
                d.real("vert", num(p, "y")?);
                d.real("horz", num(p, "x")?);
                d.real("blur", num(p, "blur")? / 2.0);
                let color = vectorcraft_effects::color_param(p, vectorcraft_color::Color::BLACK);
                self.fill_style(&mut d.0, color, "sclr");
                d.int("csrc", i64::from(darkness));
                "Adobe Drop Shadow" // brand-ok: the filter's name in the files
            }
            "stylize.innerGlow" => {
                d.int("blnd", i64::from(mode_code(p.get("mode")?.as_str()?)?));
                d.real("opac", num(p, "opacity")? / 100.0);
                d.real("blur", num(p, "blur")?);
                let color = vectorcraft_effects::color_param(p, vectorcraft_color::Color::WHITE);
                self.fill_style(&mut d.0, color, "gclr");
                d.int("gtyp", i64::from(word(p, "source").as_deref() == Some("edge")));
                "Adobe Inner Glow" // brand-ok: the filter's name in the files
            }
            "stylize.outerGlow" => {
                d.int("blnd", i64::from(mode_code(p.get("mode")?.as_str()?)?));
                d.real("opac", num(p, "opacity")? / 100.0);
                d.real("blur", num(p, "blur")? / 2.0);
                let color = vectorcraft_effects::color_param(p, vectorcraft_color::Color::BLACK);
                self.fill_style(&mut d.0, color, "sclr");
                "Adobe Outer Glow" // brand-ok: the filter's name in the files
            }
            "stylize.feather" => {
                d.real("Radius", num(p, "radius")?);
                "Adobe Fuzzy Mask" // brand-ok: the filter's name in the files
            }
            "stylize.roundCorners" => {
                d.real("radius", num(p, "radius")?);
                "Adobe Round Corners" // brand-ok: the filter's name in the files
            }
            "stylize.scribble" => {
                d.real("Angle", num(p, "angle")?);
                d.real("EdgeOverlap", num(p, "overlap")?);
                d.real("EdgeOverlapVariation", num(p, "overlapVariation")?);
                d.real("StrokeWidth", num(p, "strokeWidth")?);
                d.real("Scribbliness", num(p, "curviness")? / 100.0);
                d.real("ScribbleVariation", num(p, "curvinessVariation")? / 100.0);
                d.real("Spacing", num(p, "spacing")?);
                d.real("SpacingVariation", num(p, "spacingVariation")?);
                "Adobe Scribble Fill" // brand-ok: the filter's name in the files
            }
            _ => return None,
        })
    }

    /// Convert to Shape's, a Path effect's or Pathfinder › Add's entries ([`Self::effect_filter`])
    /// and its filter's name.
    fn path_entries(id: &str, p: &Value, d: &mut Entries) -> Option<&'static str> {
        Some(match id {
            "convertToShape.rectangle" | "convertToShape.roundedRectangle" | "convertToShape.ellipse" => {
                let shape = match id {
                    "convertToShape.rectangle" => 0,
                    "convertToShape.roundedRectangle" => 1,
                    _ => 2,
                };
                let relative = flag(p, "relative")?;
                d.int("Shape", shape);
                d.real("Absolute", if relative { 0.0 } else { 1.0 });
                d.real("AbsWidth", num(p, "width")?);
                d.real("AbsHeight", num(p, "height")?);
                d.real("RelWidth", num(p, "extraW")? / 2.0);
                d.real("RelHeight", num(p, "extraH")? / 2.0);
                if shape == 1 {
                    d.real("CornerRadius", num(p, "radius")?);
                }
                "Adobe Shape Effects" // brand-ok: the filter's name in the files
            }
            "path.offsetPath" => {
                d.real("ofst", num(p, "offset")?);
                let joins = match word(p, "joins")?.as_str() {
                    "round" => 0,
                    "bevel" => 1,
                    "miter" => 2,
                    _ => return None,
                };
                d.int("jntp", joins);
                d.real("mlim", num(p, "miterLimit")?);
                "Adobe Offset Path" // brand-ok: the filter's name in the files
            }
            "path.outlineObject" => "Adobe Outline Type",   // brand-ok: the filter's name in the files
            "path.outlineStroke" => "Adobe Outline Stroke", // brand-ok: the filter's name in the files
            "pathfinder.add" => {
                d.int("Command", 0);
                d.bool("ConvertCustom", true);
                d.bool("ExtractUnpainted", true);
                d.real("Mix", 0.5);
                d.real("Precision", 10.0);
                d.bool("RemovePoints", false);
                d.real("TrapAspect", 1.0);
                d.bool("TrapConvertCustom", false);
                d.real("TrapMaxTint", 1.0);
                d.bool("TrapReverse", false);
                d.real("TrapThickness", 0.25);
                d.real("TrapTint", 0.4);
                d.real("TrapTintTolerance", 0.05);
                "Adobe Pathfinder" // brand-ok: the filter's name in the files
            }
            _ => return None,
        })
    }

    /// The style object `orig` (`n` before baking) is written with; `None` when the reader wouldn't
    /// read the object as it is with it. A gradient fill is placed on `n`'s outline, as the object
    /// draws it. A style that isn't written leaves no gradient definition or warning behind.
    pub(super) fn object_style(&mut self, n: &Node, orig: &Node) -> Option<String> {
        if let NodeKind::Group { children, .. } = &orig.kind
            && bakes_members(orig)
            && !children.iter().all(|c| self.member_keeps_outline(c))
        {
            return None;
        }
        let graded = orig.appearance.items.iter().any(|i| matches!(i.paint(), Paint::Gradient(_)));
        let saved = (graded.then(|| self.gradient_mark()), self.warnings.len());
        let path = drawn_outline(n, orig);
        let rule = object_rule(orig);
        let text = self.style_text(&orig.appearance, Xy::of(orig), path.as_ref().map(|p| (p, rule))).filter(|t| object_reads(orig, t));
        let Some(text) = text else {
            if let Some(mark) = saved.0 {
                self.gradient_rollback(mark);
            }
            self.warnings.truncate(saved.1);
            return None;
        };
        Some(self.styles.object_style(text))
    }

    /// Is member `m` of a styled group whose effects change its members' outlines (and each of its
    /// own members) written on the outline those effects draw on: every fill and stroke as the
    /// object paints it ([`Self::direct`], [`Self::direct_stroke`]), not as filled outlines, brush
    /// art or pattern tiles, and no gradient that fits its object, which the effects would fit to
    /// the new outline?
    fn member_keeps_outline(&self, m: &Node) -> bool {
        let items_keep = m.appearance.items.iter().all(|i| match i {
            AppearanceItem::Fill(f) => f.paint.is_none() || self.direct(&f.paint),
            AppearanceItem::Stroke(s) => s.paint.is_none() || self.direct_stroke(s),
        });
        items_keep && m.children().is_none_or(|c| c.iter().all(|c| self.member_keeps_outline(c)))
    }

    /// Object `n` (as baking left it; `orig` before baking) with the style `name`: its lock and
    /// hidden state; its drawn look (a group of its vector art as baking left it, without shadows,
    /// glows or feathers, the object's transparency
    /// after the group's end, `0 0 Xd`, `6 () XW`); the object itself on `%_` lines (a path's or
    /// compound path's points painted with nothing, or with its gradient fill when its style has
    /// one; a group with its members), its name and opacity mask; then `1 (name) XW`.
    pub(super) fn styled_object(&mut self, n: &Node, orig: &Node, name: &str) {
        self.state(orig.locked, !orig.visible);
        let xy = Xy::of(orig);
        let was = self.in_look;
        self.in_look = true;
        self.line("u");
        let mut look = n.clone();
        (look.name, look.opacity, look.blend, look.isolate, look.knockout, look.knockout_shape) =
            (None, 1.0, BlendMode::Normal, false, Knockout::Neutral, false);
        if look.extras().mask.is_some() {
            look.extras_mut().mask = None;
        }
        match &look.kind {
            NodeKind::Group { children, clip: false } => {
                for c in children.clone() {
                    self.node(&c);
                }
            }
            _ => self.node(&look),
        }
        self.line("U");
        self.write_xy(xy);
        self.line("0 0 Xd\n6 () XW");
        self.in_look = was;
        // The object, on `%_` lines, in its own state (the look's members may have changed it).
        self.state(orig.locked, !orig.visible);
        let saved = std::mem::take(&mut self.body);
        self.hidden += 1;
        let fit = drawn_outline(n, orig).map(|p| p.bounding_box());
        match &orig.kind {
            NodeKind::Path { path, rule, .. } => self.styled_outline(orig, path, *rule, fit, xy),
            NodeKind::Compound { rule, .. } => self.styled_outline(orig, &compound_path(orig), *rule, fit, xy),
            NodeKind::Group { children, .. } => {
                self.line("u");
                for c in children {
                    self.node(c);
                }
                self.line("U");
            }
            _ => {}
        }
        self.dictionary(orig);
        self.hidden -= 1;
        let part = std::mem::replace(&mut self.body, saved);
        hide(&part, &mut self.body);
        self.block_end = usize::MAX;
        let _ = writeln!(self.body, "1 {} XW", ps::bytes(name.as_bytes()));
    }

    /// A styled path's or compound path's points: painted with nothing, or with its gradient fill
    /// (which its style's gradient fill is), placed on `fit` (the outline's bounds when `None`).
    fn styled_outline(&mut self, orig: &Node, path: &vectorcraft_geom::PathData, rule: vectorcraft_geom::FillRule, fit: Option<Rect>, xy: Xy) {
        let bounds = fit.unwrap_or_else(|| path.to_bezpath().bounding_box());
        let gradient = orig.appearance.items.iter().find_map(|i| match i {
            AppearanceItem::Fill(f) if matches!(f.paint, Paint::Gradient(_)) => Some(f),
            _ => None,
        });
        match gradient {
            Some(f) => {
                let fill = FillLayer { paint: self.placed(&f.paint, bounds), ..f.clone() };
                self.plain_object(path, rule, Some(&fill), None, xy.object());
            }
            None => self.plain_object(path, rule, None, None, xy.object()),
        }
    }

    /// The art styles written, in a non-printing block: `%AI9_BeginArtStyles`, each style,
    /// `%AI9_EndArtStyles`.
    pub(in crate::ai_editing) fn art_styles(&self, s: &mut String) {
        if self.styles.known.is_empty() {
            return;
        }
        s.push_str("%AI5_Begin_NonPrinting\nNp\n%AI9_BeginArtStyles\n");
        for (name, text) in &self.styles.known {
            s.push_str(&known_style(name, text));
        }
        s.push_str("%AI9_EndArtStyles\n%AI5_End_NonPrinting--\n");
    }
}

/// The outline a path's or compound path's style places a gradient fill on.
fn object_outline(n: &Node) -> Option<BezPath> {
    match &n.kind {
        NodeKind::Path { path, .. } => Some(path.to_bezpath()),
        NodeKind::Compound { .. } => Some(compound_path(n).to_bezpath()),
        _ => None,
    }
}

/// The outline a styled path or compound path draws: `n`'s, as baking left it, when it is still a
/// path or compound path, else `orig`'s.
fn drawn_outline(n: &Node, orig: &Node) -> Option<BezPath> {
    object_outline(orig)?;
    object_outline(n).or_else(|| object_outline(orig))
}

/// The fill rule of a path or compound path (a group's is the default).
fn object_rule(n: &Node) -> FillRule {
    match &n.kind {
        NodeKind::Path { rule, .. } | NodeKind::Compound { rule, .. } => *rule,
        _ => FillRule::NonZero,
    }
}

/// Does the reader read object `n` written with style text `text` as it is: a path or compound
/// path with a style it reads on any object, filled by the non-zero rule when the style fills it,
/// or a group with no fills or strokes of its own?
fn object_reads(n: &Node, text: &str) -> bool {
    let section = known_style("x", text);
    let own = n.appearance.fill().map(|f| &f.paint).filter(|p| matches!(p, Paint::Gradient(_)));
    let Some(mut read) = crate::import::read_art_style(section.as_bytes(), "x", own) else { return false };
    let group = matches!(n.kind, NodeKind::Group { .. });
    if !read.simple || (group && !read.appearance.items.is_empty()) || (!group && read.pathfinder) {
        return false;
    }
    if !group {
        let filled = n.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Fill(_)) && !i.paint().is_none());
        if filled && object_rule(n) == FillRule::EvenOdd {
            return false;
        }
        // A path's contents are nothing: the reader keeps the object's own.
        read.appearance.contents_index = n.appearance.contents_index;
    }
    same_appearance(&read.appearance, &n.appearance)
}

/// Do appearances `a` and `b` draw the same, as the reader reads one written from the other: the
/// fills and strokes that paint, within the written numbers' precision, and the effects with
/// their settings filled in from their defaults (blending modes in any case, colors as colors).
pub(in crate::ai_editing) fn same_appearance(a: &Appearance, b: &Appearance) -> bool {
    let norm = |ap: &Appearance| -> Value {
        let mut ap = ap.clone();
        let ci = ap.contents_index.unwrap_or(0).min(ap.items.len());
        let below = ap.items.iter().take(ci).filter(|i| !i.paint().is_none()).count();
        ap.items.retain(|i| !i.paint().is_none());
        ap.contents_index = (below > 0).then_some(below);
        for e in ap.effects.iter_mut().chain(ap.items.iter_mut().flat_map(|i| i.effects_mut().iter_mut())) {
            e.params = normalized_params(e);
        }
        serde_json::to_value(&ap).unwrap_or(Value::Null)
    };
    near(&norm(a), &norm(b))
}

/// Effect `e`'s settings with its defaults filled in, its blending mode in lower case and its color as
/// `#rrggbb`.
fn normalized_params(e: &Effect) -> Value {
    let mut p = vectorcraft_effects::merged_params(&e.id, &e.params);
    if let Some(m) = word(&p, "mode") {
        p["mode"] = Value::String(m);
    }
    if p.get("color").is_some() {
        p["color"] = Value::String(vectorcraft_effects::color_param(&p, vectorcraft_color::Color::BLACK).to_hex());
    }
    for key in ["shade", "source", "joins", "points", "orientation"] {
        if let Some(w) = word(&p, key) {
            p[key] = Value::String(w);
        }
    }
    p
}

/// How far apart two numbers may read in [`near`]: a number written to 4 decimals, then converted
/// from a fraction to a percentage (`5e-3`) or from radians to degrees (`3e-3`).
const NEAR: f64 = 6e-3;

/// JSON values equal but for numbers within the written numbers' precision.
fn near(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(x), Some(y)) => (x - y).abs() <= NEAR,
            _ => x == y,
        },
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(x, y)| near(x, y)),
        (Value::Object(x), Value::Object(y)) => x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| near(v, w))),
        _ => a == b,
    }
}
