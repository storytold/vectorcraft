//! Color Themes panel (local only): the Create tab makes a five-colour theme on a harmony wheel (a
//! harmony rule from the selected colour, or Custom: each colour moves on its own; with a rule the
//! colours move together), the My Themes tab lists the themes saved in the preferences. Themes are
//! saved, edited, deleted and added to Swatches as colour groups through `colorTheme.*`; the theme
//! being made is panel state.

use egui::{Rect, Sense, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_color::Color;
use vectorcraft_color::harmony::{Harmony, THEME_SIZE, move_on_wheel};
use vectorcraft_engine::cmd::colortheme::ColorTheme;

use super::{active_paint, c32, color_json, empty_state, pstate, set_pstate};
use crate::VectorcraftApp;
use crate::theme::{self, Tokens};
use crate::widgets::{self, menu_item};

/// The rule a new theme starts with.
const DEFAULT_RULE: Harmony = Harmony::Analogous2;
/// The base colour before one is picked: a teal.
const DEFAULT_BASE: Color = Color::Rgb { r: 0.1, g: 0.55, b: 0.6 };
/// Diameter of the harmony wheel.
const WHEEL: f32 = 150.0;

/// The theme being made on the Create tab.
#[derive(Clone, Debug, Default, PartialEq)]
struct Draft {
    colors: Vec<Color>,
    /// The harmony rule (`None`: Custom).
    rule: Option<Harmony>,
    /// The selected colour (the base a rule starts from).
    sel: usize,
    name: String,
    /// The saved theme being edited: Save overwrites it.
    editing: Option<String>,
}

impl Draft {
    /// A new theme from `base` by `rule`.
    fn new(base: Color, rule: Option<Harmony>) -> Self {
        let colors = rule.map_or_else(|| DEFAULT_RULE.theme(base), |r| r.theme(base));
        Self { colors, rule, ..Default::default() }
    }
    /// Set the selected colour to `c`: with a rule the theme follows from it as the new base.
    fn set_selected(&mut self, c: Color) {
        match self.rule {
            Some(r) => {
                self.colors = r.theme(c);
                self.sel = 0;
            }
            None => {
                if let Some(x) = self.colors.get_mut(self.sel) {
                    *x = c;
                }
            }
        }
    }
    /// `colorTheme.save` parameters.
    fn save_params(&self) -> Value {
        let mut p = json!({ "colors": colors_json(&self.colors) });
        if let Some(r) = self.rule {
            p["rule"] = json!(r.id());
        }
        if !self.name.trim().is_empty() {
            p["name"] = json!(self.name.trim());
        }
        if let Some(e) = &self.editing {
            p["replace"] = json!(e);
        }
        p
    }
}

/// Colours as command params, exact in their models.
fn colors_json(colors: &[Color]) -> Vec<Value> {
    colors.iter().map(color_json).collect()
}

/// The theme being made: from the current colour when the panel first shows.
fn draft(app: &VectorcraftApp, ctx: &egui::Context) -> Draft {
    if let Some(d) = pstate::<Option<Draft>>(ctx, "ct-draft") {
        return d;
    }
    let d = Draft::new(active_paint(app).color().unwrap_or(DEFAULT_BASE), Some(DEFAULT_RULE));
    set_pstate(ctx, "ct-draft", Some(d.clone()));
    d
}

fn set_draft(ctx: &egui::Context, d: Draft) {
    set_pstate(ctx, "ct-draft", Some(d));
}

/// The saved theme picked on the My Themes tab.
fn picked(app: &VectorcraftApp, ctx: &egui::Context) -> Option<String> {
    pstate::<Option<String>>(ctx, "ct-picked").filter(|n| app.session.prefs.color_themes.iter().any(|t| t.name == *n))
}

/// The tab shown: 0 Create, 1 My Themes.
fn tab(ctx: &egui::Context) -> usize {
    pstate(ctx, "ct-tab")
}

/// Save the theme being made (overwriting the theme it edits); it then edits the saved theme.
fn save(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let mut d = draft(app, ctx);
    if let Ok(r) = app.run("colorTheme.save", d.save_params()) {
        let name = r["name"].as_str().unwrap_or_default().to_string();
        set_pstate(ctx, "ct-picked", Some(name.clone()));
        d.name = name.clone();
        d.editing = Some(name);
        set_draft(ctx, d);
    }
}

/// Load saved theme `t` into the Create tab to edit it.
fn edit(ctx: &egui::Context, t: &ColorTheme) {
    let rule = t.rule.as_deref().and_then(Harmony::parse);
    set_draft(ctx, Draft { colors: t.colors.clone(), rule, sel: 0, name: t.name.clone(), editing: Some(t.name.clone()) });
    set_pstate(ctx, "ct-tab", 0usize);
}

/// Add the theme being made to Swatches as a colour group (one undo step).
fn add_draft_to_swatches(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let d = draft(app, ctx);
    let name = if d.name.trim().is_empty() { "Color Theme" } else { d.name.trim() };
    app.run("swatch.newGroup", json!({ "name": name, "colors": colors_json(&d.colors), "keepModel": true })).ok();
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    ui.horizontal(|ui| {
        let cur = tab(&ctx);
        for (i, label) in [tl!("Create"), tl!("My Themes")].into_iter().enumerate() {
            if ui.selectable_label(cur == i, egui::RichText::new(label).font(theme::semibold(12.5))).clicked() {
                set_pstate(&ctx, "ct-tab", i);
            }
        }
    });
    widgets::divider(ui);
    ui.add_space(4.0);
    if tab(&ctx) == 1 { my_themes(app, ui) } else { create(app, ui) }
}

fn create(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    let mut d = draft(app, &ctx);
    let before = d.clone();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Rule:")).color(t.text_dim));
        let labels: Vec<&str> = std::iter::once("Custom").chain(Harmony::ALL.iter().map(|h| h.label())).collect();
        let cur = d.rule.map_or("Custom", Harmony::label);
        // A rule makes the theme from the selected colour; Custom frees the colours.
        if let Some(i) = widgets::dropdown(ui, "ct-rule", cur, &labels, ui.available_width() - 4.0) {
            d.rule = i.checked_sub(1).map(|i| Harmony::ALL[i]);
            if let (Some(r), Some(&base)) = (d.rule, d.colors.get(d.sel)) {
                d.colors = r.theme(base);
                d.sel = 0;
            }
        }
    });
    ui.add_space(6.0);
    let w = widgets::harmony_wheel(ui, "ct-wheel", WHEEL, &d.colors, Some(d.sel));
    if let Some(k) = w.pressed {
        d.sel = k;
    }
    if let Some((k, set)) = w.moved
        && let Some(cur) = d.colors.get(k).map(Color::to_hsb)
    {
        let hsb = [0, 1, 2].map(|n| set[n].unwrap_or(cur[n]));
        move_on_wheel(&mut d.colors, k, hsb, d.rule.is_some());
    }
    ui.add_space(8.0);
    // The theme's colours: click one to select it.
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
    let cw = row.width() / THEME_SIZE as f32;
    for (i, c) in d.colors.iter().enumerate() {
        let r = Rect::from_min_size(row.min + vec2(i as f32 * cw, 0.0), vec2(cw - 3.0, row.height()));
        let resp = ui.interact(r, ui.scope_id().with(("ct-chip", i)), Sense::click());
        ui.painter().rect_filled(r, 2.0, c32(c));
        ui.painter().rect_stroke(r, 2.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
        if i == d.sel {
            ui.painter().rect_stroke(r.expand(1.5), 3.0, egui::Stroke::new(1.5, t.accent), egui::StrokeKind::Outside);
        }
        if resp.on_hover_text(c.to_hex()).clicked() {
            d.sel = i;
        }
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let hex = d.colors.get(d.sel).map(|c| c.to_hex().trim_start_matches('#').to_string()).unwrap_or_default();
        if let Some(c) = widgets::hex_field_named(ui, "ct-hex", Some(tl!("Color")), &hex).and_then(|h| Color::from_hex(&h)) {
            d.set_selected(c);
        }
        let current = active_paint(app).color();
        if widgets::icon_button_enabled(ui, "pipette", tl!("Set the selected color to the current color"), false, current.is_some(), 24.0).clicked()
            && let Some(c) = current
        {
            d.set_selected(c);
        }
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Name:")).color(t.text_dim));
        if let Some(n) = widgets::text_field_named(ui, "ct-name", Some(tl!("Name:")), Some(&d.name), ui.available_width() - 4.0, 1) {
            d.name = n;
        }
    });
    if d != before {
        set_draft(&ctx, d.clone());
    }
    widgets::bottom_bar(ui, |ui| {
        let tip = match &d.editing {
            Some(n) => crate::i18n::fmt(tl!("Save changes to {name}"), &[("name", n)]),
            None => tl!("Save theme to My Themes").into(),
        };
        // The tip is translated around the theme's name, not looked up whole.
        let resp = widgets::icon_button(ui, "save", "", false, 24.0);
        resp.widget_info(|| widgets::icon_button_info(ui.is_enabled(), false, &tip));
        if resp.on_hover_text(tip).clicked() {
            save(app, &ctx);
        }
        let doc = app.session.active().is_some();
        if widgets::icon_button_enabled(ui, "dc-folder", tl!("Add to Swatches"), false, doc, 24.0).clicked() {
            add_draft_to_swatches(app, &ctx);
        }
        ui.add_space((ui.available_width() - 28.0).max(0.0));
        if widgets::icon_button(ui, "file-plus", tl!("New theme from the current color"), false, 24.0).clicked() {
            let base = active_paint(app).color().unwrap_or(DEFAULT_BASE);
            set_draft(&ctx, Draft::new(base, d.rule.or(Some(DEFAULT_RULE))));
        }
    });
}

