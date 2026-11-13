/// With more tabs than fit, the strip starts late enough to show the active one (#746).
#[test]
fn the_active_tab_stays_in_view() {
    let widths = [100.0; 10];
    assert_eq!(super::first_tab_shown(&widths, Some(2), 350.0), 0, "it fits from the start");
    assert_eq!(super::first_tab_shown(&widths, Some(9), 350.0), 7, "the last three tabs shown");
    assert_eq!(super::first_tab_shown(&widths, None, 350.0), 0);
    assert_eq!(super::first_tab_shown(&widths, Some(4), 50.0), 4, "a tab wider than the room still shows");
}

/// Tabs 100 wide at 0, 100, 200…, for documents `docs` (a document listed twice has two views).
fn slots(docs: &[usize]) -> Vec<(usize, egui::Rect)> {
    docs.iter().enumerate().map(|(k, &d)| (d, egui::Rect::from_min_size(egui::pos2(k as f32 * 100.0, 0.0), egui::vec2(100.0, 30.0)))).collect()
}

/// A document's view tabs make one group, which moves as one (#5663).
#[test]
fn a_documents_view_tabs_make_one_group() {
    let groups = super::tab_groups(&slots(&[0, 1, 1, 2]));
    let spans: Vec<(usize, f32, f32)> = groups.iter().map(|(d, s)| (*d, s.min, s.max)).collect();
    assert_eq!(spans, vec![(0, 0.0, 100.0), (1, 100.0, 300.0), (2, 300.0, 400.0)]);
    assert!(super::tab_groups(&[]).is_empty());
}

/// A dragged tab drops past the middles of the tabs it crossed, as an index among all the open
/// documents (#5663).
#[test]
fn a_dragged_tab_drops_past_the_middles_it_crossed() {
    let groups = super::tab_groups(&slots(&[0, 1, 2, 3]));
    let drop = |from, x| super::tab_drop(&groups, from, x);
    assert_eq!(drop(0, 40.0), None, "still over itself");
    assert_eq!(drop(0, 140.0), None, "not yet past the next tab's middle");
    assert_eq!(drop(0, 160.0), Some((1, 200.0)), "past the second tab's middle: between it and the third");
    assert_eq!(drop(0, 900.0), Some((3, 400.0)), "past the end: last");
    assert_eq!(drop(3, -50.0), Some((0, 0.0)), "before the start: first");
    assert_eq!(drop(3, 120.0), Some((1, 100.0)), "between the first two");
    assert_eq!(drop(2, 260.0), None, "between its neighbours: where it is");
    assert_eq!(super::tab_drop(&super::tab_groups(&slots(&[0])), 0, 500.0), None, "no other tab");
    assert_eq!(super::tab_drop(&[], 0, 500.0), None);
}

/// Documents without a tab on the strip (floating, or past its end) keep their places in the
/// documents' order; the drop names an index among all of them (#5663).
#[test]
fn a_drop_counts_the_documents_the_strip_does_not_show() {
    // Documents 0, 2 and 5 have tabs; 1, 3 and 4 float.
    let groups = super::tab_groups(&slots(&[0, 2, 5]));
    let drop = |from, x| super::tab_drop(&groups, from, x).map(|(to, _)| to);
    assert_eq!(drop(0, 260.0), Some(5), "after document 5: last, as 0 leaves its place");
    assert_eq!(drop(0, 160.0), Some(4), "between 2 and 5: just before 5, which is index 4 once 0 has left its place");
    assert_eq!(drop(5, -10.0), Some(0), "before document 0");
    assert_eq!(drop(5, 60.0), Some(2), "before document 2: index 2, the floating document 1 stays before it");
    assert_eq!(drop(2, 260.0), Some(5), "after document 5");
    // With view tabs: document 1 has two, side by side; it moves as one.
    let groups = super::tab_groups(&slots(&[0, 1, 1, 2]));
    assert_eq!(super::tab_drop(&groups, 1, 20.0), Some((0, 0.0)), "both views' document goes first");
    assert_eq!(super::tab_drop(&groups, 0, 220.0), Some((1, 300.0)), "past document 1's middle, which both its tabs span");
    assert_eq!(super::tab_drop(&groups, 0, 190.0), None, "not yet past it");
}
use std::sync::Arc;

use super::{StatusReadoutWidths, status_readouts};
use crate::theme::Tokens;
use serde_json::json;
use vectorcraft_engine::Session;

/// `layers` without their ruler guides: the art.
fn art_of(layers: &[Arc<vectorcraft_doc::Node>]) -> Vec<vectorcraft_doc::Node> {
    layers
        .iter()
        .map(|l| {
            let mut l = vectorcraft_doc::Node::clone(l);
            l.strip_ruler_guides();
            l
        })
        .collect()
}

#[test]
fn help_bar_and_status_bar_render_independently() {
    for (help, status, screen_mode) in [(true, true, 0), (false, true, 0), (true, false, 0), (false, false, 0), (true, true, 3)] {
        let (mut app, ctx) = status_readout_app();
        app.ui.help_bar = help;
        app.ui.status_bar = status;
        app.ui.screen_mode = screen_mode;
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
        let mut parent = None;
        let mut output = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
            parent = Some(ui.scope_id());
            app.ui(ui);
        });
        output.textures_delta.clear();
        for _ in 0..2 {
            output = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| app.ui(ui));
            output.textures_delta.clear();
        }
        fn text(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| text(s, out)),
                _ => {}
            }
        }
        let mut shown = Vec::new();
        for shape in &output.shapes {
            text(&shape.shape, &mut shown);
        }
        assert_eq!(shown.iter().any(|t| t == "?"), help && screen_mode < 3, "{help}/{status}/{screen_mode}: {shown:?}");
        assert_eq!(shown.iter().any(|t| t.contains("Saved recovery data")), status && screen_mode < 3, "{help}/{status}/{screen_mode}: {shown:?}");
        let floor = ctx.data(|data| data.get_temp::<f32>(egui::Id::unique("menu_floor").with(ctx.viewport_id()))).unwrap();
        let expected = if screen_mode < 3 && (help || status) {
            let id = parent.unwrap().with(if help { "hint_bar" } else { "status_bar" });
            egui::containers::panel::PanelState::load(&ctx, id).unwrap().outer_rect.top()
        } else {
            screen.bottom()
        };
        assert!((floor - expected).abs() < 1.0, "menu floor {floor}, expected {expected}");
    }
}

#[test]
#[ignore = "Renders the Help Bar visibility examples; set HELP_BAR_SCREENSHOTS to an output directory"]
fn help_bar_visibility_screenshots() {
    let dir = std::path::PathBuf::from(std::env::var_os("HELP_BAR_SCREENSHOTS").expect("output directory"));
    for shown in [true, false] {
        let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 500, "height": 400})).unwrap();
        if !shown {
            crate::menus::run_ui_command(&mut app, "window.helpBar", &serde_json::Value::Null).unwrap().unwrap();
        }
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1000.0, 700.0))
            .with_accessibility_check(false)
            .wgpu()
            .build_ui_state(|ui, app: &mut crate::VectorcraftApp| app.ui(ui), app);
        h.run_steps(6);
        h.render().unwrap().save(dir.join(if shown { "help-visible.png" } else { "help-hidden.png" })).unwrap();
    }
}

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
    bar_frame(app, ctx, width, None, events, super::status_bar).0
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

/// A function that draws one of the bars at the bottom of the window.
type DrawBar = fn(&mut crate::VectorcraftApp, &mut egui::Ui);

/// The texts (positions, clips and elision state) and the screen-reader tree of one frame of
/// `draw` in a window `width` points wide, at `time` with `events`.
fn bar_frame(
    app: &mut crate::VectorcraftApp,
    ctx: &egui::Context,
    width: f32,
    time: Option<f64>,
    events: Vec<egui::Event>,
    draw: DrawBar,
) -> (Vec<StatusText>, Vec<egui::accesskit::Node>) {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 300.0));
    let input = egui::RawInput { screen_rect: Some(screen), time, events, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| draw(app, ui));
    out.textures_delta.clear();
    let texts = out
        .shapes
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
        .collect();
    let nodes = out.platform_output.accesskit_update.map(|u| u.nodes.into_iter().map(|(_, n)| n).collect()).unwrap_or_default();
    (texts, nodes)
}

/// The texts of `draw` with the pointer away, the screen-reader tree, and the texts after the
/// pointer has rested on the first text that starts with `prefix` for longer than the tooltip delay.
fn bar_hover(
    app: &mut crate::VectorcraftApp,
    ctx: &egui::Context,
    width: f32,
    prefix: &str,
    draw: DrawBar,
) -> (Vec<StatusText>, Vec<egui::accesskit::Node>, Vec<StatusText>) {
    let t = ctx.input(|i| i.time) + 1.0;
    bar_frame(app, ctx, width, Some(t), vec![egui::Event::PointerGone], draw);
    let (texts, nodes) = bar_frame(app, ctx, width, Some(t + 0.1), vec![], draw);
    let at = status_text(&texts, prefix);
    let pos = at.clip.intersect(at.bounds).center();
    bar_frame(app, ctx, width, Some(t + 1.0), vec![egui::Event::PointerMoved(pos)], draw);
    bar_frame(app, ctx, width, Some(t + 3.0), vec![], draw);
    let (hovered, _) = bar_frame(app, ctx, width, Some(t + 3.1), vec![], draw);
    (texts, nodes, hovered)
}

/// Checks the field whose full text is `full`: in a window `narrow` wide it ends in an ellipsis
/// inside the window, screen readers get the full text, and resting the pointer on it shows the
/// full text in a tooltip no wider than the window; in a window `wide` wide it shows in full with no
/// tooltip.
fn assert_clipped_text_shows_on_hover(app: &mut crate::VectorcraftApp, ctx: &egui::Context, full: &str, narrow: f32, wide: f32, draw: DrawBar) {
    let prefix: String = full.chars().take(4).collect();
    let (texts, nodes, hovered) = bar_hover(app, ctx, narrow, &prefix, draw);
    let field = status_text(&texts, &prefix);
    assert_eq!(field.text, full);
    assert!(field.elided, "{narrow}: the text is clipped: {field:?}");
    assert!(field.bounds.right() <= narrow, "{narrow}: the ellipsis is inside the window: {field:?}");
    assert!(nodes.iter().any(|n| n.value() == Some(full)), "{narrow}: screen readers receive the full text");
    let tip: Vec<_> = hovered.iter().filter(|t| t.text == full && !t.elided).collect();
    assert_eq!(tip.len(), 1, "{narrow}: the tooltip shows the full text: {hovered:?}");
    assert!(tip[0].bounds.left() >= 0.0 && tip[0].bounds.right() <= narrow, "{narrow}: the tooltip wraps within the window: {tip:?}");

    let (texts, _, hovered) = bar_hover(app, ctx, wide, &prefix, draw);
    assert!(!status_text(&texts, &prefix).elided, "{wide}: the text fits");
    assert_eq!(hovered.iter().filter(|t| t.text == full).count(), 1, "{wide}: no tooltip: {hovered:?}");
}

/// A status message too long for the bar ends in an ellipsis and shows in full on hover, in a
/// tooltip that wraps within the window (#6622).
#[test]
fn a_clipped_status_message_shows_in_full_on_hover() {
    let (mut app, ctx) = status_readout_app();
    ctx.enable_accesskit();
    let long = format!("Saved recovery data for {}illustration.vectorcraft", "a-long-folder-name/".repeat(12));
    app.ui.status = long.clone();
    assert_clipped_text_shows_on_hover(&mut app, &ctx, &long, 700.0, 20000.0, super::status_bar);
    app.ui.status = "Saved recovery data for Untitled-1".to_owned();
    let (texts, _, hovered) = bar_hover(&mut app, &ctx, 700.0, "Saved", super::status_bar);
    assert!(!status_text(&texts, "Saved").elided);
    assert_eq!(hovered.iter().filter(|t| t.text.starts_with("Saved")).count(), 1, "a message that fits has no tooltip: {hovered:?}");
}

