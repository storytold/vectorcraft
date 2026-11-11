//! Gradients in forms must follow the art, including on pages with a nonzero origin. Quartz
//! exposed a failure the import/render round trip did not: it resolved a form's shading pattern
//! in page space, leaving off-centre gradients painted entirely in their last stop's colour.
//! These fixtures are original, generated geometry; no user artwork or external files are used.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use vectorcraft_color::{BlendMode, Color, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::{Appearance, ColorMode, Document, Knockout, Node, OpacityMask};
use vectorcraft_geom::{Affine, Point, Rect, shapes};
use vectorcraft_pdf::{PdfOptions, export, import};
use vectorcraft_testkit::raster::{assert_similar, render_artboard};

#[derive(Clone, Copy, Debug)]
enum Form {
    Page,
    Opacity,
    Isolated,
    Nested,
    PassThrough,
    Mask,
    Knockout,
    Layer,
    Overprint,
}

const FORMS: [Form; 9] =
    [Form::Page, Form::Opacity, Form::Isolated, Form::Nested, Form::PassThrough, Form::Mask, Form::Knockout, Form::Layer, Form::Overprint];

fn rect(d: &mut Document, bounds: Rect, paint: Paint) -> Node {
    Node::path(d.alloc_id(), shapes::rectangle(bounds), Appearance::basic(paint, Paint::None, 0.0))
}

fn group(d: &mut Document, n: Node) -> Node {
    Node::group(d.alloc_id(), vec![Arc::new(n)])
}

/// Generate off-centre gradient art in each form type, moving the art and artboard together.
fn fixture(form: Form, kind: GradientKind, mode: ColorMode, origin: Point) -> (Document, PdfOptions) {
    let mut d = Document::new_with_mode(120.0, 180.0, mode);
    let color = |k| if mode == ColorMode::Cmyk { Color::cmyk(0.0, 0.0, 0.0, k) } else { Color::gray(k) };
    let mut gradient = GradientPaint::new(Gradient {
        kind,
        stops: vec![GradientStop::new(0.0, color(1.0)), GradientStop::new(0.6, color(0.4)), GradientStop::new(1.0, color(0.0))],
        ..Default::default()
    });
    let (start, end) = match kind {
        GradientKind::Radial => (Point::new(32.0, 35.0), Point::new(60.0, 35.0)),
        _ => (Point::new(10.0, 10.0), Point::new(70.0, 70.0)),
    };
    gradient.geom = Some(GradientGeom { start, end, aspect: 0.8, focal: None });
    let mut n = rect(&mut d, Rect::new(10.0, 10.0, 70.0, 70.0), Paint::Gradient(Box::new(gradient)));
    let mut opts = PdfOptions::default();
    match form {
        Form::Page => d.page_isolate = true,
        Form::Opacity => n.opacity = 0.6,
        Form::Isolated => {
            n = group(&mut d, n);
            n.isolate = true;
        }
        Form::Nested => {
            n.opacity = 0.75;
            n = group(&mut d, n);
            n.opacity = 0.8;
        }
        Form::PassThrough => {
            n.blend = BlendMode::Multiply;
            n = group(&mut d, n);
            n.opacity = 0.6;
        }
        Form::Mask => {
            let mask = rect(&mut d, Rect::new(10.0, 10.0, 44.0, 70.0), Paint::solid(Color::WHITE));
            n.extras_mut().mask = Some(Box::new(OpacityMask::new(mask, true)));
        }
        Form::Knockout => {
            let top = rect(&mut d, Rect::new(50.0, 50.0, 80.0, 80.0), Paint::solid(Color::WHITE));
            n = Node::group(d.alloc_id(), vec![Arc::new(n), Arc::new(top)]);
            n.knockout = Knockout::On;
        }
        Form::Layer => opts.settings.create_layers = true,
        Form::Overprint => n.appearance.fill_mut().unwrap().overprint = true,
    }
    n.transform(Affine::translate(origin.to_vec2()), false);
    d.artboards[0].rect = d.artboards[0].rect + origin.to_vec2();
    let layer = d.default_layer().unwrap();
    d.insert(Some(layer), 0, n).unwrap();
    (d, opts)
}

/// Round trips preserve gradient placement for the form types the importer supports.
#[test]
fn gradients_in_forms_keep_their_position_when_read_back() {
    // The importer has a pre-existing mismatch on the knockout's synthesized nested masks;
    // Quartz below checks its actual PDF output, along with the other forms.
    for form in FORMS.into_iter().filter(|f| !matches!(f, Form::Knockout)) {
        for kind in [GradientKind::Linear, GradientKind::Radial] {
            for origin in [Point::ORIGIN, Point::new(125.0, -80.0)] {
                let (d, opts) = fixture(form, kind, ColorMode::Rgb, origin);
                let back = import(&export(&d, &opts).unwrap()).unwrap();
                assert_similar(&render_artboard(&back), &render_artboard(&d), 5.0, 0.01);
            }
        }
    }
}

