//! Panels. Each panel reads engine state and acts only through `app.run(...)` (slider drags
//! preview through the session's interaction so one drag is one undo step).
//!
//! Every icon panel has a body (`show_icon_panel`) and a panel (≡) menu (`panel_menu`).

pub mod actions;
pub mod align;
pub mod appearance;
pub mod artboards;
pub mod asset_export;
pub mod attributes;
pub mod brushes;
pub mod character;
pub mod color;
pub mod color_guide;
pub mod color_themes;
pub mod css_properties;
pub mod doc_info;
pub mod flattener_preview;
pub mod glyphs;
pub mod gradient;
pub mod graphic_styles;
pub mod history;
pub mod image_trace;
pub mod info;
pub mod layers;
pub mod libraries;
pub mod library_panel;
pub mod links;
pub mod magic_wand;
pub mod navigator;
pub mod opentype;
pub mod paragraph;
pub mod pathfinder;
pub mod pattern_options;
pub mod properties;
pub mod separations;
pub mod stroke;
pub mod swatches;
pub mod symbols;
pub mod tabs;
pub mod text_styles;
pub mod transform;
pub mod transparency;

use egui::{Rect, Sense, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::{LiveCorners, Node, StrokeLayer};
use vectorcraft_engine::inspect::StrokeMixed;
use vectorcraft_geom::shapes::CornerKind;

use crate::theme::Tokens;
use crate::widgets::{Live, dim_label};
use crate::{VectorcraftApp, icons};

/// The first selected node (cloned), if any.
pub fn first_selected(app: &VectorcraftApp) -> Option<Node> {
    first_node(app).cloned()
}

/// The radius the Live Corners of path `n` show in the panels: that of the corners the panels set
/// (the Direct-Selected ones, else every corner), blank when they differ.
pub(crate) fn corner_radius(app: &VectorcraftApp, n: &Node) -> Option<f64> {
    corner_style(app, n).0
}

/// The radius and kind of the corners [`corner_radius`] reads, each None when they differ.
pub(crate) fn corner_style(app: &VectorcraftApp, n: &Node) -> (Option<f64>, Option<CornerKind>) {
    let partial = app.session.active().and_then(|d| d.selection.partial(n.id));
    LiveCorners::of(n).map_or((None, None), |c| c.style(&c.picked(partial)))
}

/// The width of a panel's number fields: the Transform fields', two to a row beside the reference
/// point, their labels and the W/H link, so every field in the panel is as wide (#696). Measured
/// from the full row, before its widgets.
pub(crate) fn field_width(ui: &Ui) -> f32 {
    // The reference point and its gap, the four labels and the W/H link.
    const AROUND: f32 = 37.0 + 70.0;
    ((ui.available_width() - AROUND) / 2.0).clamp(60.0, 110.0)
}

/// The Corner Radius field of a live rectangle's properties: [`corner_radius`], which a new value
/// sets on those corners.
pub(crate) fn corner_radius_row(app: &mut VectorcraftApp, ui: &mut Ui, n: &Node, id: &str) {
    let fw = field_width(ui);
    ui.horizontal(|ui| {
        dim_label(ui, tl!("Corner Radius:"));
        corner_radius_field(app, ui, n, id, fw);
    });
}

/// [`corner_radius_row`]'s field, `width` wide.
pub(crate) fn corner_radius_field(app: &mut VectorcraftApp, ui: &mut Ui, n: &Node, id: impl std::hash::Hash + std::fmt::Debug, width: f32) {
    if let Some(r) = crate::widgets::num_field(ui, id, corner_radius(app, n), app.session.general_unit(), width) {
        app.run("object.setLiveShape", json!({"radius": r})).ok();
    }
}

/// Number of selected objects.
pub(crate) fn selection_len(app: &VectorcraftApp) -> usize {
    app.session.active().map(|d| d.selection.len()).unwrap_or(0)
}

pub fn show_icon_panel(app: &mut VectorcraftApp, ui: &mut Ui, id: &str) {
    match id {
        "swatches" => swatches::show(app, ui),
        "color" => color::show(app, ui),
        "colorGuide" => color_guide::show(app, ui),
        "stroke" => stroke::show(app, ui),
        "transparency" => transparency::show(app, ui),
        "appearance" => appearance::show(app, ui),
        "graphicStyles" => graphic_styles::show(app, ui),
        "align" => align::show(app, ui),
        "pathfinder" => pathfinder::show(app, ui),
        "transform" => transform::show(app, ui),
        "history" => history::show(app, ui),
        "actions" => actions::show(app, ui),
        "info" => info::show(app, ui),
        "separations" => separations::show(app, ui),
        "artboards" => artboards::show(app, ui),
        "gradient" => gradient::show(app, ui),
        "character" => character::show(app, ui),
        "paragraph" => paragraph::show(app, ui),
        "glyphs" => glyphs::show(app, ui),
        "navigator" => navigator::show(app, ui),
        "brushes" => brushes::show(app, ui),
        "symbols" => symbols::show(app, ui),
        "patternOptions" => pattern_options::show(app, ui),
        "imageTrace" => image_trace::show(app, ui),
        "openType" => opentype::show(app, ui),
        "docInfo" => doc_info::show(app, ui),
        "charStyles" => text_styles::show(app, ui, text_styles::Kind::Char),
        "paraStyles" => text_styles::show(app, ui, text_styles::Kind::Para),
        "magicWand" => magic_wand::show(app, ui),
        "tabs" => tabs::show(app, ui),
        flattener_preview::ID => flattener_preview::show(app, ui),
        "attributes" => attributes::show(app, ui),
        "colorThemes" => color_themes::show(app, ui),
        links::ID => links::show(app, ui),
        asset_export::ID => asset_export::show(app, ui),
        css_properties::ID => css_properties::show(app, ui),
        _ => {
            dim_label(ui, tl!("This panel is on the roadmap (see the parity plan)."));
        }
    }
}

/// Items of a panel's (≡) menu. Unimplemented Illustrator items are listed disabled. False for a
/// panel without items of its own.
pub fn panel_menu_items(app: &mut VectorcraftApp, ui: &mut Ui, id: &str) -> bool {
    match id {
        "swatches" => swatches::menu(app, ui),
        "color" => color::menu(app, ui),
        "colorGuide" => color_guide::menu(app, ui),
        "stroke" => stroke::menu(app, ui),
        "transparency" => transparency::menu(app, ui),
        "appearance" => appearance::menu(app, ui),
        "graphicStyles" => graphic_styles::menu(app, ui),
        "align" => align::menu(app, ui),
        "pathfinder" => pathfinder::menu(app, ui),
        "transform" => transform::menu(app, ui),
        "history" => history::menu(app, ui),
        "info" => info::menu(app, ui),
        "separations" => separations::menu(app, ui),
        "artboards" => artboards::menu(app, ui),
        "gradient" => gradient::menu(app, ui),
        "character" => character::menu(app, ui),
        "paragraph" => paragraph::menu(app, ui),
        "glyphs" => glyphs::menu(app, ui),
        "navigator" => navigator::menu(app, ui),
        "brushes" => brushes::menu(app, ui),
        "symbols" => symbols::menu(app, ui),
        "patternOptions" => pattern_options::menu(app, ui),
        "imageTrace" => image_trace::menu(app, ui),
        "openType" => opentype::menu(app, ui),
        "docInfo" => doc_info::menu(app, ui),
        "charStyles" => text_styles::menu(app, ui, text_styles::Kind::Char),
        "paraStyles" => text_styles::menu(app, ui, text_styles::Kind::Para),
        "magicWand" => magic_wand::menu(app, ui),
        "tabs" => tabs::menu(app, ui),
        "libraries" => libraries::menu(app, ui),
        flattener_preview::ID => flattener_preview::menu(app, ui),
        "attributes" => attributes::menu(app, ui),
        "colorThemes" => color_themes::menu(app, ui),
        "layers" => layers::menu(app, ui),
        links::ID => links::menu(app, ui),
        asset_export::ID => asset_export::menu(app, ui),
        css_properties::ID => css_properties::menu(app, ui),
        _ => return false,
    }
    true
}

/// The ≡ panel-menu button drawn into `rect` (the right end of a panel's title/tab strip).
pub fn panel_menu(app: &mut VectorcraftApp, ui: &mut Ui, id: &str, rect: Rect) {
    let t = Tokens::get(ui.ctx());
    let resp = ui.interact(rect, ui.id().with(("panel-menu", id)), Sense::click());
    icons::paint(ui, "menu", rect.shrink(1.0), if resp.hovered() { t.text_strong } else { t.text_dim });
    let resp = resp.on_hover_text(tl!("Panel menu"));
    egui::Popup::menu(&resp).show(|ui| {
        crate::widgets::menu_scroll(ui, |ui| {
            ui.set_min_width(220.0);
            if panel_menu_items(app, ui, id) {
                ui.separator();
            }
            // The panel floats out of the dock or goes back in, as dragging its tab does.
            let (label, cmd) = if crate::floating::group_of(&app.ui, id).is_some() {
                (tl!("Dock Panel"), "window.panel.dock")
            } else {
                (tl!("Float Panel"), "window.panel.float")
            };
            if crate::widgets::menu_item(ui, label, true, false)
                && let Err(e) = app.run(cmd, json!({ "panel": id }))
            {
                app.ui.status = e;
            }
        });
    });
}

pub fn libraries(app: &mut VectorcraftApp, ui: &mut Ui) {
    libraries::show(app, ui);
}

// ---------- shared helpers ----------

/// The paint command of the active proxy (Fill or Stroke), or of the inactive one.
pub(crate) fn proxy_cmd(app: &VectorcraftApp, inactive: bool) -> &'static str {
    if app.session.fill_active != inactive { "paint.setFill" } else { "paint.setStroke" }
}

