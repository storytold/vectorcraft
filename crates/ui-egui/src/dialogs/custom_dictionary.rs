//! Edit › Edit Custom Dictionary…: the custom dictionary's words in a list, with an Entry field and
//! Add, Delete and Change, as the public
//! [Check and correct spelling](https://helpx.adobe.com/illustrator/desktop/design-with-text/edit-format-text/check-and-correct-spelling.html)
//! page describes it: enter a word and Add it, select a word and Delete it, or select a word, edit it
//! in the Entry field and Change it. Nothing changes until Done, which deletes and adds the words
//! that differ (`text.spelling.removeWord`, `text.spelling.addWord`); Cancel leaves the dictionary as
//! it was.
//!
//! Fields: `orig` (the words when the dialog opened), `words` (as edited, sorted), `entry` (the
//! Entry field) and `selected` (the word picked, or null).

use serde_json::{Value, json};
use vectorcraft_engine::cmd::spelling::custom_word;

use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Edit Custom Dictionary.
pub const KIND: &str = "editCustomDictionary";

/// The command its menu item runs: with no params, it opens this dialog.
pub const COMMAND: &str = "text.spelling.dictionary";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Edit Custom Dictionary").into(), body, confirm, ok: Some("Done"), min_width: 420.0, ..DialogSpec::FORM };

/// Open Edit Custom Dictionary on the custom dictionary's words.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let words = app.run(COMMAND, json!({})).map_err(|e| e.to_string())?.get("words").cloned().unwrap_or_else(|| json!([]));
    app.ui.dialog = Some(Dialog::new(KIND, json!({ "orig": words, "words": words, "entry": "", "selected": null })));
    Ok(Value::Null)
}

fn list(d: &Dialog, key: &str) -> Vec<String> {
    d.fields.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default()
}

/// The word picked in the list, while it is still there.
fn selected(d: &Dialog, words: &[String]) -> Option<String> {
    d.fields.get("selected").and_then(Value::as_str).filter(|w| words.iter().any(|x| x == w)).map(String::from)
}

/// What the buttons can do now: the Entry field as a dictionary word that isn't in the list yet
/// (for Add and Change), and the word picked (for Delete and Change).
fn actions(d: &Dialog) -> (Option<String>, Option<String>) {
    let words = list(d, "words");
    let entry = custom_word(&d.str("entry")).filter(|w| !words.contains(w));
    (entry, selected(d, &words))
}

fn set_words(d: &mut Dialog, mut words: Vec<String>, selected: Option<&str>) {
    words.sort();
    words.dedup();
    d.fields.insert("words".into(), json!(words));
    d.fields.insert("selected".into(), json!(selected));
}

/// Add: the Entry field's word joins the list, picked.
fn add(d: &mut Dialog) {
    let (Some(w), _) = actions(d) else { return };
    let mut words = list(d, "words");
    words.push(w.clone());
    set_words(d, words, Some(&w));
}

/// Delete: the word picked leaves the list; the one after it (or before, for the last) is picked.
fn delete(d: &mut Dialog) {
    let (_, Some(w)) = actions(d) else { return };
    let mut words = list(d, "words");
    let Some(i) = words.iter().position(|x| *x == w) else { return };
    words.remove(i);
    let next = words.get(i).or_else(|| words.last()).cloned();
    if let Some(n) = &next {
        d.fields.insert("entry".into(), json!(n));
    }
    set_words(d, words, next.as_deref());
}

