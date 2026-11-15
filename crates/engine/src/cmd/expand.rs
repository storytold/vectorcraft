//! Object → Expand: live shapes, type and effects (Object), strokes (Stroke) and gradient and pattern
//! fills (Fill) into plain art in one undo step; blends, repeats and Intertwine groups become groups of their steps, of their instances and of what they draw. Placed documents (Object) become editable copies of the
//! art they show, their resources joining the document. Symbol instances (Object) become their art,
//! as Break Link to Symbol places it. A linear or radial gradient fill becomes a
//! gradient mesh, or solid strips / concentric ellipses, inside a clip group shaped like the object; a freeform
//! gradient becomes a mesh shaped like the object. A pattern fill becomes the tiles it paints inside a clip
//! group shaped like the object. Create Gradient Mesh shares the sampling. A dimension whose art
//! Expand changes becomes a plain group of that art, except one selected with art it measures,
//! which Expand leaves as it is.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::cms::Model;
use vectorcraft_color::freeform::spread_scale;
use vectorcraft_color::{BlendMode, Color, Gradient, GradientKind, GradientPaint, Paint};
use vectorcraft_doc::appearance::{AppearanceItem, FillLayer};
use vectorcraft_doc::live::GradientMesh;
use vectorcraft_doc::pattern::{PatternDef, place_art};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, NodeKind, Scaling, TextObject};
use vectorcraft_geom::{Affine, BezPath, PathData, Point, Rect, SubPath, Vec2, shapes};

use super::dimension::EndedLink;
use super::edit::selected_roots;
use super::menucmds::{run_raw, squash};
use super::object::as_clipping_path;
use super::pathops::{assemble, node_path, outlinable, outline_strokes, replace_leaves, styled_copy, with_transparency};
use super::{CommandSpec, EngineError, Result, Session, bad, cmd, has_selection, number_or, switch_or, text_param};

const C: &str = "object.expand";
/// Expand Gradient To → Specify … Objects: the default and the most.
pub const DEFAULT_STEPS: u32 = 255;
pub const MAX_STEPS: u32 = 1000;
/// Rows and columns of the mesh a freeform gradient expands to.
const FREEFORM_MESH: u32 = 8;
/// Columns (around the centre) of the mesh a radial gradient expands to.
const RADIAL_MESH_COLS: usize = 8;
/// Largest turn between a conic gradient's mesh columns, so their straight outer edges stay
/// close enough to a circle to reach past the box.
const CONIC_MESH_GAP: f64 = 1.0 / 32.0;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "object.expand",
            "Expand…",
            ["Object"],
            None,
            "{object?: true (symbol instances without effects (their fills' and strokes' included) and not in perspective → their art, as Break Link to Symbol places it (the instances that art holds stay instances), type → outlines (its own fills and strokes on paths of its whole outline, which Fill and Stroke expand), live shapes → paths, effects baked, blends → groups of their steps, repeats → groups of their instances, Intertwine groups → groups of their objects and of the parts drawn over the others (clip groups), envelopes → their distorted content, placed documents → editable copies of their art, as placing the file without link gives, its symbols, patterns, swatches and images joining the document), fill?: true (gradient fills → `gradient` art, pattern fills → their tiles in a clip group shaped like the object), stroke?: true (strokes → filled outlines, the stroked paths that baking effects leaves included), gradient?: \"objects\" (default: `steps` solid strips, concentric ellipses for radial gradients, wedges around the center for conic ones, nested squares for diamond ones) | \"mesh\" (a gradient mesh); both in a clip group shaped like the object (freeform gradients always become a mesh shaped like it), steps?: 1..1000 (255; a gradient that repeats or reflects gets the same whole number of them in each run, at least one, so a run starts on an object's edge; one that needs more than 256 runs across the box stays a gradient)} one undo step; a dimension whose art an option changes becomes a plain group of its art, except one selected with art it measures, directly or in groups without fills, strokes or effects of their own: its lines and label stay as they are, and if the new art no longer fits its link it stays where it was, no longer linked → {ids}",
            has_selection,
            expand
        ),
        cmd!(
            query "object.expand.info",
            "Expand Options",
            [],
            None,
            "{} what each Expand option would change in the selection → {object, fill, stroke, gradient, appearance, symbolArt?: {fill, stroke, gradient}} (true: something to expand; gradient: a gradient fill that Fill expands (in type, on the outline Object makes of a character, or of the whole type for the type object's own fills), which Expand Gradient To applies to; appearance: appearance attributes Expand Appearance expands first, for which Object › Expand… is dimmed in the menus; symbolArt, when the selection holds symbol instances that Object expands: true when only their art holds it, which Fill and Stroke reach only with Object on)",
            has_selection,
            |s, _| {
                let roots = selected_roots(s)?;
                let e = Expandable::of(&super::dimension::without_kept(&s.doc()?.doc, &roots), &roots);
                let mut info =
                    json!({"object": e.object(), "fill": e.fill, "stroke": e.stroke, "gradient": e.gradient, "appearance": has_appearance(s)});
                if e.symbols {
                    let [fill, stroke, gradient] = e.symbol_art;
                    info["symbolArt"] = json!({"fill": fill, "stroke": stroke, "gradient": gradient});
                }
                Ok(info)
            }
        ),
    ]
}

/// Has the selection appearance attributes that Expand Appearance expands (effects, several fills
/// or strokes, a fill or stroke with its own opacity, blend mode or effects, a brush)? Object ›
/// Expand… is dimmed in the menus for them until Expand Appearance has turned them into plain art
/// (#3089); `object.expand` itself still bakes them.
pub fn has_appearance(s: &Session) -> bool {
    let (Ok(st), Ok(roots)) = (s.doc(), selected_roots(s)) else { return false };
    // A dimension selected with the art it measures is left as it is.
    let kept = super::dimension::with_their_art(&st.doc, &roots);
    roots.iter().filter_map(|r| st.doc.node(*r)).any(|n| super::effectcmd::expandable_except(n, &kept))
}

/// Every placed document in `roots` of `d` (at any depth) replaced by an editable copy of the art it
/// shows, keeping its id and place; the resources the art uses join `d` (a name `d` uses for
/// something else gets a free variant).
fn expand_placed_under(d: &mut Document, roots: &[NodeId]) -> Result<()> {
    let mut ids = vec![];
    for n in roots.iter().filter_map(|r| d.node(*r)) {
        n.walk(&mut |c| {
            if matches!(c.kind, NodeKind::PlacedDocument(_)) {
                ids.push(c.id);
            }
        });
    }
    ids.into_iter().try_for_each(|id| super::place::document::expand(d, id, C))
}

/// Expand Gradient To.
#[derive(Clone, Copy, Debug, PartialEq)]
enum GradientTo {
    Mesh,
    Objects(u32),
}

impl GradientTo {
    fn parse(p: &Value) -> Result<Self> {
        let steps = number_or(p, "steps", DEFAULT_STEPS as f64, C)?.round();
        if !(1.0..=MAX_STEPS as f64).contains(&steps) {
            return Err(bad(C, format!("steps must be 1..{MAX_STEPS}")));
        }
        match text_param(p, "gradient", C)?.unwrap_or("objects") {
            "objects" => Ok(Self::Objects(steps as u32)),
            "mesh" => Ok(Self::Mesh),
            _ => Err(bad(C, "gradient must be \"objects\" or \"mesh\"")),
        }
    }
}

/// What the selection holds for each Expand option.
#[derive(Default)]
struct Expandable {
    text: bool,
    live: bool,
    effects: bool,
    fill: bool,
    /// Gradient fills (the ones Expand Gradient To applies to), a part of `fill`.
    gradient: bool,
    stroke: bool,
    /// An envelope (Object expands it into its distorted content).
    envelope: bool,
    /// Live blends (Object expands them into groups of their steps).
    blends: bool,
    /// Placed documents (Object expands them into the art they show).
    placed: bool,
    /// Repeats (Object expands them into groups of their instances).
    repeats: bool,
    /// Intertwine groups (Object expands them into groups of what they draw).
    intertwines: bool,
    /// Symbol instances (Object expands them into their art, as Break Link to Symbol does).
    symbols: bool,
    /// Fill, stroke and gradient that only the art of those symbol instances holds: Fill and Stroke
    /// reach them only when Object expands the instances.
    symbol_art: [bool; 3],
}

impl Expandable {
    fn of(d: &Document, roots: &[NodeId]) -> Self {
        let mut e = Self::default();
        for n in roots.iter().filter_map(|r| d.node(*r)) {
            e.add(d, n);
        }
        let own = [e.fill, e.stroke, e.gradient];
        // The Fill and Stroke options also reach the art of the symbol instances that Object
        // expands: a dynamic instance's art with its own changes, and each other symbol's art once.
        let instances = super::brushsym::expandable_instances(d, roots);
        e.symbols = !instances.is_empty();
        let mut seen = std::collections::HashSet::new();
        for inst in instances.iter().filter_map(|id| d.node(*id)) {
            let NodeKind::SymbolInstance { symbol, .. } = &inst.kind else { continue };
            let Some(sym) = d.symbols.iter().find(|s| &s.name == symbol) else { continue };
            let mut a = Self::default();
            if !inst.extras().symbol_overrides.is_empty() {
                a.add(d, &vectorcraft_doc::symbol_overrides::dynamic_art(&sym.art, inst));
            } else if seen.insert(symbol.clone()) {
                a.add(d, &sym.art);
            }
            e.fill |= a.fill;
            e.gradient |= a.gradient;
            e.stroke |= a.stroke;
        }
        e.symbol_art = [e.fill && !own[0], e.stroke && !own[1], e.gradient && !own[2]];
        e
    }

