//! Color Themes panel (local only): the Create tab makes a five-colour theme on a harmony wheel (a
//! harmony rule from the selected colour, or Custom: each colour moves on its own; with a rule the
//! colours move together; the selected color is also edited in its hex field or with CMYK, RGB, Lab
//! or HSB sliders), the My Themes tab lists the themes saved in the preferences (the panel
//! menu's Show Find Field adds a field that lists only the themes whose names hold its text). Themes are
//! saved, edited, deleted and added to Swatches as colour groups through `colorTheme.*`; the theme
//! being made is panel state.

use egui::{Rect, Sense, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_color::Color;
use vectorcraft_color::harmony::{Harmony, THEME_SIZE, move_on_wheel};
use vectorcraft_engine::cmd::colortheme::ColorTheme;

use super::color::{Mode, components, from_components, track_color};
use super::{active_paint, c32, color_json, empty_state, proxy_cmd, pstate, set_pstate};
use crate::VectorcraftApp;
use crate::dialogs::recolor::slider_cell;
use crate::theme::{self, Tokens};
use crate::widgets::{self, menu_item};

/// The rule a new theme starts with.
const DEFAULT_RULE: Harmony = Harmony::Analogous2;
/// The base colour before one is picked: a teal.
const DEFAULT_BASE: Color = Color::Rgb { r: 0.1, g: 0.55, b: 0.6 };
/// Diameter of the harmony wheel.
const WHEEL: f32 = 150.0;
/// The color modes of the selected color's sliders, in the order the Color Mode menu lists them.
const SLIDER_MODES: [Mode; 4] = [Mode::Cmyk, Mode::Rgb, Mode::Lab, Mode::Hsb];
/// Widths of a slider's label and of its number field (the field's width is the Recolor dialog's).
const SLIDER_LABEL_W: f32 = 12.0;
const SLIDER_FIELD_W: f32 = 48.0;

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

/// The saved theme picked on the My Themes tab, while the list shows it: a theme that the find
/// field hides stays picked, but nothing acts on it until the list shows it again.
fn picked(app: &VectorcraftApp, ctx: &egui::Context) -> Option<String> {
    let q = query(ctx);
    pstate::<Option<String>>(ctx, "ct-picked").filter(|n| app.session.prefs.color_themes.iter().any(|t| t.name == *n && matches(&t.name, &q)))
}

/// The My Themes find field's text, kept while the panel menu hides the field.
fn find_id() -> egui::Id {
    egui::Id::unique("ct-find")
}

/// Whether the panel menu's Show Find Field shows the find field (off in a new session).
fn show_find(ctx: &egui::Context) -> bool {
    pstate(ctx, "ct-show-find")
}

/// The find field's text, trimmed and lowercased; empty while the field is hidden.
fn query(ctx: &egui::Context) -> String {
    if !show_find(ctx) {
        return String::new();
    }
    ctx.data(|d| d.get_temp::<String>(find_id())).unwrap_or_default().trim().to_lowercase()
}

/// Whether the find field lists theme `name` for `query` (trimmed and lowercased): its name holds
/// the text anywhere, in any letter case. An empty query lists every theme.
fn matches(name: &str, query: &str) -> bool {
    query.is_empty() || name.to_lowercase().contains(query)
}

/// The tab shown: 0 Create, 1 My Themes.
fn tab(ctx: &egui::Context) -> usize {
    pstate(ctx, "ct-tab")
}

/// The color mode of the sliders: the one last picked from their Color Mode menu while the app
/// runs, or else the selected color's own model (HSB for a gray). Picking a mode changes no color.
fn slider_mode(ctx: &egui::Context, c: Option<&Color>) -> Mode {
    pstate::<Option<Mode>>(ctx, "ct-mode").unwrap_or_else(|| match c.map(Mode::of) {
        Some(m @ (Mode::Cmyk | Mode::Rgb | Mode::Lab)) => m,
        _ => Mode::Hsb,
    })
}

/// The slider values a slider edit last set: the selected place and the color they made in
/// `mode`. While those still hold, the sliders show `v` as it was set, so an edit of one slider
/// leaves the others' numbers as they were instead of converting them back from the color.
#[derive(Clone, Debug, PartialEq)]
struct Held {
    sel: usize,
    color: Color,
    mode: Mode,
    v: Vec<f32>,
}

/// The numbers the sliders show for the selected color of `d` in `mode`.
fn slider_values(ctx: &egui::Context, d: &Draft, mode: Mode) -> Option<Vec<f32>> {
    let c = d.colors.get(d.sel)?;
    let held =
        pstate::<Option<Held>>(ctx, "ct-held").filter(|h| h.sel == d.sel && h.color == *c && h.mode == mode && h.v.len() == mode.labels().len());
    Some(held.map_or_else(|| components(mode, c), |h| h.v))
}

/// The Color Mode menu (CMYK, RGB, Lab and HSB) and its sliders, each a rail and a number field, for
/// the selected color of `d`. An edit sets the selected color in the menu's mode (an RGB or HSB
/// edit makes it RGB, a CMYK edit CMYK and a Lab edit Lab) as the hex field does: with a rule the
/// theme follows from it as the new base.
fn sliders(ui: &mut Ui, d: &mut Draft) {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    let current = slider_mode(&ctx, d.colors.get(d.sel));
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Color Mode:")).color(t.text_dim));
        let labels: Vec<&str> = SLIDER_MODES.iter().map(|m| m.label()).collect();
        if let Some(&m) =
            widgets::dropdown_named(ui, "ct-mode", Some(tl!("Color Mode:")), current.label(), &labels, 90.0).and_then(|i| SLIDER_MODES.get(i))
        {
            set_pstate(&ctx, "ct-mode", Some(m));
        }
    });
    ui.add_space(2.0);
    let mode = slider_mode(&ctx, d.colors.get(d.sel));
    let Some(comps) = slider_values(&ctx, d, mode) else { return };
    let spacing = 6.0;
    let rail = (ui.available_width() - SLIDER_LABEL_W - SLIDER_FIELD_W - 2.0 * spacing - 4.0).max(40.0);
    let mut set: Option<Vec<f32>> = None;
    egui::Grid::new("ct-sliders").num_columns(3).spacing([spacing, 4.0]).show(ui, |ui| {
        for (n, lbl) in mode.labels().iter().enumerate() {
            let v = comps.get(n).copied().unwrap_or(0.0);
            let shown = widgets::SliderValue::field(f64::from(v), f64::from(mode.min(n))..=f64::from(mode.max(n)), mode.suffix(n), 0);
            let track = |x: f32| c32(&track_color(mode, &comps, n, x));
            if let Some(nv) = slider_cell(ui, ("ct", mode.label(), n), lbl, shown, mode.suffix(n), &track, SLIDER_LABEL_W, rail) {
                let mut c2 = comps.clone();
                c2[n] = nv as f32;
                set = Some(c2);
            }
            ui.end_row();
        }
    });
    if let Some(v) = set {
        d.set_selected(from_components(mode, &v));
        if let Some(&color) = d.colors.get(d.sel) {
            set_pstate(&ctx, "ct-held", Some(Held { sel: d.sel, color, mode, v }));
        }
    }
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

