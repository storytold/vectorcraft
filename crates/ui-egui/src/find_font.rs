//! Type → Find Font…: the fonts the document uses (missing ones marked), Find (select the text
//! using a font), Change / Change All to another font from the list Replace With Font From shows
//! (the document's fonts, the recent fonts or every installed font), and Find in Folder… (the
//! missing fonts' files, in the Missing Fonts dialog: `ui.findFontsInFolder`). Find In limits Find
//! to the active artboard or the selected layers.

use std::borrow::Cow;
use std::sync::Arc;

use craft::behavior::{DisplayOrder, IconId, ItemSource, Selection};
use craft::host::PickKind;
use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::contract::{
    Bind, CollectionEvent, Entry, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Sizing, clicked,
};
use craft::ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, FieldRefusal, StackDialog, dialog_id};
use craft::ui::form::Field;
use craft::ui::layout_components::Stack;
use craft::ui::list::{List, ListRow};
use craft::ui::pickers::Select;
use craft::ui::primitives::IconSpec;
use craft::ui::sections::{DialogRole, FooterButton, HeaderStrength, Section};
use craft::ui::text::TextField;
use egui::{Context, Ui};
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::caches::{FindFontCache, FontsKey};
use crate::file_asks;

/// The key automation names the dialog by.
const KEY: &str = "findFont";

/// What a press in the dialog asks the app to do; `show` has no app, so [`pump`] runs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    /// Select the text set in the chosen document font.
    Find,
    /// Replace the chosen document font in the selected objects.
    Change,
    /// Replace the chosen document font everywhere.
    ChangeAll,
    /// Show list `SOURCES[i]` in Replace With Font.
    Source(usize),
    /// Search a folder for the missing fonts' files.
    FindInFolder,
}

/// Type › Find Font… on the dialog host: the values the user edits, and what [`pump`] pushes in each
/// frame (the document's fonts, whether Find in Folder… is offered).
#[derive(Clone, Debug, Default)]
pub(crate) struct FindFontDialog {
    /// The chosen document font, an index into `used`.
    selected: usize,
    /// The font Change replaces with: its family and style (`""` is the closest style).
    family: String,
    style: String,
    /// Whether Change is limited to the selection.
    selection_only: bool,
    /// The list Replace With Font From shows, one of [`SOURCES`].
    replace_from: String,
    /// Where Find looks, one of [`FIND_IN`].
    find_in: String,
    /// The rows of Replace With Font, a snapshot refreshed when the source changes or after a Change.
    rows: Vec<Row>,
    /// The families Replace With Font From lists (`None`: System, the font menu).
    families: Option<Vec<String>>,
    /// The replacement search; it lives and drops with the dialog.
    search: String,
    /// The active document's fonts (`text.fonts`), pushed by [`pump`].
    used: Arc<Vec<Value>>,
    /// Whether Find in Folder… shows, pushed by [`pump`].
    offer_folder: bool,
    /// The press [`pump`] runs next; one at a time, taken when run.
    pending: Option<Action>,
}

impl FindFontDialog {
    /// The chosen document font's index.
    pub(crate) fn selected(&self) -> usize {
        self.selected
    }

    /// Whether Find in Folder… is offered: a font is missing and a folder can be picked.
    pub(crate) fn offer_folder(&self) -> bool {
        self.offer_folder
    }
}

/// The document's fonts and whether Find in Folder… is offered for them.
fn current_fonts(app: &mut VectorcraftApp) -> (Arc<Vec<Value>>, bool) {
    let can_pick = file_asks::can(app, PickKind::Folder);
    document_fonts(app, can_pick)
}

/// Open the dialog.
pub fn open(app: &mut VectorcraftApp) {
    open_at(app, 0);
}

/// Open the dialog on the first font the document misses, to replace it (Missing Fonts' Find
/// Fonts).
pub fn open_on_missing(app: &mut VectorcraftApp) {
    let first = document_fonts(app, false).0.iter().position(|f| matches!(f["status"].as_str(), Some("missing" | "substitute")));
    open_at(app, first.unwrap_or(0));
}

