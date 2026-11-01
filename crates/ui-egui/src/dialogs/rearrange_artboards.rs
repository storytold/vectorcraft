//! Object → Artboards → Rearrange All Artboards (also the Artboards panel's button and menu, and
//! the Properties panel's Rearrange All): Layout (a grid by row or by column, one row or one
//! column), Layout Order (left to right or right to left), Columns (Rows for a grid by column),
//! Spacing and Move Artwork with Artboard, with the document's artboard count. OK runs
//! `artboard.rearrange` as one undo step (#681).
//!
//! Fields: `layout` (`gridByRow` | `gridByColumn` | `row` | `column`), `order` (`leftToRight` |
//! `rightToLeft`), `columns` (the rows of a grid by column), `spacing` (pt) and `moveArtwork`.

use serde_json::{Value, json};
use vectorcraft_engine::cmd::newdoc::ArtboardLayout;

use super::{DialogSpec, form, run_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Rearrange All Artboards.
pub const KIND: &str = "rearrangeArtboards";

const CMD: &str = "artboard.rearrange";

/// Width of the label column.
const LABEL_W: f32 = 96.0;

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Rearrange All Artboards").into(), body, confirm, min_width: 320.0, ..DialogSpec::FORM };

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
    json!({
        "layout": d.str("layout"), "order": d.str("order"), "columns": d.f64("columns", 2.0),
        "spacing": d.f64("spacing", 20.0), "moveArtwork": d.bool("moveArtwork"),
    })
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let unit = app.session.general_unit();
    let n = app.session.active().map_or(0, |doc| doc.doc.artboards.len());
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
    widgets::dim_label(ui, &crate::i18n::fmt(tl!("Artboards: {n}"), &[("n", &n.to_string())]));
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let result = run_and_close(app, CMD, params(d))?;
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
}
