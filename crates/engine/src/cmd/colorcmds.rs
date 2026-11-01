//! Edit → Edit Colors: invert, convert to CMYK/Grayscale/RGB, saturate, adjust colour balance and the
//! three Blend commands. They recolour the selected objects and everything inside them through the
//! shared colour visitor [`Recolor`]: fills and strokes (solid colours, gradient stops and text
//! runs), gradient meshes, embedded images and pattern fills. Colours linked to a global swatch are
//! unlinked when they change; Adjust Color Balance's Global mode shifts their tints instead.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::cms::Model;
use vectorcraft_color::harmony::{Guide, GuideOptions, Harmony, Variation};
use vectorcraft_color::{Color, Paint, keep_model};
use vectorcraft_doc::live::lerp_color;
use vectorcraft_doc::pattern::PatternDef;
pub(crate) use vectorcraft_doc::recolor::map_paint;
use vectorcraft_doc::recolor::{ColorVisitor, Reach, map_links, recolor_node};
use vectorcraft_doc::{Document, ImageBlob, Node, NodeId, NodeKind};

use super::edit::selected_roots;
use super::{
    CommandSpec, EngineError, Result, Session, always, bad, bool_or, check_numbers, cmd, color_value, f64_or, f64_req, has_selection, ids_param,
    str_param, unique_name,
};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "edit.colors.invert",
            "Invert Colors",
            ["Edit", "Edit Colors"],
            None,
            "{fill?: true, stroke?: true, includeImages?: true, includePatterns?: true, ids?} replace each colour by its RGB inverse (keeps the colour model) in fills, strokes, text, gradient meshes (with fills), colorized image fills (their pixels stay unchanged), other embedded images (a recoloured copy) and pattern tiles (a new pattern swatch; the original stays), as one undo step → {changed}",
            has_selection,
            |s, p| recolor(s, p, "Invert Colors", &invert)
        ),
        cmd!(
            "edit.colors.toCMYK",
            "Convert to CMYK",
            ["Edit", "Edit Colors"],
            None,
            "{fill?, stroke?, includeImages?, includePatterns?, ids?} (as edit.colors.invert) → {changed}",
            has_selection,
            |s, p| recolor(s, p, "Convert to CMYK", &to_cmyk)
        ),
        cmd!(
            "edit.colors.toGrayscale",
            "Convert to Grayscale",
            ["Edit", "Edit Colors"],
            None,
            "{fill?, stroke?, includeImages?, includePatterns?, ids?} (as edit.colors.invert) → {changed}",
            has_selection,
            |s, p| recolor(s, p, "Convert to Grayscale", &to_gray)
        ),
        cmd!(
            "edit.colors.toRGB",
            "Convert to RGB",
            ["Edit", "Edit Colors"],
            None,
            "{fill?, stroke?, includeImages?, includePatterns?, ids?} (as edit.colors.invert) → {changed}",
            has_selection,
            |s, p| {
                recolor(s, p, "Convert to RGB", &|c| {
                    let [r, g, b] = c.to_rgb();
                    Color::rgb(r, g, b)
                })
            }
        ),
        cmd!(
            "edit.colors.saturate",
            "Saturate…",
            ["Edit", "Edit Colors"],
            None,
            "{intensity: -100..100 (%; a number outside the range counts as its nearest end), fill?, stroke?, includeImages?, includePatterns?, ids?} scale saturation by (1 + intensity/100), reaching what edit.colors.invert does → {changed}",
            has_selection,
            saturate
        ),
        cmd!(
            "edit.colors.adjustBalance",
            "Adjust Color Balance…",
            ["Edit", "Edit Colors"],
            None,
            "{mode?: \"rgb\"|\"cmyk\"|\"gray\"|\"global\" (default: from the channels given), r?, g?, b? | c?, m?, y?, k? | gray? | tint?: -100..100 (% added per channel, default 0; `tint` is global mode: it shifts the tints of colours and gradient stops linked to global and spot swatches, which stay linked, and leaves other colours and images alone), convert?: false (true: the results stay in the adjusted model; false: each colour keeps its own), fill?, stroke?, includeImages?, includePatterns?, ids?} reaching what edit.colors.invert does → {changed}",
            has_selection,
            adjust_balance
        ),
        cmd!(
            "edit.colors.blendFrontToBack",
            "Blend Front to Back",
            ["Edit", "Edit Colors"],
            None,
            "{} ≥3 filled objects (solid fills or gradient meshes, which keep their shading): intermediate fills graded between the frontmost and backmost fill → {changed}",
            has_selection,
            |s, _| blend(s, BlendOrder::Stack)
        ),
        cmd!(
            "edit.colors.blendHorizontally",
            "Blend Horizontally",
            ["Edit", "Edit Colors"],
            None,
            "{} ≥3 filled objects or meshes: fills graded between the leftmost and rightmost → {changed}",
            has_selection,
            |s, _| blend(s, BlendOrder::Horizontal)
        ),
        cmd!(
            "edit.colors.blendVertically",
            "Blend Vertically",
            ["Edit", "Edit Colors"],
            None,
            "{} ≥3 filled objects or meshes: fills graded between the topmost and bottommost → {changed}",
            has_selection,
            |s, _| blend(s, BlendOrder::Vertical)
        ),
        cmd!(
            query "color.harmony",
            "Color Guide",
            [],
            None,
            "{color, rule: complementary|complementary2|splitComplementary|leftComplement|rightComplement|analogous|analogous2|monochromatic|shades|triad|triad2|triad3|tetrad|tetrad2|tetrad3|compound|compound2|highContrast|highContrast2|highContrast3|pentagram (or its label), steps?: 1..20 (4), variation?: \"tintsShades\"|\"warmCool\"|\"vividMuted\", amount?: 0..100 (50; how far the outermost steps go), limitTo?: swatch library id or name; or \"document\" (the document's swatches): every colour snaps to its nearest colour there (ΔE 2000)} the Color Guide for a base colour → {rule, colors: [\"#rrggbb\", the base first], grid: [per colour, 2·steps+1 variations from shades/cool/muted to tints/warm/vivid with the colour itself in the centre]}",
            always,
            harmony
        ),
    ]
}

