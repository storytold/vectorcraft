//! Effect > Warp uses the existing modal and live-effect commands, without changing envelope art.
use egui::{Event, Pos2, Rect, Shape, pos2, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_engine::Session;

use super::*;

fn app() -> (VectorcraftApp, NodeId) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 50, "y": 60, "width": 120, "height": 80})).unwrap()["id"].as_u64().unwrap();
    (app, NodeId(id))
}

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    crate::theme::apply(&ctx, Default::default());
    ctx
}

fn frame_output(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) -> egui::FullOutput {
    let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))), events, ..Default::default() };
    let mut output = ctx.run_ui(input, |ui| show(app, ui.ctx()));
    output.textures_delta.clear();
    output
}

fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) -> Vec<(String, Rect)> {
    let output = frame_output(app, ctx, events);
    fn collect(shape: &Shape, clip: Rect, texts: &mut Vec<(String, Rect)>) {
        match shape {
            Shape::Text(text) => {
                let rect = text.visual_bounding_rect();
                if clip.contains(rect.center()) {
                    texts.push((text.galley.text().to_string(), rect));
                }
            }
            Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, clip, texts)),
            _ => {}
        }
    }
    let mut texts = vec![];
    for shape in &output.shapes {
        collect(&shape.shape, shape.clip_rect, &mut texts);
    }
    texts
}

fn settle(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<(String, Rect)> {
    for _ in 0..3 {
        frame(app, ctx, vec![]);
    }
    frame(app, ctx, vec![])
}

fn click_text(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str) {
    let texts = frame(app, ctx, vec![]);
    let at = texts.iter().find(|(text, _)| text == label).unwrap_or_else(|| panic!("missing {label:?}: {texts:?}")).1.center();
    click_at(app, ctx, at);
}

fn click_at(app: &mut VectorcraftApp, ctx: &egui::Context, at: Pos2) {
    for event in [
        Event::PointerMoved(at),
        Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() },
        Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() },
    ] {
        frame(app, ctx, vec![event]);
    }
}

fn set(app: &mut VectorcraftApp, values: Value) {
    // Supply dialog inputs as the control channel does; the production frame previews them.
    app.ui.dialog.as_mut().unwrap().fields.extend(values.as_object().unwrap().clone());
}

#[test]
fn warp_options_controls_cover_every_existing_style() {
    for (style, label) in vectorcraft_effects::WARP_STYLES {
        let (mut app, _) = app();
        app.run("effect.dialog", json!({"effect": format!("warp.{style}"), "item": null})).unwrap();
        let ctx = context();
        let texts = settle(&mut app, &ctx);
        for want in [
            "Warp Options",
            "Style:",
            label.trim_end_matches('…'),
            "Horizontal",
            "Vertical",
            "Bend:",
            "Distortion",
            "Horizontal:",
            "Vertical:",
            "Preview",
            "OK",
            "Cancel",
        ] {
            assert!(texts.iter().any(|(text, _)| text == want), "{style}: missing {want:?}: {texts:?}");
        }
        assert!(!texts.iter().any(|(text, _)| text == "Orientation:"));
        cancel(&mut app);
    }
}

