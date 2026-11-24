//! The Properties panel: sections that change with the selection.

use craft::behavior::{ColumnWidth, IconId};
use craft::ui::buttons::{Button, IconButton};
use craft::ui::contract::{LabelScheme, Props as _, Show as _, ShowContainer as _};
use craft::ui::layout_components::{Grid, Row, Stack};
use craft::ui::patterns::panels::Inspector;

use craft::ui::runtime::commands::CommandRef;
use craft::ui::sections::HeaderStrength;
use egui::Ui;
use serde_json::json;
use vectorcraft_doc::{LiveShape, Node, NodeKind};

use super::{first_selected, section};
use crate::VectorcraftApp;

/// Run-only state of the Properties panel.
#[derive(Clone, Debug, Default)]
pub struct PropertiesUi {
    /// The Global Edit Options popover is open.
    pub global_edit_open: bool,
    /// The panel's More Options popover is open.
    pub more_options_open: bool,
}

/// Two equal columns: the Quick Actions and the paired buttons of the sections.
const HALVES: [ColumnWidth; 2] = [ColumnWidth::fill(1.0); 2];

/// One Quick Actions button: its label and the command it runs.
pub(crate) struct QuickAction {
    /// The button's label.
    pub label: &'static str,
    /// The command the button runs.
    pub command: &'static str,
}

/// The panel for what is selected: the selection's kind as its heading, then its sections (Edit
/// Artboards' with the Artboard tool, the document's with nothing selected).
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let inspector = Inspector::new(tl!("Properties")).title_strength(HeaderStrength::Quiet).empty(tl!("No document open"));
    let Some(key) = target_key(app) else {
        inspector.show(ui, |_, _| {});
        return;
    };
    let n_sel = app.session.active().map_or(0, |st| st.selection.len());
    let anchors = crate::chrome::anchor_controls(app);
    let first = first_selected(app);
    // Edit Artboards (the Artboard tool): the active artboard, whatever is selected.
    let editing_artboards = app.session.tool_id() == "artboard";
    // Ruler guides selected on their own (a click on one): their position.
    let guides = super::transform::selected_guides(app);
    let header = heading(first.as_deref(), n_sel, editing_artboards, guides.is_some());
    inspector.target(key).header(header).show(ui, |ui, _| {
        // The reference panel's labels end in a colon beside fields that keep their own width.
        Stack::new().fill_width().labels(LabelScheme::Form).show(ui, |ui, _| {
            if editing_artboards {
                artboard_sections::artboard_sections(app, ui);
            } else if first.is_none() {
                match guides {
                    Some(guides) => transform_section::guide_sections(app, ui, guides),
                    None => document_sections::document_sections(app, ui),
                }
            } else {
                body(app, ui, first.as_deref(), n_sel, anchors);
            }
        });
    });
}

/// The document's uid: production's field ids do not change with the selection, so neither does the key and the
/// scroll position stays.
fn target_key(app: &VectorcraftApp) -> Option<u64> {
    app.session.active().map(|st| st.uid)
}

