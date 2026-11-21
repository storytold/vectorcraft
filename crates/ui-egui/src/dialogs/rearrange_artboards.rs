//! Object → Artboards → Rearrange All Artboards (also the Artboards panel's button and menu, and
//! the Properties panel's Rearrange All): Layout (a grid by row or by column, one row or one
//! column), Layout Order (left to right or right to left), Columns (Rows for a grid by column),
//! Spacing and Move Artwork with Artboard, with the document's artboard count. OK runs
//! `artboard.rearrange` as one undo step (#681).
//!
//! Rearrange (the Artboard tool's canvas menu and Contextual Task Bar, the Artboards panel's
//! Rearrange… and its button while the Artboard tool is in use) is the same dialog for the
//! selected artboards (field `indices`), with their count. Its OK waits for a click on the canvas,
//! which places the grid's top-left there: the grid follows the pointer until then, Enter places it
//! at the pointer (or, off the canvas, at the first selected artboard's top-left), a Space-drag
//! still pans, and Esc, another tool or another document cancels it. The click or Enter runs
//! `artboard.rearrange` with `indices` and `at` as one undo step
//! (https://helpx.adobe.com/illustrator/desktop/create-manage-artboards/organize-manage-artboards/rearrange-artboards.html).
//!
//! Fields: `layout` (`gridByRow` | `gridByColumn` | `row` | `column`), `order` (`leftToRight` |
//! `rightToLeft`), `columns` (the rows of a grid by column), `spacing` (pt), `moveArtwork` and, for
//! Rearrange, `indices`.

use serde_json::{Value, json};
use vectorcraft_engine::cmd::newdoc::{ArtboardLayout, rearrange_grid};
use vectorcraft_geom::Rect;

use super::{DialogSpec, form, run_and_close};
use crate::canvas::Xf;
use crate::state::{Dialog, RearrangePlacement, View};
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Rearrange All Artboards.
pub const KIND: &str = "rearrangeArtboards";

const CMD: &str = "artboard.rearrange";

/// The status bar's message while a Rearrange waits for its click.
pub(crate) const PLACE_HINT: &str = "Click on the canvas to place the artboards (Esc cancels)";

/// Width of the label column.
const LABEL_W: f32 = 96.0;

pub(super) const SPEC: DialogSpec = DialogSpec { heading, body, confirm, min_width: 320.0, ..DialogSpec::FORM };

fn heading(d: &Dialog) -> String {
    if indices(d).is_some() { tl!("Rearrange") } else { tl!("Rearrange All Artboards") }.into()
}

/// The fields a fresh dialog starts with.
pub fn fields() -> Value {
    json!({"layout": "gridByRow", "order": "leftToRight", "columns": 2, "spacing": 20, "moveArtwork": true})
}

/// The fields the menu item opens with, retaining the spacing from its last confirmation.
pub fn open_fields(app: &VectorcraftApp) -> Value {
    let mut fields = fields();
    if let Some(spacing) = app.ui.rearrange_artboards_spacing.as_f64() {
        fields["spacing"] = json!(spacing);
    }
    fields
}

/// Open Rearrange for artboards `indices`, kept sorted and once each; none opens nothing. A
/// placement still waiting is dropped.
pub(crate) fn open_selected(app: &mut VectorcraftApp, indices: &[usize]) {
    cancel_placement(app);
    if indices.is_empty() {
        return;
    }
    let mut indices = indices.to_vec();
    indices.sort_unstable();
    indices.dedup();
    let mut fields = open_fields(app);
    fields["indices"] = json!(indices);
    app.ui.dialog = Some(Dialog::new(KIND, fields));
}

/// The artboards of a menu item's params that list nothing but `indices` (the Artboard tool's
/// Rearrange), as whole numbers.
pub(crate) fn menu_indices(p: &Value) -> Option<Vec<usize>> {
    let o = p.as_object().filter(|o| o.len() == 1)?;
    let list = o.get("indices")?.as_array()?;
    list.iter().map(|i| i.as_u64().and_then(|i| usize::try_from(i).ok())).collect()
}

/// The artboards a Rearrange dialog lays out; `None` for Rearrange All Artboards.
fn indices(d: &Dialog) -> Option<Vec<usize>> {
    d.fields.get("indices").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).filter_map(|i| usize::try_from(i).ok()).collect())
}

