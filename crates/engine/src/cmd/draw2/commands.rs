use super::anchors::{convert_anchor, curvature, curvature_edit, remove_anchor, remove_anchors, reshape_segment};
use super::cut::{erase_region, erase_segments, knife, split};
use super::freehand::{blob, freehand};
use super::quickpen::{quick_pen, quick_pen_length};
use super::smooth::{join_scrub, smooth_region};
use super::{CommandSpec, cmd, has_anchors, has_doc, has_selection};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "path.freehand",
            "Pencil",
            [],
            None,
            "{points: [[x,y]…] or [[x,y,pressure]…] or [[x,y,pressure,tiltX,tiltY,rotation,wheel]…] (pen pressure 0..1: the stroke keeps it, for Calligraphic and Scatter brushes with Pressure variation; a point without one presses fully. Pen tilt in degrees, each −90..90, x positive leaning right and y leaning down the page, and barrel rotation in degrees clockwise, 0..360: the stroke keeps how far the pen leaned, which way and how far its barrel turned, for brushes with Tilt, Bearing and Rotation variation. An airbrush pen's finger wheel 0..1: the stroke keeps it for brushes with Stylus Wheel variation, a point without one half way; null for what a point lacks. A pressure, tilt, rotation or wheel outside its range, or a point array of more than seven numbers, is an error), fidelity?: pt (1.5; from 0.000015625 (0.01 / 640) to 100 / 0.0313 (about 3194.89), the Pencil's and the Paintbrush's 0.01 to 100 px Fidelity at the highest and the lowest zoom; a value outside that range is an error. The tools send their Fidelity in pixels divided by the zoom, kept within the range), fit?: bool (true; false: the points are joined by straight segments, as the Pencil previews a stroke with Live curve fitting off), closed?, style?: \"pencil\" (default) | \"brush\", fill?: bool, keepSelected?: bool (true: the drawn or continued path is selected; false: a new path leaves nothing selected, and a continued or redrawn one is deselected while the rest of the selection stays), leaveSelection?: bool (false; true with keepSelected false: a new path leaves the selection as it is, as the Pencil previews a stroke that redraws a path), extend?: {id, end?: \"end\" (default) | \"start\"}, connect?: {id, end: \"start\"|\"end\"} (only with extend), redraw?: {id, within?: pt (12)}} fit a freehand stroke → {id}. extend continues the path from that end, and closes it when `closed` is true (left out, when the stroke ends within four Fidelities of the path's other end, 6 to 400 pt); with connect, the stroke ends on that end of another path's last open subpath and joins it on, in one path with the extended path's appearance (the other path is removed when nothing of it is left); redraw (Edit Selected Paths) replaces the section of the path between where the stroke starts and ends, each within `within` of one subpath (the way round a closed one that leaves the bigger shape; on an open one the way the stroke runs along it at each end decides: it replaces the part it runs along where it starts and joins the part it runs into where it lands, so it replaces the part between, keeps it and closes the path along the stroke, or, when its ends disagree, ends where it is lifted; an end that leaves or meets the path squarely follows the other end, and when both do it closes only if it goes round both pieces beyond), the stroke's ends bent smoothly onto the path; a stroke that starts on it and returns to the same point adds a loop there, and one lifted farther than `within` from it takes the path with it: a closed subpath opens where the stroke starts, an open one is cut there and drops the part the stroke heads along, and the path ends where the stroke does. Any other style, extend end or connect end, connect without extend, and a connect that names no other path with an open end are errors, a stroke of one point included, and nothing changes",
            has_doc,
            freehand
        ),
        cmd!(
            "path.curvature",
            "Curvature",
            [],
            None,
            "{points: [{x, y, corner?}], closed?, id?} create (or with id: replace) a path curving through the points → {id}",
            has_doc,
            curvature
        ),
        cmd!(
            "path.curvatureEdit",
            "Curvature",
            [],
            None,
            "{id, subpath?, op: \"move\"|\"insert\"|\"extend\"|\"close\", anchor?, segment?, t?, end?: \"start\"|\"end\", from?: \"keep\"|\"smooth\"|\"corner\", corner?, x?, y?} edit a path as the Curvature tool does, its shape kept elsewhere; the anchor edited becomes the direct-selected one → {anchor}. move: anchor to x, y, re-curving only its two segments (a smooth anchor takes the Curvature tool's handles, along the circle through it and its neighbours); insert: a smooth anchor at t on segment, the shape unchanged (with x, y: then moved there); extend: an open subpath goes on from its end (default) or start to a new end at x, y, curving into it unless corner is true; close: an open subpath closes from that end. The old end bends into the new segment as from says: keep its curve (default), smooth through its neighbours, or a corner",
            has_doc,
            curvature_edit
        ),
        cmd!(
            "path.quickPen",
            "Quick Pen",
            [],
            None,
            "{start?: [x, y] (a new path starts there), id? (or: draw on this path), subpath?: index (0), keep?: count (default: all its points; the points after the first `keep` are replaced), pieces?: [{x, y, action?: \"line\" (default)|\"continuousArc\"|\"sharpArc\", turn?: 1..359 degrees (180), flip?: bool}] (at most 10000), closed?: bool} draw a path of straight lines and circular arcs as the Quick Pen tool does, each piece from the path's last point to x, y: a line; a continuous arc, which leaves its start in the direction the path arrives in (with no direction, the first piece of a path, it is drawn as a sharp-corner arc; with its end straight ahead or straight behind, as a line); a sharp-corner arc, which turns through `turn` degrees and bulges to the left of the direction drawn (with flip, to the right). A piece that ends where the path does adds nothing. closed: a last piece that ends on the first point joins it, otherwise a straight segment closes the path. A new path takes the current paint (a black stroke when it has neither fill nor stroke); the path is selected, as one undo step → {id}. Without id, `start` must be given; with id, the subpath must be open and `start` is refused",
            has_doc,
            quick_pen
        ),
        cmd!(
            "path.quickPenLength",
            "Quick Pen",
            [],
            None,
            "{id, subpath?: index (0), end?: \"end\" (default)|\"start\", length: pt} make the line or arc at that end of the open subpath `length` long, as dragging the Quick Pen tool's length handle does: a line keeps its other point and its direction; an arc (the segments before the end that lie on one circle and turn the same way) keeps its other point, its circle and the way it turns, with a sweep of at most 359°. length runs from 0.001 to 1000000 pt for a line, and to 359° of the circle for an arc. A length the piece already has changes nothing; otherwise the path is selected, as one undo step → {id, kind: \"line\"|\"arc\", length}. An end that is neither straight nor a circular arc, a closed subpath, a locked or hidden path, a length outside the range and a result that would reach past the canvas are errors",
            has_doc,
            quick_pen_length
        ),
        cmd!(
            "path.removeAnchor",
            "Delete Anchor Point",
            [],
            None,
            "{id, subpath?, anchor, curvature?} remove one anchor, re-fitting the curve (with curvature: true, the curve where it was follows the remaining anchors, as the Curvature tool curves its points)",
            has_doc,
            remove_anchor
        ),
        cmd!(
            "path.removeAnchors",
            "Remove Anchor Points",
            ["Object", "Path"],
            None,
            "{} remove the direct-selected anchors without opening their paths, refitting the curve round each; the anchor after each removed one is selected, so running it again goes on along the path → {removedObjects}",
            has_anchors,
            remove_anchors
        ),
        cmd!(
            "path.convertAnchor",
            "Convert Anchor Point",
            [],
            None,
            "{id, subpath?, anchor, to: \"corner\"|\"smooth\", x?, y?} (smooth with x/y: out handle there, in handle mirrored; without: as the Curvature tool curves a point, along the circle through it and its neighbours, or at an open end one handle into the curve)",
            has_doc,
            convert_anchor
        ),
        cmd!(
            "path.reshapeSegment",
            "Reshape Segment",
            [],
            None,
            "{id, subpath?, segment, t, dx, dy, perpendicular?: bool} bend a segment so the point at t moves by (dx, dy); with perpendicular (Shift), both handles stand at right angles to the line between its anchors at one length, and the point at t lies as far from that line as the moved point does",
            has_doc,
            reshape_segment
        ),
        cmd!(
            "path.split",
            "Split Path",
            [],
            None,
            "{id, subpath?, segment, t: 0–1 (0.5)} or {id, subpath?, anchor}: closed paths open there, open paths become two; one of the two new end points is left selected; the paths keep the source's fill and stroke settings except that every stroke's alignment becomes Center (in a path of several subpaths, the other subpaths' strokes too) → {ids}. A t outside 0–1 is refused with an error, and nothing changes",
            has_doc,
            split
        ),
        cmd!(
            "path.knife",
            "Knife",
            [],
            None,
            "{points: [[x,y]…] (pen values after x and y are checked as path.freehand checks them and do not change the cut)} cut closed shapes (selected, or all when nothing is selected) along the polyline → {ids}",
            has_doc,
            knife
        ),
        cmd!(
            "path.eraseRegion",
            "Eraser",
            [],
            None,
            "{points: [[x,y]…] or [[x,y,pressure]…] or [[x,y,pressure,tiltX,tiltY,rotation]…] or [[x,y,pressure,tiltX,tiltY,rotation,wheel]…] (pen pressure 0..1, tilt −90..90°, barrel rotation 0..360° clockwise, finger wheel 0..1; pen values are checked as path.freehand checks them), size?: pt (10; from 0.1 to 1000), angle?: ° (0, −180–180: the tip's long axis counter-clockwise from level), roundness?: % (100, 1–100), angleMode?, roundnessMode?, sizeMode?: \"fixed\" (default) | \"random\" | \"pressure\" | \"tilt\" | \"bearing\" | \"rotation\" | \"stylusWheel\" (Eraser Tool Options' variation menus; a varied angle turns either way, clockwise or counter-clockwise), angleVariation?: ° (0–180, default 0, either way: clockwise or counter-clockwise), roundnessVariation?: % (0–100, default 0), sizeVariation?: pt (0–1000, default 0)} or {rect: [x, y, width, height] (pt; the Eraser's Alt-drag marquee)} erase the stroke of an elliptical tip from paths (selected, or all) → {ids}. The tip is `size` across its long axis, which runs at `angle` counter-clockwise from horizontal; its short axis is `roundness` percent of the long one. A value whose mode is not fixed varies by up to its variation either way: random draws one value for the drag; pressure goes from the value less the variation where the pen presses lightest to the value plus it where it presses hardest (size on pressure as path.blob); tilt from less with the pen lying flat to more with it upright; bearing and rotation from less at −180° to more at 180° of the pen's lean direction or barrel turn; stylusWheel from less at one end of an airbrush pen's finger wheel travel to more at the other. A drag without that pen reading (a mouse) erases at the value itself. An angle or roundness outside its range, an unknown mode, or a variation below 0 or past its range is refused with an error and erases nothing. `rect` in place of `points` erases everything inside the rectangle: closed paths and filled open paths without a visible painted stroke lose its area, and other open paths are cut where they cross its edges; the tip's size, angle, roundness and variations do not apply. A width or height of 0 or less, a value that is not a finite number, or `points` and `rect` together is an error. When nothing is erased, no undo step is added",
            has_doc,
            erase_region
        ),
        cmd!(
            "path.eraseSegments",
            "Path Eraser",
            [],
            None,
            "{points: [[x,y]…] (pen values after x and y are checked as path.freehand checks them and do not change what is erased), width?: pt (8; from 0.001 to 1000)} erase the parts of selected paths under the drag → {ids}. A width outside 0.001 to 1000 is refused with an error, and nothing changes",
            has_selection,
            erase_segments
        ),
        cmd!(
            "path.smoothRegion",
            "Smooth",
            [],
            None,
            "{points: [[x,y]…] (pen values after x and y are checked as path.freehand checks them and do not change the smoothing), radius?: pt (12; from 0.001 to 1000), fidelity?: pt (2.5; from 0.000078125 to about 3195, the Smooth tool's 0.05 to 100 px at the highest and the lowest zoom: the tool sends its Fidelity in screen pixels divided by the zoom, kept within the range)} smooth selected paths near the drag, keeping a corner where the path turns by more than 57.5° at the anchor and over radius and twice radius on either side, and leaving a subpath as it is when the result would not be smoother (docs/mcp.md) → {ids, before, after}. A radius or fidelity outside its range is refused with an error, and nothing changes",
            has_selection,
            smooth_region
        ),
        cmd!(
            "path.joinScrub",
            "Join",
            [],
            None,
            "{points: [[x,y]…] (pen values after x and y are checked as path.freehand checks them and do not change the join), tolerance?: pt (12; from 0.001 to 1000)} join the open-path endpoints scrubbed over, or trim two crossing open paths whose overhanging ends are scrubbed and join them at the crossing → {id}. A tolerance outside 0.001 to 1000 is refused with an error, and nothing changes",
            has_doc,
            join_scrub
        ),
        cmd!(
            "path.blob",
            "Blob Brush",
            [],
            None,
            "{points: [[x,y]…] or [[x,y,pressure]…] (pen pressure 0..1; pen values are checked as path.freehand checks them), size?: pt (10; from 0.1 to 1000), sizeMode?: \"fixed\" (default) | \"pressure\" (Blob Brush Tool Options' Size set to Pressure: the brush is size − sizeVariation wide at the lightest pen pressure and size + sizeVariation at the heaviest, at least 0.1 pt; a stroke without pressure keeps the size), sizeVariation?: pt (0; from 0 to 1000, checked with Fixed too), angle?: ° (0, −180–180: the tip's long axis counter-clockwise from horizontal, kept on the page along the stroke), roundness?: % (100, 0–100: the tip's short axis in percent of its long one, 100 round; 0 is drawn as 0.1), fidelity?: pt (0.01–100; the brush follows the stroke fitted at this tolerance as path.freehand fits a Pencil stroke, and a higher value gives a smoother shape; without it the brush follows every point), merge?: true, mergeOnlySelection?: bool (false: merge with any touching same-color blob; true: only with selected ones), keepSelected?: bool (false: the drawn or merged path is not selected, and the rest of the selection stays; true: it is selected)} filled brush shape, merged with the topmost touching same-color blob and the touching ones next to it in stacking order in its group or layer → {id}. An angle, roundness or fidelity outside its range is rejected with an error, and nothing changes",
            has_doc,
            blob
        ),
    ]
}
