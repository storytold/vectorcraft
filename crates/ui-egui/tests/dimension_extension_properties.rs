//! Selected Dimension extension-line controls through the real Properties panel.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft::ui::dialog::DialogId;
use craft::ui::runtime::env::WindowKey;
use craft::ui::shell::WindowDialogs;
use craft::ui::testing::Fixture;
use egui::{Key, Modifiers, vec2};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{NodeId, NodeKind, dimension::Dimension};
use vectorcraft_engine::Session;
use vectorcraft_ui_egui::VectorcraftApp;

/// The key the host knows the Tool Options dialog by.
const TOOL_OPTIONS: &str = "dimensionOptions";

/// The whole app in a `Fixture`, as a user sees it: every control is found by its name and pressed through input, and
/// the dialogs the app opens are reached through the window's dialog host.
struct Driver {
    app: Rc<RefCell<VectorcraftApp>>,
    fx: Fixture,
    host: WindowDialogs,
}

impl Driver {
    fn new(app: VectorcraftApp) -> Self {
        let app = Rc::new(RefCell::new(app));
        let shown = Rc::clone(&app);
        let mut fx = Fixture::new(move |ui| {
            let mut app = shown.borrow_mut();
            app.tick(ui.ctx());
            app.ui(ui);
        });
        fx.room(vec2(1800.0, 1500.0));
        let host = WindowDialogs::of(fx.ctx(), &WindowKey::main());
        let mut driver = Driver { app, fx, host };
        driver.idle(1);
        driver
    }

    /// Runs the commands the frames since the last call fired, as the window runs them, and gives the fixture the
    /// app's input for the next frame.
    fn input(&mut self) {
        let input = self.app.borrow_mut().frame_input(craft::ui::runtime::frame::take_output(self.fx.ctx()));
        self.fx.root(input.root).commands(input.commands);
        if let Some(host) = input.host {
            self.fx.host(host);
        }
    }

    /// `n` frames with no events, each with the app's input.
    fn idle(&mut self, n: usize) {
        for _ in 0..n {
            self.input();
            self.fx.frame(vec![]);
        }
        self.input();
    }

    fn run(&self, command: &str, params: Value) -> Value {
        self.app.borrow_mut().run(command, params).unwrap()
    }

    fn click(&mut self, name: &str) {
        let target = self.fx.by_label(name).unwrap_or_else(|| panic!("no control named {name}"));
        self.input();
        self.fx.click(&target);
        self.idle(3);
    }

    /// Types `text` into the number field `name` and commits it with Enter.
    fn type_number(&mut self, name: &str, text: &str) {
        self.click(name);
        self.fx.key(Key::A, Modifiers::COMMAND);
        self.fx.type_text(text);
        self.input();
        self.fx.key(Key::Enter, Modifiers::NONE);
        self.idle(3);
    }

    /// The number the field `name` shows.
    fn number(&self, name: &str) -> f64 {
        let target = self.fx.by_label(name).unwrap_or_else(|| panic!("no control named {name}"));
        self.fx.tree().numeric_of(target.node).and_then(|numeric| numeric.value).unwrap()
    }

    /// The colour `#rrggbb` the swatch `name` shows.
    fn color(&self, name: &str) -> String {
        let target = self.fx.by_label(name).unwrap_or_else(|| panic!("no control named {name}"));
        self.fx.tree().value_of(target.node).unwrap().to_owned()
    }

    /// The open dialog `key` names, with its fields.
    fn dialog(&self, key: &str) -> Option<(DialogId, Value)> {
        let open = self.host.with(|host| host.open_dialogs());
        open.into_iter().find(|dialog| dialog.key.as_deref() == Some(key)).map(|dialog| (dialog.id, dialog.fields.unwrap_or(Value::Null)))
    }

    fn set_field(&self, id: DialogId, field: &str, value: Value) {
        let ctx = self.fx.ctx().clone();
        self.host.with(|host| host.set_json_field(&ctx, id, field, value)).unwrap();
    }

    fn cancel(&mut self, id: DialogId) {
        let ctx = self.fx.ctx().clone();
        assert!(self.host.with(|host| host.cancel_by_id(&ctx, id)));
        self.idle(2);
    }

