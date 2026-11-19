//! The Color Wheel panel's Show Out-of-Gamut Colors: the grid of out-of-gamut cells, its cache, the
//! wheel's square and the box sharing it, `ui.colorWheel {gamut}`, the panel menu and the saved
//! setting.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use vectorcraft_color::cms::{self, Cms, ColorSettings};
use vectorcraft_color::{Color, hsb_to_rgb};
use vectorcraft_engine::Session;

use super::color_wheel::{GAMUT, GAMUT_CELLS, GamutGrid, GamutKey, Settings, View, box_pick, cached_grid, gamut_grid, show};
use crate::VectorcraftApp;
use crate::dialogs::color_picker::Channel;

/// Whether cell `col` of row `row` (from the top-left) of `grid` is out of gamut.
fn marked(grid: &GamutGrid, col: usize, row: usize) -> bool {
    (grid[row] >> col) & 1 == 1
}

/// An app with a selected rectangle filled with `fill`, its Color Wheel panel on `view` with Show
/// Out-of-Gamut Colors set to `gamut`.
fn app(fill: &str, view: &str, gamut: bool) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("paint.setFill", json!({"color": fill})).unwrap();
    app.run("ui.colorWheel", json!({"view": view, "channel": "hue", "gamut": gamut})).unwrap();
    app
}

fn harness(app: VectorcraftApp) -> Harness<'static, VectorcraftApp> {
    let mut h = Harness::builder().with_size(vec2(260.0, 320.0)).build_ui_state(|ui, app: &mut VectorcraftApp| show(app, ui), app);
    h.run_steps(2);
    h
}

/// The grid the panel drew last, with its key.
fn drawn(h: &Harness<'static, VectorcraftApp>) -> Option<(GamutKey, GamutGrid)> {
    super::pstate::<Option<(GamutKey, GamutGrid)>>(&h.ctx, GAMUT)
}

/// At hue 240° the field's top-right cell is nearly pure blue, which the CMYK space cannot print,
/// and its bottom-left cell is nearly black, which it can. Each cell is the color a pick at its
/// center stores, tested as the out-of-gamut warning tests it.
#[test]
fn the_hue_grid_marks_blue_and_leaves_black() {
    let cms = Cms::default();
    let grid = gamut_grid(&cms, Channel::Hue, 240.0 / 360.0);
    let last = GAMUT_CELLS - 1;
    assert!(marked(&grid, last, 0), "{grid:04x?}");
    assert!(!marked(&grid, 0, last), "{grid:04x?}");
    for row in 0..GAMUT_CELLS {
        for col in 0..GAMUT_CELLS {
            let n = GAMUT_CELLS as f32;
            let hsb = box_pick(Channel::Hue, 240.0 / 360.0, (col as f32 + 0.5) / n, (row as f32 + 0.5) / n, [0.0; 3]);
            let [r, g, b] = cms.srgb_to_rgb(hsb_to_rgb(hsb));
            assert_eq!(marked(&grid, col, row), cms.out_of_gamut(&Color::rgb(r, g, b)), "cell {col}, {row}");
        }
    }
}

/// In the Blue box at full blue, the field's bottom-left cell (no red, no green) is nearly pure
/// blue.
#[test]
fn an_rgb_box_marks_its_own_cells() {
    let grid = gamut_grid(&Cms::default(), Channel::Blue, 1.0);
    assert!(marked(&grid, 0, GAMUT_CELLS - 1), "{grid:04x?}");
}

/// With uncalibrated CMYK as the CMYK space every color counts as printable, as the out-of-gamut
/// warning has it, so no cell is marked.
#[test]
fn uncalibrated_cmyk_marks_nothing() {
    let cms = Cms::new(&ColorSettings { cmyk: cms::DEVICE_CMYK.into(), ..ColorSettings::default() }).unwrap();
    assert_eq!(gamut_grid(&cms, Channel::Hue, 240.0 / 360.0), [0; GAMUT_CELLS]);
}

/// The cache computes a grid once per key: the same channel, value, color settings and profile
/// generation reuse it, and a change of any of them computes it again.
#[test]
fn the_cache_computes_once_per_key() {
    let mut slot = None;
    let mut runs = 0;
    let settings = ColorSettings::default();
    let mut get = |slot: &mut Option<(GamutKey, GamutGrid)>, key: GamutKey| {
        cached_grid(slot, key, || {
            runs += 1;
            [runs; GAMUT_CELLS]
        })
    };
    let key = (Channel::Hue, 0.5f32.to_bits(), settings.clone(), 0);
    assert_eq!(get(&mut slot, key.clone()), [1; GAMUT_CELLS]);
    assert_eq!(get(&mut slot, key.clone()), [1; GAMUT_CELLS], "the same key is not computed again");
    assert_eq!(get(&mut slot, (Channel::Hue, 0.25f32.to_bits(), settings.clone(), 0)), [2; GAMUT_CELLS]);
    assert_eq!(get(&mut slot, (Channel::Red, 0.25f32.to_bits(), settings.clone(), 0)), [3; GAMUT_CELLS]);
    let device = ColorSettings { cmyk: cms::DEVICE_CMYK.into(), ..settings };
    assert_eq!(get(&mut slot, (Channel::Red, 0.25f32.to_bits(), device.clone(), 0)), [4; GAMUT_CELLS]);
    assert_eq!(get(&mut slot, (Channel::Red, 0.25f32.to_bits(), device, 1)), [5; GAMUT_CELLS], "a profile loaded again");
}

