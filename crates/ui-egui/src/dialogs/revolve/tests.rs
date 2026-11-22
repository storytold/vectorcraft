//! Real pointer interaction with Revolve's compact controls.
use super::*;
use crate::{dialogs, theme};
use egui::{Event, PointerButton, Rect, Vec2, pos2};
use vectorcraft_engine::Session;

struct Harness {
    app: VectorcraftApp,
    ctx: egui::Context,
    screen: Vec2,
    texts: Vec<(String, Rect)>,
}

impl Harness {
    fn new(screen: Vec2) -> Self {
        Self::with("M150 40 L180 60 L180 140 L150 160", REVOLVE, screen)
    }

    /// The dialog of 3D effect `effect` on a path drawn from SVG path data `d`.
    fn with(d: &str, effect: &str, screen: Vec2) -> Self {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width":300,"height":300})).unwrap();
        app.run("path.create", json!({"d": d})).unwrap();
        app.run("paint.setFill", json!({"color":"#338ad6"})).unwrap();
        app.run("effect.dialog", json!({"effect":effect,"item":null})).unwrap();
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        theme::apply(&ctx, Default::default());
        Self { app, ctx, screen, texts: vec![] }
    }

    fn frame(&mut self, events: Vec<Event>) {
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.screen)), events, ..Default::default() };
        let mut out = self.ctx.run_ui(input, |ui| dialogs::show(&mut self.app, ui.ctx()));
        out.textures_delta.clear();
        fn collect(shape: &egui::Shape, clip: Rect, texts: &mut Vec<(String, Rect)>) {
            match shape {
                egui::Shape::Text(t) if clip.contains_rect(t.visual_bounding_rect()) => {
                    texts.push((t.galley.text().to_string(), t.visual_bounding_rect()))
                }
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, clip, texts)),
                _ => {}
            }
        }
        self.texts.clear();
        out.shapes.iter().for_each(|s| collect(&s.shape, s.clip_rect, &mut self.texts));
    }

    fn settle(&mut self) {
        for _ in 0..5 {
            self.frame(vec![]);
        }
    }

    /// Scrolls the dialog's body to its end with the wheel over `text`, and lets the scroll finish.
    fn scroll_to_end(&mut self, text: &str) {
        let p = self.rect(text).center();
        let wheel = Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -4000.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        };
        self.frame(vec![Event::PointerMoved(p), wheel]);
        for _ in 0..60 {
            self.frame(vec![]);
        }
    }

    fn rect(&self, text: &str) -> Rect {
        self.texts.iter().find(|(s, _)| s == text).map(|(_, r)| *r).unwrap_or_else(|| panic!("missing text {text:?}"))
    }

    /// Return the rectangle of the widget drawn under `Id::unique(id)` in the revolve test harness.
    fn response(&self, id: impl std::hash::Hash + std::fmt::Debug) -> Rect {
        self.ctx.read_response(Id::unique(id)).unwrap().rect
    }

    /// The open dialog's track cube.
    fn cube_id(&self) -> Id {
        super::super::three_d_classic::cube_id(&self.app.ui.dialog.as_ref().unwrap().kind)
    }

    fn click(&mut self, p: Pos2) {
        self.frame(vec![Event::PointerMoved(p), button(p, true)]);
        self.frame(vec![button(p, false)]);
        self.settle();
    }

    fn drag(&mut self, p: Pos2, delta: Vec2) {
        self.frame(vec![Event::PointerMoved(p), button(p, true)]);
        for i in 1..=5 {
            self.frame(vec![Event::PointerMoved(p + delta * (i as f32 / 5.0))]);
        }
        self.frame(vec![button(p + delta, false)]);
        self.settle();
    }

    fn value(&self, key: &str) -> f64 {
        self.app.ui.dialog.as_ref().unwrap().f64(key, 0.0)
    }
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() }
}

#[test]
fn map_art_maps_a_symbol_onto_a_surface_of_extrude_and_bevel_classic() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width":300,"height":300})).unwrap();
    let mark = app.run("path.create", json!({"d":"M250 250L260 250L260 270L250 270Z"})).unwrap()["id"].clone();
    app.run("threeD.addGraphic", json!({"name":"Mark","ids":[mark]})).unwrap();
    app.run("path.create", json!({"d":"M50 50L150 50L150 150L50 150Z"})).unwrap();
    app.run("effect.dialog", json!({"effect":CLASSIC_EXTRUDE,"item":null})).unwrap();
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    theme::apply(&ctx, Default::default());
    let mut h = Harness { app, ctx, screen: vec2(1280.0, 900.0), texts: vec![] };
    h.settle();
    h.click(h.rect("Map Art…").center());
    assert!(h.app.ui.dialog.as_ref().unwrap().bool("__mapArt"), "Map Art… opens Map Art");
    let count: usize = h.texts.iter().find_map(|(s, _)| s.strip_prefix("/ ")).and_then(|n| n.parse().ok()).expect("the surface count");
    assert!(count >= 6, "a square's solid has at least six surfaces: {count}");
    // The Symbol menu maps the symbol onto the surface shown, the first.
    h.click(h.rect("None").center());
    h.click(h.rect("Mark").center());
    let maps = |h: &Harness| vectorcraft_effects::surface_maps(&json!({"mapArt": h.app.ui.dialog.as_ref().unwrap().fields.get("mapArt").cloned()}));
    assert_eq!(maps(&h).iter().map(|m| (m.surface, m.symbol.as_str(), m.scale_x)).collect::<Vec<_>>(), [(1, "Mark", 100.0)]);
    // The next surface has no map until it is given one; Scale to Fit fills the first.
    h.click(h.rect(">").center());
    assert_eq!(h.value("__mapSurface"), 2.0);
    assert_eq!(maps(&h).len(), 1);
    h.click(h.rect("<").center());
    h.click(h.rect("Scale to Fit").center());
    assert!(maps(&h)[0].scale_x > 100.0, "the 10-point art is scaled up to the 100-point cap: {:?}", maps(&h));
    h.click(h.rect("Done").center());
    assert!(!h.app.ui.dialog.as_ref().unwrap().bool("__mapArt"), "Done goes back to the effect's options");
    dialogs::confirm(&mut h.app).unwrap();
    let state = h.app.session.doc().unwrap();
    let id = state.selection.objects[0];
    let params = &state.doc.node(id).unwrap().appearance.effects[0].params;
    assert_eq!(vectorcraft_effects::surface_maps(params).len(), 1, "{params}");
    assert!(params.get("__mapArt").is_none() && params.get("__mapSurface").is_none(), "the dialog's own state stays out of the effect");
}