/// The RGB inverse of `c`, expressed in `c`'s colour model.
pub(crate) fn invert(c: Color) -> Color {
    c.invert_keep_model()
}

pub(crate) fn to_cmyk(c: Color) -> Color {
    c.in_model(Model::Cmyk)
}

pub(crate) fn to_gray(c: Color) -> Color {
    c.in_model(Model::Gray)
}

/// Grayscale ink amount (0 = white, 1 = black) of a colour.
fn gray_level(c: Color) -> f32 {
    match to_gray(c) {
        Color::Gray { k } => k,
        // `in_model(Gray)` always yields Gray.
        _ => 0.0,
    }
}

/// What a recolouring pass applies to a colour that can link to a swatch (a solid colour or a
/// gradient stop): `(colour, link, tint)`, true when it changed them.
pub(crate) type LinkMap<'a> = dyn Fn(&mut Color, &mut Option<String>, &mut f32) -> bool + 'a;

/// The identity colour map (a pass that only maps linked colours).
fn same(c: Color) -> Color {
    c
}

/// What a recolouring pass covers: fills (with the points of gradient meshes), strokes, the pixels
/// of embedded images and the art of patterns (params `fill`, `stroke`, `includeImages`,
/// `includePatterns`; all on by default).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Scope {
    pub fill: bool,
    pub stroke: bool,
    pub images: bool,
    pub patterns: bool,
}

impl Scope {
    pub(crate) fn of(p: &Value) -> Self {
        Self {
            fill: bool_or(p, "fill", true),
            stroke: bool_or(p, "stroke", true),
            images: bool_or(p, "includeImages", true),
            patterns: bool_or(p, "includePatterns", true),
        }
    }
}

