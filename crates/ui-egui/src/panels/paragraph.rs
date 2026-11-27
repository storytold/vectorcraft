//! Paragraph panel: seven alignment buttons, Paragraph Direction (with the Indic options), the
//! Bullets and Numbered List buttons and their presets (More Options opens Bullets and
//! Numbering…), indents, space before/after, Hyphenate, Insert Kashidas (with the Indic options),
//! Mojikumi Set and Kinsoku Set (with the East Asian options); the panel menu turns Roman Hanging
//! Punctuation on and off, opens Justification… and Hyphenation…, picks the Kinsoku Shori Type
//! (with the East Asian options) and picks the Single-line or Every-line Composer.

// PLAN: assumed import paths for the framework items below (behavior::{IconId, Mixed}, keymap::CommandRef,
// text::ui::character, ui::{buttons, choice::Select, controls::Checkbox, menu, popover, runtime::access::PopupKind}).
use craft::behavior::{IconId, Mixed};
use craft::keymap::CommandRef;
use craft::text::ui::character::{DirectionRow, ParagraphChange, ParagraphControls, ParagraphDirection, ParagraphValues, TextAlign};
use craft::ui::buttons::{IconButton, PopupMark, ToggleButton};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, MemoProps, Props as _, Show as _, ShowContainer as _, Trigger};
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::menu::{Menu, PanelRow};
use craft::ui::pickers::Select;
use craft::ui::popover::Popover;
use craft::ui::primitives::{Glyph, IconSpec};
use craft::ui::runtime::access::PopupKind;
use craft::ui::runtime::commands::{CommandState, CommandStates};
use craft::ui::style::UiTokens;
use egui::{Color32, Painter, Rect, Stroke, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::text::{Bullet, Kashida, NumberStyle};
use vectorcraft_doc::{
    Burasagari, Composer, HyphenationOptions, JustificationOptions, Justify, KinsokuShoriType, LeadingModel, Mojikumi, NodeKind, ParaDirection,
    ParaStyle, SingleWordJustify,
};

use super::character::{Metric, Shared, doc_unit, text_style};
use crate::VectorcraftApp;
use crate::edits::{self, EditPath};

pub const ALIGNMENTS: [(Justify, &str, &str, &str); 7] = [
    (Justify::Left, "dc-para-left", "Align left", "left"),
    (Justify::Center, "dc-para-center", "Align center", "center"),
    (Justify::Right, "dc-para-right", "Align right", "right"),
    (Justify::JustifyLeft, "dc-para-justify-left", "Justify with last line aligned left", "justifyLeft"),
    (Justify::JustifyCenter, "dc-para-justify-center", "Justify with last line aligned center", "justifyCenter"),
    (Justify::JustifyRight, "dc-para-justify-right", "Justify with last line aligned right", "justifyRight"),
    (Justify::JustifyAll, "dc-para-justify-all", "Justify all lines", "justifyAll"),
];

/// The framework's alignment of each [`ALIGNMENTS`] row, in the same order.
const TEXT_ALIGNS: [TextAlign; 7] = [
    TextAlign::Left,
    TextAlign::Center,
    TextAlign::Right,
    TextAlign::JustifyLeft,
    TextAlign::JustifyCenter,
    TextAlign::JustifyRight,
    TextAlign::JustifyAll,
];

/// The value of the "More Options" row in the list menus; the preset rows are numbered from 0.
const MORE: u32 = u32::MAX;

/// Paragraph attributes apply to the paragraphs of every frame of the selected stories or, while
/// the Type tool edits text, to the paragraphs its selection (or caret) touches (ending its typing
/// session).
fn format(app: &mut VectorcraftApp, p: Value) {
    para_cmd(app, "text.setFormat", p);
}

/// The alignment shortcuts' commands (Cmd+Shift+L, C, R, J and F, as in page layout apps) and the
/// `justify` each sets ([`align`]).
pub const ALIGN_COMMANDS: [(&str, &str); 5] = [
    ("type.alignLeft", "left"),
    ("type.alignCenter", "center"),
    ("type.alignRight", "right"),
    ("type.justifyLeft", "justifyLeft"),
    ("type.justifyAll", "justifyAll"),
];

/// Align the paragraphs of the selected type, or those the Type tool's selection touches, as
/// `justify` (an [`ALIGN_COMMANDS`] alignment, #1111).
pub(crate) fn align(app: &mut VectorcraftApp, justify: &str) -> Result<Value, String> {
    text_style(app).ok_or("select some type first")?;
    para_cmd(app, "text.setStyle", json!({ "justify": justify }));
    Ok(Value::Null)
}

/// `type.lineComposer` (Cmd+Alt+Shift+C): the Single-line Composer for the paragraphs the panel
/// acts on when they all break their lines every-line (either shaping), else the Every-line
/// Composer. Paragraphs that all use a Standard composer keep it; any other mix takes the Middle
/// Eastern & South Asian one. Returns the composer set.
pub(crate) fn toggle_composer(app: &mut VectorcraftApp) -> Result<Value, String> {
    text_style(app).ok_or("select some type first")?;
    let to_single = shared_para(app, |p| p.composer.is_single_line()) == Some(false);
    let standard = shared_para(app, |p| p.composer.shapes_complex_scripts()) == Some(false);
    let base = if standard { Composer::StandardEveryLine } else { Composer::EveryLine };
    let id = composer_id(base.with_breaking(to_single));
    format(app, json!({ "composer": id }));
    Ok(json!(id))
}

/// The `text.setFormat` value of `composer`.
fn composer_id(composer: Composer) -> &'static str {
    match composer {
        Composer::SingleLine => "singleLine",
        Composer::EveryLine => "everyLine",
        Composer::StandardSingleLine => "standardSingleLine",
        Composer::StandardEveryLine => "standardEveryLine",
    }
}

/// `type.hyphenate` (Cmd+Alt+Shift+H): what the Hyphenate checkbox does, on the paragraphs the
/// panel acts on. Returns whether they now hyphenate.
pub(crate) fn toggle_hyphenate(app: &mut VectorcraftApp) -> Result<Value, String> {
    text_style(app).ok_or("select some type first")?;
    let on = shared_hyphenate(app) != Some(true);
    format(app, json!({ "hyphenate": on }));
    Ok(json!(on))
}

fn para_cmd(app: &mut VectorcraftApp, cmd: &str, p: Value) {
    run_on_paragraphs(app, cmd, p).ok();
}

/// Run `cmd` on the paragraphs the Paragraph panel acts on ([`format`]) and return its result. With
/// the Selection tool these are the paragraphs of every frame of the selected stories
/// ([`super::character::selected_stories`]).
pub(crate) fn run_on_paragraphs(app: &mut VectorcraftApp, cmd: &str, mut p: Value) -> Result<Value, String> {
    if let Some((id, a, b)) = super::character::text_editing_kept(app)? {
        match super::character::text_story(app) {
            // After Select All in threaded text: every paragraph of every frame.
            Some(story) => p["ids"] = json!(story.iter().map(|f| f.0).collect::<Vec<_>>()),
            None => {
                p["ids"] = json!([id.0]);
                p["start"] = json!(a);
                p["end"] = json!(b);
            }
        }
    } else {
        super::character::target_selected_stories(app, &mut p);
    }
    app.run(cmd, p)
}

