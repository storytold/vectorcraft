//! Selection Tools in the interface: its submenu (Object › Plug-ins › Selection Tools) and the
//! Select by Artboards dialog (#5548), which lists the artboards with their number, name and size,
//! filters them by name, picks several, fits one in the window on double-click (#5549), and on OK
//! runs `addon.select.byArtboards` with the relation chosen; and Select by Link Status
//! ([`link_status`], #5526).

use craft::behavior::{Selection, keyed};
use craft::i18n::Folder;
use craft::i18n::matcher::Matcher;
use craft::menu::Node;
use craft::ui::buttons::Button;
use craft::ui::contract::{ActivateOn, Bind, CollectionEvent, Entry, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id, serde_fields, set_serde_field};
use craft::ui::display::{Shade, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::list::{List, ListRow};
use craft::ui::patterns::form_dialogs::{FormDialog, FormOutput};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::text::TextField;
use egui::Ui;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::menus::{Sep, cp};

pub mod link_status;

/// Select by Artboards, and its dialog's kind.
pub const COMMAND: &str = "addon.select.byArtboards";
pub const KIND: &str = "selectByArtboards";
/// The relations the dialog offers: (value, label).
const RELATIONS: [(&str, &str); 3] = [("overlap", "Overlapping"), ("inside", "Fully Inside"), ("outside", "Fully Outside")];
/// The rows the artboard list shows before it scrolls.
const LIST_ROWS: u16 = 9;

/// Object › Plug-ins › Selection Tools.
pub fn menu() -> Vec<Node> {
    let mut v = vec![cp("Select by Artboards…", COMMAND, Value::Null), Sep];
    v.extend(link_status::menu());
    v
}

/// `text` (already in the interface language) as a label of a dialog: the form puts its own colon
/// beside a control, so one the text ends with goes.
pub(crate) fn label(text: &str) -> Label {
    Label::from(text.trim_end_matches([':', '：']).trim_end().to_owned())
}

/// How a form dialog's pass ended, as the answer the host keeps: the values on OK, Cancel on any
/// other close.
pub(crate) fn finished<V>(shown: FormOutput<V>) -> Option<DialogResult<V>> {
    match (shown.accepted, shown.closed) {
        (Some(values), _) => Some(DialogResult::Ok(values)),
        (None, Some(_)) => Some(DialogResult::Cancel),
        (None, None) => None,
    }
}

/// What the dialog's controls edit; its serde names are the field names automation sets.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Fields {
    /// The artboards chosen, by index, sorted.
    pub(crate) artboards: Vec<usize>,
    /// How the objects relate to them: `overlap`, `inside` or `outside`.
    pub(crate) relation: String,
    /// The name search.
    pub(crate) search: String,
}

/// Select by Artboards: the artboards with their name and size, the choice and the relation.
pub(crate) struct ArtboardsDialog {
    pub(crate) fields: Fields,
    /// Each artboard's name and size text, read when the dialog opened.
    rows: Vec<(String, String)>,
    /// The artboard a double click asked to fit in the window, for [`pump`].
    go_to: Option<usize>,
}

/// Opens the dialog on the active artboard chosen, Overlapping.
pub(crate) fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let unit = app.session.general_unit();
    let rows: Vec<(String, String)> = app
        .session
        .active()
        .map(|st| {
            st.doc.artboards.iter().map(|a| (a.name.clone(), format!("{} × {}", unit.number(a.rect.width()), unit.format(a.rect.height())))).collect()
        })
        .unwrap_or_default();
    let fields = Fields { artboards: vec![app.view().map_or(0, |v| v.artboard)], relation: "overlap".into(), search: String::new() };
    app.dialogs().ok_or("no window")?.with(|host| host.open(ArtboardsDialog { fields, rows, go_to: None }));
    Ok(json!({ "dialog": KIND }))
}

/// The indices of `names` the search `query` finds, in the language `folder` folds for (all of them
/// for an empty query).
pub(crate) fn shown(names: &[String], query: &str, folder: &Folder) -> Vec<usize> {
    let matcher = Matcher::new(folder, query);
    names.iter().enumerate().filter(|(_, name)| matcher.score(name).is_some()).map(|(i, _)| i).collect()
}

/// Select All (`on`) or None: choose every row `shown`, or none of them. Rows the search hides
/// keep their state.
pub(crate) fn choose_shown(chosen: &mut Vec<usize>, shown: &[usize], on: bool) {
    if on {
        chosen.extend(shown.iter().copied());
        chosen.sort_unstable();
        chosen.dedup();
    } else {
        chosen.retain(|i| !shown.contains(i));
    }
}

/// The artboards chosen once the list reports `selected` of the rows `shown`: those chosen and
/// hidden by the search stay, the shown ones are the list's.
pub(crate) fn merge_chosen(chosen: &[usize], shown: &[usize], selected: &[usize]) -> Vec<usize> {
    let mut merged: Vec<usize> = chosen.iter().copied().filter(|i| !shown.contains(i)).chain(selected.iter().copied()).collect();
    merged.sort_unstable();
    merged.dedup();
    merged
}