/// What a click on the My Themes tab asks for.
enum Action {
    Pick(String),
    Edit(String),
}

fn my_themes(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    let pick = picked(app, &ctx);
    let mut action = None;
    if app.session.prefs.color_themes.is_empty() {
        empty_state(ui, "sun", tl!("No saved themes"), tl!("Make a theme on Create and save it."));
    } else {
        widgets::list_box(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("ct-list").max_height(300.0).auto_shrink([false, true]).show(ui, |ui| {
                for th in &app.session.prefs.color_themes {
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::hover());
                    let resp = ui.interact(r, ui.scope_id().with(("ct-theme", &th.name)), Sense::click());
                    let selected = pick.as_deref() == Some(th.name.as_str());
                    if selected {
                        ui.painter().rect_filled(r, 0.0, t.row_selected);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(r, 0.0, t.hover);
                    }
                    let inner = r.shrink2(vec2(6.0, 4.0));
                    ui.painter().text(inner.left_top(), egui::Align2::LEFT_TOP, &th.name, egui::FontId::proportional(12.0), t.text);
                    let strip = Rect::from_min_max(inner.left_bottom() - vec2(0.0, 18.0), inner.right_bottom());
                    let w = strip.width() / th.colors.len().max(1) as f32;
                    for (i, c) in th.colors.iter().enumerate() {
                        ui.painter().rect_filled(Rect::from_min_size(strip.min + vec2(i as f32 * w, 0.0), vec2(w, strip.height())), 0.0, c32(c));
                    }
                    ui.painter().rect_stroke(strip, 0.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Outside);
                    if resp.double_clicked() {
                        action = Some(Action::Edit(th.name.clone()));
                    } else if resp.on_hover_text(tl!("Click to select, double-click to edit")).clicked() {
                        action = Some(Action::Pick(th.name.clone()));
                    }
                }
            });
        });
    }
    match action {
        Some(Action::Pick(n)) => set_pstate(&ctx, "ct-picked", Some(n)),
        Some(Action::Edit(n)) => edit_saved(app, &ctx, &n),
        None => {}
    }
    let pick = picked(app, &ctx);
    widgets::bottom_bar(ui, |ui| {
        let some = pick.is_some();
        if widgets::icon_button_enabled(ui, "pencil", tl!("Edit theme"), false, some, 24.0).clicked()
            && let Some(n) = &pick
        {
            edit_saved(app, &ctx, n);
        }
        let doc = app.session.active().is_some();
        if widgets::icon_button_enabled(ui, "dc-folder", tl!("Add to Swatches"), false, some && doc, 24.0).clicked()
            && let Some(n) = &pick
        {
            app.run("colorTheme.addToSwatches", json!({ "name": n })).ok();
        }
        ui.add_space((ui.available_width() - 28.0).max(0.0));
        if widgets::icon_button_enabled(ui, "trash-2", tl!("Delete theme"), false, some, 24.0).clicked()
            && let Some(n) = &pick
        {
            delete(app, &ctx, n);
        }
    });
}

