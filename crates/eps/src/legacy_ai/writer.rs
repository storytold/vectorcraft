use super::encoding::{
    Ink, average, direct_fill, direct_stroke, finite, font_of, invertible, latin_string, lost_attributes, segment_point, unit, writable,
};
use super::{
    Affine, AnchorKind, AppearanceItem, Arc, BezPath, BlendMode, CharStyle, Cms, Color, Document, ELLIPTICAL, FREEFORM, FillLayer, FillRule,
    Gradient, GradientKind, GradientPaint, HEX_LINE, HIDDEN, HIDDEN_LAYER, HIDDEN_SHOWN, HiddenObjects, IMAGE_ALPHA, ImageBlob, Justify, LARGE_IMAGE,
    LAYER_COLORS, LAYER_NAME, LIVE, LayerColor, LegacyAiOptions, LineCap, LineJoin, MAX_IMAGE_PIXELS, MAX_NEST, NAMES, Node, NodeKind, PATTERNS,
    Paint, ParaDirection, ParaStyle, PathData, Point, Rect, STROKES, SUBLAYERS, Shape, StrokeLayer, SubPath, TITLE_AND_COLOR_NAMES, TRANSPARENCY,
    TYPE, TYPE_ATTRIBUTES, TextKind, TextObject, UNDECODED, Vec2, Write, latin, ps, stroke,
};

pub(super) struct Writer<'a> {
    doc: &'a Document,
    /// The document → art space.
    to_art: Affine,
    cms: Arc<Cms>,
    pub(super) body: String,
    /// Gradient definitions, by the name objects use.
    pub(super) gradients: Vec<(String, Gradient)>,
    /// Spot colors used: the name written and full CMYK equivalent.
    pub(super) spots: Vec<(String, [f32; 4])>,
    /// The name each spot color (by its swatch's name) is written with ([`Self::spot`]).
    spot_names: Vec<(String, String)>,
    pub(super) warnings: Vec<String>,
    /// The lock state the last `A` set.
    locked: bool,
    /// Symbols and live objects being written inside one another.
    nest: u32,
    /// What to do with hidden objects.
    hidden: HiddenObjects,
    /// Type as outlines.
    outline_text: bool,
    /// The layer being written's hidden objects for its hidden layer, and whether each is locked.
    hidden_art: Vec<(Node, bool)>,
    /// Writing a layer that is hidden (or the hidden layer of one's hidden objects): hidden objects
    /// are written where they are.
    in_hidden: bool,
    /// The fonts type is set in: PostScript name, and whether it's TrueType.
    pub(super) fonts: Vec<(String, bool)>,
}

impl<'a> Writer<'a> {
    pub(super) fn new(doc: &'a Document, to_art: Affine, o: &LegacyAiOptions) -> Self {
        Self {
            doc,
            to_art,
            cms: vectorcraft_color::cms::active(),
            body: String::new(),
            gradients: vec![],
            spots: vec![],
            spot_names: vec![],
            warnings: vec![],
            locked: false,
            nest: 0,
            hidden: o.hidden,
            outline_text: o.outline_text,
            hidden_art: vec![],
            in_hidden: false,
            fonts: vec![],
        }
    }

    fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    fn line(&mut self, s: &str) {
        self.body.push_str(s);
        self.body.push('\n');
    }

    // ---------- layers and objects ----------

    /// `name` with each character outside Windows-1252 (the encoding type and names are written
    /// in) replaced by `?`; adds the warning `warning` when it replaces one.
    pub(super) fn latin_name(&mut self, name: &str, warning: &str) -> String {
        let mut lost = false;
        let kept = name
            .chars()
            .map(|c| {
                if latin::encode(c).is_some() {
                    c
                } else {
                    lost = true;
                    '?'
                }
            })
            .collect();
        if lost {
            self.warn(warning);
        }
        kept
    }

    /// A layer's name as a PostScript string in Windows-1252, like the type; a character outside it
    /// is written as `?`, with a warning.
    fn layer_name(&mut self, name: &str) -> String {
        latin_string(&self.latin_name(name, LAYER_NAME))
    }

    /// Note spot color `name` (its swatch's name), of full CMYK equivalent `cmyk`, as a custom
    /// color of the document, and give the name it is written with. Its characters outside
    /// Windows-1252 are written as `?` (with a warning), and it is numbered when another spot color
    /// is already written with that name; the name is the same each time.
    fn spot(&mut self, name: &str, cmyk: [f32; 4]) -> String {
        let written = match self.spot_names.iter().find(|(n, _)| n == name) {
            Some((_, w)) => w.clone(),
            None => {
                let base = self.latin_name(name, TITLE_AND_COLOR_NAMES);
                let mut written = base.clone();
                let mut i = 1;
                while self.spot_names.iter().any(|(_, w)| *w == written) {
                    i += 1;
                    written = format!("{base} {i}");
                }
                self.spot_names.push((name.to_string(), written.clone()));
                written
            }
        };
        if !self.spots.iter().any(|(n, _)| *n == written) {
            self.spots.push((written.clone(), cmyk));
        }
        written
    }

