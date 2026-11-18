//! OpenType SVG glyphs: the art of a font's glyph from the SVG document its `SVG ` table holds for
//! it, made as the importer makes art of any SVG file. The text crate asks for it through the
//! reader the app registers ([`vectorcraft_text::set_svg_glyph_reader`]).

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use craft_svg::tree::usvg::roxmltree;
use vectorcraft_doc::color::Color;
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_geom::Affine;

use crate::ImportOptions;

/// The most bytes a glyph's document may have, unpacked.
const MAX_SVG_GLYPH_BYTES: u64 = 1 << 20;

/// Elements left out of a glyph, as the `SVG ` table rules them out: text (type inside type),
/// embedded content and scripts (and images of SVG documents, in [`glyph`]).
const LEFT_OUT: [&str; 3] = ["text", "foreignObject", "script"];

const XLINK: &str = "http://www.w3.org/1999/xlink";

/// Glyph `gid`'s art from its SVG `document` (possibly gzipped), in a font of `upem` units per em:
/// its element `glyph<gid>` (or the whole document when that is its root), in font units (y down),
/// the root's `viewBox` (or else its size) fitted into the em square by its larger side, centred.
/// The text's colour is `fill`: `currentColor`, and the legacy `context-fill` and `context-stroke`
/// (none without a `stroke`). Its nodes have no ids and its images their own pixels
/// ([`own_art`]). With the art, whether it uses the text's colours. `None` when the document doesn't read, is too large or has
/// no such element.
pub fn glyph_art(document: &[u8], gid: u32, upem: f64, fill: Color, stroke: Option<Color>) -> Option<(Node, bool)> {
    glyph_art_with(document, gid, upem, fill, stroke, true)
}

/// [`glyph_art`], the importer reading only what the glyph refers to when `reduce` ([`glyph`]).
fn glyph_art_with(document: &[u8], gid: u32, upem: f64, fill: Color, stroke: Option<Color>, reduce: bool) -> Option<(Node, bool)> {
    // The SVGZ budget left is the glyph's cap.
    let unpacked = AtomicU64::new(crate::MAX_SVGZ.saturating_sub(MAX_SVG_GLYPH_BYTES));
    let text = crate::text_within(document, &unpacked).ok()?;
    if text.len() as u64 > MAX_SVG_GLYPH_BYTES {
        return None;
    }
    let id = format!("glyph{gid}");
    let hex = |c: Color| {
        let [r, g, b, _] = c.to_rgba8(1.0);
        format!("#{r:02x}{g:02x}{b:02x}")
    };
    let Glyph { svg, whole, frame } = glyph(&text, &id, &hex(fill), reduce)?;
    let coloured = ["currentColor", "currentcolor", "context-fill", "context-stroke"].iter().any(|k| svg.contains(k));
    // usvg resolves `context-fill` and `context-stroke` only for markers.
    let svg = svg.replace("context-fill", &hex(fill)).replace("context-stroke", &stroke.map_or_else(|| "none".into(), hex));
    let (doc, _) = crate::import::import(&svg, &ImportOptions::default(), crate::MAX_NODES).ok()?;
    // A layer's art, its ruler guides left out.
    let art_of = |l: &Node| l.children().into_iter().flatten().filter(|c| !c.is_ruler_guide()).cloned().collect::<Vec<_>>();
    let mut art = if whole {
        Node::group(NodeId(0), doc.layers.iter().map(|l| Arc::new(Node::group(NodeId(0), art_of(l)))).collect())
    } else {
        // The glyph, top-level, is a layer when a `<g>`, else an object named after its id.
        let named = |n: &Node| n.name.as_deref() == Some(id.as_str());
        match doc.layers.iter().find(|l| named(l)) {
            Some(layer) => Node::group(NodeId(0), art_of(layer)),
            None => {
                let mut found = None;
                for layer in &doc.layers {
                    layer.walk(&mut |n| {
                        if found.is_none() && !n.is_ruler_guide() && named(n) {
                            found = Some(n.clone());
                        }
                    });
                }
                found?
            }
        }
    };
    // Without a size of its own, the document's viewport is its frame at one unit per unit.
    if let Some((w, h)) = frame.filter(|(w, h)| *w > 0.0 && *h > 0.0 && w.is_finite() && h.is_finite()) {
        let s = upem / w.max(h);
        art.transform(Affine::translate(((upem - w * s) / 2.0, (upem - h * s) / 2.0)) * Affine::scale(s), true);
    }
    Some((own_art(&art, &doc)?, coloured))
}

/// A glyph's document made ready for the importer.
struct Glyph {
    svg: String,
    /// The glyph is the whole document (its root has the glyph's id).
    whole: bool,
    /// The width and height of the root's `viewBox`, or else of its size, if it has either.
    frame: Option<(f64, f64)>,
}

