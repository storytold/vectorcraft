//! A panel's display sizes: the double triangle in its group's header steps through them
//! (`window.panel.cycleOptions`). A size is a set of the panel's Hide Options, Hide Thumbnails
//! and Show Options states, the ones its panel menu already sets, so the menu and the control
//! always agree. The sizes go from the largest to the smallest, and the smallest is followed by
//! the largest again. A panel whose parts are in no size's states goes to its largest size.

use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::panels::{part_hidden, set_part_hidden};

/// A part of a panel that can be hidden.
#[derive(Clone, Copy, Debug)]
enum Part {
    /// A Hide flag of the panels' hidden parts ([`part_hidden`]), true while hidden.
    Hide(&'static str),
    /// The Color panel's Hide Options, kept with its panel state.
    ColorOptions,
    /// The Info panel's Hide Options, which its panel menu flips too.
    InfoOptions,
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
    Sizes { panel: "info", parts: &[Part::InfoOptions], sizes: OPTIONS },
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

fn hidden(app: &VectorcraftApp, part: Part) -> bool {
    match part {
        Part::Hide(key) => part_hidden(app, key),
        Part::ColorOptions => app.ui.color_panel.hide_options,
        Part::InfoOptions => app.ui.info_hide_options,
        Part::FlattenerOptions => !app.ui.flattener_preview.show_options,
    }
}

fn set_hidden(app: &mut VectorcraftApp, part: Part, hide: bool) {
    match part {
        Part::Hide(key) => set_part_hidden(app, key, hide),
        Part::ColorOptions => app.ui.color_panel.hide_options = hide,
        Part::InfoOptions => app.ui.info_hide_options = hide,
        Part::FlattenerOptions => app.ui.flattener_preview.show_options = !hide,
    }
}

/// The size `panel` shows now (0 is the largest), or `None` when its parts match no size.
fn current(app: &VectorcraftApp, sizes: &Sizes) -> Option<usize> {
    let now: Vec<bool> = sizes.parts.iter().map(|&part| hidden(app, part)).collect();
    sizes.sizes.iter().position(|size| *size == now.as_slice())
}

/// `window.panel.cycleOptions {panel}`: show `panel` at its next display size, as the double
/// triangle in its group's header does. An unknown panel or a panel without display sizes is an
/// error, and then nothing changes. → {panel, size, sizes}: the size shown now (0 is the largest)
/// and how many sizes the panel has.
pub(crate) fn command(app: &mut VectorcraftApp, params: &Value) -> Result<Value, String> {
    let raw = params.get("panel").and_then(Value::as_str).unwrap_or_default();
    let panel = super::normalize(raw).ok_or_else(|| format!("unknown panel `{raw}`"))?;
    let sizes = sizes_of(panel).ok_or_else(|| format!("`{panel}` has no display sizes to cycle"))?;
    let next = current(app, sizes).map_or(0, |i| (i + 1) % sizes.sizes.len());
    let size = sizes.sizes.get(next).copied().unwrap_or_default();
    for (&part, &hide) in sizes.parts.iter().zip(size) {
        set_hidden(app, part, hide);
    }
    if let Some(ctx) = &app.ui_ctx {
        ctx.request_repaint();
    }
    Ok(json!({"panel": panel, "size": next, "sizes": sizes.sizes.len()}))
}
