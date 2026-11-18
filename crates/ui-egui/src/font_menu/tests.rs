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
    // A database of the bundled fonts alone, each family starred, so each row draws its star:
    // fonts that tests running alongside add to the process-wide database stay out of the rows.
    let db = vectorcraft_text::FontDb::with_font_dirs(vec![]);
    let favorites: Vec<String> = ["Source Sans 3", "Source Serif 4", "Inter", "JetBrains Mono"].map(str::to_string).into();
    let ctx = egui::Context::default();
    // Under the search field and the filters, room for fewer rows than the four families.
    let area = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 160.0));
    let mut st = super::MenuState { favorites_only: true, ..Default::default() };
    let look = super::MenuLook { samples: false, favorites: &favorites, ..Default::default() };
    let (mut list, mut stars) = (None, vec![]);
    // Frames enough for the scroll bar to finish appearing.
    for _ in 0..30 {
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(area), ..Default::default() }, |ui| {
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area));
            let shown = super::list_of(&mut child, &db, egui::Id::unique("fonts"), &mut st, "Inter", None, look);
            list = Some((shown.inner_rect, shown.content_size));
        });
        out.textures_delta.clear();
        stars = out
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) if t.galley.text() == "★" => Some(t.visual_bounding_rect()),
                _ => None,
            })
            .collect();
    }
    let Some((inner, content)) = list else { panic!("the list was not drawn") };
    let families: Vec<&str> = st.rows.iter().filter_map(super::Row::family).collect();
    assert_eq!(families.len(), favorites.len(), "the starred families are listed: {families:?}");
    assert!(content.y > inner.height() + 0.5, "the rows, {} high, fill a list {} high", content.y, inner.height());
    // The rows' stars in the list, not the ★ filter above it.
    let stars: Vec<_> = stars.into_iter().filter(|s| inner.contains(s.center())).collect();
    assert!(!stars.is_empty(), "no row star drawn in the list at {inner:?}");
    let bar = super::widgets::list_scroll_style();
    let column = bar.bar_width + bar.bar_inner_margin + bar.bar_outer_margin;
    for star in stars {
        assert!(star.right() <= area.right() - column + 0.5, "a star at {} runs under the scroll bar", star.right());
    }
}

/// In a wide row the sample starts after the widest name and a gap, not halfway along, even for a
/// name wider than the menu widens itself for; the sample keeps its room, and a row too narrow for
/// both gives the name half of it. Without samples the name runs to the Show Similar Fonts button.
#[test]
fn the_sample_starts_after_the_widest_name() {
    use super::{DISCLOSURE, NAME_GAP, NAME_MAX, SAMPLE_W, SIMILAR, STAR, name_column};
    assert_eq!(name_column(500.0, 100.0, true), DISCLOSURE + 100.0 + NAME_GAP);
    assert_eq!(name_column(600.0, NAME_MAX + 80.0, true), DISCLOSURE + NAME_MAX + 80.0 + NAME_GAP);
    assert_eq!(name_column(600.0, 1000.0, true), 600.0 - SIMILAR - STAR - 2.0 - SAMPLE_W);
    assert_eq!(name_column(200.0, 100.0, true), 100.0);
    assert_eq!(name_column(500.0, 100.0, false), 500.0 - SIMILAR - STAR);
}

/// The widest label is the widest one laid out, though it has fewer characters than others, or
/// characters that are narrow on their own.
#[test]
fn the_widest_label_is_the_widest_laid_out() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut labels: Vec<String> = ["WWWWWWWW", "Mm", "", "ffi ffl fi fl", "Source Serif 4 Display Semibold"].map(str::to_string).into();
    labels.push("i".repeat(40));
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        let font = egui::FontId::proportional(super::NAME_SIZE);
        let laid_out = |l: &String| ui.painter().layout_no_wrap(l.clone(), font.clone(), egui::Color32::WHITE).size().x;
        let widest = labels.iter().map(laid_out).fold(0.0, f32::max);
        assert_eq!(super::widest_label(ui, labels.clone()), widest);
        for l in &labels {
            assert_eq!(super::widest_label(ui, vec![l.clone()]), laid_out(l), "{l}");
        }
        assert_eq!(super::widest_label(ui, vec![]), 0.0);
    });
    out.textures_delta.clear();
}

/// The widest family name is measured again when the interface's fonts change, as they do when
/// faces for another script are added to it, not only when the font list does.
#[test]
fn the_widest_name_is_measured_again_when_the_interface_fonts_change() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let frame = |f: &mut dyn FnMut(&mut egui::Ui)| {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| f(ui));
        out.textures_delta.clear();
    };
    // A width no name has, kept as if measured for the fonts as they are.
    frame(&mut |ui| {
        let key = super::family_name_key(ui, vectorcraft_text::FontDb::global(), true);
        ui.data_mut(|d| d.insert_temp(super::family_name_id(), (key, -1.0_f32)));
    });
    frame(&mut |ui| {
        let data = ui.ctx().fonts(|f| f.definitions().font_data.values().next().cloned()).expect("an interface font");
        ui.ctx().add_font(egui::epaint::text::FontInsert { name: "font-menu-test-face".into(), data: (*data).clone(), families: vec![] });
    });
    frame(&mut |ui| assert!(super::family_name_width(ui, vectorcraft_text::FontDb::global(), true) >= 0.0, "measured again"));
}

/// The widest family name measured for one font database is not taken for another one at the same
/// generation.
#[test]
fn each_font_database_has_its_own_widest_name() {
    use vectorcraft_doc::CompositeFont;
    let short = vectorcraft_text::FontDb::with_font_dirs(vec![]);
    let long = vectorcraft_text::FontDb::with_font_dirs(vec![]);
    assert!(short.set_composite_font(&CompositeFont::new("C", "Inter", "Regular")));
    assert!(long.set_composite_font(&CompositeFont::new("A composite font with a name longer than any family", "Inter", "Regular")));
    assert_eq!(short.generation(), long.generation());
    let ctx = egui::Context::default();
    let mut widths = (0.0, 0.0);
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        widths = (super::family_name_width(ui, &short, true), super::family_name_width(ui, &long, true));
    });
    out.textures_delta.clear();
    assert!(widths.1 > widths.0 + 1.0, "the long name's database measured {} and the other {}", widths.1, widths.0);
}

