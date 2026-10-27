//! The layers of an Illustrator EPS or `.ai`.
//!
//! Besides the page it prints, such a file carries the app's own copy of the art: an EPS after its
//! `%%EOF`, a `.ai` in its `AIPrivateData` streams (see `ai` for the containers). A file in the
//! legacy format (a `.ai` in PostScript form: versions 3 to 8, and the apps that still write it)
//! is that copy itself: its program has the layers, and the prolog it names defines the operators
//! for printing (other apps name it without including it, so their files can't be run).
//!
//! The copy has what the page doesn't: the layers with their names and options, the groups,
//! compound paths and clipping groups, the objects' names, the hidden objects and layers, the art
//! outside the artboards and every artboard. `ai` reads that structure into a document, with the
//! colours, gradients, transparency and images of the art.
//!
//! Text objects are slots in that document: their characters, fonts and places are in the file's
//! text document (see `ate`). Type that shows comes from the page, which draws it, into the slot
//! whose text is where the page has it. Type that doesn't show (a hidden layer or object, or off
//! the page) is made from the text document; where that can't be read (area type, type on a path)
//! it's left out, with a warning.
//!
//! What a layer that shows has that `ai` doesn't read makes the file come in as its page, as it did
//! before; on a layer that doesn't show it is left out of that layer, with a warning. The page is
//! also the safety net: when the layers' art looks different from it (one draws an object the
//! other doesn't, or much of it differs; an EPS whose page is its art's box has nothing printed
//! outside it), the page is used.
//!
//! Where a file doesn't match what this module expects, it is left alone and its page is imported.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::sync::Arc;

use vectorcraft_color::BlendMode;
use vectorcraft_doc::{AppearanceItem, Document, Node, NodeId, NodeKind, Scaling, TextKind, Unit};
use vectorcraft_geom::{Affine, PathData, Point, Rect, Vec2};

use super::Imported;
use super::ai::{self, Structure, slot_of};
use super::ate::{Story, Texts};

mod build;
mod compare;
mod layers;
mod plan;
mod slots;