/// The first selected node, borrowed (for per-frame reads that need no copy of it).
fn first_node(app: &VectorcraftApp) -> Option<&Node> {
    let st = app.session.active()?;
    st.selection.objects.first().and_then(|id| st.doc.node(*id))
}

/// Is Alt held? A click on a colour then paints the inactive proxy.
pub(crate) fn alt_held(ui: &Ui) -> bool {
    ui.input(|i| i.modifiers.alt)
}

/// Apply a clicked colour, swatch or None (`params`: `{color}`, `{swatch}` or `{none}`) to the
/// active proxy, or with Alt held to the inactive one, which stays behind.
pub(crate) fn apply_click(app: &mut VectorcraftApp, ui: &Ui, mut params: Value) {
    let alt = alt_held(ui);
    params["focus"] = json!(!alt);
    app.run(proxy_cmd(app, alt), params).ok();
}

/// Fill and stroke as the proxies show them ([`vectorcraft_engine::Session::proxy_paints`]: the
/// Appearance panel's active item for the proxy of its kind, else the first selected object's, a
/// group's first painted object's, else the defaults for new art).
pub(crate) fn current_paints(app: &VectorcraftApp) -> (Paint, Paint) {
    app.session.proxy_paints()
}

/// Whether the selected objects' fills and strokes differ (the proxies show "?"), cached per
/// document revision and Appearance panel item.
pub(crate) fn mixed_paints(app: &VectorcraftApp, ctx: &egui::Context) -> (bool, bool) {
    let Some(st) = app.session.active() else { return (false, false) };
    let key = (st.uid, st.revision, app.session.appearance_item());
    match pstate::<Option<((u64, u64, Option<usize>), (bool, bool))>>(ctx, "proxy-mixed") {
        Some((k, mixed)) if k == key => mixed,
        _ => {
            let mixed = app.session.proxy_mixed();
            set_pstate(ctx, "proxy-mixed", Some((key, mixed)));
            mixed
        }
    }
}

