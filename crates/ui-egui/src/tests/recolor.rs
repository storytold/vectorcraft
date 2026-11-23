//! Recolor Artwork (M3.78) and Edit or Apply Color Group (M3.79), drawn headlessly.

use egui::{Event, Modifiers, PointerButton, Pos2, Rect, vec2};
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};
use vectorcraft_color::Color;
use vectorcraft_color::recolor::ColorKey;
use vectorcraft_engine::Session;

use crate::{VectorcraftApp, dialogs, panels};

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    app
}

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    ctx
}

fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>, time: f64, draw: fn(&mut VectorcraftApp, &mut egui::Ui)) {
    let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 800.0)));
    let input = egui::RawInput { events, time: Some(time), screen_rect, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| draw(app, ui));
    out.textures_delta.clear();
}

fn dialog_frame(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let time = ctx.input(|i| i.time) + 1.0;
    frame(app, ctx, vec![], time, |app, ui| dialogs::show(app, ui.ctx()));
}

fn press(at: Pos2, clicks: usize) -> Vec<Event> {
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    std::iter::once(Event::PointerMoved(at)).chain((0..clicks).flat_map(|_| [button(true), button(false)])).collect()
}

/// Rectangles filled with `colors`, all selected.
fn art(app: &mut VectorcraftApp, colors: &[&str]) {
    for (i, c) in colors.iter().enumerate() {
        let id = app.run("shape.rectangle", json!({"x": 10 + 30 * i, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
        app.run("paint.setFill", json!({"ids": [id], "color": c})).unwrap();
        app.run("paint.setStroke", json!({"ids": [id], "none": true})).unwrap();
    }
    app.run("select.all", json!({})).unwrap();
}

fn colors(app: &mut VectorcraftApp) -> usize {
    app.session.execute("recolor.colors", &json!({})).unwrap()["colors"].as_array().unwrap().len()
}

fn group_keys(app: &VectorcraftApp, name: &str) -> Vec<Value> {
    let d = &app.session.doc().unwrap().doc;
    let g = d.swatch_groups.iter().find(|g| g.name == name).unwrap();
    g.swatches.iter().filter_map(|w| w.paint.color()).map(|c| json!(ColorKey::of(&c).to_string())).collect()
}

fn field(app: &VectorcraftApp, k: &str) -> Value {
    app.ui.dialog.as_ref().and_then(|d| d.fields.get(k).cloned()).unwrap_or(Value::Null)
}

fn set(app: &mut VectorcraftApp, k: &str, v: Value) {
    app.ui.dialog.as_mut().unwrap().fields.insert(k.into(), v);
}

#[test]
fn the_dialog_reduces_previews_and_applies_as_one_undo_step() {
    let mut app = app();
    let ctx = context();
    art(&mut app, &["#ff0000", "#e01010", "#0000ff", "#1010e0", "#ff3030"]);
    app.run("ui.recolorDialog", json!({})).unwrap();
    dialog_frame(&mut app, &ctx);
    assert_eq!(field(&app, "rows").as_array().unwrap().len(), 5, "Auto: a row per colour");
    assert!(app.session.in_interaction(), "the dialog previews");
    // Two colours, Exact; OK without another frame still reduces first.
    set(&mut app, "colors", json!(2));
    set(&mut app, "method", json!("exact"));
    let undo = app.session.doc().unwrap().history.undo.len();
    dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(colors(&mut app), 2);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(colors(&mut app), 5);
    // Cancel rolls the preview back.
    app.run("ui.recolorDialog", json!({"colors": 1})).unwrap();
    set(&mut app, "method", json!("exact"));
    dialog_frame(&mut app, &ctx);
    assert_eq!(colors(&mut app), 1, "previewed");
    dialogs::cancel(&mut app);
    assert_eq!(colors(&mut app), 5);
}

#[test]
fn presets_open_colour_jobs_and_libraries_and_both_tabs_draw() {
    let mut app = app();
    let ctx = context();
    art(&mut app, &["#ff0000", "#00ff00", "#0000ff"]);
    let entries = crate::menus::menu_entries(&app);
    let presets: Vec<(&str, &Value)> =
        entries.iter().filter(|e| e.path.last().is_some_and(|p| p == "Recolor with Preset")).map(|e| (e.label.as_str(), &e.params)).collect();
    assert_eq!(presets.iter().map(|p| p.0).collect::<Vec<_>>(), ["1 Color Job…", "2 Color Job…", "3 Color Job…", "Color Library…"]);
    app.run("ui.recolorDialog", presets[1].1.clone()).unwrap();
    dialog_frame(&mut app, &ctx);
    assert_eq!((field(&app, "colors"), field(&app, "method")), (json!(2), json!("scaleTints")));
    assert_eq!(field(&app, "rows").as_array().unwrap().len(), 2);
    set(&mut app, "tab", json!("edit"));
    dialog_frame(&mut app, &ctx);
    dialogs::cancel(&mut app);
    app.run("ui.recolorDialog", presets[3].1.clone()).unwrap();
    assert_eq!(field(&app, "limitTo"), "web-safe-216", "Color Library starts on the first library");
    dialog_frame(&mut app, &ctx);
    let web = |k: &Value| vectorcraft_engine::cmd::color_value(k).unwrap().to_rgb().iter().all(|v| ((v * 5.0).round() - v * 5.0).abs() < 1e-3);
    assert!(field(&app, "rows").as_array().unwrap().iter().all(|r| web(&r["to"])));
    dialogs::cancel(&mut app);
    // The Color Guide's colours become the new colours.
    app.run("ui.recolorDialog", json!({"colors": ["#123456", "#abcdef"]})).unwrap();
    dialog_frame(&mut app, &ctx);
    let tos: Vec<Value> = field(&app, "rows").as_array().unwrap().iter().map(|r| r["to"].clone()).collect();
    assert_eq!(tos, [json!("rgb 18 52 86"), json!("rgb 171 205 239")]);
    dialogs::cancel(&mut app);
}

#[test]
fn double_clicking_a_colour_group_opens_it_and_ok_rewrites_it() {
    let mut app = app();
    let ctx = context();
    let show: fn(&mut VectorcraftApp, &mut egui::Ui) = panels::swatches::show;
    frame(&mut app, &ctx, vec![], 0.0, show);
    let folder = ctx.read_response(egui::Id::unique(("swatch-tile", "Brights"))).expect("the Brights folder").rect.center();
    frame(&mut app, &ctx, press(folder, 2), 1.0, show);
    let d = app.ui.dialog.as_ref().expect("a dialog opened");
    assert_eq!((d.kind.as_str(), d.str("group")), (dialogs::recolor::KIND, "Brights".to_string()));
    let keys = group_keys(&app, "Brights");
    let from: Vec<Value> = field(&app, "rows").as_array().unwrap().iter().map(|r| r["from"][0].clone()).collect();
    assert_eq!(from, keys, "a row per group colour, in order");
    dialog_frame(&mut app, &ctx);
    // A new first colour and a new name; nothing is selected, so only the group changes.
    let mut rows = field(&app, "rows");
    rows[0]["to"] = json!("cmyk 100 0 0 0");
    set(&mut app, "rows", rows);
    set(&mut app, "groupName", json!("Brights 2026"));
    let undo = app.session.doc().unwrap().history.undo.len();
    dialogs::confirm(&mut app).unwrap();
    let mut want = keys.clone();
    want[0] = json!("cmyk 100 0 0 0");
    assert_eq!(group_keys(&app, "Brights 2026"), want);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
}

#[test]
fn the_options_button_edits_a_selected_group_and_applies_it_to_the_art() {
    let mut app = app();
    let ctx = context();
    art(&mut app, &["#ff0000", "#00ff00", "#0000ff", "#ffff00", "#ff00ff"]);
    let show: fn(&mut VectorcraftApp, &mut egui::Ui) = panels::swatches::show;
    frame(&mut app, &ctx, vec![], 0.0, show);
    let folder = ctx.read_response(egui::Id::unique(("swatch-tile", "Brights"))).unwrap().rect.center();
    frame(&mut app, &ctx, press(folder, 1), 1.0, show);
    frame(&mut app, &ctx, vec![], 2.0, show);
    // The bottom bar's third button: Swatch Options, or Edit or Apply Color Group for a group.
    let mut bar: Vec<Rect> = ctx.viewport(|vp| {
        vp.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).filter(|w| w.rect.size() == vec2(24.0, 24.0)).map(|w| w.rect).collect()
    });
    let bottom = bar.iter().map(|r| r.top()).fold(f32::MIN, f32::max);
    bar.retain(|r| r.top() == bottom);
    bar.sort_by(|a, b| a.left().total_cmp(&b.left()));
    frame(&mut app, &ctx, press(bar[2].center(), 1), 3.0, show);
    assert_eq!(field(&app, "group"), "Brights");
    dialog_frame(&mut app, &ctx);
    let group = group_keys(&app, "Brights");
    let tos: Vec<Value> = field(&app, "rows").as_array().unwrap().iter().map(|r| r["to"].clone()).collect();
    assert_eq!(tos, group, "the art's colours in as many rows as the group has, taking its colours");
    dialogs::confirm(&mut app).unwrap();
    let used: Vec<String> = app.session.execute("recolor.colors", &json!({})).unwrap()["colors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["key"].as_str().unwrap().to_string())
        .collect();
    assert!(used.iter().all(|k| group.contains(&json!(k))), "the art uses the group's colours: {used:?}");
    assert!(matches!(ColorKey::parse(&used[0]).unwrap().color(), Color::Rgb { .. }));
}

#[test]
fn a_group_keeps_the_colours_no_art_row_took() {
    let mut app = app();
    let ctx = context();
    art(&mut app, &["#ff0000", "#0000ff"]);
    let before = group_keys(&app, "Brights");
    app.run("ui.recolorDialog", json!({"group": "Brights"})).unwrap();
    dialog_frame(&mut app, &ctx);
    assert_eq!(field(&app, "rows").as_array().unwrap().len(), 2, "two art colours");
    let mut rows = field(&app, "rows");
    rows[1]["to"] = json!("rgb 1 2 3");
    set(&mut app, "rows", rows);
    dialogs::confirm(&mut app).unwrap();
    let after = group_keys(&app, "Brights");
    assert_eq!(after.len(), before.len());
    assert_eq!((&after[0], &after[1]), (&before[0], &json!("rgb 1 2 3")));
    assert_eq!(after[2..], before[2..]);
}

/// Recolor Artwork on art filled with `colors`, on the Edit tab, drawn by a kittest harness.
fn edit_harness(colors: &[&str]) -> egui_kittest::Harness<'static, VectorcraftApp> {
    let mut app = app();
    art(&mut app, colors);
    app.run("ui.recolorDialog", json!({})).unwrap();
    set(&mut app, "tab", json!("edit"));
    assert_eq!(field(&app, "linked"), json!(true));
    let mut fonts = false;
    let mut h = egui_kittest::Harness::builder().with_size(vec2(900.0, 800.0)).build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            if !std::mem::replace(&mut fonts, true) {
                crate::theme::install_fonts(ui.ctx());
                return;
            }
            dialogs::show(app, ui.ctx());
        },
        app,
    );
    h.run_steps(4);
    h
}

/// Recolor Artwork open on the Assign tab of `app` (its art selected), drawn by a kittest harness.
fn assign_harness(mut app: VectorcraftApp, open: Value) -> egui_kittest::Harness<'static, VectorcraftApp> {
    app.run("ui.recolorDialog", open).unwrap();
    set(&mut app, "method", json!("exact"));
    let mut fonts = false;
    let builder = egui_kittest::Harness::builder().with_size(vec2(900.0, 800.0));
    let builder = if std::env::var_os("VECTORCRAFT_RECOLOR_CAPTURE_DIR").is_some() { builder.wgpu() } else { builder };
    let mut h = builder.build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            if !std::mem::replace(&mut fonts, true) {
                crate::theme::install_fonts(ui.ctx());
                return;
            }
            dialogs::show(app, ui.ctx());
        },
        app,
    );
    h.run_steps(4);
    h
}

/// Clicks the color slider named `name` at `at` (0 to 1) along its rail.
fn click_slider(h: &mut egui_kittest::Harness<'_, VectorcraftApp>, name: &str, at: f32) {
    let r = h.get_by_role_and_label(egui::accesskit::Role::Slider, name).rect();
    let pos = egui::pos2(r.left() + 4.0 + at * (r.width() - 8.0), r.center().y);
    h.input_mut().events.extend(press(pos, 1));
    h.run_steps(3);
}

/// The fill of the first object of the document.
fn first_fill(app: &VectorcraftApp) -> vectorcraft_color::Paint {
    let d = &app.session.doc().unwrap().doc;
    let id = d.layers[0].children().unwrap()[0].id;
    d.node(id).unwrap().appearance.fill_paint()
}