/// Without a row of filters drawn yet, the menu still makes room for the widest name, its sample,
/// the star and the scroll bar; without samples it is the sample's room narrower.
#[test]
fn the_menu_makes_room_for_the_widest_name_and_its_sample() {
    use super::{DISCLOSURE, NAME_GAP, NAME_MAX, SAMPLE_W, STAR};
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 900.0))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| {
        let st = super::MenuState::default();
        let look = super::MenuLook::default();
        let names = super::name_width(ui, vectorcraft_text::FontDb::global(), look.english_names, &st.rows).min(NAME_MAX);
        assert!(names > 0.0, "a family is listed");
        let width = super::menu_width(ui, &st, look);
        assert!(width >= DISCLOSURE + names + NAME_GAP + SAMPLE_W + STAR, "the menu is {width} wide for a name {names} wide");
        let plain = super::menu_width(ui, &st, super::MenuLook { samples: false, ..look });
        assert!((width - plain - SAMPLE_W).abs() < 0.01, "{width} with samples, {plain} without");
    });
    out.textures_delta.clear();
}

/// The style names of the families expanded in the listed rows count toward the names' width;
/// those of a family expanded earlier but not listed now do not.
#[test]
fn only_listed_style_names_widen_the_names() {
    use super::{Row, STYLE_INDENT};
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let long = format!("SemiCondensed ExtraLight Italic {}", "W".repeat(80));
    let db = vectorcraft_text::FontDb::global();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        let families = super::name_width(ui, db, true, &[]);
        let font = egui::FontId::proportional(super::NAME_SIZE);
        let style = STYLE_INDENT + ui.painter().layout_no_wrap(long.clone(), font, egui::Color32::WHITE).size().x;
        assert!(style > families, "the style name is wider than every family name");
        let listed = [Row::Family("A".into()), Row::Style("A".into(), long.clone())];
        assert_eq!(super::name_width(ui, db, true, &listed), style);
        assert!(super::name_width(ui, db, true, &listed[..1]) < style, "a style not listed is not measured");
    });
    out.textures_delta.clear();
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
    let order = super::wheel_order(&mut super::MenuState::default(), look);
    assert_eq!(order.first(), Some(&names[2]), "the recent family first");
    // Other tests add fonts to the process-wide database meanwhile: compare places, not neighbours.
    let at = |f: &String| order.iter().position(|x| x == f);
    assert!(matches!((at(&names[0]), at(&names[1])), (Some(a), Some(b)) if 0 < a && a < b), "then the others in the menu's order");
    assert_eq!(order.iter().filter(|f| **f == names[2]).count(), 1, "once each");
    let favorites = vec![names[0].clone(), names[1].clone()];
    let mut st = super::MenuState { favorites_only: true, ..Default::default() };
    let order = super::wheel_order(&mut st, super::MenuLook { favorites: &favorites, ..look });
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

/// The font menus of two panels (other ids, other scopes) share their filters: a filter chosen in
/// one menu is on when the other opens, and it lists its rows by it.
#[test]
fn a_filter_chosen_in_one_panels_menu_is_on_in_the_others() {
    use vectorcraft_text::FontClass;
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let fields = [("character", "character-font", 10.0), ("control-bar", "control-font", 60.0)];
    let field = |top: f32| egui::Rect::from_min_size(egui::pos2(10.0, top), egui::vec2(200.0, 30.0));
    // Draws both fields (and the open menu), and returns the texts drawn with their centers and
    // each menu's state.
    let frame = |events: Vec<egui::Event>| {
        let input =
            egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 900.0))), ..Default::default() };
        let mut ids = [egui::Id::NULL; 2];
        let mut out = ctx.run_ui(input, |ui| {
            for (i, (scope, id, top)) in fields.into_iter().enumerate() {
                ui.push_id(scope, |ui| {
                    ui.scope_builder(egui::UiBuilder::new().max_rect(field(top)), |ui| {
                        ids[i] = ui.scope_id().with(id).with("font-menu");
                        super::font_menu(ui, id, "Inter", 200.0, None, super::MenuLook { samples: false, ..Default::default() });
                    });
                });
            }
        });
        out.textures_delta.clear();
        let texts: Vec<(String, egui::Pos2)> = out
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) => Some((t.galley.text().to_string(), t.visual_bounding_rect().center())),
                _ => None,
            })
            .collect();
        let states = ids.map(|id| ctx.data(|d| d.get_temp::<super::MenuState>(id)).unwrap_or_default());
        (texts, states)
    };
    let press = |at: egui::Pos2| {
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(vec![egui::Event::PointerMoved(at), button(true)]);
        frame(vec![button(false)]);
        for _ in 0..3 {
            frame(vec![]);
        }
    };
    let open = |i: usize| press(field(fields[i].2).min + egui::vec2(10.0, 10.0));
    let close = || {
        let escape = [true, false].map(|pressed| egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Default::default(),
        });
        frame(escape.to_vec());
        for _ in 0..3 {
            frame(vec![]);
        }
    };
    // The topmost text `label`: a filter chip, above the rows.
    let chip = |label: &str| {
        let (texts, _) = frame(vec![]);
        texts
            .into_iter()
            .filter(|(t, _)| t == label)
            .map(|(_, at)| at)
            .min_by(|a, b| a.y.total_cmp(&b.y))
            .unwrap_or_else(|| panic!("{label} is drawn"))
    };
    let families = |st: &super::MenuState| st.rows.iter().filter(|r| matches!(r, Row::Family(_))).count();
    let filters = |st: &super::MenuState| (st.script, st.class, st.variable_only, st.favorites_only);
    frame(vec![]);
    // Favorites only, chosen in the first panel's menu, with no favorites: the second lists nothing.
    open(0);
    press(chip("★"));
    close();
    let (_, [first, second]) = frame(vec![]);
    assert!(first.favorites_only && second.favorites_only, "the second menu has the filter before it opens");
    open(1);
    let (_, [_, second]) = frame(vec![]);
    assert!(second.favorites_only);
    assert_eq!(families(&second), 0, "no favorites, so the second menu lists no family");
    // A script, a kind and Variable chosen in the second panel's menu are on in the first.
    press(chip(super::script_label(3)));
    press(chip(super::class_label(FontClass::Monospaced)));
    press(chip(super::variable_label()));
    close();
    open(0);
    let (_, [first, _]) = frame(vec![]);
    assert_eq!(filters(&first), (3, Some(FontClass::Monospaced), true, true));
    // Every filter, turned off in the first menu, is off in the second, which lists families again.
    press(chip("★"));
    press(chip(super::script_label(0)));
    press(chip(super::class_label(FontClass::Monospaced)));
    press(chip(super::variable_label()));
    close();
    open(1);
    let (_, [_, second]) = frame(vec![]);
    assert_eq!(filters(&second), (0, None, false, false));
    assert!(families(&second) > 0, "every family is listed again");
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

