use std::sync::Arc;

use super::*;
use serde_json::json;
use vectorcraft_engine::Session;

use crate::VectorcraftApp;

/// The straight RGBA of pixel (x, y).
fn pixel(img: &CustomCursorImage, x: usize, y: usize) -> [u8; 4] {
    let i = (y * img.size[0] as usize + x) * 4;
    [img.rgba[i], img.rgba[i + 1], img.rgba[i + 2], img.rgba[i + 3]]
}

/// The Anchor Point caret's hotspot is the middle of its hollow square: white there, with the
/// square's ink a few points above it.
#[test]
fn anchor_point_hotspot_is_inside_its_square() {
    for ppp in [1.0f32, 2.0] {
        let img = rasterize(Cursor::AnchorPoint, ppp).expect("anchor point has a bitmap");
        let [hx, hy] = img.hotspot.map(usize::from);
        assert_eq!(pixel(&img, hx, hy), [255, 255, 255, 255], "hollow at the hotspot at {ppp}x");
        let reach = (4.0 * ppp).round() as usize;
        let inked = (1..=reach.min(hy)).any(|d| {
            let p = pixel(&img, hx, hy - d);
            p[0] < 64 && p[3] > 200
        });
        assert!(inked, "ink on the square's top edge within 4 points above the hotspot at {ppp}x");
    }
}

/// Direct Selection's anchor square is hollow over an unselected path and filled over a
/// selected one; the arrow stays white. The Reshape Segment and copy arrowheads are black and
/// Group Selection's arrow white, just inside their tips.
#[test]
fn selection_arrows_are_white_or_black() {
    let at = |img: &CustomCursorImage, x: f32, y: f32, ppp: f32| {
        let [hx, hy] = img.hotspot.map(usize::from);
        pixel(img, hx + (x * ppp).round() as usize, hy + (y * ppp).round() as usize)
    };
    for ppp in [1.0f32, 2.0] {
        for c in [Cursor::DirectAnchor, Cursor::DirectAnchorSelected] {
            let img = rasterize(c, ppp).expect("a bitmap");
            let tip = at(&img, 1.5, 6.0, ppp);
            assert!(tip[0] > 192 && tip[3] > 200, "{c:?}'s arrow is white at {ppp}x: {tip:?}");
        }
        // Same arrow, so the filled square shows as more ink than the hollow one.
        let ink = |c| {
            let img = rasterize(c, ppp).expect("a bitmap");
            img.rgba.as_chunks::<4>().0.iter().filter(|p| p[0] < 64 && p[3] > 200).count()
        };
        let (hollow, filled) = (ink(Cursor::DirectAnchor), ink(Cursor::DirectAnchorSelected));
        assert!(filled > hollow, "the selected path's square is filled at {ppp}x: {filled} vs {hollow}");
        for (c, white) in [(Cursor::ReshapeSegment, false), (Cursor::SelectionCopy, false), (Cursor::GroupSelection, true)] {
            let img = rasterize(c, ppp).expect("a bitmap");
            let tip = at(&img, 1.5, 6.0, ppp);
            assert_eq!(tip[0] > 192, white, "{c:?} at {ppp}x: {tip:?}");
            assert!(tip[3] > 200, "{c:?} at {ppp}x: {tip:?}");
        }
    }
}

#[test]
fn a_cursor_not_drawn_yet_looks_like_the_one_it_borrows() {
    for c in GLYPHS {
        let Some(stand_in) = c.stand_in() else { continue };
        assert_eq!(rasterize(c, 2.0), rasterize(stand_in, 2.0), "{c:?} borrows {stand_in:?}");
    }
}

/// Loaded text after a click on the overflow out port is a text-block glyph, not the I-beam.
#[test]
fn loaded_text_cursor_is_not_the_text_i_beam() {
    assert!(Cursor::LoadedText.stand_in().is_none());
    let loaded = rasterize(Cursor::LoadedText, 2.0).expect("loaded text bitmap");
    let ibeam = rasterize(Cursor::Text, 2.0).expect("text bitmap");
    assert_ne!(loaded.rgba, ibeam.rgba);
    let [hx, hy] = loaded.hotspot.map(usize::from);
    assert!(pixel(&loaded, hx, hy)[3] > 200, "hotspot lies on the drawn frame");
}

