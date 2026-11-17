//! Actual secondary-button events on the panel's grid cells and list rows.

use super::*;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

fn harness(view: View) -> Harness<'static, VectorcraftApp> {
    let mut first = true;
    let builder = Harness::builder().with_size(vec2(300.0, 700.0)).with_accessibility_check(false);
    // Optional local visual QA, following the Image Trace harness; captures stay untracked.
    let builder = if std::env::var_os("VECTORCRAFT_SWATCHES_CAPTURE_DIR").is_some() { builder.wgpu() } else { builder };
    let mut h = builder.build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            if std::mem::replace(&mut first, false) {
                egui_extras::install_image_loaders(ui.ctx());
                crate::theme::install_fonts(ui.ctx());
                app.ui.swatches_view = view;
                return;
            }
            show(app, ui);
        },
        app(),
    );
    h.run_steps(4);
    h
}

fn right_click(h: &mut Harness<'static, VectorcraftApp>, name: &str) {
    let pos = h.ctx.read_response(egui::Id::unique((TILE, name))).expect("swatch response").rect.center();
    h.event(Event::PointerMoved(pos));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(2);
}

fn cancel_then_confirm_delete(h: &mut Harness<'static, VectorcraftApp>) {
    let before = h.state().session.doc().unwrap().doc.clone();
    let undo = h.state().session.doc().unwrap().history.undo.len();
    let dialog = h.state().ui.dialog.as_ref().expect("delete asks first");
    assert_eq!(dialog.kind, crate::dialogs::confirm::KIND);
    assert_eq!(dialog.str("message"), "Delete “Red”?");
    assert_eq!(dialog.str("detail"), "Art using a deleted global swatch keeps its colour.");
    assert_eq!(dialog.str("__command"), "swatch.delete");
    assert_eq!(dialog.fields.get("__params"), Some(&json!({"names": ["Red"]})));
    assert_eq!(h.state().session.doc().unwrap().doc, before);
    assert_eq!(h.state().session.doc().unwrap().history.undo.len(), undo);
    crate::dialogs::cancel(h.state_mut());
    assert!(h.state().ui.dialog.is_none());
    assert_eq!(h.state().session.doc().unwrap().doc, before);
    assert_eq!(h.state().session.doc().unwrap().history.undo.len(), undo);
    right_click(h, "Red");
    h.get_by_label("   Delete Swatch").click();
    h.run_steps(3);
    crate::dialogs::confirm(h.state_mut()).unwrap();
}

#[test]
fn right_click_selects_without_painting_and_dispatches_item_commands() {
    for (view, _) in View::ALL {
        for command in ["Swatch Options…", "Duplicate Swatch", "Delete Swatch"] {
            let mut h = harness(view);
            set_pstate(&h.ctx, "swatch-selected", vec!["Black".to_string()]);
            let before = h.state().session.doc().unwrap().doc.clone();
            let paint = h.state().session.paint.fill.clone();
            let undo = h.state().session.doc().unwrap().history.undo.len();
            right_click(&mut h, "Red");
            assert_eq!(selected(&h.ctx), ["Red"], "{view:?}");
            assert_eq!(h.state().session.doc().unwrap().doc, before, "opening the menu leaves art unchanged");
            assert_eq!(h.state().session.paint.fill, paint, "selecting a context target does not apply its paint");
            assert_eq!(h.state().session.doc().unwrap().history.undo.len(), undo);
            for label in ["   Swatch Options…", "   Duplicate Swatch", "   Delete Swatch"] {
                assert!(!h.get_by_label(label).accesskit_node().is_disabled(), "{view:?}: {label}");
            }
            if command == "Swatch Options…"
                && let Some(dir) = std::env::var_os("VECTORCRAFT_SWATCHES_CAPTURE_DIR")
            {
                let dir = std::path::PathBuf::from(dir);
                std::fs::create_dir_all(&dir).unwrap();
                h.render().unwrap().save(dir.join(format!("{view:?}.png"))).unwrap();
            }
            h.get_by_label(&format!("   {command}")).click();
            h.run_steps(3);
            match command {
                "Swatch Options…" => {
                    let dialog = h.state().ui.dialog.as_ref().expect("Swatch Options opened");
                    assert_eq!(dialog.kind, KIND);
                    assert_eq!(dialog.str("__swatch"), "Red");
                    assert_eq!(h.state().session.doc().unwrap().history.undo.len(), undo);
                }
                "Duplicate Swatch" => {
                    let doc = &h.state().session.doc().unwrap().doc;
                    assert_eq!(doc.swatch("Red copy").unwrap().paint, doc.swatch("Red").unwrap().paint);
                    assert!(doc.swatch("Black copy").is_none());
                    assert_eq!(h.state().session.doc().unwrap().history.undo.len(), undo + 1);
                    h.state_mut().run("edit.undo", json!({})).unwrap();
                    assert_eq!(h.state().session.doc().unwrap().doc, before);
                }
                "Delete Swatch" => {
                    assert_eq!(h.state().session.doc().unwrap().doc, before);
                    assert_eq!(h.state().session.doc().unwrap().history.undo.len(), undo);
                    cancel_then_confirm_delete(&mut h);
                    let doc = &h.state().session.doc().unwrap().doc;
                    assert!(doc.swatch("Red").is_none());
                    assert!(doc.swatch("Black").is_some());
                    assert_eq!(h.state().session.doc().unwrap().history.undo.len(), undo + 1);
                    h.state_mut().run("edit.undo", json!({})).unwrap();
                    assert_eq!(h.state().session.doc().unwrap().doc, before);
                }
                other => panic!("unexpected menu action: {other}"),
            }
        }
    }
}