#[test]
fn warp_options_style_orientation_clicks_preview_and_cancel_preserve_redo() {
    let (mut app, id) = app();
    app.run("object.move", json!({"dx": 10, "dy": 0})).unwrap();
    app.run("edit.undo", json!({})).unwrap();
    app.session.doc_mut().unwrap().mark_saved();
    let before = app.session.doc().unwrap().doc.clone();
    let selection = app.session.doc().unwrap().selection.clone();
    let undo = app.session.doc().unwrap().history.undo.len();
    let redo = app.session.doc().unwrap().history.redo.len();
    assert_eq!(redo, 1);
    app.run("effect.dialog", json!({"effect": "warp.arc", "item": null})).unwrap();
    let ctx = context();
    settle(&mut app, &ctx);
    click_text(&mut app, &ctx, "Arc");
    click_text(&mut app, &ctx, "Arc Lower");
    click_text(&mut app, &ctx, "Vertical");
    set(&mut app, json!({"bend": -30.0, "horizontal": 20.0, "vertical": -15.0}));
    frame(&mut app, &ctx, vec![Event::PointerMoved(pos2(10.0, 10.0))]);
    let state = app.session.doc().unwrap();
    let effects = &state.doc.node(id).unwrap().appearance.effects;
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].id, "warp.arcLower");
    assert_eq!(effects[0].params["orientation"], "vertical");
    assert_eq!(effects[0].params["bend"], -30.0);
    assert_eq!(effects[0].params["horizontal"], 20.0);
    assert_eq!(effects[0].params["vertical"], -15.0);
    assert_eq!(state.history.undo.len(), undo);
    click_text(&mut app, &ctx, "Cancel");
    let state = app.session.doc().unwrap();
    assert_eq!(state.doc, before);
    assert_eq!(state.selection, selection);
    assert_eq!((state.history.undo.len(), state.history.redo.len()), (undo, redo));
    assert!(!state.is_dirty());
    assert!(!app.session.in_interaction() && app.ui.dialog.is_none());
    app.run("edit.redo", json!({})).unwrap();
    assert_ne!(app.session.doc().unwrap().doc, before, "the original Redo remains usable");
}

#[test]
fn warp_options_existing_style_edit_keeps_stack_target_visibility_and_history() {
    for item in [Value::Null, json!(1)] {
        let (mut app, id) = app();
        for effect in ["path.offsetPath", "warp.arc", "stylize.roundCorners"] {
            app.run("effect.apply", json!({"effect": effect, "item": item})).unwrap();
        }
        app.run("effect.setParams", json!({"index": 1, "item": item, "visible": false, "params": {"bend": -25.0}})).unwrap();
        app.session.doc_mut().unwrap().mark_saved();
        let before = app.session.doc().unwrap().doc.clone();
        let selection = app.session.doc().unwrap().selection.clone();
        let undo = app.session.doc().unwrap().history.undo.len();
        let last_effect = app.last_effect.clone();
        app.run("effect.dialog", json!({"effect": "warp.arc", "index": 1, "item": item})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields["bend"], -25.0);
        let ctx = context();
        settle(&mut app, &ctx);
        click_text(&mut app, &ctx, "Arc");
        click_text(&mut app, &ctx, "Arc Lower");
        set(&mut app, json!({"bend": 35.0}));
        frame(&mut app, &ctx, vec![]);
        click_text(&mut app, &ctx, "OK");
        let state = app.session.doc().unwrap();
        let target = item.as_u64().map(|n| n as usize);
        let old = before.node(id).unwrap().appearance.effects_at(target).unwrap();
        let new = state.doc.node(id).unwrap().appearance.effects_at(target).unwrap();
        assert_eq!(new.len(), old.len());
        assert_eq!(new[0], old[0]);
        assert_eq!(new[2], old[2]);
        assert_eq!(new[1].id, "warp.arcLower");
        assert_eq!(new[1].params["bend"], 35.0);
        assert!(!new[1].visible);
        assert_eq!(state.selection, selection);
        assert_eq!(state.history.undo.len(), undo + 1);
        assert_eq!(app.last_effect, last_effect, "editing does not replace Last Effect");
        let after = state.doc.clone();
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc, before);
        assert!(!app.session.doc().unwrap().is_dirty());
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc, after);
    }
}

