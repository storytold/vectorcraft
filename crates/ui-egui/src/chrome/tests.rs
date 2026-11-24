use std::cell::{RefCell, RefMut};
use std::rc::Rc;
use std::sync::Arc;

use craft::behavior::Mixed;
use craft::ui::runtime::access::Role;
use craft::ui::testing::{Fixture, Target};
use serde_json::json;
use vectorcraft_engine::Session;

use crate::i18n::LangExt;

/// Draws part of the window's chrome over the app.
type Draw = Box<dyn FnMut(&mut crate::VectorcraftApp, &mut egui::Ui)>;

/// The app with part of its chrome shown in a framework fixture. Every frame runs on the fixture's one context, so a
/// press and its release, or a hover and its tooltip, reach the same widgets. The part shown can change between
/// frames ([`Shown::draw`]), as the window shows the tabs, the bars and Properties side by side. The commands the
/// frames fire run as the window runs them.
struct Shown {
    fixture: Fixture,
    app: Rc<RefCell<crate::VectorcraftApp>>,
    draw: Rc<RefCell<Draw>>,
}

impl Shown {
    /// `draw` over `app` in a room of size `room`; no frame has run yet.
    fn new(app: crate::VectorcraftApp, room: egui::Vec2, draw: impl FnMut(&mut crate::VectorcraftApp, &mut egui::Ui) + 'static) -> Self {
        let app = Rc::new(RefCell::new(app));
        let draw: Rc<RefCell<Draw>> = Rc::new(RefCell::new(Box::new(draw)));
        let (shown, drawn) = (Rc::clone(&app), Rc::clone(&draw));
        let mut fixture = Fixture::new(move |ui| {
            let mut draw = drawn.borrow_mut();
            (*draw)(&mut shown.borrow_mut(), ui);
        });
        fixture.room(room);
        // The fixture's context is the app's window, as a host's is.
        app.borrow_mut().ui_ctx = Some(fixture.ctx().clone());
        Shown { fixture, app, draw }
    }

    /// The app. Drop the borrow before the next frame: the frames borrow the app to draw.
    fn app(&self) -> RefMut<'_, crate::VectorcraftApp> {
        self.app.borrow_mut()
    }

    /// Shows `draw` from the next frame on.
    fn draw(&mut self, draw: impl FnMut(&mut crate::VectorcraftApp, &mut egui::Ui) + 'static) -> &mut Self {
        *self.draw.borrow_mut() = Box::new(draw);
        self
    }

    /// One frame of the canvas, as the window draws it beside the chrome: a fit or zoom a command asked for waits for
    /// the canvas (a view request, canvas-w10 note 3). Then the status bar again, and the view keeps the rect the test
    /// gave it.
    fn canvas_pass(&mut self) -> &mut Self {
        let rect = self.app().view().map(|v| v.rect);
        self.draw(crate::canvas::show).frame(vec![]);
        if let Some(rect) = rect {
            let mut app = self.app();
            if let Some(view) = app.view_mut() {
                view.rect = rect;
            }
        }
        self.draw(super::status_bar)
    }

    /// The room every following frame shows the chrome in.
    fn room(&mut self, room: egui::Vec2) -> &mut Self {
        self.fixture.room(room);
        self
    }

    /// One frame with `events`, then the commands it fired.
    fn frame(&mut self, events: Vec<egui::Event>) -> &mut Self {
        self.run_fired();
        self.fixture.frame(events);
        self.run_fired();
        self
    }

    /// Frames with no events until the chrome is at rest, then the commands they fired.
    fn settle(&mut self) -> &mut Self {
        self.run_fired();
        self.fixture.settle();
        self.run_fired();
        self
    }

    /// Runs the commands the frames since the last call fired, as the window runs them, and gives the fixture the
    /// app's input for the next frame.
    fn run_fired(&mut self) {
        crate::testing::fixture_input(&mut self.app.borrow_mut(), &mut self.fixture);
    }

    /// Moves the fixture's clock on by `seconds` before the next frame.
    fn advance(&mut self, seconds: f64) -> &mut Self {
        self.fixture.advance(seconds);
        self
    }

    /// The control named `label` in the latest frame.
    fn find(&self, label: &str) -> Option<Target> {
        self.fixture.by_label(label)
    }

    /// A fresh frame, then the control named `label`.
    fn named(&mut self, label: &str) -> Target {
        self.frame(Vec::new());
        self.find(label).unwrap_or_else(|| panic!("no {label:?} in {:?}", self.names()))
    }

    /// The first node of the latest frame whose name starts with `prefix`, a control before a label.
    fn starting(&self, prefix: &str) -> Option<Target> {
        let tree = self.fixture.tree();
        let named: Vec<_> = tree.keys().into_iter().filter(|key| tree.name_of(*key).is_some_and(|name| name.starts_with(prefix))).collect();
        let first = named.iter().find(|key| tree.role_of(**key) != Some(Role::LABEL)).or(named.first());
        first.and_then(|key| self.fixture.target(*key))
    }

    /// The names the latest frame gives screen readers.
    fn names(&self) -> Vec<String> {
        let tree = self.fixture.tree();
        tree.keys().into_iter().filter_map(|key| tree.name_of(key)).map(str::to_owned).collect()
    }

    /// Whether the latest frame names or paints `text`.
    fn shows(&self, text: &str) -> bool {
        self.find(text).is_some() || self.fixture.texts().iter().any(|t| t.text == text)
    }

    /// The latest frame's AccessKit tree.
    fn tree(&self) -> &craft::ui::runtime::egui::access::AccessTree {
        self.fixture.tree()
    }

    /// The texts the latest frame painted, with where each lies.
    fn texts(&self) -> Vec<StatusText> {
        self.fixture.texts().iter().map(|t| StatusText { text: t.text.clone(), at: t.rect.min, bounds: t.rect }).collect()
    }

    /// Clicks the control named `label`.
    fn click(&mut self, label: &str) -> &mut Self {
        let target = self.named(label);
        self.click_on(&target)
    }

    /// Clicks `target`.
    fn click_on(&mut self, target: &Target) -> &mut Self {
        self.fixture.click(target);
        self.run_fired();
        self
    }

    /// Clicks at `at`, where no control needs to be.
    fn click_at(&mut self, at: egui::Pos2) -> &mut Self {
        self.fixture.click_at(at);
        self.run_fired();
        self
    }

    /// Presses at `from`, moves by `by` and releases.
    fn drag_at(&mut self, from: egui::Pos2, by: egui::Vec2) -> &mut Self {
        self.fixture.drag_at(from, by);
        self.run_fired();
        self
    }

    /// A press and release of `key`.
    fn key(&mut self, key: egui::Key) -> &mut Self {
        self.fixture.key(key, egui::Modifiers::NONE);
        self.run_fired();
        self
    }

    /// Where the pointer is.
    fn pointer(&self) -> Option<egui::Pos2> {
        self.fixture.ctx().input(|i| i.pointer.hover_pos())
    }

    /// Whether the node of `target` is checked.
    fn checked(&self, target: &Target) -> bool {
        self.tree().toggled_of(target.node) == Some(Mixed::Same(true))
    }
}

/// A tab dropped on the strip lands at an index among all the open documents: the documents without a tab (the
/// floating ones) keep their places in the documents' order, and a document with two views moves as one (#5663).
#[test]
fn a_drop_counts_the_documents_the_strip_does_not_show() {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    for _ in 0..6 {
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    }
    for index in [1, 3, 4] {
        app.run("window.arrange.floatInWindow", json!({ "index": index })).unwrap();
    }
    let docked = |app: &crate::VectorcraftApp| crate::doc_windows::docked_windows(app);
    assert_eq!(docked(&app).iter().map(|w| w.0).collect::<Vec<_>>(), [0, 2, 5], "documents 1, 3 and 4 float");
    let dropped = |from: usize, before: Option<usize>| {
        let before = before.map(|d| docked(&app).into_iter().find(|w| w.0 == d).unwrap_or_else(|| panic!("document {d} has a tab")));
        super::tabs::reorder_target(&app, from, before)
    };
    assert_eq!(dropped(0, None), 5, "after the last tab: last, as 0 leaves its place");
    assert_eq!(dropped(0, Some(5)), 4, "before document 5: just before it once 0 has left its place");
    assert_eq!(dropped(5, Some(0)), 0, "before document 0: first");
    assert_eq!(dropped(5, Some(2)), 2, "before document 2: the floating document 1 stays before it");
    assert_eq!(dropped(2, None), 5, "after the last tab");
    // Document 1 has two views, side by side on the strip.
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    for _ in 0..3 {
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    }
    crate::doc_windows::show_window(&mut app, (1, 1));
    app.run("window.newWindow", json!({})).unwrap();
    let views: Vec<_> = docked(&app).into_iter().filter(|w| w.0 == 1).collect();
    assert_eq!(views.len(), 2, "document 1 has two tabs");
    let first = docked(&app).into_iter().find(|w| w.0 == 0);
    assert_eq!(super::tabs::reorder_target(&app, 1, first), 0, "both views' document goes first");
    for view in views {
        assert_eq!(super::tabs::reorder_target(&app, 2, Some(view)), 1, "before either of document 1's tabs: before the document");
    }
    let last = docked(&app).into_iter().find(|w| w.0 == 2);
    assert_eq!(super::tabs::reorder_target(&app, 0, last), 1, "past both of document 1's tabs: after the document");
}

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
fn hints_name_keys_as_the_menus_do() {
    // A drawn frame gives the window its keymap, which the Search Commands shortcut is read from.
    let mut app = crate::VectorcraftApp::new(Session::new(), crate::Services::default());
    crate::tests_synthetic::key_frame(&mut app, &crate::testing::ui_context(), vec![]);
    let text = |tool: &str| super::hint_segments(&app, tool).into_iter().map(|(s, _)| s).collect::<String>();
    for tool in vectorcraft_tools::catalog::all_tools().map(|t| t.id) {
        let hint = text(tool);
        assert!(!hint.contains("Option"), "{tool}: {hint}");
        assert!(cfg!(target_os = "macos") || !hint.contains("Cmd"), "{tool}: {hint}");
    }
    let pretty = crate::keymap::chord_text;
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

/// As the window narrows, the app bar gives up its right-hand controls one at a time, each before the one after it:
/// Discord, Explore tutorials, Arrange Documents, the search and last the workspace switcher; each keeps its whole width
/// until it goes. Whatever shows lies inside the window. Safe Mode's rows are in `tests/safe_mode.rs`, whose
/// process owns the Safe Mode switch.
#[test]
fn the_app_bar_gives_up_its_right_hand_controls_in_order() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let app = crate::VectorcraftApp::new(Session::new(), Default::default());
    // The bar is the shell's title bar, which only the whole window's frame draws.
    let mut bar = Shown::new(app, egui::vec2(1600.0, 600.0), |app, ui| app.ui(ui));
    // PLAN: shell-w9 builds Explore tutorials as `IconButton::command(.., "window.learn")`, whose command label is "Learn"; the button keeps production's name "Explore tutorials".
    // In the order the bar gives them up; the search's name goes on with its chord.
    let controls = ["Discord", "Explore tutorials", "Arrange Documents", "Search commands", "Choose a workspace"];
    let mut before: Option<Vec<bool>> = None;
    let mut search_widths = vec![];
    for width in (30..=160).rev().map(|w| w as f32 * 10.0) {
        bar.room(egui::vec2(width, 600.0));
        bar.frame(Vec::new()).frame(Vec::new());
        let shown: Vec<Option<Target>> = controls.iter().map(|c| bar.starting(c)).collect();
        for (control, target) in controls.iter().zip(&shown) {
            if let Some(t) = target {
                assert!(t.rect.left() >= 0.0 && t.rect.right() <= width, "{width}: {control} at {:?} lies inside the window", t.rect);
            }
        }
        let on: Vec<bool> = shown.iter().map(Option::is_some).collect();
        assert!(width < 1600.0 || on.iter().all(|o| *o), "a wide bar shows every control: {on:?}");
        assert!(on.windows(2).all(|w| !w[0] || w[1]), "{width}: a control outlasted one the bar gives up after it: {on:?}");
        if let Some(before) = &before {
            assert!(on.iter().zip(before).all(|(now, was)| !now || *was), "{width}: a control came back as the window narrowed: {on:?}");
        }
        if let Some(search) = shown[3] {
            search_widths.push(search.rect.width());
        }
        before = Some(on);
    }
    let (widest, narrowest) = (search_widths.first().copied(), search_widths.last().copied());
    assert!(widest.zip(narrowest).is_some_and(|(w, n)| n >= w - 0.5), "the search field keeps its width until it goes: {search_widths:?}");
}

/// A text the status bar painted, with where it lies.
#[derive(Debug)]
struct StatusText {
    text: String,
    at: egui::Pos2,
    bounds: egui::Rect,
}

/// The texts of the status bar at rest.
fn status_readout_frame(bar: &mut Shown) -> Vec<StatusText> {
    bar.settle().texts()
}

/// Find a required text record by prefix, failing the test when it is absent.
fn status_text<'a>(texts: &'a [StatusText], prefix: &str) -> &'a StatusText {
    texts.iter().find(|t| t.text.starts_with(prefix)).unwrap_or_else(|| panic!("missing {prefix:?}: {texts:?}"))
}

/// The status bar shown alone over `app`, in a window 1200 points wide.
fn status_shown(app: crate::VectorcraftApp) -> Shown {
    Shown::new(app, egui::vec2(1200.0, 300.0), super::status_bar)
}

/// An English test document with a status message, its status bar shown in a window 1400 points wide.
fn status_readout_app() -> Shown {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    app.ui.status = "Saved recovery data for Untitled-1".to_owned();
    let mut bar = status_shown(app);
    bar.room(egui::vec2(1400.0, 300.0));
    bar
}

