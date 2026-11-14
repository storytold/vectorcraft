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
use vectorcraft_doc::{AppearanceItem, Document, Node, NodeKind, PatternDef, Symbol};

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
                if let Some(fonts) = &mut t.file_fonts {
                    fonts.map_keys(|key| f(Ref::Image(key)));
                }
            }
            NodeKind::Image(im) => f(Ref::Image(&mut im.key)),
            NodeKind::PlacedDocument(pl) => f(Ref::Image(&mut pl.key)),
            NodeKind::SymbolInstance { symbol, .. } => f(Ref::Symbol(symbol)),
            _ => {}
        }
        // A color-masked image's masked pixels come along, and the key they were made from
        // follows the image's.
        if let Some(cm) = n.extras_if_any().and_then(|e| e.color_mask.as_deref_mut()) {
            f(Ref::Image(&mut cm.key));
            f(Ref::Image(&mut cm.from));
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
/// (copied) or one that is the `same` (shared), else a free variant (`taken`, copied).
fn names<T>(
    used: &[String],
    def: impl Fn(&str) -> Option<T>,
    existing: impl Fn(&str) -> Option<T>,
    same: impl Fn(&T, &T) -> bool,
    taken: impl Fn(&str) -> bool,
) -> Renames {
    let mut out = Renames::new();
    for name in used {
        let Some(d) = def(name) else { continue };
        let entry = match existing(name) {
            None => (Some(name.clone()), true),
            Some(e) if same(&e, &d) => (Some(name.clone()), false),
            Some(_) => {
                (Some(unique_name(name, |c| taken(c) || used.iter().any(|u| u == c) || out.values().any(|(n, _)| n.as_deref() == Some(c)))), true)
            }
        };
        out.insert(name.clone(), entry);
    }
    out
}

/// `n` as `dst` would copy it in ([`Document::reid`]), with the ids a document of its own gives it:
/// art copied in once, which differs from the file's only in its ids, compares equal to the file's.
fn canon(dst: &Document, n: &Node) -> Node {
    let mut own = Document::new(0.0, 0.0);
    // Pixels the art carries take a key `dst` doesn't use for other pixels, as they do in `dst`.
    let mut keys = vec![];
    vectorcraft_doc::swatches::walk_with_masks(n, &mut |c| {
        if let NodeKind::Image(im) = &c.kind
            && im.pixels.is_some()
        {
            keys.push(im.key.clone());
        }
    });
    for (k, blob) in &dst.images {
        if keys.iter().any(|key| k == key || k.strip_prefix(key.as_str()).is_some_and(|rest| rest.starts_with('-'))) {
            own.images.insert(k.clone(), blob.clone());
        }
    }
    own.reid(n)
}

/// Do symbols `a` and `b` draw the same, whatever ids their art has ([`canon`])?
fn same_symbol(dst: &Document, a: &Symbol, b: &Symbol) -> bool {
    let own = |s: &Symbol| Symbol { art: Arc::new(canon(dst, &s.art)), ..s.clone() };
    own(a) == own(b)
}

/// Do patterns `a` and `b` draw the same, whatever ids their art has ([`canon`])?
fn same_pattern(dst: &Document, a: &PatternDef, b: &PatternDef) -> bool {
    let bare = |p: &PatternDef| PatternDef { art: vec![], ..p.clone() };
    bare(a) == bare(b) && a.art.len() == b.art.len() && a.art.iter().zip(&b.art).all(|(x, y)| canon(dst, x) == canon(dst, y))
}

/// Does art `a` use a resource that `m` (images, symbols, patterns, brushes, swatches) gives
/// another name, or a swatch whose link `m` drops?
fn uses_moved(a: &Node, m: [&Renames; 5]) -> bool {
    let moved = |map: &Renames, k: &str| map.get(k).is_some_and(|(new, _)| new.as_deref() != Some(k));
    let (mut a, mut any) = (a.clone(), false);
    refs(&mut a, &mut |r| {
        any |= match r {
            Ref::Image(k) => moved(m[0], k),
            Ref::Symbol(k) => moved(m[1], k),
            Ref::Pattern(k) => moved(m[2], k),
            Ref::Brush(k) => moved(m[3], k),
            Ref::Swatch(link) => link.as_deref().is_some_and(|k| moved(m[4], k)),
        }
    });
    any
}

/// Copy each of `shared` in `map` under a free name, as [`names`] does for a definition `dst`
/// holds differently.
fn unshare(map: &mut Renames, shared: &[String], used: &[String], taken: impl Fn(&str) -> bool) {
    for name in shared {
        let free = unique_name(name, |c| taken(c) || used.iter().any(|u| u == c) || map.values().any(|(n, _)| n.as_deref() == Some(c)));
        map.insert(name.clone(), (Some(free), true));
    }
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

    let images = names(&used.images, |k| src.images.get(k), |k| dst.images.get(k), |a, b| a == b, |c| dst.images.contains_key(c));
    // Symbol and pattern art copied in takes ids of `dst`, so their art is compared by what it
    // draws: placing the same file again shares what the first placement brought.
    let mut symbols = names(
        &used.symbols,
        |n| src.symbols.iter().find(|s| s.name == n),
        |n| dst.symbols.iter().find(|s| s.name == n),
        |a, b| same_symbol(dst, a, b),
        |c| dst.symbols.iter().any(|s| s.name == c),
    );
    let mut patterns = names(&used.patterns, |n| src.pattern(n), |n| dst.pattern(n), |a, b| same_pattern(dst, a, b), |c| dst.pattern(c).is_some());
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
    let mut brushes = names(
        &used.brushes,
        |n| src_brushes.iter().find(|b| b.name == n).map(|b| (b, moves_art(b))),
        |n| dst_brushes.iter().find(|b| b.name == n).map(|b| (b, false)),
        |a, b| a == b,
        |c| dst_brushes.iter().any(|b| b.name == c),
    );
    // Likewise a symbol, pattern or brush whose art uses a resource that moves isn't the
    // document's of that name: it is copied under a free name, which can move what uses it in turn.
    loop {
        let maps = [&images, &symbols, &patterns, &brushes, &swatches];
        let shared = |map: &Renames| map.iter().filter(|(_, (_, copy))| !*copy).map(|(k, _)| k.clone()).collect::<Vec<_>>();
        let sym: Vec<String> =
            shared(&symbols).into_iter().filter(|n| src.symbols.iter().any(|s| s.name == *n && uses_moved(&s.art, maps))).collect();
        let pat: Vec<String> =
            shared(&patterns).into_iter().filter(|n| src.pattern(n).is_some_and(|p| p.art.iter().any(|a| uses_moved(a, maps)))).collect();
        let bru: Vec<String> = shared(&brushes)
            .into_iter()
            .filter(|n| src_brushes.iter().find(|b| b.name == *n).is_some_and(|b| brush_art(&mut b.clone()).into_iter().any(|a| uses_moved(a, maps))))
            .collect();
        if sym.is_empty() && pat.is_empty() && bru.is_empty() {
            break;
        }
        unshare(&mut symbols, &sym, &used.symbols, |c| dst.symbols.iter().any(|s| s.name == c));
        unshare(&mut patterns, &pat, &used.patterns, |c| dst.pattern(c).is_some());
        unshare(&mut brushes, &bru, &used.brushes, |c| dst_brushes.iter().any(|b| b.name == c));
    }

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
    // Equal font bytes can reuse an existing image entry whose runtime lease is still empty.
    // Transfer the validated source lease too: cached text nodes hold only weak references.
    for (old, (new, copy)) in &images {
        if *copy {
            continue;
        }
        let Some(blob) = src.images.get(old).filter(|b| b.mime == vectorcraft_doc::FILE_FONT_MIME) else { continue };
        let Some(target) = new.as_ref().and_then(|key| dst.images.get_mut(key)) else { continue };
        let owner = blob.file_font.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(face) = owner.as_ref().filter(|face| std::ptr::eq(face.bytes(), blob.bytes.as_slice()) || face.bytes() == blob.bytes.as_slice()) {
            *target.file_font.get_mut().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(face.clone());
        }
    }
    for (old, new) in copies(&symbols) {
        if let Some(mut def) = src.symbols.iter().find(|s| s.name == old).cloned() {
            def.name = new.to_string();
            refs(Arc::make_mut(&mut def.art), &mut rename);
            // The art takes ids of `dst`: those of `src` may name objects of `dst` already.
            def.art = Arc::new(dst.reid(&def.art));
            dst.symbols.push(def);
        }
    }
    for (old, new) in copies(&patterns) {
        if let Some(mut def) = src.pattern(old).cloned() {
            def.name = new.to_string();
            for a in &mut def.art {
                refs(Arc::make_mut(a), &mut rename);
                *a = Arc::new(dst.reid(a));
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
