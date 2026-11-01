//! EPS output read back: the DSC header and bounding box, the page's operators, gradients at both
//! levels, images, colours, the preview header and TIFF, and the data the file carries.

use std::sync::Arc;

use vectorcraft_color::{Color, Gradient, GradientKind, GradientMethod, GradientPaint, GradientStop, Paint, Swatch};
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

const GOLD: Color = Color::Cmyk { c: 0.0, m: 0.2, y: 0.8, k: 0.1 };
const LEAF: Color = Color::Cmyk { c: 1.0, m: 0.0, y: 1.0, k: 0.0 };

/// A stop linked to spot swatch `name` (of color `color`) at `tint`.
fn spot_stop(offset: f32, name: &str, color: Color, tint: f32) -> GradientStop {
    GradientStop { swatch: Some(name.into()), tint, ..GradientStop::new(offset, color.tinted(tint)) }
}

/// A document with the spot swatches Gold Ink and Leaf Ink and a rectangle filled with a linear
/// gradient of `stops`.
fn spot_gradient_doc(stops: Vec<GradientStop>) -> Document {
    spot_gradient_doc_of(Gradient::new(GradientKind::Linear, stops))
}

/// [`spot_gradient_doc`] with the rectangle filled with `gradient`.
fn spot_gradient_doc_of(gradient: Gradient) -> Document {
    let mut d = doc_with(|_| vec![]);
    d.swatches.push(Swatch { name: "Gold Ink".into(), paint: Paint::solid(GOLD), global: true, spot: true });
    d.swatches.push(Swatch { name: "Leaf Ink".into(), paint: Paint::solid(LEAF), global: true, spot: true });
    let paint = Paint::Gradient(Box::new(GradientPaint::new(gradient)));
    let n = path(&mut d, shapes::rectangle(Rect::new(20.0, 20.0, 180.0, 120.0)), paint, Paint::None, 0.0);
    let layer = d.layers[0].id;
    d.insert(Some(layer), 0, n).unwrap();
    d
}

/// The solid fill colors of the objects of `d`, down through groups.
fn fill_colors(d: &Document) -> Vec<Color> {
    fn walk(n: &Node, out: &mut Vec<Color>) {
        for i in &n.appearance.items {
            if let AppearanceItem::Fill(f) = i
                && let Some(c) = f.paint.color()
            {
                out.push(c);
            }
        }
        for c in n.children().into_iter().flatten() {
            walk(c, out);
        }
    }
    let mut out = vec![];
    for l in &d.layers {
        walk(l, &mut out);
    }
    out
}

/// Whether one of `colors` is CMYK `want`, to within rounding.
fn has_cmyk(colors: &[Color], want: [f32; 4]) -> bool {
    colors.iter().any(|c| matches!(*c, Color::Cmyk { c, m, y, k } if [c, m, y, k].iter().zip(want).all(|(a, b)| (a - b).abs() < 2e-3)))
}

#[test]
fn a_gradient_of_one_spot_ink_is_a_separation_shading() {
    let d = spot_gradient_doc(vec![
        spot_stop(0.0, "Gold Ink", GOLD, 1.0),
        spot_stop(0.5, "Gold Ink", GOLD, 0.2),
        GradientStop::new(1.0, Color::rgb(1.0, 1.0, 1.0)),
    ]);
    let out = export(&d, &opts(&d), None, None).unwrap();
    let ps = text(&out);
    assert_eq!(dsc(&ps, "DocumentCustomColors").as_deref(), Some("(Gold Ink)"));
    assert!(ps.contains("/VCsp0 [/Separation (Gold Ink) cvn /DeviceCMYK"), "{ps}");
    let page = checked_page(&ps);
    assert!(page.contains("/ShadingType 2 /ColorSpace VCsp0 /Coords"), "{page}");
    assert!(page.contains("/C0 [1] /C1 [0.2]") && page.contains("/C0 [0.2] /C1 [0]"), "tints of Gold Ink to paper: {page}");
    assert!(!out.warnings.iter().any(|w| w.contains("spot")), "{:?}", out.warnings);
    // Level 2: bands of the ink's tints.
    let out = export(&d, &EpsOptions { level: Level::Two, ..opts(&d) }, None, None).unwrap();
    let page = checked_page(&text(&out));
    assert!(!page.contains("shfill") && !page.contains(" k\n"), "{page}");
    assert!(page.contains("VCsp0 setcolorspace 1 setcolor\n") && page.contains("VCsp0 setcolorspace 0 setcolor\n"), "{page}");
    assert!(!out.warnings.iter().any(|w| w.contains("spot")), "{:?}", out.warnings);
}