#[test]
fn every_glyph_becomes_a_bitmap_with_its_hotspot_inside() {
    for ppp in [1.0, 1.25, 1.5, 2.0, 3.0] {
        for c in GLYPHS {
            let img = rasterize(c, ppp).unwrap_or_else(|| panic!("{c:?} at {ppp}"));
            let [w, h] = img.size.map(usize::from);
            assert_eq!(img.rgba.len(), w * h * 4, "{c:?}");
            assert!(img.hotspot[0] < img.size[0] && img.hotspot[1] < img.size[1], "{c:?} at {ppp}: {img:?}");
            assert!(w as f32 <= 50.0 * ppp && h as f32 <= 50.0 * ppp, "{c:?} at {ppp}: {img:?}");
            assert!(img.rgba.as_chunks::<4>().0.iter().any(|p| p[3] == 255), "{c:?}: nothing opaque");
        }
        // #1269: the normal Eyedropper is a pipette, with its tip at the hotspot; only
        // Use Precise Cursors turns it into a crosshair.
        let dropper = rasterize(Cursor::Eyedropper, ppp).unwrap();
        let precise = rasterize(Cursor::Eyedropper.precise(), ppp).unwrap();
        assert_ne!(dropper, precise, "Eyedropper must be distinguishable from its precise cursor at {ppp}");
        let [hx, hy] = dropper.hotspot.map(usize::from);
        let halo = (3.0 * ppp).ceil(); // The bitmap bounds round out to whole physical pixels.
        assert!(hx as f32 <= halo && f32::from(dropper.size[1]) - hy as f32 <= halo, "the tip is at the lower-left: {dropper:?}");
        assert!(pixel(&dropper, hx, hy)[3] > 0, "the tip covers the hotspot at {ppp}");
    }
    assert!(rasterize(Cursor::NotAllowed, 1.0).is_none(), "the busy states stay system cursors");
    assert!(rasterize(Cursor::Busy, 1.0).is_none());
    assert!(rasterize(Cursor::ScissorsNotAllowed, 1.0).is_none(), "until it's drawn it borrows the system not-allowed cursor");
    for ppp in [0.0, -1.0, f32::NAN, f32::INFINITY, 1e9] {
        assert!(rasterize(Cursor::Arrow, ppp).is_none(), "{ppp}");
    }
}

#[test]
fn the_arrow_bitmap_is_black_in_a_white_halo_with_the_tip_on_the_hotspot() {
    let img = rasterize(Cursor::Arrow, 1.0).unwrap();
    let [hx, hy] = img.hotspot.map(usize::from);
    // The tip covers the hotspot's pixel; the body is solid black, the outline's halo white.
    assert!(pixel(&img, hx, hy)[3] > 0, "{:?}", pixel(&img, hx, hy));
    let body = pixel(&img, hx + 2, hy + 8);
    assert!(body[3] == 255 && body[..3].iter().all(|&c| c < 40), "{body:?}");
    assert!(img.rgba.as_chunks::<4>().0.iter().any(|p| p[3] == 255 && p[..3].iter().all(|&c| c > 215)), "no white halo");
    // Nothing left of or above the tip but the halo.
    assert!((0..img.size[1] as usize).all(|y| pixel(&img, 0, y)[..3].iter().all(|&c| c > 100) || pixel(&img, 0, y)[3] < 128));
    // At twice the pixels per point, twice the pixels.
    let big = rasterize(Cursor::Arrow, 2.0).unwrap();
    for i in 0..2 {
        let ratio = f32::from(big.size[i]) / f32::from(img.size[i]);
        assert!((1.8..=2.2).contains(&ratio), "{:?} vs {:?}", big.size, img.size);
    }
    let [bx, by] = big.hotspot.map(usize::from);
    let body = pixel(&big, bx + 4, by + 16);
    assert!(body[3] == 255 && body[..3].iter().all(|&c| c < 40), "{body:?}");
}

