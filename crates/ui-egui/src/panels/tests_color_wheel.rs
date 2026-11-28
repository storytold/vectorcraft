//! The Color Wheel panel drawn headless through the framework's `Fixture`: what a wheel edit stores,
//! the remembered hue, the color model of a pick, the OKLCH and HSL rows and the saved settings.

use std::cell::{Ref, RefCell, RefMut};
use std::rc::Rc;

use craft::ui::testing::focus::AccessActionKind;
use craft::ui::testing::{Fixture, Target};
use egui::{Key, Modifiers, vec2};
use serde_json::{Value, json};
use vectorcraft_engine::Session;

use super::color_wheel::show;
use crate::VectorcraftApp;

/// An app with a selected rectangle filled with `fill`.
fn app(fill: &str) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("paint.setFill", json!({"color": fill})).unwrap();
    app
}

/// A hue given to a gray is remembered only until the document's next undo step: after the paint
/// changes and the same gray comes back while the rows are hidden, the rows show the gray's own hue.
#[test]
fn a_remembered_hue_is_dropped_after_the_paint_changes_while_the_rows_are_hidden() {
    for (rows, hue) in [("hsl", "HSL Hue"), ("oklch", "Hue")] {
        let mut a = app("#808080");
        a.run("ui.colorWheel", json!({"sliders": rows})).unwrap();
        let mut p = Panel::new(a);
        p.set(hue, 240.0);
        assert_eq!(p.number(hue), Some(240.0), "{rows}: the gray keeps the hue it was given");
        p.run("ui.colorWheel", json!({"sliders": "none"}));
        p.run("paint.setFill", json!({"color": "#ff0000"}));
        p.run("paint.setFill", json!({"color": "#808080"}));
        p.run("ui.colorWheel", json!({"sliders": rows}));
        assert_ne!(p.number(hue), Some(240.0), "{rows}: the remembered hue is dropped");
    }
}

/// `sliders: "oklch"` follows the active document's RGB space: it is an error in a document assigned
/// Display P3 while the working RGB space is sRGB, and is taken in a document without a profile.
#[test]
fn oklch_sliders_follow_the_active_documents_rgb_space() {
    const CHILD: &str = "VECTORCRAFT_TEST_COLOR_WHEEL_DOC_PROFILE";
    if std::env::var_os(CHILD).is_none() {
        // Color settings are process-wide: assign the profile in a separate test process.
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "panels::tests_color_wheel::oklch_sliders_follow_the_active_documents_rgb_space", "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("test result: ok. 1 passed; 0 failed;"),
            "isolated document profile test failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr),
        );
        return;
    }
    use vectorcraft_color::cms;
    let mut a = app("#339966");
    a.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    a.run("edit.assignProfile", json!({"rgb": cms::DISPLAY_P3})).unwrap();
    let mut p = Panel::new(a);
    assert!(cms::working_settings().rgb != cms::DISPLAY_P3, "the working RGB space is not Display P3");
    let refused = p.app_mut().run("ui.colorWheel", json!({"sliders": "oklch"}));
    assert!(refused.is_err(), "{refused:?}");
    p.run("document.activate", json!({"index": 0}));
    p.run("ui.colorWheel", json!({"sliders": "oklch"}));
    assert_eq!(p.app().ui.color_wheel.sliders, super::color_wheel::Sliders::Oklch);
}

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

    /// Clicks the node named `label` with the pointer.
    pub(super) fn point(&mut self, label: &str) {
        let t = self.node(label);
        self.fx.click(&t).idle(2);
    }
}
