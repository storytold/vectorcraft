/// The script filter: Thai lists families with Thai; Latin, those with neither Thai nor
/// Japanese.
#[test]
fn the_thai_filter_lists_fonts_with_thai() {
    use vectorcraft_text::FontTraits;
    let (thai, japanese, latin) =
        (FontTraits { thai: true, ..Default::default() }, FontTraits { japanese: true, ..Default::default() }, FontTraits::default());
    let passing = |t: FontTraits| (0..super::SCRIPTS).filter(|s| super::script_matches(*s, t)).collect::<Vec<_>>();
    assert_eq!(passing(thai), [0, 3]);
    assert_eq!(passing(japanese), [0, 1]);
    assert_eq!(passing(latin), [0, 2]);
    assert_eq!(super::script_label(3), "Thai");
    let zh = crate::i18n::Lang::from_code("zh-hans").unwrap();
    crate::i18n::set_current(zh);
    assert_eq!(super::script_label(1), crate::i18n::tr_ctx(zh, "font-script", "Japanese"));
    crate::i18n::set_current(crate::i18n::Lang::EN);
    assert_eq!(super::script_label(1), "Japanese");
}

/// #2277: the list's scroll bar has a column of its own, so the stars at the rows' right ends
/// stay clear of it.
#[test]
fn the_scroll_bar_leaves_the_stars_clear() {
    // Every family starred, so each row draws its star. Tests running alongside add fonts and
    // composite fonts to the process-wide database, which the menu lists too: the ★ filter keeps
    // the rows to the families starred here.
    let all = vectorcraft_text::FontDb::global().families();
    let ctx = egui::Context::default();
    let area = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 160.0));
    let mut st = super::MenuState { favorites_only: true, ..Default::default() };
    let look = super::MenuLook { samples: false, favorites: &all, ..Default::default() };
    let mut stars = vec![];
    for _ in 0..3 {
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(area), ..Default::default() }, |ui| {
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area));
            super::list(&mut child, egui::Id::unique("fonts"), &mut st, "", None, look);
        });
        out.textures_delta.clear();
        stars = out
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                // The rows' stars, not the ★ filter button at the top left.
                egui::Shape::Text(t) if t.galley.text() == "★" && t.pos.x > area.center().x => Some(t.visual_bounding_rect().right()),
                _ => None,
            })
            .collect();
    }
    let families = st.rows.len();
    assert!(!stars.is_empty() || families == 0, "{families} families, no row stars drawn");
    // The rows don't fill the list without enough families to scroll, and then there is no bar.
    if families as f32 * look.row > area.height() {
        let bar = super::widgets::list_scroll_style();
        let column = bar.bar_width + bar.bar_inner_margin + bar.bar_outer_margin;
        for right in stars {
            assert!(right <= area.right() - column + 0.5, "a star at {right} runs under the scroll bar");
        }
    }
}

use super::{Row, remap_highlight};

fn family(name: &str) -> Row {
    Row::Family(name.to_string())
}

/// The families the font menu lists that stay listed, in its order: the menu's list without the
/// composite fonts at its top, which tests running alongside define and remove in the process-wide
/// database.
fn lasting_families() -> Vec<String> {
    vectorcraft_text::FontDb::global().family_list().iter().filter(|f| !f.starts_with('.')).cloned().collect()
}

#[test]
fn recent_fonts_are_first_available_unique_and_expandable() {
    let names: Vec<_> = ["Alpha", "Bravo", "Charlie"].map(str::to_string).into();
    let recent: Vec<_> = ["Charlie", "Missing", "BRAVO", "charlie"].map(str::to_string).into();
    let rows = super::grouped_rows(&names, &recent, &["bravo".into()], |_| vec!["Bold".into()]);
    assert_eq!(
        rows,
        [
            Row::Heading("Recent Fonts"),
            family("Charlie"),
            family("Bravo"),
            Row::Style("Bravo".into(), "Bold".into()),
            Row::Heading("Fonts"),
            family("Alpha")
        ]
    );
    // A filter removing a recent family also removes it from the recent section.
    let filtered = super::grouped_rows(&names[..1], &recent, &[], |_| vec![]);
    assert_eq!(filtered, [family("Alpha")]);
    assert!(super::grouped_rows(&[], &recent, &[], |_| vec![]).is_empty());
}

