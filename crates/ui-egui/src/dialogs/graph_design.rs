//! Object › Graph › Design…, Column… and Marker…: the document's graph designs in a list.
//!
//! Graph Design: the designs, a preview of the selected one, and New Design (the selected art as a
//! design named "New Design"), Rename (to the name typed), Delete Design (the selected designs),
//! Paste Design (a copy of the selected design into the document, selected, to edit) and Select
//! Unused (the designs no graph draws with), each run at once by `graph.design`; Close closes.
//! Column: the design the selected graph series draw their columns or bars with, or None (plain
//! columns), a preview of it, Column Type, and for a repeating design Each Design Represents and
//! For Fractions; OK runs `graph.column`.
//! Marker: the design the selected graph series draw their data points with, or None (the default
//! square marker); OK runs `graph.marker`.
//! Column and Marker also list, under each library's name, the graph designs of the documents
//! Window › Swatch Libraries › Other Library… loaded that the document lacks; OK on one copies it
//! into the document with the resources its art uses.
//!
//! One [`GraphDesignDialog`] is all three. Its OK is held: it files the answer, [`pump`] runs the
//! command and answers it, and a refusal keeps the dialog open with the reason.
//!
//! Fields: `selected` (Graph Design: the selected designs' names; Column and Marker: one name, null
//! for None), in Column and Marker `library` (the library of the selected design, null for the
//! document's), in Graph Design `name` (the name Rename gives), and in Column `columnType`, `each`
//! and `fractions`. Setting `selected` to a name finds its library itself.

use std::cell::RefCell;
use std::sync::Arc;

use craft::behavior::{DisplayOrder, ItemSource, Selection};
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Entry, Pad, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Sizing, clicked};
use craft::ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, FieldRefusal, HeldAnswer, HeldOk, StackDialog, dialog_id};
use craft::ui::display::{Picture, PictureSize, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{List, ListRow, ThumbSize};
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::pickers::Select;
use craft::ui::primitives::Revision;
use craft::ui::runtime::env::Label;
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::style::SurfaceKind;
use craft::ui::text::{NumberField, TextField};
use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node};

use crate::VectorcraftApp;

/// The dialog kinds.
pub const DESIGN: &str = "graphDesign";
pub const COLUMN: &str = "graphColumn";
pub const MARKER: &str = "graphMarker";

/// The rows the design list shows before it scrolls.
const LIST_ROWS: u16 = 7;

/// The side of the square a design's preview is drawn at, in pixels.
const PREVIEW_PX: u32 = 256;

/// The values Each Design Represents takes in the dialog (`graph.column` takes any value more than 0).
const EACH_RANGE: std::ops::RangeInclusive<f64> = 0.0001..=1_000_000_000.0;

/// Which of the three dialogs a [`GraphDesignDialog`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Object › Graph › Design…: manages the document's designs.
    Design,
    /// Object › Graph › Column…: picks the design the selected series draw their columns with.
    Column,
    /// Object › Graph › Marker…: picks the design the selected series draw their points with.
    Marker,
}

impl Kind {
    /// The dialog's kind string, which automation sees.
    fn key(self) -> &'static str {
        match self {
            Kind::Design => DESIGN,
            Kind::Column => COLUMN,
            Kind::Marker => MARKER,
        }
    }
}

/// A design: its name and the library it comes from (`None`: the document's own).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct DesignKey {
    /// The id of the loaded library that holds the design.
    pub library: Option<String>,
    /// The design's name.
    pub name: String,
}

impl DesignKey {
    /// The document's design `name`.
    fn document(name: &str) -> Self {
        DesignKey { library: None, name: name.to_owned() }
    }
}

/// A row of the design list.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum RowKey {
    /// Column and Marker's None: no design.
    None,
    /// A library's name, over its designs; it cannot be picked.
    Heading(String),
    /// A design.
    Design(DesignKey),
}

/// The rows of one pass as the list's item source.
struct Rows<'a>(&'a [RowKey]);

impl DisplayOrder<RowKey> for Rows<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> RowKey {
        // The list only asks for positions below `len`.
        self.0.get(at).cloned().unwrap_or(RowKey::None)
    }

    fn is_section(&self, at: usize) -> bool {
        matches!(self.0.get(at), Some(RowKey::Heading(_)))
    }
}

impl ItemSource<RowKey> for Rows<'_> {}

/// How Column draws the design.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum ColumnType {
    #[default]
    Vertical,
    Uniform,
    Repeating,
    Sliding,
}

impl ColumnType {
    /// The name `graph.column` takes.
    fn as_str(self) -> &'static str {
        match self {
            ColumnType::Vertical => "vertical",
            ColumnType::Uniform => "uniform",
            ColumnType::Repeating => "repeating",
            ColumnType::Sliding => "sliding",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        [ColumnType::Vertical, ColumnType::Uniform, ColumnType::Repeating, ColumnType::Sliding].into_iter().find(|t| t.as_str() == s)
    }
}

