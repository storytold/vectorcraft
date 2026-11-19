use super::*;

/// A context with the fonts and the theme in `brightness`, in English.
pub(crate) fn themed(brightness: theme::Brightness) -> egui::Context {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    theme::apply(&ctx, brightness);
    ctx
}

/// One frame at `time` seconds of `add` in a 600 × 400 window with `events` → the shapes it
/// drew, `Shape::Vec`s flattened.
pub(crate) fn frame(ctx: &egui::Context, time: f64, events: Vec<egui::Event>, add: impl FnMut(&mut Ui)) -> Vec<egui::Shape> {
    fn flat(s: egui::Shape, out: &mut Vec<egui::Shape>) {
        match s {
            egui::Shape::Vec(v) => v.into_iter().for_each(|s| flat(s, out)),
            s => out.push(s),
        }
    }
    let raw =
        egui::RawInput { time: Some(time), screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 400.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, add);
    out.textures_delta.clear();
    let mut shapes = vec![];
    out.shapes.into_iter().for_each(|c| flat(c.shape, &mut shapes));
    shapes
}

// Regression test for #808: the scroll bar of the toolbar and the dock's icon column widened to
// 10 points while it was dragged or the pointer was on it, covering the group labels and the
// right edge of the tool icons. It stays a slim line.
#[test]
fn the_strip_scroll_bar_stays_a_slim_line_while_the_pointer_is_on_it() {
    let ctx = themed(Default::default());
    let area = std::cell::Cell::new(Rect::NOTHING);
    let mut add = |ui: &mut Ui| {
        area.set(
            ui.allocate_ui(vec2(80.0, 200.0), |ui| {
                strip_scroll(ui, "strip", |ui| {
                    for i in 0..40 {
                        ui.label(format!("Tool {i}"));
                    }
                });
            })
            .response
            .rect,
        );
    };
    frame(&ctx, 0.0, vec![], &mut add);
    let area = area.get();
    assert!(area.height() >= 199.0, "the strip fills the height it was given: {area:?}");
    // The pointer comes onto the bar's edge and stays there while the bar settles.
    let on_the_bar = egui::Event::PointerMoved(pos2(area.right() - 1.0, area.center().y));
    let mut widest = 0.0_f32;
    for i in 1..=15 {
        let shapes = frame(&ctx, f64::from(i) * 0.1, if i == 1 { vec![on_the_bar.clone()] } else { vec![] }, &mut add);
        for s in shapes {
            // The bar's track and handle are the filled rectangles.
            if let egui::Shape::Rect(r) = s
                && r.fill != Color32::TRANSPARENT
            {
                widest = widest.max(r.rect.width());
            }
        }
    }
    assert!(widest > 0.0, "the scroll bar is drawn");
    assert!(widest <= 4.5, "the scroll bar is {widest} points wide");
}

/// The filled rectangles of a scroll bar drawn at the right of `content` in `shapes`: narrow
/// rectangles that start at or past the content's right edge.
fn bar_rects(shapes: &[egui::Shape], content: Rect, widest: f32) -> Vec<(Rect, Color32)> {
    shapes
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Rect(r)
                if r.fill != Color32::TRANSPARENT && r.rect.width() <= widest && r.rect.height() > 4.0 && r.rect.min.x >= content.right() - 0.5 =>
            {
                Some((r.rect, r.fill))
            }
            _ => None,
        })
        .collect()
}

// Regression test for #5941: a scrolling list in a panel or a dialog showed no scroll bar until the
// pointer came over it, so nothing told the user that the list had more rows. In every brightness
// the bar shows while the pointer is elsewhere, in a column of its own beside the rows, and its
// handle stands out from its track.
#[test]
fn a_scroll_bar_shows_while_the_pointer_is_elsewhere() {
    for brightness in theme::Brightness::ALL {
        let ctx = themed(brightness);
        let rows = std::cell::Cell::new(Rect::NOTHING);
        let mut add = |ui: &mut Ui| {
            ui.allocate_ui(vec2(160.0, 200.0), |ui| {
                egui::ScrollArea::vertical().id_salt("list").auto_shrink([false, false]).show(ui, |ui| {
                    for i in 0..40 {
                        ui.label(format!("Row {i}"));
                    }
                    rows.set(ui.max_rect());
                });
            });
        };
        frame(&ctx, 0.0, vec![egui::Event::PointerMoved(pos2(500.0, 350.0))], &mut add);
        let shapes = frame(&ctx, 1.0, vec![], &mut add);
        let rows = rows.get();
        assert!(rows.width() < 160.0, "{brightness:?}: the rows leave a column for the bar: {rows:?}");
        let bar = bar_rects(&shapes, rows, 10.0);
        let (Some(track), Some(handle)) =
            (bar.iter().max_by(|a, b| a.0.height().total_cmp(&b.0.height())), bar.iter().min_by(|a, b| a.0.height().total_cmp(&b.0.height())))
        else {
            panic!("{brightness:?}: the scroll bar is drawn at rest: {bar:?}");
        };
        assert!(handle.0.height() < track.0.height(), "{brightness:?}: a handle on a track: {bar:?}");
        assert_eq!(handle.1.a(), 255, "{brightness:?}: the handle is opaque: {:?}", handle.1);
        let gap = (i32::from(handle.1.r()) - i32::from(track.1.r())).abs();
        assert!(gap >= 24, "{brightness:?}: handle {:?} on track {:?}", handle.1, track.1);
    }
}

// Regression test for #5941: a drop-down list opens with its current option in view. With its field
// in the middle of a short window, neither side has room for the list's 200 pt, and the menu is cut
// to the room beside the field; the current option still shows whole, at the end of the list and in
// its middle, also when the room beside the field is less than half the list.
#[test]
fn a_dropdown_in_a_short_window_opens_with_its_current_option_in_view() {
    let items: Vec<String> = (0..20).map(|i| format!("Option {i:02}")).collect();
    let labels: Vec<&str> = items.iter().map(String::as_str).collect();
    // (window height, space above the field, current option): about 105 pt and 65 pt on either side
    // of the field.
    for (height, above, current) in [(260.0, 117.0, 19), (260.0, 117.0, 10), (180.0, 77.0, 10), (180.0, 77.0, 3)] {
        let ctx = themed(Default::default());
        let field = std::cell::Cell::new(Rect::NOTHING);
        let popup = std::cell::RefCell::new(vec![]);
        let mut add = |ui: &mut Ui| {
            ui.add_space(above);
            field.set(ui.horizontal(|ui| dropdown_names(ui, "short-list", labels[current], &labels, 160.0)).response.rect);
            popup.replace(super::top_popup_shapes(ui.ctx()));
        };
        // The shapes of the open list, without the field under it.
        let mut run = |time: f64, events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                time: Some(time),
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, height))),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(raw, &mut add);
            out.textures_delta.clear();
            popup.take()
        };
        run(0.0, vec![]);
        let at = field.get().center();
        let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        run(0.1, vec![egui::Event::PointerMoved(at), press(true)]);
        run(0.2, vec![press(false)]);
        let mut clipped = vec![];
        for i in 3..10 {
            clipped = run(f64::from(i) * 0.1, vec![]);
        }
        // Each shape with the clip rect it is painted in.
        let mut shapes = vec![];
        fn flat(s: egui::Shape, clip: Rect, out: &mut Vec<(egui::Shape, Rect)>) {
            match s {
                egui::Shape::Vec(v) => v.into_iter().for_each(|s| flat(s, clip, out)),
                s => out.push((s, clip)),
            }
        }
        clipped.into_iter().for_each(|c| flat(c.shape, c.clip_rect, &mut shapes));
        let list = ctx
            .memory(|m| {
                m.layer_ids().filter(|l| l.order == egui::Order::Foreground && m.areas().is_visible(l)).last().and_then(|l| m.area_rect(l.id))
            })
            .expect("the list is open");
        // The options that show whole: inside the list and inside the clip they are painted in.
        let rows: Vec<(String, Rect)> = shapes
            .iter()
            .filter_map(|(s, clip)| match s {
                egui::Shape::Text(t) if list.contains_rect(t.visual_bounding_rect()) && clip.contains_rect(t.visual_bounding_rect()) => {
                    Some((t.galley.text().to_owned(), t.visual_bounding_rect()))
                }
                _ => None,
            })
            .collect();
        let case = format!("{height} pt window, {} current", labels[current]);
        assert!(rows.len() < labels.len(), "{case}: the list does not show every option in {list:?}: {rows:?}");
        assert!(rows.iter().any(|(s, _)| s == labels[current]), "{case}: the current option shows whole in {list:?}: {rows:?}");
    }
}