    /// Adds what `n` and the nodes under it hold.
    fn add(&mut self, d: &Document, n: &Node) {
        let e = self;
        n.walk(&mut |c| {
            let shape = matches!(c.kind, NodeKind::Path { .. } | NodeKind::Compound { .. });
            e.text |= matches!(c.kind, NodeKind::Text(_));
            e.live |= matches!(c.kind, NodeKind::Path { live: Some(_), .. });
            e.effects |= !c.appearance.effects.is_empty();
            e.envelope |= matches!(c.kind, NodeKind::Envelope { .. });
            e.blends |= matches!(c.kind, NodeKind::Blend { .. });
            e.placed |= matches!(c.kind, NodeKind::PlacedDocument(_));
            e.repeats |= matches!(c.kind, NodeKind::Repeat(_));
            e.intertwines |= matches!(c.kind, NodeKind::Intertwine { .. });
            // Type's outlines take its characters' stroke and fill, and a path of its whole outline
            // takes the type object's own fills and strokes (see `type.createOutlines`).
            // Any stroke Outline Stroke outlines, not only the topmost one.
            let own_stroke = c.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Stroke(st) if outlinable(st)));
            let own_pattern = c.appearance.items.iter().any(|i| pattern_fill(d, i).is_some());
            let own_gradient = c.appearance.items.iter().any(|i| gradient_fill(i).is_some());
            // Type without an outline (empty, or only spaces) gets no path for its own fills and strokes.
            let whole = match &c.kind {
                NodeKind::Text(t) if own_stroke || own_pattern || own_gradient => text_whole_outline(t),
                _ => None,
            };
            e.stroke |= match &c.kind {
                NodeKind::Text(t) => (own_stroke && whole.is_some()) || t.runs.iter().any(|r| r.style.has_stroke()),
                _ => shape && own_stroke,
            };
            // Once one is found, the next path is not measured.
            e.gradient = e.gradient
                || match &c.kind {
                    NodeKind::Text(t) => {
                        text_gradient_expands(d, t)
                            || (own_gradient && whole.as_ref().is_some_and(|(path, tb)| text_object_gradient_expands(d, c, path, *tb)))
                    }
                    _ => shape && gradient_expands(d, c),
                };
            e.fill |= e.gradient;
            e.fill |= match &c.kind {
                NodeKind::Text(t) => {
                    (own_pattern && whole.is_some())
                        || t.runs.iter().any(|r| matches!(&r.style.fill, Paint::Pattern { pattern, .. } if d.pattern(pattern).is_some()))
                }
                _ => shape && own_pattern,
            };
        });
    }

    fn object(&self) -> bool {
        self.text || self.live || self.effects || self.envelope || self.blends || self.placed || self.repeats || self.intertwines || self.symbols
    }
}

/// The gradient of a visible gradient fill.
fn gradient_fill(i: &AppearanceItem) -> Option<(&FillLayer, &GradientPaint)> {
    match i {
        AppearanceItem::Fill(f) if f.visible => match &f.paint {
            Paint::Gradient(g) => Some((f, g)),
            _ => None,
        },
        _ => None,
    }
}

/// Whether path `l` has a gradient fill that Fill expands ([`expand_fills`]): one that repeats or
/// reflects stays a gradient when its box needs more runs than are written ([`padded_on`]). For a
/// path that live-effect plug-ins paint, whether a piece of the art they paint has one
/// ([`expand_pieces`]).
fn gradient_expands(d: &Document, l: &Node) -> bool {
    if !l.appearance.items.iter().any(|i| gradient_fill(i).is_some()) {
        return false;
    }
    if !vectorcraft_render::effects::has_painted_effects(l) {
        return piece_gradient_expands(d, l);
    }
    let Some(art) = vectorcraft_render::effects::reshape(Some(d), l, None) else { return false };
    let mut any = false;
    art.walk(&mut |c| {
        any = any
            || (matches!(c.kind, NodeKind::Path { .. } | NodeKind::Compound { .. })
                && !c.is_ruler_guide()
                && c.appearance.items.iter().any(|i| gradient_fill(i).is_some())
                && piece_gradient_expands(d, c));
    });
    any
}

/// Whether type `t` has a gradient fill that Fill expands once Object has outlined it: one of the
/// paths `type.createOutlines` makes of its glyphs, ruby glyphs and underline and strikethrough
/// bars, each painted as its run with its unplaced gradients fitted to the layout's bounds, has one
/// ([`gradient_expands`] on that path). Inline graphics and color glyphs become other art. Leaves
/// the document as it is and stops at the first such path.
fn text_gradient_expands(d: &Document, t: &TextObject) -> bool {
    if !t.runs.iter().any(|r| matches!(r.style.fill, Paint::Gradient(_))) {
        return false;
    }
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
    let first = t.first_style();
    let expands = |run: usize, outline: &BezPath| {
        let style = t.runs.get(run).map_or(&first, |r| &r.style);
        if !matches!(style.fill, Paint::Gradient(_)) {
            return false;
        }
        let path = PathData::from_bezpath(outline);
        if path.is_empty() {
            return false;
        }
        let mut piece = Node::path(NodeId(u64::MAX), path, Appearance::default());
        vectorcraft_render::effects::paint_type_piece(&mut piece, style, lay.bounds, t.xf);
        gradient_expands(d, &piece)
    };
    let inline = |gi: usize| lay.inlines.iter().find(|i| i.glyph == gi).is_some_and(|i| t.runs.get(i.run).is_some_and(|r| r.inline.is_some()));
    lay.glyphs.iter().enumerate().any(|(gi, g)| !inline(gi) && g.image.is_none() && g.colour.is_none() && expands(g.run, &g.outline))
        || lay.ruby.iter().any(|g| expands(g.run, &g.outline))
        || vectorcraft_text::decorations(&lay, vectorcraft_text::FontDb::global(), t).iter().any(|(run, bar)| expands(*run, bar))
}

/// The path of type `t`'s whole outline in the document (glyphs, ruby glyphs and underline and
/// strikethrough bars), which `type.createOutlines` paints with the type object's own fills and
/// strokes, and the layout's bounds there; `None` when the type has no outline.
fn text_whole_outline(t: &TextObject) -> Option<(PathData, Rect)> {
    let db = vectorcraft_text::FontDb::global();
    let lay = vectorcraft_text::layout(db, t);
    let mut all = BezPath::new();
    for g in lay.glyphs.iter().chain(&lay.ruby) {
        all.extend(g.outline.iter());
    }
    for (_, bar) in vectorcraft_text::decorations(&lay, db, t) {
        all.extend(bar.iter());
    }
    let path = PathData::from_bezpath(&all).transformed(t.xf);
    (!path.is_empty()).then(|| (path, t.xf.transform_rect_bbox(lay.bounds)))
}

/// Whether type node `n` has a gradient fill of its own (not its characters') that Fill expands
/// once Object has outlined it: [`gradient_expands`] on `path`, the type's whole outline
/// ([`text_whole_outline`]), with its unplaced gradients fixed to `tb`, the layout's bounds in the
/// document. Leaves the document as it is.
fn text_object_gradient_expands(d: &Document, n: &Node, path: &PathData, tb: Rect) -> bool {
    let mut appearance = Appearance { items: n.appearance.items.clone(), ..Default::default() };
    appearance.pin_gradients(tb);
    gradient_expands(d, &Node::path(NodeId(u64::MAX), path.clone(), appearance))
}

/// [`gradient_expands`] for a path that no live-effect plug-in paints ([`expand_piece_fills`]).
fn piece_gradient_expands(d: &Document, l: &Node) -> bool {
    if !l.appearance.items.iter().filter_map(gradient_fill).any(|(_, g)| g.gradient.is_spread()) {
        return l.appearance.items.iter().any(|i| gradient_fill(i).is_some());
    }
    let reshaped = reshaped(d, l);
    let l = reshaped.as_ref().unwrap_or(l);
    let Some((path, _)) = node_path(l) else { return false };
    let Some(bounds) = path.bounds() else { return false };
    l.appearance.items.iter().filter(|i| gradient_fill(i).is_some()).any(|item| {
        fill_shape(d, l, item, &path, bounds).is_some_and(|(_, _, b, moved)| gradient_fill(&moved).is_some_and(|(_, g)| padded_on(g, b).is_some()))
    })
}

/// The definition and placement of a visible pattern fill whose pattern `d` defines.
fn pattern_fill<'a, 'd>(d: &'d Document, i: &'a AppearanceItem) -> Option<(&'a FillLayer, &'d PatternDef, Affine)> {
    match i {
        AppearanceItem::Fill(f) if f.visible => match &f.paint {
            Paint::Pattern { pattern, xf } => Some((f, d.pattern(pattern)?, *xf)),
            _ => None,
        },
        _ => None,
    }
}

fn expand(s: &mut Session, p: &Value) -> Result<Value> {
    let scope = s.begin_undo_scope();
    let result = expand_steps(s, p);
    if let Some(scope) = scope {
        s.end_undo_scope(scope, result.is_err());
    }
    result
}