/// Does the paragraph being edited (at the caret), else the selected text's first paragraph, run
/// right to left: its Paragraph Direction, else from its first strong character?
fn resolved_rtl(app: &VectorcraftApp, para: &ParaStyle) -> bool {
    let editing = super::character::text_editing(app);
    let text = editing.and_then(|(id, _, _)| match app.session.active().and_then(|d| d.doc.node(id)).map(|n| &n.kind) {
        Some(NodeKind::Text(t)) => Some(t.as_ref()),
        _ => None,
    });
    let Some(t) = text.or_else(|| super::character::first_selected_text(app)) else { return para.direction == Some(ParaDirection::RightToLeft) };
    let plain = t.plain_text();
    let paragraph = vectorcraft_text::edit::paragraph_at(&plain, editing.map_or(0, |(_, a, _)| a));
    vectorcraft_text::paragraph_is_rtl(plain.get(paragraph).unwrap_or_default(), para.direction)
}

/// The alignment button that shows `justify` in a paragraph running right to left (`rtl`) or not:
/// Auto aligns to the start of its direction.
fn shown_alignment(justify: Justify, rtl: bool) -> Justify {
    match justify {
        Justify::Auto if rtl => Justify::Right,
        Justify::Auto => Justify::Left,
        j => j,
    }
}

/// Add the displayed alignments touched by the byte range (all paragraphs without a range).
/// Walk forward through run fragments so paragraph lookup never rescans an earlier prefix.
/// Only Auto needs paragraph text; borrow a fragment until style runs split it.
fn collect_text_alignment(text: &vectorcraft_doc::TextObject, range: Option<(usize, usize)>, shared: &mut Option<Justify>) -> bool {
    use std::borrow::Cow;
    let (a, b) = range.map_or((0, usize::MAX), |(a, b)| (a.min(b), a.max(b)));
    let mut record = |index, plain: &str, start, end| {
        if end < a || start > b {
            return true;
        }
        let para = text.para_at(index);
        let rtl = para.justify == Justify::Auto && vectorcraft_text::paragraph_is_rtl(plain, para.direction);
        let value = shown_alignment(para.justify, rtl);
        if shared.is_some_and(|old| old != value) {
            return false;
        }
        *shared = Some(value);
        true
    };
    let (mut index, mut start, mut end) = (0, 0, 0);
    let mut plain = Cow::Borrowed("");
    for run in &text.runs {
        let mut rest = run.text.as_str();
        while !rest.is_empty() {
            let span = vectorcraft_text::edit::paragraph_at(rest, 0);
            let part = rest.get(span.clone()).unwrap_or_default();
            let para = text.para_at(index);
            if para.justify == Justify::Auto && !part.is_empty() {
                if plain.is_empty() {
                    plain = Cow::Borrowed(part);
                } else {
                    plain.to_mut().push_str(part);
                }
            }
            end += part.len();
            if span.end == rest.len() {
                break; // The same paragraph can continue in the next style run.
            }
            if !record(index, &plain, start, end) {
                return false;
            }
            // Like TextObject::paragraphs_in, a range ending at the next paragraph's start
            // includes that paragraph; the newline itself belongs to the preceding one.
            end += 1;
            start = end;
            index += 1;
            if start > b {
                return true;
            }
            plain = Cow::Borrowed("");
            rest = rest.get(span.end + 1..).unwrap_or_default();
        }
    }
    record(index, &plain, start, end) // Includes empty text and the paragraph after a final newline.
}

/// The frames of the threaded story whose whole text the Type tool has selected after Select All
/// ([`super::character::text_story`]), in thread order.
fn story_texts(app: &VectorcraftApp) -> Option<Vec<&vectorcraft_doc::TextObject>> {
    let st = app.session.active()?;
    let story = super::character::text_story(app)?;
    Some(
        story
            .into_iter()
            .filter_map(|id| match st.doc.node(id).map(|n| &n.kind) {
                Some(NodeKind::Text(t)) => Some(t.as_ref()),
                _ => None,
            })
            .collect(),
    )
}

/// Visit every paragraph the Paragraph panel acts on.
fn visit_paragraphs(app: &VectorcraftApp, mut f: impl FnMut(&ParaStyle)) {
    let Some(st) = app.session.active() else { return };
    if let Some(texts) = story_texts(app) {
        for t in texts {
            for i in 0..t.paragraph_count() {
                f(t.para_at(i));
            }
        }
        return;
    }
    if let Some((id, a, b)) = super::character::text_editing(app)
        && let Some(NodeKind::Text(text)) = st.doc.node(id).map(|n| &n.kind)
    {
        let range = text.paragraphs_in(a, b);
        for i in range {
            f(text.para_at(i));
        }
        return;
    }
    for t in super::character::selected_story_texts(app) {
        for i in 0..t.paragraph_count() {
            f(t.para_at(i));
        }
    }
}

/// A distance shared by every paragraph in scope, or `None` when they differ (blank spin field).
pub(crate) fn shared_para_field(app: &VectorcraftApp, field: impl Fn(&ParaStyle) -> f64) -> Option<f64> {
    let mut acc = Shared::None;
    let mut any = false;
    visit_paragraphs(app, |p| {
        any = true;
        acc.add(Metric(field(p)));
    });
    if !any {
        return text_style(app).map(|(_, p)| field(&p));
    }
    acc.value().map(|m| m.0)
}

/// A justification option shared by every paragraph in scope, or `None` when they differ.
pub(crate) fn shared_justification_f64(app: &VectorcraftApp, f: impl Fn(&JustificationOptions) -> f64) -> Option<f64> {
    let mut acc = Shared::None;
    let mut any = false;
    visit_paragraphs(app, |p| {
        any = true;
        acc.add(Metric(f(&p.justification)));
    });
    if !any {
        return text_style(app).map(|(_, p)| f(&p.justification));
    }
    acc.value().map(|m| m.0)
}

/// Single Word Justification shared by every paragraph in scope, or `None` when they differ.
pub(crate) fn shared_single_word_justify(app: &VectorcraftApp) -> Option<SingleWordJustify> {
    shared_para(app, |p| p.justification.single_word)
}

/// A paragraph attribute shared by every paragraph in scope, or `None` when they differ.
pub(crate) fn shared_para<T: PartialEq>(app: &VectorcraftApp, field: impl Fn(&ParaStyle) -> T) -> Option<T> {
    let mut acc = Shared::None;
    let mut any = false;
    visit_paragraphs(app, |p| {
        any = true;
        acc.add(field(p));
    });
    if !any {
        return text_style(app).map(|(_, p)| field(&p));
    }
    acc.value()
}