/// The pen nib is a fountain pen's lying along the selection arrow's axis: its point on the
/// hotspot, the slit running back along it into the round breather hole, the heel clipped
/// at the back.
#[test]
fn the_pen_nib_has_a_slit_and_a_breather_hole() {
    for ppp in [1.0f32, 2.0] {
        let img = rasterize(Cursor::PenDrawing, ppp).expect("a bitmap");
        let [hx, hy] = img.hotspot.map(usize::from);
        let at = |x: f32, y: f32| {
            let (px, py) = ((hx as f32 + x * ppp) as usize, (hy as f32 + y * ppp) as usize);
            if px < img.size[0] as usize && py < img.size[1] as usize { pixel(&img, px, py) } else { [0; 4] }
        };
        // The breather hole, ~12 points along the axis ((4.6, 10.9) on screen): fully white.
        let hole = at(4.6, 10.9);
        assert!(hole[3] > 200 && hole[0] > 192, "the breather hole at {ppp}: {hole:?}");
        // The slit on the way to the hole: white over a black field.
        let slit = at(2.4, 5.5);
        assert!(slit[3] > 40 && slit[0] > 128, "the slit at {ppp}: {slit:?}");
        // The body past the hole is solid ink.
        let body = at(5.5, 12.9);
        assert!(body[3] > 200 && body[0] < 64, "the nib's body at {ppp}: {body:?}");
        // The heel's corners carry ink out near the bitmap's edge.
        for (x, y) in [(8.8f32, 13.1), (3.3, 15.4)] {
            let corner = at(x, y);
            assert!(corner[3] > 0, "a heel corner at {ppp} ({x}, {y}): {corner:?}");
        }
    }
}

/// The Pen drawing a path is the same nib as the new-path Pen, without the asterisk.
#[test]
fn the_drawing_pen_has_no_new_path_asterisk() {
    for ppp in [1.0, 2.0] {
        let (new, drawing) = (rasterize(Cursor::Pen, ppp).unwrap(), rasterize(Cursor::PenDrawing, ppp).unwrap());
        assert_eq!(new.hotspot, drawing.hotspot, "at {ppp}");
        // The asterisk's center, 16 × 15 points from the hotspot.
        let at = |img: &CustomCursorImage| {
            let [hx, hy] = img.hotspot.map(usize::from);
            let (x, y) = (hx + (16.0 * ppp) as usize, hy + (15.0 * ppp) as usize);
            if x < img.size[0] as usize && y < img.size[1] as usize { pixel(img, x, y)[3] } else { 0 }
        };
        assert_eq!(at(&new), 255, "the new-path Pen has its asterisk at {ppp}");
        assert_eq!(at(&drawing), 0, "the drawing Pen has no badge at {ppp}");
        // The nib is the same.
        let [hx, hy] = drawing.hotspot.map(usize::from);
        let nib = (hx + (8.0 * ppp) as usize, hy + (8.0 * ppp) as usize);
        assert_eq!(pixel(&new, nib.0, nib.1), pixel(&drawing, nib.0, nib.1), "at {ppp}");
    }
}