#[test]
fn map_art_maps_no_more_surfaces_than_an_object_takes() {
    let max = vectorcraft_effects::MAX_MAPS;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width":300,"height":300})).unwrap();
    let mark = app.run("path.create", json!({"d":"M250 250L260 250L260 270L250 270Z"})).unwrap()["id"].clone();
    app.run("threeD.addGraphic", json!({"name":"Mark","ids":[mark]})).unwrap();
    // A star with 36 points has 72 corners, so its solid has two caps and 72 walls.
    let corners: Vec<String> = (0..72)
        .map(|i| {
            let (r, a) = (if i % 2 == 0 { 100.0 } else { 60.0 }, f64::from(i) * std::f64::consts::TAU / 72.0);
            format!("{:.3} {:.3}", 150.0 + r * a.cos(), 150.0 + r * a.sin())
        })
        .collect();
    app.run("path.create", json!({"d": format!("M{}Z", corners.join(" L"))})).unwrap();
    app.run("effect.dialog", json!({"effect":CLASSIC_EXTRUDE,"item":null})).unwrap();
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    theme::apply(&ctx, Default::default());
    let mut h = Harness { app, ctx, screen: vec2(1280.0, 900.0), texts: vec![] };
    h.settle();
    h.click(h.rect("Map Art…").center());
    let count: usize = h.texts.iter().find_map(|(s, _)| s.strip_prefix("/ ")).and_then(|n| n.parse().ok()).expect("the surface count");
    assert!(count > max, "{count} surfaces");
    // Every map the object takes is used, and the surface shown has none.
    let full: Vec<Value> = (1..=max).map(|surface| json!({"surface": surface, "symbol": "Mark"})).collect();
    let d = h.app.ui.dialog.as_mut().unwrap();
    d.fields.insert("mapArt".into(), json!(full));
    d.fields.insert("__mapSurface".into(), json!(max + 1));
    h.settle();
    h.click(h.rect("None").center());
    if h.texts.iter().any(|(s, _)| s == "Mark") {
        h.click(h.rect("Mark").center());
    }
    let raw = h.app.ui.dialog.as_ref().unwrap().fields.get("mapArt").cloned().unwrap_or_default();
    assert_eq!(raw.as_array().map(Vec::len), Some(max), "no map past the limit is written");
}

#[test]
fn values_labels_sliders_and_reset_dots_change_the_live_effect_in_one_undo_step() {
    let mut h = Harness::new(vec2(1280.0, 900.0));
    let before = h.app.session.doc().unwrap().doc.clone();
    h.settle();
    assert!(!h.rect("Advanced").is_negative(), "Advanced is visible without scrolling in a normal window");
    let panel = h.ctx.memory(|m| m.area_rect(Id::unique(("dialog", "effect")).with("revolve"))).unwrap();
    // Cap adds one 28-point row to the shared dialog's previous 540-point bound.
    assert!(panel.width() < 420.0 && panel.height() < 570.0, "compact controls with Cap: {panel:?}");
    let label = h.rect("Rotation X");
    h.drag(pos2(label.left() + LABEL_W + 6.0 + VALUE_W / 2.0, label.center().y), vec2(40.0, 0.0));
    assert!(h.value("rotationX") > 0.0, "the value itself drags");
    let label = h.rect("Rotation Y");
    h.drag(label.center(), vec2(40.0, 0.0));
    assert!(h.value("rotationY") > 0.0, "the label scrubs");
    let label = h.rect("Revolve angle");
    h.click(pos2(label.left() + LABEL_W + VALUE_W + 24.0, label.center().y));
    assert!(h.value("angle") < 180.0, "the slider changes the sweep");
    h.click(h.response(("revolve-reset", "rotationX")).center());
    assert_eq!(h.value("rotationX"), 0.0);
    assert!(h.value("angle") < 180.0, "a dot resets only its own setting");
    let node = h.app.session.doc().unwrap().doc.node(h.app.session.doc().unwrap().selection.objects[0]).unwrap();
    assert_eq!(node.appearance.effects[0].params["angle"], json!(h.value("angle")));
    h.click(h.response("revolve-reset-all").center());
    let defaults = vectorcraft_effects::default_params(REVOLVE).unwrap();
    let dialog = h.app.ui.dialog.as_ref().unwrap();
    for (key, value) in defaults.as_object().unwrap() {
        assert_eq!(dialog.fields.get(key), Some(value), "reset all: {key}");
    }
    dialogs::confirm(&mut h.app).unwrap();
    h.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(h.app.session.doc().unwrap().doc, before);
}

#[test]
fn preview_off_changes_fields_without_changing_the_document() {
    let mut h = Harness::new(vec2(1280.0, 900.0));
    h.app.ui.dialog.as_mut().unwrap().fields.insert("preview".into(), json!(false));
    let before = h.app.session.doc().unwrap().doc.clone();
    h.settle();
    h.drag(h.rect("Rotation Y").center(), vec2(40.0, 0.0));
    assert!(h.value("rotationY") > 0.0);
    assert_eq!(h.app.session.doc().unwrap().doc, before);
    dialogs::cancel(&mut h.app);
    assert_eq!(h.app.session.doc().unwrap().doc, before);
}

#[test]
fn command_driven_fields_preview_and_small_windows_keep_the_buttons_reachable() {
    let mut h = Harness::new(vec2(640.0, 480.0));
    h.settle();
    let dialog = h.app.ui.dialog.as_mut().unwrap();
    dialog.fields.insert("angle".into(), json!(180));
    h.settle();
    let state = h.app.session.doc().unwrap();
    let node = state.doc.node(state.selection.objects[0]).unwrap();
    assert_eq!(node.appearance.effects[0].params["angle"], json!(180));
    let screen = Rect::from_min_size(Pos2::ZERO, h.screen);
    assert!(screen.contains_rect(h.rect("OK")) && screen.contains_rect(h.rect("Cancel")));
    dialogs::cancel(&mut h.app);
    assert!(!h.app.session.in_interaction());
}