/// The status bar keeps the message and every readout where they are while the readouts' values change: a readout
/// takes the width of its widest value, so a longer value does not push the ones after it, and the readouts never
/// overlap. Screen readers get a readout's whole value.
#[test]
fn the_status_bar_stands_still_while_its_readouts_change() {
    let mut bar = status_readout_app();
    let mut baseline = None;
    for (render, frame, fps, x, y) in [
        (233.4, 4.9, 80.0, -662.25, 1647.9),
        (9.8, 1.0, 120.0, 7.0, 5.0),
        (100000.0, 10000.0, 1000.0, -1000.0, 100000.0),
        (99999.9, 9999.9, 999.0, -999.99, 9999.99),
    ] {
        let (x_value, y_value) = {
            let mut app = bar.app();
            app.perf = crate::Perf { render_ms: render, frame_ms: frame, fps };
            app.hover_doc = Some(vectorcraft_geom::Point::new(x, y));
            (app.session.general_unit().readout(x), app.session.general_unit().readout(y))
        };
        let texts = status_readout_frame(&mut bar);
        let fps_text = format!("{fps:.0} fps");
        let readouts = [
            ("render", format!("{render:.1} ms")),
            ("ui ", format!("{frame:.1} ms")),
            (fps_text.as_str(), fps_text.clone()),
            ("X:", x_value),
            ("Y:", y_value),
        ];
        let mut fields: Vec<&StatusText> = readouts
            .iter()
            .map(|(prefix, value)| {
                let text = status_text(&texts, prefix);
                assert!(text.text.contains(value.as_str()), "{text:?} must contain {value:?}");
                text
            })
            .collect();
        fields.sort_by(|a, b| a.at.x.total_cmp(&b.at.x));
        assert!(fields.windows(2).all(|w| w[0].bounds.right() <= w[1].bounds.left()), "a readout overlaps the next: {fields:?}");
        let lefts = ["Saved recovery data", "render", "ui ", fps_text.as_str(), "X:", "Y:"].map(|p| status_text(&texts, p).at.x);
        if let Some(first) = baseline {
            assert_eq!(lefts, first, "the message or a readout moved as the values changed");
        } else {
            baseline = Some(lefts);
        }
    }
    bar.app().perf.render_ms = 1000000.0;
    bar.settle();
    assert!(bar.find("render 1000000.0 ms").is_some(), "screen readers receive the complete value: {:?}", bar.names());
}

/// The status message stays where it is as the pointer enters the canvas and leaves it, and the coordinates show only
/// while it is over the canvas.
#[test]
fn status_messages_keep_their_origin_when_the_pointer_enters_and_leaves() {
    let mut bar = status_readout_app();
    let mut first = None;
    for hover in [None, Some(vectorcraft_geom::Point::new(7.0, 5.0)), None] {
        bar.app().hover_doc = hover;
        let texts = status_readout_frame(&mut bar);
        let left = status_text(&texts, "Saved recovery data").at.x;
        if let Some(first) = first {
            assert_eq!(left, first, "the message moved as the pointer went {}", if hover.is_some() { "in" } else { "out" });
        } else {
            first = Some(left);
        }
        assert_eq!(texts.iter().any(|t| t.text.starts_with("X:")), hover.is_some());
    }
}

/// The notes the status message lists: in its screen-reader description, and in the tooltip that shows once the
/// pointer has rested on the message past the tooltip delay. The message is the text that starts with `prefix`.
fn listed_on_hover(bar: &mut Shown, prefix: &str) -> (Option<String>, Vec<String>) {
    let texts = status_readout_frame(bar);
    let message = status_text(&texts, prefix);
    let (text, at) = (message.text.clone(), message.bounds.center());
    let described = bar.find(&text).and_then(|t| bar.tree().description_of(t.node).map(str::to_owned));
    bar.frame(vec![egui::Event::PointerMoved(at)]);
    bar.advance(2.0).frame(Vec::new()).frame(Vec::new());
    let shown = bar.texts().into_iter().filter(|t| t.text.contains("• ")).map(|t| t.text).collect();
    bar.frame(vec![egui::Event::PointerGone]).frame(Vec::new());
    (described, shown)
}

/// The status message of an open with several notes lists them on hover, and screen readers receive them after the
/// message, while the status bar shows that message. A later message leaves the list out, and so does an open with
/// one note.
#[test]
fn the_message_of_an_open_with_several_notes_lists_them_on_hover() {
    let mut bar = status_readout_app();
    let notes = vec!["the file's layers weren't read".to_string(), "a brush wasn't read".to_string()];
    {
        let mut app = bar.app();
        app.ui.status = crate::io::opened_with_notes(2, &notes[0]);
        let status = app.ui.status.clone();
        app.ui.status_notes = Some((status, notes));
    }
    let listed = "• the file's layers weren't read\n• a brush wasn't read";
    let (described, shown) = listed_on_hover(&mut bar, "Opened");
    assert!(described.as_deref().is_some_and(|d| d.contains(listed)), "screen readers receive the notes: {described:?}");
    assert!(shown.iter().any(|t| t.contains(listed)), "the tooltip lists the notes: {shown:?}");

    bar.app().ui.status = "Saved recovery data for Untitled-1".to_owned();
    let (described, shown) = listed_on_hover(&mut bar, "Saved");
    assert!(!described.as_deref().is_some_and(|d| d.contains("• ")) && shown.is_empty(), "a later message lists nothing: {described:?} {shown:?}");

    // One note is the message itself: no list.
    {
        let mut app = bar.app();
        app.ui.status = crate::io::opened_with_notes(1, "a brush wasn't read");
        let status = app.ui.status.clone();
        app.ui.status_notes = Some((status, vec!["a brush wasn't read".to_string()]));
    }
    let (described, shown) = listed_on_hover(&mut bar, "Opened");
    assert!(!described.as_deref().is_some_and(|d| d.contains("• ")) && shown.is_empty(), "one note lists nothing: {described:?} {shown:?}");
}

/// The status message of a failed script run shows text the script threw as it is, even when it
/// reads like one of the app's own errors, and translates the words around it; the app's own error
/// is translated. Both a run that ends in one call and a run the frames take to its end show it.
#[test]
fn german_script_status_preserves_authored_error_and_translates_native_error() {
    crate::i18n::set_current(crate::i18n::Lang::from_code("de").expect("de registered"));
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    let failed = |app: &mut crate::VectorcraftApp, source: &str, frames: bool| {
        app.session.script_frames = frames;
        let r = app.run("script.run", json!({"source": source, "name": "t.js"}));
        if frames {
            assert_eq!(r.map(|v| v["running"].clone()), Ok(json!(true)), "{source}");
            for _ in 0..1_000 {
                if !app.session.script_running() {
                    break;
                }
                crate::script_runs::poll(app);
            }
        } else {
            assert!(r.is_err(), "{source}");
        }
        assert!(!app.session.script_running());
        super::status_bar::status_message(app)
    };
    let long = "invalid array length ".repeat(120);
    for frames in [false, true] {
        let shown = failed(&mut app, "throw new Error('invalid array length')", frames);
        assert_eq!(shown, "t.js: Zeile 1, Spalte 1: Error: invalid array length (ausgelöst)");
        let shown = failed(&mut app, "throw 'invalid array length'", frames);
        assert_eq!(shown, "t.js: Zeile 1, Spalte 1: invalid array length (ausgelöst)");
        let shown = failed(&mut app, "new Array(-1)", frames);
        assert!(shown.starts_with("t.js: Zeile 1, Spalte 1: ") && shown.contains("ungültige Array-Länge"), "{shown}");
        for text in ["", "invalid array length\nsecond line", "{name}: {e}", "t.js: line 1, column 1: invalid array length (thrown)", long.as_str()] {
            let source = format!("throw {}", serde_json::to_string(text).unwrap());
            assert_eq!(failed(&mut app, &source, frames), format!("t.js: Zeile 1, Spalte 1: {text} (ausgelöst)"), "{text}");
        }
    }
    // Another message, even one spelled like the script's text, is translated as before.
    failed(&mut app, "throw 'invalid array length'", false);
    app.ui.status = "invalid array length".to_string();
    assert_eq!(super::status_bar::status_message(&app), "ungültige Array-Länge");
    // A change of language while the message stays keeps the script's text.
    failed(&mut app, "throw new Error('invalid array length')", false);
    crate::i18n::set_current(crate::i18n::Lang::from_code("fr").expect("fr registered"));
    assert_eq!(super::status_bar::status_message(&app), "t.js\u{a0}: ligne 1, colonne 1\u{a0}: Error: invalid array length (levée)");
    crate::i18n::set_current(crate::i18n::Lang::EN);
    assert_eq!(super::status_bar::status_message(&app), app.ui.status);
    // After a run that completes, the message is an ordinary one.
    app.session.script_frames = false;
    app.run("script.run", json!({"source": "1", "name": "t.js"})).unwrap();
    assert_eq!(app.session.script_authored_failure(), None);
}

/// The message of an open that starts with a line about its linked files lists the open's notes on
/// hover and for screen readers, in a wide and in a narrow window. A later message leaves the list
/// out.
#[test]
fn combined_open_and_link_status_lists_notes_on_hover_and_for_accessibility() {
    let mut bar = status_readout_app();
    let notes = vec!["the file's layers weren't read".to_string(), "a brush wasn't read".to_string()];
    let notice = crate::dialogs::missing_links::changed_on_disk_status(2);
    let opened = crate::io::opened_with_notes(2, &notes[0]);
    crate::io::show_opened(&mut bar.app(), Some(notice.clone()), notes);
    let status = bar.app().ui.status.clone();
    assert!(status.starts_with(&notice) && status.ends_with(&opened), "{status}");
    let listed = "• the file's layers weren't read\n• a brush wasn't read";
    for width in [1400.0, 700.0] {
        bar.room(egui::vec2(width, 300.0));
        let (described, shown) = listed_on_hover(&mut bar, "2 linked");
        assert!(described.as_deref().is_some_and(|d| d.contains(listed)), "screen readers receive the notes at {width}: {described:?}");
        assert!(shown.iter().any(|t| t.contains(listed)), "the tooltip lists the notes at {width}: {shown:?}");
    }

    bar.room(egui::vec2(1400.0, 300.0));
    bar.app().ui.status = crate::dialogs::missing_links::changed_on_disk_status(1);
    let (described, shown) = listed_on_hover(&mut bar, "1 linked");
    assert!(!described.as_deref().is_some_and(|d| d.contains("• ")) && shown.is_empty(), "a later message lists nothing: {described:?} {shown:?}");
}

/// The status bar's info field shows the readout `view.statusBarShow` chooses: the navigator's
/// artboard name, the tool in use, the date and time, the undo and redo counts, or the document's
/// color profile.
#[test]
fn the_status_bar_info_field_shows_the_chosen_readout() {
    let mut bar = status_readout_app();
    let texts = status_readout_frame(&mut bar);
    assert!(texts.iter().any(|t| t.text == "Selection"), "Current Tool by default: {texts:?}");

    let first = {
        let mut app = bar.app();
        app.run("artboard.new", json!({"x": 200, "y": 0, "width": 100, "height": 100, "name": "Back Cover"})).unwrap();
        app.run("view.statusBarShow", json!({"show": "artboardName"})).unwrap();
        app.session.active().unwrap().doc.artboards[0].name.clone()
    };
    assert!(status_readout_frame(&mut bar).iter().any(|t| t.text == first), "the first artboard's name");
    bar.app().run("view.goToArtboard", json!({"index": 1})).unwrap();
    assert!(status_readout_frame(&mut bar).iter().any(|t| t.text == "Back Cover"), "the navigator's artboard");

    bar.app().run("view.statusBarShow", json!({"show": "undos"})).unwrap();
    let counts = |app: &crate::VectorcraftApp| {
        let h = &app.session.active().unwrap().history;
        format!("Undos: {}  Redos: {}", h.undo.len(), h.redo.len())
    };
    let before = bar.app().session.active().unwrap().history.undo.len();
    bar.app().session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    assert_eq!(counts(&bar.app()), format!("Undos: {}  Redos: 0", before + 1));
    let shown = counts(&bar.app());
    assert!(status_readout_frame(&mut bar).iter().any(|t| t.text == shown));
    bar.app().run("edit.undo", json!({})).unwrap();
    let shown = counts(&bar.app());
    assert_eq!(shown, format!("Undos: {before}  Redos: 1"));
    assert!(status_readout_frame(&mut bar).iter().any(|t| t.text == shown), "the redo step counts");
    // The labels take the UI language's colon (#7991).
    let fr = crate::i18n::Lang::from_code("fr").unwrap();
    crate::i18n::set_current(fr);
    let (french, _) = super::status_bar::info_text(&bar.app());
    crate::i18n::set_current(crate::i18n::Lang::EN);
    assert_eq!(french, format!("{}\u{a0}: {before}  {}\u{a0}: 1", crate::i18n::tr(fr, "Undos"), crate::i18n::tr(fr, "Redos")));

    bar.app().run("view.statusBarShow", json!({"show": "colorProfile"})).unwrap();
    let working = vectorcraft_color::cms::working_settings();
    let mode_working = |app: &crate::VectorcraftApp| match app.session.active().unwrap().doc.color_mode {
        vectorcraft_doc::ColorMode::Rgb => working.rgb.clone(),
        vectorcraft_doc::ColorMode::Cmyk => working.cmyk.clone(),
    };
    let untagged = vectorcraft_color::cms::canonical_name(&mode_working(&bar.app())).to_string();
    assert_eq!(super::color_profile_name(&bar.app().session.active().unwrap().doc), untagged, "untagged: the working space");
    assert!(status_readout_frame(&mut bar).iter().any(|t| t.text == untagged));
    // A profile assigned for the document's color mode shows; one that is not available shows the
    // working space the document draws through.
    let (key, assigned) = match bar.app().session.active().unwrap().doc.color_mode {
        vectorcraft_doc::ColorMode::Rgb => ("rgb", vectorcraft_color::cms::PROPHOTO_RGB),
        vectorcraft_doc::ColorMode::Cmyk => ("cmyk", vectorcraft_color::cms::DEVICE_CMYK),
    };
    bar.app().run("edit.assignProfile", json!({ key: assigned })).unwrap();
    assert!(status_readout_frame(&mut bar).iter().any(|t| t.text == assigned), "the assigned profile");
    {
        let mut app = bar.app();
        let st = app.session.active_mut().unwrap();
        std::sync::Arc::make_mut(&mut st.doc).color_profiles =
            vectorcraft_doc::ColorProfiles { rgb: Some("Not Installed".into()), cmyk: Some("Not Installed".into()) };
    }
    assert!(status_readout_frame(&mut bar).iter().any(|t| t.text == untagged), "an unavailable profile: the working space");

    bar.app().run("view.statusBarShow", json!({"show": "dateTime"})).unwrap();
    let texts = status_readout_frame(&mut bar);
    let is_date = |s: &str| {
        let b = s.as_bytes();
        b.len() == 16
            && b.iter().enumerate().all(|(i, c)| match i {
                4 | 7 => *c == b'-',
                10 => *c == b' ',
                13 => *c == b':',
                _ => c.is_ascii_digit(),
            })
    };
    assert!(texts.iter().any(|t| is_date(&t.text)), "a date and time: {texts:?}");
    assert_eq!(super::date_time_label([2026, 3, 7, 9, 5, 59]), "2026-03-07 09:05");

    // Without a document the field shows a dash, which can still be clicked.
    {
        let mut app = bar.app();
        app.run("view.statusBarShow", json!({"show": "undos"})).unwrap();
        while app.session.active().is_some() {
            app.session.execute("file.close", &json!({})).unwrap();
        }
    }
    assert!(status_readout_frame(&mut bar).iter().any(|t| t.text == "–"));
}