    /// The name spot color `name` (its swatch's name) is written with, once [`Self::spot`] has
    /// noted it.
    pub(super) fn spot_name<'n>(&'n self, name: &'n str) -> &'n str {
        self.spot_names.iter().find(|(n, _)| n == name).map_or(name, |(_, w)| w.as_str())
    }

    /// A top-level layer. False when it isn't a layer.
    pub(super) fn layer(&mut self, n: &Node) -> bool {
        let NodeKind::Layer { color, template, printable, children, clip, preview, dim_images } = &n.kind else { return false };
        let [r, g, b] = match color {
            LayerColor::Preset(i) => LAYER_COLORS.get(usize::from(*i)).map_or([79, 128, 255], |c| c.1),
            LayerColor::Custom(rgb) => *rgb,
        };
        let index = match color {
            LayerColor::Preset(i) if usize::from(*i) < LAYER_COLORS.len() => i64::from(*i),
            _ => -1,
        };
        let flag = |b: bool| u8::from(b);
        let name = self.layer_name(n.name.as_deref().unwrap_or("Layer"));
        // A template layer doesn't print and shows dimmed.
        let _ = writeln!(
            self.body,
            "%AI5_BeginLayer\n{} {} {} {} {} 0 {index} {r} {g} {b} Lb\n{} Ln",
            flag(n.visible),
            flag(*preview),
            flag(!n.locked),
            flag(*printable && !*template),
            flag(dim_images.is_some() || *template),
            name,
        );
        self.locked = false;
        self.line("0 A");
        self.in_hidden = !n.visible;
        self.contents(children, *clip, false);
        self.in_hidden = false;
        self.line("LB\n%AI5_EndLayer--");
        true
    }

    /// The hidden layer of top-level layer `n`'s hidden objects, just above it (named after it,
    /// with its colour, lock and printing). False when it has none.
    pub(super) fn hidden_layer(&mut self, n: &Node) -> bool {
        if self.hidden_art.is_empty() {
            return false;
        }
        let art = std::mem::take(&mut self.hidden_art);
        let NodeKind::Layer { color, template, printable, .. } = &n.kind else { return false };
        let [r, g, b] = match color {
            LayerColor::Preset(i) => LAYER_COLORS.get(usize::from(*i)).map_or([79, 128, 255], |c| c.1),
            LayerColor::Custom(rgb) => *rgb,
        };
        let index = match color {
            LayerColor::Preset(i) if usize::from(*i) < LAYER_COLORS.len() => i64::from(*i),
            _ => -1,
        };
        let name = format!("{} (hidden)", n.name.as_deref().unwrap_or("Layer"));
        let name = self.layer_name(&name);
        let _ = writeln!(
            self.body,
            "%AI5_BeginLayer\n0 1 {} {} {} 0 {index} {r} {g} {b} Lb\n{} Ln",
            u8::from(!n.locked),
            u8::from(*printable && !*template),
            u8::from(*template),
            name,
        );
        self.locked = false;
        self.line("0 A");
        self.in_hidden = true;
        for (node, locked) in &art {
            self.node(node, *locked);
        }
        self.in_hidden = false;
        self.line("LB\n%AI5_EndLayer--");
        true
    }

    /// A container's members, the first clipping the others when `clip`; `locked`: the container is.
    fn contents(&mut self, children: &[Arc<Node>], clip: bool, locked: bool) {
        if clip {
            self.clip_group(children, locked);
        } else {
            for c in children {
                self.node(c, locked);
            }
        }
    }

    /// Set the lock state of the objects that follow.
    fn lock(&mut self, locked: bool) {
        if locked != self.locked {
            self.locked = locked;
            self.line(if locked { "1 A" } else { "0 A" });
        }
    }