/// The status message of an open with several notes lists them all on hover, and screen readers
/// receive them after the message, while the status bar shows that message. A later message
/// leaves the list out, and so does an open with one note.
#[test]
fn the_message_of_an_open_with_several_notes_lists_them_on_hover() {
    let (mut app, ctx) = status_readout_app();
    ctx.enable_accesskit();
    let notes = vec!["the file's layers weren't read".to_string(), "a brush wasn't read".to_string()];
    app.ui.status = crate::io::opened_with_notes(2, &notes[0]);
    app.ui.status_notes = Some((app.ui.status.clone(), notes));
    let listed = "• the file's layers weren't read\n• a brush wasn't read";
    let full = format!("{}\n{listed}", app.ui.status);
    let (texts, nodes, hovered) = bar_hover(&mut app, &ctx, 1400.0, "Opened", super::status_bar);
    assert!(!status_text(&texts, "Opened").elided);
    assert!(nodes.iter().any(|n| n.value() == Some(full.as_str())), "screen readers receive the notes: {nodes:?}");
    assert!(hovered.iter().any(|t| t.text == full), "the tooltip lists the notes: {hovered:?}");

    app.ui.status = "Saved recovery data for Untitled-1".to_owned();
    let (_, nodes, hovered) = bar_hover(&mut app, &ctx, 1400.0, "Saved", super::status_bar);
    assert!(!nodes.iter().any(|n| n.value().is_some_and(|v| v.contains("• "))), "{nodes:?}");
    assert_eq!(hovered.iter().filter(|t| t.text.starts_with("Saved")).count(), 1, "no tooltip: {hovered:?}");

    // One note is the message itself: no list.
    app.ui.status = crate::io::opened_with_notes(1, "a brush wasn't read");
    app.ui.status_notes = Some((app.ui.status.clone(), vec!["a brush wasn't read".to_string()]));
    let (_, nodes, hovered) = bar_hover(&mut app, &ctx, 1400.0, "Opened", super::status_bar);
    assert!(!nodes.iter().any(|n| n.value().is_some_and(|v| v.contains("• "))), "{nodes:?}");
    assert_eq!(hovered.iter().filter(|t| t.text.starts_with("Opened")).count(), 1, "no tooltip: {hovered:?}");
}

/// The list has at most 30 notes, then how many more there are.
#[test]
fn the_hover_lists_thirty_notes_and_counts_the_rest() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let notes: Vec<String> = (1..=32).map(|i| format!("note {i}")).collect();
    let listed = super::status_bar::listed_notes(&notes, usize::MAX);
    let lines: Vec<&str> = listed.lines().collect();
    assert_eq!((lines.len(), lines[0], lines[29], lines[30]), (31, "• note 1", "• note 30", "… (+2)"));
    assert_eq!(super::status_bar::listed_notes(&notes[..2], usize::MAX), "• note 1\n• note 2");
    assert_eq!(super::status_bar::listed_notes(&notes[..3], 1), "• note 1\n… (+2)");
}

/// A hover with more long notes than the window can show lists the ones that fit and how many
/// more there are, inside the window. A window 300 points tall is shorter than the main window's
/// smallest size (500 points) at UI Scaling 1.5.
#[test]
fn the_hover_of_many_notes_stays_inside_a_small_window() {
    let (mut app, ctx) = status_readout_app();
    let notes: Vec<String> = (1..=32).map(|i| format!("note {i}: {}", "this file's PostScript couldn't be read ".repeat(3))).collect();
    app.ui.status = crate::io::opened_with_notes(notes.len(), &notes[0]);
    app.ui.status_notes = Some((app.ui.status.clone(), notes));
    let (_, _, hovered) = bar_hover(&mut app, &ctx, 700.0, "Opened", super::status_bar);
    let tip: Vec<_> = hovered.iter().filter(|t| t.text.contains("\n• note 1:")).collect();
    let [tip] = &tip[..] else { panic!("one tooltip lists the notes: {hovered:?}") };
    assert!(tip.bounds.top() >= 0.0 && tip.bounds.bottom() <= 300.0 && tip.bounds.right() <= 700.0, "{tip:?}");
    let last = tip.text.lines().last().unwrap_or_default();
    assert!(last.starts_with("… (+") && !tip.text.contains("• note 30:"), "{last:?}");
}

/// The Help Bar's hint ends in an ellipsis when it is wider than the window and shows in full on
/// hover, in a tooltip that wraps within the window (#6622). The Dimension tool's hint is wider
/// than the smallest main window (800 points); 400 points is that window at UI Scaling 2.0.
#[test]
fn a_clipped_help_bar_hint_shows_in_full_on_hover() {
    let (mut app, ctx) = status_readout_app();
    ctx.enable_accesskit();
    app.select_tool("dimension");
    let hint: String = super::hint_segments(app.session.tool_id()).into_iter().map(|(s, _)| s).collect();
    for narrow in [800.0, 400.0] {
        assert_clipped_text_shows_on_hover(&mut app, &ctx, &hint, narrow, 3000.0, super::hint_bar);
    }
}

/// In a narrow status bar the tool name ends in an ellipsis, shows in full on hover, and leaves
/// the status message its room (#6622). 640 and 500 points are the smallest main window (800
/// points) at UI Scaling 1.25 and 1.6.
#[test]
fn a_clipped_tool_name_shows_in_full_on_hover() {
    let (mut app, ctx) = status_readout_app();
    ctx.enable_accesskit();
    for narrow in [640.0, 500.0] {
        assert_clipped_text_shows_on_hover(&mut app, &ctx, "Selection", narrow, 1400.0, super::status_bar);
        let (texts, _) = bar_frame(&mut app, &ctx, narrow, None, vec![], super::status_bar);
        let tool = status_text(&texts, "Selection");
        let message = status_text(&texts, "Saved recovery data");
        assert!(message.clip.left() >= tool.bounds.right() && message.clip.width() >= 24.0, "{narrow}: the message keeps room: {texts:?}");
    }
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

/// A middle-click on a document tab closes it as the × does (#6506): a saved document closes, and
/// a modified one asks to save first instead of losing its changes.
#[test]
fn a_middle_click_on_a_tab_closes_it_as_the_close_box_does() {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    for _ in 0..3 {
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    }
    // The second document has unsaved changes; the third stays the current one.
    app.session.set_active(1);
    app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    app.session.set_active(2);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 200.0));
    let frame = |app: &mut crate::VectorcraftApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| super::doc_tabs(app, ui));
        out.textures_delta.clear();
        let mut titles = vec![];
        for c in &out.shapes {
            if let egui::Shape::Text(t) = &c.shape
                && t.visual_bounding_rect().width() > 30.0
            {
                titles.push(t.visual_bounding_rect());
            }
        }
        titles.sort_by(|a, b| a.left().total_cmp(&b.left()));
        titles
    };
    let middle_click = |app: &mut crate::VectorcraftApp, at: egui::Pos2| {
        let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Middle, pressed, modifiers: Default::default() };
        frame(app, vec![egui::Event::PointerMoved(at), press(true)]);
        frame(app, vec![press(false)])
    };
    let titles = frame(&mut app, vec![]);
    assert_eq!(titles.len(), 3, "{titles:?}");
    // The modified document's tab: the save prompt comes up and the document stays open.
    let titles = middle_click(&mut app, titles[1].center());
    assert_eq!(app.session.documents().len(), 3, "a modified document is not closed without asking");
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(crate::unsaved::KIND), "a modified document asks first");
    app.ui.dialog = None;
    // The first, saved document's tab: it closes, and the others stay open.
    let uid = |app: &crate::VectorcraftApp, i: usize| app.session.documents()[i].uid;
    let (second, third) = (uid(&app, 1), uid(&app, 2));
    middle_click(&mut app, titles[0].center());
    assert_eq!(app.session.documents().len(), 2, "the middle-click closed the first document");
    assert_eq!((uid(&app, 0), uid(&app, 1)), (second, third));
    assert!(app.ui.dialog.is_none(), "a saved document closes without asking");
    // A middle-click on the × itself closes too: the third, saved document's.
    let titles = frame(&mut app, vec![]);
    assert_eq!(titles.len(), 2, "{titles:?}");
    let title = titles[1];
    let x = if super::CLOSE_BEFORE_TITLE { title.left() - 16.0 } else { title.right() + 20.0 };
    middle_click(&mut app, egui::pos2(x, title.center().y));
    assert_eq!(app.session.documents().len(), 1, "the middle-click on the × closed the third document");
    assert_eq!(uid(&app, 0), second);
    assert!(app.ui.dialog.is_none(), "a saved document closes without asking");
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

/// #3797: each document's tab names its own Outline or Preview.
#[test]
fn only_the_outlined_documents_tab_reads_outline() {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    app.run("view.outline", json!({})).unwrap();
    let titles: Vec<String> = (0..app.session.documents().len()).filter_map(|i| super::document_window_title(&app, (i, 1))).collect();
    assert!(titles[0].ends_with("(RGB/Preview)"), "{titles:?}");
    assert!(titles[1].ends_with("(RGB/Outline)"), "{titles:?}");
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

/// Tab goes to the isolation bar's back button and breadcrumbs and does not stop on the bar
/// behind them, which only keeps the pointer off the canvas.
#[test]
fn tab_skips_the_isolation_bars_background() {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    let a = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap()["id"].as_u64().unwrap();
    let b = app.run("shape.rectangle", json!({"x": 50, "y": 10, "width": 30, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    app.run("object.group", json!({})).unwrap();
    app.run("object.isolate", json!({"id": a})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 700.0));
    let mut frame = |events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| {
            super::isolation_bar(&mut app, ui, screen);
        });
        out.textures_delta.clear();
    };
    frame(vec![]);
    let bar = egui::Rect::from_min_size(screen.min, egui::vec2(screen.width(), 24.0));
    let back = egui::Rect::from_min_size(bar.min + egui::vec2(6.0, 3.0), egui::vec2(18.0, 18.0));
    let widget =
        |rect: egui::Rect| ctx.viewport(|vp| vp.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).find(|w| w.rect == rect).map(|w| w.id));
    let (bar, back) = (widget(bar).expect("the isolation bar"), widget(back).expect("the back button"));
    let stops = crate::widgets::accessibility_tests::tab_stops(&ctx, 10, frame);
    assert!(stops.contains(&back), "Tab reaches the back button: {stops:?}");
    assert!(!stops.contains(&bar), "{stops:?}");
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

/// In symbol editing mode the Control bar's isolation button is Exit Symbol Editing Mode and
/// leaves the mode. With a group isolated inside the symbol's art it is Exit Isolation Mode and
/// goes back to the symbol's art.
#[test]
fn the_control_bar_exits_symbol_editing_mode() {
    use vectorcraft_doc::NodeId;
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
    app.run("symbol.new", json!({"name": "Box"})).unwrap();
    app.run("symbol.edit", json!({"name": "Box"})).unwrap();
    let layer = app.session.active().unwrap().doc.symbol_edit.as_ref().expect("symbol editing mode").layer;
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    ctx.enable_accesskit();
    // The isolation button's label (the one exit label the bar shows), and a click on it.
    let label = |app: &mut crate::VectorcraftApp| {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 300.0));
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| super::control_bar(app, ui));
        out.textures_delta.clear();
        let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
        let exits: Vec<String> = nodes
            .iter()
            .filter_map(|(_, n)| n.label().map(|l| l.trim().to_string()))
            .filter(|l| l == "Exit Symbol Editing Mode" || l == "Exit Isolation Mode")
            .collect();
        assert!(exits.len() <= 1, "{exits:?}");
        exits.into_iter().next()
    };
    let click = |app: &mut crate::VectorcraftApp| {
        let r = isolation_button(app, &ctx, vec![]).expect("the bar shows the isolation button");
        let press =
            |pressed| egui::Event::PointerButton { pos: r.center(), button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        for e in [egui::Event::PointerMoved(r.center()), press(true), press(false)] {
            isolation_button(app, &ctx, vec![e]);
        }
    };
    assert_eq!(label(&mut app).as_deref(), Some("Exit Symbol Editing Mode"));
    // Two rectangles drawn into the symbol's art, grouped and isolated.
    let a = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    let b = app.run("shape.rectangle", json!({"x": 130, "y": 100, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let g = app.run("object.group", json!({})).unwrap()["id"].as_u64().unwrap();
    app.run("object.isolate", json!({"id": g})).unwrap();
    assert_eq!(label(&mut app).as_deref(), Some("Exit Isolation Mode"));
    click(&mut app);
    let st = app.session.active().unwrap();
    assert_eq!((st.isolation, st.doc.symbol_edit.is_some()), (Some(layer), true), "back to the symbol's art");
    assert_eq!(label(&mut app).as_deref(), Some("Exit Symbol Editing Mode"));
    click(&mut app);
    let st = app.session.active().unwrap();
    assert_eq!((st.isolation, st.doc.symbol_edit.is_some()), (None::<NodeId>, false), "symbol editing mode ends");
    // The symbol keeps the art drawn while editing.
    let symbol = st.doc.symbols.iter().find(|s| s.name == "Box").expect("the symbol");
    assert!(symbol.art.children().is_some_and(|c| c.len() >= 2), "the edit reached the symbol");
}

/// #4819: with Direct Selection, a selected blend shows the Control bar's and the Properties
/// panel's Convert buttons, as a path selected whole does; with the Selection tool it doesn't.
#[test]
fn a_blend_selected_with_direct_selection_shows_the_convert_buttons() {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    let rect = |app: &mut crate::VectorcraftApp, x: f64| {
        app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 30, "height": 20})).unwrap()["id"].as_u64().unwrap()
    };
    let (a, b) = (rect(&mut app, 10.0), rect(&mut app, 200.0));
    let blend = app.run("object.blend.make", json!({"ids": [a, b], "steps": 1})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [blend]})).unwrap();
    assert!(super::anchor_controls(&app) == super::AnchorControls::None, "the Selection tool");
    app.select_tool("directSelection");
    assert!(super::anchor_controls(&app) == super::AnchorControls::Convert);
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
    app.desktop_frame = !custom_titlebar;
    // The preference that makes the shell launch without its own title bar.
    app.run("prefs.set", json!({"key": "systemTitleBar", "value": !custom_titlebar})).unwrap();
    app
}

