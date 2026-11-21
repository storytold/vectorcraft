use super::*;
use vectorcraft_engine::Session;
use vectorcraft_geom::Shape as _;

/// The marquee explains E without depending on color. At the canvas edge the hint stays in
/// view, in every brightness, and the two rules have visibly different outlines.
#[test]
fn marquee_feedback_explains_the_rule_and_stays_on_canvas() {
    let draw = |ui: &mut egui::Ui| {
        let t = Tokens::get(ui.ctx());
        let xf = Xf { rect: ui.max_rect(), zoom: 1.0, center: Point::new(160.0, 100.0), rot: 0.0 };
        let prefs = vectorcraft_engine::Prefs::default();
        for (enclosed, y) in [(false, 80.0), (true, 180.0)] {
            overlays::draw_overlays(
                ui.painter(),
                &xf,
                &[Overlay::SelectionMarquee { rect: Rect::new(20.0, y - 60.0, 300.0, y), enclosed, pointer: Point::new(315.0, y) }],
                &t,
                overlays::HandleLook::of(&prefs),
                &mut overlays::SharedPaths::default(),
            );
        }
    };
    for brightness in crate::theme::Brightness::ALL {
        for width in [160.0, 320.0] {
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            crate::theme::apply(&ctx, brightness);
            let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(width, 200.0))), ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| draw(ui));
            out.textures_delta.clear();
            let text = crate::tests_labels::shapes_text(&out);
            assert!(text.contains("Fully Enclosed · E: Touching") && text.contains("Touching · E: Fully Enclosed"), "{text}");
            for shape in &out.shapes {
                if let egui::Shape::Text(label) = &shape.shape {
                    assert!(
                        label.pos.x >= shape.clip_rect.min.x && label.pos.x + label.galley.size().x <= shape.clip_rect.max.x,
                        "{brightness:?}: hint clipped"
                    );
                    assert!(
                        label.pos.y >= shape.clip_rect.min.y && label.pos.y + label.galley.size().y <= shape.clip_rect.max.y,
                        "{brightness:?}: hint clipped vertically"
                    );
                }
            }
        }
    }
    // Optional visual QA of our own UI. Captures never enter the repository.
    if let Some(path) = std::env::var_os("VECTORCRAFT_MARQUEE_CAPTURE") {
        let mut ready = false;
        let mut harness = egui_kittest::Harness::builder().with_size(vec2(320.0, 200.0)).wgpu().build_ui(move |ui| {
            if !ready {
                crate::theme::install_fonts(ui.ctx());
                crate::theme::apply(ui.ctx(), crate::theme::Brightness::MediumDark);
                ready = true;
                return;
            }
            draw(ui);
        });
        harness.run_steps(4);
        harness.render().unwrap().save(path).unwrap();
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
    {
        let __dyn: image::DynamicImage = img.into();
        let (__w, __h) = (__dyn.width(), __dyn.height());
        let (__layout, __sample, __raw) = match __dyn {
            image::DynamicImage::ImageLuma8(b) => (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLumaA8(b) => (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgb8(b) => (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgba8(b) => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLuma16(b) => (
                craft_image_codecs::ChannelLayout::Gray,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageLumaA16(b) => (
                craft_image_codecs::ChannelLayout::GrayA,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgb16(b) => (
                craft_image_codecs::ChannelLayout::Rgb,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgba16(b) => (
                craft_image_codecs::ChannelLayout::Rgba,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            other => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, other.into_rgba8().into_raw()),
        };
        let __cimg = craft_image_codecs::Image::from_raw(__w, __h, __layout, __sample, __raw).unwrap();
        craft_image_codecs::encode(&__cimg, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default())
            .map(|__b| png.extend_from_slice(&__b))
    }
    .unwrap();
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
    {
        let __dyn: image::DynamicImage = img.into();
        let (__w, __h) = (__dyn.width(), __dyn.height());
        let (__layout, __sample, __raw) = match __dyn {
            image::DynamicImage::ImageLuma8(b) => (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLumaA8(b) => (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgb8(b) => (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgba8(b) => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLuma16(b) => (
                craft_image_codecs::ChannelLayout::Gray,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageLumaA16(b) => (
                craft_image_codecs::ChannelLayout::GrayA,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgb16(b) => (
                craft_image_codecs::ChannelLayout::Rgb,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgba16(b) => (
                craft_image_codecs::ChannelLayout::Rgba,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            other => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, other.into_rgba8().into_raw()),
        };
        let __cimg = craft_image_codecs::Image::from_raw(__w, __h, __layout, __sample, __raw).unwrap();
        craft_image_codecs::encode(&__cimg, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default())
            .map(|__b| png.extend_from_slice(&__b))
    }
    .unwrap();
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

#[test]
fn bucket_modifier_only_hover_updates_the_visible_cursor() {
    use egui::{Event, Modifiers};
    use vectorcraft_tools::Cursor;

    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.select_tool("livePaintBucket");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let at = app.canvas_rect.unwrap().center();
    let shown = |app: &mut VectorcraftApp, events| {
        frame(app, &ctx, events);
        app.canvas.hover_cursor.map(|(_, _, cursor)| cursor)
    };
    // The cursor swatch preview's chips go and come back with Alt too.
    let chips = |app: &mut VectorcraftApp| {
        let view = app.view_info();
        app.session.overlays(view).iter().filter(|o| matches!(o, vectorcraft_tools::Overlay::Swatch { .. })).count()
    };
    assert_eq!(shown(&mut app, vec![Event::PointerMoved(at)]), Some(Cursor::LivePaintBucket));
    assert!(chips(&mut app) > 0);
    assert_eq!(shown(&mut app, vec![Event::ModifiersChanged(Modifiers::ALT)]), Some(Cursor::Eyedropper));
    assert_eq!(chips(&mut app), 0);
    assert_eq!(shown(&mut app, vec![Event::ModifiersChanged(Modifiers::NONE)]), Some(Cursor::LivePaintBucket));
    assert!(chips(&mut app) > 0);
}

/// A press between a Live Paint Bucket triple-click's clicks that the view tools take (a
/// middle-button pan, a pan with Space held) counts as a press that paints nothing: the
/// triple-click matches the fill the face had before the first click.
#[test]
fn bucket_triple_click_with_a_view_tool_press_between_matches_the_fill_before() {
    use egui::{Event, Modifiers, PointerButton};
    use vectorcraft_color::{Color, Paint};
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    for case in ["middle", "space"] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let ids: Vec<u64> = [0.0, 50.0, 100.0]
            .iter()
            .map(|x| app.session.execute("shape.rectangle", &json!({"x": x, "y": 100, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap())
            .collect();
        app.session.execute("select.set", &json!({"ids": ids})).unwrap();
        let g = app.session.execute("livePaint.make", &json!({})).unwrap()["id"].as_u64().unwrap();
        for (x, hex) in [(25, "#ff0000"), (75, "#ff0000"), (125, "#0000ff")] {
            app.session.execute("livePaint.fill", &json!({"group": g, "point": [x, 125], "color": hex})).unwrap();
        }
        app.session.execute("select.none", &json!({})).unwrap();
        app.select_tool("livePaintBucket");
        app.session.paint.fill = green.clone();
        if let Some(v) = app.view_mut() {
            v.center = Point::new(200.0, 150.0);
            v.zoom = 1.0;
            v.fitted = true;
        }
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let at = xf.to_screen(Point::new(25.0, 125.0));
        let click = |app: &mut VectorcraftApp, button, pos, modifiers| {
            let b = |pressed| Event::PointerButton { pos, button, pressed, modifiers };
            frame(app, &ctx, vec![Event::PointerMoved(pos), b(true)]);
            frame(app, &ctx, vec![b(false)]);
        };
        click(&mut app, PointerButton::Primary, at, Modifiers::NONE);
        if case == "middle" {
            click(&mut app, PointerButton::Middle, at, Modifiers::NONE);
        } else {
            let space = |pressed| Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
            frame(&mut app, &ctx, vec![space(true)]);
            click(&mut app, PointerButton::Primary, at, Modifiers::NONE);
            frame(&mut app, &ctx, vec![space(false)]);
        }
        click(&mut app, PointerButton::Primary, at, Modifiers::NONE);
        frame(&mut app, &ctx, vec![]);
        let group = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(g)).unwrap().clone();
        let fill = |x: f64| vectorcraft_tools::builder::face_at(&group, Point::new(x, 125.0)).unwrap().appearance.fill_paint();
        assert_eq!([fill(25.0), fill(75.0)], [green.clone(), green.clone()], "{case}: the faces that were red");
        assert_eq!(fill(125.0), Paint::solid(Color::from_hex("#0000ff").unwrap()), "{case}: the blue face");
    }
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
        dispatch(&mut app, &PointerEvent { kind, pos: at, mods: Default::default(), backquote: false, pressure: 1.0, pen: None }, view);
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
        dispatch(&mut app, &PointerEvent { kind, pos: boards[2].center(), mods: shift, backquote: false, pressure: 1.0, pen: None }, view);
    }
    assert_eq!(app.view().unwrap().artboard, 2, "the clicked artboard is the active one");
    assert_eq!(app.session.active().unwrap().selection.objects, ids, "the selection stays");
    let lefts = |app: &VectorcraftApp| ids.map(|id| app.session.active().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().x0);
    app.run("object.align", json!({"horizontal": "left", "to": "artboard"})).unwrap();
    assert_eq!(lefts(&app), [boards[2].x0; 2], "the left edges are on the active artboard's");
    app.run("object.align", json!({"horizontal": "left", "to": "artboard", "artboard": 1})).unwrap();
    assert_eq!(lefts(&app), [boards[1].x0; 2], "an artboard the caller names is kept");
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

/// The texts one headless 800 × 600 canvas frame at `time` seconds draws, tooltips included.
fn texts_at(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, events: Vec<egui::Event>) -> Vec<String> {
    fn collect(shape: &Shape, texts: &mut Vec<String>) {
        match shape {
            Shape::Text(t) => texts.push(t.galley.text().to_string()),
            Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, texts)),
            _ => {}
        }
    }
    let screen_rect = Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)));
    let mut out = ctx.run_ui(egui::RawInput { screen_rect, time: Some(time), events, ..Default::default() }, |ui| show(app, ui));
    out.textures_delta.clear();
    let mut texts = vec![];
    out.shapes.iter().for_each(|s| collect(&s.shape, &mut texts));
    texts
}

/// The screen centers and radii of the Artboard tool's add-button tooltips (`tip`) in the view `xf`.
fn add_button_tips(app: &mut VectorcraftApp, xf: &Xf, tip: &str) -> Vec<(Pos2, f32)> {
    let view = app.view_info();
    app.session
        .overlays(view)
        .into_iter()
        .filter_map(|o| match o {
            Overlay::HoverText { center, radius, text } if text == tip => Some((xf.to_screen(center), (radius * xf.zoom) as f32)),
            _ => None,
        })
        .collect()
}

/// Whether `tip` shows while the pointer rests at `at` for two seconds from `*t`, which moves on.
fn rests_with_tooltip(app: &mut VectorcraftApp, ctx: &egui::Context, t: &mut f64, at: Pos2, tip: &str) -> bool {
    texts_at(app, ctx, *t, vec![egui::Event::PointerMoved(at)]);
    let mut seen = false;
    for _ in 0..20 {
        *t += 0.1;
        seen |= texts_at(app, ctx, *t, vec![]).iter().any(|s| s == tip);
    }
    seen
}

/// While Space lends a view tool, a press on an add button pans the view instead of adding an
/// artboard: the button shows no tooltip until Space is up.
#[test]
fn an_artboard_add_button_has_no_tooltip_while_space_is_held() {
    use egui::{Event, Modifiers};
    let tip = vectorcraft_tools::xform::EDGE_BUTTON_TIP;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    app.select_tool("artboard");
    let ctx = egui::Context::default();
    texts_at(&mut app, &ctx, 0.0, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let (at, _) = add_button_tips(&mut app, &xf, tip).into_iter().max_by(|a, b| a.0.y.total_cmp(&b.0.y)).unwrap();
    let space = |pressed| Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    let mut t = 1.0;
    texts_at(&mut app, &ctx, t, vec![Event::PointerMoved(at), space(true)]);
    for _ in 0..20 {
        t += 0.1;
        assert!(!texts_at(&mut app, &ctx, t, vec![]).iter().any(|s| s == tip), "no tooltip while Space is held");
    }
    t += 0.1;
    texts_at(&mut app, &ctx, t, vec![space(false)]);
    assert!(rests_with_tooltip(&mut app, &ctx, &mut t, at, tip), "the tooltip shows again once Space is up");
}

/// On a narrow artboard with a long name, the name can cover part of the top add button, and a
/// press there selects the named artboard: that part shows no tooltip, and the rest of the button
/// keeps it.
#[test]
fn an_artboard_add_button_has_no_tooltip_where_the_artboard_name_covers_it() {
    let tip = vectorcraft_tools::xform::EDGE_BUTTON_TIP;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 20, "height": 100})).unwrap();
    let name = "An artboard name long enough to run past the middle of its top edge";
    app.session.execute("artboard.setProps", &json!({"artboards": [{"index": 0, "name": name}]})).unwrap();
    app.select_tool("artboard");
    let ctx = egui::Context::default();
    texts_at(&mut app, &ctx, 0.0, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let (at, r) = add_button_tips(&mut app, &xf, tip).into_iter().min_by(|a, b| a.0.y.total_cmp(&b.0.y)).unwrap();
    // The points of the button's disc, split by whether the canvas's name hit test takes them.
    let probe = egui::Context::default();
    let named = |p: Pos2| {
        let mut hit = false;
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
        let mut out = probe.run_ui(raw, |ui| hit = super::artboards::artboard_name_at(&app, ui, &xf, p).is_some());
        out.textures_delta.clear();
        hit
    };
    let steps = || (-6i8..=6).map(f32::from);
    let disc: Vec<Pos2> = steps().flat_map(|dx| steps().map(move |dy| at + vec2(dx, dy))).filter(|p| p.distance(at) <= r - 0.5).collect();
    let covered = disc.iter().copied().find(|p| named(*p)).expect("the name covers part of the button");
    let clear = disc.iter().copied().rev().find(|p| !named(*p)).expect("part of the button is clear of the name");
    let mut t = 1.0;
    assert!(!rests_with_tooltip(&mut app, &ctx, &mut t, covered, tip), "no tooltip where the name takes the press");
    assert!(rests_with_tooltip(&mut app, &ctx, &mut t, clear, tip), "the tooltip shows on the rest of the button");
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

/// In the Touch workspace a two-finger tap switches between the Selection tool and the drawing tool
/// chosen last, and undoes nothing; three fingers still redo. Outside it, two fingers undo again.
#[test]
fn two_finger_tap_in_touch_switches_selection_and_drawing_modes() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 40, "height": 40})).unwrap();
    crate::workspaces::switch(&mut app.ui, crate::workspaces::TOUCH).unwrap();
    app.select_tool("rectangle");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let count = |app: &VectorcraftApp| app.session.active().unwrap().doc.layers[0].children().unwrap().len();
    let c = app.canvas_rect.unwrap().center();
    let touch = |n: u64, phase, pos| egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(n), phase, pos, force: None };
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
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
    assert_eq!((app.session.tool_id(), count(&app)), ("selection", 1), "selection mode, nothing undone or drawn");
    tap(&mut app, 2);
    assert_eq!((app.session.tool_id(), count(&app)), ("rectangle", 1), "back to the drawing tool");
    // A tool chosen by hand becomes the drawing tool a switch returns to.
    app.select_tool("ellipse");
    tap(&mut app, 2);
    tap(&mut app, 2);
    assert_eq!((app.session.tool_id(), count(&app)), ("ellipse", 1));
    // The switches added no undo step: undo still takes the rectangle away, and three fingers redo it.
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(count(&app), 0);
    tap(&mut app, 3);
    assert_eq!(count(&app), 1, "three fingers redo in Touch too");
    // Presentation Mode runs neither.
    app.ui.screen_mode = 3;
    frame(&mut app, &ctx, vec![]);
    tap(&mut app, 2);
    assert_eq!(app.session.tool_id(), "ellipse", "Presentation Mode: no switch");
    app.ui.screen_mode = 0;
    // Enable Touch Gestures off: no switch.
    app.session.prefs.touch_gestures = false;
    app.select_tool("selection");
    frame(&mut app, &ctx, vec![]);
    tap(&mut app, 2);
    assert_eq!(app.session.tool_id(), "selection", "off: no switch");
    app.session.prefs.touch_gestures = true;
    // Another workspace: two fingers undo again and switch no tool.
    crate::workspaces::switch(&mut app.ui, crate::workspaces::ESSENTIALS).unwrap();
    app.select_tool("rectangle");
    frame(&mut app, &ctx, vec![]);
    let before = count(&app);
    tap(&mut app, 2);
    assert_eq!(app.session.tool_id(), "rectangle");
    assert_eq!(count(&app), before - 1, "Essentials: undone");
}

/// One frame of the canvas with `events`, with the modal backdrop drawn over it when `backdrop` is set, as the app
/// draws it for an open dialog.
fn modal_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>, backdrop: bool) {
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| {
        show(app, ui);
        if backdrop {
            crate::dialogs::modal::show(ui.ctx(), "Test", egui::Id::unique("test-dialog"), 0.0, 8, |ui| ui.label("A dialog"));
        }
    });
    out.textures_delta.clear();
}

/// Finger `n` touching at `pos`.
fn finger(n: u64, phase: egui::TouchPhase, pos: Pos2) -> egui::Event {
    egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(n), phase, pos, force: None }
}

/// The primary button pressed or released at `pos`.
fn finger_button(pos: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
}

/// A tap of `fingers` fingers beside `c`, the first also pressing as the pointer and moving a little, in frames drawn
/// with the modal backdrop when `backdrop` is set ([`modal_frame`]).
fn finger_tap(app: &mut VectorcraftApp, ctx: &egui::Context, c: Pos2, fingers: u64, backdrop: bool) {
    use egui::TouchPhase::{End, Move, Start};
    let at = |n: u64| c + vec2(n as f32 * 50.0, 0.0);
    let moved = at(0) + vec2(6.0, 6.0);
    modal_frame(app, ctx, vec![finger(0, Start, at(0)), egui::Event::PointerMoved(at(0)), finger_button(at(0), true)], backdrop);
    modal_frame(app, ctx, vec![finger(0, Move, moved), egui::Event::PointerMoved(moved)], backdrop);
    modal_frame(app, ctx, (1..fingers).map(|n| finger(n, Start, at(n))).collect(), backdrop);
    modal_frame(app, ctx, vec![finger(0, End, at(0)), finger_button(moved, false), egui::Event::PointerGone], backdrop);
    modal_frame(app, ctx, (1..fingers).map(|n| finger(n, End, at(n))).collect(), backdrop);
    modal_frame(app, ctx, vec![], backdrop);
}

/// Switch Selection and Drawing Modes from the command: Pencil when no drawing tool was chosen yet,
/// a view tool is left for Selection without becoming the drawing tool, and every selection tool
/// switches to drawing mode.
#[test]
fn switch_selection_and_drawing_modes_command() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    assert!(app.ui.touch_drawing_tool.is_none());
    app.select_tool("selection");
    let r = app.run(crate::touch::TOGGLE_MODE, json!({})).unwrap();
    assert_eq!((r, app.session.tool_id()), (json!({"tool": "pencil"}), "pencil"), "no drawing tool chosen yet: Pencil");
    app.select_tool("hand");
    assert_eq!(app.run(crate::touch::TOGGLE_MODE, json!({})).unwrap(), json!({"tool": "selection"}));
    assert_eq!(app.run(crate::touch::TOGGLE_MODE, json!({})).unwrap(), json!({"tool": "pencil"}), "the Hand tool is no drawing tool");
    app.select_tool("pen");
    for id in ["directSelection", "groupSelection", "magicWand", "lasso"] {
        app.select_tool(id);
        assert_eq!(app.run(crate::touch::TOGGLE_MODE, json!({})).unwrap(), json!({"tool": "pen"}), "{id}");
    }
}

/// Select Last Selection Tool: the Selection tool before any selection tool is chosen, then the
/// one of Selection, Direct Selection and Group Selection chosen last; Magic Wand and Lasso don't
/// count. Running it again keeps that tool, and it changes no art, selection or undo history.
#[test]
fn select_last_selection_tool_command() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap()["id"].clone();
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    let last = |app: &mut VectorcraftApp| app.run("tool.selectLastSelection", json!({})).unwrap();
    let state = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        (st.doc.clone(), st.selection.clone(), st.history.undo.len())
    };
    app.select_tool("pen");
    let before = state(&app);
    assert_eq!((last(&mut app), app.session.tool_id()), (json!({"tool": "selection"}), "selection"), "none chosen yet: Selection");
    for id in ["directSelection", "groupSelection", "selection"] {
        app.select_tool(id);
        app.select_tool("pen");
        assert_eq!(last(&mut app), json!({"tool": id}), "{id}");
    }
    app.select_tool("directSelection");
    for other in ["magicWand", "lasso", "rectangle", "hand"] {
        app.select_tool(other);
    }
    assert_eq!(last(&mut app), json!({"tool": "directSelection"}), "Magic Wand and Lasso are not remembered");
    assert_eq!(last(&mut app), json!({"tool": "directSelection"}), "again: the same tool, no cycling");
    assert!(state(&app) == before, "no art, selection or undo step changes");
}

/// A Switch Selection and Drawing Modes whose tool choice fails returns the error, in both
/// directions: the tool stays, the drawing tool to return to stays, and no undo step is added.
#[test]
fn a_failed_mode_switch_returns_its_error() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 40, "y": 40, "width": 30, "height": 20})).unwrap();
    let undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
    let steps = undo(&app);
    app.select_tool("rectangle");
    app.ui.touch_drawing_tool = Some("pen".into());
    app.fail_next_tool_choice = Some("no switch".into());
    let r = app.run(crate::touch::TOGGLE_MODE, json!({}));
    assert!(r.as_ref().is_err_and(|e| e.contains("no switch")), "{r:?}");
    assert_eq!((app.session.tool_id(), app.ui.touch_drawing_tool.as_deref(), undo(&app)), ("rectangle", Some("pen"), steps), "drawing mode");
    app.select_tool("selection");
    app.fail_next_tool_choice = Some("no switch".into());
    let r = app.run(crate::touch::TOGGLE_MODE, json!({}));
    assert!(r.as_ref().is_err_and(|e| e.contains("no switch")), "{r:?}");
    assert_eq!((app.session.tool_id(), app.ui.touch_drawing_tool.as_deref(), undo(&app)), ("selection", Some("pen"), steps), "selection mode");
    // The next switch works again.
    assert_eq!(app.run(crate::touch::TOGGLE_MODE, json!({})).unwrap(), json!({"tool": "pen"}));
    assert_eq!(undo(&app), steps);
}

/// The commands that choose a tool (Select Tool, Crop Image, and the Path Precision Tools' Corner
/// Tool and Shared Edge Merge Tool) return the error when the choice fails, and the tool and the
/// toolbar group's tool stay as they were.
#[test]
fn tool_commands_return_a_failed_choice() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let img = image::RgbaImage::from_pixel(20, 10, image::Rgba([200, 120, 40, 255]));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let data = vectorcraft_format::base64_encode(&png);
    app.run("file.place", json!({"name": "texture.png", "dataBase64": data, "link": false, "at": [150, 150]})).unwrap();
    app.select_tool("selection");
    let group = vectorcraft_tools::catalog::group_of("ellipse").unwrap();
    let commands = [
        ("tool.select", json!({"tool": "ellipse"})),
        ("ui.cropImage", json!({})),
        (crate::addons::pathtools::corner::COMMAND, json!({})),
        (crate::addons::pathtools::shared_edge::COMMAND, json!({})),
    ];
    for (command, params) in commands {
        let slot = app.ui.group_tool[group].clone();
        app.fail_next_tool_choice = Some("no switch".into());
        let r = app.run(command, params.clone());
        assert!(r.as_ref().is_err_and(|e| e.contains("no switch")), "{command}: {r:?}");
        assert_eq!(app.session.tool_id(), "selection", "{command}");
        assert_eq!(app.ui.group_tool[group], slot, "{command}");
        app.run(command, params).unwrap();
        assert_ne!(app.session.tool_id(), "selection", "{command} works again");
        app.select_tool("selection");
    }
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

/// The box-drawing shape tools show the bounding box of the shape just drawn (#5477). A drag of
/// its bottom-right handle with the tool still active scales that shape, as with the Selection
/// tool, rather than drawing a second shape from the handle.
#[test]
fn a_shape_tool_scales_the_shape_just_drawn_by_its_bounding_box_handle() {
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    for tool in ["rectangle", "roundedRectangle", "ellipse", "polygon", "star"] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.select_tool(tool);
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let drag = |app: &mut VectorcraftApp, a: Pos2, b: Pos2| {
            for ev in [
                egui::Event::PointerMoved(a),
                button(a, true),
                egui::Event::PointerMoved(a.lerp(b, 0.5)),
                egui::Event::PointerMoved(b),
                button(b, false),
            ] {
                frame(app, &ctx, vec![ev]);
            }
            frame(app, &ctx, vec![]);
        };
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        drag(&mut app, xf.to_screen(Point::new(150.0, 120.0)), xf.to_screen(Point::new(210.0, 170.0)));
        let objects = |app: &VectorcraftApp| {
            let mut n = 0;
            app.session.active().unwrap().doc.walk(|node| n += usize::from(node.path_data().is_some()));
            n
        };
        assert_eq!(objects(&app), 1, "{tool}: the drag draws one shape");
        let before = app.selection_box().unwrap_or_else(|| panic!("{tool}: the shape just drawn shows its box"));
        // The bottom-right handle, dragged 40 pt along the box's width and 30 pt along its height
        // (a polygon's or star's box turns with the shape).
        let (corner, to) = (Point::new(before.rect.x1, before.rect.y1), Point::new(before.rect.x1 + 40.0, before.rect.y1 + 30.0));
        drag(&mut app, xf.to_screen(before.to_doc() * corner), xf.to_screen(before.to_doc() * to));
        let after = app.selection_box().unwrap_or_else(|| panic!("{tool}: the shape stays selected"));
        let label = app.session.active().and_then(|st| st.history.undo.last().map(|e| e.label.clone()));
        assert_eq!(objects(&app), 1, "{tool}: no second shape, last step {label:?}");
        assert!(
            (after.rect.width() - before.rect.width() - 40.0).abs() < 0.5 && (after.rect.height() - before.rect.height() - 30.0).abs() < 0.5,
            "{tool}: {:?} -> {:?}, last step {label:?}",
            before.rect,
            after.rect
        );
        assert_eq!(label.as_deref(), Some("Scale"), "{tool}");
    }
}

/// An artboard's guide label colors its own guides only: a canvas guide, a guide of an artboard
/// without a label and a guide of a deleted artboard keep the guide color preference, and a
/// selected labeled guide takes the selection color. Hide Guides hides labeled guides too.
#[test]
fn artboard_guide_label_colors_only_its_own_guides() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("artboard.new", &json!({})).unwrap();
    app.session.execute("artboard.new", &json!({})).unwrap();
    app.session.execute("artboard.setProps", &json!({"index": 0, "guideLabel": "Red"})).unwrap();
    app.session.execute("artboard.setProps", &json!({"index": 2, "guideLabel": "#00ff00"})).unwrap();
    // Vertical guides at x 100 (artboard 1, Red), 520 (artboard 2, no label) and 940 (artboard
    // 3, whose artboard is then deleted), and a horizontal canvas guide at y 150.
    for (pos, artboard) in [(100, json!(0)), (520, json!(1)), (940, json!(2))] {
        app.session.execute("guide.add", &json!({"vertical": true, "pos": pos, "artboard": artboard})).unwrap();
    }
    app.session.execute("guide.add", &json!({"vertical": false, "pos": 150})).unwrap();
    // Deleting artboard 3 deletes its guide; a guide still naming its id (as a file might) is an
    // orphan, drawn across the canvas.
    let gone = app.session.active().unwrap().doc.artboards[2].id;
    app.session.execute("artboard.setProps", &json!({"index": 2, "guideLabel": null})).unwrap();
    app.session.execute("artboard.delete", &json!({"index": 2})).unwrap();
    app.session.execute("guide.add", &json!({"vertical": true, "pos": 300})).unwrap();
    {
        let st = app.session.active_mut().unwrap();
        let doc = std::sync::Arc::make_mut(&mut st.doc);
        let last = doc.ruler_guides().last().and_then(|g| g.id).unwrap();
        assert!(doc.edit_ruler_guide(last, |g| g.artboard = Some(gone)));
    }
    app.ui.view.guides = true;
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let t = Tokens::get(&ctx);
    let pref = LineLook::guides(&app.session.prefs, &t).color;
    let colors = |app: &mut VectorcraftApp, at: f64, vertical: bool| -> Vec<Color32> {
        shapes(app, &ctx)
            .iter()
            .filter_map(|s| match s {
                Shape::LineSegment { points, stroke } => {
                    let (a, b) = (xf.to_doc(points[0]), xf.to_doc(points[1]));
                    let (u, v) = if vertical { (a.x, b.x) } else { (a.y, b.y) };
                    ((u - at).abs() < 0.01 && (v - at).abs() < 0.01).then_some(stroke.color)
                }
                _ => None,
            })
            .collect()
    };
    let red = Color32::from_rgb(0xff, 0x4f, 0x4f);
    assert_eq!(colors(&mut app, 100.0, true), [red], "the labeled artboard's guide");
    assert_eq!(colors(&mut app, 520.0, true), [pref], "an artboard without a label");
    assert_eq!(colors(&mut app, 150.0, false), [pref], "a canvas guide");
    assert!(colors(&mut app, 940.0, true).is_empty(), "deleted with its artboard");
    assert_eq!(colors(&mut app, 300.0, true), [pref], "a guide whose artboard is gone");
    assert_ne!(red, pref);
    // Selected, it is drawn in the selection color; deselected, in the label's again.
    app.session.execute("guide.select", &json!({"indexes": [0]})).unwrap();
    assert_eq!(colors(&mut app, 100.0, true), [t.selection], "selected");
    app.session.execute("guide.select", &json!({"indexes": []})).unwrap();
    assert_eq!(colors(&mut app, 100.0, true), [red], "deselected");
    // A new label shows on the next frame.
    app.session.execute("artboard.setProps", &json!({"index": 0, "guideLabel": "Black"})).unwrap();
    assert_eq!(colors(&mut app, 100.0, true), [Color32::BLACK]);
    app.ui.view.guides = false;
    assert!(colors(&mut app, 100.0, true).is_empty(), "Hide Guides hides it");
}

/// With Preferences › Performance › Real-time Drawing and Editing off, the bounding box moves
/// with a Selection-tool move drag while the object stays put until the release. With it on, the
/// art moves and the box hides during the drag (#712), unless General › Show Bounding Box While
/// Moving and Drawing is on, which keeps the box with the art.
#[test]
fn real_time_drawing_off_moves_the_bounding_box_with_a_move_drag() {
    for (real_time, while_moving) in [(true, false), (false, false), (true, true), (false, true)] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.session.execute("prefs.set", &json!({"key": "realTimeDrawing", "value": real_time})).unwrap();
        app.session.execute("prefs.set", &json!({"key": "boundingBoxWhileMovingAndDrawing", "value": while_moving})).unwrap();
        app.ui.task_bar = false;
        app.ui.view.rulers = false;
        app.ui.view.smart_guides = false;
        app.ui.view.snap_to_point = false;
        let id = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 60})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        app.select_tool("selection");
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let is_handle = |s: &Shape| matches!(s, Shape::Rect(r) if (r.rect.width() - 6.0).abs() < 1.0 && (r.rect.height() - 6.0).abs() < 1.0 && r.fill == Color32::WHITE);
        let handles = |app: &mut VectorcraftApp| {
            shapes(app, &ctx).iter().filter(|s| is_handle(s)).map(|s| s.visual_bounding_rect().center()).collect::<Vec<_>>()
        };
        let at_rest = handles(&mut app);
        assert_eq!(at_rest.len(), 8, "real-time drawing {real_time}: the box's handles at rest");
        let left = |app: &VectorcraftApp| app.session.active().unwrap().doc.art_bounds().unwrap().x0;
        let start = left(&app);
        let view = app.view_info();
        app.session.pointer(&PointerEvent::new(PointerKind::Down, 150.0, 130.0), view).unwrap();
        app.session.pointer(&PointerEvent::new(PointerKind::Drag, 250.0, 230.0), view).unwrap();
        let during = handles(&mut app);
        let what = format!("real-time drawing {real_time}, box while moving {while_moving}");
        if real_time && !while_moving {
            assert!(during.is_empty(), "{what}: the box hides while the art moves: {during:?}");
        } else {
            assert_eq!(during.len(), 8, "{what}: the box shows during the drag");
            let corner = xf.to_screen(Point::new(200.0, 200.0));
            assert!(during.iter().any(|c| c.distance(corner) < 0.5), "{what}: the box's corner is at the dragged position: {during:?}");
            assert!(!during.iter().any(|c| c.distance(xf.to_screen(Point::new(100.0, 100.0))) < 0.5), "{what}: not at the start");
        }
        if real_time {
            assert_eq!(left(&app), start + 100.0, "{what}: the art moves with the pointer");
        } else {
            assert_eq!(left(&app), start, "{what}: the object waits for the release");
        }
        app.session.pointer(&PointerEvent::new(PointerKind::Up, 250.0, 230.0), view).unwrap();
        assert_eq!(left(&app), start + 100.0, "real-time drawing {real_time}: moved on release");
        assert_eq!(handles(&mut app).len(), 8, "real-time drawing {real_time}: the box at rest again");
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
                let guides = &app.session.active().unwrap().doc.ruler_guides();
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
            let guides = &app.session.active().unwrap().doc.ruler_guides();
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
            assert!(app.session.active().unwrap().doc.ruler_guides().is_empty());
        }
    }
}