/// A Type › Font menu item's sample follows Show samples on white: dark type on a white slot,
/// or the interface's dimmed text color with nothing behind it. The slot is white while the
/// sample is still rendering too.
#[test]
fn a_menu_items_sample_follows_show_samples_on_white() {
    use crate::theme::{Brightness, Tokens};
    let dark = Tokens::for_brightness(Brightness::Light).text;
    for b in Brightness::ALL {
        let t = Tokens::for_brightness(b);
        for on_white in [false, true] {
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            ctx.data_mut(|d| d.insert_temp(egui::Id::NULL, t));
            let slot = egui::Rect::from_min_size(egui::pos2(40.0, 20.0), super::MENU_SAMPLE_SIZE);
            let frame = || {
                let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::menu_item_sample(ui, slot, "Inter", on_white));
                out.textures_delta.clear();
                out.shapes
            };
            frame();
            super::wait_for_samples(&ctx);
            let shapes = frame();
            let tints: Vec<_> = shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Mesh(m) if m.texture_id != egui::TextureId::default() => m.vertices.first().map(|v| v.color),
                    _ => None,
                })
                .collect();
            let papers =
                shapes.iter().filter(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill == super::SAMPLE_PAPER && r.rect == slot)).count();
            assert_eq!(tints, [if on_white { dark } else { t.text_dim }], "{}, on white {on_white}", b.id());
            assert_eq!(papers, usize::from(on_white), "{}, on white {on_white}", b.id());
        }
    }
    // While a family's sample is still rendering, its slot is already white.
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let slot = egui::Rect::from_min_size(egui::pos2(40.0, 20.0), super::MENU_SAMPLE_SIZE);
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::menu_item_sample(ui, slot, "JetBrains Mono", true));
    out.textures_delta.clear();
    let shapes = out.shapes;
    super::wait_for_samples(&ctx);
    assert!(!shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Mesh(m) if m.texture_id != egui::TextureId::default())), "no sample");
    assert!(shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill == super::SAMPLE_PAPER && r.rect == slot)), "the slot is white");
}

/// The center of the first text shape whose text passes `wanted`.
fn text_at(shapes: &[egui::epaint::ClippedShape], wanted: impl Fn(&str) -> bool) -> Option<egui::Pos2> {
    shapes.iter().find_map(|c| match &c.shape {
        egui::Shape::Text(t) if wanted(t.galley.text()) => Some(t.visual_bounding_rect().center()),
        _ => None,
    })
}

/// Show Similar Fonts: the ≈ button of a family's row orders the listed families by likeness to
/// it, that family first and highlighted, without the Recent Fonts section and without picking a
/// font; the ≈ chip under the filters puts the menu's order back, and so does opening the menu
/// again.
#[test]
fn show_similar_fonts_orders_the_list_by_likeness() {
    // The bundled fonts alone: fonts that tests running alongside add stay out of the rows.
    let db = vectorcraft_text::FontDb::with_font_dirs(vec![]);
    let recent = vec!["JetBrains Mono".to_string()];
    let ctx = egui::Context::default();
    let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 800.0));
    let look = super::MenuLook { samples: false, recent: &recent, ..Default::default() };
    let mut st = super::MenuState::default();
    let picks = std::cell::RefCell::new(vec![]);
    let mut frame = |events: Vec<egui::Event>, st: &mut super::MenuState| {
        let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: Some(area), ..Default::default() }, |ui| {
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area));
            picks.borrow_mut().extend(super::list_of(&mut child, &db, egui::Id::unique("similar"), st, "Inter", None, look).inner);
        });
        out.textures_delta.clear();
        out.shapes
    };
    let click = |frame: &mut dyn FnMut(Vec<egui::Event>, &mut super::MenuState) -> Vec<egui::epaint::ClippedShape>,
                 st: &mut super::MenuState,
                 pos: egui::Pos2| {
        let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(vec![egui::Event::PointerMoved(pos)], st);
        frame(vec![button(true)], st);
        frame(vec![button(false)], st);
        frame(vec![], st)
    };
    let families = |st: &super::MenuState| st.rows.iter().filter_map(super::Row::family).map(str::to_string).collect::<Vec<_>>();
    let row_of = |st: &super::MenuState, family: &str| st.rows.iter().position(|r| r.family() == Some(family) && r.style().is_none());
    let shapes = frame(vec![], &mut st);
    let menu_order = families(&st);
    assert_eq!(st.rows.first(), Some(&super::Row::Heading("Recent Fonts")));
    // A hovered row shows its ≈ button: here Source Serif 4's, not the current family's (Inter).
    let name = text_at(&shapes, |t| t == "Source Serif 4").expect("Source Serif 4 is listed");
    let shapes = frame(vec![egui::Event::PointerMoved(name)], &mut st);
    let button = shapes
        .iter()
        .find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text() == "≈" && (t.visual_bounding_rect().center().y - name.y).abs() < 2.0 => {
                Some(t.visual_bounding_rect().center())
            }
            _ => None,
        })
        .expect("the hovered row shows ≈");
    let shapes = click(&mut frame, &mut st, button);
    assert_eq!(st.similar.as_deref(), Some("Source Serif 4"));
    assert_eq!(families(&st), db.similar_families("Source Serif 4", &db.menu_family_list()));
    assert_eq!(families(&st).first().map(String::as_str), Some("Source Serif 4"));
    assert!(!st.rows.iter().any(|r| matches!(r, super::Row::Heading(_))), "no Recent Fonts section: {:?}", st.rows);
    assert_eq!(st.highlight, Some(0));
    assert!(!picks.borrow().iter().any(|p| matches!(p, super::FontPick::Chosen(..))), "no font picked: {:?}", picks.borrow());
    // The chip under the filters clears it, and the family it ordered by stays highlighted.
    let chip = text_at(&shapes, |t| t.starts_with("≈ Source Serif 4")).expect("the chip names the family");
    click(&mut frame, &mut st, chip);
    assert_eq!(st.similar, None);
    assert_eq!(families(&st), menu_order);
    assert_eq!(st.highlight, row_of(&st, "Source Serif 4"));
    // Opening the menu again lists in the menu's order too.
    st.similar = Some("Inter".into());
    st.last = 0;
    frame(vec![], &mut st);
    assert_eq!((st.similar.as_deref(), families(&st)), (None, menu_order));
}

/// An app with a new document holding type set in each of `fonts`, one text object each.
fn app_with_type(fonts: &[&str]) -> crate::VectorcraftApp {
    let mut app = crate::VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", serde_json::json!({})).unwrap();
    for (i, font) in fonts.iter().enumerate() {
        app.run("text.create", serde_json::json!({"x": 20, "y": 40 + 30 * i, "text": "Type", "font": font})).unwrap();
    }
    app
}