/// A new Warp Options dialog opens on the sliders confirmed last time (#1760 follow-up).
#[test]
fn warp_dialog_remembers_last_confirmed_sliders() {
    let (mut app, _) = app();
    app.run("effect.dialog", json!({"effect": "warp.arc", "item": null})).unwrap();
    let ctx = context();
    settle(&mut app, &ctx);
    set(&mut app, json!({"bend": -40.0, "horizontal": 12.0, "vertical": -8.0, "orientation": "vertical"}));
    confirm(&mut app).unwrap();
    app.run("effect.dialog", json!({"effect": "warp.flag", "item": null})).unwrap();
    settle(&mut app, &ctx);
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(d.str("__effect"), "warp.flag");
    assert_eq!(d.f64("bend", 0.0), -40.0);
    assert_eq!(d.f64("horizontal", 0.0), 12.0);
    assert_eq!(d.f64("vertical", 0.0), -8.0);
    assert_eq!(d.str("orientation"), "vertical");
}

#[test]
fn warp_options_preview_toggle_and_ok_use_latest_values() {
    let (mut app, id) = app();
    let before = app.session.doc().unwrap().doc.clone();
    let undo = app.session.doc().unwrap().history.undo.len();
    app.run("effect.dialog", json!({"effect": "warp.arc", "item": null})).unwrap();
    let ctx = context();
    settle(&mut app, &ctx);
    // Exercise the checkbox transition; assigning its stored boolean bypasses the click.
    click_text(&mut app, &ctx, "Preview");
    assert!(!app.ui.dialog.as_ref().unwrap().bool("preview"));
    assert_eq!(app.session.doc().unwrap().doc, before, "clicking Preview off rolls back");
    set(&mut app, json!({"bend": 75.0}));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.doc().unwrap().doc, before, "turning Preview off rolls back");
    click_text(&mut app, &ctx, "Preview");
    assert!(app.ui.dialog.as_ref().unwrap().bool("preview"));
    assert_eq!(app.session.doc().unwrap().doc.node(id).unwrap().appearance.effects[0].params["bend"], 75.0);
    set(&mut app, json!({"bend": -40.0}));
    confirm(&mut app).unwrap();
    let state = app.session.doc().unwrap();
    assert_eq!(state.doc.node(id).unwrap().appearance.effects[0].params["bend"], -40.0);
    assert_eq!(state.history.undo.len(), undo + 1);
    assert_eq!(app.last_effect.as_ref().unwrap().1["bend"], -40.0);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, before);
}

/// Locate the actual painted number beside its row label, without requesting widget focus.
fn click_number(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str) {
    let texts = frame(app, ctx, vec![]);
    let row = texts.iter().find(|(text, _)| text == label).unwrap().1;
    let number = texts
        .iter()
        .find(|(text, rect)| {
            rect.left() > row.right() && (rect.center().y - row.center().y).abs() < 6.0 && text.trim_end_matches('%').parse::<f64>().is_ok()
        })
        .unwrap_or_else(|| panic!("no numeric field beside {label}: {texts:?}"));
    click_at(app, ctx, number.1.center());
}

fn replace_number(text: &str) -> Vec<Event> {
    let modifiers = egui::Modifiers { ctrl: true, command: true, ..Default::default() };
    vec![
        Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers },
        Event::Key { key: egui::Key::A, physical_key: None, pressed: false, repeat: false, modifiers },
        Event::Text(text.into()),
    ]
}

