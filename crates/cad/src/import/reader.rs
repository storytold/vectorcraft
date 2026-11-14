//! What the import uses of a drawing, from `craft_dxf::model`: header variables; the layer,
//! linetype and text style tables; the blocks; the entities; the layouts.

use std::collections::HashMap;

use craft_dxf::model::{self, EntityRecord};
use vectorcraft_geom::Point;

/// The first bytes of a binary DXF file (its format's signature).
pub const BINARY_SENTINEL: &[u8] = b"AutoCAD Binary DXF\r\n\x1a\x00";

/// The header variables the import reads.
#[derive(Clone, Debug, Default)]
pub(crate) struct Header {
    /// `$ACADVER` (`AC1032`…).
    pub acadver: String,
    /// `$INSUNITS`: the drawing unit (0: unitless).
    pub insunits: i64,
    /// `$MEASUREMENT`: 0 imperial, 1 metric.
    pub measurement: Option<i64>,
    /// `$LTSCALE`: the global linetype scale.
    pub ltscale: f64,
    /// `$LWDEFAULT`: the default lineweight (hundredths of a millimetre).
    pub lwdefault: i32,
}

/// A layer of the LAYER table.
#[derive(Clone, Debug)]
pub(crate) struct LayerDef {
    pub name: String,
    pub rgb: [u8; 3],
    /// Switched off (a negative colour) or frozen.
    pub hidden: bool,
    pub locked: bool,
    pub plot: bool,
    /// Its linetype (upper case).
    pub linetype: String,
    /// Hundredths of a millimetre; negative: the default.
    pub lineweight: i32,
}

/// A block definition.
#[derive(Clone, Debug)]
pub(crate) struct BlockDef<'a> {
    pub name: String,
    pub base: Point,
    pub records: Vec<EntityRecord<'a>>,
    /// A reference to another drawing (its art isn't in this file).
    pub xref: bool,
}

/// A layout: model space, or a paper layout with the block holding its art.
#[derive(Clone, Debug)]
pub(crate) struct LayoutDef {
    pub name: String,
    /// The block of a paper layout (upper case); `None` for model space.
    pub block: Option<String>,
}

/// The name of model space.
pub(crate) const MODEL: &str = "Model";
const MODEL_BLOCK: &str = "*MODEL_SPACE";
const PAPER_BLOCK: &str = "*PAPER_SPACE";

/// A drawing as the import reads it.
#[derive(Debug, Default)]
pub(crate) struct Drawing<'a> {
    pub header: Header,
    pub layers: Vec<LayerDef>,
    /// Index into `layers` by upper-case name.
    layer_index: HashMap<String, usize>,
    /// Linetype patterns by upper-case name: dash (positive), gap (negative), dot (0) lengths.
    pub linetypes: HashMap<String, Vec<f64>>,
    /// The font family of each text style (upper-case name; empty: a CAD font).
    pub styles: HashMap<String, String>,
    /// Blocks by upper-case name.
    pub blocks: HashMap<String, BlockDef<'a>>,
    /// The ENTITIES section.
    pub entities: Vec<EntityRecord<'a>>,
    /// Model space first, then the paper layouts in tab order.
    pub layouts: Vec<LayoutDef>,
    /// How many sections the file has (none: not a drawing).
    pub sections: usize,
}

/// The colour of index `i` on paper: index 7 (white on a CAD screen) is black.
pub(crate) fn aci_paper(i: u8) -> [u8; 3] {
    if i == 7 { [0; 3] } else { model::aci::rgb(i) }
}

impl<'a> Drawing<'a> {
    /// What the import uses of `doc`.
    pub fn read(doc: model::Document<'a>) -> Self {
        let h = &doc.header;
        let header = Header {
            acadver: h.acadver().unwrap_or_default(),
            insunits: h.insunits(),
            measurement: h.measurement(),
            ltscale: h.ltscale(),
            lwdefault: i32::from(h.lwdefault()),
        };
        let mut d = Drawing { header, sections: doc.sections, ..Drawing::default() };
        for l in doc.layers() {
            d.layer_index.entry(l.name.to_uppercase()).or_insert(d.layers.len());
            d.layers.push(layer(l));
        }
        for lt in doc.linetypes() {
            d.linetypes.insert(lt.name.to_uppercase(), lt.pattern.iter().map(|e| e.length).collect());
        }
        for st in doc.text_styles().into_iter().filter(|s| !s.name.is_empty()) {
            d.styles.insert(st.name.to_uppercase(), st.family().unwrap_or_default());
        }
        let block_names = doc.block_names();
        d.layouts = layouts(&doc, &block_names);
        for b in doc.blocks {
            if b.name.is_empty() {
                continue;
            }
            let def = BlockDef { name: b.name.to_string(), base: point(b.base()), xref: b.flags() & 4 != 0, records: b.entities };
            d.blocks.insert(b.name.to_uppercase(), def);
        }
        d.entities = doc.entities;
        d
    }

