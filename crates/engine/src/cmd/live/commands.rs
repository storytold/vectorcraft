use super::blend::{
    blend_expand, blend_info, blend_make, blend_options, blend_release, blend_replace_spine, blend_reverse_spine, blend_reverse_stack,
    spine_add_anchor, spine_convert_anchor, spine_move_anchor, spine_remove_anchor, spine_reshape_segment,
};
use super::common::{has_blend, has_envelope, has_mesh};
use super::envelope::{
    env_edit_contents, env_expand, env_info, env_make_mesh, env_make_top, env_make_warp, env_options, env_release, env_reset_mesh, env_reset_warp,
    env_set_mesh_point,
};
use super::mesh::{mesh_add_line, mesh_create, mesh_delete_point, mesh_expand, mesh_move_point, mesh_move_points, mesh_set_color};
use super::{CommandSpec, always, cmd, has_doc, has_multi, has_selection};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        // ---------- Blend ----------
        cmd!(
            "object.blend.make",
            "Make",
            ["Object", "Blend"],
            Some("Cmd+Alt+B"),
            "{ids?, steps?: n | distance?: pt | smooth?: bool, orientation?: page|path (default: the Blend Options set with nothing selected, else smooth colour and page), starts?: [anchor index | null, …] (per object of `ids`, else of the selection in paint order: the anchor of its first subpath the blend starts from; an open path's last anchor runs it the other way), spacing?: smooth|steps|distance (with value), value?: n, easingType?: easeIn|easeOut|easeInOut|easeOutIn, ramp?: pct (0..100), colorShift?: pct (-100..100; 0: even)} blend the selected objects (paint order) into a live blend; a blend among them takes the others in as more key objects (keeping its options, name and transparency) instead of nesting → {id}",
            has_doc,
            blend_make
        ),
        cmd!(
            "object.blend.release",
            "Release",
            ["Object", "Blend"],
            Some("Cmd+Alt+Shift+B"),
            "{} release the selected blends: the key objects come back and the spine stays as a path with no fill or stroke, below them; a blend with a name, opacity, blend mode, isolation, opacity mask or appearance of its own comes back as a group keeping them (and its knockout) → {ids (the keys), spines: [ids], groups: [ids]}",
            has_blend,
            blend_release
        ),
        cmd!(
            "object.blend.options",
            "Blend Options…",
            ["Object", "Blend"],
            None,
            "{spacing?: smooth|steps|distance, value?: n, steps?: n, distance?: pt, orientation?: page|path, smooth?: bool, easingType?: easeIn|easeOut|easeInOut|easeOutIn, ramp?: pct, colorShift?: pct} set the options of the selected blends; easingType chooses the distribution, ramp sets its strength (0..100; 0 = even), colorShift (-100..100; 0 = even) shifts colours independently of shape/position easing (omitted ones keep their value; out of range is an error); with no blend selected, the options new blends start with (spacing and orientation only; a tool setting, not an undo step) → {defaults?: true}",
            always,
            blend_options
        ),
        cmd!(
            "object.blend.expand",
            "Expand",
            ["Object", "Blend"],
            None,
            "{} replace the selected blends by groups of their keys and steps, each keeping the blend's id, name, transparency (knockout and isolation included), opacity mask and appearance → {ids}",
            has_blend,
            blend_expand
        ),
        cmd!(
            "object.blend.replaceSpine",
            "Replace Spine",
            ["Object", "Blend"],
            None,
            "{} use the selected path as the spine of the selected blend (the path is consumed)",
            has_blend,
            blend_replace_spine
        ),
        cmd!(
            "object.blend.reverseSpine",
            "Reverse Spine",
            ["Object", "Blend"],
            None,
            "{} reverse the order of the key objects along the spine",
            has_blend,
            blend_reverse_spine
        ),
        cmd!(
            query "object.blend.info",
            "Blend Info",
            [],
            None,
            "{} the options of the first selected blend, else those new blends start with → {target: blend|defaults, id?, spacing: smooth|steps|distance, steps, distance, orientation: page|path, easingType?: easeIn|easeOut|easeInOut|easeOutIn, ramp?: pct, colorShift?: pct, keys?: [ids], starts?: [anchor|null], spine?: {anchors: [{x, y, in: [x, y], out: [x, y]}], closed, keyAnchors: [anchor each key sits on] | null (keys spread evenly: a spine from Replace Spine), explicit: false for the straight lines between the key centres}}",
            always,
            blend_info
        ),
        cmd!(
            "object.blend.spine.moveAnchor",
            "Move Spine Point",
            [],
            None,
            "{id? (default: the selected blend), anchor: index, x, y, handle?: in|out, independent?: bool} move a point of the blend's spine (its handles along), or with `handle` place that handle's end at (x, y) (a smooth point keeps the other handle in line unless `independent`). A key object sitting on the point moves with it. The first edit turns the straight spine into a path",
            has_doc,
            spine_move_anchor
        ),
        cmd!(
            "object.blend.spine.convertAnchor",
            "Convert Anchor Point",
            [],
            None,
            "{id? (default: the selected blend), anchor: index, to: smooth|corner, x?, y?} convert a blend spine point: dragging a corner creates symmetric smooth handles, clicking a point with handles retracts both. Without x/y, smooth uses the neighbouring spine points",
            has_doc,
            spine_convert_anchor
        ),
        cmd!(
            "object.blend.spine.reshapeSegment",
            "Reshape Spine Segment",
            [],
            None,
            "{id? (default: the selected blend), segment: index, t: 0..1, dx, dy, perpendicular?: bool} bend a segment of the blend's spine so its point at t moves by (dx, dy), as path.reshapeSegment bends a path's (perpendicular too): a straight spine turns into a curve (#1111)",
            has_doc,
            spine_reshape_segment
        ),
        cmd!(
            "object.blend.spine.addAnchor",
            "Add Spine Point",
            [],
            None,
            "{id? (default: the selected blend), x, y} add a point to the blend's spine where it passes nearest (x, y) → {anchor}",
            has_doc,
            spine_add_anchor
        ),
        cmd!(
            "object.blend.spine.removeAnchor",
            "Delete Spine Point",
            [],
            None,
            "{id? (default: the selected blend), anchor: index} delete a point of the blend's spine that no key object sits on",
            has_doc,
            spine_remove_anchor
        ),
        cmd!(
            "object.blend.reverseFrontToBack",
            "Reverse Front to Back",
            ["Object", "Blend"],
            None,
            "{} reverse the stacking order of the selected blends",
            has_blend,
            blend_reverse_stack
        ),
        // ---------- Envelope ----------
        cmd!(
            "object.envelope.makeWithWarp",
            "Make with Warp…",
            ["Object", "Envelope Distort"],
            Some("Cmd+Alt+Shift+W"),
            "{ids?, style?: arc|arcLower|arcUpper|arch|bulge|shellLower|shellUpper|flag|wave|fish|rise|fisheye|inflate|squeeze|twist (arc), bend?: % -100..100 (50), h?: horizontal distortion % -100..100 (0), v?: vertical distortion % (0), horizontal?: bool (true) | orientation?: horizontal|vertical, horizontalDistortion?: as h, verticalDistortion?: as v} envelope the selected objects with a warp (Envelope Options from object.envelope.options with nothing selected) → {id}",
            has_selection,
            env_make_warp
        ),
        cmd!(
            "object.envelope.makeWithMesh",
            "Make with Mesh…",
            ["Object", "Envelope Distort"],
            Some("Cmd+Alt+M"),
            "{ids?, rows?: 1..50 (4), cols?: 1..50 (4), columns?: as cols} envelope the selected objects with a mesh of rows × cols patches → {id}",
            has_selection,
            env_make_mesh
        ),
        cmd!(
            "object.envelope.makeWithTopObject",
            "Make with Top Object",
            ["Object", "Envelope Distort"],
            Some("Cmd+Alt+C"),
            "{ids?} the topmost selected path becomes the envelope of the others → {id}",
            has_multi,
            env_make_top
        ),
        cmd!(
            "object.envelope.resetWithWarp",
            "Reset with Warp…",
            ["Object", "Envelope Distort"],
            None,
            "{style?, bend?, h?, v?, horizontal? | orientation? (as makeWithWarp; unset: the envelope's current warp, else arc 50 %), horizontalDistortion?: as h, verticalDistortion?: as v} make the selected envelopes warp envelopes, keeping their content",
            has_envelope,
            env_reset_warp
        ),
        cmd!(
            "object.envelope.resetWithMesh",
            "Reset with Mesh…",
            ["Object", "Envelope Distort"],
            None,
            "{rows?: 1..50, cols?: 1..50 (unset: a mesh envelope's own, else 4), maintainShape?: bool (true: the mesh follows the current envelope shape; false: a flat grid over the content), columns?: as cols} make the selected envelopes mesh envelopes, keeping their content",
            has_envelope,
            env_reset_mesh
        ),
        cmd!(
            query "object.envelope.info",
            "Envelope Info",
            [],
            None,
            "{} the selected envelope's settings, or with none selected the defaults new envelopes get → {id: n|null, type: warp|mesh|topObject, style, bend, h, v, horizontal, rows, cols, fidelity, antiAlias, preserveShape: clippingMask|transparency, distortAppearance, distortLinearGradients, distortPatternFills, editing}",
            has_doc,
            env_info
        ),
        cmd!(
            "object.envelope.release",
            "Release",
            ["Object", "Envelope Distort"],
            None,
            "{} release the selected envelopes into their content and their shape (a grey gradient mesh for warp and mesh envelopes, the path for a top-object envelope; the content keeps the envelope's opacity, blend mode and opacity mask) → {ids}",
            has_envelope,
            env_release
        ),
        cmd!(
            "object.envelope.options",
            "Envelope Options…",
            ["Object", "Envelope Distort"],
            None,
            "{fidelity?: 0..100, antiAlias?: bool, preserveShape?: clippingMask|transparency, distortAppearance?: bool, distortLinearGradients?: bool, distortPatternFills?: bool (the last two work with distortAppearance), style?, bend?, h?, v?, horizontal? (warp envelopes), horizontalDistortion?: as h, verticalDistortion?: as v, orientation?: horizontal|vertical} set the selected envelopes' options; with no envelope selected, the options new envelopes get (see object.envelope.info)",
            has_doc,
            env_options
        ),
        cmd!(
            "object.envelope.expand",
            "Expand",
            ["Object", "Envelope Distort"],
            None,
            "{} replace the selected envelopes by groups of their distorted content (type outlined), keeping name, opacity, blend mode, isolation, knockout and opacity mask → {ids}",
            has_envelope,
            env_expand
        ),
        cmd!(
            "object.envelope.editContents",
            "Edit Contents",
            ["Object", "Envelope Distort"],
            None,
            "{editing?: bool} toggle Edit Contents / Edit Envelope → {editing}",
            has_envelope,
            env_edit_contents
        ),
        cmd!(
            "object.envelope.setMeshPoint",
            "Move Envelope Mesh Point",
            [],
            None,
            "{id, index, x, y, handle?: 0 right|1 left|2 down|3 up} move a point of a mesh envelope (its handles follow), or with `handle` place that handle end at (x, y); object.mesh.movePoint, addLine and deletePoint edit mesh envelopes too",
            has_doc,
            env_set_mesh_point
        ),
        // ---------- Gradient mesh ----------
        cmd!(
            "object.mesh.create",
            "Create Gradient Mesh…",
            ["Object"],
            None,
            "{ids?, rows?: n (4), cols?: n (4), appearance?: flat|center|edge, highlight?: % (100), at?: [x,y], tolerance?: pt (0), columns?: as cols} convert filled paths into gradient meshes in their fill colour (a gradient fill: the colour it paints at each point), lightened per `appearance` (with `at`: 1×1 mesh plus lines through that point, or through the nearest point of the edge when `at` is outside within `tolerance`); an embedded image becomes a mesh over its rectangle (its transform kept) whose points take the average colour and opacity of the picture around them (a linked image is refused: embed it first) → {ids, index?}",
            has_selection,
            mesh_create
        ),
        cmd!(
            "object.mesh.setPointColor",
            "Set Mesh Point Color",
            [],
            None,
            "{id, index | points: [index…] | items: [{id, index | points}…], color?, keepModel?: bool (as paint.setFill), opacity?: 0..1} colour and/or opacity of gradient mesh points (one undo step; an index out of range changes nothing)",
            has_doc,
            mesh_set_color
        ),
        cmd!(
            "object.mesh.movePoint",
            "Move Mesh Point",
            [],
            None,
            "{id, index, x, y, handle?: 0 right|1 left|2 down|3 up} move a point of a gradient mesh or mesh envelope (its handles follow), or with `handle` place that handle end at (x, y)",
            has_doc,
            mesh_move_point
        ),
        cmd!(
            "object.mesh.movePoints",
            "Move Mesh Points",
            [],
            None,
            "{id, points: [index…] | items: [{id, index | points}…], dx, dy} move points of gradient meshes or mesh envelopes by (dx, dy), their handles with them (one undo step; an index out of range changes nothing)",
            has_doc,
            mesh_move_points
        ),
        cmd!(
            "object.mesh.addLine",
            "Add Mesh Line",
            [],
            None,
            "{id, x, y, color?, keepModel?: bool (as paint.setFill), tolerance?: pt (0)} add a row and a column through (x, y) to a gradient mesh (new point in `color`) or a mesh envelope; a point outside within `tolerance` of the edge adds them through the nearest point of the edge → {index}",
            has_doc,
            mesh_add_line
        ),
        cmd!(
            "object.mesh.deletePoint",
            "Delete Mesh Point",
            [],
            None,
            "{id, index} delete the mesh lines through a point of a gradient mesh or mesh envelope",
            has_doc,
            mesh_delete_point
        ),
        cmd!(
            "object.mesh.expand",
            "Expand Gradient Mesh",
            ["Object"],
            None,
            "{} replace the selected meshes by groups of flat-coloured pieces → {ids}",
            has_mesh,
            mesh_expand
        ),
    ]
}