// #5941: a drop-down list that draws its own rows (warp styles, tracing presets) has no current
// option for the drop-down to scroll to, so it opens where it was scrolled when it last closed, and
// a row picked below the first ones is still in view when the list opens again. That holds for a
// field in the middle of a short window too, where the list is taller on the pass it opens than
// beside the field, and for a list scrolled to its end.
#[test]
fn a_dropdown_that_draws_its_own_rows_opens_where_it_was_scrolled() {
    // (window height, space above the field, wheel, a row the wheel brings into view)
    for (height, above, wheel, row) in [(300.0, 20.0, -300.0, "Row 15"), (260.0, 117.0, -2000.0, "Row 19")] {
        own_rows_reopen(height, above, wheel, row);
    }
}

fn own_rows_reopen(height: f32, above: f32, wheel: f32, row: &str) {
    let case = format!("{height} pt window, {above} pt above the field");
    let items: Vec<String> = (0..20).map(|i| format!("Row {i:02}")).collect();
    let ctx = themed(Default::default());
    let field = std::cell::Cell::new(Rect::NOTHING);
    let current = std::cell::RefCell::new(String::from("Row 00"));
    let popup = std::cell::RefCell::new(vec![]);
    let mut add = |ui: &mut Ui| {
        ui.add_space(above);
        let shown = current.borrow().clone();
        let r = ui.horizontal(|ui| {
            combo(ui, "own-rows", &shown, 160.0, |ui| {
                let mut chosen = None;
                for (i, s) in items.iter().enumerate() {
                    if ui.selectable_label(*s == shown, s.as_str()).clicked() {
                        chosen = Some(i);
                    }
                }
                chosen
            })
        });
        if let Some(i) = r.inner {
            current.replace(items[i].clone());
        }
        field.set(r.response.rect);
        popup.replace(super::top_popup_shapes(ui.ctx()));
    };
    let mut time = 0.0;
    // The shapes of the open list, without the field under it.
    let mut run = |events: Vec<egui::Event>| {
        time += 0.1;
        let raw = egui::RawInput {
            time: Some(time),
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, height))),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, &mut add);
        out.textures_delta.clear();
        popup.take()
    };
    // The rows that show whole, with their tops: inside the open list and the clip they are painted
    // in.
    let rows = |clipped: Vec<egui::epaint::ClippedShape>| -> Vec<(String, f32)> {
        let list = ctx
            .memory(|m| {
                m.layer_ids().filter(|l| l.order == egui::Order::Foreground && m.areas().is_visible(l)).last().and_then(|l| m.area_rect(l.id))
            })
            .expect("the list is open");
        let mut shapes = vec![];
        fn flat(s: egui::Shape, clip: Rect, out: &mut Vec<(egui::Shape, Rect)>) {
            match s {
                egui::Shape::Vec(v) => v.into_iter().for_each(|s| flat(s, clip, out)),
                s => out.push((s, clip)),
            }
        }
        clipped.into_iter().for_each(|c| flat(c.shape, c.clip_rect, &mut shapes));
        shapes
            .iter()
            .filter_map(|(s, clip)| match s {
                egui::Shape::Text(t) if list.contains_rect(t.visual_bounding_rect()) && clip.contains_rect(t.visual_bounding_rect()) => {
                    Some((t.galley.text().to_owned(), t.visual_bounding_rect().top().round()))
                }
                _ => None,
            })
            .collect()
    };
    let has = |rows: &[(String, f32)], name: &str| rows.iter().any(|(s, _)| s == name);
    run(vec![]);
    let at = field.get().center();
    let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let open = |run: &mut dyn FnMut(Vec<egui::Event>) -> Vec<egui::epaint::ClippedShape>| {
        run(vec![egui::Event::PointerMoved(at), press(true)]);
        run(vec![press(false)]);
        let mut clipped = vec![];
        for _ in 0..4 {
            clipped = run(vec![]);
        }
        clipped
    };
    let first = rows(open(&mut run));
    assert!(has(&first, "Row 00") && !has(&first, row), "{case}: the list opens at its top: {first:?}");
    // Scrolled down to `row` and closed.
    let list = ctx
        .memory(|m| m.layer_ids().filter(|l| l.order == egui::Order::Foreground && m.areas().is_visible(l)).last().and_then(|l| m.area_rect(l.id)))
        .expect("the list is open");
    run(vec![egui::Event::PointerMoved(list.center())]);
    let wheel = egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, wheel),
        modifiers: Default::default(),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
    };
    run(vec![wheel]);
    let mut clipped = vec![];
    for _ in 0..4 {
        clipped = run(vec![]);
    }
    let scrolled = rows(clipped);
    assert!(has(&scrolled, row), "{case}: the wheel scrolls {row} into view: {scrolled:?}");
    // Closed and opened again twice: each time its rows are where they were when it closed. Escape
    // closes it: a list taller than the room beside its field covers the field.
    let escape = |pressed| egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    for time in ["first", "second"] {
        run(vec![escape(true)]);
        run(vec![escape(false)]);
        run(vec![]);
        let reopened = rows(open(&mut run));
        assert_eq!(reopened, scrolled, "{case}: the list opens again where it was scrolled, the {time} time");
    }
    // `row` picked: the field shows it, and the list opens again where it was when it was picked.
    let top = scrolled.iter().find(|(s, _)| s == row).map(|(_, top)| *top).expect("the row shows");
    let on_row = pos2(list.left() + 30.0, top + 6.0);
    let click = |pressed| egui::Event::PointerButton { pos: on_row, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    run(vec![egui::Event::PointerMoved(on_row), click(true)]);
    run(vec![click(false)]);
    run(vec![]);
    assert_eq!(*current.borrow(), row, "{case}: the click picks {row}");
    let reopened = rows(open(&mut run));
    assert_eq!(reopened, scrolled, "{case}: the list opens again where {row} was picked");
}

// Regression test for #5941: the toolbar's and the dock icon column's scroll bar showed nothing
// until the pointer came over the column. Its slim line shows at rest, still at most 4 points
// wide (#808).
#[test]
fn the_strip_scroll_bar_shows_while_the_pointer_is_elsewhere() {
    let ctx = themed(Default::default());
    let column = std::cell::Cell::new(Rect::NOTHING);
    let mut add = |ui: &mut Ui| {
        column.set(
            ui.allocate_ui(vec2(80.0, 200.0), |ui| {
                strip_scroll(ui, "strip", |ui| {
                    for i in 0..40 {
                        ui.label(format!("Tool {i}"));
                    }
                });
            })
            .response
            .rect,
        );
    };
    frame(&ctx, 0.0, vec![egui::Event::PointerMoved(pos2(500.0, 350.0))], &mut add);
    let shapes = frame(&ctx, 1.0, vec![], &mut add);
    let column = column.get();
    // The bar lies along the column's right edge, inside it.
    let buttons = Rect::from_min_max(column.min, pos2(column.right() - 6.0, column.bottom()));
    let bar = bar_rects(&shapes, buttons, 4.5);
    assert!(!bar.is_empty(), "the slim scroll bar is drawn at rest");
}
// Regression test for #900: a number field with a spinner and a preset list drew as three
// boxes side by side: its text box was a point taller than the 26-point row, and each joint had
// two borders.
#[test]
fn a_spinner_field_reads_as_one_box_in_three_parts() {
    let ctx = themed(Default::default());
    let mut add = |ui: &mut Ui| {
        ui.horizontal(|ui| spin_field(ui, "weight", Some(4.0), Unit::Points, 100.0, 1.0, 0.0, &[1.0, 2.0]));
    };
    frame(&ctx, 0.0, vec![], &mut add);
    let shapes = frame(&ctx, 0.1, vec![], &mut add);
    // The boxes of the row: the spinner, the field and the preset chevron, left to right.
    let mut boxes: Vec<Rect> = vec![];
    for s in &shapes {
        if let egui::Shape::Rect(r) = s
            && r.rect.height() > 20.0
            // egui 0.37's TextEdit paints its `Frame::NONE` as a transparent rect too: that is
            // not a box of the row.
            && r.fill != Color32::TRANSPARENT
            && !boxes.contains(&r.rect)
        {
            boxes.push(r.rect);
        }
    }
    boxes.sort_by(|a, b| a.min.x.total_cmp(&b.min.x));
    assert_eq!(boxes.len(), 3, "{boxes:?}");
    for b in &boxes {
        assert_eq!((b.min.y, b.height()), (boxes[0].min.y, 26.0), "all as tall as the row: {boxes:?}");
    }
    for pair in boxes.windows(2) {
        assert!((pair[1].min.x - pair[0].max.x + 1.0).abs() < 0.01, "neighbours share one border: {boxes:?}");
    }
    // The number text in the numeric field is vertically centered in the row box (#900, #4567).
    let text_shape = shapes
        .iter()
        .find_map(|s| match s {
            egui::Shape::Text(t) if t.galley.text() == "4 pt" => Some(t),
            _ => None,
        })
        .expect("field text shape present");
    let text_top = text_shape.pos.y - boxes[1].min.y;
    let text_bottom = boxes[1].max.y - (text_shape.pos.y + text_shape.galley.rect.height());
    assert!((text_top - text_bottom).abs() <= 1.5, "text is vertically centered in its box: top gap {text_top}, bottom gap {text_bottom}");
}

