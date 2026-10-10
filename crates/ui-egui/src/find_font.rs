//! Type → Find Font…: the fonts the document uses (missing ones marked), Find (select the text
//! using a font), Change / Change All to another installed font, and Find in Folder… (the missing
//! fonts' files, in the Missing Fonts dialog: `ui.findFontsInFolder`).

use egui::Ui;
use serde_json::{Value, json};

use crate::state::Dialog;
use crate::theme::Tokens;
use crate::widgets;
use crate::{VectorcraftApp, font_menu};

/// Open the dialog.
pub fn open(app: &mut VectorcraftApp) {
    open_at(app, 0);
}

/// Open the dialog on the first font the document misses, to replace it (Missing Fonts' Find
/// Fonts).
pub fn open_on_missing(app: &mut VectorcraftApp) {
    let first = fonts(app).iter().position(|f| matches!(f["status"].as_str(), Some("missing" | "substitute")));
    open_at(app, first.unwrap_or(0));
}

/// Open the dialog with the document's font `selected` (an index into `text.fonts`) chosen.
fn open_at(app: &mut VectorcraftApp, selected: usize) {
    app.ui.dialog = Some(Dialog::new("findFont", json!({ "selected": selected, "family": "Source Sans 3", "style": "", "selectionOnly": false })));
}

fn fonts(app: &mut VectorcraftApp) -> Vec<Value> {
    app.session.execute("text.fonts", &json!({})).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default()
}

pub fn confirm(app: &mut VectorcraftApp) -> Result<Value, String> {
    app.ui.dialog = None;
    Ok(Value::Null)
}

/// Replace the selected document font with the chosen one (`all`: everywhere, else the selection).
fn change(app: &mut VectorcraftApp, d: &Dialog, from: &Value, all: bool) -> Result<Value, String> {
    let style = d.str("style");
    let to = if style.is_empty() { json!({ "family": d.str("family") }) } else { json!({ "family": d.str("family"), "style": style }) };
    app.run("text.replaceFont", json!({ "from": { "family": from["family"], "style": from["style"] }, "to": to, "selectionOnly": !all }))
}

pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(mut d) = app.ui.dialog.clone() else { return };
    let t = Tokens::get(ctx);
    let list = fonts(app);
    let sample = font_menu::sample_text(app);
    // Find in Folder… shows when the document misses fonts, fonts used only in its symbols included.
    let offer_folder = crate::picks::can(app, &crate::picks::PickRequest::Folder)
        && app.session.active().is_some_and(|st| !vectorcraft_engine::cmd::fontfiles::missing_fonts(&st.doc).is_empty());
    let mut close = false;
    let mut find_in_folder = false;
    let mut act: Option<&str> = None;
    crate::dialogs::modal::show(ctx, tl!("Find Font"), egui::Id::new("dialog-find-font"), -40.0, 22, |ui: &mut Ui| {
        ui.set_width(380.0);
        crate::dialogs::modal::heading(ui, tl!("Find Font"));
        ui.add_space(10.0);
        widgets::subheader(ui, &crate::i18n::fmt(tl!("Fonts in Document: {count}"), &[("count", &list.len().to_string())]));
        let sel = d.fields.get("selected").and_then(Value::as_u64).unwrap_or(0) as usize;
        egui::Frame::NONE.fill(t.input).stroke(egui::Stroke::new(1.0, t.input_border)).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
            ui.set_min_height(120.0);
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                for (i, f) in list.iter().enumerate() {
                    let (note, color) = font_note(f);
                    let label = format!("{} {}{note}  ({})", f["family"].as_str().unwrap_or(""), f["style"].as_str().unwrap_or(""), f["runs"]);
                    let text = egui::RichText::new(label).color(color.unwrap_or(t.text));
                    if ui.selectable_label(i == sel, text).clicked() {
                        d.fields.insert("selected".into(), json!(i));
                    }
                }
            });
        });
        // The missing fonts' files, looked for in a folder (the Missing Fonts dialog).
        if offer_folder {
            ui.add_space(6.0);
            find_in_folder = ui.button(tl!("Find in Folder…")).clicked();
        }
        ui.add_space(10.0);
        widgets::subheader(ui, tl!("Replace With Font"));
        let fam = d.str("family");
        ui.horizontal(|ui| {
            // Picks the font only: nothing is previewed on the document.
            let pick = font_menu::font_menu(ui, "ff-family", &fam, 220.0, sample.as_deref(), font_menu::MenuLook::of(app));
            if let Some((f, style)) = font_menu::picked(app, pick) {
                d.fields.insert("family".into(), json!(f));
                d.fields.insert("style".into(), json!(style.unwrap_or_default()));
            }
            let styles = vectorcraft_text::FontDb::global().styles(&d.str("family"));
            // "(closest)" is ours; the font's style names are shown as they are.
            let closest = tl!("(closest)");
            let mut opts: Vec<&str> = vec![closest];
            opts.extend(styles.iter().map(String::as_str));
            let style = d.str("style");
            let cur = if style.is_empty() { closest } else { style.as_str() };
            if let Some(i) = widgets::dropdown_names(ui, "ff-style", cur, &opts, 120.0) {
                d.fields.insert("style".into(), json!(if i == 0 { String::new() } else { styles[i - 1].clone() }));
            }
        });
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            let has = sel < list.len();
            if ui.add_enabled(has, egui::Button::new(tl!("Find"))).clicked() {
                act = Some("find");
            }
            if ui.add_enabled(has, egui::Button::new(tl!("Change"))).on_hover_text(tl!("In the selected objects")).clicked() {
                act = Some("change");
            }
            if ui.add_enabled(has, egui::Button::new(tl!("Change All"))).clicked() {
                act = Some("changeAll");
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::primary_button(ui, tl!("Done")).clicked() {
                    close = true;
                }
            });
        });
    });
    let sel = d.fields.get("selected").and_then(Value::as_u64).unwrap_or(0) as usize;
    if let (Some(a), Some(from)) = (act, list.get(sel)) {
        let r = match a {
            "find" => app.run("select.font", json!({ "family": from["family"], "style": from["style"] })),
            "change" => change(app, &d, from, false),
            _ => change(app, &d, from, true),
        };
        if let Err(e) = r {
            app.ui.status = e;
        }
    }
    if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.ui.dialog = None;
    } else if app.ui.dialog.is_some() {
        app.ui.dialog = Some(d);
    }
    // Missing Fonts takes the dialog's place, searching the folder picked.
    if find_in_folder && let Err(e) = app.run("ui.findFontsInFolder", json!({})) {
        app.ui.status = e;
    }
}