/// Draws the open menu (its header and rows) for `look` under the filters of `st`, with `events`;
/// returns the shapes drawn.
fn menu_frame(ctx: &egui::Context, st: &mut super::MenuState, look: super::MenuLook, events: Vec<egui::Event>) -> Vec<egui::epaint::ClippedShape> {
    let input =
        egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 900.0))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| {
        super::list(ui, egui::Id::unique("document-filter"), st, "Inter", None, look);
    });
    out.textures_delta.clear();
    out.shapes
}

/// The families `st`'s rows list, in their order.
fn listed_families(st: &super::MenuState) -> Vec<String> {
    st.rows
        .iter()
        .filter_map(|r| match r {
            Row::Family(f) => Some(f.clone()),
            Row::Heading(_) | Row::Style(..) => None,
        })
        .collect()
}

/// The Document filter's chip lists only the families the document's type uses (hidden and locked
/// type included, a missing family left out), in the menu's order, and lists every family again
/// when it is clicked off.
#[test]
fn the_document_filter_lists_the_families_the_documents_type_uses() {
    let mut app = app_with_type(&["Source Serif 4", "Inter"]);
    app.run("object.hide", serde_json::json!({})).unwrap();
    app.run("text.create", serde_json::json!({"x": 20, "y": 120, "text": "Locked", "font": "Source Sans 3"})).unwrap();
    app.run("object.lock", serde_json::json!({})).unwrap();
    app.run("text.create", serde_json::json!({"x": 20, "y": 150, "text": "Missing", "font": "Missing Family 7059"})).unwrap();
    let db = vectorcraft_text::FontDb::global();
    let used = super::document_families(db, &app.session.active().unwrap().doc);
    let mut names: Vec<_> = used.iter().cloned().collect();
    names.sort();
    assert_eq!(names, ["inter", "missing family 7059", "source sans 3", "source serif 4"], "each run's family, as named");
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut st = super::MenuState::default();
    let look = super::MenuLook { samples: false, ..super::MenuLook::of(&app) };
    for _ in 0..3 {
        menu_frame(&ctx, &mut st, look, vec![]);
    }
    let every = listed_families(&st);
    assert!(every.len() > 3, "every family: {every:?}");
    let click = |st: &mut super::MenuState, at: egui::Pos2| {
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        menu_frame(&ctx, st, look, vec![egui::Event::PointerMoved(at), button(true)]);
        menu_frame(&ctx, st, look, vec![button(false)]);
        menu_frame(&ctx, st, look, vec![])
    };
    let chip = text_at(&menu_frame(&ctx, &mut st, look, vec![]), |t| t == "Document").expect("the Document chip is drawn");
    click(&mut st, chip);
    assert!(st.document_only);
    let in_menu: Vec<String> = every.iter().filter(|f| ["Inter", "Source Sans 3", "Source Serif 4"].contains(&f.as_str())).cloned().collect();
    assert_eq!(listed_families(&st), in_menu, "the document's available families, in the menu's order");
    click(&mut st, chip);
    assert!(!st.document_only);
    assert_eq!(listed_families(&st), every, "every family again");
}

/// Type kept in a graph design counts as the document's type, as type in a symbol does.
#[test]
fn the_document_filter_counts_type_in_graph_designs() {
    let mut app = app_with_type(&["Inter"]);
    let id = app.run("text.create", serde_json::json!({"x": 20, "y": 90, "text": "Bar", "font": "JetBrains Mono"})).unwrap()["id"].as_u64().unwrap();
    let art = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).cloned().expect("the new type");
    app.run("edit.undo", serde_json::json!({})).unwrap();
    let mut doc = (*app.session.active().unwrap().doc).clone();
    let db = vectorcraft_text::FontDb::global();
    assert!(!super::document_families(db, &doc).contains("jetbrains mono"));
    doc.graph_designs.push(vectorcraft_doc::GraphDesign { name: "Bars".into(), art: std::sync::Arc::new(art) });
    let mut names: Vec<_> = super::document_families(db, &doc).into_iter().collect();
    names.sort();
    assert_eq!(names, ["inter", "jetbrains mono"]);
}

/// Type in the art of a stored scatter brush counts as the document's type, as a scatter stroke
/// draws it, after a malformed library entry too. Type in an art or pattern brush's art does not,
/// as those brushes leave type out when they bend their art.
#[test]
fn the_document_filter_counts_type_in_scatter_brushes() {
    let mut app = app_with_type(&["Inter"]);
    let mut art = |font: &str| {
        let id = app.run("text.create", serde_json::json!({"x": 20, "y": 90, "text": "Dot", "font": font})).unwrap()["id"].as_u64().unwrap();
        let node = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).cloned().expect("the new type");
        app.run("edit.undo", serde_json::json!({})).unwrap();
        node
    };
    let (scatter, bent, tiled) = (art("JetBrains Mono"), art("Source Serif 4"), art("Source Sans 3"));
    let brush = |name: &str, kind| serde_json::to_value(vectorcraft_brush::Brush { name: name.into(), kind }).unwrap();
    let library = serde_json::json!([
        {"name": "Broken", "type": "scatter", "art": 7},
        brush("Art", vectorcraft_brush::BrushKind::Art(vectorcraft_brush::ArtBrush { art: bent, ..Default::default() })),
        brush("Pattern", vectorcraft_brush::BrushKind::Pattern(vectorcraft_brush::PatternBrush { side: tiled, ..Default::default() })),
        brush("Dots", vectorcraft_brush::BrushKind::Scatter(vectorcraft_brush::Scatter { art: scatter, ..Default::default() })),
    ]);
    let mut doc = (*app.session.active().unwrap().doc).clone();
    doc.unknown.insert(vectorcraft_brush::DOC_KEY.into(), library);
    let db = vectorcraft_text::FontDb::global();
    let mut names: Vec<_> = super::document_families(db, &doc).into_iter().collect();
    names.sort();
    assert_eq!(names, ["inter", "jetbrains mono"]);
}

/// A scatter brush made of type keeps its family in the Document filter after the type is deleted;
/// deleting the brush takes the family out, and undoing the deletion brings it back.
#[test]
fn the_document_filter_follows_a_scatter_brushs_edits_and_undo() {
    let mut app = app_with_type(&["Inter"]);
    let id = app.run("text.create", serde_json::json!({"x": 20, "y": 90, "text": "Dot", "font": "JetBrains Mono"})).unwrap()["id"].clone();
    let name = app.run("brush.new", serde_json::json!({"type": "scatter", "name": "Type Dots"})).unwrap()["name"].clone();
    app.run("edit.clear", serde_json::json!({"ids": [id]})).unwrap();
    let db = vectorcraft_text::FontDb::global();
    // One menu state throughout, so each edit has to get past the families it keeps.
    let mut st = super::MenuState { document_only: true, ..Default::default() };
    let mut families = |app: &crate::VectorcraftApp| {
        let used = super::used_families(db, &mut st, super::MenuLook::of(app)).expect("a document is open");
        let mut names: Vec<_> = used.iter().cloned().collect();
        names.sort();
        names
    };
    assert_eq!(families(&app), ["inter", "jetbrains mono"], "the brush's type");
    app.run("brush.delete", serde_json::json!({"name": name})).unwrap();
    assert_eq!(families(&app), ["inter"], "the brush deleted");
    app.run("edit.undo", serde_json::json!({})).unwrap();
    assert_eq!(families(&app), ["inter", "jetbrains mono"], "the deletion undone");
}

