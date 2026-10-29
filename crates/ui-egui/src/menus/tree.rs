use super::slots::{PERSPECTIVE_SLOTS, RECENT_FONT_IDS, RECENT_IDS, SAVED_SELECTION_IDS, SAVED_VIEW_IDS};
use super::submenus::{INSERT_BREAK, INSERT_SPECIAL, INSERT_WHITESPACE, PATH_EFFECTS, TYPE_SIZES, effect_menu, font_items, insert_items};
use super::{Item, Sep, c, cp, json, sub, todo, todos};

/// The in-window and native menu tree. The in-window bar skips the VectorCraft menu (see
/// `menu_bar`); the native Mac bar is composed from the whole tree (see `native_menu`).
/// `english_names` is Preferences › Type › Show Font Names in English (the Type → Font labels
/// follow it; tests use the default `true`).
pub fn menu_tree() -> Vec<(&'static str, Vec<Item>)> {
    menu_tree_named(true)
}

pub fn menu_tree_named(english_names: bool) -> Vec<(&'static str, Vec<Item>)> {
    let panel = |label: &'static str, id: &'static str| cp(label, "window.panel", json!({ "panel": id, "toggle": true }));
    vec![
        (
            // The application menu: About and Discord only. The in-window bar never shows it (File
            // is first there, as in Illustrator); on macOS `native_menu::mac_layout` composes the
            // App menu from this entry and the other menus (About from Help, Settings from Edit,
            // Quit from File, Language and Appearance synthesized). It stays first: `from_tree`
            // gives the first menu the App role.
            "VectorCraft",
            vec![c("About VectorCraft", "help.about"), c("Join Our Discord", "help.discord")],
        ),
        (
            "File",
            vec![
                c("New…", "file.newDialog"),
                c("New from Template…", "file.newFromTemplate"),
                c("Open…", "file.open"),
                sub("Open Recent Files", {
                    let mut v: Vec<Item> = RECENT_IDS.iter().map(|id| c("Recent File", id)).collect();
                    v.push(Sep);
                    v.push(c("Clear Recent Files", "file.clearRecent"));
                    v
                }),
                c("Show in Folder", "file.reveal"),
                Sep,
                c("Close", "file.close"),
                c("Close All", "file.closeAll"),
                c("Save", "file.save"),
                c("Save As…", "file.saveAs"),
                c("Save a Copy…", "file.saveCopy"),
                c("Save as Template…", "file.saveAsTemplate"),
                c("Save as PDF…", "file.export.pdf"),
                c("Save for Office Documents…", "document.exportForOffice"),
                c("Revert", "file.revert"),
                Sep,
                c("Place…", "file.place"),
                Sep,
                sub(
                    "Export",
                    vec![
                        c("Export for Screens…", "file.exportForScreens"),
                        c("Export As…", "file.exportAs"),
                        c("Export As SVG…", "file.export.svg"),
                        c("Export As PNG…", "file.export.png"),
                        c("Save for Web (Legacy)…", "file.saveForWeb"),
                    ],
                ),
                c("Export Selection…", "file.exportSelection"),
                Sep,
                c("Package…", "ui.packageDialog"),
                sub("Scripts", vec![c("Check Script…", "ui.checkScript"), todos("Other Script…", "Cmd+F12")]),
                Sep,
                c("Document Setup…", "file.documentSetup"),
                sub(
                    "Document Color Mode",
                    vec![
                        cp("CMYK Color", "file.documentColorMode", json!({"mode": "cmyk"})),
                        cp("RGB Color", "file.documentColorMode", json!({"mode": "rgb"})),
                    ],
                ),
                c("File Info…", "file.info"),
                Sep,
                c("Print…", "file.print"),
                Sep,
                c("Exit", "app.quit"),
            ],
        ),
        (
            "Edit",
            vec![
                c("Undo", "edit.undo"),
                c("Redo", "edit.redo"),
                Sep,
                c("Cut", "edit.cut"),
                c("Copy", "edit.copy"),
                c("Paste", "edit.paste"),
                c("Paste in Front", "edit.pasteInFront"),
                c("Paste in Back", "edit.pasteInBack"),
                c("Paste in Place", "edit.pasteInPlace"),
                c("Paste on All Artboards", "edit.pasteOnAllArtboards"),
                c("Paste without Formatting", "edit.pasteWithoutFormatting"),
                c("Clear", "edit.clear"),
                Sep,
                cp("Find and Replace…", "edit.findReplace", json!({"find": "", "replace": "", "matchCase": false, "wholeWord": false})),
                cp("Find Next", "edit.findNext", json!({"find": ""})),
                sub("Spelling", vec![todo("Auto Spell Check"), todos("Check Spelling…", "Cmd+I"), todo("Edit Custom Dictionary…")]),
                Sep,
                sub(
                    "Edit Colors",
                    vec![
                        c("Recolor Artwork…", "ui.recolorDialog"),
                        sub(
                            "Recolor with Preset",
                            vec![
                                cp("1 Color Job…", "ui.recolorDialog", json!({"colors": 1})),
                                cp("2 Color Job…", "ui.recolorDialog", json!({"colors": 2})),
                                cp("3 Color Job…", "ui.recolorDialog", json!({"colors": 3})),
                                cp("Color Library…", "ui.recolorDialog", json!({"library": ""})),
                            ],
                        ),
                        c("Adjust Color Balance…", "ui.colorBalanceDialog"),
                        c("Blend Front to Back", "edit.colors.blendFrontToBack"),
                        c("Blend Horizontally", "edit.colors.blendHorizontally"),
                        c("Blend Vertically", "edit.colors.blendVertically"),
                        c("Convert to CMYK", "edit.colors.toCMYK"),
                        c("Convert to Grayscale", "edit.colors.toGrayscale"),
                        c("Convert to RGB", "edit.colors.toRGB"),
                        c("Invert Colors", "edit.colors.invert"),
                        cp(
                            "Overprint Black…",
                            "edit.colors.overprintBlack",
                            json!({"remove": false, "percentage": 100, "fill": true, "stroke": true, "includeCmyBlacks": false, "includeSpotBlacks": false}),
                        ),
                        c("Saturate…", "ui.saturateDialog"),
                    ],
                ),
                c("Edit Original", "links.editOriginal"),
                Sep,
                c("Transparency Flattener Presets…", "ui.flattenerPresetsDialog"),
                c("Print Presets…", "ui.printPresetsDialog"),
                c("PDF Presets…", "ui.pdfPresetsDialog"),
                c("Perspective Grid Presets…", "ui.perspectivePresetsDialog"),
                Sep,
                c("Color Settings…", "edit.colorSettings"),
                c("Assign Profile…", "edit.assignProfile"),
                Sep,
                c("Keyboard Shortcuts…", "edit.keyboardShortcuts"),
                c("Preferences…", "edit.preferences"),
            ],
        ),
        (
            "Object",
            vec![
                sub("Transform", transform_items()),
                sub("Arrange", arrange_items()),
                sub(
                    "Align",
                    vec![
                        cp("Horizontal Align Left", "object.align", json!({"horizontal": "left"})),
                        cp("Horizontal Align Center", "object.align", json!({"horizontal": "center"})),
                        cp("Horizontal Align Right", "object.align", json!({"horizontal": "right"})),
                        Sep,
                        cp("Horizontal & Vertical Align Center", "object.align", json!({"horizontal": "center", "vertical": "center"})),
                        Sep,
                        cp("Vertical Align Top", "object.align", json!({"vertical": "top"})),
                        cp("Vertical Align Center", "object.align", json!({"vertical": "center"})),
                        cp("Vertical Align Bottom", "object.align", json!({"vertical": "bottom"})),
                    ],
                ),
                Sep,
                c("Group", "object.group"),
                c("Ungroup", "object.ungroup"),
                sub(
                    "Lock",
                    vec![c("Selection", "object.lock"), c("All Artwork Above", "object.lock.above"), c("Other Layers", "object.lock.otherLayers")],
                ),
                c("Unlock All", "object.unlockAll"),
                sub(
                    "Hide",
                    vec![
                        c("Selection", "object.hide"),
                        c("All Artwork Above", "object.hide.above"),
                        c("Other Layers", "object.hide.otherLayers"),
                        c("Unselected Artwork", "object.hide.unselected"),
                    ],
                ),
                c("Show All", "object.showAll"),
                Sep,
                c("Expand…", "ui.expandDialog"),
                c("Expand Appearance", "effect.expandAppearance"),
                c("Crop Image", "ui.cropImage"),
                c("Rasterize…", "object.rasterize"),
                cp("Create Gradient Mesh…", "object.mesh.create", json!({"rows": 4, "cols": 4, "appearance": "flat", "highlight": 100})),
                cp(
                    "Create Object Mosaic…",
                    "object.createObjectMosaic",
                    json!({"columns": 10, "rows": 10, "spacingX": 0, "spacingY": 0, "gray": false, "deleteRaster": false}),
                ),
                c("Vector Halftone…", "object.vectorHalftone"),
                c("Create Trim Marks", "object.createTrimMarks"),
                c("Flatten Transparency…", "ui.flattenTransparencyDialog"),
                Sep,
                c("Make Pixel Perfect", "object.makePixelPerfect"),
                Sep,
                sub(
                    "Slice",
                    vec![
                        c("Make", "object.slice.make"),
                        c("Release", "object.slice.release"),
                        c("Create from Guides", "object.slice.fromGuides"),
                        c("Create from Selection", "object.slice.fromSelection"),
                        Sep,
                        c("Duplicate Slice", "object.slice.duplicate"),
                        c("Combine Slices", "object.slice.combine"),
                        c("Divide Slices…", "object.slice.divide"),
                        Sep,
                        c("Delete All", "object.slice.deleteAll"),
                        c("Slice Options…", "object.slice.options"),
                        c("Clip to Artboard", "object.slice.clipToArtboard"),
                    ],
                ),
                Sep,
                sub(
                    "Path",
                    vec![
                        c("Join", "path.join"),
                        c("Average…", "path.average"),
                        Sep,
                        c("Outline Stroke", "object.path.outlineStroke"),
                        c("Offset Path…", "object.path.offsetPath"),
                        c("Reverse Path Direction", "path.reverse"),
                        Sep,
                        c("Simplify…", "object.path.simplify"),
                        c("Smooth…", "object.path.smooth"),
                        c("Add Anchor Points", "object.path.addAnchorPoints"),
                        c("Remove Anchor Points", "path.removeAnchors"),
                        c("Divide Objects Below", "object.path.divideObjectsBelow"),
                        c("Split Into Grid…", "object.path.splitIntoGrid"),
                        Sep,
                        cp("Clean Up…", "object.path.cleanUp", json!({"strayPoints": true, "unpaintedObjects": true, "emptyTextPaths": true})),
                    ],
                ),
                sub("Shape", vec![c("Convert to Shape", "object.shape.convertToShape"), c("Expand Shape", "object.expandShape")]),
                sub(
                    "Pattern",
                    vec![c("Make", "object.pattern.make"), c("Edit Pattern", "object.pattern.edit"), c("Tile Edge Color…", "ui.tileEdgeColor")],
                ),
                sub(
                    "Repeat",
                    vec![
                        c("Radial", "object.repeat.radial"),
                        c("Grid", "object.repeat.grid"),
                        c("Mirror", "object.repeat.mirror"),
                        Sep,
                        c("Release", "object.repeat.release"),
                        c("Options…", "object.repeat.options"),
                    ],
                ),
                sub(
                    "Intertwine",
                    vec![c("Make", "object.intertwine.make"), c("Edit", "object.intertwine.edit"), c("Release", "object.intertwine.release")],
                ),
                sub("Objects on Path", vec![c("Attach", "object.objectsOnPath.attach"), c("Expand", "object.objectsOnPath.expand")]),
                sub(
                    "Blend",
                    vec![
                        c("Make", "object.blend.make"),
                        c("Release", "object.blend.release"),
                        Sep,
                        c("Blend Options…", "object.blend.options"),
                        Sep,
                        c("Expand", "object.blend.expand"),
                        Sep,
                        c("Replace Spine", "object.blend.replaceSpine"),
                        c("Reverse Spine", "object.blend.reverseSpine"),
                        c("Reverse Front to Back", "object.blend.reverseFrontToBack"),
                    ],
                ),
                sub(
                    "Envelope Distort",
                    vec![
                        // While an envelope is selected, Reset takes Make's place (`swapped_out`).
                        c("Make with Warp…", "object.envelope.makeWithWarp"),
                        c("Reset with Warp…", "object.envelope.resetWithWarp"),
                        c("Make with Mesh…", "object.envelope.makeWithMesh"),
                        c("Reset with Mesh…", "object.envelope.resetWithMesh"),
                        c("Make with Top Object", "object.envelope.makeWithTopObject"),
                        Sep,
                        c("Release", "object.envelope.release"),
                        c("Envelope Options…", "object.envelope.options"),
                        c("Expand", "object.envelope.expand"),
                        Sep,
                        c("Edit Contents", "object.envelope.editContents"),
                    ],
                ),
                sub(
                    "Perspective",
                    vec![
                        c("Attach to Active Plane", "perspective.attach"),
                        c("Release with Perspective", "perspective.release"),
                        c("Move Plane to Match Object", "perspective.plane.matchObject"),
                        c("Edit Text", "perspective.editText"),
                    ],
                ),
                sub(
                    "Live Paint",
                    vec![
                        c("Make", "livePaint.make"),
                        c("Merge", "livePaint.merge"),
                        c("Release", "livePaint.release"),
                        Sep,
                        c("Gap Options…", "ui.livePaintGapOptions"),
                        Sep,
                        c("Expand", "livePaint.expand"),
                    ],
                ),
                sub(
                    "Image Trace",
                    vec![
                        // Traced at once with the Default preset, as the Control bar's Image Trace
                        // button does; other presets come from its arrow or the Image Trace panel
                        // (#543: the "…" form asked for a preset's name in a text box).
                        cp("Make", "imageTrace.make", json!({"preset": "Default"})),
                        cp("Make and Expand", "imageTrace.makeAndExpand", json!({"preset": "Default"})),
                        c("Release", "imageTrace.release"),
                        c("Expand", "imageTrace.expand"),
                    ],
                ),
                sub(
                    "Text Wrap",
                    vec![
                        c("Make", "object.textWrap.make"),
                        c("Release", "object.textWrap.release"),
                        c("Text Wrap Options…", "object.textWrap.options"),
                    ],
                ),
                Sep,
                sub(
                    "Clipping Mask",
                    vec![
                        c("Make", "object.clippingMask.make"),
                        c("Release", "object.clippingMask.release"),
                        c("Edit Contents", "object.clippingMask.editContents"),
                        c("Edit Clipping Path", "object.clippingMask.editMask"),
                    ],
                ),
                sub("Compound Path", vec![c("Make", "object.compoundPath.make"), c("Release", "object.compoundPath.release")]),
                sub(
                    "Artboards",
                    vec![
                        c("Convert to Artboards", "artboard.convertToArtboards"),
                        c("Rearrange All Artboards…", "artboard.rearrange"),
                        Sep,
                        c("Fit to Artwork Bounds", "artboard.fitToArt"),
                        c("Fit to Selected Art", "artboard.fitToSelection"),
                    ],
                ),
                sub(
                    "Graph",
                    vec![
                        c("Type…", "graph.setType"),
                        c("Data…", "graph.setData"),
                        c("Design…", "graph.design"),
                        c("Column…", "graph.column"),
                        c("Marker…", "graph.marker"),
                    ],
                ),
                sub(
                    "Collect for Export",
                    vec![
                        cp("As Single Asset", "assets.add", json!({"multiple": false})),
                        cp("As Multiple Assets", "assets.add", json!({"multiple": true})),
                    ],
                ),
                Sep,
                sub("Plug-ins", crate::dialogs::plugin::object_menu()),
            ],
        ),
        (
            "Type",
            vec![
                sub("Font", font_items(english_names)),
                sub("Recent Fonts", RECENT_FONT_IDS.iter().map(|id| c("Recent Font", id)).collect()),
                sub("Size", TYPE_SIZES.iter().map(|(l, n)| cp(l, "text.setStyle", json!({ "size": n }))).collect()),
                c("Bold", "type.bold"),
                c("Italic", "type.italic"),
                Sep,
                panel("Glyphs", "glyphs"),
                sub("Insert Special Character", insert_items(INSERT_SPECIAL)),
                sub("Insert Whitespace Character", insert_items(INSERT_WHITESPACE)),
                sub("Insert Break Character", insert_items(INSERT_BREAK)),
                Sep,
                c("Convert To Area Type", "type.convertToAreaType"),
                c("Convert To Point Type", "type.convertToPointType"),
                c("Area Type Options…", "text.areaOptions"),
                sub(
                    "Type on a Path",
                    PATH_EFFECTS
                        .iter()
                        .map(|&(label, effect)| cp(label, "type.pathOptions", json!({ "effect": effect })))
                        .chain([Sep, c("Type on a Path Options…", "type.pathOptions")])
                        .collect(),
                ),
                sub(
                    "Threaded Text",
                    vec![
                        c("Create", "text.thread.create"),
                        c("Release Selection", "text.thread.releaseSelection"),
                        c("Remove Threading", "text.thread.remove"),
                    ],
                ),
                c("Fit Headline", "text.fitHeadline"),
                Sep,
                c("Find Font…", "type.findFont"),
                sub(
                    "Change Case",
                    vec![
                        cp("UPPERCASE", "type.changeCase", json!({"case": "upper"})),
                        cp("lowercase", "type.changeCase", json!({"case": "lower"})),
                        cp("Title Case", "type.changeCase", json!({"case": "title"})),
                        cp("Sentence case", "type.changeCase", json!({"case": "sentence"})),
                    ],
                ),
                cp("Smart Punctuation…", "type.smartPunctuation", json!({"quotes": true, "dashes": true, "ellipsis": true, "scope": "selection"})),
                sub(
                    "Bullets and Numbering",
                    vec![
                        c("Apply Bullets", "type.applyBullets"),
                        c("Apply Numbers", "type.applyNumbers"),
                        c("Convert to Text", "type.convertListToText"),
                    ],
                ),
                Sep,
                c("Create Outlines", "type.createOutlines"),
                c("Scale Text Attributes", "type.scaleTextAttributes"),
                c("Optical Margin Alignment", "type.opticalMarginAlignment"),
                Sep,
                c("Fill with Placeholder Text", "type.fillPlaceholder"),
                c("Insert Inline Symbol", "text.insertInline"),
                Sep,
                c("Show Hidden Characters", "type.hiddenCharacters"),
                sub("Type Orientation", vec![c("Horizontal", "type.orientation.horizontal"), c("Vertical", "type.orientation.vertical")]),
            ],
        ),
        ("Select", {
            let mut v = vec![
                c("All", "select.all"),
                c("All on Active Artboard", "select.allOnArtboard"),
                c("Deselect", "select.none"),
                c("Reselect", "select.reselect"),
                Sep,
                c("Inverse", "select.inverse"),
                Sep,
                c("Next Object Above", "select.nextAbove"),
                c("Next Object Below", "select.nextBelow"),
                Sep,
                sub(
                    "Same",
                    vec![
                        c("Appearance", "select.same.appearance"),
                        c("Appearance Attribute", "select.same.appearanceAttribute"),
                        c("Blending Mode", "select.same.blendingMode"),
                        c("Fill & Stroke", "select.same.fillAndStroke"),
                        c("Fill Color", "select.same.fillColor"),
                        c("Opacity", "select.same.opacity"),
                        c("Stroke Color", "select.same.strokeColor"),
                        c("Stroke Weight", "select.same.strokeWeight"),
                        c("Graphic Style", "select.same.graphicStyle"),
                        c("Shape", "select.same.shapeType"),
                        c("Symbol Instance", "select.same.symbolInstance"),
                        Sep,
                        c("Font Family", "select.same.fontFamily"),
                        c("Font Family & Style", "select.same.fontFamilyStyle"),
                        c("Font Family, Style & Size", "select.same.fontFamilyStyleSize"),
                        c("Font Size", "select.same.fontSize"),
                        c("Text Fill Color", "select.same.textFillColor"),
                        c("Text Stroke Color", "select.same.textStrokeColor"),
                    ],
                ),
                sub(
                    "Object",
                    vec![
                        c("All on Same Layers", "select.object.allOnSameLayers"),
                        c("Direction Handles", "select.object.directionHandles"),
                        Sep,
                        c("Brush Strokes", "select.object.brushStrokes"),
                        c("Bristle Brush Strokes", "select.object.bristleBrushStrokes"),
                        c("Clipping Masks", "select.object.clippingMasks"),
                        c("Slices", "select.object.slices"),
                        c("Stray Points", "select.object.strayPoints"),
                        c("Open Paths", "select.object.openPaths"),
                        Sep,
                        c("All Text Objects", "select.object.textObjects"),
                        c("Point Text Objects", "select.object.pointText"),
                        c("Area Text Objects", "select.object.areaText"),
                    ],
                ),
                c("Start Global Edit", "select.globalEdit"),
                Sep,
                c("Save Selection…", "select.save"),
                c("Update Selection", "select.update"),
                c("Edit Selection…", "select.editSaved"),
                Sep,
            ];
            v.extend(SAVED_SELECTION_IDS.iter().map(|id| c("Saved Selection", id)));
            v
        }),
        ("Effect", effect_menu()),
        ("View", {
            let mut v = vec![
                c("Outline", "view.outline"),
                c("Overprint Preview", "view.overprintPreview"),
                c("Pixel Preview", "view.pixelPreview"),
                c("Trim View", "view.trimView"),
                c("Presentation Mode", "view.presentation"),
                sub(
                    "Screen Mode",
                    vec![
                        cp("Normal Screen Mode", "view.screenMode", json!({"mode": 0})),
                        cp("Full Screen Mode with Menu Bar", "view.screenMode", json!({"mode": 1})),
                        cp("Full Screen Mode", "view.screenMode", json!({"mode": 2})),
                    ],
                ),
                Sep,
                sub(
                    "Proof Setup",
                    vec![
                        cp("Working CMYK", "view.proofSetup", json!({"target": "workingCmyk", "proof": true})),
                        cp("Legacy Macintosh RGB (Gamma 1.8)", "view.proofSetup", json!({"target": "legacyMacRgb", "proof": true})),
                        cp("Internet Standard RGB (sRGB)", "view.proofSetup", json!({"target": "srgb", "proof": true})),
                        cp("Monitor RGB", "view.proofSetup", json!({"target": "monitorRgb", "proof": true})),
                        cp("Color blindness – Protanopia-type", "view.proofSetup", json!({"target": "protanopia", "proof": true})),
                        cp("Color blindness – Deuteranopia-type", "view.proofSetup", json!({"target": "deuteranopia", "proof": true})),
                        cp("Color blindness – Tritanopia-type", "view.proofSetup", json!({"target": "tritanopia", "proof": true})),
                        cp("Customize…", "window.panel", json!({"panel": "separations"})),
                    ],
                ),
                c("Proof Colors", "view.proofColors"),
                Sep,
                c("Zoom In", "view.zoomIn"),
                c("Zoom Out", "view.zoomOut"),
                c("Fit Artboard in Window", "view.fitArtboard"),
                c("Fit All in Window", "view.fitAll"),
                c("Actual Size", "view.actualSize"),
                Sep,
                sub("Rotate View", crate::state::ROTATE_VIEW_ANGLES.iter().map(|(l, a)| cp(l, "view.rotate", json!({ "angle": a }))).collect()),
                c("Rotate View to Selection", "view.rotateToSelection"),
                c("Reset Rotate View", "view.rotateReset"),
                Sep,
                c("Hide Edges", "view.edges"),
                c("Hide Artboards", "view.artboards"),
                c("Show Print Tiling", "view.printTiling"),
                c("Hide Slices", "view.slices.hide"),
                c("Lock Slices", "view.slices.lock"),
                c("Hide Template", "view.template"),
                Sep,
                sub(
                    "Rulers",
                    vec![
                        c("Show Rulers", "view.rulers"),
                        c("Change to Global Rulers", "view.globalRulers"),
                        c("Show Video Rulers", "view.videoRulers"),
                    ],
                ),
                c("Hide Bounding Box", "view.boundingBox"),
                c("Show Transparency Grid", "view.transparencyGrid"),
                c("Hide Text Threads", "view.textThreads"),
                c("Hide Gradient Annotator", "view.gradientAnnotator"),
                c("Hide Corner Widget", "view.cornerWidget"),
                c("Show Live Paint Gaps", "view.livePaintGaps"),
                Sep,
                sub(
                    "Guides",
                    vec![
                        c("Hide Guides", "view.guides"),
                        c("Lock Guides", "view.guides.lock"),
                        c("Make Guides", "view.guides.make"),
                        c("Release Guides", "view.guides.release"),
                        c("Clear Guides", "view.guides.clear"),
                    ],
                ),
                c("Smart Guides", "view.smartGuides"),
                sub(
                    "Perspective Grid",
                    vec![
                        c("Show Grid", "perspective.grid.show"),
                        c("Show Rulers", "perspective.grid.rulers"),
                        c("Snap to Grid", "perspective.grid.snap"),
                        c("Lock Grid", "perspective.grid.lock"),
                        c("Lock Station Point", "perspective.grid.lockStation"),
                        Sep,
                        c("Define Grid…", "ui.perspectiveGridDialog"),
                        sub("One Point Perspective", crate::dialogs::perspective_presets::menu(1, PERSPECTIVE_SLOTS[0])),
                        sub("Two Point Perspective", crate::dialogs::perspective_presets::menu(2, PERSPECTIVE_SLOTS[1])),
                        sub("Three Point Perspective", crate::dialogs::perspective_presets::menu(3, PERSPECTIVE_SLOTS[2])),
                        Sep,
                        c("Save Grid as Preset…", "ui.savePerspectivePreset"),
                    ],
                ),
                c("Show Grid", "view.grid"),
                c("Snap to Grid", "view.snapToGrid"),
                c("Snap to Pixel", "view.snapToPixel"),
                c("Snap to Point", "view.snapToPoint"),
                c("Snap to Glyph", "view.snapToGlyph"),
                Sep,
                c("New View…", "view.saved.new"),
                c("Edit Views…", "view.saved.edit"),
                Sep,
            ];
            v.extend(SAVED_VIEW_IDS.iter().map(|id| c("Saved View", id)));
            v
        }),
        ("Window", {
            let mut window_items = vec![
                c("New Window", "window.newWindow"),
                sub("Arrange", vec![todo("Cascade"), todo("Tile"), todo("Float in Window"), todo("Consolidate All Windows")]),
                sub("Workspace", crate::workspaces::menu_items()),
                Sep,
                c("Control", "window.control"),
                c("Contextual Task Bar", "window.taskBar"),
                c("Tools", "window.toolbar"),
                sub("Toolbars", vec![c("Advanced", "window.toolbarAdvanced"), c("Double Column", "window.toolbarColumns")]),
                Sep,
            ];
            let lang = crate::i18n::current();
            let mut panels = vec![
                panel("Actions", "actions"),
                panel("Align", "align"),
                panel("Appearance", "appearance"),
                panel("Artboards", "artboards"),
                panel("Asset Export", crate::panels::asset_export::ID),
                panel("Attributes", "attributes"),
                panel("Blend", crate::panels::blend::ID),
                panel("Brushes", "brushes"),
                panel("Color", "color"),
                panel("Color Guide", "colorGuide"),
                panel("Color Themes", "colorThemes"),
                panel("CSS Properties", crate::panels::css_properties::ID),
                panel("Document Info", "docInfo"),
                panel("Flattener Preview", crate::panels::flattener_preview::ID),
                panel("Gradient", "gradient"),
                panel("Graphic Styles", "graphicStyles"),
                panel("History", "history"),
                panel("Image Trace", "imageTrace"),
                panel("Info", "info"),
                panel("Layers", "layers"),
                panel("Libraries", "libraries"),
                panel("Links", crate::panels::links::ID),
                panel("Magic Wand", "magicWand"),
                panel("Navigator", "navigator"),
                panel("Pathfinder", "pathfinder"),
                panel("Pattern Options", "patternOptions"),
                panel("Properties", "properties"),
                panel("Separations Preview", "separations"),
                panel("Stroke", "stroke"),
                panel("SVG Interactivity", crate::panels::svg_interactivity::ID),
                panel("Swatches", "swatches"),
                panel("Symbols", "symbols"),
                panel("Transform", "transform"),
                panel("Transparency", "transparency"),
                sub("Type", {
                    let mut type_items = vec![
                        panel("Character", "character"),
                        panel("Character Styles", "charStyles"),
                        panel("Glyphs", "glyphs"),
                        panel("OpenType", "openType"),
                        panel("Paragraph", "paragraph"),
                        panel("Paragraph Styles", "paraStyles"),
                        panel("Tabs", "tabs"),
                    ];
                    sort_by_localized_label(&mut type_items, lang);
                    type_items
                }),
                panel("Variables", crate::panels::variables::ID),
            ];
            sort_by_localized_label(&mut panels, lang);
            window_items.extend(panels);
            window_items.push(Sep);
            let mut lib_items = vec![
                sub("Brush Libraries", crate::panels::brushes::window_menu()),
                sub("Graphic Style Libraries", crate::panels::graphic_styles::window_menu()),
                sub("Swatch Libraries", crate::panels::swatches::window_menu()),
                sub("Symbol Libraries", crate::panels::symbols::window_menu()),
            ];
            sort_by_localized_label(&mut lib_items, lang);
            window_items.extend(lib_items);
            window_items
        }),
        (
            "Help",
            vec![
                c("Join Our Discord", "help.discord"),
                c("ArtCraft Website", "help.website"),
                c("VectorCraft on getartcraft.com", "help.appPage"),
                c("VectorCraft on GitHub", "help.github"),
                Sep,
                c("Search Commands…", "help.commandPalette"),
                todos("VectorCraft Help…", "F1"),
                Sep,
                c("Check for Updates…", "help.checkForUpdates"),
                c("About VectorCraft", "help.about"),
            ],
        ),
    ]
}

