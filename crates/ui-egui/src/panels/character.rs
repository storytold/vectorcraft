//! Character panel: font family / style, size, leading, kerning, tracking, vertical and horizontal
//! scale, baseline shift, character rotation and the All Caps, Small Caps, Superscript, Subscript,
//! Underline and Strikethrough toggles.

use std::sync::{Mutex, OnceLock};

use egui::{Key, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::{CharStyle, NodeId, NodeKind};
use vectorcraft_tools::{Mods, ToolKey};

use super::{pstate, set_pstate};
use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets::{self, SpinPick, menu_item};

/// The Font Size and Leading dropdowns' presets in points (Leading's after its Auto entry), shown
/// in the type unit.
pub const SIZE_PRESETS: [f64; 15] = [6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 14.0, 18.0, 21.0, 24.0, 36.0, 48.0, 60.0, 72.0];
/// The Kerning and Tracking dropdowns' presets in 1/1000 em.
pub const TRACKING_PRESETS: [f64; 14] = [-100.0, -75.0, -50.0, -25.0, -10.0, -5.0, 0.0, 5.0, 10.0, 25.0, 50.0, 75.0, 100.0, 200.0];
pub const SCALE_PRESETS: [f64; 8] = [25.0, 50.0, 75.0, 90.0, 100.0, 110.0, 125.0, 150.0];

/// The stored character style: the selected range's (or the caret's) while the Type
/// tool edits text, else the first selected text object's first run (including groups). With it, the paragraph
/// attributes of the paragraph the selection starts in (else of the first paragraph).
/// Font/paragraph controls and named-style capture use this; dimensional fields use `shared_effective`.
pub(crate) fn text_style(app: &VectorcraftApp) -> Option<(CharStyle, vectorcraft_doc::ParaStyle)> {
    if let Some((id, a, b)) = text_editing(app)
        && let Some(NodeKind::Text(t)) = app.session.active().and_then(|d| d.doc.node(id)).map(|n| &n.kind)
    {
        return Some((vectorcraft_text::edit::insertion_style(&t.runs, a, b), t.para_at(t.paragraphs_in(a, a).start).clone()));
    }
    let t = first_selected_text(app)?;
    Some((t.first_style(), t.para_at(0).clone()))
}

/// The first selected text descendant, shared by Character and Paragraph controls.
pub(crate) fn first_selected_text(app: &VectorcraftApp) -> Option<&vectorcraft_doc::TextObject> {
    let st = app.session.active()?;
    for node in st.selection.objects.iter().filter_map(|id| st.doc.node(*id)) {
        let mut text = None;
        node.walk(&mut |n| {
            if text.is_none()
                && let NodeKind::Text(t) = &n.kind
            {
                text = Some(t.as_ref());
            }
        });
        if text.is_some() {
            return text;
        }
    }
    None
}

/// `f` of the character styles the panels act on: the runs the Type tool's selection covers (the
/// caret's style at a caret), else every run of the selected type, groups' included. `None` (a
/// blank field) where they differ, or with no type.
pub(crate) fn shared<T: PartialEq>(app: &VectorcraftApp, f: impl Fn(&CharStyle) -> T) -> Option<T> {
    shared_styles(app, |_, style| Some(f(style)))
}

/// Dimensional controls use document units and tolerate arithmetic roundoff when comparing
/// compensated runs. Keep raw styles available to font/paragraph controls and style definitions,
/// even if a collapsed affine prevents editing dimensions.
fn shared_effective(app: &VectorcraftApp, f: impl Fn(&CharStyle) -> f64) -> Option<f64> {
    shared_styles(app, |t, style| t.effective_char_style(style).map(|s| Metric(f(&s)))).map(|v| v.0)
}

struct Metric(f64);

impl PartialEq for Metric {
    fn eq(&self, other: &Self) -> bool {
        (self.0 - other.0).abs() <= 1e-9 * self.0.abs().max(other.0.abs()).max(1.0)
    }
}

fn shared_styles<T: PartialEq>(app: &VectorcraftApp, f: impl Fn(&vectorcraft_doc::TextObject, &CharStyle) -> Option<T>) -> Option<T> {
    let st = app.session.active()?;
    let text = |id: NodeId| match st.doc.node(id).map(|n| &n.kind) {
        Some(NodeKind::Text(t)) => Some(t),
        _ => None,
    };
    let mut acc = Shared::None;
    if let Some((id, a, b)) = text_editing(app) {
        let t = text(id)?;
        if b <= a {
            return f(t, &vectorcraft_text::edit::insertion_style(&t.runs, a, b));
        }
        let mut end = 0;
        for r in &t.runs {
            let start = end;
            end += r.text.len();
            if start < b && end > a {
                acc.add(f(t, &r.style));
            }
        }
    } else {
        for n in st.selection.objects.iter().filter_map(|id| st.doc.node(*id)) {
            n.walk(&mut |c| {
                if let NodeKind::Text(t) = &c.kind {
                    t.runs.iter().for_each(|r| acc.add(f(t, &r.style)));
                }
            });
        }
    }
    acc.value().flatten()
}

/// The value a run of styles share so far ([`shared`]).
enum Shared<T> {
    None,
    Same(T),
    Differ,
}

impl<T: PartialEq> Shared<T> {
    fn add(&mut self, v: T) {
        match self {
            Shared::None => *self = Shared::Same(v),
            Shared::Same(x) if *x != v => *self = Shared::Differ,
            _ => {}
        }
    }
    fn value(self) -> Option<T> {
        match self {
            Shared::Same(v) => Some(v),
            _ => None,
        }
    }
}

fn style(app: &mut VectorcraftApp, p: Value) {
    char_cmd(app, "text.setStyle", p);
}
fn format(app: &mut VectorcraftApp, p: Value) {
    char_cmd(app, "text.setFormat", p);
}

/// Apply character attributes: to the selected range while the Type tool edits text (the whole
/// object when nothing is selected), else to the selected text objects.
fn char_cmd(app: &mut VectorcraftApp, cmd: &str, p: Value) {
    if text_editing(app).is_none() {
        app.run(cmd, p).ok();
        return;
    }
    range_style(app, p);
}

/// Set the Kerning field's `value` (a number or "auto"). At a caret in text the Type tool edits
/// it is the pair kerning after the character before the caret, as Alt+←/→ steps it
/// (`type.step`); nowhere to set at the start of the text. Else it is a character attribute like
/// the others: the selected range, or all the text.
fn set_kerning(app: &mut VectorcraftApp, value: Value) {
    let Some((id, a, b)) = text_editing(app) else {
        format(app, json!({"kerning": value}));
        return;
    };
    if b > a {
        range_style(app, json!({"kerning": value}));
        return;
    }
    let Some(NodeKind::Text(t)) = app.session.active().and_then(|st| st.doc.node(id)).map(|n| &n.kind) else { return };
    let start = vectorcraft_text::edit::prev_char(&t.plain_text(), a);
    if start >= a {
        return;
    }
    end_typing(app);
    app.run("text.setRangeStyle", json!({"id": id.0, "start": start, "end": a, "kerning": value})).ok();
}

/// The faces Type › Bold and Type › Italic switch to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    Bold,
    Italic,
}

/// Words of a style name that set its weight: a style without any is the family's regular weight.
const WEIGHTS: [&str; 16] = [
    "thin",
    "hairline",
    "extralight",
    "ultralight",
    "light",
    "book",
    "medium",
    "semibold",
    "demibold",
    "demi",
    "bold",
    "extrabold",
    "ultrabold",
    "heavy",
    "black",
    "ultra",
];