// Regression test for #4567: numbers and text in input fields (numeric fields, dash/gap fields,
// etc.) are vertically centered in their boxes, not shoved against the top edge.
#[test]
fn input_elements_are_vertically_centered_in_their_boxes() {
    let ctx = themed(Default::default());
    let mut add = |ui: &mut Ui| {
        ui.horizontal(|ui| {
            num_field(ui, "plain_num", Some(12.0), Unit::Points, 70.0);
            opt_field(ui, "dash_cell", Some(12.0), Unit::Points, 40.0);
        });
    };
    frame(&ctx, 0.0, vec![], &mut add);
    let shapes = frame(&ctx, 0.1, vec![], &mut add);
    let mut boxes: Vec<Rect> = vec![];
    for s in &shapes {
        if let egui::Shape::Rect(r) = s
            && r.rect.height() > 20.0
            && r.fill != Color32::TRANSPARENT
            && !boxes.contains(&r.rect)
        {
            boxes.push(r.rect);
        }
    }
    boxes.sort_by(|a, b| a.min.x.total_cmp(&b.min.x));
    assert_eq!(boxes.len(), 2, "found both field boxes: {boxes:?}");

    let mut text_shapes: Vec<&egui::epaint::TextShape> = vec![];
    for s in &shapes {
        if let egui::Shape::Text(t) = s
            && (t.galley.text() == "12 pt" || t.galley.text() == "12")
        {
            text_shapes.push(t);
        }
    }
    text_shapes.sort_by(|a, b| a.pos.x.total_cmp(&b.pos.x));
    assert_eq!(text_shapes.len(), 2, "found both text shapes: {text_shapes:?}");

    for (b, t) in boxes.iter().zip(text_shapes.iter()) {
        let text_top = t.pos.y - b.min.y;
        let text_bottom = b.max.y - (t.pos.y + t.galley.rect.height());
        assert!(
            (text_top - text_bottom).abs() <= 1.5,
            "text '{}' is vertically centered: top gap {text_top}, bottom gap {text_bottom} in box {b:?}",
            t.galley.text()
        );
    }
}

/// The colour of every glyph of the text in `shapes`: selected glyphs are drawn in the
/// selection stroke's colour.
pub(crate) fn glyph_colours(shapes: &[egui::Shape]) -> Vec<Color32> {
    let mut colours = vec![];
    for s in shapes {
        if let egui::Shape::Text(t) = s {
            for row in &t.galley.rows {
                let visuals = &row.row.visuals;
                colours.extend(visuals.mesh.vertices[visuals.glyph_vertex_range.clone()].iter().map(|v| v.color));
            }
        }
    }
    colours
}

// Regression test for #1465: the selected number in a value field was hard to read in every
// brightness: egui draws selected text in the selection stroke's colour, the accent, on a
// selection tinted with the same accent. It is in the strong text colour.
#[test]
fn selected_text_in_a_value_field_is_in_the_strong_text_colour() {
    for brightness in theme::Brightness::ALL {
        let ctx = themed(brightness);
        let t = Tokens::get(&ctx);
        let id = std::cell::Cell::new(egui::Id::NULL);
        let mut add = |ui: &mut Ui| {
            id.set(ui.scope_id().with("w"));
            num_field(ui, "w", Some(1234.0), Unit::Points, 80.0);
        };
        frame(&ctx, 0.0, vec![], &mut add);
        // The field takes focus, and all of its text is selected.
        ctx.memory_mut(|m| m.request_focus(id.get()));
        frame(&ctx, 0.1, vec![], &mut add);
        assert!(ctx.memory(|m| m.has_focus(id.get())), "{brightness:?}: the field has focus");
        let len = Unit::Points.format(1234.0).chars().count();
        let mut state = egui::TextEdit::load_state(&ctx, id.get()).unwrap_or_default();
        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(len))));
        state.store(&ctx, id.get());
        let shapes = frame(&ctx, 0.2, vec![], &mut add);
        let colours = glyph_colours(&shapes);
        assert!(!colours.is_empty(), "{brightness:?}: the number is drawn");
        let mut used = colours.clone();
        used.dedup();
        assert_eq!(used, [t.text_strong], "{brightness:?}: selected text in {used:?}");
        assert_ne!(t.text_strong, t.accent);
    }
}

// Regression test for #1483: the theme's widget fill is the panel's colour, so a slider in a
// dialog showed only its handle. `slider` draws the rail in the buttons' border colour and
// fills it up to the handle in the accent.
#[test]
fn a_slider_shows_its_rail_filled_up_to_the_handle() {
    for brightness in theme::Brightness::ALL {
        let ctx = themed(brightness);
        let t = Tokens::get(&ctx);
        let mut value = 30.0_f64;
        let mut add = |ui: &mut Ui| {
            slider(ui, egui::Slider::new(&mut value, 0.0..=100.0).show_value(false));
        };
        frame(&ctx, 0.0, vec![], &mut add);
        let shapes = frame(&ctx, 0.1, vec![], &mut add);
        let rects: Vec<(Rect, Color32)> =
            shapes.iter().filter_map(|s| if let egui::Shape::Rect(r) = s { Some((r.rect, r.fill)) } else { None }).collect();
        let rail = rects.iter().find(|(_, c)| *c == t.button_border).map(|(r, _)| *r);
        let filled = rects.iter().find(|(_, c)| *c == t.accent).map(|(r, _)| *r);
        let (Some(rail), Some(filled)) = (rail, filled) else {
            panic!("{brightness:?}: the rail ({:?}) and its fill ({:?}) are drawn: {rects:?}", t.button_border, t.accent);
        };
        assert!(filled.width() > 1.0 && filled.width() < rail.width(), "{brightness:?}: filled {filled:?} of the rail {rail:?}");
        assert!((filled.min.x - rail.min.x).abs() < 1.0, "{brightness:?}: the fill starts at the rail's start");
    }
}