#[test]
fn recent_section_rebuilds_and_arrows_skip_its_headings() {
    let names = lasting_families();
    if names.len() < 3 {
        return;
    }
    let recent = vec![names[2].clone(), names[1].clone()];
    let ctx = egui::Context::default();
    let mut st = super::MenuState::default();
    let mut frame = |events, recent: &[String]| {
        let mut picked = None;
        let input =
            egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 800.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            picked = super::list(
                ui,
                egui::Id::unique("recent-test"),
                &mut st,
                &names[2],
                None,
                super::MenuLook { samples: false, recent, ..Default::default() },
            );
        });
        out.textures_delta.clear();
        (picked, st.rows.to_vec())
    };
    for _ in 0..3 {
        frame(vec![], &recent);
    }
    let key = |key| {
        [true, false].map(|pressed| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Default::default() }).to_vec()
    };
    assert_eq!(frame(key(egui::Key::ArrowDown), &recent).0, Some(super::FontPick::Preview(names[1].clone(), None)));
    // Other tests add fonts to the process-wide database while this one runs, so the first
    // family under the Fonts heading need not be the one this test's earlier snapshot named.
    // Read it from the rows this frame built: the highlight must skip the heading and land on
    // that family, not on a heading or a row from the recent section.
    let (picked, rows) = frame(key(egui::Key::ArrowDown), &recent);
    assert_eq!(rows.get(3), Some(&Row::Heading("Fonts")), "{rows:?}");
    let (first, style) = rows.get(4).and_then(Row::pick).expect("a family below the Fonts heading");
    assert_eq!(picked, Some(super::FontPick::Preview(first, style)), "skip Fonts heading");
    assert_eq!(frame(key(egui::Key::ArrowUp), &recent).0, Some(super::FontPick::Preview(names[1].clone(), None)));
    // Updating recent history while the menu is open must not retarget the highlight.
    let updated = vec![names[1].clone()];
    let (_, rows) = frame(vec![], &updated);
    assert_eq!(rows[1], family(&names[1]));
    assert_eq!(frame(key(egui::Key::Enter), &updated).0, Some(super::FontPick::Chosen(names[1].clone(), None)));
}

#[test]
fn recent_fonts_obey_search_and_favorite_filters() {
    let names = lasting_families();
    if names.len() < 3 {
        return;
    }
    let recent = vec![names[1].clone(), names[0].clone()];
    let favorites = vec![names[0].clone()];
    let ctx = egui::Context::default();
    let mut st = super::MenuState { favorites_only: true, ..Default::default() };
    let mut frame = |events| {
        let mut pick = None;
        let mut out = ctx.run_ui(
            egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 800.0))), ..Default::default() },
            |ui| {
                pick = super::list(
                    ui,
                    egui::Id::unique("filtered-recent"),
                    &mut st,
                    &names[2],
                    None,
                    super::MenuLook { samples: false, recent: &recent, favorites: &favorites, ..Default::default() },
                );
            },
        );
        out.textures_delta.clear();
        (pick, st.rows.to_vec())
    };
    for _ in 0..3 {
        frame(vec![]);
    }
    assert_eq!(frame(vec![]).1, [Row::Heading("Recent Fonts"), family(&names[0])]);
    let key = [true, false]
        .map(|pressed| egui::Event::Key { key: egui::Key::ArrowDown, physical_key: None, pressed, repeat: false, modifiers: Default::default() })
        .to_vec();
    assert_eq!(frame(key).0, Some(super::FontPick::Preview(names[0].clone(), None)), "skip Recent Fonts heading");
    let (pick, rows) = frame(vec![egui::Event::Text("NoSuchFontFor3515".into())]);
    assert!(rows.is_empty(), "search must filter the recent section too: {rows:?}");
    assert_eq!(pick, Some(super::FontPick::EndPreview));
}

#[test]
fn opening_on_another_font_keeps_the_recent_section_visible() {
    let names = lasting_families();
    let Some(current) = names.last() else { return };
    let recent = vec![names[0].clone()];
    let ctx = egui::Context::default();
    let mut st = super::MenuState::default();
    let mut visible = false;
    for _ in 0..3 {
        let mut out = ctx.run_ui(
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 400.0))), ..Default::default() },
            |ui| {
                super::list(
                    ui,
                    egui::Id::unique("recent-at-top"),
                    &mut st,
                    current,
                    None,
                    super::MenuLook { samples: false, recent: &recent, ..Default::default() },
                );
            },
        );
        out.textures_delta.clear();
        visible = out.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == "Recent Fonts"));
    }
    assert!(visible, "opening the picker must show its recent section");
}

#[test]
fn keeps_the_same_family_when_other_rows_are_inserted_or_removed() {
    let before = [family("Alpha"), family("Bravo"), family("Charlie")];
    assert_eq!(remap_highlight(&before, Some(2), &[family("Bravo"), family("Charlie")]), Some(1));
    assert_eq!(remap_highlight(&before, Some(2), &[family("Alpha"), family("New"), family("Bravo"), family("Charlie")]), Some(3));
}

#[test]
fn clears_the_highlight_when_its_family_is_filtered_out() {
    let before = [family("Alpha"), family("Bravo"), family("Charlie")];
    assert_eq!(remap_highlight(&before, Some(1), &[family("Alpha"), family("Charlie")]), None);
    assert_eq!(remap_highlight(&before, Some(10), &before), None);
    assert_eq!(remap_highlight(&before, None, &before), None);
}

#[test]
fn expanded_style_rows_do_not_retarget_the_highlight() {
    let before = [family("Alpha"), family("Bravo")];
    let after = [family("Alpha"), Row::Style("Alpha".into(), "Bold".into()), family("Bravo")];
    assert_eq!(remap_highlight(&before, Some(1), &after), Some(2));
    assert_eq!(remap_highlight(&after, Some(1), &before), None);
}

