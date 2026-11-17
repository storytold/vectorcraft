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
    // Selection & Anchor Display
    ("selectionTolerance", "How near, in pixels, the pointer must be to an object, anchor or handle for a click to select it."),
    (
        "marqueeSelection",
        "Whether the Selection tool's box selects objects it touches or only those fully inside; Drag Direction selects those inside on a drag to the right. E reverses it mid-drag.",
    ),
    ("objectSelectionByPathOnly", "A click selects a filled object only on its path, not inside its fill."),
    (
        "snapToPointTolerance",
        "How near, in pixels, a point must come to an anchor or ruler guide for Snap to Point to snap it; with Smart Guides on, Smart Guides' Snapping Tolerance applies instead.",
    ),
    (
        "ctrlClickSelectsBehind",
        "With the Selection tool, Cmd/Ctrl-click selects the object under the selected one at the pointer; each further click goes one object deeper.",
    ),
    ("zoomToSelection", "Zoom In and Zoom Out bring the center of the selection to the middle of the view."),
    (
        "moveLockedWithArtboard",
        "Locked and hidden artwork inside an artboard moves, scales and is copied with the artboard, as the other artwork does.",
    ),
    ("anchorSize", "The size of anchor points and direction handle ends on the canvas; larger anchors can also be picked from farther away."),
    ("handleStyle", "Draws the ends of direction handles as solid dots, hollow dots or larger solid dots."),
    ("highlightAnchorsOnHover", "With the Direct Selection tool, the anchor under the pointer is highlighted."),
    (
        "showHandlesMultipleAnchors",
        "Shows direction handles when more than one anchor is selected; off, they show only while a single anchor is selected.",
    ),
    ("hideCornerWidgetAbove", "Corner widgets hide on corners whose angle is greater than this many degrees."),
    ("penRubberBand", "The Pen tool previews the next segment from the last anchor to the pointer before you click."),
    ("curvatureRubberBand", "The Curvature tool previews the curve to the pointer before you place the next point."),
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
    // Guides & Grid
    (
        "__snapToGrid",
        "Snaps drawing, moving and scaling to the grid's subdivisions, even while the grid is hidden; while it is on, Snap to Pixel does not apply.",
    ),
    ("guideColor", "The color of guides that are not selected; an artboard with a Guide Label draws its own guides in the label's color instead."),
    ("guideStyle", "Draws guides as solid lines or as dots, including the guides already in the document."),
    (
        "lockGuidesByDefault",
        "With this on, guides are locked when VectorCraft starts; changing this option locks or unlocks them at once, and View › Guides › Lock Guides still changes the lock.",
    ),
    (
        "cmdRulerDragArtboardGuides",
        "With this on, holding Cmd/Ctrl as you start dragging from a ruler makes a guide on the active artboard only, or a canvas guide with the Artboard tool.",
    ),
    ("gridColor", "The color of the grid; the lines between major gridlines are drawn in a fainter shade of it."),
    ("gridStyle", "Draws the grid as lines, or as dots along the major gridlines at each subdivision."),
    (
        "gridlineEvery",
        "The distance between major gridlines, in every open document and in new ones; a VectorCraft document opens with the grid it was saved with.",
    ),
    (
        "gridSubdivisions",
        "How many parts each major grid square is divided into; Snap to Grid snaps to these parts, even when the zoom hides their lines.",
    ),
    ("gridsInBack", "Draws the grid behind the artwork; off, the grid is drawn over it."),
    ("showPixelGrid", "In Pixel Preview at 600% zoom or more, the active view shows a line at the edge of every document pixel over the artwork."),
    ("patternTileEdgeColor", "The color of the tile edge and the dashed swatch bounds shown while you edit a pattern."),
    // Smart Guides
    (
        "__smartGuides",
        "Snaps drawing, moving and transforming to nearby art and shows guides and labels; Pixel Preview turns them off, and Snap to Grid snaps to the grid instead.",
    ),
    ("smartGuideColor", "The color of Smart Guide lines and labels."),
    (
        "alignmentGuides",
        "Shows a line along the edge or center that art lines up with; off, art still lines up without the line, and Snap to Glyph does not apply.",
    ),
    ("objectHighlighting", "With a selection tool, outlines the unselected object under the pointer in its layer color while you are not dragging."),
    ("transformToolsGuides", "Shows the size or angle next to the pointer while you scale, rotate, shear or reflect art."),
    ("constructionGuides", "While you draw, points snap to lines at the chosen Angles through the point the segment or shape starts from."),
    ("constructionAngles", "The set of angles the construction guides follow while Construction Guides is on."),
    ("anchorPathLabels", "Names what a point snaps to, such as an anchor, a path or an alignment; off, points snap the same way without the names."),
    ("measurementLabels", "Shows distances, angles and how far art has moved next to the pointer while you draw or move it."),
    ("spacingGuides", "Snaps moved or resized art to the same gap as between nearby objects, and marks the equal gaps."),
    (
        "distanceGuides",
        "With something selected, holding Alt/Option over another object or an artboard shows the distances between it and the selection.",
    ),
    (
        "snapToLastLocation",
        "A selection dragged back to within the Snapping Tolerance of where the drag began goes back to exactly that place; a copy does not.",
    ),
    (
        "snappingTolerance",
        "How near on screen the pointer or a dragged point must come to art for Smart Guides to snap to it; the distance is the same at every zoom.",
    ),
    ("snapTangent", "Snaps the end of a line segment, or a circle drawn from its center, so that it touches a nearby curve."),
    ("snapPerpendicular", "Snaps the end of a line segment so that it meets a nearby straight segment at a right angle."),
];

/// The sentence that describes the setting `key`, in English, or `None` when it has none yet.
pub(crate) fn description(key: &str) -> Option<&'static str> {
    DESCRIPTIONS.iter().find(|(k, _)| *k == key).map(|(_, s)| *s)
}
