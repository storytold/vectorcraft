//! The stop/point popover uses the Color panel's real mode chooser.
// Integration tests may unwrap and panic (AGENTS.md, Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Mutex, MutexGuard};

use craft::behavior::OwnerClass;
use craft::color_ui::ui::ColorPanelMode as Mode;
use craft::ui::dialog::dialog_id;
use craft::ui::runtime::env::WindowKey;
use craft::ui::runtime::frame::take_output;
use craft::ui::runtime::overlay;
use craft::ui::shell::{WindowDialogs, embed};
use egui::accesskit::{Node, Role, Toggled};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2, vec2};
use serde_json::{Value, json};
use vectorcraft_color::{GradientPaint, Paint, cms};
use vectorcraft_engine::Session;
use vectorcraft_ui_egui::{VectorcraftApp, i18n};

static UI_STATE: Mutex<()> = Mutex::new(());
const MODES: [&str; 5] = ["Grayscale Slider", "RGB Sliders", "HSB Sliders", "CMYK Sliders", "Web Color Sliders"];

struct GlobalState {
    language: i18n::Lang,
    colors: cms::ColorSettings,
    _lock: MutexGuard<'static, ()>,
}

impl GlobalState {
    fn new() -> Self {
        let lock = UI_STATE.lock().unwrap_or_else(|e| e.into_inner());
        let language = i18n::current();
        let colors = cms::active_settings();
        i18n::set_current(i18n::Lang::EN);
        cms::set_active(&cms::ColorSettings { rgb: cms::SRGB.into(), ..Default::default() }).unwrap();
        Self { language, colors, _lock: lock }
    }
}

impl Drop for GlobalState {
    fn drop(&mut self) {
        cms::set_active(&self.colors).unwrap();
        i18n::set_current(self.language);
    }
}

fn key(key: Key, pressed: bool) -> Event {
    Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers: Modifiers::NONE }
}

struct Scene {
    app: VectorcraftApp,
    ctx: egui::Context,
    size: egui::Vec2,
    time: f64,
    freeform: bool,
    nodes: Vec<Node>,
}

