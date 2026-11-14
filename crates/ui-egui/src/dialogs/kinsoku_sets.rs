//! Type › Kinsoku Shori Settings… (with Show East Asian Options): the active document's custom
//! Kinsoku Shori sets, the one picked in Kinsoku shown with its four categories: Cannot Start Line,
//! Cannot End Line, Hanging Punctuation and Inseparable Characters. New adds a set based on Hard,
//! Soft, None or a set of the dialog, Delete takes the one picked out, and Add puts the character
//! of a Unicode code point into a category. Nothing changes in the document until OK,
//! which runs `type.kinsokuSets.set` with every set as one undo step; Cancel leaves them as they were.
//!
//! Fields: `sets` (the sets as `type.kinsokuSets.set` takes them, each with the name it had in the
//! document as `from`), `selected` (the index of the one picked), `name`, `cannotStart`,
//! `cannotEnd`, `hanging` and `inseparable` (its name and categories as typed, the inseparable
//! sequences separated by spaces), `base` (what New starts from), `code` (the code point Add
//! adds), `into` (the category it adds to) and `shown` (the set the name and category fields
//! were filled from). When `sets` or `selected` is changed so the picked set is no longer `shown`,
//! the fields are filled from the picked set again and what was typed in them is dropped.
//!
//! Behavior reference: the help page
//! <https://helpx.adobe.com/illustrator/desktop/design-with-text/edit-format-text/use-kinsoku-for-japanese-line-breaking.html>.

use serde_json::{Map, Value, json};
use vectorcraft_doc::KinsokuSet;

use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Kinsoku Shori Settings.
pub const KIND: &str = "kinsokuSets";

/// The command OK runs, and the menu item's.
pub const COMMAND: &str = vectorcraft_engine::cmd::kinsokusets::SET;

/// The categories, as the command's keys, in the order the dialog shows them.
const CATEGORIES: [&str; 4] = ["cannotStart", "cannotEnd", "hanging", "inseparable"];

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Kinsoku Shori Settings").into(), body, confirm, min_width: 560.0, ..DialogSpec::FORM };

/// Open Kinsoku Shori Settings on the active document's custom sets, the first one picked.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let sets: Vec<Value> = app
        .session
        .active()
        .ok_or("no document open")?
        .doc
        .kinsoku_sets
        .iter()
        .map(|k| {
            let mut v = json!(k);
            if let Some(o) = v.as_object_mut() {
                o.insert("from".into(), json!(k.name));
            }
            v
        })
        .collect();
    let mut d = Dialog::new(KIND, json!({ "sets": sets, "selected": 0, "base": "hard", "into": "cannotStart", "code": "" }));
    show_selected(&mut d);
    app.ui.dialog = Some(d);
    Ok(Value::Null)
}

/// The sets the dialog holds, as the command takes them.
fn sets(d: &Dialog) -> Vec<Map<String, Value>> {
    d.fields.get("sets").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_object().cloned()).collect()).unwrap_or_default()
}

fn put_sets(d: &mut Dialog, sets: &[Map<String, Value>]) {
    d.fields.insert("sets".into(), json!(sets));
}

fn name_of(set: &Map<String, Value>) -> &str {
    set.get("name").and_then(Value::as_str).unwrap_or_default()
}

/// The index of the set picked, within the sets (0 when there are none).
fn selected(d: &Dialog, count: usize) -> usize {
    d.fields.get("selected").and_then(Value::as_u64).map_or(0, |i| i as usize).min(count.saturating_sub(1))
}

/// Pick set `i`: its name and categories go into the fields.
fn select(d: &mut Dialog, i: usize) {
    d.fields.insert("selected".into(), json!(i));
    show_selected(d);
}

fn show_selected(d: &mut Dialog) {
    let list = sets(d);
    let set = list.get(selected(d, list.len())).cloned().unwrap_or_default();
    d.fields.insert("name".into(), json!(name_of(&set)));
    for key in CATEGORIES {
        let text = match set.get(key) {
            Some(Value::Array(seqs)) => seqs.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "),
            Some(Value::String(s)) => s.clone(),
            _ => String::new(),
        };
        d.fields.insert(key.into(), json!(text));
    }
    d.fields.insert("shown".into(), Value::Object(set));
}