/// The color mode menu's sliders edit the first selected row's new color in the mode picked (the
/// other row keeps its color), the art previews it, and OK gives the art the color in that mode:
/// a CMYK color on RGB art makes it CMYK. Cancel leaves the art as it was, with no undo step.
#[test]
fn the_color_sliders_edit_the_selected_rows_new_color_in_their_mode() {
    for confirm in [true, false] {
        let mut app = app();
        art(&mut app, &["#0000ff", "#ff0000"]);
        let before = first_fill(&app);
        let undo = app.session.doc().unwrap().history.undo.len();
        let mut h = assign_harness(app, json!({}));
        let rows = field(h.state(), "rows");
        let blue = 0;
        assert_eq!(rows[blue]["from"], json!(["rgb 0 0 255"]), "the first object's blue is the first row");
        // Both rows selected, the second one first: the sliders edit the first row.
        set(h.state_mut(), "__sel", json!([1, blue]));
        h.run_steps(2);
        assert!(h.query_by_role_and_label(egui::accesskit::Role::Slider, "R").is_some(), "RGB art shows the RGB sliders");
        set(h.state_mut(), "sliderMode", json!("cmyk"));
        h.run_steps(2);
        click_slider(&mut h, "C", 0.5);
        let edited = field(h.state(), "rows");
        let to = edited[blue]["to"].as_str().unwrap().to_string();
        assert!(to.starts_with("cmyk 50 "), "the first selected row takes C 50%: {to}");
        assert_eq!(edited[1], rows[1], "the other selected row keeps its new color");
        let Some(Color::Cmyk { c, .. }) = first_fill(h.state()).color() else {
            panic!("the art previews the CMYK color: {:?}", first_fill(h.state()))
        };
        assert!((c - 0.5).abs() < 1e-3, "{c}");
        h.get_by_label(if confirm { "OK" } else { "Cancel" }).click();
        h.run_steps(2);
        assert!(h.state().ui.dialog.is_none());
        let app = h.state();
        if confirm {
            assert_eq!(first_fill(app).color(), vectorcraft_engine::cmd::color_value(&json!(to)), "OK keeps the color, in CMYK");
            assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
        } else {
            assert_eq!(first_fill(app), before, "Cancel leaves the art unchanged");
            assert_eq!(app.session.doc().unwrap().history.undo.len(), undo, "and records no undo step");
        }
    }
}

/// A gray row shows HSB, and the sliders stay in HSB once an edit makes the color RGB.
#[test]
fn a_gray_row_keeps_the_hsb_sliders_while_it_is_edited() {
    let mut app = app();
    art(&mut app, &["gray 40"]);
    let mut h = assign_harness(app, json!({}));
    set(h.state_mut(), "__sel", json!([0]));
    h.run_steps(2);
    assert!(h.query_by_role_and_label(egui::accesskit::Role::Slider, "H").is_some(), "a gray row shows HSB");
    click_slider(&mut h, "S", 0.5);
    let to = field(h.state(), "rows")[0]["to"].clone();
    assert!(to.as_str().is_some_and(|k| k.starts_with("rgb ")), "{to}");
    assert!(h.query_by_role_and_label(egui::accesskit::Role::Slider, "H").is_some(), "still HSB after the edit");
    assert_eq!(field(h.state(), "sliderMode"), json!("hsb"));
}

/// With Limit to Library, a row whose new color is a spot color of the library shows Tint, whose T
/// slider sets the row's tint of it; OK links the art to the spot color at that tint.
#[test]
fn the_tint_slider_sets_a_spot_rows_tint() {
    let mut app = app();
    app.run("swatch.new", json!({"name": "Ink", "color": {"c": 0, "m": 1, "y": 0, "k": 0}, "spot": true})).unwrap();
    let data = app.run("swatch.library.save", json!({"names": ["Ink"], "name": "Inks"})).unwrap()["data"].as_str().unwrap().to_string();
    let lib = app.run("swatch.library.load", json!({"data": data, "name": "inks.vcswatches"})).unwrap()["library"].as_str().unwrap().to_string();
    app.run("swatch.delete", json!({"names": ["Ink"]})).unwrap();
    art(&mut app, &["cmyk 0 100 0 0"]);
    let mut h = assign_harness(app, json!({"library": lib}));
    set(h.state_mut(), "__sel", json!([0]));
    h.run_steps(2);
    assert!(h.query_by_role_and_label(egui::accesskit::Role::Slider, "T").is_some(), "a spot row shows Tint");
    click_slider(&mut h, "T", 0.4);
    assert_eq!(field(h.state(), "rows")[0]["tint"], json!(40.0));
    h.get_by_label("OK").click();
    h.run_steps(2);
    match first_fill(h.state()) {
        vectorcraft_color::Paint::Solid { swatch, tint, .. } => {
            assert_eq!(swatch.as_deref(), Some("Ink"), "linked to the spot color");
            assert!((tint - 0.4).abs() < 1e-4, "at the row's tint: {tint}");
        }
        p => panic!("{p:?}"),
    }
}

/// A spot row's tint goes when the Edit tab gives the row another new color: after a click on the
/// Brightness slider snaps the row to a second spot color, OK links the art to that one at full
/// strength. A click that snaps back to the same spot color keeps the tint.
#[test]
fn a_new_color_from_the_edit_tab_drops_a_spot_rows_tint() {
    let mut app = app();
    let lib = library_of(
        &mut app,
        &[json!({"name": "Mix Ink", "color": "#ff00ff", "spot": true}), json!({"name": "Mix Dark", "color": "#800080", "spot": true})],
    );
    art(&mut app, &["#ff00ff"]);
    let mut h = assign_harness(app, json!({"library": lib}));
    let mut rows = field(h.state(), "rows");
    rows[0]["tint"] = json!(40.0);
    set(h.state_mut(), "rows", rows);
    set(h.state_mut(), "tab", json!("edit"));
    h.run_steps(4);
    click_slider(&mut h, "Brightness", 0.9);
    let row = field(h.state(), "rows")[0].clone();
    assert_eq!((&row["to"], &row["tint"]), (&json!("rgb 255 0 255"), &json!(40.0)), "snapped back to the same spot color");
    click_slider(&mut h, "Brightness", 0.4);
    let row = field(h.state(), "rows")[0].clone();
    assert_eq!(row["to"], json!("rgb 128 0 128"), "snapped to the darker spot color");
    assert!(row.get("tint").is_none(), "the row's tint went with its old color: {row}");
    h.get_by_label("OK").click();
    h.run_steps(2);
    match first_fill(h.state()) {
        vectorcraft_color::Paint::Solid { swatch, tint, .. } => {
            assert_eq!(swatch.as_deref(), Some("Mix Dark"), "linked to the new spot color");
            assert!((tint - 1.0).abs() < 1e-4, "at full strength: {tint}");
        }
        p => panic!("{p:?}"),
    }
}

/// Randomize order moves a tint with its color, also between rows of the same new color: after
/// each click exactly one row is tinted, and some click moves the tint to the other row.
#[test]
fn randomize_order_moves_a_tint_between_rows_of_the_same_color() {
    let mut app = app();
    art(&mut app, &["#ff0000", "#0000ff"]);
    let mut h = assign_harness(app, json!({}));
    let mut rows = field(h.state(), "rows");
    for r in rows.as_array_mut().unwrap() {
        r["to"] = json!("rgb 0 255 0");
    }
    rows[0]["tint"] = json!(40.0);
    set(h.state_mut(), "rows", rows);
    h.run_steps(2);
    let mut moved = false;
    for _ in 0..8 {
        h.get_by_label("Randomly change color order").click();
        h.run_steps(2);
        let rows = field(h.state(), "rows");
        let tinted: Vec<usize> = (0..2).filter(|i| rows[*i].get("tint").is_some()).collect();
        assert_eq!(tinted.len(), 1, "one tinted row: {rows}");
        moved |= tinted == [1];
    }
    assert!(moved, "some click moves the tint to the other row");
}

/// With a one-color library, Randomly change saturation and brightness snaps a spot row back to
/// its spot color, which keeps its tint.
#[test]
fn a_saturation_change_that_snaps_back_keeps_the_tint() {
    let mut app = app();
    let lib = library_of(&mut app, &[json!({"name": "Mix Ink", "color": "#ff00ff", "spot": true})]);
    art(&mut app, &["#ff00ff"]);
    let mut h = assign_harness(app, json!({"library": lib}));
    let mut rows = field(h.state(), "rows");
    rows[0]["tint"] = json!(40.0);
    set(h.state_mut(), "rows", rows);
    h.run_steps(2);
    h.get_by_label("Randomly change saturation and brightness").click();
    h.run_steps(2);
    let row = field(h.state(), "rows")[0].clone();
    assert_eq!((&row["to"], &row["tint"]), (&json!("rgb 255 0 255"), &json!(40.0)), "{row}");
}

/// Makes the swatches of the `swatch.new` params `swatches`, saves them as a library and loads it,
/// removes them from the document again, and returns the library's id.
fn library_of(app: &mut VectorcraftApp, swatches: &[Value]) -> String {
    let names: Vec<Value> = swatches.iter().map(|s| s["name"].clone()).collect();
    for s in swatches {
        app.run("swatch.new", s.clone()).unwrap();
    }
    let data = app.run("swatch.library.save", json!({"names": names, "name": "Lib"})).unwrap()["data"].as_str().unwrap().to_string();
    let lib = app.run("swatch.library.load", json!({"data": data, "name": "lib.vcswatches"})).unwrap()["library"].as_str().unwrap().to_string();
    app.run("swatch.delete", json!({"names": names})).unwrap();
    lib
}

/// When the library's first swatch of a row's new color is a process color, the row shows no Tint,
/// even with a spot color of that color after it: `recolor.apply` gives the art the first one.
#[test]
fn a_process_swatch_before_a_spot_one_of_its_color_shows_no_tint() {
    let mut app = app();
    let lib = library_of(
        &mut app,
        &[
            json!({"name": "Mix Process", "color": {"c": 0, "m": 1, "y": 0, "k": 0}}),
            json!({"name": "Mix Ink", "color": {"c": 0, "m": 1, "y": 0, "k": 0}, "spot": true}),
        ],
    );
    art(&mut app, &["cmyk 0 100 0 0"]);
    let mut h = assign_harness(app, json!({"library": lib}));
    set(h.state_mut(), "__sel", json!([0]));
    h.run_steps(2);
    assert!(h.query_by_role_and_label(egui::accesskit::Role::Slider, "T").is_none(), "no Tint for a process swatch");
    assert!(h.query_by_role_and_label(egui::accesskit::Role::Slider, "C").is_some(), "the CMYK sliders");
}

/// Opened on a color group without art, a row whose new color is a spot color of the library shows
/// the process modes and no Tint: OK saves only the group's colors, so a tint would be lost.
#[test]
fn a_spot_row_of_a_group_without_art_shows_no_tint() {
    let mut app = app();
    let lib = library_of(&mut app, &[json!({"name": "Mix Ink", "color": {"c": 0, "m": 1, "y": 0, "k": 0}, "spot": true})]);
    app.run("swatch.newGroup", json!({"name": "Mix", "colors": ["cmyk 0 100 0 0"], "keepModel": true})).unwrap();
    let mut h = assign_harness(app, json!({"group": "Mix", "library": lib}));
    assert_ne!(field(h.state(), "__art"), json!(true), "no art");
    set(h.state_mut(), "__sel", json!([0]));
    h.run_steps(2);
    assert!(h.query_by_role_and_label(egui::accesskit::Role::Slider, "T").is_none(), "no Tint without art");
    assert!(h.query_by_role_and_label(egui::accesskit::Role::Slider, "C").is_some(), "the CMYK sliders");
}

/// A spot color whose components are finer than a row's color key still shows Tint: the row's
/// rounded color snaps back to it, as `recolor.apply` snaps it.
#[test]
fn a_spot_color_finer_than_the_rows_key_shows_tint() {
    let mut app = app();
    let lib = library_of(&mut app, &[json!({"name": "Mix Fine", "color": {"c": 0, "m": 0.12345, "y": 0, "k": 0}, "spot": true})]);
    art(&mut app, &["cmyk 0 12 0 0"]);
    let mut h = assign_harness(app, json!({"library": lib}));
    set(h.state_mut(), "__sel", json!([0]));
    h.run_steps(2);
    assert!(h.query_by_role_and_label(egui::accesskit::Role::Slider, "T").is_some(), "the spot row shows Tint");
}

/// Reset with the preview on reads the art's own colors again and discards a slider edit: OK then
/// gives RGB art a CMYK color group's color in RGB, as it does without the edit.
#[test]
fn reset_after_a_slider_edit_keeps_the_arts_model() {
    let mut app = app();
    app.run("swatch.newGroup", json!({"name": "Mix", "colors": ["cmyk 0 100 0 0"], "keepModel": true})).unwrap();
    art(&mut app, &["#ff00ff"]);
    let mut h = assign_harness(app, json!({"group": "Mix"}));
    set(h.state_mut(), "__sel", json!([0]));
    h.run_steps(2);
    click_slider(&mut h, "C", 0.5);
    assert_eq!(field(h.state(), "rows")[0]["model"], json!("new"), "the sliders edited the row");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Reset the rows to the artwork's colors").click();
    h.run_steps(3);
    assert_eq!(field(h.state(), "rows")[0]["from"], json!(["rgb 255 0 255"]), "Reset reads the art's colors, not the preview's");
    h.get_by_label("OK").click();
    h.run_steps(2);
    assert!(h.state().ui.dialog.is_none());
    assert!(matches!(first_fill(h.state()).color(), Some(Color::Rgb { .. })), "the art stays RGB: {:?}", first_fill(h.state()));
}

