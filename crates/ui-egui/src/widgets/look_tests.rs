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

// Regression test for #2083: the rows of the panels' menus didn't light up under the pointer, as
// the menu bar's items do.
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

// Regression test for #3440: the Fill/Stroke chip's popover closed on the click that ended a
// drag between its proxies. It stays open for a click on its own anchor and while a panel
// drag is on; a click elsewhere still closes it.
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
    let panel_drag = || Some(PanelDrag::Symbol("s".into()));
    // Whether it is open after: a click elsewhere; a click on its anchor that doesn't toggle it;
    // a panel drag released elsewhere.
    let open = [open_after_click(far, None), open_after_click(None, None), open_after_click(far, panel_drag())];
    assert_eq!(open, [false, true, true]);
}
