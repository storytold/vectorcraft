//! EPS output read back: the DSC header and bounding box, the page's operators, gradients at both
//! levels, images, colours, the preview header and TIFF, and the data the file carries.

use std::sync::Arc;

use vectorcraft_color::{Color, Gradient, GradientKind, GradientPaint, GradientStop, Paint, Swatch};
use vectorcraft_doc::{Appearance, AppearanceItem, Dash, Document, ImageBlob, ImageObject, LineCap, LineJoin, Node, NodeKind};
use vectorcraft_geom::{Affine, PathData, Point, Rect, shapes};

use crate::*;

/// A 200 × 200 pt document whose first layer holds what `nodes` makes.
fn doc_with(nodes: impl FnOnce(&mut Document) -> Vec<Node>) -> Document {
    let mut d = Document::new(200.0, 200.0);
    let nodes = nodes(&mut d);
    let layer = d.layers[0].id;
    for n in nodes {
        d.insert(Some(layer), usize::MAX, n).unwrap();
    }
    d
}

fn path(d: &mut Document, p: PathData, fill: Paint, stroke: Paint, width: f64) -> Node {
    Node::path(d.alloc_id(), p, Appearance::basic(fill, stroke, width))
}

fn red_rect(d: &mut Document) -> Node {
    path(d, shapes::rectangle(Rect::new(50.0, 60.0, 150.0, 100.0)), Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0)
}

/// The first artboard as the region, its bottom-left corner the origin; no preview.
fn opts(d: &Document) -> EpsOptions {
    let r = d.artboards[0].rect;
    EpsOptions { region: r, origin: Point::new(r.x0, r.y1), preview: Preview::None, ..EpsOptions::default() }
}

fn text(out: &EpsOutput) -> String {
    String::from_utf8(sections(&out.bytes).unwrap().0.to_vec()).unwrap()
}

fn page_of(d: &Document, o: &EpsOptions) -> String {
    checked_page(&text(&export(d, o, None, None).unwrap()))
}

/// The value of DSC comment `key` (`%%Key:`).
fn dsc(ps: &str, key: &str) -> Option<String> {
    let prefix = format!("%%{key}:");
    ps.lines().find_map(|l| l.strip_prefix(&prefix)).map(|v| v.trim().to_string())
}

/// The program, checked for balanced procedures, arrays, dictionaries, strings and graphics
/// state saves (the syntax a PostScript interpreter would reject first); image data and comments
/// are skipped.
fn checked_page(ps: &str) -> String {
    let mut code = String::new();
    let mut lines = ps.lines();
    while let Some(l) = lines.next() {
        if !l.starts_with('%') {
            code.push_str(l);
            code.push('\n');
        }
        // Image data follows the procedure that reads it, up to the ASCII85 end marker.
        if l.ends_with("} exec") {
            assert!(lines.any(|d| d.contains("~>")), "image data ends");
        }
    }
    let mut stack = vec![];
    let mut chars = code.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '(' => {
                let mut depth = 1;
                while depth > 0 {
                    match chars.next().expect("string closes") {
                        '\\' => {
                            chars.next();
                        }
                        '(' => depth += 1,
                        ')' => depth -= 1,
                        _ => {}
                    }
                }
            }
            '{' | '[' => stack.push(c),
            '<' if chars.peek() == Some(&'<') => {
                chars.next();
                stack.push('<');
            }
            '>' if chars.peek() == Some(&'>') => {
                chars.next();
                assert_eq!(stack.pop(), Some('<'), "dictionary closes");
            }
            '}' => assert_eq!(stack.pop(), Some('{'), "procedure closes"),
            ']' => assert_eq!(stack.pop(), Some('['), "array closes"),
            _ => {}
        }
    }
    assert!(stack.is_empty(), "unclosed: {stack:?}");
    let count = |op: &str| code.split_whitespace().filter(|t| *t == op).count();
    assert_eq!(count("q"), count("Q"), "gsave and grestore pair up");
    code
}

