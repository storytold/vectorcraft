//! Type › Composite Fonts… (with Show East Asian Options): the active document's composite fonts,
//! the one picked in Font shown as a table with a row for each class of character (Kanji, Kana,
//! Punctuation, Symbols, Roman, Numerals), then a row for each custom character set, with its font,
//! style, size, baseline and scales. New adds a font based on the one picked, Delete Font takes it
//! out, Import… adds the fonts of a file and Export… writes the one picked to a file. Custom shows
//! the custom character sets of the font picked: New adds a set based on the one picked, Delete Set
//! takes it out, and Add puts the characters typed (Direct Entry) or the character of a Unicode code
//! into it; Delete takes the character picked out. Show Sample draws a sample of the font picked
//! with lines that mark its metrics ([`sample`]). Nothing changes in the document until OK, which
//! runs `type.compositeFonts.set` with every font as one undo step; Cancel leaves them as they were.
//!
//! Fields: `fonts` (the composite fonts as `type.compositeFonts.set` takes them), `selected` (the
//! index of the one picked) and `name` (its name as typed); `custom` (whether the custom character
//! sets show), `set` (the index of the set picked), `setName` (its name as typed), `entryMode`
//! (`direct` or `unicode`), `entry` (the characters or code typed) and `char` (the index of the
//! character picked in the set); `sample` (whether the sample shows), `zoom` (its magnification in
//! percent) and `lineIcfBox`, `lineEmBox`, `lineBaseline`, `lineCapHeight`, `lineAscentDescent`,
//! `lineAscender` and `lineXHeight` (whether each of its lines shows).
//!
//! Behavior reference (public documentation):
//! <https://help.adobe.com/archive/en_US/illustrator/cs3/illustrator_cs3_help.pdf#page=347>,
//! custom character sets:
//! <https://web.archive.org/web/20220911143322/https://helpx.adobe.com/pdf/cs6/illustrator_reference.pdf#page=348>.

use serde_json::{Value, json};
use vectorcraft_doc::text::{COMPOSITE_BASELINE, COMPOSITE_SCALE, COMPOSITE_SIZE, CUSTOM_CHARS_MAX, CUSTOM_SETS_MAX};
use vectorcraft_doc::{CharClass, CompositeFont, CompositePart, CustomCharSet};
use vectorcraft_engine::cmd::compositefonts::EXTS;

use super::{DialogSpec, form};
use crate::picks::PickRequest;
use crate::state::Dialog;
use crate::{FilePick, VectorcraftApp, widgets};

mod sample;

/// The dialog kind of Composite Fonts.
pub const KIND: &str = "compositeFonts";

/// The command OK runs, and the menu item's.
pub const COMMAND: &str = "type.compositeFonts.set";

/// How many of a custom set's characters each row of its list shows.
const CHARS_PER_ROW: usize = 20;

/// The tallest the table of parts grows before it scrolls, in points: the header, the six classes
/// and a few custom sets.
const TABLE_HEIGHT: f32 = 340.0;

/// `char` is held only while a character is picked, so `ui.dialog.set` takes it from `inputs`.
pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Composite Fonts").into(),
    body,
    confirm,
    min_width: 760.0,
    inputs: |_| &["custom", "entry", "char"],
    ..DialogSpec::FORM
};

/// Open Composite Fonts on the active document's composite fonts, the first one picked, with the
/// custom character sets and the sample hidden and nothing typed.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let fonts = app.session.active().ok_or("no document open")?.doc.composite_fonts.to_vec();
    let mut d = Dialog::new(KIND, json!({ "fonts": fonts, "selected": 0, "custom": false, "entryMode": "direct", "entry": "" }));
    sample::defaults(&mut d.fields);
    show_selected(&mut d);
    app.ui.dialog = Some(d);
    Ok(Value::Null)
}

