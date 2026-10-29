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

/// A boolean on every selected text object (`Some(true)` / `Some(false)`), or `None` when they
/// differ. When the Type tool edits text, only that object counts.
pub(crate) fn shared_text_flag(app: &VectorcraftApp, f: impl Fn(&vectorcraft_doc::TextObject) -> bool) -> Option<bool> {
    if let Some((id, _, _)) = text_editing(app) {
        return app.session.active().and_then(|st| st.doc.node(id)).and_then(|n| match &n.kind {
            NodeKind::Text(t) => Some(f(t)),
            _ => None,
        });
    }
    let st = app.session.active()?;
    let mut value: Option<bool> = None;
    let mut mixed = false;
    let mut saw = false;
    for node in st.selection.objects.iter().filter_map(|id| st.doc.node(*id)) {
        node.walk(&mut |n| {
            if let NodeKind::Text(t) = &n.kind {
                saw = true;
                if mixed {
                    return;
                }
                let v = f(t);
                match value {
                    None => value = Some(v),
                    Some(a) if a == v => {}
                    Some(_) => mixed = true,
                }
            }
        });
    }
    if !saw || mixed { None } else { value }
}

/// `f` of the character styles the panels act on: the runs the Type tool's selection covers (the
/// caret's style at a caret), else every run of the selected type, groups' included. `None` (a
/// blank field) where they differ, or with no type.
pub(crate) fn shared<T: PartialEq>(app: &VectorcraftApp, f: impl Fn(&CharStyle) -> T) -> Option<T> {
    shared_styles(app, |_, _, style| Some(f(style)))
}

/// Dimensional controls use document units and tolerate arithmetic roundoff when comparing
/// compensated runs. Keep raw styles available to font/paragraph controls and style definitions,
/// even if a collapsed affine prevents editing dimensions.
fn shared_effective(app: &VectorcraftApp, f: impl Fn(&CharStyle) -> f64) -> Option<f64> {
    shared_styles(app, |t, _, style| t.effective_char_style(style).map(|s| Metric(f(&s)))).map(|v| v.0)
}

/// The Leading shown: each character's own, or for Auto leading the share of its size its
/// paragraph's Justification › Auto Leading sets, as the layout uses.
fn shared_leading(app: &VectorcraftApp) -> Option<f64> {
    shared_styles(app, |t, para, style| t.effective_char_style(style).map(|s| Metric(para.effective_leading(&s)))).map(|v| v.0)
}

pub(crate) struct Metric(pub(crate) f64);

impl PartialEq for Metric {
    fn eq(&self, other: &Self) -> bool {
        (self.0 - other.0).abs() <= 1e-9 * self.0.abs().max(other.0.abs()).max(1.0)
    }
}

/// `f` of each character style in the selection, with its text object and the paragraph the
/// characters are in (a run that spans paragraphs counts once for each).
fn shared_styles<T: PartialEq>(
    app: &VectorcraftApp,
    f: impl Fn(&vectorcraft_doc::TextObject, &vectorcraft_doc::ParaStyle, &CharStyle) -> Option<T>,
) -> Option<T> {
    let st = app.session.active()?;
    let text = |id: NodeId| match st.doc.node(id).map(|n| &n.kind) {
        Some(NodeKind::Text(t)) => Some(t),
        _ => None,
    };
    let mut acc = Shared::None;
    if let Some((id, a, b)) = text_editing(app) {
        let t = text(id)?;
        if b <= a {
            let para = t.plain_text().get(..a).map_or(0, |s| s.bytes().filter(|&c| c == b'\n').count());
            return f(t, t.para_at(para), &vectorcraft_text::edit::insertion_style(&t.runs, a, b));
        }
        each_paragraph_run(t, Some((a, b)), |para, style| acc.add(f(t, para, style)));
    } else {
        for n in st.selection.objects.iter().filter_map(|id| st.doc.node(*id)) {
            n.walk(&mut |c| {
                if let NodeKind::Text(t) = &c.kind {
                    each_paragraph_run(t, None, |para, style| acc.add(f(t, para, style)));
                }
            });
        }
    }
    acc.value().flatten()
}

