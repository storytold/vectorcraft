//! Imported colours keep their model: CMYK, Gray, spot inks (Separation, DeviceN) at a tint, in
//! solid paints and shading stops; a CMYK file opens as a CMYK document.

use vectorcraft_color::swatch::REGISTRATION;
use vectorcraft_color::{Color, Gradient, GradientKind, GradientPaint, GradientStop, Paint, Swatch};
use vectorcraft_doc::{Appearance, AppearanceItem, ColorMode, Document, Node, NodeId, NodeKind};
use vectorcraft_geom::{Rect, shapes};
use vectorcraft_testkit::pdf::{PdfPage, first_extra, pdf, pdf_with};

use crate::import_color::{Colors, Space};
use crate::*;

const Q: f32 = 1.0 / 255.0 + 1e-4;

fn add(d: &mut Document, p: Paint) {
    let x = d.node_count() as f64 * 12.0;
    let mut n = Node::path(NodeId(0), shapes::rectangle(Rect::new(x, 0.0, x + 10.0, 10.0)), Appearance::basic(p, Paint::None, 0.0));
    n.id = d.alloc_id();
    let l = d.layers[0].id;
    d.insert(Some(l), usize::MAX, n).unwrap();
}

/// The fills of the imported page, in paint order.
fn fills(d: &Document) -> Vec<Paint> {
    let NodeKind::Layer { children, .. } = &d.layers[0].kind else { panic!() };
    children
        .iter()
        .filter_map(|n| n.appearance.items.iter().find_map(|i| if let AppearanceItem::Fill(f) = i { Some(f.paint.clone()) } else { None }))
        .collect()
}

fn near(a: Color, b: Color) -> bool {
    match (a, b) {
        (Color::Cmyk { c, m, y, k }, Color::Cmyk { c: c2, m: m2, y: y2, k: k2 }) => [c - c2, m - m2, y - y2, k - k2].iter().all(|v| v.abs() <= Q),
        (Color::Gray { k }, Color::Gray { k: k2 }) => (k - k2).abs() <= Q,
        (Color::Lab { l, a, b }, Color::Lab { l: l2, a: a2, b: b2 }) => [l - l2, a - a2, b - b2].iter().all(|v| v.abs() <= 0.01),
        (Color::Rgb { r, g, b }, Color::Rgb { r: r2, g: g2, b: b2 }) => [r - r2, g - g2, b - b2].iter().all(|v| v.abs() <= Q),
        _ => false,
    }
}

fn solid(p: &Paint) -> (Color, Option<&str>, f32) {
    match p {
        Paint::Solid { color, swatch, tint } => (*color, swatch.as_deref(), *tint),
        other => panic!("{other:?}"),
    }
}

fn spot(name: &str, c: Color) -> Swatch {
    Swatch { name: name.into(), paint: Paint::solid(c), global: true, spot: true }
}

