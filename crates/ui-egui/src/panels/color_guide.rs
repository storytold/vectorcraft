//! Color Guide panel: the base colour, the harmony-rule dropdown with the harmony strip, and the
//! variation grid: a row per harmony colour with the colour itself in the centre column,
//! shades/cool/muted to the left and tints/warm/vivid to the right (steps and reach from Color
//! Guide Options). The base color is the color last chosen in the Color panel, the Swatches panel
//! or the Color Picker ([`choose_base`]); selecting artwork leaves it as it is, and the base chip
//! ("Set base color to the current color") takes the current color, such as a cell applied from
//! the guide or the selected artwork's color. A click on a cell applies it to the active proxy,
//! without changing the base, and selects it; Shift- or Cmd/Ctrl-click adds
//! cells to the selection that Save Colors as Swatches saves as swatches and Save Color Group to
//! Swatches saves as one group (without a selection, the group holds the harmony colors). The
//! selection lasts until the guide changes. Limit to Library (the library button,
//! `ui.colorGuideLimit`) snaps every colour to a swatch library or the document's swatches; Edit or
//! Apply Colors opens Recolor Artwork with the harmony colours, limited the same way.

use egui::{Rect, Sense, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_color::harmony::{Guide, GuideOptions, Harmony, Variation};
use vectorcraft_color::{Color, Paint};
use vectorcraft_engine::cmd::swatchlib;

use super::swatches::{self, colour_libraries, limit_key, limit_name};
use super::{active_paint, apply_click, c32, color_json, library_panel, pstate, set_pstate};
use crate::VectorcraftApp;
use crate::widgets::{self, menu_item};

/// The base colour before one is set: an orange.
const DEFAULT_BASE: Color = Color::Rgb { r: 230.0 / 255.0, g: 120.0 / 255.0, b: 40.0 / 255.0 };

/// The guide's base color: the one last chosen or set with the base chip; before that, the current
/// color.
pub(crate) fn base_color(app: &VectorcraftApp) -> Color {
    app.ui.color_guide_base.unwrap_or_else(|| active_paint(app).color().unwrap_or(DEFAULT_BASE))
}

/// Make `c` the guide's base color: a color chosen in the Color panel, the Swatches panel or the
/// Color Picker. A color applied from the guide itself leaves the base as it is.
pub(crate) fn choose_base(app: &mut VectorcraftApp, c: Color) {
    app.ui.color_guide_base = Some(c);
}

fn rule(ctx: &egui::Context) -> Harmony {
    let i: usize = pstate(ctx, "cg-harmony");
    Harmony::ALL[i.min(Harmony::ALL.len() - 1)]
}

/// Selected grid cells: (row, column).
fn selected(ctx: &egui::Context) -> Vec<(usize, usize)> {
    pstate(ctx, "cg-sel")
}

fn clear_selection(ctx: &egui::Context) {
    set_pstate(ctx, "cg-sel", Vec::<(usize, usize)>::new());
}

/// The guide a cell selection was made in: its base, options, harmony rule, library limit and the
/// colors of its grid (which change when the document's swatches limit the guide and are edited).
#[derive(Clone, PartialEq)]
struct SelKey {
    base: Color,
    opts: GuideOptions,
    rule: usize,
    limit: String,
    grid: Vec<Vec<Color>>,
}

fn sel_key(app: &VectorcraftApp, ctx: &egui::Context, g: &Guide) -> SelKey {
    SelKey {
        base: base_color(app),
        opts: app.ui.color_guide,
        rule: pstate(ctx, "cg-harmony"),
        limit: app.ui.color_guide_limit.clone(),
        grid: g.grid.clone(),
    }
}

/// The selected cells while they belong to the guide `g` shown now; none once the base, the
/// options, the harmony rule, the library limit or the grid's colors changed since they were picked.
fn live_selection(app: &VectorcraftApp, ctx: &egui::Context, g: &Guide) -> Vec<(usize, usize)> {
    if pstate::<Option<SelKey>>(ctx, "cg-sel-key") == Some(sel_key(app, ctx, g)) { selected(ctx) } else { Vec::new() }
}

/// What the panel's guide is made from: with a library limit, snapping every cell is too slow to
/// redo each frame, so the guide is cached until one of these changes.
#[derive(Clone, PartialEq)]
struct GuideKey {
    base: Color,
    rule: Harmony,
    opts: GuideOptions,
    limit: String,
    /// The document's identity and revision while limited to its swatches.
    doc: Option<(u64, u64)>,
}

/// The guide the panel shows, limited to the Limit to Library colours.
fn guide(app: &VectorcraftApp, ctx: &egui::Context) -> Guide {
    let (base, rule, opts) = (base_color(app), rule(ctx), app.ui.color_guide);
    let limit = &app.ui.color_guide_limit;
    if limit.is_empty() {
        return Guide::new(base, rule, &opts);
    }
    let doc = app.session.active().filter(|_| limit == swatchlib::DOCUMENT_SWATCHES).map(|d| (d.uid, d.revision));
    let key = GuideKey { base, rule, opts, limit: limit.clone(), doc };
    if let Some((k, g)) = pstate::<Option<(GuideKey, Guide)>>(ctx, "cg-guide")
        && k == key
    {
        return g;
    }
    let g = Guide::new(base, rule, &opts);
    let g = match swatchlib::limit_palette(&app.session, limit) {
        Some(p) => g.limited(&p),
        None => g,
    };
    set_pstate(ctx, "cg-guide", Some((key, g.clone())));
    g
}

/// `ui.colorGuideLimit {library}`: limit the panel's colours to a swatch library (an id or name),
/// the document's swatches ("document"), or nothing (`""` or null). → {limitTo, name}
pub(crate) fn set_limit(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let key = match p.get("library").and_then(Value::as_str).unwrap_or_default() {
        "" => String::new(),
        l => limit_key(app, l)?,
    };
    let name = limit_name(app, &key);
    app.ui.color_guide_limit = key;
    Ok(json!({"limitTo": app.ui.color_guide_limit, "name": name}))
}

/// The Limit to Library popup: None, Document Swatches, then the colour libraries.
fn limit_menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    widgets::menu_min_width(ui, 200.0);
    let cur = app.ui.color_guide_limit.clone();
    let mut chosen = None;
    for (key, label) in [("", tl!("None")), (swatchlib::DOCUMENT_SWATCHES, swatches::DOCUMENT_SWATCHES)] {
        if menu_item(ui, label, true, cur == key) {
            chosen = Some(key.to_string());
        }
    }
    ui.separator();
    chosen = library_panel::library_items(ui, &colour_libraries(app), &|id| id == cur).or(chosen);
    if let Some(key) = chosen {
        set_limit(app, &json!({ "library": key })).ok();
        clear_selection(ui.ctx());
    }
}