/// What a repeating design does with a fraction of its length.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Fractions {
    #[default]
    Chop,
    Scale,
}

impl Fractions {
    /// The name `graph.column` takes.
    fn as_str(self) -> &'static str {
        match self {
            Fractions::Chop => "chop",
            Fractions::Scale => "scale",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        [Fractions::Chop, Fractions::Scale].into_iter().find(|f| f.as_str() == s)
    }
}

/// Column's options beside the design.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ColumnOptions {
    column_type: ColumnType,
    each: f64,
    fractions: Fractions,
}

impl Default for ColumnOptions {
    fn default() -> Self {
        ColumnOptions { column_type: ColumnType::default(), each: vectorcraft_doc::ColumnDesign::EACH, fractions: Fractions::default() }
    }
}

/// What a Graph Design button asks `graph.design` to do.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Action {
    /// New Design: the selected art as a design.
    New,
    /// Rename: the design `from` to `to`.
    Rename { from: String, to: String },
    /// Delete Design: these designs.
    Delete(Vec<String>),
    /// Paste Design: a copy of this design into the document.
    Paste(String),
    /// Select Unused: the designs no graph draws with.
    SelectUnused,
}

/// What the dialog shows once an [`Action`] ran.
#[derive(Default)]
struct Effects {
    /// The designs to select.
    select: Option<Vec<String>>,
    /// Paste Design leaves the copy selected to edit, as the dialog closes.
    close: bool,
}

impl Action {
    /// `graph.design`'s params.
    fn params(&self) -> Value {
        match self {
            Action::New => json!({ "new": true }),
            Action::Rename { from, to } => json!({ "rename": from, "to": to }),
            Action::Delete(names) => json!({ "delete": names }),
            Action::Paste(name) => json!({ "paste": name }),
            Action::SelectUnused => json!({ "selectUnused": true }),
        }
    }

    /// Notes what the dialog does with the command's result `r`.
    fn apply(&self, r: &Value, effects: &mut Effects) {
        let names = |v: &Value| v.as_array().map(|a| a.iter().filter_map(|n| n.as_str().map(str::to_owned)).collect::<Vec<_>>());
        if let Some(name) = r["name"].as_str() {
            effects.select = Some(vec![name.to_owned()]);
        }
        if *self == Action::SelectUnused
            && let Some(unused) = names(&r["unused"])
        {
            effects.select = Some(unused);
        }
        effects.close |= matches!(self, Action::Paste(_));
    }
}

/// What OK of Column or Marker files for [`pump`] to run.
#[derive(Clone)]
pub(crate) enum Answer {
    /// `graph.column`.
    Column { design: Option<DesignKey>, options: ColumnOptions },
    /// `graph.marker`.
    Marker { design: Option<DesignKey> },
}

impl Answer {
    /// Runs the command; the refusal is its error.
    fn run(&self, app: &mut VectorcraftApp) -> Result<(), Label> {
        let design = |d: &Option<DesignKey>| json!({ "design": d.as_ref().map(|d| d.name.as_str()), "library": library_of(d.as_ref()) });
        let (command, params) = match self {
            Answer::Column { design: d, options } => {
                let mut params = design(d);
                params["columnType"] = json!(options.column_type.as_str());
                params["each"] = json!(options.each);
                params["fractions"] = json!(options.fractions.as_str());
                ("graph.column", params)
            }
            Answer::Marker { design: d } => ("graph.marker", design(d)),
        };
        app.run(command, params).map(|_| ()).map_err(Label::from)
    }
}

/// A loaded library's document, for the thumbnails of its designs.
struct LibraryDoc {
    id: String,
    /// Which load of the library this is ([`vectorcraft_engine::cmd::swatchlib::DesignLibrary::generation`]).
    generation: u64,
    doc: Arc<Document>,
}

/// What the dialog reads from the app: the app pushes it in every pass ([`pump`]), since the body gets no app.
struct Lists {
    /// The document's design names.
    designs: Vec<String>,
    /// The loaded libraries' designs the document lacks ([`library_designs`]).
    libraries: Vec<(String, String, Vec<String>)>,
    /// Whether art is selected: New Design needs it.
    has_art: bool,
    doc: Arc<Document>,
    doc_revision: u64,
    lib_docs: Vec<LibraryDoc>,
}

impl Lists {
    fn read(app: &VectorcraftApp) -> Result<Lists, String> {
        let designs = names(app)?;
        let st = app.session.active().ok_or("no document open")?;
        Ok(Lists {
            designs,
            libraries: library_designs(app),
            has_art: !st.selection.is_empty(),
            doc: st.doc.clone(),
            doc_revision: st.revision,
            lib_docs: app
                .session
                .graph_design_libraries
                .iter()
                .map(|l| LibraryDoc { id: l.id.clone(), generation: l.generation, doc: l.doc.clone() })
                .collect(),
        })
    }

