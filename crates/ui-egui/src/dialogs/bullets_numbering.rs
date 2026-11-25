//! More Options at the end of the Paragraph panel's Bullets and Numbered List presets (or `ui.bulletsAndNumbering {tab?}`): a
//! Bullets tab and a Numbers tab, each with the Level field and the presets, and the Position group
//! (Alignment, Left Indent and First Line Indent) of the paragraphs the Paragraph panel acts on.
//! The dialog opens on the values the Paragraph panel shows, with no preset picked and a field
//! blank where the paragraphs differ. Each change shows on the type while the dialog is open. OK
//! runs `text.setFormat` with the values that changed, and with the indents it shows when a preset
//! puts the paragraphs in a list, as one undo step; Cancel puts the type back as it was. A preset
//! picked for paragraphs that are in no list and have no indents fills in the 18 pt and -18 pt
//! indents `text.setFormat` gives them, and None takes them out again.
//!
//! Fields: `tab` (`bullets` or `numbers`), `list` (`none`, `bullet` or `number`: the kind of the
//! preset picked), `bullet` (`dot`, `dash`, `circle` or `square`), `number` (`arabic`,
//! `upperLetters`, `lowerLetters`, `upperRoman`, `lowerRoman`, `twoDigits` or `threeDigits`),
//! `listAlign` (`left` or `right`), `listLevel` (1 to 9; 1 for paragraphs in no list),
//! `leftIndent` and `firstLineIndent` (pt), as `text.setFormat` takes them, `startAt` (the Numbers
//! tab's Start At box) and `startNumber` (its number, 1 to 999999999). The dialog also keeps the
//! values it opened with and the type and range it acts on, which are not fields.
//!
//! Start At is VectorCraft's own: on, OK gives the first numbered paragraph the number in the
//! field (`text.setFormat` `startNumber`), and the numbered paragraphs after it count on from it;
//! off, the paragraphs count on from the paragraph before (`restartNumbering` false). The box is on
//! when the paragraphs have a number of their own, off when none has, and blank when they differ.
//!
//! Behavior reference (public documentation):
//! <https://web.archive.org/web/20260417034638/https://helpx.adobe.com/illustrator/desktop/design-with-text/edit-format-text/create-bulleted-or-numbered-lists.html>.

use std::borrow::Cow;

use craft::behavior::{Mixed, Selection, SelectionChange, keyed};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, CollectionEvent, Entry, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::dialog::{Dialog, DialogId, DialogOutput, DialogResult, DialogStack, FieldRefusal, StackDialog, dialog_id};
use craft::ui::display::Text;
use craft::ui::form::{Field, FieldGroup};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::ThumbAspect;
use craft::ui::pickers::Select;
use craft::ui::primitives::{HAlign, IconSpec, TextRun};
use craft::ui::runtime::env::{Env, LengthUnits};
use craft::ui::runtime::words::CommonWords;
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::tabs::{Tab, TabKind, TabStrip};
use craft::ui::text::NumberField;
use craft::ui::tile_grid::{CaptionPlace, Tile, TileGrid};
use egui::{Context, Ui};
use serde_json::{Map, Value, json};
use vectorcraft_doc::text::{LIST_HANGING_INDENT, LIST_LEVELS, MAX_LIST_START, NumberStyle, ParaList};

use crate::VectorcraftApp;
use crate::panels::paragraph::BulletGlyph;

/// The dialog kind of Bullets and Numbering.
pub const KIND: &str = "bulletsAndNumbering";

const CMD: &str = "text.setFormat";

/// The `tab` values, in the order the tabs are shown.
const TABS: [&str; 2] = ["bullets", "numbers"];
/// The `listAlign` values, in the order Alignment lists them.
const ALIGNS: [&str; 2] = ["left", "right"];
/// The indents `text.setFormat` keeps (points): a value typed outside them becomes the nearer end.
const INDENTS: std::ops::RangeInclusive<f64> = -1296.0..=1296.0;

/// The fields `ui.dialog.set` can set while the dialog does not hold them: the dialog leaves a
/// field out where the paragraphs differ, and holds no `bullet` or `number` for paragraphs in no
/// list or in another kind of list.
const INPUTS: [&str; 9] = ["list", "bullet", "number", "listAlign", "listLevel", "leftIndent", "firstLineIndent", "startAt", "startNumber"];

/// The OK button's place among the footer buttons.
const OK: usize = 0;

/// The tab the dialog shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ListTab {
    Bullets,
    Numbers,
}

impl ListTab {
    /// The tab `key` (a [`TABS`] value) names.
    fn of(key: &str) -> Option<Self> {
        [ListTab::Bullets, ListTab::Numbers].into_iter().find(|t| t.key() == key)
    }

    /// The `tab` field's value.
    fn key(self) -> &'static str {
        match self {
            ListTab::Bullets => TABS[0],
            ListTab::Numbers => TABS[1],
        }
    }
}

/// What a preset tile draws.
#[derive(Clone)]
enum Face {
    /// A bullet's glyph.
    Glyph(&'static BulletGlyph),
    /// The first two numbers of a number style.
    Text(String),
}

/// One tile of a preset row.
struct Preset {
    /// The tile's place in the row, its key.
    at: usize,
    /// The tile's name: its tooltip and accessible name.
    name: String,
    face: Face,
}

/// The open Bullets and Numbering dialog: the values it shows, the values it opened with and the type it acts on.
pub(crate) struct BulletsDialog {
    tab: ListTab,
    /// The fields of the module doc; a field the paragraphs differ in is absent.
    fields: Map<String, Value>,
    /// The values the dialog opened with.
    initial: Map<String, Value>,
    /// The `ids`, `start` and `end` the dialog's `text.setFormat` acts on.
    targets: Value,
    /// Set while the indent fields hold the hanging indent a preset filled in ([`pick`]).
    hanging: bool,
    /// The General unit the indent fields show; the pump keeps it current.
    lengths: LengthUnits,
    open: bool,
}

/// The kind of list `list` is, as the `list` field names it.
fn kind(list: Option<ParaList>) -> &'static str {
    match list {
        None => "none",
        Some(l) if l.number.is_some() => "number",
        Some(_) => "bullet",
    }
}

