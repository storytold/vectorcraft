/// With more tabs than fit, the strip starts late enough to show the active one (#746).
#[test]
fn the_active_tab_stays_in_view() {
    let widths = [100.0; 10];
    assert_eq!(super::first_tab_shown(&widths, Some(2), 350.0), 0, "it fits from the start");
    assert_eq!(super::first_tab_shown(&widths, Some(9), 350.0), 7, "the last three tabs shown");
    assert_eq!(super::first_tab_shown(&widths, None, 350.0), 0);
    assert_eq!(super::first_tab_shown(&widths, Some(4), 50.0), 4, "a tab wider than the room still shows");
}
use std::sync::Arc;

use super::{StatusReadoutWidths, status_readouts};
use crate::theme::Tokens;
use serde_json::json;
use vectorcraft_engine::Session;

#[test]
fn hints_name_keys_as_the_menus_do() {
    let text = |tool: &str| super::hint_segments(tool).into_iter().map(|(s, _)| s).collect::<String>();
    for tool in vectorcraft_tools::catalog::all_tools().map(|t| t.id) {
        let hint = text(tool);
        assert!(!hint.contains("Option"), "{tool}: {hint}");
        assert!(cfg!(target_os = "macos") || !hint.contains("Cmd"), "{tool}: {hint}");
    }
    let pretty = crate::menus::pretty_shortcut;
    assert!(text("selection").contains(&pretty("Alt+Drag")));
    assert!(text("rectangle").contains(&pretty("Alt+Drag")));
    assert!(text("zoom").contains(&pretty("Alt+Click")));
    assert!(text("rotate").contains(&pretty("Alt+Click")));
    assert!(text("eyedropper").contains(&pretty("Alt+Click")));
    assert!(text("perspectiveSelection").contains(&pretty("Alt+Drag")) && text("perspectiveSelection").contains("perpendicular"));
    assert!(text("paintbrush").contains(&pretty("Cmd+Shift+/")), "the Search Commands shortcut");
    if !cfg!(target_os = "macos") {
        assert!(text("zoom").contains("Alt+Click") && text("paintbrush").contains("Ctrl+Shift+/"));
    }
}

#[derive(Debug)]
struct StatusText {
    text: String,
    at: egui::Pos2,
    clip: egui::Rect,
    bounds: egui::Rect,
    elided: bool,
}

/// Collect status-bar text layout from a headless frame without pointer events.
fn status_readout_frame(app: &mut crate::VectorcraftApp, ctx: &egui::Context, width: f32) -> Vec<StatusText> {
    status_readout_frame_events(app, ctx, width, vec![])
}

/// Collect status-bar text, positions, clips and elision state with explicit pointer events.
fn status_readout_frame_events(app: &mut crate::VectorcraftApp, ctx: &egui::Context, width: f32, events: Vec<egui::Event>) -> Vec<StatusText> {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 300.0));
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| super::status_bar(app, ui));
    out.textures_delta.clear();
    out.shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) => Some(StatusText {
                text: t.galley.job.text.clone(),
                at: t.pos,
                clip: s.clip_rect,
                bounds: egui::Rect::from_min_size(t.pos, t.galley.size()),
                elided: t.galley.elided,
            }),
            _ => None,
        })
        .collect()
}

/// Find a required text record by prefix, failing the test when it is absent.
fn status_text<'a>(texts: &'a [StatusText], prefix: &str) -> &'a StatusText {
    texts.iter().find(|t| t.text.starts_with(prefix)).unwrap_or_else(|| panic!("missing {prefix:?}: {texts:?}"))
}

/// Prepare an English test document with the app fonts and a status message.
fn status_readout_app() -> (crate::VectorcraftApp, egui::Context) {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    app.ui.status = "Saved recovery data for Untitled-1".to_owned();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    crate::theme::apply(&ctx, crate::theme::Brightness::Dark);
    (app, ctx)
}

/// Draw the production readouts and observe the cached measurement object across frames.
fn status_width_cache_frame(app: &crate::VectorcraftApp, ctx: &egui::Context, width: f32, ppp: f32) -> Arc<StatusReadoutWidths> {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 300.0));
    let mut input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
    input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(ppp);
    let mut cache = None;
    let mut output = ctx.run_ui(input, |ui| {
        let t = Tokens::get(ui.ctx());
        status_readouts(app, ui, &t);
        cache = ui.data(|data| data.get_temp::<Arc<StatusReadoutWidths>>(ui.scope_id().with("status-readout-widths")));
    });
    output.textures_delta.clear();
    cache.expect("production status readouts populated their cache")
}

/// Live values, hover state and window width reuse the same measurement object.
#[test]
fn status_width_measurements_are_reused_across_frames() {
    let (mut app, ctx) = status_readout_app();
    let first = status_width_cache_frame(&app, &ctx, 1400.0, 1.0);
    for width in [1400.0_f32, 500.0, 2200.0] {
        app.perf = crate::Perf { render_ms: f64::from(width), frame_ms: f64::from(width) / 3.0, fps: 120.0 };
        app.hover_doc = Some(vectorcraft_geom::Point::new(-width as f64, width as f64));
        let cached = status_width_cache_frame(&app, &ctx, width, 1.0);
        assert!(Arc::ptr_eq(&first, &cached), "live state or window size remeasured the samples");
    }
    app.hover_doc = None;
    assert!(Arc::ptr_eq(&first, &status_width_cache_frame(&app, &ctx, 1400.0, 1.0)));
}

/// Each independent cache-key component refreshes widths, then subsequent frames reuse them.
#[test]
fn status_width_measurements_refresh_for_language_units_and_dpi() {
    let (mut app, ctx) = status_readout_app();
    let mut previous = status_width_cache_frame(&app, &ctx, 4000.0, 1.0);
    let french = crate::i18n::Lang::from_code("fr").unwrap();
    for (lang, unit, ppp) in [
        (french, vectorcraft_doc::Unit::Points, 1.0),
        (french, vectorcraft_doc::Unit::Millimeters, 1.0),
        (french, vectorcraft_doc::Unit::Millimeters, 1.5),
        (crate::i18n::Lang::EN, vectorcraft_doc::Unit::Millimeters, 1.5),
        (crate::i18n::Lang::EN, vectorcraft_doc::Unit::Points, 1.5),
        (crate::i18n::Lang::EN, vectorcraft_doc::Unit::Points, 1.0),
    ] {
        crate::i18n::set_current(lang);
        app.session.execute("document.setUnits", &json!({"units": unit.key()})).unwrap();
        let fresh = status_width_cache_frame(&app, &ctx, 4000.0, ppp);
        assert_eq!(fresh.key, (lang, unit, ppp));
        assert!(!Arc::ptr_eq(&previous, &fresh), "changed language/units/DPI reused stale widths");
        assert!(fresh.values.iter().all(|w| w.is_finite() && *w > 0.0));
        assert!(Arc::ptr_eq(&fresh, &status_width_cache_frame(&app, &ctx, 4000.0, ppp)));
        previous = fresh;
    }
}

/// These assertions require every target to be present, and cover the old padding limits.
#[test]
fn the_status_bar_stands_still_while_its_readouts_change() {
    let (mut app, ctx) = status_readout_app();
    let mut baseline = None;
    let mut message_baseline = None;
    for (render, frame, fps, x, y) in [
        (233.4, 4.9, 80.0, -662.25, 1647.9),
        (9.8, 1.0, 120.0, 7.0, 5.0),
        (100000.0, 10000.0, 1000.0, -1000.0, 100000.0),
        (99999.9, 9999.9, 999.0, -999.99, 9999.99),
    ] {
        app.perf = crate::Perf { render_ms: render, frame_ms: frame, fps };
        app.hover_doc = Some(vectorcraft_geom::Point::new(x, y));
        let texts = status_readout_frame(&mut app, &ctx, 1400.0);
        let message = status_text(&texts, "Saved recovery data");
        let extent = (message.at, message.clip, message.bounds, message.elided);
        if let Some(first) = message_baseline {
            assert_eq!(extent, first, "message extent changed with readout values");
        } else {
            message_baseline = Some(extent);
        }
        for (prefix, value) in [
            ("render", format!("{render:.1} ms")),
            ("ui ", format!("{frame:.1} ms")),
            ("X:", app.session.general_unit().readout(x)),
            ("Y:", app.session.general_unit().readout(y)),
        ] {
            let text = status_text(&texts, prefix);
            assert!(text.text.contains(&value), "{text:?} must contain {value:?}");
            assert!(text.bounds.width() <= text.clip.width() + 0.01, "field overflow: {text:?}");
        }
        let fps_text = format!("{fps:.0} fps");
        let fps_label = status_text(&texts, &fps_text);
        assert!(fps_label.bounds.width() <= fps_label.clip.width() + 0.01);
        let positions = ["Saved recovery data", "render", "ui ", "X:", "Y:", fps_text.as_str()].map(|p| status_text(&texts, p).at);
        if let Some(first) = baseline {
            assert_eq!(positions, first, "readout origins changed");
        } else {
            baseline = Some(positions);
        }
    }
    app.perf.render_ms = 1000000.0;
    let tree = crate::widgets::accessibility_tests::tree(|ui| super::status_bar(&mut app, ui));
    assert!(
        tree.iter().any(|n| n.role() == egui::accesskit::Role::Label && n.value() == Some("render 1000000.0 ms")),
        "screen readers receive the complete value"
    );
}

