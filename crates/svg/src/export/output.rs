use super::*;

/// XML 1.0 literal attribute whitespace is normalized to spaces; references retain the value.
pub(super) fn xml_attr_escape(s: &str) -> String {
    xml_escape(s).replace('\t', "&#9;").replace('\n', "&#10;").replace('\r', "&#13;")
}

/// Escape type for XML content. Characters XML 1.0 does not allow are dropped, except the line
/// and page breaks (U+000B, U+000C) other apps paste, which become spaces.
pub(super) fn xml_text(s: &str) -> String {
    let s: String = s
        .chars()
        .filter_map(|c| match c {
            '\u{b}' | '\u{c}' => Some(' '),
            c if valid_xml_value(c.encode_utf8(&mut [0; 4])) => Some(c),
            _ => None,
        })
        .collect();
    xml_escape(&s)
}

pub(super) fn valid_xml_value(s: &str) -> bool {
    s.chars().all(|c| matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}'))
}

/// An XML name without a colon (an NCName): a letter or `_`, then [`xml_name_char`]s.
fn xml_ncname(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_') && chars.all(xml_name_char)
}

/// Namespace-free XML 1.0 NameChar. The fixed `data-` prefix supplies NameStartChar.
fn xml_name_char(c: char) -> bool {
    matches!(c,
        'A'..='Z' | 'a'..='z' | '0'..='9' | '_' | '-' | '.' | '\u{b7}' |
        '\u{c0}'..='\u{d6}' | '\u{d8}'..='\u{f6}' | '\u{f8}'..='\u{2ff}' |
        '\u{300}'..='\u{37d}' | '\u{37f}'..='\u{1fff}' | '\u{200c}'..='\u{200d}' |
        '\u{203f}'..='\u{2040}' | '\u{2070}'..='\u{218f}' | '\u{2c00}'..='\u{2fef}' |
        '\u{3001}'..='\u{d7ff}' | '\u{f900}'..='\u{fdcf}' | '\u{fdf0}'..='\u{fffd}' |
        '\u{10000}'..='\u{effff}'
    )
}