    /// Whether the document or the library of `key` still has the design.
    fn holds(&self, key: &DesignKey) -> bool {
        match &key.library {
            None => self.designs.contains(&key.name),
            Some(id) => self.libraries.iter().any(|(l, _, designs)| l == id && designs.contains(&key.name)),
        }
    }

    /// The design called `name`: the document's, else the library `prefer` when it has one, else the first library that does.
    fn find(&self, name: &str, prefer: Option<&str>) -> Option<DesignKey> {
        if self.designs.iter().any(|n| n == name) {
            return Some(DesignKey::document(name));
        }
        let has = |(id, _, designs): &&(String, String, Vec<String>)| designs.iter().any(|n| n == name) && prefer.is_none_or(|p| p == id);
        let library = self.libraries.iter().find(has).or_else(|| self.libraries.iter().find(|(_, _, designs)| designs.iter().any(|n| n == name)))?;
        Some(DesignKey { library: Some(library.0.clone()), name: name.to_owned() })
    }

    fn library_name(&self, id: &str) -> String {
        self.libraries.iter().find(|(l, ..)| l == id).map(|(_, name, _)| name.clone()).unwrap_or_default()
    }

    /// The list's rows: Graph Design the document's designs; Column and Marker None, the document's designs, then
    /// each library's name over its designs.
    fn rows(&self, kind: Kind) -> Vec<RowKey> {
        let own = self.designs.iter().map(|n| RowKey::Design(DesignKey::document(n)));
        if kind == Kind::Design {
            return own.collect();
        }
        let libraries = self.libraries.iter().flat_map(|(id, _, designs)| {
            let from = designs.iter().map(|n| RowKey::Design(DesignKey { library: Some(id.clone()), name: n.clone() }));
            std::iter::once(RowKey::Heading(id.clone())).chain(from)
        });
        std::iter::once(RowKey::None).chain(own).chain(libraries).collect()
    }

    /// The design's document, art and the revision of that document's picture of it.
    fn art(&self, key: &DesignKey) -> Option<(Arc<Document>, Arc<Node>, Revision)> {
        let (doc, revision) = match &key.library {
            None => (&self.doc, Revision::new(self.doc_revision)),
            Some(id) => {
                let lib = self.lib_docs.iter().find(|l| &l.id == id)?;
                (&lib.doc, Revision::new(lib.generation))
            }
        };
        let art = doc.graph_designs.iter().find(|d| d.name == key.name)?.art.clone();
        Some((doc.clone(), art, revision))
    }
}

/// Graph Design, Graph Column or Graph Marker, on the window's dialog host.
pub(crate) struct GraphDesignDialog {
    id: DialogId,
    kind: Kind,
    lists: Lists,
    /// The selected rows: any number of designs in Graph Design; one design, or None, in Column and Marker.
    selection: Selection<RowKey>,
    /// Graph Design's Name field.
    name: String,
    column: ColumnOptions,
    /// The buttons pressed since [`pump`] last ran.
    actions: Vec<Action>,
    /// OK of Column and Marker.
    held: HeldAnswer<Answer>,
}

/// What [`pump`] takes from the open dialog to run.
struct Work {
    actions: Vec<Action>,
    answer: Option<Answer>,
}

impl GraphDesignDialog {
    fn new(kind: Kind, lists: Lists, chosen: Vec<DesignKey>, column: ColumnOptions) -> Self {
        let mut dialog = GraphDesignDialog {
            id: dialog_id(kind.key()),
            kind,
            lists,
            selection: Selection::new(Vec::new(), None, None),
            name: String::new(),
            column,
            actions: Vec::new(),
            held: HeldAnswer::new(),
        };
        dialog.select(chosen);
        dialog
    }

    /// The designs among the selected rows.
    fn chosen(&self) -> Vec<DesignKey> {
        designs_in(&self.selection)
    }

    /// Selects `keys` without touching the Name field: all of them in Graph Design, the first, or None, in the others.
    fn set_selection(&mut self, keys: Vec<DesignKey>) {
        let mut rows: Vec<RowKey> = keys.into_iter().map(RowKey::Design).collect();
        if self.kind != Kind::Design {
            rows.truncate(1);
            if rows.is_empty() {
                rows.push(RowKey::None);
            }
        }
        let last = rows.last().cloned();
        self.selection = Selection::new(rows, last.clone(), last);
    }