fn expand_steps(s: &mut Session, p: &Value) -> Result<Value> {
    let object = switch_or(p, "object", true, C)?;
    let fill = switch_or(p, "fill", true, C)?;
    let stroke = switch_or(p, "stroke", true, C)?;
    let to = GradientTo::parse(p)?;
    let from = s.doc()?.history.undo.len();
    // Before anything expands: outlining the art a dimension measures would leave the dimension unlinked.
    let before = s.doc()?.doc.clone();
    let selected = s.doc()?.selection.objects.clone();
    // A dimension selected with the art it measures keeps its own lines and label: it is out of the
    // document while the art expands, so no step below reaches it.
    let roots = selected_roots(s)?;
    let mut lifted = vec![];
    if !super::dimension::with_their_art(&before, &roots).is_empty() {
        lifted = s.edit("Expand", |d, _| Ok(super::dimension::lift(d, &roots)))?;
    }
    let kept: Vec<NodeId> = lifted.iter().map(super::dimension::Lifted::id).collect();
    let mut e = Expandable::of(&s.doc()?.doc, &selected_roots(s)?);
    let mut ended = vec![];
    end_dimension_links(s, &mut ended)?;
    let rev0 = s.doc()?.revision;
    // Placed documents first: their art (type, strokes, gradients) expands below.
    if object && e.placed {
        let roots = selected_roots(s)?;
        s.edit("Expand", |d, _| expand_placed_under(d, &roots))?;
        e = Expandable::of(&s.doc()?.doc, &selected_roots(s)?);
    }
    // Then the symbol instances, those in the placed documents' art included: their art expands
    // below, and the instances it holds stay instances.
    if object && e.symbols {
        let roots = selected_roots(s)?;
        // An edit that expands no instance (too many objects to make) is undone, and the
        // instances stay as they are.
        let mut expanded = None;
        let r = s.edit("Expand", |d, _| {
            let any = super::brushsym::expand_instances_under(d, &roots)?;
            expanded = Some(any);
            if any { Ok(()) } else { Err(EngineError::Other("Expand: nothing to expand".into())) }
        });
        if !matches!((&r, expanded), (Err(_), Some(false))) {
            r?;
        }
        e = Expandable::of(&s.doc()?.doc, &selected_roots(s)?);
        // The placed documents in the instances' art too.
        if e.placed {
            let roots = selected_roots(s)?;
            s.edit("Expand", |d, _| expand_placed_under(d, &roots))?;
            e = Expandable::of(&s.doc()?.doc, &selected_roots(s)?);
        }
    }
    // Intertwine groups before what they hold: their objects, and the copies of the parts drawn
    // over the others, expand below.
    if object && e.intertwines {
        let roots = selected_roots(s)?;
        s.edit("Expand", |d, _| super::intertwine::expand_intertwines(d, &roots).map(|_| ()))?;
    }
    // Blends first: what their steps hold (type, effects, strokes, gradients) expands below.
    if object && e.blends {
        let roots = selected_roots(s)?;
        s.edit("Expand", |d, _| super::live::expand_blends(d, &roots).map(|_| ()))?;
    }
    // Repeats too: their instances are copies of art that expands below.
    if object && e.repeats {
        let roots = selected_roots(s)?;
        s.edit("Expand", |d, _| super::patterncmds::expand_repeats(d, &roots).map(|_| ()))?;
    }
    // The dimensions in the art those steps made.
    end_dimension_links(s, &mut ended)?;
    if object && e.effects {
        // Effects that Expand Appearance can't expand are left as they are. When it refuses to
        // leave a process color on a grayscale image with transparency, Expand fails as a whole.
        if let Err(e) = run_raw(s, "effect.expandAppearance", &json!({}))
            && super::paint::is_colorize_refusal(&e)
        {
            return Err(e);
        }
        // What is left to expand is in the baked art: Expand Appearance leaves strokes strokes
        // (#3089), a group's own as stroked paths among its members, which Stroke outlines below.
        e = Expandable::of(&s.doc()?.doc, &selected_roots(s)?);
    }
    if object && e.envelope {
        let roots = selected_roots(s)?;
        s.edit("Expand", |d, _| {
            for r in &roots {
                super::live::expand_envelopes_under(d, *r)?;
            }
            Ok(())
        })?;
    }
    if object && e.live {
        let roots = selected_roots(s)?;
        s.edit("Expand", |d, _| {
            for r in &roots {
                if let Some(n) = d.node_mut(*r) {
                    drop_live(n);
                }
            }
            Ok(())
        })?;
    }
    if object && e.text {
        let _ = run_raw(s, "type.createOutlines", &json!({}));
    }
    if stroke && e.stroke {
        let _ = run_raw(s, "object.path.outlineStroke", &json!({}));
    }
    if fill {
        let _ = expand_gradient_fills(s, to);
        let _ = expand_pattern_fills(s);
    }
    if s.doc()?.revision == rev0 {
        return Err(EngineError::Other("Expand: nothing to expand".into()));
    }
    if !ended.is_empty() || !lifted.is_empty() {
        s.edit("Expand", |d, sel| {
            if !lifted.is_empty() {
                super::dimension::put_back(d, lifted);
                // The dimensions put back follow what Expand made of the art they measure: a
                // dimension whose art no longer fits its link stays where it was, no longer linked.
                super::dimension::refresh(&before, d)?;
                let mut ids = sel.objects.clone();
                let again: Vec<NodeId> = kept.iter().copied().filter(|id| selected.contains(id) && !ids.contains(id)).collect();
                ids.extend(again);
                sel.set(ids);
            }
            // A dimension whose source Expand replaced stays where it was, no longer linked.
            super::dimension::restore_unchanged(&before, d, ended);
            Ok(())
        })?;
        if !kept.is_empty() && s.doc()?.doc.layers == before.layers {
            return Err(EngineError::Other("Expand: nothing to expand".into()));
        }
    }
    squash(s, from, "Expand");
    Ok(json!({ "ids": s.doc()?.selection.objects.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

/// Ends the live link of each dimension under or holding the selection, adding it to `ended`.
fn end_dimension_links(s: &mut Session, ended: &mut Vec<EndedLink>) -> Result<()> {
    let roots = selected_roots(s)?;
    if !super::dimension::reached(&s.doc()?.doc, &roots).is_empty() {
        ended.extend(s.edit("Expand", |d, _| Ok(super::dimension::end_links(d, &roots)))?);
    }
    Ok(())
}

fn drop_live(n: &mut Node) {
    if let NodeKind::Path { live, .. } = &mut n.kind {
        *live = None;
    }
    if let Some(ch) = n.children_mut() {
        for c in ch.iter_mut() {
            drop_live(Arc::make_mut(c));
        }
    }
}

/// Replace every gradient fill of the selected paths by art (see [`expand_fills`]).
fn expand_gradient_fills(s: &mut Session, to: GradientTo) -> Result<()> {
    let roots = selected_roots(s)?;
    s.edit("Expand", |d, sel| {
        let (mut ids, mut changed) = (vec![], 0);
        for r in &roots {
            let (id, n) = replace_leaves(d, *r, |d, l| expand_fills(d, l, to))?;
            ids.push(id);
            changed += n;
        }
        if changed == 0 {
            return Err(EngineError::Other("Expand: no gradient fills".into()));
        }
        sel.set(ids);
        Ok(())
    })
}

/// Path `l` with each gradient fill turned into art, in appearance order: the other items stay
/// on plain copies of the path. The path's geometry effects ([`reshaped`]) and each fill's own
/// ([`fill_shape`]) shape the outline the art fills first, as on the canvas. `None` when no
/// gradient fill can be expanded. A path that live-effect plug-ins paint is expanded piece by piece
/// ([`expand_pieces`]).
fn expand_fills(d: &mut Document, l: &Node, to: GradientTo) -> Option<Node> {
    if vectorcraft_render::effects::has_painted_effects(l) {
        return expand_pieces(d, l, usize::MAX, |d, piece, _| expand_piece_fills(d, piece, to));
    }
    expand_piece_fills(d, l, to)
}

/// [`expand_fills`] of a path with no live-effect plug-in that paints what it returns.
fn expand_piece_fills(d: &mut Document, l: &Node, to: GradientTo) -> Option<Node> {
    if !matches!(l.kind, NodeKind::Path { .. } | NodeKind::Compound { .. }) || !l.appearance.items.iter().any(|i| gradient_fill(i).is_some()) {
        return None;
    }
    let reshaped = reshaped(d, l);
    let l = reshaped.as_ref().unwrap_or(l);
    let (path, _) = node_path(l)?;
    let bounds = path.bounds()?;
    let mut parts = vec![];
    let mut rest = vec![];
    let mut expanded = false;
    for item in &l.appearance.items {
        // Items that paint nothing go; a gradient that can't be expanded stays.
        let art = gradient_fill(item).and_then(|_| {
            let (fl, path, bounds, moved) = fill_shape(d, l, item, &path, bounds)?;
            let (f, g) = gradient_fill(&moved)?;
            gradient_art(d, &fl, &path, bounds, f, g, to)
        });
        let Some(art) = art else {
            if item.visible() && !item.paint().is_none() {
                rest.push(item.clone());
            }
            continue;
        };
        if !rest.is_empty() {
            parts.push(styled_copy(d, l, std::mem::take(&mut rest)));
        }
        parts.push(art);
        expanded = true;
    }
    if !expanded {
        return None;
    }
    if !rest.is_empty() {
        parts.push(styled_copy(d, l, rest));
    }
    assemble(d, l, parts)
}

/// Gradient fill `f` (gradient `g`) of path `l` (outline `path`, box `bounds`) as art, with the
/// fill's opacity, blend mode and effects other than geometry effects ([`fill_effects`]).
fn gradient_art(d: &mut Document, l: &Node, path: &PathData, bounds: Rect, f: &FillLayer, g: &GradientPaint, to: GradientTo) -> Option<Node> {
    let content = if g.gradient.kind == GradientKind::Freeform {
        let m = GradientMesh::for_path_with(path, FREEFORM_MESH, FREEFORM_MESH, &*gradient_sampler(g, bounds, d.color_mode.model()))?;
        vec![Node::new(d.alloc_id(), NodeKind::Mesh(m))]
    } else {
        // A gradient that repeats or reflects expands as its stops repeated over the box (and
        // stays a gradient when the box needs more runs than are written).
        let (g, runs) = padded_on(g, bounds)?;
        let g = &*g;
        let frame = Frame::new(g, bounds);
        match to {
            GradientTo::Mesh => vec![Node::new(d.alloc_id(), NodeKind::Mesh(frame.mesh(&g.gradient)))],
            GradientTo::Objects(n) => frame
                .pieces(&g.gradient, n as usize, runs)
                .into_iter()
                .map(|(shape, (color, opacity))| {
                    let fill = FillLayer { opacity, overprint: f.overprint, ..FillLayer::new(Paint::solid(color)) };
                    Node::path(d.alloc_id(), shape, Appearance { items: vec![AppearanceItem::Fill(fill)], ..Default::default() })
                })
                .collect(),
        }
    };
    // A freeform mesh takes the object's outline: only holes need the object to clip it.
    let art = if g.gradient.kind == GradientKind::Freeform && path.subpaths.len() == 1 {
        content.into_iter().next()?
    } else {
        let mut clip = styled_copy(d, l, vec![]);
        as_clipping_path(&mut clip).ok()?;
        let children = std::iter::once(clip).chain(content).map(Arc::new).collect();
        Node::new(d.alloc_id(), NodeKind::Group { children, clip: true })
    };
    let mut art = with_transparency(d, art, f.opacity, f.blend);
    art.appearance.effects = fill_effects(f);
    Some(art)
}

/// Path `l`, which live-effect plug-ins paint, as the art the canvas draws
/// ([`reshape`](vectorcraft_render::effects::reshape)) with ids of its own, each piece of the art
/// replaced by what `expand` makes of it (`expand` gets the piece and how many objects it may become;
/// `None` keeps the piece with its own paints). `None` when the art would hold more than `budget`
/// objects or no piece expands.
fn expand_pieces(d: &mut Document, l: &Node, budget: usize, mut expand: impl FnMut(&mut Document, &Node, usize) -> Option<Node>) -> Option<Node> {
    let art = vectorcraft_render::effects::reshape(Some(d), l, None)?;
    let mut art = d.reid(&art);
    art.locked = l.locked;
    let mut left = budget.checked_sub(art.count_with_masks())?;
    let mut expanded = false;
    expand_piece_leaves(d, &mut art, &mut left, &mut expanded, &mut expand);
    expanded.then_some(art)
}

/// Replace each path and compound path under `n` (`n` included) by what `expand` makes of it, in
/// paint order. `left` is how many more objects the art may hold, and `expanded` is set when a path
/// is replaced.
fn expand_piece_leaves(
    d: &mut Document,
    n: &mut Node,
    left: &mut usize,
    expanded: &mut bool,
    expand: &mut impl FnMut(&mut Document, &Node, usize) -> Option<Node>,
) {
    if matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. }) {
        let room = *left + n.count_with_masks();
        if let Some(new) = expand(d, n, room) {
            *left = room.saturating_sub(new.count_with_masks());
            *n = new;
            *expanded = true;
        }
        return;
    }
    for c in n.children_mut().into_iter().flatten().filter(|c| !c.is_ruler_guide()) {
        expand_piece_leaves(d, Arc::make_mut(c), left, expanded, expand);
    }
}

/// Path `l` as the canvas paints its fills and strokes: its visible geometry effects applied to its
/// path (a plain path then), and every geometry effect of its own removed, with each raster effect
/// following the Transform effects after it
/// ([`without_geometry`](vectorcraft_render::effects::without_geometry)) and those of its fills and
/// strokes following its Transform effects
/// ([`follow_items`](vectorcraft_render::effects::follow_items)). `None` when it has no
/// visible geometry effect. The swatches and patterns that live-effect plug-ins painting their
/// pieces name are looked up in `doc`. This merges the pieces of such a plug-in into one path, so a
/// path that one paints goes through [`expand_pieces`] instead.
pub(crate) fn reshaped(doc: &Document, l: &Node) -> Option<Node> {
    use vectorcraft_render::effects::{GeomContext, apply_geometry_carrying, has_geometry, without_geometry};
    if !has_geometry(&l.appearance.effects) {
        return None;
    }
    let (path, _) = node_path(l)?;
    let (g, carry) = apply_geometry_carrying(&l.appearance.effects, &path, path.bounds()?, &GeomContext { doc: Some(doc), ..GeomContext::of(l) });
    let mut m = with_path(l, g);
    // Its gradients move with the outline, as the canvas paints them.
    if carry.moves(&m.appearance) {
        carry.apply(&mut m.appearance);
    }
    // So do the raster effects of it and of its fills and strokes, by the Transform effects that
    // moved it.
    carry.hide_skipped(&mut m.appearance.effects);
    vectorcraft_render::effects::follow_items(&mut m);
    m.appearance.effects = without_geometry(&m.appearance.effects);
    Some(m)
}

/// Path `l` (outline `path`, box `bounds`) as its fill `item` paints it on the canvas, with its
/// outline, box and fill: when the fill has visible geometry effects, `l` with the outline they
/// make of `path` as a plain path, that outline's box, and the fill with its gradient moved along
/// with the outline.
pub(crate) fn fill_shape(
    doc: &Document,
    l: &Node,
    item: &AppearanceItem,
    path: &PathData,
    bounds: Rect,
) -> Option<(Node, PathData, Rect, AppearanceItem)> {
    use vectorcraft_render::effects::{GeomContext, apply_geometry_carrying, has_geometry};
    if !has_geometry(item.effects()) {
        return Some((l.clone(), path.clone(), bounds, item.clone()));
    }
    // In the fill's context, as drawing runs them: a plug-in effect is told which fill it is on.
    let (outline, carry) = apply_geometry_carrying(item.effects(), path, bounds, &GeomContext { doc: Some(doc), ..GeomContext::of(l) }.item(item));
    let b = outline.bounds()?;
    let mut one = Appearance { items: vec![item.clone()], ..Default::default() };
    if carry.moves(&one) {
        carry.apply(&mut one);
    }
    let mut moved = one.items.pop().unwrap_or_else(|| item.clone());
    carry.hide_skipped(moved.effects_mut());
    Some((with_path(l, outline.clone()), outline, b, moved))
}

/// Path or compound path `l` with `path` as its path, a plain path.
fn with_path(l: &Node, path: PathData) -> Node {
    let mut m = l.clone();
    m.kind = match &l.kind {
        NodeKind::Path { rule, clipping, guide, .. } => NodeKind::Path { path, rule: *rule, live: None, clipping: *clipping, guide: *guide },
        NodeKind::Compound { rule, .. } => NodeKind::Path { path, rule: *rule, live: None, clipping: false, guide: false },
        k => k.clone(),
    };
    m
}

/// The effects of fill `f` that stay on its expanded art: all but its geometry effects, which
/// shape the outline it paints ([`fill_shape`]), with each raster effect following the Transform
/// effects after it ([`without_geometry`](vectorcraft_render::effects::without_geometry)).
fn fill_effects(f: &FillLayer) -> Vec<vectorcraft_doc::Effect> {
    vectorcraft_render::effects::without_geometry(&f.effects)
}

/// Replace every pattern fill of the selected paths by the tiles it paints (see [`expand_patterns`]).
fn expand_pattern_fills(s: &mut Session) -> Result<()> {
    let roots = selected_roots(s)?;
    s.edit("Expand", |d, sel| {
        let (mut ids, mut changed) = (vec![], 0);
        for r in &roots {
            let (id, n) = replace_leaves(d, *r, expand_patterns)?;
            ids.push(id);
            changed += n;
        }
        if changed == 0 {
            return Err(EngineError::Other("Expand: nothing to expand".into()));
        }
        sel.set(ids);
        Ok(())
    })
}

/// Expand the pattern fills under `root` that paint a transparent or colorized image into the tiles they paint
/// ([`expand_patterns_where`]), so each tile image becomes an object of its own; the other
/// pattern fills stay. A tile image counts when its key is in `see_through` (it has see-through
/// pixels), when it has a colorize paint that `colorize` counts, or when it, a group
/// in the tile art, the fill, the shape or a group the shape is in has opacity or a blending
/// mode. Blends, envelopes, repeats and Intertwine groups that hold such a fill are expanded
/// first ([`expand_live_with_transparent_image_patterns`]). Hidden shapes and shapes in hidden
/// groups or layers and in template layers stay as they are. A pattern fill in tile art counts
/// too, down to the [`PATTERN_LEVELS`] the canvas draws, and the tile art the expansion makes is
/// expanded again for it. A pattern fill stays, and a live object stays live, when its art would
/// take the expanded art past [`MAX_GENERATED`](vectorcraft_doc::live::MAX_GENERATED) objects.
/// Returns how many objects changed.
pub(crate) fn expand_transparent_image_patterns(
    d: &mut Document,
    root: NodeId,
    see_through: &BTreeSet<String>,
    colorize: &dyn Fn(&Document, &Node) -> bool,
) -> Result<usize> {
    expand_transparent_image_patterns_within(d, root, see_through, colorize, vectorcraft_doc::live::MAX_GENERATED)
}

/// [`expand_transparent_image_patterns`] with the expanded art held to `budget` objects.
pub(crate) fn expand_transparent_image_patterns_within(
    d: &mut Document,
    root: NodeId,
    see_through: &BTreeSet<String>,
    colorize: &dyn Fn(&Document, &Node) -> bool,
    budget: usize,
) -> Result<usize> {
    let mut changed = 0;
    let made = std::cell::Cell::new(0usize);
    // Each pass brings the patterns in the expanded tiles' art up one level.
    for levels in (1..=PATTERN_LEVELS).rev() {
        let live = expand_live_with_transparent_image_patterns(d, root, see_through, colorize, levels, &made, budget)?;
        let (_, n) = replace_leaves(d, root, |d, l| {
            // Past the budget, and for most leaves, which have no pattern fill, skip the ancestry walk.
            if made.get() >= budget || !l.appearance.items.iter().any(|i| pattern_fill(d, i).is_some()) {
                return None;
            }
            // The expanded art would be shown.
            if !d.ancestry(l.id).unwrap_or_default().iter().filter_map(|a| d.node(*a)).all(drawn) {
                return None;
            }
            let which = |d: &Document, i: &AppearanceItem| transparent_pattern_fill(d, l, i, see_through, colorize, levels);
            let art = expand_patterns_where(d, l, which, budget - made.get())?;
            // The copies of the shape's other items count too.
            let count = art.count_with_masks();
            if count > budget - made.get() {
                return None;
            }
            made.set(made.get() + count);
            Some(art)
        })?;
        if live + n == 0 {
            break;
        }
        changed += live + n;
    }
    Ok(changed)
}

/// How many levels of patterns the canvas draws: a pattern fill in the art of a pattern, and so
/// on, as [`vectorcraft_render`] draws them.
const PATTERN_LEVELS: u32 = 3;

/// Whether a layer of `d` holds a pattern fill that [`expand_transparent_image_patterns`] would
/// expand with `see_through` and `colorize` ([`holds_transparent_image_patterns`]).
pub(crate) fn has_transparent_image_patterns(d: &Document, see_through: &BTreeSet<String>, colorize: &dyn Fn(&Document, &Node) -> bool) -> bool {
    d.layers.iter().any(|l| holds_transparent_image_patterns(d, l, see_through, colorize, PATTERN_LEVELS))
}

/// Whether `n`, art of `d`, is or holds a shown path or compound path (in groups, blends,
/// envelopes, repeats and Intertwine groups) with a pattern fill that
/// [`transparent_pattern_fill`] counts with `levels`. Guides, clipping paths and template layers
/// don't count.
fn holds_transparent_image_patterns(
    d: &Document,
    n: &Node,
    see_through: &BTreeSet<String>,
    colorize: &dyn Fn(&Document, &Node) -> bool,
    levels: u32,
) -> bool {
    if !drawn(n) {
        return false;
    }
    match &n.kind {
        NodeKind::Path { guide: true, .. } | NodeKind::Path { clipping: true, .. } => false,
        NodeKind::Path { .. } | NodeKind::Compound { .. } => {
            n.appearance.items.iter().any(|i| transparent_pattern_fill(d, n, i, see_through, colorize, levels))
        }
        _ => n.children().is_some_and(|ch| ch.iter().any(|c| holds_transparent_image_patterns(d, c, see_through, colorize, levels))),
    }
}

/// Whether `n` is shown and isn't a template layer.
fn drawn(n: &Node) -> bool {
    n.visible && !matches!(n.kind, NodeKind::Layer { template: true, .. })
}

/// Expand the shown blends, envelopes, repeats and Intertwine groups under `root` that hold a
/// pattern fill [`holds_transparent_image_patterns`] counts with `levels` into the art the exporters write for
/// them ([`expand_live_deep`](vectorcraft_render::effects::expand_live_deep), under one budget for
/// everything nested in each), so the shapes they draw are leaves. The objects that art holds count
/// in `made`, and an object whose art would take `made` past `budget` stays live. Returns how many
/// were expanded.
fn expand_live_with_transparent_image_patterns(
    d: &mut Document,
    root: NodeId,
    see_through: &BTreeSet<String>,
    colorize: &dyn Fn(&Document, &Node) -> bool,
    levels: u32,
    made: &std::cell::Cell<usize>,
    budget: usize,
) -> Result<usize> {
    fn outermost(
        d: &Document,
        n: &Node,
        see_through: &BTreeSet<String>,
        colorize: &dyn Fn(&Document, &Node) -> bool,
        levels: u32,
        out: &mut Vec<NodeId>,
    ) {
        if !drawn(n) {
            return;
        }
        match &n.kind {
            NodeKind::Blend { .. } | NodeKind::Envelope { .. } | NodeKind::Repeat(_) | NodeKind::Intertwine { .. } => {
                if holds_transparent_image_patterns(d, n, see_through, colorize, levels) {
                    out.push(n.id);
                }
            }
            NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => {
                children.iter().for_each(|c| outermost(d, c, see_through, colorize, levels, out));
            }
            _ => {}
        }
    }
    /// Fresh ids for everything under `n`: copies of the same art share theirs.
    fn fresh_ids(d: &mut Document, n: &mut Node) {
        for c in n.children_mut().into_iter().flatten() {
            let c = Arc::make_mut(c);
            c.id = d.alloc_id();
            fresh_ids(d, c);
        }
    }
    let mut live = vec![];
    if let Some(n) = d.node(root) {
        outermost(d, n, see_through, colorize, levels, &mut live);
    }
    let mut count = 0;
    for id in &live {
        if made.get() >= budget {
            break;
        }
        let Some(n) = d.node(*id) else { continue };
        // Random Transform effects take their seed from the ids: bake them first ([`bake_seeded`]),
        // and leave the object live when that can't be done.
        let Some(mut g) = bake_seeded(d, &vectorcraft_render::effects::expand_live_deep(Some(d), n)) else { continue };
        // An object whose art would take the expanded art past the budget stays live.
        let size = g.count_with_masks();
        if size > budget - made.get() {
            continue;
        }
        made.set(made.get() + size);
        g.id = *id;
        fresh_ids(d, &mut g);
        *d.node_mut(*id).ok_or(EngineError::NoNode(*id))? = g;
        count += 1;
    }
    Ok(count)
}

/// Whether `i`, an item of leaf `l`, is a pattern fill whose tile art holds a transparent image
/// ([`transparent_image`], through pattern fills in that art down to `levels` patterns),
/// counting the fill's and the shape's opacity and blending mode.
fn transparent_pattern_fill(
    d: &Document,
    l: &Node,
    i: &AppearanceItem,
    see_through: &BTreeSet<String>,
    colorize: &dyn Fn(&Document, &Node) -> bool,
    levels: u32,
) -> bool {
    let shape = d.ancestry(l.id).unwrap_or_default().iter().filter_map(|a| d.node(*a)).any(blended);
    pattern_fill_holds_transparent_image(d, i, see_through, colorize, shape, levels)
}

/// Whether `i` is a pattern fill whose tile art holds a transparent image ([`transparent_image`]),
/// down to `levels` patterns, counting the fill's opacity and blending mode, or anything when `on`.
fn pattern_fill_holds_transparent_image(
    d: &Document,
    i: &AppearanceItem,
    keys: &BTreeSet<String>,
    colorize: &dyn Fn(&Document, &Node) -> bool,
    on: bool,
    levels: u32,
) -> bool {
    if levels == 0 {
        return false;
    }
    let Some((f, def, _)) = pattern_fill(d, i) else { return false };
    let on = on || f.opacity < 1.0 || f.blend != BlendMode::Normal;
    def.art.iter().any(|a| transparent_image(d, a, keys, colorize, on, levels - 1))
}

/// Whether `n` has opacity or a blending mode of its own.
fn blended(n: &Node) -> bool {
    n.opacity < 1.0 || n.blend != BlendMode::Normal
}

/// Whether `n` (art of `d`) is or holds an image with see-through pixels (key in `keys`, a
/// color-masked image's masked pixels counted as its own), a colorize paint that `colorize` counts, opacity or a blending mode of its own or of a group in
/// `n`, or anywhere when `on`, also in the tile art of the pattern fills of the paths in `n`, down
/// to `levels` patterns. Hidden art doesn't count.
fn transparent_image(d: &Document, n: &Node, keys: &BTreeSet<String>, colorize: &dyn Fn(&Document, &Node) -> bool, on: bool, levels: u32) -> bool {
    if !n.visible {
        return false;
    }
    let on = on || blended(n);
    match &n.kind {
        NodeKind::Image(im) => {
            on || keys.contains(&im.key) || keys.contains(&d.shown_image(n, im).key) || (n.extras().colorize.is_some() && colorize(d, n))
        }
        NodeKind::Path { .. } | NodeKind::Compound { .. } => {
            n.appearance.items.iter().any(|i| pattern_fill_holds_transparent_image(d, i, keys, colorize, on, levels))
        }
        _ => n.children().is_some_and(|ch| ch.iter().any(|c| transparent_image(d, c, keys, colorize, on, levels))),
    }
}

/// Path `l` with each pattern fill turned into the tiles it paints over the box of the outline it
/// fills, in a clip group shaped like that outline with the fill's opacity, blend mode and effects
/// other than geometry effects, in appearance order: the other items stay on plain copies of the
/// path. The path's geometry effects ([`reshaped`]) and each fill's own ([`fill_shape`]) shape
/// that outline first, as on the canvas. The tiles are drawn as the canvas draws them
/// ([`pattern_tiles`]). A pattern fill that lays no tile there stays. `None` when no pattern fill
/// lays a tile.
fn expand_patterns(d: &mut Document, l: &Node) -> Option<Node> {
    expand_patterns_where(d, l, |_, _| true, usize::MAX)
}

/// [`expand_patterns`] for only the pattern fills `which` picks: the others stay like the other
/// items, as does a pattern fill whose tiles would take the tiles of `l` past `budget` objects. A
/// path that live-effect plug-ins paint is expanded piece by piece ([`expand_pieces`]).
fn expand_patterns_where(d: &mut Document, l: &Node, which: impl Fn(&Document, &AppearanceItem) -> bool, budget: usize) -> Option<Node> {
    if vectorcraft_render::effects::has_painted_effects(l) {
        return expand_pieces(d, l, budget, |d, piece, room| expand_piece_patterns(d, piece, &which, room));
    }
    expand_piece_patterns(d, l, which, budget)
}

/// [`expand_patterns_where`] of a path with no live-effect plug-in that paints what it returns.
fn expand_piece_patterns(d: &mut Document, l: &Node, which: impl Fn(&Document, &AppearanceItem) -> bool, budget: usize) -> Option<Node> {
    if !matches!(l.kind, NodeKind::Path { .. } | NodeKind::Compound { .. })
        || !l.appearance.items.iter().any(|i| pattern_fill(d, i).is_some() && which(d, i))
    {
        return None;
    }
    let reshaped = reshaped(d, l);
    let l = reshaped.as_ref().unwrap_or(l);
    let (path, _) = node_path(l)?;
    let bounds = path.bounds()?;
    let mut parts = vec![];
    let mut rest = vec![];
    let mut expanded = false;
    // What `assemble` adds around the tiles when every picked fill expands: a copy of the path for
    // each run of other items, a group around the parts and one for the path's transparency, and
    // the path's opacity mask.
    let (mut copies, mut run, mut picked) = (0usize, false, vec![]);
    for i in &l.appearance.items {
        if let Some((f, _, _)) = pattern_fill(d, i).filter(|_| which(d, i)) {
            run = false;
            picked.push(f.opacity < 1.0 || f.blend != BlendMode::Normal);
        } else if i.visible() && !i.paint().is_none() && !run {
            (copies, run) = (copies + 1, true);
        }
    }
    let group = copies + picked.len() > 1 || !l.appearance.effects.is_empty();
    // A lone tile group with transparency of its own goes in a group for the path's.
    let transparency = blended(l) && !group && picked.first().is_some_and(|b| *b);
    let around = copies.saturating_mul(l.count()) + usize::from(group) + usize::from(transparency) + l.count_with_masks().saturating_sub(l.count());
    let mut left = budget.saturating_sub(around);
    // A fill that expands while others follow leaves room for the copy and group that those take
    // if they don't fit, and once one doesn't fit, the ones after it stay too.
    let (mut later, mut full) = (picked.len(), false);
    for item in &l.appearance.items {
        let fill = pattern_fill(d, item).filter(|_| which(d, item)).map(|(f, def, xf)| (f, def.clone(), xf));
        later -= usize::from(fill.is_some());
        let room = if later > 0 { left.saturating_sub(l.count() + 1) } else { left };
        let tiles = fill.filter(|_| !full).and_then(|(f, def, xf)| {
            let (fl, _, bounds, _) = fill_shape(d, l, item, &path, bounds)?;
            let Some(t) = pattern_tiles(d, &def, xf, bounds, room) else {
                full = true;
                return None;
            };
            (!t.is_empty()).then_some((f, fl, t))
        });
        let art = tiles.and_then(|(f, fl, t)| tile_art(d, &fl, f, &t));
        full |= art.as_ref().is_some_and(|a| a.count_with_masks() > room);
        let Some(art) = art.filter(|a| a.count_with_masks() <= room) else {
            if item.visible() && !item.paint().is_none() {
                rest.push(item.clone());
            }
            continue;
        };
        left -= art.count_with_masks();
        if !rest.is_empty() {
            parts.push(styled_copy(d, l, std::mem::take(&mut rest)));
        }
        parts.push(art);
        expanded = true;
    }
    if !expanded {
        return None;
    }
    if !rest.is_empty() {
        parts.push(styled_copy(d, l, rest));
    }
    assemble(d, l, parts)
}

/// The tiles pattern `def` placed by `xf` lays over `bounds` in document `d`, as the canvas draws
/// them: their stroke weights and effects scale with the pattern, and when it also turns, flips,
/// stretches or shears them, the strokes (brushed ones included) and geometry effects of their
/// paths and opacity masks become filled art and their live shapes become plain paths first. The
/// same happens when it scales art with geometry effects (Roughen's detail, for one, is per inch
/// and does not scale). No tiles when it turns, flips, stretches or shears art with raster or 3D
/// effects or with a geometry effect on a group or type, or scales art with a geometry effect on a
/// group or type, which the canvas draws turned or scaled with the tile and live effects would
/// not, and when it does more than move art that holds a symbol instance, whose strokes the canvas
/// draws scaled with the tile and an instance does not scale. Random Transform effects and brushes
/// that vary at random in the art are baked first ([`seeds_baked`], [`random_brushes_baked`]).
/// `None` when the tiles would hold more than `budget` objects.
fn pattern_tiles(d: &mut Document, def: &PatternDef, xf: Affine, bounds: Rect, budget: usize) -> Option<Vec<Node>> {
    if !moves_only(xf) && def.art.iter().any(|a| has_instances(a)) {
        return Some(vec![]);
    }
    let Some(def) = seeds_baked(d, def) else {
        return Some(vec![]);
    };
    let brushes = vectorcraft_brush::library(d);
    let def = random_brushes_baked(d, &brushes, def);
    let def = &*def;
    let per_tile = def.art.iter().map(|a| a.count_with_masks()).sum::<usize>();
    if def.offsets_covering(xf.inverse().transform_rect_bbox(bounds)).len().saturating_mul(per_tile) > budget {
        return None;
    }
    let scaling = Scaling { strokes: true, effects: Some(vectorcraft_render::effects::scale_effect), patterns: true, ..Scaling::default() };
    if scales_evenly(xf) && (moves_only(xf) || !def.art.iter().any(|a| has_geometry_effects(a))) {
        return Some(def.instances_in_with(xf, bounds, scaling));
    }
    if def.art.iter().any(|a| has_unbakeable_effects(a)) {
        return Some(vec![]);
    }
    let placed = Scaling { patterns: true, ..Scaling::default() };
    let mut tiles = def.instances_in_with(Affine::IDENTITY, xf.inverse().transform_rect_bbox(bounds), placed);
    // Outlined strokes and effects can make more objects than the art had.
    let mut left = budget;
    for t in &mut tiles {
        *t = prepared(d, &brushes, t);
        left = left.checked_sub(t.count_with_masks())?;
        place_art(t, xf, scaling);
    }
    Some(tiles)
}

/// Pattern `def` with each path in its art (opacity masks included) that has a random Transform
/// effect baked into filled art ([`bake_appearance`](vectorcraft_render::effects::bake_appearance))
/// while it keeps its own id: the effect takes its seed from the id, and the canvas draws the art
/// with those ids for every tile, while the tiles get ids of their own. `None` when something
/// other than a path has such an effect. The swatches and patterns that live-effect plug-ins
/// painting their pieces name are looked up in `d`.
fn seeds_baked<'a>(d: &Document, def: &'a PatternDef) -> Option<std::borrow::Cow<'a, PatternDef>> {
    if !def.art.iter().any(|a| has_seeded(a)) {
        return Some(std::borrow::Cow::Borrowed(def));
    }
    let mut def = def.clone();
    def.art = def.art.iter().map(|a| bake_seeded(d, a).map(Arc::new)).collect::<Option<_>>()?;
    Some(std::borrow::Cow::Owned(def))
}