/// The composer every paragraph in scope shares, or `None` when they differ (neither menu item checked).
fn shared_composer(app: &VectorcraftApp) -> Option<Composer> {
    shared_para(app, |p| p.composer)
}

/// Hyphenation shared by every paragraph in scope, or `None` when they differ (checkbox off).
pub(crate) fn shared_hyphenate(app: &VectorcraftApp) -> Option<bool> {
    shared_para(app, |p| p.hyphenate)
}

/// Plain text of paragraph `index` in `plain` (newline-separated paragraphs).
fn paragraph_plain_at(plain: &str, index: usize) -> &str {
    let mut rest = plain;
    let mut idx = 0;
    while idx <= index && (!rest.is_empty() || idx == 0) {
        let span = vectorcraft_text::edit::paragraph_at(rest, 0);
        let part = rest.get(span.clone()).unwrap_or("");
        if idx == index {
            return part;
        }
        idx += 1;
        if span.end >= rest.len() {
            break;
        }
        rest = rest.get(span.end + 1..).unwrap_or("");
    }
    ""
}

/// Whether the Paragraph Direction buttons would highlight RTL for this paragraph.
fn displayed_paragraph_rtl(text: &vectorcraft_doc::TextObject, index: usize) -> bool {
    let plain = text.plain_text();
    let para = text.para_at(index);
    vectorcraft_text::paragraph_is_rtl(paragraph_plain_at(&plain, index), para.direction)
}

/// Collect the direction the Paragraph panel shows for each paragraph in `text`; `false` is LTR.
fn collect_text_direction(text: &vectorcraft_doc::TextObject, range: Option<(usize, usize)>, shared: &mut Option<bool>) -> bool {
    let para_range = range.map(|(a, b)| text.paragraphs_in(a, b)).unwrap_or_else(|| 0..text.paragraph_count());
    for i in para_range {
        let rtl = displayed_paragraph_rtl(text, i);
        if shared.is_some_and(|old| old != rtl) {
            return false;
        }
        *shared = Some(rtl);
    }
    true
}

/// `None` when paragraphs in scope disagree on displayed direction (neither direction button on).
fn selected_paragraph_direction(app: &VectorcraftApp, fallback: &ParaStyle) -> Option<bool> {
    let default = Some(resolved_rtl(app, fallback));
    let Some(st) = app.session.active() else { return default };
    let mut shared = None;
    if let Some(texts) = story_texts(app) {
        return if texts.into_iter().all(|t| collect_text_direction(t, None, &mut shared)) { shared.or(default) } else { None };
    }
    if let Some((id, a, b)) = super::character::text_editing(app)
        && let Some(NodeKind::Text(text)) = st.doc.node(id).map(|n| &n.kind)
    {
        return if collect_text_direction(text, Some((a, b)), &mut shared) { shared.or(default) } else { None };
    }
    let texts = super::character::selected_story_texts(app);
    if texts.into_iter().all(|t| collect_text_direction(t, None, &mut shared)) { shared.or(default) } else { None }
}

/// A hyphenation option shared by every paragraph in scope, or `None` when they differ.
pub(crate) fn shared_hyphenation_field<T: PartialEq + Copy>(app: &VectorcraftApp, f: impl Fn(&HyphenationOptions) -> T) -> Option<T> {
    shared_para(app, |p| f(&p.hyphenation))
}

/// The hyphenation zone shared by every paragraph in scope, or `None` when they differ.
pub(crate) fn shared_hyphenation_zone(app: &VectorcraftApp) -> Option<f64> {
    shared_para_field(app, |p| p.hyphenation.zone)
}

/// None means differing displayed alignments, following the other mixed-value controls.
fn selected_alignment(app: &VectorcraftApp, fallback: &ParaStyle) -> Option<Justify> {
    let default = Some(shown_alignment(fallback.justify, fallback.direction == Some(ParaDirection::RightToLeft)));
    let Some(st) = app.session.active() else { return default };
    let mut shared = None;
    if let Some(texts) = story_texts(app) {
        return if texts.into_iter().all(|t| collect_text_alignment(t, None, &mut shared)) { shared.or(default) } else { None };
    }
    if let Some((id, a, b)) = super::character::text_editing(app)
        && let Some(NodeKind::Text(text)) = st.doc.node(id).map(|n| &n.kind)
    {
        return if collect_text_alignment(text, Some((a, b)), &mut shared) { shared.or(default) } else { None };
    }
    let texts = super::character::selected_story_texts(app);
    if texts.into_iter().all(|t| collect_text_alignment(t, None, &mut shared)) { shared.or(default) } else { None }
}

/// The alignment the panel shows selected for the selected type, `None` while its paragraphs differ.
fn shown_justify(app: &VectorcraftApp) -> Option<Justify> {
    let para = text_style(app).map_or_else(ParaStyle::default, |(_, p)| p);
    selected_alignment(app, &para)
}

/// The Control bar's alignment buttons: Align left, Align center and Align right. The public
/// documentation has the Control panel format paragraphs while type is selected, with a way to the
/// Paragraph panel (<https://helpx.adobe.com/illustrator/using/formatting-paragraphs.html>), and
/// its alignment buttons align text as the Paragraph panel's do
/// (<https://helpx.adobe.com/illustrator/using/align-text.html>). The bar keeps to the three that
/// don't justify, for its room; all seven are in the Paragraph options the link opens.
const CONTROL_BAR_ALIGNMENTS: usize = 3;

/// The first `count` of the [`ALIGNMENTS`] as icon toggles, shared by the Paragraph panel's bars
/// and Properties; none is pressed when the paragraphs they act on have differing alignments.
pub(crate) fn alignment_toggles(app: &mut VectorcraftApp, ui: &mut Ui, count: usize) {
    let shown = shown_justify(app);
    for (j, icon, tip, id) in ALIGNMENTS.into_iter().take(count) {
        let pressed = ToggleButton::new(tl!(tip), Bind::given(shown == Some(j))).icon(IconId::new(icon)).icon_only().id_salt(id).show(ui);
        if pressed.value().is_some()
            && let Err(e) = align(app, id)
        {
            app.status(e);
        }
    }
}

/// The Control bar's paragraph controls while type is selected or edited (#5372), after its
/// Character controls: the Paragraph link, which opens the Paragraph panel in a popover under it,
/// and the Align left, Align center and Align right buttons. Always `true`: the bar trims these
/// itself as it narrows, the link and the buttons as two members of the type groups' rank, so the
/// buttons leave before the link.
pub(crate) fn control_bar(app: &mut VectorcraftApp, ui: &mut Ui) -> bool {
    if text_style(app).is_none() {
        return true;
    }
    Row::new().fit_rank(crate::chrome::rank::TYPE).show(ui, |ui, _| {
        let link = crate::panels::option_link(tl!("Paragraph:")).tooltip(tl!("Paragraph options")).show(ui);
        Popover::new(tl!("Paragraph"), Bind::kept())
            .overlay(|overlay| overlay.opened_by(&link.response, Trigger::Click))
            .show(ui, |ui, _| show(app, ui));
    });
    Row::new().fit_rank(crate::chrome::rank::TYPE).show(ui, |ui, _| alignment_toggles(app, ui, CONTROL_BAR_ALIGNMENTS));
    true
}

