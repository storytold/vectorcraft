//! The stop/point popover uses the Color panel's real mode chooser.
// Integration tests may unwrap and panic (AGENTS.md, Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Mutex, MutexGuard};

use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Shape, pos2, vec2};
use serde_json::{Value, json};
use vectorcraft_color::{GradientPaint, Paint, cms};
use vectorcraft_engine::Session;
use vectorcraft_ui_egui::{VectorcraftApp, i18n, panels::color::Mode, theme};

static UI_STATE: Mutex<()> = Mutex::new(());
const MODES: [&str; 5] = ["Grayscale", "RGB", "HSB", "CMYK", "Web Safe RGB"];
type FrameObserver = Box<dyn FnMut(&egui::Context, &egui::FullOutput)>;

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

fn visit_shapes(shape: &Shape, clip: Rect, visit: &mut impl FnMut(&Shape, Rect)) {
    if let Shape::Vec(shapes) = shape {
        for shape in shapes {
            visit_shapes(shape, clip, visit);
        }
    } else {
        visit(shape, clip);
    }
}

struct Scene {
    app: VectorcraftApp,
    ctx: egui::Context,
    last_output: egui::FullOutput,
    // Optional test-frame observation consumes initial texture uploads before they are cleared.
    on_frame: Option<FrameObserver>,
    size: egui::Vec2,
    time: f64,
    freeform: bool,
    buttons: Vec<(String, Rect)>,
}