/// A change of the color count rebuilds the rows and discards a slider edit: OK then gives RGB art
/// a CMYK color group's color in RGB, as it does without the edit.
#[test]
fn a_new_color_count_after_a_slider_edit_keeps_the_arts_model() {
    let mut app = app();
    app.run("swatch.newGroup", json!({"name": "Mix", "colors": ["cmyk 0 100 0 0", "cmyk 100 0 0 0"], "keepModel": true})).unwrap();
    art(&mut app, &["#ff00ff", "#00ffff"]);
    let mut h = assign_harness(app, json!({"group": "Mix"}));
    set(h.state_mut(), "__sel", json!([0]));
    h.run_steps(2);
    click_slider(&mut h, "C", 0.5);
    assert_eq!(field(h.state(), "rows")[0]["model"], json!("new"), "the sliders edited the row");
    set(h.state_mut(), "colors", json!(1));
    h.run_steps(3);
    assert_eq!(field(h.state(), "rows").as_array().map(Vec::len), Some(1), "the rows are rebuilt");
    assert_eq!(field(h.state(), "rows")[0].get("model"), None, "the slider edit is discarded");
    h.get_by_label("OK").click();
    h.run_steps(2);
    assert!(h.state().ui.dialog.is_none());
    assert!(matches!(first_fill(h.state()).color(), Some(Color::Rgb { .. })), "the art stays RGB: {:?}", first_fill(h.state()));
}

/// The fill of the `n`th object of the document.
fn fill_of(app: &VectorcraftApp, n: usize) -> vectorcraft_color::Paint {
    let d = &app.session.doc().unwrap().doc;
    let id = d.layers[0].children().unwrap()[n].id;
    d.node(id).unwrap().appearance.fill_paint()
}

/// A slider edit of one row gives only that row's art the slider's mode: with RGB art and a CMYK
/// color group, OK makes the edited row's art CMYK and leaves the other row's art RGB.
#[test]
fn a_slider_edit_changes_the_model_of_its_own_row_only() {
    let mut app = app();
    app.run("swatch.newGroup", json!({"name": "Mix", "colors": ["cmyk 0 100 0 0", "cmyk 100 0 0 0"], "keepModel": true})).unwrap();
    art(&mut app, &["#ff00ff", "#00ffff"]);
    let mut h = assign_harness(app, json!({"group": "Mix"}));
    let rows = field(h.state(), "rows");
    let magenta = (0..2).find(|i| rows[*i]["from"] == json!(["rgb 255 0 255"])).expect("a row of the magenta art");
    set(h.state_mut(), "__sel", json!([magenta]));
    h.run_steps(2);
    click_slider(&mut h, "C", 0.5);
    let rows = field(h.state(), "rows");
    assert_eq!(rows[magenta]["model"], json!("new"), "the edited row: {rows}");
    assert_eq!(rows[1 - magenta].get("model"), None, "the other row: {rows}");
    h.get_by_label("OK").click();
    h.run_steps(2);
    assert!(h.state().ui.dialog.is_none());
    assert!(matches!(fill_of(h.state(), 0).color(), Some(Color::Cmyk { .. })), "the edited row's art is CMYK: {:?}", fill_of(h.state(), 0));
    assert!(matches!(fill_of(h.state(), 1).color(), Some(Color::Rgb { .. })), "the other row's art stays RGB: {:?}", fill_of(h.state(), 1));
}

/// The dialog keeps its height when the tab or the sliders' color mode changes.
#[test]
fn switching_tabs_or_slider_modes_keeps_the_dialog_height() {
    let mut app = app();
    art(&mut app, &["#ff0000", "#0000ff"]);
    let mut h = assign_harness(app, json!({}));
    set(h.state_mut(), "__sel", json!([0]));
    let height = |h: &mut egui_kittest::Harness<'_, VectorcraftApp>, k: &str, v: Value| {
        set(h.state_mut(), k, v);
        h.run_steps(6);
        h.ctx.memory(|m| m.area_rect(egui::Id::unique(("dialog", "recolor")))).unwrap().height()
    };
    let assign = height(&mut h, "sliderMode", json!("cmyk"));
    for (k, v) in [
        ("sliderMode", json!("hsb")),
        ("sliderMode", json!("lab")),
        ("tab", json!("edit")),
        ("display", json!("segmented")),
        ("display", json!("bars")),
        ("sliderMode", json!("global")),
        ("display", json!("smooth")),
        ("sliderMode", json!("rgb")),
        ("tab", json!("assign")),
    ] {
        assert_eq!(height(&mut h, k, v.clone()), assign, "{k} {v}");
    }
}

/// The fills of the document's objects, in order.
fn fills(app: &VectorcraftApp) -> Vec<Option<Color>> {
    let d = &app.session.doc().unwrap().doc;
    d.layers[0].children().unwrap().iter().map(|n| d.node(n.id).unwrap().appearance.fill_paint().color()).collect()
}

/// The Edit tab's color mode menu offers Global Adjust: Saturation, Brightness, Temperature and
/// Luminosity open at 0%, which changes nothing; one slider changes every recolored color at once
/// (the rows keep their new colors) and the art previews it; back at 0% the art has its own colors
/// again; the amounts stay while the tab or the mode changes; OK keeps the change as one undo step
/// and Cancel leaves the art as it was.
#[test]
fn global_adjust_changes_every_color_at_once() {
    for confirm in [true, false] {
        let mut h = edit_harness(&["#ff0000", "#0000ff", "#00ff00"]);
        let before = fills(h.state());
        let undo = h.state().session.doc().unwrap().history.undo.len();
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Color Mode:").click();
        h.run_steps(2);
        h.get_by_label("Global Adjust").click();
        h.run_steps(3);
        assert_eq!(field(h.state(), "sliderMode"), json!("global"));
        for name in ["Saturation", "Temperature", "Luminosity"] {
            assert!(h.query_by_role_and_label(egui::accesskit::Role::Slider, name).is_some(), "{name}");
        }
        // Global Adjust's Brightness and the wheel's.
        assert_eq!(h.get_all_by_role_and_label(egui::accesskit::Role::Slider, "Brightness").count(), 2);
        assert_eq!(fills(h.state()), before, "at 0% the art keeps its colors");
        let rows = field(h.state(), "rows");
        click_slider(&mut h, "Saturation", 0.0);
        assert_eq!(field(h.state(), "globalAdjust")["saturation"], json!(-100.0));
        assert_eq!(field(h.state(), "rows"), rows, "the rows keep their new colors");
        let gray = |app: &VectorcraftApp| fills(app).iter().all(|c| c.is_some_and(|c| c.to_hsb()[1] < 1e-3));
        assert!(gray(h.state()), "every color previews gray: {:?}", fills(h.state()));
        click_slider(&mut h, "Saturation", 0.5);
        assert_eq!(field(h.state(), "globalAdjust")["saturation"], json!(0.0));
        assert_eq!(fills(h.state()), before, "back at 0% the art has its own colors");
        click_slider(&mut h, "Saturation", 0.0);
        for (k, v) in [("tab", json!("assign")), ("tab", json!("edit")), ("sliderMode", json!("hsb")), ("sliderMode", json!("global"))] {
            set(h.state_mut(), k, v);
            h.run_steps(3);
        }
        assert_eq!(field(h.state(), "globalAdjust")["saturation"], json!(-100.0), "the amount stays");
        assert!(gray(h.state()), "and still applies");
        h.get_by_label(if confirm { "OK" } else { "Cancel" }).click();
        h.run_steps(2);
        assert!(h.state().ui.dialog.is_none());
        let app = h.state();
        if confirm {
            assert!(gray(app), "OK keeps the change: {:?}", fills(app));
            assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
        } else {
            assert_eq!(fills(app), before, "Cancel leaves the art unchanged");
            assert_eq!(app.session.doc().unwrap().history.undo.len(), undo, "and records no undo step");
        }
    }
}

/// Colors opened alone with Limit to Library: OK saves the new colors with the Global Adjust as a
/// new color group, each snapped to the library again, as `recolor.apply` gives them to a group.
#[test]
fn global_adjust_on_a_new_group_keeps_its_colors_in_the_library() {
    let mut app = app();
    app.run("ui.recolorDialog", json!({"colors": ["#123456", "#abcdef"], "library": ""})).unwrap();
    assert_eq!(field(&app, "limitTo"), "web-safe-216");
    let web = |k: &Value| vectorcraft_engine::cmd::color_value(k).unwrap().to_rgb().iter().all(|v| ((v * 5.0).round() - v * 5.0).abs() < 1e-3);
    let rows: Vec<Value> = field(&app, "rows").as_array().unwrap().iter().map(|r| r["to"].clone()).collect();
    assert!(rows.iter().all(web), "the rows open snapped: {rows:?}");
    set(&mut app, "globalAdjust", json!({"brightness": -60}));
    dialogs::confirm(&mut app).unwrap();
    let keys = group_keys(&app, "Color Group");
    assert_eq!(keys.len(), 2);
    assert_ne!(keys, rows, "the Global Adjust changed the colors");
    assert!(keys.iter().all(web), "every group color is a library color: {keys:?}");
}

/// The Assign tab's per-row hex fields are named by their row, not all "Color": a screen reader can
/// tell the second from the first.
#[test]
fn each_recolor_hex_field_has_its_row_number() {
    let mut h = edit_harness(&["#ff0000", "#0000ff"]);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Assign").click();
    h.run_steps(4);
    let _ = h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Color 1");
    let _ = h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Color 2");
    assert!(h.query_by_role_and_label(egui::accesskit::Role::TextInput, "Color").is_none(), "no ambiguous Color field");
}

/// #2960: the Preserve checkboxes open as OK last left them; Cancel keeps what OK left.
#[test]
fn the_preserve_checkboxes_open_as_they_were_last_confirmed() {
    let mut app = app();
    art(&mut app, &["#ff0000", "#ffffff", "#000000", "#808080"]);
    let preserve = |app: &VectorcraftApp| ["preserveWhite", "preserveBlack", "preserveGrays"].map(|k| field(app, k));
    app.run("ui.recolorDialog", json!({})).unwrap();
    assert_eq!(preserve(&app), [json!(true), json!(true), json!(false)], "a first open");
    set(&mut app, "preserveWhite", json!(false));
    set(&mut app, "preserveBlack", json!(false));
    set(&mut app, "preserveGrays", json!(true));
    dialogs::confirm(&mut app).unwrap();
    app.run("select.all", json!({})).unwrap();
    app.run("ui.recolorDialog", json!({})).unwrap();
    assert_eq!(preserve(&app), [json!(false), json!(false), json!(true)], "as confirmed");
    set(&mut app, "preserveWhite", json!(true));
    dialogs::cancel(&mut app);
    app.run("ui.recolorDialog", json!({})).unwrap();
    assert_eq!(preserve(&app), [json!(false), json!(false), json!(true)], "Cancel keeps what OK left");
}

/// The rect of the widget drawn `size` big (the wheel is 220 across, its Brightness slider 220 by 22).
fn widget_of_size(h: &egui_kittest::Harness<'_, VectorcraftApp>, size: egui::Vec2) -> Rect {
    h.ctx
        .viewport(|vp| vp.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).find(|w| w.rect.size() == size).map(|w| w.rect))
        .expect("the widget")
}

fn new_hsb(app: &VectorcraftApp) -> Vec<[f32; 3]> {
    field(app, "rows").as_array().unwrap().iter().map(|row| vectorcraft_engine::cmd::color_value(&row["to"]).unwrap().to_hsb()).collect()
}

/// #1733: on the Edit tab, dragging a handle of CMYK art (linked, Complementary) must leave every
/// handle on the wheel. The others turn with it (intended), but they keep their place relative to
/// it: they don't drift while the pointer is held still and don't end up under the dragged one.
#[test]
fn dragging_a_wheel_handle_keeps_the_other_handles_apart() {
    let mut h = edit_harness(&["cmyk 0 100 100 0", "cmyk 100 0 0 0"]);
    let wheel = widget_of_size(&h, vec2(220.0, 220.0));
    let (c, r) = (wheel.center(), 100.0);
    // Each row's new colour: its hue, and where its handle is drawn (as `widgets::harmony_wheel`).
    let handles = |app: &VectorcraftApp| -> Vec<(f32, Pos2)> {
        field(app, "rows")
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                let [hue, s, _] = vectorcraft_engine::cmd::color_value(&row["to"]).unwrap().to_hsb();
                (hue, c + vec2(hue.to_radians().cos(), -hue.to_radians().sin()) * r * s)
            })
            .collect()
    };
    let gap = |a: f32, b: f32| (a - b).rem_euclid(360.0).min((b - a).rem_euclid(360.0));
    let start = handles(h.state());
    // Drag the red handle a quarter turn, a few points at a time.
    h.hover_at(start[0].1);
    h.run_steps(1);
    h.drag_at(start[0].1);
    h.run_steps(1);
    let (mut p, s0) = (start[0].1, start[0].1.distance(c) / r);
    for k in 1..=12 {
        let (a, s) = ((start[0].0 + 6.0 * k as f32).to_radians(), s0 + (0.75 - s0) * k as f32 / 12.0);
        p = c + vec2(a.cos(), -a.sin()) * r * s;
        h.hover_at(p);
        h.run_steps(1);
    }
    let moved = handles(h.state());
    assert!(gap(moved[1].0, start[1].0) > 10.0, "linked: the other handle turned with it (intended): {start:?} -> {moved:?}");
    let brightness = |app: &VectorcraftApp| -> Vec<f32> {
        field(app, "rows").as_array().unwrap().iter().map(|row| vectorcraft_engine::cmd::color_value(&row["to"]).unwrap().to_hsb()[2]).collect()
    };
    let moved_v = brightness(h.state());
    // Hold the pointer still: nothing should change.
    h.run_steps(20);
    let held = handles(h.state());
    let held_v = brightness(h.state());
    assert!(
        moved_v.iter().zip(&held_v).all(|(a, b)| (a - b).abs() < 0.01),
        "the brightness changed while the pointer was held still: {moved_v:?} -> {held_v:?}"
    );
    assert!(gap(held[1].0, moved[1].0) < 1.0, "the other handle turned on its own while the pointer was held still: {moved:?} -> {held:?}");
    assert!(held[0].1.distance(held[1].1) > 18.0, "the other handle went under the dragged one (pointer {p:?}): {held:?}");
    let rows = field(h.state(), "rows");
    h.drop_at(p);
    h.run_steps(1);
    assert_eq!(field(h.state(), "rows"), rows, "releasing keeps the colours the drag ended on");
    // OK keeps the whole drag as one undo step; undo brings the art's colours back.
    let keys = |app: &mut VectorcraftApp| -> Vec<Value> {
        let mut k: Vec<Value> =
            app.session.execute("recolor.colors", &json!({})).unwrap()["colors"].as_array().unwrap().iter().map(|c| c["key"].clone()).collect();
        k.sort_by_key(Value::to_string);
        k
    };
    let app = h.state_mut();
    let undo = app.session.doc().unwrap().history.undo.len();
    dialogs::confirm(app).unwrap();
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
    let mut to: Vec<Value> = rows.as_array().unwrap().iter().map(|r| r["to"].clone()).collect();
    to.sort_by_key(Value::to_string);
    assert_eq!(keys(app), to);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(keys(app), [json!("cmyk 0 100 100 0"), json!("cmyk 100 0 0 0")]);
}