/// Canvas hover entry and exit keep the status message at the same origin.
#[test]
fn status_messages_keep_their_origin_when_the_pointer_enters_and_leaves() {
    let (mut app, ctx) = status_readout_app();
    let mut first = None;
    for hover in [None, Some(vectorcraft_geom::Point::new(7.0, 5.0)), None] {
        app.hover_doc = hover;
        let texts = status_readout_frame(&mut app, &ctx, 1400.0);
        let message = status_text(&texts, "Saved recovery data");
        assert!(!message.elided);
        let position = (message.at, message.clip);
        if let Some(first) = first {
            assert_eq!(position, first);
        } else {
            first = Some(position);
        }
        assert_eq!(texts.iter().any(|t| t.text.starts_with("X:")), hover.is_some());
    }
}

/// Hovering a coordinate preserves its document-scoped value for inspection.
#[test]
fn hovering_a_coordinate_retains_its_full_value_for_inspection() {
    let (mut app, ctx) = status_readout_app();
    app.hover_doc = Some(vectorcraft_geom::Point::new(1e12, -1e12));
    let first = status_readout_frame(&mut app, &ctx, 1400.0);
    let x = status_text(&first, "X:");
    assert!(x.elided);
    let expected = x.text.clone();
    app.hover_doc = None;
    let inspect = status_readout_frame_events(&mut app, &ctx, 1400.0, vec![egui::Event::PointerMoved(x.clip.center())]);
    assert_eq!(status_text(&inspect, "X:").text, expected);
    let gone = status_readout_frame_events(&mut app, &ctx, 1400.0, vec![egui::Event::PointerGone]);
    assert!(!gone.iter().any(|t| t.text.starts_with("X:")));
    // A different document must not inherit the previous document's coordinate.
    app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let other = status_readout_frame_events(&mut app, &ctx, 1400.0, vec![egui::Event::PointerMoved(x.clip.center())]);
    assert!(!other.iter().any(|t| t.text.starts_with("X:")));
}

/// Narrow windows hide optional readouts while keeping the status message visible.
#[test]
fn narrow_status_bars_preserve_the_message_and_hide_optional_readouts() {
    let (mut app, ctx) = status_readout_app();
    app.hover_doc = Some(vectorcraft_geom::Point::new(7.0, 5.0));
    let mut origin = None;
    for (width, coords, perf) in [(1400.0, true, true), (1000.0, true, false), (800.0, false, false)] {
        let texts = status_readout_frame(&mut app, &ctx, width);
        let message = status_text(&texts, "Saved recovery data");
        assert!(!message.elided, "{width}: message was truncated: {message:?}");
        if let Some(origin) = origin {
            assert_eq!(message.at, origin);
        } else {
            origin = Some(message.at);
        }
        assert_eq!(texts.iter().any(|t| t.text.starts_with("X:")), coords, "{width}");
        assert_eq!(texts.iter().any(|t| t.text.starts_with("render")), perf, "{width}");
        let tool = status_text(&texts, "Selection");
        assert!(message.clip.left() >= tool.bounds.right(), "readouts must not overlap navigation: {texts:?}");
        for field in texts.iter().filter(|t| t.text.starts_with("X:") || t.text.starts_with("Y:") || t.text.starts_with("render")) {
            assert!(field.clip.left() >= tool.bounds.right());
            assert!(!field.clip.intersects(message.clip));
        }
    }
    // Still narrower bars may truncate a long message but must not draw over navigation.
    let texts = status_readout_frame(&mut app, &ctx, 600.0);
    let message = status_text(&texts, "Saved recovery data");
    assert!(message.clip.left() >= status_text(&texts, "Selection").bounds.right());
}

/// Busy-job transitions preserve message placement, labels and accessibility.
#[test]
fn background_activity_keeps_the_status_message_origin() {
    let (mut app, ctx) = status_readout_app();
    let before = status_readout_frame(&mut app, &ctx, 1400.0);
    let before = status_text(&before, "Saved recovery data");
    let (tx, rx) = std::sync::mpsc::channel();
    app.services.write_shared = Some(std::sync::Arc::new(|_, _| Ok(())));
    crate::background::run(
        &mut app,
        true,
        "Saving test.vectorcraft".to_owned(),
        None,
        move |_| {
            rx.recv().unwrap();
            Ok(json!({}))
        },
        |app, result| {
            app.status("Saved recovery data for Untitled-1");
            result
        },
    )
    .unwrap();
    let busy = status_readout_frame(&mut app, &ctx, 1400.0);
    let message = status_text(&busy, "Saving test.vectorcraft");
    assert_eq!((before.at, before.clip), (message.at, message.clip));
    crate::background::run(&mut app, true, "Exporting test.svg".to_owned(), None, |_| Ok(json!({})), |_, result| result).unwrap();
    let several = status_readout_frame(&mut app, &ctx, 1400.0);
    let message = status_text(&several, "Saving test.vectorcraft");
    assert!(message.text.ends_with("(+1)"));
    assert_eq!((before.at, before.clip), (message.at, message.clip));
    let tree = crate::widgets::accessibility_tests::tree(|ui| super::status_bar(&mut app, ui));
    assert!(tree.iter().any(|n| n.role() == egui::accesskit::Role::ProgressIndicator), "the busy spinner remains accessible");
    tx.send(()).unwrap();
    crate::background::wait_all(&mut app);
    let after = status_readout_frame(&mut app, &ctx, 1400.0);
    let after = status_text(&after, "Saved recovery data");
    assert_eq!((before.at, before.clip), (after.at, after.clip));
}

/// Ordinary signed coordinates fit without elision in every supported unit.
#[test]
fn ordinary_coordinates_are_readable_in_every_unit() {
    let (mut app, ctx) = status_readout_app();
    for unit in vectorcraft_doc::Unit::ALL {
        app.session.execute("document.setUnits", &json!({"units": unit.key()})).unwrap();
        for (x, y) in [(-1234.56, 1234.56), (-9999.99, 9999.99), (9999.99, -9999.99)] {
            let (x, y) = (unit.to_pt(x), unit.to_pt(y));
            app.hover_doc = Some(vectorcraft_geom::Point::new(x, y));
            let texts = status_readout_frame(&mut app, &ctx, 1400.0);
            for (axis, value) in [("X:", x), ("Y:", y)] {
                let field = status_text(&texts, axis);
                assert_eq!(field.text, format!("{axis} {}", unit.readout(value)));
                assert!(!field.elided, "{unit:?}: {field:?}");
                assert!(field.clip.contains_rect(field.bounds), "{unit:?}: {field:?}");
            }
        }
    }
}

/// Localized diagnostics remain readable without moving neighboring fields.
#[test]
fn localized_diagnostics_are_readable_and_keep_their_field_bounds() {
    let (mut app, ctx) = status_readout_app();
    for lang in crate::i18n::Lang::all() {
        crate::i18n::set_current(lang);
        let render = crate::i18n::tr_ctx(lang, "diagnostics", "render");
        let ui_name = crate::i18n::tr_ctx(lang, "diagnostics", "ui");
        let mut baseline = None;
        for (render_ms, frame_ms, fps, readable) in [(233.4, 4.9, 80.0, true), (9999.9, 9999.9, 9999.0, true), (1e12, 1e12, 1e12, false)] {
            app.perf = crate::Perf { render_ms, frame_ms, fps };
            let texts = status_readout_frame(&mut app, &ctx, 4000.0);
            let fields = [format!("{render} {render_ms:.1} ms"), format!("{ui_name} {frame_ms:.1} ms"), format!("{fps:.0} fps")];
            let bounds = fields.map(|value| {
                let field = status_text(&texts, &value);
                assert_eq!(field.text, value);
                assert_eq!(!field.elided, readable, "{}: {field:?}", lang.code());
                assert!(field.bounds.width() <= field.clip.width() + 0.01, "{}: {field:?}", lang.code());
                (field.at, field.clip)
            });
            if let Some(first) = baseline {
                assert_eq!(bounds, first, "{}: field allocation changed", lang.code());
            } else {
                baseline = Some(bounds);
            }
        }
        // Even long translations must yield their whole diagnostics group in a narrow
        // window rather than overlapping the message or navigation controls.
        let texts = status_readout_frame(&mut app, &ctx, 800.0);
        assert!(!texts.iter().any(|t| t.text == format!("{render} {:.1} ms", app.perf.render_ms)), "{}", lang.code());
    }
    crate::i18n::set_current(crate::i18n::Lang::EN);
}

