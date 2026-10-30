use super::run::normalize_panel;
use super::slots::{perspective_grid, recent_slot, saved_selection_name, saved_selection_slot, saved_view, saved_view_slot};
use super::{Brightness, Value, VectorcraftApp};

/// Checked state for toggle items.
pub fn checked(app: &VectorcraftApp, id: &str, p: &Value) -> Option<bool> {
    if id == "app.language" {
        let lang = p.get("lang").and_then(Value::as_str).unwrap_or("");
        let pref = app.session.prefs.interface_language.as_str();
        return Some(if lang.eq_ignore_ascii_case("auto") {
            crate::i18n::Lang::from_code(pref).is_none()
        } else {
            crate::i18n::Lang::from_code(pref).is_some_and(|l| l.code().eq_ignore_ascii_case(lang))
        });
    }
    let v = &app.ui.view;
    // An item that can be on or off is `Some` whatever the document and selection (off with none):
    // the macOS menu bar can't change an item's kind without rebuilding.
    // The first type object (of type on a path, with `onpath`) among the selected objects and the
    // objects inside them, in the order the type commands act on them.
    let selected_text = |onpath: bool| {
        let st = app.session.active()?;
        let mut found = None;
        for n in st.selection.objects.iter().filter_map(|id| st.doc.node(*id)) {
            n.walk(&mut |c| {
                if found.is_none()
                    && let vectorcraft_engine::doc::NodeKind::Text(t) = &c.kind
                    && (!onpath || matches!(t.kind, vectorcraft_engine::doc::TextKind::OnPath { .. }))
                {
                    found = Some(t);
                }
            });
            if found.is_some() {
                break;
            }
        }
        found
    };
    Some(match id {
        "type.orientation.vertical" | "type.orientation.horizontal" => {
            selected_text(false).is_some_and(|t| t.vertical == (id == "type.orientation.vertical"))
        }
        "type.opticalMarginAlignment" => crate::panels::character::shared_text_flag(app, |t| t.optical_margin) == Some(true),
        "view.outline" => v.outline,
        "view.pixelPreview" => v.pixel_preview,
        "view.trimView" => v.trim_view,
        "view.snapToPixel" => v.snap_to_pixel,
        "view.snapToGlyph" => v.snap_to_glyph,
        // Type on a Path effect of the selected path type.
        "type.pathOptions" if p.get("effect").is_some() => {
            selected_text(true).is_some_and(|t| p.get("effect").and_then(Value::as_str) == Some(t.path_effect.id()))
        }
        "view.smartGuides" => v.smart_guides,
        "view.grid" => v.grid,
        "view.snapToGrid" => v.snap_to_grid,
        "view.snapToPoint" => v.snap_to_point,
        "view.rulers" => v.rulers,
        // A View › Rotate View item is checked when the active view is turned to its angle; without
        // a document no item is checked.
        "view.rotate" => app.view().zip(p.get("angle").and_then(Value::as_f64)).is_some_and(|(view, a)| (view.rotation - a).abs() < 1e-9),
        "window.control" => app.ui.control_bar,
        "window.toolbar" => app.ui.toolbar,
        "window.toolbarAdvanced" => app.ui.toolbar_advanced,
        "window.toolbarColumns" => app.ui.toolbar_double,
        "window.editToolbar" => app.ui.tools_drawer,
        "window.floatTools" => {
            let tool = p.get("tool").and_then(Value::as_str).unwrap_or("");
            app.ui.floating_flyouts.iter().any(|f| f.tools.iter().any(|id| id == tool))
        }
        "window.taskBar" => app.ui.task_bar,
        "window.taskBar.pin" => app.ui.task_bar_place.pinned,
        "window.panel" => {
            let panel = p.get("panel").and_then(Value::as_str).unwrap_or("");
            normalize_panel(panel).is_some_and(|id| crate::panel_docking::showing(&app.ui, id))
        }
        "window.collapseDock" => crate::panel_docking::dock_collapsed(&app.ui),
        "window.workspace" => p.get("name").and_then(Value::as_str) == Some(app.ui.workspace.as_str()),
        "window.brightness" => p.get("brightness").and_then(Value::as_str).and_then(Brightness::parse) == Some(app.ui.brightness),
        "view.slices.lock" => app.session.slices_locked(),
        "object.slice.clipToArtboard" => app.session.active().is_some_and(|d| d.doc.slices_clip_to_artboard),
        "view.proofColors" => vectorcraft_render::proof::view().proof_colors,
        "view.overprintPreview" => vectorcraft_render::proof::view().overprint,
        "view.proofSetup" => p.get("target").and_then(Value::as_str) == Some(vectorcraft_render::proof::view().setup.target.id().as_str()),
        "perspective.grid.snap" => perspective_grid(app).is_some_and(|g| g.snap),
        "perspective.grid.lockStation" => perspective_grid(app).is_some_and(|g| g.lock_station),
        "file.documentColorMode" | "object.convertDocumentColorMode" => {
            let cmyk = app.session.active().is_some_and(|d| d.doc.color_mode == vectorcraft_engine::doc::ColorMode::Cmyk);
            p.get("mode").and_then(Value::as_str) == Some(if cmyk { "cmyk" } else { "rgb" })
        }
        // The ruler context menu lists the document units, the current one checked.
        "document.setUnits" => {
            p.get("units").and_then(Value::as_str).and_then(vectorcraft_engine::doc::Unit::named) == Some(app.session.general_unit())
        }
        _ => return None,
    })
}