impl Scene {
    /// The app has the native runtime and fonts before its first frame.
    fn new(size: egui::Vec2, freeform: bool) -> Self {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("app.language", json!({"lang": "en"})).unwrap();
        app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
        app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 120, "height": 100})).unwrap();
        app.run(
            "paint.setFill",
            json!({"keepModel": true, "gradient": {
                "kind": if freeform { "freeform" } else { "linear" },
                "stops": [{"offset": 0, "color": "#2468ac"}, {"offset": 1, "color": "#e09020"}]
            }}),
        )
        .unwrap();
        app.run("paint.setStroke", json!({"none": true, "focus": false})).unwrap();
        assert!(app.session.fill_active, "removing the stroke must retain the intended gradient fill proxy");
        assert!(matches!(app.session.proxy_paints().0, Paint::Gradient(_)), "the actual active fill, not fallback markers, is a gradient");
        app.select_tool("directSelection");
        if freeform {
            app.run("paint.freeform.selectPoint", json!({"index": 0})).unwrap();
        } else {
            app.run("gradient.selectStop", json!({"index": 0})).unwrap();
        }
        app.run("window.panel", json!({"panel": "gradient"})).unwrap();
        let ctx = egui::Context::default();
        embed::install(&ctx, craft::ui::testing::install_spec(), &vectorcraft_ui_egui::ui_fonts()).unwrap();
        app.adopt_context(&ctx);
        ctx.enable_accesskit();
        Self { app, ctx, size, time: 0.0, freeform, nodes: vec![] }
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        // The normal host tick selects the App's language before painting, too.
        i18n::set_current(self.app.ui_language());
        let mut raw = RawInput {
            events,
            time: Some(self.time),
            focused: true,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            ..Default::default()
        };
        self.app.raw_input(&self.ctx, &mut raw);
        embed::before_input(&self.ctx, |out| self.app.frame_input(out));
        let mut output = self.ctx.run_ui(raw, |ui| {
            self.app.tick(ui.ctx());
            self.app.ui(ui);
        });
        let _ = self.app.frame_input(take_output(&self.ctx));
        self.nodes =
            output.platform_output.accesskit_update.as_ref().expect("AccessKit enabled").nodes.iter().map(|(_, node)| node.clone()).collect();
        output.textures_delta.clear();
    }

    fn idle(&mut self, count: usize) {
        for _ in 0..count {
            self.frame(vec![]);
        }
    }

    fn click_with(&mut self, point: Pos2, modifiers: Modifiers) {
        self.frame(vec![Event::ModifiersChanged(modifiers), Event::PointerMoved(point)]);
        self.frame(vec![Event::PointerButton { pos: point, button: PointerButton::Primary, pressed: true, modifiers }]);
        self.frame(vec![Event::PointerButton { pos: point, button: PointerButton::Primary, pressed: false, modifiers }]);
        self.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
    }

    fn click(&mut self, point: Pos2) {
        self.click_with(point, Modifiers::NONE);
    }

    fn escape(&mut self) {
        self.frame(vec![key(Key::Escape, true)]);
        self.frame(vec![key(Key::Escape, false)]);
        self.idle(2);
    }

    fn node_rect(node: &Node) -> Option<Rect> {
        let b = node.bounds()?;
        Some(Rect::from_min_max(pos2(b.x0 as f32, b.y0 as f32), pos2(b.x1 as f32, b.y1 as f32)))
    }

    fn popover(&self) -> Rect {
        self.nodes
            .iter()
            .find(|n| n.role() == Role::Dialog && n.label() == Some("Gradient stop"))
            .and_then(Self::node_rect)
            .expect("the actual native stop popover")
    }

    fn editor_open(&self) -> bool {
        WindowDialogs::of(&self.ctx, &WindowKey::main()).with(|host| host.open_dialogs().iter().any(|d| d.id == dialog_id("gradientStop")))
    }

    fn assert_editor(&self) {
        assert!(self.editor_open(), "the real stop/point editor remains open");
    }

    fn child_open(&self) -> bool {
        overlay::top(&self.ctx).and_then(|id| overlay::entry(&self.ctx, id)).is_some_and(|entry| !entry.exiting && entry.keys == OwnerClass::Menu)
    }

    fn open_stop(&mut self) {
        self.idle(8);
        let name = if self.freeform { "Edit the point" } else { "Edit the stop" };
        let matches: Vec<_> = self.nodes.iter().filter(|node| node.label() == Some(name)).filter_map(Self::node_rect).collect();
        assert_eq!(matches.len(), 1, "the actual Gradient panel's color chip {name}");
        self.click(matches[0].center());
        self.idle(3);
        self.assert_editor();
        if self.freeform {
            assert_eq!(self.app.session.selected_freeform_point(), Some(0));
        } else {
            assert_eq!(self.app.session.selected_stop(), Some(0));
        }
    }

    fn button(&self, label: &str) -> Rect {
        let popover = self.popover();
        let matches: Vec<_> = self
            .nodes
            .iter()
            .filter(|node| node.role() == Role::Button && node.label() == Some(label))
            .filter_map(Self::node_rect)
            .filter(|rect| rect.is_positive() && popover.contains(rect.center()))
            .collect();
        assert_eq!(matches.len(), 1, "one actual native mode chooser {label:?}");
        matches[0]
    }

    fn open_mode(&mut self, label: &str) {
        self.assert_editor();
        let button = self.button(label);
        assert!(self.popover().contains(button.center()));
        self.click(button.center());
        self.idle(3);
        assert!(self.child_open(), "actual child menu opens");
        self.assert_editor();
    }

    fn mode_rows(&self) -> Vec<(String, bool, Rect)> {
        let mut rows: Vec<_> = self
            .nodes
            .iter()
            .filter_map(|node| {
                let label = node.label()?;
                if !matches!(node.role(), Role::MenuItem | Role::MenuItemRadio | Role::MenuItemCheckBox) || !MODES.contains(&label) {
                    return None;
                }
                Some((label.to_owned(), node.toggled() == Some(Toggled::True), Self::node_rect(node)?))
            })
            .collect();
        rows.sort_by(|a, b| a.2.top().total_cmp(&b.2.top()));
        rows
    }

    fn assert_modes(&self, current: Option<&str>) {
        let rows = self.mode_rows();
        assert_eq!(rows.iter().map(|(label, _, _)| label.as_str()).collect::<Vec<_>>(), MODES);
        for (label, checked, rect) in rows {
            assert_eq!(checked, current == Some(label.as_str()), "current row: {label}");
            assert!(Rect::from_min_size(Pos2::ZERO, self.size).contains(rect.center()));
        }
    }

    fn choose_mode(&mut self, label: &str) -> Rect {
        let rows = self.mode_rows();
        let rect = rows.iter().find(|(text, _, _)| text == label).expect("actual offered mode").2;
        self.click(rect.center());
        self.idle(3);
        assert!(!self.child_open(), "picking a mode closes only its dropdown");
        self.assert_editor();
        rect
    }

    fn gradient(&self) -> GradientPaint {
        let Paint::Gradient(g) = self.app.session.proxy_paints().0 else { panic!("gradient fill") };
        *g
    }

    fn snapshot(&self) -> Value {
        let state = self.app.session.active().unwrap();
        json!({"doc": state.doc.as_ref(), "selection": state.selection,
            "undo": state.history.undo.iter().map(|e| json!({"label":e.label,"doc":e.doc.as_ref(),"selection":e.selection.as_ref()})).collect::<Vec<_>>(),
            "redo": state.history.redo.iter().map(|e| json!({"label":e.label,"doc":e.doc.as_ref(),"selection":e.selection.as_ref()})).collect::<Vec<_>>(),
            "step": state.history.step(), "stop": self.app.session.selected_stop(), "point": self.app.session.selected_freeform_point()})
    }

    fn geometry(&self) -> Value {
        let state = self.app.session.active().unwrap();
        json!(state.selection.objects.iter().map(|id| &state.doc.node(*id).unwrap().kind).collect::<Vec<_>>())
    }

    fn spectrum(&self) -> Rect {
        let popover = self.popover();
        let found: Vec<_> = self
            .nodes
            .iter()
            // The component and its interactive bar share a name; only the bar carries the mode value.
            .filter(|node| node.role() == Role::Group && node.label() == Some("Color Ramp") && node.value() == Some("RGB Sliders"))
            .filter_map(Self::node_rect)
            .filter(|rect| popover.contains(rect.center()))
            .collect();
        assert_eq!(found.len(), 1, "the actual native Color panel spectrum");
        found[0]
    }
}

