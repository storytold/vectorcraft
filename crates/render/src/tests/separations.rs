//! Separations Preview of placed raster images (#4153): an image shows the inks of the visible
//! plates, as vector art does, not its composite colours.

use vectorcraft_doc::cmyk::Inks;
use vectorcraft_doc::{Document, ImageBlob, ImageObject, Node, NodeKind};

use crate::*;

/// A 20×20 document filled by image `blob`.
fn doc_with(blob: ImageBlob) -> Document {
    let mut d = Document::new(20.0, 20.0);
    d.images.insert("img".into(), blob);
    let (id, layer) = (d.alloc_id(), d.layers[0].id);
    let im = ImageObject { key: "img".into(), width: 20, height: 20, xf: Affine::IDENTITY, link: None, placement: Default::default(), pixels: None };
    d.insert(Some(layer), 0, Node::new(id, NodeKind::Image(Box::new(im)))).unwrap();
    d
}

/// The centre pixel of `d` with the plates `visible` shown (`None`: Separations Preview off).
fn centre(d: &Document, visible: Option<&[&str]>) -> [u8; 4] {
    let proof = visible.map(|v| proof::ProofSetup { separations: Some(v.iter().map(|p| p.to_string()).collect()), ..Default::default() });
    let opts = RenderOptions { background: Some([255; 4]), overprint_preview: proof.is_some(), proof, ..Default::default() };
    Renderer::new().render_region_with(d, d.artboards[0].rect, 1.0, &opts).pixel(10, 10)
}

fn near(a: [u8; 4], b: [u8; 4], tol: i32) -> bool {
    a.iter().zip(b).all(|(x, y)| (*x as i32 - y as i32).abs() <= tol)
}

const ALL: &[&str] = &["Cyan", "Magenta", "Yellow", "Black"];

#[test]
fn rgb_images_show_the_visible_plates() {
    let img = image::RgbaImage::from_pixel(20, 20, image::Rgba([0, 255, 255, 255]));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let d = doc_with(ImageBlob::new("image/png", png));
    let cms = vectorcraft_color::cms::active();
    let inks = cms.srgb_to_cmyk([0.0, 1.0, 1.0], proof::ProofSetup::default().intent);
    // One plate: its ink as grey coverage (black = 100%).
    for (i, plate) in ["Cyan", "Magenta", "Yellow", "Black"].into_iter().enumerate() {
        let v = ((1.0 - inks[i]) * 255.0).round() as u8;
        let got = centre(&d, Some(&[plate]));
        assert!(near(got, [v, v, v, 255], 4), "{plate}: {got:?} ≠ {v}");
    }
    let cyan = centre(&d, Some(&["Cyan"]));
    assert!(cyan[0] < 110, "a cyan image prints mostly cyan: {cyan:?}");
    // Hiding the cyan plate takes the cyan ink away.
    let all = centre(&d, Some(ALL));
    let no_cyan = centre(&d, Some(&["Magenta", "Yellow", "Black"]));
    assert!(no_cyan[0] > all[0] + 60, "hiding Cyan: {no_cyan:?} vs {all:?}");
    // A spot plate the image doesn't print on shows paper.
    assert_eq!(centre(&d, Some(&["Spot"])), [255; 4]);
}

#[test]
fn cmyk_images_show_their_own_inks_on_the_plates() {
    let ink = [255u8, 0, 0, 0];
    let blob = ImageBlob::cmyk_tiff(&Inks::new(20, 20, ink.repeat(400)).unwrap()).unwrap();
    let d = doc_with(blob);
    assert_eq!(centre(&d, Some(&["Cyan"])), [0, 0, 0, 255], "100% cyan is solid on the cyan plate");
    for plate in ["Magenta", "Yellow", "Black"] {
        assert_eq!(centre(&d, Some(&[plate])), [255; 4], "no {plate} ink");
    }
    assert!(near(centre(&d, Some(&["Magenta", "Yellow", "Black"])), [255; 4], 2), "paper where only cyan prints");
    let all = centre(&d, Some(ALL));
    assert!(near(all, centre(&d, None), 4), "every plate shows the image: {all:?}");
}