/// With Preferences › Guides & Grid › Artboard Guides with Cmd/Ctrl + Ruler Drag on, Cmd/Ctrl held
/// when a drag leaves either ruler makes a guide of the active artboard, even when the key is let
/// go before the release; a guide outside the artboard is not made. With the preference off, the
/// same drag makes a canvas guide.
#[test]
fn cmd_ruler_drags_make_artboard_guides_with_the_preference_on() {
    use egui::{Event, Modifiers};
    for source_vertical in [false, true] {
        for on in [false, true] {
            let mut app = VectorcraftApp::new(Session::new(), Default::default());
            app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
            app.session.execute("prefs.set", &json!({"key": "cmdRulerDragArtboardGuides", "value": on})).unwrap();
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
            let drag = |app: &mut VectorcraftApp, to: Point| {
                let at = xf.to_screen(to);
                assert!(rect.contains(at));
                let button = |pos, pressed, modifiers| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
                frame(app, &ctx, vec![Event::ModifiersChanged(Modifiers::COMMAND), Event::PointerMoved(from)]);
                frame(app, &ctx, vec![button(from, true, Modifiers::COMMAND)]);
                frame(app, &ctx, vec![Event::PointerMoved(at)]);
                frame(app, &ctx, vec![Event::ModifiersChanged(Modifiers::NONE), Event::PointerMoved(at), button(at, false, Modifiers::NONE)]);
            };
            let board = app.session.active().unwrap().doc.artboards[0].id;
            drag(&mut app, Point::new(80.0, 120.0));
            let st = app.session.active().unwrap();
            assert_eq!(st.doc.ruler_guides().len(), 1, "vertical {source_vertical}, on {on}");
            assert_eq!(st.doc.ruler_guides()[0].vertical, source_vertical);
            assert_eq!(st.doc.ruler_guides()[0].artboard, on.then_some(board), "vertical {source_vertical}, on {on}");
            // Beyond the artboard's right or bottom edge.
            let beyond = xf.to_doc(rect.right_bottom() - egui::vec2(2.0, 2.0));
            assert!(beyond.x > 400.0 && beyond.y > 300.0, "{beyond:?}");
            drag(&mut app, beyond);
            assert_eq!(app.session.active().unwrap().doc.ruler_guides().len(), if on { 1 } else { 2 }, "vertical {source_vertical}, on {on}");
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
    let guides = |app: &VectorcraftApp| app.session.active().unwrap().doc.ruler_guides().iter().map(|g| (g.vertical, g.pos)).collect::<Vec<_>>();
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
    let guides = |app: &VectorcraftApp| app.session.active().unwrap().doc.ruler_guides().iter().map(|g| (g.vertical, g.pos)).collect::<Vec<_>>();
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
    assert_eq!((vectorcraft_engine::cmd::guide_indexes(&st.doc, &st.selection.guides), st.selection.objects.clone()), (vec![0], vec![]));
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

/// #6369: only the selection box remains over the temporary halftone, and the original
/// selection's marks come back when Preview is turned off or the dialog is cancelled.
#[test]
fn halftone_preview_hides_dot_outlines_and_keeps_selection_bounds() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    app.ui.view.bounding_box = true;
    let original = app.run("shape.ellipse", json!({"x": 50, "y": 50, "width": 120, "height": 100})).unwrap()["id"].as_u64().unwrap();
    let original = vectorcraft_doc::NodeId(original);
    app.run("paint.setFill", json!({"color": "#000000"})).unwrap();
    let canvas = egui::Context::default();
    let dialog = egui::Context::default();
    let dialog_frame = |app: &mut VectorcraftApp| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
        let mut out = dialog.run_ui(raw, |ui| crate::dialogs::show(app, ui.ctx()));
        out.textures_delta.clear();
    };
    let has_interior_marks = |app: &mut VectorcraftApp| {
        frame(app, &canvas, vec![]);
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let corners = app.selection_box().unwrap().corners().map(|p| xf.to_screen(p));
        let inside = egui::Rect::from_points(&corners).shrink(10.0);
        let st = app.session.active().unwrap();
        let color = c32(st.doc.layer_color(st.selection.objects[0]));
        let points = drawn_in(app, &canvas, color, 1.0);
        assert!(corners.iter().all(|p| points.contains(p)), "the outer selection box is still drawn");
        points.iter().any(|p| inside.contains(*p))
    };
    assert!(has_interior_marks(&mut app), "the selected ellipse shows its outline");

    app.run("ui.menuDialog", json!({"command": "object.vectorHalftone"})).unwrap();
    dialog_frame(&mut app);
    assert!(app.session.in_interaction());
    assert_ne!(app.session.active().unwrap().selection.objects, vec![original], "the halftone is previewed and selected");
    assert!(!has_interior_marks(&mut app), "selection marks cover the halftone dots");

    let set_preview = |app: &mut VectorcraftApp, on| {
        crate::dialogs::set_field(app.ui.dialog.as_mut().unwrap(), "preview".into(), json!(on));
        dialog_frame(app);
    };
    set_preview(&mut app, false);
    assert_eq!(app.session.active().unwrap().selection.objects, vec![original]);
    assert!(has_interior_marks(&mut app), "turning Preview off restores the outline");

    set_preview(&mut app, true);
    assert!(!has_interior_marks(&mut app), "turning Preview back on hides the dot marks again");
    crate::dialogs::cancel(&mut app);
    assert_eq!(app.session.active().unwrap().selection.objects, vec![original]);
    assert!(has_interior_marks(&mut app), "Cancel restores the outline");

    app.run("ui.menuDialog", json!({"command": "object.move"})).unwrap();
    crate::dialogs::set_field(app.ui.dialog.as_mut().unwrap(), "preview".into(), json!(true));
    dialog_frame(&mut app);
    assert!(app.session.in_interaction());
    assert!(has_interior_marks(&mut app), "another dialog's preview keeps selection marks");
    crate::dialogs::cancel(&mut app);

    app.run("ui.menuDialog", json!({"command": "object.vectorHalftone"})).unwrap();
    dialog_frame(&mut app);
    assert!(!has_interior_marks(&mut app));
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none() && !app.session.in_interaction());
    assert!(has_interior_marks(&mut app), "OK restores ordinary selection marks on the applied dots");
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
        // The Line Segment tool draws nothing of its own (no bounding box, no type on a path
        // brackets; the box-drawing shape tools show the bounding box, #5477): only the
        // selection's overlay changes the frame.
        app.select_tool("lineSegment");
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
/// GPU takes in one buffer, without the art's raster: the render worker's frame can land in
/// either frame of a comparison.
fn frame_vertices(app: &mut VectorcraftApp, ctx: &egui::Context) -> usize {
    frame(app, ctx, vec![]);
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| show(app, ui));
    out.textures_delta.clear();
    let art = app.canvas.texture.as_ref().map(|t| t.id());
    let mesh = |p: &egui::ClippedPrimitive| match &p.primitive {
        egui::epaint::Primitive::Mesh(m) if Some(m.texture_id) != art => m.vertices.len(),
        _ => 0,
    };
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

/// A canvas more device pixels across than the largest texture side the display supports renders
/// its art at fewer pixels a point, within that size, and draws it over the whole canvas. Pixel
/// Preview's region of document pixels stays within it too.
#[test]
fn the_art_renders_within_the_largest_texture_side() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width":400,"height":300})).unwrap();
    app.canvas.worker_started = true; // every frame on this thread, however long it takes
    let ctx = egui::Context::default();
    ctx.set_pixels_per_point(2.0);
    // The first frame sets the pixels a point, the second draws at them.
    for _ in 0..2 {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
            max_texture_side: Some(1024),
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| show(&mut app, ui));
        out.textures_delta.clear();
    }
    let rect = app.canvas_rect.unwrap();
    assert!(rect.width() * ctx.pixels_per_point() > 1024.0, "the canvas is {rect:?} at {} pixels a point", ctx.pixels_per_point());
    let size = app.canvas.texture.as_ref().map(|t| t.size()).unwrap();
    assert_eq!(size[0].max(size[1]), 1024, "the frame is {size:?} pixels");
    let k = app.canvas.key.clone().unwrap();
    assert!(
        (k.w as f32 / k.ppp - rect.width()).abs() < 1.0 && (k.h as f32 / k.ppp - rect.height()).abs() < 1.0,
        "the frame covers the canvas {rect:?}: {} × {} pixels at {} a point",
        k.w,
        k.h,
        k.ppp
    );

    let xf = Xf { rect: egui::Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 500.0)), zoom: 0.5, center: Point::new(0.0, 0.0), rot: 0.0 };
    let [x0, _, x1, _] = pixel_region(&xf, 4096).expect("a region 2,000 document pixels wide");
    assert_eq!(x1 - x0, 2000);
    assert_eq!(pixel_region(&xf, 1024), None, "more than the display's largest texture");
}

/// Run a test that compares two renders with the color settings that are active when it starts.
/// A test on another thread can change the process's active settings; this thread keeps its own.
fn keeping_color_settings(test: fn()) {
    vectorcraft_color::cms::with_active(vectorcraft_color::cms::active(), test);
}

/// A placed document's bitmap made in the background renders the canvas again only where the
/// objects showing its file are: both copies of the file are in the part rendered again.
#[test]
fn a_ready_placed_document_bitmap_renders_only_its_part_of_the_canvas() {
    keeping_color_settings(a_ready_placed_bitmap);
}

fn a_ready_placed_bitmap() {
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
    while vectorcraft_render::placed_document::busy() {
        std::thread::yield_now();
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

/// Preferences › Performance › Use Monitor Profile for Canvas: with a display profile read, the
/// art is drawn converted into it, and drawn again when the profile or the preference changes.
#[test]
fn the_canvas_is_drawn_through_the_display_profile_while_the_preference_is_on() {
    use crate::display_profile::{Found, Reading};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 100, "y": 50, "width": 200, "height": 200})).unwrap();
    app.session.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    assert!(app.session.prefs.monitor_profile_display, "on by default");
    let ctx = egui::Context::default();
    // How many pixels of the art's raster one frame uploads are `c`, None when it was not re-rendered.
    let count = |app: &mut VectorcraftApp, c: egui::Color32| -> Option<usize> {
        let (_, mut delta) = frame_output(app, &ctx);
        let art = app.canvas.texture.as_ref().unwrap().id();
        let n = delta.set.iter().find(|e| e.0.eq(&art)).and_then(|e| e.1.last()).map(|d| {
            let egui::ImageData::Color(img) = &d.image;
            img.pixels.iter().filter(|p| **p == c).count()
        });
        delta.clear();
        n
    };
    let red = egui::Color32::RED;
    let srgb = count(&mut app, red).expect("the first frame");
    assert!(srgb > 1000, "the rectangle in sRGB: {srgb}");
    let p3 = crate::display_profile::tests::p3();
    let t = vectorcraft_color::cms::display::DisplayTransform::from_icc(&p3).unwrap();
    let [r, g, b] = t.apply8([255, 0, 0]);
    let shown = egui::Color32::from_rgb(r, g, b);
    assert_ne!(shown, red, "sRGB red is less saturated on a wide-gamut display");
    let reading = |found| Reading { display: Some("DP-1".into()), found };
    assert!(app.display_profile.report(reading(Found::Profile { icc: p3.clone(), from: "test".into() })));
    assert_eq!(count(&mut app, shown), Some(srgb), "drawn again, converted");
    // The same profile again: nothing is drawn again.
    assert!(!app.display_profile.report(reading(Found::Profile { icc: p3, from: "test".into() })));
    assert_eq!(count(&mut app, shown), None);
    // Off: drawn again in sRGB.
    app.session.prefs.monitor_profile_display = false;
    assert_eq!(count(&mut app, red), Some(srgb));
    // On again, then the system takes over: sRGB.
    app.session.prefs.monitor_profile_display = true;
    assert_eq!(count(&mut app, shown), Some(srgb));
    app.display_profile.report(reading(Found::System("test".into())));
    assert_eq!(count(&mut app, red), Some(srgb));
}

/// With a display profile, translucent art and the anti-aliased edges of art over the paper are
/// blended with the paper in sRGB before the conversion: drawn over the converted paper, each such
/// pixel shows the converted blend. Opaque art, and art over the pasteboard, convert as before. The
/// part an edit renders again and the render worker's frame do the same, a change of the
/// transparency grid renders the art again, and Outline converts the art alone.
#[test]
fn translucent_art_over_the_paper_is_blended_before_the_display_profile() {
    keeping_color_settings(translucent_art_over_the_paper);
}

fn translucent_art_over_the_paper() {
    use crate::display_profile::{Found, Reading};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    app.canvas.worker_started = true; // every frame on this thread
    let half = app.run("shape.rectangle", json!({"x": 100, "y": 50, "width": 200, "height": 200})).unwrap()["id"].clone();
    app.run("paint.setFill", json!({"ids": [half], "color": "#ff0000"})).unwrap();
    app.run("transparency.set", json!({"ids": [half], "opacity": 50})).unwrap();
    let disc = app.run("shape.ellipse", json!({"x": 20, "y": 20, "width": 60, "height": 60})).unwrap()["id"].clone();
    app.run("paint.setFill", json!({"ids": [disc], "color": "#2040c0"})).unwrap();
    // Left of the artboard, over the pasteboard.
    let outside = app.run("shape.rectangle", json!({"x": -60, "y": 100, "width": 40, "height": 40})).unwrap()["id"].clone();
    app.run("paint.setFill", json!({"ids": [outside], "color": "#00ff00"})).unwrap();
    app.run("transparency.set", json!({"ids": [outside], "opacity": 50})).unwrap();
    app.run("select.none", json!({})).unwrap();
    let ctx = egui::Context::default();
    // The art's raster one frame uploads: where it goes ([x, y] for a part), its width and its pixels.
    let upload = |app: &mut VectorcraftApp| -> Option<(Option<[usize; 2]>, usize, Vec<egui::Color32>)> {
        let (_, mut delta) = frame_output(app, &ctx);
        let art = app.canvas.texture.as_ref()?.id();
        let up = delta.set.iter().find(|e| e.0.eq(&art)).and_then(|e| e.1.last()).map(|d| {
            let egui::ImageData::Color(img) = &d.image;
            (d.pos, img.width(), img.pixels.clone())
        });
        delta.clear();
        up
    };
    upload(&mut app).expect("the first frame");
    let v = app.view_mut().unwrap();
    (v.zoom, v.center) = (1.0, Point::new(120.0, 150.0));
    let (_, width, srgb) = upload(&mut app).expect("the view at 100%");
    let key = app.canvas.key.clone().unwrap();
    assert!(key.backdrop.is_none(), "no display profile: nothing to blend");
    let p3 = crate::display_profile::tests::p3();
    let t = vectorcraft_color::cms::display::DisplayTransform::from_icc(&p3).unwrap();
    app.display_profile.report(Reading { display: Some("DP-1".into()), found: Found::Profile { icc: p3, from: "test".into() } });
    let (_, _, shown) = upload(&mut app).expect("drawn again, converted");
    let key = app.canvas.key.clone().unwrap();
    assert!(key.backdrop.is_some());
    let white = t.apply8([255; 3]);
    // What the canvas shows of pixel `c` over the converted paper `under`, as egui blends it.
    let over = |c: egui::Color32, under: [u8; 3]| {
        let k = 255 - u32::from(c.a());
        [0, 1, 2].map(|i| u32::from(c.to_array()[i]) + (u32::from(under[i]) * k + 127) / 255)
    };
    let plain = |s: egui::Color32| {
        let mut px = s.to_array();
        t.apply_premultiplied(&mut px);
        egui::Color32::from_rgba_premultiplied(px[0], px[1], px[2], px[3])
    };
    let (view_w, view_h) = (key.w, key.h);
    let (_, _, view) = frame_view(
        None,
        (view_w, view_h),
        &View { zoom: key.zoom, center: Point::new(key.cx, key.cy), rotation: key.rot, ..View::default() },
        key.ppp,
    );
    let to_doc = view.inverse();
    let (mut blended, mut beside) = (0, 0);
    for (i, (s, c)) in srgb.iter().zip(&shown).enumerate() {
        let p = to_doc * Point::new((i % width) as f64 + 0.5, (i / width) as f64 + 0.5);
        if s.a() == 0 || s.a() == 255 {
            assert_eq!(*c, plain(*s), "opaque and empty pixels convert as before, at {p:?}");
        } else if (1.0..399.0).contains(&p.x) && (1.0..299.0).contains(&p.y) {
            let k = 255 - u32::from(s.a());
            let mix = [0, 1, 2].map(|j| (u32::from(s.to_array()[j]) + (255 * k + 127) / 255).min(255) as u8);
            let want = t.apply8(mix);
            let got = over(*c, white);
            assert!((0..3).all(|j| got[j].abs_diff(u32::from(want[j])) <= 1), "at {p:?}: {got:?} shown, {want:?} wanted");
            blended += 1;
        } else if p.x < -1.0 {
            assert_eq!(*c, plain(*s), "over the pasteboard the art converts alone, at {p:?}");
            beside += 1;
        }
    }
    assert!(blended > 1000 && beside > 500, "{blended} pixels over the paper, {beside} over the pasteboard");
    // The test tells the two apart: the fill converted alone, over the converted paper, shows another color.
    let at = view * Point::new(200.0, 150.0);
    let at = at.y as usize * width + at.x as usize;
    let (s, c) = (srgb[at], shown[at]);
    assert!((50..=205).contains(&s.a()), "the 50% fill at the center: {s:?}");
    let alone = over(plain(s), white);
    assert!((0..3).any(|j| alone[j].abs_diff(over(c, white)[j]) > 2), "{alone:?} and {:?}", over(c, white));
    // The render worker's frame of the same key is the same.
    let opts = app.canvas.shown.as_ref().unwrap().1.clone();
    let doc = app.session.active().unwrap().doc.clone();
    let display = app.display_profile.transform(true);
    let job = |key: &CacheKey, doc: &std::sync::Arc<vectorcraft_doc::Document>| crate::render_worker::Job {
        key: key.clone(),
        doc: doc.clone(),
        w: view_w,
        h: view_h,
        view,
        opts: opts.clone(),
        scope: vectorcraft_engine::file_access::ScopeHandle::capture(),
        effects: Default::default(),
        may_copy: false,
        may_keep: false,
        display: display.clone(),
    };
    let worker = crate::canvas::render_job(&mut vectorcraft_render::Renderer::new(), &mut None, &job(&key, &doc)).img;
    let near = |a: &[u8], b: egui::Color32| a.iter().zip(b.to_array()).all(|(x, y)| x.abs_diff(y) <= 1);
    assert!(worker.pixels.as_chunks::<4>().0.iter().zip(&shown).all(|(a, b)| near(a, *b)), "the worker's frame matches");
    // An edit renders its part again, blended too.
    app.run("paint.setFill", json!({"ids": [half], "color": "#0000ff"})).unwrap();
    let (pos, part_w, part) = upload(&mut app).expect("the edit shows");
    let [x0, y0] = pos.expect("only its part");
    let key = app.canvas.key.clone().unwrap();
    let doc = app.session.active().unwrap().doc.clone();
    let whole = crate::canvas::render_job(&mut vectorcraft_render::Renderer::new(), &mut None, &job(&key, &doc)).img;
    let whole = whole.pixels.as_chunks::<4>().0;
    for (i, c) in part.iter().enumerate() {
        let at = (y0 + i / part_w) * view_w as usize + x0 + i % part_w;
        assert!(near(&whole[at], *c), "the part matches the whole frame at {at}: {c:?}, {:?}", whole[at]);
    }
    assert!(part.iter().filter(|c| c.a() != 0 && c.a() != 255).count() > 1000, "the blue fill is in the part");
    // The transparency grid replaces the paper under the art: drawn again.
    app.run("view.transparencyGrid", json!({"on": true})).unwrap();
    assert!(upload(&mut app).is_some_and(|(pos, ..)| pos.is_none()), "the grid renders the art again");
    assert!(app.canvas.key.as_ref().and_then(|k| k.backdrop.as_ref()).is_some_and(|b| b.grid.is_some()));
    // Outline: the white canvas is the interface's, so the art converts alone.
    app.run("view.outline", json!({})).unwrap();
    upload(&mut app).expect("Outline renders again");
    assert!(app.canvas.key.as_ref().is_some_and(|k| k.outline && k.backdrop.is_none()));
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
        {
            let __dyn: image::DynamicImage = img.into();
            let (__w, __h) = (__dyn.width(), __dyn.height());
            let (__layout, __sample, __raw) = match __dyn {
                image::DynamicImage::ImageLuma8(b) => (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8, b.into_raw()),
                image::DynamicImage::ImageLumaA8(b) => (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8, b.into_raw()),
                image::DynamicImage::ImageRgb8(b) => (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8, b.into_raw()),
                image::DynamicImage::ImageRgba8(b) => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, b.into_raw()),
                image::DynamicImage::ImageLuma16(b) => (
                    craft_image_codecs::ChannelLayout::Gray,
                    craft_image_codecs::SampleType::U16,
                    b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
                ),
                image::DynamicImage::ImageLumaA16(b) => (
                    craft_image_codecs::ChannelLayout::GrayA,
                    craft_image_codecs::SampleType::U16,
                    b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
                ),
                image::DynamicImage::ImageRgb16(b) => (
                    craft_image_codecs::ChannelLayout::Rgb,
                    craft_image_codecs::SampleType::U16,
                    b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
                ),
                image::DynamicImage::ImageRgba16(b) => (
                    craft_image_codecs::ChannelLayout::Rgba,
                    craft_image_codecs::SampleType::U16,
                    b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
                ),
                other => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, other.into_rgba8().into_raw()),
            };
            let __cimg = craft_image_codecs::Image::from_raw(__w, __h, __layout, __sample, __raw).unwrap();
            craft_image_codecs::encode(&__cimg, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default())
                .map(|__b| png.extend_from_slice(&__b))
        }
        .unwrap();
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
    assert_eq!(app.ui.status, "2 images couldn't be decoded and show as crossed boxes");
    app.status("");
    app.session.doc_mut().unwrap().revision += 1;
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.ui.status, "", "said once");
}

/// The plug-in effects a document's first view leaves out past its budget are reported in the
/// status bar once, in the singular or the plural, and Effect › Refresh Deferred Effects is
/// enabled until it has run them.
#[test]
fn the_first_view_reports_the_plug_in_effects_it_deferred() {
    use std::sync::atomic::{AtomicU64, Ordering};
    use vectorcraft_plugins::wat as tpl;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.canvas.worker_started = true; // render on this thread
    let module = tpl::module(&tpl::manifest("test.ui.deferred", "effect", "{}"), tpl::ECHO, "", 1);
    app.session.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&wat::parse_str(module).unwrap())})).unwrap();
    // Each reading of the clock is 3 s later: the first run's result comes back after the budget,
    // and no later run starts.
    let step = std::sync::Arc::new(AtomicU64::new(3_000));
    let ctx = egui::Context::default();
    for (count, said) in [
        (2, "Opened with 2 plug-in effects deferred. Effect › Refresh Deferred Effects runs them."),
        (1, "Opened with 1 plug-in effect deferred. Effect › Refresh Deferred Effects runs it."),
    ] {
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        // Art the plug-in has not been given before: a result it gave is kept and drawn at once.
        for i in 0..count {
            let x = 31.0 + 97.0 * f64::from(i) + 13.0 * f64::from(count);
            let id = app.session.execute("shape.rectangle", &json!({"x": x, "y": 47, "width": 23, "height": 29})).unwrap()["id"].clone();
            app.session.execute("effect.apply", &json!({"ids": [id], "effect": "plugin.test.ui.deferred"})).unwrap();
        }
        app.session.execute("select.none", &json!({})).unwrap();
        let (now, by) = (std::sync::Arc::new(AtomicU64::new(0)), step.clone());
        let clock: vectorcraft_plugins::deferred::Clock =
            std::sync::Arc::new(move || Some(now.fetch_add(by.load(Ordering::Relaxed), Ordering::Relaxed)));
        app.session.doc_mut().unwrap().plugin_effects = vectorcraft_plugins::deferred::Deferrals::with_clock(clock);
        step.store(3_000, Ordering::Relaxed);
        app.status("");
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.ui.status, said);
        assert!(crate::menus::enabled(&app, "effect.refreshDeferred"));
        app.status("");
        app.session.doc_mut().unwrap().revision += 1;
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.ui.status, "", "said once");
        step.store(0, Ordering::Relaxed);
        app.session.execute("effect.refreshDeferred", &json!({})).unwrap();
        assert!(!crate::menus::enabled(&app, "effect.refreshDeferred"), "nothing is left deferred");
    }
}

/// The render worker's side of a controlled worker: the test thread renders the jobs the canvas
/// queues, with [`crate::render_worker::render`] and the worker's own renderer and last frame.
struct WorkerSide {
    jobs: std::sync::mpsc::Receiver<crate::render_worker::Job>,
    done: std::sync::mpsc::Sender<crate::render_worker::Done>,
    renderer: vectorcraft_render::Renderer,
    kept: Option<crate::canvas::KeptFrame>,
}

impl WorkerSide {
    /// A controlled worker for the canvas, and its side.
    fn start() -> (crate::render_worker::Worker, Self) {
        let (worker, jobs, done) = crate::render_worker::Worker::test_channel();
        (worker, Self { jobs, done, renderer: vectorcraft_render::Renderer::new(), kept: None })
    }

    /// Render the newest job queued, as the worker thread does, and send its frame with a time of
    /// 50 ms: the canvas then keeps sending the document's frames to the worker. False when no job
    /// is queued.
    fn render(&mut self) -> bool {
        let Some(job) = self.jobs.try_iter().last() else { return false };
        let done = crate::render_worker::render(&mut self.renderer, &mut self.kept, job);
        self.done.send(crate::render_worker::Done { ms: 50.0, ..done }).is_ok()
    }
}

/// A drag of an object with raster effects (#1257): the drag's frames render while
/// interacting and draw the object's drop shadow and filtered content from the rasters of the
/// frame before; the frame after the drag renders without `interacting`, on the render worker,
/// and draws what a fresh renderer draws. The worker is a controlled one ([`WorkerSide`]).
#[test]
fn dragging_an_object_with_raster_effects_draws_its_rasters_moved() {
    keeping_color_settings(dragging_with_raster_effects);
}

fn dragging_with_raster_effects() {
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
    let (worker, mut side) = WorkerSide::start();
    app.canvas.worker = Some(worker);
    app.session.pointer(&PointerEvent::new(PointerKind::Up, 181.9, 131.3), view).unwrap();
    assert!(upload(&mut app).is_none(), "not rendered on this thread");
    assert!(app.canvas.worker.as_ref().and_then(|w| w.pending.as_ref()).is_some_and(|k| !k.interacting), "sent to the worker");
    assert!(side.render(), "the worker renders the frame");
    let after = upload(&mut app).expect("the worker's frame arrives");
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
    keeping_color_settings(a_drag_at_a_lower_resolution);
}

fn a_drag_at_a_lower_resolution() {
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
/// frames of a heavy document render. The worker is a controlled one ([`WorkerSide`]).
#[test]
fn a_drag_over_a_wide_blur_draws_the_blur_from_the_still_frame() {
    keeping_color_settings(a_drag_over_a_wide_blur);
}

fn a_drag_over_a_wide_blur() {
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
        // One headless frame with `events`, then, with a worker, its frames rendered and shown
        // until none is pending.
        let run = |app: &mut VectorcraftApp, side: &mut Option<WorkerSide>, events: Vec<egui::Event>| {
            let frame = |app: &mut VectorcraftApp, events| {
                let raw =
                    egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 870.0))), events, ..Default::default() };
                let mut out = ctx.run_ui(raw, |ui| show(app, ui));
                out.textures_delta.clear();
            };
            frame(app, events);
            for _ in 0..8 {
                if !side.as_mut().is_some_and(WorkerSide::render) {
                    break;
                }
                frame(app, vec![]);
            }
            assert!(app.canvas.worker.as_ref().is_none_or(|w| w.pending.is_none()), "the worker's frame is shown");
        };
        let moved = |app: &VectorcraftApp| if on_worker { app.canvas.moved } else { app.canvas.renderer.stats.moved > 0 };
        let mut side = None;
        run(&mut app, &mut side, vec![]);
        if let Some(vm) = app.view_mut() {
            (vm.zoom, vm.center) = (2.0, Point::new(290.0, 410.0));
        }
        if on_worker {
            let (worker, worker_side) = WorkerSide::start();
            app.canvas.worker = Some(worker);
            app.canvas.last_ms = 50.0;
            side = Some(worker_side);
        }
        run(&mut app, &mut side, vec![]);
        assert_eq!(app.canvas.from_worker, on_worker);
        assert!(!moved(&app), "a still frame, on the worker: {on_worker}");
        let at = app.canvas_rect.unwrap().center();
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        run(&mut app, &mut side, vec![egui::Event::PointerMoved(at)]);
        run(&mut app, &mut side, vec![button(at, true)]);
        for step in [1.0, 2.0, 3.0] {
            run(&mut app, &mut side, vec![egui::Event::PointerMoved(at + vec2(13.0, 7.0) * step)]);
            assert!(app.canvas.key.as_ref().is_some_and(|k| k.interacting && k.dragging), "step {step}, on the worker: {on_worker}");
            assert_eq!(app.canvas.from_worker, on_worker, "step {step}");
            assert!(!moved(&app), "the blur drawn from the still frame's raster, step {step}, on the worker: {on_worker}");
        }
        run(&mut app, &mut side, vec![button(at + vec2(39.0, 21.0), false)]);
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
    keeping_color_settings(a_drag_back_to_its_start);
}

fn a_drag_back_to_its_start() {
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

/// Cache metadata, worker requests and view transforms use the renderer's bounded raster size,
/// on a display that reports a larger texture side than that.
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
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(pos2(0.0, 0.0), size)),
            max_texture_side: Some(1 << 20),
            ..Default::default()
        };
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
    keeping_color_settings(cached_frames_keep_their_scale);
}

fn cached_frames_keep_their_scale() {
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
        completed.send(crate::render_worker::Done { key: job.key, doc: job.doc, opts: job.opts, img, ms: 1.0, moved: 0, reused: false }).unwrap();
        frame(&mut app, size, new_ppp);
        assert_eq!(app.canvas.key.as_ref(), Some(&expected), "the worker's frame arrived");
    }
}

/// When the view turns while the worker renders the frame at the new angle (Reset Rotate View on
/// a heavy document), the canvas draws the last frame turned until that frame arrives: each
/// corner goes where the new view puts the document point that the corner showed. The new frame
/// is then drawn unturned. In Pixel Preview the last frame's pixels stay over the document rect
/// they cover. The worker is a controlled one ([`WorkerSide`]).
#[test]
fn a_turned_view_draws_the_last_frame_turned_until_its_own_arrives() {
    keeping_color_settings(a_turned_view);
}

fn a_turned_view() {
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
    let (worker, mut side) = WorkerSide::start();
    app.canvas.worker = Some(worker);
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
    assert!(side.render(), "the worker renders the frame at 0°");
    shapes(&mut app, &ctx);
    assert!(app.canvas.key.as_ref().is_some_and(|k| k.rot == 0.0), "the worker's frame arrives");
    let b = art(&mut app).expect("the worker's frame").calc_bounds();
    assert!((b.min - rect.min).length() < 0.01 && (b.max - rect.max).length() < 0.01, "{b:?} over {rect:?}");
    // In Pixel Preview the last frame's pixels are drawn over the document rect they cover.
    app.canvas.worker = None;
    app.ui.view.pixel_preview = true;
    app.view_mut().unwrap().zoom = 8.0;
    shapes(&mut app, &ctx);
    let [x0, y0, ..] = app.canvas.key.as_ref().and_then(|k| k.pixel).expect("the pixels rendered");
    let (worker, _side) = WorkerSide::start();
    app.canvas.worker = Some(worker);
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
    keeping_color_settings(a_pan_fills_the_area);
}

fn a_pan_fills_the_area() {
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
    assert!(!fill_job.may_copy && !fill_job.may_keep, "the fill worker copies nothing and holds no frame");
    assert!(app.canvas.fill.is_none(), "nothing to draw until the fill arrives");
    // The fill arrives; the next frame draws it, and asks for no other fill.
    let img = app.canvas.renderer.render(&fill_job.doc, fill_job.w, fill_job.h, fill_job.view, &fill_job.opts);
    let (doc, opts) = (fill_job.doc, fill_job.opts);
    fill_completed.send(crate::render_worker::Done { key: fill_job.key, doc, opts, img, ms: 5.0, moved: 0, reused: false }).unwrap();
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
    fill_completed.send(crate::render_worker::Done { key: next.key, doc: next.doc, opts: next.opts, img, ms: 5.0, moved: 0, reused: false }).unwrap();
    // The worker's frame arrives: it covers the canvas, and the fill is no longer drawn or kept.
    let img = app.canvas.renderer.render(&job.doc, job.w, job.h, job.view, &job.opts);
    let expected = job.key.clone();
    completed.send(crate::render_worker::Done { key: job.key, doc: job.doc, opts: job.opts, img, ms: 50.0, moved: 0, reused: false }).unwrap();
    let (meshes, _) = frame(&mut app);
    assert_eq!(app.canvas.key.as_ref(), Some(&expected), "the worker's frame is shown");
    assert!(app.canvas.fill.is_none() && meshes.iter().all(|m| m.texture_id != fill_tex), "the fill is dropped");
    assert!(app.canvas.fill_worker.as_ref().unwrap().pending.is_none(), "the late fill was taken");
    // A pan with Outline turned on: the frame shown's options are not the ones asked for, and
    // no fill is asked for.
    app.view_mut().unwrap().outline = true;
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
/// still for STILL_SECS. An edit while the view moves draws the part it changed into the frame
/// shown, and one that changes most of the frame renders it whole at once. With a worker's build settings the same frames render at once, as
/// before.
#[test]
fn without_a_worker_a_heavy_canvas_is_reprojected_until_the_view_is_still() {
    keeping_color_settings(without_a_worker_a_heavy_canvas);
}

fn without_a_worker_a_heavy_canvas() {
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
    // An edit while the view moves shows at once: the part it changed is drawn into the frame
    // shown, at the view that frame was rendered for, and the frame stays drawn reprojected until
    // the view is still (#8688).
    app.canvas.last_ms = 50.0;
    app.view_mut().unwrap().center.y += 10.0;
    frame(&mut app);
    assert_eq!(app.canvas.key.as_ref(), Some(&now), "waiting");
    app.session.execute("shape.rectangle", &json!({"x": 110, "y": 60, "width": 20, "height": 20})).unwrap();
    let render_ms = app.canvas.last_ms;
    let (b, delay) = frame(&mut app);
    let k = app.canvas.key.clone().unwrap();
    assert_eq!(k.revision, app.session.active().unwrap().revision, "the edit is drawn");
    assert_eq!((k.cx, k.cy, k.zoom), (now.cx, now.cy, now.zoom), "at the view of the frame shown");
    assert_eq!(app.canvas.last_ms, render_ms, "the frame was not rendered whole");
    let (old, new) = (Xf::new(rect, &View { center: Point::new(now.cx, now.cy), ..*app.view().unwrap() }), Xf::new(rect, app.view().unwrap()));
    let (a, z) = (new.to_screen(old.to_doc(rect.min)), new.to_screen(old.to_doc(rect.max)));
    assert!((b.min - a).length() < 0.01 && (b.max - z).length() < 0.01, "{b:?}: the frame shown at {a:?} to {z:?}");
    assert!(delay <= std::time::Duration::from_secs_f64(STILL_SECS), "a frame within STILL_SECS: {delay:?}");
    let mut frames = 0;
    while app.canvas.key.as_ref() == Some(&k) && frames < 60 {
        frames += 1;
        frame(&mut app);
    }
    let k = app.canvas.key.clone().unwrap();
    assert_eq!((k.revision, k.cy), (app.session.active().unwrap().revision, app.view().unwrap().center.y), "rendered once still");
    // An edit that changes more than half of the frame renders whole at once, view and all.
    app.canvas.last_ms = 50.0;
    app.view_mut().unwrap().center.y += 10.0;
    frame(&mut app);
    app.session.execute("shape.rectangle", &json!({"x": -1000, "y": -1000, "width": 3000, "height": 3000})).unwrap();
    frame(&mut app);
    let k = app.canvas.key.clone().unwrap();
    assert_eq!((k.revision, k.cy), (app.session.active().unwrap().revision, app.view().unwrap().center.y), "rendered whole after the edit");
    // With a worker's build settings and no worker (VECTORCRAFT_SYNC_RENDER), a heavy document
    // renders every change of view at once.
    app.canvas.waits_for_still = false;
    app.canvas.last_ms = 50.0;
    app.view_mut().unwrap().center.x += 10.0;
    frame(&mut app);
    assert_eq!(app.canvas.key.as_ref().map(|k| k.cx), Some(app.view().unwrap().center.x), "rendered at once");
}

/// With the web build's setting (`waits_for_still`, no render worker), a heavy document's view
/// that stops moving while a drag goes on editing the document renders whole once the view has
/// been still for STILL_SECS: each edit is drawn into the frame shown and does not start the wait
/// again.
#[test]
fn without_a_worker_edits_while_the_view_is_still_do_not_hold_off_the_render() {
    keeping_color_settings(edits_while_the_view_is_still);
}

fn edits_while_the_view_is_still() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 100, "y": 50, "width": 200, "height": 200})).unwrap();
    app.canvas.worker_started = true; // no worker, as in the web build
    app.canvas.waits_for_still = true;
    let ctx = egui::Context::default();
    let frame = |app: &mut VectorcraftApp| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
    };
    frame(&mut app); // the first frame fits the view to the artboard
    frame(&mut app);
    app.canvas.last_ms = 50.0;
    // The view moves once (a drag at an edge of the canvas scrolls it) and then stays still.
    app.view_mut().unwrap().center.y += 10.0;
    frame(&mut app);
    let shown = app.canvas.key.clone().unwrap();
    assert_ne!(shown.cy, app.view().unwrap().center.y, "waiting");
    // The drag edits the document on every frame, at 60 fps (egui's input time without a clock).
    let mut frames = 0;
    while app.canvas.key.as_ref().is_none_or(|k| k.cy != app.view().unwrap().center.y) && frames < 60 {
        frames += 1;
        let x = 110 + (frames % 10) * 5;
        app.session.execute("shape.rectangle", &json!({"x": x, "y": 60, "width": 4, "height": 4})).unwrap();
        frame(&mut app);
        let k = app.canvas.key.as_ref().unwrap();
        assert_eq!(k.revision, app.session.active().unwrap().revision, "each edit is drawn");
    }
    assert!((11..=13).contains(&frames), "rendered whole about STILL_SECS after the view stopped: {frames} frames");
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

/// A bitmap made in the background for a file that only another document holds leaves this
/// document's canvas key as it is. Without a render worker (the web build's setting), a pan of a
/// heavy canvas keeps drawing the frame shown until the view is still. A light canvas renders the
/// pan at once, and its key differs from the frame shown only in the view: the pan fill and the
/// worker's pixel copy ask for that.
#[test]
fn a_bitmap_made_for_another_documents_file_leaves_a_pan_waiting() {
    keeping_color_settings(a_bitmap_for_another_documents_file);
}