#[test]
fn gradient_stop_mode_baseline_shift_and_menu_are_display_only() {
    let _globals = GlobalState::new();
    let mut scene = Scene::new(vec2(960.0, 720.0), false);
    scene.open_stop();
    let before = scene.snapshot();
    let original = scene.gradient();
    assert_eq!(original.gradient.stops[0].color.model(), cms::Model::Rgb);
    // Retain the existing Shift control through the native panel's local-to-app synchronization.
    scene.click_with(scene.spectrum().center(), Modifiers::SHIFT);
    scene.idle(3);
    assert_eq!(scene.app.ui.color_panel.mode, Mode::HsbSliders);
    assert_eq!(scene.snapshot(), before, "Shift cycling changes display, not art/history");
    scene.open_mode("HSB Sliders");
    scene.assert_modes(Some("HSB Sliders"));
    assert_eq!(scene.snapshot(), before, "opening the menu is not an edit");
    scene.escape();
    assert!(!scene.child_open());
    scene.assert_editor();
    assert_eq!(scene.snapshot(), before);
    scene.open_mode("HSB Sliders");
    scene.choose_mode("RGB Sliders");
    scene.open_mode("RGB Sliders");
    scene.assert_modes(Some("RGB Sliders"));
    scene.choose_mode("HSB Sliders");
    assert_eq!(scene.gradient(), original);
    assert_eq!(scene.snapshot(), before, "RGB/HSB preserve native RGB and one-stop selection");
    scene.escape();
    assert!(!scene.editor_open(), "after dropdown dismissal the next Escape closes the editor");
    assert_eq!(scene.snapshot(), before);
}