/// How many brand marks one frame of the app bar paints.
fn brand_marks(app: &mut crate::VectorcraftApp) -> usize {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::app_bar(app, ui));
    out.textures_delta.clear();
    out.shapes.iter().filter(|c| matches!(&c.shape, egui::Shape::Mesh(m) if m.texture_id == crate::brand::texture_id(&ctx))).count()
}

#[test]
fn without_the_preference_the_mark_stays() {
    // The default (e.g. the web build): no custom bar, no preference.
    let app = crate::VectorcraftApp::new(Session::new(), Default::default());
    assert!(!super::system_title_bar(&app));
    // An embedder without a window of its own keeps the mark with the preference on too.
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("prefs.set", json!({"key": "systemTitleBar", "value": true})).unwrap();
    assert!(!super::system_title_bar(&app));
    assert_eq!(brand_marks(&mut app), 1);
}

#[test]
fn a_desktop_frame_hides_the_mark_whatever_the_preference_says() {
    // Linux on a desktop that draws the frame (KDE, X11): the window has the desktop's title bar
    // with System Title Bar off.
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.desktop_frame = true;
    app.run("prefs.set", json!({"key": "systemTitleBar", "value": false})).unwrap();
    let hidden = !cfg!(target_os = "macos");
    assert_eq!(super::system_title_bar(&app), hidden);
    assert_eq!(brand_marks(&mut app), if hidden { 0 } else { 1 });
    // The preference applies at the next start: changing it now leaves the window's frame, and the mark, as they are.
    let mut app = title_test_app(false);
    app.run("prefs.set", json!({"key": "systemTitleBar", "value": false})).unwrap();
    assert_eq!(super::system_title_bar(&app), hidden);
    let mut app = title_test_app(true);
    app.run("prefs.set", json!({"key": "systemTitleBar", "value": true})).unwrap();
    assert!(!super::system_title_bar(&app));
    assert_eq!(brand_marks(&mut app), 1);
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
    assert_eq!(st.doc.ruler_guides(), vec![before.ruler_guides()[0].clone()], "only the deleted board's guide is removed");
    assert_eq!(art_of(&st.doc.layers), art_of(&before.layers), "artwork and symbol instances stay unchanged");
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

/// Press F6 with `modifiers` through the shortcut handler.
fn press_f6(app: &mut crate::VectorcraftApp, modifiers: egui::Modifiers) {
    let ctx = egui::Context::default();
    let ev = egui::Event::Key { key: egui::Key::F6, physical_key: None, pressed: true, repeat: false, modifiers };
    let mut out = ctx.run_ui(egui::RawInput { events: vec![ev], ..Default::default() }, |ui| crate::shortcuts::handle(app, ui.ctx()));
    out.textures_delta.clear();
}

/// Cmd+F6 is Next Document and Cmd+Shift+F6 is Previous Document by default: Ctrl+F6 and
/// Ctrl+Shift+F6 on Windows and Linux, Command+F6 and Shift+Command+F6 on macOS. Keyboard
/// Shortcuts lists both under Window. F6 and Shift+F6 are still the Color and Appearance panels'
/// keys, and no other command, tool or panel has either key by default.
#[test]
fn cmd_f6_and_cmd_shift_f6_are_next_and_previous_document() {
    use crate::shortcut_editor::{command_for_key, default_command_shortcut, panel_shortcut};
    assert_eq!(default_command_shortcut("window.nextDocument"), Some("Cmd+F6"));
    assert_eq!(default_command_shortcut("window.previousDocument"), Some("Cmd+Shift+F6"));
    assert_eq!(command_for_key("Cmd+F6"), Some("window.nextDocument"));
    assert_eq!(command_for_key("Cmd+Shift+F6"), Some("window.previousDocument"));
    assert_eq!((panel_shortcut("color"), panel_shortcut("appearance")), (Some("F6"), Some("Shift+F6")));
    let chord = crate::shortcuts::parse("Cmd+Shift+F6").expect("parses");
    assert_eq!((chord.modifiers, chord.logical_key), (egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::F6));
    let shown = (crate::menus::pretty_shortcut("Cmd+F6"), crate::menus::pretty_shortcut("Cmd+Shift+F6"));
    let want = if cfg!(target_os = "macos") { ("⌘F6", "⇧⌘F6") } else { ("Ctrl+F6", "Ctrl+Shift+F6") };
    assert_eq!((shown.0.as_str(), shown.1.as_str()), want);
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    let listed = app.run("shortcuts.list", json!({"query": "Document"})).unwrap();
    let rows: Vec<_> = listed
        .as_array()
        .expect("rows")
        .iter()
        .filter(|r| r["id"].as_str().is_some_and(|id| id.ends_with("Document") && id.starts_with("window.")))
        .map(|r| (r["label"].clone(), r["group"].clone(), r["default"].clone()))
        .collect();
    assert_eq!(
        rows,
        [(json!("Next Document"), json!("Window"), json!("Cmd+F6")), (json!("Previous Document"), json!("Window"), json!("Cmd+Shift+F6"))],
        "Keyboard Shortcuts lists both"
    );
    assert_eq!(app.run("shortcuts.conflicts", json!({})).unwrap(), json!([]), "no other default has the keys");
}

/// Cmd+F6 makes the next open document in tab order the current one, and Cmd+Shift+F6 makes the
/// previous one the current one. After the last document comes the first, and before the first document comes the
/// last. From the Home screen the keys go to the first and the last document and close the screen.
/// With no document open the commands are disabled and fail with "no document open".
#[test]
fn cmd_f6_goes_to_the_next_document_and_cmd_shift_f6_to_the_previous_one() {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    assert!(!crate::menus::enabled(&app, "window.nextDocument") && !crate::menus::enabled(&app, "window.previousDocument"));
    assert_eq!(app.run("window.nextDocument", json!({})).unwrap_err(), "no document open");
    assert_eq!(app.run("window.previousDocument", json!({})).unwrap_err(), "no document open");
    for name in ["A", "B", "C"] {
        app.run("file.new", json!({"width": 100, "height": 100, "name": name})).unwrap();
    }
    assert!(crate::menus::enabled(&app, "window.nextDocument") && crate::menus::enabled(&app, "window.previousDocument"));
    assert_eq!(app.session.active_index(), Some(2));
    let mut seen = vec![];
    for modifiers in [egui::Modifiers::COMMAND; 4] {
        press_f6(&mut app, modifiers);
        seen.push(app.session.active_index());
    }
    assert_eq!(seen, [Some(0), Some(1), Some(2), Some(0)], "Cmd+F6 goes right and wraps from the last to the first");
    seen.clear();
    for modifiers in [egui::Modifiers::COMMAND | egui::Modifiers::SHIFT; 4] {
        press_f6(&mut app, modifiers);
        seen.push(app.session.active_index());
    }
    assert_eq!(seen, [Some(2), Some(1), Some(0), Some(2)], "Cmd+Shift+F6 goes left and wraps from the first to the last");
    assert_eq!(app.run("window.nextDocument", json!({})).unwrap(), json!({"index": 0}));
    assert_eq!(app.run("window.previousDocument", json!({})).unwrap(), json!({"index": 2}));
    app.session.set_active(1);
    app.run("app.home", json!({})).unwrap();
    press_f6(&mut app, egui::Modifiers::COMMAND);
    assert_eq!((app.session.active_index(), app.ui.home), (Some(0), None), "from the Home screen Cmd+F6 shows the first");
    app.session.set_active(1);
    app.run("app.home", json!({})).unwrap();
    press_f6(&mut app, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
    assert_eq!((app.session.active_index(), app.ui.home), (Some(2), None), "and Cmd+Shift+F6 the last");
    // One document: the keys keep it current.
    let mut one = crate::VectorcraftApp::new(Session::new(), Default::default());
    one.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    press_f6(&mut one, egui::Modifiers::COMMAND);
    press_f6(&mut one, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
    assert_eq!(one.session.active_index(), Some(0));
}

/// With the Artboard tool, Cmd+F6 and Cmd+Shift+F6 switch documents as a click on a tab does: the
/// document switched back to keeps the artboard it shows as the tool's target, so the Properties
/// panel's Delete Artboard then removes that artboard. Switching is not an edit.
#[test]
fn cmd_f6_switches_documents_as_a_tab_click_does_with_the_artboard_tool() {
    let (mut app, ctx) = artboard_document_app();
    app.select_tool("artboard");
    artboard_navigation_next(&mut app, &ctx);
    assert_eq!(app.session.tool_options()["selected"], json!([1]));
    let before = app.session.active().unwrap().doc.clone();
    let undo = app.session.active().unwrap().history.undo.len();
    press_f6(&mut app, egui::Modifiers::COMMAND);
    assert_eq!(app.session.active_index(), Some(1));
    // The app draws frames while the other document is the current one.
    artboard_navigation_properties_frame(&mut app, &ctx, vec![]);
    artboard_document_tabs_frame(&mut app, &ctx, vec![]);
    press_f6(&mut app, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
    assert_eq!(app.session.active_index(), Some(0));
    app.sync_views();
    assert_eq!(app.session.tool_options()["selected"], json!([1]), "the tool targets the displayed artboard");
    assert_eq!(app.view().unwrap().artboard, 1, "the document keeps its displayed artboard");
    assert_eq!(app.session.active().unwrap().doc, before, "switching documents is not a document edit");
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
    artboard_navigation_properties_frame(&mut app, &ctx, vec![]);
    let texts = artboard_navigation_properties_frame(&mut app, &ctx, vec![]);
    let at = texts.iter().find(|(label, _)| label == "Delete Artboard").map(|(_, r)| r.center()).expect("Properties Delete button");
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    for event in [egui::Event::PointerMoved(at), button(true), button(false)] {
        artboard_navigation_properties_frame(&mut app, &ctx, vec![event]);
    }
    let names: Vec<_> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.name.clone()).collect();
    assert_eq!(names, ["Keep"], "Delete Artboard after the keys removes the artboard the document shows");
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
    assert_eq!(art_of(&st.doc.layers), art_of(&before.layers), "all artwork and symbol instances stay unchanged");
    assert_eq!(st.doc.symbols, before.symbols);
    assert_eq!(st.doc.ruler_guides(), vec![before.ruler_guides()[0].clone()]);
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
    assert_eq!(before.ruler_guides().len(), 2);
    assert!(!selection.objects.is_empty());
    assert!(history_before.1.is_empty());
    assert!(!st.is_dirty());
    let mut expected = before.as_ref().clone();
    expected.artboards = vec![before.artboards[0].clone()];
    let keep = before.ruler_guides()[0].id;
    expected.retain_guides(&mut vectorcraft_doc::Selection::default(), |g| g.id == keep);
    let base = Baseline { uid, revision, before, selection, history_before, history_limits, expected };
    close_b_tab(&mut app, &ctx, &base);
    delete_shown_board(&mut app, &ctx, &base);
}

/// #4365: Full Screen and Presentation Mode put the window full screen once, and Normal Screen
/// Mode takes it back out; a window already full screen is left as it is.
#[test]
fn full_screen_modes_fill_the_screen_and_normal_mode_restores_the_window() {
    let mut app = title_test_app(true);
    let ctx = egui::Context::default();
    let sent = |app: &mut crate::VectorcraftApp, fullscreen: bool| -> Vec<bool> {
        let mut input = egui::RawInput::default();
        input.viewports.entry(egui::ViewportId::ROOT).or_default().fullscreen = Some(fullscreen);
        let mut out = ctx.run_ui(input, |ui| super::sync_fullscreen(app, ui.ctx()));
        out.textures_delta.clear();
        let cmds = out.viewport_output.remove(&egui::ViewportId::ROOT).map(|v| v.commands).unwrap_or_default();
        cmds.iter().filter_map(|c| if let egui::ViewportCommand::Fullscreen(f) = c { Some(*f) } else { None }).collect()
    };
    assert!(sent(&mut app, false).is_empty(), "Normal Screen Mode");
    app.ui.screen_mode = 3;
    assert_eq!(sent(&mut app, false), [true], "Presentation Mode");
    assert!(sent(&mut app, true).is_empty(), "once");
    app.ui.screen_mode = 2;
    assert!(sent(&mut app, true).is_empty(), "Full Screen Mode keeps it");
    app.ui.screen_mode = 0;
    assert_eq!(sent(&mut app, true), [false], "back to a window");
    assert!(sent(&mut app, false).is_empty());
    // Full screen by the user's own choice: the modes leave it so.
    app.ui.screen_mode = 3;
    assert!(sent(&mut app, true).is_empty());
    app.ui.screen_mode = 0;
    assert!(sent(&mut app, true).is_empty(), "still full screen, as the user had it");
}

/// Snapping quick access includes the existing glyph and alignment settings. Pointer clicks use
/// their commands, and reopening reads the same state as View and Preferences (#6164).
#[test]
fn the_control_bar_snapping_menu_shares_glyph_and_alignment_settings() {
    struct RestoreLanguage(crate::i18n::Lang);
    impl Drop for RestoreLanguage {
        fn drop(&mut self) {
            crate::i18n::set_current(self.0);
        }
    }
    let _language = RestoreLanguage(crate::i18n::current());
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.ui.view.snap_to_glyph = false;
    app.run("prefs.set", json!({"key": "alignmentGuides", "value": true})).unwrap();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 400.0));
    let frame = |app: &mut crate::VectorcraftApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: Some(screen), ..Default::default() }, |ui| super::control_bar(app, ui));
        out.textures_delta.clear();
        out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default()
    };
    type Nodes = Vec<(egui::accesskit::NodeId, egui::accesskit::Node)>;
    let find = |nodes: &Nodes, name: &str| {
        nodes.iter().find(|(_, n)| n.label() == Some(name)).map(|(_, n)| n.clone()).unwrap_or_else(|| panic!("missing {name}"))
    };
    let center = |n: &egui::accesskit::Node| {
        let b = n.bounds().expect("a visible menu control");
        assert!(b.x0 >= 0.0 && b.y0 >= 0.0 && b.x1 <= 1200.0 && b.y1 <= 400.0, "menu control stays inside the window: {b:?}");
        egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)
    };
    let click = |app: &mut crate::VectorcraftApp, at: egui::Pos2| {
        let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(app, vec![egui::Event::PointerMoved(at), press(true)]);
        frame(app, vec![press(false)]);
        frame(app, vec![]);
        frame(app, vec![])
    };
    let check = |nodes: &Nodes, name: &str, on: bool| {
        let node = find(nodes, name);
        assert_eq!(node.role(), egui::accesskit::Role::CheckBox, "{name}");
        assert_eq!(node.toggled(), Some(if on { egui::accesskit::Toggled::True } else { egui::accesskit::Toggled::False }), "{name}");
        center(&node)
    };
    let dismiss = |app: &mut crate::VectorcraftApp| {
        let before = (app.ui.view.snap_to_glyph, app.session.prefs.to_json(), app.run_count);
        frame(
            app,
            vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }],
        );
        frame(
            app,
            vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: false, repeat: false, modifiers: Default::default() }],
        );
        let nodes = frame(app, vec![]);
        assert!(!nodes.iter().any(|(_, n)| n.label() == Some("Snap to Glyph")), "Escape closes Snapping");
        assert_eq!(
            (app.ui.view.snap_to_glyph, app.session.prefs.to_json(), app.run_count),
            before,
            "dismissal changes no settings and runs no command"
        );
    };
    frame(&mut app, vec![]);
    let nodes = frame(&mut app, vec![]);
    let menu = center(&find(&nodes, "Snapping"));
    let nodes = click(&mut app, menu);
    let glyph = check(&nodes, "Snap to Glyph", false);
    check(&nodes, "Alignment Guides", true);
    // The two extra rows still leave the original menu entries visible.
    for name in ["Smart Guides", "Snap to Point", "Snap to Grid", "Snap to Pixel", "Snap to Tangent", "Snap to Perpendicular"] {
        center(&find(&nodes, name));
    }
    click(&mut app, glyph);
    assert!(app.ui.view.snap_to_glyph);
    assert_eq!(crate::menus::checked(&app, "view.snapToGlyph", &json!({})), Some(true));
    assert!(app.session.prefs.alignment_guides, "glyph snapping leaves Alignment Guides alone");
    dismiss(&mut app);
    let nodes = click(&mut app, menu);
    check(&nodes, "Snap to Glyph", true);
    let alignment = check(&nodes, "Alignment Guides", true);
    let commands = app.run_count;
    click(&mut app, alignment);
    assert_eq!(app.run_count, commands + 1, "Alignment Guides uses the app command entry point");
    assert_eq!(app.run("prefs.get", json!({"key": "alignmentGuides"})).unwrap(), json!(false));
    assert!(!app.session.prefs.alignment_guides);
    crate::prefs_dialog::snapshot(&mut app);
    let saved = serde_json::to_string(&app.ui.engine_prefs).unwrap();
    let mut restored = crate::VectorcraftApp::new(Session::new(), Default::default());
    restored.ui.engine_prefs = serde_json::from_str(&saved).unwrap();
    crate::prefs_dialog::restore(&mut restored);
    assert_eq!(
        restored.run("prefs.get", json!({"key": "alignmentGuides"})).unwrap(),
        json!(false),
        "Alignment Guides survives host preference persistence"
    );
    assert!(app.ui.view.snap_to_glyph, "Alignment Guides leaves glyph snapping alone");
    dismiss(&mut app);
    let nodes = click(&mut app, menu);
    check(&nodes, "Snap to Glyph", true);
    check(&nodes, "Alignment Guides", false);
    // Changes made through the existing command surfaces are reflected on the next opening too.
    crate::menus::invoke(&mut app, "view.snapToGlyph", json!({}));
    app.run("prefs.set", json!({"key": "alignmentGuides", "value": true})).unwrap();
    dismiss(&mut app);
    let nodes = click(&mut app, menu);
    check(&nodes, "Snap to Glyph", false);
    check(&nodes, "Alignment Guides", true);
}

