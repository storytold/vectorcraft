//! Type › Bullets and Numbering › Bullets and Numbering…, and More Options at the end of the
//! Paragraph panel's Bullets and Numbered List presets (or `ui.bulletsAndNumbering {tab?}`): a
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
//! `leftIndent` and `firstLineIndent` (pt), as `text.setFormat` takes them, with `__initial` (the
//! values it opened with) and `__targets` (the type and range it acts on).
//!
//! Behavior reference (public documentation):
//! <https://web.archive.org/web/20260417034638/https://helpx.adobe.com/illustrator/desktop/design-with-text/edit-format-text/create-bulleted-or-numbered-lists.html>.

use serde_json::{Map, Value, json};
use vectorcraft_doc::text::{LIST_HANGING_INDENT, LIST_LEVELS, NumberStyle, ParaList};

use super::swatch_options::label;
use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Bullets and Numbering.
pub const KIND: &str = "bulletsAndNumbering";

/// The field holding the values the dialog opened with.
const INITIAL: &str = "__initial";
/// The field holding the `ids`, `start` and `end` the dialog's `text.setFormat` acts on.
const TARGETS: &str = "__targets";
/// Set while the indent fields hold the hanging indent a preset filled in ([`pick`]).
const HANGING: &str = "__hanging";
const CMD: &str = "text.setFormat";

/// The `tab` values, in the order the tabs are shown.
const TABS: [&str; 2] = ["bullets", "numbers"];
/// The `listAlign` values, in the order Alignment lists them.
const ALIGNS: [&str; 2] = ["left", "right"];
/// The side of a preset button (points).
const PRESET: f32 = 34.0;
/// The keys a preset button with the keyboard focus keeps from egui's focus moves.
const ARROWS: egui::EventFilter = egui::EventFilter { tab: false, horizontal_arrows: true, vertical_arrows: false, escape: false };

/// The fields `ui.dialog.set` can set while the dialog does not hold them: the dialog leaves a
/// field out where the paragraphs differ, and holds no `bullet` or `number` for paragraphs in no
/// list or in another kind of list.
const INPUTS: [&str; 7] = ["list", "bullet", "number", "listAlign", "listLevel", "leftIndent", "firstLineIndent"];

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Bullets and Numbering").into(),
    body,
    confirm,
    preview: true,
    min_width: 380.0,
    inputs: |_| &INPUTS,
    ..DialogSpec::FORM
};

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
        Some(v) => Some(v.as_str().filter(|t| TABS.contains(t)).ok_or_else(|| format!("`{key}` must be one of {}", TABS.join(", ")))?),
    };
    crate::panels::character::text_style(app).ok_or("select some type first")?;
    // While typing, the dialog acts on the paragraphs of the text being edited.
    let mut targets = json!({});
    if let Some((id, a, b)) = crate::panels::character::text_editing(app) {
        crate::panels::character::end_typing(app);
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
    let tab = tab.unwrap_or(if list == Some("number") { "numbers" } else { "bullets" });
    let mut fields = Value::Object(values.clone());
    fields[INITIAL] = Value::Object(values);
    fields[TARGETS] = targets;
    fields["tab"] = json!(tab);
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let unit = app.session.general_unit();
    let numbers = d.str("tab") == "numbers";
    if let Some(tab) = widgets::tab_bar(ui, &[tl!("Bullets"), tl!("Numbers")], usize::from(numbers)).and_then(|i| TABS.get(i)) {
        d.fields.insert("tab".into(), json!(tab));
    }
    ui.add_space(8.0);
    level(ui, d);
    ui.add_space(8.0);
    if d.str("tab") == "numbers" {
        number_tab(ui, d);
    } else {
        bullet_tab(ui, d);
    }
    ui.add_space(10.0);
    position(ui, d, unit);
    // The type shows the dialog's values; with nothing changed it is as it was.
    let p = params(d);
    if changes(&p) {
        form::preview_now(app, d, tl!("Bullets and Numbering"), CMD, Value::Object(p));
    } else if d.fields.remove(form::PREVIEWED).is_some() {
        let _ = app.session.cancel_interaction();
    }
    false
}

