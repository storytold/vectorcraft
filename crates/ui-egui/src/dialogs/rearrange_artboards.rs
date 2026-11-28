//! Object → Artboards → Rearrange All Artboards (also the Artboards panel's button and menu, and
//! the Properties panel's Rearrange All): Layout (a grid by row or by column, one row or one
//! column), Layout Order (left to right or right to left), Columns (Rows for a grid by column),
//! Spacing and Move Artwork with Artboard, with the document's artboard count. OK runs
//! `artboard.rearrange` as one undo step (#681).
//!
//! Rearrange (the Artboard tool's canvas menu and Contextual Task Bar, and the Artboards panel
//! menu's Rearrange…) is the same dialog for the selected artboards (field `indices`), with their
//! count. Its OK waits for a click on the canvas, which places the grid's top-left there: the
//! grid's outline follows the pointer until then, Enter places it at the pointer (or, off the
//! canvas, at the first selected artboard's top-left), a Space-drag still pans, and Esc, another
//! tool or another document cancels it. The click or Enter runs `artboard.rearrange` with
//! `indices` and `at` as one undo step
//! (https://helpx.adobe.com/illustrator/desktop/create-manage-artboards/organize-manage-artboards/rearrange-artboards.html).
//!
//! Fields: `layout` (`gridByRow` | `gridByColumn` | `row` | `column`), `order` (`leftToRight` |
//! `rightToLeft`), `columns` (the rows of a grid by column), `spacing` (pt), `moveArtwork` and, for
//! Rearrange, `indices`.

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
use vectorcraft_engine::cmd::newdoc::{ArtboardLayout, rearrange_grid};
use vectorcraft_geom::{BezPath, Point};
use vectorcraft_tools::Overlay;

use super::layer_options::check;
use crate::VectorcraftApp;
use crate::page_glyphs::LayoutGlyph;
use crate::state::RearrangePlacement;

/// The command Rearrange All Artboards runs; the key of its dialog.
pub(crate) const CMD: &str = "artboard.rearrange";

/// The status bar's message while a Rearrange waits for its click.
pub(crate) const PLACE_HINT: &str = "Click on the canvas to place the artboards (Esc cancels)";

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

/// `ui.menuDialog` for Rearrange All Artboards: opens the dialog on the spacing of its last OK. A
/// placement still waiting is dropped.
pub(crate) fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    cancel_placement(app);
    open_with(app, None)
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
    if let Err(e) = open_with(app, Some(indices)) {
        app.status(e);
    }
}