/// Why `ui.dialog.set` can't set `field` of Composite Fonts `d` to the value it now holds (Show
/// Sample's fields, [`sample::check_field`]).
pub(super) fn field_set(d: &Dialog, field: &str) -> Result<(), String> {
    sample::check_field(d, field)
}

/// The fonts the dialog holds.
fn fonts(d: &Dialog) -> Vec<CompositeFont> {
    d.fields.get("fonts").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default()
}

fn put_fonts(d: &mut Dialog, fonts: &[CompositeFont]) {
    d.fields.insert("fonts".into(), json!(fonts));
}

/// The index field `key` holds, within `count` items (0 when there are none).
fn index(d: &Dialog, key: &str, count: usize) -> usize {
    d.fields.get(key).and_then(Value::as_u64).map_or(0, |i| i as usize).min(count.saturating_sub(1))
}

/// The index of the font picked, within the fonts (0 when there are none).
fn selected(d: &Dialog, count: usize) -> usize {
    index(d, "selected", count)
}

/// Pick font `i`: its name goes into Name, and its first custom set is picked.
fn select(d: &mut Dialog, i: usize) {
    d.fields.insert("selected".into(), json!(i));
    show_selected(d);
}

fn show_selected(d: &mut Dialog) {
    let fonts = fonts(d);
    let name = fonts.get(selected(d, fonts.len())).map(|f| f.name.clone()).unwrap_or_default();
    d.fields.insert("name".into(), json!(name));
    select_set(d, 0);
}

/// Pick custom set `i` of the font picked: its name goes into the set's Name, and no character is
/// picked.
fn select_set(d: &mut Dialog, i: usize) {
    d.fields.insert("set".into(), json!(i));
    d.fields.remove("char");
    let fonts = fonts(d);
    let name = fonts.get(selected(d, fonts.len())).and_then(|f| f.custom.get(i)).map(|s| s.name.clone()).unwrap_or_default();
    d.fields.insert("setName".into(), json!(name));
}

/// The name of a class's row.
fn class_label(c: CharClass) -> &'static str {
    match c {
        CharClass::Kanji => tl!("Kanji"),
        CharClass::Kana => tl!("Kana"),
        CharClass::Punctuation => tl!("Punctuation"),
        CharClass::Symbols => tl!("Symbols"),
        CharClass::Roman => tl!("Roman"),
        CharClass::Numerals => tl!("Numerals"),
    }
}

/// "Composite Font N", the first N no font in `fonts` and no installed font is named.
fn new_name(fonts: &[CompositeFont]) -> String {
    let db = vectorcraft_text::FontDb::global();
    (1..)
        .map(|n| format!("Composite Font {n}"))
        .find(|n| !fonts.iter().any(|f| CompositeFont::same_name(&f.name, n)) && (db.composite_font(n).is_some() || !db.has_family(n)))
        .unwrap_or_default()
}

/// "Character Set N", the first N no set in `sets` is named.
fn new_set_name(sets: &[CustomCharSet]) -> String {
    (1..).map(|n| format!("Character Set {n}")).find(|n| !sets.iter().any(|s| CompositeFont::same_name(&s.name, n))).unwrap_or_default()
}