#[test]
fn warp_options_numeric_entry_immediate_accept_uses_latest_values() {
    for preview in [true, false] {
        for enter in [true, false] {
            let (mut app, id) = app();
            app.session.doc_mut().unwrap().mark_saved();
            let before = app.session.doc().unwrap().doc.clone();
            let selection = app.session.doc().unwrap().selection.clone();
            let undo = app.session.doc().unwrap().history.undo.len();
            app.run("effect.dialog", json!({"effect": "warp.arc", "item": null})).unwrap();
            let ctx = context();
            settle(&mut app, &ctx);
            if !preview {
                click_text(&mut app, &ctx, "Preview");
                assert_eq!(app.session.doc().unwrap().doc, before);
            }
            // Actual keyboard input: moving to the next field commits the previous one.
            click_number(&mut app, &ctx, "Horizontal:");
            frame(&mut app, &ctx, replace_number("23%"));
            click_number(&mut app, &ctx, "Vertical:");
            frame(&mut app, &ctx, replace_number("-19%"));
            click_number(&mut app, &ctx, "Bend:");
            let texts = frame(&mut app, &ctx, vec![]);
            let ok = texts.iter().find(|(text, _)| text == "OK").unwrap().1.center();
            if preview {
                let effect = &app.session.doc().unwrap().doc.node(id).unwrap().appearance.effects[0];
                assert_eq!(effect.params["horizontal"], 23.0);
                assert_eq!(effect.params["vertical"], -19.0);
                assert_eq!(effect.params["bend"], 50.0);
            } else {
                assert_eq!(app.session.doc().unwrap().doc, before, "typing with Preview off must not edit the document");
            }
            assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);
            let mut events = replace_number("-37%");
            if enter {
                // The final Text and Enter arrive in the same frame, with no idle preview first.
                events.push(Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
            }
            frame(&mut app, &ctx, events);
            if !enter {
                // Click immediately after typing; don't blur the field through another control.
                click_at(&mut app, &ctx, ok);
            }
            assert!(app.ui.dialog.is_none() && !app.session.in_interaction(), "preview={preview}, enter={enter}");
            let state = app.session.doc().unwrap();
            let effects = &state.doc.node(id).unwrap().appearance.effects;
            assert_eq!(effects.len(), 1);
            assert_eq!(effects[0].id, "warp.arc");
            assert_eq!(effects[0].params["bend"], -37.0);
            assert_eq!(effects[0].params["horizontal"], 23.0);
            assert_eq!(effects[0].params["vertical"], -19.0);
            assert_eq!(effects[0].params["orientation"], "horizontal");
            assert_eq!(state.selection, selection);
            assert_eq!(state.history.undo.len(), undo + 1);
            assert_eq!(app.last_effect.as_ref().unwrap().1["bend"], -37.0);
            let after = state.doc.clone();
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(app.session.doc().unwrap().doc, before);
            assert!(!app.session.doc().unwrap().is_dirty());
            app.run("edit.redo", json!({})).unwrap();
            assert_eq!(app.session.doc().unwrap().doc, after);
        }
    }
}

#[test]
fn warp_options_slider_drag_previews_exact_parameter_and_cancel_restores() {
    let (mut app, id) = app();
    app.session.doc_mut().unwrap().mark_saved();
    let before = app.session.doc().unwrap().doc.clone();
    let selection = app.session.doc().unwrap().selection.clone();
    let undo = app.session.doc().unwrap().history.undo.len();
    app.run("effect.dialog", json!({"effect": "warp.arc", "item": null})).unwrap();
    let ctx = context();
    let texts = settle(&mut app, &ctx);
    let row = texts.iter().find(|(text, _)| text == "Bend:").unwrap().1;
    // The real color_slider paints one outlined rail in this row. Locate that painted rail,
    // not a hardcoded screen position or an injected dialog value.
    fn rail(shape: &Shape, clip: Rect, row: Rect) -> Option<Rect> {
        match shape {
            Shape::Rect(shape)
                if shape.rect.width() > 80.0
                    && (shape.rect.height() - 7.0).abs() < 0.1
                    && shape.rect.left() > row.right()
                    && (shape.rect.center().y - row.center().y).abs() < 6.0
                    && clip.contains(shape.rect.center()) =>
            {
                Some(shape.rect)
            }
            Shape::Vec(shapes) => shapes.iter().find_map(|shape| rail(shape, clip, row)),
            _ => None,
        }
    }
    let output = frame_output(&mut app, &ctx, vec![]);
    let bar = output.shapes.iter().find_map(|shape| rail(&shape.shape, shape.clip_rect, row)).expect("Bend must have a visible slider rail");
    let from = pos2(bar.left() + 0.75 * bar.width(), bar.center().y);
    let to = pos2(bar.left() + 0.25 * bar.width(), bar.center().y);
    frame(&mut app, &ctx, vec![Event::PointerMoved(from)]);
    frame(
        &mut app,
        &ctx,
        vec![Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }],
    );
    frame(&mut app, &ctx, vec![Event::PointerMoved(to)]);
    let effect = &app.session.doc().unwrap().doc.node(id).unwrap().appearance.effects[0];
    assert_eq!(effect.params["bend"], -50.0, "the held slider must update the actual live effect");
    assert_eq!(effect.params["horizontal"], 0.0);
    assert_eq!(effect.params["vertical"], 0.0);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);
    frame(
        &mut app,
        &ctx,
        vec![Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }],
    );
    assert_eq!(app.session.doc().unwrap().doc.node(id).unwrap().appearance.effects[0].params["bend"], -50.0);
    click_text(&mut app, &ctx, "Cancel");
    let state = app.session.doc().unwrap();
    assert_eq!(state.doc, before);
    assert_eq!(state.selection, selection);
    assert_eq!(state.history.undo.len(), undo);
    assert!(!state.is_dirty());
    assert!(app.ui.dialog.is_none() && !app.session.in_interaction());
}

