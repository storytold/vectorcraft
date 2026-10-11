//! Paragraph panel: seven alignment buttons, Paragraph Direction (with the Indic options), indents,
//! space before/after, Hyphenate, Mojikumi Set and Kinsoku Set (with the East Asian options); the panel menu
//! picks the Single-line or Every-line Composer.

use egui::{Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::{Composer, Justify, NodeKind, ParaDirection, ParaStyle};

use super::character::text_style;
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

fn para_cmd(app: &mut VectorcraftApp, cmd: &str, mut p: Value) {
    if let Some((id, a, b)) = super::character::text_editing(app) {
        super::character::end_typing(app);
        p["ids"] = json!([id.0]);
        p["start"] = json!(a);
        p["end"] = json!(b);
    }
    app.run(cmd, p).ok();
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

/// The seven alignment buttons, `size` points square, the one of `para`'s alignment on (the
/// Paragraph panel's and the Properties panel's).
pub fn alignment_buttons(app: &mut VectorcraftApp, ui: &mut Ui, para: &ParaStyle, size: f32) {
    let shown = shown_alignment(para.justify, resolved_rtl(app, para));
    for (j, icon, tip, id) in ALIGNMENTS {
        if widgets::icon_button(ui, icon, tip, shown == j, size).clicked() {
            para_cmd(app, "text.setStyle", json!({"justify": id}));
        }
    }
}

/// The Hyphenate checkbox of `para`'s paragraphs.
pub fn hyphenate_check(app: &mut VectorcraftApp, ui: &mut Ui, para: &ParaStyle) {
    if widgets::check(ui, tl!("Hyphenate"), para.hyphenate, true) {
        format(app, json!({"hyphenate": !para.hyphenate}));
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Some((_, para)) = text_style(app) else {
        super::empty_state(ui, "pilcrow", tl!("No text selected"), tl!("Select a text object to edit its paragraph attributes."));
        return;
    };
    let rtl = resolved_rtl(app, &para);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        alignment_buttons(app, ui, &para, 28.0);
        // Paragraph direction, with the Middle Eastern (Indic) options.
        if app.session.prefs.show_indic_options {
            ui.add_space(6.0);
            for (to_rtl, icon, tip, id) in [
                (false, "dc-para-ltr", tl!("Left-to-Right Paragraph Direction"), "leftToRight"),
                (true, "dc-para-rtl", tl!("Right-to-Left Paragraph Direction"), "rightToLeft"),
            ] {
                if widgets::icon_button(ui, icon, tip, rtl == to_rtl, 28.0).clicked() {
                    format(app, json!({"direction": id}));
                }
            }
        }
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
        if let Some(v) = widgets::spin_field(ui, "pa-li", Some(para.left_indent), unit, fw, 1.0, -1296.0, &[]) {
            format(app, json!({"leftIndent": v}));
        }
        label(ui, "|←", tl!("Right Indent"));
        if let Some(v) = widgets::spin_field(ui, "pa-ri", Some(para.right_indent), unit, fw, 1.0, -1296.0, &[]) {
            format(app, json!({"rightIndent": v}));
        }
        ui.end_row();
        label(ui, "1→", tl!("First-line Left Indent"));
        if let Some(v) = widgets::spin_field(ui, "pa-fi", Some(para.first_line_indent), unit, fw, 1.0, -1296.0, &[]) {
            format(app, json!({"firstLineIndent": v}));
        }
        ui.label("");
        ui.label("");
        ui.end_row();
        label(ui, "↑¶", tl!("Space Before Paragraph"));
        if let Some(v) = widgets::spin_field(ui, "pa-sb", Some(para.space_before), unit, fw, 1.0, 0.0, &[]) {
            format(app, json!({"spaceBefore": v}));
        }
        label(ui, "↓¶", tl!("Space After Paragraph"));
        if let Some(v) = widgets::spin_field(ui, "pa-sa", Some(para.space_after), unit, fw, 1.0, 0.0, &[]) {
            format(app, json!({"spaceAfter": v}));
        }
        ui.end_row();
    });
    ui.add_space(4.0);
    if pstate::<bool>(ui.ctx(), "pa-hide-options") {
        return;
    }
    hyphenate_check(app, ui, &para);
    // Japanese composition: how punctuation is spaced (JLREQ 3.1), with the East Asian options.
    if app.session.prefs.show_east_asian_options {
        ui.horizontal(|ui| {
            widgets::dim_label(ui, tl!("Mojikumi Set"));
            let sets = [tl!("None"), tl!("Line-end Punctuation Half Width")];
            let current = if para.mojikumi == vectorcraft_doc::Mojikumi::LineEndHalf { sets[1] } else { sets[0] };
            // Already translated: shown as they are.
            if let Some(i) = widgets::dropdown_names(ui, "pa-mojikumi", current, &sets, ui.available_width() - 4.0) {
                format(app, json!({"mojikumi": if i == 1 { "lineEndHalf" } else { "none" }}));
            }
        });
        // Which characters may not start or end a line.
        ui.horizontal(|ui| {
            use vectorcraft_doc::Kinsoku;
            widgets::dim_label(ui, tl!("Kinsoku Set"));
            let sets = [(tl!("None"), Kinsoku::None, "none"), (tl!("Hard"), Kinsoku::Hard, "hard"), (tl!("Soft"), Kinsoku::Soft, "soft")];
            let names = sets.map(|s| s.0);
            let current = sets.iter().find(|s| s.1 == para.kinsoku).map_or(names[1], |s| s.0);
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
    for l in [tl!("Roman Hanging Punctuation"), tl!("Justification…"), tl!("Hyphenation…")] {
        menu_item(ui, l, false, false);
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
    let composer = style.as_ref().map(|(_, p)| p.composer);
    for (c, label, id) in
        [(Composer::SingleLine, tl!("Single-line Composer"), "singleLine"), (Composer::EveryLine, tl!("Every-line Composer"), "everyLine")]
    {
        if menu_item(ui, label, has, composer == Some(c)) {
            format(app, json!({ "composer": id }));
        }
    }
    ui.separator();
    if menu_item(ui, tl!("Reset Panel"), has, false) {
        para_cmd(app, "text.setStyle", json!({"justify": "auto"}));
        format(
            app,
            json!({"leftIndent": 0, "rightIndent": 0, "firstLineIndent": 0, "spaceBefore": 0, "spaceAfter": 0, "hyphenate": false, "direction": "auto", "leadingModel": "romanBaseline", "burasagari": "standard", "kinsoku": "hard", "composer": "everyLine"}),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

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
}
