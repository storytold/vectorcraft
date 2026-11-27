//! Type › Composite Fonts… (with Show East Asian Options): the active document's composite fonts,
//! the one picked in Composite Font shown as a table with a row for each class of character (Kanji,
//! Kana, Punctuation, Symbols, Roman, Numerals), then a row for each custom character set, with its
//! font, style, size, baseline and scales. The Composite Font row's New adds a font based on the one
//! picked, Delete takes it out, Rename renames it, Load… adds the fonts of a file, Replace… takes the
//! fonts of a file in place of those in the dialog and Save… writes the one picked to a file. Custom
//! shows the custom character sets of the font picked: the Character Set row's New adds a set based
//! on the one picked, Delete takes it out, and Add puts the characters typed (Direct Entry) or the
//! character of a Unicode code into it; Delete takes the character picked out. Show Sample draws a
//! sample of the font picked with lines that mark its metrics ([`sample`]). Nothing changes in the
//! document until OK, which runs `type.compositeFonts.set` with every font as one undo step; Cancel
//! leaves them as they were.
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

use std::cell::RefCell;
use std::ops::RangeInclusive;
use std::sync::Arc;

use craft::behavior::{ColumnKey, ColumnWidth, Selection, SelectionChange, keyed};
use craft::host::{PickRequest, Picked};
use craft::theme::variant::Tone;
use craft::ui::buttons::{Button, ToggleButton};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{
    Bind, CollectionEvent, ColumnSpec, Entry, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Sizing, clicked,
};
use craft::ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, FieldRefusal, HeldAnswer, HeldOk, StackDialog, dialog_id};
use craft::ui::layout_components::Row;
use craft::ui::list::{ListRow, ThumbSize};
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::patterns::presets::{
    NameCheck, NamedPresetEvent, NamedPresetRow, PickerField, PresetError, PresetExport, PresetFileEvent, PresetFileFailure, PresetFileRequest,
    PresetFiles, PresetPicker, PresetRow, PresetSource, PresetState, PresetWords,
};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::runtime::overlay::CloseReason;
use craft::ui::runtime::words::CommonWords;
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::table::Table;
use craft::ui::text::{NumberField, TextField};
use craft::ui::tile_grid::{Tile, TileGrid};
use egui::{Context, Ui};
use serde_json::{Map, Value, json};
use vectorcraft_doc::text::{COMPOSITE_BASELINE, COMPOSITE_SCALE, COMPOSITE_SIZE, CUSTOM_CHARS_MAX, CUSTOM_SETS_MAX};
use vectorcraft_doc::{CharClass, CompositeFont, CompositePart, CustomCharSet};
use vectorcraft_engine::cmd::compositefonts::{EXTS, file_bytes};
use vectorcraft_text::FontDb;

use crate::VectorcraftApp;
use crate::file_asks::{Owner, ask_while, open_params, save};
use crate::font_menu::{PickerHold, family_picker, family_sets};

mod sample;

/// The dialog kind of Composite Fonts: the host's key of the dialog.
pub const KIND: &str = "compositeFonts";

/// The command OK runs, and the menu item's.
pub const COMMAND: &str = "type.compositeFonts.set";

/// The command that reads the fonts of a file without adding them to the document.
const IMPORT: &str = "type.compositeFonts.import";

/// The purpose of the file picks, which keeps their last folder.
const PURPOSE: &str = "compositeFonts";
/// The rows of a custom set's characters shown before they scroll.
const CHAR_ROWS: u16 = 5;

/// The table's columns: the class (or the custom set's name) is the row's label.
const CLASS: ColumnKey = ColumnKey::new("class");
const FONT: ColumnKey = ColumnKey::new("font");
const STYLE: ColumnKey = ColumnKey::new("style");
const SIZE: ColumnKey = ColumnKey::new("size");
const BASELINE: ColumnKey = ColumnKey::new("baseline");
const H_SCALE: ColumnKey = ColumnKey::new("hScale");
const V_SCALE: ColumnKey = ColumnKey::new("vScale");
const CENTER: ColumnKey = ColumnKey::new("center");

/// How the Character row reads what is typed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum EntryMode {
    /// The characters typed.
    Direct,
    /// Hexadecimal codes, with or without `U+`, separated by spaces or commas.
    Unicode,
}

impl EntryMode {
    /// The mode's value in `entryMode`.
    fn name(self) -> &'static str {
        match self {
            EntryMode::Direct => "direct",
            EntryMode::Unicode => "unicode",
        }
    }
}

/// What a button of the custom character sets asks for.
enum SetAction {
    /// A new set named so, based on the set picked.
    New(String),
    /// Take the set picked out.
    Delete,
    /// Add the characters typed, or the character of the code typed, to the set picked.
    AddChars,
    /// Take the character picked out of the set picked.
    DeleteChar,
}

