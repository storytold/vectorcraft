//! Layer Options on screen: Preview off draws a layer in outline, Dim Images fades its images;
//! exports (which leave templates out) draw both as they are.

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, ImageBlob, ImageObject, Node, NodeId, NodeKind};
use vectorcraft_geom::shapes;

use crate::*;

/// A red square (10..40) and a 30×30 red image at (50, 10), on one layer set up by `layer`.
fn doc(layer: impl FnOnce(&mut NodeKind)) -> Document {
    let mut d = Document::new(100.0, 100.0);
    let img = image::RgbaImage::from_pixel(30, 30, image::Rgba([255, 0, 0, 255]));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    d.images.insert("red".into(), ImageBlob::new("image/png", png));
    let l = d.layers[0].id;
    let red = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
    let sq = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 40.0, 40.0)), red);
    d.insert(Some(l), 0, sq).unwrap();
    let mut im = Node::new(
        NodeId(0),
        NodeKind::Image(Box::new(ImageObject {
            key: "red".into(),
            width: 30,
            height: 30,
            xf: Affine::translate((50.0, 10.0)),
            link: None,
            placement: Default::default(),
            pixels: None,
        })),
    );
    im.id = d.alloc_id();
    d.insert(Some(l), 9, im).unwrap();
    layer(&mut d.node_mut(l).unwrap().kind);
    d
}

fn render(d: &Document, export: bool) -> Rendered {
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), skip_templates: export, ..Default::default() };
    Renderer::new().render(d, 100, 100, Affine::IDENTITY, &opts)
}

#[test]
fn preview_off_draws_the_layer_in_outline_on_screen_only() {
    let d = doc(|k| {
        if let NodeKind::Layer { preview, .. } = k {
            *preview = false;
        }
    });
    assert_eq!(render(&d, false).pixel(25, 25), [255, 255, 255, 255], "no fill in outline");
    assert_eq!(render(&d, true).pixel(25, 25), [255, 0, 0, 255], "exports draw it as it is");
}

#[test]
fn dim_images_fades_only_the_images() {
    let plain = render(&doc(|_| {}), false);
    assert_eq!(plain.pixel(65, 25), [255, 0, 0, 255]);
    let d = doc(|k| {
        if let NodeKind::Layer { dim_images, .. } = k {
            *dim_images = Some(50);
        }
    });
    let img = render(&d, false);
    let [r, g, b, _] = img.pixel(65, 25);
    assert!(r == 255 && (120..=135).contains(&g) && g == b, "half way to white: {:?}", img.pixel(65, 25));
    assert_eq!(img.pixel(25, 25), [255, 0, 0, 255], "vector art stays");
    assert_eq!(render(&d, true).pixel(65, 25), [255, 0, 0, 255], "exports are not dimmed");
}

/// #1683: a template layer dims its images by its Dim Images percentage and nothing else: with
/// the option off they show at full strength, and the layer's paths are never faded.
#[test]
fn a_template_layer_dims_its_images_by_dim_images() {
    let template = |dim: Option<u8>| {
        let d = doc(|k| {
            if let NodeKind::Layer { template, dim_images, .. } = k {
                (*template, *dim_images) = (true, dim);
            }
        });
        let img = render(&d, false);
        (img.pixel(65, 25), img.pixel(25, 25))
    };
    let red = [255, 0, 0, 255];
    assert_eq!(template(None), (red, red), "Dim Images off: as it is");
    let (image, path) = template(Some(50));
    assert!(image[0] == 255 && (120..=135).contains(&image[1]) && image[1] == image[2], "half way to white: {image:?}");
    assert_eq!(path, red, "vector art stays");
    let (image, _) = template(Some(20));
    assert!((198..=210).contains(&image[1]) && image[1] == image[2], "a fifth of its strength: {image:?}");
    assert_eq!(template(Some(100)), (red, red));
}

/// In Outline view a template layer still draws in Preview, dimmed, so its picture can be traced;
/// the layer above it draws in outline (#1486).
#[test]
fn a_template_layer_stays_in_preview_in_outline_view() {
    let mut d = doc(|k| {
        if let NodeKind::Layer { template, preview, dim_images, .. } = k {
            // As Layer Options makes a template: its images dimmed to 50 %.
            (*template, *dim_images) = (true, Some(50));
            // Preview off doesn't put a template in outline either.
            *preview = false;
        }
    });
    let top = Node::layer(d.alloc_id(), "Drawing", vectorcraft_doc::LayerColor::Preset(0));
    let top_id = d.insert(None, 9, top).unwrap();
    let red = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
    let sq = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(10.0, 60.0, 40.0, 90.0)), red);
    d.insert(Some(top_id), 0, sq).unwrap();
    let draw = |outline: bool| {
        let opts = RenderOptions { background: Some([255, 255, 255, 255]), outline, ..Default::default() };
        Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &opts)
    };
    let (preview, outline) = (draw(false), draw(true));
    let [r, g, b, _] = outline.pixel(65, 25);
    assert!(r == 255 && (120..=135).contains(&g) && g == b, "the template's image, dimmed: {:?}", outline.pixel(65, 25));
    assert_eq!(outline.pixel(65, 25), preview.pixel(65, 25), "as in Preview");
    assert_eq!(outline.pixel(25, 75), [255, 255, 255, 255], "the drawing layer has no fill in outline");
    assert_eq!(preview.pixel(25, 75), [255, 0, 0, 255]);
}

