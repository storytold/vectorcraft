//! Type › Kinsoku Shori Settings… (with Show East Asian Options): the active document's custom
//! Kinsoku Shori sets, the one picked in Kinsoku Set shown with its four categories: Cannot Start Line,
//! Cannot End Line, Hanging Punctuation and Inseparable Characters. New adds a set based on Hard,
//! Soft, None or a set of the dialog, Delete takes the one picked out, and Add puts the character
//! of a Unicode code point into a category. Nothing changes in the document until OK,
//! which runs `type.kinsokuSets.set` with every set as one undo step; Cancel leaves them as they were.
//!
//! Fields for automation: `sets` (the sets as `type.kinsokuSets.set` takes them, each with the name it
//! had in the document as `from`), `selected` (the index of the one picked), `name` (renames the one
//! picked), `cannotStart`, `cannotEnd`, `hanging` and `inseparable` (its categories as typed, the
//! inseparable sequences separated by spaces), `base` (what New starts from), `code` (the code
//! point Add adds) and `into` (the category it adds to). A `sets` that is not a list of sets is kept
//! as it is: no frame or button replaces it, and OK sends it to the command, which refuses it.
//!
//! Behavior reference: the help page
//! <https://helpx.adobe.com/illustrator/desktop/design-with-text/edit-format-text/use-kinsoku-for-japanese-line-breaking.html>.

use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, FieldRefusal, HeldAnswer, HeldOk, StackDialog, dialog_id};
use craft::ui::display::Text;
use craft::ui::form::{Field, FieldGroup};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::patterns::presets::{
    NameCheck, NamedPresetEvent, NamedPresetRow, PickerField, PresetError, PresetPicker, PresetRow, PresetSource, PresetState,
};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Label;
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::text::TextField;
use egui::{Context, Ui};
use serde_json::{Map, Value, json};
use vectorcraft_doc::{Kinsoku, KinsokuSet};

use crate::VectorcraftApp;

/// The command OK runs, the menu item's, and the key of the dialog on the host.
pub const COMMAND: &str = vectorcraft_engine::cmd::kinsokusets::SET;

/// The categories, as the command's keys, in the order the dialog shows them.
const CATEGORIES: [&str; 4] = ["cannotStart", "cannotEnd", "hanging", "inseparable"];

/// The categories with their labels; Add puts a character into the first three.
fn categories() -> [(&'static str, &'static str); 4] {
    [
        ("cannotStart", tl!("Cannot Start Line")),
        ("cannotEnd", tl!("Cannot End Line")),
        ("hanging", tl!("Hanging Punctuation")),
        ("inseparable", tl!("Inseparable Characters")),
    ]
}

/// What OK files for the pump: the document the dialog opened on, the names of its sets then, and the sets.
#[derive(Clone)]
struct Request {
    doc: Option<u64>,
    opened: Vec<String>,
    sets: Value,
}

/// The Kinsoku Shori Settings dialog on the host.
pub(crate) struct KinsokuDialog {
    id: DialogId,
    /// The uid of the document the dialog opened on, which OK sets the sets of.
    doc: Option<u64>,
    /// The names of that document's sets when the dialog opened: OK keeps the sets the document gained
    /// since, after the dialog's own, and New's suggestion skips these names.
    opened: Vec<String>,
    sets: Value,
    selected: usize,
    /// The picked set's categories as typed, in the order of [`CATEGORIES`].
    cats: [String; 4],
    base: String,
    code: String,
    into: String,
    picker: PresetState<String>,
    held: HeldAnswer<Request>,
    /// Why the last action or the last OK was refused, until a value changes.
    problem: Option<String>,
}

/// Open Kinsoku Shori Settings on the active document's custom sets, the first one picked.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    let dialog = KinsokuDialog::new(st.uid, st.doc.kinsoku_sets.iter());
    app.dialogs().ok_or("no window")?.with(|host| host.open(dialog));
    Ok(Value::Null)
}

/// A set of the document as the dialog lists it: as `type.kinsokuSets.set` takes it, with its name
/// in the document as `from`.
fn listed(k: &KinsokuSet) -> Value {
    let mut v = json!(k);
    if let Some(o) = v.as_object_mut() {
        o.insert("from".into(), json!(k.name));
    }
    v
}