/// A run whose family and style were split from a PostScript name (as an importer without the font
/// splits one) counts under the family of the installed font of that name, found with its style.
#[test]
fn the_document_filter_finds_a_postscript_name_split_into_family_and_style() {
    let mut app = app_with_type(&[]);
    let id = app.run("text.create", serde_json::json!({"x": 20, "y": 90, "text": "Split", "font": "Inter"})).unwrap()["id"].as_u64().unwrap();
    let mut art = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).cloned().expect("the new type");
    app.run("edit.undo", serde_json::json!({})).unwrap();
    let vectorcraft_doc::NodeKind::Text(t) = &mut art.kind else { panic!("type") };
    for r in &mut t.runs {
        // SourceSans3-Semibold, split after "Semi".
        r.style.font_family = "SourceSans3Semi".into();
        r.style.font_style = "bold".into();
    }
    let mut doc = (*app.session.active().unwrap().doc).clone();
    doc.graph_designs.push(vectorcraft_doc::GraphDesign { name: "Split".into(), art: std::sync::Arc::new(art) });
    let db = vectorcraft_text::FontDb::global();
    let names: Vec<_> = super::document_families(db, &doc).into_iter().collect();
    assert_eq!(names, ["source sans 3"]);
}

/// The Document filter combines with the others and with Recent Fonts, follows edits and undo
/// without the menu reopening, and steers the closed field's wheel.
#[test]
fn the_document_filter_combines_with_the_others_and_follows_edits() {
    let mut app = app_with_type(&["Inter", "Source Serif 4"]);
    let ctx = egui::Context::default();
    let mut st = super::MenuState { document_only: true, favorites_only: true, ..Default::default() };
    let favorites = vec!["Inter".to_string(), "Source Sans 3".to_string()];
    let recent = vec!["Source Sans 3".to_string(), "Source Serif 4".to_string()];
    let frames = |app: &crate::VectorcraftApp, st: &mut super::MenuState| {
        let look = super::MenuLook { samples: false, favorites: &favorites, recent: &recent, ..super::MenuLook::of(app) };
        for _ in 0..2 {
            menu_frame(&ctx, st, look, vec![]);
        }
    };
    frames(&app, &mut st);
    assert_eq!(listed_families(&st), ["Inter"], "favorites the document uses");
    st.favorites_only = false;
    frames(&app, &mut st);
    assert_eq!(st.rows.first(), Some(&Row::Heading("Recent Fonts")));
    assert_eq!(listed_families(&st), ["Source Serif 4", "Inter"], "only the recent family the document uses comes first");
    app.run("text.create", serde_json::json!({"x": 20, "y": 120, "text": "New", "font": "Source Sans 3"})).unwrap();
    frames(&app, &mut st);
    assert_eq!(listed_families(&st), ["Source Sans 3", "Source Serif 4", "Inter"], "an edit lists the new family");
    app.run("edit.undo", serde_json::json!({})).unwrap();
    frames(&app, &mut st);
    assert_eq!(listed_families(&st), ["Source Serif 4", "Inter"], "undo takes it out again");
    let look = super::MenuLook { samples: false, recent: &recent, ..super::MenuLook::of(&app) };
    assert_eq!(super::wheel_order(&mut st, look), ["Source Serif 4", "Inter"], "the wheel steps through the same families");
}

/// A font previewed from the menu does not change what the Document filter lists: the family the
/// previewed text had stays listed, though no other type uses it. A pick lists the families again.
#[test]
fn a_preview_keeps_the_document_filters_families() {
    let mut app = app_with_type(&["Source Serif 4", "Inter"]);
    let ctx = egui::Context::default();
    let mut st = super::MenuState { document_only: true, ..Default::default() };
    let frames = |app: &crate::VectorcraftApp, st: &mut super::MenuState| {
        let look = super::MenuLook { samples: false, recent: &[], ..super::MenuLook::of(app) };
        for _ in 0..2 {
            menu_frame(&ctx, st, look, vec![]);
        }
    };
    frames(&app, &mut st);
    assert_eq!(listed_families(&st), ["Inter", "Source Serif 4"]);
    // The selected text, the only type in Inter, previewed in Source Serif 4.
    super::apply(&mut app, &ctx, super::FontPick::Preview("Source Serif 4".into(), None));
    assert!(app.session.active().unwrap().interaction.is_some(), "the preview is open");
    frames(&app, &mut st);
    assert_eq!(listed_families(&st), ["Inter", "Source Serif 4"], "the preview leaves the list as it was");
    super::apply(&mut app, &ctx, super::FontPick::Preview("Inter".into(), None));
    frames(&app, &mut st);
    assert_eq!(listed_families(&st), ["Inter", "Source Serif 4"], "previewing back changes nothing");
    super::apply(&mut app, &ctx, super::FontPick::Chosen("Source Serif 4".into(), None));
    frames(&app, &mut st);
    assert_eq!(listed_families(&st), ["Source Serif 4"], "the pick is an edit: Inter is no longer used");
    app.run("edit.undo", serde_json::json!({})).unwrap();
    frames(&app, &mut st);
    assert_eq!(listed_families(&st), ["Inter", "Source Serif 4"], "undo lists it again");
}

/// The Document filter on a document without type lists nothing and says why; without a document
/// it is unavailable and lists every family; the filter is shared like the others.
#[test]
fn the_document_filter_on_an_empty_document_or_none() {
    let app = app_with_type(&[]);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut st = super::MenuState { document_only: true, ..Default::default() };
    let look = super::MenuLook { samples: false, ..super::MenuLook::of(&app) };
    let mut shapes = vec![];
    for _ in 0..3 {
        shapes = menu_frame(&ctx, &mut st, look, vec![]);
    }
    assert!(st.rows.is_empty(), "{:?}", st.rows);
    assert!(text_at(&shapes, |t| t == "This document uses no available fonts").is_some(), "the filter says the list is empty because of it");
    // A search that matches nothing still says so in its own words.
    st.document_only = false;
    shapes = menu_frame(&ctx, &mut st, look, vec![egui::Event::Text("NoSuchFontFor7059".into())]);
    shapes = if st.rows.is_empty() { menu_frame(&ctx, &mut st, look, vec![]) } else { shapes };
    assert!(text_at(&shapes, |t| t == "No matching fonts").is_some());
    // No document: the filter, kept on, lists every family.
    let mut st = super::MenuState { document_only: true, ..Default::default() };
    let none = egui::Context::default();
    for _ in 0..3 {
        menu_frame(&none, &mut st, super::MenuLook { samples: false, ..Default::default() }, vec![]);
    }
    assert!(st.document_only && !listed_families(&st).is_empty(), "no document: every family");
    let mut other = super::MenuState::default();
    super::Filters::of(&st).apply(&mut other);
    assert!(other.document_only, "every font menu shares the filter");
}