#[test]
fn a_cmyk_document_with_spot_colours_round_trips() {
    let mut d = Document::new_with_mode(200.0, 50.0, ColorMode::Cmyk);
    d.swatches.push(spot("Ink A", Color::cmyk(0.1, 0.9, 0.2, 0.0)));
    d.swatches.push(spot("Lab Ink", Color::lab(55.0, 60.0, 40.0)));
    add(&mut d, Paint::solid(Color::cmyk(0.1, 0.2, 0.3, 0.4)));
    add(&mut d, Paint::solid(Color::gray(0.25)));
    for (name, tint) in [("Ink A", 0.4), ("Lab Ink", 0.6), (REGISTRATION, 0.5)] {
        let p = d.tint_paint(name, tint).unwrap();
        add(&mut d, p);
    }
    let bytes = export(&d, &PdfOptions::default()).unwrap();
    let r = import_with_report(&bytes, &ImportOptions::default()).unwrap();
    let back = r.document;
    assert_eq!(back.color_mode, ColorMode::Cmyk, "a CMYK file opens in CMYK");
    assert!(back.swatches.iter().filter_map(|s| s.paint.color()).any(|c| matches!(c, Color::Cmyk { .. })), "CMYK default swatches");
    let f = fills(&back);
    assert_eq!(f.len(), 5);
    assert!(near(solid(&f[0]).0, Color::cmyk(0.1, 0.2, 0.3, 0.4)), "{:?}", f[0]);
    assert!(near(solid(&f[1]).0, Color::gray(0.25)), "{:?}", f[1]);
    // Spot inks link to new spot swatches at their tint.
    for (p, name, tint) in [(&f[2], "Ink A", 0.4), (&f[3], "Lab Ink", 0.6)] {
        let (color, link, t) = solid(p);
        assert_eq!(link, Some(name));
        assert!((t - tint).abs() <= Q, "{name}: tint {t}");
        let sw = back.swatch(name).unwrap();
        assert!(sw.spot && sw.global, "{name} is a spot swatch");
        assert!(near(color, sw.paint.color().unwrap().tinted(t)), "{name}: {color:?}");
    }
    assert!(near(back.swatch("Ink A").unwrap().paint.color().unwrap(), Color::cmyk(0.1, 0.9, 0.2, 0.0)));
    assert!(near(back.swatch("Lab Ink").unwrap().paint.color().unwrap(), Color::lab(55.0, 60.0, 40.0)), "a Lab alternate keeps Lab");
    // Registration (every plate) links to the built-in swatch, which isn't added.
    let (_, link, t) = solid(&f[4]);
    assert_eq!((link, (t * 100.0).round()), (Some(REGISTRATION), 50.0));
    assert_eq!(back.swatches.iter().filter(|s| s.spot).count(), 2);
    assert!(r.warnings.iter().all(|w| !w.contains("inks")), "{:?}", r.warnings);
}

#[test]
fn gray_imports_as_gray_and_rgb_documents_stay_rgb() {
    let mut d = Document::new(100.0, 50.0);
    add(&mut d, Paint::solid(Color::gray(0.8)));
    add(&mut d, Paint::solid(Color::rgb(0.2, 0.4, 0.6)));
    let back = import(&export(&d, &PdfOptions::default()).unwrap()).unwrap();
    assert_eq!(back.color_mode, ColorMode::Rgb);
    let f = fills(&back);
    assert!(near(solid(&f[0]).0, Color::gray(0.8)), "{:?}", f[0]);
    assert!(near(solid(&f[1]).0, Color::rgb(0.2, 0.4, 0.6)), "{:?}", f[1]);
    assert!(back.swatches.iter().all(|s| !s.spot));
}

#[test]
fn shading_stops_keep_cmyk_and_spot_tints() {
    let mut d = Document::new_with_mode(100.0, 50.0, ColorMode::Cmyk);
    d.swatches.push(spot("Ink A", Color::cmyk(0.0, 0.5, 1.0, 0.0)));
    let grad = |stops: Vec<GradientStop>| Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, stops))));
    add(&mut d, grad(vec![GradientStop::new(0.0, Color::cmyk(1.0, 0.0, 0.0, 0.0)), GradientStop::new(1.0, Color::cmyk(0.0, 0.0, 1.0, 0.2))]));
    let ink = d.global_color("Ink A").unwrap();
    let linked = |offset: f32, tint: f32| GradientStop { swatch: Some("Ink A".into()), tint, ..GradientStop::new(offset, ink.tinted(tint)) };
    add(&mut d, grad(vec![linked(0.0, 0.0), linked(1.0, 1.0)]));
    let back = import(&export(&d, &PdfOptions::default()).unwrap()).unwrap();
    let f = fills(&back);
    let stops = |p: &Paint| match p {
        Paint::Gradient(g) => g.gradient.stops.clone(),
        other => panic!("{other:?}"),
    };
    let s = stops(&f[0]);
    assert!(near(s[0].color, Color::cmyk(1.0, 0.0, 0.0, 0.0)) && near(s.last().unwrap().color, Color::cmyk(0.0, 0.0, 1.0, 0.2)), "{s:?}");
    assert!(s.iter().all(|s| matches!(s.color, Color::Cmyk { .. }) && s.swatch.is_none()));
    let s = stops(&f[1]);
    assert!(s.iter().all(|s| s.swatch.as_deref() == Some("Ink A")), "{s:?}");
    assert!(s[0].tint.abs() <= Q && (s.last().unwrap().tint - 1.0).abs() <= Q, "{s:?}");
    assert!(near(s.last().unwrap().color, Color::cmyk(0.0, 0.5, 1.0, 0.0)));
    assert!(back.swatch("Ink A").is_some_and(|w| w.spot));
}

