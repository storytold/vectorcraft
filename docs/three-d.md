# Extrude & Bevel, Inflate and Rotate

The other three 3D types of **Effect › 3D and Materials** next to [Revolve](revolve.md). Like Revolve each is a
live effect: the source path stays editable, the effect draws shaded vector faces, and SVG/PDF export, native
saves and **Object › Expand Appearance** use the same evaluator.

Select a path or compound path, then choose **Effect › 3D and Materials › Extrude & Bevel…**, **Inflate…** or
**Rotate…**. The options dialog opens beside the artboard with tabs:

- **Object** (Extrude & Bevel and Inflate): the shape's own options, below.
- **Rotation**: a **Presets** menu, **Rotation X/Y/Z** (−180° to 180°) and **Perspective** (0° to 160°, below). The rotation rings
  on the artboard turn the object about the centre of its bounds; Shift snaps to 15°.
- **Lighting**: **Shading** and the light's settings (below), as for Revolve; or drag the light on the
  artboard.

Preview changes are temporary until OK; Cancel restores the document; OK is one undo step. An object takes one
3D effect: edit its options in the Appearance panel, or remove it before applying another.

## Extrude & Bevel

The art is the front face; its outline is extruded backwards by **Depth** (0–2,000 pt), so seen straight on the
object is where the art was.

| Option | What it does |
|---|---|
| Depth | 0–2,000 pt. At 0 the object is flat (the art turned like Rotate). |
| Twist | 0° to 360°: the back face turned against the front, the walls following in slices. |
| Taper | 1–100 %: the back face's size against the front's. |
| Cap | **Solid** closes the front and back; **Hollow** leaves them open, showing the insides of the walls. |
| Bevel Shape | **None**, **Classic** (a straight chamfer), **Rounded** (convex quarter round), **Cove** (concave), **Step** (two steps) or **Ogee** (an S curve). Our own cross-sections. |
| Width, Height | How far the bevel reaches across the face and along the depth (0–1,000 pt each). The height is limited to the depth (half of it on each side with Bevel both sides). |
| Repeat, Space | The cross-section repeated 1–32 times across the width, with a share (0–90 %) of the width flat between repeats. |
| Bevel Inside | The bevel carved into the shape (the front face shrinks) instead of added around it (the walls stand outside the art). |
| Bevel both sides | The back is bevelled too. |

Holes stay holes, with walls of their own. Bevels move each corner along its bisector (mitres are limited to four
times the width), so a bevel wider than a narrow part of the shape folds over itself.

## Inflate

The shape puffed out like a cushion: its outline stays where the art is and the surface rises towards the
middle.

| Option | What it does |
|---|---|
| Depth | 0–2,000 pt of flat side between the front and the back. |
| Twist | 0° to 360°: the back turned against the front (its puff too), the sides following in slices. |
| Taper | 1–100 %: the back's size against the front's; a puffed back is lower in proportion. |
| Cap | **Solid** closes the back; **Hollow** leaves it off, so the inflated front is a shell open behind, its inside showing. The public documentation lists Cap for every 3D type without saying what it means for Inflate; this is our reading. |
| Volume | 0–100 %. The surface is a membrane under even pressure, held along the outline (and around holes): it rises vertically from the edge and rounds smoothly inside, without creases along the shape's middle. At 50 % a circle becomes a half sphere; 100 % puffs twice as high, 0 % is flat. |
| Both sides | The back puffs out too; otherwise it is flat. |

The surface is cut into a grid of 48 facets across the larger side, halved or quartered where it rises steeply near the outline (the cells meet without cracks), each clipped to the shape and flat shaded.

## Rotate

The flat art turned in 3D about the centre of its bounds. Every fill and stroke is turned with its own paint (a
stroke as its outline, so it foreshortens with the art) and shaded as one plane; a plane has two sides, so its
back shows when it is turned past 90°.

## Rotation presets

**Front**, **Back**, **Left**, **Right**, **Top** and **Bottom** face the art that way. **Off-Axis** tilts each 20°
down and 25° aside, so the top and the right show. **Isometric Left/Right/Top/Bottom** lay the art on that plane
of an isometric view (each visible axis foreshortened equally). These are our own values, from the geometry of
each view; the dialog shows a preset's name when the angles match it and **Custom** otherwise. The new effects
open at Off-Axis Front.

## Perspective