/// The Level field: the paragraphs' list level, 1 to 9, blank where they differ.
fn level(ui: &mut egui::Ui, d: &mut Dialog) {
    ui.horizontal(|ui| {
        label(ui, tl!("Level:"));
        let value = d.fields.get("listLevel").and_then(Value::as_f64);
        if let Some(v) = widgets::range_field(ui, ("bullets-numbering", "listLevel"), value, 1.0..=LIST_LEVELS as f64, "", 0, 60.0) {
            d.fields.insert("listLevel".into(), json!(v));
        }
    });
}

/// The Bullets tab: the bullet presets in a row, None first, the one the dialog applies selected.
fn bullet_tab(ui: &mut egui::Ui, d: &mut Dialog) {
    let presets = crate::panels::paragraph::bullet_presets();
    let shown = d.fields.get("bullet").and_then(Value::as_str);
    let current = match d.fields.get("list").and_then(Value::as_str) {
        Some("none") => Some(0),
        Some("bullet") => presets.iter().position(|(_, key, _)| shown == Some(*key)),
        _ => None,
    };
    label(ui, tl!("Bullet Character:"));
    let names = presets.map(|(name, _, _)| name);
    let picked = preset_row(ui, "bullets", &names, current, |painter, rect, i, color| {
        crate::panels::paragraph::paint_bullet(painter, rect.center(), presets.get(i).and_then(|p| p.2), color);
    });
    match picked.and_then(|i| presets.get(i)) {
        Some((_, _, None)) => pick(d, "none", None),
        Some((_, key, Some(_))) => pick(d, "bullet", Some(*key)),
        None => {}
    }
}

/// The Numbers tab: None and the number styles in a row, each shown by its first two numbers, the
/// one the dialog applies selected.
fn number_tab(ui: &mut egui::Ui, d: &mut Dialog) {
    let shown = d.fields.get("number").and_then(|v| serde_json::from_value::<NumberStyle>(v.clone()).ok());
    let current = match d.fields.get("list").and_then(Value::as_str) {
        Some("none") => Some(0),
        Some("number") => shown.and_then(|s| NumberStyle::ALL.iter().position(|&n| n == s)).map(|i| i + 1),
        _ => None,
    };
    label(ui, tl!("Number Style:"));
    // Screen readers name each style by its first three numbers, as the Numbered List presets do.
    let samples: Vec<String> = NumberStyle::ALL.iter().map(|&s| crate::panels::paragraph::number_sample(s, 3)).collect();
    let names: Vec<&str> = std::iter::once(tl!("None")).chain(samples.iter().map(String::as_str)).collect();
    let picked = preset_row(ui, "numbers", &names, current, |painter, rect, i, color| match i.checked_sub(1).and_then(|k| NumberStyle::ALL.get(k)) {
        None => crate::panels::paragraph::paint_bullet(painter, rect.center(), None, color),
        Some(&style) => {
            let first = format!("{} {}", style.format(1), style.format(2));
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, first, egui::FontId::proportional(11.0), color);
        }
    });
    match picked {
        Some(0) => pick(d, "none", None),
        Some(i) => {
            if let Some(key) = NumberStyle::ALL.get(i - 1).and_then(|s| serde_json::to_value(s).ok()).and_then(|v| v.as_str().map(str::to_string)) {
                pick(d, "number", Some(&key));
            }
        }
        None => {}
    }
}

