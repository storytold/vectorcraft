//! PSD export: the header, the layer records (names, bounds, opacity, blend modes, groups,
//! hidden layers), the merged image, the colour models, and a reader's view of the file.

use std::sync::Arc;

use vectorcraft_color::{BlendMode, Color};
use vectorcraft_doc::{Document, LayerColor, Node};
use vectorcraft_geom::Rect;
use vectorcraft_testkit::fixtures::DocBuilder;

use super::jpeg::ColorModel;
use super::psd::{MAX_SIDE, PsdOptions};
use super::*;

/// A 100 × 60 pt document: a red square on Layer 1, and on "Top" a half-opaque blue multiply
/// square overlapping it, a hidden green one and a sublayer "Inner" holding a grey "Corner".
fn doc() -> Document {
    let mut b = DocBuilder::new(100.0, 60.0);
    b.rect(Rect::new(10.0, 10.0, 40.0, 40.0), Color::rgb8(255, 0, 0), |_| {});
    let top = b.layer("Top");
    b.rect(Rect::new(30.0, 20.0, 60.0, 50.0), Color::rgb8(0, 0, 255), |n: &mut Node| {
        n.opacity = 0.5;
        n.blend = BlendMode::Multiply;
    });
    b.rect(Rect::new(70.0, 5.0, 90.0, 25.0), Color::rgb8(0, 255, 0), |n: &mut Node| n.visible = false);
    let corner = b.rect_node(Rect::new(0.0, 0.0, 6.0, 6.0), Color::rgb8(128, 128, 128));
    let corner = Node { name: Some("Corner".into()), ..corner };
    let inner = Node::layer(b.alloc(), "Inner", LayerColor::Preset(2));
    let inner = b.doc.insert(Some(top), usize::MAX, inner).unwrap();
    b.doc.insert(Some(inner), usize::MAX, corner).unwrap();
    b.build()
}

fn opts(psd: PsdOptions) -> RasterExportOptions {
    RasterExportOptions { psd: PsdOptions { embed_icc: false, ..psd }, ..RasterExportOptions::default() }
}

fn export(doc: &Document, o: &RasterExportOptions) -> Vec<u8> {
    Renderer::new().export_region(doc, doc.artboards[0].rect, RasterFormat::Psd, o).unwrap()
}

fn u16_at(f: &[u8], at: usize) -> u16 {
    u16::from_be_bytes(f[at..at + 2].try_into().unwrap())
}

fn u32_at(f: &[u8], at: usize) -> usize {
    u32::from_be_bytes(f[at..at + 4].try_into().unwrap()) as usize
}

/// A layer record as written.
#[derive(Debug)]
struct Rec {
    name: String,
    /// Top, left, bottom, right.
    rect: [i32; 4],
    channels: usize,
    blend: [u8; 4],
    opacity: u8,
    hidden: bool,
    divider: Option<usize>,
    /// The type tool object of a type layer.
    type_tool: Option<Vec<u8>>,
}

/// The file: header channels, colour mode, image resources, layer count and records.
struct Parsed<'a> {
    channels: u16,
    mode: u16,
    resources: &'a [u8],
    count: i16,
    records: Vec<Rec>,
    /// Where the merged image starts.
    image: usize,
}

fn parse(f: &[u8]) -> Parsed<'_> {
    assert_eq!(&f[..6], b"8BPS\0\x01");
    let (channels, mode) = (u16_at(f, 12), u16_at(f, 24));
    let mut at = 26;
    at += 4 + u32_at(f, at);
    let resources = &f[at + 4..at + 4 + u32_at(f, at)];
    at += 4 + resources.len();
    let image = at + 4 + u32_at(f, at);
    let mut out = Parsed { channels, mode, resources, count: 0, records: vec![], image };
    if u32_at(f, at) == 0 {
        return out;
    }
    out.count = u16_at(f, at + 8) as i16;
    let mut p = at + 10;
    for _ in 0..out.count.unsigned_abs() {
        let rect: [i32; 4] = std::array::from_fn(|i| u32_at(f, p + i * 4) as i32);
        let channels = u16_at(f, p + 16) as usize;
        p += 18 + channels * 6;
        assert_eq!(&f[p..p + 4], b"8BIM");
        let blend: [u8; 4] = f[p + 4..p + 8].try_into().unwrap();
        let (opacity, flags) = (f[p + 8], f[p + 10]);
        let end = p + 16 + u32_at(f, p + 12);
        let mut q = p + 16 + 8;
        q += (1 + f[q] as usize).next_multiple_of(4);
        let (mut name, mut divider, mut type_tool) = (String::new(), None, None);
        while q < end {
            assert_eq!(&f[q..q + 4], b"8BIM");
            let len = u32_at(f, q + 8);
            let data = &f[q + 12..q + 12 + len];
            match &f[q + 4..q + 8] {
                b"luni" => name = String::from_utf16(&(0..u32_at(data, 0)).map(|i| u16_at(data, 4 + i * 2)).collect::<Vec<_>>()).unwrap(),
                b"lsct" => divider = Some(u32_at(data, 0)),
                b"TySh" => type_tool = Some(data.to_vec()),
                _ => {}
            }
            q += 12 + len;
        }
        out.records.push(Rec { name, rect, channels, blend, opacity, hidden: flags & 2 != 0, divider, type_tool });
        p = end;
    }
    out
}