fn name_of(set: &Map<String, Value>) -> &str {
    set.get("name").and_then(Value::as_str).unwrap_or_default()
}

/// The text of category `key` of `set` as the field shows it.
fn category_text(set: &Map<String, Value>, key: &str) -> String {
    match set.get(key) {
        Some(Value::Array(seqs)) => seqs.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "),
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

/// The categories as typed, into `set`: space characters are left out of the single-character
/// categories, and the inseparable sequences are the words of their field.
fn take_fields(cats: &[String; 4], set: &mut Map<String, Value>) {
    for (key, text) in CATEGORIES.iter().zip(cats) {
        let v = if *key == "inseparable" {
            json!(text.split_whitespace().collect::<Vec<_>>())
        } else {
            json!(text.chars().filter(|c| !c.is_whitespace()).collect::<String>())
        };
        set.insert((*key).into(), v);
    }
}

/// "Kinsoku Set N", the first N that no set in `sets` and none of the document's sets at open (`opened`) is
/// named. A set Delete took out keeps its name out of New's, so OK gives its paragraphs Hard instead of
/// the new set's characters.
fn new_name(sets: &[Map<String, Value>], opened: &[String]) -> String {
    let taken = |n: &str| sets.iter().any(|s| KinsokuSet::same_name(name_of(s), n)) || opened.iter().any(|k| KinsokuSet::same_name(k, n));
    (1..).map(|n| format!("Kinsoku Set {n}")).find(|n| !taken(n)).unwrap_or_default()
}

/// Why `typed` cannot name a set beside `sets`: the document's naming rule, or another set of the dialog with it.
fn name_problem(sets: &[Map<String, Value>], typed: &str) -> Option<PresetError> {
    if KinsokuSet::clean_name(typed).is_none() {
        return Some(PresetError::InvalidName);
    }
    sets.iter().any(|s| KinsokuSet::same_name(name_of(s), typed)).then_some(PresetError::NameTaken)
}

/// The character `code` names: `U+3001`, `u+3001` or `3001`, in hexadecimal.
fn code_char(code: &str) -> Option<char> {
    let t = code.trim();
    let hex = t.strip_prefix("U+").or_else(|| t.strip_prefix("u+")).unwrap_or(t);
    u32::from_str_radix(hex, 16).ok().and_then(char::from_u32).filter(|c| !c.is_control() && !c.is_whitespace())
}

/// New's starting points: the built-in sets, then the dialog's own.
fn bases(list: &[Map<String, Value>]) -> Vec<Entry<String>> {
    let mut v = vec![
        Entry::new("hard".to_owned(), tl!("Hard").to_owned()),
        Entry::new("soft".to_owned(), tl!("Soft").to_owned()),
        Entry::new("none".to_owned(), tl!("None").to_owned()),
    ];
    v.extend(list.iter().map(|s| Entry::new(name_of(s).to_owned(), name_of(s).to_owned())));
    v
}

/// The built-in set `base` names, as the dialog holds a set.
fn built_in(base: &str) -> Result<Map<String, Value>, String> {
    let kind = match base {
        "hard" => Kinsoku::Hard,
        "soft" => Kinsoku::Soft,
        "none" => Kinsoku::None,
        _ => return Err("no Kinsoku set to base the new one on".into()),
    };
    json!(vectorcraft_text::kinsoku_set_from(kind, "")).as_object().cloned().ok_or_else(|| "no Kinsoku set to base the new one on".into())
}

/// The text a field of automation holds.
fn text_of(field: &str, value: &Value) -> Result<String, String> {
    value.as_str().map(str::to_owned).ok_or_else(|| FieldRefusal::Rejects(value).message(field))
}

impl KinsokuDialog {
    fn new<'a>(doc: u64, document_sets: impl Iterator<Item = &'a KinsokuSet> + Clone) -> Self {
        let mut d = KinsokuDialog {
            id: dialog_id(COMMAND),
            doc: Some(doc),
            opened: document_sets.clone().map(|k| k.name.clone()).collect(),
            sets: Value::Array(document_sets.map(listed).collect()),
            selected: 0,
            cats: Default::default(),
            base: "hard".into(),
            code: String::new(),
            into: "cannotStart".into(),
            picker: PresetState::default(),
            held: HeldAnswer::new(),
            problem: None,
        };
        d.load();
        d
    }

    /// The sets the dialog holds that are sets, as the command takes them.
    fn list(&self) -> Vec<Map<String, Value>> {
        self.sets.as_array().map(|a| a.iter().filter_map(|v| v.as_object().cloned()).collect()).unwrap_or_default()
    }

    /// Is `sets` a list of sets? One that is not is kept as it is.
    fn holds_sets(&self) -> bool {
        self.sets.as_array().is_some_and(|a| a.iter().all(Value::is_object))
    }

    fn require_list(&self) -> Result<Vec<Map<String, Value>>, String> {
        if self.holds_sets() { Ok(self.list()) } else { Err("the sets must be a list of Kinsoku sets".into()) }
    }

    /// The index of the set picked, within `count` sets (0 when there are none).
    fn index(&self, count: usize) -> usize {
        self.selected.min(count.saturating_sub(1))
    }

    /// Fills the category fields from the set picked.
    fn load(&mut self) {
        let list = self.list();
        let set = list.get(self.index(list.len()));
        for (text, key) in self.cats.iter_mut().zip(CATEGORIES) {
            *text = set.map(|s| category_text(s, key)).unwrap_or_default();
        }
    }

    /// Writes the category fields into the set picked, so `sets` always holds what is typed.
    fn commit(&mut self) {
        if !self.holds_sets() {
            return;
        }
        let mut list = self.list();
        let i = self.index(list.len());
        if let Some(set) = list.get_mut(i) {
            take_fields(&self.cats, set);
            self.sets = json!(list);
        }
    }

    /// A value changed: a refusal shown so far ends.
    fn edited(&mut self) {
        self.problem = None;
        self.held.clear_refusal();
    }

    /// Picks set `i`, its categories going into the fields.
    fn pick(&mut self, i: usize) {
        self.commit();
        self.selected = i;
        self.load();
    }

    fn pick_named(&mut self, name: &str) {
        if let Some(i) = self.list().iter().position(|s| name_of(s) == name) {
            self.pick(i);
        }
    }

    /// Adds a set named `name` based on the choice in Based On, and picks it.
    pub(crate) fn create(&mut self, name: &str) -> Result<(), String> {
        self.commit();
        let mut list = self.require_list()?;
        let mut set = match list.iter().find(|s| name_of(s) == self.base) {
            Some(s) => s.clone(),
            None => built_in(&self.base)?,
        };
        set.remove("from");
        set.insert("name".into(), json!(name));
        list.push(set);
        // The fields are already committed: picking would write them into the new set when the old pick has its index.
        self.selected = list.len() - 1;
        self.sets = json!(list);
        self.load();
        Ok(())
    }

    /// Renames the set `from` to `to`; Based On follows the set it names.
    pub(crate) fn rename(&mut self, from: &str, to: &str) -> Result<(), String> {
        let mut list = self.require_list()?;
        let set = list.iter_mut().find(|s| name_of(s) == from).ok_or("no such Kinsoku set")?;
        set.insert("name".into(), json!(to));
        if self.base == from {
            self.base = to.to_owned();
        }
        self.sets = json!(list);
        Ok(())
    }

    /// Takes the set `name` out; a Based On naming it falls back to Hard.
    pub(crate) fn delete(&mut self, name: &str) -> Result<(), String> {
        self.commit();
        let mut list = self.require_list()?;
        let i = list.iter().position(|s| name_of(s) == name).ok_or("no such Kinsoku set")?;
        let picked = self.index(list.len());
        list.remove(i);
        if self.base == name {
            self.base = "hard".into();
        }
        self.selected = if i < picked { picked - 1 } else { picked };
        self.sets = json!(list);
        self.load();
        Ok(())
    }

    /// Puts the character of the Unicode code point into the category Add targets, unless it is there.
    pub(crate) fn add_code(&mut self) -> Result<(), String> {
        self.require_list()?;
        let c = code_char(&self.code).ok_or("give a character's Unicode code point, as U+3001")?;
        let at = CATEGORIES.iter().take(3).position(|k| *k == self.into).unwrap_or(0);
        if let Some(text) = self.cats.get_mut(at)
            && !text.contains(c)
        {
            text.push(c);
        }
        self.code.clear();
        self.commit();
        Ok(())
    }

    fn apply(&mut self, events: Vec<NamedPresetEvent<String>>) {
        for event in events {
            let done = match event {
                NamedPresetEvent::Apply(name) => {
                    self.pick_named(&name);
                    Ok(())
                }
                NamedPresetEvent::Create { name, .. } => self.create(&name),
                NamedPresetEvent::Rename { key, text, .. } => self.rename(&key, &text),
                NamedPresetEvent::Delete(key) => self.delete(&key),
                _ => Ok(()),
            };
            self.edited();
            self.problem = done.err();
        }
    }

    /// Files OK's answer for the pump, with what is typed.
    fn file_ok(&mut self) {
        if self.held.is_waiting() {
            return;
        }
        self.commit();
        self.problem = None;
        self.held.file(Request { doc: self.doc, opened: self.opened.clone(), sets: self.sets.clone() });
    }

    /// The answer OK filed, once.
    fn take_ok(&mut self) -> Option<Request> {
        self.held.take()
    }

    /// The pump's answer to the filed OK: a refusal keeps the dialog open with its reason.
    fn answer_ok(&mut self, answer: Result<(), String>) {
        if let Err(reason) = &answer {
            self.problem = Some(reason.clone());
        }
        self.held.answer(answer.map_err(Label::from));
    }

    fn body(&mut self, ui: &mut Ui) {
        let list = self.list();
        let rows: Vec<PresetRow<String>> = list.iter().map(|s| PresetRow::item(name_of(s).to_owned(), name_of(s), 0)).collect();
        let mut value = list.get(self.index(list.len())).map(|s| name_of(s).to_owned());
        let suggestion = new_name(&list, &self.opened);
        // PLAN: PresetPicker has no placeholder: an empty list shows an empty name field, not "None".
        let source = PresetSource::listed(&rows);
        let refuses = |typed: &str| name_problem(&list, typed);
        let picker = PresetPicker::new(tl!("Kinsoku Set"), Bind::bound(&mut value), &source, Bind::bound(&mut self.picker)).field(PickerField::Name);
        let out = NamedPresetRow::new(tl!("Kinsoku Set"), picker).suggest_name(suggestion).check_name(NameCheck::of_host(&refuses)).show(ui);
        self.apply(out.events);

        let list = self.list();
        let entries = bases(&list);
        Row::new().show(ui, |ui, _| {
            Text::new(tl!("Based on:")).show(ui);
            if let Some(base) = Select::new(tl!("Based on:"), Bind::given(self.base.clone()), &entries).show(ui).value().cloned() {
                self.base = base;
                self.edited();
            }
        });
        if list.is_empty() {
            self.show_problem(ui);
            return;
        }
        let before = self.cats.clone();
        FieldGroup::new(tl!("Categories")).show(ui, |ui, _| {
            for ((_, label), text) in categories().into_iter().zip(self.cats.iter_mut()) {
                Field::new(label).show(ui, |ui, _| {
                    TextField::new(label, Bind::bound(text)).lines(2).show(ui);
                });
            }
        });
        if self.cats != before {
            self.commit();
            self.edited();
        }
        let targets: Vec<Entry<String>> = categories().into_iter().take(3).map(|(key, label)| Entry::new(key.to_owned(), label.to_owned())).collect();
        let mut add = false;
        Row::new().show(ui, |ui, _| {
            Text::new(tl!("Unicode")).show(ui);
            TextField::new(tl!("Unicode"), Bind::bound(&mut self.code)).show(ui);
            if let Some(into) = Select::new(tl!("Add to"), Bind::given(self.into.clone()), &targets).show(ui).value().cloned() {
                self.into = into;
            }
            add = Button::new(tl!("Add")).enabled(code_char(&self.code).is_some()).show(ui).clicked();
        });
        if add {
            self.problem = self.add_code().err();
        }
        self.show_problem(ui);
    }

    fn show_problem(&self, ui: &mut Ui) {
        // PLAN: craft_ui does not re-export `Tone`, which `Notice` needs, so the reason shows as plain text.
        if let Some(problem) = &self.problem {
            Text::new(problem.clone()).show(ui);
        }
    }
}

impl StackDialog for KinsokuDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _stack: &DialogStack) -> Option<DialogResult<()>> {
        if self.held.finish().is_some() {
            return Some(DialogResult::Ok(()));
        }
        let waiting = self.held.is_waiting();
        let shown = Dialog::new(tl!("Kinsoku Shori Settings"))
            .buttons([
                FooterButton::new(tl!("OK"), DialogRole::Accept).stays_open(true).enabled(!waiting),
                FooterButton::new(tl!("Cancel"), DialogRole::Cancel),
            ])
            .show(ui, |ui, _| {
                Stack::new().show(ui, |ui, _| self.body(ui));
            });
        if shown.output.picked == Some(0) {
            self.file_ok();
        }
        shown.output.closed.map(|_| DialogResult::Cancel)
    }

    fn key(&self) -> Option<&str> {
        Some(COMMAND)
    }

    fn confirm(&mut self, _ctx: &Context) -> Option<()> {
        if self.held.finish().is_some() {
            return Some(());
        }
        self.file_ok();
        None
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    fn json_fields(&self) -> Option<Value> {
        let list = self.list();
        let name = list.get(self.index(list.len())).map(name_of).unwrap_or_default();
        let mut fields = json!({
            "sets": self.sets, "selected": self.selected, "name": name,
            "base": self.base, "code": self.code, "into": self.into,
        });
        for (key, text) in CATEGORIES.iter().zip(&self.cats) {
            fields[*key] = json!(text);
        }
        Some(fields)
    }

    fn set_json_field(&mut self, _ctx: &Context, field: &str, value: Value) -> Result<(), String> {
        match field {
            "sets" => {
                self.sets = value;
                self.load();
            }
            "selected" => {
                let i = value.as_u64().ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
                self.pick(usize::try_from(i).unwrap_or(usize::MAX));
            }
            "name" => {
                let to = text_of(field, &value)?;
                let list = self.require_list()?;
                let from = list.get(self.index(list.len())).map(|s| name_of(s).to_owned()).ok_or("no Kinsoku set is picked")?;
                self.rename(&from, &to)?;
            }
            "base" => self.base = text_of(field, &value)?,
            "code" => self.code = text_of(field, &value)?,
            "into" => {
                let into = text_of(field, &value)?;
                if !CATEGORIES.iter().take(3).any(|k| *k == into) {
                    return Err(FieldRefusal::Rejects(&value).message(field));
                }
                self.into = into;
            }
            key => {
                let at = CATEGORIES.iter().position(|k| *k == key).ok_or_else(|| FieldRefusal::Unknown.message(field))?;
                let text = text_of(field, &value)?;
                if let Some(cat) = self.cats.get_mut(at) {
                    *cat = text;
                }
                self.commit();
            }
        }
        self.edited();
        Ok(())
    }
}