/// A page painting with colour spaces `CS0`…: `fills` are `(space, components)`.
fn page_with(spaces: &str, fills: &[(&str, &str)]) -> PdfPage {
    let content: String = fills.iter().enumerate().map(|(i, (cs, c))| format!("/{cs} cs {c} scn {} 0 10 10 re f\n", i * 12)).collect();
    PdfPage { resources: format!("/ColorSpace << {spaces} >>"), ..PdfPage::new(200.0, 50.0, &content) }
}

#[test]
fn device_n_inks_become_spot_swatches_and_process_plates_cmyk() {
    // Gold and Silver → CMYK (0, 0, gold, silver), through a calculator function.
    let f = first_extra(1);
    let func = "<< /FunctionType 4 /Domain [0 1 0 1] /Range [0 1 0 1 0 1 0 1] /Length 16 >>\nstream\n{ 0 0 4 2 roll }\nendstream";
    // Cyan and Magenta → CMYK (cyan, magenta, 0, 0).
    let process = "<< /FunctionType 4 /Domain [0 1 0 1] /Range [0 1 0 1 0 1 0 1] /Length 9 >>\nstream\n{ 0 0 }\nendstream";
    let spaces = format!(
        "/CS0 [/DeviceN [/Gold /Silver] /DeviceCMYK {f} 0 R] /CS1 [/DeviceN [/Cyan /Magenta] /DeviceCMYK {} 0 R] /CS2 [/Separation /Black /DeviceGray << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0] /N 1 >>]",
        f + 1
    );
    let page = page_with(&spaces, &[("CS0", "0.3 0"), ("CS0", "0 1"), ("CS0", "0.5 0.5"), ("CS1", "0.2 0.7"), ("CS2", "0.4"), ("CS0", "0 0")]);
    let r = import_with_report(&pdf_with(&[page], &[func, process], None), &ImportOptions::default()).unwrap();
    let d = &r.document;
    let f = fills(d);
    assert_eq!(f.len(), 6);
    assert_eq!(solid(&f[0]).1, Some("Gold"));
    assert!((solid(&f[0]).2 - 0.3).abs() < 1e-6);
    assert_eq!((solid(&f[1]).1, solid(&f[1]).2), (Some("Silver"), 1.0));
    assert!(near(d.swatch("Gold").unwrap().paint.color().unwrap(), Color::cmyk(0.0, 0.0, 1.0, 0.0)));
    assert!(near(d.swatch("Silver").unwrap().paint.color().unwrap(), Color::cmyk(0.0, 0.0, 0.0, 1.0)));
    // Two inks at once can't link to one swatch: RGB, with a warning.
    assert!(matches!(solid(&f[2]), (Color::Rgb { .. }, None, _)));
    assert!(r.warnings.iter().any(|w| w.contains("several inks")), "{:?}", r.warnings);
    // Process plates are CMYK, also as a Separation.
    assert!(near(solid(&f[3]).0, Color::cmyk(0.2, 0.7, 0.0, 0.0)), "{:?}", f[3]);
    assert!(near(solid(&f[4]).0, Color::cmyk(0.0, 0.0, 0.0, 0.4)), "{:?}", f[4]);
    // No ink painting: 0 % of the first.
    assert_eq!((solid(&f[5]).1, solid(&f[5]).2), (Some("Gold"), 0.0));
    assert_eq!(d.swatches.iter().filter(|s| s.spot).count(), 2);
}

/// The spaces `/CS1` and `/CS2` of two inks (`[/{kind} {a} …]`, `[/{kind} {b} …]`) with the same
/// alternate and tint transform, blue at 100 %: they look the same to the interpreter.
fn blues(kind: &str, a: &str, b: &str) -> String {
    let tint = "/DeviceRGB << /FunctionType 2 /Domain [0 1] /C0 [1 1 1] /C1 [0 0 1] /N 1 >>";
    format!("/CS1 [/{kind} {a} {tint}] /CS2 [/{kind} {b} {tint}]")
}