/// #5253: each wheel notch over the closed field steps one family, down the menu or up it, and the
/// steps stop at its first and last family without going round.
#[test]
fn the_wheel_steps_one_family_per_notch_and_stops_at_the_ends() {
    let order: Vec<String> = ["Alpha", "Bravo", "Charlie", "Delta"].map(str::to_string).into();
    let steps = |current, notches| super::wheel_steps(&order, current, notches);
    assert_eq!(steps("Bravo", 1), ["Charlie"]);
    assert_eq!(steps("bravo", -1), ["Alpha"], "any case");
    assert_eq!(steps("Alpha", 2), ["Bravo", "Charlie"], "a step for each notch");
    assert_eq!(steps("Bravo", 5), ["Charlie", "Delta"], "stops at the last family");
    assert_eq!(steps("Charlie", -5), ["Bravo", "Alpha"], "stops at the first family");
    assert!(steps("Delta", 1).is_empty() && steps("Alpha", -1).is_empty(), "no wrapping");
    assert!(steps("Missing", 1).is_empty() && steps("Bravo", 0).is_empty());
}

/// #5253: the wheel follows the open menu's order and filters: the recent families first, then the
/// others in the database's order, under the favourites (and other) filters.
#[test]
fn the_wheel_follows_the_menus_order_and_filters() {
    let names = lasting_families();
    if names.len() < 3 {
        return;
    }
    let recent = vec![names[2].clone()];
    let look = super::MenuLook { samples: false, recent: &recent, ..Default::default() };
    let order = super::wheel_order(&super::MenuState::default(), look);
    assert_eq!(order.first(), Some(&names[2]), "the recent family first");
    // Other tests add fonts to the process-wide database meanwhile: compare places, not neighbours.
    let at = |f: &String| order.iter().position(|x| x == f);
    assert!(matches!((at(&names[0]), at(&names[1])), (Some(a), Some(b)) if 0 < a && a < b), "then the others in the menu's order");
    assert_eq!(order.iter().filter(|f| **f == names[2]).count(), 1, "once each");
    let favorites = vec![names[0].clone(), names[1].clone()];
    let st = super::MenuState { favorites_only: true, ..Default::default() };
    let order = super::wheel_order(&st, super::MenuLook { favorites: &favorites, ..look });
    assert_eq!(order, favorites, "the favourites filter");
}

/// A field removed by a later layout pass must not intercept the next wheel event.
#[test]
fn a_family_field_omitted_from_the_last_pass_leaves_the_wheel_to_its_owner() {
    let ctx = egui::Context::default();
    let mut point = egui::Pos2::ZERO;
    for _ in 0..3 {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let resp = ui.button("Family");
            point = resp.rect.center();
            super::remember_wheel_field(ui, &resp, true);
        });
        out.textures_delta.clear();
    }
    let mut pass = 0;
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        if pass == 0 {
            let resp = ui.button("Family");
            point = resp.rect.center();
            super::remember_wheel_field(ui, &resp, true);
            ui.ctx().request_discard("the next pass removes the field");
        }
        pass += 1;
    });
    out.textures_delta.clear();
    assert!(pass > 1);
    let mut raw = egui::RawInput {
        events: vec![
            egui::Event::PointerMoved(point),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0.0, -1.0),
                phase: egui::TouchPhase::Move,
                modifiers: Default::default(),
                source: egui::MouseWheelSource::Unknown,
            },
        ],
        ..Default::default()
    };
    super::take_raw_wheel(&ctx, &mut raw);
    assert!(raw.events.iter().any(|e| matches!(e, egui::Event::MouseWheel { .. })));
}

/// Activation leaves later wheel input to the popup; an earlier turn still belongs to the field.
#[test]
fn activating_the_family_popup_leaves_its_wheel_input_available() {
    let activations = [None, Some((egui::Key::Enter, egui::Modifiers::NONE)), Some((egui::Key::Space, egui::Modifiers::SHIFT))];
    for activation in activations {
        let ctx = egui::Context::default();
        let frame = |events| {
            let mut raw = egui::RawInput { events, ..Default::default() };
            super::take_raw_wheel(&ctx, &mut raw);
            let mut point = egui::Pos2::ZERO;
            let mut out = ctx.run_ui(raw, |ui| {
                let resp = ui.button("Family");
                point = resp.rect.center();
                if activation.is_some() {
                    resp.request_focus();
                }
                super::remember_wheel_field(ui, &resp, true);
            });
            out.textures_delta.clear();
            point
        };
        let mut point = egui::Pos2::ZERO;
        for _ in 0..3 {
            point = frame(vec![]);
        }
        let button =
            |pressed| egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let opener = if let Some((key, modifiers)) = activation {
            egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
        } else {
            frame(vec![egui::Event::PointerMoved(point), button(true)]);
            button(false)
        };
        let turn = || egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        };
        let mut events = vec![egui::Event::PointerMoved(point)];
        if activation.is_some() {
            events.push(turn());
        }
        events.extend([opener, turn()]);
        let mut raw = egui::RawInput { events, ..Default::default() };
        super::take_raw_wheel(&ctx, &mut raw);
        assert_eq!(raw.events.iter().filter(|e| matches!(e, egui::Event::MouseWheel { .. })).count(), 1);
    }
}

