//! The test driver of the 3D dialog: the whole app frame in a framework fixture, so the dialog host and the
//! canvas run once per frame after the shell pass, as in the app.
use std::{
    cell::{RefCell, RefMut},
    rc::Rc,
};

use craft::ui::dialog::StackDialog as _;
use craft::ui::runtime::frame::take_output;
use craft::ui::testing::{Fixture, Target};
use egui::vec2;
use serde_json::Value;

use super::super::three_d;
use crate::VectorcraftApp;

/// An app whose 3D dialog a test drives by the controls' names, as a user would.
pub(crate) struct ThreeD {
    /// The frame driver: clicks, drags and keys go through it.
    pub(crate) fixture: Fixture,
    app: Rc<RefCell<VectorcraftApp>>,
}

impl ThreeD {
    /// Shows `app`'s whole frame in a 1440 × 900 room.
    pub(crate) fn new(app: VectorcraftApp) -> Self {
        let app = Rc::new(RefCell::new(app));
        let shown = app.clone();
        // The window's frame: the background work, then the whole layout, as the host runs them.
        let mut fixture = Fixture::new(move |ui| {
            let mut app = shown.borrow_mut();
            app.tick(ui.ctx());
            app.ui(ui);
        });
        fixture.room(vec2(1440.0, 900.0));
        Self { fixture, app }
    }

    /// The app's frame input, given as the hosts give it: the commands the frames since the last call fired run, and
    /// the runtime gets the app's command catalog, states and host.
    fn take_input(&mut self) {
        let input = self.app.borrow_mut().frame_input(take_output(self.fixture.ctx()));
        self.fixture.root(input.root).commands(input.commands);
        if let Some(host) = input.host {
            self.fixture.host(host);
        }
    }

    /// Frames until the app settles, each with the app's frame input.
    pub(crate) fn settle(&mut self) {
        self.take_input();
        self.fixture.settle();
        self.take_input();
    }

    /// The app, for commands and reads between frames.
    pub(crate) fn app(&self) -> RefMut<'_, VectorcraftApp> {
        self.app.borrow_mut()
    }

    /// Opens the 3D dialog with the `effect.dialog` params `p` and lets it settle.
    pub(crate) fn open(&mut self, p: Value) {
        if let Err(e) = three_d::open(&mut self.app(), &p) {
            panic!("the dialog did not open: {e}");
        }
        self.settle();
    }

    /// Sets a field as automation does and lets the preview settle; a refusal fails the test.
    pub(crate) fn set(&mut self, field: &str, value: Value) {
        let ctx = self.fixture.ctx().clone();
        let done = three_d::with_fields(&mut self.app(), |d| d.set_json_field(&ctx, field, value));
        if let Some(Err(e)) = done {
            panic!("{field} refused: {e}");
        }
        self.settle();
    }

    /// A copy of the dialog's values.
    pub(crate) fn fields(&self) -> craft::ui::patterns::form_dialogs::DialogFields {
        three_d::with_fields(&mut self.app(), |d| d.fields().clone()).unwrap_or_default()
    }

    /// The dialog's values as JSON, as automation lists them.
    pub(crate) fn json(&self, key: &str) -> Value {
        three_d::with_fields(&mut self.app(), |d| d.json_fields()).flatten().and_then(|v| v.get(key).cloned()).unwrap_or(Value::Null)
    }

    /// The control named `name`.
    pub(crate) fn target(&self, name: &str) -> Target {
        self.fixture.by_label(name).unwrap_or_else(|| panic!("no control named {name:?}"))
    }

    /// Presses the control named `name`.
    pub(crate) fn press(&mut self, name: &str) {
        let target = self.target(name);
        self.fixture.click(&target);
    }

    /// The value the control named `name` shows.
    pub(crate) fn shown(&self, name: &str) -> Option<String> {
        let target = self.target(name);
        self.fixture.tree().value_of(target.node).map(str::to_owned)
    }

    /// Opens the select named `name`, which shows `from`, and picks its entry `to`.
    pub(crate) fn choose(&mut self, name: &str, from: &str, to: &str) {
        assert_eq!(self.shown(name).as_deref(), Some(from), "{name} shows {from}");
        self.press(name);
        self.press(to);
        self.settle();
    }

    /// Presses OK and lets the held answer finish.
    pub(crate) fn ok(&mut self) {
        self.press("OK");
        self.settle();
    }

    /// Presses Cancel.
    pub(crate) fn cancel(&mut self) {
        self.press("Cancel");
        self.settle();
    }

    /// The surfaces Map Art lists, while its page shows.
    pub(crate) fn surfaces(&self) -> Option<std::sync::Arc<Vec<vectorcraft_effects::MapSurface>>> {
        three_d::read(&self.app().three_d_dialog.as_ref()?.data).surfaces.clone()
    }
}
