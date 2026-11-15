//! Unit Converter in the interface: its submenu (Object › Plug-ins › Unit Converter), which opens
//! the panel, and the panel. The panel shows one length in every unit of
//! [`Unit::EAST_ASIAN`], one number field each. A value typed in any field (units and arithmetic,
//! as every number field reads them) updates the others as it is typed; text that does not read
//! leaves them as they were. From Selection reads the selection's width and height
//! (`addon.units.selectionSize`), and Width or Height picks which of the two the fields show; the
//! size read is dropped when the document, its selection or the bounds measure changes. Apply gives the selection that
//! width or height through `object.setBounds` as the Transform panel's W or H field does (its
//! reference point, Constrain Width and Height Proportions and Transform Object Only, Pattern
//! Only or Both), as one undo step. Each field's copy button puts its value on the clipboard, with
//! its unit when Copy with Units is on.

use egui::Ui;
use serde_json::json;
use vectorcraft_doc::{Unit, finite_in_every_unit};

use crate::VectorcraftApp;
use crate::menus::Item;
use crate::panels::transform;
use crate::panels::{pstate, set_pstate};
use crate::widgets;

/// The panel's id (`window.panel {panel: "unitConverter"}`).
pub const ID: &str = "unitConverter";
/// The command that measures the selection.
pub const SELECTION_SIZE: &str = "addon.units.selectionSize";

/// Where the panel keeps its length and choices (panel state).
const STATE: &str = "unit-converter";
/// A row (label, field, copy button and two gaps) is 214 pt wide, inside the 240 pt body of the
/// default 260 pt floating panel with room for its scroll bar.
const LABEL_W: f32 = 90.0;
const FIELD_W: f32 = 90.0;

/// Object › Plug-ins › Unit Converter.
pub fn menu() -> Vec<Item> {
    vec![Item::Cmd("Unit Converter Panel", "window.panel", json!({ "panel": ID }))]
}

/// The selection's side the fields show and Apply sets.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub(crate) enum Side {
    #[default]
    Width,
    Height,
}

impl Side {
    /// `object.setBounds`'s param for this side.
    fn key(self) -> &'static str {
        match self {
            Side::Width => "width",
            Side::Height => "height",
        }
    }
}

/// The panel's length and choices.
#[derive(Clone, Default, Debug, PartialEq)]
pub(crate) struct Converter {
    /// The length (points), none until one is typed or read from the selection.
    pub points: Option<f64>,
    /// The selection's width and height (points) From Selection last read.
    pub size: Option<(f64, f64)>,
    /// The active document's uid and revision, and the bounds measure (Use Preview Bounds,
    /// glyph bounds), when [`Converter::size`] was read.
    pub read_at: Option<ReadAt>,
    /// Which of them the fields show, and the side Apply sets.
    pub side: Side,
    /// Copy puts the unit after the number.
    pub with_units: bool,
}

impl Converter {
    /// Show `side`: the fields take that side of the size From Selection read, if any.
    pub fn pick(&mut self, side: Side) {
        self.side = side;
        if let Some((w, h)) = self.size {
            self.points = Some(if side == Side::Width { w } else { h });
        }
    }

    /// A length typed in a field: after From Selection it replaces the side shown, so picking the
    /// other side and back keeps it.
    pub fn set(&mut self, pt: f64) {
        self.points = Some(pt);
        if let Some((w, h)) = &mut self.size {
            *(if self.side == Side::Width { w } else { h }) = pt;
        }
    }

    /// Drop the size From Selection read when the active document is no longer the one it was
    /// read from or is measured another way (`now`: its uid and revision, which a change to the
    /// selection also moves, and the bounds measure). The length the fields show stays.
    pub fn follow(&mut self, now: Option<ReadAt>) {
        if self.size.is_some() && self.read_at != now {
            self.size = None;
            self.read_at = None;
        }
    }

    /// What the copy button of the field in `unit` puts on the clipboard.
    pub fn copied(&self, unit: Unit) -> Option<String> {
        self.points.map(|pt| if self.with_units { unit.format(pt) } else { unit.number(pt) })
    }
}

/// A document's uid and revision, and the bounds measure.
pub(crate) type ReadAt = (u64, u64, vectorcraft_effects::preview::Measure);

