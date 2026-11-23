//! Object → Artboards → Rearrange All Artboards (also the Artboards panel's button and menu, and
//! the Properties panel's Rearrange All): Layout (a grid by row or by column, one row or one
//! column), Layout Order (left to right or right to left), Columns (Rows for a grid by column),
//! Spacing and Move Artwork with Artboard, with the document's artboard count. OK runs
//! `artboard.rearrange` as one undo step (#681).
//!
//! Fields: `layout` (`gridByRow` | `gridByColumn` | `row` | `column`), `order` (`leftToRight` |
//! `rightToLeft`), `columns` (the rows of a grid by column), `spacing` (pt) and `moveArtwork`.

use craft::ui::choice::Segmented;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, StackDialog, dialog_id};
use craft::ui::display::Text;
use craft::ui::form::Field;
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::env::{LengthUnits, UnitKind};
use craft::ui::style::TypeRole;
use craft::ui::text::NumberField;
use serde_json::{Value, json};
use vectorcraft_doc::Unit;
use vectorcraft_engine::cmd::newdoc::ArtboardLayout;

use super::layer_options::check;
use crate::VectorcraftApp;
use crate::page_glyphs::LayoutGlyph;

/// The command Rearrange All Artboards runs; the key of its dialog.
pub(crate) const CMD: &str = "artboard.rearrange";

static GRID_BY_ROW: LayoutGlyph = LayoutGlyph { tiles: &[(0, 0), (1, 0), (0, 1), (1, 1)] };
static GRID_BY_ROW_RTL: LayoutGlyph = LayoutGlyph { tiles: &[(1, 0), (0, 0), (1, 1), (0, 1)] };
static GRID_BY_COLUMN: LayoutGlyph = LayoutGlyph { tiles: &[(0, 0), (0, 1), (1, 0), (1, 1)] };
static GRID_BY_COLUMN_RTL: LayoutGlyph = LayoutGlyph { tiles: &[(1, 0), (1, 1), (0, 0), (0, 1)] };
static ROW: LayoutGlyph = LayoutGlyph { tiles: &[(0, 0), (1, 0), (2, 0)] };
static ROW_RTL: LayoutGlyph = LayoutGlyph { tiles: &[(2, 0), (1, 0), (0, 0)] };
/// One column reads the same either way.
static COLUMN: LayoutGlyph = LayoutGlyph { tiles: &[(0, 0), (0, 1), (0, 2)] };
static LEFT_TO_RIGHT: LayoutGlyph = LayoutGlyph { tiles: &[(0, 0), (1, 0)] };
static RIGHT_TO_LEFT: LayoutGlyph = LayoutGlyph { tiles: &[(1, 0), (0, 0)] };

/// A layout's toggle: its tooltip and its picture in the order the toggles preview (`rtl`: right to left).
fn layout_glyph(l: ArtboardLayout, rtl: bool) -> (&'static str, &'static LayoutGlyph) {
    match (l, rtl) {
        (ArtboardLayout::GridByRow, false) => (tl!("Grid by Row"), &GRID_BY_ROW),
        (ArtboardLayout::GridByRow, true) => (tl!("Grid by Row"), &GRID_BY_ROW_RTL),
        (ArtboardLayout::GridByColumn, false) => (tl!("Grid by Column"), &GRID_BY_COLUMN),
        (ArtboardLayout::GridByColumn, true) => (tl!("Grid by Column"), &GRID_BY_COLUMN_RTL),
        (ArtboardLayout::Row, false) => (tl!("Arrange by Row"), &ROW),
        (ArtboardLayout::Row, true) => (tl!("Arrange by Row"), &ROW_RTL),
        (ArtboardLayout::Column, _) => (tl!("Arrange by Column"), &COLUMN),
    }
}

/// The settings the dialog edits.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RearrangeValues {
    layout: ArtboardLayout,
    /// Layout Order is right to left.
    rtl: bool,
    columns: f64,
    spacing: f64,
    move_artwork: bool,
}

