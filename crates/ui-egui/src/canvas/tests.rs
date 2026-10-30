use super::*;
use vectorcraft_engine::Session;
use vectorcraft_geom::Shape as _;

#[test]
fn subpixel_dock_layout_does_not_change_pointer_document_coordinates() {
    let mut observed = Vec::new();
    for width in [1324.0 + 29.0 / 32.0, 1324.875, 1325.875] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 800, "height": 600, "created": 0})).unwrap();
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(2.0);
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1800.0, 900.0))),
            max_texture_side: Some(8192),
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| {
            let area = egui::Rect::from_min_size(pos2(54.0, 72.0), vec2(width + SCROLL_BAR, 636.0 + SCROLL_BAR));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area));
            show(&mut app, &mut child);
        });
        out.textures_delta.clear();
        let rect = app.canvas_rect.unwrap();
        observed.push((rect, Xf::new(rect, app.view().unwrap()).to_doc(pos2(440.0, 330.0))));
    }
    assert_eq!(observed[0], observed[1], "the same physical canvas must map the same pointer identically");
    assert_ne!(observed[0], observed[2], "a real device-pixel resize must still change the mapping");
}

/// A rendered raster becomes the image egui's own conversion makes of it, half-transparent
/// edges and all; one whose bytes don't match its size becomes an empty image of that size.
#[test]
fn rendered_rasters_become_the_same_egui_images() {
    let mut s = Session::new();
    s.execute("file.new", &serde_json::json!({})).unwrap();
    s.execute("shape.ellipse", &serde_json::json!({"x": 10, "y": 10, "width": 60, "height": 40})).unwrap();
    s.execute("paint.setFill", &serde_json::json!({"color": "#3366cc"})).unwrap();
    let doc = s.active().unwrap().doc.clone();
    let img = vectorcraft_render::Renderer::new().render(&doc, 97, 61, Affine::IDENTITY, &vectorcraft_render::RenderOptions::default());
    assert!(img.pixels.as_chunks::<4>().0.iter().any(|p| p[3] != 0 && p[3] != 255), "partly covered pixels");
    let egui = egui::ColorImage::from_rgba_premultiplied([97, 61], &img.pixels);
    assert_eq!(color_image(&img), egui);
    let short = vectorcraft_render::Rendered { width: 3, height: 2, pixels: vec![255; 5] };
    assert_eq!(color_image(&short), egui::ColorImage::new([3, 2], vec![egui::Color32::TRANSPARENT; 6]));
}

/// With VECTORCRAFT_SYSTEM_CURSORS set the glyphs aren't drawn and the system's cursors stand in: those of
/// the Pen and the other drawing tools are the system crosshair already, so Use Precise Cursors makes no
/// difference to them there.
#[test]
fn system_cursors_show_the_same_crosshair_with_precise_cursors_on_or_off() {
    for c in [
        Cursor::Pen,
        Cursor::PenDrawing,
        Cursor::PenAdd,
        Cursor::PenDelete,
        Cursor::PenClose,
        Cursor::PenContinue,
        Cursor::PenJoin,
        Cursor::PenConvert,
        Cursor::Eyedropper,
        Cursor::Slice,
        Cursor::Blend,
        Cursor::BlendObject,
        Cursor::BlendAnchor,
        Cursor::ShapeBuilder,
        Cursor::ShapeBuilderErase,
        Cursor::PencilNew,
        Cursor::PencilStraight,
        Cursor::PencilContinue,
    ] {
        assert_eq!(cursor_icon(c), egui::CursorIcon::Crosshair, "{c:?}");
        assert_eq!(cursor_icon(c), cursor_icon(c.precise()), "{c:?}");
    }
}

/// #826: in a turned view each ruler still has ticks along its whole length; at 0° the top one
/// reads x where the canvas has it.
#[test]
fn rulers_cover_their_length_at_any_rotation() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let full = egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    let [top, left, _] = ruler_rects(full);
    for deg in [0.0_f64, 30.0, 90.0, 135.0, 200.0] {
        let xf = Xf { rect: full, zoom: 1.0, center: Point::new(300.0, 200.0), rot: deg.to_radians() };
        let scale = RulerScale::document(Unit::Points, Point::ZERO);
        let mut out = ctx.run_ui(Default::default(), |ui| rulers(ui, full, &xf, None, scale, &Tokens::get(ui.ctx())));
        out.textures_delta.clear();
        // The ticks' positions along each ruler.
        let along = |ruler: egui::Rect, x: bool| -> Vec<f32> {
            let mut at: Vec<f32> = out
                .shapes
                .iter()
                .filter(|c| c.clip_rect == ruler)
                .filter_map(|c| match &c.shape {
                    Shape::LineSegment { points: [a, b], .. } if x && a.x == b.x => Some(a.x),
                    Shape::LineSegment { points: [a, b], .. } if !x && a.y == b.y => Some(a.y),
                    _ => None,
                })
                .collect();
            at.sort_by(f32::total_cmp);
            at
        };
        for (ruler, x) in [(top, true), (left, false)] {
            let at = along(ruler, x);
            let (start, end) = if x { (ruler.left(), ruler.right()) } else { (ruler.top(), ruler.bottom()) };
            let (Some(first), Some(last)) = (at.first(), at.last()) else { panic!("{deg}°: no ticks") };
            assert!(first - start < 20.0 && end - last < 20.0, "{deg}°: ticks from {first} to {last} on {start}..{end}");
            assert!(at.windows(2).all(|w| w[1] - w[0] < 20.0), "{deg}°: a gap in the ticks");
        }
        if deg == 0.0 {
            let origin = xf.to_screen(Point::ZERO).x;
            assert!(along(top, true).iter().any(|x| (x - origin).abs() < 0.01), "a tick at x = 0");
        }
    }
}

#[test]
fn ruler_origin_zero_ticks_follow_the_origin_at_any_rotation_and_unit() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let full = egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
    let [top, left, _] = ruler_rects(full);
    let origin = Point::new(350.25, 250.5);
    for deg in [0.0_f64, 30.0, 90.0, 200.0] {
        for unit in [Unit::Points, Unit::Millimeters] {
            let xf = Xf { rect: full, zoom: 1.7, center: Point::new(300.0, 200.0), rot: deg.to_radians() };
            let scale = RulerScale::document(unit, origin);
            let mut out = ctx.run_ui(Default::default(), |ui| rulers(ui, full, &xf, None, scale, &Tokens::get(ui.ctx())));
            out.textures_delta.clear();
            let at = xf.to_screen(origin);
            for (r, horizontal) in [(top, true), (left, false)] {
                let zeros: Vec<_> = out
                    .shapes
                    .iter()
                    .filter(|c| c.clip_rect == r)
                    .filter_map(|c| match &c.shape {
                        Shape::Text(t) if t.galley.text() == "0" => Some(t.pos),
                        _ => None,
                    })
                    .collect();
                // Left-ruler labels are separate digits: a zero in "100" isn't the zero label.
                assert!(
                    zeros.iter().any(|zero| {
                        let delta = if horizontal { zero.x - 2.0 - at.x } else { zero.y - 2.0 - at.y };
                        delta.abs() < 0.02
                    }),
                    "{deg}°, {unit:?}: zeros at {zeros:?}, origin {at:?}"
                );
            }
        }
    }
}

/// View › Rulers › Show Video Rulers counts the artboard in video pixels from its top-left
/// corner, a pixel as wide as the artboard's Video Ruler Pixel Aspect Ratio. At 0.9 a
/// 720 × 480 pt artboard is 800 pixels wide and 480 tall: the top ruler reads 0 at its left
/// edge and 800 at its right, labelled every 100 pixels (90 points on screen at 100%), and the
/// left ruler counts a pixel a point, from 0 at the top edge to a tick at the bottom one. At 1
/// (square pixels) the artboard is 720 pixels wide.
#[test]
fn video_rulers_count_the_artboard_in_pixels_of_its_aspect_ratio() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 720, "height": 480})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 0, "videoRulerPixelAspectRatio": 0.9})).unwrap();
    let ab = s.active().unwrap().doc.artboards[0].clone();
    let r = ab.rect;
    let scale = RulerScale::video(&ab);
    let (x, y) = (vectorcraft_geom::Vec2::new(1.0, 0.0), vectorcraft_geom::Vec2::new(0.0, 1.0));
    let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
    assert_eq!((scale.unit, scale.origin), (Unit::Pixels, Point::new(r.x0, r.y0)));
    assert!(close(scale.along(Point::new(r.x1, r.y1), x), 800.0) && close(scale.along(Point::new(r.x1, r.y1), y), 480.0));
    assert!(close(scale.along(Point::new(r.x0 + 9.0, r.y0), x), 10.0), "a pixel is 0.9 pt wide");
    assert!(close(scale.zoom_along(2.0, x), 1.8) && close(scale.zoom_along(2.0, y), 2.0));
    let square = RulerScale { stretch: [1.0, 1.0], ..scale };
    assert!(close(square.along(Point::new(r.x1, r.y0), x), 720.0));
    // Drawn at 100%.
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let full = egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
    let [top, left, _] = ruler_rects(full);
    let xf = Xf { rect: full, zoom: 1.0, center: r.center(), rot: 0.0 };
    let mut out = ctx.run_ui(Default::default(), |ui| rulers(ui, full, &xf, None, scale, &Tokens::get(ui.ctx())));
    out.textures_delta.clear();
    let labels = |ruler: egui::Rect| -> Vec<(String, Pos2)> {
        out.shapes
            .iter()
            .filter(|c| c.clip_rect == ruler)
            .filter_map(|c| match &c.shape {
                Shape::Text(t) => Some((t.galley.text().to_string(), t.pos)),
                _ => None,
            })
            .collect()
    };
    let (l, rgt) = (xf.to_screen(Point::new(r.x0, r.y0)), xf.to_screen(Point::new(r.x1, r.y1)));
    let at = |labels: &[(String, Pos2)], text: &str, x: f32| labels.iter().any(|(t, p)| t == text && (p.x - 2.0 - x).abs() < 0.02);
    let tops = labels(top);
    assert!(at(&tops, "0", l.x) && at(&tops, "800", rgt.x), "{tops:?} from {l:?} to {rgt:?}");
    assert!(at(&tops, "100", l.x + 90.0) && !tops.iter().any(|(t, _)| t == "50"), "{tops:?}");
    assert!(!tops.iter().any(|(t, _)| t == "720"), "points aren't pixels: {tops:?}");
    let lefts = labels(left);
    assert!(lefts.iter().any(|(t, p)| t == "0" && (p.y - 2.0 - l.y).abs() < 0.02), "{lefts:?}");
    let ticks: Vec<f32> = out
        .shapes
        .iter()
        .filter(|c| c.clip_rect == left)
        .filter_map(|c| match &c.shape {
            Shape::LineSegment { points: [a, b], .. } if a.y == b.y => Some(a.y),
            _ => None,
        })
        .collect();
    assert!(ticks.iter().any(|y| (y - rgt.y).abs() < 0.02), "a tick at 480 px: {ticks:?}");
}

#[test]
fn shaper_selection_highlights_only_the_visible_result() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("shape.ellipse", &json!({"x":100,"y":100,"width":100,"height":100})).unwrap();
    let line = vectorcraft_doc::NodeId(s.execute("shape.line", &json!({"x1":50,"y1":150,"x2":250,"y2":150})).unwrap()["id"].as_u64().unwrap());
    for x in [0.0, 150.0] {
        s.execute("shaper.scribble", &json!({"points":[[60.0+x,140],[70.0+x,160],[80.0+x,140],[90.0+x,160]]})).unwrap();
    }
    let st = s.active().unwrap();
    let g = st.doc.node(st.selection.objects[0]).unwrap();
    assert!(g.extras().shaper.is_some());
    let symbols = st.doc.symbol_boxes();
    let bounds = node_outline(g, &symbols).bounding_box();
    assert!((bounds.x0 - 100.0).abs() < 0.01 && (bounds.x1 - 200.0).abs() < 0.01, "{bounds:?}");
    assert_eq!(node_outline(st.doc.node(line).unwrap(), &symbols).bounding_box().x0, 50.0, "an isolated original still shows its full outline");
}

/// A clip group made from scratch: a 320 × 40 image and a 300 × 200 ellipse, both reaching
/// outside a 100 × 100 square at (100, 100) that clips them, and a rectangle beside them. The
/// task bar and rulers are off. → the app, the clip group, the ellipse and the rectangle.
fn clip_group_app() -> (VectorcraftApp, serde_json::Value, serde_json::Value, serde_json::Value) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    let img = image::RgbaImage::from_pixel(320, 40, image::Rgba([200, 120, 40, 255]));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let data = vectorcraft_format::base64_encode(&png);
    app.run("file.place", json!({"name": "texture.png", "dataBase64": data, "link": false, "at": [150, 150]})).unwrap();
    let image = json!(app.session.active().unwrap().selection.objects[0].0);
    let ellipse = app.run("shape.ellipse", json!({"x": 0, "y": 50, "width": 300, "height": 200})).unwrap()["id"].clone();
    let square = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [image, ellipse, square]})).unwrap();
    let group = app.run("object.clippingMask.make", json!({})).unwrap()["id"].clone();
    let beside = app.run("shape.rectangle", json!({"x": 330, "y": 20, "width": 50, "height": 50})).unwrap()["id"].clone();
    (app, group, ellipse, beside)
}

/// The screen points of what one frame draws in `color` with lines `width` px wide: outlines and
/// the bounding box, and with `width` 1 the anchors and the bounding box's handles too.
fn drawn_in(app: &mut VectorcraftApp, ctx: &egui::Context, color: Color32, width: f32) -> Vec<Pos2> {
    let solid = egui::epaint::ColorMode::Solid(color);
    let mut at = vec![];
    for s in shapes(app, ctx) {
        match s {
            Shape::Path(ps) if ps.stroke.color == solid && ps.stroke.width == width => at.extend(ps.points),
            Shape::Rect(r) if width == 1.0 && (r.fill == color || (r.stroke.color == color && r.stroke.width == width)) => {
                at.extend([r.rect.min, r.rect.max]);
            }
            _ => {}
        }
    }
    at
}

/// #2422: a selected clip group shows its clipping path's outline and anchors, the bounding
/// box of what shows, and not the art the path clips; so do its hover highlight and its key
/// object outline.
#[test]
fn a_selected_clip_group_shows_its_clipping_path_not_its_hidden_art() {
    let (mut app, group, _, beside) = clip_group_app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let color = c32(app.session.active().unwrap().doc.layer_color(vectorcraft_doc::NodeId(group.as_u64().unwrap())));
    // The square on screen. `inside` allows 8 px around it for the anchors and the bounding box's handles.
    let clip = egui::Rect::from_two_pos(xf.to_screen(Point::new(100.0, 100.0)), xf.to_screen(Point::new(200.0, 200.0)));
    // Each case that draws nothing, or draws outside the square.
    let mut wrong = vec![];
    let mut inside = |at: Vec<Pos2>, what: &str| {
        if at.is_empty() || !at.iter().all(|q| clip.expand(8.0).contains(*q)) {
            wrong.push(format!("{what}: {at:?}"));
        }
    };
    for tool in ["selection", "directSelection", "groupSelection"] {
        app.select_tool(tool);
        app.run("select.set", json!({"ids": [group]})).unwrap();
        inside(drawn_in(&mut app, &ctx, color, 1.0), tool);
    }
    app.select_tool("selection");
    app.run("select.set", json!({"ids": [group, beside]})).unwrap();
    app.run("select.key", json!({"id": group})).unwrap();
    inside(drawn_in(&mut app, &ctx, color, KEY_OUTLINE), "the key object");
    app.run("select.none", json!({})).unwrap();
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(xf.to_screen(Point::new(150.0, 150.0)))]);
    inside(drawn_in(&mut app, &ctx, color, 1.5), "the hover highlight");
    assert!(wrong.is_empty(), "drawn outside {clip:?}:\n{}", wrong.join("\n"));
}

/// A group of four clip groups, each a placed image larger than the rounded square that clips
/// it: one click with the Selection tool selects the group, which shows the four squares'
/// outlines and no image's rectangle, and a bounding box that is the union of the squares.
#[test]
fn a_group_of_clipped_images_shows_its_masks_and_their_bounds() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 600, "height": 300})).unwrap();
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    let img = image::RgbaImage::from_pixel(160, 160, image::Rgba([40, 120, 200, 255]));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let data = vectorcraft_format::base64_encode(&png);
    // Masks 100 × 100 at x = 20, 150, 280, 410; each image reaches 30 past its mask on every side.
    let masks: Vec<Rect> = (0..4).map(|i| Rect::new(20.0 + 130.0 * i as f64, 100.0, 120.0 + 130.0 * i as f64, 200.0)).collect();
    let mut clips = vec![];
    for m in &masks {
        app.run("file.place", json!({"name": "photo.png", "dataBase64": data, "link": false, "at": [m.center().x, m.center().y]})).unwrap();
        let image = json!(app.session.active().unwrap().selection.objects[0].0);
        let mask = app.run("shape.rectangle", json!({"x": m.x0, "y": m.y0, "width": 100, "height": 100, "radius": 16})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [image, mask]})).unwrap();
        clips.push(app.run("object.clippingMask.make", json!({})).unwrap()["id"].clone());
    }
    app.run("select.set", json!({"ids": clips})).unwrap();
    let group = vectorcraft_doc::NodeId(app.run("object.group", json!({})).unwrap()["id"].as_u64().unwrap());
    app.run("select.none", json!({})).unwrap();
    let union = masks.iter().copied().reduce(|a, b| a.union(b)).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    // One click on a visible part of the second image selects the whole group.
    app.select_tool("selection");
    let view = app.view_info();
    for kind in [PointerKind::Down, PointerKind::Up] {
        app.session.pointer(&PointerEvent::new(kind, 200.0, 150.0), view).unwrap();
    }
    assert_eq!(app.session.active().unwrap().selection.objects, vec![group]);
    // The bounding box, the bounds and the transform box fit the masks, not the images.
    let close =
        |r: Option<Rect>| r.is_some_and(|r| [r.x0 - union.x0, r.y0 - union.y0, r.x1 - union.x1, r.y1 - union.y1].iter().all(|d| d.abs() < 1e-9));
    let st = app.session.active().unwrap();
    assert!(close(st.doc.node(group).unwrap().geometric_bounds()), "the group's bounds");
    assert!(close(app.selection_box().map(|b| b.rect)), "the bounding box");
    // Everything drawn in the layer's colour lies on a mask: on its outline, its anchors or the
    // bounding box round them (8 px for the anchors and the box's handles).
    let color = c32(app.session.active().unwrap().doc.layer_color(group));
    let screen = |r: Rect| egui::Rect::from_two_pos(xf.to_screen(Point::new(r.x0, r.y0)), xf.to_screen(Point::new(r.x1, r.y1)));
    let drawn = drawn_in(&mut app, &ctx, color, 1.0);
    assert!(!drawn.is_empty());
    let near_a_mask = |q: &Pos2| masks.iter().any(|m| screen(*m).expand(8.0).contains(*q));
    let on_the_box = |q: &Pos2| {
        let b = screen(union);
        b.expand(8.0).contains(*q) && !b.shrink(8.0).contains(*q)
    };
    let stray: Vec<_> = drawn.iter().filter(|q| !near_a_mask(q) && !on_the_box(q)).collect();
    assert!(stray.is_empty(), "drawn off the masks: {stray:?}");
    // Each mask's outline is drawn: points on its left and right edges.
    for m in &masks {
        let r = screen(*m);
        let on = |x: f32| drawn.iter().any(|q| (q.x - x).abs() < 1.0 && q.y > r.min.y + 16.0 && q.y < r.max.y - 16.0);
        assert!(on(r.min.x) && on(r.max.x), "the outline of {m:?}");
    }
    // No image's rectangle: nothing along the images' top edge, 30 points above the masks.
    let top = xf.to_screen(Point::new(0.0, union.y0 - 30.0)).y;
    assert!(!drawn.iter().any(|q| (q.y - top).abs() < 0.5), "the images' top edge at y {top}");
}

/// #2422: a clip group whose hidden art has more anchors than the overlay draws (#525) still
/// shows its clipping path's anchors and hover highlight.
#[test]
fn a_clip_group_over_a_huge_path_shows_its_clipping_path() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    let n = OVERLAY_MAX_ANCHORS + 10;
    let points = (0..n).map(|i| format!("L{} {}", 20.0 + 360.0 * i as f64 / n as f64, 100 + (i % 2) * 50)).collect::<String>();
    let huge = app.run("path.create", json!({"d": format!("M20 20 {points} Z")})).unwrap()["id"].clone();
    let square = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [huge, square]})).unwrap();
    let group = app.run("object.clippingMask.make", json!({})).unwrap()["id"].clone();
    let color = c32(app.session.active().unwrap().doc.layer_color(vectorcraft_doc::NodeId(group.as_u64().unwrap())));
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    app.select_tool("selection");
    app.run("select.set", json!({"ids": [group]})).unwrap();
    let anchors = shapes(&mut app, &ctx).iter().filter(|s| matches!(s, Shape::Rect(r) if r.fill == color)).count();
    app.run("select.none", json!({})).unwrap();
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(xf.to_screen(Point::new(150.0, 150.0)))]);
    let hovered = !drawn_in(&mut app, &ctx, color, 1.5).is_empty();
    assert_eq!((anchors, hovered), (4, true), "the square's four anchors, and the hover highlight");
}

/// #2422: the art inside a clip group still shows its whole outline when it is picked on its
/// own: by Edit Contents, by a Direct Selection or Group Selection click, and in isolation mode;
/// so does its Group Selection hover highlight.
#[test]
fn the_art_a_clip_group_clips_shows_whole_when_picked() {
    let (mut app, group, ellipse, _) = clip_group_app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let color = c32(app.session.active().unwrap().doc.layer_color(vectorcraft_doc::NodeId(group.as_u64().unwrap())));
    // Something is drawn at the ellipse's left and right ends, outside the square.
    let reaches = |at: &[Pos2], what: &str| {
        let (left, right) = (xf.to_screen(Point::new(0.0, 150.0)).x, xf.to_screen(Point::new(300.0, 150.0)).x);
        assert!(at.iter().any(|q| (q.x - left).abs() < 2.0) && at.iter().any(|q| (q.x - right).abs() < 2.0), "{what}: {at:?}");
    };
    app.select_tool("selection");
    app.run("select.set", json!({"ids": [group]})).unwrap();
    app.run("object.clippingMask.editContents", json!({})).unwrap();
    reaches(&drawn_in(&mut app, &ctx, color, 1.0), "Edit Contents");
    app.run("select.none", json!({})).unwrap();
    app.select_tool("directSelection");
    let view = app.view_info();
    for kind in [PointerKind::Down, PointerKind::Up] {
        app.session.pointer(&PointerEvent::new(kind, 150.0, 120.0), view).unwrap();
    }
    assert_eq!(app.session.active().unwrap().selection.objects, vec![vectorcraft_doc::NodeId(ellipse.as_u64().unwrap())]);
    reaches(&drawn_in(&mut app, &ctx, color, 1.0), "a Direct Selection click");
    app.run("select.none", json!({})).unwrap();
    app.select_tool("groupSelection");
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(xf.to_screen(Point::new(150.0, 120.0)))]);
    reaches(&drawn_in(&mut app, &ctx, color, 1.5), "the Group Selection hover highlight");
    for kind in [PointerKind::Down, PointerKind::Up] {
        app.session.pointer(&PointerEvent::new(kind, 150.0, 120.0), view).unwrap();
    }
    assert_eq!(app.session.active().unwrap().selection.objects, vec![vectorcraft_doc::NodeId(ellipse.as_u64().unwrap())]);
    reaches(&drawn_in(&mut app, &ctx, color, 1.0), "a Group Selection click");
    app.select_tool("selection");
    app.run("object.isolate", json!({"id": group})).unwrap();
    app.run("select.all", json!({})).unwrap();
    reaches(&drawn_in(&mut app, &ctx, color, 1.0), "isolation mode");
}

/// A Shaper Group's mode widget, drawn for the Shaper and Selection tools with selection edges
/// shown or hidden: its arrow points right in Face Selection mode and left in Construction mode.
#[test]
fn the_shaper_widget_points_to_the_other_mode() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    for x in [0, 50] {
        app.session.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 100, "height": 100})).unwrap();
    }
    let points = json!({"points": [[25, 20], [75, 35], [30, 50], [125, 65]]});
    let g = app.session.execute("shaper.scribble", &points).unwrap()["id"].as_u64().unwrap();
    let ctx = egui::Context::default();
    // For each widget drawn, whether its arrow's tip is right of the circle's center.
    let right = |app: &mut VectorcraftApp| -> Vec<bool> {
        let drawn = shapes(app, &ctx);
        let r = vectorcraft_tools::shaperwidget::RADIUS_PX;
        let centers: Vec<egui::Pos2> = drawn
            .iter()
            .filter_map(|s| if let Shape::Circle(c) = s { (c.radius == r && c.fill == Color32::WHITE).then_some(c.center) } else { None })
            .collect();
        drawn
            .iter()
            .filter_map(|s| match s {
                Shape::Path(p) if p.points.len() == 3 => centers.iter().find(|c| c.distance(p.points[0]) <= r).map(|c| p.points[0].x > c.x),
                _ => None,
            })
            .collect()
    };
    app.select_tool("shaper");
    assert_eq!(right(&mut app), vec![true], "the scribble selects the group");
    app.session.execute("object.isolate", &json!({"id": g})).unwrap();
    assert_eq!(right(&mut app), vec![false]);
    app.select_tool("selection");
    assert_eq!(right(&mut app), vec![false]);
    // View › Hide Edges keeps the widget, which still answers clicks.
    app.ui.view.edges = false;
    assert_eq!(right(&mut app), vec![false]);
    app.select_tool("pen");
    assert!(right(&mut app).is_empty());
}

#[test]
fn a_symbol_instance_is_outlined_around_its_art() {
    // A symbol holding the square of the SVG import tests' `Star` symbol, 20 × 20 from (0, 0),
    // and an instance of it at (20, 30).
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="120"><defs><symbol id="Star"><path d="M0 0H20V20H0Z" fill="#e0a000"/></symbol></defs><use href="#Star" x="20" y="30"/></svg>"##;
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": "star.svg", "dataBase64": vectorcraft_format::base64_encode(svg.as_bytes())})).unwrap();
    let st = s.active().unwrap();
    let mut inst = None;
    st.doc.walk(|n| {
        if matches!(n.kind, NodeKind::SymbolInstance { .. }) {
            inst = Some(n);
        }
    });
    assert_eq!(node_outline(inst.unwrap(), &st.doc.symbol_boxes()).bounding_box(), Rect::new(20.0, 30.0, 40.0, 50.0));
}

/// One headless canvas frame on an 800 × 600 window.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| show(app, ui));
    out.textures_delta.clear();
}

/// The Image Trace panel's eye-button hold ends when the pointer is released, even when the
/// panel stopped drawing while the button was held, so a later press on the canvas does not
/// show the source image.
#[test]
fn a_release_ends_the_image_trace_hold_without_the_panel() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let made = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    let id = vectorcraft_doc::NodeId(made["id"].as_u64().unwrap());
    let ctx = egui::Context::default();
    let at = pos2(400.0, 300.0);
    let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), press(true)]);
    app.ui.trace_hold_source = Some(std::sync::Arc::new(vec![id]));
    frame(&mut app, &ctx, vec![]);
    assert!(app.ui.trace_hold_source.is_some(), "held while the button is down");
    frame(&mut app, &ctx, vec![press(false)]);
    assert_eq!(app.ui.trace_hold_source, None, "the release ends the hold");
    frame(&mut app, &ctx, vec![press(true)]);
    assert_eq!(app.ui.trace_hold_source, None, "a later press does not show the source image");
    frame(&mut app, &ctx, vec![press(false)]);
}

/// The baseline marker of selected point type runs under the line, wherever its alignment
/// puts the anchor: under centred and right-aligned type too, not from the anchor rightwards
/// (#2879).
#[test]
fn point_type_baselines_run_under_the_line_whatever_its_alignment() {
    for justify in ["left", "center", "right"] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 600, "height": 300})).unwrap();
        app.ui.task_bar = false;
        app.ui.view.rulers = false;
        let made = app.run("text.create", json!({"x": 300, "y": 150, "text": "Baseline marker", "size": 24, "justify": justify})).unwrap();
        let id = vectorcraft_doc::NodeId(made["id"].as_u64().unwrap());
        app.run("select.set", json!({"ids": [id.0]})).unwrap();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let st = app.session.active().unwrap();
        let b = st.doc.node(id).unwrap().geometric_bounds().unwrap();
        let color = c32(st.doc.layer_color(id));
        let y = xf.to_screen(Point::new(0.0, 150.0)).y;
        let (left, right) = (xf.to_screen(Point::new(b.x0, 150.0)).x, xf.to_screen(Point::new(b.x1, 150.0)).x);
        let on: Vec<f32> = shapes(&mut app, &ctx)
            .into_iter()
            .filter_map(|s| match s {
                Shape::LineSegment { points, stroke } if stroke.color == color => Some(points),
                _ => None,
            })
            .flatten()
            .filter(|q| (q.y - y).abs() < 0.5)
            .map(|q| q.x)
            .collect();
        let (lo, hi) = on.iter().fold((f32::MAX, f32::MIN), |(lo, hi), x| (lo.min(*x), hi.max(*x)));
        assert!((lo - left).abs() < 3.0 && (hi - right).abs() < 3.0, "{justify}: the baseline runs {lo}..{hi}, the type {left}..{right}");
    }
}

/// A click with the Selection tool inside an artboard makes it the active one, and Paste in
/// Place then pastes onto it, where the objects were on their own artboard (#693).
#[test]
fn a_click_activates_its_artboard_and_paste_in_place_goes_onto_it() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    let boards: Vec<_> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect();
    app.session.execute("shape.rectangle", &json!({"x": boards[0].x0 + 10.0, "y": boards[0].y0 + 15.0, "width": 20, "height": 20})).unwrap();
    app.run("edit.copy", json!({})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    app.select_tool("selection");
    let view = app.view_info();
    let at = boards[2].center();
    for kind in [PointerKind::Down, PointerKind::Up] {
        dispatch(&mut app, &PointerEvent { kind, pos: at, mods: Default::default(), pressure: 1.0 }, view);
    }
    assert_eq!(app.view().unwrap().artboard, 2, "the clicked artboard is the active one");
    let ids: Vec<vectorcraft_doc::NodeId> = app.run("edit.pasteInPlace", json!({})).unwrap()["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| vectorcraft_doc::NodeId(v.as_u64().unwrap()))
        .collect();
    let b = app.session.active().unwrap().doc.bounds_of(&ids, false).unwrap();
    assert_eq!((b.x0, b.y0), (boards[2].x0 + 10.0, boards[2].y0 + 15.0));
}

/// A Shift-click with the Selection tool on an empty spot inside an artboard makes it the active
/// one and keeps the selection, and Align to Artboard then aligns the objects to it unless the
/// call names another artboard.
#[test]
fn a_shift_click_activates_its_artboard_and_align_to_artboard_aligns_to_it() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    let boards: Vec<_> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect();
    let mut rect = |x: f64, y: f64| {
        let r = app.session.execute("shape.rectangle", &json!({"x": boards[0].x0 + x, "y": boards[0].y0 + y, "width": 20, "height": 20}));
        vectorcraft_doc::NodeId(r.unwrap()["id"].as_u64().unwrap())
    };
    let ids = [rect(10.0, 15.0), rect(50.0, 40.0)];
    app.session.execute("select.set", &json!({"ids": [ids[0].0, ids[1].0]})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    app.select_tool("selection");
    let view = app.view_info();
    let shift = Mods { shift: true, ..Default::default() };
    for kind in [PointerKind::Down, PointerKind::Up] {
        dispatch(&mut app, &PointerEvent { kind, pos: boards[2].center(), mods: shift, pressure: 1.0 }, view);
    }
    assert_eq!(app.view().unwrap().artboard, 2, "the clicked artboard is the active one");
    assert_eq!(app.session.active().unwrap().selection.objects, ids, "the selection stays");
    let lefts = |app: &VectorcraftApp| ids.map(|id| app.session.active().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().x0);
    app.run("object.align", json!({"horizontal": "left", "to": "artboard"})).unwrap();
    assert_eq!(lefts(&app), [boards[2].x0; 2], "the left edges are on the active artboard's");
    app.run("object.align", json!({"horizontal": "left", "to": "artboard", "artboard": 1})).unwrap();
    assert_eq!(lefts(&app), [boards[1].x0; 2], "an artboard the caller names is kept");
}

/// The canvas outlines the active artboard in black, 1 px wide, and outlines the other
/// artboards in gray, 0.6 px wide. In the test, the active artboard is Artboard 1 at first,
/// then the one Go to Artboard went to, and then, with several artboards selected by the
/// Artboard tool, the tool's active one (the one Shift-clicked last).
#[test]
fn the_active_artboard_is_outlined_in_black() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    let ctx = egui::Context::default();
    // → each artboard's edge as drawn: (width, color).
    let edges = |app: &mut VectorcraftApp| {
        let s = shapes(app, &ctx);
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let on = |p: &[Pos2], q: &[Pos2]| p.len() == q.len() && p.iter().zip(q).all(|(a, b)| (*a - *b).length() < 0.01);
        let boards: Vec<Rect> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect();
        boards
            .iter()
            .map(|r| {
                let q = xf.quad(*r);
                s.iter()
                    .find_map(|s| match s {
                        Shape::Path(p) if p.closed && p.fill == Color32::TRANSPARENT && on(&p.points, &q) => {
                            Some((p.stroke.width, p.stroke.color.clone()))
                        }
                        _ => None,
                    })
                    .expect("the artboard's edge")
            })
            .collect::<Vec<_>>()
    };
    let (black, gray) = (egui::epaint::ColorMode::Solid(Color32::from_gray(0)), egui::epaint::ColorMode::Solid(Color32::from_gray(120)));
    let active = |i: usize| -> Vec<(f32, egui::epaint::ColorMode)> {
        (0..3).map(|j| if j == i { (1.0, black.clone()) } else { (0.6, gray.clone()) }).collect()
    };
    assert_eq!(edges(&mut app), active(0));
    app.run("view.goToArtboard", json!({"index": 1})).unwrap();
    assert_eq!(edges(&mut app), active(1), "Artboard 2 is active: its edge is black and Artboard 1's edge is gray");
    // The Artboard tool starts on Artboard 2; a Shift-click on Artboard 3 selects it too and
    // makes it the active one.
    app.select_tool("artboard");
    let at = app.session.active().unwrap().doc.artboards[2].rect.center();
    let view = app.view_info();
    let shift = Mods { shift: true, ..Default::default() };
    for kind in [PointerKind::Down, PointerKind::Up] {
        dispatch(&mut app, &PointerEvent { kind, pos: at, mods: shift, pressure: 1.0 }, view);
    }
    assert_eq!(app.session.tool_options()["selected"], json!([2, 1]));
    assert_eq!(edges(&mut app), active(2), "Artboard 3 is the active one of the two selected");
}

// Regression test for #824: with the Artboard tool, the active artboard's name was drawn twice,
// once by the canvas as for every artboard and once, a little higher and in blue, by the tool.
// Now each artboard is named once, and the active one's name sits where the others' do.
#[test]
fn the_artboard_tool_names_its_active_artboard_once() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 2})).unwrap();
    app.select_tool("artboard");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let drawn = shapes(&mut app, &ctx);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    // Each name drawn: how far its left edge is right of the artboard's corner, and its bottom above it.
    let named = |name: &str, board: usize| -> Vec<(f32, f32)> {
        let r = app.session.active().unwrap().doc.artboards[board].rect;
        let corner = xf.to_screen(Point::new(r.x0, r.y0));
        drawn
            .iter()
            .filter_map(|s| match s {
                Shape::Text(t) if t.galley.text() == name => Some((t.pos.x - corner.x, corner.y - (t.pos.y + t.galley.size().y))),
                _ => None,
            })
            .collect()
    };
    let (active, other) = (named("01 - Artboard 1", 0), named("02 - Artboard 2", 1));
    assert_eq!(other.len(), 1, "the other artboard is named once: {other:?}");
    assert_eq!(active.len(), 1, "the active artboard is named once: {active:?}");
    assert!((active[0].0 - other[0].0).abs() < 0.6, "flush with the artboard's left edge like the other name: {active:?} vs {other:?}");
    assert!((active[0].1 - other[0].1).abs() < 2.0, "on the artboard's top edge like the other name: {active:?} vs {other:?}");
}

#[test]
fn shift_clicking_canvas_artboard_label_toggles_multi_selection() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    app.select_tool("artboard");
    if let Some(v) = app.view_mut() {
        v.center = Point::new(320.0, 50.0);
        v.zoom = 0.8;
        v.fitted = true;
    }
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_options()["selected"], json!([0]));

    let drawn = shapes(&mut app, &ctx);
    let label_pos = |name: &str| {
        drawn
            .iter()
            .find_map(|s| match s {
                Shape::Text(t) if t.galley.text() == name => Some(t.pos + vec2(10.0, 5.0)),
                _ => None,
            })
            .unwrap()
    };
    let at1 = label_pos("02 - Artboard 2");

    // Shift-click artboard 2's name label (Shift held on press, released before mouse release)
    let shift = egui::Modifiers::SHIFT;
    let button = |pressed| egui::Event::PointerButton { pos: at1, button: egui::PointerButton::Primary, pressed, modifiers: shift };
    frame(&mut app, &ctx, vec![egui::Event::ModifiersChanged(shift), egui::Event::PointerMoved(at1), button(true)]);
    frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::ModifiersChanged(egui::Modifiers::NONE),
            egui::Event::PointerButton { pos: at1, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        ],
    );

    assert_eq!(app.session.tool_options()["selected"], json!([1, 0]), "Shift-clicking artboard label adds it to selection");

    // Click artboard 3's name label without Shift
    let at2 = label_pos("03 - Artboard 3");
    let plain_btn =
        |pressed| egui::Event::PointerButton { pos: at2, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    frame(&mut app, &ctx, vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE), egui::Event::PointerMoved(at2), plain_btn(true)]);
    frame(&mut app, &ctx, vec![plain_btn(false)]);

    assert_eq!(app.session.tool_options()["selected"], json!([2]), "Plain click selects that artboard alone");
}

/// Enable Touch Gestures (#585): a two-finger tap undoes and a three-finger one redoes, and the
/// first finger's press (the pointer egui makes of it) draws nothing. Off, a tap does nothing.
#[test]
fn two_finger_tap_undoes_and_three_finger_tap_redoes() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 40, "height": 40})).unwrap();
    app.select_tool("rectangle");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let count = |app: &VectorcraftApp| app.session.active().unwrap().doc.layers[0].children().unwrap().len();
    let c = app.canvas_rect.unwrap().center();
    let touch = |n: u64, phase, pos| egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(n), phase, pos, force: None };
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    // `fingers` fingers tap at the centre, the first also pressing as the pointer and moving a
    // little (as a rectangle drag would begin).
    let tap = |app: &mut VectorcraftApp, fingers: u64| {
        let at = |n: u64| c + vec2(n as f32 * 50.0, 0.0);
        frame(app, &ctx, vec![touch(0, egui::TouchPhase::Start, at(0)), egui::Event::PointerMoved(at(0)), button(at(0), true)]);
        frame(app, &ctx, vec![touch(0, egui::TouchPhase::Move, at(0) + vec2(6.0, 6.0)), egui::Event::PointerMoved(at(0) + vec2(6.0, 6.0))]);
        frame(app, &ctx, (1..fingers).map(|n| touch(n, egui::TouchPhase::Start, at(n))).collect());
        frame(app, &ctx, vec![touch(0, egui::TouchPhase::End, at(0)), button(at(0) + vec2(6.0, 6.0), false), egui::Event::PointerGone]);
        frame(app, &ctx, (1..fingers).map(|n| touch(n, egui::TouchPhase::End, at(n))).collect());
        frame(app, &ctx, vec![]);
    };
    tap(&mut app, 2);
    assert_eq!(count(&app), 0, "the rectangle undone, none drawn");
    tap(&mut app, 3);
    assert_eq!(count(&app), 1, "redone");
    app.session.prefs.touch_gestures = false;
    tap(&mut app, 2);
    assert_eq!(count(&app), 2, "off: the first finger draws as the pointer, and nothing is undone");
}

/// The Artboard tool's label holds the artboard's name (never translated); the tools' own
/// labels are interface text.
#[test]
fn artboard_labels_name_the_artboard() {
    assert!(names_an_artboard("01 - Layers") && names_an_artboard("12 - Artboard 12 - copy"));
    assert!(!names_an_artboard("anchor") && !names_an_artboard("1 - x") && !names_an_artboard("Off the mesh - move closer"));
}

/// Double-clicking the name drawn above an artboard edits that name and does not fit a new
/// artboard or open Artboard Options.
#[test]
fn double_clicking_an_artboard_name_edits_it() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("artboard.new", &json!({"x": 500, "y": 0, "width": 100, "height": 80, "name": "Far"})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 40, "y": 40, "width": 30, "height": 20})).unwrap();
    app.select_tool("artboard");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let rect = app.canvas_rect.unwrap();
    let xf = Xf::new(rect, app.view().unwrap());
    let mut target = None;
    let _ = ctx.run_ui(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() }, |ui| {
        let ab = &app.session.active().unwrap().doc.artboards[0];
        target = Some(tool_name_rect(ui.painter(), &xf, ab.rect, 0, &ab.name).center());
    });
    let at = target.unwrap();
    assert!(rect.contains(at), "the name is in the working area, at {at:?} canvas {rect:?}");
    let before = app.session.active().unwrap().doc.artboards.len();
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let click = |app: &mut VectorcraftApp| {
        frame(app, &ctx, vec![egui::Event::PointerMoved(at)]);
        frame(app, &ctx, vec![button(true)]);
        frame(app, &ctx, vec![button(false)]);
    };
    click(&mut app);
    click(&mut app);
    assert!(ctx.data(|d| d.get_temp::<ArtboardNameEdit>(name_edit_id()).is_some()), "the name is being edited");
    assert_eq!(app.session.active().unwrap().doc.artboards.len(), before, "clicking the name does not fit an artboard");
    assert!(app.ui.dialog.is_none(), "Artboard Options stays closed");
    commit_artboard_name(&mut app, 0, "Poster");
    assert_eq!(app.session.active().unwrap().doc.artboards[0].name, "Poster");
    commit_artboard_name(&mut app, 0, "   ");
    assert_eq!(app.session.active().unwrap().doc.artboards[0].name, "Poster", "an empty name is left as it is");
}

/// Double-clicking the name drawn above a locked artboard doesn't edit it (the Artboards panel
/// still renames the artboard), fit an artboard or open Artboard Options.
#[test]
fn double_clicking_a_locked_artboards_name_leaves_it() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 40, "y": 40, "width": 30, "height": 20})).unwrap();
    app.session.execute("artboard.setProps", &json!({"index": 0, "locked": true})).unwrap();
    app.select_tool("artboard");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let rect = app.canvas_rect.unwrap();
    let xf = Xf::new(rect, app.view().unwrap());
    let mut target = None;
    let _ = ctx.run_ui(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() }, |ui| {
        let ab = &app.session.active().unwrap().doc.artboards[0];
        target = Some(tool_name_rect(ui.painter(), &xf, ab.rect, 0, &ab.name).center());
    });
    let at = target.unwrap();
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    for _ in 0..2 {
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
        frame(&mut app, &ctx, vec![button(true)]);
        frame(&mut app, &ctx, vec![button(false)]);
    }
    assert!(ctx.data(|d| d.get_temp::<ArtboardNameEdit>(name_edit_id()).is_none()), "the name isn't edited");
    assert_eq!(app.session.active().unwrap().doc.artboards.len(), 1);
    assert!(app.ui.dialog.is_none(), "Artboard Options stays closed");
}

/// The canvas artboard name field keeps its own inset: egui 0.37 insets a text field by the theme's button
/// padding unless told otherwise, but this field sits over the canvas label, so its glyphs must not move when
/// that padding changes (an ordinary themed field's do, as the probe shows).
#[test]
fn artboard_name_field_glyphs_stay_put_when_button_padding_changes() {
    /// Where the frame paints `text` (a field's text starts at its galley's top left).
    fn text_at(shapes: &[Shape], text: &str) -> Pos2 {
        shapes
            .iter()
            .find_map(|s| match s {
                Shape::Text(t) if t.galley.text() == text => Some(t.pos),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no {text:?} text"))
    }
    /// Build raw input with a fixed 800 by 600 screen so headless frames keep a stable canvas texture
    /// size.
    fn raw() -> egui::RawInput {
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() }
    }
    /// The name field's text origin relative to its rectangle with the theme's button padding `padding`: the
    /// field opened as a double click opens it, measured in the steady frame after it takes focus.
    fn editor_inset(padding: Vec2) -> Vec2 {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.session.execute("artboard.setProps", &json!({"index": 0, "name": "Inset"})).unwrap();
        app.select_tool("artboard");
        app.ui.task_bar = false;
        app.ui.view.rulers = false;
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, crate::theme::Brightness::default());
        ctx.global_style_mut(|s| s.spacing.button_padding = padding);
        ctx.run_ui(raw(), |ui| begin_artboard_name_edit(ui, &app, 0)).textures_delta.clear();
        // The first frame opens and focuses the field; the next one is its steady state.
        frame(&mut app, &ctx, vec![]);
        let (shapes, mut delta) = frame_output(&mut app, &ctx);
        delta.clear();
        let origin = text_at(&shapes, "Inset");
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let mut field = None;
        ctx.run_ui(raw(), |ui| field = artboard_name_field_rect(&app, ui, &xf, 0).map(|r| r.min)).textures_delta.clear();
        origin - field.unwrap()
    }
    /// An ordinary themed field's text origin relative to its rectangle: the padding moves it.
    fn plain_inset(padding: Vec2) -> Vec2 {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, crate::theme::Brightness::default());
        ctx.global_style_mut(|s| s.spacing.button_padding = padding);
        let mut text = String::from("Probe");
        let rect = egui::Rect::from_min_size(pos2(40.0, 40.0), vec2(140.0, 18.0));
        let mut out = ctx.run_ui(raw(), |ui| {
            ui.put(rect, egui::TextEdit::singleline(&mut text).id(egui::Id::unique("inset-probe")).font(egui::FontId::proportional(11.0)));
        });
        out.textures_delta.clear();
        let shapes: Vec<Shape> = out.shapes.into_iter().map(|c| c.shape).collect();
        text_at(&shapes, "Probe") - rect.min
    }
    let (old, themed) = (vec2(4.0, 2.0), vec2(6.0, 2.0));
    let (editor_old, editor_themed) = (editor_inset(old), editor_inset(themed));
    let (plain_old, plain_themed) = (plain_inset(old), plain_inset(themed));
    assert!((plain_themed.x - plain_old.x).abs() > 1.0, "the probe field follows the padding: {plain_old:?} → {plain_themed:?}");
    assert!((editor_themed - editor_old).length() < 0.01, "the artboard name moves with the button padding: {editor_old:?} → {editor_themed:?}");
}

/// #949: an artboard's name stays within the artboard's width on screen, ending in "…" when
/// longer, and an artboard too narrow shows none.
#[test]
fn artboard_labels_fit_their_artboard() {
    let ctx = egui::Context::default();
    let drawn = |width: f32| {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            artboard_label(ui.painter(), pos2(50.0, 50.0), width, "01 - A long artboard name", Color32::WHITE)
        });
        out.textures_delta.clear();
        out.shapes
            .into_iter()
            .filter_map(|c| match c.shape {
                egui::Shape::Text(t) => Some((t.galley.text().to_string(), t.galley.size().x)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let full = drawn(400.0);
    assert_eq!(full.first().map(|(s, _)| s.as_str()), Some("01 - A long artboard name"));
    // Laid out no wider than the artboard (the galley keeps the whole text; it draws "…").
    let (_, w) = drawn(30.0).first().cloned().unwrap();
    assert!(full[0].1 > 30.0 && w <= 30.0 + 0.5, "{full:?} {w}");
    assert!(drawn(10.0).is_empty());
}

fn middle(pos: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos, button: egui::PointerButton::Middle, pressed, modifiers: Default::default() }
}

/// One headless frame with the canvas under a 40-point bar, as it is under the document tabs.
fn frame_under_bar(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| {
        ui.add_space(40.0);
        show(app, ui);
    });
    out.textures_delta.clear();
}

#[test]
fn a_click_just_outside_the_canvas_does_not_reach_the_tool() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    // A square in the middle of the artboard, where the view is centred.
    let id = app.session.execute("shape.rectangle", &json!({"x": 180, "y": 130, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.none", &json!({})).unwrap();
    let ctx = egui::Context::default();
    frame_under_bar(&mut app, &ctx, vec![]);
    let rect = app.canvas_rect.unwrap();
    let click = |app: &mut VectorcraftApp, p: Pos2| {
        let button = |pressed| egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame_under_bar(app, &ctx, vec![egui::Event::PointerMoved(p)]);
        frame_under_bar(app, &ctx, vec![button(true)]);
        frame_under_bar(app, &ctx, vec![button(false)]);
    };
    let selected = |app: &VectorcraftApp| app.session.active().unwrap().selection.objects.clone();
    // Two pixels above the canvas, within egui's interaction radius: nothing happens.
    click(&mut app, pos2(rect.center().x, rect.top() - 2.0));
    assert!(selected(&app).is_empty(), "a click above the canvas selected {:?}", selected(&app));
    // At the canvas's centre: the square.
    click(&mut app, rect.center());
    assert_eq!(selected(&app), vec![vectorcraft_doc::NodeId(id)]);
}

#[test]
fn a_drag_from_a_ruler_onto_the_canvas_makes_a_guide() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.ui.view.rulers = true;
    app.ui.view.guides = false;
    // A drawing tool: the drag must not draw.
    app.select_tool("rectangle");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let rect = app.canvas_rect.unwrap();
    let xf = Xf::new(rect, app.view().unwrap());
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let drag = |app: &mut VectorcraftApp, from: Pos2, to: Pos2| {
        frame(app, &ctx, vec![egui::Event::PointerMoved(from)]);
        frame(app, &ctx, vec![button(from, true)]);
        frame(app, &ctx, vec![egui::Event::PointerMoved(to)]);
        frame(app, &ctx, vec![button(to, false)]);
    };
    let guides = |app: &VectorcraftApp| app.session.active().unwrap().doc.guides.iter().map(|g| (g.vertical, g.pos)).collect::<Vec<_>>();
    // From the top ruler: a horizontal guide where the button was released.
    let to = pos2(rect.center().x, rect.top() + 120.0);
    drag(&mut app, pos2(rect.center().x, rect.top() - RULER / 2.0), to);
    assert_eq!(guides(&app), [(false, xf.to_doc(to).y)]);
    assert!(app.ui.view.guides, "the new guide shows");
    // From the left ruler: a vertical one.
    let to2 = pos2(rect.left() + 200.0, rect.center().y);
    drag(&mut app, pos2(rect.left() - RULER / 2.0, rect.center().y), to2);
    assert_eq!(guides(&app), [(false, xf.to_doc(to).y), (true, xf.to_doc(to2).x)]);
    // Released back on the ruler: no guide.
    drag(&mut app, pos2(rect.center().x, rect.top() - RULER / 2.0), pos2(rect.center().x + 40.0, rect.top() - 4.0));
    assert_eq!(guides(&app).len(), 2);
    assert_eq!(app.session.active().unwrap().doc.art_bounds(), None, "the tool drew nothing");
}

/// #4646: a guide dragged out of a ruler shows as a line across the canvas while it is dragged,
/// also with Preferences › Performance › Real-time Drawing and Editing off (only its position
/// label showed then).
#[test]
fn a_guide_dragged_out_of_a_ruler_shows_while_dragged() {
    for real_time in [true, false] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.session.execute("prefs.set", &json!({"key": "realTimeDrawing", "value": real_time})).unwrap();
        app.ui.view.rulers = true;
        app.ui.view.smart_guides = false;
        app.ui.view.snap_to_point = false;
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let rect = app.canvas_rect.unwrap();
        let zoom = app.view().unwrap().zoom as f32;
        let from = pos2(rect.center().x, rect.top() - RULER / 2.0);
        let at = pos2(rect.center().x, rect.top() + 120.0);
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from)]);
        frame(&mut app, &ctx, vec![button(from, true)]);
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
        // A horizontal line across the canvas at the pointer (snapped to a whole point at most).
        let across = |s: &Shape| {
            matches!(s, Shape::LineSegment { points, .. }
                if (points[0].y - at.y).abs() <= zoom && points[0].y == points[1].y && (points[1].x - points[0].x).abs() >= rect.width() - 0.5)
        };
        assert!(shapes(&mut app, &ctx).iter().any(across), "real-time drawing {real_time}: the guide shows while dragged");
        frame(&mut app, &ctx, vec![button(at, false)]);
        assert_eq!(app.session.active().unwrap().doc.guides.len(), 1, "real-time drawing {real_time}: released, it is made");
    }
}

#[test]
fn new_ruler_guides_follow_alt_through_preview_release_and_undo() {
    use egui::{Event, Modifiers};
    for source_vertical in [false, true] {
        for initially_alt in [false, true] {
            let mut app = VectorcraftApp::new(Session::new(), Default::default());
            app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
            app.session.execute("document.setRulerOrigin", &json!({"x": 17, "y": 23})).unwrap();
            app.ui.view.rulers = true;
            app.ui.view.smart_guides = false;
            app.ui.view.snap_to_point = false;
            app.select_tool("rectangle");
            let ctx = egui::Context::default();
            frame(&mut app, &ctx, vec![]);
            let rect = app.canvas_rect.unwrap();
            let xf = Xf::new(rect, app.view().unwrap());
            let from =
                if source_vertical { pos2(rect.left() - RULER / 2.0, rect.center().y) } else { pos2(rect.center().x, rect.top() - RULER / 2.0) };
            let at = xf.to_screen(Point::new(80.0, 120.0));
            assert!(rect.contains(at));
            let held = |alt| if alt { Modifiers::ALT } else { Modifiers::NONE };
            let button = |pos, pressed, alt| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: held(alt) };
            let before = app.session.active().unwrap().doc.clone();
            let undo = app.session.active().unwrap().history.undo.len();
            frame(&mut app, &ctx, vec![Event::ModifiersChanged(held(initially_alt)), Event::PointerMoved(from)]);
            frame(&mut app, &ctx, vec![button(from, true, initially_alt)]);
            frame(&mut app, &ctx, vec![Event::PointerMoved(at)]);
            let assert_guide = |app: &VectorcraftApp, alt: bool, pos: Point| {
                let guides = &app.session.active().unwrap().doc.guides;
                assert_eq!(guides.len(), 1);
                assert_eq!(guides[0].vertical, source_vertical != alt);
                let expected = if source_vertical != alt { pos.x } else { pos.y };
                assert!((guides[0].pos - expected).abs() < 1e-4, "{:?}, expected {expected}", guides[0]);
                assert_eq!(guides[0].artboard, None);
            };
            assert_guide(&app, initially_alt, Point::new(80.0, 120.0));
            // Modifier changes with no pointer movement refresh the existing preview.
            for alt in [!initially_alt, initially_alt] {
                frame(&mut app, &ctx, vec![Event::ModifiersChanged(held(alt))]);
                assert_guide(&app, alt, Point::new(80.0, 120.0));
                assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
            }
            // Equal coordinates must not hide an orientation change in the preview.
            let equal = xf.to_screen(Point::new(120.0, 120.0));
            frame(&mut app, &ctx, vec![Event::PointerMoved(equal)]);
            frame(&mut app, &ctx, vec![Event::ModifiersChanged(held(!initially_alt))]);
            assert_guide(&app, !initially_alt, Point::new(120.0, 120.0));
            let release = Point::new(147.0, 183.0);
            let to = xf.to_screen(release);
            frame(&mut app, &ctx, vec![Event::ModifiersChanged(Modifiers::ALT), Event::PointerMoved(to), button(to, false, true)]);
            assert_guide(&app, true, release);
            let st = app.session.active().unwrap();
            assert_eq!(st.history.undo.len(), undo + 1);
            assert_eq!(st.history.undo.last().unwrap().label, "New Guide");
            assert_eq!(st.doc.ruler_origin, before.ruler_origin);
            assert_eq!(st.doc.art_bounds(), before.art_bounds(), "the rectangle tool never drew");
            let committed = st.doc.clone();
            // Canvas guides still span the viewport after changing orientation.
            let painted = shapes(&mut app, &ctx);
            let window = app.canvas_rect.unwrap();
            let position = Xf::new(window, app.view().unwrap()).to_screen(release);
            let ends = if source_vertical {
                [pos2(window.left(), position.y), pos2(window.right(), position.y)]
            } else {
                [pos2(position.x, window.top()), pos2(position.x, window.bottom())]
            };
            assert!(painted.iter().any(|s| matches!(s, Shape::LineSegment { points, .. }
                if (points[0] - ends[0]).length() < 0.01 && (points[1] - ends[1]).length() < 0.01)));
            app.session.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(&*app.session.active().unwrap().doc, &*before);
            app.session.execute("edit.redo", &json!({})).unwrap();
            assert_eq!(&*app.session.active().unwrap().doc, &*committed);
        }
    }
}

#[test]
fn new_ruler_guide_alt_cancel_keeps_document_and_redo() {
    use egui::{Event, Modifiers};
    for source_vertical in [false, true] {
        for tool in ["rectangle", "artboard"] {
            let mut app = VectorcraftApp::new(Session::new(), Default::default());
            app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
            app.session.execute("document.setRulerOrigin", &json!({"x": 17, "y": 23})).unwrap();
            app.session.execute("shape.rectangle", &json!({"x": 40, "y": 50, "width": 20, "height": 30})).unwrap();
            app.session.execute("edit.undo", &json!({})).unwrap();
            app.ui.view.rulers = true;
            app.ui.view.smart_guides = false;
            app.ui.view.snap_to_point = false;
            app.select_tool(tool);
            let ctx = egui::Context::default();
            frame(&mut app, &ctx, vec![]);
            let rect = app.canvas_rect.unwrap();
            let xf = Xf::new(rect, app.view().unwrap());
            let from =
                if source_vertical { pos2(rect.left() - RULER / 2.0, rect.center().y) } else { pos2(rect.center().x, rect.top() - RULER / 2.0) };
            let at = xf.to_screen(Point::new(80.0, 120.0));
            let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::ALT };
            let before = app.session.active().unwrap().doc.clone();
            let history = |app: &VectorcraftApp| {
                let st = app.session.active().unwrap();
                (st.history.undo.len(), st.history.redo.len(), st.is_dirty())
            };
            let before_history = history(&app);
            assert_eq!(before_history.1, 1);
            frame(&mut app, &ctx, vec![Event::ModifiersChanged(Modifiers::ALT), Event::PointerMoved(from)]);
            frame(&mut app, &ctx, vec![button(from, true)]);
            frame(&mut app, &ctx, vec![Event::PointerMoved(at)]);
            let guides = &app.session.active().unwrap().doc.guides;
            assert_eq!(guides.len(), 1);
            assert_eq!(guides[0].vertical, !source_vertical);
            let artboard = (tool == "artboard").then_some(before.artboards[0].id);
            assert_eq!(guides[0].artboard, artboard);
            // Returning to the source ruler cancels the preview and keeps the existing Redo.
            frame(&mut app, &ctx, vec![Event::PointerMoved(from), button(from, false)]);
            assert_eq!(&*app.session.active().unwrap().doc, &*before);
            assert_eq!(history(&app), before_history);
            app.session.execute("edit.redo", &json!({})).unwrap();
            assert!(app.session.active().unwrap().doc.art_bounds().is_some());
            assert!(app.session.active().unwrap().doc.guides.is_empty());
        }
    }
}

/// #451: a guide dragged out of a ruler lands on the art's side midpoints (halving an
/// artboard-sized rectangle) and on the artboard's edges; an artboard guide is drawn across
/// its artboard only.
#[test]
fn guides_from_the_rulers_snap_and_artboard_guides_span_their_artboard() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 400, "height": 300})).unwrap();
    app.session.execute("select.none", &json!({})).unwrap();
    app.ui.view.rulers = true;
    app.select_tool("selection");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let rect = app.canvas_rect.unwrap();
    let xf = Xf::new(rect, app.view().unwrap());
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let drag = |app: &mut VectorcraftApp, from: Pos2, to: Pos2| {
        frame(app, &ctx, vec![egui::Event::PointerMoved(from)]);
        frame(app, &ctx, vec![button(from, true)]);
        frame(app, &ctx, vec![egui::Event::PointerMoved(to)]);
        frame(app, &ctx, vec![button(to, false)]);
    };
    let guides = |app: &VectorcraftApp| app.session.active().unwrap().doc.guides.iter().map(|g| (g.vertical, g.pos)).collect::<Vec<_>>();
    // 3 px off the middle of the top and bottom sides: onto the line through them.
    let mid = xf.to_screen(Point::new(200.0, 150.0));
    drag(&mut app, pos2(rect.left() - RULER / 2.0, mid.y), mid + vec2(3.0, 40.0));
    // 3 px inside the artboard's bottom edge: onto it.
    let bottom = xf.to_screen(Point::new(100.0, 300.0));
    drag(&mut app, pos2(bottom.x, rect.top() - RULER / 2.0), bottom - vec2(0.0, 3.0));
    assert_eq!(guides(&app), [(true, 200.0), (false, 300.0)]);
    assert_eq!(app.session.active().unwrap().history.undo.last().unwrap().label, "New Guide");
    // An artboard guide runs from the artboard's left edge to its right edge.
    app.session.execute("guide.add", &json!({"vertical": false, "pos": 75, "artboard": 0})).unwrap();
    let s = shapes(&mut app, &ctx);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let (a, b) = (xf.to_screen(Point::new(0.0, 75.0)), xf.to_screen(Point::new(400.0, 75.0)));
    let at = |p: Pos2, q: Pos2| (p - q).length() < 0.01;
    assert!(s.iter().any(|s| matches!(s, Shape::LineSegment { points, .. } if at(points[0], a) && at(points[1], b))), "across the artboard");
    // A canvas guide crosses the whole window.
    let y = xf.to_screen(Point::new(0.0, 300.0)).y;
    let window = app.canvas_rect.unwrap();
    assert!(
        s.iter()
            .any(|s| matches!(s, Shape::LineSegment { points, .. } if points[0] == pos2(window.left(), y) && points[1] == pos2(window.right(), y)))
    );
}

/// #414: guides dragged out of the rulers are picked and dragged with the Selection tool
/// (highlighted while selected), go with Delete or Backspace, and dragged back onto their
/// ruler.
#[test]
fn the_selection_tool_moves_and_deletes_ruler_guides() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    // A square under the guides: the press takes the guide.
    app.session.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 300, "height": 200})).unwrap();
    app.session.execute("select.none", &json!({})).unwrap();
    let art = app.session.active().unwrap().doc.art_bounds();
    app.ui.view.rulers = true;
    app.select_tool("selection");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let rect = app.canvas_rect.unwrap();
    let xf = Xf::new(rect, app.view().unwrap());
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let drag = |app: &mut VectorcraftApp, from: Pos2, to: Pos2| {
        frame(app, &ctx, vec![egui::Event::PointerMoved(from)]);
        frame(app, &ctx, vec![button(from, true)]);
        frame(app, &ctx, vec![egui::Event::PointerMoved(to)]);
        frame(app, &ctx, vec![button(to, false)]);
    };
    let guides = |app: &VectorcraftApp| app.session.active().unwrap().doc.guides.iter().map(|g| (g.vertical, g.pos)).collect::<Vec<_>>();
    // Two guides out of the rulers.
    let (h, v) = (pos2(rect.center().x + 30.0, rect.center().y + 20.0), pos2(rect.center().x - 40.0, rect.center().y));
    drag(&mut app, pos2(h.x, rect.top() - RULER / 2.0), h);
    drag(&mut app, pos2(rect.left() - RULER / 2.0, v.y), v);
    assert_eq!(guides(&app), [(false, xf.to_doc(h).y), (true, xf.to_doc(v).x)]);
    // Drag the horizontal one down 30 px (across the vertical one: the nearer is picked).
    let to = pos2(h.x + 10.0, h.y + 30.0);
    drag(&mut app, h, to);
    let st = app.session.active().unwrap();
    assert_eq!(guides(&app), [(false, xf.to_doc(to).y), (true, xf.to_doc(v).x)]);
    assert_eq!((st.selection.guides.clone(), st.selection.objects.clone()), (vec![0], vec![]));
    assert_eq!(st.history.undo.last().unwrap().label, "Move Guide");
    let picked = Tokens::get(&ctx).selection;
    let s = shapes(&mut app, &ctx);
    assert!(s.iter().any(|s| matches!(s, Shape::LineSegment { points, stroke } if stroke.color == picked && points[0].y == to.y)), "highlighted");
    // Backspace deletes the selected guide; Delete too.
    let press = |app: &mut VectorcraftApp, key| {
        let ev = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        let mut out = ctx.run_ui(egui::RawInput { events: vec![ev], ..Default::default() }, |ui| crate::shortcuts::handle(app, ui.ctx()));
        out.textures_delta.clear();
    };
    press(&mut app, egui::Key::Backspace);
    assert_eq!(guides(&app), [(true, xf.to_doc(v).x)]);
    app.session.execute("guide.select", &json!({"indexes": [0]})).unwrap();
    press(&mut app, egui::Key::Delete);
    assert!(guides(&app).is_empty());
    app.session.execute("edit.undo", &json!({})).unwrap();
    // Hide Guides deselects them.
    app.session.execute("guide.select", &json!({"indexes": [0]})).unwrap();
    app.run("view.guides", json!({})).unwrap();
    assert!(app.session.active().unwrap().selection.guides.is_empty());
    app.run("view.guides", json!({})).unwrap();
    // Dragged back onto its ruler, the vertical guide goes.
    drag(&mut app, v, pos2(rect.left() - RULER / 2.0, v.y + 20.0));
    assert!(guides(&app).is_empty(), "{:?}", guides(&app));
    assert_eq!(app.session.active().unwrap().doc.art_bounds(), art, "the square stayed");
}

#[test]
fn cmd_with_another_tool_drags_with_the_selection_tool_used_last() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 50})).unwrap()["id"].as_u64().unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let cmd_frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let events = std::iter::once(egui::Event::ModifiersChanged(egui::Modifiers::COMMAND)).chain(events).collect();
        let raw = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
    };
    let button =
        |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::COMMAND };
    // A Cmd drag from `a` to `b` with the Pen tool; → the tool that did it.
    let cmd_drag = |app: &mut VectorcraftApp, a: Pos2, b: Pos2| {
        app.select_tool("pen");
        cmd_frame(app, vec![egui::Event::PointerMoved(a), button(a, true)]);
        let used = app.session.tool_id().to_string();
        cmd_frame(app, vec![egui::Event::PointerMoved(b)]);
        cmd_frame(app, vec![button(b, false)]);
        assert_eq!(app.session.tool_id(), "pen", "back to the Pen tool");
        used
    };
    let bounds = |app: &VectorcraftApp| app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap();
    // Direct Selection used last: the drag moves the rectangle's bottom-right corner only (once
    // the rectangle isn't selected whole, which would move all its anchors).
    app.select_tool("directSelection");
    app.session.execute("select.none", &json!({})).unwrap();
    let corner = xf.to_screen(Point::new(200.0, 150.0));
    assert_eq!(cmd_drag(&mut app, corner, corner + vec2(20.0, 20.0)), "directSelection");
    let b = bounds(&app);
    assert_eq!((b.x0, b.y0), (100.0, 100.0));
    assert!((b.x1 - xf.to_doc(corner + vec2(20.0, 20.0)).x).abs() < 1e-6, "{b:?}");
    // Group Selection, then Selection: each is the one used.
    app.select_tool("groupSelection");
    let inside = xf.to_screen(Point::new(130.0, 120.0));
    assert_eq!(cmd_drag(&mut app, inside, inside), "groupSelection");
    app.select_tool("selection");
    assert_eq!(cmd_drag(&mut app, inside, inside), "selection");
}

/// #525: a selection with more anchors than the overlay draws (a traced photo) shows its bounds,
/// not every outline and anchor, which made a mesh larger than the GPU takes; an ordinary one
/// still shows its outline and anchors.
#[test]
fn a_huge_selection_shows_its_bounds_instead_of_every_outline() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let points = |n: usize| (0..n).map(|i| format!("L{} {}", 20.0 + 360.0 * i as f64 / n as f64, 100 + (i % 2) * 50)).collect::<String>();
    let ctx = egui::Context::default();
    let mut drawn = |n: usize| {
        app.session.execute("path.create", &json!({"d": format!("M20 20 {} Z", points(n))})).unwrap();
        frame(&mut app, &ctx, vec![]);
        let shapes = shapes(&mut app, &ctx);
        let points: usize = shapes.iter().map(|s| if let Shape::Path(ps) = s { ps.points.len() } else { 0 }).sum();
        let squares = shapes.iter().filter(|s| matches!(s, Shape::Rect(_))).count();
        (points, squares)
    };
    let (points, squares) = drawn(OVERLAY_MAX_ANCHORS + 10);
    assert!(points < 100 && squares < 100, "{points} outline points, {squares} squares");
    let (points, squares) = drawn(200);
    assert!(points > 200 && squares > 200, "{points} outline points, {squares} squares");
}

/// #2460: type, images, symbol instances, gradient meshes and envelopes count toward the
/// points the overlay outlines, as paths' anchors do. 32,768 of one kind (an opened PDF can
/// hold as many type lines, images or mesh pieces) add only the selection's bounds to the
/// frame's mesh, under 100 vertices; one alone adds its outline.
#[test]
fn a_huge_selection_of_type_images_symbols_or_meshes_shows_its_bounds() {
    for kind in ["point type", "area type", "type on a path", "image", "symbol", "mesh", "envelope"] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.ui.task_bar = false;
        // The Rectangle tool draws nothing of its own (no bounding box, no type on a path
        // brackets): only the selection's overlay changes the frame.
        app.select_tool("rectangle");
        match kind {
            "point type" => {
                app.session.execute("text.create", &json!({"x": 20, "y": 40, "text": "Type"})).unwrap();
            }
            "area type" => {
                let area = json!({"width": 150, "height": 20});
                app.session.execute("text.create", &json!({"x": 20, "y": 40, "text": "Type", "area": area})).unwrap();
            }
            "type on a path" => {
                let path = app.session.execute("path.create", &json!({"d": "M20 100 L140 100 L260 100 L380 100"})).unwrap()["id"].clone();
                app.session.execute("text.createInPath", &json!({"path": path, "mode": "onPath", "text": "Type"})).unwrap();
            }
            "image" => {
                let data = vectorcraft_format::base64_encode(&crate::tests_sysclip::blue_png());
                app.run("file.place", json!({"name": "blue.png", "dataBase64": data, "link": false, "at": [200, 150]})).unwrap();
            }
            "symbol" => {
                app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 20})).unwrap();
                app.session.execute("symbol.new", &json!({})).unwrap();
            }
            "mesh" => {
                app.session.execute("shape.rectangle", &json!({"x": 150, "y": 50, "width": 30, "height": 20})).unwrap();
                app.session.execute("object.mesh.create", &json!({"rows": 1, "cols": 1})).unwrap();
            }
            _ => {
                app.session.execute("shape.rectangle", &json!({"x": 150, "y": 50, "width": 30, "height": 20})).unwrap();
                app.session.execute("object.envelope.makeWithMesh", &json!({"rows": 1, "cols": 1})).unwrap();
            }
        }
        let ctx = egui::Context::default();
        // The vertices that selecting everything adds to the frame's mesh.
        let overlay = |app: &mut VectorcraftApp| {
            app.session.execute("select.none", &json!({})).unwrap();
            let none = frame_vertices(app, &ctx);
            app.session.execute("select.all", &json!({})).unwrap();
            frame_vertices(app, &ctx).saturating_sub(none)
        };
        let one = overlay(&mut app);
        assert!(one > 0, "{kind}: one shows its outline");
        for _ in 0..15 {
            app.session.execute("select.all", &json!({})).unwrap();
            app.run("edit.duplicate", json!({})).unwrap();
        }
        let many = overlay(&mut app);
        assert_eq!(app.session.active().unwrap().selection.objects.len(), 32_768, "{kind}");
        assert!(many < 100, "{kind}: 32,768 show the selection's bounds, not {many} vertices");
    }
}

/// #2460: a group of 32,768 images is too much to outline: under the pointer it is not
/// highlighted, and as the key object its outline is its bounds.
#[test]
fn a_group_of_too_many_images_has_no_hover_outline_and_a_key_outline_of_its_bounds() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let data = vectorcraft_format::base64_encode(&crate::tests_sysclip::blue_png());
    app.run("file.place", json!({"name": "blue.png", "dataBase64": data, "link": false, "at": [200, 150]})).unwrap();
    for _ in 0..15 {
        app.session.execute("select.all", &json!({})).unwrap();
        app.run("edit.duplicate", json!({})).unwrap();
    }
    app.session.execute("select.all", &json!({})).unwrap();
    let group = app.session.execute("object.group", &json!({})).unwrap()["id"].clone();
    app.session.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 30, "height": 30})).unwrap();
    app.select_tool("selection");
    app.session.execute("select.none", &json!({})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let over = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap()).to_screen(Point::new(200.0, 150.0));
    let lines = |app: &mut VectorcraftApp, width: f32| {
        frame(app, &ctx, vec![egui::Event::PointerMoved(over)]);
        shapes(app, &ctx).iter().filter(|s| matches!(s, Shape::Path(ps) if ps.stroke.width == width)).count()
    };
    assert_eq!(lines(&mut app, 1.5), 0, "no hover outline");
    app.session.execute("select.all", &json!({})).unwrap();
    app.session.execute("select.key", &json!({"id": group})).unwrap();
    assert_eq!(lines(&mut app, KEY_OUTLINE), 1, "the key object's bounds");
}

/// #1111: a selected gradient mesh shows its mesh lines and points to the selection tools, not
/// only to the Mesh tool.
#[test]
fn a_selected_gradient_mesh_shows_its_lines_and_points() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 200, "height": 100})).unwrap();
    app.session.execute("object.mesh.create", &json!({"rows": 2, "cols": 2})).unwrap();
    let ctx = egui::Context::default();
    let drawn = |app: &mut VectorcraftApp| {
        frame(app, &ctx, vec![]);
        let shapes = shapes(app, &ctx);
        let lines = shapes.iter().filter(|s| matches!(s, Shape::Path(ps) if ps.points.len() > 1)).count();
        (lines, shapes.iter().filter(|s| matches!(s, Shape::Rect(_))).count())
    };
    let (none_lines, none_points) = {
        app.session.execute("select.none", &json!({})).unwrap();
        drawn(&mut app)
    };
    for tool in ["selection", "directSelection"] {
        app.select_tool(tool);
        app.session.execute("select.all", &json!({})).unwrap();
        let (lines, points) = drawn(&mut app);
        // The outline, two inner lines each way; the 3 × 3 points.
        assert!(lines >= none_lines + 3 && points >= none_points + 9, "{tool}: {lines} lines, {points} points");
    }
}

/// #541: the key object's outline is drawn thicker than the rest of the selection's.
#[test]
fn the_key_object_has_a_thicker_outline() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let a = app.session.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 50, "height": 50})).unwrap()["id"].clone();
    let b = app.session.execute("shape.ellipse", &json!({"x": 200, "y": 100, "width": 60, "height": 40})).unwrap()["id"].clone();
    app.session.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let thick =
        |app: &mut VectorcraftApp| shapes(app, &ctx).iter().filter(|s| matches!(s, Shape::Path(ps) if ps.stroke.width == KEY_OUTLINE)).count();
    assert_eq!(thick(&mut app), 0, "no key yet");
    app.session.execute("select.key", &json!({"id": b})).unwrap();
    assert!(thick(&mut app) > 0, "the key's outline");
}

/// One headless canvas frame → the shapes drawn, `Shape::Vec`s flattened.
fn shapes(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<Shape> {
    let (shapes, mut delta) = frame_output(app, ctx);
    delta.clear();
    shapes
}

/// Two headless canvas frames → the vertices of the second one's tessellated mesh, which the
/// GPU takes in one buffer.
fn frame_vertices(app: &mut VectorcraftApp, ctx: &egui::Context) -> usize {
    frame(app, ctx, vec![]);
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| show(app, ui));
    out.textures_delta.clear();
    let mesh = |p: &egui::ClippedPrimitive| if let egui::epaint::Primitive::Mesh(m) = &p.primitive { m.vertices.len() } else { 0 };
    ctx.tessellate(out.shapes, out.pixels_per_point).iter().map(mesh).sum()
}

/// One headless canvas frame → the shapes drawn (`Shape::Vec`s flattened) and the textures
/// uploaded by it (the art's raster among them, when it was re-rendered).
fn frame_output(app: &mut VectorcraftApp, ctx: &egui::Context) -> (Vec<Shape>, egui::TexturesDelta) {
    fn flat(s: Shape, out: &mut Vec<Shape>) {
        match s {
            Shape::Vec(v) => v.into_iter().for_each(|s| flat(s, out)),
            s => out.push(s),
        }
    }
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
    let out = ctx.run_ui(raw, |ui| show(app, ui));
    let mut v = vec![];
    out.shapes.into_iter().for_each(|c| flat(c.shape, &mut v));
    (v, out.textures_delta)
}

/// An edit of one object renders the canvas again only where it changed, into its texture; a
/// change of the view renders all of it.
#[test]
fn an_edit_renders_only_its_part_of_the_canvas() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width":400,"height":300})).unwrap();
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    app.canvas.worker_started = true; // every frame on this thread, however long it takes
    let id = app.run("shape.rectangle", json!({"x":50,"y":50,"width":40,"height":30})).unwrap()["id"].as_u64().unwrap();
    app.run("shape.rectangle", json!({"x":250,"y":200,"width":40,"height":30})).unwrap();
    app.run("select.none", json!({})).unwrap();
    let ctx = egui::Context::default();
    // Where the canvas texture changed in a frame, and how much of it.
    let canvas = |app: &VectorcraftApp, mut delta: egui::TexturesDelta| {
        let tex = app.canvas.texture.as_ref().map(|t| t.id());
        let set = delta.set.iter().find(|(t, _)| tex == Some(**t)).and_then(|(_, d)| d.last()).map(|d| (d.pos, d.image.size()));
        delta.clear();
        set
    };
    let (_, delta) = frame_output(&mut app, &ctx);
    assert!(canvas(&app, delta).is_some_and(|(pos, _)| pos.is_none()), "the first frame renders all of it");
    app.run("paint.setFill", json!({"color": "#ff0000", "ids": [id]})).unwrap();
    let (_, delta) = frame_output(&mut app, &ctx);
    let (pos, size) = canvas(&app, delta).expect("the edit shows");
    let whole = app.canvas.key.as_ref().map(|k| (k.w as usize, k.h as usize)).unwrap();
    assert!(pos.is_some() && size[0] * size[1] * 10 < whole.0 * whole.1, "a small part of it: {pos:?} {size:?} of {whole:?}");
    // An edit with other render options than the frame shown's renders all of it.
    app.session.prefs.pattern_tile_edge_color = "#ff00ff".into();
    app.run("paint.setFill", json!({"color": "#00ff00", "ids": [id]})).unwrap();
    let (_, delta) = frame_output(&mut app, &ctx);
    assert!(canvas(&app, delta).is_some_and(|(pos, _)| pos.is_none()), "other options render all of it");
    app.run("view.setZoom", json!({"zoom": 2})).unwrap();
    let (_, delta) = frame_output(&mut app, &ctx);
    assert!(canvas(&app, delta).is_some_and(|(pos, _)| pos.is_none()), "a new view renders all of it");
}

/// A placed document's bitmap made in the background renders the canvas again only where the
/// objects showing its file are: both copies of the file are in the part rendered again.
#[test]
fn a_ready_placed_document_bitmap_renders_only_its_part_of_the_canvas() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width":400,"height":300})).unwrap();
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    app.canvas.worker_started = true; // every frame on this thread, however long it takes
    // A document of one 60 × 30 artboard with a rectangle, placed linked twice.
    let mut src = Session::new();
    src.execute("file.new", &json!({"width": 60, "height": 30})).unwrap();
    src.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 60, "height": 30})).unwrap();
    let dir = std::env::temp_dir().join(format!("vectorcraft-placed-redraw-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("card.vectorcraft");
    std::fs::write(&path, vectorcraft_format::save_file(&src.active().unwrap().doc)).unwrap();
    let path = path.to_string_lossy().into_owned();
    let ids: Vec<u64> = [[60, 50], [140, 50]]
        .iter()
        .map(|at| {
            let r = app.session.execute("file.place", &json!({"path": path, "at": at})).unwrap();
            assert_eq!(r["linked"], true, "{r}");
            r["ids"][0].as_u64().unwrap()
        })
        .collect();
    app.run("shape.rectangle", json!({"x":250,"y":200,"width":40,"height":30})).unwrap();
    app.run("select.none", json!({})).unwrap();
    let ctx = egui::Context::default();
    // Where the canvas texture changed in a frame, and how much of it.
    let canvas = |app: &VectorcraftApp, mut delta: egui::TexturesDelta| {
        let tex = app.canvas.texture.as_ref().map(|t| t.id());
        let set = delta.set.iter().find(|(t, _)| tex == Some(**t)).and_then(|(_, d)| d.last()).map(|d| (d.pos, d.image.size()));
        delta.clear();
        set
    };
    let (_, delta) = frame_output(&mut app, &ctx);
    assert!(canvas(&app, delta).is_some_and(|(pos, _)| pos.is_none()), "the first frame renders all of it");
    let t = std::time::Instant::now();
    while vectorcraft_render::placed_document::busy() && t.elapsed().as_secs() < 20 {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let (_, delta) = frame_output(&mut app, &ctx);
    let (pos, size) = canvas(&app, delta).expect("the bitmap shows");
    let [px, py] = pos.expect("a part of the canvas");
    let key = app.canvas.key.clone().unwrap();
    let view = Affine::translate((f64::from(key.w) / 2.0, f64::from(key.h) / 2.0))
        * Affine::rotate(key.rot.to_radians())
        * Affine::scale(key.zoom * f64::from(key.ppp))
        * Affine::translate((-key.cx, -key.cy));
    let doc = &app.session.active().unwrap().doc;
    for id in ids {
        let b = doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap();
        let [x0, y0, x1, y1] = vectorcraft_render::view_pixels(b, view, key.w, key.h).unwrap();
        let inside = x0 as usize >= px && y0 as usize >= py && x1 as usize <= px + size[0] && y1 as usize <= py + size[1];
        assert!(inside, "object {id} at {:?} in the part at {:?} of size {size:?}", [x0, y0, x1, y1], [px, py]);
    }
    assert!(size[0] * size[1] * 2 <= key.w as usize * key.h as usize, "a part of it: {size:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// While an opacity mask is viewed, an edit renders all of the canvas: the mask view shows the
/// mask's coverage wherever its art reaches, past its object's bounds.
#[test]
fn an_edit_while_a_mask_is_viewed_renders_all_of_the_canvas() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width":400,"height":300})).unwrap();
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    app.canvas.worker_started = true; // every frame on this thread, however long it takes
    let rect =
        |app: &mut VectorcraftApp, x: f64| app.run("shape.rectangle", json!({"x": x, "y": 50, "width": 60, "height": 40})).unwrap()["id"].clone();
    let (obj, mask) = (rect(&mut app, 50.0), rect(&mut app, 70.0));
    app.run("select.set", json!({"ids": [obj, mask]})).unwrap();
    app.run("transparency.makeOpacityMask", json!({})).unwrap();
    app.run("transparency.viewOpacityMask", json!({})).unwrap();
    let ctx = egui::Context::default();
    let (_, mut delta) = frame_output(&mut app, &ctx);
    delta.clear();
    app.run("paint.setFill", json!({"color": "#ff0000", "ids": [obj]})).unwrap();
    let (_, mut delta) = frame_output(&mut app, &ctx);
    let tex = app.canvas.texture.as_ref().map(|t| t.id());
    let pos = delta.set.iter().find(|(t, _)| tex == Some(**t)).and_then(|(_, d)| d.last()).map(|d| d.pos);
    delta.clear();
    assert_eq!(pos, Some(None), "all of it");
}

/// #1278: a segment click marks the edge, not its endpoint anchors or neighboring handles.
#[test]
fn a_segment_pick_has_hollow_anchors_and_only_its_own_handles() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width":400,"height":300})).unwrap();
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    let id = app.run("shape.rectangle", json!({"x":50,"y":50,"width":200,"height":150})).unwrap()["id"].as_u64().unwrap();
    app.run("select.none", json!({})).unwrap();
    app.select_tool("directSelection");
    let view = app.view_info();
    for kind in [PointerKind::Down, PointerKind::Up] {
        app.session.pointer(&PointerEvent::new(kind, 150.0, 50.0), view).unwrap();
    }
    let before = app.session.doc().unwrap().doc.clone();
    let color = c32(before.layer_color(vectorcraft_doc::NodeId(id)));
    let selected = selected_anchor(color);
    let ctx = egui::Context::default();
    let painted = shapes(&mut app, &ctx);
    assert_eq!(painted.iter().filter(|s| matches!(s, Shape::Rect(r) if r.fill == selected)).count(), 0);
    assert_eq!(
        painted.iter().filter(|s| matches!(s, Shape::Path(p) if p.stroke.width == 2.0 && p.points.len() == 2)).count(),
        1,
        "only the chosen straight edge has emphasis"
    );
    assert!(!painted.iter().any(|s| matches!(s, Shape::Circle(_))), "segment picks have no corner widgets");
    assert_eq!(app.session.doc().unwrap().doc, before);
    app.run("select.anchors", json!({"id":id,"anchors":[[0,0],[0,1]]})).unwrap();
    let painted = shapes(&mut app, &ctx);
    assert_eq!(painted.iter().filter(|s| matches!(s, Shape::Rect(r) if r.fill == selected)).count(), 2);
    app.run(
        "path.create",
        json!({"anchors":[{"x":50,"y":260,"out":[100,220]},{"x":200,"y":260,"in":[150,220],"out":[250,300]},{"x":330,"y":260,"in":[280,300]}]}),
    )
    .unwrap();
    app.run("select.none", json!({})).unwrap();
    for kind in [PointerKind::Down, PointerKind::Up] {
        app.session.pointer(&PointerEvent::new(kind, 125.0, 230.0), view).unwrap();
    }
    let before = app.session.doc().unwrap().doc.clone();
    let painted = shapes(&mut app, &ctx);
    assert_eq!(
        painted.iter().filter(|s| matches!(s, Shape::LineSegment {stroke,..} if stroke.color == color)).count(),
        2,
        "only the chosen curve's two controls show"
    );
    app.ui.view.edges = false;
    let painted = shapes(&mut app, &ctx);
    assert!(!painted.iter().any(|s| matches!(s, Shape::LineSegment {stroke,..} if stroke.color == color)));
    assert_eq!(app.session.doc().unwrap().doc, before);
}

/// A group, a compound path or a clip group selected whole has every anchor selected, and Direct
/// Selection, the Pen and the anchor point tools draw its anchors filled, as they draw the anchors
/// of a path selected whole. They do the same after Group Selection clicks select a path and then its group.
/// Direction handles still show only on a path selected itself.
#[test]
fn the_anchors_of_a_container_selected_whole_draw_selected() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    let rect = |app: &mut VectorcraftApp, x: i32, y: i32| {
        app.run("shape.rectangle", json!({"x": x, "y": y, "width": 60, "height": 60})).unwrap()["id"].clone()
    };
    let in_group = rect(&mut app, 20, 20);
    let ellipse = app.run("shape.ellipse", json!({"x": 100, "y": 20, "width": 60, "height": 60})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [in_group, ellipse]})).unwrap();
    let group = app.run("object.group", json!({})).unwrap()["id"].clone();
    let (c1, c2) = (rect(&mut app, 200, 20), rect(&mut app, 220, 40));
    app.run("select.set", json!({"ids": [c1, c2]})).unwrap();
    let compound = app.run("object.compoundPath.make", json!({})).unwrap()["id"].clone();
    let hidden = app.run("shape.ellipse", json!({"x": 20, "y": 150, "width": 200, "height": 100})).unwrap()["id"].clone();
    let square = rect(&mut app, 60, 170);
    app.run("select.set", json!({"ids": [hidden, square]})).unwrap();
    let clip = app.run("object.clippingMask.make", json!({})).unwrap()["id"].clone();
    let lone = app.run("shape.ellipse", json!({"x": 300, "y": 150, "width": 60, "height": 60})).unwrap()["id"].clone();
    let color = c32(app.session.active().unwrap().doc.layer_color(vectorcraft_doc::NodeId(lone.as_u64().unwrap())));
    let ctx = egui::Context::default();
    // Filled and hollow anchor squares, and direction handle lines, drawn in the layer color.
    let looks = |app: &mut VectorcraftApp| {
        let painted = shapes(app, &ctx);
        let small = |r: &egui::epaint::RectShape| r.rect.width() < 12.0;
        let filled = painted.iter().filter(|s| matches!(s, Shape::Rect(r) if small(r) && r.fill == color)).count();
        let hollow = painted.iter().filter(|s| matches!(s, Shape::Rect(r) if small(r) && r.fill == Color32::WHITE)).count();
        let handles = painted.iter().filter(|s| matches!(s, Shape::LineSegment { stroke, .. } if stroke.color == color)).count();
        (filled, hollow, handles)
    };
    let mut wrong = vec![];
    for tool in ["directSelection", "pen", "addAnchor", "deleteAnchor", "anchorPoint"] {
        app.select_tool(tool);
        // What is selected, and what it shows: the lone ellipse selected whole is the reference
        // (four anchors with two handles each, and its center point, filled too).
        for (what, ids, want) in [
            ("a path", json!([lone]), (5, 0, 8)),
            ("a group", json!([group]), (8, 0, 0)),
            ("a compound path", json!([compound]), (8, 0, 0)),
            ("a clip group", json!([clip]), (4, 0, 0)),
        ] {
            app.run("select.set", json!({ "ids": ids })).unwrap();
            let got = looks(&mut app);
            if got != want {
                wrong.push(format!("{tool}, {what}: (filled, hollow, handles) {got:?}, want {want:?}"));
            }
        }
    }
    app.run("select.none", json!({})).unwrap();
    app.select_tool("groupSelection");
    let view = app.view_info();
    for _ in 0..2 {
        for kind in [PointerKind::Down, PointerKind::Up] {
            app.session.pointer(&PointerEvent::new(kind, 20.0, 50.0), view).unwrap();
        }
    }
    let id = |v: &serde_json::Value| vectorcraft_doc::NodeId(v.as_u64().unwrap());
    assert_eq!(app.session.active().unwrap().selection.objects, vec![id(&in_group), id(&group)]);
    app.select_tool("directSelection");
    // The rectangle's anchors and center point, then the group's eight anchors, four of them over the rectangle's.
    let got = looks(&mut app);
    if got != (13, 0, 0) {
        wrong.push(format!("a Group Selection click on a path, then on its group: {got:?}, want (13, 0, 0)"));
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// View › Pixel Preview: magnified, the art shows as the document pixels it rasterizes to (one
/// per point, hard-edged), not as smooth vectors; zoomed out or out of Pixel Preview the canvas
/// renders for the screen again.
#[test]
fn pixel_preview_shows_the_document_pixels() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    // Its 1 pt stroke covers a quarter of document pixel 99 and most of pixel 100.
    app.session.execute("shape.rectangle", &json!({"x": 100.25, "y": 50, "width": 200, "height": 200})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    let ctx = egui::Context::default();
    // The art's raster uploaded by one frame: (width, height, hard-edged, pixels).
    let upload = |app: &mut VectorcraftApp| {
        let (_, mut delta) = frame_output(app, &ctx);
        let art = app.canvas.texture.as_ref().unwrap().id();
        let up = delta.set.iter().find(|e| e.0.eq(&art)).and_then(|e| e.1.last()).map(|d| {
            let egui::ImageData::Color(img) = &d.image;
            (img.width(), img.height(), d.options.magnification == egui::TextureFilter::Nearest, img.pixels.clone())
        });
        delta.clear();
        up.expect("the art re-rendered")
    };
    let screen = upload(&mut app); // the first frame fits the view to the artboard
    assert!(screen.0 > 600 && !screen.2, "out of Pixel Preview: the screen's pixels, smoothed");
    app.ui.view.pixel_preview = true;
    let view = app.view_mut().unwrap();
    (view.zoom, view.center) = (8.0, Point::new(100.0, 150.0));
    let (w, h, hard, px) = upload(&mut app);
    assert!(hard, "hard-edged pixels");
    assert!(w <= screen.0 / 8 + 2 && h <= screen.1 / 8 + 2, "one pixel per point: {w} × {h}");
    let [x0, y0, x1, y1] = app.canvas.key.as_ref().and_then(|k| k.pixel).expect("the pixels rendered");
    assert_eq!(((x1 - x0) as usize, (y1 - y0) as usize), (w, h));
    // Each document pixel is one texel, anti-aliased by how much of it the art covers.
    let alpha = |x: i64| px[((150 - y0) * w as i64 + x - x0) as usize].a();
    assert!((40..=90).contains(&alpha(99)), "a quarter of pixel 99: {}", alpha(99));
    assert!(alpha(100) > 200, "most of pixel 100: {}", alpha(100));
    assert_eq!((alpha(98), alpha(101)), (0, 255));
    // The pixels are placed over the document rect they cover.
    let s = shapes(&mut app, &ctx);
    let art = app.canvas.texture.as_ref().unwrap().id();
    let Some(Shape::Mesh(mesh)) = s.iter().find(|s| matches!(s, Shape::Mesh(m) if m.texture_id == art)) else { panic!("the art") };
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let at = xf.to_screen(Point::new(x0 as f64, y0 as f64));
    assert!((mesh.vertices[0].pos - at).length() < 0.01, "{:?} at {at:?}", mesh.vertices[0].pos);
    // Zoomed out, a document pixel is no bigger than a screen pixel: rendered for the screen.
    app.view_mut().unwrap().zoom = 0.5;
    let (w, _, hard, _) = upload(&mut app);
    assert!(w > 600 && !hard, "zoomed out: the screen's pixels");
}

/// View › Hide Template (#2000): a template layer's art leaves the canvas, and Show Template
/// draws it again, dimmed as before; each re-renders at once.
#[test]
fn hide_template_leaves_template_layers_off_the_canvas() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    app.session.execute("shape.rectangle", &json!({"x": 100, "y": 50, "width": 200, "height": 200})).unwrap();
    let layer = app.session.active().unwrap().doc.layers[0].id.0;
    app.session.execute("layer.template", &json!({"ids": [layer], "on": true})).unwrap();
    let ctx = egui::Context::default();
    // How many pixels of the art's raster one frame uploads are painted, None when it was not
    // re-rendered.
    let painted = |app: &mut VectorcraftApp| -> Option<usize> {
        let (_, mut delta) = frame_output(app, &ctx);
        let art = app.canvas.texture.as_ref().unwrap().id();
        let n = delta.set.iter().find(|e| e.0.eq(&art)).and_then(|e| e.1.last()).map(|d| {
            let egui::ImageData::Color(img) = &d.image;
            img.pixels.iter().filter(|c| c.a() != 0).count()
        });
        delta.clear();
        n
    };
    let shown = painted(&mut app).expect("the first frame renders the art");
    assert!(shown > 1000, "the template layer's art is drawn: {shown} pixels");
    assert_eq!(app.run("view.template", json!({})).unwrap(), json!(false));
    assert_eq!(painted(&mut app), Some(0), "Hide Template re-renders without the template layer");
    assert_eq!(painted(&mut app), None, "nothing changed: no re-render");
    assert_eq!(app.run("view.template", json!({})).unwrap(), json!(true));
    assert_eq!(painted(&mut app), Some(shown), "Show Template draws it again as before");
}

/// General › Anti-aliased Artwork (#394): on (the default), the art's edges are smoothed on
/// screen; off, every pixel is either painted or not, and turning it either way re-renders.
#[test]
fn anti_aliased_artwork_preference_smooths_the_canvas() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    // Render on this thread: the worker's frame would land later than the test looks.
    app.canvas.worker_started = true;
    // A disc: its edge crosses pixels at every angle.
    app.session.execute("shape.ellipse", &json!({"x": 100, "y": 50, "width": 200, "height": 200})).unwrap();
    let ctx = egui::Context::default();
    // The alphas of the art's raster uploaded by one frame, None when it was not re-rendered.
    let alphas = |app: &mut VectorcraftApp| -> Option<Vec<u8>> {
        let (_, mut delta) = frame_output(app, &ctx);
        let art = app.canvas.texture.as_ref().unwrap().id();
        let alphas = delta.set.iter().find(|e| e.0.eq(&art)).and_then(|e| e.1.last()).map(|d| {
            let egui::ImageData::Color(img) = &d.image;
            img.pixels.iter().map(|c| c.a()).collect()
        });
        delta.clear();
        alphas
    };
    let partial = |a: &[u8]| a.iter().filter(|&&a| a != 0 && a != 255).count();
    let on = alphas(&mut app).expect("the first frame renders the art");
    assert!(partial(&on) > 50, "anti-aliased: edge pixels partly covered ({} of them)", partial(&on));
    assert!(alphas(&mut app).is_none(), "nothing changed: no re-render");
    app.session.execute("prefs.set", &json!({"key": "antiAliasedArtwork", "value": false})).unwrap();
    let off = alphas(&mut app).expect("the preference change re-renders");
    assert_eq!(partial(&off), 0, "hard edges: every pixel painted or not");
    assert!(off.iter().filter(|&&a| a == 255).count() > 1000, "the disc is still painted");
    app.session.execute("prefs.set", &json!({"key": "antiAliasedArtwork", "value": true})).unwrap();
    assert!(partial(&alphas(&mut app).expect("re-rendered")) > 50, "smooth again");
}

/// Type › Highlight Substituted Fonts: on (the default), type in a missing font has pink
/// behind it on the canvas; off, it does not, and turning it either way re-renders.
#[test]
fn highlight_substituted_fonts_preference_reaches_the_canvas() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    app.session.execute("text.create", &json!({"x": 20, "y": 120, "text": "Hamburg", "size": 60, "font": "Missing Family"})).unwrap();
    let ctx = egui::Context::default();
    // How many pink pixels the art's raster uploaded by one frame has, None when it was not
    // re-rendered.
    let pink = |app: &mut VectorcraftApp| -> Option<usize> {
        let (_, mut delta) = frame_output(app, &ctx);
        let art = app.canvas.texture.as_ref().unwrap().id();
        let n = delta.set.iter().find(|e| e.0.eq(&art)).and_then(|e| e.1.last()).map(|d| {
            let egui::ImageData::Color(img) = &d.image;
            img.pixels.iter().filter(|c| c.a() != 0 && c.r() > c.g().saturating_add(30) && c.b() > c.g()).count()
        });
        delta.clear();
        n
    };
    let on = pink(&mut app).expect("the first frame renders the art");
    assert!(on > 100, "on by default: the text has pink behind it ({on} pixels)");
    assert!(app.canvas.shown.as_ref().is_some_and(|(_, o)| o.highlight_substituted_fonts));
    app.session.execute("prefs.set", &json!({"key": "highlightSubstitutedFonts", "value": false})).unwrap();
    assert_eq!(pink(&mut app), Some(0), "the preference off re-renders without the highlight");
    assert!(app.canvas.key.as_ref().is_some_and(|k| !k.highlight_substituted_fonts));
    app.session.execute("prefs.set", &json!({"key": "highlightSubstitutedFonts", "value": true})).unwrap();
    assert_eq!(pink(&mut app), Some(on), "on again, as before");
}

/// Images whose pixels can't be decoded draw as crossed boxes on the canvas, and the status
/// bar reports each of them once: two such images give one message, and later frames that
/// draw them again give none.
#[test]
fn undecodable_images_are_marked_and_reported_once() {
    use vectorcraft_doc::{ImageBlob, ImageObject, NodeKind};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    // PNGs of noisy pixels cut off halfway: their headers read, their pixels don't decode.
    let cut = |w: u32| {
        let img = image::RgbaImage::from_fn(w, 41, |x, y| image::Rgba([(x * 37 + y * 101) as u8, (x * y) as u8, (x ^ y) as u8, 255]));
        let mut png = vec![];
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        png.truncate(png.len() / 2);
        png
    };
    {
        let st = app.session.doc_mut().unwrap();
        let d = std::sync::Arc::make_mut(&mut st.doc);
        for (i, w) in [53u32, 59].into_iter().enumerate() {
            let key = format!("cut{i}");
            d.images.insert(key.clone(), ImageBlob::new("image/png", cut(w)));
            let im = ImageObject {
                key,
                width: w,
                height: 41,
                xf: Affine::translate((40.0 + 150.0 * i as f64, 100.0)),
                link: None,
                placement: Default::default(),
                pixels: None,
            };
            let (id, layer) = (d.alloc_id(), d.layers[0].id);
            d.insert(Some(layer), 0, Node::new(id, NodeKind::Image(Box::new(im)))).unwrap();
        }
        st.revision += 1;
    }
    let ctx = egui::Context::default();
    app.status("");
    frame(&mut app, &ctx, vec![]);
    assert!(app.canvas.shown.as_ref().is_some_and(|(_, o)| o.mark_undecoded_images), "the canvas marks them");
    assert_eq!(app.ui.status, "2 image(s) couldn't be decoded and show as crossed boxes");
    app.status("");
    app.session.doc_mut().unwrap().revision += 1;
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.ui.status, "", "said once");
}

/// A drag of an object with raster effects (#1257): the drag's frames render while
/// interacting and draw the object's drop shadow and filtered content from the rasters of the
/// frame before; the frame after the drag renders without `interacting`, on the render worker,
/// and draws what a fresh renderer draws.
#[test]
fn dragging_an_object_with_raster_effects_draws_its_rasters_moved() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    let id = app.run("shape.rectangle", json!({"x": 100, "y": 80, "width": 120, "height": 90})).unwrap()["id"].clone();
    for effect in ["stylize.dropShadow", "texture.craquelure"] {
        app.run("effect.apply", json!({"effect": effect, "ids": [id]})).unwrap();
    }
    let ctx = egui::Context::default();
    // The art's raster uploaded by one frame, None when it was not re-rendered.
    let upload = |app: &mut VectorcraftApp| -> Option<Vec<Color32>> {
        let (_, mut delta) = frame_output(app, &ctx);
        let art = app.canvas.texture.as_ref().unwrap().id();
        let px = delta.set.iter().find(|e| e.0.eq(&art)).and_then(|e| e.1.last()).map(|d| {
            let egui::ImageData::Color(img) = &d.image;
            img.pixels.clone()
        });
        delta.clear();
        px
    };
    upload(&mut app).expect("the first frame renders the art");
    app.select_tool("selection");
    let view = app.view_info();
    app.session.pointer(&PointerEvent::new(PointerKind::Down, 160.0, 125.0), view).unwrap();
    for step in [1.0, 2.0, 3.0] {
        app.session.pointer(&PointerEvent::new(PointerKind::Drag, 160.0 + 7.3 * step, 125.0 + 2.1 * step), view).unwrap();
        upload(&mut app).expect("each frame of the drag renders");
        assert!(app.canvas.key.as_ref().is_some_and(|k| k.interacting), "step {step}");
        assert_eq!(app.canvas.renderer.stats.moved, 2, "the shadow and the filtered content, step {step}");
        assert!(app.canvas.moved);
    }
    // The frame after the drag runs the effects again: on the worker, while the canvas shows
    // the drag's last frame.
    app.canvas.worker = crate::render_worker::Worker::spawn(ctx.clone());
    app.session.pointer(&PointerEvent::new(PointerKind::Up, 181.9, 131.3), view).unwrap();
    assert!(upload(&mut app).is_none(), "not rendered on this thread");
    assert!(app.canvas.worker.as_ref().and_then(|w| w.pending.as_ref()).is_some_and(|k| !k.interacting), "sent to the worker");
    let until = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let mut after = None;
    while after.is_none() && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(5));
        after = upload(&mut app);
    }
    let after = after.expect("the worker's frame arrives");
    assert!(app.canvas.key.as_ref().is_some_and(|k| !k.interacting));
    assert!(!app.canvas.moved);
    app.canvas.worker = None;
    app.canvas.renderer = vectorcraft_render::Renderer::new();
    app.canvas.key = None;
    assert!(upload(&mut app) == Some(after), "the frame after the drag is what a fresh renderer draws");
}

/// A tool's drag on the canvas renders with `dragging` (#2558). At 3200% a rectangle with
/// Texturizer covers more than the raster budget, so no raster is kept for it, and each frame
/// of a Selection tool drag draws its filtered content at the drag's lower resolution. The
/// frame after the drag draws what a fresh renderer draws. A dialog's preview, an interaction
/// without a drag, renders without `dragging` and draws the same.
#[test]
fn a_drag_on_the_canvas_draws_raster_effects_at_a_lower_resolution() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    let id = app.run("shape.rectangle", json!({"x": 100, "y": 80, "width": 200, "height": 150})).unwrap()["id"].clone();
    app.run("effect.apply", json!({"effect": "texture.texturizer", "ids": [id]})).unwrap();
    app.select_tool("selection");
    let ctx = egui::Context::default();
    // One headless frame on a 1920 × 1200 window (a view larger than the drag's budget) with
    // `events` → the art's raster it uploaded, None when it was not re-rendered.
    let run = |app: &mut VectorcraftApp, events: Vec<egui::Event>| -> Option<Vec<Color32>> {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1920.0, 1200.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        let art = app.canvas.texture.as_ref().map(|t| t.id());
        let px = out.textures_delta.set.iter().find(|e| Some(*e.0) == art).and_then(|e| e.1.last()).map(|d| {
            let egui::ImageData::Color(img) = &d.image;
            img.pixels.clone()
        });
        out.textures_delta.clear();
        px
    };
    run(&mut app, vec![]);
    if let Some(vm) = app.view_mut() {
        (vm.zoom, vm.center) = (32.0, Point::new(200.0, 155.0));
    }
    let still = run(&mut app, vec![]).expect("the zoomed view renders");
    assert_eq!(app.canvas.renderer.stats.previewed, 0, "a still frame");
    let at = app.canvas_rect.unwrap().center();
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    run(&mut app, vec![egui::Event::PointerMoved(at)]);
    run(&mut app, vec![button(at, true)]);
    for step in [1.0, 2.0] {
        let drawn = run(&mut app, vec![egui::Event::PointerMoved(at + vec2(13.0, 7.0) * step)]).expect("each frame of the drag renders");
        assert!(app.canvas.key.as_ref().is_some_and(|k| k.interacting && k.dragging), "step {step}");
        assert_eq!(app.canvas.renderer.stats.previewed, 1, "the filtered content, step {step}");
        assert!(app.canvas.moved);
        assert!(drawn != still, "the rectangle moved, step {step}");
    }
    let after = run(&mut app, vec![button(at + vec2(26.0, 14.0), false)]).expect("the frame after the drag renders");
    assert!(app.canvas.key.as_ref().is_some_and(|k| !k.interacting && !k.dragging));
    assert_eq!(app.canvas.renderer.stats.previewed, 0, "the frame after the drag");
    assert!(!app.canvas.moved);
    app.canvas.renderer = vectorcraft_render::Renderer::new();
    app.canvas.key = None;
    assert!(run(&mut app, vec![]) == Some(after.clone()), "the frame after the drag is what a fresh renderer draws");
    // A dialog's preview: an interaction, and no drag.
    app.session.begin_interaction("Texturizer").unwrap();
    assert!(run(&mut app, vec![]) == Some(after), "a dialog's preview draws the effect at full resolution");
    assert!(app.canvas.key.as_ref().is_some_and(|k| k.interacting && !k.dragging));
    assert_eq!(app.canvas.renderer.stats.previewed, 0);
}

/// Dragging a small square over an A4 page filled by a rectangle blurred by 500 pt, at 200%:
/// the blur reaches far past the raster budget, so the still frame keeps the backdrop's raster
/// cropped to the view, and each frame of a Selection tool drag draws it from there instead of
/// previewing it at the drag's lower resolution: on this thread, and on the worker, where the
/// frames of a heavy document render.
#[test]
fn a_drag_over_a_wide_blur_draws_the_blur_from_the_still_frame() {
    for on_worker in [false, true] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 595, "height": 842})).unwrap();
        app.canvas.worker_started = true; // no worker unless `on_worker`
        let back = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 595, "height": 842})).unwrap()["id"].clone();
        app.run("paint.setFill", json!({"ids": [back], "color": "#1a4ce6"})).unwrap();
        app.run("effect.apply", json!({"effect": "blur.gaussian", "params": {"radius": 500}, "ids": [back]})).unwrap();
        let square = app.run("shape.rectangle", json!({"x": 260, "y": 380, "width": 60, "height": 60})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [square]})).unwrap();
        app.select_tool("selection");
        let ctx = egui::Context::default();
        // One headless frame with `events`, then, on the worker, frames until the frame it renders
        // for them is shown.
        let run = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let frame = |app: &mut VectorcraftApp, events| {
                let raw =
                    egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 870.0))), events, ..Default::default() };
                let mut out = ctx.run_ui(raw, |ui| show(app, ui));
                out.textures_delta.clear();
            };
            frame(app, events);
            let until = std::time::Instant::now() + std::time::Duration::from_secs(60);
            while app.canvas.worker.as_ref().is_some_and(|w| w.pending.is_some()) && std::time::Instant::now() < until {
                std::thread::sleep(std::time::Duration::from_millis(5));
                frame(app, vec![]);
            }
        };
        let moved = |app: &VectorcraftApp| if on_worker { app.canvas.moved } else { app.canvas.renderer.stats.moved > 0 };
        run(&mut app, vec![]);
        if let Some(vm) = app.view_mut() {
            (vm.zoom, vm.center) = (2.0, Point::new(290.0, 410.0));
        }
        if on_worker {
            app.canvas.worker = crate::render_worker::Worker::spawn(ctx.clone());
            app.canvas.last_ms = 50.0;
        }
        run(&mut app, vec![]);
        assert_eq!(app.canvas.from_worker, on_worker);
        assert!(!moved(&app), "a still frame, on the worker: {on_worker}");
        let at = app.canvas_rect.unwrap().center();
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        run(&mut app, vec![egui::Event::PointerMoved(at)]);
        run(&mut app, vec![button(at, true)]);
        for step in [1.0, 2.0, 3.0] {
            run(&mut app, vec![egui::Event::PointerMoved(at + vec2(13.0, 7.0) * step)]);
            assert!(app.canvas.key.as_ref().is_some_and(|k| k.interacting && k.dragging), "step {step}, on the worker: {on_worker}");
            assert_eq!(app.canvas.from_worker, on_worker, "step {step}");
            assert!(
                !moved(&app),
                "the blur drawn from the still frame's raster, step {step}, on the worker: {on_worker} ({} ms)",
                app.perf.render_ms
            );
        }
        run(&mut app, vec![button(at + vec2(39.0, 21.0), false)]);
        let info = app.run("document.inspect", json!({})).unwrap();
        let children = info["layers"][0]["children"].as_array().unwrap().clone();
        let x0 = |id: &serde_json::Value| children.iter().find(|c| c["id"] == *id).unwrap()["bounds"]["x"].as_f64().unwrap();
        assert!(x0(&square) > 270.0 && x0(&back) == 0.0, "the square moved, the backdrop did not");
        app.canvas.worker = None;
    }
}

/// An interaction's frames stay on the worker once it drew the frame shown, however fast that
/// frame was: the worker's renderer keeps the effect rasters that made it fast, and this
/// thread's renderer, without them, would draw the next frame slowly. Without a frame from the
/// worker, a fast frame renders on this thread as before.
#[test]
fn an_interaction_stays_on_the_worker_that_drew_the_frame_shown() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 100, "y": 50, "width": 200, "height": 200})).unwrap();
    app.canvas.worker_started = true; // the first frame renders on this thread
    let ctx = egui::Context::default();
    shapes(&mut app, &ctx);
    for from_worker in [true, false] {
        app.canvas.worker = crate::render_worker::Worker::spawn(ctx.clone());
        (app.canvas.from_worker, app.canvas.last_ms) = (from_worker, 1.0);
        app.session.begin_interaction("Preview").unwrap();
        shapes(&mut app, &ctx);
        let pending = app.canvas.worker.as_ref().and_then(|w| w.pending.as_ref()).is_some_and(|k| k.interacting);
        assert_eq!(pending, from_worker, "sent to the worker: {from_worker}");
        if !from_worker {
            assert!(app.canvas.key.as_ref().is_some_and(|k| k.interacting), "rendered on this thread");
        }
        app.session.cancel_interaction().unwrap();
        app.canvas.worker = None;
        shapes(&mut app, &ctx);
    }
}

/// The frame after an interaction renders without `interacting` also when the interaction
/// leaves the document as it was (#1257): here a drag back to where it started.
#[test]
fn a_drag_back_to_its_start_renders_again_after_it() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    let id = app.run("shape.rectangle", json!({"x": 100, "y": 80, "width": 120, "height": 90})).unwrap()["id"].clone();
    app.run("effect.apply", json!({"effect": "texture.craquelure", "ids": [id]})).unwrap();
    let ctx = egui::Context::default();
    shapes(&mut app, &ctx);
    app.select_tool("selection");
    let view = app.view_info();
    app.session.pointer(&PointerEvent::new(PointerKind::Down, 160.0, 125.0), view).unwrap();
    for (x, y) in [(173.3, 129.1), (160.0, 125.0)] {
        app.session.pointer(&PointerEvent::new(PointerKind::Drag, x, y), view).unwrap();
        shapes(&mut app, &ctx);
        assert_eq!(app.canvas.renderer.stats.moved, 1, "the filtered content at ({x}, {y})");
    }
    let revision = app.session.active().unwrap().revision;
    app.session.pointer(&PointerEvent::new(PointerKind::Up, 160.0, 125.0), view).unwrap();
    assert_eq!(app.session.active().unwrap().revision, revision, "the drag left the document as it was");
    shapes(&mut app, &ctx);
    assert!(app.canvas.key.as_ref().is_some_and(|k| !k.interacting), "rendered again");
    assert_eq!(app.canvas.renderer.stats.moved, 0);
    assert!(!app.canvas.moved);
}

/// Cache metadata, worker requests and view transforms use the renderer's bounded raster size.
/// Keep the worker controlled so huge virtual viewports never allocate huge test images.
#[test]
fn canvas_render_jobs_use_renderer_limited_dimensions() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.canvas.worker_started = true;
    app.ui.view.rulers = false;
    let frame = |app: &mut VectorcraftApp, size, ppp| {
        let mut input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(pos2(0.0, 0.0), size)), ..Default::default() };
        input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().native_pixels_per_point = Some(ppp);
        let mut output = ctx.run_ui(input, |ui| show(app, ui));
        output.textures_delta.clear();
    };
    frame(&mut app, vec2(400.0, 300.0), 1.0);
    let (worker, jobs, _completed) = crate::render_worker::Worker::test_channel();
    app.canvas.worker = Some(worker);
    app.canvas.last_ms = 50.0;
    for (size, ppp, wide) in [(vec2(70000.25, 32.0), 1.0, true), (vec2(32.0, 70000.25), 1.0, false), (vec2(40000.25, 32.0), 2.0, true)] {
        frame(&mut app, size, ppp);
        let job = jobs.try_recv().expect("oversized viewport queued on controlled worker");
        assert_eq!((job.key.w, job.key.h), (job.w, job.h));
        assert_eq!(if wide { job.w } else { job.h }, u32::from(u16::MAX));
        assert!((1..=u32::from(u16::MAX)).contains(&job.w));
        assert!((1..=u32::from(u16::MAX)).contains(&job.h));
        let center = job.view * Point::new(job.key.cx, job.key.cy);
        assert!((center.x - f64::from(job.w) / 2.0).abs() < 0.001);
        assert!((center.y - f64::from(job.h) / 2.0).abs() < 0.001);
    }
}

/// A cached texture keeps the renderer's document mapping when the viewport changes. The
/// controlled worker cannot finish early, even if egui repeats a layout pass.
#[test]
fn cached_frames_keep_their_scale_during_resize_rotation_and_dpi_changes() {
    for (old_rot, new_rot, old_ppp, new_ppp, pixel, resize_only) in [
        // Isolate the original panel/window resize: zoom, pan, rotation and DPI stay fixed.
        (0.0, 0.0, 1.0, 1.0, false, true),
        (30.0, 30.0, 1.25, 1.25, false, true),
        (0.0, 0.0, 1.0, 1.0, false, false),
        (30.0, 30.0, 1.25, 1.25, false, false),
        (30.0, 75.0, 1.25, 1.25, false, false),
        (30.0, 75.0, 1.25, 2.0, false, false),
        (30.0, 30.0, 2.0, 1.25, false, false),
        (30.0, 75.0, 1.25, 2.0, true, false),
    ] {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.canvas.worker_started = true;
        app.ui.view.pixel_preview = pixel;
        let frame = |app: &mut VectorcraftApp, size: egui::Vec2, ppp: f32| {
            let mut input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(pos2(31.25, 19.5), size)), ..Default::default() };
            // Monitor DPI is viewport input, not egui's UI zoom (which rescales the last
            // screen rect and would override the simultaneous resize under test).
            input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().native_pixels_per_point = Some(ppp);
            let mut out = ctx.run_ui(input, |ui| show(app, ui));
            assert!((out.pixels_per_point - ppp).abs() < 0.001);
            out.textures_delta.clear();
            let tex = app.canvas.texture.as_ref().expect("rendered canvas texture").id();
            out.shapes
                .into_iter()
                .find_map(|s| match s.shape {
                    Shape::Mesh(m) if m.texture_id == tex => Some(m),
                    _ => None,
                })
                .expect("the canvas texture is drawn")
        };
        frame(&mut app, vec2(800.25, 600.35), old_ppp);
        {
            let v = app.view_mut().unwrap();
            v.rotation = old_rot;
            v.zoom = 2.5;
            v.center = Point::new(210.0, 160.0);
        }
        frame(&mut app, vec2(800.25, 600.35), old_ppp);
        let old = app.canvas.key.clone().unwrap();
        let (worker, jobs, completed) = crate::render_worker::Worker::test_channel();
        app.canvas.worker = Some(worker);
        app.canvas.last_ms = 50.0;
        {
            let v = app.view_mut().unwrap();
            v.rotation = new_rot;
            if !resize_only {
                v.zoom = 3.25;
                v.center = Point::new(225.0, 170.0);
            }
        }
        let size = vec2(602.5, 720.1);
        let mesh = frame(&mut app, size, new_ppp);
        assert_eq!(app.canvas.key.as_ref(), Some(&old), "the cached frame is still displayed");
        let job = jobs.try_recv().expect("the replacement frame was submitted");
        assert_eq!(job.key.ppp, new_ppp);
        assert_eq!(job.key.rot, new_rot);
        if resize_only {
            assert_eq!((job.key.zoom, job.key.cx, job.key.cy, job.key.rot, job.key.ppp), (old.zoom, old.cx, old.cy, old.rot, old.ppp));
        }
        assert_ne!((job.key.w, job.key.h), (old.w, old.h));
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let render_view = Affine::translate((old.w as f64 / 2.0, old.h as f64 / 2.0))
            * Affine::rotate(old.rot.to_radians())
            * Affine::scale(old.zoom * old.ppp as f64)
            * Affine::translate((-old.cx, -old.cy));
        assert_eq!(mesh.vertices.len(), 4);
        for vertex in &mesh.vertices {
            assert!([pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)].contains(&vertex.uv));
            let doc = if let Some([x0, y0, x1, y1]) = old.pixel {
                Point::new(x0 as f64 + vertex.uv.x as f64 * (x1 - x0) as f64, y0 as f64 + vertex.uv.y as f64 * (y1 - y0) as f64)
            } else {
                render_view.inverse() * Point::new(vertex.uv.x as f64 * old.w as f64, vertex.uv.y as f64 * old.h as f64)
            };
            assert!(
                (vertex.pos - xf.to_screen(doc)).length() < 0.002,
                "{old_rot}° -> {new_rot}°, DPI {old_ppp} -> {new_ppp}, pixel={pixel}: {vertex:?}"
            );
        }
        // Deliver the actual replacement and verify that it replaces the cached key.
        let img = app.canvas.renderer.render(&job.doc, job.w, job.h, job.view, &job.opts);
        let expected = job.key.clone();
        completed.send(crate::render_worker::Done { key: job.key, doc: job.doc, opts: job.opts, img, ms: 1.0, moved: 0 }).unwrap();
        frame(&mut app, size, new_ppp);
        assert_eq!(app.canvas.key.as_ref(), Some(&expected), "the worker's frame arrived");
    }
}

/// When the view turns while the worker renders the frame at the new angle (Reset Rotate View on
/// a heavy document), the canvas draws the last frame turned until that frame arrives: each
/// corner goes where the new view puts the document point that the corner showed. The new frame
/// is then drawn unturned. In Pixel Preview the last frame's pixels stay over the document rect
/// they cover.
#[test]
fn a_turned_view_draws_the_last_frame_turned_until_its_own_arrives() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 100, "y": 50, "width": 200, "height": 200})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    let ctx = egui::Context::default();
    shapes(&mut app, &ctx); // the first frame fits the view to the artboard
    app.view_mut().unwrap().rotation = 30.0;
    shapes(&mut app, &ctx);
    let rect = app.canvas_rect.unwrap();
    let old = Xf::new(rect, app.view().unwrap());
    // `art` runs one frame and finds the mesh that draws the art's texture.
    let art = |app: &mut VectorcraftApp| {
        let s = shapes(app, &ctx);
        let tex = app.canvas.texture.as_ref().unwrap().id();
        s.into_iter().find_map(|s| match s {
            Shape::Mesh(m) if m.texture_id == tex => Some(m),
            _ => None,
        })
    };
    // With a worker and a last frame that took 50 ms, the next frame renders on the worker.
    app.canvas.worker = crate::render_worker::Worker::spawn(ctx.clone());
    app.canvas.last_ms = 50.0;
    app.run("view.rotateReset", json!({})).unwrap();
    let mesh = art(&mut app).expect("the last frame, turned");
    assert!(app.canvas.worker.as_ref().and_then(|w| w.pending.as_ref()).is_some_and(|k| k.rot == 0.0), "sent to the worker");
    let new = Xf::new(rect, app.view().unwrap());
    let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
    assert_eq!(mesh.vertices.len(), 4);
    for ((v, corner), uv) in mesh.vertices.iter().zip(corners).zip([pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)]) {
        let at = new.to_screen(old.to_doc(corner));
        assert!((v.pos - at).length() < 0.01 && v.uv == uv, "{v:?}: texture corner {uv:?} at {at:?}");
    }
    // The point where the last frame drew the rectangle's top-left corner lands where the new view puts that corner.
    let p = Point::new(100.0, 50.0);
    let uv = (old.to_screen(p) - rect.min) / rect.size();
    let [a, b, _, d] = [0, 1, 2, 3].map(|i| mesh.vertices[i].pos);
    assert!((a + (b - a) * uv.x + (d - a) * uv.y - new.to_screen(p)).length() < 0.01, "the rectangle's corner");
    // The worker's frame at 0° then covers the canvas unturned.
    let until = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while app.canvas.key.as_ref().is_some_and(|k| k.rot != 0.0) && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(5));
        shapes(&mut app, &ctx);
    }
    assert!(app.canvas.key.as_ref().is_some_and(|k| k.rot == 0.0), "the worker's frame arrives");
    let b = art(&mut app).expect("the worker's frame").calc_bounds();
    assert!((b.min - rect.min).length() < 0.01 && (b.max - rect.max).length() < 0.01, "{b:?} over {rect:?}");
    // In Pixel Preview the last frame's pixels are drawn over the document rect they cover.
    app.canvas.worker = None;
    app.ui.view.pixel_preview = true;
    app.view_mut().unwrap().zoom = 8.0;
    shapes(&mut app, &ctx);
    let [x0, y0, ..] = app.canvas.key.as_ref().and_then(|k| k.pixel).expect("the pixels rendered");
    app.canvas.worker = crate::render_worker::Worker::spawn(ctx.clone());
    app.canvas.last_ms = 50.0;
    app.view_mut().unwrap().rotation = 30.0;
    let mesh = art(&mut app).expect("the last frame's pixels");
    let at = Xf::new(rect, app.view().unwrap()).to_screen(Point::new(x0 as f64, y0 as f64));
    assert!((mesh.vertices[0].pos - at).length() < 0.01, "{:?} at {at:?}", mesh.vertices[0].pos);
    app.canvas.worker = None;
}

/// While the worker renders the frame for a panned view, the fill worker renders a
/// low-resolution frame, and the part of the canvas the last frame no longer covers shows the
/// art from it, drawn only there. When the worker's frame arrives, it covers the canvas and the
/// fill is dropped.
#[test]
fn a_pan_fills_the_area_the_last_frame_leaves_until_the_worker_frame_arrives() {
    // The canvas key holds the global placed-document bitmap clock, which tests on other threads
    // move: a frame whose key changed by more than the view drops the fill. An attempt that fails
    // while the clock moved runs again; one that fails while it stood still is a real failure.
    for _ in 0..20 {
        let clock = vectorcraft_render::placed_document::generation();
        match std::panic::catch_unwind(pan_fills_the_area_the_last_frame_leaves) {
            Ok(()) => return,
            Err(_) if vectorcraft_render::placed_document::generation() != clock => {}
            Err(e) => std::panic::resume_unwind(e),
        }
    }
    pan_fills_the_area_the_last_frame_leaves();
}

fn pan_fills_the_area_the_last_frame_leaves() {
    let ctx = egui::Context::default();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 300, "y": 100, "width": 80, "height": 100})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    app.ui.view.rulers = false;
    app.ui.screen_mode = 2; // no scroll bars: the canvas is the window
    // One frame → the meshes drawn and the fill's pixels, when this frame uploaded them.
    let frame = |app: &mut VectorcraftApp| {
        let (shapes, mut delta) = frame_output(app, &ctx);
        let fill = app.canvas.fill.as_ref().map(|f| f.tex.id());
        let pixels = delta.set.iter().find(|(id, _)| Some(**id) == fill).and_then(|(_, d)| d.last()).map(|d| match &d.image {
            egui::ImageData::Color(img) => img.clone(),
        });
        delta.clear();
        let meshes: Vec<egui::Mesh> = shapes
            .into_iter()
            .filter_map(|s| match s {
                Shape::Mesh(m) => Some(std::sync::Arc::unwrap_or_clone(m)),
                _ => None,
            })
            .collect();
        (meshes, pixels)
    };
    frame(&mut app);
    {
        let v = app.view_mut().unwrap();
        v.zoom = 4.0;
        v.center = Point::new(100.0, 150.0);
    }
    frame(&mut app); // the art from about x 0 to 200, without the rectangle
    let old = app.canvas.key.clone().unwrap();
    let before = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    assert!(app.canvas.fill.is_none());
    let (worker, jobs, completed) = crate::render_worker::Worker::test_channel();
    app.canvas.worker = Some(worker);
    let (fill_worker, fill_jobs, fill_completed) = crate::render_worker::Worker::test_channel();
    app.canvas.fill_worker = Some(fill_worker);
    app.canvas.last_ms = 50.0;
    // Pan right by 150 pt: the last frame covers the canvas's left quarter, and the rectangle
    // comes into its right quarter.
    app.view_mut().unwrap().center = Point::new(250.0, 150.0);
    frame(&mut app);
    assert_eq!(app.canvas.key.as_ref(), Some(&old), "the last frame is still shown");
    let job = jobs.try_recv().expect("the frame for the new view is on the worker");
    let fill_job = fill_jobs.try_recv().expect("the fill is on the fill worker");
    let (fw, fh, _) = fill_view(&job.key);
    assert!(fill_job.opts.kept_images_only && (fill_job.w, fill_job.h) == (fw, fh), "{:?}", (fill_job.w, fill_job.h));
    assert!(app.canvas.fill.is_none(), "nothing to draw until the fill arrives");
    // The fill arrives; the next frame draws it, and asks for no other fill.
    let img = app.canvas.renderer.render(&fill_job.doc, fill_job.w, fill_job.h, fill_job.view, &fill_job.opts);
    let (doc, opts) = (fill_job.doc, fill_job.opts);
    fill_completed.send(crate::render_worker::Done { key: fill_job.key, doc, opts, img, ms: 5.0, moved: 0 }).unwrap();
    let (meshes, pixels) = frame(&mut app);
    assert!(fill_jobs.try_recv().is_err(), "the fill covers the canvas");
    assert_eq!(app.canvas.key.as_ref(), Some(&old), "the last frame is still shown");
    let fill = app.canvas.fill.as_ref().expect("the fill is kept");
    let mesh = meshes.iter().find(|m| m.texture_id == fill.tex.id()).expect("the fill is drawn");
    let rect = app.canvas_rect.unwrap();
    let xf = Xf::new(rect, app.view().unwrap());
    let edge = xf.to_screen(before.to_doc(rect.right_top())).x; // the last frame's right edge
    assert!(edge > rect.left() + 100.0 && edge < xf.to_screen(Point::new(300.0, 0.0)).x, "{edge} in {rect:?}");
    assert!(
        mesh.vertices.iter().all(|v| v.pos.x >= edge - 1e-3 && rect.expand(1e-3).contains(v.pos)),
        "only right of the last frame: {:?}",
        mesh.vertices
    );
    let area: f32 = mesh.indices.as_chunks::<3>().0.iter().map(|t| polygon_area(&t.map(|i| mesh.vertices[i as usize].pos))).sum();
    let uncovered = (rect.right() - edge) * rect.height();
    assert!((area - uncovered).abs() < 1.0, "the whole uncovered area, {uncovered} square points: {area}");
    // The fill's pixel under a point of the canvas, from the uv the mesh gives that point.
    let img = pixels.expect("the fill's pixels were uploaded");
    let to_uv = Affine::scale_non_uniform(1.0 / fill.size.0 as f64, 1.0 / fill.size.1 as f64) * fill.view * xf.affine().inverse();
    let alpha = |p: Pos2| {
        let uv = to_uv * Point::new(p.x as f64, p.y as f64);
        img.pixels[(uv.y * img.size[1] as f64) as usize * img.size[0] + (uv.x * img.size[0] as f64) as usize].a()
    };
    assert!(alpha(xf.to_screen(Point::new(340.0, 150.0))) > 0, "the rectangle, in the uncovered area");
    assert_eq!(alpha(xf.to_screen(Point::new(250.0, 150.0))), 0, "no art there");
    for v in &mesh.vertices {
        assert!((to_uv * Point::new(v.pos.x as f64, v.pos.y as f64) - Point::new(v.uv.x as f64, v.uv.y as f64)).hypot() < 1e-4);
    }
    // Panned until the fill reaches only the canvas's left half: it is drawn only there, and the
    // fill for the new view is asked for.
    let fill_tex = fill.tex.id();
    let reach = fill.view.inverse() * Point::new(fill.size.0 as f64, 0.0); // the fill's right edge
    app.view_mut().unwrap().center = Point::new(reach.x, 150.0);
    let (meshes, _) = frame(&mut app);
    let right = Xf::new(rect, app.view().unwrap()).to_screen(reach).x;
    assert!((right - rect.center().x).abs() < 1.0, "{right} in {rect:?}");
    let mesh = meshes.iter().find(|m| m.texture_id == fill_tex).expect("the fill is drawn where it reaches");
    assert!(mesh.vertices.iter().all(|v| v.pos.x <= right + 1e-3), "only where the fill reaches: {:?}", mesh.vertices);
    let next = fill_jobs.try_recv().expect("the next fill");
    assert_eq!(Some(&next.key), app.canvas.worker.as_ref().unwrap().pending.as_ref());
    app.view_mut().unwrap().center = Point::new(250.0, 150.0);
    // That fill finishes after the worker's frame: it is taken and not kept.
    let img = app.canvas.renderer.render(&next.doc, next.w, next.h, next.view, &next.opts);
    fill_completed.send(crate::render_worker::Done { key: next.key, doc: next.doc, opts: next.opts, img, ms: 5.0, moved: 0 }).unwrap();
    // The worker's frame arrives: it covers the canvas, and the fill is no longer drawn or kept.
    let img = app.canvas.renderer.render(&job.doc, job.w, job.h, job.view, &job.opts);
    let expected = job.key.clone();
    completed.send(crate::render_worker::Done { key: job.key, doc: job.doc, opts: job.opts, img, ms: 50.0, moved: 0 }).unwrap();
    let (meshes, _) = frame(&mut app);
    assert_eq!(app.canvas.key.as_ref(), Some(&expected), "the worker's frame is shown");
    assert!(app.canvas.fill.is_none() && meshes.iter().all(|m| m.texture_id != fill_tex), "the fill is dropped");
    assert!(app.canvas.fill_worker.as_ref().unwrap().pending.is_none(), "the late fill was taken");
    // A pan with Outline turned on: the frame shown's options are not the ones asked for, and
    // no fill is asked for.
    app.ui.view.outline = true;
    app.view_mut().unwrap().center = Point::new(400.0, 150.0);
    frame(&mut app);
    assert!(jobs.try_iter().last().is_some_and(|j| j.key.outline), "the outline frame is on the worker");
    assert!(fill_jobs.try_recv().is_err() && app.canvas.fill.is_none(), "no fill");
    app.canvas.worker = None;
    app.canvas.fill_worker = None;
}

/// With the web build's setting (`waits_for_still`, no render worker), a heavy document's canvas
/// is not rendered while the view moves: each frame draws the frame shown moved and scaled with
/// the view and asks for a frame within STILL_SECS. The canvas renders once the view has been
/// still for STILL_SECS, and at once after an edit. With a worker's build settings the same frames render at once, as
/// before.
#[test]
fn without_a_worker_a_heavy_canvas_is_reprojected_until_the_view_is_still() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 100, "y": 50, "width": 200, "height": 200})).unwrap();
    assert!(!app.canvas.waits_for_still, "off the web build");
    app.canvas.worker_started = true; // no worker, as in the web build
    app.canvas.waits_for_still = true;
    let ctx = egui::Context::default();
    // One frame → (the bounds of the mesh that draws the art's texture, the repaint delay asked for).
    let frame = |app: &mut VectorcraftApp| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
        let tex = app.canvas.texture.as_ref().unwrap().id();
        let art = out.shapes.iter().find_map(|c| match &c.shape {
            Shape::Mesh(m) if m.texture_id == tex => Some(m.calc_bounds()),
            _ => None,
        });
        (art.expect("the art's texture is drawn"), out.viewport_output[&egui::ViewportId::ROOT].repaint_delay)
    };
    frame(&mut app); // the first frame fits the view to the artboard
    frame(&mut app);
    let rect = app.canvas_rect.unwrap();
    // A light document renders every change of view at once.
    app.canvas.last_ms = 1.0;
    app.view_mut().unwrap().center.x += 10.0;
    frame(&mut app);
    let shown = app.canvas.key.clone().unwrap();
    assert_eq!(shown.cx, app.view().unwrap().center.x, "rendered at once");
    // A heavy one draws the frame shown moved and scaled with the view.
    app.canvas.last_ms = 50.0;
    let old = Xf::new(rect, app.view().unwrap());
    for (dx, zoom) in [(10.0, 1.0), (10.0, 1.5), (-25.0, 1.5)] {
        let v = app.view_mut().unwrap();
        v.center.x += dx;
        v.zoom *= zoom;
        let (b, delay) = frame(&mut app);
        assert_eq!(app.canvas.key.as_ref(), Some(&shown), "not rendered while the view moves");
        let new = Xf::new(rect, app.view().unwrap());
        let (a, z) = (new.to_screen(old.to_doc(rect.min)), new.to_screen(old.to_doc(rect.max)));
        assert!((b.min - a).length() < 0.01 && (b.max - z).length() < 0.01, "{b:?}: the frame shown at {a:?} to {z:?}");
        assert!(delay <= std::time::Duration::from_secs_f64(STILL_SECS), "a frame within STILL_SECS: {delay:?}");
    }
    // The view stays still: frames at 60 fps (egui's input time without a clock) wait until
    // STILL_SECS has passed, and the next one renders.
    let mut frames = 0;
    let b = loop {
        frames += 1;
        let (b, _) = frame(&mut app);
        if app.canvas.key.as_ref() != Some(&shown) || frames == 60 {
            break b;
        }
    };
    assert!((12..=13).contains(&frames), "rendered after about 12 frames: {frames}");
    let now = app.canvas.key.clone().unwrap();
    assert_eq!((now.cx, now.zoom), (app.view().unwrap().center.x, app.view().unwrap().zoom), "rendered for the view");
    assert!((b.min - rect.min).length() < 0.01 && (b.max - rect.max).length() < 0.01, "{b:?} over {rect:?}");
    // An edit renders at once, even while the view moves.
    app.canvas.last_ms = 50.0;
    app.view_mut().unwrap().center.y += 10.0;
    frame(&mut app);
    assert_eq!(app.canvas.key.as_ref(), Some(&now), "waiting");
    app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    frame(&mut app);
    let k = app.canvas.key.clone().unwrap();
    assert_eq!((k.revision, k.cy), (app.session.active().unwrap().revision, app.view().unwrap().center.y), "rendered after the edit");
    // With a worker's build settings and no worker (VECTORCRAFT_SYNC_RENDER), a heavy document
    // renders every change of view at once.
    app.canvas.waits_for_still = false;
    app.canvas.last_ms = 50.0;
    app.view_mut().unwrap().center.x += 10.0;
    frame(&mut app);
    assert_eq!(app.canvas.key.as_ref().map(|k| k.cx), Some(app.view().unwrap().center.x), "rendered at once");
}

/// [`still_wait`] waits while only the view changes on a slow canvas: each change starts
/// STILL_SECS again. It does not wait for a light document, a still view, a clock that went
/// back, an edit, a switch into Pixel Preview or a canvas with no frame shown.
#[test]
fn the_canvas_waits_only_while_the_view_of_a_slow_frame_changes() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    shapes(&mut app, &egui::Context::default());
    let shown = app.canvas.key.clone().unwrap();
    let mut still = Some((shown.clone(), 0.0));
    let moved = CacheKey { cx: shown.cx + 5.0, zoom: shown.zoom * 2.0, ..shown.clone() };
    assert_eq!(still_wait(&mut still, Some(&shown), &moved, true, 10.0), Some(STILL_SECS), "the view moved");
    assert_eq!(still_wait(&mut still, Some(&shown), &moved, true, 10.0625), Some(STILL_SECS - 0.0625), "still for 62.5 ms");
    let turned = CacheKey { rot: 15.0, w: shown.w + 40, ppp: 2.0, ..moved.clone() };
    assert_eq!(still_wait(&mut still, Some(&shown), &turned, true, 11.0), Some(STILL_SECS), "turned and resized: again");
    assert_eq!(still_wait(&mut still, Some(&shown), &turned, true, 11.25), None, "still for longer than STILL_SECS");
    assert_eq!(still_wait(&mut still, Some(&shown), &turned, false, 11.0), None, "a light document");
    assert_eq!(still_wait(&mut still, Some(&shown), &turned, true, 10.5), None, "the clock went back");
    let edited = CacheKey { revision: shown.revision + 1, ..turned.clone() };
    assert_eq!(still_wait(&mut still, Some(&shown), &edited, true, 12.0), None, "an edit");
    let pixels = CacheKey { pixel: Some([0, 0, 10, 10]), ..moved.clone() };
    assert_eq!(still_wait(&mut still, Some(&shown), &pixels, true, 13.0), None, "into Pixel Preview");
    let shown_pixels = CacheKey { pixel: Some([5, 5, 20, 20]), ..shown.clone() };
    assert_eq!(still_wait(&mut still, Some(&shown_pixels), &pixels, true, 13.0), Some(STILL_SECS), "Pixel Preview, moved");
    assert_eq!(still_wait(&mut still, None, &moved, true, 14.0), None, "no frame shown");
    assert_eq!(still_wait(&mut still, Some(&shown), &shown, true, 15.0), None, "the frame shown");
}

/// File Handling › Display Bitmaps as Anti-aliased Images in Pixel Preview (#394): off, Pixel
/// Preview draws images with each pixel taking its nearest image pixel; on, smoothly, as the
/// canvas always does out of Pixel Preview.
#[test]
fn pixel_preview_smooths_bitmaps_only_when_asked() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    let ctx = egui::Context::default();
    let smooth = |app: &mut VectorcraftApp| {
        let (_, mut delta) = frame_output(app, &ctx);
        delta.clear();
        app.canvas.key.as_ref().map(|k| k.smooth_images).unwrap()
    };
    assert!(smooth(&mut app), "out of Pixel Preview");
    app.ui.view.pixel_preview = true;
    app.view_mut().unwrap().zoom = 8.0;
    assert!(!smooth(&mut app), "Pixel Preview, off by default");
    app.run("prefs.set", json!({"key": "antiAliasedBitmaps", "value": true})).unwrap();
    assert!(smooth(&mut app), "Pixel Preview, on");
}

/// Appearance of Black › On Screen (#394): Display All Blacks Accurately (the default) shows
/// pure CMYK black through the CMYK profile, a dark grey; Display All Blacks as Rich Black
/// shows it as RGB black. A rich black shows the same either way.
#[test]
fn appearance_of_black_on_screen_shows_pure_black_as_rgb_black() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300, "colorMode": "cmyk"})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    // Larger than the view: the middle of the art's raster is its fill.
    let id = app.run("shape.rectangle", json!({"x": -5000, "y": -5000, "width": 10000, "height": 10000})).unwrap()["id"].clone();
    let fill = |app: &mut VectorcraftApp, cmyk: [u8; 4]| {
        let [c, m, y, k] = cmyk;
        app.run("paint.setFill", json!({"ids": [id], "color": {"c": c, "m": m, "y": y, "k": k}})).unwrap();
    };
    let ctx = egui::Context::default();
    // The middle pixel of the art's raster uploaded by one frame, None when it was not re-rendered.
    let middle = |app: &mut VectorcraftApp| -> Option<[u8; 4]> {
        let (_, mut delta) = frame_output(app, &ctx);
        let art = app.canvas.texture.as_ref().unwrap().id();
        let px = delta.set.iter().find(|e| e.0.eq(&art)).and_then(|e| e.1.last()).map(|d| {
            let egui::ImageData::Color(img) = &d.image;
            let [w, h] = img.size;
            img.pixels[h / 2 * w + w / 2].to_array()
        });
        delta.clear();
        px
    };
    let shown = |cmyk: [f32; 4]| vectorcraft_color::Color::cmyk(cmyk[0], cmyk[1], cmyk[2], cmyk[3]).to_rgba8(1.0);
    let near = |a: [u8; 4], b: [u8; 4]| a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 1);

    fill(&mut app, [0, 0, 0, 100]);
    let accurate = middle(&mut app).expect("the first frame renders the art");
    assert!(near(accurate, shown([0.0, 0.0, 0.0, 1.0])), "accurate: through the CMYK profile, {accurate:?}");
    assert!(accurate[..3].iter().all(|v| *v > 10), "accurate: a dark grey, {accurate:?}");
    app.run("prefs.set", json!({"key": "blackOnScreen", "value": "rich"})).unwrap();
    assert_eq!(middle(&mut app).expect("the preference change re-renders"), [0, 0, 0, 255], "rich: RGB black");

    fill(&mut app, [60, 40, 40, 100]);
    let rich = middle(&mut app).expect("re-rendered");
    assert!(near(rich, shown([0.6, 0.4, 0.4, 1.0])), "a rich black is not pure black: {rich:?}");

    fill(&mut app, [0, 0, 0, 100]);
    app.run("prefs.set", json!({"key": "blackOnScreen", "value": "accurate"})).unwrap();
    assert!(near(middle(&mut app).expect("re-rendered"), accurate), "accurate again");
}

/// Guides & Grid › Show Pixel Grid (Above 600% Zoom) (#394): in Pixel Preview at 600% zoom and
/// above, a line at every document pixel over the art; none below 600%, out of Pixel Preview,
/// or with the option off.
#[test]
fn pixel_grid_shows_in_pixel_preview_from_600_percent() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 100, "y": 50, "width": 200, "height": 200})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    let ctx = egui::Context::default();
    shapes(&mut app, &ctx); // the first frame fits the view to the artboard
    // → (the pixel grid's lines, whether the first of them is drawn over the art).
    let pixel_lines = |app: &mut VectorcraftApp| {
        let s = shapes(app, &ctx);
        let art = app.canvas.texture.as_ref().unwrap().id();
        let image = s.iter().position(|s| matches!(s, Shape::Mesh(m) if m.texture_id == art)).expect("the art");
        let is_line = |s: &Shape| matches!(s, Shape::LineSegment { stroke, .. } if stroke.color == PIXEL_GRID);
        let lines: Vec<usize> = s.iter().enumerate().filter(|(_, s)| is_line(s)).map(|(i, _)| i).collect();
        (lines.len(), lines.first().is_some_and(|&i| i > image))
    };
    app.ui.view.pixel_preview = true;
    app.view_mut().unwrap().zoom = 8.0;
    let (n, over) = pixel_lines(&mut app);
    assert!(n >= 150, "a line every 8 px across the 800 × 600 canvas: {n}");
    assert!(over, "over the art");
    app.view_mut().unwrap().zoom = 5.0;
    assert_eq!(pixel_lines(&mut app).0, 0, "below 600%: none");
    app.view_mut().unwrap().zoom = 6.0;
    assert!(pixel_lines(&mut app).0 >= 200, "from 600%");
    app.ui.view.pixel_preview = false;
    assert_eq!(pixel_lines(&mut app).0, 0, "out of Pixel Preview: none");
    app.ui.view.pixel_preview = true;
    app.session.execute("prefs.set", &json!({"key": "showPixelGrid", "value": false})).unwrap();
    assert_eq!(pixel_lines(&mut app).0, 0, "the option off: none");
    // A rotated view: the lines turn with the pixels, across the whole canvas.
    app.session.execute("prefs.set", &json!({"key": "showPixelGrid", "value": true})).unwrap();
    app.view_mut().unwrap().rotation = 30.0;
    let s = shapes(&mut app, &ctx);
    let turned: Vec<_> = s
        .iter()
        .filter_map(|s| match s {
            Shape::LineSegment { points: [a, b], stroke } if stroke.color == PIXEL_GRID => Some((b.x - a.x).atan2(b.y - a.y).to_degrees().abs()),
            _ => None,
        })
        .collect();
    assert!(turned.len() >= 200, "{}", turned.len());
    assert!(turned.iter().all(|a| [30.0, 60.0, 120.0, 150.0].iter().any(|t| (a - t).abs() < 0.5)), "turned 30°: {turned:?}");
}

/// Guides & Grid (#394): the grid in Grid Color, behind the art or, with Grids In Back off,
/// over it, as dots in the Dots style; the guides in Guides Color and Style.
#[test]
fn grid_and_guides_take_their_preferences() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("guide.add", &json!({"vertical": true, "pos": 100})).unwrap();
    app.ui.view.grid = true;
    app.ui.view.guides = true;
    let (red, green) = (Color32::from_rgb(255, 0, 0), Color32::from_rgb(0, 255, 0));
    app.session.execute("prefs.set", &json!({"values": {"gridColor": "#ff0000", "guideColor": "#00ff00"}})).unwrap();
    let ctx = egui::Context::default();
    let red_line = |s: &Shape| matches!(s, Shape::LineSegment { stroke, .. } if stroke.color == red);
    // → (index of the first red gridline, index of the art's image).
    let order = |app: &mut VectorcraftApp| {
        let s = shapes(app, &ctx);
        let art = app.canvas.texture.as_ref().unwrap().id();
        let image = s.iter().position(|s| matches!(s, Shape::Mesh(m) if m.texture_id == art)).expect("the art");
        (s.iter().position(red_line).expect("a gridline in Grid Color"), image, s)
    };
    let (grid, art, s) = order(&mut app);
    assert!(grid < art, "Grids In Back: under the art");
    assert!(s.iter().any(|s| matches!(s, Shape::LineSegment { stroke, .. } if stroke.color == green)), "the guide in Guides Color");
    app.session.execute("prefs.set", &json!({"key": "gridsInBack", "value": false})).unwrap();
    let (grid, art, _) = order(&mut app);
    assert!(grid > art, "Grids In Back off: over the art");
    app.session.execute("prefs.set", &json!({"values": {"gridStyle": "dots", "guideStyle": "dots"}})).unwrap();
    let s = shapes(&mut app, &ctx);
    assert!(!s.iter().any(red_line), "no gridlines in the Dots style");
    let red_dots = |s: &Shape| matches!(s, Shape::Mesh(m) if m.vertices.len() > 40 && m.vertices.iter().all(|v| v.color == red));
    assert!(s.iter().any(red_dots), "the grid's dots");
    assert!(s.iter().any(|s| matches!(s, Shape::Circle(c) if c.fill == green)), "the guide dotted");
}

/// A canvas guide (on no artboard) is drawn along its document line in a rotated view, where it
/// snaps and is picked: both ends of each guide's line are on x = 100 or y = 120. At 0° the
/// vertical one runs from the canvas's top to its bottom.
#[test]
fn canvas_guides_turn_with_the_view() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("guide.add", &json!({"vertical": true, "pos": 100})).unwrap();
    app.session.execute("guide.add", &json!({"vertical": false, "pos": 120})).unwrap();
    app.session.execute("prefs.set", &json!({"key": "guideColor", "value": "#123456"})).unwrap();
    app.ui.view.guides = true;
    let color = Color32::from_rgb(0x12, 0x34, 0x56);
    let ctx = egui::Context::default();
    let lines = |app: &mut VectorcraftApp| -> Vec<[Pos2; 2]> {
        shapes(app, &ctx)
            .into_iter()
            .filter_map(|s| if let Shape::LineSegment { points, stroke } = s { (stroke.color == color).then_some(points) } else { None })
            .collect()
    };
    let s = lines(&mut app);
    let rect = app.canvas_rect.unwrap();
    let vertical = s.iter().find(|[a, b]| (a.x - b.x).abs() < 1e-3).expect("the vertical guide");
    assert!((vertical[0].y - rect.top()).abs() < 0.5 && (vertical[1].y - rect.bottom()).abs() < 0.5, "0°: {vertical:?} in {rect:?}");
    app.run("view.rotate", json!({"angle": 30})).unwrap();
    let s = lines(&mut app);
    let xf = Xf::new(rect, app.view().unwrap());
    assert_eq!(s.len(), 2, "{s:?}");
    let on = |v: f64, along_x: bool| {
        s.iter().any(|ends| ends.iter().all(|e| ((if along_x { xf.to_doc(*e).x } else { xf.to_doc(*e).y }) - v).abs() < 1e-3))
    };
    assert!(on(100.0, true), "x = 100: {s:?}");
    assert!(on(120.0, false), "y = 120: {s:?}");
}

/// Selection & Anchor Display › Size (#394): anchors and the bounding box's handles grow.
#[test]
fn anchor_size_scales_anchors_and_handles() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 150, "y": 100, "width": 100, "height": 100})).unwrap();
    let ctx = egui::Context::default();
    // The sizes of the squares drawn (anchors and handles are squares).
    let sizes = |app: &mut VectorcraftApp| {
        let mut v: Vec<f32> = shapes(app, &ctx).iter().filter_map(|s| if let Shape::Rect(r) = s { Some(r.rect.width()) } else { None }).collect();
        v.sort_by(f32::total_cmp);
        v.dedup();
        v
    };
    let small = sizes(&mut app);
    assert!(small.contains(&4.0) && small.contains(&6.0), "anchors 4, handles 6 by default: {small:?}");
    app.session.execute("prefs.set", &json!({"key": "anchorSize", "value": 7})).unwrap();
    let big = sizes(&mut app);
    assert!(big.contains(&8.0) && big.contains(&10.0) && !big.contains(&4.0), "4 points bigger at 7: {big:?}");
}

#[test]
fn mirror_axis_tracks_selection_options_and_view_without_editing_art() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 60, "height": 80})).unwrap();
    let id = app.run("object.repeat.mirror", json!({})).unwrap()["id"].as_u64().unwrap();
    let ctx = egui::Context::default();
    let axis = |app: &mut VectorcraftApp| {
        let color = Tokens::get(&ctx).selection;
        shapes(app, &ctx)
            .into_iter()
            .filter_map(|s| match s {
                Shape::LineSegment { points, stroke } if stroke.color == color && stroke.width == 2.0 => Some(points),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let before = serde_json::to_value(&app.session.active().unwrap().doc).unwrap();
    let lines = axis(&mut app);
    assert_eq!(lines.len(), 1);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    assert!(lines[0][0].distance(xf.to_screen(Point::new(170.0, 100.0)) - vec2(0.0, 24.0)) < 0.01);
    assert!(lines[0][1].distance(xf.to_screen(Point::new(170.0, 180.0)) + vec2(0.0, 24.0)) < 0.01);
    assert_eq!(serde_json::to_value(&app.session.active().unwrap().doc).unwrap(), before);
    app.ui.view.edges = false;
    assert!(axis(&mut app).is_empty());
    app.ui.view.edges = true;
    app.run("select.set", json!({"ids": []})).unwrap();
    assert!(axis(&mut app).is_empty());
    app.run("select.set", json!({"ids": [id]})).unwrap();
    assert_eq!(axis(&mut app), lines);
    app.run("object.repeat.options", json!({"angle": 0, "center": [180, 140]})).unwrap();
    let horizontal = axis(&mut app);
    assert_eq!(horizontal.len(), 1);
    assert!((horizontal[0][0].y - horizontal[0][1].y).abs() < 0.01);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(axis(&mut app), lines);
    app.view_mut().unwrap().rotation = 45.0;
    let rotated = axis(&mut app);
    let d = rotated[0][1] - rotated[0][0];
    assert!((d.x.abs() - d.y.abs()).abs() < 0.01);
    app.run("object.repeat.release", json!({})).unwrap();
    assert!(axis(&mut app).is_empty());
}

#[test]
fn mirror_axis_drag_uses_canvas_coordinates_in_a_zoomed_rotated_view() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 60, "height": 80})).unwrap();
    app.run("object.repeat.mirror", json!({})).unwrap();
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    app.view_mut().unwrap().zoom = 1.5;
    app.view_mut().unwrap().rotation = 30.0;
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let axis = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        vectorcraft_tools::mirror::MirrorAxis::of(&st.doc, &st.selection, app.view().unwrap().zoom).unwrap()
    };
    let initial = axis(&app);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let from = xf.to_screen(initial.center);
    let to = from + vec2(40.0, 15.0);
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from), button(from, true)]);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(to)]);
    assert!(axis(&app).center.distance(xf.to_doc(to)) < 0.001, "the mirrored artwork previews during the drag");
    frame(&mut app, &ctx, vec![button(to, false)]);
    assert!(axis(&app).center.distance(xf.to_doc(to)) < 0.001);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(axis(&app).center, initial.center);
    // Both rotation handles remain a usable size at this zoom and view angle.
    let drawn = shapes(&mut app, &ctx);
    for end in initial.ends {
        assert!(drawn.iter().any(|s| matches!(s, Shape::Circle(c)
            if c.center.distance(xf.to_screen(end)) < 0.01 && c.radius == vectorcraft_tools::mirror::HANDLE_RADIUS)));
    }
}

/// A selected grid repeat draws its row handle below the grid and a slider between its first
/// two columns (none with the bounding box hidden). Dragging the row handle down two tiles in
/// a zoomed view makes five rows without scaling the source, as one undo step, and the slider
/// sets the horizontal spacing (#2691).
#[test]
fn grid_repeat_handles_set_rows_and_spacing_in_one_undo_step_each() {
    use vectorcraft_tools::repeat_grid::{GridHandles, HANDLE_RADIUS};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 40, "height": 40})).unwrap();
    app.run("object.repeat.grid", json!({"hSpacing": 10, "vSpacing": 10})).unwrap();
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    let ctx = egui::Context::default();
    // The first frame fits the artboard in the window; then zoom to 1.5 px a point.
    frame(&mut app, &ctx, vec![]);
    app.view_mut().unwrap().zoom = 1.5;
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.view().unwrap().zoom, 1.5);
    // Rows, columns, horizontal spacing and the source's bounds.
    let grid = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        let n = st.doc.node(st.selection.objects[0]).unwrap();
        let vectorcraft_doc::NodeKind::Repeat(spec) = &n.kind else { panic!("a repeat") };
        let vectorcraft_doc::RepeatKind::Grid { rows, cols, h_spacing, .. } = spec.kind else { panic!("a grid repeat") };
        (rows, cols, h_spacing, spec.source_bounds().unwrap())
    };
    let handles = |app: &mut VectorcraftApp| {
        let frame = app.selection_box();
        let st = app.session.active().unwrap();
        GridHandles::of(&st.doc, &st.selection, frame, app.view().unwrap().zoom).unwrap()
    };
    let before = grid(&app);
    assert_eq!((before.0, before.1, before.2), (3, 3, 10.0));
    let g = handles(&mut app);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let drawn = shapes(&mut app, &ctx);
    let circle_at =
        |drawn: &[Shape], at: Pos2| drawn.iter().any(|s| matches!(s, Shape::Circle(c) if c.center.distance(at) < 0.01 && c.radius == HANDLE_RADIUS));
    assert!(circle_at(&drawn, xf.to_screen(g.rows_at)), "the row handle is drawn");
    assert!(circle_at(&drawn, xf.to_screen(g.corner_at)), "the corner handle is drawn");
    app.ui.view.bounding_box = false;
    assert!(!circle_at(&shapes(&mut app, &ctx), xf.to_screen(g.rows_at)), "hidden with the bounding box");
    app.ui.view.bounding_box = true;
    frame(&mut app, &ctx, vec![]);
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let drag = |app: &mut VectorcraftApp, from: Pos2, by: egui::Vec2| {
        frame(app, &ctx, vec![egui::Event::PointerMoved(from), button(from, true)]);
        for k in 1..=4 {
            frame(app, &ctx, vec![egui::Event::PointerMoved(from + by * k as f32 / 4.0)]);
        }
        frame(app, &ctx, vec![button(from + by, false)]);
    };
    // Two tiles of 50 pt down, at 1.5 px a point.
    drag(&mut app, xf.to_screen(g.rows_at), vec2(0.0, 150.0));
    let after = grid(&app);
    assert_eq!((after.0, after.1, after.2), (5, 3, 10.0));
    assert_eq!(after.3, before.3, "the source keeps its size");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(grid(&app).0, 3, "one undo step");
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(grid(&app).0, 5);
    // The slider between the first two columns, 6 px (4 pt) right: it stays halfway across a
    // gap 8 pt wider, 18 pt of spacing.
    frame(&mut app, &ctx, vec![]);
    let g = handles(&mut app);
    drag(&mut app, xf.to_screen(g.h_slider.unwrap()), vec2(6.0, 0.0));
    assert_eq!(grid(&app).2, 18.0);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!((grid(&app).0, grid(&app).2), (5, 10.0), "the spacing's own undo step");
}

#[test]
fn top_bounding_box_handle_scales_a_default_mirror_repeat() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
    app.run("object.repeat.mirror", json!({})).unwrap();
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);

    let initial = app.selection_bounds().unwrap();
    let bx = app.selection_box().unwrap();
    let top = bx.to_doc() * vectorcraft_tools::bbox::Handle::Top.pos(bx.rect);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let from = xf.to_screen(top);
    let to = from + vec2(0.0, -30.0);
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };

    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from), button(from, true)]);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(to)]);
    frame(&mut app, &ctx, vec![button(to, false)]);

    let scaled = app.selection_bounds().unwrap();
    assert!(scaled.height() > initial.height() + 10.0, "Top handle should scale the repeat: {initial:?} -> {scaled:?}");
    assert!((scaled.width() - initial.width()).abs() < 0.01, "Top handle should not scale width: {initial:?} -> {scaled:?}");
}

/// #1236: Curvature edits points, so existing paths show one circular marker per point and
/// no direction handles; switching back to Direct Selection keeps its normal handles.
#[test]
fn curvature_points_of_an_existing_path_have_no_direction_handles() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = vectorcraft_doc::NodeId(
        app.session
            .execute(
                "path.curvature",
                &json!({"points": [
                    {"x": 60, "y": 200}, {"x": 120, "y": 80}, {"x": 210, "y": 160}, {"x": 280, "y": 60}, {"x": 350, "y": 180}
                ]}),
            )
            .unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    let document = app.session.active().unwrap().doc.clone();
    let ctx = egui::Context::default();
    app.select_tool("curvature");
    let drawn = shapes(&mut app, &ctx);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let points: Vec<_> = document.node(id).unwrap().path_data().unwrap().anchors().map(|(_, _, a)| xf.to_screen(a.p)).collect();
    let marker = |shape: &Shape| matches!(shape, Shape::Circle(c) if points.iter().any(|p| p.distance(c.center) < 0.01));
    assert_eq!(drawn.iter().filter(|s| marker(s)).count(), 5, "one circular marker at every curve point");
    assert!(
        !drawn.iter().any(|s| matches!(s, Shape::Rect(r) if points.iter().any(|p| p.distance(r.rect.center()) < 0.01))),
        "no square at a curve point"
    );
    let color = c32(document.layer_color(id));
    let handles = |shapes: &[Shape]| shapes.iter().filter(|s| matches!(s, Shape::LineSegment { stroke, .. } if stroke.color == color)).count();
    assert_eq!(handles(&drawn), 0, "Curvature hides the direction lines");
    app.session.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 2]], "mode": "set"})).unwrap();
    let partial = shapes(&mut app, &ctx);
    assert_eq!(partial.iter().filter(|s| marker(s)).count(), 5, "other curve points stay visible");
    assert_eq!(handles(&partial), 0, "a single selected point still shows no direction lines");
    app.ui.view.edges = false;
    assert_eq!(shapes(&mut app, &ctx).iter().filter(|s| marker(s)).count(), 0, "Hide Edges still hides an existing path's markers");
    app.ui.view.edges = true;
    app.select_tool("directSelection");
    assert_eq!(handles(&shapes(&mut app, &ctx)), 2, "Direct Selection keeps the selected point's two handles");
    app.select_tool("pen");
    assert_eq!(handles(&shapes(&mut app, &ctx)), 2, "Pen keeps the selected point's two handles");
    assert_eq!(
        serde_json::to_value(&app.session.active().unwrap().doc).unwrap(),
        serde_json::to_value(&document).unwrap(),
        "tool switches and painting don't change the curve"
    );
}

/// While drawing, Curvature's current-point feedback takes the place of generic selection
/// markers: no duplicate squares or handles, and the path is unchanged by drawing a frame.
#[test]
fn curvature_drawing_shows_its_points_once() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let ctx = egui::Context::default();
    shapes(&mut app, &ctx);
    app.select_tool("curvature");
    for (x, y) in [(60.0, 200.0), (120.0, 80.0), (210.0, 160.0), (280.0, 60.0), (350.0, 180.0)] {
        let view = app.view_info();
        for kind in [vectorcraft_tools::PointerKind::Down, vectorcraft_tools::PointerKind::Up] {
            app.session.pointer(&vectorcraft_tools::PointerEvent::new(kind, x, y), view).unwrap();
        }
    }
    let st = app.session.active().unwrap();
    let id = st.selection.objects[0];
    let document = st.doc.clone();
    let selection = st.selection.clone();
    let drawn = shapes(&mut app, &ctx);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let points: Vec<_> = document.node(id).unwrap().path_data().unwrap().anchors().map(|(_, _, a)| xf.to_screen(a.p)).collect();
    let markers: Vec<_> = drawn
        .iter()
        .filter_map(|s| match s {
            Shape::Circle(c) if points.iter().any(|p| p.distance(c.center) < 0.01) => Some(c),
            _ => None,
        })
        .collect();
    assert_eq!(markers.len(), 5, "one marker per point, including the current point");
    assert_eq!(markers.iter().filter(|c| c.fill != Color32::WHITE).count(), 1, "only the last touched point is filled");
    assert!(
        !drawn.iter().any(|s| matches!(s, Shape::Rect(r) if points.iter().any(|p| p.distance(r.rect.center()) < 0.01))),
        "neither tool nor selection draws anchor squares"
    );
    let color = c32(document.layer_color(id));
    assert!(!drawn.iter().any(|s| matches!(s, Shape::LineSegment { stroke, .. } if stroke.color == color)), "no direction lines");
    app.session.execute("prefs.set", &json!({"key": "anchorSize", "value": 7})).unwrap();
    assert_eq!(
        shapes(&mut app, &ctx)
            .iter()
            .filter(|s| matches!(s, Shape::Circle(c) if c.radius == 5.0 && points.iter().any(|p| p.distance(c.center) < 0.01)))
            .count(),
        5,
        "point markers respect Anchor Size"
    );
    assert_eq!(app.session.active().unwrap().selection, selection);
    assert_eq!(serde_json::to_value(&app.session.active().unwrap().doc).unwrap(), serde_json::to_value(&document).unwrap());
}

/// Selection & Anchor Display › Handles and Show handles when multiple anchors are selected
/// (#394): handle ends drawn solid, hollow or large; with the latter off, a second selected
/// anchor hides the handles.
#[test]
fn handle_style_and_handles_of_multiple_anchors() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("shape.ellipse", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].clone();
    app.session.select_tool("directSelection", app.view_info()).unwrap();
    let ctx = egui::Context::default();
    let select = |app: &mut VectorcraftApp, anchors: serde_json::Value| {
        app.session.execute("select.anchors", &json!({"id": id, "anchors": anchors, "mode": "set"})).unwrap();
    };
    // The handle ends: (radius, filled white).
    let dots = |app: &mut VectorcraftApp| -> Vec<(f32, bool)> {
        shapes(app, &ctx)
            .iter()
            .filter_map(|s| match s {
                Shape::Circle(c) if c.fill != Color32::TRANSPARENT => Some((c.radius, c.fill == Color32::WHITE)),
                _ => None,
            })
            .collect()
    };
    select(&mut app, json!([[0, 0]]));
    assert_eq!(dots(&mut app), [(2.75, false); 2], "solid by default");
    app.session.execute("prefs.set", &json!({"key": "handleStyle", "value": "hollow"})).unwrap();
    assert_eq!(dots(&mut app), [(2.75, true); 2]);
    app.session.execute("prefs.set", &json!({"key": "handleStyle", "value": "large"})).unwrap();
    assert_eq!(dots(&mut app), [(4.25, false); 2]);
    select(&mut app, json!([[0, 0], [0, 1]]));
    assert_eq!(dots(&mut app).len(), 4, "both anchors' handles");
    app.session.execute("prefs.set", &json!({"key": "showHandlesMultipleAnchors", "value": false})).unwrap();
    assert!(dots(&mut app).is_empty(), "off: none for two anchors");
    select(&mut app, json!([[0, 1]]));
    assert_eq!(dots(&mut app).len(), 2, "one anchor still shows them");
}

/// #3219: a selected path made into a guide is deselected when guides are hidden, so the
/// selection overlay cannot reveal the hidden path.
#[test]
fn hide_guides_deselects_selected_guide_paths() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.run("view.guides.make", json!({})).unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    assert_eq!(app.session.active().unwrap().selection.objects, vec![vectorcraft_doc::NodeId(id)]);

    app.run("view.guides", json!({})).unwrap();

    assert!(app.session.active().unwrap().selection.objects.is_empty());
}

/// Smart Guides › Object Highlighting (#394): the outline of the object under the pointer does not
/// show with View › Snap to Grid or View › Pixel Preview on, even with Smart Guides and the option on.
/// #3219: guide paths hidden by View > Hide Guides are not hittable and cannot reveal
/// themselves through Smart Guides object highlighting.
#[test]
fn hidden_guide_paths_have_no_hover_highlight() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.run("view.guides.make", json!({})).unwrap();
    app.run("select.none", json!({})).unwrap();
    app.select_tool("selection");
    app.ui.view.guides = false;
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let color = c32(app.session.active().unwrap().doc.layer_color(vectorcraft_doc::NodeId(id)));
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(xf.to_screen(Point::new(100.0, 150.0)))]);
    assert!(drawn_in(&mut app, &ctx, color, 1.5).is_empty(), "a hidden guide has no hover highlight");
}

#[test]
fn snap_to_grid_and_pixel_preview_hide_the_hover_outline() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.none", &json!({})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let rect = app.canvas_rect.unwrap();
    let over = Xf::new(rect, app.view().unwrap()).to_screen(Point::new(150.0, 150.0));
    let layer = egui::epaint::ColorMode::Solid(c32(app.session.active().unwrap().doc.layer_color(vectorcraft_doc::NodeId(id))));
    let outlines = |app: &mut VectorcraftApp| {
        frame(app, &ctx, vec![egui::Event::PointerMoved(over)]);
        shapes(app, &ctx).iter().filter(|s| matches!(s, Shape::Path(ps) if ps.stroke.width == 1.5 && ps.stroke.color == layer)).count()
    };
    assert!(app.ui.view.smart_guides && app.session.prefs.object_highlighting);
    assert!(outlines(&mut app) > 0, "highlighted with Snap to Grid and Pixel Preview off");
    app.run("view.snapToGrid", json!({})).unwrap();
    assert!(app.ui.view.snap_to_grid);
    assert_eq!(outlines(&mut app), 0, "Snap to Grid on");
    app.run("view.snapToGrid", json!({})).unwrap();
    assert!(outlines(&mut app) > 0, "Snap to Grid off again");
    app.run("view.pixelPreview", json!({})).unwrap();
    assert!(app.ui.view.pixel_preview);
    assert_eq!(outlines(&mut app), 0, "Pixel Preview on");
    app.run("view.pixelPreview", json!({})).unwrap();
    assert!(outlines(&mut app) > 0, "Pixel Preview off again");
}

/// #2452: while the view scrolls under a resting pointer, the art under it isn't hit-tested:
/// no hover outline, and the tool's last cursor stays. Once the view has been still for
/// [`HOVER_STILL_SECS`], the outline comes back without the pointer moving.
#[test]
fn a_moving_view_leaves_the_art_under_the_pointer_unhighlighted_until_it_stops() {
    fn flat(s: &Shape, out: &mut Vec<Shape>) {
        match s {
            Shape::Vec(v) => v.iter().for_each(|s| flat(s, out)),
            s => out.push(s.clone()),
        }
    }
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 400, "height": 300})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.none", &json!({})).unwrap();
    let layer = egui::epaint::ColorMode::Solid(c32(app.session.active().unwrap().doc.layer_color(vectorcraft_doc::NodeId(id))));
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
    let mut time = 0.0;
    let mut outlines = |app: &mut VectorcraftApp, events: Vec<egui::Event>, dt: f64| {
        time += dt;
        let mut out = ctx.run_ui(egui::RawInput { time: Some(time), screen_rect: Some(screen), events, ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
        let mut shapes = vec![];
        out.shapes.iter().for_each(|c| flat(&c.shape, &mut shapes));
        shapes.iter().filter(|s| matches!(s, Shape::Path(ps) if ps.stroke.width == 1.5 && ps.stroke.color == layer)).count()
    };
    outlines(&mut app, vec![], 0.0);
    let at = app.canvas_rect.unwrap().center();
    assert!(outlines(&mut app, vec![egui::Event::PointerMoved(at)], 0.016) > 0, "the art under the resting pointer is highlighted");
    let wheel = |dy: f32| egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, dy),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
        source: egui::MouseWheelSource::Unknown,
    };
    for k in 0..12 {
        let before = app.view().unwrap().center;
        let shown = outlines(&mut app, vec![wheel(if k % 2 == 0 { 4.0 } else { -4.0 })], 0.016);
        assert_ne!(app.view().unwrap().center, before, "frame {k}: the wheel scrolls the view");
        assert_eq!(shown, 0, "frame {k}: no hover outline while the view moves");
    }
    assert_eq!(outlines(&mut app, vec![], HOVER_STILL_SECS / 2.0), 0, "the view has only just stopped");
    assert!(outlines(&mut app, vec![], HOVER_STILL_SECS) > 0, "still again: the outline comes back");
    // Selected, the rectangle gives the move cursor near its top edge. Scrolled up past the
    // pointer, it stays the move cursor until the view stops, then the arrow.
    app.session.execute("select.all", &json!({})).unwrap();
    outlines(&mut app, vec![egui::Event::PointerMoved(at - vec2(0.0, 200.0))], 0.016);
    let cursor = |app: &VectorcraftApp| app.canvas.hover_cursor.map(|(_, _, c)| c);
    assert_eq!(cursor(&app), Some(Cursor::Move));
    for k in 0..12 {
        let before = app.view().unwrap().center;
        outlines(&mut app, vec![wheel(-80.0)], 0.016);
        assert_ne!(app.view().unwrap().center, before, "frame {k}: the wheel scrolls the view");
        assert_eq!(cursor(&app), Some(Cursor::Move), "frame {k}: the cursor stays while the view moves");
    }
    // The wheel's smooth scrolling runs on for some frames; then the view rests. A frame of it
    // that scrolls less than half a device pixel leaves the view where it is.
    for _ in 0..200 {
        let before = app.view().unwrap().center;
        outlines(&mut app, vec![], 0.016);
        if app.view().unwrap().center == before && !ctx.input(|i| i.is_scrolling()) {
            break;
        }
    }
    assert_eq!(cursor(&app), Some(Cursor::Move), "the view has only just stopped");
    outlines(&mut app, vec![], HOVER_STILL_SECS + 0.01);
    assert_eq!(cursor(&app), Some(Cursor::Arrow), "still again: the pointer at {:?} is past the rectangle", app.hover_doc);
}

/// The wheel, a scroll bar and the Hand tool move the view by whole device pixels at 1.25
/// device pixels a point, in a zoomed and turned view: every frame's render view is the first
/// one's panned by whole pixels, which the renderer's cropped effect rasters need to be drawn
/// again, and the wheel's and the scroll bar's scrolls add up to what they were given within a
/// device pixel, as does the drag.
#[test]
fn scrolls_and_pans_move_the_view_by_whole_device_pixels() {
    const PPP: f32 = 1.25;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 400, "height": 300})).unwrap();
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
    let mut time = 0.0;
    let mut frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        time += 0.016;
        let mut input = egui::RawInput { time: Some(time), screen_rect: Some(screen), events, ..Default::default() };
        input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().native_pixels_per_point = Some(PPP);
        ctx.run_ui(input, |ui| show(app, ui)).textures_delta.clear();
    };
    frame(&mut app, vec![]);
    {
        let v = app.view_mut().unwrap();
        v.zoom = 4.0;
        v.rotation = 30.0;
        v.center = Point::new(200.3, 150.7);
    }
    frame(&mut app, vec![]);
    // The pan of the canvas's render view (the translation of the view it renders through, less
    // the half of its size), in device pixels.
    let pan = |app: &VectorcraftApp| {
        let v = app.view().unwrap();
        (Affine::rotate(v.rotation.to_radians()) * Affine::scale(v.zoom * f64::from(PPP)) * Affine::translate(-v.center.to_vec2())).translation()
    };
    let wheel = |d: egui::Vec2| egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: d,
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
        source: egui::MouseWheelSource::Unknown,
    };
    let whole = |by: vectorcraft_geom::Vec2| (by - vectorcraft_geom::Vec2::new(by.x.round(), by.y.round())).hypot() < 1e-6;
    let rect = app.canvas_rect.unwrap();
    let track = scroll_tracks(rect, screen)[0].center();
    let deltas = [vec2(0.37, 11.13), vec2(-12.71, 0.0), vec2(0.0, 7.45), vec2(21.9, -3.83), vec2(0.21, 0.21)];
    for (name, at) in [("the wheel", rect.center()), ("the horizontal scroll bar", track)] {
        frame(&mut app, vec![egui::Event::PointerMoved(at)]);
        let start = pan(&app);
        let mut given = vectorcraft_geom::Vec2::ZERO;
        for (k, d) in deltas.iter().enumerate() {
            frame(&mut app, vec![wheel(*d)]);
            // Over the scroll bar, the wheel scrolls along it.
            given += if at == track {
                vectorcraft_geom::Vec2::new(f64::from(d.x + d.y), 0.0)
            } else {
                vectorcraft_geom::Vec2::new(f64::from(d.x), f64::from(d.y))
            };
            let by = pan(&app) - start;
            assert!(whole(by), "{name}, scroll {k}: the view moved {by:?} device pixels");
        }
        // The wheel's smooth scrolling runs on for some frames.
        for k in 0..60 {
            frame(&mut app, vec![]);
            let by = pan(&app) - start;
            assert!(whole(by), "{name}, frame {k} after the scrolls: the view moved {by:?} device pixels");
        }
        let by = pan(&app) - start;
        let want = given * f64::from(PPP);
        assert!(by.hypot() > 10.0, "{name}: the view moved {by:?} device pixels");
        assert!((by.x - want.x).abs() <= 1.0 && (by.y - want.y).abs() <= 1.0, "{name}: moved {by:?} device pixels for {want:?}");
    }
    // The horizontal scroll bar's thumb dragged to points between device pixels: the view moves
    // `ratio` points for each point the thumb moves, the other way.
    let (_, span, from) = scroll_bar(&mut app, 0);
    let ratio = f64::from(span.ratio(scroll_tracks(rect, screen)[0].width()));
    assert!(ratio > 1.0, "at 400% the view moves further than the thumb: {ratio}");
    frame(&mut app, vec![egui::Event::PointerMoved(from)]);
    let start = pan(&app);
    frame(&mut app, vec![primary(from, true)]);
    for (k, dx) in [7.37_f32, 12.81, 3.33].into_iter().enumerate() {
        frame(&mut app, vec![egui::Event::PointerMoved(from + vec2(dx, 0.0))]);
        let by = pan(&app) - start;
        assert!(whole(by), "thumb drag {k}: the view moved {by:?} device pixels");
        let want = -f64::from(dx) * ratio * f64::from(PPP);
        assert!((by.x - want).abs() <= 1.0 && by.y.abs() <= 1.0, "thumb drag {k}: moved {by:?} device pixels for {want}");
    }
    frame(&mut app, vec![primary(from + vec2(3.33, 0.0), false)]);
    // A Hand tool drag to points between device pixels.
    app.select_tool("hand");
    let p0 = rect.center() + vec2(0.3, 0.1);
    frame(&mut app, vec![egui::Event::PointerMoved(p0)]);
    let start = pan(&app);
    frame(&mut app, vec![primary(p0, true)]);
    for (k, d) in [vec2(13.37, -7.61), vec2(40.19, 3.33), vec2(-5.55, 22.71)].into_iter().enumerate() {
        frame(&mut app, vec![egui::Event::PointerMoved(p0 + d)]);
        let by = pan(&app) - start;
        assert!(whole(by), "drag {k}: the view moved {by:?} device pixels");
        let want = vectorcraft_geom::Vec2::new(f64::from(d.x), f64::from(d.y)) * f64::from(PPP);
        assert!((by.x - want.x).abs() <= 1.0 && (by.y - want.y).abs() <= 1.0, "drag {k}: moved {by:?} device pixels for {want:?}");
    }
    frame(&mut app, vec![primary(p0 + vec2(-5.55, 22.71), false)]);
}

/// The part of a device pixel a scroll leaves is added to the next scroll of the same view, and
/// dropped when the view was zoomed, turned or moved in between, when the next scroll is in
/// another document, or when the device pixels a point changed.
#[test]
fn a_scrolls_leftover_pixel_part_carries_only_to_the_same_view() {
    let view = View { zoom: 1.0, center: Point::new(100.0, 100.0), fitted: true, rotation: 0.0, artboard: 0 };
    let mut v = view;
    let rest = scroll_by(&mut v, 1, 1.0, vec2(0.4, -0.3), ScrollRest::default());
    assert_eq!(v.center, view.center, "less than half a pixel does not move the view");
    assert!((rest.x - 0.4).abs() < 1e-6 && (rest.y + 0.3).abs() < 1e-6);
    let mut again = v;
    scroll_by(&mut again, 1, 1.0, vec2(0.4, -0.3), rest);
    assert_eq!(again.center, Point::new(99.0, 101.0), "the second scroll adds the first's part");
    let changed = [
        ("zoomed", View { zoom: 2.0, ..v }, 1, 1.0),
        ("turned", View { rotation: 90.0, ..v }, 1, 1.0),
        ("fitted or moved", View { center: Point::new(150.0, 120.0), ..v }, 1, 1.0),
        ("another document", v, 2, 1.0),
        ("another display scale", v, 1, 1.25),
    ];
    for (name, before, doc, ppp) in changed {
        let mut after = before;
        let left = scroll_by(&mut after, doc, ppp, vec2(0.4 / ppp, -0.3 / ppp), rest);
        assert_eq!(after.center, before.center, "{name}: the first scroll's part is dropped");
        assert!((left.x - 0.4).abs() < 1e-6 && (left.y + 0.3).abs() < 1e-6, "{name}: {left:?}");
    }
}

/// View › Snap to Glyph toggles through the View menu command (#3124).
#[test]
fn snap_to_glyph_menu_command_toggles_the_view_flag() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    assert!(!app.ui.view.snap_to_glyph);
    app.run("view.snapToGlyph", json!({})).unwrap();
    assert!(app.ui.view.snap_to_glyph);
    app.run("view.snapToGlyph", json!({})).unwrap();
    assert!(!app.ui.view.snap_to_glyph);
}

/// Smart Guides › Object Highlighting (#394): the outline of the object under the pointer
/// shows with Smart Guides on and the option on, and not otherwise.
#[test]
fn object_highlighting_preference_hides_the_hover_outline() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.none", &json!({})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let rect = app.canvas_rect.unwrap();
    let over = Xf::new(rect, app.view().unwrap()).to_screen(Point::new(150.0, 150.0));
    let layer = egui::epaint::ColorMode::Solid(c32(app.session.active().unwrap().doc.layer_color(vectorcraft_doc::NodeId(id))));
    // The hover outline: 1.5 px lines in the layer colour.
    let outlines = |app: &mut VectorcraftApp| {
        frame(app, &ctx, vec![egui::Event::PointerMoved(over)]);
        shapes(app, &ctx).iter().filter(|s| matches!(s, Shape::Path(ps) if ps.stroke.width == 1.5 && ps.stroke.color == layer)).count()
    };
    assert!(outlines(&mut app) > 0, "highlighted by default");
    app.session.execute("prefs.set", &json!({"key": "objectHighlighting", "value": false})).unwrap();
    assert_eq!(outlines(&mut app), 0, "the option off");
    app.session.execute("prefs.set", &json!({"key": "objectHighlighting", "value": true})).unwrap();
    app.ui.view.smart_guides = false;
    assert_eq!(outlines(&mut app), 0, "Smart Guides off");
}

/// Hide Corner Widget for angles greater than (#394): a rectangle's 90° corners lose their
/// widgets below 90°.
#[test]
fn corner_widgets_hide_above_the_preference_angle() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
    let ctx = egui::Context::default();
    let widgets = |app: &mut VectorcraftApp| {
        shapes(app, &ctx).iter().filter(|s| matches!(s, Shape::Circle(c) if c.radius == 3.0 && c.fill == Color32::WHITE)).count()
    };
    assert_eq!(widgets(&mut app), 4);
    app.session.execute("prefs.set", &json!({"key": "hideCornerWidgetAbove", "value": 80})).unwrap();
    assert_eq!(widgets(&mut app), 0);
}

/// View › Hide Edges hides a live rectangle's corner widgets, and the Selection tool then
/// neither shows the corner cursor over one's place nor rounds the corners from it. With
/// edges shown, a drag from the same widget rounds them.
#[test]
fn hidden_corner_widgets_do_not_answer_the_pointer() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    let id = vectorcraft_doc::NodeId(id);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    app.select_tool("selection");
    let widget = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        vectorcraft_tools::corners::CornerWidgets::of(&st.doc, &st.selection, app.view_info().zoom, false).unwrap().visible().next().unwrap()
    };
    let radius = |app: &VectorcraftApp| {
        let c = vectorcraft_doc::LiveCorners::of(app.session.active().unwrap().doc.node(id).unwrap()).unwrap();
        c.style(&c.all()).0
    };
    let drag = |app: &mut VectorcraftApp, from: Point| {
        let (view, to) = (app.view_info(), from + vectorcraft_geom::Vec2::new(10.0, 10.0));
        for (kind, at) in [(PointerKind::Down, from), (PointerKind::Drag, to), (PointerKind::Up, to)] {
            app.session.pointer(&PointerEvent::new(kind, at.x, at.y), view).unwrap();
        }
    };
    app.ui.view.edges = false;
    let at = widget(&app);
    let view = app.view_info();
    assert_ne!(app.session.cursor(at, Mods::default(), view), Cursor::CornerRadius);
    drag(&mut app, at);
    assert_eq!(radius(&app), Some(0.0), "a press on a hidden widget does not round the corners");
    app.ui.view.edges = true;
    let at = widget(&app);
    let view = app.view_info();
    assert_eq!(app.session.cursor(at, Mods::default(), view), Cursor::CornerRadius);
    drag(&mut app, at);
    assert!(radius(&app).is_some_and(|r| r > 0.0), "{:?}", radius(&app));
}

/// View › Hide Edges hides the bounding box and the type widget beside it, and the Selection
/// tool then answers neither: a double-click on the widget's place leaves point type as it is,
/// and a drag from a corner handle's place does not scale. With edges shown, both answer.
#[test]
fn a_hidden_bounding_box_and_type_widget_do_not_answer_the_pointer() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 100, "y": 100, "text": "Type"})).unwrap()["id"].as_u64().unwrap();
    let id = vectorcraft_doc::NodeId(id);
    app.session.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    app.select_tool("selection");
    let bx = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        st.doc.oriented_bounds(&st.selection.objects, app.session.prefs.use_preview_bounds).unwrap()
    };
    let widget = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        vectorcraft_tools::typewidget::TypeWidget::of(&st.doc, &st.selection, &bx(app), app.view_info().zoom).unwrap()
    };
    let area = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => matches!(t.kind, vectorcraft_doc::TextKind::Area { .. }),
        k => panic!("not type: {k:?}"),
    };
    let double_click = |app: &mut VectorcraftApp, at: Point| {
        let view = app.view_info();
        for kind in [PointerKind::Down, PointerKind::Up, PointerKind::Down, PointerKind::Up, PointerKind::DoubleClick] {
            app.session.pointer(&PointerEvent::new(kind, at.x, at.y), view).unwrap();
        }
    };
    let width = |app: &VectorcraftApp| bx(app).rect.width();
    let scale = |app: &mut VectorcraftApp| {
        let b = bx(app);
        let corner = b.to_doc() * Point::new(b.rect.x1, b.rect.y1);
        let (view, to) = (app.view_info(), corner + vectorcraft_geom::Vec2::new(30.0, 0.0));
        for (kind, at) in [(PointerKind::Down, corner), (PointerKind::Drag, to), (PointerKind::Up, to)] {
            app.session.pointer(&PointerEvent::new(kind, at.x, at.y), view).unwrap();
        }
        app.session.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    };
    let w = widget(&app);
    app.ui.view.edges = false;
    let view = app.view_info();
    assert_ne!(app.session.cursor(w.at, Mods::default(), view), Cursor::TypeWidget);
    double_click(&mut app, w.at);
    assert!(!area(&app), "a double-click on the hidden widget leaves point type");
    app.session.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let before = width(&app);
    scale(&mut app);
    assert!((width(&app) - before).abs() < 1e-6, "a drag from a hidden handle does not scale: {before} -> {}", width(&app));
    app.ui.view.edges = true;
    let w = widget(&app);
    let view = app.view_info();
    assert_eq!(app.session.cursor(w.at, Mods::default(), view), Cursor::TypeWidget);
    scale(&mut app);
    assert!(width(&app) > before + 1.0, "a drag from the handle scales: {before} -> {}", width(&app));
    let w = widget(&app);
    double_click(&mut app, w.at);
    assert!(area(&app), "a double-click on the widget converts to area type");
}

/// View → Show Live Paint Gaps draws the gaps of each selected Live Paint group in the group's
/// Gap Preview Color, and nothing for a group or path that is not a selected Live Paint group;
/// Hide Live Paint Gaps and Hide Edges hide them.
#[test]
fn show_live_paint_gaps_draws_the_selected_groups_gaps_in_their_preview_color() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    // Open paths round a square, each ending 2 pt short of its start: one Small Gap each.
    let square = |app: &mut VectorcraftApp, x: i64| {
        let anchors = [(0, 0), (100, 0), (100, 100), (0, 100), (0, 2)].map(|(dx, y)| json!({"x": x + dx, "y": y}));
        app.session.execute("path.create", &json!({ "anchors": anchors })).unwrap()["id"].as_u64().unwrap()
    };
    let mut groups = vec![];
    for (x, color) in [(0, "#123456"), (150, "#654321")] {
        let p = square(&mut app, x);
        app.session.execute("select.set", &json!({"ids": [p]})).unwrap();
        let g = app.session.execute("livePaint.make", &json!({})).unwrap()["id"].as_u64().unwrap();
        app.session.execute("livePaint.gapOptions", &json!({"id": g, "color": color})).unwrap();
        groups.push(g);
    }
    let plain = square(&mut app, 300);
    let ctx = egui::Context::default();
    let gaps = |app: &mut VectorcraftApp, rgb: [u8; 3]| {
        let solid = egui::epaint::ColorMode::Solid(c32(rgb));
        shapes(app, &ctx).iter().filter(|s| matches!(s, Shape::Path(ps) if ps.stroke.color == solid && ps.stroke.width == 2.0)).count()
    };
    let (first, second) = ([0x12, 0x34, 0x56], [0x65, 0x43, 0x21]);
    // The color the plain path's gaps would be drawn in.
    let plain_color = vectorcraft_doc::GapOptions::default().color;
    app.session.execute("select.set", &json!({"ids": [groups[0], plain]})).unwrap();
    assert_eq!((gaps(&mut app, first), gaps(&mut app, second)), (0, 0), "off until the item is chosen");
    // View → Show Live Paint Gaps runs the command.
    let tree = crate::menus::menu_tree();
    let (_, view) = tree.iter().find(|(t, _)| *t == "View").unwrap();
    assert!(view.iter().any(|i| matches!(i, crate::menus::Item::Cmd("Show Live Paint Gaps", "view.livePaintGaps", _))), "{view:?}");
    crate::menus::invoke(&mut app, "view.livePaintGaps", json!({}));
    assert!(app.ui.view.live_paint_gaps);
    assert_eq!(
        (gaps(&mut app, first), gaps(&mut app, second), gaps(&mut app, plain_color)),
        (1, 0, 0),
        "the selected group only; the plain path shows none"
    );
    app.session.execute("select.set", &json!({"ids": [groups[1]]})).unwrap();
    assert_eq!((gaps(&mut app, first), gaps(&mut app, second)), (0, 1));
    // The lines follow the group's Gap Preview Color and Paint Stops At size.
    app.session.execute("livePaint.gapOptions", &json!({"id": groups[1], "color": "#123456"})).unwrap();
    assert_eq!((gaps(&mut app, first), gaps(&mut app, second)), (1, 0));
    app.session.execute("livePaint.gapOptions", &json!({"id": groups[1], "size": 1})).unwrap();
    assert_eq!(gaps(&mut app, first), 0, "a 2 pt gap is wider than 1 pt");
    app.session.execute("livePaint.gapOptions", &json!({"id": groups[1], "size": "small"})).unwrap();
    app.ui.view.edges = false;
    assert_eq!(gaps(&mut app, first), 0, "Hide Edges hides them");
    app.ui.view.edges = true;
    assert_eq!(gaps(&mut app, first), 1);
    app.run("view.livePaintGaps", json!({})).unwrap();
    assert!(!app.ui.view.live_paint_gaps);
    assert_eq!(gaps(&mut app, first), 0, "View → Hide Live Paint Gaps");
    // The setting is kept with the interface state; a state saved before it existed loads with it off.
    app.ui.view.live_paint_gaps = true;
    let mut saved = serde_json::to_value(&app.ui).unwrap();
    assert_eq!(saved["view"]["live_paint_gaps"], json!(true));
    saved["view"].as_object_mut().unwrap().remove("live_paint_gaps");
    assert!(!serde_json::from_value::<crate::UiState>(saved).unwrap().view.live_paint_gaps);
}

/// #511: a star shows a widget in each of its ten corners with Direct Selection (none with
/// the Selection tool: it isn't a live shape); View → Hide Corner Widget hides them.
#[test]
fn a_star_shows_corner_widgets_with_direct_selection() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.star", &json!({"cx": 200, "cy": 150, "radius1": 80, "radius2": 40})).unwrap();
    let ctx = egui::Context::default();
    let widgets = |app: &mut VectorcraftApp| {
        shapes(app, &ctx).iter().filter(|s| matches!(s, Shape::Circle(c) if c.radius == 3.0 && c.fill == Color32::WHITE)).count()
    };
    app.select_tool("selection");
    assert_eq!(widgets(&mut app), 0);
    app.select_tool("directSelection");
    assert_eq!(widgets(&mut app), 10);
    app.run("view.cornerWidget", json!({})).unwrap();
    assert_eq!(widgets(&mut app), 0);
}

/// The type widget beside selected type's bounding box: hollow (white) on point type, filled
/// in the layer colour on area type.
#[test]
fn the_type_widget_shows_which_kind_the_type_is() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 100, "y": 100, "text": "Type"})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    let layer = c32(app.session.active().unwrap().doc.layer_color(vectorcraft_doc::NodeId(id)));
    let ctx = egui::Context::default();
    let widget = |app: &mut VectorcraftApp| -> Vec<Color32> {
        let r = vectorcraft_tools::typewidget::RADIUS_PX;
        shapes(app, &ctx)
            .iter()
            .filter_map(|s| if let Shape::Circle(c) = s { (c.radius == r && c.fill != Color32::TRANSPARENT).then_some(c.fill) } else { None })
            .collect()
    };
    assert_eq!(widget(&mut app), vec![Color32::WHITE]);
    app.session.execute("type.convertToAreaType", &json!({})).unwrap();
    assert_eq!(widget(&mut app), vec![layer]);
    app.ui.view.bounding_box = false;
    assert!(widget(&mut app).is_empty(), "no bounding box, no widget");
}

/// A selected live ellipse shows its two pie widgets with the Selection tool while the
/// bounding box shows.
#[test]
fn a_selected_ellipse_shows_its_pie_widgets() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.ellipse", &json!({"x": 100, "y": 100, "width": 200, "height": 100})).unwrap();
    app.session.execute("object.setLiveShape", &json!({"pieStart": 0, "pieEnd": 90})).unwrap();
    let ctx = egui::Context::default();
    let widgets = |app: &mut VectorcraftApp| {
        let r = vectorcraft_tools::pie::RADIUS_PX;
        shapes(app, &ctx).iter().filter(|s| matches!(s, Shape::Circle(c) if c.radius == r && c.fill == Color32::WHITE)).count()
    };
    app.select_tool("selection");
    assert_eq!(widgets(&mut app), 2);
    app.select_tool("directSelection");
    assert_eq!(widgets(&mut app), 0);
    app.select_tool("selection");
    app.run("view.boundingBox", json!({})).unwrap();
    assert_eq!(widgets(&mut app), 0, "no bounding box, no widgets");
}

/// The shapes Type → Show Hidden Characters draws in one frame, seen through `xf`.
fn hidden_chars(app: &mut VectorcraftApp, ctx: &egui::Context, xf: &Xf) -> Vec<Shape> {
    let raw = egui::RawInput { screen_rect: Some(xf.rect), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| hidden_chars_overlay(app, ui.painter(), xf));
    out.textures_delta.clear();
    out.shapes.into_iter().map(|c| c.shape).collect()
}

/// Show Hidden Characters lays out type once: a frame keeps the marks of the frame before, and
/// after an edit only the object edited is laid out again.
#[test]
fn show_hidden_characters_lays_out_only_type_that_changed() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let ids: Vec<_> = (0..3)
        .map(|i| app.session.execute("text.create", &json!({"x": 20, "y": 40 + 80 * i, "text": "a b\nc d"})).unwrap()["id"].as_u64().unwrap())
        .map(vectorcraft_doc::NodeId)
        .collect();
    let ctx = egui::Context::default();
    let xf = Xf { rect: egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)), zoom: 1.0, center: Point::new(200.0, 150.0), rot: 0.0 };
    // `frame` draws one frame and returns each object's marks and whether that frame found the
    // fonts and hyphenation exceptions of the frame before.
    let frame = |app: &mut VectorcraftApp| {
        let inputs = app.canvas.hidden_chars.inputs.clone();
        hidden_chars(app, &ctx, &xf);
        let kept = &app.canvas.hidden_chars;
        (kept.inputs == inputs, ids.iter().map(|id| kept.marks[id].1.clone()).collect::<Vec<_>>())
    };
    // `frames` draws a frame, runs `edit` and draws the next frame. Tests running alongside add
    // fonts and set hyphenation exceptions, which lays all type out again: `frames` draws both
    // frames again until neither the fonts nor the exceptions changed in between.
    let frames = |app: &mut VectorcraftApp, edit: &dyn Fn(&mut VectorcraftApp)| {
        (0..100)
            .find_map(|_| {
                let (_, before) = frame(app);
                edit(app);
                let (same, after) = frame(app);
                same.then_some((before, after))
            })
            .unwrap()
    };
    let same = |a: &std::sync::Arc<HiddenMarks>, b: &std::sync::Arc<HiddenMarks>| std::sync::Arc::ptr_eq(a, b);
    let (before, after) = frames(&mut app, &|_| {});
    assert!(before.iter().zip(&after).all(|(a, b)| same(a, b)), "a frame lays out no type");
    let (before, after) = frames(&mut app, &|app| {
        app.session.execute("text.setText", &json!({"id": ids[1].0, "text": "a b c"})).unwrap();
    });
    assert!(!same(&before[1], &after[1]), "the edited object is laid out again");
    assert!(same(&before[0], &after[0]) && same(&before[2], &after[2]), "the others are not");
}

/// The marks Show Hidden Characters keeps follow the type: after an edit, a zoom and a font
/// that arrives, a frame draws what a new overlay draws, and the marks moved.
#[test]
fn show_hidden_characters_follows_edits_zooms_and_fonts() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    // The type is set in Bold of a family that is not installed and whose name starts with a
    // dot; the font menus do not list such names. Source Sans 3 Bold stands in for it until the
    // family arrives below with a Regular face only.
    let text = json!({"x": 20, "y": 60, "text": "one two\nthree four", "font": ".Hidden Marks", "style": "Bold"});
    let id = app.session.execute("text.create", &text).unwrap()["id"].clone();
    let ctx = egui::Context::default();
    let mut xf = Xf { rect: egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)), zoom: 1.0, center: Point::new(200.0, 150.0), rot: 0.0 };
    // `drawn` draws a frame with what the overlay kept and checks it against a new overlay.
    let drawn = |app: &mut VectorcraftApp, xf: &Xf| {
        let kept = hidden_chars(app, &ctx, xf);
        app.canvas.hidden_chars = Default::default();
        assert_eq!(hidden_chars(app, &ctx, xf), kept, "as a new overlay draws them");
        kept
    };
    let start = drawn(&mut app, &xf);
    assert!(start.len() > 2, "{start:?}");
    app.session.execute("text.setText", &json!({"id": id, "text": "one two three\nfour"})).unwrap();
    let edited = drawn(&mut app, &xf);
    assert_ne!(edited, start, "the edit moved the marks");
    xf.zoom = 2.0;
    let zoomed = drawn(&mut app, &xf);
    assert_ne!(zoomed, edited, "zoomed in");
    assert!(vectorcraft_text::FontDb::global().add_font(vectorcraft_testkit::fonts::renamed(".Hidden Marks")) > 0, "the family is new");
    assert_ne!(drawn(&mut app, &xf), zoomed, "set in the family that arrived");
}

/// In a rotated view Show Hidden Characters draws the marks of type whose turned outline reaches
/// into the canvas, also when the box between two of its corners on screen does not.
#[test]
fn show_hidden_characters_draws_type_partly_on_a_rotated_canvas() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 20, "y": 60, "text": "a b c d e\nf g h"})).unwrap()["id"].as_u64().unwrap();
    let b = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap();
    let ctx = egui::Context::default();
    let mut xf = Xf { rect: egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)), zoom: 2.0, center: b.center(), rot: 45f64.to_radians() };
    // The screen moves so that the canvas's left edge falls halfway between the right side of
    // the box and the outline's rightmost corner, level with that corner.
    let q = xf.quad(b);
    let right = q.iter().copied().max_by(|a, c| a.x.total_cmp(&c.x)).unwrap();
    let shift = vec2(-(xf.rect_to_screen(b).right() + right.x) / 2.0, xf.rect.center().y - right.y);
    xf.center -= xf.delta_to_doc(shift);
    assert!(!xf.rect_to_screen(b).intersects(xf.rect), "the box is off the canvas");
    assert!(xf.quad(b).iter().any(|c| xf.rect.contains(*c)), "a corner is on it");
    assert!(!hidden_chars(&mut app, &ctx, &xf).is_empty(), "the marks are drawn");
}

/// The shapes Type → Show Hidden Characters draws in one canvas frame ([`show`]).
fn hidden_char_marks(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<Shape> {
    let blue = Color32::from_rgb(0x4f, 0x9d, 0xff);
    shapes(app, ctx)
        .into_iter()
        .filter(|s| matches!(s, Shape::Circle(c) if c.fill == blue) || matches!(s, Shape::Text(t) if t.fallback_color == blue))
        .collect()
}

/// Show Hidden Characters puts a dot over each space of a list paragraph and none over its
/// bullet, which stands for no character.
#[test]
fn hidden_characters_mark_no_space_at_a_bullet() {
    let mut t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::ZERO, " Item two", vectorcraft_doc::CharStyle::default());
    let plain = HiddenMarks::of(&t).spaces.len();
    t.para.list = Some(vectorcraft_doc::text::ParaList::bulleted(vectorcraft_doc::text::Bullet::Dot));
    assert_eq!((plain, HiddenMarks::of(&t).spaces.len()), (2, 2));
}

/// While Show Hidden Characters is on, a canvas frame keeps the marks of the frame before.
/// Turning it off drops the marks with their text nodes, and turning it on again draws the
/// same marks.
#[test]
fn show_hidden_characters_off_releases_the_text_nodes() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let text = json!({"x": 20, "y": 60, "text": "one two\nthree four"});
    let id = vectorcraft_doc::NodeId(app.session.execute("text.create", &text).unwrap()["id"].as_u64().unwrap());
    let ctx = egui::Context::default();
    app.run("type.hiddenCharacters", json!({})).unwrap();
    let on = hidden_char_marks(&mut app, &ctx);
    assert!(on.len() > 2, "{on:?}");
    // Tests running alongside add fonts and set hyphenation exceptions, which lays all type out
    // again: frames are drawn until neither changed between two of them.
    let kept = (0..100).any(|_| {
        let (inputs, marks) = (app.canvas.hidden_chars.inputs.clone(), app.canvas.hidden_chars.marks[&id].1.clone());
        hidden_char_marks(&mut app, &ctx);
        let after = &app.canvas.hidden_chars;
        after.inputs == inputs && std::sync::Arc::ptr_eq(&after.marks[&id].1, &marks)
    });
    assert!(kept, "a frame keeps the marks of the frame before");
    app.run("type.hiddenCharacters", json!({})).unwrap();
    assert!(hidden_char_marks(&mut app, &ctx).is_empty(), "off");
    assert!(app.canvas.hidden_chars.marks.is_empty(), "no node is kept");
    app.run("type.hiddenCharacters", json!({})).unwrap();
    assert_eq!(hidden_char_marks(&mut app, &ctx), on, "on again: the same marks");
}

/// Closing the document drops the marks Show Hidden Characters kept with its text nodes, and
/// the same type in a new document shows the same marks.
#[test]
fn show_hidden_characters_releases_the_text_nodes_of_a_closed_document() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    app.run("type.hiddenCharacters", json!({})).unwrap();
    let open = |app: &mut VectorcraftApp| {
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.session.execute("text.create", &json!({"x": 20, "y": 60, "text": "one two\nthree four"})).unwrap();
        hidden_char_marks(app, &ctx)
    };
    let first = open(&mut app);
    assert!(first.len() > 2, "{first:?}");
    assert_eq!(app.canvas.hidden_chars.marks.len(), 1, "the marks are kept with the text object's node");
    app.session.execute("file.close", &json!({})).unwrap();
    assert!(hidden_char_marks(&mut app, &ctx).is_empty(), "the Home screen");
    assert!(app.canvas.hidden_chars.marks.is_empty(), "no node of the closed document is kept");
    assert_eq!(open(&mut app), first, "the same type in a new document");
}

/// General › Zoom with Mouse Wheel (#394), through the control channel's `ui.wheel`: off,
/// the wheel scrolls and Cmd-wheel zooms; on, the wheel zooms about the pointer, Shift-wheel
/// scrolls up and down and Cmd/Ctrl-wheel sideways. Alt-wheel (Option on the Mac) and a
/// trackpad pinch zoom about the pointer either way.
#[test]
fn zoom_with_mouse_wheel_preference() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let ctx = egui::Context::default();
    // A canvas frame through the host's input hook (it feeds the control channel's input).
    let run = |app: &mut VectorcraftApp| {
        let mut raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
        app.raw_input_hook(&ctx, &mut raw);
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
    };
    run(&mut app);
    let rect = app.canvas_rect.unwrap();
    let at = rect.center() + vec2(60.0, 40.0);
    // One notch up at `at` with `mods` held, or a pinch: → (zoom ratio, scroll in points, how
    // far the point under `at` moved).
    let turn = |app: &mut VectorcraftApp, mods: &str| {
        let before = *app.view().unwrap();
        if mods == "pinch" {
            app.synthetic.extend([egui::Event::PointerMoved(at), egui::Event::Zoom(1.25)]);
        } else {
            let mut p = json!({"x": at.x, "y": at.y, "dy": 1});
            for m in mods.split('+').filter(|m| !m.is_empty()) {
                p[m] = json!(true);
            }
            crate::tests_synthetic::control(app, &ctx, "ui.wheel", p);
        }
        // egui spreads a notch over a few frames.
        for _ in 0..40 {
            run(app);
        }
        let after = *app.view().unwrap();
        let moved = Xf::new(rect, &after).to_doc(at).distance(Xf::new(rect, &before).to_doc(at));
        (after.zoom / before.zoom, (after.center - before.center) * after.zoom, moved)
    };
    let zooms_in = |(zoom, _, moved): (f64, _, f64)| zoom > 1.01 && moved < 1e-6;
    let (zoom, scroll, _) = turn(&mut app, "");
    assert!((zoom - 1.0).abs() < 1e-9 && scroll.x == 0.0 && scroll.y < -1.0, "off: the wheel scrolls up ({zoom}, {scroll:?})");
    assert!(zooms_in(turn(&mut app, "cmd")), "off: Cmd-wheel zooms in about the pointer");
    assert!(zooms_in(turn(&mut app, "alt")), "off: Alt-wheel zooms in about the pointer");
    assert!(zooms_in(turn(&mut app, "pinch")), "off: a pinch zooms in about the pointer");
    app.session.execute("prefs.set", &json!({"key": "zoomWithMouseWheel", "value": true})).unwrap();
    assert!(zooms_in(turn(&mut app, "")), "on: the wheel zooms in about the pointer");
    assert!(zooms_in(turn(&mut app, "alt")), "on: so does Alt-wheel");
    assert!(zooms_in(turn(&mut app, "pinch")), "on: and a pinch");
    let (zoom, scroll, _) = turn(&mut app, "shift");
    assert!((zoom - 1.0).abs() < 1e-9 && scroll.x == 0.0 && scroll.y < -1.0, "on: Shift-wheel scrolls up ({zoom}, {scroll:?})");
    let (zoom, scroll, _) = turn(&mut app, "cmd");
    assert!((zoom - 1.0).abs() < 1e-9 && scroll.x < -1.0 && scroll.y == 0.0, "on: Cmd-wheel scrolls sideways ({zoom}, {scroll:?})");
}

/// #888: with Zoom with Mouse Wheel, a wheel notch's zoom glides in over a few frames, as a
/// scroll does, instead of jumping on the frame it arrives; it ends at the same zoom.
#[test]
fn wheel_notches_zoom_in_a_glide() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("prefs.set", &json!({"key": "zoomWithMouseWheel", "value": true})).unwrap();
    let ctx = egui::Context::default();
    let run = |app: &mut VectorcraftApp| {
        let mut raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
        app.raw_input_hook(&ctx, &mut raw);
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
    };
    run(&mut app);
    let at = app.canvas_rect.unwrap().center();
    let before = app.view().unwrap().zoom;
    crate::tests_synthetic::control(&mut app, &ctx, "ui.wheel", json!({"x": at.x, "y": at.y, "dy": 1}));
    // The first frame the zoom moves in (the event may land a frame later).
    let mut first = 1.0;
    for _ in 0..3 {
        run(&mut app);
        first = app.view().unwrap().zoom / before;
        if first != 1.0 {
            break;
        }
    }
    for _ in 0..60 {
        run(&mut app);
    }
    let done = app.view().unwrap().zoom / before;
    assert!(done > 1.01, "the notch zooms in ({done})");
    assert!(first > 1.0 && first < done, "the first frame takes only part of it ({first} of {done})");
    run(&mut app);
    assert_eq!(app.view().unwrap().zoom / before, done, "and it stops");
}

/// #4283: with Performance › Animated Zoom off, or on without GPU Performance (which it needs), a
/// wheel notch zooms the whole step on the frame it arrives, both the plain wheel with Zoom with
/// Mouse Wheel and Alt-wheel without it.
#[test]
fn wheel_notches_zoom_at_once_without_animated_zoom() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let ctx = egui::Context::default();
    let run = |app: &mut VectorcraftApp| {
        let mut raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
        app.raw_input_hook(&ctx, &mut raw);
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
    };
    run(&mut app);
    let at = app.canvas_rect.unwrap().center();
    for (animated, gpu, wheel_zooms, alt) in
        [(false, true, true, false), (false, true, false, true), (true, false, true, false), (true, false, false, true)]
    {
        app.session.execute("prefs.set", &json!({"key": "animatedZoom", "value": animated})).unwrap();
        app.session.execute("prefs.set", &json!({"key": "gpuPerformance", "value": gpu})).unwrap();
        app.session.execute("prefs.set", &json!({"key": "zoomWithMouseWheel", "value": wheel_zooms})).unwrap();
        // Any glide or spread scroll a case before left behind dies down first.
        for _ in 0..30 {
            run(&mut app);
        }
        let before = app.view().unwrap().zoom;
        crate::tests_synthetic::control(&mut app, &ctx, "ui.wheel", json!({"x": at.x, "y": at.y, "dy": 1, "alt": alt}));
        // One line notch's whole zoom.
        let expected = ctx.options(|o| f64::from(o.input_options.line_scroll_speed) * 0.01).exp();
        // The first frame the zoom moves (the event may land a frame later).
        let mut first = 1.0;
        for _ in 0..3 {
            run(&mut app);
            first = app.view().unwrap().zoom / before;
            if first != 1.0 {
                break;
            }
        }
        assert!(
            (first - expected).abs() < 1e-9,
            "the whole notch arrives at once (animated {animated}, gpu {gpu}, wheel zooms {wheel_zooms}, alt {alt}: expected {expected}, got {first})"
        );
        for _ in 0..30 {
            run(&mut app);
        }
        let done = app.view().unwrap().zoom / before;
        assert!(
            (done - first).abs() < 1e-9,
            "all of it on the first frame (animated {animated}, gpu {gpu}, wheel zooms {wheel_zooms}, alt {alt}: {first} then {done})"
        );
    }
}

/// In each frame where a wheel scroll moves the view, the hover point (read by the rulers'
/// pointer marks, the Info panel, the hover highlight and the tool cursor) is the document point
/// under the pointer in the view the canvas draws in that frame.
#[test]
fn the_hover_point_follows_a_wheel_scroll_in_the_frame_it_moves() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let rect = app.canvas_rect.unwrap();
    let at = rect.center() + vec2(60.0, 40.0);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
    let under = |app: &VectorcraftApp| Xf::new(rect, app.view().unwrap()).to_doc(at);
    let wheel = egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, -60.0),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
        source: egui::MouseWheelSource::Unknown,
    };
    let mut events = vec![wheel];
    let mut moved = 0;
    for k in 0..30 {
        let before = app.view().unwrap().center;
        frame(&mut app, &ctx, std::mem::take(&mut events));
        moved += usize::from(app.view().unwrap().center != before);
        let hover = app.hover_doc.unwrap();
        assert!(hover.distance(under(&app)) < 1e-9, "frame {k}: {hover:?}, under the pointer {:?}", under(&app));
    }
    assert!(moved > 0, "the wheel scrolls the view");
}

/// While a middle-button pan or a Rotate View drag moves the view, the hover point is the
/// document point under the pointer in the view the canvas draws in that frame.
#[test]
fn the_hover_point_follows_a_view_drag_in_the_frame_it_moves() {
    for (tool, button) in [("rectangle", egui::PointerButton::Middle), ("rotateView", egui::PointerButton::Primary)] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.select_tool(tool);
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let rect = app.canvas_rect.unwrap();
        let press = |pos, pressed| egui::Event::PointerButton { pos, button, pressed, modifiers: Default::default() };
        let a = rect.center() + vec2(100.0, 20.0);
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(a)]);
        frame(&mut app, &ctx, vec![press(a, true)]);
        let mut p = a;
        for k in 1..=3 {
            p = a + vec2(-20.0, 30.0) * k as f32;
            let before = *app.view().unwrap();
            frame(&mut app, &ctx, vec![egui::Event::PointerMoved(p)]);
            let after = *app.view().unwrap();
            assert!(after.center != before.center || after.rotation != before.rotation, "{tool}: the drag moves the view");
            let under = Xf::new(rect, &after).to_doc(p);
            let hover = app.hover_doc.unwrap();
            assert!(hover.distance(under) < 1e-9, "{tool}, step {k}: {hover:?}, under the pointer {under:?}");
        }
        frame(&mut app, &ctx, vec![press(p, false)]);
    }
}

/// A tool drag goes on at the document point under the pointer in the view a wheel scroll moved
/// to, in the frame the wheel scrolls.
#[test]
fn a_tool_drag_follows_a_wheel_scroll_in_the_frame_it_moves() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    // With Zoom with Mouse Wheel on, Shift-wheel scrolls up and down in the frame it comes in.
    app.session.execute("prefs.set", &json!({"key": "zoomWithMouseWheel", "value": true})).unwrap();
    app.ui.view.smart_guides = false;
    app.select_tool("rectangle");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let rect = app.canvas_rect.unwrap();
    let (a, b) = (rect.center() - vec2(80.0, 60.0), rect.center() + vec2(40.0, 30.0));
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(a)]);
    frame(&mut app, &ctx, vec![primary(a, true)]);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(b)]);
    let before = *app.view().unwrap();
    let wheel = egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, -40.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::SHIFT,
        source: egui::MouseWheelSource::Unknown,
    };
    frame(&mut app, &ctx, vec![wheel]);
    let after = *app.view().unwrap();
    assert!(after.center != before.center, "Shift-wheel scrolls the view");
    let want = Xf::new(rect, &after).to_doc(b);
    let bounds = |app: &VectorcraftApp| app.session.active().unwrap().doc.layers[0].children().unwrap()[0].geometric_bounds().unwrap();
    let r = bounds(&app);
    assert!((r.x1 - want.x).abs() < 1e-6 && ((r.y0 - want.y).abs() < 1e-6 || (r.y1 - want.y).abs() < 1e-6), "{r:?} reaches {want:?}");
    frame(&mut app, &ctx, vec![primary(b, false)]);
    assert_eq!(bounds(&app), r, "the rectangle drawn");
}

/// Two fingers dragged together on a touch screen pan the canvas with them, under either
/// Zoom with Mouse Wheel setting, and draw nothing (#449).
#[test]
fn two_fingers_dragged_together_pan_the_canvas() {
    for wheel_zooms in [false, true] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.session.execute("prefs.set", &json!({"key": "zoomWithMouseWheel", "value": wheel_zooms})).unwrap();
        app.select_tool("rectangle");
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let c = app.canvas_rect.unwrap().center();
        let touch = |n: u64, phase, pos| egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(n), phase, pos, force: None };
        let fingers = |phase, d: egui::Vec2| vec![touch(1, phase, c - vec2(40.0, 0.0) + d), touch(2, phase, c + vec2(40.0, 0.0) + d)];
        let before = *app.view().unwrap();
        // egui-winit moves the pointer with the first finger, as here.
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(c - vec2(40.0, 0.0))]);
        frame(&mut app, &ctx, fingers(egui::TouchPhase::Start, Vec2::ZERO));
        for k in 1..=3 {
            frame(&mut app, &ctx, fingers(egui::TouchPhase::Move, vec2(10.0, 8.0) * k as f32));
        }
        frame(&mut app, &ctx, fingers(egui::TouchPhase::End, vec2(30.0, 24.0)));
        let after = *app.view().unwrap();
        let moved = (after.center - before.center) * after.zoom;
        assert!((after.zoom / before.zoom - 1.0).abs() < 1e-9, "no zoom: {wheel_zooms}");
        assert!((moved.x + 30.0).abs() < 0.5 && (moved.y + 24.0).abs() < 0.5, "the content follows the fingers ({wheel_zooms}): {moved:?}");
        assert_eq!(app.session.active().unwrap().doc.layers[0].children().unwrap().len(), 0, "nothing drawn");
    }
}

/// #4198: egui-winit presses with the first finger as the pointer, in the frame the second lands
/// or a frame or more before (the Pencil has begun drawing, the Selection tool moving the art
/// under it). The second finger cancels that press, as it does the press of a finger lifted and
/// landed again mid-pinch: the pinch zooms, nothing is drawn or moved, nothing enters the undo
/// history, and lifting the fingers commits nothing. One finger still draws afterwards.
#[test]
fn a_second_finger_cancels_the_tool_gesture_the_first_began() {
    for (tool, together) in [("pencil", false), ("selection", false), ("pencil", true), ("selection", true)] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap();
        app.select_tool(tool);
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let (doc, undo, zoom) = {
            let st = app.session.active().unwrap();
            (st.doc.clone(), st.history.undo.len(), app.view().unwrap().zoom)
        };
        let touch = |n: u64, phase, pos| egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(n), phase, pos, force: None };
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let press = |n: u64, pos| vec![touch(n, egui::TouchPhase::Start, pos), egui::Event::PointerMoved(pos), button(pos, true)];
        let lift = |n: u64, pos| vec![touch(n, egui::TouchPhase::End, pos), button(pos, false), egui::Event::PointerGone];
        let apart = |p: Pos2, q: Pos2| vec![touch(1, egui::TouchPhase::Move, p), egui::Event::PointerMoved(p), touch(2, egui::TouchPhase::Move, q)];
        // The first finger presses on the rectangle, and drags a little on its own unless the
        // second lands in the same frame.
        let mut p = xf.to_screen(Point::new(100.0, 100.0));
        let mut q = p + vec2(80.0, 0.0);
        if together {
            frame(&mut app, &ctx, [press(1, p), vec![touch(2, egui::TouchPhase::Start, q)]].concat());
        } else {
            frame(&mut app, &ctx, press(1, p));
            for _ in 0..2 {
                p += vec2(12.0, 6.0);
                frame(&mut app, &ctx, vec![touch(1, egui::TouchPhase::Move, p), egui::Event::PointerMoved(p)]);
            }
            assert!(matches!(ctx.data(|d| d.get_temp::<Drag>(drag_id())), Some(Drag::Tool | Drag::Art)), "{tool}: the tool's drag began");
            q = p + vec2(80.0, 0.0);
            frame(&mut app, &ctx, vec![touch(2, egui::TouchPhase::Start, q)]);
        }
        // The two pinch apart; the first lifts, the second moves alone (egui-winit's pointer
        // now), and the first lands again and pinches on.
        for _ in 0..3 {
            (p, q) = (p - vec2(15.0, 0.0), q + vec2(15.0, 0.0));
            frame(&mut app, &ctx, apart(p, q));
        }
        frame(&mut app, &ctx, lift(1, p));
        q += vec2(10.0, 0.0);
        frame(&mut app, &ctx, vec![touch(2, egui::TouchPhase::Move, q), egui::Event::PointerMoved(q)]);
        frame(&mut app, &ctx, press(1, p));
        for _ in 0..3 {
            (p, q) = (p - vec2(10.0, 5.0), q + vec2(10.0, 5.0));
            frame(&mut app, &ctx, apart(p, q));
        }
        frame(&mut app, &ctx, lift(1, p));
        frame(&mut app, &ctx, lift(2, q));
        frame(&mut app, &ctx, vec![]);
        let st = app.session.active().unwrap();
        assert!(st.interaction.is_none(), "{tool}: no tool interaction is left");
        assert_eq!(*st.doc, *doc, "{tool}: nothing drawn or moved");
        assert_eq!(st.history.undo.len(), undo, "{tool}: no undo entry");
        assert!(app.view().unwrap().zoom > zoom * 1.2, "{tool}: the pinch zoomed in");
        if tool == "pencil" {
            // One finger alone still draws.
            let mut p = app.canvas_rect.unwrap().center() - vec2(30.0, 20.0);
            frame(&mut app, &ctx, press(3, p));
            for _ in 0..4 {
                p += vec2(15.0, 10.0);
                frame(&mut app, &ctx, vec![touch(3, egui::TouchPhase::Move, p), egui::Event::PointerMoved(p)]);
            }
            frame(&mut app, &ctx, lift(3, p));
            let children = app.session.active().unwrap().doc.layers[0].children().unwrap().len();
            assert_eq!(children, 2, "one finger draws with the Pencil");
        }
    }
}

/// A canvas frame at input time `t` (seconds) with `events`, for the trackpad rotation tests.
fn timed_frame(app: &mut VectorcraftApp, ctx: &egui::Context, t: f64, events: Vec<egui::Event>) {
    let raw =
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), time: Some(t), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| show(app, ui));
    out.textures_delta.clear();
}

/// General › Trackpad Gesture to Rotate View, on by default: a two-finger rotation on a Mac
/// trackpad (egui's `Event::Rotate`, clockwise radians) turns the view about the pointer, also
/// in the frame of a pinch, which zooms about the same point. With the preference off, the view
/// does not turn.
#[test]
fn a_trackpad_rotation_turns_the_view_about_the_pointer() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let ctx = egui::Context::default();
    timed_frame(&mut app, &ctx, 0.0, vec![]);
    let rect = app.canvas_rect.unwrap();
    let at = rect.center() + vec2(60.0, 40.0);
    timed_frame(&mut app, &ctx, 0.1, vec![egui::Event::PointerMoved(at)]);
    let under = |app: &VectorcraftApp| Xf::new(rect, app.view().unwrap()).to_doc(at);
    let before = (*app.view().unwrap(), under(&app));
    timed_frame(&mut app, &ctx, 0.2, vec![egui::Event::Rotate(0.25)]);
    let after = *app.view().unwrap();
    assert!((after.rotation - before.0.rotation - 0.25f64.to_degrees()).abs() < 1e-4, "turned by 14.32°: {}", after.rotation);
    assert!((after.zoom - before.0.zoom).abs() < 1e-12, "no zoom");
    assert!(under(&app).distance(before.1) < 1e-6, "the point under the pointer stays: {:?}", under(&app));
    // A pinch and a rotation in one frame, in a new gesture.
    let before = (*app.view().unwrap(), under(&app));
    timed_frame(&mut app, &ctx, 1.0, vec![egui::Event::Zoom(1.25), egui::Event::Rotate(0.25)]);
    let after = *app.view().unwrap();
    assert!((after.zoom / before.0.zoom - 1.25).abs() < 1e-6, "the pinch zooms: {}", after.zoom / before.0.zoom);
    assert!((after.rotation - before.0.rotation - 0.25f64.to_degrees()).abs() < 1e-4, "and the rotation turns: {}", after.rotation);
    assert!(under(&app).distance(before.1) < 1e-6, "about the same point: {:?}", under(&app));
    app.session.execute("prefs.set", &json!({"key": "trackpadRotateView", "value": false})).unwrap();
    let before = *app.view().unwrap();
    timed_frame(&mut app, &ctx, 2.0, vec![egui::Event::Rotate(0.25)]);
    timed_frame(&mut app, &ctx, 2.1, vec![egui::Event::Rotate(0.25)]);
    let after = *app.view().unwrap();
    assert!(after.rotation == before.rotation && after.center == before.center, "off: the view does not turn ({})", after.rotation);
}

/// A trackpad rotation turns the view only once it reaches 5°, so a pinch that twists a little
/// only zooms. It then turns the view by all of it and follows each later rotation event, until
/// a quarter of a second passes without one.
#[test]
fn a_trackpad_rotation_turns_nothing_until_it_reaches_five_degrees() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let ctx = egui::Context::default();
    timed_frame(&mut app, &ctx, 0.0, vec![]);
    let at = app.canvas_rect.unwrap().center() + vec2(60.0, 40.0);
    timed_frame(&mut app, &ctx, 0.1, vec![egui::Event::PointerMoved(at)]);
    let rotate = |deg: f32| egui::Event::Rotate(deg.to_radians());
    let zoom = app.view().unwrap().zoom;
    timed_frame(&mut app, &ctx, 1.0, vec![egui::Event::Zoom(1.25), rotate(1.0)]);
    assert!(app.view().unwrap().zoom > zoom * 1.2, "the pinch zooms");
    timed_frame(&mut app, &ctx, 1.1, vec![egui::Event::Zoom(1.25), rotate(1.0)]);
    timed_frame(&mut app, &ctx, 1.2, vec![egui::Event::Zoom(1.25), rotate(1.0)]);
    assert_eq!(app.view().unwrap().rotation, 0.0, "3° in all does not turn the view");
    timed_frame(&mut app, &ctx, 1.3, vec![rotate(3.0)]);
    assert!((app.view().unwrap().rotation - 6.0).abs() < 1e-4, "6° in all turns by 6°: {}", app.view().unwrap().rotation);
    timed_frame(&mut app, &ctx, 1.4, vec![rotate(-1.0)]);
    assert!((app.view().unwrap().rotation - 5.0).abs() < 1e-4, "then each event turns it: {}", app.view().unwrap().rotation);
    timed_frame(&mut app, &ctx, 2.0, vec![rotate(3.0)]);
    assert!((app.view().unwrap().rotation - 5.0).abs() < 1e-4, "a new rotation starts under 5° again: {}", app.view().unwrap().rotation);
}

/// Rotation events that come while two fingers are on a touch screen (Safari on an iPad sends
/// them with the touches) do not turn the view.
#[test]
fn a_rotation_with_fingers_on_a_touch_screen_turns_nothing() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let ctx = egui::Context::default();
    timed_frame(&mut app, &ctx, 0.0, vec![]);
    let c = app.canvas_rect.unwrap().center();
    let touch = |n: u64, phase, pos| egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(n), phase, pos, force: None };
    timed_frame(&mut app, &ctx, 0.1, vec![egui::Event::PointerMoved(c - vec2(40.0, 0.0))]);
    let before = app.view().unwrap().rotation;
    let start = vec![touch(1, egui::TouchPhase::Start, c - vec2(40.0, 0.0)), touch(2, egui::TouchPhase::Start, c + vec2(40.0, 0.0))];
    timed_frame(&mut app, &ctx, 0.2, [start, vec![egui::Event::Rotate(0.25)]].concat());
    assert_eq!(app.view().unwrap().rotation, before, "the view does not turn");
}

/// General › Display Print Size at 100% Zoom (#394): `view.actualSize` is one document point per
/// screen point with it off, and 96 screen points per document inch with it on.
#[test]
fn view_actual_size_follows_display_print_size() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    app.canvas_rect = Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(460.0, 260.0)));
    app.run("view.setZoom", json!({"zoom": 250})).unwrap();
    app.run("view.actualSize", json!({})).unwrap();
    assert!((app.view().unwrap().zoom - 1.0).abs() < 1e-12, "off by default: 100% is 1:1");
    app.session.execute("prefs.set", &json!({"key": "displayPrintSize", "value": true})).unwrap();
    app.run("view.actualSize", json!({})).unwrap();
    assert!((app.view().unwrap().zoom - 96.0 / 72.0).abs() < 1e-12, "an inch is 96 screen points");
}

/// Fit Artboard in Window fits the artboard as a rotated view shows it: at 45° every corner is
/// on the canvas and one is within the 30-point margin of an edge. At 0° the zoom is unchanged.
#[test]
fn fit_artboard_fits_the_artboard_turned_with_the_view() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 612, "height": 792})).unwrap();
    let rect = egui::Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0));
    app.canvas_rect = Some(rect);
    app.run("view.fitArtboard", json!({})).unwrap();
    assert!((app.view().unwrap().zoom - (940.0 / 612.0_f64).min(740.0 / 792.0)).abs() < 1e-12, "0°");
    app.run("view.rotate", json!({"angle": 45})).unwrap();
    app.run("view.fitArtboard", json!({})).unwrap();
    let board = app.session.active().unwrap().doc.artboards[0].rect;
    let q = Xf::new(rect, app.view().unwrap()).quad(board);
    assert!(q.iter().all(|c| rect.contains(*c)), "the turned artboard is on the canvas: {q:?}");
    let gap =
        q.iter().map(|c| (c.x - rect.left()).min(rect.right() - c.x).min(c.y - rect.top()).min(rect.bottom() - c.y)).fold(f32::INFINITY, f32::min);
    assert!(gap < 31.0, "and fills it but for the margin: {gap}");
}

/// Performance › Animated Zoom (#394): the Zoom tool dragged sideways zooms about where it was
/// pressed (right in, left out), held still it zooms on, and a click still steps. Off, or with
/// GPU Performance off, a drag zooms to the area dragged across.
#[test]
fn animated_zoom_scrubs_and_holds_else_the_zoom_tool_zooms_to_an_area() {
    use egui::Event;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.select_tool("zoom");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let rect = app.canvas_rect.unwrap();
    let at = rect.center() - vec2(100.0, 50.0);
    let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    // Press at `at`, move by `by` and hold for `frames` frames (a 60th of a second each), let go.
    let press = |app: &mut VectorcraftApp, by: egui::Vec2, frames: usize| {
        frame(app, &ctx, vec![Event::PointerMoved(at)]);
        frame(app, &ctx, vec![button(at, true)]);
        frame(app, &ctx, vec![Event::PointerMoved(at + by)]);
        for _ in 0..frames {
            frame(app, &ctx, vec![]);
        }
        frame(app, &ctx, vec![button(at + by, false)]);
    };
    let doc_at = |app: &VectorcraftApp, p: Pos2| Xf::new(rect, app.view().unwrap()).to_doc(p);
    let pinned = doc_at(&app, at);
    let z0 = app.view().unwrap().zoom;
    press(&mut app, vec2(100.0, 0.0), 0);
    let z1 = app.view().unwrap().zoom;
    assert!((z1 / z0 - 2.0).abs() < 1e-6, "100 points right doubles the zoom: {z0} → {z1}");
    assert!(doc_at(&app, at).distance(pinned) < 1e-6, "about the press");
    press(&mut app, vec2(-100.0, 0.0), 0);
    assert!((app.view().unwrap().zoom - z0).abs() < 1e-6, "and back to the left");
    press(&mut app, vec2(0.0, 0.0), 0);
    assert_eq!(app.view().unwrap().zoom, crate::state::next_zoom(z0, true), "a click steps");
    let z2 = app.view().unwrap().zoom;
    press(&mut app, vec2(0.0, 0.0), 40);
    assert!(app.view().unwrap().zoom > z2 * 1.2, "held, it zooms on");
    assert!(doc_at(&app, at).distance(pinned) < 1e-6, "about the press");
    // Off (or without GPU Performance): the area dragged across fills the view.
    for key in ["animatedZoom", "gpuPerformance"] {
        app.run("prefs.reset", json!({})).unwrap();
        app.run("prefs.set", json!({"key": key, "value": false})).unwrap();
        app.view_mut().unwrap().zoom = z0;
        press(&mut app, vec2(100.0, 50.0), 0);
        let z = app.view().unwrap().zoom;
        assert!((z - z0 * (rect.width() / 100.0).min(rect.height() / 50.0) as f64).abs() < 1e-6, "{key} off: {z0} → {z}");
    }
}

/// With Animated Zoom off, the Zoom tool zooms a rotated view to the area dragged across: at 90°
/// and 180° a 100 × 50-point marquee fills the canvas as it does at 0°, centered on the canvas.
#[test]
fn the_zoom_tool_zooms_a_rotated_view_to_the_area_dragged_across() {
    use egui::Event;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.run("prefs.set", json!({"key": "animatedZoom", "value": false})).unwrap();
    app.select_tool("zoom");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let rect = app.canvas_rect.unwrap();
    let at = rect.center() - vec2(100.0, 50.0);
    let to = at + vec2(100.0, 50.0);
    let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    for angle in [90.0, 180.0] {
        app.run("view.rotate", json!({ "angle": angle })).unwrap();
        let z0 = app.view().unwrap().zoom;
        let middle = Xf::new(rect, app.view().unwrap()).to_doc(at + vec2(50.0, 25.0));
        frame(&mut app, &ctx, vec![Event::PointerMoved(at)]);
        frame(&mut app, &ctx, vec![button(at, true)]);
        frame(&mut app, &ctx, vec![Event::PointerMoved(to)]);
        frame(&mut app, &ctx, vec![button(to, false)]);
        let v = app.view().unwrap();
        let want = z0 * f64::from((rect.width() / 100.0).min(rect.height() / 50.0));
        assert!((v.zoom - want).abs() < 1e-6, "{angle}°: {z0} → {}, not {want}", v.zoom);
        assert!(v.center.distance(middle) < 1e-6, "{angle}°: the marquee's center goes to the canvas's center");
        app.view_mut().unwrap().zoom = z0;
    }
}

/// Space held while the Zoom tool drags a marquee moves the marquee and keeps its size. When Space
/// is let go, the drag sizes the marquee again from where it moved to, and the release zooms to the
/// moved marquee.
/// Space held since a Cmd+Space press sizes the marquee.
#[test]
fn space_moves_the_zoom_marquee_while_dragging() {
    use egui::{Event, Modifiers};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.run("prefs.set", json!({"key": "animatedZoom", "value": false})).unwrap();
    app.select_tool("zoom");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let rect = app.canvas_rect.unwrap();
    let at = rect.center() - vec2(100.0, 50.0);
    let space = |pressed, modifiers| Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers };
    let button = |pos, pressed, modifiers| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
    let drawn = |app: &mut VectorcraftApp| -> Vec<Pos2> {
        let lines: Vec<Vec<Pos2>> = shapes(app, &ctx)
            .into_iter()
            .filter_map(|s| match s {
                Shape::Path(p) if p.stroke.color == egui::epaint::ColorMode::Solid(Color32::from_white_alpha(230)) => Some(p.points),
                _ => None,
            })
            .collect();
        let [line] = &lines[..] else { panic!("one marquee: {lines:?}") };
        line.clone()
    };
    let z0 = app.view().unwrap().zoom;
    frame(&mut app, &ctx, vec![Event::PointerMoved(at)]);
    frame(&mut app, &ctx, vec![button(at, true, Modifiers::NONE)]);
    frame(&mut app, &ctx, vec![Event::PointerMoved(at + vec2(100.0, 50.0))]);
    frame(&mut app, &ctx, vec![space(true, Modifiers::NONE), Event::PointerMoved(at + vec2(140.0, 70.0))]);
    let moved = drawn(&mut app);
    assert_eq!(moved[0], at + vec2(40.0, 20.0), "Space moves the marquee: {moved:?}");
    assert_eq!(moved[2], at + vec2(140.0, 70.0), "and keeps its 100 × 50 size: {moved:?}");
    frame(&mut app, &ctx, vec![space(false, Modifiers::NONE), Event::PointerMoved(at + vec2(180.0, 95.0))]);
    let sized = drawn(&mut app);
    assert_eq!((sized[0], sized[2]), (at + vec2(40.0, 20.0), at + vec2(180.0, 95.0)), "let go, the drag sizes it again: {sized:?}");
    let middle = Xf::new(rect, app.view().unwrap()).to_doc(at + vec2(110.0, 57.5));
    frame(&mut app, &ctx, vec![button(at + vec2(180.0, 95.0), false, Modifiers::NONE)]);
    let v = *app.view().unwrap();
    let want = z0 * f64::from((rect.width() / 140.0).min(rect.height() / 75.0));
    assert!((v.zoom - want).abs() < 1e-6, "the release zooms to the moved marquee: {z0} → {}, not {want}", v.zoom);
    assert!(v.center.distance(middle) < 1e-6, "centered on it: {:?}, not {middle:?}", v.center);
    // A Cmd+Space press: Space held from the press on sizes the marquee, also once Cmd is let go.
    app.select_tool("selection");
    app.view_mut().unwrap().zoom = z0;
    frame(&mut app, &ctx, vec![Event::ModifiersChanged(Modifiers::COMMAND), space(true, Modifiers::COMMAND), Event::PointerMoved(at)]);
    frame(&mut app, &ctx, vec![button(at, true, Modifiers::COMMAND)]);
    frame(&mut app, &ctx, vec![Event::PointerMoved(at + vec2(50.0, 25.0))]);
    frame(&mut app, &ctx, vec![Event::ModifiersChanged(Modifiers::NONE), Event::PointerMoved(at + vec2(100.0, 50.0))]);
    let held = drawn(&mut app);
    assert_eq!((held[0], held[2]), (at, at + vec2(100.0, 50.0)), "Space held since the press sizes it: {held:?}");
    frame(&mut app, &ctx, vec![button(at + vec2(100.0, 50.0), false, Modifiers::NONE)]);
    frame(&mut app, &ctx, vec![space(false, Modifiers::NONE)]);
    let want = z0 * f64::from((rect.width() / 100.0).min(rect.height() / 50.0));
    assert!((app.view().unwrap().zoom - want).abs() < 1e-6, "{z0} → {}, not {want}", app.view().unwrap().zoom);
}

/// Space pressed after the Zoom tool's press and before the drag moves: the first movement sizes
/// the marquee, Space then moves it, and the release zooms to it instead of stepping the zoom.
#[test]
fn space_before_the_zoom_drag_moves_sizes_the_marquee_first() {
    use egui::{Event, Modifiers};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.run("prefs.set", json!({"key": "animatedZoom", "value": false})).unwrap();
    app.select_tool("zoom");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let rect = app.canvas_rect.unwrap();
    let at = rect.center() - vec2(100.0, 50.0);
    let space = |pressed| Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    let z0 = app.view().unwrap().zoom;
    frame(&mut app, &ctx, vec![Event::PointerMoved(at)]);
    frame(&mut app, &ctx, vec![button(at, true)]);
    frame(&mut app, &ctx, vec![space(true), Event::PointerMoved(at + vec2(120.0, 60.0))]);
    frame(&mut app, &ctx, vec![Event::PointerMoved(at + vec2(140.0, 70.0))]);
    let middle = Xf::new(rect, app.view().unwrap()).to_doc(at + vec2(80.0, 40.0));
    frame(&mut app, &ctx, vec![button(at + vec2(140.0, 70.0), false)]);
    frame(&mut app, &ctx, vec![space(false)]);
    let v = *app.view().unwrap();
    let want = z0 * f64::from((rect.width() / 120.0).min(rect.height() / 60.0));
    assert!((v.zoom - want).abs() < 1e-6, "the release zooms to the 120 × 60 marquee: {z0} → {}, not {want}", v.zoom);
    assert!(v.center.distance(middle) < 1e-6, "moved by Space and centered on it: {:?}, not {middle:?}", v.center);
}

/// Space pressed or let go between two pointer moves in one frame: the move before the change and
/// the move after it each follow the Space state they came with.
#[test]
fn the_zoom_marquee_follows_space_and_the_pointer_in_the_order_they_came() {
    use egui::{Event, Modifiers};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.run("prefs.set", json!({"key": "animatedZoom", "value": false})).unwrap();
    app.select_tool("zoom");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let rect = app.canvas_rect.unwrap();
    let at = rect.center() - vec2(100.0, 50.0);
    let space = |pressed| Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    let corners = |app: &mut VectorcraftApp| -> (Pos2, Pos2) {
        let lines: Vec<Vec<Pos2>> = shapes(app, &ctx)
            .into_iter()
            .filter_map(|s| match s {
                Shape::Path(p) if p.stroke.color == egui::epaint::ColorMode::Solid(Color32::from_white_alpha(230)) => Some(p.points),
                _ => None,
            })
            .collect();
        let [line] = &lines[..] else { panic!("one marquee: {lines:?}") };
        (line[0], line[2])
    };
    frame(&mut app, &ctx, vec![Event::PointerMoved(at)]);
    frame(&mut app, &ctx, vec![button(at, true)]);
    frame(&mut app, &ctx, vec![Event::PointerMoved(at + vec2(100.0, 50.0))]);
    frame(&mut app, &ctx, vec![space(true), Event::PointerMoved(at + vec2(140.0, 70.0))]);
    // Moved with Space held, then sized after it was let go.
    frame(&mut app, &ctx, vec![Event::PointerMoved(at + vec2(160.0, 80.0)), space(false), Event::PointerMoved(at + vec2(200.0, 100.0))]);
    assert_eq!(corners(&mut app), (at + vec2(60.0, 30.0), at + vec2(200.0, 100.0)), "moved, then sized");
    // Sized before Space was pressed, then moved with it held.
    frame(&mut app, &ctx, vec![Event::PointerMoved(at + vec2(210.0, 105.0)), space(true), Event::PointerMoved(at + vec2(230.0, 115.0))]);
    assert_eq!(corners(&mut app), (at + vec2(80.0, 40.0), at + vec2(230.0, 115.0)), "sized, then moved");
    frame(&mut app, &ctx, vec![button(at + vec2(230.0, 115.0), false)]);
    frame(&mut app, &ctx, vec![space(false)]);
}

/// Space pressed while the Zoom tool drags keeps the zoom cursor: the drag does not turn into a
/// Hand tool pan, and with Shift also held it does not show the rotate cursor.
#[test]
fn space_during_a_zoom_drag_keeps_the_zoom_cursor() {
    use egui::{CursorIcon, Event, Modifiers};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.run("prefs.set", json!({"key": "animatedZoom", "value": false})).unwrap();
    app.select_tool("zoom");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let c = app.canvas_rect.unwrap().center();
    // `shown` returns the system cursor the canvas sets over a window without the focus, as in
    // `the_rotate_cursor_shows_for_space_shift_and_the_rotate_view_tool`.
    let shown = |app: &mut VectorcraftApp, events: Vec<Event>| {
        let screen_rect = Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)));
        let mut out = ctx.run_ui(egui::RawInput { screen_rect, focused: false, events, ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
        out.platform_output.cursor_icon
    };
    let space = |pressed| Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    assert_eq!(shown(&mut app, vec![Event::PointerMoved(c)]), CursorIcon::ZoomIn);
    shown(&mut app, vec![button(c, true)]);
    shown(&mut app, vec![Event::PointerMoved(c + vec2(40.0, 30.0))]);
    assert_eq!(shown(&mut app, vec![space(true), Event::PointerMoved(c + vec2(60.0, 40.0))]), CursorIcon::ZoomIn, "Space during the drag");
    assert_eq!(shown(&mut app, vec![Event::ModifiersChanged(Modifiers::SHIFT)]), CursorIcon::ZoomIn, "Space+Shift during the drag");
    shown(&mut app, vec![Event::ModifiersChanged(Modifiers::NONE)]);
    shown(&mut app, vec![button(c + vec2(60.0, 40.0), false)]);
    assert_eq!(shown(&mut app, vec![]), CursorIcon::Grab, "after the drag, Space shows the hand");
    shown(&mut app, vec![space(false)]);
}

/// In a rotated view the Selection tool's marquee is drawn where the tool selects: through the
/// screen corners of the document rectangle between the press and the pointer.
#[test]
fn the_selection_marquee_is_drawn_where_it_selects_in_a_rotated_view() {
    use egui::Event;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.select_tool("selection");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    app.run("view.rotate", json!({"angle": 45})).unwrap();
    let rect = app.canvas_rect.unwrap();
    let (from, to) = (rect.center() - vec2(60.0, 20.0), rect.center() + vec2(60.0, 40.0));
    frame(&mut app, &ctx, vec![Event::PointerMoved(from)]);
    frame(
        &mut app,
        &ctx,
        vec![Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }],
    );
    frame(&mut app, &ctx, vec![Event::PointerMoved(to)]);
    let xf = Xf::new(rect, app.view().unwrap());
    let want = xf.quad(Rect::from_points(xf.to_doc(from), xf.to_doc(to)));
    let drawn: Vec<Vec<Pos2>> = shapes(&mut app, &ctx)
        .into_iter()
        .filter_map(|s| match s {
            Shape::Path(p) if p.stroke.color == egui::epaint::ColorMode::Solid(Color32::from_white_alpha(230)) => Some(p.points),
            _ => None,
        })
        .collect();
    let [line] = &drawn[..] else { panic!("one marquee: {drawn:?}") };
    assert_eq!(line.len(), 5, "closed: {line:?}");
    assert!(line.iter().zip(&want).all(|(a, b)| a.distance(*b) < 0.5), "{line:?} is not {want:?}");
    assert_eq!(line[4], line[0]);
}

/// User Interface › Large Tabs (#394): the document tabs are taller, so the canvas starts lower.
#[test]
fn large_tabs_make_the_document_tabs_taller() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let canvas_top = |app: &mut VectorcraftApp| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            crate::chrome::doc_tabs(app, ui);
            show(app, ui);
        });
        out.textures_delta.clear();
        app.canvas_rect.unwrap().top()
    };
    let small = canvas_top(&mut app);
    app.run("prefs.set", json!({"key": "largeTabs", "value": true})).unwrap();
    assert_eq!(canvas_top(&mut app) - small, 9.0, "44 points instead of 35");
}

#[test]
fn cmd_space_zooms_and_space_pans_whatever_the_tool() {
    use egui::{Event, Modifiers};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.select_tool("rectangle");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let at = app.canvas_rect.unwrap().center();
    let space = |pressed, modifiers| Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers };
    let button = |pos, pressed, modifiers| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
    // A drag from `at` to `to` with Space and `held` down.
    let drag = |app: &mut VectorcraftApp, held: Modifiers, to: Pos2| {
        frame(app, &ctx, vec![Event::ModifiersChanged(held), space(true, held), Event::PointerMoved(at)]);
        frame(app, &ctx, vec![button(at, true, held)]);
        frame(app, &ctx, vec![Event::PointerMoved(to)]);
        frame(app, &ctx, vec![button(to, false, held)]);
        frame(app, &ctx, vec![space(false, held), Event::ModifiersChanged(Modifiers::NONE)]);
    };
    let z0 = app.view().unwrap().zoom;
    drag(&mut app, Modifiers::COMMAND, at);
    let z1 = app.view().unwrap().zoom;
    assert_eq!(z1, crate::state::next_zoom(z0, true), "Cmd+Space click zooms in");
    drag(&mut app, Modifiers::COMMAND | Modifiers::ALT, at);
    assert_eq!(app.view().unwrap().zoom, crate::state::next_zoom(z1, false), "Cmd+Alt+Space click zooms out");
    assert_eq!(app.session.active().unwrap().doc.art_bounds(), None, "the Rectangle tool drew nothing");
    // Space alone pans, with the Zoom tool too.
    app.select_tool("zoom");
    let before = *app.view().unwrap();
    drag(&mut app, Modifiers::NONE, at + vec2(30.0, 0.0));
    let after = *app.view().unwrap();
    assert_eq!(after.zoom, before.zoom);
    assert!((after.center.x - (before.center.x - 30.0 / before.zoom)).abs() < 1e-6, "{before:?} → {after:?}");
}

#[test]
fn space_shift_drag_rotates_the_view_whatever_the_tool() {
    use egui::{Event, Modifiers};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.select_tool("rectangle");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let c = app.canvas_rect.unwrap().center();
    let space = |pressed, modifiers| Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers };
    let button = |pos, pressed, modifiers| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
    // `drag` drags from `from` to `to` with `held` down, and with Space as well when `with_space`.
    let drag = |app: &mut VectorcraftApp, held: Modifiers, with_space: bool, from: Pos2, to: Pos2| {
        let keys = |pressed| if with_space { vec![space(pressed, held)] } else { vec![] };
        frame(app, &ctx, [vec![Event::ModifiersChanged(held)], keys(true), vec![Event::PointerMoved(from)]].concat());
        frame(app, &ctx, vec![button(from, true, held)]);
        frame(app, &ctx, vec![Event::PointerMoved(to)]);
        frame(app, &ctx, vec![button(to, false, held)]);
        frame(app, &ctx, [keys(false), vec![Event::ModifiersChanged(Modifiers::NONE)]].concat());
    };
    // `at(deg)` is the point 100 points from the canvas center, `deg` degrees clockwise from the right.
    let at = |deg: f32| c + 100.0 * vec2(deg.to_radians().cos(), deg.to_radians().sin());
    let before = *app.view().unwrap();
    drag(&mut app, Modifiers::SHIFT, true, at(0.0), at(90.0));
    let after = *app.view().unwrap();
    assert!((after.rotation - 90.0).abs() < 1e-3, "{after:?}");
    assert_eq!((after.center, after.zoom), (before.center, before.zoom), "the view turns about its center");
    assert_eq!(app.session.active().unwrap().doc.art_bounds(), None, "the Rectangle tool drew nothing");
    // Shift does not round the angle: 90° and 20° more make 110°, not 105°.
    drag(&mut app, Modifiers::SHIFT, true, at(0.0), at(20.0));
    assert!((app.view().unwrap().rotation - 110.0).abs() < 1e-3, "{:?}", app.view());
    // Space alone still pans, and Cmd+Shift+Space still zooms.
    let v = *app.view().unwrap();
    drag(&mut app, Modifiers::NONE, true, c, c + vec2(30.0, 0.0));
    let panned = *app.view().unwrap();
    assert_eq!((panned.rotation, panned.zoom), (v.rotation, v.zoom));
    assert_ne!(panned.center, v.center, "Space pans");
    drag(&mut app, Modifiers::COMMAND | Modifiers::SHIFT, true, c, c);
    assert_eq!(app.view().unwrap().zoom, crate::state::next_zoom(v.zoom, true), "Cmd+Shift+Space click zooms in");
    assert_eq!(app.view().unwrap().rotation, v.rotation);
    // The Rotate View tool's Shift still rounds to 15°.
    app.select_tool("rotateView");
    app.view_mut().unwrap().rotation = 0.0;
    drag(&mut app, Modifiers::SHIFT, false, at(0.0), at(20.0));
    assert_eq!(app.view().unwrap().rotation, 15.0);
    drag(&mut app, Modifiers::NONE, false, at(0.0), at(20.0));
    assert!((app.view().unwrap().rotation - 35.0).abs() < 1e-3, "{:?}", app.view());
}

#[test]
fn the_rotate_cursor_shows_for_space_shift_and_the_rotate_view_tool() {
    use egui::{CursorIcon, Event, Modifiers};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.select_tool("rectangle");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let c = app.canvas_rect.unwrap().center();
    // `shown` returns the system cursor the canvas sets over a window without the focus. On macOS
    // the window paints the glyph only with the focus, and on the other desktop builds (not
    // Wayland, which the tests do not set) the window sets the system cursor next to the bitmap.
    let shown = |app: &mut VectorcraftApp, events: Vec<Event>| {
        let screen_rect = Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)));
        let mut out = ctx.run_ui(egui::RawInput { screen_rect, focused: false, events, ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
        out.platform_output.cursor_icon
    };
    let space = |pressed, modifiers| Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers };
    let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::SHIFT };
    assert_ne!(shown(&mut app, vec![Event::PointerMoved(c)]), CursorIcon::Alias);
    assert_eq!(shown(&mut app, vec![Event::ModifiersChanged(Modifiers::SHIFT), space(true, Modifiers::SHIFT)]), CursorIcon::Alias);
    // Shift let go during the drag keeps the rotate cursor; after it, Space shows the hand.
    shown(&mut app, vec![button(c, true)]);
    assert_eq!(shown(&mut app, vec![Event::ModifiersChanged(Modifiers::NONE), Event::PointerMoved(c + vec2(20.0, 30.0))]), CursorIcon::Alias);
    assert_eq!(shown(&mut app, vec![button(c + vec2(20.0, 30.0), false)]), CursorIcon::Grab);
    // Cmd+Shift+Space zooms.
    assert_eq!(shown(&mut app, vec![Event::ModifiersChanged(Modifiers::COMMAND | Modifiers::SHIFT)]), CursorIcon::ZoomIn);
    assert_ne!(shown(&mut app, vec![space(false, Modifiers::NONE), Event::ModifiersChanged(Modifiers::NONE)]), CursorIcon::Alias);
    app.select_tool("rotateView");
    assert_eq!(shown(&mut app, vec![Event::PointerMoved(c)]), CursorIcon::Alias, "the Rotate View tool");
}

#[test]
fn a_rotate_view_drag_shows_a_dial_with_the_angle() {
    use egui::{Event, Modifiers};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.select_tool("rotateView");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let c = app.canvas_rect.unwrap().center();
    let space = |pressed, modifiers| Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers };
    let button = |pos, pressed, modifiers| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
    // `dial` returns, from the shapes of one frame, the center of the dial's circle, the needle's
    // direction in whole degrees clockwise from straight up, and the angle text.
    let dial = |app: &mut VectorcraftApp| {
        let s = shapes(app, &ctx);
        let circle = s.iter().find_map(|s| if let Shape::Circle(k) = s { (k.radius == DIAL_RADIUS).then_some(k.center) } else { None });
        let needle = s.iter().find_map(|s| match s {
            Shape::LineSegment { points: [a, b], stroke } if stroke.width == 2.0 && *a == c => {
                Some((b.x - a.x).atan2(a.y - b.y).to_degrees().round() as i32)
            }
            _ => None,
        });
        let text = s.iter().find_map(|s| if let Shape::Text(t) = s { Some(t.galley.text().to_string()).filter(|t| t.ends_with('°')) } else { None });
        (circle, needle, text)
    };
    assert_eq!(dial(&mut app), (None, None, None), "no dial before a drag");
    // The Rotate View tool: a drag from 0° to 30° about the canvas center.
    let (right, at_30, below) = (c + vec2(100.0, 0.0), c + vec2(86.6, 50.0), c + vec2(0.0, 100.0));
    frame(&mut app, &ctx, vec![Event::PointerMoved(right)]);
    frame(&mut app, &ctx, vec![button(right, true, Modifiers::NONE)]);
    frame(&mut app, &ctx, vec![Event::PointerMoved(at_30)]);
    assert_eq!(dial(&mut app), (Some(c), Some(30), Some("30°".to_string())));
    frame(&mut app, &ctx, vec![button(at_30, false, Modifiers::NONE)]);
    assert_eq!(dial(&mut app), (None, None, None), "the dial goes with the release");
    assert!((app.view().unwrap().rotation - 30.0).abs() < 1e-2, "{:?}", app.view());
    // A Space+Shift drag with another tool shows it too: 30° and 90° more make 120°.
    app.select_tool("rectangle");
    frame(&mut app, &ctx, vec![Event::ModifiersChanged(Modifiers::SHIFT), space(true, Modifiers::SHIFT), Event::PointerMoved(right)]);
    frame(&mut app, &ctx, vec![button(right, true, Modifiers::SHIFT)]);
    frame(&mut app, &ctx, vec![Event::PointerMoved(below)]);
    assert_eq!(dial(&mut app), (Some(c), Some(120), Some("120°".to_string())));
    assert!((app.view().unwrap().rotation - 120.0).abs() < 1e-2, "{:?}", app.view());
    // Presentation Mode turned on during the drag hides the dial.
    let mode = app.ui.screen_mode;
    app.ui.screen_mode = 3;
    assert_eq!(dial(&mut app), (None, None, None), "no dial in Presentation Mode");
    app.ui.screen_mode = mode;
    frame(&mut app, &ctx, vec![button(below, false, Modifiers::SHIFT)]);
    frame(&mut app, &ctx, vec![space(false, Modifiers::NONE), Event::ModifiersChanged(Modifiers::NONE)]);
    assert_eq!(dial(&mut app), (None, None, None), "the dial goes with the release");
}

#[test]
fn space_pressed_mid_drag_moves_what_the_tool_draws() {
    use egui::{Event, Modifiers};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.ui.view.smart_guides = false;
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let space = |pressed| Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    // Press at `a`, drag to `b`, then hold Space while moving on by `by` and let go of both.
    let draw = |app: &mut VectorcraftApp, a: Pos2, b: Pos2, by: egui::Vec2| {
        frame(app, &ctx, vec![Event::PointerMoved(a)]);
        frame(app, &ctx, vec![button(a, true)]);
        frame(app, &ctx, vec![Event::PointerMoved(b)]);
        frame(app, &ctx, vec![space(true)]);
        frame(app, &ctx, vec![Event::PointerMoved(b + by)]);
        frame(app, &ctx, vec![space(false)]);
        frame(app, &ctx, vec![button(b + by, false)]);
    };
    let near = |p: Point, q: Point| p.distance(q) < 1e-3;
    let at = app.canvas_rect.unwrap().center() - vec2(100.0, 60.0);
    let center = app.view().unwrap().center;
    // The rectangle moves at its size instead of growing, and the view doesn't pan.
    app.select_tool("rectangle");
    draw(&mut app, at, at + vec2(80.0, 40.0), vec2(30.0, 20.0));
    assert_eq!(app.view().unwrap().center, center, "Space during a drag doesn't pan");
    let st = app.session.active().unwrap();
    let r = st.doc.node(st.selection.objects[0]).unwrap().path_data().unwrap().bounds().unwrap();
    assert!(
        near(Point::new(r.x0, r.y0), xf.to_doc(at + vec2(30.0, 20.0))) && near(Point::new(r.x1, r.y1), xf.to_doc(at + vec2(110.0, 60.0))),
        "{r:?}"
    );
    // The Pen's anchor being dragged out moves, handles and all.
    app.session.execute("select.none", &json!({})).unwrap();
    app.select_tool("pen");
    let a = at + vec2(0.0, 120.0);
    draw(&mut app, a, a + vec2(40.0, 0.0), vec2(20.0, 10.0));
    let st = app.session.active().unwrap();
    let anchor = st.doc.node(st.selection.objects[0]).unwrap().path_data().unwrap().subpaths[0].anchors[0];
    assert!(near(anchor.p, xf.to_doc(a + vec2(20.0, 10.0))) && near(anchor.h_out, xf.to_doc(a + vec2(60.0, 10.0))), "{anchor:?}");
}

#[test]
fn double_clicking_type_with_the_selection_tool_puts_the_caret_there() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 100, "y": 150, "text": "Hello world", "size": 24})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.none", &json!({})).unwrap();
    app.select_tool("selection");
    let ctx = egui::Context::default();
    let timed = |app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>| {
        let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), time: Some(time), events, ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
    };
    timed(&mut app, 0.0, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let text = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.plain_text(),
        _ => panic!("not text"),
    };
    // Double-click the middle of the text.
    let b = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap();
    let at = xf.to_screen(b.center());
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    timed(&mut app, 1.0, vec![egui::Event::PointerMoved(at), button(true), button(false), button(true), button(false)]);
    timed(&mut app, 1.1, vec![]);
    // The Type tool edits the text, with the caret inside it, where it was clicked: typing goes
    // into the text, not to tool shortcuts (X would swap fill and stroke).
    assert_eq!(app.session.tool_id(), "type");
    assert!(app.session.tool_wants_text());
    let o = app.session.tool_options();
    assert_eq!(o["editing"], json!(id));
    let caret = o["caret"].as_u64().unwrap();
    assert!((1..11).contains(&caret), "caret {caret}");
    app.session.tool_text("X", app.view_info()).unwrap();
    let t = text(&app);
    assert_eq!((t.len(), t.find('X')), (12, Some(caret as usize)), "{t}");
}

/// #2119: type copied with an Alt-drag, double-clicked, all of its text selected and typed over
/// keeps its font, size and colour: through an IME (the marked text cleared, then committed by
/// Return, as Windows sends them) and with Backspace then typing. Whole app frames, one input
/// event per frame.
fn retype_all_of_a_copied_type(ime: bool) {
    use egui::{Event, Key, Modifiers};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let original = app
        .session
        .execute("text.create", &json!({"x": 60, "y": 100, "text": "Chair 123", "size": 36, "font": "Georgia", "color": "#808080"}))
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    app.session.execute("select.none", &json!({})).unwrap();
    app.select_tool("selection");
    let ctx = egui::Context::default();
    let time = std::cell::Cell::new(0.0);
    let frame = |app: &mut VectorcraftApp, events: Vec<Event>| {
        time.set(time.get() + 0.03);
        let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 850.0));
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), time: Some(time.get()), events, ..Default::default() }, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        out.textures_delta.clear();
    };
    for _ in 0..3 {
        frame(&mut app, vec![]);
    }
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let node = |app: &VectorcraftApp, id: u64| app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
    let styles = |app: &VectorcraftApp, id: u64| match &node(app, id).kind {
        NodeKind::Text(t) => {
            t.runs.iter().map(|r| (r.text.clone(), r.style.font_family.clone(), r.style.size, r.style.fill.clone())).collect::<Vec<_>>()
        }
        _ => panic!("not text"),
    };
    let button = |pos, pressed, modifiers| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
    let key = |key, pressed, modifiers| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers };
    // 1. Alt-drag the type: a copy, selected.
    let from = xf.to_screen(node(&app, original).geometric_bounds().unwrap().center());
    let to = from + vec2(0.0, 120.0 * xf.zoom as f32);
    frame(&mut app, vec![Event::PointerMoved(from), Event::ModifiersChanged(Modifiers::ALT)]);
    frame(&mut app, vec![button(from, true, Modifiers::ALT)]);
    for k in 1..=6 {
        frame(&mut app, vec![Event::PointerMoved(from + (to - from) * (k as f32 / 6.0))]);
    }
    frame(&mut app, vec![button(to, false, Modifiers::ALT)]);
    frame(&mut app, vec![Event::ModifiersChanged(Modifiers::NONE)]);
    let copy = app.session.active().unwrap().selection.objects[0].0;
    assert_ne!(copy, original);
    let style = styles(&app, original)[0].clone();
    // 2. Double-click it at once: the Type tool edits it.
    let at = xf.to_screen(node(&app, copy).geometric_bounds().unwrap().center());
    frame(&mut app, vec![Event::PointerMoved(at)]);
    for pressed in [true, false, true, false] {
        frame(&mut app, vec![button(at, pressed, Modifiers::NONE)]);
    }
    frame(&mut app, vec![]);
    assert_eq!(app.session.tool_options()["editing"], json!(copy));
    // 3. Select all of its text and type over it.
    frame(&mut app, vec![key(Key::A, true, Modifiers::COMMAND)]);
    frame(&mut app, vec![key(Key::A, false, Modifiers::COMMAND)]);
    if ime {
        frame(&mut app, vec![preedit("t", 1)]);
        frame(&mut app, vec![preedit("table", 5)]);
        let marked = styles(&app, copy);
        assert_eq!(marked.iter().map(|r| (&r.1, r.2, &r.3)).collect::<Vec<_>>(), vec![(&style.1, style.2, &style.3)], "composing: {marked:?}");
        // Return commits the composition.
        frame(&mut app, vec![preedit("", 0), Event::Ime(egui::ImeEvent::Commit("table".into()))]);
    } else {
        frame(&mut app, vec![key(Key::Backspace, true, Modifiers::NONE)]);
        frame(&mut app, vec![key(Key::Backspace, false, Modifiers::NONE)]);
        frame(&mut app, vec![Event::Text("table".into())]);
        frame(&mut app, vec![key(Key::Enter, true, Modifiers::NONE)]);
        frame(&mut app, vec![key(Key::Enter, false, Modifiers::NONE)]);
    }
    frame(&mut app, vec![]);
    let after = styles(&app, copy);
    assert!(after.iter().any(|r| r.0.starts_with("table")), "ime={ime}: typed over: {after:?}");
    assert!(
        after.iter().all(|r| (&r.1, r.2, &r.3) == (&style.1, style.2, &style.3)),
        "ime={ime}: the copy keeps {:?} {} {:?}, got {after:?}",
        style.1,
        style.2,
        style.3
    );
}

#[test]
fn retyping_all_of_a_copied_type_through_an_ime_keeps_its_style() {
    retype_all_of_a_copied_type(true);
}

#[test]
fn retyping_all_of_a_copied_type_after_backspace_keeps_its_style() {
    retype_all_of_a_copied_type(false);
}

#[test]
fn double_clicking_type_with_the_direct_selection_tool_puts_the_caret_there() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 100, "y": 150, "text": "Hello world", "size": 24})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.none", &json!({})).unwrap();
    app.select_tool("directSelection");
    let ctx = egui::Context::default();
    let timed = |app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>| {
        let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), time: Some(time), events, ..Default::default() }, |ui| {
            crate::shortcuts::handle(app, ui.ctx());
            show(app, ui)
        });
        out.textures_delta.clear();
    };
    timed(&mut app, 0.0, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let text = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.plain_text(),
        _ => panic!("not text"),
    };
    // Double-click the middle of the text with Direct Selection.
    let b = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap();
    let at = xf.to_screen(b.center());
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    timed(&mut app, 1.0, vec![egui::Event::PointerMoved(at), button(true), button(false), button(true), button(false)]);
    timed(&mut app, 1.1, vec![]);
    // The Type tool edits the text, with the caret inside it, where it was clicked: typing goes
    // into the text, not to tool shortcuts.
    assert_eq!(app.session.tool_id(), "type");
    assert!(app.session.tool_wants_text());
    let o = app.session.tool_options();
    assert_eq!(o["editing"], json!(id));
    let caret = o["caret"].as_u64().unwrap();
    assert!((1..11).contains(&caret), "caret {caret}");
    app.session.tool_text("X", app.view_info()).unwrap();
    let t = text(&app);
    assert_eq!((t.len(), t.find('X')), (12, Some(caret as usize)), "{t}");

    // Pressing Escape exits text editing and returns to Direct Selection tool.
    let esc = |pressed| egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    timed(&mut app, 1.2, vec![esc(true), esc(false)]);
    assert_eq!(app.session.tool_id(), "directSelection");
    assert!(!app.session.tool_wants_text());
}

#[test]
fn escape_while_editing_in_isolation_returns_to_selection_tool_and_keeps_isolation_until_second_escape() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let tid = app.session.execute("text.create", &json!({"x": 100, "y": 150, "text": "Nested hello", "size": 24})).unwrap()["id"].as_u64().unwrap();
    // Group the text inside a group.
    app.session.execute("select.all", &json!({})).unwrap();
    app.session.execute("object.group", &json!({})).unwrap();
    app.select_tool("selection");
    let ctx = egui::Context::default();
    let timed = |app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>| {
        let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), time: Some(time), events, ..Default::default() }, |ui| {
            crate::shortcuts::handle(app, ui.ctx());
            show(app, ui)
        });
        out.textures_delta.clear();
    };
    timed(&mut app, 0.0, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let b = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(tid)).unwrap().geometric_bounds().unwrap();
    let at = xf.to_screen(b.center());
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let esc = |pressed| egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed, repeat: false, modifiers: Default::default() };

    // 1st double-click on group enters isolation mode.
    timed(&mut app, 1.0, vec![egui::Event::PointerMoved(at), button(true), button(false), button(true), button(false)]);
    timed(&mut app, 1.1, vec![]);
    assert!(app.session.active().unwrap().isolation.is_some(), "should be in isolation mode");
    assert_eq!(app.session.tool_id(), "selection");

    // 2nd double-click on the text inside the isolated group enters text editing.
    timed(&mut app, 2.0, vec![egui::Event::PointerMoved(at), button(true), button(false), button(true), button(false)]);
    timed(&mut app, 2.1, vec![]);
    assert_eq!(app.session.tool_id(), "type");
    assert!(app.session.tool_wants_text());
    assert!(app.session.active().unwrap().isolation.is_some(), "isolation mode still active while editing text");

    // 1st Escape exits text edit mode, returns to Selection tool, but keeps isolation mode.
    timed(&mut app, 3.0, vec![esc(true), esc(false)]);
    timed(&mut app, 3.1, vec![]);
    assert_eq!(app.session.tool_id(), "selection");
    assert!(!app.session.tool_wants_text());
    assert!(app.session.active().unwrap().isolation.is_some(), "1st Escape must NOT end isolation mode");

    // 2nd Escape exits isolation mode.
    timed(&mut app, 4.0, vec![esc(true), esc(false)]);
    timed(&mut app, 4.1, vec![]);
    assert_eq!(app.session.tool_id(), "selection");
    assert!(app.session.active().unwrap().isolation.is_none(), "2nd Escape must exit isolation mode");
}

#[test]
fn a_symbol_dropped_on_the_canvas_is_placed_there() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 20})).unwrap();
    let name = app.session.execute("symbol.new", &json!({})).unwrap()["name"].as_str().unwrap().to_string();
    app.session.execute("select.none", &json!({})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    // Released over empty canvas: an instance centred there.
    let at = xf.to_screen(Point::new(250.0, 180.0));
    egui::DragAndDrop::set_payload(&ctx, widgets::PanelDrag::Symbol(name.clone()));
    let up = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() };
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), up]);
    let st = app.session.active().unwrap();
    let placed = st.selection.objects.first().copied().unwrap();
    let n = st.doc.node(placed).unwrap();
    assert!(matches!(&n.kind, NodeKind::SymbolInstance { symbol, .. } if *symbol == name));
    let c = n.geometric_bounds().unwrap().center();
    assert!((c - xf.to_doc(at)).hypot() < 1e-6, "{c:?}");
}

#[test]
fn a_library_symbol_dropped_on_the_canvas_is_added_and_placed_there_in_one_step() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    // Dropped on art too, it is placed, not applied to the art.
    app.session.execute("shape.rectangle", &json!({"x": 200, "y": 150, "width": 100, "height": 60})).unwrap();
    app.session.execute("select.none", &json!({})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let undo = app.session.active().unwrap().history.undo.len();
    let at = xf.to_screen(Point::new(250.0, 180.0));
    egui::DragAndDrop::set_payload(&ctx, widgets::PanelDrag::LibrarySymbol { library: "basic-shapes".into(), name: "Star".into() });
    let up = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() };
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), up]);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.symbols.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Star"]);
    assert_eq!(st.history.undo.len(), undo + 1, "added and placed as one undo step");
    assert_eq!(st.history.undo.last().unwrap().label, "Add to Symbols");
    let placed = st.selection.objects.first().copied().unwrap();
    let n = st.doc.node(placed).unwrap();
    assert!(matches!(&n.kind, NodeKind::SymbolInstance { symbol, .. } if symbol == "Star"));
    // The registration point (the centre) is where it was dropped, in document coordinates
    // (the pointer's screen position, which rounds 250, 180 a little).
    let size = st.doc.unknown.get(vectorcraft_doc::SYMBOL_SIZES).and_then(|m| m.get("Star")).cloned().unwrap();
    let b = n.geometric_bounds().unwrap();
    assert!((b.width() - size[0].as_f64().unwrap()).abs() < 1e-6, "{b:?} {size}");
    assert!((b.center() - xf.to_doc(at)).hypot() < 1e-6 && (b.center() - Point::new(250.0, 180.0)).hypot() < 0.1, "{b:?}");
    app.run("edit.undo", json!({})).unwrap();
    let st = app.session.active().unwrap();
    assert!(st.doc.symbols.is_empty() && st.doc.node(placed).is_none());
}

#[test]
fn a_brush_dropped_on_a_path_is_applied_to_it() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("path.create", &json!({"anchors": [{"x": 100, "y": 150}, {"x": 300, "y": 150}]})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.none", &json!({})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let brush = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        st.doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.stroke().and_then(|s| s.brush.clone())
    };
    let drop = |app: &mut VectorcraftApp, at: Point| {
        let at = xf.to_screen(at);
        egui::DragAndDrop::set_payload(&ctx, widgets::PanelDrag::Brush { name: "Arrow".into(), def: json!({}) });
        let up = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() };
        frame(app, &ctx, vec![egui::Event::PointerMoved(at), up]);
    };
    // Off the path: nothing.
    drop(&mut app, Point::new(200.0, 250.0));
    assert_eq!(brush(&app), None);
    // On it: the path's stroke takes the brush.
    drop(&mut app, Point::new(200.0, 150.0));
    assert_eq!(brush(&app).as_deref(), Some("Arrow"));
}

#[test]
fn a_library_brush_dropped_on_a_path_is_added_and_applied_in_one_step() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("path.create", &json!({"anchors": [{"x": 100, "y": 150}, {"x": 300, "y": 150}]})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.none", &json!({})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let drop = |app: &mut VectorcraftApp, at: Point| {
        let at = xf.to_screen(at);
        egui::DragAndDrop::set_payload(&ctx, widgets::PanelDrag::LibraryBrush { library: "scatter-shapes".into(), name: "Sparkles".into() });
        let up = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() };
        frame(app, &ctx, vec![egui::Event::PointerMoved(at), up]);
    };
    let undo = app.session.active().unwrap().history.undo.len();
    // Off the path: nothing.
    drop(&mut app, Point::new(200.0, 250.0));
    assert!(vectorcraft_brush::find(&app.session.active().unwrap().doc, "Sparkles").is_none());
    // On it: added, with the symbol it scatters, and applied, as one step.
    drop(&mut app, Point::new(200.0, 150.0));
    let st = app.session.active().unwrap();
    assert!(vectorcraft_brush::find(&st.doc, "Sparkles").is_some() && st.doc.symbols.iter().any(|s| s.name == "Sparkle"), "{}", app.ui.status);
    let n = st.doc.node(vectorcraft_doc::NodeId(id)).unwrap();
    assert_eq!(n.appearance.stroke().and_then(|s| s.brush.clone()).as_deref(), Some("Sparkles"));
    assert_eq!(st.history.undo.len(), undo + 1);
    assert_eq!(st.history.undo.last().unwrap().label, "Add to Brushes");
}

#[test]
fn middle_drag_pans_the_view_with_any_tool() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    // A drawing tool: the middle button must pan, never draw.
    app.select_tool("rectangle");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(pos2(400.0, 300.0))]);
    let before = *app.view().unwrap();
    frame(&mut app, &ctx, vec![middle(pos2(400.0, 300.0), true)]);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(pos2(450.0, 330.0))]);
    frame(&mut app, &ctx, vec![middle(pos2(450.0, 330.0), false)]);
    let after = *app.view().unwrap();
    assert_eq!(after.zoom, before.zoom);
    assert!((after.center.x - (before.center.x - 50.0 / before.zoom)).abs() < 1e-6);
    assert!((after.center.y - (before.center.y - 30.0 / before.zoom)).abs() < 1e-6);
    // Released: moving no longer pans, and nothing was drawn.
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(pos2(300.0, 200.0))]);
    assert_eq!(app.view().unwrap().center, after.center);
    assert_eq!(app.session.active().unwrap().doc.art_bounds(), None);
}

#[test]
fn swatches_and_proxy_paints_dropped_on_art_fill_the_object_hit() {
    use vectorcraft_color::{Color, GradientGeom, GradientPaint, Paint};
    use widgets::{PanelDrag, SwatchRows};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.none", &json!({})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let drop = |app: &mut VectorcraftApp, d: PanelDrag, at: Point| {
        let at = xf.to_screen(at);
        egui::DragAndDrop::set_payload(&ctx, d);
        let up = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() };
        frame(app, &ctx, vec![egui::Event::PointerMoved(at), up]);
    };
    let fill = |app: &VectorcraftApp| app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.fill_paint();
    let red = Paint::solid(Color::from_hex("#ed1c24").unwrap());
    let rows = SwatchRows { grabbed: "Red".into(), names: vec!["Red".into()], groups: false };
    let swatch = PanelDrag::Paint { paint: red.clone(), params: json!({"swatch": "Red"}), rows: Some(rows), library: None };
    // Off the art nothing happens.
    drop(&mut app, swatch.clone(), Point::new(300.0, 250.0));
    assert_eq!(fill(&app), Paint::solid(Color::WHITE));
    drop(&mut app, swatch, Point::new(100.0, 100.0));
    assert_eq!(fill(&app), red, "paint.setFill with the hit id");
    assert!(app.session.active().unwrap().selection.is_empty(), "the selection stays as it was");
    // A proxy's paint works the same.
    drop(&mut app, PanelDrag::paint(Paint::solid(Color::rgb(0.0, 0.0, 1.0))), Point::new(60.0, 60.0));
    assert_eq!(fill(&app), Paint::solid(Color::rgb(0.0, 0.0, 1.0)));
    // A dragged gradient (the Gradient panel's thumbnail, a proxy) fits the object it lands on.
    let mut g = GradientPaint::new(Default::default());
    g.geom = Some(GradientGeom { start: Point::new(0.0, 0.0), end: Point::new(10.0, 0.0), aspect: 1.0, focal: None });
    drop(&mut app, PanelDrag::paint(Paint::Gradient(Box::new(g))), Point::new(100.0, 100.0));
    assert!(matches!(fill(&app), Paint::Gradient(g) if g.geom.is_none()));
}

#[test]
fn dropping_the_appearance_thumbnail_on_art_copies_the_appearance() {
    use vectorcraft_doc::NodeId;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
    run(&mut app, "file.new", json!({"width": 400, "height": 300}));
    let a = run(&mut app, "shape.rectangle", json!({"x": 20, "y": 20, "width": 100, "height": 100}))["id"].as_u64().unwrap();
    run(&mut app, "effect.apply", json!({"effect": "distort.twist"}));
    run(&mut app, "transparency.set", json!({"opacity": 40}));
    let b = run(&mut app, "shape.rectangle", json!({"x": 200, "y": 20, "width": 100, "height": 100}))["id"].as_u64().unwrap();
    run(&mut app, "select.set", json!({ "ids": [a] }));
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    // The Appearance panel's thumbnail (its drag payload) released over the second rectangle.
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let drop = |app: &mut VectorcraftApp, at: Point| {
        let at = xf.to_screen(at);
        egui::DragAndDrop::set_payload(&ctx, widgets::PanelDrag::Appearance(NodeId(a)));
        frame(app, &ctx, vec![egui::Event::PointerMoved(at)]);
        let up = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() };
        frame(app, &ctx, vec![up]);
    };
    drop(&mut app, Point::new(250.0, 70.0));
    let doc = &app.session.active().unwrap().doc;
    let (na, nb) = (doc.node(NodeId(a)).unwrap(), doc.node(NodeId(b)).unwrap());
    assert_eq!((&nb.appearance, nb.opacity), (&na.appearance, na.opacity));
    assert_eq!(nb.appearance.effects[0].id, "distort.twist");
    // Dropped on empty canvas or on the source itself: nothing changes.
    let undo_len = app.session.active().unwrap().history.undo.len();
    drop(&mut app, Point::new(350.0, 250.0));
    drop(&mut app, Point::new(70.0, 70.0));
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo_len);
}

#[test]
fn a_graphic_style_dropped_on_art_applies_to_the_object_hit() {
    use vectorcraft_doc::NodeId;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
    run(&mut app, "file.new", json!({"width": 400, "height": 300}));
    let a = NodeId(run(&mut app, "shape.rectangle", json!({"x": 20, "y": 20, "width": 100, "height": 100}))["id"].as_u64().unwrap());
    let b = NodeId(run(&mut app, "shape.rectangle", json!({"x": 200, "y": 20, "width": 100, "height": 100}))["id"].as_u64().unwrap());
    run(&mut app, "select.set", json!({ "ids": [a.0] }));
    let name = app.session.active().unwrap().doc.graphic_styles[2].name.clone();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let at = xf.to_screen(Point::new(250.0, 70.0));
    egui::DragAndDrop::set_payload(&ctx, widgets::PanelDrag::GraphicStyle(name.clone()));
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
    let up = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() };
    frame(&mut app, &ctx, vec![up]);
    let st = app.session.active().unwrap();
    let g = st.doc.graphic_style(&name).unwrap();
    // `graphicStyle.apply` with the hit object's id: linked, the selection left alone.
    assert_eq!(st.doc.node(b).unwrap().extras().graphic_style, Some(g.id));
    assert_eq!(st.doc.node(b).unwrap().appearance, g.appearance);
    assert_eq!(st.doc.node(a).unwrap().extras().graphic_style, None);
    assert_eq!(st.selection.objects, [a]);
}

#[test]
fn art_moved_off_the_canvas_waits_and_drops_on_the_panel() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    let id = vectorcraft_doc::NodeId(id);
    app.select_tool("selection");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let before = app.session.active().unwrap().doc.clone();
    let undo = app.session.active().unwrap().history.undo.len();
    let button =
        |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let start = xf.to_screen(Point::new(100.0, 100.0));
    let left = |app: &VectorcraftApp| app.session.active().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().x0;
    let moved_to = |screen: Pos2| 50.0 + xf.to_doc(screen).x - 100.0;
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start), button(start, true)]);
    // Moving on the canvas moves the art...
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start + vec2(40.0, 0.0))]);
    assert!(app.session.active().unwrap().interaction.is_some());
    // ...over a panel it waits where it was, and the panels get the art.
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(pos2(850.0, 100.0))]);
    assert!(app.session.active().unwrap().interaction.is_some(), "the move waits, not cancels");
    assert!((left(&app) - moved_to(start + vec2(40.0, 0.0))).abs() < 0.01, "the art stays where it last moved");
    assert_eq!(egui::DragAndDrop::payload::<widgets::PanelDrag>(&ctx).as_deref(), Some(&widgets::PanelDrag::Art(vec![id])));
    // Released over the panel, the move goes back and nothing is recorded.
    frame(&mut app, &ctx, vec![button(pos2(850.0, 100.0), false)]);
    let st = app.session.active().unwrap();
    assert!(st.interaction.is_none());
    assert_eq!(*st.doc, *before);
    assert_eq!(st.history.undo.len(), undo);
}

/// #4233: the Direct and Group Selection tools hand art they move whole to the panels too (Asset
/// Export, Graphic Styles…), as the Selection tool does; the move waits and nothing is recorded.
#[test]
fn art_moved_whole_off_the_canvas_with_direct_selection_drops_on_the_panel() {
    for tool in ["directSelection", "groupSelection"] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let id = app.session.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
        let id = vectorcraft_doc::NodeId(id);
        app.session.execute("select.none", &json!({})).unwrap();
        app.select_tool(tool);
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let before = app.session.active().unwrap().doc.clone();
        let undo = app.session.active().unwrap().history.undo.len();
        let button = |pos: Pos2, pressed: bool| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        // Pressed on the fill: the rectangle is selected whole.
        let start = xf.to_screen(Point::new(100.0, 100.0));
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start), button(start, true)]);
        assert_eq!(app.session.active().unwrap().selection.whole(), [id], "{tool}: selected whole");
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start + vec2(40.0, 0.0))]);
        assert!(app.session.active().unwrap().interaction.is_some(), "{tool}: the move is live");
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(pos2(850.0, 100.0))]);
        assert!(app.session.active().unwrap().interaction.is_some(), "{tool}: the move waits, not cancels");
        assert_eq!(egui::DragAndDrop::payload::<widgets::PanelDrag>(&ctx).as_deref(), Some(&widgets::PanelDrag::Art(vec![id])), "{tool}");
        frame(&mut app, &ctx, vec![button(pos2(850.0, 100.0), false)]);
        let st = app.session.active().unwrap();
        assert!(st.interaction.is_none(), "{tool}");
        assert_eq!(*st.doc, *before, "{tool}: the move goes back");
        assert_eq!(st.history.undo.len(), undo, "{tool}: nothing recorded");
    }
}

/// An anchor point the Direct Selection tool drags off the canvas isn't art for the panels: the
/// reshape goes on and no art drag starts (#4233).
#[test]
fn an_anchor_dragged_off_the_canvas_is_not_handed_to_the_panels() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.select_tool("directSelection");
    app.session.execute("select.anchors", &json!({"id": id, "anchors": [[0, 0]]})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let button =
        |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let corner = xf.to_screen(Point::new(50.0, 50.0));
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(corner), button(corner, true)]);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(corner + vec2(20.0, 0.0))]);
    assert!(app.session.active().unwrap().selection.partial(vectorcraft_doc::NodeId(id)).is_some(), "an anchor is picked");
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(pos2(850.0, 100.0))]);
    assert!(
        !matches!(egui::DragAndDrop::payload::<widgets::PanelDrag>(&ctx).as_deref(), Some(widgets::PanelDrag::Art(_))),
        "no art drag for an anchor"
    );
    frame(&mut app, &ctx, vec![button(pos2(850.0, 100.0), false)]);
}

/// #2826: a move taken over a panel waits where it left the canvas, and goes on with the
/// pointer when it comes back — it isn't dropped or put back.
#[test]
fn art_moved_off_the_canvas_resumes_when_the_pointer_returns() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.select_tool("selection");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let undo = app.session.active().unwrap().history.undo.len();
    let button =
        |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let start = xf.to_screen(Point::new(100.0, 100.0));
    let left = |app: &VectorcraftApp| app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap().x0;
    let moved_to = |screen: Pos2| 50.0 + xf.to_doc(screen).x - 100.0;
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start), button(start, true)]);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start + vec2(40.0, 0.0))]);
    assert!((left(&app) - moved_to(start + vec2(40.0, 0.0))).abs() < 0.01, "the move is live");
    // Over a panel the move waits where it last moved, and stays live.
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(pos2(850.0, 100.0))]);
    assert!(app.session.active().unwrap().interaction.is_some(), "the move waits, not cancels");
    assert!((left(&app) - moved_to(start + vec2(40.0, 0.0))).abs() < 0.01, "the art stays where it last moved");
    // Back on the canvas it goes on with the pointer, 60 pt right.
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start + vec2(60.0, 0.0))]);
    assert!(app.session.active().unwrap().interaction.is_some(), "the move is live again");
    assert!((left(&app) - moved_to(start + vec2(60.0, 0.0))).abs() < 0.01, "the move resumes with the pointer");
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start + vec2(70.0, 0.0))]);
    frame(&mut app, &ctx, vec![button(start + vec2(70.0, 0.0), false)]);
    let st = app.session.active().unwrap();
    assert!((left(&app) - moved_to(start + vec2(70.0, 0.0))).abs() < 0.01, "released where the pointer was");
    assert_eq!(st.history.undo.len(), undo + 1, "one move, one undo step");
}

/// One frame of keyboard handling, canvas and the end-of-pass IME observation, as the app runs
/// them; returns the IME output.
fn typing_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Option<egui::output::IMEOutput> {
    VectorcraftApp::install_ime_reset_hooks(ctx);
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| {
        crate::ime::begin_ui(app, ui.ctx());
        crate::shortcuts::handle(app, ui.ctx());
        show(app, ui);
        crate::ime::end_of_pass(app, ui.ctx());
        crate::ime::finish_ui(app, ui.ctx());
    });
    out.textures_delta.clear();
    out.platform_output.ime
}

fn preedit(t: &str, chars: usize) -> egui::Event {
    egui::Event::Ime(egui::ImeEvent::Preedit { text: t.into(), active_range_chars: Some(chars..chars) })
}

fn plain(app: &VectorcraftApp, id: u64) -> String {
    match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.plain_text(),
        _ => panic!("not text"),
    }
}

/// #2826: the app ends a move released over a panel before the panels draw, so a panel takes
/// the art as it was, and no move is left open.
#[test]
fn a_move_released_over_a_panel_ends_before_the_panels_draw() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.select_tool("selection");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let before = app.session.active().unwrap().doc.clone();
    let undo = app.session.active().unwrap().history.undo.len();
    let button =
        |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let start = xf.to_screen(Point::new(100.0, 100.0));
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start), button(start, true)]);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start + vec2(40.0, 0.0))]);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(pos2(850.0, 100.0))]);
    assert!(app.session.active().unwrap().interaction.is_some());
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
        events: vec![button(pos2(850.0, 100.0), false)],
        ..Default::default()
    };
    let mut out = ctx.run_ui(raw, |ui| {
        crate::canvas::release_over_panel(&mut app, ui.ctx());
        assert!(app.session.active().unwrap().interaction.is_none(), "the move is ended");
        assert_eq!(*app.session.active().unwrap().doc, *before, "the art is back where it was");
        assert_eq!(
            egui::DragAndDrop::payload::<widgets::PanelDrag>(ui.ctx()).as_deref(),
            Some(&widgets::PanelDrag::Art(vec![vectorcraft_doc::NodeId(id)])),
            "the panel still gets the art"
        );
        assert!(ui.ctx().data(|d| d.get_temp::<Drag>(drag_id())).is_none(), "no drag is left");
    });
    out.textures_delta.clear();
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
}

#[test]
fn japanese_ime_composes_on_the_canvas_with_its_window_at_the_caret() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    // The type placed starts empty (Fill New Type Objects With Placeholder Text off).
    app.session.prefs.placeholder_text = false;
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let ctx = egui::Context::default();
    // Not editing: the IME stays off (single-key tool shortcuts keep working).
    assert!(typing_frame(&mut app, &ctx, vec![]).is_none());
    app.select_tool("type");
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let at = xf.to_screen(Point::new(100.0, 100.0));
    let click =
        |p: Pos2, pressed| egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    typing_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), click(at, true)]);
    typing_frame(&mut app, &ctx, vec![click(at, false)]);
    assert!(app.session.tool_wants_text());
    let id = app.session.active().unwrap().selection.objects[0].0;
    // Editing: the IME is allowed, its window at the caret (the text's baseline at y = 100).
    let ime = typing_frame(&mut app, &ctx, vec![]).expect("IME allowed while editing");
    assert!(ime.rect.contains(pos2(ime.rect.center().x, at.y)) && (ime.rect.center().x - at.x).abs() < 2.0, "{:?} vs {at:?}", ime.rect);
    assert!(!ime.should_interrupt_composition);
    // gagaku → がが → 雅楽, then commit (the macOS sequence).
    typing_frame(&mut app, &ctx, vec![preedit("g", 1)]);
    typing_frame(&mut app, &ctx, vec![preedit("が", 1), preedit("がg", 2)]);
    typing_frame(&mut app, &ctx, vec![preedit("ががく", 3)]);
    assert_eq!(plain(&app, id), "ががく");
    // While composing: plain text events, the clipboard and Undo (the native menu's ⌘Z
    // reaches `menus::invoke`) all wait.
    typing_frame(&mut app, &ctx, vec![egui::Event::Text("x".into()), egui::Event::Paste("y".into())]);
    crate::menus::invoke(&mut app, "edit.undo", json!({}));
    assert!(!crate::menus::enabled(&app, "edit.undo"));
    assert_eq!(plain(&app, id), "ががく");
    let ime2 = typing_frame(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::Preedit { text: "雅楽".into(), active_range_chars: Some(0..2) })])
        .unwrap();
    assert!(ime2.rect.center().x <= ime.rect.center().x + 1.0, "the window stays at the clause being converted");
    typing_frame(&mut app, &ctx, vec![preedit("", 0), egui::Event::Ime(egui::ImeEvent::Commit("雅楽".into()))]);
    assert_eq!(plain(&app, id), "雅楽");
    assert!(!app.session.tool_composing());
    // After the commit, text and Undo work again.
    typing_frame(&mut app, &ctx, vec![egui::Event::Text("!".into())]);
    assert_eq!(plain(&app, id), "雅楽!");
    crate::menus::invoke(&mut app, "edit.undo", json!({}));
    assert_eq!(plain(&app, id), "");
}

/// Vietnamese Telex via the Windows IME (TSF): the preedit grows digit by digit, then an empty
/// preedit plus a single Commit lands the amount once, not duplicated.
#[test]
fn vietnamese_telex_digits_commit_once() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.prefs.placeholder_text = false;
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let ctx = egui::Context::default();
    typing_frame(&mut app, &ctx, vec![]);
    app.select_tool("type");
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let at = xf.to_screen(Point::new(100.0, 100.0));
    let click =
        |p: Pos2, pressed| egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    typing_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), click(at, true)]);
    typing_frame(&mut app, &ctx, vec![click(at, false)]);
    let id = app.session.active().unwrap().selection.objects[0].0;
    typing_frame(&mut app, &ctx, vec![]);
    typing_frame(&mut app, &ctx, vec![preedit("3", 1)]);
    typing_frame(&mut app, &ctx, vec![preedit("31", 2)]);
    typing_frame(&mut app, &ctx, vec![preedit("316", 3)]);
    typing_frame(&mut app, &ctx, vec![preedit("3160", 4)]);
    typing_frame(&mut app, &ctx, vec![preedit("", 0), egui::Event::Ime(egui::ImeEvent::Commit("3160".into()))]);
    assert_eq!(plain(&app, id), "3160");
    assert!(!app.session.tool_composing());
}

/// Keyboard-hook IMEs (UniKey/OpenKey) type Vietnamese by injecting VK_PACKET characters and
/// Backspaces, which egui surfaces as interleaved Event::Text and Event::Key(Backspace). When a
/// tone key fires, the Backspaces and the re-injected characters arrive in one frame; the Type
/// tool must apply them in order to build "tiếng" (captured from a real UniKey session), not
/// insert every character first and delete afterwards.
#[test]
fn hook_ime_backspace_and_reinject_builds_vietnamese() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.prefs.placeholder_text = false;
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let ctx = egui::Context::default();
    typing_frame(&mut app, &ctx, vec![]);
    app.select_tool("type");
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let at = xf.to_screen(Point::new(100.0, 100.0));
    let click =
        |p: Pos2, pressed| egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    typing_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), click(at, true)]);
    typing_frame(&mut app, &ctx, vec![click(at, false)]);
    let id = app.session.active().unwrap().selection.objects[0].0;
    typing_frame(&mut app, &ctx, vec![]);
    let txt = |s: &str| egui::Event::Text(s.into());
    let bs = || egui::Event::Key { key: egui::Key::Backspace, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    // When a tone key fires, the Backspaces and the re-injected VK_PACKET characters land
    // together in one poll (one egui frame).
    typing_frame(&mut app, &ctx, vec![txt("t")]);
    typing_frame(&mut app, &ctx, vec![txt("i")]);
    typing_frame(&mut app, &ctx, vec![txt("e")]);
    typing_frame(&mut app, &ctx, vec![bs(), txt("ê")]); // 2nd "e" -> ê
    typing_frame(&mut app, &ctx, vec![txt("n")]);
    typing_frame(&mut app, &ctx, vec![txt("g")]);
    typing_frame(&mut app, &ctx, vec![bs(), bs(), bs(), txt("ế"), txt("n"), txt("g")]); // "s" tone
    assert_eq!(plain(&app, id), "tiếng");
}

/// Clear History waits while an IME composes, as Undo does: the marked text and the steps stay.
#[test]
fn clear_history_waits_while_the_ime_composes() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.prefs.placeholder_text = false;
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 300, "y": 200, "width": 20, "height": 20})).unwrap();
    let ctx = egui::Context::default();
    typing_frame(&mut app, &ctx, vec![]);
    app.select_tool("type");
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let at = xf.to_screen(Point::new(100.0, 100.0));
    let click =
        |p: Pos2, pressed| egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    typing_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), click(at, true)]);
    typing_frame(&mut app, &ctx, vec![click(at, false)]);
    let id = app.session.active().unwrap().selection.objects[0].0;
    typing_frame(&mut app, &ctx, vec![preedit("が", 1)]);
    assert!(app.session.tool_composing());
    assert!(!crate::menus::enabled(&app, "edit.clearHistory"));
    crate::menus::invoke(&mut app, "edit.clearHistory", json!({}));
    assert!(app.session.tool_composing(), "the composition goes on");
    assert!(!app.session.active().unwrap().history.undo.is_empty(), "the steps stay");
    typing_frame(&mut app, &ctx, vec![preedit("", 0), egui::Event::Ime(egui::ImeEvent::Commit("雅".into()))]);
    assert_eq!(plain(&app, id), "雅");
    assert!(crate::menus::enabled(&app, "edit.clearHistory"));
}

#[test]
fn clicking_away_mid_composition_keeps_the_text_and_interrupts_the_ime() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let other = app.session.execute("text.create", &json!({"x": 300, "y": 250, "text": "Other", "size": 24})).unwrap()["id"].as_u64().unwrap();
    let ctx = egui::Context::default();
    typing_frame(&mut app, &ctx, vec![]);
    app.select_tool("type");
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let (a, b) = (xf.to_screen(Point::new(100.0, 100.0)), xf.to_screen(Point::new(310.0, 245.0)));
    let click =
        |p: Pos2, pressed| egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    typing_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(a), click(a, true)]);
    typing_frame(&mut app, &ctx, vec![click(a, false)]);
    let id = app.session.active().unwrap().selection.objects[0].0;
    typing_frame(&mut app, &ctx, vec![preedit("しょうこ", 4)]);
    // macOS sends nothing for a click away; the tool keeps the marked text and the IME is
    // told to drop its composition. A press on other type edits that type at once.
    let ime = typing_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(b), click(b, true)]);
    assert_eq!(plain(&app, id), "しょうこ");
    assert!(!app.session.tool_composing());
    assert_eq!(app.session.active().unwrap().selection.objects, vec![vectorcraft_doc::NodeId(other)]);
    let ime = ime.expect("editing the other text");
    assert!(ime.should_interrupt_composition);
    let request_id = ime.reset_request.expect("the final destination publishes its reset request");
    assert!(app.ime_marked.is_none());
    assert!(app.take_ime_discard(), "the host tells the system IME to drop its marked text");
    typing_frame(&mut app, &ctx, vec![click(b, false)]);
    // Interrupted once, not every frame.
    assert!(!typing_frame(&mut app, &ctx, vec![]).unwrap().should_interrupt_composition);
    assert!(!app.take_ime_discard());
    // A click on blank canvas ends the editing (#2871): the marked text stays, the frame asks
    // for no IME, and the host tells the system IME to drop its composition.
    let blank = xf.to_screen(Point::new(150.0, 200.0));
    typing_frame(&mut app, &ctx, vec![egui::Event::ImeResetCompleted { request_id }, preedit("が", 1)]);
    typing_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(blank), click(blank, true)]);
    let ime = typing_frame(&mut app, &ctx, vec![click(blank, false)]);
    assert!(plain(&app, other).contains('が'), "{}", plain(&app, other));
    assert!(!app.session.tool_composing() && !app.session.tool_wants_text());
    assert!(ime.is_none());
    assert!(app.ime_marked.is_none());
    assert!(app.take_ime_discard());
}

#[test]
fn ime_edge_orders_never_eat_committed_text_or_lose_the_composition() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let ctx = egui::Context::default();
    typing_frame(&mut app, &ctx, vec![]);
    app.select_tool("type");
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let at = xf.to_screen(Point::new(100.0, 100.0));
    let click =
        |p: Pos2, pressed| egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    typing_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), click(at, true)]);
    typing_frame(&mut app, &ctx, vec![click(at, false)]);
    let id = app.session.active().unwrap().selection.objects[0].0;
    typing_frame(&mut app, &ctx, vec![egui::Event::Text("雅".into())]);
    let backspace =
        |pressed| egui::Event::Key { key: egui::Key::Backspace, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    // The IME empties its marked text and the Backspace comes in the same frame.
    typing_frame(&mut app, &ctx, vec![preedit("が", 1)]);
    typing_frame(&mut app, &ctx, vec![preedit("", 0), backspace(true), backspace(false)]);
    assert_eq!(plain(&app, id), "雅", "the committed character stays");
    // A bare line-break commit ends the composition with the marked text, no newline.
    typing_frame(&mut app, &ctx, vec![preedit("がく", 2)]);
    typing_frame(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::Commit("\n".into()))]);
    assert!(!app.session.tool_composing());
    assert_eq!(plain(&app, id), "雅がく");
    assert!(app.take_ime_discard());
    // Switching apps mid-composition: the composition goes on when the window comes back.
    typing_frame(&mut app, &ctx, vec![preedit("らく", 2)]);
    typing_frame(&mut app, &ctx, vec![egui::Event::WindowFocused(false)]);
    typing_frame(&mut app, &ctx, vec![egui::Event::WindowFocused(true), preedit("らくか", 3)]);
    assert!(app.session.tool_composing());
    assert!(!app.take_ime_discard());
    assert_eq!(plain(&app, id), "雅がくらくか");
}

/// Reset to a blank document, select the Type tool and click document point (100, 100), returning
/// the screen position used by IME fixtures.
fn type_at(app: &mut VectorcraftApp, ctx: &egui::Context) -> egui::Pos2 {
    app.session.prefs.placeholder_text = false;
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    typing_frame(app, ctx, vec![]);
    app.select_tool("type");
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let at = xf.to_screen(Point::new(100.0, 100.0));
    let click =
        |p: Pos2, pressed| egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    typing_frame(app, ctx, vec![egui::Event::PointerMoved(at), click(at, true)]);
    typing_frame(app, ctx, vec![click(at, false)]);
    at
}

/// The outgoing descriptor carries the committed logical text and UTF-8 offsets, keeps a
/// reversed selection reversed, and labels only accepted IME mutations `InputMethod`.
#[test]
fn ime_output_carries_text_offsets_and_explicit_cause() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    type_at(&mut app, &ctx);
    let empty = typing_frame(&mut app, &ctx, vec![]).expect("the Type tool owns the IME");
    let payload = empty.surrounding_text.available().expect("an empty document is available");
    assert_eq!((payload.text.as_str(), payload.cursor, payload.anchor), ("", 0, 0));
    assert_eq!(empty.change_cause, egui::output::ImeChangeCause::Other);

    let typed = typing_frame(&mut app, &ctx, vec![egui::Event::Text("Héllo".into())]).unwrap();
    let payload = typed.surrounding_text.available().unwrap();
    assert_eq!((payload.text.as_str(), payload.cursor, payload.anchor), ("Héllo", 6, 6));
    assert_eq!(typed.change_cause, egui::output::ImeChangeCause::Other, "ordinary typing is application-origin");

    let shift_left =
        || egui::Event::Key { key: egui::Key::ArrowLeft, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::SHIFT };
    // "Héllo" has five characters: four Shift+Lefts stop at the start of its two-byte é,
    // leaving cursor 1 < anchor 6.
    let mut reversed = None;
    for _ in 0..4 {
        reversed = typing_frame(&mut app, &ctx, vec![shift_left()]);
    }
    let reversed = reversed.unwrap();
    let payload = reversed.surrounding_text.available().unwrap();
    assert_eq!((payload.text.as_str(), payload.cursor, payload.anchor), ("Héllo", 1, 6), "cursor > anchor stays reversed");

    // One accepted IME deletion (one byte before the selection) then the commit: both receipts
    // chain, and the commit still replaces the preserved selection ("éllo").
    let ime = typing_frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 0 }),
            egui::Event::Ime(egui::ImeEvent::Commit("Z".into())),
        ],
    )
    .unwrap();
    let payload = ime.surrounding_text.available().unwrap();
    assert_eq!((payload.text.as_str(), payload.cursor, payload.anchor), ("Z", 1, 1));
    assert_eq!(ime.change_cause, egui::output::ImeChangeCause::InputMethod);

    let ordinary = typing_frame(&mut app, &ctx, vec![egui::Event::Text("!".into())]).unwrap();
    assert_eq!(ordinary.change_cause, egui::output::ImeChangeCause::Other, "typing after a commit is Other");
}

/// A pre-edit is excluded from the outgoing text; committing the composition is the accepted
/// IME mutation that earns `InputMethod`.
#[test]
fn ime_output_excludes_the_preedit_and_marks_the_commit() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    type_at(&mut app, &ctx);
    typing_frame(&mut app, &ctx, vec![egui::Event::Text("ab".into())]);
    let marked = typing_frame(&mut app, &ctx, vec![preedit("xy", 2)]).unwrap();
    let payload = marked.surrounding_text.available().unwrap();
    assert_eq!((payload.text.as_str(), payload.cursor, payload.anchor), ("ab", 2, 2), "the marked text is not committed text");
    let committed = typing_frame(&mut app, &ctx, vec![preedit("", 0), egui::Event::Ime(egui::ImeEvent::Commit("xy".into()))]).unwrap();
    let payload = committed.surrounding_text.available().unwrap();
    assert_eq!((payload.text.as_str(), payload.cursor, payload.anchor), ("abxy", 4, 4));
    assert_eq!(committed.change_cause, egui::output::ImeChangeCause::InputMethod);
}

/// An application edit anywhere in the frame dominates an IME receipt in it.
#[test]
fn ime_output_marks_mixed_application_and_ime_mutations_other() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    type_at(&mut app, &ctx);
    typing_frame(&mut app, &ctx, vec![egui::Event::Text("ab".into())]);
    let ime = typing_frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::Text("x".into()),
            egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 0 }),
            egui::Event::Ime(egui::ImeEvent::Commit("y".into())),
        ],
    )
    .unwrap();
    let payload = ime.surrounding_text.available().unwrap();
    assert_eq!((payload.text.as_str(), payload.cursor, payload.anchor), ("aby", 3, 3));
    assert_eq!(ime.change_cause, egui::output::ImeChangeCause::Other, "application edit before the IME dominates");

    let pasted = typing_frame(&mut app, &ctx, vec![egui::Event::Paste("c".into())]).unwrap();
    let payload = pasted.surrounding_text.available().unwrap();
    assert_eq!((payload.text.as_str(), payload.cursor, payload.anchor), ("abyc", 4, 4));
    assert_eq!(pasted.change_cause, egui::output::ImeChangeCause::Other, "paste is application-origin");

    // The other order: an accepted IME commit followed by an application edit in the same
    // frame is still `Other`.
    let ime_then_text =
        typing_frame(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::Commit("y".into())), egui::Event::Text("x".into())]).unwrap();
    let payload = ime_then_text.surrounding_text.available().unwrap();
    assert_eq!((payload.text.as_str(), payload.cursor, payload.anchor), ("abycyx", 6, 6));
    assert_eq!(ime_then_text.change_cause, egui::output::ImeChangeCause::Other, "application edit after the IME dominates");
}

/// Leaving and re-entering the same text object keeps the owner but starts a new generation.
#[test]
fn ime_owner_replacement_starts_a_new_generation() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.prefs.placeholder_text = false;
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 100, "y": 100, "text": "Same", "size": 24})).unwrap()["id"].as_u64().unwrap();
    let ctx = egui::Context::default();
    typing_frame(&mut app, &ctx, vec![]);
    app.select_tool("type");
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let center = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap().center();
    let at = xf.to_screen(center);
    let click =
        |p: Pos2, pressed| egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    typing_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), click(at, true)]);
    typing_frame(&mut app, &ctx, vec![click(at, false)]);
    let first = typing_frame(&mut app, &ctx, vec![]).expect("the text is being edited");
    assert_eq!(first.owner, egui::Id::unique(("vectorcraft-type-ime", app.session.active().unwrap().uid, id)));
    let escape = |pressed| egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    typing_frame(&mut app, &ctx, vec![escape(true)]);
    typing_frame(&mut app, &ctx, vec![escape(false)]);
    assert!(typing_frame(&mut app, &ctx, vec![]).is_none(), "Escape ends the edit context");
    app.select_tool("type");
    typing_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), click(at, true)]);
    typing_frame(&mut app, &ctx, vec![click(at, false)]);
    let second = typing_frame(&mut app, &ctx, vec![]).expect("re-entering starts another context");
    assert_eq!(second.owner, first.owner, "the owner key is the document and text object");
    assert_ne!(second.context_generation, first.context_generation, "a replacement epoch resets once");
}

/// One tracker serves several native viewports: each gets its own generation for the same
/// editor, and an unchanged second pass in either keeps it.
#[test]
fn ime_context_is_scoped_per_viewport() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    type_at(&mut app, &ctx);
    let aux = egui::ViewportId::from_hash_of("vectorcraft-ime-aux");
    let frame = |app: &mut VectorcraftApp, viewport: egui::ViewportId| {
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
            viewport_id: viewport,
            ..Default::default()
        };
        raw.viewports.entry(viewport).or_default();
        let mut out = ctx.run_ui(raw, |ui| {
            crate::shortcuts::handle(app, ui.ctx());
            show(app, ui);
            crate::ime::end_of_pass(app, ui.ctx());
        });
        out.textures_delta.clear();
        out.platform_output.ime.expect("the Type tool owns the IME in both viewports")
    };
    let root = egui::ViewportId::ROOT;
    let root_first = frame(&mut app, root);
    let aux_first = frame(&mut app, aux);
    assert_ne!(root_first.context_generation, aux_first.context_generation, "viewport state must not collide");
    let root_second = frame(&mut app, root);
    let aux_second = frame(&mut app, aux);
    assert_eq!(root_second.context_generation, root_first.context_generation);
    assert_eq!(aux_second.context_generation, aux_first.context_generation);
}

/// A minimized/occluded viewport has no IME owner: the descriptor disappears, and the next
/// visible pass starts a new generation instead of inheriting the old context.
#[test]
fn hidden_viewport_observes_no_ime_owner() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    type_at(&mut app, &ctx);
    let root = egui::ViewportId::ROOT;
    let visible = typing_frame(&mut app, &ctx, vec![]).unwrap();
    let mut raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
    raw.viewports.entry(root).or_default().minimized = Some(true);
    let mut out = ctx.run_ui(raw, |ui| {
        crate::shortcuts::handle(&mut app, ui.ctx());
        show(&mut app, ui);
        crate::ime::end_of_pass(&mut app, ui.ctx());
    });
    out.textures_delta.clear();
    assert!(out.platform_output.ime.is_none(), "a hidden viewport has no IME owner");
    let again = typing_frame(&mut app, &ctx, vec![]).expect("the visible editor comes back");
    assert_ne!(again.context_generation, visible.context_generation, "refocus starts a new generation");
}

/// A frame that is discarded and drawn again sees the mutation in both passes, and its
/// unchanged second pass keeps the first pass's `InputMethod` cause and generation.
#[test]
fn ime_observation_runs_in_every_pass_of_a_discarded_frame() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    type_at(&mut app, &ctx);
    let mut first = true;
    let mut seen = vec![];
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| {
        if first {
            first = false;
            let view = app.view_info();
            let target = crate::ime::target(&mut app.session, view);
            let before = crate::ime::stamp(&mut app.session, view);
            app.session.tool_text("x", view).unwrap();
            crate::ime::note_ime_apply(&mut app, ui.ctx(), view, target, before);
            ui.ctx().request_discard("ime observation second pass");
        }
        crate::shortcuts::handle(&mut app, ui.ctx());
        show(&mut app, ui);
        crate::ime::end_of_pass(&mut app, ui.ctx());
        seen.push(ui.ctx().output(|o| o.ime.clone()).expect("IME output in this pass"));
    });
    out.textures_delta.clear();
    assert_eq!(seen.len(), 2, "the frame really ran two passes");
    assert_eq!(seen[0].context_generation, seen[1].context_generation, "the discarded pass is not a new context");
    assert_eq!(seen[0].change_cause, egui::output::ImeChangeCause::InputMethod);
    assert_eq!(seen[1].change_cause, egui::output::ImeChangeCause::InputMethod, "an unchanged pass retains the frame cause");
    let payload = seen[1].surrounding_text.available().unwrap();
    assert_eq!((payload.text.as_str(), payload.cursor, payload.anchor), ("x", 1, 1));
}

#[test]
fn tool_drag_moved_without_pointer_delta() {
    let ctx = egui::Context::default();
    let rect = egui::Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
    let v = View::default();
    let xf = Xf::new(rect, &v);
    let a = pos2(100.0, 100.0);
    let b = pos2(150.0, 120.0);
    let mut first = false;
    let mut second = false;
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(rect), events: vec![egui::Event::PointerMoved(a)], ..Default::default() }, |ui| {
        let pointer = ui.input(|i| i.pointer.clone());
        first = tool_drag_moved(ui, &xf, a, &pointer);
        second = tool_drag_moved(ui, &xf, a, &pointer);
    });
    out.textures_delta.clear();
    assert!(first, "the first drag sample at a position runs");
    assert!(!second, "the same position without a delta is not sent again");
    let mut moved = false;
    out = ctx.run_ui(egui::RawInput { screen_rect: Some(rect), events: vec![egui::Event::PointerMoved(b)], ..Default::default() }, |ui| {
        let pointer = ui.input(|i| i.pointer.clone());
        moved = tool_drag_moved(ui, &xf, b, &pointer);
    });
    out.textures_delta.clear();
    assert!(moved, "a new document position runs even when egui reports no delta");
}

/// #834 review: a press held still is not a drag. Before the press has dragged, the first held
/// frame without a delta at the press's own position sends nothing, so a click stays a click;
/// the first move after it does.
#[test]
fn stationary_press_is_not_a_drag() {
    let ctx = egui::Context::default();
    let rect = egui::Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
    let v = View::default();
    let xf = Xf::new(rect, &v);
    let a = pos2(100.0, 100.0);
    let b = pos2(150.0, 120.0);
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(rect), events: vec![egui::Event::PointerMoved(a)], ..Default::default() }, |ui| {
        ui.data_mut(|d| {
            d.remove::<Point>(tool_drag_doc_id());
            d.insert_temp(tool_press_doc_id(), xf.to_doc(a));
        });
    });
    out.textures_delta.clear();
    let mut held = true;
    out = ctx.run_ui(egui::RawInput { screen_rect: Some(rect), ..Default::default() }, |ui| {
        let pointer = ui.input(|i| i.pointer.clone());
        assert_eq!(pointer.delta(), egui::Vec2::ZERO);
        held = tool_drag_moved(ui, &xf, a, &pointer);
    });
    out.textures_delta.clear();
    assert!(!held, "a press held still at its own position is not sent as a drag");
    let mut moved = false;
    out = ctx.run_ui(egui::RawInput { screen_rect: Some(rect), events: vec![egui::Event::PointerMoved(b)], ..Default::default() }, |ui| {
        let pointer = ui.input(|i| i.pointer.clone());
        moved = tool_drag_moved(ui, &xf, b, &pointer);
    });
    out.textures_delta.clear();
    assert!(moved, "the first move after the press is a drag");
}

/// #834: a bezier drag's preview follows every position the pointer is seen at. egui's frame
/// delta is zero in the frame the pointer comes back into the window after leaving it
/// (`PointerGone` forgets where it was), so a drag gated on it kept the preview where the
/// pointer left until the next move; a move in the release's own frame was lost the same way,
/// and the release committed the stale shape. A click's jitter in its release frame stays a
/// click.
#[test]
fn a_bezier_drag_preview_follows_the_pointer_back_into_the_window_and_to_the_release() {
    use egui::{Event, Modifiers};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.ui.view.smart_guides = false;
    app.ui.view.snap_to_point = false;
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    let near = |p: Point, q: Pos2| p.distance(xf.to_doc(q)) < 1e-3;
    let first_anchor = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        st.doc.node(st.selection.objects[0]).unwrap().path_data().unwrap().subpaths[0].anchors[0]
    };
    // The Pen pulls a handle out of a new anchor: the pointer leaves the window mid-drag, comes
    // back further on and stops there, then moves once more in the frame it lets go.
    app.select_tool("pen");
    let a = app.canvas_rect.unwrap().center() - vec2(100.0, 60.0);
    frame(&mut app, &ctx, vec![Event::PointerMoved(a)]);
    frame(&mut app, &ctx, vec![button(a, true)]);
    frame(&mut app, &ctx, vec![Event::PointerMoved(a + vec2(40.0, 0.0))]);
    assert!(near(first_anchor(&app).h_out, a + vec2(40.0, 0.0)), "{:?}", first_anchor(&app));
    frame(&mut app, &ctx, vec![Event::PointerGone]);
    frame(&mut app, &ctx, vec![Event::PointerMoved(a + vec2(80.0, 0.0))]);
    assert!(near(first_anchor(&app).h_out, a + vec2(80.0, 0.0)), "back in the window: {:?}", first_anchor(&app));
    frame(&mut app, &ctx, vec![]);
    assert!(near(first_anchor(&app).h_out, a + vec2(80.0, 0.0)), "still: {:?}", first_anchor(&app));
    let end = a + vec2(100.0, 20.0);
    frame(&mut app, &ctx, vec![Event::PointerMoved(end), button(end, false)]);
    assert!(near(first_anchor(&app).h_out, end), "released: {:?}", first_anchor(&app));
    assert!(app.session.active().unwrap().interaction.is_none(), "committed");
    // Direct Selection moving that anchor: the same path through the engine's previews.
    app.select_tool("directSelection");
    let anchor_at = xf.to_screen(first_anchor(&app).p);
    frame(&mut app, &ctx, vec![Event::PointerMoved(anchor_at)]);
    frame(&mut app, &ctx, vec![button(anchor_at, true)]);
    frame(&mut app, &ctx, vec![Event::PointerMoved(anchor_at + vec2(30.0, 30.0))]);
    assert!(near(first_anchor(&app).p, anchor_at + vec2(30.0, 30.0)), "{:?}", first_anchor(&app));
    frame(&mut app, &ctx, vec![Event::PointerGone]);
    frame(&mut app, &ctx, vec![Event::PointerMoved(anchor_at + vec2(60.0, 30.0))]);
    assert!(near(first_anchor(&app).p, anchor_at + vec2(60.0, 30.0)), "back in the window: {:?}", first_anchor(&app));
    let end = anchor_at + vec2(60.0, 50.0);
    frame(&mut app, &ctx, vec![Event::PointerMoved(end), button(end, false)]);
    assert!(near(first_anchor(&app).p, end), "released: {:?}", first_anchor(&app));
    // A click whose release frame brings a little jitter is still a click: the Pen places a
    // corner anchor (no handle pulled out by the jitter) and the anchor stays where pressed.
    app.session.execute("select.none", &json!({})).unwrap();
    app.select_tool("pen");
    let c = app.canvas_rect.unwrap().center() + vec2(120.0, 80.0);
    frame(&mut app, &ctx, vec![Event::PointerMoved(c)]);
    frame(&mut app, &ctx, vec![button(c, true), Event::PointerMoved(c + vec2(0.6, 0.2)), button(c + vec2(0.6, 0.2), false)]);
    frame(&mut app, &ctx, vec![]);
    // (egui hands the press the frame's last pointer position, so it lands within the jitter.)
    let a = first_anchor(&app);
    assert!(a.p.distance(xf.to_doc(c)) < 1.0 && a.h_out == a.p && a.h_in == a.p, "corner anchor where pressed: {a:?}");
    assert!(app.session.active().unwrap().interaction.is_none(), "committed");
}

#[test]
fn frame_moves_keeps_touch_feed_and_mouse_samples_with_their_pressure() {
    let ctx = egui::Context::default();
    let a = pos2(10.0, 20.0);
    let b = pos2(20.0, 30.0);
    let c = pos2(30.0, 40.0);
    let events = vec![egui::Event::PointerMoved(a), egui::Event::PointerMoved(b), egui::Event::PointerMoved(c)];
    let mut mouse = vec![];
    let mut out = ctx.run_ui(egui::RawInput { events: events.clone(), ..Default::default() }, |ui| mouse = frame_moves(ui, None));
    out.textures_delta.clear();
    // A mouse keeps its moves too, without a pressure.
    assert_eq!(mouse, [(a, None), (b, None)], "the last move follows the normal pointer path");
    // A macOS pen: the feed kept a pressure for each move, the last for the last.
    let feed = StylusFeed::default();
    for p in [0.9, 0.3, 0.0, 0.5] {
        feed.set_move(crate::stylus::PenSample { pressure: p, eraser: false });
    }
    let mut samples = vec![];
    let mut out = ctx.run_ui(egui::RawInput { events: events.clone(), ..Default::default() }, |ui| samples = frame_moves(ui, Some(&feed)));
    out.textures_delta.clear();
    assert_eq!(samples, [(a, Some(0.3)), (b, None)], "paired from the end; a lift's 0 is no pressure");
    let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| samples = frame_moves(ui, Some(&feed)));
    out.textures_delta.clear();
    assert_eq!(samples, [(a, None), (b, None)], "the moves were taken: a mouse's moves again");
    let events = [pen(egui::TouchPhase::Move, a, 0.2), pen(egui::TouchPhase::Move, b, 0.8), pen(egui::TouchPhase::Move, c, 0.6)].concat();
    let mut samples = vec![];
    let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| samples = frame_moves(ui, None));
    out.textures_delta.clear();
    assert_eq!(samples, [(a, Some(0.2)), (b, Some(0.8))], "the last sample follows the normal pointer path");
    let events = (0..200).flat_map(|n| pen(egui::TouchPhase::Move, pos2(n as f32, 0.0), 0.5)).collect();
    let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| samples = frame_moves(ui, None));
    out.textures_delta.clear();
    assert!(samples.len() <= 64, "a stalled frame stays bounded");
}

#[test]
fn long_touch_drops_a_tool_press_without_committing_art() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width":800,"height":600})).unwrap();
    app.select_tool("rectangle");
    let ctx = egui::Context::default();
    let tick = |app: &mut VectorcraftApp, time, events| {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
    };
    tick(&mut app, 0.0, vec![]);
    let pos = app.canvas_rect.unwrap().center();
    tick(&mut app, 0.1, pen(egui::TouchPhase::Start, pos, 0.5));
    assert!(matches!(ctx.data(|d| d.get_temp::<Drag>(drag_id())), Some(Drag::Tool)));
    tick(&mut app, 1.5, vec![]);
    assert!(matches!(ctx.data(|d| d.get_temp::<Drag>(drag_id())), Some(Drag::Gesture)), "long touch drops the tool drag");
    assert!(app.session.active().unwrap().interaction.is_none());
    tick(&mut app, 1.6, pen(egui::TouchPhase::End, pos, 0.5));
    assert!(app.session.active().unwrap().doc.art_bounds().is_none(), "releasing cannot commit the cancelled press");
}

/// #2826: a second finger drops a move waiting over a panel, so its interaction doesn't
/// outlive the drag that began it.
#[test]
fn a_second_finger_drops_a_move_waiting_over_a_panel() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.prefs.touch_gestures = true;
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap();
    app.select_tool("selection");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let before = app.session.active().unwrap().doc.clone();
    let start = xf.to_screen(Point::new(100.0, 100.0));
    // One finger presses on the art, moves it, and leaves the canvas for a panel.
    frame(&mut app, &ctx, pen(egui::TouchPhase::Start, start, 0.5));
    frame(&mut app, &ctx, pen(egui::TouchPhase::Move, start + vec2(40.0, 0.0), 0.5));
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(pos2(850.0, 100.0))]);
    assert!(matches!(ctx.data(|d| d.get_temp::<Drag>(drag_id())), Some(Drag::Art)));
    assert!(app.session.active().unwrap().interaction.is_some());
    assert!(egui::DragAndDrop::payload::<widgets::PanelDrag>(&ctx).is_some());
    // A second finger lands: the move is dropped, not left live without its pointer.
    let second = egui::Event::Touch {
        device_id: egui::TouchDeviceId(1),
        id: egui::TouchId(2),
        phase: egui::TouchPhase::Start,
        pos: pos2(400.0, 300.0),
        force: Some(0.5),
    };
    frame(&mut app, &ctx, vec![second]);
    assert!(matches!(ctx.data(|d| d.get_temp::<Drag>(drag_id())), Some(Drag::Gesture)));
    let st = app.session.active().unwrap();
    assert!(st.interaction.is_none(), "the move is dropped");
    assert_eq!(*st.doc, *before, "the art is back where it was");
    assert!(egui::DragAndDrop::payload::<widgets::PanelDrag>(&ctx).is_none(), "no panel drag is left");
    // Lifting the first finger commits nothing.
    frame(&mut app, &ctx, pen(egui::TouchPhase::End, start + vec2(40.0, 0.0), 0.5));
    assert_eq!(*app.session.active().unwrap().doc, *before);
}

/// The events eframe's winit integration sends for a pen (Windows Ink) or a finger: a touch
/// with its pressure, and the mouse it stands in for (#491).
fn pen(phase: egui::TouchPhase, pos: Pos2, force: f32) -> Vec<egui::Event> {
    let touch = egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(1), phase, pos, force: Some(force) };
    let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    match phase {
        egui::TouchPhase::Start => vec![touch, egui::Event::PointerMoved(pos), button(true)],
        egui::TouchPhase::Move => vec![touch, egui::Event::PointerMoved(pos)],
        egui::TouchPhase::End | egui::TouchPhase::Cancel => vec![touch, button(false), egui::Event::PointerGone],
    }
}

/// A pen stroke across `pts` (document points), one frame per sample, with its pressures.
fn pen_stroke(app: &mut VectorcraftApp, ctx: &egui::Context, pts: &[(f64, f64, f32)]) {
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let last = pts.len() - 1;
    for (i, &(x, y, force)) in pts.iter().enumerate() {
        let phase = if i == 0 { egui::TouchPhase::Start } else { egui::TouchPhase::Move };
        frame(app, ctx, pen(phase, xf.to_screen(Point::new(x, y)), force));
        if i == last {
            frame(app, ctx, pen(egui::TouchPhase::End, xf.to_screen(Point::new(x, y)), force));
        }
    }
}

/// #491: a pen or a finger draws as the mouse does, and its pressure reaches the tools that
/// use it; the pointer leaving with the lift doesn't move the stroke's end.
#[test]
fn pen_and_touch_input_drive_the_tools_with_their_pressure() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    app.select_tool("pencil");
    pen_stroke(&mut app, &ctx, &[(100.0, 100.0, 0.5), (140.0, 120.0, 0.6), (180.0, 150.0, 0.7), (220.0, 200.0, 0.4)]);
    let b = app.session.active().unwrap().doc.art_bounds().expect("the pen drew a path");
    assert!(b.x0 > 90.0 && b.y0 > 90.0 && b.x1 < 230.0 && b.y1 < 210.0 && b.x1 > 200.0, "the stroke stays where the pen went: {b:?}");

    // Bloat with Use Pressure Pen: each sample carries the pen's pressure.
    app.session.execute("shape.rectangle", &json!({"x": 300, "y": 300, "width": 200, "height": 200})).unwrap();
    app.select_tool("bloat");
    app.session.set_tool_option("usePressure", &json!(true));
    pen_stroke(&mut app, &ctx, &[(500.0, 350.0, 0.2), (500.0, 400.0, 0.5), (500.0, 420.0, 0.8)]);
    let (cmd, p) = app.session.journal.last().unwrap().clone();
    assert_eq!(cmd, "object.liquify");
    let pressures: Vec<f64> = p["points"].as_array().unwrap().iter().map(|s| (s[2].as_f64().unwrap() * 100.0).round() / 100.0).collect();
    assert_eq!(pressures.first(), Some(&0.2), "the press's own pressure: {pressures:?}");
    assert!(pressures.contains(&0.5) && pressures.contains(&0.8), "{pressures:?}");
}

/// #764: on Linux under X11 a pen reaches egui as a mouse, and the host reads its pressure
/// itself (`Services::stylus`): the Paintbrush and Liquify's Use Pressure Pen get that
/// pressure, and once a mouse moves (`None`) a stroke presses fully.
#[test]
fn the_hosts_pen_pressure_drives_the_tools_under_mouse_events() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let feed = StylusFeed::default();
    app.services.stylus = Some(feed.clone());
    app.session.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    // One frame per sample: the reader has set the pressure before the frame's mouse events.
    let stroke = |app: &mut VectorcraftApp, pts: &[(f64, f64, Option<f32>)]| {
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let mut at = Pos2::ZERO;
        for (i, &(x, y, pressure)) in pts.iter().enumerate() {
            feed.set(pressure);
            at = xf.to_screen(Point::new(x, y));
            let mut events = vec![egui::Event::PointerMoved(at)];
            if i == 0 {
                events.push(button(at, true));
            }
            frame(app, &ctx, events);
        }
        frame(app, &ctx, vec![button(at, false)]);
    };
    let pressures = |p: &serde_json::Value| -> Vec<f64> {
        // A point without a third value was pressed fully.
        let pressure = |s: &serde_json::Value| s.get(2).and_then(serde_json::Value::as_f64).map_or(1.0, |f| (f * 100.0).round() / 100.0);
        p["points"].as_array().unwrap().iter().map(pressure).collect()
    };

    app.select_tool("paintbrush");
    stroke(&mut app, &[(100.0, 100.0, Some(0.3)), (140.0, 120.0, Some(0.6)), (180.0, 150.0, Some(0.9))]);
    let (cmd, p) = app.session.journal.last().unwrap().clone();
    assert_eq!(cmd, "path.freehand");
    assert_eq!(pressures(&p), [0.3, 0.6, 0.9], "{p}");
    // The lift reaches the feed before the frame of the stroke's last move: that move keeps the
    // pressure the pen last touched with, not the lift's 0.
    stroke(&mut app, &[(100.0, 300.0, Some(0.3)), (140.0, 320.0, Some(0.5)), (180.0, 350.0, Some(0.0))]);
    let (cmd, p) = app.session.journal.last().unwrap().clone();
    assert_eq!(cmd, "path.freehand");
    assert_eq!(pressures(&p), [0.3, 0.5, 0.5], "{p}");

    app.session.execute("shape.rectangle", &json!({"x": 300, "y": 300, "width": 200, "height": 200})).unwrap();
    app.select_tool("bloat");
    app.session.set_tool_option("usePressure", &json!(true));
    stroke(&mut app, &[(500.0, 350.0, Some(0.2)), (500.0, 400.0, Some(0.5)), (500.0, 420.0, Some(0.8))]);
    let (cmd, p) = app.session.journal.last().unwrap().clone();
    assert_eq!(cmd, "object.liquify");
    let pen = pressures(&p);
    assert_eq!(pen.first(), Some(&0.2), "the press's own pressure: {pen:?}");
    assert!(pen.contains(&0.5) && pen.contains(&0.8), "{pen:?}");

    // A mouse moved last: no pen pressure.
    stroke(&mut app, &[(350.0, 350.0, None), (350.0, 400.0, None), (350.0, 420.0, None)]);
    let (cmd, p) = app.session.journal.last().unwrap().clone();
    assert_eq!(cmd, "object.liquify");
    assert!(pressures(&p).iter().all(|f| *f == 1.0), "{p}");

    // A tap whose touch and lift both reach the feed before the frame that handles its press
    // presses with the tap's pressure, not the lift's 0.
    for p in [0.0, 0.4, 0.7, 0.0] {
        feed.set(Some(p));
    }
    let at = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap()).to_screen(Point::new(450.0, 400.0));
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), button(true)]);
    frame(&mut app, &ctx, vec![button(false)]);
    let (cmd, p) = app.session.journal.last().unwrap().clone();
    assert_eq!(cmd, "object.liquify");
    assert_eq!(pressures(&p).first(), Some(&0.7), "{p}");
}

/// An app with an 800 × 600 document whose host reads the pen itself (`Services::stylus`),
/// after its first frame, with the pointer in the window.
fn pen_app() -> (VectorcraftApp, StylusFeed, egui::Context) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let feed = StylusFeed::default();
    app.services.stylus = Some(feed.clone());
    app.session.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(Pos2::new(400.0, 300.0))]);
    (app, feed, ctx)
}

/// The pen turned to its eraser end, with `pressure`.
fn eraser_end(pressure: f32) -> Option<crate::stylus::PenSample> {
    Some(crate::stylus::PenSample { pressure, eraser: true })
}

/// #764: turning the pen to its eraser end selects the Eraser, and turning it back to the tip
/// selects the tool before it, as the reference app's help describes ("Erase parts of a path
/// using a stylus pen"). Each turn acts once.
#[test]
fn the_eraser_end_selects_the_eraser_and_the_tip_brings_back_the_tool() {
    let (mut app, feed, ctx) = pen_app();
    app.select_tool("paintbrush");
    feed.set(Some(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "paintbrush", "the tip hovers");
    feed.set_sample(eraser_end(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "eraser", "the eraser end hovers");
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "eraser");
    feed.set(Some(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "paintbrush", "the tip brings back the tool before");
    assert_eq!(app.ui.group_tool.get(vectorcraft_tools::catalog::group_of("paintbrush").unwrap()).map(String::as_str), Some("paintbrush"));

    // A turn acts once: a tool that becomes active while the eraser end stays in stays, and
    // the tip doesn't replace a tool other than the Eraser.
    feed.set_sample(eraser_end(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "eraser");
    app.session.select_tool("pencil", app.view_info()).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "pencil");
    feed.set(Some(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "pencil");
}

/// #764: the eraser end erases what it crosses, also when it comes in and presses in one
/// frame; the tool before it does not paint a stroke.
#[test]
fn the_eraser_end_erases() {
    let (mut app, feed, ctx) = pen_app();
    app.session.execute("shape.rectangle", &json!({"x": 300, "y": 200, "width": 100, "height": 100})).unwrap();
    app.select_tool("paintbrush");
    let steps = app.session.journal.len();
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let [a, b, c] = [280.0, 350.0, 420.0].map(|x| xf.to_screen(Point::new(x, 250.0)));
    feed.set_sample(eraser_end(0.6));
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(a), button(a, true)]);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(b)]);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(c)]);
    frame(&mut app, &ctx, vec![button(c, false)]);
    let cmds: Vec<&str> = app.session.journal.since(steps).iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(cmds, ["path.eraseRegion"], "one Eraser step, no Paintbrush stroke");
    assert_eq!(app.session.active().unwrap().doc.layers[0].children().unwrap().len(), 2, "the rectangle cut in two");
}

/// #764: a turn of the pen while a press is held takes effect after the release.
#[test]
fn a_flip_during_a_press_waits_for_the_release() {
    let (mut app, feed, ctx) = pen_app();
    app.select_tool("paintbrush");
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let [a, b, c] = [100.0, 150.0, 200.0].map(|x| xf.to_screen(Point::new(x, 100.0)));
    feed.set(Some(0.5));
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(a), button(a, true)]);
    feed.set_sample(eraser_end(0.5));
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(b)]);
    assert_eq!(app.session.tool_id(), "paintbrush");
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(c), button(c, false)]);
    assert_eq!(app.session.tool_id(), "paintbrush");
    assert_eq!(app.session.journal.last().unwrap().0, "path.freehand", "the stroke ends with the Paintbrush");
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "eraser", "the frame after the release");
}

/// #764: a tool chosen while the eraser end stands in (the toolbar and shortcuts call
/// `select_tool`; menus and MCP run `tool.select`) stays when the tip comes back.
#[test]
fn a_tool_chosen_while_the_eraser_end_stands_in_stays() {
    let (mut app, feed, ctx) = pen_app();
    app.select_tool("paintbrush");
    feed.set_sample(eraser_end(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "eraser");
    app.select_tool("rectangle");
    feed.set(Some(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "rectangle");

    feed.set_sample(eraser_end(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "eraser");
    app.run("tool.select", json!({"tool": "eraser"})).unwrap();
    feed.set(Some(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "eraser", "the Eraser chosen again stays too");
}

/// #764: a mouse moving while the eraser end stands in does not change the tool, and an Eraser
/// chosen before the eraser end comes in stays when the tip comes back.
#[test]
fn a_mouse_and_an_explicit_eraser_change_nothing() {
    let (mut app, feed, ctx) = pen_app();
    app.select_tool("paintbrush");
    feed.set(None);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "paintbrush", "a mouse");
    feed.set_sample(eraser_end(0.0));
    frame(&mut app, &ctx, vec![]);
    feed.set(None);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "eraser", "a mouse between the turns");
    feed.set(Some(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "paintbrush");

    app.select_tool("eraser");
    feed.set_sample(eraser_end(0.0));
    frame(&mut app, &ctx, vec![]);
    feed.set(Some(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "eraser", "chosen before the eraser end came in");
}

/// #764: the host reads the pen over every window. A turn made while the pointer is outside
/// this window takes effect when the pointer comes in, and a turn there and back does not
/// change the tool.
#[test]
fn a_turn_over_another_window_waits_for_the_pointer() {
    let (mut app, feed, ctx) = pen_app();
    app.select_tool("paintbrush");
    feed.set(Some(0.0));
    frame(&mut app, &ctx, vec![]);
    let inside = egui::Event::PointerMoved(Pos2::new(400.0, 300.0));
    frame(&mut app, &ctx, vec![egui::Event::PointerGone]);
    feed.set_sample(eraser_end(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "paintbrush", "the pointer is in another window");
    frame(&mut app, &ctx, vec![inside.clone()]);
    assert_eq!(app.session.tool_id(), "eraser", "the pointer came in");

    frame(&mut app, &ctx, vec![egui::Event::PointerGone]);
    feed.set(Some(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "eraser");
    frame(&mut app, &ctx, vec![inside.clone()]);
    assert_eq!(app.session.tool_id(), "paintbrush");

    frame(&mut app, &ctx, vec![egui::Event::PointerGone]);
    feed.set_sample(eraser_end(0.0));
    frame(&mut app, &ctx, vec![]);
    feed.set(Some(0.0));
    frame(&mut app, &ctx, vec![inside]);
    assert_eq!(app.session.tool_id(), "paintbrush", "turned there and back outside");
}

/// #764: the tip brings back the Perspective Grid tool as it was, with a grid hidden by Hide
/// Grid still hidden (choosing the tool afresh shows the grid).
#[test]
fn the_tip_brings_back_the_perspective_grid_tool_without_showing_the_grid() {
    let (mut app, feed, ctx) = pen_app();
    let visible =
        |app: &VectorcraftApp| vectorcraft_tools::distort::perspective::PerspectiveGrid::current(&app.session.active().unwrap().doc).visible;
    app.select_tool("perspectiveGrid");
    assert!(visible(&app));
    app.session.execute("perspective.grid.show", &json!({"visible": false})).unwrap();
    feed.set(Some(0.0));
    frame(&mut app, &ctx, vec![]);
    feed.set_sample(eraser_end(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "eraser");
    feed.set(Some(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "perspectiveGrid");
    assert!(!visible(&app), "the grid stays hidden");
}

/// #3328: every position a mouse reports within one frame reaches the Pencil, not only the
/// frame's last one, so a fast stroke keeps the hand's movement.
#[test]
fn a_mouse_stroke_keeps_every_move_of_a_frame() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    app.select_tool("pencil");
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let at = |x: f64, y: f64| xf.to_screen(Point::new(x, y));
    let button = |p, pressed| egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    let start = at(100.0, 100.0);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start), button(start, true)]);
    // One frame of a fast stroke: a zigzag of 20 moves, 6 pt apart.
    let zigzag: Vec<_> = (1..=20).map(|i| at(100.0 + 6.0 * f64::from(i), if i % 2 == 0 { 100.0 } else { 112.0 })).collect();
    frame(&mut app, &ctx, zigzag.iter().map(|p| egui::Event::PointerMoved(*p)).collect());
    let end = *zigzag.last().unwrap();
    frame(&mut app, &ctx, vec![button(end, false)]);
    let (cmd, p) = app.session.journal.last().unwrap().clone();
    assert_eq!(cmd, "path.freehand");
    let pts = p["points"].as_array().unwrap();
    let expected: Vec<_> = std::iter::once(start)
        .chain(zigzag.iter().copied())
        .map(|p| {
            let p = xf.to_doc(p);
            json!([p.x, p.y])
        })
        .collect();
    assert_eq!(pts, expected.as_slice(), "the stroke keeps every move of the frame, in order");
}

/// A frame whose moves end where the last frame's did still reaches the Pencil: the stroke went
/// out and came back in between.
#[test]
fn a_mouse_stroke_keeps_a_frame_that_ends_where_it_started() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    app.select_tool("pencil");
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let at = |x: f64, y: f64| xf.to_screen(Point::new(x, y));
    let button = |p, pressed| egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    let (start, a) = (at(100.0, 100.0), at(160.0, 100.0));
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start), button(start, true)]);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(a)]);
    // Out to a bump and back to `a`, all in one frame.
    let bump = [at(170.0, 80.0), at(180.0, 70.0), at(170.0, 80.0), a];
    frame(&mut app, &ctx, bump.iter().map(|p| egui::Event::PointerMoved(*p)).collect());
    frame(&mut app, &ctx, vec![button(a, false)]);
    let (_, p) = app.session.journal.last().unwrap().clone();
    let n = p["points"].as_array().unwrap().len();
    assert!(n >= 5, "the bump's moves are kept: {n} points");
}

/// Only a tool that builds a stroke from the drag's samples gets every move of a frame (#4227):
/// the Selection tool and the other tools get one drag a frame, at the pointer's position, so a
/// costly drag preview runs once.
#[test]
fn only_the_freehand_tools_get_every_move_of_a_frame() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    let ctx = egui::Context::default();
    let events: Vec<_> = (0..20).map(|i| egui::Event::PointerMoved(pos2(100.0 + i as f32 * 5.0, 100.0))).collect();
    for (tool, every) in
        [("pencil", true), ("paintbrush", true), ("blobBrush", true), ("selection", false), ("directSelection", false), ("width", false)]
    {
        app.select_tool(tool);
        let mut moves = vec![];
        let mut out = ctx.run_ui(egui::RawInput { events: events.clone(), ..Default::default() }, |ui| moves = tool_frame_moves(ui, &app));
        out.textures_delta.clear();
        assert_eq!(moves.len(), if every { 19 } else { 0 }, "{tool}");
    }
}

/// #491: a pen tap whose press and lift come in one frame selects what it tapped, and the
/// lift (the pointer gone with it) doesn't drag the selection to the canvas's centre.
#[test]
fn a_pen_tap_within_one_frame_clicks_where_it_tapped() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.none", &json!({})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    app.select_tool("selection");
    let before = app.session.active().unwrap().doc.art_bounds();
    let at = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap()).to_screen(Point::new(150.0, 150.0));
    let mut tap = pen(egui::TouchPhase::Start, at, 0.5);
    tap.extend(pen(egui::TouchPhase::End, at, 0.5));
    frame(&mut app, &ctx, tap);
    frame(&mut app, &ctx, vec![]);
    let doc = app.session.active().unwrap();
    assert_eq!(doc.selection.objects, vec![vectorcraft_doc::NodeId(id)]);
    assert_eq!(doc.doc.art_bounds(), before, "the tap moved nothing");
}

/// The Contextual Task Bar shows a Stroke chip beside the Fill chip, drawn with the selection's
/// stroke paint. A click on it brings the Stroke proxy forward and opens Swatches, as a click on
/// the Fill chip does for the Fill proxy.
#[test]
fn the_task_bar_shows_a_stroke_chip_beside_fill() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 180, "y": 130, "width": 40, "height": 40})).unwrap()["id"].clone();
    app.session.execute("paint.setFill", &json!({"ids": [id], "color": "#cc3322"})).unwrap();
    app.session.execute("paint.setStroke", &json!({"ids": [id], "color": "#2244cc"})).unwrap();
    app.select_tool("selection");
    let ctx = egui::Context::default();
    // Past the bar's fade-in, so its chips draw at full opacity.
    for _ in 0..30 {
        frame(&mut app, &ctx, vec![]);
    }
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| show(&mut app, ui));
    out.textures_delta.clear();
    let bar = task_bar_rect(&ctx).expect("the task bar shows");
    // Right to left: the frame's margin, More Options, Lock, Recolor, then Stroke and Fill.
    let stroke_x = bar.right() - 8.0 - 3.0 * (30.0 + 6.0) - 13.0;
    let fill_x = stroke_x - 26.0 - 6.0;
    let blue = Color32::from_rgb(0x22, 0x44, 0xcc);
    let chip_with = |x: f32| {
        out.shapes.iter().any(|c| match &c.shape {
            egui::Shape::Rect(r) => r.fill == blue && bar.contains(r.rect.center()) && (r.rect.center().x - x).abs() < 4.0,
            _ => false,
        })
    };
    assert!(chip_with(stroke_x), "the Stroke chip is drawn with the stroke color");
    assert!(!chip_with(fill_x), "the Fill chip is not");

    let click = |app: &mut VectorcraftApp, x: f32| {
        let at = pos2(x, bar.center().y);
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(app, &ctx, vec![egui::Event::PointerMoved(at)]);
        frame(app, &ctx, vec![button(true)]);
        frame(app, &ctx, vec![button(false)]);
    };
    app.session.fill_active = true;
    click(&mut app, stroke_x);
    assert!(!app.session.fill_active, "the Stroke proxy is in front");
    assert_eq!(app.ui.open_panel.as_deref(), Some("swatches"));
    app.ui.open_panel = None;
    click(&mut app, fill_x);
    assert!(app.session.fill_active, "the Fill proxy is in front");
    assert_eq!(app.ui.open_panel.as_deref(), Some("swatches"));
}

/// The Contextual Task Bar's Fill and Stroke chips show the paints the proxies show: a group's
/// painted art, and "?" when the selected objects' paints differ. A click on a chip shows the
/// Swatches panel wherever it is, closed included.
#[test]
fn the_task_bar_chips_follow_the_proxies_and_show_swatches_anywhere() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let mut square = |x: f64, stroke: &str| {
        let id = app.session.execute("shape.rectangle", &json!({"x": x, "y": 130, "width": 40, "height": 40})).unwrap()["id"].clone();
        app.session.execute("paint.setFill", &json!({"ids": [id], "color": "#cc3322"})).unwrap();
        app.session.execute("paint.setStroke", &json!({"ids": [id], "color": stroke})).unwrap();
        id
    };
    square(160.0, "#2244cc");
    let second = square(220.0, "#22cc44");
    app.select_tool("selection");
    app.session.execute("select.all", &json!({})).unwrap();
    let ctx = egui::Context::default();
    let shapes = |app: &mut VectorcraftApp| {
        for _ in 0..30 {
            frame(app, &ctx, vec![]);
        }
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
        out.shapes
    };
    // Right to left: the frame's margin, More Options, Lock, Recolor, then Stroke and Fill.
    let stroke_x = |bar: egui::Rect| bar.right() - 8.0 - 3.0 * (30.0 + 6.0) - 13.0;
    let chip_with = |shapes: &[egui::epaint::ClippedShape], bar: egui::Rect, x: f32, color: Color32| {
        shapes.iter().any(|c| match &c.shape {
            egui::Shape::Rect(r) => r.fill == color && bar.contains(r.rect.center()) && (r.rect.center().x - x).abs() < 4.0,
            _ => false,
        })
    };
    let (blue, green) = (Color32::from_rgb(0x22, 0x44, 0xcc), Color32::from_rgb(0x22, 0xcc, 0x44));

    // Two strokes that differ: "?" on white, as the proxy shows them.
    let out = shapes(&mut app);
    let bar = task_bar_rect(&ctx).expect("the task bar shows");
    assert!(chip_with(&out, bar, stroke_x(bar), Color32::WHITE), "the Stroke chip shows the strokes differ");
    assert!(!chip_with(&out, bar, stroke_x(bar), blue), "the Stroke chip does not show the first object's stroke");

    // A group shows its painted art's stroke, as the proxy does.
    app.session.execute("paint.setStroke", &json!({"ids": [second], "color": "#2244cc"})).unwrap();
    app.session.execute("select.all", &json!({})).unwrap();
    app.session.execute("object.group", &json!({})).unwrap();
    let out = shapes(&mut app);
    let bar = task_bar_rect(&ctx).expect("the task bar shows");
    assert!(chip_with(&out, bar, stroke_x(bar), blue), "the Stroke chip shows the group's art's stroke");
    assert!(!chip_with(&out, bar, stroke_x(bar), green));

    // Swatches closed: a click on the Stroke chip shows it.
    app.run("window.panel.close", json!({"panel": "swatches"})).unwrap();
    assert!(!crate::panel_docking::showing(&app.ui, "swatches"));
    let at = pos2(stroke_x(bar), bar.center().y);
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
    frame(&mut app, &ctx, vec![button(true)]);
    frame(&mut app, &ctx, vec![button(false)]);
    frame(&mut app, &ctx, vec![]);
    assert!(!app.session.fill_active, "the Stroke proxy is in front");
    assert!(crate::panel_docking::showing(&app.ui, "swatches"), "the Swatches panel shows");
}

/// The Contextual Task Bar shows Recolor Artwork (a palette) when the selection has colours,
/// and it opens the Recolor Artwork dialog; art painted with None has no button.
#[test]
fn the_task_bar_recolors_coloured_art() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 180, "y": 130, "width": 40, "height": 40})).unwrap()["id"].clone();
    app.session.execute("paint.setFill", &json!({"ids": [id], "color": "#cc3322"})).unwrap();
    app.select_tool("selection");
    let ctx = egui::Context::default();
    let click = |app: &mut VectorcraftApp| {
        frame(app, &ctx, vec![]);
        frame(app, &ctx, vec![]);
        let bar = task_bar_rect(&ctx).expect("the task bar shows");
        // Right to left: the frame's margin, More Options, Lock, then Recolor.
        let at = pos2(bar.right() - 8.0 - 30.0 - 6.0 - 30.0 - 6.0 - 15.0, bar.center().y);
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(app, &ctx, vec![egui::Event::PointerMoved(at)]);
        frame(app, &ctx, vec![button(true)]);
        frame(app, &ctx, vec![button(false)]);
    };
    let st = app.session.active().unwrap();
    assert!(has_colors(&st.doc, &st.selection.objects));
    click(&mut app);
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(crate::dialogs::recolor::KIND));

    app.ui.dialog = None;
    app.session.execute("paint.setFill", &json!({"ids": [id], "none": true})).unwrap();
    app.session.execute("paint.setStroke", &json!({"ids": [id], "none": true})).unwrap();
    let st = app.session.active().unwrap();
    assert!(!has_colors(&st.doc, &st.selection.objects), "None paints have no colours");
    click(&mut app);
    assert!(app.ui.dialog.is_none(), "no Recolor button without colours");
}

/// The Contextual Task Bar's Lock button is off in isolation mode, as Object › Lock › Selection
/// is: a click locks nothing and shows no error. Outside isolation mode it locks the selection.
#[test]
fn the_task_bar_lock_button_is_off_in_isolation_mode() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let a = app.session.execute("shape.rectangle", &json!({"x": 150, "y": 130, "width": 40, "height": 40})).unwrap()["id"].clone();
    let b = app.session.execute("shape.rectangle", &json!({"x": 210, "y": 130, "width": 40, "height": 40})).unwrap()["id"].clone();
    app.session.execute("select.set", &json!({"ids": [a.clone(), b]})).unwrap();
    let g = app.session.execute("object.group", &json!({})).unwrap()["id"].clone();
    app.select_tool("selection");
    let ctx = egui::Context::default();
    let click_lock = |app: &mut VectorcraftApp| {
        frame(app, &ctx, vec![]);
        frame(app, &ctx, vec![]);
        let bar = task_bar_rect(&ctx).expect("the task bar shows");
        // Right to left: the frame's margin, More Options, then Lock.
        let at = pos2(bar.right() - 8.0 - 30.0 - 6.0 - 15.0, bar.center().y);
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(app, &ctx, vec![egui::Event::PointerMoved(at)]);
        frame(app, &ctx, vec![button(true)]);
        frame(app, &ctx, vec![button(false)]);
    };
    let locked = |app: &VectorcraftApp, id: &serde_json::Value| {
        app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().locked
    };
    app.session.execute("object.isolate", &json!({"id": g.clone()})).unwrap();
    app.session.execute("select.set", &json!({"ids": [a.clone()]})).unwrap();
    click_lock(&mut app);
    assert!(!locked(&app, &a), "nothing is locked in isolation mode");
    assert_eq!(app.ui.status, "", "no error");
    app.session.execute("object.exitIsolation", &json!({})).unwrap();
    app.session.execute("select.set", &json!({"ids": [g.clone()]})).unwrap();
    click_lock(&mut app);
    assert!(locked(&app, &g), "the group is locked outside isolation mode");
}

/// The Contextual Task Bar shows Image Trace first for a selected image: it traces the image
/// with the Default preset and leaves the Image Trace panel as it was. For the Image Trace
/// object it made, the first button is Expand, which keeps only the traced paths.
#[test]
fn the_task_bar_traces_an_image_then_expands_the_trace() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let img = image::RgbaImage::from_fn(20, 20, |x, _| image::Rgba(if x < 10 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let data = vectorcraft_format::base64_encode(&png);
    app.run("file.place", json!({"name": "half.png", "dataBase64": data, "link": false, "at": [200, 150]})).unwrap();
    app.select_tool("selection");
    let ctx = egui::Context::default();
    let click_first = |app: &mut VectorcraftApp| {
        frame(app, &ctx, vec![]);
        frame(app, &ctx, vec![]);
        let bar = task_bar_rect(&ctx).expect("the task bar shows");
        // Left to right: the frame's margin, the handle and its spacing, then the first button.
        let at = pos2(bar.left() + 8.0 + 4.0 + 6.0 + 12.0, bar.center().y);
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(app, &ctx, vec![egui::Event::PointerMoved(at)]);
        frame(app, &ctx, vec![button(true)]);
        frame(app, &ctx, vec![button(false)]);
    };
    let first = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        st.selection.objects.first().and_then(|id| st.doc.node(*id)).cloned().unwrap()
    };
    assert!(matches!(first(&app).kind, NodeKind::Image(_)));
    click_first(&mut app);
    assert_eq!(first(&app).name.as_deref(), Some("Image Trace"), "the image was traced");
    assert_eq!(app.ui.open_panel, None, "the Image Trace panel is left as it was");

    click_first(&mut app);
    let expanded = first(&app);
    assert_ne!(expanded.name.as_deref(), Some("Image Trace"), "the trace was expanded");
    let mut images = 0;
    expanded.walk(&mut |n| images += usize::from(matches!(n.kind, NodeKind::Image(_))));
    assert_eq!(images, 0, "only the traced paths are kept");
}

/// The labels the Contextual Task Bar draws in a headless frame (after one to lay it out), with
/// their rects; none while it isn't shown. The SVG loaders are installed so the icons draw as
/// images instead of egui's ⚠ placeholder, which the label list would otherwise pick up.
fn task_bar_labels(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<(String, egui::Rect)> {
    egui_extras::install_image_loaders(ctx);
    frame(app, ctx, vec![]);
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| show(app, ui));
    out.textures_delta.clear();
    let Some(bar) = task_bar_rect(ctx) else { return vec![] };
    out.shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) if bar.contains(t.visual_bounding_rect().center()) => Some((t.galley.text().to_string(), t.visual_bounding_rect())),
            _ => None,
        })
        .collect()
}

/// With the Artboard tool, the Contextual Task Bar sits under the selected artboards and offers
/// Resize, Duplicate, Rearrange All (with two or more artboards) and Export instead of the
/// selected objects' actions. With the Selection tool it offers the objects' actions again, and
/// with nothing selected it doesn't show.
#[test]
fn the_task_bar_offers_the_artboard_tools_actions() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    app.select_tool("selection");
    let ctx = egui::Context::default();
    let names = |app: &mut VectorcraftApp| task_bar_labels(app, &ctx).into_iter().map(|(t, _)| t).collect::<Vec<_>>();
    assert!(names(&mut app).is_empty() && task_bar_rect(&ctx).is_none(), "nothing selected: no bar");

    app.select_tool("artboard");
    crate::panels::artboards::select(&mut app, 1);
    // Artboard 2 in the middle of the window, at 100%.
    let b = app.session.active().unwrap().doc.artboards[1].rect;
    let view = app.view_mut().unwrap();
    (view.center, view.zoom) = (b.center(), 1.0);
    assert_eq!(names(&mut app), ["Resize", "Duplicate", "Rearrange All", "Export"]);
    // Under artboard 2.
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let bar = task_bar_rect(&ctx).unwrap();
    let center = xf.to_screen(b.center());
    assert!(bar.top() > xf.to_screen(Point::new(b.x0, b.y1)).y && bar.x_range().contains(center.x), "{bar:?} under {b:?}");

    // A selected object with the Selection tool: its own actions, none of the artboards'.
    app.select_tool("selection");
    app.session.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap();
    assert_eq!(names(&mut app), ["Edit Path", "Offset Path", "Simplify", "Duplicate"]);

    // One artboard: nothing to rearrange.
    app.session.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    app.select_tool("artboard");
    assert_eq!(names(&mut app), ["Resize", "Duplicate", "Export"]);
}

/// A selected group's Contextual Task Bar offers Ungroup and Isolate. While an opacity mask is
/// edited, when Enter Isolation Mode is off, a group in the mask art gets Ungroup without Isolate.
#[test]
fn the_task_bar_leaves_isolate_out_while_an_opacity_mask_is_edited() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.select_tool("selection");
    let ctx = egui::Context::default();
    let names = |app: &mut VectorcraftApp| task_bar_labels(app, &ctx).into_iter().map(|(t, _)| t).collect::<Vec<_>>();
    let rect = |app: &mut VectorcraftApp, x: f64| {
        app.session.execute("shape.rectangle", &json!({"x": x, "y": 100, "width": 40, "height": 40})).unwrap()["id"].clone()
    };
    let group = |app: &mut VectorcraftApp, x: f64| {
        let (a, b) = (rect(app, x), rect(app, x + 50.0));
        app.session.execute("select.set", &json!({"ids": [a, b]})).unwrap();
        app.session.execute("object.group", &json!({})).unwrap();
    };
    group(&mut app, 20.0);
    assert_eq!(names(&mut app), ["Ungroup", "Isolate", "Duplicate"]);
    let (obj, m) = (rect(&mut app, 200.0), rect(&mut app, 200.0));
    app.session.execute("transparency.makeOpacityMask", &json!({"ids": [obj, m]})).unwrap();
    app.session.execute("transparency.editOpacityMask", &json!({"id": obj})).unwrap();
    group(&mut app, 200.0);
    assert_eq!(names(&mut app), ["Ungroup", "Duplicate"]);
}

/// The Artboard tool's Contextual Task Bar buttons act on its selected artboards: Resize opens
/// Artboard Options for the active one, Rearrange All opens Rearrange All Artboards, Export opens
/// Export for Screens with the selected artboards checked, and Duplicate copies each selected
/// artboard in one undo step and selects the copies.
#[test]
fn the_artboard_tools_task_bar_buttons_act_on_the_selected_artboards() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    app.select_tool("artboard");
    crate::panels::artboards::select(&mut app, 2);
    app.session.set_tool_option("selected", &json!([2, 0]));
    let ctx = egui::Context::default();
    let press = |app: &mut VectorcraftApp, label: &str| {
        let labels = task_bar_labels(app, &ctx);
        let at = labels.iter().find(|(t, _)| t == label).unwrap_or_else(|| panic!("{label} in {labels:?}")).1.center();
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(app, &ctx, vec![egui::Event::PointerMoved(at)]);
        frame(app, &ctx, vec![button(true)]);
        frame(app, &ctx, vec![button(false)]);
        app.ui.dialog.take()
    };
    let d = press(&mut app, "Resize").expect("Artboard Options");
    assert_eq!((d.kind.as_str(), d.f64("index", 9.0)), ("artboardOptions", 2.0));
    let d = press(&mut app, "Rearrange All").expect("Rearrange All Artboards");
    assert_eq!(d.kind, crate::dialogs::rearrange_artboards::KIND);
    let d = press(&mut app, "Export").expect("Export for Screens");
    assert_eq!((d.kind.as_str(), &d.fields["boards"]), (crate::dialogs::EXPORT_FOR_SCREENS, &json!([true, false, true])));

    let boards =
        |app: &VectorcraftApp| app.session.active().unwrap().doc.artboards.iter().map(|a| (a.name.clone(), a.rect.size())).collect::<Vec<_>>();
    let before = boards(&app);
    assert!(press(&mut app, "Duplicate").is_none());
    let after = boards(&app);
    assert_eq!(after[..3], before[..]);
    assert_eq!(after[3..], [("Artboard 1 copy".to_string(), before[0].1), ("Artboard 3 copy".to_string(), before[2].1)]);
    assert_eq!(app.session.tool_options()["selected"], json!([4, 3]), "the copies are selected, artboard 3's copy active");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(boards(&app), before, "one undo step");
}

/// With anchors direct-selected, the Contextual Task Bar offers Remove Anchor Points, Connect,
/// Cut Path, Corner and Smooth. Smooth and Corner convert the selected anchor, Cut Path opens
/// the path there and Connect closes it again, as the Control bar's anchor buttons do.
#[test]
fn the_task_bar_connects_cuts_and_converts_direct_selected_anchors() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 150, "y": 100, "width": 100, "height": 80})).unwrap()["id"].as_u64().unwrap();
    app.select_tool("directSelection");
    app.run("select.anchors", json!({"id": id, "anchors": [[0, 1]]})).unwrap();
    let ctx = egui::Context::default();
    let names = |app: &mut VectorcraftApp| task_bar_labels(app, &ctx).into_iter().map(|(t, _)| t).collect::<Vec<_>>();
    assert_eq!(names(&mut app), ["Remove Anchor Points", "Connect", "Cut Path", "Corner", "Smooth", "Duplicate"]);
    let press = |app: &mut VectorcraftApp, label: &str| {
        let labels = task_bar_labels(app, &ctx);
        let at = labels.iter().find(|(t, _)| t == label).unwrap_or_else(|| panic!("{label} in {labels:?}")).1.center();
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(app, &ctx, vec![egui::Event::PointerMoved(at)]);
        frame(app, &ctx, vec![button(true)]);
        frame(app, &ctx, vec![button(false)]);
    };
    let subpath =
        |app: &VectorcraftApp| app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().path_data().unwrap().subpaths[0].clone();
    let last_undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.last().unwrap().label.clone();
    press(&mut app, "Smooth");
    let a = subpath(&app).anchors[1];
    assert!(a.has_in() && a.has_out(), "smooth: {a:?}");
    assert_eq!(last_undo(&app), "Convert Anchor Points");
    press(&mut app, "Corner");
    let a = subpath(&app).anchors[1];
    assert!(!a.has_in() && !a.has_out(), "corner: {a:?}");
    press(&mut app, "Cut Path");
    assert_eq!(last_undo(&app), "Cut Path");
    let sp = subpath(&app);
    assert!(!sp.closed && sp.anchors.len() == 5, "the rectangle opens at the anchor: {sp:?}");
    press(&mut app, "Connect");
    assert_eq!(last_undo(&app), "Join");
    let sp = subpath(&app);
    assert!(sp.closed && sp.anchors.len() == 4, "the ends join again: {sp:?}");
}

/// The Contextual Task Bar's handle drags it: unpinned it keeps that offset as it follows the
/// selection, pinned it stays put when the selection changes, unpinned again it follows from
/// there, and Reset Bar Position puts it back under the selection. It never leaves the canvas,
/// also when the window shrinks.
#[test]
fn the_task_bar_is_dragged_by_its_handle_pinned_and_reset() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let mut square =
        |x: f64, y: f64| app.session.execute("shape.rectangle", &json!({"x": x, "y": y, "width": 40, "height": 40})).unwrap()["id"].clone();
    let (a, b) = (square(140.0, 40.0), square(160.0, 150.0));
    app.select_tool("selection");
    let ctx = egui::Context::default();
    let bar = || task_bar_rect(&ctx).expect("the task bar shows");
    let select = |app: &mut VectorcraftApp, id: &serde_json::Value| {
        app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
        frame(app, &ctx, vec![]);
        frame(app, &ctx, vec![]);
        bar().min
    };
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let drag = |app: &mut VectorcraftApp, by: Vec2| {
        let grip = pos2(bar().left() + 11.0, bar().center().y);
        frame(app, &ctx, vec![egui::Event::PointerMoved(grip)]);
        frame(app, &ctx, vec![button(grip, true)]);
        frame(app, &ctx, vec![egui::Event::PointerMoved(grip + by)]);
        frame(app, &ctx, vec![button(grip + by, false)]);
        frame(app, &ctx, vec![]);
        bar().min
    };
    let near = |p: Pos2, q: Pos2| assert!((p - q).length() < 1.5, "{p:?} is not at {q:?}");
    let (under_a, under_b) = (select(&mut app, &a), select(&mut app, &b));
    assert!(under_b.y > under_a.y + 100.0, "each bar sits under its selection");

    // Dragged unpinned: it keeps its offset from the selection.
    select(&mut app, &a);
    let by = vec2(40.0, -30.0);
    near(drag(&mut app, by), under_a + by);
    near(select(&mut app, &b), under_b + by);
    assert!(!app.ui.task_bar_place.pinned);

    // Pinned: it stays where it is across selection changes, and the handle still moves it.
    assert_eq!(app.run("window.taskBar.pin", json!({})).unwrap(), json!(true));
    assert_eq!(crate::menus::checked(&app, "window.taskBar.pin", &json!({})), Some(true));
    near(select(&mut app, &a), under_b + by);
    let spot = drag(&mut app, vec2(-20.0, 10.0));
    near(spot, under_b + by + vec2(-20.0, 10.0));
    near(select(&mut app, &b), spot);

    // Unpinned: it follows the selection from where it is, kept on the canvas.
    assert_eq!(app.run("window.taskBar.pin", json!({"pinned": false})).unwrap(), json!(false));
    near(select(&mut app, &a), spot);
    let canvas = app.canvas_rect.unwrap();
    let last = canvas.max - vec2(8.0, 8.0) - bar().size();
    near(select(&mut app, &b), (under_b + (spot - under_a)).min(last));
    assert!(app.run("window.taskBar.pin", json!({"pinned": "yes"})).is_err());

    // Kept on the canvas: dragged far off, then pinned in the corner of a shrinking window.
    near(drag(&mut app, vec2(-2000.0, -2000.0)), canvas.min + vec2(8.0, 8.0));
    app.run("window.taskBar.pin", json!({"pinned": true})).unwrap();
    drag(&mut app, vec2(2000.0, 2000.0));
    near(bar().max, canvas.max - vec2(8.0, 8.0));
    let small = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(640.0, 480.0))), ..Default::default() };
    for _ in 0..2 {
        let mut out = ctx.run_ui(small.clone(), |ui| show(&mut app, ui));
        out.textures_delta.clear();
    }
    let canvas = app.canvas_rect.unwrap();
    assert!(canvas.width() < 700.0, "the canvas shrank: {canvas:?}");
    near(bar().max, canvas.max - vec2(8.0, 8.0));

    // Reset: back under the selection, unpinned.
    frame(&mut app, &ctx, vec![]);
    app.run("window.taskBar.reset", json!({})).unwrap();
    assert!(!app.ui.task_bar_place.pinned);
    near(select(&mut app, &a), under_a);
    near(select(&mut app, &b), under_b);
}

/// In a view turned 180° the Contextual Task Bar shows under the selection's lowest corner on
/// screen, not over the selection.
#[test]
fn the_task_bar_stays_under_the_selection_in_a_rotated_view() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 170, "y": 90, "width": 60, "height": 120})).unwrap()["id"].clone();
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    app.select_tool("selection");
    app.run("view.rotate", json!({"angle": 180})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    let bar = task_bar_rect(&ctx).expect("the task bar shows");
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let lowest = xf.quad(app.selection_bounds().unwrap()).iter().map(|c| c.y).fold(f32::NEG_INFINITY, f32::max);
    assert!(bar.top() > lowest, "the bar {bar:?} starts below {lowest}");
}

fn ruler_guide_origin_ui(alt: bool, cancel: bool) {
    use egui::{Event, Modifiers};
    for (origin, preview, release) in [
        (Point::new(17.0, 23.0), Point::new(222.0, 238.0), Point::new(232.0, 243.0)),
        (Point::new(-17.0, -23.0), Point::new(223.0, 237.0), Point::new(233.0, 242.0)),
        (Point::ZERO, Point::new(225.0, 235.0), Point::new(230.0, 245.0)),
    ] {
        for source_vertical in [false, true] {
            let mut app = VectorcraftApp::new(Session::new(), Default::default());
            app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
            app.run("document.setRulerOrigin", json!({"x": origin.x, "y": origin.y})).unwrap();
            app.run("shape.rectangle", json!({"x": 40, "y": 50, "width": 20, "height": 30})).unwrap();
            let redo = app.session.active().unwrap().doc.clone();
            app.run("edit.undo", json!({})).unwrap();
            app.session.active_mut().unwrap().mark_saved();
            app.ui.view.rulers = true;
            app.ui.view.smart_guides = false;
            app.ui.view.snap_to_point = false;
            app.ui.view.snap_to_grid = false;
            app.ui.view.snap_to_pixel = false;
            app.select_tool("rectangle");
            let ctx = egui::Context::default();
            frame(&mut app, &ctx, vec![]);
            let view = app.view_mut().unwrap();
            view.zoom = 1.0;
            view.rotation = 0.0;
            view.center = Point::new(200.0, 150.0);
            frame(&mut app, &ctx, vec![]);
            let rect = app.canvas_rect.unwrap();
            let xf = Xf::new(rect, app.view().unwrap());
            let from =
                if source_vertical { pos2(rect.left() - RULER / 2.0, rect.center().y) } else { pos2(rect.center().x, rect.top() - RULER / 2.0) };
            let to = xf.to_screen(Point::new(223.0, 237.0));
            let end = xf.to_screen(Point::new(231.0, 244.0));
            assert!(rect.contains(to) && rect.contains(end));
            let mods = if alt { Modifiers::SHIFT | Modifiers::ALT } else { Modifiers::SHIFT };
            let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: mods };
            let before = app.session.active().unwrap().doc.clone();
            let selection = app.session.active().unwrap().selection.clone();
            let history = |app: &VectorcraftApp| {
                let st = app.session.active().unwrap();
                (st.history.undo.len(), st.history.redo.len(), st.is_dirty())
            };
            let original_history = history(&app);
            assert_eq!(original_history.1, 1);
            frame(&mut app, &ctx, vec![Event::ModifiersChanged(mods), Event::PointerMoved(from)]);
            frame(&mut app, &ctx, vec![button(from, true)]);
            frame(&mut app, &ctx, vec![Event::PointerMoved(to)]);
            let check = |app: &VectorcraftApp, switched: bool, expected: Point| {
                let guides = &app.session.active().unwrap().doc.guides;
                assert_eq!(guides.len(), 1);
                let vertical = source_vertical != switched;
                assert_eq!(guides[0].vertical, vertical);
                let want = if vertical { expected.x } else { expected.y };
                assert!((guides[0].pos - want).abs() < 1e-4, "origin={origin:?} vertical={vertical}: {:?}, want {want}", guides[0]);
            };
            assert_eq!(history(&app).0, original_history.0, "preview has no undo entry");
            if cancel {
                // Cancellation is a baseline positive control, independent of snap correctness.
                assert_eq!(app.session.active().unwrap().doc.guides.len(), 1);
                frame(&mut app, &ctx, vec![Event::PointerMoved(from), button(from, false)]);
                assert_eq!(app.session.active().unwrap().doc, before);
                assert_eq!(app.session.active().unwrap().selection, selection);
                assert_eq!(history(&app), original_history);
                app.run("edit.redo", json!({})).unwrap();
                assert_eq!(app.session.active().unwrap().doc, redo);
                continue;
            }
            check(&app, alt, preview);
            if alt {
                // Used only by the private #1782 compatibility regression, not standalone RED.
                frame(&mut app, &ctx, vec![Event::ModifiersChanged(Modifiers::SHIFT)]);
                check(&app, false, preview);
                frame(&mut app, &ctx, vec![Event::ModifiersChanged(mods)]);
                check(&app, true, preview);
            }
            frame(&mut app, &ctx, vec![Event::PointerMoved(end), button(end, false)]);
            check(&app, alt, release);
            let st = app.session.active().unwrap();
            assert_eq!(st.doc.ruler_origin_for(0), origin);
            assert_eq!(st.doc.layers, before.layers, "the rectangle tool draws no artwork");
            assert_eq!(st.selection, selection);
            assert_eq!(history(&app), (original_history.0 + 1, 0, true));
            assert_eq!(st.history.undo.last().unwrap().label, "New Guide");
            let after = st.doc.clone();
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(app.session.active().unwrap().doc, before);
            assert_eq!(history(&app), (original_history.0, 1, false));
            app.run("edit.redo", json!({})).unwrap();
            assert_eq!(app.session.active().unwrap().doc, after);
        }
    }
}

#[test]
fn ruler_guide_shift_origin_preview_release_and_history() {
    ruler_guide_origin_ui(false, false);
}

#[test]
fn ruler_guide_shift_origin_cancel_preserves_redo() {
    ruler_guide_origin_ui(false, true);
}

#[test]
fn ruler_guide_origin_display_fallback_accepts_only_bounded_finite_pairs() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({})).unwrap();
    // The global zero point as stored (artboard rulers check their own, see `ruler_origin_for`).
    app.run("view.globalRulers", json!({"global": true})).unwrap();
    let max = vectorcraft_engine::MAX_COORD;
    for (origin, expected) in [
        (Point::ZERO, Point::ZERO),
        (Point::new(17.0, -23.0), Point::new(17.0, -23.0)),
        (Point::new(max, -max), Point::new(max, -max)),
        (Point::new(f64::NAN, 23.0), Point::ZERO),
        (Point::new(17.0, f64::INFINITY), Point::ZERO),
        (Point::new(max + 1.0, 23.0), Point::ZERO),
        (Point::new(17.0, -max - 1.0), Point::ZERO),
    ] {
        std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).ruler_origin = origin;
        assert_eq!(app.ruler_origin(), expected);
    }
}

#[test]
fn ruler_guide_origin_tooltip_matches_selected_guide_fields() {
    use egui::Event;
    for (vertical, target, shown) in [(true, 222.0, 205.0), (false, 238.0, 215.0)] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("document.setRulerOrigin", json!({"x": 17, "y": 23})).unwrap();
        app.run("guide.add", json!({"vertical": vertical, "pos": 100})).unwrap();
        app.ui.view.guides = true;
        app.ui.view.smart_guides = false;
        app.ui.view.snap_to_point = false;
        app.ui.view.snap_to_grid = false;
        app.ui.view.snap_to_pixel = false;
        app.select_tool("selection");
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let view = app.view_mut().unwrap();
        view.zoom = 1.0;
        view.rotation = 0.0;
        view.center = Point::new(200.0, 150.0);
        frame(&mut app, &ctx, vec![]);
        app.session.active_mut().unwrap().mark_saved();
        let before = app.session.active().unwrap().doc.clone();
        let undo = app.session.active().unwrap().history.undo.len();
        let rect = app.canvas_rect.unwrap();
        let xf = Xf::new(rect, app.view().unwrap());
        let from = xf.to_screen(if vertical { Point::new(100.0, 50.0) } else { Point::new(50.0, 100.0) });
        let to = xf.to_screen(if vertical { Point::new(target, 50.0) } else { Point::new(50.0, target) });
        assert!(rect.contains(from) && rect.contains(to));
        let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, &ctx, vec![Event::PointerMoved(from)]);
        frame(&mut app, &ctx, vec![button(from, true)]);
        frame(&mut app, &ctx, vec![Event::PointerMoved(to)]);
        assert!((app.session.active().unwrap().doc.guides[0].pos - target).abs() < 1e-4);
        assert_eq!(app.session.active().unwrap().selection.guides, vec![0]);
        // Observe the real selected-guide field; no expected strings are inserted into UI state.
        let fields = egui::Context::default();
        crate::theme::install_fonts(&fields);
        let mut texts = vec![];
        for _ in 0..3 {
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0))), ..Default::default() };
            let mut out = fields.run_ui(input, |ui| crate::panels::transform::show(&mut app, ui));
            out.textures_delta.clear();
            fn collect(shape: &egui::Shape, texts: &mut Vec<String>) {
                match shape {
                    egui::Shape::Text(t) => texts.push(t.galley.text().to_string()),
                    egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, texts)),
                    _ => {}
                }
            }
            texts.clear();
            out.shapes.iter().for_each(|s| collect(&s.shape, &mut texts));
        }
        assert!(texts.iter().any(|t| t == &Unit::Points.format(shown)), "{texts:?}");
        let view = app.view_info();
        let overlays = app.session.overlays(view);
        let want = format!("{}: {shown:.2} pt", if vertical { "X" } else { "Y" });
        assert!(
            overlays.iter().any(|o| matches!(o, vectorcraft_tools::Overlay::Measure { text, .. } if text == &want)),
            "{overlays:?}, field={shown}"
        );
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
        frame(&mut app, &ctx, vec![button(to, false)]);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
        assert!((app.session.active().unwrap().doc.guides[0].pos - target).abs() < 1e-4, "stored coordinate remains absolute");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc, before);
    }
}

/// Distance Guides on a canvas at 100%: a 400 × 300 pt document with a 50 pt square at (50, 50),
/// selected, and with `other` another 100 pt to its right, the Selection tool and Smart Guides
/// on → the app, its context, the view, the screen point over the middle of where the other
/// square is (or would be) and the selected square's id.
fn distance_scene(other: bool) -> (VectorcraftApp, egui::Context, Xf, Pos2, serde_json::Value) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let a = app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 50, "height": 50})).unwrap()["id"].clone();
    if other {
        app.run("shape.rectangle", json!({"x": 200, "y": 50, "width": 50, "height": 50})).unwrap();
    }
    app.run("select.set", json!({"ids": [a]})).unwrap();
    app.ui.view.smart_guides = true;
    app.select_tool("selection");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let view = app.view_mut().unwrap();
    view.zoom = 1.0;
    view.rotation = 0.0;
    view.center = Point::new(200.0, 150.0);
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let over = xf.to_screen(Point::new(225.0, 75.0));
    (app, ctx, xf, over, a)
}

/// One frame with `events` → the shapes drawn, flattened. The modifiers held stay from one
/// frame to the next until an `Event::ModifiersChanged`.
fn distance_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<Shape> {
    distance_frame_covered(app, ctx, events, None)
}

/// What opens over the canvas in [`distance_frame_covered`].
#[derive(Clone, Copy)]
enum Cover {
    /// A floating window (a floating panel, a menu, a popup) over this rectangle.
    Panel(egui::Rect),
    /// A modal dialog, with the backdrop that keeps the rest of the app from the pointer.
    Dialog,
}

/// [`distance_frame`] with `cover` drawn over the canvas.
fn distance_frame_covered(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>, cover: Option<Cover>) -> Vec<Shape> {
    let raw = egui::RawInput { screen_rect: Some(WINDOW), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| {
        show(app, ui);
        match cover {
            Some(Cover::Panel(r)) => {
                egui::Area::new(egui::Id::unique("test-floating-panel")).order(egui::Order::Foreground).fixed_pos(r.min).show(ui.ctx(), |ui| {
                    ui.allocate_space(r.size());
                });
            }
            Some(Cover::Dialog) => {
                crate::dialogs::modal::show(ui.ctx(), "Test", egui::Id::unique("test-dialog"), 0.0, 8, |ui| ui.label("A dialog"));
            }
            None => {}
        }
    });
    out.textures_delta.clear();
    fn flat(s: Shape, out: &mut Vec<Shape>) {
        match s {
            Shape::Vec(v) => v.into_iter().for_each(|s| flat(s, out)),
            s => out.push(s),
        }
    }
    let mut v = vec![];
    out.shapes.into_iter().for_each(|c| flat(c.shape, &mut v));
    v
}

/// The texts drawn in `s`.
fn texts(s: &[Shape]) -> Vec<String> {
    s.iter().filter_map(|s| if let Shape::Text(t) = s { Some(t.galley.text().to_string()) } else { None }).collect()
}

/// The label "100.00 pt" drawn in `s` and the pill of the smart guide colour it sits in, if any.
fn gap_label(s: &[Shape]) -> Option<(egui::Rect, Option<egui::Rect>)> {
    let magenta = c32(vectorcraft_tools::guides::MAGENTA);
    let text =
        s.iter().find_map(|s| if let Shape::Text(t) = s { (t.galley.text() == "100.00 pt").then(|| t.visual_bounding_rect()) } else { None })?;
    let pill =
        s.iter().find_map(|s| if let Shape::Rect(r) = s { (r.fill == magenta && r.rect.contains(text.center())).then_some(r.rect) } else { None });
    Some((text, pill))
}

/// Distance Guides on screen: with a square selected and Alt held over another to its right,
/// the frame draws the measured line between their facing edges with the gap's length in a
/// pill of the smart guide colour on its middle; without Alt, or with Alt let go, nothing.
#[test]
fn alt_over_another_object_draws_the_distance_guide_label() {
    use egui::{Event, Modifiers};
    let (mut app, ctx, xf, over, _) = distance_scene(true);
    let mut run = |events| distance_frame(&mut app, &ctx, events);
    run(vec![Event::PointerMoved(over)]);
    assert_eq!(gap_label(&run(vec![])), None, "without Alt");
    run(vec![Event::ModifiersChanged(Modifiers::ALT)]);
    let s = run(vec![]);
    let (text, pill) = gap_label(&s).expect("the gap's length is drawn");
    let pill = pill.expect("in a pill of the smart guide colour");
    let mid = xf.to_screen(Point::new(150.0, 75.0));
    assert!(pill.center().distance(mid) < 1.0 && pill.contains_rect(text), "{pill:?} on {mid:?}");
    let (from, to) = (xf.to_screen(Point::new(100.0, 75.0)), xf.to_screen(Point::new(200.0, 75.0)));
    let magenta = c32(vectorcraft_tools::guides::MAGENTA);
    let measured = s.iter().any(|s| matches!(s, Shape::LineSegment { points, stroke } if *points == [from, to] && stroke.color == magenta));
    assert!(measured, "the measured line between the facing edges");
    run(vec![Event::ModifiersChanged(Modifiers::NONE)]);
    assert_eq!(gap_label(&run(vec![])), None, "Alt let go");
}

/// The Distance Guides go when the pointer stops hovering for the tool with Alt still held: it
/// leaves the canvas for a panel or out of the window, or a press pans (Alt+Space) and takes it
/// over the panels; they come back when it hovers the other square again.
#[test]
fn distance_guides_go_when_the_hover_ends() {
    use egui::{Event, Modifiers};
    let (mut app, ctx, xf, over, _) = distance_scene(true);
    let outside = pos2(xf.rect.max.x + 10.0, xf.rect.center().y);
    assert!(WINDOW.contains(outside) && !xf.rect.contains(outside), "a panel beside the canvas");
    let mut run = |events| distance_frame(&mut app, &ctx, events);
    run(vec![Event::ModifiersChanged(Modifiers::ALT), Event::PointerMoved(over)]);
    assert!(gap_label(&run(vec![])).is_some(), "shown over the other square");
    run(vec![Event::PointerMoved(outside)]);
    assert_eq!(gap_label(&run(vec![])), None, "gone over a panel, Alt still held");
    run(vec![Event::PointerMoved(over)]);
    assert!(gap_label(&run(vec![])).is_some(), "back over the other square");
    run(vec![Event::PointerGone]);
    assert_eq!(gap_label(&run(vec![])), None, "gone out of the window");
    // Back over the square, Space held with Alt pans: they go at the press, and stay gone while
    // the pan takes the pointer out over a panel.
    let space = |pressed| Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers: Modifiers::ALT };
    let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::ALT };
    run(vec![Event::PointerMoved(over)]);
    run(vec![space(true)]);
    assert!(gap_label(&run(vec![])).is_some(), "Space held: still shown");
    run(vec![button(over, true)]);
    assert_eq!(gap_label(&run(vec![])), None, "gone at the pan's press");
    run(vec![Event::PointerMoved(over + vec2(20.0, 0.0))]);
    run(vec![Event::PointerMoved(outside)]);
    assert_eq!(gap_label(&run(vec![])), None, "still gone after panning out over a panel");
    run(vec![button(outside, false), space(false)]);
}

/// With Alt held and the pointer still, the Distance Guides follow what lies under it: they go
/// under a floating panel or a modal dialog and come back when it closes; scrolled so that
/// empty artboard lies under the pointer, they measure to the artboard; when an edit puts an
/// object under the pointer, they measure to it; and the Selection tool switched to shows them
/// at once.
#[test]
fn distance_guides_follow_what_lies_under_a_still_pointer() {
    use egui::{Event, Modifiers};
    let (mut app, ctx, _, over, _) = distance_scene(true);
    let run = |app: &mut VectorcraftApp, events, cover| distance_frame_covered(app, &ctx, events, cover);
    run(&mut app, vec![Event::ModifiersChanged(Modifiers::ALT), Event::PointerMoved(over)], None);
    assert!(gap_label(&run(&mut app, vec![], None)).is_some(), "shown over the other square");
    // A floating panel, then a modal dialog, opens over the still pointer.
    for cover in [Cover::Panel(egui::Rect::from_center_size(over, vec2(120.0, 120.0))), Cover::Dialog] {
        run(&mut app, vec![], Some(cover));
        run(&mut app, vec![], Some(cover));
        assert_eq!(gap_label(&run(&mut app, vec![], Some(cover))), None, "gone under it");
        // A frame for egui to drop the closed window, one for the hover to come back.
        run(&mut app, vec![], None);
        run(&mut app, vec![], None);
        assert!(gap_label(&run(&mut app, vec![], None)).is_some(), "back once it closes");
    }
    // Another tool and back to the Selection tool (its shortcut, the toolbar): a new Selection
    // tool, hovered at once.
    app.select_tool("rectangle");
    run(&mut app, vec![], None);
    assert_eq!(gap_label(&run(&mut app, vec![], None)), None, "the Rectangle tool shows none");
    app.select_tool("selection");
    run(&mut app, vec![], None);
    assert!(gap_label(&run(&mut app, vec![], None)).is_some(), "the Selection tool again");
    // The view scrolls 150 pt (the wheel, a gesture, the keys): empty artboard is under the
    // pointer, and the selected square is 300 pt from the artboard's right edge.
    app.view_mut().unwrap().center.x += 150.0;
    run(&mut app, vec![], None);
    let s = run(&mut app, vec![], None);
    assert_eq!(gap_label(&s), None, "no longer over the other square");
    assert!(texts(&s).iter().any(|t| t == "300.00 pt"), "to the artboard: {:?}", texts(&s));

    // A square made under the still pointer, the first selected again (an edit, an undo).
    let (mut app, ctx, _, over, a) = distance_scene(false);
    let run = |app: &mut VectorcraftApp, events| distance_frame(app, &ctx, events);
    run(&mut app, vec![Event::ModifiersChanged(Modifiers::ALT), Event::PointerMoved(over)]);
    let s = run(&mut app, vec![]);
    assert!(gap_label(&s).is_none() && texts(&s).iter().any(|t| t == "300.00 pt"), "to the artboard: {:?}", texts(&s));
    app.run("shape.rectangle", json!({"x": 200, "y": 50, "width": 50, "height": 50})).unwrap();
    app.run("select.set", json!({"ids": [a]})).unwrap();
    run(&mut app, vec![]);
    assert!(gap_label(&run(&mut app, vec![])).is_some(), "to the new square");
    // Another document (a new tab) under the still pointer: nothing selected there, none.
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    run(&mut app, vec![]);
    assert_eq!(gap_label(&run(&mut app, vec![])), None, "another document");
}

/// #4518: a floating panel or a modal dialog over the canvas hides the pointer from it, so Object
/// Highlighting, the rulers and the Info panel don't follow a pointer that is over the dialog.
#[test]
fn a_panel_or_dialog_over_the_canvas_hides_the_pointer_from_it() {
    let (mut app, ctx, _, over, _) = distance_scene(true);
    let run = |app: &mut VectorcraftApp, events, cover| distance_frame_covered(app, &ctx, events, cover);
    run(&mut app, vec![egui::Event::PointerMoved(over)], None);
    run(&mut app, vec![], None);
    assert!(app.hover_doc.is_some(), "over the canvas");
    for cover in [Cover::Panel(egui::Rect::from_center_size(over, vec2(120.0, 120.0))), Cover::Dialog] {
        // A new window takes a sizing pass before it covers anything, as in the test above.
        for _ in 0..3 {
            run(&mut app, vec![], Some(cover));
        }
        assert_eq!(app.hover_doc, None, "under it");
        for _ in 0..3 {
            run(&mut app, vec![], None);
        }
        assert!(app.hover_doc.is_some(), "back once it closes");
    }
}

/// The 800 × 600 window [`frame`] draws the canvas in.
const WINDOW: egui::Rect = egui::Rect { min: Pos2::ZERO, max: Pos2 { x: 800.0, y: 600.0 } };

/// A 400 × 300 pt document at 400%, turned `rotation` degrees, with the Rectangle tool: a press
/// that reached the tool would draw.
fn zoomed_in(rotation: f64) -> (VectorcraftApp, egui::Context) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.select_tool("rectangle");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let v = app.view_mut().unwrap();
    v.zoom = 4.0;
    v.rotation = rotation;
    frame(&mut app, &ctx, vec![]);
    (app, ctx)
}

/// Where the document's point `p` is on screen.
fn on_screen(app: &VectorcraftApp, p: Point) -> Pos2 {
    Xf::new(app.canvas_rect.unwrap(), app.view().unwrap()).to_screen(p)
}

/// Scroll bar `k` (0 horizontal, 1 vertical): its track, its span and the thumb's centre.
fn scroll_bar(app: &mut VectorcraftApp, k: usize) -> (egui::Rect, Span, Pos2) {
    let rect = app.canvas_rect.unwrap();
    let track = scroll_tracks(rect, WINDOW)[k];
    let span = scroll_spans(&Xf::new(rect, app.view().unwrap()), app.content_bounds())[k];
    let len = if k == 0 { track.width() } else { track.height() };
    let (at, size) = span.thumb(len);
    let c = if k == 0 { pos2(track.left() + at + size / 2.0, track.center().y) } else { pos2(track.center().x, track.top() + at + size / 2.0) };
    (track, span, c)
}

fn primary(pos: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
}

/// #2322: the Normal and Full Screen with Menu Bar modes keep the scroll bars along the right
/// and bottom edges, beside the rulers; Full Screen and Presentation Mode give the canvas the
/// whole window.
#[test]
fn scroll_bars_take_the_right_and_bottom_edges_in_the_windowed_screen_modes() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.ui.view.rulers = true;
    let ctx = egui::Context::default();
    for (mode, max) in
        [(0, WINDOW.max - vec2(SCROLL_BAR, SCROLL_BAR)), (1, WINDOW.max - vec2(SCROLL_BAR, SCROLL_BAR)), (2, WINDOW.max), (3, WINDOW.max)]
    {
        app.ui.screen_mode = mode;
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.canvas_rect.unwrap().max, max, "screen mode {mode}");
    }
}

/// #2322: the thumb is as long, against its track, as the view against the art and the
/// artboards with half a window around them; zoomed in it is shorter.
#[test]
fn a_scroll_bar_thumb_is_as_long_as_the_view_is_against_the_content() {
    assert_eq!(Span { lo: 0.0, hi: 100.0, v0: 0.0, v1: 100.0 }.thumb(50.0), (0.0, 50.0), "all in view: the whole track");
    assert_eq!(Span { lo: 0.0, hi: 400.0, v0: 150.0, v1: 250.0 }.thumb(100.0), (37.5, 25.0));
    assert_eq!(Span { lo: 0.0, hi: 1000.0, v0: 450.0, v1: 550.0 }.thumb(100.0), (40.0, 20.0), "never shorter than MIN_THUMB");
    let (mut app, ctx) = zoomed_in(0.0);
    let len = |app: &mut VectorcraftApp| {
        let (track, span, _) = scroll_bar(app, 0);
        span.thumb(track.width()).1
    };
    let at_400 = len(&mut app);
    app.view_mut().unwrap().zoom = 8.0;
    frame(&mut app, &ctx, vec![]);
    assert!(len(&mut app) < at_400, "zoomed in, the thumb is shorter");
}

/// #2322: dragging a thumb scrolls the view along the screen's axis, in a turned view too, by
/// as much as the thumb's travel stands for; the drag never reaches the tool under the bar.
#[test]
fn dragging_a_scroll_bar_thumb_scrolls_the_view_and_draws_nothing() {
    for rotation in [0.0, 90.0, 30.0] {
        for k in 0..2 {
            let (mut app, ctx) = zoomed_in(rotation);
            let p = Point::new(200.0, 150.0);
            let before = on_screen(&app, p);
            let (track, span, from) = scroll_bar(&mut app, k);
            let len = if k == 0 { track.width() } else { track.height() };
            let step = if k == 0 { vec2(40.0, 0.0) } else { vec2(0.0, 40.0) };
            let moved = (40.0 * span.ratio(len)).clamp(span.lo - span.v0, span.hi - span.v1);
            assert!(moved > 40.0, "at 400% the view moves further than the thumb: {moved}");
            frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from)]);
            frame(&mut app, &ctx, vec![primary(from, true)]);
            frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from + step / 2.0)]);
            frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from + step)]);
            frame(&mut app, &ctx, vec![primary(from + step, false)]);
            frame(&mut app, &ctx, vec![]);
            assert_eq!(app.session.active().unwrap().doc.art_bounds(), None, "the Rectangle tool under the bar drew nothing");
            let after = on_screen(&app, p);
            let want = before - step.normalized() * moved;
            assert!((after - want).length() < 0.5, "rotation {rotation}, bar {k}: {before:?} -> {after:?}, want {want:?}");
        }
    }
}

/// #2322: a click on the track beyond the thumb scrolls a page (most of the window) towards it.
#[test]
fn a_click_on_a_scroll_bar_track_scrolls_a_page() {
    let (mut app, ctx) = zoomed_in(0.0);
    let p = Point::new(200.0, 150.0);
    let before = on_screen(&app, p);
    let (track, _, _) = scroll_bar(&mut app, 0);
    let page = 0.9 * app.canvas_rect.unwrap().width();
    let at = pos2(track.right() - 3.0, track.center().y);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
    frame(&mut app, &ctx, vec![primary(at, true)]);
    frame(&mut app, &ctx, vec![primary(at, false)]);
    frame(&mut app, &ctx, vec![]);
    let after = on_screen(&app, p);
    assert!((after.x - (before.x - page)).abs() < 0.5 && (after.y - before.y).abs() < 0.5, "{before:?} -> {after:?}, page {page}");
    assert_eq!(app.session.active().unwrap().doc.art_bounds(), None, "the Rectangle tool drew nothing");
}

/// #2322: the wheel over the horizontal bar scrolls sideways, over the vertical one up and
/// down (over the canvas it scrolls up and down).
#[test]
fn the_wheel_over_a_scroll_bar_scrolls_along_it() {
    for k in 0..2 {
        let (mut app, ctx) = zoomed_in(0.0);
        let p = Point::new(200.0, 150.0);
        let before = on_screen(&app, p);
        let (track, _, _) = scroll_bar(&mut app, k);
        let at = track.center();
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -30.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        };
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
        frame(&mut app, &ctx, vec![wheel]);
        for _ in 0..30 {
            frame(&mut app, &ctx, vec![]);
        }
        let after = on_screen(&app, p);
        let want = if k == 0 { before - vec2(30.0, 0.0) } else { before - vec2(0.0, 30.0) };
        assert!((after - want).length() < 0.5, "bar {k}: {before:?} -> {after:?}, want {want:?}");
    }
}
#[test]
fn live_trace_selection_highlight_excludes_the_retained_source_rectangle() {
    use std::sync::Arc;
    use vectorcraft_doc::{Appearance, ImageObject, NodeId, TraceView};
    let mut source = Node::new(
        NodeId(2),
        NodeKind::Image(Box::new(ImageObject {
            key: "synthetic".into(),
            width: 200,
            height: 200,
            xf: Affine::IDENTITY,
            link: None,
            placement: Default::default(),
            pixels: None,
        })),
    );
    source.visible = false;
    let art = Node::path(NodeId(3), vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(20.0, 30.0, 80.0, 90.0)), Appearance::default());
    let mut group = Node::group(NodeId(1), vec![Arc::new(source), Arc::new(art)]);
    let doc = vectorcraft_doc::Document::new(300.0, 300.0);
    let symbols = doc.symbol_boxes();
    for view in TraceView::ALL {
        group.extras_mut().trace = Some(Box::new(json!({"view":view.id()})));
        let mut visited = vec![];
        walk_drawn(&group, &mut |n| visited.push(n.id));
        assert_eq!(visited.contains(&NodeId(2)), view.shows_source());
        let want = if view.shows_source() {
            vectorcraft_geom::Rect::new(0.0, 0.0, 200.0, 200.0)
        } else {
            vectorcraft_geom::Rect::new(20.0, 30.0, 80.0, 90.0)
        };
        assert_eq!(node_outline(&group, &symbols).bounding_box(), want);
    }
    group.extras_mut().trace = None;
    assert_eq!(node_outline(&group, &symbols).bounding_box(), vectorcraft_geom::Rect::new(0.0, 0.0, 200.0, 200.0));
}

/// Hovering over point type shows only its baseline (not the transform bounding box), while
/// selecting it shows both the bounding box with handles and the baseline.
#[test]
fn point_type_hover_shows_baseline_only_and_selection_shows_both_box_and_baseline() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 600, "height": 300})).unwrap();
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    let made = app.run("text.create", json!({"x": 200, "y": 150, "text": "Lorem ipsum", "size": 36})).unwrap();
    let id = vectorcraft_doc::NodeId(made["id"].as_u64().unwrap());
    app.run("select.none", json!({})).unwrap();
    app.select_tool("selection");

    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let color = c32(app.session.active().unwrap().doc.layer_color(id));
    let layer_mode = egui::epaint::ColorMode::Solid(color);

    // Hover over the text: hover highlight shows the baseline (1.5 px stroke), NOT a bounding box.
    let over = xf.to_screen(Point::new(220.0, 140.0));
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(over)]);
    let hover_shapes = shapes(&mut app, &ctx);

    // There should be a 1.5px path outline for the hover highlight.
    let hover_outlines: Vec<_> =
        hover_shapes.iter().filter(|s| matches!(s, Shape::Path(ps) if ps.stroke.width == 1.5 && ps.stroke.color == layer_mode)).collect();
    assert!(!hover_outlines.is_empty(), "hover highlight shows baseline");

    let is_handle = |s: &Shape| matches!(s, Shape::Rect(r) if (r.rect.width() - 6.0).abs() < 1.0 && (r.rect.height() - 6.0).abs() < 1.0 && r.fill == Color32::WHITE);

    // There should be NO bounding box resize handles (size 6x6 white squares) while just hovering.
    let handles: Vec<_> = hover_shapes.iter().filter(|s| is_handle(s)).collect();
    assert!(handles.is_empty(), "hovering does not show bounding box handles: {handles:?}");

    // Now select the text (click): both the bounding box (with handles) and the baseline are shown.
    app.run("select.set", json!({"ids": [id.0]})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let sel_shapes = shapes(&mut app, &ctx);

    // Bounding box handles (8 handles around the text) should now be present.
    let handles: Vec<_> = sel_shapes.iter().filter(|s| is_handle(s)).collect();
    assert_eq!(handles.len(), 8, "selected text shows 8 bounding box handles");

    // Baseline (Shape::LineSegment at y=150) should also be present.
    let y = xf.to_screen(Point::new(0.0, 150.0)).y;
    let baselines: Vec<_> = sel_shapes
        .iter()
        .filter(|s| matches!(s, Shape::LineSegment { points, stroke } if stroke.color == color && (points[0].y - y).abs() < 0.5))
        .collect();
    assert!(!baselines.is_empty(), "selected text shows baseline alongside bounding box");
}

/// #2572: Show Text Threads links a thread's frames from the out port of one to the in port of
/// the next, where the tools draw those ports, each with the arrow of a threaded port; nothing
/// is drawn while no frame of the thread is selected.
#[test]
fn text_threads_run_from_each_out_port_to_the_next_in_port() {
    use vectorcraft_tools::text::{Port, port_centre};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    let text = "Words that fill more than the frame holds. ".repeat(10);
    let a = app.run("text.create", json!({"x": 20, "y": 20, "text": text, "area": {"width": 150, "height": 60}})).unwrap()["id"].clone();
    let b = app.run("text.thread.add", json!({"from": a, "at": [300, 200]})).unwrap()["id"].clone();
    let frame = |app: &VectorcraftApp, id: &serde_json::Value| match &app
        .session
        .active()
        .unwrap()
        .doc
        .node(vectorcraft_doc::NodeId(id.as_u64().unwrap()))
        .unwrap()
        .kind
    {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("not text"),
    };
    let zoom = 2.0;
    let (out, input) = (port_centre(&frame(&app, &a), Port::Out, 0.5).unwrap(), port_centre(&frame(&app, &b), Port::In, 0.5).unwrap());
    let o = thread_overlays(&app, zoom);
    assert!(o.iter().any(|x| matches!(x, Overlay::Line { a, b, .. } if *a == out && *b == input)), "the thread line: {o:?}");
    let boxes: Vec<Point> =
        o.iter().filter_map(|x| if let Overlay::Highlight { quad, .. } = x { Some(quad[0].midpoint(quad[2])) } else { None }).collect();
    assert_eq!(boxes.len(), 2);
    assert!(boxes[0].distance(out) < 1e-9 && boxes[1].distance(input) < 1e-9, "{boxes:?}");
    // Two arrows: three strokes each, beside the thread line.
    assert_eq!(o.iter().filter(|x| matches!(x, Overlay::Line { .. })).count(), 7);
    app.run("select.none", json!({})).unwrap();
    assert!(thread_overlays(&app, zoom).is_empty());
}

/// A double-click with the Selection tool on the out port of a threaded frame breaks the
/// thread there: no new frame is threaded on, and the story flows back into the first frame.
#[test]
fn a_double_click_on_a_threaded_out_port_breaks_the_thread() {
    use vectorcraft_tools::text::{Port, port_centre};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    let text = "Words that fill more than the frame holds. ".repeat(10);
    let a = app.run("text.create", json!({"x": 20, "y": 20, "text": text, "area": {"width": 150, "height": 60}})).unwrap()["id"].as_u64().unwrap();
    app.run("text.thread.add", json!({"from": a, "at": [300, 200]})).unwrap();
    app.run("select.set", json!({"ids": [a]})).unwrap();
    app.select_tool("selection");
    let doc = |app: &VectorcraftApp| app.session.active().unwrap().doc.clone();
    let before = doc(&app);
    let NodeKind::Text(t) = &before.node(vectorcraft_doc::NodeId(a)).unwrap().kind else { panic!("not text") };
    let view = app.view_info();
    let at = port_centre(t, Port::Out, 1.0 / view.zoom).unwrap();
    let count = doc(&app).layers[0].children().map_or(0, |c| c.len());
    for kind in [PointerKind::Down, PointerKind::Up, PointerKind::Down, PointerKind::Up, PointerKind::DoubleClick] {
        crate::canvas::dispatch(&mut app, &PointerEvent::new(kind, at.x, at.y), view);
    }
    let d = doc(&app);
    assert!(d.text_threads.is_empty(), "the thread is broken");
    assert_eq!(d.layers[0].children().map_or(0, |c| c.len()), count, "no new frame");
    let NodeKind::Text(t) = &d.node(vectorcraft_doc::NodeId(a)).unwrap().kind else { panic!("not text") };
    assert_eq!(t.plain_text(), text, "the story flows into the first frame");
    assert_eq!(app.session.tool_id(), "selection");
}

/// With more than `PORTS_MAX` type objects selected the tools draw no ports, and Show Text
/// Threads draws only its lines, no port boxes.
#[test]
fn text_threads_of_a_huge_selection_have_lines_but_no_ports() {
    use vectorcraft_tools::text::PORTS_MAX;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    let text = "Words that fill more than the frame holds. ".repeat(10);
    let a = app.run("text.create", json!({"x": 20, "y": 20, "text": text, "area": {"width": 150, "height": 60}})).unwrap()["id"].clone();
    app.run("text.thread.add", json!({"from": a, "at": [300, 200]})).unwrap();
    app.run("text.create", json!({"x": 20, "y": 300, "text": "x", "area": {"width": 50, "height": 20}})).unwrap();
    while app.session.active().unwrap().doc.layers[0].children().map_or(0, |c| c.len()) <= PORTS_MAX {
        app.run("select.all", json!({})).unwrap();
        app.run("edit.duplicate", json!({})).unwrap();
    }
    app.run("select.all", json!({})).unwrap();
    let o = thread_overlays(&app, 1.0);
    assert!(o.iter().any(|x| matches!(x, Overlay::Line { .. })), "the thread lines");
    assert!(!o.iter().any(|x| matches!(x, Overlay::Highlight { .. } | Overlay::Path { .. })), "no port boxes");
}

/// A frame of a thread in perspective has no ports: Show Text Threads draws no port box on it
/// and no line to or from it; the links between frames with ports keep theirs.
#[test]
fn text_threads_skip_the_links_of_a_frame_in_perspective() {
    use vectorcraft_tools::text::{Port, port_centre};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    let text = "Words that fill more than the frame holds. ".repeat(20);
    let id = |v: serde_json::Value| vectorcraft_doc::NodeId(v["id"].as_u64().unwrap());
    let a = id(app.run("text.create", json!({"x": 20, "y": 20, "text": text, "area": {"width": 150, "height": 60}})).unwrap());
    let b = id(app.run("text.thread.add", json!({"from": a.0, "at": [200, 20]})).unwrap());
    let c = id(app.run("text.thread.add", json!({"from": b.0, "at": [20, 200]})).unwrap());
    let d = id(app.run("text.thread.add", json!({"from": c.0, "at": [200, 200]})).unwrap());
    app.session
        .edit("Perspective", |doc, _| {
            if let Some(n) = doc.node_mut(b) {
                n.extras_mut().perspective = Some(Box::new(vectorcraft_doc::PerspectiveAttachment::new("left", 0.0)));
            }
            Ok(())
        })
        .unwrap();
    app.run("select.set", json!({"ids": [a.0, b.0, c.0, d.0]})).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.text_threads, [vec![a, b, c, d]], "b is still threaded");
    let frame = |id| match &st.doc.node(id).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("not text"),
    };
    let (out, input) = (port_centre(&frame(c), Port::Out, 1.0).unwrap(), port_centre(&frame(d), Port::In, 1.0).unwrap());
    let o = thread_overlays(&app, 1.0);
    let boxes: Vec<Point> =
        o.iter().filter_map(|x| if let Overlay::Highlight { quad, .. } = x { Some(quad[0].midpoint(quad[2])) } else { None }).collect();
    assert_eq!(boxes.len(), 2, "only c's out port and d's in port: {boxes:?}");
    assert!(boxes[0].distance(out) < 1e-9 && boxes[1].distance(input) < 1e-9, "{boxes:?}");
    // One thread line (c to d) and its two arrows of three strokes each.
    assert_eq!(o.iter().filter(|x| matches!(x, Overlay::Line { .. })).count(), 7);
}

/// #4319: Draw Inside marks the object it draws into with a dotted open rectangle round its
/// corners while the mode lasts, selected or not, and the marks go with the mode.
#[test]
fn draw_inside_marks_the_corners_of_its_object() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let id =
        vectorcraft_doc::NodeId(app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 120, "height": 90})).unwrap()["id"].as_u64().unwrap());
    let ctx = egui::Context::default();
    let color = c32(app.session.active().unwrap().doc.layer_color(id));
    // The marks' dashes: one-pixel dashes in the layer colour no longer than a dash.
    let dashes = |app: &mut VectorcraftApp| -> Vec<[Pos2; 2]> {
        shapes(app, &ctx)
            .into_iter()
            .filter_map(|s| match s {
                Shape::LineSegment { points, stroke } if stroke.color == color && stroke.width == 1.0 && points[0].distance(points[1]) <= 3.01 => {
                    Some(points)
                }
                _ => None,
            })
            .collect()
    };
    let normal = dashes(&mut app).len();
    app.run("view.drawMode", json!({"mode": "inside"})).unwrap();
    let marks = dashes(&mut app);
    // Eight arms of 16 px, each four dashes of 3 px with 2 px gaps.
    assert_eq!(marks.len() - normal, 32, "{marks:?}");
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let b = app.session.active().unwrap().doc.node(id).unwrap().visual_bounds().unwrap();
    let corner = xf.to_screen(Point::new(b.x0, b.y0)) - vec2(4.0, 4.0);
    assert!(marks.iter().any(|m| m[0].distance(corner) < 2.0), "a mark starts at the padded top-left corner");
    // Deselected, the object is still marked.
    app.run("select.set", json!({"ids": []})).unwrap();
    assert_eq!(dashes(&mut app).len(), marks.len());
    // Rotated, the arms follow the object's turned sides.
    app.view_mut().unwrap().rotation = 30.0;
    assert_eq!(dashes(&mut app).len(), marks.len());
    app.view_mut().unwrap().rotation = 0.0;
    app.run("view.drawMode", json!({"mode": "normal"})).unwrap();
    assert_eq!(dashes(&mut app).len(), normal);
}

/// #4681: a click on another object moves the Contextual Task Bar to it without hiding it for the
/// press (it flashed); a drag hides it until the drag ends.
#[test]
fn the_task_bar_stays_up_through_a_click_and_gives_way_to_a_drag() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 40, "y": 60, "width": 60, "height": 60})).unwrap();
    app.run("shape.rectangle", json!({"x": 260, "y": 60, "width": 60, "height": 60})).unwrap();
    app.select_tool("selection");
    let ctx = egui::Context::default();
    for _ in 0..30 {
        frame(&mut app, &ctx, vec![]);
    }
    let first = task_bar_rect(&ctx).expect("the task bar shows under the second rectangle");
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let at = xf.to_screen(Point::new(70.0, 90.0));
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), button(at, true)]);
    frame(&mut app, &ctx, vec![]);
    assert!(app.session.tool_busy(), "the press is under way");
    let pressed = task_bar_rect(&ctx).expect("the task bar stays up through the press");
    assert!(pressed.center().x < first.center().x - 50.0, "it moved under the rectangle pressed: {pressed:?} from {first:?}");
    // Dragged, it gives way until the drag ends.
    let to = at + vec2(40.0, 30.0);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at + vec2(20.0, 15.0))]);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(to)]);
    frame(&mut app, &ctx, vec![]);
    assert!(task_bar_rect(&ctx).is_none(), "hidden while the object is dragged");
    frame(&mut app, &ctx, vec![button(to, false)]);
    frame(&mut app, &ctx, vec![]);
    assert!(task_bar_rect(&ctx).is_some(), "back once the drag ends");
}

/// #4038: View › Hide Artboards leaves no artboard edges and shows the whole canvas as paper;
/// the Artboard tool still shows them.
#[test]
fn hidden_artboards_leave_a_paper_canvas_without_edges() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let ctx = egui::Context::default();
    let look = |app: &mut VectorcraftApp| {
        let shapes = shapes(app, &ctx);
        let canvas = app.canvas_rect.unwrap();
        let paper = shapes.iter().any(|s| matches!(s, Shape::Rect(r) if r.fill == Color32::WHITE && r.rect.contains_rect(canvas)));
        let edge =
            shapes.iter().any(|s| matches!(s, Shape::Path(p) if p.closed && p.stroke.color == egui::epaint::ColorMode::Solid(Color32::from_gray(0))));
        (paper, edge)
    };
    assert_eq!(look(&mut app), (false, true), "shown: an edge, the pasteboard round it");
    app.run("view.artboards", json!({})).unwrap();
    assert!(!app.ui.view.artboards);
    assert_eq!(look(&mut app), (true, false), "hidden: paper everywhere, no edge");
    app.select_tool("artboard");
    assert!(look(&mut app).1, "the Artboard tool shows them");
    app.select_tool("selection");
    app.run("view.artboards", json!({})).unwrap();
    assert_eq!(look(&mut app), (false, true));
}