/// The window the Control bar docking tests draw the bar in.
const DOCK_SCREEN: egui::Rect = egui::Rect { min: egui::Pos2::ZERO, max: egui::pos2(1200.0, 300.0) };

/// The window the Control bar panel menu tests draw the bar in, tall enough for the whole menu.
const MENU_SCREEN: egui::Rect = egui::Rect { min: egui::Pos2::ZERO, max: egui::pos2(1200.0, 700.0) };

/// One headless frame of the Control bar alone; returns its accessibility nodes.
fn control_bar_frame(app: &mut crate::VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<egui::accesskit::Node> {
    control_bar_frame_in(app, ctx, DOCK_SCREEN, events)
}

/// [`control_bar_frame`] in window `screen`.
fn control_bar_frame_in(
    app: &mut crate::VectorcraftApp,
    ctx: &egui::Context,
    screen: egui::Rect,
    events: Vec<egui::Event>,
) -> Vec<egui::accesskit::Node> {
    let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: Some(screen), ..Default::default() }, |ui| super::control_bar(app, ui));
    out.textures_delta.clear();
    out.platform_output.accesskit_update.map(|u| u.nodes.into_iter().map(|(_, n)| n).collect()).unwrap_or_default()
}

/// The center of the bounds of the node whose label, without a check mark, is `name`.
fn node_center(nodes: &[egui::accesskit::Node], name: &str) -> Option<egui::Pos2> {
    let n = nodes.iter().find(|n| n.label().is_some_and(|l| l.trim_start_matches('✓').trim() == name))?;
    let b = n.bounds()?;
    Some(egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32))
}

/// A primary-button press or release at `pos`.
fn press_at(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
}

/// The Control bar's panel menu has Dock to Top and Dock to Bottom, the edge in use checked, and
/// each docks the bar to that edge of the window.
#[test]
fn the_control_bar_panel_menu_docks_it_to_the_top_or_the_bottom() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    let frame = |app: &mut crate::VectorcraftApp, events| control_bar_frame_in(app, &ctx, MENU_SCREEN, events);
    let pick = |app: &mut crate::VectorcraftApp, item: &str| {
        frame(app, vec![]);
        let menu = node_center(&frame(app, vec![]), "Panel menu").unwrap_or_else(|| panic!("the bar has a panel menu"));
        frame(app, vec![egui::Event::PointerMoved(menu), press_at(menu, true)]);
        frame(app, vec![press_at(menu, false)]);
        let nodes = frame(app, vec![]);
        // The groups of options are listed first, every one on; the edge items follow.
        let checked: Vec<String> =
            nodes.iter().filter_map(|n| n.label()).filter(|l| l.starts_with('✓') && l.contains("Dock")).map(str::to_string).collect();
        let groups = nodes.iter().filter_map(|n| n.label()).filter(|l| CONTROL_BAR_GROUP_LABELS.iter().any(|g| *l == format!("✓ {g}"))).count();
        assert_eq!(groups, CONTROL_BAR_GROUP_LABELS.len(), "every group is listed, on");
        let at = node_center(&nodes, item).unwrap_or_else(|| panic!("the menu lists {item}"));
        // Below the button at the top, and above it at the bottom: the menu opens inside the window.
        assert!(MENU_SCREEN.contains(at) && (at.y < menu.y) == app.ui.control_bar_bottom, "{item} at {at:?}, the button at {menu:?}");
        frame(app, vec![egui::Event::PointerMoved(at), press_at(at, true)]);
        frame(app, vec![press_at(at, false)]);
        frame(app, vec![]);
        let menu = node_center(&frame(app, vec![]), "Panel menu").unwrap_or_else(|| panic!("the bar still shows"));
        (checked, menu.y)
    };
    let (checked, y) = pick(&mut app, "Dock to Bottom");
    assert_eq!(checked, ["✓ Dock to Top"], "the bar starts docked at the top");
    assert!(app.ui.control_bar_bottom, "Dock to Bottom docks it at the bottom");
    assert!(y > MENU_SCREEN.bottom() - 34.0, "the bar is drawn along the bottom of the window: {y}");
    let (checked, y) = pick(&mut app, "Dock to Top");
    assert_eq!(checked, ["✓ Dock to Bottom"]);
    assert!(!app.ui.control_bar_bottom && y < 34.0, "Dock to Top docks it at the top again: {y}");
}

/// In a window too short for the whole Control bar panel menu, the menu stays inside the window and
/// scrolls, so Dock to Bottom, its last item, can still be reached and clicked.
#[test]
fn the_control_bar_panel_menu_scrolls_in_a_short_window() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    control_bar_frame(&mut app, &ctx, vec![]);
    let menu = node_center(&control_bar_frame(&mut app, &ctx, vec![]), "Panel menu").unwrap_or_else(|| panic!("the bar has a panel menu"));
    control_bar_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(menu), press_at(menu, true)]);
    control_bar_frame(&mut app, &ctx, vec![press_at(menu, false)]);
    let nodes = control_bar_frame(&mut app, &ctx, vec![]);
    let first = node_center(&nodes, "Snapping").filter(|p| p.y > 34.0).unwrap_or_else(|| panic!("the menu lists Snapping"));
    let wheel = egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -200.0),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
        source: egui::MouseWheelSource::Unknown,
    };
    for _ in 0..6 {
        control_bar_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(first), wheel.clone()]);
    }
    let nodes = control_bar_frame(&mut app, &ctx, vec![]);
    let at = node_center(&nodes, "Dock to Bottom").unwrap_or_else(|| panic!("the menu lists Dock to Bottom"));
    assert!(DOCK_SCREEN.shrink(1.0).contains(at), "scrolled into the window: {at:?}");
    control_bar_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), press_at(at, true)]);
    control_bar_frame(&mut app, &ctx, vec![press_at(at, false)]);
    assert!(app.ui.control_bar_bottom, "Dock to Bottom docks the bar at the bottom");
}