/// Paint the active proxy (Fill or Stroke) with the selected colour of the theme being made, in its
/// own model: the selection and the new-art default take it in one paint command, so one undo
/// step, and no step when the art already has it. With characters selected with the Type tool,
/// only those characters take it and the new-art default stays as it was, as with a swatch click.
/// The theme stays as it is.
fn set_active_color(app: &mut VectorcraftApp, d: &Draft) {
    if let Some(c) = d.colors.get(d.sel) {
        app.run(proxy_cmd(app, false), json!({ "color": color_json(c), "keepModel": true })).ok();
    }
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
    sliders(ui, &mut d);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let hex = d.colors.get(d.sel).map(|c| c.to_hex().trim_start_matches('#').to_string()).unwrap_or_default();
        if let Some(c) = widgets::hex_field_named(ui, "ct-hex", Some(tl!("Color")), &hex).and_then(|h| Color::from_hex(&h)) {
            d.set_selected(c);
        }
        let can_set = app.session.active().is_some() && d.colors.get(d.sel).is_some();
        let tip = tl!("Set the current fill or stroke to the selected color");
        if widgets::icon_button_enabled(ui, "paint-bucket", tip, false, can_set, 24.0).clicked() {
            set_active_color(app, &d);
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
    // The find field lists the saved themes whose names hold its text, in their saved order.
    if show_find(&ctx) {
        widgets::search_field(ui, find_id(), tl!("Find"));
        ui.add_space(4.0);
    }
    let q = query(&ctx);
    let pick = picked(app, &ctx);
    let mut action = None;
    if app.session.prefs.color_themes.is_empty() {
        empty_state(ui, "sun", tl!("No saved themes"), tl!("Make a theme on Create and save it."));
    } else {
        widgets::list_box(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("ct-list").max_height(300.0).auto_shrink([false, true]).show(ui, |ui| {
                let mut shown = app.session.prefs.color_themes.iter().filter(|th| matches(&th.name, &q)).peekable();
                if shown.peek().is_none() {
                    widgets::dim_label(ui, tl!("No matches."));
                }
                for th in shown {
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
        ui.separator();
        let find = show_find(&ctx);
        if menu_item(ui, tl!("Show Find Field"), true, find) {
            set_pstate(&ctx, "ct-show-find", !find);
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

    /// The texts one frame draws: the panel, or with `menu_only` its panel menu, with `events`.
    fn drawn(app: &mut VectorcraftApp, ctx: &egui::Context, menu_only: bool, events: Vec<egui::Event>) -> Vec<(String, Rect)> {
        crate::i18n::set_current(crate::i18n::Lang::EN);
        let screen = Rect::from_min_size(egui::Pos2::ZERO, vec2(320.0, 700.0));
        let input = egui::RawInput { events, screen_rect: Some(screen), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| if menu_only { menu(app, ui) } else { show(app, ui) });
        out.textures_delta.clear();
        fn texts(s: &egui::Shape, out: &mut Vec<(String, Rect)>) {
            match s {
                egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect())),
                egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        let mut found = vec![];
        out.shapes.iter().for_each(|s| texts(&s.shape, &mut found));
        found
    }

    /// The saved themes the My Themes list draws, in order.
    fn listed(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<String> {
        drawn(app, ctx, false, vec![]);
        let names: Vec<String> = themes(app).iter().map(|t| t.name.clone()).collect();
        drawn(app, ctx, false, vec![]).into_iter().map(|(t, _)| t).filter(|t| names.contains(t)).collect()
    }

    /// Click panel menu item `label` (with its check mark, or three spaces when unchecked).
    fn click_menu(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str) {
        let texts = drawn(app, ctx, true, vec![]);
        let at = texts.iter().find(|(t, _)| t == label).unwrap_or_else(|| panic!("no {label:?} in {texts:?}")).1.center();
        let button =
            |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        drawn(app, ctx, true, vec![egui::Event::PointerMoved(at), button(true), button(false)]);
    }

    /// My Themes with three saved themes and the find field holding `text`.
    fn three_themes(text: &str) -> (VectorcraftApp, egui::Context) {
        let mut app = app();
        for (name, color) in [("Harbor", "#2266aa"), ("Harvest", "#cc8822"), ("Forest", "#227744")] {
            app.run("colorTheme.save", json!({"color": color, "rule": "complementary", "name": name})).unwrap();
        }
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        set_pstate(&ctx, "ct-tab", 1usize);
        set_pstate(&ctx, "ct-show-find", true);
        ctx.data_mut(|d| d.insert_temp(find_id(), text.to_string()));
        (app, ctx)
    }

    /// Show Find Field is a checked item of the My Themes menu only, off in a new session; each
    /// choice turns the field on or off.
    #[test]
    fn show_find_field_toggles_the_find_field_on_my_themes() {
        let mut app = app();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let has = |texts: &[(String, Rect)], label: &str| texts.iter().any(|(t, _)| t == label);
        let texts = drawn(&mut app, &ctx, true, vec![]);
        assert!(!has(&texts, "   Show Find Field") && !has(&texts, "✓ Show Find Field"), "the Create tab has no find field");
        set_pstate(&ctx, "ct-tab", 1usize);
        assert!(has(&drawn(&mut app, &ctx, true, vec![]), "   Show Find Field"), "off in a new session");
        assert!(!has(&drawn(&mut app, &ctx, false, vec![]), "Find"), "no field while it is off");
        click_menu(&mut app, &ctx, "   Show Find Field");
        assert!(show_find(&ctx));
        assert!(has(&drawn(&mut app, &ctx, true, vec![]), "✓ Show Find Field"));
        assert!(has(&drawn(&mut app, &ctx, false, vec![]), "Find"), "the field shows its hint");
        click_menu(&mut app, &ctx, "✓ Show Find Field");
        assert!(!show_find(&ctx));
    }

    /// The find field lists the themes whose names hold its text, in any letter case, anywhere in
    /// the name, in their saved order; it reads names only, and its text is plain text.
    #[test]
    fn the_find_field_lists_themes_whose_names_hold_its_text() {
        let (mut app, ctx) = three_themes("  HAR ");
        assert_eq!(listed(&mut app, &ctx), ["Harbor", "Harvest"]);
        ctx.data_mut(|d| d.insert_temp(find_id(), "vest".to_string()));
        assert_eq!(listed(&mut app, &ctx), ["Harvest"], "a match in the middle of a name");
        // Colours are not searched: no name holds "2266aa".
        ctx.data_mut(|d| d.insert_temp(find_id(), "2266aa".to_string()));
        assert!(listed(&mut app, &ctx).is_empty());
        assert!(drawn(&mut app, &ctx, false, vec![]).iter().any(|(t, _)| t == "No matches."));
        assert_eq!(themes(&app).len(), 3, "the saved themes stay");
        // Pattern characters are plain text, and letters outside ASCII match in either case.
        app.run("colorTheme.save", json!({"color": "#aa2266", "rule": "complementary", "name": "Été a.b*"})).unwrap();
        ctx.data_mut(|d| d.insert_temp(find_id(), "ÉTÉ A.B*".to_string()));
        assert_eq!(listed(&mut app, &ctx), ["Été a.b*"]);
        ctx.data_mut(|d| d.insert_temp(find_id(), ".*".to_string()));
        assert!(listed(&mut app, &ctx).is_empty());
        // Blank text, or the field turned off, lists every theme; turned on again the text applies.
        ctx.data_mut(|d| d.insert_temp(find_id(), "   ".to_string()));
        assert_eq!(listed(&mut app, &ctx).len(), 4);
        ctx.data_mut(|d| d.insert_temp(find_id(), "har".to_string()));
        set_pstate(&ctx, "ct-show-find", false);
        assert_eq!(listed(&mut app, &ctx).len(), 4);
        set_pstate(&ctx, "ct-show-find", true);
        assert_eq!(listed(&mut app, &ctx), ["Harbor", "Harvest"]);
    }

    /// With no saved themes the panel says so, whatever the find field holds.
    #[test]
    fn no_saved_themes_is_not_no_matches() {
        let mut app = app();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        set_pstate(&ctx, "ct-tab", 1usize);
        set_pstate(&ctx, "ct-show-find", true);
        ctx.data_mut(|d| d.insert_temp(find_id(), "har".to_string()));
        drawn(&mut app, &ctx, false, vec![]);
        let texts: Vec<String> = drawn(&mut app, &ctx, false, vec![]).into_iter().map(|(t, _)| t).collect();
        assert!(texts.iter().any(|t| t == "No saved themes"), "{texts:?}");
        assert!(!texts.iter().any(|t| t == "No matches."));
    }

    /// A picked theme that the find field hides stays picked, but neither the menu nor the footer
    /// acts on it until the list shows it again.
    #[test]
    fn a_theme_the_find_field_hides_is_not_acted_on() {
        let (mut app, ctx) = three_themes("har");
        set_pstate(&ctx, "ct-picked", Some("Forest".to_string()));
        assert_eq!(picked(&app, &ctx), None);
        drawn(&mut app, &ctx, true, vec![]);
        click_menu(&mut app, &ctx, "   Delete Theme");
        assert_eq!(themes(&app).len(), 3, "Delete Theme is off");
        click_menu(&mut app, &ctx, "   Edit Theme");
        assert_eq!(tab(&ctx), 1, "Edit Theme is off");
        let undo = undo_len(&app);
        click_menu(&mut app, &ctx, "   Add to Swatches");
        assert_eq!(undo_len(&app), undo, "Add to Swatches is off");
        // Cleared, the field lists Forest again and it is picked as before.
        ctx.data_mut(|d| d.insert_temp(find_id(), String::new()));
        assert_eq!(picked(&app, &ctx).as_deref(), Some("Forest"));
        ctx.data_mut(|d| d.insert_temp(find_id(), "har".to_string()));
        set_pstate(&ctx, "ct-show-find", false);
        assert_eq!(picked(&app, &ctx).as_deref(), Some("Forest"), "the field turned off lists it too");
        set_pstate(&ctx, "ct-show-find", true);
        // A theme the field shows is acted on.
        set_pstate(&ctx, "ct-picked", Some("Harvest".to_string()));
        click_menu(&mut app, &ctx, "   Delete Theme");
        assert_eq!(themes(&app).iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["Harbor", "Forest"]);
    }

    /// The footer buttons are off for a picked theme that the find field hides.
    #[test]
    fn footer_buttons_are_off_for_a_hidden_pick() {
        use crate::widgets::accessibility_tests::{named, tree};
        let mut app = app();
        app.run("colorTheme.save", json!({"color": "#227744", "rule": "complementary", "name": "Forest"})).unwrap();
        for (text, off) in [("har", true), ("for", false)] {
            let shown = tree(|ui| {
                let ctx = ui.ctx().clone();
                set_pstate(&ctx, "ct-tab", 1usize);
                set_pstate(&ctx, "ct-show-find", true);
                set_pstate(&ctx, "ct-picked", Some("Forest".to_string()));
                ctx.data_mut(|d| d.insert_temp(find_id(), text.to_string()));
                show(&mut app, ui);
            });
            for label in ["Edit theme", "Add to Swatches", "Delete theme"] {
                assert_eq!(named(&shown, label).is_disabled(), off, "{label} with {text:?}");
            }
        }
    }

    const SET_ACTIVE: &str = "Set the current fill or stroke to the selected color";

    /// A click on the panel button that screen readers name `label`. egui finds what the pointer is
    /// over from the frame before, so the move gets a frame of its own.
    fn click(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str) {
        crate::i18n::set_current(crate::i18n::Lang::EN);
        ctx.enable_accesskit();
        let mut time = ctx.input(|i| i.time);
        let mut run = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            time += 0.1;
            let screen = Rect::from_min_size(egui::Pos2::ZERO, vec2(320.0, 520.0));
            let input = egui::RawInput { events, time: Some(time), screen_rect: Some(screen), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| show(app, ui));
            out.textures_delta.clear();
            out
        };
        run(app, vec![]);
        let out = run(app, vec![]);
        let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
        let node = nodes.iter().find(|(_, n)| n.label() == Some(label));
        let b = node.unwrap_or_else(|| panic!("nothing named {label:?}")).1.bounds().expect("bounds");
        let at = egui::pos2((b.x0 + b.x1) as f32 * 0.5, (b.y0 + b.y1) as f32 * 0.5);
        let button =
            |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        run(app, vec![egui::Event::PointerMoved(at)]);
        run(app, vec![button(true)]);
        run(app, vec![button(false)]);
    }

    /// A Custom theme of five different colours with the third selected.
    fn five_colors(ctx: &egui::Context) -> Draft {
        let colors = ["#cc2200", "#22aa44", "#3355dd", "#ddbb11", "#884499"].map(|h| Color::from_hex(h).unwrap()).to_vec();
        let d = Draft { colors, rule: None, sel: 2, name: "Mine".into(), editing: None };
        set_draft(ctx, d.clone());
        d
    }

    /// The fill and stroke paints of object `id`.
    fn paints(app: &VectorcraftApp, id: vectorcraft_doc::NodeId) -> (vectorcraft_color::Paint, vectorcraft_color::Paint) {
        let n = app.session.doc().unwrap().doc.node(id).unwrap();
        (n.appearance.fill().unwrap().paint.clone(), n.appearance.stroke().unwrap().paint.clone())
    }

    fn undo_len(app: &VectorcraftApp) -> usize {
        app.session.doc().unwrap().history.undo.len()
    }

    /// Screen readers name the Set Active Color button by its tool tip; it is off without a document.
    #[test]
    fn set_active_color_is_named_and_needs_a_document() {
        use crate::widgets::accessibility_tests::{named, tree};
        let mut app = app();
        let shown = tree(|ui| show(&mut app, ui));
        assert_eq!(named(&shown, SET_ACTIVE).role(), egui::accesskit::Role::Button);
        assert!(!named(&shown, SET_ACTIVE).is_disabled());
        let mut none = VectorcraftApp::new(Session::new(), Default::default());
        let shown = tree(|ui| show(&mut none, ui));
        assert!(named(&shown, SET_ACTIVE).is_disabled(), "no document: nothing to paint");
    }

    /// Set Active Color paints the active proxy of the selection with the selected theme colour,
    /// one undo step a click, none when the art already has it, and leaves the theme as it is.
    #[test]
    fn set_active_color_paints_the_active_proxy_with_the_selected_color() {
        let mut app = app();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
        let id = app.session.doc().unwrap().selection.objects[0];
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let d = five_colors(&ctx);
        let (_, stroke) = paints(&app, id);
        let undo = undo_len(&app);
        click(&mut app, &ctx, SET_ACTIVE);
        assert_eq!(paints(&app, id), (vectorcraft_color::Paint::solid(d.colors[2]), stroke.clone()), "the selected colour, not the first");
        assert_eq!(undo_len(&app), undo + 1, "one undo step");
        click(&mut app, &ctx, SET_ACTIVE);
        assert_eq!(undo_len(&app), undo + 1, "no step when the fill already has the colour");
        // With the Stroke proxy in front, the stroke takes the colour and the fill stays.
        app.run("paint.toggleActive", json!({"fill": false})).unwrap();
        set_draft(&ctx, Draft { sel: 3, ..d.clone() });
        click(&mut app, &ctx, SET_ACTIVE);
        let solid = |i: usize| vectorcraft_color::Paint::solid(d.colors[i]);
        assert_eq!(paints(&app, id), (solid(2), solid(3)));
        assert_eq!(undo_len(&app), undo + 2);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(paints(&app, id), (solid(2), stroke.clone()), "one undo takes the stroke back");
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(paints(&app, id), (solid(2), solid(3)));
        // The theme being made and the saved themes stay as they were.
        assert_eq!(draft(&app, &ctx), Draft { sel: 3, ..d.clone() });
        assert!(themes(&app).is_empty());
        // The reverse pipette paints nothing.
        let undo = undo_len(&app);
        click(&mut app, &ctx, "Set the selected color to the current color");
        assert_eq!((paints(&app, id), undo_len(&app)), ((solid(2), solid(3)), undo));
    }

    /// With nothing selected, Set Active Color sets the new-art default alone, without an undo step,
    /// and new art takes it.
    #[test]
    fn set_active_color_with_nothing_selected_sets_the_default_for_new_art() {
        let mut app = app();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let d = five_colors(&ctx);
        let undo = undo_len(&app);
        click(&mut app, &ctx, SET_ACTIVE);
        assert_eq!(app.session.paint.fill, vectorcraft_color::Paint::solid(d.colors[2]));
        assert_eq!(undo_len(&app), undo, "no art changed");
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
        let id = app.session.doc().unwrap().selection.objects[0];
        assert_eq!(paints(&app, id).0, vectorcraft_color::Paint::solid(d.colors[2]));
    }

    /// In a CMYK document Set Active Color keeps an RGB theme colour in RGB.
    #[test]
    fn set_active_color_keeps_the_model_in_a_cmyk_document() {
        let mut app = app();
        app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
        let id = app.session.doc().unwrap().selection.objects[0];
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let d = five_colors(&ctx);
        click(&mut app, &ctx, SET_ACTIVE);
        assert_eq!(paints(&app, id).0.color(), Some(d.colors[2]));
        assert!(matches!(d.colors[2], Color::Rgb { .. }));
    }

    /// With characters selected with the Type tool, Set Active Color paints those characters as a
    /// swatch click does: the type and the undo history come out as they do from `paint.setFill` with
    /// the same color, and the new-art default stays as it was.
    #[test]
    fn set_active_color_paints_selected_characters_as_a_swatch_click_does() {
        use vectorcraft_doc::{NodeId, NodeKind};
        // The editing handshake of text_styles::tests::select_type_range.
        let typed = || {
            let mut app = app();
            let id = NodeId(app.run("text.create", json!({"x": 10, "y": 50, "text": "ABcdef", "size": 18})).unwrap()["id"].as_u64().unwrap());
            app.session.select_tool("type", Default::default()).unwrap();
            app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
            app.session.set_tool_option("select", &json!({"start": 2, "end": 4}));
            super::super::character::end_typing(&mut app);
            assert_eq!(super::super::character::text_editing(&app), Some((id, 2, 4)));
            (app, id)
        };
        let runs = |app: &VectorcraftApp, id: NodeId| match &app.session.doc().unwrap().doc.node(id).unwrap().kind {
            NodeKind::Text(t) => t.runs.clone(),
            _ => panic!("not type"),
        };
        let (mut by_button, id) = typed();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let d = five_colors(&ctx);
        let undo = undo_len(&by_button);
        let default = (by_button.session.paint.fill.clone(), by_button.session.paint.stroke.clone());
        assert_ne!(default.0, vectorcraft_color::Paint::solid(d.colors[2]));
        click(&mut by_button, &ctx, SET_ACTIVE);
        let (mut by_command, _) = typed();
        by_command.run("paint.setFill", json!({"color": color_json(&d.colors[2]), "keepModel": true})).unwrap();
        let painted = runs(&by_button, id);
        assert!(painted.iter().any(|r| r.style.fill == vectorcraft_color::Paint::solid(d.colors[2])), "the characters take the color");
        assert!(painted.iter().any(|r| r.style.fill != vectorcraft_color::Paint::solid(d.colors[2])), "the other characters keep theirs");
        assert_eq!(painted, runs(&by_command, id));
        assert_eq!((&by_button.session.paint.fill, &by_button.session.paint.stroke), (&default.0, &default.1), "the default stays");
        assert_eq!(
            (&by_button.session.paint.fill, &by_button.session.paint.stroke),
            (&by_command.session.paint.fill, &by_command.session.paint.stroke)
        );
        assert_eq!((undo_len(&by_button), undo_len(&by_command)), (undo + 1, undo + 1));
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

    /// One frame of the panel in a 260 × 700 window with `events`, in English, with screen readers on.
    fn step(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
        crate::i18n::set_current(crate::i18n::Lang::EN);
        ctx.enable_accesskit();
        let time = ctx.input(|i| i.time) + 0.1;
        let screen = Rect::from_min_size(egui::Pos2::ZERO, vec2(260.0, 700.0));
        let input = egui::RawInput { events, time: Some(time), screen_rect: Some(screen), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| show(app, ui));
        out.textures_delta.clear();
        out
    }

    /// The screen-reader nodes of the frame after a settling one.
    fn nodes(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<(egui::accesskit::NodeId, egui::accesskit::Node)> {
        step(app, ctx, vec![]);
        step(app, ctx, vec![]).platform_output.accesskit_update.expect("an accessibility tree").nodes
    }

    /// The labels of the color sliders the Create tab shows (named by one letter, unlike the wheel's
    /// Brightness slider), with each one's range, in order.
    fn slider_ranges(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<(String, f64, f64)> {
        let mut found: Vec<(f64, String, f64, f64)> = nodes(app, ctx)
            .into_iter()
            .filter(|(_, n)| n.role() == egui::accesskit::Role::Slider && n.label().is_some_and(|l| l.chars().count() == 1))
            .filter_map(|(_, n)| {
                let top = n.bounds()?.y0;
                Some((top, n.label()?.to_string(), n.min_numeric_value()?, n.max_numeric_value()?))
            })
            .collect();
        found.sort_by(|a, b| a.0.total_cmp(&b.0));
        found.into_iter().map(|(_, l, lo, hi)| (l, lo, hi)).collect()
    }

    /// The node id and bounds of the screen-reader node named `label` whose role is (with `rail`) or
    /// is not a slider.
    fn node(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str, rail: bool) -> (egui::accesskit::NodeId, egui::Pos2) {
        let all = nodes(app, ctx);
        let (id, n) = all
            .iter()
            .find(|(_, n)| n.label() == Some(label) && (n.role() == egui::accesskit::Role::Slider) == rail)
            .unwrap_or_else(|| panic!("nothing named {label:?}"));
        let b = n.bounds().expect("bounds");
        (*id, egui::pos2((b.x0 + b.x1) as f32 * 0.5, (b.y0 + b.y1) as f32 * 0.5))
    }

    /// A screen reader's set-value action on the slider named `label`.
    fn set_slider(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str, value: f64) {
        use egui::accesskit::{Action, ActionData, ActionRequest, TreeId};
        let (target_node, _) = node(app, ctx, label, true);
        let data = Some(ActionData::NumericValue(value));
        step(
            app,
            ctx,
            vec![egui::Event::AccessKitActionRequest(ActionRequest { action: Action::SetValue, target_node, target_tree: TreeId::ROOT, data })],
        );
        step(app, ctx, vec![]);
    }

    /// A click at `at`, each event in a frame of its own.
    fn press_at(app: &mut VectorcraftApp, ctx: &egui::Context, at: egui::Pos2) {
        let button =
            |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        step(app, ctx, vec![egui::Event::PointerMoved(at)]);
        step(app, ctx, vec![button(true)]);
        step(app, ctx, vec![button(false)]);
    }

    fn key(app: &mut VectorcraftApp, ctx: &egui::Context, key: egui::Key, modifiers: egui::Modifiers) {
        step(app, ctx, vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }]);
        step(app, ctx, vec![egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers }]);
    }

    /// Type `text` into the number field named `label` in place of its number, and press Enter.
    fn type_into(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str, text: &str) {
        let (_, at) = node(app, ctx, label, false);
        press_at(app, ctx, at);
        key(app, ctx, egui::Key::A, egui::Modifiers::COMMAND);
        step(app, ctx, vec![egui::Event::Text(text.into())]);
        key(app, ctx, egui::Key::Enter, egui::Modifiers::NONE);
        step(app, ctx, vec![]);
    }

    /// The Create tab offers a Color Mode menu of CMYK, RGB, Lab and HSB with a rail and a number
    /// field for each component over its range, beside the hex field. It starts in the selected
    /// color's own model (HSB for a gray), keeps the mode picked while the app runs, and picking a
    /// mode changes no color.
    #[test]
    fn the_create_tab_offers_cmyk_rgb_lab_and_hsb_sliders() {
        let mut app = app();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut d = five_colors(&ctx);
        let named = |app: &mut VectorcraftApp, ctx: &egui::Context, label: &str| nodes(app, ctx).iter().any(|(_, n)| n.label() == Some(label));
        assert!(named(&mut app, &ctx, "Color Mode:"), "the menu is named");
        assert!(named(&mut app, &ctx, "Color"), "the hex field stays");
        let rgb = vec![("R".to_string(), 0.0, 255.0), ("G".into(), 0.0, 255.0), ("B".into(), 0.0, 255.0)];
        assert_eq!(slider_ranges(&mut app, &ctx), rgb, "an RGB color starts in RGB");
        for (c, first) in [(Color::cmyk(0.1, 0.2, 0.3, 0.4), "C"), (Color::lab(50.0, -20.0, 30.0), "L"), (Color::gray(0.5), "H")] {
            d.colors[2] = c;
            set_draft(&ctx, d.clone());
            assert_eq!(slider_ranges(&mut app, &ctx)[0].0, first, "{c:?}");
        }
        let expect = [
            vec![("C", 0.0, 100.0), ("M", 0.0, 100.0), ("Y", 0.0, 100.0), ("K", 0.0, 100.0)],
            vec![("R", 0.0, 255.0), ("G", 0.0, 255.0), ("B", 0.0, 255.0)],
            vec![("L", 0.0, 100.0), ("a", -128.0, 127.0), ("b", -128.0, 127.0)],
            vec![("H", 0.0, 360.0), ("S", 0.0, 100.0), ("B", 0.0, 100.0)],
        ];
        for (m, rows) in SLIDER_MODES.into_iter().zip(expect) {
            set_pstate(&ctx, "ct-mode", Some(m));
            let rows: Vec<(String, f64, f64)> = rows.into_iter().map(|(l, lo, hi)| (l.to_string(), lo, hi)).collect();
            assert_eq!(slider_ranges(&mut app, &ctx), rows, "{m:?}");
            assert_eq!(draft(&app, &ctx), d, "picking a mode changes no color");
        }
        // The picked mode stays when another color is selected.
        set_draft(&ctx, Draft { sel: 0, ..d.clone() });
        assert_eq!(slider_ranges(&mut app, &ctx)[0].0, "H");
    }

    /// In Custom, a rail, the arrow keys and the number field change the selected color alone, in
    /// the menu's mode, and the selection stays; the other numbers stay as they were set.
    #[test]
    fn a_slider_edit_changes_only_the_selected_custom_color() {
        let mut app = app();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let d = five_colors(&ctx);
        let start = components(Mode::Rgb, &d.colors[2]);
        set_slider(&mut app, &ctx, "R", 10.0);
        let now = draft(&app, &ctx);
        assert_eq!(now.colors[2], from_components(Mode::Rgb, &[10.0, start[1], start[2]]));
        assert_eq!(now.sel, 2);
        for i in [0, 1, 3, 4] {
            assert_eq!(now.colors[i], d.colors[i], "color {i} stays");
        }
        // The arrow keys step the focused rail by one.
        let (_, at) = node(&mut app, &ctx, "G", true);
        press_at(&mut app, &ctx, at);
        let held = pstate::<Option<Held>>(&ctx, "ct-held").expect("held").v;
        key(&mut app, &ctx, egui::Key::ArrowRight, egui::Modifiers::NONE);
        let after = pstate::<Option<Held>>(&ctx, "ct-held").expect("held").v;
        assert_eq!(after, vec![10.0, held[1] + 1.0, start[2]]);
        // The number field.
        type_into(&mut app, &ctx, "B", "40");
        let now = draft(&app, &ctx);
        assert_eq!(now.colors[2], from_components(Mode::Rgb, &[10.0, held[1] + 1.0, 40.0]));
        assert_eq!((now.sel, now.rule), (2, None));
        assert_eq!(now.colors[0], d.colors[0]);
    }

    /// The sliders keep the numbers they set: an HSB color taken to no saturation keeps its hue on
    /// the H slider, and S back up brings that hue back. Another color shows its own numbers.
    #[test]
    fn the_sliders_keep_the_numbers_they_set() {
        let mut app = app();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        five_colors(&ctx);
        set_pstate(&ctx, "ct-mode", Some(Mode::Hsb));
        let hue = |app: &mut VectorcraftApp, ctx: &egui::Context| {
            let (id, _) = node(app, ctx, "H", true);
            nodes(app, ctx).into_iter().find(|(n, _)| *n == id).and_then(|(_, n)| n.numeric_value())
        };
        set_slider(&mut app, &ctx, "H", 200.0);
        set_slider(&mut app, &ctx, "S", 0.0);
        assert_eq!(hue(&mut app, &ctx), Some(200.0), "a gray keeps the hue it was set to");
        set_slider(&mut app, &ctx, "S", 80.0);
        let [h, s, _] = draft(&app, &ctx).colors[2].to_hsb();
        assert!((h - 200.0).abs() < 0.5 && (s - 0.8).abs() < 1e-3, "{h} {s}");
        set_draft(&ctx, Draft { sel: 0, ..draft(&app, &ctx) });
        let own = components(Mode::Hsb, &draft(&app, &ctx).colors[0])[0].round();
        assert_eq!(hue(&mut app, &ctx), Some(f64::from(own)), "another color shows its own numbers");
    }

    /// With a rule, a slider edit makes the edited color the theme's new base, as a hex edit does,
    /// and selects it.
    #[test]
    fn with_a_rule_a_slider_edit_rebuilds_the_theme_from_the_edited_color() {
        let mut app = app();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let d = Draft { rule: Some(Harmony::Triad), ..five_colors(&ctx) };
        set_draft(&ctx, d.clone());
        set_pstate(&ctx, "ct-mode", Some(Mode::Hsb));
        let start = components(Mode::Hsb, &d.colors[2]);
        set_slider(&mut app, &ctx, "H", 200.0);
        let base = from_components(Mode::Hsb, &[200.0, start[1], start[2]]);
        let now = draft(&app, &ctx);
        assert_eq!((now.colors, now.sel), (Harmony::Triad.theme(base), 0));
    }

    /// A CMYK edit keeps the color CMYK and a Lab edit keeps it Lab, with the numbers set, through
    /// Save and Add to Swatches in a CMYK document; a number that is not one, or not finite, changes
    /// nothing, and one past the range sets its end.
    #[test]
    fn cmyk_and_lab_slider_edits_keep_their_models() {
        let mut app = app();
        app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let d = five_colors(&ctx);
        set_pstate(&ctx, "ct-mode", Some(Mode::Cmyk));
        let start = components(Mode::Cmyk, &d.colors[2]);
        set_slider(&mut app, &ctx, "C", 20.0);
        set_slider(&mut app, &ctx, "K", 40.0);
        let cmyk = from_components(Mode::Cmyk, &[20.0, start[1], start[2], 40.0]);
        assert_eq!(draft(&app, &ctx).colors[2], cmyk);
        assert!(matches!(cmyk, Color::Cmyk { k, .. } if (k - 0.4).abs() < 1e-6));
        // Lab on another color.
        set_draft(&ctx, Draft { sel: 3, ..draft(&app, &ctx) });
        set_pstate(&ctx, "ct-mode", Some(Mode::Lab));
        let start = components(Mode::Lab, &d.colors[3]);
        set_slider(&mut app, &ctx, "b", -60.0);
        let lab = Color::lab(start[0], start[1], -60.0);
        assert_eq!(draft(&app, &ctx).colors[3], lab);
        // Not numbers, not finite, past the range.
        let before = draft(&app, &ctx);
        type_into(&mut app, &ctx, "L", "abc");
        set_slider(&mut app, &ctx, "a", f64::NAN);
        assert_eq!(draft(&app, &ctx), before);
        set_slider(&mut app, &ctx, "a", 500.0);
        assert_eq!(draft(&app, &ctx).colors[3], Color::lab(start[0], 127.0, -60.0));
        let colors = draft(&app, &ctx).colors;
        save(&mut app, &ctx);
        assert_eq!(themes(&app)[0].colors, colors, "saved in their models");
        add_draft_to_swatches(&mut app, &ctx);
        let g = app.session.doc().unwrap().doc.swatch_groups.last().expect("the group");
        assert_eq!(g.swatches.iter().filter_map(|w| w.paint.color()).collect::<Vec<_>>(), colors);
    }
}
