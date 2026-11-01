use super::apply::{kept_work, opaque, plan, stroked};
use super::options::FlattenOptions;
use super::{
    Appearance, Color, Deserialize, Document, Node, NodeId, NodeKind, Paint, PathData, Rect, Result, Serialize, Session, Value, bad, isolated_doc,
    json, roots_of, shape_node, str_param, targets,
};

/// The top-level objects among `ids` (or the selection), in paint order: layers stand for their
/// contents; hidden and locked objects and clipping paths are left alone.
pub(super) fn target_roots(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    let d = &s.doc()?.doc;
    let mut ids = vec![];
    for id in targets(s, p)? {
        match d.node(id) {
            Some(n) if n.is_layer() => ids.extend(n.children().into_iter().flatten().map(|c| c.id)),
            Some(_) => ids.push(id),
            None => {}
        }
    }
    let mut keyed: Vec<(Vec<usize>, NodeId)> = ids.into_iter().filter_map(|id| d.index_path(id).map(|p| (p, id))).collect();
    keyed.sort();
    keyed.dedup();
    let roots = roots_of(d, keyed.into_iter().map(|(_, id)| id).collect());
    // A clipping path stays where it clips.
    let clipping = |id: NodeId| {
        d.parent_of(id).and_then(|p| d.node(p)).is_some_and(|p| p.clips() && p.children().and_then(|c| c.first()).is_some_and(|c| c.id == id))
    };
    let editable = d.editable();
    Ok(roots.into_iter().filter(|id| d.is_visible(*id) && editable(*id) && !clipping(*id)).collect())
}

// ---------- preview ----------

/// What the Flattener Preview highlights.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Highlight {
    /// Nothing: the art in colour.
    #[default]
    None,
    /// Areas the raster/vector balance rasterizes whole.
    RasterizedRegions,
    /// Objects with transparency of their own.
    TransparentObjects,
    /// Every object flattening changes.
    AllAffected,
    /// Objects with pattern paint in the flattened art.
    ExpandedPatterns,
    /// Strokes that become filled outlines.
    OutlinedStrokes,
    /// Type that becomes outlines.
    OutlinedText,
    /// Every area that becomes an image.
    AllRasterized,
}

impl Highlight {
    pub const ALL: [Self; 8] = [
        Self::None,
        Self::RasterizedRegions,
        Self::TransparentObjects,
        Self::AllAffected,
        Self::ExpandedPatterns,
        Self::OutlinedStrokes,
        Self::OutlinedText,
        Self::AllRasterized,
    ];

    /// The id `flattener.preview` takes.
    pub fn id(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::RasterizedRegions => "rasterizedRegions",
            Self::TransparentObjects => "transparentObjects",
            Self::AllAffected => "allAffected",
            Self::ExpandedPatterns => "expandedPatterns",
            Self::OutlinedStrokes => "outlinedStrokes",
            Self::OutlinedText => "outlinedText",
            Self::AllRasterized => "allRasterized",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None (Color Preview)",
            Self::RasterizedRegions => "Rasterized Complex Regions",
            Self::TransparentObjects => "Transparent Objects",
            Self::AllAffected => "All Affected Objects",
            Self::ExpandedPatterns => "Expanded Patterns",
            Self::OutlinedStrokes => "Outlined Strokes",
            Self::OutlinedText => "Outlined Text",
            Self::AllRasterized => "All Rasterized Regions",
        }
    }

    /// A highlight by id or label, any case.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        Self::ALL.into_iter().find(|h| h.id().eq_ignore_ascii_case(s) || h.label().eq_ignore_ascii_case(s))
    }
}

/// How the Flattener Preview treats overprints: kept as they are, simulated in the preview, or
/// dropped; only Preserve flattens with Preserve Overprints and Spot Colors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Overprints {
    #[default]
    Preserve,
    Simulate,
    Discard,
}

