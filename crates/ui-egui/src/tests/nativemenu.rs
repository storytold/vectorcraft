//! The native menu bar's app side (`menus::native`), tested on every platform with a fake backend:
//! when the app reads the menus again, the Settings pages the Mac layout lists, and the key
//! equivalents the menus hand back.

use std::cell::RefCell;
use std::rc::Rc;

use craft::menu::{Backend, Chord, Dispatch, Event as MenuEvent, Item, MenuBar, MenuRole, NativeMenu, Node, mac_layout};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, vec2};
use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::prefscmds::PREF_CATEGORIES;

use crate::{VectorcraftApp, menus};

/// A new app, after the installed fonts' scan it starts has ended: until then Type › Font lists
/// only the loaded fonts (#5483), and a scan that ends part way through a test adds items between
/// two reads of the menus.
fn app(doc: bool) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    vectorcraft_text::FontDb::global().family_list();
    if doc {
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 37, "height": 40})).unwrap();
    }
    app
}

/// The item of `bar` that runs `command`.
fn find<'a>(bar: &'a MenuBar, command: &str) -> &'a Item {
    bar.items().into_iter().find(|it| it.command.id == command).unwrap_or_else(|| panic!("no {command} item"))
}

/// Settings ▸ lists the Preferences dialog's own pages, General… with ⌘K, and each opens the
/// dialog on its page.
#[test]
fn settings_lists_every_preferences_page() {
    let mut m = Mac::new(true);
    let layout = mac_layout(&m.app.menus.bar, &menus::native::options());
    let app_menu = layout.bar.menus.iter().find(|menu| menu.role == MenuRole::App).expect("the app menu");
    let settings = app_menu
        .children
        .iter()
        .find_map(|n| match n {
            Node::Submenu { label, children, .. } if label == "Settings" => Some(children),
            _ => None,
        })
        .expect("the Settings submenu");
    let pages: Vec<&Item> = settings.iter().filter_map(|n| if let Node::Item(it) = n { Some(it) } else { None }).collect();
    assert_eq!(pages.len(), PREF_CATEGORIES.len());
    assert_eq!((pages[0].label.as_str(), pages[0].shortcut), ("General…", Some(Chord::parse("Cmd+K").unwrap())));
    assert!(pages[1..].iter().all(|it| it.shortcut.is_none()));
    for (it, page) in pages.iter().zip(PREF_CATEGORIES) {
        assert_eq!(
            (it.command.id.as_str(), &it.command.params, it.label.clone()),
            ("edit.preferences", &json!({"category": page}), format!("{page}…"))
        );
    }
    let units = pages.iter().find(|it| it.command.params["category"] == "Units").expect("the Units page").command.clone();
    m.fake.report(MenuEvent::Click(units));
    m.frame(vec![]);
    assert!(crate::prefs_dialog::is_open(&m.app), "a page opens the dialog, not a parameter form");
    assert_eq!(crate::prefs_dialog::page(&m.app), Some("Units"));
}

/// A native menu can't turn a plain item into a check item in place: the app's read of the menus
/// is guarded by a state key, and these tests count the reads through a fake backend.
///
/// The fake backend: the menus the app synced, and events to report.
#[derive(Clone, Default)]
struct Fake(Rc<RefCell<(Vec<MenuBar>, Vec<MenuEvent>)>>);

impl Backend for Fake {
    fn sync(&mut self, bar: &MenuBar) {
        self.0.borrow_mut().0.push(bar.clone());
    }
    fn drain(&mut self) -> Vec<MenuEvent> {
        std::mem::take(&mut self.0.borrow_mut().1)
    }
    fn attach(&mut self, _dispatch: Dispatch) {}
}

impl Fake {
    /// How many times the app pushed the menus; the driver pushes only when the app laid them out
    /// again, so one push is one read.
    fn synced(&self) -> usize {
        self.0.borrow().0.len()
    }
    fn last(&self) -> MenuBar {
        self.0.borrow().0.last().cloned().unwrap()
    }
    fn report(&self, e: MenuEvent) {
        self.0.borrow_mut().1.push(e);
    }
}

/// The app with the fake native menu (or none), driven a whole frame at a time like the desktop
/// app.
struct Mac {
    app: VectorcraftApp,
    fake: Fake,
    ctx: egui::Context,
    time: f64,
}

impl Mac {
    fn new(native: bool) -> Self {
        let mut app = app(true);
        let fake = Fake::default();
        if native {
            app.services.native_menu = Some(NativeMenu::new(Box::new(fake.clone())));
        }
        let mut m = Mac { app, fake, ctx: crate::testing::ui_context(), time: 0.0 };
        for _ in 0..3 {
            m.frame(vec![]);
        }
        m
    }