/// A row of preset buttons named `names`, `current` selected, each drawn by `paint` (painter,
/// button, index, color). ← and → move the choice along the row from the button with the keyboard
/// focus, and the focus with it. Returns the button clicked or moved to.
fn preset_row(
    ui: &mut egui::Ui,
    salt: &str,
    names: &[&str],
    current: Option<usize>,
    paint: impl Fn(&egui::Painter, egui::Rect, usize, egui::Color32),
) -> Option<usize> {
    let t = Tokens::get(ui.ctx());
    let ids: Vec<egui::Id> = (0..names.len()).map(|i| egui::Id::unique(("bullets-numbering-preset", salt, i))).collect();
    let mut picked = None;
    let mut focused = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (i, (&name, &id)) in names.iter().zip(&ids).enumerate() {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(PRESET, PRESET), egui::Sense::hover());
            let resp = ui.interact(rect, id, egui::Sense::click());
            let selected = current == Some(i);
            let fill = if selected {
                t.row_selected
            } else if resp.hovered() {
                t.hover
            } else {
                t.input
            };
            ui.painter().rect_filled(rect, egui::CornerRadius::same(3), fill);
            let (width, border) = if selected || resp.has_focus() { (1.5, t.accent) } else { (1.0, t.input_border) };
            ui.painter().rect_stroke(rect, egui::CornerRadius::same(3), egui::Stroke::new(width, border), egui::StrokeKind::Inside);
            paint(ui.painter(), rect, i, t.text_strong);
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, ui.is_enabled(), selected, name));
            if resp.has_focus() {
                focused = Some(i);
                // ← and → move the choice, not egui's focus.
                ui.memory_mut(|m| m.set_focus_lock_filter(id, ARROWS));
            }
            if resp.on_hover_text(name).clicked() {
                picked = Some(i);
            }
        }
    });
    if let Some(i) = focused {
        let (back, on) = ui.input_mut(|inp| {
            (inp.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft), inp.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight))
        });
        let to = if back {
            i.checked_sub(1)
        } else if on {
            Some(i + 1).filter(|&j| j < names.len())
        } else {
            None
        };
        if back || on {
            // egui moves the focus with an arrow pressed on a button that has just taken the
            // focus, before its focus-lock filter applies: the row moves it instead.
            ui.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
        }
        if let Some(j) = to {
            ui.memory_mut(|m| m.request_focus(ids[j]));
            picked = Some(j);
        }
    }
    picked
}

/// Pick a preset of kind `list` (`none`, `bullet` or `number`) with key `key` (for `bullet` or
/// `number`). For paragraphs that were in no list and had no indents, the indent fields take the
/// hanging indent `text.setFormat` gives a paragraph that joins a list; None puts them back while
/// they still hold it.
fn pick(d: &mut Dialog, list: &str, key: Option<&str>) {
    d.fields.insert("list".into(), json!(list));
    if let Some(key) = key {
        d.fields.insert(list.into(), json!(key));
    }
    let initial = d.fields.get(INITIAL).cloned().unwrap_or_default();
    let was = |k: &str| initial.get(k).and_then(Value::as_f64);
    let now = |d: &Dialog| (d.fields.get("leftIndent").and_then(Value::as_f64), d.fields.get("firstLineIndent").and_then(Value::as_f64));
    let hanging = (Some(LIST_HANGING_INDENT), Some(-LIST_HANGING_INDENT));
    if list != "none" {
        let unlisted = initial.get("list").and_then(Value::as_str) == Some("none");
        if unlisted && (was("leftIndent"), was("firstLineIndent")) == (Some(0.0), Some(0.0)) && now(d) == (Some(0.0), Some(0.0)) {
            d.fields.insert("leftIndent".into(), json!(LIST_HANGING_INDENT));
            d.fields.insert("firstLineIndent".into(), json!(-LIST_HANGING_INDENT));
            d.fields.insert(HANGING.into(), json!(true));
        }
    } else if d.fields.remove(HANGING).is_some() && now(d) == hanging {
        d.fields.insert("leftIndent".into(), json!(0.0));
        d.fields.insert("firstLineIndent".into(), json!(0.0));
    }
}

/// The Position group: Alignment, Left Indent and First Line Indent.
fn position(ui: &mut egui::Ui, d: &mut Dialog, unit: vectorcraft_doc::Unit) {
    widgets::section_header(ui, tl!("Position"));
    egui::Grid::new("bullets-numbering-position").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
        label(ui, tl!("Alignment:"));
        let names = [tl!("Left"), tl!("Right")];
        let current = d.fields.get("listAlign").and_then(Value::as_str).and_then(|a| ALIGNS.iter().position(|x| *x == a));
        // Already translated: shown as they are.
        if let Some(i) = widgets::dropdown_names(ui, ("bullets-numbering", "listAlign"), current.map_or("", |i| names[i]), &names, 100.0)
            && let Some(a) = ALIGNS.get(i)
        {
            d.fields.insert("listAlign".into(), json!(a));
        }
        ui.end_row();
        label(ui, tl!("Left Indent:"));
        form::length(ui, d, "leftIndent", unit, 90.0);
        ui.end_row();
        label(ui, tl!("First Line Indent:"));
        form::length(ui, d, "firstLineIndent", unit, 90.0);
        ui.end_row();
    });
}