    /// Opens the colour picker the swatch `name` asks for under `key`, answers it with `hex` and presses its OK.
    fn pick_color(&mut self, name: &str, key: &str, hex: &str) {
        self.click(name);
        let (id, _) = self.dialog(key).expect("the swatch opens the colour picker");
        self.set_field(id, "color", json!(hex));
        let ctx = self.fx.ctx().clone();
        assert!(self.host.with(|host| host.confirm_by_id(&ctx, id)));
        self.idle(3);
    }
}

fn recipe(app: &VectorcraftApp, id: NodeId) -> Dimension {
    app.session.active().unwrap().doc.node(id).unwrap().extras().dimension.as_deref().unwrap().clone()
}

fn generated_styles(app: &VectorcraftApp, id: NodeId, extensions: [u8; 3], weight: f64) {
    let state = app.session.active().unwrap();
    let children = state.doc.node(id).unwrap().children().unwrap();
    let paths: Vec<_> = children.iter().filter(|node| matches!(&node.kind, NodeKind::Path { .. })).collect();
    assert_eq!(paths.len(), 3, "two extension paths and the main dimension line");
    for extension in &paths[..2] {
        let stroke = extension.appearance.stroke().unwrap();
        assert_eq!(stroke.width, weight);
        assert_eq!(stroke.paint, Paint::solid(Color::rgb8(extensions[0], extensions[1], extensions[2])));
    }
    let main = paths[2].appearance.stroke().unwrap();
    assert_eq!(main.width, 2.1);
    assert_eq!(main.paint, Paint::solid(Color::rgb8(255, 0, 0)));
    let text = children.iter().find(|node| matches!(&node.kind, NodeKind::Text(_))).unwrap();
    assert_eq!(text.appearance.fill().unwrap().paint, Paint::solid(Color::rgb8(0, 0, 255)));
}

/// A document with an ellipse and the dimension of its width: a red main line, blue text and black extension lines.
fn dimension_app() -> (VectorcraftApp, NodeId) {
    vectorcraft_ui_egui::i18n::set_current(vectorcraft_ui_egui::i18n::Lang::EN);
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("paint.setFill", json!({"color": "#d0d0d0"})).unwrap();
    app.run("shape.ellipse", json!({"x": 100, "y": 129.435, "width": 41.13, "height": 41.13})).unwrap();
    let made = app
        .run(
            "dimension.create",
            json!({"a": [100,150], "b": [141.13,150], "orientation": "horizontal", "offset": -25,
        "options": {"units": "pt", "precision": 2, "lineWeight": 2.1, "lineColor": [255,0,0], "textColor": [0,0,255],
            "extensionWeight": 0.5, "extensionColor": [0,0,0]}}),
        )
        .unwrap();
    let id = NodeId(made["id"].as_u64().unwrap());
    (app, id)
}

/// Selects the dimension alone and shows the Properties panel with its Dimension section.
fn select_properties(d: &mut Driver, id: NodeId) {
    d.app.borrow_mut().select_tool("selection");
    d.run("select.set", json!({"ids": [id.0]}));
    d.run("window.panel", json!({"panel": "properties"}));
    d.idle(4);
    assert!(d.fx.by_label("Apply to All").is_some(), "the Dimension section is in Properties");
}

fn fixture() -> (Driver, NodeId) {
    let (app, id) = dimension_app();
    let mut d = Driver::new(app);
    select_properties(&mut d, id);
    (d, id)
}

fn geometry(dimension: &Dimension) -> Value {
    let mut value = serde_json::to_value(dimension).unwrap();
    value.as_object_mut().unwrap().remove("options");
    value
}