/// `view.statusBarShow` takes only the five readouts, leaving the choice as it was otherwise, and the
/// choice is saved with the UI state, which reads a value it does not know as Current Tool.
#[test]
fn the_status_bar_show_choice_is_checked_and_saved() {
    let bar = status_readout_app();
    let mut app = bar.app();
    assert_eq!(app.run("view.statusBarShow", json!({"show": "dateTime"})).unwrap(), json!("dateTime"));
    for bad in [json!({}), json!({"show": "zoom"}), json!({"show": 2}), json!({"show": null}), json!({"show": "DateTime"})] {
        let err = app.run("view.statusBarShow", bad.clone()).unwrap_err();
        assert!(err.contains("`show` must be one of artboardName, currentTool, dateTime, undos, colorProfile"), "{bad}: {err}");
        assert_eq!(app.ui.status_bar_show, super::StatusBarShow::DateTime, "{bad} leaves the choice");
    }
    let saved = serde_json::to_value(&app.ui).unwrap();
    assert_eq!(saved["status_bar_show"], json!("dateTime"));
    let read: crate::state::UiState = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(read.status_bar_show, super::StatusBarShow::DateTime);
    for (stored, shown) in [
        (json!("colorProfile"), super::StatusBarShow::ColorProfile),
        (json!("zoom"), super::StatusBarShow::CurrentTool),
        (json!(7), super::StatusBarShow::CurrentTool),
    ] {
        let mut v = saved.clone();
        v["status_bar_show"] = stored;
        assert_eq!(serde_json::from_value::<crate::state::UiState>(v).unwrap().status_bar_show, shown);
    }
    let mut v = saved;
    v.as_object_mut().unwrap().remove("status_bar_show");
    assert_eq!(
        serde_json::from_value::<crate::state::UiState>(v).unwrap().status_bar_show,
        super::StatusBarShow::CurrentTool,
        "state saved before it"
    );
}

/// While background work runs the status bar shows it where the message was, with the count of the jobs after the
/// first, and the message of the work's result takes the same place when it ends. Screen readers find the progress.
#[test]
fn background_activity_keeps_the_status_message_origin() {
    let mut bar = status_readout_app();
    let before = status_text(&status_readout_frame(&mut bar), "Saved recovery data").at.x;
    let (tx, rx) = std::sync::mpsc::channel();
    {
        let mut app = bar.app();
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
    }
    let busy = status_readout_frame(&mut bar);
    assert_eq!(status_text(&busy, "Saving test.vectorcraft").at.x, before, "the work shows where the message was");
    crate::background::run(&mut bar.app(), true, "Exporting test.svg".to_owned(), None, |_| Ok(json!({})), |_, result| result).unwrap();
    let several = status_readout_frame(&mut bar);
    let message = status_text(&several, "Saving test.vectorcraft");
    assert!(message.text.ends_with("(+1)"), "{message:?}");
    assert_eq!(message.at.x, before);
    assert!(!bar.fixture.by_role(Role::PROGRESS_INDICATOR).is_empty(), "the busy progress stays accessible: {:?}", bar.names());
    tx.send(()).unwrap();
    crate::background::wait_all(&mut bar.app());
    let after = status_readout_frame(&mut bar);
    assert_eq!(status_text(&after, "Saved recovery data").at.x, before, "the result's message takes the same place");
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
    app.view_mut().unwrap().rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(460.0, 260.0));
    let mut bar = status_shown(app);
    // Where the view is once the canvas has drawn.
    let at = |bar: &mut Shown| {
        bar.canvas_pass();
        let app = bar.app();
        let view = app.view().unwrap();
        (view.artboard, view.center)
    };
    bar.click("Next Artboard");
    assert_eq!(at(&mut bar), (1, Point::new(320.0, 50.0)), "next");
    bar.click("Last Artboard");
    assert_eq!(at(&mut bar), (2, Point::new(540.0, 50.0)), "last");
    bar.click("Previous Artboard");
    assert_eq!(at(&mut bar), (1, Point::new(320.0, 50.0)), "previous");
    // Fit Artboard in Window and Actual Size show the navigator's artboard.
    bar.app().run("view.setZoom", json!({"zoom": 300, "center": [0, 0]})).unwrap();
    bar.app().run("view.fitArtboard", json!({})).unwrap();
    assert_eq!(at(&mut bar), (1, Point::new(320.0, 50.0)));
    bar.app().run("view.actualSize", json!({})).unwrap();
    let center = at(&mut bar).1;
    assert_eq!((center, bar.app().view().unwrap().zoom), (Point::new(320.0, 50.0), 1.0));
    bar.click("First Artboard");
    assert_eq!(at(&mut bar), (0, Point::new(100.0, 50.0)), "first");
    // The command also takes an index, clamped to the last artboard.
    let mut app = bar.app();
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

/// Read the painted coordinates while checking that drawing never changes the source
/// position, document, selection or either branch of its edit history.
fn assert_status_coordinates(bar: &mut Shown, expected: Option<&str>) {
    let read = |app: &crate::VectorcraftApp| {
        let state =
            app.session.active().map(|st| (st.doc.clone(), st.selection.clone(), st.history.undo.len(), st.history.redo.len(), st.is_dirty()));
        (app.hover_doc, state)
    };
    let (hover, before) = read(&bar.app());
    let texts = status_readout_frame(bar);
    let coordinates: Vec<&str> =
        texts.iter().filter_map(|t| (t.text.starts_with("X: ") || t.text.starts_with("Y: ")).then_some(t.text.as_str())).collect();
    let expected: Vec<&str> = expected.into_iter().flat_map(|text| text.split("  ")).collect();
    assert_eq!(coordinates, expected, "{texts:?}");
    let (hover_after, after) = read(&bar.app());
    assert_eq!(hover_after, hover, "tools still receive the absolute document position");
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
    let mut bar = status_shown(app);
    assert_status_coordinates(&mut bar, Some("X: 72.00 pt  Y: 144.00 pt"));
    bar.app().run("document.setRulerOrigin", json!({"x": 36, "y": 72})).unwrap();
    assert_status_coordinates(&mut bar, Some("X: 36.00 pt  Y: 72.00 pt"));
    bar.app().run("document.setRulerOrigin", json!({"x": -72, "y": 216})).unwrap();
    assert_status_coordinates(&mut bar, Some("X: 144.00 pt  Y: -72.00 pt"));
    bar.app().run("document.setUnits", json!({"units": "Inches"})).unwrap();
    assert_status_coordinates(&mut bar, Some("X: 2.00 in  Y: -1.00 in"));
    bar.app().run("document.setUnits", json!({"units": "Millimeters"})).unwrap();
    assert_status_coordinates(&mut bar, Some("X: 50.80 mm  Y: -25.40 mm"));
    bar.app().run("document.resetRulerOrigin", json!({})).unwrap();
    assert_status_coordinates(&mut bar, Some("X: 25.40 mm  Y: 50.80 mm"));
    // Keep a nonempty Redo branch as well as Undo while painting.
    bar.app().run("edit.undo", json!({})).unwrap();
    assert_status_coordinates(&mut bar, Some("X: 50.80 mm  Y: -25.40 mm"));
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
    let mut bar = status_shown(app);
    assert_status_coordinates(&mut bar, Some("X: 735.00 pt  Y: -240.00 pt"));
    bar.app().run("view.goToArtboard", json!({"index": 1})).unwrap();
    assert_status_coordinates(&mut bar, Some("X: 60.00 pt  Y: 40.00 pt"));
    bar.app().run("document.setRulerOrigin", json!({"artboard": 1, "x": 800, "y": -190})).unwrap();
    assert_status_coordinates(&mut bar, Some("X: -40.00 pt  Y: -20.00 pt"));
    bar.app().run("view.globalRulers", json!({"global": true})).unwrap();
    assert_status_coordinates(&mut bar, Some("X: 760.00 pt  Y: -210.00 pt"));
    bar.app().run("document.setRulerOrigin", json!({"x": -20, "y": -10})).unwrap();
    assert_status_coordinates(&mut bar, Some("X: 780.00 pt  Y: -200.00 pt"));
    bar.app().run("view.globalRulers", json!({"global": false})).unwrap();
    assert_status_coordinates(&mut bar, Some("X: -40.00 pt  Y: -20.00 pt"));
    bar.app().run("document.resetRulerOrigin", json!({"artboard": 1})).unwrap();
    assert_status_coordinates(&mut bar, Some("X: 60.00 pt  Y: 40.00 pt"));
    bar.app().run("view.goToArtboard", json!({"index": 0})).unwrap();
    assert_status_coordinates(&mut bar, Some("X: 735.00 pt  Y: -240.00 pt"));
}

#[test]
fn status_coordinates_handle_missing_pointer_or_document() {
    let mut bar = status_shown(crate::VectorcraftApp::new(Session::new(), Default::default()));
    assert_status_coordinates(&mut bar, None);
    bar.app().hover_doc = Some(vectorcraft_geom::Point::new(42.0, -21.0));
    assert_status_coordinates(&mut bar, Some("X: 42.00 pt  Y: -21.00 pt"));
    {
        let mut app = bar.app();
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.run("document.setRulerOrigin", json!({"x": 10, "y": 20})).unwrap();
        app.hover_doc = None;
    }
    assert_status_coordinates(&mut bar, None);
}

#[test]
fn status_coordinates_keep_the_inspected_point_relative_after_an_origin_change() {
    let mut bar = status_readout_app();
    {
        let mut app = bar.app();
        app.run("view.globalRulers", json!({"global": true})).unwrap();
        app.run("document.setRulerOrigin", json!({"x": 36, "y": 72})).unwrap();
        app.hover_doc = Some(vectorcraft_geom::Point::new(72.0, 144.0));
    }
    // PLAN: the readouts keep no hovered value once the pointer leaves the canvas (the retention memory is gone, :777's verdict), so the point stays inspected through `hover_doc`.
    let inspecting = status_readout_frame(&mut bar);
    assert_eq!(status_text(&inspecting, "X:").text, "X: 36.00 pt");
    assert_eq!(status_text(&inspecting, "Y:").text, "Y: 72.00 pt");
    bar.app().run("document.setRulerOrigin", json!({"x": -72, "y": 216})).unwrap();
    let before = bar.app().session.active().unwrap().doc.clone();
    let inspecting = status_readout_frame(&mut bar);
    assert_eq!(status_text(&inspecting, "X:").text, "X: 144.00 pt");
    assert_eq!(status_text(&inspecting, "Y:").text, "Y: -72.00 pt");
    assert_eq!(bar.app().hover_doc, Some(vectorcraft_geom::Point::new(72.0, 144.0)), "the readouts never change the absolute canvas hover state");
    assert_eq!(bar.app().session.active().unwrap().doc, before, "painting is read-only");
    bar.app().hover_doc = None;
    let gone = status_readout_frame(&mut bar);
    assert!(!gone.iter().any(|field| field.text.starts_with("X:") || field.text.starts_with("Y:")));
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
    app.view_mut().unwrap().rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(460.0, 260.0));
    app.select_tool("artboard");
    app.run("view.goToArtboard", json!({"index": 1})).unwrap();
    let first = app.session.active().unwrap().doc.artboards[0].rect.center();
    let mut bar = status_shown(app);
    bar.room(egui::vec2(1200.0, 600.0));
    let entries = ["1  Front", "2  Back", "3  Artboard 3"];
    bar.settle();
    assert!(bar.shows("2  Back"), "the field shows artboard 2's number and name: {:?}", bar.names());
    assert!(bar.find("1  Front").is_none(), "the list is closed: {:?}", bar.names());
    bar.click("Artboard");
    let rows: Vec<Target> = entries.iter().map(|e| bar.find(e).unwrap_or_else(|| panic!("the list shows {e:?}: {:?}", bar.names()))).collect();
    assert!(rows.windows(2).all(|w| w[0].rect.bottom() <= w[1].rect.top()), "the rows in the artboards' order: {rows:?}");
    bar.click_on(&rows[0]);
    bar.canvas_pass();
    {
        let app = bar.app();
        let view = app.view().unwrap();
        assert_eq!((view.artboard, view.center), (0, first), "Front, fitted in the window");
        assert_eq!(app.session.tool_options()["active"], json!(0));
    }
    bar.settle();
    assert!(bar.shows("1  Front"), "{:?}", bar.names());
    assert!(bar.find("2  Back").is_none(), "the list closed: {:?}", bar.names());
}

/// Where the artboard flows keep the canvas: below the strip the tabs and the status bar take at the top of their
/// fixture, and left of Properties, so no click of theirs lands on it.
const ARTBOARD_CANVAS: egui::Rect = egui::Rect { min: egui::pos2(0.0, 60.0), max: egui::pos2(800.0, 300.0) };

/// The room the artboard flows draw the status bar, the tabs and Properties in.
const ARTBOARD_ROOM: egui::Vec2 = egui::vec2(1200.0, 700.0);