/// Open the dialog with the document's font `selected` (an index into `text.fonts`) chosen,
/// System's font ([`system_family`]) to change to, and the last Find In choice (Document at first).
/// Before the window has a context there is no host, and nothing opens.
fn open_at(app: &mut VectorcraftApp, selected: usize) {
    let Some(dialogs) = app.dialogs() else { return };
    let find_in = FIND_IN.iter().find(|f| **f == app.ui.find_font_in).copied().unwrap_or("document");
    let (used, offer_folder) = current_fonts(app);
    let source = "system";
    let rows = replacement_rows(app, source, &used);
    let family = system_family();
    let style = rows.iter().find(|r| r.family.eq_ignore_ascii_case(&family)).map(|r| r.style.clone()).unwrap_or_default();
    let families = replacement_families(app, source, &used);
    let dialog = FindFontDialog {
        selected,
        family,
        style,
        replace_from: source.into(),
        find_in: find_in.into(),
        rows,
        families,
        used,
        offer_folder,
        ..FindFontDialog::default()
    };
    dialogs.with(|host| host.open(dialog));
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
fn can_replace(d: &FindFontDialog, families: Option<&[String]>) -> bool {
    let (family, style) = (&d.family, &d.style);
    if !style.is_empty() && !exact(family, style) {
        return false;
    }
    match families {
        Some(list) => can_replace_in(family, installed, list),
        None => can_replace_in(family, installed, &vectorcraft_text::FontDb::global().menu_family_list()),
    }
}

/// [`can_replace`] for the installed check `is_installed` and the shown list `list`.
fn can_replace_in(family: &str, is_installed: impl Fn(&str) -> bool, list: &[String]) -> bool {
    is_installed(family) && list.iter().any(|f| f.eq_ignore_ascii_case(family))
}

/// Show list `source` ([`SOURCES`]): the chosen family stays when the list has it and it is
/// installed, otherwise the list's first installed family (none when there is none) is chosen in
/// its closest style. System chooses [`system_family`]. A new list starts with an empty search.
fn set_source(app: &VectorcraftApp, d: &mut FindFontDialog, source: &str, used: &[Value]) {
    d.replace_from = source.into();
    d.search.clear();
    d.families = replacement_families(app, source, used);
    d.rows = replacement_rows(app, source, used);
    // The chosen font stays only when the new list has that family and style; otherwise the family
    // and the style both come from a row the list shows, so Change replaces with what is listed.
    if d.rows.iter().any(|r| r.family.eq_ignore_ascii_case(&d.family) && r.style.eq_ignore_ascii_case(&d.style) && listed(r)) {
        return;
    }
    let system = if source == "system" { system_family() } else { String::new() };
    let pick = d
        .rows
        .iter()
        .find(|r| !system.is_empty() && r.family.eq_ignore_ascii_case(&system) && listed(r))
        .or_else(|| d.rows.iter().find(|r| listed(r)))
        .cloned();
    (d.family, d.style) = pick.map(|r| (r.family, r.style)).unwrap_or_default();
}

/// Find In's choices (the `findIn` field): the whole document, the active artboard, or the layers
/// highlighted in the Layers panel (else the current layer).
const FIND_IN: [&str; 3] = ["document", "artboard", "layers"];
const FIND_IN_LABELS: [&str; 3] = ["Document", "Artboard", "Layers"];

/// The `scope` Find passes `select.font` for Find In choice `find_in`. A document without
/// artboards, or without a layer to look in, is looked through whole.
fn find_scope(app: &VectorcraftApp, find_in: &str) -> Value {
    let Some(st) = app.session.active() else { return json!("document") };
    let n = st.doc.artboards.len();
    match find_in {
        "artboard" if n > 0 => json!({ "artboards": [crate::panels::artboards::selected(app, n)] }),
        "layers" => {
            let is_layer = |id: &vectorcraft_doc::NodeId| st.doc.node(*id).is_some_and(vectorcraft_doc::Node::is_layer);
            let mut layers: Vec<u64> = st.highlighted_rows().into_iter().filter(is_layer).map(|id| id.0).collect();
            if layers.is_empty() {
                layers.extend(st.current_layer().map(|id| id.0));
            }
            if layers.is_empty() { json!("document") } else { json!({ "layers": layers }) }
        }
        _ => json!("document"),
    }
}

/// Run the dialog's button `button` (Find, Change or Change All) on the document font `from`.
fn run_button(app: &mut VectorcraftApp, d: &FindFontDialog, button: Action, from: &Value) -> Result<Value, String> {
    match button {
        Action::Find => {
            let scope = find_scope(app, &d.find_in);
            app.run("select.font", json!({ "family": from["family"], "style": from["style"], "scope": scope }))
        }
        Action::Change => change(app, d, from, false),
        _ => change(app, d, from, true),
    }
}

fn fonts(app: &mut VectorcraftApp) -> Vec<Value> {
    app.session.execute("text.fonts", &json!({})).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default()
}

/// The active document's fonts ([`fonts`]) and, when `missing` is asked for, whether it misses a
/// font ([`vectorcraft_engine::cmd::fontfiles::missing_fonts`]; `false` when it is not asked
/// for). Both are kept in [`crate::CanvasCache::find_font`] and worked out again only for another
/// document, another version of its content or another font generation. No document: no fonts.
fn document_fonts(app: &mut VectorcraftApp, missing: bool) -> (Arc<Vec<Value>>, bool) {
    let Some(st) = app.session.active() else { return (Arc::default(), false) };
    // The generation is read first: fonts loaded while the answers are worked out leave the entry
    // under the older generation, so the next frame works them out again.
    let key = FontsKey { uid: st.uid, doc: Arc::downgrade(&st.doc), generation: vectorcraft_text::FontDb::global().generation() };
    if app.canvas.find_font.as_ref().is_none_or(|c| c.key != key) {
        let fonts = Arc::new(fonts(app));
        app.canvas.find_font = Some(FindFontCache { key, fonts, missing: None });
    }
    if missing
        && let Some(c) = app.canvas.find_font.as_mut()
        && c.missing.is_none()
        && let Some(st) = app.session.active()
    {
        c.missing = Some(!vectorcraft_engine::cmd::fontfiles::missing_fonts(&st.doc).is_empty());
    }
    let c = app.canvas.find_font.as_ref();
    (c.map(|c| c.fonts.clone()).unwrap_or_default(), missing && c.and_then(|c| c.missing).unwrap_or(false))
}

/// Replace the selected document font with the chosen one (`all`: everywhere, else the selection).
fn change(app: &mut VectorcraftApp, d: &FindFontDialog, from: &Value, all: bool) -> Result<Value, String> {
    let to = if d.style.is_empty() { json!({ "family": d.family }) } else { json!({ "family": d.family, "style": d.style }) };
    app.run("text.replaceFont", json!({ "from": { "family": from["family"], "style": from["style"] }, "to": to, "selectionOnly": !all }))
}

/// Runs what the dialog asked for since the last pass, and pushes the document's fonts and the
/// Find in Folder… offer into it. One line of `dialogs::host::pump`.
///
/// The press is taken out of the dialog when it runs, so a second pass of the same frame finds none
/// and runs nothing twice. Nothing runs inside the host's lock: the dialog is copied out, the app
/// acts, and the copy goes back.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let (used, offer_folder) = current_fonts(app);
    let taken = dialogs.with(|host| {
        let d = host.get_mut::<FindFontDialog>()?;
        let fresh = !Arc::ptr_eq(&d.used, &used);
        d.used = used.clone();
        d.offer_folder = offer_folder;
        let action = d.pending.take();
        Some((d.clone(), action, fresh))
    });
    let Some((mut d, action, fresh)) = taken else { return };
    if fresh || action.is_some() {
        d.families = replacement_families(app, &d.replace_from, &used);
    }
    let mut folder = false;
    match action {
        Some(Action::Source(i)) => set_source(app, &mut d, SOURCES.get(i).copied().unwrap_or("system"), &used),
        Some(Action::FindInFolder) => match app.run("ui.findFontsInFolder", json!({})) {
            // Missing Fonts takes the dialog's place.
            Ok(_) => folder = true,
            Err(e) => app.ui.status = e,
        },
        Some(button) => {
            if let Some(from) = used.get(d.selected) {
                match run_button(app, &d, button, from) {
                    // The rows are a snapshot: the document's fonts and the recent fonts have moved on.
                    Ok(_) if button != Action::Find => {
                        let (now, _) = document_fonts(app, false);
                        d.rows = replacement_rows(app, &d.replace_from, &now);
                        d.families = replacement_families(app, &d.replace_from, &now);
                    }
                    Ok(_) => {}
                    Err(e) => app.ui.status = e,
                }
            }
        }
        None => {}
    }
    app.ui.find_font_in = d.find_in.clone();
    dialogs.with(|host| {
        if folder {
            host.cancel::<FindFontDialog>(ctx);
        } else if let Some(open) = host.get_mut::<FindFontDialog>() {
            *open = d;
        }
    });
}

/// The rows of a list as a collection source: `text(at)` is worked out for the rows the list asks for.
struct Lines<'a> {
    len: usize,
    text: &'a dyn Fn(usize) -> String,
}

impl DisplayOrder<usize> for Lines<'_> {
    fn len(&self) -> usize {
        self.len
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.len).then_some(*key)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        (at < self.len).then(|| Cow::Owned((self.text)(at)))
    }
}

impl ItemSource<usize> for Lines<'_> {}

/// The footer's buttons in the order [`StackDialog::show`] reads `picked` in.
const FOOTER_ACTIONS: [Action; 3] = [Action::Find, Action::Change, Action::ChangeAll];

/// A row of the document's fonts: the font and its runs, with the note and its icon when the font
/// is missing, substituted or lacks characters.
fn document_row(f: &Value) -> ListRow<'static> {
    let (note, tone) = font_note(f);
    let row = ListRow::new(format!("{} {} ({})", f["family"].as_str().unwrap_or(""), f["style"].as_str().unwrap_or(""), f["runs"]));
    let detail = note.trim().trim_start_matches('—').trim();
    if detail.is_empty() { row } else { row.detail(detail.to_string()).icon(IconSpec::svg(IconId::new("warning"))).tone(tone) }
}

