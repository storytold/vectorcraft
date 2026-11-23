use serde_json::json;
use vectorcraft_engine::Session;

use super::super::overlays::{anchor_square, curve_point, mark_options};
use super::super::{VectorcraftApp, show};
use super::*;

const SIZE: (f32, f32) = (400.0, 300.0);

fn id_of(made: &serde_json::Value) -> NodeId {
    NodeId(made["id"].as_u64().unwrap())
}

/// A curve with handles, a live rectangle, a group, point type and a symbol instance, all selected.
fn scene() -> (VectorcraftApp, NodeId) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let curve = id_of(&app.run("path.create", json!({"d": "M40 40 C60 10 90 10 110 40 S160 70 180 40 L180 90"})).unwrap());
    app.run("shape.rectangle", json!({"x": 200.5, "y": 30.25, "width": 60, "height": 40})).unwrap();
    let a = id_of(&app.run("shape.ellipse", json!({"x": 40, "y": 120, "width": 50, "height": 30})).unwrap());
    let b = id_of(&app.run("shape.rectangle", json!({"x": 100, "y": 120, "width": 30, "height": 30})).unwrap());
    app.run("select.set", json!({"ids": [a.0, b.0]})).unwrap();
    app.run("object.group", json!({})).unwrap();
    app.run("text.create", json!({"x": 200, "y": 150, "text": "Mark", "size": 20})).unwrap();
    let art = id_of(&app.run("shape.rectangle", json!({"x": 280, "y": 200, "width": 20, "height": 20})).unwrap());
    app.run("symbol.new", json!({"name": "Mark", "ids": [art.0]})).unwrap();
    app.run("select.all", json!({})).unwrap();
    (app, curve)
}

fn xf(zoom: f64, center: (f64, f64), rot_degrees: f64) -> Xf {
    Xf {
        rect: egui::Rect::from_min_size(Pos2::ZERO, vec2(SIZE.0, SIZE.1)),
        zoom,
        center: Point::new(center.0, center.1),
        rot: rot_degrees.to_radians(),
    }
}

const VIEW: (f64, f64) = (200.0, 150.0);

/// The marks drawn one shape at a time, as a frame draws them with nothing kept.
fn plain(app: &VectorcraftApp, p: &egui::Painter, xf: &Xf, curve_points_from_tool: bool) {
    let st = app.session.active().unwrap();
    let MarkOptions { direct, curvature, look, all_handles, .. } = mark_options(app, curve_points_from_tool);
    let symbols = st.doc.symbol_boxes();
    let anchor = |direct: bool| look.grow + if direct { 5.0 } else { 4.0 };
    let (mut handles, mut with_handles) = (vec![], 0);
    if let Some((k, n)) = st.selection.key.and_then(|k| Some((k, st.doc.node(k)?))) {
        for line in to_screen_path(&node_outline(n, &symbols), xf) {
            p.add(Shape::line(line, Stroke::new(KEY_OUTLINE, c32(st.doc.layer_color(k)))));
        }
    }
    for id in &st.selection.objects {
        let Some(n) = st.doc.node(*id) else { continue };
        let color = c32(st.doc.layer_color(*id));
        let partial = st.selection.partial(*id);
        let stroke = |bp: &BezPath, width: f32| {
            for line in to_screen_path(bp, xf) {
                p.add(Shape::line(line, Stroke::new(width, color)));
            }
        };
        if !matches!(n.kind, NodeKind::Text(ref tx) if matches!(tx.kind, vectorcraft_doc::TextKind::Point)) {
            stroke(&node_outline(n, &symbols), 1.0);
        }
        stroke(&linked_crosses(n), 1.0);
        walk_drawn(n, &mut |c| {
            let envelope = vectorcraft_doc::live::envelope_overlay(c).and_then(|(_, grid)| grid);
            let grid = match &c.kind {
                NodeKind::Mesh(m) => Some(m),
                _ => envelope.as_ref(),
            };
            if let Some(grid) = grid {
                for q in &grid.points {
                    anchor_square(p, xf.to_screen(q.p), color, false, anchor(direct));
                }
                return;
            }
            let NodeKind::Path { path, .. } = &c.kind else { return };
            if curvature && curve_points_from_tool {
                return;
            }
            let selected_segments = st.selection.segments.get(&c.id);
            if let Some(segments) = selected_segments {
                for &(si, seg) in segments {
                    let Some(sp) = path.subpaths.get(si).filter(|sp| seg < sp.segment_count()) else { continue };
                    let curve = sp.segment(seg);
                    let mut outline = vectorcraft_geom::BezPath::new();
                    outline.move_to(curve.p0);
                    if sp.segment_is_line(seg) {
                        outline.line_to(curve.p3);
                    } else {
                        outline.curve_to(curve.p1, curve.p2, curve.p3);
                    }
                    stroke(&outline, 2.0);
                }
            }
            for (si, ai, a) in path.anchors() {
                let (sel, handles_shown) = match partial {
                    Some(set) => {
                        let sel = set.contains(&(si, ai));
                        (sel, sel)
                    }
                    None => (true, direct && c.id == *id),
                };
                let sp = xf.to_screen(a.p);
                if !curvature && handles_shown {
                    with_handles += 1;
                    let sides = path.subpaths.get(si).map(|subpath| st.selection.handle_sides(c.id, si, ai, subpath)).unwrap_or((true, true));
                    handles.extend(
                        [(a.h_in, sides.0), (a.h_out, sides.1)]
                            .into_iter()
                            .filter(|(h, show)| *show && h.distance(a.p) > 1e-6)
                            .map(|(h, _)| (sp, xf.to_screen(h), color)),
                    );
                }
                let anchor_selected = sel && selected_segments.is_none();
                let ink = if anchor_selected && partial.is_some() { selected_anchor(color) } else { color };
                if curvature {
                    curve_point(p, sp, ink, anchor_selected && partial.is_some(), look.grow + 6.0);
                } else {
                    anchor_square(p, sp, ink, anchor_selected, anchor(partial.is_some() || direct));
                }
            }
        });
        if n.shows_center()
            && let Some(b) = n.geometric_bounds_with(&symbols)
        {
            anchor_square(p, xf.to_screen(b.center()), color, true, anchor(false));
        }
        if let NodeKind::Text(tx) = &n.kind
            && matches!(tx.kind, vectorcraft_doc::TextKind::Point)
        {
            for (a, b) in tx.baselines() {
                p.line_segment([xf.to_screen(tx.xf * a), xf.to_screen(tx.xf * b)], Stroke::new(1.0, color));
            }
            p.circle_filled(xf.to_screen(tx.xf * Point::ZERO), 2.5, color);
        }
    }
    if all_handles || with_handles <= 1 {
        for (sp, hp, color) in handles {
            p.line_segment([sp, hp], Stroke::new(1.0, color));
            for shape in look.shapes(hp, color).into_iter().flatten() {
                p.add(shape);
            }
        }
    }
}