    fn frame(&mut self, events: Vec<Event>) -> egui::FullOutput {
        self.time += 0.1;
        let raw = egui::RawInput {
            events,
            time: Some(self.time),
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 850.0))),
            ..Default::default()
        };
        let app = &mut self.app;
        craft::ui::shell::embed::before_input(&self.ctx, |out| app.frame_input(out));

        crate::testing::app_frame(app, &self.ctx, raw)
    }

    /// AppKit chose an item by its key equivalent `sc`. A paste the menu asks the host for
    /// (`ViewportCommand::RequestPaste`) comes in the next frame, as egui-winit sends it for a
    /// clipboard that holds a picture and no text.
    fn key_equivalent(&mut self, sc: &str) -> egui::FullOutput {
        self.fake.report(MenuEvent::Key(Chord::parse(sc).unwrap()));
        let out = self.frame(vec![]);
        let asked = out.viewport_output.values().any(|v| v.commands.contains(&egui::ViewportCommand::RequestPaste));
        if !asked {
            return out;
        }
        let picture = std::sync::Arc::new(egui::ColorImage::filled([1, 1], egui::Color32::BLUE));
        self.frame(vec![Event::PasteImage(picture)])
    }

    /// Where the document's point (`x`, `y`) is on the screen.
    fn on_canvas(&self, x: f64, y: f64) -> Pos2 {
        self.app.canvas_view().expect("the canvas was shown").to_screen(vectorcraft_geom::Point::new(x, y))
    }

    /// The canvas's rectangle on the screen.
    fn canvas_rect(&self) -> Rect {
        self.app.canvas_view().expect("the canvas was shown").rect
    }

    /// Run `n` frames, each with the events that `step` returns (`step` may act on the app first),
    /// and return the number of frames in which the app read the menus.
    fn reads(&mut self, n: usize, mut step: impl FnMut(&mut Mac, usize) -> Vec<Event>) -> usize {
        let synced = self.fake.synced();
        for i in 0..n {
            let events = step(self, i);
            self.frame(events);
        }
        self.fake.synced() - synced
    }

    /// Press on the selected rectangle and move the pointer 8 points to the right: the Selection
    /// tool's move begins. Returns where the press was.
    fn start_move(&mut self) -> Pos2 {
        let start = self.on_canvas(38.5, 40.0);
        self.frame(vec![Event::PointerMoved(start), click(start, true)]);
        self.frame(vec![Event::PointerMoved(start + vec2(8.0, 0.0))]);
        assert!(self.app.session.in_interaction(), "the move began");
        start
    }

    /// Open the Rotate dialog with Preview on: the rotation previews on the canvas.
    fn rotate_preview(&mut self) {
        menus::invoke(&mut self.app, "object.rotate", json!({}));
        self.frame(vec![]);
        crate::tests_synthetic::control(&mut self.app, &self.ctx, "ui.dialog.set", json!({"field": "preview", "value": true}));
        self.frame(vec![]);
        assert!(self.app.session.in_interaction(), "the rotation previews");
    }

    /// Set `n` new angles in the open dialog through the control port, one a frame, and return the
    /// number of frames in which the app read the menus.
    fn angles(&mut self, n: usize) -> usize {
        self.reads(n, |m, i| {
            crate::tests_synthetic::control(&mut m.app, &m.ctx, "ui.dialog.set", json!({"field": "angle", "value": 10 * (i + 1)}));
            vec![]
        })
    }
}

