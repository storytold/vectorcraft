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
        let (mut name, mut divider) = (String::new(), None);
        while q < end {
            assert_eq!(&f[q..q + 4], b"8BIM");
            let len = u32_at(f, q + 8);
            let data = &f[q + 12..q + 12 + len];
            match &f[q + 4..q + 8] {
                b"luni" => name = String::from_utf16(&(0..u32_at(data, 0)).map(|i| u16_at(data, 4 + i * 2)).collect::<Vec<_>>()).unwrap(),
                b"lsct" => divider = Some(u32_at(data, 0)),
                _ => {}
            }
            q += 12 + len;
        }
        out.records.push(Rec { name, rect, channels, blend, opacity, hidden: flags & 2 != 0, divider });
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

/// The composite as a reader sees it.
fn merged(f: &[u8]) -> Vec<u8> {
    ::psd::Psd::from_bytes(f).expect("a PSD a reader takes").rgba()
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
    let psd = ::psd::Psd::from_bytes(&file).unwrap();
    assert_eq!((psd.width(), psd.height(), psd.layers().len()), (100, 60, 2));
    let red = psd.layer_by_name("Layer 1").unwrap().rgba();
    assert_eq!(pixel(&red, 100, 20, 20), [255, 0, 0, 255]);
    assert_eq!(pixel(&red, 100, 50, 45)[3], 0, "the blue square isn't on Layer 1");

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
    assert_eq!(::psd::Psd::from_bytes(&file).unwrap().width(), 208);
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
    let psd = ::psd::Psd::from_bytes(&file).unwrap();
    assert_eq!((psd.layers().len(), psd.groups().len()), (3, 3));
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
    let psd = ::psd::Psd::from_bytes(&file).unwrap();
    assert!(psd.layer_by_name("Background").unwrap().rgba().chunks(4).all(|p| p == [0, 0, 255, 255]));
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

#[test]
fn long_unicode_name_is_cut_on_a_character_boundary() {
    let mut b = DocBuilder::new(100.0, 60.0);
    b.layer(&("a".repeat(254) + "😀"));
    b.rect(Rect::new(0.0, 0.0, 5.0, 5.0), Color::rgb8(0, 0, 0), |_| {});
    let file = export(&b.build(), &opts(PsdOptions::default()));
    assert_eq!(parse(&file).records.last().unwrap().name, "a".repeat(254));
}