/// Do the name and category fields describe the set picked, `set`?
fn shows(d: &Dialog, set: &Map<String, Value>) -> bool {
    d.fields.get("shown").and_then(Value::as_object) == Some(set)
}

/// The set picked with the name and categories as typed: space characters are left out of the
/// single-character categories, and the inseparable sequences are the words of their field.
fn take_fields(d: &Dialog, set: &mut Map<String, Value>) {
    set.insert("name".into(), json!(d.str("name")));
    for key in CATEGORIES {
        let text = d.str(key);
        let v = if key == "inseparable" {
            json!(text.split_whitespace().collect::<Vec<_>>())
        } else {
            json!(text.chars().filter(|c| !c.is_whitespace()).collect::<String>())
        };
        set.insert(key.into(), v);
    }
}

/// "Kinsoku Set N", the first N no set in `sets` is named.
fn new_name(sets: &[Map<String, Value>]) -> String {
    (1..).map(|n| format!("Kinsoku Set {n}")).find(|n| !sets.iter().any(|s| KinsokuSet::same_name(name_of(s), n))).unwrap_or_default()
}

/// The character `code` names: `U+3001`, `u+3001` or `3001`, in hexadecimal.
fn code_char(code: &str) -> Option<char> {
    let t = code.trim();
    let hex = t.strip_prefix("U+").or_else(|| t.strip_prefix("u+")).unwrap_or(t);
    u32::from_str_radix(hex, 16).ok().and_then(char::from_u32).filter(|c| !c.is_control() && !c.is_whitespace())
}

/// What a button asked for, done once the fields are drawn.
enum Action {
    New,
    Delete,
    Add,
}

/// New's starting points: the built-in sets, then the dialog's own.
fn bases(list: &[Map<String, Value>]) -> Vec<(String, String)> {
    let mut v = vec![
        (tl!("Hard").to_string(), "hard".to_string()),
        (tl!("Soft").to_string(), "soft".to_string()),
        (tl!("None").to_string(), "none".to_string()),
    ];
    v.extend(list.iter().map(|s| (name_of(s).to_string(), name_of(s).to_string())));
    v
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let mut list = sets(d);
    let i = selected(d, list.len());
    if list.get(i).is_some_and(|set| !shows(d, set)) {
        show_selected(d);
    }
    let mut act = None;
    let mut picked = None;
    let names: Vec<String> = list.iter().map(|s| name_of(s).to_string()).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let bases = bases(&list);
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Kinsoku Set"));
        if names.is_empty() {
            widgets::dim_label(ui, tl!("None"));
        } else if let Some(k) = widgets::dropdown_names(ui, "kinsoku-sets-set", names.get(i).copied().unwrap_or_default(), &names, 200.0) {
            picked = Some(k);
        }
        ui.add_space(8.0);
        ui.spacing_mut().item_spacing.x = 4.0;
        if widgets::flat_button(ui, tl!("New"), 48.0).clicked() {
            act = Some(Action::New);
        }
        widgets::dim_label(ui, tl!("Based on:"));
        let base = d.str("base");
        let shown = bases.iter().find(|b| b.1 == base).map_or(base.as_str(), |b| b.0.as_str());
        let base_names: Vec<&str> = bases.iter().map(|b| b.0.as_str()).collect();
        if let Some(k) = widgets::dropdown_names(ui, "kinsoku-sets-base", shown, &base_names, 120.0)
            && let Some(b) = bases.get(k)
        {
            d.fields.insert("base".into(), json!(b.1));
        }
        if ui.add_enabled_ui(!list.is_empty(), |ui| widgets::flat_button(ui, tl!("Delete"), 56.0)).inner.clicked() {
            act = Some(Action::Delete);
        }
    });
    if let Some(k) = picked {
        select(d, k);
        return false;
    }
    ui.add_space(8.0);
    if let Some(set) = list.get_mut(i) {
        egui::Grid::new("kinsoku-sets-fields").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            widgets::dim_label(ui, tl!("Name:"));
            form::text(ui, d, "name", 260.0);
            ui.end_row();
            for (key, label) in [
                ("cannotStart", tl!("Cannot Start Line")),
                ("cannotEnd", tl!("Cannot End Line")),
                ("hanging", tl!("Hanging Punctuation")),
                ("inseparable", tl!("Inseparable Characters")),
            ] {
                widgets::dim_label(ui, label);
                form::text_area(ui, d, key, 380.0, 2);
                ui.end_row();
            }
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            widgets::dim_label(ui, tl!("Unicode"));
            form::text(ui, d, "code", 80.0);
            let into = d.str("into");
            let targets = [("cannotStart", tl!("Cannot Start Line")), ("cannotEnd", tl!("Cannot End Line")), ("hanging", tl!("Hanging Punctuation"))];
            let shown = targets.iter().find(|t| t.0 == into).map_or("", |t| t.1);
            if let Some(k) = widgets::dropdown_names(ui, "kinsoku-sets-into", shown, &targets.map(|t| t.1), 160.0)
                && let Some(t) = targets.get(k)
            {
                d.fields.insert("into".into(), json!(t.0));
            }
            if ui.add_enabled_ui(code_char(&d.str("code")).is_some(), |ui| widgets::flat_button(ui, tl!("Add"), 48.0)).inner.clicked() {
                act = Some(Action::Add);
            }
        });
        let was = name_of(set).to_string();
        take_fields(d, set);
        // Based on follows the set it names through a rename.
        if d.str("base") == was && name_of(set) != was {
            let now = name_of(set).to_string();
            d.fields.insert("base".into(), json!(now));
        }
        d.fields.insert("shown".into(), Value::Object(set.clone()));
    }
    put_sets(d, &list);
    if let Some(a) = act
        && let Err(e) = run(app, d, a)
    {
        app.status(e);
    }
    false
}