/// A dialog slider that a click focused shows no focus outline; once an arrow key steps it, the
/// outline shows, so a keyboard user sees where the focus is.
#[test]
fn a_slider_focused_by_a_click_shows_its_outline_only_after_a_key() {
    let ctx = themed(Default::default());
    let t = Tokens::get(&ctx);
    let rail = std::cell::Cell::new(Rect::NOTHING);
    let mut add = |ui: &mut Ui| {
        let shown = SliderValue::number(5.0, 0.0..=10.0);
        let (_, _, _) = color_slider_stepped(ui, "outline-test", "Test", Some(shown), 0.5, 180.0, &|_| Color32::GRAY);
        rail.set(ui.min_rect());
    };
    // An outline: a stroked rect in the accent around the rail.
    let outlined = |shapes: &[egui::Shape]| {
        shapes
            .iter()
            .any(|s| matches!(s, egui::Shape::Rect(r) if r.stroke.color == t.accent && r.stroke.width > 0.0 && r.fill == Color32::TRANSPARENT))
    };
    frame(&ctx, 0.0, vec![], &mut add);
    let at = rail.get().center();
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    frame(&ctx, 0.1, vec![egui::Event::PointerMoved(at), button(true)], &mut add);
    frame(&ctx, 0.2, vec![button(false)], &mut add);
    let shapes = frame(&ctx, 0.3, vec![], &mut add);
    assert!(ctx.memory(|m| m.focused().is_some()), "the click focused the rail");
    assert!(!outlined(&shapes), "no outline after a click");
    let arrow =
        |pressed| egui::Event::Key { key: egui::Key::ArrowRight, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE };
    frame(&ctx, 0.4, vec![arrow(true), arrow(false)], &mut add);
    let shapes = frame(&ctx, 0.5, vec![], &mut add);
    assert!(outlined(&shapes), "an arrow key shows the outline");
}

/// Regression test for #2083: the rows of the panels' menus didn't light up under the pointer, as
/// the menu bar's items do.
#[test]
fn a_panel_menu_row_lights_up_under_the_pointer() {
    let ctx = themed(Default::default());
    let row_at = std::cell::Cell::new(Rect::NOTHING);
    let mut add = |ui: &mut Ui| {
        row_at.set(
            ui.scope(|ui| {
                menu_item_name(ui, "Rename", true, false);
            })
            .response
            .rect,
        );
    };
    // A filled rectangle behind the row's text.
    let lit = |shapes: &[egui::Shape], row: Rect| {
        shapes.iter().any(|s| matches!(s, egui::Shape::Rect(r) if r.fill != Color32::TRANSPARENT && r.rect.contains(row.center())))
    };
    frame(&ctx, 0.0, vec![], &mut add);
    let row = row_at.get();
    let at_rest = frame(&ctx, 0.1, vec![], &mut add);
    assert!(!lit(&at_rest, row), "no frame at rest");
    frame(&ctx, 0.2, vec![egui::Event::PointerMoved(row.center())], &mut add);
    let hovered = frame(&ctx, 0.3, vec![], &mut add);
    assert!(lit(&hovered, row), "the row under the pointer is lit: {row:?}");
}

/// Each painted row label names its own numeric field rather than the field's internal salt.
#[test]
fn a_label_rows_input_is_named_by_the_row_label() {
    use egui_kittest::kittest::Queryable;
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut h = egui_kittest::Harness::builder().with_size(vec2(400.0, 200.0)).build_ui(|ui| {
        label_row(ui, "Opacity", 70.0, |ui| {
            let _ = spin_field(ui, "opacity", Some(50.0), Unit::Points, 80.0, 1.0, 0.0, &[]);
        });
        label_row(ui, "Angle", 70.0, |ui| {
            let _ = range_field(ui, "plugin-angle", 30.0, 0.0..=360.0, "°", 1, 80.0);
        });
    });
    h.run_steps(2);
    // The painted row label is the value field's exact accessible name: not its internal id.
    let _ = h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Opacity");
    let _ = h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Angle");
    assert!(h.query_by_role_and_label(egui::accesskit::Role::TextInput, "opacity").is_none(), "no internal widget id");
    assert!(h.query_by_role_and_label(egui::accesskit::Role::TextInput, "plugin-angle").is_none(), "no internal widget id");
}

/// A label followed by a checkbox or other widget does not name a later field.
#[test]
fn a_label_does_not_name_a_field_after_another_widget() {
    use egui_kittest::kittest::Queryable;
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut h = egui_kittest::Harness::builder().with_size(vec2(400.0, 200.0)).with_accessibility_check(false).build_ui(|ui| {
        field_label(ui, "Opacity");
        let _ = check(ui, "Preview", true, true);
        let _ = spin_field(ui, "unrelated", Some(50.0), Unit::Points, 80.0, 1.0, 0.0, &[]);
    });
    h.run_steps(2);
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::TextInput, "Opacity").is_none(),
        "the label belongs to the next widget, not a later field"
    );
    assert!(h.query_by_role_and_label(egui::accesskit::Role::TextInput, "unrelated").is_none(), "the field is not named by its id");
}

/// A grid's label is in the cell before the field: egui moves the cursor to the next cell
/// without allocating a widget, so the label still names that field exactly.
#[test]
fn a_grid_fields_adjacent_label_names_it_exactly() {
    use egui_kittest::kittest::Queryable;
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut h = egui_kittest::Harness::builder().with_size(vec2(400.0, 200.0)).build_ui(|ui| {
        egui::Grid::new("field-name-grid").num_columns(2).show(ui, |ui| {
            dim_label(ui, "X:");
            let _ = num_field(ui, "xfp-x", Some(12.0), Unit::Points, 80.0);
            ui.end_row();
        });
    });
    h.run_steps(2);
    let _ = h.get_by_role_and_label(egui::accesskit::Role::TextInput, "X:");
    assert!(h.query_by_role_and_label(egui::accesskit::Role::TextInput, "xfp-x").is_none(), "no internal widget id");
}

/// A row whose visible names are painted below its fields (Stroke's dash/gap cells) passes
/// the exact translated name, never a Debug-formatted widget id.
#[test]
fn an_explicit_field_name_is_exact_and_has_no_widget_id() {
    use egui_kittest::kittest::Queryable;
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut h = egui_kittest::Harness::builder().with_size(vec2(400.0, 200.0)).build_ui(|ui| {
        let _ = opt_field_named(ui, ("dash", 0), Some("dash 1"), Some(4.0), Unit::Points, 60.0);
        let _ = num_field_named(ui, "xfp-x", Some("X:"), Some(12.0), Unit::Points, 80.0);
    });
    h.run_steps(2);
    let _ = h.get_by_role_and_label(egui::accesskit::Role::TextInput, "dash 1");
    let _ = h.get_by_role_and_label(egui::accesskit::Role::TextInput, "X:");
    for leaked in ["dash 0", "xfp-x", "(\"dash\", 0)"] {
        assert!(
            h.query_by_role_and_label(egui::accesskit::Role::TextInput, leaked).is_none(),
            "the internal widget id {leaked:?} is not an accessible name"
        );
    }
}

/// The Enter and focus guard's reconstructed id equals the id egui gives the ComboBox button.
#[test]
fn a_combos_keyboard_guard_reads_the_combos_own_button_id() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let same = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = same.clone();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(400.0, 200.0)).with_accessibility_check(false).build_ui(move |ui| {
        // The id the Enter/focus guard reconstructs before drawing, and the id egui's
        // `ComboBox::from_id_salt` gives the button: they must be equal, or the guard
        // reads and gives back the keyboard of nothing.
        let expected = combo_button_id(ui, &"combo-guard-test");
        let response = craft_ui::ComboBox::from_id_salt("combo-guard-test").selected_text("One").show_ui(ui, |_| ());
        seen.set(expected == response.response.id);
    });
    h.run_steps(2);
    assert!(same.get(), "the guard's id is the ComboBox button's id");
}