/// The merged image's planes (`channels` planes of `w` × `h`), unpacked.
fn merged_planes(f: &[u8], channels: usize, w: usize, h: usize) -> Vec<Vec<u8>> {
    let data = &f[parse(f).image..];
    assert_eq!(u16_at(data, 0), 1, "PackBits");
    let mut q = 2 + channels * h * 2;
    (0..channels)
        .map(|c| {
            let mut plane = vec![];
            for row in 0..h {
                let n = u16_at(data, 2 + (c * h + row) * 2) as usize;
                unpack(&data[q..q + n], &mut plane);
                q += n;
            }
            assert_eq!(plane.len(), w * h);
            plane
        })
        .collect()
}

fn unpack(mut src: &[u8], out: &mut Vec<u8>) {
    while let Some((&n, rest)) = src.split_first() {
        if n < 128 {
            out.extend(&rest[..n as usize + 1]);
            src = &rest[n as usize + 1..];
        } else {
            out.extend(std::iter::repeat_n(rest[0], 257 - n as usize));
            src = &rest[1..];
        }
    }
}

/// The stored composite: RGB stays white-matted, with its original alpha.
fn merged(f: &[u8]) -> Vec<u8> {
    let psd = craft_psd::PsdFile::from_bytes(f).expect("a PSD a reader takes");
    if psd.header.color_mode != craft_psd::ColorMode::Rgb {
        return psd.composite_rgba8().expect("composite").data;
    }
    assert_eq!(psd.header.depth, 8);
    let samples = psd.decode_merged().expect("stored composite planes");
    let count = psd.header.width as usize * psd.header.height as usize;
    (0..count)
        .flat_map(|i| [samples[i], samples[count + i], samples[2 * count + i], if psd.merged_has_alpha() { samples[3 * count + i] } else { 255 }])
        .collect()
}

fn pixel(px: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
    px[(y * width + x) * 4..][..4].try_into().unwrap()
}

#[test]
fn layers_are_the_top_level_layers_and_the_merged_image_is_the_render() {
    let doc = doc();
    let file = export(&doc, &opts(PsdOptions::default()));
    let p = parse(&file);
    assert_eq!((p.channels, p.mode), (4, 3), "RGB with the merged image's transparency");
    assert_eq!(p.count, -2, "two layers; negative: the merged alpha is its transparency");
    let names: Vec<&str> = p.records.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["Layer 1", "Top"], "bottom first");
    let red = &p.records[0];
    assert_eq!(red.rect, [10, 10, 40, 40], "cropped to what the layer paints");
    assert_eq!((red.channels, red.blend, red.opacity, red.hidden, red.divider), (4, *b"norm", 255, false, None));
    assert_eq!(p.records[1].rect, [0, 0, 50, 60], "the blue square and the corner");

    // A reader decodes it: the layers, in place, and the merged image.
    let psd = craft_psd::PsdFile::from_bytes(&file).unwrap();
    assert_eq!((psd.header.width, psd.header.height, psd.layers().len()), (100, 60, 2));
    let layer = psd.iter_layers().find(|l| l.name() == "Layer 1").unwrap();
    assert_eq!(layer.record.rect, craft_psd::layer::Rect { top: 10, left: 10, bottom: 40, right: 40 });
    let red = layer.rgba8().unwrap().data;
    assert_eq!(pixel(&red, 30, 10, 10), [255, 0, 0, 255]);

    // The merged image is the flat render: colours on white where transparent, alpha apart.
    let img = Renderer::new().render_region_with(&doc, doc.artboards[0].rect, 1.0, &opts(PsdOptions::default()).render_options(RasterFormat::Psd));
    let want: Vec<u8> = img.to_straight().as_chunks::<4>().0.iter().flat_map(|p| [on_white(p), [p[3]; 3]].concat()[..4].to_vec()).collect();
    assert_eq!(merged(&file), want);
    let mixed = pixel(&want, 100, 35, 30);
    assert!(mixed[0].abs_diff(128) <= 1 && mixed[1] == 0 && mixed[2] == 0, "red under half-opaque blue multiply: {mixed:?}");
    assert_eq!(pixel(&want, 100, 5, 50), [255, 255, 255, 0]);
}