#[test]
fn gradient_stop_mode_converts_only_selected_stop_and_unlinks_tint_with_one_undo() {
    let _globals = GlobalState::new();
    for tint in [false, true] {
        let mut scene = Scene::new(vec2(960.0, 720.0), false);
        if tint {
            scene.app.run("swatch.new", json!({"name":"Ink","color":{"c":0,"m":1,"y":0,"k":0},"spot":true})).unwrap();
            scene
                .app
                .run(
                    "paint.setFill",
                    json!({"gradient":{"stops":[
                        {"offset":0,"swatch":"Ink","tint":50}, {"offset":1,"color":"#e09020"}
                    ]}}),
                )
                .unwrap();
        }
        scene.open_stop();
        let before = scene.snapshot();
        let original = scene.gradient();
        let geometry = scene.geometry();
        let undo = scene.app.session.active().unwrap().history.undo.len();
        if tint {
            assert_eq!(original.gradient.stops[0].swatch.as_deref(), Some("Ink"));
            assert!((original.gradient.stops[0].tint - 0.5).abs() < 1e-6);
        }
        scene.open_mode(if tint { "Tint" } else { "RGB Sliders" });
        scene.assert_modes(if tint { None } else { Some("RGB Sliders") });
        scene.choose_mode("CMYK Sliders");
        let converted = scene.gradient();
        assert_eq!(converted.gradient.stops[0].color.model(), cms::Model::Cmyk);
        assert!(converted.gradient.stops[0].swatch.is_none(), "process mode unlinks a tint");
        let mut expected = original.clone();
        expected.gradient.stops[0].color = converted.gradient.stops[0].color;
        expected.gradient.stops[0].swatch = None;
        expected.gradient.stops[0].tint = 1.0;
        expected.gradient.stops[0].plates = None;
        assert_eq!(converted, expected, "other stops, placement and gradient settings are untouched");
        assert_eq!(scene.geometry(), geometry);
        assert_eq!(scene.app.session.active().unwrap().history.undo.len(), undo + 1);
        scene.app.run("edit.undo", json!({})).unwrap();
        assert_eq!(serde_json::to_value(scene.app.session.active().unwrap().doc.as_ref()).unwrap(), before["doc"]);
        assert_eq!(scene.gradient(), original);
        // Undo changes the gradient under the editor, which closes it; the next steps open it again.
        scene.idle(3);
        assert!(!scene.editor_open(), "an undo from outside the editor closes it");
        scene.open_stop();
        if !tint {
            // Hide Options is the existing compact mixer; its child menu can outgrow the editor.
            scene.app.run("ui.colorPanel", json!({"options": false})).unwrap();
            scene.idle(3);
            scene.open_mode("RGB Sliders");
            scene.assert_modes(Some("RGB Sliders"));
            let row = scene.mode_rows().into_iter().find(|(label, _, _)| label == "Web Color Sliders").unwrap().2;
            let parent = scene.popover();
            assert!(!parent.contains(row.center()), "compact editor must expose a real child row outside itself: parent={parent:?}, row={row:?}");
            scene.choose_mode("Web Color Sliders");
            assert_eq!(scene.app.ui.color_panel.mode, Mode::WebColorSliders);
            assert_ne!(
                scene.gradient().gradient.stops[0].color,
                original.gradient.stops[0].color,
                "the original non-web-safe RGB stop is actually converted"
            );
            assert_eq!(scene.geometry(), geometry);
            scene.app.run("edit.undo", json!({})).unwrap();
            assert_eq!(serde_json::to_value(scene.app.session.active().unwrap().doc.as_ref()).unwrap(), before["doc"]);
            assert_eq!(scene.gradient(), original, "one Undo restores the pre-conversion paint");
            scene.idle(3);
            assert!(!scene.editor_open(), "an undo from outside the editor closes it");
            scene.open_stop();
        }
        // A real outside click closes both editor and child, without another edit.
        scene.idle(3);
        // Undo restores art, while the chosen display mode remains a UI preference.
        scene.open_mode(if tint { "Tint" } else { "Web Color Sliders" });
        let closed_boundary = scene.snapshot();
        let outside = pos2(2.0, scene.size.y - 2.0);
        assert!(!scene.popover().contains(outside));
        assert!(scene.mode_rows().iter().all(|(_, _, r)| !r.contains(outside)));
        scene.click(outside);
        scene.idle(3);
        assert!(!scene.editor_open());
        assert!(!scene.child_open());
        assert_eq!(scene.snapshot(), closed_boundary);
    }
}

#[test]
fn gradient_stop_mode_freeform_point_uses_the_same_real_menu() {
    let _globals = GlobalState::new();
    let mut scene = Scene::new(vec2(960.0, 720.0), true);
    scene.open_stop();
    let before = scene.snapshot();
    let original = scene.gradient();
    scene.open_mode("RGB Sliders");
    scene.assert_modes(Some("RGB Sliders"));
    scene.choose_mode("HSB Sliders");
    assert_eq!(scene.app.ui.color_panel.mode, Mode::HsbSliders);
    assert_eq!(scene.gradient(), original, "display-only point editing keeps every freeform point");
    assert_eq!(scene.snapshot(), before);
    let (points, _) = scene.app.session.proxy_freeform(false).unwrap();
    let undo = scene.app.session.active().unwrap().history.undo.len();
    let geometry = scene.geometry();
    scene.open_mode("HSB Sliders");
    scene.choose_mode("CMYK Sliders");
    let (converted, _) = scene.app.session.proxy_freeform(false).unwrap();
    assert_eq!(converted.points[0].color.model(), cms::Model::Cmyk);
    let mut expected = points;
    expected.points[0].color = converted.points[0].color;
    assert_eq!(converted, expected, "conversion changes only the selected freeform point's color");
    assert_eq!(scene.geometry(), geometry);
    assert_eq!(scene.app.session.active().unwrap().history.undo.len(), undo + 1);
    scene.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(serde_json::to_value(scene.app.session.active().unwrap().doc.as_ref()).unwrap(), before["doc"]);
    assert_eq!(scene.gradient(), original, "one Undo restores the complete point paint");
    // Undo changes the gradient under the editor, which closes it; the next steps open it again.
    scene.idle(3);
    assert!(!scene.editor_open(), "an undo from outside the editor closes it");
    scene.open_stop();
    let boundary = scene.snapshot();
    scene.open_mode("RGB Sliders");
    scene.escape();
    scene.assert_editor();
    scene.escape();
    assert!(!scene.editor_open());
    assert_eq!(scene.snapshot(), boundary, "both Escape stages preserve artwork and history");
}