/// The gripper of a Control bar docked at the top or the bottom of [`DOCK_SCREEN`]: the bar's first
/// item, inside its 10-point margin, centered in the 34-point bar.
fn docked_grip(bottom: bool) -> egui::Pos2 {
    egui::pos2(13.0, if bottom { DOCK_SCREEN.bottom() - 17.0 } else { 17.0 })
}

/// The gripper of the Control bar floating with its top-left corner at `pos`: inside its 1-point
/// border and 10-point margin.
fn floating_grip(pos: [f32; 2]) -> egui::Pos2 {
    egui::pos2(pos[0] + 14.0, pos[1] + 17.0)
}

/// A drag of the Control bar's gripper from `from` to `to` in window `screen`, with `during` sent
/// at the last move before the release, released at `to`.
fn drag_gripper_in(
    app: &mut crate::VectorcraftApp,
    ctx: &egui::Context,
    screen: egui::Rect,
    from: egui::Pos2,
    to: egui::Pos2,
    during: Vec<egui::Event>,
) {
    // A bar that has just started floating takes a frame to show.
    control_bar_frame_in(app, ctx, screen, vec![]);
    control_bar_frame_in(app, ctx, screen, vec![]);
    control_bar_frame_in(app, ctx, screen, vec![egui::Event::PointerMoved(from), press_at(from, true)]);
    for k in 1..=4 {
        control_bar_frame_in(app, ctx, screen, vec![egui::Event::PointerMoved(from.lerp(to, k as f32 / 4.0))]);
    }
    control_bar_frame_in(app, ctx, screen, during);
    control_bar_frame_in(app, ctx, screen, vec![press_at(to, false)]);
    control_bar_frame_in(app, ctx, screen, vec![]);
}

/// [`drag_gripper_in`] in [`DOCK_SCREEN`] with nothing sent during the drag.
fn drag_gripper(app: &mut crate::VectorcraftApp, ctx: &egui::Context, from: egui::Pos2, to: egui::Pos2) {
    drag_gripper_in(app, ctx, DOCK_SCREEN, from, to, vec![]);
}

/// Dragging the Control bar's gripper and releasing it within 24 points of the top or the bottom of
/// the window docks the bar at that edge, the edge it is at included.
#[test]
fn dragging_the_control_bar_gripper_docks_it_at_the_nearer_edge() {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    drag_gripper(&mut app, &ctx, docked_grip(false), egui::pos2(400.0, 290.0));
    assert!(app.ui.control_bar_bottom && !app.ui.control_bar_floating, "released at the bottom: docked at the bottom");
    drag_gripper(&mut app, &ctx, docked_grip(true), egui::pos2(400.0, 280.0));
    assert!(app.ui.control_bar_bottom && !app.ui.control_bar_floating, "released at the bottom again: still at the bottom");
    drag_gripper(&mut app, &ctx, docked_grip(true), egui::pos2(400.0, 10.0));
    assert!(!app.ui.control_bar_bottom && !app.ui.control_bar_floating, "released at the top: docked at the top");
    drag_gripper(&mut app, &ctx, docked_grip(false), egui::pos2(400.0, 20.0));
    assert!(!app.ui.control_bar_bottom && !app.ui.control_bar_floating, "released at the top again: still at the top");
}

/// Dragging the docked Control bar's gripper away from both edges and releasing it there floats
/// the bar under the pointer, held where the gripper was grabbed: 800 points wide, over the window,
/// with its gripper and panel menu. The bar stays docked until the release.
#[test]
fn dragging_the_control_bar_gripper_away_from_both_edges_floats_it() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    let from = docked_grip(false);
    control_bar_frame(&mut app, &ctx, vec![]);
    control_bar_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from), press_at(from, true)]);
    let to = egui::pos2(400.0, 150.0);
    for k in 1..=4 {
        control_bar_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from.lerp(to, k as f32 / 4.0))]);
    }
    assert!(!app.ui.control_bar_floating && super::control_bar_moving(&ctx), "still docked while the drag goes on");
    control_bar_frame(&mut app, &ctx, vec![press_at(to, false)]);
    assert!(app.ui.control_bar_floating && !super::control_bar_moving(&ctx), "released away from both edges: floating");
    assert_eq!(crate::menus::checked(&app, "window.control.dock", &json!({"edge": "float"})), Some(true));
    assert_eq!(app.ui.control_bar_float_pos, Some([to.x - from.x, to.y - from.y]), "held where the gripper was grabbed");
    control_bar_frame(&mut app, &ctx, vec![]);
    let menu = node_center(&control_bar_frame(&mut app, &ctx, vec![]), "Panel menu").unwrap_or_else(|| panic!("the floating bar has a panel menu"));
    let left = to.x - from.x;
    assert!((menu.y - to.y).abs() < 1.0 && (menu.x - (left + 800.0 - 19.0)).abs() < 1.0, "the panel menu ends the 800-point bar: {menu:?}");
}

/// A floating Control bar's gripper moves it, and docks it again at the top or the bottom of the
/// window; from either edge it floats again.
#[test]
fn floating_control_bar_moves_and_redocks_from_each_edge() {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    app.run("window.control.dock", json!({"edge": "float"})).unwrap();
    let at = super::CONTROL_BAR_FLOAT_AT;
    drag_gripper(&mut app, &ctx, floating_grip(at), egui::pos2(300.0, 100.0));
    assert!(app.ui.control_bar_floating, "moved, it still floats");
    let moved = [300.0 - 14.0, 100.0 - 17.0];
    assert_eq!(app.ui.control_bar_float_pos, Some(moved));
    drag_gripper(&mut app, &ctx, floating_grip(moved), egui::pos2(500.0, 290.0));
    assert!(!app.ui.control_bar_floating && app.ui.control_bar_bottom, "released at the bottom: docked there");
    drag_gripper(&mut app, &ctx, docked_grip(true), egui::pos2(500.0, 160.0));
    assert!(app.ui.control_bar_floating, "from the bottom it floats again");
    let [x, y] = app.ui.control_bar_float_pos.unwrap_or_default();
    drag_gripper(&mut app, &ctx, floating_grip([x, y]), egui::pos2(500.0, 8.0));
    assert!(!app.ui.control_bar_floating && !app.ui.control_bar_bottom, "released at the top: docked there");
    drag_gripper(&mut app, &ctx, docked_grip(false), egui::pos2(700.0, 120.0));
    assert!(app.ui.control_bar_floating, "from the top it floats again");
}

/// A floating Control bar's panel menu keeps Dock to Top and Dock to Bottom, neither checked, and
/// each docks the bar at its edge.
#[test]
fn floating_control_bar_menu_docks_to_either_edge() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    let frame = |app: &mut crate::VectorcraftApp, events| control_bar_frame_in(app, &ctx, MENU_SCREEN, events);
    for (item, bottom) in [("Dock to Bottom", true), ("Dock to Top", false)] {
        app.run("window.control.dock", json!({"edge": "float"})).unwrap();
        frame(&mut app, vec![]);
        let menu = node_center(&frame(&mut app, vec![]), "Panel menu").unwrap_or_else(|| panic!("the floating bar has a panel menu"));
        frame(&mut app, vec![egui::Event::PointerMoved(menu), press_at(menu, true)]);
        frame(&mut app, vec![press_at(menu, false)]);
        let nodes = frame(&mut app, vec![]);
        let checked = nodes.iter().filter_map(|n| n.label()).filter(|l| l.starts_with('✓') && l.contains("Dock")).count();
        assert_eq!(checked, 0, "neither edge is checked while the bar floats");
        let at = node_center(&nodes, item).unwrap_or_else(|| panic!("the menu lists {item}"));
        frame(&mut app, vec![egui::Event::PointerMoved(at), press_at(at, true)]);
        frame(&mut app, vec![press_at(at, false)]);
        assert!(!app.ui.control_bar_floating && app.ui.control_bar_bottom == bottom, "{item} docks the bar");
    }
}

/// Escape during a gripper drag cancels it: a docked bar stays at its edge and a floating one where
/// it floats.
#[test]
fn control_bar_drag_escape_restores_placement() {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let escape =
        || vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }];
    drag_gripper_in(&mut app, &ctx, DOCK_SCREEN, docked_grip(false), egui::pos2(400.0, 150.0), escape());
    assert!(!app.ui.control_bar_floating && !app.ui.control_bar_bottom, "the docked bar stays at the top");
    assert!(!super::control_bar_moving(&ctx), "the drag is over");
    drag_gripper_in(&mut app, &ctx, DOCK_SCREEN, docked_grip(false), egui::pos2(400.0, 290.0), escape());
    assert!(!app.ui.control_bar_bottom, "a canceled drag over the bottom does not dock it there");
    app.run("window.control.dock", json!({"edge": "float"})).unwrap();
    let at = super::CONTROL_BAR_FLOAT_AT;
    drag_gripper_in(&mut app, &ctx, DOCK_SCREEN, floating_grip(at), egui::pos2(500.0, 200.0), escape());
    assert!(app.ui.control_bar_floating, "the floating bar still floats");
    assert_eq!(app.ui.control_bar_float_pos, None, "where it floated");
    // A bar hidden in the middle of a drag leaves no drag to take a later Escape once the button
    // is released.
    let (from, to) = (floating_grip(at), egui::pos2(500.0, 200.0));
    control_bar_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from), press_at(from, true)]);
    for k in 1..=4 {
        control_bar_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from.lerp(to, k as f32 / 4.0))]);
    }
    assert!(super::control_bar_moving(&ctx), "the drag goes on");
    for events in [vec![press_at(to, false)], vec![]] {
        let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: Some(DOCK_SCREEN), ..Default::default() }, |_| {});
        out.textures_delta.clear();
    }
    assert!(!super::control_bar_moving(&ctx), "released with the bar hidden, the drag is over");
}

/// An Escape in the same frame as the gripper's release is the drag's: the frame's shortcuts, which
/// run before the bar is drawn, still see the drag, and the bar stays where it was.
#[test]
fn control_bar_drag_escape_on_release_frame_is_the_drags() {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let (from, to) = (docked_grip(false), egui::pos2(400.0, 150.0));
    control_bar_frame(&mut app, &ctx, vec![]);
    control_bar_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from), press_at(from, true)]);
    for k in 1..=4 {
        control_bar_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from.lerp(to, k as f32 / 4.0))]);
    }
    let escape = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    let mut moving = None;
    let input = egui::RawInput { events: vec![escape, press_at(to, false)], screen_rect: Some(DOCK_SCREEN), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| {
        moving = Some(super::control_bar_moving(ui.ctx()));
        super::control_bar(&mut app, ui);
    });
    out.textures_delta.clear();
    assert_eq!(moving, Some(true), "the release frame's shortcuts see the drag");
    assert!(!app.ui.control_bar_floating && !app.ui.control_bar_bottom, "the canceled drag leaves the bar at the top");
    control_bar_frame(&mut app, &ctx, vec![]);
    assert!(!super::control_bar_moving(&ctx), "the drag is over");
}

/// A gripper drag released outside the window leaves the bar where it was.
#[test]
fn control_bar_drag_release_outside_window_keeps_previous_placement() {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    drag_gripper(&mut app, &ctx, docked_grip(false), egui::pos2(400.0, DOCK_SCREEN.bottom() + 120.0));
    assert!(!app.ui.control_bar_floating && !app.ui.control_bar_bottom, "the bar stays at the top");
    app.run("window.control.dock", json!({"edge": "float"})).unwrap();
    drag_gripper(&mut app, &ctx, floating_grip(super::CONTROL_BAR_FLOAT_AT), egui::pos2(-50.0, 150.0));
    assert!(app.ui.control_bar_floating && app.ui.control_bar_float_pos.is_none(), "the floating bar stays where it was");
}

