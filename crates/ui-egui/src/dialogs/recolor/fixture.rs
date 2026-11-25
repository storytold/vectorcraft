//! The one test harness of Recolor Artwork: an app whose window shows its dialog host in a framework
//! fixture, each frame the host's pass and then the app's dialog pump, as the app runs them. Controls
//! are found by their accessible names; the dialog's fields are read and set as automation does.

use std::cell::{RefCell, RefMut};
use std::rc::Rc;

use craft::ui::dialog::{DialogHost, DialogId, HeldOk, dialog_id};
use craft::ui::runtime::access::Role;
use craft::ui::testing::pointer::part_rect_of_node;
use craft::ui::testing::{Fixture, Room, Target};
use egui::{Event, Modifiers, PointerButton, Pos2, Rect};
use serde_json::{Value, json};

use super::{Fields, GROUP_ACTION, KIND, RecolorDialog};
use crate::VectorcraftApp;

/// Recolor Artwork over an app, in a window-sized fixture.
pub(crate) struct Recolor {
    fixture: Fixture,
    app: Rc<RefCell<VectorcraftApp>>,
}

impl Recolor {
    /// `app` in a fixture whose frames run the window's dialog host and then the app's dialog pump.
    pub(crate) fn new(app: VectorcraftApp) -> Self {
        let app = Rc::new(RefCell::new(app));
        let shared = Rc::clone(&app);
        let mut fixture = Fixture::new(move |ui| {
            let mut app = shared.borrow_mut();
            if let Some(dialogs) = app.dialogs() {
                dialogs.with(|host| host.run(ui));
            }
            let ctx = ui.ctx().clone();
            crate::dialogs::host::pump(&mut app, &ctx);
        });
        fixture.in_room(Room::Window);
        app.borrow_mut().ui_ctx = Some(fixture.ctx().clone());
        Recolor { fixture, app }
    }

