# Tangent circles while dragging

## Intent

When the Selection tool moves one circle, let it snap into an exact external tangency with two
nearby circles. The drag keeps the moved circle's radius: this is a constrained move, not a resize.

## Goals

- Keep the existing drag behavior when no tangent snap applies.
- Match circular live ellipses and closed plain paths that are geometrically circular.
- Ignore hidden, locked, guide and clipping art, and anything in the moved selection.
- Ignore edges when the centers and radii cannot reach the exact sum of their radii.
- Show the two center-to-contact radius lines and a small tangent label when snapped.

## Non-goals

- Changing the moved circle's radius.
- Tangency to ellipses, arbitrary curves, lines or points.
- Changing the two reference circles.
- A separate always-visible tool or a new `.wasm` plug-in. The installed plug-in ABI cannot take
  over pointer movement inside the Ellipse or Selection tools.

## Approach

An object-move gesture asks a precomputed circle index for two references. For fixed moved radius
`r` and references `(c1, r1)` and `(c2, r2)`, valid moved centers are the two intersections of
circles centered at `c1` and `c2` with radii `r + r1` and `r + r2`. The snap chooses the valid
center nearest the pointer-derived center within the existing Smart Guides snapping tolerance.

Only circles already near an external tangent surface are indexed as reference candidates. Candidate
sets are capped before pairing, and the correction nearest the proposed center wins. The tangent
correction replaces ordinary Smart Guides alignment for that frame, while Snap to Grid or Snap to
Pixel and Shift-constrained drags continue to take precedence.

The index and the moved circle are gathered when a move begins; the references come from the
document snapshot, so Alt-drag copies follow the same rules. This keeps the feature deterministic
and lets the exact tangency persist in the committed document.

## Verification

- Unit tests for the circle extraction and two-circle intersection.
- Tool tests for an exact snap, no tangent when the reference circles are unreachable, ignored
  hidden/locked references, no resize, plain-path circles and Alt-drag.
- An engine/app regression showing the transformed circle has both center distances equal to the
  corresponding radius sum, plus Undo and Redo.
