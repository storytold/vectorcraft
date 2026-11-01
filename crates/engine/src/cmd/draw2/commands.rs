use super::anchors::{convert_anchor, curvature, curvature_edit, remove_anchor, remove_anchors, reshape_segment};
use super::cut::{erase_region, erase_segments, knife, split};
use super::freehand::{blob, freehand};
use super::smooth::{join_scrub, smooth_region};
use super::{CommandSpec, cmd, has_anchors, has_doc, has_selection};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "path.freehand",
            "Pencil",
            [],
            None,
            "{points: [[x,y]…] or [[x,y,pressure]…] (pen pressure 0..1: the stroke keeps it, for Calligraphic brushes with Pressure variation; a point without one presses fully), fidelity?: pt (1.5), fit?: bool (true; false: the points are joined by straight segments, as the Pencil previews a stroke with Live curve fitting off), closed?, style?: \"pencil\"|\"brush\", fill?: bool, keepSelected?: bool (true: the drawn or continued path is selected; false: it is not, and the rest of the selection stays), extend?: {id, end: \"start\"|\"end\"}, connect?: {id, end: \"start\"|\"end\"}, redraw?: {id, within?: pt (12)}} fit a freehand stroke → {id}. extend continues the path from that end; with connect, the stroke ends on that end of another path's last open subpath and joins it on, in one path with the extended path's appearance (the other path is removed when nothing of it is left); redraw (Edit Selected Paths) replaces the section of the path between where the stroke starts and ends, each within `within` of one subpath (the shorter way round a closed one), and draws a new path when the stroke doesn't end on it",
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
            "{id, subpath?, segment, t} or {id, subpath?, anchor}: closed paths open there, open paths become two → {ids}",
            has_doc,
            split
        ),
        cmd!(
            "path.knife",
            "Knife",
            [],
            None,
            "{points: [[x,y]…]} cut closed shapes (selected, or all when nothing is selected) along the polyline → {ids}",
            has_doc,
            knife
        ),
        cmd!(
            "path.eraseRegion",
            "Eraser",
            [],
            None,
            "{points: [[x,y]…] or [[x,y,pressure]…] (pen pressure 0..1), size?: pt (10), sizeMode?: \"fixed\" (default) | \"pressure\" (Eraser Tool Options' Size set to Pressure: the tip is size − sizeVariation across where the pen presses lightest and size + sizeVariation where it presses hardest, as path.blob), sizeVariation?: pt (0), angle?: ° (0), roundness?: % (100, kept within 1–100)} erase the stroke of an elliptical tip from paths (selected, or all) → {ids}. The tip is `size` across its long axis, which runs at `angle` counter-clockwise from horizontal; its short axis is `roundness` percent of the long one",
            has_doc,
            erase_region
        ),
        cmd!(
            "path.eraseSegments",
            "Path Eraser",
            [],
            None,
            "{points: [[x,y]…], width?: pt (8)} erase the parts of selected paths under the drag → {ids}",
            has_selection,
            erase_segments
        ),
        cmd!(
            "path.smoothRegion",
            "Smooth",
            [],
            None,
            "{points: [[x,y]…], radius?: pt (12), fidelity?: pt (2.5)} smooth selected paths near the drag → {ids, before, after}",
            has_selection,
            smooth_region
        ),
        cmd!(
            "path.joinScrub",
            "Join",
            [],
            None,
            "{points: [[x,y]…], tolerance?: pt (12)} join the open-path endpoints scrubbed over → {id}",
            has_doc,
            join_scrub
        ),
        cmd!(
            "path.blob",
            "Blob Brush",
            [],
            None,
            "{points: [[x,y]…] or [[x,y,pressure]…] (pen pressure 0..1), size?: pt (10), sizeMode?: \"fixed\" (default) | \"pressure\" (Blob Brush Tool Options' Size set to Pressure: the brush is size − sizeVariation wide at the lightest pen pressure and size + sizeVariation at the heaviest, at least 0.1 pt; a stroke without pressure keeps the size), sizeVariation?: pt (0), angle?: ° (0, kept within −180–180: the tip's long axis counter-clockwise from horizontal, kept on the page along the stroke), roundness?: % (100, kept within 0–100: the tip's short axis in percent of its long one, 100 round; 0 is drawn as 0.1), fidelity?: pt (0.01–100; the brush follows the stroke fitted at this tolerance as path.freehand fits a Pencil stroke, and a higher value gives a smoother shape; without it the brush follows every point), merge?: true, mergeOnlySelection?: bool (false: merge with any touching same-color blob; true: only with selected ones), keepSelected?: bool (false: the drawn or merged path is not selected, and the rest of the selection stays; true: it is selected)} filled brush shape, merged with the topmost touching same-color blob and the touching ones next to it in stacking order in its group or layer → {id}",
            has_doc,
            blob
        ),
    ]
}