/// A family field at `top`, the input's clock at `time`. Returns the wheel events left in the
/// input, the notches the field took and the point in the middle of the field.
fn stream_frame(ctx: &egui::Context, top: Option<f32>, time: f64, events: Vec<egui::Event>) -> (usize, i32, egui::Pos2) {
    covered_stream_frame(ctx, top, false, time, events)
}

/// [`stream_frame`], with an overlay over the whole field when `covered` is set.
fn covered_stream_frame(ctx: &egui::Context, top: Option<f32>, covered: bool, time: f64, events: Vec<egui::Event>) -> (usize, i32, egui::Pos2) {
    let mut raw = egui::RawInput { time: Some(time), events, ..Default::default() };
    super::take_raw_wheel(ctx, &mut raw);
    let left = raw.events.iter().filter(|e| matches!(e, egui::Event::MouseWheel { .. })).count();
    let mut notches = 0;
    let mut point = egui::Pos2::ZERO;
    let mut out = ctx.run_ui(raw, |ui| {
        let Some(top) = top else { return };
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, top), egui::vec2(120.0, 24.0));
        let resp = ui.put(rect, egui::Button::new("Family"));
        point = resp.rect.center();
        for turn in super::remember_wheel_field(ui, &resp, true) {
            if let super::WheelInput::Notches(n) = turn {
                notches += n;
            }
        }
        if covered {
            let cover = rect.expand(5.0);
            egui::Area::new(egui::Id::unique("stream overlay")).order(egui::Order::Foreground).fixed_pos(cover.min).show(ui.ctx(), |ui| {
                ui.allocate_exact_size(cover.size(), egui::Sense::click());
            });
        }
    });
    out.textures_delta.clear();
    (left, notches, point)
}

fn stream_turn(phase: egui::TouchPhase, source: egui::MouseWheelSource, dy: f32) -> egui::Event {
    egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: egui::vec2(0.0, dy), phase, modifiers: Default::default(), source }
}

fn plain_turn() -> egui::Event {
    stream_turn(egui::TouchPhase::Move, egui::MouseWheelSource::Unknown, -1.0)
}

/// A context with a family field drawn at `top` for three frames, from clock time 100.
fn stream_context(top: f32) -> (egui::Context, egui::Pos2) {
    let ctx = egui::Context::default();
    let mut point = egui::Pos2::ZERO;
    for i in 0..3 {
        point = stream_frame(&ctx, Some(top), 100.0 + f64::from(i), vec![]).2;
    }
    (ctx, point)
}

/// A wheel stream the family field took stays its own when the pointer moves onto a number field
/// beside it: the number field steps only after the wheel pauses for the stream gap.
#[test]
fn a_stream_the_family_field_took_leaves_a_number_field_beside_it() {
    let ctx = egui::Context::default();
    let value = std::cell::Cell::new(10.0);
    let (family, number) = (std::cell::Cell::new(egui::Pos2::ZERO), std::cell::Cell::new(egui::Pos2::ZERO));
    let frame = |time: f64, events: Vec<egui::Event>| {
        let mut raw = egui::RawInput { time: Some(time), events, ..Default::default() };
        super::take_raw_wheel(&ctx, &mut raw);
        let mut notches = 0;
        let mut out = ctx.run_ui(raw, |ui| {
            let resp = ui.put(egui::Rect::from_min_size(egui::pos2(10.0, 40.0), egui::vec2(120.0, 24.0)), egui::Button::new("Family"));
            family.set(resp.rect.center());
            for turn in super::remember_wheel_field(ui, &resp, true) {
                if let super::WheelInput::Notches(n) = turn {
                    notches += n;
                }
            }
            ui.add_space(40.0);
            let top = ui.cursor().top();
            if let Some(v) = crate::widgets::plain_field(ui, "size", value.get(), "", 0, 80.0) {
                value.set(v);
            }
            number.set(egui::pos2(ui.min_rect().left() + 20.0, (top + ui.min_rect().bottom()) / 2.0));
        });
        out.textures_delta.clear();
        notches
    };
    for i in 0..3 {
        frame(100.0 + f64::from(i), vec![]);
    }
    assert_eq!(frame(110.0, vec![egui::Event::PointerMoved(family.get()), plain_turn()]), 1, "the family field took the turn");
    frame(110.1, vec![egui::Event::PointerMoved(number.get()), plain_turn()]);
    assert_eq!(value.get(), 10.0, "the stream began on the family field");
    frame(110.1 + crate::widgets::WHEEL_STREAM_GAP + 0.1, vec![plain_turn()]);
    assert_eq!(value.get(), 9.0, "a new stream");
}