#[test]
fn flat_writes_the_merged_image_alone_on_white() {
    let doc = doc();
    let o = opts(PsdOptions { layers: false, ..PsdOptions::default() });
    let file = export(&doc, &o);
    let p = parse(&file);
    assert_eq!((p.channels, p.mode, p.count), (3, 3, 0));
    assert!(p.records.is_empty());
    // The resolution resource: 72 ppi in 16.16 fixed point.
    assert_eq!(&p.resources[..6], b"8BIM\x03\xed");
    assert_eq!(u32_at(p.resources, 12), 72 << 16);
    let img = Renderer::new().render_region_with(&doc, doc.artboards[0].rect, 1.0, &o.render_options(RasterFormat::Psd));
    assert_eq!(merged(&file), img.to_straight(), "the composite is the flat render");
    // The same flat file from a rendered image.
    assert_eq!(o.encode(&img, RasterFormat::Psd).unwrap(), file);
    let file = export(&doc, &RasterExportOptions { ppi: 150.0, ..o });
    assert_eq!(u32_at(parse(&file).resources, 12), 150 << 16);
    assert_eq!(craft_psd::PsdFile::from_bytes(&file).unwrap().header.width, 208);
}

#[test]
fn maximum_editability_writes_groups_and_an_object_per_layer() {
    let doc = doc();
    let o = opts(PsdOptions { max_editability: true, ..PsdOptions::default() });
    let file = export(&doc, &o);
    let p = parse(&file);
    let rows: Vec<(&str, Option<usize>)> = p.records.iter().map(|r| (r.name.as_str(), r.divider)).collect();
    let rect = doc.layers[0].children().unwrap()[0].display_name();
    assert_eq!(
        rows,
        [
            ("</Layer group>", Some(3)),
            (rect.as_str(), None),
            ("Layer 1", Some(1)),
            ("</Layer group>", Some(3)),
            (rect.as_str(), None),
            ("</Layer group>", Some(3)),
            ("Corner", None),
            ("Inner", Some(1)),
            ("Top", Some(1)),
        ],
        "groups end below their layers and open above them; the hidden square is left out"
    );
    // The object keeps its opacity and blending in its record; groups pass through.
    assert_eq!((p.records[4].opacity, p.records[4].blend, p.records[4].rect), (128, *b"mul ", [20, 30, 50, 60]));
    assert_eq!((p.records[8].blend, p.records[8].opacity), (*b"pass", 255));
    assert_eq!(p.records[6].rect, [0, 0, 6, 6]);
    let psd = craft_psd::PsdFile::from_bytes(&file).unwrap();
    let content_layers = psd.layers().iter().filter(|l| matches!(l.section_type(), craft_psd::SectionType::Other)).count();
    let groups =
        psd.layers().iter().filter(|l| matches!(l.section_type(), craft_psd::SectionType::OpenFolder | craft_psd::SectionType::ClosedFolder)).count();
    assert_eq!((content_layers, groups), (3, 3));
    assert_eq!(merged(&file), merged(&export(&doc, &opts(PsdOptions::default()))), "the same merged image");

    // Hidden layers: written hidden, drawn as if visible, instead of left out.
    let o = opts(PsdOptions { max_editability: true, hidden_layers: true, ..PsdOptions::default() });
    let file = export(&doc, &o);
    let p = parse(&file);
    let hidden: Vec<&Rec> = p.records.iter().filter(|r| r.hidden).collect();
    assert_eq!(hidden.len(), 1);
    assert_eq!(hidden[0].rect, [5, 70, 25, 90]);
}

#[test]
fn hidden_top_level_layers_are_optional() {
    let mut doc = doc();
    Arc::make_mut(&mut doc.layers[1]).visible = false;
    let names = |o: PsdOptions| parse(&export(&doc, &opts(o))).records.iter().map(|r| (r.name.clone(), r.hidden)).collect::<Vec<_>>();
    assert_eq!(names(PsdOptions::default()), [("Layer 1".to_string(), false)]);
    assert_eq!(names(PsdOptions { hidden_layers: true, ..PsdOptions::default() }), [("Layer 1".to_string(), false), ("Top".to_string(), true)]);

    // Nothing left to write as a layer: no transparency plane either (it would read as a channel).
    Arc::make_mut(&mut doc.layers[0]).visible = false;
    let file = export(&doc, &opts(PsdOptions::default()));
    let p = parse(&file);
    assert_eq!((p.channels, p.count), (3, 0));
}

#[test]
fn guides_are_not_layers() {
    let mut b = DocBuilder::new(20.0, 20.0);
    b.rect(Rect::new(0.0, 0.0, 5.0, 5.0), Color::rgb8(0, 0, 0), |_| {});
    b.rect(Rect::new(0.0, 10.0, 20.0, 10.0), Color::rgb8(0, 0, 0), |n: &mut Node| {
        if let vectorcraft_doc::NodeKind::Path { guide, .. } = &mut n.kind {
            *guide = true;
        }
    });
    let o = opts(PsdOptions { max_editability: true, ..PsdOptions::default() });
    assert_eq!(parse(&export(&b.build(), &o)).count, -3, "the group and the square");
}

