//! The SVG writer.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::Arc;

use vectorcraft_color::{BlendMode, GradientKind, GradientPaint, Paint};
use vectorcraft_doc::{
    AppearanceItem, Document, FillLayer, LineCap, LineJoin, Node, NodeId, NodeKind, StrokeAlign, StrokeLayer, TextKind, TextObject,
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
        used_ids: HashSet::new(),
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

    let nl = if opts.minify { "" } else { "\n" };
    let mut out = String::new();
    // A file in neither UTF-8 nor UTF-16 must name its encoding.
    if !opts.minify || opts.encoding == Encoding::Latin1 {
        out.push_str(&format!("<?xml version=\"1.0\" encoding=\"{}\"?>{nl}", opts.encoding.xml_name()));
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
    if w.tiny() {
        out.push_str(" version=\"1.2\" baseProfile=\"tiny\"");
    }
    if !opts.responsive {
        // Internal document units are points; `ww` / `hh` are points too. SVG treats unitless
        // numbers as CSS pixels, so a 210 mm artboard (595.27 pt) would render as 595.27 px and
        // come out 0.75× the size in a viewer (72/96). Append `pt` so the rendered size matches
        // the document's. (#864)
        out.push_str(&format!(" width=\"{ww}pt\" height=\"{hh}pt\""));
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
    if !doc.title.is_empty() {
        out.push_str(&w.indent(1));
        out.push_str(&format!("<title>{}</title>{nl}", xml_escape(&doc.title)));
    }
    // File Info's description.
    if !doc.metadata.description.trim().is_empty() {
        out.push_str(&w.indent(1));
        out.push_str(&format!("<desc>{}</desc>{nl}", xml_escape(&doc.metadata.description)));
    }
    let latin1 = opts.encoding == Encoding::Latin1;
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
    Output { svg: out, linked: w.linked, warnings: w.warnings, encoding: opts.encoding }
}

/// Write every character of `s` beyond ISO 8859-1 as a character reference (all of them sit in
/// attribute values, text or style sheets, where references read as the character).
fn latin1_refs(s: &mut String) {
    if s.chars().all(|c| u32::from(c) < 0x100) {
        return;
    }
    let mut o = String::with_capacity(s.len() + 64);
    for c in s.chars() {
        if u32::from(c) < 0x100 {
            o.push(c);
        } else {
            let _ = write!(o, "&#x{:X};", u32::from(c));
        }
    }
    *s = o;
}

/// `chars` (sorted) as a CSS `unicode-range` (`U+41-5A,U+61`).
fn unicode_range(chars: &BTreeSet<char>) -> String {
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for c in chars.iter().map(|c| u32::from(*c)) {
        match runs.last_mut() {
            Some((_, end)) if c == *end + 1 => *end = c,
            _ => runs.push((c, c)),
        }
    }
    let one = |(a, b): (u32, u32)| if a == b { format!("U+{a:X}") } else { format!("U+{a:X}-{b:X}") };
    runs.into_iter().map(one).collect::<Vec<_>>().join(",")
}

/// A CSS declaration block as the literal value of an `<!ENTITY>`: quotes, `%` and markup
/// characters become references that still read as themselves inside the `style` attribute.
fn entity_value(decl: &str) -> String {
    let mut o = String::with_capacity(decl.len());
    for c in decl.chars() {
        match c {
            '"' => o.push_str("&#34;"),
            '%' => o.push_str("&#37;"),
            // Doubly escaped: the entity's replacement text is parsed again where it is used.
            '&' => o.push_str("&#38;#38;"),
            '<' => o.push_str("&#38;#60;"),
            _ => o.push(c),
        }
    }
    o
}

/// Image `b` as SVG viewers show it: PNG, JPEG, GIF, WebP and SVG as they are, other formats (a
/// CMYK TIFF) as PNG.
fn web_image(b: &ImageBlob) -> std::borrow::Cow<'_, ImageBlob> {
    use std::borrow::Cow;
    if matches!(b.mime.as_str(), "image/png" | "image/jpeg" | "image/jpg" | "image/gif" | "image/webp" | "image/svg+xml") {
        return Cow::Borrowed(b);
    }
    let png = image::load_from_memory(&b.bytes).ok().and_then(|img| {
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).ok()?;
        Some(out)
    });
    png.map_or(Cow::Borrowed(b), |png| Cow::Owned(ImageBlob::png(png)))
}

/// The usual file extension of an image MIME type.
fn image_ext(mime: &str) -> &str {
    match mime {
        "image/jpeg" => "jpg",
        "image/svg+xml" => "svg",
        m => m.strip_prefix("image/").filter(|e| !e.is_empty() && e.chars().all(|c| c.is_ascii_alphanumeric())).unwrap_or("bin"),
    }
}

struct Writer<'a> {
    doc: &'a Document,
    opts: &'a ExportOptions,
    /// Document → SVG user space (artboard origin translation, symbol instance transforms).
    xf: Affine,
    body: String,
    defs: String,
    /// Unique CSS declaration blocks; class name is `cls-{index + 1}`.
    classes: Vec<String>,
    used_ids: HashSet<String>,
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
}

/// The characters type uses from one face, under one `@font-face` description: the family the
/// type names and the weight and italic its style asks for.
struct FontUse {
    family: String,
    weight: u16,
    italic: bool,
    face: Arc<FontFace>,
    chars: BTreeSet<char>,
}

/// A symbol's `<symbol>` def and the instance transforms that can `<use>` it.
struct SymbolDef {
    /// The def's id, once written.
    id: Option<String>,
    reuse: Reuse,
}

/// Which instance transforms paint a symbol's art exactly as the def under that transform (the
/// canvas transforms the art's geometry alone: stroke weights, effects, patterns and unlinked
/// masks stay put).
#[derive(Clone, Copy)]
pub(crate) struct Reuse {
    /// Translations (no pattern paints or unlinked masks).
    moves: bool,
    /// Rotations and reflections (nor effects or brushes).
    turns: bool,
    /// Any other transform (nor strokes or live objects).
    scales: bool,
}

/// How one stroke is written.
enum StrokePlan {
    /// Its brush art (document space).
    Brush(Vec<Node>),
    Written(Written),
}

/// How deep symbols may sit in one another's art (deeper instances are left out).
const MAX_SYMBOL_NEST: u32 = 8;

impl Reuse {
    const NONE: Self = Self { moves: false, turns: false, scales: false };

    /// What a def of symbol art `art` stands for.
    pub(crate) fn of(doc: &Document, art: &Node) -> Self {
        let mut r = Self { moves: true, turns: true, scales: true };
        r.scan(doc, art, 0);
        r
    }

    /// Narrow down to what `n` (inside symbols nested `depth` deep) allows.
    fn scan(&mut self, doc: &Document, n: &Node, depth: u32) {
        let pattern = |p: &Paint| matches!(p, Paint::Pattern { .. });
        let text_patterns = matches!(&n.kind, NodeKind::Text(t) if t.runs.iter().any(|r| pattern(&r.style.fill) || pattern(&r.style.stroke)));
        let unlinked_mask = n.mask.as_ref().is_some_and(|m| !m.linked);
        if depth > MAX_SYMBOL_NEST || text_patterns || unlinked_mask || vectorcraft_doc::pattern::uses_pattern(n, None) {
            *self = Self::NONE;
            return;
        }
        let effects = |fx: &[vectorcraft_doc::Effect]| fx.iter().any(|e| e.visible);
        if effects(&n.appearance.effects) || n.appearance.items.iter().any(|i| effects(i.effects())) {
            (self.turns, self.scales) = (false, false);
        }
        for i in &n.appearance.items {
            if let AppearanceItem::Stroke(s) = i
                && s.visible
                && !s.paint.is_none()
                && s.width > 0.0
            {
                // Stroke weights don't scale; brushes can orient their art to the page.
                self.scales = false;
                self.turns &= s.brush.is_none();
            }
        }
        match &n.kind {
            NodeKind::Blend { .. } | NodeKind::Envelope { .. } | NodeKind::Mesh(_) | NodeKind::Repeat(_) | NodeKind::PlacedDocument(_) => {
                (self.turns, self.scales) = (false, false)
            }
            NodeKind::SymbolInstance { symbol, .. } => {
                if let Some(s) = doc.symbols.iter().find(|s| s.name == *symbol) {
                    self.scan(doc, &s.art, depth + 1);
                }
            }
            _ => {}
        }
        if let Some(m) = &n.mask {
            self.scan(doc, &m.art, depth);
        }
        for c in n.children().into_iter().flatten() {
            self.scan(doc, c, depth);
        }
    }

    /// Can the def stand for an instance with transform `xf`?
    pub(crate) fn allows(self, xf: Affine) -> bool {
        const EPS: f64 = 1e-9;
        let [a, b, c, d, _, _] = xf.as_coeffs();
        if (a - 1.0).abs() < EPS && b.abs() < EPS && c.abs() < EPS && (d - 1.0).abs() < EPS {
            return self.moves;
        }
        let rigid = (a * a + b * b - 1.0).abs() < EPS && (c * c + d * d - 1.0).abs() < EPS && (a * c + b * d).abs() < EPS;
        if rigid { self.turns } else { self.scales }
    }
}

/// Art about to be written moved by `m.inverse()` (a symbol instance's): map what the canvas
/// leaves on the page by `m` first, so it stays there. That is every pattern paint's placement
/// and the art of unlinked opacity masks.
fn pin_to_page(n: &mut Node, m: Affine) {
    vectorcraft_doc::pattern::transform_pattern_paints(n, m);
    if let Some(mask) = n.mask.as_deref_mut() {
        let linked = mask.linked;
        let art = std::sync::Arc::make_mut(&mut mask.art);
        if !linked {
            art.transform(m, false);
        }
        pin_to_page(art, m);
    }
    for c in n.children_mut().into_iter().flatten() {
        pin_to_page(std::sync::Arc::make_mut(c), m);
    }
}

/// The Dublin Core terms File Info writes to `<metadata>`: the format, title, author (creator),
/// description, each keyword (subject), copyright notice and URL (rights) and created date, the
/// empty ones left out.
fn dublin_core(doc: &Document) -> Vec<(&'static str, String)> {
    let info = &doc.metadata;
    let mut terms = vec![("format", "image/svg+xml".to_string())];
    let created = info.created.map(vectorcraft_doc::metadata::iso8601).unwrap_or_default();
    let fields = [("title", doc.title.as_str()), ("creator", info.author.as_str()), ("description", info.description.as_str())];
    let rights = [("rights", info.copyright_notice.as_str()), ("rights", info.copyright_url.as_str()), ("date", created.as_str())];
    let keywords = info.keywords.iter().map(|k| ("subject", k.as_str()));
    terms.extend(fields.into_iter().chain(keywords).chain(rights).filter(|(_, v)| !v.trim().is_empty()).map(|(k, v)| (k, v.to_string())));
    terms
}