/// Pattern `def` with each path in its art (opacity masks included) whose stroke has a brush that
/// varies at random ([`draws_random_art`]) [`prepared`] where the pattern defines it: the brush
/// takes its seed from the path's points, and the canvas draws the art there for every tile.
fn random_brushes_baked<'a>(
    d: &mut Document,
    brushes: &[vectorcraft_brush::Brush],
    def: std::borrow::Cow<'a, PatternDef>,
) -> std::borrow::Cow<'a, PatternDef> {
    if !def.art.iter().any(|a| has_random_brush(brushes, a)) {
        return def;
    }
    let mut def = def.into_owned();
    def.art = def.art.iter().map(|a| Arc::new(bake_random_brushes(d, brushes, a))).collect();
    std::borrow::Cow::Owned(def)
}

/// Does brush `name` of `brushes` draw different art for different points: a calligraphic or
/// scatter brush with Random variation, or a bristle brush?
fn draws_random_art(brushes: &[vectorcraft_brush::Brush], name: &str) -> bool {
    use vectorcraft_brush::{BrushKind, Variation};
    brushes.iter().find(|b| b.name == name).is_some_and(|b| match &b.kind {
        BrushKind::Calligraphic(c) => c.modes().contains(&Variation::Random),
        BrushKind::Scatter(s) => s.modes().contains(&Variation::Random),
        BrushKind::Bristle(_) => true,
        BrushKind::Art(_) | BrushKind::Pattern(_) => false,
    })
}