/// A field that moved under a pointer that stayed put leaves the wheel, including a reported
/// pointer position that did not change, until the pointer really moves.
#[test]
fn a_field_that_slides_under_a_still_pointer_leaves_the_wheel() {
    let (ctx, point) = stream_context(40.0);
    let below = egui::pos2(point.x, point.y + 200.0);
    stream_frame(&ctx, Some(40.0), 110.0, vec![egui::Event::PointerMoved(below)]);
    stream_frame(&ctx, Some(40.0 + 200.0), 111.0, vec![]);
    let (left, notches, moved) = stream_frame(&ctx, Some(240.0), 120.0, vec![plain_turn()]);
    assert_eq!(moved, below);
    assert_eq!((left, notches), (1, 0), "the pointer did not move");
    let (left, notches, _) = stream_frame(&ctx, Some(240.0), 130.0, vec![egui::Event::PointerMoved(below), plain_turn()]);
    assert_eq!((left, notches), (1, 0), "a report of the same position is not a move");
    let nudged = below + egui::vec2(2.0, 0.0);
    let (left, notches, _) = stream_frame(&ctx, Some(240.0), 140.0, vec![egui::Event::PointerMoved(nudged), plain_turn()]);
    assert_eq!((left, notches), (0, 1), "the pointer moved onto the field");
}

/// A field the pointer was moved onto loses that when the field moves, even back to the same
/// place, or when it is missing from a pass.
#[test]
fn a_field_that_moves_away_and_back_needs_the_pointer_moved_again() {
    let (ctx, point) = stream_context(40.0);
    stream_frame(&ctx, Some(40.0), 110.0, vec![egui::Event::PointerMoved(point)]);
    stream_frame(&ctx, Some(44.0), 111.0, vec![]);
    stream_frame(&ctx, Some(40.0), 112.0, vec![]);
    let (left, notches, _) = stream_frame(&ctx, Some(40.0), 120.0, vec![plain_turn()]);
    assert_eq!((left, notches), (1, 0), "the field moved after the pointer did");

    let (ctx, point) = stream_context(40.0);
    stream_frame(&ctx, Some(40.0), 110.0, vec![egui::Event::PointerMoved(point)]);
    stream_frame(&ctx, None, 111.0, vec![]);
    stream_frame(&ctx, Some(40.0), 112.0, vec![]);
    stream_frame(&ctx, Some(40.0), 113.0, vec![]);
    let (left, notches, _) = stream_frame(&ctx, Some(40.0), 120.0, vec![plain_turn()]);
    assert_eq!((left, notches), (1, 0), "the field was gone for a pass");

    let (ctx, point) = stream_context(40.0);
    stream_frame(&ctx, Some(40.0), 110.0, vec![egui::Event::PointerMoved(point), egui::Event::PointerGone]);
    let (left, notches, _) = stream_frame(&ctx, Some(40.0), 120.0, vec![plain_turn()]);
    assert_eq!((left, notches), (1, 0), "the pointer left the window");
}

/// An overlay that comes and goes over a still pointer, with no input while it is up, leaves the
/// field needing the pointer moved again; a stream the field held leaves it.
#[test]
fn a_field_covered_by_an_overlay_needs_the_pointer_moved_again() {
    let (ctx, point) = stream_context(40.0);
    stream_frame(&ctx, Some(40.0), 110.0, vec![egui::Event::PointerMoved(point)]);
    covered_stream_frame(&ctx, Some(40.0), true, 111.0, vec![]);
    covered_stream_frame(&ctx, Some(40.0), true, 112.0, vec![]);
    stream_frame(&ctx, Some(40.0), 113.0, vec![]);
    stream_frame(&ctx, Some(40.0), 114.0, vec![]);
    let (left, notches, _) = stream_frame(&ctx, Some(40.0), 120.0, vec![plain_turn()]);
    assert_eq!((left, notches), (1, 0), "the overlay covered the field");
    let nudged = point + egui::vec2(2.0, 0.0);
    let (left, notches, _) = stream_frame(&ctx, Some(40.0), 130.0, vec![egui::Event::PointerMoved(nudged), plain_turn()]);
    assert_eq!((left, notches), (0, 1), "the pointer moved onto the field again");

    let (ctx, point) = stream_context(40.0);
    let (left, notches, _) = stream_frame(&ctx, Some(40.0), 110.0, vec![egui::Event::PointerMoved(point), plain_turn()]);
    assert_eq!((left, notches), (0, 1));
    covered_stream_frame(&ctx, Some(40.0), true, 110.1, vec![]);
    covered_stream_frame(&ctx, Some(40.0), true, 110.2, vec![]);
    stream_frame(&ctx, Some(40.0), 110.3, vec![]);
    let (left, notches, _) = stream_frame(&ctx, Some(40.0), 110.4, vec![plain_turn()]);
    assert_eq!((left, notches), (1, 0), "the overlay covered the field during the stream");
}