    /// One object (inside a container locked when `locked`).
    fn node(&mut self, n: &Node, locked: bool) {
        if !n.visible && !self.in_hidden {
            match self.hidden {
                HiddenObjects::Omit => {
                    self.warn(HIDDEN);
                    return;
                }
                HiddenObjects::Layer => {
                    self.warn(HIDDEN_LAYER);
                    self.hidden_art.push((n.clone(), locked || n.locked));
                    return;
                }
                HiddenObjects::Visible => self.warn(HIDDEN_SHOWN),
            }
        }
        if n.opacity < 1.0 || n.blend != BlendMode::Normal || n.extras().mask.as_ref().is_some_and(|m| !m.disabled) {
            self.warn(TRANSPARENCY);
        }
        if n.name.as_deref().is_some_and(|s| !s.is_empty()) && !n.is_layer() {
            self.warn(NAMES);
        }
        let locked = locked || n.locked;
        match &n.kind {
            NodeKind::Layer { children, clip, .. } => {
                self.warn(SUBLAYERS);
                self.group(children, *clip, locked);
            }
            NodeKind::Group { children, clip } => self.group(children, *clip, locked),
            NodeKind::Path { path, rule, guide, .. } => {
                self.lock(locked);
                if *guide {
                    self.guide(path);
                } else {
                    self.shape(n, path, *rule);
                }
            }
            NodeKind::Compound { children, rule } => {
                self.lock(locked);
                let mut subpaths = vec![];
                for p in children.iter().filter_map(|c| c.path_data()) {
                    subpaths.extend(p.subpaths.iter().cloned());
                }
                self.shape(n, &PathData::new(subpaths), *rule);
            }
            NodeKind::Text(t) => {
                self.lock(locked);
                self.text(t);
            }
            NodeKind::Image(im) => {
                self.lock(locked);
                self.image(self.doc.image_blob(im), im.width, im.height, im.xf);
            }
            NodeKind::SymbolInstance { symbol, xf } => {
                self.warn(LIVE);
                if let Some(sym) = self.doc.symbols.iter().find(|s| &s.name == symbol)
                    && self.nest < MAX_NEST
                {
                    let mut art = (*sym.art).clone();
                    art.transform(*xf, false);
                    art.visible = true;
                    self.nest += 1;
                    self.node(&art, locked);
                    self.nest -= 1;
                }
            }
            NodeKind::Blend { .. }
            | NodeKind::Envelope { .. }
            | NodeKind::Mesh(_)
            | NodeKind::Repeat(_)
            | NodeKind::PlacedDocument(_)
            | NodeKind::Intertwine { .. } => {
                self.warn(LIVE);
                if self.nest < MAX_NEST {
                    let g = vectorcraft_effects::expand_live_deep(Some(self.doc), n);
                    self.nest += 1;
                    if let Some(children) = g.children() {
                        self.group(children, false, locked);
                    }
                    self.nest -= 1;
                }
            }
        }
    }

    /// A group (`u` … `U`), or a clipping group (`q` … `Q`).
    fn group(&mut self, children: &[Arc<Node>], clip: bool, locked: bool) {
        if children.is_empty() {
            return;
        }
        self.lock(locked);
        if clip {
            self.clip_group(children, locked);
        } else {
            self.line("u");
            self.contents(children, false, locked);
            self.lock(locked);
            self.line("U");
        }
    }

    /// A clipping group: its first member's outline clips the others (`h W n`). That member's fills
    /// paint behind the art, inside the clip, and its strokes over it, outside.
    fn clip_group(&mut self, children: &[Arc<Node>], locked: bool) {
        let Some((clip, rest)) = children.split_first() else { return };
        // Nothing to clip by hides the clipped art.
        let Some((outline, rule)) = vectorcraft_effects::clip_outline(clip) else { return };
        let paint = clip.clip_paint();
        self.lock(locked);
        self.line("q");
        let path = PathData::from_bezpath(&outline);
        self.fill_rule(rule);
        let several = path.subpaths.len() > 1;
        if several {
            self.line("*u");
        }
        for sp in path.subpaths.iter().filter(|sp| !sp.anchors.is_empty()) {
            self.mask(sp);
        }
        if several {
            self.line("*U");
        }
        if let Some(fill) = &paint.fill {
            self.node(fill, locked);
        }
        for c in rest {
            self.node(c, locked);
        }
        self.lock(locked);
        self.line("Q");
        if let Some(stroke) = &paint.stroke {
            self.node(stroke, locked);
        }
    }

    // ---------- paths ----------

    /// A clipping path (§5.6.2): its points, `h` (closed) or `H`, `W`, then painted with nothing.
    fn mask(&mut self, sp: &SubPath) {
        self.subpath(sp);
        self.line(if sp.closed { "h\nW\nn" } else { "H\nW\nN" });
    }

    /// A guide (§6): `(N) *`, its path, `(N) *`.
    fn guide(&mut self, path: &PathData) {
        for sp in path.subpaths.iter().filter(|sp| !sp.anchors.is_empty()) {
            self.line("(N) *");
            self.subpath(sp);
            self.line("(N) *");
        }
    }

    /// A path's fills and strokes: one object with one fill and a plain stroke over it where it
    /// can be, else a group of an object for each (in paint order).
    fn shape(&mut self, n: &Node, path: &PathData, rule: FillRule) {
        if path.is_empty() {
            return;
        }
        let items: Vec<&AppearanceItem> = n
            .appearance
            .items
            .iter()
            .filter(|i| match i {
                AppearanceItem::Fill(f) => f.visible && !f.paint.is_none(),
                AppearanceItem::Stroke(s) => s.visible && !s.paint.is_none() && s.width > 0.0 && s.width.is_finite(),
            })
            .collect();
        for i in &items {
            if i.opacity() < 1.0 || i.blend() != BlendMode::Normal {
                self.warn(TRANSPARENCY);
            }
        }
        match items.as_slice() {
            [] => self.path_object(path, rule, None, None),
            [AppearanceItem::Fill(f)] if direct_fill(&f.paint) => self.path_object(path, rule, Some(f), None),
            [AppearanceItem::Stroke(s)] if direct_stroke(s) => self.path_object(path, rule, None, Some(s)),
            [AppearanceItem::Fill(f), AppearanceItem::Stroke(s)] if direct_fill(&f.paint) && direct_stroke(s) => {
                self.path_object(path, rule, Some(f), Some(s))
            }
            [one] => self.item(path, rule, one),
            items => {
                self.line("u");
                for i in items {
                    self.item(path, rule, i);
                }
                self.line("U");
            }
        }
    }