/// `text.setFormat` parameters: the fields that differ from the values the dialog opened with,
/// on the dialog's type and range. With a preset that puts the paragraphs in a list, the indents
/// the dialog shows go too, so that the hanging indent `text.setFormat` gives a paragraph joining a
/// list does not replace them. Alignment and Level go only when the paragraphs are in a list or
/// the preset puts them in one.
fn params(d: &Dialog) -> Map<String, Value> {
    let initial = d.fields.get(INITIAL).and_then(Value::as_object);
    let was = |key: &str| initial.and_then(|m| m.get(key)).and_then(Value::as_str);
    let field = |key: &str| d.fields.get(key).and_then(Value::as_str);
    let mut p = d.fields.get(TARGETS).and_then(Value::as_object).cloned().unwrap_or_default();
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
        && initial.and_then(|m| m.get("listLevel")).and_then(Value::as_f64) != Some(l)
        && (listing || was("list") != Some("none"))
    {
        p.insert("listLevel".into(), json!(l));
    }
    for key in ["leftIndent", "firstLineIndent"] {
        if !d.fields.contains_key(key) {
            continue;
        }
        // A length typed with its unit ("6 mm") is sent in points.
        let v = d.f64(key, 0.0);
        if listing || initial.and_then(|m| m.get(key)).and_then(Value::as_f64) != Some(v) {
            p.insert(key.into(), json!(v));
        }
    }
    p
}

