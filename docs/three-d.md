# Extrude & Bevel, Inflate and Rotate

The other three 3D types of **Effect › 3D and Materials** next to [Revolve](revolve.md). Like Revolve each is a
live effect: the source path stays editable, the effect draws shaded vector faces, and SVG/PDF export, native
saves and **Object › Expand Appearance** use the same evaluator.

Select a path, compound path, type, clipping group, blend, symbol instance, or a group or layer of them, then choose
**Effect › 3D and Materials › Extrude & Bevel…**, **Inflate…** or
**Rotate…**. The options dialog opens beside the artboard with tabs:

- **Object** (Extrude & Bevel and Inflate): the shape's own options, below.
- **Rotation**: a **Presets** menu, **Rotation X/Y/Z** (−180° to 180°) and **Perspective** (0° to 160°, below). The rotation rings
  on the artboard turn the object about the centre of its bounds; Shift snaps to 15°.
- **Lighting**: **Shading** and the light's settings (below), as for Revolve; or drag the light on the
  artboard.

Preview changes are temporary until OK; Cancel restores the document; OK is one undo step. An object takes one
3D effect: edit its options in the Appearance panel or the 3D and Materials panel, or remove it before applying
another.

Each solid follows its shape as it is filled. Separate and nested contours are built as drawn. Contours that cross,
touch or run along each other or themselves, as type often draws a glyph, are first merged into one outline by
[`vectorcraft-pathops`](../crates/pathops/README.md), keeping every contour but the numerical debris a boolean leaves.
Whether contours meet is judged on the outline as it is followed (see the curve tolerance below), so curves that
overlap by less than that are built as drawn. All three effects share pathops'
[limits](../crates/pathops/README.md#limits): a shape it refuses is built from its contours as drawn, where it can be.
An open subpath is closed by a straight line either way.

## 3D and Materials panel

**Window › 3D and Materials** opens a panel that edits the 3D effect of the selection without a dialog, as the
public [3D and Materials panel options](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/3d-materials-panel-options.html)
and [Create 3D vector artwork](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/create-3d-vector-artwork.html)
pages describe it.

- The object types **Plane**, **Extrude**, **Revolve** and **Inflate** sit at the top. A type applies its effect
  to each selected object that has no 3D effect (Plane is Rotate, Extrude is Extrude & Bevel), and changes the
  3D effect of each other selected object in place (`effect.setParams` with `threeDType`): the settings both
  effects take keep their values, and the rest start at the new effect's defaults. One click is one undo step,
  and an object whose effect already has the type does not change.
- Below them are the Object, Rotation and Lighting tabs of the effect's options dialog, with the same controls
  and ranges, showing the 3D effect of the first selected object (its own effects first, then those of its fills
  and strokes). A changed setting goes to the 3D effect of each selected object whose effect takes that setting;
  other effects and objects without a 3D effect do not change. A setting the selected 3D effects hold with
  different values shows its default with its reset dot filled; a checkbox shows a dash instead, and a mixed
  Shading leaves the light settings available. Its reset dot, a choice from its menu or a click on its checkbox
  (which turns a mixed one on) sets it on all of them; a typed or dragged number sets it when it differs from the
  default shown. Other changes leave it as each effect has it.
  The list of lights shows the first object's lights, and an edit to it (Add Light, Delete Light, or a setting
  of one light in a list) reaches only the objects whose list of lights matches the first object's. Add Light is
  unavailable while a setting of a single light differs between the selected effects.
- Each change is one undo step. Dragging a value or a slider previews it on the artboard and records one step
  when the pointer is released; a typed value is recorded when it is entered. Drawing the panel records nothing.