#[test]
fn warp_options_mixed_styles_change_only_after_explicit_style_choice() {
    for item in [Value::Null, json!(1)] {
        for edit in ["none", "bend", "style"] {
            for accept in [false, true] {
                let (mut app, first) = app();
                for effect in ["path.offsetPath", "warp.arc", "stylize.roundCorners"] {
                    app.run("effect.apply", json!({"effect": effect, "item": item})).unwrap();
                }
                let second =
                    NodeId(app.run("shape.rectangle", json!({"x": 230, "y": 60, "width": 120, "height": 80})).unwrap()["id"].as_u64().unwrap());
                for effect in ["path.offsetPath", "warp.flag", "stylize.roundCorners"] {
                    app.run("effect.apply", json!({"effect": effect, "item": item})).unwrap();
                }
                app.run("effect.setParams", json!({"index": 1, "item": item, "visible": false})).unwrap();
                app.run("select.set", json!({"ids": [first.0, second.0]})).unwrap();
                app.run("object.move", json!({"dx": 5, "dy": 0})).unwrap();
                app.run("edit.undo", json!({})).unwrap();
                app.session.doc_mut().unwrap().mark_saved();
                let before = app.session.doc().unwrap().doc.clone();
                let selection = app.session.doc().unwrap().selection.clone();
                let undo = app.session.doc().unwrap().history.undo.len();
                let redo = app.session.doc().unwrap().history.redo.len();
                assert_eq!(redo, 1);
                let last_effect = app.last_effect.clone();
                let mut expected = before.clone();
                for id in [first, second] {
                    let node = std::sync::Arc::make_mut(&mut expected).node_mut(id).unwrap();
                    let effects = node.appearance.effects_mut(item.as_u64().map(|i| i as usize)).unwrap();
                    match edit {
                        "bend" => effects[1].params["bend"] = json!(25.0),
                        "style" => effects[1].id = "warp.arcLower".into(),
                        _ => {}
                    }
                }
                // Ordinary Effect menu route: first object's Arc is already present, then Edit.
                app.run("effect.dialog", json!({"effect": "warp.arc", "item": item})).unwrap();
                assert_eq!(app.ui.dialog.as_ref().unwrap().kind, effect::EXISTS);
                let ctx = context();
                settle(&mut app, &ctx);
                click_text(&mut app, &ctx, "Edit");
                settle(&mut app, &ctx);
                assert_eq!(app.session.doc().unwrap().doc, before, "opening must not broadcast the first target's style");
                match edit {
                    "bend" => {
                        click_number(&mut app, &ctx, "Bend:");
                        frame(&mut app, &ctx, replace_number("25%"));
                        // Blur via the existing orientation choice; it stays Horizontal.
                        click_text(&mut app, &ctx, "Horizontal");
                    }
                    "style" => {
                        click_text(&mut app, &ctx, "Arc");
                        click_text(&mut app, &ctx, "Arc Lower");
                    }
                    _ => {}
                }
                assert_eq!(app.session.doc().unwrap().doc, expected, "edit={edit}, item={item}");
                assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);
                click_text(&mut app, &ctx, "Preview");
                assert!(!app.ui.dialog.as_ref().unwrap().bool("preview"));
                assert_eq!(app.session.doc().unwrap().doc, before);
                click_text(&mut app, &ctx, "Preview");
                assert!(app.ui.dialog.as_ref().unwrap().bool("preview"));
                assert_eq!(app.session.doc().unwrap().doc, expected, "Style intent survives Preview off/on");
                click_text(&mut app, &ctx, if accept { "OK" } else { "Cancel" });
                assert!(app.ui.dialog.is_none() && !app.session.in_interaction());
                let state = app.session.doc().unwrap();
                assert_eq!(state.selection, selection);
                assert_eq!(app.last_effect, last_effect);
                if accept {
                    // Whole-document equality also checks both targets, sibling effects and
                    // the first visible / second hidden Warp. Existing OK history stays one step.
                    assert_eq!(state.doc, expected);
                    assert_eq!(state.history.undo.len(), undo + 1);
                    app.run("edit.undo", json!({})).unwrap();
                    assert_eq!(app.session.doc().unwrap().doc, before);
                    assert!(!app.session.doc().unwrap().is_dirty());
                    app.run("edit.redo", json!({})).unwrap();
                    assert_eq!(app.session.doc().unwrap().doc, expected);
                } else {
                    assert_eq!(state.doc, before);
                    assert_eq!((state.history.undo.len(), state.history.redo.len()), (undo, redo));
                    assert!(!state.is_dirty());
                    app.run("edit.redo", json!({})).unwrap();
                    assert_ne!(app.session.doc().unwrap().doc, before, "Cancel keeps the original Redo usable");
                }
            }
        }
    }
}