/// The swatch each fill, stroke and type run links to, in paint order (a gradient's: its first
/// stop's).
fn links(d: &Document) -> Vec<Option<String>> {
    let mut out = vec![];
    d.walk(|n: &Node| {
        let runs: Vec<&Paint> = match &n.kind {
            NodeKind::Text(t) => t.runs.iter().flat_map(|r| [&r.style.fill, &r.style.stroke]).filter(|p| !p.is_none()).collect(),
            _ => vec![],
        };
        for p in n.appearance.items.iter().map(|i| i.paint()).chain(runs) {
            out.push(match p {
                Paint::Solid { swatch, .. } => swatch.clone(),
                Paint::Gradient(g) => g.gradient.stops.first().and_then(|s| s.swatch.clone()),
                _ => None,
            });
        }
    });
    out
}

fn named(names: &[Option<&str>]) -> Vec<Option<String>> {
    names.iter().map(|n| n.map(String::from)).collect()
}

#[test]
fn spot_inks_with_the_same_values_keep_their_own_swatches() {
    // The page's walk names the ink each path is painted in: Blue1 and Blue2 fills, then a fill of
    // Blue1 stroked in Blue2.
    for (kind, a, b) in [("Separation", "/Blue1", "/Blue2"), ("DeviceN", "[/Blue1]", "[/Blue2]")] {
        let content = "/CS1 cs 1 scn 0 0 10 10 re f /CS2 cs 0.5 scn 12 0 10 10 re f /CS2 CS 1 SCN /CS1 cs 1 scn 24 0 10 10 re B";
        let page = PdfPage { resources: format!("/ColorSpace << {} >>", blues(kind, a, b)), ..PdfPage::new(200.0, 50.0, content) };
        let d = import(&pdf(&[page], None)).unwrap();
        assert_eq!(links(&d), named(&[Some("Blue1"), Some("Blue2"), Some("Blue1"), Some("Blue2")]), "{kind}");
        assert!((solid(&fills(&d)[1]).2 - 0.5).abs() < 1e-6, "{kind}: Blue2 keeps its tint");
        for name in ["Blue1", "Blue2"] {
            let sw = d.swatch(name).unwrap_or_else(|| panic!("{kind}: no {name}"));
            assert!(sw.spot && sw.global, "{kind}: {name} is a spot swatch");
            assert!(near(sw.paint.color().unwrap(), Color::rgb(0.0, 0.0, 1.0)), "{kind}: {name}");
        }
    }
}

#[test]
fn inks_with_the_same_values_on_different_pages_keep_their_own_swatches() {
    // Each page names its own ink `/CS0`.
    let page = |ink: &str| {
        let tint = "/DeviceRGB << /FunctionType 2 /Domain [0 1] /C0 [1 1 1] /C1 [0 0 1] /N 1 >>";
        PdfPage {
            resources: format!("/ColorSpace << /CS0 [/Separation /{ink} {tint}] >>"),
            ..PdfPage::new(100.0, 50.0, "/CS0 cs 1 scn 0 0 10 10 re f")
        }
    };
    let d = import(&pdf(&[page("Blue1"), page("Blue2")], None)).unwrap();
    assert_eq!(links(&d), named(&[Some("Blue1"), Some("Blue2")]));
    assert_eq!(d.swatches.iter().filter(|s| s.spot).count(), 2);
}

#[test]
fn inks_set_before_a_content_window_ends_reach_the_paths_after_it() {
    // The inks are set in the first 64 KiB window of the content and a path is painted in them
    // two windows later, then a fill in Blue2 after the state saved before the windows is
    // restored.
    let filler = "q Q ".repeat(40_000);
    let content = format!("/CS1 cs 1 scn /CS2 CS 1 SCN q {filler}0 0 10 10 re B Q /CS2 cs 1 scn 12 0 10 10 re f");
    let page = PdfPage { resources: format!("/ColorSpace << {} >>", blues("Separation", "/Blue1", "/Blue2")), ..PdfPage::new(200.0, 50.0, &content) };
    let d = import(&pdf(&[page], None)).unwrap();
    assert_eq!(links(&d), named(&[Some("Blue1"), Some("Blue2"), Some("Blue2")]));
}