/// What SVG Tiny 1.2 does without masks.
const TINY_MASKS: &str = "SVG Tiny 1.2 has no masks: opacity masks are left out and knockout groups written as plain groups";
/// A region covering any artwork (mask and filter extents).
const BIG: &str = "x=\"-100000\" y=\"-100000\" width=\"200000\" height=\"200000\"";
/// Attributes of every `<mask>`: user-space units, and luminance taken from the sRGB values (as the
/// canvas and PDF take it) rather than from linearised ones.
const MASK: &str = "maskUnits=\"userSpaceOnUse\" color-interpolation=\"sRGB\"";
/// Attribute of an exported opacity mask listing its options that differ from the defaults
/// (clipping, not inverted): `noclip`, `invert`.
pub(crate) const MASK_FLAGS: &str = "data-vectorcraft-mask";

/// Turn an object name into a valid, readable XML id.
pub(crate) fn sanitize_id(name: &str) -> String {
    let mut s: String = name.trim().chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' }).collect();
    if s.is_empty() || !s.starts_with(|c: char| c.is_alphabetic() || c == '_') {
        s.insert(0, '_');
    }
    s
}

impl Writer<'_> {
    /// Writing the SVG Tiny 1.2 profile?
    fn tiny(&self) -> bool {
        self.opts.profile == Profile::Tiny12
    }
    fn num(&self, v: f64) -> String {
        fmt_num(v, self.opts.decimals)
    }
    fn indent(&self, depth: usize) -> String {
        if self.opts.minify { String::new() } else { "  ".repeat(depth) }
    }
    fn line(&mut self, s: &str) {
        let ind = self.indent(self.depth);
        self.body.push_str(&ind);
        self.body.push_str(s);
        if !self.opts.minify {
            self.body.push('\n');
        }
    }
    fn def(&mut self, depth: usize, s: &str) {
        let ind = self.indent(depth + 1);
        self.defs.push_str(&ind);
        self.defs.push_str(s);
        if !self.opts.minify {
            self.defs.push('\n');
        }
    }
    /// The `href` of image `im`: its file with [`ImageMode::Link`] (or when its pixels are
    /// missing), else its bytes. `None` when it has neither.
    fn image_href(&mut self, im: &vectorcraft_doc::ImageObject) -> Option<String> {
        let (doc, link) = (self.doc, self.opts.images == ImageMode::Link);
        match (im.link.as_ref().filter(|_| link), doc.images.get(&im.key)) {
            (Some(l), _) => Some(l.path.clone()),
            (None, Some(b)) if !b.bytes.is_empty() => Some(self.blob_href(&web_image(b))),
            _ => im.link.as_ref().map(|l| l.path.clone()),
        }
    }

    /// Images drawn more than once in `art` (the pieces of a distorted image) go into the defs
    /// once; each piece then draws them with `<use>`.
    fn share_images(&mut self, art: &Node) {
        let mut seen: HashMap<(String, u32, u32), usize> = HashMap::new();
        art.walk(&mut |c| {
            if let NodeKind::Image(im) = &c.kind {
                *seen.entry((im.key.clone(), im.width, im.height)).or_default() += 1;
            }
        });
        let mut repeated: Vec<_> = seen.into_iter().filter(|(k, n)| *n > 1 && !self.shared_images.contains_key(k)).map(|(k, _)| k).collect();
        repeated.sort();
        for (key, width, height) in repeated {
            let link = self.linked_of(art, &key);
            let im = vectorcraft_doc::ImageObject { key: key.clone(), width, height, xf: Affine::IDENTITY, link, placement: Default::default() };
            let Some(href) = self.image_href(&im) else { continue };
            let id = self.fresh_id("image");
            self.def(
                0,
                &format!(
                    "<image id=\"{id}\" width=\"{width}\" height=\"{height}\" preserveAspectRatio=\"none\" xlink:href=\"{}\"/>",
                    xml_escape(&href)
                ),
            );
            self.shared_images.insert((key, width, height), id);
        }
    }

    /// The link of the first image with blob `key` in `art`.
    fn linked_of(&self, art: &Node, key: &str) -> Option<vectorcraft_doc::LinkInfo> {
        let mut link = None;
        art.walk(&mut |c| {
            if let NodeKind::Image(im) = &c.kind
                && im.key == key
                && link.is_none()
            {
                link = im.link.clone();
            }
        });
        link
    }

    fn fresh_id(&mut self, prefix: &str) -> String {
        let mut i = 1;
        loop {
            let id = format!("{}{prefix}-{i}", self.id_prefix);
            if self.used_ids.insert(id.clone()) {
                return id;
            }
            i += 1;
        }
    }
    /// `base` (with the unique prefix) if free, else `base-2`, `base-3`…
    fn unique_id(&mut self, base: &str) -> String {
        let base = format!("{}{base}", self.id_prefix);
        let mut cand = base.clone();
        for i in 2.. {
            if self.used_ids.insert(cand.clone()) {
                break;
            }
            cand = format!("{base}-{i}");
        }
        cand
    }

    /// Reserve ids for every named object up front so generated def ids never collide with them.
    fn assign_name_ids(&mut self) {
        if self.opts.object_ids == ObjectIds::Minimal {
            return;
        }
        let mut named: Vec<(NodeId, String)> = Vec::new();
        self.doc.walk(|n| {
            if let Some(name) = &n.name {
                named.push((n.id, sanitize_id(name)));
            }
        });
        for (id, base) in named {
            let cand = self.unique_id(&base);
            self.names.insert(id, cand);
        }
    }
    /// ` id="…"` for a named object, plus ` data-name="…"` with the name itself when the id had to
    /// differ from it (spaces, punctuation, duplicates, the unique prefix).
    /// The id of `n`'s element (with its name when the id isn't it), a sublayer's mark
    /// ([`crate::import::SUBLAYER`]), then the object's own data as `data-*` attributes.
    fn id_attr(&self, n: &Node) -> String {
        if self.anonymous {
            return String::new();
        }
        let mut out = match (self.names.get(&n.id), n.name.as_deref()) {
            (Some(id), Some(name)) if name != id => format!(" id=\"{}\" data-name=\"{}\"", xml_escape(id), xml_escape(name)),
            (Some(id), _) => format!(" id=\"{}\"", xml_escape(id)),
            (None, _) => String::new(),
        };
        // A sublayer is marked as one, so import makes it a sublayer again, not a group.
        if !out.is_empty() && n.is_layer() && self.layer_nest > 1 {
            out.push_str(&format!(" {}=\"sublayer\"", crate::import::SUBLAYER));
        }
        for (k, v) in n.attrs.as_deref().map_or(&[][..], |a| a.data.as_slice()) {
            // Names as XML takes them; ours (`name`, `vc-…`) are written by the export itself.
            let valid = k.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) && k != "name" && !k.starts_with("vc-");
            if valid && !k.is_empty() {
                out.push_str(&format!(" data-{k}=\"{}\"", xml_escape(v)));
            }
        }
        out
    }
    fn matrix(&self, m: Affine) -> String {
        let c = m.as_coeffs();
        format!("matrix({} {} {} {} {} {})", self.num(c[0]), self.num(c[1]), self.num(c[2]), self.num(c[3]), self.num(c[4]), self.num(c[5]))
    }

    /// Style properties as attributes, an inline style, or a CSS class, per the styling option.
    fn attrs(&mut self, props: &Props) -> String {
        if props.is_empty() {
            return String::new();
        }
        // SVG Tiny has presentation attributes alone.
        let styling = if self.tiny() { Styling::PresentationAttributes } else { self.opts.styling };
        match styling {
            Styling::PresentationAttributes => {
                // CSS-only properties go in a style attribute (SVG Tiny has none: left out).
                let (style, attrs): (Vec<_>, Vec<_>) =
                    props.iter().partition(|(k, _)| matches!(*k, "mix-blend-mode" | "isolation" | "font-feature-settings" | "font-kerning"));
                let mut s: String = attrs.iter().map(|(k, v)| format!(" {k}=\"{}\"", xml_escape(v))).collect();
                if !style.is_empty() {
                    if self.tiny() {
                        self.warn("SVG Tiny 1.2 has no style sheets: blend modes, isolation and font features are left out");
                    } else {
                        s.push_str(&format!(" style=\"{}\"", xml_escape(&css::declarations(style))));
                    }
                }
                s
            }
            Styling::InlineStyle => format!(" style=\"{}\"", xml_escape(&css::declarations(props))),
            Styling::StyleEntities => format!(" style=\"&st{};\"", self.class(css::declarations(props))),
            Styling::InternalCss => {
                let i = self.class(css::declarations(props));
                format!(" class=\"{}cls-{i}\"", self.id_prefix)
            }
        }
    }

    /// The 1-based number of a distinct declaration block (a CSS class or a style entity).
    fn class(&mut self, decl: String) -> usize {
        match self.classes.iter().position(|c| *c == decl) {
            Some(i) => i + 1,
            None => {
                self.classes.push(decl);
                self.classes.len()
            }
        }
    }

    /// `<metadata>`: the Dublin Core terms of File Info ([`ExportOptions::metadata`]) and the
    /// native document ([`ExportOptions::preserve_editing`]) with the [`body_hash`] of the markup
    /// around this element.
    fn metadata(&self, out: &mut String, native: Option<(&[u8], String)>) {
        if !self.opts.metadata && native.is_none() {
            return;
        }
        let nl = if self.opts.minify { "" } else { "\n" };
        let mut line = |depth: usize, s: &str| {
            out.push_str(&self.indent(depth));
            out.push_str(s);
            out.push_str(nl);
        };
        line(1, "<metadata>");
        if self.opts.metadata {
            line(2, "<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\">");
            line(3, "<rdf:Description rdf:about=\"\">");
            for (term, value) in dublin_core(self.doc) {
                line(4, &format!("<dc:{term}>{}</dc:{term}>", xml_escape(&value)));
            }
            line(3, "</rdf:Description>");
            line(2, "</rdf:RDF>");
        }
        if let Some((bytes, hash)) = native {
            line(
                2,
                &format!(
                    "<vectorcraft:document xmlns:vectorcraft=\"{EDITING_NS}\" hash=\"{hash}\"><![CDATA[{}]]></vectorcraft:document>",
                    base64_encode(bytes)
                ),
            );
        }
        line(1, "</metadata>");
    }

    fn path_d(&self, p: &PathData, m: Affine) -> String {
        let mut s = String::new();
        let pt = |p: Point| {
            let p = m * p;
            format!("{} {}", self.num(p.x), self.num(p.y))
        };
        for sp in &p.subpaths {
            let n = sp.anchors.len();
            if n == 0 {
                continue;
            }
            if !s.is_empty() {
                s.push(' ');
            }
            s.push('M');
            s.push_str(&pt(sp.anchors[0].p));
            for i in 0..sp.segment_count() {
                let (a, b) = (&sp.anchors[i], &sp.anchors[(i + 1) % n]);
                if sp.segment_is_line(i) {
                    if sp.closed && i == n - 1 {
                        break;
                    }
                    s.push_str(&format!(" L{}", pt(b.p)));
                } else {
                    s.push_str(&format!(" C{} {} {}", pt(a.h_out), pt(b.h_in), pt(b.p)));
                }
            }
            if sp.closed {
                s.push_str(" Z");
            }
        }
        s
    }

    /// A fill or stroke value. Gradients resolve unset geometry against `bounds` (the space
    /// [`Self::xf`] maps into user space, as the renderer does).
    fn paint(&mut self, p: &Paint, bounds: Option<Rect>) -> String {
        match p {
            Paint::None => "none".into(),
            Paint::Pattern { pattern, xf } => match self.pattern_def(pattern, *xf) {
                Some(id) => format!("url(#{id})"),
                None => "none".into(),
            },
            Paint::Solid { color, .. } => color.to_hex(),
            Paint::Gradient(g) => format!("url(#{})", self.gradient_def(g, bounds.unwrap_or(Rect::new(0.0, 0.0, 1.0, 1.0)))),
        }
    }

    /// A `<pattern>` def for pattern `name` placed by `xf`: one period (super-tile) of the tiling,
    /// with every instance that reaches into it.
    fn pattern_def(&mut self, name: &str, xf: Affine) -> Option<String> {
        let doc = self.doc;
        let def = doc.pattern(name)?;
        let m = self.xf * xf;
        let key = format!("{name}|{:?}", m.as_coeffs());
        if let Some(id) = self.patterns.get(&key) {
            return Some(id.clone());
        }
        if self.pattern_nest > 4 {
            return None;
        }
        let id = self.fresh_id("pattern");
        self.patterns.insert(key, id.clone());
        let (pw, ph) = def.period();
        let (saved_body, saved_xf, saved_depth) = (std::mem::take(&mut self.body), self.xf, self.depth);
        self.depth = 3;
        self.pattern_nest += 1;
        for o in def.offsets_covering(Rect::new(0.0, 0.0, pw, ph)) {
            self.xf = def.instance_xf(o);
            for a in &def.art {
                self.node(a);
            }
        }
        self.pattern_nest -= 1;
        let content = std::mem::replace(&mut self.body, saved_body);
        self.xf = saved_xf;
        self.depth = saved_depth;
        let head = format!(
            "<pattern id=\"{id}\" patternUnits=\"userSpaceOnUse\" width=\"{}\" height=\"{}\" patternTransform=\"{}\">",
            self.num(pw),
            self.num(ph),
            self.matrix(m)
        );
        self.def(1, &head);
        self.defs.push_str(&content);
        self.def(1, "</pattern>");
        Some(id)
    }

    /// The id of a `<linearGradient>`/`<radialGradient>` def for `g` resolved against `bounds`.
    /// Identical gradients share one def; a gradient from a swatch takes the swatch's name as its
    /// id. A midpoint other than halfway is written as an extra stop marked `data-vc-midpoint`
    /// (what import turns back into a midpoint).
    fn gradient_def(&mut self, g: &GradientPaint, bounds: Rect) -> String {
        if g.gradient.kind == GradientKind::Freeform {
            // Object fills and strokes are images (see `raster_paint`); characters' paints aren't.
            self.warn("freeform gradients on characters are written as linear gradients");
        }
        let mut geom = g.resolve(bounds);
        geom.transform(self.xf, g.gradient.kind);
        let (s, e) = (geom.start, geom.end);
        // An off-centre focal point: `fx`/`fy`, in the gradient's own (unsquashed) space.
        let focal = |w: &Self, f: Point| format!(" fx=\"{}\" fy=\"{}\"", w.num(f.x), w.num(f.y));
        let (tag, prefix, attrs) = match g.gradient.kind {
            GradientKind::Radial => {
                let r = self.num((e - s).hypot());
                let attrs = if (geom.aspect - 1.0).abs() < 1e-9 {
                    let f = geom.focal.map(|f| focal(self, f)).unwrap_or_default();
                    format!(" cx=\"{}\" cy=\"{}\" r=\"{r}\"{f}", self.num(s.x), self.num(s.y))
                } else {
                    let v = e - s;
                    let m = Affine::translate(s.to_vec2()) * Affine::rotate(v.y.atan2(v.x)) * Affine::scale_non_uniform(1.0, geom.aspect);
                    let f = geom.focal.map(|f| focal(self, m.inverse() * f)).unwrap_or_default();
                    format!(" cx=\"0\" cy=\"0\" r=\"{r}\"{f} gradientTransform=\"{}\"", self.matrix(m))
                };
                ("radialGradient", "radial-gradient", attrs)
            }
            GradientKind::Linear | GradientKind::Freeform => (
                "linearGradient",
                "linear-gradient",
                format!(" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"", self.num(s.x), self.num(s.y), self.num(e.x), self.num(e.y)),
            ),
        };
        let stops: Vec<String> = g
            .gradient
            .expanded()
            .map(|(offset, c, o, mid)| {
                let op = if o < 1.0 { format!(" stop-opacity=\"{}\"", fmt_num(o as f64, 3)) } else { String::new() };
                let mid = mid.map(|m| format!(" data-vc-midpoint=\"{}\"", fmt_num(m as f64, 4))).unwrap_or_default();
                format!("<stop offset=\"{}\" stop-color=\"{}\"{op}{mid}/>", fmt_num(offset as f64, 4), c.to_hex())
            })
            .collect();
        let key = format!("{tag}{attrs}{}", stops.concat());
        if let Some(id) = self.gradients.get(&key) {
            return id.clone();
        }
        let id = match &g.swatch {
            Some(name) => self.unique_id(&sanitize_id(name)),
            None => self.fresh_id(prefix),
        };
        self.gradients.insert(key, id.clone());
        self.def(1, &format!("<{tag} id=\"{id}\"{attrs} gradientUnits=\"userSpaceOnUse\">"));
        for stop in &stops {
            self.def(2, stop);
        }
        self.def(1, &format!("</{tag}>"));
        id
    }

    fn fill_props(&mut self, f: &FillLayer, rule: FillRule, bounds: Option<Rect>, p: &mut Props) {
        let paint = self.paint(&f.paint, bounds);
        p.push(("fill", paint));
        if f.opacity < 1.0 {
            p.push(("fill-opacity", fmt_num(f.opacity as f64, 3)));
        }
        if rule == FillRule::EvenOdd {
            p.push(("fill-rule", "evenodd".into()));
        }
    }

    fn stroke_props(&mut self, s: &StrokeLayer, width: f64, bounds: Option<Rect>, p: &mut Props) {
        let paint = self.paint(&s.paint, bounds);
        p.push(("stroke", paint));
        if s.opacity < 1.0 {
            p.push(("stroke-opacity", fmt_num(s.opacity as f64, 3)));
        }
        if (width - 1.0).abs() > 1e-9 {
            p.push(("stroke-width", self.num(width)));
        }
        match s.cap {
            LineCap::Butt => {}
            LineCap::Round => p.push(("stroke-linecap", "round".into())),
            LineCap::Square => p.push(("stroke-linecap", "square".into())),
        }
        match s.join {
            LineJoin::Miter => {
                if (s.miter_limit - 4.0).abs() > 1e-9 {
                    p.push(("stroke-miterlimit", self.num(s.miter_limit)));
                }
            }
            LineJoin::Round => p.push(("stroke-linejoin", "round".into())),
            LineJoin::Bevel => p.push(("stroke-linejoin", "bevel".into())),
        }
        if let Some(d) = &s.dash
            && d.is_dashed()
        {
            p.push(("stroke-dasharray", d.pattern.iter().map(|v| self.num(*v)).collect::<Vec<_>>().join(" ")));
            if d.offset != 0.0 {
                p.push(("stroke-dashoffset", self.num(d.offset)));
            }
        }
    }

    /// How stroke `st` is written (`bp`: the shape in document space, built on first use from
    /// `paths`): its brush art, or a plain stroke or filled outlines matching the canvas.
    fn stroke_plan(&mut self, st: &StrokeLayer, bp: &mut Option<BezPath>, paths: &[&PathData]) -> StrokePlan {
        if stroke::is_plain(st) {
            return StrokePlan::Written(stroke::for_writer(&BezPath::new(), st));
        }
        let bp = bp.get_or_insert_with(|| doc_path(paths));
        let doc = self.doc;
        let brushes = self.brushes.get_or_insert_with(|| vectorcraft_brush::library(doc));
        match st.brush.as_deref().and_then(|name| brushes.iter().find(|b| b.name == name)) {
            Some(b) => StrokePlan::Brush(vectorcraft_brush::stroke_pieces(b, bp, st)),
            None => StrokePlan::Written(stroke::for_writer(bp, st)),
        }
    }

    /// The `clip-path` (inside) or `mask` (outside) attribute that keeps an aligned stroke on its
    /// side of the shape `d`; `reach` (document space) is what the stroke covers.
    fn side_attr(&mut self, side: Option<StrokeAlign>, d: &str, rule: FillRule, reach: Rect) -> String {
        match side {
            None | Some(StrokeAlign::Center) => String::new(),
            Some(StrokeAlign::Inside) => {
                let cid = self.fresh_id("clip-path");
                let r = if rule == FillRule::EvenOdd { " clip-rule=\"evenodd\"" } else { "" };
                self.def(1, &format!("<clipPath id=\"{cid}\">"));
                self.def(2, &format!("<path d=\"{d}\"{r}/>"));
                self.def(1, "</clipPath>");
                format!(" clip-path=\"url(#{cid})\"")
            }
            Some(StrokeAlign::Outside) if self.tiny() => {
                // No masks: clipped by everything around the shape instead.
                let cid = self.fresh_id("clip-path");
                let b = self.xf.transform_rect_bbox(reach).inflate(1.0, 1.0);
                let (x0, y0, x1, y1) = (self.num(b.x0), self.num(b.y0), self.num(b.x1), self.num(b.y1));
                self.def(1, &format!("<clipPath id=\"{cid}\">"));
                self.def(2, &format!("<path d=\"M{x0} {y0} H{x1} V{y1} H{x0} Z {d}\" clip-rule=\"evenodd\"/>"));
                self.def(1, "</clipPath>");
                format!(" clip-path=\"url(#{cid})\"")
            }
            Some(StrokeAlign::Outside) => {
                let mid = self.fresh_id("mask");
                let b = self.xf.transform_rect_bbox(reach).inflate(1.0, 1.0);
                let (x, y, w, h) = (self.num(b.x0), self.num(b.y0), self.num(b.width()), self.num(b.height()));
                let fr = if rule == FillRule::EvenOdd { " fill-rule=\"evenodd\"" } else { "" };
                self.def(1, &format!("<mask id=\"{mid}\" {MASK} x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\">"));
                self.def(2, &format!("<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" fill=\"#fff\"/>"));
                self.def(2, &format!("<path d=\"{d}\" fill=\"#000\"{fr}/>"));
                self.def(1, "</mask>");
                format!(" mask=\"url(#{mid})\"")
            }
        }
    }

    /// Paint a shape (`d`) with a node's appearance stack. `paths` is the same shape in document
    /// space, for strokes that aren't plain (it may be empty when all are).
    fn shape(&mut self, n: &Node, d: &str, paths: &[&PathData], rule: FillRule, bounds: Option<Rect>) {
        let id = self.id_attr(n);
        let items: Vec<&AppearanceItem> = n
            .appearance
            .items
            .iter()
            .filter(|i| match i {
                AppearanceItem::Fill(f) => f.visible && !f.paint.is_none(),
                AppearanceItem::Stroke(s) => s.visible && !s.paint.is_none() && s.width > 0.0,
            })
            .collect();
        let mut bp = None;
        let plans: Vec<Option<StrokePlan>> = items
            .iter()
            .map(|i| match i {
                AppearanceItem::Stroke(s) => Some(self.stroke_plan(s, &mut bp, paths)),
                AppearanceItem::Fill(_) => None,
            })
            .collect();
        let fills = items.iter().filter(|i| matches!(i, AppearanceItem::Fill(_))).count();
        let strokes = items.len() - fills;
        // A stroke's paint box is the bounds grown by half its weight, as on the canvas.
        let paint_bounds = |s: &StrokeLayer| bounds.map(|b| s.paint_bounds(b));
        let simple = fills <= 1
            && strokes <= 1
            && !items.iter().any(|i| has_raster(i.effects()))
            && items.iter().all(|i| i.blend() == BlendMode::Normal)
            && !items.iter().any(|i| freeform(i.paint()).is_some())
            && plans.iter().flatten().all(|p| matches!(p, StrokePlan::Written(Written { shape: WrittenShape::Stroke { .. }, side: None })));
        if simple {
            let mut p = Props::new();
            match items.iter().find_map(|i| if let AppearanceItem::Fill(f) = i { Some(f) } else { None }) {
                Some(f) => self.fill_props(f, rule, bounds, &mut p),
                None => {
                    p.push(("fill", "none".into()));
                    if rule == FillRule::EvenOdd {
                        p.push(("fill-rule", "evenodd".into()));
                    }
                }
            }
            if let Some(s) = items.iter().find_map(|i| if let AppearanceItem::Stroke(s) = i { Some(s) } else { None }) {
                self.stroke_props(s, s.width, paint_bounds(s), &mut p);
                if matches!(items.first(), Some(AppearanceItem::Stroke(_))) && fills == 1 {
                    p.push(("paint-order", "stroke".into()));
                }
            }
            p.extend(css::transparency(n));
            let a = self.attrs(&p);
            self.line(&format!("<path{id} d=\"{d}\"{a}/>"));
            return;
        }
        let a = self.attrs(&css::transparency(n));
        self.line(&format!("<g{id}{a}>"));
        self.depth += 1;
        for (it, plan) in items.into_iter().zip(plans) {
            // The item's own raster effects filter that item alone.
            let filters = self.open_filters(n, it.effects());
            let mut p = Props::new();
            match (it, plan) {
                (AppearanceItem::Fill(f), _) => {
                    let blend = (f.blend != BlendMode::Normal).then(|| ("mix-blend-mode", blend_css(f.blend).to_string()));
                    if let (Some(g), Some(b)) = (freeform(&f.paint), bounds) {
                        p.extend((f.opacity < 1.0).then(|| ("opacity", fmt_num(f.opacity as f64, 3))));
                        p.extend(blend);
                        self.raster_paint(g, b, &[d.to_string()], rule, &p, "");
                    } else {
                        self.fill_props(f, rule, bounds, &mut p);
                        p.extend(blend);
                        let a = self.attrs(&p);
                        self.line(&format!("<path d=\"{d}\"{a}/>"));
                    }
                }
                (AppearanceItem::Stroke(s), Some(StrokePlan::Brush(art))) => {
                    // The brush art takes the stroke's opacity and blend mode as a group.
                    if s.opacity < 1.0 {
                        p.push(("opacity", fmt_num(s.opacity as f64, 3)));
                    }
                    if s.blend != BlendMode::Normal {
                        p.push(("mix-blend-mode", blend_css(s.blend).into()));
                    }
                    let a = self.attrs(&p);
                    self.line(&format!("<g{a}>"));
                    self.depth += 1;
                    for piece in &art {
                        self.node(piece);
                    }
                    self.depth -= 1;
                    self.line("</g>");
                }
                (AppearanceItem::Stroke(s), Some(StrokePlan::Written(w))) => {
                    let side = self.side_attr(w.side, d, rule, w.reach(s, bounds.unwrap_or_default()));
                    let blend = (s.blend != BlendMode::Normal).then(|| ("mix-blend-mode", blend_css(s.blend).to_string()));
                    let opacity = (s.opacity < 1.0).then(|| ("opacity", fmt_num(s.opacity as f64, 3)));
                    match (&w.shape, freeform(&s.paint).zip(paint_bounds(s))) {
                        (WrittenShape::Stroke { width }, Some((g, b))) => {
                            // The stroke's outline clips the gradient's image.
                            let bp = bp.get_or_insert_with(|| doc_path(paths));
                            let outline = stroke::line_outline(bp, s, *width, OUTLINE_TOLERANCE);
                            let clip = self.path_d(&PathData::from_bezpath(&outline), self.xf);
                            p.extend(opacity);
                            p.extend(blend);
                            self.raster_paint(g, b, &[clip], FillRule::NonZero, &p, &side);
                        }
                        (WrittenShape::Fill(outlines), Some((g, b))) if s.path_gradient().is_none() => {
                            let clip: Vec<String> = outlines.iter().map(|o| self.path_d(&PathData::from_bezpath(o), self.xf)).collect();
                            p.extend(opacity);
                            p.extend(blend);
                            self.raster_paint(g, b, &clip, FillRule::NonZero, &p, &side);
                        }
                        (WrittenShape::Stroke { width }, _) => {
                            p.push(("fill", "none".into()));
                            self.stroke_props(s, *width, paint_bounds(s), &mut p);
                            p.extend(blend);
                            let a = self.attrs(&p);
                            self.line(&format!("<path d=\"{d}\"{a}{side}/>"));
                        }
                        (WrittenShape::Fill(outlines), _) if s.path_gradient().is_some() => {
                            // A gradient along or across the stroke: slices clipped to its outlines.
                            if let Some(ws) = bp.as_ref().and_then(|bp| stroke::written_slices(bp, rule, s, outlines)) {
                                p.extend(opacity);
                                p.extend(blend);
                                self.sliced(&ws, &p, &side);
                            }
                        }
                        (WrittenShape::Fill(outlines), _) => {
                            let paint = self.paint(&s.paint, paint_bounds(s));
                            let ds: Vec<String> = outlines.iter().map(|o| self.path_d(&PathData::from_bezpath(o), self.xf)).collect();
                            let opacity = (s.opacity < 1.0).then(|| fmt_num(s.opacity as f64, 3));
                            if let [one] = &ds[..] {
                                p.push(("fill", paint));
                                p.extend(opacity.map(|o| ("fill-opacity", o)));
                                p.extend(blend);
                                let a = self.attrs(&p);
                                self.line(&format!("<path d=\"{one}\"{a}{side}/>"));
                            } else if !ds.is_empty() {
                                // The line and its arrowheads overlap: one group takes the opacity.
                                p.extend(opacity.map(|o| ("opacity", o)));
                                p.extend(blend);
                                let a = self.attrs(&p);
                                self.line(&format!("<g{a}{side}>"));
                                self.depth += 1;
                                let fill = self.attrs(&vec![("fill", paint)]);
                                for one in &ds {
                                    self.line(&format!("<path d=\"{one}\"{fill}/>"));
                                }
                                self.depth -= 1;
                                self.line("</g>");
                            }
                        }
                    }
                }
                (AppearanceItem::Stroke(_), None) => {}
            }
            self.close_filters(filters);
        }
        self.depth -= 1;
        self.line("</g>");
    }

    /// A stroke whose gradient runs along or across it ([`stroke::written_slices`]): a group (with
    /// `props` and the `side` attribute) of its slices, clipped to its outlines.
    fn sliced(&mut self, ws: &stroke::WrittenSlices, props: &Props, side: &str) {
        self.warn("gradients along or across strokes are written as slices of linear gradients");
        let cid = self.fresh_id("clip-path");
        let clip = self.path_d(&PathData::from_bezpath(&ws.clip), self.xf);
        self.def(1, &format!("<clipPath id=\"{cid}\">"));
        self.def(2, &format!("<path d=\"{clip}\"/>"));
        self.def(1, "</clipPath>");
        let a = self.attrs(props);
        self.line(&format!("<g{a}{side}>"));
        self.depth += 1;
        self.line(&format!("<g clip-path=\"url(#{cid})\">"));
        self.depth += 1;
        for (shape, paint) in &ws.slices {
            let fill = self.paint(paint, None);
            let fill = self.attrs(&vec![("fill", fill)]);
            let d = self.path_d(&PathData::from_bezpath(shape), self.xf);
            self.line(&format!("<path d=\"{d}\"{fill}/>"));
        }
        self.depth -= 1;
        self.line("</g>");
        self.depth -= 1;
        self.line("</g>");
    }

    fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    /// An object with an opacity mask: `<g mask="url(#…)">` around the unmasked object. The mask
    /// art goes in `<defs>`; no-clip adds a white backdrop and invert a colour-inverting filter.
    fn masked(&mut self, n: &Node, m: &vectorcraft_doc::OpacityMask) {
        if self.tiny() {
            self.warn(TINY_MASKS);
            let mut bare = n.clone();
            bare.mask = None;
            return self.node_body(&bare);
        }
        let mid = self.fresh_id("mask");
        // Mask art is a picture of its own: it takes no part in a knockout group around the object.
        let knockout = std::mem::take(&mut self.knockout);
        let art = self.detached(2, |w| w.node(&m.art));
        self.knockout = knockout;
        let inv = m.invert.then(|| {
            let fid = self.fresh_id("invert");
            self.def(1, &format!("<filter id=\"{fid}\" filterUnits=\"userSpaceOnUse\" color-interpolation-filters=\"sRGB\" {BIG}>"));
            self.def(2, "<feColorMatrix type=\"matrix\" values=\"-1 0 0 0 1 0 -1 0 0 1 0 0 -1 0 1 0 0 0 1 0\"/>");
            self.def(1, "</filter>");
            fid
        });
        // The mask's options, so import restores them (and its art without backdrop and filter).
        let flags: Vec<&str> = [(!m.clip).then_some("noclip"), m.invert.then_some("invert")].into_iter().flatten().collect();
        let data = if flags.is_empty() { String::new() } else { format!(" {MASK_FLAGS}=\"{}\"", flags.join(" ")) };
        self.def(1, &format!("<mask id=\"{mid}\" {MASK} {BIG}{data}>"));
        if let Some(fid) = &inv {
            self.def(1, &format!("<g filter=\"url(#{fid})\">"));
        }
        if !m.clip || m.invert {
            let c = if m.clip { "black" } else { "white" };
            self.def(1, &format!("<rect {BIG} fill=\"{c}\"/>"));
        }
        self.defs.push_str(&art);
        if inv.is_some() {
            self.def(1, "</g>");
        }
        self.def(1, "</mask>");
        self.line(&format!("<g mask=\"url(#{mid})\">"));
        self.depth += 1;
        let mut bare = n.clone();
        bare.mask = None;
        self.node_body(&bare);
        self.depth -= 1;
        self.line("</g>");
    }

    /// What `write` writes to the body, taken out of it (written at `depth`, for `<defs>`).
    fn detached(&mut self, depth: usize, write: impl FnOnce(&mut Self)) -> String {
        let (body, saved) = (std::mem::take(&mut self.body), self.depth);
        self.depth = depth;
        write(self);
        self.depth = saved;
        std::mem::replace(&mut self.body, body)
    }

    /// Props of a group (or layer): a knockout group is isolated, a hidden one not displayed.
    fn group_props(&self, n: &Node) -> Props {
        let mut p = css::transparency(n);
        if !n.isolate && n.knocks_out(self.knockout) {
            p.push(("isolation", "isolate".into()));
        }
        p
    }

    /// The children of group `n`, as the elements of a knockout group when it is one.
    fn group_children(&mut self, n: &Node, children: &[std::sync::Arc<Node>]) {
        let knockout = n.knocks_out(self.knockout);
        let enclosing = std::mem::replace(&mut self.knockout, knockout);
        self.children(children);
        self.knockout = enclosing;
    }

    /// Children of the group being written. SVG has no knockout groups: in one, each element is
    /// drawn through a mask of where the elements above it don't paint (the same look for Normal
    /// blending). The masks nest, so element i sits inside the masks of elements i+1…n.
    fn children(&mut self, children: &[std::sync::Arc<Node>]) {
        if self.knockout && self.tiny() {
            self.warn(TINY_MASKS);
        }
        if !self.knockout || self.tiny() {
            for c in children {
                self.node(c);
            }
            return;
        }
        let elements = Node::knockout_elements(children);
        let Some((first, rest)) = elements.split_first() else { return };
        for c in rest.iter().rev() {
            let mid = self.knockout_mask(c);
            self.line(&format!("<g mask=\"url(#{mid})\">"));
            self.depth += 1;
        }
        self.node(first);
        for c in rest {
            self.depth -= 1;
            self.line("</g>");
            self.node(c);
        }
    }

    /// A mask that is 1 − the knockout shape of `c`: white, then `c` painted black (at full object
    /// opacity without its own mask, unless those define its shape). Returns the mask id.
    fn knockout_mask(&mut self, c: &Node) -> String {
        let filter = match &self.knockout_filter {
            Some(f) => f.clone(),
            None => {
                let f = self.fresh_id("knockout-shape");
                self.def(1, &format!("<filter id=\"{f}\" filterUnits=\"userSpaceOnUse\" {BIG}>"));
                self.def(2, "<feColorMatrix type=\"matrix\" values=\"0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 1 0\"/>");
                self.def(1, "</filter>");
                self.knockout_filter = Some(f.clone());
                f
            }
        };
        let shape = if c.knockout_shape { c.clone() } else { Node { opacity: 1.0, mask: None, ..c.clone() } };
        let anonymous = std::mem::replace(&mut self.anonymous, true);
        let art = self.detached(3, |w| w.node(&shape));
        self.anonymous = anonymous;
        let mid = self.fresh_id("knockout");
        self.def(1, &format!("<mask id=\"{mid}\" {MASK} {BIG}>"));
        self.def(2, &format!("<rect {BIG} fill=\"white\"/>"));
        self.def(2, &format!("<g filter=\"url(#{filter})\">"));
        self.defs.push_str(&art);
        self.def(2, "</g>");
        self.def(1, "</mask>");
        mid
    }

    /// An object, inside `<a>` when it links to a URL (Attributes panel).
    fn node(&mut self, n: &Node) {
        let layer = usize::from(n.is_layer());
        self.layer_nest += layer;
        match n.url().filter(|_| n.visible && !self.anonymous) {
            Some(url) => {
                self.line(&format!("<a xlink:href=\"{}\">", xml_escape(url)));
                self.depth += 1;
                self.node_body(n);
                self.depth -= 1;
                self.line("</a>");
            }
            None => self.node_body(n),
        }
        self.layer_nest -= layer;
    }

    fn node_body(&mut self, n: &Node) {
        // Hidden layers and objects are left out, unless the options keep them (hidden). Template
        // layers never print.
        if !(n.visible || self.opts.hidden_layers) {
            return;
        }
        if has_raster(&n.appearance.effects) {
            return self.filtered(n);
        }
        if let Some(m) = n.mask.as_deref()
            && !m.disabled
        {
            return self.masked(n, m);
        }
        match &n.kind {
            NodeKind::Layer { template: true, .. } => {}
            NodeKind::Layer { children, clip: false, .. } | NodeKind::Group { children, clip: false } => {
                let id = self.id_attr(n);
                let a = self.attrs(&self.group_props(n));
                self.line(&format!("<g{id}{a}>"));
                self.depth += 1;
                self.group_children(n, children);
                self.depth -= 1;
                self.line("</g>");
            }
            NodeKind::Group { children, clip: true } | NodeKind::Layer { children, clip: true, .. } => {
                let Some((clip, rest)) = children.split_first() else { return };
                let cid = self.fresh_id("clip-path");
                self.def(1, &format!("<clipPath id=\"{cid}\">"));
                // The region every output clips to; with nothing to clip by, an empty clip path
                // hides the clipped art.
                if let Some((bp, rule)) = vectorcraft_effects::clip_outline(clip) {
                    let d = self.path_d(&PathData::from_bezpath(&bp), self.xf);
                    let r = if rule == FillRule::EvenOdd { " clip-rule=\"evenodd\"" } else { "" };
                    self.def(2, &format!("<path d=\"{d}\"{r}/>"));
                }
                self.def(1, "</clipPath>");
                let id = self.id_attr(n);
                let a = self.attrs(&self.group_props(n));
                // The clipping path's fill paints behind the clipped art and its stroke over it,
                // outside the clip (the group then wraps both).
                let paint = clip.clip_paint();
                if paint.stroke.is_some() {
                    self.line(&format!("<g{id}{a}>"));
                    self.depth += 1;
                    self.line(&format!("<g clip-path=\"url(#{cid})\">"));
                } else {
                    self.line(&format!("<g{id} clip-path=\"url(#{cid})\"{a}>"));
                }
                self.depth += 1;
                if let Some(fill) = &paint.fill {
                    self.node(fill);
                }
                self.group_children(n, rest);
                self.depth -= 1;
                self.line("</g>");
                if let Some(stroke) = &paint.stroke {
                    // One element keeps the clipping path's id.
                    let anonymous = self.anonymous;
                    self.anonymous |= paint.fill.is_some();
                    self.node(stroke);
                    self.anonymous = anonymous;
                    self.depth -= 1;
                    self.line("</g>");
                }
            }
            NodeKind::Path { guide: true, .. } => {}
            NodeKind::Path { path, rule, .. } => {
                if path.is_empty() {
                    return;
                }
                let d = self.path_d(path, self.xf);
                self.shape(n, &d, &[path], *rule, n.geometric_bounds());
            }
            NodeKind::Compound { children, rule } => {
                let paths: Vec<&PathData> = children.iter().filter(|c| c.visible).filter_map(|c| c.path_data()).filter(|p| !p.is_empty()).collect();
                if paths.is_empty() {
                    return;
                }
                let d: Vec<String> = paths.iter().map(|p| self.path_d(p, self.xf)).collect();
                self.shape(n, &d.join(" "), &paths, *rule, n.geometric_bounds());
            }
            NodeKind::Text(t) => self.text_node(n, t),
            NodeKind::Image(im) => {
                let id = self.id_attr(n);
                let m = self.matrix(self.xf * im.xf);
                let a = self.attrs(&css::transparency(n));
                if let Some(shared) = self.shared_images.get(&(im.key.clone(), im.width, im.height)) {
                    let shared = shared.clone();
                    self.line(&format!("<use{id} transform=\"{m}\"{a} xlink:href=\"#{shared}\"/>"));
                    return;
                }
                let Some(href) = self.image_href(im) else { return };
                self.line(&format!(
                    "<image{id} width=\"{}\" height=\"{}\" transform=\"{m}\" preserveAspectRatio=\"none\"{a} xlink:href=\"{}\"/>",
                    im.width,
                    im.height,
                    xml_escape(&href)
                ));
            }
            NodeKind::SymbolInstance { symbol, xf } => {
                let doc = self.doc;
                let Some(sym) = doc.symbols.iter().find(|s| s.name == *symbol) else { return };
                if self.symbol_nest > MAX_SYMBOL_NEST {
                    return;
                }
                let id = self.id_attr(n);
                let a = self.attrs(&css::transparency(n));
                if let Some(def) = self.symbol_use(sym, n, *xf) {
                    let m = self.xf * *xf;
                    let tr = if m == Affine::IDENTITY { String::new() } else { format!(" transform=\"{}\"", self.matrix(m)) };
                    self.line(&format!("<use{id} xlink:href=\"#{def}\"{tr}{a}/>"));
                    return;
                }
                // An instance of its own: the (stained) art moved by the instance, its pattern
                // paints and unlinked masks left in place as on the canvas.
                let mut art = vectorcraft_brush::instance_art(&sym.art, n);
                let moved = self.instance_xf * *xf;
                if moved.determinant().abs() > 1e-12 {
                    pin_to_page(&mut art, moved.inverse());
                }
                let saved = (self.xf, self.instance_xf);
                self.xf = saved.0 * *xf;
                self.instance_xf = moved;
                self.line(&format!("<g{id}{a}>"));
                self.depth += 1;
                // The symbol's art is the instance's own picture, outside any knockout around it.
                let knockout = std::mem::take(&mut self.knockout);
                self.symbol_nest += 1;
                self.node(&art);
                self.symbol_nest -= 1;
                self.knockout = knockout;
                self.depth -= 1;
                self.line("</g>");
                (self.xf, self.instance_xf) = saved;
            }
            // Live blends/envelopes/meshes export their evaluated (expanded) form.
            NodeKind::Blend { .. } | NodeKind::Envelope { .. } | NodeKind::Mesh(_) | NodeKind::Repeat(_) | NodeKind::PlacedDocument(_) => {
                let g = vectorcraft_effects::expand_live_deep(Some(self.doc), n);
                self.share_images(&g);
                self.node_body(&g);
            }
        }
    }

    /// The `href` of image bytes `b`: a `data:` URI, or with [`ImageMode::Link`] a file written
    /// next to the SVG, named after the bytes (image keys such as "raster-1" repeat across
    /// documents).
    fn blob_href(&mut self, b: &ImageBlob) -> String {
        if self.opts.images != ImageMode::Link {
            return format!("data:{};base64,{}", b.mime, base64_encode(&b.bytes));
        }
        let name = format!("{}.{}", b.content_key(), image_ext(&b.mime));
        if !self.linked.iter().any(|l| l.name == name) {
            self.linked.push(LinkedImage { name: name.clone(), bytes: b.bytes.clone() });
        }
        name
    }

    /// A freeform gradient `g` painted over `bounds` (the space [`Self::xf`] maps into user
    /// space), which SVG can't express: an `<image>` of the colour field over its painted box,
    /// sampled as the canvas samples it (at twice the box's size in points) and clipped to
    /// `clip` (outlines in user space filled by `rule`), in a group with `props` and the `side`
    /// attribute. Nothing is written for an empty box.
    fn raster_paint(&mut self, g: &GradientPaint, bounds: Rect, clip: &[String], rule: FillRule, props: &Props, side: &str) {
        use vectorcraft_color::freeform::{grid_size, painted_box, spread_scale};
        let Some(b) = painted_box(bounds) else { return };
        let (cols, rows) = grid_size(b, 2.0 * b.width().max(b.height()));
        let field = g.freeform_on(b).field(spread_scale(b));
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        let rgba: Vec<u8> = field.grid(b, cols, rows).flat_map(|([r, g, bl], a)| [q(r), q(g), q(bl), q(a)]).collect();
        let mut png = Vec::new();
        let Some(img) = image::RgbaImage::from_raw(u32::from(cols), u32::from(rows), rgba) else { return };
        if img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).is_err() {
            return;
        }
        self.warn("freeform gradients are written as images clipped to their shapes");
        let href = self.blob_href(&ImageBlob::new("image/png", png));
        let cid = self.fresh_id("clip-path");
        let r = if rule == FillRule::EvenOdd { " clip-rule=\"evenodd\"" } else { "" };
        self.def(1, &format!("<clipPath id=\"{cid}\">"));
        for d in clip {
            self.def(2, &format!("<path d=\"{d}\"{r}/>"));
        }
        self.def(1, "</clipPath>");
        let a = self.attrs(props);
        let nested = !side.is_empty();
        if nested {
            self.line(&format!("<g{a}{side}>"));
            self.depth += 1;
            self.line(&format!("<g clip-path=\"url(#{cid})\">"));
        } else {
            self.line(&format!("<g clip-path=\"url(#{cid})\"{a}>"));
        }
        self.depth += 1;
        let (x, y, w, h) = (self.num(b.x0), self.num(b.y0), self.num(b.width()), self.num(b.height()));
        let tr = if self.xf == Affine::IDENTITY { String::new() } else { format!(" transform=\"{}\"", self.matrix(self.xf)) };
        self.line(&format!(
            "<image x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\"{tr} preserveAspectRatio=\"none\" xlink:href=\"{}\"/>",
            xml_escape(&href)
        ));
        self.depth -= 1;
        self.line("</g>");
        if nested {
            self.depth -= 1;
            self.line("</g>");
        }
    }

    /// The id of the `<symbol>` def instance `inst` (transform `xf`) can `<use>`, written on first
    /// use. `None` when the instance needs its own copy of the art: stained by its fill, or moved
    /// by a transform under which the def would not look as the canvas paints it ([`Reuse`]).
    fn symbol_use(&mut self, sym: &vectorcraft_doc::Symbol, inst: &Node, xf: Affine) -> Option<String> {
        // SVG Tiny has no `<symbol>`.
        if self.tiny() || vectorcraft_brush::stain(inst).is_some() {
            return None;
        }
        if !self.symbols.contains_key(&sym.name) {
            let reuse = Reuse::of(self.doc, &sym.art);
            self.symbols.insert(sym.name.clone(), SymbolDef { id: None, reuse });
        }
        let def = self.symbols.get(&sym.name)?;
        if !def.reuse.allows(xf) {
            return None;
        }
        if let Some(id) = &def.id {
            return Some(id.clone());
        }
        // The art in symbol space, as its own picture: no knockout around it, and no object ids
        // (every instance shows it).
        let id = self.unique_id(&sanitize_id(&sym.name));
        let saved = (self.xf, self.instance_xf, std::mem::take(&mut self.knockout), std::mem::take(&mut self.names), self.anonymous);
        (self.xf, self.instance_xf, self.anonymous) = (Affine::IDENTITY, Affine::IDENTITY, false);
        self.symbol_nest += 1;
        let art = self.detached(3, |w| w.node(&sym.art));
        self.symbol_nest -= 1;
        (self.xf, self.instance_xf, self.knockout, self.names, self.anonymous) = saved;
        // The symbol's name, when the id had to differ from it, comes back on import.
        let name = if id == sym.name { String::new() } else { format!(" data-name=\"{}\"", xml_escape(&sym.name)) };
        self.def(1, &format!("<symbol id=\"{id}\"{name} overflow=\"visible\">"));
        self.defs.push_str(&art);
        self.def(1, "</symbol>");
        if let Some(def) = self.symbols.get_mut(&sym.name) {
            def.id = Some(id.clone());
        }
        Some(id)
    }

    /// Raster effects (drop shadow, glows, Gaussian blur, feather) as SVG filters: one `<g filter>`
    /// per effect, the first effect innermost (the reference app's stacking order). Photoshop-style
    /// filters have no SVG equivalent: they are left out with a warning (exports from the app turn
    /// such objects into images first).
    fn filtered(&mut self, n: &Node) {
        let mut inner = n.clone();
        inner.appearance.effects.retain(|e| !vectorcraft_effects::is_raster(&e.id));
        let opened = self.open_filters(n, &n.appearance.effects);
        self.node_body(&inner);
        self.close_filters(opened);
    }

    /// Open one `<g filter>` per visible raster effect of `effects` (an object's, or one fill or
    /// stroke's) on `n`, outermost last effect; returns how many to close.
    fn open_filters(&mut self, n: &Node, effects: &[vectorcraft_doc::Effect]) -> usize {
        use vectorcraft_effects::RasterFx;
        let fx = vectorcraft_effects::raster_effects(effects);
        if self.tiny() {
            if !fx.is_empty() {
                self.warn("SVG Tiny 1.2 has no filters: shadows, glows, blurs and feathers are left out");
            }
            return 0;
        }
        let bounds = n.visual_bounds();
        let reach: f64 = fx.iter().map(|f| f.outset(bounds.unwrap_or_default())).sum::<f64>() + 2.0;
        let region = bounds.map(|b| self.xf.transform_rect_bbox(b).inflate(reach, reach));
        let mut opened = 0;
        for f in fx.iter().rev() {
            if matches!(f, RasterFx::Pixel(_)) {
                self.warn("SVG has no pixel effects such as Radial Blur, Smart Blur or Unsharp Mask: they are left out");
                continue;
            }
            let fid = self.fresh_id("filter");
            let region_attr = match region {
                Some(r) => format!(
                    " filterUnits=\"userSpaceOnUse\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"",
                    self.num(r.x0),
                    self.num(r.y0),
                    self.num(r.width()),
                    self.num(r.height())
                ),
                None => String::new(),
            };
            let sd = |blur: f64| fmt_num((blur / 2.0).max(0.0), 3);
            let flood = |c: &vectorcraft_color::Color, o: f32| {
                format!("<feFlood flood-color=\"{}\" flood-opacity=\"{}\"/>", c.to_hex(), fmt_num(o as f64, 3))
            };
            let body = match f {
                RasterFx::DropShadow { opacity, dx, dy, blur, color, .. } => format!(
                    "<feGaussianBlur in=\"SourceAlpha\" stdDeviation=\"{}\"/><feOffset dx=\"{}\" dy=\"{}\" result=\"shadow\"/>{}<feComposite in2=\"shadow\" operator=\"in\" result=\"paint\"/><feMerge><feMergeNode in=\"paint\"/><feMergeNode in=\"SourceGraphic\"/></feMerge>",
                    sd(*blur),
                    self.num(*dx),
                    self.num(*dy),
                    flood(color, *opacity)
                ),
                RasterFx::OuterGlow { opacity, blur, color, .. } => format!(
                    "<feGaussianBlur in=\"SourceAlpha\" stdDeviation=\"{}\" result=\"glow\"/>{}<feComposite in2=\"glow\" operator=\"in\" result=\"paint\"/><feMerge><feMergeNode in=\"paint\"/><feMergeNode in=\"SourceGraphic\"/></feMerge>",
                    sd(*blur),
                    flood(color, *opacity)
                ),
                RasterFx::InnerGlow { opacity, blur, color, center, .. } => {
                    // Edge: the blurred inverse silhouette; Center: the blurred silhouette; both clipped to the shape.
                    let src = if *center {
                        "<feGaussianBlur in=\"SourceAlpha\" stdDeviation=\"SD\" result=\"glow\"/>".replace("SD", &sd(*blur))
                    } else {
                        format!(
                            "<feComponentTransfer in=\"SourceAlpha\"><feFuncA type=\"table\" tableValues=\"1 0\"/></feComponentTransfer><feGaussianBlur stdDeviation=\"{}\" result=\"glow\"/>",
                            sd(*blur)
                        )
                    };
                    format!(
                        "{src}{}<feComposite in2=\"glow\" operator=\"in\"/><feComposite in2=\"SourceAlpha\" operator=\"in\" result=\"paint\"/><feMerge><feMergeNode in=\"SourceGraphic\"/><feMergeNode in=\"paint\"/></feMerge>",
                        flood(color, *opacity)
                    )
                }
                RasterFx::Feather { radius } => format!(
                    "<feGaussianBlur in=\"SourceAlpha\" stdDeviation=\"{}\" result=\"soft\"/><feComposite in=\"SourceGraphic\" in2=\"soft\" operator=\"in\" result=\"f\"/><feComposite in=\"f\" in2=\"SourceAlpha\" operator=\"in\"/>",
                    sd(*radius)
                ),
                RasterFx::GaussianBlur { radius } => format!("<feGaussianBlur in=\"SourceGraphic\" stdDeviation=\"{}\"/>", sd(*radius)),
                RasterFx::Pixel(_) => continue,
            };
            // Filters composite normally: a shadow's or glow's other blend mode is recorded for import.
            let mode = match f {
                RasterFx::DropShadow { mode, .. } | RasterFx::OuterGlow { mode, .. } | RasterFx::InnerGlow { mode, .. }
                    if *mode != BlendMode::Normal =>
                {
                    format!(" {}=\"{}\"", crate::import::BLEND, blend_css(*mode))
                }
                _ => String::new(),
            };
            self.def(1, &format!("<filter id=\"{fid}\"{region_attr}{mode} color-interpolation-filters=\"sRGB\">{body}</filter>"));
            self.line(&format!("<g filter=\"url(#{fid})\">"));
            self.depth += 1;
            opened += 1;
        }
        opened
    }

    fn close_filters(&mut self, opened: usize) {
        for _ in 0..opened {
            self.depth -= 1;
            self.line("</g>");
        }
    }

    /// The `<text>`/`<tspan>` properties of a character style. `space`: the text's layout bounds
    /// and the map from text space to user space, which character gradients resolve against
    /// (as on the canvas).
    fn char_props(&mut self, st: &CharStyle, space: (Rect, Affine)) -> Props {
        let decimals = self.opts.decimals;
        let len = |v: f64| fmt_num(v, decimals);
        let mut p = css::font_props(st, &len);
        // Character paints resolve in the text's space (`to_user` maps it into user space).
        let (bounds, to_user) = space;
        let saved = std::mem::replace(&mut self.xf, to_user);
        let fill = self.paint(&st.fill, Some(bounds));
        p.push(("fill", fill));
        if st.has_stroke() {
            // Weight, cap, join, miter limit and dashes as for object strokes; a gradient spans
            // the text grown by half the weight, as on the canvas.
            let layer = st.stroke_layer();
            self.stroke_props(&layer, layer.width, Some(layer.paint_bounds(bounds)), &mut p);
        }
        self.xf = saved;
        p.extend(css::type_props(st, &len));
        p
    }

    /// Type: its characters (live text, or glyph outlines when text is exported as outlines) and,
    /// as the canvas paints them, the object's own fills and strokes on the glyph outlines: those
    /// below the Characters row under the characters, the others over them.
    ///
    /// Inline graphics ([`vectorcraft_doc::TextRun::inline`]) are written after the characters as
    /// instances of their symbols (a `<use>` of the symbol's def where it can be shared), all in
    /// one group carrying the object's transparency. The live text skips their characters and
    /// places the text after each one; type on a path with inline graphics is written as outlines
    /// (a `<textPath>` couldn't leave room for the art).
    fn text_node(&mut self, n: &Node, t: &TextObject) {
        let doc = self.doc;
        let resolved = doc.inline_resolved(t);
        let t = &*resolved;
        let inline = t.runs.iter().any(|r| r.inline.is_some());
        let outlined = self.opts.outline_text || t.vertical || (inline && matches!(t.kind, TextKind::OnPath { .. }));
        let chars = |w: &mut Self, n: &Node| if outlined { w.text_outlines(n, t) } else { w.text(n, t) };
        let painted = n.appearance.items.iter().any(|i| i.visible() && !i.paint().is_none());
        if !painted && !inline {
            return chars(self, n);
        }
        let id = self.id_attr(n);
        let a = self.attrs(&css::transparency(n));
        self.line(&format!("<g{id}{a}>"));
        self.depth += 1;
        let db = vectorcraft_text::FontDb::global();
        let lay = vectorcraft_text::layout(db, t);
        let mut all = lay.to_bezpath();
        for (_, bar) in vectorcraft_text::decorations(&lay, db, t) {
            all.extend(bar.iter());
        }
        let pd = PathData::from_bezpath(&all).transformed(t.xf);
        let d = self.path_d(&pd, self.xf);
        let tb = Some(t.xf.transform_rect_bbox(lay.bounds));
        let (below, above) = n.appearance.split_contents();
        let glyphs = |w: &mut Self, items: &[AppearanceItem]| {
            if !items.is_empty() {
                let ap = vectorcraft_doc::Appearance { items: items.to_vec(), ..Default::default() };
                w.shape(&Node::path(NodeId(u64::MAX), pd.clone(), ap), &d, &[&pd], FillRule::NonZero, tb);
            }
        };
        glyphs(self, below);
        // The group carries the id and the object's transparency.
        let bare = Node { opacity: 1.0, blend: BlendMode::Normal, isolate: false, ..n.clone() };
        let anonymous = std::mem::replace(&mut self.anonymous, true);
        chars(self, &bare);
        for ig in &lay.inlines {
            let Some(art) = t.runs.get(ig.run).and_then(|r| r.inline.as_ref()) else { continue };
            let inst = Node::new(
                NodeId(u64::MAX),
                NodeKind::SymbolInstance { symbol: art.symbol.clone(), xf: t.xf * ig.xf * doc.symbol_natural_xf(&art.symbol) },
            );
            self.node(&inst);
        }
        glyphs(self, above);
        self.anonymous = anonymous;
        self.depth -= 1;
        self.line("</g>");
    }

    /// Text as glyph outlines: one compound path per run, painted like the run (gradients span
    /// the whole text, as on the canvas).
    fn text_outlines(&mut self, n: &Node, t: &TextObject) {
        let db = vectorcraft_text::FontDb::global();
        let lay = vectorcraft_text::layout(db, t);
        let mut runs: Vec<(usize, kurbo::BezPath)> = vec![];
        // Underline and strikethrough bars join their run's outlines.
        let bars = vectorcraft_text::decorations(&lay, db, t);
        for (run, outline) in lay.glyphs.iter().map(|g| (g.run, &g.outline)).chain(bars.iter().map(|(r, b)| (*r, b))) {
            match runs.iter_mut().find(|(r, _)| *r == run) {
                Some((_, bp)) => bp.extend(outline.iter()),
                None => runs.push((run, outline.clone())),
            }
        }
        let id = self.id_attr(n);
        let a = self.attrs(&css::transparency(n));
        self.line(&format!("<g{id}{a}>"));
        self.depth += 1;
        // The glyphs stay in text space; the text's transform joins the document's meanwhile.
        let saved = self.xf;
        self.xf = saved * t.xf;
        for (r, bp) in runs {
            let Some(run) = t.runs.get(r) else { continue };
            let pd = PathData::from_bezpath(&bp);
            // An unnamed id: the pieces carry no id attribute (the group has it).
            let glyphs = Node::path(NodeId(u64::MAX), pd.clone(), run.style.appearance());
            let d = self.path_d(&pd, self.xf);
            self.shape(&glyphs, &d, &[&pd], FillRule::NonZero, Some(lay.bounds));
        }
        self.xf = saved;
        self.depth -= 1;
        self.line("</g>");
    }

    /// Point and area type, every laid-out line at its own position: wraps, indents, tabs and
    /// spacing come out as on the canvas. Centred and right-aligned lines are anchored
    /// (`text-anchor`) at their centre or right end, so they stay aligned in a viewer whose font
    /// differs; other lines are placed where each style run (and, justified, each word) starts.
    fn text(&mut self, n: &Node, t: &TextObject) {
        // Live SVG text is written in logical order from left-aligned pieces: bidirectional text
        // (and right-to-left paragraphs) keep their look as outlines until it is written with
        // `direction`/`unicode-bidi`.
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
        if lay.glyphs.iter().any(|g| g.rtl) || lay.lines.iter().any(|l| l.rtl) {
            self.warn("bidirectional text is outlined to preserve shaping and visual order");
            return self.text_outlines(n, t);
        }
        if let TextKind::OnPath { path, start, end } = &t.kind {
            // `<textPath>` has a start offset only: an end bracket, Align to Path and Spacing
            // keep their look as outlines.
            if end.is_some() || t.path_align != vectorcraft_doc::PathAlign::Baseline || t.path_spacing != 0.0 {
                self.warn("type on a path with an end bracket, Align to Path or Spacing is outlined to keep its look");
                return self.text_outlines(n, t);
            }
            return self.text_on_path(n, t, path, *start);
        }
        self.note_fonts(t, &lay);
        let lines = text_lines(t, &lay, self.opts.fewer_tspans);
        // Each line is anchored as its paragraph is aligned. A tab starts a new chunk at its
        // stop, and so does the text after an inline graphic: lines with tabs, and text with
        // inline graphics, are never anchored.
        let has_inline = t.runs.iter().any(|r| r.inline.is_some());
        let anchors: Vec<Option<(&str, f64)>> = lines
            .iter()
            .map(|l| {
                match t.para_at(l.para).justify {
                    Justify::Center => Some(("middle", 0.5)),
                    Justify::Right => Some(("end", 1.0)),
                    _ => None,
                }
                .filter(|_| !has_inline && !l.segs.iter().any(|s| s.brk))
            })
            .collect();
        // Every line anchored alike (the text's only paragraph style, typically): the <text>
        // carries the anchor. Otherwise each anchored line is a <tspan> carrying its own.
        let uniform = anchors.first().copied().filter(|a| a.is_some() && anchors.iter().all(|b| b == a)).flatten();
        // The <text> element's user space is text space.
        let space = (lay.bounds, Affine::IDENTITY);
        let base = TextBase { props: self.char_props(&t.first_style(), space), space, anchored: uniform.is_some() };
        let mut props = base.props.clone();
        if let Some((a, _)) = uniform {
            props.push(("text-anchor", a.into()));
        }
        props.extend(css::transparency(n));
        let id = self.id_attr(n);
        let a = self.attrs(&props);
        let m = self.xf * t.xf;
        let tr = if m == Affine::IDENTITY { String::new() } else { format!(" transform=\"{}\"", self.matrix(m)) };
        let starts: Vec<Option<(f64, f64)>> = lines.iter().zip(&anchors).map(|(l, a)| line_start(&l.segs, a.map(|a| a.1))).collect();
        // The `<text>` carries the first line's position too, so readers that place text by its
        // own x/y start where the first line does.
        let at = match starts.iter().flatten().next() {
            Some((x, y)) => format!(" x=\"{}\" y=\"{}\"", self.num(*x), self.num(*y)),
            None => String::new(),
        };
        let mut s = format!("<text{id}{tr}{at} xml:space=\"preserve\"{a}>");
        for ((line, start), anchor) in lines.iter().zip(starts).zip(&anchors) {
            let Some(start) = start else { continue };
            match anchor.filter(|_| uniform.is_none()) {
                // A line anchored on its own: one positioned <tspan> with the anchor around it.
                Some((a, _)) => {
                    s.push_str(&format!("<tspan x=\"{}\" y=\"{}\" text-anchor=\"{a}\">", self.num(start.0), self.num(start.1)));
                    let inner = TextBase { anchored: true, ..base.clone() };
                    self.text_line(&mut s, t, &line.segs, start, &inner, false);
                    s.push_str("</tspan>");
                }
                None => self.text_line(&mut s, t, &line.segs, start, &base, true),
            }
        }
        s.push_str("</text>");
        self.line(&s);
    }

    /// One line of laid-out text starting at `start`. Default: a positioned `<tspan>` per segment
    /// (only the first of an anchored line). Fewer tspans: one positioned `<tspan>` per line with
    /// the style changes nested in it, positioned again only after tabs and justified word spaces.
    /// Baseline shifts are relative (`dy`), undone by the next segment that isn't shifted.
    /// `positioned`: false when the caller already positioned the line (an outer `<tspan>`).
    fn text_line(&mut self, s: &mut String, t: &TextObject, line: &[Segment], start: (f64, f64), base: &TextBase, positioned: bool) {
        let fewer = self.opts.fewer_tspans && positioned;
        if fewer {
            s.push_str(&format!("<tspan x=\"{}\" y=\"{}\">", self.num(start.0), self.num(start.1)));
        }
        let mut place = !fewer && positioned;
        let mut first = true;
        // The baseline shift the current text position carries.
        let mut shift = 0.0;
        for seg in line {
            let Some(st) = t.runs.get(seg.run).map(|r| &r.style) else { continue };
            let mut attrs = String::new();
            if place {
                let (x, y) = if first { start } else { (seg.x, seg.y) };
                attrs.push_str(&format!(" x=\"{}\" y=\"{}\"", self.num(x), self.num(y)));
                shift = 0.0;
            }
            first = false;
            if st.baseline_shift != shift {
                attrs.push_str(&format!(" dy=\"{}\"", self.num(shift - st.baseline_shift)));
                shift = st.baseline_shift;
            }
            // After a tab or a justified word space the next segment is placed.
            place = seg.brk || (!fewer && !base.anchored && positioned);
            if (st.h_scale - st.v_scale).abs() > 1e-9 {
                attrs.push_str(&format!(" textLength=\"{}\" lengthAdjust=\"spacingAndGlyphs\"", self.num(seg.advance)));
            }
            if st.rotation != 0.0 {
                attrs.push_str(&format!(" rotate=\"{}\"", self.num(-st.rotation)));
            }
            let diff = run_diff(&base.props, self.char_props(st, base.space));
            attrs.push_str(&self.attrs(&diff));
            let text = xml_escape(&seg.text);
            if attrs.is_empty() && fewer {
                s.push_str(&text);
            } else {
                s.push_str(&format!("<tspan{attrs}>{text}</tspan>"));
            }
        }
        if fewer {
            s.push_str("</tspan>");
        }
    }

    /// Record the characters of laid-out type `t` under the faces that supply them, for
    /// [`Self::font_faces`] (when fonts are embedded).
    fn note_fonts(&mut self, t: &TextObject, lay: &vectorcraft_text::TextLayout) {
        if !self.opts.embed_fonts || self.opts.outline_text {
            return;
        }
        if self.tiny() {
            return self.warn("SVG Tiny 1.2 has no web fonts: fonts are not embedded");
        }
        let db = FontDb::global();
        let plain = t.plain_text();
        let mut face: Option<Arc<FontFace>> = None;
        for g in &lay.glyphs {
            let Some(st) = t.runs.get(g.run).map(|r| &r.style) else { continue };
            if face.as_ref().is_none_or(|f| f.id() != g.font_id) {
                face = db.face_by_id(g.font_id);
            }
            let Some(f) = &face else { continue };
            let (weight, italic) = font_descriptor(st);
            let at = self.fonts.iter().position(|u| u.face.id() == f.id() && (u.weight, u.italic) == (weight, italic) && u.family == st.font_family);
            let at = at.unwrap_or_else(|| {
                let family = st.font_family.clone();
                self.fonts.push(FontUse { family, weight, italic, face: f.clone(), chars: BTreeSet::new() });
                self.fonts.len() - 1
            });
            let Some(chars) = self.fonts.get_mut(at).map(|u| &mut u.chars) else { continue };
            // As written: a hyphen where a line breaks, capitals for All Caps.
            let src = if g.len == 0 { "-" } else { plain.get(g.byte..g.byte + g.len).unwrap_or("") };
            for c in src.chars().filter(|c| !c.is_control() && *c != '\u{ad}') {
                if st.all_caps {
                    chars.extend(c.to_uppercase());
                } else {
                    chars.insert(c);
                }
            }
        }
    }

    /// The `@font-face` rules of the faces type uses ([`ExportOptions::embed_fonts`]): each face
    /// subset to its characters (whole when its licence forbids subsetting, left out with a
    /// warning when it forbids embedding). Faces sharing a description cover their own
    /// characters (`unicode-range`).
    fn font_faces(&mut self) -> Vec<String> {
        let uses = std::mem::take(&mut self.fonts);
        let mut rules = Vec::new();
        for u in &uses {
            let chars: Vec<char> = u.chars.iter().copied().collect();
            let name = format!("{} {}", u.face.family, u.face.style);
            let Some(font) = u.face.embed(&chars) else {
                self.warn(&format!("the licence of the font {name} doesn't allow embedding: viewers show its type in their own fonts"));
                continue;
            };
            if !font.subset {
                self.warn(&format!("the font {name} is embedded whole: its licence doesn't allow subsetting"));
            }
            let shared = uses.iter().filter(|o| (&o.family, o.weight, o.italic) == (&u.family, u.weight, u.italic)).count() > 1;
            let range = if shared { format!(";unicode-range:{}", unicode_range(&u.chars)) } else { String::new() };
            let (mime, format) = if font.cff { ("font/otf", "opentype") } else { ("font/ttf", "truetype") };
            rules.push(format!(
                "@font-face{{font-family:{};font-weight:{};font-style:{};src:url(data:{mime};base64,{}) format(\"{format}\"){range}}}",
                css_string(&u.family),
                u.weight,
                if u.italic { "italic" } else { "normal" },
                base64_encode(&font.data)
            ));
        }
        rules
    }

    /// Type on a path: a `<textPath>` along the path (a def).
    fn text_on_path(&mut self, n: &Node, t: &TextObject, path: &PathData, start: f64) {
        let id = self.id_attr(n);
        let pid = self.fresh_id("text-path");
        let d = self.path_d(path, self.xf * t.xf);
        self.def(1, &format!("<path id=\"{pid}\" d=\"{d}\"/>"));
        // The <text> element's user space is the document's; gradients span the laid-out text.
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
        self.note_fonts(t, &lay);
        let space = (lay.bounds, self.xf * t.xf);
        let base = self.char_props(&t.first_style(), space);
        let mut props = base.clone();
        match t.para_at(0).justify {
            Justify::Center | Justify::JustifyCenter => props.push(("text-anchor", "middle".into())),
            Justify::Right | Justify::JustifyRight => props.push(("text-anchor", "end".into())),
            _ => {}
        }
        props.extend(css::transparency(n));
        let a = self.attrs(&props);
        let offset = fmt_num(start * 100.0, 3);
        let mut s = format!("<text{id} xml:space=\"preserve\"{a}><textPath xlink:href=\"#{pid}\" startOffset=\"{offset}%\">");
        for run in &t.runs {
            let diff = run_diff(&base, self.char_props(&run.style, space));
            let a = self.attrs(&diff);
            s.push_str(&format!("<tspan{a}>{}</tspan>", xml_escape(&run.text.replace('\n', " "))));
        }
        s.push_str("</textPath></text>");
        self.line(&s);
    }
}