/// A fixed time the Recently Added tests count from.
fn clock() -> std::time::SystemTime {
    std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_800_000_000)
}

const DAY: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// The Recently Added filter's list of `families` (newest first), as found in `db` at [`clock`],
/// to be found again from `recheck_at` on.
fn added_in(db: &vectorcraft_text::FontDb, families: &[&str], recheck_at: Option<std::time::SystemTime>) -> Option<super::RecentlyAddedFonts> {
    let list = vectorcraft_text::RecentlyAdded { families: families.iter().map(|f| f.to_string()).collect(), recheck_at };
    // The font folders' first scan changes the generation: it runs before the generation is read.
    db.family_list();
    Some(((db.id(), db.generation()), clock(), std::sync::Arc::new(super::AddedFonts::of(&list))))
}

/// Draws the open menu of the families in `db` twice for `look` under the filters of `st`, with
/// `events` in the first pass; returns the shapes of the second.
fn settle_menu_of(
    ctx: &egui::Context,
    db: &vectorcraft_text::FontDb,
    st: &mut super::MenuState,
    look: super::MenuLook,
    events: Vec<egui::Event>,
) -> Vec<egui::epaint::ClippedShape> {
    let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 900.0));
    let mut shapes = vec![];
    for events in [events, vec![]] {
        let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: Some(area), ..Default::default() }, |ui| {
            let _ = super::list_of(ui, db, egui::Id::unique("recently-added"), st, "Inter", None, look);
        });
        out.textures_delta.clear();
        shapes = out.shapes;
    }
    shapes
}

/// The Recently Added filter lists only the families added recently, the newest first and without
/// the Recent Fonts section, the recent family among them; it combines with the ★ filter and the
/// search, Show Similar Fonts orders what it lists, and the closed field's wheel follows its order.
#[test]
fn the_recently_added_filter_lists_new_families_newest_first() {
    // The bundled fonts alone: fonts that tests running alongside add stay out of the rows.
    let db = vectorcraft_text::FontDb::with_font_dirs(vec![]);
    let ctx = egui::Context::default();
    let recent = vec!["Inter".to_string()];
    let favorites = vec!["Inter".to_string(), "Source Serif 4".to_string()];
    let look = super::MenuLook { samples: false, recent: &recent, clock: Some(clock()), ..Default::default() };
    let newest_first = ["Source Serif 4", "JetBrains Mono", "Inter"];
    let mut st = super::MenuState { added_only: true, added: added_in(&db, &newest_first, None), ..Default::default() };
    settle_menu_of(&ctx, &db, &mut st, look, vec![]);
    assert_eq!(listed_families(&st), newest_first, "newest first, the recent family among them");
    assert!(!st.rows.iter().any(|r| matches!(r, Row::Heading(_))), "no Recent Fonts section: {:?}", st.rows);
    assert_eq!(super::wheel_order_in(&db, &mut st, look), newest_first, "the wheel steps through the same families");
    st.favorites_only = true;
    settle_menu_of(&ctx, &db, &mut st, super::MenuLook { favorites: &favorites, ..look }, vec![]);
    assert_eq!(listed_families(&st), ["Source Serif 4", "Inter"], "only the favorites among them");
    st.favorites_only = false;
    st.similar = Some("Inter".into());
    settle_menu_of(&ctx, &db, &mut st, look, vec![]);
    let listed: Vec<String> = newest_first.iter().map(|f| f.to_string()).collect();
    assert_eq!(listed_families(&st), db.similar_families("Inter", &listed), "Show Similar Fonts orders only these");
    st.similar = None;
    settle_menu_of(&ctx, &db, &mut st, look, vec![]);
    assert_eq!(listed_families(&st), newest_first, "newest first again once it is cleared");
    settle_menu_of(&ctx, &db, &mut st, look, vec![egui::Event::Text("mono".into())]);
    assert_eq!(listed_families(&st), ["JetBrains Mono"], "the search narrows the list");
}