#[test]
fn selected_dimension_properties_exposes_extension_weight_and_color() {
    let (mut app, id) = dimension_app();
    let original = recipe(&app, id);
    assert!((original.length - 41.13).abs() < 1e-9);
    assert!(original.label().unwrap().contains("41.13"));
    generated_styles(&app, id, [0, 0, 0], 0.5);
    let document = app.session.active().unwrap().doc.clone();
    let paint = app.session.paint.clone();

    // The artwork control, restored before the Properties controls are judged.
    let mut control = original.options.clone();
    control.extension_weight = 0.35;
    control.extension_color = [128, 128, 128];
    app.run("dimension.update", json!({"ids": [id.0], "options": control})).unwrap();
    generated_styles(&app, id, [128, 128, 128], 0.35);
    let changed = recipe(&app, id);
    assert_eq!((changed.a, changed.b, changed.offset, changed.length), (original.a, original.b, original.offset, original.length));
    assert_eq!(changed.label().unwrap(), original.label().unwrap());
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc, document);
    assert_eq!(recipe(&app, id), original);
    assert_eq!(app.session.paint, paint);

    // Tool Options holds both extension values.
    let mut d = Driver::new(app);
    d.run("tool.options", json!({"tool": "dimension"}));
    d.idle(2);
    let (tool_options, fields) = d.dialog(TOOL_OPTIONS).expect("Tool Options is open");
    assert_eq!(fields["extensionWeight"], json!(0.5));
    assert_eq!(fields["extensionColor"], json!([0, 0, 0]));
    d.cancel(tool_options);
    assert!(d.dialog(TOOL_OPTIONS).is_none());

    // Properties holds them too, apart from the main line's.
    select_properties(&mut d, id);
    assert!((d.number("Extension Line Weight") - 0.5).abs() < 1e-9);
    assert_eq!(d.color("Extension Line Color"), "#000000");
    assert!((d.number("Line Weight") - 2.1).abs() < 1e-9, "the selected main-line weight is shown");
    assert_eq!(d.color("Line Color"), "#ff0000");
    assert_eq!(d.color("Text Color"), "#0000ff");
    let app = d.app.borrow();
    assert_eq!(recipe(&app, id), original);
    assert_eq!(app.session.active().unwrap().doc, document);
    assert_eq!(app.session.paint, paint);
}

#[test]
fn properties_edits_both_extensions_preserves_main_text_and_roundtrips_editable_recipe() {
    let (mut d, id) = fixture();
    let original = recipe(&d.app.borrow(), id);
    let selection = d.app.borrow().session.active().unwrap().selection.clone();
    let paint = d.app.borrow().session.paint.clone();
    d.type_number("Extension Line Weight", "0.35");
    let weighted = recipe(&d.app.borrow(), id);
    assert_eq!(weighted.options.extension_weight, 0.35);
    assert_eq!(geometry(&weighted), geometry(&original));
    generated_styles(&d.app.borrow(), id, [0, 0, 0], 0.35);
    let undo = d.app.borrow().session.active().unwrap().history.undo.len();
    d.run("edit.undo", json!({}));
    assert_eq!(recipe(&d.app.borrow(), id), original);
    generated_styles(&d.app.borrow(), id, [0, 0, 0], 0.5);
    d.run("edit.redo", json!({}));
    assert_eq!(recipe(&d.app.borrow(), id), weighted);
    assert_eq!(d.app.borrow().session.active().unwrap().history.undo.len(), undo, "one typed weight is one step");
    d.idle(2);

    d.pick_color("Extension Line Color", "extensionColor", "#808080");
    let edited = recipe(&d.app.borrow(), id);
    let mut expected = original.clone();
    expected.options.extension_weight = 0.35;
    expected.options.extension_color = [128, 128, 128];
    assert_eq!(edited, expected, "only the selected independent extension fields change");
    generated_styles(&d.app.borrow(), id, [128, 128, 128], 0.35);
    assert_eq!(d.app.borrow().session.active().unwrap().selection, selection);
    assert_eq!(d.app.borrow().session.paint, paint);

    // The native save and open keep the recipe editable.
    let saved = d.run("document.save", json!({"format": "vectorcraft", "preview": false}));
    let data = saved["dataBase64"].as_str().expect("an untitled native save returns bytes");
    d.run("document.open", json!({"name": "dimension.vectorcraft", "dataBase64": data}));
    assert_eq!(recipe(&d.app.borrow(), id), edited);
    select_properties(&mut d, id);
    d.type_number("Extension Line Weight", "0.6");
    assert_eq!(recipe(&d.app.borrow(), id).options.extension_weight, 0.6, "the native recipe remains editable through Properties");
    assert_eq!(geometry(&recipe(&d.app.borrow(), id)), geometry(&edited));
    generated_styles(&d.app.borrow(), id, [128, 128, 128], 0.6);
    assert_eq!(d.app.borrow().session.paint, paint);
}