/// Open Bullets and Numbering on the paragraph attributes the Paragraph panel shows, on tab `tab`
/// (`bullets` or `numbers`; without it, Numbers when the paragraphs are a numbered list).
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    use crate::panels::paragraph::{shared_para, shared_para_field};
    let key = "tab";
    let tab = match p.get(key) {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_str().and_then(ListTab::of).ok_or_else(|| format!("`{key}` must be one of {}", TABS.join(", ")))?),
    };
    crate::panels::character::text_style(app).ok_or("select some type first")?;
    // A dialog this one replaces leaves its preview on the type: end it, so the values read below are the document's own.
    std::mem::take(&mut app.bullets_preview).cancel(app);
    // While typing, the dialog acts on the paragraphs of the text being edited.
    let mut targets = json!({});
    if let Some((id, a, b)) = crate::panels::character::text_editing_kept(app)? {
        targets = json!({"ids": [id.0], "start": a, "end": b});
    }
    let mut values = Map::new();
    let list = shared_para(app, |p| kind(p.list));
    if let Some(k) = list {
        values.insert("list".into(), json!(k));
    }
    if let Some(Some(b)) = shared_para(app, |p| p.list.filter(|l| l.number.is_none()).map(|l| l.bullet)) {
        values.insert("bullet".into(), json!(b));
    }
    if let Some(Some(n)) = shared_para(app, |p| p.list.and_then(|l| l.number)) {
        values.insert("number".into(), json!(n));
    }
    // A paragraph in no list counts as aligned left, the alignment a list starts with.
    if let Some(a) = shared_para(app, |p| p.list.map(|l| l.align).unwrap_or_default()) {
        values.insert("listAlign".into(), json!(a));
    }
    // …and the paragraphs' list level, 1 for a paragraph in no list (the level a list starts at).
    if let Some(l) = shared_para(app, |p| p.list.map_or(1, |l| l.depth())) {
        values.insert("listLevel".into(), json!(l));
    }
    if let Some(v) = shared_para_field(app, |p| p.left_indent) {
        values.insert("leftIndent".into(), json!(v));
    }
    if let Some(v) = shared_para_field(app, |p| p.first_line_indent) {
        values.insert("firstLineIndent".into(), json!(v));
    }
    // …and Start At: on with the number when the paragraphs share one, off when none has one.
    if let Some(start) = shared_para(app, |p| p.list.and_then(|l| l.start)) {
        values.insert("startAt".into(), json!(start.is_some()));
        if let Some(n) = start {
            values.insert("startNumber".into(), json!(n));
        }
    }
    let tab = tab.unwrap_or(if list == Some("number") { ListTab::Numbers } else { ListTab::Bullets });
    let host = app.dialogs().ok_or("no window")?;
    let dialog = BulletsDialog {
        tab,
        fields: values.clone(),
        initial: values,
        targets,
        hanging: false,
        lengths: crate::canvas::general_lengths(app),
        open: true,
    };
    host.with(|h| h.open(dialog));
    Ok(Value::Null)
}

impl BulletsDialog {
    /// The tabs, the Level field, the presets of the shown tab and the Position group.
    fn body(&mut self, ui: &mut Ui) {
        Stack::new().show(ui, |ui, _| {
            self.tabs(ui);
            Field::new(tl!("Level:")).id_salt("listLevel").show(ui, |ui, _| self.level(ui));
            match self.tab {
                ListTab::Bullets => self.bullet_tab(ui),
                ListTab::Numbers => self.number_tab(ui),
            }
            FieldGroup::new(tl!("Position")).show(ui, |ui, _| {
                Field::new(tl!("Alignment:")).id_salt("listAlign").show(ui, |ui, _| self.alignment(ui));
                Field::new(tl!("Left Indent:")).id_salt("leftIndent").show(ui, |ui, _| self.indent(ui, "leftIndent", tl!("Left Indent:")));
                Field::new(tl!("First Line Indent:"))
                    .id_salt("firstLineIndent")
                    .show(ui, |ui, _| self.indent(ui, "firstLineIndent", tl!("First Line Indent:")));
            });
        });
    }

