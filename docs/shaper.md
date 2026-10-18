# Shaper

Sketch a basic shape with the Shaper tool (Shift+N). Rectangles, squares and ellipses snap to 45° rotation steps; hexagons to 90°. Triangles point up or down; sideways sketches snap to one of those orientations. Lines keep the angle you draw, including while Shift is held.

Scribble over overlapping paths to make an editable Shaper Group:

- Start in a filled region and cross into other regions to merge the regions touched. The result uses the starting region’s fill.
- Scribble within a region to remove its fill and keep its stroke. Hidden overlap boundaries are part of the same visible face; visible dividing strokes create separate faces.
- End the scribble outside the artwork to remove the touched fills and exposed boundary strokes. Shared edges along surviving filled areas remain, closing their contours. Compatible stroke pieces join at the new corners, so the selected stroke’s join style applies there.
- Scribble over an unfilled stroke segment to remove that piece. A line passing through a shape is split at the intersections, so its protruding ends can be removed separately.

Click once to select the group; click again to select its visible fill and change its color. Double-click with Shaper to enter construction editing and select an original shape. The Selection tool then moves or resizes it normally, and the visible composition updates. Escape leaves construction editing. The original live shapes and edit recipes survive native save/reopen. Outside construction editing, selection highlights and bounds show only the visible result. Outside construction editing, a marquee selects the group or its faces only where it reaches the visible result, and Direct Selection's clicks and marquee select only the anchor points of the visible result.

While the group or one of its faces is selected (Face Selection mode) and the Shaper or Selection tool is active, an arrow widget sits beside the middle of the right side of the original shapes’ bounds. Click it to enter Construction mode: the original shapes are isolated and nothing is selected, and a click with Shaper selects the group’s original shape under the pointer. Click the widget again to leave Construction mode with the group selected. The arrow points right in Face Selection mode and left in Construction mode.

Smart Guides and Snap to Point do not snap to the original shapes. The reference point of the transform tools snaps to an original shape only when that shape is selected, or under the pointer in Construction mode. Outside Construction mode the Lasso does not pick the anchor points of the original shapes, and the Add Anchor Point, Delete Anchor Point, Anchor Point and Scissors tools do not pick the anchor points or segments of an original shape that is not selected.

Object › Shaper › Release restores the original paths. Expand keeps the visible result as ordinary paths. Each scribble and source edit records one undo step. Sources in a composition must share a parent layer/group; Shaper limits a composition to 64 source paths and 4,096 segments.

Finer gesture heuristics remain to be implemented.

Behavior reference: [public Shaper documentation](https://helpx.adobe.com/illustrator/using/building-new-shapes-using-shape.html). Rotation steps and independent fill/stroke cases follow the contributor’s supplied requirements. No reference images or product assets are included.