/// What a button asked for, done once the table is drawn.
enum Action {
    New,
    Delete,
    Import,
    Export,
    /// Custom: show or hide the custom character sets.
    Custom,
    /// Show Sample or Hide Sample.
    Sample,
    NewSet,
    DeleteSet,
    /// Add the characters typed, or the character of the code typed, to the set picked.
    AddChars,
    /// Take the character picked out of the set picked.
    DeleteChar,
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let mut list = fonts(d);
    let i = selected(d, list.len());
    let mut act = None;
    let names: Vec<&str> = list.iter().map(|f| f.name.as_str()).collect();
    let pick = FilePick { filters: vec![FilePick::filter("Composite fonts", EXTS)], ..Default::default() };
    let can_import = crate::picks::can(app, &PickRequest::Open(pick));
    let mut picked = None;
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Font:"));
        if names.is_empty() {
            widgets::dim_label(ui, tl!("None"));
        } else if let Some(k) = widgets::dropdown_names(ui, "composite-fonts-font", names.get(i).copied().unwrap_or_default(), &names, 220.0) {
            picked = Some(k);
        }
        ui.add_space(8.0);
        ui.spacing_mut().item_spacing.x = 4.0;
        if widgets::flat_button(ui, tl!("New"), 48.0).clicked() {
            act = Some(Action::New);
        }
        if ui.add_enabled_ui(!list.is_empty(), |ui| widgets::flat_button(ui, tl!("Delete Font"), 84.0)).inner.clicked() {
            act = Some(Action::Delete);
        }
        if can_import && widgets::flat_button(ui, tl!("Import…"), 64.0).clicked() {
            act = Some(Action::Import);
        }
        if ui.add_enabled_ui(!list.is_empty(), |ui| widgets::flat_button(ui, tl!("Export…"), 64.0)).inner.clicked() {
            act = Some(Action::Export);
        }
        if ui.add_enabled_ui(!list.is_empty(), |ui| widgets::flat_button(ui, tl!("Custom"), 64.0)).inner.clicked() {
            act = Some(Action::Custom);
        }
        let label = if d.bool("sample") { tl!("Hide Sample") } else { tl!("Show Sample") };
        if ui.add_enabled_ui(!list.is_empty(), |ui| widgets::flat_button(ui, label, 96.0)).inner.clicked() {
            act = Some(Action::Sample);
        }
    });
    if let Some(k) = picked {
        select(d, k);
        return false;
    }
    ui.add_space(8.0);
    if let Some(f) = list.get_mut(i) {
        ui.horizontal(|ui| {
            widgets::dim_label(ui, tl!("Name:"));
            form::text(ui, d, "name", 260.0);
        });
        f.name = d.str("name");
        ui.add_space(8.0);
        parts_table(app, ui, f);
        if d.bool("sample") {
            ui.add_space(8.0);
            sample::show(ui, d, f);
        }
        if d.bool("custom") {
            ui.add_space(8.0);
            ui.separator();
            if let Some(a) = custom_sets(ui, d, f) {
                act = Some(a);
            }
        }
    }
    put_fonts(d, &list);
    if let Some(a) = act {
        // Import…'s file dialog, shown off the UI thread, imports into the dialog as it is then.
        let r = match a {
            Action::Import => crate::picks::in_dialog(app, d, |app, d| run(app, d, Action::Import)),
            a => run(app, d, a),
        };
        if let Err(e) = r {
            app.status(e);
        }
    }
    false
}