#[test]
fn properties_apply_all_copies_full_style_to_editable_dimensions_and_undoes_as_one_edit() {
    let (mut d, id) = fixture();
    let other = NodeId(
        d.run(
            "dimension.create",
            json!({"a": [200,150], "b": [241.13,150], "offset": -30,
        "options": {"lineWeight": 1.3, "lineColor": [0,128,0], "extensionWeight": 0.8, "extensionColor": [0,128,0]}}),
        )["id"]
            .as_u64()
            .unwrap(),
    );
    let locked = NodeId(
        d.run(
            "dimension.create",
            json!({"a": [100,220], "b": [141.13,220], "offset": -20,
        "options": {"lineWeight": 1.7, "lineColor": [192,0,192], "extensionWeight": 0.9, "extensionColor": [192,0,192]}}),
        )["id"]
            .as_u64()
            .unwrap(),
    );
    d.run("object.lock", json!({"ids": [locked.0]}));
    let other_original = recipe(&d.app.borrow(), other);
    let locked_original = recipe(&d.app.borrow(), locked);
    select_properties(&mut d, id);
    d.type_number("Extension Line Weight", "0.35");
    d.pick_color("Extension Line Color", "extensionColor", "#808080");
    assert_eq!(recipe(&d.app.borrow(), other), other_original, "selected edits do not change an unselected dimension");
    assert_eq!(recipe(&d.app.borrow(), locked), locked_original);
    let selected = recipe(&d.app.borrow(), id);
    let before = d.app.borrow().session.active().unwrap().doc.clone();
    let undo = d.app.borrow().session.active().unwrap().history.undo.len();
    let paint = d.app.borrow().session.paint.clone();
    d.click("Apply to All");
    assert_eq!(recipe(&d.app.borrow(), other).options, selected.options, "Apply to All copies the complete style, not only extensions");
    assert_eq!(geometry(&recipe(&d.app.borrow(), other)), geometry(&other_original));
    assert_eq!(recipe(&d.app.borrow(), locked), locked_original, "the locked control is untouched");
    assert_eq!(recipe(&d.app.borrow(), id), selected);
    assert_eq!(d.app.borrow().session.active().unwrap().history.undo.len(), undo + 1);
    let after = d.app.borrow().session.active().unwrap().doc.clone();
    d.run("edit.undo", json!({}));
    assert_eq!(d.app.borrow().session.active().unwrap().doc, before);
    d.run("edit.redo", json!({}));
    assert_eq!(d.app.borrow().session.active().unwrap().doc, after);
    assert_eq!(d.app.borrow().session.paint, paint);
}

#[test]
fn properties_set_default_is_inherited_by_an_options_free_new_dimension() {
    let (mut d, id) = fixture();
    let other =
        NodeId(d.run("dimension.create", json!({"a": [200,150], "b": [241.13,150], "options": {"extensionWeight": 0.8}}))["id"].as_u64().unwrap());
    let other_original = recipe(&d.app.borrow(), other);
    select_properties(&mut d, id);
    d.type_number("Extension Line Weight", "0.35");
    d.pick_color("Extension Line Color", "extensionColor", "#808080");
    let selected = recipe(&d.app.borrow(), id);
    let document = d.app.borrow().session.active().unwrap().doc.clone();
    let history = {
        let app = d.app.borrow();
        let state = app.session.active().unwrap();
        (state.history.undo.len(), state.history.redo.len())
    };
    let paint = d.app.borrow().session.paint.clone();
    d.click("Set as Default");
    {
        let app = d.app.borrow();
        let state = app.session.active().unwrap();
        assert_eq!(state.doc, document, "setting tool defaults does not apply them to existing dimensions");
        assert_eq!((state.history.undo.len(), state.history.redo.len()), history);
    }
    assert_eq!(recipe(&d.app.borrow(), other), other_original);
    d.run("window.panel.close", json!({"panel": "properties"}));
    d.run("tool.options", json!({"tool": "dimension"}));
    d.idle(2);
    let (tool_options, fields) = d.dialog(TOOL_OPTIONS).expect("Tool Options is open");
    assert_eq!(fields["extensionWeight"], json!(0.35));
    assert_eq!(fields["extensionColor"], json!([128, 128, 128]));
    // The dialog edits a draft until Save: a cancelled one leaves the style alone.
    d.set_field(tool_options, "extensionWeight", json!(0.9));
    assert_eq!(d.dialog(TOOL_OPTIONS).unwrap().1["extensionWeight"], json!(0.9));
    d.cancel(tool_options);
    let new = NodeId(d.run("dimension.create", json!({"a": [100,220], "b": [141.13,220], "offset": -20}))["id"].as_u64().unwrap());
    assert_eq!(recipe(&d.app.borrow(), new).options, selected.options, "no explicit options: new dimensions inherit the full style");
    assert_eq!(recipe(&d.app.borrow(), id), selected);
    assert_eq!(recipe(&d.app.borrow(), other), other_original);
    assert_eq!(d.app.borrow().session.paint, paint);
}
