//! The SVG writer.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::Arc;

use vectorcraft_color::{BlendMode, Color, GradientKind, GradientPaint, Paint};
use vectorcraft_doc::{
    AppearanceItem, Document, FillLayer, LineCap, LineJoin, Node, NodeId, NodeKind, SVG_EVENTS, StrokeAlign, StrokeLayer, TextKind, TextObject,
};
use vectorcraft_doc::{CharStyle, ImageBlob, Justify};
use vectorcraft_effects::stroke::{self, Written, WrittenShape};
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, Point, Rect};

use vectorcraft_text::{FontDb, FontFace};

use crate::{
    EDITING_NS, Encoding, ExportOptions, ImageMode, LinkedImage, ObjectIds, Output, Profile, Styling, base64_encode, body_hash, fmt_num, fnv1a,
    xml_escape,
};

use crate::css::{self, Props, blend_css, css_string, font_descriptor};

mod nodes;
mod output;
mod paint;
mod text;

use nodes::SymbolDef;
pub(crate) use nodes::{MASK_FLAGS, Reuse};
pub(crate) use output::sanitize_id;
use output::{entity_value, latin1_refs, valid_xml_value, xml_attr_escape, xml_text};
use text::FontUse;

pub(crate) fn export(doc: &Document, opts: &ExportOptions, native: Option<&[u8]>) -> Output {
    // Live geometry effects (Roughen, Warp, Offset Path, Effect → Pathfinder…) export as their result.
    let baked = vectorcraft_effects::bake_document(doc);
    let doc = baked.as_ref().unwrap_or(doc);
    if opts.object_ids != ObjectIds::Unique {
        return write(doc, opts, native, "");
    }
    // Unique ids: every id and class name gets a prefix hashed from the output written without
    // one, so the same document always gets the same ids and different ones never share any.
    let plain = write(doc, opts, native, "");
    let prefix = format!("u{:010x}-", fnv1a(plain.svg.as_bytes()) & 0xff_ffff_ffff);
    write(doc, opts, native, &prefix)
}

/// `doc` as SVG with `opts`, the native file `native` embedded for Preserve Editing, its generated
/// ids starting with `id_prefix`.
fn write(doc: &Document, opts: &ExportOptions, native: Option<&[u8]>, id_prefix: &str) -> Output {
    let rect = opts
        .artboard
        .and_then(|i| doc.artboards.get(i))
        .map(|a| a.rect)
        .or_else(|| doc.art_bounds())
        .or_else(|| doc.artboards.first().map(|a| a.rect))
        .unwrap_or(Rect::new(0.0, 0.0, 1.0, 1.0));
    let mut w = Writer {
        doc,
        opts,
        xf: Affine::translate((-rect.x0, -rect.y0)),
        body: String::new(),
        defs: String::new(),
        classes: Vec::new(),
        class_ids: HashMap::new(),
        used_ids: HashSet::new(),
        next_suffix: HashMap::new(),
        next_fresh: HashMap::new(),
        linked_names: HashSet::new(),
        names: HashMap::new(),
        depth: 1,
        patterns: HashMap::new(),
        gradients: HashMap::new(),
        pattern_nest: 0,
        brushes: None,
        knockout: doc.page_knockout,
        anonymous: false,
        knockout_filter: None,
        warnings: vec![],
        id_prefix,
        linked: Vec::new(),
        instance_xf: Affine::IDENTITY,
        symbols: HashMap::new(),
        symbol_nest: 0,
        fonts: Vec::new(),
        shared_images: HashMap::new(),
        layer_nest: 0,
        non_latin1_names: false,
        foreign_ns: BTreeMap::new(),
    };
    w.assign_name_ids();
    // Page Isolated Blending / Page Knockout Group: the page content is one isolated group.
    let page_group = doc.page_isolate || doc.page_knockout;
    if page_group {
        let a = w.attrs(&vec![("isolation", "isolate".into())]);
        w.line(&format!("<g{a}>"));
        w.depth += 1;
    }
    w.children(&doc.layers);
    if page_group {
        w.depth -= 1;
        w.line("</g>");
    }

    // XML names cannot use character references. Preserve names that Latin-1 cannot encode.
    let encoding = if opts.encoding == Encoding::Latin1 && w.non_latin1_names {
        w.warn("Unicode data attribute names require UTF-8; SVG was written in UTF-8");
        Encoding::Utf8
    } else {
        opts.encoding
    };
    let nl = if opts.minify { "" } else { "\n" };
    let mut out = String::new();
    // A file in neither UTF-8 nor UTF-16 must name its encoding.
    if !opts.minify || encoding == Encoding::Latin1 {
        out.push_str(&format!("<?xml version=\"1.0\" encoding=\"{}\"?>{nl}", encoding.xml_name()));
    }
    if opts.styling == Styling::StyleEntities && !w.classes.is_empty() {
        out.push_str("<!DOCTYPE svg [");
        for (i, c) in w.classes.iter().enumerate() {
            out.push_str(&format!("{nl}{}<!ENTITY st{} \"{}\">", w.indent(1), i + 1, entity_value(c)));
        }
        out.push_str(&format!("{nl}]>{nl}"));
    }
    let (ww, hh) = (w.num(rect.width()), w.num(rect.height()));
    out.push_str("<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"");
    for (ns, prefix) in &w.foreign_ns {
        out.push_str(&format!(" xmlns:{prefix}=\"{}\"", xml_attr_escape(ns)));
    }
    if w.tiny() {
        out.push_str(" version=\"1.2\" baseProfile=\"tiny\"");
    }
    if !opts.responsive {
        out.push_str(&format!(" width=\"{ww}\" height=\"{hh}\""));
    }
    out.push_str(&format!(" viewBox=\"0 0 {ww} {hh}\">{nl}"));
    let css = opts.styling == Styling::InternalCss && !w.classes.is_empty();
    let fonts = w.font_faces();
    if !w.defs.is_empty() || css || !fonts.is_empty() {
        out.push_str(&w.indent(1));
        out.push_str(&format!("<defs>{nl}"));
        if css || !fonts.is_empty() {
            out.push_str(&w.indent(2));
            out.push_str("<style>");
            let classes = w.classes.iter().enumerate().filter(|_| css).map(|(i, c)| format!(".{id_prefix}cls-{}{{{c}}}", i + 1));
            for rule in classes.chain(fonts) {
                out.push_str(&format!("{nl}{}{}", w.indent(3), xml_escape(&rule)));
            }
            out.push_str(&format!("{nl}{}</style>{nl}", w.indent(2)));
        }
        out.push_str(&w.defs);
        out.push_str(&w.indent(1));
        out.push_str(&format!("</defs>{nl}"));
    }
    // An artboard exported on its own is named after it (#3544), else after the document.
    // A name that is blank once the characters XML can't hold are dropped falls back too.
    let board = opts.artboard.filter(|_| opts.artboard_title).and_then(|i| doc.artboards.get(i)).map(|a| xml_text(&a.name));
    let title = board.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| xml_text(&doc.title));
    if !title.is_empty() {
        out.push_str(&w.indent(1));
        out.push_str(&format!("<title>{title}</title>{nl}"));
    }
    // File Info's description.
    if !doc.metadata.description.trim().is_empty() {
        out.push_str(&w.indent(1));
        out.push_str(&format!("<desc>{}</desc>{nl}", xml_text(&doc.metadata.description)));
    }
    // Window › SVG Interactivity › Link JavaScript Files.
    for file in &doc.svg_scripts {
        if file.is_empty() || !valid_xml_value(file) {
            w.warn("JavaScript files with empty names or invalid XML characters were left out of SVG");
            continue;
        }
        out.push_str(&w.indent(1));
        out.push_str(&format!("<script xlink:href=\"{}\"/>{nl}", xml_attr_escape(file)));
    }
    let latin1 = encoding == Encoding::Latin1;
    if latin1 {
        latin1_refs(&mut out);
        latin1_refs(&mut w.body);
    }
    // The editing data carries a hash of the markup around its `<metadata>`.
    let native = native.filter(|_| opts.preserve_editing).map(|bytes| (bytes, body_hash(&[&out, &w.body, "</svg>", nl])));
    let mut meta = String::new();
    w.metadata(&mut meta, native);
    if latin1 {
        latin1_refs(&mut meta);
    }
    out.push_str(&meta);
    out.push_str(&w.body);
    out.push_str("</svg>");
    out.push_str(nl);
    Output { svg: out, linked: w.linked, warnings: w.warnings, encoding }
}