#[test]
fn the_header_follows_dsc_and_the_file_ends_with_eof() {
    let d = doc_with(|d| vec![red_rect(d)]);
    let out = export(&d, &EpsOptions { title: "Poster (draft)".into(), created: Some(0), ..opts(&d) }, None, None).unwrap();
    let ps = text(&out);
    assert!(ps.starts_with("%!PS-Adobe-3.0 EPSF-3.0\n"), "{ps}");
    assert!(dsc(&ps, "Creator").unwrap().starts_with("VectorCraft"));
    assert_eq!(dsc(&ps, "Title").as_deref(), Some("(Poster \\(draft\\))"));
    assert_eq!(dsc(&ps, "CreationDate").as_deref(), Some("(1970-01-01 00:00:00 UTC)"));
    assert_eq!(dsc(&ps, "LanguageLevel").as_deref(), Some("3"));
    assert_eq!(dsc(&ps, "DocumentData").as_deref(), Some("Clean7Bit"));
    assert_eq!(dsc(&ps, "Pages").as_deref(), Some("1"));
    let order = ["%%EndComments", "%%BeginProlog", "%%EndProlog", "%%BeginSetup", "%%EndSetup", "%%Page: 1 1", "showpage", "%%Trailer", "%%EOF"];
    let at: Vec<usize> = order.iter().map(|m| ps.find(m).unwrap_or_else(|| panic!("{m}"))).collect();
    assert!(at.windows(2).all(|w| w[0] < w[1]), "sections in order");
    assert!(ps.ends_with("%%EOF\n"));
    assert!(ps.is_ascii() && ps.lines().all(|l| l.len() <= 255), "7-bit lines DSC readers take");
    let page = checked_page(&ps);
    assert!(page.contains("1 0 0 rg\n50 60 m\n150 60 l\n150 100 l\n50 100 l\n50 60 l\nh\nf\n"), "red, in document space: {page}");
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
}

#[test]
fn the_bounding_box_is_y_up_from_the_origin() {
    let d = doc_with(|d| vec![red_rect(d)]);
    // The art's bounds on a 200 pt artboard: 60..100 down from the top is 100..140 up from the bottom.
    let art = Rect::new(50.0, 60.0, 150.5, 100.0);
    let out = export(&d, &EpsOptions { region: art, ..opts(&d) }, None, None).unwrap();
    let ps = text(&out);
    assert_eq!(dsc(&ps, "BoundingBox").as_deref(), Some("50 100 151 140"));
    assert_eq!(dsc(&ps, "HiResBoundingBox").as_deref(), Some("50 100 150.5 140"));
    // The page maps document space onto it: y flipped about the artboard's bottom edge.
    assert!(ps.contains("[1 0 0 -1 0 200] cm"), "{ps}");
    assert_eq!(bounding_box(art, Point::new(0.0, 200.0)), [50, 100, 151, 140]);
    assert_eq!(preview_rect(art, Point::new(0.0, 200.0)), Rect::new(50.0, 60.0, 151.0, 100.0));
    // An artboard of its own: the origin at its bottom-left corner.
    let ab = Rect::new(300.0, -50.0, 400.0, 50.0);
    assert_eq!(bounding_box(ab, Point::new(300.0, 50.0)), [0, 0, 100, 100]);
    // Objects off the region are left out.
    let off = page_of(&d, &EpsOptions { region: ab, origin: Point::new(300.0, 50.0), ..opts(&d) });
    assert!(off.contains("[1 0 0 -1 -300 50] cm") && !off.contains(" rg\n"), "nothing drawn: {off}");
}

fn gradient_doc(kind: GradientKind) -> Document {
    let stops = vec![
        GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)),
        GradientStop::new(0.4, Color::rgb(1.0, 1.0, 0.0)),
        GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0)),
    ];
    let paint = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(kind, stops))));
    doc_with(|d| vec![path(d, shapes::rectangle(Rect::new(20.0, 20.0, 180.0, 120.0)), paint, Paint::None, 0.0)])
}