/// Words that name the regular weight, upright: not part of what Bold and Italic keep.
const PLAIN: [&str; 5] = ["regular", "normal", "roman", "plain", "upright"];

/// A style name's (bold, regular weight, italic, the other words): `Bold Italic` is (true, false,
/// true, []), `Narrow Italic` (false, true, true, ["narrow"]), `SemiBold` (false, false, false, []).
fn face_of(style: &str) -> (bool, bool, bool, Vec<String>) {
    let words: Vec<String> = style.split([' ', '-', '_']).filter(|w| !w.is_empty()).map(str::to_ascii_lowercase).collect();
    let italic = words.iter().any(|w| w == "italic" || w == "oblique");
    let bold = words.iter().any(|w| w == "bold");
    let regular = !words.iter().any(|w| WEIGHTS.contains(&w.as_str()));
    let mut rest: Vec<String> =
        words.into_iter().filter(|w| !WEIGHTS.contains(&w.as_str()) && !PLAIN.contains(&w.as_str()) && w != "italic" && w != "oblique").collect();
    rest.sort();
    (bold, regular, italic, rest)
}

/// The style of `styles` that is bold (else of regular weight) and italic (else upright), with the
/// same other words as style `like` (a Narrow face stays Narrow, a plain one plain).
fn pick_face<'a>(styles: &'a [String], bold: bool, italic: bool, like: &str) -> Option<&'a str> {
    let (.., keep) = face_of(like);
    styles
        .iter()
        .map(String::as_str)
        .filter(|s| {
            let (b, r, i, rest) = face_of(s);
            i == italic && (if bold { b } else { r }) && rest == keep
        })
        // `Regular` before `Regular Text`-like doubles: the plainest name is the family's own face.
        .min_by_key(|s| s.len())
}

/// Type › Bold and Type › Italic (Cmd+Shift+B / Cmd+Shift+I while the Type tool edits text, as in
/// page layout apps): the family's own Bold or Italic face for the text the panels act on, or back
/// to its regular weight (upright) when it already is bold (italic). No fake bold or slant: a family
/// without that face is left as it is, and the message says so (#724).
pub(crate) fn toggle_face(app: &mut VectorcraftApp, face: Face) -> Result<Value, String> {
    let (s, _) = text_style(app).ok_or("select some type first")?;
    let styles = vectorcraft_text::FontDb::global().styles(&s.font_family);
    let (bold, _, italic, _) = face_of(&s.font_style);
    let (want_bold, want_italic) = match face {
        Face::Bold => (!bold, italic),
        Face::Italic => (bold, !italic),
    };
    let Some(name) = pick_face(&styles, want_bold, want_italic, &s.font_style) else {
        let wanted = match (want_bold, want_italic) {
            (true, true) => "Bold Italic",
            (true, false) => "Bold",
            (false, true) => "Italic",
            (false, false) => "Regular",
        };
        return Err(format!("{} has no {} style", s.font_family, wanted));
    };
    let name = name.to_string();
    style(app, json!({ "style": name }));
    Ok(json!({ "style": name }))
}

/// `text.setRangeStyle` with `p` on the range the Type tool has selected (the whole text when
/// nothing is selected). Does nothing unless the Type tool edits text.
pub(crate) fn range_style(app: &mut VectorcraftApp, p: Value) {
    let Some((id, a, b)) = text_editing(app) else { return };
    end_typing(app);
    let mut q = p;
    q["id"] = json!(id.0);
    if b > a {
        q["start"] = json!(a);
        q["end"] = json!(b);
    }
    app.run("text.setRangeStyle", q).ok();
}

// ---------- Type tool editing glue (selection, clipboard, keys) ----------

/// The text object the Type tool is editing and its selection (clamped to the text).
pub(crate) fn text_editing(app: &VectorcraftApp) -> Option<(NodeId, usize, usize)> {
    let o = app.session.tool_options();
    let id = NodeId(o.get("editing")?.as_u64()?);
    let len = match &app.session.active()?.doc.node(id)?.kind {
        NodeKind::Text(t) => vectorcraft_text::edit::runs_len(&t.runs),
        _ => return None,
    };
    let g = |k: &str| o.get(k).and_then(Value::as_u64).map_or(0, |v| (v as usize).min(len));
    Some((id, g("start"), g("end")))
}

/// Close the Type tool's typing session (one undo step) before another command edits the text.
pub(crate) fn end_typing(app: &mut VectorcraftApp) {
    if app.session.tool_options().get("typing").and_then(Value::as_bool) == Some(true) {
        app.session.set_tool_option("commitTyping", &json!(true));
        app.session.commit_interaction().ok();
        app.sync_views();
    }
}

static CTX: OnceLock<egui::Context> = OnceLock::new();
/// Plain text of the last text copied inside the Type tool (paste from the native menu, which
/// can't read the system clipboard).
static CLIP: Mutex<String> = Mutex::new(String::new());

fn text_copy(app: &mut VectorcraftApp) -> bool {
    let Some((id, a, b)) = text_editing(app) else { return false };
    if a == b {
        return false;
    }
    let Ok(r) = app.session.execute("text.getRange", &json!({"id": id.0, "start": a, "end": b})) else { return false };
    app.session.set_tool_option("copy", &r["runs"]);
    let text = r["text"].as_str().unwrap_or("").to_string();
    if let Some(ctx) = CTX.get() {
        ctx.copy_text(text.clone());
    }
    *CLIP.lock().unwrap_or_else(|e| e.into_inner()) = text;
    true
}

fn text_key(app: &mut VectorcraftApp, k: ToolKey, m: Mods) {
    let v = app.view_info();
    if let Err(e) = app.session.tool_key(k, m, v) {
        app.status(e.to_string());
    }
}

fn text_paste(app: &mut VectorcraftApp, s: Option<String>) {
    let s = s.unwrap_or_else(|| CLIP.lock().unwrap_or_else(|e| e.into_inner()).clone());
    if !s.is_empty() {
        let v = app.view_info();
        if let Err(e) = app.session.tool_text(&s, v) {
            app.status(e.to_string());
        }
    }
}

/// Edit-menu commands while the Type tool edits text act on the text (Cut/Copy/Paste/Clear;
/// Select All is the engine's `select.all`). `None` = not intercepted.
pub(crate) fn intercept_text_command(app: &mut VectorcraftApp, id: &str) -> Option<Result<Value, String>> {
    if !app.session.tool_wants_text() {
        return None;
    }
    match id {
        "edit.copy" => {
            text_copy(app);
        }
        "edit.cut" => {
            if text_copy(app) {
                text_key(app, ToolKey::Delete, Mods::default());
            }
        }
        "edit.paste" | "edit.pasteWithoutFormatting" => text_paste(app, None),
        "edit.clear" => text_key(app, ToolKey::Delete, Mods::default()),
        _ => return None,
    }
    app.sync_views();
    Some(Ok(Value::Null))
}

enum TextInput {
    Key(ToolKey, Mods),
    Copy,
    Cut,
    Paste(Option<String>),
    Face(Face),
}