impl Default for RearrangeValues {
    fn default() -> Self {
        RearrangeValues { layout: ArtboardLayout::GridByRow, rtl: false, columns: 2.0, spacing: 20.0, move_artwork: true }
    }
}

impl RearrangeValues {
    /// `artboard.rearrange` parameters.
    fn params(&self) -> Value {
        json!({
            "layout": self.layout.id(), "order": self.order(), "columns": self.columns,
            "spacing": self.spacing, "moveArtwork": self.move_artwork,
        })
    }

    fn order(&self) -> &'static str {
        if self.rtl { "rightToLeft" } else { "leftToRight" }
    }

    /// A grid has a count to set; one row or column has none.
    fn is_grid(&self) -> bool {
        matches!(self.layout, ArtboardLayout::GridByRow | ArtboardLayout::GridByColumn)
    }

    /// One column reads the same either way, so its order is not a choice.
    fn has_order(&self) -> bool {
        self.layout != ArtboardLayout::Column
    }
}

/// The length unit a document unit shows as.
fn length_kind(unit: Unit) -> UnitKind {
    match unit {
        Unit::Picas => UnitKind::Picas,
        Unit::Inches => UnitKind::Inches,
        Unit::Millimeters => UnitKind::Millimetres,
        Unit::Centimeters => UnitKind::Centimetres,
        Unit::Pixels => UnitKind::Pixels,
        Unit::Meters => UnitKind::Metres,
        Unit::Yards => UnitKind::Yards,
        Unit::Feet | Unit::FeetInches => UnitKind::Feet,
        _ => UnitKind::Points,
    }
}

/// `ui.menuDialog` for Rearrange All Artboards: opens the dialog on the spacing of its last OK.
pub(crate) fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let mut values = RearrangeValues::default();
    if let Some(spacing) = app.ui.rearrange_artboards_spacing.as_f64() {
        values.spacing = spacing;
    }
    let artboards = app.session.active().map_or(0, |doc| doc.doc.artboards.len());
    let lengths = LengthUnits::new(UnitKind::Points, length_kind(app.session.general_unit()), 72.0);
    let lengths = crate::fields::bare_points(lengths, app.session.prefs.numbers_without_units_are_points);
    let dialogs = app.dialogs().ok_or("no window")?;
    dialogs.with(|host| host.open(RearrangeArtboardsDialog { id: dialog_id(CMD), open: true, values, artboards, lengths }));
    Ok(Value::Null)
}

/// The Rearrange All Artboards dialog; its answer is the settings OK confirmed, which [`pump`] runs.
pub(crate) struct RearrangeArtboardsDialog {
    id: DialogId,
    open: bool,
    values: RearrangeValues,
    /// The document's artboard count, which bounds Columns.
    artboards: usize,
    /// The lengths Spacing keeps and shows.
    lengths: LengthUnits,
}