/// The artboards a waiting placement lays out.
fn placed(pl: &RearrangePlacement) -> Vec<usize> {
    pl.params["indices"].as_array().into_iter().flatten().filter_map(Value::as_u64).filter_map(|i| usize::try_from(i).ok()).collect()
}

/// Drop the placement waiting for its click, and its message in the status bar.
pub(crate) fn cancel_placement(app: &mut VectorcraftApp) {
    if app.ui.rearrange_placement.take().is_some() && app.ui.status == PLACE_HINT {
        app.status("");
    }
}

/// Whether a placement waits for its click: one whose document is no longer the active one, whose
/// tool is no longer the tool in use, or whose listed places no longer hold the artboards it was
/// confirmed for (removed or moved in the list) is dropped.
pub(crate) fn refresh_placement(app: &mut VectorcraftApp) -> bool {
    let Some(pl) = app.ui.rearrange_placement.as_ref() else { return false };
    let same = app.session.active().filter(|d| d.uid == pl.doc).is_some_and(|d| {
        let listed = placed(pl);
        listed.len() == pl.ids.len() && listed.iter().zip(&pl.ids).all(|(i, id)| d.doc.artboards.get(*i).is_some_and(|a| a.id == *id))
    });
    if same && app.session.tool_id() == pl.tool {
        return true;
    }
    cancel_placement(app);
    false
}

/// The canvas's input while a placement waits and no modal dialog is open (`modal`): Esc cancels
/// it, a press on the canvas runs `artboard.rearrange` with the grid's top-left (`at`) at the
/// pressed point, and Enter, when no other control has the keyboard focus, runs it with `at` at the
/// pointer over the canvas, else with the grid at the first selected artboard's top-left. While
/// Space lends the view tools (`space`), presses pan or zoom as usual. Returns true when the
/// placement takes the frame's input, so no tool gets it.
pub(crate) fn placement_input(app: &mut VectorcraftApp, ui: &egui::Ui, resp: &egui::Response, rect: egui::Rect, modal: bool, space: bool) -> bool {
    if !refresh_placement(app) || modal {
        return false;
    }
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        cancel_placement(app);
        return true;
    }
    let armed = app.ui.rearrange_placement.as_ref().is_some_and(|pl| pl.armed);
    if let Some(pl) = app.ui.rearrange_placement.as_mut() {
        pl.armed = true;
    }
    if space {
        return false;
    }
    let pointer = ui.input(|i| i.pointer.clone());
    // The grid follows the pointer.
    if pointer.is_moving() {
        ui.ctx().request_repaint();
    }
    let xf = Xf::at(rect, app.view().unwrap_or(&View::default()), app.canvas_ui_zoom);
    // A finger press once a pen has been seen navigates the view and places nothing.
    let navigates = crate::touch::finger_navigates_only();
    let pressed = pointer.interact_pos().filter(|p| pointer.primary_pressed() && !navigates && resp.hovered() && rect.contains(*p));
    // Enter belongs to the control that has the keyboard focus, when another one has it.
    let free_keys = ui.memory(|m| m.focused().is_none_or(|id| id == resp.id));
    let entered = armed && free_keys && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let at = match (pressed, entered) {
        (Some(p), _) => Some(xf.to_doc(p)),
        (None, true) => pointer.hover_pos().filter(|p| resp.hovered() && rect.contains(*p)).map(|p| xf.to_doc(p)),
        (None, false) => return false,
    };
    let Some(mut pl) = app.ui.rearrange_placement.take() else { return false };
    let listed = placed(&pl);
    let locked = app.session.active().is_some_and(|d| listed.iter().all(|i| d.doc.artboards.get(*i).is_some_and(|a| a.locked)));
    if let Some(at) = at {
        pl.params["at"] = json!([at.x, at.y]);
    }
    match app.run(CMD, pl.params) {
        // Every selected artboard locked: nothing could move.
        Ok(r) if r["moved"] == 0 && locked => app.status("the artboard is locked"),
        Ok(_) => app.status(""),
        Err(e) => app.status(e),
    }
    true
}