    /// One fill or stroke of a path as an object of its own.
    fn item(&mut self, path: &PathData, rule: FillRule, item: &AppearanceItem) {
        match item {
            AppearanceItem::Fill(f) => self.fill_object(path, rule, f),
            AppearanceItem::Stroke(s) if direct_stroke(s) => self.path_object(path, rule, None, Some(s)),
            AppearanceItem::Stroke(s) => {
                self.warn(STROKES);
                let outline = stroke::outline_region(path, rule, s);
                let fill = FillLayer { overprint: s.overprint, ..FillLayer::new(s.paint.clone()) };
                self.fill_object(&outline, FillRule::NonZero, &fill);
            }
        }
    }

    /// A fill as an object of its own: patterns as their tiles clipped to the path.
    fn fill_object(&mut self, path: &PathData, rule: FillRule, f: &FillLayer) {
        if direct_fill(&f.paint) {
            return self.path_object(path, rule, Some(f), None);
        }
        let Paint::Pattern { pattern, xf } = &f.paint else {
            // A freeform gradient: its average colour.
            self.warn(FREEFORM);
            let color = match &f.paint {
                Paint::Gradient(g) => average(&g.gradient),
                _ => Color::gray(0.5),
            };
            let flat = FillLayer { paint: Paint::solid(color), ..f.clone() };
            return self.path_object(path, rule, Some(&flat), None);
        };
        self.warn(PATTERNS);
        let doc = self.doc;
        let Some(def) = doc.pattern(pattern) else {
            let flat = FillLayer { paint: Paint::solid(Color::gray(0.5)), ..f.clone() };
            return self.path_object(path, rule, Some(&flat), None);
        };
        let area = path.to_bezpath().bounding_box();
        let tiles = def.instances_in(*xf, area);
        self.line("q");
        self.fill_rule(rule);
        let several = path.subpaths.len() > 1;
        if several {
            self.line("*u");
        }
        for sp in path.subpaths.iter().filter(|sp| !sp.anchors.is_empty()) {
            self.mask(sp);
        }
        if several {
            self.line("*U");
        }
        let locked = self.locked;
        for t in &tiles {
            self.node(t, locked);
        }
        self.lock(locked);
        self.line("Q");
    }

    /// A path object painted with `fill` (a colour or a linear or radial gradient) and `stroke`
    /// (a plain line of a colour) over it; several subpaths make a compound path.
    fn path_object(&mut self, path: &PathData, rule: FillRule, fill: Option<&FillLayer>, stroke: Option<&StrokeLayer>) {
        let subpaths: Vec<&SubPath> = path.subpaths.iter().filter(|sp| !sp.anchors.is_empty()).collect();
        if subpaths.is_empty() {
            return;
        }
        let gradient = match fill.map(|f| &f.paint) {
            Some(Paint::Gradient(g)) => Some(self.gradient(g, path.to_bezpath().bounding_box())),
            _ => None,
        };
        if let Some(f) = fill {
            let paint = match &f.paint {
                // The colour under the gradient: its first stop's.
                Paint::Gradient(g) => g.gradient.stops.first().map_or(Paint::None, |s| Paint::solid(s.color)),
                p => p.clone(),
            };
            self.color(&paint, false);
            self.line(if f.overprint { "1 O" } else { "0 O" });
        }
        if let Some(s) = stroke {
            self.color(&s.paint, true);
            self.line(if s.overprint { "1 R" } else { "0 R" });
            self.line_style(s);
        }
        self.fill_rule(rule);
        let op = match (fill.is_some(), stroke.is_some()) {
            (true, true) => 'b',
            (true, false) => 'f',
            (false, true) => 's',
            (false, false) => 'n',
        };
        let several = subpaths.len() > 1;
        if several {
            self.line("*u");
        }
        for sp in subpaths {
            self.subpath(sp);
            if let Some(g) = &gradient {
                // The gradient fills between `Bb` and `BB`, whose flag strokes the path too (§3.5.3).
                self.body.push_str(g);
                self.line(if sp.closed { "f" } else { "F" });
                self.line(match (stroke.is_some(), sp.closed) {
                    (false, _) => "0 BB",
                    (true, false) => "1 BB",
                    (true, true) => "2 BB",
                });
            } else {
                self.line(&if sp.closed { op.to_string() } else { op.to_ascii_uppercase().to_string() });
            }
        }
        if several {
            self.line("*U");
        }
    }