/// Runs the OK the dialog filed: the sets go to the document it opened on, also when another tab is active
/// by now; a document closed since closes the dialog and changes nothing. Called by `dialogs::host::pump`.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &Context) {
    let Some(host) = app.dialogs() else { return };
    let Some(request) = host.with(|h| h.get_mut::<KinsokuDialog>().and_then(KinsokuDialog::take_ok)) else { return };
    let answer = apply(app, &request);
    let gone = answer.is_err() && !app.session.documents().iter().any(|d| Some(d.uid) == request.doc);
    // The command ran outside the host's lock, which a dialog's own methods take.
    host.with(|h| {
        if let Some(d) = h.get_mut::<KinsokuDialog>() {
            d.answer_ok(answer);
        }
        if gone {
            h.cancel::<KinsokuDialog>(ctx);
        }
    });
}

fn apply(app: &mut VectorcraftApp, request: &Request) -> Result<(), String> {
    if app.session.active().map(|st| st.uid) != request.doc {
        let index = app.session.documents().iter().position(|d| Some(d.uid) == request.doc).ok_or("that document is no longer open")?;
        app.run("document.activate", json!({ "index": index }))?;
    }
    // A `sets` that is not a list of sets goes to the command as it is, which refuses it.
    let sets = match request.sets.as_array().filter(|a| a.iter().all(Value::is_object)) {
        Some(list) => {
            // A set the document gained while the dialog was open (a script, another command) is
            // kept, after the dialog's sets.
            let mut list = list.clone();
            if let Some(st) = app.session.active() {
                list.extend(st.doc.kinsoku_sets.iter().filter(|k| !request.opened.contains(&k.name)).map(listed));
            }
            Value::Array(list)
        }
        None => request.sets.clone(),
    };
    app.run(COMMAND, json!({ "sets": sets })).map(|_| ())
}

#[cfg(test)]
mod tests;