/// The custom character sets of `f`: Character Set with New and Delete Set, the set's Name, the
/// Character row that adds characters, and the set's characters with Delete. The button pressed.
fn custom_sets(ui: &mut egui::Ui, d: &mut Dialog, f: &mut CompositeFont) -> Option<Action> {
    let mut act = None;
    let k = index(d, "set", f.custom.len());
    let names: Vec<&str> = f.custom.iter().map(|s| s.name.as_str()).collect();
    let mut picked = None;
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Character Set:"));
        if names.is_empty() {
            widgets::dim_label(ui, tl!("None"));
        } else if let Some(n) = widgets::dropdown_names(ui, "composite-fonts-set", names.get(k).copied().unwrap_or_default(), &names, 220.0) {
            picked = Some(n);
        }
        ui.add_space(8.0);
        ui.spacing_mut().item_spacing.x = 4.0;
        if ui.add_enabled_ui(f.custom.len() < CUSTOM_SETS_MAX, |ui| widgets::flat_button(ui, tl!("New"), 48.0)).inner.clicked() {
            act = Some(Action::NewSet);
        }
        if ui.add_enabled_ui(!f.custom.is_empty(), |ui| widgets::flat_button(ui, tl!("Delete Set"), 84.0)).inner.clicked() {
            act = Some(Action::DeleteSet);
        }
    });
    if let Some(n) = picked {
        select_set(d, n);
        return None;
    }
    let Some(set) = f.custom.get_mut(k) else { return act };
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Name:"));
        form::text(ui, d, "setName", 260.0);
    });
    set.name = d.str("setName");
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Character:"));
        let modes = [tl!("Direct Entry"), tl!("Unicode")];
        let unicode = d.str("entryMode") == "unicode";
        if let Some(m) = widgets::dropdown(ui, "composite-fonts-entry-mode", modes[usize::from(unicode)], &modes, 120.0) {
            d.fields.insert("entryMode".into(), json!(if m == 1 { "unicode" } else { "direct" }));
        }
        form::text(ui, d, "entry", 160.0);
        if widgets::flat_button(ui, tl!("Add"), 48.0).clicked() {
            act = Some(Action::AddChars);
        }
    });
    ui.add_space(4.0);
    let chosen = d.fields.get("char").and_then(Value::as_u64).map(|c| c as usize);
    let mut pick_char = None;
    // Only the rows in view are built, so a set of thousands of characters stays quick to draw.
    let chars: Vec<char> = set.characters.chars().collect();
    let rows = chars.len().div_ceil(CHARS_PER_ROW);
    egui::ScrollArea::vertical().id_salt("composite-fonts-chars").max_height(120.0).show_rows(ui, 24.0, rows, |ui, range| {
        for row in range {
            ui.horizontal(|ui| {
                for (n, c) in chars.iter().enumerate().skip(row * CHARS_PER_ROW).take(CHARS_PER_ROW) {
                    let r = ui.add(egui::Button::selectable(chosen == Some(n), c.to_string()).min_size(egui::vec2(24.0, 24.0)));
                    if r.on_hover_text(format!("U+{:04X}", *c as u32)).clicked() {
                        pick_char = Some(n);
                    }
                }
            });
        }
    });
    if let Some(n) = pick_char {
        d.fields.insert("char".into(), json!(n));
    }
    let can_delete = chosen.is_some_and(|n| n < chars.len());
    if ui.add_enabled_ui(can_delete, |ui| widgets::flat_button(ui, tl!("Delete"), 64.0)).inner.clicked() {
        act = Some(Action::DeleteChar);
    }
    act
}

/// Do what button `act` asks for on the dialog's fonts.
fn run(app: &mut VectorcraftApp, d: &mut Dialog, act: Action) -> Result<(), String> {
    let mut list = fonts(d);
    let i = selected(d, list.len());
    match act {
        Action::New => {
            let mut f = list.get(i).cloned().unwrap_or_else(|| {
                let st = vectorcraft_doc::CharStyle::default();
                CompositeFont::new("", st.font_family, st.font_style)
            });
            f.name = new_name(&list);
            list.push(f);
            put_fonts(d, &list);
            select(d, list.len() - 1);
        }
        Action::Delete => {
            if i < list.len() {
                list.remove(i);
            }
            put_fonts(d, &list);
            select(d, i.min(list.len().saturating_sub(1)));
        }
        Action::Import => import(app, d)?,
        Action::Export => {
            let f = list.get(i).ok_or("no composite font")?;
            crate::io::save_command_output(app, "type.compositeFonts.export", EXTS[0], json!({ "fonts": [f] }))?;
        }
        Action::Custom => {
            let open = !d.bool("custom");
            d.fields.insert("custom".into(), json!(open));
        }
        Action::Sample => {
            let open = !d.bool("sample");
            d.fields.insert("sample".into(), json!(open));
        }
        Action::NewSet | Action::DeleteSet | Action::AddChars | Action::DeleteChar => {
            let f = list.get_mut(i).ok_or("no composite font")?;
            let at = edit_sets(d, f, act)?;
            put_fonts(d, &list);
            if let Some(k) = at {
                select_set(d, k);
            }
        }
    }
    Ok(())
}

