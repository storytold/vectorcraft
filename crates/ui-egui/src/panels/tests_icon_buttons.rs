//! Headless frames of the Align and Pathfinder panels' icon buttons: each icon fills its glyph square as the app's
//! Lucide icons do.

use egui::{ImageData, Rect, Shape, TextureId};
use serde_json::json;

use super::tests_appearance::{app_with_rect, run};
use super::*;
use crate::tests::panel_harness::Panel;

/// The share of its square a Lucide icon's art may span: the Lucide grid keeps 2 of its 24 units clear on each side.
const LUCIDE_ART: f32 = 20.0 / 24.0;

/// The longer side of the inked pixels of the icon drawn inside `rect`, as a share of the icon's image side.
fn ink_share(shapes: &[Shape], images: &[(TextureId, ImageData)], rect: Rect) -> f32 {
    let mut shares = shapes.iter().filter_map(|shape| match shape {
        Shape::Mesh(mesh) if mesh.texture_id != TextureId::default() && rect.contains_rect(mesh.calc_bounds()) => {
            let image = images.iter().find(|(id, _)| *id == mesh.texture_id).map(|(_, image)| image);
            let Some(ImageData::Color(image)) = image else { panic!("no image for the icon's texture in this frame") };
            let [w, h] = image.size;
            let inked = |x: usize, y: usize| image.pixels[y * w + x].a() > 0;
            let cols: Vec<usize> = (0..w).filter(|&x| (0..h).any(|y| inked(x, y))).collect();
            let rows: Vec<usize> = (0..h).filter(|&y| (0..w).any(|x| inked(x, y))).collect();
            let span = |v: &[usize]| v.first().zip(v.last()).map_or(0, |(a, b)| b - a + 1);
            Some(span(&cols).max(span(&rows)) as f32 / w.max(h) as f32)
        }
        _ => None,
    });
    shares.next().unwrap_or_else(|| panic!("no icon drawn in {rect:?}"))
}

/// Two selected rectangles, so every button is on.
fn two_rects() -> VectorcraftApp {
    let mut app = app_with_rect();
    run(&mut app, "shape.rectangle", json!({"x": 50, "y": 50, "width": 100, "height": 100}));
    run(&mut app, "select.all", json!({}));
    app
}

fn assert_full_size(draw: fn(&mut VectorcraftApp, &mut egui::Ui), names: &[&str]) {
    let mut panel = Panel::new(two_rects(), draw);
    // A new pixel density draws every icon again, so the frame carries their images.
    let (shapes, images) = panel.paint_at(2.0);
    for name in names {
        let share = ink_share(&shapes, &images, panel.rect(name));
        assert!(share >= LUCIDE_ART, "{name}: the icon spans {share} of its square, less than a Lucide icon's {LUCIDE_ART}");
    }
}

#[test]
fn the_align_panel_icons_fill_their_square() {
    let mut names: Vec<&str> = align::ALIGN.iter().chain(&align::DISTRIBUTE).map(|r| r.1).collect();
    names.extend(["Vertical Distribute Space", "Horizontal Distribute Space"]);
    assert_full_size(align::show, &names);
}

#[test]
fn the_pathfinder_panel_icons_fill_their_square() {
    let names: Vec<&str> = pathfinder::SHAPE_MODES.iter().chain(&pathfinder::PATHFINDERS).map(|r| r.1).collect();
    assert_full_size(pathfinder::show, &names);
}