/// `svg` with only what glyph `id` draws, as if the root `use`d it: the rest of the document (a
/// font may keep all its glyphs in one) only defines what it refers to, and what its parents apply
/// doesn't reach it. Without the [`LEFT_OUT`] elements and event attributes, the root without a
/// size (the em square is its frame) and in the text's color `color`. With `reduce`, the `defs`
/// hold only what the glyph refers to ([`referred`]), when that can be followed; else the whole
/// rest of the document. `None` when it doesn't read or has no element `id`.
fn glyph(svg: &str, id: &str, color: &str, reduce: bool) -> Option<Glyph> {
    // Before the XML reader: it nests by recursion and expands entities without a total limit.
    if crate::import::xml_too_deep(svg) || crate::entities::expansion_too_large(svg) {
        return None;
    }
    let xml = roxmltree::Document::parse_with_options(svg, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() }).ok()?;
    let root = xml.root_element();
    let whole = root.attribute("id") == Some(id);
    let target = if whole { root } else { xml.descendants().find(|n| n.is_element() && n.attribute("id") == Some(id))? };
    let at = target.range();
    // Edits: byte ranges to cut and what to put in their place, in order.
    let mut edits: Vec<(Range<usize>, String)> = vec![];
    // Cut with the left-out elements: images of SVG documents, as the table asks (a glyph draws no
    // documents inside it). Their link is `href`, else `xlink:href`, as the importer reads it. Any
    // data URL not typed as a raster goes too: the importer reads an untyped one (`text/plain`) as
    // SVG, SVGZ as well, when its bytes aren't a raster.
    let svg_image = |n: &roxmltree::Node| {
        let data = link(*n).map(|h| h.value()).map(|h| h.trim_start()).filter(|h| h.get(..5).is_some_and(|v| v.eq_ignore_ascii_case("data:")));
        let raster = |h: &str| {
            ["data:image/png", "data:image/jpeg", "data:image/jpg", "data:image/gif", "data:image/webp"]
                .iter()
                .any(|t| h.get(..t.len()).is_some_and(|v| v.eq_ignore_ascii_case(t)) && h[t.len()..].starts_with([';', ',']))
        };
        n.tag_name().name() == "image" && data.is_some_and(|h| !raster(h))
    };
    let cut = |n: &roxmltree::Node| n.is_element() && (LEFT_OUT.contains(&n.tag_name().name()) || svg_image(n));
    for n in root.descendants().filter(cut) {
        edits.push((n.range(), String::new()));
    }
    for a in root.attributes().filter(|a| matches!(a.name(), "width" | "height" | "color")) {
        edits.push((a.range(), String::new()));
    }
    // A glyph is art: event attributes (scripts) go too.
    for a in root.descendants().filter(|n| n.is_element()).flat_map(|n| n.attributes()).filter(|a| a.name().starts_with("on")) {
        edits.push((a.range(), String::new()));
    }
    // The text's colour, right after the root's name.
    let start = root.range().start + 1;
    let name = start + svg.get(start..)?.find(|c: char| c.is_whitespace() || c == '>' || c == '/')?;
    edits.push((name..name, format!(" color=\"{color}\"")));
    // The namespaces the glyph's parents declare, which it leaves: the root declares them too, or the
    // glyph itself, where the root gives the prefix (or the default) another one.
    let own = target.range().start + 1;
    let own = own + svg.get(own..)?.find(|c: char| c.is_whitespace() || c == '>' || c == '/')?;
    for ns in target.namespaces() {
        let uri = crate::xml_escape(ns.uri());
        let at_root = root.lookup_namespace_uri(ns.name());
        match ns.name() {
            Some(prefix) if at_root.is_none() => edits.push((name..name, format!(" xmlns:{prefix}=\"{uri}\""))),
            _ if whole || at_root == Some(ns.uri()) || declares(svg, target, ns.name())? => {}
            Some(prefix) => edits.push((own..own, format!(" xmlns:{prefix}=\"{uri}\""))),
            None => edits.push((own..own, format!(" xmlns=\"{uri}\""))),
        }
    }
    edits.sort_by_key(|(r, _)| (r.start, r.end));
    let out = if whole {
        edited(svg, 0..svg.len(), &edits)?
    } else {
        // The root's content goes in `defs`, and the glyph after it, on its own.
        let content = root.first_child()?.range().start;
        let close = svg.get(..root.range().end)?.rfind("</")?;
        let defs = match reduce.then(|| referred(svg, &xml, target, cut)).flatten() {
            Some(pieces) => pieces.into_iter().map(|r| edited(svg, r, &edits)).collect::<Option<String>>()?,
            None => [edited(svg, content..at.start, &edits)?, edited(svg, at.end..close, &edits)?].concat(),
        };
        [
            edited(svg, 0..content, &edits)?,
            "<defs>".into(),
            defs,
            "</defs>".into(),
            edited(svg, at.clone(), &edits)?,
            edited(svg, close..svg.len(), &edits)?,
        ]
        .concat()
    };
    let number = |v: &str| v.trim().trim_end_matches("px").parse::<f64>().ok();
    let view_box = root.attribute("viewBox").and_then(|v| {
        let n: Vec<f64> = v.split(|c: char| c.is_whitespace() || c == ',').filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect();
        match n.as_slice() {
            [_, _, w, h] => Some((*w, *h)),
            _ => None,
        }
    });
    let size = || Some((number(root.attribute("width")?)?, number(root.attribute("height")?)?));
    Some(Glyph { svg: out, whole, frame: view_box.or_else(size) })
}