/// View › Hide Template (#2000) leaves template layers out of the screen view and nothing else:
/// a guide path on another layer still draws, unlike in an export.
#[test]
fn hide_template_leaves_only_the_template_layers_out() {
    let mut d = doc(|k| {
        if let NodeKind::Layer { template, .. } = k {
            *template = true;
        }
    });
    let top = d.add_layer(Some("Guides"));
    let mut g = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(70.0, 70.0, 90.0, 90.0)), Appearance::default());
    if let NodeKind::Path { guide, .. } = &mut g.kind {
        *guide = true;
    }
    d.insert(Some(top), 0, g).unwrap();
    let render = |hide: bool| {
        let opts = RenderOptions { background: Some([255, 255, 255, 255]), hide_templates: hide, ..Default::default() };
        Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &opts)
    };
    let guide_drawn = |r: &Rendered| (60..100).flat_map(|y| (60..100).map(move |x| (x, y))).any(|(x, y)| r.pixel(x, y) != [255, 255, 255, 255]);
    let shown = render(false);
    assert_ne!(shown.pixel(25, 25), [255, 255, 255, 255], "the template layer's square shows (dimmed)");
    assert!(guide_drawn(&shown));
    let hidden = render(true);
    assert_eq!(hidden.pixel(25, 25), [255, 255, 255, 255], "Hide Template: the template layer is left out");
    assert_eq!(hidden.pixel(65, 25), [255, 255, 255, 255], "its image too");
    assert!(guide_drawn(&hidden), "the guide on the other layer still draws");
}

/// A guide path draws on screen only: exports leave it out, as they leave templates out (and as
/// the SVG, PSD and PDF exports and printing already do).
#[test]
fn guides_draw_on_screen_only() {
    let mut d = Document::new(100.0, 100.0);
    let l = d.layers[0].id;
    let mut g = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(70.0, 70.0, 90.0, 90.0)), Appearance::default());
    if let NodeKind::Path { guide, .. } = &mut g.kind {
        *guide = true;
    }
    d.insert(Some(l), 0, g).unwrap();
    let drawn = |export: bool| {
        let r = render(&d, export);
        (60..100).flat_map(|y| (60..100).map(move |x| (x, y))).filter(|&(x, y)| r.pixel(x, y) != [255, 255, 255, 255]).count()
    };
    assert!(drawn(false) > 0, "on screen the guide shows");
    assert_eq!(drawn(true), 0, "exports leave it out");
}

/// #3219: View > Hide Guides hides paths made into guides without using the export/template
/// filtering switch; Show Guides (the default) draws them again.
#[test]
fn hide_guides_hides_guide_paths_on_screen() {
    let mut d = Document::new(100.0, 100.0);
    let l = d.layers[0].id;
    let mut g = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(70.0, 70.0, 90.0, 90.0)), Appearance::default());
    if let NodeKind::Path { guide, .. } = &mut g.kind {
        *guide = true;
    }
    d.insert(Some(l), 0, g).unwrap();
    let pixels = |hide_guides: bool| {
        let opts = RenderOptions { background: Some([255; 4]), hide_guides, ..Default::default() };
        let r = Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &opts);
        (60..100).flat_map(|y| (60..100).map(move |x| (x, y))).filter(|&(x, y)| r.pixel(x, y) != [255; 4]).count()
    };
    assert!(pixels(false) > 0, "Show Guides draws the guide path");
    assert_eq!(pixels(true), 0, "Hide Guides leaves the guide path out");
}

/// #3219: hiding guides omits guide paths, including those inside cached group effects, without
/// hiding ordinary artwork. The same renderer must show them again when the view is toggled back.
#[test]
fn hidden_guide_paths_leave_the_artwork_and_refresh_cached_effects() {
    for effect in [false, true] {
        let mut d = Document::new(120.0, 120.0);
        let layer = d.layers[0].id;
        let mut guide =
            Node::path(d.alloc_id(), shapes::rectangle(Rect::new(20.5, 20.5, 60.5, 60.5)), Appearance::basic(Paint::None, Paint::None, 0.0));
        if let NodeKind::Path { guide, .. } = &mut guide.kind {
            *guide = true;
        }
        let id = guide.id;
        let art = Node::path(
            d.alloc_id(),
            shapes::rectangle(Rect::new(80.0, 80.0, 100.0, 100.0)),
            Appearance::basic(Paint::solid(Color::rgb(0.0, 0.0, 1.0)), Paint::None, 0.0),
        );
        let mut group = Node::group(d.alloc_id(), vec![Arc::new(guide), Arc::new(art)]);
        if effect {
            group.appearance.effects.push(vectorcraft_doc::Effect {
                id: "blur.gaussian".into(),
                params: serde_json::json!({"radius":2}),
                visible: true,
            });
        }
        d.insert(Some(layer), 0, group).unwrap();
        let mut without_guide = d.clone();
        without_guide.remove(id).unwrap();
        let mut renderer = Renderer::new();
        for outline in [false, true] {
            let opts = RenderOptions { outline, background: Some([255; 4]), ..Default::default() };
            let shown = renderer.render(&d, 120, 120, Affine::IDENTITY, &opts);
            let hidden = RenderOptions { hide_guides: true, ..opts.clone() };
            let expected = Renderer::new().render(&without_guide, 120, 120, Affine::IDENTITY, &hidden);
            assert_ne!(shown.pixels, expected.pixels, "the guide is initially visible, effect={effect}, outline={outline}");
            assert!(
                renderer.render(&d, 120, 120, Affine::IDENTITY, &hidden).pixels == expected.pixels,
                "only guides hide, effect={effect}, outline={outline}"
            );
            assert!(
                renderer.render(&d, 120, 120, Affine::IDENTITY, &opts).pixels == shown.pixels,
                "guides show again, effect={effect}, outline={outline}"
            );
        }
    }
}