/// The rows of the dialog for a document of `artboards` artboards, Spacing shown in `unit`.
fn body(ui: &mut egui::Ui, v: &mut RearrangeValues, artboards: usize, lengths: LengthUnits) {
    Field::new(tl!("Layout:")).id_salt("layout").show(ui, |ui, _| {
        let entries: Vec<Entry<ArtboardLayout>> = ArtboardLayout::ALL
            .into_iter()
            .map(|l| {
                let (tip, glyph) = layout_glyph(l, v.rtl);
                Entry::new(l, tip).icon(IconSpec::glyph(glyph))
            })
            .collect();
        if let Some(l) = Segmented::new(tl!("Layout:"), Bind::given(v.layout), &entries).show(ui).value().copied() {
            v.layout = l;
        }
    });
    Field::new(tl!("Layout Order:")).id_salt("order").enabled(v.has_order()).show(ui, |ui, _| {
        let entries = [
            Entry::new(false, tl!("Change to Left-to-Right Layout")).icon(IconSpec::glyph(&LEFT_TO_RIGHT)),
            Entry::new(true, tl!("Change to Right-to-Left Layout")).icon(IconSpec::glyph(&RIGHT_TO_LEFT)),
        ];
        if let Some(rtl) = Segmented::new(tl!("Layout Order:"), Bind::given(v.rtl), &entries).show(ui).value().copied() {
            v.rtl = rtl;
        }
    });
    // The rows of a grid by column.
    let count = if v.layout == ArtboardLayout::GridByColumn { tl!("Rows:") } else { tl!("Columns:") };
    Field::new(count).id_salt("columns").enabled(v.is_grid()).show(ui, |ui, _| {
        let most = artboards.max(1) as f64;
        let field = NumberField::new(count, Bind::given(Some(v.columns))).range(1.0..=most).step(1.0).show(ui);
        if let Some(Some(columns)) = field.edit.value().copied() {
            v.columns = columns.round().clamp(1.0, most);
        }
    });
    Field::new(tl!("Spacing:")).id_salt("spacing").show(ui, |ui, _| {
        let field = NumberField::length(tl!("Spacing:"), Bind::given(Some(v.spacing)), lengths).show(ui);
        if let Some(Some(spacing)) = field.edit.value().copied() {
            v.spacing = spacing;
        }
    });
    Field::new(tl!("Options:")).id_salt("options").show(ui, |ui, _| {
        if let Some(on) = check(ui, tl!("Move Artwork with Artboard"), v.move_artwork, true) {
            v.move_artwork = on;
        }
    });
    Text::new(crate::i18n::fmt(tl!("Artboards: {n}"), &[("n", &artboards.to_string())])).role(TypeRole::Caption).show(ui);
}

impl StackDialog for RearrangeArtboardsDialog {
    type Result = RearrangeValues;

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(CMD)
    }

    fn show(&mut self, ui: &mut egui::Ui, _: &DialogStack) -> Option<DialogResult<RearrangeValues>> {
        let (artboards, lengths) = (self.artboards, self.lengths);
        let shown = FormDialog::new(tl!("Rearrange All Artboards"), Bind::bound(&mut self.values)).opened(Bind::bound(&mut self.open)).show(
            ui,
            |ui, _, v| {
                body(ui, v, artboards, lengths);
                true
            },
        );
        match (shown.accepted, shown.closed) {
            (Some(value), _) => Some(DialogResult::Ok(value)),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }

    fn confirm(&mut self, _: &egui::Context) -> Option<RearrangeValues> {
        Some(self.values.clone())
    }

    fn json_fields(&self) -> Option<Value> {
        let v = &self.values;
        Some(json!({"layout": v.layout.id(), "order": v.order(), "columns": v.columns, "spacing": v.spacing, "moveArtwork": v.move_artwork}))
    }

    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        let rejects = || FieldRefusal::Rejects(&value).message(field);
        let inactive = || FieldRefusal::Inactive.message(field);
        let v = &mut self.values;
        match field {
            "layout" => v.layout = value.as_str().and_then(ArtboardLayout::parse).ok_or_else(rejects)?,
            "order" => {
                let rtl = match value.as_str() {
                    Some("leftToRight") => false,
                    Some("rightToLeft") => true,
                    _ => return Err(rejects()),
                };
                if !v.has_order() {
                    return Err(inactive());
                }
                v.rtl = rtl;
            }
            "columns" => {
                let columns = value.as_f64().filter(|c| c.is_finite()).ok_or_else(rejects)?;
                if !v.is_grid() {
                    return Err(inactive());
                }
                v.columns = columns.round().clamp(1.0, self.artboards.max(1) as f64);
            }
            "spacing" => v.spacing = value.as_f64().filter(|s| s.is_finite()).ok_or_else(rejects)?,
            "moveArtwork" => v.move_artwork = value.as_bool().ok_or_else(rejects)?,
            _ => return Err(FieldRefusal::Unknown.message(field)),
        }
        Ok(())
    }
}