#[test]
fn reserved_swatches_have_disabled_item_actions() {
    for (view, _) in View::ALL {
        for name in ["[None]", REGISTRATION] {
            let mut h = harness(view);
            let before = h.state().session.doc().unwrap().doc.clone();
            let undo = h.state().session.doc().unwrap().history.undo.len();
            right_click(&mut h, name);
            assert_eq!(selected(&h.ctx), [name]);
            for label in ["   Swatch Options…", "   Duplicate Swatch", "   Delete Swatch"] {
                assert!(h.get_by_label(label).accesskit_node().is_disabled(), "{view:?}: {name}: {label}");
            }
            // egui closes a context menu even when the clicked item is disabled.
            h.get_by_label("   Delete Swatch").click();
            h.run_steps(2);
            assert!(h.state().ui.dialog.is_none());
            assert_eq!(h.state().session.doc().unwrap().doc, before);
            assert_eq!(h.state().session.doc().unwrap().history.undo.len(), undo);
        }
    }
}

#[test]
fn right_clicking_the_active_panel_item_preserves_its_selection() {
    for (view, _) in View::ALL {
        let mut h = harness(view);
        let names = vec!["Black".to_string(), "Red".to_string()];
        set_pstate(&h.ctx, "swatch-selected", names.clone());
        right_click(&mut h, "Red");
        assert_eq!(selected(&h.ctx), names);
        h.get_by_label("   Delete Swatch").click();
        h.run_steps(3);
        assert_eq!(h.state().ui.dialog.as_ref().unwrap().fields.get("__params"), Some(&json!({"names": ["Red"]})));
        crate::dialogs::confirm(h.state_mut()).unwrap();
        let doc = &h.state().session.doc().unwrap().doc;
        assert!(doc.swatch("Black").is_some(), "only the clicked item is deleted");
        assert!(doc.swatch("Red").is_none());
    }
}

#[test]
fn a_swatch_deleted_while_its_menu_is_open_disables_actions() {
    let mut first = true;
    // Keep the original response alive to exercise the stale-entry guard after external deletion.
    let mut h = Harness::builder().with_size(vec2(300.0, 300.0)).with_accessibility_check(false).build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            if std::mem::replace(&mut first, false) {
                crate::theme::install_fonts(ui.ctx());
                return;
            }
            let e = Entry::Swatch { name: "Red".into(), paint: Paint::solid(Color::BLACK), global: false, spot: false };
            let resp = ui.button("Target");
            item_context_menu(app, ui, &resp, &e);
        },
        app(),
    );
    h.run_steps(4);
    h.get_by_label("Target").click_secondary();
    h.run_steps(3);
    assert!(!h.get_by_label("   Duplicate Swatch").accesskit_node().is_disabled());
    h.state_mut().run("swatch.delete", json!({"name": "Red"})).unwrap();
    let before = h.state().session.doc().unwrap().doc.clone();
    let undo = h.state().session.doc().unwrap().history.undo.len();
    h.run_steps(3);
    for label in ["   Swatch Options…", "   Duplicate Swatch", "   Delete Swatch"] {
        assert!(h.get_by_label(label).accesskit_node().is_disabled(), "{label}");
    }
    h.get_by_label("   Duplicate Swatch").click();
    h.run_steps(2);
    assert!(h.state().ui.dialog.is_none());
    assert_eq!(h.state().session.doc().unwrap().doc, before);
    assert_eq!(h.state().session.doc().unwrap().history.undo.len(), undo);
}
