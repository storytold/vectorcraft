//! Recolor Artwork (M3.78) and Edit or Apply Color Group (M3.79), drawn headlessly.

use egui::{Event, Modifiers, PointerButton, Pos2, Rect, vec2};
use egui_kittest::kittest::Queryable;
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
    for (k, v) in [("sliderMode", json!("hsb")), ("sliderMode", json!("lab")), ("tab", json!("edit")), ("tab", json!("assign"))] {
        assert_eq!(height(&mut h, k, v.clone()), assign, "{k} {v}");
    }
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
