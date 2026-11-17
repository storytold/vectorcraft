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