/// A field missing from a pass loses the pointer's placement while another font field keeps
/// drawing in that pass, as when a panel collapses and the Control bar's field stays.
#[test]
fn a_field_gone_for_a_pass_beside_another_field_needs_the_pointer_moved_again() {
    let ctx = egui::Context::default();
    let field_at = |ui: &mut egui::Ui, top: f32| {
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, top), egui::vec2(120.0, 24.0));
        let resp = ui.put(rect, egui::Button::new("Family"));
        super::remember_wheel_field(ui, &resp, true);
    };
    let frame = |time: f64, family: bool, events: Vec<egui::Event>| {
        let mut raw = egui::RawInput { time: Some(time), events, ..Default::default() };
        super::take_raw_wheel(&ctx, &mut raw);
        let left = raw.events.iter().filter(|e| matches!(e, egui::Event::MouseWheel { .. })).count();
        let mut out = ctx.run_ui(raw, |ui| {
            field_at(ui, 200.0);
            if family {
                field_at(ui, 40.0);
            }
        });
        out.textures_delta.clear();
        left
    };
    for i in 0..3 {
        frame(100.0 + f64::from(i), true, vec![]);
    }
    let point = egui::pos2(70.0, 52.0);
    assert_eq!(frame(110.0, true, vec![egui::Event::PointerMoved(point), plain_turn()]), 0, "the pointer moved onto the field");
    frame(120.0, false, vec![]);
    frame(121.0, true, vec![]);
    frame(122.0, true, vec![]);
    assert_eq!(frame(130.0, true, vec![plain_turn()]), 1, "the field was gone for a pass");
}

/// A pointer at rest on a field that stayed put keeps stepping it, stream after stream.
#[test]
fn a_pointer_resting_on_its_field_keeps_the_wheel_across_streams() {
    let (ctx, point) = stream_context(40.0);
    let (left, notches, _) = stream_frame(&ctx, Some(40.0), 110.0, vec![egui::Event::PointerMoved(point), plain_turn()]);
    assert_eq!((left, notches), (0, 1));
    let (left, notches, _) = stream_frame(&ctx, Some(40.0), 110.0 + crate::widgets::WHEEL_STREAM_GAP / 2.0, vec![plain_turn()]);
    assert_eq!((left, notches), (0, 1), "the same stream");
    let (left, notches, _) = stream_frame(&ctx, Some(40.0), 200.0, vec![plain_turn()]);
    assert_eq!((left, notches), (0, 1), "a new stream, with the pointer still on the field");
}

/// A stream that began elsewhere stays there when the pointer moves onto a field, until a gap
/// with no wheel input ends it.
#[test]
fn a_wheel_stream_that_began_elsewhere_stays_there_until_a_gap() {
    let (ctx, point) = stream_context(40.0);
    let away = point + egui::vec2(0.0, 200.0);
    let step = crate::widgets::WHEEL_STREAM_GAP * 0.8;
    let (left, _, _) = stream_frame(&ctx, Some(40.0), 110.0, vec![egui::Event::PointerMoved(away), plain_turn()]);
    assert_eq!(left, 1, "the stream began off the field");
    let (left, notches, _) = stream_frame(&ctx, Some(40.0), 110.0 + step, vec![egui::Event::PointerMoved(point), plain_turn()]);
    assert_eq!((left, notches), (1, 0), "a nudge onto the field does not take a running stream");
    let (left, notches, _) = stream_frame(&ctx, Some(40.0), 110.0 + 2.0 * step, vec![plain_turn()]);
    assert_eq!((left, notches), (1, 0), "each event renews the stream");
    let (left, notches, _) = stream_frame(&ctx, Some(40.0), 110.0 + 2.0 * step + crate::widgets::WHEEL_STREAM_GAP, vec![plain_turn()]);
    assert_eq!((left, notches), (0, 1), "the stream ended, and the pointer was moved onto the field");
}

/// A stream that leaves its field goes to what is under the pointer and does not come back.
#[test]
fn a_wheel_stream_that_leaves_its_field_does_not_return() {
    let (ctx, point) = stream_context(40.0);
    let away = point + egui::vec2(0.0, 200.0);
    let (left, notches, _) = stream_frame(&ctx, Some(40.0), 110.0, vec![egui::Event::PointerMoved(point), plain_turn()]);
    assert_eq!((left, notches), (0, 1));
    let (left, notches, _) = stream_frame(&ctx, Some(40.0), 110.1, vec![egui::Event::PointerMoved(away), plain_turn()]);
    assert_eq!((left, notches), (1, 0), "the pointer left the field");
    let (left, notches, _) = stream_frame(&ctx, Some(40.0), 110.2, vec![egui::Event::PointerMoved(point), plain_turn()]);
    assert_eq!((left, notches), (1, 0), "the stream stays off the field");
}

