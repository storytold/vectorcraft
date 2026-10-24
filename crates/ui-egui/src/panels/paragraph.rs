//! Paragraph panel: seven alignment buttons, Paragraph Direction (with the Indic options), the
//! Bullets and Numbered List buttons and their presets, indents, space before/after, Hyphenate,
//! Insert Kashidas (with the Indic options), Mojikumi Set and Kinsoku Set (with the East Asian
//! options); the panel menu turns Roman Hanging Punctuation on and off, opens Justification… and
//! Hyphenation… and picks the Single-line or Every-line Composer.

use egui::{Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::{
    Composer, HyphenationOptions, JustificationOptions, Justify, Kinsoku, Mojikumi, NodeKind, ParaDirection, ParaStyle, SingleWordJustify,
};

use super::character::{Metric, Shared, text_style};
use super::{pstate, set_pstate};
use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets::{self, menu_item};

pub const ALIGNMENTS: [(Justify, &str, &str, &str); 7] = [
    (Justify::Left, "dc-para-left", "Align left", "left"),
    (Justify::Center, "dc-para-center", "Align center", "center"),
    (Justify::Right, "dc-para-right", "Align right", "right"),
    (Justify::JustifyLeft, "dc-para-justify-left", "Justify with last line aligned left", "justifyLeft"),
    (Justify::JustifyCenter, "dc-para-justify-center", "Justify with last line aligned center", "justifyCenter"),
    (Justify::JustifyRight, "dc-para-justify-right", "Justify with last line aligned right", "justifyRight"),
    (Justify::JustifyAll, "dc-para-justify-all", "Justify all lines", "justifyAll"),
];

/// Paragraph attributes apply to the selected text objects' paragraphs or, while the Type tool
/// edits text, to the paragraphs its selection (or caret) touches (ending its typing session).
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

fn para_cmd(app: &mut VectorcraftApp, cmd: &str, p: Value) {
    run_on_paragraphs(app, cmd, p).ok();
}

/// Run `cmd` on the paragraphs the Paragraph panel acts on ([`format`]) and return its result.
pub(crate) fn run_on_paragraphs(app: &mut VectorcraftApp, cmd: &str, mut p: Value) -> Result<Value, String> {
    if let Some((id, a, b)) = super::character::text_editing(app) {
        super::character::end_typing(app);
        p["ids"] = json!([id.0]);
        p["start"] = json!(a);
        p["end"] = json!(b);
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

/// Visit every paragraph the Paragraph panel acts on.
fn visit_paragraphs(app: &VectorcraftApp, mut f: impl FnMut(&ParaStyle)) {
    let Some(st) = app.session.active() else { return };
    if let Some((id, a, b)) = super::character::text_editing(app)
        && let Some(NodeKind::Text(text)) = st.doc.node(id).map(|n| &n.kind)
    {
        let range = text.paragraphs_in(a, b);
        for i in range {
            f(text.para_at(i));
        }
        return;
    }
    for node in st.selection.objects.iter().filter_map(|id| st.doc.node(*id)) {
        node.walk(&mut |c| {
            if let NodeKind::Text(t) = &c.kind {
                for i in 0..t.paragraph_count() {
                    f(t.para_at(i));
                }
            }
        });
    }
}

/// A distance shared by every paragraph in scope, or `None` when they differ (blank spin field).
fn shared_para_field(app: &VectorcraftApp, field: impl Fn(&ParaStyle) -> f64) -> Option<f64> {
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
fn shared_para<T: PartialEq + Copy>(app: &VectorcraftApp, field: impl Fn(&ParaStyle) -> T) -> Option<T> {
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
    if let Some((id, a, b)) = super::character::text_editing(app)
        && let Some(NodeKind::Text(text)) = st.doc.node(id).map(|n| &n.kind)
    {
        return if collect_text_direction(text, Some((a, b)), &mut shared) { shared.or(default) } else { None };
    }
    fn collect(node: &vectorcraft_doc::Node, shared: &mut Option<bool>) -> bool {
        if let NodeKind::Text(text) = &node.kind {
            return collect_text_direction(text, None, shared);
        }
        node.children().is_none_or(|children| children.iter().all(|child| collect(child, shared)))
    }
    for node in st.selection.objects.iter().filter_map(|id| st.doc.node(*id)) {
        if !collect(node, &mut shared) {
            return None;
        }
    }
    shared.or(default)
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
    if let Some((id, a, b)) = super::character::text_editing(app)
        && let Some(NodeKind::Text(text)) = st.doc.node(id).map(|n| &n.kind)
    {
        return if collect_text_alignment(text, Some((a, b)), &mut shared) { shared.or(default) } else { None };
    }
    fn collect(node: &vectorcraft_doc::Node, shared: &mut Option<Justify>) -> bool {
        if let NodeKind::Text(text) = &node.kind {
            return collect_text_alignment(text, None, shared);
        }
        node.children().is_none_or(|children| children.iter().all(|child| collect(child, shared)))
    }
    for node in st.selection.objects.iter().filter_map(|id| st.doc.node(*id)) {
        if !collect(node, &mut shared) {
            return None;
        }
    }
    shared.or(default)
}

/// The seven alignment buttons shared by Paragraph and Properties; no button is selected
/// when the paragraphs they act on have differing displayed alignments.
pub fn alignment_buttons(app: &mut VectorcraftApp, ui: &mut Ui, para: &ParaStyle, size: f32) {
    let shown = selected_alignment(app, para);
    for (j, icon, tip, id) in ALIGNMENTS {
        if widgets::icon_button(ui, icon, tip, shown == Some(j), size).clicked() {
            para_cmd(app, "text.setStyle", json!({"justify": id}));
        }
    }
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

/// The Bullets button and the arrow beside it (Paragraph panel). The button makes the paragraphs
/// the panel acts on a bulleted list (`text.setFormat {list: "bullet"}`) and shows pressed while
/// they all are one. The arrow lists the presets (`{bullet}`): None turns the list off, and the
/// one every paragraph has is checked.
pub fn bullets_button(app: &mut VectorcraftApp, ui: &mut Ui, size: f32) {
    use vectorcraft_doc::text::Bullet;
    let t = Tokens::get(ui.ctx());
    let shown = shared_para(app, |p| p.list.filter(|l| l.number.is_none()).map(|l| l.bullet));
    if widgets::icon_button(ui, "dc-para-bullets", tl!("Bullets"), matches!(shown, Some(Some(_))), size).clicked() {
        format(app, json!({"list": "bullet"}));
    }
    let (rect, arrow) = ui.allocate_exact_size(vec2(14.0, size), egui::Sense::click());
    if arrow.hovered() {
        ui.painter().rect_filled(rect, egui::CornerRadius::same(4), t.hover);
    }
    crate::icons::paint(ui, "chevron-down", egui::Rect::from_center_size(rect.center(), vec2(10.0, 10.0)), t.text_dim);
    arrow.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tl!("Bullets")));
    let arrow = arrow.on_hover_text(tl!("Bullets"));
    let presets = [
        (tl!("None"), "none", None),
        (tl!("Filled Dot"), "dot", Some(Bullet::Dot)),
        (tl!("Dash"), "dash", Some(Bullet::Dash)),
        (tl!("Hollow Circle"), "circle", Some(Bullet::Circle)),
        (tl!("Filled Square"), "square", Some(Bullet::Square)),
    ];
    let picked = egui::Popup::menu(&arrow).show(|ui| {
        let mut picked = None;
        for (name, key, bullet) in presets {
            if bullet_row(ui, name, bullet, shown == Some(bullet)) {
                picked = Some(key);
            }
        }
        picked
    });
    if let Some(key) = picked.and_then(|r| r.inner) {
        format(app, json!({ "bullet": key }));
    }
}

/// A row of the Bullets presets: a check when `checked`, the bullet drawn (a slash for None), then
/// its name. Whether it was clicked.
fn bullet_row(ui: &mut Ui, name: &str, bullet: Option<vectorcraft_doc::text::Bullet>, checked: bool) -> bool {
    use vectorcraft_doc::text::Bullet;
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(130.0, 22.0), egui::Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, egui::CornerRadius::same(4), t.hover);
    }
    let painter = ui.painter();
    let font = egui::FontId::proportional(12.5);
    if checked {
        painter.text(rect.left_center() + vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, "✓", font.clone(), t.text_strong);
    }
    let c = rect.left_center() + vec2(28.0, 0.0);
    let stroke = egui::Stroke::new(1.5, t.text_strong);
    match bullet {
        None => painter.line_segment([c + vec2(-5.0, 5.0), c + vec2(5.0, -5.0)], stroke),
        Some(Bullet::Dot) => painter.circle_filled(c, 3.0, t.text_strong),
        Some(Bullet::Dash) => painter.line_segment([c - vec2(4.0, 0.0), c + vec2(4.0, 0.0)], stroke),
        Some(Bullet::Circle) => painter.circle_stroke(c, 3.0, egui::Stroke::new(1.2, t.text_strong)),
        Some(Bullet::Square) => painter.rect_filled(egui::Rect::from_center_size(c, vec2(6.0, 6.0)), 0.0, t.text_strong),
    };
    painter.text(rect.left_center() + vec2(42.0, 0.0), egui::Align2::LEFT_CENTER, name, font, t.text_strong);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, ui.is_enabled(), checked, name));
    resp.clicked()
}