/// A change made in a part's row of the table.
enum PartEdit {
    /// A family picked in the Font column, with the style the picker gave.
    Font(String, Option<String>),
    Style(String),
    Size(f64),
    Baseline(f64),
    HScale(f64),
    VScale(f64),
    ScaleAtCenter(bool),
}

/// The Composite Fonts dialog on the dialog host. OK files the fonts for [`pump`], which runs
/// [`COMMAND`] and answers whether the dialog may close.
pub struct CompositeFontsDialog {
    fonts: Vec<CompositeFont>,
    /// The index of the font picked.
    selected: usize,
    /// The index of the custom set picked, in the font picked.
    set: usize,
    /// Whether the custom character sets show.
    custom: bool,
    entry: String,
    entry_mode: EntryMode,
    /// The index of the character picked in the set picked.
    picked_char: Option<usize>,
    /// Show Sample's fields ([`sample::defaults`]).
    sample: Map<String, Value>,
    /// The view settings of the Composite Font row's picker and of the Character Set row's.
    pickers: (PresetState<String>, PresetState<String>),
    /// What the family pickers of the Font column read.
    hold: PickerHold,
    /// OK, from the click to the answer of [`pump`].
    held: HeldAnswer<Vec<CompositeFont>>,
    /// The file dialog a Load…, Replace… or Save… asked for, which [`pump`] takes.
    file: Option<PresetFileRequest>,
    /// Identifies the draft that opened a Load or Replace picker.
    file_owner: Arc<()>,
    /// Why the last action on the fonts or sets was refused.
    refusal: Option<String>,
    /// The words of the file status line, kept from the last pass for [`pump`].
    words: Option<Arc<PresetWords>>,
}

/// Open Composite Fonts on the active document's composite fonts, the first one picked, with the
/// custom character sets and the sample hidden and nothing typed.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let fonts = app.session.active().ok_or("no document open")?.doc.composite_fonts.to_vec();
    let dialogs = app.dialogs().ok_or("no window to show the dialog in")?;
    let dialog = CompositeFontsDialog::new(fonts, crate::font_menu::hold(app));
    dialogs.with(|host| host.open(dialog));
    Ok(Value::Null)
}

/// Runs `f` on the open dialog; `None` when it is not open.
fn with_dialog<R>(app: &VectorcraftApp, f: impl FnOnce(&mut CompositeFontsDialog) -> R) -> Option<R> {
    let dialogs = app.dialogs()?;
    dialogs.with(|host| host.get_mut::<CompositeFontsDialog>().map(f))
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

/// Why `name` can't name a font or character set beside `names`: it is empty, or one of them has it in
/// any case.
fn name_error<'a>(mut names: impl Iterator<Item = &'a str>, name: &str) -> Option<PresetError> {
    if name.trim().is_empty() {
        return Some(PresetError::InvalidName);
    }
    names.any(|taken| CompositeFont::same_name(taken, name)).then_some(PresetError::NameTaken)
}

/// The suggested file name of a font's export: its name without the characters a path may not hold.
fn export_name(name: &str) -> String {
    let safe: String = name.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c }).collect();
    format!("{safe}.{}", EXTS[0])
}

/// The file entries of the Composite Font row over `fonts`: Load…, Replace… and Save…, which writes the
/// font the row names.
fn files(fonts: &[CompositeFont]) -> PresetFiles<'_, String> {
    PresetFiles::new(PURPOSE, tl!("Composite fonts"), EXTS).replace(true).export(move |key: Option<&String>| {
        let font = fonts.iter().find(|f| Some(&f.name) == key)?;
        let bytes = file_bytes(std::slice::from_ref(font)).ok()?;
        Some(PresetExport::new(export_name(&font.name), bytes))
    })
}

/// The part row `at` of `f` names: a class's part for the first rows, then each custom set's.
fn part_mut(f: &mut CompositeFont, at: usize) -> Option<&mut CompositePart> {
    match CharClass::ALL.get(at) {
        Some(class) => Some(f.parts.get_mut(*class)),
        None => f.custom.get_mut(at - CharClass::ALL.len()).map(|s| &mut s.part),
    }
}

impl CompositeFontsDialog {
    /// The dialog over `fonts`, the first picked.
    fn new(fonts: Vec<CompositeFont>, hold: PickerHold) -> Self {
        let mut sample = Map::new();
        sample::defaults(&mut sample);
        CompositeFontsDialog {
            fonts,
            selected: 0,
            set: 0,
            custom: false,
            entry: String::new(),
            entry_mode: EntryMode::Direct,
            picked_char: None,
            sample,
            pickers: (PresetState::new(), PresetState::new()),
            hold,
            held: HeldAnswer::new(),
            file: None,
            file_owner: Arc::new(()),
            refusal: None,
            words: None,
        }
    }