fn a_bitmap_for_another_documents_file() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 100, "y": 50, "width": 200, "height": 200})).unwrap();
    app.canvas.worker_started = true; // no worker, as in the web build
    app.canvas.waits_for_still = true;
    // Another window's document places a file, which its canvas asks a bitmap of.
    let mut other = VectorcraftApp::new(Session::new(), Default::default());
    other.canvas.worker_started = true; // render on this thread
    let mut src = Session::new();
    src.execute("file.new", &json!({"width": 70, "height": 40})).unwrap();
    src.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 70, "height": 40})).unwrap();
    let dir = std::env::temp_dir().join(format!("vectorcraft-placed-pan-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("badge.vectorcraft");
    std::fs::write(&path, vectorcraft_format::save_file(&src.active().unwrap().doc)).unwrap();
    other.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    other.session.execute("file.place", &json!({"path": path.to_string_lossy(), "at": [100, 100]})).unwrap();
    let ctx = egui::Context::default();
    let frame = |app: &mut VectorcraftApp| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
    };
    frame(&mut app); // the first frame fits the view to the artboard
    frame(&mut app);
    app.canvas.last_ms = 1.0;
    app.view_mut().unwrap().center.x += 10.0;
    frame(&mut app);
    let shown = app.canvas.key.clone().unwrap();
    // The other canvas asks for the bitmap, and it is made: the generation changes before the
    // bitmap stops being pending.
    let generation = vectorcraft_render::placed_document::generation();
    shapes(&mut other, &egui::Context::default());
    while vectorcraft_render::placed_document::busy() {
        std::thread::yield_now();
    }
    assert_ne!(vectorcraft_render::placed_document::generation(), generation, "a bitmap was made");
    app.canvas.last_ms = 50.0;
    app.view_mut().unwrap().center.x += 10.0;
    frame(&mut app);
    assert_eq!(app.canvas.key.as_ref(), Some(&shown), "a heavy canvas is not rendered while the view moves");
    app.canvas.last_ms = 1.0;
    app.view_mut().unwrap().center.x += 10.0;
    frame(&mut app);
    let next = app.canvas.key.clone().unwrap();
    assert_eq!(next.cx, app.view().unwrap().center.x, "a light canvas renders at once");
    assert!(super::cursor::only_view_differs(&shown, &next), "only the view differs: {shown:?} {next:?}");
    std::fs::remove_dir_all(&dir).ok();
}

/// A document's [`CacheKey::placed`] moves to the current placed-document bitmap generation only
/// when a bitmap of a file it shows in a placed document was made, or when the files made are no
/// longer known and it shows a placed document. A bitmap of another document's file, or of a file
/// it keeps only for Undo, leaves it, so a pan of a heavy
/// document keeps waiting for the view to stop. Each change keeps the value before it and the
/// last generation that value was checked at. A closed document's value is dropped.
#[test]
fn a_documents_placed_key_moves_only_for_bitmaps_of_its_own_files() {
    let mut session = Session::new();
    session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    let plain = session.active().unwrap().uid;
    let mut src = Session::new();
    src.execute("file.new", &json!({"width": 60, "height": 30})).unwrap();
    src.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 60, "height": 30})).unwrap();
    let dir = std::env::temp_dir().join(format!("vectorcraft-placed-key-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("card.vectorcraft");
    std::fs::write(&path, vectorcraft_format::save_file(&src.active().unwrap().doc)).unwrap();
    session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let r = session.execute("file.place", &json!({"path": path.to_string_lossy(), "at": [60, 50]})).unwrap();
    let st = session.active().unwrap();
    let placing = st.uid;
    let file = match &st.doc.node(vectorcraft_doc::NodeId(r["ids"][0].as_u64().unwrap())).unwrap().kind {
        vectorcraft_doc::NodeKind::PlacedDocument(p) => p.key.clone(),
        other => panic!("a placed document: {other:?}"),
    };
    type Made<'a> = &'a dyn Fn(u64, u64) -> Option<std::collections::HashSet<String>>;
    fn key(keys: &mut std::collections::HashMap<u64, crate::PlacedKey>, session: &Session, uid: u64, now: u64, made: Made<'_>) -> u64 {
        let st = session.documents().iter().find(|d| d.uid == uid).unwrap();
        placed_key_at(keys, session, st, now, made)
    }
    let files = |names: &[&str]| Some(names.iter().map(|n| n.to_string()).collect::<std::collections::HashSet<_>>());
    let unasked = |_: u64, _: u64| -> Option<std::collections::HashSet<String>> { panic!("no bitmaps to look up") };
    let mut keys = std::collections::HashMap::new();
    assert_eq!(key(&mut keys, &session, placing, 5, &unasked), 5, "checked for the first time");
    assert_eq!(key(&mut keys, &session, placing, 5, &unasked), 5, "no bitmap made since");
    let other = |from: u64, to: u64| {
        assert_eq!((from, to), (5, 6), "the bitmaps made since the last check");
        files(&["another document's file"])
    };
    assert_eq!(key(&mut keys, &session, placing, 6, &other), 5, "another document's file");
    assert_eq!(key(&mut keys, &session, placing, 7, &|_, _| files(&["another document's file", file.as_str()])), 7, "its own file");
    assert_eq!(keys[&placing], crate::PlacedKey { checked: 7, key: 7, before: (5, 6) });
    assert_eq!(key(&mut keys, &session, placing, 8, &|_, _| None), 8, "files no longer known");
    assert_eq!(keys[&placing].before, (7, 7));
    assert_eq!(key(&mut keys, &session, plain, 8, &unasked), 8, "the other document, checked for the first time");
    assert_eq!(key(&mut keys, &session, plain, 9, &|_, _| None), 8, "no images: files no longer known");
    assert_eq!(key(&mut keys, &session, plain, 10, &|_, _| files(&[file.as_str()])), 8, "no images: a file another document holds");
    // The same document holding an image and showing no placed document.
    let mut raster = session.documents().iter().find(|d| d.uid == plain).unwrap().clone();
    let blob = vectorcraft_doc::ImageBlob { mime: "image/png".into(), bytes: Default::default(), proxy: None };
    std::sync::Arc::make_mut(&mut raster.doc).images.insert("an image".into(), blob);
    assert_eq!(placed_key_at(&mut keys, &session, &raster, 11, |_, _| None), 8, "an image and no placed document: files no longer known");
    assert_eq!(keys.len(), 2);
    let index = session.documents().iter().position(|d| d.uid == plain).unwrap();
    session.execute("file.close", &json!({"index": index})).unwrap();
    assert_eq!(key(&mut keys, &session, placing, 12, &|_, _| files(&[])), 8, "no bitmaps of its files");
    assert_eq!(keys.keys().copied().collect::<Vec<_>>(), [placing], "the closed document's value is dropped");
    // The placed document deleted: its file stays in the images for Undo, and no longer shows.
    assert_eq!(session.active().unwrap().uid, placing);
    session.execute("edit.clear", &json!({"ids": [r["ids"][0]]})).unwrap();
    assert!(session.active().unwrap().doc.images.contains_key(&file), "kept for Undo");
    assert_eq!(key(&mut keys, &session, placing, 13, &|_, _| files(&[file.as_str()])), 8, "a file kept only for Undo");
    std::fs::remove_dir_all(&dir).ok();
}

/// The canvas key takes the document's value from [`placed_key`], which a bitmap made for a test
/// running beside this one does not move: this document holds no images.
#[test]
fn the_canvas_key_takes_the_documents_placed_value() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 100, "y": 50, "width": 200, "height": 200})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    let uid = app.session.active().unwrap().uid;
    let generation = vectorcraft_render::placed_document::generation();
    let marked = generation.wrapping_add(1000);
    app.canvas.placed_keys.insert(uid, crate::PlacedKey { checked: generation, key: marked, before: (marked, generation) });
    shapes(&mut app, &egui::Context::default());
    assert_eq!(app.canvas.key.as_ref().map(|k| k.placed), Some(marked));
}

/// A frame shown at one [`CacheKey::placed`] value and drawn again at the next looks at the
/// bitmaps made since the last check before the change: those made earlier were of other
/// documents' files, and past 256 bitmaps they are no longer known ([`placed_since`]).
#[test]
fn a_frame_drawn_again_after_one_change_of_its_placed_value_looks_at_the_bitmaps_since_the_last_check() {
    let entry = crate::PlacedKey { checked: 400, key: 400, before: (5, 399) };
    assert_eq!(placed_since(Some(&entry), 5, 400), 399, "the value changed once since the frame shown");
    assert_eq!(placed_since(Some(&entry), 3, 400), 3, "it changed more than once");
    assert_eq!(placed_since(Some(&entry), 5, 401), 5, "a value the document no longer has");
    assert_eq!(placed_since(None, 5, 400), 5, "a document not checked");
}

/// A frame drawn again at a new [`CacheKey::placed`] value draws again where the document shows
/// the files whose bitmaps were made after [`placed_since`]. After one change of the value, the
/// bitmaps made before the last check of the old value are not looked up: past 256 bitmaps they
/// are no longer known, and the whole frame would render. At the same value nothing is looked up.
#[test]
fn a_frame_drawn_again_at_a_new_placed_value_draws_where_the_bitmaps_since_the_last_check_show() {
    let mut session = Session::new();
    let mut src = Session::new();
    src.execute("file.new", &json!({"width": 60, "height": 30})).unwrap();
    src.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 60, "height": 30})).unwrap();
    let dir = std::env::temp_dir().join(format!("vectorcraft-placed-changes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("card.vectorcraft");
    std::fs::write(&path, vectorcraft_format::save_file(&src.active().unwrap().doc)).unwrap();
    session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let r = session.execute("file.place", &json!({"path": path.to_string_lossy(), "at": [60, 50]})).unwrap();
    session.execute("shape.rectangle", &json!({"x": 250, "y": 200, "width": 40, "height": 30})).unwrap();
    let doc = &session.active().unwrap().doc;
    let placed = doc.node(vectorcraft_doc::NodeId(r["ids"][0].as_u64().unwrap())).unwrap();
    let file = match &placed.kind {
        vectorcraft_doc::NodeKind::PlacedDocument(p) => p.key.clone(),
        other => panic!("a placed document: {other:?}"),
    };
    let bounds = placed.geometric_bounds().unwrap();
    let entry = crate::PlacedKey { checked: 400, key: 400, before: (5, 399) };
    // Only the bitmaps made after generation 399 are known: one of the placed file.
    let made = |from: u64, to: u64| (from >= 399 && to == 400).then(|| std::collections::HashSet::from([file.clone()]));
    let unasked = |_: u64, _: u64| -> Option<std::collections::HashSet<String>> { panic!("no bitmaps to look up") };
    assert_eq!(placed_changes(Some(&entry), 400, 400, doc, unasked), Some(vectorcraft_render::Changed::Nothing), "the same value");
    match placed_changes(Some(&entry), 5, 400, doc, made) {
        Some(vectorcraft_render::Changed::Area(a)) => {
            assert!(a.x0 <= bounds.x0 && a.y0 <= bounds.y0 && a.x1 >= bounds.x1 && a.y1 >= bounds.y1, "{a:?} holds {bounds:?}");
            assert!(a.x1 < 250.0, "{a:?}: not the rectangle");
        }
        other => panic!("the placed document's part: {other:?}"),
    }
    assert_eq!(placed_changes(Some(&entry), 3, 400, doc, made), None, "the value changed more than once: no longer known");
    let none = |_: u64, _: u64| Some(std::collections::HashSet::new());
    assert_eq!(placed_changes(Some(&entry), 5, 400, doc, none), Some(vectorcraft_render::Changed::Nothing), "no bitmaps since");
    std::fs::remove_dir_all(&dir).ok();
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
    app.canvas.worker_started = true; // render on this thread
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

/// The Split view's divider of a selected Image Trace object is drawn where the Selection tool
/// takes it in a zoomed, turned view. Dragging its middle across the image previews the divider,
/// commits one undo step and leaves the traced art where it was; Undo puts the divider back.
#[test]
fn trace_split_divider_drag_in_a_zoomed_rotated_view_is_one_undo_step() {
    use vectorcraft_tools::trace_split::TraceDivider;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let data = vectorcraft_format::base64_encode(&crate::tests_sysclip::blue_png());
    app.run("file.place", json!({"name": "blue.png", "dataBase64": data, "link": false, "at": [200, 150]})).unwrap();
    app.run("imageTrace.make", json!({"preset": "Default"})).unwrap();
    app.run("imageTrace.setView", json!({"view": "split"})).unwrap();
    app.select_tool("selection");
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    let divider = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        TraceDivider::of(&st.doc, &st.selection, app.view().unwrap().zoom).unwrap()
    };
    let initial = divider(&app);
    assert_eq!(initial.split, 0.5);
    let middle = initial.ends[0].midpoint(initial.ends[1]);
    {
        let v = app.view_mut().unwrap();
        (v.zoom, v.rotation, v.center, v.fitted) = (30.0, 30.0, middle, true);
    }
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let art = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        st.doc.node(initial.id).unwrap().geometric_bounds()
    };
    let before = (art(&app), app.session.active().unwrap().history.undo.len());
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    // The divider is drawn between its ends on screen.
    let ends = initial.ends.map(|q| xf.to_screen(q));
    let drawn = shapes(&mut app, &ctx);
    assert!(
        drawn.iter().any(|s| matches!(s, Shape::LineSegment { points, .. }
            if points.iter().zip(ends).all(|(a, b)| a.distance(b) < 0.5) || points.iter().rev().zip(ends).all(|(a, b)| a.distance(b) < 0.5))),
        "the divider is drawn from {:?} to {:?}",
        ends[0],
        ends[1]
    );
    // A quarter of the image's width to the right, in the image's own direction.
    let from = xf.to_screen(middle);
    let st = app.session.active().unwrap();
    let NodeKind::Image(im) = &st.doc.node(initial.id).unwrap().children().unwrap()[0].kind else { panic!("no source image") };
    let to = xf.to_screen(im.xf * vectorcraft_geom::Point::new(f64::from(im.width) * 0.75, f64::from(im.height) * 0.5));
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from), button(from, true)]);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(to)]);
    assert!((divider(&app).split - 0.75).abs() < 0.01, "the divider previews during the drag: {}", divider(&app).split);
    frame(&mut app, &ctx, vec![button(to, false)]);
    assert!((divider(&app).split - 0.75).abs() < 0.01);
    assert_eq!((art(&app), app.session.active().unwrap().history.undo.len()), (before.0, before.1 + 1), "one undo step, the art unmoved");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(divider(&app).split, 0.5);
    // Escape during a drag puts the divider back with no undo step.
    let steps = app.session.active().unwrap().history.undo.len();
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from), button(from, true)]);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(to)]);
    assert!((divider(&app).split - 0.75).abs() < 0.01);
    let escape = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    let raw =
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), events: vec![escape], ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| {
        crate::shortcuts::handle(&mut app, ui.ctx());
        show(&mut app, ui)
    });
    out.textures_delta.clear();
    assert!(!app.session.tool_busy(), "Escape ends the drag");
    assert_eq!(divider(&app).split, 0.5, "Escape puts the divider back");
    frame(&mut app, &ctx, vec![button(to, false)]);
    assert_eq!(divider(&app).split, 0.5, "Escape cancels the drag");
    assert_eq!(app.session.active().unwrap().history.undo.len(), steps, "no undo step");
    // With the bounding box hidden, the divider is not drawn.
    app.ui.view.bounding_box = false;
    let drawn = shapes(&mut app, &ctx);
    assert!(
        !drawn.iter().any(|s| matches!(s, Shape::LineSegment { points, .. } if points.iter().zip(ends).all(|(a, b)| a.distance(b) < 0.5))),
        "no divider with the bounding box hidden"
    );
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

/// A selected radial repeat draws its instances handle and its radius handle (none with the
/// bounding box hidden). In a zoomed view turned 30°, dragging the instances handle two steps down
/// adds two instances without scaling the source, and dragging the radius handle 20 pt outward
/// makes the circle 20 pt larger, as one undo step each; Escape puts a drag back.
#[test]
fn radial_repeat_handles_set_instances_and_radius_in_one_undo_step_each() {
    use vectorcraft_tools::repeat_radial::{HANDLE_RADIUS, INSTANCE_STEP, RadialHandles};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 180, "y": 40, "width": 40, "height": 40})).unwrap();
    app.run("object.repeat.radial", json!({"instances": 6, "radius": 60})).unwrap();
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    let ctx = egui::Context::default();
    // The first frame fits the artboard in the window; then zoom to 1.5 px a point and turn 30°.
    frame(&mut app, &ctx, vec![]);
    app.view_mut().unwrap().zoom = 1.5;
    app.view_mut().unwrap().rotation = 30.0;
    frame(&mut app, &ctx, vec![]);
    // Instances, radius and the source's bounds.
    let radial = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        let n = st.doc.node(st.selection.objects[0]).unwrap();
        let vectorcraft_doc::NodeKind::Repeat(spec) = &n.kind else { panic!("a repeat") };
        let vectorcraft_doc::RepeatKind::Radial { instances, radius, .. } = spec.kind else { panic!("a radial repeat") };
        (instances, radius, spec.source_bounds().unwrap())
    };
    let handles = |app: &mut VectorcraftApp| {
        let frame = app.selection_box();
        let st = app.session.active().unwrap();
        RadialHandles::of(&st.doc, &st.selection, frame, app.view().unwrap().zoom).unwrap()
    };
    let before = radial(&app);
    assert_eq!((before.0, before.1), (6, 60.0));
    let h = handles(&mut app);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let circle_at =
        |drawn: &[Shape], at: Pos2| drawn.iter().any(|s| matches!(s, Shape::Circle(c) if c.center.distance(at) < 0.01 && c.radius == HANDLE_RADIUS));
    let drawn = shapes(&mut app, &ctx);
    assert!(circle_at(&drawn, xf.to_screen(h.instances_at)), "the instances handle is drawn");
    assert!(circle_at(&drawn, xf.to_screen(h.radius_at)), "the radius handle is drawn");
    app.ui.view.bounding_box = false;
    let drawn = shapes(&mut app, &ctx);
    assert!(!circle_at(&drawn, xf.to_screen(h.instances_at)) && !circle_at(&drawn, xf.to_screen(h.radius_at)), "hidden with the bounding box");
    app.ui.view.bounding_box = true;
    frame(&mut app, &ctx, vec![]);
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let drag = |app: &mut VectorcraftApp, from: Pos2, to: Pos2| {
        frame(app, &ctx, vec![egui::Event::PointerMoved(from), button(from, true)]);
        for k in 1..=4 {
            frame(app, &ctx, vec![egui::Event::PointerMoved(from + (to - from) * k as f32 / 4.0)]);
        }
    };
    let release = |app: &mut VectorcraftApp, at: Pos2| frame(app, &ctx, vec![button(at, false)]);
    // 2.2 steps down the page (the screen's down turned 30°): two more instances.
    let to = xf.to_screen(h.instances_at + vectorcraft_geom::Vec2::new(0.0, 2.2 * INSTANCE_STEP / 1.5));
    drag(&mut app, xf.to_screen(h.instances_at), to);
    assert_eq!(radial(&app).0, 8, "the drag previews");
    release(&mut app, to);
    let after = radial(&app);
    assert_eq!((after.0, after.1), (8, 60.0));
    assert_eq!(after.2, before.2, "the source keeps its size");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(radial(&app).0, 6, "one undo step");
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(radial(&app).0, 8);
    // 20 pt outward from the center.
    frame(&mut app, &ctx, vec![]);
    let h = handles(&mut app);
    let out = (h.radius_at - h.center) / h.radius;
    let to = xf.to_screen(h.radius_at + out * 20.0);
    drag(&mut app, xf.to_screen(h.radius_at), to);
    release(&mut app, to);
    assert_eq!((radial(&app).0, radial(&app).1), (8, 80.0));
    assert_eq!(radial(&app).2, before.2, "the source keeps its size");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!((radial(&app).0, radial(&app).1), (8, 60.0), "the radius's own undo step");
    // Escape during a drag puts the radius back and leaves no undo step.
    frame(&mut app, &ctx, vec![]);
    let h = handles(&mut app);
    drag(&mut app, xf.to_screen(h.radius_at), xf.to_screen(h.radius_at + out * 30.0));
    assert_eq!(radial(&app).1, 90.0, "the drag previews");
    let escape = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
    let mut full = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events: vec![escape], ..Default::default() }, |ui| {
        crate::shortcuts::handle(&mut app, ui.ctx());
        show(&mut app, ui)
    });
    full.textures_delta.clear();
    assert_eq!(radial(&app).1, 60.0);
    release(&mut app, xf.to_screen(h.radius_at + out * 30.0));
    assert_eq!(radial(&app).1, 60.0, "the release after Escape changes nothing");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(radial(&app).0, 6, "the instances step is the one before");
}

/// A selected radial repeat draws its splitter on its circle. In a zoomed view turned 30°,
/// dragging the splitter back around the circle past two instances hides them while the other
/// four stay where they were, as one undo step; Escape puts a drag back.
#[test]
fn radial_repeat_splitter_hides_instances_in_one_undo_step() {
    use vectorcraft_geom::Affine;
    use vectorcraft_tools::repeat_radial::{HANDLE_RADIUS, RadialHandles};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 180, "y": 40, "width": 40, "height": 40})).unwrap();
    app.run("object.repeat.radial", json!({"instances": 6, "radius": 60})).unwrap();
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    app.view_mut().unwrap().zoom = 1.5;
    app.view_mut().unwrap().rotation = 30.0;
    frame(&mut app, &ctx, vec![]);
    // Instances shown, end angle and where the instances are.
    let radial = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        let n = st.doc.node(st.selection.objects[0]).unwrap();
        let vectorcraft_doc::NodeKind::Repeat(spec) = &n.kind else { panic!("a repeat") };
        let vectorcraft_doc::RepeatKind::Radial { instances, end_angle, .. } = spec.kind else { panic!("a radial repeat") };
        (instances, end_angle, spec.transforms(Some(&st.doc)))
    };
    let handles = |app: &mut VectorcraftApp| {
        let frame = app.selection_box();
        let st = app.session.active().unwrap();
        RadialHandles::of(&st.doc, &st.selection, frame, app.view().unwrap().zoom).unwrap()
    };
    // `h`'s splitter turned `deg` degrees about the center (positive: the way the instances run).
    let around =
        |h: &RadialHandles, deg: f64| h.center + (Affine::rotate(deg.to_radians()) * (h.splitter_at.unwrap() - h.center).to_point()).to_vec2();
    let before = radial(&app);
    assert_eq!((before.0, before.1, before.2.len()), (6, 360.0, 6));
    let h = handles(&mut app);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let drawn = shapes(&mut app, &ctx);
    let at = xf.to_screen(h.splitter_at.unwrap());
    assert!(
        drawn.iter().any(|s| matches!(s, Shape::Circle(c) if c.center.distance(at) < 0.01 && c.radius == HANDLE_RADIUS)),
        "the splitter is drawn"
    );
    frame(&mut app, &ctx, vec![]);
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let drag = |app: &mut VectorcraftApp, from: Pos2, to: Pos2| {
        frame(app, &ctx, vec![egui::Event::PointerMoved(from), button(from, true)]);
        for k in 1..=4 {
            frame(app, &ctx, vec![egui::Event::PointerMoved(from + (to - from) * k as f32 / 4.0)]);
        }
    };
    let release = |app: &mut VectorcraftApp, at: Pos2| frame(app, &ctx, vec![button(at, false)]);
    // Back 120° (two instances 60° apart): four shown, the rest in place.
    let to = xf.to_screen(around(&h, -120.0));
    drag(&mut app, xf.to_screen(h.splitter_at.unwrap()), to);
    assert_eq!(radial(&app).0, 4, "the drag previews");
    release(&mut app, to);
    let after = radial(&app);
    assert_eq!((after.0, after.1, after.2.len()), (4, 180.0, 4));
    for (k, (a, b)) in after.2.iter().zip(&before.2).enumerate() {
        assert!(a.as_coeffs().iter().zip(b.as_coeffs()).all(|(x, y)| (x - y).abs() < 1e-9), "instance {k} stays where it was");
    }
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!((radial(&app).0, radial(&app).1), (6, 360.0), "one undo step");
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!((radial(&app).0, radial(&app).1), (4, 180.0));
    // Escape during a drag forward puts the split back and leaves no undo step.
    frame(&mut app, &ctx, vec![]);
    let h = handles(&mut app);
    let to = xf.to_screen(around(&h, 120.0));
    drag(&mut app, xf.to_screen(h.splitter_at.unwrap()), to);
    assert_eq!((radial(&app).0, radial(&app).1), (6, 360.0), "the drag previews");
    let escape = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
    let mut full = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events: vec![escape], ..Default::default() }, |ui| {
        crate::shortcuts::handle(&mut app, ui.ctx());
        show(&mut app, ui)
    });
    full.textures_delta.clear();
    assert_eq!((radial(&app).0, radial(&app).1), (4, 180.0));
    release(&mut app, to);
    assert_eq!(radial(&app).0, 4, "the release after Escape changes nothing");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!((radial(&app).0, radial(&app).1), (6, 360.0), "the split was the step before");
}

/// While a symbol with 9-slice guides is edited, the canvas draws its four guides dotted across
/// its art, with ruler guides hidden too, and none once the mode ends. Dragging the left guide in
/// a zoomed view moves that guide alone, as one undo step.
#[test]
fn the_edited_symbols_nine_slice_guides_are_drawn_and_dragged_on_the_canvas() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
    app.run("symbol.new", json!({"name": "Box", "nineSlice": true})).unwrap();
    app.select_tool("selection");
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    app.view_mut().unwrap().zoom = 1.5;
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    // The dots drawn along the guide from document point `a` to `b`.
    let dots = |app: &mut VectorcraftApp, a: Point, b: Point| {
        let (a, b) = (xf.to_screen(a), xf.to_screen(b));
        let on = |q: Pos2| {
            let (d, len) = (b - a, (b - a).length());
            let t = (q - a).dot(d) / (len * len);
            (-0.01..=1.01).contains(&t) && (a + d * t).distance(q) < 0.01
        };
        shapes(app, &ctx).into_iter().filter(|s| matches!(s, Shape::LineSegment { points: [p, q], .. } if on(*p) && on(*q))).count()
    };
    let guides = [
        (Point::new(125.0, 100.0), Point::new(125.0, 200.0)),
        (Point::new(100.0, 125.0), Point::new(200.0, 125.0)),
        (Point::new(175.0, 100.0), Point::new(175.0, 200.0)),
        (Point::new(100.0, 175.0), Point::new(200.0, 175.0)),
    ];
    assert_eq!(dots(&mut app, guides[0].0, guides[0].1), 0, "not while the instance is only selected");
    app.run("symbol.edit", json!({})).unwrap();
    app.ui.view.guides = false;
    for (a, b) in guides {
        assert!(dots(&mut app, a, b) > 10, "a dotted guide from {a:?} to {b:?}");
    }
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let from = xf.to_screen(Point::new(125.0, 150.0));
    let to = xf.to_screen(Point::new(110.0, 160.0));
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from), button(from, true)]);
    for k in 1..=4 {
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from + (to - from) * k as f32 / 4.0)]);
    }
    frame(&mut app, &ctx, vec![button(to, false)]);
    let guides_now = |app: &VectorcraftApp| app.session.active().unwrap().doc.symbol_nine_slice("Box").unwrap();
    let g = guides_now(&app);
    assert!((g.left - 0.1).abs() < 1e-3 && (g.top, g.right, g.bottom) == (0.25, 0.25, 0.25), "{g:?}");
    assert!(app.session.active().unwrap().doc.symbol_edit.is_some(), "still editing the symbol");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(guides_now(&app).left, 0.25, "one undo step");
    app.run("edit.redo", json!({})).unwrap();
    assert!((guides_now(&app).left - 0.1).abs() < 1e-3);
    // Dragged on past the art's left edge at x = 100, the guide lies outside the art and is drawn
    // there, along the art's height.
    let (from, to) = (xf.to_screen(Point::new(110.0, 150.0)), xf.to_screen(Point::new(80.0, 150.0)));
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from), button(from, true)]);
    for k in 1..=4 {
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from + (to - from) * k as f32 / 4.0)]);
    }
    frame(&mut app, &ctx, vec![button(to, false)]);
    assert!((guides_now(&app).left + 0.2).abs() < 1e-3, "{:?}", guides_now(&app));
    assert!(dots(&mut app, Point::new(80.0, 100.0), Point::new(80.0, 200.0)) > 10, "the guide drawn outside the art");
    app.run("symbol.exitEditing", json!({})).unwrap();
    assert_eq!(dots(&mut app, Point::new(110.0, 100.0), Point::new(110.0, 200.0)), 0, "none outside the mode");
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

/// View › Pixel Preview turns Smart Guides off in the tools: a square moved to 3 pt below another
/// square's top lines up with it only while Pixel Preview is off.
#[test]
fn pixel_preview_stops_smart_guides_snapping_a_move() {
    for pixel in [false, true] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
        let id = app.session.execute("shape.rectangle", &json!({"x": 400, "y": 300, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
        if pixel {
            app.run("view.pixelPreview", json!({})).unwrap();
        }
        assert!(app.ui.view.smart_guides);
        assert_eq!(app.ui.view.pixel_preview, pixel);
        app.run("select.set", json!({"ids": [id]})).unwrap();
        app.select_tool("selection");
        let view = app.view_info();
        let mut at = |kind: PointerKind, y: f64| crate::canvas::dispatch(&mut app, &PointerEvent::new(kind, 425.0, y), view);
        at(PointerKind::Down, 325.0);
        for y in [300.0, 250.0, 200.0, 150.0, 128.0] {
            at(PointerKind::Drag, y);
        }
        at(PointerKind::Up, 128.0);
        let top = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap().y0;
        assert_eq!(top, if pixel { 103.0 } else { 100.0 }, "Pixel Preview {pixel}");
    }
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

/// One headless canvas frame → the bounding box drawn: its outline's corners on screen (the
/// closed four-point line in the layer color; `None` when it is hidden) and the number of its
/// handles (white 6 px squares, to 0.01 px: at a fractional screen position a square's corners
/// round to a size a little off 6).
fn bounding_box_drawn(app: &mut VectorcraftApp, ctx: &egui::Context) -> (Option<Vec<Pos2>>, usize) {
    let st = app.session.active().unwrap();
    let layer = st.doc.layer_color(st.selection.objects[0]);
    bounding_box_drawn_in(app, ctx, layer)
}

/// [`bounding_box_drawn`] for a box in the layer color `layer`, whatever is selected.
fn bounding_box_drawn_in(app: &mut VectorcraftApp, ctx: &egui::Context, layer: [u8; 3]) -> (Option<Vec<Pos2>>, usize) {
    let layer = egui::epaint::ColorMode::Solid(c32(layer));
    let drawn = shapes(app, ctx);
    let outline = drawn.iter().find_map(|s| match s {
        Shape::Path(ps) if ps.closed && ps.points.len() == 4 && ps.stroke.color == layer => Some(ps.points.clone()),
        _ => None,
    });
    let handle = |s: &&Shape| matches!(s, Shape::Rect(r) if (r.rect.size() - vec2(6.0, 6.0)).length() < 0.01 && r.fill == Color32::WHITE);
    (outline, drawn.iter().filter(handle).count())
}

/// The selection's bounding box corners on screen, as measured from the document.
fn box_corners(app: &mut VectorcraftApp) -> Vec<Pos2> {
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    app.selection_box().unwrap().corners().iter().map(|q| xf.to_screen(*q)).collect()
}

/// Do two outlines have the same corners, to 0.01 px?
fn same_corners(a: &[Pos2], b: &[Pos2]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(p, q)| p.distance(*q) < 0.01)
}

/// #3526: a resize by a bounding-box handle keeps the bounding box and its handles drawn at the
/// bounds the drag gives, with Real-time Drawing on and off (off, the document stays at the drag's
/// snapshot until the release).
#[test]
fn a_resize_drag_keeps_the_bounding_box_at_the_new_bounds() {
    for real_time in [true, false] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.session.execute("shape.ellipse", &json!({"x": 100, "y": 100, "width": 100, "height": 60})).unwrap();
        app.session.execute("prefs.set", &json!({"key": "realTimeDrawing", "value": real_time})).unwrap();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        app.select_tool("selection");
        let before = box_corners(&mut app);
        let (outline, handles) = bounding_box_drawn(&mut app, &ctx);
        assert!(outline.is_some_and(|o| same_corners(&o, &before)) && handles == 8, "real time {real_time}: the box at rest");
        let view = app.view_info();
        app.session.pointer(&PointerEvent::new(PointerKind::Down, 200.0, 160.0), view).unwrap();
        app.session.pointer(&PointerEvent::new(PointerKind::Drag, 263.0, 211.0), view).unwrap();
        assert!(app.session.tool_resizing(), "real time {real_time}: the handle was hit");
        let (during, handles) = bounding_box_drawn(&mut app, &ctx);
        assert_eq!(handles, 8, "real time {real_time}: the handles show during the resize");
        app.session.pointer(&PointerEvent::new(PointerKind::Up, 263.0, 211.0), view).unwrap();
        let after = box_corners(&mut app);
        assert!(!same_corners(&after, &before), "real time {real_time}: the drag resized the ellipse");
        assert!(during.as_ref().is_some_and(|o| same_corners(o, &after)), "real time {real_time}: {during:?}, resized to {after:?}");
        let (outline, handles) = bounding_box_drawn(&mut app, &ctx);
        assert!(outline.is_some_and(|o| same_corners(&o, &after)) && handles == 8, "real time {real_time}: the box after the release");
    }
}

/// With General › Show Bounding Box While Moving and Drawing on, a move of the selection keeps the
/// bounding box and its handles drawn where the drag carries the selection, with Real-time Drawing
/// on and off: a Selection-tool move, an Alt-drag copy, and a shape tool's move by the center point.
/// The box follows each step of the drag and ends where the release leaves the selection. With View
/// › Hide Bounding Box, no box shows during the move. With the preference off, the box hides during
/// the move (#712), except for a Selection-tool move with Real-time Drawing off, where the art waits
/// for the release and the box moves instead.
#[test]
fn a_move_drag_keeps_the_bounding_box_where_the_drag_carries_the_selection() {
    for (real_time, while_moving) in [(true, true), (false, true), (true, false), (false, false)] {
        for (tool, alt) in [("selection", false), ("selection", true), ("rectangle", false)] {
            let what = format!("{tool}, alt {alt}, real time {real_time}, box while moving {while_moving}");
            let mut app = VectorcraftApp::new(Session::new(), Default::default());
            app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
            app.session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 60})).unwrap();
            app.session.execute("attributes.set", &json!({"showCenter": true})).unwrap();
            app.session.execute("prefs.set", &json!({"key": "realTimeDrawing", "value": real_time})).unwrap();
            app.session.execute("prefs.set", &json!({"key": "boundingBoxWhileMovingAndDrawing", "value": while_moving})).unwrap();
            let ctx = egui::Context::default();
            frame(&mut app, &ctx, vec![]);
            app.select_tool(tool);
            let (outline, handles) = bounding_box_drawn(&mut app, &ctx);
            let before = box_corners(&mut app);
            assert!(outline.is_some_and(|o| same_corners(&o, &before)) && handles == 8, "{what}: the box at rest");
            let mods = Mods { alt, ..Mods::default() };
            let view = app.view_info();
            let at = |kind, x, y| PointerEvent::new(kind, x, y).with_mods(mods);
            app.session.pointer(&at(PointerKind::Down, 150.0, 130.0), view).unwrap();
            app.session.pointer(&at(PointerKind::Drag, 170.0, 140.0), view).unwrap();
            assert!(app.session.tool_transforming() && !app.session.tool_resizing(), "{what}: the drag moves the selection");
            let (first, handles) = bounding_box_drawn(&mut app, &ctx);
            if !while_moving && !(tool == "selection" && !real_time) {
                assert_eq!((first, handles), (None, 0), "{what}: the box hides during the move");
                app.session.pointer(&at(PointerKind::Up, 190.0, 165.0), view).unwrap();
                assert_eq!(bounding_box_drawn(&mut app, &ctx).1, 8, "{what}: back after the move");
                continue;
            }
            assert_eq!(handles, 8, "{what}: the handles show during the move");
            app.session.pointer(&at(PointerKind::Drag, 190.0, 165.0), view).unwrap();
            let (during, handles) = bounding_box_drawn(&mut app, &ctx);
            assert_eq!(handles, 8, "{what}");
            assert!(
                first.as_ref().zip(during.as_ref()).is_some_and(|(a, b)| !same_corners(a, b)),
                "{what}: the box follows the drag: {first:?}, {during:?}"
            );
            app.ui.view.bounding_box = false;
            assert_eq!(bounding_box_drawn(&mut app, &ctx), (None, 0), "{what}: View › Hide Bounding Box");
            app.ui.view.bounding_box = true;
            app.session.pointer(&at(PointerKind::Up, 190.0, 165.0), view).unwrap();
            let after = box_corners(&mut app);
            assert!(!same_corners(&after, &before), "{what}: the drag moved the rectangle");
            assert!(during.as_ref().is_some_and(|o| same_corners(o, &after)), "{what}: {during:?}, moved to {after:?}");
        }
    }
}