/// A stream loses its field when the pointer leaves without a wheel turn or when the field moves.
#[test]
fn a_wheel_stream_loses_its_field_when_the_pointer_leaves_or_the_field_moves() {
    let (ctx, point) = stream_context(40.0);
    let away = point + egui::vec2(0.0, 200.0);
    let (left, notches, _) = stream_frame(&ctx, Some(40.0), 110.0, vec![egui::Event::PointerMoved(point), plain_turn()]);
    assert_eq!((left, notches), (0, 1));
    stream_frame(&ctx, Some(40.0), 110.1, vec![egui::Event::PointerMoved(away)]);
    let (left, notches, _) = stream_frame(&ctx, Some(40.0), 110.2, vec![egui::Event::PointerMoved(point), plain_turn()]);
    assert_eq!((left, notches), (1, 0), "the pointer left the field during the stream");

    let (ctx, point) = stream_context(40.0);
    let (left, notches, _) = stream_frame(&ctx, Some(40.0), 110.0, vec![egui::Event::PointerMoved(point), plain_turn()]);
    assert_eq!((left, notches), (0, 1));
    stream_frame(&ctx, Some(44.0), 110.1, vec![]);
    let (left, notches, _) = stream_frame(&ctx, Some(44.0), 110.2, vec![plain_turn()]);
    assert_eq!((left, notches), (1, 0), "the field moved during the stream");

    let (ctx, point) = stream_context(40.0);
    let (left, notches, _) = stream_frame(&ctx, Some(40.0), 110.0, vec![egui::Event::PointerMoved(point), plain_turn()]);
    assert_eq!((left, notches), (0, 1));
    stream_frame(&ctx, Some(40.0), 110.1, vec![egui::Event::PointerGone]);
    let (left, notches, _) = stream_frame(&ctx, Some(40.0), 110.2, vec![egui::Event::PointerMoved(point), plain_turn()]);
    assert_eq!((left, notches), (1, 0), "the pointer left the window during the stream");
}

/// Trackpad momentum and zero-delta phase events keep their stream going, and momentum alone does
/// not start a stream on a field.
#[test]
fn trackpad_momentum_stays_with_its_stream() {
    use egui::{MouseWheelSource::Momentum, MouseWheelSource::Trackpad, TouchPhase};
    let (ctx, point) = stream_context(40.0);
    let away = point + egui::vec2(0.0, 200.0);
    let step = crate::widgets::WHEEL_STREAM_GAP * 0.8;
    stream_frame(&ctx, Some(40.0), 110.0, vec![egui::Event::PointerMoved(away), stream_turn(TouchPhase::Move, Trackpad, -1.0)]);
    // The fingers lift with no delta, the pointer moves onto the field, and momentum follows.
    stream_frame(&ctx, Some(40.0), 110.0 + step, vec![stream_turn(TouchPhase::End, Trackpad, 0.0)]);
    let (left, notches, _) =
        stream_frame(&ctx, Some(40.0), 110.0 + 2.0 * step, vec![egui::Event::PointerMoved(point), stream_turn(TouchPhase::Start, Momentum, -1.0)]);
    assert_eq!((left, notches), (1, 0), "momentum continues the stream that began off the field");
    let (left, notches, _) = stream_frame(&ctx, Some(40.0), 200.0, vec![stream_turn(TouchPhase::Move, Momentum, -1.0)]);
    assert_eq!((left, notches), (1, 0), "momentum does not start a stream on the field");
    let (left, notches, _) = stream_frame(&ctx, Some(40.0), 300.0, vec![stream_turn(TouchPhase::Move, Trackpad, -1.0)]);
    assert_eq!((left, notches), (0, 1), "fingers on the trackpad do");
}

/// A stream that began as a sideways scroll stays off the field when it turns vertical.
#[test]
fn a_sideways_turn_that_starts_a_stream_keeps_it_off_the_field() {
    let (ctx, point) = stream_context(40.0);
    let sideways = egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(-1.0, 0.0),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
        source: egui::MouseWheelSource::Unknown,
    };
    let (left, notches, _) = stream_frame(&ctx, Some(40.0), 110.0, vec![egui::Event::PointerMoved(point), sideways]);
    assert_eq!((left, notches), (1, 0));
    let (left, notches, _) = stream_frame(&ctx, Some(40.0), 110.1, vec![plain_turn()]);
    assert_eq!((left, notches), (1, 0), "the stream began as a sideways scroll");
}

/// A stream loses its field when a layout pass is skipped, even though the field comes back in the
/// same place before the stream ends.
#[test]
fn a_wheel_stream_loses_its_field_when_a_pass_is_skipped() {
    let (ctx, point) = stream_context(40.0);
    let (left, notches, _) = stream_frame(&ctx, Some(40.0), 110.0, vec![egui::Event::PointerMoved(point), plain_turn()]);
    assert_eq!((left, notches), (0, 1));
    stream_frame(&ctx, None, 110.1, vec![]);
    stream_frame(&ctx, Some(40.0), 110.2, vec![]);
    let (left, notches, _) = stream_frame(&ctx, Some(40.0), 110.3, vec![plain_turn()]);
    assert_eq!((left, notches), (1, 0), "the field missed a pass during the stream");
}