impl Overprints {
    pub const ALL: [Self; 3] = [Self::Preserve, Self::Simulate, Self::Discard];

    pub fn id(self) -> &'static str {
        match self {
            Self::Preserve => "preserve",
            Self::Simulate => "simulate",
            Self::Discard => "discard",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Preserve => "Preserve",
            Self::Simulate => "Simulate",
            Self::Discard => "Discard",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|o| o.id().eq_ignore_ascii_case(s.trim()) || o.label().eq_ignore_ascii_case(s.trim()))
    }
}

/// What flattening would do (Window → Flattener Preview, `flattener.preview`).
#[derive(Clone, Debug, Default)]
pub struct FlattenReport {
    /// Top-level objects with transparency of their own.
    pub transparent: Vec<NodeId>,
    /// Top-level objects flattening changes: the overlapping groups with transparency, and those
    /// the outline and overprint options change.
    pub affected: Vec<NodeId>,
    /// Objects with pattern paint in the flattened art.
    pub patterns: Vec<NodeId>,
    /// Paths whose strokes become filled outlines.
    pub strokes: Vec<NodeId>,
    /// Type that becomes outlines.
    pub text: Vec<NodeId>,
    /// Areas the raster/vector balance rasterizes whole.
    pub complex: Vec<PathData>,
    /// Every area that becomes an image.
    pub rasterized: Vec<PathData>,
    /// Flat-colour regions made.
    pub vector: usize,
}

impl FlattenReport {
    /// The objects `h` highlights.
    pub fn objects(&self, h: Highlight) -> &[NodeId] {
        match h {
            Highlight::TransparentObjects => &self.transparent,
            Highlight::AllAffected => &self.affected,
            Highlight::ExpandedPatterns => &self.patterns,
            Highlight::OutlinedStrokes => &self.strokes,
            Highlight::OutlinedText => &self.text,
            _ => &[],
        }
    }

    /// The areas `h` highlights.
    pub fn areas(&self, h: Highlight) -> &[PathData] {
        match h {
            Highlight::RasterizedRegions => &self.complex,
            Highlight::AllRasterized => &self.rasterized,
            _ => &[],
        }
    }

    /// How many objects or areas each highlight shows.
    pub fn counts(&self) -> Value {
        json!({
            "transparentObjects": self.transparent.len(),
            "allAffected": self.affected.len(),
            "expandedPatterns": self.patterns.len(),
            "outlinedStrokes": self.strokes.len(),
            "outlinedText": self.text.len(),
            "rasterizedRegions": self.complex.len(),
            "allRasterized": self.rasterized.len(),
            "vectorRegions": self.vector,
        })
    }

    /// The report as `flattener.preview` answers: the highlighted objects (or areas) of `doc` and
    /// the counts.
    pub fn to_json(&self, doc: &Document, h: Highlight) -> Value {
        let rect = |r: Rect| json!({"x": r.x0, "y": r.y0, "width": r.width(), "height": r.height()});
        let objects = self.objects(h).iter().filter_map(|id| Some(json!({"id": id.0, "bounds": rect(doc.node(*id)?.visual_bounds()?)})));
        let areas = self.areas(h).iter().filter_map(|p| Some(json!({ "bounds": rect(p.bounds()?) })));
        json!({"highlight": h.id(), "regions": objects.chain(areas).collect::<Vec<_>>(), "counts": self.counts()})
    }

    /// The art `h` highlights, opaque, alone in a copy of `doc`: the objects and the areas (filled
    /// black). Its coverage is the highlight. `None` when nothing is highlighted.
    pub fn highlight_art(&self, doc: &Document, h: Highlight) -> Option<Document> {
        let (ids, areas) = (self.objects(h), self.areas(h));
        if ids.is_empty() && areas.is_empty() {
            return None;
        }
        let mut tmp = doc.clone();
        let mut art: Vec<Node> = ids.iter().filter_map(|id| doc.node(*id).cloned()).collect();
        art.iter_mut().for_each(opaque);
        for path in areas {
            let mut n = shape_node(&mut tmp, path.clone(), None);
            n.appearance = Appearance::basic(Paint::solid(Color::rgb(0.0, 0.0, 0.0)), Paint::None, 0.0).into();
            art.push(n);
        }
        Some(isolated_doc(&tmp, art))
    }
}