#[test]
fn type_in_spot_inks_with_the_same_values_keeps_its_own_ink() {
    // Type filled in Blue1, a black square, type stroked in Blue2: type or its outlines, as the
    // fonts of the machine allow.
    let content =
        "/CS1 cs 1 scn BT /F1 36 Tf 10 50 Td (Ink) Tj ET 0 g 0 0 5 5 re f /CS1 cs 1 scn /CS2 CS 1 SCN BT /F1 36 Tf 1 Tr 10 10 Td (Line) Tj ET";
    let resources = format!(
        "/ColorSpace << {} >> /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >>",
        blues("Separation", "/Blue1", "/Blue2")
    );
    let d = import(&pdf(&[PdfPage { resources, ..PdfPage::new(300.0, 100.0, content) }], None)).unwrap();
    let mut seen = links(&d);
    seen.dedup();
    assert_eq!(seen, named(&[Some("Blue1"), None, Some("Blue2")]));
}

#[test]
fn gradients_in_spot_inks_with_the_same_values_keep_their_own_ink() {
    // A shading in Blue2 drawn by `sh`, then a shading pattern in Blue1 filling a square.
    let shading = |ink: &str| {
        format!(
            "<< /ShadingType 2 /ColorSpace [/Separation /{ink} /DeviceRGB << /FunctionType 2 /Domain [0 1] /C0 [1 1 1] /C1 [0 0 1] /N 1 >>] /Coords [0 0 50 0] /Function << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> /Extend [true true] >>"
        )
    };
    let resources = format!("/Shading << /Sh0 {} >> /Pattern << /P0 << /PatternType 2 /Shading {} >> >>", shading("Blue2"), shading("Blue1"));
    let content = "q 0 0 50 50 re W n /Sh0 sh Q /Pattern cs /P0 scn 60 0 50 50 re f";
    let d = import(&pdf(&[PdfPage { resources, ..PdfPage::new(200.0, 50.0, content) }], None)).unwrap();
    let mut stops = vec![];
    d.walk(|n: &Node| {
        for i in &n.appearance.items {
            if let Paint::Gradient(g) = i.paint() {
                stops.push(g.gradient.stops.iter().map(|s| s.swatch.clone()).collect::<Vec<_>>());
            }
        }
    });
    assert_eq!(stops.len(), 2, "{stops:?}");
    assert!(stops[0].iter().all(|s| s.as_deref() == Some("Blue2")), "{stops:?}");
    assert!(stops[1].iter().all(|s| s.as_deref() == Some("Blue1")), "{stops:?}");
}

#[test]
fn a_color_set_after_a_pattern_is_in_the_ink_of_its_space() {
    // Blue1's space is set, `/P0 scn` names a shading pattern in Blue2, and `1 scn` sets a color
    // without a pattern: the interpreter fills in Blue1, the color space's ink.
    let tint = "/DeviceRGB << /FunctionType 2 /Domain [0 1] /C0 [1 1 1] /C1 [0 0 1] /N 1 >>";
    let shading = format!(
        "<< /ShadingType 2 /ColorSpace [/Separation /Blue2 {tint}] /Coords [0 0 50 0] /Function << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> >>"
    );
    let resources =
        format!("/ColorSpace << {} >> /Pattern << /P0 << /PatternType 2 /Shading {shading} >> >>", blues("Separation", "/Blue1", "/Blue2"));
    let d = import(&pdf(&[PdfPage { resources, ..PdfPage::new(100.0, 50.0, "/CS1 cs /P0 scn 1 scn 0 0 10 10 re f") }], None)).unwrap();
    assert_eq!(links(&d), named(&[Some("Blue1")]));
}

