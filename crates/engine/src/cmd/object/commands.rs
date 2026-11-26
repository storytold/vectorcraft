use super::align::{align, can_distribute, distribute, distribute_spacing};
use super::arrange::{Arrange, arrange, exit_isolation, group, hide, isolate, lock, new_group, send_to_current_layer, show_all, ungroup, unlock_all};
use super::bounds::set_bounds;
use super::compound::{clip_edit, clip_make, clip_release, compound_make, compound_release};
use super::pixelgrid::set_pixel_aligned;
use super::props::set_props;
use super::shape::{expand_shape, set_live_shape};
use super::transform::{move_cmd, nudge, reflect, rotate, scale, shear, transform, transform_again};
use super::{CommandSpec, cmd, has_doc, has_multi, has_selection, has_selection_or_guides};
use crate::cmd::has_selected_member_or_guide;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "object.transform",
            "Transform",
            [],
            None,
            "{matrix: [a,b,c,d,e,f], copy?: bool, ids?, strokes?: bool, corners?: bool, typeAreas?: bool, anchors?: bool, objects?: bool (Transform Objects, default true; false: the art stays and, with patterns, only its pattern tiles transform), patterns?: bool (Transform Patterns: pattern fills and strokes transform with the art; default: prefs transformPatternTiles), releaseOutside?: bool, interpolate?: bool (with anchors and a matrix that only moves them: the handles of each segment with one moved end are re-mapped, as path.moveAnchors's; default: prefs interpolateControlPoints), drawBehind?: bool (with copy: in Draw Behind, view.drawMode, each copy goes directly below its original instead of above it, as the Selection and Direct Selection tools' Alt-drags send; default false)} apply an affine to the selection (or ids); anchors (without ids or copy): paths with anchors selected with Direct Selection have only those anchors and their handles transformed, as the Transform tools and object.move/rotate/scale/reflect/shear do (default: the whole objects); strokes/corners: Scale Strokes & Effects / Scale Corners (default: the preferences); typeAreas: area type among them (not type inside a group) reshapes its frame by the matrix and its text reflows at its size, as a bounding-box handle drag does (default: the type transforms too); releaseOutside (without copy or anchors): a Shaper Group's original shape that ends wholly outside the bounds the group's original shapes had before takes shaper.releaseSource in the same undo step, named Remove from Shaper Group, as the Selection tool's moves in Construction mode do",
            has_doc,
            transform
        ),
        cmd!(
            "object.move",
            "Move…",
            ["Object", "Transform"],
            Some("Cmd+Shift+M"),
            "{ids?: [id…] (selected first; default: the selection), dx, dy, copy?, objects?: bool (Transform Objects, default true; false: the art stays and, with patterns, only its pattern tiles transform), patterns?: bool (Transform Patterns: pattern fills and strokes transform with the art; default: prefs transformPatternTiles), typeAreas?: bool (area type: the frame is reshaped and the text reflows at its size), interpolate?: bool (as path.moveAnchors's; default: prefs interpolateControlPoints)}. With anchors selected with Direct Selection (and no copy), only those anchors and their handles transform, about the centre of their own box.",
            has_selection_or_guides,
            move_cmd
        ),
        cmd!(
            "object.rotate",
            "Rotate…",
            ["Object", "Transform"],
            None,
            "{ids?: [id…] (selected first; default: the selection), angle: deg (counter-clockwise), absolute?: bool (angle is the bounding box's new angle, not an amount), origin?: [x,y] (default: the bounding box centre), copy?, objects?: bool (Transform Objects, default true; false: the art stays and, with patterns, only its pattern tiles transform), patterns?: bool (Transform Patterns: pattern fills and strokes transform with the art; default: prefs transformPatternTiles), typeAreas?: bool (area type: the frame is reshaped and the text reflows at its size)} → {ids}. With anchors selected with Direct Selection (and no copy), only those anchors and their handles transform, about the centre of their own box. Selected ruler guides that can be edited rotate too (without ids), each as the line view.guides.release makes of it, and the default origin is the center of the objects' box and those lines together: a guide that ends up vertical or horizontal stays a ruler guide, and one at any other angle becomes a selected guide path along its line. absolute with only ruler guides selected is an error.",
            has_selection_or_guides,
            rotate
        ),
        cmd!(
            "object.scale",
            "Scale…",
            ["Object", "Transform"],
            None,
            "{ids?: [id…] (selected first; default: the selection), sx: % (along the bounding box's width), sy?: % (along its height), origin?: [x,y] (a page point), basis?: \"box\"|\"page\" (box, the default: sx and sy follow the bounding box, turned with rotated objects until Reset Bounding Box; page: the page's axes; a saved step without basis, recorded before it, plays along the page's axes), copy?, strokes?: bool (Scale Strokes & Effects: stroke weights, dashes and effect distances scale; off keeps them, type strokes included), corners?: bool (Scale Corners: live corner radii scale), objects?: bool (Transform Objects, default true; false: the art stays and, with patterns, only its pattern tiles transform), patterns?: bool (Transform Patterns: pattern fills and strokes transform with the art; default: prefs transformPatternTiles), typeAreas?: bool (area type: the frame is reshaped and the text reflows at its size)} (strokes/corners default to the preferences). With anchors selected with Direct Selection (and no copy), only those anchors and their handles transform, about the centre of their own box.",
            has_selection,
            scale
        ),
        cmd!(
            "object.reflect",
            "Reflect…",
            ["Object", "Transform"],
            None,
            "{ids?: [id…] (selected first; default: the selection), axis: \"vertical\"|\"horizontal\"|deg (the mirror line, counter-clockwise from horizontal; 0: horizontal; omitted: vertical; vertical and horizontal are the bounding box's own axes, deg a line on the page), origin?, basis?: \"box\"|\"page\" (for a named axis; box, the default: the bounding box's axes, turned with rotated objects until Reset Bounding Box; page: the page's axes; a saved step without basis, recorded before it, plays along the page's axes), copy?, objects?: bool (Transform Objects, default true; false: the art stays and, with patterns, only its pattern tiles transform), patterns?: bool (Transform Patterns: pattern fills and strokes transform with the art; default: prefs transformPatternTiles), typeAreas?: bool (area type: the frame is reshaped and the text reflows at its size)}. With anchors selected with Direct Selection (and no copy), only those anchors and their handles transform, about the centre of their own box.",
            has_selection,
            reflect
        ),
        cmd!(
            "object.shear",
            "Shear…",
            ["Object", "Transform"],
            None,
            "{ids?: [id…] (selected first; default: the selection), angle: deg (-359 to 359, sheared by its tangent, so 120 slants as -60; 0 and ±180 have no slant and, without copy, change nothing; ±90 and ±270 are an error; positive turns the vertical edges clockwise with the horizontal axis, the top to the right; the horizontal edges clockwise with the vertical axis, the right side down), axis?: \"horizontal\"|\"vertical\"|deg (the line the art slides along, counter-clockwise from horizontal on the page, -359 to 359; 0: horizontal, 90: vertical; omitted: horizontal), origin?, copy?, objects?: bool (Transform Objects, default true; false: the art stays and, with patterns, only its pattern tiles transform), patterns?: bool (Transform Patterns: pattern fills and strokes transform with the art; default: prefs transformPatternTiles), typeAreas?: bool (area type: the frame is reshaped and the text reflows at its size)}. With anchors selected with Direct Selection (and no copy), only those anchors and their handles transform, about the centre of their own box.",
            has_selection,
            shear
        ),
        cmd!(
            "object.transformAgain",
            "Transform Again",
            ["Object", "Transform"],
            Some("Cmd+D"),
            "{copy?, strokes?, corners?, patterns?, typeAreas?, interpolate?: bool (as object.transform's; default: the value the transform it repeats used, else object.transform's default)} the last transform's matrix on the selection (after a perspective move or scale: that plane-space transform, without these params)",
            has_selection_or_guides,
            transform_again
        ),
        cmd!(
            "object.nudge",
            "Nudge",
            [],
            None,
            "{dx: -1|0|1, dy: -1|0|1, big?: bool (×10), copy?: bool, patterns?: bool (Transform Patterns: pattern fills and strokes transform with the objects that move whole; default: prefs transformPatternTiles), typeAreas?: bool (area type: the frame is reshaped and the text reflows at its size), objects?: bool (Transform Objects, as object.move), interpolate?: bool (as path.moveAnchors's; default: prefs interpolateControlPoints)} arrow-key nudge by the keyboard increment (the selected anchors (a selected curved segment bends instead: its anchors stay and its handles keep their directions while its middle moves by the step), else objects, along the x and y axes turned by the constrainAngle preference; else ruler guides, along the page axes)",
            has_selection_or_guides,
            nudge
        ),
        cmd!(
            "object.arrange.bringToFront",
            "Bring to Front",
            ["Object", "Arrange"],
            Some("Cmd+Shift+]"),
            "{ids?: [id…] (selected first; default: the selection)}",
            has_selection,
            |s, _| arrange(s, Arrange::Front)
        ),
        cmd!(
            "object.arrange.bringForward",
            "Bring Forward",
            ["Object", "Arrange"],
            Some("Cmd+]"),
            "{ids?: [id…] (selected first; default: the selection)}",
            has_selection,
            |s, _| arrange(s, Arrange::Forward)
        ),
        cmd!(
            "object.arrange.sendBackward",
            "Send Backward",
            ["Object", "Arrange"],
            Some("Cmd+["),
            "{ids?: [id…] (selected first; default: the selection)}",
            has_selection,
            |s, _| arrange(s, Arrange::Backward)
        ),
        cmd!(
            "object.arrange.sendToBack",
            "Send to Back",
            ["Object", "Arrange"],
            Some("Cmd+Shift+["),
            "{ids?: [id…] (selected first; default: the selection)}",
            has_selection,
            |s, _| arrange(s, Arrange::Back)
        ),
        cmd!(
            "object.arrange.sendToCurrentLayer",
            "Send to Current Layer",
            ["Object", "Arrange"],
            None,
            "{ids?: [id…] (selected first; default: the selection)}",
            has_selection,
            send_to_current_layer
        ),
        cmd!(
            "object.group",
            "Group",
            ["Object"],
            Some("Cmd+G"),
            "{ids?: [id…] (selected first; default: the selection)} → {id}",
            has_selection,
            group
        ),
        cmd!(
            "object.newGroup",
            "Group",
            [],
            None,
            "{parent?: layer or group id (default: the current layer), index?: n (0 = bottom, or directly above the clipping path in a clipping layer or group; default: on top), compound?: bool (default false; true: an empty compound path instead of a group)} → {id}: an empty group or compound path in a shown, unlocked layer or group; the selection stays as it is",
            has_doc,
            new_group
        ),
        cmd!(
            "object.ungroup",
            "Ungroup",
            ["Object"],
            Some("Cmd+Shift+G"),
            "{ids?: [id…] (selected first; default: the selection)}",
            has_selection,
            ungroup
        ),
        cmd!(
            "object.lock",
            "Selection",
            ["Object", "Lock"],
            Some("Cmd+2"),
            "{ids?: [id…] (selected first; default: the selection, selected ruler guides included; in isolation mode, only the objects inside the isolated object)}",
            has_selected_member_or_guide,
            lock
        ),
        cmd!(
            "object.unlockAll",
            "Unlock All",
            ["Object"],
            Some("Cmd+Alt+2"),
            "{scope?: \"document\"|\"group\" (default document, which in isolation mode is the isolated object; group: only the objects inside the groups that hold the selected objects, a selected group holding its own; the in-window menu item chosen with Shift+Alt)} unlock the locked objects and select them; document scope outside isolation mode also unlocks ruler guides locked on their own → {count}",
            has_doc,
            unlock_all
        ),
        cmd!(
            "object.hide",
            "Selection",
            ["Object", "Hide"],
            Some("Cmd+3"),
            "{ids?: [id…] (selected first; default: the selection, selected ruler guides included; in isolation mode, only the objects inside the isolated object)}",
            has_selected_member_or_guide,
            hide
        ),
        cmd!(
            "object.showAll",
            "Show All",
            ["Object"],
            Some("Cmd+Alt+3"),
            "{scope?: \"document\"|\"group\" (default document, which in isolation mode is the isolated object; group: only the objects inside the groups that hold the selected objects, a selected group holding its own; the in-window menu item chosen with Shift+Alt)} show the hidden objects and select them; document scope outside isolation mode also shows ruler guides hidden on their own → {count}",
            has_doc,
            show_all
        ),
        cmd!(
            "object.compoundPath.make",
            "Make",
            ["Object", "Compound Path"],
            Some("Cmd+8"),
            "{ids?: [id…] (selected first; default: the selection)}",
            has_selection,
            compound_make
        ),
        cmd!(
            "object.compoundPath.release",
            "Release",
            ["Object", "Compound Path"],
            Some("Cmd+Alt+Shift+8"),
            "{ids?: [id…] (selected first; default: the selection)} release the selected compound paths into their paths, each with the compound path's paint; a path of several subpaths (an SVG `d` with several `M`s) splits into one path per subpath",
            has_selection,
            compound_release
        ),
        cmd!(
            "object.clippingMask.make",
            "Make",
            ["Object", "Clipping Mask"],
            Some("Cmd+7"),
            "{ids?: [id…] (selected first; default: the selection)} the topmost selected object (a path, compound path or text, which loses its paint) clips the others: compound holes, even-odd fills and glyph outlines clip as drawn. Paint given to the clipping path later (paint commands with its id) shows: its fill behind the clipped art, its stroke over it → {id} of the clip group",
            has_multi,
            clip_make
        ),
        cmd!(
            "object.clippingMask.release",
            "Release",
            ["Object", "Clipping Mask"],
            Some("Cmd+Alt+7"),
            "{ids?: [id…] (selected first; default: the selection)} the selected clip groups become plain groups; their clipping path (path, compound path or text) stays, with the paint it has (none unless painted after Make)",
            has_selection,
            clip_release
        ),
        cmd!(
            "object.clippingMask.editContents",
            "Edit Contents",
            ["Object", "Clipping Mask"],
            None,
            "{} select the clipped art of the selected clip groups → {count}",
            has_selection,
            |s, _| clip_edit(s, false)
        ),
        cmd!(
            "object.clippingMask.editMask",
            "Edit Clipping Path",
            ["Object", "Clipping Mask"],
            None,
            "{} select the clipping paths of the selected clip groups → {count}",
            has_selection,
            |s, _| clip_edit(s, true)
        ),
        cmd!(
            "object.isolate",
            "Enter Isolation Mode",
            [],
            None,
            "{id?} isolate a group, a layer or a single object (default: the first selected object); type, a ruler guide, a hidden or locked object, an object in a hidden or locked group or layer, and a guide path while Lock Guides is on are refused, and so is every object while an opacity mask is edited",
            crate::cmd::maskedit::outside_mask_editing,
            isolate
        ),
        cmd!(
            "object.exitIsolation",
            "Exit Isolation Mode",
            [],
            None,
            "{all?: bool (default false)} leave isolation mode; from an isolated sublayer, go up one level to its parent layer instead (the next call goes up again), unless all is true or the parent layer is hidden or locked; no undo step for the move up; while a symbol is edited, a group isolated in its art goes back to the symbol's layer first, with or without all; leaving an isolated Live Paint group adds the paths, compound paths and groups of them drawn in it (shown and unlocked with everything in them) to the group as livePaint.merge does, in one undo step, and leaves type, images and other objects in it as they are; object.isolate on a level outside the group does the same. Leaving a Live Paint group drawn in another isolated Live Paint group, and that group too, does this for each, innermost first, one undo step each",
            has_doc,
            exit_isolation
        ),
        cmd!(
            "object.setProps",
            "Object Properties",
            [],
            None,
            "{ids?|id?, name?, visible?, locked? (visible: false or locked: true deselects every selected object hidden or locked after the call, the key object included), opacity?: 0..100, blend?: \"Multiply\"…, isolate?, knockout?: \"on\"|\"off\"|\"neutral\"|bool (true = on, false = neutral), knockoutShape?: bool, data?: {key: \"value\" | null (removes it)} (the object's own data, SVG data-* attributes: {pivot: \"100,180\"} is data-pivot; document.node → attrs.data)}. On a path directly in a graph series, the graph also stores the object's opacity and blend mode, and on a series group the group's opacity, blend mode, isolate, knockout and knockoutShape (exceptions: docs/mcp.md); on a graph label, or a graph's axis line, tick mark or radar ring, the graph stores that object's opacity and blend mode for every label or line of its kind. A path directly in a Live Paint group can't take the name Face or Edge, which name the group's faces and edges, unless it has it already",
            has_doc,
            set_props
        ),
        cmd!(
            "object.align",
            "Align",
            ["Window", "Align"],
            None,
            "{horizontal?: \"left\"|\"center\"|\"right\", vertical?: \"top\"|\"center\"|\"bottom\", to?: \"selection\"|\"artboard\"|\"key\"|\"auto\" (\"auto\": resolve at each call from the current selection: the key object, key anchor or key guide while set, else the artboard for exactly one selected object without selected anchors, else the selection; default: the key object when the selection has one, select.key, else the selection; a key guide, select.key {guide}, stays put and the objects line up on its line: a vertical guide sets where left, center and right go and vertical alignment leaves them, a horizontal one the other way round; aligning to the key, the other selected ruler guides move too: a vertical guide to the key's left edge, center or right edge, a horizontal one to its top, center or bottom; aligning to the artboard moves selected guides to its edges too (#3463), and with only guides selected aligning to the selection moves them to their collective bounds), artboard?: index (0-based; with to: \"artboard\", or \"auto\" choosing the artboard, the artboard to align to; the app passes the active one; default: the artboard under the center of the selection, else the first), bounds?: \"preview\"|\"geometric\" (default: the Use Preview Bounds preference; preview bounds take in strokes, geometry effects and projected 3D art), patterns?: bool (Transform Patterns: pattern fills and strokes transform with the art; default: prefs transformPatternTiles)} With anchor points selected (Direct Selection), the points align instead: to their own bounds, to the artboard (#1180), or, aligning to the key with a key anchor (select.key {id, anchor}), to the key anchor, which stays put",
            has_selection_or_guides,
            align
        ),
        cmd!(
            "object.distribute",
            "Distribute",
            ["Window", "Align"],
            None,
            "{horizontal?: \"left\"|\"center\"|\"right\", vertical?: \"top\"|\"center\"|\"bottom\", spacing?: pt (used with a key, select.key: the key object, or the key guide, select.key {guide}, stays put and the lines are placed this far apart, outward from its line, in their order on that line; 0 puts them all on its line; a key guide along the axis leaves everything where it is; not used without a key, or with anchor points selected and no key anchor), to?: \"selection\"|\"artboard\"|\"key\" (default: the key while there is one, else the selection; \"key\" without spacing distributes as \"selection\" does; with anchor points selected, with \"selection\" or \"artboard\" the key anchor is not the key and all the points are spaced as without one; not used with spacing and a key object; with a key guide, \"selection\" or \"artboard\" distributes without it as the key), artboard?: index (0-based; with to: \"artboard\", the artboard to distribute across; the app passes the active one; default: as object.align's), bounds?: \"preview\"|\"geometric\" (default: the Use Preview Bounds preference; preview bounds take in strokes, geometry effects and projected 3D art), patterns?: bool (Transform Patterns: pattern fills and strokes transform with the art; default: prefs transformPatternTiles)} space the selected objects' left edges, centers or right edges (top edges, centers or bottom edges) in their order on that line, with the selected ruler guides across the axis (vertical ones for horizontal; #1597): a guide has no size, so its edges and center are its line, and guides along the axis stay. Without spacing and a key they are spaced evenly, the first and last stay put, and fewer than three don't move; with spacing and a key, see spacing (two are enough); with to: \"artboard\", the first one's left (top) edge goes to the artboard's left (top) edge, the last one's right (bottom) edge goes to the artboard's right (bottom) edge, and two are enough. With anchor points selected (Direct Selection), the points are spaced evenly between the first and the last along the axis instead (#1636); with spacing and a key anchor (select.key {id, anchor}), the key anchor stays put and the others are placed spacing apart, outward from it, in their order along the axis",
            can_distribute,
            distribute
        ),
        cmd!(
            "object.distributeSpacing",
            "Distribute Spacing",
            ["Window", "Align"],
            None,
            "{axis?: \"horizontal\"|\"vertical\" (default: \"horizontal\"), spacing?: pt (the key object, select.key, or the key guide, select.key {guide}, stays put and the others are spaced from it; a key guide along the axis leaves everything where it is), to?: \"selection\"|\"artboard\"|\"key\" (used without spacing, and with a key guide; with anchor points selected, with \"selection\" or \"artboard\" the key anchor is not the key and all the points are spaced as without one; default: the key while there is one, else the selection; \"key\" without spacing spaces as \"selection\" does), artboard?: index (0-based; with to: \"artboard\", the artboard to space across; the app passes the active one; default: as object.align's), bounds?: \"preview\"|\"geometric\" (default: the Use Preview Bounds preference; preview bounds take in strokes, geometry effects and projected 3D art), patterns?: bool (Transform Patterns: pattern fills and strokes transform with the art; default: prefs transformPatternTiles)} without spacing, make the gaps between the selected objects equal, in the order of their left (top) edges, with the selected ruler guides across the axis as items with no size (#1597; guides along the axis stay): the first item's left (top) edge and the last one's right (bottom) edge stay put, or with to: \"artboard\" go to the artboard's left (top) and right (bottom) edges. With anchor points selected (Direct Selection), the points are spaced evenly along the axis instead, or `spacing` apart from the first (#1636), or with a key anchor (select.key {id, anchor}) `spacing` apart outward from the key anchor, which stays put",
            can_distribute,
            distribute_spacing
        ),
        cmd!(
            "object.setBounds",
            "Set Bounds",
            [],
            None,
            "{x?, y?, width?, height?, reference?: whole number >= 0 (9-point grid 0..8; 0.0 equals 0; a whole number past 8 is read as 8; omitted/null: center, 4), proportional?, strokes?, corners?, copy?: bool, patterns?: bool (Transform Patterns), typeAreas?: bool (area type: the frame is reshaped and the text reflows at its size), objects?: bool (Transform Objects, as object.move), interpolate?: bool (with anchor points selected and only x or y changed: as path.moveAnchors's; default: prefs interpolateControlPoints)} (Transform panel; width/height and the reference point follow the bounding box, rotated with rotated objects; x, y are page coordinates; with Use Preview Bounds the values measure the visual bounds; with the typeGlyphBounds preference, each point type object selected as an object (not inside a selected group) is measured by its glyphs' ink (underline and strikethrough included), which the scale gives the size asked for; strokes/corners as object.scale; with anchor points selected (Direct Selection) and no copy, the values are the box around those points, square to the page, and only they move or scale, as with object.move (#1824))",
            has_selection,
            set_bounds
        ),
        cmd!("object.expandShape", "Expand Shape", ["Object", "Shape"], None, "{} convert live shapes to plain paths", has_selection, expand_shape),
        cmd!(
            "object.setLiveShape",
            "Live Shape Properties",
            [],
            None,
            "{id?, ids?, radius?: pt (or % of each corner's maximum when `relativeRadii` is true), kind?: \"round\"|\"invertedRound\"|\"chamfer\", relativeRadii?: bool (Relative rounding, as in the reference app: store radii as fractions of each corner's maximum so they scale with the shape), corners?: [i…], rectanglesOnly?: bool (only the live rectangles among the targets; the other objects stay as they are), items?: [{id, corners?: [i…]}] (several objects, each with its own corners, in place of ids and corners: the canvas's corner widgets across a selection), sides?: n, polygonAngle?: degrees (counter-clockwise from its first vertex straight up), polygonRadius?: pt (centre to vertex), sideLength?: pt (sets the radius), makeSidesEqual?: true (a polygon scaled unevenly: drops the uneven scale and shear, keeping its centre, angle and mean radius), pieStart?, pieEnd?: degrees (an ellipse's pie, counter-clockwise from 3 o'clock; 0 to 360 is the whole ellipse), invertPie?: true (swaps them: the other part of the ellipse), constrainPie?: bool (Constrain Pie Angle: with one of pieStart and pieEnd, not both and not with invertPie, the other angle moves by as much, so each ellipse keeps its span), lineLength?: pt (a live line's length, above 0; its first end stays), lineAngle?: degrees (a live line's angle, counter-clockwise from 3 o'clock, from its first end to its second; it turns about its midpoint), rectangleAngle?: degrees (a live rectangle's, counterclockwise from level), ellipseAngle?: degrees (a live ellipse's, counterclockwise from level; its pie turns with it)} (rectangleAngle, ellipseAngle and polygonAngle turn each shape of that kind about its own center and leave other objects as they are; a pixel-aligned shape then goes back on the pixel grid, as after other edits, which can move its center or change its size; an angle that is not a number is an error, and `null` counts as an omitted key; a shape that a turn would take past the coordinate limit (±4,000,000 pt) is an error that changes nothing; Live Corners on any path: radius and kind set its corners (anchors that aren't smooth, where two sides, straight or curved (#1144), meet at an angle): `corners` (anchor indices of the path with its corners uncut, counting every subpath's anchors in order: a rectangle's 0 top-left, 1 top-right, 2 bottom-right, 3 bottom-left; a polygon's from the first vertex clockwise), else the corners with a Direct-Selected anchor, else every corner; each radius is drawn no larger than half the corner's shorter side allows; a path that isn't a live shape keeps its uncut outline so its corners stay editable, and is plain again once none is cut; sides: a polygon's, which keep the radius they shared; the polygon params apply in that order: sides, makeSidesEqual, polygonAngle, then sideLength or else polygonRadius; a line's lineLength applies before its lineAngle)",
            has_selection,
            set_live_shape
        ),
        cmd!(
            "object.setPixelAligned",
            "Align to Pixel Grid",
            [],
            None,
            "{ids?|id?, on?: bool (default true)} → {ids (the objects whose property changed)} (Transform panel › Align to Pixel Grid: on, each object's horizontal and vertical straight segments move to whole points, or half points when a visible, painted centered stroke has an odd weight, its stroke weights become whole points (at least 1), and every later move, scale, rotation, reflection, Transform Each, Align move, Transform panel change, move of direct-selected anchors or of one anchor (path.moveAnchor), path.setAnchors, object.setLiveShape, move of a compound path's path on its own, artboard move or resize that carries the art (artboard.move, artboard.setProps) or copies it (artboard.move with copy, Duplicate Artboards), every copy that edit.duplicate or a paste moves and object.slice.move of an object slice snaps them again, as does every stroke.set, stroke.weightUp or stroke.weightDown edit (which also rounds the new weight) and every appearance.setItem, appearance.addStroke, appearance.removeItem, appearance.clear, appearance.reduceToBasic, appearance.showAllHidden, appearance.transfer, appearance.copyFrom, paint.setStroke or graphicStyle.apply edit, and every other command that changes the object; diagonal segments and curves keep their shape apart from the anchors they share with those segments, and coordinates that differ stay at least 1 point apart; a live rectangle, polygon, line or pie moves and scales as a whole to the snapped bounds (a full ellipse, all curves, stays as it is), and a path with Live Corners snaps the outline its corners are cut from; a compound path snaps its paths with its own stroke, Make Compound Path from an aligned path or compound path gives an aligned compound path whose paths drop the property, and Release Compound Path gives each path the compound path's property; off, the object keeps its geometry and stops snapping. Only paths and compound paths can be aligned: type, images, groups, guides, paths inside a compound path and other objects are an error and nothing changes. One undo step; nothing changes and no undo step is made when every object already has the value. Saved in the native file)",
            has_selection,
            set_pixel_aligned
        ),
    ]
}