/// The grid a waiting placement makes with its top-left at the pointer: the outline of each
/// artboard that moves, in `color`, 1 point wide on screen.
pub(crate) fn paint_placement(app: &VectorcraftApp, painter: &egui::Painter, xf: &Xf, color: egui::Color32) {
    let Some(pl) = app.ui.rearrange_placement.as_ref().filter(|pl| app.session.tool_id() == pl.tool) else { return };
    let (Some(at), Some(st)) = (app.hover_doc, app.session.active().filter(|d| d.uid == pl.doc)) else { return };
    let boards: Vec<&vectorcraft_doc::Artboard> = placed(pl).into_iter().filter_map(|i| st.doc.artboards.get(i)).collect();
    let sizes: Vec<(f64, f64)> = boards.iter().map(|a| (a.rect.width(), a.rect.height())).collect();
    let p = &pl.params;
    let layout = p["layout"].as_str().and_then(ArtboardLayout::parse).unwrap_or_default();
    let lines = p["columns"].as_f64().filter(|c| c.is_finite()).map_or(2, |c| c.round().max(1.0) as u64);
    let spacing = p["spacing"].as_f64().filter(|s| s.is_finite()).unwrap_or(20.0).clamp(-1.0e5, 1.0e5);
    let (origins, ..) = rearrange_grid(&sizes, at, layout, lines, spacing, p["order"] == "rightToLeft");
    // A locked artboard keeps its cell but stays where it is.
    for ((o, (w, h)), a) in origins.iter().zip(&sizes).zip(&boards) {
        if !a.locked {
            painter.add(egui::Shape::closed_line(xf.quad(Rect::new(o.x, o.y, o.x + w, o.y + h)), egui::Stroke::new(1.0, color)));
        }
    }
}

/// A layout's toggle: its tooltip and its artboards' cells (column, row) in their order, left to
/// right.
fn layout_glyph(l: ArtboardLayout) -> (&'static str, &'static [(u8, u8)]) {
    match l {
        ArtboardLayout::GridByRow => (tl!("Grid by Row"), &[(0, 0), (1, 0), (0, 1), (1, 1)]),
        ArtboardLayout::GridByColumn => (tl!("Grid by Column"), &[(0, 0), (0, 1), (1, 0), (1, 1)]),
        ArtboardLayout::Row => (tl!("Arrange by Row"), &[(0, 0), (1, 0), (2, 0)]),
        ArtboardLayout::Column => (tl!("Arrange by Column"), &[(0, 0), (0, 1), (0, 2)]),
    }
}

/// `cells` mirrored left to right when `rtl` (the order the layout toggles preview).
fn mirrored(cells: &[(u8, u8)], rtl: bool) -> Vec<(u8, u8)> {
    let last = cells.iter().map(|c| c.0).max().unwrap_or(0);
    cells.iter().map(|&(x, y)| (if rtl { last - x } else { x }, y)).collect()
}

/// `artboard.rearrange` parameters from the fields.
fn params(d: &Dialog) -> Value {
    let mut p = json!({
        "layout": d.str("layout"), "order": d.str("order"), "columns": d.f64("columns", 2.0),
        "spacing": d.f64("spacing", 20.0), "moveArtwork": d.bool("moveArtwork"),
    });
    if let Some(indices) = indices(d) {
        p["indices"] = json!(indices);
    }
    p
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let unit = app.session.general_unit();
    let selected = indices(d);
    let n = selected.as_ref().map_or_else(|| app.session.active().map_or(0, |doc| doc.doc.artboards.len()), Vec::len);
    let layout = ArtboardLayout::parse(&d.str("layout")).unwrap_or_default();
    let rtl = d.str("order") == "rightToLeft";
    widgets::label_row(ui, tl!("Layout:"), LABEL_W, |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for l in ArtboardLayout::ALL {
            let (tip, cells) = layout_glyph(l);
            if widgets::layout_button(ui, &mirrored(cells, rtl), layout == l, tip) {
                d.fields.insert("layout".into(), json!(l.id()));
            }
        }
    });
    ui.add_space(4.0);
    widgets::label_row(ui, tl!("Layout Order:"), LABEL_W, |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        // One column reads the same either way.
        ui.add_enabled_ui(layout != ArtboardLayout::Column, |ui| {
            for (value, cells, tip) in [
                ("leftToRight", [(0, 0), (1, 0)], tl!("Change to Left-to-Right Layout")),
                ("rightToLeft", [(1, 0), (0, 0)], tl!("Change to Right-to-Left Layout")),
            ] {
                if widgets::layout_button(ui, &cells, (value == "rightToLeft") == rtl, tip) {
                    d.fields.insert("order".into(), json!(value));
                }
            }
        });
    });
    ui.add_space(4.0);
    // The rows of a grid by column; one row or column has no count to set.
    let label = if layout == ArtboardLayout::GridByColumn { tl!("Rows:") } else { tl!("Columns:") };
    widgets::label_row(ui, label, LABEL_W, |ui| {
        let grid = matches!(layout, ArtboardLayout::GridByRow | ArtboardLayout::GridByColumn);
        ui.add_enabled_ui(grid, |ui| {
            let most = n.max(1) as f64;
            if let Some(v) = widgets::spin_plain(ui, "rearrange-columns", d.f64("columns", 2.0), "", 0, 76.0, 1.0, 1.0, &[]) {
                d.fields.insert("columns".into(), json!(v.round().clamp(1.0, most)));
            }
        });
    });
    ui.add_space(4.0);
    widgets::label_row(ui, tl!("Spacing:"), LABEL_W, |ui| {
        form::length(ui, d, "spacing", unit, 100.0);
    });
    ui.add_space(4.0);
    widgets::label_row(ui, tl!("Options:"), LABEL_W, |ui| {
        form::check(ui, d, "moveArtwork", tl!("Move Artwork with Artboard"));
    });
    ui.add_space(8.0);
    let count = if selected.is_some() { tl!("Selected Artboards: {n}") } else { tl!("Artboards: {n}") };
    widgets::dim_label(ui, &crate::i18n::fmt(count, &[("n", &n.to_string())]));
    false
}

