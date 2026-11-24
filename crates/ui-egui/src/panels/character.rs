//! Character panel: font family / style, size, leading, kerning, tracking, vertical and horizontal
//! scale, baseline shift, character rotation, the anti-aliasing method and the All Caps, Small Caps,
//! Superscript, Subscript, Underline and Strikethrough toggles and the Language menu.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::{Arc, Mutex, Weak};
use std::time::SystemTime;

// PLAN: assumed the framework paths below (craft::text::ui re-exports, craft_ui widgets and runtime, craft::font_manager).
use craft::behavior::EditEvent;
use craft::font_manager::axes::{FontAxis, NamedInstance};
use craft::font_manager::picker::{FilterLayout, FontScript};
use craft::keymap::CommandRef;
use craft::text::ui::DocUnit;
use craft::text::ui::character::{
    AxesLayout, BaselinePosition, Caps, CharacterChange, CharacterControls, CharacterControlsOutput, CharacterRows, CharacterValues, FaceAxes,
    FontHeightReference as HeightReference, FontPickerSource, Kerning, Leading, SIZE_PRESETS, keys,
};
use craft::text::ui::type_options::TextAntiAlias;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Lazy, Props as _, Show as _, ShowContainer as _, Trigger};
use craft::ui::display::Divider;
use craft::ui::layout_components::Row;
use craft::ui::menu::PanelRow;
use craft::ui::pickers::ComboBox;
use craft::ui::popover::Popover;
use craft::ui::runtime::commands::{CommandState, CommandStates};
use craft::ui::runtime::env::Env;
use craft::ui::runtime::focus::{self, FocusTarget};
use craft::ui::text::{NumberField, NumberPick};
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_doc::setup::{LANGUAGES, char_language_name};
use vectorcraft_doc::text::TypeAntiAlias;
use vectorcraft_doc::{CharStyle, NodeId, NodeKind, RubyAlign, RubyPosition, RubySpan, TextObject};
use vectorcraft_engine::DocState;
use vectorcraft_tools::{Mods, ToolKey};

// PLAN: assumed crate::edits::{apply_edits, apply_event, EditPath} (wg.A16, widgets-w13).
use crate::VectorcraftApp;
use crate::edits::{EditPath, apply_edits, apply_event};
use crate::font_menu;
use crate::panels::{pstate, set_pstate};

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
    match &first_selected_text_node(app)?.kind {
        NodeKind::Text(t) => Some(t.as_ref()),
        _ => None,
    }
}