/// Properties docked at the right, as the window shows it.
fn properties(app: &mut crate::VectorcraftApp, ui: &mut egui::Ui) {
    egui::Panel::right("navigation-properties").exact_size(300.0).show(ui, |ui| crate::panels::properties::show(app, ui));
}

fn artboard_navigation_app(tool: &str) -> (Shown, vectorcraft_doc::NodeId) {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.view_mut().unwrap().rect = ARTBOARD_CANVAS;
    app.select_tool(tool);
    crate::i18n::set_current(crate::i18n::Lang::EN);
    (Shown::new(app, ARTBOARD_ROOM, super::status_bar), vectorcraft_doc::NodeId(id))
}

/// Click the actual status-bar Next control, without sending a canvas pointer event.
fn artboard_navigation_next(bar: &mut Shown) {
    bar.draw(super::status_bar);
    let next = bar.named("Next Artboard");
    assert!(!bar.app().view().unwrap().rect.contains(next.center()));
    bar.click_on(&next);
    bar.canvas_pass();
}

/// Properties' Delete Artboard button, once Properties shows the board named `shown`.
fn properties_delete(bar: &mut Shown, shown: &str) -> Target {
    bar.draw(properties);
    bar.settle();
    assert!(bar.shows(shown), "Properties shows the board {shown:?}: {:?}", bar.names());
    let delete = bar.named("Delete Artboard");
    assert!(!bar.app().view().unwrap().rect.contains(delete.center()));
    delete
}

/// A press and release of `key` through the window's whole frame, as the keyboard sends it. The window's first frame
/// builds its keymap and the keys the tool claims, as a running window has them before any key.
fn key_through_the_app(app: &mut crate::VectorcraftApp, key: egui::Key, modifiers: egui::Modifiers) {
    let ctx = crate::testing::ui_context();
    crate::tests_synthetic::key_frame(app, &ctx, vec![]);
    let event = |pressed| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers };
    crate::tests_synthetic::key_frame(app, &ctx, vec![event(true)]);
    crate::tests_synthetic::key_frame(app, &ctx, vec![event(false)]);
}

/// Delete the shown artboard with `key`, or with Properties' Delete Artboard button.
fn trigger_artboard_delete(bar: &mut Shown, key: Option<egui::Key>, before: &vectorcraft_doc::Document) {
    if let Some(key) = key {
        key_through_the_app(&mut bar.app(), key, egui::Modifiers::NONE);
    } else {
        let delete = properties_delete(bar, &before.artboards[1].name);
        bar.click_on(&delete);
    }
}

/// None clicks Properties' Delete Artboard; Some routes the real key through shortcuts.
fn artboard_navigation_delete(key: Option<egui::Key>) {
    for multiple in [false, true] {
        artboard_navigation_delete_case(key, multiple);
    }
}

fn artboard_navigation_delete_case(key: Option<egui::Key>, multiple: bool) {
    let (mut bar, id) = artboard_navigation_app("artboard");
    let (before, selection, undo) = {
        let mut app = bar.app();
        let selected = if multiple { json!([0, 2]) } else { json!([0]) };
        app.session.set_tool_option("selected", &selected);
        assert_eq!(app.view().unwrap().artboard, 0);
        assert_eq!(app.session.tool_options()["active"], json!(0));
        assert_eq!(app.session.tool_options()["selected"], selected);
        app.session.active_mut().unwrap().mark_saved();
        let st = app.session.active().unwrap();
        (st.doc.clone(), st.selection.clone(), st.history.undo.len())
    };
    navigate_to_next_board(&mut bar, &before, &selection, undo);
    delete_navigated_board(&mut bar, key, id, &before, &selection, undo);
}

fn navigate_to_next_board(bar: &mut Shown, before: &vectorcraft_doc::Document, selection: &vectorcraft_doc::Selection, undo: usize) {
    artboard_navigation_next(bar);
    let app = bar.app();
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
    bar: &mut Shown,
    key: Option<egui::Key>,
    id: vectorcraft_doc::NodeId,
    before: &vectorcraft_doc::Document,
    selection: &vectorcraft_doc::Selection,
    undo: usize,
) {
    trigger_artboard_delete(bar, key, before);
    let mut app = bar.app();
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
        let (mut bar, _) = artboard_navigation_app(tool);
        let (redo, before, selection, history, options) = {
            let mut app = bar.app();
            app.run("object.move", json!({"dx": 7, "dy": 9})).unwrap();
            let redo = app.session.active().unwrap().doc.clone();
            app.run("edit.undo", json!({})).unwrap();
            app.session.active_mut().unwrap().mark_saved();
            let st = app.session.active().unwrap();
            let before = st.doc.clone();
            let selection = st.selection.clone();
            let history = (st.history.undo.len(), st.history.redo.len());
            assert_eq!(history.1, 1);
            (redo, before, selection, history, app.session.tool_options())
        };

        artboard_navigation_next(&mut bar);

        let mut app = bar.app();
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

/// The Control bar shown alone over `app`, in a window 1600 points wide.
fn control_shown(app: crate::VectorcraftApp) -> Shown {
    Shown::new(app, egui::vec2(1600.0, 300.0), super::control_bar)
}

/// The Control bar's isolation button, if it shows: Isolate Selected Object, Exit Isolation Mode or Exit Symbol
/// Editing Mode.
fn isolation_button(bar: &mut Shown) -> Option<Target> {
    bar.settle();
    ["Isolate Selected Object", "Exit Isolation Mode", "Exit Symbol Editing Mode"].iter().find_map(|name| bar.find(name))
}

/// Entering isolation mode leaves the canvas where it was: the breadcrumb bar is drawn over the
/// canvas and takes no layout height from it (#3126).
#[test]
fn the_isolation_bar_does_not_take_layout_height_from_the_canvas() {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap()["id"].as_u64().unwrap();
    // Where the canvas starts below the tabs, as the frame laid it out.
    let below = Rc::new(std::cell::Cell::new(0.0));
    let seen = Rc::clone(&below);
    let mut bar = Shown::new(app, egui::vec2(1200.0, 700.0), move |app, ui| {
        super::doc_tabs(app, ui);
        let area = ui.available_rect_before_wrap();
        seen.set(area.top());
        super::isolation_bar(app, ui, area);
        assert_eq!(ui.available_rect_before_wrap().top(), area.top(), "the bar takes no layout space");
    });
    bar.settle();
    let before = below.get();
    bar.app().run("object.isolate", json!({"id": id})).unwrap();
    assert!(bar.app().session.active().unwrap().isolation.is_some());
    bar.settle();
    assert_eq!(below.get(), before, "isolation mode leaves the canvas where it was");
}

/// The isolation bar shown over the top of a 1200 × 700 canvas.
fn isolation_shown(app: crate::VectorcraftApp) -> Shown {
    Shown::new(app, egui::vec2(1200.0, 700.0), |app, ui| {
        let area = ui.max_rect();
        super::isolation_bar(app, ui, area);
    })
}

/// A point on the isolation bar's own surface: right of its crumbs, at the end of the path, which fills the bar up
/// to the pattern editing buttons at its right.
fn isolation_bar_surface(bar: &mut Shown) -> egui::Pos2 {
    let back = bar.named("Back");
    let path = bar.named("Isolation path");
    egui::pos2(path.rect.right() - 8.0, back.center().y)
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
    let mut bar = isolation_shown(app);
    let isolation = |bar: &Shown| bar.app().session.active().unwrap().isolation;

    // Clicking back from `a` steps up to parent group `g1`
    bar.click("Back");
    assert_eq!(isolation(&bar), Some(NodeId(g1)));

    // Clicking back from `g1` steps up to grandparent group `g2`
    bar.click("Back");
    assert_eq!(isolation(&bar), Some(NodeId(g2)));

    // Clicking back from `g2` (parent is layer) exits isolation mode completely
    bar.click("Back");
    assert_eq!(isolation(&bar), None);

    // Deep isolate again to test clicking ancestor breadcrumb directly
    bar.app().run("object.isolate", json!({"id": a})).unwrap();
    assert_eq!(isolation(&bar), Some(NodeId(a)));

    // The first crumb is Layer 1. Clicking it exits isolation directly.
    bar.click("Layer 1");
    assert_eq!(isolation(&bar), None);

    // Deep isolate again to test clicking an intermediate ancestor group breadcrumb directly
    bar.app().run("object.isolate", json!({"id": a})).unwrap();
    assert_eq!(isolation(&bar), Some(NodeId(a)));

    // The second crumb is g2, the first "<Group>". Clicking it isolates g2 directly.
    bar.click("<Group>");
    assert_eq!(isolation(&bar), Some(NodeId(g2)));
}

/// With a sublayer isolated, the back arrow goes up one layer at a time and then leaves isolation
/// mode. The top-level layer's crumb leaves isolation mode from any depth, and a parent sublayer's
/// crumb isolates that sublayer.
#[test]
fn isolation_bar_back_button_steps_up_through_sublayers() {
    use vectorcraft_doc::NodeId;
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    let top = app.session.active().unwrap().doc.layers[0].id;
    let mid = NodeId(app.run("layer.newSublayer", json!({"parent": top.0, "name": "Sublayer B"})).unwrap()["id"].as_u64().unwrap());
    let deep = NodeId(app.run("layer.newSublayer", json!({"parent": mid.0, "name": "Sublayer C"})).unwrap()["id"].as_u64().unwrap());
    app.run("layer.setCurrent", json!({"id": deep.0})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
    app.run("object.isolate", json!({"id": deep.0})).unwrap();
    let mut bar = isolation_shown(app);
    let isolation = |bar: &Shown| bar.app().session.active().unwrap().isolation;
    let steps = |bar: &Shown| bar.app().session.doc().unwrap().history.undo.len();

    let before = steps(&bar);
    bar.click("Back");
    assert_eq!(isolation(&bar), Some(mid), "back from the deepest sublayer");
    bar.click("Back");
    assert_eq!(isolation(&bar), Some(top), "back from the middle sublayer");
    bar.click("Back");
    assert_eq!(isolation(&bar), None, "back from the top-level layer");
    assert_eq!(steps(&bar), before, "no undo step");

    // "Layer 1" is the first crumb.
    bar.app().run("object.isolate", json!({"id": deep.0})).unwrap();
    bar.click("Layer 1");
    assert_eq!(isolation(&bar), None, "the top-level layer's crumb");

    // "Sublayer B" is the second crumb.
    bar.app().run("object.isolate", json!({"id": deep.0})).unwrap();
    bar.click("Sublayer B");
    assert_eq!(isolation(&bar), Some(mid), "the parent sublayer's crumb");
}

/// A click on the isolation bar's empty space leaves nested isolation mode altogether, selects the
/// isolated object as Exit Isolation Mode does and adds no undo step. A drag across the bar does not
/// leave it.
#[test]
fn isolation_bar_background_click_exits_nested_isolation() {
    use vectorcraft_doc::NodeId;
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    let rect = |app: &mut crate::VectorcraftApp, x: f64| {
        app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 30, "height": 20})).unwrap()["id"].as_u64().unwrap()
    };
    let (a, b) = (rect(&mut app, 10.0), rect(&mut app, 50.0));
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let g1 = app.run("object.group", json!({})).unwrap()["id"].as_u64().unwrap();
    let c = rect(&mut app, 90.0);
    app.run("select.set", json!({"ids": [g1, c]})).unwrap();
    app.run("object.group", json!({})).unwrap();
    app.run("object.isolate", json!({"id": a})).unwrap();
    let mut bar = isolation_shown(app);
    let empty = isolation_bar_surface(&mut bar);
    bar.drag_at(empty, egui::vec2(-200.0, 2.0));
    assert_eq!(bar.app().session.active().unwrap().isolation, Some(NodeId(a)), "a drag across the bar stays in isolation mode");
    let steps = bar.app().session.doc().unwrap().history.undo.len();
    bar.click_at(empty);
    let app = bar.app();
    let st = app.session.active().unwrap();
    assert_eq!((st.isolation, st.selection.objects.clone()), (None, vec![NodeId(a)]), "the click leaves isolation mode");
    assert_eq!(app.session.doc().unwrap().history.undo.len(), steps, "no undo step");
}

/// While a pattern is edited, a click on the isolation bar's empty space leaves pattern editing
/// open; the back arrow still ends it as Done does.
#[test]
fn isolation_bar_background_click_leaves_pattern_edit_open() {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    let f = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [f]})).unwrap();
    app.run("object.pattern.make", json!({"name": "Dots", "width": 20, "height": 20})).unwrap();
    assert!(app.session.active().unwrap().doc.pattern_edit.is_some());
    let mut bar = isolation_shown(app);
    let empty = isolation_bar_surface(&mut bar);
    bar.click_at(empty);
    {
        let app = bar.app();
        let st = app.session.active().unwrap();
        assert!(st.doc.pattern_edit.is_some() && st.isolation.is_some(), "pattern editing stays open");
    }
    bar.click("Back");
    let app = bar.app();
    let st = app.session.active().unwrap();
    assert!(st.doc.pattern_edit.is_none() && st.isolation.is_none(), "the back arrow ends pattern editing");
}

/// While an opacity mask is edited, a click on the isolation bar's empty space stops editing the
/// mask as Exit Isolation Mode does: the masked object is selected again.
#[test]
fn isolation_bar_background_click_stops_mask_editing() {
    use vectorcraft_doc::NodeId;
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    let obj = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 200, "height": 200})).unwrap()["id"].as_u64().unwrap();
    let m = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 200})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [obj, m]})).unwrap();
    app.run("transparency.makeOpacityMask", json!({})).unwrap();
    app.run("transparency.editOpacityMask", json!({})).unwrap();
    assert!(app.session.active().unwrap().doc.mask_edit.is_some());
    let mut bar = isolation_shown(app);
    let empty = isolation_bar_surface(&mut bar);
    bar.click_at(empty);
    let app = bar.app();
    let st = app.session.active().unwrap();
    assert!(st.doc.mask_edit.is_none() && st.isolation.is_none(), "mask editing ends");
    assert_eq!(st.selection.objects, vec![NodeId(obj)]);
}