/// Premultiplied pixels of a frame.
struct Picture {
    size: (usize, usize),
    pixels: Vec<[f32; 4]>,
}

/// The font texture at `uv`, filtered linearly as the GPU does.
fn sample(atlas: &egui::ColorImage, uv: (f32, f32)) -> [f32; 4] {
    let (w, h) = (atlas.width(), atlas.height());
    let (x, y) = (uv.0 * w as f32 - 0.5, uv.1 * h as f32 - 0.5);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let texel =
        |dx: f32, dy: f32| atlas[(((x.floor() + dx).max(0.0) as usize).min(w - 1), ((y.floor() + dy).max(0.0) as usize).min(h - 1))].to_array();
    let (a, b, c, d) = (texel(0.0, 0.0), texel(1.0, 0.0), texel(0.0, 1.0), texel(1.0, 1.0));
    std::array::from_fn(|i| {
        let top = f32::from(a[i]) * (1.0 - fx) + f32::from(b[i]) * fx;
        let bottom = f32::from(c[i]) * (1.0 - fx) + f32::from(d[i]) * fx;
        top * (1.0 - fy) + bottom * fy
    })
}

fn rasterize(primitives: &[egui::ClippedPrimitive], pixels_per_point: f32, atlas: &egui::ColorImage) -> Picture {
    let size = ((SIZE.0 * pixels_per_point).ceil() as usize, (SIZE.1 * pixels_per_point).ceil() as usize);
    let mut pixels = vec![[0.0f32; 4]; size.0 * size.1];
    // Vertices snap to 1/256 of a pixel, as on a GPU, so that the edge tests are exact and a pixel
    // center on an edge two triangles share belongs to exactly one of them.
    const GRID: f32 = 256.0;
    let cross =
        |a: (i64, i64), b: (i64, i64), c: (i64, i64)| i128::from(b.0 - a.0) * i128::from(c.1 - a.1) - i128::from(b.1 - a.1) * i128::from(c.0 - a.0);
    for primitive in primitives {
        let Primitive::Mesh(mesh) = &primitive.primitive else { continue };
        let clip = primitive.clip_rect;
        for [i, j, k] in mesh.indices.as_chunks::<3>().0 {
            let mut v = [mesh.vertices[*i as usize], mesh.vertices[*j as usize], mesh.vertices[*k as usize]];
            let at = |v: &Vertex| ((v.pos.x * pixels_per_point * GRID).round() as i64, (v.pos.y * pixels_per_point * GRID).round() as i64);
            let area = cross(at(&v[0]), at(&v[1]), at(&v[2]));
            if area == 0 {
                continue;
            }
            if area < 0 {
                v.swap(1, 2);
            }
            let area = area.abs() as f32;
            let [a, b, c] = [at(&v[0]), at(&v[1]), at(&v[2])];
            let range = |lo: i64, hi: i64, clip_lo: f32, clip_hi: f32, n: usize| {
                let lo = (lo as f32 / GRID).max(clip_lo * pixels_per_point).floor().max(0.0) as usize;
                let hi = ((hi as f32 / GRID).min(clip_hi * pixels_per_point).ceil().max(0.0) as usize).min(n);
                lo..hi
            };
            let upper_left = |p: (i64, i64), q: (i64, i64)| q.1 < p.1 || (q.1 == p.1 && q.0 > p.0);
            for y in range(a.1.min(b.1).min(c.1), a.1.max(b.1).max(c.1), clip.min.y, clip.max.y, size.1) {
                for x in range(a.0.min(b.0).min(c.0), a.0.max(b.0).max(c.0), clip.min.x, clip.max.x, size.0) {
                    let q = (x as i64 * 256 + 128, y as i64 * 256 + 128);
                    let (w0, w1, w2) = (cross(b, c, q), cross(c, a, q), cross(a, b, q));
                    let inside = |w: i128, p, q| w > 0 || (w == 0 && upper_left(p, q));
                    if !(inside(w0, b, c) && inside(w1, c, a) && inside(w2, a, b)) {
                        continue;
                    }
                    let (w0, w1, w2) = (w0 as f32 / area, w1 as f32 / area, w2 as f32 / area);
                    let uv = (w0 * v[0].uv.x + w1 * v[1].uv.x + w2 * v[2].uv.x, w0 * v[0].uv.y + w1 * v[1].uv.y + w2 * v[2].uv.y);
                    let texel = sample(atlas, uv);
                    let dst = &mut pixels[y * size.0 + x];
                    let channels: [[u8; 4]; 3] = [v[0].color.to_array(), v[1].color.to_array(), v[2].color.to_array()];
                    let src: [f32; 4] = std::array::from_fn(|i| {
                        (w0 * f32::from(channels[0][i]) + w1 * f32::from(channels[1][i]) + w2 * f32::from(channels[2][i])) * texel[i] / 255.0
                    });
                    let keep = 1.0 - src[3] / 255.0;
                    for i in 0..4 {
                        dst[i] = src[i] + dst[i] * keep;
                    }
                }
            }
        }
    }
    Picture { size, pixels }
}

