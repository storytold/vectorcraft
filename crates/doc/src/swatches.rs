//! Swatch lookups across colour groups and the walks over the paints that link to swatches.
//!
//! A swatch lives either in `Document::swatches` or in one of `Document::swatch_groups`; every
//! lookup here covers both. Solid paints and gradient stops link to a global swatch by name, at a
//! tint (`Paint::Solid.swatch` / `.tint`, `GradientStop.swatch` / `.tint`); a tint swatch is a
//! swatch whose own paint links to its base that way. [`Document::map_solid_paints`] rewrites
//! those links (and colours) wherever art and swatches hold them, copying only the nodes that
//! change.

use std::sync::Arc;

use vectorcraft_color::swatch::REGISTRATION;
use vectorcraft_color::{Color, Paint, Swatch, tint_percent};

use crate::{AppearanceItem, Document, Node, NodeId, NodeKind};

impl Document {
    /// Every swatch: the ungrouped ones first, then each colour group's in order.
    pub fn swatches_iter(&self) -> impl Iterator<Item = &Swatch> {
        self.swatches.iter().chain(self.swatch_groups.iter().flat_map(|g| g.swatches.iter()))
    }
    pub fn swatches_iter_mut(&mut self) -> impl Iterator<Item = &mut Swatch> {
        self.swatches.iter_mut().chain(self.swatch_groups.iter_mut().flat_map(|g| g.swatches.iter_mut()))
    }
    /// Swatch `name`, the built-in Registration swatch ([`vectorcraft_color::swatch::registration`])
    /// included.
    pub fn swatch(&self, name: &str) -> Option<&Swatch> {
        self.swatches_iter().find(|s| s.name == name).or_else(|| (name == REGISTRATION).then(vectorcraft_color::swatch::registration))
    }
    pub fn swatch_mut(&mut self, name: &str) -> Option<&mut Swatch> {
        self.swatches_iter_mut().find(|s| s.name == name)
    }
    /// The colour of global (or spot) swatch `name`, which its tints scale ([`Document::linked_color`]
    /// of its own); `None` when `name` isn't a global solid colour of its own (a tint swatch isn't).
    pub fn global_color(&self, name: &str) -> Option<Color> {
        let w = self.swatch(name).filter(|w| w.global)?;
        match &w.paint {
            Paint::Solid { color, swatch: None, .. } => Some(self.linked_color(*color, w.spot)),
            _ => None,
        }
    }
    /// The colour art linked to a swatch of `color` shows and prints from: a Lab spot colour is
    /// its working-CMYK equivalent when the Spot Colors options use CMYK values
    /// ([`Document::spot_use_lab`] off); any other colour is itself.
    pub fn linked_color(&self, color: Color, spot: bool) -> Color {
        match color {
            Color::Lab { .. } if spot && !self.spot_use_lab => {
                let cms = vectorcraft_color::cms::active();
                cms.convert(&color, vectorcraft_color::cms::Model::Cmyk, cms.settings().intent)
            }
            _ => color,
        }
    }
    /// The link a colour taken from swatch `name` gets: a global solid colour links to itself at
    /// 100 %, a tint swatch to its base at its tint; `None` for other swatches.
    pub fn swatch_link(&self, name: &str) -> Option<(String, f32)> {
        let w = self.swatch(name)?;
        match (w.tint_of(), &w.paint) {
            (Some((base, tint)), _) => Some((base.to_string(), tint)),
            (None, Paint::Solid { .. }) if w.global => Some((name.to_string(), 1.0)),
            _ => None,
        }
    }
    /// Tint `tint` (0..1) of global swatch `name` as a solid paint linked to it.
    pub fn tint_paint(&self, name: &str, tint: f32) -> Option<Paint> {
        let tint = tint.clamp(0.0, 1.0);
        Some(Paint::Solid { color: self.global_color(name)?.tinted(tint), swatch: Some(name.to_string()), tint })
    }
    /// Index of the colour group holding swatch `name` (`None` when ungrouped or missing).
    pub fn swatch_group_of(&self, name: &str) -> Option<usize> {
        self.swatch_groups.iter().position(|g| g.swatches.iter().any(|s| s.name == name))
    }
    /// Is `name` used by a swatch or a colour group (they share one namespace)?
    pub fn swatch_name_taken(&self, name: &str) -> bool {
        self.swatch(name).is_some() || self.swatch_groups.iter().any(|g| g.name == name)
    }
    /// `base`, or the first of "base 2", "base 3"… no swatch or colour group uses.
    pub fn free_swatch_name(&self, base: &str) -> String {
        if !self.swatch_name_taken(base) {
            return base.to_string();
        }
        self.numbered_swatch_name(base, 2)
    }
    /// The first of "base `from`", "base `from + 1`"… no swatch or colour group uses.
    fn numbered_swatch_name(&self, base: &str, from: usize) -> String {
        (from..).map(|i| format!("{base} {i}")).find(|n| !self.swatch_name_taken(n)).unwrap_or_else(|| base.to_string())
    }
    /// The name a new swatch of `paint` gets by default, unused in the document: a colour's values
    /// ([`color_name`], made free like [`Document::free_swatch_name`]), or "New Gradient Swatch 1",
    /// "New Pattern Swatch 1"…
    pub fn new_swatch_name(&self, paint: &Paint) -> String {
        let base = match paint {
            Paint::Solid { swatch: Some(n), tint, .. } if *tint < 1.0 => return self.free_swatch_name(&format!("{n} {}%", tint_percent(*tint))),
            Paint::Solid { color, .. } => return self.free_swatch_name(&color_name(*color)),
            Paint::Gradient(_) => "New Gradient Swatch",
            _ => "New Pattern Swatch",
        };
        self.numbered_swatch_name(base, 1)
    }
    /// Remove swatch `name` from wherever it lives.
    pub fn remove_swatch(&mut self, name: &str) -> Option<Swatch> {
        if let Some(i) = self.swatches.iter().position(|s| s.name == name) {
            return Some(self.swatches.remove(i));
        }
        let g = self.swatch_group_of(name)?;
        let list = &mut self.swatch_groups[g].swatches;
        let i = list.iter().position(|s| s.name == name)?;
        Some(list.remove(i))
    }