/// A profile loaded again under the same name can change which colors are printable while the
/// color settings stay the same, so the panel tests the colors again after a profile is
/// registered.
#[test]
fn the_grid_is_computed_again_after_a_profile_is_loaded() {
    let mut h = harness(app("#3366cc", "box", true));
    let Some((before, _)) = drawn(&h) else { panic!("the box drew no grid") };
    /// Unregisters the profile when dropped, also after a failed assertion.
    struct Unregister(&'static str);
    impl Drop for Unregister {
        fn drop(&mut self) {
            cms::unregister_icc(self.0);
        }
    }
    let name = "Color Wheel Gamut Reload CMYK";
    let bytes = cms::icc_bytes(cms::GENERIC_CMYK).unwrap();
    cms::register_icc(&bytes, Some(name.into())).unwrap();
    let _unregister = Unregister(name);
    h.run_steps(2);
    let Some((after, _)) = drawn(&h) else { panic!("the box drew no grid") };
    assert_eq!((after.0, after.1, &after.2), (before.0, before.1, &before.2));
    assert!(after.3 > before.3, "the key follows the loaded profiles: {} then {}", before.3, after.3);
}

/// The wheel's square is the hue box at the ring's hue: both draw the same grid, under one key, so
/// a switch between them does not test the colors again.
#[test]
fn the_wheel_and_the_hue_box_shade_the_same_cells() {
    let mut h = harness(app("#3366cc", "wheel", true));
    let cms = cms::active();
    let Some((key, grid)) = drawn(&h) else { panic!("the wheel drew no grid") };
    let z = f32::from_bits(key.1);
    assert_eq!((key.0, &key.2), (Channel::Hue, cms.settings()));
    assert!((z * 360.0 - 220.0).abs() < 0.5, "the ring's hue: {}", z * 360.0);
    assert_eq!(grid, gamut_grid(&cms, Channel::Hue, z));
    h.state_mut().run("ui.colorWheel", json!({"view": "box"})).unwrap();
    h.run_steps(2);
    // Other tests in this process may register profiles between the frames, which changes the
    // key's profile generation; the channel, value, settings and grid stay the same.
    let Some((boxed, boxed_grid)) = drawn(&h) else { panic!("the box drew no grid") };
    assert_eq!((boxed.0, boxed.1, &boxed.2, boxed_grid), (key.0, key.1, &key.2, grid));
}

/// With Show Out-of-Gamut Colors off, the panel tests no colors.
#[test]
fn with_the_setting_off_no_grid_is_computed() {
    let h = harness(app("#3366cc", "box", false));
    assert_eq!(drawn(&h), None);
}

/// `ui.colorWheel {gamut}` sets the setting and keeps the others; a key set to null keeps its value
/// while the other keys apply; a value that is not true or false is an error that changes nothing,
/// translated where messages are.
#[test]
fn ui_color_wheel_sets_gamut() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let with = |v: serde_json::Value| {
        let mut v = v;
        let rest = json!({"sliders": "none", "harmony": false, "harmonyRule": "complementary", "vision": false, "documentColors": false});
        v.as_object_mut().unwrap().extend(rest.as_object().unwrap().clone());
        v
    };
    app.run("ui.colorWheel", json!({"view": "box"})).unwrap();
    assert_eq!(app.run("ui.colorWheel", json!({"gamut": true})).unwrap(), with(json!({"view": "box", "channel": "hue", "gamut": true})));
    let e = app.run("ui.colorWheel", json!({"view": "wheel", "gamut": "yes"})).unwrap_err();
    assert_eq!(e, "`gamut` must be true or false");
    assert_eq!(app.ui.color_wheel, Settings { view: View::Box, gamut: true, ..Default::default() }, "an error changes nothing");
    assert_eq!(app.run("ui.colorWheel", json!({"gamut": null})).unwrap()["gamut"], json!(true), "null keeps the value");
    app.run("ui.colorWheel", json!({"channel": "red"})).unwrap();
    let got = app.run("ui.colorWheel", json!({"view": "wheel", "channel": null, "gamut": null})).unwrap();
    assert_eq!(got, with(json!({"view": "wheel", "channel": "red", "gamut": true})), "null keeps a value while the other keys apply");
    let got = app.run("ui.colorWheel", json!({"view": null, "gamut": false})).unwrap();
    assert_eq!(got, with(json!({"view": "wheel", "channel": "red", "gamut": false})), "a null view keeps the view while gamut applies");
    for code in ["de", "fr", "ja", "zh-hans"] {
        let lang = crate::i18n::Lang::from_code(code).expect("registered");
        let shown = crate::i18n::message(lang, &e);
        assert_ne!(shown, e, "{code}");
        assert!(shown.contains("`gamut`"), "{code}: {shown}");
    }
}