    /// The Bullets and Numbers tabs.
    fn tabs(&mut self, ui: &mut Ui) {
        let tabs = [(ListTab::Bullets, tl!("Bullets")), (ListTab::Numbers, tl!("Numbers"))];
        let source = keyed(&tabs[..], |tab: &(ListTab, &'static str)| tab.0).with_text(|tab| Cow::Borrowed(tab.1));
        let out = TabStrip::new(tl!("Bullets and Numbering")).kind(TabKind::Text).shown(self.tab).natural_height(true).show_items(
            ui,
            &source,
            &mut |key, _| Tab::new(tabs.iter().find(|tab| tab.0 == *key).map_or("", |tab| tab.1)),
        );
        for event in out.response.events {
            if let CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } = event
                && let Some(&picked) = keys.last()
            {
                self.tab = picked;
            }
        }
    }

    /// The Level field: the paragraphs' list level, 1 to 9, blank where they differ.
    fn level(&mut self, ui: &mut Ui) {
        let value = self.fields.get("listLevel").and_then(Value::as_f64);
        let edit = NumberField::new(tl!("Level:"), Bind::given(value)).range(1.0..=LIST_LEVELS as f64).commit_decimals(0).show(ui);
        if let Some(Some(v)) = edit.value().copied() {
            self.fields.insert("listLevel".into(), json!(v));
        }
    }

    /// The Bullets tab: the bullet presets in a row, None first, the one the dialog applies selected.
    fn bullet_tab(&mut self, ui: &mut Ui) {
        let presets = crate::panels::paragraph::bullet_presets();
        let shown = self.fields.get("bullet").and_then(Value::as_str);
        let current = match self.fields.get("list").and_then(Value::as_str) {
            Some("none") => Some(0),
            Some("bullet") => presets.iter().position(|(_, key, _)| shown == Some(*key)),
            _ => None,
        };
        Text::new(tl!("Bullet Character:")).show(ui);
        let tiles: Vec<Preset> = presets
            .iter()
            .enumerate()
            .map(|(at, &(name, _, bullet))| Preset { at, name: name.to_owned(), face: Face::Glyph(BulletGlyph::of(bullet)) })
            .collect();
        let picked = preset_row(ui, tl!("Bullet Character:"), &tiles, current);
        match picked.and_then(|i| presets.get(i)) {
            Some((_, _, None)) => pick(self, "none", None),
            Some((_, key, _)) => pick(self, "bullet", Some(*key)),
            None => {}
        }
    }

    /// The Numbers tab: None and the number styles in a row, each shown by its first two numbers, the
    /// one the dialog applies selected.
    fn number_tab(&mut self, ui: &mut Ui) {
        let shown = self.fields.get("number").and_then(|v| serde_json::from_value::<NumberStyle>(v.clone()).ok());
        let current = match self.fields.get("list").and_then(Value::as_str) {
            Some("none") => Some(0),
            Some("number") => shown.and_then(|s| NumberStyle::ALL.iter().position(|&n| n == s)).map(|i| i + 1),
            _ => None,
        };
        Text::new(tl!("Number Style:")).show(ui);
        // Screen readers name each style by its first three numbers, as the Numbered List presets do.
        let none = Preset { at: 0, name: tl!("None").to_owned(), face: Face::Glyph(BulletGlyph::of(None)) };
        let styles = NumberStyle::ALL.iter().enumerate().map(|(i, &style)| Preset {
            at: i + 1,
            name: crate::panels::paragraph::number_sample(style, 3),
            face: Face::Text(format!("{} {}", style.format(1), style.format(2))),
        });
        let tiles: Vec<Preset> = std::iter::once(none).chain(styles).collect();
        match preset_row(ui, tl!("Number Style:"), &tiles, current) {
            Some(0) => pick(self, "none", None),
            Some(i) => {
                if let Some(key) = NumberStyle::ALL.get(i - 1).and_then(|s| serde_json::to_value(s).ok()).and_then(|v| v.as_str().map(str::to_string))
                {
                    pick(self, "number", Some(&key));
                }
            }
            None => {}
        }
        self.start_at(ui);
    }

    /// The Start At box and its number, 1 to [`MAX_LIST_START`]: the box shows the framework's mixed
    /// state while the paragraphs' numbers differ, and a click then turns it on; the number field is
    /// dimmed unless the box is on, and shows 1 until a number is typed.
    fn start_at(&mut self, ui: &mut Ui) {
        Row::new().show(ui, |ui, _| {
            let on = self.fields.get("startAt").and_then(Value::as_bool);
            let edit = Checkbox::new(Bind::given(on.map_or(Mixed::Mixed, Mixed::Same))).label(tl!("Start at:")).id_salt("startAt").show(ui);
            if let Some(&v) = edit.value() {
                self.fields.insert("startAt".into(), json!(v));
            }
            let value = self.fields.get("startNumber").and_then(Value::as_f64).unwrap_or(1.0);
            let edit = NumberField::new(tl!("Start at:"), Bind::given(Some(value)))
                .range(1.0..=f64::from(MAX_LIST_START))
                .commit_decimals(0)
                .enabled(on == Some(true))
                .id_salt("startNumber")
                .show(ui);
            if let Some(Some(v)) = edit.value().copied() {
                self.fields.insert("startNumber".into(), json!(v));
            }
        });
    }

    /// Alignment: Left or Right, the framework's mixed word where the paragraphs differ.
    fn alignment(&mut self, ui: &mut Ui) {
        let entries = [Entry::new(0usize, tl!("Left")), Entry::new(1, tl!("Right"))];
        let current = self.fields.get("listAlign").and_then(Value::as_str).and_then(|a| ALIGNS.iter().position(|x| *x == a));
        let edit = Select::new(tl!("Alignment:"), Bind::given(current.map_or(Mixed::Mixed, Mixed::Same)), &entries).show(ui);
        if let Some(&i) = edit.value()
            && let Some(a) = ALIGNS.get(i)
        {
            self.fields.insert("listAlign".into(), json!(a));
        }
    }

    /// An indent field (`leftIndent` or `firstLineIndent`), in the General unit, blank where the paragraphs differ.
    fn indent(&mut self, ui: &mut Ui, key: &'static str, label: &'static str) {
        let value = self.fields.get(key).and_then(Value::as_f64);
        let edit = NumberField::length(label, Bind::given(value), self.lengths).range(INDENTS).show(ui);
        if let Some(Some(v)) = edit.value().copied() {
            self.fields.insert(key.into(), json!(v));
        }
    }
}

/// A row of preset tiles named `label`, `current` selected, one tile of each of `presets`. Returns the tile the user
/// picked this pass, never the one already selected.
fn preset_row(ui: &mut Ui, label: &'static str, presets: &[Preset], current: Option<usize>) -> Option<usize> {
    let mut selection = current.map_or_else(Selection::default, Selection::single);
    let source = keyed(presets, |p: &Preset| p.at);
    let grid = TileGrid::new(label).select(SelectMode::Single).caption(CaptionPlace::None).natural_height(true);
    let _ = grid.selection(Bind::bound(&mut selection)).show_items(ui, &source, &mut |at, _| {
        let Some(preset) = presets.get(*at) else { return Tile::new(String::new()) };
        let face = preset.face.clone();
        Tile::new(preset.name.clone()).thumb(ThumbAspect::SQUARE, move |cx, rect| {
            cx.with_draw(|draw| match &face {
                Face::Glyph(glyph) => draw.icon(rect, &IconSpec::glyph(*glyph)),
                Face::Text(text) => {
                    let _ = draw.text(rect, &TextRun::new(text.as_str()).align(HAlign::Center));
                }
            })
        })
    });
    selection.lead.filter(|at| Some(*at) != current)
}

/// Pick a preset of kind `list` (`none`, `bullet` or `number`) with key `key` (for `bullet` or
/// `number`). For paragraphs that were in no list and had no indents, the indent fields take the
/// hanging indent `text.setFormat` gives a paragraph that joins a list; None puts them back while
/// they still hold it.
fn pick(d: &mut BulletsDialog, list: &str, key: Option<&str>) {
    d.fields.insert("list".into(), json!(list));
    if let Some(key) = key {
        d.fields.insert(list.into(), json!(key));
    }
    let initial = d.initial.clone();
    let was = |k: &str| initial.get(k).and_then(Value::as_f64);
    let now = |d: &BulletsDialog| (d.fields.get("leftIndent").and_then(Value::as_f64), d.fields.get("firstLineIndent").and_then(Value::as_f64));
    let hanging = (Some(LIST_HANGING_INDENT), Some(-LIST_HANGING_INDENT));
    if list != "none" {
        let unlisted = initial.get("list").and_then(Value::as_str) == Some("none");
        if unlisted && (was("leftIndent"), was("firstLineIndent")) == (Some(0.0), Some(0.0)) && now(d) == (Some(0.0), Some(0.0)) {
            d.fields.insert("leftIndent".into(), json!(LIST_HANGING_INDENT));
            d.fields.insert("firstLineIndent".into(), json!(-LIST_HANGING_INDENT));
            d.hanging = true;
        }
    } else if std::mem::take(&mut d.hanging) && now(d) == hanging {
        d.fields.insert("leftIndent".into(), json!(0.0));
        d.fields.insert("firstLineIndent".into(), json!(0.0));
    }
}

/// `text.setFormat` parameters: the fields that differ from the values the dialog opened with,
/// on the dialog's type and range. With a preset that puts the paragraphs in a list, the indents
/// the dialog shows go too, so that the hanging indent `text.setFormat` gives a paragraph joining a
/// list does not replace them. Alignment and Level go only when the paragraphs are in a list or
/// the preset puts them in one.
fn params(d: &BulletsDialog) -> Map<String, Value> {
    let initial = &d.initial;
    let was = |key: &str| initial.get(key).and_then(Value::as_str);
    let field = |key: &str| d.fields.get(key).and_then(Value::as_str);
    let mut p = d.targets.as_object().cloned().unwrap_or_default();
    match field("list") {
        Some("none") if was("list") != Some("none") => {
            p.insert("list".into(), json!("none"));
        }
        Some(kind @ ("bullet" | "number")) => match field(kind) {
            Some(key) if was("list") != Some(kind) || was(kind) != Some(key) => {
                p.insert(kind.into(), json!(key));
            }
            None if was("list") != Some(kind) => {
                p.insert("list".into(), json!(kind));
            }
            _ => {}
        },
        _ => {}
    }
    let listing = ["bullet", "number"].iter().any(|k| p.contains_key(*k)) || p.get("list").is_some_and(|l| l.as_str() != Some("none"));
    // `text.setFormat` sets `listAlign` only on paragraphs in a list.
    if let Some(a) = field("listAlign")
        && was("listAlign") != Some(a)
        && (listing || was("list") != Some("none"))
    {
        p.insert("listAlign".into(), json!(a));
    }
    // …and `listLevel` too.
    if let Some(l) = d.fields.get("listLevel").and_then(Value::as_f64)
        && initial.get("listLevel").and_then(Value::as_f64) != Some(l)
        && (listing || was("list") != Some("none"))
    {
        p.insert("listLevel".into(), json!(l));
    }
    // …and Start At, unless the paragraphs are made bulleted or taken out of their list: on, the
    // number shown; off, the paragraphs count on from the one before.
    if let Some(on) = d.fields.get("startAt").and_then(Value::as_bool)
        && !matches!(field("list"), Some("none" | "bullet"))
    {
        let n = d.fields.get("startNumber").and_then(Value::as_f64).unwrap_or(1.0);
        let was_on = initial.get("startAt").and_then(Value::as_bool);
        let was_n = initial.get("startNumber").and_then(Value::as_f64);
        if on && (was_on != Some(true) || was_n != Some(n)) {
            p.insert("startNumber".into(), json!(n));
        } else if !on && was_on != Some(false) {
            p.insert("restartNumbering".into(), json!(false));
        }
    }
    // For paragraphs in a list and paragraphs in none, an indent left as it was is not sent:
    // `text.setFormat` then gives each paragraph that joins the list its hanging indent.
    let mixed = !initial.contains_key("list");
    for key in ["leftIndent", "firstLineIndent"] {
        if !d.fields.contains_key(key) {
            continue;
        }
        // A length typed with its unit ("6 mm") is sent in points.
        let v = d.fields.get(key).and_then(Value::as_f64).unwrap_or(0.0);
        let changed = initial.get(key).and_then(Value::as_f64) != Some(v);
        if changed || (listing && !mixed) {
            p.insert(key.into(), json!(v));
        }
    }
    p
}

/// Does `p` change anything (more than the targets)?
fn changes(p: &Map<String, Value>) -> bool {
    p.keys().any(|k| !matches!(k.as_str(), "ids" | "start" | "end"))
}

impl StackDialog for BulletsDialog {
    /// The `text.setFormat` parameters OK runs.
    type Result = Value;

