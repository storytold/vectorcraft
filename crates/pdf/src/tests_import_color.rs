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

#[test]
fn inks_that_cannot_be_told_apart_import_as_rgb() {
    // Two spaces of other inks with the same alternate and tint transform.
    let tint = "/DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 1 0 0] /N 1 >>";
    let spaces = format!("/CS0 [/Separation /Tealish {tint}] /CS1 [/Separation /Rubyish {tint}] /CS2 [/Separation /Tealish {tint}]");
    let d = import(&pdf(&[page_with(&spaces, &[("CS0", "1")])], None)).unwrap();
    assert!(matches!(solid(&fills(&d)[0]), (Color::Rgb { .. }, None, _)));
    // The same ink defined twice is still that ink.
    let spaces = format!("/CS0 [/Separation /Tealish {tint}] /CS2 [/Separation /Tealish {tint}]");
    let d = import(&pdf(&[page_with(&spaces, &[("CS2", "0.5")])], None)).unwrap();
    assert_eq!(solid(&fills(&d)[0]).1, Some("Tealish"));
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
    let pdf = hayro_syntax::Pdf::new(pdf(&[PdfPage::new(10.0, 10.0, "")], None)).unwrap();
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
        assert_eq!(c.space_of(text, n), want, "{text}");
    }
    let t = "Color { color_space: ColorSpace(DeviceCmyk), components: [0.101960786, 0.2, 0.3, 0.4], opacity: 1.0 }";
    let (space, comps) = crate::import_color::split_color(t).unwrap();
    assert_eq!((space, comps), ("ColorSpace(DeviceCmyk)", vec![0.101960786, 0.2, 0.3, 0.4]));
    assert_eq!(crate::import_color::split_color("Color { color_space: X, components: [NaN], opacity: 1.0 }"), None);
}
