//! Reading an ASCII DXF file: its text, the group-code pairs, and what the import uses of its
//! sections (header variables; the layer, linetype, text style and block record tables; the
//! blocks; the entities; the layouts).

use std::borrow::Cow;
use std::collections::HashMap;

use vectorcraft_geom::Point;

/// The most group-code pairs a drawing may hold (about 300 MB of DXF).
const MAX_PAIRS: usize = 20_000_000;

/// The first bytes of a binary DXF file (its format's signature).
pub const BINARY_SENTINEL: &[u8] = b"AutoCAD Binary DXF\r\n\x1a\x00";

/// One group: its code and its value as written.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Pair<'a> {
    pub code: i32,
    pub value: &'a str,
}

/// Values of a run of pairs by group code.
pub(crate) trait Groups<'a> {
    /// The value of the first `code`, as written.
    fn raw(&self, code: i32) -> Option<&'a str>;
    /// The first `code` as a name (trimmed, never empty).
    fn name(&self, code: i32) -> Option<&'a str> {
        self.raw(code).map(str::trim).filter(|s| !s.is_empty())
    }
    /// The first `code` as a finite number.
    fn num(&self, code: i32) -> Option<f64> {
        self.raw(code).and_then(number)
    }
    fn num_or(&self, code: i32, default: f64) -> f64 {
        self.num(code).unwrap_or(default)
    }
    /// The first `code` as a whole number.
    fn int(&self, code: i32) -> Option<i64> {
        self.raw(code).and_then(integer)
    }
    fn int_or(&self, code: i32, default: i64) -> i64 {
        self.int(code).unwrap_or(default)
    }
    /// The point at `code` (x) and `code + 10` (y); 0 for what is missing.
    fn point(&self, code: i32) -> Point {
        Point::new(self.num_or(code, 0.0), self.num_or(code + 10, 0.0))
    }
    /// [`Groups::point`] when its x is given.
    fn point_opt(&self, code: i32) -> Option<Point> {
        self.num(code).map(|x| Point::new(x, self.num_or(code + 10, 0.0)))
    }
}

impl<'a> Groups<'a> for [Pair<'a>] {
    fn raw(&self, code: i32) -> Option<&'a str> {
        self.iter().find(|p| p.code == code).map(|p| p.value)
    }
}

/// A finite number (`1e999`, `NaN` and junk are none).
pub(crate) fn number(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// A whole number; some writers put integers as `1.0`.
pub(crate) fn integer(s: &str) -> Option<i64> {
    let s = s.trim();
    s.parse::<i64>().ok().or_else(|| number(s).filter(|v| v.abs() < 9.0e15 && v.fract() == 0.0).map(|v| v as i64))
}

/// One object: its type (the value of its `0` group) and the groups up to the next object.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Record<'a> {
    pub kind: &'a str,
    pub g: &'a [Pair<'a>],
}

/// The text of a DXF file: UTF-8 (DXF 2007 and later), else the Windows code page older versions
/// write (a byte order mark is dropped).
pub(crate) fn decode(bytes: &[u8]) -> Cow<'_, str> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => Cow::Borrowed(s),
        Err(_) => Cow::Owned(bytes.iter().map(|b| windows_1252(*b)).collect()),
    }
}

/// A byte of the Windows Western code page.
fn windows_1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—',
        '˜', '™', 'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
    ];
    match b {
        0x80..=0x9F => HIGH.get(usize::from(b - 0x80)).copied().unwrap_or('?'),
        _ => char::from(b),
    }
}