fn picture(ctx: &egui::Context, pixels_per_point: f32, draw: impl FnOnce(&egui::Painter)) -> Picture {
    // The window is larger than the picture at each display scale, and a new scale applies to the
    // pass after it is set.
    let raw = || egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(4.0 * SIZE.0, 4.0 * SIZE.1))),
        max_texture_side: Some(8192),
        ..Default::default()
    };
    ctx.set_pixels_per_point(pixels_per_point);
    ctx.run_ui(raw(), |_| {}).textures_delta.clear();
    let mut draw = Some(draw);
    let mut out = ctx.run_ui(raw(), |ui| {
        if let Some(draw) = draw.take() {
            draw(&ui.painter_at(egui::Rect::from_min_size(Pos2::ZERO, vec2(SIZE.0, SIZE.1))));
        }
    });
    out.textures_delta.clear();
    let primitives = ctx.tessellate(std::mem::take(&mut out.shapes), out.pixels_per_point);
    rasterize(&primitives, out.pixels_per_point, &ctx.fonts(|f| f.image()))
}

/// The most a color channel may differ, out of 255: kept vertices move by rounding errors far under
/// a pixel.
const CHANNEL: f32 = 3.0;

fn assert_same(a: &Picture, b: &Picture, what: &str, allowed_per_mille: usize) {
    assert_eq!(a.size, b.size, "{what}");
    let drawn = b.pixels.iter().filter(|p| p[3] > 0.0).count();
    assert!(drawn > 0, "{what}: the picture is empty");
    let differ: Vec<usize> = (0..b.pixels.len()).filter(|i| a.pixels[*i].iter().zip(&b.pixels[*i]).any(|(x, y)| (x - y).abs() > CHANNEL)).collect();
    let first: Vec<_> = differ.iter().take(4).map(|i| ((i % a.size.0, i / a.size.0), a.pixels[*i], b.pixels[*i])).collect();
    assert!(
        differ.len() * 1000 <= drawn * allowed_per_mille,
        "{what}: {} of {drawn} drawn pixels differ, first at (pixel, got, want): {first:?}",
        differ.len()
    );
}

fn paint(marks: &mut SelectionMarks, app: &VectorcraftApp, p: &egui::Painter, xf: &Xf, curve_points_from_tool: bool) {
    let st = app.session.active().unwrap();
    let key = st.selection.key;
    marks.paint(st, p, xf, &mark_options(app, curve_points_from_tool), key, &st.selection.objects);
}

/// Kept marks, marks built from nothing and plain shapes all draw the same pixels. Kept marks may
/// differ at up to 1 pixel in 1,000: at a corner of exactly 90 degrees on a rotated view, plain
/// shapes take a miter or a cut corner by a rounding error of the corner's position, which a pan
/// changes, and moved triangles keep the corner they were built with. An anchor square left stale
/// covers more than this limit.
#[track_caller]
fn assert_matches_plain(ctx: &egui::Context, marks: &mut SelectionMarks, app: &VectorcraftApp, xf: &Xf, pixels_per_point: f32, what: &str) {
    let want = picture(ctx, pixels_per_point, |p| plain(app, p, xf, false));
    for frame in ["this frame", "the next frame"] {
        let kept = picture(ctx, pixels_per_point, |p| paint(marks, app, p, xf, false));
        assert_same(&kept, &want, &format!("{what}, kept, {frame}"), 1);
    }
    let mut fresh = SelectionMarks::default();
    for frame in ["drawn once", "built from nothing"] {
        let cold = picture(ctx, pixels_per_point, |p| paint(&mut fresh, app, p, xf, false));
        assert_same(&cold, &want, &format!("{what}, {frame}"), 0);
    }
    assert!(fresh.builds > 0, "{what}: the second frame builds the marks");
}

#[test]
fn marks_draw_the_same_pixels_as_plain_shapes_at_each_view() {
    let (mut app, _) = scene();
    let ctx = egui::Context::default();
    for tool in ["selection", "directSelection", "curvature"] {
        app.select_tool(tool);
        for pixels_per_point in [1.0, 1.25, 2.0] {
            let mut marks = SelectionMarks::default();
            for zoom in [0.5, 1.0, 2.37, 8.0] {
                for rot in [0.0, 30.0, 90.0] {
                    let view = xf(zoom, VIEW, rot);
                    assert_matches_plain(&ctx, &mut marks, &app, &view, pixels_per_point, &format!("{tool} {pixels_per_point}x {zoom} {rot}"));
                }
            }
        }
    }
}

#[test]
fn a_pan_moves_the_marks_without_building_them() {
    let (mut app, _) = scene();
    let ctx = egui::Context::default();
    for tool in ["selection", "directSelection"] {
        app.select_tool(tool);
        for (pixels_per_point, rot) in [(1.0, 0.0), (1.25, 0.0), (2.0, 30.0)] {
            let mut marks = SelectionMarks::default();
            assert_matches_plain(&ctx, &mut marks, &app, &xf(1.5, VIEW, rot), pixels_per_point, &format!("{tool}, before the pan"));
            let builds = marks.builds;
            for (dx, dy) in [(10.0, 0.0), (3.0, 7.0), (0.37, 0.61), (-31.3, 12.9), (160.0, 90.0)] {
                let view = xf(1.5, (VIEW.0 + dx, VIEW.1 + dy), rot);
                assert_matches_plain(&ctx, &mut marks, &app, &view, pixels_per_point, &format!("{tool}, {pixels_per_point}x pan {dx} {dy}"));
            }
            assert_eq!(marks.builds, builds, "{tool}: a pan builds nothing");
        }
    }
}

