//! The Properties panel's Transform section (X, Y, W, H, Rotate, the flips, More Options and a live
//! shape's rows), the Corner rows of Direct-Selected corners and the Transform section of ruler
//! guides selected on their own.

use craft::behavior::{IconId, Mixed};
use craft::draw::ui::TransformGroups;
use craft::ui::buttons::IconButton;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _, clicked};
use craft::ui::display::{Shade, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::pickers::Select;
use craft::ui::popover::Popover;
use craft::ui::text::NumberField;
use egui::Ui;
use serde_json::json;
use vectorcraft_doc::{LiveCorners, LiveShape, NodeKind};

use crate::VectorcraftApp;
use crate::edits::{EditPath, apply_edits};
use crate::panels::{self, first_selected, transform};

/// The Transform section for the selection: its position and size about the reference point, the
/// rotation, the two flips and the More Options popover, then the rows of a selected live shape.
/// "No Selection" in place of the section when nothing is selected.
pub fn transform_section(app: &mut VectorcraftApp, ui: &mut Ui) {
    if app.session.active().is_none() {
        return;
    }
    if app.selection_box().is_none() {
        Text::new(tl!("No Selection")).shade(Shade::Muted).show(ui);
        return;
    }
    super::section(tl!("Transform")).show(ui, |ui, _| {
        let mut groups = TransformGroups::NONE;
        groups.reference = true;
        groups.position = true;
        groups.size = true;
        transform::object_fields(app, ui, groups);
        Row::new().show(ui, |ui, _| {
            transform::rotate_field(app, ui, &[]);
            // The icons are named for their mirror line: `flip-vertical-2` draws a vertical line, the
            // left-right flip, which the Properties panel calls Flip Along Horizontal Axis (#1910).
            IconButton::command(IconId::new("flip-vertical-2"), "ui.transform.flipHorizontal").label(tl!("Flip Along Horizontal Axis")).show(ui);
            IconButton::command(IconId::new("flip-horizontal-2"), "ui.transform.flipVertical").label(tl!("Flip Along Vertical Axis")).show(ui);
            more_options(app, ui);
        });
        if let Some(n) = first_selected(app)
            && let NodeKind::Path { live: Some(live), .. } = &n.kind
        {
            transform::live_shape_rows(app, ui, &n, live, false);
        }
    });
}

/// The More Options icon and its popover: the Transform panel's Scale Corners and Scale Strokes &
/// Effects, which stay open while they are toggled.
fn more_options(app: &mut VectorcraftApp, ui: &mut Ui) {
    // A copy, so the popover's binding does not borrow `app` while its body runs commands.
    let mut open = app.ui.properties_panel.more_options_open;
    let more = IconButton::new(IconId::new("ellipsis")).label(tl!("More Options")).expanded(open).show(ui);
    if clicked(&more.response) {
        open = !open;
    }
    let prefs = &app.session.prefs;
    let rows = [(tl!("Scale Corners"), "scaleCorners", prefs.scale_corners), (tl!("Scale Strokes & Effects"), "scaleStrokes", prefs.scale_strokes)];
    Popover::new(tl!("More Options"), Bind::bound(&mut open)).opened_by(&more.response).show(ui, |ui, _| {
        for (label, key, on) in rows {
            if let Some(on) = Checkbox::new(Bind::given(on)).label(label).show(ui).events.committed().copied() {
                transform::set_pref(app, key, on);
            }
        }
    });
    app.ui.properties_panel.more_options_open = open;
}

/// The Corners of the paths whose corners Direct Selection has selected: their kind, and their
/// radius with the Corners link beside it, which `object.setLiveShape` sets on those corners
/// ([`panels::picked_corners`]); the link opens the Corners pop-up for them. The Properties panel
/// overview shows the control while the Direct Selection tool is chosen and a corner of an object
/// is selected
/// (<https://helpx.adobe.com/illustrator/desktop/get-started/learn-the-basics/properties-panel-overview.html>),
/// and the Corners link opens the corner settings
/// (<https://web.archive.org/web/20200308215011/https://helpx.adobe.com/illustrator/using/reshape-with-live-corners.html>).
/// One live rectangle or live polygon selected alone has its own kind and radius rows, which
/// already set the selected corners: it shows the Corners link alone, also while it is selected
/// whole.
pub(super) fn corner_rows(app: &mut VectorcraftApp, ui: &mut Ui) {
    if app.session.tool_id() != "directSelection" {
        return;
    }
    let Some(st) = app.session.active() else { return };
    let own_rows = matches!(st.selection.objects[..], [id]
        if st.doc.node(id).is_some_and(|n| matches!(n.live_shape(), Some(LiveShape::Rectangle { .. } | LiveShape::Polygon { .. }))));
    // A corner of a path is Direct-Selected.
    let corner_selected = st.selection.objects.iter().any(|id| {
        st.doc.node(*id).and_then(LiveCorners::of).is_some_and(|lc| st.selection.partial(*id).is_some_and(|a| !lc.corners_of(a).is_empty()))
    });
    if !own_rows && !corner_selected {
        return;
    }
    let Some(picked) = panels::picked_corners(app) else { return };
    if own_rows {
        Row::new().show(ui, |ui, _| panels::corners_link(app, ui));
        return;
    }
    // The corners differ in kind: no entry is the value.
    let kind = picked.kind.map_or(Mixed::Mixed, Mixed::Same);
    let entries = panels::corner_entries();
    let chosen = Field::new(tl!("Corner")).show(ui, |ui, _| Select::new(tl!("Corner"), Bind::given(kind), &entries).show(ui).value().copied()).inner;
    if let Some(Some(kind)) = chosen {
        panels::set_picked_corners(app, &picked, json!({"kind": kind}));
    }
    let lengths = crate::canvas::general_lengths(app);
    let mut edit = None;
    Field::new(tl!("Corner Radius")).show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            edit = Some(NumberField::length(tl!("Corner Radius"), Bind::given(picked.radius), lengths).show(ui).edit);
            panels::corners_link(app, ui);
        });
    });
    if let Some(edit) = edit {
        // Every value of a scrub sets the corners; the gesture is one undo step.
        apply_edits(app, &edit, EditPath::Scope, |app, v: &Option<f64>, _| {
            // A negative radius typed in rounds nothing: the command takes 0 or more.
            if let Some(r) = v {
                panels::set_picked_corners(app, &picked, json!({"radius": r.max(0.0)}));
            }
        });
    }
}

/// Ruler guides selected on their own: a Transform section with the X of the vertical ones and
/// the Y of the horizontal ones, which move them, as the Transform panel shows them (#1597).
pub(super) fn guide_sections(app: &mut VectorcraftApp, ui: &mut Ui, guides: (Option<f64>, Option<f64>)) {
    super::section(tl!("Transform")).show(ui, |ui, _| transform::guide_fields(app, ui, guides));
}