/// A document tab's × sits where the platform puts it (#673): after the title on Windows,
/// Linux and the web, before it on macOS; clicking it closes that document.
#[test]
fn a_tabs_close_box_sits_where_the_platform_puts_it() {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 200.0));
    let frame = |app: &mut crate::VectorcraftApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| super::doc_tabs(app, ui));
        out.textures_delta.clear();
        let mut titles = vec![];
        for c in &out.shapes {
            // The titles, not the × icons.
            if let egui::Shape::Text(t) = &c.shape
                && t.visual_bounding_rect().width() > 30.0
            {
                titles.push(t.visual_bounding_rect());
            }
        }
        titles.sort_by(|a, b| a.left().total_cmp(&b.left()));
        titles
    };
    let titles = frame(&mut app, vec![]);
    assert_eq!(titles.len(), 2, "{titles:?}");
    // The first tab's ×: 20 pt past its title's end, or 16 pt before its start.
    let first = titles[0];
    let x = if super::CLOSE_BEFORE_TITLE { first.left() - 16.0 } else { first.right() + 20.0 };
    let at = egui::pos2(x, first.center().y);
    let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, vec![egui::Event::PointerMoved(at), press(true)]);
    frame(&mut app, vec![press(false)]);
    assert_eq!(app.session.documents().len(), 1, "the × closed the first document");
    assert!(super::CLOSE_BEFORE_TITLE == cfg!(target_os = "macos"));
}

/// One headless frame of the status bar; returns the artboard navigator's buttons, left to
/// right: first, previous, next, last.
fn navigator(app: &mut crate::VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<egui::Rect> {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 300.0));
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| super::status_bar(app, ui));
    out.textures_delta.clear();
    let size = egui::vec2(18.0, 18.0);
    let mut r: Vec<egui::Rect> =
        ctx.viewport(|vp| vp.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).filter(|w| w.rect.size() == size).map(|w| w.rect).collect());
    r.sort_by(|a, b| a.left().total_cmp(&b.left()));
    r
}

/// Artboard navigation updates the active artboard and fitted view together.
#[test]
fn the_artboard_navigator_goes_to_artboards_and_fitting_shows_the_current_one() {
    use vectorcraft_geom::Point;
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    // Three 200 × 100 artboards in a row, centred at x = 100, 320 and 540.
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("artboard.new", json!({})).unwrap();
    app.run("artboard.new", json!({})).unwrap();
    app.canvas_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(460.0, 260.0)));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let buttons = navigator(&mut app, &ctx, vec![]);
    assert_eq!(buttons.len(), 4);
    let click = |app: &mut crate::VectorcraftApp, r: egui::Rect| {
        let b =
            |pressed| egui::Event::PointerButton { pos: r.center(), button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        navigator(app, &ctx, vec![egui::Event::PointerMoved(r.center())]);
        navigator(app, &ctx, vec![b(true)]);
        navigator(app, &ctx, vec![b(false)]);
    };
    let at = |app: &crate::VectorcraftApp| (app.view().unwrap().artboard, app.view().unwrap().center);
    click(&mut app, buttons[2]);
    assert_eq!(at(&app), (1, Point::new(320.0, 50.0)), "next");
    click(&mut app, buttons[3]);
    assert_eq!(at(&app), (2, Point::new(540.0, 50.0)), "last");
    click(&mut app, buttons[1]);
    assert_eq!(at(&app), (1, Point::new(320.0, 50.0)), "previous");
    // Fit Artboard in Window and Actual Size show the navigator's artboard.
    app.run("view.setZoom", json!({"zoom": 300, "center": [0, 0]})).unwrap();
    app.run("view.fitArtboard", json!({})).unwrap();
    assert_eq!(at(&app), (1, Point::new(320.0, 50.0)));
    app.run("view.actualSize", json!({})).unwrap();
    assert_eq!((at(&app).1, app.view().unwrap().zoom), (Point::new(320.0, 50.0), 1.0));
    click(&mut app, buttons[0]);
    assert_eq!(at(&app), (0, Point::new(100.0, 50.0)), "first");
    // The command also takes an index, clamped to the last artboard.
    assert_eq!(app.run("view.goToArtboard", json!({"index": 9})), Ok(json!({"index": 2})));
    assert!(app.run("view.goToArtboard", json!({"index": "sideways"})).is_err());
}

/// storytold/vectorcraft#2719: status-bar 100% matches Actual Size when Display Print Size is on.
#[test]
fn set_zoom_100_matches_actual_size_with_display_print_size() {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.session.execute("prefs.set", &json!({"key": "displayPrintSize", "value": true})).unwrap();
    app.run("view.setZoom", json!({"zoom": 100})).unwrap();
    assert!((app.view().unwrap().zoom - 96.0 / 72.0).abs() < 1e-12);
}

/// Screen readers name the artboard navigator's four buttons, the ones that lead nowhere from
/// the first artboard disabled.
#[test]
fn screen_readers_name_the_artboard_navigators_buttons() {
    use crate::widgets::accessibility_tests::{named, tree};
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    let tree = tree(|ui| super::status_bar(&mut app, ui));
    for (label, enabled) in [("First Artboard", false), ("Previous Artboard", false), ("Next Artboard", true), ("Last Artboard", true)] {
        let n = named(&tree, label);
        assert_eq!((n.role(), n.is_disabled()), (egui::accesskit::Role::Button, !enabled), "{label}");
    }
}

/// One headless frame of the status bar, with room above it for the artboard list; returns the
/// texts painted and where they are.
fn status_texts(app: &mut crate::VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<(String, egui::Rect)> {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 600.0));
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| super::status_bar(app, ui));
    out.textures_delta.clear();
    out.shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) => Some((t.galley.text().to_string(), t.visual_bounding_rect())),
            _ => None,
        })
        .collect()
}

/// Read the painted coordinates while checking that drawing never changes the source
/// position, document, selection or either branch of its edit history.
fn assert_status_coordinates(app: &mut crate::VectorcraftApp, expected: Option<&str>) {
    let hover = app.hover_doc;
    let before = app.session.active().map(|st| (st.doc.clone(), st.selection.clone(), st.history.undo.len(), st.history.redo.len(), st.is_dirty()));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let texts = status_texts(app, &ctx, vec![]);
    let coordinates: Vec<&str> =
        texts.iter().filter_map(|(text, _)| (text.starts_with("X: ") || text.starts_with("Y: ")).then_some(text.as_str())).collect();
    let expected: Vec<&str> = expected.into_iter().flat_map(|text| text.split("  ")).collect();
    assert_eq!(coordinates, expected, "{texts:?}");
    assert_eq!(app.hover_doc, hover, "tools still receive the absolute document position");
    let after = app.session.active().map(|st| (st.doc.clone(), st.selection.clone(), st.history.undo.len(), st.history.redo.len(), st.is_dirty()));
    assert_eq!(after, before, "painting is read-only");
}

#[test]
fn status_coordinates_follow_global_ruler_origins_and_general_units() {
    use vectorcraft_geom::Point;
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("view.globalRulers", json!({"global": true})).unwrap();
    app.session.active_mut().unwrap().mark_saved();
    app.hover_doc = Some(Point::new(72.0, 144.0));
    assert_status_coordinates(&mut app, Some("X: 72.00 pt  Y: 144.00 pt"));
    app.run("document.setRulerOrigin", json!({"x": 36, "y": 72})).unwrap();
    assert_status_coordinates(&mut app, Some("X: 36.00 pt  Y: 72.00 pt"));
    app.run("document.setRulerOrigin", json!({"x": -72, "y": 216})).unwrap();
    assert_status_coordinates(&mut app, Some("X: 144.00 pt  Y: -72.00 pt"));
    app.run("document.setUnits", json!({"units": "Inches"})).unwrap();
    assert_status_coordinates(&mut app, Some("X: 2.00 in  Y: -1.00 in"));
    app.run("document.setUnits", json!({"units": "Millimeters"})).unwrap();
    assert_status_coordinates(&mut app, Some("X: 50.80 mm  Y: -25.40 mm"));
    app.run("document.resetRulerOrigin", json!({})).unwrap();
    assert_status_coordinates(&mut app, Some("X: 25.40 mm  Y: 50.80 mm"));
    // Keep a nonempty Redo branch as well as Undo while painting.
    app.run("edit.undo", json!({})).unwrap();
    assert_status_coordinates(&mut app, Some("X: 50.80 mm  Y: -25.40 mm"));
}

#[test]
fn status_coordinates_follow_the_active_artboard_and_ruler_mode() {
    use vectorcraft_geom::Point;
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("artboard.new", json!({"x": 700, "y": -250, "width": 200, "height": 150})).unwrap();
    app.hover_doc = Some(Point::new(760.0, -210.0));
    app.run("document.setRulerOrigin", json!({"artboard": 0, "x": 25, "y": 30})).unwrap();
    app.run("view.goToArtboard", json!({"index": 0})).unwrap();
    assert_status_coordinates(&mut app, Some("X: 735.00 pt  Y: -240.00 pt"));
    app.run("view.goToArtboard", json!({"index": 1})).unwrap();
    assert_status_coordinates(&mut app, Some("X: 60.00 pt  Y: 40.00 pt"));
    app.run("document.setRulerOrigin", json!({"artboard": 1, "x": 800, "y": -190})).unwrap();
    assert_status_coordinates(&mut app, Some("X: -40.00 pt  Y: -20.00 pt"));
    app.run("view.globalRulers", json!({"global": true})).unwrap();
    assert_status_coordinates(&mut app, Some("X: 760.00 pt  Y: -210.00 pt"));
    app.run("document.setRulerOrigin", json!({"x": -20, "y": -10})).unwrap();
    assert_status_coordinates(&mut app, Some("X: 780.00 pt  Y: -200.00 pt"));
    app.run("view.globalRulers", json!({"global": false})).unwrap();
    assert_status_coordinates(&mut app, Some("X: -40.00 pt  Y: -20.00 pt"));
    app.run("document.resetRulerOrigin", json!({"artboard": 1})).unwrap();
    assert_status_coordinates(&mut app, Some("X: 60.00 pt  Y: 40.00 pt"));
    app.run("view.goToArtboard", json!({"index": 0})).unwrap();
    assert_status_coordinates(&mut app, Some("X: 735.00 pt  Y: -240.00 pt"));
}