/// Type › Bullets and Numbering › Apply Bullets (`type.applyBullets {bullet?}`): the paragraphs of
/// the selected type, or those the Type tool's selection touches, become a bulleted list with
/// `bullet`; without it, a list keeps its bullet and other paragraphs take the dot.
pub(crate) fn apply_bullets(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    text_style(app).ok_or("select some type first")?;
    let params = match p.get("bullet") {
        Some(b) => json!({ "bullet": b }),
        None => json!({ "list": "bullet" }),
    };
    run_on_paragraphs(app, "text.setFormat", params)
}

/// Type › Bullets and Numbering › Apply Numbers (`type.applyNumbers {number?}`): the paragraphs of
/// the selected type, or those the Type tool's selection touches, become a numbered list with
/// number style `number`; without it, a numbered list keeps its style and other paragraphs are
/// numbered 1, 2, 3.
pub(crate) fn apply_numbers(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    text_style(app).ok_or("select some type first")?;
    let params = match p.get("number") {
        Some(n) => json!({ "number": n }),
        None => json!({ "list": "number" }),
    };
    run_on_paragraphs(app, "text.setFormat", params)
}

/// The Bullets presets, in the order they are listed: (name, `text.setFormat`'s `bullet`, bullet;
/// None for the preset that turns the list off).
pub(crate) fn bullet_presets() -> [(&'static str, &'static str, Option<Bullet>); 5] {
    [
        (tl!("None"), "none", None),
        (tl!("Filled Dot"), "dot", Some(Bullet::Dot)),
        (tl!("Dash"), "dash", Some(Bullet::Dash)),
        (tl!("Hollow Circle"), "circle", Some(Bullet::Circle)),
        (tl!("Filled Square"), "square", Some(Bullet::Square)),
    ]
}

/// The first `n` numbers of number style `style`, as the Numbered List presets show them ("1, 2,
/// 3").
pub(crate) fn number_sample(style: vectorcraft_doc::text::NumberStyle, n: u32) -> String {
    (1..=n).map(|k| style.format(k)).collect::<Vec<_>>().join(", ")
}

/// Type › Bullets and Numbering › Convert to Text (`type.convertListToText`): the markers of the
/// list paragraphs the Paragraph panel acts on ([`run_on_paragraphs`]: with the Selection tool,
/// every frame of the selected stories) become characters of the text and the lists are turned
/// off (`text.convertListToText`, which also moves the Type tool's anchor and caret past the
/// characters inserted at or before them).
pub(crate) fn convert_list_to_text(app: &mut VectorcraftApp) -> Result<Value, String> {
    text_style(app).ok_or("select some type first")?;
    run_on_paragraphs(app, "text.convertListToText", json!({}))
}

/// A bullet preset drawn as a value glyph: the shapes the Bullets presets show (a slash for None).
/// [`BulletGlyph::of`] hands out the shared `'static` glyph, as icon specs in menu entries need.
#[derive(MemoProps)]
pub(crate) struct BulletGlyph(#[memo_props(hash = "a fieldless enum: its derived `Hash` is its variant")] pub Option<Bullet>);

impl BulletGlyph {
    /// The glyph of `bullet`.
    pub(crate) fn of(bullet: Option<Bullet>) -> &'static BulletGlyph {
        match bullet {
            None => &BulletGlyph(None),
            Some(Bullet::Dot) => &BulletGlyph(Some(Bullet::Dot)),
            Some(Bullet::Dash) => &BulletGlyph(Some(Bullet::Dash)),
            Some(Bullet::Circle) => &BulletGlyph(Some(Bullet::Circle)),
            Some(Bullet::Square) => &BulletGlyph(Some(Bullet::Square)),
        }
    }
}

impl Glyph for BulletGlyph {
    fn paint(&self, painter: &Painter, tokens: &UiTokens, rect: Rect, color: Color32, _mirrored: bool) {
        let c = rect.center();
        // The shapes are drawn for a 12-point box.
        let unit = rect.width().min(rect.height()) / 12.0;
        let stroke = Stroke::new(tokens.stroke.thin, color);
        match self.0 {
            None => painter.line_segment([c + vec2(-5.0, 5.0) * unit, c + vec2(5.0, -5.0) * unit], stroke),
            Some(Bullet::Dot) => painter.circle_filled(c, 3.0 * unit, color),
            Some(Bullet::Dash) => painter.line_segment([c - vec2(4.0, 0.0) * unit, c + vec2(4.0, 0.0) * unit], stroke),
            Some(Bullet::Circle) => painter.circle_stroke(c, 3.0 * unit, stroke),
            Some(Bullet::Square) => painter.rect_filled(Rect::from_center_size(c, vec2(6.0, 6.0) * unit), 0.0, color),
        };
    }
}

/// The two list kinds of the Paragraph panel's list row.
#[derive(Clone, Copy)]
enum ListKind {
    Bullets,
    Numbers,
}

impl ListKind {
    fn name(self) -> &'static str {
        match self {
            ListKind::Bullets => tl!("Bullets"),
            ListKind::Numbers => tl!("Numbered List"),
        }
    }

    fn icon(self) -> &'static str {
        match self {
            ListKind::Bullets => "dc-para-bullets",
            ListKind::Numbers => "dc-para-numbers",
        }
    }

    fn menu_salt(self) -> &'static str {
        match self {
            ListKind::Bullets => "bullets-menu",
            ListKind::Numbers => "numbers-menu",
        }
    }

    /// The `text.setFormat` `list` value, and the Bullets and Numbering tab More Options opens.
    fn key(self) -> &'static str {
        match self {
            ListKind::Bullets => "bullet",
            ListKind::Numbers => "number",
        }
    }

    fn tab(self) -> &'static str {
        match self {
            ListKind::Bullets => "bullets",
            ListKind::Numbers => "numbers",
        }
    }
}

/// What a list control did this pass.
enum ListEvent {
    /// The toggle made the paragraphs a list of the kind.
    Toggle,
    /// A menu row ran: a preset's index or [`MORE`].
    Pick(u32),
}

/// What every paragraph in scope shares of the bullets: `Some(Some(b))` a bulleted list, `Some(None)` no list.
fn bullets_shown(app: &VectorcraftApp) -> Option<Option<Bullet>> {
    shared_para(app, |p| p.list.filter(|l| l.number.is_none()).map(|l| l.bullet))
}

/// What every paragraph in scope shares of the numbering.
fn numbers_shown(app: &VectorcraftApp) -> Option<Option<NumberStyle>> {
    shared_para(app, |p| p.list.and_then(|l| l.number))
}

/// The paragraphs the panel acts on become a list of `kind` (`text.setFormat {list}`).
fn toggle_list(app: &mut VectorcraftApp, kind: ListKind) {
    format(app, json!({ "list": kind.key() }));
}