struct Writer<'a> {
    doc: &'a Document,
    opts: &'a ExportOptions,
    /// Document → SVG user space (artboard origin translation, symbol instance transforms).
    xf: Affine,
    body: String,
    defs: String,
    /// Unique CSS declaration blocks, in first-use order (used for deterministic output).
    classes: Vec<String>,
    /// 1-based class/entity number by declaration; avoids a linear scan for each style.
    class_ids: HashMap<String, usize>,
    used_ids: HashSet<String>,
    /// The next suffix to try for a base name, so a repeated name doesn't probe the ones taken again.
    next_suffix: HashMap<String, usize>,
    /// The next number [`Writer::fresh_id`] tries for a prefix (every lower one is taken).
    next_fresh: HashMap<String, usize>,
    /// The names in `linked`.
    linked_names: HashSet<String>,
    names: HashMap<NodeId, String>,
    depth: usize,
    /// `<pattern>` def ids by pattern name + placement.
    patterns: HashMap<String, String>,
    /// Gradient def ids by content (everything but the id).
    gradients: HashMap<String, String>,
    pattern_nest: u32,
    /// Whether the group being written is a knockout group (what its neutral children inherit).
    knockout: bool,
    /// Writing a copy of an object (a knockout mask): no object ids, so they stay unique.
    anonymous: bool,
    /// The filter that paints art black keeping its alpha (knockout masks), once defined.
    knockout_filter: Option<String>,
    /// The brush library, parsed when the first brushed stroke is written.
    brushes: Option<Vec<vectorcraft_brush::Brush>>,
    /// Features written approximately (once each).
    warnings: Vec<String>,
    /// Prepended to every id and class name ([`ObjectIds::Unique`]).
    id_prefix: &'a str,
    /// Image files the SVG links to ([`ImageMode::Link`]).
    linked: Vec<LinkedImage>,
    /// The transform of the symbol instances written out as their own art around the object
    /// being written (document space; identity outside them and in symbol defs).
    instance_xf: Affine,
    /// What each symbol's instances can share, by symbol name (see [`Writer::symbol_use`]).
    symbols: HashMap<String, SymbolDef>,
    /// Symbols being written inside one another (instances nested deeper are left out: a symbol
    /// can't contain itself).
    symbol_nest: u32,
    /// The characters type uses from each face ([`ExportOptions::embed_fonts`]).
    fonts: Vec<FontUse>,
    /// Images written once in the defs and drawn with `<use>`, by key and size: the pieces an
    /// envelope cuts a distorted image into share its pixels.
    shared_images: HashMap<(String, u32, u32), String>,
    /// Layers being written inside one another (more than one: a sublayer).
    layer_nest: usize,
    /// An emitted attribute name requires an encoding wider than Latin-1.
    non_latin1_names: bool,
    /// The prefix written for each namespace of other applications' attributes, by its URI
    /// ([`Writer::foreign_attrs`]): declared on the root.
    foreign_ns: BTreeMap<String, String>,
}