/// Rearrange All Artboards runs at once. Rearrange closes and waits for its click on the canvas
/// ([`placement_input`]).
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let result = match indices(d) {
        None => {
            cancel_placement(app);
            run_and_close(app, CMD, params(d))?
        }
        Some(listed) => {
            let st = app.session.active().ok_or_else(|| "no document".to_string())?;
            let ids: Vec<u32> = listed.iter().filter_map(|i| st.doc.artboards.get(*i)).map(|a| a.id).collect();
            if ids.len() != listed.len() {
                return Err("no such artboard".into());
            }
            let (doc, tool) = (st.uid, app.session.tool_id().to_string());
            app.ui.rearrange_placement = Some(RearrangePlacement { doc, tool, params: params(d), ids, armed: false });
            app.ui.dialog = None;
            app.status(PLACE_HINT);
            Value::Null
        }
    };
    app.ui.rearrange_artboards_spacing = json!(d.f64("spacing", 20.0));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    /// A frame of the open dialog with `events` → the texts painted. The dialog measures itself on
    /// its first frame, so a fresh context needs two.
    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<String> {
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
        out.shapes.iter().filter_map(|s| if let egui::epaint::Shape::Text(t) = &s.shape { Some(t.galley.text().to_string()) } else { None }).collect()
    }

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        ctx
    }

    fn origins(app: &VectorcraftApp) -> Vec<(f64, f64)> {
        app.session.active().unwrap().doc.artboards.iter().map(|a| (a.rect.x0, a.rect.y0)).collect()
    }

    /// #681: the menu item opens the dialog with the artboard count; Columns reads Rows for a grid
    /// by column; OK lays the artboards out right to left, as one undo step.
    #[test]
    fn opens_from_the_menu_and_rearranges_by_layout_and_order() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 50, "artboards": 3})).unwrap();
        // The menu item has no params of its own: it opens this dialog, not the generic one.
        let (id, p) = crate::menus::click_target("Rearrange All Artboards…", CMD, &Value::Null);
        crate::menus::invoke(&mut app, &id, p);
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND));
        let ctx = context();
        frame(&mut app, &ctx, vec![]);
        let texts = frame(&mut app, &ctx, vec![]);
        for want in ["Rearrange All Artboards", "Layout:", "Layout Order:", "Columns:", "Spacing:", "Move Artwork with Artboard", "Artboards: 3"] {
            assert!(texts.iter().any(|t| t == want), "{want:?} in {texts:?}");
        }
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("layout".into(), json!("gridByColumn"));
        let texts = frame(&mut app, &ctx, vec![]);
        assert!(texts.iter().any(|t| t == "Rows:") && !texts.iter().any(|t| t == "Columns:"), "{texts:?}");
        // One row, right to left, 10 pt apart.
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("layout".into(), json!("row"));
        d.fields.insert("order".into(), json!("rightToLeft"));
        d.fields.insert("spacing".into(), json!(10));
        frame(&mut app, &ctx, vec![]);
        let undo = app.session.doc().unwrap().history.undo.len();
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(origins(&app), [(220.0, 0.0), (110.0, 0.0), (0.0, 0.0)]);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
    }

    /// The spacing confirmed last is the one the dialog opens with next time (#4558).
    #[test]
    fn keeps_spacing_when_reopening_after_rearranging() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 50, "artboards": 3})).unwrap();
        app.run("ui.menuDialog", json!({"command": CMD})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("spacing".into(), json!(37));
        crate::dialogs::confirm(&mut app).unwrap();

        app.run("ui.menuDialog", json!({"command": CMD})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().f64("spacing", 20.0), 37.0);
    }

    /// Clicking the layout and order toggles sets the fields (the toggles' rects from the frame).
    #[test]
    fn the_toggles_set_layout_and_order() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 50, "artboards": 4})).unwrap();
        app.run("ui.menuDialog", json!({ "command": CMD })).unwrap();
        let ctx = context();
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![]);
        // The 26-point toggles, left to right then top to bottom: four layouts, then two orders.
        let toggles: Vec<egui::Rect> = ctx.viewport(|vp| {
            let mut r: Vec<egui::Rect> = vp
                .prev_pass
                .widgets
                .layers()
                .flat_map(|(_, w)| w.iter())
                .filter(|w| w.sense.senses_click() && w.rect.width() == 26.0 && w.rect.height() == 26.0)
                .map(|w| w.rect)
                .collect();
            r.sort_by(|a, b| a.top().total_cmp(&b.top()).then(a.left().total_cmp(&b.left())));
            r
        });
        assert_eq!(toggles.len(), 6, "{toggles:?}");
        for (i, key, want) in [(3, "layout", "column"), (1, "layout", "gridByColumn"), (5, "order", "rightToLeft")] {
            let at = toggles[i].center();
            let b = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), b(true)]);
            frame(&mut app, &ctx, vec![b(false)]);
            assert_eq!(app.ui.dialog.as_ref().unwrap().str(key), want);
            // A frame for the order toggles to follow the layout (one column greys them).
            frame(&mut app, &ctx, vec![]);
        }
        crate::dialogs::confirm(&mut app).unwrap();
        // Two rows (Columns 2 reads Rows for a grid by column), the first column at the right.
        assert_eq!(origins(&app), [(120.0, 0.0), (120.0, 70.0), (0.0, 0.0), (0.0, 70.0)]);
    }

    /// The Artboard tool's canvas menu opens Rearrange for the artboards the tool has selected,
    /// with their count; OK moves nothing and waits for a click on the canvas with those artboards
    /// and the dialog's fields. Rearrange All Artboards still lays out every artboard at once.
    #[test]
    fn rearrange_from_the_canvas_menu_waits_for_a_click_with_the_selected_artboards() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 50, "artboards": 3})).unwrap();
        app.select_tool("artboard");
        app.session.set_tool_option("selected", &json!([0, 2]));
        let items = crate::menus::context_items(&app);
        let target = items.iter().find_map(|i| match i {
            crate::menus::Item::Cmd("Rearrange", id, p) => Some(crate::menus::click_target("Rearrange", id, p)),
            _ => None,
        });
        let (id, p) = target.expect("Rearrange in the canvas menu");
        crate::menus::invoke(&mut app, &id, p);
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND));
        let ctx = context();
        frame(&mut app, &ctx, vec![]);
        let texts = frame(&mut app, &ctx, vec![]);
        for want in ["Rearrange", "Layout:", "Move Artwork with Artboard", "Selected Artboards: 2"] {
            assert!(texts.iter().any(|t| t == want), "{want:?} in {texts:?}");
        }
        assert!(!texts.iter().any(|t| t == "Rearrange All Artboards" || t == "Artboards: 3"), "{texts:?}");
        app.ui.dialog.as_mut().unwrap().fields.insert("layout".into(), json!("row"));
        let (before, undo) = (origins(&app), app.session.doc().unwrap().history.undo.len());
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!((origins(&app), app.session.doc().unwrap().history.undo.len()), (before.clone(), undo));
        let waiting = app.ui.rearrange_placement.as_ref().expect("waits for a click");
        assert_eq!((waiting.params["indices"].clone(), waiting.params["layout"].clone()), (json!([0, 2]), json!("row")));
        assert_eq!(app.ui.status, PLACE_HINT);

        // Rearrange All Artboards drops it and runs at once.
        app.run("ui.menuDialog", json!({"command": CMD})).unwrap();
        crate::dialogs::confirm(&mut app).unwrap();
        assert_ne!(origins(&app), before);
        assert!(app.ui.rearrange_placement.is_none());
        assert_ne!(app.ui.status, PLACE_HINT);
    }
}