/// Is `n` a path with a visible stroke whose brush varies at random ([`draws_random_art`])?
fn random_brushed(brushes: &[vectorcraft_brush::Brush], n: &Node) -> bool {
    let random = |st: &vectorcraft_doc::appearance::StrokeLayer| st.visible && st.brush.as_deref().is_some_and(|b| draws_random_art(brushes, b));
    matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. })
        && n.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Stroke(st) if random(st)))
}

/// Does `n`, something in it or one of their opacity masks have a stroke whose brush varies at
/// random ([`random_brushed`])?
fn has_random_brush(brushes: &[vectorcraft_brush::Brush], n: &Node) -> bool {
    random_brushed(brushes, n)
        || n.extras().mask.as_ref().is_some_and(|m| has_random_brush(brushes, &m.art))
        || (!matches!(n.kind, NodeKind::Compound { .. }) && n.children().is_some_and(|c| c.iter().any(|c| has_random_brush(brushes, c))))
}

/// `n` with the paths in it whose brush varies at random [`prepared`] ([`random_brushes_baked`]).
fn bake_random_brushes(d: &mut Document, brushes: &[vectorcraft_brush::Brush], n: &Node) -> Node {
    if !has_random_brush(brushes, n) {
        return n.clone();
    }
    if random_brushed(brushes, n) {
        return prepared(d, brushes, n);
    }
    let mut n = n.clone();
    if !matches!(n.kind, NodeKind::Compound { .. }) {
        for c in n.children_mut().into_iter().flatten() {
            *c = Arc::new(bake_random_brushes(d, brushes, c));
        }
    }
    if let Some(m) = n.extras_if_any().and_then(|e| e.mask.as_mut()) {
        m.art = Arc::new(bake_random_brushes(d, brushes, &m.art));
    }
    n
}