/// Menu slots that are left out while they have nothing to show (unused saved views, recent files,
/// User Defined swatch and graphic style libraries).
pub(super) fn hidden_when_disabled(id: &str) -> bool {
    id.starts_with("view.goto")
        || saved_selection_slot(id).is_some()
        || id.starts_with("file.openRecent")
        || id.starts_with(crate::panels::swatches::USER_SLOT)
        || id.starts_with(crate::panels::graphic_styles::USER_SLOT)
        || id.starts_with(crate::panels::symbols::USER_SLOT)
        || id.starts_with(crate::panels::brushes::USER_SLOT)
        || id.starts_with(crate::dialogs::perspective_presets::SLOT)
}

/// Menu items another item stands in for right now: Envelope Distort's Reset with Warp and Reset
/// with Mesh take the place of Make with Warp and Make with Mesh while an envelope is selected.
fn swapped_out(app: &VectorcraftApp, id: &str) -> bool {
    let reset = match id {
        "object.envelope.makeWithWarp" | "object.envelope.makeWithMesh" => false,
        "object.envelope.resetWithWarp" | "object.envelope.resetWithMesh" => true,
        _ => return false,
    };
    enabled(app, "object.envelope.release") != reset
}

/// How the menus show item `id` (params `p`) now: (enabled, checked: `Some` for an item that is on
/// or off), or `None` while they leave it out: an empty slot ([`hidden_when_disabled`]) or an item
/// another stands in for ([`swapped_out`]).
pub fn shown_state(app: &VectorcraftApp, id: &str, p: &Value) -> Option<(bool, Option<bool>)> {
    let en = enabled(app, id);
    if (!en && hidden_when_disabled(id)) || swapped_out(app, id) {
        return None;
    }
    Some((en, checked(app, id, p)))
}

/// Changes whenever a plug-in is installed or removed: Object › Plug-ins and Effect › Plug-ins
/// list them (the command palette's cache follows it).
pub fn plugin_revision() -> u64 {
    vectorcraft_plugins::registry::revision()
}

/// Effective shortcut of a command: the user's override (Edit → Keyboard Shortcuts) or the default.
pub fn shortcut_of(id: &str) -> Option<&'static str> {
    crate::shortcut_editor::command_shortcut(id)
}

/// Is a command currently enabled?
/// Commands that would act on the text being typed, held back while an IME composes in the Type
/// tool (its marked text isn't committed yet): Undo/Redo — the native menu takes ⌘Z ahead of the
/// IME —, the clipboard and the selection.
/// Clear History also waits: it ends the typing session as Undo does.
pub(crate) fn waits_for_ime(app: &VectorcraftApp, id: &str) -> bool {
    app.session.tool_composing()
        && (matches!(
            id,
            "edit.undo" | "edit.redo" | "edit.cut" | "edit.copy" | "edit.clear" | "edit.duplicate" | "select.all" | "select.none" | "select.inverse"
        ) || id == "edit.clearHistory"
            || id.starts_with("edit.paste"))
}

/// Engine commands that take `ids` and otherwise act on the Appearance panel's subject (the
/// targeted layer, group or object, else the selection). Without `ids` (or with an empty list) and
/// without a subject they are off, as the panel's buttons are: in menus, in the command palette and
/// on their keys.
pub(super) const SUBJECT_COMMANDS: [&str; 2] = ["appearance.addFill", "appearance.addStroke"];