/// `f` of the paragraph and style of each piece of `t`'s runs that lies in one paragraph (a
/// paragraph's closing line break belongs to it): the pieces within bytes `a..b`, or with no
/// range every run's, empty ones included. With no range, a final empty paragraph (the text ends
/// in a line break) counts too, with the style of the run before it, as the caret there types.
fn each_paragraph_run<'a>(
    t: &'a vectorcraft_doc::TextObject,
    range: Option<(usize, usize)>,
    mut f: impl FnMut(&'a vectorcraft_doc::ParaStyle, &'a CharStyle),
) {
    let (mut end, mut para) = (0, 0);
    // The style of the last character and whether it is a line break: the paragraph after it has
    // no characters of its own.
    let mut last: Option<(&'a CharStyle, bool)> = None;
    for r in &t.runs {
        if r.text.is_empty() {
            if range.is_none_or(|(a, b)| a < end && end < b) {
                f(t.para_at(para), &r.style);
            }
            continue;
        }
        for piece in r.text.split_inclusive('\n') {
            let start = end;
            end += piece.len();
            if range.is_none_or(|(a, b)| start < b && end > a) {
                f(t.para_at(para), &r.style);
            }
            para += usize::from(piece.ends_with('\n'));
            last = Some((&r.style, piece.ends_with('\n')));
        }
    }
    if range.is_none()
        && let Some((style, true)) = last
    {
        f(t.para_at(para), style);
    }
}

/// The value a run of styles share so far ([`shared`]).
pub(crate) enum Shared<T> {
    None,
    Same(T),
    Differ,
}

impl<T: PartialEq> Shared<T> {
    pub(crate) fn add(&mut self, v: T) {
        match self {
            Shared::None => *self = Shared::Same(v),
            Shared::Same(x) if *x != v => *self = Shared::Differ,
            _ => {}
        }
    }
    pub(crate) fn value(self) -> Option<T> {
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

/// The Character panel fields that the Highlight Font Name, Highlight Kerning and Highlight
/// Tracking commands give the keyboard focus ([`highlight`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Field {
    /// The font family menu: it opens, and its search field takes the focus.
    FontName,
    Kerning,
    Tracking,
}

impl Field {
    /// The id the panel draws the field with ([`show`], [`metrics_grid`]).
    fn salt(self) -> &'static str {
        match self {
            Field::FontName => "ch-font",
            Field::Kerning => "ch-kern",
            Field::Tracking => "ch-track",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Field::FontName => "fontName",
            Field::Kerning => "kerning",
            Field::Tracking => "tracking",
        }
    }
}

/// `type.highlightFontName`, `type.highlightKerning` and `type.highlightTracking`: show the
/// Character panel if it is hidden (a shown one is activated, so its floating window comes
/// forward), and give `field` the keyboard focus when the panel next draws it, a number field with
/// its value selected (typing replaces it, Enter applies it). Without selected type or text the
/// Type tool edits, the panel shows, nothing takes the focus and the command fails. → `{field}`.
pub(crate) fn highlight(app: &mut VectorcraftApp, field: Field) -> Result<Value, String> {
    // `window.panel` would hide a floating panel in front of its window, or an open flyout.
    if crate::panel_docking::showing(&app.ui, "character") {
        app.run("window.panel.activate", json!({"panel": "character"}))?;
    } else {
        app.run("window.panel", json!({"panel": "character"}))?;
    }
    if text_style(app).is_none() {
        app.character_focus = None;
        return Err("select some type first".into());
    }
    app.character_focus = Some(field);
    Ok(json!({ "field": field.name() }))
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
    let target_before = crate::ime::target(&mut app.session, v);
    let before = crate::ime::stamp(&mut app.session, v);
    let r = app.session.tool_key(k, m, v);
    match &r {
        Err(e) => app.status(e.to_string()),
        Ok(_) => {
            if let Some(ctx) = app.ui_ctx.clone().or_else(|| CTX.get().cloned()) {
                crate::ime::note_application_edit(app, &ctx, v, target_before, before);
            }
        }
    }
}

fn text_paste(app: &mut VectorcraftApp, s: Option<String>) {
    let s = s.unwrap_or_else(|| CLIP.lock().unwrap_or_else(|e| e.into_inner()).clone());
    if !s.is_empty() {
        let v = app.view_info();
        let target_before = crate::ime::target(&mut app.session, v);
        let before = crate::ime::stamp(&mut app.session, v);
        let r = app.session.tool_text(&s, v);
        match &r {
            Err(e) => app.status(e.to_string()),
            Ok(_) => {
                if let Some(ctx) = app.ui_ctx.clone().or_else(|| CTX.get().cloned()) {
                    crate::ime::note_application_edit(app, &ctx, v, target_before, before);
                }
            }
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
        // The visible label names the value field beside it, also in a grid cell that draws more
        // than one widget (egui 0.37's strict accessibility check).
        ui.label_inputs_by(l.id, add);
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
            let leading = shared_leading(app);
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
        app.character_focus = None;
        super::empty_state(ui, "type", tl!("No text selected"), tl!("Select a text object to edit its character attributes."));
        return;
    };
    let w = ui.available_width();
    // The field a Highlight command asked for takes the focus as it is drawn. If no field took it
    // (a sizing pass), the next frame tries again.
    if let Some(f) = app.character_focus {
        widgets::arm_field_focus(ui.ctx(), f.salt());
    }
    font_pickers(app, ui, &s, ("ch-font", "ch-style"), (w - 4.0, w - 4.0));
    variable_font(app, ui, &s, w - 4.0);
    ui.add_space(2.0);
    let hidden = pstate::<bool>(ui.ctx(), "ch-hide-options");
    metrics_grid(app, ui, "ch-grid", &s, w, !hidden);
    if app.character_focus.is_some() && !widgets::disarm_field_focus(ui.ctx()) {
        app.character_focus = None;
    }
    if hidden {
        return;
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let small = shared(app, |s| s.small_caps.is_some()) == Some(true);
        let position = shared(app, |s| s.position).map(|p| p.id()).unwrap_or("");
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

/// The Character Direction submenu: Default, Left to Right and Right to Left set the characters the
/// panel acts on ([`char_cmd`]) as the bidi algorithm sets them, or left to right or right to left
/// inside their paragraph. The direction those characters share is checked, and none is checked
/// when they differ. At a caret in text the Type tool edits, with no characters selected, the items
/// are disabled.
fn char_direction_items(app: &mut VectorcraftApp, ui: &mut Ui) {
    use vectorcraft_doc::ParaDirection;
    let direction = shared(app, |s| s.char_direction);
    let enabled = text_editing(app).is_none_or(|(_, a, b)| b > a);
    for (label, d, key) in [
        (tl!("Default"), None, "default"),
        (tl!("Left to Right"), Some(ParaDirection::LeftToRight), "leftToRight"),
        (tl!("Right to Left"), Some(ParaDirection::RightToLeft), "rightToLeft"),
    ] {
        if menu_item(ui, label, enabled, direction == Some(d)) {
            format(app, json!({"charDirection": key}));
        }
    }
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
    let small = shared(app, |s| s.small_caps.is_some()) == Some(true);
    let position = shared(app, |s| s.position).map(|p| p.id()).unwrap_or("");
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
        // Tate-chu-yoko: the selected characters stand side by side across one em of a vertical column.
        let tcy = s.as_ref().is_some_and(|s| s.tate_chu_yoko);
        if menu_item(ui, tl!("Tate-chu-yoko"), has, tcy) {
            format(app, json!({"tateChuYoko": !tcy}));
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
        ui.add_enabled_ui(has, |ui| {
            ui.menu_button(format!("   {}", tl!("Character Direction")), |ui| char_direction_items(app, ui));
        });
        ui.separator();
    }
    // No Break: the selected characters stay on one line.
    let no_break = s.as_ref().is_some_and(|s| s.no_break);
    if menu_item(ui, tl!("No Break"), has, no_break) {
        format(app, json!({"noBreak": !no_break}));
    }
    // Fractional Widths: the whole type object's spacing, on unless turned off. Not a character
    // range, so a selection inside the text sets the object the Type tool is editing.
    let fractional = fractional_widths_on(app);
    if menu_item(ui, tl!("Fractional Widths"), has, fractional) {
        let mut p = json!({});
        if let Some((id, _, _)) = text_editing(app) {
            p["id"] = json!(id.0);
        }
        app.run("type.fractionalWidths", p).ok();
    }
    // System Layout: whole-point spacing for every text object in the document. It does not need
    // a type selection; a document is enough.
    let open = app.session.active().is_some();
    let system = app.session.active().is_some_and(|st| st.doc.setup.system_layout);
    if menu_item(ui, tl!("System Layout"), open, system) {
        app.run("type.systemLayout", json!({})).ok();
    }
    menu_item(ui, tl!("Standard Vertical Roman Alignment"), false, false);
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

/// Whether Fractional Widths is on for the type the panel shows: the object being edited, else
/// the first selected text object. On when there is no type (the item is disabled then).
fn fractional_widths_on(app: &VectorcraftApp) -> bool {
    if let Some((id, _, _)) = text_editing(app)
        && let Some(NodeKind::Text(t)) = app.session.active().and_then(|d| d.doc.node(id)).map(|n| &n.kind)
    {
        return t.fractional_widths;
    }
    first_selected_text(app).is_none_or(|t| t.fractional_widths)
}

/// Reset Panel: the characters the panel acts on ([`char_cmd`]) take tracking 0, Auto leading and
/// kerning, no baseline shift, 100% scales, no rotation, no underline or strikethrough, All Caps
/// and Small Caps off, the normal position (neither superscript nor subscript), Roman Baseline
/// character alignment, Proportional Metrics, Tate-chu-yoko and No Break off, the Default character
/// direction and the Default digits, whether or not Show East Asian Options and Show Indic Options
/// are on.
fn reset_panel(app: &mut VectorcraftApp) {
    let d = CharStyle::default();
    style(app, json!({"tracking": 0, "leading": "auto"}));
    format(
        app,
        json!({
            "kerning": "auto", "baselineShift": 0, "hScale": 100, "vScale": 100, "rotation": 0, "underline": false, "strikethrough": false,
            "allCaps": false, "smallCaps": d.small_caps.is_some(), "position": d.position.id(), "charAlign": d.char_align,
            "proportionalMetrics": d.proportional_metrics, "tateChuYoko": d.tate_chu_yoko, "noBreak": d.no_break, "charDirection": "default",
            "digits": d.digits
        }),
    );
}

#[cfg(test)]
mod tests;