#[test]
fn status_coordinates_handle_missing_pointer_or_document() {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    assert_status_coordinates(&mut app, None);
    app.hover_doc = Some(vectorcraft_geom::Point::new(42.0, -21.0));
    assert_status_coordinates(&mut app, Some("X: 42.00 pt  Y: -21.00 pt"));
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("document.setRulerOrigin", json!({"x": 10, "y": 20})).unwrap();
    app.hover_doc = None;
    assert_status_coordinates(&mut app, None);
}

#[test]
fn status_coordinates_keep_the_inspected_point_relative_after_an_origin_change() {
    let (mut app, ctx) = status_readout_app();
    app.run("view.globalRulers", json!({"global": true})).unwrap();
    app.run("document.setRulerOrigin", json!({"x": 36, "y": 72})).unwrap();
    app.hover_doc = Some(vectorcraft_geom::Point::new(72.0, 144.0));
    let first = status_readout_frame(&mut app, &ctx, 1400.0);
    let at = status_text(&first, "X:").bounds.center();
    app.hover_doc = None;
    let inspecting = status_readout_frame_events(&mut app, &ctx, 1400.0, vec![egui::Event::PointerMoved(at)]);
    assert_eq!(status_text(&inspecting, "X:").text, "X: 36.00 pt");
    assert_eq!(status_text(&inspecting, "Y:").text, "Y: 72.00 pt");
    app.run("document.setRulerOrigin", json!({"x": -72, "y": 216})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let inspecting = status_readout_frame(&mut app, &ctx, 1400.0);
    assert_eq!(status_text(&inspecting, "X:").text, "X: 144.00 pt");
    assert_eq!(status_text(&inspecting, "Y:").text, "Y: -72.00 pt");
    assert_eq!(app.hover_doc, None, "inspection never changes the absolute canvas hover state");
    assert_eq!(app.session.active().unwrap().doc, before, "painting is read-only");
    let gone = status_readout_frame_events(&mut app, &ctx, 1400.0, vec![egui::Event::PointerGone]);
    assert!(!gone.iter().any(|field| field.text.starts_with("X:") || field.text.starts_with("Y:")));
}

/// A click at `at` in the status bar; returns the texts of the frame after it.
fn status_click(app: &mut crate::VectorcraftApp, ctx: &egui::Context, at: egui::Pos2) -> Vec<(String, egui::Rect)> {
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    for event in [egui::Event::PointerMoved(at), button(true), button(false)] {
        status_texts(app, ctx, vec![event]);
    }
    status_texts(app, ctx, vec![])
}

/// The navigator shows the active artboard's number and name, and its list shows every artboard
/// by number and name. Choosing one goes to it as the navigator's buttons do
/// (`view.goToArtboard`): it becomes the active artboard and the Artboard tool's, the view
/// centers on it, and the list closes.
#[test]
fn the_artboard_navigator_lists_the_artboards_by_number_and_name() {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    app.run("artboard.setProps", json!({"index": 0, "name": "Front"})).unwrap();
    app.run("artboard.setProps", json!({"index": 1, "name": "Back"})).unwrap();
    app.canvas_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(460.0, 260.0)));
    app.select_tool("artboard");
    app.run("view.goToArtboard", json!({"index": 1})).unwrap();
    let first = app.session.active().unwrap().doc.artboards[0].rect.center();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let texts = status_texts(&mut app, &ctx, vec![]);
    let field = texts.iter().find(|(s, _)| s == "2  Back").map(|(_, r)| *r).expect("the field shows artboard 2's number and name");
    assert!(!texts.iter().any(|(s, _)| s == "Front"), "the list is closed: {texts:?}");
    let texts = status_click(&mut app, &ctx, field.center());
    let at = |s: &str| texts.iter().find(|(t, _)| t == s).map(|(_, r)| *r).unwrap_or_else(|| panic!("no {s:?} in {texts:?}"));
    let rows: Vec<egui::Rect> = [("1", "Front"), ("2", "Back"), ("3", "Artboard 3")]
        .into_iter()
        .map(|(number, name)| {
            let (n, r) = (at(number), at(name));
            assert!(r.y_range().contains(n.center().y) && n.right() < r.left(), "{number} before {name} on one row: {texts:?}");
            r
        })
        .collect();
    assert!(rows.windows(2).all(|w| w[0].bottom() <= w[1].top()), "the rows in the artboards' order: {rows:?}");
    let texts = status_click(&mut app, &ctx, at("Front").center());
    assert_eq!((app.view().unwrap().artboard, app.view().unwrap().center), (0, first), "Front, fitted in the window");
    assert_eq!(app.session.tool_options()["active"], json!(0));
    assert!(texts.iter().any(|(s, _)| s == "1  Front"), "{texts:?}");
    assert!(!texts.iter().any(|(s, _)| s == "Back"), "the list closed: {texts:?}");
}

/// With 1,000 artboards the list lays out at most 12 rows. It opens on the active
/// artboard's row, a swipe scrolls it to the last artboard, and choosing that row goes there.
#[test]
fn the_artboard_list_lays_out_only_the_rows_near_the_view() {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 100, "height": 100, "artboards": 1000})).unwrap();
    app.canvas_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(460.0, 260.0)));
    app.run("view.goToArtboard", json!({"index": 699})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let rows = |texts: &[(String, egui::Rect)]| texts.iter().filter(|(s, _)| s.starts_with("Artboard ")).cloned().collect::<Vec<_>>();
    let texts = status_texts(&mut app, &ctx, vec![]);
    let field = texts.iter().find(|(s, _)| s == "700  Artboard 700").map(|(_, r)| *r).expect("the field");
    let shown = rows(&status_click(&mut app, &ctx, field.center()));
    assert!((1..=12).contains(&shown.len()), "{} rows laid out", shown.len());
    let active = shown.iter().find(|(s, _)| s == "Artboard 700").map(|(_, r)| *r).expect("the active artboard's row in view");
    let swipe = |phase, dy| egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, dy),
        phase,
        modifiers: Default::default(),
        source: egui::MouseWheelSource::Unknown,
    };
    status_texts(
        &mut app,
        &ctx,
        vec![egui::Event::PointerMoved(active.center()), swipe(egui::TouchPhase::Start, 0.0), swipe(egui::TouchPhase::Move, -100_000.0)],
    );
    let shown = rows(&status_texts(&mut app, &ctx, vec![swipe(egui::TouchPhase::End, 0.0)]));
    assert!((1..=12).contains(&shown.len()), "{} rows laid out", shown.len());
    let last = shown.iter().find(|(s, _)| s == "Artboard 1000").map(|(_, r)| *r).expect("the last row after the swipe");
    status_click(&mut app, &ctx, last.center());
    assert_eq!(app.view().unwrap().artboard, 999);
}

fn artboard_navigation_app(tool: &str) -> (crate::VectorcraftApp, vectorcraft_doc::NodeId) {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.canvas_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(460.0, 260.0)));
    app.select_tool(tool);
    crate::i18n::set_current(crate::i18n::Lang::EN);
    (app, vectorcraft_doc::NodeId(id))
}

/// Click the actual status-bar Next control, without sending a canvas pointer event.
fn artboard_navigation_next(app: &mut crate::VectorcraftApp, ctx: &egui::Context) {
    let buttons = navigator(app, ctx, vec![]);
    assert_eq!(buttons.len(), 4);
    let at = buttons[2].center();
    assert!(!app.canvas_rect.unwrap().contains(at));
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    for event in [egui::Event::PointerMoved(at), button(true), button(false)] {
        navigator(app, ctx, vec![event]);
    }
}

fn artboard_navigation_properties_frame(app: &mut crate::VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<(String, egui::Rect)> {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 700.0));
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| {
        egui::Panel::right("navigation-properties").exact_size(300.0).show(ui, |ui| crate::panels::properties::show(app, ui));
    });
    out.textures_delta.clear();
    out.shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) => Some((t.galley.text().to_string(), egui::Rect::from_min_size(t.pos, t.galley.size()))),
            _ => None,
        })
        .collect()
}

/// Delete the shown artboard with `key`, or with Properties' Delete Artboard button.
fn trigger_artboard_delete(app: &mut crate::VectorcraftApp, ctx: &egui::Context, key: Option<egui::Key>, before: &vectorcraft_doc::Document) {
    if let Some(key) = key {
        let ev = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 300.0));
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events: vec![ev], ..Default::default() }, |ui| {
            crate::shortcuts::handle(app, ui.ctx());
            super::status_bar(app, ui);
        });
        out.textures_delta.clear();
    } else {
        artboard_navigation_properties_frame(app, ctx, vec![]);
        let texts = artboard_navigation_properties_frame(app, ctx, vec![]);
        assert!(texts.iter().any(|(label, _)| label == &before.artboards[1].name), "Properties shows the navigated board");
        let at = texts.iter().find(|(label, _)| label == "Delete Artboard").map(|(_, rect)| rect.center()).expect("Properties Delete button");
        assert!(!app.canvas_rect.unwrap().contains(at));
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        for event in [egui::Event::PointerMoved(at), button(true), button(false)] {
            artboard_navigation_properties_frame(app, ctx, vec![event]);
        }
    }
}