/// The sections of a selection, in the reference order.
fn body(app: &mut VectorcraftApp, ui: &mut Ui, first: Option<&Node>, n_sel: usize, anchors: crate::chrome::AnchorControls) {
    let kind = first.map(|n| &n.kind);
    if n_sel == 1
        && let Some(NodeKind::Group { clip: true, children }) = kind
    {
        let has_contents = children.len() > 1;
        Grid::new(&HALVES).show(ui, |ui, _| {
            Button::command("object.clippingMask.editMask").label(tl!("Edit Clipping Path")).show(ui);
            if has_contents {
                Button::command("object.clippingMask.editContents").label(tl!("Edit Contents")).show(ui);
            }
        });
    }
    transform_section(app, ui);
    transform_section::corner_rows(app, ui);
    // The Gradient tool: the gradient's type and, for a freeform one, how clicks add points (#921).
    if app.session.tool_id() == "gradient" {
        section(tl!("Gradient")).show(ui, |ui, _| {
            super::gradient::type_row(app, ui);
            super::gradient::draw_radios(app, ui);
        });
    }
    let is_image = n_sel == 1 && matches!(kind, Some(NodeKind::Image(_)));
    if is_image {
        object_sections::image_section(app, ui);
    }
    if let Some((preset, view)) = crate::panels::image_trace::selected_trace(app) {
        object_sections::trace_section(app, ui, &preset, view);
    }
    if let Some(oop) = object_sections::selected_objects_on_path(app) {
        object_sections::objects_on_path_section(app, ui, &oop);
    }
    graph::section(app, ui);
    if matches!(kind, Some(NodeKind::Text(_))) {
        type_sections(app, ui);
    }
    if crate::dialogs::dimension::selected(app).is_some() {
        crate::dialogs::dimension::properties_section(app, ui);
    }
    appearance_section::appearance_section(app, ui);
    section(tl!("Align")).show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            super::align::align_buttons(app, ui);
            // The rest (Distribute Objects, Distribute Spacing, Align To) is the Align panel's.
            more_options(ui, "align", tl!("More Align options"));
        });
    });
    if n_sel > 1 {
        section(tl!("Pathfinder")).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                for (icon, tip, op) in [
                    ("squares-unite", tl!("Unite"), "unite"),
                    ("squares-subtract", tl!("Minus Front"), "minusFront"),
                    ("squares-intersect", tl!("Intersect"), "intersect"),
                    ("squares-exclude", tl!("Exclude"), "exclude"),
                ] {
                    IconButton::command(IconId::new(icon), CommandRef::new(format!("object.pathfinder.{op}"))).label(tip).show(ui);
                }
                more_options(ui, "pathfinder", tl!("More Pathfinder options"));
            });
        });
    }
    if anchors != crate::chrome::AnchorControls::None {
        section(tl!("Anchor Point")).show(ui, |ui, _| crate::chrome::anchor_buttons(app, ui, anchors));
    }
    let actions = quick_actions(app);
    section(tl!("Quick Actions")).show(ui, |ui, _| {
        if is_image {
            crate::panels::image_trace::trace_button(app, ui);
        }
        Grid::new(&HALVES).show(ui, |ui, _| {
            for a in &actions {
                Button::command(a.command).label(a.label).show(ui);
            }
        });
        appearance_section::global_edit_row(app, ui);
    });
}

/// The ellipsis button that shows the panel `panel` for the section's further options.
fn more_options(ui: &mut Ui, panel: &str, tip: &'static str) {
    IconButton::command(IconId::new("ellipsis"), CommandRef::with_params("window.panel", json!({ "panel": panel }))).label(tip).show(ui);
}

/// The Quick Actions the selection offers, in order. Lock and Isolate Group are left out while their commands are
/// off (in isolation mode, while an opacity mask is edited).
pub(crate) fn quick_actions(app: &mut VectorcraftApp) -> Vec<QuickAction> {
    let n_sel = super::selection_len(app);
    let first = first_selected(app);
    let kind = first.as_deref().map(|n| &n.kind);
    let is_group = matches!(kind, Some(NodeKind::Group { .. }));
    let is_live = matches!(kind, Some(NodeKind::Path { live: Some(l), .. }) if !matches!(**l, LiveShape::Path { .. }));
    // Blend Options for one selected blend, Merge Live Paint for a Live Paint group selected with more to merge into
    // it, as the Properties panel overview lists them
    // (<https://helpx.adobe.com/illustrator/desktop/get-started/learn-the-basics/properties-panel-overview.html>,
    // <https://helpx.adobe.com/illustrator/using/blending-objects.html>, #4817).
    let blend_options = n_sel == 1 && matches!(kind, Some(NodeKind::Blend { .. }));
    let merge_live_paint = n_sel > 1 && can_merge_live_paint(app);
    // Repeat Options for one selected repeat, or one object of its source art in isolation mode (#5181).
    let repeat_options = n_sel == 1 && crate::menus::enabled(app, "object.repeat.options");
    let is_image = n_sel == 1 && matches!(kind, Some(NodeKind::Image(_)));
    let mut actions = vec![];
    let mut add = |on: bool, label: &'static str, command: &'static str| {
        if on {
            actions.push(QuickAction { label, command });
        }
    };
    add(n_sel > 1, tl!("Group"), "object.group");
    add(n_sel > 1, tl!("Make Clipping Mask"), "object.clippingMask.make");
    add(merge_live_paint, tl!("Merge Live Paint"), "livePaint.merge");
    add(blend_options, tl!("Blend Options"), "ui.blendOptions");
    add(repeat_options, tl!("Repeat Options"), "object.repeat.options");
    add(is_group, tl!("Ungroup"), "object.ungroup");
    add(is_group, tl!("Isolate Group"), "object.isolate");
    add(is_live, tl!("Expand Shape"), "object.expandShape");
    // A dynamic symbol instance with appearance changes of its own: Reset shows the symbol's again.
    add(super::symbols::changed_instances_selected(app), tl!("Reset"), "symbol.instance.reset");
    add(multi_color(app), tl!("Recolor"), "ui.recolorDialog");
    add(is_image, tl!("Crop Image"), "ui.cropImage");
    add(is_image, tl!("Mask"), "object.maskImage");
    // Remove Background at once with its defaults.
    add(is_image, tl!("Remove Background"), crate::addons::image::REMOVE_BACKGROUND);
    // Object › Path's Offset Path and Simplify take paths: type alone could only fail, and offsets live through the
    // Appearance panel's Effect › Path › Offset Path (#2238).
    let (has_type, type_only) = app.session.active().map_or((false, false), |st| {
        let is_text = |id: &vectorcraft_doc::NodeId| st.doc.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Text(_)));
        let objects = &st.selection.objects;
        (objects.iter().any(is_text), !objects.is_empty() && objects.iter().all(is_text))
    });
    // A type selection's own Quick Actions: Create Outlines, Retype and Find and Replace…
    // (<https://helpx.adobe.com/illustrator/desktop/get-started/learn-the-basics/properties-panel-overview.html>).
    let retype = has_type && (crate::menus::enabled(app, "ui.retype") || crate::menus::enabled(app, "type.retype"));
    add(has_type, tl!("Create Outlines"), "type.createOutlines");
    add(retype, tl!("Retype"), "ui.retype");
    add(has_type, tl!("Find and Replace…"), "ui.findAndReplace");
    add(!type_only, tl!("Offset Path"), "object.path.offsetPath");
    add(!type_only, tl!("Simplify"), "object.path.simplify");
    add(true, tl!("Arrange: Bring to Front"), "object.arrange.bringToFront");
    add(true, tl!("Lock"), "object.lock");
    actions.retain(|a| !matches!(a.command, "object.lock" | "object.isolate") || crate::menus::enabled(app, a.command));
    actions
}