/// The Pencil's and the Paintbrush's closing cursors are the crosshair those tools show, with a
/// ring below right.
#[test]
fn the_closing_cursors_are_the_crosshair_with_a_ring() {
    for ppp in [1.0, 2.0] {
        let cross = rasterize(Cursor::Crosshair, ppp).unwrap();
        for c in [Cursor::PencilClose, Cursor::PaintbrushClose] {
            let img = rasterize(c, ppp).unwrap();
            assert_eq!(img.hotspot, cross.hotspot, "{c:?} at {ppp}");
            let [hx, hy] = img.hotspot.map(usize::from);
            let at = |img: &CustomCursorImage, dx: f32, dy: f32| {
                let (x, y) = ((hx as f32 + dx * ppp) as usize, (hy as f32 + dy * ppp) as usize);
                if x < img.size[0] as usize && y < img.size[1] as usize { pixel(img, x, y) } else { [0; 4] }
            };
            // Black pixels in the square 4 to 12 points right of and below the hotspot, clear of the arms.
            let ring_ink = |img: &CustomCursorImage| {
                let steps = (8.0 * ppp) as usize;
                (0..steps * steps)
                    .filter(|i| {
                        let p = at(img, 4.0 + (i % steps) as f32 / ppp, 4.0 + (i / steps) as f32 / ppp);
                        p[3] > 200 && p[0] < 64
                    })
                    .count()
            };
            assert!(ring_ink(&img) > 0, "{c:?} has its ring at {ppp}");
            assert_eq!(ring_ink(&cross), 0, "the crosshair has no ring at {ppp}");
            assert_eq!(at(&img, 8.0, 8.0)[3], 0, "{c:?}'s ring is hollow at {ppp}");
            // The arms are the crosshair's.
            assert_eq!(at(&img, -6.0, 0.0), at(&cross, -6.0, 0.0), "{c:?} at {ppp}");
        }
    }
}

/// The Pencil's new-path cursor is the crosshair with a small star below right, where the
/// closing cursor's ring is hollow and the straight-line cursor has a single line.
#[test]
fn the_pencil_new_path_cursor_is_the_crosshair_with_a_star() {
    for ppp in [1.0, 2.0] {
        let cross = rasterize(Cursor::Crosshair, ppp).unwrap();
        let img = rasterize(Cursor::PencilNew, ppp).unwrap();
        let ring = rasterize(Cursor::PencilClose, ppp).unwrap();
        let straight = rasterize(Cursor::PencilStraight, ppp).unwrap();
        assert_eq!(img.hotspot, cross.hotspot, "at {ppp}");
        let at = |img: &CustomCursorImage, dx: f32, dy: f32| {
            let [hx, hy] = img.hotspot.map(usize::from);
            let (x, y) = ((hx as f32 + dx * ppp) as usize, (hy as f32 + dy * ppp) as usize);
            if x < img.size[0] as usize && y < img.size[1] as usize { pixel(img, x, y) } else { [0; 4] }
        };
        let inked = |p: [u8; 4]| p[3] > 200 && p[0] < 64;
        assert!(inked(at(&img, 8.5, 8.5)), "the star's middle at {ppp}");
        assert_eq!(at(&ring, 8.5, 8.5)[3], 0, "the closing ring is hollow there at {ppp}");
        assert_eq!(at(&cross, 8.5, 8.5)[3], 0, "the crosshair has no badge at {ppp}");
        // Three strokes cross at the middle: the diagonal the straight-line cursor lacks, and the
        // upright one.
        assert!(inked(at(&img, 5.5, 5.5)) && !inked(at(&straight, 5.5, 5.5)), "the second diagonal at {ppp}");
        // At 1 point per pixel the upright stroke covers two pixel columns in gray.
        let dark = |p: [u8; 4]| p[3] > 200 && p[0] < 128;
        let upright = |img: &CustomCursorImage, dy: f32| [7.5, 8.0, 8.5].iter().any(|&dx| dark(at(img, dx, dy)));
        assert!(upright(&img, 4.5) && upright(&img, 11.5), "the upright stroke at {ppp}");
        assert!(!upright(&straight, 4.5) && !upright(&straight, 11.5), "the straight line has none at {ppp}");
        assert_eq!(at(&img, -6.0, 0.0), at(&cross, -6.0, 0.0), "the arms are the crosshair's at {ppp}");
    }
}

