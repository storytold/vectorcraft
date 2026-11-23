//! A panel's display sizes: the double triangle at the end of its tab strip steps through them
//! (`window.panel.cycleOptions`). A size is a set of the panel's Hide Options, Hide Thumbnails
//! and Show Options states, the ones its panel menu already sets, so the menu and the control
//! always agree. The sizes go from the largest to the smallest, and the smallest is followed by
//! the largest again. A panel whose parts are in no size's states goes to its largest size.

use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::panels::{pstate, set_pstate};

/// A part of a panel that can be hidden.
#[derive(Clone, Copy, Debug)]
enum Part {
    /// A Hide flag the panel keeps with its panel state ([`pstate`]), true while hidden.
    Hide(&'static str),
    /// The Color panel's Hide Options, kept with the preferences.
    ColorOptions,
    /// The Flattener Preview panel's Show Options.
    FlattenerOptions,
}

/// A panel that has display sizes: its parts, and the parts each size hides, largest first.
struct Sizes {
    panel: &'static str,
    parts: &'static [Part],
    sizes: &'static [&'static [bool]],
}

/// Options shown, then hidden.
const OPTIONS: &[&[bool]] = &[&[false], &[true]];

/// The panels with display sizes.
const PANELS: &[Sizes] = &[
    Sizes { panel: "align", parts: &[Part::Hide("align-hide-options")], sizes: OPTIONS },
    Sizes { panel: "attributes", parts: &[Part::Hide("attr-hide-options")], sizes: OPTIONS },
    Sizes { panel: "character", parts: &[Part::Hide("ch-hide-options")], sizes: OPTIONS },
    Sizes { panel: "color", parts: &[Part::ColorOptions], sizes: OPTIONS },
    Sizes { panel: crate::panels::flattener_preview::ID, parts: &[Part::FlattenerOptions], sizes: OPTIONS },
    Sizes { panel: "gradient", parts: &[Part::Hide("gradient-hide-options")], sizes: OPTIONS },
    Sizes { panel: "info", parts: &[Part::Hide("info-hide-options")], sizes: OPTIONS },
    Sizes { panel: "paragraph", parts: &[Part::Hide("pa-hide-options")], sizes: OPTIONS },
    Sizes { panel: "stroke", parts: &[Part::Hide("stroke-hide-options")], sizes: OPTIONS },
    Sizes { panel: "transform", parts: &[Part::Hide("xf-hide-options")], sizes: OPTIONS },
    // Thumbnails and options, then thumbnails alone, then neither.
    Sizes {
        panel: "transparency",
        parts: &[Part::Hide("tr-hide-thumbs"), Part::Hide("tr-hide-options")],
        sizes: &[&[false, false], &[false, true], &[true, true]],
    },
];

fn sizes_of(panel: &str) -> Option<&'static Sizes> {
    PANELS.iter().find(|s| s.panel == panel)
}

/// Does `panel` have display sizes to step through?
pub(crate) fn has_sizes(panel: &str) -> bool {
    sizes_of(panel).is_some()
}

fn hidden(app: &VectorcraftApp, ctx: &egui::Context, part: Part) -> bool {
    match part {
        Part::Hide(key) => pstate(ctx, key),
        Part::ColorOptions => app.ui.color_hide_options,
        Part::FlattenerOptions => !app.ui.flattener_preview.show_options,
    }
}

fn set_hidden(app: &mut VectorcraftApp, ctx: &egui::Context, part: Part, hide: bool) {
    match part {
        Part::Hide(key) => set_pstate(ctx, key, hide),
        Part::ColorOptions => app.ui.color_hide_options = hide,
        Part::FlattenerOptions => app.ui.flattener_preview.show_options = !hide,
    }
}

/// The size `panel` shows now (0 is the largest), or `None` when its parts match no size.
fn current(app: &VectorcraftApp, ctx: &egui::Context, sizes: &Sizes) -> Option<usize> {
    let now: Vec<bool> = sizes.parts.iter().map(|&part| hidden(app, ctx, part)).collect();
    sizes.sizes.iter().position(|size| *size == now.as_slice())
}

/// `window.panel.cycleOptions {panel}`: show `panel` at its next display size, as the double
/// triangle on its tab strip does. An unknown panel, a panel without display sizes, or no panels
/// drawn yet is an error, and then nothing changes. → {panel, size, sizes}: the size shown now
/// (0 is the largest) and how many sizes the panel has.
pub(crate) fn command(app: &mut VectorcraftApp, params: &Value) -> Result<Value, String> {
    let raw = params.get("panel").and_then(Value::as_str).unwrap_or_default();
    let panel = super::normalize(raw).ok_or_else(|| format!("unknown panel `{raw}`"))?;
    let sizes = sizes_of(panel).ok_or_else(|| format!("`{panel}` has no display sizes to cycle"))?;
    let ctx = app.ui_ctx.clone().ok_or("the panels are not shown")?;
    let next = current(app, &ctx, sizes).map_or(0, |i| (i + 1) % sizes.sizes.len());
    for (&part, &hide) in sizes.parts.iter().zip(sizes.sizes[next]) {
        set_hidden(app, &ctx, part, hide);
    }
    ctx.request_repaint();
    Ok(json!({"panel": panel, "size": next, "sizes": sizes.sizes.len()}))
}

/// Width of the double triangle's cell at the start of a group header.
pub(crate) const CELL: f32 = 20.0;

/// Draw the double triangle for `panel` centered in `cell`; a click shows its next size.
pub(crate) fn button(app: &mut VectorcraftApp, ui: &mut egui::Ui, cell: egui::Rect, panel: &str) {
    let rect = egui::Rect::from_center_size(cell.center(), egui::vec2(16.0_f32.min(cell.width()), 22.0_f32.min(cell.height())));
    if !rect.is_positive() {
        return;
    }
    let label = tl!("Cycle Display Sizes");
    let response = ui.interact(rect, ui.scope_id().with(("cycle-display-sizes", panel)), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    let tokens = crate::theme::Tokens::get(ui.ctx());
    let color = if response.hovered() || response.has_focus() { tokens.text_strong } else { tokens.text_dim };
    let c = rect.center();
    let (w, h, gap) = (3.5, 3.5, 1.5);
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let up = vec![egui::pos2(c.x - w, c.y - gap), egui::pos2(c.x + w, c.y - gap), egui::pos2(c.x, c.y - gap - h)];
    let down = vec![egui::pos2(c.x - w, c.y + gap), egui::pos2(c.x, c.y + gap + h), egui::pos2(c.x + w, c.y + gap)];
    for points in [up, down] {
        painter.add(egui::Shape::convex_polygon(points, color, egui::Stroke::NONE));
    }
    if response.on_hover_text(label).clicked()
        && let Err(error) = app.run("window.panel.cycleOptions", json!({"panel": panel}))
    {
        app.ui.status = error;
    }
}