fn warp_key(app: &mut VectorcraftApp, ctx: &egui::Context, key: egui::Key, modifiers: egui::Modifiers) {
    frame(app, ctx, vec![Event::ModifiersChanged(modifiers), Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }]);
    frame(
        app,
        ctx,
        vec![Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers }, Event::ModifiersChanged(egui::Modifiers::NONE)],
    );
}

fn tab_to_warp_control(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str, modifiers: egui::Modifiers) -> egui::Id {
    // The popup container is also a Tab stop: its rectangle contains every option.
    // Require the real widget's focus announcement as well as its visible label.
    for _ in 0..32 {
        let mut events = frame_output(
            app,
            ctx,
            vec![Event::ModifiersChanged(modifiers), Event::Key { key: egui::Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers }],
        )
        .platform_output
        .events;
        events.extend(
            frame_output(
                app,
                ctx,
                vec![
                    Event::Key { key: egui::Key::Tab, physical_key: None, pressed: false, repeat: false, modifiers },
                    Event::ModifiersChanged(egui::Modifiers::NONE),
                ],
            )
            .platform_output
            .events,
        );
        if !events.iter().any(|event| {
            let egui::output::OutputEvent::FocusGained(info) = event else { return false };
            info.enabled
                && match info.role {
                    egui::Role::ComboBox => info.current_text_value.as_deref() == Some(label),
                    egui::Role::Button => info.label.as_deref() == Some(label),
                    _ => false,
                }
        }) {
            continue;
        }
        let texts = frame(app, ctx, vec![]);
        let at = texts.iter().find(|(text, _)| text == label).unwrap_or_else(|| panic!("missing keyboard target {label}")).1.center();
        if let Some(id) = ctx.memory(|m| m.focused())
            && ctx.read_response(id).is_some_and(|response| response.rect.contains(at) && ctx.layer_id_at(at) == Some(response.layer_id))
        {
            return id;
        }
    }
    panic!("Tab traversal did not focus {label}");
}