/// Edit saved theme `name` on the Create tab.
fn edit_saved(app: &VectorcraftApp, ctx: &egui::Context, name: &str) {
    if let Some(th) = app.session.prefs.color_themes.iter().find(|t| t.name == name) {
        edit(ctx, th);
    }
}

/// Delete saved theme `name`; the theme being made no longer edits it.
fn delete(app: &mut VectorcraftApp, ctx: &egui::Context, name: &str) {
    if app.run("colorTheme.delete", json!({ "name": name })).is_ok() {
        let mut d = draft(app, ctx);
        if d.editing.as_deref() == Some(name) {
            d.editing = None;
            set_draft(ctx, d);
        }
    }
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let doc = app.session.active().is_some();
    if tab(&ctx) == 1 {
        let pick = picked(app, &ctx);
        if menu_item(ui, tl!("Edit Theme"), pick.is_some(), false)
            && let Some(n) = &pick
        {
            edit_saved(app, &ctx, n);
        }
        if menu_item(ui, tl!("Add to Swatches"), pick.is_some() && doc, false)
            && let Some(n) = &pick
        {
            app.run("colorTheme.addToSwatches", json!({ "name": n })).ok();
        }
        if menu_item(ui, tl!("Delete Theme"), pick.is_some(), false)
            && let Some(n) = &pick
        {
            delete(app, &ctx, n);
        }
    } else {
        if menu_item(ui, tl!("Save Theme"), true, false) {
            save(app, &ctx);
        }
        if menu_item(ui, tl!("Add to Swatches"), doc, false) {
            add_draft_to_swatches(app, &ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app
    }

    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context) {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            show(app, ui);
            menu(app, ui);
        });
        out.textures_delta.clear();
    }

    fn themes(app: &VectorcraftApp) -> &[ColorTheme] {
        &app.session.prefs.color_themes
    }

    /// Screen readers name the Save button by its tool tip.
    #[test]
    fn screen_readers_name_the_save_button() {
        use crate::widgets::accessibility_tests::{role_and_state, tree};
        let mut app = app();
        let shown = tree(|ui| show(&mut app, ui));
        assert_eq!(role_and_state(&shown, "Save theme to My Themes"), (egui::accesskit::Role::Button, None));
    }

    #[test]
    fn create_save_edit_and_resave_a_theme() {
        let mut app = app();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        app.run("paint.setFill", json!({"color": "#cc4422"})).unwrap();
        frame(&mut app, &ctx);
        let d = draft(&app, &ctx);
        assert_eq!(d.colors, DEFAULT_RULE.theme(Color::from_hex("#cc4422").unwrap()), "a theme from the current colour");
        save(&mut app, &ctx);
        assert_eq!(themes(&app).len(), 1);
        assert_eq!((themes(&app)[0].name.as_str(), &themes(&app)[0].colors), ("Theme 1", &d.colors));
        assert_eq!(themes(&app)[0].rule.as_deref(), Some("analogous2"));
        assert_eq!(draft(&app, &ctx).editing.as_deref(), Some("Theme 1"), "it now edits the saved theme");
        // Another rule from the selected colour, then save: the same theme changes in place.
        let mut d = draft(&app, &ctx);
        d.sel = 2;
        let base = d.colors[2];
        d.rule = Some(Harmony::Triad);
        d.set_selected(base);
        assert_eq!((d.colors.clone(), d.sel), (Harmony::Triad.theme(base), 0));
        set_draft(&ctx, d.clone());
        save(&mut app, &ctx);
        assert_eq!(themes(&app).len(), 1, "saving an edited theme overwrites it");
        assert_eq!(themes(&app)[0].colors, d.colors);
        // Custom: only the selected colour changes; a new name saves a new theme.
        let mut d = Draft { rule: None, sel: 1, editing: None, name: "Mine".into(), ..d };
        d.set_selected(Color::from_hex("#00ff00").unwrap());
        assert_eq!(d.colors[1].to_hex(), "#00ff00");
        assert_eq!(d.colors[0], themes(&app)[0].colors[0]);
        set_draft(&ctx, d);
        save(&mut app, &ctx);
        assert_eq!(themes(&app).iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["Theme 1", "Mine"]);
        assert!(themes(&app)[1].rule.is_none());
        // Both tabs draw.
        set_pstate(&ctx, "ct-tab", 1usize);
        frame(&mut app, &ctx);
        assert_eq!(picked(&app, &ctx).as_deref(), Some("Mine"), "the theme just saved is picked");
    }

    #[test]
    fn my_themes_edit_add_to_swatches_and_delete() {
        let mut app = app();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        set_pstate(&ctx, "ct-tab", 1usize);
        frame(&mut app, &ctx);
        app.run("colorTheme.save", json!({"color": "#2266aa", "rule": "complementary", "name": "Harbor"})).unwrap();
        frame(&mut app, &ctx);
        set_pstate(&ctx, "ct-picked", Some("Harbor".to_string()));
        edit_saved(&app, &ctx, "Harbor");
        assert_eq!(tab(&ctx), 0, "editing shows the Create tab");
        let d = draft(&app, &ctx);
        assert_eq!((d.rule, d.editing.as_deref(), d.colors.len()), (Some(Harmony::Complementary), Some("Harbor"), 5));
        frame(&mut app, &ctx);
        // Add to Swatches (the picked saved theme), one colour group in one undo step.
        let undo = app.session.doc().unwrap().history.undo.len();
        app.run("colorTheme.addToSwatches", json!({"name": "Harbor"})).unwrap();
        let st = app.session.doc().unwrap();
        assert_eq!(st.history.undo.len(), undo + 1);
        assert_eq!(st.doc.swatch_groups.last().map(|g| (g.name.as_str(), g.swatches.len())), Some(("Harbor", 5)));
        // The theme being made goes to Swatches as it is.
        add_draft_to_swatches(&mut app, &ctx);
        assert_eq!(app.session.doc().unwrap().doc.swatch_groups.last().map(|g| g.name.as_str()), Some("Harbor 2"));
        // Deleting it frees the theme being made from editing it.
        delete(&mut app, &ctx, "Harbor");
        assert!(themes(&app).is_empty());
        assert_eq!(draft(&app, &ctx).editing, None);
        assert_eq!(picked(&app, &ctx), None);
        frame(&mut app, &ctx);
    }

    /// In a CMYK document the theme being made goes to Swatches with its colors in their own model.
    #[test]
    fn the_theme_being_made_keeps_its_model_in_a_cmyk_document() {
        let mut app = app();
        app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        frame(&mut app, &ctx);
        app.run("colorTheme.save", json!({"color": "#2266aa", "rule": "complementary", "name": "Harbor"})).unwrap();
        frame(&mut app, &ctx);
        edit_saved(&app, &ctx, "Harbor");
        let colors = draft(&app, &ctx).colors;
        assert!(colors.iter().all(|c| matches!(c, Color::Rgb { .. })), "{colors:?}");
        add_draft_to_swatches(&mut app, &ctx);
        let g = app.session.doc().unwrap().doc.swatch_groups.last().expect("the group");
        assert_eq!(g.swatches.iter().filter_map(|w| w.paint.color()).collect::<Vec<_>>(), colors);
    }
}
