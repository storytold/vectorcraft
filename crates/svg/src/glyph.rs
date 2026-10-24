//! OpenType SVG glyphs: the art of a font's glyph from the SVG document its `SVG ` table holds for
//! it, made as the importer makes art of any SVG file. The text crate asks for it through the
//! reader the app registers ([`vectorcraft_text::set_svg_glyph_reader`]).

use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use usvg::roxmltree;
use vectorcraft_doc::color::Color;
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_geom::Affine;

use crate::ImportOptions;

/// The most bytes a glyph's document may have, unpacked.
const MAX_SVG_GLYPH_BYTES: u64 = 1 << 20;

/// Elements left out of a glyph, as the `SVG ` table rules them out: text (type inside type),
/// embedded content and scripts (and images of SVG documents, in [`glyph`]).
const LEFT_OUT: [&str; 3] = ["text", "foreignObject", "script"];

/// Glyph `gid`'s art from its SVG `document` (possibly gzipped), in a font of `upem` units per em:
/// its element `glyph<gid>` (or the whole document when that is its root), in font units (y down),
/// the root's `viewBox` (or else its size) fitted into the em square by its larger side, centred.
/// The text's colour is `fill`: `currentColor`, and the legacy `context-fill` and `context-stroke`
/// (none without a `stroke`). Its nodes have no ids and its images their own pixels
/// ([`own_art`]). With the art, whether it uses the text's colours. `None` when the document doesn't read, is too large or has
/// no such element.
pub fn glyph_art(document: &[u8], gid: u32, upem: f64, fill: Color, stroke: Option<Color>) -> Option<(Node, bool)> {
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
    let Glyph { svg, whole, frame } = glyph(&text, &id, &hex(fill))?;
    let coloured = ["currentColor", "currentcolor", "context-fill", "context-stroke"].iter().any(|k| svg.contains(k));
    // usvg resolves `context-fill` and `context-stroke` only for markers.
    let svg = svg.replace("context-fill", &hex(fill)).replace("context-stroke", &stroke.map_or_else(|| "none".into(), hex));
    let (doc, _) = crate::import::import(&svg, &ImportOptions::default()).ok()?;
    let mut art = if whole {
        Node::group(NodeId(0), doc.layers.iter().map(|l| Arc::new(Node::group(NodeId(0), l.children().cloned().unwrap_or_default()))).collect())
    } else {
        // The glyph, top-level, is a layer when a `<g>`, else an object named after its id.
        let named = |n: &Node| n.name.as_deref() == Some(id.as_str());
        match doc.layers.iter().find(|l| named(l)) {
            Some(layer) => Node::group(NodeId(0), layer.children().cloned().unwrap_or_default()),
            None => {
                let mut found = None;
                for layer in &doc.layers {
                    layer.walk(&mut |n| {
                        if found.is_none() && named(n) {
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
/// size (the em square is its frame) and in the text's colour `color`. `None` when it doesn't read
/// or has no element `id`.
fn glyph(svg: &str, id: &str, color: &str) -> Option<Glyph> {
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
    // documents inside it). Their link is `href`, else `xlink:href`, as the importer reads it.
    let svg_image = |n: &roxmltree::Node| {
        let href = n.attribute("href").or_else(|| n.attribute(("http://www.w3.org/1999/xlink", "href")));
        n.tag_name().name() == "image" && href.and_then(|h| h.trim_start().get(..14)).is_some_and(|v| v.eq_ignore_ascii_case("data:image/svg"))
    };
    for n in root.descendants().filter(|n| n.is_element() && (LEFT_OUT.contains(&n.tag_name().name()) || svg_image(n))) {
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
    // The namespaces the glyph's parents declare, which it leaves: the root declares them too.
    for ns in target.namespaces() {
        if let Some(prefix) = ns.name().filter(|p| root.lookup_namespace_uri(Some(p)).is_none()) {
            edits.push((name..name, format!(" xmlns:{prefix}=\"{}\"", crate::xml_escape(ns.uri()))));
        }
    }
    edits.sort_by_key(|(r, _)| (r.start, r.end));
    let out = if whole {
        edited(svg, 0..svg.len(), &edits)?
    } else {
        // The root's content goes in `defs`, and the glyph after it, on its own.
        let content = root.first_child()?.range().start;
        let close = svg.get(..root.range().end)?.rfind("</")?;
        [
            edited(svg, 0..content, &edits)?,
            "<defs>".into(),
            edited(svg, content..at.start, &edits)?,
            edited(svg, at.end..close, &edits)?,
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

/// `svg[range]` with the `edits` (in order) that lie in it made: one around the range (a left-out
/// element holding the glyph) isn't, so it stays whole, defining only.
fn edited(svg: &str, range: Range<usize>, edits: &[(Range<usize>, String)]) -> Option<String> {
    let mut out = String::new();
    let mut at = range.start;
    for (r, with) in edits.iter().filter(|(r, _)| range.start <= r.start && r.end <= range.end) {
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

/// `n` as art of its own, made from `doc` (the glyph's document): no ids (they aren't a document's),
/// its members' neither, and its images holding the pixels `doc` keeps for them, named by their
/// content. A linked image, or one whose pixels `doc` doesn't hold, is left out (`None` when `n`
/// is one).
fn own_art(n: &Node, doc: &Document) -> Option<Node> {
    let mut n = n.clone();
    n.id = NodeId(u64::MAX);
    if let NodeKind::Image(im) = &mut n.kind {
        // A glyph's images are embedded: a link (to no file of the font's) shows nothing.
        let blob = doc.images.get(&im.key).filter(|b| im.link.is_none() && !b.bytes.is_empty())?;
        im.key = blob.content_key();
        im.pixels = Some(Arc::new(blob.clone()));
    }
    if let Some(children) = n.children_mut() {
        *children = children.iter().filter_map(|c| own_art(c, doc).map(Arc::new)).collect();
    }
    Some(n)
}
