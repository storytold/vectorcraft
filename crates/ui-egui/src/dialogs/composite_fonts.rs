//! Type › Composite Fonts… (with Show East Asian Options): the active document's composite fonts,
//! the one picked in Font shown as a table with a row for each class of character (Kanji, Kana,
//! Punctuation, Symbols, Roman, Numerals) and its font, style, size, baseline and scales. New adds
//! a font based on the one picked, Delete Font takes it out, Import… adds the fonts of a file and
//! Export… writes the one picked to a file. Nothing changes in the document until OK, which runs
//! `type.compositeFonts.set` with every font as one undo step; Cancel leaves them as they were.
//!
//! Fields: `fonts` (the composite fonts as `type.compositeFonts.set` takes them), `selected` (the
//! index of the one picked) and `name` (its name as typed).
//!
//! Behavior reference (public documentation):
//! <https://help.adobe.com/archive/en_US/illustrator/cs3/illustrator_cs3_help.pdf#page=347>.

use serde_json::{Value, json};
use vectorcraft_doc::text::{COMPOSITE_BASELINE, COMPOSITE_SCALE, COMPOSITE_SIZE};
use vectorcraft_doc::{CharClass, CompositeFont, CompositePart};
use vectorcraft_engine::cmd::compositefonts::EXTS;

use super::{DialogSpec, form};
use crate::picks::PickRequest;
use crate::state::Dialog;
use crate::{FilePick, VectorcraftApp, widgets};

/// The dialog kind of Composite Fonts.
pub const KIND: &str = "compositeFonts";

/// The command OK runs, and the menu item's.
pub const COMMAND: &str = "type.compositeFonts.set";

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |_| tl!("Composite Fonts").into(), body, confirm, min_width: 760.0, ..DialogSpec::FORM };

/// Open Composite Fonts on the active document's composite fonts, the first one picked.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let fonts = app.session.active().ok_or("no document open")?.doc.composite_fonts.to_vec();
    let mut d = Dialog::new(KIND, json!({ "fonts": fonts, "selected": 0 }));
    show_selected(&mut d);
    app.ui.dialog = Some(d);
    Ok(Value::Null)
}

/// The fonts the dialog holds.
fn fonts(d: &Dialog) -> Vec<CompositeFont> {
    d.fields.get("fonts").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default()
}

fn put_fonts(d: &mut Dialog, fonts: &[CompositeFont]) {
    d.fields.insert("fonts".into(), json!(fonts));
}

/// The index of the font picked, within the fonts (0 when there are none).
fn selected(d: &Dialog, count: usize) -> usize {
    d.fields.get("selected").and_then(Value::as_u64).map_or(0, |i| i as usize).min(count.saturating_sub(1))
}

/// Pick font `i`: its name goes into Name.
fn select(d: &mut Dialog, i: usize) {
    d.fields.insert("selected".into(), json!(i));
    show_selected(d);
}

fn show_selected(d: &mut Dialog) {
    let fonts = fonts(d);
    let name = fonts.get(selected(d, fonts.len())).map(|f| f.name.clone()).unwrap_or_default();
    d.fields.insert("name".into(), json!(name));
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

/// What a button asked for, done once the table is drawn.
enum Action {
    New,
    Delete,
    Import,
    Export,
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
    }
    Ok(())
}

/// The table of `f`'s parts: a row for each class with its font, style, size, baseline, scales and
/// (Kana) Scale at Center; the values a class can't take are not shown.
fn parts_table(app: &VectorcraftApp, ui: &mut egui::Ui, f: &mut CompositeFont) {
    let db = vectorcraft_text::FontDb::global();
    egui::Grid::new("composite-fonts-parts").num_columns(8).spacing([8.0, 6.0]).show(ui, |ui| {
        for h in ["", tl!("Font"), tl!("Style"), tl!("Size"), tl!("Baseline"), tl!("Horizontal Scale"), tl!("Vertical Scale"), ""] {
            widgets::dim_label(ui, h);
        }
        ui.end_row();
        for class in CharClass::ALL {
            let p: &mut CompositePart = f.parts.get_mut(class);
            ui.label(class_label(class));
            let id = ("composite-font", class.key());
            if let Some(crate::font_menu::FontPick::Chosen(family, style)) =
                crate::font_menu::font_menu(ui, id, &p.font, 190.0, None, crate::font_menu::MenuLook::of(app))
                && db.composite_font(&family).is_none()
            {
                p.style = style.unwrap_or_else(|| db.styles(&family).first().cloned().unwrap_or_else(|| "Regular".into()));
                p.font = family;
            }
            let styles = db.styles(&p.font);
            let style_names: Vec<&str> = styles.iter().map(String::as_str).collect();
            if let Some(k) = widgets::dropdown_names(ui, ("composite-font-style", class.key()), &p.style, &style_names, 120.0)
                && let Some(s) = styles.get(k)
            {
                p.style.clone_from(s);
            }
            let percent = |ui: &mut egui::Ui, key: &str, v: &mut f64, range: &std::ops::RangeInclusive<f64>| {
                if let Some(n) = widgets::range_field(ui, ("composite-font", class.key(), key), *v, range.clone(), "%", 1, 64.0) {
                    *v = n;
                }
            };
            percent(ui, "size", &mut p.size, &COMPOSITE_SIZE);
            percent(ui, "baseline", &mut p.baseline, &COMPOSITE_BASELINE);
            if class.scales() {
                percent(ui, "hScale", &mut p.h_scale, &COMPOSITE_SCALE);
                percent(ui, "vScale", &mut p.v_scale, &COMPOSITE_SCALE);
            } else {
                ui.label("");
                ui.label("");
            }
            if class == CharClass::Kana {
                ui.checkbox(&mut p.scale_at_center, tl!("Scale at Center"));
            } else {
                ui.label("");
            }
            ui.end_row();
        }
    });
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