#[test]
fn the_shadows_switch_shows_its_settings_and_turns_the_shadow_on_in_one_undo_step() {
    let mut h = Harness::with("M100 100 L200 100 L200 200 L100 200 Z", EXTRUDE, vec2(1280.0, 900.0));
    let before = h.app.session.doc().unwrap().doc.clone();
    h.settle();
    h.click(h.rect("Lighting").center());
    assert!(!h.texts.iter().any(|(s, _)| s == "Shadow Bounds"), "the settings show only while shadows are on");
    h.click(h.rect("Shadows").center());
    assert_eq!(h.app.ui.dialog.as_ref().unwrap().fields.get("shadow"), Some(&json!(true)));
    assert!(h.texts.iter().any(|(s, _)| s == "Distance from Object"));
    let state = h.app.session.doc().unwrap();
    let node = state.doc.node(state.selection.objects[0]).unwrap();
    assert_eq!(node.appearance.effects[0].params["shadow"], json!(true), "the preview draws the shadow");
    dialogs::confirm(&mut h.app).unwrap();
    h.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(h.app.session.doc().unwrap().doc, before);
}

#[test]
fn lighting_tab_shows_light_fields_and_keeps_the_dialog_compact() {
    let mut h = Harness::new(vec2(1280.0, 900.0));
    h.settle();
    h.click(h.rect("Lighting").center());
    assert_eq!(h.app.ui.dialog.as_ref().unwrap().str("__gizmo"), "light");
    for label in ["Shading", "Color", "Intensity", "Height", "Softness", "Ambient Light", "Light behind the object"] {
        assert!(Rect::from_min_size(Pos2::ZERO, h.screen).contains_rect(h.rect(label)));
    }
    let panel = h.ctx.memory(|m| m.area_rect(Id::unique(("dialog", "effect")).with("revolve"))).unwrap();
    // The Shadows switch adds one 28-point row to the 570-point bound; its settings show only
    // while it is on.
    assert!(panel.width() < 420.0 && panel.height() < 600.0, "compact lighting controls with light management: {panel:?}");
    h.drag(h.rect("Height").center(), vec2(24.0, 0.0));
    assert_ne!(h.value("lightHeight"), 30.0);
    h.click(h.response(("revolve-reset", "lightHeight")).center());
    assert_eq!(h.value("lightHeight"), 30.0);
    h.click(h.rect("Rotation").center());
    assert_eq!(h.app.ui.dialog.as_ref().unwrap().str("__gizmo"), "rotation");
    assert!(!h.rect("Rotation X").is_negative());
}

#[test]
fn automatic_smooth_shading_previews_saves_and_respects_cancel_and_one_step_undo() {
    for confirm in [false, true] {
        let mut h = Harness::new(vec2(1280.0, 900.0));
        let before = h.app.session.doc().unwrap().doc.clone();
        h.settle();
        h.click(h.rect("Lighting").center());
        assert!(!h.texts.iter().any(|(text, _)| text == "Smooth shading"));
        assert!(!h.app.ui.dialog.as_ref().unwrap().fields.contains_key("smoothShading"));
        let art = |h: &Harness| {
            let state = h.app.session.doc().unwrap();
            vectorcraft_effects::revolve_art(state.doc.node(state.selection.objects[0]).unwrap()).unwrap()
        };
        let gradients = |n: &vectorcraft_doc::Node| {
            let mut count = 0;
            n.walk(&mut |face| {
                count += face.appearance.items.iter().filter(|item| {
                    matches!(item, vectorcraft_doc::AppearanceItem::Fill(fill) if matches!(fill.paint, vectorcraft_doc::color::Paint::Gradient(_)))
                }).count();
            });
            count
        };
        assert!(gradients(&art(&h)) > 0, "default rendering uses vector gradients");
        h.click(h.rect("Shading").center());
        assert_eq!(gradients(&art(&h)), 0, "Shading still controls all lighting");
        h.click(h.response(("revolve-reset", "shade")).center());
        assert!(gradients(&art(&h)) > 0);
        h.drag(h.rect("Height").center(), vec2(24.0, 0.0));
        h.app.ui.dialog.as_mut().unwrap().fields.insert("preview".into(), json!(false));
        h.settle();
        assert_eq!(h.app.session.doc().unwrap().doc, before);
        h.app.ui.dialog.as_mut().unwrap().fields.insert("preview".into(), json!(true));
        h.settle();
        if confirm {
            dialogs::confirm(&mut h.app).unwrap();
            let state = h.app.session.doc().unwrap();
            let node = state.doc.node(state.selection.objects[0]).unwrap();
            let loaded = vectorcraft_format::load(&vectorcraft_format::save(&state.doc, false)).unwrap();
            assert!(!node.appearance.effects[0].params.as_object().unwrap().contains_key("smoothShading"));
            assert_eq!(vectorcraft_effects::revolve_art(loaded.node(node.id).unwrap()), vectorcraft_effects::revolve_art(node));
            h.app.run("edit.undo", json!({})).unwrap();
        } else {
            dialogs::cancel(&mut h.app);
        }
        assert_eq!(h.app.session.doc().unwrap().doc, before);
    }
}

#[test]
fn advanced_visibility_option_is_saved_resettable_and_leaves_preview_geometry_complete() {
    let mut h = Harness::new(vec2(1280.0, 1000.0));
    h.settle();
    h.click(h.rect("Advanced").center());
    let panel = h.ctx.memory(|m| m.area_rect(Id::unique(("dialog", "effect")).with("revolve"))).unwrap();
    h.frame(vec![
        Event::PointerMoved(panel.center()),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -220.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        },
    ]);
    h.settle();
    let before = vectorcraft_effects::revolve_art({
        let state = h.app.session.doc().unwrap();
        state.doc.node(state.selection.objects[0]).unwrap()
    })
    .unwrap();
    let checkbox = h.rect("Keep visible surfaces only when expanding");
    h.click(checkbox.center());
    assert!(!h.app.ui.dialog.as_ref().unwrap().bool("expandVisibleOnly"));
    let state = h.app.session.doc().unwrap();
    let node = state.doc.node(state.selection.objects[0]).unwrap();
    assert_eq!(node.appearance.effects[0].params["expandVisibleOnly"], json!(false));
    assert_eq!(vectorcraft_effects::revolve_art(node), Some(before));
    h.click(h.response(("revolve-reset", "expandVisibleOnly")).center());
    assert!(h.app.ui.dialog.as_ref().unwrap().bool("expandVisibleOnly"));
    h.click(h.rect("Keep visible surfaces only when expanding").center());
    h.click(h.response("revolve-reset-all").center());
    assert!(h.app.ui.dialog.as_ref().unwrap().bool("expandVisibleOnly"));
    dialogs::confirm(&mut h.app).unwrap();
    let state = h.app.session.doc().unwrap();
    let loaded = vectorcraft_format::load(&vectorcraft_format::save(&state.doc, false)).unwrap();
    assert_eq!(loaded.node(state.selection.objects[0]).unwrap().appearance.effects[0].params["expandVisibleOnly"], json!(true));
}

