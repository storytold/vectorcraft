//! Type → Find Font…: the fonts the document uses (missing ones marked), Find (select the text
//! using a font), Change / Change All to another font from the list Replace With Font From shows
//! (the document's fonts, the recent fonts or every installed font), and Find in Folder… (the
//! missing fonts' files, in the Missing Fonts dialog: `ui.findFontsInFolder`).

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

/// Open the dialog with the document's font `selected` (an index into `text.fonts`) chosen, and
/// System's font ([`system_family`]) to change to.
fn open_at(app: &mut VectorcraftApp, selected: usize) {
    app.ui.dialog = Some(Dialog::new(
        "findFont",
        json!({ "selected": selected, "family": system_family(), "style": "", "selectionOnly": false, "replaceFrom": "system" }),
    ));
}

/// The font System chooses: Source Sans 3 when it is installed, otherwise the font menu's first
/// family (none when the menu is empty).
fn system_family() -> String {
    system_family_in(installed, &vectorcraft_text::FontDb::global().menu_family_list())
}

/// [`system_family`] for the installed check `is_installed` and the font menu `menu`.
fn system_family_in(is_installed: impl Fn(&str) -> bool, menu: &[String]) -> String {
    const PREFERRED: &str = "Source Sans 3";
    if is_installed(PREFERRED) { PREFERRED.to_string() } else { menu.first().cloned().unwrap_or_default() }
}

/// Replace With Font From's lists (the `replaceFrom` field): the document's fonts, the recent
/// fonts and every installed font.
const SOURCES: [&str; 3] = ["document", "recent", "system"];
const SOURCE_LABELS: [&str; 3] = ["Document", "Recent", "System"];

/// The families Replace With Font From lists. `None` is System: the font menu, every installed
/// family. Document lists the families `text.fonts` names, once each, in its order; Recent lists
/// the font menu's recent families (newest first, Number of Recent Fonts at most) that are
/// installed.
fn replacement_families(app: &VectorcraftApp, source: &str, used: &[Value]) -> Option<Vec<String>> {
    let names: Vec<String> = match source {
        "document" => used.iter().filter_map(|f| f["family"].as_str()).map(str::to_string).collect(),
        "recent" => {
            let installed = vectorcraft_text::FontDb::global().menu_family_list();
            app.recent_fonts().iter().filter_map(|r| installed.iter().find(|f| f.eq_ignore_ascii_case(r)).cloned()).collect()
        }
        _ => return None,
    };
    let mut out: Vec<String> = vec![];
    for n in names {
        if !out.iter().any(|o| o.eq_ignore_ascii_case(&n)) {
            out.push(n);
        }
    }
    Some(out)
}

/// Is `family` installed: it resolves to a face of its own, as `text.replaceFont` requires.
fn installed(family: &str) -> bool {
    !family.is_empty()
        && vectorcraft_text::FontDb::global().resolve(family, "Regular").is_some_and(|(_, m)| m != vectorcraft_text::FontMatch::Missing)
}

/// Can Change and Change All replace with the dialog's font: a family is chosen, it is installed,
/// and it is one of the list Replace With Font From shows (System's list is the font menu).
fn can_replace(d: &Dialog, families: Option<&[String]>) -> bool {
    let family = d.str("family");
    match families {
        Some(list) => can_replace_in(&family, installed, list),
        None => can_replace_in(&family, installed, &vectorcraft_text::FontDb::global().menu_family_list()),
    }
}

/// [`can_replace`] for the installed check `is_installed` and the shown list `list`.
fn can_replace_in(family: &str, is_installed: impl Fn(&str) -> bool, list: &[String]) -> bool {
    is_installed(family) && list.iter().any(|f| f.eq_ignore_ascii_case(family))
}