#[test]
fn a_long_pan_at_high_zoom_builds_the_marks_again_and_stays_exact() {
    let (app, _) = scene();
    let ctx = egui::Context::default();
    let mut marks = SelectionMarks::default();
    // 2,000 points per document point: the art runs far past what `f32` holds to a fine fraction.
    let zoom = 2_000.0;
    assert_matches_plain(&ctx, &mut marks, &app, &xf(zoom, (180.0, 41.0), 0.0), 1.0, "the top of the curve's straight end");
    let builds = marks.builds;
    assert_matches_plain(&ctx, &mut marks, &app, &xf(zoom, (180.0, 89.0), 0.0), 1.0, "96,000 points along the straight end");
    assert!(marks.builds > builds, "a pan past MAX_PAN builds the marks again");
}

/// Marks land exactly where the rest of the canvas puts a point, also at the canvas's far edge at
/// the highest zoom.
#[test]
fn marks_are_placed_exactly_where_the_view_puts_a_point() {
    let far = vectorcraft_doc::MAX_COORD - 1.0;
    for (center, zoom, rot) in [(VIEW, 1.0, 0.0), (VIEW, 2.37, 30.0), ((far, far), 640.0, 0.0), ((far, -far), 640.0, 30.0)] {
        let view = xf(zoom, center, rot);
        let map = ViewMap::new(&view);
        for i in 0..2_000 {
            let t = f64::from(i) / 7.0;
            let p = Point::new(center.0 + t.sin() * 0.3, center.1 + t.cos() * 0.2);
            assert_eq!(map.to_screen(p), view.to_screen(p), "{p:?} at zoom {zoom}, rotation {rot}");
        }
    }
}

#[test]
fn an_idle_frame_draws_the_mesh_it_joined() {
    let (app, _) = scene();
    let ctx = egui::Context::default();
    let mut marks = SelectionMarks::default();
    let view = xf(1.0, VIEW, 0.0);
    picture(&ctx, 1.0, |p| paint(&mut marks, &app, p, &view, false));
    assert_eq!((marks.builds, marks.composes), (0, 0), "the first frame at a view keeps nothing");
    picture(&ctx, 1.0, |p| paint(&mut marks, &app, p, &view, false));
    let (builds, composes) = (marks.builds, marks.composes);
    assert!(builds > 0 && composes == 1);
    assert_matches_plain(&ctx, &mut marks, &app, &view, 1.0, "an idle frame");
    assert_eq!((marks.builds, marks.composes), (builds, composes), "an idle frame builds and joins nothing");
}

/// While the zoom or rotation changes in each frame, nothing is built or kept. The picture stays the
/// same, and the first frame that repeats a view builds the marks.
#[test]
fn a_zoom_keeps_nothing_until_the_view_stays() {
    let (mut app, _) = scene();
    let ctx = egui::Context::default();
    app.select_tool("directSelection");
    let mut marks = SelectionMarks::default();
    let settled = xf(1.5, VIEW, 0.0);
    assert_matches_plain(&ctx, &mut marks, &app, &settled, 1.0, "before the zoom");
    let builds = marks.builds;
    for (zoom, rot) in [(1.6, 0.0), (1.7, 0.0), (1.7, 15.0), (1.9, 15.0), (2.37, 40.0)] {
        let view = xf(zoom, VIEW, rot);
        let want = picture(&ctx, 1.0, |p| plain(&app, p, &view, false));
        let drawn = picture(&ctx, 1.0, |p| paint(&mut marks, &app, p, &view, false));
        assert_same(&drawn, &want, &format!("zoom {zoom}, rotation {rot}"), 0);
        assert_eq!(marks.builds, builds, "zoom {zoom}, rotation {rot} builds nothing");
        assert!(marks.objects.is_empty() && marks.frame.is_empty(), "zoom {zoom}, rotation {rot} keeps nothing");
    }
    let last = xf(2.37, VIEW, 40.0);
    let want = picture(&ctx, 1.0, |p| plain(&app, p, &last, false));
    let drawn = picture(&ctx, 1.0, |p| paint(&mut marks, &app, p, &last, false));
    assert_same(&drawn, &want, "the same view again", 1);
    assert!(marks.builds > builds && !marks.objects.is_empty(), "the same view again builds the marks");
}

/// Whether every vertex of the kept frame lies within a few pixels of the picture.
fn all_near_the_picture(marks: &SelectionMarks) -> bool {
    let near = egui::Rect::from_min_size(Pos2::ZERO, vec2(SIZE.0, SIZE.1)).expand(8.0);
    marks.frame.vertices.iter().all(|v| near.contains(v.pos))
}

/// A group of rectangles 1,000 points apart keeps triangles only for the ones in view. One that
/// comes into view gets its triangles then.
#[test]
fn a_frame_holds_no_triangles_of_a_groups_subpaths_out_of_view() {
    for tool in ["selection", "directSelection"] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        for i in 0..12 {
            app.run("shape.rectangle", json!({"x": 40 + i * 1000, "y": 40, "width": 60, "height": 40})).unwrap();
        }
        app.run("select.all", json!({})).unwrap();
        app.run("object.group", json!({})).unwrap();
        app.run("select.all", json!({})).unwrap();
        app.select_tool(tool);
        let ctx = egui::Context::default();
        let mut marks = SelectionMarks::default();
        for (dx, seen) in [(0.0, 1), (1000.0, 2), (1000.37, 2), (11_000.0, 3), (0.0, 3)] {
            let view = xf(1.0, (VIEW.0 + dx, VIEW.1), 0.0);
            assert_matches_plain(&ctx, &mut marks, &app, &view, 1.0, &format!("{tool}, pan {dx}"));
            assert!(!marks.frame.is_empty(), "{tool}, pan {dx}: a rectangle is in view");
            assert!(all_near_the_picture(&marks), "{tool}, pan {dx}: triangles of a rectangle out of view");
            let waiting: usize = marks
                .objects
                .values()
                .flat_map(|entry| &entry.marks)
                .map(|mark| match mark {
                    Mark::Rigid { pieces, .. } => pieces.iter().filter(|piece| piece.shape.is_some()).count(),
                    _ => 0,
                })
                .sum();
            assert_eq!(waiting, 12 - seen, "{tool}, pan {dx}: rectangles not yet in view get no triangles");
        }
    }
}