/// Does `n`, something in it or one of their opacity masks have a visible geometry effect?
fn has_geometry_effects(n: &Node) -> bool {
    let geometry = |e: &vectorcraft_doc::Effect| e.visible && vectorcraft_render::effects::is_geometry(&e.id);
    n.appearance.effects.iter().any(geometry)
        || n.appearance.items.iter().any(|i| i.effects().iter().any(geometry))
        || n.extras().mask.as_ref().is_some_and(|m| has_geometry_effects(&m.art))
        || n.children().is_some_and(|c| c.iter().any(|c| has_geometry_effects(c)))
}

/// Does `n`, something in it or one of their opacity masks hold a symbol instance?
fn has_instances(n: &Node) -> bool {
    matches!(n.kind, NodeKind::SymbolInstance { .. })
        || n.extras().mask.as_ref().is_some_and(|m| has_instances(&m.art))
        || n.children().is_some_and(|c| c.iter().any(|c| has_instances(c)))
}

/// Does `a` only move (no scale, turn, flip, stretch or shear)?
fn moves_only(a: Affine) -> bool {
    let [m0, m1, m2, m3, _, _] = a.as_coeffs();
    let eps = 1e-9;
    (m0 - 1.0).abs() <= eps && m1.abs() <= eps && m2.abs() <= eps && (m3 - 1.0).abs() <= eps
}

/// Is `e` a visible Transform effect with Random on (which takes its seed from the object's id)?
pub(crate) fn seeded(e: &vectorcraft_doc::Effect) -> bool {
    e.visible && e.id == "distort.transform" && e.params.get("random").and_then(Value::as_bool) == Some(true)
}

/// Does `n` have a visible Transform effect with Random on, its own or a fill's or stroke's?
fn seeded_here(n: &Node) -> bool {
    n.appearance.effects.iter().any(seeded) || n.appearance.items.iter().any(|i| i.effects().iter().any(seeded))
}

/// Does `n`, something in it or one of their opacity masks have a random Transform effect
/// ([`seeded_here`])?
pub(crate) fn has_seeded(n: &Node) -> bool {
    seeded_here(n)
        || n.extras().mask.as_ref().is_some_and(|m| has_seeded(&m.art))
        || (!matches!(n.kind, NodeKind::Compound { .. }) && n.children().is_some_and(|c| c.iter().any(|c| has_seeded(c))))
}

/// `n` with the paths in it that have a random Transform effect baked ([`seeds_baked`]). `None`
/// when something else has one. The swatches and patterns that live-effect plug-ins painting their
/// pieces name are looked up in `d`.
fn bake_seeded(d: &Document, n: &Node) -> Option<Node> {
    if !has_seeded(n) {
        return Some(n.clone());
    }
    let mut n = if seeded_here(n) {
        if !matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. }) {
            return None;
        }
        vectorcraft_render::effects::bake_appearance(Some(d), n)?
    } else {
        n.clone()
    };
    if !matches!(n.kind, NodeKind::Compound { .. }) {
        for c in n.children_mut().into_iter().flatten() {
            *c = Arc::new(bake_seeded(d, c)?);
        }
    }
    if let Some(m) = n.extras_if_any().and_then(|e| e.mask.as_mut()) {
        m.art = Arc::new(bake_seeded(d, &m.art)?);
    }
    Some(n)
}

/// Does `n`, something in it or one of their opacity masks have a visible effect that [`prepared`]
/// cannot bake: a raster or 3D effect, or a geometry effect on something other than a path?
fn has_unbakeable_effects(n: &Node) -> bool {
    use vectorcraft_render::effects::{is_geometry, is_raster, is_three_d};
    let path = matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. });
    let unbakeable = |e: &vectorcraft_doc::Effect| e.visible && (is_raster(&e.id) || is_three_d(&e.id) || (!path && is_geometry(&e.id)));
    n.appearance.effects.iter().any(unbakeable)
        || n.appearance.items.iter().any(|i| i.effects().iter().any(unbakeable))
        || n.extras().mask.as_ref().is_some_and(|m| has_unbakeable_effects(&m.art))
        || n.children().is_some_and(|c| c.iter().any(|c| has_unbakeable_effects(c)))
}

/// Tile art `n` ready for a turn, flip, stretch or shear: the strokes and geometry effects of its
/// paths baked ([`baked`], [`outline_brushed`]) and its live shapes turned into plain paths (a
/// stretch would rebuild their corners round), in its opacity masks too.
fn prepared(d: &mut Document, brushes: &[vectorcraft_brush::Brush], n: &Node) -> Node {
    let mut t = baked(d, n);
    outline_brushed(d, brushes, &mut t);
    drop_live(&mut t);
    prepare_masks(d, brushes, &mut t);
    t
}

/// The opacity masks of `n` and of everything in it [`prepared`].
fn prepare_masks(d: &mut Document, brushes: &[vectorcraft_brush::Brush], n: &mut Node) {
    if let Some(m) = n.extras_if_any().and_then(|e| e.mask.as_mut()) {
        m.art = Arc::new(prepared(d, brushes, &m.art));
    }
    for c in n.children_mut().into_iter().flatten() {
        prepare_masks(d, brushes, Arc::make_mut(c));
    }
}