/// Show list `source` ([`SOURCES`]): the chosen family stays when the list has it and it is
/// installed, otherwise the list's first installed family (none when there is none) is chosen in
/// its closest style. System chooses [`system_family`].
fn set_source(app: &VectorcraftApp, d: &mut Dialog, source: &str, used: &[Value]) {
    d.fields.insert("replaceFrom".into(), json!(source));
    let families = replacement_families(app, source, used);
    if can_replace(d, families.as_deref()) {
        return;
    }
    let family = match families {
        Some(l) => l.into_iter().find(|f| installed(f)),
        None => Some(system_family()),
    };
    d.fields.insert("family".into(), json!(family.unwrap_or_default()));
    d.fields.insert("style".into(), json!(""));
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
    crate::dialogs::modal::show(ctx, tl!("Find Font"), egui::Id::unique("dialog-find-font"), -40.0, 22, |ui: &mut Ui| {
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
        let source = d.str("replaceFrom");
        ui.horizontal(|ui| {
            widgets::field_label(ui, tl!("Replace With Font From"));
            let at = SOURCES.iter().position(|s| *s == source).unwrap_or(2);
            if let Some(i) = widgets::dropdown_named(ui, "ff-from", Some(tl!("Replace With Font From")), SOURCE_LABELS[at], &SOURCE_LABELS, 120.0) {
                set_source(app, &mut d, SOURCES[i], &list);
            }
        });
        let families = replacement_families(app, &d.str("replaceFrom"), &list);
        let fam = d.str("family");
        ui.horizontal(|ui| {
            if let Some(families) = &families {
                // Font names are shown as they are.
                let names: Vec<&str> = families.iter().map(String::as_str).collect();
                if let Some(i) = widgets::dropdown_names_named(ui, "ff-family-list", Some(tl!("Replace With Font")), &fam, &names, 220.0) {
                    d.fields.insert("family".into(), json!(families[i]));
                    d.fields.insert("style".into(), json!(""));
                }
            } else {
                // Picks the font only: nothing is previewed on the document.
                let pick = font_menu::font_menu(ui, "ff-family", &fam, 220.0, sample.as_deref(), font_menu::MenuLook::of(app));
                if let Some((f, style)) = font_menu::picked(app, pick) {
                    d.fields.insert("family".into(), json!(f));
                    d.fields.insert("style".into(), json!(style.unwrap_or_default()));
                }
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
        // An empty list (Recent before any font was applied) leaves nothing to change to.
        let replace = sel < list.len() && can_replace(&d, families.as_deref());
        ui.horizontal(|ui| {
            let has = sel < list.len();
            if ui.add_enabled(has, egui::Button::new(tl!("Find"))).clicked() {
                act = Some("find");
            }
            if ui.add_enabled(replace, egui::Button::new(tl!("Change"))).on_hover_text(tl!("In the selected objects")).clicked() {
                act = Some("change");
            }
            if ui.add_enabled(replace, egui::Button::new(tl!("Change All"))).clicked() {
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

    /// Change All leaves locked text in its font, and the status bar says so.
    #[test]
    fn change_all_says_when_locked_text_keeps_its_font() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Hi", "font": "Missing Family"})).unwrap();
        app.session.execute("object.lock", &json!({})).unwrap();
        open(&mut app);
        let d = app.ui.dialog.clone().unwrap();
        let from = fonts(&mut app)[0].clone();
        let r = change(&mut app, &d, &from, true).unwrap();
        assert_eq!(r["runs"], 0, "{r}");
        assert_eq!(fonts(&mut app)[0]["family"], "Missing Family");
        assert_eq!(app.ui.status, "some text is hidden or locked, so its font wasn't replaced");
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

    /// A document set in Inter and Source Serif 4 (Inter twice), with Inter the one recent font.
    fn two_font_document() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        for (y, font) in [(20, "Inter"), (40, "Source Serif 4"), (60, "Inter")] {
            app.session.execute("text.create", &json!({"x": 10, "y": y, "text": "Hi", "font": font})).unwrap();
        }
        app.ui.recent_fonts = vec!["Inter".into()];
        app
    }

    #[test]
    fn replace_with_font_from_lists_the_document_s_recent_or_installed_fonts() {
        let mut app = two_font_document();
        let used = fonts(&mut app);
        let mut document = replacement_families(&app, "document", &used).unwrap();
        document.sort();
        assert_eq!(document, ["Inter", "Source Serif 4"]);
        assert_eq!(replacement_families(&app, "recent", &used).unwrap(), ["Inter"]);
        assert_eq!(replacement_families(&app, "system", &used), None, "System is the font menu of every installed family");
        open(&mut app);
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("replaceFrom"), "system");
        for source in SOURCES {
            let mut d = app.ui.dialog.clone().unwrap();
            set_source(&app, &mut d, source, &used);
            let listed = replacement_families(&app, source, &used);
            assert!(can_replace(&d, listed.as_deref()), "{source}: {}", d.str("family"));
            app.ui.dialog = Some(d);
            let text = crate::tests_labels::painted_text(&mut app, |app, ui| show(app, ui.ctx()));
            assert!(text.contains("Replace With Font From"), "{text}");
        }
    }

    #[test]
    fn change_all_replaces_with_a_font_from_the_document() {
        let mut app = two_font_document();
        open(&mut app);
        let used = fonts(&mut app);
        let mut d = app.ui.dialog.clone().unwrap();
        set_source(&app, &mut d, "document", &used);
        d.fields.insert("family".into(), json!("Inter"));
        let from = used.iter().find(|f| f["family"] == "Source Serif 4").unwrap().clone();
        change(&mut app, &d, &from, true).unwrap();
        let families: Vec<Value> = fonts(&mut app).iter().map(|f| f["family"].clone()).collect();
        assert_eq!(families, [json!("Inter")]);
    }

    /// A font applied with Change All becomes Recent's first family.
    #[test]
    fn change_all_adds_the_new_font_to_recent() {
        let mut app = two_font_document();
        open(&mut app);
        let used = fonts(&mut app);
        let mut d = app.ui.dialog.clone().unwrap();
        d.fields.insert("family".into(), json!("Source Serif 4"));
        let from = used.iter().find(|f| f["family"] == "Inter").unwrap().clone();
        change(&mut app, &d, &from, true).unwrap();
        let used = fonts(&mut app);
        assert_eq!(replacement_families(&app, "recent", &used).unwrap(), ["Source Serif 4", "Inter"]);
    }

    /// A Change All that changes no text (the font's only text is locked) leaves Recent as it was.
    #[test]
    fn a_change_all_that_changes_nothing_leaves_recent_unchanged() {
        let mut app = two_font_document();
        app.run("select.font", json!({"family": "Source Serif 4"})).unwrap();
        app.run("object.lock", json!({})).unwrap();
        open(&mut app);
        let used = fonts(&mut app);
        let mut d = app.ui.dialog.clone().unwrap();
        d.fields.insert("family".into(), json!("Source Sans 3"));
        let from = used.iter().find(|f| f["family"] == "Source Serif 4").unwrap().clone();
        assert_eq!(change(&mut app, &d, &from, true).unwrap()["runs"], 0);
        assert_eq!(app.ui.recent_fonts, ["Inter"]);
    }

    /// Change to another name of an installed family records the family's own name in Recent.
    #[test]
    fn change_records_the_family_s_own_name_in_recent() {
        let mut app = two_font_document();
        open(&mut app);
        let used = fonts(&mut app);
        let mut d = app.ui.dialog.clone().unwrap();
        d.fields.insert("family".into(), json!("SourceSerif4-Regular"));
        let from = used.iter().find(|f| f["family"] == "Inter").unwrap().clone();
        assert_eq!(change(&mut app, &d, &from, true).unwrap()["runs"], 2);
        assert_eq!(app.ui.recent_fonts, ["Source Serif 4", "Inter"]);
    }

    #[test]
    fn an_empty_recent_list_leaves_nothing_to_change_to() {
        let mut app = two_font_document();
        app.ui.recent_fonts.clear();
        open(&mut app);
        let used = fonts(&mut app);
        let mut d = app.ui.dialog.clone().unwrap();
        set_source(&app, &mut d, "recent", &used);
        assert_eq!(replacement_families(&app, "recent", &used).unwrap(), Vec::<String>::new());
        assert_eq!(d.str("family"), "");
        assert!(!can_replace(&d, Some(&[])));
        app.ui.dialog = Some(d);
        let text = crate::tests_labels::painted_text(&mut app, |app, ui| show(app, ui.ctx()));
        assert!(text.contains("Recent"), "{text}");
        assert_eq!(app.ui.status, "");
    }

    /// Document lists a missing family, but Change and Change All stay off for it, in every list.
    #[test]
    fn a_missing_document_font_is_listed_but_not_offered_to_change_to() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        for (y, font) in [(20, "Missing Family"), (40, "Inter")] {
            app.session.execute("text.create", &json!({"x": 10, "y": y, "text": "Hi", "font": font})).unwrap();
        }
        open(&mut app);
        let used = fonts(&mut app);
        let document = replacement_families(&app, "document", &used).unwrap();
        assert!(document.iter().any(|f| f == "Missing Family"), "{document:?}");
        let mut d = app.ui.dialog.clone().unwrap();
        d.fields.insert("family".into(), json!("Missing Family"));
        set_source(&app, &mut d, "document", &used);
        assert_eq!(d.str("family"), "Inter", "the first installed family is chosen");
        assert!(can_replace(&d, Some(&document)));
        d.fields.insert("family".into(), json!("Missing Family"));
        assert!(!can_replace(&d, Some(&document)));
        assert!(!can_replace(&d, None), "System");
        set_source(&app, &mut d, "system", &used);
        assert!(can_replace(&d, None), "System chooses an installed family: {}", d.str("family"));
        // A document set only in a missing family leaves nothing to change to.
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 20, "text": "Hi", "font": "Missing Family"})).unwrap();
        let used = fonts(&mut app);
        set_source(&app, &mut d, "document", &used);
        assert_eq!(d.str("family"), "");
        assert!(!can_replace(&d, replacement_families(&app, "document", &used).as_deref()));
    }

    /// An empty Recent list clears the font; System then chooses an installed one again.
    #[test]
    fn system_after_an_empty_recent_list_chooses_an_installed_font() {
        let mut app = two_font_document();
        app.ui.recent_fonts.clear();
        open(&mut app);
        let used = fonts(&mut app);
        let mut d = app.ui.dialog.clone().unwrap();
        set_source(&app, &mut d, "recent", &used);
        assert_eq!(d.str("family"), "");
        set_source(&app, &mut d, "system", &used);
        assert_eq!((d.str("family").as_str(), d.str("style").as_str()), ("Source Sans 3", ""));
        assert!(can_replace(&d, None));
    }

    /// The dialog opens on System's font: Source Sans 3 when it is installed, otherwise the font
    /// menu's first family.
    #[test]
    fn the_dialog_opens_on_an_installed_font() {
        let menu = ["Inter".to_string(), "Source Serif 4".to_string()];
        assert_eq!(system_family_in(|_| true, &menu), "Source Sans 3");
        assert_eq!(system_family_in(|f| f != "Source Sans 3", &menu), "Inter");
        assert_eq!(system_family_in(|_| false, &[]), "");
        let mut app = two_font_document();
        open(&mut app);
        let d = app.ui.dialog.clone().unwrap();
        assert_eq!(d.str("family"), system_family());
        assert!(can_replace(&d, None), "{}", d.str("family"));
    }

    /// A family that is installed but left out of the font menu (a hidden `.` family that
    /// Document lists) cannot be changed to from System.
    #[test]
    fn system_offers_only_the_font_menu_s_families() {
        let menu = ["Inter".to_string()];
        assert!(can_replace_in("inter", |_| true, &menu));
        assert!(!can_replace_in(".Hidden", |_| true, &menu));
        assert!(!can_replace_in("Inter", |_| false, &menu));
        let hidden = vectorcraft_text::FontDb::global().family_list().iter().find(|f| f.starts_with('.') && installed(f)).cloned();
        if let Some(hidden) = hidden {
            let mut app = two_font_document();
            open(&mut app);
            let used = fonts(&mut app);
            let mut d = app.ui.dialog.clone().unwrap();
            d.fields.insert("family".into(), json!(hidden));
            assert!(!can_replace(&d, None), "{hidden}");
            set_source(&app, &mut d, "system", &used);
            assert_eq!(d.str("family"), system_family());
        }
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