/// What the `<tspan>` of a run whose properties are `run` sets over its `<text>`'s `base`: the
/// properties that differ, and the initial value of each one `base` sets and the run doesn't.
fn run_diff(base: &Props, run: Props) -> Props {
    let reset: Props =
        base.iter().filter(|(k, _)| !run.iter().any(|(r, _)| r == k)).filter_map(|(k, _)| initial_value(k).map(|v| (*k, v.to_string()))).collect();
    run.into_iter().filter(|kv| !base.contains(kv)).chain(reset).collect()
}

/// The initial value of an inherited property a run may leave unset (decorations can't be undone
/// on a `<tspan>`).
fn initial_value(k: &str) -> Option<&'static str> {
    Some(match k {
        "font-weight" | "font-style" | "font-feature-settings" => "normal",
        "stroke" | "stroke-dasharray" => "none",
        "stroke-width" | "stroke-opacity" => "1",
        "stroke-linecap" => "butt",
        "stroke-linejoin" => "miter",
        "stroke-miterlimit" => "4",
        "stroke-dashoffset" | "letter-spacing" => "0",
        "font-kerning" => "auto",
        _ => return None,
    })
}

/// A shape's paths as one path (document space).
fn doc_path(paths: &[&PathData]) -> BezPath {
    let mut bp = BezPath::new();
    for p in paths {
        bp.extend(p.to_bezpath());
    }
    bp
}