#[test]
fn gradients_are_smooth_shadings_at_level_3_and_stepped_fills_at_level_2() {
    let linear = gradient_doc(GradientKind::Linear);
    let page = page_of(&linear, &opts(&linear));
    assert!(page.contains("/ShadingType 2 /ColorSpace /DeviceRGB"), "{page}");
    assert!(page.contains("/FunctionType 3") && page.contains("/Bounds [0.4 ]") && page.contains("/Encode [0 1 0 1 ]"), "{page}");
    assert!(page.contains("/C0 [1 0 0] /C1 [1 1 0]") && page.contains("/C0 [1 1 0] /C1 [0 0 1]"), "{page}");
    let clip = page.find("W\n").unwrap();
    assert!(page[clip..].contains("shfill"), "clipped to the shape: {page}");
    let radial = gradient_doc(GradientKind::Radial);
    assert!(page_of(&radial, &opts(&radial)).contains("/ShadingType 3"));

    for (d, o) in [
        (&linear, EpsOptions { level: Level::Two, ..opts(&linear) }),
        (&radial, EpsOptions { level: Level::Two, ..opts(&radial) }),
        (&linear, EpsOptions { compatible_gradients: true, ..opts(&linear) }),
    ] {
        let page = page_of(d, &o);
        assert!(!page.contains("shfill") && !page.contains("ShadingType"), "{page}");
        let bands = page.matches("rectfill").count() + page.matches("arc fill").count();
        assert!(bands > 100, "a band per colour step: {bands}");
        assert!(
            page.contains("0 0 1 rg")
                && page.contains(
                    " 0 rg
"
                ),
            "from red through yellow to blue"
        );
    }
    assert!(page_of(&radial, &EpsOptions { level: Level::Two, ..opts(&radial) }).contains("0 360 newpath arc fill"));
    let ps = text(&export(&linear, &EpsOptions { level: Level::Two, ..opts(&linear) }, None, None).unwrap());
    assert_eq!(dsc(&ps, "LanguageLevel").as_deref(), Some("2"));
}

#[test]
fn strokes_keep_their_caps_joins_and_dashes() {
    let d = doc_with(|d| {
        let mut n = path(d, shapes::rectangle(Rect::new(20.0, 20.0, 80.0, 80.0)), Paint::None, Paint::solid(Color::BLACK), 4.0);
        for i in &mut n.appearance.items {
            if let AppearanceItem::Stroke(s) = i {
                s.cap = LineCap::Round;
                s.join = LineJoin::Bevel;
                s.dash = Some(Dash { pattern: vec![6.0, 2.0], offset: 1.0, ..Dash::default() });
            }
        }
        vec![n]
    });
    let page = page_of(&d, &opts(&d));
    assert!(page.contains("4 w 1 J 2 j 10 M [6 2 ] 1 d\n"), "{page}");
    assert!(page.contains("0 0 0 rg\n20 20 m\n80 20 l\n80 80 l\n20 80 l\n20 20 l\nh\nS\n"), "{page}");
}

#[test]
fn transparency_is_written_opaque_with_a_warning() {
    let d = doc_with(|d| {
        let mut n = red_rect(d);
        n.opacity = 0.5;
        vec![n]
    });
    let out = export(&d, &opts(&d), None, None).unwrap();
    assert!(text(&out).contains("1 0 0 rg"), "drawn, opaque");
    assert!(out.warnings.iter().any(|w| w.contains("written opaque")), "{:?}", out.warnings);
}

#[test]
fn spot_colours_are_separations_and_overprints_are_kept_or_dropped() {
    let mut d = doc_with(|_| vec![]);
    let gold = Color::cmyk(0.0, 0.2, 0.8, 0.1);
    d.swatches.push(Swatch { name: "Gold Ink".into(), paint: Paint::solid(gold), global: true, spot: true });
    let mut n = red_rect(&mut d);
    if let Some(AppearanceItem::Fill(f)) = n.appearance.items.first_mut() {
        f.paint = Paint::Solid { color: gold, swatch: Some("Gold Ink".into()), tint: 0.5 };
        f.overprint = true;
    }
    let layer = d.layers[0].id;
    d.insert(Some(layer), 0, n).unwrap();
    let ps = text(&export(&d, &opts(&d), None, None).unwrap());
    assert_eq!(dsc(&ps, "DocumentCustomColors").as_deref(), Some("(Gold Ink)"));
    assert_eq!(dsc(&ps, "CMYKCustomColor").as_deref(), Some("0 0.2 0.8 0.1 (Gold Ink)"));
    assert!(ps.contains("/VCsp0 [/Separation (Gold Ink) cvn /DeviceCMYK {dup 0 mul exch dup 0.2 mul exch dup 0.8 mul exch 0.1 mul}] def"), "{ps}");
    let page = checked_page(&ps);
    assert!(page.contains("true op\nVCsp0 setcolorspace 0.5 setcolor\n") && page.contains("false op"), "{page}");
    assert!(!page_of(&d, &EpsOptions { overprint: Overprint::Discard, ..opts(&d) }).contains("true op"));
}