    /// The font picked.
    fn font(&self) -> Option<&CompositeFont> {
        self.fonts.get(self.selected)
    }

    /// The index of the custom set picked, within the font picked (0 when it has none).
    fn set_index(&self) -> usize {
        self.set.min(self.font().map_or(0, |f| f.custom.len().saturating_sub(1)))
    }

    /// Pick font `i` (the last when it is past them): its first custom set is picked, and no character.
    fn select(&mut self, i: usize) {
        self.selected = i.min(self.fonts.len().saturating_sub(1));
        self.select_set(0);
    }

    /// Pick custom set `i` of the font picked, and no character.
    fn select_set(&mut self, i: usize) {
        self.set = i;
        self.picked_char = None;
    }

    /// New: a font named `name`, based on the one picked, and picks it.
    fn new_font(&mut self, name: String) {
        let mut f = self.font().cloned().unwrap_or_else(|| {
            let st = vectorcraft_doc::CharStyle::default();
            CompositeFont::new("", st.font_family, st.font_style)
        });
        f.name = name;
        self.fonts.push(f);
        self.select(self.fonts.len() - 1);
    }

    /// Delete: take the font picked out and pick the one that takes its place.
    fn delete_font(&mut self) {
        if self.selected < self.fonts.len() {
            self.fonts.remove(self.selected);
        }
        self.select(self.selected);
    }

    /// Rename the font named `key` to `text`.
    fn rename_font(&mut self, key: &str, text: String) {
        if let Some(f) = self.fonts.iter_mut().find(|f| f.name == key) {
            f.name = text;
        }
    }

    /// Load… adds `read` to the fonts, a font of the same name replaced, and picks the first of them;
    /// Replace… takes `read` in place of the fonts.
    fn load(&mut self, read: Vec<CompositeFont>, replace: bool) {
        if replace {
            self.fonts = read;
            self.select(0);
            return;
        }
        let mut first = None;
        for f in read {
            let at = match self.fonts.iter_mut().enumerate().find(|(_, g)| CompositeFont::same_name(&g.name, &f.name)) {
                Some((k, g)) => {
                    *g = f;
                    k
                }
                None => {
                    self.fonts.push(f);
                    self.fonts.len() - 1
                }
            };
            first.get_or_insert(at);
        }
        if let Some(k) = first {
            self.select(k);
        }
    }

    /// Apply what the Composite Font row reported.
    fn font_event(&mut self, event: NamedPresetEvent<String>) {
        let at = |dialog: &Self, key: &str| dialog.fonts.iter().position(|f| f.name == key);
        match event {
            NamedPresetEvent::Apply(key) => {
                if let Some(i) = at(self, &key) {
                    self.select(i);
                }
            }
            NamedPresetEvent::Create { name, .. } => self.new_font(name),
            NamedPresetEvent::Rename { key, text, .. } => self.rename_font(&key, text),
            NamedPresetEvent::Delete(key) => {
                if let Some(i) = at(self, &key) {
                    self.selected = i;
                    self.delete_font();
                }
            }
            NamedPresetEvent::File(request) => self.file = Some(request),
            _ => {}
        }
    }

    /// Apply what the Character Set row reported.
    fn set_event(&mut self, event: NamedPresetEvent<String>) {
        let at = |dialog: &Self, key: &str| dialog.font().and_then(|f| f.custom.iter().position(|s| s.name == key));
        let done = match event {
            NamedPresetEvent::Apply(key) => {
                if let Some(i) = at(self, &key) {
                    self.select_set(i);
                }
                Ok(())
            }
            NamedPresetEvent::Create { name, .. } => self.edit_sets(SetAction::New(name)),
            NamedPresetEvent::Rename { key, text, .. } => {
                if let (Some(i), Some(f)) = (at(self, &key), self.fonts.get_mut(self.selected))
                    && let Some(s) = f.custom.get_mut(i)
                {
                    s.name = text;
                }
                Ok(())
            }
            NamedPresetEvent::Delete(key) => {
                if let Some(i) = at(self, &key) {
                    self.set = i;
                    self.edit_sets(SetAction::Delete)
                } else {
                    Ok(())
                }
            }
            _ => Ok(()),
        };
        self.refusal = done.err();
    }