/// `n` with each path in it that has a brushed stroke turned into filled art and brush art
/// ([`outline_strokes`]).
fn outline_brushed(d: &mut Document, brushes: &[vectorcraft_brush::Brush], n: &mut Node) {
    if matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. }) {
        let brushed = n.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Stroke(st) if st.brush.is_some()));
        if brushed && let Some(o) = outline_strokes(d, brushes, n) {
            *n = o;
        }
        return;
    }
    for c in n.children_mut().into_iter().flatten() {
        outline_brushed(d, brushes, Arc::make_mut(c));
    }
}

/// Is `a` a move and a positive uniform scale (which brush nibs, strokes and effects follow by
/// scaling alone)?
fn scales_evenly(a: Affine) -> bool {
    let [m0, m1, m2, m3, _, _] = a.as_coeffs();
    let eps = 1e-9 * m0.abs().max(m3.abs());
    m0 > 0.0 && m1.abs() <= eps && m2.abs() <= eps && (m0 - m3).abs() <= eps
}

/// `n` with the strokes and geometry effects of each path in it baked into filled art
/// ([`bake_appearance`](vectorcraft_render::effects::bake_appearance)), with the swatches and
/// patterns that live-effect plug-ins painting their pieces name looked up in `d`.
fn baked(d: &Document, n: &Node) -> Node {
    if let Some(b) = vectorcraft_render::effects::bake_appearance(Some(d), n) {
        return b;
    }
    let mut n = n.clone();
    for c in n.children_mut().into_iter().flatten() {
        *c = Arc::new(baked(d, c));
    }
    n
}

/// Pattern tiles `tiles` of fill `f` of path `l` as a clip group shaped like `l` that isolates
/// their blending (as the canvas draws a tile on its own), with the fill's opacity, blend mode and
/// effects other than geometry effects ([`fill_effects`]).
fn tile_art(d: &mut Document, l: &Node, f: &FillLayer, tiles: &[Node]) -> Option<Node> {
    let mut clip = styled_copy(d, l, vec![]);
    as_clipping_path(&mut clip).ok()?;
    let tiles: Vec<Node> = tiles.iter().map(|t| d.reid(t)).collect();
    let children = std::iter::once(clip).chain(tiles).map(Arc::new).collect();
    let mut art = Node::new(d.alloc_id(), NodeKind::Group { children, clip: true });
    art.isolate = true;
    let mut art = with_transparency(d, art, f.opacity, f.blend);
    art.appearance.effects = fill_effects(f);
    Some(art)
}

/// The colour and opacity gradient fill `g` paints at a document point of an object whose box is
/// `bounds`, as the canvas draws it (the end colours continue past the ends). Colours stay in the
/// color model their stops share; display RGB where models differ. Freeform points mix in the
/// model they share ([`vectorcraft_color::Freeform::model_field`]); where their models differ, the
/// RGB color they mix to is stored in `doc_model` (the document's model) the way `paint.setFill`
/// stores an RGB color ([`super::paint::stored_color`]).
pub(crate) fn gradient_sampler(g: &GradientPaint, bounds: Rect, doc_model: Model) -> Box<dyn Fn(Point) -> (Color, f32) + '_> {
    if g.gradient.kind == GradientKind::Freeform {
        let b = bounds.abs();
        let field = g.freeform_on(b).model_field(spread_scale(b));
        let store_in = (field.model().is_none() && doc_model != Model::Rgb).then_some(doc_model);
        return Box::new(move |p| {
            let (c, a) = field.sample(p);
            (super::paint::stored_color(store_in, c), a)
        });
    }
    let (geom, spread) = (g.resolve(bounds), g.gradient.spreader());
    Box::new(move |p| g.gradient.sample_keep_model(spread(geom.param_at(g.gradient.kind, p)) as f32))
}

/// For stops repeated by [`padded_on`]: where along the vector (0 to 1) their first run starts and
/// how many runs lie evenly from there to 1.
type Runs = Option<(f64, usize)>;

/// `g` placed on box `bounds` as a gradient that pads past its ends: a linear, radial or diamond
/// gradient that repeats or reflects with its stops repeated as many times as the box needs
/// ([`vectorcraft_color::Gradient::padded_over`]), a conic one with its stops repeated around the
/// turn ([`vectorcraft_color::Gradient::unrolled`]); others as they are. With it, for the repeated
/// stops, where along the vector (0 to 1) their first run starts and how many runs lie evenly from
/// there to 1. `None` when the box needs more runs than are written
/// ([`vectorcraft_color::Padded::capped`]): the gradient stays.
fn padded_on(g: &GradientPaint, bounds: Rect) -> Option<(std::borrow::Cow<'_, GradientPaint>, Runs)> {
    use std::borrow::Cow;
    if !g.gradient.is_spread() {
        return Some((Cow::Borrowed(g), None));
    }
    let geom = g.resolve(bounds);
    match g.gradient.padded_over(&geom, bounds) {
        Some(p) if p.capped => None,
        Some(p) => {
            let runs = Some((p.first, p.runs));
            Some((Cow::Owned(GradientPaint { gradient: p.gradient, geom: Some(p.geom), angle: p.geom.angle_deg(), ..g.clone() }), runs))
        }
        None => {
            let runs = Some((0.0, g.gradient.repeat_count() as usize));
            Some((Cow::Owned(GradientPaint { gradient: g.gradient.unrolled().into_owned(), geom: Some(geom), ..g.clone() }), runs))
        }
    }
}

/// The edges (gradient parameters, from `lo` to `hi`) of about `n` bands [`Frame::pieces`] cuts:
/// evenly spaced, or for stops repeated in `runs` ([`padded_on`]: where the first run starts and
/// how many runs lie evenly from there to 1) the same whole number of bands in each run between
/// `lo` and `hi` (at least one, about `n` in all), so every run starts on an edge and no band
/// holds two runs' colors.
fn band_edges(lo: f64, hi: f64, n: usize, runs: Option<(f64, usize)>) -> Vec<f64> {
    let n = n.max(1);
    let even = || {
        let step = (hi - lo) / n as f64;
        (0..=n).map(|k| lo + k as f64 * step).collect()
    };
    let Some((first, count)) = runs.filter(|&(first, count)| count > 0 && (0.0..1.0).contains(&first)) else { return even() };
    let run = (1.0 - first) / count as f64;
    let (from, to) = (lo.max(first), hi.min(1.0));
    let shown = (to - from) / run;
    if !(shown > 0.0 && shown.is_finite()) {
        return even();
    }
    let step = run / (n as f64 / shown).round().max(1.0);
    // Both whole numbers of steps from `first`, at most `n` plus the runs shown, plus one.
    let (k0, k1) = (((from - first) / step).ceil(), ((to - first) / step).floor());
    let mut edges = vec![lo];
    for k in (k0 as usize)..(k1 as usize).saturating_add(1) {
        let t = first + k as f64 * step;
        if t > edges.last().copied().unwrap_or(lo) + 1e-12 && t < hi - 1e-12 {
            edges.push(t);
        }
    }
    edges.push(hi);
    edges
}

/// Where the colour changes along a gradient between parameters `t0` and `t1`: both ends, every
/// stop between them (in its own colour, so coincident stops keep a sharp edge) and every midpoint
/// off the centre, as (parameter, colour, opacity) in order, with more between them where they
/// are over `max_gap` apart. The colour is linear in display RGB between neighbours, as the
/// canvas draws it.
fn breaks(g: &Gradient, t0: f64, t1: f64, max_gap: f64) -> Vec<(f64, (Color, f32))> {
    let at = |t: f64| (t, g.sample_keep_model(t as f32));
    let mut v = vec![at(t0)];
    // A Perceptual gradient, or one mixed around the hue wheel, curves between its stops: the mesh
    // takes a column at each of the stops its expansion adds, so the linear mix between columns
    // follows that curve.
    if !g.method.is_classic() {
        v.extend(g.expanded().map(|(o, ..)| f64::from(o)).filter(|o| *o > t0 && *o < t1).map(at));
    }
    for (i, s) in g.stops.iter().enumerate() {
        let o = s.offset as f64;
        if o > t0 && o < t1 {
            v.push((o, (s.color, s.opacity)));
        }
        if let Some(n) = g.stops.get(i + 1)
            && (s.midpoint - 0.5).abs() > 1e-3
        {
            let m = (s.offset + (n.offset - s.offset) * s.midpoint) as f64;
            if m > t0 && m < t1 {
                v.push(at(m));
            }
        }
    }
    v.push(at(t1));
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = Vec::with_capacity(v.len());
    for w in v.windows(2) {
        out.push(w[0]);
        let (a, b) = (w[0].0, w[1].0);
        let n = ((b - a) / max_gap).ceil() as usize;
        out.extend((1..n).map(|k| at(a + (b - a) * k as f64 / n as f64)));
    }
    out.extend(v.last().copied());
    out
}

/// A linear or radial gradient's geometry over an object's box: its start, the vector to its
/// end, the vector across it (radial: the ellipse's other axis), and the parameter ranges that
/// cover the box (linear: along the vector `t0..t1` and across it `s0..s1` in points; radial:
/// `0..t1`, each level's ellipse centred on the way from the focal point to the start).
/// A conic gradient also has its placement, the box and the radius in its frame that reaches
/// past the box's corners ([`Frame::pieces`] and [`Frame::mesh`] go around its center). A
/// diamond gradient has its frame ([`vectorcraft_color::GradientGeom::diamond_frame`]; `None`
/// when it has no extent), the box and the level of the square that reaches its corners.
struct Frame {
    conic: Option<(vectorcraft_color::GradientGeom, Rect, f64)>,
    diamond: Option<(Option<Affine>, Rect, f64)>,
    radial: bool,
    start: Point,
    focal: Point,
    along: Vec2,
    across: Vec2,
    t0: f64,
    t1: f64,
    s0: f64,
    s1: f64,
}