#[test]
fn a_mesh_in_one_of_two_inks_with_the_same_values_keeps_the_ink() {
    // A patch mesh in Plum, which looks the same to the interpreter as Wine: its points keep
    // Plum's CMYK values, and Plum is a spot swatch.
    let tint = "/DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0.5 1 0 0] /N 1 >>";
    let data = "000A0A0A250A3F0A5A255A3F5A5A5A5A3F5A255A0A3F0A250AFFFFFFFF>";
    let mesh = format!(
        "<< /ShadingType 6 /ColorSpace [/Separation /Plum {tint}] /BitsPerCoordinate 8 /BitsPerComponent 8 /BitsPerFlag 8 /Decode [0 255 0 255 0 1] /Filter /ASCIIHexDecode /Length {} >>\nstream\n{data}\nendstream",
        data.len()
    );
    let resources = format!("/ColorSpace << /CS1 [/Separation /Wine {tint}] >> /Shading << /Sh0 {} 0 R >>", first_extra(1));
    let d = import(&pdf_with(&[PdfPage { resources, ..PdfPage::new(100.0, 100.0, "/Sh0 sh") }], &[&mesh], None)).unwrap();
    let mut points = vec![];
    d.walk(|n: &Node| {
        if let NodeKind::Mesh(m) = &n.kind {
            points.extend(m.points.iter().map(|p| p.color));
        }
    });
    assert!(!points.is_empty());
    assert!(points.iter().all(|c| near(*c, Color::cmyk(0.5, 1.0, 0.0, 0.0))), "{points:?}");
    assert!(d.swatch("Plum").is_some_and(|s| s.spot));
}

#[test]
fn inks_that_cannot_be_told_apart_import_as_rgb() {
    // Two spaces of other inks with the same alternate and tint transform, on a page with a
    // shading the interpreter can't read: it draws nothing for it, so the page's walk counts one
    // path more than the interpreter draws, and its ink names are left out.
    let tint = "/DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 1 0 0] /N 1 >>";
    let spaces = format!("/CS0 [/Separation /Tealish {tint}] /CS1 [/Separation /Rubyish {tint}] /CS2 [/Separation /Tealish {tint}]");
    let page = PdfPage {
        resources: format!("/ColorSpace << {spaces} >> /Shading << /Sh0 << /ShadingType 99 >> >>"),
        ..PdfPage::new(200.0, 50.0, "/Sh0 sh /CS0 cs 1 scn 0 0 10 10 re f /CS1 cs 1 scn 12 0 10 10 re f")
    };
    let d = import(&pdf(&[page], None)).unwrap();
    assert_eq!(fills(&d).len(), 2);
    assert!(fills(&d).iter().all(|p| matches!(solid(p), (Color::Rgb { .. }, None, _))), "{:?}", fills(&d));
    assert!(d.swatches.iter().all(|s| !s.spot));
    // The same ink defined twice is still that ink.
    let spaces = format!("/CS0 [/Separation /Tealish {tint}] /CS2 [/Separation /Tealish {tint}]");
    let d = import(&pdf(&[page_with(&spaces, &[("CS2", "0.5")])], None)).unwrap();
    assert_eq!(solid(&fills(&d)[0]).1, Some("Tealish"));
}

#[test]
fn ai_files_read_through_their_pdf_part_keep_inks_with_the_same_values_apart() {
    // A page with an editor's private data, read as a `.ai` is.
    let mut page = page_with(&blues("Separation", "/Blue1", "/Blue2"), &[("CS1", "1"), ("CS2", "1")]);
    page.entries = "/PieceInfo << /Illustrator << /Private << /AIPrivateData1 7 /NumBlock 1 >> >> >>".into();
    let r = import_with_report(&pdf(&[page], None), &ImportOptions { uncalibrated_rgb: true, ..Default::default() }).unwrap();
    assert_eq!(links(&r.document), named(&[Some("Blue1"), Some("Blue2")]));
}

#[test]
fn inks_with_the_same_values_export_as_separations_of_their_own() {
    let page = page_with(&blues("Separation", "/Blue1", "/Blue2"), &[("CS1", "1"), ("CS2", "0.4")]);
    let d = import(&pdf(&[page], None)).unwrap();
    let out = export(&d, &PdfOptions::uncompressed()).unwrap();
    let text = String::from_utf8_lossy(&out);
    assert!(text.contains("/Separation/Blue1") && text.contains("/Separation/Blue2"), "each ink is a separation");
    let back = import(&out).unwrap();
    assert_eq!(links(&back), named(&[Some("Blue1"), Some("Blue2")]));
    assert!((solid(&fills(&back)[1]).2 - 0.4).abs() <= Q);
}