    /// Selects `keys`, and Graph Design's Name field takes the first one's name.
    fn select(&mut self, keys: Vec<DesignKey>) {
        if self.kind == Kind::Design {
            self.name = keys.first().map(|k| k.name.clone()).unwrap_or_default();
        }
        self.set_selection(keys);
    }

    /// Takes what the app pushed in, and drops the selected designs it no longer has.
    fn feed(&mut self, lists: Lists) {
        self.lists = lists;
        let chosen = self.chosen();
        let kept: Vec<DesignKey> = chosen.iter().filter(|k| self.lists.holds(k)).cloned().collect();
        if kept.len() != chosen.len() {
            self.set_selection(kept);
        }
    }

    fn take_work(&mut self) -> Work {
        Work { actions: std::mem::take(&mut self.actions), answer: self.held.take() }
    }

    /// What OK files.
    fn answer(&self) -> Answer {
        let design = self.chosen().into_iter().next();
        match self.kind {
            Kind::Marker => Answer::Marker { design },
            _ => Answer::Column { design, options: self.column.clone() },
        }
    }

    fn show_design(&mut self, ui: &mut egui::Ui) -> Option<DialogResult<()>> {
        let close = FooterButton::new(tl!("Close").to_owned(), DialogRole::Cancel);
        let shown = Dialog::new(tl!("Graph Design").to_owned()).buttons([close]).show(ui, |ui, _| self.design_body(ui));
        shown.output.closed.map(|_| DialogResult::Cancel)
    }

    fn design_body(&mut self, ui: &mut egui::Ui) {
        // PLAN: the sheet's note 3 lists library headings in Graph Design; production lists the document's designs
        // only and every action runs on a document design's name, so the list follows production.
        let GraphDesignDialog { lists, selection, name, actions, .. } = self;
        let lists = &*lists;
        let before = selection.items.clone();
        let under_list = |ui: &mut egui::Ui, selection: &mut Selection<RowKey>, side: &mut Side<'_>| {
            if selection.items != before {
                *side.name = designs_in(selection).first().map(|k| k.name.clone()).unwrap_or_default();
            }
            Field::new(tl!("Name:").to_owned()).show(ui, |ui, _| {
                TextField::new(tl!("Name").to_owned(), Bind::bound(&mut *side.name)).show(ui);
            });
        };
        let under_preview = |ui: &mut egui::Ui, selection: &Selection<RowKey>, side: &mut Side<'_>| {
            let Side { name, actions } = side;
            let chosen: Vec<String> = designs_in(selection).into_iter().map(|k| k.name).collect();
            let button = |ui: &mut egui::Ui, label: &str, enabled: bool| {
                clicked(&Button::new(label.to_owned()).enabled(enabled).sizing(Sizing::FillWidth).show(ui))
            };
            if button(ui, tl!("New Design"), lists.has_art) {
                actions.push(Action::New);
            }
            let rename = chosen.len() == 1 && !name.trim().is_empty();
            if button(ui, tl!("Rename"), rename)
                && let Some(from) = chosen.first()
            {
                actions.push(Action::Rename { from: from.clone(), to: name.clone() });
            }
            if button(ui, tl!("Delete Design"), !chosen.is_empty()) {
                actions.push(Action::Delete(chosen.clone()));
            }
            if button(ui, tl!("Paste Design"), chosen.len() == 1)
                && let Some(only) = chosen.first()
            {
                actions.push(Action::Paste(only.clone()));
            }
            if button(ui, tl!("Select Unused"), !lists.designs.is_empty()) {
                actions.push(Action::SelectUnused);
            }
        };
        picker(ui, Kind::Design, lists, selection, &mut Side { name, actions }, under_list, under_preview);
    }

    fn show_form(&mut self, ui: &mut egui::Ui) -> Option<DialogResult<()>> {
        if self.held.is_accepted() {
            self.held.finish();
            return Some(DialogResult::Ok(()));
        }
        let title = if self.kind == Kind::Column { tl!("Graph Column") } else { tl!("Graph Marker") };
        let waiting = self.held.is_waiting();
        let GraphDesignDialog { kind, lists, selection, column, held, .. } = self;
        let (kind, lists) = (*kind, &*lists);
        let form = FormDialog::new(title.to_owned(), Bind::bound(&mut *column)).accept_stays_open(true).ok_enabled(!waiting);
        let shown = form.show(ui, |ui, _, options| {
            let (rows, opts) = (selection.items.clone(), options.clone());
            picker(ui, kind, lists, selection, &mut (), |_, _, _| {}, |_, _, _| {});
            if kind == Kind::Column {
                column_options(ui, options);
            }
            if let Some((reason, _)) = held.refusal() {
                Text::new(reason.clone()).show(ui);
            }
            // A refusal is about the values it refused.
            if selection.items != rows || *options != opts {
                held.clear_refusal();
            }
            true
        });
        if let Some(options) = shown.accepted {
            self.column = options;
            let answer = self.answer();
            self.held.file(answer);
            return None;
        }
        shown.closed.map(|_| DialogResult::Cancel)
    }
}