/// How many triangles the direction handles add to a frame of a direct-selected path with `anchors`
/// anchors 1,000 points apart.
fn handle_triangles(anchors: usize, view: &Xf) -> (usize, VectorcraftApp, SelectionMarks, egui::Context) {
    let mut d = String::from("M40 150");
    for i in 0..anchors - 1 {
        let (from, to) = (40 + i * 1000, 40 + (i + 1) * 1000);
        // Handles alternate between level, which the tessellator rounds to whole pixels, and slanted.
        let y = if i % 2 == 0 { 150 } else { 110 };
        d += &format!(" C{} {y} {} 190 {to} 150", from + 30, to - 30);
    }
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("path.create", json!({"d": d})).unwrap();
    app.run("select.all", json!({})).unwrap();
    app.select_tool("directSelection");
    let ctx = egui::Context::default();
    let mut marks = SelectionMarks::default();
    let mut triangles = |app: &mut VectorcraftApp, all_handles: bool| {
        app.session.prefs.show_handles_multiple_anchors = all_handles;
        for _ in 0..2 {
            picture(&ctx, 1.0, |p| paint(&mut marks, app, p, view, false));
        }
        marks.frame.indices.len() / 3
    };
    let without = triangles(&mut app, false);
    let with = triangles(&mut app, true);
    (with - without, app, marks, ctx)
}

/// A frame of a long path shows the handles in view and holds the triangles of those alone, exactly
/// as many as a path that ends just past the view. Handles that come into view draw correctly, level
/// ones after a move by a fraction of a pixel too.
#[test]
fn a_frame_holds_no_triangles_of_handles_out_of_view() {
    let view = xf(1.0, VIEW, 0.0);
    let (short, ..) = handle_triangles(2, &view);
    let (long, app, mut marks, ctx) = handle_triangles(40, &view);
    assert!(short > 0, "the handle in view draws");
    assert_eq!(long, short, "the long path's handles out of view add nothing");
    for (dx, dy) in [(0.0, 0.0), (0.0, 0.37), (1000.0, 0.0), (1000.61, 0.61), (2000.0, 0.0), (2000.0, 0.5), (5000.5, 0.25), (0.0, 0.0)] {
        let view = xf(1.0, (VIEW.0 + dx, VIEW.1 + dy), 0.0);
        assert_matches_plain(&ctx, &mut marks, &app, &view, 1.0, &format!("pan {dx} {dy}"));
    }
}

#[test]
fn no_stale_marks_after_a_change_with_the_selection_tool() {
    no_stale_marks_after_a_change("selection");
}

#[test]
fn no_stale_marks_after_a_change_with_the_direct_selection_tool() {
    no_stale_marks_after_a_change("directSelection");
}

/// Each change draws what plain shapes draw, and builds no more than the change touches.
fn no_stale_marks_after_a_change(tool: &str) {
    let (mut app, curve) = scene();
    let ctx = egui::Context::default();
    let mut marks = SelectionMarks::default();
    let view = xf(1.5, VIEW, 30.0);
    app.select_tool(tool);
    assert_matches_plain(&ctx, &mut marks, &app, &view, 1.0, "at the start");
    let all = app.session.active().unwrap().selection.objects.clone();
    let ids = |list: &[NodeId]| json!({"ids": list.iter().map(|i| i.0).collect::<Vec<_>>()});

    // An edit of one object builds that object, and the symbol instance that measures the art of a
    // document at a new revision.
    let builds = marks.builds;
    app.run("select.set", json!({"ids": [curve.0]})).unwrap();
    app.run("object.nudge", json!({"dx": 12, "dy": -5})).unwrap();
    app.run("select.set", ids(&all)).unwrap();
    assert_matches_plain(&ctx, &mut marks, &app, &view, 1.0, "after an edit");
    assert_eq!(marks.builds, builds + 2, "an edit of one object leaves the others as they were");

    app.run("edit.undo", json!({})).unwrap();
    app.run("select.set", ids(&all)).unwrap();
    assert_matches_plain(&ctx, &mut marks, &app, &view, 1.0, "after an undo");
    app.run("edit.redo", json!({})).unwrap();
    app.run("select.set", ids(&all)).unwrap();
    assert_matches_plain(&ctx, &mut marks, &app, &view, 1.0, "after a redo");

    // A layer color.
    let layer = app.session.active().unwrap().doc.layers[0].id;
    app.run("layer.setProps", json!({"id": layer.0, "color": "#ff00aa"})).unwrap();
    app.run("select.set", ids(&all)).unwrap();
    assert_matches_plain(&ctx, &mut marks, &app, &view, 1.0, "after a layer color change");

    // A change of the selection: fewer objects, other anchors, a segment, the key object.
    app.run("select.set", ids(&all[..2])).unwrap();
    assert_matches_plain(&ctx, &mut marks, &app, &view, 1.0, "with fewer objects");
    assert_eq!(marks.objects.len(), 2, "an object that left the selection has no entry");
    let st = app.session.active_mut().unwrap();
    st.selection.anchors.insert(curve, [(0, 1)].into_iter().collect());
    assert_matches_plain(&ctx, &mut marks, &app, &view, 1.0, "with one anchor of the curve selected");
    let st = app.session.active_mut().unwrap();
    st.selection.anchors.insert(curve, [(0, 2), (0, 3)].into_iter().collect());
    assert_matches_plain(&ctx, &mut marks, &app, &view, 1.0, "with other anchors of the curve selected");
    let st = app.session.active_mut().unwrap();
    st.selection.segments.insert(curve, [(0, 2)].into_iter().collect());
    assert_matches_plain(&ctx, &mut marks, &app, &view, 1.0, "with a segment of the curve selected");
    let st = app.session.active_mut().unwrap();
    st.selection.segments.clear();
    st.selection.anchors.clear();
    st.selection.key = Some(curve);
    assert_matches_plain(&ctx, &mut marks, &app, &view, 1.0, "with a key object");
    let st = app.session.active_mut().unwrap();
    st.selection.key = Some(all[1]);
    assert_matches_plain(&ctx, &mut marks, &app, &view, 1.0, "with another key object");
    app.session.active_mut().unwrap().selection.key = None;

    // The tool and the display preferences.
    app.select_tool(if tool == "selection" { "directSelection" } else { "selection" });
    assert_matches_plain(&ctx, &mut marks, &app, &view, 1.0, "with the other tool");
    app.select_tool(tool);
    app.session.prefs.anchor_size = 6;
    assert_matches_plain(&ctx, &mut marks, &app, &view, 1.0, "with larger anchors");
    app.session.prefs.handle_style = "hollow".into();
    assert_matches_plain(&ctx, &mut marks, &app, &view, 1.0, "with hollow handles");
    app.session.prefs.show_handles_multiple_anchors = !app.session.prefs.show_handles_multiple_anchors;
    assert_matches_plain(&ctx, &mut marks, &app, &view, 1.0, "with the handles option changed");
    app.session.prefs.anchor_size = 1;
    assert_matches_plain(&ctx, &mut marks, &app, &view, 1.0, "with the smallest anchors, which draw as lines");
}

