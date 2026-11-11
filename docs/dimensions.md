# Dimensions

The **Dimension Tool** is in the Modify section of the toolbar. Its strip offers **Linear**,
**Angular**, **Radial** and **Tool Options…** (double-clicking the tool opens Tool Options too).

- **Linear:** hover a path segment and drag outwards: releasing places the dimension. Or click two
  points (anchors snap, Shift constrains to 45°), then click to place. A click within 8 pixels of
  an anchor of any path, in groups and compound paths too, takes that anchor, and the dimension
  stays linked to it. A press on an anchor picks it when the pointer moves less than 8 pixels
  before the release; a longer drag drags out the dimension of the segment under the pointer. A
  curved segment is measured from end to end. With Smart Guides on, both points also snap into line with other objects'
  anchors and centres, and the second onto the construction guides through the first; the guides
  show while hovering, before each click.
  A linear dimension is aligned, horizontal or vertical, as the
  [linear dimensions help](https://helpx.adobe.com/illustrator/desktop/measure-and-align/plot-and-measure/plot-linear-dimensions-of-objects.html)
  describes: after the second click, clicking again places an aligned dimension, and moving the
  pointer away gives a horizontal or vertical one. Which one a pointer position gives is
  VectorCraft's own rule. When the pointer is beside the two points (the nearest point of the
  line through them lies between them) or on that line, the dimension is aligned. When the
  pointer is past either point, the line through the points and the perpendicular through that
  point divide the area beyond it into two quarters, one on each side of the line. The quarter
  that contains the direction straight up or down from the point gives the horizontal distance,
  and the quarter that contains the direction straight left or right gives the vertical distance.
  A click within 8 pixels of the second point, or of where the pointer was for the second click,
  places an aligned dimension. Two points on one horizontal or vertical line always give an
  aligned dimension. A dragged segment uses the same rule with its two ends.
  A horizontal or vertical dimension keeps its orientation when it or the art it measures turns,
  and shows the new horizontal or vertical distance, as the help describes for rotated objects. It
  shows 0 when its points lie on one line across its axis. When the line of a linked dimension
  lies outside its points, the line stays on the same side of the art, as far past the outermost
  point as before. An unlinked dimension that
  turns has its line where the turn puts it, or, when that would put it between the points, as
  far past the outermost point as before.
- **Angular:** hover a corner where two straight segments meet and drag outwards. Or click two
  straight lines that meet (or would, extended), then click to place.
- **Radial:** hover a circle, or a segment that is a perfect arc, and drag: outwards gives its
  radius, inwards its diameter. Clicking it gives its radius. Ellipses have no single radius.
- **Escape** or switching tools cancels a dimension being placed.

**Tool Options** set the style of new dimensions: units (inches, millimetres, points, picas,
pixels, centimetres, metres, feet, yards), precision (0–6 decimals), scale, the dimension and
extension lines (weight, type, colour, arrows), and the text (font, size, colour, position and
alignment). **Position** is **Away from the Line** (the label on the far side of the dimension line
from the object) or **Between Line and Object**; **Alignment** is **Left**, **Center** or
**Right** along the line. **Save** keeps them for new dimensions; existing ones don't change.

Scale means **drawing : real size**, independent of zoom. At **1:10** a 1 in line on the page is
labelled 10 in; at **10:1**, 0.1 in. It scales lengths, not angles or the size of text and arrows.

**Properties** shows a selected dimension's units, precision, line, arrows, text size, colour,
position and alignment; changing one changes that dimension. **Apply to All** gives every
dimension in the document its style; **Set as Default** makes it the style of new dimensions.

Measurements follow the art they measure through moves, scaling and anchor and handle edits,
undo and redo included. Deleting the art, or changing its number of anchors or whether it is
closed, removes its dimensions in the same undo step rather than attaching them to a different
edge. A linked dimension moved on its own stays where it was dropped (its distance from the art
changes); moved together with its art, it keeps its distance.

Native `.vectorcraft` files keep the recipes and links. SVG, PDF and bitmap output contain
ordinary paths and text, so exported dimensions look the same. Ungrouping a dimension leaves its
paths and text as ordinary art. Object › Expand, Object › Expand Appearance, Type › Create Outlines
and Object › Path › Outline Stroke turn a dimension whose art they change into a plain group of that
art, also when they reach only a part of it: moving or scaling the group does not redraw it, it does
not follow the art it measured, and undoing the command gives the live dimension back. The menus dim
Object › Expand… while a whole dimension is selected, because its label has appearance items;
Object › Expand Appearance and the `object.expand` command still reach it. A dimension selected with
the art it measures also becomes a plain group when Object › Expand, Object › Expand Appearance or
Object › Path › Outline Stroke changes only that art and the art no longer fits the dimension's link,
for example because its number of anchors changed. When the art keeps its id and anchor count and the
dimension's own art does not change, the dimension stays live and its label shows the new length. Pasted
dimensions keep their geometry and drop their links, so they never bind to unrelated art in another
document.

## Commands

- `dimension.create`: two points, or a source (segment, angle, circle, arc, points), the mode,
  the orientation of a linear dimension (`aligned`, the default, `horizontal` or `vertical`),
  offset and style; returns the group id and label. `offset` is the distance of the dimension
  line from the first point: along the left normal of a → b for an aligned dimension, downward
  for a horizontal one and leftward for a vertical one. A negative offset goes the other way.
- `dimension.update`: restyle or move existing dimensions, or change the orientation of linear
  ones. A new orientation keeps the line on the side of the points it was on. An update that
  changes nothing records no undo step.
- `dimension.applyToAll`: every dimension takes one dimension's style, in one undo step.
- `tool.select {"tool":"dimension"}` and
  `tool.setOption {"tool":"dimension","values":{"mode":"linear","style":{"units":"mm","scale":"1:10"}}}`
  (`mode`: `linear`, `angular` or `radial`).
- `tool.options {"tool":"dimension"}` opens the `dimensionOptions` dialog.

Recipes live on ordinary groups (the node's `dimension` field); rendering and export use their
generated children. Commands check options and bounds before changing anything. Linked
dimensions refresh inside `Session::edit`, sharing the source edit's undo step; documents without
any (`Document::dimensions` unset) skip that refresh.