/// What Graph Design's own rows read and write beside the selection.
struct Side<'a> {
    name: &'a mut String,
    actions: &'a mut Vec<Action>,
}

/// The list of designs with a preview of the first one picked beside it; `under_list` and `under_preview` add the
/// dialog's own rows below each, sharing `side`.
fn picker<S>(
    ui: &mut egui::Ui,
    kind: Kind,
    lists: &Lists,
    selection: &mut Selection<RowKey>,
    side: &mut S,
    mut under_list: impl FnMut(&mut egui::Ui, &mut Selection<RowKey>, &mut S),
    mut under_preview: impl FnMut(&mut egui::Ui, &Selection<RowKey>, &mut S),
) {
    let rows = lists.rows(kind);
    let source = Rows(&rows);
    Row::new().show(ui, |ui, _| {
        Stack::new().show(ui, |ui, _| {
            design_list(kind, selection).show_items(ui, &source, &mut |key, _| row(key, lists));
            under_list(ui, selection, side);
        });
        Stack::new().show(ui, |ui, _| {
            preview(ui, lists, designs_in(selection).first());
            under_preview(ui, selection, side);
        });
    });
}

/// The list: Graph Design picks any number of designs, Column and Marker exactly one row.
fn design_list(kind: Kind, selection: &mut Selection<RowKey>) -> List<'_, RowKey> {
    let list = List::new(tl!("Designs").to_owned())
        .id_salt(kind.key())
        .surface(SurfaceKind::Sunken)
        .pad(Pad::Control)
        .max_rows(LIST_ROWS)
        .selection(Bind::bound(selection));
    if kind == Kind::Design { list } else { list.select(SelectMode::Single).allow_empty(false) }
}

fn row<'a>(key: &RowKey, lists: &Lists) -> ListRow<'a> {
    match key {
        RowKey::None => ListRow::new(tl!("None").to_owned()),
        RowKey::Design(design) => ListRow::new(design.name.clone()),
        // The library's own name, not interface text.
        RowKey::Heading(id) => ListRow::new(lists.library_name(id)).section_heading(),
    }
}

/// Column's Column Type, and for a repeating design Each Design Represents and For Fractions.
fn column_options(ui: &mut egui::Ui, options: &mut ColumnOptions) {
    let repeating = options.column_type == ColumnType::Repeating;
    let types = [
        Entry::new(ColumnType::Vertical, tl!("Vertically Scaled").to_owned()),
        Entry::new(ColumnType::Uniform, tl!("Uniformly Scaled").to_owned()),
        Entry::new(ColumnType::Repeating, tl!("Repeating").to_owned()),
        Entry::new(ColumnType::Sliding, tl!("Sliding").to_owned()),
    ];
    Field::new(tl!("Column Type:").to_owned()).show(ui, |ui, _| {
        Select::new(tl!("Column Type").to_owned(), Bind::bound(&mut options.column_type), &types).show(ui);
    });
    Field::new(tl!("Each Design Represents:").to_owned()).show(ui, |ui, _| {
        let mut each = Some(options.each);
        NumberField::new(tl!("Each Design Represents").to_owned(), Bind::bound(&mut each))
            .range(EACH_RANGE)
            .fixed_decimals(4)
            .enabled(repeating)
            .show(ui);
        // A cleared field keeps the last number.
        options.each = each.unwrap_or(options.each);
    });
    let fractions = [Entry::new(Fractions::Chop, tl!("Chop Design").to_owned()), Entry::new(Fractions::Scale, tl!("Scale Design").to_owned())];
    Field::new(tl!("For Fractions:").to_owned()).show(ui, |ui, _| {
        Select::new(tl!("For Fractions").to_owned(), Bind::bound(&mut options.fractions), &fractions).enabled(repeating).show(ui);
    });
}

/// The design `key` drawn in the preview's square, or an empty square without one.
fn preview(ui: &mut egui::Ui, lists: &Lists, key: Option<&DesignKey>) {
    let shown = key.and_then(|key| lists.art(key));
    let revision = shown.as_ref().map(|(.., revision)| *revision).unwrap_or_default();
    let image = || shown.as_ref().map_or_else(blank, |(doc, art, _)| thumbnail(doc, art));
    Picture::new("", Revision::of(&key).get(), &image).revision(revision).extent(PictureSize::Sample(ThumbSize::Large)).show(ui);
}

fn blank() -> egui::ColorImage {
    egui::ColorImage::filled([1, 1], egui::Color32::TRANSPARENT)
}

