//! The panel harness: one panel drawn in the framework `Fixture`, its controls found by accessible name.
//!
//! A command a control fires goes through the same `shortcuts::run_fired` the window runs, so a click here reaches the
//! app as it does in production.

use std::{
    cell::{RefCell, RefMut},
    rc::Rc,
};

use craft_ui::runtime::frame::take_output;
use craft_ui::testing::{Fixture, Target};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, Ui, Vec2, vec2};

use crate::{VectorcraftApp, shortcuts};

/// The room every panel is drawn in: narrow like a docked panel, tall enough for its last section.
pub(crate) const ROOM: Vec2 = vec2(300.0, 1600.0);

/// One panel (or a few drawn together) over an app, driven by accessible names.
pub(crate) struct Panel {
    fixture: Fixture,
    app: Rc<RefCell<VectorcraftApp>>,
}

impl Panel {
    /// Draws `draw` over `app` and settles its first frames.
    pub(crate) fn new(app: VectorcraftApp, draw: fn(&mut VectorcraftApp, &mut Ui)) -> Self {
        let app = Rc::new(RefCell::new(app));
        let shown = Rc::clone(&app);
        let mut fixture = Fixture::new(move |ui| draw(&mut shown.borrow_mut(), ui));
        fixture.room(ROOM);
        let mut panel = Panel { fixture, app };
        panel.frame();
        panel.frame();
        panel
    }

    /// The app. Drop the borrow before the next driver call: the panel borrows the app to draw.
    pub(crate) fn app(&self) -> RefMut<'_, VectorcraftApp> {
        self.app.borrow_mut()
    }

    /// The egui context of the fixture.
    pub(crate) fn ctx(&self) -> &egui::Context {
        self.fixture.ctx()
    }

    /// One frame with no input.
    pub(crate) fn frame(&mut self) {
        self.fixture.frame(Vec::new());
        self.run_fired();
    }

    /// Runs the commands the frames since the last call fired.
    fn run_fired(&mut self) {
        let ctx = self.fixture.ctx().clone();
        let fired = take_output(&ctx).fired;
        let mut app = self.app.borrow_mut();
        for one in &fired {
            // PLAN: assumed `run_fired(app, ctx, &Fired)` takes one fired command (kb.C21).
            shortcuts::run_fired(&mut app, &ctx, one);
        }
    }

    /// What a driver call ends with: the fired commands run, and a frame shows their effect.
    fn after(&mut self) {
        self.run_fired();
        self.fixture.frame(Vec::new());
        self.run_fired();
    }

    /// The names the latest frame shows.
    fn names(&self) -> Vec<String> {
        let tree = self.fixture.tree();
        tree.keys().into_iter().filter_map(|key| tree.name_of(key)).map(str::to_owned).collect()
    }

    /// The control named `name` in the latest frame.
    fn look(&self, name: &str) -> Target {
        self.fixture.by_label(name).unwrap_or_else(|| panic!("no {name:?} in {:?}", self.names()))
    }

    /// A fresh frame, then the control named `name`.
    fn find(&mut self, name: &str) -> Target {
        self.fixture.frame(Vec::new());
        self.look(name)
    }

    /// Clicks the control named `name`.
    pub(crate) fn click(&mut self, name: &str) {
        let target = self.find(name);
        self.fixture.click(&target);
        self.after();
    }

    /// Clicks the control named `name` with `mods` held.
    pub(crate) fn click_with(&mut self, name: &str, mods: Modifiers) {
        let target = self.find(name);
        self.fixture.hold(mods).click(&target).hold(Modifiers::NONE);
        self.after();
    }

    /// Clicks at `pos`, where no control needs to be.
    pub(crate) fn click_at(&mut self, pos: Pos2) {
        self.fixture.click_at(pos);
        self.after();
    }

    /// Presses on the control named `name` and keeps the button down until [`Panel::release`].
    pub(crate) fn press(&mut self, name: &str) {
        let target = self.find(name);
        self.fixture.press(&target);
        self.after();
    }

    /// Releases the button a [`Panel::press`] held, where the pointer is.
    pub(crate) fn release(&mut self) {
        let at = self.fixture.ctx().input(|i| i.pointer.latest_pos()).unwrap_or_default();
        self.fixture.release_at(at);
        self.after();
    }

    /// Clicks the field named `name`, replaces its text with `text` and presses Enter.
    pub(crate) fn type_into(&mut self, name: &str, text: &str) {
        self.type_into_with(name, text, Modifiers::NONE);
    }

    /// [`Panel::type_into`] with `mods` held for the Enter.
    pub(crate) fn type_into_with(&mut self, name: &str, text: &str, mods: Modifiers) {
        let target = self.find(name);
        self.fixture.click(&target).key(Key::A, Modifiers::COMMAND).type_text(text).key(Key::Enter, mods);
        self.after();
    }

    /// Types `text` into the focused field.
    pub(crate) fn type_text(&mut self, text: &str) {
        self.fixture.type_text(text);
        self.after();
    }

    /// A key press and release.
    pub(crate) fn key(&mut self, key: Key, mods: Modifiers) {
        self.fixture.key(key, mods);
        self.after();
    }

    /// Drags from the control named `from` to the one named `to`.
    pub(crate) fn drag(&mut self, from: &str, to: &str, mods: Modifiers) {
        let (from, to) = (self.find(from), self.look(to));
        self.fixture.hold(mods).drag(&from, to.center() - from.center()).hold(Modifiers::NONE);
        self.after();
    }

    /// Two clicks on the control named `name`, close enough in time to be a double click.
    pub(crate) fn double_click(&mut self, name: &str) {
        let target = self.find(name);
        self.fixture.click(&target).click(&target);
        self.after();
    }

    /// A right click on the control named `name`.
    pub(crate) fn secondary_click(&mut self, name: &str) {
        let at = self.find(name).center();
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE };
        self.fixture.frame(vec![Event::PointerMoved(at)]);
        self.fixture.frame(vec![button(true)]);
        self.fixture.frame(vec![button(false)]);
        self.after();
    }

    /// Lets `seconds` pass.
    pub(crate) fn advance(&mut self, seconds: f64) {
        self.fixture.advance(seconds);
    }

    /// Whether the latest frame shows a control named `name`.
    pub(crate) fn shows(&self, name: &str) -> bool {
        self.fixture.by_label(name).is_some()
    }

    /// Whether the control named `name` takes input.
    pub(crate) fn enabled(&self, name: &str) -> bool {
        !self.fixture.tree().is_disabled_key(self.look(name).node)
    }

    /// Where the control named `name` is.
    pub(crate) fn rect(&self, name: &str) -> Rect {
        self.look(name).rect
    }

    /// The value text of the control named `name`.
    pub(crate) fn value(&self, name: &str) -> Option<String> {
        self.fixture.tree().value_of(self.look(name).node).map(str::to_owned)
    }

    /// The toggle state of the control named `name`.
    // PLAN: assumed the facade re-exports the behavior crate's `Mixed` as `craft::behavior::Mixed`.
    pub(crate) fn toggled(&self, name: &str) -> Option<craft::behavior::Mixed<bool>> {
        self.fixture.tree().toggled_of(self.look(name).node)
    }

    /// Whether the control named `name` has the focus.
    pub(crate) fn focused(&self, name: &str) -> bool {
        self.fixture.tree().focused_key() == self.look(name).node
    }
}