/// Change: the word picked becomes the Entry field's word.
fn change(d: &mut Dialog) {
    let (Some(to), Some(w)) = actions(d) else { return };
    let mut words: Vec<String> = list(d, "words").into_iter().filter(|x| *x != w).collect();
    words.push(to.clone());
    set_words(d, words, Some(&to));
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let words = list(d, "words");
    let current = selected(d, &words);
    let (entry, picked) = actions(d);
    let mut pick = None;
    let (mut added, mut deleted, mut changed) = (false, false, false);
    widgets::dim_label(ui, tl!("Entry:"));
    form::text(ui, d, "entry", 380.0);
    ui.add_space(8.0);
    ui.horizontal_top(|ui| {
        widgets::list_box(ui, |ui| {
            ui.set_width(260.0);
            let row = ui.spacing().interact_size.y;
            egui::ScrollArea::vertical().id_salt("custom-dictionary").min_scrolled_height(200.0).max_height(200.0).show_rows(
                ui,
                row,
                words.len(),
                |ui, range| {
                    ui.set_width(ui.available_width());
                    for w in words.get(range).unwrap_or_default() {
                        if ui.selectable_label(current.as_ref() == Some(w), w.as_str()).clicked() {
                            pick = Some(w.clone());
                        }
                    }
                },
            );
        });
        ui.add_space(16.0);
        ui.vertical(|ui| {
            added = ui.add_enabled_ui(entry.is_some(), |ui| widgets::flat_button(ui, tl!("Add"), 90.0).clicked()).inner;
            ui.add_space(6.0);
            deleted = ui.add_enabled_ui(picked.is_some(), |ui| widgets::flat_button(ui, tl!("Delete"), 90.0).clicked()).inner;
            ui.add_space(6.0);
            changed = ui.add_enabled_ui(entry.is_some() && picked.is_some(), |ui| widgets::flat_button(ui, tl!("Change"), 90.0).clicked()).inner;
        });
    });
    // Picking a word puts it in the Entry field, to edit for Change.
    if let Some(w) = pick {
        d.fields.insert("entry".into(), json!(w));
        d.fields.insert("selected".into(), json!(w));
    }
    if added {
        add(d);
    } else if deleted {
        delete(d);
    } else if changed {
        change(d);
    }
    false
}