/// Write every character of `s` beyond ISO 8859-1 as a character reference. Called only when
/// names fit Latin-1, so references occur in values, text or style sheets.
pub(super) fn latin1_refs(s: &mut String) {
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

/// A CSS declaration block as the literal value of an `<!ENTITY>`: quotes, `%` and markup
/// characters become references that still read as themselves inside the `style` attribute.
pub(super) fn entity_value(decl: &str) -> String {
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
    pub(super) fn tiny(&self) -> bool {
        self.opts.profile == Profile::Tiny12
    }

    pub(super) fn num(&self, v: f64) -> String {
        fmt_num(v, self.opts.decimals)
    }

    pub(super) fn indent(&self, depth: usize) -> String {
        if self.opts.minify { String::new() } else { "  ".repeat(depth) }
    }

    pub(super) fn line(&mut self, s: &str) {
        let ind = self.indent(self.depth);
        self.body.push_str(&ind);
        self.body.push_str(s);
        if !self.opts.minify {
            self.body.push('\n');
        }
    }

    pub(super) fn def(&mut self, depth: usize, s: &str) {
        let ind = self.indent(depth + 1);
        self.defs.push_str(&ind);
        self.defs.push_str(s);
        if !self.opts.minify {
            self.defs.push('\n');
        }
    }

    pub(super) fn fresh_id(&mut self, prefix: &str) -> String {
        let next = self.next_fresh.entry(prefix.to_owned()).or_insert(1);
        loop {
            let id = format!("{}{prefix}-{next}", self.id_prefix);
            *next += 1;
            if self.used_ids.insert(id.clone()) {
                return id;
            }
        }
    }

    /// `base` (with the unique prefix) if free, else `base-2`, `base-3`…
    pub(super) fn unique_id(&mut self, base: &str) -> String {
        let base = format!("{}{base}", self.id_prefix);
        if self.used_ids.insert(base.clone()) {
            return base;
        }
        let next = self.next_suffix.entry(base.clone()).or_insert(2);
        loop {
            let cand = format!("{base}-{next}");
            *next += 1;
            if self.used_ids.insert(cand.clone()) {
                return cand;
            }
        }
    }

    /// Reserve ids for every named object up front so generated def ids never collide with them.
    pub(super) fn assign_name_ids(&mut self) {
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
    /// The id of `n`'s element (with its name when the id isn't it), then the object's own data
    /// as `data-*` attributes. Sublayers also retain their import marker.
    pub(super) fn id_attr(&mut self, n: &Node) -> String {
        if self.anonymous {
            return String::new();
        }
        let mut out = self.names.get(&n.id).map_or_else(String::new, |id| format!(" id=\"{}\"", xml_escape(id)));
        if let (Some(id), Some(name)) = (self.names.get(&n.id), n.name.as_deref())
            && name != id
        {
            if valid_xml_value(name) {
                out.push_str(&format!(" data-name=\"{}\"", xml_attr_escape(name)));
            } else {
                self.warn("Object names containing invalid XML characters were left out of SVG name attributes");
            }
        }
        // Preserve sublayer identity independently of object data attributes.
        if !out.is_empty() && n.is_layer() && self.layer_nest > 1 {
            out.push_str(&format!(" {}=\"sublayer\"", crate::import::SUBLAYER));
        }
        let mut keys = HashSet::new();
        for (k, v) in n.extras().attrs.as_deref().map_or(&[][..], |a| a.data.as_slice()) {
            // Ours (`name`, `vc-…`) are written by the export itself.
            if k == "name" || k.starts_with("vc-") {
                continue;
            }
            if k.is_empty() || !k.chars().all(xml_name_char) || !valid_xml_value(v) {
                self.warn("Data attributes with invalid XML names or characters were left out of SVG");
                continue;
            }
            // Native data is an ordered list. Keep the first valid value of each key.
            if !keys.insert(k.as_str()) {
                self.warn("Repeated data attribute keys were written only once in SVG");
                continue;
            }
            self.non_latin1_names |= k.chars().any(|c| u32::from(c) > 0xff);
            out.push_str(&format!(" data-{k}=\"{}\"", xml_attr_escape(v)));
        }
        self.foreign_attrs(n, &mut out);
        self.event_attrs(n, &mut out);
        out
    }

    /// `n`'s attributes in other applications' namespaces (`plotter:pen="2"`), added to `out`.
    /// Each namespace is declared once, on the root, under the prefix the file gave it unless the
    /// export uses that prefix itself or for another namespace ([`Writer::foreign_prefix`]).
    fn foreign_attrs(&mut self, n: &Node, out: &mut String) {
        let attrs = n.extras().attrs.as_deref().map_or(&[][..], |a| a.foreign.as_slice());
        let mut seen = HashSet::new();
        for a in attrs {
            if !xml_ncname(&a.prefix) || !xml_ncname(&a.name) || a.ns.is_empty() || !valid_xml_value(&a.ns) || !valid_xml_value(&a.value) {
                self.warn("Attributes of other applications with invalid XML names or characters were left out of SVG");
                continue;
            }
            // Native data is a list: one attribute per namespace and name.
            if !seen.insert((a.ns.as_str(), a.name.as_str())) {
                self.warn("Repeated attributes of other applications were written only once in SVG");
                continue;
            }
            let prefix = self.foreign_prefix(&a.ns, &a.prefix);
            self.non_latin1_names |= prefix.chars().chain(a.name.chars()).any(|c| u32::from(c) > 0xff);
            out.push_str(&format!(" {prefix}:{}=\"{}\"", a.name, xml_attr_escape(&a.value)));
        }
    }

    /// The prefix namespace `ns` is written under: `wanted` (the file's), else `wanted2`,
    /// `wanted3`… when the export's own prefixes or another namespace already have it. XML keeps
    /// prefixes starting with `xml` for itself: those become `ns`, `ns2`…
    fn foreign_prefix(&mut self, ns: &str, wanted: &str) -> String {
        if let Some(p) = self.foreign_ns.get(ns) {
            return p.clone();
        }
        const OURS: [&str; 5] = ["svg", "xlink", "rdf", "dc", "vectorcraft"];
        let base = if wanted.to_ascii_lowercase().starts_with("xml") { "ns" } else { wanted };
        let taken = |p: &str| OURS.contains(&p) || self.foreign_ns.values().any(|q| q == p);
        let mut prefix = base.to_string();
        let mut k = 2;
        while taken(&prefix) {
            prefix = format!("{base}{k}");
            k += 1;
        }
        self.foreign_ns.insert(ns.to_string(), prefix.clone());
        prefix
    }

    /// Window › SVG Interactivity: `n`'s events as event attributes (`onclick="…"`), added to
    /// `out`. The SVG Tiny 1.2 profile leaves them out with a warning.
    fn event_attrs(&mut self, n: &Node, out: &mut String) {
        let events = n.extras().attrs.as_deref().map_or(&[][..], |a| a.events.as_slice());
        if events.is_empty() {
            return;
        }
        if self.tiny() {
            self.warn("SVG Tiny 1.2 has no event attributes: SVG Interactivity events were left out");
            return;
        }
        let mut seen = HashSet::new();
        for (event, script) in events {
            // Native data can name anything: only the panel's events are written, once each.
            if !SVG_EVENTS.contains(&event.as_str()) || !valid_xml_value(script) {
                self.warn("SVG Interactivity events with unknown names or invalid XML characters were left out of SVG");
                continue;
            }
            if !seen.insert(event.as_str()) {
                self.warn("Repeated SVG Interactivity events were written only once in SVG");
                continue;
            }
            out.push_str(&format!(" {event}=\"{}\"", xml_attr_escape(script)));
        }
    }

    pub(super) fn matrix(&self, m: Affine) -> String {
        let c = m.as_coeffs();
        format!("matrix({} {} {} {} {} {})", self.num(c[0]), self.num(c[1]), self.num(c[2]), self.num(c[3]), self.num(c[4]), self.num(c[5]))
    }

    /// How style properties are written: the styling option, but SVG Tiny has presentation
    /// attributes alone.
    pub(super) fn styling(&self) -> Styling {
        if self.tiny() { Styling::PresentationAttributes } else { self.opts.styling }
    }

    /// Style properties as attributes, an inline style, or a CSS class, per the styling option.
    pub(super) fn attrs(&mut self, props: &Props) -> String {
        if props.is_empty() {
            return String::new();
        }
        match self.styling() {
            Styling::PresentationAttributes => {
                // CSS-only properties go in a style attribute (SVG Tiny has none: left out).
                let (style, attrs): (Vec<_>, Vec<_>) = props.iter().partition(|(k, _)| {
                    matches!(*k, "mix-blend-mode" | "isolation" | "font-feature-settings" | "font-variation-settings" | "font-kerning")
                });
                let mut s: String = attrs.iter().map(|(k, v)| format!(" {k}=\"{}\"", xml_escape(v))).collect();
                if !style.is_empty() {
                    if self.tiny() {
                        self.warn("SVG Tiny 1.2 has no style sheets: blend modes, isolation, font features and font variation settings are left out");
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
        if let Some(&i) = self.class_ids.get(decl.as_str()) {
            return i;
        }
        let i = self.classes.len() + 1;
        self.class_ids.insert(decl.clone(), i);
        self.classes.push(decl);
        i
    }

    /// `<metadata>`: the Dublin Core terms of File Info ([`ExportOptions::metadata`]) and the
    /// native document ([`ExportOptions::preserve_editing`]) with the [`body_hash`] of the markup
    /// around this element.
    pub(super) fn metadata(&self, out: &mut String, native: Option<(&[u8], String)>) {
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

    pub(super) fn path_d(&self, p: &PathData, m: Affine) -> String {
        let mut s = String::new();
        // Each point as "x y", appended in place (no string per number or per point).
        let dec = self.opts.decimals;
        let pt = |s: &mut String, p: Point| {
            let p = m * p;
            crate::push_num(s, p.x, dec);
            s.push(' ');
            crate::push_num(s, p.y, dec);
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
            pt(&mut s, sp.anchors[0].p);
            for i in 0..sp.segment_count() {
                let (a, b) = (&sp.anchors[i], &sp.anchors[(i + 1) % n]);
                if sp.segment_is_line(i) {
                    if sp.closed && i == n - 1 {
                        break;
                    }
                    s.push_str(" L");
                    pt(&mut s, b.p);
                } else {
                    s.push_str(" C");
                    pt(&mut s, a.h_out);
                    s.push(' ');
                    pt(&mut s, b.h_in);
                    s.push(' ');
                    pt(&mut s, b.p);
                }
            }
            if sp.closed {
                s.push_str(" Z");
            }
        }
        s
    }

    /// `p` written with `m` as the SVG basic shape it exactly is, as the reference app's SVG
    /// export writes simple shapes (#4652): an axis-aligned `<rect>`, `<circle>` or `<ellipse>`,
    /// a `<line>`, or a `<polygon>`/`<polyline>` of straight segments. The element name and its
    /// geometry attributes, at the written precision; `None` keeps `<path>`. `<rect>` and
    /// `<ellipse>` start their outline elsewhere than the path, so with a `dashed` stroke a
    /// rectangle is a `<polygon>` and an ellipse stays `<path>`.
    /// Sources: an Illustrator ellipse is an `ellipse` tag unless an anchor is added
    /// (<https://community.adobe.com/t5/illustrator-discussions/trouble-exporting-shape-with-points-to-svg/td-p/10720325>);
    /// rectangles export as `<rect>` and `<polygon>`
    /// (<https://community.adobe.com/t5/illustrator-discussions/force-svg-export-using-all-lt-rect-gt/td-p/11282378>);
    /// a triangle without curves exports as a polygon (<https://hackaday.io/project/170755/log/175565>).
    pub(super) fn basic_shape(&self, p: &PathData, m: Affine, dashed: bool) -> Option<(&'static str, String)> {
        let [sp] = p.subpaths.as_slice() else { return None };
        if !(0..sp.segment_count()).all(|i| sp.segment_is_line(i)) {
            return self.ellipse_shape(sp, m).filter(|_| !dashed);
        }
        let pts: Vec<Point> = sp.anchors.iter().map(|a| m * a.p).collect();
        if sp.closed && !dashed && pts.len() == 4 {
            // The corners as written, so the rectangle's numbers are the path's own.
            let snap = |v: f64| self.num(v).parse::<f64>().unwrap_or(v);
            let corners: Vec<Point> = pts.iter().map(|q| Point::new(snap(q.x), snap(q.y))).collect();
            if let Some(r) = axis_rect(&corners) {
                let (x, y, w, h) = (self.num(r.x0), self.num(r.y0), self.num(r.width()), self.num(r.height()));
                return Some(("rect", format!(" x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\"")));
            }
        }
        let points = |tag: &'static str| {
            let mut s = String::from(" points=\"");
            for (i, q) in pts.iter().enumerate() {
                if i > 0 {
                    s.push(' ');
                }
                crate::push_num(&mut s, q.x, self.opts.decimals);
                s.push(',');
                crate::push_num(&mut s, q.y, self.opts.decimals);
            }
            s.push('"');
            Some((tag, s))
        };
        match (sp.closed, pts.as_slice()) {
            (false, [a, b]) => {
                let (x1, y1, x2, y2) = (self.num(a.x), self.num(a.y), self.num(b.x), self.num(b.y));
                // A line of zero length as written keeps `<path>`: renderers disagree on its caps.
                ((&x1, &y1) != (&x2, &y2)).then(|| ("line", format!(" x1=\"{x1}\" y1=\"{y1}\" x2=\"{x2}\" y2=\"{y2}\"")))
            }
            (false, [_, _, _, ..]) => points("polyline"),
            (true, [_, _, _, ..]) => points("polygon"),
            _ => None,
        }
    }

    /// A closed 4-anchor subpath that is an axis-aligned ellipse drawn the standard way (each
    /// quarter a cubic with handles [`KAPPA`] of the radii long), within the written precision:
    /// `<circle>` when its radii are written the same, else `<ellipse>`.
    fn ellipse_shape(&self, sp: &SubPath, m: Affine) -> Option<(&'static str, String)> {
        let tol = 10f64.powi(-i32::from(self.opts.decimals));
        let (anchors, closed) = (sp.anchors.as_slice(), sp.closed);
        let [a0, a1, a2, a3] = anchors else { return None };
        let [p0, p1, p2, p3] = [a0.p, a1.p, a2.p, a3.p].map(|p| m * p);
        let c = p0.midpoint(p2);
        let near = |a: f64, b: f64| (a - b).abs() <= tol;
        // Opposite anchors share the centre: one pair on the horizontal axis, the other on the vertical.
        let (rx, ry) = if near(p0.y, c.y) && near(p1.x, c.x) {
            ((p0.x - c.x).abs(), (p1.y - c.y).abs())
        } else if near(p0.x, c.x) && near(p1.y, c.y) {
            ((p1.x - c.x).abs(), (p0.y - c.y).abs())
        } else {
            return None;
        };
        let quarters = anchors.iter().zip(anchors.iter().cycle().skip(1)).all(|(a, b)| {
            let (pa, pb) = (m * a.p, m * b.p);
            (m * a.h_out).distance(pa + (pb - c) * KAPPA) <= tol && (m * b.h_in).distance(pb + (pa - c) * KAPPA) <= tol
        });
        if !closed || c.distance(p1.midpoint(p3)) > tol || rx <= tol || ry <= tol || !quarters {
            return None;
        }
        let (cx, cy, rx, ry) = (self.num(c.x), self.num(c.y), self.num(rx), self.num(ry));
        Some(if rx == ry {
            ("circle", format!(" cx=\"{cx}\" cy=\"{cy}\" r=\"{rx}\""))
        } else {
            ("ellipse", format!(" cx=\"{cx}\" cy=\"{cy}\" rx=\"{rx}\" ry=\"{ry}\""))
        })
    }

    pub(super) fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }
}

/// The rectangle whose corners `q` are, in order either way round, with its edges alternately
/// horizontal and vertical and none of zero length (`<rect>` doesn't draw an empty one).
fn axis_rect(q: &[Point]) -> Option<Rect> {
    let [a, b, c, d] = *q else { return None };
    let h = |p: Point, q: Point| p.y == q.y && p.x != q.x;
    let v = |p: Point, q: Point| p.x == q.x && p.y != q.y;
    ((h(a, b) && v(b, c) && h(c, d) && v(d, a)) || (v(a, b) && h(b, c) && v(c, d) && h(d, a))).then(|| Rect::from_points(a, c))
}