/// Whether Merge Live Paint can take the selection: a Live Paint group among the selected objects
/// and nothing Live Paint refuses (type, images and the rest; `livePaint.merge` refuses them too).
fn can_merge_live_paint(app: &VectorcraftApp) -> bool {
    use vectorcraft_tools::builder::{is_live_paint, live_paint_takes};
    app.session.active().is_some_and(|st| {
        let nodes = || st.selection.objects.iter().filter_map(|id| st.doc.node(*id));
        nodes().any(is_live_paint) && nodes().all(live_paint_takes)
    })
}

/// Whether the selection uses more than one colour (`recolor.colors`), which offers the Recolor
/// quick action; memoised per document revision.
pub(crate) fn multi_color(app: &mut VectorcraftApp) -> bool {
    let Some(st) = app.session.active() else { return false };
    let key = (st.uid, st.revision);
    if let Some((k, multi)) = app.canvas.multi_color
        && k == key
    {
        return multi;
    }
    let r = app.session.execute("recolor.colors", &json!({})).unwrap_or_default();
    let multi = r["colors"].as_array().is_some_and(|c| c.len() > 1);
    app.canvas.multi_color = Some((key, multi));
    multi
}

/// The panel's heading: what the sections are for (the artboards with the Artboard tool, the
/// selected ruler guides, the document with nothing selected), else the selection's kind.
fn heading(first: Option<&vectorcraft_doc::Node>, n_sel: usize, editing_artboards: bool, guides: bool) -> String {
    match (first, n_sel) {
        _ if editing_artboards => tl!("Artboard").to_string(),
        (None, _) if guides => tl!("Guides").to_string(),
        (None, _) => tl!("Document").to_string(),
        (_, n) if n > 1 => crate::i18n::tn(n as u64, "{n} Object", "{n} Objects"),
        (Some(n), _) if crate::panels::image_trace::is_trace(n) => tl!("Image Tracing").to_string(),
        (Some(n), _) => tl!(n.kind_label()).to_string(),
    }
}

mod appearance_section;
mod artboard_sections;
mod document_sections;
mod object_sections;
mod transform_section;
mod type_sections;

pub(crate) use appearance_section::{GlobalEditKey, global_edit_popover};
pub(crate) use artboard_sections::{ArtboardSizeChoice, artboard_presets, artboard_size_controls, artboard_size_row};
pub use transform_section::transform_section;
pub use type_sections::type_sections;

pub(crate) mod dynamic_text;
pub(crate) mod graph;
pub(crate) mod vertical_align;

#[cfg(test)]
mod tests;