    /// Visit every colour that can link to a swatch — solid paints and gradient stops of fills,
    /// strokes and text runs — in the art, symbol definitions, pattern tiles, graphic styles and the
    /// swatches themselves (tint swatches, gradient swatches' stops). `f(colour, link, tint)` may
    /// change them and returns true when it did ([`Paint::map_links`]). Only nodes holding a changed
    /// paint (and the paths to them) are copied. Returns the number of changed paints.
    pub fn map_solid_paints(&mut self, f: &mut LinkFn) -> usize {
        let mut n = map_trees(&mut self.layers, f);
        for s in &mut self.symbols {
            n += map_tree(&mut s.art, f);
        }
        for p in &mut self.patterns {
            n += map_trees(&mut p.art, f);
        }
        for gs in &mut self.graphic_styles {
            for it in &mut gs.appearance.items {
                n += usize::from(item_paint_mut(it).map_links(f));
            }
        }
        for w in self.swatches_iter_mut() {
            n += usize::from(w.paint.map_links(f));
        }
        n
    }

    /// [`Document::map_solid_paints`] limited to the art in the subtrees of `ids` (each paint
    /// visited once even when an id is inside another).
    pub fn map_solid_paints_in(&mut self, ids: &[NodeId], f: &mut LinkFn) -> usize {
        let roots: Vec<NodeId> =
            ids.iter().copied().filter(|id| !self.ancestry(*id).is_some_and(|a| a[..a.len() - 1].iter().any(|p| ids.contains(p)))).collect();
        let mut n = 0;
        for id in roots {
            let Some(node) = self.node(id) else { continue };
            if let Some((new, count)) = map_node(node, f)
                && let Some(slot) = self.node_mut(id)
            {
                *slot = new;
                n += count;
            }
        }
        n
    }
}

impl Document {
    /// Visit every paint held by the art, symbol definitions, pattern tiles and graphic styles
    /// (fills, strokes and text runs), and the colour of every gradient-mesh point as a solid paint.
    pub fn visit_paints(&self, f: &mut dyn FnMut(&Paint)) {
        let mut tree = |n: &Node| {
            n.walk(&mut |m| {
                node_paints(m).for_each(&mut *f);
                if let NodeKind::Mesh(mesh) = &m.kind {
                    mesh.points.iter().for_each(|p| f(&Paint::solid(p.color)));
                }
            })
        };
        self.layers
            .iter()
            .chain(self.symbols.iter().map(|s| &s.art))
            .chain(self.graph_designs.iter().map(|g| &g.art))
            .chain(self.patterns.iter().flat_map(|p| &p.art))
            .for_each(|n| tree(n));
        self.graphic_styles.iter().flat_map(|gs| &gs.appearance.items).for_each(|it| f(item_paint(it)));
    }
}

/// The default name of a solid swatch: its values in its own colour model ("C=10 M=20 Y=30 K=0",
/// "R=255 G=128 B=0", "Gray K=40", "L=52 a=70 b=-30").
pub fn color_name(c: Color) -> String {
    // Clamped first, so a conversion's tiny negative never reads "-0".
    let pct = |v: f32| (v.clamp(0.0, 1.0) * 100.0).round();
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round();
    match c {
        Color::Cmyk { c, m, y, k } => format!("C={} M={} Y={} K={}", pct(c), pct(m), pct(y), pct(k)),
        Color::Rgb { r, g, b } => format!("R={} G={} B={}", byte(r), byte(g), byte(b)),
        Color::Gray { k } => format!("Gray K={}", pct(k)),
        // `+ 0.0` turns a rounded -0 into 0.
        Color::Lab { l, a, b } => format!("L={} a={} b={}", l.round() + 0.0, a.round() + 0.0, b.round() + 0.0),
    }
}