/// Does `p` change anything (more than the targets)?
fn changes(p: &Map<String, Value>) -> bool {
    p.keys().any(|k| !matches!(k.as_str(), "ids" | "start" | "end"))
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let p = params(d);
    if !changes(&p) {
        if app.session.in_interaction() {
            let _ = app.session.cancel_interaction();
        }
        app.ui.dialog = None;
        return Ok(Value::Null);
    }
    form::commit_preview(app, CMD, Value::Object(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_doc::text::{Bullet, ListAlign};
    use vectorcraft_engine::Session;
    use vectorcraft_engine::doc::{NodeId, NodeKind, ParaStyle};

    /// A selected area type object with two paragraphs.
    fn app() -> (VectorcraftApp, u64) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
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

    fn frame(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
    }

    fn set(app: &mut VectorcraftApp, key: &str, v: Value) {
        app.ui.dialog.as_mut().unwrap().fields.insert(key.into(), v);
    }

    fn pick_preset(app: &mut VectorcraftApp, list: &str, key: Option<&str>) {
        pick(app.ui.dialog.as_mut().unwrap(), list, key);
    }

    /// `ui.bulletsAndNumbering` opens on the selected type's values; a preset, Alignment and the
    /// indents show on the type at once, and OK keeps them as one `text.setFormat`, which one Undo
    /// takes back.
    #[test]
    fn ok_applies_the_marker_alignment_and_indents_as_one_undo_step() {
        let (mut app, id) = app();
        let undo = app.session.active().unwrap().history.undo.len();
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        let d = app.ui.dialog.clone().expect("Bullets and Numbering opened");
        assert_eq!((d.kind.as_str(), d.str("tab"), d.str("list"), d.str("listAlign")), (KIND, "bullets".into(), "none".into(), "left".into()));
        assert_eq!((d.f64("leftIndent", 1.0), d.f64("firstLineIndent", 1.0)), (0.0, 0.0));
        pick_preset(&mut app, "bullet", Some("square"));
        set(&mut app, "listAlign", json!("right"));
        set(&mut app, "leftIndent", json!(24));
        set(&mut app, "firstLineIndent", json!(-12));
        frame(&mut app);
        assert!(app.session.in_interaction(), "the preview runs");
        let want = Some(ParaList::bulleted(Bullet::Square).aligned(ListAlign::Right));
        assert_eq!(first(&app, id), (want, 24.0, -12.0), "previewed on the type");
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none() && !app.session.in_interaction());
        assert_eq!((first(&app, id), paras(&app, id).1.list), ((want, 24.0, -12.0), want));
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "one undo step");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(first(&app, id), (None, 0.0, 0.0));
        // The Numbers tab's presets make a numbered list, keeping the alignment the dialog shows.
        app.run("text.setFormat", json!({"list": "bullet", "listAlign": "right"})).unwrap();
        app.run("ui.bulletsAndNumbering", json!({"tab": "numbers"})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("tab"), "numbers");
        pick_preset(&mut app, "number", Some("upperRoman"));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(first(&app, id).0, Some(ParaList::numbered(NumberStyle::UpperRoman).aligned(ListAlign::Right)));
        // OK with nothing changed adds no undo step, and an unknown tab is an error.
        let undo = app.session.active().unwrap().history.undo.len();
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("tab"), "numbers", "a numbered list opens on Numbers");
        frame(&mut app);
        super::super::confirm(&mut app).unwrap();
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
        assert!(app.run("ui.bulletsAndNumbering", json!({"tab": "levels"})).is_err());
        assert!(app.ui.dialog.is_none());
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
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        pick_preset(&mut app, "number", Some("lowerLetters"));
        set(&mut app, "listAlign", json!("right"));
        set(&mut app, "leftIndent", json!(30));
        frame(&mut app);
        assert_eq!(first(&app, id), (Some(ParaList::numbered(NumberStyle::LowerLetters).aligned(ListAlign::Right)), 30.0, -18.0));
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
        super::super::cancel(&mut app);
        assert!(app.ui.dialog.is_none());
        assert_eq!(first(&app, id), before);
    }

    /// A preset picked for paragraphs in no list and without indents fills in the 18 pt and
    /// -18 pt hanging indent, and None takes it out again; indents that are set stay, and so do
    /// indents set back to 0 with the preset.
    #[test]
    fn a_preset_fills_in_the_hanging_indent_a_new_list_gets() {
        let (mut app, id) = app();
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        pick_preset(&mut app, "bullet", Some("dot"));
        let d = app.ui.dialog.clone().unwrap();
        assert_eq!((d.f64("leftIndent", 0.0), d.f64("firstLineIndent", 0.0)), (18.0, -18.0));
        pick_preset(&mut app, "number", Some("arabic"));
        pick_preset(&mut app, "none", None);
        let d = app.ui.dialog.clone().unwrap();
        assert_eq!((d.f64("leftIndent", 1.0), d.f64("firstLineIndent", 1.0)), (0.0, 0.0));
        assert!(!changes(&params(&d)));
        super::super::cancel(&mut app);
        app.run("text.setFormat", json!({"leftIndent": 10})).unwrap();
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        pick_preset(&mut app, "bullet", Some("dash"));
        assert_eq!(app.ui.dialog.as_ref().unwrap().f64("leftIndent", 0.0), 10.0);
        super::super::confirm(&mut app).unwrap();
        assert_eq!(first(&app, id), (Some(ParaList::bulleted(Bullet::Dash)), 10.0, 0.0));
        // Indents set back to 0 with the preset are kept: the hanging indent does not replace them.
        let (mut app, id) = self::app();
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        pick_preset(&mut app, "number", Some("arabic"));
        set(&mut app, "leftIndent", json!(0));
        set(&mut app, "firstLineIndent", json!(0));
        frame(&mut app);
        assert_eq!(first(&app, id), (Some(ParaList::numbered(NumberStyle::Arabic)), 0.0, 0.0), "previewed as shown");
        super::super::confirm(&mut app).unwrap();
        assert_eq!(first(&app, id), (Some(ParaList::numbered(NumberStyle::Arabic)), 0.0, 0.0));
    }

    /// Paragraphs that differ leave the preset unpicked and the fields blank, and OK sends only
    /// what was changed: each paragraph keeps its own indents and list.
    #[test]
    fn differing_paragraphs_show_blank_fields_and_keep_what_is_left_alone() {
        let (mut app, id) = app();
        app.run("text.setFormat", json!({"ids": [id], "bullet": "circle", "leftIndent": 30, "start": 0, "end": 0})).unwrap();
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        let d = app.ui.dialog.clone().unwrap();
        for key in ["list", "bullet", "leftIndent"] {
            assert!(!d.fields.contains_key(key), "{key} is blank");
        }
        assert_eq!((d.str("listAlign"), d.f64("firstLineIndent", 1.0)), ("left".into(), 0.0));
        set(&mut app, "listAlign", json!("right"));
        super::super::confirm(&mut app).unwrap();
        let (one, two) = paras(&app, id);
        assert_eq!((one.list, one.left_indent), (Some(ParaList::bulleted(Bullet::Circle).aligned(ListAlign::Right)), 30.0));
        assert_eq!((two.list, two.left_indent), (None, 0.0), "the paragraph in no list stays out of one");
    }

    /// Alignment changed for paragraphs that stay in no list sends nothing: there is no preview,
    /// and OK adds no undo step. A preset picked with it takes the alignment.
    #[test]
    fn alignment_alone_on_paragraphs_in_no_list_adds_no_undo_step() {
        let (mut app, id) = app();
        let undo = app.session.active().unwrap().history.undo.len();
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        set(&mut app, "listAlign", json!("right"));
        assert!(!changes(&params(app.ui.dialog.as_ref().unwrap())));
        frame(&mut app);
        assert!(!app.session.in_interaction(), "no preview");
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo, "no undo step");
        assert_eq!(first(&app, id), (None, 0.0, 0.0));
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        set(&mut app, "listAlign", json!("right"));
        pick_preset(&mut app, "bullet", Some("dot"));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(first(&app, id).0, Some(ParaList::bulleted(Bullet::Dot).aligned(ListAlign::Right)));
    }

    /// `ui.dialog.set` sets the fields the dialog does not hold: a bullet or a number for type in
    /// no list, and the fields left blank where the paragraphs differ.
    #[test]
    fn the_control_channel_sets_fields_the_dialog_does_not_hold() {
        let ctx = egui::Context::default();
        let (mut app, id) = app();
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        for key in ["bullet", "number"] {
            assert!(!app.ui.dialog.as_ref().unwrap().fields.contains_key(key), "{key} is not held");
        }
        let fields = json!([{"field": "list", "value": "number"}, {"field": "number", "value": "upperRoman"}]);
        crate::tests_synthetic::control(&mut app, &ctx, "ui.dialog.set", json!({"fields": fields}));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(first(&app, id).0, Some(ParaList::numbered(NumberStyle::UpperRoman)));
        app.run("text.setFormat", json!({"ids": [id], "bullet": "circle", "leftIndent": 30, "firstLineIndent": -6, "start": 0, "end": 0})).unwrap();
        app.run("text.setFormat", json!({"ids": [id], "listAlign": "right", "listLevel": 2, "start": 0, "end": 0})).unwrap();
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        let held = app.ui.dialog.as_ref().unwrap().fields.clone();
        for key in INPUTS {
            assert!(!held.contains_key(key), "{key} is blank");
        }
        let fields = json!([
            {"field": "list", "value": "bullet"},
            {"field": "bullet", "value": "square"},
            {"field": "listAlign", "value": "left"},
            {"field": "listLevel", "value": 3},
            {"field": "leftIndent", "value": 24},
            {"field": "firstLineIndent", "value": -12},
        ]);
        crate::tests_synthetic::control(&mut app, &ctx, "ui.dialog.set", json!({"fields": fields}));
        super::super::confirm(&mut app).unwrap();
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
        assert_eq!(app.ui.dialog.as_ref().unwrap().f64("listLevel", 0.0), 1.0, "in no list: level 1");
        pick_preset(&mut app, "number", Some("arabic"));
        set(&mut app, "listLevel", json!(3));
        frame(&mut app);
        assert_eq!(first(&app, id).0, Some(ParaList::numbered(NumberStyle::Arabic).at_level(3)), "previewed on the type");
        super::super::confirm(&mut app).unwrap();
        assert_eq!(paras(&app, id).1.list, Some(ParaList::numbered(NumberStyle::Arabic).at_level(3)));
        app.run("text.setFormat", json!({"ids": [id], "listLevel": 5, "start": 0, "end": 0})).unwrap();
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        assert!(!app.ui.dialog.as_ref().unwrap().fields.contains_key("listLevel"), "levels 5 and 3 differ: blank");
        super::super::cancel(&mut app);
        app.run("text.setFormat", json!({"ids": [id], "listLevel": 5})).unwrap();
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().f64("listLevel", 0.0), 5.0);
        assert!(!changes(&params(app.ui.dialog.as_ref().unwrap())), "nothing changed yet");
        set(&mut app, "listLevel", json!(2));
        assert_eq!(params(app.ui.dialog.as_ref().unwrap()).get("listLevel"), Some(&json!(2.0)));
        super::super::confirm(&mut app).unwrap();
        assert_eq!((first(&app, id).0.map(|l| l.level), paras(&app, id).1.list.map(|l| l.level)), (Some(2), Some(2)));
    }

    /// `text.setFormat` sets `listLevel` only on paragraphs in a list, so a Level change alone on
    /// paragraphs in no list sends nothing and adds no undo step; with a preset it applies.
    #[test]
    fn level_alone_on_paragraphs_in_no_list_adds_no_undo_step() {
        let (mut app, id) = app();
        let undo = app.session.active().unwrap().history.undo.len();
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        set(&mut app, "listLevel", json!(3));
        assert!(!changes(&params(app.ui.dialog.as_ref().unwrap())));
        frame(&mut app);
        assert!(!app.session.in_interaction(), "no preview");
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo, "no undo step");
        assert_eq!(first(&app, id), (None, 0.0, 0.0));
        app.run("ui.bulletsAndNumbering", json!({})).unwrap();
        set(&mut app, "listLevel", json!(3));
        pick_preset(&mut app, "bullet", Some("dot"));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(first(&app, id).0, Some(ParaList::bulleted(Bullet::Dot).at_level(3)));
    }

    /// The presets are radio buttons named for screen readers, the one the dialog applies chosen;
    /// a click picks one, and ← and → move the choice from the focused one.
    #[test]
    fn the_presets_are_named_radio_buttons_that_the_arrow_keys_move_along() {
        use egui::accesskit::{Role, Toggled};
        let names = ["None", "Filled Dot", "Dash"];
        let tree = crate::widgets::accessibility_tests::tree(|ui| {
            let _ = preset_row(ui, "test", &names, Some(1), |_, _, _, _| {});
        });
        assert_eq!(crate::widgets::accessibility_tests::role_and_state(&tree, "Filled Dot"), (Role::RadioButton, Some(Toggled::True)));
        assert_eq!(crate::widgets::accessibility_tests::role_and_state(&tree, "Dash"), (Role::RadioButton, Some(Toggled::False)));
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let origin = std::cell::Cell::new(egui::Pos2::ZERO);
        let run = |events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
            let mut picked = None;
            let mut out = ctx.run_ui(input, |ui| {
                origin.set(ui.cursor().min);
                picked = preset_row(ui, "test", &names, Some(1), |_, _, _, _| {});
            });
            out.textures_delta.clear();
            picked
        };
        let key = |k| egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        assert_eq!(run(vec![]), None);
        assert_eq!(run(vec![key(egui::Key::ArrowRight)]), None, "no preset has the focus");
        ctx.memory_mut(|m| m.request_focus(egui::Id::unique(("bullets-numbering-preset", "test", 1usize))));
        run(vec![]);
        assert_eq!(run(vec![key(egui::Key::ArrowRight)]), Some(2));
        assert_eq!(run(vec![key(egui::Key::ArrowLeft)]), Some(1), "the focus moved to Dash");
        assert_eq!(run(vec![key(egui::Key::ArrowLeft)]), Some(0));
        assert_eq!(run(vec![key(egui::Key::ArrowLeft)]), None, "None is the first");
        // The middle of the third button, two buttons and their 4 pt gaps along.
        let at = origin.get() + egui::vec2(PRESET * 2.5 + 8.0, PRESET * 0.5);
        let click = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        run(vec![egui::Event::PointerMoved(at)]);
        assert_eq!(run(vec![click(true), click(false)]), Some(2));
    }
}