    fn id(&self) -> DialogId {
        dialog_id(KIND)
    }

    fn key(&self) -> Option<&str> {
        Some(KIND)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Value>> {
        let common = Env::of(ui).words::<CommonWords>();
        let buttons = [FooterButton::new(common.ok.clone(), DialogRole::Accept), FooterButton::new(common.cancel.clone(), DialogRole::Cancel)];
        let mut open = self.open;
        let dialog = Dialog::new(tl!("Bullets and Numbering")).buttons(buttons).opened(Bind::bound(&mut open));
        let shown = dialog.show(ui, |ui, _| self.body(ui));
        self.open = open;
        let DialogOutput { closed, picked, .. } = shown.output;
        closed?;
        Some(if picked == Some(OK) { DialogResult::Ok(Value::Object(params(self))) } else { DialogResult::Cancel })
    }

    fn confirm(&mut self, _: &Context) -> Option<Value> {
        Some(Value::Object(params(self)))
    }

    fn json_fields(&self) -> Option<Value> {
        let mut fields = self.fields.clone();
        fields.insert("tab".into(), json!(self.tab.key()));
        Some(Value::Object(fields))
    }

    fn set_json_field(&mut self, _: &Context, field: &str, value: Value) -> Result<(), String> {
        if field == "tab" {
            self.tab = value.as_str().and_then(ListTab::of).ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
        } else if !INPUTS.contains(&field) {
            return Err(FieldRefusal::Unknown.message(field));
        } else if value.is_null() {
            return Err(FieldRefusal::Rejects(&value).message(field));
        } else {
            self.fields.insert(field.into(), value);
        }
        Ok(())
    }
}

/// Runs once per frame: keeps the indent fields' unit current, shows the dialog's values on the type while they change
/// something, and answers a finished dialog: OK keeps the preview as one undo step, Cancel (Escape and the close box
/// too) and an OK that changes nothing put the type back.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &Context) {
    let Some(host) = app.dialogs() else { return };
    if let Some(result) = host.with(|h| h.take::<BulletsDialog>()) {
        let live = std::mem::take(&mut app.bullets_preview);
        match result {
            DialogResult::Ok(Value::Object(p)) if changes(&p) => {
                if let Err(e) = live.commit(app, CMD, Value::Object(p)) {
                    app.status(e);
                    // The dialog is gone: a preview the refused OK left running goes with it. Nothing runs
                    // when the refusal came before the interaction began, which is the right outcome too.
                    let _ = app.session.cancel_interaction();
                }
            }
            _ => live.cancel(app),
        }
    }
    let lengths = crate::canvas::general_lengths(app);
    // The host's lock is released before the preview runs: a command never runs inside `with`.
    let Some(p) = host.with(|h| {
        h.get_mut::<BulletsDialog>().map(|d| {
            d.lengths = lengths;
            params(d)
        })
    }) else {
        return;
    };
    let mut live = std::mem::take(&mut app.bullets_preview);
    let _ = live.run(app, changes(&p), tl!("Bullets and Numbering"), CMD, &Value::Object(p));
    app.bullets_preview = live;
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_doc::text::{Bullet, ListAlign};
    use vectorcraft_engine::Session;
    use vectorcraft_engine::doc::{NodeId, NodeKind, ParaStyle};