/// What flattening `roots` of `doc` with `o` would do.
pub fn report(doc: &Document, roots: &[NodeId], o: &FlattenOptions) -> Result<FlattenReport> {
    let plan = plan(doc, roots, o, false)?;
    let mut r = FlattenReport { transparent: plan.transparent, ..Default::default() };
    // Every object under `id` (but a compound path's own pieces), visible ones only.
    fn visit(n: &Node, f: &mut dyn FnMut(&Node)) {
        if !n.visible {
            return;
        }
        f(n);
        if !matches!(n.kind, NodeKind::Compound { .. }) {
            for c in n.children().into_iter().flatten() {
                visit(c, f);
            }
        }
    }
    let pattern = |n: &Node| n.appearance.items.iter().any(|i| i.visible() && matches!(i.paint(), Paint::Pattern { .. }));
    for f in &plan.flat {
        r.affected.extend(&f.roots);
        r.vector += f.regions.len();
        for root in f.roots.iter().filter_map(|id| doc.node(*id)) {
            visit(root, &mut |n| {
                // Flattened art is outlined throughout and its patterns become part of the image.
                if matches!(n.kind, NodeKind::Text(_)) {
                    r.text.push(n.id);
                } else if stroked(n) {
                    r.strokes.push(n.id);
                }
                if pattern(n) {
                    r.patterns.push(n.id);
                }
            });
        }
        if let Some(a) = &f.area {
            if a.whole {
                r.complex.push(a.outline());
            }
            r.rasterized.push(a.outline());
        }
    }
    for root in plan.kept.iter().filter_map(|id| doc.node(*id)).filter(|n| kept_work(n, o)) {
        r.affected.push(root.id);
        visit(root, &mut |n| {
            if o.text_to_outlines && matches!(n.kind, NodeKind::Text(_)) {
                r.text.push(n.id);
            } else if o.strokes_to_outlines && stroked(n) {
                r.strokes.push(n.id);
            }
        });
    }
    r.affected.sort_by_key(|id| doc.index_path(*id));
    Ok(r)
}

impl Session {
    /// What flattening would do with the options `p` asks for (as `flattener.preview`): the
    /// options and the report.
    pub fn flattener_report(&self, p: &Value) -> Result<(FlattenOptions, FlattenReport)> {
        const C: &str = "flattener.preview";
        let mut o = self.flatten_options(p).map_err(|m| bad(C, m))?;
        if let Some(v) = str_param(p, "overprints") {
            let ov = Overprints::parse(v).ok_or_else(|| bad(C, format!("unknown overprints `{v}` (preserve, simulate or discard)")))?;
            o.preserve_overprints = ov == Overprints::Preserve;
        }
        let roots = if p.get("ids").is_some() {
            target_roots(self, p)?
        } else {
            let layers: Vec<u64> = self.doc()?.doc.layers.iter().map(|l| l.id.0).collect();
            target_roots(self, &json!({ "ids": layers }))?
        };
        let r = report(&self.doc()?.doc, &roots, &o)?;
        Ok((o, r))
    }
}

pub(super) fn preview(s: &mut Session, p: &Value) -> Result<Value> {
    let h = match str_param(p, "highlight") {
        Some(v) => Highlight::parse(v).ok_or_else(|| bad("flattener.preview", format!("unknown highlight `{v}`")))?,
        None => Highlight::None,
    };
    let (o, r) = s.flattener_report(p)?;
    let mut out = r.to_json(&s.doc()?.doc, h);
    out["options"] = serde_json::to_value(&o).unwrap_or_default();
    Ok(out)
}