/// The stroke the Stroke panel, Control bar and Properties show: while the Type tool edits text,
/// the selected characters' stroke, else [`vectorcraft_engine::Session::shown_stroke`].
pub(crate) fn current_stroke(app: &VectorcraftApp) -> Option<StrokeLayer> {
    if character::text_editing(app).is_some() {
        return character::text_style(app).map(|(c, _)| c.stroke_layer());
    }
    app.session.shown_stroke()
}

/// Which Stroke panel values the selection doesn't share and whether Align Stroke applies
/// ([`vectorcraft_engine::DocState::stroke_mixed`]), cached per document revision. While the Type
/// tool edits text, the selected characters' stroke shows as it is.
pub(crate) fn stroke_mixed(app: &VectorcraftApp, ctx: &egui::Context) -> StrokeMixed {
    let Some(st) = app.session.active().filter(|_| character::text_editing(app).is_none()) else { return StrokeMixed::default() };
    let key = (st.uid, st.revision);
    match pstate::<Option<((u64, u64), StrokeMixed)>>(ctx, "stroke-mixed") {
        Some((k, mixed)) if k == key => mixed,
        _ => {
            let mixed = st.stroke_mixed();
            set_pstate(ctx, "stroke-mixed", Some((key, mixed)));
            mixed
        }
    }
}