/// Do custom set action `act` on font `f`. The set to pick afterwards, when the sets changed.
fn edit_sets(d: &mut Dialog, f: &mut CompositeFont, act: Action) -> Result<Option<usize>, String> {
    let k = index(d, "set", f.custom.len());
    match act {
        Action::NewSet => {
            if f.custom.len() >= CUSTOM_SETS_MAX {
                return Err(format!("a composite font keeps at most {CUSTOM_SETS_MAX} character sets"));
            }
            // Based on the set picked: its characters, font and values.
            let mut s = f.custom.get(k).cloned().unwrap_or_else(|| CustomCharSet::new("", f.parts.kanji.clone()));
            s.name = new_set_name(&f.custom);
            f.custom.push(s);
            Ok(Some(f.custom.len() - 1))
        }
        Action::DeleteSet => {
            if k < f.custom.len() {
                f.custom.remove(k);
            }
            Ok(Some(k.min(f.custom.len().saturating_sub(1))))
        }
        Action::AddChars => {
            let set = f.custom.get_mut(k).ok_or("no character set")?;
            let entry = d.str("entry");
            let add: Vec<char> = if d.str("entryMode") == "unicode" {
                entry
                    .split(|c: char| c.is_whitespace() || c == ',')
                    .filter(|code| !code.is_empty())
                    .map(|code| CustomCharSet::char_from_code(code).ok_or_else(|| format!("`{code}` is not a code from U+0000 to U+FFFF")))
                    .collect::<Result<_, _>>()?
            } else {
                entry.chars().collect()
            };
            if let Some(c) = add.iter().find(|&&c| !CustomCharSet::takes(c)) {
                return Err(format!("`{}`: U+{:04X} is a control character or past U+FFFF", set.name, *c as u32));
            }
            let mut count = set.characters.chars().count();
            for c in add {
                if set.characters.contains(c) {
                    continue;
                }
                if count == CUSTOM_CHARS_MAX {
                    return Err(format!("`{}`: a character set keeps at most {CUSTOM_CHARS_MAX} characters", set.name));
                }
                set.characters.push(c);
                count += 1;
            }
            d.fields.insert("entry".into(), json!(""));
            Ok(None)
        }
        Action::DeleteChar => {
            let set = f.custom.get_mut(k).ok_or("no character set")?;
            let n = d.fields.get("char").and_then(Value::as_u64).map(|c| c as usize);
            if let Some(n) = n {
                set.characters = set.characters.chars().enumerate().filter(|(j, _)| *j != n).map(|(_, c)| c).collect();
            }
            d.fields.remove("char");
            Ok(None)
        }
        Action::New | Action::Delete | Action::Import | Action::Export | Action::Custom | Action::Sample => Ok(None),
    }
}

/// The table of `f`'s parts: a row for each class, then for each custom character set, with its
/// font, style, size, baseline, scales and (Kana) Scale at Center; the values a row can't take are
/// not shown. Past [`TABLE_HEIGHT`] the table scrolls, so the controls under it stay on screen.
fn parts_table(app: &VectorcraftApp, ui: &mut egui::Ui, f: &mut CompositeFont) {
    egui::ScrollArea::vertical().id_salt("composite-fonts-table").max_height(TABLE_HEIGHT).show(ui, |ui| parts_grid(app, ui, f));
}

