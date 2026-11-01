# Dimensions

The **Dimension Tool** is in the Modify section of the toolbar. Its strip offers **Linear**,
**Angular**, **Radial** and **Tool Options…** (double-clicking the tool opens Tool Options too).

- **Linear:** hover a path segment and drag outwards: releasing places the dimension. Or click two
  points (anchors snap, Shift constrains to 45°), then click to place. A curved segment is measured
  from end to end. With Smart Guides on, both points also snap into line with other objects'
  anchors and centres, and the second onto the construction guides through the first; the guides
  show while hovering, before each click.
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
paths and text as ordinary art. Pasted dimensions keep their geometry and drop their links, so
they never bind to unrelated art in another document.

## Commands

- `dimension.create`: two points, or a source (segment, angle, circle, arc, points), the mode,
  offset and style; returns the group id and label.
- `dimension.update`: restyle or move existing dimensions.
- `dimension.applyToAll`: every dimension takes one dimension's style, in one undo step.
- `tool.select {"tool":"dimension"}` and
  `tool.setOption {"tool":"dimension","values":{"mode":"linear","style":{"units":"mm","scale":"1:10"}}}`
  (`mode`: `linear`, `angular` or `radial`).
- `tool.options {"tool":"dimension"}` opens the `dimensionOptions` dialog.

Recipes live on ordinary groups (the node's `dimension` field); rendering and export use their
generated children. Commands check options and bounds before changing anything. Linked
dimensions refresh inside `Session::edit`, sharing the source edit's undo step; documents without
any (`Document::dimensions` unset) skip that refresh.