    /// `rule` as `XR` (1: even-odd).
    fn fill_rule(&mut self, rule: FillRule) {
        self.line(if rule == FillRule::EvenOdd { "1 XR" } else { "0 XR" });
    }

    /// One subpath's points, in art space: `m`, then a line or curve to each point (lower case: the
    /// point is smooth, upper case: a corner, §5.3.2), and for a closed one the segment back to the
    /// start, which says whether the start is smooth or a corner (as the specification's examples
    /// close paths: the start point again, then the painting operator).
    fn subpath(&mut self, sp: &SubPath) {
        let Some(first) = sp.anchors.first() else { return };
        let xf = self.to_art;
        let s = &mut self.body;
        let pt = |s: &mut String, p: Point| {
            let p = xf * p;
            ps::push_nums(s, &[p.x, p.y]);
        };
        pt(s, first.p);
        s.push_str("m\n");
        let segment = |s: &mut String, a: &vectorcraft_geom::Anchor, b: &vectorcraft_geom::Anchor| {
            let straight = a.h_out == a.p && b.h_in == b.p;
            let smooth = b.kind == AnchorKind::Smooth;
            if straight {
                pt(s, b.p);
                s.push_str(if smooth { "l\n" } else { "L\n" });
            } else {
                pt(s, a.h_out);
                pt(s, b.h_in);
                pt(s, b.p);
                s.push_str(if smooth { "c\n" } else { "C\n" });
            }
        };
        for pair in sp.anchors.windows(2) {
            if let [a, b] = pair {
                segment(s, a, b);
            }
        }
        if sp.closed
            && sp.anchors.len() > 1
            && let Some(last) = sp.anchors.last()
        {
            segment(s, last, first);
        }
    }

    /// The stroke's width, cap, join, miter limit and dashes: `w J j M d`.
    fn line_style(&mut self, st: &StrokeLayer) {
        let cap = match st.cap {
            LineCap::Butt => 0,
            LineCap::Round => 1,
            LineCap::Square => 2,
        };
        let join = match st.join {
            LineJoin::Miter => 0,
            LineJoin::Round => 1,
            LineJoin::Bevel => 2,
        };
        let s = &mut self.body;
        ps::push_num(s, st.width);
        let _ = write!(s, " w {cap} J {join} j ");
        ps::push_num(s, if st.miter_limit.is_finite() { st.miter_limit.max(1.0) } else { 10.0 });
        s.push_str(" M [");
        let dash = st.dash.as_ref().filter(|d| d.is_dashed());
        if let Some(d) = dash {
            ps::push_nums(s, &d.pattern);
            s.pop();
        }
        s.push_str("] ");
        ps::push_num(s, dash.map_or(0.0, |d| d.offset));
        s.push_str(" d\n");
    }

    // ---------- colour ----------

    /// How `p` (a solid paint) is written.
    pub(super) fn ink(&self, p: &Paint) -> Ink {
        let Paint::Solid { color, swatch, tint } = p else { return Ink::Gray(0.0) };
        let doc = self.doc;
        let intent = self.cms.settings().intent;
        if let Some(name) = swatch
            && let Some(sw) = doc.swatch(name).filter(|s| s.spot)
            && let Some(full) = sw.paint.color()
        {
            let full = doc.linked_color(full, true);
            return Ink::Spot { cmyk: self.cms.to_cmyk(&full, intent).map(unit), name: name.clone(), tint: unit(*tint) };
        }
        match *color {
            // Grey is ink coverage; the format's grey is lightness.
            Color::Gray { k } => Ink::Gray(unit(1.0 - k)),
            Color::Cmyk { c, m, y, k } => Ink::Cmyk([c, m, y, k].map(unit)),
            Color::Rgb { r, g, b } => Ink::Rgb(self.cms.to_cmyk(color, intent).map(unit), [r, g, b].map(unit)),
            Color::Lab { .. } => {
                let rgb = color.to_rgb().map(unit);
                Ink::Rgb(self.cms.to_cmyk(color, intent).map(unit), rgb)
            }
        }
    }

    /// Set the fill (or with `stroke`, the stroke) colour to solid paint `p`.
    fn color(&mut self, p: &Paint, stroke: bool) {
        let ink = match self.ink(p) {
            Ink::Spot { cmyk, name, tint } => Ink::Spot { name: self.spot(&name, cmyk), cmyk, tint },
            ink => ink,
        };
        let op = |fill: &'static str, line: &'static str| if stroke { line } else { fill };
        let s = &mut self.body;
        match ink {
            Ink::Gray(v) => {
                ps::push_num(s, f64::from(v));
                let _ = writeln!(s, " {}", op("g", "G"));
            }
            Ink::Cmyk(c) => {
                ps::push_nums(s, &c.map(f64::from));
                let _ = writeln!(s, "{}", op("k", "K"));
            }
            Ink::Rgb(_, rgb) => {
                ps::push_nums(s, &rgb.map(f64::from));
                let _ = writeln!(s, "{}", op("Xa", "XA"));
            }
            Ink::Spot { cmyk, name, tint } => {
                ps::push_nums(s, &cmyk.map(f64::from));
                let _ = write!(s, "{} ", latin_string(&name));
                // The format's tint is how much lighter than the full colour (0: full).
                ps::push_num(s, f64::from(1.0 - tint));
                let _ = writeln!(s, " {}", op("x", "X"));
            }
        }
    }