/// Route editing keys (with modifiers), clipboard events and Cmd+C/X/V to the Type tool (Cmd+A is
/// left to the Select All shortcut, which selects the text being edited).
pub(crate) fn route_type_input(app: &mut VectorcraftApp, ctx: &egui::Context) {
    CTX.get_or_init(|| ctx.clone());
    let mut todo = vec![];
    ctx.input_mut(|i| {
        i.events.retain(|e| match e {
            egui::Event::Copy => {
                todo.push(TextInput::Copy);
                false
            }
            egui::Event::Cut => {
                todo.push(TextInput::Cut);
                false
            }
            egui::Event::Paste(s) => {
                todo.push(TextInput::Paste(Some(s.clone())));
                false
            }
            egui::Event::Key { key, pressed, modifiers: m, .. } => {
                let mods = Mods { shift: m.shift, alt: m.alt, cmd: m.command, ctrl: m.ctrl && !m.command, space: false };
                let tk = match key {
                    Key::ArrowLeft => Some(ToolKey::Left),
                    Key::ArrowRight => Some(ToolKey::Right),
                    Key::ArrowUp => Some(ToolKey::Up),
                    Key::ArrowDown => Some(ToolKey::Down),
                    Key::Home => Some(ToolKey::Home),
                    Key::End => Some(ToolKey::End),
                    Key::Backspace => Some(ToolKey::Backspace),
                    Key::Delete => Some(ToolKey::Delete),
                    Key::Tab if !m.command => Some(ToolKey::Tab),
                    _ => None,
                };
                if let Some(tk) = tk {
                    if *pressed {
                        todo.push(TextInput::Key(tk, mods));
                    }
                    return false;
                }
                // Type › Bold and Italic, as page layout apps have them while text is edited.
                if m.command && m.shift && !m.alt && matches!(key, Key::B | Key::I) {
                    if *pressed {
                        todo.push(TextInput::Face(if *key == Key::B { Face::Bold } else { Face::Italic }));
                    }
                    return false;
                }
                if m.command && !m.shift && !m.alt && matches!(key, Key::C | Key::X | Key::V) {
                    if *pressed {
                        todo.push(match key {
                            Key::C => TextInput::Copy,
                            Key::X => TextInput::Cut,
                            _ => TextInput::Paste(None),
                        });
                    }
                    return false;
                }
                true
            }
            _ => true,
        })
    });
    for t in todo {
        match t {
            TextInput::Key(k, m) => text_key(app, k, m),
            TextInput::Copy => {
                text_copy(app);
            }
            TextInput::Cut => {
                if text_copy(app) {
                    text_key(app, ToolKey::Delete, Mods::default());
                }
            }
            TextInput::Paste(s) => text_paste(app, s),
            TextInput::Face(f) => {
                if let Err(e) = toggle_face(app, f) {
                    app.status(e);
                }
            }
        }
    }
}

/// A compact labelled cell: short glyph label + spinner field.
fn cell(ui: &mut Ui, label: &str, tip: &str, add: impl FnOnce(&mut Ui)) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        let l =
            ui.add_sized(vec2(22.0, 24.0), egui::Label::new(egui::RichText::new(label).size(11.5).strong().color(t.text))).on_hover_text(tl!(tip));
        crate::scrub::note_label(ui, l.rect);
        add(ui);
    });
}

/// The Character panel's grid of numeric fields, `w` wide: Font Size, Leading, Kerning and
/// Tracking, then with `more` the scales, Baseline Shift and Character Rotation (`s`: the style
/// shown). The Properties panel shows its first rows.
pub(crate) fn metrics_grid(app: &mut VectorcraftApp, ui: &mut Ui, id: &str, s: &CharStyle, w: f32, more: bool) {
    let fw = ((w - 66.0) / 2.0).clamp(60.0, 100.0);
    let unit = app.session.type_unit();
    egui::Grid::new(id).num_columns(2).spacing([6.0, 4.0]).show(ui, |ui| {
        cell(ui, "T", tl!("Font Size"), |ui| size_field(app, ui, "ch-size", fw));
        cell(ui, "A↕", tl!("Leading"), |ui| {
            let leading = shared_effective(app, CharStyle::effective_leading);
            let auto = shared(app, |s| s.leading.is_none()) == Some(true);
            match widgets::spin_field_auto(ui, "ch-lead", leading, Some(auto), unit, fw, 1.0, 0.1, &SIZE_PRESETS) {
                Some(SpinPick::Value(v)) => style(app, json!({"leading": v})),
                Some(SpinPick::Auto) => style(app, json!({"leading": "auto"})),
                None => {}
            }
        });
        ui.end_row();
        cell(ui, "VA", tl!("Kerning (0 = Auto)"), |ui| {
            let kerning = shared(app, |s| s.kerning.unwrap_or(0.0));
            if let Some(v) = widgets::spin_plain(ui, "ch-kern", kerning, "", 0, fw, 10.0, -1000.0, &TRACKING_PRESETS) {
                set_kerning(app, if v == 0.0 { json!("auto") } else { json!(v) });
            }
        });
        cell(ui, "VA↔", tl!("Tracking"), |ui| {
            if let Some(v) = widgets::spin_plain(ui, "ch-track", shared(app, |s| s.tracking), "", 0, fw, 10.0, -1000.0, &TRACKING_PRESETS) {
                style(app, json!({"tracking": v}));
            }
        });
        ui.end_row();
        if !more {
            return;
        }
        cell(ui, "IT", tl!("Vertical Scale %"), |ui| {
            if let Some(v) = widgets::spin_plain(ui, "ch-vs", shared(app, |s| s.v_scale), "%", 1, fw, 1.0, 1.0, &SCALE_PRESETS) {
                format(app, json!({"vScale": v}));
            }
        });
        cell(ui, "T↔", tl!("Horizontal Scale %"), |ui| {
            if let Some(v) = widgets::spin_plain(ui, "ch-hs", shared_effective(app, |s| s.h_scale), "%", 1, fw, 1.0, 1.0, &SCALE_PRESETS) {
                format(app, json!({"hScale": v}));
            }
        });
        ui.end_row();
        cell(ui, "Aª", tl!("Baseline Shift"), |ui| {
            if let Some(v) = widgets::spin_field(ui, "ch-bs", shared_effective(app, |s| s.baseline_shift), unit, fw, 1.0, -1296.0, &[]) {
                format(app, json!({"baselineShift": v}));
            }
        });
        cell(ui, "⟲T", tl!("Character Rotation"), |ui| {
            if let Some(v) = widgets::spin_plain(ui, "ch-rot", s.rotation, "°", 1, fw, 15.0, -360.0, &super::transform::ANGLE_PRESETS) {
                format(app, json!({"rotation": v}));
            }
        });
        ui.end_row();
    });
}

/// The Font Size combo (Character and Properties panels, Control bar): typed, stepped, scrubbed or
/// picked from the presets, in the type unit, blank where the selected text's sizes differ.
pub(crate) fn size_field(app: &mut VectorcraftApp, ui: &mut Ui, id: &str, width: f32) {
    let size = shared_effective(app, |s| s.size);
    if let Some(v) = widgets::spin_field(ui, id, size, app.session.type_unit(), width, 1.0, 0.1, &SIZE_PRESETS) {
        style(app, json!({"size": v}));
    }
}