/// A marker dragged past the wheel's edge is fully saturated, every frame and after the release.
#[test]
fn a_marker_dragged_past_the_edge_stays_fully_saturated() {
    let mut h = edit_harness(&["rgb 200 100 100", "rgb 100 200 200"]);
    let wheel = widget_of_size(&h, vec2(220.0, 220.0));
    let (c, r) = (wheel.center(), 100.0);
    let [hue, s, _] = new_hsb(h.state())[0];
    let along = |d: f32| c + vec2(hue.to_radians().cos(), -hue.to_radians().sin()) * d;
    let (at, out) = (along(r * s), along(r + 8.0));
    h.hover_at(at);
    h.run_steps(1);
    h.drag_at(at);
    h.run_steps(1);
    // Outwards a few points at a time, so the drag starts on the marker.
    for k in 1..=10 {
        h.hover_at(along(r * s + (r + 8.0 - r * s) * k as f32 / 10.0));
        h.run_steps(1);
    }
    for _ in 0..3 {
        h.run_steps(1);
        assert_eq!(new_hsb(h.state())[0][1], 1.0, "held past the edge");
    }
    h.drop_at(out);
    h.run_steps(1);
    assert_eq!(new_hsb(h.state())[0][1], 1.0, "released past the edge");
}

/// The Brightness slider dragged to its top gives full brightness, every frame and after the release.
#[test]
fn the_brightness_slider_dragged_to_the_top_stays_there() {
    let mut h = edit_harness(&["rgb 200 100 100", "rgb 100 200 200"]);
    let slider = widget_of_size(&h, vec2(220.0, 22.0));
    let v = new_hsb(h.state())[0][2];
    let at = egui::pos2(slider.left() + 4.0 + v * (slider.width() - 8.0), slider.center().y);
    let top = egui::pos2(slider.right() + 10.0, slider.center().y);
    h.hover_at(at);
    h.run_steps(1);
    h.drag_at(at);
    h.run_steps(1);
    h.hover_at(top);
    for _ in 0..3 {
        h.run_steps(1);
        assert_eq!(new_hsb(h.state())[0][2], 1.0, "held at the top");
    }
    h.drop_at(top);
    h.run_steps(1);
    assert_eq!(new_hsb(h.state())[0][2], 1.0, "released at the top");
}

/// #3416: only the chips scroll; the wheel, heading and actions fit after resize or UI scaling.
#[test]
fn many_edit_colors_keep_the_dialog_on_screen() {
    let mut small_layout = None;
    for count in [5, 20, 200, 400] {
        let colors: Vec<String> = (1..=count).map(|i| format!("#{:06x}", i * 9973)).collect();
        let refs: Vec<&str> = colors.iter().map(String::as_str).collect();
        let mut h = edit_harness(&refs);
        assert_eq!(field(h.state(), "rows").as_array().unwrap().len(), count);
        for (physical, zoom) in [
            (vec2(900.0, 800.0), 1.0),
            (vec2(800.0, 600.0), 1.0),
            (vec2(1280.0, 720.0), 1.25),
            (vec2(1440.0, 900.0), 1.5),
            (vec2(1920.0, 1200.0), 2.0),
            (vec2(900.0, 800.0), 1.0),
        ] {
            h.ctx.set_zoom_factor(zoom);
            for _ in 0..12 {
                h.set_size(physical / zoom);
                h.run_steps(1);
            }
            let screen = h.ctx.content_rect();
            assert_eq!(screen.size(), physical / zoom, "the requested effective screen size");
            let dialog = h.ctx.memory(|m| m.area_rect(egui::Id::unique(("dialog", "recolor")))).unwrap();
            assert!(screen.contains_rect(dialog), "{count} colors, {physical:?} at {zoom}: dialog {dialog:?} in {screen:?}");
            let wheel = widget_of_size(&h, vec2(220.0, 220.0));
            assert!(screen.contains_rect(wheel), "the wheel is accessible");
            let heading = h.get_by_role_and_label(egui::accesskit::Role::Label, "Recolor Artwork").rect();
            assert!(screen.contains_rect(heading), "the heading is accessible");
            for label in ["OK", "Cancel", "Preview"] {
                assert!(screen.contains_rect(h.get_by_label(label).rect()), "{label} is accessible");
            }
            if count <= 20 {
                h.ctx.viewport(|vp| {
                    for w in vp.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).filter(|w| w.rect.size() == vec2(28.0, 20.0)) {
                        assert!(screen.contains_rect(w.rect), "a small palette needs no scrolling");
                    }
                });
            }
            if count <= 20 && physical == vec2(900.0, 800.0) {
                let layout = (dialog, wheel);
                assert_eq!(*small_layout.get_or_insert(layout), layout, "a few chips keep the existing tab height and wheel position");
            }
        }
    }
}

/// Scrolling reaches the last color without moving the wheel or actions; editing still previews,
/// cancels, and confirms as one undo step through actual egui input.
#[test]
fn the_last_edit_color_can_be_selected_edited_and_undone() {
    let colors: Vec<String> = (1..=400).map(|i| format!("#{:06x}", i * 9973)).collect();
    let refs: Vec<&str> = colors.iter().map(String::as_str).collect();
    for confirm in [false, true] {
        let mut h = edit_harness(&refs);
        h.set_size(vec2(800.0, 600.0));
        h.run_steps(12);
        let before = h.state().session.doc().unwrap().doc.clone();
        let undo = h.state().session.doc().unwrap().history.undo.len();
        let wheel = widget_of_size(&h, vec2(220.0, 220.0));
        let chips = |h: &egui_kittest::Harness<'_, VectorcraftApp>| -> Vec<Rect> {
            h.ctx.viewport(|vp| {
                vp.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).filter(|w| w.rect.size() == vec2(28.0, 20.0)).map(|w| w.rect).collect()
            })
        };
        let first = chips(&h)[0];
        assert!(!h.ctx.content_rect().contains_rect(*chips(&h).last().unwrap()), "the last chip starts out of view");
        h.input_mut().events.extend([
            Event::PointerMoved(first.center()),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, -10000.0),
                phase: egui::TouchPhase::Move,
                modifiers: Modifiers::NONE,
                source: egui::MouseWheelSource::Unknown,
            },
        ]);
        h.run_steps(12);
        let last = *chips(&h).last().unwrap();
        assert!(h.ctx.content_rect().contains_rect(last), "the last chip is visible after scrolling: {last:?}");
        assert_eq!(widget_of_size(&h, vec2(220.0, 220.0)), wheel, "scrolling only moves the chips");
        h.input_mut().events.extend(press(last.center(), 1));
        h.run_steps(2);
        assert_eq!(field(h.state(), "__sel"), json!([399]));
        set(h.state_mut(), "linked", json!(false));
        let rows = field(h.state(), "rows");
        let slider = widget_of_size(&h, vec2(220.0, 22.0));
        h.input_mut().events.extend(press(egui::pos2(slider.left() + 4.0 + 0.25 * (slider.width() - 8.0), slider.center().y), 1));
        h.run_steps(2);
        let edited = field(h.state(), "rows");
        assert_ne!(edited[399]["to"], rows[399]["to"], "the last color changed");
        assert_eq!(&edited.as_array().unwrap()[..399], &rows.as_array().unwrap()[..399], "unlinked editing preserves the other colors");
        let after = h.state().session.doc().unwrap().doc.clone();
        assert_ne!(after, before, "the edit previews on the artwork");
        h.get_by_label("Preview").click();
        h.run_steps(2);
        assert_eq!(h.state().session.doc().unwrap().doc, before, "Preview off restores the artwork");
        h.get_by_label("Preview").click();
        h.run_steps(2);
        assert_eq!(h.state().session.doc().unwrap().doc, after, "Preview on restores the edit");
        h.get_by_label(if confirm { "OK" } else { "Cancel" }).click();
        h.run_steps(2);
        assert!(h.state().ui.dialog.is_none());
        if confirm {
            assert_eq!(h.state().session.doc().unwrap().doc, after);
            // Creating 400 objects fills the bounded history; one Undo must still restore all the art.
            h.state_mut().run("edit.undo", json!({})).unwrap();
        } else {
            assert_eq!(h.state().session.doc().unwrap().history.undo.len(), undo);
        }
        assert_eq!(h.state().session.doc().unwrap().doc, before);
    }
}

/// The height of the open Recolor Artwork dialog.
fn dialog_height(h: &egui_kittest::Harness<'_, VectorcraftApp>) -> f32 {
    h.ctx.memory(|m| m.area_rect(egui::Id::unique(("dialog", "recolor")))).unwrap().height()
}

/// Clicks the Edit tab's display button named `name` and lets the dialog settle.
fn show_display(h: &mut egui_kittest::Harness<'_, VectorcraftApp>, name: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::Button, name).click();
    h.run_steps(6);
}

/// The Edit tab opens on the smooth color wheel and its buttons switch to the segmented wheel and to
/// the color bars and back: the dialog keeps its size, and the rows and the selected row stay. The
/// choice stays while the dialog is open; the next opening starts on the smooth wheel again.
#[test]
fn the_edit_tab_shows_a_smooth_or_segmented_wheel_or_color_bars() {
    let mut h = edit_harness(&["#ff0000", "#0000ff", "#00ff00"]);
    set(h.state_mut(), "__sel", json!([1]));
    h.run_steps(4);
    let (rows, height) = (field(h.state(), "rows"), dialog_height(&h));
    let slider = |h: &egui_kittest::Harness<'_, VectorcraftApp>, name: &str| h.query_by_role_and_label(egui::accesskit::Role::Slider, name).is_some();
    let bar = |h: &egui_kittest::Harness<'_, VectorcraftApp>, name: &str| h.query_by_role_and_label(egui::accesskit::Role::Button, name).is_some();
    assert!(slider(&h, "Color wheel") && !slider(&h, "Segmented color wheel"), "the smooth wheel first");
    assert!(!bar(&h, "Color 1"));
    show_display(&mut h, "Show segmented color wheel");
    assert_eq!(field(h.state(), "display"), json!("segmented"));
    assert!(slider(&h, "Segmented color wheel") && !slider(&h, "Color wheel"));
    assert_eq!(widget_of_size(&h, vec2(220.0, 220.0)).size(), vec2(220.0, 220.0), "the same size of wheel");
    assert!(slider(&h, "Brightness"));
    show_display(&mut h, "Show color bars");
    assert_eq!(field(h.state(), "display"), json!("bars"));
    assert!(!slider(&h, "Color wheel") && !slider(&h, "Segmented color wheel"));
    assert!(["Color 1", "Color 2", "Color 3"].iter().all(|n| bar(&h, n)), "a bar per color");
    assert_eq!(dialog_height(&h), height, "the bars keep the dialog's height");
    show_display(&mut h, "Show smooth color wheel");
    assert!(slider(&h, "Color wheel"));
    assert_eq!(dialog_height(&h), height);
    assert_eq!(field(h.state(), "rows"), rows, "switching changes no color");
    assert_eq!(field(h.state(), "__sel"), json!([1]), "and keeps the selected row");
    show_display(&mut h, "Show color bars");
    set(h.state_mut(), "tab", json!("assign"));
    h.run_steps(3);
    set(h.state_mut(), "tab", json!("edit"));
    h.run_steps(3);
    assert!(bar(&h, "Color 1"), "the choice stays while the dialog is open");
    let app = h.state_mut();
    dialogs::cancel(app);
    app.run("select.all", json!({})).unwrap();
    app.run("ui.recolorDialog", json!({})).unwrap();
    assert_eq!(field(app, "display"), Value::Null, "a new opening starts on the smooth wheel");
}

/// The segmented wheel's patches: 24 hues, centered on red, by 5 saturations, at the brightness asked.
#[test]
fn the_segmented_wheel_has_24_hues_by_5_saturations() {
    let seg = |h, s, v| crate::widgets::segment_color(h, s, v).to_hsb();
    let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3);
    assert!(near(seg(7.0, 0.95, 1.0), [0.0, 0.9, 1.0]), "{:?}", seg(7.0, 0.95, 1.0));
    assert!(near(seg(8.0, 0.0, 0.5), [15.0, 0.1, 0.5]), "{:?}", seg(8.0, 0.0, 0.5));
    assert!(near(seg(355.0, 1.0, 1.0), [0.0, 0.9, 1.0]), "{:?}", seg(355.0, 1.0, 1.0));
    assert!(near(seg(120.0, 0.5, 1.0), [120.0, 0.5, 1.0]), "{:?}", seg(120.0, 0.5, 1.0));
    let mut patches = std::collections::BTreeSet::new();
    for h in 0..360 {
        for s in 0..=100 {
            patches.insert(ColorKey::of(&crate::widgets::segment_color(h as f32, s as f32 / 100.0, 1.0)).to_string());
        }
    }
    assert_eq!(patches.len(), 24 * 5);
}