/// With General › Show Bounding Box While Moving and Drawing on, the Rectangle, Rounded Rectangle,
/// Ellipse, Polygon and Star tools draw the bounding box and its handles around the shape being
/// drawn, at each step of the drag, where the release leaves the new shape's box; with Real-time
/// Drawing on and off, and whether or not something was selected before (off, the document and the
/// selection stay as they were until the release). Line Segment, which shows no box, draws none,
/// and View › Hide Bounding Box hides it. With the preference off, no box shows while a shape is
/// drawn.
#[test]
fn a_shape_being_drawn_shows_its_bounding_box() {
    for (real_time, while_drawing) in [(true, true), (false, true), (true, false), (false, false)] {
        for selected in [false, true] {
            for tool in ["rectangle", "roundedRectangle", "ellipse", "polygon", "star", "lineSegment"] {
                let what = format!("{tool}, real time {real_time}, selected before {selected}, box while drawing {while_drawing}");
                let mut app = VectorcraftApp::new(Session::new(), Default::default());
                app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
                let old =
                    app.session.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 30, "height": 30})).unwrap()["id"].as_u64().unwrap();
                let layer = app.session.active().unwrap().doc.layer_color(vectorcraft_doc::NodeId(old));
                if !selected {
                    app.session.execute("select.none", &json!({})).unwrap();
                }
                app.session.execute("prefs.set", &json!({"key": "realTimeDrawing", "value": real_time})).unwrap();
                app.session.execute("prefs.set", &json!({"key": "boundingBoxWhileMovingAndDrawing", "value": while_drawing})).unwrap();
                let ctx = egui::Context::default();
                frame(&mut app, &ctx, vec![]);
                app.select_tool(tool);
                frame(&mut app, &ctx, vec![]);
                let view = app.view_info();
                app.session.pointer(&PointerEvent::new(PointerKind::Down, 150.0, 120.0), view).unwrap();
                app.session.pointer(&PointerEvent::new(PointerKind::Drag, 180.0, 140.0), view).unwrap();
                assert_eq!(app.session.draws_new_shape_box(), while_drawing && tool != "lineSegment", "{what}");
                let (first, handles) = bounding_box_drawn_in(&mut app, &ctx, layer);
                if tool == "lineSegment" || !while_drawing {
                    assert_eq!((first, handles), (None, 0), "{what}: no box");
                    continue;
                }
                assert_eq!(handles, 8, "{what}: the handles show while the shape is drawn");
                app.session.pointer(&PointerEvent::new(PointerKind::Drag, 210.0, 170.0), view).unwrap();
                let (during, handles) = bounding_box_drawn_in(&mut app, &ctx, layer);
                assert_eq!(handles, 8, "{what}");
                assert!(
                    first.as_ref().zip(during.as_ref()).is_some_and(|(a, b)| !same_corners(a, b)),
                    "{what}: the box follows the drag: {first:?}, {during:?}"
                );
                app.ui.view.bounding_box = false;
                assert_eq!(bounding_box_drawn_in(&mut app, &ctx, layer), (None, 0), "{what}: View › Hide Bounding Box");
                app.ui.view.bounding_box = true;
                app.session.pointer(&PointerEvent::new(PointerKind::Up, 210.0, 170.0), view).unwrap();
                let selection = app.session.active().unwrap().selection.objects.clone();
                assert!(selection.len() == 1 && selection[0] != vectorcraft_doc::NodeId(old), "{what}: the new shape is selected");
                let after = box_corners(&mut app);
                assert!(during.as_ref().is_some_and(|o| same_corners(o, &after)), "{what}: {during:?}, drawn to {after:?}");
            }
        }
    }
}

/// With General › Show Bounding Box While Moving and Drawing on, a failed preview of a shape being
/// drawn (with Real-time Drawing on it leaves the document and the selection as they were before
/// the drag) shows no box rather than the box of the selection from before the drag. With
/// Real-time Drawing on and off.
#[test]
fn a_failed_preview_of_a_shape_being_drawn_shows_no_bounding_box() {
    for real_time in [true, false] {
        let what = format!("real time {real_time}");
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let old = app.session.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 30, "height": 30})).unwrap()["id"].as_u64().unwrap();
        let layer = app.session.active().unwrap().doc.layer_color(vectorcraft_doc::NodeId(old));
        app.session.execute("prefs.set", &json!({"key": "realTimeDrawing", "value": real_time})).unwrap();
        app.session.execute("prefs.set", &json!({"key": "boundingBoxWhileMovingAndDrawing", "value": true})).unwrap();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        app.select_tool("rectangle");
        frame(&mut app, &ctx, vec![]);
        let view = app.view_info();
        app.session.pointer(&PointerEvent::new(PointerKind::Down, 150.0, 120.0), view).unwrap();
        app.session.pointer(&PointerEvent::new(PointerKind::Drag, 180.0, 140.0), view).unwrap();
        assert_eq!(bounding_box_drawn_in(&mut app, &ctx, layer).1, 8, "{what}: the box of the shape being drawn");
        assert!(app.session.preview("shape.rectangle", &json!({"x": "left"})).is_err(), "{what}: the preview fails");
        assert!(app.session.draws_new_shape_box(), "{what}: the shape is still being drawn");
        let selection = app.session.active().unwrap().selection.objects.clone();
        assert_eq!(selection, vec![vectorcraft_doc::NodeId(old)], "{what}: the selection from before the drag");
        assert_eq!(bounding_box_drawn_in(&mut app, &ctx, layer), (None, 0), "{what}: no box, and not the old selection's");
        app.session.pointer(&PointerEvent::new(PointerKind::Drag, 210.0, 170.0), view).unwrap();
        assert_eq!(bounding_box_drawn_in(&mut app, &ctx, layer).1, 8, "{what}: the next step's preview shows the box again");
    }
}

/// With General › Show Bounding Box While Moving and Drawing on, the box of a shape being drawn is
/// in the color of the layer the shape goes on, also when the selection from before the drag is on
/// another layer: with Real-time Drawing off, the shape is only in the pending preview.
#[test]
fn a_shape_being_drawn_has_its_box_in_its_own_layers_color() {
    for real_time in [true, false] {
        let what = format!("real time {real_time}");
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let old = app.session.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 30, "height": 30})).unwrap()["id"].as_u64().unwrap();
        let old_layer = app.session.active().unwrap().doc.layer_color(vectorcraft_doc::NodeId(old));
        let new_layer = vectorcraft_doc::NodeId(app.session.execute("layer.new", &json!({})).unwrap()["id"].as_u64().unwrap());
        let new_color = app.session.active().unwrap().doc.layer_color(new_layer);
        assert_ne!(old_layer, new_color, "{what}: the two layers have different colors");
        assert_eq!(app.session.active().unwrap().selection.objects, vec![vectorcraft_doc::NodeId(old)], "{what}: the old rectangle stays selected");
        app.session.execute("prefs.set", &json!({"key": "realTimeDrawing", "value": real_time})).unwrap();
        app.session.execute("prefs.set", &json!({"key": "boundingBoxWhileMovingAndDrawing", "value": true})).unwrap();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        app.select_tool("rectangle");
        frame(&mut app, &ctx, vec![]);
        let view = app.view_info();
        app.session.pointer(&PointerEvent::new(PointerKind::Down, 150.0, 120.0), view).unwrap();
        app.session.pointer(&PointerEvent::new(PointerKind::Drag, 180.0, 140.0), view).unwrap();
        let (outline, handles) = bounding_box_drawn_in(&mut app, &ctx, new_color);
        assert!(outline.is_some() && handles == 8, "{what}: the box in the new layer's color");
        assert_eq!(bounding_box_drawn_in(&mut app, &ctx, old_layer).0, None, "{what}: not in the old selection's layer color");
        app.session.pointer(&PointerEvent::new(PointerKind::Up, 180.0, 140.0), view).unwrap();
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.layer_containing(st.selection.objects[0]), Some(new_layer), "{what}: the shape went on the new layer");
    }
}

/// With General › Show Bounding Box While Moving and Drawing on and Real-time Drawing off, a shape
/// drawn while point type is selected shows its box but not the type widget, which belongs to the
/// type selected before the drag.
#[test]
fn a_shape_being_drawn_has_no_type_widget() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 40, "y": 40, "text": "Type"})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    let layer = app.session.active().unwrap().doc.layer_color(vectorcraft_doc::NodeId(id));
    app.session.execute("prefs.set", &json!({"key": "realTimeDrawing", "value": false})).unwrap();
    app.session.execute("prefs.set", &json!({"key": "boundingBoxWhileMovingAndDrawing", "value": true})).unwrap();
    let ctx = egui::Context::default();
    let widgets = |app: &mut VectorcraftApp| {
        let r = vectorcraft_tools::typewidget::RADIUS_PX;
        shapes(app, &ctx).iter().filter(|s| matches!(s, Shape::Circle(c) if c.radius == r && c.fill != Color32::TRANSPARENT)).count()
    };
    frame(&mut app, &ctx, vec![]);
    app.select_tool("selection");
    assert_eq!(widgets(&mut app), 1, "the type widget of the selected type");
    app.select_tool("rectangle");
    frame(&mut app, &ctx, vec![]);
    let view = app.view_info();
    app.session.pointer(&PointerEvent::new(PointerKind::Down, 150.0, 120.0), view).unwrap();
    app.session.pointer(&PointerEvent::new(PointerKind::Drag, 210.0, 170.0), view).unwrap();
    assert_eq!(bounding_box_drawn_in(&mut app, &ctx, layer).1, 8, "the box of the shape being drawn");
    assert_eq!(widgets(&mut app), 0, "no type widget beside it");
}

/// With General › Show Bounding Box While Moving and Drawing on, the Pencil, the Pen and the
/// Paintbrush draw no bounding box while they draw, as they show none afterwards; the selection's
/// box does not show either.
#[test]
fn freehand_and_pen_drawing_show_no_bounding_box() {
    for tool in ["pencil", "pen", "paintbrush"] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 30, "height": 30})).unwrap();
        app.session.execute("prefs.set", &json!({"key": "boundingBoxWhileMovingAndDrawing", "value": true})).unwrap();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        app.select_tool(tool);
        frame(&mut app, &ctx, vec![]);
        let view = app.view_info();
        app.session.pointer(&PointerEvent::new(PointerKind::Down, 150.0, 120.0), view).unwrap();
        for (x, y) in [(170.0, 130.0), (190.0, 150.0), (210.0, 170.0)] {
            app.session.pointer(&PointerEvent::new(PointerKind::Drag, x, y), view).unwrap();
        }
        assert!(!app.session.draws_new_shape_box(), "{tool}");
        let layer = app.session.active().unwrap().doc.layer_color(app.session.active().unwrap().doc.default_layer().unwrap());
        assert_eq!(bounding_box_drawn_in(&mut app, &ctx, layer), (None, 0), "{tool}: no box while it draws");
        app.session.pointer(&PointerEvent::new(PointerKind::Up, 210.0, 170.0), view).unwrap();
    }
}

/// A move, a rotation and a corner-radius drag keep hiding the bounding box and its handles until
/// the release (#712, #1229). With General › Show Bounding Box While Moving and Drawing on, a move
/// shows them, and a rotation and a corner-radius drag still hide them.
#[test]
fn a_move_rotate_or_corner_radius_drag_hides_the_bounding_box() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    app.select_tool("selection");
    let drag = |app: &mut VectorcraftApp, from: Point, to: Point| {
        let view = app.view_info();
        app.session.pointer(&PointerEvent::new(PointerKind::Down, from.x, from.y), view).unwrap();
        app.session.pointer(&PointerEvent::new(PointerKind::Drag, to.x, to.y), view).unwrap();
        assert!(app.session.tool_transforming() && !app.session.tool_resizing());
        let during = bounding_box_drawn(app, &ctx);
        app.session.pointer(&PointerEvent::new(PointerKind::Up, to.x, to.y), view).unwrap();
        during
    };
    assert_eq!(bounding_box_drawn(&mut app, &ctx).1, 8);
    assert_eq!(drag(&mut app, Point::new(150.0, 150.0), Point::new(160.0, 160.0)), (None, 0), "a move");
    assert_eq!(bounding_box_drawn(&mut app, &ctx).1, 8, "back after the move");
    let offset = 10.0 / app.view_info().zoom;
    assert_eq!(drag(&mut app, Point::new(210.0 + offset, 210.0 + offset), Point::new(260.0, 160.0)), (None, 0), "a rotation");
    assert_eq!(bounding_box_drawn(&mut app, &ctx).1, 8, "back after the rotation");
    let corner = {
        let st = app.session.active().unwrap();
        vectorcraft_tools::corners::CornerWidgets::of(&st.doc, &st.selection, app.view_info().zoom, false).unwrap().visible().next().unwrap()
    };
    assert_eq!(drag(&mut app, corner, corner + vectorcraft_geom::Vec2::new(10.0, 10.0)), (None, 0), "a corner-radius drag");
    assert_eq!(bounding_box_drawn(&mut app, &ctx).1, 8, "back after the corner-radius drag");
    // General › Show Bounding Box While Moving and Drawing keeps the box during a move only. The
    // corner-radius drag and the rotation are undone first: the rectangle is at 110, 110 to 210, 210.
    app.session.execute("edit.undo", &json!({})).unwrap();
    app.session.execute("edit.undo", &json!({})).unwrap();
    app.session.execute("prefs.set", &json!({"key": "boundingBoxWhileMovingAndDrawing", "value": true})).unwrap();
    assert_eq!(drag(&mut app, Point::new(150.0, 150.0), Point::new(160.0, 160.0)).1, 8, "a move with the preference on");
    assert_eq!(drag(&mut app, Point::new(220.0 + offset, 220.0 + offset), Point::new(270.0, 170.0)), (None, 0), "a rotation with the preference on");
    let corner = {
        let st = app.session.active().unwrap();
        vectorcraft_tools::corners::CornerWidgets::of(&st.doc, &st.selection, app.view_info().zoom, false).unwrap().visible().next().unwrap()
    };
    assert_eq!(drag(&mut app, corner, corner + vectorcraft_geom::Vec2::new(10.0, 10.0)), (None, 0), "a corner-radius drag with the preference on");
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

/// #3797: View › Outline is a setting of each document's view. A document opened or made after
/// another was put in Outline opens in Preview, where a click inside its fill picks it, and each
/// document keeps its own setting.
#[test]
fn outline_is_per_document_and_new_documents_start_in_preview() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 100, "height": 100})).unwrap();
    app.sync_views();
    let inside = Point::new(70.0, 70.0);
    let picks_fill = |app: &VectorcraftApp| super::outline::hit_at(app, inside, 1.0).is_some();
    assert!(picks_fill(&app), "Preview picks the fill");
    app.run("view.outline", json!({})).unwrap();
    assert!(app.outline_view() && app.view_info().outline);
    assert!(!picks_fill(&app), "Outline picks only the path");
    // An SVG opened now opens in Preview, its fill drawn and picked.
    let svg =
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200"><rect x="20" y="20" width="100" height="100" fill="#ff0000"/></svg>"##;
    crate::io::open_bytes(&mut app, "b.svg", svg, None).unwrap();
    assert_eq!(app.session.active_index(), Some(1));
    assert!(!app.outline_view() && !app.view_info().outline, "the opened document is in Preview");
    assert!(picks_fill(&app));
    assert_eq!(crate::menus::dynamic_label(&app, "view.outline", "Outline"), "Outline");
    assert_eq!(crate::menus::checked(&app, "view.outline", &json!({})), Some(false));
    // A new document opens in Preview too.
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    assert!(!app.outline_view());
    // The first document is still in Outline.
    app.session.execute("document.activate", &json!({"index": 0})).unwrap();
    assert!(app.outline_view() && !picks_fill(&app));
    assert_eq!(crate::menus::dynamic_label(&app, "view.outline", "Outline"), "Preview");
    assert_eq!(crate::menus::checked(&app, "view.outline", &json!({})), Some(true));
    // Without a document there is nothing to put in Outline.
    while !app.session.documents().is_empty() {
        app.session.execute("file.close", &json!({})).unwrap();
    }
    app.sync_views();
    assert!(!app.outline_view() && !crate::menus::enabled(&app, "view.outline"));
    assert!(app.run("view.outline", json!({})).is_err());
}

/// One headless canvas frame → (the colors of the filled rectangles that cover the canvas, the
/// fills of the shapes laid over an artboard, the artboards' edges drawn, the colors of the
/// artboards' names).
fn canvas_look(app: &mut VectorcraftApp, ctx: &egui::Context) -> (Vec<Color32>, Vec<Color32>, usize, Vec<Color32>) {
    let s = shapes(app, ctx);
    let canvas = app.canvas_rect.unwrap();
    let xf = Xf::new(canvas, app.view().unwrap());
    let quads: Vec<Vec<Pos2>> = app.session.active().unwrap().doc.artboards.iter().map(|a| xf.quad(a.rect).to_vec()).collect();
    let on = |p: &[Pos2]| quads.iter().any(|q| q.len() == p.len() && q.iter().zip(p).all(|(a, b)| (*a - *b).length() < 0.01));
    let covers: Vec<Color32> = s
        .iter()
        .filter_map(|s| match s {
            Shape::Rect(r) if r.rect.contains_rect(canvas) => Some(r.fill),
            _ => None,
        })
        .collect();
    let papers: Vec<Color32> = s
        .iter()
        .filter_map(|s| match s {
            Shape::Path(p) if p.fill != Color32::TRANSPARENT && on(&p.points) => Some(p.fill),
            _ => None,
        })
        .collect();
    let edges = s.iter().filter(|s| matches!(s, Shape::Path(p) if p.closed && p.fill == Color32::TRANSPARENT && on(&p.points))).count();
    let names: Vec<Color32> = s
        .iter()
        .filter_map(|s| match s {
            Shape::Text(t) if names_an_artboard(t.galley.text()) => Some(t.fallback_color),
            _ => None,
        })
        .collect();
    (covers, papers, edges, names)
}

/// In Outline, in every interface brightness, color scheme and User Interface › Canvas Color, the
/// whole canvas is white, pasteboard included, so the black outlines of art on the pasteboard show.
/// The artboards draw no paper there, only their edges, and their names draw in a dark gray. In
/// Preview the pasteboard keeps its color and the artboards their white paper.
#[test]
fn outline_draws_on_a_white_canvas_with_artboards_as_edges() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 2})).unwrap();
    for brightness in crate::theme::Brightness::ALL {
        for scheme in crate::theme::Scheme::ALL {
            // Canvas Color: matching the brightness, Dark Gray and Medium Gray, as `prefs_dialog` sets it.
            for canvas_color in [None, Some(Color32::from_gray(0x5a)), Some(Color32::from_gray(0x96))] {
                let ctx = egui::Context::default();
                crate::theme::apply_with(&ctx, brightness, Default::default(), scheme);
                if let Some(c) = canvas_color {
                    let mut t = Tokens::get(&ctx);
                    t.pasteboard = c;
                    ctx.data_mut(|d| d.insert_temp(egui::Id::NULL, t));
                }
                let case = format!("{brightness:?} {scheme:?} {canvas_color:?}");
                let pasteboard = Tokens::get(&ctx).pasteboard;
                assert_ne!(pasteboard, Color32::WHITE, "{case}");
                app.view_mut().unwrap().outline = false;
                let (covers, papers, edges, names) = canvas_look(&mut app, &ctx);
                assert_eq!(covers.last(), Some(&pasteboard), "{case}: Preview keeps the pasteboard");
                assert_eq!(papers, [Color32::WHITE; 2], "{case}: and the artboards' paper");
                assert_eq!(edges, 2, "{case}");
                assert_eq!(names, [Tokens::get(&ctx).text_dim; 2], "{case}");
                app.view_mut().unwrap().outline = true;
                let (covers, papers, edges, names) = canvas_look(&mut app, &ctx);
                assert_eq!(covers.last(), Some(&Color32::WHITE), "{case}: the canvas is white in Outline");
                assert!(!covers.contains(&pasteboard), "{case}: no pasteboard in Outline");
                assert!(papers.is_empty(), "{case}: no paper over the artboards in Outline: {papers:?}");
                assert_eq!(edges, 2, "{case}: the artboards show as their edges");
                assert_eq!(names, [OUTLINE_DIM; 2], "{case}: names readable on white");
            }
        }
    }
}

/// In Outline the canvas stays white with Simulate Colored Paper on, with an artboard's own
/// background color, and with View › Hide Artboards, which in Preview show the paper color, the
/// artboard's color and the paper color over the whole canvas.
#[test]
fn outline_draws_no_colored_paper() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 2})).unwrap();
    {
        let doc = std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc);
        doc.setup.simulate_paper = true;
        doc.setup.grid_colors = [vectorcraft_color::Color::rgb(0.9, 0.8, 0.6), vectorcraft_color::Color::rgb(0.5, 0.5, 0.5)];
        doc.artboards[1].color = Some(vectorcraft_color::Color::rgb(0.6, 0.8, 1.0));
    }
    let paper = crate::panels::c32(&app.session.active().unwrap().doc.setup.paper());
    let ctx = egui::Context::default();
    crate::theme::apply(&ctx, crate::theme::Brightness::Dark);
    app.view_mut().unwrap().outline = false;
    let (_, papers, _, _) = canvas_look(&mut app, &ctx);
    assert_eq!(papers.len(), 2);
    assert_eq!(papers[0], paper, "Preview: the simulated paper");
    assert_ne!(papers[1], paper, "Preview: the artboard's own color");
    app.view_mut().unwrap().outline = true;
    let (covers, papers, edges, _) = canvas_look(&mut app, &ctx);
    assert_eq!(covers.last(), Some(&Color32::WHITE));
    assert!(papers.is_empty(), "{papers:?}");
    assert_eq!(edges, 2);
    app.ui.view.artboards = false;
    app.view_mut().unwrap().outline = false;
    let (covers, _, edges, _) = canvas_look(&mut app, &ctx);
    assert_eq!(covers.last(), Some(&paper), "Preview with Hide Artboards: the paper over the whole canvas");
    assert_eq!(edges, 0);
    app.view_mut().unwrap().outline = true;
    let (covers, papers, edges, _) = canvas_look(&mut app, &ctx);
    assert_eq!(covers.last(), Some(&Color32::WHITE), "Outline with Hide Artboards");
    assert!(!covers.contains(&paper));
    assert!(papers.is_empty());
    assert_eq!(edges, 0);
}

/// A split-view pane or floating window draws its own view's Outline, whatever the current view
/// shows: the white canvas and dark gray artboard names in Outline, and the pasteboard and the
/// theme's names in Preview.
#[test]
fn a_pane_draws_its_own_views_outline() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100, "artboards": 2})).unwrap();
    crate::doc_windows::new_window(&mut app).unwrap();
    let ctx = egui::Context::default();
    crate::theme::apply(&ctx, crate::theme::Brightness::Dark);
    let t = Tokens::get(&ctx);
    let other = (0, 1);
    for pane_outline in [true, false] {
        crate::doc_windows::view_mut(&mut app, other).unwrap().outline = pane_outline;
        app.view_mut().unwrap().outline = !pane_outline;
        let cell = egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 400.0));
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            panes::pane(&mut app, ui, other, cell);
        });
        out.textures_delta.clear();
        let mut s = vec![];
        fn flat(s: Shape, out: &mut Vec<Shape>) {
            match s {
                Shape::Vec(v) => v.into_iter().for_each(|s| flat(s, out)),
                s => out.push(s),
            }
        }
        out.shapes.into_iter().for_each(|c| flat(c.shape, &mut s));
        let fills: Vec<Color32> = s.iter().filter_map(|s| if let Shape::Rect(r) = s { Some(r.fill) } else { None }).collect();
        let names: Vec<Color32> = s
            .iter()
            .filter_map(|s| if let Shape::Text(x) = s { names_an_artboard(x.galley.text()).then_some(x.fallback_color) } else { None })
            .collect();
        let (canvas, name) = if pane_outline { (Color32::WHITE, OUTLINE_DIM) } else { (t.pasteboard, t.text_dim) };
        assert_eq!(fills.first(), Some(&canvas), "pane in Outline: {pane_outline}");
        assert_eq!(names, [name; 2], "pane in Outline: {pane_outline}");
    }
}

/// With a display profile, a split-view pane or floating window blends translucent art with the
/// paper before the conversion, as the canvas does, and in its own Outline converts the art alone.
#[test]
fn a_pane_blends_translucent_art_before_the_display_profile() {
    use crate::display_profile::{Found, Reading};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let half = app.run("shape.rectangle", json!({"x": 50, "y": 25, "width": 100, "height": 50})).unwrap()["id"].clone();
    app.run("paint.setFill", json!({"ids": [half], "color": "#ff0000"})).unwrap();
    app.run("transparency.set", json!({"ids": [half], "opacity": 50})).unwrap();
    app.run("select.none", json!({})).unwrap();
    crate::doc_windows::new_window(&mut app).unwrap();
    app.canvas.panes.worker_started = true; // every pane render on this thread
    let p3 = crate::display_profile::tests::p3();
    let t = vectorcraft_color::cms::display::DisplayTransform::from_icc(&p3).unwrap();
    app.display_profile.report(Reading { display: Some("DP-1".into()), found: Found::Profile { icc: p3, from: "test".into() } });
    let ctx = egui::Context::default();
    let other = (0, 1);
    let slot = (app.session.documents()[0].uid, 1);
    // The pane's raster one frame uploads, and the key it was rendered for.
    let pane = |app: &mut VectorcraftApp| -> (Vec<egui::Color32>, CacheKey) {
        let cell = egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 400.0));
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            panes::pane(app, ui, other, cell);
        });
        let (key, tex) = app.canvas.panes.shown.get(&slot).expect("the pane's render");
        let id = tex.id();
        let pixels = out.textures_delta.set.iter().find(|e| e.0.eq(&id)).and_then(|e| e.1.last()).map(|d| {
            let egui::ImageData::Color(img) = &d.image;
            img.pixels.clone()
        });
        let key = key.clone();
        out.textures_delta.clear();
        (pixels.expect("the pane uploads"), key)
    };
    app.session.prefs.monitor_profile_display = false;
    let (srgb, key) = pane(&mut app);
    assert!(key.backdrop.is_none(), "no display profile: nothing to blend");
    app.session.prefs.monitor_profile_display = true;
    let (shown, key) = pane(&mut app);
    assert!(key.backdrop.is_some(), "the pane's Preview blends with its paper");
    assert_eq!(srgb.len(), shown.len());
    let plain = |s: egui::Color32| {
        let mut px = s.to_array();
        t.apply_premultiplied(&mut px);
        egui::Color32::from_rgba_premultiplied(px[0], px[1], px[2], px[3])
    };
    let mut blended = 0;
    for (s, c) in srgb.iter().zip(&shown) {
        if s.a() == 0 || s.a() == 255 {
            assert_eq!(*c, plain(*s), "opaque and empty pixels convert as before");
        } else {
            assert_eq!(c.a(), s.a(), "the alpha is kept");
            let p = plain(*s);
            if (0..3).any(|j| c.to_array()[j].abs_diff(p.to_array()[j]) > 2) {
                blended += 1;
            }
        }
    }
    assert!(blended > 100, "{blended} translucent pixels blended with the paper");
    // The pane's own Outline: the art converts alone, while the current view stays in Preview.
    crate::doc_windows::view_mut(&mut app, other).unwrap().outline = true;
    app.view_mut().unwrap().outline = false;
    let (_, key) = pane(&mut app);
    assert!(key.outline && key.backdrop.is_none(), "the pane's Outline has no paper to blend with");
}

/// #3797: a preferences file saved while View › Outline was an interface setting still loads, with
/// its other view settings, and its Outline value does not put a document in Outline.
#[test]
fn old_outline_preference_loads_but_does_not_restore_outline() {
    let mut saved = serde_json::to_value(crate::UiState::default()).unwrap();
    assert!(saved["view"].get("outline").is_none(), "Outline is not saved with the interface");
    saved["view"]["outline"] = json!(true);
    saved["view"]["rulers"] = json!(true);
    let ui = serde_json::from_value::<crate::UiState>(saved).unwrap().sanitized();
    assert!(ui.view.rulers, "the other view settings load");
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.ui = ui;
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    assert!(!app.outline_view(), "a new document is in Preview");
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

/// A selected live polygon shows its side widget, a white diamond beside its box, with the
/// Selection tool and the shape tools while the bounding box shows, and hides with the box while
/// the polygon is moved.
#[test]
fn a_selected_polygon_shows_its_side_widget() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.polygon", &json!({"cx": 200, "cy": 150, "radius": 60, "sides": 5})).unwrap();
    let ctx = egui::Context::default();
    // A white diamond as wide and tall as the widget.
    let size = 2.0 * vectorcraft_tools::sidewidget::RADIUS_PX;
    let diamond = |ps: &egui::epaint::PathShape| {
        let span = |f: fn(&egui::Pos2) -> f32| {
            let v: Vec<f32> = ps.points.iter().map(f).collect();
            v.iter().copied().fold(f32::MIN, f32::max) - v.iter().copied().fold(f32::MAX, f32::min)
        };
        ps.closed && ps.points.len() == 4 && ps.fill == Color32::WHITE && (span(|p| p.x) - size).abs() < 0.01 && (span(|p| p.y) - size).abs() < 0.01
    };
    let widgets = |app: &mut VectorcraftApp| shapes(app, &ctx).iter().filter(|s| matches!(s, Shape::Path(ps) if diamond(ps))).count();
    app.select_tool("selection");
    assert_eq!(widgets(&mut app), 1);
    app.select_tool("polygon");
    assert_eq!(widgets(&mut app), 1);
    app.select_tool("directSelection");
    assert_eq!(widgets(&mut app), 0);
    app.select_tool("lineSegment");
    assert_eq!(widgets(&mut app), 0);
    app.select_tool("selection");
    app.run("view.boundingBox", json!({})).unwrap();
    assert_eq!(widgets(&mut app), 0, "no bounding box, no widget");
    app.run("view.boundingBox", json!({})).unwrap();
    // A move of the polygon hides the box, and the widget with it, until the release.
    let view = app.view_info();
    app.session.pointer(&PointerEvent::new(PointerKind::Down, 200.0, 150.0), view).unwrap();
    app.session.pointer(&PointerEvent::new(PointerKind::Drag, 260.0, 150.0), view).unwrap();
    assert_eq!(widgets(&mut app), 0, "hidden while the polygon moves");
    app.session.pointer(&PointerEvent::new(PointerKind::Up, 260.0, 150.0), view).unwrap();
    assert_eq!(widgets(&mut app), 1, "back after the release");
    app.session.execute("shape.ellipse", &json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    assert_eq!(widgets(&mut app), 0, "an ellipse has none");
}

/// Hide Shape Widgets (`view.shapeWidgets`) hides a selected live rectangle's and polygon's corner
/// widgets, a polygon's side widget and an ellipse's pie widgets with the Selection tool and the
/// shape tools, and leaves the bounding box and its eight handles; the Selection tool then shows no
/// corner cursor over a hidden widget. Direct Selection keeps its corner widgets, which follow View ›
/// Hide Corner Widget alone. Show Shape Widgets brings them back, neither adds an undo step, and a
/// tool switch keeps the setting.
#[test]
fn hidden_shape_widgets_leave_the_bounding_box() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = |v: serde_json::Value| v["id"].as_u64().unwrap();
    let rect = id(app.session.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 80, "height": 80})).unwrap());
    let polygon = id(app.session.execute("shape.polygon", &json!({"cx": 200, "cy": 150, "radius": 50, "sides": 5})).unwrap());
    let ellipse = id(app.session.execute("shape.ellipse", &json!({"x": 280, "y": 100, "width": 100, "height": 60})).unwrap());
    let ctx = egui::Context::default();
    let side = 2.0 * vectorcraft_tools::sidewidget::RADIUS_PX;
    let diamond = |ps: &egui::epaint::PathShape| {
        let span = |f: fn(&egui::Pos2) -> f32| {
            let v: Vec<f32> = ps.points.iter().map(f).collect();
            v.iter().copied().fold(f32::MIN, f32::max) - v.iter().copied().fold(f32::MAX, f32::min)
        };
        ps.closed && ps.points.len() == 4 && ps.fill == Color32::WHITE && (span(|p| p.x) - side).abs() < 0.01 && (span(|p| p.y) - side).abs() < 0.01
    };
    // The selection's corner, pie and side widgets drawn, and the bounding box's handles.
    let drawn = |app: &mut VectorcraftApp, id: u64| {
        app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
        let (_, handles) = bounding_box_drawn(app, &ctx);
        let all = shapes(app, &ctx);
        let circles = |r: f32| all.iter().filter(|s| matches!(s, Shape::Circle(c) if c.radius == r && c.fill == Color32::WHITE)).count();
        let diamonds = all.iter().filter(|s| matches!(s, Shape::Path(ps) if diamond(ps))).count();
        (circles(3.0), circles(vectorcraft_tools::pie::RADIUS_PX), diamonds, handles)
    };
    let steps = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
    for tool in ["selection", "polygon", "rectangle"] {
        app.select_tool(tool);
        assert_eq!(drawn(&mut app, rect), (4, 0, 0, 8), "{tool}");
        assert_eq!(drawn(&mut app, polygon), (5, 0, 1, 8), "{tool}");
        assert_eq!(drawn(&mut app, ellipse), (0, 2, 0, 8), "{tool}");
        let before = steps(&app);
        assert_eq!(app.run("view.shapeWidgets", json!({})).unwrap(), json!(false));
        assert_eq!(steps(&app), before, "{tool}: the setting adds no undo step");
        assert_eq!(drawn(&mut app, rect), (0, 0, 0, 8), "{tool}");
        assert_eq!(drawn(&mut app, polygon), (0, 0, 0, 8), "{tool}");
        assert_eq!(drawn(&mut app, ellipse), (0, 0, 0, 8), "{tool}");
        assert_eq!(app.run("view.shapeWidgets", json!({})).unwrap(), json!(true));
    }
    // Direct Selection: the corner widgets stay until View › Hide Corner Widget.
    app.run("view.shapeWidgets", json!({"on": false})).unwrap();
    app.select_tool("directSelection");
    assert_eq!(drawn(&mut app, rect).0, 4);
    app.run("view.cornerWidget", json!({})).unwrap();
    assert_eq!(drawn(&mut app, rect).0, 0);
    app.run("view.cornerWidget", json!({})).unwrap();
    // The Selection tool shows no corner cursor over a hidden widget, and the tool switches kept the
    // setting.
    app.select_tool("selection");
    assert!(!app.ui.view.shape_widgets);
    drawn(&mut app, rect);
    let at = {
        let st = app.session.active().unwrap();
        vectorcraft_tools::corners::CornerWidgets::of(&st.doc, &st.selection, app.view_info().zoom, false).unwrap().visible().next().unwrap()
    };
    let view = app.view_info();
    assert_ne!(app.session.cursor(at, Mods::default(), view), Cursor::CornerRadius);
    app.run("view.shapeWidgets", json!({"on": true})).unwrap();
    let view = app.view_info();
    assert_eq!(app.session.cursor(at, Mods::default(), view), Cursor::CornerRadius);
}

/// A selected Objects on Path group shows its Rotate All widget with the Selection and Objects on
/// Path tools.
#[test]
fn a_selected_objects_on_path_group_shows_its_rotate_all_widget() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = |v: serde_json::Value| v["id"].as_u64().unwrap();
    let line = id(app.session.execute("shape.line", &json!({"x1": 20, "y1": 200, "x2": 380, "y2": 200})).unwrap());
    let a = id(app.session.execute("shape.rectangle", &json!({"x": 40, "y": 40, "width": 20, "height": 20})).unwrap());
    let b = id(app.session.execute("shape.rectangle", &json!({"x": 80, "y": 40, "width": 20, "height": 20})).unwrap());
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    app.session.execute("object.objectsOnPath.attach", &json!({"path": line})).unwrap();
    let ctx = egui::Context::default();
    let widgets = |app: &mut VectorcraftApp| {
        let r = vectorcraft_tools::objects_on_path::ROTATE_ALL_RADIUS_PX;
        shapes(app, &ctx).iter().filter(|s| matches!(s, Shape::Circle(c) if c.radius == r && c.fill == Color32::WHITE)).count()
    };
    app.select_tool("selection");
    assert_eq!(widgets(&mut app), 1);
    app.select_tool("objectsOnPath");
    assert_eq!(widgets(&mut app), 1);
    app.select_tool("directSelection");
    assert_eq!(widgets(&mut app), 0);
    app.select_tool("selection");
    app.run("select.set", json!({"ids": [a]})).unwrap();
    assert_eq!(widgets(&mut app), 0, "an object in the group, not the group");
}

/// A selected Objects on Path group of two or more objects shows its Space widget, a white square
/// beyond its last object, with the Selection and Objects on Path tools.
#[test]
fn a_selected_objects_on_path_group_shows_its_space_widget() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = |v: serde_json::Value| v["id"].as_u64().unwrap();
    let line = id(app.session.execute("shape.line", &json!({"x1": 20, "y1": 200, "x2": 380, "y2": 200})).unwrap());
    let a = id(app.session.execute("shape.rectangle", &json!({"x": 40, "y": 40, "width": 20, "height": 20})).unwrap());
    let b = id(app.session.execute("shape.rectangle", &json!({"x": 80, "y": 40, "width": 20, "height": 20})).unwrap());
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let g = id(app.session.execute("object.objectsOnPath.attach", &json!({"path": line})).unwrap());
    let ctx = egui::Context::default();
    let widgets = |app: &mut VectorcraftApp| {
        let side = 2.0 * vectorcraft_tools::objects_on_path::SPACE_HALF_PX;
        shapes(app, &ctx)
            .iter()
            .filter(|s| matches!(s, Shape::Rect(r) if r.rect.width() == side && r.rect.height() == side && r.fill == Color32::WHITE))
            .count()
    };
    app.select_tool("selection");
    assert_eq!(widgets(&mut app), 1);
    app.select_tool("objectsOnPath");
    assert_eq!(widgets(&mut app), 1);
    app.select_tool("directSelection");
    assert_eq!(widgets(&mut app), 0);
    app.select_tool("selection");
    app.run("select.set", json!({"ids": [a]})).unwrap();
    assert_eq!(widgets(&mut app), 0, "an object in the group, not the group");
    // With one object on the path there is no space to set.
    app.run("edit.clear", json!({"ids": [b]})).unwrap();
    app.run("select.set", json!({"ids": [g]})).unwrap();
    assert_eq!(widgets(&mut app), 0, "one object");
}