    /// Do custom set action `act` on the font picked. The set picked afterwards is the one the action
    /// leaves; nothing changes when it is refused.
    fn edit_sets(&mut self, act: SetAction) -> Result<(), String> {
        let CompositeFontsDialog { fonts, selected, set, entry, entry_mode, picked_char, .. } = self;
        let f = fonts.get_mut(*selected).ok_or("no composite font")?;
        let k = (*set).min(f.custom.len().saturating_sub(1));
        let after = match act {
            SetAction::New(name) => {
                if f.custom.len() >= CUSTOM_SETS_MAX {
                    return Err(format!("a composite font keeps at most {CUSTOM_SETS_MAX} character sets"));
                }
                // Based on the set picked: its characters, font and values.
                let mut s = f.custom.get(k).cloned().unwrap_or_else(|| CustomCharSet::new("", f.parts.kanji.clone()));
                s.name = name;
                f.custom.push(s);
                Some(f.custom.len() - 1)
            }
            SetAction::Delete => {
                if k < f.custom.len() {
                    f.custom.remove(k);
                }
                Some(k.min(f.custom.len().saturating_sub(1)))
            }
            SetAction::AddChars => {
                let set = f.custom.get_mut(k).ok_or("no character set")?;
                let add: Vec<char> = if *entry_mode == EntryMode::Unicode {
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
                entry.clear();
                None
            }
            SetAction::DeleteChar => {
                let set = f.custom.get_mut(k).ok_or("no character set")?;
                if let Some(n) = *picked_char {
                    set.characters = set.characters.chars().enumerate().filter(|(j, _)| *j != n).map(|(_, c)| c).collect();
                }
                *picked_char = None;
                None
            }
        };
        if let Some(k) = after {
            *set = k;
            *picked_char = None;
        }
        Ok(())
    }

    /// Apply `edit` to the part of row `at` of the font picked.
    fn edit_part(&mut self, at: usize, edit: PartEdit) {
        let Some(part) = self.fonts.get_mut(self.selected).and_then(|f| part_mut(f, at)) else { return };
        match edit {
            PartEdit::Font(family, style) => {
                let db = FontDb::global();
                // A composite font can't be a part's font, as the command refuses.
                if db.composite_font(&family).is_some() {
                    return;
                }
                part.style = style.unwrap_or_else(|| db.styles(&family).first().cloned().unwrap_or_else(|| "Regular".into()));
                part.font = family;
            }
            PartEdit::Style(style) => part.style = style,
            PartEdit::Size(v) => part.size = v,
            PartEdit::Baseline(v) => part.baseline = v,
            PartEdit::HScale(v) => part.h_scale = v,
            PartEdit::VScale(v) => part.v_scale = v,
            PartEdit::ScaleAtCenter(on) => part.scale_at_center = on,
        }
    }

    /// Files OK's answer for [`pump`], unless one waits.
    fn file_ok(&mut self) {
        if !self.held.is_waiting() {
            self.held.file(self.fonts.clone());
        }
    }

    /// The Composite Font row and its table, the Custom and Show Sample switches, the sample and the custom
    /// character sets.
    fn body(&mut self, ui: &mut Ui) {
        self.font_row(ui);
        if self.font().is_none() {
            return;
        }
        self.parts_table(ui);
        let shown = sample::shown(&self.sample);
        Row::new().show(ui, |ui, _| {
            if let Some(on) = ToggleButton::new(tl!("Custom"), Bind::given(self.custom)).show(ui).value().copied() {
                self.custom = on;
            }
            if let Some(on) = Checkbox::new(Bind::given(shown)).label(tl!("Show Sample")).show(ui).value().copied() {
                self.sample.insert("sample".into(), json!(on));
            }
        });
        if shown && let Some(f) = self.fonts.get(self.selected).cloned() {
            sample::show(ui, &mut self.sample, &f);
        }
        if self.custom {
            self.custom_sets(ui);
        }
        let reason = self.refusal.clone().map(Label::from).or_else(|| self.held.refusal().map(|(reason, _)| reason.clone()));
        if let Some(reason) = reason {
            Notice::new(Tone::Bad, reason).layout(NoticeLayout::Inline).show(ui);
        }
    }

    /// The Composite Font row: the fonts to pick, New, Rename, Delete and the file entries.
    fn font_row(&mut self, ui: &mut Ui) {
        let events = {
            let fonts = &self.fonts;
            let rows: Vec<PresetRow<String>> = fonts.iter().map(|f| PresetRow::item(f.name.clone(), f.name.clone(), 0)).collect();
            let current = self.fonts.get(self.selected).map(|f| f.name.clone());
            let entries = files(fonts);
            let source = PresetSource::listed(&rows);
            let refuses = |typed: &str| name_error(fonts.iter().map(|f| f.name.as_str()), typed);
            let picker =
                PresetPicker::new(tl!("Composite Font"), Bind::given(current), &source, Bind::bound(&mut self.pickers.0)).field(PickerField::Name);
            NamedPresetRow::new(tl!("Composite Font"), picker)
                .suggest_name(new_name(fonts))
                .check_name(NameCheck::of_host(&refuses))
                .files(&entries)
                .show(ui)
                .events
        };
        for event in events {
            self.font_event(event);
        }
    }

    /// The table of the font picked's parts: a row for each class, then for each custom character set,
    /// with its font, style, size, baseline, scales and (Kana) Scale at Center; a row has only the cells
    /// it can take.
    fn parts_table(&mut self, ui: &mut Ui) {
        let Some(font) = self.fonts.get(self.selected) else { return };
        let db = FontDb::global();
        let rows: Vec<PartRow> = CharClass::ALL
            .into_iter()
            .map(|class| (class_label(class).to_owned(), font.parts.get(class).clone(), class.scales(), class == CharClass::Kana))
            // A set's name is the user's own, shown as it is.
            .chain(font.custom.iter().map(|s| (s.name.clone(), s.part.clone(), true, false)))
            .map(|(label, part, scales, center)| {
                let styles = db.styles(&part.font).into_iter().map(|style| Entry::new(style.clone(), style)).collect();
                PartRow { label, part, scales, center, styles }
            })
            .collect();
        let columns = vec![
            ColumnSpec::new(CLASS, Label::Empty, ColumnWidth::Content),
            ColumnSpec::new(FONT, tl!("Font"), ColumnWidth::Content),
            ColumnSpec::new(STYLE, tl!("Style"), ColumnWidth::fill(1.0)),
            ColumnSpec::new(SIZE, tl!("Size"), ColumnWidth::Controls(4.0)),
            ColumnSpec::new(BASELINE, tl!("Baseline"), ColumnWidth::Controls(4.0)),
            ColumnSpec::new(H_SCALE, tl!("Horizontal Scale"), ColumnWidth::Controls(4.0)),
            ColumnSpec::new(V_SCALE, tl!("Vertical Scale"), ColumnWidth::Controls(4.0)),
            ColumnSpec::new(CENTER, Label::Empty, ColumnWidth::Content),
        ];
        let keys: Vec<usize> = (0..rows.len()).collect();
        let edits: RefCell<Vec<(usize, PartEdit)>> = RefCell::new(Vec::new());
        let (settings, filters) = (self.hold.settings.clone(), self.hold.filters.clone());
        let size = Percent { column: SIZE, label: tl!("Size"), range: &COMPOSITE_SIZE, edit: PartEdit::Size };
        let baseline = Percent { column: BASELINE, label: tl!("Baseline"), range: &COMPOSITE_BASELINE, edit: PartEdit::Baseline };
        let h_scale = Percent { column: H_SCALE, label: tl!("Horizontal Scale"), range: &COMPOSITE_SCALE, edit: PartEdit::HScale };
        let v_scale = Percent { column: V_SCALE, label: tl!("Vertical Scale"), range: &COMPOSITE_SCALE, edit: PartEdit::VScale };
        {
            let sets = family_sets(self.hold.document.as_deref(), &self.hold.recently_added);
            let edits = &edits;
            Table::<usize>::new(tl!("Composite Font"), columns)
                .select(SelectMode::None)
                .min_rows(1)
                .max_rows(8)
                .sizing(Sizing::FillWidth)
                .show_items(ui, &keyed(&keys, |k: &usize| *k), &mut |key, icx| {
                    let at = *key;
                    let Some(row) = rows.get(at) else { return ListRow::new(Label::Empty) };
                    let p = &row.part;
                    // PLAN: the picker's settings and filters are given by value: `Bind::bound` can't be built in this
                    // closure (the row it returns outlives the call), so a favourite or filter changed from a cell does
                    // not reach `hold` and `hold.store` has nothing to write back.
                    let picker =
                        family_picker(tl!("Font"), Bind::given(p.font.clone()), Bind::given(settings.clone()), Bind::given(filters.clone()), &sets);
                    let style = Select::new(tl!("Style"), Bind::given(p.style.clone()), &row.styles);
                    let mut list = ListRow::new(row.label.clone())
                        .cells(icx.cells())
                        .cell_field(FONT, picker, move |out| {
                            if let Some(family) = out.picked {
                                edits.borrow_mut().push((at, PartEdit::Font(family, out.picked_style)));
                            }
                        })
                        .cell_field(STYLE, style, move |out| {
                            if let Some(style) = out.value().cloned() {
                                edits.borrow_mut().push((at, PartEdit::Style(style)));
                            }
                        });
                    list = percent(list, &size, p.size, at, edits);
                    list = percent(list, &baseline, p.baseline, at, edits);
                    if row.scales {
                        list = percent(list, &h_scale, p.h_scale, at, edits);
                        list = percent(list, &v_scale, p.v_scale, at, edits);
                    }
                    if row.center {
                        let center = Checkbox::new(Bind::given(p.scale_at_center)).label(tl!("Scale at Center"));
                        list = list.cell_field(CENTER, center, move |out| {
                            if let Some(on) = out.value().copied() {
                                edits.borrow_mut().push((at, PartEdit::ScaleAtCenter(on)));
                            }
                        });
                    }
                    list
                });
        }
        for (at, edit) in edits.into_inner() {
            self.edit_part(at, edit);
        }
    }

    /// The custom character sets of the font picked: the Character Set row, the Character row that adds
    /// characters, and the set's characters with Delete.
    fn custom_sets(&mut self, ui: &mut Ui) {
        let events = {
            let sets = self.fonts.get(self.selected).map(|f| f.custom.as_slice()).unwrap_or_default();
            let rows: Vec<PresetRow<String>> = sets.iter().map(|s| PresetRow::item(s.name.clone(), s.name.clone(), 0)).collect();
            let current = sets.get(self.set_index()).map(|s| s.name.clone());
            let source = PresetSource::listed(&rows);
            let refuses = |typed: &str| name_error(sets.iter().map(|s| s.name.as_str()), typed);
            let picker =
                PresetPicker::new(tl!("Character Set"), Bind::given(current), &source, Bind::bound(&mut self.pickers.1)).field(PickerField::Name);
            NamedPresetRow::new(tl!("Character Set"), picker)
                .suggest_name(new_set_name(sets))
                .check_name(NameCheck::of_host(&refuses))
                .show(ui)
                .events
        };
        for event in events {
            self.set_event(event);
        }
        let Some(set) = self.font().and_then(|f| f.custom.get(self.set_index())) else { return };
        let chars: Vec<char> = set.characters.chars().collect();
        let mut act = None;
        Row::new().show(ui, |ui, _| {
            let modes = [Entry::new(EntryMode::Direct, tl!("Direct Entry")), Entry::new(EntryMode::Unicode, tl!("Unicode"))];
            Select::new(tl!("Character"), Bind::bound(&mut self.entry_mode), &modes).show(ui);
            TextField::new(tl!("Character"), Bind::bound(&mut self.entry)).show(ui);
            if clicked(&Button::new(tl!("Add")).show(ui)) {
                act = Some(SetAction::AddChars);
            }
        });
        let keys: Vec<usize> = (0..chars.len()).collect();
        let selection = self.picked_char.map_or_else(Selection::default, Selection::single);
        let out = TileGrid::<usize>::new(tl!("Characters"))
            .select(SelectMode::Single)
            .thumb_size(ThumbSize::Small)
            .max_rows(CHAR_ROWS)
            .selection(Bind::given(selection))
            .show_items(ui, &keyed(&keys, |k: &usize| *k), &mut |key, _| {
                let c = chars.get(*key).copied().unwrap_or_default();
                Tile::new(c.to_string()).tooltip(format!("U+{:04X}", c as u32))
            });
        for event in out.response.events {
            if let CollectionEvent::Selection { change, .. } = event {
                match change {
                    SelectionChange::Set(keys) => self.picked_char = keys.last().copied(),
                    SelectionChange::Clear => self.picked_char = None,
                    _ => {}
                }
            }
        }
        let can_delete = self.picked_char.is_some_and(|n| n < chars.len());
        if clicked(&Button::new(tl!("Delete")).enabled(can_delete).show(ui)) {
            act = Some(SetAction::DeleteChar);
        }
        if let Some(act) = act {
            self.refusal = self.edit_sets(act).err();
        }
    }
}

/// A row of the parts table as it shows this pass.
struct PartRow {
    label: String,
    part: CompositePart,
    /// The row has Horizontal and Vertical Scale.
    scales: bool,
    /// The row has Scale at Center.
    center: bool,
    /// The styles of the part's font.
    styles: Vec<Entry<String>>,
}

/// A percent column of the parts table.
struct Percent {
    column: ColumnKey,
    /// The name of the column's field.
    label: &'static str,
    /// The values the field takes.
    range: &'static RangeInclusive<f64>,
    /// The edit a value typed in the field makes.
    edit: fn(f64) -> PartEdit,
}

/// `list` with a field of the column `spec` names showing `value`, the value of row `at`, which reports what is typed into `edits`.
fn percent<'r>(list: ListRow<'r>, spec: &Percent, value: f64, at: usize, edits: &'r RefCell<Vec<(usize, PartEdit)>>) -> ListRow<'r> {
    let Percent { column, label, range, edit } = spec;
    let (edit, field) = (*edit, NumberField::new(*label, Bind::given(Some(value))).range((*range).clone()).unit("%"));
    list.cell_field(column.clone(), field, move |out| {
        if let Some(Some(v)) = out.edit.value().copied() {
            edits.borrow_mut().push((at, edit(v)));
        }
    })
}

impl StackDialog for CompositeFontsDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        dialog_id(KIND)
    }

    fn key(&self) -> Option<&str> {
        Some(KIND)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let env = Env::of(ui);
        let common = env.words::<CommonWords>();
        self.words = Some(env.words::<PresetWords>());
        // The app accepted the held OK: this pass closes the dialog with it. A measuring run closes nothing.
        let finish = self.held.is_accepted() && !craft::ui::runtime::measure::active(ui.ctx());
        let buttons = [
            FooterButton::new(common.ok.clone(), DialogRole::Accept).stays_open(true).enabled(!self.held.is_waiting()),
            FooterButton::new(common.cancel.clone(), DialogRole::Cancel),
        ];
        let shown = Dialog::new(tl!("Composite Fonts")).buttons(buttons).close_on_enter(true).show(ui, |ui, cx| {
            if finish {
                cx.request_close_as(CloseReason::Activated);
            }
            self.body(ui);
        });
        let output = shown.output;
        if self.held.refusal().is_some_and(|(_, refused)| *refused != self.fonts) {
            self.held.clear_refusal();
        }
        // The pass reports a press only in the pass that took the input, so a second pass of the frame files nothing.
        if output.picked == Some(0) {
            self.file_ok();
        }
        if finish && output.closed == Some(CloseReason::Activated) {
            return Some(self.held.finish().map_or(DialogResult::Cancel, |_| DialogResult::Ok(())));
        }
        output.closed.map(|_| DialogResult::Cancel)
    }

    /// OK as the button: files the fonts for [`pump`] and answers none; once the app accepted them, it answers.
    fn confirm(&mut self, _: &Context) -> Option<()> {
        if self.held.is_accepted() {
            return self.held.finish().map(|_| ());
        }
        self.file_ok();
        None
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    fn json_fields(&self) -> Option<Value> {
        let mut fields = Map::new();
        fields.insert("fonts".into(), json!(self.fonts));
        fields.insert("selected".into(), json!(self.selected));
        fields.insert("name".into(), json!(self.font().map(|f| f.name.as_str()).unwrap_or_default()));
        fields.insert("custom".into(), json!(self.custom));
        fields.insert("entryMode".into(), json!(self.entry_mode.name()));
        fields.insert("entry".into(), json!(self.entry));
        fields.insert("set".into(), json!(self.set));
        let set_name = self.font().and_then(|f| f.custom.get(self.set_index())).map(|s| s.name.as_str()).unwrap_or_default();
        fields.insert("setName".into(), json!(set_name));
        // `char` is held only while a character is picked.
        if let Some(n) = self.picked_char {
            fields.insert("char".into(), json!(n));
        }
        fields.extend(self.sample.clone());
        Some(Value::Object(fields))
    }

    fn set_json_field(&mut self, _: &Context, field: &str, value: Value) -> Result<(), String> {
        let rejected = |value: &Value| FieldRefusal::Rejects(value).message(field);
        let index = |value: &Value| value.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| rejected(value));
        let text = |value: &Value| value.as_str().map(str::to_owned).ok_or_else(|| rejected(value));
        match field {
            "fonts" => {
                self.fonts = serde_json::from_value(value.clone()).map_err(|_| rejected(&value))?;
                self.select(self.selected);
            }
            "selected" => {
                let i = index(&value)?;
                if i >= self.fonts.len() {
                    return Err(rejected(&value));
                }
                self.select(i);
            }
            "name" => {
                let name = text(&value)?;
                self.fonts.get_mut(self.selected).ok_or_else(|| FieldRefusal::Inactive.message(field))?.name = name;
            }
            "custom" => self.custom = value.as_bool().ok_or_else(|| rejected(&value))?,
            "entryMode" => {
                self.entry_mode = match value.as_str() {
                    Some("direct") => EntryMode::Direct,
                    Some("unicode") => EntryMode::Unicode,
                    _ => return Err(rejected(&value)),
                }
            }
            "entry" => self.entry = text(&value)?,
            "set" => {
                let i = index(&value)?;
                if i >= self.font().map_or(0, |f| f.custom.len()) {
                    return Err(rejected(&value));
                }
                self.select_set(i);
            }
            "setName" => {
                let name = text(&value)?;
                let k = self.set_index();
                let set = self.fonts.get_mut(self.selected).and_then(|f| f.custom.get_mut(k));
                set.ok_or_else(|| FieldRefusal::Inactive.message(field))?.name = name;
            }
            "char" => self.picked_char = Some(index(&value)?),
            field if sample::is_field(field) => {
                sample::check_field(field, &value)?;
                self.sample.insert(field.to_owned(), value);
            }
            _ => return Err(FieldRefusal::Unknown.message(field)),
        }
        Ok(())
    }
}