#[test]
fn rgb_is_written_as_cmyk_when_asked_and_in_cmyk_documents() {
    let d = doc_with(|d| vec![red_rect(d)]);
    assert!(page_of(&d, &opts(&d)).contains("1 0 0 rg"));
    let cmyk = page_of(&d, &EpsOptions { cmyk: true, ..opts(&d) });
    assert!(!cmyk.contains(" rg\n") && cmyk.lines().any(|l| l.ends_with(" k") && l.split(' ').count() == 5), "{cmyk}");
    let mut c = d.clone();
    c.color_mode = vectorcraft_doc::ColorMode::Cmyk;
    assert!(!page_of(&c, &opts(&c)).contains(" rg\n"));
}

/// A 4 × 2 PNG placed 10 pt a pixel: the left half opaque green, the right half transparent.
fn png_doc() -> Document {
    let mut img = image::RgbaImage::new(4, 2);
    for (x, _, p) in img.enumerate_pixels_mut() {
        *p = if x < 2 { image::Rgba([0, 200, 0, 255]) } else { image::Rgba([0, 0, 0, 0]) };
    }
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let mut d = Document::new(200.0, 200.0);
    d.images.insert("pic".into(), ImageBlob::new("image/png", png));
    let im = ImageObject {
        key: "pic".into(),
        width: 4,
        height: 2,
        xf: Affine::translate((10.0, 10.0)) * Affine::scale(10.0),
        link: None,
        placement: Default::default(),
    };
    let n = Node::new(d.alloc_id(), NodeKind::Image(im));
    let layer = d.layers[0].id;
    d.insert(Some(layer), 0, n).unwrap();
    d
}

/// The image data after the first image procedure, ASCII85-decoded.
fn image_data(page: &str) -> Vec<u8> {
    let at = page.find("} exec\n").unwrap() + 7;
    let end = page[at..].find("~>").unwrap() + at + 2;
    ps::ascii85_decode(&page[at..end]).unwrap()
}

#[test]
fn images_are_flate_with_masked_pixels_at_level_3_and_run_length_at_level_2() {
    let d = png_doc();
    let l3 = export(&d, &opts(&d), None, None).unwrap();
    let ps = text(&l3);
    checked_page(&ps);
    assert!(ps.contains("[40 0 0 20 10 10] cm\n/DeviceRGB setcolorspace"), "the pixel grid on its placement: {ps}");
    assert!(
        ps.contains(
            "/ImageType 4 /Width 4 /Height 2 /BitsPerComponent 8 /Decode [0 1 0 1 0 1] /ImageMatrix [4 0 0 2 0 0] /MaskColor [255 255 0 0 255 255]"
        ),
        "{ps}"
    );
    assert!(ps.contains("/DataSource VCsrc /FlateDecode filter >> image\nVCsrc flushfile } exec\n"), "{ps}");
    assert!(l3.warnings.is_empty(), "{:?}", l3.warnings);
    let pixels = ps::inflate(&image_data(&ps), 1 << 20).unwrap();
    let (g, key) = ([0, 200, 0], [255, 0, 255]);
    let row = [g, g, key, key].concat();
    assert_eq!(pixels, [row.clone(), row].concat());
    let l2 = export(&d, &EpsOptions { level: Level::Two, ..opts(&d) }, None, None).unwrap();
    let ps = text(&l2);
    checked_page(&ps);
    assert!(ps.contains("/ImageType 1") && ps.contains("/RunLengthDecode filter") && !ps.contains("MaskColor"), "{ps}");
    let row = [g, g, [255; 3], [255; 3]].concat();
    assert_eq!(unpack(&image_data(&ps)), [row.clone(), row].concat());
    assert!(l2.warnings.iter().any(|w| w.contains("written white at PostScript Level 2")), "{:?}", l2.warnings);
}

/// Run-length data decoded up to its end marker.
fn unpack(data: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    let mut i = 0;
    while let Some(&n) = data.get(i) {
        match n {
            128 => break,
            0..=127 => {
                out.extend(&data[i + 1..i + 2 + n as usize]);
                i += 2 + n as usize;
            }
            _ => {
                out.extend(std::iter::repeat_n(data[i + 1], 257 - n as usize));
                i += 2;
            }
        }
    }
    out
}