/// The Pencil's straight-line cursor is the crosshair with a short straight line below right,
/// through the point where the closing cursor's ring is hollow.
#[test]
fn the_pencil_straight_line_cursor_is_the_crosshair_with_a_line() {
    for ppp in [1.0, 2.0] {
        let cross = rasterize(Cursor::Crosshair, ppp).unwrap();
        let img = rasterize(Cursor::PencilStraight, ppp).unwrap();
        let ring = rasterize(Cursor::PencilClose, ppp).unwrap();
        assert_eq!(img.hotspot, cross.hotspot, "at {ppp}");
        let at = |img: &CustomCursorImage, dx: f32, dy: f32| {
            let [hx, hy] = img.hotspot.map(usize::from);
            let (x, y) = ((hx as f32 + dx * ppp) as usize, (hy as f32 + dy * ppp) as usize);
            if x < img.size[0] as usize && y < img.size[1] as usize { pixel(img, x, y) } else { [0; 4] }
        };
        let inked = |p: [u8; 4]| p[3] > 200 && p[0] < 64;
        assert!(inked(at(&img, 8.5, 8.5)), "the line crosses the badge's middle at {ppp}");
        assert_eq!(at(&ring, 8.5, 8.5)[3], 0, "the closing ring is hollow there at {ppp}");
        assert_eq!(at(&cross, 8.5, 8.5)[3], 0, "the crosshair has no badge at {ppp}");
        // Along the line, not across it: no ink off its ends.
        assert!(!inked(at(&img, 5.0, 5.0)) && !inked(at(&img, 12.0, 12.0)), "a line, not a cross, at {ppp}");
        assert_ne!(img, ring, "at {ppp}");
        assert_eq!(at(&img, -6.0, 0.0), at(&cross, -6.0, 0.0), "the arms are the crosshair's at {ppp}");
    }
}

/// The Pencil's continuation cursor is the straight-line cursor with a filled square on the
/// line's lower end, beside the line where the straight-line cursor has no ink.
#[test]
fn the_pencil_continuation_cursor_is_the_straight_line_cursor_with_an_end_point() {
    for ppp in [1.0, 2.0] {
        let cross = rasterize(Cursor::Crosshair, ppp).unwrap();
        let img = rasterize(Cursor::PencilContinue, ppp).unwrap();
        let straight = rasterize(Cursor::PencilStraight, ppp).unwrap();
        assert_eq!(img.hotspot, cross.hotspot, "at {ppp}");
        let at = |img: &CustomCursorImage, dx: f32, dy: f32| {
            let [hx, hy] = img.hotspot.map(usize::from);
            let (x, y) = ((hx as f32 + dx * ppp) as usize, (hy as f32 + dy * ppp) as usize);
            if x < img.size[0] as usize && y < img.size[1] as usize { pixel(img, x, y) } else { [0; 4] }
        };
        let inked = |p: [u8; 4]| p[3] > 200 && p[0] < 64;
        assert!(inked(at(&img, 8.5, 8.5)), "the line crosses the badge's middle at {ppp}");
        assert!(inked(at(&img, 3.5, 13.5)), "the end point's square at {ppp}");
        assert!(!inked(at(&straight, 3.5, 13.5)), "the straight-line cursor has no end point at {ppp}");
        assert!(!inked(at(&img, 12.0, 12.0)), "a line, not a cross, at {ppp}");
        assert_eq!(at(&img, -6.0, 0.0), at(&cross, -6.0, 0.0), "the arms are the crosshair's at {ppp}");
    }
}

#[test]
fn each_bitmap_is_made_once_so_the_os_cursor_is_kept() {
    let mut images = Images::default();
    let a = images.get(Cursor::Pen, 1.5).unwrap();
    let b = images.get(Cursor::Pen, 1.5).unwrap();
    assert!(Arc::ptr_eq(&a.rgba, &b.rgba));
    assert!(!Arc::ptr_eq(&a.rgba, &images.get(Cursor::Pen, 2.0).unwrap().rgba));
    assert!(images.get(Cursor::NotAllowed, 1.5).is_none());
    assert!(images.get(Cursor::Arrow, f32::NAN).is_none());
    assert_eq!(images.0.len(), 2);
}