/// Opacity and blend mode as the Transparency panel and Control bar show them: the Appearance
/// panel's active item's, else the first selected object's.
pub(crate) fn current_transparency(app: &VectorcraftApp) -> Option<(f32, BlendMode)> {
    let n = first_node(app)?;
    Some(match app.session.appearance_item().and_then(|i| n.appearance.items.get(i)) {
        Some(it) => (it.opacity(), it.blend()),
        None => (n.opacity, n.blend),
    })
}

/// Is the active proxy "?" (the selected objects' paints differ)?
pub(crate) fn active_mixed(app: &VectorcraftApp, ctx: &egui::Context) -> bool {
    let (f, s) = mixed_paints(app, ctx);
    if app.session.fill_active { f } else { s }
}

/// The paint behind the active proxy.
pub(crate) fn active_paint(app: &VectorcraftApp) -> Paint {
    let (f, s) = current_paints(app);
    if app.session.fill_active { f } else { s }
}

/// Draw the Fill/Stroke proxy and handle its clicks through commands.
pub(crate) fn proxy(app: &mut VectorcraftApp, ui: &mut Ui, size: f32) {
    let (f, s) = current_paints(app);
    let mixed = mixed_paints(app, ui.ctx());
    let c = crate::widgets::fill_stroke_proxy(ui, &f, &s, mixed, app.session.fill_active, size);
    if (c.fill && !app.session.fill_active) || (c.stroke && app.session.fill_active) {
        app.run("paint.toggleActive", json!({})).ok();
    }
    if c.swap {
        app.run("paint.swap", json!({})).ok();
    }
    if c.default {
        app.run("paint.default", json!({})).ok();
    }
    if let Some(stroke) = c.pick {
        app.run("ui.colorPicker", json!({ "stroke": stroke })).ok();
    }
}

/// The Fill (or `stroke`) chip of the Control bar and the Properties panel, `size` square (with a
/// `chevron` after it): the proxy's paint, "?" when the selected objects' paints differ. A click
/// brings that proxy forward and opens a popover with the Swatches panel; Shift-click (or the
/// popover's toggle) shows the Color panel's mixer instead.
pub(crate) fn paint_chip(app: &mut VectorcraftApp, ui: &mut Ui, stroke: bool, size: f32, chevron: bool) {
    let (fill, stroke_paint) = current_paints(app);
    let mixed = mixed_paints(app, ui.ctx());
    let (paint, mixed) = if stroke { (stroke_paint, mixed.1) } else { (fill, mixed.0) };
    let resp = crate::widgets::chip_button(ui, size, chevron, |ui, chip| {
        crate::widgets::proxy_chip(ui, chip, &paint, mixed, stroke.then_some(size * 0.25));
    });
    let resp = resp.on_hover_text(if stroke {
        "Stroke: click for swatches, Shift-click for the color mixer"
    } else {
        "Fill: click for swatches, Shift-click for the color mixer"
    });
    if resp.clicked() {
        app.run("paint.toggleActive", json!({ "fill": !stroke })).ok();
        set_pstate(ui.ctx(), MIXER, ui.input(|i| i.modifiers.shift));
    }
    // A popover that keeps its own open state: the Swatches body opens menus of its own (Swatch
    // Libraries, Show Swatch Kinds), which a remembered egui popup would close with them (#536).
    crate::widgets::popover(&resp, resp.clicked(), |ui| paint_popover(app, ui));
}

