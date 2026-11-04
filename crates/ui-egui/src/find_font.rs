//! Type → Find Font…: the fonts the document uses (missing ones marked), Find (select the text
//! using a font), Change / Change All to another font from the list Replace With Font From shows
//! (the document's fonts, the recent fonts or every installed font), and Find in Folder… (the
//! missing fonts' files, in the Missing Fonts dialog: `ui.findFontsInFolder`).

use egui::Ui;
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::widgets;

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
    let used = fonts(app);
    let source = "system";
    let rows = replacement_rows(app, source, &used);
    let family = system_family();
    let style = rows.iter().find(|r| r.family.eq_ignore_ascii_case(&family)).map(|r| r.style.clone()).unwrap_or_default();
    app.ui.dialog = Some(Dialog::new(
        "findFont",
        json!({ "selected": selected, "family": family, "style": style, "selectionOnly": false, "replaceFrom": source, "rows": rows_json(&rows) }),
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

/// A row of Replace With Font: a family and one of its styles (`""` is the family's closest style),
/// so one click picks both.
#[derive(Clone, Debug, PartialEq)]
struct Row {
    family: String,
    style: String,
}

/// What the row says: the family and its style, or the family alone for its closest style.
fn row_label(family: &str, style: &str) -> String {
    if style.is_empty() { family.to_string() } else { format!("{family} {style}") }
}

/// Does a row match the search `query` (lower case): its family and style, as typed.
fn row_matches(family: &str, style: &str, query: &str) -> bool {
    query.is_empty() || format!("{family} {style}").to_lowercase().contains(query)
}

/// Replace With Font's rows for `source` ([`SOURCES`]). Document lists the family and style pairs
/// `text.fonts` names, once each, in its order; Recent and System list their families with each
/// family's closest style and then its styles.
fn replacement_rows(app: &VectorcraftApp, source: &str, used: &[Value]) -> Vec<Row> {
    let db = vectorcraft_text::FontDb::global();
    let mut rows: Vec<Row> = vec![];
    match source {
        "document" => {
            for f in used {
                let (family, style) = (f["family"].as_str().unwrap_or_default(), f["style"].as_str().unwrap_or_default());
                if rows.iter().any(|r| r.family.eq_ignore_ascii_case(family) && r.style.eq_ignore_ascii_case(style)) {
                    continue;
                }
                rows.push(Row { family: family.to_string(), style: style.to_string() });
            }
        }
        "recent" => {
            for family in replacement_families(app, "recent", used).unwrap_or_default() {
                push_family(&mut rows, db, &family);
            }
        }
        _ => {
            for family in db.menu_family_list().iter() {
                push_family(&mut rows, db, family);
            }
        }
    }
    rows
}

/// Add `family` to `rows`: each style once, the closest style first, every row naming its style.
fn push_family(rows: &mut Vec<Row>, db: &vectorcraft_text::FontDb, family: &str) {
    if family.is_empty() {
        return;
    }
    let mut styles = db.styles(family);
    // The style a style-less request picks comes first, by its own name, not again without one. Its
    // name comes from the catalog's metadata: the rows of every family would otherwise have to read
    // the family's font files.
    if let Some(closest) = db.closest_style(family)
        && let Some(i) = styles.iter().position(|s| s.eq_ignore_ascii_case(&closest))
    {
        let style = styles.remove(i);
        styles.insert(0, style);
    }
    if styles.is_empty() {
        rows.push(Row { family: family.to_string(), style: String::new() });
        return;
    }
    for style in styles {
        rows.push(Row { family: family.to_string(), style });
    }
}

/// `rows` as the dialog stores them (`{"family": …, "style": …}`).
fn rows_json(rows: &[Row]) -> Value {
    Value::Array(rows.iter().map(|r| json!({ "family": r.family, "style": r.style })).collect())
}

/// Can the replacement name that very face: `family` and `style` both exist, no stand-in. A row of
/// an unavailable style stays listed, but only a font with that style is chosen as the replacement.
fn exact(family: &str, style: &str) -> bool {
    !family.is_empty() && vectorcraft_text::FontDb::global().resolve(family, style).is_some_and(|(_, m)| m == vectorcraft_text::FontMatch::Exact)
}

/// Can a row be chosen as the replacement: its family is installed and the style is the font's own
/// (or the family names no styles, and its closest style stands for them).
fn listed(row: &Row) -> bool {
    installed(&row.family) && (row.style.is_empty() || exact(&row.family, &row.style))
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
    let style = d.str("style");
    if !style.is_empty() && !exact(&family, &style) {
        return false;
    }
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
    let rows = replacement_rows(app, source, used);
    d.fields.insert("rows".into(), rows_json(&rows));
    // The chosen font stays only when the new list has that family and style; otherwise the family
    // and the style both come from a row the list shows, so Change replaces with what is listed.
    let (family, style) = (d.str("family"), d.str("style"));
    if rows.iter().any(|r| r.family.eq_ignore_ascii_case(&family) && r.style.eq_ignore_ascii_case(&style) && listed(r)) {
        return;
    }
    let system = if source == "system" { system_family() } else { String::new() };
    let pick =
        rows.iter().find(|r| !system.is_empty() && r.family.eq_ignore_ascii_case(&system) && listed(r)).or_else(|| rows.iter().find(|r| listed(r)));
    d.fields.insert("family".into(), json!(pick.map(|r| r.family.clone()).unwrap_or_default()));
    d.fields.insert("style".into(), json!(pick.map(|r| r.style.clone()).unwrap_or_default()));
}

fn fonts(app: &mut VectorcraftApp) -> Vec<Value> {
    app.session.execute("text.fonts", &json!({})).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default()
}

pub fn confirm(app: &mut VectorcraftApp) -> Result<Value, String> {
    if let Some(ctx) = app.ui_ctx.clone() {
        clear_search(&ctx);
    }
    app.ui.dialog = None;
    Ok(Value::Null)
}

/// Forget the replacement search: the next open starts with an empty one.
fn clear_search(ctx: &egui::Context) {
    let (source_key, query_id) = (egui::Id::unique("ff-search-source"), egui::Id::unique("ff-search"));
    ctx.data_mut(|m| {
        m.remove::<String>(source_key);
        m.remove::<String>(query_id);
    });
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
    // Find in Folder… shows when the document misses fonts, fonts used only in its symbols included.
    let offer_folder = crate::picks::can(app, &crate::picks::PickRequest::Folder)
        && app.session.active().is_some_and(|st| !vectorcraft_engine::cmd::fontfiles::missing_fonts(&st.doc).is_empty());
    let mut close = false;
    let mut find_in_folder = false;
    let mut act: Option<&str> = None;
    // The replacement-font search, kept in egui memory: cleared when the dialog closes, and when
    // the source changes.
    let source_key = egui::Id::unique("ff-search-source");
    let query_id = egui::Id::unique("ff-search");
    crate::dialogs::modal::show(ctx, tl!("Find Font"), egui::Id::unique("dialog-find-font"), -40.0, 22, |ui: &mut Ui| {
        ui.set_width(380.0);
        crate::dialogs::modal::heading(ui, tl!("Find Font"));
        ui.add_space(10.0);
        widgets::subheader(ui, &crate::i18n::fmt(tl!("Fonts in Document: {count}"), &[("count", &list.len().to_string())]));
        let sel = d.fields.get("selected").and_then(Value::as_u64).unwrap_or(0) as usize;
        egui::Frame::NONE.fill(t.input).stroke(egui::Stroke::new(1.0, t.input_border)).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
            ui.set_min_height(120.0);
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical().id_salt("ff-document-list").max_height(160.0).auto_shrink([false, true]).show(ui, |ui| {
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
        // The rows Replace With Font lists for the source: one click picks the family and its style.
        let rows: Vec<(String, String)> = d
            .fields
            .get("rows")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|r| (r["family"].as_str().unwrap_or_default().to_string(), r["style"].as_str().unwrap_or_default().to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let (fam, style) = (d.str("family"), d.str("style"));
        // A search like the Character panel's font menu: the family and style are filtered as typed.
        // A new source starts with an empty search.
        let source = d.str("replaceFrom");
        if ui.data(|m| m.get_temp::<String>(source_key).as_deref() != Some(source.as_str())) {
            ui.data_mut(|m| {
                m.insert_temp(source_key, source.clone());
                m.insert_temp(query_id, String::new());
            });
        }
        let query = widgets::search_field(ui, query_id, tl!("Search")).to_lowercase();
        let shown: Vec<&(String, String)> = rows.iter().filter(|(f, s)| row_matches(f, s, &query)).collect();
        let mut picked: Option<(String, String, bool)> = None;
        // Ten rows at a time, whichever height the rows get from the style.
        let row = (ui.spacing().interact_size.y.max(ui.text_style_height(&egui::TextStyle::Body)) + ui.spacing().item_spacing.y).max(18.0);
        egui::Frame::NONE.fill(t.input).stroke(egui::Stroke::new(1.0, t.input_border)).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let area = egui::ScrollArea::vertical()
                .id_salt("ff-rows-list")
                // Ten rows at most, and no taller than the rows there are: a search of three fonts
                // leaves a list three rows tall.
                .max_height((shown.len() as f32 * row).clamp(row, 10.0 * row))
                .auto_shrink([false, true])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible);
            area.show_rows(ui, row, shown.len(), |ui, range| {
                for i in range {
                    let Some((f, s)) = shown.get(i) else { continue };
                    let is_selected = f.eq_ignore_ascii_case(&fam) && s.eq_ignore_ascii_case(&style);
                    let resp = ui.selectable_label(is_selected, row_label(f, s));
                    if resp.clicked() {
                        picked = Some((f.clone(), s.clone(), resp.double_clicked()));
                    }
                }
            });
        });
        if let Some((f, s, twice)) = picked {
            d.fields.insert("family".into(), json!(f));
            d.fields.insert("style".into(), json!(s));
            if twice && can_replace(&d, families.as_deref()) {
                act = Some("change");
            }
        }
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
        match r {
            // The rows are a snapshot: the document's fonts and the recent fonts have moved on.
            Ok(_) if a != "find" => {
                let used = fonts(app);
                d.fields.insert("rows".into(), rows_json(&replacement_rows(app, &d.str("replaceFrom"), &used)));
            }
            Ok(_) => {}
            Err(e) => app.ui.status = e,
        }
    }
    if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.ui.dialog = None;
        // The next open starts with an empty search.
        clear_search(ctx);
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
    fn change_all_clears_the_skip_warning_after_unlocking_text() {
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
        app.run("object.unlockAll", json!({})).unwrap();
        assert_eq!(app.ui.status, "some text is hidden or locked, so its font wasn't replaced");
        let r = change(&mut app, &d, &from, true).unwrap();
        assert_eq!(r["runs"], 1, "{r}");
        assert!(r.get("warning").is_none(), "{r}");
        assert_eq!(app.ui.status, "");
        app.ui.status = "Saved another document".into();
        let r = app.run("text.replaceFont", json!({"from": {"family": "Missing Family"}, "to": {"family": "Source Sans 3"}})).unwrap();
        assert_eq!(r["runs"], 0, "{r}");
        assert_eq!(app.ui.status, "Saved another document");
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

    /// One headless frame of the dialog: every text it painted, with where.
    fn frame(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>) -> Vec<(String, egui::Rect)> {
        fn collect(s: &egui::epaint::Shape, out: &mut Vec<(String, egui::Rect)>) {
            match s {
                egui::epaint::Shape::Text(t) => out.push((t.galley.text().to_string(), egui::Rect::from_min_size(t.pos, t.galley.size()))),
                egui::epaint::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
                _ => {}
            }
        }
        let raw = egui::RawInput { events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui.ctx()));
        let mut texts = vec![];
        for c in &out.shapes {
            collect(&c.shape, &mut texts);
        }
        out.textures_delta.clear();
        texts
    }

    /// The texts of a [`frame`] that are in `labels`, in order.
    fn painted(frame: &[(String, egui::Rect)], labels: &[String]) -> Vec<String> {
        frame.iter().map(|(t, _)| t.clone()).filter(|t| labels.contains(t)).collect()
    }

    /// A click at `at`: the press on one frame, the release on the next.
    fn click(ctx: &egui::Context, app: &mut VectorcraftApp, at: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(ctx, app, vec![egui::Event::PointerMoved(at), button(true)]);
        frame(ctx, app, vec![button(false)]);
        frame(ctx, app, vec![]);
    }

    /// A document set in `families`, one text each.
    fn many_font_document(families: &[&str]) -> VectorcraftApp {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        for (i, family) in families.iter().enumerate() {
            app.session.execute("text.create", &json!({"x": 10, "y": 10.0 + i as f64 * 20.0, "text": "Hi", "font": family})).unwrap();
        }
        app
    }

    /// The catalog's closest style is the face a style-less request resolves to, and finding it reads
    /// no font file.
    #[test]
    fn the_closest_style_is_the_face_a_style_less_request_gets() {
        let db = vectorcraft_text::FontDb::global();
        for family in ["Source Sans 3", "Inter"] {
            let closest = db.closest_style(family).unwrap_or_else(|| panic!("{family} is installed"));
            let face = db.face(family, "").unwrap_or_else(|| panic!("{family} resolves"));
            assert_eq!(closest, face.style, "{family}");
        }
    }

    /// A document font drawn in a substitute style stays listed, but it is not chosen as the
    /// replacement and Change stays off for it: only a style the font has can be the target.
    #[test]
    fn a_substituted_style_is_listed_but_not_chosen() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        // A style Source Sans 3 has none of: its closest style stands in.
        app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Hi", "font": "Source Sans 3", "style": "Black Wide"})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 70, "text": "Hi", "font": "Inter"})).unwrap();
        open(&mut app);
        let used = fonts(&mut app);
        assert!(used.iter().any(|f| f["family"] == "Source Sans 3" && f["style"] == "Black Wide" && f["status"] == "substitute"), "{used:?}");
        let rows = replacement_rows(&app, "document", &used);
        let stand_in = Row { family: "Source Sans 3".into(), style: "Black Wide".into() };
        assert!(rows.contains(&stand_in), "the row stays listed: {rows:?}");
        let mut d = app.ui.dialog.clone().unwrap();
        set_source(&app, &mut d, "document", &used);
        assert_eq!((d.str("family").as_str(), d.str("style").as_str()), ("Inter", "Regular"), "the first font whose style it has");
        d.fields.insert("family".into(), json!("Source Sans 3"));
        d.fields.insert("style".into(), json!("Black Wide"));
        assert!(!can_replace(&d, replacement_families(&app, "document", &used).as_deref()), "Change stays off for the stand-in style");
    }

    /// A new source picks the family and the style from a row its list shows, so Change replaces with
    /// what is listed rather than a closest style of its own choosing.
    #[test]
    fn a_new_source_picks_a_row_its_list_shows() {
        let mut app = two_font_document();
        let used = fonts(&mut app);
        let mut d = Dialog::new("findFont", json!({"family": "Inter", "style": "", "selected": 0, "replaceFrom": "system"}));
        set_source(&app, &mut d, "document", &used);
        assert_eq!((d.str("family").as_str(), d.str("style").as_str()), ("Inter", "Regular"), "the document's own row");
        d.fields.insert("family".into(), json!("Inter"));
        d.fields.insert("style".into(), json!("Black Wide"));
        set_source(&app, &mut d, "document", &used);
        assert_eq!((d.str("family").as_str(), d.str("style").as_str()), ("Inter", "Regular"), "a pair the list lacks gives way");
    }

    /// Confirming the dialog with Enter forgets the replacement search, as Done and Escape do.
    #[test]
    fn confirming_the_dialog_clears_the_replacement_search() {
        let mut app = two_font_document();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        app.ui_ctx = Some(ctx.clone());
        open(&mut app);
        let query_id = egui::Id::unique("ff-search");
        ctx.data_mut(|m| m.insert_temp(query_id, "times".to_string()));
        confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none(), "the dialog closes");
        assert_eq!(ctx.data(|m| m.get_temp::<String>(query_id)), None, "the search text is gone");
    }

    /// The rows follow a replacement: Change All in the Document source leaves the list showing the
    /// document's fonts as they are now.
    #[test]
    fn the_rows_refresh_after_a_change() {
        let mut app = two_font_document();
        open(&mut app);
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        frame(&ctx, &mut app, vec![]);
        let used = fonts(&mut app);
        let mut d = app.ui.dialog.clone().unwrap();
        set_source(&app, &mut d, "document", &used);
        d.fields.insert("family".into(), json!("Source Serif 4"));
        d.fields.insert("style".into(), json!(""));
        app.ui.dialog = Some(d);
        frame(&ctx, &mut app, vec![]);
        let texts = frame(&ctx, &mut app, vec![]);
        let change_all = texts.iter().find(|(t, _)| t == "Change All").map(|(_, r)| r.center()).expect("Change All is shown");
        click(&ctx, &mut app, change_all);
        assert!(fonts(&mut app).iter().all(|f| f["family"] == "Source Serif 4"), "the document changed");
        let rows = app.ui.dialog.as_ref().expect("the dialog stays open").fields["rows"].as_array().cloned().unwrap_or_default();
        let families: Vec<&str> = rows.iter().filter_map(|r| r["family"].as_str()).collect();
        assert!(
            !families.is_empty() && families.iter().all(|f| *f == "Source Serif 4"),
            "the rows list the document's fonts after the change: {families:?}"
        );
    }

    /// Closing the dialog clears the replacement search: the next open starts with an empty one.
    #[test]
    fn closing_the_dialog_clears_the_replacement_search() {
        let mut app = two_font_document();
        open(&mut app);
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        frame(&ctx, &mut app, vec![]);
        let query_id = egui::Id::unique("ff-search");
        let source_id = egui::Id::unique("ff-search-source");
        ctx.data_mut(|m| {
            m.insert_temp(query_id, "times".to_string());
            m.insert_temp(source_id, "system".to_string());
        });
        let escape = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        frame(&ctx, &mut app, vec![escape]);
        frame(&ctx, &mut app, vec![]);
        assert!(app.ui.dialog.is_none(), "Escape closes the dialog");
        assert_eq!(ctx.data(|m| m.get_temp::<String>(query_id)), None, "the search text is gone");
        assert_eq!(ctx.data(|m| m.get_temp::<String>(source_id)), None, "the source it was typed under is gone");
    }

    /// The wheel over the rows scrolls them, and leaves the document's font list where it was: two
    /// lists, two scrollbars.
    #[test]
    fn the_rows_list_scrolls_with_the_wheel_alone() {
        let families = ["Arial", "Georgia", "Verdana", "Tahoma", "Courier New", "Times New Roman", "Consolas", "Impact"];
        let mut app = many_font_document(&families);
        open(&mut app);
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        frame(&ctx, &mut app, vec![]);
        let texts = frame(&ctx, &mut app, vec![]);
        let used = fonts(&mut app);
        let doc_labels: Vec<String> = used
            .iter()
            .map(|f| {
                let (note, _) = font_note(f);
                format!("{} {}{note}  ({})", f["family"].as_str().unwrap_or(""), f["style"].as_str().unwrap_or(""), f["runs"])
            })
            .collect();
        let rows = replacement_rows(&app, "system", &used);
        let labels: Vec<String> = rows.iter().map(|r| row_label(&r.family, &r.style)).collect();
        let at = |texts: &[(String, egui::Rect)], text: &str| texts.iter().find(|(t, _)| t == text).map(|(_, r)| *r);
        let start = at(&texts, &labels[0]).expect("the rows start at their top");
        let (doc_before, rows_before) = (painted(&texts, &doc_labels), painted(&texts, &labels));
        assert!(!doc_before.is_empty(), "the document's fonts are listed: {doc_before:?}");
        assert!(!rows_before.is_empty(), "{rows_before:?}");
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, -20.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        };
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(start.center()), wheel]);
        for _ in 0..30 {
            frame(&ctx, &mut app, vec![]);
        }
        let texts = frame(&ctx, &mut app, vec![]);
        assert!(at(&texts, &labels[0]).is_none(), "the wheel scrolls the first rows out of view");
        assert_ne!(painted(&texts, &labels), rows_before, "the wheel scrolls the rows");
        assert_eq!(painted(&texts, &doc_labels), doc_before, "the document's font list stays where it was");
    }
    /// The search filters the rows by family and style, whatever case was typed.
    #[test]
    fn the_search_filters_rows_by_family_and_style() {
        assert!(row_matches("Arial", "Bold", ""), "no search lists every row");
        assert!(row_matches("Arial", "Bold", "arial"));
        assert!(row_matches("Arial", "Bold", "rial b"));
        assert!(row_matches("Source Sans 3", "Semibold", "sans 3 sem"));
        assert!(!row_matches("Arial", "Bold", "times"));
        assert!(!row_matches("Arial", "Bold", "bold italic"));
    }

    /// Replace With Font's rows: Document lists the family and style pairs the document uses, once
    /// each; Recent and System list each family with its closest style first and then its styles.
    #[test]
    fn replacement_rows_pick_a_family_and_its_style() {
        let mut app = two_font_document();
        let used = fonts(&mut app);
        let document = replacement_rows(&app, "document", &used);
        assert_eq!(
            document,
            [Row { family: "Inter".into(), style: "Regular".into() }, Row { family: "Source Serif 4".into(), style: "Regular".into() }],
            "one row per family and style the document uses"
        );
        let db = vectorcraft_text::FontDb::global();
        let closest = db.closest_style("Inter").expect("Inter is installed");
        let recent = replacement_rows(&app, "recent", &used);
        assert_eq!(recent.first(), Some(&Row { family: "Inter".into(), style: closest.clone() }), "the closest style comes first, by name");
        assert!(recent.iter().all(|r| r.family == "Inter"), "{recent:?}");
        assert_eq!(recent.len(), db.styles("Inter").len(), "each style once, the closest not repeated");
        assert!(recent.iter().all(|r| !r.style.is_empty()), "every row names its style: {recent:?}");
        let system = replacement_rows(&app, "system", &used);
        assert!(system.iter().any(|r| r.family == "Source Sans 3" && r.style == "Regular"), "{system:?}");
        assert!(!system.iter().any(|r| r.family.starts_with('.')), "the font menu's hidden families stay out");
        assert!(system.iter().all(|r| !r.style.is_empty()), "no nameless rows: {system:?}");
        let menu = db.menu_family_list();
        let listed: std::collections::BTreeSet<&str> = system.iter().map(|r| r.family.as_str()).collect();
        assert_eq!(listed.len(), menu.len(), "every family the font menu shows is listed");
        let mut unique = system.clone();
        unique.dedup();
        assert_eq!(unique.len(), system.len(), "no family and style is listed twice");
        // The dialog stores the rows when it opens and when the source changes, and a picked row
        // sets both fields Change then uses.
        open(&mut app);
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields["rows"].as_array().map(Vec::len), Some(system.len()));
        let mut d = app.ui.dialog.clone().unwrap();
        set_source(&app, &mut d, "document", &used);
        assert_eq!(d.fields["rows"].as_array().map(Vec::len), Some(document.len()));
        d.fields.insert("family".into(), json!("Inter"));
        d.fields.insert("style".into(), json!("Regular"));
        let from = used.iter().find(|f| f["family"] == "Source Serif 4").unwrap().clone();
        change(&mut app, &d, &from, true).unwrap();
        assert!(fonts(&mut app).iter().all(|f| f["family"] == "Inter"));
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
        assert_eq!((d.str("family").as_str(), d.str("style").as_str()), ("Source Sans 3", "Regular"), "the family's first listed style");
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