/// A bullet preset, by its `bullet_presets` key ("none" turns the list off).
fn set_bullet(app: &mut VectorcraftApp, key: &str) {
    format(app, json!({ "bullet": key }));
}

/// A number style, or `None` to turn the list off.
fn set_number_style(app: &mut VectorcraftApp, style: Option<NumberStyle>) {
    format(app, json!({ "number": style.map_or(json!("none"), |s| json!(s)) }));
}

/// The rows after the presets: a rule and More Options.
fn more_entries(entries: &mut Vec<Entry<u32>>) {
    entries.push(Entry::separator());
    entries.push(Entry::new(MORE, tl!("More Options")));
}

/// The Bullets presets, the one every paragraph has checked.
fn bullet_entries(shown: Option<Option<Bullet>>) -> Vec<Entry<u32>> {
    let mut entries: Vec<Entry<u32>> = bullet_presets()
        .iter()
        .enumerate()
        .map(|(i, (name, _, bullet))| Entry::new(i as u32, *name).icon(IconSpec::glyph(BulletGlyph::of(*bullet))).checked(shown == Some(*bullet)))
        .collect();
    more_entries(&mut entries);
    entries
}

/// None and the number styles, each shown by its first three numbers, the one every paragraph has checked.
fn number_entries(shown: Option<Option<NumberStyle>>) -> Vec<Entry<u32>> {
    let mut entries = vec![Entry::new(0, tl!("None")).icon(IconSpec::glyph(BulletGlyph::of(None))).checked(shown == Some(None))];
    entries.extend(
        NumberStyle::ALL.iter().enumerate().map(|(i, &style)| Entry::new(i as u32 + 1, number_sample(style, 3)).checked(shown == Some(Some(style)))),
    );
    more_entries(&mut entries);
    entries
}

/// A list toggle and the menu button beside it: the toggle makes the paragraphs a list of `kind`, the menu lists `entries`.
fn list_control(ui: &mut Ui, kind: ListKind, pressed: bool, entries: &[Entry<u32>]) -> Option<ListEvent> {
    let name = kind.name();
    let toggle = ToggleButton::new(name, Bind::given(pressed)).icon(IconId::new(kind.icon())).icon_only().id_salt(kind.icon()).show(ui);
    let arrow = IconButton::new(IconId::new("chevron-down"))
        .label(name)
        .popup(PopupKind::Menu)
        .popup_mark(PopupMark::Menu)
        .id_salt(kind.menu_salt())
        .show(ui);
    let pick = Menu::new(name).values().opened_by(&arrow.response, Trigger::Press).pick(ui, entries);
    if toggle.value().is_some() { Some(ListEvent::Toggle) } else { pick.picked.map(ListEvent::Pick) }
}

/// The Bullets and Numbered List controls (Paragraph panel). Each toggle makes the paragraphs the
/// panel acts on a list and shows pressed while they all are one. Its menu lists the presets: None
/// turns the list off, and the one every paragraph has is checked. More Options after them opens
/// Bullets and Numbering on the kind's tab.
fn lists_row(app: &mut VectorcraftApp, ui: &mut Ui) {
    Row::new().show(ui, |ui, _| {
        let shown = bullets_shown(app);
        match list_control(ui, ListKind::Bullets, matches!(shown, Some(Some(_))), &bullet_entries(shown)) {
            Some(ListEvent::Toggle) => toggle_list(app, ListKind::Bullets),
            Some(ListEvent::Pick(MORE)) => more_options(app, ListKind::Bullets.tab()),
            Some(ListEvent::Pick(i)) => {
                let presets = bullet_presets();
                if let Some(&(_, key, _)) = presets.get(i as usize) {
                    set_bullet(app, key);
                }
            }
            None => {}
        }
        let shown = numbers_shown(app);
        match list_control(ui, ListKind::Numbers, matches!(shown, Some(Some(_))), &number_entries(shown)) {
            Some(ListEvent::Toggle) => toggle_list(app, ListKind::Numbers),
            Some(ListEvent::Pick(MORE)) => more_options(app, ListKind::Numbers.tab()),
            Some(ListEvent::Pick(0)) => set_number_style(app, None),
            Some(ListEvent::Pick(i)) => {
                if let Some(&style) = (i as usize).checked_sub(1).and_then(|k| NumberStyle::ALL.get(k)) {
                    set_number_style(app, Some(style));
                }
            }
            None => {}
        }
    });
}

/// More Options: open Bullets and Numbering on `tab` (`ui.bulletsAndNumbering`), or show why it
/// can't open.
fn more_options(app: &mut VectorcraftApp, tab: &str) {
    if let Err(e) = app.run("ui.bulletsAndNumbering", json!({ "tab": tab })) {
        app.status(e);
    }
}

/// The Hyphenate checkbox for the paragraphs the panel acts on.
pub(crate) fn hyphenate_check(app: &mut VectorcraftApp, ui: &mut Ui) {
    let hyphenate = shared_hyphenate(app);
    let shown = hyphenate.map_or(Mixed::Mixed, Mixed::Same);
    if Checkbox::new(Bind::given(shown)).label(tl!("Hyphenate")).show(ui).events.landed().is_some() {
        format(app, json!({"hyphenate": hyphenate != Some(true)}));
    }
}

/// The paragraph attributes the panel's controls show, for the selected type (default values with none).
pub(crate) fn values(app: &VectorcraftApp) -> ParagraphValues {
    let mut values = ParagraphValues::default();
    let Some((_, para)) = text_style(app) else { return values };
    let points = |field: fn(&ParaStyle) -> f64| shared_para_field(app, field).map(|v| v as f32);
    values.align = selected_alignment(app, &para).and_then(|j| ALIGNMENTS.iter().position(|a| a.0 == j)).and_then(|i| TEXT_ALIGNS.get(i).copied());
    values.direction =
        selected_paragraph_direction(app, &para).map(|rtl| if rtl { ParagraphDirection::RightToLeft } else { ParagraphDirection::LeftToRight });
    // The framework's start and end indents are the app's absolute left and right ones.
    values.start_indent = points(|p| p.left_indent);
    values.end_indent = points(|p| p.right_indent);
    values.first_line_indent = points(|p| p.first_line_indent);
    values.space_before = points(|p| p.space_before);
    values.space_after = points(|p| p.space_after);
    values.hyphenate = shared_hyphenate(app);
    values
}