/// Regression test for #3440: the Fill/Stroke chip's popover closed on the click that ended a
/// drag between its proxies. It stays open for a click on its own anchor and while a panel
/// drag is on; a click elsewhere still closes it.
#[test]
fn a_popover_stays_open_for_a_click_on_its_anchor_or_during_a_panel_drag() {
    let press = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    // Opens the popover, then clicks at `click` (with `drag` as the panel drag under way).
    let open_after_click = |click: Option<Pos2>, drag: Option<PanelDrag>| -> bool {
        let ctx = themed(Default::default());
        let toggle = std::cell::Cell::new(true);
        let shown = std::cell::Cell::new(false);
        let anchor = std::cell::Cell::new(Rect::NOTHING);
        let mut add = |ui: &mut Ui| {
            let resp = ui.allocate_response(vec2(40.0, 24.0), Sense::hover());
            anchor.set(resp.rect);
            shown.set(popover(&resp, toggle.replace(false), |ui| ui.label("Colour")).is_some());
        };
        frame(&ctx, 0.0, vec![], &mut add);
        assert!(shown.get(), "the toggle opens it");
        frame(&ctx, 0.2, vec![], &mut add);
        assert!(shown.get(), "it stays open");
        if let Some(d) = drag {
            egui::DragAndDrop::set_payload(&ctx, d);
        }
        let pos = click.unwrap_or_else(|| anchor.get().center());
        frame(&ctx, 0.3, vec![egui::Event::PointerMoved(pos), press(pos, true)], &mut add);
        frame(&ctx, 0.4, vec![press(pos, false)], &mut add);
        frame(&ctx, 0.5, vec![], &mut add);
        shown.get()
    };
    let far = Some(pos2(500.0, 350.0));
    let panel_drag = || Some(PanelDrag::Symbol { document: 1, name: "s".into() });
    // Whether it is open after: a click elsewhere; a click on its anchor that doesn't toggle it;
    // a panel drag released elsewhere.
    let open = [open_after_click(far, None), open_after_click(None, None), open_after_click(far, panel_drag())];
    assert_eq!(open, [false, true, true]);
}

/// #4267: a click on a spinner arrow steps once; an arrow held down steps again and again after a
/// pause even past the click timeout (#6307), and letting go doesn't step once more.
#[test]
fn a_held_spinner_arrow_repeats() {
    let ctx = themed(Default::default());
    let value = std::cell::Cell::new(5.0);
    let scroll = std::cell::Cell::new(0.0);
    let spinner_rect = std::cell::Cell::new(Rect::NOTHING);
    let mut add = |ui: &mut Ui| {
        let area = egui::ScrollArea::vertical().max_height(100.0).scroll_source(egui::scroll_area::ScrollSource::DRAG).show(ui, |ui| {
            ui.add_space(35.0);
            let row = ui.horizontal(|ui| {
                if let Some(v) = spin_plain(ui, "repeat", value.get(), "", 0, 100.0, 1.0, 0.0, &[]) {
                    value.set(v);
                }
            });
            spinner_rect.set(Rect::from_min_size(row.response.rect.min, vec2(16.0, 26.0)));
            ui.add_space(150.0);
        });
        scroll.set(area.state.clamped_offset().y);
    };
    let shapes = frame(&ctx, 0.0, vec![], &mut add);
    let spinner = shapes
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Rect(r) if r.rect.height() > 20.0 && r.fill != Color32::TRANSPARENT => Some(r.rect),
            _ => None,
        })
        .min_by(|a, b| a.min.x.total_cmp(&b.min.x))
        .expect("the spinner box");
    let up = spinner.center_top() + vec2(0.0, 6.0);
    let button = |pressed| egui::Event::PointerButton { pos: up, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    // A click: one step, on release.
    frame(&ctx, 0.1, vec![egui::Event::PointerMoved(up), button(true)], &mut add);
    frame(&ctx, 0.2, vec![button(false)], &mut add);
    assert_eq!(value.get(), 6.0, "a click steps once");
    // Held: nothing more until the pause is over, then a step every repeat.
    frame(&ctx, 1.0, vec![button(true)], &mut add);
    frame(&ctx, 1.3, vec![], &mut add);
    assert_eq!(value.get(), 6.0, "no step before the pause");
    for i in 0..17 {
        let t = 1.5 + f64::from(i) * 0.1;
        frame(&ctx, t, vec![], &mut add);
        assert_eq!(value.get(), 7.0 + f64::from(i), "a step every repeat at {t}");
    }
    frame(&ctx, 3.2, vec![button(false)], &mut add);
    frame(&ctx, 3.3, vec![], &mut add);
    assert_eq!(value.get(), 23.0, "letting go doesn't step once more");

    let down = spinner_rect.get().center_bottom() - vec2(0.0, 6.0);
    let modifiers = egui::Modifiers::SHIFT;
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
    frame(&ctx, 3.9, vec![egui::Event::PointerMoved(down)], &mut add);
    frame(&ctx, 4.0, vec![egui::Event::ModifiersChanged(modifiers), button(down, true)], &mut add);
    frame(&ctx, 4.5, vec![], &mut add);
    assert_eq!(value.get(), 13.0, "Shift+down steps by ten: press {down:?}, current {:?}", spinner_rect.get());
    frame(&ctx, 4.9, vec![], &mut add);
    assert_eq!(value.get(), 3.0, "Shift+down repeats past the click timeout");

    let outside = down - vec2(0.0, 45.0);
    frame(&ctx, 5.0, vec![egui::Event::PointerMoved(outside)], &mut add);
    frame(&ctx, 5.1, vec![], &mut add);
    frame(&ctx, 5.2, vec![button(outside, false)], &mut add);
    assert_eq!(value.get(), 3.0, "dragging away and releasing doesn't step");
    assert!(scroll.get() > 0.0, "a drag starting on the spinner can still scroll the panel");
}

/// The Kerning field reads `(10+5)` as 15 whether it shows a manual value (`12`) or an automatic
/// one (`(12)`), and refuses `(10+5)*2` either way.
#[test]
fn kerning_reads_parentheses_around_its_text_with_a_manual_value_too() {
    let typed = |automatic: bool, text: &str| {
        let ctx = themed(Default::default());
        let picked = std::cell::Cell::new(None);
        let mut add = |ui: &mut Ui| {
            ui.horizontal(|ui| {
                if let Some(p) = spin_kerning(ui, "kern", Some(12.0), Some(automatic), Some(false), "Optical", automatic, 100.0, &[]) {
                    picked.set(Some(p));
                }
            });
        };
        arm_field_focus(&ctx, "kern");
        frame(&ctx, 0.0, vec![], &mut add);
        frame(&ctx, 0.1, vec![], &mut add);
        let enter = |pressed| egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
        frame(&ctx, 0.2, vec![egui::Event::Text(text.into()), enter(true), enter(false)], &mut add);
        frame(&ctx, 0.3, vec![], &mut add);
        picked.get()
    };
    for automatic in [false, true] {
        assert_eq!(typed(automatic, "(10+5)"), Some(SpinPick::Value(15.0)), "automatic: {automatic}");
        assert_eq!(typed(automatic, "(10+5)*2"), None, "automatic: {automatic}");
        assert_eq!(typed(automatic, "(1,000)"), None, "a thousands comma, automatic: {automatic}");
        assert_eq!(typed(automatic, "(1,5)"), Some(SpinPick::Value(1.5)), "a decimal comma, automatic: {automatic}");
    }
}