/// Whether the start tag of `element` in `svg` declares namespace `prefix` (`None`: the default
/// one). `None` when the tag doesn't read.
fn declares(svg: &str, element: roxmltree::Node, prefix: Option<&str>) -> Option<bool> {
    let want = prefix.map_or_else(|| "xmlns".to_string(), |p| format!("xmlns:{p}"));
    let tag = svg.get(element.range().start + 1..)?;
    let mut rest = tag.trim_start_matches(|c: char| !(c.is_whitespace() || c == '>' || c == '/'));
    loop {
        rest = rest.trim_start();
        if rest.is_empty() || rest.starts_with(['>', '/']) {
            return Some(false);
        }
        let eq = rest.find('=')?;
        let value = rest.get(eq + 1..)?.trim_start();
        let quote = value.chars().next().filter(|q| matches!(q, '"' | '\''))?;
        let close = 1 + value.get(1..)?.find(quote)?;
        if rest.get(..eq)?.trim_end() == want {
            return Some(true);
        }
        rest = value.get(close + 1..)?;
    }
}

/// The byte ranges of `svg` (whose XML is `xml`) that the `defs` of glyph element `target` need, in
/// order: the elements it refers to with `url(#id)` (in an attribute, a `style` or a sheet) or
/// `href="#id"`, those they refer to in turn, and every `<style>` sheet, each whole, with the start
/// and end tags of the elements holding them (and what those refer to), so what they inherit and the
/// sheets' rules match as in the whole document. `None` when that can't be followed and the whole
/// rest of the document goes in: a document with a DTD or an `xml:base`, ids used twice, a
/// reference to an element that isn't there, holds the glyph or is inside one `cut` takes out, a
/// link to another file, a reference with an escape, or a sheet with selectors that look at
/// siblings ([`plain_sheet`]). Each element is queued and read at most once.
fn referred<'a, 'i>(
    svg: &str,
    xml: &'a roxmltree::Document<'i>,
    target: roxmltree::Node<'a, 'i>,
    cut: impl Fn(&roxmltree::Node<'a, 'i>) -> bool,
) -> Option<Vec<Range<usize>>> {
    if svg.contains("<!DOCTYPE") || target.ancestors().skip(1).any(|a| cut(&a)) {
        return None;
    }
    let root = xml.root_element();
    // Whether element `inner` is `outer` or inside it.
    let within =
        |outer: roxmltree::Node, inner: roxmltree::Node| outer.range().start <= inner.range().start && inner.range().end <= outer.range().end;
    let mut ids = HashMap::new();
    let mut wanted = vec![];
    // The elements `cut` takes out with what they hold, found in document order.
    let mut left_out = HashSet::new();
    let mut cut_end = 0;
    for n in xml.descendants().filter(|n| n.is_element()) {
        if n.attribute(("http://www.w3.org/XML/1998/namespace", "base")).is_some() {
            return None;
        }
        if let Some(id) = n.attribute("id")
            && ids.insert(id, n).is_some()
        {
            return None;
        }
        if n.range().start >= cut_end && cut(&n) {
            cut_end = n.range().end;
        }
        if n.range().start < cut_end {
            left_out.insert(n.id());
        } else if n.tag_name().name() == "style" && !within(target, n) {
            wanted.push(n);
        }
    }
    let mut refs = vec![];
    for a in root.attributes() {
        refer(root, a, &mut refs)?;
    }
    // Elements whose whole subtree is read, those queued to be, the elements kept whole and the
    // elements holding them.
    let mut read = HashSet::new();
    let mut queued: HashSet<_> = wanted.iter().map(|n| n.id()).collect();
    queued.insert(target.id());
    let mut holders = HashSet::new();
    let mut kept = wanted.clone();
    let mut queue = wanted;
    queue.push(target);
    while let Some(n) = queue.pop() {
        // The elements holding a kept one go in around it: what they refer to goes in too. Above
        // one already held, all are.
        if n != target {
            for a in n.ancestors().skip(1).take_while(|a| *a != root) {
                if !holders.insert(a.id()) {
                    break;
                }
                for at in a.attributes() {
                    refer(a, at, &mut refs)?;
                }
            }
        }
        let mut stack = vec![n];
        while let Some(m) = stack.pop() {
            if !m.is_element() || !read.insert(m.id()) {
                continue;
            }
            for a in m.attributes() {
                refer(m, a, &mut refs)?;
            }
            if m.tag_name().name() == "style" {
                let sheet: String = m.children().filter_map(|c| c.text()).collect();
                if !plain_sheet(&sheet) || m.children().filter_map(|c| c.text()).try_for_each(|t| urls(t, &mut refs)).is_none() {
                    return None;
                }
            }
            stack.extend(m.children());
        }
        for id in refs.drain(..) {
            let r = *ids.get(id)?;
            if within(target, r) {
                continue;
            }
            if within(r, target) || left_out.contains(&r.id()) {
                return None;
            }
            if !read.contains(&r.id()) && queued.insert(r.id()) {
                kept.push(r);
                queue.push(r);
            }
        }
    }
    kept.sort_by_key(|n| n.range().start);
    let mut tags = HashSet::new();
    let mut pieces = vec![];
    // The end of the last element kept whole: one starting before it is inside it, in its range.
    let mut end = 0;
    for n in kept {
        if n.range().start < end {
            continue;
        }
        end = n.range().end;
        pieces.push(n.range());
        for a in n.ancestors().skip(1).take_while(|a| *a != root) {
            if !tags.insert(a.id()) {
                break;
            }
            let (first, last) = (a.first_child()?.range(), a.last_child()?.range());
            pieces.push(a.range().start..first.start);
            pieces.push(last.end..a.range().end);
        }
    }
    pieces.sort_by_key(|r| r.start);
    Some(pieces)
}