/// Done: deletes the words no longer in the list, then adds the new ones. A word that can't be
/// saved keeps the dialog open with the error, its list as edited.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let (orig, words) = (list(d, "orig"), list(d, "words"));
    // The dictionary as it is now: another window may have deleted a word already.
    let now = app.run(COMMAND, json!({})).map_err(|e| e.to_string())?.get("words").cloned().unwrap_or_default();
    let kept = |w: &String| now.as_array().is_some_and(|a| a.contains(&json!(w)));
    for w in orig.iter().filter(|w| !words.contains(w) && kept(w)) {
        app.run("text.spelling.removeWord", json!({ "word": w })).map_err(|e| e.to_string())?;
    }
    for w in words.iter().filter(|w| !orig.contains(w)) {
        app.run("text.spelling.addWord", json!({ "word": w })).map_err(|e| e.to_string())?;
    }
    app.ui.dialog = None;
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use egui::{Event, PointerButton, Pos2, Rect, vec2};
    use serde_json::{Value, json};

    use crate::VectorcraftApp;

    // The custom dictionary is the whole process's: each test here uses words no other test does.
    const A: &str = "zqvdicta";
    const B: &str = "zqvdictb";
    const M: &str = "zqvdictm";
    const X: &str = "zqvdictx";
    const Y: &str = "zqvdicty";

    fn app() -> VectorcraftApp {
        VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default())
    }

    fn has(app: &mut VectorcraftApp, w: &str) -> bool {
        app.run("text.spelling.dictionary", json!({})).unwrap()["words"].as_array().unwrap().contains(&json!(w))
    }

    fn forget(app: &mut VectorcraftApp, words: &[&str]) {
        for w in words {
            let _ = app.run("text.spelling.removeWord", json!({ "word": w }));
        }
    }

    fn set(app: &mut VectorcraftApp, k: &str, v: Value) {
        app.ui.dialog.as_mut().unwrap().fields.insert(k.into(), v);
    }

    /// The dialog's list, of `mine` (other tests may have their words in the dictionary meanwhile).
    fn words(app: &VectorcraftApp, mine: &[&str]) -> Vec<String> {
        super::list(app.ui.dialog.as_ref().unwrap(), "words").into_iter().filter(|w| mine.contains(&w.as_str())).collect()
    }

    /// One frame of the dialog on an 800 × 600 window → the text painted, with its rect.
    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) -> Vec<(String, Rect)> {
        // A window takes a sizing frame and fades in: each frame is a quarter second on.
        let time = ctx.input(|i| i.time) + 0.25;
        let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)));
        let raw = egui::RawInput { screen_rect, events, time: Some(time), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| crate::dialogs::show(app, ui.ctx()));
        out.textures_delta.clear();
        fn walk(s: &egui::Shape, v: &mut Vec<(String, Rect)>) {
            match s {
                egui::Shape::Text(t) => v.push((t.galley.text().to_string(), Rect::from_min_size(t.pos, t.galley.size()))),
                egui::Shape::Vec(s) => s.iter().for_each(|s| walk(s, v)),
                _ => {}
            }
        }
        let mut v = vec![];
        out.shapes.iter().for_each(|c| walk(&c.shape, &mut v));
        v
    }

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        ctx
    }

    fn click(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str) -> Vec<(String, Rect)> {
        frame(app, ctx, vec![]);
        let texts = frame(app, ctx, vec![]);
        let at = texts.iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}` in {texts:?}"));
        let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(app, ctx, vec![Event::PointerMoved(at)]);
        frame(app, ctx, vec![press(true)]);
        frame(app, ctx, vec![press(false)]);
        frame(app, ctx, vec![])
    }

    /// Edit › Edit Custom Dictionary… (beside Spelling, as the public page has it) opens the
    /// dialog on the custom dictionary's words.
    #[test]
    fn the_edit_menu_opens_it_on_the_custom_dictionary() {
        let mut app = app();
        forget(&mut app, &[M]);
        app.run("text.spelling.addWord", json!({ "word": M })).unwrap();
        let e = crate::menus::menu_entries(&app).into_iter().find(|e| e.label == "Edit Custom Dictionary…").expect("in the Edit menu");
        assert_eq!((e.path.as_slice(), e.command.as_deref(), e.enabled), (&["Edit".to_string()][..], Some(super::COMMAND), true));
        crate::menus::invoke(&mut app, super::COMMAND, json!({}));
        let d = app.ui.dialog.as_ref().expect("Edit Custom Dictionary opens");
        assert_eq!(d.kind, super::KIND);
        assert_eq!(words(&app, &[M]), [M]);
        forget(&mut app, &[M]);
    }

    /// Add, Delete and Change edit the list; nothing changes in the dictionary until Done, which
    /// applies them; Cancel leaves it as it was.
    #[test]
    fn add_delete_and_change_apply_on_done() {
        let mut app = app();
        forget(&mut app, &[A, B, "zqvdictc"]);
        app.run("text.spelling.addWord", json!({ "word": A })).unwrap();
        super::open(&mut app).unwrap();
        let ctx = context();
        // Add: the Entry field's word (kept in lower case) joins the list.
        set(&mut app, "entry", json!(" ZqvDictB "));
        click(&mut app, &ctx, "Add");
        assert_eq!(words(&app, &[A, B, "zqvdictc"]), [A, B]);
        assert!(!has(&mut app, B), "not until Done");
        // Change: pick A, edit it in the Entry field.
        set(&mut app, "selected", json!(A));
        set(&mut app, "entry", json!("zqvdictc"));
        click(&mut app, &ctx, "Change");
        assert_eq!(words(&app, &[A, B, "zqvdictc"]), [B, "zqvdictc"]);
        // Delete the word picked.
        set(&mut app, "selected", json!(B));
        click(&mut app, &ctx, "Delete");
        assert_eq!(words(&app, &[A, B, "zqvdictc"]), ["zqvdictc"]);
        // Cancel: the dictionary is as it was.
        crate::dialogs::cancel(&mut app);
        assert!(has(&mut app, A) && !has(&mut app, B) && !has(&mut app, "zqvdictc"));
        // Again, then Done.
        super::open(&mut app).unwrap();
        set(&mut app, "selected", json!(A));
        set(&mut app, "entry", json!("zqvdictc"));
        click(&mut app, &ctx, "Change");
        set(&mut app, "entry", json!(B));
        click(&mut app, &ctx, "Add");
        click(&mut app, &ctx, "Done");
        assert!(app.ui.dialog.is_none(), "Done closes it");
        assert!(!has(&mut app, A) && has(&mut app, B) && has(&mut app, "zqvdictc"));
        forget(&mut app, &[A, B, "zqvdictc"]);
    }

    /// Add and Change need one word that isn't in the list yet; Delete and Change need a word
    /// picked. A button that can't act does nothing.
    #[test]
    fn the_buttons_act_only_on_a_word_they_can_take() {
        let mut app = app();
        forget(&mut app, &[X, Y]);
        app.run("text.spelling.addWord", json!({ "word": X })).unwrap();
        super::open(&mut app).unwrap();
        let ctx = context();
        for entry in ["", "two words", X, &"z".repeat(65)] {
            set(&mut app, "entry", json!(entry));
            assert_eq!(super::actions(app.ui.dialog.as_ref().unwrap()).0, None, "{entry:?}");
            click(&mut app, &ctx, "Add");
            assert_eq!(words(&app, &[X, Y]), [X], "{entry:?} isn't added");
        }
        // Nothing picked: Delete and Change do nothing.
        set(&mut app, "entry", json!(Y));
        click(&mut app, &ctx, "Delete");
        click(&mut app, &ctx, "Change");
        assert_eq!(words(&app, &[X, Y]), [X]);
        crate::dialogs::cancel(&mut app);
        forget(&mut app, &[X]);
    }
}
