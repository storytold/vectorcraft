//! Edit → Edit Colors: invert, convert to CMYK/Grayscale/RGB, saturate, adjust colour balance and the
//! three Blend commands. They recolour the selected objects and everything inside them through the
//! shared colour visitor [`Recolor`]: fills and strokes (solid colours, gradient stops and text
//! runs), gradient meshes, embedded images and pattern fills. Colours linked to a global swatch are
//! unlinked when they change.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::cms::Model;
use vectorcraft_color::harmony::{Guide, GuideOptions, Harmony, Variation};
use vectorcraft_color::{Color, Paint, keep_model};
use vectorcraft_doc::live::lerp_color;
use vectorcraft_doc::pattern::PatternDef;
use vectorcraft_doc::{AppearanceItem, Document, ImageBlob, Node, NodeId, NodeKind};

use super::edit::selected_roots;
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "edit.colors.invert",
            "Invert Colors",
            ["Edit", "Edit Colors"],
            None,
            "{fill?: true, stroke?: true, includeImages?: true, includePatterns?: true, ids?} replace each colour by its RGB inverse (keeps the colour model) in fills, strokes, text, gradient meshes (with fills), embedded images (a recoloured copy) and pattern tiles (a new pattern swatch; the original stays), as one undo step → {changed}",
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
            "{intensity: -100..100 (%), fill?, stroke?, includeImages?, includePatterns?, ids?} scale saturation by (1 + intensity/100), reaching what edit.colors.invert does → {changed}",
            has_selection,
            saturate
        ),
        cmd!(
            "edit.colors.adjustBalance",
            "Adjust Color Balance…",
            ["Edit", "Edit Colors"],
            None,
            "{mode?: \"rgb\"|\"cmyk\"|\"gray\"|\"global\" (default: from the channels given), r?, g?, b? | c?, m?, y?, k? | gray?: -100..100 (% added per channel, default 0), convert?: false (true: the results stay in the adjusted model; false: each colour keeps its own), fill?, stroke?, includeImages?, includePatterns?, ids?} reaching what edit.colors.invert does → {changed}. Global mode (tints of global and spot colours) isn't available yet",
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
            "{color, rule: complementary|complementary2|splitComplementary|leftComplement|rightComplement|analogous|analogous2|monochromatic|shades|triad|triad2|triad3|tetrad|tetrad2|tetrad3|compound|compound2|highContrast|highContrast2|highContrast3|pentagram (or its label), steps?: 1..20 (4), variation?: \"tintsShades\"|\"warmCool\"|\"vividMuted\", amount?: 0..100 (50; how far the outermost steps go)} the Color Guide for a base colour → {rule, colors: [\"#rrggbb\", the base first], grid: [per colour, 2·steps+1 variations from shades/cool/muted to tints/warm/vivid with the colour itself in the centre]}",
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

/// Apply `f` to a paint; returns whether anything changed.
pub(crate) fn map_paint(p: &mut Paint, f: &dyn Fn(Color) -> Color) -> bool {
    match p {
        Paint::Solid { color, swatch } => {
            let n = f(*color);
            if n != *color {
                *color = n;
                *swatch = None;
                return true;
            }
            false
        }
        Paint::Gradient(g) => {
            let mut ch = false;
            for st in &mut g.gradient.stops {
                let n = f(st.color);
                if n != st.color {
                    st.color = n;
                    ch = true;
                }
            }
            if ch {
                g.swatch = None;
            }
            ch
        }
        _ => false,
    }
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
/// pixels of embedded images and the art of patterns used as fills or strokes. A recoloured image
/// becomes a new image and a recoloured pattern a new pattern swatch ("Dots 2"); the originals stay
/// as they are for other objects. Colours that change lose their swatch link. Only nodes that
/// change are kept as copies.
pub(crate) struct Recolor<'a> {
    f: &'a dyn Fn(Color) -> Color,
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
        Self { f, scope, images: HashMap::new(), patterns: HashMap::new(), new_images: vec![], new_patterns: vec![], changed: 0 }
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

    /// `n` recoloured, or `None` when nothing changed.
    fn subtree(&mut self, d: &Document, n: &Node) -> Option<Node> {
        let before = self.changed;
        let mut m = n.clone();
        self.node(d, &mut m);
        (self.changed > before).then_some(m)
    }

    fn node(&mut self, d: &Document, n: &mut Node) {
        let Scope { fill, stroke, images, .. } = self.scope;
        for it in &mut n.appearance.items {
            match it {
                AppearanceItem::Fill(l) if fill => self.paint(d, &mut l.paint),
                AppearanceItem::Stroke(l) if stroke => self.paint(d, &mut l.paint),
                _ => {}
            }
        }
        match &mut n.kind {
            NodeKind::Text(t) => {
                for r in &mut t.runs {
                    if fill {
                        self.paint(d, &mut r.style.fill);
                    }
                    if stroke {
                        self.paint(d, &mut r.style.stroke);
                    }
                }
            }
            NodeKind::Mesh(m) if fill => {
                let mut ch = false;
                for p in &mut m.points {
                    let c = (self.f)(p.color);
                    ch |= c != p.color;
                    p.color = c;
                }
                self.changed += ch as usize;
            }
            NodeKind::Image(im) if images && im.link.is_none() => {
                if let Some(k) = self.image(d, &im.key) {
                    im.key = k;
                    self.changed += 1;
                }
            }
            _ => {
                for c in n.children_mut().into_iter().flatten() {
                    if let Some(new) = self.subtree(d, c) {
                        *c = Arc::new(new);
                    }
                }
            }
        }
    }

    fn paint(&mut self, d: &Document, p: &mut Paint) {
        let changed = match p {
            Paint::Pattern { pattern, .. } if self.scope.patterns => self.pattern(d, pattern).map(|new| *pattern = new).is_some(),
            _ => map_paint(p, self.f),
        };
        self.changed += changed as usize;
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

pub(crate) fn recolor(s: &mut Session, p: &Value, label: &str, f: &dyn Fn(Color) -> Color) -> Result<Value> {
    let ids = match ids_param(p, "ids") {
        Some(v) => v,
        None => selected_roots(s)?,
    };
    let scope = Scope::of(p);
    let n = s.edit(label, |d, _| Ok(Recolor::new(scope, f).run(d, &ids)))?;
    Ok(json!({ "changed": n }))
}

fn saturate(s: &mut Session, p: &Value) -> Result<Value> {
    let i = (f64_or(p, "intensity", 0.0).clamp(-100.0, 100.0) / 100.0) as f32;
    recolor(s, p, "Saturate", &move |c| {
        let [h, sat, v] = c.to_hsb();
        if sat <= 0.0 {
            return c;
        }
        keep_model(c, Color::from_hsb(h, (sat * (1.0 + i)).clamp(0.0, 1.0), v))
    })
}

fn adjust_balance(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "edit.colors.adjustBalance";
    let given = |keys: &[&str]| keys.iter().any(|k| p.get(*k).is_some());
    let mode = match str_param(p, "mode") {
        Some(m) => m.to_ascii_lowercase(),
        None if given(&["c", "m", "y", "k"]) => "cmyk".into(),
        None if given(&["gray"]) => "gray".into(),
        None if given(&["r", "g", "b"]) => "rgb".into(),
        None => return Err(bad(C, "give a mode or r/g/b, c/m/y/k or gray adjustments")),
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
            Box::new(move |c| match to_gray(c) {
                Color::Gray { k } => Color::gray(cl(k + dgray)),
                other => other,
            })
        }
        "global" => {
            return Err(bad(C, "mode `global` shifts the tints of global and spot colours, which aren't supported yet: use rgb, cmyk or gray"));
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
    let (Some(&first_id), Some(&last_id)) = (ids.first(), ids.last()) else {
        return Err(EngineError::Other("Blend colors: select at least three objects with filled colours".into()));
    };
    let first = d.node(first_id).and_then(fill_color).unwrap_or_default();
    let last = d.node(last_id).and_then(fill_color).unwrap_or_default();
    // Ends in one model blend in it; mixed ones give colours in the document's model.
    let model = if first.model() == last.model() { first.model() } else { d.color_mode.model() };
    let n = ids.len();
    let label = match order {
        BlendOrder::Stack => "Blend Front to Back",
        BlendOrder::Horizontal => "Blend Horizontally",
        BlendOrder::Vertical => "Blend Vertically",
    };
    let fills = Scope { fill: true, stroke: false, images: false, patterns: false };
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
        Ok(changed)
    })?;
    Ok(json!({ "changed": changed }))
}

fn harmony(_: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "color.harmony";
    let base = p.get("color").and_then(color_value).ok_or_else(|| bad(C, "missing or invalid `color`"))?;
    let rule = str_param(p, "rule").ok_or_else(|| bad(C, "missing `rule`"))?;
    let rule = Harmony::parse(rule).ok_or_else(|| {
        let ids: Vec<String> = Harmony::ALL.iter().map(|h| h.id()).collect();
        bad(C, format!("unknown rule `{rule}` ({})", ids.join("|")))
    })?;
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
    let g = Guide::new(base, rule, &opts);
    let hex = |cs: &[Color]| cs.iter().map(Color::to_hex).collect::<Vec<_>>();
    Ok(json!({ "rule": rule.id(), "colors": hex(&g.colors), "grid": g.grid.iter().map(|r| hex(r)).collect::<Vec<_>>() }))
}