/// Adds to `refs` the ids attribute `a` of element `n` refers to: an `href`'s `#id`, a value's
/// `url(#id)`s ([`urls`]). Of the links, only the one the importer follows: a plain `href`, else an
/// `xlink:href`. `None` when it refers to anything else, other than an image's or a link's file or
/// a data URL.
fn refer<'a>(n: roxmltree::Node, a: roxmltree::Attribute<'a, '_>, refs: &mut Vec<&'a str>) -> Option<()> {
    if a.name() != "href" {
        return urls(a.value(), refs);
    }
    if link(n).map(|l| l.range()) != Some(a.range()) {
        return Some(());
    }
    let v = a.value().trim_matches(|c: char| c.is_ascii_whitespace());
    match v.strip_prefix('#') {
        Some(id) => refs.push(plain_id(id)?),
        None if v.is_empty() || starts_with_ignoring_case(v, "data:") || matches!(n.tag_name().name(), "image" | "a") => {}
        None => return None,
    }
    Some(())
}

/// Element `n`'s link as the importer follows it (SVG 2): its plain `href`, else its `xlink:href`.
fn link<'a, 'i>(n: roxmltree::Node<'a, 'i>) -> Option<roxmltree::Attribute<'a, 'i>> {
    let href = |ns: Option<&str>| n.attributes().find(|a| a.name() == "href" && a.namespace() == ns);
    href(None).or_else(|| href(Some(XLINK)))
}

/// Adds to `refs` the ids of the `url(#id)`s in CSS or attribute value `v` (in any case, the id
/// quoted or not), passing data URLs by. `None` when one is of another form: another file, an
/// escape, a URL that doesn't close.
fn urls<'a>(v: &'a str, refs: &mut Vec<&'a str>) -> Option<()> {
    // An escape can spell `url(` or an id in another way.
    if v.contains('\\') {
        return None;
    }
    let space = |c: char| c.is_ascii_whitespace();
    let mut from = 0;
    while let Some(at) = v.get(from..)?.as_bytes().windows(4).position(|w| w.eq_ignore_ascii_case(b"url(")) {
        from += at + 4;
        let rest = v.get(from..)?.trim_start_matches(space);
        let (quote, rest) = match rest.chars().next() {
            Some(q @ ('"' | '\'')) => (Some(q), rest.get(1..)?),
            _ => (None, rest),
        };
        if starts_with_ignoring_case(rest, "data:") {
            continue;
        }
        let id = rest.strip_prefix('#')?;
        let end = id.find(|c: char| space(c) || matches!(c, ')' | '"' | '\'')).unwrap_or(id.len());
        let mut tail = id.get(end..)?.trim_start_matches(space);
        if let Some(q) = quote {
            tail = tail.strip_prefix(q)?.trim_start_matches(space);
        }
        if !tail.starts_with(')') {
            return None;
        }
        refs.push(plain_id(id.get(..end)?)?);
    }
    Some(())
}

/// `id` when it is matched as it is written: not empty, and without an escape (`\`), a percent
/// escape or a space.
fn plain_id(id: &str) -> Option<&str> {
    (!id.is_empty() && !id.contains(|c: char| matches!(c, '\\' | '%') || c.is_whitespace())).then_some(id)
}

fn starts_with_ignoring_case(s: &str, prefix: &str) -> bool {
    s.get(..prefix.len()).is_some_and(|p| p.eq_ignore_ascii_case(prefix))
}

/// Whether the rules of style sheet `css` are matched by each element and its ancestors alone, so
/// they match as before when other elements are left out: selectors of type, `*`, `#id` and
/// `.class`, joined by descendant and child (`>`) combinators, in lists. No pseudo-classes
/// (`:nth-child`, `:has`), sibling combinators, attribute selectors, escapes or at-rules.
fn plain_sheet(css: &str) -> bool {
    let mut text = String::new();
    let mut rest = css;
    while let Some(at) = rest.find("/*") {
        text.push_str(&rest[..at]);
        let Some(end) = rest[at + 2..].find("*/") else { return false };
        text.push(' ');
        rest = &rest[at + 2 + end + 2..];
    }
    text.push_str(rest);
    if text.contains(['@', '\\']) {
        return false;
    }
    let mut rest = text.as_str();
    while let Some(open) = rest.find('{') {
        let selectors = &rest[..open];
        if !selectors.chars().all(|c| c.is_ascii_alphanumeric() || c.is_ascii_whitespace() || matches!(c, '_' | '-' | '.' | '#' | '*' | '>' | ',')) {
            return false;
        }
        let body = &rest[open + 1..];
        let Some(close) = body.find('}') else { return false };
        if body[..close].contains('{') {
            return false;
        }
        rest = &body[close + 1..];
    }
    rest.trim().is_empty()
}