/// The command's params for the dialog's `fields`.
fn params(fields: &Fields) -> Value {
    json!({ "artboards": fields.artboards, "relation": fields.relation })
}

impl StackDialog for ArtboardsDialog {
    type Result = Fields;

    fn id(&self) -> DialogId {
        dialog_id(KIND)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Fields>> {
        let ArtboardsDialog { fields, rows, go_to } = self;
        let can = !fields.artboards.is_empty();
        let shown = FormDialog::new(label(tl!("Select by Artboards")), Bind::bound(fields)).ok_enabled(can).show(ui, |ui, _, fields| {
            body(ui, fields, rows, go_to);
            !fields.artboards.is_empty()
        });
        finished(shown)
    }

    fn confirm(&mut self, _: &egui::Context) -> Option<Fields> {
        (!self.fields.artboards.is_empty()).then(|| self.fields.clone())
    }

    fn json_fields(&self) -> Option<Value> {
        serde_fields(&self.fields)
    }

    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        set_serde_field(&mut self.fields, field, value)
    }
}

/// The dialog's rows: the search, the artboards, Select All / None with the count, and the relation.
fn body(ui: &mut Ui, fields: &mut Fields, rows: &[(String, String)], go_to: &mut Option<usize>) {
    let names: Vec<String> = rows.iter().map(|(name, _)| name.clone()).collect();
    TextField::search(label(tl!("Search artboards")), Bind::bound(&mut fields.search)).show(ui);
    let visible = shown(&names, &fields.search, &Env::of(ui).folder());
    let on: Vec<usize> = fields.artboards.iter().copied().filter(|i| visible.contains(i)).collect();
    let lead = on.last().copied();
    let mut selection = Selection::new(on, lead, lead);
    let widest = rows.len().to_string();
    let source = keyed(&visible, |i| *i);
    let out = List::<usize>::new(label(tl!("Select by Artboards")))
        .select(SelectMode::Multi)
        .allow_empty(true)
        .activate(ActivateOn::DoubleClick)
        .empty(label(tl!("No artboards match.")))
        .max_rows(LIST_ROWS)
        .selection(Bind::bound(&mut selection))
        .show_items(ui, &source, &mut |&i, item| match rows.get(i) {
            Some((name, size)) => {
                ListRow::new(name.clone()).lead_text((i + 1).to_string(), widest.clone()).trailing(size.clone()).state(item.row.clone())
            }
            None => ListRow::new(""),
        });
    for event in out.events {
        if let CollectionEvent::Activate(i) = event {
            *go_to = Some(i);
        }
    }
    fields.artboards = merge_chosen(&fields.artboards, &visible, &selection.items);
    Row::new().show(ui, |ui, _| {
        if Button::new(label(tl!("Select All"))).show(ui).clicked() {
            choose_shown(&mut fields.artboards, &visible, true);
        }
        if Button::new(label(tl!("None"))).show(ui).clicked() {
            choose_shown(&mut fields.artboards, &visible, false);
        }
        // Rows hidden by the search stay chosen: the count says how many are.
        let count =
            crate::i18n::fmt(tl!("Chosen: {n} of {total}"), &[("n", &fields.artboards.len().to_string()), ("total", &rows.len().to_string())]);
        Text::new(Label::from(count.to_string())).shade(Shade::Muted).show(ui);
    });
    let relations: Vec<Entry<String>> = RELATIONS.iter().map(|&(value, text)| Entry::new(value.to_owned(), label(tl!(text)))).collect();
    Field::new(label(tl!("Select:"))).show(ui, |ui, _| {
        let picked = Select::new(label(tl!("Select:")), Bind::given(fields.relation.clone()), &relations).show(ui).value().cloned();
        if let Some(relation) = picked {
            fields.relation = relation;
        }
    });
}

/// Each frame: runs what the dialog asked for. A double click fits that artboard in the window
/// (`view.goToArtboard`, as a double-click on its number in the Artboards panel does) and the dialog
/// stays open; OK selects the objects, after the dialog closed.
pub(crate) fn pump(app: &mut VectorcraftApp, _: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let id = dialog_id(KIND);
    let (answer, go_to) =
        dialogs.with(|host| (host.take_by_id::<ArtboardsDialog>(id), host.get_mut_by_id::<ArtboardsDialog>(id).and_then(|d| d.go_to.take())));
    if let Some(index) = go_to {
        // An artboard the list showed a frame ago can't be missing, and a failure only leaves the view.
        app.run("view.goToArtboard", json!({ "index": index })).ok();
    }
    if let Some(DialogResult::Ok(fields)) = answer
        && let Err(e) = app.run(COMMAND, params(&fields))
    {
        app.status(e);
    }
}

#[cfg(test)]
mod tests;