/// One recolouring pass, the colour visitor Edit Colors and Recolor Artwork share: `f` maps the
/// solid colours and gradient stops of fills, strokes and text runs, gradient-mesh points, the
/// pixels of embedded images without a colorize paint and the art of patterns used as fills or
/// strokes. A recoloured image becomes a new image and a recoloured pattern a new pattern swatch
/// ("Dots 2"); the originals stay as they are for other objects. Colours that change lose their
/// swatch link. Only nodes that change are kept as copies.
pub(crate) struct Recolor<'a> {
    f: &'a dyn Fn(Color) -> Color,
    /// Maps solid colours and gradient stops with their link and tint in place of `f`.
    links: Option<&'a LinkMap<'a>>,
    scope: Scope,
    /// Image key → the key of its recoloured copy (`None`: unchanged).
    images: HashMap<String, Option<String>>,
    /// Pattern name → the name of its recoloured copy (`None`: unchanged, or being recoloured).
    patterns: HashMap<String, Option<String>>,
    new_images: Vec<(String, ImageBlob)>,
    new_patterns: Vec<PatternDef>,
    changed: usize,
}

impl<'a> Recolor<'a> {
    pub(crate) fn new(scope: Scope, f: &'a dyn Fn(Color) -> Color) -> Self {
        Self { f, links: None, scope, images: HashMap::new(), patterns: HashMap::new(), new_images: vec![], new_patterns: vec![], changed: 0 }
    }

    /// A pass that maps only solid colours and gradient stops, with their swatch link and tint
    /// (`links`); meshes and images keep their colours.
    pub(crate) fn links(scope: Scope, links: &'a LinkMap<'a>) -> Self {
        Self { links: Some(links), ..Self::new(Scope { images: false, ..scope }, &same) }
    }

    /// A pass that maps solid colours and gradient stops with their swatch link and tint
    /// (`links`), and meshes, images and patterns through `f`.
    pub(crate) fn with_links(scope: Scope, f: &'a dyn Fn(Color) -> Color, links: &'a LinkMap<'a>) -> Self {
        Self { links: Some(links), ..Self::new(scope, f) }
    }

    /// Recolour the objects `ids` and everything inside them (each once), add the new images and
    /// pattern swatches, and return the number of paints, meshes and images changed.
    pub(crate) fn run(mut self, d: &mut Document, ids: &[NodeId]) -> usize {
        let targets: HashSet<NodeId> = ids.iter().copied().collect();
        let layers: Vec<(usize, Node)> = d.layers.iter().enumerate().filter_map(|(i, l)| Some((i, self.find(d, l, &targets)?))).collect();
        for (i, l) in layers {
            d.layers[i] = Arc::new(l);
        }
        for (k, b) in self.new_images {
            d.images.entry(k).or_insert(b);
        }
        for mut def in self.new_patterns {
            def.art = def.art.iter().map(|n| Arc::new(d.reid(n))).collect();
            super::patterncmds::add_pattern(d, def);
        }
        self.changed
    }

    /// `n` with the targets inside it recoloured, or `None` when nothing changed.
    fn find(&mut self, d: &Document, n: &Arc<Node>, targets: &HashSet<NodeId>) -> Option<Node> {
        if targets.contains(&n.id) {
            return self.subtree(d, n);
        }
        let mut out: Option<Node> = None;
        for (i, c) in n.children()?.iter().enumerate() {
            if let Some(new) = self.find(d, c, targets) {
                out.get_or_insert_with(|| (**n).clone()).children_mut()?[i] = Arc::new(new);
            }
        }
        out
    }

    /// `n` recoloured (through the shared colour visitor), or `None` when nothing changed.
    fn subtree(&mut self, d: &Document, n: &Node) -> Option<Node> {
        let Scope { fill, stroke, images, .. } = self.scope;
        let mut m = n.clone();
        let changed = recolor_node(&mut m, Reach { fill, stroke, images }, &mut Visit { pass: self, d });
        self.changed += changed;
        (changed > 0).then_some(m)
    }

    /// Recolour one paint: true when it changed.
    fn paint(&mut self, d: &Document, p: &mut Paint) -> bool {
        match p {
            Paint::Pattern { pattern, .. } if self.scope.patterns => self.pattern(d, pattern).map(|new| *pattern = new).is_some(),
            _ => match self.links {
                Some(f) => map_links(p, f),
                None => map_paint(p, self.f),
            },
        }
    }

    /// The key of image `key` recoloured (one copy per pass), `None` when no pixel changes.
    fn image(&mut self, d: &Document, key: &str) -> Option<String> {
        if let Some(done) = self.images.get(key) {
            return done.clone();
        }
        let f = self.f;
        let new = d.images.get(key)?.map_rgb(|[r, g, b]| {
            let [r, g, b, _] = f(Color::rgb8(r, g, b)).to_rgba8(1.0);
            [r, g, b]
        });
        let new = new.map(|blob| {
            let k = blob.content_key();
            self.new_images.push((k.clone(), blob));
            k
        });
        self.images.insert(key.to_string(), new.clone());
        new
    }