/// The font family menu and the font style dropdown of style `s` (Character and Properties panels,
/// Control bar), with their `ids` and `widths`.
pub(crate) fn font_pickers(app: &mut VectorcraftApp, ui: &mut Ui, s: &CharStyle, ids: (&str, &str), widths: (f32, f32)) {
    let sample = crate::font_menu::sample_text(app);
    if let Some(pick) = crate::font_menu::font_menu(ui, ids.0, &s.font_family, widths.0, sample.as_deref(), crate::font_menu::MenuLook::of(app)) {
        crate::font_menu::apply(app, ui.ctx(), pick);
    }
    let styles = vectorcraft_text::FontDb::global().styles(&s.font_family);
    let names: Vec<&str> = styles.iter().map(String::as_str).collect();
    if let Some(name) = widgets::dropdown_names(ui, ids.1, &s.font_style, &names, widths.1).and_then(|i| names.get(i)) {
        style(app, json!({"style": name}));
    }
}

/// The Variable Font button, `width` wide, shown when the font of style `s` is a variable font: it
/// opens the font's axis sliders ([`axis_sliders`]).
pub(crate) fn variable_font(app: &mut VectorcraftApp, ui: &mut Ui, s: &CharStyle, width: f32) {
    let Some(face) = vectorcraft_text::FontDb::global().face_version(&s.font_family, &s.font_style, s.font_version.as_deref()) else { return };
    if face.axes().iter().all(|a| a.hidden) {
        return;
    }
    let resp = widgets::flat_button(ui, tl!("Variable Font"), width);
    widgets::popover(&resp, resp.clicked(), |ui| {
        ui.set_width(240.0);
        axis_sliders(app, ui, &face);
    });
}

/// One slider and field per axis of `face`'s variable font (the axes it asks to hide left out),
/// named as the font names it: each sets that axis of the text the panel acts on
/// (`text.setFormat {fontAxes}`, or `text.setRangeStyle` while the Type tool edits text), one undo
/// step per drag. A blank field: the text's settings of that axis differ.
pub(crate) fn axis_sliders(app: &mut VectorcraftApp, ui: &mut Ui, face: &vectorcraft_text::FontFace) {
    let t = Tokens::get(ui.ctx());
    let db = vectorcraft_text::FontDb::global();
    for axis in face.axes().into_iter().filter(|a| !a.hidden) {
        let tag = String::from_utf8_lossy(&axis.tag).into_owned();
        // Each character style's setting: its own, else its face's (a named instance's, or the default).
        let setting = |s: &CharStyle| {
            s.font_axes.get(&tag).copied().or_else(|| {
                let face = db.face_version(&s.font_family, &s.font_style, s.font_version.as_deref())?;
                face.axis_value(axis.tag).map(f64::from)
            })
        };
        let value = shared(app, setting).flatten();
        let (min, max) = (f64::from(axis.min.min(axis.max)), f64::from(axis.max.max(axis.min)));
        ui.horizontal(|ui| {
            let label = widgets::dim_name(ui, &axis.name);
            crate::scrub::note_label(ui, label.rect);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(v) = widgets::spin_plain(ui, ("ch-axis-field", axis.tag), value, "", 1, 72.0, 1.0, min, &[]) {
                    axes_cmd(app, json!({ &tag: v.clamp(min, max) }), widgets::Live::Released);
                }
            });
        });
        let at = value.map_or(0.0, |v| if max > min { ((v - min) / (max - min)) as f32 } else { 0.0 });
        // A mixed value leaves the field blank and the slider without a value.
        let shown = value.map(|v| widgets::SliderValue::field(v, min..=max, "", 1));
        let (picked, phase) =
            widgets::color_slider_valued(ui, ("ch-axis", axis.tag), &axis.name, shown, at, ui.available_width(), &|_| t.button_border);
        if let Some(p) = picked {
            let v = ((min + f64::from(p) * (max - min)) * 10.0).round() / 10.0;
            axes_cmd(app, json!({ &tag: v }), phase);
        }
    }
}

/// Set Variable Font settings `axes` on the text the panel acts on, live while a slider drags.
fn axes_cmd(app: &mut VectorcraftApp, axes: Value, phase: widgets::Live) {
    match text_editing(app) {
        Some((id, a, b)) => {
            end_typing(app);
            let mut q = json!({"id": id.0, "fontAxes": axes});
            if b > a {
                q["start"] = json!(a);
                q["end"] = json!(b);
            }
            super::live_run(app, "Character", "text.setRangeStyle", q, phase);
        }
        None => super::live_run(app, "Character", "text.setFormat", json!({ "fontAxes": axes }), phase),
    }
}

/// The Control bar's type controls while type is selected or edited: the Character link (the
/// Character panel in a popover), the font family and style menus and the Font Size combo.
pub(crate) fn control_bar(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some((s, _)) = text_style(app) else { return };
    let link = ui
        .link(egui::RichText::new(tl!("Character:")).size(12.0).color(Tokens::get(ui.ctx()).text).underline())
        .on_hover_text(tl!("Character options"));
    widgets::popover(&link, link.clicked(), |ui| {
        ui.set_width(260.0);
        show(app, ui);
    });
    font_pickers(app, ui, &s, ("cb-font", "cb-font-style"), (150.0, 96.0));
    size_field(app, ui, "cb-font-size", 96.0);
    ui.add_space(4.0);
    ui.separator();
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some((s, _)) = text_style(app) else {
        super::empty_state(ui, "type", tl!("No text selected"), tl!("Select a text object to edit its character attributes."));
        return;
    };
    let w = ui.available_width();
    font_pickers(app, ui, &s, ("ch-font", "ch-style"), (w - 4.0, w - 4.0));
    variable_font(app, ui, &s, w - 4.0);
    ui.add_space(2.0);
    let hidden = pstate::<bool>(ui.ctx(), "ch-hide-options");
    metrics_grid(app, ui, "ch-grid", &s, w, !hidden);
    if hidden {
        return;
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let small = s.small_caps.is_some();
        let position = s.position.id();
        for (kind, tip, on, change) in [
            (Glyph::AllCaps, tl!("All Caps"), s.all_caps, json!({"allCaps": !s.all_caps})),
            (Glyph::SmallCaps, tl!("Small Caps"), small, json!({"smallCaps": !small})),
            (Glyph::Super, tl!("Superscript"), position == "superscript", position_toggle(position, "superscript")),
            (Glyph::Sub, tl!("Subscript"), position == "subscript", position_toggle(position, "subscript")),
            (Glyph::Underline, tl!("Underline"), s.underline, json!({"underline": !s.underline})),
            (Glyph::Strike, tl!("Strikethrough"), s.strikethrough, json!({"strikethrough": !s.strikethrough})),
        ] {
            if style_toggle(ui, kind, tip, on) {
                format(app, change);
            }
        }
    });
}

