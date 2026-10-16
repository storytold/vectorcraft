//! One record per entry in the record list. Sizes follow the FreeHand 9 layout unless a version
//! delta says otherwise. A name this module does not know cannot be skipped safely, so parsing
//! stops.

use crate::Error;
use crate::container::Limits;
use crate::cursor::{Cur, mul_size};
use crate::mac;

/// `m11, m12, m13, m21, m22, m23` with `x' = m11·x + m12·y + m13`.
pub(crate) type Matrix = [f64; 6];

pub(crate) const IDENTITY: Matrix = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0];

#[derive(Debug, Clone)]
pub(crate) struct Node3 {
    pub a: [f64; 2],
    pub cin: [f64; 2],
    pub cout: [f64; 2],
}

#[derive(Debug, Clone)]
pub(crate) struct Prop {
    pub kind: u16,
    pub id: u16,
    pub num: u32,
}

#[derive(Debug)]
pub(crate) enum ColorRec {
    Rgb(f64, f64, f64),
    /// Older `TintColor` stored as a fraction of `base` toward white (`tint` on a 0..65536 scale).
    Tint {
        base: u32,
        tint: u16,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GradNote {
    None,
    Tapered,
    Cone,
    Contour,
}

#[derive(Debug)]
pub(crate) enum FillRec {
    Solid(u32),
    /// A PostScript fill, kept as the solid colour it names.
    PostScript(u32),
    Linear {
        c1: u32,
        c2: u32,
        angle: f64,
        stops: u32,
        note: GradNote,
    },
    Radial {
        c1: u32,
        c2: u32,
        cx: f64,
        cy: f64,
        stops: u32,
        note: GradNote,
    },
    Lens {
        color: u32,
        value: f64,
        mode: u8,
    },
    Pattern(u32),
    Tile,
    Custom {
        color: u32,
        width: Option<f64>,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct StrokeRec {
    pub color: u32,
    pub dashes: u32,
    pub width: f64,
    pub arrows: bool,
    /// A pattern stroke, drawn as this solid colour.
    pub patterned: bool,
    /// A PostScript stroke, drawn as this solid colour.
    pub postscript: bool,
}

#[derive(Debug, Clone)]
pub(crate) enum ShapeRec {
    Rect { style: u32, xform: u32, b: [f64; 4], rx: f64, ry: f64 },
    Oval { style: u32, xform: u32, b: [f64; 4], arc: Option<(f64, f64, bool)> },
    Poly { style: u32, xform: u32, n: u16, even_odd: bool, cx: f64, cy: f64, r1: f64, r2: f64, a1: f64, a2: f64 },
}

#[derive(Debug, Clone)]
pub(crate) struct TextRec {
    pub xform: u32,
    pub text: u32,
    pub attrs: u32,
    pub props: Vec<Prop>,
}

#[derive(Debug)]
pub(crate) struct ParaRec {
    pub block: u32,
    /// (start index into the block, character-style id).
    pub runs: Vec<(u16, u32)>,
}

#[derive(Debug)]
pub(crate) enum Rec {
    None,
    Str(String),
    List {
        ids: Vec<u32>,
    },
    Pairs {
        parent: u32,
        pairs: Vec<(u32, u32)>,
    },
    Graphic {
        parent: u32,
        attrs: u32,
        pairs: Vec<(u32, u32)>,
    },
    Holder {
        parent: u32,
        value: u32,
    },
    FilterStyle {
        style: u32,
    },
    Block {
        layers: u32,
    },
    Layer {
        elements: u32,
        name: u32,
        visibility: u16,
    },
    /// `clip` is a `ClipGroup`: the same on-disk layout as `Group`.
    Group {
        style: u32,
        elements: u32,
        xform: u32,
        clip: bool,
    },
    Composite {
        style: u32,
        elements: u32,
    },
    Path {
        style: u32,
        closed: bool,
        even_odd: bool,
        nodes: Vec<Node3>,
    },
    Shape(ShapeRec),
    Xform(Matrix),
    Fill(FillRec),
    Stroke(StrokeRec),
    Dashes(Vec<f64>),
    Color(ColorRec),
    Stops(Vec<(u32, f64)>),
    Props(Vec<Prop>),
    SymbolClass {
        group: u32,
    },
    SymbolInstance {
        class: u32,
        matrix: Matrix,
    },
    Blend {
        lists: [u32; 3],
    },
    Text(TextRec),
    Story {
        paras: Vec<u32>,
    },
    Paragraph(ParaRec),
    Image,
}

pub(crate) struct RawFile {
    pub version: u16,
    pub recs: Vec<Rec>,
    pub names: Vec<String>,
    pub block_id: u32,
    pub props_id: u32,
    pub fallback: Option<(f64, f64)>,
}

impl RawFile {
    pub(crate) fn get(&self, id: u32) -> Option<&Rec> {
        if id == 0 { None } else { self.recs.get(usize::try_from(id.saturating_sub(1)).ok()?) }
    }

    pub(crate) fn name(&self, id: u32) -> &str {
        if id == 0 {
            return "";
        }
        self.names.get(usize::try_from(id.saturating_sub(1)).unwrap_or(usize::MAX)).map(String::as_str).unwrap_or("")
    }
}

struct Ctx<'a, 'b> {
    c: &'a mut Cur<'b>,
    version: u16,
    limits: &'a Limits,
    nodes: usize,
    items: usize,
}

pub(crate) fn parse(c: &mut Cur<'_>, types: &[String], version: u16, limits: &Limits) -> Result<Vec<Rec>, Error> {
    c.set_max_id(types.len());
    let mut ctx = Ctx { c, version, limits, nodes: 0, items: 0 };
    let mut recs = Vec::new();
    recs.try_reserve(types.len()).map_err(|_| Error::new("the FreeHand file claims more records than can be read"))?;
    for name in types {
        recs.push(read_one(&mut ctx, name)?);
    }
    Ok(recs)
}

fn read_one(x: &mut Ctx<'_, '_>, name: &str) -> Result<Rec, Error> {
    match name {
        "MName" | "MString" => Ok(Rec::Str(sized_string(x, false)?)),
        "UString" => Ok(Rec::Str(sized_string(x, true)?)),
        "TextBlok" => Ok(Rec::Str(text_blok(x)?)),
        "PropLst" => prop_list(x),
        "ElemPropLst" | "StylePropLst" => elem_props(x),
        "GraphicStyle" => graphic_style(x),
        "AttributeHolder" => Ok(Rec::Holder { parent: x.c.id()?, value: x.c.id()? }),
        "FilterAttributeHolder" => {
            let _parent = x.c.id()?;
            let _filter = x.c.id()?;
            Ok(Rec::FilterStyle { style: x.c.id()? })
        }
        "List" | "MList" | "BrushList" => list(x),
        "Xform" => xform(x),
        "Block" => block(x),
        "Layer" => layer(x),
        "Group" => group(x, false),
        "ClipGroup" => group(x, true),
        "CompositePath" => composite(x),
        "Path" => path(x),
        "Rectangle" => rectangle(x),
        "Oval" => oval(x),
        "PolygonFigure" => polygon(x),
        "ArrowPath" => arrow_path(x),
        "VMpObj" | "AGDFont" | "TEffect" | "VDict" => bag(x),
        "BasicFill" => {
            let color = x.c.id()?;
            x.c.skip(4)?;
            Ok(Rec::Fill(FillRec::Solid(color)))
        }
        "BasicLine" => basic_line(x),
        "LinePat" => line_pat(x),
        "PatternLine" => pattern_line(x),
        "LinearFill" => linear(x, true, GradNote::None),
        "TaperedFillX" => linear(x, false, GradNote::None),
        "TaperedFill" => tapered(x),
        "ConeFill" => cone(x),
        "RadialFillX" => radial_x(x, 0, GradNote::None),
        "NewRadialFill" => radial_x(x, 23, GradNote::None),
        "RadialFill" => radial(x),
        "NewContourFill" | "ContourFill" => contour(x),
        "MultiColorList" => stops(x),
        "LensFill" => lens(x),
        "PatternFill" => {
            let color = x.c.id()?;
            x.c.skip(8)?;
            Ok(Rec::Fill(FillRec::Pattern(color)))
        }
        "TileFill" => tile(x),
        "CustomProc" => custom(x),
        "Color6" => color6(x),
        "SpotColor6" => spot6(x),
        "TintColor6" => tint6(x),
        "ProcessColor" => process_color(x),
        "SpotColor" => named_rgb(x, 16),
        "PantoneColor" => named_rgb(x, 28),
        "TintColor" => tint_color(x),
        "TextColumn" | "TextInPath" | "TFOnPath" => text_obj(x),
        "TString" => tstring(x),
        "Paragraph" => paragraph(x),
        "ImageImport" => image_import(x),
        "SwfImport" => swf_import(x),
        "Data" => data_rec(x),
        "DataList" => data_list(x),
        "SymbolClass" => symbol_class(x),
        "SymbolInstance" => symbol_instance(x),
        "SymbolLibrary" => symbol_library(x),
        "NewBlend" => new_blend(x),
        "TabTable" => tab_table(x),
        "DateTime" => skip_rec(x, 14),
        "FHDocHeader" => skip_rec(x, 4),
        "Guides" => guides(x),
        "PerspectiveGrid" => perspective_grid(x),
        "FileDescriptor" => file_descriptor(x),
        "Procedure" | "Collector" | "Element" | "ElemList" | "Figure" | "MpObject" | "MasterPageDocMan" => skip_rec(x, 4),
        "Halftone" => {
            let _ = x.c.id()?;
            skip_rec(x, 8)
        }
        "ImageFill" => skip_rec(x, 6),
        "BendFilter" => skip_rec(x, 10),
        "DuetFilter" | "ExpandFilter" => skip_rec(x, 14),
        "FWBlurFilter" => skip_rec(x, 12),
        "FWFeatherFilter" => skip_rec(x, 8),
        "FWSharpenFilter" | "RaggedFilter" => skip_rec(x, 16),
        "FWBevelFilter" => {
            let _ = x.c.id()?;
            skip_rec(x, 28)
        }
        "SketchFilter" => skip_rec(x, 11),
        "TransformFilter" => skip_rec(x, 39),
        "GradientMaskFilter" => {
            let _ = x.c.id()?;
            Ok(Rec::None)
        }
        "OpacityFilter" => {
            let _ = x.c.id()?;
            let _ = x.c.u16()?;
            Ok(Rec::None)
        }
        "FWShadowFilter" => fw_shadow(x),
        "FWGlowFilter" => fw_glow(x),
        "MDict" => mdict(x),
        "MQuickDict" => mquick(x),
        "Envelope" => envelope(x),
        "PerspectiveEnvelope" => skip_rec(x, 177),
        "EPSImport" => skip_rec(x, 38),
        "Import" => skip_rec(x, 34),
        "MasterPageElement" | "MasterPageLayerElement" => skip_rec(x, 14),
        "MasterPageLayerInstance" | "MasterPageSymbolInstance" => master_matrix(x),
        "MasterPageSymbolClass" => skip_rec(x, 12),
        "MultiBlend" => multi_blend(x),
        "BlendObject" => blend_object(x),
        "ConnectorLine" => connector(x),
        "Brush" => {
            let _ = x.c.id()?;
            let _ = x.c.id()?;
            Ok(Rec::None)
        }
        "BrushStroke" => {
            let _ = x.c.id()?;
            let _ = x.c.id()?;
            let _ = x.c.id()?;
            Ok(Rec::None)
        }
        "BrushTip" => {
            let _ = x.c.id()?;
            x.c.skip(60)?;
            if x.version >= 11 {
                x.c.skip(4)?;
            }
            Ok(Rec::None)
        }
        "CalligraphicStroke" => {
            let _ = x.c.id()?;
            x.c.skip(12)?;
            let _ = x.c.id()?;
            Ok(Rec::None)
        }
        "ContentFill" | "CharacterFill" => Ok(Rec::None),
        "PSFill" => {
            let color = x.c.id()?;
            let _ = x.c.id()?;
            Ok(Rec::Fill(FillRec::PostScript(color)))
        }
        "PSLine" => {
            let color = x.c.id()?;
            let _ = x.c.id()?;
            let width = x.c.fixed()?;
            Ok(Rec::Stroke(StrokeRec { color, dashes: 0, width, arrows: false, patterned: false, postscript: true }))
        }
        "LineTable" => line_table(x),
        "AGDSelection" => agd_selection(x),
        "Extrusion" => extrusion(x),
        "PathText" => path_text(x),
        _ => Err("the FreeHand file has a record whose size is not known".into()),
    }
}

fn skip_rec(x: &mut Ctx<'_, '_>, n: usize) -> Result<Rec, Error> {
    x.c.skip(n)?;
    Ok(Rec::None)
}

fn take_items(x: &mut Ctx<'_, '_>, n: usize, what: &str) -> Result<(), Error> {
    if n > x.limits.max_items || x.items.saturating_add(n) > x.limits.max_items {
        return match what {
            "list" => Err("the FreeHand file claims more list entries than can be read".into()),
            "path nodes" => Err("the FreeHand file claims more path nodes than can be read".into()),
            _ => Err("the FreeHand file claims more entries than can be read".into()),
        };
    }
    x.items += n;
    Ok(())
}

fn read_props(x: &mut Ctx<'_, '_>, n: usize) -> Result<Vec<Prop>, Error> {
    take_items(x, n, "properties")?;
    let mut v = Vec::new();
    v.try_reserve(n).map_err(|_| crate::entries())?;
    for _ in 0..n {
        let kind = x.c.u16()?;
        let id = x.c.u16()?;
        let num = if kind == 2 { x.c.id()? } else { x.c.u32()? };
        v.push(Prop { kind, id, num });
    }
    Ok(v)
}

fn read_pairs(x: &mut Ctx<'_, '_>, count: u16) -> Result<Vec<(u32, u32)>, Error> {
    let n = usize::from(count);
    take_items(x, n, "properties")?;
    let mut v = Vec::new();
    v.try_reserve(n).map_err(|_| crate::entries())?;
    for _ in 0..n {
        v.push((x.c.id()?, x.c.id()?));
    }
    Ok(v)
}

fn read_ids(x: &mut Ctx<'_, '_>, count: u16) -> Result<Vec<u32>, Error> {
    let n = usize::from(count);
    if n >= 1024 && n.saturating_mul(2) > x.c.remaining() {
        return Err("the FreeHand file claims more list entries than can be read".into());
    }
    take_items(x, n, "list")?;
    let mut v = Vec::new();
    v.try_reserve(n).map_err(|_| crate::entries())?;
    for _ in 0..n {
        v.push(x.c.id()?);
    }
    Ok(v)
}

fn sized_string(x: &mut Ctx<'_, '_>, utf16: bool) -> Result<String, Error> {
    let start = x.c.pos;
    let size = usize::from(x.c.u16()?);
    let len = usize::from(x.c.u16()?);
    if size > x.limits.max_string {
        return Err(crate::entries());
    }
    let total = size.checked_add(1).and_then(|n| n.checked_mul(4)).ok_or_else(crate::entries)?;
    let body = total.checked_sub(4).ok_or_else(|| x.c.fail())?;
    if body > x.c.remaining() {
        return Err(x.c.fail());
    }
    let text = if utf16 {
        let n = len.min(body / 2).min(x.limits.max_string);
        let mut units = Vec::new();
        for i in 0..n {
            let u = x.c.u16()?;
            if u == 0 {
                break;
            }
            if i == 0 && u == 0xFEFF {
                continue;
            }
            units.push(u);
        }
        String::from_utf16_lossy(&units)
    } else {
        let n = len.min(body).min(x.limits.max_string);
        let bytes = x.c.bytes(n)?;
        let cut = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
        mac::decode(bytes.get(..cut).unwrap_or(bytes))
    };
    let end = start + total;
    if end < x.c.pos || end > x.c.pos + x.c.remaining() {
        return Err(x.c.fail());
    }
    x.c.pos = end;
    Ok(text)
}

fn text_blok(x: &mut Ctx<'_, '_>) -> Result<String, Error> {
    let size = usize::from(x.c.u16()?);
    let len = usize::from(x.c.u16()?);
    if size > x.limits.max_string {
        return Err(crate::entries());
    }
    let body = size.checked_mul(4).ok_or_else(crate::entries)?;
    if body > x.c.remaining() {
        return Err(x.c.fail());
    }
    let end = x.c.pos + body;
    let n = len.min(body / 2);
    let mut units = Vec::new();
    for _ in 0..n {
        let u = x.c.u16()?;
        if u == 0 {
            break;
        }
        match u {
            0x09 => units.push(u),
            0x0B => units.push(u16::from(b'\n')),
            0x01..=0x1F => {}
            _ => units.push(u),
        }
    }
    x.c.pos = end;
    Ok(String::from_utf16_lossy(&units))
}

fn prop_list(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let capacity = x.c.u16()?;
    let count = x.c.u16()?;
    x.c.skip(4)?;
    let pairs = read_pairs(x, count)?;
    if x.version <= 8 {
        let extra = u32::from(capacity.saturating_sub(count));
        x.c.skip(mul_size(extra, 4, x.limits.max_decompressed)?)?;
    }
    Ok(Rec::Pairs { parent: 0, pairs })
}

fn elem_props(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    if x.version >= 9 {
        x.c.skip(2)?;
    }
    let count = x.c.u16()?;
    x.c.skip(2)?;
    let parent = x.c.id()?;
    let _name = x.c.id()?;
    let pairs = read_pairs(x, count)?;
    Ok(Rec::Pairs { parent, pairs })
}

fn graphic_style(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    x.c.skip(2)?;
    let count = x.c.u16()?;
    x.c.skip(2)?;
    let parent = x.c.id()?;
    let attrs = x.c.id()?;
    let pairs = read_pairs(x, count)?;
    Ok(Rec::Graphic { parent, attrs, pairs })
}

fn list(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let capacity = x.c.u16()?;
    let count = x.c.u16()?;
    x.c.skip(6)?;
    let _kind = x.c.u16()?;
    let ids = read_ids(x, count)?;
    if x.version <= 8 {
        let extra = u32::from(capacity.saturating_sub(count));
        x.c.skip(mul_size(extra, 2, x.limits.max_decompressed)?)?;
    }
    Ok(Rec::List { ids })
}

fn component_count(f1: u8, f2: u8) -> usize {
    if f1 & 0x04 != 0 {
        return 0;
    }
    let mut n = 0;
    if f1 & 0x10 == 0 {
        n += 1;
    }
    if f2 & 0x40 != 0 {
        n += 1;
    }
    if f2 & 0x20 != 0 {
        n += 1;
    }
    if f1 & 0x20 == 0 {
        n += 1;
    }
    if f1 & 0x01 != 0 {
        n += 1;
    }
    if f1 & 0x02 != 0 {
        n += 1;
    }
    n
}

fn matrix_body(c: &mut Cur<'_>, f1: u8, f2: u8) -> Result<Matrix, Error> {
    let mut m = IDENTITY;
    if f1 & 0x04 != 0 {
        return Ok(m);
    }
    if f1 & 0x10 == 0 {
        m[0] = c.fixed()?;
    }
    if f2 & 0x40 != 0 {
        m[3] = c.fixed()?;
    }
    if f2 & 0x20 != 0 {
        m[1] = c.fixed()?;
    }
    if f1 & 0x20 == 0 {
        m[4] = c.fixed()?;
    }
    if f1 & 0x01 != 0 {
        m[2] = c.fixed()?;
    }
    if f1 & 0x02 != 0 {
        m[5] = c.fixed()?;
    }
    Ok(m)
}

fn xform(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    if x.version <= 8 {
        x.c.skip(2)?;
        let m11 = x.c.fixed()?;
        let m21 = x.c.fixed()?;
        let m12 = x.c.fixed()?;
        let m22 = x.c.fixed()?;
        let m13 = x.c.fixed()?;
        let m23 = x.c.fixed()?;
        x.c.skip(26)?;
        return Ok(Rec::Xform([m11, m12, m13, m21, m22, m23]));
    }
    let f1 = x.c.u8()?;
    let f2 = x.c.u8()?;
    let m = matrix_body(x.c, f1, f2)?;
    let g1 = x.c.u8()?;
    let g2 = x.c.u8()?;
    let _inverse = matrix_body(x.c, g1, g2)?;
    Ok(Rec::Xform(m))
}

fn flagged_matrix(x: &mut Ctx<'_, '_>) -> Result<Matrix, Error> {
    let f1 = x.c.u8()?;
    let f2 = x.c.u8()?;
    matrix_body(x.c, f1, f2)
}

fn block(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let layers = if x.version >= 11 || x.version == 9 {
        let mut ids = [0u32; 12];
        for slot in &mut ids {
            *slot = x.c.id()?;
        }
        x.c.skip(14)?;
        for _ in 0..3 {
            let _ = x.c.id()?;
        }
        x.c.skip(1)?;
        let n = if x.version < 10 { 1 } else { 4 };
        for _ in 0..n {
            let _ = x.c.id()?;
        }
        ids.get(5).copied().unwrap_or(0)
    } else if x.version == 10 {
        let _ = x.c.u16()?;
        let mut ids = [0u32; 21];
        for slot in &mut ids {
            *slot = x.c.id()?;
        }
        x.c.skip(1)?;
        let _ = x.c.id()?;
        let _ = x.c.id()?;
        ids.get(5).copied().unwrap_or(0)
    } else if x.version == 8 {
        let mut ids = [0u32; 12];
        for slot in &mut ids {
            *slot = x.c.id()?;
        }
        x.c.skip(14)?;
        ids.get(5).copied().unwrap_or(0)
    } else {
        let mut ids = [0u32; 11];
        for slot in &mut ids {
            *slot = x.c.id()?;
        }
        x.c.skip(10)?;
        for _ in 0..3 {
            let _ = x.c.id()?;
        }
        ids.get(5).copied().unwrap_or(0)
    };
    Ok(Rec::Block { layers })
}

fn layer(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let _style = x.c.id()?;
    if x.version > 3 {
        x.c.skip(4)?;
    }
    x.c.skip(6)?;
    let elements = x.c.id()?;
    let name = x.c.id()?;
    let visibility = x.c.u16()?;
    x.c.skip(2)?;
    Ok(Rec::Layer { elements, name, visibility })
}

fn group(x: &mut Ctx<'_, '_>, clip: bool) -> Result<Rec, Error> {
    let style = x.c.id()?;
    let _parent = x.c.id()?;
    if x.version > 3 {
        x.c.skip(4)?;
    }
    x.c.skip(4)?;
    let elements = x.c.id()?;
    let xform = x.c.id()?;
    Ok(Rec::Group { style, elements, xform, clip })
}

fn composite(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let style = x.c.id()?;
    let _parent = x.c.id()?;
    if x.version > 3 {
        x.c.skip(4)?;
    }
    x.c.skip(4)?;
    Ok(Rec::Composite { style, elements: x.c.id()? })
}

fn read_nodes(x: &mut Ctx<'_, '_>, count: u32) -> Result<Vec<Node3>, Error> {
    let n = usize::try_from(count).unwrap_or(usize::MAX);
    let need = n.saturating_mul(27);
    if n > x.limits.max_items || x.nodes.saturating_add(n) > x.limits.max_items || (n >= 1024 && need > x.c.remaining()) {
        return Err("the FreeHand file claims more path nodes than can be read".into());
    }
    x.nodes += n;
    let mut nodes = Vec::new();
    nodes.try_reserve(n).map_err(|_| Error::new("the FreeHand file claims more path nodes than can be read"))?;
    for _ in 0..n {
        x.c.skip(3)?;
        let ax = x.c.fixed()?;
        let ay = x.c.fixed()?;
        let ix = x.c.fixed()?;
        let iy = x.c.fixed()?;
        let ox = x.c.fixed()?;
        let oy = x.c.fixed()?;
        nodes.push(Node3 { a: [ax, ay], cin: [ix, iy], cout: [ox, oy] });
    }
    Ok(nodes)
}

fn path(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let capacity = x.c.u16()?;
    let style = x.c.id()?;
    let _parent = x.c.id()?;
    if x.version > 3 {
        x.c.skip(4)?;
    }
    x.c.skip(9)?;
    let flags = x.c.u8()?;
    let count = x.c.u16()?;
    let nodes = read_nodes(x, u32::from(count))?;
    if x.version <= 8 {
        let extra = u32::from(capacity.saturating_sub(count));
        x.c.skip(mul_size(extra, 27, x.limits.max_decompressed)?)?;
    }
    Ok(Rec::Path { style, closed: flags & 1 != 0, even_odd: flags & 2 != 0, nodes })
}

fn rectangle(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let style = x.c.id()?;
    let _parent = x.c.id()?;
    if x.version > 3 {
        x.c.skip(4)?;
    }
    x.c.skip(8)?;
    let xform = x.c.id()?;
    let b = [x.c.fixed()?, x.c.fixed()?, x.c.fixed()?, x.c.fixed()?];
    let rx = x.c.fixed()?;
    let ry = x.c.fixed()?;
    if x.version >= 11 {
        for _ in 0..6 {
            let _ = x.c.fixed()?;
        }
        x.c.skip(9)?;
    }
    Ok(Rec::Shape(ShapeRec::Rect { style, xform, b, rx, ry }))
}

fn oval(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let style = x.c.id()?;
    let _parent = x.c.id()?;
    if x.version > 3 {
        x.c.skip(4)?;
    }
    x.c.skip(8)?;
    let xform = x.c.id()?;
    let b = [x.c.fixed()?, x.c.fixed()?, x.c.fixed()?, x.c.fixed()?];
    let arc = if x.version >= 11 {
        let start = x.c.fixed()?;
        let end = x.c.fixed()?;
        let pie = x.c.u8()? != 0;
        x.c.skip(1)?;
        if start.is_finite() && end.is_finite() && (start - end).abs() > 1e-4 { Some((start, end, pie)) } else { None }
    } else {
        None
    };
    Ok(Rec::Shape(ShapeRec::Oval { style, xform, b, arc }))
}

fn polygon(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let style = x.c.id()?;
    let _parent = x.c.id()?;
    x.c.skip(12)?;
    let xform = x.c.id()?;
    let n = x.c.u16()?;
    let even_odd = x.c.u8()? != 0;
    let cx = x.c.fixed()?;
    let cy = x.c.fixed()?;
    let r1 = x.c.fixed()?;
    let r2 = x.c.fixed()?;
    let a1 = x.c.fixed()?;
    let a2 = x.c.fixed()?;
    x.c.skip(8)?;
    Ok(Rec::Shape(ShapeRec::Poly { style, xform, n, even_odd, cx, cy, r1, r2, a1, a2 }))
}

fn arrow_path(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let count = if x.version > 8 {
        x.c.skip(20)?;
        let count = x.c.u16()?;
        if x.version > 3 {
            x.c.skip(4)?;
        }
        x.c.skip(4)?;
        count
    } else {
        let count = x.c.u16()?;
        x.c.skip(20)?;
        if x.version > 3 {
            x.c.skip(4)?;
        }
        x.c.skip(4)?;
        count
    };
    x.c.skip(mul_size(u32::from(count), 27, x.limits.max_decompressed)?)?;
    Ok(Rec::None)
}

fn bag(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    x.c.skip(4)?;
    let n = usize::from(x.c.u16()?);
    x.c.skip(2)?;
    Ok(Rec::Props(read_props(x, n)?))
}

fn basic_line(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let color = x.c.id()?;
    let dashes = x.c.id()?;
    let a0 = x.c.id()?;
    let a1 = x.c.id()?;
    let _miter = x.c.fixed()?;
    let width = x.c.fixed()?;
    x.c.skip(4)?;
    Ok(Rec::Stroke(StrokeRec { color, dashes, width, arrows: a0 != 0 || a1 != 0, patterned: false, postscript: false }))
}

fn line_pat(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let count = x.c.u16()?;
    if x.version <= 8 && count == 0 {
        x.c.skip(26)?;
        return Ok(Rec::Dashes(Vec::new()));
    }
    x.c.skip(8)?;
    let n = usize::from(count);
    take_items(x, n, "dash pattern")?;
    let mut lens = Vec::new();
    lens.try_reserve(n).map_err(|_| crate::entries())?;
    for _ in 0..n {
        lens.push(x.c.fixed()?);
    }
    Ok(Rec::Dashes(lens))
}

fn pattern_line(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let color = x.c.id()?;
    x.c.skip(8)?;
    let _miter = x.c.fixed()?;
    let width = x.c.fixed()?;
    x.c.skip(4)?;
    Ok(Rec::Stroke(StrokeRec { color, dashes: 0, width, arrows: false, patterned: true, postscript: false }))
}

fn linear(x: &mut Ctx<'_, '_>, tail16: bool, note: GradNote) -> Result<Rec, Error> {
    let c1 = x.c.id()?;
    let c2 = x.c.id()?;
    let angle = x.c.fixed()?;
    x.c.skip(8)?;
    let stops = x.c.id()?;
    if tail16 {
        x.c.skip(16)?;
    }
    Ok(Rec::Fill(FillRec::Linear { c1, c2, angle, stops, note }))
}

fn tapered(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let c1 = x.c.id()?;
    let c2 = x.c.id()?;
    let angle = x.c.fixed()?;
    x.c.skip(4)?;
    Ok(Rec::Fill(FillRec::Linear { c1, c2, angle, stops: 0, note: GradNote::Tapered }))
}

fn cone(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let c1 = x.c.id()?;
    let c2 = x.c.id()?;
    let _angle = x.c.fixed()?;
    let _cx = x.c.fixed()?;
    let _cy = x.c.fixed()?;
    let stops = x.c.id()?;
    x.c.skip(16)?;
    Ok(Rec::Fill(FillRec::Linear { c1, c2, angle: 0.0, stops, note: GradNote::Cone }))
}

fn radial_x(x: &mut Ctx<'_, '_>, extra: usize, note: GradNote) -> Result<Rec, Error> {
    let c1 = x.c.id()?;
    let c2 = x.c.id()?;
    let cx = x.c.fixed()?;
    let cy = x.c.fixed()?;
    x.c.skip(8)?;
    let stops = x.c.id()?;
    if extra > 0 {
        x.c.skip(extra)?;
    }
    Ok(Rec::Fill(FillRec::Radial { c1, c2, cx, cy, stops, note }))
}

fn radial(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let c1 = x.c.id()?;
    let c2 = x.c.id()?;
    let cx = x.c.fixed()?;
    let cy = x.c.fixed()?;
    x.c.skip(4)?;
    Ok(Rec::Fill(FillRec::Radial { c1, c2, cx, cy, stops: 0, note: GradNote::None }))
}

fn contour(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    if x.version < 10 {
        return Err(crate::unknown_size());
    }
    let c1 = x.c.id()?;
    let c2 = x.c.id()?;
    let cx = x.c.fixed()?;
    let cy = x.c.fixed()?;
    x.c.skip(8)?;
    x.c.skip(2)?;
    let stops = x.c.id()?;
    x.c.skip(2)?;
    let _ = x.c.fixed()?;
    let _ = x.c.fixed()?;
    x.c.skip(4)?;
    Ok(Rec::Fill(FillRec::Radial { c1, c2, cx, cy, stops, note: GradNote::Contour }))
}

fn stops(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let count = x.c.u16()?;
    x.c.skip(2)?;
    let n = usize::from(count);
    take_items(x, n, "gradient stops")?;
    let mut v = Vec::new();
    v.try_reserve(n).map_err(|_| crate::entries())?;
    for _ in 0..n {
        let color = x.c.id()?;
        let pos = x.c.fixed()?;
        x.c.skip(4)?;
        v.push((color, pos));
    }
    Ok(Rec::Stops(v))
}

fn lens(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let color = x.c.id()?;
    x.c.skip(6)?;
    let value = x.c.fixed()?;
    x.c.skip(27)?;
    let mode = x.c.u8()?;
    Ok(Rec::Fill(FillRec::Lens { color, value, mode }))
}

fn tile(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let _ = x.c.id()?;
    let _ = x.c.id()?;
    x.c.skip(8)?;
    for _ in 0..5 {
        let _ = x.c.fixed()?;
    }
    Ok(Rec::Fill(FillRec::Tile))
}

fn custom(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let count = x.c.u16()?;
    let _name = x.c.id()?;
    x.c.skip(4)?;
    let n = usize::from(count);
    take_items(x, n, "custom paint")?;
    let mut color = 0u32;
    let mut width = None;
    for _ in 0..n {
        let kind = x.c.u8()?;
        match kind {
            0 => {
                x.c.skip(7)?;
                let id = u32::from(x.c.u16()?);
                if color == 0 {
                    color = id;
                }
            }
            2 => {
                x.c.skip(3)?;
                let w = x.c.fixed()?;
                x.c.skip(2)?;
                if width.is_none() {
                    width = Some(w);
                }
            }
            3 | 4 => {
                x.c.skip(3)?;
                let _ = x.c.fixed()?;
                x.c.skip(2)?;
            }
            _ => x.c.skip(9)?,
        }
    }
    Ok(Rec::Fill(FillRec::Custom { color, width }))
}

fn channel(c: &mut Cur<'_>) -> Result<f64, Error> {
    Ok((f64::from(c.u16()?) / 65535.0).clamp(0.0, 1.0))
}

fn rgb3(c: &mut Cur<'_>) -> Result<(f64, f64, f64), Error> {
    Ok((channel(c)?, channel(c)?, channel(c)?))
}

fn color6_tail(version: u16, variant: u16) -> usize {
    let base: usize = match variant {
        4 => 16,
        7 => 28,
        9 => 36,
        _ => 12,
    };
    if version < 10 { base.saturating_sub(2) } else { base }
}

fn color6(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let variant = x.c.u16()?;
    let _name = x.c.id()?;
    let rgb = rgb3(x.c)?;
    x.c.skip(4)?;
    let _other = x.c.id()?;
    x.c.skip(color6_tail(x.version, variant))?;
    Ok(Rec::Color(ColorRec::Rgb(rgb.0, rgb.1, rgb.2)))
}

fn spot6(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let words = x.c.u16()?;
    let _name = x.c.id()?;
    let rgb = rgb3(x.c)?;
    x.c.skip(if x.version >= 10 { 18 } else { 16 })?;
    x.c.skip(mul_size(u32::from(words), 4, x.limits.max_decompressed)?)?;
    Ok(Rec::Color(ColorRec::Rgb(rgb.0, rgb.1, rgb.2)))
}

fn tint6(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    x.c.skip(2)?;
    let _name = x.c.id()?;
    let rgb = rgb3(x.c)?;
    x.c.skip(if x.version >= 10 { 28 } else { 26 })?;
    Ok(Rec::Color(ColorRec::Rgb(rgb.0, rgb.1, rgb.2)))
}

fn cmyk_rgb(c: u16, m: u16, y: u16, k: u16) -> (f64, f64, f64) {
    let u = |v: u16| f64::from(v) / 65535.0;
    let (c, m, y, k) = (u(c), u(m), u(y), u(k));
    ((1.0 - c) * (1.0 - k), (1.0 - m) * (1.0 - k), (1.0 - y) * (1.0 - k))
}

fn process_color(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let _name = x.c.id()?;
    x.c.skip(2)?;
    let (r, g, b) = rgb3(x.c)?;
    x.c.skip(4)?;
    if r == 0.0 && g == 0.0 && b == 0.0 {
        let k = x.c.u16()?;
        let c = x.c.u16()?;
        let m = x.c.u16()?;
        let y = x.c.u16()?;
        let (r, g, b) = cmyk_rgb(c, m, y, k);
        Ok(Rec::Color(ColorRec::Rgb(r, g, b)))
    } else {
        x.c.skip(8)?;
        Ok(Rec::Color(ColorRec::Rgb(r, g, b)))
    }
}

fn named_rgb(x: &mut Ctx<'_, '_>, tail: usize) -> Result<Rec, Error> {
    let _name = x.c.id()?;
    x.c.skip(2)?;
    let rgb = rgb3(x.c)?;
    x.c.skip(tail)?;
    Ok(Rec::Color(ColorRec::Rgb(rgb.0, rgb.1, rgb.2)))
}

fn tint_color(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let _name = x.c.id()?;
    x.c.skip(2)?;
    let (r, g, b) = rgb3(x.c)?;
    x.c.skip(4)?;
    if r == 0.0 && g == 0.0 && b == 0.0 {
        let base = x.c.id()?;
        let tint = x.c.u16()?;
        x.c.skip(2)?;
        Ok(Rec::Color(ColorRec::Tint { base, tint }))
    } else {
        let _ = x.c.id()?;
        x.c.skip(4)?;
        Ok(Rec::Color(ColorRec::Rgb(r, g, b)))
    }
}

fn text_obj(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    x.c.skip(4)?;
    let count = usize::from(x.c.u16()?);
    x.c.skip(2)?;
    let _style = x.c.id()?;
    let _parent = x.c.id()?;
    x.c.skip(8)?;
    let xform = x.c.id()?;
    let text = x.c.id()?;
    let attrs = x.c.id()?;
    let props = read_props(x, count)?;
    Ok(Rec::Text(TextRec { xform, text, attrs, props }))
}

fn tstring(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let capacity = x.c.u16()?;
    let count = x.c.u16()?;
    x.c.skip(16)?;
    let paras = read_ids(x, count)?;
    if x.version < 9 {
        let extra = u32::from(capacity.saturating_sub(count));
        x.c.skip(mul_size(extra, 2, x.limits.max_decompressed)?)?;
    }
    Ok(Rec::Story { paras })
}

fn paragraph(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    x.c.skip(2)?;
    let count = x.c.u16()?;
    x.c.skip(2)?;
    let _style = x.c.id()?;
    let block = x.c.id()?;
    let n = usize::from(count);
    take_items(x, n, "text runs")?;
    let mut runs = Vec::new();
    runs.try_reserve(n).map_err(|_| crate::entries())?;
    for _ in 0..n {
        let start = x.c.u16()?;
        let style = x.c.id()?;
        x.c.skip(20)?;
        runs.push((start, style));
    }
    Ok(Rec::Paragraph(ParaRec { block, runs }))
}

fn image_import(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    if x.version <= 8 {
        return Err(crate::unknown_size());
    }
    let _ = x.c.id()?;
    let _ = x.c.id()?;
    if x.version > 3 {
        x.c.skip(4)?;
    }
    x.c.skip(4)?;
    let _ = x.c.id()?;
    let _ = x.c.id()?;
    let _ = x.c.id()?;
    let _ = x.c.id()?;
    for _ in 0..4 {
        let _ = x.c.fixed()?;
    }
    x.c.skip(18)?;
    let _ = x.c.cstr(1024)?;
    if x.version >= 11 {
        x.c.skip(2)?;
    }
    Ok(Rec::Image)
}

fn swf_import(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let _ = x.c.id()?;
    let _ = x.c.id()?;
    x.c.skip(8)?;
    let _ = x.c.id()?;
    let _ = x.c.id()?;
    let _ = x.c.id()?;
    let _ = x.c.id()?;
    for _ in 0..4 {
        let _ = x.c.fixed()?;
    }
    x.c.skip(7)?;
    Ok(Rec::Image)
}

fn data_rec(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let blocks = u32::from(x.c.u16()?);
    let nbytes = usize::try_from(x.c.u32()?).map_err(|_| crate::entries())?;
    if nbytes > x.limits.max_decompressed {
        return Err(crate::entries());
    }
    x.c.skip(nbytes)?;
    let pad_to = mul_size(blocks, 4, x.limits.max_decompressed)?;
    if pad_to < nbytes {
        return Err(x.c.fail());
    }
    x.c.skip(pad_to - nbytes)?;
    Ok(Rec::None)
}

fn data_list(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let count = x.c.u16()?;
    let _total = x.c.u32()?;
    x.c.skip(4)?;
    let _ids = read_ids(x, count)?;
    Ok(Rec::None)
}

fn symbol_class(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let _name = x.c.id()?;
    let group = x.c.id()?;
    let _date = x.c.id()?;
    let _library = x.c.id()?;
    let _list = x.c.id()?;
    Ok(Rec::SymbolClass { group })
}

fn symbol_instance(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let _style = x.c.id()?;
    let _parent = x.c.id()?;
    x.c.skip(8)?;
    let class = x.c.id()?;
    let matrix = flagged_matrix(x)?;
    Ok(Rec::SymbolInstance { class, matrix })
}

fn symbol_library(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    x.c.skip(2)?;
    let count = x.c.u16()?;
    x.c.skip(8)?;
    let n = u32::from(count).saturating_add(3);
    if n > 100_000 {
        return Err(crate::entries());
    }
    for _ in 0..n {
        let _ = x.c.id()?;
    }
    Ok(Rec::None)
}

fn new_blend(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let _style = x.c.id()?;
    let _parent = x.c.id()?;
    x.c.skip(8)?;
    let lists = [x.c.id()?, x.c.id()?, x.c.id()?];
    x.c.skip(26)?;
    Ok(Rec::Blend { lists })
}

fn tab_table(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let capacity = u32::from(x.c.u16()?);
    let _count = x.c.u16()?;
    x.c.skip(mul_size(capacity, 6, x.limits.max_decompressed)?)?;
    Ok(Rec::None)
}

fn guides(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let count = u32::from(x.c.u16()?);
    let _ = x.c.id()?;
    let _ = x.c.id()?;
    x.c.skip(4)?;
    let rest = mul_size(count, 8, x.limits.max_decompressed)?.saturating_add(12);
    x.c.skip(rest)?;
    Ok(Rec::None)
}

fn perspective_grid(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let mut found = false;
    for _ in 0..4096 {
        if x.c.remaining() == 0 {
            break;
        }
        if x.c.u8()? == 0 {
            found = true;
            break;
        }
    }
    if !found {
        return Err(x.c.fail());
    }
    x.c.skip(58)?;
    Ok(Rec::None)
}

fn file_descriptor(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let _ = x.c.id()?;
    let _ = x.c.id()?;
    x.c.skip(5)?;
    let n = usize::from(x.c.u16()?);
    x.c.skip(n)?;
    Ok(Rec::None)
}

fn fw_shadow(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let _ = x.c.id()?;
    x.c.skip(2)?;
    let _ = x.c.u8()?;
    let _ = x.c.u8()?;
    let _ = x.c.fixed()?;
    x.c.skip(2)?;
    let _ = x.c.u16()?;
    let _ = x.c.fixed()?;
    x.c.skip(2)?;
    let _ = x.c.u16()?;
    Ok(Rec::None)
}

fn fw_glow(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let _ = x.c.id()?;
    x.c.skip(3)?;
    let _ = x.c.u8()?;
    let _ = x.c.fixed()?;
    x.c.skip(2)?;
    let _ = x.c.u16()?;
    let _ = x.c.fixed()?;
    let _ = x.c.fixed()?;
    Ok(Rec::None)
}

fn mdict(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    x.c.skip(2)?;
    let count = x.c.u16()?;
    x.c.skip(2)?;
    let _pairs = read_pairs(x, count)?;
    Ok(Rec::None)
}

fn mquick(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let count = u32::from(x.c.u16()?);
    let bytes = mul_size(count, 4, x.limits.max_decompressed)?.saturating_add(5);
    x.c.skip(bytes)?;
    Ok(Rec::None)
}

fn envelope(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    x.c.skip(2)?;
    let _ = x.c.id()?;
    let _ = x.c.id()?;
    x.c.skip(14)?;
    let count1 = u32::from(x.c.u16()?);
    let _ = x.c.id()?;
    x.c.skip(19)?;
    let count2 = u32::from(x.c.u16()?);
    let a = mul_size(count2, 4, x.limits.max_decompressed)?;
    let b = mul_size(count1, 27, x.limits.max_decompressed)?;
    x.c.skip(a.saturating_add(b))?;
    Ok(Rec::None)
}

fn master_matrix(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    x.c.skip(14)?;
    let f1 = x.c.u8()?;
    let f2 = x.c.u8()?;
    let n = component_count(f1, f2).saturating_mul(4);
    x.c.skip(n)?;
    x.c.skip(2)?;
    Ok(Rec::None)
}

fn multi_blend(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let count = u32::from(x.c.u16()?);
    let _ = x.c.id()?;
    x.c.skip(8)?;
    let _ = x.c.id()?;
    let _ = x.c.id()?;
    let _ = x.c.id()?;
    let tail = mul_size(count, 6, x.limits.max_decompressed)?.saturating_add(32);
    x.c.skip(tail)?;
    Ok(Rec::None)
}

fn blend_object(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let _ = x.c.id()?;
    let _ = x.c.id()?;
    x.c.skip(8)?;
    let _ = x.c.id()?;
    x.c.skip(16)?;
    Ok(Rec::None)
}

fn connector(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    x.c.skip(20)?;
    let count = u32::from(x.c.u16()?);
    let nodes = mul_size(count, 27, x.limits.max_decompressed)?;
    x.c.skip(nodes.saturating_add(46))?;
    Ok(Rec::None)
}

fn line_table(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let a = x.c.u16()?;
    let b = x.c.u16()?;
    let count = if x.version >= 10 { b } else { a };
    let n = usize::from(count);
    take_items(x, n, "line table")?;
    for _ in 0..n {
        x.c.skip(48)?;
        let _ = x.c.id()?;
    }
    Ok(Rec::None)
}

fn agd_selection(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let n = u32::from(x.c.u16()?);
    let bytes = mul_size(n, 4, x.limits.max_decompressed)?.saturating_add(6);
    x.c.skip(bytes)?;
    Ok(Rec::None)
}

fn extrusion(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let start = x.c.pos;
    let f1 = x.c.peek_at(start + 96)?;
    let f2 = x.c.peek_at(start + 97)?;
    let comp = component_count(f1, f2).saturating_mul(4);
    let total = 98usize.checked_add(comp).ok_or_else(crate::entries)?;
    x.c.skip(total)?;
    Ok(Rec::None)
}

fn path_text(x: &mut Ctx<'_, '_>) -> Result<Rec, Error> {
    let _ = x.c.id()?;
    let _ = x.c.id()?;
    x.c.skip(2)?;
    let _len = x.c.u16()?;
    x.c.skip(4)?;
    let _ = x.c.id()?;
    let _ = x.c.id()?;
    Ok(Rec::None)
}