/// A selected Objects on Path group whose objects have room to move along the path shows its Move
/// All widget, a white diamond below its first object, with the Selection and Objects on Path tools;
/// objects spread evenly to both ends of an open path have no room, and show none.
#[test]
fn a_selected_objects_on_path_group_shows_its_move_all_widget_when_it_has_room() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = |v: serde_json::Value| v["id"].as_u64().unwrap();
    let line = id(app.session.execute("shape.line", &json!({"x1": 20, "y1": 200, "x2": 380, "y2": 200})).unwrap());
    let a = id(app.session.execute("shape.rectangle", &json!({"x": 40, "y": 40, "width": 20, "height": 20})).unwrap());
    let b = id(app.session.execute("shape.rectangle", &json!({"x": 80, "y": 40, "width": 20, "height": 20})).unwrap());
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let g = id(app.session.execute("object.objectsOnPath.attach", &json!({"path": line})).unwrap());
    let ctx = egui::Context::default();
    // A diamond: its top and bottom corners one above the other, its full height apart.
    let diamond = |d: &egui::epaint::PathShape| {
        let h = 2.0 * vectorcraft_tools::objects_on_path::MOVE_ALL_HALF_PX;
        let [top, _, bottom, _] = d.points[..] else { return false };
        d.closed && d.fill == Color32::WHITE && top.x == bottom.x && (bottom.y - top.y - h).abs() < 1e-3
    };
    let widgets = |app: &mut VectorcraftApp| shapes(app, &ctx).iter().filter(|s| matches!(s, Shape::Path(d) if diamond(d))).count();
    app.select_tool("selection");
    assert_eq!(widgets(&mut app), 0, "Auto: the objects reach both ends");
    app.session.execute("object.objectsOnPath.options", &json!({"ids": [g], "spacing": 60})).unwrap();
    assert_eq!(widgets(&mut app), 1);
    app.select_tool("objectsOnPath");
    assert_eq!(widgets(&mut app), 1);
    app.select_tool("directSelection");
    assert_eq!(widgets(&mut app), 0);
    app.select_tool("selection");
    app.run("select.set", json!({"ids": [a]})).unwrap();
    assert_eq!(widgets(&mut app), 0, "an object in the group, not the group");
    // One object can move along the path whatever its Space.
    app.session.execute("object.objectsOnPath.options", &json!({"ids": [g], "spacing": "auto"})).unwrap();
    app.run("edit.clear", json!({"ids": [b]})).unwrap();
    app.run("select.set", json!({"ids": [g]})).unwrap();
    assert_eq!(widgets(&mut app), 1, "one object");
}

/// A selected Objects on Path group shows a Select/Move widget on each object's attachment point
/// with the Selection and Objects on Path tools, the chosen object's filled blue.
#[test]
fn a_selected_objects_on_path_group_shows_a_select_move_widget_per_object() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = |v: serde_json::Value| v["id"].as_u64().unwrap();
    let line = id(app.session.execute("shape.line", &json!({"x1": 20, "y1": 200, "x2": 380, "y2": 200})).unwrap());
    let a = id(app.session.execute("shape.rectangle", &json!({"x": 40, "y": 40, "width": 20, "height": 20})).unwrap());
    let b = id(app.session.execute("shape.rectangle", &json!({"x": 80, "y": 40, "width": 20, "height": 20})).unwrap());
    let c = id(app.session.execute("shape.rectangle", &json!({"x": 120, "y": 40, "width": 20, "height": 20})).unwrap());
    app.run("select.set", json!({"ids": [a, b, c]})).unwrap();
    app.session.execute("object.objectsOnPath.attach", &json!({"path": line})).unwrap();
    let ctx = egui::Context::default();
    let r = vectorcraft_tools::objects_on_path::MEMBER_RADIUS_PX;
    let widgets = |app: &mut VectorcraftApp, fill: Color32| {
        shapes(app, &ctx).iter().filter(|s| matches!(s, Shape::Circle(k) if k.radius == r && k.fill == fill)).count()
    };
    let (ring, chosen) = (Color32::TRANSPARENT, super::overlays::MEMBER_CHOSEN);
    app.select_tool("selection");
    assert_eq!((widgets(&mut app, ring), widgets(&mut app, chosen)), (3, 0));
    app.run("object.objectsOnPath.selectMember", json!({"member": b})).unwrap();
    assert_eq!((widgets(&mut app, ring), widgets(&mut app, chosen)), (2, 1), "the chosen one is filled");
    app.select_tool("objectsOnPath");
    assert_eq!((widgets(&mut app, ring), widgets(&mut app, chosen)), (2, 1));
    app.select_tool("directSelection");
    assert_eq!(widgets(&mut app, ring) + widgets(&mut app, chosen), 0);
    app.select_tool("selection");
    app.run("select.set", json!({"ids": [a]})).unwrap();
    assert_eq!(widgets(&mut app, ring) + widgets(&mut app, chosen), 0, "an object in the group, not the group");
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
    // Each attempt sets different text: text the object already has is no edit, and a second
    // attempt with the same text would leave the object as it was.
    let edits = std::cell::Cell::new(0);
    let (before, after) = frames(&mut app, &|app| {
        edits.set(edits.get() + 1);
        app.session.execute("text.setText", &json!({"id": ids[1].0, "text": format!("a b c {}", edits.get())})).unwrap();
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
        .filter(|s| match s {
            Shape::Circle(c) => c.fill == blue || c.stroke.color == blue,
            Shape::Text(t) => t.fallback_color == blue,
            Shape::LineSegment { stroke, .. } => stroke.color == blue,
            Shape::Path(p) => p.stroke.color == egui::epaint::ColorMode::Solid(blue),
            _ => false,
        })
        .collect()
}

/// Show Hidden Characters marks each tab, nonbreaking space, narrow nonbreaking space, space of
/// a set width and discretionary hyphen with a mark of its kind, a plain space with a dot, and
/// a tab's arrow spans the tab's advance to its stop.
#[test]
fn hidden_characters_mark_tabs_nonbreaking_spaces_and_discretionary_hyphens() {
    use super::tool_overlays::CharMark::*;
    let text = "a b\tc\u{00A0}d\u{202F}e\u{2003}f\u{2009}g\u{200A}h\u{00AD}i";
    let t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(10.0, 50.0), text, vectorcraft_doc::CharStyle::default());
    let marks = HiddenMarks::of(&t);
    let kinds: Vec<_> = marks.chars.iter().map(|c| c.0).collect();
    assert_eq!(kinds, [Tab, NoBreakSpace, NarrowNoBreakSpace, FixedSpace, FixedSpace, FixedSpace, DiscretionaryHyphen]);
    assert_eq!(marks.spaces.len(), 1, "only the plain space has a dot");
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    let tab = lay.glyphs.iter().find(|g| text[g.byte..].starts_with('\t')).unwrap();
    assert!(tab.advance > 1.0, "{}", tab.advance);
    let (_, a, b) = marks.chars[0];
    let start = t.xf * vectorcraft_geom::Point::new(tab.origin.x, tab.origin.y - 3.0);
    assert!((a - start).hypot() < 1e-9 && (b.x - a.x - tab.advance).abs() < 1e-9 && b.y == a.y, "{a:?} {b:?} {start:?}");
}

/// Show Hidden Characters marks a line that a forced line break (U+2028) ends with a bent arrow
/// and no ¶; the line a paragraph return ends gets ¶, and the story's end #.
#[test]
fn hidden_characters_mark_forced_line_breaks() {
    let t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::ZERO, "ab\u{2028}cd\nef", vectorcraft_doc::CharStyle::default());
    let marks = HiddenMarks::of(&t);
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    assert_eq!(lay.lines.len(), 3);
    assert_eq!(marks.breaks.len(), 1);
    let (at, along, up) = marks.breaks[0];
    let first = &lay.lines[0];
    assert_eq!(at, vectorcraft_geom::Point::new(first.x1 + 1.0, first.baseline));
    assert_eq!((along - at, up - at), (vectorcraft_geom::Vec2::new(1.0, 0.0), vectorcraft_geom::Vec2::new(0.0, -1.0)));
    let ends: Vec<_> = marks.ends.iter().map(|e| (e.1, e.0.y)).collect();
    assert_eq!(ends, [("¶", lay.lines[1].baseline), ("#", lay.lines[2].baseline)]);
    // A story that ends with a forced line break: the arrow, and # on the empty line after it.
    let t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::ZERO, "ab\u{2028}", vectorcraft_doc::CharStyle::default());
    let marks = HiddenMarks::of(&t);
    assert_eq!((marks.breaks.len(), marks.ends.iter().map(|e| e.1).collect::<Vec<_>>()), (1, vec!["#"]));
}

/// A discretionary hyphen gets one ¬ also where the line breaks at it and the layout draws a
/// hyphen there, which stands for no character of the text.
#[test]
fn hidden_characters_mark_a_discretionary_hyphen_at_a_break_once() {
    let style = vectorcraft_doc::CharStyle::default();
    let db = vectorcraft_text::FontDb::global();
    let word = vectorcraft_text::layout(db, &vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::ZERO, "coco-", style.clone()));
    let width = word.lines[0].x1 - word.lines[0].x0 + 2.0;
    let mut t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::ZERO, "coco\u{00AD}co", style);
    t.kind = vectorcraft_doc::text::TextKind::Area {
        frame: vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::Rect::new(0.0, 0.0, width, 400.0).to_path(0.1)),
    };
    let lay = vectorcraft_text::layout(db, &t);
    assert_eq!(lay.lines.len(), 2, "the line breaks at the discretionary hyphen");
    assert!(lay.glyphs.iter().any(|g| g.len == 0), "the layout draws a hyphen at the break");
    let marks = HiddenMarks::of(&t);
    assert_eq!(marks.chars.iter().map(|c| c.0).collect::<Vec<_>>(), [super::tool_overlays::CharMark::DiscretionaryHyphen]);
}

/// In vertical type the marks follow the column: a space's dot sits beside the column, halfway
/// down the space, and ¶ goes below the column's last character.
#[test]
fn hidden_characters_follow_vertical_type() {
    let mut t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::ZERO, "a b\nc", vectorcraft_doc::CharStyle::default());
    t.vertical = true;
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    let space = lay.glyphs.iter().find(|g| g.byte == 1).unwrap();
    let marks = HiddenMarks::of(&t);
    let dot = marks.spaces[0];
    assert!((dot - vectorcraft_geom::Point::new(space.origin.x + 3.0, space.origin.y + space.advance / 2.0)).hypot() < 1e-9, "{dot:?} {space:?}");
    let (at, glyph, align) = marks.ends[0];
    assert_eq!((glyph, align), ("¶", egui::Align2::CENTER_TOP));
    let b = lay.glyphs.iter().find(|g| g.byte == 2).unwrap();
    assert!(at.y > b.origin.y, "{at:?} below {:?}", b.origin);
}

/// In a right-to-left paragraph ¶ goes at the line's left end, where the paragraph ends.
#[test]
fn hidden_characters_end_a_right_to_left_line_at_its_left() {
    let t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::ZERO, "אב גד\nהו", vectorcraft_doc::CharStyle::default());
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    assert!(lay.lines[0].rtl);
    let marks = HiddenMarks::of(&t);
    let (at, glyph, align) = marks.ends[0];
    assert_eq!((glyph, align), ("¶", egui::Align2::RIGHT_BOTTOM));
    assert_eq!(at.x, lay.lines[0].x0 - 1.0);
}

/// Type on a path has the marks over its characters and # after its last one.
#[test]
fn hidden_characters_mark_type_on_a_path() {
    let mut t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::ZERO, "a b\tc", vectorcraft_doc::CharStyle::default());
    let path = vectorcraft_geom::Line::new((0.0, 100.0), (600.0, 100.0)).to_path(0.1);
    t.kind = vectorcraft_doc::text::TextKind::OnPath { path: vectorcraft_geom::PathData::from_bezpath(&path), start: 0.0, end: None };
    let marks = HiddenMarks::of(&t);
    assert_eq!(marks.spaces.len(), 1);
    assert_eq!(marks.chars.iter().map(|c| c.0).collect::<Vec<_>>(), [super::tool_overlays::CharMark::Tab]);
    assert_eq!(marks.ends.iter().map(|e| e.1).collect::<Vec<_>>(), ["#"]);
}

/// The overlay draws each kind of mark: a dot, two rings, the tab's arrow, a tick, ¬, the forced
/// line break's bent arrow, ¶ and #.
#[test]
fn show_hidden_characters_draws_every_kind_of_mark() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let text = "a b\tc\u{00A0}d\u{202F}e\u{2003}f\u{00AD}g\u{2028}h\ni";
    app.session.execute("text.create", &json!({"x": 20, "y": 60, "text": text})).unwrap();
    let ctx = egui::Context::default();
    let xf = Xf { rect: egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)), zoom: 1.0, center: Point::new(200.0, 150.0), rot: 0.0 };
    let shapes = hidden_chars(&mut app, &ctx, &xf);
    let count = |f: &dyn Fn(&Shape) -> bool| shapes.iter().filter(|s| f(s)).count();
    assert_eq!(count(&|s| matches!(s, Shape::Circle(c) if c.fill != Color32::TRANSPARENT)), 1, "{shapes:?}");
    assert_eq!(count(&|s| matches!(s, Shape::Circle(c) if c.fill == Color32::TRANSPARENT)), 2, "{shapes:?}");
    assert_eq!(count(&|s| matches!(s, Shape::LineSegment { .. })), 2, "the tab's shaft and the tick: {shapes:?}");
    assert_eq!(count(&|s| matches!(s, Shape::Path(_))), 3, "the tab's head and the break's arrow: {shapes:?}");
    assert_eq!(count(&|s| matches!(s, Shape::Text(_))), 3, "¬, ¶ and #: {shapes:?}");
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
    let text = json!({"x": 20, "y": 60, "text": "one\ttwo\u{00A0}x\nthree\u{2028}four"});
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

/// A canvas frame at input time `t` (seconds) with `events`, for the trackpad rotation tests.
fn timed_frame(app: &mut VectorcraftApp, ctx: &egui::Context, t: f64, events: Vec<egui::Event>) {
    let raw =
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), time: Some(t), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| show(app, ui));
    out.textures_delta.clear();
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

/// While the Type tool edits text, Space types a space: held, it shows no hand and a drag with it
/// does not pan the view. Cmd+Space still zooms.
#[test]
fn space_types_a_space_while_editing_text_and_does_not_pan() {
    use egui::{CursorIcon, Event, Modifiers};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.prefs.placeholder_text = false;
    let ctx = egui::Context::default();
    let id = start_typing(&mut app, &ctx);
    let at = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap()).to_screen(Point::new(100.0, 100.0));
    let space = |pressed, modifiers| Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers };
    let button = |pos, pressed, modifiers| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
    // The cursor the canvas sets over a window without the focus, as in
    // `the_rotate_cursor_shows_for_space_shift_and_the_rotate_view_tool`.
    let shown = |app: &mut VectorcraftApp| {
        let screen_rect = Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)));
        let mut out = ctx.run_ui(egui::RawInput { screen_rect, focused: false, ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
        out.platform_output.cursor_icon
    };
    typing_frame(&mut app, &ctx, vec![Event::Text("a".into())]);
    typing_frame(&mut app, &ctx, vec![Event::PointerMoved(at), space(true, Modifiers::NONE), Event::Text(" ".into())]);
    assert_eq!(plain(&app, id), "a ", "Space types a space");
    assert_ne!(shown(&mut app), CursorIcon::Grab, "held while editing, Space shows no hand");
    let before = *app.view().unwrap();
    typing_frame(&mut app, &ctx, vec![button(at, true, Modifiers::NONE)]);
    typing_frame(&mut app, &ctx, vec![Event::PointerMoved(at + vec2(30.0, 0.0))]);
    typing_frame(&mut app, &ctx, vec![button(at + vec2(30.0, 0.0), false, Modifiers::NONE)]);
    let after = *app.view().unwrap();
    assert_eq!((after.center, after.zoom, after.rotation), (before.center, before.zoom, before.rotation), "a drag with Space held does not pan");
    assert!(app.session.tool_wants_text(), "editing goes on");
    typing_frame(&mut app, &ctx, vec![space(false, Modifiers::NONE)]);
    // Cmd+Space still lends the Zoom tool.
    typing_frame(&mut app, &ctx, vec![Event::ModifiersChanged(Modifiers::COMMAND), space(true, Modifiers::COMMAND), Event::PointerMoved(at)]);
    assert_eq!(shown(&mut app), CursorIcon::ZoomIn, "Cmd+Space shows the magnifier");
    typing_frame(&mut app, &ctx, vec![button(at, true, Modifiers::COMMAND)]);
    typing_frame(&mut app, &ctx, vec![button(at, false, Modifiers::COMMAND)]);
    assert_eq!(app.view().unwrap().zoom, crate::state::next_zoom(before.zoom, true), "Cmd+Space click zooms in");
    typing_frame(&mut app, &ctx, vec![space(false, Modifiers::COMMAND), Event::ModifiersChanged(Modifiers::NONE)]);
    assert_eq!(plain(&app, id), "a ", "the text is as typed");
}

/// While the Type tool edits text, a zoom marquee that Cmd+Space started keeps its Space rules once
/// Cmd is let go: Space held since the press sizes it, and Space pressed again moves it on every
/// frame it stays held, without typing a space. With no marquee, Space types a space again, and
/// during an animated zoom drag it types none.
#[test]
fn space_moves_a_text_edit_zoom_marquee_after_cmd_is_let_go() {
    use egui::{Event, Modifiers};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.prefs.placeholder_text = false;
    app.run("prefs.set", json!({"key": "animatedZoom", "value": false})).unwrap();
    let ctx = egui::Context::default();
    let id = start_typing(&mut app, &ctx);
    let rect = app.canvas_rect.unwrap();
    let at = rect.center() - vec2(100.0, 50.0);
    let space = |pressed, modifiers| Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers };
    let button = |pos, pressed, modifiers| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
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
    typing_frame(&mut app, &ctx, vec![Event::Text("a".into())]);
    let z0 = app.view().unwrap().zoom;
    // Space held since the Cmd+Space press sizes the marquee, also once Cmd is let go.
    typing_frame(&mut app, &ctx, vec![Event::ModifiersChanged(Modifiers::COMMAND), space(true, Modifiers::COMMAND), Event::PointerMoved(at)]);
    typing_frame(&mut app, &ctx, vec![button(at, true, Modifiers::COMMAND)]);
    typing_frame(&mut app, &ctx, vec![Event::PointerMoved(at + vec2(50.0, 25.0))]);
    typing_frame(&mut app, &ctx, vec![Event::ModifiersChanged(Modifiers::NONE), Event::PointerMoved(at + vec2(80.0, 40.0))]);
    typing_frame(&mut app, &ctx, vec![Event::PointerMoved(at + vec2(100.0, 50.0))]);
    assert_eq!(corners(&mut app), (at, at + vec2(100.0, 50.0)), "Space held since the press sizes it");
    // Let go and pressed again, Space moves the marquee on this frame and the frames after it.
    typing_frame(&mut app, &ctx, vec![space(false, Modifiers::NONE)]);
    typing_frame(&mut app, &ctx, vec![space(true, Modifiers::NONE), Event::Text(" ".into()), Event::PointerMoved(at + vec2(120.0, 60.0))]);
    typing_frame(&mut app, &ctx, vec![Event::PointerMoved(at + vec2(140.0, 70.0))]);
    assert_eq!(corners(&mut app), (at + vec2(40.0, 20.0), at + vec2(140.0, 70.0)), "Space moves it and keeps its 100 × 50 size");
    assert_eq!(plain(&app, id), "a", "the Space that moves the marquee types nothing");
    typing_frame(&mut app, &ctx, vec![space(false, Modifiers::NONE)]);
    typing_frame(&mut app, &ctx, vec![space(true, Modifiers::NONE), Event::Ime(egui::ImeEvent::Commit(" ".into()))]);
    assert_eq!(plain(&app, id), "a", "nor does a space an input method commits for it");
    // Let go, the drag sizes it again, and the release zooms to the moved marquee.
    typing_frame(&mut app, &ctx, vec![space(false, Modifiers::NONE), Event::PointerMoved(at + vec2(180.0, 95.0))]);
    assert_eq!(corners(&mut app), (at + vec2(40.0, 20.0), at + vec2(180.0, 95.0)), "let go, the drag sizes it again");
    typing_frame(&mut app, &ctx, vec![button(at + vec2(180.0, 95.0), false, Modifiers::NONE)]);
    let want = z0 * f64::from((rect.width() / 140.0).min(rect.height() / 75.0));
    assert!((app.view().unwrap().zoom - want).abs() < 1e-6, "{z0} → {}, not {want}", app.view().unwrap().zoom);
    assert!(app.session.tool_wants_text(), "editing goes on");
    // With no marquee, Space types a space.
    typing_frame(&mut app, &ctx, vec![space(true, Modifiers::NONE), Event::Text(" ".into())]);
    typing_frame(&mut app, &ctx, vec![space(false, Modifiers::NONE)]);
    assert_eq!(plain(&app, id), "a ", "Space types a space");
    // With Animated Zoom, Space during the Cmd+Space drag does not type a space either.
    app.run("prefs.set", json!({"key": "animatedZoom", "value": true})).unwrap();
    typing_frame(&mut app, &ctx, vec![Event::ModifiersChanged(Modifiers::COMMAND), space(true, Modifiers::COMMAND), Event::PointerMoved(at)]);
    typing_frame(&mut app, &ctx, vec![button(at, true, Modifiers::COMMAND)]);
    typing_frame(&mut app, &ctx, vec![Event::ModifiersChanged(Modifiers::NONE), space(false, Modifiers::NONE)]);
    typing_frame(&mut app, &ctx, vec![space(true, Modifiers::NONE), Event::Text(" ".into())]);
    typing_frame(&mut app, &ctx, vec![button(at, false, Modifiers::NONE)]);
    typing_frame(&mut app, &ctx, vec![space(false, Modifiers::NONE)]);
    assert_eq!(plain(&app, id), "a ", "the text is as typed");
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
    // Without Georgia on this computer, the Type tool asks first; OK starts the edit.
    if app.ui.dialog.as_ref().is_some_and(|d| d.kind == crate::dialogs::confirm::MISSING_FONT_EDIT) {
        crate::dialogs::confirm(&mut app).unwrap();
        frame(&mut app, vec![]);
    }
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

/// Double-clicking type set in a family that isn't available asks before the Type tool edits it:
/// Cancel leaves the document, its undo history and the selection as they were, with nothing
/// edited; OK puts the caret where the type was clicked, and typing edits the type, which keeps
/// the family it names.
#[test]
fn double_clicking_type_in_a_missing_font_asks_before_editing() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let params = json!({"x": 100, "y": 150, "text": "Hello world", "size": 24, "font": "Missing Test Family 7258"});
    let id = app.session.execute("text.create", &params).unwrap()["id"].as_u64().unwrap();
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
    let node = |app: &VectorcraftApp| app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
    let b = node(&app).geometric_bounds().unwrap();
    let at = xf.to_screen(b.center());
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let double_click = |app: &mut VectorcraftApp, time: f64| {
        timed(app, time, vec![egui::Event::PointerMoved(at), button(true), button(false), button(true), button(false)]);
        timed(app, time + 0.1, vec![]);
    };
    let asking = |app: &VectorcraftApp| app.ui.dialog.as_ref().filter(|d| d.kind == crate::dialogs::confirm::MISSING_FONT_EDIT).cloned();
    double_click(&mut app, 1.0);
    let d = asking(&app).expect("the question about the missing font");
    assert!(d.str("detail").contains("Missing Test Family 7258 Regular"), "{}", d.str("detail"));
    assert_eq!(app.session.tool_options()["editing"], json!(null));
    // Cancel: nothing is edited, and the document, its undo history and the selection stay.
    let (before, undo, selected) =
        (node(&app), app.session.doc().unwrap().history.undo.len(), app.session.active().unwrap().selection.objects.clone());
    crate::dialogs::cancel(&mut app);
    timed(&mut app, 1.5, vec![]);
    assert_eq!(app.session.tool_options()["editing"], json!(null));
    assert!(!app.session.tool_wants_text());
    assert_eq!(node(&app), before);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);
    assert_eq!(app.session.active().unwrap().selection.objects, selected);
    // A new double-click asks again; OK edits with the caret where the type was clicked.
    double_click(&mut app, 3.0);
    assert!(asking(&app).is_some(), "asked again");
    crate::dialogs::confirm(&mut app).unwrap();
    timed(&mut app, 3.5, vec![]);
    assert!(app.ui.dialog.is_none());
    let o = app.session.tool_options();
    assert_eq!(o["editing"], json!(id));
    let caret = o["caret"].as_u64().unwrap();
    assert!((1..11).contains(&caret), "caret {caret}");
    app.session.tool_text("X", app.view_info()).unwrap();
    let NodeKind::Text(t) = &node(&app).kind else { panic!("not text") };
    assert_eq!(t.plain_text().find('X'), Some(caret as usize), "{}", t.plain_text());
    assert!(t.runs.iter().all(|r| r.style.font_family == "Missing Test Family 7258"));
}

/// A Type tool press that reaches type in a missing font while another dialog is open (as one sent
/// with `ui.pointer` does) leaves that dialog open and starts no edit.
#[test]
fn a_press_on_type_in_a_missing_font_keeps_an_open_dialog() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let params = json!({"x": 100, "y": 150, "text": "Hello world", "size": 24, "font": "Missing Test Family 8408"});
    let id = app.session.execute("text.create", &params).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.none", &json!({})).unwrap();
    app.select_tool("type");
    let doc = app.session.active().unwrap().uid;
    crate::dialogs::confirm::ask_replace_files(&mut app, doc, &["/tmp/a.svg".to_string()], &[], "file.export", json!({}));
    let b = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap();
    let view = app.view_info();
    for kind in [PointerKind::Down, PointerKind::Up] {
        crate::canvas::dispatch(&mut app, &PointerEvent::new(kind, b.center().x, b.center().y), view);
    }
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(crate::dialogs::confirm::REPLACE_FILES));
    crate::dialogs::cancel(&mut app);
    let r = app.session.tool_poll(view, false);
    crate::canvas::apply_requests(&mut app, r);
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.session.tool_options()["editing"], json!(null));
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

/// One Escape leaves isolation mode from a sublayer two levels down, where Exit Isolation Mode goes
/// up one layer at a time.
#[test]
fn escape_leaves_an_isolated_sublayer_in_one_step() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let top = app.session.active().unwrap().doc.layers[0].id;
    let mid = app.session.execute("layer.newSublayer", &json!({"parent": top.0})).unwrap()["id"].as_u64().unwrap();
    let deep = app.session.execute("layer.newSublayer", &json!({"parent": mid})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("layer.setCurrent", &json!({"id": deep})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 20})).unwrap();
    app.session.execute("object.isolate", &json!({"id": deep})).unwrap();
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
    let esc = |pressed| egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    timed(&mut app, 0.0, vec![]);
    assert_eq!(app.session.active().unwrap().isolation, Some(vectorcraft_doc::NodeId(deep)));
    timed(&mut app, 1.0, vec![esc(true), esc(false)]);
    timed(&mut app, 1.1, vec![]);
    assert_eq!(app.session.active().unwrap().isolation, None, "one Escape leaves every level");
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
    let document = app.session.active().unwrap().uid;
    egui::DragAndDrop::set_payload(&ctx, widgets::PanelDrag::Symbol { document, name: name.clone() });
    let up = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() };
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), up]);
    let st = app.session.active().unwrap();
    let placed = st.selection.objects.first().copied().unwrap();
    let n = st.doc.node(placed).unwrap();
    assert!(matches!(&n.kind, NodeKind::SymbolInstance { symbol, .. } if *symbol == name));
    let c = n.geometric_bounds().unwrap().center();
    assert!((c - xf.to_doc(at)).hypot() < 1e-6, "{c:?}");
}

/// A new active document with a symbol `name` made from a `w` by `h` rectangle and no art; → its
/// uid.
fn document_with_symbol(app: &mut VectorcraftApp, name: &str, w: f64, h: f64) -> u64 {
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": w, "height": h})).unwrap();
    app.session.execute("symbol.new", &json!({"name": name})).unwrap();
    app.session.execute("edit.clear", &json!({})).unwrap();
    app.session.active().unwrap().uid
}

/// The natural size the active document records for symbol `name`.
fn symbol_size(app: &VectorcraftApp, name: &str) -> (f64, f64) {
    let st = app.session.active().unwrap();
    let size = st.doc.unknown.get(vectorcraft_doc::SYMBOL_SIZES).and_then(|m| m.get(name)).cloned().unwrap();
    (size[0].as_f64().unwrap(), size[1].as_f64().unwrap())
}

/// A symbol dragged from one document's Symbols panel and released on the canvas after another
/// document with a different symbol of the same name became active places the dragged symbol: it
/// is added under a free name, the other document's symbol is left as it was, the source document
/// is not changed, and one Undo takes back both the symbol and the instance.
#[test]
fn a_symbol_dragged_from_another_document_places_that_documents_symbol() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let source = document_with_symbol(&mut app, "Leaf", 40.0, 20.0);
    let ctx = egui::Context::default();
    // While Leaf is held, a document with a Leaf of its own becomes active.
    egui::DragAndDrop::set_payload(&ctx, widgets::PanelDrag::Symbol { document: source, name: "Leaf".into() });
    document_with_symbol(&mut app, "Leaf", 10.0, 70.0);
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let undo = app.session.active().unwrap().history.undo.len();
    let at = xf.to_screen(Point::new(250.0, 180.0));
    let up = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() };
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), up]);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.symbols.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Leaf", "Leaf 2"]);
    assert_eq!(st.history.undo.len(), undo + 1, "added and placed as one undo step");
    let placed = st.selection.objects.first().copied().expect("the placed instance is selected");
    let n = st.doc.node(placed).unwrap();
    assert!(matches!(&n.kind, NodeKind::SymbolInstance { symbol, .. } if symbol == "Leaf 2"), "{:?}", n.kind);
    let b = n.geometric_bounds().unwrap();
    assert!((b.width() - 40.0).abs() < 1e-6 && (b.height() - 20.0).abs() < 1e-6, "the dragged symbol's size: {b:?}");
    assert!((b.center() - xf.to_doc(at)).hypot() < 1e-6, "{b:?}");
    assert_eq!(symbol_size(&app, "Leaf"), (10.0, 70.0), "this document's Leaf is unchanged");
    app.run("edit.undo", json!({})).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.symbols.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Leaf"]);
    assert!(st.doc.node(placed).is_none());
    assert!(app.session.set_active(0));
    assert_eq!(app.session.active().unwrap().uid, source);
    assert_eq!(app.session.active().unwrap().doc.symbols.len(), 1, "the source document is unchanged");
    assert_eq!(symbol_size(&app, "Leaf"), (40.0, 20.0));
}

/// A symbol dragged from a document that was closed before the release places nothing, even
/// where the active document has a symbol of the same name.
#[test]
fn a_symbol_whose_document_closed_places_nothing() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let source = document_with_symbol(&mut app, "Leaf", 40.0, 20.0);
    document_with_symbol(&mut app, "Leaf", 10.0, 70.0);
    app.session.execute("file.close", &json!({"index": 0})).unwrap();
    assert!(app.session.documents().iter().all(|d| d.uid != source));
    let ctx = egui::Context::default();
    egui::DragAndDrop::set_payload(&ctx, widgets::PanelDrag::Symbol { document: source, name: "Leaf".into() });
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let undo = app.session.active().unwrap().history.undo.len();
    let at = xf.to_screen(Point::new(250.0, 180.0));
    let up = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() };
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), up]);
    let st = app.session.active().unwrap();
    assert_eq!(st.history.undo.len(), undo, "no edit");
    assert!(st.selection.objects.is_empty(), "nothing placed");
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
fn a_brush_with_art_dropped_off_the_art_places_a_copy_of_its_art_there() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("path.create", &json!({"anchors": [{"x": 100, "y": 150}, {"x": 300, "y": 150}]})).unwrap();
    app.session.execute("select.none", &json!({})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let drop = |app: &mut VectorcraftApp, name: &str, at: Point| {
        let def = app.session.execute("brush.get", &json!({"name": name})).unwrap();
        let at = xf.to_screen(at);
        egui::DragAndDrop::set_payload(&ctx, widgets::PanelDrag::Brush { name: name.into(), def });
        let up = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() };
        frame(app, &ctx, vec![egui::Event::PointerMoved(at), up]);
    };
    let undo = app.session.active().unwrap().history.undo.len();
    // A calligraphic brush has no art: nothing.
    drop(&mut app, "3 pt. Round", Point::new(200.0, 250.0));
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
    // An art brush: a copy of its art, centered where it lands, selected.
    drop(&mut app, "Arrow", Point::new(200.0, 250.0));
    let st = app.session.active().unwrap();
    assert_eq!(st.history.undo.len(), undo + 1);
    assert_eq!(st.history.undo.last().unwrap().label, "Place Brush Art");
    let [placed] = st.selection.objects.as_slice() else { panic!("one object selected: {:?}", st.selection.objects) };
    let b = st.doc.node(*placed).unwrap().geometric_bounds().unwrap();
    assert!((b.center() - Point::new(200.0, 250.0)).hypot() < 0.1, "{b:?}");
    // A scatter brush and a pattern brush (its tiles, in one group) place their art too.
    for (i, name) in ["Dots", "Chain"].into_iter().enumerate() {
        let at = Point::new(80.0 + 200.0 * i as f64, 60.0);
        drop(&mut app, name, at);
        let st = app.session.active().unwrap();
        assert_eq!(st.history.undo.len(), undo + 2 + i, "{name}");
        let [placed] = st.selection.objects.as_slice() else { panic!("{name}: one object selected: {:?}", st.selection.objects) };
        let n = st.doc.node(*placed).unwrap();
        assert!((n.geometric_bounds().unwrap().center() - at).hypot() < 0.1, "{name}");
        if name == "Chain" {
            assert_eq!(n.children().unwrap()[0].name.as_deref(), Some("Side Tile"));
        }
    }
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
fn a_color_dropped_on_a_selected_mesh_recolors_the_point_or_patch_under_it() {
    use vectorcraft_color::{Color, Paint};
    use vectorcraft_doc::{GradientMesh, NodeId, NodeKind};
    use widgets::PanelDrag;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    let id = NodeId(id);
    app.session.execute("object.mesh.create", &json!({"ids": [id.0], "rows": 2, "columns": 2})).unwrap();
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
    let mesh = |app: &VectorcraftApp| -> GradientMesh {
        match &app.session.active().unwrap().doc.node(id).unwrap().kind {
            NodeKind::Mesh(m) => m.clone(),
            _ => panic!("not a gradient mesh"),
        }
    };
    let colors = |app: &VectorcraftApp| mesh(app).points.iter().map(|q| q.color).collect::<Vec<_>>();
    let undo_len = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
    let before = colors(&app);
    let m = mesh(&app);
    let (center, patch) = (m.points[4].p, Point::new((m.points[0].p.x + m.points[4].p.x) / 2.0, (m.points[0].p.y + m.points[4].p.y) / 2.0));
    let (red, blue) = (Color::rgb(1.0, 0.0, 0.0), Color::rgb(0.0, 0.0, 1.0));
    app.session.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    // On a mesh point: that point alone.
    drop(&mut app, PanelDrag::paint(Paint::solid(red)), center);
    let after = colors(&app);
    assert_eq!(after[4], red);
    assert_eq!((0..9).filter(|i| after[*i] != before[*i]).count(), 1, "only the point under the pointer");
    // Inside a patch: its four points.
    drop(&mut app, PanelDrag::paint(Paint::solid(blue)), patch);
    let after = colors(&app);
    assert_eq!([0, 1, 3, 4].map(|i| after[i]), [blue; 4]);
    assert_eq!([2, 5, 6, 7, 8].map(|i| after[i]), [2, 5, 6, 7, 8].map(|i| before[i]));
    assert!(app.session.active().unwrap().selection.contains(id), "the selection stays");
    // The same color again changes nothing and records no undo step.
    let n = undo_len(&app);
    drop(&mut app, PanelDrag::paint(Paint::solid(blue)), patch);
    assert_eq!(undo_len(&app), n);
    // With Object Selection by Path Only, a drop inside a patch still reaches its points, and a
    // drop 4 screen pixels outside a corner point still reaches that point.
    app.session.execute("prefs.set", &json!({"key": "objectSelectionByPathOnly", "value": true})).unwrap();
    let green = Color::rgb(0.0, 1.0, 0.0);
    let far = Point::new((m.points[4].p.x + m.points[8].p.x) / 2.0, (m.points[4].p.y + m.points[8].p.y) / 2.0);
    // Art below the patch, with its outline within the selection tolerance, is not taken for it.
    let h = 1.0 / xf.zoom;
    let below = app.session.execute("shape.rectangle", &json!({"x": far.x - h, "y": far.y - h, "width": 2.0 * h, "height": 2.0 * h})).unwrap()["id"]
        .as_u64()
        .unwrap();
    app.session.execute("object.arrange.sendToBack", &json!({"ids": [below]})).unwrap();
    app.session.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    drop(&mut app, PanelDrag::paint(Paint::solid(green)), far);
    assert_eq!([4, 5, 7, 8].map(|i| colors(&app)[i]), [green; 4]);
    assert_ne!(app.session.active().unwrap().doc.node(NodeId(below)).unwrap().appearance.fill_paint(), Paint::solid(green));
    let off = 4.0 / xf.zoom;
    drop(&mut app, PanelDrag::paint(Paint::solid(red)), Point::new(m.points[0].p.x - off, m.points[0].p.y));
    assert_eq!(colors(&app)[0], red, "the corner point within reach");
    assert_eq!(colors(&app)[1], blue);
    // An unselected object on top of the mesh takes the drop itself.
    app.session.execute("prefs.set", &json!({"key": "objectSelectionByPathOnly", "value": false})).unwrap();
    let c = m.points[4].p;
    let cover = app.session.execute("shape.rectangle", &json!({"x": c.x - 10.0, "y": c.y - 10.0, "width": 20, "height": 20})).unwrap()["id"]
        .as_u64()
        .unwrap();
    app.session.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let before = colors(&app);
    drop(&mut app, PanelDrag::paint(Paint::solid(green)), c);
    assert_eq!(colors(&app), before, "the mesh under it keeps its colors");
    assert_eq!(app.session.active().unwrap().doc.node(NodeId(cover)).unwrap().appearance.fill_paint(), Paint::solid(green));
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

/// Art Alt-dragged off the canvas (a copy while it moves there) reaches the panels as the art the
/// drag started with, which is what stays when the release over a panel takes the copy back.
#[test]
fn art_alt_dragged_off_the_canvas_reaches_the_panels_as_the_original() {
    use egui::{Event, Modifiers};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    let id = vectorcraft_doc::NodeId(id);
    app.select_tool("selection");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let before = app.session.active().unwrap().doc.clone();
    let button = |pos: Pos2, pressed: bool| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::ALT };
    let start = xf.to_screen(Point::new(100.0, 100.0));
    frame(&mut app, &ctx, vec![Event::ModifiersChanged(Modifiers::ALT), Event::PointerMoved(start), button(start, true)]);
    frame(&mut app, &ctx, vec![Event::PointerMoved(start + vec2(40.0, 0.0))]);
    let st = app.session.active().unwrap();
    assert_eq!(st.interaction.as_ref().map(|i| i.label.as_str()), Some("Copy"));
    assert!(!st.selection.objects.contains(&id), "the copy is selected while it moves");
    frame(&mut app, &ctx, vec![Event::PointerMoved(pos2(850.0, 100.0))]);
    assert_eq!(egui::DragAndDrop::payload::<widgets::PanelDrag>(&ctx).as_deref(), Some(&widgets::PanelDrag::Art(vec![id])));
    frame(&mut app, &ctx, vec![button(pos2(850.0, 100.0), false)]);
    let st = app.session.active().unwrap();
    assert!(st.interaction.is_none());
    assert_eq!(*st.doc, *before, "the copy goes, the original stays");
    assert!(st.doc.node(id).is_some());
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

/// Starts editing new point type with the Type tool; returns its id.
fn start_typing(app: &mut VectorcraftApp, ctx: &egui::Context) -> u64 {
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    typing_frame(app, ctx, vec![]);
    app.select_tool("type");
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let at = xf.to_screen(Point::new(100.0, 100.0));
    let click =
        |p: Pos2, pressed| egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    typing_frame(app, ctx, vec![egui::Event::PointerMoved(at), click(at, true)]);
    typing_frame(app, ctx, vec![click(at, false)]);
    assert!(app.session.tool_wants_text());
    app.session.active().unwrap().selection.objects[0].0
}

/// Returns a key press and its release.
fn tap(key: egui::Key) -> [egui::Event; 2] {
    [true, false].map(|pressed| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Default::default() })
}