/// Ruler guides on a layer and a sublayer change nothing in the export: the same file with and
/// without Maximum Editability, the same drawn bounds and the same cached size of each layer.
#[test]
fn ruler_guides_are_not_exported() {
    use vectorcraft_doc::{NodeId, NodeKind};
    let plain = doc();
    let mut guided = plain.clone();
    let guide = |id, vertical, pos| Node::new(NodeId(id), NodeKind::RulerGuide { vertical, pos, artboard: None });
    let (bottom, inner) = (guided.layers[0].id, guided.layers[1].children().unwrap().last().unwrap().id);
    guided.insert(Some(bottom), usize::MAX, guide(900, true, 20.0)).unwrap();
    guided.insert(Some(bottom), usize::MAX, guide(901, false, 15.0)).unwrap();
    guided.insert(Some(inner), usize::MAX, guide(902, true, 3.0)).unwrap();
    for psd in [PsdOptions::default(), PsdOptions { max_editability: true, ..PsdOptions::default() }] {
        assert_eq!(export(&guided, &opts(psd.clone())), export(&plain, &opts(psd)));
    }
    assert_eq!(art_bounds(&guided), art_bounds(&plain));
    for (g, p) in guided.layers.iter().zip(&plain.layers) {
        assert_eq!(crate::drawn_bounds(&guided, g), crate::drawn_bounds(&plain, p));
        assert_eq!(crate::lru::node_bytes(g), crate::lru::node_bytes(p));
    }
}

#[test]
fn a_background_colour_is_the_bottom_layer() {
    let doc = doc();
    let file = export(&doc, &RasterExportOptions { background: Some([0, 0, 255]), ..opts(PsdOptions::default()) });
    let p = parse(&file);
    assert_eq!((p.channels, p.count), (3, 3), "opaque: no merged alpha");
    assert_eq!((p.records[0].name.as_str(), p.records[0].rect), ("Background", [0, 0, 60, 100]));
    let psd = craft_psd::PsdFile::from_bytes(&file).unwrap();
    assert!(psd.iter_layers().find(|l| l.name() == "Background").unwrap().rgba8().unwrap().data.chunks(4).all(|p| p == [0, 0, 255, 255]));
    assert_eq!(pixel(&merged(&file), 100, 5, 50), [0, 0, 255, 255]);
}

#[test]
fn cmyk_planes_hold_the_inks_inverted_and_gray_is_one_plane() {
    let doc = doc();
    let cmyk = export(&doc, &opts(PsdOptions { color_model: ColorModel::Cmyk, ..PsdOptions::default() }));
    let p = parse(&cmyk);
    assert_eq!((p.channels, p.mode, p.count, p.records[0].channels), (5, 4, -2, 5));

    let gray = export(&doc, &opts(PsdOptions { color_model: ColorModel::Gray, layers: false, ..PsdOptions::default() }));
    let p = parse(&gray);
    assert_eq!((p.channels, p.mode), (1, 1));
    let px = merged(&gray);
    assert_eq!(pixel(&px, 100, 5, 50), [255, 255, 255, 255], "white paper");
    assert!((100..160).contains(&px[(20 * 100 + 20) * 4]), "red is a middle grey");

    // A CMYK black is black ink alone; no ink is 255.
    let mut b = DocBuilder::new(10.0, 10.0);
    b.rect(Rect::new(0.0, 0.0, 5.0, 10.0), Color::cmyk(0.0, 0.0, 0.0, 1.0), |_| {});
    let doc = b.build();
    let file = export(&doc, &opts(PsdOptions { color_model: ColorModel::Cmyk, layers: false, ..PsdOptions::default() }));
    let planes = merged_planes(&file, 4, 10, 10);
    assert_eq!(planes.iter().map(|p| p[0]).collect::<Vec<_>>(), [255, 255, 255, 0], "black ink");
    assert_eq!(planes.iter().map(|p| p[9]).collect::<Vec<_>>(), [255; 4], "no ink");
    // Layered: the layer's inks are the same under its alpha.
    let file = export(&doc, &opts(PsdOptions { color_model: ColorModel::Cmyk, ..PsdOptions::default() }));
    assert_eq!(merged_planes(&file, 5, 10, 10)[3][0], 0);
}

#[test]
fn the_profile_and_the_size_limit() {
    let doc = doc();
    let file = export(&doc, &RasterExportOptions::default());
    let resources = parse(&file).resources;
    let icc = resources.windows(6).position(|w| w == b"8BIM\x04\x0f").expect("the profile resource");
    assert!(u32_at(resources, icc + 8) > 100);
    let o = RasterExportOptions { ppi: 72.0 * 301.0, ..RasterExportOptions::default() };
    let err = Renderer::new().export_region(&doc, doc.artboards[0].rect, RasterFormat::Psd, &o).unwrap_err();
    assert!(err.contains(&MAX_SIDE.to_string()), "{err}");
}

/// A descriptor value as the format's specification lays it out.
#[derive(Debug, PartialEq)]
enum Item {
    Text(String),
    Enum(String, String),
    Long(i32),
    Doub(f64),
    Unit(String, f64),
    Object(String, Vec<(String, Item)>),
    Raw(Vec<u8>),
}

/// Reads an id (a four-character code after a zero length, else a length and its bytes) at `at`.
fn read_id(d: &[u8], at: &mut usize) -> String {
    let n = u32_at(d, *at);
    let len = if n == 0 { 4 } else { n };
    let s = String::from_utf8(d[*at + 4..*at + 4 + len].to_vec()).unwrap();
    *at += 4 + len;
    s
}