/// A modified turn is left in the input and keeps the stream away from the field.
#[test]
fn a_modified_turn_that_starts_a_stream_keeps_it_off_the_field() {
    let (ctx, point) = stream_context(40.0);
    let zoom = egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0.0, -1.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::COMMAND,
        source: egui::MouseWheelSource::Unknown,
    };
    let (left, notches, _) = stream_frame(&ctx, Some(40.0), 110.0, vec![egui::Event::PointerMoved(point), zoom]);
    assert_eq!((left, notches), (1, 0));
    let (left, notches, _) = stream_frame(&ctx, Some(40.0), 110.1, vec![plain_turn()]);
    assert_eq!((left, notches), (1, 0), "the stream began as a modified turn");
}

/// The Font Technology filter's Variable is the font adjective, not the Variables panel's noun.
#[test]
fn the_variable_filter_is_named_as_fonts_are() {
    use crate::i18n::{Lang, tr, tr_ctx};
    let (ja, zh) = (Lang::from_code("ja").unwrap(), Lang::from_code("zh-hans").unwrap());
    assert_eq!((tr_ctx(ja, "font-technology", "Variable"), tr(ja, "Variable")), ("バリアブル", "変数"));
    assert_eq!((tr_ctx(zh, "font-technology", "Variable"), tr(zh, "Variable")), ("可变", "变量"));
    assert_eq!(tr(ja, "Font Technology"), "フォントテクノロジー");
    assert_eq!(super::variable_label(), "Variable");
}

/// Font Technology › Variable lists only variable font families, together with the other
/// filters; clicked again, every family is back.
#[test]
fn the_variable_filter_lists_only_variable_families() {
    use vectorcraft_text::FontClass;
    use vectorcraft_text::test_fonts::{TWO_AXIS_FAMILY, two_axis_font};
    let db = vectorcraft_text::FontDb::global();
    db.add_font(two_axis_font().unwrap());
    let ctx = egui::Context::default();
    let mut st = super::MenuState::default();
    let mut favorites: Vec<String> = vec![];
    let frame = |st: &mut super::MenuState, events, favorites: &[String]| {
        let mut out = ctx.run_ui(
            egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 800.0))), ..Default::default() },
            |ui| {
                super::list(
                    ui,
                    egui::Id::unique("variable-filter"),
                    st,
                    "Source Sans 3",
                    None,
                    super::MenuLook { samples: false, favorites, ..Default::default() },
                );
            },
        );
        out.textures_delta.clear();
        let button = out.shapes.iter().find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text() == "Variable" => Some(t.visual_bounding_rect().center()),
            _ => None,
        });
        let families: Vec<String> = st
            .rows
            .iter()
            .filter_map(|r| match r {
                Row::Family(f) => Some(f.clone()),
                _ => None,
            })
            .collect();
        (families, button)
    };
    let has = |list: &[String], f: &str| list.iter().any(|x| x == f);
    let click = |st: &mut super::MenuState, favorites: &[String]| {
        let (_, at) = frame(st, vec![], favorites);
        let at = at.expect("the Variable filter is drawn");
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(st, vec![egui::Event::PointerMoved(at), button(true)], favorites);
        frame(st, vec![button(false)], favorites);
        frame(st, vec![], favorites).0
    };
    for _ in 0..3 {
        frame(&mut st, vec![], &favorites);
    }
    let all = frame(&mut st, vec![], &favorites).0;
    assert!(has(&all, TWO_AXIS_FAMILY) && has(&all, "Source Sans 3") && has(&all, "JetBrains Mono"), "{all:?}");
    let variable = click(&mut st, &favorites);
    assert!(st.variable_only, "the click turns the filter on");
    assert!(has(&variable, TWO_AXIS_FAMILY), "{variable:?}");
    assert!(!has(&variable, "Source Sans 3") && !has(&variable, "Inter") && !has(&variable, "JetBrains Mono"), "static families: {variable:?}");
    assert!(variable.iter().all(|f| db.family_traits(f).variable), "{variable:?}");
    // With the kind, script and favourites filters: each must pass.
    st.class = Some(FontClass::Sans);
    assert!(has(&frame(&mut st, vec![], &favorites).0, TWO_AXIS_FAMILY));
    st.class = Some(FontClass::Monospaced);
    assert!(!has(&frame(&mut st, vec![], &favorites).0, TWO_AXIS_FAMILY), "a sans serif family under Monospaced");
    st.class = None;
    st.script = 3;
    assert!(!has(&frame(&mut st, vec![], &favorites).0, TWO_AXIS_FAMILY), "a family without Thai under Thai");
    st.script = 0;
    st.favorites_only = true;
    favorites.push("Source Sans 3".into());
    assert!(frame(&mut st, vec![], &favorites).0.is_empty(), "the one favourite is static");
    favorites.push(TWO_AXIS_FAMILY.into());
    assert_eq!(frame(&mut st, vec![], &favorites).0, [TWO_AXIS_FAMILY]);
    st.favorites_only = false;
    // Off again: every family is listed.
    let lasting = lasting_families();
    let again = click(&mut st, &favorites);
    assert!(!st.variable_only);
    assert!(lasting.iter().all(|f| has(&again, f)), "{again:?}");
    assert!(has(&again, TWO_AXIS_FAMILY) && has(&again, "Source Sans 3") && has(&again, "JetBrains Mono"), "{again:?}");
}