/// What [`Document::map_solid_paints`] applies to each linkable colour: `(colour, link, tint)`,
/// true when it changed them.
pub type LinkFn<'a> = dyn FnMut(&mut Color, &mut Option<String>, &mut f32) -> bool + 'a;

/// Every colour in the subtree of `n` with the swatch it links to: solid fills, strokes, text runs
/// and gradient stops (with their link), then mesh points (unlinked), in paint order.
pub fn node_colors(n: &Node, f: &mut dyn FnMut(&Color, Option<&str>)) {
    n.walk(&mut |m| {
        for p in node_paints(m) {
            match p {
                Paint::Solid { color, swatch, .. } => f(color, swatch.as_deref()),
                Paint::Gradient(g) => g.gradient.stops.iter().for_each(|s| f(&s.color, s.swatch.as_deref())),
                _ => {}
            }
        }
        if let NodeKind::Mesh(mesh) = &m.kind {
            mesh.points.iter().for_each(|p| f(&p.color, None));
        }
    });
}

fn item_paint(it: &AppearanceItem) -> &Paint {
    match it {
        AppearanceItem::Fill(l) => &l.paint,
        AppearanceItem::Stroke(l) => &l.paint,
    }
}

fn item_paint_mut(it: &mut AppearanceItem) -> &mut Paint {
    match it {
        AppearanceItem::Fill(l) => &mut l.paint,
        AppearanceItem::Stroke(l) => &mut l.paint,
    }
}

/// The paints a node holds itself (not its children): appearance items, then text runs.
fn node_paints(n: &Node) -> impl Iterator<Item = &Paint> {
    let runs = match &n.kind {
        NodeKind::Text(t) => t.runs.as_slice(),
        _ => &[],
    };
    n.appearance.items.iter().map(item_paint).chain(runs.iter().flat_map(|r| [&r.style.fill, &r.style.stroke]))
}

/// Mutable [`node_paints`], in the same order.
fn node_paints_mut(n: &mut Node) -> impl Iterator<Item = &mut Paint> {
    let runs = match &mut n.kind {
        NodeKind::Text(t) => t.runs.as_mut_slice(),
        _ => &mut [],
    };
    n.appearance.items.iter_mut().map(item_paint_mut).chain(runs.iter_mut().flat_map(|r| [&mut r.style.fill, &mut r.style.stroke]))
}

/// `n` with `f` applied to its subtree's linkable colours, or `None` when nothing changed (then
/// nothing was copied). Also returns the number of changed paints.
fn map_node(n: &Node, f: &mut LinkFn) -> Option<(Node, usize)> {
    // Try each own solid or gradient paint on a scratch copy first, so unchanged nodes are never
    // cloned.
    let mut changes: Vec<(usize, Paint)> = vec![];
    for (i, p) in node_paints(n).enumerate() {
        if matches!(p, Paint::Solid { .. } | Paint::Gradient(_)) {
            let mut q = p.clone();
            if q.map_links(f) {
                changes.push((i, q));
            }
        }
    }
    let mut count = changes.len();
    let mut out: Option<Node> = None;
    if !changes.is_empty() {
        let mut m = n.clone();
        let mut changes = changes.into_iter().peekable();
        for (i, p) in node_paints_mut(&mut m).enumerate() {
            if changes.peek().is_some_and(|c| c.0 == i)
                && let Some((_, q)) = changes.next()
            {
                *p = q;
            }
        }
        out = Some(m);
    }
    if let Some(ch) = n.children() {
        for (i, c) in ch.iter().enumerate() {
            if let Some((new, k)) = map_node(c, f) {
                let m = out.get_or_insert_with(|| n.clone());
                if let Some(slot) = m.children_mut().and_then(|v| v.get_mut(i)) {
                    *slot = Arc::new(new);
                }
                count += k;
            }
        }
    }
    out.map(|m| (m, count))
}

fn map_tree(n: &mut Arc<Node>, f: &mut LinkFn) -> usize {
    match map_node(n, f) {
        Some((new, count)) => {
            *n = Arc::new(new);
            count
        }
        None => 0,
    }
}

fn map_trees(v: &mut [Arc<Node>], f: &mut LinkFn) -> usize {
    v.iter_mut().map(|n| map_tree(n, f)).sum()
}