    /// The name of pattern `name` recoloured (one copy per pass), `None` when its colours stay.
    fn pattern(&mut self, d: &Document, name: &str) -> Option<String> {
        if let Some(done) = self.patterns.get(name) {
            return done.clone();
        }
        // A pattern inside its own art is left as it is.
        self.patterns.insert(name.to_string(), None);
        let mut def = d.pattern(name)?.clone();
        // The whole tile is recoloured, whichever of the object's paints uses the pattern.
        let (scope, before) = (self.scope, self.changed);
        self.scope = Scope { fill: true, stroke: true, ..scope };
        for n in &mut def.art {
            if let Some(new) = self.subtree(d, n) {
                *n = Arc::new(new);
            }
        }
        let changed = self.changed > before;
        (self.scope, self.changed) = (scope, before);
        if !changed {
            return None;
        }
        def.name = unique_name(name, |n| d.swatch_name_taken(n) || d.pattern(n).is_some() || self.new_patterns.iter().any(|p| p.name == n));
        let new = def.name.clone();
        self.new_patterns.push(def);
        self.patterns.insert(name.to_string(), Some(new.clone()));
        Some(new)
    }
}

/// What a pass over document `d` does to each colour the shared visitor meets.
struct Visit<'p, 'a> {
    pass: &'p mut Recolor<'a>,
    d: &'p Document,
}

impl ColorVisitor for Visit<'_, '_> {
    fn paint(&mut self, p: &mut Paint) -> bool {
        self.pass.paint(self.d, p)
    }
    fn color(&mut self, c: Color) -> Color {
        (self.pass.f)(c)
    }
    fn image(&mut self, key: &str) -> Option<String> {
        self.pass.image(self.d, key)
    }
}

pub(crate) fn recolor(s: &mut Session, p: &Value, label: &str, f: &dyn Fn(Color) -> Color) -> Result<Value> {
    run_pass(s, p, label, Recolor::new(Scope::of(p), f))
}

/// [`recolor`] with solid colours and gradient stops mapped through `links`, with their swatch
/// link and tint.
pub(crate) fn recolor_links(s: &mut Session, p: &Value, label: &str, f: &dyn Fn(Color) -> Color, links: &LinkMap) -> Result<Value> {
    run_pass(s, p, label, Recolor::with_links(Scope::of(p), f, links))
}

/// Run `pass` on the `ids` param's objects (default: the selection) as one undo step.
fn run_pass(s: &mut Session, p: &Value, label: &str, pass: Recolor) -> Result<Value> {
    let ids = match ids_param(p, "ids") {
        Some(v) => v,
        None => selected_roots(s)?,
    };
    let colorized_ok = super::paint::check_colorized(&s.doc()?.doc).is_ok();
    let n = s.edit(label, |d, _| {
        let n = pass.run(d, &ids);
        super::paint::check_colorized_kept(colorized_ok, d).map_err(|e| bad(label, e))?;
        Ok(n)
    })?;
    Ok(json!({ "changed": n }))
}

fn saturate(s: &mut Session, p: &Value) -> Result<Value> {
    let i = (f64_req(p, "intensity", "edit.colors.saturate")?.clamp(-100.0, 100.0) / 100.0) as f32;
    recolor(s, p, "Saturate", &move |c| {
        let [h, sat, v] = c.to_hsb();
        let to = (sat * (1.0 + i)).clamp(0.0, 1.0);
        // A color whose saturation stays the same is returned as it is, so it keeps its swatch link.
        if sat <= 0.0 || to == sat {
            return c;
        }
        keep_model(c, Color::from_hsb(h, to, v))
    })
}

