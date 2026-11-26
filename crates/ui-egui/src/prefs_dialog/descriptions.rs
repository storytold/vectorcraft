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
    ("showToolCards", "Shows a card with what a toolbar tool does and what its keys do instead of its tip; it needs Show Tool Tips."),
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
    (
        "displayPrintSize",
        "At 100% zoom, one inch of the document fills an inch of the display, or 96 screen points where the system reports no usable display size.",
    ),
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
    ("interpolateControlPoints", "When you move anchor points, each curve with one moved end keeps its shape relative to its ends."),
    ("deleteKeepsPathsJoined", "Delete removes the selected anchor points and refits the curve around each one, instead of opening the path there."),
    ("doubleClickAddsAnchorPoints", "Double-clicking a path segment with the Direct Selection tool adds an anchor point there."),
    ("holdToolKeys", "Holding a tool's shortcut key chooses the tool until you let go, and then the previous tool is chosen again."),
    ("holdShortcuts", "Holding a key runs the tool or command in its Hold column in Keyboard Shortcuts; a quick press runs its usual shortcut."),
    ("keyHoldDelay", "How long, in seconds, a key must be held before its hold shortcut runs instead of its usual shortcut."),
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
    ("showOnShapeCreation", "Drawing a rectangle, ellipse, polygon, star or line opens the Transform panel if it is closed."),
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
    // Slices
    ("showSliceNumbers", "Shows each slice's number in a box at its top left on the canvas; off, the slice outlines still show."),
    ("sliceLineColor", "The color of user and object slice outlines and their number boxes; auto slices use a dimmer shade of it."),
    // Hyphenation
    (
        "hyphenationLanguage",
        "New type takes this language while Document Setup's language is English: USA and [Normal Character Style] sets none; type already in a document keeps its own language.",
    ),
    (
        "hyphenationExceptions",
        "In paragraphs with Hyphenate on, a listed word breaks only at the hyphens typed in it, or nowhere when it has none, whatever its case; Korean words take no hyphen, and Indian-language words do not break inside a character cluster.",
    ),
    // Plug-ins & Scratch Disks
    (
        "pluginsFolder",
        "On the desktop, this folder's .wasm plug-ins load at startup and when OK sets a new folder, except in Safe Mode; one that asks for permissions loads only after you grant them. The web version loads none.",
    ),
    (
        "pluginInstructionBudget",
        "The larger budget raises each object filter run's base instructions fivefold and keeps the same allowance per input byte; heavy filters, live effects, exporters and importers keep their own limits.",
    ),
    ("scratchPrimary", "The first scratch disk choice: Startup, None or a folder path; VectorCraft stores it but does not use scratch disks yet."),
    ("scratchSecondary", "The second scratch disk choice: Startup, None or a folder path; VectorCraft stores it but does not use scratch disks yet."),
    // User Interface
    ("uiBrightness", "How light or dark the interface is."),
    ("uiColorScheme", "The set of colors the interface is drawn in."),
    ("uiAccent", "The color of controls that are on, chosen or focused; the menu is dimmed while the Color Scheme has an accent of its own."),
    (
        "canvasColor",
        "The color of the canvas around the artboards; Match User Interface Brightness takes it from Brightness. Outline view always shows it white.",
    ),
    (
        "autoCollapseIconPanels",
        "A panel opened from its icon closes when you press Esc or click outside it; off, it stays open until you click an icon.",
    ),
    ("toolGroupLabels", "Shows each tool group's name above its tools in the toolbar; off, the groups follow one another with nothing between them."),
    (
        "openDocumentsAsTabs",
        "From now on, documents you open or create open as tabs; off, each opens in a floating window. Documents already open stay where they are.",
    ),
    ("largeTabs", "Makes the document tabs taller and their names larger."),
    (
        "systemTitleBar",
        "From the next start, the system draws the window's title bar where VectorCraft would otherwise draw its own; on Linux and the BSDs, the VECTORCRAFT_TITLEBAR variable takes precedence.",
    ),
    (
        "appTitleBar",
        "From the next start, VectorCraft draws the window's title bar even where the desktop would draw it, unless System Title Bar is on or the VECTORCRAFT_TITLEBAR variable chooses it.",
    ),
    ("uiScaling", "Makes the interface larger or smaller; art on the canvas keeps its size at the same zoom."),
    ("scaleCursorWithUi", "The tool pointers VectorCraft draws grow and shrink with UI Scaling; off, they keep their size."),
    ("interfaceLanguage", "The language of the interface; Auto uses the system's language. The dialog switches language as soon as you choose one."),
    // Performance
    ("gpuPerformance", "Animated Zoom works only while this is on; off, Animated Zoom is dimmed and zooming does not animate."),
    (
        "animatedZoom",
        "The wheel's zoom glides in, and the Zoom tool zooms as you drag sideways or hold it down; off, the Zoom tool zooms to the area you drag across.",
    ),
    (
        "gpuPreference",
        "From the next start, the desktop version shows the canvas through this kind of graphics processor; the artwork is drawn on the CPU either way. Automatic prefers the integrated one on Windows and macOS. The web version does not use it.",
    ),
    (
        "lowLatencyDrawing",
        "From the next start, the window shows frames without waiting for the display to refresh, at most 240 a second; off, the display paces them. A present mode set at launch overrides it.",
    ),
    (
        "historyStates",
        "The most undo steps each document keeps; lowering it drops the oldest steps when you click OK. Steps can also go sooner when they use too much memory.",
    ),
    (
        "realTimeDrawing",
        "Shows the artwork changing while you drag; off, outlines follow the drag and the artwork changes when you release. A ruler guide always moves as you drag.",
    ),
    (
        "renderThreads",
        "How many worker threads draw the artwork; -1 uses one fewer than the cores VectorCraft may use, at most 8, and 0 uses none. The web version always uses none.",
    ),
    (
        "linuxDisplayServer",
        "From the next start on a Wayland desktop, X11 (XWayland) opens the window through XWayland; the other choices do so only when a pen is attached.",
    ),
    (
        "monitorProfileDisplay",
        "Converts the artwork and artboards on the canvas to the display's color profile where the system does not already do so; off, the canvas is drawn in sRGB.",
    ),
    // File Handling
    ("backgroundSave", "On the desktop, a save to a file is written in the background while you keep editing; off, you wait until it is written."),
    (
        "backgroundExport",
        "On the desktop, exports to files and packages are written in the background while you keep editing, except 3D exports; off, you wait for them.",
    ),
    (
        "autosaveRecovery",
        "Every few minutes, keeps a recovery copy of each document with unsaved changes made since its last copy; off, no timed copies are made.",
    ),
    ("autosaveInterval", "How many minutes pass between recovery copies while Automatically Save Recovery Data is on."),
    (
        "recoveryFolder",
        "On the desktop, the folder recovery copies are kept in; leave it blank for VectorCraft's own folder. The web version keeps them in the browser.",
    ),
    ("recoveryOffForComplex", "No timed recovery copies are made of documents with more than 20,000 objects."),
    (
        "recentFilesCount",
        "Open Recent Files and the Home screen list at most this many recent files; 0 lists none, and VectorCraft still remembers up to 30.",
    ),
    (
        "lowResProxyEps",
        "When a document opens, a linked EPS image that shows its saved preview keeps it instead of reading the EPS file again; changed files still follow Update Links.",
    ),
    ("pdfImportEmbeddedFonts", "Uses a PDF's embedded glyphs for imported text when its font is missing; off, it uses the fallback font."),
    (
        "antiAliasedBitmaps",
        "In Pixel Preview, images are drawn smoothed; off, their pixels show with hard edges. Outside Pixel Preview, images are always smoothed.",
    ),
    (
        "updateLinks",
        "When a linked file changes, Automatically reads it again, Ask When Modified asks you first, and Manually leaves it and shows a note in the status bar.",
    ),
    (
        "pdfTextAs",
        "The text of a PDF you open or embed comes in as editable type, or as outlines that look the same on a computer without its fonts.",
    ),
    (
        "templatesFolder",
        "On the desktop, the folder New from Template… and Save as Template… start in; leave it blank for VectorCraft Templates in your documents folder.",
    ),
    (
        "editOriginalSystemDefaults",
        "Edit Original opens a linked file in the system's default app for its type; off, a linked raster image opens in PhotoCraft where it is installed.",
    ),
    (
        "appendConverted",
        "A file saved by an older VectorCraft version opens with [Converted] after its name, and Save asks where to save it; off, Save replaces the file.",
    ),
    (
        "useCompression",
        "VectorCraft files and templates are saved compressed, so they are smaller, unless a save says otherwise; saves for versions that cannot open compressed files stay uncompressed.",
    ),
    // Clipboard Handling
    (
        "copyAsSvg",
        "Copy and Cut also give other apps SVG code of the art; on the desktop, the code is not copied as text when the art embeds more than 1 MiB of images.",
    ),
    ("copyAsPdf", "On Windows, Copy and Cut also give other apps a PDF of the art; the other platforms do not copy a PDF yet."),
    ("copyAicb", "VectorCraft stores this choice, but Copy and Cut do not use a legacy vector clipboard format yet."),
    (
        "aicbMode",
        "Preserve Paths or Preserve Appearance and Overprints for the legacy vector clipboard; VectorCraft stores the choice but does not use it yet.",
    ),
    (
        "pasteTextFormatting",
        "Keep Formatting keeps the styles of text copied from VectorCraft type when it is pasted into type; Keep Plain Text gives it the style where it lands.",
    ),
    // Appearance of Black
    (
        "blackOnScreen",
        "Display All Blacks as Rich Black shows pure CMYK black (black ink alone at 100%) as RGB black on the canvas; Display All Blacks Accurately converts it through the CMYK working space.",
    ),
    (
        "blackOutput",
        "Output All Blacks as Rich Black draws pure CMYK black as RGB black in RGB and grayscale raster exports and Print as Bitmap; it does not change vector exports, CMYK JPEG, TIFF and PSD, or other printing.",
    ),
    // Devices
    (
        "touchWorkspace",
        "On a touch screen, switches to the Touch workspace; turning this off, or losing the touch screen, goes back to the workspace it switched from.",
    ),
    (
        "touchGestures",
        "A two-finger tap on the canvas undoes, or switches between selection and drawing tools in the Touch workspace, and a three-finger tap redoes; neither works in Presentation Mode.",
    ),
];

/// The sentence that describes the setting `key`, in English, or `None` when it has none yet.
pub(crate) fn description(key: &str) -> Option<&'static str> {
    DESCRIPTIONS.iter().find(|(k, _)| *k == key).map(|(_, s)| *s)
}