/// The `text.setFormat` params of the Superscript or Subscript toggle `p` for text whose
/// position is `position`: `normal` when `p` is on, else `p`, which turns the other one off.
fn position_toggle(position: &str, p: &str) -> Value {
    json!({"position": if position == p { "normal" } else { p }})
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Glyph {
    AllCaps,
    SmallCaps,
    Super,
    Sub,
    Underline,
    Strike,
}

/// A drawn "T" style toggle (All Caps, Small Caps, Superscript, Subscript, Underline, Strikethrough).
fn style_toggle(ui: &mut Ui, g: Glyph, tip: &str, on: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(30.0, 26.0), egui::Sense::click());
    if on {
        ui.painter().rect_filled(r, 3, t.tool_active);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, 3, t.hover);
    }
    let col = t.text_strong;
    let big = egui::FontId::proportional(15.0);
    let small = egui::FontId::proportional(10.5);
    let c = r.center();
    let p = ui.painter();
    match g {
        Glyph::AllCaps => {
            p.text(c - vec2(5.0, 0.0), egui::Align2::CENTER_CENTER, "T", big.clone(), col);
            p.text(c + vec2(5.0, 0.0), egui::Align2::CENTER_CENTER, "T", big, col);
        }
        Glyph::SmallCaps => {
            p.text(c - vec2(4.0, 0.0), egui::Align2::CENTER_CENTER, "T", big, col);
            p.text(c + vec2(6.0, 2.0), egui::Align2::CENTER_CENTER, "T", small, col);
        }
        Glyph::Super => {
            p.text(c - vec2(3.0, 0.0), egui::Align2::CENTER_CENTER, "T", big, col);
            p.text(c + vec2(6.0, -5.0), egui::Align2::CENTER_CENTER, "1", small, col);
        }
        Glyph::Sub => {
            p.text(c - vec2(3.0, 0.0), egui::Align2::CENTER_CENTER, "T", big, col);
            p.text(c + vec2(6.0, 5.0), egui::Align2::CENTER_CENTER, "1", small, col);
        }
        Glyph::Underline => {
            p.text(c - vec2(0.0, 1.0), egui::Align2::CENTER_CENTER, "T", big, col);
            p.line_segment([c + vec2(-5.0, 7.0), c + vec2(5.0, 7.0)], egui::Stroke::new(1.2, col));
        }
        Glyph::Strike => {
            p.text(c, egui::Align2::CENTER_CENTER, "T", big, col);
            p.line_segment([c + vec2(-6.0, 1.0), c + vec2(6.0, 1.0)], egui::Stroke::new(1.2, col));
        }
    }
    let resp = resp.on_hover_text(tl!(tip));
    resp.clicked()
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let st = text_style(app);
    let has = st.is_some();
    let hidden: bool = pstate(ui.ctx(), "ch-hide-options");
    if menu_item(ui, if hidden { tl!("Show Options") } else { tl!("Hide Options") }, true, false) {
        set_pstate(ui.ctx(), "ch-hide-options", !hidden);
    }
    ui.separator();
    let s = st.map(|x| x.0);
    for (label, key, on) in [
        (tl!("All Caps"), "allCaps", s.as_ref().is_some_and(|s| s.all_caps)),
        (tl!("Underline"), "underline", s.as_ref().is_some_and(|s| s.underline)),
        (tl!("Strikethrough"), "strikethrough", s.as_ref().is_some_and(|s| s.strikethrough)),
    ] {
        if menu_item(ui, label, has, on) {
            format(app, json!({key: !on}));
        }
    }
    let small = s.as_ref().is_some_and(|s| s.small_caps.is_some());
    let position = s.as_ref().map_or("normal", |s| s.position.id());
    for (label, on, change) in [
        (tl!("Small Caps"), small, json!({"smallCaps": !small})),
        (tl!("Superscript"), position == "superscript", position_toggle(position, "superscript")),
        (tl!("Subscript"), position == "subscript", position_toggle(position, "subscript")),
    ] {
        if menu_item(ui, label, has, on) {
            format(app, change);
        }
    }
    ui.separator();
    // Character Alignment: where characters smaller than the largest on their line line up, with
    // the East Asian options.
    if app.session.prefs.show_east_asian_options {
        let align = s.as_ref().map(|s| s.char_align);
        ui.add_enabled_ui(has, |ui| {
            // Indented like the items beside it (their check column).
            ui.menu_button(format!("   {}", tl!("Character Alignment")), |ui| {
                use vectorcraft_doc::CharAlign;
                for (label, a, key) in [
                    (tl!("Roman Baseline"), CharAlign::RomanBaseline, "romanBaseline"),
                    (tl!("Em Box Top/Right"), CharAlign::EmBoxTop, "emBoxTop"),
                    (tl!("Em Box Center"), CharAlign::EmBoxCenter, "emBoxCenter"),
                    (tl!("Em Box Bottom/Left"), CharAlign::EmBoxBottom, "emBoxBottom"),
                    (tl!("ICF Top/Right"), CharAlign::IcfTop, "icfTop"),
                    (tl!("ICF Bottom/Left"), CharAlign::IcfBottom, "icfBottom"),
                ] {
                    if menu_item(ui, label, true, align == Some(a)) {
                        format(app, json!({"charAlign": key}));
                    }
                }
            });
        });
        // Proportional Metrics: full-width glyphs on the font's proportional widths (`palt`, `vpal`).
        let proportional = s.as_ref().is_some_and(|s| s.proportional_metrics);
        if menu_item(ui, tl!("Proportional Metrics"), has, proportional) {
            format(app, json!({"proportionalMetrics": !proportional}));
        }
        ui.separator();
    }
    // Digits: the numeral system the selected characters' digits are drawn in, with the Middle
    // Eastern (Indic) options, as Paragraph Direction is in the Paragraph panel.
    if app.session.prefs.show_indic_options {
        let digits = s.as_ref().map(|s| s.digits);
        ui.add_enabled_ui(has, |ui| {
            ui.menu_button(format!("   {}", tl!("Digits")), |ui| {
                use vectorcraft_doc::Digits;
                for (label, d, key) in [
                    (tl!("Default"), Digits::Default, "default"),
                    (tl!("Arabic"), Digits::Arabic, "arabic"),
                    (tl!("Hindi"), Digits::Hindi, "hindi"),
                    (tl!("Farsi"), Digits::Farsi, "farsi"),
                ] {
                    if menu_item(ui, label, true, digits == Some(d)) {
                        format(app, json!({"digits": key}));
                    }
                }
            });
        });
        ui.separator();
    }
    for l in ["Standard Vertical Roman Alignment", "Tate-chu-yoko", "Fractional Widths", "System Layout", "No Break"] {
        menu_item(ui, tl!(l), false, l == "Fractional Widths");
    }
    ui.separator();
    // The installed fonts are always listed; this picks up fonts installed since the app started.
    #[cfg(not(target_arch = "wasm32"))]
    if menu_item(ui, tl!("Refresh Font List"), true, false)
        && let Ok(r) = app.run("text.rescanFonts", json!({}))
    {
        app.status(format!("{} font families available", r["families"]));
    }
    if menu_item(ui, tl!("Reset Panel"), has, false) {
        reset_panel(app);
    }
}