fn click(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

/// Run `drag` on a new app with the fake native menu, and run it again on another new app until
/// the plug-in and font generations hold still through a run (at most 20 runs). Return that app
/// and what `drag` returned. The generations are process-wide and only grow. A test running
/// alongside can bump them (a font it adds, the installed fonts' scan ending), and the app then
/// reads the menus again, so every step whose menu reads a test counts runs inside `drag`.
fn undisturbed<R>(drag: impl Fn(&mut Mac) -> R) -> (Mac, R) {
    let fonts = vectorcraft_text::FontDb::global();
    let generations = || (crate::menus::plugin_revision(), fonts.generation(), fonts.installed_fonts_ready());
    for _ in 0..20 {
        let mut m = Mac::new(true);
        let before = generations();
        let r = drag(&mut m);
        if generations() == before {
            return (m, r);
        }
    }
    panic!("tests running alongside kept bumping the generations");
}

/// Each step of a drag previews the move as a new revision: the menus aren't read again for it
/// while the button is held, and are read when the button is released, with Undo naming the move.
#[test]
fn a_drag_reads_the_menus_when_it_is_released() {
    let (m, (read, synced, ended, released)) = undisturbed(|m| {
        let start = m.start_move();
        let read = m.reads(6, |_, i| vec![Event::PointerMoved(start + vec2(8.0 * (i + 2) as f32, 0.0))]);
        let synced = m.fake.synced();
        m.frame(vec![click(start + vec2(56.0, 0.0), false)]);
        (read, synced, !m.app.session.in_interaction(), m.fake.synced())
    });
    assert_eq!(read, 0, "the menus weren't read at any step of the drag");
    assert!(ended);
    assert_eq!(released, synced + 1, "the menus were read on the release");
    assert_eq!(find(&m.fake.last(), "edit.undo").label, "Undo Move");
}

/// Escape ends a Rectangle tool drag while its button is still held: the menus are read then, with
/// the preview's rectangle gone (nothing selected, so Cut is off), and not at the steps before.
#[test]
fn escape_during_a_drag_reads_the_menus() {
    let (m, read) = undisturbed(|m| {
        m.app.run("select.none", json!({})).unwrap();
        m.app.run("tool.select", json!({"tool": "rectangle"})).unwrap();
        let start = m.on_canvas(200.0, 150.0);
        m.frame(vec![Event::PointerMoved(start), click(start, true)]);
        m.frame(vec![Event::PointerMoved(start + vec2(10.0, 10.0))]);
        assert!(m.app.session.in_interaction(), "the rectangle is being drawn");
        let read = m.reads(6, |_, i| vec![Event::PointerMoved(start + vec2(10.0, 10.0) * (i + 2) as f32)]);
        let cut = find(&m.fake.last(), "edit.cut").enabled;
        let synced = m.fake.synced();
        m.frame(vec![Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
        (read, cut, synced, !m.app.session.in_interaction(), m.fake.synced())
    });
    let (read, cut, synced, dropped, escaped) = read;
    assert_eq!(read, 0, "the menus weren't read at any step of the drag");
    assert!(cut, "the preview's rectangle is selected");
    assert!(dropped, "Escape dropped the rectangle");
    assert_eq!(escaped, synced + 1, "the menus were read");
    assert!(!find(&m.fake.last(), "edit.cut").enabled, "nothing is selected");
}

/// A move dragged off the canvas onto a panel stops being a move with its button still held
/// (`canvas::drag_art_out`): the menus are read then, without a release or a key, and Undo still
/// names the step before the drag.
#[test]
fn a_move_dragged_onto_a_panel_reads_the_menus() {
    // The whole drag, the wait over the panel and the release run undisturbed: a generation bumped
    // by a test running alongside would read the menus once more.
    let (m, (undo, read, synced, waits, over, dropped, released)) = undisturbed(|m| {
        let undo = find(&m.fake.last(), "edit.undo").label.clone();
        let start = m.start_move();
        let read = m.reads(4, |_, i| vec![Event::PointerMoved(start + vec2(8.0 * (i + 2) as f32, 0.0))]);
        let synced = m.fake.synced();
        let panel = Pos2::new(m.canvas_rect().right() + 40.0, start.y);
        // Over a panel the move waits: the menus keep the interaction's state.
        m.frame(vec![Event::PointerMoved(panel)]);
        let (waits, over) = (m.app.session.in_interaction(), m.fake.synced());
        // Released over the panel, the move is dropped and the menus are read.
        m.frame(vec![click(panel, false)]);
        (undo, read, synced, waits, over, !m.app.session.in_interaction(), m.fake.synced())
    });
    assert_eq!(read, 0, "the menus weren't read at any step of the drag");
    assert!(waits, "the move waits over the panel");
    assert_eq!(over, synced, "the menus weren't read");
    assert!(dropped, "the move was dropped");
    assert_eq!(released, synced + 1, "the menus were read");
    assert_eq!(find(&m.fake.last(), "edit.undo").label, undo);
}

/// A dialog previews each angle that the control port sets, and the menus aren't read for the
/// angles: `ui.dialog.cancel` on the control port ends the preview, and the menus are read then,
/// without a release or a key.
#[test]
fn a_control_port_cancel_of_a_dialog_preview_reads_the_menus() {
    let (_m, (read, synced, closed, after)) = undisturbed(|m| {
        m.rotate_preview();
        let read = m.angles(4);
        let synced = m.fake.synced();
        crate::tests_synthetic::control(&mut m.app, &m.ctx, "ui.dialog.cancel", json!({}));
        m.frame(vec![]);
        (read, synced, !m.app.session.in_interaction() && !m.app.any_dialog_open(), m.fake.synced())
    });
    assert_eq!(read, 0, "the menus weren't read while the dialog previewed the angles");
    assert!(closed);
    assert_eq!(after, synced + 1, "the menus were read");
}

/// OK applies the rotation a dialog previews as one undo step: the menus are read then, with Undo
/// naming it, and not for the angles previewed before.
#[test]
fn applying_a_dialog_preview_reads_the_menus() {
    let (m, (read, synced, closed, after)) = undisturbed(|m| {
        m.rotate_preview();
        let read = m.angles(4);
        let synced = m.fake.synced();
        crate::tests_synthetic::control(&mut m.app, &m.ctx, "ui.dialog.confirm", json!({}));
        // The OK enters the next frame's pass and applies there; the dialog's layer and the preview's interaction are
        // gone two frames later.
        m.frame(vec![]);
        m.frame(vec![]);
        m.frame(vec![]);
        (read, synced, !m.app.session.in_interaction() && !m.app.any_dialog_open(), m.fake.synced())
    });
    assert_eq!(read, 0, "the menus weren't read while the dialog previewed the angles");
    assert!(closed);
    assert_eq!(after, synced + 1, "the menus were read");
    assert_eq!(find(&m.fake.last(), "edit.undo").label, "Undo Rotate");
}

/// Fonts that come in after the menus were read (the installed fonts' scan ending, #5483) make the
/// next frame read them again, with no click or key, so Type › Font lists them.
#[test]
fn fonts_that_load_read_the_menus_again() {
    use vectorcraft_text::test_fonts as t;
    let db = vectorcraft_text::FontDb::global();
    // A test font no other test loaded yet, so adding it changes the fonts.
    let fonts = [t::vertical_font(true), t::two_axis_font(), t::variable_font()];
    let mut m = Mac::new(true);
    let synced = m.fake.synced();
    assert!(fonts.into_iter().flatten().any(|data| db.add_font(data) > 0), "one of the test fonts is new to the font database");
    m.frame(vec![]);
    assert!(m.fake.synced() > synced, "the menus were read again");
}

/// The font menu previews each font it highlights on the selected type, in one interaction: the
/// menus are read as the preview begins and as it ends, and not for the fonts between.
#[test]
fn the_font_menu_preview_reads_the_menus_when_it_ends() {
    use craft::behavior::Preview;

    use crate::font_menu::{apply_preview, end_previews};
    let owner = egui::Id::unique("font-menu-preview-test");
    // The app reads the menus after each step as at the end of a frame.
    let read = |m: &mut Mac, step: &dyn Fn(&mut VectorcraftApp)| {
        let synced = m.fake.synced();
        step(&mut m.app);
        menus::native::sync(&mut m.app, &m.ctx);
        m.fake.synced() - synced
    };
    let (_m, read) = undisturbed(|m| {
        m.app.run("select.none", json!({})).unwrap();
        m.app.run("text.create", json!({"x": 50, "y": 50, "text": "Hi"})).unwrap();
        m.frame(vec![]);
        let shows = |font: &'static str| move |app: &mut VectorcraftApp| apply_preview(app, owner, Preview::Shows(font.into()));
        assert_eq!(read(m, &shows("Source Serif 4")), 1, "the menus were read as the preview began");
        let between = read(m, &shows("Source Sans 3")) + read(m, &shows("Inter"));
        let ended = read(m, &|app| end_previews(app, &[owner]));
        (between, ended, !m.app.session.in_interaction())
    });
    let (between, ended, done) = read;
    assert_eq!(between, 0, "the menus weren't read for the fonts after the first");
    assert_eq!(ended, 1, "the menus were read as the preview ended");
    assert!(done);
}

/// #5159: ⌘V chosen by the menu bar's key equivalent pastes a picture another app copied (a
/// screenshot): AppKit gives the key-down to the menu, so the window never reports the key, and
/// the release the key equivalent makes is what tells the paste. Each press pastes once.
#[test]
fn a_key_equivalent_paste_pastes_a_picture_from_another_app() {
    use vectorcraft_doc::NodeKind;
    use vectorcraft_engine::cmd::clipboard::PNG;
    let (mut app, board) = super::sysclip::app();
    super::sysclip::copy_elsewhere(&board, vec![(PNG, super::sysclip::blue_png())]);
    let fake = Fake::default();
    app.services.native_menu = Some(NativeMenu::new(Box::new(fake.clone())));
    let mut m = Mac { app, fake, ctx: crate::testing::ui_context(), time: 0.0 };
    for _ in 0..3 {
        m.frame(vec![]);
    }
    let images = |m: &Mac| {
        let d = &m.app.session.active().unwrap().doc;
        d.layers[0].children().unwrap().iter().filter(|n| matches!(n.kind, NodeKind::Image(_))).count()
    };
    for n in 1..=2 {
        m.key_equivalent("Cmd+V");
        m.frame(vec![]);
        m.frame(vec![]);
        assert_eq!(images(&m), n, "press {n} pastes one picture");
    }
}