#[test]
fn gradients_mixing_spot_inks_with_other_inks_are_device_n_shadings() {
    // Gold Ink, with its midpoint at 30%, to Leaf Ink at 50%.
    let mut gold = spot_stop(0.0, "Gold Ink", GOLD, 1.0);
    gold.midpoint = 0.3;
    let two = spot_gradient_doc(vec![gold, spot_stop(1.0, "Leaf Ink", LEAF, 0.5)]);
    let out = export(&two, &opts(&two), None, None).unwrap();
    let ps = text(&out);
    assert!(
        ps.contains("%%DocumentCustomColors: (Gold Ink)\n%%+ (Leaf Ink)\n%%CMYKCustomColor: 0 0.2 0.8 0.1 (Gold Ink)\n%%+ 1 0 1 0 (Leaf Ink)\n"),
        "both inks, each list continued right below its comment: {ps}"
    );
    assert!(
        ps.contains(
            "/VCdn0 [/DeviceN [(Gold Ink) cvn (Leaf Ink) cvn ] /DeviceCMYK {2 array astore 0 1 index 1 get 1 mul add \
             0 2 index 0 get 0.2 mul add 0 3 index 0 get 0.8 mul add 3 index 1 get 1 mul add 0 4 index 0 get 0.1 mul add 5 -1 roll pop}] def"
        ),
        "{ps}"
    );
    let page = checked_page(&ps);
    assert!(page.contains("/ShadingType 2 /ColorSpace VCdn0 /Coords") && page.contains("/Bounds [0.3 ]"), "{page}");
    assert!(page.contains("/C0 [1 0] /C1 [0.5 0.25]") && page.contains("/C0 [0.5 0.25] /C1 [0 0.5]"), "the inks through the midpoint: {page}");
    assert!(!out.warnings.iter().any(|w| w.contains("spot")), "{:?}", out.warnings);

    // Gold Ink to 60% process yellow to 40% gray: Yellow, Black and Gold Ink.
    let mixed = spot_gradient_doc(vec![
        spot_stop(0.0, "Gold Ink", GOLD, 1.0),
        GradientStop::new(0.5, Color::cmyk(0.0, 0.0, 0.6, 0.0)),
        GradientStop::new(1.0, Color::Gray { k: 0.4 }),
    ]);
    let out = export(&mixed, &opts(&mixed), None, None).unwrap();
    let ps = text(&out);
    assert!(ps.contains("/VCdn0 [/DeviceN [/Yellow /Black (Gold Ink) cvn ] /DeviceCMYK {3 array astore"), "{ps}");
    let page = checked_page(&ps);
    assert!(page.contains("/C0 [0 0 1] /C1 [0.6 0 0]") && page.contains("/C0 [0.6 0 0] /C1 [0 0.4 0]"), "{page}");
    assert!(!out.warnings.iter().any(|w| w.contains("spot")), "{:?}", out.warnings);

    // Read back as bands, the tint transforms give each ink's CMYK equivalent.
    for (d, end) in [(&two, [0.5, 0.0, 0.5, 0.0]), (&mixed, [0.0, 0.0, 0.0, 0.4])] {
        let out = export(d, &EpsOptions { compatible_gradients: true, ..opts(d) }, None, None).unwrap();
        let page = checked_page(&text(&out));
        assert!(page.contains("VCdn0 setcolorspace ") && !page.contains("shfill"), "{page}");
        let colors = fill_colors(&import(&out.bytes).unwrap().document);
        assert!(has_cmyk(&colors, [0.0, 0.2, 0.8, 0.1]) && has_cmyk(&colors, end), "{colors:?}");
    }

    // Level 2 has no DeviceN: process colors, with a warning.
    let out = export(&two, &EpsOptions { level: Level::Two, ..opts(&two) }, None, None).unwrap();
    assert!(!text(&out).contains("DeviceN"));
    assert!(out.warnings.iter().any(|w| w.contains("process colors at PostScript Level 2")), "{:?}", out.warnings);
}