/// An instance's outline comes from its symbol's art, which is not part of the instance's node.
#[test]
fn an_edit_of_a_symbol_outlines_its_instance_again() {
    let (mut app, _) = scene();
    let ctx = egui::Context::default();
    let mut marks = SelectionMarks::default();
    let view = xf(2.0, VIEW, 0.0);
    let instance = {
        let st = app.session.active().unwrap();
        st.selection.objects.iter().copied().find(|id| matches!(st.doc.node(*id).map(|n| &n.kind), Some(NodeKind::SymbolInstance { .. }))).unwrap()
    };
    app.run("select.set", json!({"ids": [instance.0]})).unwrap();
    assert_matches_plain(&ctx, &mut marks, &app, &view, 1.0, "before the edit");
    let before = app.session.active().unwrap().doc.node(instance).unwrap().geometric_bounds_with(&app.session.active().unwrap().doc.symbol_boxes());
    app.run("symbol.edit", json!({"name": "Mark"})).unwrap();
    app.run("select.all", json!({})).unwrap();
    app.run("object.scale", json!({"sx": 3, "sy": 2})).unwrap();
    app.run("symbol.exitEditing", json!({})).unwrap();
    app.run("select.set", json!({"ids": [instance.0]})).unwrap();
    let after = app.session.active().unwrap().doc.node(instance).unwrap().geometric_bounds_with(&app.session.active().unwrap().doc.symbol_boxes());
    assert_ne!(before, after, "the edit changes what the instance measures");
    assert_matches_plain(&ctx, &mut marks, &app, &view, 1.0, "after the edit");
}