/// The setting survives a JSON round trip of the preferences, and a saved value that is not true
/// or false reads as off without losing the other settings.
#[test]
fn the_gamut_setting_is_kept_and_read_leniently() {
    let ui = crate::UiState { color_wheel: Settings { view: View::Box, gamut: true, ..Default::default() }, ..Default::default() };
    let saved = serde_json::to_value(&ui).unwrap();
    assert_eq!(saved["color_wheel"]["gamut"], json!(true));
    let back: crate::UiState = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(back.color_wheel, ui.color_wheel);
    let mut odd = saved;
    odd["color_wheel"] = json!({"view": "box", "channel": "red", "gamut": "yes"});
    let back: crate::UiState = serde_json::from_value(odd).unwrap();
    assert_eq!(back.color_wheel, Settings { view: View::Box, channel: Channel::Red, gamut: false, ..Default::default() });
}

/// The panel menu's Show Out-of-Gamut Colors is checked while the setting is on, and switches it.
#[test]
fn the_panel_menu_switches_the_shading() {
    let app = app("#ff0000", "wheel", false);
    let mut fonts = false;
    let mut h = Harness::builder().with_size(vec2(260.0, 360.0)).build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            // The fonts, which draw the check mark, take effect from the next frame.
            if !fonts {
                crate::theme::install_fonts(ui.ctx());
                fonts = true;
                return;
            }
            assert!(super::panel_menu_items(app, ui, super::color_wheel::ID));
        },
        app,
    );
    h.run_steps(3);
    h.get_by_label("   Show Out-of-Gamut Colors").click();
    h.run_steps(2);
    assert!(h.state().ui.color_wheel.gamut);
    assert!(h.query_by_label("✓ Show Out-of-Gamut Colors").is_some());
    h.get_by_label("✓ Show Out-of-Gamut Colors").click();
    h.run_steps(2);
    assert!(!h.state().ui.color_wheel.gamut);
}

/// The edge runs between the marked cells and the others, inside the grid only, and bits past the
/// grid's width are not cells.
#[test]
fn the_edge_runs_between_marked_and_unmarked_cells() {
    assert_eq!(crate::widgets::shade_edges(&[0b01, 0b00]), vec![[(1, 0), (1, 1)], [(0, 1), (1, 1)]]);
    assert_eq!(crate::widgets::shade_edges(&[0b11, 0b11]), Vec::<[(usize, usize); 2]>::new(), "all marked");
    assert_eq!(crate::widgets::shade_edges(&[0b100]), Vec::<[(usize, usize); 2]>::new(), "bit 2 is past a 1 × 1 grid");
    assert_eq!(crate::widgets::shade_edges(&[]), Vec::<[(usize, usize); 2]>::new());
}

/// The gray covers each marked cell and no other, and each edge is drawn dark, then light on top.
#[test]
fn the_gray_covers_the_marked_cells_only() {
    use egui::{Pos2, Rect, Shape};
    let ctx = egui::Context::default();
    let rect = Rect::from_min_size(Pos2::new(10.0, 20.0), vec2(16.0, 16.0));
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::widgets::shade_cells(ui.painter(), rect, &[0b01, 0b10]));
    out.textures_delta.clear();
    let grays: Vec<Rect> = out
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            Shape::Rect(r) if r.fill == crate::widgets::SHADE => Some(r.rect),
            _ => None,
        })
        .collect();
    let cell = |col: f32, row: f32| Rect::from_min_size(Pos2::new(10.0 + col * 8.0, 20.0 + row * 8.0), vec2(8.0, 8.0));
    assert_eq!(grays, vec![cell(0.0, 0.0), cell(1.0, 1.0)]);
    let lines: Vec<(f32, egui::Color32)> = out
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            Shape::LineSegment { stroke, .. } => Some((stroke.width, stroke.color)),
            _ => None,
        })
        .collect();
    let dark = vec![(2.0, egui::Color32::BLACK); 4];
    let light = vec![(1.0, egui::Color32::WHITE); 4];
    assert_eq!(lines, [dark, light].concat(), "four edges, dark under light");
}
