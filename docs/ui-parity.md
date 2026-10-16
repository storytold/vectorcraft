# UI and interaction parity

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-16 · **Change:** minor (batch exporting, #1935; document window scroll bars, #2322; Anchor Point tool: a click on a handle end retracts that handle only, #2267; scores unchanged; Smart Guides: the dragged end of a straight segment keeps to its extension at any angle, #2120; Warp Options controls verified by focused checks and original-image review; estimates unchanged; existing-guide Escape cancellation validated 2026-10-14; scores and hours unchanged; open-path anchor Delete keeps surviving runs separate; estimates unchanged; Artboard tab-switch follow-up historically validated on `a0671777`; scores and hours unchanged; Artboard clean active-tab close/Delete follow-up validated 2026-10-15; Curvature curve geometry, #1236 in part; Width-point finite-arithmetic follow-up validated; scores and hours unchanged; Image Trace adjustment discovery; later 2026-10-14 merges: macOS save sheet Format menu back to missing, #1855, #866 reopened; anchor X/Y/W/H in the Transform panel, Properties and Control bar, #1824; Snap to Grid over Snap to Pixel, #1816; toolbar labels, columns and the All Tools drawer, #1751; Actions panel selection, #1629 in part; scores unchanged; 2026-10-14 merges: anchor Align and Distribute, #1636; Delete on closed paths and whole objects, #1674; selection inside isolation, #1666; Escape in a Direct Selection drag and while typing, #1743, #1686; Pen clicks under 3 px, #1663; [ and ] on more tools, #1661 in part; fields commit on click away, #1787; command search keys, #1757; a guide's position in the Transform panel is a convenience with no parity claim, #1659; scores unchanged; intersect Smart Guides for drawn points and dragged anchors, #1602 in part; a ruler guide's position in the Transform panel, #1597 in part; Symbol Options before New Symbol, #1560; tool flyout widths, #1616; Pathfinder Trim, Merge and Crop strokes, #1570; scores unchanged; G1/M11: Artboard deletion and history selection normalization; history regressions and full workspace CI pass; scores and hours unchanged; Width Point Edit: Shift-double-click enables adjoining adjustment; plain double-click keeps it off; scores and hours unchanged; per-corner radius fields with a link toggle, split from #1594; Join merges anchors lying on one another, #1533; correction: Average and Join with a corner or smooth choice is missing, not done; guide objects take the guide colour and style and Lock Guides, #1534; Keyboard Shortcuts lists commands outside the menus, #1539; round corners next to curves and the Smooth tool's reach, #1512; Make Compound Path on a group, #1537; dialog slider rails, #1483; template layers stay in Preview in Outline view, #1486; Direct Selection takes the selected object's anchor where anchors coincide, #1508; Tabs panel stop drags, #1458; Split Into Grid dialog, #1462; Transform Objects and Reflect's angle, #1448 and #1460; Pathfinder panel on a group, #1450; value fields: readable selection, #1465 in part, and a focused field follows its value, #1457; scores unchanged; Transform dialogs have a live Preview and step with the arrow keys, #1412; transform tools, dialogs and Move act on direct-selected anchors, #1408; the bounding box hides during a corner drag, #1229; scores unchanged; dialogs focus their first field on every open, #1367, and a first field that starts greyed out once it is enabled, #1395; scores unchanged; Rectangular Grid and Polar Grid click dialogs take the skew, frame, compound path and Fill Grid options, #1377 in part, score unchanged; Live Corners: a Direct Selection click on a widget selects its corner, #1239; custom ruler origin, #1212; Artboard tool clicks fit the topmost editable object in one undo step (#1227); scores unchanged; Pathfinder single-result source names and SVG IDs retained, #1293; DXF control-point SPLINE closure read, #1299 follow-up; Document Setup: Edit Artboards closes the modal after applying valid settings; pointer-click regressions; scores and hours unchanged; Artboards: Escape exits editing; an active drag cancels first; shortcut-routing regression; scores and hours unchanged; Simplify preview/counts and valid rewritten-path selection (#1272); parity remains partial; Texture-effect drag redraws optimized (#1257); idle Windows interaction still unverified; scores unchanged; Curvature point display improved (#1236, in part); scores unchanged; Direct Selection: no-op anchor Align preserves Undo, Redo and saved state (#1180 follow-up); Eyedropper cursor distinguished from precise crosshair (#1269); scores unchanged; Eyedropper can read visible locked and outside-isolation sources (#1251); scores unchanged; Direct Selection: independent segment picks and Delete (#1278); scores unchanged; Adding paint to an unpainted zero-width stroke restores the 1 pt default (#1253); scores unchanged; macOS save sheets offer a Format menu, #866; Attributes fill-rule controls edit the compound when a member is selected, #1197; a marquee needs to touch a path, not just lie inside its fill, #1103; earlier: Live Corners round corners between curved sides, #1144) · **Target:** Adobe Illustrator 2026 (30.x)

How VectorCraft's tools, handles, snapping and panels behave next to Illustrator's. A vector illustrator's
trust is in the details: where a handle lands, what Shift and Alt do mid-drag, what a smart guide snaps to,
how far an arrow key nudges. This checklist is the work list for the **interaction-fidelity pass**
([gaps.md](gaps.md) G1). Part of [target-app-parity.md](target-app-parity.md).

**How it is measured:** from the tool state machines' own documentation and tests (`crates/tools/src/*.rs`,
`crates/ui-egui/src/canvas.rs`, `shortcuts.rs`, the `tests_*` files) against Illustrator's public documentation
(helpx tool pages and the keyboard-shortcut reference). Never against the running app: the clean-room rule
forbids launching or measuring Illustrator, so there has been **no side-by-side session**. Every "done" below is
"behaves as the documentation describes", which is weaker than "a power user can't tell". Status:
**done** (documented behaviour implemented and tested), **partial**, **missing**.

## Summary