/// The node of [`first_selected_text`]: the first text object among the selected objects and
/// their descendants, in selection order, then paint order within each selected object.
pub(crate) fn first_selected_text_node(app: &VectorcraftApp) -> Option<&vectorcraft_doc::Node> {
    let st = app.session.active()?;
    for node in st.selection.objects.iter().filter_map(|id| st.doc.node(*id)) {
        let mut text = None;
        node.walk(&mut |n| {
            if text.is_none() && matches!(n.kind, NodeKind::Text(_)) {
                text = Some(n);
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
/// caret's style at a caret), else every run of every frame of the selected stories
/// ([`selected_stories`]). `None` (a blank field) where they differ, or with no type.
pub(crate) fn shared<T: PartialEq>(app: &VectorcraftApp, f: impl Fn(&CharStyle) -> T) -> Option<T> {
    shared_styles(app, |_, _, style, _| Some(f(style)))
}

/// The fonts (family and style) of the character styles the panels act on ([`shared`]), each
/// once, in the order the runs come.
pub(crate) fn selected_fonts(app: &VectorcraftApp) -> Vec<(String, String)> {
    let fonts = std::cell::RefCell::new(Vec::<(String, String)>::new());
    shared_styles(app, |_, _, style, _| {
        let mut fonts = fonts.borrow_mut();
        if !fonts.iter().any(|(family, face)| *family == style.font_family && *face == style.font_style) {
            fonts.push((style.font_family.clone(), style.font_style.clone()));
        }
        Some(())
    });
    fonts.into_inner()
}

/// Dimensional controls use document units and tolerate arithmetic roundoff when comparing
/// compensated runs. Keep raw styles available to font/paragraph controls and style definitions,
/// even if a collapsed affine prevents editing dimensions.
fn shared_effective(app: &VectorcraftApp, f: impl Fn(&CharStyle) -> f64) -> Option<f64> {
    shared_styles(app, |t, _, style, _| t.effective_char_style(style).map(|s| Metric(f(&s)))).map(|v| v.0)
}

/// The Leading shown: each character's own, or for Auto leading the share of the em size their
/// font height reference gives them ([`vectorcraft_text::FontDb::em_size`]) that their paragraph's
/// Justification › Auto Leading sets, as the layout uses. An inline graphic keeps its size.
fn shared_leading(app: &VectorcraftApp) -> Option<f64> {
    let db = vectorcraft_text::FontDb::global();
    shared_styles(app, |t, para, style, inline| {
        let s = t.effective_char_style(style)?;
        let em = if inline { s.size } else { db.em_size(&s, t.vertical) };
        Some(Metric(para.effective_leading(&CharStyle { size: em, ..s })))
    })
    .map(|v| v.0)
}

pub(crate) struct Metric(pub(crate) f64);

impl PartialEq for Metric {
    fn eq(&self, other: &Self) -> bool {
        (self.0 - other.0).abs() <= 1e-9 * self.0.abs().max(other.0.abs()).max(1.0)
    }
}

/// `f` of each character style in the selection, with its text object, the paragraph the
/// characters are in (a run that spans paragraphs counts once for each) and whether the run is an
/// inline graphic.
fn shared_styles<T: PartialEq>(
    app: &VectorcraftApp,
    f: impl Fn(&vectorcraft_doc::TextObject, &vectorcraft_doc::ParaStyle, &CharStyle, bool) -> Option<T>,
) -> Option<T> {
    let mut acc = Shared::None;
    each_style(app, |t, para, style, inline| acc.add(f(t, para, style, inline)));
    acc.value().flatten()
}

/// Whether `f` holds for any of the character styles [`shared`] reads.
pub(crate) fn any_style(app: &VectorcraftApp, f: impl Fn(&CharStyle) -> bool) -> bool {
    let mut any = false;
    each_style(app, |_, _, style, _| any = any || f(style));
    any
}

/// Call `f` with each character style the panels act on, as [`shared_styles`] describes: the
/// caret's style at a Type tool caret, else each piece of the runs the Type tool's selection
/// covers, else of every run of every frame of the selected stories.
fn each_style(app: &VectorcraftApp, mut f: impl FnMut(&vectorcraft_doc::TextObject, &vectorcraft_doc::ParaStyle, &CharStyle, bool)) {
    let Some(st) = app.session.active() else {
        return;
    };
    let text = |id: NodeId| match st.doc.node(id).map(|n| &n.kind) {
        Some(NodeKind::Text(t)) => Some(t),
        _ => None,
    };
    if let Some((id, a, b)) = text_editing(app) {
        let Some(t) = text(id) else {
            return;
        };
        if b <= a {
            let para = t.plain_text().get(..a).map_or(0, |s| s.bytes().filter(|&c| c == b'\n').count());
            f(t, t.para_at(para), &vectorcraft_text::edit::insertion_style(&t.runs, a, b), false);
            return;
        }
        match text_story(app) {
            // After Select All in threaded text: all the text of every frame of the story.
            Some(story) => {
                for t in story.into_iter().filter_map(text) {
                    each_paragraph_run(t, None, |para, style, inline| f(t, para, style, inline));
                }
            }
            None => each_paragraph_run(t, Some((a, b)), |para, style, inline| f(t, para, style, inline)),
        }
    } else {
        for t in selected_story_texts(app) {
            each_paragraph_run(t, None, |para, style, inline| f(t, para, style, inline));
        }
    }
}

/// `f` of the paragraph and style of each piece of `t`'s runs that lies in one paragraph (a
/// paragraph's closing line break belongs to it), and whether the run is an inline graphic: the
/// pieces within bytes `a..b`, or with no range every run's, empty ones included. With no range, a
/// final empty paragraph (the text ends in a line break) counts too, with the style of the run
/// before it, as the caret there types.
fn each_paragraph_run<'a>(
    t: &'a vectorcraft_doc::TextObject,
    range: Option<(usize, usize)>,
    mut f: impl FnMut(&'a vectorcraft_doc::ParaStyle, &'a CharStyle, bool),
) {
    let (mut end, mut para) = (0, 0);
    // The style of the last character and whether it is a line break: the paragraph after it has
    // no characters of its own.
    let mut last: Option<(&'a CharStyle, bool)> = None;
    for r in &t.runs {
        if r.text.is_empty() {
            if range.is_none_or(|(a, b)| a < end && end < b) {
                f(t.para_at(para), &r.style, r.inline.is_some());
            }
            continue;
        }
        for piece in r.text.split_inclusive('\n') {
            let start = end;
            end += piece.len();
            if range.is_none_or(|(a, b)| start < b && end > a) {
                f(t.para_at(para), &r.style, r.inline.is_some());
            }
            para += usize::from(piece.ends_with('\n'));
            last = Some((&r.style, piece.ends_with('\n')));
        }
    }
    if range.is_none()
        && let Some((style, true)) = last
    {
        f(t.para_at(para), style, false);
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
pub(crate) fn format(app: &mut VectorcraftApp, p: Value) {
    char_cmd(app, "text.setFormat", p);
}

/// [`format`] that returns the command's result: `text.setRangeStyle` on the range the Type tool
/// has selected (the whole text when nothing is selected) while it edits text, else
/// `text.setFormat` on the selected stories ([`selected_stories`]).
pub(crate) fn run_format(app: &mut VectorcraftApp, mut p: Value) -> Result<Value, String> {
    let Some((id, a, b)) = text_editing_kept(app)? else {
        target_selected_stories(app, &mut p);
        return app.run("text.setFormat", p);
    };
    if let Some(first) = text_story(app).and_then(|story| story.first().copied()) {
        return story_style(app, first, p);
    }
    p["id"] = json!(id.0);
    if b > a {
        p["start"] = json!(a);
        p["end"] = json!(b);
    }
    app.run("text.setRangeStyle", p)
}

/// The Tate-chu-yoko Settings Up/Down and Left/Right the characters the panels act on share, in
/// document points; `None` where they differ, or with no type.
pub(crate) fn shared_tcy_offsets(app: &VectorcraftApp) -> (Option<f64>, Option<f64>) {
    (shared_effective(app, |s| s.tcy_up_down), shared_effective(app, |s| s.tcy_left_right))
}

/// The Warichu Settings the characters the panels act on share: Lines, Line Gap (document
/// points), Scale (percent) and Alignment; `None` for each that differs, or with no type.
pub(crate) struct SharedWarichu {
    pub lines: Option<u8>,
    pub gap: Option<f64>,
    pub scale: Option<f64>,
    pub align: Option<vectorcraft_doc::WarichuAlign>,
}

pub(crate) fn shared_warichu(app: &VectorcraftApp) -> SharedWarichu {
    SharedWarichu {
        lines: shared(app, |s| s.warichu_lines),
        gap: shared_effective(app, |s| s.warichu_gap),
        scale: shared(app, |s| s.warichu_scale),
        align: shared(app, |s| s.warichu_align),
    }
}

/// Apply character attributes: to the selected range while the Type tool edits text (the whole
/// object when nothing is selected), else to every frame of the selected stories
/// ([`selected_stories`]).
fn char_cmd(app: &mut VectorcraftApp, cmd: &str, mut p: Value) {
    if text_editing(app).is_none() {
        target_selected_stories(app, &mut p);
        app.run(cmd, p).ok();
        return;
    }
    range_style(app, p);
}

/// Set the Kerning field's `value` (a number, "auto", or "optical"). At a caret in text the Type tool edits
/// it is the pair kerning after the character before the caret, as Alt+←/→ steps it
/// (`type.step`), or at the start of a line the space before the character after it, where Auto
/// and Optical set no space (#5111); nowhere to set at an empty line's start. Else it is a
/// character attribute like the others: the selected range, or all the text.
fn set_kerning(app: &mut VectorcraftApp, value: Value) {
    let Ok(editing) = text_editing_kept(app) else { return };
    let Some((id, a, b)) = editing else {
        format(app, json!({"kerning": value}));
        return;
    };
    if b > a {
        range_style(app, json!({"kerning": value}));
        return;
    }
    let Some(NodeKind::Text(t)) = app.session.active().and_then(|st| st.doc.node(id)).map(|n| &n.kind) else { return };
    let Some((target, before)) = vectorcraft_text::edit::caret_kerning_target(&t.plain_text(), a) else { return };
    let params = if before {
        json!({"id": id.0, "start": target.start, "end": target.end, "kerningBefore": value.as_f64().unwrap_or(0.0)})
    } else {
        json!({"id": id.0, "start": target.start, "end": target.end, "kerning": value})
    };
    app.run("text.setRangeStyle", params).ok();
}

/// The kerning at the caret, in 1/1000 em, using the layout the canvas draws: the pair before it,
/// or at the start of a line the space before the character after it.
fn caret_kerning(app: &mut VectorcraftApp) -> Option<(f64, bool)> {
    let (id, a, b) = text_editing(app)?;
    if a != b {
        return None;
    }
    let NodeKind::Text(t) = &app.session.active()?.doc.node(id)?.kind else { return None };
    let text = t.plain_text();
    let (target, before) = vectorcraft_text::edit::caret_kerning_target(&text, a)?;
    let start = target.start;
    let style = vectorcraft_text::edit::style_at(&t.runs, start + 1);
    if before {
        return Some((style.kerning_before, false));
    }
    if let Some(v) = style.kerning {
        return Some((v, false));
    }
    let state = app.session.active()?;
    let key = (state.uid, state.revision, id, a);
    // PLAN: assumed app.canvas.caret_kerning: Option<((u64, u64, NodeId, usize), f64)> (ty.S18).
    if let Some((cached_key, value)) = app.canvas.caret_kerning
        && cached_key == key
    {
        return Some((value, true));
    }
    let original = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
    let value = vectorcraft_engine::cmd::effective_kerning(vectorcraft_text::FontDb::global(), t, &original, start, a);
    let value = value.round();
    app.canvas.caret_kerning = Some((key, value));
    Some((value, true))
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

/// The Character panel fields that the Highlight Font Name, Highlight Font Size (Type › Size ›
/// Other…), Highlight Kerning and Highlight Tracking commands give the keyboard focus
/// ([`highlight`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Field {
    /// The font family menu: it opens, and its search field takes the focus.
    FontName,
    FontSize,
    Kerning,
    Tracking,
}

impl Field {
    /// The document key the panel's field carries, which [`show`] focuses.
    fn key(self) -> craft::behavior::DocKey {
        match self {
            Field::FontName => keys::FAMILY,
            Field::FontSize => keys::SIZE,
            Field::Kerning => keys::KERNING,
            Field::Tracking => keys::TRACKING,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Field::FontName => "fontName",
            Field::FontSize => "fontSize",
            Field::Kerning => "kerning",
            Field::Tracking => "tracking",
        }
    }
}

/// `type.highlightFontName`, `type.highlightFontSize`, `type.highlightKerning` and
/// `type.highlightTracking`: show the Character panel if it is hidden (a shown one is activated,
/// so its floating window comes forward), and give `field` the keyboard focus when the panel next
/// draws it, a number field with its value selected (typing replaces it, Enter applies it).
/// Without selected type or text the Type tool edits, the panel shows, nothing takes the focus and
/// the command fails. → `{field}`.
pub(crate) fn highlight(app: &mut VectorcraftApp, field: Field) -> Result<Value, String> {
    // `window.panel` would hide a floating panel in front of its window, or an open flyout.
    if crate::panel_docking::visible(app, "character") {
        app.run("window.panel.activate", json!({"panel": "character"}))?;
    } else {
        crate::panel_docking::reveal_panel(app, "character")?;
    }
    if text_style(app).is_none() {
        app.character_focus = None;
        return Err("select some type first".into());
    }
    app.character_focus = Some(field);
    Ok(json!({ "field": field.name() }))
}

/// `type.underline` (Cmd+Shift+U): what the Underline button does, on the text the panels act on.
/// Returns whether the text is now underlined: off only when every character was underlined.
pub(crate) fn toggle_underline(app: &mut VectorcraftApp) -> Result<Value, String> {
    // The typing Real-time Drawing off left unapplied is applied before the text is read.
    text_editing_kept(app)?;
    text_style(app).ok_or("select some type first")?;
    let on = shared(app, |s| s.underline) != Some(true);
    run_format(app, json!({"underline": on}))?;
    Ok(json!(on))
}

/// `type.superscript` (Cmd+Shift+=) and `type.subscript` (Cmd+Alt+Shift+=): what the Superscript
/// or Subscript button does, on the text the panels act on. Returns the position the text now
/// has: `"normal"` only when every character had `position`, else `position`.
pub(crate) fn toggle_position(app: &mut VectorcraftApp, position: &str) -> Result<Value, String> {
    // The typing Real-time Drawing off left unapplied is applied before the text is read.
    text_editing_kept(app)?;
    text_style(app).ok_or("select some type first")?;
    let change = position_toggle(shared_position(app), position);
    let to = change["position"].clone();
    run_format(app, change)?;
    Ok(to)
}

/// `text.setRangeStyle` with `p` on the range the Type tool has selected (the whole text when
/// nothing is selected). Does nothing unless the Type tool edits text.
pub(crate) fn range_style(app: &mut VectorcraftApp, p: Value) {
    let Ok(Some((id, a, b))) = text_editing_kept(app) else { return };
    if let Some(first) = text_story(app).and_then(|story| story.first().copied()) {
        story_style(app, first, p).ok();
        return;
    }
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

/// [`text_editing`] after [`end_typing`], for an edit of the text: the selection is read in the
/// text with the typing applied, which Real-time Drawing off leaves unapplied until then. When the
/// typing can't be kept, the error (also shown in the status bar), so the edit stops instead of
/// acting on a selection in the text without the typing.
pub(crate) fn text_editing_kept(app: &mut VectorcraftApp) -> Result<Option<(NodeId, usize, usize)>, String> {
    try_end_typing(app)?;
    Ok(text_editing(app))
}

/// The frames of the threaded story whose whole text the Type tool has selected after Select All,
/// in the order of the edited frame's thread as the document holds it now; `None` when its
/// selection is in one type object.
pub(crate) fn text_story(app: &VectorcraftApp) -> Option<Vec<NodeId>> {
    let o = app.session.tool_options();
    let st = app.session.active()?;
    let id = NodeId(o.get("editing")?.as_u64()?);
    if o.get("story")?.as_array()?.is_empty() {
        return None;
    }
    let ids = st.doc.story_of(id);
    (ids.len() >= 2).then_some(ids)
}

/// The type objects the panels act on with the Selection tool: the selected type objects and those
/// in selected groups, each with every frame of its threaded story (locked and hidden frames
/// included), each once, the frames of a story in thread order.
pub(crate) fn selected_stories(app: &VectorcraftApp) -> Vec<NodeId> {
    app.session.active().map_or_else(Vec::new, |st| story_frames(st).to_vec())
}

/// The last list [`selected_stories`] worked out, with the document version (its address, which
/// the held `Weak` keeps from being reused, and its revision) and the selected objects it was
/// worked out for. The Character and Paragraph panels read the list once for each field they show.
struct StoryFrames {
    doc: Weak<vectorcraft_doc::Document>,
    revision: u64,
    selection: Vec<NodeId>,
    frames: Rc<[NodeId]>,
}

thread_local! {
    static STORY_FRAMES: RefCell<Option<StoryFrames>> = const { RefCell::new(None) };
}

#[cfg(test)]
thread_local! {
    /// How many times [`build_selected_stories`] ran on this thread.
    static STORY_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Whether [`story_frames`] builds the list on every read instead of keeping it.
    static STORIES_UNKEPT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// [`selected_stories`] of `st`: the kept list while `st`'s document, revision and selected
/// objects are the ones it was worked out for, else a new one, kept in its place.
fn story_frames(st: &DocState) -> Rc<[NodeId]> {
    #[cfg(test)]
    if STORIES_UNKEPT.get() {
        return build_selected_stories(st).into();
    }
    STORY_FRAMES.with_borrow_mut(|kept| {
        if let Some(k) = kept.as_ref()
            && k.revision == st.revision
            && k.doc.strong_count() > 0
            && std::ptr::eq(k.doc.as_ptr(), Arc::as_ptr(&st.doc))
            && k.selection == st.selection.objects
        {
            return k.frames.clone();
        }
        let frames: Rc<[NodeId]> = build_selected_stories(st).into();
        *kept = Some(StoryFrames {
            doc: Arc::downgrade(&st.doc),
            revision: st.revision,
            selection: st.selection.objects.clone(),
            frames: frames.clone(),
        });
        frames
    })
}

/// The list [`selected_stories`] describes, worked out from `st`'s selection.
fn build_selected_stories(st: &DocState) -> Vec<NodeId> {
    #[cfg(test)]
    STORY_BUILDS.set(STORY_BUILDS.get() + 1);
    let mut frames = Vec::new();
    let mut taken = HashSet::new();
    for selected in st.selection.objects.iter().filter_map(|id| st.doc.node(*id)) {
        selected.walk(&mut |child| {
            if matches!(child.kind, NodeKind::Text(_)) && !taken.contains(&child.id) {
                for id in st.doc.story_of(child.id) {
                    if taken.insert(id) {
                        frames.push(id);
                    }
                }
            }
        });
    }
    frames
}

/// The type objects of [`selected_stories`].
pub(crate) fn selected_story_texts(app: &VectorcraftApp) -> Vec<&vectorcraft_doc::TextObject> {
    let Some(st) = app.session.active() else { return Vec::new() };
    story_frames(st)
        .iter()
        .filter_map(|&id| match st.doc.node(id).map(|n| &n.kind) {
            Some(NodeKind::Text(t)) => Some(t.as_ref()),
            _ => None,
        })
        .collect()
}

/// Give the command parameters `p` the `ids` of [`selected_stories`], so a command run with the
/// Selection tool sets every frame of each selected story. `p` stays as it is when it names its
/// targets, with no type selected, and while an object of a symbol instance's art is selected,
/// which the commands refuse.
pub(crate) fn target_selected_stories(app: &VectorcraftApp, p: &mut Value) {
    if p.get("ids").is_some() || p.get("id").is_some() || app.session.active().is_none_or(|st| st.selection.symbol_part.is_some()) {
        return;
    }
    let ids = selected_stories(app);
    if let Some(params) = p.as_object_mut()
        && !ids.is_empty()
    {
        params.insert("ids".into(), json!(ids.iter().map(|id| id.0).collect::<Vec<_>>()));
    }
}

/// `text.setRangeStyle` with `p` on all the text of every frame of the story that `first` is a
/// frame of, as one undo step.
fn story_style(app: &mut VectorcraftApp, first: NodeId, mut p: Value) -> Result<Value, String> {
    end_typing(app);
    p["id"] = json!(first.0);
    p["story"] = json!(true);
    app.run("text.setRangeStyle", p)
}

/// Close the Type tool's typing session (one undo step) before another command edits the text. The
/// typing Real-time Drawing off left unapplied is applied first; when it can't be kept, the error
/// shows and the session goes on.
pub(crate) fn end_typing(app: &mut VectorcraftApp) {
    try_end_typing(app).ok();
}

/// [`end_typing`] that also returns the error when the typing can't be kept.
fn try_end_typing(app: &mut VectorcraftApp) -> Result<(), String> {
    if app.session.tool_options().get("typing").and_then(Value::as_bool) == Some(true) {
        if let Err(e) = app.keep_typing() {
            app.status(e.clone());
            return Err(e);
        }
        app.session.set_tool_option("commitTyping", &json!(true));
        app.session.commit_interaction().ok();
        app.sync_views();
    }
    Ok(())
}

/// Plain text of the last text copied inside the Type tool (paste from the native menu, which
/// can't read the system clipboard).
static CLIP: Mutex<String> = Mutex::new(String::new());
static CLIP_RUNS: Mutex<Option<Value>> = Mutex::new(None);

/// Copy the Type tool's selected text (with `cut`, for a Cut, whose caller then deletes it).
pub(crate) fn text_copy(app: &mut VectorcraftApp, cut: bool) -> bool {
    let Some((id, a, b)) = text_editing(app) else { return false };
    let story = text_story(app);
    // An empty selection has nothing to copy, unless the story's text is in the other frames.
    if a == b && (story.is_none() || app.session.active().is_none_or(|st| st.doc.story_text(id).is_empty())) {
        return false;
    }
    // After Select All in threaded text: the whole story.
    let range = if story.is_some() { json!({"id": id.0, "story": true}) } else { json!({"id": id.0, "start": a, "end": b}) };
    let Ok(r) = app.session.execute("text.getRange", &range) else { return false };
    let runs = r["runs"].clone();
    app.session.set_tool_option("copy", &runs);
    *CLIP_RUNS.lock().unwrap_or_else(|e| e.into_inner()) = Some(runs.clone());
    let text = r["text"].as_str().unwrap_or("").to_string();
    *CLIP.lock().unwrap_or_else(|e| e.into_inner()) = text.clone();
    if let Ok(parsed_runs) = serde_json::from_value::<Vec<vectorcraft_doc::TextRun>>(runs)
        && !parsed_runs.is_empty()
    {
        let mut t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::ZERO, "", vectorcraft_doc::CharStyle::default());
        t.runs = parsed_runs;
        let node = vectorcraft_doc::Node::new(vectorcraft_doc::NodeId(1), vectorcraft_doc::NodeKind::Text(Box::new(t)));
        let Ok(st) = app.session.doc() else { return false };
        let clip = vectorcraft_engine::cmd::clipboard::Clipboard::from_node(st, node);
        app.session.clipboard = clip;
        // The desktop clipboard must own the same styled art as the internal clipboard. An egui
        // plain-text copy would replace its formats, so let the publisher provide both text and
        // rich art. Paste on the canvas can then keep the runs after Type editing ends.
        if app.clipboard.readable_any_time() {
            app.publish_clipboard(cut);
            return true;
        }
    }
    if let Some(ctx) = app.ui_ctx.clone() {
        ctx.copy_text(text);
    }
    true
}

pub(crate) fn text_key(app: &mut VectorcraftApp, k: ToolKey, m: Mods) {
    let v = app.view_info();
    if let Err(e) = app.session.tool_key(k, m, v) {
        app.status(e.to_string());
    }
}

pub(crate) fn text_paste(app: &mut VectorcraftApp, s: Option<String>, plain: bool) {
    let s = match s {
        Some(text) => text,
        None if app.clipboard.readable_any_time() => app.written_clipboard_text().unwrap_or_default(),
        None => CLIP.lock().unwrap_or_else(|e| e.into_inner()).clone(),
    };
    if !s.is_empty() {
        let internal = app.type_paste_has_our_clipboard();
        let mut styled = false;
        if !plain
            && internal
            && let Some(runs) = CLIP_RUNS.lock().unwrap_or_else(|e| e.into_inner()).clone()
        {
            let clip_text: String =
                serde_json::from_value::<Vec<vectorcraft_doc::TextRun>>(runs.clone()).unwrap_or_default().into_iter().map(|r| r.text).collect();
            if clip_text == s {
                app.session.set_tool_option("copy", &runs);
                styled = true;
            }
        }
        if !styled {
            app.session.set_tool_option("copy", &json!([]));
        }
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
            text_copy(app, false);
        }
        "edit.cut" => {
            if text_copy(app, true) {
                text_key(app, ToolKey::Delete, Mods::default());
            }
        }
        "edit.paste" => text_paste(app, None, false),
        "edit.pasteWithoutFormatting" => text_paste(app, None, true),
        "edit.clear" => text_key(app, ToolKey::Delete, Mods::default()),
        _ => return None,
    }
    app.sync_views();
    Some(Ok(Value::Null))
}

/// The anti-aliasing method of the type objects the panel acts on: the one the Type tool edits,
/// else every frame of the selected stories ([`selected_stories`]). `None` where they differ, or
/// with no type.
fn shared_anti_alias(app: &VectorcraftApp) -> Option<TypeAntiAlias> {
    let st = app.session.active()?;
    let mut acc = Shared::None;
    match text_editing(app) {
        Some((id, _, _)) => {
            if let Some(NodeKind::Text(t)) = st.doc.node(id).map(|n| &n.kind) {
                acc.add(t.anti_alias);
            }
        }
        None => {
            for t in selected_story_texts(app) {
                acc.add(t.anti_alias);
            }
        }
    }
    acc.value()
}

/// Set the anti-aliasing method `m` of the frame the Type tool edits, whatever part of its text
/// is selected, else every frame of the selected stories (`text.setFormat {antiAlias}`).
fn set_anti_alias(app: &mut VectorcraftApp, m: TypeAntiAlias) {
    let mut p = json!({"antiAlias": m.id()});
    if let Some((id, _, _)) = text_editing(app) {
        end_typing(app);
        p["ids"] = json!([id.0]);
    } else {
        target_selected_stories(app, &mut p);
    }
    app.run("text.setFormat", p).ok();
}

/// Set Variable Font settings `axes` on the text the panel acts on, inside the caller's undo scope.
pub(crate) fn axes_cmd(app: &mut VectorcraftApp, axes: Value) {
    match text_editing_kept(app) {
        // The typing can't be kept (the error shows): nothing is set.
        Err(_) => {}
        Ok(Some((id, a, b))) => {
            let mut q = json!({"id": id.0, "fontAxes": axes});
            // After Select All in threaded text: every frame of the story.
            if let Some(first) = text_story(app).and_then(|story| story.first().copied()) {
                q["id"] = json!(first.0);
                q["story"] = json!(true);
            } else if b > a {
                q["start"] = json!(a);
                q["end"] = json!(b);
            }
            app.run("text.setRangeStyle", q).ok();
        }
        Ok(None) => {
            let mut p = json!({ "fontAxes": axes });
            target_selected_stories(app, &mut p);
            app.run("text.setFormat", p).ok();
        }
    }
}

/// The anti-aliasing methods the panel offers, in the order of [`TypeAntiAlias::ALL`].
const ANTI_ALIAS_METHODS: [TextAntiAlias; 4] = [TextAntiAlias::None, TextAntiAlias::Sharp, TextAntiAlias::Crisp, TextAntiAlias::Strong];

/// The baseline positions with their `text.setFormat` ids.
const POSITIONS: [(BaselinePosition, &str); 3] =
    [(BaselinePosition::Normal, "normal"), (BaselinePosition::Superscript, "superscript"), (BaselinePosition::Subscript, "subscript")];

/// The font height references: the document's, the controls' and the `text.setFormat` id.
const HEIGHTS: [(vectorcraft_doc::FontHeightReference, HeightReference, &str); 4] = [
    (vectorcraft_doc::FontHeightReference::EmBox, HeightReference::EmBox, "emBox"),
    (vectorcraft_doc::FontHeightReference::CapHeight, HeightReference::CapHeight, "capHeight"),
    (vectorcraft_doc::FontHeightReference::XHeight, HeightReference::XHeight, "xHeight"),
    (vectorcraft_doc::FontHeightReference::IcfBox, HeightReference::IcfBox, "icfBox"),
];

/// Whether a style has one of the two-state attributes on.
type IsOn = fn(&CharStyle) -> bool;

/// The two-state style attributes the panel menu toggles, by `text.setFormat` key. A mixed selection
/// reads as off, so choosing the row turns it on.
const TOGGLES: [(&str, IsOn); 10] = [
    ("allCaps", |s| s.all_caps),
    ("smallCaps", |s| s.small_caps.is_some()),
    ("underline", |s| s.underline),
    ("strikethrough", |s| s.strikethrough),
    ("proportionalMetrics", |s| s.proportional_metrics),
    ("tateChuYoko", |s| s.tate_chu_yoko),
    ("warichu", |s| s.warichu),
    ("standardVerticalRoman", |s| s.standard_vertical_roman),
    ("noBreak", |s| s.no_break),
    ("kashidas", |s| s.kashidas),
];

/// Whether every character the panel acts on has the attribute `key` of [`TOGGLES`].
fn toggled(app: &VectorcraftApp, key: &str) -> bool {
    TOGGLES.iter().find(|(k, _)| *k == key).is_some_and(|(_, on)| shared(app, on) == Some(true))
}

/// A control value as the command's JSON number: the controls hold `f32`, and a plain widening would
/// write 0.1 as 0.10000000149.
fn number(v: f32) -> f64 {
    (f64::from(v) * 1e4).round() / 1e4
}

/// The unit the type lengths show in.
pub(crate) fn type_unit(app: &VectorcraftApp) -> DocUnit {
    doc_unit(app.session.type_unit())
}

/// `unit` as the controls' length unit: pixels are points in the document.
pub(crate) fn doc_unit(unit: vectorcraft_doc::Unit) -> DocUnit {
    // PLAN: assumed canvas::rulers::unit_kind(Unit) -> UnitKind (canvas-w15).
    DocUnit::new(crate::canvas::rulers::unit_kind(unit), 72.0)
}

/// How many of `unit` a point is.
fn unit_scale(unit: DocUnit) -> f64 {
    unit.kind.per_inch(72.0).unwrap_or(unit.pixels_per_inch) / 72.0
}

/// The axes and named instances of the current face when it has a visible axis.
pub(crate) fn face_axes(app: &VectorcraftApp) -> Option<(Vec<FontAxis>, Vec<NamedInstance>)> {
    let (s, _) = text_style(app)?;
    let face = vectorcraft_text::FontDb::global().face_version(&s.font_family, &s.font_style, s.font_version.as_deref())?;
    let axes = craft::font_manager::axes::read_axes(face.file_data(), face.face_index());
    if axes.iter().all(|a| a.hidden) {
        return None;
    }
    Some((axes, craft::font_manager::axes::read_named_instances(face.file_data(), face.face_index())))
}

/// A variation axis's tag and value.
type AxisSetting = ([u8; 4], f32);

/// The axis settings the selection shares, and the axes whose settings differ. The first is `None`
/// while the font has no visible axis.
fn axis_values(app: &VectorcraftApp) -> (Option<Vec<AxisSetting>>, Vec<[u8; 4]>) {
    let Some((axes, _)) = face_axes(app) else { return (None, Vec::new()) };
    let db = vectorcraft_text::FontDb::global();
    let (mut held, mut mixed) = (Vec::new(), Vec::new());
    for axis in axes.iter().filter(|a| !a.hidden) {
        let tag = String::from_utf8_lossy(&axis.tag).into_owned();
        // Each character style's setting: its own, else its face's (a named instance's, or the default).
        let setting = |s: &CharStyle| {
            s.font_axes.get(&tag).copied().or_else(|| {
                let face = db.face_version(&s.font_family, &s.font_style, s.font_version.as_deref())?;
                face.axis_value(axis.tag).map(f64::from)
            })
        };
        match shared(app, setting) {
            Some(Some(v)) => held.push((axis.tag, v as f32)),
            Some(None) => {}
            None => mixed.push(axis.tag),
        }
    }
    (Some(held), mixed)
}

/// The character attributes the controls show, `None` where the selection differs.
pub(crate) fn values(app: &mut VectorcraftApp) -> CharacterValues {
    let single = |v: Option<f64>| v.map(|v| v as f32);
    let mode = shared(app, |s| (s.optical_kerning, s.kerning));
    let at_caret = if text_editing(app).is_some_and(|(_, a, b)| a == b) { caret_kerning(app) } else { None };
    let optical = mode.is_some_and(|(optical, _)| optical);
    let (kerning, auto_kerning) = match at_caret {
        Some((v, true)) => (Some(if optical { Kerning::Optical } else { Kerning::Metrics }), Some(v as f32)),
        Some((v, false)) => (Some(Kerning::Manual(v as f32)), None),
        None => (
            mode.map(|(optical, manual)| match manual {
                Some(v) => Kerning::Manual(v as f32),
                None if optical => Kerning::Optical,
                None => Kerning::Metrics,
            }),
            None,
        ),
    };
    let leading =
        if shared(app, |s| s.leading.is_none()) == Some(true) { Some(Leading::Auto) } else { shared_leading(app).map(|v| Leading::Fixed(v as f32)) };
    // The two flags are one choice here: both on, or a selection that differs, is mixed.
    let caps = match (shared(app, |s| s.all_caps), shared(app, |s| s.small_caps.is_some())) {
        (Some(false), Some(false)) => Some(Caps::Normal),
        (Some(true), Some(false)) => Some(Caps::AllCaps),
        (Some(false), Some(true)) => Some(Caps::SmallCaps),
        _ => None,
    };
    let position = shared_position(app);
    let (axes, mixed_axes) = axis_values(app);
    let language =
        shared(app, |s| s.language.clone()).map(|l| char_language_name(l.as_deref()).map_or_else(|| l.unwrap_or_default(), str::to_string));
    let mut values = CharacterValues::default();
    // The font shows as the type takes it: the insertion style while editing, else the first selected text's first run.
    let font = text_style(app).map(|(s, _)| (s.font_family, s.font_style));
    values.family = font.as_ref().map(|(family, _)| family.clone());
    values.style = font.map(|(_, style)| style);
    values.size = single(shared_effective(app, |s| s.size));
    values.leading = leading;
    values.kerning = kerning;
    values.tracking = single(shared(app, |s| s.tracking));
    values.vertical_scale = single(shared(app, |s| s.v_scale));
    values.horizontal_scale = single(shared_effective(app, |s| s.h_scale));
    values.baseline_shift = single(shared_effective(app, |s| s.baseline_shift));
    values.caps = caps;
    values.position = POSITIONS.iter().find(|(_, id)| *id == position).map(|(p, _)| *p);
    values.underline = shared(app, |s| s.underline);
    values.strikethrough = shared(app, |s| s.strikethrough);
    values.language = language;
    values.axes = axes;
    values.mixed_axes = mixed_axes;
    values.rotation = single(shared(app, |s| s.rotation));
    values.aki_left = single(shared(app, |s| s.aki_left));
    values.aki_right = single(shared(app, |s| s.aki_right));
    values.tsume = single(shared(app, |s| s.tsume));
    values.diacritic_x = single(shared(app, |s| s.diacritic_horizontal));
    values.diacritic_y = single(shared(app, |s| s.diacritic_vertical));
    values.auto_kerning = auto_kerning;
    values.anti_alias =
        shared_anti_alias(app).and_then(|m| TypeAntiAlias::ALL.iter().position(|d| *d == m)).and_then(|i| ANTI_ALIAS_METHODS.get(i).copied());
    values.font_height = shared(app, |s| s.font_height_reference).and_then(|r| HEIGHTS.iter().find(|(d, ..)| *d == r).map(|(_, h, _)| *h));
    values
}

/// The row groups the panel shows. Hide Options leaves out everything below the metrics; Show East
/// Asian and Indic Options add their rows.
fn rows(app: &VectorcraftApp) -> CharacterRows {
    let shown = !app.ui.character_hide_options;
    let prefs = &app.session.prefs;
    CharacterRows::default()
        .with(CharacterRows::COLOR, false)
        .with(CharacterRows::FAUX, false)
        .with(CharacterRows::OPENTYPE, false)
        .with(CharacterRows::SCALES, shown)
        .with(CharacterRows::BASELINE, shown)
        .with(CharacterRows::ROTATION, shown)
        .with(CharacterRows::ANTI_ALIAS, shown)
        .with(CharacterRows::SUPERSCRIPT, shown)
        .with(CharacterRows::STYLE_TOGGLES, shown)
        .with(CharacterRows::FONT_HEIGHT, app.ui.character_font_height)
        .with(CharacterRows::EAST_ASIAN, shown && prefs.show_east_asian_options)
        .with(CharacterRows::MIDDLE_EASTERN, shown && prefs.show_indic_options)
}

/// Runs one change of the controls on the text the panel acts on.
fn apply(app: &mut VectorcraftApp, change: &CharacterChange) {
    use CharacterChange as C;
    let max_diacritic = vectorcraft_doc::text::MAX_DIACRITIC_POSITION;
    match change {
        C::Style(s) => style(app, json!({"style": s})),
        C::Size(v) => style(app, json!({"size": number(*v)})),
        C::Leading(Leading::Auto) => style(app, json!({"leading": "auto"})),
        C::Leading(Leading::Fixed(v)) => style(app, json!({"leading": number(*v)})),
        C::Tracking(v) => style(app, json!({"tracking": number(*v)})),
        C::Kerning(Kerning::Metrics) => set_kerning(app, json!("auto")),
        C::Kerning(Kerning::Optical) => set_kerning(app, json!("optical")),
        C::Kerning(Kerning::Manual(v)) => set_kerning(app, json!(number(*v))),
        C::VerticalScale(v) => format(app, json!({"vScale": number(*v)})),
        C::HorizontalScale(v) => format(app, json!({"hScale": number(*v)})),
        C::BaselineShift(v) => format(app, json!({"baselineShift": number(*v)})),
        C::Rotation(v) => format(app, json!({"rotation": number(*v)})),
        // The fields stop at the largest value the commands take.
        C::AkiLeft(v) => format(app, json!({"akiLeft": number(*v).min(vectorcraft_doc::text::MAX_AKI)})),
        C::AkiRight(v) => format(app, json!({"akiRight": number(*v).min(vectorcraft_doc::text::MAX_AKI)})),
        C::Tsume(v) => format(app, json!({"tsume": number(*v).min(vectorcraft_doc::text::MAX_TSUME)})),
        C::DiacriticX(v) => format(app, json!({"diacriticHorizontal": number(*v).min(max_diacritic)})),
        C::DiacriticY(v) => format(app, json!({"diacriticVertical": number(*v).min(max_diacritic)})),
        C::Caps(c) => format(app, json!({"allCaps": *c == Caps::AllCaps, "smallCaps": *c == Caps::SmallCaps})),
        C::Position(p) => {
            if let Some((_, id)) = POSITIONS.iter().find(|(d, _)| d == p) {
                format(app, json!({"position": id}));
            }
        }
        C::Underline(on) => format(app, json!({"underline": on})),
        C::Strikethrough(on) => format(app, json!({"strikethrough": on})),
        C::Language(name) => format(app, json!({"language": name})),
        C::AntiAlias(method) => {
            if let Some(m) = ANTI_ALIAS_METHODS.iter().position(|f| f == method).and_then(|i| TypeAntiAlias::ALL.get(i)) {
                set_anti_alias(app, *m);
            }
        }
        C::FontHeight(h) => {
            if let Some((_, _, id)) = HEIGHTS.iter().find(|(_, f, _)| f == h) {
                format(app, json!({"fontHeightReference": id}));
            }
        }
        C::Axes(axes) => {
            // The sliders send every axis the selection shares plus the changed one: only the axes
            // that differ from the document are set, so a mixed axis stays as it is.
            let held = axis_values(app).0.unwrap_or_default();
            let changed: serde_json::Map<String, Value> = axes
                .iter()
                .filter(|(tag, v)| held.iter().find(|(t, _)| t == tag).is_none_or(|(_, h)| h != v))
                .map(|(tag, v)| (String::from_utf8_lossy(tag).into_owned(), json!((f64::from(*v) * 10.0).round() / 10.0)))
                .collect();
            if !changed.is_empty() {
                axes_cmd(app, Value::Object(changed));
            }
        }
        // The family goes through its own event (`apply_character_event`); the faux styles and OpenType rows are off.
        _ => {}
    }
}

/// Runs one edit event of the controls, each gesture as one undo step: a family pick goes to the
/// font menu's own path.
fn apply_character_event(app: &mut VectorcraftApp, event: EditEvent<CharacterChange>) {
    if matches!(event.value(), Some(CharacterChange::Family(_))) {
        let family = event.filter_map(|change| match change {
            CharacterChange::Family(f) => Some(f),
            _ => None,
        });
        if let Some(family) = family {
            font_menu::apply_family(app, family, None);
        }
        return;
    }
    apply_event(app, &event, &EditPath::Scope, &mut |app, change, _| apply(app, change));
}

/// The controls over the selected type, with the row groups `rows`: the Character panel shows
/// [`rows`], Properties a shorter set. Nothing shows without type.
pub(crate) fn controls(app: &mut VectorcraftApp, ui: &mut Ui, rows: CharacterRows) {
    let Some((current, _)) = text_style(app) else { return };
    let values = values(app);
    let styles = vectorcraft_text::FontDb::global().styles(&current.font_family);
    let face = face_axes(app);
    let languages: Option<Vec<(String, String)>> =
        rows.contains(CharacterRows::STYLE_TOGGLES).then(|| LANGUAGES.iter().map(|(name, _, _)| (name.to_string(), name.to_string())).collect());
    let catalog = vectorcraft_text::FontDb::global().catalog();
    let before = font_menu::picker_settings(app);
    let mut settings = before.clone();
    let document = font_menu::document_family_set(app);
    let recent = font_menu::recently_added_set(app, SystemTime::now());
    let sets = font_menu::family_sets(Some(&document[..]), &recent[..]);
    let sample = font_menu::sample_text(app);
    let unit = type_unit(app);
    let mut source = FontPickerSource::new(catalog)
        .settings(Bind::bound(&mut settings))
        .filters(Bind::bound(&mut app.ui.font_filters))
        .sets(&sets)
        .scripts(&FontScript::ALL)
        .filter_layout(FilterLayout::Chips);
    if let Some(text) = &sample {
        source = source.preview_text(text);
    }
    let mut controls = CharacterControls::new(tl!("Character"), &values, &[], &styles)
        .unit(unit)
        .rows(rows)
        .font_picker(source)
        .axes_layout(AxesLayout::Button)
        .anti_alias_methods(&ANTI_ALIAS_METHODS);
    if let Some(languages) = &languages {
        controls = controls.languages(languages);
    }
    if let Some((axes, instances)) = &face {
        controls = controls.axes(FaceAxes::new(axes, instances).italic(face_of(&current.font_style).2));
    }
    let out = controls.show(ui);
    font_menu::store_picker_settings(app, &before, settings);
    let CharacterControlsOutput { picker, preview_family, edits, .. } = out;
    if let Some(owner) = picker {
        font_menu::apply_preview(app, owner, preview_family);
    }
    for event in edits {
        apply_character_event(app, event);
    }
}

/// The Character panel: the controls over the selected type (an empty state without any), then the
/// Snap to Glyph options while they are shown, which need no type.
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    if text_style(app).is_none() {
        app.character_focus = None;
        super::empty_state(ui, "type", tl!("No text selected"), tl!("Select a text object to edit its character attributes."));
    } else {
        controls(app, ui, rows(app));
        // The Highlight commands' request is a signal the framework resolves once the field exists.
        if let Some(field) = app.character_focus.take() {
            focus::request(ui.ctx(), FocusTarget::Doc(field.key()));
        }
        // Ruby (ふりがな): the reading over the selected characters, its placement and options.
        ruby_section(app, ui);
    }
    if app.ui.snap_to_glyph_options {
        snap_to_glyph(app, ui);
    }
}

/// The Snap to Glyph block (Show Snap to Glyph Options): View › Snap to Glyph itself, then which
/// guides it gives, each running `view.snapToGlyph.options` (dimmed while Snap to Glyph is off).
/// Showing the block scrolls the panel to it (#5273); it does not turn snapping on.
fn snap_to_glyph(app: &mut VectorcraftApp, ui: &mut Ui) {
    let reveal = std::mem::take(&mut app.ui.snap_to_glyph_reveal);
    // The estimate is the block's length before it first runs: a checkbox and three rows of two.
    Lazy::new("ch-snap-to-glyph", 120.0).reveal(reveal).show(ui, |ui| {
        let enabled = app.ui.view.snap_to_glyph;
        let mut on = enabled;
        Checkbox::new(Bind::bound(&mut on)).label(tl!("Snap to Glyph")).show(ui);
        if on != enabled {
            // A UI command that takes no params: it can't refuse.
            app.run("view.snapToGlyph", json!({})).ok();
        }
        let o = app.session.prefs.glyph_snap;
        let options = [
            (tl!("Baseline"), "baseline", o.baseline),
            (tl!("x-height"), "xHeight", o.x_height),
            (tl!("Glyph Bounds"), "bounds", o.bounds),
            (tl!("Proximity Guides"), "proximity", o.proximity),
            (tl!("Angular Guides"), "angular", o.angular),
            (tl!("Anchor Point"), "anchors", o.anchors),
        ];
        for pair in options.chunks(2) {
            Row::new().show(ui, |ui, _| {
                for &(label, key, held) in pair {
                    let mut value = held;
                    Checkbox::new(Bind::bound(&mut value)).label(label).enabled(enabled).show(ui);
                    if value != held {
                        // A known key and a bool: the command can't refuse it.
                        app.run("view.snapToGlyph.options", json!({ key: value })).ok();
                    }
                }
            });
        }
    });
}

/// The Font Size field of the bars, named `name`: typed, stepped, scrubbed or picked from the
/// presets, in the type unit, blank where the selected text's sizes differ.
fn size_field(app: &mut VectorcraftApp, ui: &mut Ui, name: &'static str) {
    let unit = type_unit(app);
    let scale = unit_scale(unit);
    let units = Env::of(ui).units();
    let presets: Vec<Entry<NumberPick>> = SIZE_PRESETS
        .iter()
        .map(|&p| {
            let n = f64::from(p) * scale;
            Entry::new(NumberPick::Number(n), units.number(n, 0..=2))
        })
        .collect();
    let mut size = shared_effective(app, |s| s.size).map(|v| v * scale);
    let out = NumberField::new(name, Bind::bound(&mut size)).unit_kind(unit.kind).range(0.1 * scale..=1296.0 * scale).presets(&presets).show(ui);
    apply_edits(app, &out.edit, EditPath::Scope, |app, value, _| {
        if let Some(v) = value {
            style(app, json!({"size": v / scale}));
        }
    });
}

/// The Control bar's type controls while type is selected or edited: the Character link (the
/// Character panel in a popover), the font family and style menus and the Font Size field. The
/// bar's own elision trims them, so they always show: it returns `true`.
pub(crate) fn control_bar(app: &mut VectorcraftApp, ui: &mut Ui) -> bool {
    if text_style(app).is_none() {
        return true;
    }
    Divider::vertical().show(ui);
    let link = crate::panels::option_link(tl!("Character:")).show(ui).response;
    Popover::new(tl!("Character options"), Bind::kept()).overlay(|overlay| overlay.opened_by(&link, Trigger::Click)).show(ui, |ui, _| show(app, ui));
    Row::new().show(ui, |ui, _| {
        font_menu::font_fields(app, ui);
        size_field(app, ui, tl!("Font Size"));
    });
    true
}

/// The Contextual Task Bar's type controls while type is selected or edited: the font fields and the
/// Font Size field, named for screen readers.
pub(crate) fn task_bar(app: &mut VectorcraftApp, ui: &mut Ui) {
    if text_style(app).is_none() {
        return;
    }
    font_menu::font_fields(app, ui);
    size_field(app, ui, tl!("Font Size"));
}

/// The position kind (`normal`, `superscript` or `subscript`) that every character a Superscript
/// or Subscript toggle changes has, else `""`: at a Type tool caret with no characters selected
/// every character of the text (of each frame of a threaded story), as the toggle changes all of
/// it there ([`run_format`], [`range_style`]), else the characters [`shared`] reads. Superscripts
/// in different proportions count as superscript.
fn shared_position(app: &VectorcraftApp) -> &'static str {
    let Some((id, _, _)) = text_editing(app).filter(|&(_, a, b)| b <= a) else {
        return shared(app, |s| s.position.id()).unwrap_or("");
    };
    let Some(st) = app.session.active() else { return "" };
    let mut acc = Shared::None;
    for frame in text_story(app).unwrap_or_else(|| vec![id]) {
        if let Some(NodeKind::Text(t)) = st.doc.node(frame).map(|n| &n.kind) {
            each_paragraph_run(t, None, |_, style, _| acc.add(style.position.id()));
        }
    }
    acc.value().unwrap_or("")
}

/// The `text.setFormat` params of the Superscript or Subscript toggle `p` for text whose
/// position is `position`: `normal` when `p` is on, else `p`, which turns the other one off.
fn position_toggle(position: &str, p: &str) -> Value {
    json!({"position": if position == p { "normal" } else { p }})
}

/// A panel menu row: a command with its label and state, a separator, or a submenu.
enum Item {
    Row(CommandRef, String, CommandState),
    Separator,
    Submenu(String, Vec<Item>),
}

/// The row of command `id` with `params`.
fn row(id: &str, params: Value, label: &str, state: CommandState) -> Item {
    Item::Row(CommandRef::with_params(id, params), label.to_string(), state)
}

/// A `ui.character.set` row: attribute `key` set to `value`, checked while the selection has it.
fn set_row(key: &str, value: &str, label: &str, checked: bool, enabled: bool) -> Item {
    row("ui.character.set", json!({"key": key, "value": value}), label, CommandState::new().enabled(enabled).checked(checked))
}

/// The Character Direction submenu: Default, Left to Right and Right to Left set the characters the
/// panel acts on ([`char_cmd`]) as the bidi algorithm sets them, or left to right or right to left
/// inside their paragraph. The direction those characters share is checked, and none is checked
/// when they differ. At a caret in text the Type tool edits, with no characters selected, the items
/// are disabled.
fn char_direction_items(app: &VectorcraftApp, has: bool) -> Vec<Item> {
    use vectorcraft_doc::ParaDirection;
    let direction = shared(app, |s| s.char_direction);
    let enabled = has && text_editing(app).is_none_or(|(_, a, b)| b > a);
    [
        (tl!("Default"), None, "default"),
        (tl!("Left to Right"), Some(ParaDirection::LeftToRight), "leftToRight"),
        (tl!("Right to Left"), Some(ParaDirection::RightToLeft), "rightToLeft"),
    ]
    .into_iter()
    .map(|(label, d, key)| set_row("charDirection", key, label, direction == Some(d), enabled))
    .collect()
}

/// The Character Alignment submenu: where characters smaller than the largest on their line line up.
fn char_align_items(app: &VectorcraftApp, has: bool) -> Vec<Item> {
    use vectorcraft_doc::CharAlign;
    let align = shared(app, |s| s.char_align);
    [
        (tl!("Roman Baseline"), CharAlign::RomanBaseline, "romanBaseline"),
        (tl!("Em Box Top/Right"), CharAlign::EmBoxTop, "emBoxTop"),
        (tl!("Em Box Center"), CharAlign::EmBoxCenter, "emBoxCenter"),
        (tl!("Em Box Bottom/Left"), CharAlign::EmBoxBottom, "emBoxBottom"),
        (tl!("ICF Top/Right"), CharAlign::IcfTop, "icfTop"),
        (tl!("ICF Bottom/Left"), CharAlign::IcfBottom, "icfBottom"),
    ]
    .into_iter()
    .map(|(label, a, key)| set_row("charAlign", key, label, align == Some(a), has))
    .collect()
}

/// The numeral systems Character panel menu › Digits lists after Default, with their English
/// labels, translated in the `digits` context (Hindi, Bengali, Tamil… are also languages): the
/// Arabic-script ones, then the Indic scripts' own digits from Devanagari on.
pub(crate) const DIGITS: [(&str, vectorcraft_doc::Digits); 12] = {
    use vectorcraft_doc::Digits;
    [
        ("Arabic", Digits::Arabic),
        ("Hindi", Digits::Hindi),
        ("Farsi", Digits::Farsi),
        ("Devanagari", Digits::Devanagari),
        ("Bengali", Digits::Bengali),
        ("Gurmukhi", Digits::Gurmukhi),
        ("Gujarati", Digits::Gujarati),
        ("Odia", Digits::Odia),
        ("Tamil", Digits::Tamil),
        ("Telugu", Digits::Telugu),
        ("Kannada", Digits::Kannada),
        ("Malayalam", Digits::Malayalam),
    ]
};

/// Character panel menu › Digits: the numeral system the selected characters' digits are drawn in
/// (checked when they share it).
fn digits_items(app: &VectorcraftApp, has: bool) -> Vec<Item> {
    use vectorcraft_doc::Digits;
    let digits = shared(app, |s| s.digits);
    let mut items = vec![set_row("digits", Digits::Default.id(), tl!("Default"), digits == Some(Digits::Default), has)];
    for (label, d) in DIGITS {
        if d == Digits::Devanagari {
            items.push(Item::Separator);
        }
        items.push(set_row("digits", d.id(), crate::i18n::tr_ctx(crate::i18n::current(), "digits", label), digits == Some(d), has));
    }
    items
}

/// The panel menu's rows with their labels and states: the one list [`menu_rows`] shows and
/// [`menu_states`] describes.
fn menu_items(app: &VectorcraftApp) -> Vec<Item> {
    let has = text_style(app).is_some();
    let prefs = &app.session.prefs;
    // A check item is checked when every character the panel acts on has it; choosing it with a
    // mixed selection turns it on.
    let toggle = |key: &str, label: &str, enabled: bool| {
        row("ui.character.toggle", json!({"key": key}), label, CommandState::new().enabled(enabled).checked(toggled(app, key)))
    };
    let option = |option: &str, shown: bool, hide: &str, show: &str| {
        row("ui.character.show", json!({"option": option}), if shown { hide } else { show }, CommandState::new())
    };
    let position = shared_position(app);
    let place = |id: &str, label: &str, p: &str| row(id, Value::Null, label, CommandState::new().enabled(has).checked(position == p));
    let mut items = vec![
        option("options", !app.ui.character_hide_options, tl!("Hide Options"), tl!("Show Options")),
        option("snapToGlyph", app.ui.snap_to_glyph_options, tl!("Hide Snap to Glyph Options"), tl!("Show Snap to Glyph Options")),
        option("fontHeight", app.ui.character_font_height, tl!("Hide Font Height Options"), tl!("Show Font Height Options")),
        Item::Separator,
        toggle("allCaps", tl!("All Caps"), has),
        toggle("underline", tl!("Underline"), has),
        toggle("strikethrough", tl!("Strikethrough"), has),
        toggle("smallCaps", tl!("Small Caps"), has),
        place("type.superscript", tl!("Superscript"), "superscript"),
        place("type.subscript", tl!("Subscript"), "subscript"),
        Item::Separator,
    ];
    if prefs.show_east_asian_options {
        items.push(Item::Submenu(tl!("Character Alignment").to_string(), char_align_items(app, has)));
        items.extend([
            toggle("proportionalMetrics", tl!("Proportional Metrics"), has),
            toggle("tateChuYoko", tl!("Tate-chu-yoko"), has),
            row("ui.tateChuYokoSettings", Value::Null, tl!("Tate-chu-yoko Settings…"), CommandState::new().enabled(has)),
            toggle("warichu", tl!("Warichu"), has),
            row("ui.warichuSettings", Value::Null, tl!("Warichu Settings…"), CommandState::new().enabled(has)),
            toggle("standardVerticalRoman", tl!("Standard Vertical Roman Alignment"), has),
            Item::Separator,
        ]);
    }
    if prefs.show_indic_options {
        // A mixed selection is unchecked; choosing Kashidas enables them for the whole range.
        let selected_characters = text_editing(app).is_none_or(|(_, start, end)| end > start);
        items.extend([
            Item::Submenu(tl!("Digits").to_string(), digits_items(app, has)),
            Item::Submenu(tl!("Character Direction").to_string(), char_direction_items(app, has)),
            toggle("kashidas", tl!("Kashidas"), has && selected_characters),
            Item::Separator,
        ]);
    }
    let open = app.session.active().is_some();
    let system = app.session.active().is_some_and(|st| st.doc.setup.system_layout);
    items.extend([
        toggle("noBreak", tl!("No Break"), has),
        row("type.fractionalWidths", Value::Null, tl!("Fractional Widths"), CommandState::new().enabled(has).checked(fractional_widths_on(app))),
        // System Layout is whole-point spacing for every text object in the document: a document is enough.
        row("type.systemLayout", Value::Null, tl!("System Layout"), CommandState::new().enabled(open).checked(system)),
        Item::Separator,
    ]);
    // The installed fonts are always listed; this picks up fonts installed since the app started.
    #[cfg(not(target_arch = "wasm32"))]
    items.push(row("ui.character.refreshFonts", json!({}), tl!("Refresh Font List"), CommandState::new()));
    items.push(row("ui.character.reset", json!({}), tl!("Reset Panel"), CommandState::new().enabled(has)));
    items
}

/// `items` as the panel menu's rows.
fn panel_rows(items: Vec<Item>) -> Vec<PanelRow> {
    // PLAN: assumed PanelRow::{Command, Separator, Submenu} as ui-widgets menu/panel.rs defines them.
    items
        .into_iter()
        .map(|item| match item {
            Item::Row(command, ..) => PanelRow::Command(command),
            Item::Separator => PanelRow::Separator,
            Item::Submenu(label, rows) => PanelRow::Submenu(label.into(), panel_rows(rows)),
        })
        .collect()
}

/// The Character panel menu's rows.
pub(crate) fn menu_rows(app: &VectorcraftApp) -> Vec<PanelRow> {
    panel_rows(menu_items(app))
}

/// Gives each row of the Character panel menu its label, check and enabled state.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    fn record(items: Vec<Item>, states: &mut CommandStates) {
        for item in items {
            match item {
                Item::Row(command, label, state) => states.set(command, state.title(label)),
                Item::Submenu(_, rows) => record(rows, states),
                Item::Separator => {}
            }
        }
    }
    record(menu_items(app), states);
}

/// Runs the `ui.character.*` command `id`; `None` for any other id.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let action = id.strip_prefix("ui.character.")?;
    // A row the menu shows disabled does nothing, however it is run.
    let mut states = CommandStates::new();
    menu_states(app, &mut states);
    if states.get(&CommandRef::with_params(id, params.clone())).is_some_and(|state| !state.is_enabled()) {
        return Some(Err(format!("{id} is not available now")));
    }
    Some(run_action(app, action, params))
}

fn run_action(app: &mut VectorcraftApp, action: &str, params: &Value) -> Result<Value, String> {
    let text = |name: &str| params.get(name).and_then(Value::as_str).ok_or_else(|| format!("ui.character.{action} needs {name}"));
    match action {
        "toggle" => {
            let key = text("key")?;
            if !TOGGLES.iter().any(|(k, _)| *k == key) {
                return Err(format!("unknown character attribute {key}"));
            }
            let on = !toggled(app, key);
            format(app, json!({ key: on }));
            Ok(json!(on))
        }
        "set" => {
            let key = text("key")?;
            if !["charAlign", "charDirection", "digits"].contains(&key) {
                return Err(format!("unknown character attribute {key}"));
            }
            let value = params.get("value").cloned().ok_or("ui.character.set needs value")?;
            format(app, json!({ key: value }));
            Ok(Value::Null)
        }
        "show" => {
            let flip = |flag: &mut bool| {
                *flag = !*flag;
                *flag
            };
            let on = match text("option")? {
                "options" => flip(&mut app.ui.character_hide_options),
                "fontHeight" => flip(&mut app.ui.character_font_height),
                "snapToGlyph" => {
                    let on = flip(&mut app.ui.snap_to_glyph_options);
                    app.ui.snap_to_glyph_reveal = on;
                    on
                }
                other => return Err(format!("unknown character option {other}")),
            };
            Ok(json!(on))
        }
        "reset" => {
            reset_panel(app);
            Ok(Value::Null)
        }
        #[cfg(not(target_arch = "wasm32"))]
        "refreshFonts" => {
            let r = app.run("text.rescanFonts", json!({}))?;
            app.status(format!("{} font families available", r["families"]));
            Ok(r)
        }
        other => Err(format!("unknown command ui.character.{other}")),
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

/// Reset Panel, in one undo step: the type objects the panel acts on take the Sharp anti-aliasing
/// method, and the characters the panel acts on ([`char_cmd`]) take tracking 0, Auto leading and
/// kerning, no baseline shift, 100% scales, no rotation, no underline or strikethrough, All Caps
/// and Small Caps off, the normal position (neither superscript nor subscript), Roman Baseline
/// character alignment, Proportional Metrics, Tate-chu-yoko, Warichu, Standard Vertical Roman Alignment and
/// No Break off, Tate-chu-yoko Settings Up/Down and Left/Right 0, the default Warichu Settings (2 lines, no
/// line gap, 50%, Auto alignment), Insert Aki Left and Right 0, Tsume 0, the Default character direction,
/// Kashidas on, the Default digits, Adjust Horizontal and Vertical Diacritic Position 0 and the Em Box
/// font height reference, whether or not Show East Asian
/// Options, Show Indic Options and Show Font Height Options are on. Inside an undo group that is
/// already open, the reset is part of that group's step.
fn reset_panel(app: &mut VectorcraftApp) {
    // Typing in progress stays an undo step of its own.
    end_typing(app);
    let open = app.session.active().is_some_and(|st| st.undo_group.is_some());
    if !open {
        app.session.begin_named_undo_group(Some("Reset Panel".into()));
    }
    let d = CharStyle::default();
    style(app, json!({"tracking": 0, "leading": "auto"}));
    format(
        app,
        json!({
            "kerning": "auto", "baselineShift": 0, "hScale": 100, "vScale": 100, "rotation": 0, "underline": false, "strikethrough": false,
            "allCaps": false, "smallCaps": d.small_caps.is_some(), "position": d.position.id(), "charAlign": d.char_align,
            "proportionalMetrics": d.proportional_metrics, "tateChuYoko": d.tate_chu_yoko,
            "tcyUpDown": d.tcy_up_down, "tcyLeftRight": d.tcy_left_right, "akiLeft": d.aki_left, "akiRight": d.aki_right,
            "tsume": d.tsume, "warichu": d.warichu, "warichuLines": d.warichu_lines, "warichuGap": d.warichu_gap, "warichuScale": d.warichu_scale,
            "warichuAlign": d.warichu_align,
            "standardVerticalRoman": d.standard_vertical_roman, "noBreak": d.no_break, "charDirection": "default", "kashidas": d.kashidas,
            "digits": d.digits, "diacriticHorizontal": d.diacritic_horizontal, "diacriticVertical": d.diacritic_vertical,
            "fontHeightReference": d.font_height_reference
        }),
    );
    set_anti_alias(app, TypeAntiAlias::Sharp);
    // Closes the active document's undo group only, so an undo group open in another document
    // stays open.
    if !open && let Some(st) = app.session.active_mut() {
        st.undo_group = None;
    }
}

// ---------- Ruby (ふりがな, phonetic guide) ----------

/// The Ruby section's fields, kept between frames; `synced` is the selection and ruby they were
/// read from, so they reload when either changes.
#[derive(Clone, Debug, PartialEq)]
struct RubyFields {
    reading: String,
    /// "above" or "below" (right/left of the column in vertical type).
    position: String,
    /// "start", "center" or "end".
    align: String,
    /// Percentage of the base text's size, 1–100.
    size: f64,
    overhang: bool,
    synced: String,
}

impl Default for RubyFields {
    fn default() -> Self {
        Self { reading: String::new(), position: "above".into(), align: "center".into(), size: 50.0, overhang: true, synced: String::new() }
    }
}

impl RubyFields {
    /// The fields of `span`, or the defaults (no reading, above, centred, half size, overhanging).
    fn from_span(span: Option<&RubySpan>) -> Self {
        let mut f = Self::default();
        if let Some(r) = span {
            f.reading.clone_from(&r.text);
            f.position = match r.position {
                RubyPosition::Above => "above",
                RubyPosition::Below => "below",
            }
            .into();
            f.align = match r.align {
                RubyAlign::Center => "center",
                RubyAlign::Start => "start",
                RubyAlign::End => "end",
            }
            .into();
            f.size = r.safe_size_percent();
            f.overhang = r.overhang;
        }
        f
    }
}

/// The text the Ruby section acts on: the Type tool's selected characters when it edits a range,
/// else the first selected text object's whole text.
fn ruby_target(app: &VectorcraftApp) -> Option<(NodeId, usize, usize)> {
    if let Some((id, a, b)) = text_editing(app) {
        return (b > a).then_some((id, a, b));
    }
    let st = app.session.active()?;
    for node in st.selection.objects.iter().filter_map(|id| st.doc.node(*id)) {
        let mut target: Option<(NodeId, usize)> = None;
        node.walk(&mut |n| {
            if target.is_none()
                && let NodeKind::Text(t) = &n.kind
            {
                target = Some((n.id, t.plain_text().len()));
            }
        });
        if let Some((id, len)) = target {
            return (len > 0).then_some((id, 0, len));
        }
    }
    None
}

/// The ruby over `a..b` of `t`: the first span touching the range.
fn ruby_in(t: &TextObject, a: usize, b: usize) -> Option<&RubySpan> {
    t.ruby_in(a, b).next()
}

/// Whether `a..b` is exactly one ruby span: an option edit can re-apply it in place, without
/// dropping another reading under the selection or changing the span's range. Apply replaces the
/// ruby over the selection when it is not.
fn ruby_covers(t: &TextObject, a: usize, b: usize) -> bool {
    let mut spans = t.ruby_in(a, b);
    spans.next().is_some_and(|r| r.start == a && r.end == b) && spans.next().is_none()
}

/// The selection and ruby the section's fields were built from.
fn ruby_sync_key(id: NodeId, a: usize, b: usize, span: Option<&RubySpan>) -> String {
    let ruby = span.map_or_else(String::new, |r| format!("{}|{:?}|{:?}|{}|{}", r.text, r.position, r.align, r.size_percent, r.overhang));
    format!("{}:{a}:{b}:{ruby}", id.0)
}

/// The `type.ruby` params of the section: set the reading over `a..b`, or `remove` it.
fn ruby_params(id: NodeId, a: usize, b: usize, f: &RubyFields, remove: bool) -> Value {
    let mut p = json!({"id": id.0, "start": a, "end": b});
    if remove {
        p["remove"] = json!(true);
        return p;
    }
    p["text"] = json!(f.reading.trim());
    p["position"] = json!(f.position);
    p["align"] = json!(f.align);
    p["size"] = json!(f.size);
    p["overhang"] = json!(f.overhang);
    p
}

/// Draw the Ruby section: the reading over the selected characters, its position, alignment,
/// size and overhang, with Apply and Remove. Every action goes through `type.ruby` in one undo
/// step; the fields follow the selection and the ruby it covers. An option edit re-applies the
/// ruby only when it is the one span over exactly the selection; Apply replaces the ruby over
/// the selection.
/// A dim secondary label, as the section's field labels and the applied mark draw.
fn dim_label(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).weak());
}

fn ruby_section(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some((id, a, b)) = ruby_target(app) else { return };
    let (span, auto_apply) = app.session.active().and_then(|st| st.doc.node(id)).map_or((None, false), |n| match &n.kind {
        NodeKind::Text(t) => (ruby_in(t, a, b).cloned(), ruby_covers(t, a, b)),
        _ => (None, false),
    });
    let key = ruby_sync_key(id, a, b, span.as_ref());
    let mut f = pstate::<RubyFields>(ui.ctx(), "ruby-fields");
    if f.synced != key {
        f = RubyFields::from_span(span.as_ref());
        f.synced.clone_from(&key);
    }
    ui.add_space(4.0);
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Ruby")).strong());
        if span.is_some() {
            dim_label(ui, tl!("applied"));
        }
    });
    let mut apply = false;
    ui.horizontal(|ui| {
        dim_label(ui, &format!("{}:", tl!("Reading")));
        ui.add(egui::TextEdit::singleline(&mut f.reading).hint_text(tl!("Reading")).desired_width(f32::INFINITY));
    });
    ui.horizontal(|ui| {
        dim_label(ui, &format!("{}:", tl!("Position")));
        let options = [Entry::new(tl!("Above").to_string(), tl!("Above")), Entry::new(tl!("Below").to_string(), tl!("Below"))];
        let text = if f.position == "below" { tl!("Below") } else { tl!("Above") }.to_string();
        let pick = ComboBox::new(tl!("Position"), Bind::given(text), &options).id_salt("ruby-position").show(ui);
        if let Some(text) = pick.edit.events.committed() {
            let value = if text.as_str() == tl!("Below") { "below" } else { "above" };
            if f.position != value {
                f.position = value.into();
                apply = auto_apply;
            }
        }
        dim_label(ui, &format!("{}:", tl!("Align")));
        let options = [
            Entry::new(tl!("Start").to_string(), tl!("Start")),
            Entry::new(tl!("Center").to_string(), tl!("Center")),
            Entry::new(tl!("End").to_string(), tl!("End")),
        ];
        let text = tl!(match f.align.as_str() {
            "start" => "Start",
            "end" => "End",
            _ => "Center",
        })
        .to_string();
        let pick = ComboBox::new(tl!("Align"), Bind::given(text), &options).id_salt("ruby-align").show(ui);
        if let Some(text) = pick.edit.events.committed() {
            let value = if text.as_str() == tl!("Start") {
                "start"
            } else if text.as_str() == tl!("End") {
                "end"
            } else {
                "center"
            };
            if f.align != value {
                f.align = value.into();
                apply = auto_apply;
            }
        }
    });
    ui.horizontal(|ui| {
        dim_label(ui, &format!("{}:", tl!("Size")));
        let size = ui.add(egui::DragValue::new(&mut f.size).range(1.0..=100.0).speed(1.0).suffix("%"));
        // Dragging sets the value as it goes and ends here; typed, it commits on Enter or focus
        // loss. Apply then, unless it is already the ruby's size.
        if (size.drag_stopped() || size.lost_focus()) && auto_apply && span.as_ref().is_some_and(|r| r.safe_size_percent() != f.size) {
            apply = true;
        }
        if ui.checkbox(&mut f.overhang, tl!("Overhang")).changed() && auto_apply {
            apply = true;
        }
    });
    let remove = ui
        .horizontal(|ui| {
            let mut remove = false;
            if ui.add_enabled(!f.reading.trim().is_empty(), egui::Button::new(tl!("Apply"))).clicked() {
                apply = true;
            }
            if ui.add_enabled(span.is_some(), egui::Button::new(tl!("Remove"))).clicked() {
                remove = true;
            }
            remove
        })
        .inner;
    if remove || apply {
        // A typing session is its own undo step; the ruby command is the next one.
        end_typing(app);
        if remove {
            app.run("type.ruby", ruby_params(id, a, b, &f, true)).ok();
        } else {
            app.run("type.ruby", ruby_params(id, a, b, &f, false)).ok();
        }
    }
    set_pstate(ui.ctx(), "ruby-fields", f);
}

#[cfg(test)]
mod tests;