/// Long dropdown lists use the window's room instead of stopping at `combo_height` (200 pt), and
/// show a solid scroll bar without hovering when the list still overflows.
#[test]
fn a_long_dropdown_uses_window_room_and_shows_its_scroll_bar_when_clipped() {
    let items: Vec<String> = (0..20).map(|i| format!("Format {i:02}")).collect();
    let labels: Vec<&str> = items.iter().map(String::as_str).collect();
    let run = |win_h: f32| {
        let ctx = themed(Default::default());
        let button_rect = std::cell::Cell::new(Rect::NOTHING);
        let mut draw = |ui: &mut Ui| {
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                let before = ui.next_widget_position();
                let _ = dropdown_names(ui, "long-formats", labels[0], &labels, 160.0);
                button_rect.set(Rect::from_min_size(before, vec2(160.0, 26.0)));
            });
        };
        let mut step = |time: f64, events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                time: Some(time),
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, win_h))),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(raw, &mut draw);
            out.textures_delta.clear();
            out.shapes
        };
        step(0.0, vec![]);
        let at = button_rect.get().center();
        let click = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        step(0.1, vec![egui::Event::PointerMoved(at), click(true)]);
        step(0.2, vec![click(false)]);
        // Move the pointer away so the scroll bar is dormant, not hovered, and let the popup settle.
        step(0.3, vec![egui::Event::PointerMoved(pos2(350.0, 10.0))]);
        let mut clipped = vec![];
        for i in 4..10 {
            clipped = step(f64::from(i) * 0.1, vec![]);
        }
        (ctx, clipped)
    };

    // In a tall window, all 20 items show without stopping at the 200 pt cap.
    let (_tall_ctx, tall_shapes) = run(720.0);
    let mut texts = vec![];
    fn collect_text(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Vec(v) => v.iter().for_each(|s| collect_text(s, out)),
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            _ => {}
        }
    }
    for c in &tall_shapes {
        collect_text(&c.shape, &mut texts);
    }
    assert!(texts.iter().any(|t| t == "Format 19"), "the 20th option is shown without scrolling: {texts:?}");

    // In a short window where the list overflows, a solid scroll bar is drawn even while dormant.
    let (short_ctx, short_shapes) = run(220.0);
    let mut short_texts = vec![];
    for c in &short_shapes {
        collect_text(&c.shape, &mut short_texts);
    }
    assert!(!short_texts.iter().any(|t| t == "Format 19"), "the list overflows the 220 pt window");
    let popup = short_ctx
        .memory(|m| m.layer_ids().filter(|l| l.order == egui::Order::Foreground && m.areas().is_visible(l)).last().and_then(|l| m.area_rect(l.id)))
        .expect("the popup is open");
    let mut bar_rects = vec![];
    fn collect_rects(s: &egui::Shape, out: &mut Vec<egui::epaint::RectShape>) {
        match s {
            egui::Shape::Vec(v) => v.iter().for_each(|s| collect_rects(s, out)),
            egui::Shape::Rect(r) => out.push(r.clone()),
            _ => {}
        }
    }
    for c in &short_shapes {
        collect_rects(&c.shape, &mut bar_rects);
    }
    let has_scroll_bar = bar_rects.iter().any(|r| {
        r.fill != Color32::TRANSPARENT
            && r.rect.width() <= list_scroll_style().bar_width + 1.0
            && r.rect.height() >= 12.0
            && r.rect.height() < popup.height()
            && r.rect.right() <= popup.right()
            && r.rect.left() >= popup.right() - list_scroll_style().allocated_width() - 8.0
    });
    assert!(has_scroll_bar, "the overflowing dropdown draws its scroll bar without hover in {popup:?}");
}

// #5941: a drop-down list stopped at the room on one side of its field, so a window with room for
// the whole list still cut it short. The list opens below its field when it fits there, above it
// when it fits there instead, and otherwise moves up from below into the window over its field;
// each time every option shows whole, inside the window.
#[test]
fn a_dropdown_list_shows_every_option_when_the_window_has_room() {
    let items: Vec<String> = (0..20).map(|i| format!("Option {i:02}")).collect();
    let labels: Vec<&str> = items.iter().map(String::as_str).collect();
    // (window height, space above the field, where the list goes)
    for (height, above, side) in [(700.0, 16.0, "below"), (700.0, 650.0, "above"), (640.0, 300.0, "over")] {
        let case = format!("{height} pt window, field {above} pt down");
        let ctx = themed(Default::default());
        let field = std::cell::Cell::new(Rect::NOTHING);
        let popup = std::cell::RefCell::new(vec![]);
        let mut add = |ui: &mut Ui| {
            ui.add_space(above);
            field.set(ui.horizontal(|ui| dropdown_names(ui, "fit-list", labels[0], &labels, 160.0)).response.rect);
            popup.replace(super::top_popup_shapes(ui.ctx()));
        };
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(400.0, height));
        // The shapes of the open list, without the field under it.
        let mut run = |time: f64, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(egui::RawInput { time: Some(time), screen_rect: Some(screen), events, ..Default::default() }, &mut add);
            out.textures_delta.clear();
            popup.take()
        };
        run(0.0, vec![]);
        let at = field.get().center();
        let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        run(0.1, vec![egui::Event::PointerMoved(at), press(true)]);
        run(0.2, vec![press(false)]);
        run(0.3, vec![egui::Event::PointerMoved(pos2(390.0, 2.0))]);
        let mut clipped = vec![];
        for i in 4..10 {
            clipped = run(f64::from(i) * 0.1, vec![]);
        }
        let list = ctx
            .memory(|m| {
                m.layer_ids().filter(|l| l.order == egui::Order::Foreground && m.areas().is_visible(l)).last().and_then(|l| m.area_rect(l.id))
            })
            .expect("the list is open");
        assert!(screen.contains_rect(list), "{case}: the list {list:?} is inside the window");
        let field = field.get();
        // The list opens from the button, 1 pt inside the field's border.
        match side {
            "below" => assert!(list.top() >= field.bottom() - 1.5, "{case}: the list {list:?} opens below the field {field:?}"),
            "above" => assert!(list.bottom() <= field.top() + 1.5, "{case}: the list {list:?} opens above the field {field:?}"),
            _ => assert!(list.top() < field.bottom() && list.bottom() > field.top(), "{case}: the list {list:?} covers the field {field:?}"),
        }
        let mut texts = vec![];
        fn flat(s: &egui::Shape, clip: Rect, out: &mut Vec<(String, Rect, Rect)>) {
            match s {
                egui::Shape::Vec(v) => v.iter().for_each(|s| flat(s, clip, out)),
                egui::Shape::Text(t) => out.push((t.galley.text().to_owned(), t.visual_bounding_rect(), clip)),
                _ => {}
            }
        }
        clipped.iter().for_each(|c| flat(&c.shape, c.clip_rect, &mut texts));
        let whole: Vec<&str> = labels
            .iter()
            .copied()
            .filter(|name| {
                texts.iter().any(|(s, r, clip)| s == name && list.contains_rect(*r) && r.top() >= clip.top() && r.bottom() <= clip.bottom())
            })
            .collect();
        assert_eq!(whole, labels, "{case}: every option shows whole in {list:?}");
    }
}

// #5941: a drop-down list stops above the bottom bars, as other menus do (#2086). In a 700 pt window
// whose bars start at 640 pt, the list opens below a field near the top, and from a field in the
// middle it moves up over the field until it ends at the bars; each time every option shows whole.
#[test]
fn a_dropdown_list_stops_above_the_bottom_bars() {
    let items: Vec<String> = (0..20).map(|i| format!("Option {i:02}")).collect();
    let labels: Vec<&str> = items.iter().map(String::as_str).collect();
    let floor = 640.0;
    for above in [16.0, 300.0] {
        let case = format!("field {above} pt down");
        let ctx = themed(Default::default());
        let field = std::cell::Cell::new(Rect::NOTHING);
        let popup = std::cell::RefCell::new(vec![]);
        let mut add = |ui: &mut Ui| {
            set_menu_floor(ui.ctx(), floor);
            ui.add_space(above);
            field.set(ui.horizontal(|ui| dropdown_names(ui, "floor-list", labels[0], &labels, 160.0)).response.rect);
            popup.replace(super::top_popup_shapes(ui.ctx()));
        };
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 700.0));
        let mut run = |time: f64, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(egui::RawInput { time: Some(time), screen_rect: Some(screen), events, ..Default::default() }, &mut add);
            out.textures_delta.clear();
            popup.take()
        };
        run(0.0, vec![]);
        let at = field.get().center();
        let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        run(0.1, vec![egui::Event::PointerMoved(at), press(true)]);
        run(0.2, vec![press(false)]);
        run(0.3, vec![egui::Event::PointerMoved(pos2(390.0, 2.0))]);
        let mut clipped = vec![];
        for i in 4..10 {
            clipped = run(f64::from(i) * 0.1, vec![]);
        }
        let list = ctx
            .memory(|m| {
                m.layer_ids().filter(|l| l.order == egui::Order::Foreground && m.areas().is_visible(l)).last().and_then(|l| m.area_rect(l.id))
            })
            .expect("the list is open");
        assert!(list.top() >= 0.0 && list.bottom() <= floor + 0.5, "{case}: the list {list:?} ends above the bars at {floor}");
        let mut texts = vec![];
        fn flat(s: &egui::Shape, clip: Rect, out: &mut Vec<(String, Rect, Rect)>) {
            match s {
                egui::Shape::Vec(v) => v.iter().for_each(|s| flat(s, clip, out)),
                egui::Shape::Text(t) => out.push((t.galley.text().to_owned(), t.visual_bounding_rect(), clip)),
                _ => {}
            }
        }
        clipped.iter().for_each(|c| flat(&c.shape, c.clip_rect, &mut texts));
        let whole: Vec<&str> = labels
            .iter()
            .copied()
            .filter(|name| {
                texts.iter().any(|(s, r, clip)| s == name && list.contains_rect(*r) && r.top() >= clip.top() && r.bottom() <= clip.bottom())
            })
            .collect();
        assert_eq!(whole, labels, "{case}: every option shows whole in {list:?}");
    }
}