/// None clicks Properties' Delete Artboard; Some routes the real key through shortcuts.
fn artboard_navigation_delete(key: Option<egui::Key>) {
    for multiple in [false, true] {
        artboard_navigation_delete_case(key, multiple);
    }
}

fn artboard_navigation_delete_case(key: Option<egui::Key>, multiple: bool) {
    let (mut app, id) = artboard_navigation_app("artboard");
    let selected = if multiple { json!([0, 2]) } else { json!([0]) };
    app.session.set_tool_option("selected", &selected);
    assert_eq!(app.view().unwrap().artboard, 0);
    assert_eq!(app.session.tool_options()["active"], json!(0));
    assert_eq!(app.session.tool_options()["selected"], selected);
    app.session.active_mut().unwrap().mark_saved();
    let st = app.session.active().unwrap();
    let before = st.doc.clone();
    let selection = st.selection.clone();
    let undo = st.history.undo.len();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    navigate_to_next_board(&mut app, &ctx, &before, &selection, undo);
    delete_navigated_board(&mut app, &ctx, key, id, &before, &selection, undo);
}

fn navigate_to_next_board(
    app: &mut crate::VectorcraftApp,
    ctx: &egui::Context,
    before: &vectorcraft_doc::Document,
    selection: &vectorcraft_doc::Selection,
    undo: usize,
) {
    artboard_navigation_next(app, ctx);
    assert_eq!(app.view().unwrap().artboard, 1, "Next shows board 2");
    assert_eq!(app.session.tool_options()["active"], json!(1), "Next must change the destructive target before deletion");
    assert_eq!(app.session.tool_options()["selected"], json!([1]), "navigation replaces the old artboard multiselection");
    assert_eq!(app.view().unwrap().center, before.artboards[1].rect.center());
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.as_ref(), before, "navigation only changes view and tool state");
    assert_eq!(&st.selection, selection, "ordinary artwork stays selected");
    assert_eq!(st.history.undo.len(), undo);
    assert!(st.history.redo.is_empty());
    assert!(!st.is_dirty());
}

fn delete_navigated_board(
    app: &mut crate::VectorcraftApp,
    ctx: &egui::Context,
    key: Option<egui::Key>,
    id: vectorcraft_doc::NodeId,
    before: &vectorcraft_doc::Document,
    selection: &vectorcraft_doc::Selection,
    undo: usize,
) {
    trigger_artboard_delete(app, ctx, key, before);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.artboards, vec![before.artboards[0].clone(), before.artboards[2].clone()], "first and third original board records survive");
    assert_eq!(st.doc.node(id), before.node(id), "selected artwork survives unchanged");
    assert_eq!(&st.selection, selection);
    assert_eq!(st.history.undo.len(), undo + 1);
    assert_eq!(st.history.undo.last().unwrap().label, "Delete Artboard");
    assert!(st.history.redo.is_empty());
    assert!(st.is_dirty());
    let deleted = st.doc.clone();
    app.run("edit.undo", json!({})).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.as_ref(), before, "one Undo restores every board and the artwork");
    assert_eq!(&st.selection, selection);
    assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo, 1));
    assert!(!st.is_dirty());
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc, deleted);
}

#[test]
fn artboard_navigation_next_then_delete_uses_shown_board() {
    artboard_navigation_delete(Some(egui::Key::Delete));
}

#[test]
fn artboard_navigation_next_then_backspace_uses_shown_board() {
    artboard_navigation_delete(Some(egui::Key::Backspace));
}

#[test]
fn artboard_navigation_next_then_properties_delete_uses_shown_board() {
    artboard_navigation_delete(None);
}

/// Navigating artboards preserves other tools, their selections and undo history.
#[test]
fn artboard_navigation_preserves_other_tools_selection_and_history() {
    for tool in ["selection", "rectangle"] {
        let (mut app, _) = artboard_navigation_app(tool);
        app.run("object.move", json!({"dx": 7, "dy": 9})).unwrap();
        let redo = app.session.active().unwrap().doc.clone();
        app.run("edit.undo", json!({})).unwrap();
        app.session.active_mut().unwrap().mark_saved();
        let st = app.session.active().unwrap();
        let before = st.doc.clone();
        let selection = st.selection.clone();
        let history = (st.history.undo.len(), st.history.redo.len());
        assert_eq!(history.1, 1);
        let options = app.session.tool_options();

        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        artboard_navigation_next(&mut app, &ctx);

        assert_eq!(app.view().unwrap().artboard, 1);
        assert_eq!(app.view().unwrap().center, before.artboards[1].rect.center());
        assert_eq!(app.session.tool_id(), tool);
        assert_eq!(app.session.tool_options(), options);
        let st = app.session.active().unwrap();
        assert_eq!(st.doc, before);
        assert_eq!(st.selection, selection);
        assert_eq!((st.history.undo.len(), st.history.redo.len()), history);
        assert!(!st.is_dirty());
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc, redo, "navigation preserves the original Redo branch");
    }
}

#[test]
fn the_tab_title_names_the_opacity_mask_while_it_is_edited() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
    let title = |s: &Session| super::tab_title(s.active().unwrap(), 1.0, false, false);
    assert!(title(&s).ends_with("(RGB/Preview)"), "{}", title(&s));
    s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
    assert!(title(&s).ends_with("(<Opacity Mask>/Opacity Mask)"), "{}", title(&s));
    s.execute("transparency.stopEditingOpacityMask", &json!({})).unwrap();
    assert!(title(&s).ends_with("(RGB/Preview)"));
}

/// One headless frame of the Control bar; returns the isolation button's rectangle, if it shows.
fn isolation_button(app: &mut crate::VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Option<egui::Rect> {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 300.0));
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| super::control_bar(app, ui));
    out.textures_delta.clear();
    ctx.viewport(|vp| vp.prev_pass.widgets.get(egui::Id::unique(super::ISOLATION_BUTTON)).map(|w| w.rect))
}

/// Entering isolation mode leaves the canvas where it was: the breadcrumb bar is drawn over the
/// canvas and takes no layout height from it (#3126).
#[test]
fn the_isolation_bar_does_not_take_layout_height_from_the_canvas() {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap()["id"].as_u64().unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 700.0));
    let top = |app: &mut crate::VectorcraftApp| {
        let mut below = 0.0;
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
            super::doc_tabs(app, ui);
            below = ui.available_rect_before_wrap().top();
            let area = ui.available_rect_before_wrap();
            super::isolation_bar(app, ui, area);
            assert_eq!(ui.available_rect_before_wrap().top(), below, "the bar takes no layout space");
        });
        out.textures_delta.clear();
        below
    };
    let before = top(&mut app);
    app.run("object.isolate", json!({"id": id})).unwrap();
    assert!(app.session.active().unwrap().isolation.is_some());
    assert_eq!(top(&mut app), before, "isolation mode leaves the canvas where it was");
}

/// #4468: In nested isolation, clicking the back button steps up one level in the hierarchy,
/// and clicking ancestor breadcrumbs navigates directly to that isolation level.
#[test]
fn isolation_bar_navigation_and_back_button_steps_up() {
    use vectorcraft_doc::NodeId;
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    let rect = |app: &mut crate::VectorcraftApp, x: f64| {
        app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 30, "height": 20})).unwrap()["id"].as_u64().unwrap()
    };
    let (a, b) = (rect(&mut app, 10.0), rect(&mut app, 50.0));
    // Group 1 contains a and b
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let g1 = app.run("object.group", json!({})).unwrap()["id"].as_u64().unwrap();
    let c = rect(&mut app, 90.0);
    // Group 2 contains g1 and c
    app.run("select.set", json!({"ids": [g1, c]})).unwrap();
    let g2 = app.run("object.group", json!({})).unwrap()["id"].as_u64().unwrap();

    // Isolate path a (deepest level: Layer 1 > <Group> > <Group> > <Path>)
    app.run("object.isolate", json!({"id": a})).unwrap();
    assert_eq!(app.session.active().unwrap().isolation, Some(NodeId(a)));

    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    egui_extras::install_image_loaders(&ctx);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 700.0));

    let click_at = |app: &mut crate::VectorcraftApp, pos: egui::Pos2| {
        let button = egui::PointerButton::Primary;
        let mut run_frame = |events| {
            let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| {
                super::isolation_bar(app, ui, screen);
            });
            out.textures_delta.clear();
        };
        run_frame(vec![egui::Event::PointerMoved(pos)]);
        run_frame(vec![egui::Event::PointerButton { pos, button, pressed: true, modifiers: Default::default() }]);
        run_frame(vec![egui::Event::PointerButton { pos, button, pressed: false, modifiers: Default::default() }]);
    };

    // Clicking back from `a` steps up to parent group `g1`
    click_at(&mut app, egui::pos2(15.0, 12.0));
    assert_eq!(app.session.active().unwrap().isolation, Some(NodeId(g1)));

    // Clicking back from `g1` steps up to grandparent group `g2`
    click_at(&mut app, egui::pos2(15.0, 12.0));
    assert_eq!(app.session.active().unwrap().isolation, Some(NodeId(g2)));

    // Clicking back from `g2` (parent is layer) exits isolation mode completely
    click_at(&mut app, egui::pos2(15.0, 12.0));
    assert_eq!(app.session.active().unwrap().isolation, None);

    // Deep isolate again to test clicking ancestor breadcrumb directly
    app.run("object.isolate", json!({"id": a})).unwrap();
    assert_eq!(app.session.active().unwrap().isolation, Some(NodeId(a)));

    // First crumb is Layer 1 (at x ~ 30..80). Clicking it exits isolation directly.
    click_at(&mut app, egui::pos2(45.0, 12.0));
    assert_eq!(app.session.active().unwrap().isolation, None);

    // Deep isolate again to test clicking an intermediate ancestor group breadcrumb directly
    app.run("object.isolate", json!({"id": a})).unwrap();
    assert_eq!(app.session.active().unwrap().isolation, Some(NodeId(a)));

    // Second crumb is g2 ("<Group>") at x ~ 95..140.
    // Clicking g2 isolates g2 directly.
    click_at(&mut app, egui::pos2(115.0, 12.0));
    assert_eq!(app.session.active().unwrap().isolation, Some(NodeId(g2)));
}

