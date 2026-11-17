//! The sentence the Preferences dialog shows under its fields for the setting under the pointer,
//! or for the setting whose control has the keyboard focus, by the setting's key, in English.
//! Every sentence has a row in each complete catalog.

/// (preference key, sentence), in the order the dialog lists the settings.
pub(crate) const DESCRIPTIONS: &[(&str, &str)] = &[
    // General
    ("__taskBar", "Shows a bar of commands for the selection or tool on the canvas; it hides while you drag and in Presentation Mode."),
    ("keyboardIncrement", "How far an arrow key moves the selection; with Shift, it moves ten times as far."),
    ("constrainAngle", "The angle of the axes that arrow keys move along and that Shift holds drawing and moving to."),
    ("cornerRadius", "The corner radius the Rounded Rectangle tool draws with; changing the radius while drawing changes this value too."),
    (
        "pasteOffset",
        "How far right and down Paste moves the copy when it is given no position, as from a script; Paste in the app centers it in the view.",
    ),
    ("disableAutoAddDelete", "Stops the Pen tool from deleting the anchor you click on a selected path, or adding one where you click its segment."),
    ("usePreciseCursors", "Drawing tools show a crosshair pointer instead of their icon; Caps Lock does the same while it is on."),
    ("showToolTips", "Shows a tip when the pointer rests on a button or field that has one."),
    ("showExternalScriptWarning", "Shows a warning before a script you choose runs."),
    ("antiAliasedArtwork", "Smooths the edges of artwork on the canvas."),
    (
        "selectSameTintPercent",
        "Select › Same › Fill Color, Stroke Color and Fill & Stroke match only the same tint of a global or spot swatch, not any tint of it.",
    ),
    ("showHomeScreen", "Shows the Home screen when no document is open; off, the window stays empty until you click Home."),
    (
        "usePreviewBounds",
        "Measures objects with their strokes and shape-changing effects for size, position and alignment; shadows, glows and blurs are not measured.",
    ),
    ("displayPrintSize", "At 100% zoom, one inch of the document takes 96 screen points instead of 72."),
    ("doubleClickToIsolate", "Double-clicking a group, path or image with the Selection tool opens it in isolation mode."),
    (
        "transformPatternTiles",
        "Pattern fills and strokes transform with the art, unless a transform or the Transform panel's menu chooses otherwise.",
    ),
    ("scaleCorners", "Live corner radii scale with the art, unless the scale chooses otherwise."),
    ("scaleStrokes", "Stroke weights, dashes and effect distances scale with the art, unless the scale chooses otherwise."),
    ("zoomWithMouseWheel", "The mouse wheel zooms the canvas instead of scrolling it."),
    ("ctrlWheelScrollsSideways", "The mouse wheel with Cmd/Ctrl held scrolls the canvas sideways."),
    ("scrubNumericFields", "Dragging sideways on a numeric field's label changes the field's value."),
    ("trackpadRotateView", "A two-finger rotate gesture on a trackpad rotates the canvas view."),
    ("holdToolKeys", "Holding a tool's shortcut key chooses the tool until you let go, and then the previous tool is chosen again."),
    ("boundingBoxWhileMovingAndDrawing", "Shows the bounding box while you move the selection or draw a shape."),
    ("japaneseCropMarks", "Create Trim Marks draws Japanese-style marks instead of Roman ones."),
    ("newArtBasic", "New art takes only the fill and stroke of the selection; off, it takes the selected object's whole appearance."),
    (
        "constrainProportions",
        "Changing the width or height in the Transform panel, the Properties panel or the Control bar changes the other in proportion.",
    ),
    ("singleCornerRadius", "A live rectangle's properties show one Corner Radius field for all four corners; off, each corner has its own field."),
    (
        "transformPanelPatterns",
        "What edits in the Transform panel transform: the object, its pattern tiles or both; Default follows Transform Pattern Tiles.",
    ),
    (
        "useRegistrationPointForSymbol",
        "With one symbol instance selected, the Transform panel uses the symbol's registration point as its reference point.",
    ),
    ("fadeRegionOutsideArtboard", "Darkens the canvas outside the artboards while the Artboard tool is active."),
    (
        "artboardUpdateWhileDragging",
        "With Fade Region Outside Artboard on, the area outside the artboards stays dark while you drag with the Artboard tool.",
    ),
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
