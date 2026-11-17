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
//! selection lasts until the guide changes. A harmony color or a cell dragged with the primary button
//! carries its color alone, as shown, without applying it or changing the selection; the Swatches
//! panel makes a swatch of it. When the base is a spot color, or a tint of one, chosen from the
//! Swatches panel, the Color panel's tint slider or the base chip ([`SpotBase`]), the cells right of
//! the center in the base row of Tints/Shades are tints of that spot color: a click applies the tint,
//! a drag carries it, and saving makes tint swatches linked to the spot color. Every other cell is
//! a process color. Limit to Library (the library button,
//! `ui.colorGuideLimit`) snaps every colour to a swatch library or the document's swatches; Edit or
//! Apply Colors opens Recolor Artwork with the harmony colours, limited the same way.

use egui::{Rect, Sense, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_color::harmony::{self, Guide, GuideOptions, Harmony, Variation};
use vectorcraft_color::{Color, Paint};
use vectorcraft_engine::cmd::swatchlib;

use super::swatches::{self, colour_libraries, limit_key, limit_name};
use super::{active_paint, apply_click, c32, color_json, library_panel, paint_params, pstate, set_pstate};
use crate::VectorcraftApp;
use crate::widgets::{self, PanelDrag, menu_item};

/// The base colour before one is set: an orange.
const DEFAULT_BASE: Color = Color::Rgb { r: 230.0 / 255.0, g: 120.0 / 255.0, b: 40.0 / 255.0 };

/// The spot color swatch the guide's base is a tint of (`UiState::color_guide_spot`).
#[derive(Clone, Debug, PartialEq)]
pub struct SpotBase {
    /// The document the swatch belongs to (its `uid`).
    pub doc: u64,
    /// The spot color swatch's name.
    pub swatch: String,
    /// The base's tint of it, 0..1.
    pub tint: f32,
}

/// The spot color the base is a tint of, while its swatch is still a spot color of the active
/// document.
fn live_spot(app: &VectorcraftApp) -> Option<&SpotBase> {
    let s = app.ui.color_guide_spot.as_ref()?;
    let st = app.session.active()?;
    let spot = st.doc.swatch(&s.swatch).is_some_and(|w| w.spot) && st.doc.global_color(&s.swatch).is_some();
    (st.uid == s.doc && spot).then_some(s)
}

/// The color of tint `tint` (0..1) of global swatch `name` in the active document.
fn tint_color(app: &VectorcraftApp, name: &str, tint: f32) -> Option<Color> {
    app.session.active()?.doc.tint_paint(name, tint)?.color()
}

/// The guide's base color: the one last chosen or set with the base chip (a spot color base
/// follows edits to its swatch); before that, the current color.
pub(crate) fn base_color(app: &VectorcraftApp) -> Color {
    if let Some(c) = live_spot(app).and_then(|s| tint_color(app, &s.swatch, s.tint)) {
        return c;
    }
    app.ui.color_guide_base.unwrap_or_else(|| active_paint(app).color().unwrap_or(DEFAULT_BASE))
}

/// Make `c` the guide's base color: a color chosen in the Color panel, the Swatches panel or the
/// Color Picker. A color applied from the guide itself leaves the base as it is.
pub(crate) fn choose_base(app: &mut VectorcraftApp, c: Color) {
    choose_linked_base(app, c, None);
}

/// [`choose_base`] for a color linked to a global swatch at a tint (0..1): when that swatch is a
/// spot color of the active document, the base is that tint of it ([`SpotBase`]).
pub(crate) fn choose_linked_base(app: &mut VectorcraftApp, c: Color, link: Option<(String, f32)>) {
    app.ui.color_guide_base = Some(c);
    app.ui.color_guide_spot = link.and_then(|(swatch, tint)| {
        let st = app.session.active()?;
        st.doc.swatch(&swatch).filter(|w| w.spot)?;
        Some(SpotBase { doc: st.uid, swatch, tint: tint.clamp(0.0, 1.0) })
    });
}

/// `swatch.edit` renamed swatch `from` of the active document to `to`: a spot color base linked to
/// it links to `to`.
pub(crate) fn swatch_renamed(app: &mut VectorcraftApp, from: &str, to: &str) {
    let uid = app.session.active().map(|st| st.uid);
    if let Some(s) = app.ui.color_guide_spot.as_mut()
        && Some(s.doc) == uid
        && s.swatch == from
    {
        s.swatch = to.to_string();
    }
}

/// The global swatch and tint (0..1) a solid paint links to.
fn paint_link(p: &Paint) -> Option<(String, f32)> {
    match p {
        Paint::Solid { swatch: Some(n), tint, .. } => Some((n.clone(), *tint)),
        _ => None,
    }
}

/// The tint of the spot color base that cell (`row`, `col`) of the guide `g` is: a cell right of
/// the center in the base row of Tints/Shades, without a library limit. Its tint is the base's
/// tint less the share the cell goes towards white. → (swatch, tint 0..1)
fn cell_tint(app: &VectorcraftApp, g: &Guide, row: usize, col: usize) -> Option<(String, f32)> {
    let opts = app.ui.color_guide.clamped();
    let centre = g.centre();
    if row != 0 || col <= centre || opts.variation != Variation::TintsShades || !app.ui.color_guide_limit.is_empty() {
        return None;
    }
    let s = live_spot(app)?;
    let step = i32::try_from(col - centre).ok()?;
    Some((s.swatch.clone(), s.tint * (1.0 - harmony::step_fraction(&opts, step))))
}

/// The paint cell (`row`, `col`) of `g` applies, carries and saves: a tint of the spot color base
/// ([`cell_tint`]) or the color the cell shows.
fn cell_paint(app: &VectorcraftApp, g: &Guide, row: usize, col: usize) -> Option<Paint> {
    let color = *g.grid.get(row)?.get(col)?;
    Some(match cell_tint(app, g, row, col) {
        Some((name, tint)) => Paint::Solid { color, swatch: Some(name), tint },
        None => Paint::solid(color),
    })
}

/// The `swatch.new` / `swatch.newGroup` entry for cell (`row`, `col`) of `g`: `{swatch, tint}` for
/// a tint of the spot color base, the color otherwise.
fn cell_entry(app: &VectorcraftApp, g: &Guide, row: usize, col: usize) -> Option<Value> {
    Some(match cell_paint(app, g, row, col)? {
        p @ Paint::Solid { swatch: Some(_), .. } => paint_params(&p),
        p => color_json(&p.color()?),
    })
}

/// `g` with the base row's spot color tints ([`cell_tint`]) shown in their own colors.
fn with_spot_tints(app: &VectorcraftApp, mut g: Guide) -> Guide {
    let cols = g.grid.first().map_or(0, Vec::len);
    let tints: Vec<(usize, Color)> =
        (0..cols).filter_map(|col| cell_tint(app, &g, 0, col).and_then(|(n, t)| tint_color(app, &n, t)).map(|c| (col, c))).collect();
    for (col, c) in tints {
        g.grid[0][col] = c;
    }
    g
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
    spot: Option<SpotBase>,
    opts: GuideOptions,
    rule: usize,
    limit: String,
    grid: Vec<Vec<Color>>,
}

fn sel_key(app: &VectorcraftApp, ctx: &egui::Context, g: &Guide) -> SelKey {
    SelKey {
        base: base_color(app),
        spot: live_spot(app).cloned(),
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

/// The guide the panel shows, limited to the Limit to Library colours; without a limit, a spot
/// color base's tints show in their own colors.
fn guide(app: &VectorcraftApp, ctx: &egui::Context) -> Guide {
    let (base, rule, opts) = (base_color(app), rule(ctx), app.ui.color_guide);
    let limit = &app.ui.color_guide_limit;
    if limit.is_empty() {
        return with_spot_tints(app, Guide::new(base, rule, &opts));
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

/// Save the selected cells' colors as swatches (one undo step), each in its own model; a tint of a
/// spot color base becomes a tint swatch of it.
fn save_selected(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let g = guide(app, ctx);
    let colors: Vec<Value> = live_selection(app, ctx, &g).into_iter().filter_map(|(r, c)| cell_entry(app, &g, r, c)).collect();
    if !colors.is_empty() {
        app.run("swatch.new", json!({ "colors": colors, "keepModel": true })).ok();
    }
}

/// Save Color Group to Swatches: the selected cells as one swatch group, in grid order (row by
/// row, left to right), each cell once; with no cell of the shown guide selected, the harmony
/// colors. The group is named after the harmony rule, saved in one undo step, and each color
/// keeps its own model; a tint of a spot color base becomes a tint swatch of it.
fn save_group(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let g = guide(app, ctx);
    let mut cells = live_selection(app, ctx, &g);
    cells.sort_unstable();
    cells.dedup();
    let mut colors: Vec<Value> = cells.into_iter().filter_map(|(r, c)| cell_entry(app, &g, r, c)).collect();
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
    if app.ui.color_guide_base.is_none() {
        let current = active_paint(app);
        choose_linked_base(app, current.color().unwrap_or(DEFAULT_BASE), paint_link(&current));
    }
    let h = rule(&ctx);
    let g = guide(app, &ctx);
    // Selected cells belong to the guide they were picked from: a new base, new options, another
    // harmony rule, another library limit or new grid colors clear them.
    let key = sel_key(app, &ctx, &g);
    if pstate::<Option<SelKey>>(&ctx, "cg-sel-key").as_ref() != Some(&key) {
        set_pstate(&ctx, "cg-sel-key", Some(key));
        clear_selection(&ctx);
    }
    let base = g.colors[0];
    let mut chosen: Option<Paint> = None;
    ui.horizontal(|ui| {
        let (r, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
        widgets::swatch_tile(ui, r, &Paint::solid(base), false, resp.hovered());
        if resp.on_hover_text(tl!("Set base color to the current color")).clicked() {
            let current = active_paint(app);
            if let Some(c) = current.color() {
                choose_linked_base(app, c, paint_link(&current));
                clear_selection(&ctx);
            }
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
        let resp = ui.interact(r, ui.scope_id().with(("cg-h", i)), Sense::click_and_drag());
        widgets::drag_source(ui, &resp, || PanelDrag::paint(Paint::solid(*c)));
        ui.painter().rect_filled(r, 0.0, c32(c));
        if resp.on_hover_text(c.to_hex()).clicked() {
            chosen = Some(Paint::solid(*c));
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
            let resp = ui.interact(r, ui.scope_id().with(("cg-v", ri, ci)), Sense::click_and_drag());
            let paint = cell_paint(app, &g, ri, ci).unwrap_or_else(|| Paint::solid(*v));
            widgets::drag_source(ui, &resp, || PanelDrag::paint(paint.clone()));
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
                    chosen = Some(paint);
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
    if let Some(p) = chosen {
        apply_click(app, ui, paint_params(&p));
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

    /// One frame at `time` with `events` of the Color Guide panel, followed by the Swatches panel when
    /// `swatches` is set; → the center of the guide widget `id` as laid out in that frame.
    fn guide_frame(
        app: &mut VectorcraftApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        time: f64,
        swatches: bool,
        id: impl std::hash::Hash + std::fmt::Debug,
    ) -> egui::Pos2 {
        let screen_rect = Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(280.0, 900.0)));
        let input = egui::RawInput { events, time: Some(time), screen_rect, ..Default::default() };
        let mut at = None;
        let mut out = ctx.run_ui(input, |ui| {
            show(app, ui);
            at = ctx.read_response(ui.scope_id().with(&id)).map(|r| r.rect.center());
            if swatches {
                super::swatches::show(app, ui);
            }
        });
        out.textures_delta.clear();
        at.expect("the guide widget is drawn")
    }

    /// Press on the guide widget `id`, move far enough to start a drag, move to `to` (the widget
    /// itself when None) and release there, a frame per step from `time`. → the drag the panel
    /// started, read before the release.
    fn drag_from(
        app: &mut VectorcraftApp,
        ctx: &egui::Context,
        id: (&str, usize, usize),
        to: Option<egui::Pos2>,
        time: f64,
        swatches: bool,
    ) -> Option<PanelDrag> {
        let from = match id.0 {
            "cg-h" => guide_frame(app, ctx, vec![], time, swatches, ("cg-h", id.1)),
            _ => guide_frame(app, ctx, vec![], time, swatches, ("cg-v", id.1, id.2)),
        };
        let to = to.unwrap_or(from);
        let button =
            |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        let step = |app: &mut VectorcraftApp, events, t| {
            guide_frame(app, ctx, events, t, swatches, ("cg-h", 0usize));
        };
        step(app, vec![egui::Event::PointerMoved(from), button(from, true)], time + 0.1);
        step(app, vec![egui::Event::PointerMoved(from + vec2(8.0, 3.0))], time + 0.2);
        let held = egui::DragAndDrop::payload::<PanelDrag>(ctx).map(|d| (*d).clone());
        step(app, vec![egui::Event::PointerMoved(to)], time + 0.3);
        step(app, vec![egui::Event::PointerMoved(to), button(to, false)], time + 0.4);
        held
    }

    /// A harmony color or a variation dragged from the guide carries that color alone, as a solid
    /// paint; the drag applies nothing to the selected art and leaves the base and the selected
    /// cells as they were.
    #[test]
    fn a_dragged_guide_color_carries_its_color_and_applies_nothing() {
        let mut app = app();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        app.run("paint.setFill", json!({"color": "#3366cc"})).unwrap();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx);
        let g = guide(&app, &ctx);
        set_pstate(&ctx, "cg-sel", vec![(0usize, 1usize), (1, 2)]);
        let fill = |app: &VectorcraftApp| crate::panels::current_paints(app).0.color().map(|c| c.to_hex());
        let held = drag_from(&mut app, &ctx, ("cg-h", 1, 0), None, 1.0, false);
        assert_eq!(held, Some(PanelDrag::paint(Paint::solid(g.colors[1]))), "the harmony color");
        let held = drag_from(&mut app, &ctx, ("cg-v", 0, 4), None, 2.0, false);
        assert_eq!(held, Some(PanelDrag::paint(Paint::solid(g.grid[0][4]))), "the grabbed cell only, not the selected ones");
        assert_eq!(held.and_then(|d| d.color()), Some(g.grid[0][4]));
        assert!(egui::DragAndDrop::payload::<PanelDrag>(&ctx).is_none(), "the release ends the drag");
        assert_eq!(fill(&app).as_deref(), Some("#3366cc"), "the drag paints no art");
        assert_eq!(base_color(&app).to_hex(), "#3366cc", "the base stays");
        assert_eq!(selected(&ctx), [(0, 1), (1, 2)], "the selected cells stay");
    }

    /// A variation of a guide limited to a library, dragged onto the Swatches panel, becomes one
    /// swatch of the color the cell shows, in its own model in a CMYK document, as one undo step; the
    /// same color dragged again is added under another name.
    #[test]
    fn a_guide_color_dragged_onto_the_swatches_panel_becomes_a_swatch() {
        let mut app = app();
        app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
        app.run("paint.setFill", json!({"color": "#c04020", "keepModel": true})).unwrap();
        app.run("ui.colorGuideLimit", json!({"library": "web-safe-216"})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        guide_frame(&mut app, &ctx, vec![], 0.0, true, ("cg-h", 0usize));
        let g = guide(&app, &ctx);
        let cell = g.grid[0][1];
        assert!(matches!(cell, Color::Rgb { .. }), "{cell:?}");
        assert!(library_hexes(&app, "web-safe-216").contains(&cell.to_hex()), "the cell shows a library color");
        let tile = |ctx: &egui::Context| ctx.read_response(egui::Id::unique(("swatch-tile", "Red"))).expect("the Red swatch is drawn").rect.center();
        let (count, undo) = {
            let st = app.session.doc().unwrap();
            (st.doc.swatches_iter().count(), st.history.undo.len())
        };
        drag_from(&mut app, &ctx, ("cg-v", 0, 1), Some(tile(&ctx)), 1.0, true);
        let st = app.session.doc().unwrap();
        assert_eq!(st.doc.swatches_iter().count(), count + 1, "{}", app.ui.status);
        assert_eq!(st.history.undo.len(), undo + 1, "one undo step");
        let first = st.doc.swatches.last().cloned().expect("a swatch");
        assert_eq!(first.paint, Paint::solid(cell), "the shown color, still RGB");
        assert!(!first.global, "a plain process color");
        drag_from(&mut app, &ctx, ("cg-v", 0, 1), Some(tile(&ctx)), 2.0, true);
        let newest = app.session.doc().unwrap().doc.swatches.last().cloned().expect("a swatch");
        assert_eq!(newest.paint, Paint::solid(cell));
        assert_ne!(newest.name, first.name, "another name");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc.swatches_iter().count(), count + 1, "Undo takes the second one off");
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

    /// A document with a selected rectangle and the spot color swatch "Ink", made the guide's base
    /// as clicking it in the Swatches panel makes it, at `tint` (0..1) of the spot color.
    fn spot_app(tint: f32) -> VectorcraftApp {
        let mut app = app();
        app.run("swatch.new", json!({"name": "Ink", "color": "#0066cc", "spot": true})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        app.run("paint.setFill", json!({"color": "#808080"})).unwrap();
        let ink = app.session.active().unwrap().doc.global_color("Ink").unwrap();
        choose_linked_base(&mut app, ink.tinted(tint), Some(("Ink".to_string(), tint)));
        app
    }

    /// The selected art's fill: (the swatch it links to, its tint 0..1, its color).
    fn fill_link(app: &VectorcraftApp) -> (Option<String>, f32, String) {
        match crate::panels::current_paints(app).0 {
            Paint::Solid { color, swatch, tint } => (swatch, tint, color.to_hex()),
            other => panic!("a solid fill: {other:?}"),
        }
    }

    /// The tint the cell `steps` right of the centre has with a base at `base` tint.
    fn expected_tint(app: &VectorcraftApp, base: f32, steps: i32) -> f32 {
        base * (1.0 - harmony::step_fraction(&app.ui.color_guide.clamped(), steps))
    }

    /// With a spot color base, a cell right of the centre in the base row of Tints/Shades paints the
    /// art with a tint of the spot color, which follows an edit of the spot color; the shades, the
    /// other rows, the other variation modes and a library limit give process colors.
    #[test]
    fn a_spot_color_base_gives_tints_of_that_spot_color() {
        let mut app = spot_app(1.0);
        let ctx = egui::Context::default();
        frame(&mut app, &ctx);
        let g = guide(&app, &ctx);
        let c = g.centre();
        let ink = app.session.active().unwrap().doc.global_color("Ink").unwrap();
        assert_eq!(g.grid[0][c].to_hex(), ink.to_hex(), "the spot color in the centre");
        let at = rect_of(&mut app, &ctx, ("cg-v", 0usize, c + 2)).center();
        click(&mut app, &ctx, at);
        let (swatch, tint, hex) = fill_link(&app);
        assert_eq!(swatch.as_deref(), Some("Ink"), "a tint of the spot color");
        let want = expected_tint(&app, 1.0, 2);
        assert!((tint - want).abs() < 1e-4, "{tint} vs {want}");
        assert_eq!(hex, g.grid[0][c + 2].to_hex(), "the color the cell shows");
        assert_eq!(hex, ink.tinted(tint).to_hex());
        // The painted tint follows an edit of the spot color, and so does the base.
        app.run("swatch.edit", json!({"name": "Ink", "color": "#cc0000"})).unwrap();
        let red = app.session.active().unwrap().doc.global_color("Ink").unwrap();
        assert_eq!(fill_link(&app).2, red.tinted(tint).to_hex(), "the art follows the spot color");
        assert_eq!(base_color(&app), red, "the base follows the spot color");
        frame(&mut app, &ctx);
        // Shades, another harmony row and the harmony strip are process colors.
        for id in [("cg-v", 0usize, c - 1), ("cg-v", 1, c + 1)] {
            let at = rect_of(&mut app, &ctx, id).center();
            click(&mut app, &ctx, at);
            assert_eq!(fill_link(&app).0, None, "{id:?}");
        }
        let at = rect_of(&mut app, &ctx, ("cg-h", 0usize)).center();
        click(&mut app, &ctx, at);
        assert_eq!(fill_link(&app).0, None, "the harmony strip");
        // Warm/Cool and a library limit: process colors.
        app.ui.color_guide.variation = Variation::WarmCool;
        frame(&mut app, &ctx);
        let at = rect_of(&mut app, &ctx, ("cg-v", 0usize, c + 1)).center();
        click(&mut app, &ctx, at);
        assert_eq!(fill_link(&app).0, None, "Warm/Cool");
        app.ui.color_guide.variation = Variation::TintsShades;
        app.run("ui.colorGuideLimit", json!({"library": "web-safe-216"})).unwrap();
        frame(&mut app, &ctx);
        let at = rect_of(&mut app, &ctx, ("cg-v", 0usize, c + 1)).center();
        click(&mut app, &ctx, at);
        assert_eq!(fill_link(&app).0, None, "limited to a library");
    }

    /// A base that is already a tint of the spot color gives lighter tints of it; a plain color with
    /// the spot color's values, a deleted swatch and a swatch that is no longer a spot color give
    /// process colors.
    #[test]
    fn a_tinted_spot_base_gives_lighter_tints_and_a_lost_link_gives_process_colors() {
        let mut app = spot_app(0.5);
        let ctx = egui::Context::default();
        frame(&mut app, &ctx);
        let c = guide(&app, &ctx).centre();
        let cell = |app: &VectorcraftApp, ctx: &egui::Context, col| cell_paint(app, &guide(app, ctx), 0, col).unwrap();
        let Paint::Solid { swatch, tint, .. } = cell(&app, &ctx, c + 1) else { panic!("a solid") };
        assert_eq!(swatch.as_deref(), Some("Ink"));
        let want = expected_tint(&app, 0.5, 1);
        assert!((tint - want).abs() < 1e-4 && tint < 0.5, "{tint} vs {want}");
        // The same values without the link.
        choose_base(&mut app, Color::from_hex("#0066cc").unwrap());
        assert_eq!(cell(&app, &ctx, c + 1), Paint::solid(guide(&app, &ctx).grid[0][c + 1]));
        // The spot color made a process color, then deleted.
        let link = Some(("Ink".to_string(), 1.0));
        let ink = app.session.active().unwrap().doc.global_color("Ink").unwrap();
        choose_linked_base(&mut app, ink, link.clone());
        assert!(matches!(cell(&app, &ctx, c + 1), Paint::Solid { swatch: Some(_), .. }));
        app.run("swatch.edit", json!({"name": "Ink", "spot": false, "global": false})).unwrap();
        assert!(matches!(cell(&app, &ctx, c + 1), Paint::Solid { swatch: None, .. }), "no longer a spot color");
        choose_linked_base(&mut app, ink, link.clone());
        assert!(app.ui.color_guide_spot.is_none(), "a process swatch gives no spot base");
        app.run("swatch.edit", json!({"name": "Ink", "spot": true})).unwrap();
        choose_linked_base(&mut app, ink, link);
        app.run("swatch.delete", json!({"name": "Ink"})).unwrap();
        assert!(matches!(cell(&app, &ctx, c + 1), Paint::Solid { swatch: None, .. }), "a deleted swatch");
    }

    /// A spot color base stays linked to its swatch when Swatch Options renames it: the tints link to
    /// the new name and follow an edit of it.
    #[test]
    fn a_renamed_spot_color_stays_the_base() {
        let mut app = spot_app(1.0);
        let ctx = egui::Context::default();
        frame(&mut app, &ctx);
        let c = guide(&app, &ctx).centre();
        app.run("swatch.edit", json!({"name": "Ink", "newName": "Deep Ink"})).unwrap();
        let Paint::Solid { swatch, .. } = cell_paint(&app, &guide(&app, &ctx), 0, c + 1).unwrap() else { panic!("a solid") };
        assert_eq!(swatch.as_deref(), Some("Deep Ink"));
        app.run("swatch.edit", json!({"name": "Deep Ink", "color": "#cc0000"})).unwrap();
        let red = app.session.active().unwrap().doc.global_color("Deep Ink").unwrap();
        assert_eq!(base_color(&app), red, "the base follows the renamed spot color");
    }

    /// Save Colors as Swatches and Save Color Group save a spot color base's tints as tint swatches
    /// linked to it, beside process colors for the other cells, each in one undo step.
    #[test]
    fn saved_spot_tints_are_tint_swatches_of_the_spot_color() {
        let mut app = spot_app(1.0);
        let ctx = egui::Context::default();
        frame(&mut app, &ctx);
        let g = guide(&app, &ctx);
        let c = g.centre();
        let undo = app.session.doc().unwrap().history.undo.len();
        set_pstate(&ctx, "cg-sel", vec![(0usize, c + 1), (0, c - 1), (0, c + 3)]);
        save_selected(&mut app, &ctx);
        let st = app.session.doc().unwrap();
        assert_eq!(st.history.undo.len(), undo + 1, "one undo step");
        let made: Vec<_> = st.doc.swatches[st.doc.swatches.len() - 3..].to_vec();
        let tint = |w: &vectorcraft_color::Swatch| w.tint_of().map(|(b, t)| (b.to_string(), t));
        assert_eq!(tint(&made[0]).map(|(b, _)| b).as_deref(), Some("Ink"));
        assert!((tint(&made[0]).unwrap().1 - expected_tint(&app, 1.0, 1)).abs() < 1e-4);
        assert_eq!(made[1].paint, Paint::solid(g.grid[0][c - 1]), "a shade is a process color");
        assert_eq!(tint(&made[2]).map(|(b, _)| b).as_deref(), Some("Ink"));
        assert!(made[0].name.starts_with("Ink ") && made[0].name.ends_with('%'), "{}", made[0].name);
        // A group of the selected cells, in grid order.
        set_pstate(&ctx, "cg-sel", vec![(1usize, c + 1), (0, c + 2)]);
        save_group(&mut app, &ctx);
        let group = app.session.doc().unwrap().doc.swatch_groups.last().cloned().unwrap();
        assert_eq!(group.swatches.len(), 2);
        assert_eq!(tint(&group.swatches[0]).map(|(b, _)| b).as_deref(), Some("Ink"), "the base row's tint");
        assert_eq!(group.swatches[1].paint, Paint::solid(g.grid[1][c + 1]), "another row is a process color");
        // The saved tints follow an edit of the spot color.
        app.run("swatch.edit", json!({"name": "Ink", "color": "#cc0000"})).unwrap();
        let d = &app.session.active().unwrap().doc;
        let red = d.global_color("Ink").unwrap();
        let first = d.swatch(&made[0].name).unwrap();
        assert_eq!(first.paint.color(), Some(red.tinted(first.tint_of().unwrap().1)));
    }

    /// A spot color base's tint dragged from the guide carries the tint, and the Swatches panel makes
    /// a tint swatch of it.
    #[test]
    fn a_dragged_spot_tint_becomes_a_tint_swatch() {
        let mut app = spot_app(1.0);
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        guide_frame(&mut app, &ctx, vec![], 0.0, true, ("cg-h", 0usize));
        let g = guide(&app, &ctx);
        let c = g.centre();
        let tile = |ctx: &egui::Context| ctx.read_response(egui::Id::unique(("swatch-tile", "Red"))).expect("the Red swatch is drawn").rect.center();
        let held = drag_from(&mut app, &ctx, ("cg-v", 0, c + 1), Some(tile(&ctx)), 1.0, true);
        let Some(PanelDrag::Paint { paint: Paint::Solid { swatch, tint, .. }, .. }) = held else { panic!("a solid paint: {held:?}") };
        assert_eq!(swatch.as_deref(), Some("Ink"));
        let newest = app.session.doc().unwrap().doc.swatches.last().cloned().expect("a swatch");
        let (base, made) = newest.tint_of().unwrap_or_else(|| panic!("a tint swatch: {newest:?} {}", app.ui.status));
        assert_eq!(base, "Ink");
        assert!((made - tint).abs() < 1e-4, "{made} vs {tint}");
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