/// The Control bar's Isolate Selected Object isolates a path inside a group that Direct
/// Selection selected, and Exit Isolation Mode leaves with the path selected. Type, symbol
/// instances, blends, two objects and an envelope's contents get no isolate button, and pattern
/// editing gets no exit button.
#[test]
fn the_control_bar_isolates_the_selected_object_and_exits_isolation_mode() {
    use vectorcraft_doc::NodeId;
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    let rect = |app: &mut crate::VectorcraftApp, x: f64| {
        app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 30, "height": 20})).unwrap()["id"].as_u64().unwrap()
    };
    let (a, b) = (rect(&mut app, 10.0), rect(&mut app, 50.0));
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    app.run("object.group", json!({})).unwrap();
    app.select_tool("directSelection");
    app.run("select.anchors", json!({"id": a, "anchors": [[0, 0]]})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let click = |app: &mut crate::VectorcraftApp| {
        let r = isolation_button(app, &ctx, vec![]).expect("the bar shows the isolation button");
        let press =
            |pressed| egui::Event::PointerButton { pos: r.center(), button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        for e in [egui::Event::PointerMoved(r.center()), press(true), press(false)] {
            isolation_button(app, &ctx, vec![e]);
        }
    };
    click(&mut app);
    assert_eq!(app.session.active().unwrap().isolation, Some(NodeId(a)), "Isolate Selected Object isolates the path");
    click(&mut app);
    let st = app.session.active().unwrap();
    assert_eq!((st.isolation, st.selection.objects.clone()), (None, vec![NodeId(a)]), "Exit Isolation Mode selects the path");
    app.select_tool("selection");
    // Type, a symbol instance, a blend and two objects.
    let text = app.run("text.create", json!({"x": 10, "y": 100, "text": "Hi"})).unwrap()["id"].as_u64().unwrap();
    let c = rect(&mut app, 100.0);
    app.run("select.set", json!({"ids": [c]})).unwrap();
    let symbol = app.run("symbol.new", json!({"name": "Box"})).unwrap()["id"].as_u64().unwrap();
    let (d, e) = (rect(&mut app, 150.0), rect(&mut app, 250.0));
    let blend = app.run("object.blend.make", json!({"ids": [d, e], "steps": 1})).unwrap()["id"].as_u64().unwrap();
    for ids in [json!([text]), json!([symbol]), json!([blend]), json!([text, symbol])] {
        app.run("select.set", json!({ "ids": ids })).unwrap();
        assert_eq!(isolation_button(&mut app, &ctx, vec![]), None, "{ids}");
    }
    // An envelope's contents, while Edit Contents is on.
    let g = rect(&mut app, 350.0);
    app.run("select.set", json!({"ids": [g]})).unwrap();
    app.run("object.envelope.makeWithWarp", json!({})).unwrap();
    app.run("object.envelope.editContents", json!({"editing": true})).unwrap();
    assert_eq!(app.session.active().unwrap().selection.objects, vec![NodeId(g)]);
    assert_eq!(isolation_button(&mut app, &ctx, vec![]), None, "the envelope's controls show instead");
    let f = rect(&mut app, 300.0);
    app.run("select.set", json!({"ids": [f]})).unwrap();
    app.run("object.pattern.make", json!({"name": "Dots", "width": 20, "height": 20})).unwrap();
    assert!(app.session.active().unwrap().isolation.is_some());
    assert_eq!(isolation_button(&mut app, &ctx, vec![]), None, "the isolation bar ends pattern editing");
}

/// The Zoom tool's hint follows Performance › Animated Zoom (#394): a drag zooms as it goes,
/// or (off) zooms into the area dragged across.
#[test]
fn the_zoom_hint_follows_animated_zoom() {
    fn texts(s: &egui::Shape, out: &mut String) {
        match s {
            egui::Shape::Text(t) => out.push_str(t.galley.text()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.select_tool("zoom");
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let hint = |app: &mut crate::VectorcraftApp| {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::hint_bar(app, ui));
        out.textures_delta.clear();
        let mut shown = String::new();
        out.shapes.iter().for_each(|c| texts(&c.shape, &mut shown));
        shown
    };
    let on = hint(&mut app);
    assert!(on.contains(" right or left to zoom in or out") && on.contains("Hold"), "{on}");
    app.run("prefs.set", json!({"key": "animatedZoom", "value": false})).unwrap();
    let off = hint(&mut app);
    assert!(off.contains(" to zoom into an area") && !off.contains("Hold"), "{off}");
}

fn title_test_app(custom_titlebar: bool) -> crate::VectorcraftApp {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.custom_titlebar = custom_titlebar;
    // The preference that makes the shell launch without its own title bar.
    app.run("prefs.set", json!({"key": "systemTitleBar", "value": !custom_titlebar})).unwrap();
    app
}

#[test]
fn without_the_preference_the_mark_stays() {
    // The default (e.g. the web build): no custom bar, no preference.
    let app = crate::VectorcraftApp::new(Session::new(), Default::default());
    assert!(!super::system_title_bar(&app));
}

#[test]
fn window_title_with_no_document_is_the_app_name() {
    assert_eq!(super::window_title(&title_test_app(true)), "VectorCraft");
}

#[test]
fn window_title_follows_the_active_file() {
    let mut app = title_test_app(true);
    app.run("file.new", json!({"width": 100, "height": 100, "name": "foo.svg"})).unwrap();
    assert_eq!(super::window_title(&app), "foo.svg \u{2014} VectorCraft");
    // Unsaved changes gain a `*` prefix, like the `*` in the document tabs.
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    assert_eq!(super::window_title(&app), "*foo.svg \u{2014} VectorCraft");
}

#[test]
fn system_mode_hides_the_mark() {
    for custom in [true, false] {
        let mut app = title_test_app(custom);
        app.run("file.new", json!({"width": 100, "height": 100, "name": "foo.svg"})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::app_bar(&mut app, ui));
        let uploads = out.textures_delta.set.values().flat_map(|d| d.iter()).filter(|d| d.image.size() == [128, 128]).count();
        out.textures_delta.clear();
        let marks: Vec<egui::Rect> = out
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Mesh(m) if m.texture_id == crate::brand::texture_id(&ctx) => Some(m.calc_bounds()),
                _ => None,
            })
            .collect();
        let system = super::system_title_bar(&app);
        assert_eq!(system, !custom && !cfg!(target_os = "macos"));
        assert_eq!(marks.is_empty(), system, "mark in system mode (custom={custom})");
        assert_eq!(uploads, if system { 0 } else { 1 }, "mark texture in system mode (custom={custom})");
    }
}

#[test]
fn sync_window_title_sends_only_on_change() {
    let mut app = title_test_app(true);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    super::sync_window_title(&mut app, &ctx);
    assert_eq!(app.last_window_title, "VectorCraft");
    let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
    out.textures_delta.clear();
    let cmds = out.viewport_output.remove(&egui::ViewportId::ROOT).map(|v| v.commands).unwrap_or_default();
    assert!(cmds.iter().any(|c| matches!(c, egui::ViewportCommand::Title(t) if t == "VectorCraft")), "{cmds:?}");
    // Same document state again: the cached title stops a repeat command.
    super::sync_window_title(&mut app, &ctx);
    let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
    out.textures_delta.clear();
    let cmds = out.viewport_output.remove(&egui::ViewportId::ROOT).map(|v| v.commands).unwrap_or_default();
    assert!(!cmds.iter().any(|c| matches!(c, egui::ViewportCommand::Title(_))), "repeat Title: {cmds:?}");
    app.run("file.new", json!({"width": 100, "height": 100, "name": "foo.svg"})).unwrap();
    super::sync_window_title(&mut app, &ctx);
    assert_eq!(app.last_window_title, "foo.svg \u{2014} VectorCraft");
}
/// Draw actual tabs, including the same per-frame view synchronization as the app.
fn artboard_document_tabs_frame(app: &mut crate::VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<egui::Rect> {
    app.sync_views();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 700.0));
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| super::doc_tabs(app, ui));
    out.textures_delta.clear();
    let mut titles: Vec<_> = out
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(t) if t.galley.text().contains(" @ ") => Some(t.visual_bounding_rect()),
            _ => None,
        })
        .collect();
    titles.sort_by(|a, b| a.left().total_cmp(&b.left()));
    titles
}

