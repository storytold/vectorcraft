//! Type → Find Font…: the fonts the document uses (missing ones marked), Find (select the text
//! using a font), and Change / Change All to another installed font.

use egui::Ui;
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::state::Dialog;
use crate::theme::{self, Tokens};
use crate::widgets;

/// Open the dialog.
pub fn open(app: &mut VectorcraftApp) {
    app.ui.dialog = Some(Dialog::new("findFont", json!({ "selected": 0, "family": "Source Sans 3", "style": "", "selectionOnly": false })));
}

fn fonts(app: &mut VectorcraftApp) -> Vec<Value> {
    app.session.execute("text.fonts", &json!({})).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default()
}

pub fn confirm(app: &mut VectorcraftApp) -> Result<Value, String> {
    app.ui.dialog = None;
    Ok(Value::Null)
}

/// Replace the selected document font with the chosen one (`all`: everywhere, else the selection).
fn change(app: &mut VectorcraftApp, d: &Dialog, from: &Value, all: bool) -> Result<Value, String> {
    let style = d.str("style");
    let to = if style.is_empty() { json!({ "family": d.str("family") }) } else { json!({ "family": d.str("family"), "style": style }) };
    app.run("text.replaceFont", json!({ "from": { "family": from["family"], "style": from["style"] }, "to": to, "selectionOnly": !all }))
}

pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(mut d) = app.ui.dialog.clone() else { return };
    let t = Tokens::get(ctx);
    let list = fonts(app);
    let mut close = false;
    let mut act: Option<&str> = None;
    egui::Area::new(egui::Id::new("modal-dim")).order(egui::Order::Middle).fixed_pos(egui::pos2(0.0, 0.0)).show(ctx, |ui| {
        ui.allocate_rect(ctx.content_rect(), egui::Sense::click());
    });
    egui::Window::new(tl!("Find Font"))
        .id(egui::Id::new("dialog-find-font"))
        .order(egui::Order::Foreground)
        .collapsible(false)
        .resizable(false)
        .title_bar(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center() + egui::vec2(0.0, -40.0))
        .constrain(true)
        .frame(egui::Frame::window(&ctx.global_style()).fill(t.panel).inner_margin(egui::Margin::same(22)))
        .show(ctx, |ui: &mut Ui| {
            ui.set_width(380.0);
            ui.label(egui::RichText::new(tl!("Find Font")).font(theme::semibold(16.0)).color(t.text));
            ui.add_space(10.0);
            widgets::subheader(ui, &crate::i18n::fmt(tl!("Fonts in Document: {count}"), &[("count", &list.len().to_string())]));
            let sel = d.fields.get("selected").and_then(Value::as_u64).unwrap_or(0) as usize;
            egui::Frame::NONE.fill(t.input).stroke(egui::Stroke::new(1.0, t.input_border)).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
                ui.set_min_height(120.0);
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                    for (i, f) in list.iter().enumerate() {
                        let (note, color) = font_note(f);
                        let label = format!("{} {}{note}  ({})", f["family"].as_str().unwrap_or(""), f["style"].as_str().unwrap_or(""), f["runs"]);
                        let text = egui::RichText::new(label).color(color.unwrap_or(t.text));
                        if ui.selectable_label(i == sel, text).clicked() {
                            d.fields.insert("selected".into(), json!(i));
                        }
                    }
                });
            });
            ui.add_space(10.0);
            widgets::subheader(ui, tl!("Replace With Font"));
            let fam = d.str("family");
            ui.horizontal(|ui| {
                if let Some(f) = widgets::font_dropdown(ui, "ff-family", &fam, 220.0) {
                    d.fields.insert("family".into(), json!(f));
                    d.fields.insert("style".into(), json!(""));
                }
                let styles = vectorcraft_text::FontDb::global().styles(&d.str("family"));
                // "(closest)" is ours; the font's style names are shown as they are.
                let closest = tl!("(closest)");
                let mut opts: Vec<&str> = vec![closest];
                opts.extend(styles.iter().map(String::as_str));
                let style = d.str("style");
                let cur = if style.is_empty() { closest } else { style.as_str() };
                if let Some(i) = widgets::dropdown_names(ui, "ff-style", cur, &opts, 120.0) {
                    d.fields.insert("style".into(), json!(if i == 0 { String::new() } else { styles[i - 1].clone() }));
                }
            });
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                let has = sel < list.len();
                if ui.add_enabled(has, egui::Button::new(tl!("Find"))).clicked() {
                    act = Some("find");
                }
                if ui.add_enabled(has, egui::Button::new(tl!("Change"))).on_hover_text(tl!("In the selected objects")).clicked() {
                    act = Some("change");
                }
                if ui.add_enabled(has, egui::Button::new(tl!("Change All"))).clicked() {
                    act = Some("changeAll");
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::primary_button(ui, tl!("Done")).clicked() {
                        close = true;
                    }
                });
            });
        });
    let sel = d.fields.get("selected").and_then(Value::as_u64).unwrap_or(0) as usize;
    if let (Some(a), Some(from)) = (act, list.get(sel)) {
        let r = match a {
            "find" => app.run("select.font", json!({ "family": from["family"], "style": from["style"] })),
            "change" => change(app, &d, from, false),
            _ => change(app, &d, from, true),
        };
        if let Err(e) = r {
            app.ui.status = e;
        }
    }
    if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.ui.dialog = None;
    } else if app.ui.dialog.is_some() {
        app.ui.dialog = Some(d);
    }
}

/// What a Find Font row says after the font's name (`text.fonts` row): a missing family, a style
/// standing in for another, characters the font lacks; and the colour to flag it with.
fn font_note(f: &Value) -> (String, Option<egui::Color32>) {
    let lacking = f["missingGlyphs"].as_u64().unwrap_or(0);
    let glyphs =
        if lacking > 0 { crate::i18n::fmt(tl!("  — {n} characters from another font"), &[("n", &lacking.to_string())]) } else { String::new() };
    match f["status"].as_str() {
        Some("missing") => (format!("{}{glyphs}", tl!("  — missing")), Some(egui::Color32::from_rgb(230, 90, 90))),
        Some("substitute") => {
            let used = f["resolved"]["style"].as_str().unwrap_or("");
            (
                format!("{}{glyphs}", crate::i18n::fmt(tl!("  — substituted by {style}"), &[("style", used)])),
                Some(egui::Color32::from_rgb(220, 160, 60)),
            )
        }
        _ if lacking > 0 => (glyphs, Some(egui::Color32::from_rgb(220, 160, 60))),
        _ => (String::new(), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialog_draws_and_changes_all() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Hi", "font": "Missing Family"})).unwrap();
        open(&mut app);
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut app, ui.ctx()));
        out.textures_delta.clear();
        let d = app.ui.dialog.clone().unwrap();
        let from = fonts(&mut app)[0].clone();
        assert_eq!(from["missing"], true);
        change(&mut app, &d, &from, true).unwrap();
        assert_eq!(fonts(&mut app)[0]["family"], "Source Sans 3");
    }

    #[test]
    fn rows_flag_missing_fonts_substituted_styles_and_lacking_characters() {
        let row = |status: &str, glyphs: u64| json!({"status": status, "resolved": {"style": "W4"}, "missingGlyphs": glyphs});
        assert_eq!(font_note(&row("exact", 0)), (String::new(), None));
        assert!(font_note(&row("missing", 0)).0.contains("missing"));
        assert!(font_note(&row("substitute", 0)).0.contains("substituted by W4"));
        let (note, color) = font_note(&row("exact", 2));
        assert!(note.contains("2 characters") && color.is_some());
    }
}