impl StackDialog for FindFontDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        dialog_id(KEY)
    }

    fn key(&self) -> Option<&str> {
        Some(KEY)
    }

    fn show(&mut self, ui: &mut Ui, _stack: &DialogStack) -> Option<DialogResult<()>> {
        let count = self.used.len();
        let has = self.selected < count;
        // An empty list (Recent before any font was applied) leaves nothing to change to.
        let replace = has && can_replace(self, self.families.as_deref());
        let query = self.search.to_lowercase();
        // Rows of Replace With Font the search keeps, by index into `rows`.
        let shown: Vec<usize> = self.rows.iter().enumerate().filter(|(_, r)| row_matches(&r.family, &r.style, &query)).map(|(i, _)| i).collect();
        let chosen = shown.iter().position(|&i| {
            self.rows.get(i).is_some_and(|r| r.family.eq_ignore_ascii_case(&self.family) && r.style.eq_ignore_ascii_case(&self.style))
        });
        let mut doc_selection = Selection::single(self.selected);
        let mut row_selection = chosen.map_or_else(Selection::default, Selection::single);
        let source = SOURCES.iter().position(|s| *s == self.replace_from).unwrap_or(2);
        let find_in = FIND_IN.iter().position(|s| *s == self.find_in).unwrap_or(0);
        let entries =
            |labels: &[&str]| -> Vec<Entry<usize>> { labels.iter().enumerate().map(|(i, l)| Entry::new(i, crate::i18n::t(l).to_string())).collect() };
        let (source_entries, find_in_entries) = (entries(&SOURCE_LABELS), entries(&FIND_IN_LABELS));
        let heading = crate::i18n::fmt(tl!("Fonts in Document: {count}"), &[("count", &count.to_string())]);
        let buttons = [
            FooterButton::new(tl!("Find").to_string(), DialogRole::Other).stays_open(true).enabled(has),
            FooterButton::new(tl!("Change").to_string(), DialogRole::Other)
                .stays_open(true)
                .enabled(replace)
                .tooltip(tl!("In the selected objects").to_string()),
            FooterButton::new(tl!("Change All").to_string(), DialogRole::Other).stays_open(true).enabled(replace),
            FooterButton::new(tl!("Done").to_string(), DialogRole::Accept),
        ];

        let mut folder = false;
        let mut new_source = None;
        let mut new_find_in = None;
        let mut new_search = None;
        let mut activated = None;
        let used = self.used.clone();
        let (rows, offer_folder) = (&self.rows, self.offer_folder);
        let doc_text = |at: usize| {
            used.get(at)
                .map_or_else(String::new, |f| format!("{} {} ({})", f["family"].as_str().unwrap_or(""), f["style"].as_str().unwrap_or(""), f["runs"]))
        };
        let row_text = |at: usize| shown.get(at).and_then(|&i| rows.get(i)).map_or_else(String::new, |r| row_label(&r.family, &r.style));
        let out = Dialog::new(tl!("Find Font").to_string()).buttons(buttons).show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                Section::fixed(heading.clone()).strength(HeaderStrength::Caption).show(ui, |ui, _| {
                    let docs = Lines { len: count, text: &doc_text };
                    List::<usize>::new(tl!("Fonts in Document").to_string())
                        .select(SelectMode::Single)
                        .sizing(Sizing::Content)
                        .max_rows(8)
                        .selection(Bind::bound(&mut doc_selection))
                        .show_items(ui, &docs, &mut |key, _| used.get(*key).map_or_else(|| ListRow::new(String::new()), document_row));
                });
                // The missing fonts' files, looked for in a folder (the Missing Fonts dialog).
                if offer_folder {
                    folder = clicked(&Button::new(tl!("Find in Folder…").to_string()).show(ui));
                }
                Section::fixed(tl!("Replace With Font").to_string()).strength(HeaderStrength::Caption).show(ui, |ui, _| {
                    Field::new(tl!("Replace With Font From").to_string()).show(ui, |ui, _| {
                        let edit = Select::new(tl!("Replace With Font From").to_string(), Bind::given(source), &source_entries).show(ui);
                        new_source = edit.events.value().copied();
                    });
                    let edit = TextField::search(tl!("Search").to_string(), Bind::given(self.search.clone())).show(ui);
                    new_search = edit.events.value().cloned();
                    let lines = Lines { len: shown.len(), text: &row_text };
                    let list = List::<usize>::new(tl!("Replace With Font").to_string())
                        .select(SelectMode::Single)
                        .sizing(Sizing::Content)
                        .max_rows(10)
                        .selection(Bind::bound(&mut row_selection))
                        .show_items(ui, &lines, &mut |key, _| ListRow::new(row_text(*key)));
                    activated = list.events.iter().find_map(|e| if let CollectionEvent::Activate(key) = e { Some(*key) } else { None });
                });
                Field::new(tl!("Find In").to_string()).show(ui, |ui, _| {
                    let edit = Select::new(tl!("Find In").to_string(), Bind::given(find_in), &find_in_entries).show(ui);
                    new_find_in = edit.events.value().copied();
                });
            });
        });

        if let Some(i) = doc_selection.lead {
            self.selected = i;
        }
        if let Some(text) = new_search {
            self.search = text;
        }
        if let Some(i) = new_find_in.and_then(|i| FIND_IN.get(i)) {
            self.find_in = (*i).into();
        }
        // A click on a row picks its family and style; a double click also asks for Change.
        let picked = activated.or(row_selection.lead.filter(|at| Some(*at) != chosen));
        if let Some(row) = picked.and_then(|at| shown.get(at)).and_then(|&i| self.rows.get(i)).cloned() {
            (self.family, self.style) = (row.family, row.style);
            if activated.is_some() && replace_after_pick(self) {
                self.pending = Some(Action::Change);
            }
        }
        if let Some(i) = new_source {
            self.pending = Some(Action::Source(i));
        }
        if folder {
            self.pending = Some(Action::FindInFolder);
        }
        if let Some(action) = out.output.picked.and_then(|i| FOOTER_ACTIONS.get(i)) {
            self.pending = Some(*action);
        }
        out.output.result(|| ())
    }

    fn confirm(&mut self, _ctx: &Context) -> Option<()> {
        Some(())
    }

    fn json_fields(&self) -> Option<Value> {
        Some(json!({
            "selected": self.selected,
            "family": self.family,
            "style": self.style,
            "selectionOnly": self.selection_only,
            "replaceFrom": self.replace_from,
            "findIn": self.find_in,
            "rows": rows_json(&self.rows),
        }))
    }

    fn set_json_field(&mut self, _ctx: &Context, field: &str, value: Value) -> Result<(), String> {
        let refuse = || FieldRefusal::Rejects(&value).message(field);
        match field {
            "selected" => self.selected = value.as_u64().map(|i| i as usize).filter(|i| *i < self.used.len()).ok_or_else(refuse)?,
            "family" => self.family = value.as_str().ok_or_else(refuse)?.to_string(),
            "style" => self.style = value.as_str().ok_or_else(refuse)?.to_string(),
            "selectionOnly" => self.selection_only = value.as_bool().ok_or_else(refuse)?,
            "findIn" => self.find_in = value.as_str().and_then(|v| FIND_IN.iter().find(|f| **f == v)).ok_or_else(refuse)?.to_string(),
            // The rows follow the source, which needs the app: the pump changes it.
            "replaceFrom" => {
                self.pending = Some(Action::Source(value.as_str().and_then(|v| SOURCES.iter().position(|s| *s == v)).ok_or_else(refuse)?));
            }
            "rows" => return Err(refuse()),
            _ => return Err(FieldRefusal::Unknown.message(field)),
        }
        Ok(())
    }
}