| Area | % | Remaining (h) | Kind |
|---|---:|---:|---|
| Pen, Curvature and anchor tools | 85% | 6–10 | estimated |
| Direct Selection, handles and anchors | 85% | 5–8 | estimated |
| Smart guides and snapping | 70% | 8–12 | estimated |
| Transform handles, bounding box, Free Transform | 80% | 5–8 | estimated |
| Nudging, numeric entry, precision, units | 85% | 3–5 | estimated |
| Modifier keys and temporary tools | 80% | 5–8 | estimated |
| Keyboard shortcuts (default set) | 70% | 6–10 | estimated |
| Cursors | 75% | 3–5 | estimated |
| Panels, Control bar, Properties, context menus | 70% | 15–25 | estimated |
| Look and feel (layout, theme, metrics) | 80% | 5–10 | measured once against public screenshots on 2026-10-02; panels added since not re-measured |
| Accessibility (screen reader, keyboard-only canvas) | 20% | 15–25 | estimated: AccessKit on by default for egui widgets; canvas and custom panels not described, never tested with a screen reader |
| **UI/UX overall** | **~55%** | **~70–110** (overlaps the feature areas) | estimated |

The overall figure is lower than the rows because what users notice first (small differences across many
tools, never checked side by side) isn't captured row by row.

## Pen, Curvature and anchor tools

