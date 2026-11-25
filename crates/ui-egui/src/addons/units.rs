//! Unit Converter in the interface: its submenu (Object › Plug-ins › Unit Converter), which opens
//! the panel, and the panel. The panel shows one length in every unit of
//! [`Unit::EAST_ASIAN`], one number field each. A value typed in any field (units and arithmetic,
//! as every number field reads them) updates the others; a length too large to show in every unit
//! leaves them as they were. From Selection reads the selection's width and height
//! (`addon.units.selectionSize`), and Width or Height picks which of the two the fields show; the
//! size read is dropped when the document, its selection or the bounds measure changes. Apply gives the selection that
//! width or height through `object.setBounds` as the Transform panel's W or H field does (its
//! reference point, Constrain Width and Height Proportions and Transform Object Only, Pattern
//! Only or Both), as one undo step. Each field's copy button puts its value on the clipboard, with
//! its unit when Copy with Units is on.

use craft::behavior::{IconId, NavShape};
use craft::menu::Node;
use craft::ui::buttons::{Button, IconButton};
use craft::ui::choice::{Checkbox, Radio, RadioGroup};
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::runtime::env::{LengthUnits, UnitKind};
use craft::ui::text::NumberField;
use egui::Ui;
use serde_json::json;
use vectorcraft_doc::{Unit, finite_in_every_unit};

use crate::VectorcraftApp;
use crate::canvas::rulers::unit_kind;
use crate::menus::cp;
use crate::panels::empty_state;
use crate::panels::transform;

/// The panel's id (`window.panel {panel: "unitConverter"}`).
pub const ID: &str = "unitConverter";
/// The command that measures the selection.
pub const SELECTION_SIZE: &str = "addon.units.selectionSize";

/// Object › Plug-ins › Unit Converter.
pub fn menu() -> Vec<Node> {
    vec![cp("Unit Converter Panel", "window.panel", json!({ "panel": ID }))]
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

    /// The value a field now holds: a changed length that shows in every unit replaces the
    /// length; one too large for some unit (near the largest number, in Q and Ha) leaves the
    /// other fields as they were.
    pub fn typed(&mut self, value: Option<f64>) {
        if let Some(pt) = value
            && Some(pt) != self.points
            && finite_in_every_unit(pt)
        {
            self.set(pt);
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
pub(crate) fn apply(app: &mut VectorcraftApp, st: &Converter) -> Result<(), String> {
    let Some(pt) = st.points else { return Ok(()) };
    let reference = app.selection_box().map_or_else(|| app.ui.transform_panel.reference_point, |b| transform::panel_reference(app, &b).index);
    let params = json!({ st.side.key(): pt, "reference": reference, "proportional": app.session.prefs.constrain_proportions });
    let params = transform::with_pattern_switches(app, params);
    app.run("object.setBounds", params)?;
    Ok(())
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    if app.session.active().is_none() {
        empty_state(ui, "ruler", tl!("No document"), tl!("Open a document to convert lengths."));
        return;
    }
    let selected = app.session.active().is_some_and(|d| !d.selection.is_empty());
    let mut conv = std::mem::take(&mut app.addon_panels.units);
    conv.follow(document_now(app));

    let mut typed = conv.points;
    let mut copy = None;
    for unit in Unit::EAST_ASIAN {
        let lengths = LengthUnits::new(UnitKind::Points, unit_kind(unit), 72.0);
        Row::new().show(ui, |ui, _| {
            Field::new(tl!(unit.label())).show(ui, |ui, _| {
                NumberField::length(tl!(unit.label()), Bind::bound(&mut typed), lengths).show(ui);
            });
            // The button's name carries the row's unit ("Copy Millimeters").
            let name = format!("{} {}", tl!("Copy"), tl!(unit.label()));
            if IconButton::new(IconId::new("copy")).label(name).enabled(conv.points.is_some()).show(ui).clicks > 0 {
                copy = conv.copied(unit);
            }
        });
    }
    conv.typed(typed);

    Checkbox::new(Bind::bound(&mut conv.with_units)).label(tl!("Copy with Units")).show(ui);

    let mut side = conv.side;
    let (mut read, mut set) = (false, false);
    crate::panels::section(tl!("Selection")).show(ui, |ui, _| {
        RadioGroup::new(tl!("Side"), Bind::bound(&mut side)).nav(NavShape::Spatial).show(ui, |ui, _| {
            Radio::new(Side::Width).label(tl!("Width")).show(ui);
            Radio::new(Side::Height).label(tl!("Height")).show(ui);
        });
        Row::new().show(ui, |ui, _| {
            read = Button::new(tl!("From Selection")).enabled(selected).show(ui).clicks > 0;
            set = Button::new(tl!("Apply")).enabled(selected && conv.points.is_some()).show(ui).clicks > 0;
        });
    });
    if side != conv.side {
        conv.pick(side);
    }

    let mut failure = None;
    if let Some(text) = copy
        && let Err(e) = app.clipboard.write_text(&text)
    {
        failure = Some(e.to_string());
    }
    if read && let Err(e) = from_selection(app, &mut conv) {
        failure = Some(e);
    }
    if set && let Err(e) = apply(app, &conv) {
        failure = Some(e);
    }
    app.addon_panels.units = conv;
    if let Some(e) = failure {
        app.status(e);
    }
}

#[cfg(test)]
mod tests;