fn read_unicode(d: &[u8], at: &mut usize) -> String {
    let n = u32_at(d, *at);
    let units: Vec<u16> = (0..n).map(|i| u16_at(d, *at + 4 + i * 2)).collect();
    *at += 4 + n * 2;
    String::from_utf16(&units).unwrap().trim_end_matches('\0').to_string()
}

/// Reads a descriptor at `at`: its class and items.
fn read_descriptor(d: &[u8], at: &mut usize) -> (String, Vec<(String, Item)>) {
    read_unicode(d, at);
    let class = read_id(d, at);
    let count = u32_at(d, *at);
    *at += 4;
    let mut items = vec![];
    for _ in 0..count {
        let key = read_id(d, at);
        let ty = d[*at..*at + 4].to_vec();
        *at += 4;
        let f64_at = |at: usize| f64::from_be_bytes(d[at..at + 8].try_into().unwrap());
        let v = match &ty[..] {
            b"TEXT" => Item::Text(read_unicode(d, at)),
            b"enum" => Item::Enum(read_id(d, at), read_id(d, at)),
            b"long" => {
                *at += 4;
                Item::Long(u32_at(d, *at - 4) as i32)
            }
            b"doub" => {
                *at += 8;
                Item::Doub(f64_at(*at - 8))
            }
            b"UntF" => {
                let unit = String::from_utf8(d[*at..*at + 4].to_vec()).unwrap();
                *at += 12;
                Item::Unit(unit, f64_at(*at - 8))
            }
            b"Objc" => {
                let (class, items) = read_descriptor(d, at);
                Item::Object(class, items)
            }
            b"tdta" => {
                let n = u32_at(d, *at);
                *at += 4 + n;
                Item::Raw(d[*at - n..*at].to_vec())
            }
            other => panic!("unexpected item type {other:?}"),
        };
        items.push((key, v));
    }
    (class, items)
}

/// A type tool object: its transform, its text descriptor and its warp descriptor.
struct TypeTool {
    transform: [f64; 6],
    text: Vec<(String, Item)>,
    warp: (String, Vec<(String, Item)>),
}

fn type_tool(d: &[u8]) -> TypeTool {
    assert_eq!(u16_at(d, 0), 1, "version");
    let transform = std::array::from_fn(|i| f64::from_be_bytes(d[2 + i * 8..10 + i * 8].try_into().unwrap()));
    assert_eq!((u16_at(d, 50), u32_at(d, 52)), (50, 16), "text and descriptor versions");
    let mut at = 56;
    let (class, text) = read_descriptor(d, &mut at);
    assert_eq!(class, "TxLr");
    assert_eq!((u16_at(d, at), u32_at(d, at + 2)), (1, 16), "warp and descriptor versions");
    at += 6;
    let warp = read_descriptor(d, &mut at);
    assert_eq!(d.len(), (at + 16).next_multiple_of(2), "the four warped bounds end the block (padded to an even length)");
    TypeTool { transform, text, warp }
}

fn item<'a>(items: &'a [(String, Item)], key: &str) -> &'a Item {
    &items.iter().find(|(k, _)| k == key).unwrap_or_else(|| panic!("no {key}")).1
}

/// `s` as the text engine stores a string: UTF-16BE after a byte order mark.
fn engine_string(s: &str) -> Vec<u8> {
    [0xFE, 0xFF].into_iter().chain(s.encode_utf16().flat_map(u16::to_be_bytes)).collect()
}

fn type_node(b: &mut DocBuilder, origin: (f64, f64), text: &str, f: impl FnOnce(&mut vectorcraft_doc::TextObject)) -> Node {
    use vectorcraft_doc::{CharStyle, NodeKind, TextObject};
    let style = CharStyle { size: 12.0, fill: vectorcraft_color::Paint::solid(Color::rgb8(0, 0, 0)), ..CharStyle::default() };
    let mut t = TextObject::point(vectorcraft_geom::Point::new(origin.0, origin.1), text, style);
    f(&mut t);
    Node::new(b.alloc(), NodeKind::Text(Box::new(t)))
}

/// Layer 1: a red square, point type, a blue square.
fn type_doc() -> Document {
    let mut b = DocBuilder::new(100.0, 60.0);
    b.rect(Rect::new(0.0, 0.0, 30.0, 30.0), Color::rgb8(255, 0, 0), |_| {});
    let t = type_node(&mut b, (20.0, 40.0), "Hi\nyou", |_| {});
    b.add(t);
    b.rect(Rect::new(60.0, 10.0, 90.0, 40.0), Color::rgb8(0, 0, 255), |_| {});
    b.build()
}