#[test]
fn run_length_and_ascii85_round_trip() {
    let cycle: Vec<u8> = (0..=255u8).cycle().take(1000).collect();
    for data in [vec![], vec![7], vec![1, 2, 3], vec![9; 300], cycle, [vec![0; 9], vec![1, 2, 2, 2, 2, 3]].concat()] {
        let mut rl = vec![];
        ps::packbits(&data, &mut rl);
        rl.push(128);
        assert_eq!(unpack(&rl), data);
        let a85 = ps::ascii85(&data);
        assert!(a85.lines().all(|l| !l.starts_with('%') && l.len() <= 80), "{a85}");
        assert_eq!(ps::ascii85_decode(&a85).unwrap(), data);
    }
    assert_eq!(ps::ascii85(b"Man "), "9jqo^~>\n");
    assert_eq!(ps::ascii85(&[0, 0, 0, 0]), "z~>\n");
    assert!(ps::ascii85_decode("9jqo^").is_none(), "no end marker");
    assert!(ps::ascii85_decode("9jqo^v~>").is_none(), "not base 85");
    assert_eq!(ps::num(1.0 / 3.0), "0.3333");
    assert_eq!((ps::num(-0.00001), ps::num(f64::NAN), ps::num(2.5)), ("0".into(), "0".into(), "2.5".into()));
    assert_eq!(ps::string("a(b)\\é"), "(a\\(b\\)\\\\\\303\\251)");
}

/// A 3 × 2 preview: black, white, transparent; then red, green, blue.
fn preview() -> Raster {
    let rgba = vec![0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 0, 255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255];
    Raster { width: 3, height: 2, rgba }
}

#[test]
fn a_tiff_preview_sits_behind_the_binary_header() {
    let d = doc_with(|d| vec![red_rect(d)]);
    for (kind, transparent) in [(Preview::TiffColor, true), (Preview::TiffColor, false), (Preview::TiffBlackWhite, true)] {
        let out = export(&d, &EpsOptions { preview: kind, transparent_preview: transparent, ..opts(&d) }, Some(&preview()), None).unwrap();
        let b = &out.bytes;
        let word = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap()) as usize;
        assert_eq!(&b[..4], &[0xC5, 0xD0, 0xD3, 0xC6]);
        let (ps_at, ps_len, tiff_at, tiff_len) = (word(4), word(8), word(20), word(24));
        assert_eq!((ps_at, word(12), word(16)), (30, 0, 0), "PostScript right after the header; no metafile");
        assert_eq!(tiff_at, ps_at + ps_len, "the TIFF follows the PostScript");
        assert_eq!(tiff_at + tiff_len, b.len());
        assert_eq!(&b[28..30], &[0xFF, 0xFF], "no checksum");
        assert!(b[ps_at..].starts_with(b"%!PS-Adobe-3.0 EPSF-3.0") && b[..tiff_at].ends_with(b"%%EOF\n"));
        let tiff = &b[tiff_at..];
        assert!(tiff.starts_with(b"II*\0"));
        assert_eq!(sections(b).unwrap().1, Some(tiff));
        if kind == Preview::TiffBlackWhite {
            // A bit a pixel, a byte a row, always opaque: black, white, (transparent) white; red
            // and blue are darker than mid-grey, green isn't. The strip is the file's end.
            let strip = &tiff[tiff.len() - 4..];
            assert_eq!(unpack(&[strip, &[128]].concat()), [0b1000_0000, 0b1010_0000]);
            continue;
        }
        let img = image::load_from_memory_with_format(tiff, image::ImageFormat::Tiff).unwrap().to_rgba8();
        assert_eq!(img.dimensions(), (3, 2));
        let expect: [u8; 4] = if transparent { [0, 0, 0, 0] } else { [255; 4] };
        assert_eq!(img.get_pixel(2, 0).0, expect, "transparent, or on white");
        assert_eq!(img.get_pixel(0, 1).0, [255, 0, 0, 255]);
    }
    // Asked for but not rendered: the file has no preview, and says so.
    let out = export(&d, &EpsOptions { preview: Preview::TiffColor, ..opts(&d) }, None, None).unwrap();
    assert!(out.bytes.starts_with(b"%!PS") && out.warnings.iter().any(|w| w.contains("preview")));
}