#[test]
fn warp_options_style_keyboard_enter_does_not_accept_dialog() {
    for preview in [true, false] {
        let (mut app, id) = app();
        app.run("object.move", json!({"dx": 10, "dy": 0})).unwrap();
        let moved = app.session.doc().unwrap().doc.clone();
        app.run("edit.undo", json!({})).unwrap();
        app.session.doc_mut().unwrap().mark_saved();
        let before = app.session.doc().unwrap().doc.clone();
        let selection = app.session.doc().unwrap().selection.clone();
        let entries = |items: &[vectorcraft_engine::HistoryEntry]| {
            items.iter().map(|entry| (entry.label.clone(), entry.doc.clone(), entry.selection.clone())).collect::<Vec<_>>()
        };
        let undo = entries(&app.session.doc().unwrap().history.undo);
        let redo = entries(&app.session.doc().unwrap().history.redo);
        assert_eq!(redo.len(), 1);
        let last_effect = app.last_effect.clone();
        app.run("effect.dialog", json!({"effect": "warp.arc", "item": null})).unwrap();
        let ctx = context();
        settle(&mut app, &ctx);
        if !preview {
            click_text(&mut app, &ctx, "Preview");
        }
        assert_eq!(app.ui.dialog.as_ref().unwrap().bool("preview"), preview);
        let style_id = tab_to_warp_control(&mut app, &ctx, "Arc", egui::Modifiers::SHIFT);
        // Both directions are actual key events, including returning to the Style combo.
        warp_key(&mut app, &ctx, egui::Key::Tab, egui::Modifiers::NONE);
        assert_ne!(ctx.memory(|m| m.focused()), Some(style_id));
        warp_key(&mut app, &ctx, egui::Key::Tab, egui::Modifiers::SHIFT);
        assert_eq!(ctx.memory(|m| m.focused()), Some(style_id));
        assert!(!egui::ComboBox::is_open(&ctx, style_id));
        warp_key(&mut app, &ctx, egui::Key::Enter, egui::Modifiers::NONE);
        assert!(app.ui.dialog.is_some(), "WARP_STYLE_ENTER_OPEN: opening Style must not accept Warp Options");
        assert!(egui::ComboBox::is_open(&ctx, style_id));
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("__effect"), "warp.arc");
        assert_eq!(entries(&app.session.doc().unwrap().history.undo), undo);
        assert_eq!(entries(&app.session.doc().unwrap().history.redo), redo);
        assert_eq!(app.last_effect, last_effect);

        tab_to_warp_control(&mut app, &ctx, "Arc Lower", egui::Modifiers::NONE);
        warp_key(&mut app, &ctx, egui::Key::Enter, egui::Modifiers::NONE);
        assert!(app.ui.dialog.is_some(), "WARP_STYLE_ENTER_CHOICE: choosing Style must not accept Warp Options");
        assert!(!egui::ComboBox::is_open(&ctx, style_id));
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("__effect"), "warp.arcLower");
        assert!(app.ui.dialog.as_ref().unwrap().bool("__warp_style_changed"));
        let state = app.session.doc().unwrap();
        assert_eq!(state.selection, selection);
        assert_eq!(entries(&state.history.undo), undo);
        assert_eq!(entries(&state.history.redo), redo);
        assert_eq!(app.last_effect, last_effect);
        if preview {
            assert_eq!(state.doc.node(id).unwrap().appearance.effects[0].id, "warp.arcLower");
            assert!(app.session.in_interaction());
        } else {
            assert_eq!(state.doc, before);
            assert!(!state.is_dirty() && !app.session.in_interaction());
        }
        click_text(&mut app, &ctx, "Cancel");
        let state = app.session.doc().unwrap();
        assert_eq!(state.doc, before);
        assert_eq!(state.selection, selection);
        assert_eq!(entries(&state.history.undo), undo);
        assert_eq!(entries(&state.history.redo), redo);
        assert!(!state.is_dirty());
        assert!(app.ui.dialog.is_none() && !app.session.in_interaction());
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc, moved);
    }
}