/// In a window too short for a floating bar between the two docking zones, a release docks the bar
/// at the nearer edge; a floating bar saved on a bigger window is drawn inside a smaller one, its
/// panel menu in reach, without docking it.
#[test]
fn floating_control_bar_stays_reachable_in_tiny_and_resized_windows() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    let tiny = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 70.0));
    drag_gripper_in(&mut app, &ctx, tiny, egui::pos2(13.0, 17.0), egui::pos2(300.0, 40.0), vec![]);
    assert!(!app.ui.control_bar_floating && app.ui.control_bar_bottom, "released below the middle: docked at the bottom");
    app.run("window.control.dock", json!({"edge": "float"})).unwrap();
    app.ui.control_bar_float_pos = Some([1000.0, 250.0]);
    let small = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500.0, 120.0));
    control_bar_frame_in(&mut app, &ctx, small, vec![]);
    let menu = node_center(&control_bar_frame_in(&mut app, &ctx, small, vec![]), "Panel menu")
        .unwrap_or_else(|| panic!("the floating bar has a panel menu"));
    assert!(small.shrink(1.0).contains(menu), "the panel menu is inside the smaller window: {menu:?}");
    assert!(app.ui.control_bar_floating, "a smaller window does not dock the bar");
    // The gripper too: a drag from where the bar is drawn moves it.
    let grip = egui::pos2(small.right() - 500.0 + 14.0, small.bottom() - 34.0 + 17.0);
    drag_gripper_in(&mut app, &ctx, small, grip, egui::pos2(200.0, 60.0), vec![]);
    assert!(app.ui.control_bar_floating, "moved inside the smaller window");
    assert_eq!(app.ui.control_bar_float_pos, Some([0.0, 43.0]), "kept inside the window, its gripper under the pointer");
}

/// `window.control.dock {edge}` docks the Control bar to the top or the bottom of the window, or
/// floats it, and the panel menu checks the edge in use; another edge, or none, is an error and
/// changes nothing. A floating bar floats at the position it last floated at, docked in between
/// or hidden by Tab and shown again.
#[test]
fn the_control_bar_dock_command_takes_top_bottom_or_float() {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    assert_eq!(app.run("window.control.dock", json!({"edge": "bottom"})).unwrap(), json!({"edge": "bottom"}));
    assert!(app.ui.control_bar_bottom);
    assert_eq!(crate::menus::checked(&app, "window.control.dock", &json!({"edge": "bottom"})), Some(true));
    assert_eq!(crate::menus::checked(&app, "window.control.dock", &json!({"edge": "top"})), Some(false));
    for p in [json!({"edge": "left"}), json!({"edge": "Bottom"}), json!({"edge": "Float"}), json!({"edge": 1}), json!({})] {
        let err = app.run("window.control.dock", p.clone()).unwrap_err();
        assert_eq!(err, "`edge` must be one of top, bottom, float", "{p}");
        assert!(app.ui.control_bar_bottom && !app.ui.control_bar_floating, "{p} leaves the bar at the bottom");
    }
    assert_eq!(app.run("window.control.dock", json!({"edge": "float"})).unwrap(), json!({"edge": "float"}));
    assert!(app.ui.control_bar_floating && app.ui.control_bar_bottom, "floating, it keeps the edge it docked to last");
    for edge in ["top", "bottom"] {
        assert_eq!(crate::menus::checked(&app, "window.control.dock", &json!({ "edge": edge })), Some(false), "{edge}");
    }
    assert_eq!(crate::menus::checked(&app, "window.control.dock", &json!({"edge": "float"})), Some(true));
    let err = app.run("window.control.dock", json!({"edge": "side"})).unwrap_err();
    assert_eq!(err, "`edge` must be one of top, bottom, float");
    assert!(app.ui.control_bar_floating, "an error leaves the bar floating");
    app.ui.control_bar_float_pos = Some([240.0, 160.0]);
    assert_eq!(app.run("window.control.dock", json!({"edge": "top"})).unwrap(), json!({"edge": "top"}));
    assert!(!app.ui.control_bar_bottom && !app.ui.control_bar_floating);
    app.run("window.control.dock", json!({"edge": "float"})).unwrap();
    assert_eq!(app.ui.control_bar_float_pos, Some([240.0, 160.0]), "it floats where it last floated");
    // Tab hides the floating bar, and Tab again shows it floating where it was.
    app.ui.control_bar = true;
    app.run("window.dock", json!({})).unwrap();
    assert!(!app.ui.control_bar && app.ui.control_bar_floating, "Tab hides the bar");
    app.run("window.dock", json!({})).unwrap();
    assert!(app.ui.control_bar && app.ui.control_bar_floating, "Tab shows it floating again");
    assert_eq!(app.ui.control_bar_float_pos, Some([240.0, 160.0]));
}

/// The labels of the Control bar panel menu's groups of options, in the menu's order.
const CONTROL_BAR_GROUP_LABELS: [&str; 15] = [
    "Snapping",
    "Anchor Points",
    "Tool Options",
    "Object Options",
    "Image Options",
    "Fill and Stroke",
    "Opacity",
    "Graphic Style",
    "Character",
    "Paragraph",
    "Vertical Alignment",
    "Align and Distribute",
    "Transform",
    "Artboard Options",
    "Document Options",
];

/// Opens the Control bar's panel menu, at the right end of the bar `control_frame` draws, unless
/// it is open, and returns the text the bar and the menu show.
fn open_control_bar_menu(app: &mut crate::VectorcraftApp, ctx: &egui::Context) -> Vec<(String, egui::Rect)> {
    use crate::tests_removeanchors::{click_control, control_frame};
    let open = |texts: &[(String, egui::Rect)]| texts.iter().any(|(t, _)| t.ends_with("Dock to Bottom"));
    let texts = control_frame(app, ctx, vec![]);
    if open(&texts) {
        return texts;
    }
    click_control(app, ctx, egui::pos2(1387.0, 17.0));
    let texts = control_frame(app, ctx, vec![]);
    assert!(open(&texts), "the panel menu opens: {texts:?}");
    texts
}

/// Turns group `label` of the Control bar's panel menu on or off with a click on its item, which
/// shows checked while the group is on.
fn toggle_control_bar_group(app: &mut crate::VectorcraftApp, ctx: &egui::Context, label: &str, on: bool) {
    let menu = open_control_bar_menu(app, ctx);
    let item = if on { format!("✓ {label}") } else { format!("   {label}") };
    crate::tests_removeanchors::click_control(app, ctx, crate::tests_removeanchors::at(&menu, &item));
}

/// The Control bar's panel menu lists every group of options, each checked while it is on, and a
/// click on one turns that group off or on again. A group that is off stays out of the bar when
/// the selection changes, the other groups and the menu button stay, and with no document the
/// menu still lists every group.
#[test]
fn the_control_bar_panel_menu_turns_each_group_on_or_off() {
    use crate::tests_removeanchors::{control_frame, has};
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let on = |app: &crate::VectorcraftApp, group: &str| crate::menus::checked(app, "window.control.options", &json!({ "group": group }));
    let menu = open_control_bar_menu(&mut app, &ctx);
    for label in CONTROL_BAR_GROUP_LABELS {
        assert!(has(&menu, &format!("✓ {label}")), "{label} is listed, on");
    }
    assert!(has(&menu, "✓ Dock to Top") && has(&menu, "   Dock to Bottom"), "the edge items stay: {menu:?}");
    toggle_control_bar_group(&mut app, &ctx, "Fill and Stroke", true);
    assert_eq!(on(&app, "fillStroke"), Some(false));
    assert!(has(&open_control_bar_menu(&mut app, &ctx), "   Fill and Stroke"), "listed, off, with no document");
    toggle_control_bar_group(&mut app, &ctx, "Fill and Stroke", false);
    assert_eq!(on(&app, "fillStroke"), Some(true));

    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap()["id"].clone();
    app.run("select.set", json!({ "ids": [id] })).unwrap();
    let bar = control_frame(&mut app, &ctx, vec![]);
    for s in ["Snapping", "Stroke:", "Opacity:", "Style:", "Transform"] {
        assert!(has(&bar, s), "{s} shows with every group on");
    }
    toggle_control_bar_group(&mut app, &ctx, "Fill and Stroke", true);
    let bar = control_frame(&mut app, &ctx, vec![]);
    assert!(!has(&bar, "Stroke:"), "Fill and Stroke is off");
    for s in ["Snapping", "Opacity:", "Style:", "Transform"] {
        assert!(has(&bar, s), "{s} stays");
    }
    app.run("select.set", json!({ "ids": [] })).unwrap();
    let bar = control_frame(&mut app, &ctx, vec![]);
    assert!(has(&bar, "Document Setup") && !has(&bar, "Stroke:"), "still off with nothing selected");
    app.run("select.set", json!({ "ids": [id] })).unwrap();
    assert!(!has(&control_frame(&mut app, &ctx, vec![]), "Stroke:"), "still off with the rectangle selected again");
    toggle_control_bar_group(&mut app, &ctx, "Fill and Stroke", false);
    assert!(has(&control_frame(&mut app, &ctx, vec![]), "Stroke:"), "on again");
}

/// With every group of options off, the Control bar keeps its gripper, the selection's label and
/// its panel menu, which lists every group off; nothing selected shows no Document Setup either.
#[test]
fn the_control_bar_keeps_its_label_and_menu_with_every_group_off() {
    use crate::tests_removeanchors::{control_frame, has};
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap()["id"].clone();
    app.run("select.set", json!({ "ids": [id] })).unwrap();
    for group in crate::chrome::CONTROL_BAR_GROUPS {
        app.run("window.control.options", json!({ "group": group, "visible": false })).unwrap();
    }
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let bar = control_frame(&mut app, &ctx, vec![]);
    assert!(has(&bar, crate::panels::appearance::object_label(&app)), "the label stays: {bar:?}");
    for s in ["Snapping", "Stroke:", "Opacity:", "Style:", "Transform", "X:"] {
        assert!(!has(&bar, s), "{s} is off");
    }
    let menu = open_control_bar_menu(&mut app, &ctx);
    for label in CONTROL_BAR_GROUP_LABELS {
        assert!(has(&menu, &format!("   {label}")), "{label} is listed, off");
    }
    app.run("select.set", json!({ "ids": [] })).unwrap();
    let bar = control_frame(&mut app, &ctx, vec![]);
    assert!(!has(&bar, "Document Setup") && !has(&bar, "Preferences"), "{bar:?}");
}

/// `window.control.options {group, visible?}` turns a group of the Control bar's options on or
/// off, or toggles it, and the panel menu's item shows it; an unknown or missing group, or a
/// `visible` that is not true or false, is an error and changes nothing.
#[test]
fn the_control_bar_options_command_turns_a_group_on_or_off() {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    let checked = |app: &crate::VectorcraftApp, group: &str| crate::menus::checked(app, "window.control.options", &json!({ "group": group }));
    for group in crate::chrome::CONTROL_BAR_GROUPS {
        assert_eq!(checked(&app, group), Some(true), "{group} starts on");
    }
    let run = |app: &mut crate::VectorcraftApp, p: serde_json::Value| app.run("window.control.options", p);
    assert_eq!(run(&mut app, json!({"group": "opacity", "visible": false})).unwrap(), json!({"group": "opacity", "visible": false}));
    assert_eq!(checked(&app, "opacity"), Some(false));
    assert_eq!(run(&mut app, json!({"group": "opacity", "visible": false})).unwrap(), json!({"group": "opacity", "visible": false}), "off stays off");
    assert_eq!(run(&mut app, json!({"group": "opacity"})).unwrap(), json!({"group": "opacity", "visible": true}), "toggled on");
    assert_eq!(run(&mut app, json!({"group": "transform"})).unwrap(), json!({"group": "transform", "visible": false}), "toggled off");
    assert_eq!(run(&mut app, json!({"group": "transform", "visible": null})).unwrap(), json!({"group": "transform", "visible": true}));
    assert_eq!(run(&mut app, json!({"group": "paragraph", "visible": false})).unwrap()["visible"], json!(false));
    let before = app.ui.control_bar_hidden_groups.clone();
    for (p, err) in [
        (json!({"group": "stroke"}), "`group` must be one of"),
        (json!({"group": "Opacity"}), "`group` must be one of"),
        (json!({"group": 1}), "`group` must be one of"),
        (json!({}), "`group` must be one of"),
        (json!({"group": "paragraph", "visible": "true"}), "`visible` must be true or false"),
        (json!({"group": "opacity", "visible": 0}), "`visible` must be true or false"),
    ] {
        let e = run(&mut app, p.clone()).unwrap_err();
        assert!(e.starts_with(err), "{p}: {e}");
        assert_eq!(app.ui.control_bar_hidden_groups, before, "{p} changes nothing");
    }
    assert_eq!(checked(&app, "paragraph"), Some(false));
    assert_eq!(checked(&app, "stroke"), Some(false), "an unknown group is never checked");
}