#[test]
fn preserve_text_writes_point_type_as_a_type_layer_between_pixel_layers() {
    let doc = type_doc();
    let o = RasterExportOptions { ppi: 144.0, ..opts(PsdOptions { preserve_text: true, ..PsdOptions::default() }) };
    let file = export(&doc, &o);
    let p = parse(&file);
    let rows: Vec<(&str, Option<usize>, bool)> = p.records.iter().map(|r| (r.name.as_str(), r.divider, r.type_tool.is_some())).collect();
    assert_eq!(
        rows,
        [
            ("</Layer group>", Some(3), false),
            ("Layer 1", None, false),
            ("Hi\nyou", None, true),
            ("Layer 1", None, false),
            ("Layer 1", Some(1), false),
        ],
        "the layer becomes a group: the art below the type, the type layer, the art above it"
    );
    assert_eq!(p.records[1].rect, [0, 0, 60, 60], "the red square alone, at 2 pixels a point");
    assert_eq!(p.records[3].rect, [20, 120, 80, 180], "the blue square alone");
    assert_ne!(p.records[2].rect, [0; 4], "the type layer keeps its pixels");

    let t = type_tool(p.records[2].type_tool.as_deref().unwrap());
    assert_eq!(t.transform, [1.0, 0.0, 0.0, 1.0, 40.0, 80.0], "the first baseline's start, in pixels");
    assert_eq!(item(&t.text, "Txt "), &Item::Text("Hi\ryou".into()));
    assert_eq!(item(&t.text, "Ornt"), &Item::Enum("Ornt".into(), "Hrzn".into()));
    let Item::Object(_, bounds) = item(&t.text, "bounds") else { panic!("bounds") };
    let [l, top, r, bottom] = ["Left", "Top ", "Rght", "Btom"].map(|k| match item(bounds, k) {
        Item::Unit(_, v) => *v,
        other => panic!("{other:?}"),
    });
    assert!(l < 4.0 && top < 0.0 && r > l && bottom > 24.0, "the ink around the baselines: {l} {top} {r} {bottom}");
    assert_eq!(t.warp.0, "warp");
    assert_eq!(item(&t.warp.1, "warpStyle"), &Item::Enum("warpStyle".into(), "warpNone".into()));

    let Item::Raw(engine) = item(&t.text, "EngineData") else { panic!("EngineData") };
    let has = |needle: &[u8]| engine.windows(needle.len()).any(|w| w == needle);
    assert!(has(&engine_string("Hi\ryou\r")), "the text ends with a paragraph break");
    assert!(has(b"/FontSize 24.0"), "the size in pixels at 144 ppi");
    assert!(has(b"/RunLengthArray [ 3 4 ]"), "a paragraph run per paragraph, breaks counted");
    assert!(has(b"/RunLengthArray [ 7 ]"), "one style run over the text and the closing break");
    assert!(has(b"/Values [ 1.0 0.0 0.0 0.0 ]"), "black");
    assert!(has(b"/ShapeType 0"), "point type");
    assert!(has(b"/UseFractionalGlyphWidths true"), "Fractional Widths on");

    // A reader takes the file, and the merged image is the same render.
    assert_eq!(craft_psd::PsdFile::from_bytes(&file).unwrap().layers().len(), p.records.len());
    let plain = export(&doc, &RasterExportOptions { ppi: 144.0, ..opts(PsdOptions::default()) });
    assert_eq!(merged(&file), merged(&plain));
    assert_eq!(parse(&plain).records.iter().filter(|r| r.type_tool.is_some()).count(), 0, "off: no type layers");
}