/// A family leaves the list when its 30 days end, while the menu is open; with none left the
/// filter stays on and the menu says nothing matches. Its chip turns it off, and every font menu
/// shares it.
#[test]
fn the_recently_added_filter_drops_families_as_their_30_days_end() {
    let db = vectorcraft_text::FontDb::with_font_dirs(vec![]);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let look = super::MenuLook { samples: false, clock: Some(clock()), ..Default::default() };
    let mut st = super::MenuState { added_only: true, added: added_in(&db, &["Inter"], Some(clock() + DAY)), ..Default::default() };
    settle_menu_of(&ctx, &db, &mut st, look, vec![]);
    assert_eq!(listed_families(&st), ["Inter"]);
    let mut other = super::MenuState::default();
    super::Filters::of(&st).apply(&mut other);
    assert!(other.added_only, "every font menu shares the filter");
    let later = super::MenuLook { clock: Some(clock() + DAY), ..look };
    let shapes = settle_menu_of(&ctx, &db, &mut st, later, vec![]);
    assert!(st.rows.is_empty(), "{:?}", st.rows);
    assert!(st.added_only, "the filter stays on");
    assert!(text_at(&shapes, |t| t == "No matching fonts").is_some(), "the menu says nothing matches");
    let chip = text_at(&shapes, |t| t == "Recently Added").expect("the Recently Added chip is drawn");
    let button = |pressed| egui::Event::PointerButton { pos: chip, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    settle_menu_of(&ctx, &db, &mut st, later, vec![egui::Event::PointerMoved(chip), button(true)]);
    settle_menu_of(&ctx, &db, &mut st, later, vec![button(false)]);
    assert!(!st.added_only, "the chip turns the filter off");
    assert!(listed_families(&st).len() > 1, "every family: {:?}", st.rows);
}

/// When the clock goes back before the time the list was found for (the system clock is set back),
/// the filter finds its families again rather than keep the list it found.
#[test]
fn the_recently_added_filter_finds_its_families_again_when_the_clock_goes_back() {
    // The bundled fonts alone, which are never recently added.
    let db = vectorcraft_text::FontDb::with_font_dirs(vec![]);
    let ctx = egui::Context::default();
    let mut st = super::MenuState { added_only: true, added: added_in(&db, &["Inter"], None), ..Default::default() };
    settle_menu_of(&ctx, &db, &mut st, super::MenuLook { samples: false, clock: Some(clock()), ..Default::default() }, vec![]);
    assert_eq!(listed_families(&st), ["Inter"]);
    settle_menu_of(&ctx, &db, &mut st, super::MenuLook { samples: false, clock: Some(clock() - DAY), ..Default::default() }, vec![]);
    assert!(st.rows.is_empty(), "found again a day earlier: {:?}", st.rows);
}

/// A font file installed just now is recently added by its creation time, and no longer 31 days
/// later. Where the file system keeps no creation time, the filter lists nothing.
#[test]
fn the_recently_added_filter_lists_a_font_installed_just_now() {
    let dir = std::env::temp_dir().join(format!("vc-font-menu-recently-added-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("SourceSerif4-Regular.ttf");
    std::fs::copy(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts/SourceSerif4-Regular.ttf"), &file).unwrap();
    let dated = std::fs::metadata(&file).and_then(|m| m.created()).is_ok();
    let db = vectorcraft_text::FontDb::with_font_dirs(vec![dir.clone()]);
    let ctx = egui::Context::default();
    let now = std::time::SystemTime::now();
    let mut st = super::MenuState { added_only: true, ..Default::default() };
    settle_menu_of(&ctx, &db, &mut st, super::MenuLook { samples: false, clock: Some(now), ..Default::default() }, vec![]);
    let expected: &[&str] = if dated { &["Source Serif 4"] } else { &[] };
    assert_eq!(listed_families(&st), expected);
    settle_menu_of(&ctx, &db, &mut st, super::MenuLook { samples: false, clock: Some(now + DAY * 31), ..Default::default() }, vec![]);
    assert!(st.rows.is_empty(), "31 days later: {:?}", st.rows);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// While the samples are shown, a row under the filters offers Font Preview Size's Small, Medium
/// and Large at every size of the rows: a click on one asks for that size ([`super::apply_preview_size`]
/// leaves the rows' own size as it is). Without samples the row is not drawn.
#[test]
fn the_preview_size_row_asks_for_the_size_clicked() {
    // The bundled fonts alone: fonts that tests running alongside add stay out of the rows.
    let db = vectorcraft_text::FontDb::with_font_dirs(vec![]);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut st = super::MenuState::default();
    for size in ["small", "medium", "large"] {
        let look = super::MenuLook { row: super::preview_row(size), ..Default::default() };
        let shapes = settle_menu_of(&ctx, &db, &mut st, look, vec![]);
        assert!(text_at(&shapes, |t| t == "Font Preview Size").is_some(), "the row is drawn at {size}");
        let mut click = |label: &str| {
            let shapes = settle_menu_of(&ctx, &db, &mut st, look, vec![]);
            let at = text_at(&shapes, |t| t == label).unwrap_or_else(|| panic!("{label} is drawn at {size}"));
            let button =
                |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            settle_menu_of(&ctx, &db, &mut st, look, vec![egui::Event::PointerMoved(at), button(true)]);
            settle_menu_of(&ctx, &db, &mut st, look, vec![button(false)]);
            ctx.data_mut(|d| d.remove_temp::<&'static str>(super::preview_size_id()))
        };
        assert_eq!(click("Large"), Some("large"), "at {size}");
        assert_eq!(click("Small"), Some("small"), "at {size}");
        assert_eq!(click("Medium"), Some("medium"), "at {size}");
    }
    let look = super::MenuLook { row: super::preview_row("medium"), ..Default::default() };
    let shapes = settle_menu_of(&ctx, &db, &mut st, super::MenuLook { samples: false, ..look }, vec![]);
    assert!(text_at(&shapes, |t| t == "Font Preview Size").is_none(), "without samples the row is not drawn");
    super::wait_for_samples(&ctx);
}

/// Enter on a focused size button asks for that size and picks no font: the key is not also read as
/// the list's Enter, which would apply the highlighted font and close the menu.
#[test]
fn enter_on_a_focused_size_button_picks_no_font() {
    let db = vectorcraft_text::FontDb::with_font_dirs(vec![]);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    // The Recently Added filter gives the list rows from the bundled fonts alone.
    let mut st = super::MenuState { added_only: true, added: added_in(&db, &["Source Serif 4", "Inter"], None), ..Default::default() };
    let id = egui::Id::unique("size-enter");
    let look = super::MenuLook { row: super::preview_row("small"), clock: Some(clock()), ..Default::default() };
    let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 900.0));
    let frame = |st: &mut super::MenuState, events: Vec<egui::Event>| {
        let mut pick = None;
        let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: Some(area), ..Default::default() }, |ui| {
            pick = super::list_of(ui, &db, id, st, "Inter", None, look).inner;
        });
        out.textures_delta.clear();
        (out.shapes, pick)
    };
    frame(&mut st, vec![]);
    let (shapes, _) = frame(&mut st, vec![]);
    let at = text_at(&shapes, |t| t == "Large").expect("Large is drawn");
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut st, vec![egui::Event::PointerMoved(at), button(true)]);
    frame(&mut st, vec![button(false)]);
    ctx.data_mut(|d| d.remove_temp::<&'static str>(super::preview_size_id()));
    let enter = |pressed| egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: Some(egui::Key::Enter),
        pressed,
        repeat: false,
        modifiers: Default::default(),
    };
    let tab =
        |pressed| egui::Event::Key { key: egui::Key::Tab, physical_key: Some(egui::Key::Tab), pressed, repeat: false, modifiers: Default::default() };
    assert!(!listed_families(&st).is_empty(), "the list has rows an Enter could pick");
    // Tab moves the focus along the controls until it is on the Large button, where Enter is pressed.
    let mut found = None;
    for _ in 0..60 {
        frame(&mut st, vec![tab(true)]);
        frame(&mut st, vec![tab(false)]);
        let on_large = ctx.memory(|m| m.focused()).and_then(|f| ctx.read_response(f)).is_some_and(|r| r.rect.contains(at));
        if !on_large {
            continue;
        }
        let (_, pick) = frame(&mut st, vec![enter(true)]);
        frame(&mut st, vec![enter(false)]);
        if ctx.data_mut(|d| d.remove_temp::<&'static str>(super::preview_size_id())) == Some("large") {
            found = Some(pick);
            break;
        }
    }
    let pick = found.expect("Enter on the focused Large button asks for the size");
    assert!(pick.is_none(), "Enter on the size button picks no font: {pick:?}");
    super::wait_for_samples(&ctx);
}

/// The size a font menu's button asked for becomes Font Preview Size, which sizes every font
/// menu's rows, and the host is asked to write the preferences file. The document is left as it
/// was: a font previewed on the selected type stays previewed, and the change adds no undo step.
#[test]
fn a_preview_size_asked_for_sets_the_preference_and_keeps_the_preview() {
    let ctx = egui::Context::default();
    let mut app = app_with_type(&[]);
    let id = app.run("text.create", serde_json::json!({"x": 20, "y": 90, "text": "Size", "font": "Inter"})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", serde_json::json!({"ids": [id]})).unwrap();
    let font = |app: &crate::VectorcraftApp| match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
        vectorcraft_doc::NodeKind::Text(t) => t.first_style().font_family,
        _ => String::new(),
    };
    super::apply(&mut app, &ctx, super::FontPick::Preview("Source Serif 4".into(), None));
    assert_eq!(font(&app), "Source Serif 4");
    assert_eq!(super::MenuLook::of(&app).row, super::preview_row("medium"));
    assert!(!app.take_prefs_changed(), "nothing to write yet");
    let undo_depth = |app: &crate::VectorcraftApp| app.session.doc().unwrap().history.undo.len();
    let depth = undo_depth(&app);
    // The rows' own size: nothing changes.
    ctx.data_mut(|d| d.insert_temp(super::preview_size_id(), "medium"));
    super::apply_preview_size(&mut app, &ctx);
    assert_eq!(app.session.prefs.font_preview_size, "medium");
    assert!(!app.take_prefs_changed(), "nothing to write for the size the rows have");
    ctx.data_mut(|d| d.insert_temp(super::preview_size_id(), "large"));
    super::apply_preview_size(&mut app, &ctx);
    assert_eq!(app.session.prefs.font_preview_size, "large");
    assert_eq!(undo_depth(&app), depth, "the size is not an undo step");
    assert_eq!(super::MenuLook::of(&app).row, super::preview_row("large"));
    assert!(app.take_prefs_changed(), "the host is asked to write the preferences file");
    assert_eq!(app.ui.engine_prefs["fontPreviewSize"], "large");
    assert!(app.session.in_interaction(), "the preview is still open");
    assert_eq!(font(&app), "Source Serif 4", "the preview stays on the type");
    // Asked for once, applied once.
    super::apply_preview_size(&mut app, &ctx);
    assert_eq!(app.session.prefs.font_preview_size, "large");
    assert!(!app.take_prefs_changed(), "nothing more to write");
    // A saved value that is none of the sizes draws Medium's rows, and Medium replaces it.
    app.session.prefs.font_preview_size = "huge".into();
    assert_eq!(super::MenuLook::of(&app).row, super::preview_row("medium"));
    ctx.data_mut(|d| d.insert_temp(super::preview_size_id(), "medium"));
    super::apply_preview_size(&mut app, &ctx);
    assert_eq!(app.session.prefs.font_preview_size, "medium");
    assert!(app.take_prefs_changed(), "the replaced value is written");
    super::apply(&mut app, &ctx, super::FontPick::EndPreview);
    assert_eq!(font(&app), "Inter");
    app.run("edit.undo", serde_json::json!({})).unwrap();
    assert!(
        app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).is_none(),
        "undo takes the type away: no step was added for the size"
    );
}