/// A group of options that is off takes no room in the Control bar: with Fill and Stroke, Opacity
/// and Graphic Style off, or Transform off, the Align buttons fit a narrower window, and the panel
/// menu button still shows there.
#[test]
fn groups_that_are_off_leave_their_room_to_the_later_groups() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    let ids: Vec<serde_json::Value> =
        [10, 100].iter().map(|x| app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 30, "height": 20})).unwrap()["id"].clone()).collect();
    app.run("select.set", json!({ "ids": ids })).unwrap();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    // The narrowest window, in steps of 10 points, whose bar shows the Align buttons.
    let narrowest = |app: &mut crate::VectorcraftApp| {
        (40..=240).map(|w| w as f32 * 10.0).find(|&w| {
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, 300.0));
            control_bar_frame_in(app, &ctx, screen, vec![]);
            let nodes = control_bar_frame_in(app, &ctx, screen, vec![]);
            node_center(&nodes, "Horizontal Align Left").is_some() && node_center(&nodes, "Panel menu").is_some()
        })
    };
    let all = narrowest(&mut app).expect("the Align buttons show in a wide window");
    for group in ["fillStroke", "opacity", "graphicStyle"] {
        app.run("window.control.options", json!({ "group": group, "visible": false })).unwrap();
    }
    let fewer = narrowest(&mut app).expect("they show with fewer groups");
    assert!(fewer < all, "the Align buttons fit {fewer} wide with three groups off, {all} with every group on");
    for group in ["fillStroke", "opacity", "graphicStyle"] {
        app.run("window.control.options", json!({ "group": group, "visible": true })).unwrap();
    }
    app.run("window.control.options", json!({"group": "transform", "visible": false})).unwrap();
    let no_transform = narrowest(&mut app).expect("they show without Transform");
    assert!(no_transform < all, "without the Transform link's room the Align buttons fit {no_transform} wide, {all} with it");
}

/// With the Artboard tool, the Artboard Options group holds the artboard's own controls and the
/// Transform group the Transform link and the X/Y/W/H fields; each one off leaves the other, and
/// with Artboard Options off the fields find room in a window that is too narrow for them otherwise.
#[test]
fn the_artboard_tool_s_control_bar_follows_its_two_groups() {
    use crate::tests_removeanchors::{control_frame, has};
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    app.select_tool("artboard");
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let bar = control_frame(&mut app, &ctx, vec![]);
    for s in ["Artboard", "Name:", "Transform", "Move Artwork with Artboard"] {
        assert!(has(&bar, s), "{s} shows with every group on: {bar:?}");
    }
    assert!(!has(&bar, "X:"), "the fields need more room than is left: {bar:?}");
    app.run("window.control.options", json!({"group": "artboardOptions", "visible": false})).unwrap();
    let bar = control_frame(&mut app, &ctx, vec![]);
    assert!(!has(&bar, "Name:") && !has(&bar, "Move Artwork with Artboard"), "{bar:?}");
    assert!(has(&bar, "Artboard") && has(&bar, "Transform") && has(&bar, "X:"), "{bar:?}");
    app.run("window.control.options", json!({"group": "artboardOptions", "visible": true})).unwrap();
    app.run("window.control.options", json!({"group": "transform", "visible": false})).unwrap();
    let bar = control_frame(&mut app, &ctx, vec![]);
    assert!(!has(&bar, "Transform") && !has(&bar, "X:"), "{bar:?}");
    assert!(has(&bar, "Name:") && has(&bar, "Move Artwork with Artboard"), "{bar:?}");
}

/// The Anchor Points group holds Convert and Anchors for direct-selected anchors; off, the bar keeps
/// its Anchor Point label.
#[test]
fn the_anchor_points_group_holds_convert_and_anchors() {
    use crate::tests_removeanchors::{control_frame, has};
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.select_tool("directSelection");
    app.run("select.anchors", json!({"id": id, "anchors": [[0, 0]]})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let bar = control_frame(&mut app, &ctx, vec![]);
    assert!(has(&bar, "Convert:") && has(&bar, "Anchors:"), "{bar:?}");
    app.run("window.control.options", json!({"group": "anchors", "visible": false})).unwrap();
    let bar = control_frame(&mut app, &ctx, vec![]);
    assert!(!has(&bar, "Convert:") && !has(&bar, "Anchors:"), "{bar:?}");
    assert!(has(&bar, "Anchor Point") && has(&bar, "Stroke:"), "{bar:?}");
}

/// Menus stop above a Control bar docked at the bottom when the status bar is hidden, above the
/// status and hint bars when they show, and at the bottom of the window when the bar is docked at
/// the top or a screen mode hides it.
#[test]
fn menus_stop_above_a_control_bar_docked_at_the_bottom() {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    let floor = |app: &mut crate::VectorcraftApp, status: bool, bottom: bool, screen_mode: u8| {
        (app.ui.status_bar, app.ui.help_bar, app.ui.control_bar, app.ui.control_bar_bottom, app.ui.screen_mode) =
            (status, status, true, bottom, screen_mode);
        for _ in 0..3 {
            let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
                app.logic(ui.ctx());
                app.ui(ui);
            });
            out.textures_delta.clear();
        }
        crate::widgets::menu_floor(&ctx)
    };
    let bar_top = screen.bottom() - 34.0;
    let f = floor(&mut app, false, true, 0);
    assert!((f - bar_top).abs() < 1.0, "status bar hidden: menus stop at the Control bar's top, {f}");
    let f = floor(&mut app, false, true, 2);
    assert_eq!(f, screen.bottom(), "screen mode 2 hides the Control bar: menus stop at the window's bottom");
    let f = floor(&mut app, false, true, 0);
    assert!((f - bar_top).abs() < 1.0, "shown again: {f}");
    let f = floor(&mut app, false, false, 0);
    assert_eq!(f, screen.bottom(), "docked at the top: menus stop at the window's bottom");
    let f = floor(&mut app, true, true, 0);
    assert!(f < bar_top - 1.0, "the status and hint bars show above the Control bar: menus stop above them, {f}");
    // Floating after docking at the bottom, the bar is no bottom bar.
    app.ui.control_bar_floating = true;
    let f = floor(&mut app, false, true, 0);
    assert_eq!(f, screen.bottom(), "floating: menus stop at the window's bottom");
}

/// With the Artboard tool in use, the Control bar shows the Align buttons after the artboard
/// controls while it has room, and the Distribute buttons for two or more selected artboards. A
/// click aligns the selected artboards as the Align panel's button does. The group is left out
/// while the panel menu has it off.
#[test]
fn the_control_bar_aligns_the_selected_artboards_with_the_artboard_tool() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("artboard.new", json!({"x": 150, "y": 40, "width": 60, "height": 80})).unwrap();
    app.select_tool("artboard");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(2400.0, 300.0));
    let frame = |app: &mut crate::VectorcraftApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: Some(screen), ..Default::default() }, |ui| super::control_bar(app, ui));
        out.textures_delta.clear();
        out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default()
    };
    let button = |app: &mut crate::VectorcraftApp, name: &str| {
        frame(app, vec![]);
        let nodes = frame(app, vec![]);
        let n = nodes.iter().find(|(_, n)| n.label() == Some(name)).map(|(_, n)| n)?;
        n.bounds().map(|b| egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32)))
    };
    app.session.set_tool_option("selected", &json!([1]));
    assert!(button(&mut app, "Vertical Align Top").is_some(), "one artboard: the Align buttons");
    assert!(button(&mut app, "Horizontal Distribute Left").is_none(), "one artboard: no Distribute buttons");
    app.session.set_tool_option("selected", &json!([1, 0]));
    assert!(button(&mut app, "Horizontal Distribute Left").is_some(), "two artboards: the Distribute buttons");
    let r = button(&mut app, "Vertical Align Top").expect("Vertical Align Top");
    let press =
        |pressed| egui::Event::PointerButton { pos: r.center(), button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    for events in [vec![egui::Event::PointerMoved(r.center()), press(true)], vec![press(false)]] {
        frame(&mut app, events);
    }
    let tops: Vec<f64> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect.y0).collect();
    assert_eq!(tops, [0.0, 0.0], "{}", app.ui.status);
    app.run("window.control.options", json!({"group": "alignDistribute", "visible": false})).unwrap();
    assert!(button(&mut app, "Vertical Align Top").is_none(), "the group is off");
}

/// Two or more selected objects show all six Distribute Objects buttons in the Control bar;
/// a single object keeps the bar as it was. A click distributes as the Align panel's button does.
#[test]
fn the_control_bar_shows_the_distribute_buttons_for_two_or_more_objects() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    let rect = |app: &mut crate::VectorcraftApp, x: f64| {
        app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 30, "height": 20})).unwrap()["id"].clone()
    };
    let (a, b, c) = (rect(&mut app, 10.0), rect(&mut app, 100.0), rect(&mut app, 200.0));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 300.0));
    let frame = |app: &mut crate::VectorcraftApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: Some(screen), ..Default::default() }, |ui| super::control_bar(app, ui));
        out.textures_delta.clear();
        out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default()
    };
    const DISTRIBUTE: [&str; 6] = [
        "Vertical Distribute Top",
        "Vertical Distribute Center",
        "Vertical Distribute Bottom",
        "Horizontal Distribute Left",
        "Horizontal Distribute Center",
        "Horizontal Distribute Right",
    ];
    let rects = |app: &mut crate::VectorcraftApp| {
        frame(app, vec![]);
        let nodes = frame(app, vec![]);
        DISTRIBUTE.map(|name| {
            let n = nodes.iter().find(|(_, n)| n.label() == Some(name)).map(|(_, n)| n)?;
            n.bounds().map(|b| egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32)))
        })
    };
    app.run("select.set", json!({"ids": [a]})).unwrap();
    assert!(rects(&mut app).iter().all(Option::is_none), "one object: no distribute buttons");
    app.run("select.set", json!({"ids": [a, b, c]})).unwrap();
    let rects = rects(&mut app);
    assert!(rects.iter().all(Option::is_some), "three objects: {rects:?}");
    let r = rects[3].expect("Horizontal Distribute Left");
    let press =
        |pressed| egui::Event::PointerButton { pos: r.center(), button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    for events in [vec![egui::Event::PointerMoved(r.center()), press(true)], vec![press(false)]] {
        frame(&mut app, events);
    }
    let left = |app: &crate::VectorcraftApp, id: &serde_json::Value| {
        app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().geometric_bounds().unwrap().x0
    };
    assert_eq!((left(&app, &a), left(&app, &b), left(&app, &c)), (10.0, 105.0, 200.0), "{}", app.ui.status);
}