/// A selected linked image and a selected placed document are crossed from corner to corner, on a
/// turned or distorted image along its frame; a selected embedded image shows its outline alone,
/// along its frame when turned or distorted. A render of the document is not crossed (#9080).
#[test]
fn a_selected_linked_image_is_crossed_and_an_embedded_one_is_not() {
    use craft_image_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType};
    let dir = vectorcraft_testkit::temp_dir("selection-marks-linked");
    let png = dir.join("red.png");
    let pixels = [200u8, 40, 40, 255].repeat(40 * 40);
    let image = Image::from_raw(40, 40, ChannelLayout::Rgba, SampleType::U8, pixels).unwrap();
    std::fs::write(&png, craft_image_codecs::encode(&image, Format::Png, &EncodeOptions::default()).unwrap()).unwrap();
    let placed = dir.join("art.vectorcraft");
    std::fs::write(&placed, vectorcraft_format::save_file(&vectorcraft_doc::Document::new(60.0, 40.0))).unwrap();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let place = |app: &mut VectorcraftApp, path: &std::path::Path, link: bool, at: [f64; 2]| {
        NodeId(app.run("file.place", json!({"path": path, "link": link, "at": at})).unwrap()["ids"][0].as_u64().unwrap())
    };
    let linked = place(&mut app, &png, true, [100.0, 120.0]);
    let embedded = place(&mut app, &png, false, [300.0, 120.0]);
    let document = place(&mut app, &placed, true, [200.0, 230.0]);
    let ctx = egui::Context::default();
    let view = xf(1.0, VIEW, 0.0);
    // Whether `picture` draws within a pixel of each of the points a quarter and three quarters of
    // the way along each diagonal of `corners`, clear of the outline.
    let crossed_in = |picture: &Picture, corners: [Point; 4]| -> [bool; 4] {
        let drawn = |q: Point| {
            let s = view.to_screen(q);
            (-1..=1).any(|dy| {
                (-1..=1).any(|dx| {
                    let (x, y) = ((s.x.round() as i64 + dx) as usize, (s.y.round() as i64 + dy) as usize);
                    picture.pixels[y * picture.size.0 + x][3] > 0.0
                })
            })
        };
        let [tl, tr, br, bl] = corners;
        assert!([(tl, tr), (tr, br), (br, bl), (bl, tl)].into_iter().all(|(a, b)| drawn(a.midpoint(b))), "the outline runs along the frame");
        [tl.lerp(br, 0.25), tl.lerp(br, 0.75), tr.lerp(bl, 0.25), tr.lerp(bl, 0.75)].map(drawn)
    };
    // The same for the selection marks.
    let crossed = |app: &VectorcraftApp, corners: [Point; 4]| {
        crossed_in(&picture(&ctx, 1.0, |p| paint(&mut SelectionMarks::default(), app, p, &view, false)), corners)
    };
    let corners = |app: &VectorcraftApp, id: NodeId| {
        let n = app.session.active().unwrap().doc.node(id).unwrap();
        match &n.kind {
            NodeKind::PlacedDocument(p) => p.corners(),
            _ => n.image_corners().unwrap(),
        }
    };
    app.run("select.all", json!({})).unwrap();
    assert_eq!(crossed(&app, corners(&app, linked)), [true; 4], "a linked image");
    assert_eq!(crossed(&app, corners(&app, document)), [true; 4], "a placed document");
    assert_eq!(crossed(&app, corners(&app, embedded)), [false; 4], "an embedded image");
    assert_matches_plain(&ctx, &mut SelectionMarks::default(), &app, &view, 1.0, "linked and embedded images");
    app.run("select.set", json!({"ids": [linked.0, embedded.0]})).unwrap();
    app.run("object.rotate", json!({"angle": 30})).unwrap();
    assert_eq!(crossed(&app, corners(&app, linked)), [true; 4], "a turned linked image, along its frame");
    assert_eq!(crossed(&app, corners(&app, embedded)), [false; 4], "a turned embedded image, outlined along its frame");
    {
        let doc = &app.session.active().unwrap().doc;
        let [tl, tr, br, bl] = corners(&app, linked);
        let bounds = doc.node(linked).unwrap().geometric_bounds().unwrap();
        let out = vectorcraft_testkit::raster::render_region(doc, bounds, 1.0);
        let pixel = |q: Point| out.pixel((q.x - bounds.x0).round() as u32, (q.y - bounds.y0).round() as u32);
        // Halfway between the middle of the left side and the center: clear of both diagonals.
        let clear = pixel(tl.midpoint(bl).midpoint(tl.midpoint(br)));
        assert!(clear[0] > 150 && clear[1] < 100 && clear[3] == 255, "the render shows the image: {clear:?}");
        let on_lines = [tl.lerp(br, 0.25), tl.lerp(br, 0.75), tr.lerp(bl, 0.25), tr.lerp(bl, 0.75)];
        assert!(on_lines.iter().all(|q| pixel(*q) == clear), "output of a selected linked image is not crossed");
    }
    // Move each image's bottom right corner up its right side, so its corners no longer make a
    // parallelogram and one diagonal turns away from where it ran.
    app.session
        .edit("Distort", |doc, _| {
            for id in [linked, embedded] {
                if let Some(n) = doc.node_mut(id)
                    && let Some([tl, tr, br, bl]) = n.image_corners()
                {
                    n.set_image_corners([tl, tr, br.lerp(tr, 0.4), bl]).map_err(|e| vectorcraft_engine::EngineError::Other(format!("{e:?}")))?;
                }
            }
            Ok(())
        })
        .unwrap();
    let doc = &app.session.active().unwrap().doc;
    assert!([linked, embedded].iter().all(|id| doc.node(*id).unwrap().is_distorted_image()), "both images are distorted");
    assert_eq!(crossed(&app, corners(&app, linked)), [true; 4], "a distorted linked image, corner to corner");
    assert_eq!(crossed(&app, corners(&app, embedded)), [false; 4], "a distorted embedded image, outlined along its frame");
    // The key object's outline alone, with no other selection marks.
    for id in [linked, embedded] {
        let st = app.session.active_mut().unwrap();
        st.selection.objects.clear();
        st.selection.key = Some(id);
        let key = picture(&ctx, 1.0, |p| paint(&mut SelectionMarks::default(), &app, p, &view, false));
        assert_eq!(crossed_in(&key, corners(&app, id)), [false; 4], "the key object's outline of a distorted image, {id:?}");
    }
    // The hover highlight outlines a linked image along its frame and does not cross it.
    app.run("select.set", json!({"ids": []})).unwrap();
    for id in [linked, embedded] {
        let [tl, tr, br, bl] = corners(&app, id);
        app.hover_doc = Some(tl.midpoint(tr).midpoint(bl.midpoint(br)));
        let hovered = picture(&ctx, 1.0, |p| super::super::overlays::hover_highlight(&app, p, &view));
        assert_eq!(crossed_in(&hovered, [tl, tr, br, bl]), [false; 4], "the hover highlight of a distorted image, {id:?}");
    }
}