/// Does a double click on the row just picked ask for Change: the chosen font can replace.
fn replace_after_pick(d: &FindFontDialog) -> bool {
    d.selected < d.used.len() && can_replace(d, d.families.as_deref())
}

/// What a Find Font row says after the font's name (`text.fonts` row): a missing family, a style
/// standing in for another, characters the font lacks; and the tone to flag it with.
pub(crate) fn font_note(f: &Value) -> (String, Tone) {
    let lacking = f["missingGlyphs"].as_u64().unwrap_or(0);
    let glyphs =
        if lacking > 0 { crate::i18n::fmt(tl!("  — {n} characters from another font"), &[("n", &lacking.to_string())]) } else { String::new() };
    match f["status"].as_str() {
        Some("missing") => (format!("{}{glyphs}", tl!("  — missing")), Tone::Bad),
        Some("substitute") => {
            let used = f["resolved"]["style"].as_str().unwrap_or("");
            (format!("{}{glyphs}", crate::i18n::fmt(tl!("  — substituted by {style}"), &[("style", used)])), Tone::Warn)
        }
        _ if lacking > 0 => (glyphs, Tone::Warn),
        _ => (String::new(), Tone::Neutral),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An app whose window has a dialog host (it has a context).
    fn new_app() -> VectorcraftApp {
        crate::testing::windowed(VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default()))
    }

    /// Runs `f` on the open Find Font dialog.
    fn edit<R>(app: &VectorcraftApp, f: impl FnOnce(&mut FindFontDialog) -> R) -> R {
        let dialogs = app.dialogs().expect("the window has a dialog host");
        dialogs.with(|host| f(host.get_mut::<FindFontDialog>().expect("Find Font is open")))
    }

    /// A copy of the open Find Font dialog.
    fn opened(app: &VectorcraftApp) -> FindFontDialog {
        edit(app, |d| d.clone())
    }

    /// One pass of [`pump`].
    fn pump_now(app: &mut VectorcraftApp) {
        let ctx = app.ui_ctx.clone().expect("the window has a context");
        pump(app, &ctx);
    }

    /// Find In passes Find the active artboard, or the selected layers (else the current
    /// layer), as `select.font`'s scope. The choice is set as automation sets it, and Find runs
    /// from the pump.
    #[test]
    fn find_in_limits_find_to_the_artboard_or_the_layers() {
        let mut app = new_app();
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("artboard.new", &json!({"x": 200, "y": 0, "width": 100, "height": 100})).unwrap();
        let text = |app: &mut VectorcraftApp, x: i32| {
            app.session.execute("text.create", &json!({"x": x, "y": 40, "text": "Hi", "font": "Inter"})).unwrap()["id"].as_u64().unwrap()
        };
        let first = text(&mut app, 10);
        let layer = app.session.execute("layer.new", &json!({})).unwrap()["id"].as_u64().unwrap();
        let second = text(&mut app, 210);
        let third = text(&mut app, 20);
        // Choose Find In and the Inter row on the open dialog, press Find, and let the pump run it.
        let find = |app: &mut VectorcraftApp, find_in: &str| {
            open(app);
            let inter = fonts(app).iter().position(|f| f["family"] == "Inter").unwrap();
            let ctx = app.ui_ctx.clone().unwrap();
            edit(app, |d| {
                d.set_json_field(&ctx, "findIn", json!(find_in)).unwrap();
                d.selected = inter;
                d.pending = Some(Action::Find);
            });
            app.session.active_mut().unwrap().selection.objects.clear();
            pump_now(app);
            let mut ids: Vec<u64> = app.session.active().unwrap().selection.objects.iter().map(|id| id.0).collect();
            ids.sort_unstable();
            // A second pass of the frame finds the press taken: nothing runs twice.
            app.session.active_mut().unwrap().selection.objects.clear();
            pump_now(app);
            assert!(app.session.active().unwrap().selection.objects.is_empty(), "the press ran once");
            ids
        };
        open(&mut app);
        assert_eq!(opened(&app).find_in, "document");
        assert_eq!(find(&mut app, "document"), [first, second, third]);
        // Without a view, the active artboard is the first.
        assert_eq!(find(&mut app, "artboard"), [first, third]);
        assert_eq!(find_scope(&app, "layers"), json!({ "layers": [layer] }));
        assert_eq!(find(&mut app, "layers"), [second, third]);
        // Opening the dialog again keeps the last choice.
        open(&mut app);
        assert_eq!(opened(&app).find_in, "layers");
    }

    /// Change All leaves locked text in its font, and the status bar says so.
    #[test]
    fn change_all_clears_the_skip_warning_after_unlocking_text() {
        let mut app = new_app();
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Hi", "font": "Missing Family"})).unwrap();
        app.session.execute("object.lock", &json!({})).unwrap();
        open(&mut app);
        let d = opened(&app);
        let from = fonts(&mut app)[0].clone();
        let r = change(&mut app, &d, &from, true).unwrap();
        assert_eq!(r["runs"], 0, "{r}");
        assert_eq!(fonts(&mut app)[0]["family"], "Missing Family");
        assert_eq!(app.ui.status, "some text is hidden or locked, so its font wasn't replaced");
        app.run("object.unlockAll", json!({})).unwrap();
        assert_eq!(app.ui.status, "", "unlocking the skipped text clears the warning");
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
    fn replacement_keeps_only_font_warnings_that_still_apply() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Hi", "font": "Missing Family"})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 70, "text": "Bye", "font": "Missing Family"})).unwrap();
        app.session.execute("object.lock", &json!({})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 90, "text": "Other", "font": "Inter"})).unwrap();

        let replace = |app: &mut VectorcraftApp, from: &str, from_style: Option<&str>, style: &str| {
            app.run("text.replaceFont", json!({"from": {"family": from, "style": from_style}, "to": {"family": "Source Sans 3", "style": style}}))
                .unwrap()
        };
        let first = replace(&mut app, "Missing Family", Some("Regular"), "Black Wide");
        assert_eq!(first["runs"], 1, "{first}");
        assert_eq!(first["skipped"], 1, "{first}");
        let style_warning = first["warning"].as_str().unwrap().split_once("; ").unwrap().1.to_string();
        assert!(style_warning.contains("Source Sans 3 Black Wide is drawn in Source Sans 3"), "{style_warning}");

        let unrelated = replace(&mut app, "Inter", None, "Regular");
        assert_eq!(unrelated["runs"], 1, "{unrelated}");
        assert!(unrelated.get("warning").is_none(), "{unrelated}");
        assert_eq!(app.ui.status, first["warning"].as_str().unwrap());

        app.run("object.unlockAll", json!({})).unwrap();
        let second = replace(&mut app, "Missing Family", None, "Regular");
        assert_eq!(second["runs"], 1, "{second}");
        assert!(second.get("warning").is_none(), "{second}");
        assert_eq!(app.ui.status, style_warning);

        let third = replace(&mut app, "Source Sans 3", Some("Black Wide"), "Regular");
        assert_eq!(third["runs"], 1, "{third}");
        assert!(third.get("warning").is_none(), "{third}");
        assert_eq!(app.ui.status, "");
    }

    #[test]
    fn a_style_specific_retry_clears_a_family_wide_skipped_warning() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Hi", "font": "Missing Family"})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 70, "text": "Bye", "font": "Missing Family"})).unwrap();
        app.session.execute("object.lock", &json!({})).unwrap();
        let first = app
            .run("text.replaceFont", json!({"from": {"family": "Missing Family"}, "to": {"family": "Source Sans 3", "style": "Regular"}}))
            .unwrap();
        assert_eq!((first["runs"].as_u64(), first["skipped"].as_u64()), (Some(1), Some(1)));
        app.run("object.unlockAll", json!({})).unwrap();
        let second = app
            .run(
                "text.replaceFont",
                json!({"from": {"family": "Missing Family", "style": "Regular"}, "to": {"family": "Source Sans 3", "style": "Regular"}}),
            )
            .unwrap();
        assert_eq!(second["runs"], 1, "{second}");
        assert!(second.get("warning").is_none(), "{second}");
        assert_eq!(app.ui.status, "");
    }

    #[test]
    fn new_text_in_the_source_font_does_not_keep_an_old_skip_warning() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let replace = |app: &mut VectorcraftApp, from: &str, style: &str| {
            app.run("text.replaceFont", json!({"from": {"family": from}, "to": {"family": "Source Sans 3", "style": style}})).unwrap()
        };
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 30, "text": "Editable", "font": "Missing Family"})).unwrap();
        let skipped = app.session.execute("text.create", &json!({"x": 10, "y": 60, "text": "Locked", "font": "Missing Family"})).unwrap()["id"]
            .as_u64()
            .unwrap();
        app.session.execute("object.lock", &json!({})).unwrap();
        let first = replace(&mut app, "Missing Family", "Regular");
        assert_eq!((first["runs"].as_u64(), first["skipped"].as_u64()), (Some(1), Some(1)));
        app.session.execute("text.create", &json!({"x": 10, "y": 90, "text": "New", "font": "Missing Family"})).unwrap();
        app.run("object.unlockAll", json!({})).unwrap();
        app.session.active_mut().unwrap().selection.objects = vec![vectorcraft_doc::NodeId(skipped)];
        let second = app
            .run("text.replaceFont", json!({"from": {"family": "Missing Family"}, "to": {"family": "Source Sans 3"}, "selectionOnly": true}))
            .unwrap();
        assert_eq!(second["runs"], 1, "{second}");
        assert!(second.get("warning").is_none(), "{second}");
        assert_eq!(app.ui.status, "");
        assert!(fonts(&mut app).iter().any(|font| font["family"] == "Missing Family"));
    }

    #[test]
    fn unlocking_skipped_text_clears_its_warning() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 30, "text": "Other", "font": "Inter"})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 60, "text": "Locked", "font": "Missing Family"})).unwrap();
        app.session.execute("object.lock", &json!({})).unwrap();
        let first = app.run("text.replaceFont", json!({"from": {"family": "Missing Family"}, "to": {"family": "Source Sans 3"}})).unwrap();
        assert_eq!(first["skipped"], 1);
        app.run("object.unlockAll", json!({})).unwrap();
        assert_eq!(app.ui.status, "");
        let unrelated = app.run("text.replaceFont", json!({"from": {"family": "Inter"}, "to": {"family": "Source Sans 3"}})).unwrap();
        assert_eq!(unrelated["runs"], 1);
        assert_eq!(app.ui.status, "");
    }

    #[test]
    fn a_document_revision_refreshes_the_skip_warning_on_tick() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 60, "text": "Locked", "font": "Missing Family"})).unwrap();
        app.session.execute("object.lock", &json!({})).unwrap();
        let first = app.run("text.replaceFont", json!({"from": {"family": "Missing Family"}, "to": {"family": "Source Sans 3"}})).unwrap();
        assert_eq!(first["skipped"], 1);
        assert!(!app.ui.status.is_empty());

        app.session.execute("object.unlockAll", &json!({})).unwrap();
        let ctx = crate::testing::ui_context();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| app.tick(ui.ctx()));
        out.textures_delta.clear();
        assert_eq!(app.ui.status, "");
    }

    #[test]
    fn a_selection_only_retry_ignores_unselected_text_in_the_same_font() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Outside", "font": "Missing Family"})).unwrap();
        let selected = app.session.execute("text.create", &json!({"x": 10, "y": 70, "text": "Selected", "font": "Missing Family"})).unwrap()["id"]
            .as_u64()
            .unwrap();
        app.session.execute("object.lock", &json!({})).unwrap();
        app.session.active_mut().unwrap().selection.objects = vec![vectorcraft_doc::NodeId(selected)];
        let params = json!({"from": {"family": "Missing Family"}, "to": {"family": "Source Sans 3"}, "selectionOnly": true});
        let first = app.run("text.replaceFont", params.clone()).unwrap();
        assert_eq!((first["runs"].as_u64(), first["skipped"].as_u64()), (Some(0), Some(1)));
        app.run("object.unlockAll", json!({})).unwrap();
        app.session.active_mut().unwrap().selection.objects = vec![vectorcraft_doc::NodeId(selected)];
        let second = app.run("text.replaceFont", params).unwrap();
        assert_eq!(second["runs"], 1, "{second}");
        assert!(second.get("warning").is_none(), "{second}");
        assert_eq!(app.ui.status, "");
        assert!(app.session.execute("text.fonts", &json!({})).unwrap().as_array().unwrap().iter().any(|font| font["family"] == "Missing Family"));
    }

    #[test]
    fn a_changed_selection_keeps_the_original_warning_scope() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 30, "text": "Outside", "font": "Missing Family"})).unwrap();
        let added = app.session.execute("text.create", &json!({"x": 10, "y": 60, "text": "Added", "font": "Missing Family"})).unwrap()["id"]
            .as_u64()
            .unwrap();
        let skipped = app.session.execute("text.create", &json!({"x": 10, "y": 90, "text": "Locked", "font": "Missing Family"})).unwrap()["id"]
            .as_u64()
            .unwrap();
        app.session.execute("object.lock", &json!({})).unwrap();
        app.session.active_mut().unwrap().selection.objects = vec![vectorcraft_doc::NodeId(skipped)];
        let params = json!({"from": {"family": "Missing Family"}, "to": {"family": "Source Sans 3"}, "selectionOnly": true});
        let first = app.run("text.replaceFont", params.clone()).unwrap();
        assert_eq!(first["skipped"], 1, "{first}");
        app.run("object.unlockAll", json!({})).unwrap();
        app.session.active_mut().unwrap().selection.objects = vec![vectorcraft_doc::NodeId(skipped), vectorcraft_doc::NodeId(added)];
        let second = app.run("text.replaceFont", params).unwrap();
        assert_eq!(second["runs"], 2, "{second}");
        assert!(second.get("warning").is_none(), "{second}");
        assert_eq!(app.ui.status, "");
        assert!(app.session.execute("text.fonts", &json!({})).unwrap().as_array().unwrap().iter().any(|font| font["family"] == "Missing Family"));
    }

    #[test]
    fn changing_selection_does_not_clear_a_skipped_font_warning() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let selected = app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Locked", "font": "Missing Family"})).unwrap()["id"]
            .as_u64()
            .unwrap();
        app.session.execute("object.lock", &json!({})).unwrap();
        app.session.active_mut().unwrap().selection.objects = vec![vectorcraft_doc::NodeId(selected)];
        let first = app
            .run("text.replaceFont", json!({"from": {"family": "Missing Family"}, "to": {"family": "Source Sans 3"}, "selectionOnly": true}))
            .unwrap();
        assert_eq!(first["skipped"], 1, "{first}");
        let warning = app.ui.status.clone();
        app.session.execute("text.create", &json!({"x": 10, "y": 70, "text": "Other", "font": "Inter"})).unwrap();
        let unrelated =
            app.run("text.replaceFont", json!({"from": {"family": "Inter"}, "to": {"family": "Source Sans 3"}, "selectionOnly": true})).unwrap();
        assert_eq!(unrelated["runs"], 1, "{unrelated}");
        assert!(unrelated.get("warning").is_none(), "{unrelated}");
        assert_eq!(app.ui.status, warning);
        app.run("object.unlockAll", json!({})).unwrap();
        let retry = app.run("text.replaceFont", json!({"from": {"family": "Missing Family"}, "to": {"family": "Source Sans 3"}})).unwrap();
        assert_eq!(retry["runs"], 1, "{retry}");
        assert!(retry.get("warning").is_none(), "{retry}");
        assert_eq!(app.ui.status, "");
    }

    #[test]
    fn replacing_a_font_in_another_document_keeps_the_original_warning() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Locked", "font": "Missing Family"})).unwrap();
        app.session.execute("object.lock", &json!({})).unwrap();
        let first = app.run("text.replaceFont", json!({"from": {"family": "Missing Family"}, "to": {"family": "Source Sans 3"}})).unwrap();
        assert_eq!(first["skipped"], 1, "{first}");
        let warning = app.ui.status.clone();
        let original_uid = app.session.active().unwrap().uid;

        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Other", "font": "Inter"})).unwrap();
        let second = app.run("text.replaceFont", json!({"from": {"family": "Inter"}, "to": {"family": "Source Sans 3"}})).unwrap();
        assert_eq!(second["runs"], 1, "{second}");
        assert_eq!(app.ui.status, warning);
        app.session.set_active(0);
        assert_eq!(app.session.active().unwrap().uid, original_uid);
        assert_eq!(app.ui.status, warning);
    }

    #[test]
    fn a_comma_in_a_substituted_style_keeps_its_warning() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Hi", "font": "Missing Family"})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 70, "text": "Other", "font": "Inter"})).unwrap();
        let first = app
            .run("text.replaceFont", json!({"from": {"family": "Missing Family"}, "to": {"family": "Source Sans 3", "style": "Black, Wide"}}))
            .unwrap();
        let warning = first["warning"].as_str().unwrap().to_string();
        assert!(warning.contains("Black, Wide"), "{warning}");
        let unrelated = app.run("text.replaceFont", json!({"from": {"family": "Inter"}, "to": {"family": "Source Serif 4"}})).unwrap();
        assert_eq!(unrelated["runs"], 1, "{unrelated}");
        assert!(unrelated.get("warning").is_none(), "{unrelated}");
        assert_eq!(app.ui.status, warning);
    }

    #[test]
    fn an_unrelated_substituted_style_does_not_keep_a_resolved_warning() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Hi", "font": "Missing Family"})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 70, "text": "Other", "font": "Inter"})).unwrap();
        app.run("text.replaceFont", json!({"from": {"family": "Inter"}, "to": {"family": "Source Sans 3", "style": "Narrow"}})).unwrap();
        let first = app
            .run("text.replaceFont", json!({"from": {"family": "Missing Family"}, "to": {"family": "Source Sans 3", "style": "Black, Wide"}}))
            .unwrap();
        assert!(first["warning"].as_str().is_some_and(|warning| warning.contains("Black, Wide")), "{first}");
        let resolved = app
            .run(
                "text.replaceFont",
                json!({"from": {"family": "Source Sans 3", "style": "Black, Wide"}, "to": {"family": "Inter", "style": "Regular"}}),
            )
            .unwrap();
        assert!(resolved.get("warning").is_none(), "{resolved}");
        assert_eq!(app.ui.status, "");
    }

    #[test]
    fn find_in_folder_puts_the_missing_fonts_dialog_in_find_fonts_place() {
        let dir = std::fs::canonicalize(vectorcraft_testkit::temp_dir("find-font-folder")).unwrap();
        let mut app = new_app();
        app.session.search_rules = Some(vectorcraft_engine::cmd::findfiles::Rules { user_content: vec![dir.clone()], ..Default::default() });
        app.session.search_threads = Some(1);
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let picked = dir.to_string_lossy().into_owned();
        // PLAN: fl.A20's `scripted` and `poll` live in files-w7's test module; `tests::picks` is its guessed path
        let picker = crate::tests::picks::scripted(&mut app);
        // Nothing missing: no button, and no folder is asked for.
        open(&mut app);
        pump_now(&mut app);
        assert!(!opened(&app).offer_folder());
        assert_eq!(app.run("ui.findFontsInFolder", json!({})).err().as_deref(), Some("No fonts are missing in this document"));
        app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Hi", "font": "Missing Family"})).unwrap();
        pump_now(&mut app);
        assert!(opened(&app).offer_folder(), "a missing font and a folder picker");
        // Find in Folder… asks for the folder, and Missing Fonts takes Find Font's place.
        edit(&app, |d| d.pending = Some(Action::FindInFolder));
        pump_now(&mut app);
        picker.answer(&[&picked]);
        let ctx = app.ui_ctx.clone().unwrap();
        crate::tests::picks::poll(&mut app, &ctx);
        assert!(!app.dialogs().unwrap().with(|host| host.is_open::<FindFontDialog>()), "Find Font is gone");
        // PLAN: files-w11 owns Missing Fonts' hosted type; its test asserts the search there
    }

    /// `f`'s answer, read while the font menu held still: tests running alongside add fonts to
    /// the shared database and lend it composite fonts.
    fn while_fonts_hold<T>(mut f: impl FnMut() -> T) -> T {
        let db = vectorcraft_text::FontDb::global();
        loop {
            let before = db.menu_family_list();
            let out = f();
            if before == db.menu_family_list() {
                return out;
            }
        }
    }

    /// A document set in Inter and Source Serif 4 (Inter twice), with Inter the one recent font.
    fn two_font_document() -> VectorcraftApp {
        let mut app = new_app();
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
        assert_eq!(opened(&app).replace_from, "system");
        for source in SOURCES {
            let mut d = opened(&app);
            set_source(&app, &mut d, source, &used);
            let listed = replacement_families(&app, source, &used);
            assert_eq!(d.families, listed, "{source}: the dialog keeps the list it shows");
            assert!(can_replace(&d, listed.as_deref()), "{source}: {}", d.family);
        }
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
        let mut app = new_app();
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
        let mut d = opened(&app);
        set_source(&app, &mut d, "document", &used);
        assert_eq!((d.family.as_str(), d.style.as_str()), ("Inter", "Regular"), "the first font whose style it has");
        (d.family, d.style) = ("Source Sans 3".into(), "Black Wide".into());
        assert!(!can_replace(&d, replacement_families(&app, "document", &used).as_deref()), "Change stays off for the stand-in style");
    }

    /// A new source picks the family and the style from a row its list shows, so Change replaces with
    /// what is listed rather than a closest style of its own choosing.
    #[test]
    fn a_new_source_picks_a_row_its_list_shows() {
        let mut app = two_font_document();
        let used = fonts(&mut app);
        let mut d = FindFontDialog { family: "Inter".into(), replace_from: "system".into(), ..FindFontDialog::default() };
        set_source(&app, &mut d, "document", &used);
        assert_eq!((d.family.as_str(), d.style.as_str()), ("Inter", "Regular"), "the document's own row");
        (d.family, d.style) = ("Inter".into(), "Black Wide".into());
        set_source(&app, &mut d, "document", &used);
        assert_eq!((d.family.as_str(), d.style.as_str()), ("Inter", "Regular"), "a pair the list lacks gives way");
    }

    /// The rows follow a replacement: Change All in the Document source leaves the list showing the
    /// document's fonts as they are now.
    #[test]
    fn the_rows_refresh_after_a_change() {
        let mut app = two_font_document();
        open(&mut app);
        let used = fonts(&mut app);
        let inter = used.iter().position(|f| f["family"] == "Inter").unwrap();
        // Source set through automation, then the pump; the press itself comes from the footer.
        let ctx = app.ui_ctx.clone().unwrap();
        edit(&app, |d| d.set_json_field(&ctx, "replaceFrom", json!("document")).unwrap());
        pump_now(&mut app);
        edit(&app, |d| {
            (d.family, d.style) = ("Source Serif 4".into(), String::new());
            d.selected = inter;
            d.pending = Some(Action::ChangeAll);
        });
        pump_now(&mut app);
        assert!(fonts(&mut app).iter().all(|f| f["family"] == "Source Serif 4"), "the document changed");
        let d = opened(&app);
        let now = fonts(&mut app);
        assert_eq!(d.rows, replacement_rows(&app, "document", &now), "the rows follow the document");
        assert!(d.rows.iter().all(|r| r.family == "Source Serif 4"), "{:?}", d.rows);
        assert!(d.pending.is_none(), "the press was taken");
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
        let (system, menu) = while_fonts_hold(|| (replacement_rows(&app, "system", &used), db.menu_family_list()));
        assert!(system.iter().any(|r| r.family == "Source Sans 3" && r.style == "Regular"), "{system:?}");
        assert!(!system.iter().any(|r| r.family.starts_with('.')), "the font menu's hidden families stay out");
        // A composite font is a family without styles of its own, listed by its name alone.
        assert!(system.iter().all(|r| !r.style.is_empty() || db.styles(&r.family).is_empty()), "no nameless rows: {system:?}");
        let listed: std::collections::BTreeSet<&str> = system.iter().map(|r| r.family.as_str()).collect();
        assert_eq!(listed.len(), menu.len(), "every family the font menu shows is listed");
        let mut unique = system.clone();
        unique.dedup();
        assert_eq!(unique.len(), system.len(), "no family and style is listed twice");
        // The dialog stores the rows when it opens and when the source changes, and a picked row
        // sets both fields Change then uses.
        let (rows, system) = while_fonts_hold(|| {
            open(&mut app);
            (opened(&app).rows, replacement_rows(&app, "system", &used))
        });
        assert_eq!(rows.len(), system.len());
        let mut d = opened(&app);
        set_source(&app, &mut d, "document", &used);
        assert_eq!(d.rows.len(), document.len());
        (d.family, d.style) = ("Inter".into(), "Regular".into());
        let from = used.iter().find(|f| f["family"] == "Source Serif 4").unwrap().clone();
        change(&mut app, &d, &from, true).unwrap();
        assert!(fonts(&mut app).iter().all(|f| f["family"] == "Inter"));
    }

    #[test]
    fn change_all_replaces_with_a_font_from_the_document() {
        let mut app = two_font_document();
        open(&mut app);
        let used = fonts(&mut app);
        let mut d = opened(&app);
        set_source(&app, &mut d, "document", &used);
        d.family = "Inter".into();
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
        let mut d = opened(&app);
        d.family = "Source Serif 4".into();
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
        let mut d = opened(&app);
        d.family = "Source Sans 3".into();
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
        let mut d = opened(&app);
        d.family = "SourceSerif4-Regular".into();
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
        let mut d = opened(&app);
        set_source(&app, &mut d, "recent", &used);
        assert_eq!(replacement_families(&app, "recent", &used).unwrap(), Vec::<String>::new());
        assert_eq!(d.family, "");
        assert!(!can_replace(&d, Some(&[])));
        assert_eq!(app.ui.status, "");
    }

    /// Document lists a missing family, but Change and Change All stay off for it, in every list.
    #[test]
    fn a_missing_document_font_is_listed_but_not_offered_to_change_to() {
        let mut app = new_app();
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        for (y, font) in [(20, "Missing Family"), (40, "Inter")] {
            app.session.execute("text.create", &json!({"x": 10, "y": y, "text": "Hi", "font": font})).unwrap();
        }
        open(&mut app);
        let used = fonts(&mut app);
        let document = replacement_families(&app, "document", &used).unwrap();
        assert!(document.iter().any(|f| f == "Missing Family"), "{document:?}");
        let mut d = opened(&app);
        d.family = "Missing Family".into();
        set_source(&app, &mut d, "document", &used);
        assert_eq!(d.family, "Inter", "the first installed family is chosen");
        assert!(can_replace(&d, Some(&document)));
        d.family = "Missing Family".into();
        assert!(!can_replace(&d, Some(&document)));
        assert!(!can_replace(&d, None), "System");
        set_source(&app, &mut d, "system", &used);
        assert!(can_replace(&d, None), "System chooses an installed family: {}", d.family);
        // A document set only in a missing family leaves nothing to change to.
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 20, "text": "Hi", "font": "Missing Family"})).unwrap();
        let used = fonts(&mut app);
        set_source(&app, &mut d, "document", &used);
        assert_eq!(d.family, "");
        assert!(!can_replace(&d, replacement_families(&app, "document", &used).as_deref()));
    }

    /// An empty Recent list clears the font; System then chooses an installed one again.
    #[test]
    fn system_after_an_empty_recent_list_chooses_an_installed_font() {
        let mut app = two_font_document();
        app.ui.recent_fonts.clear();
        open(&mut app);
        let used = fonts(&mut app);
        let mut d = opened(&app);
        set_source(&app, &mut d, "recent", &used);
        assert_eq!(d.family, "");
        set_source(&app, &mut d, "system", &used);
        assert_eq!((d.family.as_str(), d.style.as_str()), ("Source Sans 3", "Regular"), "the family's first listed style");
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
        let d = opened(&app);
        assert_eq!(d.family, system_family());
        assert!(can_replace(&d, None), "{}", d.family);
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
            let mut d = opened(&app);
            d.family = hidden.clone();
            assert!(!can_replace(&d, None), "{hidden}");
            set_source(&app, &mut d, "system", &used);
            assert_eq!(d.family, system_family());
        }
    }

    #[test]
    fn rows_flag_missing_fonts_substituted_styles_and_lacking_characters() {
        let row = |status: &str, glyphs: u64| json!({"status": status, "resolved": {"style": "W4"}, "missingGlyphs": glyphs});
        assert_eq!(font_note(&row("exact", 0)), (String::new(), Tone::Neutral));
        let missing = font_note(&row("missing", 0));
        assert!(missing.0.contains("missing") && missing.1 == Tone::Bad);
        let substitute = font_note(&row("substitute", 0));
        assert!(substitute.0.contains("substituted by W4") && substitute.1 == Tone::Warn);
        let (note, tone) = font_note(&row("exact", 2));
        assert!(note.contains("2 characters") && tone == Tone::Warn);
    }

    /// The font list the dialog keeps for the active document.
    fn kept_list(app: &VectorcraftApp) -> Arc<Vec<Value>> {
        app.canvas.find_font.as_ref().expect("the dialog kept the document's fonts").fonts.clone()
    }

    /// The kept list and the Find in Folder… answer are what `text.fonts` and `missing_fonts` give
    /// now, with a folder picker (`can_pick`) or without.
    fn assert_current(app: &mut VectorcraftApp, can_pick: bool, offered: bool, why: &str) {
        let listed = fonts(app);
        assert_eq!(*kept_list(app), listed, "{why}: the list");
        let missing = app.session.active().is_some_and(|st| !vectorcraft_engine::cmd::fontfiles::missing_fonts(&st.doc).is_empty());
        assert_eq!(offered, can_pick && missing, "{why}: Find in Folder…");
    }

    /// A document set in a missing family and in Inter.
    fn missing_font_document() -> VectorcraftApp {
        let mut app = new_app();
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        for (y, font) in [(20, "Missing Family"), (40, "Inter")] {
            app.session.execute("text.create", &json!({"x": 10, "y": y, "text": "Hi", "font": font})).unwrap();
        }
        app
    }

    /// Frames that change nothing in the document, a new selection included, keep the font list
    /// and the missing-font answer worked out on the first; the answer is worked out only once a
    /// folder picker is there.
    #[test]
    fn the_font_list_is_worked_out_once_per_document_version() {
        let mut app = missing_font_document();
        assert!(kept_over_frames(&mut app, false, "no folder picker"), "frames of one document version keep its list");
        assert_eq!(app.canvas.find_font.as_ref().and_then(|c| c.missing), None, "no picker: the document is not searched for missing fonts");
        assert!(kept_over_frames(&mut app, true, "a folder picker"), "a picker alone keeps the list");
        assert_eq!(app.canvas.find_font.as_ref().and_then(|c| c.missing), Some(true));
        app.run("select.all", json!({})).unwrap();
        assert!(kept_over_frames(&mut app, true, "a new selection"), "a new selection keeps the list");
    }

    /// Asks for the document's fonts as a frame does and tells whether they kept the list kept
    /// before them. Another test thread that changes the available fonts meanwhile makes the list
    /// work out again, so the asks are repeated until the fonts stay as they are.
    fn kept_over_frames(app: &mut VectorcraftApp, can_pick: bool, why: &str) -> bool {
        for _ in 0..20 {
            document_fonts(app, can_pick);
            // The generation the kept list was worked out under: the database's generation only
            // counts up, so the same one after the frames means no fonts changed in between.
            let before = kept_list(app);
            let generation = app.canvas.find_font.as_ref().map(|c| c.key.generation);
            let (_, offered) = document_fonts(app, can_pick);
            assert_current(app, can_pick, offered, why);
            if Some(vectorcraft_text::FontDb::global().generation()) == generation {
                return Arc::ptr_eq(&before, &kept_list(app));
            }
        }
        panic!("{why}: the available fonts kept changing")
    }

    /// An edit, its undo and a Change All each give the list of the document as it is then.
    #[test]
    fn an_edit_works_the_font_list_out_again() {
        let mut app = missing_font_document();
        let (first, _) = document_fonts(&mut app, true);
        app.run("text.replaceFont", json!({"from": {"family": "Missing Family"}, "to": {"family": "Inter"}})).unwrap();
        let (_, offered) = document_fonts(&mut app, true);
        assert_current(&mut app, true, offered, "after the replacement");
        assert!(!offered, "nothing is missing");
        assert_eq!(kept_list(&app).len(), 1, "{:?}", kept_list(&app));
        app.run("edit.undo", json!({})).unwrap();
        let (_, offered) = document_fonts(&mut app, true);
        assert_current(&mut app, true, offered, "after the undo");
        assert_eq!(*kept_list(&app), *first, "the undo brings back the first list");
        open(&mut app);
        let mut d = opened(&app);
        (d.family, d.style) = ("Source Serif 4".into(), String::new());
        let from = first.iter().find(|f| f["family"] == "Inter").unwrap().clone();
        change(&mut app, &d, &from, true).unwrap();
        let (_, offered) = document_fonts(&mut app, true);
        assert_current(&mut app, true, offered, "after Change All");
        assert!(kept_list(&app).iter().any(|f| f["family"] == "Source Serif 4"), "{:?}", kept_list(&app));
    }

    /// Fonts that become available, a composite font defined among them, give the list and the
    /// missing-font answer as they are with those fonts.
    #[test]
    fn other_available_fonts_work_the_font_list_out_again() {
        const FAMILY: &str = "Find Font Kept List CF";
        let mut app = missing_font_document();
        app.session.execute("text.create", &json!({"x": 10, "y": 60, "text": "Ab12", "font": FAMILY})).unwrap();
        document_fonts(&mut app, true);
        // The same document under another generation of the font database: what is kept for it no
        // longer counts.
        let entry = app.canvas.find_font.as_mut().unwrap();
        entry.key.generation = u64::MAX;
        entry.fonts = Arc::default();
        entry.missing = Some(false);
        let (_, offered) = document_fonts(&mut app, true);
        assert_current(&mut app, true, offered, "another font generation");
        let row = |app: &VectorcraftApp| kept_list(app).iter().find(|f| f["family"] == FAMILY).cloned().unwrap();
        assert_eq!(row(&app)["status"], "missing");
        let generation = vectorcraft_text::FontDb::global().generation();
        let parts = json!({"name": FAMILY, "parts": {"kanji": {"font": "Source Sans 3"}, "roman": {"font": "JetBrains Mono"}, "numerals": {"font": "Inter"}}});
        app.run("type.compositeFonts.set", json!({"fonts": [parts]})).unwrap();
        assert_ne!(vectorcraft_text::FontDb::global().generation(), generation, "defining a composite font is a new generation");
        let (_, offered) = document_fonts(&mut app, true);
        assert_current(&mut app, true, offered, "the composite font defined");
        assert_eq!(row(&app)["status"], "exact");
        app.run("type.compositeFonts.set", json!({"fonts": []})).unwrap();
        let (_, offered) = document_fonts(&mut app, true);
        assert_current(&mut app, true, offered, "the composite font taken out");
        assert_eq!(row(&app)["status"], "missing");
    }

    /// Another document in front, and pattern editing entered and canceled, each give the list of
    /// the document in front as it is then.
    #[test]
    fn the_font_list_follows_the_document_in_front() {
        let mut app = missing_font_document();
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let id = app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Hi", "font": "Source Serif 4"})).unwrap()["id"].clone();
        let current = |app: &mut VectorcraftApp, why: &str| {
            let (_, offered) = document_fonts(app, true);
            assert_current(app, true, offered, why);
            offered
        };
        current(&mut app, "the second document");
        app.session.set_active(0);
        assert!(current(&mut app, "the first document"), "the first document misses a font");
        app.session.set_active(1);
        app.run("select.set", json!({"ids": [id]})).unwrap();
        app.run("object.pattern.make", json!({"name": "Kept List"})).unwrap();
        assert!(app.session.active().unwrap().doc.pattern_edit.is_some(), "pattern editing");
        current(&mut app, "pattern editing");
        app.run("object.pattern.cancel", json!({})).unwrap();
        current(&mut app, "pattern editing canceled");
    }
}