#[test]
fn ink_names_never_clash_with_the_document_swatches() {
    let taken = Document::new(1.0, 1.0).swatches.iter().find(|s| !s.paint.is_none() && s.name != REGISTRATION).unwrap().name.clone();
    let spaces = format!(
        "/CS0 [/Separation /{} /DeviceRGB << /FunctionType 2 /Domain [0 1] /C0 [1 1 1] /C1 [1 0 0] /N 1 >>]",
        taken.replace(' ', "#20").replace('=', "#3D")
    );
    let d = import(&pdf(&[page_with(&spaces, &[("CS0", "1")])], None)).unwrap();
    let f = fills(&d);
    let (color, link, _) = solid(&f[0]);
    let renamed = format!("{taken} 2");
    assert_eq!(link, Some(renamed.as_str()));
    assert!(near(color, Color::rgb(1.0, 0.0, 0.0)));
    assert!(d.swatch(&renamed).unwrap().spot);
}

#[test]
fn colour_spaces_are_read_from_the_interpreters_debug_form() {
    let pdf = craft_pdf::syntax::Pdf::new(pdf(&[PdfPage::new(10.0, 10.0, "")], None)).unwrap();
    let mut c = Colors::new(&pdf, vec![]);
    for (text, n, want) in [
        ("ColorSpace(DeviceCmyk)", 4, Space::Cmyk),
        ("ColorSpace(DeviceGray)", 1, Space::Gray),
        ("ColorSpace(DeviceRgb)", 3, Space::Rgb),
        ("ColorSpace(ICCBased(ICCColor {..}))", 4, Space::Cmyk),
        ("ColorSpace(ICCBased(ICCColor {..}))", 1, Space::Gray),
        ("ColorSpace(ICCBased(ICCColor {..}))", 3, Space::Other),
        ("ColorSpace(Lab(Lab { range: [] }))", 3, Space::Other),
        ("ColorSpace(Separation(Separation { tint_transform: Function(Type2(..)) }))", 1, Space::Other),
        ("ColorSpace(DeviceCmyk)", 3, Space::Other),
    ] {
        assert_eq!(c.space_of(text, n, None), want, "{text}");
    }
    let t = "Color { color_space: ColorSpace(DeviceCmyk), components: [0.101960786, 0.2, 0.3, 0.4], opacity: 1.0 }";
    let (space, comps) = crate::import_color::split_color(t).unwrap();
    assert_eq!((space, comps), ("ColorSpace(DeviceCmyk)", vec![0.101960786, 0.2, 0.3, 0.4]));
    assert_eq!(crate::import_color::split_color("Color { color_space: X, components: [NaN], opacity: 1.0 }"), None);
}

/// A dictionary value the reader can't parse (a name with a broken `#` escape, a number with a
/// stray `-`) is left out when the file's ink spaces are looked for, and the inks after it are found.
#[test]
fn inks_are_found_past_dictionary_values_the_reader_cannot_parse() {
    use craft_pdf::interpret::Function;
    use craft_pdf::syntax::object::{FromBytes, Object};
    let tint = "<< /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 0 1 0] /N 1 >>";
    let key = format!("{:?}", Function::new(&Object::from_bytes(tint.as_bytes()).unwrap()).unwrap());
    for junk in ["/#zz", "12-.5"] {
        let page = page_with(&format!("/A {junk} /CS0 [/Separation /Gold /DeviceCMYK {tint}]"), &[("CS0", "1")]);
        // Opened by the reader alone, without the checks of `pages::open`.
        let pdf = craft_pdf::syntax::Pdf::new(pdf(&[page], None)).unwrap();
        let mut c = Colors::new(&pdf, vec![]);
        let space = c.space_of(&format!("ColorSpace(Separation(Separation {{ tint_transform: {key} }}))"), 1, None);
        assert_eq!(space, Space::Inks(0), "{junk}");
        let gold = c.native(space, &[1.0]).unwrap();
        assert_eq!(gold.link, Some(("Gold".to_string(), 1.0)), "{junk}");
        assert!(near(gold.color, Color::cmyk(0.0, 0.0, 1.0, 0.0)), "{junk}: {:?}", gold.color);
    }
}