/// One change of the panel's controls, on the paragraphs the panel acts on.
fn apply(app: &mut VectorcraftApp, change: &ParagraphChange) {
    let length = |key: &str, v: f32| {
        let mut p = json!({});
        p[key] = json!(f64::from(v));
        p
    };
    match *change {
        ParagraphChange::Align(a) => {
            let row = TEXT_ALIGNS.iter().position(|&t| t == a).and_then(|i| ALIGNMENTS.get(i));
            if let Some(&(_, _, _, id)) = row
                && let Err(e) = align(app, id)
            {
                app.status(e);
            }
        }
        ParagraphChange::Direction(d) => {
            let id = match d {
                ParagraphDirection::LeftToRight => "leftToRight",
                ParagraphDirection::RightToLeft => "rightToLeft",
                _ => "auto",
            };
            format(app, json!({ "direction": id }));
        }
        ParagraphChange::StartIndent(v) => format(app, length("leftIndent", v)),
        ParagraphChange::EndIndent(v) => format(app, length("rightIndent", v)),
        ParagraphChange::FirstLineIndent(v) => format(app, length("firstLineIndent", v)),
        ParagraphChange::SpaceBefore(v) => format(app, length("spaceBefore", v)),
        ParagraphChange::SpaceAfter(v) => format(app, length("spaceAfter", v)),
        ParagraphChange::Hyphenate(on) => format(app, json!({ "hyphenate": on })),
        _ => {}
    }
}

/// A labelled select whose entries are numbered from 0; the committed pick, if any, this pass.
/// `current` is blank when `None`. Browsing a popup must not apply paragraph formatting.
fn pick_row(ui: &mut Ui, label: &'static str, salt: &'static str, entries: &[Entry<u32>], current: Option<u32>) -> Option<u32> {
    let mut picked = None;
    Field::new(label).id_salt(salt).show(ui, |ui, _| {
        let shown = current.map_or(Mixed::Mixed, Mixed::Same);
        picked = Select::new(label, Bind::given(shown), entries).id_salt(salt).show(ui).events.committed().copied();
    });
    picked
}

/// How long the kashidas are that justify Arabic lines, with the Middle Eastern (Indic) options.
fn kashida_row(app: &mut VectorcraftApp, ui: &mut Ui, para: &ParaStyle) {
    let kinds = [
        (tl!("None"), Kashida::None, "none"),
        (tl!("Short"), Kashida::Short, "short"),
        (tl!("Medium"), Kashida::Medium, "medium"),
        (tl!("Long"), Kashida::Long, "long"),
        (tl!("Stylistic"), Kashida::Stylistic, "stylistic"),
    ];
    let entries: Vec<Entry<u32>> = kinds.iter().enumerate().map(|(i, k)| Entry::new(i as u32, k.0)).collect();
    let current = kinds.iter().position(|k| k.1 == para.kashida).map(|i| i as u32);
    if let Some(i) = pick_row(ui, tl!("Insert Kashidas"), "pa-kashida", &entries, current)
        && let Some((_, _, key)) = kinds.get(i as usize)
    {
        format(app, json!({"kashida": key}));
    }
}

/// Japanese composition: how punctuation is spaced (JLREQ 3.1), with the East Asian options. Blank while the paragraphs' sets differ.
fn mojikumi_row(app: &mut VectorcraftApp, ui: &mut Ui) {
    let sets = mojikumi_sets();
    let entries: Vec<Entry<u32>> = sets.iter().enumerate().map(|(i, s)| Entry::new(i as u32, s.0)).collect();
    let current = shared_para(app, |p| p.mojikumi).and_then(|m| sets.iter().position(|s| s.1 == m)).map(|i| i as u32);
    if let Some(i) = pick_row(ui, tl!("Mojikumi Set"), "pa-mojikumi", &entries, current)
        && let Some((_, _, key)) = sets.get(i as usize)
    {
        format(app, json!({"mojikumi": key}));
    }
}

