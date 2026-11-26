//! The panel harness: one panel drawn in the framework `Fixture`, its controls found by accessible name.
//!
//! Each frame takes the app's frame input as the hosts give it ([`VectorcraftApp::frame_input`]): the commands the
//! frames before fired run first, and the runtime gets the app's command catalog, states and host, so a control bound
//! to a command shows the command's label and a click here reaches the app as it does in production.

use std::{
    cell::{RefCell, RefMut},
    rc::Rc,
};

use craft::behavior::PartRef;
use craft::ui::runtime::frame::take_output;
use craft::ui::testing::pointer::part_rect_of_node;
use craft::ui::testing::{Fixture, Target};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, Ui, Vec2, vec2};

use crate::VectorcraftApp;

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
        // A host the test installed on the app's window (a scripted file service) goes with it to the fixture's.
        if let Some(host) = app.borrow().ui_ctx.as_ref().and_then(craft::host::Host::try_of_ctx) {
            craft::host::Host::clone(&host).install(fixture.ctx());
        }
        // The fixture's context is the app's window, as a host's is.
        app.borrow_mut().ui_ctx = Some(fixture.ctx().clone());
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
        self.frame_with(Vec::new());
    }

    /// One frame with `events`, its input taken from the app first.
    fn frame_with(&mut self, events: Vec<Event>) {
        self.take_input();
        self.fixture.frame(events);
        self.take_input();
    }

    /// The app's frame input for the next frame, given as the hosts give it: the commands the frames since the last
    /// call fired run first.
    fn take_input(&mut self) {
        let input = self.app.borrow_mut().frame_input(take_output(self.fixture.ctx()));
        self.fixture.root(input.root).commands(input.commands);
        if let Some(host) = input.host {
            self.fixture.host(host);
        }
    }

    /// What a driver call ends with: the fired commands run, and a frame shows their effect.
    fn after(&mut self) {
        self.frame();
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
        self.frame();
        self.look(name)
    }

    /// Clicks the control named `name`.
    pub(crate) fn click(&mut self, name: &str) {
        let target = self.find(name);
        self.fixture.click(&target);
        self.after();
    }

    /// Clicks the item named `name` of the open menu, where a control of the panel may have the same name.
    pub(crate) fn click_menu_item(&mut self, name: &str) {
        let tree = self.fixture.tree();
        let item = self.fixture.by_role(craft::ui::contract::Role::MENU_ITEM).into_iter().find(|t| tree.name_of(t.node) == Some(name));
        let item = item.unwrap_or_else(|| panic!("no menu item {name:?} in {:?}", self.names()));
        self.fixture.click(&item);
        self.after();
    }

    /// Whether the item named `name` of the open menu takes input, where a control of the panel may have the same name.
    pub(crate) fn menu_item_enabled(&self, name: &str) -> bool {
        let tree = self.fixture.tree();
        let item = self.fixture.by_role(craft::ui::contract::Role::MENU_ITEM).into_iter().find(|t| tree.name_of(t.node) == Some(name));
        let item = item.unwrap_or_else(|| panic!("no menu item {name:?} in {:?}", self.names()));
        !tree.is_disabled_key(item.node)
    }

    /// Clicks the control of `role` named `name`, where nodes of other roles share the name (a section's region,
    /// heading and disclosure button).
    pub(crate) fn click_role(&mut self, name: &str, role: craft::ui::contract::Role) {
        self.click_role_with(name, role, Modifiers::NONE);
    }

    /// [`Panel::click_role`] with `mods` held.
    pub(crate) fn click_role_with(&mut self, name: &str, role: craft::ui::contract::Role, mods: Modifiers) {
        self.frame();
        let target = self.look_role(name, role);
        self.fixture.hold(mods).click(&target).hold(Modifiers::NONE);
        self.after();
    }

    /// The control of `role` named `name` in the latest frame, where nodes of other roles share the name.
    fn look_role(&self, name: &str, role: craft::ui::contract::Role) -> Target {
        let tree = self.fixture.tree();
        let target = self.fixture.by_role(role).into_iter().find(|t| tree.name_of(t.node) == Some(name));
        target.unwrap_or_else(|| panic!("no {role:?} named {name:?} in {:?}", self.names()))
    }

    /// Gives the control named `name` the keyboard focus, as a screen reader's Focus action does.
    pub(crate) fn focus(&mut self, name: &str) {
        let target = self.find(name);
        self.fixture.access(craft::ui::testing::focus::AccessActionKind::Focus, &target);
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
        self.click_at_with(pos, Modifiers::NONE);
    }

    /// Clicks at `pos` with `mods` held, where no control needs to be (a row's room past its link).
    pub(crate) fn click_at_with(&mut self, pos: Pos2, mods: Modifiers) {
        self.fixture.hold(mods).click_at(pos).hold(Modifiers::NONE);
        self.after();
    }

    /// Presses on the control named `name` and keeps the button down until [`Panel::release`].
    pub(crate) fn press(&mut self, name: &str) {
        let target = self.find(name);
        self.fixture.press(&target);
        self.after();
    }

    /// [`Panel::drag_hold`] on the control named `name` whose role is `role`, where controls of other roles share
    /// the name (a slider beside its number box).
    pub(crate) fn drag_hold_role(&mut self, name: &str, role: craft::ui::contract::Role, by: Vec2) {
        self.frame();
        let target = self.look_role(name, role);
        self.fixture.drag_hold_at(target.center(), by);
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
        let to = self.find(to).center();
        self.drag_to(from, to, mods);
    }

    /// Drags from the control named `from` and releases at `to`.
    pub(crate) fn drag_to(&mut self, from: &str, to: Pos2, mods: Modifiers) {
        let from = self.find(from).center();
        self.drag_between(from, to, mods);
    }

    /// Drags from `from` and releases at `to` with `mods` held, where no control needs to be at `from` (a row's room
    /// past its link).
    pub(crate) fn drag_between(&mut self, from: Pos2, to: Pos2, mods: Modifiers) {
        self.fixture.hold(mods).drag_at(from, to - from).hold(Modifiers::NONE);
        self.after();
    }

    /// Clicks the pointer-only part `part` (a row's thumbnail, a step arrow) of the control named `name`.
    pub(crate) fn click_part(&mut self, name: &str, part: impl Into<PartRef>, mods: Modifiers) {
        let (target, part) = (self.find(name), part.into());
        let at = part_rect_of_node(self.ctx(), target.node.into(), part).unwrap_or_else(|| panic!("{name:?} has no part {part:?}")).center();
        self.fixture.hold(mods).click_at(at).hold(Modifiers::NONE);
        self.after();
    }

    /// Draws the panel in `room` from the next frame on.
    pub(crate) fn room(&mut self, room: Vec2) -> &mut Self {
        self.fixture.room(room);
        self
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
        self.frame_with(vec![Event::PointerMoved(at)]);
        self.frame_with(vec![button(true)]);
        self.frame_with(vec![button(false)]);
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

    /// Whether some of the control named `name` can be seen in the latest frame.
    pub(crate) fn in_view(&self, name: &str) -> bool {
        self.fixture.visible(&self.look(name)).is_some()
    }

    /// Whether the control named `name` takes input.
    pub(crate) fn enabled(&self, name: &str) -> bool {
        !self.fixture.tree().is_disabled_key(self.look(name).node)
    }

    /// Where the control named `name` is.
    pub(crate) fn rect(&self, name: &str) -> Rect {
        self.look(name).rect
    }

    /// One frame with no input at `pixels_per_point`; the shapes it painted, nested shapes flattened, and the images of
    /// the textures it made.
    pub(crate) fn paint_at(&mut self, pixels_per_point: f32) -> (Vec<egui::Shape>, Vec<(egui::TextureId, egui::ImageData)>) {
        self.fixture.pixels_per_point(pixels_per_point).record_paint();
        self.take_input();
        self.fixture.frame(Vec::new());
        let Some(mut paint) = self.fixture.take_paint() else { return (Vec::new(), Vec::new()) };
        let images = paint.textures_delta.set.iter().flat_map(|(id, deltas)| deltas.iter().map(move |delta| (*id, delta.image.clone()))).collect();
        // The test reads the images; no renderer applies them.
        paint.textures_delta.clear();
        let mut shapes = Vec::new();
        let mut open: Vec<egui::Shape> = paint.shapes.into_iter().map(|clipped| clipped.shape).collect();
        while let Some(shape) = open.pop() {
            match shape {
                egui::Shape::Vec(nested) => open.extend(nested),
                shape => shapes.push(shape),
            }
        }
        (shapes, images)
    }

    /// Where the control of `role` named `name` is, where nodes of other roles share the name.
    pub(crate) fn rect_role(&self, name: &str, role: craft::ui::contract::Role) -> Rect {
        self.look_role(name, role).rect
    }

    /// The value text of the control named `name`.
    pub(crate) fn value(&self, name: &str) -> Option<String> {
        self.fixture.tree().value_of(self.look(name).node).map(str::to_owned)
    }

    /// The value text of the control of `role` named `name`, where nodes of other roles share the name (a slider
    /// beside its number box).
    pub(crate) fn value_role(&self, name: &str, role: craft::ui::contract::Role) -> Option<String> {
        self.fixture.tree().value_of(self.look_role(name, role).node).map(str::to_owned)
    }

    /// The description of the control named `name` (a list row's trailing texts).
    pub(crate) fn description(&self, name: &str) -> Option<String> {
        self.fixture.tree().description_of(self.look(name).node).map(str::to_owned)
    }

    /// The value texts of the latest frame's controls, in tree order.
    pub(crate) fn values(&self) -> Vec<String> {
        let tree = self.fixture.tree();
        tree.keys().into_iter().filter_map(|key| tree.value_of(key)).map(str::to_owned).collect()
    }

    /// Whether a control in the latest frame holds the value text `value`.
    pub(crate) fn shows_value(&self, value: &str) -> bool {
        self.shows_value_with(|shown| shown == value)
    }

    /// Whether a control in the latest frame holds a value text that `matches`.
    pub(crate) fn shows_value_with(&self, matches: impl Fn(&str) -> bool) -> bool {
        let tree = self.fixture.tree();
        tree.keys().into_iter().any(|key| tree.value_of(key).is_some_and(&matches))
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