/// The rows of [`parts_table`].
fn parts_grid(app: &VectorcraftApp, ui: &mut egui::Ui, f: &mut CompositeFont) {
    egui::Grid::new("composite-fonts-parts").num_columns(8).spacing([8.0, 6.0]).show(ui, |ui| {
        for h in ["", tl!("Font"), tl!("Style"), tl!("Size"), tl!("Baseline"), tl!("Horizontal Scale"), tl!("Vertical Scale"), ""] {
            widgets::dim_label(ui, h);
        }
        ui.end_row();
        for class in CharClass::ALL {
            ui.label(class_label(class));
            part_row(app, ui, class.key(), f.parts.get_mut(class), class.scales(), class == CharClass::Kana);
        }
        for (n, s) in f.custom.iter_mut().enumerate() {
            // A set's name is the user's own, shown as it is.
            ui.label(s.name.as_str());
            part_row(app, ui, &format!("custom-{n}"), &mut s.part, true, false);
        }
    });
}

/// The cells of part `p`'s row (keyed `key`) after its label: font, style, size, baseline, the
/// scales when `scales`, and Scale at Center when `center`.
fn part_row(app: &VectorcraftApp, ui: &mut egui::Ui, key: &str, p: &mut CompositePart, scales: bool, center: bool) {
    let db = vectorcraft_text::FontDb::global();
    let id = ("composite-font", key);
    if let Some(crate::font_menu::FontPick::Chosen(family, style)) =
        crate::font_menu::font_menu(ui, id, &p.font, 190.0, None, crate::font_menu::MenuLook::of(app))
        && db.composite_font(&family).is_none()
    {
        p.style = style.unwrap_or_else(|| db.styles(&family).first().cloned().unwrap_or_else(|| "Regular".into()));
        p.font = family;
    }
    let styles = db.styles(&p.font);
    let style_names: Vec<&str> = styles.iter().map(String::as_str).collect();
    if let Some(k) = widgets::dropdown_names(ui, ("composite-font-style", key), &p.style, &style_names, 120.0)
        && let Some(s) = styles.get(k)
    {
        p.style.clone_from(s);
    }
    let percent = |ui: &mut egui::Ui, field: &str, v: &mut f64, range: &std::ops::RangeInclusive<f64>| {
        if let Some(n) = widgets::range_field(ui, ("composite-font", key, field), *v, range.clone(), "%", 1, 64.0) {
            *v = n;
        }
    };
    percent(ui, "size", &mut p.size, &COMPOSITE_SIZE);
    percent(ui, "baseline", &mut p.baseline, &COMPOSITE_BASELINE);
    if scales {
        percent(ui, "hScale", &mut p.h_scale, &COMPOSITE_SCALE);
        percent(ui, "vScale", &mut p.v_scale, &COMPOSITE_SCALE);
    } else {
        ui.label("");
        ui.label("");
    }
    if center {
        ui.checkbox(&mut p.scale_at_center, tl!("Scale at Center"));
    } else {
        ui.label("");
    }
    ui.end_row();
}

/// Import…: the fonts of a picked file join those in the dialog, a font of the same name replaced,
/// and the first of them is picked.
fn import(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
    let pick = FilePick { filters: vec![FilePick::filter("Composite fonts", EXTS)], ..Default::default() };
    let path = crate::picks::open(app, &pick).ok_or("cancelled")?;
    let r = app.run("type.compositeFonts.import", json!({ "path": path, "apply": false }))?;
    let read: Vec<CompositeFont> = serde_json::from_value(r["fonts"].clone()).map_err(|e| e.to_string())?;
    let mut list = fonts(d);
    let mut first = None;
    for f in read {
        let at = match list.iter_mut().enumerate().find(|(_, g)| CompositeFont::same_name(&g.name, &f.name)) {
            Some((k, g)) => {
                *g = f;
                k
            }
            None => {
                list.push(f);
                list.len() - 1
            }
        };
        first.get_or_insert(at);
    }
    put_fonts(d, &list);
    if let Some(k) = first {
        select(d, k);
    }
    Ok(())
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let r = app.run(COMMAND, json!({ "fonts": fonts(d) }));
    // A name two fonts share, or a value out of range, keeps the dialog open to be changed.
    if r.is_ok() {
        app.ui.dialog = None;
    }
    r
}

#[cfg(test)]
mod tests;