| Behaviour (Illustrator, public docs) | Status | Notes |
|---|---|---|
| Click: corner anchor; drag: smooth anchor with symmetric handles | done | `pen.rs` |
| A press that moves under 3 screen pixels is a click (a corner without handles) | done | #1663 |
| Shift constrains segments and handles to 45° | done | Illustrator's Constrain Angle preference is honoured (`constrainAngle`) |
| Alt mid-drag splits the handles (only the outgoing follows) | done | keeps the curve already shaped |
| Space mid-drag repositions the anchor, handles and all | done | |
| Click the first anchor to close; the drag shapes the closing curve | done | |
| Click the last anchor to retract its outgoing handle; drag from it to pull a new one | done | |
| Alt over a handle, anchor or segment acts as the Anchor Point tool | done | |
| Cmd/Ctrl lends the last-used selection tool | done | the path being drawn continues afterwards |
| Click an open path's end to continue it; click another path's end to join | done | #776 |
| Rubber Band preview | done | Preferences option |
| Auto Add/Delete on a selected path; Shift or the preference disables it | done | |
| Pen cursors (new path ×, continue /, close o, add +, delete −, convert ^) | partial | vector cursors exist; not every state verified |
| Each anchor snaps to Smart Guides and construction angles | done | `DrawSnap` |
| Curvature tool: click/drag points, double-click or Alt-click for a corner, temporary Direct Selection, point conversion | partial | circular point markers without direction handles or duplicate markers, Anchor Size and Hide Edges regression-tested (#1236, in part); curve geometry: each stretch of three points follows the circle through them (handles along its tangent, chord/(3·cos²(α/2)) long, shorter in a turn tighter than 30°), open ends curving in, the same rule for the rubber band, dragged, inserted and converted points (#1236, in part); neighbours of a point dragged on an existing path keep their tangents, and modifiers requested in #975 remain |
| Add/Delete/Anchor Point tools; Anchor Point tool drags segments to reshape | done | #2267: a click on a handle end retracts that handle and leaves the anchor a corner with the other one |
| Pencil: Fidelity, Keep selected, Alt to Smooth, edit selected paths, close by proximity | partial | Alt to Smooth is missing; with Keep selected (Pencil and Paintbrush, on by default) off, the path a stroke draws or continues is not selected afterwards; a filled Pencil path keeps a stroke of None (#1804, part of #1679) |
| Join tool scrubbing, Average, Join | done | Join merges the direct-selected anchors lying on one another along a path, as after Average, into one (#1533) |
| Average and Join in one step, with a corner or smooth choice for the joined anchor | missing | `path.join` and `path.average` are separate commands, and Join has no corner/smooth choice; [gap 1](gaps.md) |
| Smooth tool smooths only the stretch of path under the drag | done | #1512 |
| Simplify (slider, auto, corner angle threshold) | partial | Tolerance, default-on Preview and original/new point counts (#1272); Corner Point Angle Threshold, Convert to Straight Lines and Show Original Path; slider, auto-simplify and Retain my latest settings remain missing |

## Direct Selection, anchors and handles

| Behaviour | Status | Notes |
|---|---|---|
| Click/Shift-click anchors; marquee toggles with Shift | done | #483 |
| Align selected anchors to their bounds or the artboard; repeating an alignment that moves nothing keeps Undo, Redo and saved state | done | #1180 follow-up; engine regressions include rounding-only moves and unchanged handles; #1641 (fixes #1636): Align and Distribute act on the anchors selected with Direct Selection, the points of one path aligning to each other and Distribute spacing them |
| Align selected anchors to a key anchor | missing | #1636 |
| A marquee selects the objects whose paths it touches (anchors or segments): drawn inside a filled shape, over a background, it leaves that shape (Preview and Outline) | done | #1103 |
| Click a segment independently of its endpoint anchors; only its direction handles show | done | #1278: hollow endpoint squares, selected edge emphasized; headless egui checked |
| Drag a curved segment to bend it, a straight one to move its anchors | done | |
| Drag by an anchor of a path selected whole moves the whole path; Alt-drag by an anchor or a segment copies it, Alt toggles the copy during the drag, and the copy snaps by the anchor pressed, onto the original too | done | #1104 |
| Handles shown are the handles that drag; tolerance from Selection & Anchor Display | done | #494 |
| Drag a handle with Shift (45° about its anchor), Alt (move one handle alone) | done | handles snap to smart guides |
| Show handles when multiple anchors are selected; Highlight anchors on mouse over; anchor and handle size | done | Preferences › Selection & Anchor Display |
| Convert selected anchors to corner/smooth from the Control bar and Properties | done | |
| Cut path at selected anchors, Remove Anchor Points, Delete removes segments | done | Open-path anchor Delete removes adjoining segments without bridging the gap; targeted core and full-frame Delete/Backspace regressions pass for survivor selection and Undo/Redo. Explicit Remove Anchor Points keeps its reconnect/refit behavior; #1278: segment Delete preserves neighboring edges and endpoint anchors; a second Delete removes the remainder; explicit anchor Delete retains its behavior; #1674: Delete removes every selected anchor of a closed path and objects selected whole along with them, and keeps the remainder selected; #2241: Remove Anchor Points then selects the next anchor along the path (the one before at an open end), so it can be run again without picking each anchor |
| Direct Selection and Group Selection pick only inside the isolated object in isolation mode | done | #1666 |
| Escape cancels a Direct Selection drag | done | #1803 (#1743) |
| Live Corners widgets (radius readout, Alt-click cycles kind, double-click the Corners dialog) | done | a Direct Selection click on a widget selects its corner (#1239); a press-and-drag on a widget of a path selected whole still rounds every corner (#908), and adding widgets to the selection by Shift-click (#1232) is missing; the bounding box hides while a widget is dragged with the Selection tool (#1229); a round corner next to a curved side curves smoothly instead of sticking out of the shape (#1512) |
| Corners between curved sides (#1144), Relative rounding | partial | Relative rounding missing |
| Pie widgets on a live ellipse: drag one to set Pie Start or End Angle, double-click one to make the ellipse whole | done | Selection tool, with the bounding box showing |
| Isolation mode by double-click (groups and single objects), breadcrumbs, Esc to exit | done | Single paths, compound paths, images, meshes and placed files isolate too ([helpx: Isolate objects](https://helpx.adobe.com/africa/illustrator/desktop/manage-objects/select-objects/isolate-objects.html)) |
| "Magnetic" moves of selected anchors with a modifier (#955 request) | missing | not an Illustrator feature as documented; under discussion |
| Coinciding anchors of overlapping objects: the selected object's anchor is taken, also under another object | done | #1508 |

## Smart guides and snapping

| Behaviour | Status | Notes |
|---|---|---|
| Snap to anchors, centres, edges, artboards, bleed and ruler guides with magenta lines | done | `guides.rs` kinds: Anchor, Center, Edge, Artboard, Bleed, Guide |
| Anchor/Path labels ("anchor", "center", "path", "align") | done | |
| "intersect" label and snapping to path intersections | partial | #1658 (part of #1602): a point drawn with the Pen, the shape tools or the other drawing tools and an anchor dragged with Direct Selection snap to where paths and ruler guides cross; a moved selection's own anchors and the Width tool don't |
| Construction guides at preference angles while drawing | done | #506 |
| Line extension: the dragged end of a straight segment snaps to the segment's own direction at any angle, past its other end or short of it, with the guide along it | done | #2120; Direct Selection and Curvature, one anchor dragged (with several, the move has no single direction); nearer the horizontal or vertical through the other end, the point lines up with it ("align"), so a nearly level segment can still be straightened |
| Measurement labels while drawing, moving and resizing | partial | sizes and offsets; angle and distance readouts while drawing missing |
| Spacing guides (equal distances) | partial | for moves (#394); not for resizing or drawing |
| Snapping tolerance preference | done | |
| Snap to Point, Snap to Grid, Snap to Pixel; Pixel Preview | done | #1840: with Snap to Grid on, anchors and objects snap to the grid even when Snap to Pixel is also ticked (#1816 waits for the reporter) |
| Snap to Glyph | missing | menu stub |
| Tangent and perpendicular snapping for line endpoints (new in 30.0) | missing | |
| Snapping panel/popover consolidating the snap options (30.0) | missing | |
| Customisable rotation snap angle (#973) | missing | Illustrator: Constrain Angle and construction angles only |
| Ruler guides: drag out, select, move, delete, lock, Make/Release Guides | done | #414, #451; marquee selects unlocked ruler guides |
| Escape cancels an existing ruler-guide press, move or copy | done | Selection and Direct/Group Selection; app-input regressions cover preselected guides, document/interaction-start selection, Undo/Redo and saved state, later on-canvas motion/release and temporary borrowing from idle Pen. All eight checks and the Direct Selection Escape control passed in current full CI. [Validation evidence](target-app-parity.md#evidence-what-ships-today-by-area) |
| Ruler origin: corner drag with preview, double-click reset to the active artboard, relative X/Y fields, native save/reopen | done | #1212; `document.setRulerOrigin`, `document.resetRulerOrigin`; geometry and command coordinates stay absolute |
| Ruler guides as Layers rows: shown, hidden, locked and deleted with their layer; rows dragged to another layer | done | `guide.setLayer`; guides from older files go on a layer |
| Objects made into guides: drawn in the guide colour and style of Preferences; Lock Guides locks them too (no select, move or delete); a dragged selection snaps to ruler guides with Smart Guides on | done | #1534 (#1543, #1583) |
| A selected ruler guide's X or Y in the Transform panel; typing a value moves it | not scored | #1659 (part of #1597); several guides move together; not in Properties or the Control bar. Not established by the documentation we could find: the help pages on guides, rulers and the Transform, Control and Properties panels (read again on 2026-10-14) don't describe it, and the one public statement is a user's [community answer](https://community.adobe.com/questions-652/adding-a-new-custom-guide-785586), which is not documentation. Kept as a convenience pending the owner's decision; no parity claim |
| Alt swaps a ruler guide's orientation while dragging | missing | |
| Alt swaps a new ruler guide's orientation while dragging | done | Both rulers; preview, snapping and final placement follow the current modifier; existing-guide Alt-copy is unchanged. |
| Global/artboard ruler mode switch and video rulers | missing | menu stubs; custom document origin is done (#1212) |

## Transform handles, bounding box, Free Transform

| Behaviour | Status | Notes |
|---|---|---|
| Moving a rounded rectangle with Craquelure at high DPI | partial | Distance searches optimized; moved-viewport pixels unchanged for three texture effects; `texture_drag_perf` probe available. Idle Windows mouse-drag QA remains (#1257) |
| Bounding-box handles: Shift proportional, Alt from centre, rotate outside the corners | done | |
| Rotated objects keep a rotated bounding box and their angle; Reset Bounding Box | done | |
| Free Transform, Scale and Reflect along a rotated box | missing | |
| Free Transform widget: constrain, free distort, perspective distort; the key-held variants | done | |
| Transform tools: click sets the reference point (snaps to anchors/centres), Alt-click opens the dialog, Alt-release copies | done | with anchors selected with Direct Selection, the tools, the dialogs and Move transform those anchors alone, about the centre of their box by default (#1408; the default point and whole-object copies are our choices) |
| Move, Rotate, Scale, Reflect and Shear dialogs: live Preview; Up/Down step angles and percentages by 1, Shift by 10 | done | #1412; Preview starts off (our choice: the public documentation gives no default); OK is one undo step and Transform Again repeats it |
| Transform Again, Transform Each, Scale Strokes & Effects, Scale Corners | done | |
| Transform Objects beside Transform Patterns in the Move, Rotate, Scale, Reflect and Shear dialogs and Transform Each; Reflect across an angle | done | #1448, #1460; off, only the pattern tiles transform; the checkboxes are always enabled (our choice); the link that keeps Transform Each's two scales equal is missing (not documented) |
| Split Into Grid dialog: number, height or width, gutter and total for rows and columns, Add Guides, Preview | done | #1462; opens at the selection's size; several paths become one grid painted as the topmost one; a typed height or width keeps the gutter and changes the total (our choice) |
| Reference point locator (nine squares) in Control bar and Transform panel | done | |
| Transform panel, Properties and Control bar X/Y/W/H of the anchor points selected with Direct Selection | done | #1839 (fixes #1824): they show and edit the selected points' bounds; a whole-object selection is unchanged |

## Nudging, numeric entry and precision

| Behaviour | Status | Notes |
|---|---|---|
| Arrow keys nudge by Keyboard Increment (0.001–1296 pt); Shift ×10; Alt copies | done | |
| Fields commit what was typed on a click away or a move to another field, as Enter does; Escape discards it | done | #1787 |
| Number fields: Up/Down step, Shift ×10, Cmd/Ctrl ÷10, wheel over a focused field | done | Shift+arrow in the Stroke weight field doesn't step by 10 (#991) |
| Arithmetic in fields (`+ - * /`, and `%` of the value before it: `3cm*50%`, `50pt+25%`) and unit suffixes (`3 mm`, `1in`, `2p6`) | done | `Unit::parse`; mixed units in one expression not verified |
| Numeric fields: focus click selects the whole value; dragging selects from the pressed character | partial | #1326; select-all waits for click release; px/pt/plain/optional fields, both directions and replacement text tested; native API input, 17 px partial replacement, Undo/Redo, saved-file round trip and screenshots verified; Tab, double-click, arrows and wheel preserved; short movements still classified as clicks can select all and remain open |
| Units: pt, pc, in, mm, cm, px, ft-in, m, yd, ft; General/Stroke/Type units | done | units preference reverting to points on SVG output reported (#864) |
| Field precision: 3 decimals for steps; values kept as f64 | done | |
| Scrubby labels (beyond Illustrator) | done | #400 |
| Dialogs open with the first field focused and selected; Enter confirms | done | on every open, not only the first (#1367); a first field that starts greyed out takes the focus once enabled (#1395) |
| Value fields: selected text readable in every brightness; a focused field follows its value when a slider or preview changes it | partial | #1490 (part of #1465: the wheel over a hovered, unfocused field still scrolls the panel), #1457 |
| Sliders in dialogs and popups (Save for Web, raster export options, Preferences, the opacity popup) show their rail, filled up to the handle | done | #1483 |
| Rectangular Grid and Polar Grid click dialogs: size, divider counts with a skew slider each, Use Outside Rectangle As Frame / Create Compound Path From Ellipses, Fill Grid; kept as the tool's options, used by drags too | partial | #1377. The options are the documented ones; how far a skew percentage moves the dividers and which way the radial skew turns are our own choice (the public documentation gives neither), and the reference point locator and a double-click on the tool are missing |

## Modifier keys and temporary tools

| Behaviour | Status | Notes |
|---|---|---|
| Space: Hand; Cmd+Space / Cmd+Alt+Space: zoom in/out; middle button pans | done | |
| Document window scroll bars along the right and bottom edges (Normal and Full Screen Mode with Menu Bar; none in Full Screen and Presentation Mode): a thumb as long as the view against the art and the artboards; dragging it scrolls, a click on the track pages, the wheel over a bar scrolls along it; in a rotated view they follow the screen's axes | partial | #2322. The span is the art and the artboards with half a window around them, not Illustrator's 227 in canvas: VectorCraft has no bounded drawing area (#2284). A track click pages rather than jumping to the click; the page size and the rotated-view behaviour are our own choice (the public documentation gives neither) |
| Cmd/Ctrl: last-used selection tool; Cmd+Alt: Group Selection inside Direct Selection | partial | |
| Eyedropper reads visible sources while their layer/object is locked or outside isolation | done | Ordinary/Shift/Shift+Alt and gradient-stop reads preserve the source and selection; Alt reverse remains editable-only; hidden art stays excluded (`eyedropper_sources.rs`, #1251) |
| Add a visible stroke after drawing with inherited None/zero-width paint | done | Paint and Swap Fill/Stroke restore the existing 1 pt default; positive widths and painted zero widths stay as set; Undo/Redo and targeted appearance items tested (`stroke_visibility.rs`, #1253) |
| Double-click a stroked segment to create a width point in Width Point Edit | done | Finite-arithmetic follow-up: engine regressions and full CI passed; [Validation evidence](target-app-parity.md#evidence-what-ships-today-by-area). Command and pointer regressions cover opening, sampled widths, adjoining adjustment on/off through checkbox/OK, compound paths, Cancel/Delete and Undo/Redo; dialog layout reviewed. [Validation evidence](target-app-parity.md#evidence-what-ships-today-by-area) |
| Shift-double-click a width point enables adjoining adjustment in Width Point Edit | done | Tool dispatch, plain double-click control, neighboring widths, Undo/Redo and Cancel in `dialogs/width_point.rs` |
| Alt-drag duplicates (objects, artboards with their art) | done | |
| Document Setup → Edit Artboards applies valid settings, closes the dialog and activates the Artboard tool | done | Pointer-click tests cover successful handoff, invalid settings, OK, Cancel and Undo in `dialogs/document_setup.rs`. |
| Escape leaves Artboard editing for Selection; during a drag it first cancels the gesture | done | Headless egui key events cover idle Escape, active-drag cancellation and a subsequent Escape in `tests_distortkeys.rs`. |
| Escape while typing ends the edit, keeps the type selected and switches to the Selection tool | done | #1802 (#1686, #1656) |
| Shift-click selects several artboards (Artboard tool, Artboards panel); they move, Alt-copy and delete together | done | `artboard.move`/`artboard.delete` `indices` (#1139); panel menu, trash and Properties share selected-set deletion; all-selected retains artboard 1 and a single board cannot be deleted, with pointer-event and Undo/Redo coverage |
| Artboard Delete/Backspace delete selected boards while protecting artwork, the last board and active drags | done | Headless shortcut regressions in `tests_distortkeys.rs` cover retaining the original first board when all are selected, single-board protection, artwork, saved state and Undo/Redo; the navigator follows the tool’s surviving board through repeated deletion and history changes. |
| Artboard navigator synchronizes the active tool selection before deletion | done | Actual status-bar Next → Delete/Backspace and Next → Properties Delete regressions in `chrome.rs` cover surviving board records, preserved artwork and Undo/Redo; other tools keep object selection and history. **Tab-switch follow-up:** document-tab changes align the Artboard tool with the displayed board. Actual A → B → A tab clicks followed by Properties Delete and controls for same-tab multiselection, Move/Scale Artwork options and other tools passed, along with historical full CI on `a0671777`. [Validation evidence](target-app-parity.md#evidence-what-ships-today-by-area). **Clean-tab close:** with the Artboard tool active, closing a clean active tab restores the surviving document's displayed board before Properties Delete. |
| Artboard history prunes removed indices while preserving valid multiselection and tool options | tested | Headless pointer/key tests in `panels/properties.rs` cover panel New → Undo → Delete/Backspace/Properties, Duplicate → Undo → Delete, partial pruning and valid multiselection. They check surviving board records, artwork, object selection, Move/Scale Artwork options, saved state and Undo/Redo. Full workspace CI passes. |
| Click an object with the Artboard tool to create an artboard fitted to its visible bounds | done | Topmost editable object or group; drag and modified clicks retain their meaning; one undo step (`artboard_click.rs`, #1227) |
| Shape tools: Shift constrains, Alt from centre, Space moves while dragging, arrows change sides/points/rows | done | |
| Tool double-click opens its options | done | |
| Per-platform mapping (Cmd ↔ Ctrl, Option ↔ Alt) | done | |
| Eyedropper cursor; Use Precise Cursors | partial | original pipette at the sampling tip, distinct from the precise crosshair at five DPI scales (#1269); OS cursor display and Caps Lock unverified; a Precise Cursors command toggles the preference and can take a shortcut (#1663); cursors are painted into the window on Wayland (#1652, not run on Linux) |
| Every tool's modifiers checked against the shortcut reference | missing | this is the fidelity pass |

## Keyboard shortcuts

~110 distinct modified shortcuts on menu commands plus single-key tool shortcuts for the default toolbar
(`crates/tools/src/catalog.rs`), a shortcut editor with chords, and saved sets. Not yet diffed item by item
against Illustrator's published default shortcut list; estimated ~70% of it. Remaining: the panel shortcuts,
the type shortcuts (tracking/kerning/leading/baseline steps with Alt/Shift), and selection shortcuts in the
Layers panel. The shortcut editor also lists the commands that are in no menu but run without parameters (Edit › Duplicate,
panel-menu commands such as Break Link to Symbol), so they can be given shortcuts (#1539). The [ and ] keys reach the Blob Brush, the Eraser and the Symbolism tools in the app (#1734, part of #1661).

## Panels, Control bar, Properties, context menus

| Behaviour | Status | Notes |
|---|---|---|
| Effect > Warp Options: style previews, orientation choices and percent controls | done | #1760; focused input, mixed-style and history checks passed; original captures reviewed. No geometry or optional Properties changes. |
| Command search: Up and Down move through the results, Enter runs the highlighted one | done | #1798 (#1757) |
| Pathfinder panel: a group selected alone counts its nested shapes, so an expanded stroked object keeps the Shape Modes enabled | done | #1642 (#1635) |
| Object › Expand on radial, grid and mirror repeats makes groups of their instances | done | #1765 (#1756) |
| Select › Same matches type on its characters' fill and stroke | done | #1746 (#1737) |
| ~40 panels with ≡ menus; dock, icon column, collapse, tabs | done | 51 panel modules incl. tests |
| Panels float inside the window, stack, dock back | done | #495 |
| Floating groups stack into sets; sets dock as extra dock columns (a multi-column dock); groups drop anywhere in a set or column, or onto its tabs; a column taller than the window scrolls | done | `window.panel.float {below, above}`, `window.panel.dock {column}` |
| Panels outside the app window (another monitor) | missing | single OS window |
| Edit Toolbar (•••): the All Tools drawer opens beside the toolbar by its button (Escape or a click outside closes it), dims the tools already in the toolbar; while it is open, tools drag from it onto the toolbar (between two tools, or onto one to join its group), from the toolbar into it, and within the toolbar; each toolbar keeps its edits | done | #1018; `window.editToolbar`, `toolbar.addTool`/`removeTool`/`reset`; the drawer's Reset (Illustrator has it in a drawer menu); dragging a toolbar button moves the tool it shows, not its whole group; New/Manage Toolbars missing; tool flyouts are as wide as their longest translated name and shortcut need (#1616) |
| Quick Image Trace presets reveal editable controls; Properties reopens them | done | Threshold, Paths, Corners and Noise; `panels/image_trace.rs` headless interaction tests. No algorithm parity claim. |
| Toolbar without group labels by default, one or two columns; All Tools drawer by flyout group with shortcuts, its menu (Reset, Basic, Advanced) and Show section | done | #1751; a preference brings the labels back; `toolbar.show` sets the Fill/Stroke, Coloring, Draw Mode and Screen Mode controls |
| Actions panel: a clicked action stays selected for Play Current Selection and Delete Selection | partial | #1853 (part of #1629); editing actions and action shortcuts remain |
| Properties panel per context (every selection type's Quick Actions) | partial | |
| Contextual task bar | done | |
| Control bar with Transform and Stroke popovers | done | |
| Pathfinder shape results keep their source names and SVG IDs | done | #1293; Minus Front keeps the back name, Minus Back the front name; Unite/Intersect/Exclude use the backmost named input; compound output carries it once; geometry and paint unchanged |
| Attributes fill-rule controls with a compound member selected | done | Edits the compound's rule; direction still edits only the member; rendered-hole and Undo/Redo regression tests (#1197) |
| Make Compound Path on a group (such as outlined type) keeps its backmost path's paint; compound paths joined into one keep their holes | done | #1537 |
| Symbols panel: New Symbol opens Symbol Options (Name) before making the symbol; Alt-click skips it | partial | #1560 (#1566); Registration 9-point grid in Symbol Options (`symbol.new` / `symbol.place`); export type, static or dynamic and 9-slice settings still missing |
| Pathfinder Trim, Merge and Crop remove strokes; Merge joins same-fill objects whatever their strokes | done | #1570 (#1572) |
| Pathfinder panel on one group selected alone; a greyed-out button says what it needs | partial | #1450; the group's members are combined; one plain path selected alone stays disabled (the public documentation doesn't say what the operations do to a single path) |
| Pathfinder Divide: an open cutting line splits a filled path | done | #1307; straight, crossed, bent and curved cutters, holes and frontmost appearance regression-tested; native automation: independent piece movement, one Undo/Redo, saved-document round trip and window capture; existing bounded-map fallback remains |
| Canvas context menu | done | Layers rows, panels and type have none (gap) |
| Blend panel (30.7) | missing | Blend Options dialog exists |
| SVG Interactivity, Retype, Comments, Version History panels | missing | the last two are cloud features (out of scope) |
| Workspaces (save, reset, switch) | done | |
| DXF export/import preserves intended curve closure | done | #1299; R13–2018 closed SPLINE bit set/read; an open curve ending at its start stays open; knots and control points unchanged; native CIRCLE/ARC export and CNC preset remain pending |
| macOS save sheet: choose the format from a menu | missing | #866 (reopened); the menu was withdrawn by #1855 to restore saving: Save As, Save a Copy and the first Save showed no sheet with it. It comes back once the format can be set before the sheet opens |
| Preferences: every option acts | partial | ~16 stored but unused (#394) |
| Tabs panel: a stop drags from where the press landed, a stop just added included | done | #1458 |
| Outline view keeps template layers in Preview, dimmed | done | #1486; a picture placed to be traced stays visible under the outlines |

## Look and feel

Medium Dark theme and four brightness levels, the categorized and Advanced toolbars, 35 pt document tabs,
33 pt panel tabs, hint bar, contextual task bar and native macOS menus, restyled to values measured from
public screenshots on 2026-10-02 (~75–80% then). Missing: a System theme that follows the OS (#825), a
re-measure of the panels added since. Slider rails show on Medium Dark (#1808, #1662).

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-16 | minor | Batch exporting in Export for Screens (#1935): a file name pattern (document and item names, a counter, the date), folders per format, item or both, Layers and Selection tabs with thumbnails, EPS, AI and VectorCraft rows, a PDF row with printer's marks, and export presets (built-in Logo Package and Print-Ready PDF, saved ones); scores and hours unchanged (estimated) |
| 2026-10-16 | minor | Anchor Point tool: a click on one handle end retracts that handle only; the anchor becomes a corner and keeps the other handle (#2267). Handle drags start past the same 2 px as anchor drags. Scores and hours unchanged. |
| 2026-10-16 | minor | Smart Guides: the dragged end of a straight segment snaps to the segment's extension at any angle, with the guide from its other end (#2120); scores and hours unchanged |
| 2026-10-14 | minor | Effect > Warp: style, orientation and percent controls; focused checks passed and original captures reviewed; scores and hours unchanged |
| 2026-10-14 | minor | G11: open-path anchor Delete leaves separate surviving runs; dedicated Remove Anchors remains connected. Keyboard and history regression coverage added; scores and hours unchanged. |
| 2026-10-14 | minor | Curvature: the Curvature tool's curve: each stretch of three points follows the circle through them (handles along its tangent, chord/(3·cos²(α/2)) long, shorter in a turn tighter than 30°), and the ends of an open path curve in, so uneven spacing neither overshoots nor loops; the rubber band, dragged, inserted and converted points use the same rule; the Pencil's fitting unchanged (#1236, in part); scores unchanged (estimated) |
| 2026-10-14 | minor | Quick Image Trace presets reveal the adjustment panel with Advanced open; Properties reopens it. Sliders keep their preset row until release so the re-trace is committed. UI regressions added; scores and remaining estimates unchanged. |
| 2026-10-14 | minor | The macOS save sheet's Format menu goes back to missing: #1855 withdrew it to restore saving (#866 reopened). Rows added for the X/Y/W/H of direct-selected anchors (#1839, fixes #1824), the toolbar and All Tools drawer (#1751) and the Actions panel selection (#1853, part of #1629); the snapping row notes Snap to Grid over Snap to Pixel (#1840). Scores unchanged (estimated) |
| 2026-10-14 | minor | Existing-guide Escape cancellation: eight app-input regressions, the Direct Selection Escape control and current full CI passed; scores and hours unchanged. |
| 2026-10-15 | minor | Artboard tool: closing a clean active tab restores the displayed board before Properties Delete. Nine targeted checks passed; three original close-route captures reviewed; scores and hours unchanged. |
| 2026-10-14 | minor | Artboard tab changes align the tool with the displayed board; same-tab multiselection and Move/Scale Artwork options preserved. Three tab regressions and historical full CI on `a0671777` passed; scores and hours unchanged. |
| 2026-10-14 | minor | Width-point finite-arithmetic follow-up: atomicity, history and native round-trip regressions and current full CI passed; scores and hours unchanged. |
| 2026-10-14 | minor | Align and Distribute act on the anchor points selected with Direct Selection: the points of one path align to each other, not to the artboard, and Distribute spaces them (#1641, fixes #1636; aligning to a key anchor is still missing); Delete removes every selected anchor of a closed path (it used to remove the wrong ones), removes objects selected whole along with the selected anchors of others, and keeps the remainder selected so a second Delete removes it (#1674); In isolation mode Direct Selection and Group Selection pick only inside the isolated object (#1666); Escape cancels a Direct Selection drag (#1803, #1743); Escape while typing ends the edit, keeps the type selected and switches to the Selection tool (#1802, #1686, #1656); A filled Pencil path keeps a stroke of None (#1804, part of #1679); With the Pen, a press that moves under 3 screen pixels is a click (a corner without handles), and a Precise Cursors command toggles the existing preference and can take a shortcut (#1663); The [ and ] keys reach the Blob Brush, the Eraser and the Symbolism tools in the app (#1734, part of #1661); In the Pathfinder panel a group selected alone counts its nested shapes, so an expanded stroked object keeps the Shape Modes enabled (#1642, #1635); Object › Expand turns radial, grid and mirror repeats into groups of their instances (#1765, #1756); Select › Same matches type on its characters' fill and stroke (#1746, #1737); Fields commit what was typed when you click away or move to another field, as Enter does; Escape still discards it (#1787); Up and Down move through the command search results and Enter runs the highlighted one (#1798, #1757); Slider rails show on Medium Dark (#1808, #1662); On Windows the custom title bar's window stays truly borderless, so the pointer lines up with what is drawn on systems where the hidden caption offset it (#1791, #1758; awaiting the reporter's confirmation); Tool cursors are painted into the window on Wayland (#1652; not run on Linux, #1645 stays open until confirmed); the guide-position row (#1659) is now "not scored": the documentation found doesn't establish it, and it is kept as a convenience pending the owner's decision; scores unchanged |
| 2026-10-13 | minor | Pathfinder's Trim, Merge and Crop remove the objects' strokes, and Merge joins same-fill objects whatever their strokes were, as the public Pathfinder page describes (#1572, fixes #1570); Offset Path with an inset larger than the shapes says the offset leaves nothing, instead of asking to select paths (#1591, fixes #1590); The Symbols panel's New Symbol opens Symbol Options to name the symbol first, and Alt-click skips it (#1566, fixes #1560); Tool flyouts are as wide as their longest translated name and shortcut need, so names no longer run into shortcuts in Spanish and other languages (#1650, fixes #1616); Smart Guides: a point drawn with the Pen or a shape tool, and an anchor dragged with Direct Selection, snaps to where paths and ruler guides cross, labelled "intersect" (#1658, part of #1602; a moved selection's own anchors don't); The Transform panel shows a selected ruler guide's X or Y, and typing a value moves it (#1659, part of #1597). This is not established by the documentation we could find: the public help pages on guides, rulers and the Transform, Control and Properties panels (read again on 2026-10-14) describe selecting and dragging unlocked guides and the panels' X and Y for a selection, not a guide's position in a panel, and the one public statement of it is a user's [community answer](https://community.adobe.com/questions-652/adding-a-new-custom-guide-785586), which is not documentation. It is kept as a convenience pending the owner's decision, with no parity claim and nothing counted towards a score; Select Behind (Ctrl/Cmd-click) tests each object once, so a deep stack of overlapping objects no longer slows it; same objects in the same order (#1571); Commands that take `ids` run on the objects given with nothing selected, over MCP, the control channel and the CLI too (#1577, fixes #1575); scores unchanged (estimated) |
| 2026-10-13 | minor | G1/M11: Artboard panel deletion, protected deletion keys and navigator/tool synchronization before and after deletion; history normalization preserves valid multiselection; history regressions and full workspace CI pass; scores and hours unchanged |
| 2026-10-16 | minor | Document window scroll bars (#2322): thumb drag, track paging and the wheel over a bar; hidden in Full Screen and Presentation Mode; scores unchanged |
| 2026-10-13 | minor | Alt during a drag from either ruler swaps the new guide's orientation ([public shortcut reference](https://helpx.adobe.com/illustrator/using/default-keyboard-shortcuts.html)); tool and egui event tests cover preview, snapping, release coordinates, cancellation and one-step Undo/Redo; scores unchanged (estimated). |
| 2026-10-13 | minor | G1: create width points through Width Point Edit with optional adjoining adjustment; command/pointer/history regressions and full CI passed; dialog layout reviewed; scores and hours unchanged |
| 2026-10-13 | minor | Corner Radius: a link toggle behind the field in the Properties and Transform panels; off, a live rectangle shows a radius field for each corner (Top Left, Top Right, Bottom Right, Bottom Left), each setting its own corner (`singleCornerRadius` preference); split from #1594; score unchanged (estimated) |
| 2026-10-13 | minor | Join merges the selected anchor points that lie on one another, as after Average, into one anchor (#1582, fixes #1533); correction: "Join with corner/smooth choice" was listed as done, but `path.join` and `path.average` are separate commands and Join has no such choice, so Average and Join in one step with that choice is its own row, missing; the Smooth tool smooths only the stretch of path under the drag and a round corner next to a curved side curves smoothly instead of sticking out of the shape (#1547, fixes #1512); Lock Guides also locks the objects made into guides: they can't be selected, moved or deleted (#1543); they are drawn in the guide colour and style of Preferences, and a dragged selection snaps to ruler guides with Smart Guides on (#1583, fixes #1534); Edit › Keyboard Shortcuts lists Edit › Duplicate and the other commands that are in no menu but run without parameters (panel-menu commands such as Break Link to Symbol), so they can be given shortcuts (#1549, #1580, fixes #1539); Make Compound Path on a group (such as outlined type) keeps the paint of its backmost path instead of drawing nothing, and compound paths joined into one keep their holes (#1537); scores unchanged (estimated) |
| 2026-10-13 | minor | Sliders in Save for Web, the raster export options, Preferences and the opacity popup show their rail (#1506, fixes #1483); a template layer stays in Preview, dimmed, in Outline view (#1505, fixes #1486); Direct Selection takes the selected object's anchor where anchors of overlapping objects coincide (#1509, fixes #1508); a tab stop in the Tabs panel drags from where the press landed (#1487, fixes #1458); scores unchanged (estimated) |
| 2026-10-12 | minor | Width Point Edit: Shift-double-click enables adjoining adjustment; plain double-click keeps it off; scores and hours unchanged |
| 2026-10-13 | minor | Split Into Grid has its row and column sizes, gutters, totals, Add Guides and Preview (#1488, fixes #1462); Transform Objects in the Transform dialogs and Transform Each, and Reflect across an angle (#1494, fixes #1448 and #1460); the Pathfinder panel works on a group selected alone and its greyed-out buttons say what they need (#1493, for #1450); selected text in value fields is white or black on the highlight (#1490, part of #1465); a focused field follows its value when a slider or preview changes it (#1497, fixes #1457); an Alt-drag copies once per drag (#1464); scores unchanged (estimated) |
| 2026-10-12 | minor | Transform dialogs (Move, Rotate, Scale, Reflect, Shear) preview on the canvas and step their angles and percentages with the arrow keys (#1428, fixes #1412); the transform tools, dialogs and Move act on anchors selected with Direct Selection (#1427, fixes #1408); the bounding box hides during a corner drag with the Selection tool (#1402, fixes #1229); a picture pastes when blank text sits beside it on the clipboard (#1404); scores unchanged (estimated) |
| 2026-10-12 | minor | Dialogs opened from a menu take the keyboard focus every time they open (#1367); a first field that starts greyed out takes it once enabled (#1395); art flush with an artboard within a rounding error moves, copies and scales with it (#1375, from #1295); scores and hours unchanged |
| 2026-10-12 | minor | Rectangular Grid and Polar Grid tool options (skew, frame, compound path, Fill Grid; #1377, in part): a skew slider under each divider count, kept as tool options and used by drags; the skew curve is our own; score unchanged (estimated) |
| 2026-10-12 | minor | Live Corners: a Direct Selection click on a widget selects its corner, Shift-click toggles it (#1239); #908 and #1232 stay open; score unchanged (estimated) |
| 2026-10-12 | minor | Custom ruler origin (#1212): corner drag previews and sets it; double-click resets to the active artboard; X/Y fields use it and native files keep it; scores unchanged (estimated) |
| 2026-10-12 | minor | Artboard tool clicks fit the topmost editable object in one undo step (#1227); scores unchanged |
| 2026-10-12 | minor | Pathfinder single-result booleans preserve source names and SVG IDs (#1293); paint-order, grouped/compound sources, unchanged geometry/appearance and Undo/Redo regression tests; scores and estimates unchanged |
| 2026-10-12 | minor | DXF reads control-point SPLINE closure (#1299 follow-up after #1320); open curves stay open and curves retained; rational/non-rational import and export versions regression-tested; scores/hours unchanged |
| 2026-10-12 | minor | Document Setup: Edit Artboards closes the modal after applying valid settings; pointer-click regressions; scores and hours unchanged |
| 2026-10-12 | minor | Artboards: Escape exits editing; an active drag cancels first; shortcut-routing regression; scores and hours unchanged |
| 2026-10-12 | minor | Simplify previews and shows original/new point counts; refits/subdivisions select changed paths whole (#1272); auto/slider/corner-angle UI and Show Original remain partial |
| 2026-10-12 | minor | Texture-effect drag redraws optimized (#1257); idle Windows interaction still unverified; scores unchanged |
| 2026-10-12 | minor | Curvature: circular point markers, no direction handles or duplicate points; existing-path interpolation and #975 remain separate (#1236, in part); scores unchanged (estimated) |
| 2026-10-12 | minor | Direct Selection: repeating anchor Align and rounding-only Align leave Undo, Redo and saved state intact (#1180 follow-up); scores unchanged (estimated) |
| 2026-10-12 | minor | Eyedropper: original pipette at the sampling tip; precise setting and DPI checked (#1269); OS visual QA and Caps Lock unverified; scores unchanged |
| 2026-10-12 | minor | Eyedropper can read visible locked and outside-isolation sources (#1251); scores unchanged |
| 2026-10-12 | minor | Direct Selection: segment click/display and Delete preserve neighboring edges (#1278); pointer/egui/native checks, OS visual QA pending; score unchanged |
| 2026-10-12 | minor | Adding paint to an unpainted zero-width stroke restores the 1 pt default (#1253); scores unchanged |
| 2026-10-12 | minor | macOS native save sheets gain a localised Format menu (#866); filename regression tests and an actual SVG save, PDF selection and cancellation verified; scores unchanged (estimated) |
| 2026-10-12 | minor | Attributes: directly selected compound members read and change the compound fill rule; path direction stays member-local (#1197); score unchanged |
| 2026-10-12 | minor | Pathfinder Divide: open cutting lines split filled paths with one Undo (#1307); scores and hours unchanged |
| 2026-10-12 | minor | Numeric entry: click-to-focus selection waits for release so text drags keep their start (#1326, in part); keyboard focus, double-click, arrows and wheel preserved; scores and hours unchanged; short movements below the click threshold remain open |
| 2026-10-12 | minor | Double Click To Isolate: the Selection tool also isolates a single path, compound path, image, mesh or placed file; only it is hit, marqueed and selected inside; scores and hours unchanged |
| 2026-10-11 | minor | A marquee inside a filled shape no longer selects it: only touching its path does (#1103); score unchanged (estimated) |
| 2026-10-11 | minor | Live Corners: corners between curved sides round too, the curve trimmed to what the cut leaves (#1144); score unchanged (estimated) |
| 2026-10-11 | minor | Artboards: Shift-click selects several, which move, copy and delete together (#1139); score unchanged (estimated) |
| 2026-10-11 | minor | Edit Toolbar opens the All Tools drawer beside the toolbar instead of a centred dialog, dims the tools the toolbar holds, and edits the toolbar by drag and drop while it is open (#1018); score unchanged (estimated) |
| 2026-10-11 | minor | Direct Selection: dragging a path selected whole by an anchor moves it whole; Alt-drag copies by an anchor too, toggles during the drag and snaps onto the original (#1104); score unchanged (estimated) |
| 2026-10-10 | major | First checklist, built from the tool sources and tests against Illustrator 2026's public documentation |