/// On the segmented wheel a marker moves as freely as on the smooth one, past patch edges and
/// without snapping to a patch; linked colors turn with it from where the drag started, an excluded
/// row keeps its color, and the smooth wheel shows the result. Its Brightness slider works too.
#[test]
fn a_segmented_wheel_marker_moves_freely() {
    let mut h = edit_harness(&["rgb 200 100 100", "rgb 100 200 200", "rgb 20 160 20"]);
    let mut rows = field(h.state(), "rows");
    let green = (0..3).find(|i| rows[*i]["from"] == json!(["rgb 20 160 20"])).unwrap();
    let red = (0..3).find(|i| rows[*i]["from"] == json!(["rgb 200 100 100"])).unwrap();
    rows[green]["exclude"] = json!(true);
    set(h.state_mut(), "rows", rows);
    set(h.state_mut(), "__sel", json!([red]));
    h.run_steps(4);
    show_display(&mut h, "Show segmented color wheel");
    let start = new_hsb(h.state());
    let wheel = widget_of_size(&h, vec2(220.0, 220.0));
    let (c, r) = (wheel.center(), 100.0);
    let at = |hue: f32, s: f32| c + vec2(hue.to_radians().cos(), -hue.to_radians().sin()) * r * s;
    let [hue, s, _] = start[red];
    h.hover_at(at(hue, s));
    h.run_steps(1);
    h.drag_at(at(hue, s));
    h.run_steps(1);
    // 37° on in small steps, across two patch edges, ending inside a patch away from its center.
    for k in 1..=10 {
        h.hover_at(at(hue + 3.7 * k as f32, s));
        h.run_steps(1);
    }
    h.drop_at(at(hue + 37.0, s));
    h.run_steps(2);
    let moved = new_hsb(h.state());
    let gap = |a: f32, b: f32| (a - b).rem_euclid(360.0).min((b - a).rem_euclid(360.0));
    let want = (hue + 37.0).rem_euclid(360.0);
    assert!(gap(moved[red][0], want) < 1.0, "the marker follows the pointer: {} -> {}", hue, moved[red][0]);
    assert!(gap(moved[red][0], (want / 15.0).round() * 15.0) > 2.0, "not snapped to a patch center: {}", moved[red][0]);
    let other = 3 - red - green;
    assert!((gap(moved[other][0], start[other][0]) - 37.0).abs() < 1.5, "linked: {:?} -> {:?}", start[other], moved[other]);
    assert_eq!(field(h.state(), "rows")[green]["to"], json!("rgb 20 160 20"), "the excluded row stays");
    let rows = field(h.state(), "rows");
    show_display(&mut h, "Show smooth color wheel");
    assert_eq!(field(h.state(), "rows"), rows, "the smooth wheel shows the same colors");
    show_display(&mut h, "Show segmented color wheel");
    click_slider(&mut h, "Brightness", 0.25);
    assert!((new_hsb(h.state())[red][2] - 0.25).abs() < 0.02, "the Brightness slider: {:?}", new_hsb(h.state()));
}

/// The segmented hue and brightness wheel's patches: 24 hues by 5 brightness steps, at the
/// saturation asked; the innermost ring is the darkest.
#[test]
fn the_segmented_brightness_wheel_has_24_hues_by_5_brightnesses() {
    let bright = crate::widgets::WheelAxes::HueBrightness;
    let seg = |h, v, s| crate::widgets::segment_color_on(bright, h, v, s).to_hsb();
    let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3);
    assert!(near(seg(7.0, 0.95, 1.0), [0.0, 1.0, 0.9]), "{:?}", seg(7.0, 0.95, 1.0));
    assert!(near(seg(8.0, 0.0, 0.5), [15.0, 0.5, 0.1]), "{:?}", seg(8.0, 0.0, 0.5));
    assert!(near(seg(120.0, 0.5, 0.25), [120.0, 0.25, 0.5]), "{:?}", seg(120.0, 0.5, 0.25));
    let mut patches = std::collections::BTreeSet::new();
    for h in 0..360 {
        for v in 0..=100 {
            patches.insert(ColorKey::of(&crate::widgets::segment_color_on(bright, h as f32, v as f32 / 100.0, 1.0)).to_string());
        }
    }
    assert_eq!(patches.len(), 24 * 5);
}

/// The button under the wheel tools switches the wheel to hue and brightness: its name says what a
/// click shows next, the slider under the wheel becomes a Saturation slider, and switching changes
/// no color, previews nothing and records no undo step. The choice holds for the segmented wheel and
/// across the color bars, where the button is off; the next opening shows hue and saturation again.
#[test]
fn the_brightness_and_hue_wheel_swaps_the_wheel_axis_and_the_slider() {
    let mut h = edit_harness(&["#ff0000", "#0000ff", "#00ff00"]);
    let (rows, height) = (field(h.state(), "rows"), dialog_height(&h));
    let doc = h.state().session.doc().unwrap().doc.clone();
    let undo = h.state().session.doc().unwrap().history.undo.len();
    let slider = |h: &egui_kittest::Harness<'_, VectorcraftApp>, name: &str| h.query_by_role_and_label(egui::accesskit::Role::Slider, name).is_some();
    assert_eq!(field(h.state(), "wheelAxes"), Value::Null, "hue and saturation first");
    assert!(slider(&h, "Brightness") && !slider(&h, "Saturation"));
    click_button(&mut h, "Show brightness and hue on wheel");
    assert_eq!(field(h.state(), "wheelAxes"), json!("hueBrightness"));
    assert!(slider(&h, "Saturation") && !slider(&h, "Brightness"), "the slider edits the component the wheel leaves out");
    assert!(h.query_by_role_and_label(egui::accesskit::Role::Button, "Show saturation and hue on wheel").is_some(), "the button names the way back");
    assert_eq!(field(h.state(), "rows"), rows, "switching changes no color");
    assert_eq!(h.state().session.doc().unwrap().doc, doc, "nor the art");
    assert_eq!(h.state().session.doc().unwrap().history.undo.len(), undo);
    show_display(&mut h, "Show segmented color wheel");
    assert_eq!(field(h.state(), "wheelAxes"), json!("hueBrightness"), "the segmented wheel keeps the choice");
    assert!(slider(&h, "Saturation"));
    show_display(&mut h, "Show color bars");
    let button = h.get_by_role_and_label(egui::accesskit::Role::Button, "Show saturation and hue on wheel");
    assert!(button.accesskit_node().is_disabled(), "the button is for the wheels");
    assert_eq!(dialog_height(&h), height, "the button keeps the dialog's height");
    show_display(&mut h, "Show smooth color wheel");
    assert!(slider(&h, "Saturation"), "the bars keep the choice");
    click_button(&mut h, "Show saturation and hue on wheel");
    assert_eq!(field(h.state(), "wheelAxes"), json!("hueSaturation"));
    assert!(slider(&h, "Brightness") && !slider(&h, "Saturation"));
    assert_eq!(field(h.state(), "rows"), rows);
    assert_eq!(dialog_height(&h), height);
    set(h.state_mut(), "wheelAxes", json!("sideways"));
    h.run_steps(3);
    assert!(slider(&h, "Brightness"), "an unknown value shows hue and saturation");
    set(h.state_mut(), "wheelAxes", json!("hueBrightness"));
    h.run_steps(3);
    let app = h.state_mut();
    dialogs::cancel(app);
    app.run("select.all", json!({})).unwrap();
    app.run("ui.recolorDialog", json!({})).unwrap();
    assert_eq!(field(app, "wheelAxes"), Value::Null, "a new opening shows hue and saturation");
}

/// On the hue and brightness wheel a marker sits at its color's brightness, and a drag sets hue and
/// brightness and keeps saturation; linked colors turn and darken with it. The art previews the
/// drag, Cancel restores it and OK keeps it as one undo step. The Saturation slider sets the
/// selected color's saturation and keeps its brightness.
#[test]
fn a_marker_on_the_brightness_wheel_sets_hue_and_brightness() {
    for confirm in [false, true] {
        let mut h = edit_harness(&["rgb 200 100 100", "rgb 100 200 200"]);
        set(h.state_mut(), "__sel", json!([0]));
        h.run_steps(4);
        let before = h.state().session.doc().unwrap().doc.clone();
        let undo = h.state().session.doc().unwrap().history.undo.len();
        click_button(&mut h, "Show brightness and hue on wheel");
        let start = new_hsb(h.state());
        let [hue, s, v] = start[0];
        // Around by 30° and in to 40% brightness, a few points at a time from the marker.
        let from = wheel_at(&h, hue, v);
        h.hover_at(from);
        h.run_steps(1);
        h.drag_at(from);
        h.run_steps(1);
        let c = widget_of_size(&h, vec2(220.0, 220.0)).center();
        let to = |k: f32| {
            let a = (hue + 30.0 * k).to_radians();
            c + vec2(a.cos(), -a.sin()) * 100.0 * (v + (0.4 - v) * k)
        };
        let end = to(1.0);
        for k in 1..=10 {
            let p = to(k as f32 / 10.0);
            h.hover_at(p);
            h.run_steps(1);
        }
        h.drop_at(end);
        h.run_steps(2);
        let moved = new_hsb(h.state());
        let gap = |a: f32, b: f32| (a - b).rem_euclid(360.0).min((b - a).rem_euclid(360.0));
        assert!(gap(moved[0][0], hue + 30.0) < 1.0, "hue: {start:?} -> {moved:?}");
        assert!((moved[0][2] - 0.4).abs() < 0.02, "brightness: {start:?} -> {moved:?}");
        assert!((moved[0][1] - s).abs() < 0.02, "saturation stays: {start:?} -> {moved:?}");
        assert!(gap(moved[1][0], start[1][0] + 30.0) < 1.5, "linked, it turns: {start:?} -> {moved:?}");
        assert!((moved[1][2] - (start[1][2] - (v - 0.4))).abs() < 0.02, "and darkens alike: {start:?} -> {moved:?}");
        click_slider(&mut h, "Saturation", 0.25);
        let slid = new_hsb(h.state());
        assert!((slid[0][1] - 0.25).abs() < 0.02, "the Saturation slider: {slid:?}");
        assert!((slid[0][2] - 0.4).abs() < 0.02, "keeps the brightness: {slid:?}");
        let after = h.state().session.doc().unwrap().doc.clone();
        assert_ne!(after, before, "the art previews the edit");
        h.get_by_label(if confirm { "OK" } else { "Cancel" }).click();
        h.run_steps(2);
        assert!(h.state().ui.dialog.is_none());
        if confirm {
            assert_eq!(h.state().session.doc().unwrap().doc, after);
            assert_eq!(h.state().session.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
            h.state_mut().run("edit.undo", json!({})).unwrap();
        } else {
            assert_eq!(h.state().session.doc().unwrap().history.undo.len(), undo, "Cancel records no undo step");
        }
        assert_eq!(h.state().session.doc().unwrap().doc, before);
    }
}

/// On the hue and brightness wheels the Add Color tool adds the hue and brightness clicked at the
/// base color's saturation, the patch's color on the segmented wheel.
#[test]
fn the_add_color_tool_on_the_brightness_wheel_adds_the_brightness_clicked() {
    let mut h = edit_harness(&["rgb 200 100 100", "rgb 100 200 200"]);
    set(h.state_mut(), "__sel", json!([0]));
    h.run_steps(4);
    let s = new_hsb(h.state())[0][1];
    click_button(&mut h, "Show brightness and hue on wheel");
    click_button(&mut h, "Add Color Tool");
    let at = wheel_at(&h, 120.0, 0.5);
    h.input_mut().events.extend(press(at, 1));
    h.run_steps(3);
    let [hue, sat, b] = new_hsb(h.state())[2];
    assert!((hue - 120.0).abs() < 1.0 && (sat - s).abs() < 0.01 && (b - 0.5).abs() < 0.01, "{:?}", [hue, sat, b]);
    assert_eq!(field(h.state(), "__sel"), json!([2]));
    // The new color is now the base color; select the first one again.
    set(h.state_mut(), "__sel", json!([0]));
    h.run_steps(3);
    show_display(&mut h, "Show segmented color wheel");
    let patch = wheel_at(&h, 187.0, 0.95);
    h.input_mut().events.extend(press(patch, 1));
    h.run_steps(3);
    let [hue, sat, b] = *new_hsb(h.state()).last().unwrap();
    assert!((hue - 180.0).abs() < 1.0 && (sat - s).abs() < 0.01 && (b - 0.9).abs() < 0.01, "the patch's color: {:?}", [hue, sat, b]);
}

/// The rows as `(current colors, new color)` pairs, sorted.
fn row_pairs(app: &VectorcraftApp) -> Vec<String> {
    let mut v: Vec<String> = field(app, "rows").as_array().unwrap().iter().map(|r| format!("{} {}", r["from"], r["to"])).collect();
    v.sort();
    v
}

/// The color bars: one per recolored row (none for an excluded row), the last one selected with a
/// click and edited with the H slider, then dragged to the top, which moves its row and changes no
/// color. Preview shows the edit, Cancel restores the art and OK keeps it as one undo step.
#[test]
fn color_bars_select_edit_and_reorder_the_rows() {
    for confirm in [false, true] {
        let mut h = edit_harness(&["#ff0000", "#0000ff", "#00ff00", "#ffff00"]);
        let before = h.state().session.doc().unwrap().doc.clone();
        let undo = h.state().session.doc().unwrap().history.undo.len();
        let mut rows = field(h.state(), "rows");
        rows[1]["exclude"] = json!(true);
        set(h.state_mut(), "rows", rows);
        set(h.state_mut(), "linked", json!(false));
        set(h.state_mut(), "sliderMode", json!("hsb"));
        show_display(&mut h, "Show color bars");
        let bar = |h: &egui_kittest::Harness<'_, VectorcraftApp>, name: &str| {
            h.query_by_role_and_label(egui::accesskit::Role::Button, name).map(|n| n.rect())
        };
        assert!(bar(&h, "Color 2").is_none(), "no bar for the excluded row");
        let first = bar(&h, "Color 1").expect("a bar per recolored row");
        let last = bar(&h, "Color 4").expect("a bar per recolored row");
        assert!(bar(&h, "Color 3").is_some_and(|r| r.top() > first.bottom() && r.bottom() < last.top()), "in row order");
        h.input_mut().events.extend(press(last.center(), 1));
        h.run_steps(3);
        assert_eq!(field(h.state(), "__sel"), json!([3]), "a click selects the bar's row");
        let unedited = field(h.state(), "rows");
        click_slider(&mut h, "H", 0.5);
        let edited = field(h.state(), "rows");
        assert_ne!(edited[3]["to"], unedited[3]["to"], "the H slider edits the selected bar's color");
        assert_eq!(&edited.as_array().unwrap()[..3], &unedited.as_array().unwrap()[..3], "and only that one");
        let previewed = h.state().session.doc().unwrap().doc.clone();
        assert_ne!(previewed, before, "the art previews the edit");
        let pairs = row_pairs(h.state());
        let last = bar(&h, "Color 4").unwrap();
        h.hover_at(last.center());
        h.run_steps(1);
        h.drag_at(last.center());
        h.run_steps(1);
        let top = egui::pos2(last.center().x, first.top() + 2.0);
        for k in 1..=8 {
            h.hover_at(last.center().lerp(top, k as f32 / 8.0));
            h.run_steps(1);
        }
        h.drop_at(top);
        h.run_steps(3);
        let rows = field(h.state(), "rows");
        assert_eq!(rows[0], edited[3], "the dragged row moved to the top: {rows}");
        assert_eq!(rows.as_array().unwrap()[1..], [edited[0].clone(), edited[1].clone(), edited[2].clone()], "the others shift down");
        assert_eq!(row_pairs(h.state()), pairs, "moving a bar changes no color");
        assert_eq!(field(h.state(), "__sel"), json!([0]), "the moved row stays selected");
        assert_eq!(h.state().session.doc().unwrap().doc, previewed, "and the art keeps the same colors");
        h.get_by_label(if confirm { "OK" } else { "Cancel" }).click();
        h.run_steps(2);
        assert!(h.state().ui.dialog.is_none());
        let app = h.state();
        if confirm {
            assert_eq!(app.session.doc().unwrap().doc, previewed, "OK keeps the edit");
            assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "as one undo step");
        } else {
            assert_eq!(app.session.doc().unwrap().doc, before, "Cancel restores the art");
            assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);
        }
    }
}

