//! PDF/X-1a, PDF/X-3 and PDF/X-4: each standard's header, output intent, boxes and identification,
//! and what each refuses (RGB in PDF/X-1a, transparency in PDF/X-1a and PDF/X-3, passwords).

use serde_json::json;
use vectorcraft_color::cms::{self, GENERIC_CMYK, SRGB};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, ImageBlob, ImageObject, Node, NodeKind};
use vectorcraft_geom::{Affine, Rect, shapes};

use crate::*;

const X: [Standard; 3] = [Standard::PdfX1a, Standard::PdfX3, Standard::PdfX4];

/// A document with a rectangle filled with each of `fills`.
fn doc(fills: &[Color]) -> Document {
    let mut d = Document::new(200.0, 100.0);
    let l = d.layers[0].id;
    for (i, c) in fills.iter().enumerate() {
        let x = 10.0 + 30.0 * i as f64;
        let n =
            Node::path(d.alloc_id(), shapes::rectangle(Rect::new(x, 10.0, x + 20.0, 30.0)), Appearance::basic(Paint::solid(*c), Paint::None, 0.0));
        d.insert(Some(l), i, n).unwrap();
    }
    d
}

/// Options for `standard` at its version, content streams readable, a fixed date, changed by `f`.
fn opts(standard: Standard, f: impl FnOnce(&mut PdfSettings)) -> PdfOptions {
    let mut o = PdfOptions::uncompressed();
    (o.settings.standard, o.settings.compatibility) = (standard, standard.version());
    o.created = Some(1_700_000_000);
    f(&mut o.settings);
    o
}

fn settings(v: serde_json::Value) -> PdfSettings {
    serde_json::from_value(v).unwrap()
}

/// The file's text with its stream data left out (what dictionaries say).
fn text(pdf: &[u8]) -> String {
    String::from_utf8_lossy(pdf).into_owned()
}

#[test]
fn each_standard_writes_its_header_output_intent_boxes_and_identification() {
    let mut d = doc(&[Color::cmyk(0.0, 1.0, 1.0, 0.0), Color::rgb(0.2, 0.4, 0.9)]);
    // PDF/X files have a title: an untitled document gets one.
    d.title.clear();
    // As a PDF string (parentheses escaped).
    let profile = cms::active().settings().cmyk.replace('(', r"\(").replace(')', r"\)");
    for (standard, header) in [(Standard::PdfX1a, "%PDF-1.3"), (Standard::PdfX3, "%PDF-1.3"), (Standard::PdfX4, "%PDF-1.6")] {
        let r = export_with_report(&d, &opts(standard, |_| {})).unwrap();
        let t = text(&r.bytes);
        assert!(r.bytes.starts_with(header.as_bytes()), "{standard:?}: {}", &t[..8]);
        // The output intent: the CMYK profile in effect, embedded.
        for want in ["/OutputIntents[", "/S/GTS_PDFX", "/DestOutputProfile ", &format!("/OutputConditionIdentifier({profile})")] {
            assert!(t.contains(want), "{standard:?}: {want}");
        }
        // (PDF/X-3 and PDF/X-4 share the stream of the profile that tags the colours.)
        let oi: u32 = t.split("/DestOutputProfile ").nth(1).unwrap().split(' ').next().unwrap().parse().unwrap();
        let (start, end) = crate::patch::Xref::read(&r.bytes).unwrap().dict(&r.bytes, oi).unwrap();
        assert!(text(&r.bytes[start..end]).contains("/N 4"), "{standard:?}: a CMYK profile");
        // Every page is trimmed to its artboard; the file is untrapped, titled and dated.
        for want in ["/TrimBox[0 0 200 100]", "/Trapped/False", "/Title(Untitled)", "/CreationDate", "/ModDate"] {
            assert!(t.contains(want), "{standard:?}: {want}");
        }
        let (version, conformance) = match standard {
            Standard::PdfX1a => ("PDF/X-1:2001", Some("PDF/X-1a:2001")),
            Standard::PdfX3 => ("PDF/X-3:2002", None),
            _ => ("PDF/X-4", None),
        };
        assert!(t.contains(&format!("/GTS_PDFXVersion({version})")), "{standard:?}");
        assert_eq!(conformance.is_some_and(|c| t.contains(&format!("/GTS_PDFXConformance({c})"))), conformance.is_some(), "{standard:?}");
        // The metadata agrees with the header; PDF/X-4 names its standard there too.
        let xmp_version = if standard == Standard::PdfX4 { "1.6" } else { "1.3" };
        assert!(t.contains(&format!("<pdf:PDFVersion>{xmp_version}</pdf:PDFVersion>")), "{standard:?}");
        assert_eq!(t.contains("<pdfxid:GTS_PDFXVersion xmlns:pdfxid=\"http://www.npes.org/pdfx/ns/id/\">PDF/X-4<"), standard == Standard::PdfX4);
        assert_eq!(t.contains("<pdf:Trapped>False</pdf:Trapped>"), standard == Standard::PdfX4);
        assert!(!t.contains("/Encrypt"), "{standard:?}");
        // The file reads back.
        let back = import(&r.bytes).unwrap();
        assert_eq!(back.artboards.len(), 1, "{standard:?}");
        assert!(r.warnings.is_empty(), "{standard:?}: {:?}", r.warnings);
    }
    // Trapped, a named output intent with its condition and registry.
    let o = opts(Standard::PdfX4, |s| {
        s.output.trapped = true;
        s.output.output_intent = GENERIC_CMYK.into();
        s.output.output_condition_id = "CGATS TR 001".into();
        s.output.registry = "http://www.color.org".into();
    });
    let t = text(&export(&d, &o).unwrap());
    for want in ["/Trapped/True", "<pdf:Trapped>True<", "/OutputConditionIdentifier(CGATS TR 001)", "/RegistryName(http://www.color.org)"] {
        assert!(t.contains(want), "{want}");
    }
    // A registered condition that isn't a profile is named without one.
    let o = opts(Standard::PdfX1a, |s| {
        s.output.output_intent = "FOGRA39".into();
        s.output.registry = "http://www.color.org".into();
    });
    let t = text(&export(&d, &o).unwrap());
    assert!(t.contains("/OutputConditionIdentifier(FOGRA39)") && !t.contains("/DestOutputProfile"), "{t}");
    // PDF/X-4 always embeds its output intent's profile.
    let e = export(&d, &opts(Standard::PdfX4, |s| (s.output.output_intent, s.output.registry) = ("FOGRA39".into(), "http://www.color.org".into())));
    assert!(matches!(e, Err(PdfError::BadSetting(m)) if m.contains("embeds")));
}