    // ---------- gradients ----------

    /// The definition gradient `g` uses (defined once for each distinct gradient), and its
    /// instance on a path of bounds `bounds`: `Bb`, a radial's highlight `x y angle fraction Bh`
    /// (its distance a fraction of the radius, at most 1, §3.5.5; `0 0 0 0 Bh` at the center,
    /// §3.5.6), `1 (name) x y angle length 1 0 0 1 0 0 Bg`.
    fn gradient(&mut self, g: &GradientPaint, bounds: Rect) -> String {
        let name = self.define(&g.gradient, g.swatch.as_deref());
        for stop in &g.gradient.stops {
            if stop.opacity < 1.0 {
                self.warn(TRANSPARENCY);
            }
            if let Some(sw) = &stop.swatch {
                // Spot colours in stops are custom colours too.
                let p = Paint::Solid { color: stop.color, swatch: Some(sw.clone()), tint: stop.tint };
                if let Ink::Spot { cmyk, name, .. } = self.ink(&p) {
                    self.spot(&name, cmyk);
                }
            }
        }
        let geom = g.resolve(bounds);
        let (start, end) = (self.to_art * geom.start, self.to_art * geom.end);
        let d = end - start;
        let (angle, length) = (d.y.atan2(d.x).to_degrees(), d.hypot());
        // The matrix is relative to the origin of a canvas whose size the specification doesn't
        // give (§3.5.4), so it stays the identity: an elliptical radial is written circular.
        if g.gradient.kind == GradientKind::Radial && (geom.aspect - 1.0).abs() > 1e-6 {
            self.warn(ELLIPTICAL);
        }
        let mut s = String::from("Bb\n");
        if g.gradient.kind == GradientKind::Radial {
            // Every radial instance has a highlight (§3.5.5), `0 0 0 0 Bh` at the center (§3.5.6).
            // It lies within the gradient's circle: one past the radius is written on it.
            let off: Vec2 = geom.focal.map_or(Vec2::ZERO, |f| self.to_art * f - start);
            let fraction = if off.hypot() > 1e-9 && length > 1e-9 { off.hypot() / length } else { 0.0 };
            let (off, angle) = if fraction > 0.0 { (off / fraction.max(1.0), off.y.atan2(off.x).to_degrees()) } else { (Vec2::ZERO, 0.0) };
            ps::push_nums(&mut s, &[off.x, off.y, angle, fraction.min(1.0)]);
            s.push_str("Bh\n");
        }
        s.push_str("1 ");
        s.push_str(&latin_string(&name));
        s.push(' ');
        ps::push_nums(&mut s, &[start.x, start.y, angle, length]);
        s.push_str("1 0 0 1 0 0 Bg\n");
        s
    }

    /// The name of the definition of `g` (its swatch's name when it has one, with each character
    /// outside Windows-1252 written as `?` and a warning), defined the first time.
    fn define(&mut self, g: &Gradient, swatch: Option<&str>) -> String {
        if let Some((name, _)) = self.gradients.iter().find(|(_, d)| d == g) {
            return name.clone();
        }
        let taken = |n: &str, defs: &[(String, Gradient)]| defs.iter().any(|(m, _)| m == n);
        let base = self.latin_name(swatch.filter(|s| !s.is_empty()).unwrap_or("Gradient"), TITLE_AND_COLOR_NAMES);
        let mut name = base.clone();
        let mut i = 1;
        while taken(&name, &self.gradients) {
            i += 1;
            name = format!("{base} {i}");
        }
        self.gradients.push((name.clone(), g.clone()));
        name
    }

    // ---------- type ----------

    /// Type: a text object when the format can hold it as text, else its outlines.
    fn text(&mut self, t: &TextObject) {
        if !invertible(t.xf) {
            return;
        }
        if !self.outline_text {
            if writable(t) {
                return self.text_object(t);
            }
            self.warn(TYPE);
        }
        self.outlines(t);
    }