fn artboard_document_click_tab(app: &mut crate::VectorcraftApp, ctx: &egui::Context, index: usize) {
    let titles = artboard_document_tabs_frame(app, ctx, vec![]);
    assert_eq!(titles.len(), 2, "both real document tabs must be visible");
    let at = titles[index].center();
    assert!(!app.canvas_rect.unwrap().contains(at));
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    for event in [egui::Event::PointerMoved(at), button(true), button(false)] {
        artboard_document_tabs_frame(app, ctx, vec![event]);
    }
    assert_eq!(app.session.active_index(), Some(index), "the actual tab click must activate its document");
}

fn artboard_document_app() -> (crate::VectorcraftApp, egui::Context) {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100, "artboards": 2})).unwrap();
    app.run("artboard.setProps", json!({"index": 0, "name": "Keep"})).unwrap();
    app.run("artboard.setProps", json!({"index": 1, "name": "Shown"})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
    app.run("symbol.new", json!({"name": "Tab symbol"})).unwrap();
    app.run("shape.rectangle", json!({"x": 230, "y": 10, "width": 30, "height": 20})).unwrap();
    app.run("guide.add", json!({"artboard": 0, "pos": 20})).unwrap();
    app.run("guide.add", json!({"artboard": 1, "pos": 250, "vertical": true})).unwrap();
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    // Neither tab nor Properties clicks enter the canvas; a canvas move could hide the bug.
    app.canvas_rect = Some(egui::Rect::from_min_max(egui::pos2(0.0, 40.0), egui::pos2(800.0, 260.0)));
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    artboard_document_click_tab(&mut app, &ctx, 0);
    (app, ctx)
}

#[test]
fn artboard_document_tab_roundtrip_properties_delete_uses_shown_board() {
    let (mut app, ctx) = artboard_document_app();
    app.select_tool("artboard");
    app.run("tool.setOption", json!({"key": "moveArt", "value": false})).unwrap();
    app.run("tool.setOption", json!({"key": "scaleArt", "value": true})).unwrap();
    artboard_navigation_next(&mut app, &ctx);
    assert_eq!(app.view().unwrap().artboard, 1);
    assert_eq!(app.session.tool_options()["selected"], json!([1]), "start with the tool and navigator aligned");
    app.session.active_mut().unwrap().mark_saved();
    let before = app.session.active().unwrap().doc.clone();
    let selection = app.session.active().unwrap().selection.clone();
    let undo = app.session.active().unwrap().history.undo.len();
    let other = app.session.documents()[1].doc.clone();
    let other_history = (app.session.documents()[1].history.undo.len(), app.session.documents()[1].history.redo.len());

    // No direct set_active, tool reselection, or selected-option repair after this round trip.
    artboard_document_click_tab(&mut app, &ctx, 1);
    artboard_document_click_tab(&mut app, &ctx, 0);
    assert_eq!(app.view().unwrap().artboard, 1, "the document keeps its displayed artboard");
    assert_eq!(app.session.active().unwrap().doc, before, "switching tabs is not a document edit");
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
    assert!(!app.session.active().unwrap().is_dirty());
    let selected_before_delete = app.session.tool_options()["selected"].clone();
    let active_before_delete = app.session.tool_options()["active"].clone();
    assert_eq!(app.session.tool_options()["moveArt"], json!(false));
    assert_eq!(app.session.tool_options()["scaleArt"], json!(true));

    artboard_navigation_properties_frame(&mut app, &ctx, vec![]);
    let texts = artboard_navigation_properties_frame(&mut app, &ctx, vec![]);
    assert!(texts.iter().any(|(label, _)| label == "Shown"), "Properties must display the second board before deletion");
    let at = texts.iter().find(|(label, _)| label == "Delete Artboard").map(|(_, r)| r.center()).expect("actual Properties Delete button");
    assert!(!app.canvas_rect.unwrap().contains(at));
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let commands = app.run_count;
    for event in [egui::Event::PointerMoved(at), button(true), button(false)] {
        artboard_navigation_properties_frame(&mut app, &ctx, vec![event]);
    }
    let st = app.session.active().unwrap();
    assert_eq!(
        st.doc.artboards.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
        vec!["Keep"],
        "Properties Delete after a document tab round trip must remove the shown board"
    );
    assert_eq!(selected_before_delete, json!([1]));
    assert_eq!(active_before_delete, json!(1));
    assert_eq!(st.doc.artboards, vec![before.artboards[0].clone()], "the original first board survives");
    assert_eq!(st.doc.guides, vec![before.guides[0].clone()], "only the deleted board's guide is removed");
    assert_eq!(st.doc.layers, before.layers, "artwork and symbol instances stay unchanged");
    assert_eq!(st.doc.symbols, before.symbols, "symbol definitions stay unchanged");
    assert_eq!(st.selection, selection);
    assert_eq!(app.run_count, commands.wrapping_add(1), "one command from the actual Properties control");
    assert_eq!(st.history.undo.len(), undo + 1);
    assert_eq!(st.history.undo.last().unwrap().label, "Delete Artboard");
    assert!(st.history.redo.is_empty());
    assert!(st.is_dirty());
    let deleted = st.doc.clone();
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc, before, "one Undo restores the correct board, guides and art");
    assert!(!app.session.active().unwrap().is_dirty());
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc, deleted);
    assert_eq!(app.session.documents()[1].doc, other);
    assert_eq!((app.session.documents()[1].history.undo.len(), app.session.documents()[1].history.redo.len()), other_history);
}

#[test]
fn artboard_document_tab_same_document_preserves_multiselection_and_options() {
    let (mut app, ctx) = artboard_document_app();
    app.select_tool("artboard");
    artboard_navigation_next(&mut app, &ctx);
    app.run("tool.setOption", json!({"key": "selected", "value": [1, 0]})).unwrap();
    app.run("tool.setOption", json!({"key": "moveArt", "value": false})).unwrap();
    app.run("tool.setOption", json!({"key": "scaleArt", "value": true})).unwrap();
    let options = app.session.tool_options();
    let before = app.session.active().unwrap().doc.clone();
    let history = (app.session.active().unwrap().history.undo.len(), app.session.active().unwrap().history.redo.len());
    artboard_document_click_tab(&mut app, &ctx, 0);
    for _ in 0..3 {
        artboard_document_tabs_frame(&mut app, &ctx, vec![]);
    }
    assert_eq!(app.session.tool_options(), options, "an already-active tab must not collapse valid multiselection or options");
    assert_eq!(app.view().unwrap().artboard, 1);
    assert_eq!(app.session.active().unwrap().doc, before);
    assert_eq!((app.session.active().unwrap().history.undo.len(), app.session.active().unwrap().history.redo.len()), history);
}

#[test]
fn artboard_document_tab_roundtrip_preserves_other_tools() {
    for tool in ["selection", "rectangle"] {
        let (mut app, ctx) = artboard_document_app();
        app.select_tool(tool);
        artboard_navigation_next(&mut app, &ctx);
        let before = app.session.active().unwrap().doc.clone();
        let selection = app.session.active().unwrap().selection.clone();
        let options = app.session.tool_options();
        let history = (app.session.active().unwrap().history.undo.len(), app.session.active().unwrap().history.redo.len());
        artboard_document_click_tab(&mut app, &ctx, 1);
        artboard_document_click_tab(&mut app, &ctx, 0);
        assert_eq!(app.session.tool_id(), tool);
        assert_eq!(app.session.tool_options(), options);
        assert_eq!(app.view().unwrap().artboard, 1);
        assert_eq!(app.session.active().unwrap().doc, before);
        assert_eq!(app.session.active().unwrap().selection, selection);
        assert_eq!((app.session.active().unwrap().history.undo.len(), app.session.active().unwrap().history.redo.len()), history);
    }
}

/// One history entry: its label, document and selection.
type HistoryRow = (String, Arc<vectorcraft_doc::Document>, Arc<vectorcraft_doc::Selection>);

/// The undo and redo entries of a document.
type HistoryDump = (Vec<HistoryRow>, Vec<HistoryRow>);

fn history_of(st: &vectorcraft_engine::DocState) -> HistoryDump {
    let entries =
        |items: &[vectorcraft_engine::HistoryEntry]| items.iter().map(|e| (e.label.clone(), e.doc.clone(), e.selection.clone())).collect::<Vec<_>>();
    (entries(&st.history.undo), entries(&st.history.redo))
}

fn assert_outside_canvas(app: &crate::VectorcraftApp, ctx: &egui::Context) {
    let p = ctx.input(|i| i.pointer.hover_pos()).expect("a real chrome or panel pointer position");
    assert!(!app.canvas_rect.unwrap().contains(p), "the pointer must stay outside the canvas: {p:?}");
}

/// Document A as it was saved, before its second tab closed and its shown board was deleted.
struct Baseline {
    uid: u64,
    revision: u64,
    before: Arc<vectorcraft_doc::Document>,
    selection: vectorcraft_doc::Selection,
    history_before: HistoryDump,
    history_limits: (usize, usize),
    expected: vectorcraft_doc::Document,
}