/// The group-code pairs of `text`, up to `0 EOF`. A line that isn't a group code ends the
/// reading: what came before is kept and the second value says where the file is damaged.
pub(crate) fn pairs(text: &str) -> Result<(Vec<Pair<'_>>, Option<String>), String> {
    let mut out = Vec::new();
    let mut lines = text.lines().enumerate();
    let mut damaged = None;
    while let Some((n, code)) = lines.next() {
        let c = code.trim();
        if c.is_empty() && out.is_empty() {
            continue;
        }
        let Ok(code) = c.parse::<i32>() else {
            damaged = Some(format!("the file is damaged at line {}: what follows is left out", n + 1));
            break;
        };
        let value = lines.next().map_or("", |(_, v)| v);
        if code == 0 && value.trim() == "EOF" {
            break;
        }
        if out.len() >= MAX_PAIRS {
            return Err("the drawing is too large to open".into());
        }
        out.push(Pair { code, value });
    }
    if out.is_empty() {
        return Err("not a DXF drawing".into());
    }
    Ok((out, damaged))
}

/// The objects in `pairs`: each from a `0` group up to the next.
fn records<'a>(pairs: &'a [Pair<'a>]) -> Vec<Record<'a>> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (i, p) in pairs.iter().enumerate() {
        if p.code == 0 {
            if let Some(s) = start {
                out.extend(record(pairs, s, i));
            }
            start = Some(i);
        }
    }
    if let Some(s) = start {
        out.extend(record(pairs, s, pairs.len()));
    }
    out
}

fn record<'a>(pairs: &'a [Pair<'a>], start: usize, end: usize) -> Option<Record<'a>> {
    let kind = pairs.get(start)?.value.trim();
    Some(Record { kind, g: pairs.get(start + 1..end)? })
}

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
    pub records: Vec<Record<'a>>,
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
    pub entities: Vec<Record<'a>>,
    /// Model space first, then the paper layouts in tab order.
    pub layouts: Vec<LayoutDef>,
    /// How many sections the file has (none: not a drawing).
    pub sections: usize,
}

impl<'a> Drawing<'a> {
    /// Read the sections of `pairs`.
    pub fn read(pairs: &'a [Pair<'a>]) -> Self {
        let mut d = Drawing { header: Header { ltscale: 1.0, lwdefault: 25, ..Header::default() }, ..Drawing::default() };
        let all = records(pairs);
        let mut section: Option<&str> = None;
        let (mut tables, mut blocks, mut objects) = (vec![], vec![], vec![]);
        for r in all {
            match (r.kind, section) {
                ("SECTION", _) => {
                    d.sections += 1;
                    section = r.g.name(2);
                    if section == Some("HEADER") {
                        d.header = header(r.g);
                    }
                }
                ("ENDSEC", _) => section = None,
                (_, Some("TABLES")) => tables.push(r),
                (_, Some("BLOCKS")) => blocks.push(r),
                (_, Some("ENTITIES")) => d.entities.push(r),
                (_, Some("OBJECTS")) => objects.push(r),
                _ => {}
            }
        }
        let mut block_records = HashMap::new();
        let mut table = "";
        for r in &tables {
            match r.kind {
                "TABLE" => table = r.g.name(2).unwrap_or(""),
                "LAYER" if table == "LAYER" => {
                    if let Some(l) = layer(r.g) {
                        d.layer_index.entry(l.name.to_uppercase()).or_insert(d.layers.len());
                        d.layers.push(l);
                    }
                }
                "LTYPE" if table == "LTYPE" => {
                    if let Some(name) = r.g.name(2) {
                        let pattern: Vec<f64> = r.g.iter().filter(|p| p.code == 49).filter_map(|p| number(p.value)).collect();
                        d.linetypes.insert(name.to_uppercase(), pattern);
                    }
                }
                "STYLE" if table == "STYLE" => {
                    if let Some(name) = r.g.name(2) {
                        d.styles.insert(name.to_uppercase(), font_family(r.g));
                    }
                }
                "BLOCK_RECORD" if table == "BLOCK_RECORD" => {
                    if let (Some(h), Some(name)) = (r.g.name(5), r.g.name(2)) {
                        block_records.insert(h.to_uppercase(), name.to_uppercase());
                    }
                }
                _ => {}
            }
        }
        d.read_blocks(&blocks, &mut block_records);
        d.layouts = layouts(&objects, &block_records, &d.entities);
        d
    }

    fn read_blocks(&mut self, records: &[Record<'a>], block_records: &mut HashMap<String, String>) {
        let mut current: Option<BlockDef<'a>> = None;
        for r in records {
            match r.kind {
                "BLOCK" => {
                    let name = r.g.name(2).or_else(|| r.g.name(3)).unwrap_or("").to_string();
                    // R13+ blocks name their block record: layouts find their block through it.
                    if let Some(owner) = r.g.name(330) {
                        block_records.entry(owner.to_uppercase()).or_insert_with(|| name.to_uppercase());
                    }
                    let xref = r.g.int_or(70, 0) & 4 != 0;
                    current = Some(BlockDef { name, base: r.g.point(10), records: vec![], xref });
                }
                "ENDBLK" => {
                    if let Some(b) = current.take().filter(|b| !b.name.is_empty()) {
                        self.blocks.insert(b.name.to_uppercase(), b);
                    }
                }
                _ => {
                    if let Some(b) = &mut current {
                        b.records.push(*r);
                    }
                }
            }
        }
    }

    /// The layout named `name` (any case; `model` is model space).
    pub fn layout(&self, name: &str) -> Option<&LayoutDef> {
        self.layouts.iter().find(|l| l.name.eq_ignore_ascii_case(name.trim()))
    }

    /// The entity records of `layout`.
    pub fn layout_records(&self, layout: &LayoutDef) -> Vec<Record<'a>> {
        let block = |name: &str| self.blocks.get(name).map(|b| b.records.clone()).unwrap_or_default();
        // The active paper layout keeps its art in the ENTITIES section, flagged paper space.
        let (block_name, paper) = match &layout.block {
            None => (MODEL_BLOCK, false),
            Some(b) if b == PAPER_BLOCK => (PAPER_BLOCK, true),
            Some(b) => return block(b),
        };
        let mut out: Vec<Record<'a>> = groups(&self.entities).filter(|g| in_paper_space(g) == paper).flatten().copied().collect();
        out.extend(block(block_name));
        out
    }

    /// The layer named `name` (any case).
    pub fn layer(&self, name: &str) -> Option<&LayerDef> {
        self.layer_index.get(&name.to_uppercase()).and_then(|i| self.layers.get(*i))
    }
}