/// Many color bars scroll within the wheel's height, the dialog keeps its height, and the last bar
/// can be reached and selected; with every row excluded the bars area says there is nothing to
/// recolor.
#[test]
fn color_bars_scroll_and_show_when_nothing_is_recolored() {
    let colors: Vec<String> = (1..=40).map(|i| format!("#{:06x}", i * 9973)).collect();
    let refs: Vec<&str> = colors.iter().map(String::as_str).collect();
    let mut h = edit_harness(&refs);
    let height = dialog_height(&h);
    show_display(&mut h, "Show color bars");
    assert_eq!(dialog_height(&h), height, "40 bars keep the dialog's height");
    let first = h.get_by_role_and_label(egui::accesskit::Role::Button, "Color 1").rect();
    let wheel_bottom = first.top() + 251.0;
    let last = |h: &egui_kittest::Harness<'_, VectorcraftApp>| h.get_by_role_and_label(egui::accesskit::Role::Button, "Color 40").rect();
    assert!(last(&h).top() > wheel_bottom, "the last bar starts out of view: {:?}", last(&h));
    h.input_mut().events.extend([
        Event::PointerMoved(first.center()),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -10000.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
            source: egui::MouseWheelSource::Unknown,
        },
    ]);
    h.run_steps(12);
    let shown = last(&h);
    assert!(shown.bottom() <= wheel_bottom + 1.0, "the last bar scrolls into view: {shown:?}");
    assert_eq!(dialog_height(&h), height);
    h.input_mut().events.extend(press(shown.center(), 1));
    h.run_steps(3);
    assert_eq!(field(h.state(), "__sel"), json!([39]));
    let mut rows = field(h.state(), "rows");
    for r in rows.as_array_mut().unwrap() {
        r["exclude"] = json!(true);
    }
    set(h.state_mut(), "rows", rows);
    h.run_steps(4);
    assert!(h.query_by_role_and_label(egui::accesskit::Role::Button, "Color 1").is_none());
    let _ = h.get_by_label("No colors to recolor (preserved colors are left out).");
}

/// Where the Edit tab's wheel draws a marker for hue `hue` and saturation `s` (or brightness, on
/// the hue and brightness wheel).
fn wheel_at(h: &egui_kittest::Harness<'_, VectorcraftApp>, hue: f32, s: f32) -> Pos2 {
    let c = widget_of_size(h, vec2(220.0, 220.0)).center();
    c + vec2(hue.to_radians().cos(), -hue.to_radians().sin()) * 100.0 * s
}

/// Clicks the Edit tab's button named `name` and lets the dialog settle.
fn click_button(h: &mut egui_kittest::Harness<'_, VectorcraftApp>, name: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::Button, name).click();
    h.run_steps(3);
}

/// The Add Color tool: a click on the smooth wheel adds the hue and saturation under it, at the
/// base color's brightness, as a row with no current colors that becomes the selected row; the art
/// keeps its colors, and the Assign tab shows the row without current colors. A second click on the
/// button turns the tool off, and a click on the wheel then adds nothing.
#[test]
fn the_add_color_tool_adds_a_row_with_no_current_colors() {
    let mut h = edit_harness(&["rgb 200 100 100", "rgb 100 200 200"]);
    set(h.state_mut(), "__sel", json!([0]));
    h.run_steps(4);
    let (fills_before, v) = (fills(h.state()), new_hsb(h.state())[0][2]);
    click_button(&mut h, "Add Color Tool");
    assert_eq!(field(h.state(), "wheelTool"), json!("add"));
    let at = wheel_at(&h, 120.0, 0.5);
    h.input_mut().events.extend(press(at, 1));
    h.run_steps(3);
    let rows = field(h.state(), "rows");
    assert_eq!(rows.as_array().unwrap().len(), 3, "a new row: {rows}");
    assert_eq!(rows[2]["from"], json!([]), "with no current colors");
    assert_eq!(rows[2]["added"], json!(true), "marked as an added color");
    let [hue, s, b] = new_hsb(h.state())[2];
    assert!((hue - 120.0).abs() < 1.0 && (s - 0.5).abs() < 0.01 && (b - v).abs() < 0.01, "{:?}", [hue, s, b]);
    assert_eq!(field(h.state(), "__sel"), json!([2]), "the new color is selected");
    assert_eq!(fills(h.state()), fills_before, "a color with no current colors recolors no art");
    // A second click adds another color; the tool stays on.
    let next = wheel_at(&h, 240.0, 0.75);
    h.input_mut().events.extend(press(next, 1));
    h.run_steps(3);
    assert_eq!(field(h.state(), "rows").as_array().unwrap().len(), 4);
    click_button(&mut h, "Add Color Tool");
    assert_eq!(field(h.state(), "wheelTool"), Value::Null, "a second click turns the tool off");
    h.input_mut().events.extend(press(at, 1));
    h.run_steps(3);
    assert_eq!(field(h.state(), "rows").as_array().unwrap().len(), 4, "without the tool a click adds nothing");
    assert_eq!(field(h.state(), "__sel"), json!([2]), "it selects the marker clicked");
    set(h.state_mut(), "tab", json!("assign"));
    h.run_steps(3);
    let _ = h.get_by_label("Current Colors (2)");
    assert!(h.query_by_label("Color 4").is_some(), "the Assign tab has a row for each added color");
    let app = h.state_mut();
    let undo = app.session.doc().unwrap().history.undo.len();
    dialogs::confirm(app).unwrap();
    assert_eq!(fills(app), fills_before, "OK leaves the art's colors");
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo, "and records no undo step");
}

/// On the segmented wheel the Add Color tool adds the patch's color. Opened on a color group without
/// art, OK adds the added color to the group, as one undo step.
#[test]
fn an_added_color_joins_the_color_group() {
    let app = app();
    let before = group_keys(&app, "Brights");
    let mut h = assign_harness(app, json!({"group": "Brights"}));
    set(h.state_mut(), "tab", json!("edit"));
    h.run_steps(4);
    show_display(&mut h, "Show segmented color wheel");
    let v = new_hsb(h.state())[0][2];
    click_button(&mut h, "Add Color Tool");
    let patch = wheel_at(&h, 187.0, 0.95);
    h.input_mut().events.extend(press(patch, 1));
    h.run_steps(3);
    let added = field(h.state(), "rows").as_array().unwrap().last().unwrap()["to"].clone();
    let [hue, s, b] = vectorcraft_engine::cmd::color_value(&added).unwrap().to_hsb();
    assert!((hue - 180.0).abs() < 1.0 && (s - 0.9).abs() < 0.01 && (b - v).abs() < 0.01, "the patch's color: {:?}", [hue, s, b]);
    let app = h.state_mut();
    let undo = app.session.doc().unwrap().history.undo.len();
    dialogs::confirm(app).unwrap();
    let mut want = before.clone();
    want.push(added);
    assert_eq!(group_keys(app, "Brights"), want);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
}

/// The Remove Color tool: a click on the base color's marker removes nothing; a click on another
/// marker leaves its row of art's colors with no new color, so OK leaves that art's colors as they
/// were while the other rows recolor theirs. The tools are off for the color bars.
#[test]
fn the_remove_color_tool_removes_a_marker_but_not_the_base_colors() {
    let mut h = edit_harness(&["rgb 200 100 100", "rgb 100 200 200", "rgb 20 160 20"]);
    let fills_before = fills(h.state());
    let mut rows = field(h.state(), "rows");
    let tos = ["rgb 255 0 0", "rgb 0 255 0", "rgb 0 0 255"];
    for (r, to) in rows.as_array_mut().unwrap().iter_mut().zip(tos) {
        r["to"] = json!(to);
    }
    set(h.state_mut(), "rows", rows.clone());
    set(h.state_mut(), "linked", json!(false));
    set(h.state_mut(), "__sel", json!([0]));
    h.run_steps(4);
    click_button(&mut h, "Remove Color Tool");
    assert_eq!(field(h.state(), "wheelTool"), json!("remove"));
    let (red, green) = (wheel_at(&h, 0.0, 1.0), wheel_at(&h, 120.0, 1.0));
    h.input_mut().events.extend(press(red, 1));
    h.run_steps(3);
    assert_eq!(field(h.state(), "rows"), rows, "the base color stays");
    h.input_mut().events.extend(press(green, 1));
    h.run_steps(3);
    let now = field(h.state(), "rows");
    assert_eq!(now.as_array().unwrap().len(), 3, "the row of art's colors stays");
    assert_eq!((&now[0], &now[1]["to"], &now[2]), (&rows[0], &Value::Null, &rows[2]), "with no new color: {now}");
    assert_eq!(field(h.state(), "__sel"), json!([0]), "the base color stays selected");
    show_display(&mut h, "Show color bars");
    let tool = h.get_by_role_and_label(egui::accesskit::Role::Button, "Remove Color Tool");
    assert!(tool.accesskit_node().is_disabled(), "the tools are for the wheels");
    let kept = rows[1]["from"][0].clone();
    let app = h.state_mut();
    dialogs::confirm(app).unwrap();
    let after = fills(app);
    for (b, a) in fills_before.iter().zip(&after) {
        let was = json!(ColorKey::of(&b.unwrap()).to_string());
        if was == kept {
            assert_eq!(a, b, "the art of the removed color keeps it");
        } else {
            assert_ne!(a, b, "the other art takes its new color");
        }
    }
}