- **Quick Actions › Expand as Wireframes** sits under the Object tab (under the first tab for Revolve and Rotate,
  which have no Object tab). It replaces each selected object that has a 3D effect, the 3D (Classic) effects included, with a group of paths, the
  edges of the faces the effect draws for the object as it is turned and projected now (`effect.expandAsWireframes`),
  as the public [Render 3D vector artwork](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/render-3d-vector-artwork.html)
  page describes for converting the wireframe into editable paths. Each edge appears once: edges that meet end to
  end form one path, closed where it comes back to its start, and curved edges stay curves. A face partly covered
  by another face gives its whole outline. Faces turned away from the viewer are included where the effect draws
  them (Draw Hidden Faces and the Wireframe surface of Extrude & Bevel (Classic)). Geometry effects after a 3D
  effect, on the object or on a fill or stroke, reshape the edges as they reshape the drawn art. Shadows are not
  part of the wireframe. The paths have a 0.5 pt black stroke with round joins and no fill, which is VectorCraft's
  own choice. The group keeps the object's place, id, name, opacity, blending mode, opacity mask, attributes
  and text wrap and is selected; objects without a visible 3D effect do not change. It is one undo step, and an error changes nothing
  when no selected object has a visible 3D effect.
- The Lighting tab also holds the Shadows switch and its settings ([Shadows](#shadows)). The Materials tab holds
  the Graphics ([Graphics](#graphics)); base materials and ray-traced rendering are not built (see below). The
  rotation and light rings on the artboard are part of the options dialog only.

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
times the width), so a bevel wider than a narrow part of the shape folds over itself. Each facet has a seam ([Seams](#seams)).

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

The surface starts with a grid of 48 cells across the larger side, halved or quartered where it rises steeply near the outline. White-light Inflate retains the existing clipped polygon cells, shading and paint composition. With colored lights, simple cells are triangulated with shared border samples, and lighting interpolates shared membrane normals through ordinary RGB vector gradients, including on a puffed back. Solid fills batch those triangles into three channel groups and one coverage clip for the whole surface. Source gradients retain per-facet vector composition and apply transparent coverage once. Seam widths retain the 0.05 to 0.75 pt bounds, and every facet's seam has round joins ([Seams](#seams)). Shading is automatic, with no additional dialog option.

## Seams

Facets side by side, each antialiased on its own, would leave hairlines between them, so 3D facets are drawn with
a seam of their own paint, with round joins. In Extrude & Bevel, Inflate, Rotate and the three 3D (Classic) effects a
facet's seam is half as wide as the facet is thick, from 0.05 to 0.75 pt; Revolve's seam is always 0.75 pt.

How a facet carries its seam depends on the lights and the paint:

- Extrude & Bevel, Inflate and Rotate, when no light is colored (one white light or several): a path filled with
  the facet's shading and stroked with its seam. With a colored light, Extrude & Bevel and Rotate (planar shading)
  are drawn the same way.
- Inflate with a colored light and shading on: a solid-color fill draws each facet's light ramps with its seam in three channel
  groups clipped to the union of its facets, so no seam reaches past the surface; a gradient fill fills each facet's
  ramps through a clip of the facet and its seam, outlined with round joins.
- Extrude & Bevel (Classic), Rotate (Classic) and Revolve (Classic): a path and a seam for every face; gradient and
  pattern faces seam only the base paint, and the shading and highlight layers cover the face alone. The Wireframe
  surface draws edges only, with no seam.
- Revolve, with shading on: when a `lights` list is given (even one of white lights), a solid-color or gradient face fills its light
  ramps through a clip of the face and its 0.75 pt seam, outlined with round joins. With the single legacy light,
  a face is a path and a seam; with a colored `lightColor` a gradient face fills through the same kind of clip.

A facet seen almost edge-on is a sliver, and a seam with mitered joins would reach past the solid as a thorn; a round
one reaches no more than half its width past its facet. So where a facet has no neighbor (Rotate's plane, every
silhouette corner) its convex corners are drawn rounded by up to 0.375 pt (half the largest seam), except a
solid-color Inflate under a colored light, whose clip keeps its corners sharp.

## Fills and strokes

Extrude & Bevel and Inflate build a solid from the path for the object's top fill, and one from the outline of
each visible stroke (with its caps, joins, dashes, alignment and arrowheads), each in its own paint. An admin's
reply on the reference app's bug forum says that strokes "will be treated as outlined when applying 3D"
([August 2024](https://illustrator.uservoice.com/forums/601447-illustrator-desktop-bugs/suggestions/48755201-expanding-a-3d-extruded-object-with-a-stroke-as-a)).
The rest is our reading:

- When the fill and every stroke are opaque with Normal blending, the faces of all the solids are drawn in one
  back-to-front order, and the caps turned toward the viewer are drawn last, the lower item's first. With caps on,
  each side face of the fill's solid that is turned toward the viewer is drawn before every face of the same
  shape's strokes, turned toward the viewer, whose bounds overlap it, so the stroke's walls cover the fill's walls
  at concave corners. This applies to strokes on closed paths without dashes, a width profile or a brush, painted
  with a solid color or a gradient without transparent stops; with other strokes, and without caps, the faces keep
  their depth order. Otherwise
  each solid is drawn on its own with its opacity and blending, the lower item's first. With a light that is not
  white, each solid is also drawn on its own when one of them is a puffed, shaded Inflate surface in a solid color
  (drawn as one composite) or is painted with a gradient that has transparent stops (drawn with an opacity mask).
- The solids turn, twist and taper about the center of the bounds of all their shapes, Inflate's grid covers
  those bounds, and with Perspective one eye sees every solid. A stroke too narrow to hold a point of Inflate's
  grid stays flat.
- A stroke whose outline takes more than 65,536 points, or whose solid takes more than 163,840 faces, is left out,
  and the fill's solid still draws.

## Groups

Extrude & Bevel, Inflate and Rotate also apply to a group or layer. The public documentation says that applied to
several objects one by one each turns about its own axis, while applied to a group or layer all its objects turn
about a shared axis ([Create 3D objects](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/create-3d-objects.html),
said there of Revolve). So the effect on a group builds a solid from every visible path, compound path and type in
it, at any depth of its groups, each from its own fills and strokes as above ([Fills and strokes](#fills-and-strokes)),
and they turn, twist and taper together about the center of their bounds together; with Perspective one eye sees
them all. The members stay editable: moving or editing one reshapes the solids. Our reading:

- Each member's own geometry effects shape it first, then those of each group inside the 3D group that holds it,
  applied to that group's members together; the 3D group's geometry effects before the 3D effect reshape the
  members, and those after it bend the result as a whole. The members' other effects, and the group's own fills and
  strokes, are not drawn. A member's opacity and blending mode apply to its solids, and the blending mode of a group
  inside the 3D group applies to the solids of that group's members together. An opacity mask on
  a member masks its solids, and one on a group inside the 3D group masks the solids of that group's members
  together. The mask stays where it is on the page; it does not turn with the solids. A disabled mask is ignored.
- When a member, or a group inside the 3D group, has an opacity mask or a blending mode other than Normal, each
  solid is drawn on its own, the backmost member's first, as when a solid is transparent.
- When every solid is opaque, all members' faces are drawn in one back-to-front order; otherwise each solid is drawn
  on its own, the backmost member's first.
- Hidden members and guides are left out. A group holding a live object other than a blend (an envelope, a mesh, a
  repeat) is refused with a message saying what to release or expand; so is one with more than 4,096 shapes.

## Clipping groups, blends and symbol instances

Every 3D effect (Extrude & Bevel, Extrude & Bevel (Classic), Inflate, Rotate, Rotate (Classic), Revolve and Revolve (Classic)) also applies to a clipping
group, a blend or a symbol instance, alone or inside a group or layer. The help page says that effects apply to
objects, groups or layers ([Apply and modify effects](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/apply-filter-effects/apply-and-modify-effects.html)),
that a clipping mask hides everything outside its shape ([Clipping masks](https://helpx.adobe.com/illustrator/using/clipping-masks.html)),
and that a blend is treated as one object ([Blending objects](https://helpx.adobe.com/illustrator/using/blending-objects.html)).
How the solid is built from each is VectorCraft's own rule: it is built from what the object shows, as a group's
solids are ([Groups](#groups)), turned about one shared axis.

- A clipping group (or a layer with a clipping mask) builds from its art inside the clipping path: each shape's fill
  area cut to the clip, and each stroke's outline cut to the clip and filled in the stroke's paint. The clipping path
  draws no solid. A clipping group inside another keeps what both clips show. A clip that shows none of the art
  leaves nothing to build, and the effect is refused.
- A blend builds from its key objects and its steps, each in its own paint, in their order.
- A symbol instance builds from its symbol's art where the instance places it. Redefining the symbol reshapes the
  solid. An instance of a symbol that is not in the document, a symbol that holds an instance of itself, or art
  nested more than 64 levels deep is refused.
- Inside a 3D group, the geometry effects of a clipping group, a blend or a symbol instance reshape the art it shows,
  as a group's do. Inside a clipping group, a group's geometry effects reshape its members before the clip cuts them.
- Art whose symbol instances, blends and clipping groups take more than 262,144 objects to go through (counting
  hidden objects, and a symbol's art once for each instance that shows it) is refused.
- The objects stay live: editing the clipping path, the blend's keys or the symbol reshapes the solid.
- An image is refused, alone or in a group: trace it first with Object › Image Trace › Make. The help page routes
  images to 3D through Image Trace ([Create 3D vector artwork](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/create-3d-vector-artwork.html)).
  A hidden image in a group is left out like any hidden member. An image in a clipping group is left out when its
  frame, as its transform places it, lies wholly outside the area that every clipping path around it fills together
  (holes of an even-odd or compound clipping path included), when the image has no visible fills or strokes of its
  own and neither the image nor a group holding it inside the clipping group has effects. A clipping path hides
  everything outside its shape ([the help page](https://helpx.adobe.com/illustrator/using/clipping-masks.html)). An
  image that the clips show any part of is refused. Resolving one effect's art cuts image frames to clipping paths at
  most 1,024 times (VectorCraft's own limit); past that, the art is refused.

## Rotate

The flat art turned in 3D about the centre of its bounds. Every fill and stroke is turned with its own paint (a
stroke as its outline, so it foreshortens with the art) and shaded as one plane; a plane has two sides, so its
back shows when it is turned past 90°. The plane has a seam ([Seams](#seams)).

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

## Shadows

The public [3D and Materials panel options](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/3d-materials-panel-options.html)
page lists shadow settings that apply while shadows are on: a Position behind or below the object, a Distance
from Object from 0 to 100 % and Shadow Bounds from 10 to 400 %. The Lighting tab of Revolve, Extrude & Bevel,
Inflate and Rotate has a **Shadows** switch with those three settings below it.

| Option | |
|---|---|
| Shadows | Off by default. On, the first light casts a hard-edged shadow of the solid, drawn behind it. |
| Position | **Behind Object** (the default): on a plane square to the view, behind the object. **Below Object**: on the floor under the object, square to its own vertical axis and turned with it. |
| Distance from Object | 0–100 %, default 0 %: how far the plane is from the object, as a share of the object's largest size. |
| Shadow Bounds | 10–400 %, default 100 %: how far the plane reaches. At 100 % it is a square four times the object's largest size across, about the foot of the object's center; the shadow is cut at its edges. |

Our reading:

- The shadow is one path, filled black at 50 % opacity with the nonzero rule: every face of every solid of the
  object (its fill and its strokes, or the members of a group) projected along the first light onto the plane.
  Art that is partly transparent casts its part at 50 % times its opacity, and art with a blending mode casts
  its part in that mode, each as a path of its own. A member whose
  opacity mask or blending mode applies to its solids casts its part in that mask and mode too. The color, the
  opacity and how Distance and Bounds scale are our own.
- No shadow is drawn while Shading is off (the Shadows settings are then unavailable), when the first light's
  Intensity is 0 %, or when it does not reach the plane: a light behind the object casts none behind it, and a
  light from under the floor casts none below it. A fill, stroke or group member at 0 % opacity casts none. With no X rotation the floor is seen edge-on, so a
  shadow below draws nothing until the object is tilted (the default Off-Axis Front position shows it).
- Softness is not applied to the shadow, and only the first light casts one.
- A shadow is cast from at most 327,680 faces, the solids of one object together (our own limit). Past it, a
  command is refused with "The 3D shadow is too complex; turn Shadows off or simplify the art", and a loaded
  file draws the source path.
- Canvas, SVG and PDF exports draw the shadow, the art's bounds take it in, and Expand Appearance keeps it as a
  path below the faces. Extrude & Bevel (Classic), Revolve (Classic) and Rotate (Classic) have no shadow.

## Graphics

The public [Map artworks on 3D object](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/map-artwork-on-3d-objects.html)
page describes graphics: art kept with the document and placed on single surfaces of a 3D object, and the
[3D and Materials panel options](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/3d-materials-panel-options.html)
page lists Graphics and Add Materials and Graphics on the Materials tab. VectorCraft maps graphics onto planar faces:
Extrude & Bevel's front and back caps, and the plane of the Plane type (Rotate).

- The panel's **Materials** tab shows the document's graphics as thumbnails. A graphic is a document symbol, as
  the public page allows existing symbols to be used. **Add as Single Graphic** keeps the selected art as a new
  symbol (`threeD.addGraphic`) and leaves the art as it is; art dragged from the artboard onto the thumbnails is
  added the same way. The thumbnails and Add as Single Graphic show without a selected 3D object too.
- Clicking a thumbnail maps that graphic onto the front of the 3D effect of each selected object
  (`threeD.mapGraphic`), in one undo step. A graphic larger than the object starts scaled down to fit inside it.
- Below the thumbnails the tab lists the graphics mapped onto the first selected object. The selected one shows
  its Face (Front or Back; Plane has only a front), X and Y (the offset of its center from the center of the
  object's bounds, in the face's plane), Scale (a percentage of the symbol's size) and Rotation
  (counterclockwise), and **Remove Graphic** takes it off. Each change is one undo step.
- While the tab is open, a blue handle sits on the center of each mapped graphic whose face is turned toward the
  viewer. Dragging a handle moves that graphic over its face, previewed while the pointer is down and recorded as
  one undo step when it is released. The handles are hidden while the selected objects have different graphics,
  and while a geometry effect (such as Transform) comes after the 3D effect. A press on the canvas while its
  context menu is open only closes the menu and does not grab a handle.

These rules are VectorCraft's own:

- A graphic is drawn on the visible face of each of the object's solids (a fill's, and a stroke's outline),
  turned, foreshortened by the perspective and twisted and tapered with the face, and clipped to it. It is shaded
  with the face's light: each color is multiplied by the light that falls on the face, except in pattern tiles,
  which keep their own colors. It is drawn with the
  opacity and blending mode of the fill or stroke the face is painted with. A graphic on a face turned away from
  the viewer is not drawn.
- On the back cap, X runs to the right as the back is seen from behind, so the graphic reads the right way round
  there.
- The symbol's fills and strokes are drawn as filled areas, a stroke as its outline, and their gradients and
  patterns are placed with the graphic. Images, hidden objects,
  guides and clipping paths in the symbol are left out, and clipping groups draw their art unclipped. Instances
  of other symbols in it are drawn. Symbols may nest in its art up to 8 levels deep, and its objects may be nested
  up to 128 levels deep; commands refuse a graphic past either limit, and a loaded file draws the object without it.
- Without perspective, the graphic maps exactly. With perspective, straight lines stay straight and each curve is
  cut into 8 cubic pieces before it is mapped. The part of a graphic at or behind the eye is left out.
- An object takes up to 64 graphics. One graphic draws up to 4,096 fills and strokes, and the graphics of one 3D
  effect draw up to 400,000 path elements together. Commands refuse a graphic past either limit. A loaded file
  draws the object without a graphic past the first limit, and without all of its graphics past the second.
- A hollow Extrude & Bevel has no caps, so its graphics are not drawn until Cap is turned on again, and
  `threeD.mapGraphic` refuses to map a new one onto it. Changing the type to Revolve or Inflate
  drops the graphics, and changing between Extrude and Plane keeps them, except that Plane drops the graphics
  on the back cap.
- `effect.apply` and `effect.setParams` refuse a graphic that names a symbol the document does not have, unless
  the effect already holds a graphic of that symbol. A graphic whose symbol is deleted is no longer drawn. Renaming a symbol renames it in the graphics that use it,
  in the layers, in the art of symbols, patterns, graph designs, brushes and opacity masks, and in graphic
  styles, and Select All Unused in the Symbols panel counts a symbol a graphic uses as used, a graphic in a graphic style included.

## Parameters (`effect.apply`, `effect.setParams`)

`threeD.extrude`: `depth`, `twist`, `taper` (%), `cap`, `bevel` (`none`, `classic`, `rounded`, `cove`, `step`,
`ogee`), `bevelWidth`, `bevelHeight`, `bevelRepeat`, `bevelSpace` (%), `bevelInside`, `bevelBothSides`.
`threeD.inflate`: `depth`, `volume` (%), `bothSides`. All three take the view and lighting parameters Revolve
uses: `rotationX`, `rotationY`, `rotationZ`, `perspectiveAngle` (degrees), `shade`, `lightColor` (`#rrggbb`, default `#ffffff`), `lightIntensity` (%), `lightRotation`,
`lightHeight` (degrees), `lightSoftness` (%), `ambient` (%, to 200), `lightBehind`. Lengths are points and scale with the object (Scale Strokes & Effects).
Commands validate the shape and the parameters before the document changes. All four also take `lights`: null (the default, use the earlier single-light fields), or an array of one to ten objects. Each object takes `color` (opaque `#rrggbb`, white), `intensity` (0..100%, 80), `rotation` (-180..180 degrees, 125.26), `height` (0..90 degrees, 30), `softness` (0..100%, 0), and `behind` (boolean, false). Omitted settings use those defaults. An explicit list replaces the earlier single light; `ambient` and `shade` remain shared. Set `lights` to null to return to the single-light fields. Invalid lists, counts, keys and values are rejected before editing; malformed loaded lists fall back to the earlier light safely. All four take `shadow` (boolean, false), `shadowPosition` (`behind` or `below`), `shadowDistance` (0..100%, 0) and `shadowBounds` (10..400%, 100) ([Shadows](#shadows)); values out of range are rejected before editing.
`threeD.extrude` and `threeD.rotate` take `graphics` ([Graphics](#graphics)): an array of up to 64 objects, each
with `symbol` (the name of a document symbol), `face` (`front` or `back`, default `front`; `threeD.rotate` takes
`front` only), `x` and `y` (points, default 0), `scale` (1..10000%, default 100) and `rotation` (-360..360 degrees,
default 0). Other keys, values out of range and graphics on the other 3D effects are rejected before editing.

## What differs from the public documentation

Behaviour reference: the public [3D and Materials panel options](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/3d-materials-panel-options.html)
and [Create 3D objects](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/create-3d-objects.html)
pages, and the archived Extrude & Bevel option list (depth 0–2,000, cap on/off, bevel height, bevel extent in or
out, perspective). No Adobe software, assets, presets or bevel shapes were used.

- **Materials** (base and Adobe Substance), graphics on curved surfaces (Revolve, Inflate and Extrude & Bevel's
  walls and bevels), soft shadows, shadows of lights after the first, and **ray-traced rendering** are not built:
  shading uses directional lights, in the paint of the fill or stroke the solid is built for
  ([Fills and strokes](#fills-and-strokes)), and the shadow is hard-edged ([Shadows](#shadows)). Graphics map
  onto planar faces only ([Graphics](#graphics)).
- The bevel shapes and their names are our own; custom bevel paths are not supported.
- Visibility is a painter's algorithm: faces of a closed solid turned away are left out, and the rest are drawn
  back to front, with a flat cap or flat back turned toward the viewer drawn after all of them. Concave solids
  seen at steep angles can overlap wrongly; open (hollow) solids show both sides. Twisted walls and bevels are
  drawn in pieces about as long as a wall slice is wide (fewer near the limit of 32,768 facets). The piece drawn
  on top can still lie a few points behind another nearby piece.
- Extrude & Bevel and Rotate retain planar shading. Colored Inflate uses smooth vector lighting; white-light Inflate keeps its original polygon shading; bounded cell triangulation leaves a cell flat when it contains a complete hole or an unusually complex/degenerate contour. Its geometric silhouette is still a polygon approximation.
- Paths, compound paths, type, clipping groups, blends, symbol instances and groups or layers of them (see
  [Groups](#groups) and [Clipping groups, blends and symbol instances](#clipping-groups-blends-and-symbol-instances));
  images have to be traced first, and other live objects expanded. Revolve on a group sweeps its members about one
  axis ([revolve.md](revolve.md)). One 3D effect per object.
- Curves are followed within 1/2000 of the shape's size (at least 0.05 pt), so a small curve takes a few points
  and a large one up to 32. Extrude & Bevel and Inflate take up to 65,536 outline points and 163,840 faces for each
  solid (a stroke's outline is counted on its own), and Rotate (which only turns the outline) takes 65,536 points.
  These limits are VectorCraft's own values. Before any point is sampled, Extrude & Bevel counts the faces its walls,
  bevels and caps need, and Inflate counts the faces of its sides; when they do not fit, every curve of the shape
  takes fewer pieces, down to 4 (a curve that needs fewer takes fewer). A shape that does not fit at 4 pieces a
  curve is refused because the 3D surface is too complex. Inflate counts the faces of its front and back as it
  builds them, against the same 163,840. The point limit is counted on the shape as drawn and, when its contours
  meet, again on the merged outline, which can have more points (bars crossing in a grid trace every hole between
  them); past it, the shape is too complex for the effect.
- The faces drawn, exported and made by **Object › Expand Appearance** keep the shape's curves: the caps of
  Extrude & Bevel, the flat face of Rotate, and the edges of the walls and bevels where they meet a cap are cubic
  curves, with smooth anchors where the art's are smooth, while visibility and depth are worked out on the
  sampled points. Without perspective and bevel each curve maps exactly. With perspective, a curve is split into
  cubics until each lies within 0.001 pt of the projected outline. A face inset or outset by a bevel follows its
  sampled outline within the sampling tolerance above, one curve for each sampled edge through its two sampled
  points, so the cap and the walls next to it draw the edge they share as the same curves. The edges between two bevel steps or two wall slices, a
  piece of curve that does not fit after 8 halvings, an edge of a twisted wall cut into pieces, and the curves
  past 1,048,576 for one solid stay straight. The tolerances and limits are VectorCraft's own values. Inflate's
  faces are polygons.
- Inflate also limits the work of filling its grid, which grows with the outline's points times the grid cells its
  outline crosses. An outline of tens of thousands of points that crosses nearly every cell (a comb of 4,000 teeth
  across the shape) is refused because the 3D surface is too complex; a closed path of 65,536 points round a
  circle builds.
- **3D (Classic)**: Extrude & Bevel (Classic), Revolve (Classic) and Rotate (Classic) are built, with their surface
  types and blend steps. Revolve (Classic) sweeps the surface Revolve sweeps, at 64 segments for a full turn, and draws
  every face back to front, so it has no Draw Hidden Faces setting. Rotate (Classic) turns the plane Rotate turns, seen
  from either side, with the No Shading and Diffuse surfaces only. The track cube, map art, Turntable and 3D export
  are separate, unbuilt items.

Release measurements and a repeatable CLI scene are in [3D lighting performance](3d-lighting-performance.md).

## Revision history

| Date | Change |
|---|---|
| 2026-11-12 | The seams of Extrude & Bevel, Inflate and Rotate under a white light, and of the faces of Extrude & Bevel (Classic), Revolve (Classic) and Rotate (Classic), went from miter to round joins (the seams of Revolve and of colored lights, and 3D (Classic) Wireframe edges, were already round): no thorns on edge-on facets, and facets with no neighbor now have rounded convex corners ([Seams](#seams)). |
| 2026-11-08 | Contours that cross, touch or overlap are merged by vectorcraft-pathops before Extrude & Bevel, Inflate and Rotate build a solid, sharing its limits; others are built as drawn. |
| 2026-11-06 | Extrude & Bevel, Inflate and Rotate apply to groups and layers: every path, compound path and type in them turns about one shared axis (#4506). |
| 2026-10-21 | Four original lighting presets, shared by all four 3D effects and commands, with effective-setting recognition and existing preview/history behavior. |
| 2026-10-17 | Smooth colored Inflate lighting and surface batching; legacy white-light appearance and the 0.05 seam minimum preserved; shared light Color and multiple lights remain vector output. |