/// The Control bar's Align To menu follows its Align buttons. It shows the Align panel's Align To
/// choice (Artboard for one object, which the buttons align to the artboard) and screen readers
/// get it as the value of a combo box named "Align To:". Its menu checks that choice, has Align to
/// Key Object off while no key object can be made, and a pick chooses as the Align panel does:
/// Artboard and Selection are stored with the UI state and let go of the key object, and Key
/// Object makes the topmost selected object the key, keeping the stored choice. The Align
/// buttons then align to the choice.
#[test]
fn the_control_bar_align_to_menu_chooses_what_align_aligns_to() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    let rect = |app: &mut crate::VectorcraftApp, x: f64| {
        app.run("shape.rectangle", json!({"x": x, "y": 40, "width": 30, "height": 20})).unwrap()["id"].clone()
    };
    let (a, b) = (rect(&mut app, 50.0), rect(&mut app, 150.0));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 300.0));
    let frame = |app: &mut crate::VectorcraftApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: Some(screen), ..Default::default() }, |ui| super::control_bar(app, ui));
        out.textures_delta.clear();
        out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default()
    };
    type Nodes = Vec<(egui::accesskit::NodeId, egui::accesskit::Node)>;
    // A control by its name; a menu row's name follows its check column.
    let find = |nodes: &Nodes, name: &str| {
        nodes.iter().find(|(_, n)| n.label().is_some_and(|l| l.trim_start_matches(['✓', ' ']) == name)).map(|(_, n)| n.clone())
    };
    let center = |n: &egui::accesskit::Node| {
        let b = n.bounds().unwrap_or_else(|| panic!("{n:?}"));
        egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)
    };
    let click = |app: &mut crate::VectorcraftApp, at: egui::Pos2| {
        let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(app, vec![egui::Event::PointerMoved(at), press(true)]);
        frame(app, vec![press(false)]);
        frame(app, vec![]);
        frame(app, vec![])
    };
    let menu = |app: &mut crate::VectorcraftApp| {
        frame(app, vec![]);
        let nodes = frame(app, vec![]);
        find(&nodes, "Align To:").unwrap_or_else(|| panic!("the Control bar has the Align To menu"))
    };
    // Opens the menu, checks its rows and picks `pick`, or closes it with Escape.
    let choose = |app: &mut crate::VectorcraftApp, checked: &str, key_on: bool, pick: Option<&str>| {
        let at = center(&menu(app));
        let nodes = click(app, at);
        for name in ["Align to Selection", "Align to Key Object", "Align to Artboard"] {
            let row = find(&nodes, name).unwrap_or_else(|| panic!("the menu lists {name}"));
            assert_eq!(row.label().is_some_and(|l| l.starts_with('✓')), name == checked, "{name} checked");
            assert_eq!(row.is_disabled(), name == "Align to Key Object" && !key_on, "{name} off");
        }
        let nodes = match pick {
            Some(pick) => click(app, center(&find(&nodes, pick).unwrap_or_else(|| panic!("{pick}")))),
            None => {
                let escape =
                    |pressed| egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
                frame(app, vec![escape(true)]);
                frame(app, vec![escape(false)]);
                frame(app, vec![])
            }
        };
        assert!(find(&nodes, "Align to Artboard").is_none(), "the menu closes");
    };
    let key = |app: &crate::VectorcraftApp| app.session.active().unwrap().selection.key;
    let x0 = |app: &crate::VectorcraftApp, id: &serde_json::Value| {
        app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().geometric_bounds().unwrap().x0
    };

    // One object: the buttons align it to the artboard, and Align to Key Object is off.
    app.run("select.set", json!({"ids": [a]})).unwrap();
    let m = menu(&mut app);
    assert_eq!(m.role(), egui::accesskit::Role::ComboBox);
    assert_eq!(m.value(), Some("Align to Artboard"), "one object shows Artboard");
    choose(&mut app, "Align to Artboard", false, None);
    assert_eq!(key(&app), None);

    // Two objects: the stored choice, Selection.
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    assert_eq!(menu(&mut app).value(), Some("Align to Selection"));
    choose(&mut app, "Align to Selection", true, Some("Align to Artboard"));
    assert_eq!(app.ui.align_to, crate::panels::align::AlignTo::Artboard, "the choice the Align panel shows");
    assert_eq!(menu(&mut app).value(), Some("Align to Artboard"));
    let left = find(&frame(&mut app, vec![]), "Horizontal Align Left").unwrap_or_else(|| panic!("Horizontal Align Left"));
    click(&mut app, center(&left));
    assert_eq!((x0(&app, &a), x0(&app, &b)), (0.0, 0.0), "aligned to the artboard: {}", app.ui.status);

    // Key Object makes the topmost the key and keeps the stored choice.
    choose(&mut app, "Align to Artboard", true, Some("Align to Key Object"));
    assert_eq!(key(&app), Some(vectorcraft_doc::NodeId(b.as_u64().unwrap())), "the topmost object is the key");
    assert_eq!(app.ui.align_to, crate::panels::align::AlignTo::Artboard, "the stored choice stays");
    assert_eq!(menu(&mut app).value(), Some("Align to Key Object"));

    // Selection lets go of the key and is stored.
    choose(&mut app, "Align to Key Object", true, Some("Align to Selection"));
    assert_eq!(key(&app), None, "the key object is let go");
    assert_eq!(app.ui.align_to, crate::panels::align::AlignTo::Selection);
    assert_eq!(menu(&mut app).value(), Some("Align to Selection"));
}

/// The Control bar shows the Global Edit Options dropdown while Start Global Edit can run. Its
/// options are the session's, which the Properties panel's dropdown also sets: Match Size from
/// the Control bar narrows the Results and the objects Start Global Edit takes. The dropdown is off
/// and closed while Global Edit is on, and absent when the selection cannot start it.
#[test]
fn the_control_bar_shows_the_global_edit_options() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let rect = |app: &mut crate::VectorcraftApp, x: f64, size: f64| {
        app.run("shape.rectangle", json!({"x": x, "y": 20, "width": size, "height": size})).unwrap()["id"].clone()
    };
    let a = rect(&mut app, 20.0, 40.0);
    rect(&mut app, 100.0, 40.0);
    rect(&mut app, 200.0, 80.0);
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 300.0));
    let frame = |app: &mut crate::VectorcraftApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: Some(screen), ..Default::default() }, |ui| super::control_bar(app, ui));
        out.textures_delta.clear();
        out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default()
    };
    type Nodes = Vec<(egui::accesskit::NodeId, egui::accesskit::Node)>;
    let find = |nodes: &Nodes, name: &str| nodes.iter().find(|(_, n)| n.label() == Some(name) || n.value() == Some(name)).map(|(_, n)| n.clone());
    let center = |n: &egui::accesskit::Node| {
        let b = n.bounds().unwrap_or_else(|| panic!("{n:?}"));
        egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)
    };
    let click = |app: &mut crate::VectorcraftApp, at: egui::Pos2| {
        let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(app, vec![egui::Event::PointerMoved(at), press(true)]);
        frame(app, vec![press(false)]);
        frame(app, vec![]);
        frame(app, vec![])
    };
    app.run("select.set", json!({"ids": []})).unwrap();
    frame(&mut app, vec![]);
    assert!(find(&frame(&mut app, vec![]), "Global Edit Options").is_none(), "nothing selected: no dropdown");
    app.run("select.set", json!({"ids": [a]})).unwrap();
    frame(&mut app, vec![]);
    let nodes = frame(&mut app, vec![]);
    let button = find(&nodes, "Global Edit Options").unwrap_or_else(|| panic!("one rectangle: the dropdown shows"));
    let nodes = click(&mut app, center(&button));
    assert!(find(&nodes, "Results: 3").is_some(), "the options open with their Results");
    let size = find(&nodes, "Size").unwrap_or_else(|| panic!("the Size check"));
    let nodes = click(&mut app, center(&size));
    assert!(app.session.global_edit_options.size, "Match Size is the session's option, shared with Properties");
    assert!(find(&nodes, "Results: 2").is_some(), "Size narrows the Results");
    app.run("select.globalEdit", json!({})).unwrap();
    let ge = app.session.active().unwrap().global_edit.as_ref().map(|g| g.members.len());
    assert_eq!(ge, Some(2), "Start Global Edit takes the two rectangles of the same size");
    let nodes = frame(&mut app, vec![]);
    let button = find(&nodes, "Global Edit Options").unwrap_or_else(|| panic!("Global Edit on: the dropdown still shows"));
    assert!(button.is_disabled(), "the options are off while Global Edit is on");
    let results = |nodes: &Nodes| nodes.iter().any(|(_, n)| n.label().or(n.value()).is_some_and(|t| t.starts_with("Result")));
    assert!(!results(&nodes), "Global Edit starting closes the open options");
    assert!(!results(&frame(&mut app, vec![])));
    app.run("select.globalEdit", json!({})).unwrap();
    frame(&mut app, vec![]);
    assert!(!results(&frame(&mut app, vec![])), "the options stay closed once Global Edit stops");
    // At any width the button either fits inside the bar or is left out, and a wider bar never
    // leaves it out once a narrower one showed it.
    let mut seen = false;
    for width in (600..=2400).step_by(10) {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width as f32, 300.0));
        let mut nodes = Vec::new();
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| super::control_bar(&mut app, ui));
            out.textures_delta.clear();
            nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
        }
        let shown = find(&nodes, "Global Edit Options").and_then(|n| n.bounds());
        if let Some(b) = shown {
            assert!(b.x1 <= f64::from(width), "{width}: {b:?}");
        }
        assert!(!seen || shown.is_some(), "{width}: left out after a narrower bar showed it");
        seen |= shown.is_some();
    }
    assert!(seen, "a wide bar has room for it");
}

/// At any width the Control bar shows the inline X, Y, W and H fields whole inside its right margin,
/// or leaves all four out and keeps the Transform link; a wider bar never leaves them out once a
/// narrower one showed them. The Align buttons, and for two objects the Distribute buttons, are
/// trimmed the same way ahead of the Transform link (#5372).
#[test]
fn the_control_bar_shows_the_inline_transform_fields_only_when_they_fit() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    for count in [1, 2] {
        let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let ids: Vec<_> = (0..count)
            .map(|i| app.run("shape.rectangle", json!({"x": 20 + 100 * i, "y": 20, "width": 40, "height": 30})).unwrap()["id"].clone())
            .collect();
        app.run("select.set", json!({ "ids": ids })).unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::theme::install_fonts(&ctx);
        let mut seen = false;
        let mut menu_seen = false;
        // The Align group and the Align To menu, then for two objects the Distribute group (#5372):
        // each by its controls.
        let mut groups: Vec<(&str, Vec<&str>)> = vec![(
            "Align",
            vec![
                "Horizontal Align Left",
                "Horizontal Align Center",
                "Horizontal Align Right",
                "Horizontal & Vertical Align Center",
                "Vertical Align Top",
                "Vertical Align Center",
                "Vertical Align Bottom",
            ],
        )];
        groups.push(("Align To", vec!["Align To:"]));
        if count == 2 {
            groups.push((
                "Distribute",
                vec![
                    "Vertical Distribute Top",
                    "Vertical Distribute Center",
                    "Vertical Distribute Bottom",
                    "Horizontal Distribute Left",
                    "Horizontal Distribute Center",
                    "Horizontal Distribute Right",
                ],
            ));
        }
        let mut groups_seen = vec![false; groups.len()];
        for width in (600..=2400).step_by(10) {
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width as f32, 300.0));
            let mut nodes = Vec::new();
            for _ in 0..2 {
                let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| super::control_bar(&mut app, ui));
                out.textures_delta.clear();
                nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
            }
            let spacing = ctx.global_style().spacing.item_spacing.x;
            // The right edge of the box of the field its label names. The text box sits inside the
            // field's box with the same inset on both sides, and one item spacing after the label.
            let right = |name: &str| {
                let (id, label) = nodes.iter().find(|(_, n)| n.role() == egui::accesskit::Role::Label && n.value() == Some(name))?;
                let input = nodes.iter().find(|(_, n)| n.labelled_by().contains(id)).and_then(|(_, n)| n.bounds())?;
                let inset = input.x0 - label.bounds()?.x1 - f64::from(spacing);
                Some(input.x1 + inset)
            };
            let fields = ["X:", "Y:", "W:", "H:"].map(right);
            assert!(fields.iter().all(Option::is_some) || fields.iter().all(Option::is_none), "{count} objects, {width}: {fields:?}");
            assert!(nodes.iter().any(|(_, n)| n.label() == Some("Transform")), "{count} objects, {width}: the Transform link shows");
            // The panel menu button takes the bar's right end, and the bar's content ends before it
            // (without the button, at the bar's 10-point inner margin). Once a bar shows the button,
            // a wider one keeps it: the fields and groups trim to the room left of it.
            let menu = nodes.iter().find(|(_, n)| n.label() == Some("Panel menu")).and_then(|(_, n)| n.bounds());
            assert!(!menu_seen || menu.is_some(), "{count} objects, {width}: the panel menu button left out after a narrower bar showed it");
            menu_seen |= menu.is_some();
            let edge = menu.map_or(f64::from(width) - 10.0, |m| m.x0);
            if let Some(h) = fields[3] {
                assert!(h <= edge, "{count} objects, {width}: H ends at {h}, the content's room at {edge}");
            }
            assert!(!seen || fields[3].is_some(), "{count} objects, {width}: left out after a narrower bar showed them");
            seen |= fields[3].is_some();
            // The Align and Distribute buttons show whole inside the margin, with the Transform
            // link after them, or are left out; once shown, a wider bar keeps them.
            let end = |name: &str| nodes.iter().find(|(_, n)| n.label() == Some(name)).and_then(|(_, n)| n.bounds()).map(|b| b.x1);
            let inside = |x: f64| x <= edge;
            for ((group, names), was) in groups.iter().zip(groups_seen.iter_mut()) {
                let ends: Vec<Option<f64>> = names.iter().map(|n| end(n)).collect();
                let shown = ends.iter().all(Option::is_some);
                assert!(shown || ends.iter().all(Option::is_none), "{count} objects, {width}: {group} {ends:?}");
                if let Some(Some(last)) = ends.last() {
                    assert!(inside(*last), "{count} objects, {width}: {group} ends at {last}");
                    let transform = end("Transform");
                    assert!(transform.is_some_and(inside), "{count} objects, {width}: Transform after {group} ends at {transform:?}");
                }
                assert!(!*was || shown, "{count} objects, {width}: {group} left out after a narrower bar showed it");
                *was |= shown;
            }
        }
        assert!(seen, "{count} objects: a wide bar has room for the fields");
        assert!(menu_seen, "{count} objects: a wide bar has room for the panel menu button");
        assert!(groups_seen.iter().all(|s| *s), "{count} objects: a wide bar shows every group: {groups_seen:?}");
    }
}