/// Extrude & Bevel, Inflate and Rotate open the same compact controls, with their own options, the
/// Position presets and the artboard gizmo turning about the centre of the art.
#[test]
fn the_other_3d_effects_show_their_options_and_position_presets() {
    use vectorcraft_effects::{EXTRUDE, INFLATE, ROTATE, ROTATION_PRESETS};
    let square = "M100 100 L200 100 L200 200 L100 200 Z";
    for (effect, own) in [
        (EXTRUDE, &["Depth", "Twist", "Taper", "Cap", "Bevel", "Bevel Shape", "Space", "Bevel Inside", "Bevel both sides"][..]),
        (INFLATE, &["Depth", "Twist", "Taper", "Cap", "Volume", "Both sides"][..]),
        (ROTATE, &[][..]),
    ] {
        let mut h = Harness::with(square, effect, vec2(1400.0, 1000.0));
        h.settle();
        // Extrude & Bevel and Inflate open on their Object tab; Rotate on Rotation.
        for text in own.iter() {
            h.rect(text);
        }
        if !own.is_empty() {
            h.app.ui.dialog.as_mut().unwrap().fields.insert("__tab".into(), json!("rotation"));
            h.settle();
        }
        for text in ["Presets", "Rotation X", "Perspective"] {
            h.rect(text);
        }
        // The defaults are the Off-Axis Front position, shown by name.
        h.rect("Off-Axis Front");
        assert!(!h.texts.iter().any(|(t, _)| t == "Revolve angle"), "{effect}: Revolve's own options stay out");
        let d = h.app.ui.dialog.as_ref().unwrap();
        assert!(dialogs::revolve_gizmo::active(d), "{effect}: the gizmo is on");
        // A preset's angles show its name; other angles are Custom.
        let [x, y, z] = ROTATION_PRESETS[14].1;
        let d = h.app.ui.dialog.as_mut().unwrap();
        d.fields.extend([("rotationX".into(), json!(x)), ("rotationY".into(), json!(y)), ("rotationZ".into(), json!(z))]);
        h.settle();
        h.rect("Isometric Top");
        h.app.ui.dialog.as_mut().unwrap().fields.insert("rotationZ".into(), json!(12.5));
        h.settle();
        h.rect("Custom");
    }
}

/// Extrude & Bevel (Classic): its extrude and bevel options on the Object tab, Position on the
/// Rotation tab, and the surface with its lights, highlight, blend steps and shading color on the
/// Lighting tab; OK keeps the edit as one undo step.
#[test]
fn classic_extrude_bevel_shows_its_options_and_applies_as_one_step() {
    use vectorcraft_effects::CLASSIC_EXTRUDE;
    let mut h = Harness::with("M100 100 L200 100 L200 200 L100 200 Z", CLASSIC_EXTRUDE, vec2(1400.0, 1000.0));
    let before = h.app.session.doc().unwrap().doc.clone();
    h.settle();
    for text in ["Depth", "Cap", "Bevel Shape", "Height", "Bevel Inside"] {
        h.rect(text);
    }
    assert!(!h.texts.iter().any(|(t, _)| t == "Twist" || t == "Taper" || t == "Width"), "the newer effect's options stay out");
    assert!(!h.ctx.read_response(Id::unique(("revolve-reset", "bevelHeight"))).unwrap().enabled(), "no bevel: its height is off");
    h.app.ui.dialog.as_mut().unwrap().fields.insert("__tab".into(), json!("rotation"));
    h.settle();
    for text in ["Presets", "Rotation X", "Perspective", "Off-Axis Front"] {
        h.rect(text);
    }
    h.click(h.rect("Lighting").center());
    h.settle();
    for text in ["Surface", "Plastic", "Move Light to Back", "Light Intensity"] {
        h.rect(text);
    }
    // The lighting sphere makes the tab taller than the dialog's body, which scrolls.
    h.scroll_to_end("Light Intensity");
    for text in ["Ambient Light", "Highlight Intensity", "Highlight Size", "Blend Steps", "Shading Color", "Draw Hidden Faces"] {
        h.rect(text);
    }
    assert!(h.ctx.read_response(Id::unique(("revolve-reset", "highlightSize"))).unwrap().enabled());
    // The light gizmo is on for a lit surface.
    assert_eq!(dialogs::revolve_gizmo::hint(h.app.ui.dialog.as_ref().unwrap()), "Drag the light, or its rings. Shift snaps to 15°.");
    // Diffuse has no highlight; Wireframe and No Shading take no light.
    h.app.ui.dialog.as_mut().unwrap().fields.insert("surface".into(), json!("diffuse"));
    h.settle();
    assert!(!h.ctx.read_response(Id::unique(("revolve-reset", "highlightSize"))).unwrap().enabled());
    assert!(h.ctx.read_response(Id::unique(("revolve-reset", "ambient"))).unwrap().enabled());
    h.app.ui.dialog.as_mut().unwrap().fields.insert("surface".into(), json!("wireframe"));
    h.settle();
    assert!(!h.ctx.read_response(Id::unique(("revolve-reset", "ambient"))).unwrap().enabled());
    assert!(h.ctx.read_response(Id::unique(("revolve-reset", "drawHiddenFaces"))).unwrap().enabled());
    assert_eq!(dialogs::revolve_gizmo::hint(h.app.ui.dialog.as_ref().unwrap()), "Drag values or labels. Double-click a value to type.");
    // A custom shading color shows its color row.
    let d = h.app.ui.dialog.as_mut().unwrap();
    d.fields.insert("surface".into(), json!("diffuse"));
    d.fields.insert("shadingColor".into(), json!("custom"));
    h.settle();
    h.rect("#000000");
    dialogs::confirm(&mut h.app).unwrap();
    let state = h.app.session.doc().unwrap();
    let fx = &state.doc.node(state.selection.objects[0]).unwrap().appearance.effects[0];
    assert_eq!(fx.id, CLASSIC_EXTRUDE);
    assert_eq!((fx.params["surface"].clone(), fx.params["shadingColor"].clone()), (json!("diffuse"), json!("custom")));
    h.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(h.app.session.doc().unwrap().doc, before);
}