/// The Numbered List button and the arrow beside it (Paragraph panel). The button makes the
/// paragraphs the panel acts on a numbered list (`text.setFormat {list: "number"}`) and shows
/// pressed while they all are one. The arrow lists the presets (`{number}`), each shown by its
/// first three numbers: None turns the list off, and the one every paragraph has is checked.
pub fn numbers_button(app: &mut VectorcraftApp, ui: &mut Ui, size: f32) {
    use vectorcraft_doc::text::NumberStyle;
    let t = Tokens::get(ui.ctx());
    let shown = shared_para(app, |p| p.list.and_then(|l| l.number));
    if widgets::icon_button(ui, "dc-para-numbers", tl!("Numbered List"), matches!(shown, Some(Some(_))), size).clicked() {
        format(app, json!({"list": "number"}));
    }
    let (rect, arrow) = ui.allocate_exact_size(vec2(14.0, size), egui::Sense::click());
    if arrow.hovered() {
        ui.painter().rect_filled(rect, egui::CornerRadius::same(4), t.hover);
    }
    crate::icons::paint(ui, "chevron-down", egui::Rect::from_center_size(rect.center(), vec2(10.0, 10.0)), t.text_dim);
    arrow.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tl!("Numbered List")));
    let arrow = arrow.on_hover_text(tl!("Numbered List"));
    let picked = egui::Popup::menu(&arrow).show(|ui| {
        let mut picked = None;
        if bullet_row(ui, tl!("None"), None, shown == Some(None)) {
            picked = Some(None);
        }
        for style in NumberStyle::ALL {
            let sample = (1..=3).map(|n| style.format(n)).collect::<Vec<_>>().join(", ");
            if number_row(ui, &sample, shown == Some(Some(style))) {
                picked = Some(Some(style));
            }
        }
        picked
    });
    if let Some(style) = picked.and_then(|r| r.inner) {
        format(app, json!({ "number": style.map_or(json!("none"), |s| json!(s)) }));
    }
}

/// A row of the Numbered List presets: a check when `checked`, then the style's first numbers
/// (`sample`). Whether it was clicked.
fn number_row(ui: &mut Ui, sample: &str, checked: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(130.0, 22.0), egui::Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, egui::CornerRadius::same(4), t.hover);
    }
    let painter = ui.painter();
    let font = egui::FontId::proportional(12.5);
    if checked {
        painter.text(rect.left_center() + vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, "✓", font.clone(), t.text_strong);
    }
    painter.text(rect.left_center() + vec2(22.0, 0.0), egui::Align2::LEFT_CENTER, sample, font, t.text_strong);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, ui.is_enabled(), checked, sample));
    resp.clicked()
}