#[test]
fn the_native_document_and_thumbnail_come_back() {
    let d = doc_with(|d| vec![red_rect(d)]);
    let native: Vec<u8> = (0..20_000u32).flat_map(|i| (i % 251).to_le_bytes()).collect();
    for kind in [Preview::None, Preview::TiffColor] {
        let o = EpsOptions { native: Some(native.clone()), preview: kind, ..opts(&d) };
        let out = export(&d, &o, Some(&preview()), Some(&preview())).unwrap();
        assert_eq!(super::native(&out.bytes).as_deref(), Some(&native[..]));
        let png = super::thumbnail(&out.bytes).unwrap();
        assert_eq!(image::load_from_memory(&png).unwrap().to_rgba8().dimensions(), (3, 2));
        let ps = text(&out);
        checked_page(&ps);
        assert!(ps.find("%VectorCraft_BeginData: thumbnail").unwrap() < ps.find("%%BeginProlog").unwrap());
        assert!(ps.find("%VectorCraft_BeginData: native").unwrap() > ps.find("%%Trailer").unwrap());
        assert!(ps.lines().all(|l| l.len() <= 255));
    }
    let plain = export(&d, &opts(&d), None, None).unwrap();
    assert_eq!((super::native(&plain.bytes), super::thumbnail(&plain.bytes)), (None, None));
    // Damaged files give nothing back.
    assert_eq!(super::native(b"%VectorCraft_BeginData: native\n% 9jqo^\n%VectorCraft_EndData\n"), None);
    let mut bad = vec![0xC5, 0xD0, 0xD3, 0xC6];
    bad.extend([0xFF; 26]);
    assert_eq!(sections(&bad), None);
}

#[test]
fn clipping_groups_clip_and_hidden_or_template_art_is_left_out() {
    let mut d = doc_with(|d| {
        let clip = path(d, shapes::ellipse(Rect::new(0.0, 0.0, 100.0, 100.0)), Paint::None, Paint::None, 0.0);
        let inside = red_rect(d);
        let mut hidden = path(d, shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Paint::solid(Color::rgb(0.0, 0.0, 1.0)), Paint::None, 0.0);
        hidden.visible = false;
        let id = d.alloc_id();
        vec![Node::new(id, NodeKind::Group { children: vec![Arc::new(clip), Arc::new(inside)], clip: true }), hidden]
    });
    let mut t = Node::layer(d.alloc_id(), "Template", vectorcraft_doc::LayerColor::Preset(0));
    if let NodeKind::Layer { template, .. } = &mut t.kind {
        *template = true;
    }
    let blue = path(&mut d, shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Paint::solid(Color::rgb(0.0, 0.0, 1.0)), Paint::None, 0.0);
    *t.children_mut().unwrap() = vec![Arc::new(blue)];
    d.layers.push(Arc::new(t));
    let page = page_of(&d, &opts(&d));
    let clip = page.find("W\n").unwrap();
    assert!(page[..clip].contains("q\n") && page[clip..].contains("1 0 0 rg"), "{page}");
    assert!(!page.contains("0 0 1 rg"), "hidden and template art: {page}");
}

#[test]
fn level_preview_and_overprint_ids_read_back() {
    for l in Level::ALL {
        assert_eq!(Level::from_id(l.id()), Some(l));
    }
    assert_eq!(Level::from_id("LanguageLevel 2"), Some(Level::Two));
    assert_eq!(Level::from_id("1"), None);
    for p in Preview::ALL {
        assert_eq!(Preview::from_id(p.id()), Some(p));
    }
    assert_eq!(Overprint::from_id("DISCARD"), Some(Overprint::Discard));
}

#[test]
fn a_rich_document_is_well_formed_at_both_levels() {
    let d = (*vectorcraft_testkit::fixtures::rich_session().doc().unwrap().doc).clone();
    for level in Level::ALL {
        for cmyk in [false, true] {
            let out = export(&d, &EpsOptions { level, cmyk, ..opts(&d) }, None, None).unwrap();
            let page = checked_page(&text(&out));
            assert!(page.lines().count() > 50, "{page}");
        }
    }
}
