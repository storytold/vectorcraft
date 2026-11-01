//! Gradient meshes written as PDF Type 6 (Coons patch) shadings (ISO 32000-1, 8.7.4.5.7): the
//! viewer blends each patch's corner colours smoothly, instead of the mesh being cut into small
//! flat-coloured paths that band, show seams and make the file large.

use vectorcraft_color::Color;
use vectorcraft_doc::live::{GradientMesh, MeshAppearance};
use vectorcraft_doc::{ColorMode, Document, Node, NodeKind};
use vectorcraft_geom::{Rect, shapes};
use vectorcraft_testkit::raster::{assert_similar, render_artboard};

use crate::*;

/// A 200 × 200 pt document with a 2 × 2 mesh over (20, 20)–(180, 180), its nine points coloured
/// by `color(row, col)` and made `opacity` opaque.
fn doc(color: impl Fn(usize, usize) -> Color, opacity: f32) -> Document {
    let mut m = GradientMesh::for_path(&shapes::rectangle(Rect::new(20.0, 20.0, 180.0, 180.0)), 2, 2, Color::BLACK, MeshAppearance::Flat, 0.0)
        .expect("a mesh");
    for r in 0..=2 {
        for c in 0..=2 {
            let i = m.idx(r, c);
            m.points[i].color = color(r, c);
            m.points[i].opacity = opacity;
        }
    }
    let mut d = Document::new(200.0, 200.0);
    let l = d.layers[0].id;
    let id = d.alloc_id();
    d.insert(Some(l), 0, Node::new(id, NodeKind::Mesh(m))).unwrap();
    d
}

fn rgb(r: usize, c: usize) -> Color {
    Color::rgb(r as f32 / 2.0, c as f32 / 2.0, 1.0 - (r + c) as f32 / 4.0)
}

/// How many meshes and paths `d` holds.
fn kinds(d: &Document) -> (usize, usize) {
    let (mut meshes, mut paths) = (0, 0);
    d.walk(|n| match n.kind {
        NodeKind::Mesh(_) => meshes += 1,
        NodeKind::Path { .. } => paths += 1,
        _ => {}
    });
    (meshes, paths)
}

#[test]
fn a_gradient_mesh_is_written_as_a_smooth_patch_shading() {
    let d = doc(rgb, 1.0);
    let bytes = export(&d, &PdfOptions::default()).unwrap();
    let back = import(&bytes).unwrap();
    // The shading comes back as a mesh, not as the 256 flat quads 8 × 8 a patch made.
    let (meshes, paths) = kinds(&back);
    assert!(meshes >= 1 && paths <= 4, "meshes {meshes}, paths {paths}");
    let mut corners = vec![];
    back.walk(|n| {
        if let NodeKind::Mesh(m) = &n.kind {
            corners.extend(m.points.iter().map(|p| p.color.to_rgb()));
        }
    });
    for want in [rgb(0, 0), rgb(2, 2), rgb(1, 1)] {
        let want = want.to_rgb();
        assert!(corners.iter().any(|c| c.iter().zip(want).all(|(a, b)| (a - b).abs() < 0.02)), "{want:?} in {corners:?}");
    }
    assert_similar(&render_artboard(&d), &render_artboard(&back), 6.0, 0.01);
}

#[test]
fn a_cmyk_mesh_is_written_smooth_in_its_inks() {
    let cmyk = |r: usize, c: usize| Color::Cmyk { c: r as f32 / 2.0, m: c as f32 / 2.0, y: 0.2, k: 0.0 };
    let mut d = doc(cmyk, 1.0);
    d.color_mode = ColorMode::Cmyk;
    let back = import(&export(&d, &PdfOptions::default()).unwrap()).unwrap();
    let (meshes, paths) = kinds(&back);
    assert!(meshes >= 1 && paths <= 4, "meshes {meshes}, paths {paths}");
    back.walk(|n| {
        if let NodeKind::Mesh(m) = &n.kind {
            assert!(m.points.iter().all(|p| matches!(p.color, Color::Cmyk { .. })), "{:?}", m.points);
        }
    });
    assert_similar(&render_artboard(&d), &render_artboard(&back), 6.0, 0.02);
}

#[test]
fn a_see_through_mesh_is_still_written_as_quads() {
    // Its opacities would need a soft mask of a second patch shading; the quads carry their own.
    let d = doc(rgb, 0.5);
    let back = import(&export(&d, &PdfOptions::default()).unwrap()).unwrap();
    assert_eq!(kinds(&back).0, 0);
    // The seams between quads, as before.
    assert_similar(&render_artboard(&d), &render_artboard(&back), 6.0, 0.05);
}

#[test]
fn a_mesh_of_mixed_colour_models_still_exports() {
    // RGB, grey and CMYK points: the PDF writer needs one colour space a shading.
    let mixed = |r: usize, c: usize| match (r + c) % 3 {
        0 => Color::rgb(1.0, 0.2, 0.2),
        1 => Color::gray(0.4),
        _ => Color::Cmyk { c: 0.8, m: 0.0, y: 0.3, k: 0.0 },
    };
    let d = doc(mixed, 1.0);
    let back = import(&export(&d, &PdfOptions::default()).unwrap()).unwrap();
    assert!(kinds(&back).0 >= 1);
    assert_similar(&render_artboard(&d), &render_artboard(&back), 8.0, 0.02);
}
