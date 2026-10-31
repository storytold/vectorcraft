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

The surface starts with a grid of 48 cells across the larger side, halved or quartered where it rises steeply near the outline. White-light Inflate retains the existing clipped polygon cells, shading, paint composition and seam joins. With colored lights, simple cells are triangulated with shared border samples, and lighting interpolates shared membrane normals through ordinary RGB vector gradients, including on a puffed back. Solid fills batch those triangles into three channel groups and one coverage clip for the whole surface. Source gradients retain per-facet vector composition and apply transparent coverage once. Seam widths retain the 0.05 to 0.75 pt bounds; colored facets use round joins. Shading is automatic, with no additional dialog option.

## Fills and strokes

Extrude & Bevel and Inflate build a solid from the path for the object's top fill, and one from the outline of
each visible stroke (with its caps, joins, dashes, alignment and arrowheads), each in its own paint. An admin's
reply on the reference app's bug forum says that strokes "will be treated as outlined when applying 3D"
([August 2024](https://illustrator.uservoice.com/forums/601447-illustrator-desktop-bugs/suggestions/48755201-expanding-a-3d-extruded-object-with-a-stroke-as-a)).
The rest is our reading:

- When the fill and every stroke are opaque with Normal blending, the faces of all the solids are drawn in one
  back-to-front order, and the caps turned toward the viewer are drawn last, the lower item's first. Otherwise
  each solid is drawn on its own with its opacity and blending, the lower item's first. With a light that is not
  white, each solid is also drawn on its own when one of them is a puffed, shaded Inflate surface in a solid color
  (drawn as one composite) or is painted with a gradient that has transparent stops (drawn with an opacity mask).
- The solids turn, twist and taper about the center of the bounds of all their shapes, Inflate's grid covers
  those bounds, and with Perspective one eye sees every solid. A stroke too narrow to hold a point of Inflate's
  grid stays flat.
- A stroke whose outline takes more than 2,048 points is left out, and the fill's solid still draws.

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

### Presets

The Lighting tab has **Standard**, **Diffuse**, **Top Left** and **Right** choices. Choosing one
replaces the whole light rig with one white front light at 80% intensity and sets Ambient Light.
It leaves Shading, the object's view, geometry and other effect options unchanged. Manual edits
remain available; the list shows **Custom** whenever the effective lighting matches none of the
presets, including when there is more than one light. An equivalent explicit single light is
recognized too. Reset all returns to Standard. Preview, Cancel and Undo work as for other options.

These are original VectorCraft settings based on the public preset names and directions. Adobe's
documentation does not publish numeric rigs; the values are not copied from its software or files
and are not claimed to reproduce its presets exactly. Standard retains the existing default.

| Preset | Rotation | Height | Softness | Ambient Light |
|---|---:|---:|---:|---:|
| Standard | 125.26438968275465 degrees | 30 degrees | 0% | 25% |
| Diffuse | 125.26438968275465 degrees | 30 degrees | 100% | 35% |
| Top Left | 135 degrees | 45 degrees | 0% | 25% |
| Right | 0 degrees | 30 degrees | 0% | 25% |

`effect.lightingPreset {index: 0, item: null, preset: "standard" | "diffuse" | "topLeft" | "right", ids?, target?}`
applies the same settings through commands, CLI and MCP. The index and item address an already
applied 3D effect. Every target must have a 3D effect at that position; validation is atomic, and
the whole operation is one undo step. The numeric light settings are saved, not a preset name.
Existing documents keep their appearance unless a preset is chosen.

### Light settings

One to ten directional lights. **Lights** selects the light whose settings and artboard gizmo are edited. **Add Light** adds and selects a white front light; **Delete Light** removes the selected light while retaining at least one. Ambient Light is shared by all lights. The public panel options' Light Settings are kept in their order:

| Option | |
|---|---|
| Color | Opaque RGB light color, white by default. The picker previews changes; its dot resets to white. |
| Intensity | 0–100 %, default 80 %: the light's brightness. |
| Rotation | −180° to 180°: round the object on the page. Our reading: 0° is on the object's right, counterclockwise, as Illustrator's other angles. |
| Height | 0° to 90°: from beside the object (0°) to straight in front of it (90°). |
| Softness | 0–100 %, default 0 %: how far round the object the light reaches. Our reading: at 0 % the light falls off with the angle to nothing at 90° (as before), at 100 % it reaches nothing only straight away from the light. |
| Ambient Light | 0–200 %, default 25 %: neutral light from everywhere. Each channel is clamped to the art's own color; at 100 % ambient every channel is fully lit, so higher values add nothing. |
| Light behind the object | The documented "Move selected light to the back of object": the light shines from behind. |

Colored illumination multiplies each source RGB channel by `clamp(ambient + sum(intensity * diffuse * color), 0, 1)`. Each light has its own direction and softness. Ambient is added once, and channel saturation happens after the sum. Ambient light stays neutral. Softness supplies the same diffuse falloff as before. White supplies neutral illumination; Shading off ignores the lights. Solid fills and gradient stops (including freeform points) receive the color. Pattern fills retain the existing neutral shadow approximation because their referenced tiles are not evaluated per face. SVG/PDF and Expand Appearance keep the shaded solids and gradients as vectors.

Revolve interpolates the directional illumination before clipping each RGB channel, adding vector stops where a channel saturates. Its continuous caps keep flat lighting. A source gradient and the light ramp are combined with isolated vector Multiply groups; a vector clip applies the curved face boundary once, avoiding antialiasing seams, and a vector opacity mask preserves transparent stops when needed. With multiple lights, each RGB channel has its own affine vector gradient; disjoint channel ramps combine through Screen before multiplying the source paint. This preserves differently colored directions and clips each channel after interpolation. No raster image is needed.

The documentation gives no numeric defaults; ours keep the earlier light: Rotation 125.26° and Height 30° are the direction
the default light had (45° left of the eye and 45° up). Effects saved before (and scripts) give the light by
`lightAzimuth` and `lightElevation`; those are read as the Rotation, Height and side of the same direction, so the
shading is exactly as before.

## Parameters (`effect.apply`, `effect.setParams`)

`threeD.extrude`: `depth`, `twist`, `taper` (%), `cap`, `bevel` (`none`, `classic`, `rounded`, `cove`, `step`,
`ogee`), `bevelWidth`, `bevelHeight`, `bevelRepeat`, `bevelSpace` (%), `bevelInside`, `bevelBothSides`.
`threeD.inflate`: `depth`, `volume` (%), `bothSides`. All three take the view and lighting parameters Revolve
uses: `rotationX`, `rotationY`, `rotationZ`, `perspectiveAngle` (degrees), `shade`, `lightColor` (`#rrggbb`, default `#ffffff`), `lightIntensity` (%), `lightRotation`,
`lightHeight` (degrees), `lightSoftness` (%), `ambient` (%, to 200), `lightBehind`. Lengths are points and scale with the object (Scale Strokes & Effects).
Commands validate the shape and the parameters before the document changes. All four also take `lights`: null (the default, use the earlier single-light fields), or an array of one to ten objects. Each object takes `color` (opaque `#rrggbb`, white), `intensity` (0..100%, 80), `rotation` (-180..180 degrees, 125.26), `height` (0..90 degrees, 30), `softness` (0..100%, 0), and `behind` (boolean, false). Omitted settings use those defaults. An explicit list replaces the earlier single light; `ambient` and `shade` remain shared. Set `lights` to null to return to the single-light fields. Invalid lists, counts, keys and values are rejected before editing; malformed loaded lists fall back to the earlier light safely.

## What differs from the public documentation

Behaviour reference: the public [3D and Materials panel options](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/3d-materials-panel-options.html)
and [Create 3D objects](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/create-3d-objects.html)
pages, and the archived Extrude & Bevel option list (depth 0–2,000, cap on/off, bevel height, bevel extent in or
out, perspective). No Adobe software, assets, presets or bevel shapes were used.

- **Materials** (base, Adobe Substance and graphics), **shadows**
  and **ray-traced rendering** are not built: shading uses directional lights, in the paint of the fill or
  stroke the solid is built for ([Fills and strokes](#fills-and-strokes)).
- The bevel shapes and their names are our own; custom bevel paths are not supported.
- Visibility is a painter's algorithm: faces of a closed solid turned away are left out, and the rest are drawn
  back to front, with a flat cap or flat back turned toward the viewer drawn after all of them. Concave solids
  seen at steep angles can overlap wrongly; open (hollow) solids show both sides. Twisted walls and bevels are
  drawn in pieces about as long as a wall slice is wide (fewer near the limit of 32,768 facets). The piece drawn
  on top can still lie a few points behind another nearby piece.
- Extrude & Bevel and Rotate retain planar shading. Colored Inflate uses smooth vector lighting; white-light Inflate keeps its original polygon shading; bounded cell triangulation leaves a cell flat when it contains a complete hole or an unusually complex/degenerate contour. Its geometric silhouette is still a polygon approximation.
- Paths and compound paths only: convert type, images and groups to outlines first. One 3D effect per object.
- Curves are followed within 1/2000 of the shape's size (at least 0.05 pt), so a small curve takes a few points
  and a large one up to 32. Extrude & Bevel and Inflate take up to 2,048 outline points (a stroke's outline is
  counted on its own), Rotate (which only turns the outline) 65,536.
- **3D (Classic)** (the track cube, surface types, blend steps, map art), Turntable and 3D export are separate,
  unbuilt items.

Release measurements and a repeatable CLI scene are in [3D lighting performance](3d-lighting-performance.md).

## Revision history

| Date | Change |
|---|---|
| 2026-10-21 | Four original lighting presets, shared by all four 3D effects and commands, with effective-setting recognition and existing preview/history behavior. |
| 2026-10-17 | Smooth colored Inflate lighting and surface batching; legacy white-light appearance and the 0.05 seam minimum preserved; shared light Color and multiple lights remain vector output. |