/// The active document's uid and revision, and the bounds measure.
pub(crate) fn document_now(app: &VectorcraftApp) -> Option<ReadAt> {
    app.session.active().map(|st| (st.uid, st.revision, app.session.bounds_measure()))
}

/// From Selection: read the selection's width and height into `st`, and show the side picked.
pub(crate) fn from_selection(app: &mut VectorcraftApp, st: &mut Converter) -> Result<(), String> {
    let r = app.run(SELECTION_SIZE, json!({}))?;
    if let (Some(w), Some(h)) = (r["width"]["points"].as_f64(), r["height"]["points"].as_f64()) {
        st.size = Some((w, h));
        st.read_at = document_now(app);
        st.pick(st.side);
    }
    Ok(())
}

/// Apply: give the selection the length as its width or height, as the Transform panel's W or H
/// field does: from its reference point (a symbol instance's registration point with Use
/// Registration Point for Symbol on), with the other side following under Constrain Width and
/// Height Proportions, and with the panel menu's Transform Object Only, Pattern Only or Both.
/// Without a length it does nothing.
pub(crate) fn apply(app: &mut VectorcraftApp, ui: &Ui, st: &Converter) -> Result<(), String> {
    let Some(pt) = st.points else { return Ok(()) };
    let reference = app.selection_box().map_or_else(|| transform::reference_index(ui), |b| transform::panel_reference(app, ui, &b).index);
    let params = json!({ st.side.key(): pt, "reference": reference, "proportional": transform::proportional(app, ui) });
    let params = transform::with_pattern_switches(app, params);
    app.run("object.setBounds", params)?;
    Ok(())
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let mut st: Converter = pstate(ui.ctx(), STATE);
    st.follow(document_now(app));
    // A number typed without a unit is in its row's unit, so 2 in the Picas row is 2 picas, also
    // with Preferences › Units › Numbers Without Units Are Points on; other panels' fields keep it.
    let bare_points = widgets::typed_unit(ui.ctx(), Unit::Picas) == Unit::Points;
    widgets::set_bare_numbers_are_points(ui.ctx(), false);
    for unit in Unit::EAST_ASIAN {
        widgets::label_row(ui, unit.label(), LABEL_W, |ui| {
            // A length too large to show in one of the units (near the largest number, in Q and
            // Ha) leaves the others as they were, as text that does not read does.
            if let Some(pt) = widgets::num_field_live(ui, ("unit-converter", unit.key()), st.points, unit, FIELD_W)
                && finite_in_every_unit(pt)
            {
                st.set(pt);
            }
            // The button's name and tip carry the row's unit ("Copy Millimeters").
            let tip = format!("{} {}", tl!("Copy"), tl!(unit.label()));
            let copy = widgets::icon_button_enabled(ui, "copy", &tip, false, st.points.is_some(), 22.0);
            if copy.clicked()
                && let Some(text) = st.copied(unit)
            {
                ui.ctx().copy_text(text);
            }
        });
    }
    widgets::set_bare_numbers_are_points(ui.ctx(), bare_points);
    if widgets::check(ui, "Copy with Units", st.with_units, true) {
        st.with_units = !st.with_units;
    }
    ui.add_space(6.0);
    widgets::subheader(ui, tl!("Selection"));
    let selected = app.session.active().is_some_and(|d| !d.selection.is_empty());
    ui.horizontal(|ui| {
        for (side, label) in [(Side::Width, "Width"), (Side::Height, "Height")] {
            if widgets::radio(ui, label, st.side == side, true) {
                st.pick(side);
            }
        }
    });
    ui.horizontal(|ui| {
        let read = ui.add_enabled_ui(selected, |ui| widgets::flat_button(ui, tl!("From Selection"), 110.0)).inner;
        if read.clicked()
            && let Err(e) = from_selection(app, &mut st)
        {
            app.status(e);
        }
        let set = ui.add_enabled_ui(selected && st.points.is_some(), |ui| widgets::flat_button(ui, tl!("Apply"), 70.0)).inner;
        if set.clicked()
            && let Err(e) = apply(app, ui, &st)
        {
            app.status(e);
        }
    });
    set_pstate(ui.ctx(), STATE, st);
}

#[cfg(test)]
mod tests;