/// `art` drawn on white at the preview's side.
fn thumbnail(doc: &Document, art: &Node) -> egui::ColorImage {
    thread_local! {
        static RENDERER: RefCell<vectorcraft_render::Renderer> = RefCell::new(vectorcraft_render::Renderer::new());
    }
    RENDERER
        .with(|r| r.borrow_mut().render_node_thumbnail(doc, art, PREVIEW_PX, Some([255, 255, 255, 255])))
        .map_or_else(blank, |img| crate::canvas::color_image(&img))
}

/// The designs among `selection`'s rows.
fn designs_in(selection: &Selection<RowKey>) -> Vec<DesignKey> {
    selection
        .items
        .iter()
        .filter_map(|row| match row {
            RowKey::Design(design) => Some(design.clone()),
            _ => None,
        })
        .collect()
}

/// Column and Marker's `library`: the library of the selected design, `None` for the document's.
fn library_of(design: Option<&DesignKey>) -> Option<&str> {
    design.and_then(|d| d.library.as_deref())
}

impl StackDialog for GraphDesignDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(self.kind.key())
    }

    fn show(&mut self, ui: &mut egui::Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        match self.kind {
            Kind::Design => self.show_design(ui),
            Kind::Column | Kind::Marker => self.show_form(ui),
        }
    }

    /// OK files the answer for [`pump`]; once the command accepted it, OK closes the dialog. Graph Design has no OK.
    fn confirm(&mut self, _: &egui::Context) -> Option<()> {
        if self.kind == Kind::Design {
            return None;
        }
        if self.held.finish().is_some() {
            return Some(());
        }
        if !self.held.is_waiting() {
            let answer = self.answer();
            self.held.file(answer);
        }
        None
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    fn json_fields(&self) -> Option<Value> {
        let chosen = self.chosen();
        let first = chosen.first();
        Some(match self.kind {
            Kind::Design => json!({ "selected": chosen.iter().map(|k| k.name.as_str()).collect::<Vec<_>>(), "name": self.name }),
            Kind::Marker => json!({ "selected": first.map(|k| k.name.as_str()), "library": library_of(first) }),
            Kind::Column => json!({
                "selected": first.map(|k| k.name.as_str()),
                "library": library_of(first),
                "columnType": self.column.column_type.as_str(),
                "each": self.column.each,
                "fractions": self.column.fractions.as_str(),
            }),
        })
    }

    /// Sets a field as its row does: a design the document and the libraries no longer hold is refused, and in
    /// Column `each` and `fractions` take no input unless `columnType` is `repeating`.
    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        let rejects = || FieldRefusal::Rejects(&value).message(field);
        match (self.kind, field) {
            (Kind::Design, "selected") => {
                let names = value.as_array().ok_or_else(rejects)?;
                let keys = names
                    .iter()
                    .map(|n| n.as_str().and_then(|n| self.lists.find(n, None)).filter(|k| k.library.is_none()).ok_or_else(rejects))
                    .collect::<Result<Vec<_>, _>>()?;
                self.select(keys);
            }
            (Kind::Design, "name") => self.name = value.as_str().ok_or_else(rejects)?.to_owned(),
            (Kind::Column | Kind::Marker, "selected") => {
                let key = match &value {
                    Value::Null => None,
                    Value::String(n) => {
                        let current = self.chosen();
                        Some(self.lists.find(n, library_of(current.first())).ok_or_else(rejects)?)
                    }
                    _ => return Err(rejects()),
                };
                self.select(key.into_iter().collect());
            }
            (Kind::Column | Kind::Marker, "library") => {
                let library = match &value {
                    Value::Null => None,
                    Value::String(id) => Some(id.clone()),
                    _ => return Err(rejects()),
                };
                let name = self.chosen().into_iter().next().map(|k| k.name).ok_or_else(rejects)?;
                let moved = DesignKey { library, name };
                if !self.lists.holds(&moved) {
                    return Err(rejects());
                }
                self.select(vec![moved]);
            }
            (Kind::Column, "columnType") => self.column.column_type = value.as_str().and_then(ColumnType::parse).ok_or_else(rejects)?,
            (Kind::Column, "each" | "fractions") if self.column.column_type != ColumnType::Repeating => {
                return Err(FieldRefusal::Inactive.message(field));
            }
            (Kind::Column, "each") => self.column.each = value.as_f64().filter(|v| EACH_RANGE.contains(v)).ok_or_else(rejects)?,
            (Kind::Column, "fractions") => self.column.fractions = value.as_str().and_then(Fractions::parse).ok_or_else(rejects)?,
            _ => return Err(FieldRefusal::Unknown.message(field)),
        }
        self.held.clear_refusal();
        Ok(())
    }
}