/// Do what button `act` asks for on the dialog's sets.
fn run(app: &mut VectorcraftApp, d: &mut Dialog, act: Action) -> Result<(), String> {
    let mut list = sets(d);
    let i = selected(d, list.len());
    match act {
        Action::New => {
            let base = d.str("base");
            let mut set = match list.iter().find(|s| name_of(s) == base) {
                Some(s) => s.clone(),
                None => {
                    let r = app.run("type.kinsokuSets.base", json!({ "base": base }))?;
                    r["set"].as_object().cloned().ok_or("no Kinsoku set to base the new one on")?
                }
            };
            set.remove("from");
            set.insert("name".into(), json!(new_name(&list)));
            list.push(set);
            put_sets(d, &list);
            select(d, list.len() - 1);
        }
        Action::Delete => {
            if i < list.len() {
                let gone = name_of(&list[i]).to_string();
                list.remove(i);
                if d.str("base") == gone {
                    d.fields.insert("base".into(), json!("hard"));
                }
            }
            put_sets(d, &list);
            select(d, i.min(list.len().saturating_sub(1)));
        }
        Action::Add => {
            let c = code_char(&d.str("code")).ok_or("give a character's Unicode code point, as U+3001")?;
            let into = d.str("into");
            let key = if CATEGORIES.contains(&into.as_str()) && into != "inseparable" { into } else { "cannotStart".into() };
            let mut text = d.str(&key);
            if !text.contains(c) {
                text.push(c);
            }
            d.fields.insert(key, json!(text));
            d.fields.insert("code".into(), json!(""));
            if let Some(set) = list.get_mut(i) {
                take_fields(d, set);
                d.fields.insert("shown".into(), Value::Object(set.clone()));
            }
            put_sets(d, &list);
        }
    }
    Ok(())
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    // The fields as typed, also when they were set (`ui.dialog.set`) with no frame drawn since,
    // while they still describe the set picked.
    let mut list = sets(d);
    let i = selected(d, list.len());
    if let Some(set) = list.get_mut(i)
        && shows(d, set)
    {
        take_fields(d, set);
    }
    let r = app.run(COMMAND, json!({ "sets": list }));
    // A name two sets share, a reserved name or a category too long keeps the dialog open to be
    // changed.
    if r.is_ok() {
        app.ui.dialog = None;
    }
    r
}

#[cfg(test)]
mod tests;