/// How closely stroke outlines that clip an image follow the stroke (points).
const OUTLINE_TOLERANCE: f64 = 0.01;

/// The freeform gradient a paint is, if it is one (SVG has no freeform gradients).
fn freeform(p: &Paint) -> Option<&GradientPaint> {
    match p {
        Paint::Gradient(g) if g.gradient.kind == GradientKind::Freeform => Some(g),
        _ => None,
    }
}

/// Any visible raster effect (shadow, glow, blur, feather) in `effects`?
fn has_raster(effects: &[vectorcraft_doc::Effect]) -> bool {
    effects.iter().any(|e| e.visible && vectorcraft_effects::is_raster(&e.id))
}

/// What the lines of one `<text>` share.
#[derive(Clone)]
struct TextBase {
    /// The `<text>` element's character properties, which each `<tspan>` differs from.
    props: Props,
    /// The text's layout bounds and the map from text space to user space (character paints).
    space: (Rect, Affine),
    /// Lines anchored at their centre or right end (`text-anchor`), positioned once each.
    anchored: bool,
}

/// One laid-out line: its paragraph (index) and its segments.
struct Line {
    para: usize,
    segs: Vec<Segment>,
}

/// Characters of one style on one line, written as one `<tspan>`.
struct Segment {
    /// Index into `TextObject::runs`.
    run: usize,
    /// Where the segment starts (text space; `y` is the baseline).
    x: f64,
    y: f64,
    text: String,
    /// The laid-out width (tracking and justification included).
    advance: f64,
    /// The next segment needs its own position (after a tab, or a word space of a justified line).
    brk: bool,
}