/// Which characters may not start or end a line, with the East Asian options: None, Hard, Soft, the
/// document's custom sets, then the dialog that edits them.
fn kinsoku_row(app: &mut VectorcraftApp, ui: &mut Ui) {
    let builtin = [(tl!("None"), "None", "none"), (tl!("Hard"), "Hard", "hard"), (tl!("Soft"), "Soft", "soft")];
    let custom: Vec<String> = app.session.active().map(|st| st.doc.kinsoku_sets.iter().map(|k| k.name.clone()).collect()).unwrap_or_default();
    let mut entries: Vec<Entry<u32>> = builtin.iter().enumerate().map(|(i, s)| Entry::new(i as u32, s.0)).collect();
    entries.extend(custom.iter().enumerate().map(|(i, name)| Entry::new((builtin.len() + i) as u32, name.clone())));
    let settings = entries.len() as u32;
    // A step over the closed field skips the dialog's row: it sets no value.
    entries.push(Entry::new(settings, tl!("Kinsoku Shori Settings…")).stepped(false));
    let current = shared_kinsoku_name(app)
        .and_then(|n| builtin.iter().position(|s| s.1 == n).or_else(|| custom.iter().position(|c| *c == n).map(|i| builtin.len() + i)))
        .map(|i| i as u32);
    let Some(i) = pick_row(ui, tl!("Kinsoku Set"), "pa-kinsoku", &entries, current) else { return };
    if let Some((_, _, key)) = builtin.get(i as usize) {
        format(app, json!({"kinsoku": key}));
    } else if let Some(name) = (i as usize).checked_sub(builtin.len()).and_then(|k| custom.get(k)) {
        format(app, json!({"kinsoku": name}));
    } else if i == settings {
        crate::menus::invoke(app, crate::dialogs::kinsoku_sets::COMMAND, json!({}));
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some((_, para)) = text_style(app) else {
        super::empty_state(ui, "pilcrow", tl!("No text selected"), tl!("Select a text object to edit its paragraph attributes."));
        return;
    };
    let hidden = app.ui.paragraph_hide_options;
    let shown = values(app);
    // Paragraph direction, with the Middle Eastern (Indic) options.
    let direction = if app.session.prefs.show_indic_options { DirectionRow::Explicit } else { DirectionRow::Hidden };
    // Indents and paragraph spacing are distances (General); type sizes follow Units ▸ Type.
    let out = ParagraphControls::new(tl!("Paragraph"), &shown)
        .unit(doc_unit(app.session.general_unit()))
        .hyphenate(!hidden)
        .direction_row(direction)
        .show_with(ui, |ui| lists_row(app, ui));
    for event in &out.edits {
        edits::apply_event(app, event, &EditPath::Scope, &mut |app, change, _| apply(app, change));
    }
    if hidden {
        return;
    }
    if app.session.prefs.show_indic_options {
        kashida_row(app, ui, &para);
    }
    if app.session.prefs.show_east_asian_options {
        mojikumi_row(app, ui);
        kinsoku_row(app, ui);
    }
}

/// The panel menu's row that sets `key` to `value` (`ui.paragraph.set`).
fn set_ref(key: &str, value: &str) -> CommandRef {
    CommandRef::with_params("ui.paragraph.set", json!({ "key": key, "value": value }))
}

/// The panel menu's row that toggles `key` (`ui.paragraph.toggle`).
fn toggle_ref(key: &str) -> CommandRef {
    CommandRef::with_params("ui.paragraph.toggle", json!({ "key": key }))
}

/// Show or Hide Options.
fn show_ref() -> CommandRef {
    CommandRef::with_params("ui.paragraph.show", json!({ "option": "options" }))
}

/// Reset Panel.
fn reset_ref() -> CommandRef {
    CommandRef::new("ui.paragraph.reset")
}

/// The Burasagari rows: label, value, `text.setFormat` key.
fn burasagari_rows() -> [(&'static str, Burasagari, &'static str); 3] {
    [(tl!("None"), Burasagari::None, "none"), (tl!("Regular"), Burasagari::Standard, "standard"), (tl!("Force"), Burasagari::Forced, "forced")]
}

/// The Kinsoku Shori Type rows: label, value, `text.setFormat` key.
fn kinsoku_types() -> [(&'static str, KinsokuShoriType, &'static str); 3] {
    [
        (tl!("Push In First"), KinsokuShoriType::PushInFirst, "pushInFirst"),
        (tl!("Push Out First"), KinsokuShoriType::PushOutFirst, "pushOutFirst"),
        (tl!("Push Out Only"), KinsokuShoriType::PushOutOnly, "pushOutOnly"),
    ]
}

/// The four composers, as the menu lists them.
fn composers() -> [(&'static str, Composer); 4] {
    [
        (tl!("Middle Eastern & South Asian Single-line Composer"), Composer::SingleLine),
        (tl!("Middle Eastern & South Asian Every-line Composer"), Composer::EveryLine),
        (tl!("Standard Single-line Composer"), Composer::StandardSingleLine),
        (tl!("Standard Every-line Composer"), Composer::StandardEveryLine),
    ]
}

/// The Paragraph panel menu's rows; the labels and chords come from the commands' catalogue entries.
pub(crate) fn menu_rows(app: &VectorcraftApp) -> Vec<PanelRow> {
    let cmd = |r: CommandRef| PanelRow::Command(r);
    let mut rows = vec![
        cmd(show_ref()),
        PanelRow::Separator,
        cmd(toggle_ref("romanHangingPunctuation")),
        cmd(CommandRef::new("ui.justification")),
        cmd(CommandRef::new("ui.hyphenation")),
    ];
    if app.session.prefs.show_east_asian_options {
        rows.push(PanelRow::Separator);
        rows.push(cmd(set_ref("leadingModel", "emBoxTop")));
        rows.push(cmd(set_ref("leadingModel", "romanBaseline")));
        rows.push(PanelRow::Submenu(tl!("Burasagari").into(), burasagari_rows().iter().map(|r| cmd(set_ref("burasagari", r.2))).collect()));
        rows.push(cmd(toggle_ref("kurikaeshiMojiShori")));
        rows.push(PanelRow::Submenu(tl!("Kinsoku Shori Type").into(), kinsoku_types().iter().map(|r| cmd(set_ref("kinsokuType", r.2))).collect()));
        rows.push(cmd(toggle_ref("bunriKinshi")));
        rows.push(cmd(toggle_ref("koreanLineBreakAnywhere")));
    }
    rows.push(PanelRow::Separator);
    rows.extend(composers().iter().map(|(_, c)| cmd(set_ref("composer", composer_id(*c)))));
    rows.push(PanelRow::Separator);
    rows.push(cmd(reset_ref()));
    rows
}

/// A menu row's state: enabled, and checked when `checked` has a value.
fn state(enabled: bool, checked: Option<bool>) -> CommandState {
    let state = CommandState::new().enabled(enabled);
    match checked {
        Some(on) => state.checked(on),
        None => state,
    }
}

/// The enabled and checked state of the rows of [`menu_rows`]: all but the Options one need type selected.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    let style = text_style(app);
    let has = style.is_some();
    let title = if app.ui.paragraph_hide_options { tl!("Show Options") } else { tl!("Hide Options") };
    states.set(show_ref(), CommandState::new().title(title));
    let mut put = |r: CommandRef, enabled: bool, checked: Option<bool>| states.set(r, state(enabled, checked));
    // Quotation marks, hyphens, full stops and commas at a line's ends hang outside the margin.
    put(toggle_ref("romanHangingPunctuation"), has, Some(style.as_ref().is_some_and(|(_, p)| p.roman_hanging)));
    put(CommandRef::new("ui.justification"), has, None);
    put(CommandRef::new("ui.hyphenation"), has, None);
    if app.session.prefs.show_east_asian_options {
        let top_to_top = style.as_ref().is_some_and(|(_, p)| p.leading_model == LeadingModel::EmBoxTop);
        put(set_ref("leadingModel", "emBoxTop"), has, Some(top_to_top));
        put(set_ref("leadingModel", "romanBaseline"), has, Some(has && !top_to_top));
        let hang = style.as_ref().map(|(_, p)| p.burasagari);
        for (_, b, key) in burasagari_rows() {
            put(set_ref("burasagari", key), has, Some(hang == Some(b)));
        }
        put(toggle_ref("kurikaeshiMojiShori"), has, Some(shared_para(app, |p| p.kurikaeshi_moji_shori) == Some(true)));
        let kinsoku_type = shared_para(app, |p| p.kinsoku_type);
        for (_, t, key) in kinsoku_types() {
            put(set_ref("kinsokuType", key), has, Some(kinsoku_type == Some(t)));
        }
        // Inseparable pairs stay on one line; only with a kinsoku set.
        let (kinsoku_on, bunri) = bunri_kinshi_item(app);
        put(toggle_ref("bunriKinshi"), has && kinsoku_on, Some(bunri));
        put(toggle_ref("koreanLineBreakAnywhere"), has, Some(shared_para(app, |p| p.korean_line_break_anywhere) == Some(true)));
    }
    let composer = shared_composer(app);
    for (_, c) in composers() {
        put(set_ref("composer", composer_id(c)), has, Some(composer == Some(c)));
    }
    put(reset_ref(), has, None);
}

/// Runs a `ui.paragraph.*` command; `None` for any other id.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let command = id.strip_prefix("ui.paragraph.")?;
    let key = params.get("key").and_then(Value::as_str);
    let value = params.get("value").and_then(Value::as_str);
    Some(match command {
        "show" => {
            app.ui.paragraph_hide_options = !app.ui.paragraph_hide_options;
            Ok(Value::Null)
        }
        "reset" => text_style(app).ok_or_else(|| "select some type first".to_string()).map(|_| reset_panel(app)).map(|()| Value::Null),
        "toggle" => text_style(app).ok_or_else(|| "select some type first".to_string()).and_then(|(_, para)| toggle_row(app, key, &para)),
        "set" => text_style(app).ok_or_else(|| "select some type first".to_string()).and_then(|_| set_row(app, key, value)),
        _ => Err(format!("unknown paragraph command {id}")),
    })
}

/// A `ui.paragraph.toggle` row.
fn toggle_row(app: &mut VectorcraftApp, key: Option<&str>, para: &ParaStyle) -> Result<Value, String> {
    match key {
        Some("romanHangingPunctuation") => format(app, json!({"romanHangingPunctuation": !para.roman_hanging})),
        Some("kurikaeshiMojiShori") => toggle_kurikaeshi_moji_shori(app),
        Some("bunriKinshi") => toggle_bunri_kinshi(app),
        Some("koreanLineBreakAnywhere") => toggle_korean_line_break_anywhere(app),
        _ => return Err(format!("unknown paragraph option {key:?}")),
    }
    Ok(Value::Null)
}