impl Scene {
    /// No UI frames: an observer can attach before the first font upload.
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
        }
        app.run("window.panel", json!({"panel": "gradient"})).unwrap();
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        egui_extras::install_image_loaders(&ctx);
        ctx.enable_accesskit();
        Self { app, ctx, last_output: Default::default(), on_frame: None, size, time: 0.0, freeform, buttons: vec![] }
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
        self.app.raw_input_hook(&self.ctx, &mut raw);
        let mut output = self.ctx.run_ui(raw, |ui| self.app.ui(ui));
        self.buttons.clear();
        let update = output.platform_output.accesskit_update.as_ref().expect("AccessKit enabled");
        for (_, node) in &update.nodes {
            if node.role() == egui::accesskit::Role::Button
                && let Some((label, bounds)) = node.label().zip(node.bounds())
            {
                self.buttons
                    .push((label.to_owned(), Rect::from_min_max(pos2(bounds.x0 as f32, bounds.y0 as f32), pos2(bounds.x1 as f32, bounds.y1 as f32))));
            }
        }
        if let Some(observer) = &mut self.on_frame {
            observer(&self.ctx, &output);
        }
        output.textures_delta.clear();
        self.last_output = output;
    }

    fn idle(&mut self, count: usize) {
        for _ in 0..count {
            self.frame(vec![]);
        }
    }

    fn click_with(&mut self, point: Pos2, modifiers: Modifiers) {
        self.frame(vec![
            Event::ModifiersChanged(modifiers),
            Event::PointerMoved(point),
            Event::PointerButton { pos: point, button: PointerButton::Primary, pressed: true, modifiers },
        ]);
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

    fn popover(&self) -> Rect {
        self.ctx.memory(|m| m.area_rect(egui::Id::unique("gradient-stop-popover"))).expect("actual painted popover area")
    }

    fn assert_editor(&self) {
        assert_eq!(self.app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some("gradientStop"));
    }

    fn painted(&self, mut visitor: impl FnMut(&Shape, Rect)) {
        for clipped in &self.last_output.shapes {
            visit_shapes(&clipped.shape, clipped.clip_rect, &mut visitor);
        }
    }

    fn open_stop(&mut self) {
        self.idle(8);
        if self.freeform {
            // The real Color: chip in freeform_section is the 18-point square next to its label.
            let mut labels = vec![];
            let mut chips = vec![];
            self.painted(|shape, clip| match shape {
                Shape::Text(text) if text.galley.text() == "Color:" => {
                    labels.push(text.galley.rect.translate(text.pos.to_vec2()));
                }
                Shape::Rect(rect)
                    if (rect.rect.width() - 18.0).abs() < 0.01 && (rect.rect.height() - 18.0).abs() < 0.01 && clip.contains(rect.rect.center()) =>
                {
                    chips.push(rect.rect)
                }
                _ => {}
            });
            chips.retain(|chip| {
                labels.iter().any(|label| {
                    chip.left() >= label.right() && chip.left() - label.right() < 20.0 && (chip.center().y - label.center().y).abs() < 10.0
                })
            });
            chips.dedup();
            assert_eq!(chips.len(), 1, "actual freeform Color chip, not a fabricated dialog: {chips:?}");
            self.click(chips[0].center());
        } else {
            let mut markers = vec![];
            self.painted(|shape, clip| {
                if let Shape::Path(path) = shape
                    && path.closed
                    && path.points.len() == 5
                {
                    let rect = Rect::from_points(&path.points);
                    if (rect.width() - 12.0).abs() < 0.01
                        && (rect.height() - 15.0).abs() < 0.01
                        && path.points[0].distance(pos2(rect.center().x, rect.top())) < 0.01
                        && clip.contains(rect.center())
                    {
                        markers.push(rect.center());
                    }
                }
            });
            assert_eq!(markers.len(), 2, "real two-stop Gradient panel: {markers:?}");
            markers.sort_by(|a, b| a.x.total_cmp(&b.x));
            self.click(markers[0]);
            self.click(markers[0]);
        }
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
            .buttons
            .iter()
            .filter(|(text, rect)| text == label && rect.is_positive() && popover.contains(rect.center()))
            .map(|(_, rect)| *rect)
            .collect();
        assert_eq!(matches.len(), 1, "one actual accessible {label:?} button; labels: {:?}", self.buttons);
        matches[0]
    }

    fn open_mode(&mut self, label: &str) {
        self.assert_editor();
        let button = self.button(label);
        assert!(self.popover().contains(button.center()));
        self.click(button.center());
        self.idle(3);
        assert!(egui::Popup::is_any_open(&self.ctx), "actual child menu opens");
        self.assert_editor();
    }

    fn mode_rows(&self) -> Vec<(String, bool, Rect)> {
        let mut rows: Vec<_> = self
            .buttons
            .iter()
            .filter_map(|(text, rect)| {
                let (label, checked) = if let Some(label) = text.strip_prefix("✓ ") { (label, true) } else { (text.strip_prefix("   ")?, false) };
                MODES.contains(&label).then_some((label.to_owned(), checked, *rect))
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
        assert!(!egui::Popup::is_any_open(&self.ctx), "picking a mode closes only its dropdown");
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
        let mut found = vec![];
        self.painted(|shape, clip| {
            if let Shape::Mesh(mesh) = shape
                && mesh.vertices.len() == 73 * 13
                && mesh.indices.len() == 72 * 12 * 6
            {
                let points: Vec<_> = mesh.vertices.iter().map(|v| v.pos).collect();
                let rect = Rect::from_points(&points);
                if popover.contains(rect.center()) && clip.contains(rect.center()) {
                    found.push(rect);
                }
            }
        });
        assert_eq!(found.len(), 1, "actual 72-by-12 Color spectrum mesh");
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
    // A legitimate existing control must succeed before the intended old-source failure.
    scene.click_with(scene.spectrum().center(), Modifiers::SHIFT);
    scene.idle(3);
    assert_eq!(scene.app.ui.color_mode, Some(Mode::Hsb));
    assert_eq!(scene.snapshot(), before, "Shift cycling changes display, not art/history");
    scene.open_mode("HSB"); // Baseline: this actual chooser is absent on ed6f.
    scene.assert_modes(Some("HSB"));
    assert_eq!(scene.snapshot(), before, "opening the menu is not an edit");
    scene.escape();
    assert!(!egui::Popup::is_any_open(&scene.ctx));
    scene.assert_editor();
    assert_eq!(scene.snapshot(), before);
    scene.open_mode("HSB");
    scene.choose_mode("RGB");
    scene.open_mode("RGB");
    scene.assert_modes(Some("RGB"));
    scene.choose_mode("HSB");
    assert_eq!(scene.gradient(), original);
    assert_eq!(scene.snapshot(), before, "RGB/HSB preserve native RGB and one-stop selection");
    scene.escape();
    assert!(scene.app.ui.dialog.is_none(), "after dropdown dismissal the next Escape closes the editor");
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
        scene.open_mode(if tint { "Tint" } else { "RGB" });
        scene.assert_modes(if tint { None } else { Some("RGB") });
        scene.choose_mode("CMYK");
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
        if !tint {
            // Hide Options is the existing compact mixer; its child menu can outgrow the editor.
            scene.app.run("ui.colorPanel", json!({"options": false})).unwrap();
            scene.idle(3);
            scene.open_mode("RGB");
            scene.assert_modes(Some("RGB"));
            let row = scene.mode_rows().into_iter().find(|(label, _, _)| label == "Web Safe RGB").unwrap().2;
            let parent = scene.popover();
            assert!(!parent.contains(row.center()), "compact editor must expose a real child row outside itself: parent={parent:?}, row={row:?}");
            scene.choose_mode("Web Safe RGB");
            assert_eq!(scene.app.ui.color_mode, Some(Mode::WebSafe));
            assert_ne!(
                scene.gradient().gradient.stops[0].color,
                original.gradient.stops[0].color,
                "the original non-web-safe RGB stop is actually converted"
            );
            assert_eq!(scene.geometry(), geometry);
            scene.app.run("edit.undo", json!({})).unwrap();
            assert_eq!(serde_json::to_value(scene.app.session.active().unwrap().doc.as_ref()).unwrap(), before["doc"]);
            assert_eq!(scene.gradient(), original, "one Undo restores the pre-conversion paint");
        }
        // A real outside click closes both editor and child, without another edit.
        scene.idle(3);
        // Undo restores art, while the chosen display mode remains a UI preference.
        scene.open_mode(if tint { "Tint" } else { "Web Safe RGB" });
        let closed_boundary = scene.snapshot();
        let outside = pos2(2.0, scene.size.y - 2.0);
        assert!(!scene.popover().contains(outside));
        assert!(scene.mode_rows().iter().all(|(_, _, r)| !r.contains(outside)));
        scene.click(outside);
        scene.idle(3);
        assert!(scene.app.ui.dialog.is_none());
        assert!(!egui::Popup::is_any_open(&scene.ctx));
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
    scene.open_mode("RGB");
    scene.assert_modes(Some("RGB"));
    scene.choose_mode("HSB");
    assert_eq!(scene.app.ui.color_mode, Some(Mode::Hsb));
    assert_eq!(scene.gradient(), original, "display-only point editing keeps every freeform point");
    assert_eq!(scene.snapshot(), before);
    scene.open_mode("HSB");
    scene.escape();
    scene.assert_editor();
    scene.escape();
    assert!(scene.app.ui.dialog.is_none());
    assert_eq!(scene.snapshot(), before);
}