/// Arrows, Home, End, Tab, Enter and clipboard events go to the Type tool in order with the text
/// of the same frame, and each Enter types a line break.
#[test]
fn the_type_tool_takes_text_and_editing_keys_in_the_order_they_came() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.prefs.placeholder_text = false;
    let ctx = egui::Context::default();
    let id = start_typing(&mut app, &ctx);
    let text = |t: &str| egui::Event::Text(t.into());
    typing_frame(&mut app, &ctx, vec![text("ax")]);
    // ← moves the caret before the text that follows it goes in.
    typing_frame(&mut app, &ctx, [tap(egui::Key::ArrowLeft).to_vec(), vec![text("b")]].concat());
    assert_eq!(plain(&app, id), "abx");
    // Home and End move the caret before the text that follows each goes in.
    typing_frame(&mut app, &ctx, [tap(egui::Key::Home).to_vec(), vec![text("0")], tap(egui::Key::End).to_vec(), vec![text("9")]].concat());
    assert_eq!(plain(&app, id), "0abx9");
    // Tab and a paste go in between the texts around them.
    let paste = egui::Event::Paste("p".into());
    typing_frame(&mut app, &ctx, [vec![text("t")], tap(egui::Key::Tab).to_vec(), vec![text("u"), paste, text("q")]].concat());
    assert_eq!(plain(&app, id), "0abx9t\tupq");
    // Enter types its line break between the texts around it, once for each press.
    typing_frame(&mut app, &ctx, [vec![text("1")], tap(egui::Key::Enter).to_vec(), tap(egui::Key::Enter).to_vec(), vec![text("2")]].concat());
    assert_eq!(plain(&app, id), "0abx9t\tupq1\n\n2");
    // A key that comes before a composition starts in the same frame still moves the caret.
    typing_frame(&mut app, &ctx, [tap(egui::Key::Home).to_vec(), vec![preedit("k", 1)]].concat());
    typing_frame(&mut app, &ctx, vec![preedit("", 0), egui::Event::Ime(egui::ImeEvent::Commit("k".into()))]);
    assert_eq!(plain(&app, id), "0abx9t\tupq1\n\nk2");
}

/// Enter keeps each key event's modifiers while sharing a frame with typed text.
#[test]
fn shift_enter_types_a_soft_return_in_order_with_plain_enter() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.prefs.placeholder_text = false;
    let ctx = egui::Context::default();
    let id = start_typing(&mut app, &ctx);
    let text = |t: &str| egui::Event::Text(t.into());
    let enter =
        |modifiers| [true, false].map(|pressed| egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed, repeat: false, modifiers });
    typing_frame(
        &mut app,
        &ctx,
        [vec![text("a")], enter(egui::Modifiers::SHIFT).to_vec(), vec![text("b")], enter(egui::Modifiers::NONE).to_vec(), vec![text("c")]].concat(),
    );
    assert_eq!(plain(&app, id), "a\u{2028}b\nc");
}

/// Within one frame, Home between two texts, Tab inside a line, and Backspace after a paste each
/// act on the text as the events before them left it.
#[test]
fn home_tab_and_backspace_act_on_the_text_before_them_in_the_frame() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.prefs.placeholder_text = false;
    let ctx = egui::Context::default();
    let id = start_typing(&mut app, &ctx);
    let text = |t: &str| egui::Event::Text(t.into());
    typing_frame(&mut app, &ctx, vec![text("abc")]);
    typing_frame(&mut app, &ctx, [vec![text("d")], tap(egui::Key::Home).to_vec(), vec![text("0")]].concat());
    assert_eq!(plain(&app, id), "0abcd");
    typing_frame(&mut app, &ctx, [tap(egui::Key::ArrowRight).to_vec(), tap(egui::Key::Tab).to_vec(), vec![text("t")]].concat());
    assert_eq!(plain(&app, id), "0a\ttbcd");
    typing_frame(&mut app, &ctx, [tap(egui::Key::End).to_vec(), vec![egui::Event::Paste("pq".into())], tap(egui::Key::Backspace).to_vec()].concat());
    assert_eq!(plain(&app, id), "0a\ttbcdp");
}

/// Cmd/Ctrl+Enter bound to a command runs the command and types no line break, also when a plain
/// Enter comes in the same frame. Unbound, it ends editing and keeps the text.
#[test]
fn a_command_on_cmd_enter_runs_without_typing_a_line_break() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.prefs.placeholder_text = false;
    let ctx = egui::Context::default();
    let id = start_typing(&mut app, &ctx);
    let cmd_enter = [true, false].map(|pressed| egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    typing_frame(&mut app, &ctx, vec![egui::Event::Text("a".into())]);
    crate::keymap::set_override(&mut app.ui.shortcut_overrides, "view.grid", "Cmd+Enter");
    crate::shortcut_editor::sync(&app.ui);
    let grid = app.ui.view.grid;
    typing_frame(&mut app, &ctx, cmd_enter.to_vec());
    let ran = (plain(&app, id), app.ui.view.grid, app.session.tool_wants_text());
    typing_frame(&mut app, &ctx, [tap(egui::Key::Enter).to_vec(), cmd_enter.to_vec()].concat());
    app.ui.shortcut_overrides.clear();
    crate::shortcut_editor::sync(&app.ui);
    assert_eq!((ran.0.as_str(), ran.1, ran.2), ("a", !grid, true), "Cmd+Enter runs Show Grid only");
    assert_eq!((plain(&app, id).as_str(), app.ui.view.grid), ("a\n", grid), "Enter types a line break and Cmd+Enter runs Show Grid");
    assert!(app.session.tool_wants_text(), "a bound Cmd+Enter does not end editing");
    typing_frame(&mut app, &ctx, cmd_enter.to_vec());
    assert_eq!(plain(&app, id), "a\n", "nothing is bound to Cmd+Enter: no line break");
    assert!(!app.session.tool_wants_text(), "unbound, Cmd+Enter ends editing");
    assert_eq!(app.session.tool_id(), "selection");
}

/// Cmd+Enter (Ctrl+Enter on Windows and Linux) ends editing like Escape: a repeat of it, and text,
/// keys and a paste after it in the same frame, leave the type as it was. Enter, Shift+Enter and
/// Ctrl+Enter on macOS still type line breaks.
#[test]
fn cmd_enter_ends_text_editing_and_keeps_the_text() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.prefs.placeholder_text = false;
    let ctx = egui::Context::default();
    let id = start_typing(&mut app, &ctx);
    let key = |pressed, repeat, modifiers| egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed, repeat, modifiers };
    let mac_ctrl = egui::Modifiers { ctrl: true, ..Default::default() };
    typing_frame(&mut app, &ctx, vec![egui::Event::Text("a".into())]);
    typing_frame(&mut app, &ctx, vec![key(true, false, egui::Modifiers::NONE), key(false, false, egui::Modifiers::NONE)]);
    typing_frame(&mut app, &ctx, vec![key(true, false, egui::Modifiers::SHIFT), key(false, false, egui::Modifiers::SHIFT)]);
    typing_frame(&mut app, &ctx, vec![key(true, false, mac_ctrl), key(false, false, mac_ctrl)]);
    assert_eq!(plain(&app, id), "a\n\u{2028}\n");
    assert!(app.session.tool_wants_text());
    let held = vec![
        key(true, false, egui::Modifiers::COMMAND),
        key(true, true, egui::Modifiers::COMMAND),
        egui::Event::Text("b".into()),
        egui::Event::Key { key: egui::Key::Backspace, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() },
        egui::Event::Paste("c".into()),
        egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() },
    ];
    typing_frame(&mut app, &ctx, held);
    assert!(!app.session.tool_wants_text(), "editing ended");
    assert_eq!(app.session.tool_id(), "selection");
    assert_eq!(plain(&app, id), "a\n\u{2028}\n", "the text is kept, and nothing after the chord changes it");
    assert_eq!(app.session.active().unwrap().selection.objects, vec![vectorcraft_doc::NodeId(id)], "the type stays selected");
    typing_frame(&mut app, &ctx, vec![key(true, true, egui::Modifiers::COMMAND)]);
    typing_frame(&mut app, &ctx, vec![key(false, false, egui::Modifiers::COMMAND)]);
    assert!(app.ui.dialog.is_none(), "the repeats of the chord open no dialog");
    assert_eq!(plain(&app, id), "a\n\u{2028}\n");
}

/// While the IME composes, Cmd+Enter is the IME's: it does not end editing, drop the marked text or
/// type a line break, in the frame that starts the composition or in a later one.
#[test]
fn cmd_enter_while_composing_keeps_the_composition_and_editing() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.prefs.placeholder_text = false;
    let ctx = egui::Context::default();
    let id = start_typing(&mut app, &ctx);
    let cmd_enter = [true, false].map(|pressed| egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    typing_frame(&mut app, &ctx, vec![egui::Event::Text("a".into())]);
    typing_frame(&mut app, &ctx, [vec![preedit("か", 1)], cmd_enter.to_vec()].concat());
    assert!(app.session.tool_composing(), "the same frame keeps the composition");
    assert!(app.session.tool_wants_text(), "the same frame keeps editing");
    assert_eq!(plain(&app, id), "aか", "the same frame keeps the marked text and types no line break");
    typing_frame(&mut app, &ctx, cmd_enter.to_vec());
    assert!(app.session.tool_composing(), "a later frame keeps the composition");
    assert!(app.session.tool_wants_text(), "a later frame keeps editing");
    assert_eq!(plain(&app, id), "aか", "a later frame keeps the marked text and types no line break");
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

/// #4711: Shift pressed and released while a corner handle is held still scales proportionally and
/// then freely again at once, without waiting for the pointer to move.
#[test]
fn a_modifier_changed_mid_scale_applies_with_the_pointer_held_still() {
    use egui::{Event, Modifiers};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.ui.view.smart_guides = false;
    app.ui.view.snap_to_point = false;
    let id = app.session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 50})).unwrap()["id"].as_u64().unwrap();
    app.select_tool("selection");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let with = |app: &mut VectorcraftApp, mut events: Vec<Event>, modifiers: Modifiers| {
        events.insert(0, Event::ModifiersChanged(modifiers));
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
    };
    let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    let height =
        |app: &VectorcraftApp| app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap().height();
    let corner = xf.to_screen(Point::new(200.0, 150.0));
    let to = xf.to_screen(Point::new(300.0, 150.0));
    with(&mut app, vec![Event::PointerMoved(corner)], Modifiers::NONE);
    with(&mut app, vec![button(corner, true)], Modifiers::NONE);
    with(&mut app, vec![Event::PointerMoved(to)], Modifiers::NONE);
    assert!((height(&app) - 50.0).abs() < 1e-6, "a free scale keeps the height: {}", height(&app));
    with(&mut app, vec![], Modifiers::SHIFT);
    assert!((height(&app) - 100.0).abs() < 1e-6, "Shift scales proportionally at once: {}", height(&app));
    with(&mut app, vec![], Modifiers::NONE);
    assert!((height(&app) - 50.0).abs() < 1e-6, "releasing Shift frees it at once: {}", height(&app));
    with(&mut app, vec![button(to, false)], Modifiers::NONE);
    assert!(app.session.active().unwrap().interaction.is_none(), "committed");
}

/// Backtick switches a moved Selection drag's live preview while the pointer stays still.
#[test]
fn backtick_changed_mid_selection_drag_applies_with_the_pointer_held_still() {
    use egui::Event;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let tile = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    app.run("object.pattern.make", json!({"ids": [tile], "name": "Dots", "width": 20, "height": 20})).unwrap();
    app.run("object.pattern.done", json!({})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
    app.run("paint.setFill", json!({"ids": [id], "swatch": "Dots"})).unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.select_tool("selection");
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    app.ui.view.smart_guides = false;
    app.ui.view.snap_to_point = false;
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let from = xf.to_screen(Point::new(125.0, 125.0));
    let to = xf.to_screen(Point::new(145.0, 125.0));
    let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let key = |pressed| Event::Key { key: egui::Key::Backtick, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    let node = |app: &VectorcraftApp| app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
    let pattern_x = |node: &vectorcraft_doc::Node| {
        let vectorcraft_color::Paint::Pattern { xf, .. } = node.appearance.fill_paint() else { panic!("pattern fill expected") };
        xf.as_coeffs()[4]
    };
    let before = node(&app);
    frame(&mut app, &ctx, vec![Event::PointerMoved(from), button(from, true)]);
    frame(&mut app, &ctx, vec![Event::PointerMoved(to)]);
    let moved = node(&app);
    assert!((moved.geometric_bounds().unwrap().x0 - before.geometric_bounds().unwrap().x0 - 20.0).abs() < 0.001);
    frame(&mut app, &ctx, vec![key(true)]);
    let pattern_only = node(&app);
    assert_eq!(pattern_only.geometric_bounds(), before.geometric_bounds(), "Backtick restores geometry in the live preview");
    assert!((pattern_x(&pattern_only) - pattern_x(&before) - 20.0).abs() < 0.001, "Backtick moves the pattern in the live preview");
    frame(&mut app, &ctx, vec![key(false)]);
    let ordinary = node(&app);
    assert_eq!(ordinary.geometric_bounds(), moved.geometric_bounds(), "releasing Backtick restores the ordinary move in the live preview");
    assert!((pattern_x(&ordinary) - pattern_x(&before)).abs() < 0.001, "the ordinary move restores the pattern matrix");
    frame(&mut app, &ctx, vec![button(to, false)]);
}

/// Backtick pressed while a flare's rings' end point is placed places the rings anew, with the
/// pointer held still.
#[test]
fn backtick_places_a_flares_rings_anew_with_the_pointer_held_still() {
    use egui::Event;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.select_tool("flare");
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    app.ui.view.smart_guides = false;
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let from = xf.to_screen(Point::new(100.0, 100.0));
    let to = xf.to_screen(Point::new(130.0, 100.0));
    let end = xf.to_screen(Point::new(180.0, 160.0));
    assert!(app.canvas_rect.unwrap().contains(end));
    let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let key = |pressed| Event::Key { key: egui::Key::Backtick, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    let rings = |app: &VectorcraftApp| {
        let st = app.session.doc().unwrap();
        let g = st.doc.node(st.selection.objects[0]).unwrap();
        let vectorcraft_doc::NodeKind::Group { children, .. } = &g.kind else { panic!("flare is a group") };
        children.iter().filter(|c| c.name.as_deref() == Some("Ring")).map(|c| c.geometric_bounds().unwrap()).collect::<Vec<_>>()
    };
    // Frames a sixtieth of a second apart while the pointer moves, and a second apart with it
    // still, so it counts as still once it stops.
    let mut time = 0.0;
    let mut step = |app: &mut VectorcraftApp, events: Vec<Event>| {
        time += if events.iter().any(|e| matches!(e, Event::PointerMoved(_))) { 1.0 / 60.0 } else { 1.0 };
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
    };
    step(&mut app, vec![Event::PointerMoved(from), button(from, true)]);
    step(&mut app, vec![Event::PointerMoved(to)]);
    step(&mut app, vec![button(to, false)]);
    step(&mut app, vec![Event::PointerMoved(end - vec2(5.0, 5.0))]);
    step(&mut app, vec![Event::PointerMoved(end)]);
    step(&mut app, vec![]);
    step(&mut app, vec![]);
    let before = rings(&app);
    assert_eq!(before.len(), 10);
    assert!(before.iter().all(|r| r.center().y > 90.0 && r.center().x > 90.0), "the rings run towards the pointer: {before:?}");
    step(&mut app, vec![key(true)]);
    let after = rings(&app);
    assert_eq!(after.len(), 10);
    assert_ne!(after, before, "~ places the rings anew");
    step(&mut app, vec![key(false)]);
    assert_eq!(rings(&app), after, "letting ~ go keeps the placement");
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
    Some(crate::stylus::PenSample { pressure, eraser: true, ..Default::default() })
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
    // Right to left: the frame's margin, More Options, Shape Properties (Lock's place), Recolor,
    // then Stroke and Fill.
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
    // Right to left: the frame's margin, More Options, Shape Properties (Lock's place), Recolor,
    // then Stroke and Fill.
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
        // Right to left: the frame's margin, More Options, Shape Properties (Lock's place), then Recolor.
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

/// Recolor Artwork shows in the bars when any selected object has a color, whichever of them it is,
/// and not for an empty selection or ids that are gone.
#[test]
fn has_colors_looks_at_every_selected_object() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let mut rect = |x: i32| {
        vectorcraft_doc::NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap())
    };
    let (blank, colored) = (rect(10), rect(80));
    s.execute("paint.setFill", &json!({"ids": [blank.0], "none": true})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [blank.0], "none": true})).unwrap();
    s.execute("paint.setFill", &json!({"ids": [colored.0], "color": "#cc3322"})).unwrap();
    let doc = &s.active().unwrap().doc;
    let gone = vectorcraft_doc::NodeId(u64::MAX);
    assert!(!has_colors(doc, &[]), "nothing selected");
    assert!(!has_colors(doc, &[gone]), "an id that is gone");
    assert!(!has_colors(doc, &[blank, gone]));
    assert!(has_colors(doc, &[blank, colored]), "the color is on the second object");
    assert!(has_colors(doc, &[colored, blank, gone]), "the color is on the first object");
}