/// What a Find Font row says after the font's name (`text.fonts` row): a missing family, a style
/// standing in for another, characters the font lacks; and the colour to flag it with.
pub(crate) fn font_note(f: &Value) -> (String, Option<egui::Color32>) {
    let lacking = f["missingGlyphs"].as_u64().unwrap_or(0);
    let glyphs =
        if lacking > 0 { crate::i18n::fmt(tl!("  — {n} characters from another font"), &[("n", &lacking.to_string())]) } else { String::new() };
    match f["status"].as_str() {
        Some("missing") => (format!("{}{glyphs}", tl!("  — missing")), Some(egui::Color32::from_rgb(230, 90, 90))),
        Some("substitute") => {
            let used = f["resolved"]["style"].as_str().unwrap_or("");
            (
                format!("{}{glyphs}", crate::i18n::fmt(tl!("  — substituted by {style}"), &[("style", used)])),
                Some(egui::Color32::from_rgb(220, 160, 60)),
            )
        }
        _ if lacking > 0 => (glyphs, Some(egui::Color32::from_rgb(220, 160, 60))),
        _ => (String::new(), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialog_draws_and_changes_all() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Hi", "font": "Missing Family"})).unwrap();
        open(&mut app);
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut app, ui.ctx()));
        out.textures_delta.clear();
        let d = app.ui.dialog.clone().unwrap();
        let from = fonts(&mut app)[0].clone();
        assert_eq!(from["missing"], true);
        change(&mut app, &d, &from, true).unwrap();
        assert_eq!(fonts(&mut app)[0]["family"], "Source Sans 3");
    }

    #[test]
    fn find_in_folder_puts_the_missing_fonts_dialog_in_find_fonts_place() {
        let dir = std::fs::canonicalize(vectorcraft_testkit::temp_dir("find-font-folder")).unwrap();
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.search_rules = Some(vectorcraft_engine::cmd::findfiles::Rules { user_content: vec![dir.clone()], ..Default::default() });
        app.session.search_threads = Some(1);
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        assert!(!crate::menus::enabled(&app, "ui.findFontsInFolder"), "no folder picker (the web)");
        let picked = dir.to_string_lossy().into_owned();
        app.services.pick_folder = Some(Box::new(move || Some(picked.clone())));
        assert!(crate::menus::enabled(&app, "ui.findFontsInFolder"));
        // Nothing missing: no button, and no folder is asked for.
        open(&mut app);
        let text = crate::tests_labels::painted_text(&mut app, |app, ui| show(app, ui.ctx()));
        assert!(text.contains("Fonts in Document") && !text.contains("Find in Folder…"), "{text}");
        assert_eq!(app.run("ui.findFontsInFolder", json!({})).err().as_deref(), Some("No fonts are missing in this document"));
        app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Hi", "font": "Missing Family"})).unwrap();
        let text = crate::tests_labels::painted_text(&mut app, |app, ui| show(app, ui.ctx()));
        assert!(text.contains("Find in Folder…"), "{text}");
        app.run("ui.findFontsInFolder", json!({})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.kind.as_str(), d.str("folder")), (crate::dialogs::missing_fonts::KIND, dir.to_string_lossy().into_owned()));
        assert_eq!(d.fields["fonts"][0]["family"], "Missing Family");
        assert!(app.ui.dialog_search.is_some());
        crate::dialogs::cancel(&mut app);
    }

    #[test]
    fn rows_flag_missing_fonts_substituted_styles_and_lacking_characters() {
        let row = |status: &str, glyphs: u64| json!({"status": status, "resolved": {"style": "W4"}, "missingGlyphs": glyphs});
        assert_eq!(font_note(&row("exact", 0)), (String::new(), None));
        assert!(font_note(&row("missing", 0)).0.contains("missing"));
        assert!(font_note(&row("substitute", 0)).0.contains("substituted by W4"));
        let (note, color) = font_note(&row("exact", 2));
        assert!(note.contains("2 characters") && color.is_some());
    }
}