/// One headless 800×600 frame of the whole window, a tenth of a second after the last.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
    let time = Some(ctx.input(|i| i.time) + 0.1);
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), time, events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| {
        app.logic(ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
    out
}

/// One [`frame`] with `focused` set: true for a window that has the focus, false while another app
/// or a sheet has it.
fn focus_frame(app: &mut VectorcraftApp, ctx: &egui::Context, focused: bool, events: Vec<egui::Event>) -> egui::FullOutput {
    let time = Some(ctx.input(|i| i.time) + 0.1);
    let screen_rect = Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)));
    let raw = egui::RawInput { screen_rect, time, focused, events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| {
        app.logic(ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
    out
}

#[test]
fn the_canvas_hands_the_tool_cursor_to_the_os_and_idles_while_the_pointer_rests() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let ctx = egui::Context::default();
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![]);
    }
    let canvas = app.canvas_rect.unwrap();
    let over = canvas.center();
    let out = frame(&mut app, &ctx, vec![egui::Event::PointerMoved(over)]);
    if OS_CURSORS {
        let img = out.platform_output.cursor_image.as_ref().expect("the Selection tool's arrow as an OS cursor");
        assert_eq!(img, &app.canvas.cursors.get(Cursor::Arrow, 1.0).unwrap());
        // Where the OS can't show the bitmap, its system cursor.
        assert_eq!(out.platform_output.cursor_icon, egui::CursorIcon::Default);
    } else {
        assert_eq!(out.platform_output.cursor_icon, egui::CursorIcon::None);
    }
    // The same bitmap while the pointer moves (the OS keeps its cursor), and an idle window
    // once it rests: no frame is asked for.
    let next = frame(&mut app, &ctx, vec![egui::Event::PointerMoved(over + vec2(5.0, 3.0))]);
    if OS_CURSORS {
        assert!(Arc::ptr_eq(&next.platform_output.cursor_image.unwrap().rgba, &out.platform_output.cursor_image.unwrap().rgba));
    }
    let mut idle = frame(&mut app, &ctx, vec![]);
    for _ in 0..3 {
        idle = frame(&mut app, &ctx, vec![]);
    }
    let delay = idle.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.repaint_delay).unwrap();
    assert!(delay > std::time::Duration::from_millis(100), "{delay:?} {:?}", ctx.repaint_causes());
    // Off the canvas, the panels' cursors.
    let off = frame(&mut app, &ctx, vec![egui::Event::PointerMoved(Pos2::new(canvas.left() - 20.0, canvas.center().y))]);
    assert!(off.platform_output.cursor_image.is_none());
}

/// On Wayland the glyph is painted and the pointer hidden over the canvas, on every build (#1645).
#[test]
fn a_wayland_window_paints_the_tool_cursor() {
    let mut app = VectorcraftApp::new(Session::new(), crate::Services { wayland: true, ..Default::default() });
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let ctx = egui::Context::default();
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![]);
    }
    let over = app.canvas_rect.unwrap().center();
    let out = frame(&mut app, &ctx, vec![egui::Event::PointerMoved(over)]);
    assert!(out.platform_output.cursor_image.is_none());
    assert_eq!(out.platform_output.cursor_icon, egui::CursorIcon::None);
}

/// Without a cursor theme the canvas keeps the system pointer visible (#2450).
#[test]
fn no_cursor_theme_keeps_the_system_pointer() {
    assert!(!hide_system_pointer(true));
    assert!(hide_system_pointer(false));
    let mut app = VectorcraftApp::new(Session::new(), crate::Services { wayland: true, no_cursor_theme: true, ..Default::default() });
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let ctx = egui::Context::default();
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![]);
    }
    let over = app.canvas_rect.unwrap().center();
    let out = frame(&mut app, &ctx, vec![egui::Event::PointerMoved(over)]);
    assert_ne!(out.platform_output.cursor_icon, egui::CursorIcon::None);
}