    /// The app, for commands and the document; drop it before the next frame.
    pub(crate) fn app(&self) -> RefMut<'_, VectorcraftApp> {
        self.app.borrow_mut()
    }

    /// The dialog's id on the host.
    fn id() -> DialogId {
        dialog_id(KIND)
    }

    /// Runs `f` on the window's dialog host with the fixture's context; `None` without a window.
    pub(crate) fn host<R>(&self, f: impl FnOnce(&mut DialogHost, &egui::Context) -> R) -> Option<R> {
        let dialogs = self.app.borrow().dialogs()?;
        let ctx = self.fixture.ctx().clone();
        Some(dialogs.with(|host| f(host, &ctx)))
    }

    /// Runs `f` on the open dialog with the app; `None` while it is not open.
    pub(crate) fn with_dialog<R>(&self, f: impl FnOnce(&VectorcraftApp, &mut RecolorDialog) -> R) -> Option<R> {
        let app = self.app.borrow();
        let dialogs = app.dialogs()?;
        dialogs.with(|host| host.get_mut::<RecolorDialog>().map(|d| f(&app, d)))
    }

    /// Whether Recolor Artwork is open.
    pub(crate) fn is_open(&self) -> bool {
        self.host(|host, _| host.is_open::<RecolorDialog>()).unwrap_or(false)
    }

    /// The dialog's fields, the `__` marks included; empty while it is not open.
    pub(crate) fn fields(&self) -> Fields {
        self.with_dialog(|_, d| d.fields.clone()).unwrap_or_default()
    }

    /// Field `k`.
    pub(crate) fn field(&self, k: &str) -> Option<Value> {
        self.fields().fields.get(k).cloned()
    }

    /// The rows.
    pub(crate) fn rows(&self) -> Vec<Value> {
        super::rows(&self.fields())
    }

    /// One frame without input.
    pub(crate) fn frame(&mut self) -> &mut Self {
        self.fixture.frame(Vec::new());
        self
    }

    /// One frame with `events` and `modifiers` held.
    pub(crate) fn frame_with(&mut self, events: Vec<Event>, modifiers: Modifiers) -> &mut Self {
        self.fixture.hold(modifiers);
        self.fixture.frame(events);
        self.fixture.hold(Modifiers::NONE);
        self
    }

    /// Frames until the window comes to rest.
    pub(crate) fn settled(&mut self) -> &mut Self {
        self.fixture.settle();
        self
    }

    /// Sets `field` as `ui.dialog.set` does, before the next frame; a refused value leaves the
    /// dialog as it was.
    pub(crate) fn set(&mut self, field: &str, value: Value) -> Result<(), String> {
        self.host(|host, ctx| host.set_json_field(ctx, Self::id(), field, value)).unwrap_or_else(|| Err("no window".into()))
    }

    /// A Color Groups button as `ui.dialog.set` of `groupAction` gives it, and the frame that runs it.
    pub(crate) fn act(&mut self, action: &str) -> Result<(), String> {
        self.set(GROUP_ACTION, json!(action))?;
        self.frame();
        Ok(())
    }

    /// OK as `ui.dialog.confirm` gives it: the frames in which the app commits the recolour and the
    /// dialog closes. `Err` with the reason the app refused it, which keeps the dialog open.
    pub(crate) fn confirm(&mut self) -> Result<(), String> {
        self.host(|host, ctx| host.confirm_by_id(ctx, Self::id()));
        self.frame().frame();
        match self.host(|host, _| host.held_state(Self::id())).flatten() {
            Some(HeldOk::Refused(reason)) => Err(reason.as_str().to_owned()),
            _ if self.is_open() => Err("Recolor Artwork stayed open".into()),
            _ => Ok(()),
        }
    }

    /// Cancel as `ui.dialog.cancel` gives it, and the frame in which the preview goes.
    pub(crate) fn cancel(&mut self) {
        self.host(|host, ctx| host.cancel_by_id(ctx, Self::id()));
        self.frame();
    }

    /// The control named `label`.
    pub(crate) fn by_label(&self, label: &str) -> Option<Target> {
        self.fixture.by_label(label)
    }

    /// The slider named `label`: a channel's rail, not the number field that shares its name.
    pub(crate) fn slider(&self, label: &str) -> Option<Target> {
        let tree = self.fixture.tree();
        self.fixture.by_role(Role::SLIDER).into_iter().find(|t| tree.name_of(t.node) == Some(label))
    }

    /// Where the thumb of the slider named `label` runs, left to right: its track inset by half the thumb.
    pub(crate) fn slider_travel(&self, label: &str) -> Option<egui::Rangef> {
        let rail = self.slider(label)?;
        let thumb = part_rect_of_node(self.fixture.ctx(), rail.node.into(), craft::behavior::PartKey::new("thumb"))?.width();
        Some(egui::Rangef::new(rail.rect.left() + thumb / 2.0, rail.rect.right() - thumb / 2.0))
    }

    /// The control named `label`; a trial without it fails.
    fn target(&self, label: &str) -> Target {
        self.by_label(label).unwrap_or_else(|| panic!("no control named {label:?}"))
    }

    /// Whether the control named `label` takes input.
    pub(crate) fn enabled(&self, label: &str) -> bool {
        !self.fixture.tree().is_disabled_key(self.target(label).node)
    }

    /// A click on the control named `label`.
    pub(crate) fn click_label(&mut self, label: &str) -> &mut Self {
        let target = self.target(label);
        self.fixture.click(&target);
        self
    }

    /// Two clicks on the control named `label` within the double-click time, after it: clicks before them are not
    /// counted with them.
    pub(crate) fn double_click(&mut self, label: &str) -> &mut Self {
        self.fixture.advance(1.0);
        let at = self.target(label).center();
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        self.fixture.frame(vec![Event::PointerMoved(at)]);
        for pressed in [true, false, true, false] {
            self.fixture.frame(vec![button(pressed)]);
        }
        self
    }

    /// The rects of the graphics objects inside `area` (a bar's parts), left to right.
    pub(crate) fn graphics_in(&self, area: Rect) -> Vec<Rect> {
        let mut rects: Vec<Rect> =
            self.fixture.by_role(Role::GRAPHICS_OBJECT).into_iter().map(|t| t.rect).filter(|r| area.contains_rect(*r)).collect();
        rects.sort_by(|a, b| a.left().total_cmp(&b.left()));
        rects
    }

    /// A press at `at`, held, after the pointer moved there.
    pub(crate) fn press_at(&mut self, at: Pos2) -> &mut Self {
        self.fixture.frame(vec![Event::PointerMoved(at)]);
        self.fixture.frame(vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
        self
    }

    /// The held button moved to `to`.
    pub(crate) fn move_to(&mut self, to: Pos2) -> &mut Self {
        self.fixture.frame(vec![Event::PointerMoved(to)]);
        self
    }

    /// The held button released at `at`, then a frame without input.
    pub(crate) fn release(&mut self, at: Pos2) -> &mut Self {
        self.fixture.release_at(at);
        self
    }
}