// PLAN: hub-w2 owns `dialogs/host.rs` (`pump` calls `composite_fonts::pump`) and `dialogs.rs` (its `CompositeFonts` row
// and the `field_set` branch go: `SPEC` and `field_set` no longer exist here).
/// Runs once per frame after the shell pass: answers OK's held fonts with the command's result, and takes the
/// file dialog a Load…, Replace… or Save… asked for.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &Context) {
    let Some((held, request)) = with_dialog(app, |d| {
        let held = d.held.take();
        let request = d.file.take().map(|request| (request, Arc::clone(&d.file_owner)));
        (held, request)
    }) else {
        return;
    };
    if let Some(fonts) = held {
        // A name two fonts share or a value out of range keeps the dialog open with the reason.
        let answer = app.run(COMMAND, json!({ "fonts": fonts })).map(|_| ()).map_err(Label::from);
        with_dialog(app, |d| d.held.answer(answer));
    }
    if let Some((request, owner)) = request {
        ask_file(app, request, owner);
    }
}

/// Asks for the file `request` names. Open answers belong to the draft that requested them.
fn ask_file(app: &mut VectorcraftApp, request: PresetFileRequest, owner: Arc<()>) {
    // The pick's answer comes back through the continuation, so the immediate `{pending}` value says nothing more.
    match request.clone() {
        PresetFileRequest::Open { purpose, filter, extensions, replace, .. } => {
            let exts: Vec<&str> = extensions.iter().map(String::as_str).collect();
            let pick = PickRequest::open(purpose).filter(filter, &exts);
            let current = move |app: &VectorcraftApp| with_dialog(app, |draft| Arc::ptr_eq(&draft.file_owner, &owner)).unwrap_or(false);
            ask_while(app, pick, Owner::Free, current, move |app, picked| load_file(app, &request, replace, &picked));
        }
        PresetFileRequest::Save { purpose, filter, extensions, file_name, bytes, .. } => {
            let exts: Vec<&str> = extensions.iter().map(String::as_str).collect();
            let mut pick = PickRequest::save(purpose).filter(filter, &exts).file_name(file_name);
            if let Some(ext) = exts.first() {
                pick = pick.suggested_extension(*ext);
            }
            save(app, pick, bytes, Owner::Free, move |app, picked| saved(app, &request, picked));
        }
        _ => {}
    }
}