#[cfg(test)]
use build::{Frame, MADE, prune_unseen};
#[cfg(test)]
use compare::{MAX_DIFFERENCE, MAX_SOLID_DIFFERENCE, PageView, compare, compared, comparison_render, comparison_samples, differ, differ_in_colour};
pub use layers::{ai_alone, layered_ai};
pub(super) use layers::{layered, program_alone};
pub use slots::is_loss;
#[cfg(test)]
use slots::join_pieces;

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_doc::{CharStyle, TextObject};

    /// `doc` with a square ring round (20, 20)–(80, 80) (or at `at`), stroked `color` `width` wide,
    /// named `name`.
    fn ring(doc: &mut Document, name: Option<&str>, color: vectorcraft_color::Color, width: f64, at: Rect) {
        let stroke = vectorcraft_color::Paint::solid(color);
        let mut n = Node::path(
            doc.alloc_id(),
            vectorcraft_geom::shapes::rectangle(at),
            vectorcraft_doc::Appearance::basic(vectorcraft_color::Paint::None, stroke, width),
        );
        n.name = name.map(str::to_string);
        let layer = doc.layers[0].id;
        doc.insert(Some(layer), usize::MAX, n).unwrap();
    }

    /// A text object's red stroke drawn over a dark red one of about the same lightness is drawn
    /// by the page: it isn't taken out as unseen (#1102). An object the page doesn't draw still is.
    #[test]
    fn a_stroke_over_one_as_light_is_not_taken_for_unseen() {
        let (dark, red) = (vectorcraft_color::Color::rgb(0.55, 0.0, 0.0), vectorcraft_color::Color::rgb(1.0, 0.0, 0.0));
        let square = Rect::new(20.0, 20.0, 80.0, 80.0);
        let mut page = Document::new(100.0, 100.0);
        ring(&mut page, None, dark, 12.0, square);
        ring(&mut page, None, red, 6.0, square);
        let mut layers = Document::new(100.0, 100.0);
        ring(&mut layers, Some(MADE), dark, 12.0, square);
        ring(&mut layers, Some(MADE), red, 6.0, square);
        ring(&mut layers, Some(MADE), red, 2.0, Rect::new(40.0, 40.0, 60.0, 60.0));
        let rect = page.artboards[0].rect;
        let frame = Frame { page: &page, page_rect: rect, rect, whole: false };
        let mut view = PageView::new(&frame);
        assert_eq!(prune_unseen(&frame, &mut view, &mut layers), 1, "only the ring the page doesn't draw");
        let widths: Vec<f64> = layers.layers[0].children().unwrap().iter().filter_map(|n| n.appearance.stroke().map(|s| s.width)).collect();
        assert_eq!(widths, [12.0, 6.0]);
        // Taken by lightness alone, the red and the dark red are alike.
        let px = |c: vectorcraft_color::Color| {
            let [r, g, b] = c.to_rgb().map(|v| (v * 255.0).round() as u8);
            [r, g, b, 255]
        };
        assert!(!differ(&px(dark), &px(red)) && differ_in_colour(&px(dark), &px(red)));
    }

    fn piece(text: &str, x: f64, y: f64) -> Arc<Node> {
        Arc::new(Node::new(NodeId(0), NodeKind::Text(Box::new(TextObject::point(Point::new(x, y), text, CharStyle::default())))))
    }

    /// Lines that the page draws in pieces, the pieces of one between those of the other, are each
    /// joined, and only their own pieces go.
    #[test]
    fn interleaved_lines_are_joined_without_taking_each_others_pieces() {
        let mut parts = vec![piece("He", 0.0, 0.0), piece("ab", 0.0, -20.0), piece("cd", 16.0, -20.0), piece("llo", 20.0, 0.0)];
        let lines = [("Hello".to_string(), Point::new(0.0, 0.0)), ("abcd".to_string(), Point::new(0.0, -20.0))];
        join_pieces(&mut parts, &lines);
        let texts: Vec<String> = parts.iter().filter_map(|n| if let NodeKind::Text(t) = &n.kind { Some(t.plain_text()) } else { None }).collect();
        assert_eq!(texts, ["Hello", "abcd"]);
    }
    /// A flattened page samples a fine stripe pattern once per comparison pixel; that
    /// aliases to white although vector coverage is gray. Shared minification avoids the
    /// false fallback, while genuinely missing art must still be rejected.
    #[test]
    fn raster_page_comparison_filters_minification_without_accepting_missing_art() {
        use vectorcraft_color::{Color, Paint};
        use vectorcraft_doc::{Appearance, ImageBlob, ImageObject};
        let mut vector = Document::new(1000.0, 500.0);
        let layer = vector.layers[0].id;
        for i in 0..400 {
            let x = f64::from(i) * 2.5 + 0.5;
            let n = Node::path(
                vector.alloc_id(),
                vectorcraft_geom::shapes::rectangle(Rect::new(x, 0.0, x + 1.0, 500.0)),
                Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
            );
            vector.insert(Some(layer), usize::MAX, n).unwrap();
        }
        let rect = vector.artboards[0].rect;
        let source = vectorcraft_render::Renderer::new().render_region(&vector, rect, 1.6, true);
        let mut png = Vec::new();
        image::RgbaImage::from_raw(source.width, source.height, source.to_straight())
            .unwrap()
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let blob = ImageBlob::new("image/png", png);
        let key = blob.content_key();
        let mut page = Document::new(1000.0, 500.0);
        page.images.insert(key.clone(), blob);
        let image = Node::new(
            page.alloc_id(),
            NodeKind::Image(Box::new(ImageObject {
                key,
                width: source.width,
                height: source.height,
                xf: Affine::scale(1.0 / 1.6),
                link: None,
                placement: Default::default(),
                pixels: None,
            })),
        );
        page.insert(Some(page.layers[0].id), usize::MAX, image).unwrap();
        assert_eq!(comparison_samples(&page), 4);
        assert_eq!(comparison_samples(&vector), 1, "ordinary vector pages retain their comparison");
        let mut renderer = vectorcraft_render::Renderer::new();
        let a = comparison_render(&mut renderer, &page, rect, 0.4, 1);
        let b = comparison_render(&mut vectorcraft_render::Renderer::new(), &vector, rect, 0.4, 1);
        assert!(compared(&a, &b, &[], &[]).solid > MAX_SOLID_DIFFERENCE, "the old point sampling falsely refused this page");
        let frame = Frame { page: &page, page_rect: rect, rect, whole: false };
        let mut view = PageView::new(&frame);
        assert!(compare(&frame, &mut view, &vector).is_ok());
        assert!(compare(&frame, &mut view, &Document::new(1000.0, 500.0)).is_err(), "missing artwork remains an error");
        let shifted = Rect::new(0.0, 0.0, 500.0, 500.0);
        assert!(view.difference(&vector, shifted).any > MAX_DIFFERENCE, "wrong framing remains an error");
    }
}