/// Is there an Appearance panel subject: a targeted layer, group or object, or a selected one?
pub(super) fn has_subject(app: &VectorcraftApp) -> bool {
    app.session.active().is_some_and(|st| !st.selection.subjects().is_empty())
}

/// Can command `id` run now? An engine command asks the engine, a UI command the app's state;
/// none that would act on text an input method is still composing.
pub fn enabled(app: &VectorcraftApp, id: &str) -> bool {
    if waits_for_ime(app, id) {
        return false;
    }
    if SUBJECT_COMMANDS.contains(&id) {
        return has_subject(app);
    }
    if let Some(c) = vectorcraft_engine::find_command(id) {
        // The system clipboard's contents can be pasted with an empty internal clipboard.
        return (c.enabled)(&app.session).is_ok() || app.system_paste && id.starts_with("edit.paste");
    }
    match id {
        // Save is off for a clean document that already has its own file.
        "file.save" => app.session.active().is_some_and(|d| d.path.is_none() || d.converted || d.is_dirty()),
        "type.bold" | "type.italic" | "type.underline" | "type.lineComposer" | "type.hyphenate" => {
            crate::panels::character::text_style(app).is_some()
        }
        id if crate::panels::paragraph::ALIGN_COMMANDS.iter().any(|c| c.0 == id) => crate::panels::character::text_style(app).is_some(),
        "type.applyBullets" | "type.applyNumbers" | "type.convertListToText" => crate::panels::character::text_style(app).is_some(),
        id if let Some(b) = crate::panels::align::button(id) => enabled(app, b.command()),
        "file.reveal" => app.services.reveal.is_some() && app.session.active().is_some_and(|d| d.path.is_some()),
        "file.cancelExport" | "file.exportInBackground" => app.background.jobs.iter().any(|j| j.progress.is_some() && !j.cancelled()),
        "plugin.cancelRuns" => crate::plugin_runs::any_running(app),
        "plugin.hideRuns" => crate::plugin_runs::any_shown(app),
        "imageTrace.cancelRuns" => crate::trace_runs::any_running(app),
        "imageTrace.hideRuns" => crate::trace_runs::any_shown(app),
        "file.place"
        | "file.export.svg"
        | "file.export.png"
        | "file.exportAs"
        | "file.exportForScreens"
        | "file.documentSetup"
        | "view.zoomIn"
        | "view.zoomOut"
        | "view.fitArtboard"
        | "view.fitAll"
        | "view.actualSize"
        | "view.rotate" => app.session.active().is_some(),
        "view.rotateToSelection" => app.session.active().is_some_and(|d| !d.selection.objects.is_empty()),
        id if id.starts_with("file.openRecent") => recent_slot(app, id).is_some(),
        "file.clearRecent" => !app.ui.recent_files.is_empty(),
        id if id.starts_with("type.recentFont") => {
            id["type.recentFont".len()..].parse::<usize>().is_ok_and(|n| n >= 1 && n <= app.recent_fonts().len()) && app.session.active().is_some()
        }
        id if saved_selection_slot(id).is_some() => saved_selection_name(app, id).is_some(),
        id if saved_view_slot(id).is_some() => saved_view(app, id).is_some(),
        "effect.dialog" | "ui.recolorDialog" | "ui.effectGalleryDialog" => app.session.active().is_some_and(|d| !d.selection.is_empty()),
        "ui.cropImage" => selected_image(app, |_| true),
        "effect.applyLast" | "effect.last" => app.last_effect.is_some() && app.session.active().is_some_and(|d| !d.selection.is_empty()),
        "file.export.pdf" | "ui.savePdfDialog" | "ui.fileInfoDialog" | "ui.rasterEffectsSettingsDialog" => app.session.active().is_some(),
        "ui.missingFontsDialog" => !cfg!(target_arch = "wasm32") && app.session.active().is_some(),
        "ui.findFontsInFolder" => crate::picks::can(app, &crate::picks::PickRequest::Folder) && app.session.active().is_some(),
        "ui.swatchOptions" | "ui.newSwatch" | "ui.newColorGroup" => app.session.active().is_some(),
        "ui.graphicStyleOptions" | "ui.newCharStyle" | "ui.newParaStyle" => app.session.active().is_some(),
        "ui.newArtboard" => app.session.active().is_some(),
        "ui.sampleScreenColor" => crate::screen_color::available(app),
        "ui.newBrush" => app.session.active().is_some(),
        "ui.brushOptions" => crate::dialogs::brush_options::available(app),
        "ui.hyphenation" | "ui.justification" => crate::panels::character::text_style(app).is_some(),
        "ui.colorBalanceDialog" | "ui.saturateDialog" => app.session.active().is_some_and(|d| !d.selection.is_empty()),
        "ui.saveSwatchLibrary" => app.session.active().is_some(),
        id if id.starts_with(crate::panels::swatches::USER_SLOT) => crate::panels::swatches::user_library(app, id).is_some(),
        "ui.flattenTransparencyDialog" => app.session.active().is_some_and(|d| !d.selection.is_empty()),
        "ui.saveGraphicStyleLibrary" => app.session.active().is_some(),
        id if id.starts_with(crate::panels::graphic_styles::USER_SLOT) => crate::panels::graphic_styles::user_library(app, id).is_some(),
        "ui.saveSymbolLibrary" => app.session.active().is_some_and(|d| !d.doc.symbols.is_empty()),
        id if id.starts_with(crate::panels::symbols::USER_SLOT) => crate::panels::symbols::user_library(app, id).is_some(),
        "ui.saveBrushLibrary" => crate::panels::brushes::has_brushes(app),
        id if id.starts_with(crate::panels::brushes::USER_SLOT) => crate::panels::brushes::user_library(app, id).is_some(),
        // Dimmed for appearance attributes, which Expand Appearance expands first (#3089).
        "ui.expandDialog" => {
            app.session.active().is_some_and(|d| !d.selection.is_empty()) && !vectorcraft_engine::cmd::expand::has_appearance(&app.session)
        }
        "ui.spotColors" => app.session.active().is_some(),
        "ui.menuDialog" => app.session.active().is_some_and(|d| !d.selection.is_empty()),
        "ui.simplifyAuto" | "ui.simplifyMoreOptions" => crate::dialogs::simplify_slider::showing(app),
        "ui.smoothAuto" => crate::dialogs::smooth_slider::showing(app),
        "ui.dialogReset" => crate::dialogs::can_reset(app),
        "ui.dxfOptionsDialog" => app.session.active().is_some(),
        "links.editOriginal" | "links.reveal" => selected_image(app, |im| im.link.is_some()) || selected_placed(app),
        "ui.placementOptionsDialog" => selected_image(app, |_| true) || selected_placed(app),
        "ui.findLinksInFolder" => crate::picks::can(app, &crate::picks::PickRequest::Folder) && app.session.active().is_some(),
        "ui.packageDialog" | "docInfo.save" => app.session.active().is_some(),
        "ui.batchDialog" => !app.ui.action_sets.is_empty() && app.ui.action_sets.iter().any(|s| !s.actions.is_empty()),
        "ui.epsOptionsDialog" => app.session.active().is_some(),
        "file.saveForWeb" | "file.saveForWeb.browser" => app.session.active().is_some(),
        "file.exportSelection" => app.session.active().is_some_and(|d| !d.selection.is_empty()),
        "css.copy" | "css.exportFile" => app.session.active().is_some(),
        "print.printerSetup" => app.services.print.as_ref().is_some_and(|s| s.has_setup()),
        "plugin.dialog" => app.session.active().is_some(),
        "ui.perspectiveGridDialog" | "ui.savePerspectivePreset" => app.session.active().is_some(),
        id if id.starts_with(crate::dialogs::perspective_presets::SLOT) => {
            app.session.active().is_some() && crate::dialogs::perspective_presets::slot_preset(app, id).is_some()
        }
        "ui.perspectivePlane" => app.session.active().is_some(),
        // Only one window per document so far: greyed out, as Window › Arrange's items are (#3724).
        "window.newWindow" => false,
        "window.nextDocument" | "window.previousDocument" => !app.session.documents().is_empty(),
        _ => true,
    }
}

/// Is a placed document selected?
fn selected_placed(app: &VectorcraftApp) -> bool {
    app.session.active().is_some_and(|st| {
        st.selection.objects.iter().any(|id| st.doc.node(*id).is_some_and(|n| matches!(n.kind, vectorcraft_doc::NodeKind::PlacedDocument(_))))
    })
}

/// Is an image `keep` accepts selected?
pub(super) fn selected_image(app: &VectorcraftApp, keep: impl Fn(&vectorcraft_doc::ImageObject) -> bool) -> bool {
    app.session.active().is_some_and(|st| {
        st.selection.objects.iter().any(|id| st.doc.node(*id).is_some_and(|n| matches!(&n.kind, vectorcraft_doc::NodeKind::Image(im) if keep(im))))
    })
}