#[test]
fn type_a_type_layer_cant_draw_stays_pixels() {
    use vectorcraft_doc::{NodeKind, TextKind};
    use vectorcraft_geom::PathData;
    let mut b = DocBuilder::new(100.0, 60.0);
    let area = type_node(&mut b, (10.0, 20.0), "Area", |t| {
        t.kind = TextKind::Area { frame: PathData::from_bezpath(&vectorcraft_geom::Shape::to_path(&Rect::new(5.0, 5.0, 60.0, 50.0), 0.1)) };
    });
    b.add(area);
    let stroked = type_node(&mut b, (10.0, 50.0), "Stroked", |t| {
        t.runs[0].style.stroke = vectorcraft_color::Paint::solid(Color::rgb8(255, 0, 0));
        t.runs[0].style.stroke_width = 1.0;
    });
    b.add(stroked);
    let kerned = type_node(&mut b, (40.0, 50.0), "Kerned", |t| t.runs[0].style.kerning = Some(50.0));
    b.add(kerned);
    let indented = type_node(&mut b, (70.0, 50.0), "Indented", |t| t.para.first_line_indent = 4.0);
    b.add(indented);
    let listed = type_node(&mut b, (10.0, 35.0), "Listed", |t| t.para.list = Some(vectorcraft_doc::text::ParaList::default()));
    b.add(listed);
    let system = type_node(&mut b, (40.0, 35.0), "System", |t| t.system_layout = true);
    b.add(system);
    let hanging = type_node(&mut b, (70.0, 35.0), "\"Hung\"", |t| t.optical_margin = true);
    b.add(hanging);
    let tabbed = type_node(&mut b, (10.0, 10.0), "a\tb", |t| {
        t.para.tabs = vec![vectorcraft_doc::TabStop { position: 30.0, align: Default::default(), leader: String::new(), align_on: '.' }];
    });
    b.add(tabbed);
    let leading = type_node(&mut b, (40.0, 10.0), "Auto", |t| t.para.justification.auto_leading = 150.0);
    b.add(leading);
    let standard = type_node(&mut b, (40.0, 20.0), "Std", |t| t.para.composer = vectorcraft_doc::Composer::StandardEveryLine);
    b.add(standard);
    // Hebrew: right to left from its first strong character, so Auto alignment is right. In an
    // installed font that covers it when there is one; otherwise its font lacks it.
    let hebrew_face = vectorcraft_text::FontDb::global().face_covering('ש');
    let hebrew = type_node(&mut b, (70.0, 10.0), "שלום", |t| {
        if let Some(f) = &hebrew_face {
            (t.runs[0].style.font_family, t.runs[0].style.font_style) = (f.family.clone(), f.style.clone());
        }
    });
    b.add(hebrew);
    let tracked = type_node(&mut b, (10.0, 58.0), "Tracked", |t| t.runs[0].style.tracking = 0.5);
    b.add(tracked);
    let british = type_node(&mut b, (70.0, 58.0), "Colour", |t| t.runs[0].style.language = Some("en-GB".into()));
    b.add(british);
    // Drawn partly with a fallback font: the bundled fonts have no 縦.
    let fallback = type_node(&mut b, (40.0, 58.0), "A縦", |_| {});
    b.add(fallback);
    let mut doc = b.build();
    let with = |o: PsdOptions| export(&doc, &opts(o));
    assert_eq!(with(PsdOptions { preserve_text: true, ..PsdOptions::default() }), with(PsdOptions::default()), "one pixel layer either way");

    // An empty point type object has no text to edit.
    let empty = Node::new(
        doc.alloc_id(),
        NodeKind::Text(Box::new(vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::ZERO, "", Default::default()))),
    );
    doc.insert(Some(doc.layers[0].id), usize::MAX, empty).unwrap();
    let file = export(&doc, &opts(PsdOptions { preserve_text: true, ..PsdOptions::default() }));
    assert!(parse(&file).records.iter().all(|r| r.type_tool.is_none()));
}

#[test]
fn vertical_type_and_maximum_editability() {
    let mut b = DocBuilder::new(100.0, 60.0);
    b.rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::rgb8(255, 0, 0), |_| {});
    let t = type_node(&mut b, (50.0, 10.0), "Up", |t| t.vertical = true);
    b.add(t);
    let hidden = type_node(&mut b, (10.0, 50.0), "Hidden", |_| {});
    b.add(Node { visible: false, ..hidden });
    let doc = b.build();
    let o = opts(PsdOptions { preserve_text: true, max_editability: true, hidden_layers: true, ..PsdOptions::default() });
    let file = export(&doc, &o);
    let p = parse(&file);
    let rows: Vec<(&str, bool, bool)> = p.records.iter().map(|r| (r.name.as_str(), r.type_tool.is_some(), r.hidden)).collect();
    let square = doc.layers[0].children().unwrap()[0].display_name();
    assert_eq!(
        rows,
        [("</Layer group>", false, false), (square.as_str(), false, false), ("Up", true, false), ("Hidden", true, true), ("Layer 1", false, false)],
        "each object a layer of its own; hidden type is a hidden type layer"
    );
    let t = type_tool(p.records[2].type_tool.as_deref().unwrap());
    assert_eq!(item(&t.text, "Ornt"), &Item::Enum("Ornt".into(), "Vrtc".into()));
    let Item::Raw(engine) = item(&t.text, "EngineData") else { panic!("EngineData") };
    assert!(engine.windows(19).any(|w| w == b"/WritingDirection 2"), "vertical writing");
}

#[test]
fn type_layer_keeps_fractional_widths_off() {
    let mut b = DocBuilder::new(100.0, 60.0);
    let t = type_node(&mut b, (10.0, 30.0), "Whole", |t| t.fractional_widths = false);
    b.add(t);
    let file = export(&b.build(), &opts(PsdOptions { preserve_text: true, ..PsdOptions::default() }));
    let p = parse(&file);
    let record = p.records.iter().find(|r| r.type_tool.is_some()).expect("a type layer");
    let t = type_tool(record.type_tool.as_deref().unwrap());
    let Item::Raw(engine) = item(&t.text, "EngineData") else { panic!("EngineData") };
    let needle = b"/UseFractionalGlyphWidths false";
    assert!(engine.windows(needle.len()).any(|w| w == needle), "whole-point advances, as the pixels are drawn");
}

