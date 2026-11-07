//! Resources placed art brings along: the images, symbols, pattern swatches, colour swatches and
//! brushes it uses (also through symbol, pattern and brush art and through tint swatches) join the
//! document. A name the document already uses for something different gets a free variant (`Star 2`,
//! `pdf-image-1 2`…) and the art's references follow; a colour linked to a swatch the document
//! defines differently keeps its colour, unlinked. Text styles are left as they are: the art carries
//! its own formatting.

use std::collections::BTreeMap;
use std::sync::Arc;

use vectorcraft_brush::{Brush, BrushKind};
use vectorcraft_color::Paint;
use vectorcraft_doc::{AppearanceItem, Document, Node, NodeKind};

use super::unique_name;

/// Visit `n` and every descendant (children and opacity-mask art) mutably.
pub(super) fn each_mut(n: &mut Node, f: &mut impl FnMut(&mut Node)) {
    f(n);
    if let Some(m) = n.extras_if_any().and_then(|e| e.mask.as_mut()) {
        each_mut(Arc::make_mut(&mut m.art), f);
    }
    if let Some(ch) = n.children_mut() {
        for c in ch {
            each_mut(Arc::make_mut(c), f);
        }
    }
}

/// A reference from art to a document resource.
enum Ref<'a> {
    Image(&'a mut String),
    Symbol(&'a mut String),
    Pattern(&'a mut String),
    Brush(&'a mut String),
    /// A colour's (or gradient's) swatch link; renaming it to `None` unlinks it.
    Swatch(&'a mut Option<String>),
}

/// Every resource reference held by paint `p`.
fn paint_refs(p: &mut Paint, f: &mut impl FnMut(Ref)) {
    match p {
        Paint::Pattern { pattern, .. } => f(Ref::Pattern(pattern)),
        Paint::Gradient(g) if g.swatch.is_some() => f(Ref::Swatch(&mut g.swatch)),
        _ => {}
    }
    p.map_links(&mut |_, link, _| {
        if link.is_some() {
            f(Ref::Swatch(link));
        }
        false
    });
}

/// Every resource reference in `n` and its descendants.
fn refs(n: &mut Node, f: &mut impl FnMut(Ref)) {
    each_mut(n, &mut |n| {
        for it in &mut n.appearance.items {
            match it {
                AppearanceItem::Fill(fl) => paint_refs(&mut fl.paint, f),
                AppearanceItem::Stroke(st) => {
                    paint_refs(&mut st.paint, f);
                    if let Some(brush) = &mut st.brush {
                        f(Ref::Brush(brush));
                    }
                }
            }
        }
        match &mut n.kind {
            NodeKind::Text(t) => {
                for r in &mut t.runs {
                    paint_refs(&mut r.style.fill, f);
                    paint_refs(&mut r.style.stroke, f);
                }
            }
            NodeKind::Image(im) => f(Ref::Image(&mut im.key)),
            NodeKind::PlacedDocument(pl) => f(Ref::Image(&mut pl.key)),
            NodeKind::SymbolInstance { symbol, .. } => f(Ref::Symbol(symbol)),
            _ => {}
        }
    });
}

/// Resource names in first-use order.
#[derive(Default)]
struct Used {
    images: Vec<String>,
    symbols: Vec<String>,
    patterns: Vec<String>,
    brushes: Vec<String>,
    swatches: Vec<String>,
}

impl Used {
    fn note(&mut self, r: Ref) {
        let (list, name) = match r {
            Ref::Image(k) => (&mut self.images, &*k),
            Ref::Symbol(k) => (&mut self.symbols, &*k),
            Ref::Pattern(k) => (&mut self.patterns, &*k),
            Ref::Brush(k) => (&mut self.brushes, &*k),
            Ref::Swatch(Some(k)) => (&mut self.swatches, &*k),
            Ref::Swatch(None) => return,
        };
        if !list.contains(name) {
            list.push(name.clone());
        }
    }
}

/// What each resource the art uses is called in the document (`None`: a swatch link to drop) and
/// whether its definition is copied in.
type Renames = BTreeMap<String, (Option<String>, bool)>;

/// Names in `dst` for the `used` definitions of `src` (`def`): the same name when `dst` has none
/// (copied) or an equal one (shared), else a free variant (`taken`, copied).
fn names<T: PartialEq>(
    used: &[String],
    def: impl Fn(&str) -> Option<T>,
    existing: impl Fn(&str) -> Option<T>,
    taken: impl Fn(&str) -> bool,
) -> Renames {
    let mut out = Renames::new();
    for name in used {
        let Some(d) = def(name) else { continue };
        let entry = match existing(name) {
            None => (Some(name.clone()), true),
            Some(e) if e == d => (Some(name.clone()), false),
            Some(_) => {
                (Some(unique_name(name, |c| taken(c) || used.iter().any(|u| u == c) || out.values().any(|(n, _)| n.as_deref() == Some(c)))), true)
            }
        };
        out.insert(name.clone(), entry);
    }
    out
}

/// The renamed resources whose definitions are copied: (name in `src`, name in `dst`).
fn copies(m: &Renames) -> Vec<(&str, &str)> {
    m.iter().filter(|(_, (_, copy))| *copy).filter_map(|(old, (new, _))| Some((old.as_str(), new.as_deref()?))).collect()
}

/// The art a brush holds.
fn brush_art(b: &mut Brush) -> Vec<&mut Node> {
    match &mut b.kind {
        BrushKind::Scatter(s) => vec![&mut s.art],
        BrushKind::Art(a) => vec![&mut a.art],
        BrushKind::Pattern(p) => {
            [Some(&mut p.side), p.outer_corner.as_mut(), p.inner_corner.as_mut(), p.start.as_mut(), p.end.as_mut()].into_iter().flatten().collect()
        }
        BrushKind::Calligraphic(_) | BrushKind::Bristle(_) => vec![],
    }
}

/// Bring the resources `nodes` (art from `src`) use into `dst`, renaming on conflict, and point
/// `nodes` at the names they got.
pub(crate) fn adopt(dst: &mut Document, src: &Document, nodes: &mut [Node]) {
    let mut used = Used::default();
    for n in nodes.iter_mut() {
        refs(n, &mut |r| used.note(r));
    }
    let src_brushes = vectorcraft_brush::library(src);
    // Symbol, pattern and brush art and tint swatches can use more resources (the lists grow while
    // they are walked).
    let (mut si, mut pi, mut wi, mut bi) = (0, 0, 0, 0);
    loop {
        let mut art: Vec<Node> = vec![];
        if let Some(name) = used.symbols.get(si) {
            si += 1;
            art.extend(src.symbols.iter().find(|s| s.name == *name).map(|s| (*s.art).clone()));
        } else if let Some(name) = used.patterns.get(pi) {
            pi += 1;
            art.extend(src.pattern(name).into_iter().flat_map(|p| p.art.iter().map(|a| (**a).clone())));
        } else if let Some(name) = used.swatches.get(wi) {
            wi += 1;
            if let Some(mut paint) = src.swatch(name).map(|w| w.paint.clone()) {
                paint_refs(&mut paint, &mut |r| used.note(r));
            }
        } else if let Some(name) = used.brushes.get(bi) {
            bi += 1;
            if let Some(mut b) = src_brushes.iter().find(|b| b.name == *name).cloned() {
                art.extend(brush_art(&mut b).into_iter().map(|a| a.clone()));
            }
        } else {
            break;
        }
        for mut a in art {
            refs(&mut a, &mut |r| used.note(r));
        }
    }

    let images = names(&used.images, |k| src.images.get(k), |k| dst.images.get(k), |c| dst.images.contains_key(c));
    let symbols = names(
        &used.symbols,
        |n| src.symbols.iter().find(|s| s.name == n),
        |n| dst.symbols.iter().find(|s| s.name == n),
        |c| dst.symbols.iter().any(|s| s.name == c),
    );
    let patterns = names(&used.patterns, |n| src.pattern(n), |n| dst.pattern(n), |c| dst.pattern(c).is_some());
    // A swatch the document defines differently isn't renamed (that would add look-alike swatches):
    // the colours linked to it keep their look, unlinked.
    let swatches: Renames = used
        .swatches
        .iter()
        .filter_map(|n| {
            let w = src.swatch(n)?;
            let entry = match dst.swatch(n) {
                None => (Some(n.clone()), true),
                Some(e) if e == w => (Some(n.clone()), false),
                Some(_) => (None, false),
            };
            Some((n.clone(), entry))
        })
        .collect();
    // A brush whose art uses a resource that moves to another name isn't the document's brush of
    // that name, whatever it looks like.
    let moved = |map: &Renames, k: &str| map.get(k).is_some_and(|(new, _)| new.as_deref() != Some(k));
    let moves_art = |b: &Brush| {
        let (mut b, mut any) = (b.clone(), false);
        for a in brush_art(&mut b) {
            refs(a, &mut |r| {
                any |= match r {
                    Ref::Image(k) => moved(&images, k),
                    Ref::Symbol(k) => moved(&symbols, k),
                    Ref::Pattern(k) => moved(&patterns, k),
                    Ref::Swatch(link) => link.as_deref().is_some_and(|k| moved(&swatches, k)),
                    Ref::Brush(_) => false,
                }
            });
        }
        any
    };
    let mut dst_brushes = vectorcraft_brush::library(dst);
    let brushes = names(
        &used.brushes,
        |n| src_brushes.iter().find(|b| b.name == n).map(|b| (b, moves_art(b))),
        |n| dst_brushes.iter().find(|b| b.name == n).map(|b| (b, false)),
        |c| dst_brushes.iter().any(|b| b.name == c),
    );

    let mut rename = |r: Ref| {
        let (map, k) = match r {
            Ref::Image(k) => (&images, k),
            Ref::Symbol(k) => (&symbols, k),
            Ref::Pattern(k) => (&patterns, k),
            Ref::Brush(k) => (&brushes, k),
            Ref::Swatch(link) => {
                if let Some((new, _)) = link.as_deref().and_then(|k| swatches.get(k)) {
                    *link = new.clone();
                }
                return;
            }
        };
        if let Some((Some(new), _)) = map.get(k.as_str()) {
            *k = new.clone();
        }
    };
    for n in nodes.iter_mut() {
        refs(n, &mut rename);
    }
    for (old, new) in copies(&images) {
        if let Some(blob) = src.images.get(old) {
            dst.images.insert(new.to_string(), blob.clone());
        }
    }
    for (old, new) in copies(&symbols) {
        if let Some(mut def) = src.symbols.iter().find(|s| s.name == old).cloned() {
            def.name = new.to_string();
            refs(Arc::make_mut(&mut def.art), &mut rename);
            dst.symbols.push(def);
        }
    }
    for (old, new) in copies(&patterns) {
        if let Some(mut def) = src.pattern(old).cloned() {
            def.name = new.to_string();
            for a in &mut def.art {
                refs(Arc::make_mut(a), &mut rename);
            }
            dst.patterns.push(def);
        }
    }
    let new_brushes = copies(&brushes);
    if !new_brushes.is_empty() {
        for (old, new) in new_brushes {
            if let Some(mut def) = src_brushes.iter().find(|b| b.name == old).cloned() {
                def.name = new.to_string();
                for a in brush_art(&mut def) {
                    refs(a, &mut rename);
                }
                dst_brushes.push(def);
            }
        }
        vectorcraft_brush::store(dst, &dst_brushes);
    }
    for (old, _) in copies(&swatches) {
        if let Some(mut w) = src.swatch(old).cloned() {
            paint_refs(&mut w.paint, &mut rename);
            dst.swatches.push(w);
        }
    }
}