    /// A selected area type object with two paragraphs, in an app with a window for its dialogs.
    fn app() -> (VectorcraftApp, u64) {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let id = app.run("text.create", json!({"x": 10, "y": 10, "text": "One\nTwo", "area": {"width": 200, "height": 100}})).unwrap()["id"]
            .as_u64()
            .unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        (app, id)
    }

    fn paras(app: &VectorcraftApp, id: u64) -> (ParaStyle, ParaStyle) {
        match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => (t.para_at(0).clone(), t.para_at(1).clone()),
            _ => panic!("not text"),
        }
    }

    /// The first paragraph's list and indents.
    fn first(app: &VectorcraftApp, id: u64) -> (Option<ParaList>, f64, f64) {
        let p = paras(app, id).0;
        (p.list, p.left_indent, p.first_line_indent)
    }

    /// The context the app's dialogs open on.
    fn window(app: &VectorcraftApp) -> Context {
        app.ui_ctx.clone().expect("the app has a window")
    }

    /// Runs `f` on the open dialog.
    fn with_dialog<R>(app: &VectorcraftApp, f: impl FnOnce(&mut BulletsDialog) -> R) -> R {
        app.dialogs().expect("a window").with(|h| f(h.get_mut::<BulletsDialog>().expect("Bullets and Numbering is open")))
    }

    fn is_open(app: &VectorcraftApp) -> bool {
        app.dialogs().expect("a window").with(|h| h.is_open::<BulletsDialog>())
    }

    /// The open dialog's fields, as automation reads them.
    fn fields(app: &VectorcraftApp) -> Value {
        with_dialog(app, |d| d.json_fields()).expect("the dialog lists its fields")
    }

    /// The open dialog's id and key.
    fn top(app: &VectorcraftApp) -> (DialogId, Option<String>) {
        let open = app.dialogs().expect("a window").with(|h| h.open_dialogs());
        let top = open.last().expect("a dialog is open");
        (top.id, top.key.clone())
    }

    /// Sets field `key` of the open dialog, as automation does.
    fn set(app: &mut VectorcraftApp, key: &str, v: Value) {
        let (ctx, (id, _)) = (window(app), top(app));
        app.dialogs().expect("a window").with(|h| h.set_json_field(&ctx, id, key, v)).unwrap_or_else(|e| panic!("{key}: {e}"));
    }

    /// One frame's pump.
    fn frame(app: &mut VectorcraftApp) {
        let ctx = window(app);
        crate::dialogs::host::pump(app, &ctx);
    }

    /// OK on the open dialog, as automation answers it, then the frame's pump.
    fn ok(app: &mut VectorcraftApp) {
        let (ctx, (id, _)) = (window(app), top(app));
        assert!(app.dialogs().expect("a window").with(|h| h.confirm_by_id(&ctx, id)), "OK is enabled");
        frame(app);
    }

    /// Cancel on the open dialog, as Esc gives it, then the frame's pump.
    fn cancel(app: &mut VectorcraftApp) {
        let (ctx, (id, _)) = (window(app), top(app));
        assert!(app.dialogs().expect("a window").with(|h| h.cancel_by_id(&ctx, id)));
        frame(app);
    }

