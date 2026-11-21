//! The Color Wheel panel drawn headless through the framework's `Fixture`: what a wheel edit stores,
//! the remembered hue, the color model of a pick, the OKLCH and HSL rows and the saved settings.

use std::cell::{Ref, RefCell, RefMut};
use std::rc::Rc;

use craft::ui::testing::focus::AccessActionKind;
use craft::ui::testing::{Fixture, Target};
use egui::{Key, Modifiers, vec2};
use serde_json::{Value, json};
use vectorcraft_color::cms::{self, Cms, ColorSettings, delta_e2000};
use vectorcraft_color::{Color, Paint};
use vectorcraft_engine::Session;

use super::color_wheel::{Settings, Sliders, display_hsb, oklch_track, remembered_hsb, remembered_hsl, remembered_oklch, set, show, stored};
use crate::VectorcraftApp;

/// The app shown through the panel's `show`, driven by the parts' names and keys.
// PLAN: sh.SH4 `crate::testing::ui_context()` has no entry point on `Fixture` here, so the app's fonts are not installed.
pub(super) struct Panel {
    app: Rc<RefCell<VectorcraftApp>>,
    pub(super) fx: Fixture,
}

impl Panel {
    /// The panel of `app` in a flyout tall enough for the rows, settled.
    pub(super) fn new(app: VectorcraftApp) -> Self {
        let app = Rc::new(RefCell::new(app));
        let shown = app.clone();
        let mut fx = Fixture::new(move |ui| show(&mut shown.borrow_mut(), ui));
        fx.room(vec2(260.0, 420.0)).settle();
        Panel { app, fx }
    }

    pub(super) fn app(&self) -> Ref<'_, VectorcraftApp> {
        self.app.borrow()
    }

    pub(super) fn app_mut(&self) -> RefMut<'_, VectorcraftApp> {
        self.app.borrow_mut()
    }

    /// Runs a command, then two frames so the panel shows its result.
    pub(super) fn run(&mut self, id: &str, params: Value) {
        self.app.borrow_mut().run(id, params).unwrap();
        self.fx.idle(2);
    }

    /// The node named `label`.
    pub(super) fn node(&self, label: &str) -> Target {
        self.fx.by_label(label).unwrap_or_else(|| panic!("no node named {label}"))
    }

    /// The numeric value of the node named `label`.
    pub(super) fn number(&self, label: &str) -> Option<f64> {
        self.fx.tree().numeric_of(self.node(label).node)?.value
    }

    /// The text value of the node named `label`.
    pub(super) fn value(&self, label: &str) -> Option<String> {
        self.fx.tree().value_of(self.node(label).node).map(str::to_owned)
    }

    pub(super) fn disabled(&self, label: &str) -> bool {
        self.fx.tree().is_disabled_key(self.node(label).node)
    }

    /// Sets the row named `label` as a screen reader does.
    pub(super) fn set(&mut self, label: &str, v: f64) {
        let t = self.node(label);
        self.fx.access(AccessActionKind::SetValue(v), &t).idle(2);
    }

    /// Gives the node named `label` the keyboard focus, then presses `key`.
    pub(super) fn key(&mut self, label: &str, key: Key, modifiers: Modifiers) {
        let t = self.node(label);
        self.fx.access(AccessActionKind::Focus, &t).key(key, modifiers);
    }

    /// Activates the node named `label` as a screen reader's click does.
    pub(super) fn click(&mut self, label: &str) {
        let t = self.node(label);
        self.fx.access(AccessActionKind::Click, &t).idle(2);
    }
}

/// An app with a selected rectangle filled with `fill`.
fn app(fill: serde_json::Value) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("paint.setFill", json!({"color": fill})).unwrap();
    app
}

fn fill(app: &VectorcraftApp) -> Color {
    crate::panels::current_paints(app).0.color().expect("a solid fill")
}

fn hsb(c: &Color) -> [f32; 3] {
    display_hsb(&cms::active(), c)
}

fn near(a: [f32; 3], b: [f32; 3], tol: [f32; 3]) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() <= tol[i])
}

fn undo_len(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrCHECK
}