/// Object → Transform (also in the canvas context menu).
pub(super) fn transform_items() -> Vec<Item> {
    vec![
        c("Transform Again", "object.transformAgain"),
        Sep,
        c("Move…", "object.move"),
        c("Rotate…", "object.rotate"),
        c("Reflect…", "object.reflect"),
        c("Scale…", "object.scale"),
        c("Shear…", "object.shear"),
        Sep,
        c("Transform Each…", "object.transformEach"),
        Sep,
        c("Reset Bounding Box", "object.resetBoundingBox"),
    ]
}

/// Object → Arrange (also in the canvas context menu).
pub(super) fn arrange_items() -> Vec<Item> {
    vec![
        c("Bring to Front", "object.arrange.bringToFront"),
        c("Bring Forward", "object.arrange.bringForward"),
        c("Send Backward", "object.arrange.sendBackward"),
        c("Send to Back", "object.arrange.sendToBack"),
        Sep,
        c("Send to Current Layer", "object.arrange.sendToCurrentLayer"),
    ]
}

/// Sort menu items alphabetically by their display label in `lang`, folding Latin accents so
/// panels and submenus appear in the expected alphabetical order in every interface language (#4143).
fn sort_by_localized_label(items: &mut [Item], lang: crate::i18n::Lang) {
    items.sort_by_cached_key(|it| {
        let label = match it {
            Item::Cmd(l, id, _) => crate::i18n::tr_id(lang, id, l),
            Item::Sub(l, _) | Item::Todo(l, _) | Item::Header(l) => crate::i18n::tr(lang, l),
            Item::Sep => "",
        };
        (
            crate::i18n::name_sort_key(label),
            match it {
                Item::Cmd(l, ..) | Item::Sub(l, ..) | Item::Todo(l, ..) | Item::Header(l) => *l,
                Item::Sep => "",
            },
        )
    });
}
