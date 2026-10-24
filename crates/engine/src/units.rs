//! Units (Preferences ▸ Units): the unit every length the UI shows and reads is in.
//!
//! General (rulers, positions, sizes, dialog distances, canvas readouts) is the active document's
//! units, so they travel with the file: Document Setup (`document.setUnits`) changes them, and
//! Preferences ▸ Units ▸ General changes them too and is also the units new documents start in (and
//! the unit shown while no document is open). Stroke (weights, dashes) and Type (sizes, leading,
//! baseline shift) are application preferences. Typed values with a unit ("5 mm", "12 pt") work in
//! any unit.

use vectorcraft_doc::Unit;

use crate::{Result, Session};

/// Which Units preference a length follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Measure {
    General,
    Stroke,
    Type,
}

impl Measure {
    /// Its name in `prefs.list` (`measure`).
    pub fn name(self) -> &'static str {
        match self {
            Measure::General => "general",
            Measure::Stroke => "stroke",
            Measure::Type => "type",
        }
    }

    /// The preference that picks its unit.
    pub fn pref_key(self) -> &'static str {
        match self {
            Measure::General => "unitsGeneral",
            Measure::Stroke => "unitsStroke",
            Measure::Type => "unitsType",
        }
    }
}

/// The unit a Units preference value names (points when it names none).
fn pref_unit(value: &str) -> Unit {
    Unit::named(value).unwrap_or_default()
}

impl Session {
    /// The unit lengths of kind `m` show in and bare numbers typed for them are read in.
    pub fn unit(&self, m: Measure) -> Unit {
        match m {
            Measure::General => self.active().map_or_else(|| self.default_units(), |d| d.doc.units),
            Measure::Stroke => pref_unit(&self.prefs.units_stroke),
            Measure::Type => pref_unit(&self.prefs.units_type),
        }
    }

    /// The General unit: the active document's units (Preferences ▸ Units ▸ General without one).
    pub fn general_unit(&self) -> Unit {
        self.unit(Measure::General)
    }

    /// Units ▸ Type: the unit type sizes, leading and baseline shift show in.
    pub fn type_unit(&self) -> Unit {
        self.unit(Measure::Type)
    }

    /// The unit type sizes and leading show in: [`Self::type_unit`] or, when
    /// [`Prefs::show_east_asian_options`] is on and `east_asian` is true, Units ▸ East Asian Type.
    pub fn type_unit_for(&self, east_asian: bool) -> Unit {
        if east_asian && self.prefs.show_east_asian_options { pref_unit(&self.prefs.units_asian_type) } else { self.type_unit() }
    }

    /// Preferences ▸ Units ▸ General: the units new documents start in.
    pub fn default_units(&self) -> Unit {
        pref_unit(&self.prefs.units_general)
    }

    /// Set the active document's units (one undo step, as Document Setup does; none when they
    /// already are `u`).
    pub(crate) fn set_document_units(&mut self, u: Unit) -> Result<()> {
        if self.doc()?.doc.units == u {
            return Ok(());
        }
        self.edit("Document Setup", |d, _| {
            d.units = u;
            Ok(())
        })
    }
}