/// Opens the dialog for `indices` (Rearrange), or for every artboard (Rearrange All Artboards).
fn open_with(app: &mut VectorcraftApp, indices: Option<Vec<usize>>) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document")?;
    let artboards = indices.as_ref().map_or(st.doc.artboards.len(), Vec::len);
    let selected = indices
        .map(|indices| {
            indices.into_iter().map(|i| st.doc.artboards.get(i).map(|a| (i, a.id)).ok_or("no such artboard")).collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    let values = RearrangeValues { spacing: app.ui.rearrange_artboards_spacing.as_f64().unwrap_or(20.0), ..RearrangeValues::default() };
    let document = (st.uid, st.reverts);
    let lengths = LengthUnits::new(UnitKind::Points, length_kind(app.session.general_unit()), 72.0);
    let lengths = crate::fields::bare_points(lengths, app.session.prefs.numbers_without_units_are_points);
    let dialogs = app.dialogs().ok_or("no window")?;
    dialogs.with(|host| host.open(RearrangeArtboardsDialog { id: dialog_id(CMD), open: true, values, document, selected, artboards, lengths }));
    Ok(Value::Null)
}

/// The artboards of a menu item's params that list nothing but `indices` (the Artboard tool's
/// Rearrange), as whole numbers.
pub(crate) fn menu_indices(p: &Value) -> Option<Vec<usize>> {
    let o = p.as_object().filter(|o| o.len() == 1)?;
    let list = o.get("indices")?.as_array()?;
    list.iter().map(|i| i.as_u64().and_then(|i| usize::try_from(i).ok())).collect()
}

/// The Rearrange All Artboards dialog, or Rearrange for the selected artboards; its answer is the
/// settings OK confirmed, which [`pump`] runs or leaves waiting for a click on the canvas.
pub(crate) struct RearrangeArtboardsDialog {
    id: DialogId,
    open: bool,
    values: RearrangeValues,
    document: (u64, u64),
    /// Each selected artboard's opening row index and stable ID. None lays out the entire document.
    selected: Option<Vec<(usize, u32)>>,
    /// The artboards the dialog lays out (the document's, or the selected ones), which bounds Columns
    /// and is the count it shows.
    artboards: usize,
    /// The lengths Spacing keeps and shows.
    lengths: LengthUnits,
}

pub(crate) struct RearrangeAnswer {
    values: RearrangeValues,
    document: (u64, u64),
    selected: Option<Vec<(usize, u32)>>,
}

impl RearrangeArtboardsDialog {
    fn answer(&self, values: RearrangeValues) -> RearrangeAnswer {
        RearrangeAnswer { values, document: self.document, selected: self.selected.clone() }
    }
}

/// The rows of the dialog for `artboards` artboards, Spacing shown in `unit`.
fn body(ui: &mut egui::Ui, v: &mut RearrangeValues, artboards: usize, selected: bool, lengths: LengthUnits) {
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
    let label = if selected { tl!("Selected Artboards: {n}") } else { tl!("Artboards: {n}") };
    Text::new(crate::i18n::fmt(label, &[("n", &artboards.to_string())])).role(TypeRole::Caption).show(ui);
}

impl StackDialog for RearrangeArtboardsDialog {
    type Result = RearrangeAnswer;

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(CMD)
    }

    fn show(&mut self, ui: &mut egui::Ui, _: &DialogStack) -> Option<DialogResult<RearrangeAnswer>> {
        let (artboards, lengths) = (self.artboards, self.lengths);
        let selected = self.selected.is_some();
        let title = if selected { tl!("Rearrange") } else { tl!("Rearrange All Artboards") };
        let shown = FormDialog::new(title, Bind::bound(&mut self.values)).opened(Bind::bound(&mut self.open)).show(ui, |ui, _, v| {
            body(ui, v, artboards, selected, lengths);
            true
        });
        match (shown.accepted, shown.closed) {
            (Some(value), _) => Some(DialogResult::Ok(self.answer(value))),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }

    fn confirm(&mut self, _: &egui::Context) -> Option<RearrangeAnswer> {
        Some(self.answer(self.values.clone()))
    }

    fn json_fields(&self) -> Option<Value> {
        let v = &self.values;
        let mut fields =
            json!({"layout": v.layout.id(), "order": v.order(), "columns": v.columns, "spacing": v.spacing, "moveArtwork": v.move_artwork});
        if let Some(selected) = &self.selected {
            fields["indices"] = json!(selected.iter().map(|(index, _)| index).collect::<Vec<_>>());
        }
        Some(fields)
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

/// Runs `artboard.rearrange` for the settings OK confirmed, and remembers the spacing of a successful
/// one. Rearrange for the selected artboards runs nothing yet: it waits for its click on the canvas
/// ([`place`]).
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let Some(DialogResult::Ok(RearrangeAnswer { values, document, selected })) = dialogs.with(|host| host.take::<RearrangeArtboardsDialog>()) else {
        return;
    };
    let applied = match app.session.active() {
        Some(st) if st.uid == document.0 && st.reverts != document.1 => Err(tl!("Reopen Rearrange Artboards after reverting the drawing").to_owned()),
        Some(st) if (st.uid, st.reverts) == document => match selected {
            Some(selected) => wait_for_click(app, &selected, values.params()),
            None => app.run(CMD, values.params()).map(drop),
        },
        _ => Err(tl!("Rearrange Artboards can only change the document where it was opened.").to_owned()),
    };
    match applied {
        Ok(()) => app.ui.rearrange_artboards_spacing = json!(values.spacing),
        Err(e) => app.status(e),
    }
    ctx.request_repaint();
}

/// Rearrange the originally selected artboards in their current row order when the canvas is clicked.
fn wait_for_click(app: &mut VectorcraftApp, selected: &[(usize, u32)], mut params: Value) -> Result<(), String> {
    let st = app.session.active().ok_or_else(|| "no document".to_string())?;
    let wanted: std::collections::HashSet<_> = selected.iter().map(|&(_, id)| id).collect();
    let (indices, ids): (Vec<_>, Vec<_>) =
        st.doc.artboards.iter().enumerate().filter_map(|(index, a)| wanted.contains(&a.id).then_some((index, a.id))).unzip();
    if ids.len() != selected.len() {
        return Err(tl!("An artboard selected when the dialog opened was removed.").to_owned());
    }
    params["indices"] = json!(indices);
    let (doc, tool) = (st.uid, app.session.tool_id().to_string());
    app.ui.rearrange_placement = Some(RearrangePlacement { doc, reverts: st.reverts, tool, params, ids, armed: false });
    app.status(PLACE_HINT);
    Ok(())
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
/// tool is no longer in use, or whose confirmed artboards were reloaded, removed or reordered
/// is dropped.
pub(crate) fn refresh_placement(app: &mut VectorcraftApp) -> bool {
    let Some(pl) = app.ui.rearrange_placement.as_ref() else { return false };
    let same = app.session.active().filter(|d| d.uid == pl.doc && d.reverts == pl.reverts).is_some_and(|d| {
        let listed = placed(pl);
        listed.len() == pl.ids.len() && listed.iter().zip(&pl.ids).all(|(i, id)| d.doc.artboards.get(*i).is_some_and(|a| a.id == *id))
    });
    if same && app.session.tool_id() == pl.tool {
        return true;
    }
    cancel_placement(app);
    false
}

/// Whether a placement waits and has been seen by one canvas pass, so a key that arrives now is not
/// the Enter that confirmed its dialog.
pub(crate) fn armed(app: &VectorcraftApp) -> bool {
    app.ui.rearrange_placement.as_ref().is_some_and(|pl| pl.armed)
}

/// Marks a waiting placement as seen by the canvas (see [`armed`]).
pub(crate) fn arm(app: &mut VectorcraftApp) {
    if let Some(pl) = app.ui.rearrange_placement.as_mut() {
        pl.armed = true;
    }
}

/// Runs the waiting placement's `artboard.rearrange` with the grid's top-left at `at` (the pressed
/// point, or the pointer when Enter placed it); without `at` the grid starts at the first selected
/// artboard's top-left. When every selected artboard is locked, the status bar says so.
pub(crate) fn place(app: &mut VectorcraftApp, at: Option<Point>) {
    if !refresh_placement(app) {
        return;
    }
    let Some(mut pl) = app.ui.rearrange_placement.take() else { return };
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
}

/// The grid a waiting placement makes with its top-left at the pointer (`app.hover_doc`): the
/// outline of each artboard that moves, in the Artboard tool's color, 1 point wide on screen.
pub(crate) fn placement_overlays(app: &VectorcraftApp) -> Vec<Overlay> {
    let Some(pl) = app.ui.rearrange_placement.as_ref().filter(|pl| app.session.tool_id() == pl.tool) else { return vec![] };
    let (Some(at), Some(st)) = (app.hover_doc, app.session.active().filter(|d| d.uid == pl.doc && d.reverts == pl.reverts)) else {
        return vec![];
    };
    let boards: Vec<&vectorcraft_doc::Artboard> = placed(pl).into_iter().filter_map(|i| st.doc.artboards.get(i)).collect();
    let sizes: Vec<(f64, f64)> = boards.iter().map(|a| (a.rect.width(), a.rect.height())).collect();
    let p = &pl.params;
    let layout = p["layout"].as_str().and_then(ArtboardLayout::parse).unwrap_or_default();
    let lines = p["columns"].as_f64().filter(|c| c.is_finite()).map_or(2, |c| c.round().max(1.0) as u64);
    let spacing = p["spacing"].as_f64().filter(|s| s.is_finite()).unwrap_or(20.0).clamp(-1.0e5, 1.0e5);
    let (origins, ..) = rearrange_grid(&sizes, at, layout, lines, spacing, p["order"] == "rightToLeft");
    // A locked artboard keeps its cell but stays where it is.
    origins
        .iter()
        .zip(&sizes)
        .zip(&boards)
        .filter(|(_, a)| !a.locked)
        .map(|((o, (w, h)), _)| {
            let mut path = BezPath::new();
            path.move_to(*o);
            path.line_to((o.x + w, o.y));
            path.line_to((o.x + w, o.y + h));
            path.line_to((o.x, o.y + h));
            path.close_path();
            Overlay::Path { path, color: vectorcraft_tools::xform::BLUE, width: 1.0, dashed: false }
        })
        .collect()
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

    /// The open dialog's fields.
    fn fields(app: &VectorcraftApp) -> Value {
        app.dialogs().unwrap().with(|host| host.open_dialogs().last().and_then(|d| d.fields.clone())).unwrap()
    }

    fn check_rearrange_stays_in_opening_document(selected: bool) {
        let mut app = app(3);
        app.run("file.new", json!({"width": 100, "height": 50, "artboards": 3})).unwrap();
        let destination = app.session.active().unwrap().uid;
        app.run("document.activate", json!({"index": 0})).unwrap();
        if selected {
            app.select_tool("artboard");
            open_selected(&mut app, &[0, 2]);
        } else {
            open_from_menu(&mut app);
        }
        set(&app, &[("layout", json!("row")), ("order", json!("rightToLeft")), ("spacing", json!(37))]).unwrap();

        app.run("document.activate", json!({"index": 1})).unwrap();
        assert_eq!(app.session.active().unwrap().uid, destination);
        let before = origins(&app);
        let undo = app.session.active().unwrap().history.undo.len();
        let spacing = app.ui.rearrange_artboards_spacing.clone();
        ok(&mut app);
        assert_eq!(origins(&app), before, "OK leaves the other tab's artboards in place");
        assert!(app.ui.rearrange_placement.is_none(), "the other tab cannot start placement");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
        assert_eq!(app.ui.rearrange_artboards_spacing, spacing);
    }

    #[test]
    fn rearrange_opening_for_all_artboards_leaves_a_new_tab_untouched() {
        check_rearrange_stays_in_opening_document(false);
    }

    #[test]
    fn rearrange_opening_for_selected_artboards_cannot_place_in_a_new_tab() {
        check_rearrange_stays_in_opening_document(true);
    }

    fn check_revert_retires_rearrange(selected: bool, already_confirmed: bool) {
        let mut app = app(3);
        app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|e| e.to_string())));
        app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string())));
        let dir = vectorcraft_testkit::TempDir::new("rearrange-artboards-revert");
        app.run("file.saveAs", json!({"path": dir.join("artboards.vectorcraft")})).unwrap();
        let saved = origins(&app);
        let source = app.session.active().unwrap();
        let version = (source.uid, source.reverts);

        app.run(CMD, json!({"layout": "row", "spacing": 47})).unwrap();
        assert_ne!(origins(&app), saved);
        if selected {
            app.select_tool("artboard");
            open_selected(&mut app, &[0, 2]);
        } else {
            open_from_menu(&mut app);
        }
        set(&app, &[("layout", json!("row")), ("spacing", json!(37))]).unwrap();
        if already_confirmed {
            ok(&mut app);
            assert!(refresh_placement(&mut app));
        }

        app.run("file.revert", json!({"confirmed": true})).unwrap();
        let source = app.session.active().unwrap();
        assert_eq!(source.uid, version.0);
        assert_ne!(source.reverts, version.1);
        assert_eq!(origins(&app), saved);
        let undo = source.history.undo.len();
        let remembered = app.ui.rearrange_artboards_spacing.clone();
        if already_confirmed {
            assert!(!refresh_placement(&mut app), "reloaded artboards cannot receive a pending placement");
            place(&mut app, Some(Point::new(700.0, 300.0)));
        } else {
            ok(&mut app);
        }
        assert_eq!(origins(&app), saved, "reloaded artboards stay at their saved locations");
        assert!(app.ui.rearrange_placement.is_none());
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
        assert_eq!(app.ui.rearrange_artboards_spacing, remembered);
    }

    #[test]
    fn revert_retires_rearrange_all_artboards_settings() {
        check_revert_retires_rearrange(false, false);
    }

    #[test]
    fn revert_retires_rearrange_selected_artboards_settings() {
        check_revert_retires_rearrange(true, false);
    }

    #[test]
    fn revert_retires_rearrange_canvas_placement() {
        check_revert_retires_rearrange(true, true);
    }

    #[test]
    fn rearrange_opening_keeps_the_intended_artboards_after_rows_move() {
        let mut app = app(3);
        app.select_tool("artboard");
        let st = app.session.active().unwrap();
        let source = [st.doc.artboards[0].id, st.doc.artboards[2].id];
        open_selected(&mut app, &[0, 2]);
        set(&app, &[("layout", json!("row")), ("spacing", json!(20))]).unwrap();
        app.run("artboard.reorder", json!({"index": 0, "to": 1})).unwrap();
        let before = origins(&app);
        let undo = app.session.active().unwrap().history.undo.len();

        ok(&mut app);
        let placement = app.ui.rearrange_placement.as_ref().expect("the original artboards wait for placement");
        assert_eq!(placement.ids, source);
        assert_eq!(placement.params["indices"], json!([1, 2]));
        place(&mut app, Some(Point::new(700.0, 300.0)));
        assert_eq!(origins(&app), [before[0], (700.0, 300.0), (820.0, 300.0)]);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
    }

    #[test]
    fn rearrange_opening_does_not_place_a_deleted_artboards_replacement() {
        let mut app = app(3);
        app.select_tool("artboard");
        open_selected(&mut app, &[0]);
        app.run("artboard.delete", json!({"index": 0})).unwrap();
        let before = origins(&app);
        let undo = app.session.active().unwrap().history.undo.len();

        ok(&mut app);
        assert!(app.ui.rearrange_placement.is_none(), "the replacement row is not a placement target");
        assert_eq!(origins(&app), before);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
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
        assert_eq!(fields(&app)["spacing"].as_f64(), Some(37.0));
    }

    /// The Artboard tool's canvas menu opens Rearrange for the artboards the tool has selected,
    /// with their count; OK moves nothing and waits for a click on the canvas with those artboards
    /// and the dialog's fields. Rearrange All Artboards still lays out every artboard at once.
    #[test]
    fn rearrange_from_the_canvas_menu_waits_for_a_click_with_the_selected_artboards() {
        let mut app = app(3);
        app.select_tool("artboard");
        app.session.set_tool_option("selected", &json!([0, 2]));
        let items = crate::menus::context_items(&app);
        let target = items.iter().find_map(|i| match i {
            crate::menus::Node::Item(item) if item.label == "Rearrange" => {
                Some(crate::menus::click_target("Rearrange", &item.command.id, &item.command.params))
            }
            _ => None,
        });
        let (id, p) = target.expect("Rearrange in the canvas menu");
        crate::menus::invoke(&mut app, &id, p);
        assert!(app.dialogs().unwrap().with(|host| host.get::<RearrangeArtboardsDialog>().is_some()), "Rearrange opens");
        let shown = fields(&app);
        assert_eq!(shown["indices"], json!([0, 2]));
        let (title, count) = app.dialogs().unwrap().with(|host| {
            let d = host.get::<RearrangeArtboardsDialog>().unwrap();
            (d.selected.is_some(), d.artboards)
        });
        assert_eq!((title, count), (true, 2), "Rearrange for the 2 selected artboards");
        set(&app, &[("layout", json!("row"))]).unwrap();
        let (before, undo) = (origins(&app), app.session.doc().unwrap().history.undo.len());
        ok(&mut app);
        assert!(!app.any_dialog_open());
        assert_eq!((origins(&app), app.session.doc().unwrap().history.undo.len()), (before.clone(), undo));
        let waiting = app.ui.rearrange_placement.as_ref().expect("waits for a click");
        assert_eq!((waiting.params["indices"].clone(), waiting.params["layout"].clone()), (json!([0, 2]), json!("row")));
        assert_eq!(app.ui.status, PLACE_HINT);

        // Rearrange All Artboards drops it and runs at once.
        app.run("ui.menuDialog", json!({"command": CMD})).unwrap();
        assert!(app.ui.rearrange_placement.is_none());
        assert!(fields(&app).get("indices").is_none());
        ok(&mut app);
        assert_ne!(origins(&app), before);
        assert_ne!(app.ui.status, PLACE_HINT);
    }

    /// The outline of a waiting placement follows the pointer: the first selected artboard's cell
    /// starts at the pointer, and a locked selected artboard has none.
    #[test]
    fn the_placement_outline_starts_at_the_pointer_and_leaves_locked_artboards_out() {
        let mut app = app(3);
        app.session.execute("artboard.setProps", &json!({"index": 2, "locked": true})).unwrap();
        app.select_tool("artboard");
        app.session.set_tool_option("selected", &json!([0, 1, 2]));
        crate::panels::artboards::open_rearrange_selected(&mut app);
        set(&app, &[("layout", json!("row")), ("spacing", json!(10))]).unwrap();
        ok(&mut app);
        assert!(placement_overlays(&app).is_empty(), "no pointer over the canvas");
        app.hover_doc = Some(Point::new(500.0, 300.0));
        let starts: Vec<Point> = placement_overlays(&app)
            .iter()
            .map(|o| match o {
                Overlay::Path { path, .. } => path.elements().first().and_then(|e| e.end_point()).unwrap(),
                o => panic!("{o:?}"),
            })
            .collect();
        assert_eq!(starts, [Point::new(500.0, 300.0), Point::new(610.0, 300.0)]);
    }
}