/// The laid-out lines of `t` as segments: a new segment at every style change, after tabs and,
/// on justified lines, after each word space (unless `fewer`). The text comes from the clusters
/// the layout placed: soft hyphens draw nothing, a line broken by hyphenation ends in `-`, all
/// caps are upper case.
fn text_lines(t: &TextObject, lay: &vectorcraft_text::TextLayout, fewer: bool) -> Vec<Line> {
    let plain = t.plain_text();
    let mut lines = Vec::with_capacity(lay.lines.len());
    for line in &lay.lines {
        let para = plain.as_bytes().get(..line.start).map_or(0, |b| b.iter().filter(|&&c| c == b'\n').count());
        let split_words = !fewer && !matches!(t.para_at(para).justify, Justify::Auto | Justify::Left | Justify::Center | Justify::Right);
        let mut segs: Vec<Segment> = Vec::new();
        let mut cluster = None;
        let source = |g: &vectorcraft_text::PositionedGlyph| if g.len == 0 { "-" } else { plain.get(g.byte..g.byte + g.len).unwrap_or("") };
        let mut glyphs = lay.glyphs.get(line.glyph_start..line.glyph_end).unwrap_or_default();
        // Spaces where the line wrapped (it doesn't end its paragraph) don't belong to the line.
        if plain.get(line.end..).is_some_and(|rest| !rest.is_empty() && !rest.starts_with('\n')) {
            while let Some((_, rest)) = glyphs.split_last().filter(|(g, _)| source(g).chars().all(|c| c.is_whitespace() && c != '\t')) {
                glyphs = rest;
            }
        }
        let mut after_inline = false;
        for g in glyphs {
            let Some(run) = t.runs.get(g.run) else { continue };
            if run.inline.is_some() {
                // An inline graphic (written as art): the text after it is placed anew.
                if let Some(last) = segs.last_mut() {
                    last.brk = true;
                }
                after_inline = true;
                cluster = None;
                continue;
            }
            if g.len > 0 && cluster == Some(g.byte) {
                // Another glyph of the same cluster.
                if let Some(last) = segs.last_mut() {
                    last.advance += g.advance;
                }
                continue;
            }
            cluster = Some(g.byte);
            let src: String = source(g).chars().filter(|c| *c != '\u{ad}').collect();
            let piece = if run.style.all_caps { src.to_uppercase() } else { src };
            let brk = piece == "\t" || (split_words && !piece.is_empty() && piece.chars().all(char::is_whitespace));
            let fresh = std::mem::take(&mut after_inline);
            match segs.last_mut() {
                Some(last) if last.run == g.run && !last.brk && !fresh => {
                    last.text.push_str(&piece);
                    last.advance += g.advance;
                    last.brk = brk;
                }
                _ => segs.push(Segment { run: g.run, x: g.origin.x, y: g.origin.y, text: piece, advance: g.advance, brk }),
            }
        }
        segs.retain(|s| !s.text.is_empty());
        if !segs.is_empty() {
            lines.push(Line { para, segs });
        }
    }
    lines
}

/// Where a line of segments starts: its first segment's origin or, `anchor`ed (0.5: centre, 1:
/// right end), that point of its width. `None` for an empty line.
fn line_start(line: &[Segment], anchor: Option<f64>) -> Option<(f64, f64)> {
    let (first, last) = (line.first()?, line.last()?);
    let x = match anchor {
        Some(f) => first.x + f * (last.x + last.advance - first.x),
        None => first.x,
    };
    Some((x, first.y))
}