#[cfg(test)]
mod tests {
    use vectorcraft_color::SwatchGroup;
    use vectorcraft_geom::{Rect, shapes};

    use super::*;
    use crate::Appearance;

    fn linked(color: Color, name: &str) -> Paint {
        Paint::Solid { color, swatch: Some(name.into()), tint: 1.0 }
    }

    /// A document with a grouped global swatch "Brand", a rectangle filled with it and an unlinked one.
    fn doc() -> (Document, NodeId, NodeId) {
        let mut d = Document::new(100.0, 100.0);
        let red = Color::rgb(1.0, 0.0, 0.0);
        d.swatch_groups.push(SwatchGroup {
            name: "Mine".into(),
            swatches: vec![Swatch { name: "Brand".into(), paint: Paint::solid(red), global: true, spot: false }],
        });
        let layer = d.layers[0].id;
        let mut ids = vec![];
        for paint in [linked(red, "Brand"), Paint::solid(red)] {
            let id = d.alloc_id();
            let r = shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0));
            d.insert(Some(layer), usize::MAX, Node::path(id, r, Appearance::basic(paint, Paint::None, 0.0))).unwrap();
            ids.push(id);
        }
        (d, ids[0], ids[1])
    }

    #[test]
    fn lookups_cover_colour_groups() {
        let (mut d, _, _) = doc();
        let total = d.swatches.len() + d.swatch_groups.iter().map(|g| g.swatches.len()).sum::<usize>();
        assert_eq!(d.swatches_iter().count(), total);
        assert!(d.swatch("Brand").is_some_and(|s| s.global));
        assert_eq!(d.swatch_group_of("Brand").map(|g| d.swatch_groups[g].name.as_str()), Some("Mine"));
        d.swatch_mut("Brand").unwrap().spot = true;
        assert!(d.swatch("Brand").unwrap().spot);
        assert!(d.swatch_name_taken("Mine") && d.swatch_name_taken("Brand") && !d.swatch_name_taken("Nope"));
        assert_eq!(d.remove_swatch("Brand").map(|s| s.name), Some("Brand".into()));
        assert!(d.swatch("Brand").is_none() && d.remove_swatch("Brand").is_none());
    }

    #[test]
    fn default_names_follow_the_colour_model() {
        assert_eq!(color_name(Color::cmyk(0.1, 0.2, 0.3, -1e-7)), "C=10 M=20 Y=30 K=0");
        assert_eq!(color_name(Color::rgb(1.0, 0.5, -1e-7)), "R=255 G=128 B=0");
        assert_eq!(color_name(Color::Gray { k: 0.4 }), "Gray K=40");
        let (d, _, _) = doc();
        assert_eq!(d.free_swatch_name("Brand"), "Brand 2");
        assert_eq!(d.free_swatch_name("Mine"), "Mine 2", "colour groups share the namespace");
        assert_eq!(d.new_swatch_name(&Paint::solid(Color::rgb(1.0, 0.0, 0.0))), "R=255 G=0 B=0");
    }

    #[test]
    fn mapping_copies_only_changed_nodes() {
        let (mut d, a, b) = doc();
        let before = d.clone();
        let blue = Color::rgb(0.0, 0.0, 1.0);
        let n = d.map_solid_paints(&mut |c, s, _| {
            if s.as_deref() != Some("Brand") {
                return false;
            }
            *c = blue;
            true
        });
        assert_eq!(n, 1);
        assert_eq!(d.node(a).unwrap().appearance.fill_paint(), linked(blue, "Brand"));
        assert_eq!(d.node(b).unwrap().appearance.fill_paint(), Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
        let unchanged = |d: &Document| d.layers[0].children().unwrap()[1].clone();
        assert!(Arc::ptr_eq(&unchanged(&d), &unchanged(&before)), "the unlinked rectangle is shared, not copied");
        // Nothing to change: the tree is untouched.
        let again = d.clone();
        assert_eq!(d.map_solid_paints(&mut |_, _, _| false), 0);
        assert!(Arc::ptr_eq(&d.layers[0], &again.layers[0]));
    }

    #[test]
    fn mapping_within_ids_visits_each_paint_once() {
        let (mut d, a, b) = doc();
        let layer = d.layers[0].id;
        let mut seen = 0;
        let n = d.map_solid_paints_in(&[layer, a], &mut |_, s, _| {
            seen += 1;
            s.take().is_some()
        });
        assert_eq!((n, seen), (1, 2), "the layer covers `a`: each fill is visited once");
        assert_eq!(d.node(a).unwrap().appearance.fill_paint(), Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
        let mut colors = vec![];
        node_colors(d.node(b).unwrap(), &mut |c, s| colors.push((*c, s.map(str::to_string))));
        assert_eq!(colors, [(Color::rgb(1.0, 0.0, 0.0), None)]);
    }
}