impl Frame {
    fn new(g: &GradientPaint, bounds: Rect) -> Self {
        let geom = g.resolve(bounds);
        let radial = g.gradient.kind == GradientKind::Radial;
        // A vector of no length paints as one pointing right (as the canvas does).
        let along = if geom.start.distance(geom.end) < 1e-9 { Vec2::new(1.0, 0.0) } else { geom.end - geom.start };
        let normal = Vec2::new(-along.y, along.x) / along.hypot();
        let across = if radial { normal * along.hypot() * geom.aspect } else { normal };
        let corners = [bounds.origin(), Point::new(bounds.x1, bounds.y0), Point::new(bounds.x1, bounds.y1), Point::new(bounds.x0, bounds.y1)];
        let range = |f: &dyn Fn(Point) -> f64| corners.iter().map(|c| f(*c)).fold((f64::MAX, f64::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)));
        let (mut t0, mut t1) = range(&|c| (c - geom.start).dot(along) / along.hypot2());
        let (s0, s1) = range(&|c| (c - geom.start).dot(normal));
        if radial {
            let geom = vectorcraft_color::GradientGeom { end: geom.start + along, ..geom };
            (t0, t1) = (0.0, range(&|c| geom.param_at(GradientKind::Radial, c)).1);
        }
        // Keep a sliver of span so the art maps back invertibly.
        t1 = t1.max(t0 + 1e-6);
        let conic = (g.gradient.kind == GradientKind::Conic).then(|| {
            let to_unit = geom.conic_frame().inverse();
            let reach = corners.iter().map(|c| (to_unit * *c).to_vec2().hypot()).fold(0.0f64, f64::max).max(1e-6);
            // Past the corners, and far enough out that the chords between columns at most
            // CONIC_MESH_GAP of a turn apart stay outside them.
            let reach = if reach.is_finite() { reach * 1.01 / (std::f64::consts::PI * CONIC_MESH_GAP).cos() } else { 1.0 };
            (geom, bounds, reach)
        });
        let diamond = (g.gradient.kind == GradientKind::Diamond).then(|| {
            let reach = corners.iter().map(|c| geom.param_at(GradientKind::Diamond, *c)).fold(0.0f64, f64::max);
            (geom.diamond_frame(), bounds, if reach.is_finite() { reach.max(1e-6) * 1.01 } else { 1.0 })
        });
        Self { conic, diamond, radial, start: geom.start, focal: geom.focal_point(), along, across, t0, t1, s0, s1 }
    }

    /// Linear: the point at parameter `t` along and `s` across. Radial: the point at parameter `t`
    /// and angle `s` (radians).
    fn at(&self, t: f64, s: f64) -> Point {
        if self.radial {
            self.focal + (self.start - self.focal + self.along * s.cos() + self.across * s.sin()) * t
        } else {
            self.start + self.along * t + self.across * s
        }
    }

    /// A gradient mesh covering the box: columns (linear) or rings (radial) where the colour
    /// changes, so it paints the gradient exactly.
    fn mesh(&self, g: &Gradient) -> GradientMesh {
        if let Some((geom, _, reach)) = &self.conic {
            // Columns around the center where the color changes, at most CONIC_MESH_GAP of a turn
            // apart so the outer edge stays past the box; the seam is two columns in one place.
            let br = breaks(g, 0.0, 1.0, CONIC_MESH_GAP);
            let us: Vec<f64> = br.iter().map(|(t, _)| *t).collect();
            let frame = geom.conic_frame();
            let at = |u: f64, v: f64| {
                let a = u * std::f64::consts::TAU;
                frame * Point::new(reach * v * a.cos(), reach * v * a.sin())
            };
            return GradientMesh::from_grid(&us, &[0.0, 1.0], &at, &|_, c, _| br[c].1);
        }
        if let Some((frame, b, reach)) = &self.diamond {
            let Some(frame) = frame else {
                // No extent: the whole box is the last stop.
                let last = g.sample_keep_model(1.0);
                let at = |u: f64, v: f64| Point::new(b.x0 + u * b.width(), b.y0 + v * b.height());
                return GradientMesh::from_grid(&[0.0, 1.0], &[0.0, 1.0], &at, &|_, _, _| last);
            };
            // Rings of squares where the color changes, with columns at the corners: each patch
            // lies between two rings along one side, where the level runs straight across it.
            let br = breaks(g, 0.0, *reach, f64::INFINITY);
            let ts: Vec<f64> = br.iter().map(|(t, _)| t / reach).collect();
            let at = |u: f64, v: f64| *frame * (diamond_perimeter(u).to_vec2() * (v * reach)).to_point();
            const CORNERS: [f64; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];
            let mut mesh = GradientMesh::from_grid(&CORNERS, &ts, &at, &|r, _, _| br[r].1);
            // Every column is at a corner, where `from_grid`'s tangent is the mean of the two
            // sides: each handle along a ring points a third of the way along its own side, so
            // the sides stay straight.
            let side = |v: f64, from: usize, to: Option<&f64>| to.map_or(vectorcraft_geom::Vec2::ZERO, |&u| (at(u, v) - at(CORNERS[from], v)) / 3.0);
            for (r, &v) in ts.iter().enumerate() {
                for c in 0..CORNERS.len() {
                    if let Some(point) = mesh.points.get_mut(r * CORNERS.len() + c) {
                        point.handles[0] = side(v, c, CORNERS.get(c + 1));
                        point.handles[1] = side(v, c, c.checked_sub(1).and_then(|i| CORNERS.get(i)));
                    }
                }
            }
            return mesh;
        }
        // Rings as close as the columns around them, so the mesh draws smoothly.
        let gap = if self.radial { (self.t1 - self.t0) / RADIAL_MESH_COLS as f64 } else { f64::INFINITY };
        let br = breaks(g, self.t0, self.t1, gap);
        let ts: Vec<f64> = br.iter().map(|(t, _)| (t - self.t0) / (self.t1 - self.t0)).collect();
        let t = |x: f64| self.t0 + x * (self.t1 - self.t0);
        if self.radial {
            let angles: Vec<f64> = (0..=RADIAL_MESH_COLS).map(|i| i as f64 / RADIAL_MESH_COLS as f64).collect();
            GradientMesh::from_grid(&angles, &ts, &|u, v| self.at(t(v), u * std::f64::consts::TAU), &|r, _, _| br[r].1)
        } else {
            GradientMesh::from_grid(&ts, &[0.0, 1.0], &|u, v| self.at(t(u), self.s0 + v * (self.s1 - self.s0)), &|_, c, _| br[c].1)
        }
    }

    /// `n` solid shapes covering the box in paint order, each coloured as the gradient in the
    /// middle of its band: for a linear gradient, rectangles from each band's start to the far
    /// end, for a radial one, nested ellipses (largest first), for a conic one, wedges around the
    /// center from each band's start to the seam, for a diamond one, nested squares (largest
    /// first). Each paints over the ones below except its band, so no seam shows between bands.
    /// With translucent stops each shape is just its band, so nothing doubles up. Stops repeated
    /// in `runs` ([`padded_on`]) get the same whole number of bands in each run ([`band_edges`]).
    fn pieces(&self, g: &Gradient, n: usize, runs: Option<(f64, usize)>) -> Vec<(PathData, (Color, f32))> {
        let opaque = g.stops.iter().all(|s| s.opacity >= 1.0);
        let color = |e: &[f64], k: usize| {
            let (a, b) = (e.get(k).copied().unwrap_or(0.0), e.get(k + 1).copied().unwrap_or(1.0));
            g.sample_keep_model(((a + b) / 2.0) as f32)
        };
        if let Some((geom, b, _)) = &self.conic {
            // Evenly spaced around the turn, as the wedges are.
            let n = band_edges(0.0, 1.0, n, runs).len().saturating_sub(1);
            return geom
                .conic_wedges(*b, n, opaque)
                .into_iter()
                .map(|(pts, t)| (PathData::single(SubPath::polyline(&pts, true)), g.sample_keep_model(t as f32)))
                .collect();
        }
        if let Some((frame, b, reach)) = &self.diamond {
            let Some(frame) = frame else {
                return vec![(shapes::rectangle(*b), g.sample_keep_model(1.0))];
            };
            // Nested squares about the center, largest first, as a radial gradient's ellipses.
            let edges = band_edges(0.0, *reach, n, runs);
            let square = |t: f64| -> SubPath {
                let pts: Vec<Point> = [0.0, 0.25, 0.5, 0.75].iter().map(|&u| *frame * (diamond_perimeter(u).to_vec2() * t).to_point()).collect();
                SubPath::polyline(&pts, true)
            };
            return (0..edges.len().saturating_sub(1))
                .rev()
                .map(|k| {
                    let mut shape = PathData::single(square(edges.get(k + 1).copied().unwrap_or(*reach)));
                    if !opaque && k > 0 {
                        let mut hole = square(edges.get(k).copied().unwrap_or(0.0));
                        hole.reverse();
                        shape.subpaths.push(hole);
                    }
                    (shape, color(&edges, k))
                })
                .collect();
        }
        let edges = band_edges(self.t0, self.t1, n, runs);
        let bands = edges.len().saturating_sub(1);
        let edge = |k: usize| edges.get(k).copied().unwrap_or(self.t1);
        if self.radial {
            let ellipse = |t: f64| {
                let c = self.focal + (self.start - self.focal) * t;
                let m = Affine::new([self.along.x * t, self.along.y * t, self.across.x * t, self.across.y * t, c.x, c.y]);
                shapes::ellipse(Rect::new(-1.0, -1.0, 1.0, 1.0)).transformed(m)
            };
            return (0..bands)
                .rev()
                .map(|k| {
                    let mut shape = ellipse(edge(k + 1));
                    if !opaque && k > 0 {
                        let mut hole = ellipse(edge(k)).subpaths.remove(0);
                        hole.reverse();
                        shape.subpaths.push(hole);
                    }
                    (shape, color(&edges, k))
                })
                .collect();
        }
        (0..bands)
            .map(|k| {
                let a = edge(k);
                let b = if opaque { self.t1 } else { edge(k + 1) };
                let pts = [self.at(a, self.s0), self.at(b, self.s0), self.at(b, self.s1), self.at(a, self.s1)];
                (PathData::single(SubPath::polyline(&pts, true)), color(&edges, k))
            })
            .collect()
    }
}

/// The point a fraction `u` (0..1) of the way around the square with corners (1, 0), (0, 1),
/// (-1, 0) and (0, -1), from the first: a diamond gradient's last stop in its frame.
fn diamond_perimeter(u: f64) -> Point {
    const CORNERS: [(f64, f64); 5] = [(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0), (1.0, 0.0)];
    let s = if u.is_finite() { u.clamp(0.0, 1.0) * 4.0 } else { 0.0 };
    let i = (s.floor() as usize).min(3);
    let f = s - i as f64;
    let ((x0, y0), (x1, y1)) = (CORNERS.get(i).copied().unwrap_or((1.0, 0.0)), CORNERS.get(i + 1).copied().unwrap_or((1.0, 0.0)));
    Point::new(x0 + (x1 - x0) * f, y0 + (y1 - y0) * f)
}