// #5941: a click on the scroll bar's track of an open drop-down list scrolls it and leaves it open,
// so an option it scrolled into view can be picked. A list closed by that click would open again
// with its current option in view.
#[test]
fn a_click_on_a_dropdown_lists_scroll_bar_scrolls_it_and_leaves_it_open() {
    let items: Vec<String> = (0..40).map(|i| format!("Option {i:02}")).collect();
    let labels: Vec<&str> = items.iter().map(String::as_str).collect();
    let ctx = themed(Default::default());
    let field = std::cell::Cell::new(Rect::NOTHING);
    let picked = std::cell::Cell::new(None);
    let popup = std::cell::RefCell::new(vec![]);
    let mut add = |ui: &mut Ui| {
        ui.add_space(16.0);
        let r = ui.horizontal(|ui| dropdown_names(ui, "bar-click", labels[0], &labels, 160.0));
        field.set(r.response.rect);
        if let Some(i) = r.inner {
            picked.set(Some(i));
        }
        popup.replace(super::top_popup_shapes(ui.ctx()));
    };
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
    let mut time = 0.0;
    // The options that show whole on the open list, with their rects, and the scroll bar's rects.
    let mut run = |events: Vec<egui::Event>| {
        time += 0.1;
        let mut out = ctx.run_ui(egui::RawInput { time: Some(time), screen_rect: Some(screen), events, ..Default::default() }, &mut add);
        out.textures_delta.clear();
        let (mut texts, mut bars) = (vec![], vec![]);
        fn flat(s: &egui::Shape, clip: Rect, out: &mut Vec<(String, Rect)>, bars: &mut Vec<Rect>) {
            match s {
                egui::Shape::Vec(v) => v.iter().for_each(|s| flat(s, clip, out, bars)),
                egui::Shape::Text(t) => {
                    let r = t.visual_bounding_rect();
                    if r.top() >= clip.top() && r.bottom() <= clip.bottom() {
                        out.push((t.galley.text().to_owned(), r));
                    }
                }
                egui::Shape::Rect(r) if r.fill.a() > 0 && r.rect.width() <= 8.0 && r.rect.height() > 40.0 => bars.push(r.rect),
                _ => {}
            }
        }
        popup.take().iter().for_each(|c| flat(&c.shape, c.clip_rect, &mut texts, &mut bars));
        (texts, bars)
    };
    let list_rect = || {
        ctx.memory(|m| m.layer_ids().filter(|l| l.order == egui::Order::Foreground && m.areas().is_visible(l)).last().and_then(|l| m.area_rect(l.id)))
    };
    let click =
        |at: Pos2, pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    run(vec![]);
    let at = field.get().center();
    run(vec![egui::Event::PointerMoved(at), click(at, true)]);
    run(vec![click(at, false)]);
    let (mut before, mut bars) = (vec![], vec![]);
    for _ in 0..4 {
        (before, bars) = run(vec![]);
    }
    assert!(list_rect().is_some(), "the list is open");
    assert!(before.iter().any(|(s, _)| s == "Option 00"), "the list opens at its first option: {before:?}");
    // The track, below the handle: near the bottom of the tallest bar rect drawn (the track).
    let bar = bars
        .iter()
        .copied()
        .max_by(|a, b| a.height().total_cmp(&b.height()))
        .unwrap_or_else(|| panic!("the list {:?} draws its scroll bar: {before:?}", list_rect()));
    let track = pos2(bar.center().x, bar.bottom() - 6.0);
    run(vec![egui::Event::PointerMoved(track), click(track, true)]);
    run(vec![]);
    // The button is let go just past the bar's edge, closer than a drag, and the pointer then moves
    // sideways onto the rows, in the same pass.
    let let_go = pos2(bar.right() + 1.0, track.y);
    let onto_row = before.first().map(|(_, r)| pos2(r.center().x, track.y)).expect("a row shows");
    run(vec![egui::Event::PointerMoved(let_go), click(let_go, false), egui::Event::PointerMoved(onto_row)]);
    let mut after = vec![];
    for _ in 0..4 {
        (after, _) = run(vec![]);
    }
    assert!(list_rect().is_some(), "a click on the scroll bar leaves the list open");
    let new = after
        .iter()
        .find(|(s, _)| !before.iter().any(|(b, _)| b == s))
        .cloned()
        .unwrap_or_else(|| panic!("the click scrolled options into view: {after:?}"));
    let row = new.1.center();
    run(vec![egui::Event::PointerMoved(row), click(row, true)]);
    run(vec![click(row, false)]);
    run(vec![]);
    let index = labels.iter().position(|l| *l == new.0);
    assert_eq!(picked.get(), index, "the option scrolled into view is picked: {}", new.0);
    assert!(list_rect().is_none(), "picking an option closes the list");
}