/// Is an entity (its first record) in paper space?
fn in_paper_space(group: &[Record<'_>]) -> bool {
    group.first().is_some_and(|r| r.g.int(67) == Some(1))
}

/// The entities of `records`, each with the records that belong to it (a polyline's vertices,
/// an insert's attributes, and the SEQEND closing them).
pub(crate) fn groups<'r, 'a>(records: &'r [Record<'a>]) -> impl Iterator<Item = &'r [Record<'a>]> {
    let mut i = 0;
    std::iter::from_fn(move || {
        let start = i;
        let head = records.get(start)?;
        i += 1;
        if matches!(head.kind, "POLYLINE" | "INSERT") {
            while let Some(r) = records.get(i) {
                match r.kind {
                    "VERTEX" | "ATTRIB" => i += 1,
                    "SEQEND" => {
                        i += 1;
                        break;
                    }
                    _ => break,
                }
            }
        }
        records.get(start..i)
    })
}

fn header(g: &[Pair<'_>]) -> Header {
    let mut h = Header { ltscale: 1.0, lwdefault: 25, ..Header::default() };
    let mut var = "";
    for p in g {
        if p.code == 9 {
            var = p.value.trim();
            continue;
        }
        match (var, p.code) {
            ("$ACADVER", 1) => h.acadver = p.value.trim().to_string(),
            ("$INSUNITS", 70) => h.insunits = integer(p.value).unwrap_or(0),
            ("$MEASUREMENT", 70) => h.measurement = integer(p.value),
            ("$LTSCALE", 40) => h.ltscale = number(p.value).filter(|v| *v > 0.0).unwrap_or(1.0),
            ("$LWDEFAULT", 370) => h.lwdefault = integer(p.value).and_then(|v| i32::try_from(v).ok()).filter(|v| *v >= 0).unwrap_or(25),
            _ => {}
        }
    }
    h
}

fn layer(g: &[Pair<'_>]) -> Option<LayerDef> {
    let name = g.name(2)?.to_string();
    let aci = g.int_or(62, 7);
    let flags = g.int_or(70, 0);
    let rgb = super::entity::true_color(g).unwrap_or_else(|| super::entity::aci_paper(aci.unsigned_abs()));
    Some(LayerDef {
        // CAD apps never plot this layer of dimension points.
        plot: g.int_or(290, 1) != 0 && !name.eq_ignore_ascii_case("Defpoints"),
        name,
        rgb,
        hidden: aci < 0 || flags & 1 != 0,
        locked: flags & 4 != 0,
        linetype: g.name(6).unwrap_or("CONTINUOUS").to_uppercase(),
        lineweight: g.int(370).and_then(|v| i32::try_from(v).ok()).unwrap_or(-3),
    })
}

/// A text style's font family: the one 2000-and-later files name, else the font file's name
/// (TrueType); CAD fonts (`.shx`) have none.
fn font_family(g: &[Pair<'_>]) -> String {
    if let Some(i) = g.iter().position(|p| p.code == 1001 && p.value.trim() == "ACAD")
        && let Some(f) = g.get(i + 1..).and_then(|rest| rest.name(1000))
    {
        return f.to_string();
    }
    let file = g.name(3).unwrap_or("");
    let path = std::path::Path::new(file);
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "ttf" | "otf" | "ttc" => path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
        _ => String::new(),
    }
}

/// Model space, then each paper layout (its LAYOUT object, in tab order); a file without layout
/// objects but with paper-space art has one paper layout.
/// The most bytes of layout and block names the layouts keep together. Each layout keeps its own
/// copy of its block's name, so many layouts naming one long block name multiplied it; real
/// drawings have a few dozen layouts with short names.
const MAX_LAYOUT_NAME_BYTES: usize = 4 << 20;

fn layouts(objects: &[Record<'_>], block_records: &HashMap<String, String>, entities: &[Record<'_>]) -> Vec<LayoutDef> {
    let mut paper: Vec<(i64, LayoutDef)> = vec![];
    let mut seen = std::collections::HashSet::new();
    let mut name_bytes = 0usize;
    for r in objects.iter().filter(|r| r.kind == "LAYOUT") {
        // The layout's own groups follow its subclass marker.
        let own = r.g.iter().position(|p| p.code == 100 && p.value.trim() == "AcDbLayout").and_then(|i| r.g.get(i + 1..)).unwrap_or(&[]);
        let Some(name) = own.name(1) else { continue };
        let block = own.name(330).and_then(|h| block_records.get(&h.to_uppercase()));
        if name.eq_ignore_ascii_case(MODEL) || block.map(String::as_str) == Some(MODEL_BLOCK) {
            continue;
        }
        let Some(block) = block else { continue };
        let key = name.to_ascii_lowercase();
        let cost = name.len() + block.len();
        // A layout past the allowance is skipped alone: a later short one is still offered, and a
        // skipped one reserves neither its name nor its bytes.
        if seen.contains(&key) || name_bytes.saturating_add(cost) > MAX_LAYOUT_NAME_BYTES {
            continue;
        }
        seen.insert(key);
        name_bytes += cost;
        paper.push((own.int_or(71, i64::MAX), LayoutDef { name: name.to_string(), block: Some(block.clone()) }));
    }
    paper.sort_by_key(|(order, _)| *order);
    if paper.is_empty() && groups(entities).any(in_paper_space) {
        paper.push((1, LayoutDef { name: "Layout1".into(), block: Some(PAPER_BLOCK.into()) }));
    }
    std::iter::once(LayoutDef { name: MODEL.into(), block: None }).chain(paper.into_iter().map(|(_, l)| l)).collect()
}

#[cfg(test)]
mod tests {
    use super::integer;

    #[test]
    fn integer_rejects_fractional_float_values() {
        assert_eq!(integer("1.0"), Some(1));
        assert_eq!(integer("1.9"), None);
    }
}