/// The Hyphenate checkbox for the paragraphs the panel acts on.
pub fn hyphenate_check(app: &mut VectorcraftApp, ui: &mut Ui, _para: &ParaStyle) {
    let hyphenate = shared_hyphenate(app);
    if widgets::check(ui, tl!("Hyphenate"), hyphenate == Some(true), true) {
        format(app, json!({"hyphenate": hyphenate != Some(true)}));
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Some((_, para)) = text_style(app) else {
        super::empty_state(ui, "pilcrow", tl!("No text selected"), tl!("Select a text object to edit its paragraph attributes."));
        return;
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        alignment_buttons(app, ui, &para, 28.0);
        // Paragraph direction, with the Middle Eastern (Indic) options.
        if app.session.prefs.show_indic_options {
            ui.add_space(6.0);
            let direction = selected_paragraph_direction(app, &para);
            for (to_rtl, icon, tip, id) in [
                (false, "dc-para-ltr", tl!("Left-to-Right Paragraph Direction"), "leftToRight"),
                (true, "dc-para-rtl", tl!("Right-to-Left Paragraph Direction"), "rightToLeft"),
            ] {
                if widgets::icon_button(ui, icon, tip, direction == Some(to_rtl), 28.0).clicked() {
                    format(app, json!({"direction": id}));
                }
            }
        }
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        bullets_button(app, ui, 28.0);
        ui.add_space(6.0);
        numbers_button(app, ui, 28.0);
    });
    ui.add_space(4.0);
    let fw = ((ui.available_width() - 66.0) / 2.0).clamp(60.0, 100.0);
    // Indents and paragraph spacing are distances (General); type sizes follow Units ▸ Type.
    let unit = app.session.general_unit();
    let label = |ui: &mut Ui, s: &str, tip: &str| {
        let l = ui.add_sized(vec2(22.0, 24.0), egui::Label::new(egui::RichText::new(s).size(11.5).strong().color(t.text))).on_hover_text(tip);
        crate::scrub::note_label(ui, l.rect);
    };
    egui::Grid::new("para-grid").num_columns(4).spacing([4.0, 4.0]).show(ui, |ui| {
        label(ui, "→|", tl!("Left Indent"));
        if let Some(v) = widgets::spin_field(ui, "pa-li", shared_para_field(app, |p| p.left_indent), unit, fw, 1.0, -1296.0, &[]) {
            format(app, json!({"leftIndent": v}));
        }
        label(ui, "|←", tl!("Right Indent"));
        if let Some(v) = widgets::spin_field(ui, "pa-ri", shared_para_field(app, |p| p.right_indent), unit, fw, 1.0, -1296.0, &[]) {
            format(app, json!({"rightIndent": v}));
        }
        ui.end_row();
        label(ui, "1→", tl!("First-line Left Indent"));
        if let Some(v) = widgets::spin_field(ui, "pa-fi", shared_para_field(app, |p| p.first_line_indent), unit, fw, 1.0, -1296.0, &[]) {
            format(app, json!({"firstLineIndent": v}));
        }
        ui.label("");
        ui.label("");
        ui.end_row();
        label(ui, "↑¶", tl!("Space Before Paragraph"));
        if let Some(v) = widgets::spin_field(ui, "pa-sb", shared_para_field(app, |p| p.space_before), unit, fw, 1.0, 0.0, &[]) {
            format(app, json!({"spaceBefore": v}));
        }
        label(ui, "↓¶", tl!("Space After Paragraph"));
        if let Some(v) = widgets::spin_field(ui, "pa-sa", shared_para_field(app, |p| p.space_after), unit, fw, 1.0, 0.0, &[]) {
            format(app, json!({"spaceAfter": v}));
        }
        ui.end_row();
    });
    ui.add_space(4.0);
    if pstate::<bool>(ui.ctx(), "pa-hide-options") {
        return;
    }
    hyphenate_check(app, ui, &para);
    // How long the kashidas are that justify Arabic lines, with the Middle Eastern (Indic) options.
    if app.session.prefs.show_indic_options {
        ui.horizontal(|ui| {
            use vectorcraft_doc::text::Kashida;
            widgets::dim_label(ui, tl!("Insert Kashidas"));
            let kinds = [
                (tl!("None"), Kashida::None, "none"),
                (tl!("Short"), Kashida::Short, "short"),
                (tl!("Medium"), Kashida::Medium, "medium"),
                (tl!("Long"), Kashida::Long, "long"),
                (tl!("Stylistic"), Kashida::Stylistic, "stylistic"),
            ];
            let names = kinds.map(|k| k.0);
            let current = kinds.iter().find(|k| k.1 == para.kashida).map_or(names[0], |k| k.0);
            // Already translated: shown as they are.
            if let Some(i) = widgets::dropdown_names(ui, "pa-kashida", current, &names, ui.available_width() - 4.0)
                && let Some((_, _, key)) = kinds.get(i)
            {
                format(app, json!({"kashida": key}));
            }
        });
    }
    // Japanese composition: how punctuation is spaced (JLREQ 3.1), with the East Asian options.
    if app.session.prefs.show_east_asian_options {
        ui.horizontal(|ui| {
            widgets::dim_label(ui, tl!("Mojikumi Set"));
            let sets = [tl!("None"), tl!("Line-end Punctuation Half Width")];
            let current = match shared_para(app, |p| p.mojikumi) {
                Some(Mojikumi::LineEndHalf) => sets[1],
                Some(Mojikumi::None) => sets[0],
                None => "",
            };
            // Already translated: shown as they are.
            if let Some(i) = widgets::dropdown_names(ui, "pa-mojikumi", current, &sets, ui.available_width() - 4.0) {
                format(app, json!({"mojikumi": if i == 1 { "lineEndHalf" } else { "none" }}));
            }
        });
        // Which characters may not start or end a line.
        ui.horizontal(|ui| {
            widgets::dim_label(ui, tl!("Kinsoku Set"));
            let sets = [(tl!("None"), Kinsoku::None, "none"), (tl!("Hard"), Kinsoku::Hard, "hard"), (tl!("Soft"), Kinsoku::Soft, "soft")];
            let names = sets.map(|s| s.0);
            let current = shared_para(app, |p| p.kinsoku).and_then(|k| sets.iter().find(|s| s.1 == k).map(|s| s.0)).unwrap_or("");
            // Already translated: shown as they are.
            if let Some(i) = widgets::dropdown_names(ui, "pa-kinsoku", current, &names, ui.available_width() - 4.0)
                && let Some((_, _, key)) = sets.get(i)
            {
                format(app, json!({"kinsoku": key}));
            }
        });
    }
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let style = text_style(app);
    let has = style.is_some();
    let hidden: bool = pstate(ui.ctx(), "pa-hide-options");
    if menu_item(ui, if hidden { tl!("Show Options") } else { tl!("Hide Options") }, true, false) {
        set_pstate(ui.ctx(), "pa-hide-options", !hidden);
    }
    ui.separator();
    // Quotation marks, hyphens, full stops and commas at a line's ends hang outside the margin.
    let roman = style.as_ref().is_some_and(|(_, p)| p.roman_hanging);
    if menu_item(ui, tl!("Roman Hanging Punctuation"), has, roman) {
        format(app, json!({"romanHangingPunctuation": !roman}));
    }
    if menu_item(ui, tl!("Justification…"), has, false)
        && let Err(e) = app.run("ui.justification", json!({}))
    {
        app.status(e);
    }
    if menu_item(ui, tl!("Hyphenation…"), has, false)
        && let Err(e) = app.run("ui.hyphenation", json!({}))
    {
        app.status(e);
    }
    // How leading is measured (Japanese layout measures it from em box top to em box top), with
    // the East Asian options.
    if app.session.prefs.show_east_asian_options {
        ui.separator();
        let top_to_top = style.as_ref().is_some_and(|(_, p)| p.leading_model == vectorcraft_doc::LeadingModel::EmBoxTop);
        if menu_item(ui, tl!("Top-to-Top Leading"), has, top_to_top) {
            format(app, json!({"leadingModel": "emBoxTop"}));
        }
        if menu_item(ui, tl!("Bottom-to-Bottom Leading"), has, has && !top_to_top) {
            format(app, json!({"leadingModel": "romanBaseline"}));
        }
        // Hanging punctuation: a comma or full stop ending a line stands outside it.
        let hang = style.as_ref().map(|(_, p)| p.burasagari);
        ui.add_enabled_ui(has, |ui| {
            // Indented like the items beside it (their check column).
            ui.menu_button(format!("   {}", tl!("Burasagari")), |ui| {
                use vectorcraft_doc::Burasagari;
                for (label, b, key) in [
                    (tl!("None"), Burasagari::None, "none"),
                    (tl!("Regular"), Burasagari::Standard, "standard"),
                    (tl!("Force"), Burasagari::Forced, "forced"),
                ] {
                    if menu_item(ui, label, true, hang == Some(b)) {
                        format(app, json!({"burasagari": key}));
                    }
                }
            });
        });
    }
    ui.separator();
    let composer = shared_composer(app);
    for (c, label, id) in
        [(Composer::SingleLine, tl!("Single-line Composer"), "singleLine"), (Composer::EveryLine, tl!("Every-line Composer"), "everyLine")]
    {
        if menu_item(ui, label, has, composer == Some(c)) {
            format(app, json!({ "composer": id }));
        }
    }
    ui.separator();
    if menu_item(ui, tl!("Reset Panel"), has, false) {
        reset_panel(app);
    }
}