/// Open Graph Design.
pub fn open_design(app: &mut VectorcraftApp) -> Result<Value, String> {
    let lists = Lists::read(app)?;
    let first = lists.designs.first().map(|n| DesignKey::document(n));
    open(app, GraphDesignDialog::new(Kind::Design, lists, first.into_iter().collect(), ColumnOptions::default()))
}

/// Open Column for the selected graph series, on the design and options they draw their columns with (None for plain
/// columns).
pub fn open_column(app: &mut VectorcraftApp) -> Result<Value, String> {
    let v = app.session.execute("graph.column", &json!({})).map_err(|e| e.to_string())?;
    let options = ColumnOptions {
        column_type: v["columnType"].as_str().and_then(ColumnType::parse).unwrap_or_default(),
        each: v["each"].as_f64().unwrap_or(vectorcraft_doc::ColumnDesign::EACH),
        fractions: v["fractions"].as_str().and_then(Fractions::parse).unwrap_or_default(),
    };
    let chosen = v["design"].as_str().map(DesignKey::document);
    open(app, GraphDesignDialog::new(Kind::Column, Lists::read(app)?, chosen.into_iter().collect(), options))
}

/// Open Marker for the selected graph series, on the design they draw with (None for the default).
pub fn open_marker(app: &mut VectorcraftApp) -> Result<Value, String> {
    let v = app.session.execute("graph.marker", &json!({})).map_err(|e| e.to_string())?;
    let chosen = v["design"].as_str().map(DesignKey::document);
    open(app, GraphDesignDialog::new(Kind::Marker, Lists::read(app)?, chosen.into_iter().collect(), ColumnOptions::default()))
}

fn open(app: &VectorcraftApp, dialog: GraphDesignDialog) -> Result<Value, String> {
    app.dialogs().ok_or("no window")?.with(|host| host.open(dialog));
    Ok(Value::Null)
}

/// The document's design names, read from the document.
fn names(app: &VectorcraftApp) -> Result<Vec<String>, String> {
    let st = app.session.active().ok_or("no document open")?;
    Ok(st.doc.graph_designs.iter().map(|d| d.name.clone()).collect())
}

/// The designs of the loaded libraries that the document lacks (the document's own design of a name
/// is the one used), by library: its id, its name and the design names, libraries with none left
/// out.
fn library_designs(app: &VectorcraftApp) -> Vec<(String, String, Vec<String>)> {
    let own = names(app).unwrap_or_default();
    app.session
        .graph_design_libraries
        .iter()
        .map(|l| (l.id.clone(), l.name.clone(), l.doc.graph_designs.iter().map(|g| g.name.clone()).filter(|n| !own.contains(n)).collect::<Vec<_>>()))
        .filter(|(.., names)| !names.is_empty())
        .collect()
}