/// `svg[range]` with the `edits` (in order) that lie in it made: one around the range (a left-out
/// element holding the glyph) isn't, so it stays whole, defining only. The `edits` are sorted by
/// where they start.
fn edited(svg: &str, range: Range<usize>, edits: &[(Range<usize>, String)]) -> Option<String> {
    let mut out = String::new();
    let mut at = range.start;
    let first = edits.partition_point(|(r, _)| r.start < range.start);
    let within = edits.get(first..)?.iter().take_while(|(r, _)| r.start <= range.end);
    for (r, with) in within.filter(|(r, _)| r.end <= range.end) {
        // An element inside one already cut goes with it.
        if r.start < at {
            continue;
        }
        out.push_str(svg.get(at..r.start)?);
        out.push_str(with);
        at = r.end;
    }
    out.push_str(svg.get(at..range.end)?);
    Some(out)
}

/// `n` as art of its own, made from `doc` (the glyph's document): no ids (they aren't a document's)
/// or links, its members' neither, and its images holding the pixels `doc` keeps for them, named
/// by their content. A linked image, or one whose pixels `doc` doesn't hold, is left out (`None`
/// when `n` is one).
fn own_art(n: &Node, doc: &Document) -> Option<Node> {
    let mut n = n.clone();
    n.id = NodeId(u64::MAX);
    // A glyph is drawn, not followed: an `<a>`'s link (a `javascript:` one too) would be written
    // as a link again when its text is exported as outlines.
    if n.url().is_some() {
        n.edit_attrs(|a| a.url.clear());
    }
    if let NodeKind::Image(im) = &mut n.kind {
        // A glyph's images are embedded: a link (to no file of the font's) shows nothing.
        let blob = doc.images.get(&im.key).filter(|b| im.link.is_none() && !b.bytes.is_empty())?;
        im.key = blob.content_key();
        im.pixels = Some(Arc::new(blob.clone()));
    }
    if let Some(children) = n.children_mut() {
        *children = children.iter().filter(|c| !c.is_ruler_guide()).filter_map(|c| own_art(c, doc).map(Arc::new)).collect();
    }
    Some(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A document of `n` glyphs `glyph0`… that share one gradient, each two paths.
    fn shared(n: usize) -> String {
        let glyphs: String = (0..n)
            .map(|i| {
                format!(
                    r##"<g id="glyph{i}"><path fill="url(#shade)" d="M{i} 0c100 -200 300 -200 400 0s-100 300 -400 0z"/><path fill="#{:06x}" d="M0 0h50v-50z"/></g>"##,
                    i * 97 % 0xff_ffff
                )
            })
            .collect();
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1000 1000"><defs><linearGradient id="shade"><stop offset="0" stop-color="#f00"/><stop offset="1" stop-color="#00f"/></linearGradient></defs>{glyphs}</svg>"##
        )
    }

    fn rewrite(svg: &str, gid: u32, reduce: bool) -> String {
        glyph(svg, &format!("glyph{gid}"), "#000000", reduce).unwrap().svg
    }

    fn art(svg: &str, gid: u32, reduce: bool) -> Node {
        glyph_art_with(svg.as_bytes(), gid, 1000.0, Color::BLACK, None, reduce).unwrap().0
    }

    #[test]
    fn a_glyph_of_a_shared_document_hands_the_importer_only_what_it_refers_to() {
        let (small, large) = (shared(20), shared(2000));
        assert!(large.len() > 100 << 10 && (large.len() as u64) < MAX_SVG_GLYPH_BYTES, "{}", large.len());
        let one = rewrite(&large, 10, true);
        assert_eq!(one, rewrite(&small, 10, true), "the same, however many other glyphs the document holds");
        let middle = rewrite(&large, 1000, true);
        assert!(middle.len() < 2 << 10, "{} bytes", middle.len());
        assert!(middle.contains(r#"id="shade""#) && middle.contains(r#"id="glyph1000""#), "{middle}");
        assert!(!middle.contains(r#"id="glyph999""#) && !middle.contains(r#"id="glyph1001""#), "{middle}");
        assert_eq!(art(&large, 1000, true), art(&large, 1000, false));
    }

    /// Documents whose glyph refers to art across them, each one drawn the same from the `defs`
    /// left after reducing as from the whole document.
    #[test]
    fn a_reduced_document_draws_the_glyph_as_the_whole_one_does() {
        let other = r##"<g id="glyph8"><path fill="#123456" d="M0 0h900v-900z"/></g>"##;
        let docs = [
            // A gradient inside another glyph, inheriting from one in `defs`, under a fill.
            format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg"><g id="glyph3" fill="#0f0"><linearGradient id="g" href="#base"/><path d="M0 0h10v-10z"/></g>
                <defs><linearGradient id="base"><stop offset="0" stop-color="#f00"/><stop offset="1" stop-color="#00f"/></linearGradient></defs>
                <g id="glyph1"><path fill="url(#g)" d="M0 0h500v-500z"/></g>{other}</svg>"##
            ),
            // `<use>` by `href`, `xlink:href` and a prefix declared on the glyph's parent.
            format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink">
                <g xmlns:l="http://www.w3.org/1999/xlink"><defs><path id="a" d="M0 0h100v-100z"/><path id="b" d="M0 0h300v-300z"/><path id="c" d="M0 0h7v-7z"/></defs>
                <g id="glyph1"><use href="#a" xlink:href="#b" fill="#f00"/><use l:href="#b" x="400" fill="#0f0"/></g></g>{other}</svg>"##
            ),
            // A sheet's rules: a `url()` they give, a gradient's chain, a class on another glyph's group.
            format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg"><style><![CDATA[ .p {{ fill: url( '#g1' ) }} .warm > linearGradient stop, #x {{ stop-color: #f80 }} /* {{}}@ */ ]]></style>
                <g id="glyph2" class="warm"><linearGradient id="g1" href="#g2"/><linearGradient id="g2"><stop offset="0"/><stop offset="1" stop-color="#00f"/></linearGradient></g>
                <g id="glyph1"><path class="p" d="M0 0h500v-500z"/></g>{other}</svg>"##
            ),
            // Clip paths, masks, filters, markers and patterns, a cycle, the glyph using itself, a
            // non-ASCII id.
            format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg"><defs><clipPath id="c"><rect width="300" height="300"/></clipPath>
                <mask id="m"><rect width="500" height="500" fill="#fff"/></mask><filter id="f"><feGaussianBlur stdDeviation="2"/></filter>
                <marker id="k" markerWidth="4" markerHeight="4"><circle r="2" fill="url(#pat)"/></marker>
                <pattern id="pat" width="10" height="10" patternUnits="userSpaceOnUse"><rect width="5" height="5" fill="#0f0"/></pattern>
                <linearGradient id="loop1" href="#loop2"/><linearGradient id="loop2" href="#loop1"><stop offset="0" stop-color="#f0f"/></linearGradient>
                <linearGradient id="ü"><stop offset="0" stop-color="#ff0"/></linearGradient></defs>
                <g id="glyph1"><path style="clip-path:url(#c);mask:URL(&quot;#m&quot;)" filter="url(#f)" marker-start="url(#k)" fill="url(#pat)" stroke="url(#loop1)" d="M0 0L300 300"/>
                <rect fill="url(#ü)" width="9" height="9"/><use href="#glyph1"/></g>{other}</svg>"##
            ),
            // A rule on the glyph's own parent, which it leaves either way; a quoted, spaced URL to
            // an id with punctuation in it.
            format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg"><style>.outer .p {{ fill: #f00 }} .outer > g {{ stroke-width: 9 }}</style>
                <linearGradient id="a;b"><stop offset="0" stop-color="#0ff"/></linearGradient>
                <g class="outer"><g id="glyph1"><path class="p" stroke='url( "#a;b" )' d="M0 0h500v-500z"/></g></g>{other}</svg>"##
            ),
            // A pattern whose art takes its paint from the group holding it.
            format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg"><defs><linearGradient id="p2"><stop offset="0" stop-color="#f0f"/></linearGradient></defs>
                <g fill="url(#p2)"><pattern id="pat" width="10" height="10" patternUnits="userSpaceOnUse"><rect width="5" height="5"/></pattern></g>
                <g id="glyph1"><path fill="url(#pat)" d="M0 0h500v-500z"/></g>{other}</svg>"##
            ),
        ];
        for doc in &docs {
            let (reduced, whole) = (rewrite(doc, 1, true), rewrite(doc, 1, false));
            assert!(reduced.len() < whole.len() && !reduced.contains("#123456"), "reduced: {reduced}");
            assert_eq!(art(doc, 1, true), art(doc, 1, false), "{reduced}");
        }
    }

    /// Whatever the reduction can't follow keeps the whole document in the `defs`.
    #[test]
    fn what_cant_be_followed_keeps_the_whole_document() {
        let glyph = |defs: &str, art: &str| {
            format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink">{defs}<g id="glyph1">{art}</g><g id="glyph2"><path fill="#123456" d="M0 0h9v-9z"/></g></svg>"##
            )
        };
        let gradient = r##"<linearGradient id="g"><stop offset="0" stop-color="#f00"/></linearGradient>"##;
        let path = |fill: &str| format!(r##"<path fill="{fill}" d="M0 0h500v-500z"/>"##);
        let docs = [
            glyph("<style>path:nth-child(1) { fill: #f00 }</style>", &path("#000")),
            glyph("<style>g + g path { fill: #f00 }</style>", &path("#000")),
            glyph("<style>[fill] { stroke: #f00 }</style>", &path("#000")),
            glyph("<style>@import url(x.css);</style>", &path("#000")),
            glyph(r"<style>.a { fill: url(#g\31) }</style>", &path("#000")),
            glyph("<style>path { fill: #f00 </style>", &path("#000")),
            glyph(gradient, &path(r"url(#\67)")),
            glyph(gradient, r#"<path style="fill:u\72l(#g)" d="M0 0h5v-5z"/>"#),
            glyph(gradient, &path("url(#%67)")),
            glyph(gradient, &path("url('#g)")),
            glyph(gradient, &path("url(#g) url(#g x)")),
            glyph(gradient, &path("url(#nothing)")),
            glyph(gradient, &path("url(other.svg#g)")),
            glyph(gradient, r##"<use href="other.svg#g"/>"##),
            glyph(&format!("{gradient}{gradient}"), &path("url(#g)")),
            glyph(&format!(r#"<g xml:base="x/">{gradient}</g>"#), &path("url(#g)")),
            glyph(&format!("<text>{gradient}</text>"), &path("url(#g)")),
            r##"<svg xmlns="http://www.w3.org/2000/svg"><g id="outer"><g id="glyph1"><use href="#outer"/></g></g><g id="glyph2"><path d="M0 0h9v-9z"/></g></svg>"##
                .to_string(),
            r##"<?xml version="1.0"?><!DOCTYPE svg [<!ENTITY e "#f00">]><svg xmlns="http://www.w3.org/2000/svg"><g id="glyph1"><path fill="&e;" d="M0 0h5v-5z"/></g><g id="glyph2"/></svg>"##
                .to_string(),
        ];
        for doc in &docs {
            assert_eq!(rewrite(doc, 1, true), rewrite(doc, 1, false), "{doc}");
        }
    }

    #[test]
    fn edits_apply_inside_the_range_asked_for() {
        let edits = vec![(0..1, "A".to_string()), (2..2, "B".into()), (3..5, String::new()), (4..6, "C".into())];
        // An edit overlapping one already made is passed by; one not inside the range is not made.
        assert_eq!(edited("0123456", 0..7, &edits).as_deref(), Some("A1B256"));
        assert_eq!(edited("0123456", 2..5, &edits).as_deref(), Some("B2"));
        assert_eq!(edited("0123456", 4..7, &edits).as_deref(), Some("C6"));
        assert_eq!(edited("0123456", 1..2, &edits).as_deref(), Some("1B"));
    }

    /// Of an element's `href` and `xlink:href`, only the plain one is followed, as the importer
    /// follows it: the other's element stays out, and one that isn't there doesn't keep the whole
    /// document.
    #[test]
    fn only_the_link_the_importer_follows_is_kept() {
        let doc = |xlink: &str| {
            format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><defs><path id="a" d="M0 0h100v-100z"/><path id="b" d="M0 0h300v-300z"/></defs>
                <g id="glyph1"><use xlink:href="{xlink}" href="#a" fill="#f00"/><use xlink:href="#b" fill="#0f0"/></g><g id="glyph2"><path fill="#123456" d="M0 0h9v-9z"/></g></svg>"##
            )
        };
        for xlink in ["#nothing", "other.svg#b"] {
            let doc = doc(xlink);
            let reduced = rewrite(&doc, 1, true);
            assert!(reduced.len() < rewrite(&doc, 1, false).len() && !reduced.contains("#123456"), "{reduced}");
            assert_eq!(art(&doc, 1, true), art(&doc, 1, false));
        }
        let doc = doc("#b").replace(r##"<use xlink:href="#b" fill="#0f0"/>"##, "");
        let reduced = rewrite(&doc, 1, true);
        assert!(reduced.contains(r#"id="a""#) && !reduced.contains(r#"id="b""#), "{reduced}");
        assert_eq!(art(&doc, 1, true), art(&doc, 1, false));
    }

    /// A glyph inside an element that gives a prefix (or the default namespace) another namespace
    /// than the root does keeps its own after it leaves that element.
    #[test]
    fn a_glyph_keeps_its_namespaces_when_the_root_gives_them_others() {
        let flat = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:v="http://www.w3.org/1999/xlink"><defs><path id="a" d="M0 0h100v-100z"/></defs>
            <g id="glyph1"><use v:href="#a" fill="#f00"/></g><g id="glyph2"/></svg>"##;
        let nested = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:v="urn:other"><defs><path id="a" d="M0 0h100v-100z"/></defs>
            <g xmlns:v="http://www.w3.org/1999/xlink"><g id="glyph1"><use v:href="#a" fill="#f00"/></g></g><g id="glyph2"/></svg>"##;
        // The glyph declaring the prefix itself keeps it as it is.
        let declared = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:v="urn:other"><defs><path id="a" d="M0 0h100v-100z"/></defs>
            <g xmlns:v="http://www.w3.org/1999/xlink"><g xmlns:v='http://www.w3.org/1999/xlink' id="glyph1"><use v:href="#a" fill="#f00"/></g></g><g id="glyph2"/></svg>"##;
        for reduce in [true, false] {
            assert_eq!(art(nested, 1, reduce), art(flat, 1, reduce), "{}", rewrite(nested, 1, reduce));
            assert_eq!(art(declared, 1, reduce), art(flat, 1, reduce), "{}", rewrite(declared, 1, reduce));
        }
        assert_eq!(rewrite(declared, 1, true).matches("xmlns:v=").count(), 2);
        // A path in another default namespace is no SVG path, out of that element too.
        let other = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:s="http://www.w3.org/2000/svg">
            <s:g xmlns="urn:other"><s:g id="glyph1"><s:path fill="#f00" d="M0 0h5v-5z"/><path fill="#0f0" d="M0 0h9v-9z"/></s:g></s:g><s:g id="glyph2"/></svg>"##;
        let only = r##"<svg xmlns="http://www.w3.org/2000/svg"><g id="glyph1"><path fill="#f00" d="M0 0h5v-5z"/></g><g id="glyph2"/></svg>"##;
        for reduce in [true, false] {
            assert_eq!(art(other, 1, reduce), art(only, 1, reduce), "{}", rewrite(other, 1, reduce));
        }
    }

    /// Many references to the same elements keep each of them once.
    #[test]
    fn repeated_references_keep_each_element_once() {
        let uses: String = (0..2000).map(|i| format!(r##"<use href="#a" x="{i}"/><path fill="url(#g)" d="M{i} 0h1v-1z"/>"##)).collect();
        let doc = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg"><defs><linearGradient id="g"><stop offset="0" stop-color="#f00"/></linearGradient>
            <g id="a"><path fill="url(#g)" d="M0 0h5v-5z"/><use href="#b"/><use href="#b"/></g><path id="b" fill="url(#g)" d="M0 0h2v-2z"/></defs>
            <g id="glyph1">{uses}</g><g id="glyph2"><path fill="#123456" d="M0 0h9v-9z"/></g></svg>"##
        );
        let reduced = rewrite(&doc, 1, true);
        for id in ["g", "a", "b"] {
            assert_eq!(reduced.matches(&format!(r#"id="{id}""#)).count(), 1, "{id}");
        }
        assert!(!reduced.contains("#123456"));
    }

    /// An empty document, or one without the glyph, has no art; one of exactly the cap reads, one a
    /// byte larger doesn't.
    #[test]
    fn empty_and_capped_documents() {
        assert!(glyph_art(b"", 1, 1000.0, Color::BLACK, None).is_none());
        assert!(glyph_art(br#"<svg xmlns="http://www.w3.org/2000/svg"/>"#, 1, 1000.0, Color::BLACK, None).is_none());
        let doc = shared(20);
        let at = doc.len() - "</svg>".len();
        let padded = |len: u64| {
            let pad = "x".repeat(len as usize - doc.len() - "<!---->".len());
            format!("{}<!--{pad}-->{}", &doc[..at], &doc[at..])
        };
        let (full, over) = (padded(MAX_SVG_GLYPH_BYTES), padded(MAX_SVG_GLYPH_BYTES + 1));
        assert_eq!(full.len() as u64, MAX_SVG_GLYPH_BYTES);
        assert_eq!(art(&full, 3, true), art(&doc, 3, true));
        assert!(glyph_art(over.as_bytes(), 3, 1000.0, Color::BLACK, None).is_none());
    }

    /// This thread's CPU time in seconds where the system reports it, else the time since a first
    /// call.
    fn cpu_seconds() -> f64 {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        let schedstat = std::fs::read_to_string("/proc/thread-self/schedstat").ok();
        match schedstat.as_deref().and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok()) {
            Some(ns) => ns / 1e9,
            None => START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64(),
        }
    }

    /// The importer's time for 50 distinct glyphs of shared documents, read whole and reduced, the
    /// two alternating in one process: `cargo test -p vectorcraft-svg --lib glyph::tests::timing --
    /// --ignored --nocapture`.
    #[test]
    #[ignore = "timing, run by hand"]
    fn timing() {
        for n in [100, 500, 2000] {
            let doc = shared(n);
            let gids: Vec<u32> = (0..50).map(|i| (i * n / 50) as u32).collect();
            let run = |reduce: bool| {
                let t = cpu_seconds();
                for g in &gids {
                    assert!(glyph_art_with(doc.as_bytes(), *g, 1000.0, Color::BLACK, None, reduce).is_some());
                }
                cpu_seconds() - t
            };
            run(false);
            run(true);
            let (mut whole, mut reduced) = (vec![], vec![]);
            for round in 0..9 {
                if round % 2 == 0 {
                    whole.push(run(false));
                    reduced.push(run(true));
                } else {
                    reduced.push(run(true));
                    whole.push(run(false));
                }
            }
            let median = |v: &mut Vec<f64>| {
                v.sort_by(f64::total_cmp);
                v[v.len() / 2]
            };
            let ms = |v: &[f64]| v.iter().map(|s| format!("{:.1}", s * 1e3)).collect::<Vec<_>>().join(" ");
            println!("{n} glyphs, {} bytes: whole {} ms, reduced {} ms", doc.len(), ms(&whole), ms(&reduced));
            println!("{n} glyphs: median whole {:.1} ms, reduced {:.1} ms", median(&mut whole) * 1e3, median(&mut reduced) * 1e3);
        }
    }
}