#[test]
fn a_gradient_mixing_a_spot_ink_with_rgb_is_written_without_the_ink() {
    let d = spot_gradient_doc(vec![spot_stop(0.0, "Gold Ink", GOLD, 1.0), GradientStop::new(1.0, Color::rgb(0.0, 1.0, 0.0))]);
    let out = export(&d, &opts(&d), None, None).unwrap();
    let ps = text(&out);
    assert!(!ps.contains("/Separation") && !ps.contains("/DeviceN"), "{ps}");
    assert!(out.warnings.iter().any(|w| w.contains("RGB or Lab colors are written without the spot ink")), "{:?}", out.warnings);
    // Written as CMYK, the green stop mixes with the ink.
    let out = export(&d, &EpsOptions { cmyk: true, ..opts(&d) }, None, None).unwrap();
    assert!(text(&out).contains("/ColorSpace VCdn0"));
    assert!(!out.warnings.iter().any(|w| w.contains("spot")), "{:?}", out.warnings);
}

#[test]
fn spot_inks_named_like_process_inks_all_or_none_are_not_device_n_colorants() {
    for name in ["Cyan", "Black", "All", "None"] {
        // Gold Ink to the spot ink, with no process ink in the stops.
        let mut d = spot_gradient_doc(vec![spot_stop(0.0, "Gold Ink", GOLD, 1.0), spot_stop(1.0, name, LEAF, 1.0)]);
        d.swatches.retain(|s| s.name != name);
        d.swatches.push(Swatch { name: name.into(), paint: Paint::solid(LEAF), global: true, spot: true });
        let out = export(&d, &opts(&d), None, None).unwrap();
        let ps = text(&out);
        assert!(!ps.contains("/DeviceN") && !ps.contains("/Separation"), "{name}: {ps}");
        assert!(out.warnings.iter().any(|w| w.contains("can't share a DeviceN color space")), "{name}: {:?}", out.warnings);
    }
}

#[test]
fn a_device_n_space_has_at_most_32_colorants() {
    for (inks, device_n) in [(32, true), (33, false)] {
        let names: Vec<String> = (0..inks).map(|i| format!("Ink {i}")).collect();
        let stops = names.iter().enumerate().map(|(i, n)| spot_stop(i as f32 / (inks - 1) as f32, n, LEAF, 1.0)).collect();
        let mut d = spot_gradient_doc(stops);
        d.swatches.extend(names.iter().map(|n| Swatch { name: n.clone(), paint: Paint::solid(LEAF), global: true, spot: true }));
        let out = export(&d, &opts(&d), None, None).unwrap();
        let ps = text(&out);
        assert_eq!(ps.contains("/DeviceN [(Ink 0) cvn "), device_n, "{inks} inks: {ps}");
        assert_eq!(out.warnings.iter().any(|w| w.contains("can't share a DeviceN color space")), !device_n, "{:?}", out.warnings);
    }
}

#[test]
fn gradients_of_more_than_32_spot_inks_are_written_in_process_colors_at_every_level() {
    // 400 stops over 40 inks.
    let names: Vec<String> = (0..40).map(|i| format!("Ink {i}")).collect();
    let stops = (0..400).map(|i| spot_stop(i as f32 / 399.0, &names[i % names.len()], LEAF, 1.0)).collect();
    let mut d = spot_gradient_doc(stops);
    d.swatches.extend(names.iter().map(|n| Swatch { name: n.clone(), paint: Paint::solid(LEAF), global: true, spot: true }));
    for level in [Level::Three, Level::Two] {
        let out = export(&d, &EpsOptions { level, ..opts(&d) }, None, None).unwrap();
        let ps = text(&out);
        assert!(!ps.contains("/DeviceN") && !ps.contains("VCsp"), "{level:?}: {ps}");
        assert!(out.warnings.iter().any(|w| w.contains("can't share a DeviceN color space")), "{level:?}: {:?}", out.warnings);
        assert!(!out.warnings.iter().any(|w| w.contains("PostScript Level 2")), "{level:?}: {:?}", out.warnings);
    }
}