/// A `ui.paragraph.set` row.
fn set_row(app: &mut VectorcraftApp, key: Option<&str>, value: Option<&str>) -> Result<Value, String> {
    let value = value.ok_or("missing value")?;
    match key {
        Some("kinsokuType") => set_kinsoku_type(app, value),
        Some(key @ ("leadingModel" | "burasagari" | "composer")) => {
            let mut p = json!({});
            p[key] = json!(value);
            format(app, p);
        }
        _ => return Err(format!("unknown paragraph option {key:?}")),
    }
    Ok(Value::Null)
}

/// Kinsoku Shori Type (`key`: `"pushInFirst"`, `"pushOutFirst"` or `"pushOutOnly"`) for the
/// paragraphs the panel acts on ([`format`]), in one undo step.
fn set_kinsoku_type(app: &mut VectorcraftApp, key: &str) {
    format(app, json!({"kinsokuType": key}));
}

/// The Mojikumi Set menu's entries, translated: each one's label, its set and its `mojikumi` value
/// for `text.setFormat`. The five predefined sets come first, then Line-end Punctuation Half Width,
/// the set new type gets.
fn mojikumi_sets() -> [(&'static str, Mojikumi, &'static str); 6] {
    [
        (tl!("None"), Mojikumi::None, "none"),
        (tl!("YakumonoHankaku"), Mojikumi::YakumonoHankaku, "yakumonoHankaku"),
        (tl!("GyomatsuYakumonoHankaku"), Mojikumi::GyomatsuYakumonoHankaku, "gyomatsuYakumonoHankaku"),
        (tl!("GyomatsuYakumonoZenkaku"), Mojikumi::GyomatsuYakumonoZenkaku, "gyomatsuYakumonoZenkaku"),
        (tl!("YakumonoZenkaku"), Mojikumi::YakumonoZenkaku, "yakumonoZenkaku"),
        (tl!("Line-end Punctuation Half Width"), Mojikumi::LineEndHalf, "lineEndHalf"),
    ]
}

/// Bunri-Kinshi's menu item for the paragraphs the panel acts on ([`format`]): (available, checked).
/// It is available when none of them has kinsoku None, and checked when every one of them has it.
fn bunri_kinshi_item(app: &VectorcraftApp) -> (bool, bool) {
    (shared_para(app, ParaStyle::kinsoku_on) == Some(true), shared_para(app, |p| p.bunri_kinshi) == Some(true))
}

/// The Kinsoku set every paragraph the panel acts on ([`format`]) has, by name (None, Hard, Soft or
/// a custom set's), or `None` when they differ.
fn shared_kinsoku_name(app: &VectorcraftApp) -> Option<String> {
    let mut first: Option<String> = None;
    let mut differ = false;
    visit_paragraphs(app, |p| match &first {
        None => first = Some(p.kinsoku_name().to_string()),
        Some(n) => differ |= n != p.kinsoku_name(),
    });
    match first {
        None => text_style(app).map(|(_, p)| p.kinsoku_name().to_string()),
        Some(_) if differ => None,
        n => n,
    }
}

/// Bunri-Kinshi (Paragraph panel menu): a click turns it off for all the paragraphs the panel acts
/// on ([`format`]) when every one of them has it, and on for all of them otherwise, in one undo step.
fn toggle_bunri_kinshi(app: &mut VectorcraftApp) {
    let on = shared_para(app, |p| p.bunri_kinshi) == Some(true);
    format(app, json!({"bunriKinshi": !on}));
}

/// Kurikaeshi Moji Shori (Paragraph panel menu): the menu item is checked when every paragraph the
/// panel acts on ([`format`]) has it, and a click turns it off for all of them then, and on for all
/// of them otherwise, in one undo step.
fn toggle_kurikaeshi_moji_shori(app: &mut VectorcraftApp) {
    let on = shared_para(app, |p| p.kurikaeshi_moji_shori) == Some(true);
    format(app, json!({"kurikaeshiMojiShori": !on}));
}

/// Korean Line Break Anywhere (Paragraph panel menu): the menu item is checked when every paragraph
/// the panel acts on ([`format`]) has it, and a click turns it off for all of them then, and on for
/// all of them otherwise, in one undo step.
fn toggle_korean_line_break_anywhere(app: &mut VectorcraftApp) {
    let on = shared_para(app, |p| p.korean_line_break_anywhere) == Some(true);
    format(app, json!({"koreanLineBreakAnywhere": !on}));
}

/// Reset Panel: the paragraphs the panel acts on ([`format`]) take Auto alignment, no list, no
/// indents or paragraph spacing, Hyphenate off with the default Hyphenation and Justification
/// options, Auto direction, Bottom-to-Bottom Leading, the Line-end Punctuation Half Width Mojikumi
/// Set, Regular burasagari, the Hard kinsoku set, Push Out Only, the Every-line Composer, Bunri-Kinshi,
/// Kurikaeshi Moji Shori, Roman Hanging Punctuation and Korean Line Break Anywhere off, no kashidas and no tab stops, as one undo
/// step named Reset Panel (inside an undo group that is already open, as part of its step).
fn reset_panel(app: &mut VectorcraftApp) {
    let h = HyphenationOptions::default();
    let j = vectorcraft_doc::JustificationOptions::default();
    // The Type tool's typing session stays an undo step of its own.
    super::character::end_typing(app);
    let open = app.session.active().is_some_and(|st| st.undo_group.is_some());
    if !open {
        app.session.begin_named_undo_group(Some("Reset Panel".into()));
    }
    para_cmd(app, "text.setStyle", json!({"justify": "auto"}));
    format(
        app,
        json!({
            "list": "none", "leftIndent": 0, "rightIndent": 0, "firstLineIndent": 0, "spaceBefore": 0, "spaceAfter": 0, "hyphenate": false,
            "wordsLongerThan": h.words_longer_than, "afterFirst": h.after_first, "beforeLast": h.before_last,
            "hyphenLimit": h.hyphen_limit, "hyphenationZone": h.zone, "hyphenateCapitalized": h.capitalized, "kashida": "none",
            "wordSpacing": j.word_spacing, "letterSpacing": j.letter_spacing, "glyphScaling": j.glyph_scaling,
            "autoLeading": j.auto_leading, "singleWordJustification": "fullJustify",
            "direction": "auto", "leadingModel": "romanBaseline", "mojikumi": "lineEndHalf", "burasagari": "standard", "kinsoku": "hard",
            "kinsokuType": "pushOutOnly", "composer": "everyLine", "romanHangingPunctuation": false, "koreanLineBreakAnywhere": false,
            "bunriKinshi": false, "kurikaeshiMojiShori": false
        }),
    );
    para_cmd(app, "text.tabs.clear", json!({}));
    // Closes the active document's undo group only, so an undo group open in another document
    // stays open.
    if !open && let Some(st) = app.session.active_mut() {
        st.undo_group = None;
    }
}

#[cfg(test)]
mod tests;