/// Runs `artboard.rearrange` for the settings OK confirmed, and remembers the spacing of a successful one.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let Some(DialogResult::Ok(values)) = dialogs.with(|host| host.take::<RearrangeArtboardsDialog>()) else { return };
    match app.run(CMD, values.params()) {
        Ok(_) => app.ui.rearrange_artboards_spacing = json!(values.spacing),
        Err(e) => app.status(e),
    }
    ctx.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn app(artboards: u32) -> VectorcraftApp {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 100, "height": 50, "artboards": artboards})).unwrap();
        app
    }

    fn origins(app: &VectorcraftApp) -> Vec<(f64, f64)> {
        app.session.active().unwrap().doc.artboards.iter().map(|a| (a.rect.x0, a.rect.y0)).collect()
    }

    /// Opens the dialog as the menu item does.
    fn open_from_menu(app: &mut VectorcraftApp) {
        let (id, p) = crate::menus::click_target("Rearrange All Artboards…", CMD, &Value::Null);
        crate::menus::invoke(app, &id, p);
    }

    fn open_id(app: &VectorcraftApp) -> DialogId {
        app.dialogs().unwrap().with(|host| host.open_dialogs().last().map(|d| d.id)).unwrap()
    }

    /// Sets `edits` in the open dialog in order; the first refusal is returned.
    fn set(app: &VectorcraftApp, edits: &[(&str, Value)]) -> Result<(), String> {
        let (ctx, id) = (app.ui_ctx.clone().unwrap(), open_id(app));
        edits.iter().try_for_each(|(field, value)| app.dialogs().unwrap().with(|host| host.set_json_field(&ctx, id, field, value.clone())))
    }

    /// Presses OK and runs the host pump, which runs the command.
    fn ok(app: &mut VectorcraftApp) {
        let (ctx, id) = (app.ui_ctx.clone().unwrap(), open_id(app));
        app.dialogs().unwrap().with(|host| host.confirm_by_id(&ctx, id));
        crate::dialogs::host::pump(app, &ctx);
    }

    /// #681: the menu item opens the dialog; each setting reaches `artboard.rearrange`, as one undo step; a
    /// setting the layout makes meaningless is refused.
    #[test]
    fn rearrange_fields_reach_the_command() {
        type Settings<'a> = &'a [(&'a str, Value)];
        type Positions<'a> = &'a [(f64, f64)];
        let cases: [(u32, Settings<'_>, Positions<'_>); 2] = [
            (3, &[("layout", json!("row")), ("order", json!("rightToLeft")), ("spacing", json!(10))], &[(220.0, 0.0), (110.0, 0.0), (0.0, 0.0)]),
            (
                4,
                &[("layout", json!("gridByColumn")), ("order", json!("rightToLeft")), ("columns", json!(2))],
                &[(120.0, 0.0), (120.0, 70.0), (0.0, 0.0), (0.0, 70.0)],
            ),
        ];
        for (artboards, edits, want) in cases {
            let mut app = app(artboards);
            open_from_menu(&mut app);
            set(&app, edits).unwrap();
            let undo = app.session.doc().unwrap().history.undo.len();
            ok(&mut app);
            assert!(!app.any_dialog_open());
            assert_eq!(origins(&app), want);
            assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
        }
        let mut app = app(3);
        open_from_menu(&mut app);
        assert!(set(&app, &[("layout", json!("column")), ("order", json!("rightToLeft"))]).is_err(), "one column has no order");
        assert!(set(&app, &[("layout", json!("column")), ("columns", json!(2))]).is_err(), "one column has no count");
        assert!(set(&app, &[("layout", json!("row")), ("columns", json!(2))]).is_err(), "one row has no count");
        assert!(set(&app, &[("layout", json!("gridByRow")), ("columns", json!(2))]).is_ok());
    }

    /// The spacing confirmed last is the one the dialog opens with next time (#4558).
    #[test]
    fn keeps_spacing_when_reopening_after_rearranging() {
        let mut app = app(3);
        app.run("ui.menuDialog", json!({"command": CMD})).unwrap();
        set(&app, &[("spacing", json!(37))]).unwrap();
        ok(&mut app);

        app.run("ui.menuDialog", json!({"command": CMD})).unwrap();
        let fields = app.dialogs().unwrap().with(|host| host.open_dialogs().last().and_then(|d| d.fields.clone())).unwrap();
        assert_eq!(fields["spacing"].as_f64(), Some(37.0));
    }
}