**Perspective** runs 0° to 160°, as the public panel options give it ("how close or far the object appears"),
for Revolve too. The documentation doesn't say how the angle places the eye; our reading: the eye stands where a
sphere about the object's centre fills that angle of the view, the sphere's radius √3 times the object's size
(for Extrude & Bevel, Inflate and Rotate half its largest side, depth included; for Revolve the larger of its
radius and its height, the sizes the earlier strength used), so the object fits inside it. 0° is no
perspective; wider angles bring the eye nearer, and at 160° it is just outside that sphere, so never inside the
object. Default 0°, as before.

Effects saved before the angle (and scripts) give `perspective` as a 0–100 % strength. It is read as the angle
that shows exactly the same view: the strength put the eye at 4 × that size / strength, so
sin(angle / 2) = √3 × strength / 4, and 100 % is about 51.3°. An angle given as well wins; the dialog shows and
saves the angle.

## Lighting

One white directional light, with the public panel options' Light Settings in their order (Color aside):

| Option | |
|---|---|
| Intensity | 0–100 %, default 80 %: the light's brightness. |
| Rotation | −180° to 180°: round the object on the page. Our reading: 0° is on the object's right, counterclockwise, as Illustrator's other angles. |
| Height | 0° to 90°: from beside the object (0°) to straight in front of it (90°). |
| Softness | 0–100 %, default 0 %: how far round the object the light reaches. Our reading: at 0 % the light falls off with the angle to nothing at 90° (as before), at 100 % it reaches nothing only straight away from the light. |
| Ambient Light | 0–200 %, default 25 %: light from everywhere. Faces are never brighter than the art's own colour, so past 100 % (less the light's share) it adds nothing more. |
| Light behind the object | The documented "Move selected light to the back of object": the light shines from behind. |

The documentation gives no defaults; ours keep the earlier light: Rotation 125.26° and Height 30° are the direction
the default light had (45° left of the eye and 45° up). Effects saved before (and scripts) give the light by
`lightAzimuth` and `lightElevation`; those are read as the Rotation, Height and side of the same direction, so the
shading is exactly as before.

## Parameters (`effect.apply`, `effect.setParams`)

`threeD.extrude`: `depth`, `twist`, `taper` (%), `cap`, `bevel` (`none`, `classic`, `rounded`, `cove`, `step`,
`ogee`), `bevelWidth`, `bevelHeight`, `bevelRepeat`, `bevelSpace` (%), `bevelInside`, `bevelBothSides`.
`threeD.inflate`: `depth`, `volume` (%), `bothSides`. All three take the view and lighting parameters Revolve
uses: `rotationX`, `rotationY`, `rotationZ`, `perspectiveAngle` (degrees), `shade`, `lightIntensity` (%), `lightRotation`,
`lightHeight` (degrees), `lightSoftness` (%), `ambient` (%, to 200), `lightBehind`. Lengths are points and scale with the object (Scale Strokes & Effects).
Commands validate the shape and the parameters before the document changes.

## What differs from the public documentation

Behaviour reference: the public [3D and Materials panel options](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/3d-materials-panel-options.html)
and [Create 3D objects](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/create-3d-objects.html)
pages, and the archived Extrude & Bevel option list (depth 0–2,000, cap on/off, bevel height, bevel extent in or
out, perspective). No Adobe software, assets, presets or bevel shapes were used.

- **Materials** (base, Adobe Substance and graphics), **several lights**, light colour, light presets, **shadows**
  and **ray-traced rendering** are not built: shading is one directional light, flat per face, painted with the
  object's top fill (else its top stroke) for Extrude & Bevel and Inflate.
- The bevel shapes and their names are our own; custom bevel paths are not supported.
- Visibility is a painter's algorithm: faces of a closed solid turned away are left out, the rest drawn back to
  front. Concave solids seen at steep angles can overlap wrongly; open (hollow) solids show both sides.
  Twisted walls and bevels are drawn in pieces about as long as a wall slice is wide (fewer near the limit of
  32,768 facets), and a cap or flat back turned toward the viewer is drawn over them. The piece drawn on top can
  still lie a few points behind another nearby piece.
- Shading is flat per facet: an inflated rim seen almost edge-on shows faint banding where the facets get finer.
- Paths and compound paths only: convert type, images and groups to outlines first. One 3D effect per object.
- Curves are followed within 1/2000 of the shape's size (at least 0.05 pt), so a small curve takes a few points
  and a large one up to 32. Extrude & Bevel and Inflate take up to 2,048 outline points, Rotate (which only turns
  the outline) 65,536.
- **3D (Classic)** (the track cube, surface types, blend steps, map art), Turntable and 3D export are separate,
  unbuilt items.