/// Layer form bounds stay page-sized after translation, without extra imported clipping groups.
#[test]
fn translated_layer_forms_cover_the_paper_in_their_new_space() {
    // The layer-box repair added upstream must move with the gradient fix's form transform.
    // A local-space paper box would otherwise introduce a clipping group on import.
    for origin in [Point::ORIGIN, Point::new(125.0, -80.0), Point::new(-250.0, 450.0)] {
        let (d, mut opts) = fixture(Form::Layer, GradientKind::Radial, ColorMode::Rgb, origin);
        opts.settings.compression.compress_text = false;
        let bytes = export(&d, &opts).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        let boxes: Vec<Vec<f64>> = text
            .split("/Subtype/Form")
            .skip(1)
            .map(|form| {
                form.split("/BBox[").nth(1).unwrap().split(']').next().unwrap().split_ascii_whitespace().map(|v| v.parse().unwrap()).collect()
            })
            .collect();
        assert_eq!(boxes, [vec![0.0, 0.0, 120.0, 180.0]], "{origin:?}: the layer covers exactly the page");
        let back = import(&bytes).unwrap();
        let children = back.layers[0].children().unwrap();
        assert_eq!(children.len(), 1);
        assert!(children[0].path_data().is_some(), "{origin:?}: no clipping group wraps the layer's art");
    }
}

// Test the renderer that exhibited the fault, not just our own importer (which accepted the
// old files). OS bindings are confined to this integration-test module.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod quartz {
    use objc2_core_foundation::CFData;
    use objc2_core_graphics::{CGBitmapContextCreate, CGColorSpace, CGContext, CGDataProvider, CGImageAlphaInfo, CGPDFDocument};
    use vectorcraft_testkit::raster::Image;

    use super::*;

    /// Render the first PDF page through Quartz as an oracle independent of our importer.
    fn render(bytes: &[u8]) -> Image {
        let data = CFData::from_bytes(bytes);
        let provider = CGDataProvider::with_cf_data(Some(&data)).unwrap();
        let doc = CGPDFDocument::with_provider(Some(&provider)).unwrap();
        let page = CGPDFDocument::page(Some(&doc), 1).unwrap();
        let space = CGColorSpace::new_device_rgb().unwrap();
        let (width, height) = (120, 180);
        let mut rgba = vec![255u8; width * height * 4];
        // SAFETY: the buffer holds every RGBA row, stays allocated and is not accessed until the
        // context is dropped. All inputs are the small, generated fixtures above.
        let context = unsafe {
            CGBitmapContextCreate(rgba.as_mut_ptr().cast(), width, height, 8, width * 4, Some(&space), CGImageAlphaInfo::PremultipliedLast.0)
        }
        .unwrap();
        CGContext::draw_pdf_page(Some(&context), Some(&page));
        drop(context);
        Image { width: width as u32, height: height as u32, rgba }
    }

    /// Quartz places RGB and CMYK radial centres, edges and masks with their translated art.
    #[test]
    fn quartz_keeps_off_centre_radials_in_transparency_and_layer_forms() {
        for mode in [ColorMode::Rgb, ColorMode::Cmyk] {
            for form in FORMS {
                for origin in [Point::ORIGIN, Point::new(125.0, -80.0)] {
                    let (d, opts) = fixture(form, GradientKind::Radial, mode, origin);
                    let img = render(&export(&d, &opts).unwrap());
                    let centre = img.over_white(32, 35)[0];
                    let edge = img.over_white(32, 55)[0];
                    assert!(centre < 150 && edge.saturating_sub(centre) > 40, "{mode:?} {form:?} {origin:?}: centre {centre}, edge {edge}");
                    assert_eq!(img.over_white(90, 100), [255; 3], "art stays in its bounds");
                    if matches!(form, Form::Mask) {
                        assert_eq!(img.over_white(46, 35), [255; 3], "the mask moves with the gradient");
                    }
                }
            }
        }
    }

    /// Quartz retains the linear ramp's direction inside forms on a translated artboard.
    #[test]
    fn quartz_keeps_linear_gradient_directions_in_forms() {
        for form in FORMS.into_iter().filter(|f| !matches!(f, Form::Mask | Form::Knockout)) {
            let (d, opts) = fixture(form, GradientKind::Linear, ColorMode::Rgb, Point::new(-30.0, 45.0));
            let img = render(&export(&d, &opts).unwrap());
            let (start, end) = (img.over_white(20, 20)[0], img.over_white(60, 60)[0]);
            assert!(end.saturating_sub(start) > 60, "{form:?}: start {start}, end {end}");
        }
    }

    /// A gradient's stop-opacity mask remains aligned with its colour through nested forms.
    #[test]
    fn quartz_keeps_gradient_stop_opacity_aligned_inside_nested_forms() {
        let (mut d, opts) = fixture(Form::Nested, GradientKind::Radial, ColorMode::Rgb, Point::ORIGIN);
        let mut leaves = vec![];
        d.walk(|n| {
            if n.path_data().is_some() {
                leaves.push(n.id);
            }
        });
        for id in leaves {
            let fill = d.node_mut(id).unwrap().appearance.fill_mut().unwrap();
            let Paint::Gradient(g) = &mut fill.paint else { panic!("gradient fixture") };
            for stop in &mut g.gradient.stops {
                stop.opacity = 0.6 * (1.0 - stop.offset);
            }
        }
        let img = render(&export(&d, &opts).unwrap());
        let (centre, edge) = (img.over_white(32, 35)[0], img.over_white(32, 55)[0]);
        assert!(centre > 145 && centre < 205 && edge > 240, "centre {centre}, edge {edge}");
    }
}
