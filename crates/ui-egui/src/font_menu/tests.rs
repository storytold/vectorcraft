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
    // Every family starred, so each row draws its star.
    let all = vectorcraft_text::FontDb::global().families();
    let families = all.len();
    let ctx = egui::Context::default();
    let area = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 160.0));
    let mut st = super::MenuState::default();
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
    let names = vectorcraft_text::FontDb::global().menu_family_list();
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
    let names = vectorcraft_text::FontDb::global().menu_family_list();
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
    let names = vectorcraft_text::FontDb::global().menu_family_list();
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
    let again = click(&mut st, &favorites);
    assert!(!st.variable_only);
    assert_eq!(again, frame(&mut st, vec![], &[]).0);
    assert!(has(&again, TWO_AXIS_FAMILY) && has(&again, "Source Sans 3") && has(&again, "JetBrains Mono"), "{again:?}");
}