/// A selected group outlines, marks and crosses only the members the canvas draws: a hidden member
/// and the members of a hidden group inside it leave no outline, anchors or crossing lines.
#[test]
fn a_selected_group_leaves_its_hidden_members_unmarked() {
    use craft_image_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType};
    let dir = vectorcraft_testkit::temp_dir("selection-marks-hidden");
    let png = dir.join("red.png");
    let pixels = [200u8, 40, 40, 255].repeat(40 * 40);
    let image = Image::from_raw(40, 40, ChannelLayout::Rgba, SampleType::U8, pixels).unwrap();
    std::fs::write(&png, craft_image_codecs::encode(&image, Format::Png, &EncodeOptions::default()).unwrap()).unwrap();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let left = id_of(&app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 20, "height": 20})).unwrap());
    let right = id_of(&app.run("shape.rectangle", json!({"x": 330, "y": 230, "width": 20, "height": 20})).unwrap());
    let hidden = id_of(&app.run("shape.rectangle", json!({"x": 120, "y": 100, "width": 40, "height": 40})).unwrap());
    let linked = NodeId(app.run("file.place", json!({"path": png, "link": true, "at": [240.0, 150.0]})).unwrap()["ids"][0].as_u64().unwrap());
    app.run("select.set", json!({"ids": [linked.0]})).unwrap();
    let inner = id_of(&app.run("object.group", json!({})).unwrap());
    app.run("select.set", json!({"ids": [left.0, right.0, hidden.0, inner.0]})).unwrap();
    let group = id_of(&app.run("object.group", json!({})).unwrap());
    app.session
        .edit("Hide", |doc, _| {
            for id in [hidden, inner] {
                if let Some(n) = doc.node_mut(id) {
                    n.visible = false;
                }
            }
            Ok(())
        })
        .unwrap();
    app.run("select.set", json!({"ids": [group.0]})).unwrap();
    let ctx = egui::Context::default();
    let view = xf(1.0, VIEW, 0.0);
    let marks = picture(&ctx, 1.0, |p| paint(&mut SelectionMarks::default(), &app, p, &view, false));
    // Whether the marks draw within a pixel of document point `q`.
    let drawn = |q: Point| {
        let s = view.to_screen(q);
        (-1..=1).any(|dy| {
            (-1..=1).any(|dx| {
                let (x, y) = ((s.x.round() as i64 + dx) as usize, (s.y.round() as i64 + dy) as usize);
                marks.pixels[y * marks.size.0 + x][3] > 0.0
            })
        })
    };
    let doc = &app.session.active().unwrap().doc;
    let sides = |id: NodeId| {
        let b = doc.node(id).unwrap().geometric_bounds().unwrap();
        [Point::new(b.x0, b.y0), Point::new(b.x1, b.y0), Point::new(b.x1, b.y1), Point::new(b.x0, b.y1)]
    };
    for id in [left, right] {
        let [tl, tr, br, bl] = sides(id);
        assert!([tl, tr, br, bl, tl.midpoint(tr), br.midpoint(bl)].into_iter().all(drawn), "a visible member is outlined and marked");
    }
    let [tl, tr, br, bl] = doc.node(linked).unwrap().image_corners().unwrap();
    assert!(
        ![tl.lerp(br, 0.25), tl.lerp(br, 0.75), tr.lerp(bl, 0.25), tr.lerp(bl, 0.75)].into_iter().any(drawn),
        "the lines across a linked image in a hidden group"
    );
    assert!(![tl.midpoint(tr), tr.midpoint(br)].into_iter().any(drawn), "the outline of a linked image in a hidden group");
    let [tl, tr, br, bl] = sides(hidden);
    assert!(![tl, tr, br, bl, tl.midpoint(tr), tr.midpoint(br), br.midpoint(bl), bl.midpoint(tl)].into_iter().any(drawn), "a hidden member");
    assert_matches_plain(&ctx, &mut SelectionMarks::default(), &app, &view, 1.0, "a group with hidden members");
    // The hover highlight of the group outlines its visible members only.
    let (left_sides, right_sides, hidden_sides, image) =
        (sides(left), sides(right), sides(hidden), doc.node(linked).unwrap().image_corners().unwrap());
    app.run("select.set", json!({"ids": []})).unwrap();
    app.hover_doc = Some(left_sides[0].midpoint(left_sides[2]));
    let hovered = picture(&ctx, 1.0, |p| super::super::overlays::hover_highlight(&app, p, &view));
    let lit = |q: Point| {
        let s = view.to_screen(q);
        (-1..=1).any(|dy| {
            (-1..=1).any(|dx| {
                let (x, y) = ((s.x.round() as i64 + dx) as usize, (s.y.round() as i64 + dy) as usize);
                hovered.pixels[y * hovered.size.0 + x][3] > 0.0
            })
        })
    };
    for [tl, tr, br, bl] in [left_sides, right_sides] {
        assert!([tl.midpoint(tr), br.midpoint(bl)].into_iter().all(lit), "the hover highlight outlines a visible member");
    }
    let [tl, tr, br, bl] = hidden_sides;
    assert!(![tl.midpoint(tr), tr.midpoint(br), br.midpoint(bl), bl.midpoint(tl)].into_iter().any(lit), "the hover highlight of a hidden member");
    let [tl, tr, br, bl] = image;
    assert!(
        ![tl.midpoint(tr), tr.midpoint(br), tl.lerp(br, 0.25), tr.lerp(bl, 0.25)].into_iter().any(lit),
        "the hover highlight of a linked image in a hidden group"
    );
}

/// A frame of the whole canvas keeps entries for the selected objects only.
#[test]
fn the_canvas_drops_entries_it_does_not_draw() {
    let (mut app, curve) = scene();
    let ctx = egui::Context::default();
    let frame = |app: &mut VectorcraftApp| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
    };
    frame(&mut app);
    frame(&mut app);
    let all = app.session.active().unwrap().selection.objects.len();
    assert_eq!(app.canvas.selection_marks.objects.len(), all);
    app.run("select.set", json!({"ids": [curve.0]})).unwrap();
    frame(&mut app);
    assert_eq!(app.canvas.selection_marks.objects.len(), 1, "objects that left the selection");
    app.run("edit.clear", json!({"ids": [curve.0]})).unwrap();
    frame(&mut app);
    assert_eq!(app.canvas.selection_marks.objects.len(), 0, "an object that left the document");
    app.run("select.all", json!({})).unwrap();
    frame(&mut app);
    frame(&mut app);
    assert!(!app.canvas.selection_marks.objects.is_empty());
    app.run("view.edges", json!({})).unwrap();
    frame(&mut app);
    assert_eq!(app.canvas.selection_marks.objects.len(), 0, "hidden edges keep nothing");
}

#[test]
fn past_the_limit_the_marks_are_dropped_and_built_again() {
    let (app, _) = scene();
    let ctx = egui::Context::default();
    let mut marks = SelectionMarks::default();
    let view = xf(1.0, VIEW, 0.0);
    assert_matches_plain(&ctx, &mut marks, &app, &view, 1.0, "at the limit");
    assert!(marks.kept > 0);
    marks.trim(marks.kept);
    assert!(!marks.objects.is_empty(), "at the limit the marks stay");
    marks.trim(marks.kept - 1);
    assert!(marks.objects.is_empty() && marks.frame.is_empty(), "past the limit nothing stays");
    assert_matches_plain(&ctx, &mut marks, &app, &view, 1.0, "after dropping them");
}
