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
    let folder = ctx.read_response(egui::Id::new(("swatch-tile", "Brights"))).expect("the Brights folder").rect.center();
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
    let folder = ctx.read_response(egui::Id::new(("swatch-tile", "Brights"))).unwrap().rect.center();
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
            let dialog = h.ctx.memory(|m| m.area_rect(egui::Id::new(("dialog", "recolor")))).unwrap();
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