/// Revolve (Classic): its angle, cap, offset and axis side on the Object tab, Position on the
/// Rotation tab, and the classic surface on the Lighting tab, without Draw Hidden Faces; OK keeps
/// the edit as one undo step.
#[test]
fn classic_revolve_shows_its_options_and_applies_as_one_step() {
    use vectorcraft_effects::CLASSIC_REVOLVE;
    let mut h = Harness::with("M100 40 L130 60 L130 140 L100 160", CLASSIC_REVOLVE, vec2(1400.0, 1000.0));
    let before = h.app.session.doc().unwrap().doc.clone();
    h.settle();
    for text in ["Angle", "Cap", "Offset", "Axis side", "Left"] {
        h.rect(text);
    }
    assert!(!h.texts.iter().any(|(t, _)| t == "Segments" || t == "Revolve angle" || t == "Depth"), "the other effects' options stay out");
    h.app.ui.dialog.as_mut().unwrap().fields.insert("__tab".into(), json!("rotation"));
    h.settle();
    for text in ["Presets", "Rotation X", "Perspective", "Off-Axis Front"] {
        h.rect(text);
    }
    h.click(h.rect("Lighting").center());
    h.settle();
    for text in ["Surface", "Plastic", "Move Light to Back", "Light Intensity"] {
        h.rect(text);
    }
    h.scroll_to_end("Light Intensity");
    for text in ["Ambient Light", "Highlight Size", "Blend Steps", "Shading Color"] {
        h.rect(text);
    }
    assert!(!h.texts.iter().any(|(t, _)| t == "Draw Hidden Faces"), "every face is drawn");
    assert_eq!(dialogs::revolve_gizmo::hint(h.app.ui.dialog.as_ref().unwrap()), "Drag the light, or its rings. Shift snaps to 15°.");
    let d = h.app.ui.dialog.as_mut().unwrap();
    d.fields.insert("edge".into(), json!("right"));
    d.fields.insert("offset".into(), json!(12.0));
    d.fields.insert("surface".into(), json!("wireframe"));
    h.settle();
    assert_eq!(dialogs::revolve_gizmo::hint(h.app.ui.dialog.as_ref().unwrap()), "Drag values or labels. Double-click a value to type.");
    dialogs::confirm(&mut h.app).unwrap();
    let state = h.app.session.doc().unwrap();
    let fx = &state.doc.node(state.selection.objects[0]).unwrap().appearance.effects[0];
    assert_eq!(fx.id, CLASSIC_REVOLVE);
    assert_eq!(
        (fx.params["edge"].clone(), fx.params["offset"].as_f64(), fx.params["surface"].clone()),
        (json!("right"), Some(12.0), json!("wireframe"))
    );
    h.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(h.app.session.doc().unwrap().doc, before);
}

/// Rotate (Classic): Position on the Rotation tab, with no Object tab, and No Shading or Diffuse
/// on the Lighting tab, without the highlight or Draw Hidden Faces; OK keeps the edit as one undo
/// step.
#[test]
fn classic_rotate_shows_its_options_and_applies_as_one_step() {
    use vectorcraft_effects::CLASSIC_ROTATE;
    let mut h = Harness::with("M100 100 L200 100 L200 200 L100 200 Z", CLASSIC_ROTATE, vec2(1400.0, 1000.0));
    let before = h.app.session.doc().unwrap().doc.clone();
    h.settle();
    for text in ["Rotation", "Presets", "Rotation X", "Perspective", "Off-Axis Front"] {
        h.rect(text);
    }
    assert!(!h.texts.iter().any(|(t, _)| t == "Object" || t == "Depth" || t == "Angle"), "no Object tab");
    h.click(h.rect("Lighting").center());
    h.settle();
    for text in ["Surface", "Diffuse", "Move Light to Back", "Light Intensity"] {
        h.rect(text);
    }
    h.scroll_to_end("Light Intensity");
    for text in ["Ambient Light", "Blend Steps", "Shading Color"] {
        h.rect(text);
    }
    assert!(
        !h.texts.iter().any(|(t, _)| t == "Highlight Intensity" || t == "Highlight Size" || t == "Draw Hidden Faces"),
        "no highlight and no hidden faces"
    );
    assert_eq!(dialogs::revolve_gizmo::hint(h.app.ui.dialog.as_ref().unwrap()), "Drag the light, or its rings. Shift snaps to 15°.");
    let d = h.app.ui.dialog.as_mut().unwrap();
    d.fields.insert("surface".into(), json!("none"));
    d.fields.insert("rotationY".into(), json!(40.0));
    h.settle();
    assert_eq!(dialogs::revolve_gizmo::hint(h.app.ui.dialog.as_ref().unwrap()), "Drag values or labels. Double-click a value to type.");
    dialogs::confirm(&mut h.app).unwrap();
    let state = h.app.session.doc().unwrap();
    let fx = &state.doc.node(state.selection.objects[0]).unwrap().appearance.effects[0];
    assert_eq!(fx.id, CLASSIC_ROTATE);
    assert_eq!((fx.params["surface"].clone(), fx.params["rotationY"].as_f64()), (json!("none"), Some(40.0)));
    assert!(fx.params.get("highlightSize").is_none() && fx.params.get("drawHiddenFaces").is_none());
    h.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(h.app.session.doc().unwrap().doc, before);
}