/// Reset Panel: the paragraphs the panel acts on ([`format`]) take Auto alignment, no list, no
/// indents or paragraph spacing, Hyphenate off with the default Hyphenation and Justification
/// options, Auto direction, Bottom-to-Bottom Leading, Regular burasagari, the Hard kinsoku set, the
/// Every-line Composer, Roman Hanging Punctuation off and no kashidas.
fn reset_panel(app: &mut VectorcraftApp) {
    let h = HyphenationOptions::default();
    let j = vectorcraft_doc::JustificationOptions::default();
    para_cmd(app, "text.setStyle", json!({"justify": "auto"}));
    format(
        app,
        json!({
            "list": "none", "leftIndent": 0, "rightIndent": 0, "firstLineIndent": 0, "spaceBefore": 0, "spaceAfter": 0, "hyphenate": false,
            "wordsLongerThan": h.words_longer_than, "afterFirst": h.after_first, "beforeLast": h.before_last,
            "hyphenLimit": h.hyphen_limit, "hyphenationZone": h.zone, "hyphenateCapitalized": h.capitalized, "kashida": "none",
            "wordSpacing": j.word_spacing, "letterSpacing": j.letter_spacing, "glyphScaling": j.glyph_scaling,
            "autoLeading": j.auto_leading, "singleWordJustification": "fullJustify",
            "direction": "auto", "leadingModel": "romanBaseline", "burasagari": "standard", "kinsoku": "hard", "composer": "everyLine",
            "romanHangingPunctuation": false
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn alignment_row(app: &mut VectorcraftApp, ui: &mut Ui, size: f32) -> egui::Rect {
        let para = text_style(app).map_or_else(ParaStyle::default, |(_, p)| p);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            alignment_buttons(app, ui, &para, size);
        })
        .response
        .rect
    }

    // Inspect the actual selected wells, including the shared Properties-size buttons.
    fn alignment_frame(ctx: &egui::Context, app: &mut VectorcraftApp, size: f32) -> (Vec<Justify>, egui::Pos2) {
        let mut row = egui::Rect::NOTHING;
        let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| row = alignment_row(app, ui, size));
        frame.textures_delta.clear();
        fn wells(shape: &egui::Shape, color: egui::Color32, size: f32, out: &mut Vec<egui::Pos2>) {
            match shape {
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| wells(s, color, size, out)),
                egui::Shape::Rect(r) if r.fill == color && r.rect.size() == vec2(size, size) => out.push(r.rect.center()),
                _ => {}
            }
        }
        let centers: [egui::Pos2; 7] = std::array::from_fn(|i| row.min + vec2(size / 2.0 + i as f32 * (size + 3.0), size / 2.0));
        let mut active = Vec::new();
        for shape in &frame.shapes {
            wells(&shape.shape, Tokens::get(ctx).tool_active, size, &mut active);
        }
        let selected = active
            .iter()
            .map(|pos| {
                let index = centers.iter().position(|center| center.distance(*pos) < 0.1).expect("alignment well");
                ALIGNMENTS[index].0
            })
            .collect();
        (selected, centers[1])
    }

    fn editing_text(text: &str) -> (VectorcraftApp, Value) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
        app.session.select_tool("type", Default::default()).unwrap();
        let id = app.run("text.create", json!({"x": 20, "y": 50, "text": text})).unwrap()["id"].clone();
        app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
        assert!(super::super::character::text_editing(&app).is_some());
        (app, id)
    }

    fn select_range(app: &mut VectorcraftApp, start: usize, end: usize) {
        app.session.set_tool_option("select", &json!({"start": start, "end": end}));
    }

    #[test]
    fn paragraph_mixed_direction_buttons_show_neither() {
        let (mut app, id) = editing_text("A\nB");
        app.session.prefs.show_indic_options = true;
        app.run("text.setFormat", json!({"id": id, "direction": "leftToRight"})).unwrap();
        app.run("text.setFormat", json!({"id": id, "start": 2, "end": 3, "direction": "rightToLeft"})).unwrap();
        app.session.set_tool_option("selectAll", &json!(true));
        let para = text_style(&app).unwrap().1;
        assert_eq!(selected_paragraph_direction(&app, &para), None);
        app.session.set_tool_option("selectAll", &json!(true));
        app.run("text.setFormat", json!({"id": id, "direction": "leftToRight"})).unwrap();
        assert_eq!(selected_paragraph_direction(&app, &para), Some(false));
        app.session.set_tool_option("selectAll", &json!(true));
        app.run("text.setFormat", json!({"id": id, "direction": "rightToLeft"})).unwrap();
        assert_eq!(selected_paragraph_direction(&app, &para), Some(true));
    }

    #[test]
    fn paragraph_mixed_composer_checks_neither_menu_item() {
        let (mut app, id) = editing_text("A\nB\n");
        app.run("text.setFormat", json!({"id": id, "composer": "singleLine"})).unwrap();
        app.run("text.setFormat", json!({"id": id, "start": 2, "end": 3, "composer": "everyLine"})).unwrap();
        app.session.set_tool_option("selectAll", &json!(true));
        assert_eq!(shared_composer(&app), None);
        app.run("text.setFormat", json!({"id": id, "composer": "everyLine"})).unwrap();
        assert_eq!(shared_composer(&app), Some(Composer::EveryLine));
    }

    #[test]
    fn paragraph_mixed_hyphenate_checks_off() {
        let (mut app, id) = editing_text("A\nB\n");
        app.run("text.setFormat", json!({"id": id, "hyphenate": true})).unwrap();
        app.run("text.setFormat", json!({"id": id, "start": 2, "end": 3, "hyphenate": false})).unwrap();
        app.session.set_tool_option("selectAll", &json!(true));
        assert_eq!(shared_hyphenate(&app), None);
        app.run("text.setFormat", json!({"id": id, "hyphenate": true})).unwrap();
        assert_eq!(shared_hyphenate(&app), Some(true));
    }

    #[test]
    fn paragraph_mixed_kinsoku_and_mojikumi_show_blank_dropdowns() {
        let (mut app, id) = editing_text("A\nB\n");
        app.session.prefs.show_east_asian_options = true;
        app.run("text.setFormat", json!({"id": id, "kinsoku": "hard", "mojikumi": "none"})).unwrap();
        app.run("text.setFormat", json!({"id": id, "start": 2, "end": 3, "kinsoku": "soft", "mojikumi": "lineEndHalf"})).unwrap();
        app.session.set_tool_option("selectAll", &json!(true));
        assert_eq!(shared_para(&app, |p| p.kinsoku), None);
        assert_eq!(shared_para(&app, |p| p.mojikumi), None);
        app.run("text.setFormat", json!({"id": id, "kinsoku": "hard", "mojikumi": "lineEndHalf"})).unwrap();
        assert_eq!(shared_para(&app, |p| p.kinsoku), Some(Kinsoku::Hard));
        assert_eq!(shared_para(&app, |p| p.mojikumi), Some(Mojikumi::LineEndHalf));
    }

    #[test]
    fn paragraph_mixed_indent_fields_show_blank() {
        let (mut app, id) = editing_text("A\nB\n");
        app.run("text.setFormat", json!({"id": id, "leftIndent": 10})).unwrap();
        app.run("text.setFormat", json!({"id": id, "start": 2, "end": 3, "leftIndent": 20})).unwrap();
        app.session.set_tool_option("selectAll", &json!(true));
        assert_eq!(shared_para_field(&app, |p| p.left_indent), None);
        assert_eq!(shared_para_field(&app, |p| p.space_before), Some(0.0));
        app.run("text.setFormat", json!({"id": id, "leftIndent": 12})).unwrap();
        assert_eq!(shared_para_field(&app, |p| p.left_indent), Some(12.0));
        app.session.set_tool_option("stopEditing", &json!(true));
        let other = app.run("text.create", json!({"x": 40, "y": 80, "text": "C"})).unwrap()["id"].clone();
        app.run("text.setFormat", json!({"id": other, "leftIndent": 5})).unwrap();
        app.run("select.set", json!({"ids": [id, other]})).unwrap();
        assert_eq!(shared_para_field(&app, |p| p.left_indent), None);
    }

    #[test]
    fn paragraph_mixed_alignment_click_center_and_undo() {
        let (mut app, id) = editing_text("é\nאב\n終");
        app.run("text.setStyle", json!({"id": id, "justify": "left"})).unwrap();
        app.run("text.setStyle", json!({"id": id, "start": 3, "end": 7, "justify": "right"})).unwrap();
        app.run("text.setStyle", json!({"id": id, "start": 8, "end": 11, "justify": "justifyAll"})).unwrap();
        app.run("text.setFormat", json!({"id": id, "start": 3, "end": 7, "leftIndent": 17, "spaceAfter": 9})).unwrap();
        select_range(&mut app, 0, 7);
        let before = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.undo.len();
        let ctx = egui::Context::default();
        let (selected, center) = alignment_frame(&ctx, &mut app, 28.0);
        assert!(selected.is_empty(), "mixed paragraphs must not light one alignment: {selected:?}");
        super::super::tests_appearance::click(&ctx, &mut app, center, |app, ui| {
            alignment_row(app, ui, 28.0);
        });
        assert_eq!(alignment_frame(&ctx, &mut app, 28.0).0, [Justify::Center]);
        let node = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap();
        let NodeKind::Text(text) = &node.kind else { panic!("text") };
        assert_eq!(
            (text.para_at(0).justify, text.para_at(1).justify, text.para_at(2).justify),
            (Justify::Center, Justify::Center, Justify::JustifyAll)
        );
        let NodeKind::Text(old) = &before.node(node.id).unwrap().kind else { panic!("text") };
        for i in 0..3 {
            let mut expected = old.para_at(i).clone();
            if i < 2 {
                expected.justify = Justify::Center;
            }
            assert_eq!(text.para_at(i), &expected, "other paragraph fields stay unchanged");
        }
        assert_eq!(text.runs, old.runs);
        assert_eq!(app.session.active().unwrap().history.undo.len(), history + 1);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc, before);
        assert!(alignment_frame(&ctx, &mut app, 28.0).0.is_empty());
    }

    #[test]
    fn paragraph_mixed_alignment_caret_unicode_boundaries_and_uniform_controls() {
        let (mut app, id) = editing_text("é\nאב\n");
        app.run("text.setStyle", json!({"id": id, "justify": "left"})).unwrap();
        app.run("text.setStyle", json!({"id": id, "start": 3, "end": 7, "justify": "right"})).unwrap();
        app.run("text.setStyle", json!({"id": id, "start": 8, "end": 8, "justify": "justifyAll"})).unwrap();
        let ctx = egui::Context::default();
        for (start, end, expected) in [
            (0, 2, vec![Justify::Left]),
            (2, 2, vec![Justify::Left]), // newline belongs to the preceding paragraph
            (3, 3, vec![Justify::Right]),
            (4, 6, vec![Justify::Right]),          // byte offsets inside multi-byte characters
            (0, 3, vec![]),                        // ending at the next paragraph's start touches both
            (7, 0, vec![]),                        // reversed selection
            (8, 8, vec![Justify::JustifyAll]),     // empty trailing paragraph
            (100, 100, vec![Justify::JustifyAll]), // existing selection clamping
        ] {
            select_range(&mut app, start, end);
            assert_eq!(alignment_frame(&ctx, &mut app, 24.0).0, expected, "range {start}..{end}");
        }
        for (expected, _, _, key) in ALIGNMENTS {
            app.run("text.setStyle", json!({"id": id, "justify": key})).unwrap();
            app.session.set_tool_option("selectAll", &json!(true));
            assert_eq!(alignment_frame(&ctx, &mut app, 28.0).0, [expected]);
        }
    }

    #[test]
    fn paragraph_mixed_alignment_auto_direction_and_split_runs() {
        let (mut app, id) = editing_text("é\nאב");
        // Character formatting splits the RTL paragraph across runs.
        app.run("text.setRangeStyle", json!({"id": id, "start": 3, "end": 5, "size": 24})).unwrap();
        app.session.set_tool_option("selectAll", &json!(true));
        let ctx = egui::Context::default();
        assert!(alignment_frame(&ctx, &mut app, 28.0).0.is_empty(), "Auto resolves separately per paragraph");
        select_range(&mut app, 3, 7);
        assert_eq!(alignment_frame(&ctx, &mut app, 28.0).0, [Justify::Right]);
        app.run("text.setFormat", json!({"id": id, "start": 3, "end": 7, "direction": "leftToRight"})).unwrap();
        app.session.set_tool_option("selectAll", &json!(true));
        assert_eq!(alignment_frame(&ctx, &mut app, 28.0).0, [Justify::Left]);
        app.run("text.setStyle", json!({"id": id, "start": 0, "end": 2, "justify": "left"})).unwrap();
        assert_eq!(alignment_frame(&ctx, &mut app, 28.0).0, [Justify::Left], "explicit Left and resolved Auto agree");
        app.run("text.setFormat", json!({"id": id, "start": 0, "end": 2, "direction": "rightToLeft"})).unwrap();
        assert_eq!(alignment_frame(&ctx, &mut app, 28.0).0, [Justify::Left], "explicit alignment ignores direction");
    }

    #[test]
    fn paragraph_mixed_alignment_explicit_rtl_and_empty_controls() {
        let (mut app, id) = editing_text("Latin\nאב\n\n");
        app.run("text.setStyle", json!({"id": id, "justify": "auto"})).unwrap();
        app.run("text.setFormat", json!({"id": id, "direction": "rightToLeft"})).unwrap();
        // Auto with an explicit direction still needs the real, possibly split, paragraph text.
        app.run("text.setRangeStyle", json!({"id": id, "start": 0, "end": 2, "size": 24})).unwrap();
        let ctx = egui::Context::default();
        for (start, end, expected) in [
            (0, 5, Justify::Right),  // nonempty Latin with explicit RTL
            (6, 10, Justify::Right), // nonempty Hebrew with explicit RTL
            (11, 11, Justify::Left), // existing resolver's empty paragraph behavior
            (12, 12, Justify::Left), // empty trailing paragraph
        ] {
            select_range(&mut app, start, end);
            assert_eq!(alignment_frame(&ctx, &mut app, 28.0).0, [expected], "explicit RTL range {start}..{end}");
        }
        app.run("text.setStyle", json!({"id": id, "start": 0, "end": 5, "justify": "left"})).unwrap();
        select_range(&mut app, 0, 10);
        assert!(alignment_frame(&ctx, &mut app, 28.0).0.is_empty(), "explicit Left and Auto/RTL differ");
        let (mut empty, id) = editing_text("");
        empty.run("text.setStyle", json!({"id": id, "justify": "auto"})).unwrap();
        empty.run("text.setFormat", json!({"id": id, "direction": "rightToLeft"})).unwrap();
        assert_eq!(alignment_frame(&egui::Context::default(), &mut empty, 28.0).0, [Justify::Left]);
    }

    #[test]
    fn paragraph_mixed_alignment_object_group_and_no_text_controls() {
        let (mut app, id) = editing_text("A\nB");
        app.run("text.setStyle", json!({"id": id, "justify": "left"})).unwrap();
        app.run("text.setStyle", json!({"id": id, "start": 2, "end": 3, "justify": "right"})).unwrap();
        app.session.set_tool_option("stopEditing", &json!(true));
        let ctx = egui::Context::default();
        assert!(alignment_frame(&ctx, &mut app, 28.0).0.is_empty(), "whole object covers every paragraph");
        app.run("object.group", json!({})).unwrap();
        assert!(alignment_frame(&ctx, &mut app, 28.0).0.is_empty(), "group descendant has mixed paragraphs");
        app.run("text.setStyle", json!({"justify": "center"})).unwrap();
        assert_eq!(alignment_frame(&ctx, &mut app, 28.0).0, [Justify::Center]);
        let group = app.session.active().unwrap().selection.objects.clone();
        let other = app.run("text.create", json!({"x": 20, "y": 100, "text": "C"})).unwrap()["id"].clone();
        let mut ids: Vec<Value> = group.iter().map(|id| json!(id.0)).collect();
        ids.push(other);
        app.run("select.set", json!({"ids": ids})).unwrap();
        assert!(alignment_frame(&ctx, &mut app, 28.0).0.is_empty(), "different selected objects");
        app.run("select.set", json!({"ids": []})).unwrap();
        assert_eq!(alignment_frame(&ctx, &mut app, 28.0).0, [Justify::Left], "no-text default stays unchanged");
        let (mut empty, _) = editing_text("");
        assert_eq!(alignment_frame(&egui::Context::default(), &mut empty, 28.0).0, [Justify::Left]);
    }

    /// New Hebrew type aligns to the start of its direction (Align Right shows), an explicit
    /// alignment stays as chosen, and Paragraph Direction (with the Indic options) sets the
    /// direction: Auto then follows it.
    #[test]
    fn alignment_buttons_show_the_resolved_alignment_and_direction_sets_it() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 200})).unwrap();
        app.session.execute("text.create", &json!({"x": 200, "y": 50, "text": "שלום"})).unwrap();
        let shown = |app: &VectorcraftApp| {
            let para = text_style(app).unwrap().1;
            shown_alignment(para.justify, resolved_rtl(app, &para))
        };
        assert_eq!(shown(&app), Justify::Right);
        app.session.execute("object.group", &json!({})).unwrap();
        assert_eq!(shown(&app), Justify::Right, "group controls resolve the same text descendant's direction");
        app.session.execute("text.setStyle", &json!({"justify": "left"})).unwrap();
        assert_eq!(shown(&app), Justify::Left);
        app.session.execute("text.setStyle", &json!({"justify": "auto"})).unwrap();
        app.session.execute("text.setFormat", &json!({"direction": "leftToRight"})).unwrap();
        assert_eq!(shown(&app), Justify::Left);
        // The direction buttons show with the Indic options only.
        crate::i18n::set_current(crate::i18n::Lang::EN);
        let tips = |app: &mut VectorcraftApp| {
            let ctx = egui::Context::default();
            let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
            frame.textures_delta.clear();
            frame.shapes.len()
        };
        let without = tips(&mut app);
        app.session.prefs.show_indic_options = true;
        assert!(tips(&mut app) > without, "two more buttons");
    }

    /// Insert Kashidas is in the panel with the Middle Eastern (Indic) options only, shows the
    /// paragraph's setting, and Reset Panel sets it back to None.
    #[test]
    fn insert_kashidas_shows_with_the_indic_options_and_reset_panel_turns_it_off() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
        let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "كتب محمد"})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        crate::i18n::set_current(crate::i18n::Lang::EN);
        let painted = |app: &mut VectorcraftApp| crate::tests_labels::painted_text(app, show);
        assert!(!painted(&mut app).contains("Insert Kashidas"));
        app.session.prefs.show_indic_options = true;
        let text = painted(&mut app);
        assert!(text.contains("Insert Kashidas") && text.contains("None"), "{text}");
        app.run("text.setFormat", json!({"kashida": "medium"})).unwrap();
        assert!(painted(&mut app).contains("Medium"));
        let kashida = |app: &VectorcraftApp| {
            let node = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap();
            let NodeKind::Text(text) = &node.kind else { panic!("text") };
            text.para_at(0).kashida
        };
        assert_eq!(kashida(&app), vectorcraft_doc::text::Kashida::Medium);
        reset_panel(&mut app);
        assert_eq!(kashida(&app), vectorcraft_doc::text::Kashida::None);
    }

    /// The Bullets button makes the paragraphs the Type tool's selection touches a bulleted list
    /// with the dot and shows pressed; the arrow's presets pick another bullet, None turns the list
    /// off, Type › Bullets and Numbering › Apply Bullets applies a list, and Reset Panel turns it
    /// off.
    #[test]
    fn the_bullets_button_and_its_presets_set_the_list_and_reset_panel_clears_it() {
        use vectorcraft_doc::text::{Bullet, ParaList};
        crate::i18n::set_current(crate::i18n::Lang::EN);
        let (mut app, id) = editing_text("One\nTwo");
        select_range(&mut app, 0, 0);
        let lists = |app: &VectorcraftApp| {
            let node = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap();
            let NodeKind::Text(text) = &node.kind else { panic!("text") };
            (text.para_at(0).list, text.para_at(1).list)
        };
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let row = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut draw = |app: &mut VectorcraftApp, ui: &mut Ui| {
            row.set(ui.horizontal(|ui| bullets_button(app, ui, 28.0)).response.rect);
        };
        super::super::tests_appearance::frame_events(&ctx, &mut app, vec![], &mut draw);
        let min = row.get().min;
        let (button, arrow) = (min + vec2(14.0, 14.0), min + vec2(28.0 + 7.0, 14.0));
        super::super::tests_appearance::click(&ctx, &mut app, button, &mut draw);
        assert_eq!(lists(&app), (Some(ParaList::bulleted(Bullet::Dot)), None), "the caret's paragraph");
        assert_eq!(shared_para(&app, |p| p.list.map(|l| l.bullet)), Some(Some(Bullet::Dot)), "the button shows pressed");
        let mut pick = |app: &mut VectorcraftApp, name: &str| {
            super::super::tests_appearance::click(&ctx, app, arrow, &mut draw);
            let texts = super::super::tests_appearance::frame_events(&ctx, app, vec![], &mut draw);
            let at = super::super::tests_appearance::text_rect(&texts, name).center();
            super::super::tests_appearance::click(&ctx, app, at, &mut draw);
        };
        pick(&mut app, "Filled Square");
        assert_eq!(lists(&app), (Some(ParaList::bulleted(Bullet::Square)), None));
        pick(&mut app, "None");
        assert_eq!(lists(&app), (None, None));
        app.session.set_tool_option("selectAll", &json!(true));
        app.run("type.applyBullets", json!({})).unwrap();
        assert_eq!(lists(&app), (Some(ParaList::bulleted(Bullet::Dot)), Some(ParaList::bulleted(Bullet::Dot))));
        app.run("type.applyBullets", json!({"bullet": "dash"})).unwrap();
        assert_eq!(lists(&app).0, Some(ParaList::bulleted(Bullet::Dash)));
        app.session.set_tool_option("selectAll", &json!(true));
        reset_panel(&mut app);
        assert_eq!(lists(&app), (None, None));
    }

    /// The Numbered List button makes the paragraphs the Type tool's selection touches a numbered
    /// list 1, 2, 3 and shows pressed; the arrow's presets, each shown by its first three numbers,
    /// pick another style, None turns the list off, Type › Bullets and Numbering › Apply Numbers
    /// applies a numbered list, and Reset Panel turns it off.
    #[test]
    fn the_numbered_list_button_and_its_presets_set_the_list_and_reset_panel_clears_it() {
        use vectorcraft_doc::text::{NumberStyle, ParaList};
        crate::i18n::set_current(crate::i18n::Lang::EN);
        let (mut app, id) = editing_text("One\nTwo");
        select_range(&mut app, 0, 0);
        let lists = |app: &VectorcraftApp| {
            let node = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap();
            let NodeKind::Text(text) = &node.kind else { panic!("text") };
            (text.para_at(0).list, text.para_at(1).list)
        };
        let numbered = |style| Some(ParaList::numbered(style));
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let row = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut draw = |app: &mut VectorcraftApp, ui: &mut Ui| {
            row.set(ui.horizontal(|ui| numbers_button(app, ui, 28.0)).response.rect);
        };
        super::super::tests_appearance::frame_events(&ctx, &mut app, vec![], &mut draw);
        let min = row.get().min;
        let (button, arrow) = (min + vec2(14.0, 14.0), min + vec2(28.0 + 7.0, 14.0));
        super::super::tests_appearance::click(&ctx, &mut app, button, &mut draw);
        assert_eq!(lists(&app), (numbered(NumberStyle::Arabic), None), "the caret's paragraph");
        assert_eq!(shared_para(&app, |p| p.list.and_then(|l| l.number)), Some(Some(NumberStyle::Arabic)), "the button shows pressed");
        let mut pick = |app: &mut VectorcraftApp, name: &str| {
            super::super::tests_appearance::click(&ctx, app, arrow, &mut draw);
            let texts = super::super::tests_appearance::frame_events(&ctx, app, vec![], &mut draw);
            let at = super::super::tests_appearance::text_rect(&texts, name).center();
            super::super::tests_appearance::click(&ctx, app, at, &mut draw);
        };
        for (name, style) in [
            ("I, II, III", NumberStyle::UpperRoman),
            ("a, b, c", NumberStyle::LowerLetters),
            ("001, 002, 003", NumberStyle::ThreeDigits),
            ("1, 2, 3", NumberStyle::Arabic),
        ] {
            pick(&mut app, name);
            assert_eq!(lists(&app), (numbered(style), None), "{name}");
        }
        pick(&mut app, "None");
        assert_eq!(lists(&app), (None, None));
        app.session.set_tool_option("selectAll", &json!(true));
        app.run("type.applyNumbers", json!({})).unwrap();
        assert_eq!(lists(&app), (numbered(NumberStyle::Arabic), numbered(NumberStyle::Arabic)));
        // The Numbered List button shows pressed and the Bullets button doesn't, and the other way
        // round for a bulleted list.
        // Each button and the arrow beside it have the same name; only a button shows pressed.
        let pressed = |app: &mut VectorcraftApp| {
            let nodes = crate::widgets::accessibility_tests::tree(|ui| {
                ui.horizontal(|ui| {
                    bullets_button(app, ui, 28.0);
                    numbers_button(app, ui, 28.0);
                });
            });
            let on = |label| nodes.iter().any(|n| n.label() == Some(label) && n.toggled() == Some(egui::accesskit::Toggled::True));
            (on("Bullets"), on("Numbered List"))
        };
        assert_eq!(pressed(&mut app), (false, true));
        app.session.set_tool_option("selectAll", &json!(true));
        app.run("type.applyBullets", json!({})).unwrap();
        assert_eq!(pressed(&mut app), (true, false));
        app.session.set_tool_option("selectAll", &json!(true));
        app.run("type.applyNumbers", json!({})).unwrap();
        app.run("type.applyNumbers", json!({"number": "upperLetters"})).unwrap();
        assert_eq!(lists(&app).0, numbered(NumberStyle::UpperLetters));
        app.session.set_tool_option("selectAll", &json!(true));
        reset_panel(&mut app);
        assert_eq!(lists(&app), (None, None));
    }

    /// Reset Panel turns Hyphenate and Roman Hanging Punctuation off and sets the Hyphenation and
    /// Justification options back to their defaults in every paragraph it acts on, so the paragraphs
    /// save neither.
    #[test]
    fn reset_panel_resets_the_hyphenation_options() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
        let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "First\nSecond"})).unwrap()["id"].clone();
        app.run(
            "text.setFormat",
            json!({
                "hyphenate": true, "wordsLongerThan": 8, "afterFirst": 2, "beforeLast": 4, "hyphenLimit": 2, "hyphenationZone": 18, "hyphenateCapitalized": false,
                "wordSpacing": [60, 90, 200], "letterSpacing": [-5, 0, 5], "glyphScaling": [95, 100, 105], "autoLeading": 150, "singleWordJustification": "alignLeft", "romanHangingPunctuation": true
            }),
        )
        .unwrap();
        let paras = |app: &VectorcraftApp| {
            let node = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap();
            let NodeKind::Text(text) = &node.kind else { panic!("text") };
            [text.para_at(0).clone(), text.para_at(1).clone()]
        };
        for para in paras(&app) {
            assert!(para.hyphenate);
            assert_ne!(para.hyphenation, HyphenationOptions::default());
            assert!(para.roman_hanging);
            assert_ne!(para.justification, vectorcraft_doc::JustificationOptions::default());
        }
        reset_panel(&mut app);
        for para in paras(&app) {
            assert!(!para.hyphenate);
            assert_eq!(para.hyphenation, HyphenationOptions::default());
            assert_eq!(serde_json::to_value(&para).unwrap().get("hyphenation"), None, "saves no hyphenation options");
            assert!(!para.roman_hanging, "Roman Hanging Punctuation off");
            assert_eq!(serde_json::to_value(&para).unwrap().get("justification"), None, "saves no justification options");
        }
    }
}