/// The chip popovers' width (a narrow Swatches or Color panel).
const POPOVER_WIDTH: f32 = 248.0;
/// Panel state: the chip popovers show the Color panel's mixer instead of the swatches.
const MIXER: &str = "paint-popover-mixer";

/// The body of a Fill/Stroke chip popover: a Swatches / Color mixer toggle above that panel's body.
fn paint_popover(app: &mut VectorcraftApp, ui: &mut Ui) {
    ui.set_width(POPOVER_WIDTH);
    let mixer: bool = pstate(ui.ctx(), MIXER);
    ui.horizontal(|ui| {
        for (icon, tip, m) in [("swatch-book", "Swatches", false), ("palette", "Color Mixer (Shift-click the chip)", true)] {
            if crate::widgets::icon_button(ui, icon, tip, mixer == m, 22.0).clicked() {
                set_pstate(ui.ctx(), MIXER, m);
            }
        }
    });
    crate::widgets::divider(ui);
    if mixer {
        color::show(app, ui);
    } else {
        swatches::popover(app, ui);
    }
}

/// Run `cmd` live: while dragging, preview on top of an interaction snapshot; on release, commit
/// it as one undo step. Falls back to a plain run when no document is open.
pub(crate) fn live_run(app: &mut VectorcraftApp, label: &str, cmd: &str, params: Value, phase: Live) {
    match phase {
        Live::Idle => {}
        Live::Dragging | Live::Released => {
            if app.session.begin_interaction(label).is_ok() {
                if let Err(e) = app.session.preview(cmd, &params) {
                    app.ui.status = e.to_string();
                }
                if phase == Live::Released {
                    app.session.commit_interaction().ok();
                }
                app.sync_views();
            } else if phase == Live::Released {
                app.run(cmd, params).ok();
            }
        }
    }
}

/// Per-panel UI state kept in egui memory (not document state).
pub(crate) fn pstate<T: Clone + Default + Send + Sync + 'static>(ctx: &egui::Context, key: &str) -> T {
    ctx.data(|d| d.get_temp::<T>(egui::Id::new(("panel-state", key)))).unwrap_or_default()
}
pub(crate) fn set_pstate<T: Clone + Send + Sync + 'static>(ctx: &egui::Context, key: &str, v: T) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(("panel-state", key)), v));
}

/// [`crate::i18n::label_or_name`] in the UI language.
pub(crate) fn label_or_name(s: &str, built_in: bool) -> &str {
    crate::i18n::label_or_name(crate::i18n::current(), s, built_in)
}

/// "Recent Colors" header + a row of chips (the Session's recent colours, which every paint
/// command feeds); returns the one clicked, for the caller to apply.
pub(crate) fn recent_colors_row(app: &VectorcraftApp, ui: &mut Ui) -> Option<Color> {
    let t = Tokens::get(ui.ctx());
    crate::widgets::subheader(ui, tl!("Recent Colors"));
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    ui.painter().rect_stroke(r, 0.0, egui::Stroke::new(1.0, t.input_border), egui::StrokeKind::Inside);
    let mut chosen = None;
    for (i, c) in app.session.recent_colors.iter().enumerate() {
        let cell = Rect::from_min_size(r.min + vec2(3.0 + i as f32 * 19.0, 3.0), vec2(16.0, 16.0));
        if cell.right() > r.right() - 2.0 {
            break;
        }
        let resp = ui.interact(cell, ui.id().with(("recent", i)), Sense::click());
        crate::widgets::swatch_tile(ui, cell, &Paint::solid(*c), false, resp.hovered());
        if resp.on_hover_text(c.to_hex()).clicked() {
            chosen = Some(*c);
        }
    }
    chosen
}