/// Close document B's tab with its close box, leaving A as it was.
fn close_b_tab(app: &mut crate::VectorcraftApp, ctx: &egui::Context, base: &Baseline) {
    let Baseline { uid, revision, before, selection, history_before, history_limits, expected: _ } = base;
    // Route from the status bar along the panel edge to the tab strip, never through canvas.
    for at in [egui::pos2(1000.0, 280.0), egui::pos2(1000.0, 20.0)] {
        artboard_document_tabs_frame(app, ctx, vec![egui::Event::PointerMoved(at)]);
        assert_outside_canvas(app, ctx);
    }
    artboard_document_click_tab(app, ctx, 1);
    assert_outside_canvas(app, ctx);
    app.session.active_mut().unwrap().mark_saved();
    let other_uid = app.session.active().unwrap().uid;
    assert_ne!(other_uid, *uid);
    assert!(app.session.documents().iter().all(|d| !d.is_dirty()), "both ordinary documents are in saved state");
    assert!(app.ui.dialog.is_none());
    eprintln!("ARTBOARD_CLOSE_TARGET_SETUP_READY");

    // The actual B tab x, using the existing close-box test's title-relative hit position.
    let titles = artboard_document_tabs_frame(app, ctx, vec![]);
    assert_outside_canvas(app, ctx);
    assert_eq!(titles.len(), 2);
    let title = titles[1];
    let x = if super::CLOSE_BEFORE_TITLE { title.left() - 16.0 } else { title.right() + 20.0 };
    let at = egui::pos2(x, title.center().y);
    assert!(!app.canvas_rect.unwrap().contains(at));
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    for event in [egui::Event::PointerMoved(at), button(true), button(false)] {
        artboard_document_tabs_frame(app, ctx, vec![event]);
        assert_outside_canvas(app, ctx);
    }
    assert_eq!(app.session.documents().len(), 1, "B tab x must really close B");
    assert_eq!(app.session.active_index(), Some(0));
    assert_eq!(app.session.active().unwrap().uid, *uid, "the survivor is the original A, not a replacement document");
    assert!(app.session.documents().iter().all(|d| d.uid != other_uid));
    assert!(app.ui.dialog.is_none(), "a clean close must not leave a save/discard modal");
    for _ in 0..2 {
        artboard_document_tabs_frame(app, ctx, vec![]);
        assert_outside_canvas(app, ctx);
    }
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.as_ref(), before.as_ref(), "closing B must not edit A");
    assert_eq!(&st.selection, selection);
    assert_eq!(history_of(st), *history_before, "closing B preserves all A Undo/Redo entries");
    assert_eq!(st.revision, *revision);
    assert_eq!((st.history.limit, st.history.byte_budget), *history_limits);
    assert!(!st.is_dirty());
    assert_eq!(app.view().unwrap().artboard, 1);
    eprintln!("ARTBOARD_CLOSE_TARGET_CLOSED_B");
}

/// Delete the board Properties shows, then Undo and Redo it.
fn delete_shown_board(app: &mut crate::VectorcraftApp, ctx: &egui::Context, base: &Baseline) {
    let Baseline { uid, revision, before, selection, history_before, history_limits, expected } = base;
    // Cross the tab strip to the Properties panel before moving down to its Delete control.
    app.sync_views();
    artboard_navigation_properties_frame(app, ctx, vec![egui::Event::PointerMoved(egui::pos2(1000.0, 20.0))]);
    assert_outside_canvas(app, ctx);
    app.sync_views();
    let texts = artboard_navigation_properties_frame(app, ctx, vec![]);
    assert_outside_canvas(app, ctx);
    assert!(texts.iter().any(|(label, _)| label == "Shown"), "actual Properties must display Shown before deletion");
    assert_eq!(app.view().unwrap().artboard, 1);
    let tool_before_delete = app.session.tool_options();
    eprintln!("ARTBOARD_CLOSE_TARGET_SHOWN_DISPLAYED tool={tool_before_delete}");
    let at = texts.iter().find(|(label, _)| label == "Delete Artboard").map(|(_, r)| r.center()).expect("actual Properties Delete button");
    assert!(at.x > app.canvas_rect.unwrap().right(), "Delete is in the right panel");
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let commands = app.run_count;
    for event in [egui::Event::PointerMoved(at), button(true), button(false)] {
        app.sync_views();
        artboard_navigation_properties_frame(app, ctx, vec![event]);
        assert_outside_canvas(app, ctx);
    }
    let st = app.session.active().unwrap();
    assert_eq!(app.run_count, commands.wrapping_add(1), "the actual Properties click executes one command");
    assert_eq!(st.doc.artboards.len(), 1, "the real Delete action removed one board");
    assert_eq!(st.history.undo.len(), history_before.0.len() + 1);
    assert_eq!(st.history.undo.last().unwrap().label, "Delete Artboard");
    assert_eq!((st.history.limit, st.history.byte_budget), *history_limits);
    eprintln!("ARTBOARD_CLOSE_TARGET_DELETE_CLICKED tool_before={tool_before_delete}");
    // Do not assert tool alignment before this destructive-action oracle: it must expose the wrong deletion.
    assert_eq!(
        st.doc.artboards.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
        vec!["Keep"],
        "ARTBOARD_CLOSE_TARGET_WRONG_SURVIVOR: Properties showing Shown must delete Shown after closing B"
    );
    assert_eq!(st.doc.as_ref(), expected, "only Shown and its attached guide may be removed from the full document");
    assert_eq!(st.doc.layers, before.layers, "all artwork and symbol instances stay unchanged");
    assert_eq!(st.doc.symbols, before.symbols);
    assert_eq!(st.doc.guides, vec![before.guides[0].clone()]);
    assert_eq!(&st.selection, selection);
    assert_eq!(st.uid, *uid);
    assert_eq!(st.revision, revision + 1);
    undo_and_redo_the_deletion(app, base);
}

/// Undo and Redo of the deletion that [`delete_shown_board`] made.
fn undo_and_redo_the_deletion(app: &mut crate::VectorcraftApp, base: &Baseline) {
    let Baseline { uid, before, selection, history_before, history_limits, expected, revision: _ } = base;
    let st = app.session.active().unwrap();
    let history_after = history_of(st);
    assert_eq!(&history_after.0[..history_before.0.len()], history_before.0.as_slice());
    let entry = history_after.0.last().unwrap();
    assert_eq!(entry.1.as_ref(), before.as_ref());
    assert_eq!(entry.2.as_ref(), selection);
    assert!(history_after.1.is_empty());
    assert!(st.is_dirty());
    let deleted = st.doc.clone();
    app.run("edit.undo", json!({})).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.uid, *uid);
    assert_eq!(st.doc.as_ref(), before.as_ref(), "one Undo restores the full saved A including both boards, guides and artwork");
    assert_eq!(&st.selection, selection);
    assert_eq!(history_of(st).0, history_before.0);
    assert_eq!(st.history.redo.len(), 1);
    assert_eq!(st.history.redo[0].label, "Delete Artboard");
    assert_eq!(st.history.redo[0].doc.as_ref(), deleted.as_ref());
    assert_eq!(st.history.redo[0].selection.as_ref(), selection);
    assert!(!st.is_dirty());
    app.run("edit.redo", json!({})).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.uid, *uid);
    assert_eq!(st.doc.as_ref(), expected, "one Redo repeats only the correct deletion");
    assert_eq!(&st.selection, selection);
    assert_eq!(history_of(st), history_after);
    assert_eq!((st.history.limit, st.history.byte_budget), *history_limits);
    assert!(st.is_dirty());
    assert_eq!(app.session.documents().len(), 1, "Undo/Redo do not reopen B");
    eprintln!("ARTBOARD_CLOSE_TARGET_MODEL_AND_HISTORY_PRESERVED");
}

#[test]
fn artboard_close_active_tab_properties_delete_uses_shown_board() {
    let (mut app, ctx) = artboard_document_app();
    app.select_tool("artboard");
    app.run("tool.setOption", json!({"key": "moveArt", "value": false})).unwrap();
    app.run("tool.setOption", json!({"key": "scaleArt", "value": true})).unwrap();
    artboard_navigation_next(&mut app, &ctx);
    assert_outside_canvas(&app, &ctx);
    assert_eq!(app.view().unwrap().artboard, 1);
    assert_eq!(app.session.tool_options()["selected"], json!([1]), "the initial navigator action targets Shown");
    app.session.active_mut().unwrap().mark_saved();
    let st = app.session.active().unwrap();
    let (uid, revision) = (st.uid, st.revision);
    let before = st.doc.clone();
    let selection = st.selection.clone();
    let history_before = history_of(st);
    let history_limits = (st.history.limit, st.history.byte_budget);
    assert_eq!(before.artboards.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), vec!["Keep", "Shown"]);
    assert_eq!(before.guides.len(), 2);
    assert!(!selection.objects.is_empty());
    assert!(history_before.1.is_empty());
    assert!(!st.is_dirty());
    let mut expected = before.as_ref().clone();
    expected.artboards = vec![before.artboards[0].clone()];
    expected.guides = vec![before.guides[0].clone()];
    let base = Baseline { uid, revision, before, selection, history_before, history_limits, expected };
    close_b_tab(&mut app, &ctx, &base);
    delete_shown_board(&mut app, &ctx, &base);
}