/// Edit or Apply Colors: Recolor Artwork with the harmony colours as the new colours (of the
/// selected art, or of the colours alone), limited to the panel's library.
fn edit_or_apply(app: &mut VectorcraftApp, g: &Guide) {
    let mut p = json!({ "colors": g.colors.iter().map(color_json).collect::<Vec<_>>() });
    if limit_name(app, &app.ui.color_guide_limit).is_some() {
        p["library"] = json!(app.ui.color_guide_limit);
    }
    app.run("ui.recolorDialog", p).ok();
}

/// Save the selected cells' colors as swatches (one undo step), each in its own model.
fn save_selected(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let g = guide(app, ctx);
    let colors: Vec<Value> = live_selection(app, ctx, &g).into_iter().filter_map(|(r, c)| g.grid.get(r)?.get(c)).map(color_json).collect();
    if !colors.is_empty() {
        app.run("swatch.new", json!({ "colors": colors, "keepModel": true })).ok();
    }
}

/// Save Color Group to Swatches: the selected cells as one swatch group, in grid order (row by
/// row, left to right), each cell once; with no cell of the shown guide selected, the harmony
/// colors. The group is named after the harmony rule, saved in one undo step, and each color
/// keeps its own model.
fn save_group(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let g = guide(app, ctx);
    let mut cells = live_selection(app, ctx, &g);
    cells.sort_unstable();
    cells.dedup();
    let mut colors: Vec<Value> = cells.into_iter().filter_map(|(r, c)| g.grid.get(r)?.get(c)).map(color_json).collect();
    if colors.is_empty() {
        colors = g.colors.iter().map(color_json).collect();
    }
    app.run("swatch.newGroup", json!({"name": rule(ctx).label(), "colors": colors, "keepModel": true})).ok();
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    // The current color becomes the base when the panel first shows; selecting artwork later
    // leaves it as it is.
    let first = base_color(app);
    let set = *app.ui.color_guide_base.get_or_insert(first);
    let h = rule(&ctx);
    let g = guide(app, &ctx);
    // Selected cells belong to the guide they were picked from: a new base, new options, another
    // harmony rule, another library limit or new grid colors clear them.
    let key = SelKey { base: set, ..sel_key(app, &ctx, &g) };
    if pstate::<Option<SelKey>>(&ctx, "cg-sel-key").as_ref() != Some(&key) {
        set_pstate(&ctx, "cg-sel-key", Some(key));
        clear_selection(&ctx);
    }
    let base = g.colors[0];
    let mut chosen: Option<Color> = None;
    ui.horizontal(|ui| {
        let (r, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
        widgets::swatch_tile(ui, r, &Paint::solid(base), false, resp.hovered());
        if resp.on_hover_text(tl!("Set base color to the current color")).clicked()
            && let Some(c) = active_paint(app).color()
        {
            app.ui.color_guide_base = Some(c);
            clear_selection(&ctx);
        }
        let labels: Vec<&str> = Harmony::ALL.iter().map(|h| h.label()).collect();
        if let Some(i) = widgets::dropdown(ui, "cg-rule", h.label(), &labels, ui.available_width() - 4.0) {
            set_pstate(&ctx, "cg-harmony", i);
            clear_selection(&ctx);
        }
    });
    // Harmony strip.
    let (strip, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
    let w = strip.width() / g.colors.len() as f32;
    for (i, c) in g.colors.iter().enumerate() {
        let r = Rect::from_min_size(strip.min + vec2(i as f32 * w, 0.0), vec2(w, strip.height()));
        let resp = ui.interact(r, ui.scope_id().with(("cg-h", i)), Sense::click());
        ui.painter().rect_filled(r, 0.0, c32(c));
        if resp.on_hover_text(c.to_hex()).clicked() {
            chosen = Some(*c);
        }
    }
    ui.add_space(6.0);
    // Variation grid: a row per harmony colour, the colour itself in the centre column.
    let mut sel = selected(&ctx);
    let cols = g.grid.first().map_or(1, Vec::len);
    let centre = g.centre();
    let cell_w = ui.available_width() / cols as f32;
    let gap = if cell_w >= 8.0 { 1.0 } else { 0.0 };
    // Cell edges on whole pixels across the full width.
    let x = |i: usize| (i as f32 * cell_w).round();
    let multi = ui.input(|i| i.modifiers.shift || i.modifiers.command);
    for (ri, row) in g.grid.iter().enumerate() {
        let (rr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
        for (ci, v) in row.iter().enumerate() {
            let r = Rect::from_min_size(rr.min + vec2(x(ci), 0.0), vec2(x(ci + 1) - x(ci) - gap, 17.0));
            let resp = ui.interact(r, ui.scope_id().with(("cg-v", ri, ci)), Sense::click());
            ui.painter().rect_filled(r, 0.0, c32(v));
            if sel.contains(&(ri, ci)) {
                ui.painter().rect_stroke(r.shrink(1.0), 0.0, egui::Stroke::new(1.0, egui::Color32::WHITE), egui::StrokeKind::Inside);
                ui.painter().rect_stroke(r, 0.0, egui::Stroke::new(1.5, t.accent), egui::StrokeKind::Outside);
            } else if ci == centre {
                ui.painter().rect_stroke(r, 0.0, egui::Stroke::new(1.0, t.text_strong), egui::StrokeKind::Inside);
            }
            if resp.on_hover_text(v.to_hex()).clicked() {
                if !multi {
                    sel.clear();
                    chosen = Some(*v);
                }
                match sel.iter().position(|s| *s == (ri, ci)) {
                    Some(i) => {
                        sel.remove(i);
                    }
                    None => sel.push((ri, ci)),
                }
                set_pstate(&ctx, "cg-sel", sel.clone());
            }
        }
    }
    let (left, right) = app.ui.color_guide.variation.sides();
    let key = &app.ui.color_guide_limit;
    let limit = limit_name(app, key).map(|n| library_panel::library_name(key, &n).to_string());
    ui.horizontal(|ui| {
        widgets::dim_label(ui, left);
        // The library the colours are limited to, centred between the ends.
        let middle = ui.available_width() - 40.0;
        let label = limit.as_deref().unwrap_or_default();
        ui.add_sized([middle.max(0.0), 16.0], egui::Label::new(egui::RichText::new(label).size(11.0).color(t.text_dim)).truncate());
        widgets::dim_label(ui, right);
    });
    widgets::bottom_bar(ui, |ui| {
        let tip = match &limit {
            Some(name) => crate::i18n::fmt(tl!("Limit colors to swatch library: {name}"), &[("name", name)]),
            None => tl!("Limit colors to swatch library").into(),
        };
        // The tip is translated around the library's name, not looked up whole.
        let lr = widgets::icon_button(ui, "library", "", limit.is_some(), 24.0);
        lr.widget_info(|| widgets::icon_button_info(ui.is_enabled(), limit.is_some(), &tip));
        let lr = lr.on_hover_text(tip);
        egui::Popup::menu(&lr).show(|ui| widgets::menu_scroll(ui, |ui| limit_menu(app, ui)));
        ui.add_space((ui.available_width() - 3.0 * 28.0).max(0.0));
        // Recolor Artwork with the harmony colours as the new colours: the selected art's, or
        // (without art) the colours themselves, which OK saves as a colour group.
        let doc = app.session.active().is_some();
        if widgets::icon_button_enabled(ui, "palette", tl!("Edit or Apply Colors"), false, doc, 24.0).clicked() {
            edit_or_apply(app, &g);
        }
        if widgets::icon_button_enabled(ui, "dc-new-item", tl!("Save selected colors as swatches"), false, !sel.is_empty(), 24.0).clicked() {
            save_selected(app, &ctx);
        }
        if widgets::icon_button(ui, "dc-folder", tl!("Save color group to Swatch panel"), false, 24.0).clicked() {
            save_group(app, &ctx);
        }
    });
    if let Some(c) = chosen {
        apply_click(app, ui, json!({"color": color_json(&c)}));
    }
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let mode = app.ui.color_guide.variation;
    for (m, l) in [
        (Variation::TintsShades, tl!("Show Tints/Shades")),
        (Variation::WarmCool, tl!("Show Warm/Cool")),
        (Variation::VividMuted, tl!("Show Vivid/Muted")),
    ] {
        if menu_item(ui, l, true, m == mode) {
            app.ui.color_guide.variation = m;
        }
    }
    ui.separator();
    let ctx = ui.ctx().clone();
    if menu_item(ui, tl!("Save Colors as Swatches"), !live_selection(app, &ctx, &guide(app, &ctx)).is_empty(), false) {
        save_selected(app, &ctx);
    }
    if menu_item(ui, tl!("Color Guide Options…"), true, false) {
        app.run("ui.colorGuideOptions", json!({})).ok();
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

    /// Screen readers name the library button by its tool tip, pressed while a library limits the
    /// colors.
    #[test]
    fn screen_readers_name_the_limit_button() {
        use crate::widgets::accessibility_tests::{role_and_state, tree};
        use egui::accesskit::{Role, Toggled};
        let mut app = app();
        let shown = tree(|ui| show(&mut app, ui));
        assert_eq!(role_and_state(&shown, "Limit colors to swatch library"), (Role::Button, None));
        app.run("ui.colorGuideLimit", json!({"library": "Earth Tones"})).unwrap();
        let shown = tree(|ui| show(&mut app, ui));
        assert_eq!(role_and_state(&shown, "Limit colors to swatch library: Earth Tones"), (Role::Button, Some(Toggled::True)));
    }

    /// The first show takes the current color; a color chosen in the Color panel, Swatches or the
    /// Color Picker becomes the base; selecting artwork with another color leaves it.
    #[test]
    fn a_chosen_color_becomes_the_base_and_selecting_art_keeps_it() {
        let mut app = app();
        let red = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
        app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
        let blue = app.run("shape.rectangle", json!({"x": 20, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
        app.run("paint.setFill", json!({"color": "#3366cc"})).unwrap();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx);
        assert_eq!(base_color(&app).to_hex(), "#3366cc", "the first show takes the current color");
        app.run("select.set", json!({"ids": [red]})).unwrap();
        frame(&mut app, &ctx);
        assert_eq!(base_color(&app).to_hex(), "#3366cc", "selecting art keeps the base");
        choose_base(&mut app, Color::from_hex("#20a060").unwrap());
        frame(&mut app, &ctx);
        app.ui.color_guide.steps = 6;
        frame(&mut app, &ctx);
        let g = guide(&app, &ctx);
        assert_eq!(g.grid[0].len(), 13);
        assert_eq!(g.grid[0][6].to_hex(), "#20a060", "the chosen base sits in the centre of the first row");
        app.run("select.set", json!({"ids": [blue]})).unwrap();
        frame(&mut app, &ctx);
        assert_eq!(base_color(&app).to_hex(), "#20a060");
    }

    /// The rect of the panel widget `id` after one frame.
    fn rect_of(app: &mut VectorcraftApp, ctx: &egui::Context, id: impl std::hash::Hash + std::fmt::Debug) -> Rect {
        let mut r = None;
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            show(app, ui);
            r = ctx.read_response(ui.scope_id().with(&id)).map(|r| r.rect);
        });
        out.textures_delta.clear();
        r.expect("the widget is drawn")
    }

    fn click(app: &mut VectorcraftApp, ctx: &egui::Context, at: egui::Pos2) {
        let button =
            |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        for events in [vec![egui::Event::PointerMoved(at), button(true)], vec![button(false)]] {
            let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| show(app, ui));
            out.textures_delta.clear();
        }
    }

    /// A harmony color or a variation clicked in the guide paints the art without changing the
    /// base; the base chip then makes the current color the base.
    #[test]
    fn a_guide_color_applies_without_moving_the_base_until_set_as_base() {
        let mut app = app();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        app.run("paint.setFill", json!({"color": "#3366cc"})).unwrap();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx);
        let g = guide(&app, &ctx);
        let fill = |app: &VectorcraftApp| crate::panels::current_paints(app).0.color().map(|c| c.to_hex());
        let at = rect_of(&mut app, &ctx, ("cg-h", 1)).center();
        click(&mut app, &ctx, at);
        assert_eq!(fill(&app), Some(g.colors[1].to_hex()), "the harmony color paints the art");
        assert_eq!(base_color(&app).to_hex(), "#3366cc", "and leaves the base");
        let at = rect_of(&mut app, &ctx, ("cg-v", 0usize, 2usize)).center();
        click(&mut app, &ctx, at);
        let variation = g.grid[0][2].to_hex();
        assert_eq!(fill(&app), Some(variation.clone()), "a variation paints the art");
        assert_eq!(base_color(&app).to_hex(), "#3366cc");
        assert_eq!(guide(&app, &ctx).grid, g.grid, "the guide stays as it was");
        // Set base color to the current color: the applied variation becomes the base.
        let mut chip = None;
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            show(&mut app, ui);
            chip = Some(ui.min_rect().left_top() + egui::vec2(8.0, 8.0));
        });
        out.textures_delta.clear();
        click(&mut app, &ctx, chip.expect("the panel is drawn"));
        assert_eq!(base_color(&app).to_hex(), variation, "the base chip adopts the current color");
    }

    #[test]
    fn save_colors_as_swatches_saves_each_selected_cell() {
        let mut app = app();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx);
        let before = app.session.doc().unwrap().doc.swatches.len();
        set_pstate(&ctx, "cg-sel", vec![(0usize, 0usize), (1, 4), (0, 8)]);
        let undo = app.session.doc().unwrap().history.undo.len();
        save_selected(&mut app, &ctx);
        let st = app.session.doc().unwrap();
        assert_eq!(st.doc.swatches.len(), before + 3);
        assert_eq!(st.history.undo.len(), undo + 1, "one undo step");
        let g = guide(&app, &ctx);
        assert_eq!(st.doc.swatches.last().unwrap().paint.color().map(|c| c.to_hex()), Some(g.grid[0][8].to_hex()));
    }

    /// In a CMYK document the cells of an RGB guide are saved in RGB, their own model.
    #[test]
    fn saved_cells_keep_their_model_in_a_cmyk_document() {
        let mut app = app();
        app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
        app.run("paint.setFill", json!({"color": "#3366cc", "keepModel": true})).unwrap();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx);
        set_pstate(&ctx, "cg-sel", vec![(0usize, 0usize), (0, 6)]);
        save_selected(&mut app, &ctx);
        let g = guide(&app, &ctx);
        assert!(matches!(g.grid[0][6], Color::Rgb { .. }), "{:?}", g.grid[0][6]);
        let sw = &app.session.doc().unwrap().doc.swatches;
        let saved: Vec<Color> = sw[sw.len() - 2..].iter().filter_map(|w| w.paint.color()).collect();
        assert_eq!(saved, [g.grid[0][0], g.grid[0][6]]);
    }

    /// The colors of the document's newest swatch group, and the group's name.
    fn last_group(app: &VectorcraftApp) -> (String, Vec<Color>) {
        let g = app.session.doc().unwrap().doc.swatch_groups.last().cloned().expect("a swatch group");
        (g.name, g.swatches.iter().filter_map(|w| w.paint.color()).collect())
    }

    /// With cells selected, Save Color Group to Swatches saves exactly those cells as one group, in
    /// grid order and each cell once, in one undo step that Undo removes.
    #[test]
    fn save_color_group_saves_the_selected_cells_in_grid_order() {
        let mut app = app();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx);
        let st = app.session.doc().unwrap();
        let (groups, swatches, undo) = (st.doc.swatch_groups.len(), st.doc.swatches.len(), st.history.undo.len());
        set_pstate(&ctx, "cg-sel", vec![(1usize, 4usize), (0, 8), (0, 0), (1, 4)]);
        save_group(&mut app, &ctx);
        let g = guide(&app, &ctx);
        let st = app.session.doc().unwrap();
        assert_eq!(st.doc.swatch_groups.len(), groups + 1, "one new group");
        assert_eq!(st.doc.swatches.len(), swatches, "no loose swatches");
        assert_eq!(st.history.undo.len(), undo + 1, "one undo step");
        let (name, made) = last_group(&app);
        assert_eq!(name, rule(&ctx).label());
        assert_eq!(made, [g.grid[0][0], g.grid[0][8], g.grid[1][4]]);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc.swatch_groups.len(), groups, "Undo removes the group");
    }

    /// With no cell selected, or only cells outside the shown grid, the group holds the harmony
    /// colors.
    #[test]
    fn save_color_group_without_a_selection_saves_the_harmony_colors() {
        let mut app = app();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx);
        let g = guide(&app, &ctx);
        save_group(&mut app, &ctx);
        assert_eq!(last_group(&app).1, g.colors);
        set_pstate(&ctx, "cg-sel", vec![(99usize, 0usize), (0, 99)]);
        save_group(&mut app, &ctx);
        assert_eq!(last_group(&app).1, g.colors, "cells outside the grid are ignored");
    }

    /// A guide limited to a library saves its snapped cells, and an RGB cell stays RGB in a CMYK
    /// document.
    #[test]
    fn save_color_group_saves_limited_cells_in_their_own_model() {
        let mut app = app();
        app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
        app.run("paint.setFill", json!({"color": "#c04020", "keepModel": true})).unwrap();
        app.run("ui.colorGuideLimit", json!({"library": "web-safe-216"})).unwrap();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx);
        set_pstate(&ctx, "cg-sel", vec![(0usize, 1usize), (0, 7)]);
        save_group(&mut app, &ctx);
        let g = guide(&app, &ctx);
        let made = last_group(&app).1;
        assert_eq!(made, [g.grid[0][1], g.grid[0][7]]);
        assert!(made.iter().all(|c| matches!(c, Color::Rgb { .. })), "{made:?}");
        let members = library_hexes(&app, "web-safe-216");
        assert!(made.iter().all(|c| members.contains(&c.to_hex())), "the snapped colors");
    }

    /// A selection made in one guide is not saved from another: new options, the variation-mode
    /// menu, the harmony rule or the library limit clear it.
    #[test]
    fn a_changed_guide_drops_the_cell_selection() {
        let mut app = app();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx);
        let pick = |ctx: &egui::Context| set_pstate(ctx, "cg-sel", vec![(0usize, 1usize), (0, 2)]);
        type Change = fn(&mut VectorcraftApp, &egui::Context);
        let changes: [Change; 4] = [
            |app, _| app.ui.color_guide.steps = 6,
            |app, _| app.ui.color_guide.variation = Variation::WarmCool,
            |_, ctx| set_pstate(ctx, "cg-harmony", 3usize),
            |app, _| {
                app.run("ui.colorGuideLimit", json!({"library": "web-safe-216"})).unwrap();
            },
        ];
        for (i, change) in changes.iter().enumerate() {
            frame(&mut app, &ctx);
            pick(&ctx);
            assert_eq!(live_selection(&app, &ctx, &guide(&app, &ctx)).len(), 2, "change {i}: the selection holds");
            change(&mut app, &ctx);
            let g = guide(&app, &ctx);
            assert!(live_selection(&app, &ctx, &g).is_empty(), "change {i}: nothing is saved from the old guide");
            save_group(&mut app, &ctx);
            assert_eq!(last_group(&app).1, g.colors, "change {i}: the harmony colors");
            frame(&mut app, &ctx);
            assert!(selected(&ctx).is_empty(), "change {i}: the panel clears the selection");
        }
    }

    /// Limited to the document's swatches, painting art keeps the cell selection, and a new swatch
    /// that changes the grid's colors drops it.
    #[test]
    fn a_new_document_swatch_drops_the_cell_selection() {
        let mut app = app();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        choose_base(&mut app, Color::from_hex("#3a7bd5").unwrap());
        app.run("ui.colorGuideLimit", json!({"library": "document"})).unwrap();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx);
        set_pstate(&ctx, "cg-sel", vec![(0usize, 1usize), (0, 2)]);
        app.run("paint.setFill", json!({"color": "#102030"})).unwrap();
        frame(&mut app, &ctx);
        assert_eq!(selected(&ctx).len(), 2, "an edit that leaves the grid as it is keeps the selection");
        let before = guide(&app, &ctx);
        app.run("swatch.new", json!({"colors": ["#3a7bd5"]})).unwrap();
        let g = guide(&app, &ctx);
        assert_ne!(g.grid, before.grid, "the new swatch changes the grid");
        assert!(live_selection(&app, &ctx, &g).is_empty(), "nothing is saved from the old grid");
        save_group(&mut app, &ctx);
        assert_eq!(last_group(&app).1, g.colors);
        frame(&mut app, &ctx);
        assert!(selected(&ctx).is_empty(), "the panel clears the selection");
    }

    /// Every colour of the panel's guide.
    fn shown(app: &VectorcraftApp, ctx: &egui::Context) -> Vec<String> {
        let g = guide(app, ctx);
        g.grid.iter().flatten().chain(&g.colors).map(Color::to_hex).collect()
    }

    fn library_hexes(app: &VectorcraftApp, id: &str) -> Vec<String> {
        let (_, lib) = swatchlib::library(&app.session, id).unwrap();
        lib.iter().filter_map(|w| w.paint.color()).map(|c| c.to_hex()).collect()
    }

    #[test]
    fn limit_to_library_snaps_the_panel_and_follows_the_document_swatches() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.run("paint.setFill", json!({"color": "#3a7bd5"})).unwrap();
        frame(&mut app, &ctx);
        assert!(shown(&app, &ctx).contains(&"#3a7bd5".to_string()), "no limit: the base itself");
        let r = app.run("ui.colorGuideLimit", json!({"library": "Earth Tones"})).unwrap();
        assert_eq!((r["limitTo"].as_str(), r["name"].as_str()), (Some("earth-tones"), Some("Earth Tones")));
        frame(&mut app, &ctx);
        let members = library_hexes(&app, "earth-tones");
        assert!(shown(&app, &ctx).iter().all(|h| members.contains(h)), "every cell is a library colour");
        // The document's swatches: the cached guide follows a new swatch.
        app.run("ui.colorGuideLimit", json!({"library": "document"})).unwrap();
        frame(&mut app, &ctx);
        assert!(!shown(&app, &ctx).contains(&"#3a7bd4".to_string()));
        app.run("swatch.new", json!({"colors": ["#3a7bd4"]})).unwrap();
        frame(&mut app, &ctx);
        let doc: Vec<String> = app.session.doc().unwrap().doc.swatches_iter().filter_map(|w| w.paint.color()).map(|c| c.to_hex()).collect();
        let cells = shown(&app, &ctx);
        assert!(cells.iter().all(|h| doc.contains(h)));
        assert_eq!(guide(&app, &ctx).colors[0].to_hex(), "#3a7bd4", "the base snaps to the new swatch");
        // The popup lists libraries with colours only, draws; unknown libraries are refused and ""
        // lifts the limit.
        let libs = colour_libraries(&app);
        assert!(libs.iter().any(|l| l.id == "earth-tones") && !libs.iter().any(|l| l.id == "metallic-gradients"));
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| limit_menu(&mut app, ui));
        out.textures_delta.clear();
        assert!(app.run("ui.colorGuideLimit", json!({"library": "nope"})).is_err());
        assert_eq!(app.ui.color_guide_limit, "document");
        assert_eq!(app.run("ui.colorGuideLimit", json!({"library": ""})).unwrap()["name"], Value::Null);
        assert!(shown(&app, &ctx).contains(&"#3a7bd5".to_string()));
    }

    #[test]
    fn edit_or_apply_without_art_saves_the_limited_colours_as_a_group() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.run("paint.setFill", json!({"color": "#c04020"})).unwrap();
        app.run("select.none", json!({})).unwrap();
        app.run("ui.colorGuideLimit", json!({"library": "web-safe-216"})).unwrap();
        frame(&mut app, &ctx);
        let g = guide(&app, &ctx);
        edit_or_apply(&mut app, &g);
        let d = app.ui.dialog.as_ref().expect("Recolor opens on the colours alone");
        assert_eq!((d.kind.as_str(), d.str("limitTo").as_str()), ("recolor", "web-safe-216"), "limited to the same library");
        let groups = app.session.doc().unwrap().doc.swatch_groups.len();
        crate::dialogs::confirm(&mut app).unwrap();
        let doc = &app.session.doc().unwrap().doc;
        assert_eq!(doc.swatch_groups.len(), groups + 1, "OK saves a new colour group");
        let made: Vec<String> = doc.swatch_groups.last().unwrap().swatches.iter().filter_map(|w| w.paint.color()).map(|c| c.to_hex()).collect();
        assert_eq!(made, g.colors.iter().map(Color::to_hex).collect::<Vec<_>>());
        let members = library_hexes(&app, "web-safe-216");
        assert!(made.iter().all(|h| members.contains(h)));
        // Colours from elsewhere snap to the library as the dialog opens.
        app.run("ui.recolorDialog", json!({"colors": ["#123457"], "library": "web-safe-216"})).unwrap();
        let to = app.ui.dialog.as_ref().unwrap().fields["rows"][0]["to"].clone();
        assert_eq!(vectorcraft_engine::cmd::color_value(&to).map(|c| c.to_hex()).as_deref(), Some("#003366"));
    }
}