#[test]
fn near_white_stops_next_to_a_spot_ink_keep_their_ink() {
    let d = spot_gradient_doc(vec![spot_stop(0.0, "Gold Ink", GOLD, 1.0), GradientStop::new(1.0, Color::Gray { k: 0.001 })]);
    let out = export(&d, &opts(&d), None, None).unwrap();
    let ps = text(&out);
    assert!(ps.contains("/VCdn0 [/DeviceN [/Black (Gold Ink) cvn ]"), "{ps}");
    assert!(!out.warnings.iter().any(|w| w.contains("spot")), "{:?}", out.warnings);
    // A near-white RGB stop is RGB, not paper.
    let d = spot_gradient_doc(vec![spot_stop(0.0, "Gold Ink", GOLD, 1.0), GradientStop::new(1.0, Color::rgb(1.0, 1.0, 0.999))]);
    let out = export(&d, &opts(&d), None, None).unwrap();
    assert!(out.warnings.iter().any(|w| w.contains("RGB or Lab colors are written without the spot ink")), "{:?}", out.warnings);
}

#[test]
fn a_white_stop_linked_to_a_process_swatch_is_paper_next_to_a_spot_ink() {
    let white = Color::rgb(1.0, 1.0, 1.0);
    let mut d = spot_gradient_doc(vec![
        spot_stop(0.0, "Gold Ink", GOLD, 1.0),
        GradientStop { swatch: Some("Paper".into()), ..GradientStop::new(1.0, white) },
    ]);
    d.swatches.push(Swatch { name: "Paper".into(), paint: Paint::solid(white), global: true, spot: false });
    let out = export(&d, &opts(&d), None, None).unwrap();
    let page = checked_page(&text(&out));
    assert!(page.contains("/ShadingType 2 /ColorSpace VCsp0 /Coords"), "{page}");
    assert!(!out.warnings.iter().any(|w| w.contains("spot")), "{:?}", out.warnings);
}

#[test]
fn perceptual_gradients_of_spot_inks_are_written_in_process_colors() {
    let stops = vec![spot_stop(0.0, "Gold Ink", GOLD, 1.0), spot_stop(1.0, "Leaf Ink", LEAF, 1.0)];
    let mut perceptual = Gradient::new(GradientKind::Linear, stops.clone());
    perceptual.method = GradientMethod::Perceptual;
    let d = spot_gradient_doc_of(perceptual);
    let out = export(&d, &opts(&d), None, None).unwrap();
    let page = checked_page(&text(&out));
    assert!(!page.contains("VCdn") && !page.contains("VCsp"), "{page}");
    assert!(page.contains("/ColorSpace /DeviceCMYK"), "{page}");
    assert!(out.warnings.iter().any(|w| w.contains("Perceptual gradients of spot colors are written in process colors")), "{:?}", out.warnings);
    // The in-between stops follow the Perceptual mix, as for a gradient without spot inks.
    let process = |c: Color| GradientStop::new(0.0, c);
    let mut plain = Gradient::new(GradientKind::Linear, vec![process(GOLD), GradientStop { offset: 1.0, ..process(LEAF) }]);
    plain.method = GradientMethod::Perceptual;
    let p = spot_gradient_doc_of(plain);
    let plain_page = checked_page(&text(&export(&p, &opts(&p), None, None).unwrap()));
    let shading = |s: &str| -> Option<String> {
        let i = s.find("/ShadingType")?;
        Some(s[i..i + s[i..].find("shfill")?].to_string())
    };
    assert!(shading(&page).is_some_and(|s| s.contains("/Bounds")), "{page}");
    assert_eq!(shading(&page), shading(&plain_page));
    // Classic keeps the inks.
    let d = spot_gradient_doc(stops);
    assert!(text(&export(&d, &opts(&d), None, None).unwrap()).contains("/ColorSpace VCdn0"));
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
        pixels: None,
    };
    let n = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
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

