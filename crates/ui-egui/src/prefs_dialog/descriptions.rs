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
    ("ctrlWheelScrollsSideways", "While Zoom with Mouse Wheel is off, the mouse wheel with Cmd/Ctrl held scrolls the canvas sideways."),
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
    // Type
    (
        "typeSizeIncrement",
        "The step for type size and leading: Cmd/Ctrl+Shift+. and Cmd/Ctrl+Shift+, change the size by one step, or five with Alt too; in horizontal type, Alt with the Up or Down Arrow changes the leading by one step, or five with Cmd/Ctrl.",
    ),
    (
        "trackingIncrement",
        "The step for tracking and kerning, in thousandths of an em: in horizontal type, Alt with the Left or Right Arrow changes the kerning at the caret or the tracking of the selected text by one step, or five with Cmd/Ctrl.",
    ),
    (
        "baselineShiftIncrement",
        "The step for baseline shift: in horizontal type, Alt and Shift with the Up or Down Arrow move the baseline by one step, or five with Cmd/Ctrl.",
    ),
    (
        "showEastAsianOptions",
        "Shows the options for Japanese and other East Asian type in the Character, Paragraph, OpenType and Glyphs panels, and Type › Composite Fonts.",
    ),
    (
        "showIndicOptions",
        "Shows the options for Arabic, Hebrew and Indic type in the Character and Paragraph panels, such as paragraph direction, kashidas and digits.",
    ),
    ("typeSelectionByPathOnly", "The selection tools select type only on its baseline, its area's frame or its path, not anywhere inside its box."),
    ("fontNamesInEnglish", "Font menus list each font by its English name; off, a font that also has a name in its own language shows that name."),
    (
        "autoSizeAreaType",
        "New area type starts with Auto Size on, so a rectangular frame of horizontal type grows or shrinks in height to fit its text.",
    ),
    ("fontPreview", "Font lists and the Type › Font menu show a sample of each font beside its name."),
    (
        "fontSamplesOnWhite",
        "Font samples are drawn in dark type on white, whatever the interface's brightness; it applies only while in-menu font previews are on.",
    ),
    ("fontPreviewSize", "How tall each row is in the font lists of font fields, such as the Character panel's, and so how large each sample is."),
    ("recentFontsCount", "How many of the fonts used last, from 1 to 15, Type › Recent Fonts and the font lists show, newest first."),
    ("missingGlyphProtection", "When you change the font or font style of text, characters the new font has no glyph for keep the font they had."),
    (
        "highlightAlternateGlyphs",
        "A pink highlight on the canvas marks glyphs that OpenType features replaced, such as ligatures; it does not print or export.",
    ),
    (
        "showCharacterAlternates",
        "With the Type tool, selecting one character shows a row of its alternate glyphs beside it on the canvas to choose from.",
    ),
    (
        "highlightSubstitutedFonts",
        "A pink highlight on the canvas marks text drawn in a substitute font because its font is missing; it does not print or export.",
    ),
    ("placeholderText", "New type made with a Type tool starts with placeholder text, selected, so typing replaces it."),
    ("typedLists", "Typing a list marker such as 1. or * and then a space at the start of a paragraph makes the paragraph a list."),
    ("fontsFolder", "Fonts in this folder show in the font menus and can be used without being installed."),
    (
        "overrideCharColor",
        "Applying a graphic style to type removes the characters' own fill or stroke where the style has one, so the style's paint shows.",
    ),
    ("alignGlyphBoundsPoint", "Align and Distribute measure point type by the ink of its glyphs instead of its em box."),
    ("alignGlyphBoundsArea", "Align and Distribute measure area type by the ink of its glyphs instead of its frame."),
    ("typeGlyphBounds", "The bounding box and the Transform panel measure point type by the ink of its glyphs instead of its em box."),
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