    /// The layout named `name` (any case; `model` is model space).
    pub fn layout(&self, name: &str) -> Option<&LayoutDef> {
        self.layouts.iter().find(|l| l.name.eq_ignore_ascii_case(name.trim()))
    }

    /// The entity records of `layout`.
    pub fn layout_records(&self, layout: &LayoutDef) -> Vec<EntityRecord<'a>> {
        let block = |name: &str| self.blocks.get(name).map(|b| b.records.clone()).unwrap_or_default();
        // The active paper layout keeps its art in the ENTITIES section, flagged paper space.
        let (block_name, paper) = match &layout.block {
            None => (MODEL_BLOCK, false),
            Some(b) if b == PAPER_BLOCK => (PAPER_BLOCK, true),
            Some(b) => return block(b),
        };
        let mut out: Vec<EntityRecord<'a>> = self.entities.iter().filter(|e| e.paper_space() == paper).cloned().collect();
        out.extend(block(block_name));
        out
    }

    /// The layer named `name` (any case).
    pub fn layer(&self, name: &str) -> Option<&LayerDef> {
        self.layer_index.get(&name.to_uppercase()).and_then(|i| self.layers.get(*i))
    }
}

pub(crate) fn point(p: model::Point) -> Point {
    Point::new(p[0], p[1])
}

fn layer(l: model::Layer) -> LayerDef {
    let rgb = match l.color {
        model::Color::Rgb(rgb) => rgb,
        model::Color::Index(i) => aci_paper(i),
        model::Color::ByLayer | model::Color::ByBlock => aci_paper(7),
    };
    LayerDef {
        // CAD apps never plot this layer of dimension points.
        plot: l.plot && !l.name.eq_ignore_ascii_case("Defpoints"),
        rgb,
        hidden: !l.on || l.frozen,
        locked: l.locked,
        linetype: l.linetype.to_uppercase(),
        lineweight: i32::try_from(l.lineweight.to_dxf()).unwrap_or(-3),
        name: l.name,
    }
}

/// The most bytes of layout and block names the layouts keep together. Each layout keeps its own
/// copy of its block's name, so many layouts naming one long block name multiplied it; real
/// drawings have a few dozen layouts with short names.
const MAX_LAYOUT_NAME_BYTES: usize = 4 << 20;

/// Model space, then each paper layout (its LAYOUT object, in tab order); a file without layout
/// objects but with paper-space art has one paper layout.
fn layouts(doc: &model::Document<'_>, block_names: &HashMap<u64, &str>) -> Vec<LayoutDef> {
    let mut paper: Vec<(i64, LayoutDef)> = vec![];
    let mut seen = std::collections::HashSet::new();
    let mut name_bytes = 0usize;
    for l in doc.layouts() {
        let block = l.block_record.and_then(|h| block_names.get(&h)).map(|n| n.to_uppercase());
        if l.name.eq_ignore_ascii_case(MODEL) || block.as_deref() == Some(MODEL_BLOCK) {
            continue;
        }
        let Some(block) = block else { continue };
        let key = l.name.to_ascii_lowercase();
        let cost = l.name.len() + block.len();
        // A layout past the allowance is skipped alone: a later short one is still offered, and a
        // skipped one reserves neither its name nor its bytes.
        if seen.contains(&key) || name_bytes.saturating_add(cost) > MAX_LAYOUT_NAME_BYTES {
            continue;
        }
        seen.insert(key);
        name_bytes += cost;
        paper.push((l.tab_order, LayoutDef { name: l.name, block: Some(block) }));
    }
    paper.sort_by_key(|(order, _)| *order);
    if paper.is_empty() && doc.entities.iter().any(EntityRecord::paper_space) {
        paper.push((1, LayoutDef { name: "Layout1".into(), block: Some(PAPER_BLOCK.into()) }));
    }
    std::iter::once(LayoutDef { name: MODEL.into(), block: None }).chain(paper.into_iter().map(|(_, l)| l)).collect()
}