    /// A text object (§15): `type To`, the text path (`a b c d tx ty startPt Tp`, an area's frame
    /// or the path type runs along, `TP`), then each run's style and characters, `TO`. The matrix
    /// takes text space (y up, the first baseline's start for point type) to art space; the frame
    /// and path are in art space, as the specification's examples have them (§15.5).
    fn text_object(&mut self, t: &TextObject) {
        if lost_attributes(t) {
            self.warn(TYPE_ATTRIBUTES);
        }
        // Area type that Shrink Text to Fit scales down is written in the styles it draws in.
        let fitted = matches!((&t.kind, t.area.fit), (TextKind::Area { .. }, vectorcraft_doc::AreaFit::ShrinkText { .. }))
            .then(|| vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t).fit_scale)
            .filter(|f| *f != 1.0)
            .map(|f| vectorcraft_text::scaled(t, f));
        let t = fitted.as_ref().unwrap_or(t);
        let m = self.to_art * t.xf * Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 0.0]);
        let (kind, path, start) = match &t.kind {
            TextKind::Point => (0, None, 0.0),
            TextKind::Area { frame } => (1, frame.subpaths.iter().find(|sp| !sp.anchors.is_empty()).cloned(), 0.0),
            TextKind::OnPath { path, .. } => {
                let sp = path.subpaths.iter().find(|sp| !sp.anchors.is_empty()).cloned();
                let fraction = t.kind.path_span().map_or(0.0, |(s, _)| s);
                let start = sp
                    .as_ref()
                    .map_or(0.0, |sp| segment_point(&PathData::new(vec![sp.clone()]).transformed(self.to_art * t.xf).to_bezpath(), fraction));
                (2, sp, start)
            }
        };
        let _ = writeln!(self.body, "{kind} To");
        let [a, b, c, d, e, f] = m.as_coeffs();
        ps::push_nums(&mut self.body, &[a, b, c, d, e, f, start]);
        self.line("Tp");
        if let Some(sp) = path {
            let placed = SubPath::new(sp.anchors.iter().map(|a| a.transform(t.xf)).collect(), sp.closed);
            self.subpath(&placed);
            self.line(if sp.closed { "n" } else { "N" });
        }
        self.line("TP");
        let mut para = 0usize;
        let mut para_start = true;
        for run in &t.runs {
            let text = if run.style.all_caps { run.text.to_uppercase() } else { run.text.clone() };
            let mut pieces = text.split('\n').peekable();
            while let Some(piece) = pieces.next() {
                let ends_paragraph = pieces.peek().is_some();
                if piece.is_empty() && !ends_paragraph {
                    continue;
                }
                if para_start {
                    self.paragraph(t.para_at(para));
                    para_start = false;
                }
                self.char_style(&run.style, t.para_at(para));
                let mut bytes: Vec<u8> = piece.chars().filter_map(latin::encode).collect();
                if ends_paragraph {
                    bytes.push(b'\r');
                    para += 1;
                    para_start = true;
                }
                self.body.push_str(&ps::bytes(&bytes));
                self.line(" Tx");
            }
        }
        self.line("TO");
    }

    /// A paragraph's attributes: alignment `Ta` and indents `Ti` (first line's, left, right).
    fn paragraph(&mut self, p: &ParaStyle) {
        let align = match p.justify {
            Justify::Auto if p.direction == Some(ParaDirection::RightToLeft) => 2,
            Justify::Auto | Justify::Left => 0,
            Justify::Center => 1,
            Justify::Right => 2,
            Justify::JustifyLeft | Justify::JustifyCenter | Justify::JustifyRight => 3,
            Justify::JustifyAll => 4,
        };
        let _ = writeln!(self.body, "{align} Ta");
        ps::push_nums(&mut self.body, &[p.first_line_indent, p.left_indent, p.right_indent]);
        self.line("Ti");
    }

    /// A run's character attributes and paint style: `Tr`, its fill and stroke, `Tf`, `Ts`, `Tz`,
    /// `Tt`, `TA`, and the leading with the space before its paragraph, `Tl`.
    fn char_style(&mut self, st: &CharStyle, p: &ParaStyle) {
        let fill = !st.fill.is_none();
        let stroke = st.has_stroke();
        let render = match (fill, stroke) {
            (true, false) => 0,
            (false, true) => 1,
            (true, true) => 2,
            (false, false) => 3,
        };
        let _ = writeln!(self.body, "{render} Tr");
        if fill {
            self.color(&st.fill, false);
            self.line(if st.overprint_fill { "1 O" } else { "0 O" });
        }
        if stroke {
            self.color(&st.stroke, true);
            self.line(if st.overprint_stroke { "1 R" } else { "0 R" });
            self.line_style(&st.stroke_layer());
        }
        let (name, truetype, ascent, descent) = font_of(st);
        if !self.fonts.iter().any(|(n, _)| *n == name) {
            self.fonts.push((name.clone(), truetype));
        }
        let size = finite(st.size, 12.0);
        let _ = write!(self.body, "/_{name} ");
        ps::push_nums(&mut self.body, &[size, ascent * size, descent * size]);
        self.line("Tf");
        ps::push_num(&mut self.body, finite(st.baseline_shift, 0.0));
        self.line(" Ts");
        ps::push_nums(&mut self.body, &[finite(st.h_scale, 100.0), finite(st.v_scale, 100.0)]);
        self.line("Tz");
        ps::push_num(&mut self.body, finite(st.tracking, 0.0));
        self.line(" Tt");
        self.line(if st.kerning.is_none() { "1 TA" } else { "0 TA" });
        ps::push_nums(&mut self.body, &[finite(st.effective_leading(), size * 1.2), finite(p.space_before, 0.0)]);
        self.line("Tl");
    }

    /// Type as its glyphs' outlines: a group of a compound path for each run, with the run's fill
    /// and stroke.
    fn outlines(&mut self, t: &TextObject) {
        let layout = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
        let scale = t.xf.determinant().abs().sqrt();
        self.line("u");
        for (i, run) in t.runs.iter().enumerate() {
            let mut bp = BezPath::new();
            // Ruby (furigana) glyphs in their base text's run's paint, as the canvas draws them.
            for g in layout.glyphs.iter().chain(&layout.ruby).filter(|g| g.run == i) {
                bp.extend(g.outline.iter());
            }
            if bp.elements().is_empty() {
                continue;
            }
            bp.apply_affine(t.xf);
            let path = PathData::from_bezpath(&bp);
            let fill =
                (!run.style.fill.is_none()).then(|| FillLayer { overprint: run.style.overprint_fill, ..FillLayer::new(run.style.fill.clone()) });
            let stroke = run.style.has_stroke().then(|| {
                let mut s = run.style.stroke_layer();
                s.width *= scale;
                s
            });
            let items: Vec<AppearanceItem> = fill.map(AppearanceItem::Fill).into_iter().chain(stroke.map(AppearanceItem::Stroke)).collect();
            let mut n = Node::path(vectorcraft_doc::NodeId(0), path.clone(), vectorcraft_doc::Appearance::default());
            n.appearance = vectorcraft_doc::Appearance { items, ..vectorcraft_doc::Appearance::default() }.into();
            self.shape(&n, &path, FillRule::NonZero);
        }
        // Colour bitmap glyphs (emoji): their images, which the runs' paint doesn't paint.
        for g in &layout.glyphs {
            if let Some((image, to_text)) = g.placed_image() {
                self.image(Some(&image.blob), image.width, image.height, t.xf * to_text);
            }
            // COLR glyphs: their layers, as art.
            if let Some(art) = g.placed_colour(t.xf) {
                self.node(&art, false);
            }
        }
        self.line("U");
    }

    // ---------- images ----------

    /// An image (§8): `%AI5_File:`, `%AI5_BeginRaster`, its matrix (an art-space matrix applied to
    /// the pixels with their rows running downwards, `(u, -v)`) as `[ a b c d tx ty ]` and its
    /// operands with `XI`, its RGB samples in hex on comment lines of [`HEX_LINE`] digits,
    /// `%AI5_EndRaster`, then `F`. §8.4's examples have no `F`; without it, an app reading the file
    /// can leave out an image in a clipping group and the art after it. No path is open there, so
    /// the `F` paints nothing. `blob` holds the pixels, placed as a grid of `width` × `height`
    /// by `xf`.
    fn image(&mut self, blob: Option<&ImageBlob>, width: u32, height: u32, xf: Affine) {
        let flip = Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 0.0]);
        let m = self.to_art * xf * flip;
        if !invertible(m) {
            return;
        }
        if blob.is_some_and(|b| b.bytes.is_empty()) {
            self.warn(LARGE_IMAGE);
            return;
        }
        let Some(img) = blob.and_then(|b| image::load_from_memory(&b.bytes).ok()) else {
            self.warn(UNDECODED);
            return;
        };
        let (w, h) = (img.width(), img.height());
        if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_IMAGE_PIXELS {
            self.warn(LARGE_IMAGE);
            return;
        }
        // The object places the image's own pixel grid (its width × height) wherever the file's is.
        let fit = Affine::scale_non_uniform(f64::from(width.max(1)) / f64::from(w), f64::from(height.max(1)) / f64::from(h));
        let m = self.to_art * xf * fit * flip;
        let rgba = img.to_rgba8();
        let mut hex = String::with_capacity(rgba.len() / 4 * 7);
        hex.push('%');
        let mut on_line = 0;
        let mut see_through = false;
        for px in rgba.pixels() {
            let [r, g, b, a] = px.0;
            if a < 255 {
                see_through = true;
            }
            // Over white.
            let over = |c: u8| ((u32::from(c) * u32::from(a) + 255 * (255 - u32::from(a)) + 127) / 255) as u8;
            for c in [over(r), over(g), over(b)] {
                let _ = write!(hex, "{c:02X}");
                on_line += 2;
                if on_line >= HEX_LINE {
                    hex.push_str("\n%");
                    on_line = 0;
                }
            }
        }
        if hex.ends_with('%') {
            hex.pop();
        } else {
            hex.push('\n');
        }
        if see_through {
            self.warn(IMAGE_ALPHA);
        }
        let [a, b, c, d, e, f] = m.as_coeffs();
        let s = &mut self.body;
        s.push_str("%AI5_File:\n%AI5_BeginRaster\n[ ");
        ps::push_nums(s, &[a, b, c, d, e, f]);
        let _ = writeln!(s, "] 0 0 {w} {h} {w} {h} 8 3 0 0 0 0 XI");
        s.push_str(&hex);
        s.push_str("%AI5_EndRaster\nF\n");
    }
}