/// Screen readers find Small, Medium and Large as radio buttons in a group named Font Preview
/// Size, with the size of the rows chosen.
#[test]
fn the_preview_size_row_is_a_radio_group_showing_the_size() {
    use crate::widgets::accessibility_tests::{named, role_and_state, tree};
    use egui::accesskit::{Role, Toggled};
    // The bundled fonts alone: fonts that tests running alongside add stay out of the rows.
    let db = vectorcraft_text::FontDb::with_font_dirs(vec![]);
    let sizes = ["small", "medium", "large"];
    for size in sizes {
        let mut st = super::MenuState::default();
        let look = super::MenuLook { row: super::preview_row(size), ..Default::default() };
        let nodes = tree(|ui| {
            let _ = super::list_of(ui, &db, egui::Id::unique("size-group"), &mut st, "Inter", None, look);
        });
        assert_eq!(named(&nodes, "Font Preview Size").role(), Role::RadioGroup, "at {size}");
        for other in sizes {
            let chosen = if other == size { Toggled::True } else { Toggled::False };
            assert_eq!(role_and_state(&nodes, super::preview_size_label(other)), (Role::RadioButton, Some(chosen)), "{other} at {size}");
        }
    }
}

/// A new Font Preview Size chosen in the open menu keeps the list on the rows it showed: the
/// view starts the same number of rows down at the new height, not the same number of points.
#[test]
fn a_new_preview_size_keeps_the_list_on_its_rows() {
    // The bundled fonts alone: fonts that tests running alongside add stay out of the rows.
    let db = vectorcraft_text::FontDb::with_font_dirs(vec![]);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut st = super::MenuState::default();
    // One frame `height` points high at `size`: the list's offset, top, view height and content height.
    let frame = |height: f32, size: &str, st: &mut super::MenuState| {
        let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, height));
        let look = super::MenuLook { row: super::preview_row(size), ..Default::default() };
        let mut shown = None;
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(area), ..Default::default() }, |ui| {
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area));
            // Opening on the last family scrolls the list to its end.
            let list = super::list_of(&mut child, &db, egui::Id::unique("keep-rows"), st, "Source Serif 4", None, look);
            shown = Some((list.state.clamped_offset().y, list.inner_rect.top(), list.inner_rect.height(), list.content_size.y));
        });
        out.textures_delta.clear();
        shown.unwrap_or_else(|| panic!("the list was not drawn"))
    };
    let (_, top, _, _) = frame(900.0, "medium", &mut st);
    // A fresh opening, with room for fewer rows than the list holds.
    let mut st = super::MenuState::default();
    let mut medium = (0.0, 0.0, 0.0, 0.0);
    for _ in 0..4 {
        medium = frame(top + 64.0, "medium", &mut st);
    }
    let (offset, _, view, content) = medium;
    assert!(content > view + 0.5 && offset > 0.5, "the list is scrolled: {medium:?}");
    let rows_down = offset / super::preview_row("medium");
    let mut large = (0.0, 0.0, 0.0, 0.0);
    for _ in 0..3 {
        large = frame(top + 64.0, "large", &mut st);
    }
    let expected = rows_down * super::preview_row("large");
    assert!((large.0 - expected).abs() < 0.5, "the view starts {rows_down} rows down: offset {} for {expected}", large.0);
    super::wait_for_samples(&ctx);
}
