//! The sentence the Preferences dialog shows under its fields for the setting under the pointer,
//! or for the setting whose control has the keyboard focus, by the setting's key, in English.
//! Every sentence has a row in each complete catalog.

/// (preference key, sentence), in the order the dialog lists the settings.
pub(crate) const DESCRIPTIONS: &[(&str, &str)] = &[
    // Units
    (
        "unitsGeneral",
        "The unit for new documents from built-in sizes other than pixel sizes and for opened files that record no unit; choosing one here and clicking OK also sets the active document's units.",
    ),
    ("unitsStroke", "Stroke weights, dashes and gaps show in this unit, and numbers typed for them are read in it."),
    ("unitsType", "Type sizes, leading and baseline shift show in this unit, and numbers typed for them are read in it."),
    ("unitsAsianType", "The unit for East Asian type measurements, which no field uses yet."),
    ("numbersWithoutUnitsArePoints", "When on, a number typed without a unit into a field in picas is read in points."),
    ("identifyObjectsBy", "With XML ID, object names show as XML IDs in the Layers panel, the Variables panel and Layer Options."),
];

/// The sentence that describes the setting `key`, in English, or `None` when it has none yet.
pub(crate) fn description(key: &str) -> Option<&'static str> {
    DESCRIPTIONS.iter().find(|(k, _)| *k == key).map(|(_, s)| *s)
}