/// On macOS a window without the focus shows the system pointer over the canvas, and the glyph
/// is not painted next to it (#2020). With the focus back, the canvas sets `CursorIcon::None` again
/// on the next frame, without a pointer move.
#[test]
fn a_window_without_the_focus_shows_the_system_cursor_alone_on_macos() {
    // Wayland: every desktop build paints the glyphs.
    let mut app = VectorcraftApp::new(Session::new(), crate::Services { wayland: true, ..Default::default() });
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("tool.select", json!({"tool": "rectangle"})).unwrap();
    let ctx = egui::Context::default();
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![]);
    }
    let over = app.canvas_rect.unwrap().center();
    let key = focus_frame(&mut app, &ctx, true, vec![egui::Event::PointerMoved(over)]);
    assert_eq!(key.platform_output.cursor_icon, egui::CursorIcon::None, "with the focus: the glyph alone");
    let away = focus_frame(&mut app, &ctx, false, vec![egui::Event::WindowFocused(false), egui::Event::PointerMoved(over + vec2(5.0, 3.0))]);
    if HIDES_ONLY_WHEN_KEY {
        assert_eq!(away.platform_output.cursor_icon, egui::CursorIcon::Crosshair, "macOS without the focus: the system pointer alone");
    } else {
        assert_eq!(away.platform_output.cursor_icon, egui::CursorIcon::None, "other platforms: the glyph without the focus too");
    }
    let back = focus_frame(&mut app, &ctx, true, vec![egui::Event::WindowFocused(true)]);
    assert_eq!(back.platform_output.cursor_icon, egui::CursorIcon::None, "the focus back: the glyph alone");
}

/// User Interface › Scale Cursor Proportional to UI (#394): off, the tool cursors keep their
/// size whatever the UI Scaling; on, they grow with it.
#[test]
fn tool_cursors_scale_with_the_ui_only_when_asked() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("prefs.set", json!({"key": "uiScaling", "value": 2.0})).unwrap();
    let ctx = egui::Context::default();
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![]);
    }
    assert_eq!(ctx.zoom_factor(), 2.0);
    let over = app.canvas_rect.unwrap().center();
    let shown =
        |app: &mut VectorcraftApp, d: f32| frame(app, &ctx, vec![egui::Event::PointerMoved(over + vec2(d, 0.0))]).platform_output.cursor_image;
    if OS_CURSORS {
        assert_eq!(shown(&mut app, 0.0).unwrap(), app.canvas.cursors.get(Cursor::Arrow, 1.0).unwrap(), "off: its own size");
        app.run("prefs.set", json!({"key": "scaleCursorWithUi", "value": true})).unwrap();
        assert_eq!(shown(&mut app, 4.0).unwrap(), app.canvas.cursors.get(Cursor::Arrow, 2.0).unwrap(), "on: twice as big");
    }
    // Painted into the window (macOS, the web), the glyph is drawn at the scale asked for, its
    // hotspot where the pointer is.
    let painted = |scale: f32| {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("probe"));
            assert!(paint(&ui.ctx().layer_painter(layer), Cursor::Arrow, pos2(100.0, 100.0), scale));
        });
        out.textures_delta.clear();
        out.textures_delta.clear();
        out.shapes.iter().fold(egui::Rect::NOTHING, |r, c| r.union(c.shape.visual_bounding_rect()))
    };
    let (full, half) = (painted(1.0), painted(0.5));
    assert!((half.height() * 2.0 - full.height()).abs() < 1.5 && full.height() > 15.0, "{full:?} {half:?}");
    assert!(full.contains(pos2(100.0, 100.0)) && half.contains(pos2(100.0, 100.0)), "{full:?} {half:?}");
}

#[test]
fn wayland_windows_paint_the_glyphs() {
    assert!(os_cursors(true, false), "Windows, X11");
    assert!(!os_cursors(true, true), "Wayland: the compositor would size the bitmap (#1645)");
    assert!(!os_cursors(false, false) && !os_cursors(false, true), "macOS, the web");
}

#[test]
fn macos_windows_paint_the_glyphs_only_with_the_focus() {
    assert!(paint_glyphs(true, true), "macOS with the focus");
    assert!(!paint_glyphs(true, false), "macOS without the focus: the system pointer shows (#2020)");
    assert!(paint_glyphs(false, true) && paint_glyphs(false, false), "Wayland, the web: painted with the focus or without");
}

#[test]
fn arrow_hotspot_is_tip() {
    let pts = arrow_points(pos2(10.0, 20.0));
    assert_eq!(pts[0], pos2(10.0, 20.0));
    assert!(pts.iter().all(|q| q.x >= 10.0 && q.y >= 20.0));
}