/// Right-clicks the color bar named `name`.
fn right_click_bar(h: &mut egui_kittest::Harness<'_, VectorcraftApp>, name: &str) {
    let at = h.get_by_role_and_label(egui::accesskit::Role::Button, name).rect().center();
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    h.event(Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

/// A color bar's right-click menu has Remove Color: off for the base color's bar; on an added
/// color's bar it removes the row, and on a bar of art's colors it leaves the row with no new color
/// and no bar.
#[test]
fn a_color_bars_menu_removes_its_color() {
    let mut h = edit_harness(&["#ff0000", "#0000ff", "#00ff00"]);
    let mut rows = field(h.state(), "rows");
    rows.as_array_mut().unwrap().push(json!({"from": [], "to": "rgb 10 20 30", "added": true}));
    set(h.state_mut(), "rows", rows.clone());
    set(h.state_mut(), "__sel", json!([0]));
    h.run_steps(4);
    show_display(&mut h, "Show color bars");
    right_click_bar(&mut h, "Color 1");
    assert!(h.get_by_label("   Remove Color").accesskit_node().is_disabled(), "the base color can't be removed");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    right_click_bar(&mut h, "Color 4");
    h.get_by_label("   Remove Color").click();
    h.run_steps(3);
    assert_eq!(field(h.state(), "rows").as_array().unwrap()[..], rows.as_array().unwrap()[..3], "the added row goes");
    right_click_bar(&mut h, "Color 2");
    h.get_by_label("   Remove Color").click();
    h.run_steps(3);
    let now = field(h.state(), "rows");
    assert_eq!((&now[1]["from"], &now[1]["to"]), (&rows[1]["from"], &Value::Null), "{now}");
    assert!(h.query_by_role_and_label(egui::accesskit::Role::Button, "Color 2").is_none(), "its bar goes");
    assert_eq!(field(h.state(), "__sel"), json!([0]));
}

/// Right-clicks the point `at`.
fn right_click_at(h: &mut egui_kittest::Harness<'_, VectorcraftApp>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    h.event(Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

/// The smaller angle between hues `a` and `b`, in degrees.
fn hue_gap(a: f32, b: f32) -> f32 {
    (a - b).rem_euclid(360.0).min((b - a).rem_euclid(360.0))
}

/// A marker's right-click menu has Set As Base Color, Select Shade, Color Picker and Remove Color.
/// Set As Base Color marks the row as the base, whose menu then has neither Set As Base Color nor
/// Remove Color; with another color selected, a harmony rule builds from the base color, and OK
/// recolors the art.
#[test]
fn a_markers_menu_sets_the_base_color_that_the_rules_build_from() {
    let mut h = edit_harness(&["#ff0000", "#0000ff", "#00ff00"]);
    let hsb = new_hsb(h.state());
    let at = wheel_at(&h, hsb[1][0], hsb[1][1]);
    right_click_at(&mut h, at);
    for item in ["   Set As Base Color", "   Select Shade", "   Color Picker", "   Remove Color"] {
        assert!(h.query_by_label(item).is_some(), "{item}");
    }
    h.get_by_label("   Set As Base Color").click();
    h.run_steps(3);
    let rows = field(h.state(), "rows");
    assert_eq!(rows[1]["base"], json!(true), "{rows}");
    assert!(rows[0].get("base").is_none() && rows[2].get("base").is_none());
    right_click_at(&mut h, at);
    assert!(h.get_by_label("   Set As Base Color").accesskit_node().is_disabled(), "it is the base already");
    assert!(h.get_by_label("   Remove Color").accesskit_node().is_disabled(), "the base color can't be removed");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    // The selected color is another one; the rule still builds from the base.
    set(h.state_mut(), "__sel", json!([0]));
    h.run_steps(3);
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Harmony Rules:").click();
    h.run_steps(2);
    h.get_by_label("Triad").click();
    h.run_steps(3);
    let now = new_hsb(h.state());
    assert!(hue_gap(now[1][0], hsb[1][0]) < 1.0, "the base color keeps its hue: {hsb:?} -> {now:?}");
    assert!(hue_gap(now[0][0], now[1][0]) > 110.0 && hue_gap(now[2][0], now[1][0]) > 110.0, "the others are a triad from it: {now:?}");
    assert_eq!(field(h.state(), "rows")[1]["base"], json!(true), "the rule keeps the base mark");
    let before = fills(h.state());
    let app = h.state_mut();
    dialogs::confirm(app).unwrap();
    assert_eq!(fills(app), before, "OK keeps the previewed colors");
}

/// Select Shade on a marker's menu shows a grid of the color's hue; a click on a cell gives the color
/// that saturation and brightness, keeping its hue, and closes the menu. Unlinked, the other colors
/// stay; linked, they move as a drag would move them.
#[test]
fn select_shade_gives_a_marker_the_shade_clicked() {
    for linked in [false, true] {
        let mut h = edit_harness(&["rgb 200 100 100", "rgb 100 200 200"]);
        set(h.state_mut(), "linked", json!(linked));
        h.run_steps(3);
        let hsb = new_hsb(h.state());
        let at = wheel_at(&h, hsb[0][0], hsb[0][1]);
        right_click_at(&mut h, at);
        h.get_by_label("   Select Shade").click();
        h.run_steps(2);
        let cell = "Saturation 50%, Brightness 57%";
        h.get_by_label(cell).click();
        h.run_steps(3);
        let now = new_hsb(h.state());
        assert!(hue_gap(now[0][0], hsb[0][0]) < 1.0 && (now[0][1] - 0.5).abs() < 0.01 && (now[0][2] - 4.0 / 7.0).abs() < 0.01, "{now:?}");
        let other_moved = (now[1][2] - hsb[1][2]).abs() > 0.01;
        assert_eq!(other_moved, linked, "linked {linked}: {hsb:?} -> {now:?}");
        assert!(h.query_by_label(cell).is_none(), "the menu closes");
        assert_eq!(field(h.state(), "__sel"), json!([0]));
    }
}

/// Color Picker on a marker's menu opens the Color Picker for that color.
#[test]
fn a_markers_menu_opens_the_color_picker() {
    let mut h = edit_harness(&["rgb 200 100 100", "rgb 100 200 200"]);
    let hsb = new_hsb(h.state());
    let at = wheel_at(&h, hsb[1][0], hsb[1][1]);
    right_click_at(&mut h, at);
    h.get_by_label("   Color Picker").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.dialog.as_ref().map(|d| d.kind.as_str()), Some("colorPicker"));
}

/// A right-click on the wheel away from the markers offers Add New Color, which adds the hue and
/// saturation there as a new color. A color added on the line from the center to a marker moves
/// with that marker when the colors are unlinked, and the others stay.
#[test]
fn add_new_color_on_a_markers_line_moves_with_that_marker() {
    let mut h = edit_harness(&["rgb 200 100 100", "rgb 100 200 200"]);
    set(h.state_mut(), "linked", json!(false));
    set(h.state_mut(), "__sel", json!([0]));
    h.run_steps(4);
    let hsb = new_hsb(h.state());
    let at = wheel_at(&h, 90.0, 0.6);
    right_click_at(&mut h, at);
    assert!(h.query_by_label("   Set As Base Color").is_none(), "no marker there");
    h.get_by_label("   Add New Color").click();
    h.run_steps(3);
    let rows = field(h.state(), "rows");
    assert_eq!(rows.as_array().unwrap().len(), 3, "{rows}");
    assert_eq!((&rows[2]["from"], &rows[2]["added"]), (&json!([]), &json!(true)));
    assert!(rows[2].get("follows").is_none(), "off every marker's line");
    let [hue, s, v] = new_hsb(h.state())[2];
    assert!((hue - 90.0).abs() < 1.0 && (s - 0.6).abs() < 0.01 && (v - hsb[0][2]).abs() < 0.01, "{:?}", [hue, s, v]);
    assert_eq!(field(h.state(), "__sel"), json!([2]));
    // Halfway along the second color's line.
    let on_line = wheel_at(&h, hsb[1][0], hsb[1][1] / 2.0);
    right_click_at(&mut h, on_line);
    h.get_by_label("   Add New Color").click();
    h.run_steps(3);
    let rows = field(h.state(), "rows");
    assert_eq!(rows.as_array().unwrap().len(), 4);
    assert!(rows[1]["tie"].is_u64() && rows[3]["follows"] == rows[1]["tie"], "{rows}");
    // Drag the second color's marker a little way round: the color on its line turns with it.
    let before = new_hsb(h.state());
    let start = wheel_at(&h, before[1][0], before[1][1]);
    h.hover_at(start);
    h.run_steps(1);
    h.drag_at(start);
    h.run_steps(1);
    let mut p = start;
    for k in 1..=6 {
        p = wheel_at(&h, before[1][0] + 5.0 * k as f32, before[1][1]);
        h.hover_at(p);
        h.run_steps(1);
    }
    h.drop_at(p);
    h.run_steps(2);
    let after = new_hsb(h.state());
    let turned = hue_gap(after[1][0], before[1][0]);
    assert!(turned > 20.0, "the marker turned: {before:?} -> {after:?}");
    assert!((hue_gap(after[3][0], before[3][0]) - turned).abs() < 1.0, "the color on its line turned as far: {before:?} -> {after:?}");
    assert!(hue_gap(after[0][0], before[0][0]) < 0.5 && hue_gap(after[2][0], before[2][0]) < 0.5, "the others stay: {before:?} -> {after:?}");
}

/// Set Current Color As Base Color makes the selected color the base; it is off while the selected
/// color is the base. A color bar's menu has Set As Base Color too.
#[test]
fn set_current_color_as_base_color_marks_the_selected_color() {
    let mut h = edit_harness(&["#ff0000", "#0000ff", "#00ff00"]);
    let name = "Set Current Color As Base Color";
    assert!(h.get_by_role_and_label(egui::accesskit::Role::Button, name).accesskit_node().is_disabled(), "the base color is the selected one");
    set(h.state_mut(), "__sel", json!([2]));
    h.run_steps(3);
    click_button(&mut h, name);
    assert_eq!(field(h.state(), "rows")[2]["base"], json!(true));
    assert!(h.get_by_role_and_label(egui::accesskit::Role::Button, name).accesskit_node().is_disabled());
    show_display(&mut h, "Show color bars");
    right_click_bar(&mut h, "Color 1");
    h.get_by_label("   Set As Base Color").click();
    h.run_steps(3);
    let rows = field(h.state(), "rows");
    assert_eq!((&rows[0]["base"], rows[2].get("base")), (&json!(true), None), "{rows}");
}

// Optional local visual QA; these captures are not committed assets.
fn capture_recolor(h: &mut egui_kittest::Harness<'_, VectorcraftApp>, name: &str) {
    if let Some(dir) = std::env::var_os("VECTORCRAFT_RECOLOR_CAPTURE_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        h.render().unwrap().save(dir.join(format!("{name}.png"))).unwrap();
    }
}

/// Secondary-click the actual New chip; the menu must dispatch through its UI callbacks.
fn row_context_menu(h: &mut egui_kittest::Harness<'_, VectorcraftApp>, row: usize) {
    let pos = h.ctx.read_response(egui::Id::unique(("recolor-new", row))).expect("New chip").rect.center();
    h.event(Event::PointerMoved(pos));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(2);
}

#[test]
fn row_context_menu_pick_from_swatches_and_limit_to_library() {
    use egui::accesskit::Role;

    for empty_new in [false, true] {
        let mut app = app();
        art(&mut app, &["#ff0000", "#0000ff"]);
        // A known, single-color palette makes it impossible to pass without snapping the rows.
        let doc = &app.session.doc().unwrap().doc;
        let names: Vec<_> = doc
            .swatches
            .iter()
            .filter(|s| !s.is_reserved())
            .map(|s| s.name.clone())
            .chain(doc.swatch_groups.iter().map(|g| g.name.clone()))
            .collect();
        app.run("swatch.delete", json!({"names": names})).unwrap();
        app.run("swatch.new", json!({"name": "Only Yellow", "color": "#ffff00"})).unwrap();
        let undo = app.session.doc().unwrap().history.undo.len();
        let mut h = assign_harness(app, json!({}));
        let row = 1;
        if empty_new {
            let mut rows = field(h.state(), "rows");
            rows[row]["to"] = Value::Null;
            set(h.state_mut(), "rows", rows);
            h.run_steps(2);
        }
        let before = field(h.state(), "rows");
        let expected = if empty_new { before[row]["from"][0].clone() } else { before[row]["to"].clone() };
        row_context_menu(&mut h, row);
        assert_eq!(field(h.state(), "__sel"), json!([row]), "right-click selects the targeted row");
        capture_recolor(&mut h, "assign-row-menu");
        h.get_by_label("   Pick from Swatches").click();
        h.run_steps(3);
        let picker = h.state().ui.dialog.as_ref().expect("picker opened by menu");
        assert_eq!(picker.kind, "colorPicker");
        assert!(picker.bool("swatches"));
        assert_eq!(picker.fields["__recolorRow"], json!(row));
        assert_eq!(picker.fields.get("color").and_then(vectorcraft_engine::cmd::color_value), vectorcraft_engine::cmd::color_value(&expected));
        capture_recolor(&mut h, "picker-swatches");
        h.get_by_role_and_label(Role::Button, "Cancel").click();
        h.run_steps(3);
        assert_eq!(field(h.state(), "rows"), before, "picker Cancel preserves all rows");

        row_context_menu(&mut h, row);
        h.get_by_label("Limit to Library").click();
        h.run_steps(3);
        capture_recolor(&mut h, "assign-library-menu");
        h.get_by_label("   Document Swatches").click();
        h.run_steps(3);
        assert_eq!(field(h.state(), "limitTo"), json!("document"));
        let snapped = field(h.state(), "rows");
        for (i, r) in snapped.as_array().unwrap().iter().enumerate() {
            assert_eq!(r["to"], "rgb 255 255 0", "row {i} snaps to the palette");
        }
        assert_ne!(snapped, before, "the menu changes row colors");
        row_context_menu(&mut h, row);
        h.get_by_label("Limit to Library").click();
        h.run_steps(3);
        h.get_by_label("   None").click();
        h.run_steps(3);
        assert_eq!(field(h.state(), "limitTo"), json!(""));
        assert_eq!(h.state().session.doc().unwrap().history.undo.len(), undo, "menu actions remain previews");
        h.get_by_role_and_label(Role::Button, "Cancel").click();
        h.run_steps(2);
        assert_eq!(colors(h.state_mut()), 2, "Recolor Cancel restores the original artwork");
    }
}

/// Whether the dialog's button `name` is disabled.
fn disabled(h: &egui_kittest::Harness<'_, VectorcraftApp>, name: &str) -> bool {
    h.get_by_role_and_label(egui::accesskit::Role::Button, name).accesskit_node().is_disabled()
}

/// Undo Changes and Redo Changes step back and forward through the dialog's own color edits, and
/// the art previews each step; the document's undo history is unchanged until OK, which records
/// one step. Both buttons start disabled, and a new edit after Undo Changes leaves nothing to redo.
#[test]
fn undo_and_redo_changes_step_through_the_dialogs_own_edits() {
    let mut h = edit_harness(&["#ff0000", "#00ff00", "#0000ff"]);
    let undo = h.state().session.doc().unwrap().history.undo.len();
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Assign").click();
    h.run_steps(3);
    assert!(disabled(&h, "Undo Changes") && disabled(&h, "Redo Changes"), "nothing to step to on opening");
    let state = |h: &egui_kittest::Harness<'_, VectorcraftApp>| (field(h.state(), "rows"), fills(h.state()));
    let start = state(&h);
    h.get_by_label("Randomly change color order").click();
    h.run_steps(3);
    let first = state(&h);
    assert_ne!(first, start, "the order changed and previews");
    h.get_by_label("Randomly change saturation and brightness").click();
    h.run_steps(3);
    let second = state(&h);
    assert_ne!(second, first);
    assert!(!disabled(&h, "Undo Changes") && disabled(&h, "Redo Changes"));
    for (button, want) in [("Undo Changes", &first), ("Undo Changes", &start), ("Redo Changes", &first), ("Redo Changes", &second)] {
        h.get_by_role_and_label(egui::accesskit::Role::Button, button).click();
        h.run_steps(3);
        assert_eq!(&state(&h), want, "{button}");
    }
    assert_eq!(h.state().session.doc().unwrap().history.undo.len(), undo, "the dialog's steps leave the document's history alone");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Undo Changes").click();
    h.run_steps(3);
    assert_eq!(state(&h), first);
    assert!(!disabled(&h, "Redo Changes"));
    h.get_by_label("Randomly change saturation and brightness").click();
    h.run_steps(3);
    assert!(disabled(&h, "Redo Changes"), "a new edit leaves nothing to redo");
    let last = state(&h);
    h.get_by_label("OK").click();
    h.run_steps(2);
    let app = h.state_mut();
    assert!(app.ui.dialog.is_none());
    assert_eq!(fills(app), last.1, "OK keeps the edits");
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "as one undo step");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(fills(app), start.1);
    // A new opening has nothing to undo.
    app.run("select.all", json!({})).unwrap();
    app.run("ui.recolorDialog", json!({})).unwrap();
    h.run_steps(3);
    assert!(disabled(&h, "Undo Changes") && disabled(&h, "Redo Changes"), "nothing to step to on a new opening");
    h.get_by_label("Cancel").click();
    h.run_steps(2);
}

/// A drag is one step from press to release: Undo Changes after dragging the Edit tab's Brightness
/// slider across several frames gives the rows the drag started on, with nothing left to undo.
#[test]
fn a_drag_is_one_step_of_undo_changes() {
    let mut h = edit_harness(&["rgb 200 100 100", "rgb 100 200 200"]);
    let rows = field(h.state(), "rows");
    let slider = widget_of_size(&h, vec2(220.0, 22.0));
    let v = new_hsb(h.state())[0][2];
    let at = egui::pos2(slider.left() + 4.0 + v * (slider.width() - 8.0), slider.center().y);
    h.hover_at(at);
    h.run_steps(1);
    h.drag_at(at);
    h.run_steps(1);
    for k in 1..=6 {
        h.hover_at(egui::pos2(at.x + 12.0 * k as f32, at.y));
        h.run_steps(1);
    }
    h.drop_at(egui::pos2(at.x + 72.0, at.y));
    h.run_steps(2);
    assert_ne!(field(h.state(), "rows"), rows, "the drag changed the colors");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Undo Changes").click();
    h.run_steps(3);
    assert_eq!(field(h.state(), "rows"), rows, "one Undo Changes goes back to before the drag");
    assert!(disabled(&h, "Undo Changes"), "the drag was one step");
    assert!(!disabled(&h, "Redo Changes"));
}

/// Reset is a step of its own: Undo Changes gives back the rows Reset discarded.
#[test]
fn undo_changes_reverses_reset() {
    let mut app = app();
    art(&mut app, &["#ff0000", "#00ff00", "#0000ff"]);
    let mut h = assign_harness(app, json!({}));
    let start = field(h.state(), "rows");
    h.get_by_label("Randomly change saturation and brightness").click();
    h.run_steps(3);
    let edited = field(h.state(), "rows");
    assert_ne!(edited, start);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Reset the rows to the artwork's colors").click();
    h.run_steps(3);
    assert_eq!(field(h.state(), "rows"), start, "Reset reads the art's colors");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Undo Changes").click();
    h.run_steps(3);
    assert_eq!(field(h.state(), "rows"), edited, "Undo Changes gives back the rows before Reset");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Redo Changes").click();
    h.run_steps(3);
    assert_eq!(field(h.state(), "rows"), start, "Redo Changes resets them again");
}

/// `ui.recolorUndo` and `ui.recolorRedo` step the open dialog as its buttons do, also through a
/// field set without a frame between, and a new count of Colors is one step that restores the count
/// and the rows; outside the dialog they are an error, and a new opening has nothing to undo.
#[test]
fn the_recolor_undo_and_redo_commands_step_the_open_dialog() {
    let mut app = app();
    let ctx = context();
    art(&mut app, &["#ff0000", "#e01010", "#0000ff", "#1010e0"]);
    assert!(app.run("ui.recolorUndo", json!({})).is_err(), "no dialog open");
    // A field set before the dialog's first frame is a step back to the fields it opened with.
    app.run("ui.recolorDialog", json!({})).unwrap();
    set(&mut app, "colors", json!(2));
    assert!(dialogs::recolor::can_step(&app, true) && !dialogs::recolor::can_step(&app, false), "Undo Changes can run before the first frame");
    let r = app.run("ui.recolorUndo", json!({})).unwrap();
    assert_eq!(r, json!({"changed": true, "canUndo": false, "canRedo": true}));
    assert_eq!(field(&app, "colors"), Value::Null);
    assert_eq!(field(&app, "rows").as_array().unwrap().len(), 4, "the rows of the art as it opened");
    dialogs::cancel(&mut app);
    app.run("select.all", json!({})).unwrap();
    app.run("ui.recolorDialog", json!({})).unwrap();
    dialog_frame(&mut app, &ctx);
    assert!(!dialogs::recolor::can_step(&app, true) && !dialogs::recolor::can_step(&app, false));
    let start = field(&app, "rows");
    assert_eq!(start.as_array().unwrap().len(), 4);
    set(&mut app, "colors", json!(2));
    dialog_frame(&mut app, &ctx);
    assert_eq!(field(&app, "rows").as_array().unwrap().len(), 2);
    let two = field(&app, "rows");
    set(&mut app, "globalAdjust", json!({"saturation": -50}));
    dialog_frame(&mut app, &ctx);
    let r = app.run("ui.recolorUndo", json!({})).unwrap();
    assert_eq!(r, json!({"changed": true, "canUndo": true, "canRedo": true}));
    assert_eq!((field(&app, "globalAdjust"), field(&app, "rows")), (Value::Null, two.clone()));
    app.run("ui.recolorUndo", json!({})).unwrap();
    dialog_frame(&mut app, &ctx);
    assert_eq!((field(&app, "colors"), field(&app, "rows")), (Value::Null, start.clone()), "the count and its rows, not reduced again");
    assert!(!dialogs::recolor::can_step(&app, true));
    assert_eq!(app.run("ui.recolorUndo", json!({})).unwrap()["changed"], json!(false));
    app.run("ui.recolorRedo", json!({})).unwrap();
    assert_eq!((field(&app, "colors"), field(&app, "rows")), (json!(2), two));
    // A field set without a frame between is a step before Undo Changes goes back.
    set(&mut app, "method", json!("exact"));
    assert_eq!(app.run("ui.recolorUndo", json!({})).unwrap()["changed"], json!(true));
    assert_eq!(field(&app, "method"), json!("scaleTints"));
    dialogs::cancel(&mut app);
    assert!(app.run("ui.recolorRedo", json!({})).is_err(), "Cancel closed the dialog");
    app.run("select.all", json!({})).unwrap();
    app.run("ui.recolorDialog", json!({})).unwrap();
    dialog_frame(&mut app, &ctx);
    assert!(!dialogs::recolor::can_step(&app, true) && !dialogs::recolor::can_step(&app, false), "a new opening");
}

/// An edit made while the pointer is held from the dialog's first frame is a step from the fields
/// the dialog opened with, not where Undo Changes starts.
#[test]
fn an_edit_with_the_pointer_down_from_the_first_frame_is_a_step() {
    let mut app = app();
    let ctx = context();
    art(&mut app, &["#ff0000", "#e01010", "#0000ff", "#1010e0"]);
    app.run("ui.recolorDialog", json!({})).unwrap();
    let at = Pos2::new(2.0, 2.0);
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(&mut app, &ctx, vec![Event::PointerMoved(at), button(true)], 1.0, |app, ui| dialogs::show(app, ui.ctx()));
    let start = field(&app, "rows");
    assert_eq!(start.as_array().unwrap().len(), 4);
    set(&mut app, "colors", json!(2));
    frame(&mut app, &ctx, vec![], 2.0, |app, ui| dialogs::show(app, ui.ctx()));
    frame(&mut app, &ctx, vec![button(false)], 3.0, |app, ui| dialogs::show(app, ui.ctx()));
    assert_eq!(field(&app, "rows").as_array().unwrap().len(), 2);
    let r = app.run("ui.recolorUndo", json!({})).unwrap();
    assert_eq!(r, json!({"changed": true, "canUndo": false, "canRedo": true}));
    assert_eq!((field(&app, "colors"), field(&app, "rows")), (Value::Null, start));
}

/// Rows or weights set before the first frame, which the art's reduction then replaces or drops,
/// are no step: Undo Changes is not available and changes nothing.
#[test]
fn rows_and_weights_set_before_the_art_is_read_are_no_step() {
    for (k, v) in [("rows", json!([{"to": "#123456"}])), ("weights", json!([25, 75]))] {
        let mut app = app();
        art(&mut app, &["#ff0000", "#0000ff"]);
        app.run("ui.recolorDialog", json!({})).unwrap();
        set(&mut app, k, v);
        assert!(!dialogs::recolor::can_step(&app, true), "{k}: Undo Changes is not available");
        let r = app.run("ui.recolorUndo", json!({})).unwrap();
        assert_eq!(r, json!({"changed": false, "canUndo": false, "canRedo": false}), "{k}");
        assert!(!app.session.in_interaction(), "{k}: a step that changes nothing previews nothing");
        assert_eq!(field(&app, "rows").as_array().unwrap().len(), 2, "{k}: the rows of the art");
    }
}

/// `ui.recolorUndo` and `ui.recolorRedo` preview the step they go to at once, without a frame
/// between, as the buttons do.
#[test]
fn the_recolor_undo_and_redo_commands_preview_the_step() {
    let mut app = app();
    let ctx = context();
    art(&mut app, &["#ff0000", "#e01010", "#0000ff", "#1010e0"]);
    app.run("ui.recolorDialog", json!({})).unwrap();
    dialog_frame(&mut app, &ctx);
    let start = fills(&app);
    set(&mut app, "globalAdjust", json!({"saturation": -100}));
    dialog_frame(&mut app, &ctx);
    let adjusted = fills(&app);
    assert_ne!(adjusted, start, "the adjustment previews");
    app.run("ui.recolorUndo", json!({})).unwrap();
    assert_eq!(fills(&app), start, "Undo Changes previews the step before");
    app.run("ui.recolorRedo", json!({})).unwrap();
    assert_eq!(fills(&app), adjusted, "Redo Changes previews the step after");
    dialogs::cancel(&mut app);
}

/// With the Color Groups list shown, the Color Picker opened from a row and closed with Cancel
/// leaves Recolor Artwork the size it was and where it was, in every frame.
#[test]
fn the_color_picker_leaves_the_dialog_in_place() {
    use egui::accesskit::Role;

    let mut app = app();
    art(&mut app, &["#ff0000", "#0000ff"]);
    let mut h = assign_harness(app, json!({}));
    assert_eq!(field(h.state(), "groupStorage"), json!(true), "the list is shown");
    let window = egui::Id::unique(("dialog", "recolor"));
    let rect = |h: &egui_kittest::Harness<'_, VectorcraftApp>| h.ctx.memory(|m| m.area_rect(window));
    let before = rect(&h).expect("dialog drawn");
    row_context_menu(&mut h, 1);
    h.get_by_label("   Pick from Swatches").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.dialog.as_ref().map(|d| d.kind.as_str()), Some("colorPicker"));
    assert_eq!(rect(&h), Some(before), "the dialog keeps its size while the picker is open");
    h.get_by_role_and_label(Role::Button, "Cancel").click();
    for step in 0..4 {
        h.run_steps(1);
        assert_eq!(rect(&h), Some(before), "frame {step} after Cancel");
    }
    assert_eq!(h.state().ui.dialog.as_ref().map(|d| d.kind.as_str()), Some("recolor"));
}