#[test]
fn pdfx_1a_files_have_no_rgb() {
    let mut d = doc(&[Color::rgb(0.9, 0.1, 0.1), Color::Lab { l: 50.0, a: 40.0, b: -20.0 }, Color::gray(0.5), Color::cmyk(0.1, 0.2, 0.3, 0.4)]);
    // An RGB image too.
    let png = {
        let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([10, 200, 30, 255]));
        let mut out = std::io::Cursor::new(vec![]);
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    d.images.insert("img".into(), ImageBlob::new("image/png", png));
    let l = d.layers[0].id;
    let im = ImageObject {
        key: "img".into(),
        width: 4,
        height: 4,
        xf: Affine::translate((150.0, 50.0)) * Affine::scale(5.0),
        link: None,
        placement: Default::default(),
    };
    let n = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
    d.insert(Some(l), 0, n).unwrap();
    let t = text(&export(&d, &opts(Standard::PdfX1a, |_| {})).unwrap());
    for no in ["/DeviceRGB", "/ICCBased", "/Lab", "/CalRGB"] {
        assert!(!t.contains(no), "PDF/X-1a has no {no}");
    }
    let ops: Vec<&str> = t.split_whitespace().collect();
    assert!(!ops.contains(&"rg") && !ops.contains(&"RG"), "no RGB fills or strokes");
    assert!(ops.iter().filter(|o| **o == "k").count() >= 3, "the RGB, Lab and CMYK fills are CMYK");
    assert!(ops.contains(&"g"), "grey stays grey");
    assert!(t.contains("/ColorSpace/DeviceCMYK"), "the image is CMYK");
    // An RGB destination or output intent is refused; PDF/X-3 tags colours instead.
    let refused = |f: fn(&mut PdfSettings)| export(&d, &opts(Standard::PdfX1a, f)).unwrap_err();
    assert!(matches!(refused(|s| s.output.destination = SRGB.into()), PdfError::BadSetting(m) if m.contains("output.destination")));
    assert!(matches!(refused(|s| s.output.output_intent = SRGB.into()), PdfError::BadSetting(m) if m.contains("output.outputIntent")));
    let x3 = text(&export(&d, &opts(Standard::PdfX3, |_| {})).unwrap());
    assert!(x3.contains("/ICCBased") && !x3.contains("/DeviceRGB"), "PDF/X-3 colours are ICC-based");
    assert!(export(&d, &opts(Standard::PdfX3, |s| s.output.output_intent = SRGB.into())).is_ok(), "PDF/X-3 may print in RGB");
}

#[test]
fn pdfx_1a_and_x3_files_have_no_transparency() {
    let mut d = doc(&[Color::cmyk(1.0, 0.0, 0.0, 0.0), Color::cmyk(0.0, 1.0, 0.0, 0.0)]);
    let half = d.layers[0].children().unwrap()[1].id;
    d.node_mut(half).unwrap().opacity = 0.5;
    for standard in [Standard::PdfX1a, Standard::PdfX3] {
        // The writer can't flatten (the app does, before writing): it refuses.
        let e = export(&d, &opts(standard, |_| {})).unwrap_err();
        assert!(matches!(&e, PdfError::Unsupported(m) if m.contains("transparency") && m.contains("flatten")), "{standard:?}: {e}");
    }
    // Opaque art passes; PDF/X-4 keeps the transparency.
    d.node_mut(half).unwrap().opacity = 1.0;
    let t = text(&export(&d, &opts(Standard::PdfX3, |_| {})).unwrap());
    assert!(!t.contains("/S/Transparency") && !t.contains("/SMask"));
    d.node_mut(half).unwrap().opacity = 0.5;
    let t = text(&export(&d, &opts(Standard::PdfX4, |_| {})).unwrap());
    assert!(t.contains("/ca 0.5") || t.contains("/S/Transparency"), "PDF/X-4 keeps transparency");
}