#[test]
fn type_layer_keeps_the_anti_aliasing_method() {
    use vectorcraft_doc::text::TypeAntiAlias;
    let engine_anti_alias = |doc: &Document, o: &RasterExportOptions| {
        let file = export(doc, o);
        let p = parse(&file);
        let t = type_tool(p.records.iter().find_map(|r| r.type_tool.as_deref()).expect("a type layer"));
        let Item::Raw(engine) = item(&t.text, "EngineData") else { panic!("EngineData") };
        let at = engine.windows(11).position(|w| w == b"/AntiAlias ").expect("AntiAlias");
        let Item::Enum(_, method) = item(&t.text, "AntA") else { panic!("AntA") };
        (method.clone(), engine[at + 11])
    };
    let o = opts(PsdOptions { preserve_text: true, ..PsdOptions::default() });
    for (aa, key, number) in [
        (TypeAntiAlias::None, "Anno", b'0'),
        (TypeAntiAlias::Sharp, "AnSh", b'4'),
        (TypeAntiAlias::Crisp, "AnCr", b'1'),
        (TypeAntiAlias::Strong, "AnSt", b'2'),
    ] {
        let mut b = DocBuilder::new(100.0, 60.0);
        let t = type_node(&mut b, (10.0, 30.0), "Edges", |t| t.anti_alias = aa);
        b.add(t);
        let doc = b.build();
        assert_eq!(engine_anti_alias(&doc, &o), (key.to_string(), number), "{aa:?}");
        let hard = RasterExportOptions { anti_alias: crate::AntiAlias::None, ..o.clone() };
        assert_eq!(engine_anti_alias(&doc, &hard), ("Anno".to_string(), b'0'), "{aa:?} exported without anti-aliasing");
    }
}

#[test]
fn variation_selectors_and_joiners_need_no_glyph() {
    let mut b = DocBuilder::new(100.0, 60.0);
    // In English: USA, the default language, given by its tag.
    let t = type_node(&mut b, (10.0, 30.0), "A\u{FE0F}\u{200D}B\u{E0100}", |t| t.runs[0].style.language = Some("en-US".into()));
    b.add(t);
    let file = export(&b.build(), &opts(PsdOptions { preserve_text: true, ..PsdOptions::default() }));
    assert_eq!(parse(&file).records.iter().filter(|r| r.type_tool.is_some()).count(), 1);
}

#[test]
fn hidden_type_left_out_keeps_its_layer_whole() {
    let mut b = DocBuilder::new(100.0, 60.0);
    b.rect(Rect::new(0.0, 0.0, 30.0, 30.0), Color::rgb8(255, 0, 0), |_| {});
    let t = type_node(&mut b, (20.0, 40.0), "Hidden", |_| {});
    b.add(Node { visible: false, ..t });
    b.rect(Rect::new(60.0, 10.0, 90.0, 40.0), Color::rgb8(0, 0, 255), |_| {});
    let doc = b.build();
    let with = |o: PsdOptions| export(&doc, &opts(o));
    assert_eq!(with(PsdOptions { preserve_text: true, ..PsdOptions::default() }), with(PsdOptions::default()), "one pixel layer either way");
    let shown = parse(&with(PsdOptions { preserve_text: true, hidden_layers: true, ..PsdOptions::default() }))
        .records
        .iter()
        .filter(|r| r.type_tool.is_some() && r.hidden)
        .count();
    assert_eq!(shown, 1, "with hidden layers: a hidden type layer");
}

#[test]
fn blending_art_keeps_its_layer_whole() {
    use vectorcraft_color::BlendMode;
    use vectorcraft_doc::AppearanceItem;
    // The square's blend mode, on the object (`fill`: false) or on its fill.
    let build = |blend: BlendMode, above: bool, fill: bool| {
        let mut b = DocBuilder::new(100.0, 60.0);
        let square = |b: &mut DocBuilder| {
            b.rect(Rect::new(0.0, 0.0, 60.0, 40.0), Color::rgb8(255, 0, 0), |n| {
                if !fill {
                    n.blend = blend;
                } else if let Some(AppearanceItem::Fill(f)) = n.appearance.items.first_mut() {
                    f.blend = blend;
                }
            });
        };
        if !above {
            square(&mut b);
        }
        let t = type_node(&mut b, (10.0, 30.0), "Type", |_| {});
        b.add(t);
        if above {
            square(&mut b);
        }
        b.build()
    };
    let o = opts(PsdOptions { preserve_text: true, ..PsdOptions::default() });
    for (above, fill) in [(true, false), (false, false), (true, true)] {
        let normal = export(&build(BlendMode::Normal, above, fill), &o);
        assert_eq!(parse(&normal).records.iter().filter(|r| r.type_tool.is_some()).count(), 1, "normal art (above: {above}): a type layer");
        let doc = build(BlendMode::Multiply, above, fill);
        let file = export(&doc, &o);
        assert!(parse(&file).records.iter().all(|r| r.type_tool.is_none()), "multiplied art (above: {above}, fill: {fill}): one pixel layer");
        assert_eq!(file, export(&doc, &opts(PsdOptions::default())));
    }
}

#[test]
fn long_unicode_name_is_cut_on_a_character_boundary() {
    let mut b = DocBuilder::new(100.0, 60.0);
    b.layer(&("a".repeat(254) + "😀"));
    b.rect(Rect::new(0.0, 0.0, 5.0, 5.0), Color::rgb8(0, 0, 0), |_| {});
    let file = export(&b.build(), &opts(PsdOptions::default()));
    assert_eq!(parse(&file).records.last().unwrap().name, "a".repeat(254));
}