    fn pick_preset(app: &mut VectorcraftApp, list: &str, key: Option<&str>) {
        with_dialog(app, |d| pick(d, list, key));
    }

    /// The `text.setFormat` parameters the open dialog would send.
    fn sent(app: &VectorcraftApp) -> Map<String, Value> {
        with_dialog(app, |d| params(d))
    }

    /// `ui.bulletsAndNumbering` opens on the selected type's values; a preset, Alignment and the
    /// indents show on the type at once, and OK keeps them as one `text.setFormat`, which one Undo
    /// takes back.
    #[test]
    fn ok_applies_the_marker_alignment_and_indents_as_one_undo_step() {
        let (mut app, id) = app();
        let undo = app.session.active().unwrap().history.undo.len();
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        assert_eq!(top(&app).1.as_deref(), Some(KIND));
        let f = fields(&app);
        assert_eq!((f["tab"].as_str(), f["list"].as_str(), f["listAlign"].as_str()), (Some("bullets"), Some("none"), Some("left")));
        assert_eq!((f["leftIndent"].as_f64(), f["firstLineIndent"].as_f64()), (Some(0.0), Some(0.0)));
        pick_preset(&mut app, "bullet", Some("square"));
        set(&mut app, "listAlign", json!("right"));
        set(&mut app, "leftIndent", json!(24));
        set(&mut app, "firstLineIndent", json!(-12));
        frame(&mut app);
        assert!(app.session.in_interaction(), "the preview runs");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo, "the preview adds no undo step");
        let want = Some(ParaList::bulleted(Bullet::Square).aligned(ListAlign::Right));
        assert_eq!(first(&app, id), (want, 24.0, -12.0), "previewed on the type");
        ok(&mut app);
        assert!(!is_open(&app) && !app.session.in_interaction());
        assert_eq!((first(&app, id), paras(&app, id).1.list), ((want, 24.0, -12.0), want));
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "one undo step");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(first(&app, id), (None, 0.0, 0.0));
        // The Numbers tab's presets make a numbered list, keeping the alignment the dialog shows.
        app.run("text.setFormat", json!({"list": "bullet", "listAlign": "right"})).unwrap();
        app.run("ui.bulletsAndNumbering", json!({"tab": "numbers"})).unwrap();
        assert_eq!(fields(&app)["tab"], "numbers");
        pick_preset(&mut app, "number", Some("upperRoman"));
        ok(&mut app);
        assert_eq!(first(&app, id).0, Some(ParaList::numbered(NumberStyle::UpperRoman).aligned(ListAlign::Right)));
        // OK with nothing changed adds no undo step, and an unknown tab is an error.
        let undo = app.session.active().unwrap().history.undo.len();
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        assert_eq!(fields(&app)["tab"], "numbers", "a numbered list opens on Numbers");
        frame(&mut app);
        ok(&mut app);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
        assert!(app.run("ui.bulletsAndNumbering", json!({"tab": "levels"})).is_err());
        assert!(!is_open(&app));
        app.run("select.set", json!({"ids": []})).unwrap();
        assert!(app.run("ui.bulletsAndNumbering", json!({})).is_err(), "no type selected");
    }

    /// Cancel puts back what the preview changed: the list, its alignment and the indents.
    #[test]
    fn cancel_puts_the_type_back_as_it_was() {
        let (mut app, id) = app();
        app.run("text.setFormat", json!({"number": "arabic"})).unwrap();
        let before = first(&app, id);
        assert_eq!(before, (Some(ParaList::numbered(NumberStyle::Arabic)), 18.0, -18.0));
        let undo = app.session.active().unwrap().history.undo.len();
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        pick_preset(&mut app, "number", Some("lowerLetters"));
        set(&mut app, "listAlign", json!("right"));
        set(&mut app, "leftIndent", json!(30));
        frame(&mut app);
        assert_eq!(first(&app, id), (Some(ParaList::numbered(NumberStyle::LowerLetters).aligned(ListAlign::Right)), 30.0, -18.0));
        // Another change previews again, still without an undo step.
        set(&mut app, "leftIndent", json!(36));
        frame(&mut app);
        assert_eq!(first(&app, id).1, 36.0);
        set(&mut app, "leftIndent", json!(30));
        frame(&mut app);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo, "the preview adds no undo step");
        // Changed back in the dialog, the type is as it was.
        set(&mut app, "listAlign", json!("left"));
        set(&mut app, "leftIndent", json!(18));
        pick_preset(&mut app, "number", Some("arabic"));
        frame(&mut app);
        assert!(!app.session.in_interaction(), "no preview while nothing changes");
        assert_eq!(first(&app, id), before);
        pick_preset(&mut app, "none", None);
        frame(&mut app);
        assert_eq!(first(&app, id).0, None, "None previews the list off");
        cancel(&mut app);
        assert!(!is_open(&app));
        assert_eq!(first(&app, id), before);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo, "Cancel leaves no undo step");
    }

    /// A preset picked for paragraphs in no list and without indents fills in the 18 pt and
    /// -18 pt hanging indent, and None takes it out again; indents that are set stay, and so do
    /// indents set back to 0 with the preset.
    #[test]
    fn a_preset_fills_in_the_hanging_indent_a_new_list_gets() {
        let (mut app, id) = app();
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        pick_preset(&mut app, "bullet", Some("dot"));
        let f = fields(&app);
        assert_eq!((f["leftIndent"].as_f64(), f["firstLineIndent"].as_f64()), (Some(18.0), Some(-18.0)));
        pick_preset(&mut app, "number", Some("arabic"));
        pick_preset(&mut app, "none", None);
        let f = fields(&app);
        assert_eq!((f["leftIndent"].as_f64(), f["firstLineIndent"].as_f64()), (Some(0.0), Some(0.0)));
        assert!(!changes(&sent(&app)));
        cancel(&mut app);
        app.run("text.setFormat", json!({"leftIndent": 10})).unwrap();
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        pick_preset(&mut app, "bullet", Some("dash"));
        assert_eq!(fields(&app)["leftIndent"].as_f64(), Some(10.0));
        ok(&mut app);
        assert_eq!(first(&app, id), (Some(ParaList::bulleted(Bullet::Dash)), 10.0, 0.0));
        // Indents set back to 0 with the preset are kept: the hanging indent does not replace them.
        let (mut app, id) = self::app();
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        pick_preset(&mut app, "number", Some("arabic"));
        set(&mut app, "leftIndent", json!(0));
        set(&mut app, "firstLineIndent", json!(0));
        frame(&mut app);
        assert_eq!(first(&app, id), (Some(ParaList::numbered(NumberStyle::Arabic)), 0.0, 0.0), "previewed as shown");
        ok(&mut app);
        assert_eq!(first(&app, id), (Some(ParaList::numbered(NumberStyle::Arabic)), 0.0, 0.0));
    }

    /// Paragraphs that differ leave the preset unpicked and the fields blank, and OK sends only
    /// what was changed: each paragraph keeps its own indents and list.
    #[test]
    fn differing_paragraphs_show_blank_fields_and_keep_what_is_left_alone() {
        let (mut app, id) = app();
        app.run("text.setFormat", json!({"ids": [id], "bullet": "circle", "leftIndent": 30, "start": 0, "end": 0})).unwrap();
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        let f = fields(&app);
        for key in ["list", "bullet", "leftIndent"] {
            assert!(f.get(key).is_none(), "{key} is blank");
        }
        assert_eq!((f["listAlign"].as_str(), f["firstLineIndent"].as_f64()), (Some("left"), Some(0.0)));
        set(&mut app, "listAlign", json!("right"));
        ok(&mut app);
        let (one, two) = paras(&app, id);
        assert_eq!((one.list, one.left_indent), (Some(ParaList::bulleted(Bullet::Circle).aligned(ListAlign::Right)), 30.0));
        assert_eq!((two.list, two.left_indent), (None, 0.0), "the paragraph in no list stays out of one");
    }

    /// A preset picked for a list paragraph and a paragraph in no list, both without indents, gives
    /// the paragraph that joins the list the hanging indent, and the list paragraph keeps its indents.
    #[test]
    fn a_preset_for_listed_and_unlisted_paragraphs_indents_the_one_joining_the_list() {
        let (mut app, id) = app();
        app.run("text.setFormat", json!({"ids": [id], "bullet": "circle", "leftIndent": 0, "firstLineIndent": 0, "start": 0, "end": 0})).unwrap();
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        let f = fields(&app);
        assert!(f.get("list").is_none(), "the list kinds differ");
        assert_eq!((f["leftIndent"].as_f64(), f["firstLineIndent"].as_f64()), (Some(0.0), Some(0.0)));
        pick_preset(&mut app, "bullet", Some("dot"));
        frame(&mut app);
        assert_eq!(paras(&app, id).1.left_indent, 18.0, "previewed with the hanging indent");
        ok(&mut app);
        let (one, two) = paras(&app, id);
        let dot = Some(ParaList::bulleted(Bullet::Dot));
        assert_eq!((one.list, one.left_indent, one.first_line_indent), (dot, 0.0, 0.0));
        assert_eq!((two.list, two.left_indent, two.first_line_indent), (dot, 18.0, -18.0));
    }

    /// Alignment changed for paragraphs that stay in no list sends nothing: there is no preview,
    /// and OK adds no undo step. A preset picked with it takes the alignment.
    #[test]
    fn alignment_alone_on_paragraphs_in_no_list_adds_no_undo_step() {
        let (mut app, id) = app();
        let undo = app.session.active().unwrap().history.undo.len();
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        set(&mut app, "listAlign", json!("right"));
        assert!(!changes(&sent(&app)));
        frame(&mut app);
        assert!(!app.session.in_interaction(), "no preview");
        ok(&mut app);
        assert!(!is_open(&app));
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo, "no undo step");
        assert_eq!(first(&app, id), (None, 0.0, 0.0));
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        set(&mut app, "listAlign", json!("right"));
        pick_preset(&mut app, "bullet", Some("dot"));
        ok(&mut app);
        assert_eq!(first(&app, id).0, Some(ParaList::bulleted(Bullet::Dot).aligned(ListAlign::Right)));
    }

    /// `ui.dialog.set` sets the fields the dialog does not hold: a bullet or a number for type in
    /// no list, and the fields left blank where the paragraphs differ.
    #[test]
    fn the_control_channel_sets_fields_the_dialog_does_not_hold() {
        let (mut app, id) = app();
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        for key in ["bullet", "number"] {
            assert!(fields(&app).get(key).is_none(), "{key} is not held");
        }
        set(&mut app, "list", json!("number"));
        set(&mut app, "number", json!("upperRoman"));
        ok(&mut app);
        assert_eq!(first(&app, id).0, Some(ParaList::numbered(NumberStyle::UpperRoman)));
        app.run("text.setFormat", json!({"ids": [id], "bullet": "circle", "leftIndent": 30, "firstLineIndent": -6, "start": 0, "end": 0})).unwrap();
        app.run("text.setFormat", json!({"ids": [id], "listAlign": "right", "listLevel": 2, "start": 0, "end": 0})).unwrap();
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        let held = fields(&app);
        // Neither paragraph has a number of its own, so Start at is off, not blank.
        for key in INPUTS.into_iter().filter(|k| !["startAt", "startNumber"].contains(k)) {
            assert!(held.get(key).is_none(), "{key} is blank");
        }
        for (key, value) in [
            ("list", json!("bullet")),
            ("bullet", json!("square")),
            ("listAlign", json!("left")),
            ("listLevel", json!(3)),
            ("leftIndent", json!(24)),
            ("firstLineIndent", json!(-12)),
        ] {
            set(&mut app, key, value);
        }
        ok(&mut app);
        let (one, two) = paras(&app, id);
        let square = Some(ParaList::bulleted(Bullet::Square).at_level(3));
        assert_eq!((one.list, one.left_indent, one.first_line_indent), (square, 24.0, -12.0));
        assert_eq!((two.list, two.left_indent, two.first_line_indent), (square, 24.0, -12.0));
    }

    /// The Level field opens on the paragraphs' list level (1 for paragraphs in no list, blank
    /// where they differ), shows a change on the type at once, and OK sends it, with a preset or
    /// alone.
    #[test]
    fn the_level_field_sets_the_list_level() {
        let (mut app, id) = app();
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        assert_eq!(fields(&app)["listLevel"].as_f64(), Some(1.0), "in no list: level 1");
        pick_preset(&mut app, "number", Some("arabic"));
        set(&mut app, "listLevel", json!(3));
        frame(&mut app);
        assert_eq!(first(&app, id).0, Some(ParaList::numbered(NumberStyle::Arabic).at_level(3)), "previewed on the type");
        ok(&mut app);
        assert_eq!(paras(&app, id).1.list, Some(ParaList::numbered(NumberStyle::Arabic).at_level(3)));
        app.run("text.setFormat", json!({"ids": [id], "listLevel": 5, "start": 0, "end": 0})).unwrap();
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        assert!(fields(&app).get("listLevel").is_none(), "levels 5 and 3 differ: blank");
        cancel(&mut app);
        app.run("text.setFormat", json!({"ids": [id], "listLevel": 5})).unwrap();
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        assert_eq!(fields(&app)["listLevel"].as_f64(), Some(5.0));
        assert!(!changes(&sent(&app)), "nothing changed yet");
        set(&mut app, "listLevel", json!(2));
        assert_eq!(sent(&app).get("listLevel"), Some(&json!(2.0)));
        ok(&mut app);
        assert_eq!((first(&app, id).0.map(|l| l.level), paras(&app, id).1.list.map(|l| l.level)), (Some(2), Some(2)));
    }

    /// Start at: the box opens off for paragraphs without a number of their own and on with the
    /// number for one that has it; a change shows on the type at once, OK keeps it as one undo
    /// step, Cancel puts the type back, and turning the box off sends `restartNumbering` false.
    #[test]
    fn start_at_gives_the_first_numbered_paragraph_its_number() {
        let (mut app, id) = app();
        app.run("text.setFormat", json!({"number": "arabic"})).unwrap();
        let undo = app.session.active().unwrap().history.undo.len();
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        let f = fields(&app);
        assert_eq!((f.get("startAt"), f.get("startNumber")), (Some(&json!(false)), None));
        set(&mut app, "startAt", json!(true));
        set(&mut app, "startNumber", json!(4));
        frame(&mut app);
        let starts = |app: &VectorcraftApp| {
            let (one, two) = paras(app, id);
            (one.list.and_then(|l| l.start), two.list.and_then(|l| l.start))
        };
        assert_eq!(starts(&app), (Some(4), None), "previewed on the type");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo, "the preview adds no undo step");
        cancel(&mut app);
        assert_eq!(starts(&app), (None, None), "Cancel puts it back");
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        set(&mut app, "startAt", json!(true));
        set(&mut app, "startNumber", json!(4));
        ok(&mut app);
        assert_eq!((starts(&app), app.session.active().unwrap().history.undo.len()), ((Some(4), None), undo + 1));
        // One paragraph with a number and one without: the box is blank, and OK sends nothing for it.
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        assert!(fields(&app).get("startAt").is_none() && !changes(&sent(&app)));
        // A preset that makes the paragraphs bulleted sends no number.
        set(&mut app, "startAt", json!(true));
        pick_preset(&mut app, "bullet", Some("dot"));
        let p = sent(&app);
        assert!(!p.contains_key("startNumber") && !p.contains_key("restartNumbering"));
        cancel(&mut app);
        // A paragraph with a number opens the box on with it, and off sends it back to counting on.
        let solo = app.run("text.create", json!({"x": 10, "y": 200, "text": "Solo"})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [solo]})).unwrap();
        app.run("text.setFormat", json!({"number": "arabic", "startNumber": 9})).unwrap();
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        let f = fields(&app);
        assert_eq!((f.get("startAt"), f["startNumber"].as_f64()), (Some(&json!(true)), Some(9.0)));
        assert!(!changes(&sent(&app)), "nothing changed yet");
        set(&mut app, "startAt", json!(false));
        assert_eq!(sent(&app).get("restartNumbering"), Some(&json!(false)));
        ok(&mut app);
        let start = match &app.session.active().unwrap().doc.node(NodeId(solo)).unwrap().kind {
            NodeKind::Text(t) => t.para_at(0).list.map(|l| l.start),
            _ => panic!("not text"),
        };
        assert_eq!(start, Some(None), "still numbered, with no number of its own");
    }

    /// `text.setFormat` sets `listLevel` only on paragraphs in a list, so a Level change alone on
    /// paragraphs in no list sends nothing and adds no undo step; with a preset it applies.
    #[test]
    fn level_alone_on_paragraphs_in_no_list_adds_no_undo_step() {
        let (mut app, id) = app();
        let undo = app.session.active().unwrap().history.undo.len();
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        set(&mut app, "listLevel", json!(3));
        assert!(!changes(&sent(&app)));
        frame(&mut app);
        assert!(!app.session.in_interaction(), "no preview");
        ok(&mut app);
        assert!(!is_open(&app));
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo, "no undo step");
        assert_eq!(first(&app, id), (None, 0.0, 0.0));
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        set(&mut app, "listLevel", json!(3));
        pick_preset(&mut app, "bullet", Some("dot"));
        ok(&mut app);
        assert_eq!(first(&app, id).0, Some(ParaList::bulleted(Bullet::Dot).at_level(3)));
    }
}