fn adjust_balance(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "edit.colors.adjustBalance";
    check_numbers(p, &["r", "g", "b", "c", "m", "y", "k", "gray", "tint"], C)?;
    let given = |keys: &[&str]| keys.iter().any(|k| p.get(*k).is_some());
    let mode = match str_param(p, "mode") {
        Some(m) => m.to_ascii_lowercase(),
        None if given(&["c", "m", "y", "k"]) => "cmyk".into(),
        None if given(&["gray"]) => "gray".into(),
        None if given(&["r", "g", "b"]) => "rgb".into(),
        None if given(&["tint"]) => "global".into(),
        None => return Err(bad(C, "give a mode or r/g/b, c/m/y/k, gray or tint adjustments")),
    };
    let g = |k: &str| (f64_or(p, k, 0.0).clamp(-100.0, 100.0) / 100.0) as f32;
    let cl = |v: f32| v.clamp(0.0, 1.0);
    let adjust: Box<dyn Fn(Color) -> Color> = match mode.as_str() {
        "rgb" => {
            let (dr, dg, db) = (g("r"), g("g"), g("b"));
            Box::new(move |c| {
                let [r, gg, b] = c.to_rgb();
                Color::rgb(cl(r + dr), cl(gg + dg), cl(b + db))
            })
        }
        "cmyk" => {
            let (dc, dm, dy, dk) = (g("c"), g("m"), g("y"), g("k"));
            Box::new(move |c| {
                let [cc, m, y, k] = c.to_cmyk();
                Color::cmyk(cl(cc + dc), cl(m + dm), cl(y + dy), cl(k + dk))
            })
        }
        "gray" => {
            let dgray = g("gray");
            Box::new(move |c| Color::gray(cl(gray_level(c) + dgray)))
        }
        "global" => {
            // Each linked colour moves to the shifted tint of its global swatch's current colour.
            let d = &s.doc()?.doc;
            let bases: HashMap<String, Color> = d.swatches_iter().filter_map(|w| Some((w.name.clone(), d.global_color(&w.name)?))).collect();
            let dt = g("tint");
            let shift = move |c: &mut Color, link: &mut Option<String>, tint: &mut f32| {
                let Some(base) = link.as_ref().and_then(|l| bases.get(l)) else { return false };
                let t = cl(*tint + dt);
                if t == *tint {
                    return false;
                }
                (*tint, *c) = (t, base.tinted(t));
                true
            };
            return run_pass(s, p, "Adjust Colors", Recolor::links(Scope::of(p), &shift));
        }
        other => return Err(bad(C, format!("unknown mode `{other}` (rgb|cmyk|gray|global)"))),
    };
    let convert = bool_or(p, "convert", false);
    recolor(s, p, "Adjust Colors", &move |c| {
        let out = adjust(c);
        if convert { out } else { keep_model(c, out) }
    })
}

enum BlendOrder {
    Stack,
    Horizontal,
    Vertical,
}

/// Leaf objects with a solid fill, and gradient meshes, among the selection (paint order).
fn filled_leaves(d: &Document, roots: &[NodeId]) -> Vec<NodeId> {
    fn visit(n: &Node, out: &mut Vec<NodeId>) {
        match &n.kind {
            NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => {
                for c in children {
                    visit(c, out);
                }
            }
            _ => {
                if fill_color(n).is_some() {
                    out.push(n.id);
                }
            }
        }
    }
    let mut out = vec![];
    for r in roots {
        if let Some(n) = d.node(*r) {
            visit(n, &mut out);
        }
    }
    out
}

/// The colour an object blends by: its solid fill, or the average of a mesh's points.
fn fill_color(n: &Node) -> Option<Color> {
    match &n.kind {
        NodeKind::Text(t) => t.first_style().fill.color(),
        NodeKind::Image(_) => n.extras().colorize.as_deref().and_then(Paint::color).or_else(|| n.appearance.fill_paint().color()),
        NodeKind::Mesh(m) => {
            let k = m.points.len() as f32;
            let sum = m.points.iter().map(|p| p.color.to_rgb()).reduce(|a, c| [a[0] + c[0], a[1] + c[1], a[2] + c[2]])?;
            Some(Color::rgb(sum[0] / k, sum[1] / k, sum[2] / k))
        }
        _ => n.appearance.fill_paint().color(),
    }
}