/// In symbol editing mode, a click on the isolation bar's empty space leaves the mode, also while a
/// group inside the symbol's art is isolated, and the symbol keeps the art drawn while editing.
#[test]
fn isolation_bar_background_click_leaves_nested_symbol_editing() {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
    app.run("symbol.new", json!({"name": "Box"})).unwrap();
    app.run("symbol.edit", json!({"name": "Box"})).unwrap();
    let a = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    let b = app.run("shape.rectangle", json!({"x": 130, "y": 100, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let g = app.run("object.group", json!({})).unwrap()["id"].as_u64().unwrap();
    app.run("object.isolate", json!({"id": g})).unwrap();
    let mut bar = isolation_shown(app);
    let empty = isolation_bar_surface(&mut bar);
    bar.click_at(empty);
    {
        let app = bar.app();
        let st = app.session.active().unwrap();
        assert!(st.doc.symbol_edit.is_none() && st.isolation.is_none(), "symbol editing mode ends");
        let symbol = st.doc.symbols.iter().find(|s| s.name == "Box").expect("the symbol");
        assert!(symbol.art.children().is_some_and(|c| c.len() >= 2), "the edit reached the symbol");
    }
    // Symbol editing entered while a group was isolated: the click leaves that isolation too.
    {
        let mut app = bar.app();
        let c = app.run("shape.rectangle", json!({"x": 200, "y": 150, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        let d = app.run("shape.rectangle", json!({"x": 250, "y": 150, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [c, d]})).unwrap();
        let h = app.run("object.group", json!({})).unwrap()["id"].as_u64().unwrap();
        app.run("object.isolate", json!({"id": h})).unwrap();
        app.run("symbol.edit", json!({"name": "Box"})).unwrap();
        assert!(app.session.active().unwrap().doc.symbol_edit.is_some());
    }
    bar.click_at(empty);
    let app = bar.app();
    let st = app.session.active().unwrap();
    assert!(st.doc.symbol_edit.is_none() && st.isolation.is_none(), "the isolation from before symbol editing ends too");
}

/// A click on the current object's name at the end of the breadcrumbs, which is not a link, leaves
/// isolation mode as a click on the bar's empty space does.
#[test]
fn isolation_bar_click_on_the_current_name_exits() {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    let a = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("object.isolate", json!({"id": a})).unwrap();
    // The crumbs are "Layer 1 › <Path>"; the leaf is the isolated path's name.
    let leaf = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(a)).unwrap().display_name();
    let mut bar = isolation_shown(app);
    bar.click(&leaf);
    assert_eq!(bar.app().session.active().unwrap().isolation, None);
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
    let mut bar = control_shown(app);
    let click = |bar: &mut Shown| {
        let button = isolation_button(bar).expect("the bar shows the isolation button");
        bar.click_on(&button);
    };
    click(&mut bar);
    assert_eq!(bar.app().session.active().unwrap().isolation, Some(NodeId(a)), "Isolate Selected Object isolates the path");
    click(&mut bar);
    {
        let app = bar.app();
        let st = app.session.active().unwrap();
        assert_eq!((st.isolation, st.selection.objects.clone()), (None, vec![NodeId(a)]), "Exit Isolation Mode selects the path");
    }
    bar.app().select_tool("selection");
    // Type, a symbol instance, a blend and two objects.
    let (text, symbol, blend) = {
        let mut app = bar.app();
        let text = app.run("text.create", json!({"x": 10, "y": 100, "text": "Hi"})).unwrap()["id"].as_u64().unwrap();
        let c = rect(&mut app, 100.0);
        app.run("select.set", json!({"ids": [c]})).unwrap();
        let symbol = app.run("symbol.new", json!({"name": "Box"})).unwrap()["id"].as_u64().unwrap();
        let (d, e) = (rect(&mut app, 150.0), rect(&mut app, 250.0));
        let blend = app.run("object.blend.make", json!({"ids": [d, e], "steps": 1})).unwrap()["id"].as_u64().unwrap();
        (text, symbol, blend)
    };
    for ids in [json!([text]), json!([symbol]), json!([blend]), json!([text, symbol])] {
        bar.app().run("select.set", json!({ "ids": ids })).unwrap();
        assert!(isolation_button(&mut bar).is_none(), "{ids}");
    }
    // An envelope's contents, while Edit Contents is on.
    {
        let mut app = bar.app();
        let g = rect(&mut app, 350.0);
        app.run("select.set", json!({"ids": [g]})).unwrap();
        app.run("object.envelope.makeWithWarp", json!({})).unwrap();
        app.run("object.envelope.editContents", json!({"editing": true})).unwrap();
        assert_eq!(app.session.active().unwrap().selection.objects, vec![NodeId(g)]);
    }
    assert!(isolation_button(&mut bar).is_none(), "the envelope's controls show instead");
    {
        let mut app = bar.app();
        let f = rect(&mut app, 300.0);
        app.run("select.set", json!({"ids": [f]})).unwrap();
        app.run("object.pattern.make", json!({"name": "Dots", "width": 20, "height": 20})).unwrap();
        assert!(app.session.active().unwrap().isolation.is_some());
    }
    assert!(isolation_button(&mut bar).is_none(), "the isolation bar ends pattern editing");
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
    let mut bar = control_shown(app);
    // The isolation button's label (the one exit label the bar shows), and a click on it.
    let label = |bar: &mut Shown| {
        bar.settle();
        let exits: Vec<&str> = ["Exit Symbol Editing Mode", "Exit Isolation Mode"].into_iter().filter(|l| bar.find(l).is_some()).collect();
        assert!(exits.len() <= 1, "{exits:?}");
        exits.into_iter().next()
    };
    let click = |bar: &mut Shown| {
        let button = isolation_button(bar).expect("the bar shows the isolation button");
        bar.click_on(&button);
    };
    assert_eq!(label(&mut bar), Some("Exit Symbol Editing Mode"));
    // Two rectangles drawn into the symbol's art, grouped and isolated.
    {
        let mut app = bar.app();
        let a = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        let b = app.run("shape.rectangle", json!({"x": 130, "y": 100, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        let g = app.run("object.group", json!({})).unwrap()["id"].as_u64().unwrap();
        app.run("object.isolate", json!({"id": g})).unwrap();
    }
    assert_eq!(label(&mut bar), Some("Exit Isolation Mode"));
    click(&mut bar);
    {
        let app = bar.app();
        let st = app.session.active().unwrap();
        assert_eq!((st.isolation, st.doc.symbol_edit.is_some()), (Some(layer), true), "back to the symbol's art");
    }
    assert_eq!(label(&mut bar), Some("Exit Symbol Editing Mode"));
    click(&mut bar);
    let app = bar.app();
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
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.select_tool("zoom");
    let mut bar = Shown::new(app, egui::vec2(1600.0, 300.0), super::hint_bar);
    let hint = |bar: &mut Shown| bar.settle().texts().into_iter().map(|t| t.text).collect::<String>();
    let on = hint(&mut bar);
    assert!(on.contains(" right or left to zoom in or out") && on.contains("Hold"), "{on}");
    bar.app().run("prefs.set", json!({"key": "animatedZoom", "value": false})).unwrap();
    let off = hint(&mut bar);
    assert!(off.contains(" to zoom into an area") && !off.contains("Hold"), "{off}");
}

/// With no document the window's title names the app.
#[test]
fn window_title_with_no_document_is_the_app_name() {
    let app = crate::VectorcraftApp::new(Session::new(), Default::default());
    let title = super::window_title(&app);
    assert!(title.contains("VectorCraft"), "{title}");
}

/// The window's title names the active file and the app, and unsaved changes mark it.
#[test]
fn window_title_follows_the_active_file() {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 100, "height": 100, "name": "foo.svg"})).unwrap();
    let saved = super::window_title(&app);
    assert!(saved.contains("foo.svg") && saved.contains("VectorCraft"), "{saved}");
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    let modified = super::window_title(&app);
    assert!(
        modified != saved && modified.contains("foo.svg") && modified.contains("VectorCraft"),
        "an edit marks the title: {saved} then {modified}"
    );
}

/// The document tabs over `app`, with the same per-frame view synchronization as the app.
fn tabs_with_views(app: &mut crate::VectorcraftApp, ui: &mut egui::Ui) {
    app.sync_views();
    super::doc_tabs(app, ui);
}

/// The document tabs, in the strip's order.
fn artboard_document_tabs(bar: &mut Shown) -> Vec<Target> {
    bar.draw(tabs_with_views);
    bar.settle();
    bar.fixture.by_role(Role::TAB)
}

fn artboard_document_click_tab(bar: &mut Shown, index: usize) {
    let tabs = artboard_document_tabs(bar);
    assert_eq!(tabs.len(), 2, "both real document tabs must be visible");
    assert!(!bar.app().view().unwrap().rect.contains(tabs[index].center()));
    bar.click_on(&tabs[index]);
    assert_eq!(bar.app().session.active_index(), Some(index), "the actual tab click must activate its document");
}

fn artboard_document_app() -> Shown {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100, "artboards": 2})).unwrap();
    app.run("artboard.setProps", json!({"index": 0, "name": "Keep"})).unwrap();
    app.run("artboard.setProps", json!({"index": 1, "name": "Shown"})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
    app.run("symbol.new", json!({"name": "Tab symbol"})).unwrap();
    app.run("shape.rectangle", json!({"x": 230, "y": 10, "width": 30, "height": 20})).unwrap();
    app.run("guide.add", json!({"artboard": 0, "pos": 20})).unwrap();
    app.run("guide.add", json!({"artboard": 1, "pos": 250, "vertical": true})).unwrap();
    // Neither tab nor Properties clicks enter the canvas; a canvas move could hide the bug. Each document's view
    // has the canvas.
    app.view_mut().unwrap().rect = ARTBOARD_CANVAS;
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app.view_mut().unwrap().rect = ARTBOARD_CANVAS;
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut bar = Shown::new(app, ARTBOARD_ROOM, tabs_with_views);
    artboard_document_click_tab(&mut bar, 0);
    bar
}

#[test]
fn artboard_document_tab_roundtrip_properties_delete_uses_shown_board() {
    let mut bar = artboard_document_app();
    {
        let mut app = bar.app();
        app.select_tool("artboard");
        app.run("tool.setOption", json!({"key": "moveArt", "value": false})).unwrap();
        app.run("tool.setOption", json!({"key": "scaleArt", "value": true})).unwrap();
    }
    artboard_navigation_next(&mut bar);
    let (before, selection, undo, other, other_history) = {
        let mut app = bar.app();
        assert_eq!(app.view().unwrap().artboard, 1);
        assert_eq!(app.session.tool_options()["selected"], json!([1]), "start with the tool and navigator aligned");
        app.session.active_mut().unwrap().mark_saved();
        let before = app.session.active().unwrap().doc.clone();
        let selection = app.session.active().unwrap().selection.clone();
        let undo = app.session.active().unwrap().history.undo.len();
        let other = app.session.documents()[1].doc.clone();
        let other_history = (app.session.documents()[1].history.undo.len(), app.session.documents()[1].history.redo.len());
        (before, selection, undo, other, other_history)
    };

    // No direct set_active, tool reselection, or selected-option repair after this round trip.
    artboard_document_click_tab(&mut bar, 1);
    artboard_document_click_tab(&mut bar, 0);
    let (selected_before_delete, active_before_delete) = {
        let app = bar.app();
        assert_eq!(app.view().unwrap().artboard, 1, "the document keeps its displayed artboard");
        assert_eq!(app.session.active().unwrap().doc, before, "switching tabs is not a document edit");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
        assert!(!app.session.active().unwrap().is_dirty());
        assert_eq!(app.session.tool_options()["moveArt"], json!(false));
        assert_eq!(app.session.tool_options()["scaleArt"], json!(true));
        (app.session.tool_options()["selected"].clone(), app.session.tool_options()["active"].clone())
    };

    let delete = properties_delete(&mut bar, "Shown");
    let commands = bar.app().run_count;
    bar.click_on(&delete);
    let mut app = bar.app();
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
    let mut bar = artboard_document_app();
    bar.app().select_tool("artboard");
    artboard_navigation_next(&mut bar);
    let (options, before, history) = {
        let mut app = bar.app();
        app.run("tool.setOption", json!({"key": "selected", "value": [1, 0]})).unwrap();
        app.run("tool.setOption", json!({"key": "moveArt", "value": false})).unwrap();
        app.run("tool.setOption", json!({"key": "scaleArt", "value": true})).unwrap();
        let options = app.session.tool_options();
        let before = app.session.active().unwrap().doc.clone();
        let history = (app.session.active().unwrap().history.undo.len(), app.session.active().unwrap().history.redo.len());
        (options, before, history)
    };
    artboard_document_click_tab(&mut bar, 0);
    for _ in 0..3 {
        artboard_document_tabs(&mut bar);
    }
    let app = bar.app();
    assert_eq!(app.session.tool_options(), options, "an already-active tab must not collapse valid multiselection or options");
    assert_eq!(app.view().unwrap().artboard, 1);
    assert_eq!(app.session.active().unwrap().doc, before);
    assert_eq!((app.session.active().unwrap().history.undo.len(), app.session.active().unwrap().history.redo.len()), history);
}

#[test]
fn artboard_document_tab_roundtrip_preserves_other_tools() {
    for tool in ["selection", "rectangle"] {
        let mut bar = artboard_document_app();
        bar.app().select_tool(tool);
        artboard_navigation_next(&mut bar);
        let (before, selection, options, history) = {
            let app = bar.app();
            let before = app.session.active().unwrap().doc.clone();
            let selection = app.session.active().unwrap().selection.clone();
            let options = app.session.tool_options();
            let history = (app.session.active().unwrap().history.undo.len(), app.session.active().unwrap().history.redo.len());
            (before, selection, options, history)
        };
        artboard_document_click_tab(&mut bar, 1);
        artboard_document_click_tab(&mut bar, 0);
        let app = bar.app();
        assert_eq!(app.session.tool_id(), tool);
        assert_eq!(app.session.tool_options(), options);
        assert_eq!(app.view().unwrap().artboard, 1);
        assert_eq!(app.session.active().unwrap().doc, before);
        assert_eq!(app.session.active().unwrap().selection, selection);
        assert_eq!((app.session.active().unwrap().history.undo.len(), app.session.active().unwrap().history.redo.len()), history);
    }
}

/// Press F6 with `modifiers` through the window's frame.
fn press_f6(app: &mut crate::VectorcraftApp, modifiers: egui::Modifiers) {
    key_through_the_app(app, egui::Key::F6, modifiers);
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
    let mut bar = artboard_document_app();
    bar.app().select_tool("artboard");
    artboard_navigation_next(&mut bar);
    let (before, undo) = {
        let app = bar.app();
        assert_eq!(app.session.tool_options()["selected"], json!([1]));
        (app.session.active().unwrap().doc.clone(), app.session.active().unwrap().history.undo.len())
    };
    press_f6(&mut bar.app(), egui::Modifiers::COMMAND);
    assert_eq!(bar.app().session.active_index(), Some(1));
    // The app draws frames while the other document is the current one.
    bar.draw(properties).settle();
    artboard_document_tabs(&mut bar);
    press_f6(&mut bar.app(), egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
    {
        let mut app = bar.app();
        assert_eq!(app.session.active_index(), Some(0));
        app.sync_views();
        assert_eq!(app.session.tool_options()["selected"], json!([1]), "the tool targets the displayed artboard");
        assert_eq!(app.view().unwrap().artboard, 1, "the document keeps its displayed artboard");
        assert_eq!(app.session.active().unwrap().doc, before, "switching documents is not a document edit");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
    }
    let delete = properties_delete(&mut bar, "Shown");
    bar.click_on(&delete);
    let names: Vec<_> = bar.app().session.active().unwrap().doc.artboards.iter().map(|a| a.name.clone()).collect();
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

fn assert_outside_canvas(bar: &Shown) {
    let p = bar.pointer().expect("a real chrome or panel pointer position");
    assert!(!bar.app().view().unwrap().rect.contains(p), "the pointer must stay outside the canvas: {p:?}");
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
fn close_b_tab(bar: &mut Shown, base: &Baseline) {
    let Baseline { uid, revision, before, selection, history_before, history_limits, expected: _ } = base;
    // Route from the status bar along the panel edge to the tab strip, never through canvas.
    bar.draw(tabs_with_views);
    for at in [egui::pos2(1000.0, 280.0), egui::pos2(1000.0, 20.0)] {
        bar.frame(vec![egui::Event::PointerMoved(at)]);
        assert_outside_canvas(bar);
    }
    artboard_document_click_tab(bar, 1);
    assert_outside_canvas(bar);
    let other_uid = {
        let mut app = bar.app();
        app.session.active_mut().unwrap().mark_saved();
        let other_uid = app.session.active().unwrap().uid;
        assert_ne!(other_uid, *uid);
        assert!(app.session.documents().iter().all(|d| !d.is_dirty()), "both ordinary documents are in saved state");
        assert!(!app.any_dialog_open());
        other_uid
    };
    eprintln!("ARTBOARD_CLOSE_TARGET_SETUP_READY");

    // B's tab and its close box, which the tab's name names.
    let tabs = artboard_document_tabs(bar);
    assert_outside_canvas(bar);
    assert_eq!(tabs.len(), 2);
    let title = bar.tree().name_of(tabs[1].node).unwrap_or_else(|| panic!("B's tab is named: {:?}", bar.names())).to_owned();
    let close = bar.find(&format!("Close {title}")).unwrap_or_else(|| panic!("B's tab has a close box: {:?}", bar.names()));
    assert!(!bar.app().view().unwrap().rect.contains(close.center()));
    bar.click_on(&close);
    assert_outside_canvas(bar);
    let app = bar.app();
    assert_eq!(app.session.documents().len(), 1, "B tab x must really close B");
    assert_eq!(app.session.active_index(), Some(0));
    assert_eq!(app.session.active().unwrap().uid, *uid, "the survivor is the original A, not a replacement document");
    assert!(app.session.documents().iter().all(|d| d.uid != other_uid));
    assert!(!app.any_dialog_open(), "a clean close must not leave a save/discard modal");
    drop(app);
    for _ in 0..2 {
        artboard_document_tabs(bar);
        assert_outside_canvas(bar);
    }
    let app = bar.app();
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
fn delete_shown_board(bar: &mut Shown, base: &Baseline) {
    let Baseline { uid, revision, before, selection, history_before, history_limits, expected } = base;
    // Cross the tab strip to the Properties panel before moving down to its Delete control.
    bar.draw(|app, ui| {
        app.sync_views();
        properties(app, ui);
    });
    bar.frame(vec![egui::Event::PointerMoved(egui::pos2(1000.0, 20.0))]);
    assert_outside_canvas(bar);
    bar.settle();
    assert_outside_canvas(bar);
    assert!(bar.shows("Shown"), "actual Properties must display Shown before deletion: {:?}", bar.names());
    let tool_before_delete = {
        let app = bar.app();
        assert_eq!(app.view().unwrap().artboard, 1);
        app.session.tool_options()
    };
    eprintln!("ARTBOARD_CLOSE_TARGET_SHOWN_DISPLAYED tool={tool_before_delete}");
    let delete = bar.named("Delete Artboard");
    assert!(delete.center().x > bar.app().view().unwrap().rect.right(), "Delete is in the right panel");
    let commands = bar.app().run_count;
    bar.click_on(&delete);
    assert_outside_canvas(bar);
    let mut app = bar.app();
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
    undo_and_redo_the_deletion(&mut app, base);
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
    let mut bar = artboard_document_app();
    {
        let mut app = bar.app();
        app.select_tool("artboard");
        app.run("tool.setOption", json!({"key": "moveArt", "value": false})).unwrap();
        app.run("tool.setOption", json!({"key": "scaleArt", "value": true})).unwrap();
    }
    artboard_navigation_next(&mut bar);
    assert_outside_canvas(&bar);
    let mut app = bar.app();
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
    drop(app);
    let mut expected = before.as_ref().clone();
    expected.artboards = vec![before.artboards[0].clone()];
    let keep = before.ruler_guides()[0].id;
    expected.retain_guides(&mut vectorcraft_doc::Selection::default(), |g| g.id == keep);
    let base = Baseline { uid, revision, before, selection, history_before, history_limits, expected };
    close_b_tab(&mut bar, &base);
    delete_shown_board(&mut bar, &base);
}

/// The screen mode the app keeps (not saved, #472) maps to the shell's: Normal Screen Mode (0) to Standard, Full
/// Screen Mode with Menu Bar (1) to FullScreenWithMenuBar, Full Screen Mode (2) and Presentation Mode (3) to
/// FullScreen (#4365). Entering and leaving the OS's full screen is the shell's.
#[test]
fn full_screen_modes_fill_the_screen_and_normal_mode_restores_the_window() {
    use craft::ui::shell::ScreenMode;
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    for (kept, mode) in [(0, ScreenMode::Standard), (1, ScreenMode::FullScreenWithMenuBar), (2, ScreenMode::FullScreen), (3, ScreenMode::FullScreen)]
    {
        app.ui.screen_mode = kept;
        assert_eq!(app.ui.screen_mode(), mode, "screen mode {kept}");
    }
    // PLAN: hub-w1 passes `allow_reveal` inline in `ui_frame` (its decision 41), so this table cannot assert that only Presentation Mode (3) turns the reveal off; that needs one reader of the rule, `UiState::allow_reveal(&self) -> bool` (hub-w1).
}

/// The Rectangle, Rounded Rectangle, Ellipse, Polygon and Star tools show Hide Shape Widgets in the
/// Control bar, with a selection and without one; a click hides the widgets and the button then reads
/// Show Shape Widgets, and a second click shows them again. The Selection, Spiral and Line Segment
/// tools show no such button.
#[test]
fn the_shape_tools_control_bar_hides_and_shows_the_shape_widgets() {
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
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap()["id"].clone();
    let mut bar = control_shown(app);
    let shows = |bar: &mut Shown, name: &str| bar.settle().find(name).is_some();
    for ids in [json!([id]), json!([])] {
        bar.app().run("select.set", json!({ "ids": ids })).unwrap();
        for tool in ["rectangle", "roundedRectangle", "ellipse", "polygon", "star"] {
            bar.app().select_tool(tool);
            assert!(shows(&mut bar, "Hide Shape Widgets"), "{tool} {ids}");
            bar.click("Hide Shape Widgets");
            assert!(!bar.app().ui.view.shape_widgets, "{tool} {ids}");
            assert!(!shows(&mut bar, "Hide Shape Widgets"), "{tool} {ids}");
            bar.click("Show Shape Widgets");
            assert!(bar.app().ui.view.shape_widgets, "{tool} {ids}");
        }
    }
    for tool in ["selection", "spiral", "lineSegment"] {
        bar.app().select_tool(tool);
        assert!(!shows(&mut bar, "Hide Shape Widgets"), "{tool}");
    }
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
    let mut bar = control_shown(app);
    bar.room(egui::vec2(1200.0, 400.0));
    let room = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 400.0));
    // The menu row `name`, which shows inside the window and is checked while `on`.
    let check = |bar: &Shown, name: &str, on: bool| {
        let row = bar.find(name).unwrap_or_else(|| panic!("missing {name}: {:?}", bar.names()));
        assert!(room.contains_rect(row.rect), "menu control stays inside the window: {row:?}");
        assert_eq!(bar.checked(&row), on, "{name}");
        row
    };
    let open = |bar: &mut Shown| {
        bar.click("Snapping");
        bar.settle();
    };
    let dismiss = |bar: &mut Shown| {
        let state = |app: &crate::VectorcraftApp| (app.ui.view.snap_to_glyph, app.session.prefs.to_json(), app.run_count);
        let before = state(&bar.app());
        bar.key(egui::Key::Escape).settle();
        assert!(bar.find("Snap to Glyph").is_none(), "Escape closes Snapping");
        assert_eq!(state(&bar.app()), before, "dismissal changes no settings and runs no command");
    };
    open(&mut bar);
    let glyph = check(&bar, "Snap to Glyph", false);
    check(&bar, "Alignment Guides", true);
    // The two extra rows still leave the original menu entries visible.
    for name in ["Smart Guides", "Snap to Point", "Snap to Grid", "Snap to Pixel", "Snap to Tangent", "Snap to Perpendicular"] {
        let row = bar.find(name).unwrap_or_else(|| panic!("missing {name}"));
        assert!(room.contains_rect(row.rect), "menu control stays inside the window: {row:?}");
    }
    bar.click_on(&glyph).settle();
    {
        let app = bar.app();
        assert!(app.ui.view.snap_to_glyph);
        assert_eq!(crate::menus::checked(&app, "view.snapToGlyph", &json!({})), Some(true));
        assert!(app.session.prefs.alignment_guides, "glyph snapping leaves Alignment Guides alone");
    }
    dismiss(&mut bar);
    open(&mut bar);
    check(&bar, "Snap to Glyph", true);
    let alignment = check(&bar, "Alignment Guides", true);
    let commands = bar.app().run_count;
    bar.click_on(&alignment).settle();
    let mut app = bar.app();
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
    drop(app);
    dismiss(&mut bar);
    open(&mut bar);
    check(&bar, "Snap to Glyph", true);
    check(&bar, "Alignment Guides", false);
    // Changes made through the existing command surfaces are reflected on the next opening too.
    {
        let mut app = bar.app();
        crate::menus::invoke(&mut app, "view.snapToGlyph", json!({}));
        app.run("prefs.set", json!({"key": "alignmentGuides", "value": true})).unwrap();
    }
    dismiss(&mut bar);
    open(&mut bar);
    check(&bar, "Snap to Glyph", false);
    check(&bar, "Alignment Guides", true);
}

/// `window.control.dock {edge}` docks the Control bar to the top or the bottom of the window, or floats it, and the
/// panel menu checks the edge in use; another edge, or none, is an error and changes nothing. A floating bar keeps
/// the edge it docked to last, and floats again where it last floated after docking in between.
/// `window.control.options {group, visible?}` turns a group of the bar's options on or off, or toggles it, and the
/// panel menu's item shows it; an unknown or missing group, or a `visible` that is not true or false, is an error
/// and changes nothing.
#[test]
fn control_bar_commands_set_its_place_and_groups() {
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    let edge = |app: &crate::VectorcraftApp, edge: &str| crate::menus::checked(app, "window.control.dock", &json!({ "edge": edge }));
    assert_eq!(app.run("window.control.dock", json!({"edge": "bottom"})).unwrap(), json!({"edge": "bottom"}));
    assert!(app.ui.control_bar_bottom);
    assert_eq!((edge(&app, "bottom"), edge(&app, "top")), (Some(true), Some(false)));
    for p in [json!({"edge": "left"}), json!({"edge": "Bottom"}), json!({"edge": "Float"}), json!({"edge": 1}), json!({})] {
        let err = app.run("window.control.dock", p.clone()).unwrap_err();
        assert!(err.starts_with("`edge` must be one of"), "{p}: {err}");
        assert!(app.ui.control_bar_bottom && !app.ui.control_bar_floating, "{p} leaves the bar at the bottom");
    }
    assert_eq!(app.run("window.control.dock", json!({"edge": "float"})).unwrap(), json!({"edge": "float"}));
    assert!(app.ui.control_bar_floating && app.ui.control_bar_bottom, "floating, it keeps the edge it docked to last");
    assert_eq!((edge(&app, "top"), edge(&app, "bottom"), edge(&app, "float")), (Some(false), Some(false), Some(true)));
    assert!(app.run("window.control.dock", json!({"edge": "side"})).is_err());
    assert!(app.ui.control_bar_floating, "an error leaves the bar floating");
    app.ui.control_bar_float_pos = Some([240.0, 160.0]);
    assert_eq!(app.run("window.control.dock", json!({"edge": "top"})).unwrap(), json!({"edge": "top"}));
    assert!(!app.ui.control_bar_bottom && !app.ui.control_bar_floating);
    app.run("window.control.dock", json!({"edge": "float"})).unwrap();
    assert_eq!(app.ui.control_bar_float_pos, Some([240.0, 160.0]), "it floats where it last floated");

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

/// Opens the Control bar's panel menu with its button unless it is open.
fn open_control_bar_menu(bar: &mut Shown) {
    bar.settle();
    if bar.find("Dock to Bottom").is_none() {
        bar.click("Panel menu").settle();
    }
    assert!(bar.find("Dock to Bottom").is_some(), "the panel menu opens: {:?}", bar.names());
}

/// With every group of options off, the Control bar keeps its gripper, the selection's label and
/// its panel menu, which lists every group off; nothing selected shows no Document Setup either.
#[test]
fn the_control_bar_keeps_its_label_and_menu_with_every_group_off() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap()["id"].clone();
    app.run("select.set", json!({ "ids": [id] })).unwrap();
    for group in crate::chrome::CONTROL_BAR_GROUPS {
        app.run("window.control.options", json!({ "group": group, "visible": false })).unwrap();
    }
    let label = crate::panels::appearance::object_label(&app).to_owned();
    let mut bar = control_shown(app);
    bar.room(egui::vec2(1400.0, 900.0)).settle();
    assert!(bar.shows(&label), "the label stays: {:?}", bar.names());
    for s in ["Snapping", "Stroke:", "Opacity:", "Style:", "Transform", BAR_FIELDS[0]] {
        assert!(!bar.shows(s), "{s} is off");
    }
    open_control_bar_menu(&mut bar);
    for label in CONTROL_BAR_GROUP_LABELS {
        let item = bar.find(label).unwrap_or_else(|| panic!("{label} is listed: {:?}", bar.names()));
        assert!(!bar.checked(&item), "{label} is listed, off");
    }
    bar.key(egui::Key::Escape);
    bar.app().run("select.set", json!({ "ids": [] })).unwrap();
    bar.settle();
    assert!(!bar.shows("Document Setup") && !bar.shows("Preferences"), "{:?}", bar.names());
}

/// The Control bar's selection label follows a selection whose middle object changes while its
/// length, first and last object stay the same, and a target circle with the selection unchanged.
#[test]
fn the_control_bar_label_follows_every_selected_object() {
    use crate::tests_removeanchors::{control_frame, has};
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    let id = |v: serde_json::Value| vectorcraft_doc::NodeId(v["id"].as_u64().unwrap());
    let r: Vec<_> = (0..3).map(|i| id(app.run("shape.rectangle", json!({"x": i * 30, "y": 10, "width": 20, "height": 20})).unwrap())).collect();
    let t = id(app.run("text.create", json!({"x": 10, "y": 100, "text": "Hi"})).unwrap());
    app.run("select.set", json!({ "ids": r })).unwrap();
    let ctx = crate::testing::ui_context();
    let alike = crate::panels::appearance::object_label(&app);
    assert!(has(&control_frame(&mut app, &ctx, vec![]), alike), "{alike}");
    app.session.active_mut().unwrap().selection.objects[1] = t;
    let bar = control_frame(&mut app, &ctx, vec![]);
    assert!(has(&bar, "Mixed Objects"), "{bar:?}");
    let layer = app.session.active().unwrap().doc.default_layer().unwrap();
    app.session.active_mut().unwrap().selection.target = Some(layer);
    assert!(has(&control_frame(&mut app, &ctx, vec![]), "Layer"), "the targeted layer");
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
    let mut bar = control_shown(app);
    // The narrowest window, in steps of 10 points, whose bar shows the Align buttons.
    let narrowest = |bar: &mut Shown| {
        (40..=240).map(|w| w as f32 * 10.0).find(|&w| {
            bar.room(egui::vec2(w, 300.0)).settle();
            bar.find("Horizontal Align Left").is_some() && bar.find("Panel menu").is_some()
        })
    };
    let all = narrowest(&mut bar).expect("the Align buttons show in a wide window");
    for group in ["fillStroke", "opacity", "graphicStyle"] {
        bar.app().run("window.control.options", json!({ "group": group, "visible": false })).unwrap();
    }
    let fewer = narrowest(&mut bar).expect("they show with fewer groups");
    assert!(fewer < all, "the Align buttons fit {fewer} wide with three groups off, {all} with every group on");
    for group in ["fillStroke", "opacity", "graphicStyle"] {
        bar.app().run("window.control.options", json!({ "group": group, "visible": true })).unwrap();
    }
    bar.app().run("window.control.options", json!({"group": "transform", "visible": false})).unwrap();
    let no_transform = narrowest(&mut bar).expect("they show without Transform");
    assert!(no_transform < all, "without the Transform link's room the Align buttons fit {no_transform} wide, {all} with it");
}

/// With the Artboard tool, the Artboard Options group holds the artboard's own controls and the
/// Transform group the Transform link and the X/Y/W/H fields; each one off leaves the other, and
/// with Artboard Options off the fields find room in a window that is too narrow for them otherwise.
#[test]
fn the_artboard_tool_s_control_bar_follows_its_two_groups() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    app.select_tool("artboard");
    let mut bar = control_shown(app);
    bar.room(egui::vec2(2400.0, 300.0)).settle();
    for s in ["Artboard", "Name:", "Transform", "Move Artwork with Artboard"] {
        assert!(bar.shows(s), "{s} shows with every group on: {:?}", bar.names());
    }
    let options = |bar: &mut Shown, visible: bool| {
        bar.app().run("window.control.options", json!({"group": "artboardOptions", "visible": visible})).unwrap();
    };
    // Some width shows the fields with Artboard Options off and leaves them out with it on.
    let freed = (40..=240).map(|w| w as f32 * 10.0).any(|w| {
        bar.room(egui::vec2(w, 300.0));
        options(&mut bar, true);
        let on = bar.settle().shows(BAR_FIELDS[0]);
        options(&mut bar, false);
        let off = bar.settle().shows(BAR_FIELDS[0]);
        !on && off
    });
    assert!(freed, "with Artboard Options off the fields find room in a window too narrow for them otherwise");
    options(&mut bar, false);
    bar.room(egui::vec2(2400.0, 300.0)).settle();
    assert!(!bar.shows("Name:") && !bar.shows("Move Artwork with Artboard"), "{:?}", bar.names());
    assert!(bar.shows("Artboard") && bar.shows("Transform") && bar.shows(BAR_FIELDS[0]), "{:?}", bar.names());
    options(&mut bar, true);
    bar.app().run("window.control.options", json!({"group": "transform", "visible": false})).unwrap();
    bar.settle();
    assert!(!bar.shows("Transform") && !bar.shows(BAR_FIELDS[0]), "{:?}", bar.names());
    assert!(bar.shows("Name:") && bar.shows("Move Artwork with Artboard"), "{:?}", bar.names());
}

/// Whether the Control bar shows each of `names`, at each width from 2400 down to 150 points in 10-point steps.
fn control_bar_widths(bar: &mut Shown, names: &[&str]) -> Vec<Vec<bool>> {
    (15..=240)
        .rev()
        .map(|w| {
            bar.room(egui::vec2(w as f32 * 10.0, 300.0)).settle();
            names.iter().map(|name| bar.find(name).is_some()).collect()
        })
        .collect()
}

/// A 20 × 20 PNG, half black and half white, written to a folder `dir` of its own; its path.
fn half_png(dir: &str) -> std::path::PathBuf {
    let img = image::RgbaImage::from_fn(20, 20, |x, _| image::Rgba(if x < 10 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let dir = vectorcraft_testkit::temp_dir(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("half.png");
    std::fs::write(&path, &png).unwrap();
    path
}

/// As its room runs short the Control bar leaves its members out in its priority order: some width shows the first
/// of each pair without the second, which goes before it. Select Similar never shows while one of a selected image's
/// controls is left out, and the X, Y, W and H fields show or go together, before the groups ahead of them.
#[test]
fn the_control_bar_leaves_members_out_in_its_priority_order() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    let rect = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap()["id"].clone();
    let path = half_png("control-bar-priority");
    let linked = app.run("file.place", json!({"path": path, "link": true})).unwrap()["ids"].clone();
    let embedded = app.run("file.place", json!({"path": path, "link": false})).unwrap()["ids"].clone();
    let mut bar = control_shown(app);
    let select = |bar: &mut Shown, ids: serde_json::Value| {
        bar.app().run("select.set", json!({ "ids": ids })).unwrap();
    };
    let groups = |bar: &mut Shown, groups: &[&str], visible: bool| {
        for group in groups {
            bar.app().run("window.control.options", json!({ "group": group, "visible": visible })).unwrap();
        }
    };
    // A wide bar shows `kept` and `left`, and some width shows `kept` without `left`.
    let left_first = |bar: &mut Shown, case: &str, kept: &str, left: &str| {
        let seen = control_bar_widths(bar, &[kept, left]);
        assert!(seen.iter().any(|s| s[0] && s[1]), "{case}: a wide bar shows {kept} and {left}");
        assert!(seen.iter().any(|s| s[0] && !s[1]), "{case}: some width shows {kept} without {left}");
    };

    select(&mut bar, json!([rect]));
    groups(&mut bar, &["fillStroke"], false);
    left_first(&mut bar, "a rectangle", "Style:", "Opacity:");
    groups(&mut bar, &["fillStroke"], true);

    select(&mut bar, json!([]));
    groups(&mut bar, &["fillStroke", "opacity", "graphicStyle"], false);
    left_first(&mut bar, "nothing selected", "Preferences", "Document Setup");
    groups(&mut bar, &["fillStroke", "opacity", "graphicStyle"], true);

    select(&mut bar, linked);
    left_first(&mut bar, "a linked image", "Embed", "Edit Original");

    select(&mut bar, json!([]));
    bar.app().select_tool("artboard");
    left_first(&mut bar, "the Artboard tool", "Transform", "Rearrange All Artboards");
    bar.app().select_tool("selection");

    // With these groups off, the image's controls come right before Select Similar.
    select(&mut bar, embedded);
    let ahead = ["opacity", "graphicStyle", "alignDistribute", "transform"];
    groups(&mut bar, &ahead, false);
    let seen = control_bar_widths(&mut bar, &["Remove Background", "Select Similar Objects"]);
    assert!(seen.iter().any(|s| s[0] && s[1]), "a wide bar shows Select Similar after the image's controls");
    assert!(seen.iter().any(|s| s[0] && !s[1]), "some width shows the image's controls without Select Similar");
    assert!(seen.iter().all(|s| !s[1] || s[0]), "Select Similar shows while an image control is left out");
    groups(&mut bar, &ahead, true);

    select(&mut bar, json!([rect]));
    let [x, y, w, h] = BAR_FIELDS;
    let seen = control_bar_widths(&mut bar, &["Horizontal Align Left", x, y, w, h]);
    assert!(seen.iter().all(|s| s[2..].iter().all(|f| *f == s[1])), "the four fields show or go together");
    assert!(seen.iter().any(|s| s[0] && s[1]), "a wide bar shows the fields and the Align buttons");
    assert!(seen.iter().any(|s| s[0] && !s[1]), "some width shows the Align buttons without the fields");
}

/// With type selected, the Character and Paragraph groups leave the Control bar before the members ahead of them: at
/// every width that shows the Character group, Opacity, Recolor Artwork and the Style picker show too, the Paragraph
/// group leaves before the Character group, the Align buttons leave before the Paragraph group, and some width shows
/// Opacity without them.
#[test]
fn the_type_groups_leave_the_control_bar_before_opacity_and_style() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    let text = app.run("text.create", json!({"x": 10, "y": 100, "text": "Hi"})).unwrap()["id"].clone();
    app.run("select.set", json!({ "ids": [text] })).unwrap();
    let mut bar = control_shown(app);
    let names = ["Character:", "Paragraph:", "Opacity:", "Recolor Artwork", "Style:", "Horizontal Align Left"];
    let seen = control_bar_widths(&mut bar, &names);
    assert!(seen.iter().any(|s| s.iter().all(|shown| *shown)), "a wide bar shows {names:?}");
    for s in &seen {
        assert!(!s[0] || s[2..5].iter().all(|shown| *shown), "the Character group shows only with {:?}: {s:?}", &names[2..5]);
        assert!(!s[1] || s[0], "the Paragraph group shows only with the Character group: {s:?}");
        assert!(!s[5] || s[1], "the Align buttons show only with the Paragraph group: {s:?}");
    }
    assert!(seen.iter().any(|s| s[2] && !s[0]), "some width shows Opacity without the Character group");
}

/// The names of the bar's X, Y, W and H fields, in their order.
const BAR_FIELDS: [&str; 4] = ["X position", "Y position", "Width", "Height"];

/// The caption of the bar's field named `field`: the link that names it, not the field.
fn caption(bar: &Shown, field: &str) -> Option<Target> {
    let tree = bar.tree();
    bar.fixture.by_role(Role::LINK).into_iter().find(|t| tree.name_of(t.node) == Some(field))
}

/// The Control bar's labels that open a panel: a click on the Linked File or Embedded label shows the Links panel
/// in its dock, expanded or collapsed to icons, and a rectangle's label opens nothing. A click on the X, Y, W or H
/// caption opens the Transform panel under it and a second click closes it, leaving the selection and its bounds as
/// they were and no field with the keyboard.
#[test]
fn label_links_open_their_panels() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    let path = half_png("control-bar-image-label");
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap()["id"].clone();
    let linked = app.run("file.place", json!({"path": path, "link": true})).unwrap()["ids"].clone();
    let embedded = app.run("file.place", json!({"path": path, "link": false})).unwrap()["ids"].clone();
    let links = crate::panels::links::ID;
    // The default layout floats Links: dock it beside Layers so that both passes reach a docked panel.
    app.run("window.panel", json!({"panel": links})).unwrap();
    app.run("window.panel.dock", json!({"panel": links, "onto": "layers"})).unwrap();
    let mut bar = control_shown(app);
    bar.room(egui::vec2(2400.0, 600.0));
    // Closes the Links panel, clicks `name` on the bar and answers whether the panel shows.
    let click = |bar: &mut Shown, name: &str| {
        // Closing a panel that is already closed is an error that changes nothing.
        bar.app().run("window.panel.close", json!({"panel": links})).ok();
        assert!(!crate::panel_docking::visible(&bar.app(), links), "{name}: the panel is closed first");
        bar.click(name);
        crate::panel_docking::visible(&bar.app(), links)
    };
    // Whether Links floats, and whether it opens in its collapsed column's flyout.
    let placed = |bar: &Shown| {
        let app = bar.app();
        let layout = crate::panel_docking::layout_of(&app.ui);
        (layout.floating.iter().any(|w| w.panels().any(|p| p == links)), layout.opens_in_flyout(&links.to_owned()))
    };
    for collapsed in [false, true] {
        bar.app().run("window.collapseDock", json!({ "collapsed": collapsed })).unwrap();
        bar.app().run("select.set", json!({ "ids": linked })).unwrap();
        assert!(click(&mut bar, "Linked File"), "collapsed {collapsed}, status: {}", bar.app().ui.status);
        assert_eq!(placed(&bar), (false, collapsed), "Links opened in its dock, collapsed {collapsed}");
        bar.app().run("select.set", json!({ "ids": embedded })).unwrap();
        assert!(click(&mut bar, "Embedded"), "collapsed {collapsed}, status: {}", bar.app().ui.status);
        assert_eq!(placed(&bar), (false, collapsed), "Links opened in its dock, collapsed {collapsed}");
        bar.app().run("select.set", json!({ "ids": [id] })).unwrap();
        let name = crate::panels::appearance::object_label(&bar.app()).to_owned();
        assert!(!click(&mut bar, &name), "collapsed {collapsed}: a rectangle's label opens nothing");
    }

    let state = |bar: &Shown| {
        let app = bar.app();
        let st = app.session.active().unwrap();
        let b = st.doc.bounds_of(&st.selection.objects, false).unwrap();
        (st.selection.objects.len(), [b.x0, b.y0, b.x1, b.y1])
    };
    // The Transform panel's Rotate field is not on the bar itself.
    let open = |bar: &Shown| bar.find("Rotate").is_some();
    let before = state(&bar);
    for name in BAR_FIELDS {
        bar.settle();
        assert!(!open(&bar), "{name}: the Transform panel is closed at first");
        let at = caption(&bar, name).unwrap_or_else(|| panic!("no {name} caption: {:?}", bar.names()));
        bar.click_on(&at).settle();
        assert!(open(&bar), "{name}: a click on the caption opens the Transform panel");
        assert!(bar.fixture.ime().is_none(), "{name}: no field took the keyboard");
        assert_eq!(state(&bar), before, "{name}: the selection and its bounds stay");
        bar.click_on(&at).settle();
        assert!(!open(&bar), "{name}: a second click closes it");
    }
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
    let ctx = crate::testing::ui_context();
    let bar = control_frame(&mut app, &ctx, vec![]);
    assert!(has(&bar, "Convert:") && has(&bar, "Anchors:"), "{bar:?}");
    app.run("window.control.options", json!({"group": "anchors", "visible": false})).unwrap();
    let bar = control_frame(&mut app, &ctx, vec![]);
    assert!(!has(&bar, "Convert:") && !has(&bar, "Anchors:"), "{bar:?}");
    assert!(has(&bar, "Anchor Point") && has(&bar, "Stroke:"), "{bar:?}");
}

/// With the Artboard tool in use, the Control bar shows the Align buttons after the artboard
/// controls while it has room, and the Distribute buttons for two or more selected artboards with
/// Align to Artboard and three or more with Align to Selection. A
/// click aligns the selected artboards as the Align panel's button does. The group is left out
/// while the panel menu has it off.
#[test]
fn the_control_bar_aligns_the_selected_artboards_with_the_artboard_tool() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("artboard.new", json!({"x": 150, "y": 40, "width": 60, "height": 80})).unwrap();
    app.select_tool("artboard");
    let mut bar = control_shown(app);
    bar.room(egui::vec2(2400.0, 300.0));
    let button = |bar: &mut Shown, name: &str| bar.settle().find(name);
    bar.app().session.set_tool_option("selected", &json!([1]));
    assert!(button(&mut bar, "Vertical Align Top").is_some(), "one artboard: the Align buttons");
    assert!(button(&mut bar, "Horizontal Distribute Left").is_none(), "one artboard: no Distribute buttons");
    bar.app().session.set_tool_option("selected", &json!([1, 0]));
    assert!(button(&mut bar, "Horizontal Distribute Left").is_none(), "two artboards, Align to Selection: no Distribute buttons");
    bar.app().ui.align_to = crate::panels::align::AlignTo::Artboard;
    assert!(button(&mut bar, "Horizontal Distribute Left").is_some(), "two artboards, Align to Artboard: the Distribute buttons");
    bar.app().ui.align_to = crate::panels::align::AlignTo::Selection;
    let top = button(&mut bar, "Vertical Align Top").expect("Vertical Align Top");
    bar.click_on(&top);
    {
        let app = bar.app();
        let tops: Vec<f64> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect.y0).collect();
        assert_eq!(tops, [0.0, 0.0], "{}", app.ui.status);
    }
    bar.app().run("window.control.options", json!({"group": "alignDistribute", "visible": false})).unwrap();
    assert!(button(&mut bar, "Vertical Align Top").is_none(), "the group is off");
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
    let mut bar = control_shown(app);
    const DISTRIBUTE: [&str; 6] = [
        "Vertical Distribute Top",
        "Vertical Distribute Center",
        "Vertical Distribute Bottom",
        "Horizontal Distribute Left",
        "Horizontal Distribute Center",
        "Horizontal Distribute Right",
    ];
    let buttons = |bar: &mut Shown| {
        bar.settle();
        DISTRIBUTE.map(|name| bar.find(name))
    };
    bar.app().run("select.set", json!({"ids": [a]})).unwrap();
    assert!(buttons(&mut bar).iter().all(Option::is_none), "one object: no distribute buttons");
    bar.app().run("select.set", json!({"ids": [a, b, c]})).unwrap();
    let shown = buttons(&mut bar);
    assert!(shown.iter().all(Option::is_some), "three objects: {shown:?}");
    let left_edges = shown[3].expect("Horizontal Distribute Left");
    bar.click_on(&left_edges);
    let left = |app: &crate::VectorcraftApp, id: &serde_json::Value| {
        app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().geometric_bounds().unwrap().x0
    };
    let app = bar.app();
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
    let mut bar = control_shown(app);
    // The Align To menu and the value it shows.
    let menu = |bar: &mut Shown| {
        let menu = bar.settle().find("Align To:").unwrap_or_else(|| panic!("the Control bar has the Align To menu"));
        assert_eq!(bar.tree().role_of(menu.node), Some(Role::COMBO_BOX));
        (menu, bar.tree().value_of(menu.node).map(str::to_owned))
    };
    // Opens the menu, checks its rows and picks `pick`, or closes it with Escape.
    let choose = |bar: &mut Shown, checked: &str, key_on: bool, pick: Option<&str>| {
        let (opener, _) = menu(bar);
        bar.click_on(&opener).settle();
        for name in ["Align to Selection", "Align to Key Object", "Align to Artboard"] {
            let row = bar.find(name).unwrap_or_else(|| panic!("the menu lists {name}"));
            assert_eq!(bar.checked(&row), name == checked, "{name} checked");
            assert_eq!(bar.tree().is_disabled_key(row.node), name == "Align to Key Object" && !key_on, "{name} off");
        }
        match pick {
            Some(pick) => {
                bar.click(pick);
            }
            None => {
                bar.key(egui::Key::Escape);
            }
        }
        assert!(bar.settle().find("Align to Artboard").is_none(), "the menu closes");
    };
    let key = |bar: &Shown| bar.app().session.active().unwrap().selection.key;
    let x0 = |app: &crate::VectorcraftApp, id: &serde_json::Value| {
        app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().geometric_bounds().unwrap().x0
    };

    // One object: the buttons align it to the artboard, and Align to Key Object is off.
    bar.app().run("select.set", json!({"ids": [a]})).unwrap();
    assert_eq!(menu(&mut bar).1.as_deref(), Some("Align to Artboard"), "one object shows Artboard");
    choose(&mut bar, "Align to Artboard", false, None);
    assert_eq!(key(&bar), None);

    // Two objects: the stored choice, Selection.
    bar.app().run("select.set", json!({"ids": [a, b]})).unwrap();
    assert_eq!(menu(&mut bar).1.as_deref(), Some("Align to Selection"));
    choose(&mut bar, "Align to Selection", true, Some("Align to Artboard"));
    assert_eq!(bar.app().ui.align_to, crate::panels::align::AlignTo::Artboard, "the choice the Align panel shows");
    assert_eq!(menu(&mut bar).1.as_deref(), Some("Align to Artboard"));
    bar.click("Horizontal Align Left");
    {
        let app = bar.app();
        assert_eq!((x0(&app, &a), x0(&app, &b)), (0.0, 0.0), "aligned to the artboard: {}", app.ui.status);
    }

    // Key Object makes the topmost the key and keeps the stored choice.
    choose(&mut bar, "Align to Artboard", true, Some("Align to Key Object"));
    assert_eq!(key(&bar), Some(vectorcraft_doc::NodeId(b.as_u64().unwrap())), "the topmost object is the key");
    assert_eq!(bar.app().ui.align_to, crate::panels::align::AlignTo::Artboard, "the stored choice stays");
    assert_eq!(menu(&mut bar).1.as_deref(), Some("Align to Key Object"));

    // Selection lets go of the key and is stored.
    choose(&mut bar, "Align to Key Object", true, Some("Align to Selection"));
    assert_eq!(key(&bar), None, "the key object is let go");
    assert_eq!(bar.app().ui.align_to, crate::panels::align::AlignTo::Selection);
    assert_eq!(menu(&mut bar).1.as_deref(), Some("Align to Selection"));
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
    let mut bar = control_shown(app);
    // Whether the latest frame shows the options' Results line.
    let results = |bar: &Shown| bar.names().iter().chain(bar.texts().iter().map(|t| &t.text)).any(|t| t.starts_with("Result"));
    bar.app().run("select.set", json!({"ids": []})).unwrap();
    assert!(bar.settle().find("Global Edit Options").is_none(), "nothing selected: no dropdown");
    bar.app().run("select.set", json!({"ids": [a]})).unwrap();
    bar.click("Global Edit Options").settle();
    assert!(bar.shows("Results: 3"), "the options open with their Results: {:?}", bar.names());
    bar.click("Size").settle();
    assert!(bar.app().session.global_edit_options.size, "Match Size is the session's option, shared with Properties");
    assert!(bar.shows("Results: 2"), "Size narrows the Results: {:?}", bar.names());
    bar.app().run("select.globalEdit", json!({})).unwrap();
    let ge = bar.app().session.active().unwrap().global_edit.as_ref().map(|g| g.members.len());
    assert_eq!(ge, Some(2), "Start Global Edit takes the two rectangles of the same size");
    let button = bar.named("Global Edit Options");
    assert!(bar.tree().is_disabled_key(button.node), "the options are off while Global Edit is on");
    // The closed popover's body still runs through its exit fade, so the options are looked for once it ends.
    assert!(!results(bar.advance(1.0).settle()), "Global Edit starting closes the open options");
    bar.app().run("select.globalEdit", json!({})).unwrap();
    assert!(!results(bar.settle()), "the options stay closed once Global Edit stops");
}

/// The Control bar's Select Similar Objects button selects the objects like the first selected one
/// by the attribute kept for the session (All at first), and the Select Similar Options menu beside
/// it selects by the item chosen, which is kept and checked from then on. Both show with something
/// selected while the Object Options group is on.
#[test]
fn the_control_bar_selects_similar_objects() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let rect = |app: &mut crate::VectorcraftApp, x: f64, color: &str| {
        let id = app.run("shape.rectangle", json!({"x": x, "y": 20, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
        app.run("paint.setFill", json!({"ids": [id], "color": color})).unwrap();
        id
    };
    let a = rect(&mut app, 20.0, "#ff0000");
    let b = rect(&mut app, 100.0, "#ff0000");
    let c = rect(&mut app, 200.0, "#0000ff");
    let mut bar = control_shown(app);
    bar.room(egui::vec2(2400.0, 300.0));
    let selected = |bar: &Shown| {
        let mut ids: Vec<u64> = bar.app().session.active().unwrap().selection.objects.iter().map(|i| i.0).collect();
        ids.sort();
        ids
    };
    let select = |bar: &mut Shown, ids: serde_json::Value| {
        bar.app().run("select.set", json!({ "ids": ids })).unwrap();
        bar.settle();
    };
    select(&mut bar, json!([]));
    assert!(bar.find("Select Similar Objects").is_none() && bar.find("Select Similar Options").is_none(), "nothing selected");
    select(&mut bar, json!([a]));
    bar.click("Select Similar Objects");
    assert_eq!(selected(&bar), vec![a, b], "All: the red rectangles");
    // The menu selects by the item chosen and keeps it.
    select(&mut bar, json!([a]));
    bar.click("Select Similar Options").settle();
    for label in [
        "All",
        "Fill Color",
        "Fill & Stroke",
        "Stroke Color",
        "Stroke Weight",
        "Opacity",
        "Blending Mode",
        "Appearance",
        "Graphic Style",
        "Shape",
        "Symbol Instance",
    ] {
        assert!(bar.find(label).is_some(), "{label} in the menu: {:?}", bar.names());
    }
    let all = bar.find("All").unwrap_or_else(|| panic!("All"));
    assert!(bar.checked(&all), "All is checked at first");
    bar.click("Stroke Color");
    assert_eq!(bar.app().session.select_similar, "strokeColor");
    assert_eq!(selected(&bar), vec![a, b, c], "Stroke Color: all three share the default stroke");
    select(&mut bar, json!([c]));
    bar.click("Select Similar Objects");
    assert_eq!(selected(&bar), vec![a, b, c], "the button keeps Stroke Color");
    bar.click("Select Similar Options").settle();
    let stroke = bar.find("Stroke Color").unwrap_or_else(|| panic!("Stroke Color"));
    assert!(bar.checked(&stroke), "the kept item is checked");
    bar.key(egui::Key::Escape);
    // The panel menu's Object Options group turns them off.
    bar.app().run("window.control.options", json!({"group": "objectOptions"})).unwrap();
    assert!(bar.settle().find("Select Similar Objects").is_none(), "Object Options off");
    bar.app().run("window.control.options", json!({"group": "objectOptions"})).unwrap();
    assert!(bar.settle().find("Select Similar Objects").is_some(), "Object Options on again");
}