/// A colour as command JSON, keeping its model.
pub(crate) use vectorcraft_tools::params::color_json;

/// Paint as command params (`{color}`, `{swatch, tint?}`, `{none}`, `{gradient}` (lossless) or
/// `{pattern}`).
pub(crate) fn paint_params(p: &Paint) -> Value {
    match p {
        Paint::None => json!({"none": true}),
        Paint::Solid { swatch: Some(n), tint, .. } if *tint < 1.0 => json!({"swatch": n, "tint": tint * 100.0}),
        Paint::Solid { swatch: Some(n), .. } => json!({"swatch": n}),
        Paint::Solid { color, .. } => json!({"color": color_json(color)}),
        Paint::Gradient(g) => json!({"gradient": vectorcraft_tools::params::gradient_params(g)}),
        Paint::Pattern { pattern, .. } => json!({"pattern": pattern}),
    }
}

/// Egui colour of a document colour.
pub(crate) fn c32(c: &Color) -> egui::Color32 {
    let [r, g, b, _] = c.to_rgba8(1.0);
    egui::Color32::from_rgb(r, g, b)
}

/// A labelled empty-state message used by list panels.
pub(crate) fn empty_state(ui: &mut Ui, icon: &str, title: &str, body: &str) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(12.0);
    ui.vertical_centered(|ui| {
        icons::icon(ui, icon, 28.0, t.text_disabled);
        ui.add_space(4.0);
        ui.label(egui::RichText::new(tl!(title)).size(12.5).color(t.text));
        ui.label(egui::RichText::new(tl!(body)).size(11.5).color(t.text_dim));
    });
    ui.add_space(12.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paint_params_roundtrip_shapes() {
        assert_eq!(paint_params(&Paint::None), json!({"none": true}));
        let p = paint_params(&Paint::solid(Color::cmyk(0.1, 0.2, 0.3, 0.4)));
        assert!((p["color"]["k"].as_f64().unwrap() - 0.4).abs() < 1e-6);
        let g = Paint::Gradient(Box::new(vectorcraft_color::GradientPaint::new(Default::default())));
        let p = paint_params(&g);
        assert_eq!(p["gradient"]["kind"], "linear");
        assert_eq!(p["gradient"]["stops"].as_array().unwrap().len(), 2);
        assert_eq!(paint_params(&vectorcraft_doc::pattern::pattern_paint("Dots")), json!({"pattern": "Dots"}));
        assert_eq!(p["gradient"]["stops"][0]["midpoint"], json!(0.5));
        let tint = Paint::Solid { color: Color::gray(0.5), swatch: Some("Ink".into()), tint: 0.5 };
        assert_eq!(paint_params(&tint), json!({"swatch": "Ink", "tint": 50.0}), "a tint keeps its percentage");
    }
}
#[cfg(test)]
mod tests_appearance;
#[cfg(test)]
mod tests_asset_export;
#[cfg(test)]
mod tests_constrain;
#[cfg(test)]
mod tests_css_properties;
#[cfg(test)]
mod tests_effectedit;
#[cfg(test)]
mod tests_fontsize;
#[cfg(test)]
mod tests_freeform;
#[cfg(test)]
mod tests_links;
#[cfg(test)]
mod tests_maskview;
#[cfg(test)]
mod tests_stroke;
#[cfg(test)]
mod tests_strokedepth;
#[cfg(test)]
mod tests_strokeux;
#[cfg(test)]
mod tests_style_libraries;
#[cfg(test)]
mod tests_units;