#[test]
fn pdfx_settings_rules() {
    for standard in X {
        // A password is refused, and so are editing data.
        let e = opts(standard, |s| s.security.open_password = "x".into()).settings.check().unwrap_err();
        assert!(matches!(e, PdfError::BadSetting(_)), "{standard:?}: {e}");
        assert!(export(&doc(&[]), &opts(standard, |s| s.security.permissions_password = "x".into())).is_err());
        assert!(
            matches!(opts(standard, |s| s.preserve_editing = true).settings.check(), Err(PdfError::BadSetting(m)) if m.contains("preserveEditing"))
        );
        // Its own version, never PDF 1.7 or 2.0.
        assert!(standard.allows(standard.version()), "{standard:?}");
        assert!(!standard.allows(Compatibility::Pdf17) && !standard.allows(Compatibility::Pdf20), "{standard:?}");
        let e = opts(standard, |s| s.compatibility = Compatibility::Pdf17).settings.check().unwrap_err();
        assert!(matches!(e, PdfError::BadSetting(ref m) if m.contains(standard.label())), "{standard:?}: {e}");
        // A name that is no profile, without a registry.
        let e = opts(standard, |s| s.output.output_intent = "No Such Press".into()).settings.check().unwrap_err();
        assert!(matches!(e, PdfError::BadSetting(ref m) if m.contains("No Such Press")), "{standard:?}: {e}");
    }
    assert_eq!(Standard::PdfX4.version(), Compatibility::Pdf16);
    assert!(Standard::PdfX4.allows(Compatibility::Pdf15) && !Standard::PdfX1a.allows(Compatibility::Pdf15));
    // Layers: PDF/X-4 only.
    assert!(
        matches!(settings(json!({"standard": "pdfX1a", "compatibility": "1.4", "createLayers": true})).check(), Err(PdfError::BadSetting(m)) if m.contains("createLayers"))
    );
    assert!(settings(json!({"standard": "pdfX4", "compatibility": "1.6", "createLayers": true})).check_values().is_ok());
    // A date is required.
    assert!(matches!(crate::pdfx::require_date(Standard::PdfX4, None), Err(PdfError::Unsupported(_))));
    assert!(crate::pdfx::require_date(Standard::None, None).is_ok());
    // PDF/X settings don't warn about the output condition: the output intent is always written.
    assert!(settings(json!({"standard": "pdfX4", "compatibility": "1.6", "output": {"outputCondition": "Coated"}})).warnings().is_empty());
}

#[test]
fn verify_finds_what_each_standard_forbids() {
    use crate::pdfx::verify;
    let file = |body: &str| format!("%PDF-1.4\n1 0 obj\n{body}\nendobj\n").into_bytes();
    for (body, what) in [
        ("<</Type/XObject/SMask 5 0 R>>", "soft masks"),
        ("<</Group<</S/Transparency/CS/DeviceCMYK>>>>", "transparency groups"),
        ("<</Type/ExtGState/ca 0.5>>", "opacity"),
        ("<</Type/ExtGState/BM/Multiply>>", "blend modes"),
    ] {
        for standard in [Standard::PdfX1a, Standard::PdfX3] {
            assert!(matches!(verify(&file(body), standard), Err(PdfError::Unsupported(m)) if m.contains(what)), "{standard:?}: {body}");
        }
        assert!(verify(&file(body), Standard::PdfX4).is_ok(), "PDF/X-4 keeps {what}");
    }
    for ok in ["<</Type/ExtGState/SMask/None/ca 1/CA 1.0/BM/Normal>>", "<</Length 9>>\nstream\n/SMask 1 /DeviceRGB\nendstream"] {
        assert!(verify(&file(ok), Standard::PdfX1a).is_ok(), "{ok}");
    }
    assert!(matches!(verify(&file("[/ICCBased 3 0 R]"), Standard::PdfX1a), Err(PdfError::Unsupported(m)) if m.contains("ICCBased")));
    assert!(verify(&file("[/ICCBased 3 0 R]"), Standard::PdfX3).is_ok());
    assert!(verify(&file("<</PageLabels 3 0 R>>"), Standard::PdfX1a).is_ok(), "whole names only");
    let font = "<</Type/FontDescriptor/FontName/A>>";
    assert!(matches!(verify(&file(font), Standard::PdfX4), Err(PdfError::Unsupported(m)) if m.contains("fonts")));
    assert!(verify(&file("<</Type/FontDescriptor/FontFile2 4 0 R>>"), Standard::PdfX4).is_ok());
}