/// Spot colours defined in Lab can have an ICC Lab profile as their alternate space, `[/ICCBased
/// profile]` with 3 components: the tint transform gives Lab values, which are not RGB (#1032). The
/// profile is told by its `Alternate` (`[/Lab …]`), else by its header's data colour space.
#[test]
fn spot_colours_with_an_icc_lab_alternate_keep_their_lab_values() {
    let f = first_extra(1);
    // A profile's first 20 bytes: size, CMM, version, class, then the data colour space.
    let profile = |head: &str, alternate: &str| format!("<< /N 3 {alternate} /Length {} >>\nstream\n{head}\nendstream", head.len());
    let lab_header = profile("0000none0000spacLab XYZ ", "");
    let lab_alternate = profile("0000none0000spacXYZ XYZ ", "/Alternate [/Lab << /WhitePoint [0.9642 1 0.8249] /Range [-128 127 -128 127] >>]");
    let rgb = profile("0000none0000mntrRGB XYZ ", "");
    let tint = |c1: &str| format!("<< /FunctionType 2 /Domain [0 1] /C0 [100 0 0] /C1 [{c1}] /N 1 >>");
    let spaces = format!(
        "/CS0 [/Separation /Plum [/ICCBased {f} 0 R] {}] /CS1 [/Separation /Sky [/ICCBased {} 0 R] {}] /CS2 [/Separation /Leafy [/ICCBased {} 0 R] << /FunctionType 2 /Domain [0 1] /C0 [1 1 1] /C1 [0.5 0.9 0.1] /N 1 >>]",
        tint("32.9412 45 -18"),
        f + 1,
        tint("51 -26 -45"),
        f + 2
    );
    let page = page_with(&spaces, &[("CS0", "1"), ("CS1", "0.5"), ("CS2", "1")]);
    let d = import(&pdf_with(&[page], &[&lab_header, &lab_alternate, &rgb], None)).unwrap();
    let f = fills(&d);
    assert_eq!(solid(&f[0]).1, Some("Plum"));
    assert!(near(d.swatch("Plum").unwrap().paint.color().unwrap(), Color::lab(32.941, 45.0, -18.0)), "{:?}", d.swatch("Plum"));
    assert_eq!((solid(&f[1]).1, solid(&f[1]).2), (Some("Sky"), 0.5));
    assert!(near(d.swatch("Sky").unwrap().paint.color().unwrap(), Color::lab(51.0, -26.0, -45.0)), "{:?}", d.swatch("Sky"));
    // An RGB profile stays RGB.
    assert!(near(d.swatch("Leafy").unwrap().paint.color().unwrap(), Color::rgb(0.5, 0.9, 0.1)), "{:?}", d.swatch("Leafy"));
}

#[test]
fn rgb_numbers_stand_in_editor_documents_only() {
    // GUARDS: an editor's document keeps its `ICCBased` RGB numbers; a PDF's profile stands.
    let f = first_extra(1);
    let icc = vectorcraft_color::cms::icc_bytes(vectorcraft_color::cms::PROPHOTO_RGB).unwrap();
    let hex: String = icc.iter().map(|b| format!("{b:02x}")).collect();
    let profile = format!("<< /N 3 /Filter /ASCIIHexDecode /Length {} >>\nstream\n{hex}>\nendstream", hex.len() + 1);
    let mut page = page_with(&format!("/CS0 [/ICCBased {f} 0 R]"), &[("CS0", "0.2 0.6 0.4")]);
    page.entries = "/PieceInfo << /Illustrator << /Private << /AIPrivateData1 7 /NumBlock 1 >> >> >>".into();
    let bytes = pdf_with(&[page], &[&profile], None);
    let color = |uncalibrated_rgb| {
        solid(&fills(&import_with_report(&bytes, &ImportOptions { uncalibrated_rgb, ..Default::default() }).unwrap().document)[0]).0
    };
    assert!(near(color(true), Color::rgb(0.2, 0.6, 0.4)), "{:?}", color(true));
    assert!(!near(color(false), Color::rgb(0.2, 0.6, 0.4)), "{:?}", color(false));
}
