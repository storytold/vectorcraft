//! Gradient meshes in SVG export: SVG has no mesh paint, so a mesh is written as flat-coloured
//! pieces, each patch cut finely enough that neighbouring pieces differ by about one 8-bit
//! colour step at most, and no finer: a patch of one colour is one piece, not 64 alike.

use vectorcraft_color::Color;
use vectorcraft_doc::live::{GradientMesh, MeshAppearance};
use vectorcraft_doc::{Document, Node, NodeKind};
use vectorcraft_geom::{Rect, shapes};
use vectorcraft_svg::{ExportOptions, export};
use vectorcraft_testkit::raster::{Image, channel_diff, render_artboard};

/// A 200 × 200 pt document with a `rows` × `cols` mesh over (20, 20)–(180, 180), each point
/// coloured by `color(row, col)`.
fn doc(rows: u32, cols: u32, color: impl Fn(usize, usize) -> Color) -> Document {
    let mut m = GradientMesh::for_path(&shapes::rectangle(Rect::new(20.0, 20.0, 180.0, 180.0)), rows, cols, Color::BLACK, MeshAppearance::Flat, 0.0)
        .expect("a mesh");
    for r in 0..=rows as usize {
        for c in 0..=cols as usize {
            let i = m.idx(r, c);
            m.points[i].color = color(r, c);
        }
    }
    let mut d = Document::new(200.0, 200.0);
    let l = d.layers[0].id;
    let id = d.alloc_id();
    d.insert(Some(l), 0, Node::new(id, NodeKind::Mesh(m))).unwrap();
    d
}

/// Filled shapes in the SVG.
fn pieces(svg: &str) -> usize {
    ["<polygon", "<rect", "<path"].iter().map(|e| svg.matches(e).count()).sum()
}

/// The exported SVG rendered by resvg on white at 1 px/pt.
fn resvg_render(svg: &str) -> Image {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).expect("parse");
    let mut pm = resvg::tiny_skia::Pixmap::new(200, 200).unwrap();
    pm.fill(resvg::tiny_skia::Color::WHITE);
    resvg::render(&tree, resvg::tiny_skia::Transform::identity(), &mut pm.as_mut());
    Image { width: 200, height: 200, rgba: pm.data().to_vec() }
}

/// Compare the SVG as resvg draws it with the canvas, over the pixels `inside` the mesh (the
/// canvas grows a mesh's outline by half a pixel, so its edge is left out): at most `bad` of them
/// differ by more than `tol` in a channel.
fn assert_looks_alike(d: &Document, svg: &str, inside: impl Fn(f64, f64) -> bool, tol: u8, bad: f64) {
    let (mine, canvas) = (resvg_render(svg), render_artboard(d));
    let (mut n, mut off, mut worst) = (0usize, 0usize, 0u8);
    for y in 0..200 {
        for x in 0..200 {
            if !inside(x as f64 + 0.5, y as f64 + 0.5) {
                continue;
            }
            n += 1;
            let c = channel_diff(mine.pixel(x, y), canvas.pixel(x, y));
            worst = worst.max(c);
            if c > tol {
                off += 1;
            }
        }
    }
    assert!(n > 0 && (off as f64) <= bad * n as f64, "{off} of {n} pixels differ by more than {tol} (worst {worst})");
}

/// Inside the mesh square, two pixels in from its edge.
fn in_square(x: f64, y: f64) -> bool {
    (22.0..178.0).contains(&x) && (22.0..178.0).contains(&y)
}

#[test]
fn a_patch_of_one_colour_is_one_piece() {
    // 4 × 4 patches, every point the same colour: 16 pieces, not 16 × 64.
    let d = doc(4, 4, |_, _| Color::rgb8(40, 120, 200));
    let svg = export(&d, &ExportOptions::default());
    assert!(pieces(&svg) <= 16, "{} pieces", pieces(&svg));
    assert_looks_alike(&d, &svg, in_square, 2, 0.0);
}

#[test]
fn a_patch_is_cut_no_finer_than_its_colour_changes() {
    // Each patch changes by one 8-bit step from left to right: one piece is close enough.
    let d = doc(2, 2, |_, c| Color::rgb8(100 + c as u8, 100 + c as u8, 100 + c as u8));
    let svg = export(&d, &ExportOptions::default());
    assert!(pieces(&svg) <= 4, "{} pieces", pieces(&svg));
    assert_looks_alike(&d, &svg, in_square, 2, 0.0);
}

#[test]
fn a_strong_gradient_is_still_cut_finely() {
    // Black to white across one patch: as many pieces as before, so the steps stay small.
    let d = doc(1, 1, |_, c| if c == 0 { Color::BLACK } else { Color::WHITE });
    let svg = export(&d, &ExportOptions::default());
    assert!(pieces(&svg) >= 64, "{} pieces", pieces(&svg));
    // Flat pieces step where the canvas shades smoothly: within half a piece's step (255 / 16).
    assert_looks_alike(&d, &svg, in_square, 17, 0.0);
}

#[test]
fn a_curved_patch_of_one_colour_keeps_its_shape() {
    // A mesh over a circle: its patches' edges are curves, which one piece must still follow.
    let mut m =
        GradientMesh::for_path(&shapes::ellipse(Rect::new(20.0, 20.0, 180.0, 180.0)), 2, 2, Color::BLACK, MeshAppearance::Flat, 0.0).expect("a mesh");
    for p in &mut m.points {
        p.color = Color::rgb8(200, 40, 60);
    }
    let mut d = Document::new(200.0, 200.0);
    let l = d.layers[0].id;
    let id = d.alloc_id();
    d.insert(Some(l), 0, Node::new(id, NodeKind::Mesh(m))).unwrap();
    let svg = export(&d, &ExportOptions::default());
    assert!(pieces(&svg) <= 4, "{} pieces", pieces(&svg));
    // Two pixels in from the circle: the pieces' edges are straight between points 1/8 of a
    // patch edge apart, as the quads' were.
    assert_looks_alike(&d, &svg, |x, y| (x - 100.0).hypot(y - 100.0) < 78.0, 2, 0.0);
}