fn colorized_doc(gray: Vec<u8>, paint: Option<Paint>) -> Document {
    let mut d = Document::new(200.0, 200.0);
    d.swatches.push(Swatch { name: "Ink".into(), paint: Paint::solid(Color::cmyk(0.0, 1.0, 1.0, 0.0)), global: true, spot: true });
    let mut png = Vec::new();
    image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(gray.len() as u32, 1, gray).unwrap())
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    d.images.insert("gray".into(), ImageBlob::png(png));
    let im = ImageObject {
        key: "gray".into(),
        width: 3,
        height: 1,
        xf: Affine::translate((10.0, 10.0)) * Affine::scale(10.0),
        link: None,
        placement: Default::default(),
        pixels: None,
    };
    let mut n = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
    n.extras_mut().colorize = paint.map(Box::new);
    d.insert(d.default_layer(), 0, n).unwrap();
    d
}

fn one_bit_blob() -> ImageBlob {
    // Uncompressed two-pixel TIFF, BlackIsZero; black then white.
    let entries: [(u16, u16, u32); 9] =
        [(256, 3, 2), (257, 3, 1), (258, 3, 1), (259, 3, 1), (262, 3, 1), (273, 4, 122), (277, 3, 1), (278, 3, 1), (279, 4, 1)];
    let mut bytes = b"II*\0".to_vec();
    bytes.extend_from_slice(&8u32.to_le_bytes());
    bytes.extend_from_slice(&9u16.to_le_bytes());
    for (tag, kind, value) in entries {
        bytes.extend_from_slice(&tag.to_le_bytes());
        bytes.extend_from_slice(&kind.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.push(0b0100_0000);
    ImageBlob::new("image/tiff", bytes)
}

fn colorized_samples(d: &Document, level: Level) -> (String, Vec<u8>, Vec<String>) {
    let out = export(d, &EpsOptions { level, ..opts(d) }, None, None).unwrap();
    let ps = text(&out);
    checked_page(&ps);
    let encoded = image_data(&ps);
    let samples = if level == Level::Three { ps::inflate(&encoded, 1 << 20).unwrap() } else { unpack(&encoded) };
    (ps, samples, out.warnings)
}

#[test]
fn binary_colorized_images_are_ink_stencils_at_both_levels() {
    for (paint, setup) in [
        (Paint::solid(Color::rgb8(255, 0, 0)), "1 0 0 rg"),
        (Paint::Solid { color: Color::rgb8(255, 0, 0), swatch: Some("Ink".into()), tint: 0.5 }, "VCsp0 setcolorspace 0.5 setcolor"),
        (Paint::registration(), "VCsp0 setcolorspace 1 setcolor"),
    ] {
        let d = colorized_doc(vec![0, 255, 0], Some(paint.clone()));
        for level in [Level::Two, Level::Three] {
            let (ps, samples, warnings) = colorized_samples(&d, level);
            assert!(ps.contains("imagemask") && ps.contains("/BitsPerComponent 1 /Decode [1 0]"), "{ps}");
            assert!(ps.contains(setup), "{ps}");
            assert_eq!(samples, vec![0b1010_0000]);
            assert!(warnings.is_empty(), "{warnings:?}");
            assert!(ps.contains(if level == Level::Two { "/RunLengthDecode" } else { "/FlateDecode" }));
            if matches!(paint, Paint::Solid { swatch: Some(_), .. }) {
                assert!(ps.contains("/Separation"));
            }
            if paint.is_registration() {
                assert!(ps.contains("/All /DeviceCMYK"));
                assert!(dsc(&ps, "DocumentCustomColors").is_none());
            } else if matches!(paint, Paint::Solid { swatch: Some(_), .. }) {
                assert_eq!(dsc(&ps, "DocumentCustomColors").as_deref(), Some("(Ink)"));
            }
        }
    }
}

#[test]
fn one_bit_source_uses_the_same_stencil_coverage() {
    let mut d = colorized_doc(vec![0, 255, 255], Some(Paint::solid(Color::rgb8(0, 0, 255))));
    d.images.insert("gray".into(), one_bit_blob());
    let id = d.layers[0].children().unwrap()[0].id;
    if let NodeKind::Image(im) = &mut d.node_mut(id).unwrap().kind {
        im.width = 2;
    }
    assert!(d.images["gray"].gray_source().unwrap().one_bit);
    let (_, samples, warnings) = colorized_samples(&d, Level::Three);
    assert_eq!(samples, vec![0b1000_0000]);
    assert!(warnings.is_empty());
}

#[test]
fn one_source_with_two_spot_tints_writes_two_inks() {
    let first = Paint::Solid { color: Color::rgb8(255, 0, 0), swatch: Some("Ink".into()), tint: 1.0 };
    let mut d = colorized_doc(vec![0, 255, 0], Some(first));
    let mut n = (*d.layers[0].children().unwrap()[0]).clone();
    n.id = d.alloc_id();
    n.extras_mut().colorize = Some(Box::new(Paint::Solid { color: Color::rgb8(255, 128, 128), swatch: Some("Ink".into()), tint: 0.5 }));
    d.insert(d.default_layer(), 1, n).unwrap();
    let ps = page_of(&d, &opts(&d));
    assert!(ps.contains("VCsp0 setcolorspace 1 setcolor") && ps.contains("VCsp0 setcolorspace 0.5 setcolor"), "{ps}");
    assert_eq!(ps.matches("imagemask").count(), 2);
}

#[test]
fn gray_colorized_ramp_scales_process_and_spot_inks_over_paper() {
    for level in [Level::Two, Level::Three] {
        let process = colorized_doc(vec![0, 128, 255], Some(Paint::solid(Color::rgb8(255, 0, 0))));
        let (ps, pixels, warnings) = colorized_samples(&process, level);
        assert!(ps.contains("/DeviceRGB setcolorspace") && !ps.contains("imagemask"));
        if level == Level::Three {
            assert!(ps.contains("/ImageType 4") && ps.contains("/MaskColor [255 255 0 0 255 255]"));
            assert_eq!(pixels, vec![255, 0, 0, 255, 128, 128, 255, 0, 255]);
            assert_eq!(warnings, vec!["partly transparent image pixels are written over white"]);
        } else {
            assert!(ps.contains("/ImageType 1") && !ps.contains("/MaskColor"));
            assert_eq!(pixels, vec![255, 0, 0, 255, 128, 128, 255, 255, 255]);
            assert_eq!(
                warnings,
                vec![
                    "partly transparent image pixels are written over white",
                    "transparent image pixels are written white at PostScript Level 2 (Level 3 masks them out)"
                ]
            );
        }

        let gray = colorized_doc(vec![0, 128, 255], Some(Paint::solid(Color::gray(0.5))));
        let (ps, pixels, warnings) = colorized_samples(&gray, level);
        assert!(ps.contains("/DeviceGray setcolorspace"));
        assert_eq!(pixels, if level == Level::Three { vec![128, 192, 0] } else { vec![128, 192, 255] });
        assert_eq!(ps.contains("/MaskColor [0 0]"), level == Level::Three);
        assert_eq!(warnings.len(), if level == Level::Three { 1 } else { 2 });

        let spot = colorized_doc(vec![0, 128, 255], Some(Paint::Solid { color: Color::rgb8(255, 128, 128), swatch: Some("Ink".into()), tint: 0.5 }));
        let (ps, pixels, warnings) = colorized_samples(&spot, level);
        assert!(ps.contains("/Separation (Ink) cvn") && ps.contains("VCsp0 setcolorspace"));
        assert_eq!(dsc(&ps, "DocumentCustomColors").as_deref(), Some("(Ink)"));
        assert_eq!(pixels, vec![128, 64, 0]);
        assert_eq!(ps.contains("/MaskColor [0 0]"), level == Level::Three);
        assert_eq!(warnings.len(), if level == Level::Three { 1 } else { 2 });

        let mut cmyk = process.clone();
        cmyk.color_mode = vectorcraft_doc::ColorMode::Cmyk;
        let (ps, pixels, _) = colorized_samples(&cmyk, level);
        assert!(ps.contains("/DeviceCMYK setcolorspace"));
        assert_eq!(pixels.len(), 12);
        assert_eq!(&pixels[8..], if level == Level::Three { &[255, 0, 255, 0][..] } else { &[0, 0, 0, 0][..] });
    }
}

#[test]
fn continuous_colorized_coverage_without_clear_pixels_only_warns_for_midtones() {
    let d = colorized_doc(vec![0, 128, 254], Some(Paint::solid(Color::rgb8(255, 0, 255))));
    for level in [Level::Two, Level::Three] {
        let (ps, pixels, warnings) = colorized_samples(&d, level);
        assert!(ps.contains("/ImageType 1") && !ps.contains("/MaskColor"));
        assert_eq!(&pixels[..3], &[255, 0, 255]);
        assert_eq!(warnings, vec!["partly transparent image pixels are written over white"]);
    }
    let d = colorized_doc(vec![0, 128, 255], Some(Paint::solid(Color::rgb8(255, 0, 255))));
    let (ps, pixels, _) = colorized_samples(&d, Level::Three);
    assert!(ps.contains("/MaskColor [255 255 0 0 255 255]"));
    assert_eq!(&pixels[..3], &[255, 1, 255]);
    assert_eq!(&pixels[6..], &[255, 0, 255]);
}

#[test]
fn uncolorized_image_stays_on_original_eps_path() {
    let d = colorized_doc(vec![0, 128, 255], None);
    let ps = page_of(&d, &opts(&d));
    assert!(!ps.contains("imagemask") && ps.contains("/ImageType 1"));
    let patterned = colorized_doc(vec![0, 128, 255], Some(Paint::Pattern { pattern: "missing".into(), xf: Affine::IDENTITY }));
    assert_eq!(page_of(&patterned, &opts(&patterned)), ps);
    let mut color = png_doc();
    let original = page_of(&color, &opts(&color));
    let id = color.layers[0].children().unwrap()[0].id;
    color.node_mut(id).unwrap().extras_mut().colorize = Some(Box::new(Paint::solid(Color::rgb8(255, 0, 0))));
    assert_eq!(page_of(&color, &opts(&color)), original);
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
    // #1715: the result is bounded while it is made, not by the length of the text.
    assert_eq!(ps::ascii85_decode_within("zz~>", 8), Some(vec![0; 8]));
    assert!(ps::ascii85_decode_within("zzz~>", 8).is_none(), "three `z` are 12 bytes");
    assert!(ps::ascii85_decode_within("zz9jqo^~>", 8).is_none());
    assert!(ps::ascii85_decode_within(&format!("{}~>", "z".repeat(1 << 16)), 1 << 10).is_none());
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
fn an_instance_whose_art_reaches_the_region_is_written() {
    // The symbol's red rectangle, (100, 100) to (140, 140), drawn from (-50, -50): on the artboard,
    // while the 20 × 20 square around the origin, moved to (-60, -60) to (-40, -40), is off it.
    let mut d = doc_with(|d| vec![Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Far".into(), xf: Affine::translate((-50.0, -50.0)) })]);
    let art = path(&mut d, shapes::rectangle(Rect::new(100.0, 100.0, 140.0, 140.0)), Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
    d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Far".into(), art: Arc::new(art) });
    let page = page_of(&d, &opts(&d));
    assert!(page.contains("1 0 0 rg"), "{page}");
}

#[test]
fn an_instance_whose_brush_art_reaches_the_region_is_written() {
    // The symbol's art: a line from (0, 40) to (0, 60) with a 1 pt red stroke painted by a 20 pt
    // round nib. Drawn from (-5, 0), the line lies left of the artboard, and the nib paints 10 pt
    // either side of it, onto the artboard.
    let mut d = doc_with(|d| vec![Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Nib".into(), xf: Affine::translate((-5.0, 0.0)) })]);
    let mut lib = vectorcraft_brush::library(&d);
    let Some(vectorcraft_brush::BrushKind::Calligraphic(c)) = lib.iter_mut().find(|b| b.name == "3 pt. Round").map(|b| &mut b.kind) else {
        panic!("the 3 pt. Round brush")
    };
    c.size = 20.0;
    vectorcraft_brush::store(&mut d, &lib);
    let mut art = path(&mut d, shapes::line((0.0, 40.0).into(), (0.0, 60.0).into()), Paint::None, Paint::solid(Color::rgb(1.0, 0.0, 0.0)), 1.0);
    art.appearance.stroke_mut().unwrap().brush = Some("3 pt. Round".into());
    d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Nib".into(), art: Arc::new(art) });
    let page = page_of(&d, &opts(&d));
    assert!(page.contains("1 0 0 rg"), "{page}");
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

/// A colour bitmap glyph (emoji) is written as an image where it sits; the type's fill doesn't
/// paint it.
#[test]
fn colour_bitmap_glyphs_are_images() {
    use vectorcraft_doc::{CharStyle, TextObject};
    use vectorcraft_text::test_fonts::{COLOUR_CHAR, colour_bitmap_family, colour_bitmap_font};
    vectorcraft_text::FontDb::global().add_font(colour_bitmap_font(false).unwrap());
    let d = doc_with(|d| {
        let st = CharStyle { font_family: colour_bitmap_family(false).into(), size: 32.0, ..CharStyle::default() };
        vec![Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(20.0, 100.0), &COLOUR_CHAR.to_string(), st))))]
    });
    let ps = text(&export(&d, &opts(&d), None, None).unwrap());
    checked_page(&ps);
    assert!(ps.contains("/Width 16 /Height 16"), "{ps}");
    // One em (32 pt) square, its top 24 pt above the baseline (12 of 16 px), 4 pt right of the
    // origin (2 px), in text space.
    assert!(ps.contains("[32 0 0 32 4 -24] cm"), "{ps}");
}

#[test]
fn colr_glyphs_are_vector_art() {
    use vectorcraft_doc::{CharStyle, TextObject};
    use vectorcraft_text::test_fonts::{COLOUR_CHAR, colr_family, colr_font};
    vectorcraft_text::FontDb::global().add_font(colr_font(false).unwrap());
    let d = doc_with(|d| {
        let fill = vectorcraft_color::Paint::solid(vectorcraft_color::Color::rgb(0.0, 0.0, 1.0));
        let st = CharStyle { font_family: colr_family(false).into(), size: 32.0, fill, ..CharStyle::default() };
        vec![Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(20.0, 100.0), &COLOUR_CHAR.to_string(), st))))]
    });
    let ps = text(&export(&d, &opts(&d), None, None).unwrap());
    checked_page(&ps);
    assert!(!ps.contains("/Width"), "no image: {ps}");
    assert!(ps.contains("1 0 0 rg"), "the red layer: {ps}");
    assert!(ps.contains("0 0 1 rg"), "the foreground layer in the fill: {ps}");
}

/// Ruby (furigana) is written with its base text, as the canvas draws it: one more `m` per contour
/// of the reading's glyphs.
#[test]
fn ruby_is_written_with_its_type() {
    use vectorcraft_doc::{CharStyle, RubySpan, TextObject};
    let typed = |ruby: bool| {
        let mut t = TextObject::point(Point::new(20.0, 100.0), "AB", CharStyle { size: 30.0, ..CharStyle::default() });
        if ruby {
            t.ruby = vec![RubySpan::new(0, 2, "xyz")];
        }
        t
    };
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &typed(true));
    let reading = lay.ruby.iter().flat_map(|g| g.outline.elements()).filter(|e| matches!(e, kurbo::PathEl::MoveTo(_))).count();
    assert!(reading >= 3, "the reading has ink: {reading}");
    let moves = |ruby: bool| {
        let d = doc_with(|d| vec![Node::new(d.alloc_id(), NodeKind::Text(Box::new(typed(ruby))))]);
        page_of(&d, &opts(&d)).lines().filter(|l| l.ends_with(" m")).count()
    };
    assert_eq!(moves(true), moves(false) + reading, "the reading's contours are written");
}