/// Reset Panel: the characters the panel acts on ([`char_cmd`]) take tracking 0, Auto leading and
/// kerning, no baseline shift, 100% scales, no rotation, no underline or strikethrough, All Caps
/// and Small Caps off, the normal position (neither superscript nor subscript), Roman Baseline
/// character alignment, Proportional Metrics off and the Default digits, whether or not Show
/// East Asian Options and Show Indic Options are on.
fn reset_panel(app: &mut VectorcraftApp) {
    let d = CharStyle::default();
    style(app, json!({"tracking": 0, "leading": "auto"}));
    format(
        app,
        json!({
            "kerning": "auto", "baselineShift": 0, "hScale": 100, "vScale": 100, "rotation": 0, "underline": false, "strikethrough": false,
            "allCaps": false, "smallCaps": d.small_caps.is_some(), "position": d.position.id(), "charAlign": d.char_align,
            "proportionalMetrics": d.proportional_metrics, "digits": d.digits
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    #[test]
    fn scaled_dimensions_do_not_show_false_mixed_values_from_roundoff() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        let a = app.run("text.create", json!({"x": 20, "y": 50, "text": "Axis", "size": 12})).unwrap()["id"].clone();
        app.run("object.scale", json!({"sx": 120, "sy": 110})).unwrap();
        let b = app.run("text.create", json!({"x": 20, "y": 90, "text": "Label", "size": 24})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        app.run("text.setStyle", json!({"size": 7, "leading": 9})).unwrap();
        app.run("text.setFormat", json!({"hScale": 130, "baselineShift": 3})).unwrap();
        for (actual, expected) in [
            (shared_effective(&app, |s| s.size), 7.0),
            (shared_effective(&app, CharStyle::effective_leading), 9.0),
            (shared_effective(&app, |s| s.h_scale), 130.0),
            (shared_effective(&app, |s| s.baseline_shift), 3.0),
        ] {
            assert!((actual.expect("equal dimensions must not be mixed") - expected).abs() < 1e-8);
        }
    }

    #[test]
    fn scaled_collapsed_text_keeps_nondimensional_controls_available() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        app.run("text.create", json!({"x": 20, "y": 50, "text": "Axis", "size": 12})).unwrap();
        app.run("object.transform", json!({"matrix": [0, 0, 0, 0, 0, 0]})).unwrap();
        assert!(text_style(&app).is_some());
        assert_eq!(shared(&app, |s| s.tracking), Some(0.0));
        assert_eq!(shared_effective(&app, |s| s.size), None);
        app.run("text.setStyle", json!({"tracking": 40})).unwrap();
        app.run("text.setFormat", json!({"leftIndent": 4})).unwrap();
        assert_eq!(shared(&app, |s| s.tracking), Some(40.0));
        assert_eq!(text_style(&app).unwrap().1.left_indent, 4.0);
        let ctx = egui::Context::default();
        let texts = super::super::tests_appearance::frame_events(&ctx, &mut app, vec![], show);
        assert!(!texts.iter().any(|(s, _)| s == "No text selected"));
    }

    #[test]
    fn scaled_character_fields_show_document_sizes_and_mixed_group_values() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        let a = app.run("text.create", json!({"x": 20, "y": 50, "text": "Axis", "size": 12})).unwrap()["id"].clone();
        app.run("object.scale", json!({"sx": 200})).unwrap();
        assert_eq!(shared_effective(&app, |s| s.size), Some(24.0));
        assert_eq!(text_style(&app).unwrap().0.size, 12.0, "style definitions keep stored attributes");
        let b = app.run("text.create", json!({"x": 20, "y": 90, "text": "Label", "size": 24})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        app.run("object.group", json!({})).unwrap();
        assert_eq!(shared_effective(&app, |s| s.size), Some(24.0), "different local sizes, same displayed size");
        app.run("object.scale", json!({"sx": 200, "sy": 100})).unwrap();
        assert_eq!(shared_effective(&app, |s| s.h_scale), Some(200.0));
        app.run("text.setStyle", json!({"size": 10})).unwrap();
        assert_eq!(shared_effective(&app, |s| s.size), Some(10.0));
        app.run("text.setFormat", json!({"hScale": 100})).unwrap();
        assert_eq!(shared_effective(&app, |s| s.h_scale), Some(100.0));
    }

    #[test]
    fn scaled_range_fields_follow_the_type_tool_selection() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        app.session.select_tool("type", Default::default()).unwrap();
        let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "Axis label", "size": 12})).unwrap()["id"].clone();
        app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
        assert!(text_editing(&app).is_some());
        app.run("object.scale", json!({"sx": 200})).unwrap();
        app.run("text.setRangeStyle", json!({"id": id, "start": 0, "end": 4, "size": 10})).unwrap();
        app.session.set_tool_option("selectAll", &json!(true));
        assert_eq!(shared_effective(&app, |s| s.size), None);
        app.session.set_tool_option("select", &json!({"start": 0, "end": 4}));
        assert_eq!(shared_effective(&app, |s| s.size), Some(10.0));
        assert_eq!(text_style(&app).unwrap().0.size, 5.0);
    }

    /// The Kerning field at a caret kerns that pair, not the whole text (#2217).
    #[test]
    fn kerning_at_a_caret_sets_the_pair_not_the_whole_text() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        app.session.select_tool("type", Default::default()).unwrap();
        let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "AVAX", "size": 12})).unwrap()["id"].clone();
        app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
        app.session.set_tool_option("select", &json!({"start": 2, "end": 2}));
        set_kerning(&mut app, json!(-50));
        let kerning = |app: &VectorcraftApp, at: usize| {
            let st = app.session.active().unwrap();
            let Some(NodeKind::Text(t)) = st.doc.node(NodeId(id.as_u64().unwrap())).map(|n| &n.kind) else { panic!("text") };
            vectorcraft_text::edit::style_at(&t.runs, at + 1).kerning // the character at `at`
        };
        assert_eq!(kerning(&app, 1), Some(-50.0), "the character before the caret");
        assert_eq!(kerning(&app, 3), None, "the rest is untouched");
        assert_eq!(kerning(&app, 0), None);
        // A selected range still takes it whole.
        app.session.set_tool_option("select", &json!({"start": 2, "end": 4}));
        set_kerning(&mut app, json!(30));
        assert_eq!((kerning(&app, 1), kerning(&app, 2), kerning(&app, 3)), (Some(-50.0), Some(30.0), Some(30.0)));
    }

    /// A variable font gets the Variable Font button, which opens one slider per axis named as the
    /// font names it; a drag along one sets that axis of the selected type in one undo step.
    #[test]
    fn variable_fonts_get_a_slider_per_axis() {
        use vectorcraft_text::test_fonts::{TWO_AXIS_FAMILY, two_axis_font};
        let db = vectorcraft_text::FontDb::global();
        db.add_font(two_axis_font().unwrap());
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        let plain = app.run("text.create", json!({"x": 20, "y": 50, "text": "Static"})).unwrap()["id"].clone();
        let id = app.run("text.create", json!({"x": 20, "y": 120, "text": "lIn", "font": TWO_AXIS_FAMILY})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [plain]})).unwrap();
        assert!(!crate::tests_labels::painted_text(&mut app, show).contains("Variable Font"), "a static font");
        app.run("select.set", json!({"ids": [id]})).unwrap();
        assert!(crate::tests_labels::painted_text(&mut app, show).contains("Variable Font"));
        let face = db.face(TWO_AXIS_FAMILY, "Regular").unwrap();
        let text = crate::tests_labels::painted_text(&mut app, |app, ui| axis_sliders(app, ui, &face));
        assert!(text.contains("Weight\n") && text.contains("Width\n") && text.contains("400") && text.contains("100"), "{text}");
        // The labels are the font's names in every interface language, not catalog words (ja
        // translates the interface's Weight as 線幅, the stroke weight).
        crate::i18n::set_current(crate::i18n::Lang::from_code("ja").unwrap());
        let text = crate::tests_labels::painted_text(&mut app, |app, ui| axis_sliders(app, ui, &face));
        crate::i18n::set_current(crate::i18n::Lang::EN);
        assert!(text.contains("Weight\n") && text.contains("Width\n") && !text.contains("線幅"), "{text}");
        // Screen readers name each slider by its axis.
        let tree = crate::widgets::accessibility_tests::tree(|ui| axis_sliders(&mut app, ui, &face));
        for name in ["Weight", "Width"] {
            assert!(tree.iter().any(|n| n.role() == egui::accesskit::Role::Slider && n.label() == Some(name)), "{name}");
        }
        // ...with the number its field shows, on the axis's range.
        use crate::widgets::accessibility_tests::slider_value;
        let wght = face.axes().into_iter().find(|a| &a.tag == b"wght").unwrap();
        let range = (Some(f64::from(wght.min)), Some(f64::from(wght.max)));
        assert_eq!(slider_value(&tree, "Weight"), (Some(400.0), range.0, range.1, Some("400".into())));
        // Drag the Weight slider from its start to past its end.
        let ctx = egui::Context::default();
        let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let mut slider = None;
            let input = egui::RawInput { events, ..Default::default() };
            ctx.run_ui(input, |ui| {
                axis_sliders(app, ui, &face);
                slider = ctx.read_response(ui.id().with(("ch-axis", *b"wght"))).map(|r| r.rect);
            })
            .textures_delta
            .clear();
            slider
        };
        frame(&mut app, vec![]);
        let r = frame(&mut app, vec![]).expect("the Weight slider is drawn");
        let none = egui::Modifiers::NONE;
        let (from, to) = (egui::pos2(r.left() + 4.0, r.center().y), egui::pos2(r.right() + 20.0, r.center().y));
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: none };
        frame(&mut app, vec![egui::Event::PointerMoved(from), button(from, true)]);
        for i in 1..=4 {
            frame(&mut app, vec![egui::Event::PointerMoved(from + (to - from) * i as f32 / 4.0)]);
        }
        frame(&mut app, vec![button(to, false)]);
        frame(&mut app, vec![]);
        let axes = |app: &VectorcraftApp| shared(app, |s| s.font_axes.clone()).unwrap();
        assert_eq!(axes(&app).into_iter().collect::<Vec<_>>(), [("wght".to_string(), 700.0)]);
        app.run("edit.undo", json!({})).unwrap();
        assert!(axes(&app).is_empty(), "one undo step");
        // Regular and Bold selected: the weights differ and the field is blank; the widths agree.
        let bold = app.run("text.create", json!({"x": 20, "y": 160, "text": "lIn", "font": TWO_AXIS_FAMILY, "style": "Bold"})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [id, bold]})).unwrap();
        let text = crate::tests_labels::painted_text(&mut app, |app, ui| axis_sliders(app, ui, &face));
        assert!(!text.contains("400") && !text.contains("700") && text.contains("100"), "{text}");
        let tree = crate::widgets::accessibility_tests::tree(|ui| axis_sliders(&mut app, ui, &face));
        assert_eq!(slider_value(&tree, "Weight"), (None, None, None, None), "a blank field: no value");
        assert_eq!(slider_value(&tree, "Width").0, Some(100.0));
        // Reset Panel leaves the settings, as it leaves the font and style.
        app.run("select.set", json!({"ids": [id]})).unwrap();
        app.run("text.setFormat", json!({"fontAxes": {"wght": 650}})).unwrap();
        reset_panel(&mut app);
        assert_eq!(axes(&app).into_iter().collect::<Vec<_>>(), [("wght".to_string(), 650.0)]);
    }

    /// Bold and Italic pick the family's own faces by name, and toggle back (#724).
    #[test]
    fn bold_and_italic_pick_the_familys_own_faces() {
        let styles: Vec<String> =
            ["Light", "Narrow", "Regular", "Italic", "Narrow Italic", "SemiBold", "Bold", "Bold Condensed", "Narrow Bold", "Bold Italic", "Black"]
                .iter()
                .map(|s| s.to_string())
                .collect();
        assert_eq!(pick_face(&styles, true, false, "Regular"), Some("Bold"), "not Bold Condensed, Narrow Bold or SemiBold");
        assert_eq!(pick_face(&styles, true, true, "Italic"), Some("Bold Italic"));
        assert_eq!(pick_face(&styles, false, true, "Regular"), Some("Italic"));
        assert_eq!(pick_face(&styles, false, false, "Italic"), Some("Regular"), "upright again, not Narrow");
        assert_eq!(pick_face(&styles, true, false, "Narrow"), Some("Narrow Bold"), "a Narrow face stays Narrow");
        assert_eq!(pick_face(&styles, false, true, "Narrow"), Some("Narrow Italic"));
        let oblique: Vec<String> = ["Book", "Oblique", "Bold-Oblique"].iter().map(|s| s.to_string()).collect();
        assert_eq!(pick_face(&oblique, true, true, "Oblique"), Some("Bold-Oblique"));
        assert_eq!(pick_face(&oblique, false, false, "Oblique"), None, "Book has a weight word: no regular face");
        assert_eq!(face_of("Bold Italic"), (true, false, true, vec![]));
        assert_eq!(face_of("SemiBold"), (false, false, false, vec![]));
        assert_eq!(face_of("Narrow Italic"), (false, true, true, vec!["narrow".to_string()]));
        // No type selected: a message, nothing changed.
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        assert!(toggle_face(&mut app, Face::Bold).is_err());
        assert!(!crate::menus::enabled(&app, "type.bold"));
    }

    /// From one frame of the panel: a point inside each of the Small Caps, Superscript and
    /// Subscript buttons, and which of them are drawn on. The two buttons with a "1" are
    /// Superscript (the "1" up) and Subscript; Small Caps is the button before them.
    fn script_buttons(ctx: &egui::Context, app: &mut VectorcraftApp) -> ([egui::Pos2; 3], [bool; 3]) {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
        out.textures_delta.clear();
        fn walk(s: &egui::Shape, on: egui::Color32, ones: &mut Vec<egui::Pos2>, lit: &mut Vec<egui::Rect>) {
            match s {
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, on, ones, lit)),
                egui::Shape::Text(t) if t.galley.text() == "1" => ones.push(t.visual_bounding_rect().center()),
                egui::Shape::Rect(r) if r.fill == on && r.rect.size() == vec2(30.0, 26.0) => lit.push(r.rect),
                _ => {}
            }
        }
        let (mut ones, mut lit) = (vec![], vec![]);
        for c in &out.shapes {
            walk(&c.shape, Tokens::get(ctx).tool_active, &mut ones, &mut lit);
        }
        let [sup, sub] = ones[..] else { panic!("two script buttons: {ones:?}") };
        assert!(sup.x < sub.x && sup.y < sub.y, "Superscript, then Subscript: {ones:?}");
        // Each "1" is drawn 6 points right of its button's center and 5 points above or below it.
        let y = (sup.y + sub.y) / 2.0;
        let (sup, sub) = (egui::pos2(sup.x - 6.0, y), egui::pos2(sub.x - 6.0, y));
        let points = [sup - vec2(sub.x - sup.x, 0.0), sup, sub];
        (points, points.map(|p| lit.iter().any(|r| r.contains(p))))
    }

    /// Each run's (small caps, position).
    fn scripts(app: &VectorcraftApp, id: &Value) -> Vec<(Option<f64>, &'static str)> {
        match &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind {
            NodeKind::Text(t) => t.runs.iter().map(|r| (r.style.small_caps, r.style.position.id())).collect(),
            _ => panic!("text"),
        }
    }

    /// The Small Caps, Superscript and Subscript buttons set the selected type's small caps and
    /// position, with one undo step for each click, and show them; Superscript and Subscript turn
    /// each other off. While the Type tool edits, they set and show the selected characters.
    #[test]
    fn small_caps_superscript_and_subscript_buttons_set_and_show_the_text() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        app.session.select_tool("type", Default::default()).unwrap();
        let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "E=mc2", "size": 12})).unwrap()["id"].clone();
        let ctx = egui::Context::default();
        let (at, lit) = script_buttons(&ctx, &mut app);
        assert_eq!(lit, [false; 3]);
        let small = Some(app.session.active().unwrap().doc.setup.small_caps_size);
        for (button, runs, shown) in [
            (0, (small, "normal"), [true, false, false]),
            (1, (small, "superscript"), [true, true, false]),
            (2, (small, "subscript"), [true, false, true]),
            (2, (small, "normal"), [true, false, false]),
            (1, (small, "superscript"), [true, true, false]),
            (1, (small, "normal"), [true, false, false]),
            (0, (None, "normal"), [false; 3]),
        ] {
            let undo = app.session.active().unwrap().history.undo.len();
            super::super::tests_appearance::click(&ctx, &mut app, at[button], show);
            assert_eq!(scripts(&app, &id), [runs], "button {button}");
            assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "button {button}");
            assert_eq!(script_buttons(&ctx, &mut app).1, shown, "button {button}");
        }
        // The Type tool's selection: "2" alone.
        app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
        app.session.set_tool_option("select", &json!({"start": 4, "end": 5}));
        assert!(text_editing(&app).is_some());
        super::super::tests_appearance::click(&ctx, &mut app, at[1], show);
        assert_eq!(scripts(&app, &id), [(None, "normal"), (None, "superscript")]);
        assert_eq!(script_buttons(&ctx, &mut app).1, [false, true, false]);
        app.session.set_tool_option("select", &json!({"start": 0, "end": 4}));
        assert_eq!(script_buttons(&ctx, &mut app).1, [false; 3]);
    }

    /// The panel menu's Small Caps, Superscript and Subscript set the selected type's small caps
    /// and position and show a check mark when on; Superscript and Subscript turn each other off.
    #[test]
    fn small_caps_superscript_and_subscript_menu_items_set_and_check_the_text() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "E=mc2", "size": 12})).unwrap()["id"].clone();
        let ctx = egui::Context::default();
        let checked = |app: &mut VectorcraftApp| {
            let texts = super::super::tests_appearance::frame_events(&ctx, app, vec![], menu);
            ["Small Caps", "Superscript", "Subscript"].map(|l| texts.iter().any(|(t, _)| *t == format!("✓ {l}")))
        };
        assert_eq!(checked(&mut app), [false; 3]);
        let small = Some(app.session.active().unwrap().doc.setup.small_caps_size);
        for (item, runs, shown) in [
            ("Small Caps", (small, "normal"), [true, false, false]),
            ("Superscript", (small, "superscript"), [true, true, false]),
            ("Subscript", (small, "subscript"), [true, false, true]),
            ("Subscript", (small, "normal"), [true, false, false]),
            ("Small Caps", (None, "normal"), [false; 3]),
        ] {
            let texts = super::super::tests_appearance::frame_events(&ctx, &mut app, vec![], menu);
            let at = texts.iter().find(|(t, _)| t.trim_start_matches(['✓', ' ']) == item).unwrap_or_else(|| panic!("{item}: {texts:?}")).1;
            super::super::tests_appearance::click(&ctx, &mut app, at.center(), menu);
            assert_eq!(scripts(&app, &id), [runs], "{item}");
            assert_eq!(checked(&mut app), shown, "{item}");
        }
    }

    /// Character Alignment and Proportional Metrics are in the panel menu with the East Asian
    /// options only (as Mojikumi Set and Top-to-Top Leading are in the Paragraph panel).
    #[test]
    fn character_alignment_shows_with_the_east_asian_options() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "雅楽"})).unwrap()["id"].clone();
        app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
        let shown = |app: &mut VectorcraftApp| {
            let text = crate::tests_labels::painted_text(app, menu);
            (text.contains("Character Alignment"), text.contains("Proportional Metrics"))
        };
        assert_eq!(shown(&mut app), (false, false));
        app.session.prefs.show_east_asian_options = true;
        assert_eq!(shown(&mut app), (true, true));
    }

    /// Digits is in the panel menu with the Middle Eastern (Indic) options only, as Paragraph
    /// Direction is in the Paragraph panel.
    #[test]
    fn digits_show_with_the_indic_options() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "صفحة 3"})).unwrap()["id"].clone();
        app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
        let shown = |app: &mut VectorcraftApp| crate::tests_labels::painted_text(app, menu).contains("Digits");
        assert!(!shown(&mut app));
        app.session.prefs.show_indic_options = true;
        assert!(shown(&mut app));
    }

    /// Reset Panel sets every character attribute the panel shows, except the font family, style
    /// and size, back to its default (Small Caps, the position, Character Alignment, Proportional
    /// Metrics and Digits included), in the selected type and in the text the Type tool has
    /// selected while it edits.
    #[test]
    fn reset_panel_resets_every_attribute_but_the_font() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "One 12"})).unwrap()["id"].as_u64().unwrap();
        let runs = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => t.runs.iter().map(|r| (r.text.clone(), r.style.clone())).collect::<Vec<_>>(),
            _ => panic!("text"),
        };
        let differs = |s: &CharStyle, d: &CharStyle| {
            [
                s.tracking != d.tracking,
                s.leading != d.leading,
                s.kerning != d.kerning,
                s.baseline_shift != d.baseline_shift,
                s.h_scale != d.h_scale,
                s.v_scale != d.v_scale,
                s.rotation != d.rotation,
                s.underline != d.underline,
                s.strikethrough != d.strikethrough,
                s.all_caps != d.all_caps,
                s.small_caps != d.small_caps,
                s.position != d.position,
                s.char_align != d.char_align,
                s.proportional_metrics != d.proportional_metrics,
                s.digits != d.digits,
            ]
        };
        let custom = |app: &mut VectorcraftApp| {
            app.run(
                "text.setRangeStyle",
                json!({
                    "id": id, "tracking": 50, "leading": 30, "kerning": 20, "baselineShift": 3, "hScale": 120, "vScale": 90, "rotation": 15,
                    "underline": true, "strikethrough": true, "allCaps": true, "smallCaps": true, "position": "superscript",
                    "charAlign": "emBoxCenter", "proportionalMetrics": true, "digits": "hindi"
                }),
            )
            .unwrap();
        };
        let plain = runs(&app)[0].1.clone();
        assert_eq!(differs(&plain, &CharStyle::default()), [false; 15]);
        custom(&mut app);
        let styled = runs(&app)[0].1.clone();
        assert_eq!(differs(&styled, &CharStyle::default()), [true; 15]);
        app.run("select.set", json!({"ids": [id]})).unwrap();
        reset_panel(&mut app);
        assert_eq!(runs(&app), [("One 12".to_string(), plain.clone())]);
        // While the Type tool edits text, Reset Panel resets only its selection.
        custom(&mut app);
        app.select_tool("type");
        app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
        app.session.set_tool_option("select", &json!({"start": 4, "end": 6}));
        assert!(text_editing(&app).is_some());
        reset_panel(&mut app);
        assert_eq!(runs(&app), [("One ".to_string(), styled), ("12".to_string(), plain)]);
    }
}