// #5941: a drop-down list with an option wider than the window ran past the window's right edge,
// and its scroll bar with it. The list stays inside the window and its bar shows; the long option's
// name is cut short.
#[test]
fn a_dropdown_list_wider_than_the_window_keeps_its_scroll_bar_in_view() {
    let mut items: Vec<String> = (0..40).map(|i| format!("Option {i:02}")).collect();
    items[1] = "Option 01, whose name is much wider than this window".into();
    let labels: Vec<&str> = items.iter().map(String::as_str).collect();
    let short: Vec<&str> = labels.iter().copied().filter(|l| l.len() < 10).collect();
    let two: Vec<&str> = labels[..2].to_vec();
    // Which list the field offers: 0 all, 1 the short names, 2 the first two.
    let offered = std::cell::Cell::new(0);
    let ctx = themed(Default::default());
    let field = std::cell::Cell::new(Rect::NOTHING);
    let popup = std::cell::RefCell::new(vec![]);
    let mut add = |ui: &mut Ui| {
        ui.add_space(16.0);
        let options = [&labels, &short, &two][offered.get()];
        field.set(ui.horizontal(|ui| dropdown_names(ui, "wide-list", options[0], options, 160.0)).response.rect);
        popup.replace(super::top_popup_shapes(ui.ctx()));
    };
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(220.0, 300.0));
    let window = std::cell::Cell::new(screen);
    let mut time = 0.0;
    // The texts painted on the open list with their clip rects, and the scroll bar's rects.
    let mut run = |events: Vec<egui::Event>| {
        time += 0.1;
        let mut out = ctx.run_ui(egui::RawInput { time: Some(time), screen_rect: Some(window.get()), events, ..Default::default() }, &mut add);
        out.textures_delta.clear();
        let (mut texts, mut bars) = (vec![], vec![]);
        fn flat(s: &egui::Shape, clip: Rect, texts: &mut Vec<(String, bool)>, bars: &mut Vec<Rect>) {
            match s {
                egui::Shape::Vec(v) => v.iter().for_each(|s| flat(s, clip, texts, bars)),
                egui::Shape::Text(t) => texts.push((t.galley.text().to_owned(), t.galley.elided)),
                egui::Shape::Rect(r) if r.fill.a() > 0 && r.rect.width() <= 8.0 && r.rect.height() > 40.0 => bars.push(r.rect.intersect(clip)),
                _ => {}
            }
        }
        popup.take().iter().for_each(|c| flat(&c.shape, c.clip_rect, &mut texts, &mut bars));
        (texts, bars)
    };
    run(vec![]);
    let at = field.get().center();
    let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    run(vec![egui::Event::PointerMoved(at), press(true)]);
    run(vec![press(false)]);
    run(vec![egui::Event::PointerMoved(pos2(210.0, 290.0))]);
    let (mut texts, mut bars) = (vec![], vec![]);
    for _ in 0..6 {
        (texts, bars) = run(vec![]);
    }
    let list_rect = || {
        ctx.memory(|m| m.layer_ids().filter(|l| l.order == egui::Order::Foreground && m.areas().is_visible(l)).last().and_then(|l| m.area_rect(l.id)))
    };
    let list = list_rect().expect("the list is open");
    assert!(screen.contains_rect(list), "the list {list:?} is inside the window");
    assert!(
        bars.iter().any(|b| b.width() > 0.0 && screen.contains_rect(*b) && list.contains_rect(*b)),
        "the bar shows in the list {list:?}: {bars:?}"
    );
    assert!(texts.iter().any(|(s, elided)| *elided && s.starts_with("Option 01")), "the long name is cut short: {texts:?}");
    // The window grows while the list is open: the list grows as wide as its names need, and no
    // wider.
    window.set(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 300.0)));
    for _ in 0..6 {
        (texts, _) = run(vec![]);
    }
    let grown = list_rect().expect("the list is still open");
    assert!(grown.width() < 500.0, "the list {grown:?} stays as wide as its names need");
    assert!(texts.iter().any(|(s, elided)| !*elided && s == labels[1]), "the long name shows in full: {texts:?}");
    // Narrowed and widened again, it comes back to the same width.
    for size in [screen.size(), vec2(1000.0, 300.0)] {
        window.set(Rect::from_min_size(Pos2::ZERO, size));
        for _ in 0..6 {
            run(vec![]);
        }
    }
    let again = list_rect().expect("the list is still open");
    assert!((again.width() - grown.width()).abs() < 0.5, "the list is as wide as before: {again:?}, {grown:?}");
    // Closed, the window narrowed again and the long option gone: the list opens as narrow as its
    // names, not as wide as the window.
    let escape = |pressed| egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    run(vec![escape(true)]);
    run(vec![escape(false)]);
    assert!(list_rect().is_none(), "Escape closes the list");
    window.set(screen);
    offered.set(1);
    run(vec![]);
    let at = field.get().center();
    let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    run(vec![egui::Event::PointerMoved(at), press(true)]);
    run(vec![press(false)]);
    run(vec![egui::Event::PointerMoved(pos2(210.0, 290.0))]);
    let mut widths = vec![];
    for _ in 0..4 {
        run(vec![]);
        widths.push(list_rect().expect("the list is open").width());
    }
    assert!(widths.iter().all(|w| *w < 180.0), "the list of short names stays narrow: {widths:?}");
    // Opened in a wide window, narrowed and widened again: it comes back to the width it opened at,
    // the long list that scrolls and a list of two that does not.
    for (which, count) in [(0, "40"), (2, "2")] {
        run(vec![escape(true)]);
        run(vec![escape(false)]);
        offered.set(which);
        window.set(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 300.0)));
        run(vec![]);
        run(vec![egui::Event::PointerMoved(at), press(true)]);
        run(vec![press(false)]);
        run(vec![egui::Event::PointerMoved(pos2(990.0, 290.0))]);
        for _ in 0..6 {
            run(vec![]);
        }
        let opened = list_rect().expect("the list is open");
        for size in [screen.size(), vec2(1000.0, 300.0)] {
            window.set(Rect::from_min_size(Pos2::ZERO, size));
            for _ in 0..6 {
                run(vec![]);
            }
        }
        let restored = list_rect().expect("the list is still open");
        // Within the half point that egui rounds a popup's position to.
        assert!((restored.width() - opened.width()).abs() < 0.5, "{count} options: the list is as wide as it opened: {restored:?}, {opened:?}");
    }
    // Opened in a narrow window tall enough for every option, then made wide and short: the list
    // scrolls now, and its bar's column comes on top of the long name, which shows in full.
    run(vec![escape(true)]);
    run(vec![escape(false)]);
    offered.set(0);
    window.set(Rect::from_min_size(Pos2::ZERO, vec2(220.0, 1400.0)));
    run(vec![]);
    run(vec![egui::Event::PointerMoved(at), press(true)]);
    run(vec![press(false)]);
    run(vec![egui::Event::PointerMoved(pos2(210.0, 1390.0))]);
    for _ in 0..6 {
        run(vec![]);
    }
    window.set(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 300.0)));
    for _ in 0..6 {
        (texts, _) = run(vec![]);
    }
    assert!(texts.iter().any(|(s, elided)| !*elided && s == labels[1]), "the long name shows in full beside the bar: {texts:?}");
}

// #5941: in a drop-down list narrowed to the window, an option whose name is cut short shows the
// full name in a tooltip while the pointer rests on it.
#[test]
fn a_dropdown_option_cut_short_shows_its_full_name_on_hover() {
    let mut items: Vec<String> = (0..10).map(|i| format!("Option {i:02}")).collect();
    items[1] = "Option 01, whose name is much wider than this window".into();
    let labels: Vec<&str> = items.iter().map(String::as_str).collect();
    let ctx = themed(Default::default());
    let field = std::cell::Cell::new(Rect::NOTHING);
    let mut add = |ui: &mut Ui| {
        ui.add_space(16.0);
        field.set(ui.horizontal(|ui| dropdown_names(ui, "hover-list", labels[0], &labels, 160.0)).response.rect);
    };
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(220.0, 300.0));
    let mut time = 0.0;
    // Every text painted, on any layer, with its rect and whether it was cut short.
    let mut run = |events: Vec<egui::Event>| {
        time += 0.1;
        let mut out = ctx.run_ui(egui::RawInput { time: Some(time), screen_rect: Some(screen), events, ..Default::default() }, &mut add);
        out.textures_delta.clear();
        let mut texts = vec![];
        fn flat(s: &egui::Shape, out: &mut Vec<(String, Rect, bool)>) {
            match s {
                egui::Shape::Vec(v) => v.iter().for_each(|s| flat(s, out)),
                egui::Shape::Text(t) => out.push((t.galley.text().to_owned(), t.visual_bounding_rect(), t.galley.elided)),
                _ => {}
            }
        }
        out.shapes.iter().for_each(|c| flat(&c.shape, &mut texts));
        texts
    };
    run(vec![]);
    let at = field.get().center();
    let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    run(vec![egui::Event::PointerMoved(at), press(true)]);
    run(vec![press(false)]);
    run(vec![egui::Event::PointerMoved(pos2(210.0, 290.0))]);
    let mut texts = vec![];
    for _ in 0..6 {
        texts = run(vec![]);
    }
    let full = |texts: &[(String, Rect, bool)]| texts.iter().any(|(s, _, elided)| !*elided && s == labels[1]);
    assert!(!full(&texts), "the long name is not shown in full before the hover: {texts:?}");
    let row = texts
        .iter()
        .find(|(s, _, elided)| *elided && s.starts_with("Option 01"))
        .map(|(_, r, _)| r.center())
        .unwrap_or_else(|| panic!("the long name is cut short: {texts:?}"));
    run(vec![egui::Event::PointerMoved(row)]);
    for _ in 0..10 {
        texts = run(vec![]);
    }
    assert!(full(&texts), "the full name shows on hover: {texts:?}");
}

/// Enter on text that reads as no number in a range field refuses it as a unit field does: the
/// field flashes and an open dialog is not confirmed with the old value.
#[test]
fn a_range_field_refuses_text_that_reads_as_no_number() {
    let typed = |text: &str| {
        let ctx = themed(Default::default());
        let picked = std::cell::Cell::new(None);
        let mut add = |ui: &mut Ui| {
            if let Some(v) = range_field(ui, "count", 12.0, 0.0..=100.0, "", 0, 80.0) {
                picked.set(Some(v));
            }
        };
        arm_field_focus(&ctx, "count");
        frame(&ctx, 0.0, vec![], &mut add);
        frame(&ctx, 0.1, vec![], &mut add);
        let enter = |pressed| egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
        frame(&ctx, 0.2, vec![egui::Event::Text(text.into()), enter(true), enter(false)], &mut add);
        let refused = ctx.data_mut(|d| d.remove_temp::<bool>(dialog_reject_commit_flag())).unwrap_or(false);
        (picked.get(), refused)
    };
    assert_eq!(typed("40"), (Some(40.0), false), "a number commits");
    assert_eq!(typed("abc"), (None, true), "text that reads as no number is refused");
    assert_eq!(typed("(10+5)*2"), (None, true), "parentheses the field doesn't read are refused");
}