/// The Contextual Task Bar's Lock button locks the selected member in isolation mode and the
/// selected group outside isolation mode.
#[test]
fn the_task_bar_lock_button_locks_selected_art_in_isolation_mode() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    // Plain paths: a basic shape shows its properties in Lock's place.
    let square =
        |x: f64| json!({"anchors": [{"x": x, "y": 130}, {"x": x + 40.0, "y": 130}, {"x": x + 40.0, "y": 170}, {"x": x, "y": 170}], "closed": true});
    let a = app.session.execute("path.create", &square(150.0)).unwrap()["id"].clone();
    let b = app.session.execute("path.create", &square(210.0)).unwrap()["id"].clone();
    app.session.execute("select.set", &json!({"ids": [a.clone(), b.clone()]})).unwrap();
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
    assert!(locked(&app, &a), "the selected member is locked in isolation mode");
    assert!(!locked(&app, &b), "the other member stays unlocked");
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
    {
        let __dyn: image::DynamicImage = img.into();
        let (__w, __h) = (__dyn.width(), __dyn.height());
        let (__layout, __sample, __raw) = match __dyn {
            image::DynamicImage::ImageLuma8(b) => (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLumaA8(b) => (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgb8(b) => (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgba8(b) => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLuma16(b) => (
                craft_image_codecs::ChannelLayout::Gray,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageLumaA16(b) => (
                craft_image_codecs::ChannelLayout::GrayA,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgb16(b) => (
                craft_image_codecs::ChannelLayout::Rgb,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgba16(b) => (
                craft_image_codecs::ChannelLayout::Rgba,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            other => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, other.into_rgba8().into_raw()),
        };
        let __cimg = craft_image_codecs::Image::from_raw(__w, __h, __layout, __sample, __raw).unwrap();
        craft_image_codecs::encode(&__cimg, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default())
            .map(|__b| png.extend_from_slice(&__b))
    }
    .unwrap();
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

/// For a selected image the Contextual Task Bar offers Remove Background, which clears the
/// background in the image's pixels at once (no dialog) with the command's defaults.
#[test]
fn the_task_bar_removes_a_selected_images_background() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let img =
        image::RgbaImage::from_fn(20, 20, |x, y| image::Rgba(if (6..14).contains(&x) && (6..14).contains(&y) { [200, 0, 0, 255] } else { [255; 4] }));
    let mut png = vec![];
    {
        let __dyn: image::DynamicImage = img.into();
        let (__w, __h) = (__dyn.width(), __dyn.height());
        let (__layout, __sample, __raw) = match __dyn {
            image::DynamicImage::ImageLuma8(b) => (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLumaA8(b) => (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgb8(b) => (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgba8(b) => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLuma16(b) => (
                craft_image_codecs::ChannelLayout::Gray,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageLumaA16(b) => (
                craft_image_codecs::ChannelLayout::GrayA,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgb16(b) => (
                craft_image_codecs::ChannelLayout::Rgb,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgba16(b) => (
                craft_image_codecs::ChannelLayout::Rgba,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            other => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, other.into_rgba8().into_raw()),
        };
        let __cimg = craft_image_codecs::Image::from_raw(__w, __h, __layout, __sample, __raw).unwrap();
        craft_image_codecs::encode(&__cimg, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default())
            .map(|__b| png.extend_from_slice(&__b))
    }
    .unwrap();
    let data = vectorcraft_format::base64_encode(&png);
    app.run("file.place", json!({"name": "dot.png", "dataBase64": data, "link": false, "at": [200, 150]})).unwrap();
    app.select_tool("selection");
    let ctx = egui::Context::default();
    let labels = task_bar_labels(&mut app, &ctx);
    let at = labels.iter().find(|(t, _)| t == "Remove Background").map(|(_, r)| r.center()).expect("Remove Background is offered");
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
    frame(&mut app, &ctx, vec![button(true)]);
    frame(&mut app, &ctx, vec![button(false)]);
    assert!(app.ui.dialog.is_none(), "it runs at once");
    let st = app.session.active().unwrap();
    let Some(NodeKind::Image(im)) = st.selection.objects.first().and_then(|id| st.doc.node(*id)).map(|n| &n.kind) else { panic!("an image") };
    let px = craft_image_codecs::decode(&st.doc.images[&im.key].bytes)
        .map(|__cimg| {
            let (__w, __h) = __cimg.dimensions();
            match (__cimg.layout(), __cimg.sample_type()) {
                (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                _ => image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(__w, __h, __cimg.to_rgba8()).unwrap()),
            }
        })
        .unwrap()
        .to_rgba8();
    assert_eq!((px.get_pixel(0, 0).0[3], px.get_pixel(10, 10).0[3]), (0, 255), "the white background is cleared, the red dot kept");
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
/// with nothing in the document the blank canvas actions.
#[test]
fn the_task_bar_offers_the_artboard_tools_actions() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    app.select_tool("selection");
    let ctx = egui::Context::default();
    let names = |app: &mut VectorcraftApp| task_bar_labels(app, &ctx).into_iter().map(|(t, _)| t).collect::<Vec<_>>();
    assert_eq!(names(&mut app), ["Draw (Pencil)", "Place Image"], "a blank canvas");

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

/// The names and places of the buttons the Contextual Task Bar draws, from the accessibility tree
/// (`ctx` has it enabled): an icon-only button is named by its hover text.
fn task_bar_buttons(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<(String, egui::Rect)> {
    frame(app, ctx, vec![]);
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| show(app, ui));
    out.textures_delta.clear();
    let Some(bar) = task_bar_rect(ctx) else { return vec![] };
    let Some(update) = out.platform_output.accesskit_update.as_ref() else { return vec![] };
    let mut v: Vec<(String, egui::Rect)> = update
        .nodes
        .iter()
        .filter(|(_, n)| n.role() == egui::accesskit::Role::Button)
        .filter_map(|(_, n)| {
            let b = n.bounds()?;
            let r = egui::Rect::from_min_max(Pos2::new(b.x0 as f32, b.y0 as f32), Pos2::new(b.x1 as f32, b.y1 as f32));
            let label = n.label()?;
            bar.contains(r.center()).then(|| (label.to_string(), r))
        })
        .collect();
    v.sort_by(|a, b| a.1.left().total_cmp(&b.1.left()));
    v
}

/// With anchors direct-selected, the Contextual Task Bar offers Remove Anchor Points, Connect,
/// Cut Path and the two convert buttons, which show their icons alone. Converting to smooth and to
/// corner changes the selected anchor, Cut Path opens the path there and Connect closes it again,
/// as the Control bar's anchor buttons do.
#[test]
fn the_task_bar_connects_cuts_and_converts_direct_selected_anchors() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 150, "y": 100, "width": 100, "height": 80})).unwrap()["id"].as_u64().unwrap();
    app.select_tool("directSelection");
    app.run("select.anchors", json!({"id": id, "anchors": [[0, 1]]})).unwrap();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let names = |app: &mut VectorcraftApp| task_bar_labels(app, &ctx).into_iter().map(|(t, _)| t).collect::<Vec<_>>();
    assert_eq!(names(&mut app), ["Remove Anchor Points", "Connect", "Cut Path", "Duplicate"]);
    let buttons = task_bar_buttons(&mut app, &ctx).into_iter().map(|(t, _)| t).collect::<Vec<_>>();
    let expected = [
        "Remove Anchor Points",
        "Connect",
        "Cut Path",
        "Convert Selected Anchor Points to Corner",
        "Convert Selected Anchor Points to Smooth",
        "Duplicate",
    ];
    assert_eq!(buttons[..expected.len()], expected, "{buttons:?}");
    let press = |app: &mut VectorcraftApp, label: &str| {
        let labels = task_bar_buttons(app, &ctx);
        let at = labels.iter().find(|(t, _)| t == label).unwrap_or_else(|| panic!("{label} in {labels:?}")).1.center();
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(app, &ctx, vec![egui::Event::PointerMoved(at)]);
        frame(app, &ctx, vec![button(true)]);
        frame(app, &ctx, vec![button(false)]);
    };
    let subpath =
        |app: &VectorcraftApp| app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().path_data().unwrap().subpaths[0].clone();
    let last_undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.last().unwrap().label.clone();
    press(&mut app, "Convert Selected Anchor Points to Smooth");
    let a = subpath(&app).anchors[1];
    assert!(a.has_in() && a.has_out(), "smooth: {a:?}");
    assert_eq!(last_undo(&app), "Convert Anchor Points");
    press(&mut app, "Convert Selected Anchor Points to Corner");
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
                let guides = &app.session.active().unwrap().doc.ruler_guides();
                assert_eq!(guides.len(), 1);
                let vertical = source_vertical != switched;
                assert_eq!(guides[0].vertical, vertical);
                let want = if vertical { expected.x } else { expected.y };
                assert!((guides[0].pos - want).abs() < 1e-4, "origin={origin:?} vertical={vertical}: {:?}, want {want}", guides[0]);
            };
            assert_eq!(history(&app).0, original_history.0, "preview has no undo entry");
            if cancel {
                // Cancellation is a baseline positive control, independent of snap correctness.
                assert_eq!(app.session.active().unwrap().doc.ruler_guides().len(), 1);
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
            let art = |d: &vectorcraft_doc::Document| {
                d.layers
                    .iter()
                    .map(|l| {
                        let mut l = vectorcraft_doc::Node::clone(l);
                        l.strip_ruler_guides();
                        l
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(art(&st.doc), art(&before), "the rectangle tool draws no artwork");
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
        assert!((app.session.active().unwrap().doc.ruler_guides()[0].pos - target).abs() < 1e-4);
        let st = app.session.active().unwrap();
        assert_eq!(vectorcraft_engine::cmd::guide_indexes(&st.doc, &st.selection.guides), vec![0]);
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
        assert!((app.session.active().unwrap().doc.ruler_guides()[0].pos - target).abs() < 1e-4, "stored coordinate remains absolute");
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

#[test]
fn text_thread_overlays_keep_document_order_with_mixed_selection() {
    use vectorcraft_doc::NodeId;
    use vectorcraft_tools::text::{Port, port_centre};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    let text = "Words that fill more than the frame holds. ".repeat(10);
    let id = |value: serde_json::Value| NodeId(value["id"].as_u64().unwrap());
    let first = id(app.run("text.create", json!({"x": 20, "y": 20, "text": text, "area": {"width": 150, "height": 60}})).unwrap());
    let first_next = id(app.run("text.thread.add", json!({"from": first.0, "at": [300, 20]})).unwrap());
    let second = id(app
        .run(
            "text.create",
            json!({"x": 20, "y": 200, "text": "Words that fill more than the frame holds. ".repeat(10), "area": {"width": 150, "height": 60}}),
        )
        .unwrap());
    let second_next = id(app.run("text.thread.add", json!({"from": second.0, "at": [300, 200]})).unwrap());
    let unselected = id(app
        .run("text.create", json!({"x": 20, "y": 100, "text": "An unselected story. ".repeat(20), "area": {"width": 150, "height": 60}}))
        .unwrap());
    let unselected_next = id(app.run("text.thread.add", json!({"from": unselected.0, "at": [300, 100]})).unwrap());
    let unrelated_a = id(app.run("shape.rectangle", json!({"x": 450, "y": 300, "width": 20, "height": 20})).unwrap());
    let unrelated_b = id(app.run("shape.rectangle", json!({"x": 480, "y": 300, "width": 20, "height": 20})).unwrap());
    app.run("select.set", json!({"ids": [unrelated_a.0, unrelated_b.0, second_next.0, first.0]})).unwrap();

    let state = app.session.active().unwrap();
    assert_eq!(state.selection.objects, [unrelated_a, unrelated_b, second_next, first]);
    assert_eq!(state.doc.text_threads, [vec![first, first_next], vec![second, second_next], vec![unselected, unselected_next]]);
    let frame = |id| match &state.doc.node(id).unwrap().kind {
        NodeKind::Text(text) => (**text).clone(),
        _ => panic!("thread member is text"),
    };
    let px = 1.0;
    let expected = [
        (port_centre(&frame(first), Port::Out, px).unwrap(), port_centre(&frame(first_next), Port::In, px).unwrap()),
        (port_centre(&frame(second), Port::Out, px).unwrap(), port_centre(&frame(second_next), Port::In, px).unwrap()),
    ];
    let overlays = thread_overlays(&app, 1.0);
    let lines: Vec<_> = overlays
        .iter()
        .filter_map(|overlay| match overlay {
            Overlay::Line { a, b, .. } if expected.iter().any(|(from, to)| *a == *from && *b == *to) => Some((*a, *b)),
            _ => None,
        })
        .collect();
    assert_eq!(lines, expected, "thread lines follow document order, not selection order");
    assert_eq!(overlays.iter().filter(|overlay| matches!(overlay, Overlay::Line { .. })).count(), 14, "two thread links and their port arrows only");
    let ports: Vec<_> = overlays
        .iter()
        .filter_map(|overlay| match overlay {
            Overlay::Highlight { quad, .. } => Some(quad[0].midpoint(quad[2])),
            _ => None,
        })
        .collect();
    assert_eq!(ports, expected.into_iter().flat_map(|(from, to)| [from, to]).collect::<Vec<_>>());
}

/// Two document-order threads hit the first two selected IDs among 512 unrelated paths.
/// Their real port geometry and document-order output remain unchanged.
#[test]
fn text_thread_overlays_keep_two_early_hits_with_many_unrelated_paths() {
    use std::sync::Arc;
    use vectorcraft_doc::NodeId;
    use vectorcraft_tools::text::{Port, port_centre, ports_shown};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    let text = "Words that fill more than the frame holds. ".repeat(10);
    let id = |value: serde_json::Value| NodeId(value["id"].as_u64().unwrap());
    let mut threads = vec![];
    for y in [20, 200] {
        let first = id(app.run("text.create", json!({"x": 20, "y": y, "text": text, "area": {"width": 150, "height": 60}})).unwrap());
        let next = id(app.run("text.thread.add", json!({"from": first.0, "at": [300, y]})).unwrap());
        threads.push([first, next]);
    }
    let path = id(app.run("shape.rectangle", json!({"x": 450, "y": 300, "width": 20, "height": 20})).unwrap());
    let mut selected = vec![threads[0][0], threads[1][0], path];
    {
        let st = app.session.active_mut().unwrap();
        let doc = Arc::make_mut(&mut st.doc);
        let template = doc.node(path).unwrap().clone();
        let mut additions = Vec::with_capacity(511);
        for _ in 1..512 {
            let mut p = template.clone();
            p.id = doc.alloc_id();
            selected.push(p.id);
            additions.push(Arc::new(p));
        }
        Arc::make_mut(&mut doc.layers[0]).children_mut().unwrap().extend(additions);
        doc.drop_lookup_cache();
    }
    app.run("select.set", json!({"ids": selected.iter().map(|id| id.0).collect::<Vec<_>>()})).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.text_threads, threads.iter().map(|t| t.to_vec()).collect::<Vec<_>>());
    assert_eq!(st.selection.objects, selected);
    assert_eq!(st.selection.objects.len(), 514);
    assert!(ports_shown(&st.doc, &st.selection.objects));
    let frame = |id| match &st.doc.node(id).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("thread member is text"),
    };
    let expected: Vec<_> =
        threads.iter().map(|[a, b]| (port_centre(&frame(*a), Port::Out, 1.0).unwrap(), port_centre(&frame(*b), Port::In, 1.0).unwrap())).collect();
    let overlays = thread_overlays(&app, 1.0);
    let links: Vec<_> = overlays
        .iter()
        .filter_map(|o| match o {
            Overlay::Line { a, b, .. } if expected.contains(&(*a, *b)) => Some((*a, *b)),
            _ => None,
        })
        .collect();
    assert_eq!(links, expected, "the two thread links stay in document order");
    assert_eq!(overlays.iter().filter(|o| matches!(o, Overlay::Line { .. })).count(), 14, "two links and their port arrows only");
    let ports: Vec<_> = overlays
        .iter()
        .filter_map(|o| match o {
            Overlay::Highlight { quad, .. } => Some(quad[0].midpoint(quad[2])),
            _ => None,
        })
        .collect();
    assert_eq!(ports, expected.into_iter().flat_map(|(a, b)| [a, b]).collect::<Vec<_>>());
}

/// One two-frame thread misses a selection of 512 unrelated paths.
/// This checks ordinary empty output, not elapsed time or allocation.
#[test]
fn text_thread_overlays_skip_one_unselected_thread_with_many_unrelated_paths() {
    use std::sync::Arc;
    use vectorcraft_doc::NodeId;
    use vectorcraft_tools::text::{ported, ports_shown};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    let text = "Words that fill more than the frame holds. ".repeat(10);
    let id = |value: serde_json::Value| NodeId(value["id"].as_u64().unwrap());
    let first = id(app.run("text.create", json!({"x": 20, "y": 20, "text": text, "area": {"width": 150, "height": 60}})).unwrap());
    let next = id(app.run("text.thread.add", json!({"from": first.0, "at": [300, 20]})).unwrap());
    let path = id(app.run("shape.rectangle", json!({"x": 450, "y": 300, "width": 20, "height": 20})).unwrap());
    let mut selected = vec![path];
    {
        let st = app.session.active_mut().unwrap();
        let doc = Arc::make_mut(&mut st.doc);
        let template = doc.node(path).unwrap().clone();
        let mut additions = Vec::with_capacity(511);
        for _ in 1..512 {
            let mut p = template.clone();
            p.id = doc.alloc_id();
            selected.push(p.id);
            additions.push(Arc::new(p));
        }
        Arc::make_mut(&mut doc.layers[0]).children_mut().unwrap().extend(additions);
        doc.drop_lookup_cache();
    }
    app.run("select.set", json!({"ids": selected.iter().map(|id| id.0).collect::<Vec<_>>()})).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.text_threads, [vec![first, next]]);
    assert_eq!(st.selection.objects, selected);
    assert_eq!(st.selection.objects.len(), 512);
    assert!(selected.iter().all(|id| *id != first && *id != next));
    assert!([first, next].into_iter().all(|id| ported(&st.doc, id).is_some()), "both thread frames have ports");
    assert!(ports_shown(&st.doc, &st.selection.objects));
    assert!(thread_overlays(&app, 1.0).is_empty());
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

/// One headless canvas frame → whether the fade outside the artboards is drawn.
fn fades(app: &mut VectorcraftApp, ctx: &egui::Context) -> bool {
    fade_mesh(&shapes(app, ctx)).is_some()
}

/// The fade outside the artboards among `shapes`, with its position: the mesh in [`FADE_OUTSIDE`].
fn fade_mesh(shapes: &[Shape]) -> Option<(usize, &egui::Mesh)> {
    shapes.iter().enumerate().find_map(|(i, s)| match s {
        Shape::Mesh(m) if !m.vertices.is_empty() && m.vertices.iter().all(|v| v.color == FADE_OUTSIDE) => Some((i, &**m)),
        _ => None,
    })
}

/// Does `mesh` cover the screen point `p`?
fn mesh_covers(mesh: &egui::Mesh, p: Pos2) -> bool {
    let cross = |a: Pos2, b: Pos2| (b - a).x * (p - a).y - (b - a).y * (p - a).x;
    mesh.indices.chunks(3).any(|t| {
        let [a, b, c] = [t[0], t[1], t[2]].map(|i| mesh.vertices[i as usize].pos);
        let (d0, d1, d2) = (cross(a, b), cross(b, c), cross(c, a));
        (d0 >= 0.0 && d1 >= 0.0 && d2 >= 0.0) || (d0 <= 0.0 && d1 <= 0.0 && d2 <= 0.0)
    })
}

/// Artboard Options › Fade Region Outside Artboard: while the Artboard tool is active the area
/// outside the artboards draws darker; with another tool, or with the preference off, it draws as
/// usual.
#[test]
fn the_artboard_tool_fades_the_pasteboard() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(FADE_OUTSIDE.a(), 102, "black at 40%: each channel under it shows at 60%");
    assert!(app.session.prefs.fade_region_outside_artboard, "on by default");
    assert!(!fades(&mut app, &ctx), "the Selection tool");
    app.select_tool("artboard");
    assert!(fades(&mut app, &ctx), "the Artboard tool");
    app.select_tool("rectangle");
    assert!(!fades(&mut app, &ctx), "the Rectangle tool");
    app.run("prefs.set", json!({"key": "fadeRegionOutsideArtboard", "value": false})).unwrap();
    app.select_tool("artboard");
    assert!(!fades(&mut app, &ctx), "the preference off");
}

/// Presentation Mode shows only the art: with the Artboard tool active and the fade on, it draws no
/// fade, and the fade comes back when Presentation Mode ends.
#[test]
fn presentation_mode_draws_no_fade() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    app.select_tool("artboard");
    assert!(fades(&mut app, &ctx), "the Artboard tool");
    app.run("view.presentation", json!({})).unwrap();
    assert_eq!(app.ui.screen_mode, 3);
    assert_eq!(app.session.tool_id(), "artboard");
    assert!(!fades(&mut app, &ctx), "Presentation Mode");
    app.run("view.presentation", json!({})).unwrap();
    assert!(fades(&mut app, &ctx), "after Presentation Mode");
}

/// The fade covers the art that lies on the pasteboard as well as the pasteboard: it is drawn over
/// the art, over a point of an object outside the artboard and not over a point of an object on it,
/// in an upright view and in a rotated one.
#[test]
fn the_fade_darkens_art_on_the_pasteboard_and_not_on_the_artboard() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": -150, "y": 100, "width": 100, "height": 100})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 150, "y": 100, "width": 100, "height": 100})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    let ctx = egui::Context::default();
    shapes(&mut app, &ctx); // the first frame fits the view to the artboard
    app.view_mut().unwrap().zoom = 0.5;
    app.select_tool("artboard");
    for rotation in [0.0, 30.0] {
        app.view_mut().unwrap().rotation = rotation;
        let s = shapes(&mut app, &ctx);
        let art = app.canvas.texture.as_ref().unwrap().id();
        let image = s.iter().position(|s| matches!(s, Shape::Mesh(m) if m.texture_id == art)).expect("the art");
        let (at, mesh) = fade_mesh(&s).expect("the fade");
        assert!(at > image, "over the art, rotation {rotation}");
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        assert!(mesh_covers(mesh, xf.to_screen(Point::new(-100.0, 150.0))), "the object outside, rotation {rotation}");
        assert!(!mesh_covers(mesh, xf.to_screen(Point::new(200.0, 150.0))), "the object on the artboard, rotation {rotation}");
        assert!(mesh_covers(mesh, xf.to_screen(Point::new(200.0, -40.0))), "the pasteboard, rotation {rotation}");
    }
}

/// Artboard Options › Update While Dragging: on, the pasteboard stays darker while the Artboard tool
/// moves or resizes an artboard; off, it draws as usual during the drag and darker again after it.
#[test]
fn update_while_dragging_keeps_or_drops_the_fade_during_an_artboard_drag() {
    for update in [true, false] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("prefs.set", json!({"key": "artboardUpdateWhileDragging", "value": update})).unwrap();
        app.select_tool("artboard");
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let r = app.session.active().unwrap().doc.artboards[0].rect;
        let view = app.view_info();
        // A move from the artboard's middle, then a resize from its right edge.
        let (cx, cy) = (r.center().x, r.center().y);
        for (from, to) in
            [(Point::new(cx, cy), Point::new(cx + 40.0, cy + 30.0)), (Point::new(r.x1 + 40.0, cy + 30.0), Point::new(r.x1 + 90.0, cy + 30.0))]
        {
            dispatch(&mut app, &PointerEvent::new(PointerKind::Down, from.x, from.y), view);
            for p in [from + (to - from) * 0.5, to] {
                dispatch(&mut app, &PointerEvent::new(PointerKind::Drag, p.x, p.y), view);
            }
            assert!(app.session.tool_transforming(), "the artboard is being dragged");
            assert_eq!(fades(&mut app, &ctx), update, "Update While Dragging {update}");
            dispatch(&mut app, &PointerEvent::new(PointerKind::Up, to.x, to.y), view);
            assert!(!app.session.tool_transforming());
            assert!(fades(&mut app, &ctx), "darker again after the drag");
        }
        let moved = app.session.active().unwrap().doc.artboards[0].rect;
        assert!(moved.x0 > r.x0 && moved.width() > r.width(), "moved, then resized: {moved:?} from {r:?}");
    }
}

/// The fade is drawn only on the canvas: exports with the Artboard tool active and the fade on are
/// the same as with the Selection tool and the fade off.
#[test]
fn the_pasteboard_fade_leaves_exports_unchanged() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": -40, "y": 20, "width": 120, "height": 40})).unwrap();
    let exports = |app: &mut VectorcraftApp| -> Vec<serde_json::Value> {
        ["svg", "png"].iter().map(|f| app.run("document.export", json!({"format": f})).unwrap()).collect()
    };
    app.run("prefs.set", json!({"key": "fadeRegionOutsideArtboard", "value": false})).unwrap();
    let plain = exports(&mut app);
    app.run("prefs.set", json!({"key": "fadeRegionOutsideArtboard", "value": true})).unwrap();
    app.select_tool("artboard");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    assert!(fades_outside_artboards(&app));
    assert_eq!(exports(&mut app), plain);
}

/// Finished worker renders are uploaded whenever they are not older than what is currently
/// displayed on screen. During continuous edit streams (drag previews, held arrow nudges, or
/// command bursts) where the document revision advances every frame, intermediate worker frames
/// are uploaded progressively to prevent the canvas from freezing. Obsolete frames older than what
/// is already displayed are discarded.
#[test]
fn poll_worker_accepts_frames_not_older_than_displayed_and_discards_obsolete_frames() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.canvas.worker_started = true;
    let ctx = egui::Context::default();
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
    let mut out = ctx.run_ui(raw.clone(), |ui| show(&mut app, ui));
    out.textures_delta.clear();

    let doc = app.session.active().unwrap().doc.clone();
    let initial_key = app.canvas.key.clone().expect("first frame initializes key");
    let key_doc = initial_key.doc;

    // Displayed canvas texture is initially at revision 2.
    let mut displayed_key = initial_key.clone();
    displayed_key.revision = 2;
    app.canvas.key = Some(displayed_key);

    let (worker, _jobs, done_tx) = crate::render_worker::Worker::test_channel();
    app.canvas.worker = Some(worker);

    // Worker completes revision 5 (intermediate frame from an edit stream).
    // It is newer than displayed revision 2: it MUST be accepted and uploaded.
    let mut rev5_key = initial_key.clone();
    rev5_key.revision = 5;
    let img5 = vectorcraft_render::Rendered { width: 10, height: 10, pixels: vec![200; 400] };
    done_tx
        .send(crate::render_worker::Done {
            key: rev5_key,
            doc: doc.clone(),
            opts: vectorcraft_render::RenderOptions::default(),
            img: img5,
            ms: 2.5,
            moved: 0,
            reused: false,
        })
        .unwrap();

    let mut out = ctx.run_ui(raw.clone(), |ui| poll_worker(&mut app, ui, key_doc));
    let set_count = out.textures_delta.set.len();
    out.textures_delta.clear();
    assert_eq!(set_count, 1, "intermediate frame 5 is uploaded to texture");
    assert_eq!(app.canvas.key.as_ref().unwrap().revision, 5, "intermediate frame 5 is uploaded");
    assert!(app.canvas.from_worker);

    // Worker completes an older revision 3 (out of order). It is older than displayed 5, so discarded.
    let mut rev3_key = initial_key.clone();
    rev3_key.revision = 3;
    let img3 = vectorcraft_render::Rendered { width: 10, height: 10, pixels: vec![150; 400] };
    done_tx
        .send(crate::render_worker::Done {
            key: rev3_key,
            doc: doc.clone(),
            opts: vectorcraft_render::RenderOptions::default(),
            img: img3,
            ms: 2.5,
            moved: 0,
            reused: false,
        })
        .unwrap();

    let mut out = ctx.run_ui(raw.clone(), |ui| poll_worker(&mut app, ui, key_doc));
    let set_count = out.textures_delta.set.len();
    out.textures_delta.clear();
    assert_eq!(set_count, 0, "older out-of-order frame 3 emits NO texture upload delta, leaving displayed texture intact");
    assert_eq!(app.canvas.key.as_ref().unwrap().revision, 5, "older out-of-order frame 3 is discarded");

    // Worker completes another intermediate revision 8. Newer than displayed 5, so uploaded.
    let mut rev8_key = initial_key.clone();
    rev8_key.revision = 8;
    let img8 = vectorcraft_render::Rendered { width: 10, height: 10, pixels: vec![180; 400] };
    done_tx
        .send(crate::render_worker::Done {
            key: rev8_key,
            doc: doc.clone(),
            opts: vectorcraft_render::RenderOptions::default(),
            img: img8,
            ms: 2.5,
            moved: 0,
            reused: false,
        })
        .unwrap();

    let mut out = ctx.run_ui(raw.clone(), |ui| poll_worker(&mut app, ui, key_doc));
    let set_count = out.textures_delta.set.len();
    out.textures_delta.clear();
    assert_eq!(set_count, 1, "newer frame 8 is uploaded to texture");
    assert_eq!(app.canvas.key.as_ref().unwrap().revision, 8, "newer frame 8 is uploaded");

    // Finally, the settling frame at revision 10 completes: it is accepted and uploaded.
    let mut rev10_key = initial_key;
    rev10_key.revision = 10;
    let img10 = vectorcraft_render::Rendered { width: 10, height: 10, pixels: vec![255; 400] };
    done_tx
        .send(crate::render_worker::Done {
            key: rev10_key,
            doc: doc.clone(),
            opts: vectorcraft_render::RenderOptions::default(),
            img: img10,
            ms: 2.5,
            moved: 0,
            reused: false,
        })
        .unwrap();

    let mut out = ctx.run_ui(raw, |ui| poll_worker(&mut app, ui, key_doc));
    let set_count = out.textures_delta.set.len();
    out.textures_delta.clear();
    assert_eq!(set_count, 1, "settled frame 10 is uploaded to texture");
    assert_eq!(app.canvas.key.as_ref().unwrap().revision, 10, "settled frame 10 is uploaded");
}

/// The labels the Contour bar draws in a headless frame, with their rects; none while it isn't
/// shown.
fn contour_bar_labels(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<(String, egui::Rect)> {
    egui_extras::install_image_loaders(ctx);
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| show(app, ui));
    out.textures_delta.clear();
    let Some(bar) = contour_bar::bar_rect(ctx) else { return vec![] };
    out.shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) if bar.contains(t.visual_bounding_rect().center()) => Some((t.galley.text().to_string(), t.visual_bounding_rect())),
            _ => None,
        })
        .collect()
}

/// A click on the centre of the label `name` of the Contour bar, laid out by one frame before it.
fn click_contour_bar(app: &mut VectorcraftApp, ctx: &egui::Context, name: &str) {
    let labels = contour_bar_labels(app, ctx, vec![]);
    let at = labels.iter().find(|(t, _)| t == name).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{name}` in {labels:?}"));
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    contour_bar_labels(app, ctx, vec![egui::Event::PointerMoved(at)]);
    contour_bar_labels(app, ctx, vec![button(true), button(false)]);
}

/// The Contour bar shows with the Contour tool and a shape selected: Sharp, Round and Square set the
/// corner style (of a selected Contour at once, else of the next one), and Bake turns a Contour into
/// plain paths.
#[test]
fn the_contour_bar_sets_corners_and_bakes() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 80, "height": 60})).unwrap();
    let ctx = egui::Context::default();

    // Only the Contour tool shows it.
    app.select_tool("selection");
    assert!(contour_bar_labels(&mut app, &ctx, vec![]).is_empty());
    app.select_tool("contour");
    contour_bar_labels(&mut app, &ctx, vec![]);
    let names: Vec<String> = contour_bar_labels(&mut app, &ctx, vec![]).into_iter().map(|(t, _)| t).collect();
    for want in ["Contour", "Sharp", "Round", "Square", "Bake"] {
        assert!(names.iter().any(|n| n == want), "{want} in {names:?}");
    }
    let offsets = |app: &VectorcraftApp| {
        let mut found = vec![];
        app.session.active().unwrap().doc.walk(|n| {
            if let Some(e) = n.appearance.effects.iter().find(|e| e.id == "path.offsetPath") {
                found.push(e.params["joins"].as_str().unwrap_or_default().to_string());
            }
        });
        found
    };

    // No Contour yet: Round is kept for the next one, and Bake does nothing.
    let steps = app.session.active().unwrap().history.undo.len();
    click_contour_bar(&mut app, &ctx, "Round");
    assert_eq!(app.session.tool_options()["corners"], json!("round"));
    click_contour_bar(&mut app, &ctx, "Bake");
    assert_eq!(app.session.active().unwrap().history.undo.len(), steps, "nothing to bake");
    assert!(offsets(&app).is_empty());

    // A Contour made now has round corners; Square changes the selected Contour in place.
    app.session.execute("addon.contour.set", &json!({"offset": 8, "corners": "round"})).unwrap();
    assert_eq!(offsets(&app), ["round"]);
    click_contour_bar(&mut app, &ctx, "Square");
    assert_eq!(offsets(&app), ["bevel"], "the selected Contour changed, none was added");

    // Bake: the live effect is gone and the art stays.
    click_contour_bar(&mut app, &ctx, "Bake");
    assert!(offsets(&app).is_empty(), "baked");
}

/// A frame the render worker finished for `job`: white pixels of the job's size.
fn finished(job: &crate::render_worker::Job) -> crate::render_worker::Done {
    let img = vectorcraft_render::Rendered { width: job.w, height: job.h, pixels: vec![255; job.w as usize * job.h as usize * 4] };
    crate::render_worker::Done { key: job.key.clone(), doc: job.doc.clone(), opts: job.opts.clone(), img, ms: 40.0, moved: 0, reused: false }
}

/// A job for the render worker to pan over a document of `doc`: a view at 100% moved by `(dx, dy)`
/// device pixels, in a 256×128 frame.
fn pan_job(doc: &std::sync::Arc<vectorcraft_doc::Document>, key: &CacheKey, (dx, dy): (f64, f64)) -> crate::render_worker::Job {
    crate::render_worker::Job {
        key: CacheKey { cx: 128.0 - dx, cy: 64.0 - dy, w: 256, h: 128, ..key.clone() },
        doc: doc.clone(),
        w: 256,
        h: 128,
        view: Affine::translate((dx, dy)),
        opts: vectorcraft_render::RenderOptions::default(),
        scope: vectorcraft_engine::file_access::ScopeHandle::capture(),
        effects: Default::default(),
        may_copy: true,
        may_keep: true,
        display: None,
    }
}

/// The document and the canvas key the render worker tests start from: rectangles on whole pixels.
fn pan_doc() -> (std::sync::Arc<vectorcraft_doc::Document>, CacheKey) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 256, "height": 128})).unwrap();
    for (x, color) in [(16, "#c02040"), (96, "#2040c0"), (176, "#20a040")] {
        let id = app.run("shape.rectangle", json!({"x": x, "y": 16, "width": 64, "height": 96})).unwrap()["id"].clone();
        app.run("paint.setFill", json!({"ids": [id], "color": color})).unwrap();
    }
    app.canvas.worker_started = true;
    shapes(&mut app, &egui::Context::default());
    let key = app.canvas.key.clone().expect("a frame was rendered");
    (app.session.active().unwrap().doc.clone(), CacheKey { zoom: 1.0, rot: 0.0, ppp: 1.0, pixel: None, ..key })
}

/// A pan by whole device pixels takes the pixels that stay on screen from the last frame: a mark
/// put in that frame moves with the pan, and the frame matches a whole render where the mark is not.
#[test]
fn a_whole_pixel_pan_copies_the_pixels_that_stay_on_screen() {
    keeping_color_settings(a_whole_pixel_pan);
}

fn a_whole_pixel_pan() {
    let (doc, key) = pan_doc();
    for pan in [(10.0, 0.0), (-10.0, 0.0), (0.0, 7.0), (0.0, -7.0), (-13.0, 5.0)] {
        let mut r = vectorcraft_render::Renderer::new();
        let mut kept = None;
        let first = render_job(&mut r, &mut kept, &pan_job(&doc, &key, (0.0, 0.0)));
        assert!(!first.reused, "the first frame renders whole");
        let k = kept.as_mut().and_then(|k| k.pixels.as_mut()).expect("the frame holds its pixels");
        // The middle of the frame, which stays on screen.
        k[(64 * 256 + 128) * 4..(64 * 256 + 128) * 4 + 4].copy_from_slice(&[1, 2, 3, 4]);
        let job = pan_job(&doc, &key, pan);
        let frame = render_job(&mut r, &mut kept, &job);
        assert!(frame.reused, "{pan:?}");
        let at = ((64 + pan.1 as i64) * 256 + 128 + pan.0 as i64) as usize * 4;
        assert_eq!(frame.img.pixels[at..at + 4], [1, 2, 3, 4], "{pan:?}: the marked pixel moved with the art");
        let whole = vectorcraft_render::Renderer::new().render(&doc, 256, 128, job.view, &job.opts);
        let mut img = frame.img.pixels;
        img[at..at + 4].copy_from_slice(&whole.pixels[at..at + 4]);
        assert_eq!(img, whole.pixels, "{pan:?}: the same picture as a whole render");
    }
}

/// Installing, replacing or removing a plug-in draws a document with a plug-in effect again, and a
/// pan from the frame drawn before the change renders whole instead of copying its pixels. The key
/// of a document without a plug-in effect does not follow the plug-ins. The other tests share the
/// plug-in registry, and the plug-in is removed after.
#[test]
fn a_plug_in_change_draws_the_document_s_plug_in_effects_again() {
    keeping_color_settings(a_plug_in_change);
}

fn a_plug_in_change() {
    use vectorcraft_plugins::wat as tpl;
    let id = "test.canvas.redraw";
    let module = vectorcraft_format::base64_encode(&wat::parse_str(tpl::module(&tpl::manifest(id, "effect", "{}"), tpl::ECHO, "", 1)).unwrap());
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("plugin.install", json!({"dataBase64": module})).unwrap();
    app.session.execute("file.new", &json!({"width": 256, "height": 128})).unwrap();
    let rect = app.run("shape.rectangle", json!({"x": 16, "y": 16, "width": 64, "height": 96})).unwrap()["id"].clone();
    app.canvas.worker_started = true; // render on this thread
    let ctx = egui::Context::default();
    shapes(&mut app, &ctx);
    // Other keys' fields (placed documents' bitmaps) follow what other tests do; this one's don't.
    let plugins = |app: &VectorcraftApp| app.canvas.key.as_ref().expect("a frame was rendered").plugins;
    let plain = plugins(&app);
    app.run("plugin.install", json!({"dataBase64": module})).unwrap();
    shapes(&mut app, &ctx);
    assert_eq!(plugins(&app), plain, "no plug-in effect, so nothing to draw again");
    app.run("effect.apply", json!({"effect": vectorcraft_plugins::effect::effect_id(id), "ids": [rect]})).unwrap();
    shapes(&mut app, &ctx);
    let before = app.canvas.key.clone().expect("a frame was rendered");
    app.run("plugin.install", json!({"dataBase64": module})).unwrap();
    shapes(&mut app, &ctx);
    let after = plugins(&app);
    assert_ne!(after, before.plugins, "drawn again once the plug-in was replaced");
    // The worker's frame from before the change gives no pixels to the next pan.
    let doc = app.session.active().unwrap().doc.clone();
    let at_100 = CacheKey { zoom: 1.0, rot: 0.0, ppp: 1.0, pixel: None, ..before };
    let mut r = vectorcraft_render::Renderer::new();
    let mut kept = None;
    render_job(&mut r, &mut kept, &pan_job(&doc, &at_100, (0.0, 0.0)));
    let pan = pan_job(&doc, &CacheKey { plugins: after, ..at_100.clone() }, (10.0, 0.0));
    assert!(!render_job(&mut r, &mut kept, &pan).reused, "copied from the frame before the change");
    app.run("plugin.remove", json!({"id": id})).unwrap();
    shapes(&mut app, &ctx);
    assert_ne!(plugins(&app), after, "drawn again once the plug-in was removed");
}

/// A frame does not copy from the last one when the view moves by part of a pixel or over half the
/// frame, when copying is not allowed, or when a listed key field, render option, zoom, rotation,
/// canvas size or device scale differs. Each such frame has the job's size. The session test
/// compares such frames with a fresh render pixel by pixel.
#[test]
fn a_frame_renders_whole_when_anything_but_a_whole_pixel_pan_changed() {
    keeping_color_settings(a_frame_renders_whole);
}

fn a_frame_renders_whole() {
    let (doc, key) = pan_doc();
    let other_opts = |f: &dyn Fn(&mut vectorcraft_render::RenderOptions)| {
        let mut job = pan_job(&doc, &key, (10.0, 0.0));
        f(&mut job.opts);
        job
    };
    let other_key = |f: &dyn Fn(&mut CacheKey)| {
        let mut job = pan_job(&doc, &key, (10.0, 0.0));
        f(&mut job.key);
        job
    };
    let mut other_doc = (*doc).clone();
    other_doc.layers.clear();
    let scaled = {
        let mut job = pan_job(&doc, &key, (10.0, 0.0));
        job.key.zoom = 2.0;
        job.view = Affine::translate((10.0, 0.0)) * Affine::scale(2.0);
        job
    };
    let turned = {
        let mut job = pan_job(&doc, &key, (10.0, 0.0));
        job.key.rot = 30.0;
        job.view = Affine::translate((10.0, 0.0)) * Affine::rotate(30f64.to_radians());
        job
    };
    let resized = {
        let mut job = pan_job(&doc, &key, (10.0, 0.0));
        (job.key.w, job.w) = (200, 200);
        job
    };
    let denser = other_key(&|k| k.ppp = 2.0);
    let not_allowed = crate::render_worker::Job { may_copy: false, ..pan_job(&doc, &key, (10.0, 0.0)) };
    let cases: Vec<(&str, crate::render_worker::Job)> = vec![
        ("part of a pixel across", pan_job(&doc, &key, (10.5, 0.0))),
        ("part of a pixel down", pan_job(&doc, &key, (0.0, 0.25))),
        ("too far to keep half", pan_job(&doc, &key, (250.0, 0.0))),
        ("zoom", scaled),
        ("rotation", turned),
        ("canvas size", resized),
        ("device scale", denser),
        ("not allowed", not_allowed),
        ("document", other_key(&|k| k.doc += 1)),
        ("revision", crate::render_worker::Job { doc: std::sync::Arc::new(other_doc), ..other_key(&|k| k.revision += 1) }),
        ("interaction", other_key(&|k| k.interacting = true)),
        ("outline key", other_key(&|k| k.outline = true)),
        ("Pixel Preview", other_key(&|k| k.pixel = Some([0, 0, 256, 128]))),
        ("placed bitmaps", other_key(&|k| k.placed += 1)),
        ("plug-ins", other_key(&|k| k.plugins += 1)),
        ("outline", other_opts(&|o| o.outline = true)),
        ("proof", other_opts(&|o| o.proof = Some(Default::default()))),
        ("overprint preview", other_opts(&|o| o.overprint_preview = true)),
        ("rich black", other_opts(&|o| o.rich_black = true)),
        ("trim", other_opts(&|o| o.trim = true)),
        ("anti-alias", other_opts(&|o| o.anti_alias = vectorcraft_render::AntiAlias::None)),
        ("smooth images", other_opts(&|o| o.smooth_images = false)),
        ("opacity mask view", other_opts(&|o| o.mask_view = Some(vectorcraft_doc::NodeId(1)))),
    ];
    let mut copied = vec![];
    for (name, job) in cases {
        let mut r = vectorcraft_render::Renderer::new();
        let mut kept = None;
        render_job(&mut r, &mut kept, &pan_job(&doc, &key, (0.0, 0.0)));
        let frame = render_job(&mut r, &mut kept, &job);
        if frame.reused {
            copied.push(name);
        }
        let whole = vectorcraft_render::Renderer::new().render(&job.doc, job.w, job.h, job.view, &job.opts);
        assert_eq!((frame.img.width, frame.img.height), (whole.width, whole.height), "{name}");
    }
    assert!(copied.is_empty(), "copied from the last frame: {copied:?}");
}

/// The worker keeps a frame only when a later one can copy from it: a frame that does not allow
/// keeping, a frame over the limit and a frame that drew a raster from before its object moved are
/// not kept. A frame that may not copy can still be kept.
#[test]
fn the_render_worker_keeps_one_frame_up_to_its_limit() {
    keeping_color_settings(the_render_worker_keeps_one_frame);
}

fn the_render_worker_keeps_one_frame() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 256, "height": 128})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 16, "y": 16, "width": 64, "height": 64})).unwrap()["id"].clone();
    app.run("effect.apply", json!({"effect": "stylize.dropShadow", "ids": [id]})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.run("object.move", json!({"dx": 8, "dy": 0})).unwrap();
    let after = app.session.active().unwrap().doc.clone();
    let (_, key) = pan_doc();
    let mut r = vectorcraft_render::Renderer::new();
    let mut kept = None;
    render_job(&mut r, &mut kept, &pan_job(&before, &key, (0.0, 0.0)));
    assert!(kept.is_some(), "kept");
    let mut moved = pan_job(&after, &CacheKey { revision: key.revision + 1, interacting: true, ..key.clone() }, (0.0, 0.0));
    moved.opts.interacting = true;
    let frame = render_job(&mut r, &mut kept, &moved);
    assert!(frame.moved > 0, "the shadow drew from the raster made before the move");
    let mut wrong = vec![];
    if kept.is_some() {
        wrong.push("a frame that drew a raster from before its object moved");
    }

    let (doc, key) = pan_doc();
    render_job(&mut r, &mut kept, &pan_job(&doc, &key, (0.0, 0.0)));
    assert!(kept.is_some(), "kept");
    let off = crate::render_worker::Job { may_keep: false, ..pan_job(&doc, &key, (0.0, 0.0)) };
    render_job(&mut r, &mut kept, &off);
    if kept.is_some() {
        wrong.push("a frame that does not allow keeping");
    }
    let whole = crate::render_worker::Job { may_copy: false, ..pan_job(&doc, &key, (0.0, 0.0)) };
    render_job(&mut r, &mut kept, &whole);
    assert!(kept.is_some(), "a frame that may not copy is kept");
    let mut big = pan_job(&doc, &key, (0.0, 0.0));
    (big.key.w, big.key.h, big.w, big.h) = (4097, 4097, 4097, 4097);
    render_job(&mut r, &mut kept, &big);
    if kept.is_some() {
        wrong.push("a frame over 64 MiB");
    }
    assert!(wrong.is_empty(), "kept: {wrong:?}");
}

/// Through a pan, an edit, an undo, a zoom, a resize, a view mode and a color setting, each frame the
/// worker renders whole matches a fresh render, and each copied frame is within one level of it.
#[test]
fn worker_frames_match_a_fresh_render_through_a_session() {
    keeping_color_settings(worker_frames_match_a_fresh_render);
}

fn worker_frames_match_a_fresh_render() {
    let (doc, key) = pan_doc();
    let mut edited = (*doc).clone();
    edited.layers.reverse();
    let edited = std::sync::Arc::new(edited);
    let pan = |at: (f64, f64)| pan_job(&doc, &key, at);
    let changed = |f: &dyn Fn(&mut crate::render_worker::Job)| {
        let mut job = pan((4.5, -4.0));
        f(&mut job);
        job
    };
    let steps: Vec<(&str, crate::render_worker::Job, bool)> = vec![
        ("still", pan((0.0, 0.0)), false),
        ("pan", pan((10.0, 0.0)), true),
        ("pan back", pan((3.0, -4.0)), true),
        ("part of a pixel", pan((3.5, -4.0)), false),
        ("whole again", pan((4.5, -4.0)), true),
        ("settled", changed(&|j| j.may_copy = false), false),
        ("pan after settling", pan((5.5, -4.0)), true),
        ("back before the edit", pan((4.5, -4.0)), true),
        (
            "edit",
            changed(&|j| {
                j.doc = edited.clone();
                j.key.revision += 1;
            }),
            false,
        ),
        ("undo", pan((4.5, -4.0)), false),
        (
            "zoom",
            changed(&|j| {
                j.key.zoom = 2.0;
                j.view = Affine::translate((4.5, -4.0)) * Affine::scale(2.0);
            }),
            false,
        ),
        (
            "resize",
            changed(&|j| {
                (j.key.w, j.key.h, j.w, j.h) = (200, 100, 200, 100);
            }),
            false,
        ),
        (
            "outline",
            changed(&|j| {
                j.key.outline = true;
                j.opts.outline = true;
            }),
            false,
        ),
        (
            "rich black",
            changed(&|j| {
                j.key.rich_black = true;
                j.opts.rich_black = true;
            }),
            false,
        ),
        ("setting back", pan((4.5, -4.0)), false),
        ("first pan after the setting", pan((5.5, -4.0)), false),
        ("next pan", pan((6.5, -4.0)), true),
    ];
    let mut r = vectorcraft_render::Renderer::new();
    let mut kept = None;
    for (name, job, copies) in steps {
        let frame = render_job(&mut r, &mut kept, &job);
        assert_eq!(frame.reused, copies, "{name}");
        let whole = vectorcraft_render::Renderer::new().render(&job.doc, job.w, job.h, job.view, &job.opts);
        if copies {
            let off = frame.img.pixels.iter().zip(&whole.pixels).map(|(a, b)| a.abs_diff(*b)).max();
            assert!(off.is_some_and(|d| d <= 1), "{name}: {off:?}");
        } else {
            assert_eq!(frame.img.pixels, whole.pixels, "{name}");
        }
    }
}

/// A pan over art the frame's size decides stays within one level of a fresh render of the frame,
/// because it renders whole: a full-page raster effect whose raster a part of the frame would make
/// at another scale, and a group drawn offscreen across the exposed part's edge with an Inner Glow
/// child inside the part, whose filter layer starts at another pixel in the part. The worker does
/// not keep such a frame for the pan to copy from.
#[test]
fn a_pan_over_art_the_frame_size_decides_renders_whole() {
    keeping_color_settings(a_pan_over_art);
}

fn a_pan_over_art() {
    let (_, key) = pan_doc();
    let effect = |effect: &str, params: serde_json::Value| {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 595, "height": 842})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 595, "height": 842})).unwrap()["id"].clone();
        app.run("paint.setFill", json!({"ids": [id], "color": "#1a4de6"})).unwrap();
        app.run("effect.apply", json!({"effect": effect, "ids": [id], "params": params})).unwrap();
        app.session.active().unwrap().doc.clone()
    };
    let group_glow = {
        use vectorcraft_doc::Appearance;
        use vectorcraft_doc::color::{BlendMode, Color, Paint};
        use vectorcraft_geom::{Rect, shapes::rectangle};
        let mut d = vectorcraft_doc::Document::new(1600.0, 1200.0);
        let basic = |c: Color| Appearance::basic(Paint::solid(c), Paint::None, 0.0);
        let wide = Node::path(d.alloc_id(), rectangle(Rect::new(100.0, 100.0, 1590.0, 900.0)), basic(Color::rgb(0.9, 0.8, 0.3)));
        let mut child = Node::path(d.alloc_id(), rectangle(Rect::new(1453.0, 200.0, 1583.0, 500.0)), basic(Color::rgb(0.1, 0.3, 0.9)));
        child.appearance.effects = vec![vectorcraft_doc::Effect { id: "stylize.innerGlow".into(), params: json!({"blur": 40}), visible: true }];
        child.blend = BlendMode::Multiply;
        let mut group = Node::group(d.alloc_id(), vec![std::sync::Arc::new(wide), std::sync::Arc::new(child)]);
        group.isolate = true;
        let layer = d.layers[0].id;
        d.insert(Some(layer), 0, group).unwrap();
        std::sync::Arc::new(d)
    };
    let cases = [
        ("Gaussian Blur 500 pt", effect("blur.gaussian", json!({"radius": 500})), (-200.0, -200.0), 2.0, (1440, 870)),
        ("Craquelure", effect("texture.craquelure", json!({})), (-200.0, -200.0), 10.0, (3840, 2160)),
        ("Inner Glow in a group drawn offscreen", group_glow, (0.0, 0.0), 1.0, (1600, 1200)),
    ];
    for (name, doc, (x, y), zoom, (w, h)) in cases {
        let job = |dx: f64| {
            let mut job = pan_job(&doc, &key, (dx, 0.0));
            (job.key.w, job.key.h, job.w, job.h, job.key.zoom) = (w, h, w, h, zoom);
            job.view = Affine::translate((x + dx, y)) * Affine::scale(zoom);
            job
        };
        let mut r = vectorcraft_render::Renderer::new();
        let mut kept = None;
        render_job(&mut r, &mut kept, &job(0.0));
        let pan = job(-40.0);
        let frame = render_job(&mut r, &mut kept, &pan);
        let whole = vectorcraft_render::Renderer::new().render(&pan.doc, pan.w, pan.h, pan.view, &pan.opts);
        let off = frame.img.pixels.iter().zip(&whole.pixels).map(|(a, b)| a.abs_diff(*b)).max();
        assert!(off.is_some_and(|d| d <= 1), "{name}: a channel differs by {off:?}");
        assert!(!frame.reused, "{name}: the pan renders whole");
        assert!(kept.is_none(), "{name}: the frame is not kept");
    }
}

/// The canvas asks the worker for a frame that may copy pixels when the view moves, keeps the whole
/// frame's time, and renders the frame whole once the view has been still for STILL_SECS, not
/// before and only once.
#[test]
fn a_frame_with_copied_pixels_renders_whole_once_the_view_stops() {
    keeping_color_settings(a_frame_with_copied_pixels);
}

fn a_frame_with_copied_pixels() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.canvas.worker_started = true;
    let (worker, jobs, done_tx) = crate::render_worker::Worker::test_channel();
    app.canvas.worker = Some(worker);
    let ctx = egui::Context::default();
    let frame = |app: &mut VectorcraftApp, t: f64| {
        let raw =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), time: Some(t), ..Default::default() };
        ctx.run_ui(raw, |ui| show(app, ui)).textures_delta.clear();
    };
    frame(&mut app, 1.0);
    done_tx.send(finished(&jobs.try_recv().expect("the first frame"))).unwrap();
    frame(&mut app, 1.0);
    assert_eq!(app.canvas.last_ms, 40.0);
    assert!(app.canvas.reused.is_none());

    app.view_mut().unwrap().center.x += 5.0;
    frame(&mut app, 2.0);
    let pan = jobs.try_recv().expect("the pan's frame goes to the worker");
    assert!(pan.may_copy, "a new view may copy");
    done_tx.send(crate::render_worker::Done { ms: 2.0, reused: true, ..finished(&pan) }).unwrap();
    frame(&mut app, 3.0);
    assert_eq!(app.canvas.reused, Some(3.0));
    assert_eq!(app.canvas.last_ms, 40.0, "a frame with copied pixels keeps the whole frame's time");
    assert_eq!(app.perf.render_ms, 2.0);

    frame(&mut app, 3.0 + STILL_SECS / 2.0);
    assert!(jobs.try_recv().is_err(), "the view has not been still long enough");
    frame(&mut app, 3.0 + STILL_SECS);
    let settle = jobs.try_recv().expect("the frame renders whole");
    assert!(!settle.may_copy, "the settling frame copies nothing");
    assert_eq!(settle.key, pan.key);
    frame(&mut app, 3.0 + STILL_SECS * 2.0);
    assert!(jobs.try_recv().is_err(), "queued once");
    done_tx.send(finished(&settle)).unwrap();
    frame(&mut app, 4.0);
    assert!(app.canvas.reused.is_none());
    frame(&mut app, 5.0);
    assert!(jobs.try_recv().is_err(), "nothing to render after it");
}

/// The frame that renders whole after a pan stops copies nothing and stays on the worker, so the
/// first step of the next pan copies from it. The jobs are the ones the canvas asks for.
#[test]
fn a_pan_after_the_settling_frame_copies() {
    keeping_color_settings(a_pan_after_the_settling_frame);
}

fn a_pan_after_the_settling_frame() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.canvas.worker_started = true;
    let (worker, jobs, done_tx) = crate::render_worker::Worker::test_channel();
    app.canvas.worker = Some(worker);
    let ctx = egui::Context::default();
    let frame = |app: &mut VectorcraftApp, t: f64| {
        let raw =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), time: Some(t), ..Default::default() };
        ctx.run_ui(raw, |ui| show(app, ui)).textures_delta.clear();
    };
    let mut r = vectorcraft_render::Renderer::new();
    let mut kept = None;
    let mut render = |name: &str, copies: bool| {
        let job = jobs.try_recv().unwrap_or_else(|_| panic!("{name}: the canvas asks for a frame"));
        let f = render_job(&mut r, &mut kept, &job);
        assert_eq!(f.reused, copies, "{name}");
        let done = crate::render_worker::Done { key: job.key, doc: job.doc, opts: job.opts, img: f.img, ms: 40.0, moved: f.moved, reused: f.reused };
        done_tx.send(done).unwrap();
    };
    frame(&mut app, 1.0);
    render("the first frame", false);
    frame(&mut app, 1.0);
    // At 100% the view moves by whole device pixels.
    app.view_mut().unwrap().zoom = 1.0;
    frame(&mut app, 1.5);
    render("the zoom", false);
    frame(&mut app, 1.5);

    app.view_mut().unwrap().center.x += 5.0;
    frame(&mut app, 1.75);
    render("the first pan after the zoom", false);
    frame(&mut app, 1.75);

    app.view_mut().unwrap().center.x += 5.0;
    frame(&mut app, 2.0);
    render("the pan", true);
    frame(&mut app, 3.0);
    frame(&mut app, 3.0 + STILL_SECS);
    render("the settling frame", false);
    frame(&mut app, 4.0);

    app.view_mut().unwrap().center.x += 5.0;
    frame(&mut app, 5.0);
    render("the next pan", true);
}

/// A frame after a zoom or an edit holds no pixels, and the frame before it lets its pixels go. The
/// first pan step after it renders whole and holds its pixels, and the next step copies them.
#[test]
fn a_frame_after_a_zoom_or_an_edit_holds_no_pixels() {
    let (doc, key) = pan_doc();
    let mut edited = (*doc).clone();
    edited.layers.reverse();
    let edited = std::sync::Arc::new(edited);
    let zoomed = |key: &CacheKey, at: f64, zoom: f64| {
        let mut job = pan_job(&doc, key, (at, 0.0));
        job.key.zoom = zoom;
        job.view = Affine::translate((at, 0.0)) * Affine::scale(zoom);
        job
    };
    let revised = CacheKey { revision: key.revision + 1, ..key.clone() };
    let steps: Vec<(&str, crate::render_worker::Job, bool, bool)> = vec![
        ("the first frame", pan_job(&doc, &key, (0.0, 0.0)), false, true),
        ("a zoom", zoomed(&key, 0.0, 2.0), false, false),
        ("another zoom", zoomed(&key, 0.0, 1.5), false, false),
        ("the first pan after the zoom", zoomed(&key, 10.0, 1.5), false, true),
        ("the next pan", zoomed(&key, 20.0, 1.5), true, true),
        ("an edit", crate::render_worker::Job { doc: edited.clone(), ..pan_job(&doc, &revised, (0.0, 0.0)) }, false, false),
        ("the first pan after the edit", crate::render_worker::Job { doc: edited.clone(), ..pan_job(&doc, &revised, (10.0, 0.0)) }, false, true),
        ("the next pan after the edit", crate::render_worker::Job { doc: edited.clone(), ..pan_job(&doc, &revised, (20.0, 0.0)) }, true, true),
    ];
    let mut r = vectorcraft_render::Renderer::new();
    let mut kept = None;
    for (name, job, copies, holds) in steps {
        assert_eq!(render_job(&mut r, &mut kept, &job).reused, copies, "{name}: copied");
        assert_eq!(kept.as_ref().is_some_and(|k| k.pixels.is_some()), holds, "{name}: holds its pixels");
    }
}

/// The worker thread itself copies pixels on a pan and reports it.
#[test]
fn the_render_worker_thread_copies_a_pan() {
    let (doc, key) = pan_doc();
    let mut worker = crate::render_worker::Worker::spawn(egui::Context::default()).expect("a worker");
    // Blocks until the worker sends its frame. A worker thread that stopped gives None.
    let wait = |worker: &mut crate::render_worker::Worker| worker.wait(std::time::Duration::MAX).expect("the worker sends the frame");
    worker.submit(pan_job(&doc, &key, (0.0, 0.0)));
    assert!(!wait(&mut worker).reused);
    worker.submit(pan_job(&doc, &key, (10.0, 0.0)));
    assert!(wait(&mut worker).reused);
}

/// #4155: a document's first frame renders on the render worker, not on the UI thread, where a
/// heavy one froze the window for seconds while it opened. The canvas shows the document's paper
/// meanwhile, does not queue the frame again while it renders, and shows it once it lands.
#[test]
fn a_documents_first_frame_renders_on_the_worker() {
    keeping_color_settings(a_documents_first_frame);
}

fn a_documents_first_frame() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.canvas.worker_started = true;
    let (worker, jobs, done_tx) = crate::render_worker::Worker::test_channel();
    app.canvas.worker = Some(worker);
    let ctx = egui::Context::default();
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
    let frame = |app: &mut VectorcraftApp| ctx.run_ui(raw.clone(), |ui| show(app, ui)).textures_delta.clear();

    frame(&mut app);
    let job = jobs.try_recv().expect("the first frame is queued on the worker");
    assert!(app.canvas.texture.is_none(), "not rendered on the UI thread");
    frame(&mut app);
    assert!(jobs.try_recv().is_err(), "not queued again while it renders");
    assert!(app.canvas.texture.is_none());

    done_tx.send(finished(&job)).unwrap();
    frame(&mut app);
    assert!(app.canvas.texture.is_some(), "shown once it lands");
    assert_eq!(app.canvas.key.as_ref(), Some(&job.key));
    assert!(app.canvas.from_worker);
}

/// The canvas waits for a document's first frame only briefly (`FIRST_FRAME_GRACE`): `wait`
/// returns a frame finished before or during the wait, clearing the pending job, and nothing once
/// the time is up.
#[test]
fn the_render_worker_waits_for_a_frame_only_as_long_as_asked() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.canvas.worker_started = true;
    let (worker, jobs, _) = crate::render_worker::Worker::test_channel();
    app.canvas.worker = Some(worker);
    let ctx = egui::Context::default();
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(200.0, 150.0))), ..Default::default() };
    ctx.run_ui(raw, |ui| show(&mut app, ui)).textures_delta.clear();
    let job = jobs.try_recv().expect("a job to finish");

    let (mut w, _, done_tx) = crate::render_worker::Worker::test_channel();
    let t0 = std::time::Instant::now();
    assert!(w.wait(std::time::Duration::from_millis(20)).is_none());
    assert!(t0.elapsed() >= std::time::Duration::from_millis(20), "waited the time asked");

    w.pending = Some(job.key.clone());
    done_tx.send(finished(&job)).unwrap();
    assert_eq!(w.wait(std::time::Duration::from_secs(5)).map(|d| d.key), Some(job.key.clone()), "finished before the wait");
    assert!(w.pending.is_none());

    let late = finished(&job);
    let sender = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(30));
        done_tx.send(late).unwrap();
    });
    assert!(w.wait(std::time::Duration::from_secs(5)).is_some(), "finished during the wait");
    sender.join().unwrap();
}

/// #3901: with Direct Selection, the Selection tool's bounding box shows while Cmd/Ctrl is held
/// (a press then takes its handles), and not otherwise.
#[test]
fn direct_selection_shows_the_bounding_box_while_cmd_is_held() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 50})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.select_tool("directSelection");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let corner = xf.to_screen(Point::new(200.0, 150.0));
    // A frame with `modifiers` held → is a white box handle drawn at the bottom-right corner?
    let handle_drawn = |app: &mut VectorcraftApp, modifiers: egui::Modifiers| {
        let screen = Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)));
        let raw = egui::RawInput { screen_rect: screen, events: vec![egui::Event::ModifiersChanged(modifiers)], ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear(); // the render worker's frame can land in any of these
        fn flat(s: Shape, out: &mut Vec<Shape>) {
            match s {
                Shape::Vec(v) => v.into_iter().for_each(|s| flat(s, out)),
                s => out.push(s),
            }
        }
        let mut v = vec![];
        out.shapes.into_iter().for_each(|c| flat(c.shape, &mut v));
        v.iter().any(|s| matches!(s, Shape::Rect(r) if r.fill == Color32::WHITE && (r.rect.width() - 6.0).abs() < 1.0 && r.rect.center().distance(corner) < 0.5))
    };
    assert!(!handle_drawn(&mut app, egui::Modifiers::NONE), "Direct Selection alone shows no box");
    assert!(handle_drawn(&mut app, egui::Modifiers::COMMAND), "Cmd held shows the Selection tool's box");
    assert!(!handle_drawn(&mut app, egui::Modifiers::NONE), "gone once Cmd is released");
    assert!(app.ui.view.bounding_box, "View › Show Bounding Box is unchanged");
}

/// #5239: a double-click on a ruler adds a guide at the pointer: a vertical one on the top ruler
/// at its x, a horizontal one on the left ruler at its y, as one undo step, shown even when
/// guides were hidden. The first click of the pair adds nothing.
#[test]
fn double_clicking_a_ruler_adds_a_guide_at_that_position() {
    // The ruler to click, the guide's orientation and its position, the pointer at that position.
    for (top, vertical, pos) in [(true, true, 120.0), (false, false, 80.0)] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.ui.view.rulers = true;
        app.ui.view.guides = false;
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let view = app.view_mut().unwrap();
        view.zoom = 2.0;
        view.rotation = 0.0;
        view.center = Point::new(200.0, 150.0);
        frame(&mut app, &ctx, vec![]);
        let rect = app.canvas_rect.unwrap();
        let xf = Xf::new(rect, app.view().unwrap());
        let at = if top {
            pos2(xf.to_screen(Point::new(pos, 0.0)).x, rect.top() - RULER / 2.0)
        } else {
            pos2(rect.left() - RULER / 2.0, xf.to_screen(Point::new(0.0, pos)).y)
        };
        let guides = |app: &mut VectorcraftApp| {
            let list = app.session.execute("guide.list", &json!({})).unwrap();
            list.as_array().unwrap().iter().map(|g| (g["vertical"].as_bool().unwrap(), g["pos"].as_f64().unwrap())).collect::<Vec<_>>()
        };
        let steps = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
        let before = steps(&app);
        let click = |app: &mut VectorcraftApp| {
            let button =
                |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            frame(app, &ctx, vec![egui::Event::PointerMoved(at)]);
            frame(app, &ctx, vec![button(true)]);
            frame(app, &ctx, vec![button(false)]);
        };
        click(&mut app);
        assert_eq!(guides(&mut app), vec![], "a single click adds none");
        click(&mut app);
        let got = guides(&mut app);
        assert_eq!(got.len(), 1, "top ruler: {top}, {got:?}");
        assert!(got[0].0 == vertical && (got[0].1 - pos).abs() < 0.01, "top ruler: {top}, want vertical {vertical} at {pos}: {got:?}");
        assert!(app.ui.view.guides, "the new guide is shown");
        assert_eq!(steps(&app), before + 1, "one undo step");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(guides(&mut app), vec![]);
    }
}

/// A fresh app with a 400 x 300 document, rulers on, the view at zoom 2 turned `rotation` degrees,
/// and its first frame drawn.
fn ruler_guide_app(rotation: f64) -> (VectorcraftApp, egui::Context) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.ui.view.rulers = true;
    app.ui.view.guides = false;
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let view = app.view_mut().unwrap();
    view.zoom = 2.0;
    view.rotation = rotation;
    view.center = Point::new(200.0, 150.0);
    frame(&mut app, &ctx, vec![]);
    (app, ctx)
}

/// One primary click at `at`, as three frames (move, press, release).
fn click_at(app: &mut VectorcraftApp, ctx: &egui::Context, at: Pos2) {
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(app, ctx, vec![egui::Event::PointerMoved(at)]);
    frame(app, ctx, vec![button(true)]);
    frame(app, ctx, vec![button(false)]);
}

/// The ruler guides as (vertical, position).
fn guide_list(app: &mut VectorcraftApp) -> Vec<(bool, f64)> {
    let list = app.session.execute("guide.list", &json!({})).unwrap();
    list.as_array().unwrap().iter().map(|g| (g["vertical"].as_bool().unwrap(), g["pos"].as_f64().unwrap())).collect()
}

/// An edit to a 3D effect renders the part it changed on this thread, measured by the renderer
/// that drew the frame shown ([`vectorcraft_render::Renderer::changed`]): it draws the part with
/// the 3D art it made to measure it.
#[test]
fn an_edit_to_a_3d_effect_redraws_with_the_art_measured() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    let id = app.run("shape.rectangle", json!({"x": 380, "y": 280, "width": 40, "height": 40})).unwrap()["id"].clone();
    app.run("effect.apply", json!({"effect": "threeD.extrude", "ids": [id], "params": {"depth": 60.0}})).unwrap();
    let ctx = egui::Context::default();
    shapes(&mut app, &ctx);
    assert_eq!(app.canvas.renderer.stats.three_d_built, 1, "the whole frame made the art");
    app.run("effect.setParams", json!({"ids": [id], "index": 0, "item": null, "params": {"depth": 5.0}})).unwrap();
    // Drawing only a part keeps the whole frame's time; drawing it whole measures it again.
    app.canvas.last_ms = 7.25;
    shapes(&mut app, &ctx);
    assert!(!app.canvas.from_worker);
    assert_eq!(app.canvas.last_ms, 7.25, "only the part that changed was drawn");
    assert_eq!(app.canvas.renderer.stats.three_d_built, 0, "the part was drawn with the art made to measure the edit");
}

/// An edit to a 3D effect off the view, and one the render worker draws, leave none of the art made
/// to measure them held by this thread's renderer.
#[test]
fn art_measured_for_an_edit_not_drawn_here_is_let_go() {
    for worker in [false, true] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
        app.canvas.worker_started = true; // render on this thread
        // Off the view, or filling it, for the worker to draw.
        let (x, size) = if worker { (0, 800) } else { (5000, 40) };
        let id = app.run("shape.rectangle", json!({"x": x, "y": 0, "width": size, "height": size})).unwrap()["id"].clone();
        app.run("effect.apply", json!({"effect": "threeD.extrude", "ids": [id], "params": {"depth": 60.0}})).unwrap();
        let ctx = egui::Context::default();
        shapes(&mut app, &ctx);
        // A worker whose jobs the test receives: none finishes unless the test says so.
        let mut jobs = None;
        if worker {
            let (w, rx, done) = crate::render_worker::Worker::test_channel();
            app.canvas.worker = Some(w);
            jobs = Some((rx, done));
            app.canvas.last_ms = 100.0;
        }
        app.run("effect.setParams", json!({"ids": [id], "index": 0, "item": null, "params": {"depth": 5.0}})).unwrap();
        // Measured through the canvas's renderer, the edit holds the art made for it.
        let (shown, opts) = app.canvas.shown.clone().unwrap();
        let now = app.session.active().unwrap().doc.clone();
        app.canvas.renderer.changed(&shown, &now, &opts);
        assert!(app.canvas.renderer.measured_bytes() > 0, "worker {worker}: art held");
        if !worker {
            app.canvas.last_ms = 7.25;
        }
        shapes(&mut app, &ctx);
        if !worker {
            // Nothing in view to draw: no frame, whole or part, began since the measure.
            assert_eq!(app.canvas.last_ms, 7.25, "not drawn whole");
            assert!(app.canvas.renderer.stats.three_d_built > 0, "no part drawn");
        }
        if let Some((rx, _)) = &jobs {
            assert!(rx.try_recv().is_ok(), "the worker draws it");
        }
        assert_eq!(app.canvas.renderer.measured_bytes(), 0, "worker {worker}");
        app.canvas.worker = None;
    }
}

/// The shapes Edit › Spelling › Auto Spell Check draws in one frame, seen through `xf`.
fn spelling_lines(app: &mut VectorcraftApp, ctx: &egui::Context, xf: &Xf) -> Vec<Shape> {
    let raw = egui::RawInput { screen_rect: Some(xf.rect), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| spelling_overlay(app, ui.painter(), xf));
    out.textures_delta.clear();
    out.shapes.into_iter().map(|c| c.shape).collect()
}

/// A document with one point type object of `text` at the top left, and a view of it.
fn spelling_doc(text: &str) -> (VectorcraftApp, u64, Xf) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 20, "y": 60, "text": text})).unwrap()["id"].as_u64().unwrap();
    let xf = Xf { rect: egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)), zoom: 1.0, center: Point::new(200.0, 150.0), rot: 0.0 };
    (app, id, xf)
}

/// Auto Spell Check draws one wavy line under each unknown word, under its glyphs, and none under
/// the words the dictionaries know.
#[test]
fn auto_spell_check_underlines_the_unknown_words() {
    let ctx = egui::Context::default();
    let (mut app, _, xf) = spelling_doc("the cat sat");
    assert!(spelling_lines(&mut app, &ctx, &xf).is_empty(), "every word is known");
    let (mut app, id, xf) = spelling_doc("the Qzvbtrw sat\nZvqtrbx");
    let lines = spelling_lines(&mut app, &ctx, &xf);
    assert_eq!(lines.len(), 2, "one line under each unknown word: {lines:?}");
    // The first line runs under the second word: below the type's top and within its width.
    let bounds = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap();
    let on_screen = xf.rect_to_screen(bounds);
    let Shape::Path(first) = &lines[0] else { panic!("{:?}", lines[0]) };
    let xs = || first.points.iter().map(|p| p.x);
    let (left, right) = (xs().fold(f32::MAX, f32::min), xs().fold(f32::MIN, f32::max));
    assert!(left > on_screen.left() + 5.0 && right < on_screen.right() - 5.0, "under the middle word: {left}..{right} in {on_screen:?}");
    assert!(first.points.iter().all(|p| p.y > on_screen.top() && p.y < on_screen.center().y + on_screen.height()), "{first:?}");
    assert!(first.points.windows(2).any(|w| w[0].y != w[1].y), "wavy");
}

/// The lines follow the type and the dictionaries: an edit, Ignore All and Add to Dictionary each
/// change what is underlined in the next frame, and a frame with no change checks no type again.
#[test]
fn auto_spell_check_follows_edits_ignore_all_and_the_dictionary() {
    let ctx = egui::Context::default();
    let (mut app, id, xf) = spelling_doc("Wqzxvtr and Pzxqvrt");
    let kept = |app: &VectorcraftApp| app.canvas.spelling.lines[&vectorcraft_doc::NodeId(id)].1.clone();
    assert_eq!(spelling_lines(&mut app, &ctx, &xf).len(), 2);
    // Tests running alongside add fonts and words, which checks all type again: frames are drawn
    // until neither changed between two of them.
    let again = (0..100).any(|_| {
        let (inputs, before) = (app.canvas.spelling.inputs.clone(), kept(&app));
        spelling_lines(&mut app, &ctx, &xf);
        app.canvas.spelling.inputs == inputs && std::sync::Arc::ptr_eq(&kept(&app), &before)
    });
    assert!(again, "a frame keeps the lines of the frame before");
    app.session.execute("text.setText", &json!({"id": id, "text": "Wqzxvtr and Pzxqvrt or Qxzvwtr"})).unwrap();
    assert_eq!(spelling_lines(&mut app, &ctx, &xf).len(), 3, "the edit is checked");
    app.session.execute("text.spelling.ignoreAll", &json!({"word": "wqzxvtr"})).unwrap();
    assert_eq!(spelling_lines(&mut app, &ctx, &xf).len(), 2, "Ignore All");
    app.session.execute("text.spelling.addWord", &json!({"word": "Pzxqvrt"})).unwrap();
    let after_add = spelling_lines(&mut app, &ctx, &xf).len();
    app.session.execute("text.spelling.removeWord", &json!({"word": "pzxqvrt"})).unwrap();
    assert_eq!(after_add, 1, "Add to Dictionary");
    assert_eq!(spelling_lines(&mut app, &ctx, &xf).len(), 2, "deleted from the dictionary again");
}

/// Type with an effect shown, and hidden type, has no lines: the helpx page leaves type with
/// effects out of Auto Spell Check.
#[test]
fn auto_spell_check_leaves_out_type_with_effects_and_hidden_type() {
    let ctx = egui::Context::default();
    let (mut app, id, xf) = spelling_doc("Xqzvtrw");
    assert_eq!(spelling_lines(&mut app, &ctx, &xf).len(), 1);
    app.session.execute("effect.apply", &json!({"ids": [id], "effect": "distort.roughen", "params": {"size": 1}})).unwrap();
    assert!(spelling_lines(&mut app, &ctx, &xf).is_empty(), "an effect");
    app.session.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(spelling_lines(&mut app, &ctx, &xf).len(), 1, "undone");
    app.session.execute("object.hide", &json!({"ids": [id]})).unwrap();
    assert!(spelling_lines(&mut app, &ctx, &xf).is_empty(), "hidden");
}

/// Edit › Spelling › Auto Spell Check is a toggle, off at first: on, the canvas draws the lines;
/// off again, it draws none and keeps no text node.
#[test]
fn auto_spell_check_is_a_toggle_of_the_canvas_lines() {
    let ctx = egui::Context::default();
    let (mut app, _, _) = spelling_doc("Yqzvtrx");
    let red = |s: &Shape| matches!(s, Shape::Path(p) if p.stroke.color == egui::epaint::ColorMode::Solid(Color32::from_rgb(0xe0, 0x20, 0x20)));
    let count = |app: &mut VectorcraftApp| shapes(app, &ctx).iter().filter(|s| red(s)).count();
    assert!(!app.ui.view.auto_spell_check, "off at first");
    assert_eq!(count(&mut app), 0);
    assert_eq!(app.run("edit.autoSpellCheck", json!({})).unwrap(), json!(true));
    assert_eq!(count(&mut app), 1, "on");
    assert_eq!(app.run("edit.autoSpellCheck", json!({})).unwrap(), json!(false));
    assert_eq!(count(&mut app), 0, "off");
    assert!(app.canvas.spelling.lines.is_empty(), "no node is kept");
}

/// #8363: Trim View hides the ruler guides, the guide paths and the grid while it is on, and they
/// are neither snapped to nor picked, as hidden guides are; turned off, they come back as they
/// were. The rulers stay.
#[test]
fn trim_view_hides_the_guides_and_the_grid() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.canvas.worker_started = true; // render on this thread
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("guide.add", &json!({"vertical": true, "pos": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 150, "y": 100, "width": 100, "height": 100})).unwrap();
    app.run("view.guides.make", json!({})).unwrap();
    app.run("select.none", json!({})).unwrap();
    (app.ui.view.grid, app.ui.view.guides, app.ui.view.rulers) = (true, true, true);
    let (red, green) = (Color32::from_rgb(255, 0, 0), Color32::from_rgb(0, 255, 0));
    app.session.execute("prefs.set", &json!({"values": {"gridColor": "#ff0000", "guideColor": "#00ff00"}})).unwrap();
    let ctx = egui::Context::default();
    // → (a gridline, a ruler guide, guide paths in the art, guides snapped to and picked).
    let drawn = |app: &mut VectorcraftApp| {
        let s = shapes(app, &ctx);
        let line = |c: Color32| s.iter().any(|s| matches!(s, Shape::LineSegment { stroke, .. } if stroke.color == c));
        let guide_paths = !app.canvas.shown.as_ref().unwrap().1.hide_guides;
        (line(red), line(green), guide_paths, app.view_info().guides)
    };
    assert_eq!(drawn(&mut app), (true, true, true, true), "Trim View off: all of them");

    app.run("view.trimView", json!({})).unwrap();
    assert_eq!(drawn(&mut app), (false, false, false, false), "Trim View on: none of them");
    assert!(app.ui.view.grid && app.ui.view.guides && app.ui.view.rulers, "the choices stay, and so do the rulers");

    app.run("view.trimView", json!({})).unwrap();
    assert_eq!(drawn(&mut app), (true, true, true, true), "Trim View off: back as they were");

    // Hidden before, they stay hidden after.
    app.run("view.guides", json!({})).unwrap();
    app.run("view.trimView", json!({})).unwrap();
    app.run("view.trimView", json!({})).unwrap();
    assert_eq!(drawn(&mut app), (true, false, false, false), "guides hidden before Trim View stay hidden");
}

/// #8363: a selected guide path is deselected when Trim View hides it, as Hide Guides does (#3219).
#[test]
fn trim_view_deselects_selected_guide_paths() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.run("view.guides.make", json!({})).unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    assert_eq!(app.session.active().unwrap().selection.objects, vec![vectorcraft_doc::NodeId(id)]);

    app.run("view.trimView", json!({})).unwrap();

    assert!(app.session.active().unwrap().selection.objects.is_empty());
}

/// #8363: Trim View hides the guides, so a guide dragged or double-clicked out of a ruler while it
/// is on would be made but never seen. None is: no preview line, no guide, no undo step, the
/// stored Show/Hide Guides choice is left alone, and the status line says why. A drag already
/// going when Trim View turns on ends as a release off the canvas does.
#[test]
fn trim_view_makes_no_guide_from_the_rulers() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.ui.view.rulers = true;
    app.ui.view.guides = false;
    app.ui.view.smart_guides = false;
    app.ui.view.snap_to_point = false;
    app.ui.view.trim_view = true;
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let rect = app.canvas_rect.unwrap();
    let from = pos2(rect.center().x, rect.top() - RULER / 2.0);
    let at = pos2(rect.center().x, rect.top() + 120.0);
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let across = |s: &Shape| {
        matches!(s, Shape::LineSegment { points, .. }
            if (points[0].y - at.y).abs() <= 2.0 && points[0].y == points[1].y && (points[1].x - points[0].x).abs() >= rect.width() - 0.5)
    };
    let count = |app: &VectorcraftApp| app.session.active().unwrap().doc.ruler_guides().len();
    let steps = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
    let before = steps(&app);

    // A drag: nothing previews and nothing is made.
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from)]);
    frame(&mut app, &ctx, vec![button(from, true)]);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
    assert!(!shapes(&mut app, &ctx).iter().any(across), "no guide previews");
    assert!(!app.session.ruler_guide_dragging());
    frame(&mut app, &ctx, vec![button(at, false)]);
    assert_eq!(count(&app), 0, "no guide from a drag");
    assert!(!app.ui.view.guides, "the stored choice is left alone");
    assert_eq!(app.ui.status, crate::menus::TRIM_VIEW_HIDES);

    // A double-click: nothing either.
    app.ui.status.clear();
    let click = |app: &mut VectorcraftApp| {
        frame(app, &ctx, vec![egui::Event::PointerMoved(from)]);
        frame(app, &ctx, vec![button(from, true)]);
        frame(app, &ctx, vec![button(from, false)]);
    };
    click(&mut app);
    click(&mut app);
    assert_eq!(count(&app), 0, "no guide from a double-click");
    assert!(!app.ui.view.guides);
    assert_eq!(app.ui.status, crate::menus::TRIM_VIEW_HIDES);
    assert_eq!(steps(&app), before, "no undo step");

    // Trim View turned on during a drag: the drag ends without a guide.
    app.ui.view.trim_view = false;
    let from2 = pos2(rect.center().x + 30.0, rect.top() - RULER / 2.0);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from2)]);
    frame(&mut app, &ctx, vec![button(from2, true)]);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
    assert!(app.session.ruler_guide_dragging(), "the drag started with Trim View off");
    app.ui.view.trim_view = true;
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(pos2(at.x + 5.0, at.y))]);
    assert!(!app.session.ruler_guide_dragging(), "ended");
    assert!(!shapes(&mut app, &ctx).iter().any(across), "no guide previews");
    frame(&mut app, &ctx, vec![button(at, false)]);
    assert_eq!(count(&app), 0, "no guide from the drag Trim View interrupted");
    assert_eq!(steps(&app), before, "no undo step");

    // With Trim View off again, a drag makes a guide as before.
    app.ui.view.trim_view = false;
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from)]);
    frame(&mut app, &ctx, vec![button(from, true)]);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
    frame(&mut app, &ctx, vec![button(at, false)]);
    assert_eq!(count(&app), 1);
    assert!(app.ui.view.guides, "the new guide shows");
}

/// #8363: a guide drag Trim View cancelled stays cancelled until the button is released. Trim
/// View turned off again before the release does not start a new guide: the release makes none
/// and adds no undo step. The next drag works as before.
#[test]
fn a_ruler_drag_trim_view_cancelled_stays_cancelled_until_released() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.ui.view.rulers = true;
    app.ui.view.smart_guides = false;
    app.ui.view.snap_to_point = false;
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let rect = app.canvas_rect.unwrap();
    let from = pos2(rect.center().x, rect.top() - RULER / 2.0);
    let at = pos2(rect.center().x, rect.top() + 120.0);
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let count = |app: &VectorcraftApp| app.session.active().unwrap().doc.ruler_guides().len();
    let steps = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
    let before = steps(&app);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from)]);
    frame(&mut app, &ctx, vec![button(from, true)]);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
    assert!(app.session.ruler_guide_dragging());
    app.ui.view.trim_view = true;
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(pos2(at.x, at.y + 5.0))]);
    assert!(!app.session.ruler_guide_dragging(), "Trim View ends the drag");
    app.ui.view.trim_view = false;
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(pos2(at.x, at.y + 10.0))]);
    assert!(!app.session.ruler_guide_dragging(), "turned off again, no new guide starts");
    frame(&mut app, &ctx, vec![button(pos2(at.x, at.y + 10.0), false)]);
    assert_eq!(count(&app), 0, "the release makes no guide");
    assert_eq!(steps(&app), before, "no undo step");
    // The next drag makes a guide.
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from)]);
    frame(&mut app, &ctx, vec![button(from, true)]);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
    frame(&mut app, &ctx, vec![button(at, false)]);
    assert_eq!(count(&app), 1);
}

/// #8363: Trim View turned on or off during a drag of the rulers' origin out of their corner
/// cancels it, as hiding the rulers does: the snap targets gathered at its start would hold
/// guides Trim View hides. The origin is left where it was. A drag wholly with Trim View on, or
/// wholly off, sets it.
#[test]
fn trim_view_toggled_during_a_ruler_origin_drag_cancels_it() {
    for (start, end, sets) in [(false, true, false), (true, false, false), (true, true, true), (false, false, true)] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.session.execute("guide.add", &json!({"vertical": true, "pos": 150})).unwrap();
        app.ui.view.rulers = true;
        app.ui.view.trim_view = start;
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let rect = app.canvas_rect.unwrap();
        let xf = Xf::new(rect, app.view().unwrap());
        let origin = |app: &VectorcraftApp| {
            let d = &app.session.active().unwrap().doc;
            format!("{:?} {:?}", d.ruler_origin, d.artboard_ruler_origins)
        };
        let was = origin(&app);
        let from = pos2(rect.left() - RULER / 2.0, rect.top() - RULER / 2.0);
        let at = xf.to_screen(Point::new(151.0, 100.0));
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from)]);
        frame(&mut app, &ctx, vec![button(from, true)]);
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
        assert!(crate::canvas::ruler_origin_dragging(&ctx), "{start} {end}: dragging");
        app.ui.view.trim_view = end;
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at + vec2(1.0, 0.0))]);
        frame(&mut app, &ctx, vec![button(at + vec2(1.0, 0.0), false)]);
        assert_eq!(origin(&app) != was, sets, "Trim View {start} then {end}: origin set {sets}");
    }
}

/// #8363: a Selection move begun with the guides shown gathers the ruler guides to snap to. When
/// they stop showing during the move (Trim View or Hide Guides turned on), it no longer snaps to
/// them; with them shown throughout, it does.
#[test]
fn a_move_stops_snapping_to_guides_hidden_during_it() {
    // What hides the guides mid-move (none: they stay shown), and whether the move then snaps.
    for (hide, snaps) in [(None, true), (Some("trim"), false), (Some("guides"), false)] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 50, "height": 50})).unwrap();
        app.session.execute("guide.add", &json!({"vertical": true, "pos": 200})).unwrap();
        app.select_tool("selection");
        app.ui.view.smart_guides = false;
        app.ui.view.snap_to_point = true;
        app.ui.view.guides = true;
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let rect = app.canvas_rect.unwrap();
        let xf = Xf::new(rect, app.view().unwrap());
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        // Grabbed at its centre, moved first well away from the guide, then 1.5 px short of it.
        let grab = xf.to_screen(Point::new(75.0, 75.0));
        let away = xf.to_screen(Point::new(120.0, 75.0));
        let near = xf.to_screen(Point::new(200.0, 75.0)) - vec2(1.5, 0.0);
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(grab)]);
        frame(&mut app, &ctx, vec![button(grab, true)]);
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(away)]);
        match hide {
            Some("trim") => app.ui.view.trim_view = true,
            Some(_) => app.ui.view.guides = false,
            None => {}
        }
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(near)]);
        frame(&mut app, &ctx, vec![button(near, false)]);
        let b = app.session.active().unwrap().doc.art_bounds().unwrap();
        let centre = (b.x0 + b.x1) / 2.0;
        assert_eq!((centre - 200.0).abs() < 1e-6, snaps, "{hide:?}: the grabbed centre at {centre}");
        assert!((centre - xf.to_doc(near).x).abs() < 1e-6 || snaps, "{hide:?}: unsnapped, it follows the pointer: {centre}");
    }
}
