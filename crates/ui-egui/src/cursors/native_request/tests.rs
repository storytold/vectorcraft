use super::*;

#[test]
fn every_glyph_and_rotation_preserves_pixels_and_logical_hotspots() {
    let cursors = super::super::GLYPHS.into_iter().chain((0..8).map(Cursor::RotateToward));
    for cursor in cursors {
        for scale in [0.75, 1.0, 1.25, 1.5, 2.0] {
            let bitmap = prepare(cursor, scale).unwrap();
            let [one, two] = &bitmap.representations;
            assert_eq!(two.size, one.size.map(|x| x * 2));
            assert_eq!(two.hotspot, one.hotspot.map(|x| x * 2));
            for (i, dest) in bitmap.representations.iter().enumerate() {
                let source = rasterize(cursor, scale * (i + 1) as f32).unwrap();
                let dx = dest.hotspot[0] - source.hotspot[0];
                let dy = dest.hotspot[1] - source.hotspot[1];
                for y in 0..dest.size[1] {
                    for x in 0..dest.size[0] {
                        let offset = (usize::from(y) * usize::from(dest.size[0]) + usize::from(x)) * 4;
                        let actual = &dest.rgba[offset..offset + 4];
                        if x >= dx && x < dx + source.size[0] && y >= dy && y < dy + source.size[1] {
                            let at = (usize::from(y - dy) * usize::from(source.size[0]) + usize::from(x - dx)) * 4;
                            assert_eq!(actual, &source.rgba[at..at + 4], "{cursor:?}, {scale}");
                        } else {
                            assert_eq!(actual, [0; 4]);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn invalid_inputs_and_system_states_are_bounded_and_cached() {
    let mut cache = Cache::default();
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::MAX, 100.0] {
        assert!(cache.get(Cursor::Arrow, scale).is_none());
    }
    for cursor in [Cursor::Busy, Cursor::NotAllowed, Cursor::ScissorsNotAllowed] {
        assert!(cache.get(cursor, 1.0).is_none());
        assert!(cache.get(cursor, 1.0).is_none());
    }
    assert_eq!(cache.0.len(), 3);
    let good = rasterize(Cursor::Pen, 1.0).unwrap();
    for bad in [
        CustomCursorImage { rgba: Arc::from([]), ..good.clone() },
        CustomCursorImage { size: [u16::MAX, 1], ..good.clone() },
        CustomCursorImage { hotspot: good.size, ..good.clone() },
    ] {
        assert!(align([bad, good.clone()]).is_none());
    }
    let (_, retained) = cache.get(Cursor::Pen, 1.0).unwrap();
    assert!(Arc::ptr_eq(&retained, &cache.get(Cursor::Pen, 1.0).unwrap().1));
    for i in 0..140 {
        cache.get(Cursor::Pen, 1.0 + i as f32 / 1000.0);
    }
    assert!(cache.0.len() <= CAPACITY);
    assert!(valid(&retained.representations[0]));
}

#[test]
fn requests_are_consumed_per_viewport_and_expire_on_empty_passes() {
    let ctx = Context::default();
    let raw = |id| {
        let mut input = egui::RawInput { viewport_id: id, events: vec![egui::Event::PointerMoved(egui::pos2(10.0, 10.0))], ..Default::default() };
        input.viewports.entry(id).or_default();
        input
    };
    let root = ViewportId::ROOT;
    let panel = ViewportId::from_hash_of("panel");
    let _ = headless(&ctx, raw(root), |ui| {
        request(ui.ctx(), Cursor::Pen, CursorIcon::Crosshair, true, 1.0);
    });
    let _ = headless(&ctx, raw(panel), |ui| {
        clear(ui.ctx());
        assert!(take(ui.ctx()).is_none());
    });
    assert!(ctx.data(|d| d.get_temp::<Request>(Id::unique(("native-canvas-cursor", root)))).is_some());
    let _ = headless(&ctx, raw(root), |ui| {
        clear(ui.ctx());
        assert!(take(ui.ctx()).is_none());
    });
    let _ = headless(&ctx, raw(root), |ui| {
        request(ui.ctx(), Cursor::Pen, CursorIcon::Crosshair, true, 1.25);
        let req = take(ui.ctx()).unwrap();
        assert_eq!(req.viewport, root);
        assert_eq!(req.pass, ui.ctx().cumulative_pass_nr());
        assert!(take(ui.ctx()).is_none());
        request(ui.ctx(), Cursor::Pen, CursorIcon::Crosshair, false, 1.0);
        assert!(take(ui.ctx()).is_none());
    });
}

fn headless(ctx: &egui::Context, input: egui::RawInput, draw: impl FnMut(&mut egui::Ui)) -> egui::FullOutput {
    let mut output = ctx.run_ui(input, draw);
    output.textures_delta.clear();
    output
}