/// The classic effects' track cube: a drag on its face turns the object live and the presets show
/// Custom, Escape during a drag puts the angles back, the ring turns Z alone, the arrow keys turn
/// it with the keyboard focus, and OK keeps it all as one undo step. The newer effects have no cube.
#[test]
fn the_track_cube_turns_the_classic_effects_live_in_one_undo_step() {
    use vectorcraft_effects::{CLASSIC_EXTRUDE, CLASSIC_REVOLVE, CLASSIC_ROTATE, EXTRUDE, ROTATE};
    let square = "M100 100 L200 100 L200 200 L100 200 Z";
    for effect in [EXTRUDE, ROTATE] {
        let mut h = Harness::with(square, effect, vec2(1400.0, 1000.0));
        h.app.ui.dialog.as_mut().unwrap().fields.insert("__tab".into(), json!("rotation"));
        h.settle();
        assert!(!h.texts.iter().any(|(t, _)| t == "Track cube"), "{effect} has no track cube");
    }
    for effect in [CLASSIC_EXTRUDE, CLASSIC_REVOLVE] {
        let mut h = Harness::with(square, effect, vec2(1400.0, 1000.0));
        h.app.ui.dialog.as_mut().unwrap().fields.insert("__tab".into(), json!("rotation"));
        h.settle();
        h.rect("Track cube");
    }
    let mut h = Harness::with(square, CLASSIC_ROTATE, vec2(1400.0, 1000.0));
    let before = h.app.session.doc().unwrap().doc.clone();
    h.settle();
    h.rect("Off-Axis Front");
    let start = ["rotationX", "rotationY", "rotationZ"].map(|k| h.value(k));
    let cube = h.ctx.read_response(h.cube_id()).unwrap().rect;
    // A face, dragged right: the object turns, the document previews it, and the preset is Custom.
    h.drag(cube.center(), vec2(30.0, 0.0));
    let turned = ["rotationX", "rotationY", "rotationZ"].map(|k| h.value(k));
    assert_ne!(turned, start, "the drag turned the object");
    h.rect("Custom");
    let fx = |h: &Harness| {
        let state = h.app.session.doc().unwrap();
        state.doc.node(state.selection.objects[0]).unwrap().appearance.effects[0].params.clone()
    };
    assert_eq!(fx(&h)["rotationY"].as_f64(), Some(turned[1]), "the preview follows the cube");
    // Escape during a drag puts the angles back and keeps the dialog open.
    let p = cube.center();
    h.frame(vec![Event::PointerMoved(p), button(p, true)]);
    h.frame(vec![Event::PointerMoved(p + vec2(0.0, 30.0))]);
    h.frame(vec![Event::PointerMoved(p + vec2(0.0, 40.0))]);
    assert_ne!(h.value("rotationX"), turned[0]);
    h.frame(vec![Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
    h.frame(vec![button(p + vec2(0.0, 40.0), false)]);
    h.settle();
    assert!(h.app.ui.dialog.is_some(), "Escape ends the drag, not the dialog");
    assert_eq!(["rotationX", "rotationY", "rotationZ"].map(|k| h.value(k)), turned);
    // The ring turns in the screen plane: Z alone.
    let top = pos2(cube.center().x, cube.center().y - 53.5);
    h.drag(top, vec2(12.0, 2.0));
    assert_eq!((h.value("rotationX"), h.value("rotationY")), (turned[0], turned[1]));
    assert!(h.value("rotationZ") > turned[2], "clockwise: {} -> {}", turned[2], h.value("rotationZ"));
    // With the keyboard focus (the drags gave it), Right turns about the screen's vertical axis.
    let z = h.value("rotationZ");
    assert!(h.ctx.memory(|m| m.has_focus(h.cube_id())), "the drag gave the cube the keyboard focus");
    h.frame(vec![Event::Key { key: egui::Key::PageUp, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
    h.settle();
    assert!((h.value("rotationZ") - (z - 1.0)).abs() < 0.011, "Page Up turns 1° counterclockwise");
    let y = h.value("rotationY");
    h.frame(vec![Event::Key { key: egui::Key::ArrowRight, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
    h.settle();
    assert_ne!(h.value("rotationY"), y, "Right turns the cube");
    let last = h.value("rotationZ");
    dialogs::confirm(&mut h.app).unwrap();
    assert_eq!(fx(&h)["rotationZ"].as_f64(), Some(last));
    h.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(h.app.session.doc().unwrap().doc, before);
}

/// A mixed rotation (the 3D and Materials panel's selected effects differ) leaves the cube off.
#[test]
fn the_track_cube_is_off_while_a_rotation_is_mixed() {
    use vectorcraft_effects::CLASSIC_ROTATE;
    let mut h = Harness::with("M100 100 L200 100 L200 200 L100 200 Z", CLASSIC_ROTATE, vec2(1400.0, 1000.0));
    h.app.ui.dialog.as_mut().unwrap().fields.insert("rotationX".into(), Value::Null);
    h.settle();
    let cube = h.ctx.read_response(h.cube_id()).unwrap().rect;
    h.drag(cube.center(), vec2(30.0, 0.0));
    let d = h.app.ui.dialog.as_ref().unwrap();
    assert_eq!(d.fields.get("rotationX"), Some(&Value::Null));
    assert!(!h.ctx.read_response(h.cube_id()).unwrap().sense.senses_drag());
}

#[test]
fn extrude_bevel_options_enable_with_a_bevel_shape() {
    let mut h = Harness::with("M100 100 L200 100 L200 200 L100 200 Z", vectorcraft_effects::EXTRUDE, vec2(1400.0, 1000.0));
    h.settle();
    let width = h.response(("revolve-reset", "bevelWidth"));
    assert!(!h.ctx.read_response(Id::unique(("revolve-reset", "bevelWidth"))).unwrap().enabled(), "no bevel: its sizes are off");
    h.app.ui.dialog.as_mut().unwrap().fields.insert("bevel".into(), json!("rounded"));
    h.settle();
    assert!(h.ctx.read_response(Id::unique(("revolve-reset", "bevelWidth"))).unwrap().enabled());
    assert_eq!(h.response(("revolve-reset", "bevelWidth")), width);
}

#[test]
fn the_bevel_reset_restores_the_bevel_settings_and_keeps_the_others() {
    let mut h = Harness::with("M100 100 L200 100 L200 200 L100 200 Z", vectorcraft_effects::EXTRUDE, vec2(1400.0, 1000.0));
    let before = h.app.session.doc().unwrap().doc.clone();
    h.settle();
    let enabled = |h: &Harness| h.ctx.read_response(Id::unique("revolve-reset-bevel")).unwrap().enabled();
    assert!(!enabled(&h), "every bevel setting at its default: Reset is off");
    let changed = json!({"bevel": "step", "bevelWidth": 12.0, "bevelHeight": 3.0, "bevelRepeat": 4, "bevelSpace": 30.0, "bevelInside": true, "bevelBothSides": true, "depth": 90.0, "twist": 45.0, "rotationX": 10.0});
    h.app.ui.dialog.as_mut().unwrap().fields.extend(changed.as_object().unwrap().clone());
    h.settle();
    assert!(enabled(&h));
    h.click(h.response("revolve-reset-bevel").center());
    let defaults = vectorcraft_effects::default_params(vectorcraft_effects::EXTRUDE).unwrap();
    let dialog = h.app.ui.dialog.as_ref().unwrap();
    for key in ["bevel", "bevelWidth", "bevelHeight", "bevelRepeat", "bevelSpace", "bevelInside", "bevelBothSides"] {
        assert_eq!(dialog.fields.get(key), defaults.get(key), "reset: {key}");
    }
    for key in ["depth", "twist", "rotationX"] {
        assert_eq!(dialog.fields.get(key), changed.get(key), "kept: {key}");
    }
    assert!(!enabled(&h), "Reset turns itself off");
    // One bevel setting off its default is enough to turn Reset on again.
    h.app.ui.dialog.as_mut().unwrap().fields.insert("bevelBothSides".into(), json!(true));
    h.settle();
    assert!(enabled(&h));
    dialogs::confirm(&mut h.app).unwrap();
    let state = h.app.session.doc().unwrap();
    let params = vectorcraft_effects::merged_params(
        vectorcraft_effects::EXTRUDE,
        &state.doc.node(state.selection.objects[0]).unwrap().appearance.effects[0].params,
    );
    assert_eq!((params["depth"].clone(), params["bevelBothSides"].clone(), params["bevel"].clone()), (json!(90.0), json!(true), json!("none")));
    h.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(h.app.session.doc().unwrap().doc, before);
}

#[test]
fn cap_checkbox_previews_resets_and_respects_cancel_and_one_step_undo() {
    for confirm in [false, true] {
        let mut h = Harness::new(vec2(1280.0, 1000.0));
        let before = h.app.session.doc().unwrap().doc.clone();
        let dialog = h.app.ui.dialog.as_mut().unwrap();
        dialog.fields.insert("offset".into(), json!(20));
        dialog.fields.insert("rotationX".into(), json!(-35));
        h.settle();
        let art = |h: &Harness| {
            let state = h.app.session.doc().unwrap();
            vectorcraft_effects::revolve_art(state.doc.node(state.selection.objects[0]).unwrap()).unwrap()
        };
        assert!(h.app.ui.dialog.as_ref().unwrap().bool("cap"), "Cap starts on");
        let capped = art(&h);
        h.click(h.rect("Cap").center());
        assert!(!h.app.ui.dialog.as_ref().unwrap().bool("cap"));
        assert_ne!(art(&h), capped);
        h.click(h.response(("revolve-reset", "cap")).center());
        assert!(h.app.ui.dialog.as_ref().unwrap().bool("cap"));
        assert_eq!(art(&h), capped);
        h.click(h.rect("Cap").center());
        h.click(h.response("revolve-reset-all").center());
        assert!(h.app.ui.dialog.as_ref().unwrap().bool("cap"));
        h.click(h.rect("Cap").center());
        if confirm {
            dialogs::confirm(&mut h.app).unwrap();
            let state = h.app.session.doc().unwrap();
            assert_eq!(state.doc.node(state.selection.objects[0]).unwrap().appearance.effects[0].params["cap"], json!(false));
            let loaded = vectorcraft_format::load(&vectorcraft_format::save(&state.doc, false)).unwrap();
            assert_eq!(loaded.node(state.selection.objects[0]).unwrap().appearance.effects[0].params["cap"], json!(false));
            h.app.run("edit.undo", json!({})).unwrap();
        } else {
            dialogs::cancel(&mut h.app);
        }
        assert_eq!(h.app.session.doc().unwrap().doc, before);
        assert!(!h.app.session.in_interaction());
    }
}

#[test]
fn light_picker_previews_and_reset_dot_and_cancel_restore_white() {
    let mut h = Harness::new(vec2(1280.0, 900.0));
    let before = h.app.session.doc().unwrap().doc.clone();
    h.settle();
    h.click(h.rect("Lighting").center());
    h.click(pos2(h.rect("#FFFFFF").left() - 18.0, h.rect("Color").center().y));
    let green =
        h.texts.iter().find(|(s, _)| s.starts_with("G ")).map(|(_, r)| *r).unwrap_or_else(|| panic!("the opaque RGB picker opens: {:?}", h.texts));
    h.drag(green.center(), vec2(-100.0, 0.0));
    let color = h.app.ui.dialog.as_ref().unwrap().str("lightColor");
    assert_ne!(color, "#ffffff");
    let st = h.app.session.doc().unwrap();
    assert_eq!(st.doc.node(st.selection.objects[0]).unwrap().appearance.effects[0].params["lightColor"], json!(color));
    h.click(h.rect("Color").center());
    h.click(h.response(("revolve-reset", "lightColor")).center());
    assert_eq!(h.app.ui.dialog.as_ref().unwrap().str("lightColor"), "#ffffff");
    dialogs::cancel(&mut h.app);
    assert_eq!(h.app.session.doc().unwrap().doc, before);
}

#[test]
fn every_3d_dialog_has_color_and_reset_all_restores_white() {
    for id in [REVOLVE, EXTRUDE, INFLATE, vectorcraft_effects::ROTATE] {
        let mut h = Harness::with("M100 100L200 100L200 200L100 200Z", id, vec2(1280.0, 900.0));
        h.settle();
        h.click(h.rect("Lighting").center());
        h.app.ui.dialog.as_mut().unwrap().fields.insert("lightColor".into(), json!("blue"));
        h.settle();
        assert!(!h.rect("#FFFFFF").is_negative(), "invalid stored colors show the renderer's white fallback");
        h.app.ui.dialog.as_mut().unwrap().fields.insert("lightColor".into(), json!("#ff8000"));
        h.settle();
        assert!(!h.rect("Color").is_negative());
        assert!(!h.rect("#FF8000").is_negative());
        h.click(h.response("revolve-reset-all").center());
        assert_eq!(h.app.ui.dialog.as_ref().unwrap().str("lightColor"), "#ffffff");
        dialogs::confirm(&mut h.app).unwrap();
    }
}

#[test]
fn reset_all_keeps_map_art_and_its_options() {
    for effect in [CLASSIC_EXTRUDE, CLASSIC_REVOLVE] {
        let mut h = Harness::with("M100 100L200 100L200 200L100 200Z", effect, vec2(1280.0, 900.0));
        h.settle();
        let maps = json!([{"surface": 1, "symbol": "Mark"}]);
        let d = h.app.ui.dialog.as_mut().unwrap();
        d.fields.insert("mapArt".into(), maps.clone());
        d.fields.insert("mapShade".into(), json!(true));
        d.fields.insert("mapInvisible".into(), json!(true));
        d.fields.insert("rotationX".into(), json!(40.0));
        h.settle();
        h.click(h.response("revolve-reset-all").center());
        let d = h.app.ui.dialog.as_ref().unwrap();
        assert_eq!(d.f64("rotationX", 0.0), -21.88, "{effect}: the view is reset");
        assert_eq!(d.fields.get("mapArt"), Some(&maps), "{effect}: the maps stay");
        assert!(d.bool("mapShade") && d.bool("mapInvisible"), "{effect}: Shade Artwork and Invisible Geometry stay");
        dialogs::cancel(&mut h.app);
    }
}

#[test]
fn light_list_add_select_edit_delete_and_reset_keep_each_light_independent() {
    for effect in [REVOLVE, EXTRUDE, INFLATE, vectorcraft_effects::ROTATE] {
        let mut h = Harness::with("M100 100L200 100L200 200L100 200Z", effect, vec2(1280.0, 900.0));
        let before = h.app.session.doc().unwrap().doc.clone();
        h.settle();
        h.click(h.rect("Lighting").center());
        let one = h.app.session.doc().unwrap().doc.clone();
        h.click(h.rect("Delete Light").center());
        assert_eq!(h.app.session.doc().unwrap().doc, one, "retain at least one light");
        h.click(h.rect("Add Light").center());
        let d = h.app.ui.dialog.as_ref().unwrap();
        let first = d.fields["lights"][0].clone();
        assert_eq!(d.fields["lights"].as_array().unwrap().len(), 2);
        assert_eq!(d.fields["__light"], 1);
        assert_eq!(selected_light(d).f64("lightHeight", 0.0), 90.0);
        h.drag(h.rect("Height").center(), vec2(-35.0, 0.0));
        let d = h.app.ui.dialog.as_ref().unwrap();
        assert_ne!(d.fields["lights"][1]["height"], 90);
        assert_eq!(d.fields["lights"][0], first);
        let two = h.app.session.doc().unwrap().doc.clone();
        h.click(h.rect("Light 2").center());
        h.click(h.rect("Light 1").center());
        assert_eq!(h.app.session.doc().unwrap().doc, two, "selection changes no effect params");
        assert_eq!(h.app.ui.dialog.as_ref().unwrap().fields["__light"], 0);
        h.click(h.rect("Delete Light").center());
        assert_eq!(h.app.ui.dialog.as_ref().unwrap().fields["lights"].as_array().unwrap().len(), 1);
        h.click(h.response("revolve-reset-all").center());
        assert!(h.app.ui.dialog.as_ref().unwrap().fields["lights"].is_null());
        h.click(h.rect("Add Light").center());
        dialogs::confirm(&mut h.app).unwrap();
        h.app.run("edit.undo", json!({})).unwrap();
        assert_eq!(h.app.session.doc().unwrap().doc, before, "whole dialog is one undo step");
    }
}

#[test]
fn light_list_stops_at_ten_and_cancel_restores_the_source() {
    let mut h = Harness::new(vec2(1280.0, 900.0));
    let before = h.app.session.doc().unwrap().doc.clone();
    h.settle();
    h.click(h.rect("Lighting").center());
    for _ in 0..11 {
        h.click(h.rect("Add Light").center());
    }
    let d = h.app.ui.dialog.as_ref().unwrap();
    assert_eq!(d.fields["lights"].as_array().unwrap().len(), 10);
    assert_eq!(d.fields["__light"], 9);
    dialogs::cancel(&mut h.app);
    assert_eq!(h.app.session.doc().unwrap().doc, before);
}

#[test]
fn lighting_presets_preview_custom_edits_reset_and_commit_as_one_undo_step() {
    for effect in [REVOLVE, EXTRUDE, INFLATE, vectorcraft_effects::ROTATE] {
        let mut h = Harness::with("M100 100L200 100L200 200L100 200Z", effect, vec2(1280.0, 900.0));
        let before = h.app.session.doc().unwrap().doc.clone();
        h.settle();
        h.click(h.rect("Lighting").center());
        let view = h.app.ui.dialog.as_ref().unwrap().fields["rotationX"].clone();
        h.click(h.rect("Standard").center());
        h.click(h.rect("Right").center());
        assert_eq!(h.value("lightRotation"), 0.0);
        assert_eq!(h.app.ui.dialog.as_ref().unwrap().fields["rotationX"], view);
        h.drag(h.rect("Height").center(), vec2(20.0, 0.0));
        assert!(!h.rect("Custom").is_negative());
        h.click(h.rect("Custom").center());
        h.click(h.rect("Diffuse").center());
        assert_eq!(h.value("lightSoftness"), 100.0);
        assert_eq!(h.value("ambient"), 35.0);
        h.click(h.rect("Add Light").center());
        assert!(!h.rect("Custom").is_negative());
        h.click(h.rect("Custom").center());
        h.click(h.rect("Top Left").center());
        let d = h.app.ui.dialog.as_ref().unwrap();
        assert!(d.fields["lights"].is_null());
        assert_eq!(d.fields["__light"], 0);
        assert_eq!(d.f64("lightRotation", 0.0), 135.0);
        h.click(h.response("revolve-reset-all").center());
        assert!(!h.rect("Standard").is_negative());
        h.click(h.rect("Standard").center());
        h.click(h.rect("Right").center());
        dialogs::confirm(&mut h.app).unwrap();
        h.app.run("edit.undo", json!({})).unwrap();
        assert_eq!(h.app.session.doc().unwrap().doc, before);
    }
}

#[test]
fn lighting_presets_cancel_restores_an_existing_custom_light_rig() {
    let mut h = Harness::new(vec2(1280.0, 900.0));
    h.settle();
    dialogs::confirm(&mut h.app).unwrap();
    h.app
        .run(
            "effect.setParams",
            json!({"index":0,"item":null,"params":{
                "ambient":19,"lights":[{"color":"#ff8040"},{"color":"#4080ff"}]
            }}),
        )
        .unwrap();
    let before = h.app.session.doc().unwrap().doc.clone();
    h.app.run("effect.dialog", json!({"effect":REVOLVE,"index":0,"item":null})).unwrap();
    h.settle();
    h.click(h.rect("Lighting").center());
    h.click(h.rect("Custom").center());
    h.click(h.rect("Right").center());
    assert_ne!(h.app.session.doc().unwrap().doc, before);
    dialogs::cancel(&mut h.app);
    assert_eq!(h.app.session.doc().unwrap().doc, before);
}