fn blend(s: &mut Session, order: BlendOrder) -> Result<Value> {
    let roots = selected_roots(s)?;
    let d = &s.doc()?.doc;
    let mut ids = filled_leaves(d, &roots);
    if ids.len() < 3 {
        return Err(EngineError::Other("Blend colors: select at least three objects with filled colours".into()));
    }
    let center = |id: &NodeId| d.node(*id).and_then(|n| n.geometric_bounds()).map(|b| b.center()).unwrap_or_default();
    match order {
        BlendOrder::Stack => {
            // Front to back: frontmost first.
            ids.reverse();
        }
        BlendOrder::Horizontal => ids.sort_by(|a, b| center(a).x.total_cmp(&center(b).x)),
        BlendOrder::Vertical => ids.sort_by(|a, b| center(a).y.total_cmp(&center(b).y)),
    }
    let first = ids.first().and_then(|id| d.node(*id)).and_then(fill_color).unwrap_or_default();
    let last = ids.last().and_then(|id| d.node(*id)).and_then(fill_color).unwrap_or_default();
    // Ends in one model blend in it; mixed ones give colours in the document's model.
    let model = if first.model() == last.model() { first.model() } else { d.color_mode.model() };
    let n = ids.len();
    let label = match order {
        BlendOrder::Stack => "Blend Front to Back",
        BlendOrder::Horizontal => "Blend Horizontally",
        BlendOrder::Vertical => "Blend Vertically",
    };
    let fills = Scope { fill: true, stroke: false, images: false, patterns: false };
    let colorized_ok = super::paint::check_colorized(&s.doc()?.doc).is_ok();
    let changed = s.edit(label, |d, _| {
        let mut changed = 0;
        for (i, id) in ids.iter().enumerate().take(n - 1).skip(1) {
            let c = lerp_color(&first, &last, i as f32 / (n - 1) as f32).in_model(model);
            // A mesh moves its average to the blended colour and keeps its shading.
            let f: Box<dyn Fn(Color) -> Color> = match d.node(*id).map(|n| (&n.kind, fill_color(n))) {
                Some((NodeKind::Mesh(_), Some(mean))) => {
                    let ([cr, cg, cb], [mr, mg, mb]) = (c.to_rgb(), mean.to_rgb());
                    Box::new(move |p| {
                        let [r, g, b] = p.to_rgb();
                        keep_model(p, Color::rgb((r + cr - mr).clamp(0.0, 1.0), (g + cg - mg).clamp(0.0, 1.0), (b + cb - mb).clamp(0.0, 1.0)))
                    })
                }
                _ => Box::new(move |_| c),
            };
            changed += Recolor::new(fills, &*f).run(d, &[*id]);
        }
        super::paint::check_colorized_kept(colorized_ok, d).map_err(|e| bad(label, e))?;
        Ok(changed)
    })?;
    Ok(json!({ "changed": changed }))
}

fn harmony(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "color.harmony";
    let base = p.get("color").and_then(color_value).ok_or_else(|| bad(C, "missing or invalid `color`"))?;
    let rule = str_param(p, "rule").ok_or_else(|| bad(C, "missing `rule`"))?;
    let rule = Harmony::parse(rule).ok_or_else(|| {
        let ids: Vec<String> = Harmony::ALL.iter().map(|h| h.id()).collect();
        bad(C, format!("unknown rule `{rule}` ({})", ids.join("|")))
    })?;
    check_numbers(p, &["steps", "amount"], C)?;
    let mut opts = GuideOptions::default();
    if let Some(n) = p.get("steps").and_then(Value::as_u64) {
        opts.steps = n.min(u32::MAX as u64) as u32;
    }
    if let Some(v) = str_param(p, "variation") {
        opts.variation = Variation::parse(v).ok_or_else(|| bad(C, format!("unknown variation `{v}` (tintsShades|warmCool|vividMuted)")))?;
    }
    if let Some(a) = p.get("amount").and_then(Value::as_f64) {
        opts.amount = a as f32;
    }
    let mut g = Guide::new(base, rule, &opts);
    if let Some(palette) = super::swatchlib::limit_param(s, p, C)? {
        g = g.limited(&palette);
    }
    let hex = |cs: &[Color]| cs.iter().map(Color::to_hex).collect::<Vec<_>>();
    Ok(json!({ "rule": rule.id(), "colors": hex(&g.colors), "grid": g.grid.iter().map(|r| hex(r)).collect::<Vec<_>>() }))
}