/// Reads the fonts of the file picked and gives them to the dialog (Load… or Replace…).
fn load_file(app: &mut VectorcraftApp, request: &PresetFileRequest, replace: bool, picked: &Picked) -> Result<Value, String> {
    // PLAN: assumed fl.A23 `file_asks::open_params(&Picked) -> Option<Map<String, Value>>`; `None` is a cancelled pick.
    let Some(mut params) = open_params(picked) else { return Ok(Value::Null) };
    params.insert("apply".into(), json!(false));
    let read = app.run(IMPORT, Value::Object(params))?;
    let fonts: Vec<CompositeFont> = serde_json::from_value(read["fonts"].clone()).map_err(|e| e.to_string())?;
    with_dialog(app, |d| d.load(fonts, replace));
    let name = picked.first_path().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
    // Only the file's name goes into the status line.
    if let Some(event) = request.loaded(name, Vec::new()) {
        report(app, event);
    }
    Ok(Value::Null)
}

/// Reports the save Save… asked for: the file service wrote it where the user picked, asking first about
/// replacing a file, or the browser took it (its save picker or a download).
fn saved(app: &mut VectorcraftApp, request: &PresetFileRequest, picked: Picked) -> Result<Value, String> {
    let event = match picked {
        Picked::Saved { path, .. } => request.saved(path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
        Picked::Downloaded(name) => request.saved(name),
        // The service's message is the system's; the words give the reason.
        Picked::Failed(_) => {
            let PresetFileRequest::Save { file_name, .. } = request else { return Ok(Value::Null) };
            Some(request.failed(file_name.clone(), PresetFileFailure::Other))
        }
        _ => return Ok(Value::Null),
    };
    if let Some(event) = event {
        report(app, event);
    }
    Ok(Value::Null)
}

/// Puts what a file pick did on the status line.
fn report(app: &mut VectorcraftApp, event: PresetFileEvent) {
    if let Some(words) = with_dialog(app, |d| d.words.clone()).flatten() {
        app.status(event.status(&words));
    }
}

#[cfg(test)]
mod tests;