/// Runs the open dialog's buttons and held OK, and pushes the app's lists into it. Every pass: the lists change
/// without a pass of the dialog (an undo, a library loaded). The commands run outside the host's lock.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let Some(work) = dialogs.with(|host| host.get_mut::<GraphDesignDialog>().map(GraphDesignDialog::take_work)) else { return };
    let mut effects = Effects::default();
    for action in &work.actions {
        match app.run("graph.design", action.params()) {
            Ok(r) => action.apply(&r, &mut effects),
            Err(e) => app.status(e),
        }
    }
    let answered = work.answer.map(|answer| answer.run(app));
    let lists = Lists::read(app).ok();
    dialogs.with(|host| {
        if let Some(dialog) = host.get_mut::<GraphDesignDialog>() {
            if let Some(lists) = lists {
                dialog.feed(lists);
            }
            if let Some(names) = effects.select {
                dialog.select(names.iter().map(|n| DesignKey::document(n)).collect());
            }
            if let Some(answered) = answered {
                dialog.held.answer(answered);
            }
        }
        if effects.close {
            host.cancel::<GraphDesignDialog>(ctx);
        }
    });
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::GraphDesignDialog;
    use crate::VectorcraftApp;

    fn app_with_design() -> (VectorcraftApp, egui::Context) {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = crate::testing::ui_context();
        app.ui_ctx = Some(ctx.clone());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let r = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [r]})).unwrap();
        app.run("graph.design", json!({"new": true})).unwrap();
        (app, ctx)
    }

    fn select_graph(app: &mut VectorcraftApp, kind: &str) {
        let g = app.run("graph.create", json!({"type": kind, "x": 100, "y": 100, "width": 200, "height": 100})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [g]})).unwrap();
    }

    /// The open dialog's fields, as automation lists them.
    fn fields(app: &VectorcraftApp) -> Value {
        app.dialogs().unwrap().with(|host| host.open_dialogs()).last().unwrap().fields.clone().unwrap()
    }

    /// `ui.dialog.set` on the open dialog.
    fn set(app: &VectorcraftApp, ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        app.dialogs().unwrap().with(|host| {
            let id = host.open_dialogs().last().unwrap().id;
            host.set_json_field(ctx, id, field, value)
        })
    }

    /// OK through the host: it files the held answer, the pump runs the command, and OK closes the dialog.
    fn ok(app: &mut VectorcraftApp, ctx: &egui::Context) {
        let dialogs = app.dialogs().unwrap();
        assert!(!dialogs.with(|host| host.confirm::<GraphDesignDialog>(ctx)), "OK holds its answer for the pump");
        super::pump(app, ctx);
        assert!(dialogs.with(|host| host.confirm::<GraphDesignDialog>(ctx)), "the accepted answer closes the dialog");
        assert!(!dialogs.with(|host| host.is_open::<GraphDesignDialog>()));
    }

    #[test]
    fn marker_lists_the_designs_and_applies_the_one_picked() {
        let (mut app, ctx) = app_with_design();
        select_graph(&mut app, "line");
        super::open_marker(&mut app).unwrap();
        assert_eq!(fields(&app)["selected"], json!(null), "None: the default marker");
        set(&app, &ctx, "selected", json!("New Design")).unwrap();
        ok(&mut app, &ctx);
        assert_eq!(app.session.execute("graph.marker", &json!({})).unwrap()["design"], "New Design");
        super::open_marker(&mut app).unwrap();
        assert_eq!(fields(&app)["selected"], "New Design");
        // None goes back to the default marker.
        set(&app, &ctx, "selected", json!(null)).unwrap();
        ok(&mut app, &ctx);
        assert_eq!(app.session.execute("graph.marker", &json!({})).unwrap()["design"], json!(null));
    }

    #[test]
    fn column_opens_from_the_graph_menu_and_applies_the_design_and_its_options() {
        let (mut app, ctx) = app_with_design();
        select_graph(&mut app, "column");
        crate::menus::invoke(&mut app, "graph.column", json!({}));
        let f = fields(&app);
        assert_eq!(
            (f["selected"].clone(), f["columnType"].clone(), f["each"].clone(), f["fractions"].clone()),
            (json!(null), json!("vertical"), json!(10.0), json!("chop"))
        );
        // Each Design Represents and For Fractions take input for a repeating design only.
        assert!(set(&app, &ctx, "each", json!(0.5)).is_err());
        assert!(set(&app, &ctx, "fractions", json!("scale")).is_err());
        set(&app, &ctx, "selected", json!("New Design")).unwrap();
        set(&app, &ctx, "columnType", json!("repeating")).unwrap();
        set(&app, &ctx, "each", json!(0.5)).unwrap();
        set(&app, &ctx, "fractions", json!("scale")).unwrap();
        ok(&mut app, &ctx);
        let v = app.session.execute("graph.column", &json!({})).unwrap();
        assert_eq!(v, json!({"designs": ["New Design"], "design": "New Design", "columnType": "repeating", "each": 0.5, "fractions": "scale"}));
        // It opens again on what the series draw with, and None goes back to plain columns.
        super::open_column(&mut app).unwrap();
        let f = fields(&app);
        assert_eq!((f["selected"].clone(), f["columnType"].clone(), f["each"].clone()), (json!("New Design"), json!("repeating"), json!(0.5)));
        set(&app, &ctx, "selected", json!(null)).unwrap();
        ok(&mut app, &ctx);
        assert_eq!(app.session.execute("graph.column", &json!({})).unwrap()["design"], json!(null));
    }

    #[test]
    fn marker_lists_a_loaded_documents_designs_and_ok_copies_the_one_picked_in() {
        // A document with design "New Design", loaded as a swatch library (Other Library…).
        let (src, _) = app_with_design();
        let bytes = vectorcraft_format::save_file(&src.session.active().unwrap().doc);
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = crate::testing::ui_context();
        app.ui_ctx = Some(ctx.clone());
        let p = json!({"dataBase64": vectorcraft_format::base64_encode(&bytes), "name": "Dots.vectorcraft"});
        app.run("swatch.library.load", p).unwrap();
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        select_graph(&mut app, "line");
        let libraries = super::library_designs(&app);
        assert_eq!(libraries.len(), 1);
        let (id, _, designs) = &libraries[0];
        assert_eq!(designs, &["New Design".to_string()]);
        crate::menus::invoke(&mut app, "graph.marker", json!({}));
        set(&app, &ctx, "selected", json!("New Design")).unwrap();
        assert_eq!(fields(&app)["library"], json!(id), "the design is found in its library");
        ok(&mut app, &ctx);
        assert_eq!(app.session.execute("graph.marker", &json!({})).unwrap()["design"], "New Design");
        assert_eq!(super::names(&app).unwrap(), ["New Design"]);
        // The document has it now, so the library lists it no more.
        assert!(super::library_designs(&app).is_empty());
    }
}
