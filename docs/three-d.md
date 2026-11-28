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
- **Render Settings › Wireframe**, under the object types, draws each selected 3D object as the edges of its
  faces instead of its shaded surface, as the public
  [Render 3D vector artwork](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/render-3d-vector-artwork.html)
  page describes for editing Depth, Taper and Twist in a mesh view. The item is checked while the first selected
  object's effect is in wireframe mode, and choosing it turns the mode on or off for every selected Extrude & Bevel,
  Revolve, Inflate and Rotate (`effect.setParams` with `render: {mode}`), as one undo step. The effect stays live:
  each edit to its settings redraws the edges. The edges are the ones Expand as Wireframes gives, each once, with a
  0.5 pt black stroke, round joins and no fill (VectorCraft's own choice). Geometry effects after the 3D effect
  reshape the drawn edges, as they reshape shaded art. Expand as Wireframes reshapes each face before it joins the
  edges, so with such an effect its paths can differ from the drawn ones. The mesh shows every edge whatever the
  opacity, blending mode or opacity mask of the fills, strokes and group members, as Expand as Wireframes does;
  the object's own opacity, blending mode and opacity mask apply to it as a whole. The shadow and the graphics are
  not drawn.
  The mode is saved with the effect and drawn in every output, and Expand Appearance gives the edges as paths.
  Choosing Wireframe again draws the shaded surface. The 3D (Classic) effects keep their own Wireframe surface.
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
- **Export 3D Object** is a button in the panel, as the public
  [Export 3D vector artwork](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/export-3d-vector-artwork.html)
  page describes; VectorCraft places it at the bottom of the panel. It adds each selected object that has a visible
  3D effect to Asset Export as a 3D asset, the same as
  Asset Export's own Export 3D Object (`assets.add3D` on the selection), selects their assets in the Asset
  Export panel and shows that panel, where the Format row and Export write the mesh files
  ([3D assets in mcp.md](mcp.md#3d-assets)). Selected objects without a visible 3D effect are left out, and so are
  a group whose 3D effect is on a fill or stroke, which draws nothing to export, and a 3D object inside another
  selected 3D object, whose asset holds it. The button is unavailable while no selected object has a 3D effect that
  can be exported. Adding the assets is one undo step; when every one of the objects is an asset already, nothing is
  recorded and their assets are selected again.
- The Lighting tab also holds the Shadows switch and its settings ([Shadows](#shadows)). The Materials tab holds
  the Graphics ([Graphics](#graphics)); base materials and ray-traced rendering are not built (see below). The
  rotation and light rings on the artboard are part of the options dialog only. The panel's Lighting tab has the
  lighting widget instead ([Lighting widget](#lighting-widget)).

## 3D meshes

The meshes that Export 3D Object writes come from the `three-d` crate's `Mesh`, built from the same faces the effect
draws. The vector facets on the canvas and in SVG and PDF do not change with what follows. Besides positions,
normals and triangles, a mesh carries:

- **Normals.** A corner takes its face's normal. Where two bands of one bevel meet, the shared corners take the mean of
  the two bands' normals, unless the bevel turns there by 60° or more (a Step profile's turns) or one of the bands lies
  flat toward the front or back; caps, walls and the edges between a bevel and a cap or wall stay hard. On Inflate's
  puffed front and back each corner takes the surface's own normal. The bevel settings shape its profile into a
  rounded curve or an ornamental one ([the help page](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/3d-materials-panel-options.html));
  the smoothing rule and the 60° crease are VectorCraft's own.
- **Texture coordinates** for every corner, each from 0 to 1, u to the right and v down, as glTF 2.0 places
  `TEXCOORD_0` (<https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html>). Caps, Rotate's plane and Inflate's
  front and back take each corner's art point, before any twist or taper, in the bounds of the whole object, which all
  of its parts share; the back is not mirrored. Extrude & Bevel's bevels and walls and Inflate's walls take u along each contour of the outline from its
  first point, as the distance over the contour's length, so the seam at the first point has corners at both 0 and 1,
  and v across the side from the front cap's edge to the back cap's. Revolve's side takes u around the sweep from its
  start and v along each contour of the profile; its end caps span the disk they close, and its cut sides take each
  corner's profile point in the profile's bounds. An axis with no extent takes 0.5. These layouts are VectorCraft's
  own.
- **A part for each triangle**: `Front` (a front cap, Rotate's plane, Inflate's front), `Back`, `Side` (walls and
  Revolve's swept side), `FrontBevel`, `BackBevel`, `Cap` (Revolve's end caps) and `Cut` (the flat sides of a Revolve
  of less than 360°).

The exported files write the normals and the texture coordinates of every corner: OBJ's `vt` (up from the bottom),
glTF's `TEXCOORD_0` and USD's `primvars:st` (up from the bottom). A face with mapped graphics writes the coordinates
of its texture instead, laid out as the graphics are ([Graphics](#graphics)). The files do not write the parts. A
mesh whose lists do not match, or that holds a value that is not a number, stops the export with an error before any
file is written.

## Extrude & Bevel

The art is the front face; its outline is extruded backwards by **Depth** (0–2,000 pt), so seen straight on the
object is where the art was.

| Option | What it does |
|---|---|
| Depth | 0–2,000 pt. At 0 the object is flat (the art turned like Rotate). |
| Twist | 0° to 360°: the back face turned against the front, the walls following in slices. |
| Taper | 1–100 %: the back face's size against the front's. Without a twist, each wall (and each bevel band) stays flat, so it is one face, not cut into slices. |
| Cap | **Solid** closes the front and back; **Hollow** leaves them open, showing the insides of the walls. |
| Bevel Shape | **None**, **Classic** (a straight chamfer), **Rounded** (convex quarter round), **Cove** (concave), **Step** (two steps) or **Ogee** (an S curve). Our own cross-sections. After them come the custom bevels ([Custom bevels](#custom-bevels)). |
| Width, Height | How far the bevel reaches across the face and along the depth (0–1,000 pt each). The height is limited to the depth (half of it on each side with Bevel both sides). |
| Repeat, Space | The cross-section repeated 1–32 times across the width, with a share (0–90 %) of the width flat between repeats. |
| Bevel Inside | The bevel carved into the shape (the front face shrinks) instead of added around it (the walls stand outside the art). |
| Bevel both sides | The back is bevelled too. |
| Reset | Next to the Bevel heading, in the dialog and the panel. It sets every bevel setting above back to its default (Bevel Shape None, Width and Height 4 pt, Repeat 1, Space 0 %, both checkboxes off) and leaves Depth, Twist, Taper, Cap, the rotation, the lights and the graphics as they are, as the public [3D and Materials panel options](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/3d-materials-panel-options.html) page describes. In the panel it sets them on each selected Extrude & Bevel as one undo step. It is unavailable while every bevel setting is at its default. |

Holes stay holes, with walls of their own. Bevels move each corner along its bisector. A corner the bevel opens (a
sharp point with the bevel outside, a narrow notch with it inside) whose miter would reach past four times the width is
cut square across instead; at the other corners the miter is limited to four times the width. An inside bevel on
a single straight triangle or parallelogram with ordinary miters stops before its edges meet, keeping the requested
bevel height. Other outlines can still fold where a bevel is wider than a narrow part of the shape. Each facet has a seam ([Seams](#seams)).

### Custom bevels

The public [Add custom bevel paths](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/add-custom-bevel-paths.html)
page describes a custom bevel as one open path saved as a symbol in a bevels document, then listed in the Bevel menu.
VectorCraft keeps its custom bevels in `Bevels.vectorcraft`, a native document next to the preferences (none on the
web or in runs with `VECTORCRAFT_NO_PREFS`). The layout below is VectorCraft's own.

- **Edit Bevels…** under Bevel Shape in the 3D and Materials panel (`threeD.bevels.edit`) opens the file, and first
  creates it with one example bevel when it is missing.
- Each symbol whose art is one open path (alone or in groups of one member) is one bevel, named after the symbol and
  listed after the built-in shapes in the Bevel Shape menus of Extrude & Bevel and Extrude & Bevel (Classic).
- The path runs from its first anchor, at the cap's edge, to its last, at the side wall. Across the page is across
  the bevel's width and down the page is along its height, whichever way the path is drawn. Points past its ends are
  held to them, curves are cut into straight lines, and a path is simplified to at most 64 points.
- A custom bevel takes Width, Height, Repeat, Space, Bevel Inside and Bevel both sides like the built-in shapes.
- The file is read when the app starts and again each time VectorCraft saves it; `threeD.bevels.reload` reads it
  again. A file larger than 64 MB is not read, at most 256 symbols are read, and a symbol that is not one open path,
  has more than 4,096 anchors, is cut into more than 8,192 straight lines, has ends that are not apart both across and down, or repeats an earlier name is listed
  in `skipped` by `threeD.bevels.list`.
- An effect that uses a custom bevel keeps its name and a copy of its points, so the document draws it the same way
  without the bevels file.

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

The surface starts with a grid of 48 cells across the larger side, halved or quartered where it rises steeply near the outline. White-light Inflate retains the existing clipped polygon cells, shading and paint composition. With colored lights, simple cells are triangulated with shared border samples, and lighting interpolates shared membrane normals through ordinary RGB vector gradients, including on a puffed back. A solid fill or stroke drawn alone, or drawn whole because a fill or stroke of the object is translucent or blended, batches those triangles into three channel groups and one coverage clip for the whole surface; when its faces share one depth order with another solid's, each triangle has its own clip and channel ramps. Source gradients retain per-facet vector composition and apply transparent coverage once. Seam widths retain the 0.05 to 0.75 pt bounds, and every facet's seam has round joins ([Seams](#seams)). Shading is automatic, with no additional dialog option.

## Seams

Facets side by side, each antialiased on its own, would leave hairlines between them, so 3D facets are drawn with
a seam of their own paint, with round joins. In Extrude & Bevel, Inflate, Rotate and the three 3D (Classic) effects a
facet's seam is half as wide as the facet is thick, from 0.05 to 0.75 pt; Revolve's seam is always 0.75 pt.

How a facet carries its seam depends on the lights and the paint:

- Extrude & Bevel, Inflate and Rotate, when no light is colored (one white light or several): a path filled with
  the facet's shading and stroked with its seam. With a colored light, Extrude & Bevel and Rotate (planar shading)
  are drawn the same way.
- Inflate with a colored light and shading on: a solid-color fill or stroke drawn as one composite draws each facet's light ramps with its seam in three channel
  groups clipped to the union of its facets, so no seam reaches past the surface; a gradient fill, and a solid-color
  fill or stroke whose faces share one depth order with another solid's, fills each facet's ramps through a clip of
  the facet and its seam, outlined with round joins.
  The facets of a surface drawn this way lie over an underlay
  of themselves whose seams are 2 pt wide, reaching 1 pt past each edge, and one clip to the union of the facets and
  their seams removes what the underlay paints past the surface. Where two facets' antialiased edges meet, the
  underlay fills the pixels they leave partly uncovered with the neighbors' own light, so no black channel backdrop
  or background shows there. Compared with the same art drawn at 3x and averaged down, away from the surface's outline
  and from places where the light changes sharply, a pixel drawn at 1:1 is opaque where the 3x art is opaque, and
  otherwise its alpha is within 3 levels. Inflate seen straight on with an opaque fill also has each color channel
  within 3 levels (VectorCraft's own measure).
  In the composite, a facet that is one closed polygon whose edges do not cross is drawn over the underlay as one
  filled path, the facet and the outline of its round seam together, so a pixel at its edge is covered once and not
  by a fill and then by a stroke of the same color. Other facets keep a fill and a seam stroke. Turned to X 50°,
  Y 35° in an opaque fill, at most one color channel sample is more than 3 levels off the 3x art, and by at most 4
  (VectorCraft's own measure).
- Extrude & Bevel (Classic), Rotate (Classic) and Revolve (Classic): a path and a seam for every face; gradient and
  pattern faces seam only the base paint, and the shading and highlight layers cover the face alone. The Wireframe
  surface draws edges only, with no seam.
- Revolve, with shading on: when a `lights` list is given (even one of white lights), a solid-color or gradient face fills its light
  ramps through a clip of the face and its 0.75 pt seam, outlined with round joins. With the single legacy light,
  a face is a path and a seam; with a colored `lightColor` a gradient face fills through the same kind of clip.
  Under a colored `lightColor` or a `lights` list, the faces of a solid-color or gradient surface lie over an underlay
  of themselves under one clip, as colored Inflate's facets do; in a group, each run of one member's faces in the
  shared depth order takes its own underlay, and a face with a part of a mapped graphic keeps that part right after
  it. Faces in a gradient with transparent stops carry its opacity mask and are drawn without an underlay. At 1:1,
  away from the surface's outline, a pixel is opaque where the same art drawn at 3x and averaged down is opaque
  (VectorCraft's own measure).

A facet seen almost edge-on is a sliver, and a seam with mitered joins would reach past the solid as a thorn; a round
one reaches no more than half its width past its facet. So where a facet has no neighbor (Rotate's plane, every
silhouette corner) its convex corners are drawn rounded by up to 0.375 pt (half the largest seam), except a
solid-color Inflate surface under a colored light drawn as one composite, whose clip keeps its corners sharp.

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
  with a solid color or a gradient without transparent stops, under Extrude & Bevel (both kinds). Under Inflate
  the fill can rise in front of such a stroke, so it is drawn like the other strokes. Other strokes (dashed, with a
  width profile or a brush, on an open subpath, or in a paint that may show through) leave parts of the fill's edge
  uncovered, such as the gaps between dashes. Each face of such a stroke that passes through a face of the fill's
  solid, or through a face of another of the shape's strokes, where the two overlap in the drawing is cut where
  their depths meet, and the faces of the shape's fill and strokes that overlap are drawn in the order of their
  depths where they overlap, with caps on or off. A fill wall in a gap stays in its own depth order. Where such
  faces lie in front of one another in a ring, so that no order draws them all correctly, a ring that waits for no
  face outside it gives up its own face that comes first in the order of their depths, and the faces left are
  looked at afresh, as they may be a smaller ring or none. A face that is only in front of a face of a ring is
  still drawn after that face. Comparing the faces and looking for rings share one limit of 4,000,000 steps
  (VectorCraft's own). Rings of a few faces take a few steps a face. A ring that is still a ring after it gives up
  a face is walked again, so a large ring can cost about the square of its faces (some 4,000 faces tangled at
  random reach the limit). The lists a ring is looked for in are made only when the steps left are at least a
  quarter of the pairs, so a solid whose comparing uses nearly all of the limit breaks its rings as before; and a
  face that waits only for a face drawn last goes in its turn with no search. Past the limit the first face left by
  depth goes next, ring or not. Inflate's curved surfaces follow this only as closely as their flat facets do. A
  solid painted with a gradient that has transparent stops under a light that is not white is drawn with an opacity
  mask: its faces keep their places in this order, in runs that each carry the mask, and a graphic drawn after one
  of its faces carries the mask too ([Graphics](#graphics)). Such a gradient, and a gradient with transparent stops
  under the classic effects' shading, is placed once on the whole solid when it has no placement of its own, so its
  colors and its transparency line up across the faces. A member of a 3D group with its own opacity mask or
  blending mode draws its fill's and strokes' faces in this order inside that mask or blending mode when they are
  opaque with Normal blending. Otherwise each solid is drawn on its own with its opacity and blending, the lower
  item's first. With a light that is not white, a puffed, shaded Inflate surface in a solid color takes its place
  in this order too, each of its faces drawn with its own light ramps; a lone solid of that kind is still drawn as
  one composite, and so is each such solid when they hold more than 30,720 faces together.
- The solids turn, twist and taper about the center of the bounds of all their shapes, Inflate's grid covers
  those bounds, and with Perspective one eye sees every solid. A stroke too narrow to hold a point of Inflate's
  grid stays flat.
- A stroke whose outline takes more than 65,536 points, or whose solid takes more than 163,840 faces, is left out,
  and the fill's solid still draws.
- The solids of one Extrude & Bevel or Inflate effect keep up to 163,840 faces together: the fill's and every
  stroke's, and on a group or layer every member's (VectorCraft's own limit). The solids are built in order, and
  once their faces pass it no later solid is built: the object draws its source art, and applying or editing the
  effect, Export 3D Object and **Expand as Wireframes** report that the 3D surface is too complex. A stroke left
  out for its own limits does not count.
- Inflate works out how high every shape puffs before it builds the first solid, because the highest puff sets
  how far away the eye is. At a Volume of 0 % no shape puffs, so each shape's puff is worked out only when its
  solid is built, and none is worked out for the solids after the limit stops. The shadow builds no solid for a
  fill or stroke that is fully transparent.

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
- When a member, or a group inside the 3D group, has an opacity mask or a blending mode other than Normal, the
  members' faces are not drawn in one shared order. Each member whose fills and strokes are opaque with Normal
  blending draws their faces in one order of its own inside its masks and blending modes, as described under
  [Fills and strokes](#fills-and-strokes). Otherwise each solid is drawn on its own, the backmost member's first, as
  when a solid is transparent.
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

## Track cube

The Rotation tab of Extrude & Bevel (Classic), Revolve (Classic) and Rotate (Classic) has a track cube below the
presets, drawn at the current X, Y and Z rotation inside a ring
([set 3D rotation position options](https://web.archive.org/web/20220911143322/https://helpx.adobe.com/pdf/cs6/illustrator_reference.pdf#page=249)):

- Dragging a face turns the object freely. With Shift, a drag across turns it about the screen's vertical axis and a
  drag up or down about its horizontal axis, whichever the drag follows first.
- Dragging an edge turns it about the object axis that edge runs along. The edge under the pointer shows its axis's
  color: red for X, green for Y, blue for Z.
- Dragging in the ring turns it in the plane of the screen.
- The front face is blue, the top and bottom are light gray, the sides are mid gray and the back is dark gray, as
  the help page above describes, along with the red, green and blue axis edges.
- Escape during a drag puts the angles back. With the keyboard focus, the arrow keys turn it about the screen's
  horizontal and vertical axes, and Page Up and Page Down turn it in the screen plane, 1° a press or 15° with Shift.

The drag changes the effect's `rotationX`, `rotationY` and `rotationZ` and previews live; X, Y and Z and the
presets follow it (**Custom** off a preset). OK keeps the change as one undo step. The angles stay between -180°
and 180°: of the two sets of angles that give the same turn, the cube writes the one nearest the current values.
Untouched axes retain their original precision; only a changed axis is rounded to the fields' precision. Page Up
and Page Down change Z while preserving fractional X and Y values.
In the 3D and Materials panel, a drag is one undo step when the pointer is released, and the cube is off while the
selected effects hold different rotations. The cube's 0.5° per point of drag, Escape during a drag and its keyboard
steps are VectorCraft's own.

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

### Lighting widget

On the 3D and Materials panel's Lighting tab, above the list of lights, a sphere shows every light of the
effect where it shines from, as the public
[3D and Materials panel options](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/3d-materials-panel-options.html)
page describes for dragging light sources around the object. The sphere is seen from the front: a light's
Rotation is its angle around the center, and its Height runs from the rim (0°) to the center (90°). A light in
front of the object is a sun filled with its color, and a light behind the object is hollow. The selected light
is drawn larger, above the others.

- Pressing a light selects it in the list of lights and records nothing. Dragging it moves it on its side of the
  object, rounded to 0.1°; Shift snaps Rotation and Height to 15°. The drag is previewed while the pointer is
  down and is one undo step when it is released.
- **Move Light to Back** moves the selected light behind the object, and **Move Light to Front** moves it back
  in front, as the page's "Move selected light to the back/front of object" does. It sets the same
  `lightBehind` (or a list light's `behind`) as the **Light behind the object** checkbox.
- Each light takes the keyboard focus. Left and Right turn the focused light around the object by 1°
  (counterclockwise and clockwise), Up and Down raise and lower it by 1°, and Shift makes each step 15°. Each
  press changes only the setting it moves and is one undo step. A
  step past 90° or below 0° Height changes nothing. Screen readers name each light with its number, Rotation,
  Height and, for a light behind the object, its side.

The widget sends the same settings as the Rotation, Height and Light behind the object controls
(`effect.setParams`); there is no command of its own. The sphere's look, the 0.1° rounding and the arrow keys
are VectorCraft's own. The options dialogs of Extrude & Bevel, Revolve, Inflate and Rotate keep the light rings
on the artboard instead.

The Lighting tab of Extrude & Bevel (Classic), Revolve (Classic) and Rotate (Classic) has the same sphere, in
the options dialog as well as in the panel, while the surface is Diffuse or Plastic, as the archived
[lighting options](https://help.adobe.com/archive/en_US/illustrator/cs5/illustrator_cs5_help.pdf#page=272) of
the classic 3D dialogs describe. Their lights have no color, so every sun is white. The light rings on the
artboard stay available in the dialog. **Add Light** adds a light at the front center of the sphere (Rotation
0°, Height 90°), and **Delete Light** removes the selected light and is unavailable while one light is left.

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
| Shadows | Off by default. On, the first light casts a shadow of the solid, drawn behind it: hard-edged at the light's Softness 0 %, blurred above it. |
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
- The first light's Softness blurs the shadow with a Gaussian Blur: at 0 % it stays the hard-edged path, and above
  it the blur radius is Softness × the object's largest size × (5 % + 20 % × Distance from Object), at most
  1,000 pt. At 100 % Softness a 100 pt object casts a shadow blurred by 5 pt on a plane that touches it and by
  25 pt at 100 % Distance. This mapping is our own. The blur scales with the object, and it draws, takes room and
  exports as a Gaussian Blur effect does. Only the first light casts a
  shadow.
- A shadow is cast from at most 327,680 faces, the solids of one object together (our own limit). Past it, a
  command is refused with "The 3D shadow is too complex; turn Shadows off or simplify the art", and a loaded
  file draws the source path. A command checks the shadow from the same builds that check the solids, as
  drawing casts it, so each solid is built once. A stroke whose solid is left out of the art (past its own
  limits) casts no shadow.
- Canvas, SVG and PDF exports draw the shadow, the art's bounds take it in (with its blur), and Expand Appearance
  keeps it below the faces: a hard shadow as a path, a blurred one as an image. Extrude & Bevel (Classic), Revolve (Classic) and Rotate (Classic) have no shadow.

## Graphics

The public [Map artworks on 3D object](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/map-artwork-on-3d-objects.html)
page describes graphics: art kept with the document and placed on single surfaces of a 3D object, and the
[3D and Materials panel options](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/3d-materials-panel-options.html)
page lists Graphics and Add Materials and Graphics on the Materials tab. VectorCraft maps graphics onto
Extrude & Bevel's front and back caps and its side (its walls and bevels), the plane of the Plane type (Rotate),
the swept side of Revolve, which the public page's example of a label on a bottle describes, and Inflate's puffed
front and back.

- The panel's **Materials** tab shows the document's graphics as thumbnails. A graphic is a document symbol, as
  the public page allows existing symbols to be used. **Add as Single Graphic** keeps the selected art as a new
  symbol (`threeD.addGraphic`) and leaves the art as it is; art dragged from the artboard onto the thumbnails is
  added the same way. The thumbnails and Add as Single Graphic show without a selected 3D object too.
- Clicking a thumbnail maps that graphic onto the front of the 3D effect of each selected object
  (`threeD.mapGraphic`), in one undo step. A graphic larger than the object starts scaled down to fit inside it.
- Below the thumbnails the tab lists the graphics mapped onto the first selected object. The selected one shows
  its Face (Front, Back or Side, on Inflate Front or Back for its puffed front or back; Plane and Revolve show no
  Face, as a plane has only a front and Revolve's graphics go on its side), X and Y (the offset of its center from
  the center of the object's bounds, in the face's plane, or on a side from its middle unrolled flat), Scale (a percentage of the symbol's size) and Rotation
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
  opacity and blending mode of the fill or stroke the face is painted with. Solids whose fills and strokes have the
  same opacity and blending mode share one graphic on each face, clipped to their faces there and drawn after the
  last of them. An object's fill and stroke share it when the fill is drawn with an opacity mask too, and the
  graphic takes the mask of the solid it is drawn after; members of a 3D group drawn with a mask keep graphics of
  their own. A graphic on a face turned away from the viewer is not drawn.
- On the back cap, X runs to the right as the back is seen from behind, so the graphic reads the right way round
  there.
- On Inflate a graphic is placed in the art's plane, X and Y measured from the center of the object's bounds as on
  a cap, and carried straight out onto the puffed face above each point, so it follows the puff. The back takes
  graphics only with Both sides and Cap on (`threeD.mapGraphic` refuses one otherwise, and one already there is
  not drawn); it reads the right way round from behind, and Twist and Taper turn and shrink its graphics with it.
  The membrane is cut into triangles, and each one draws the part of the graphic that falls on it, shaded as
  that triangle is shaded and drawn only where the surface's outside faces the viewer; nearer triangles drawn
  after it cover it. When a cell of the membrane cannot be cut into triangles, the object draws without its
  graphics and commands refuse them, as when the triangles take more faces than a 3D effect draws. Every solid of
  the object (a fill's, and a stroke's outline) carries the graphics. The handle sits on the point of the puffed
  face above the graphic's center, and only where the viewer sees that face. The cutting into straight pieces,
  the seam strokes and the limits below are those of Revolve; when the triangles take more faces than a 3D
  effect draws, the object draws without its graphics and commands refuse them.
- On Revolve a graphic lies on the swept side, which has one face (Front), unrolled flat. X runs around the axis:
  the angle from the middle of the side times the profile's distance from the axis at that point, so a graphic
  keeps its size at every distance and wraps farther around the axis where the profile comes nearer to it. Y runs
  along the profile, toward its lower end, measured from the middle of its length; a profile's edges on the axis
  sweep nothing and add no length. On a closed profile that does not touch the axis, Y is measured from where its
  outer side crosses the middle of its height, and the graphic is cut on its inner side. The middle of the side
  is the middle of the part of the sweep that faces the viewer before the object is turned, which is the part at
  90° with the axis on the left and at 270° with it on the right (the middle of the sweep when no part faces the
  viewer). On a compound profile the graphics go on its longest contour. Each face
  of the side draws the part of the graphic that falls on it, shaded as the face is shaded, so the graphic narrows
  toward the silhouette and is not drawn where the side's outside faces away or past the ends of the sweep; nearer
  faces drawn after it cover it. A full turn cuts the graphic at the back, on the edge between two segments nearest
  half a turn from the middle, and the end caps take no graphic; a part
  that runs past the end of the profile reaches the curved rim of the cap and stops there. On a group or layer the
  graphics go once, on the side of the member with the longest profile. Curves
  are cut into straight pieces within 0.05 pt before they are mapped, and an opaque part takes a 0.3 pt stroke of
  its paint over the seams between faces; a part filled with a pattern, or whose gradient has a transparent stop
  or point, takes none. A graphic larger than the half of the side seen from the front starts
  scaled to fit it. The panel shows no handle for a Revolve's graphics; X and Y move them. Expand Appearance keeps
  every face of a surface with graphics.
- On Extrude & Bevel a graphic on the Side lies on the walls and bevels, unrolled flat. X runs along the art's own
  outline (the front cap's edge, or with a bevel carved into the shape the walls' outline), through its corners and
  around its curves: 0 is where the outline's top crosses the vertical line
  through the middle of its bounds, and X grows clockwise around the outline as the front is seen, so the graphic
  reads the right way round from outside. Y runs from the front cap's edge toward the back cap's, measured from the
  middle: across each bevel step along its slant, and across the walls along the depth. Both are lengths before
  the twist and the taper, which carry the graphic with the side. The side is cut at the anchor of the outline
  nearest half way around from X = 0. On a compound shape the graphics go on its longest contour, and on an object
  with several solids (a fill and its stroke's outline, or the members of a group or layer) they go once, on the
  side of the solid with the longest outline. Each piece of the side that shows its outside to the viewer draws the
  part of the graphic that falls on it, shaded as the piece is, with the same cutting and seams as on Revolve; a
  piece of a stroke's face cut where it passes through a wall draws the part within its own outline. A graphic larger than the
  part of the side unrolled about X = 0 that does not reach its cut starts scaled to fit it. The blue handle sits on the graphic's center while that point is on a
  piece the viewer sees, and dragging it moves the graphic over the visible part of the side.
- The symbol's fills and strokes are drawn as filled areas, a stroke as its outline, and their gradients and
  patterns are placed with the graphic. Images, hidden objects,
  guides and clipping paths in the symbol are left out, and clipping groups draw their art unclipped. Instances
  of other symbols in it are drawn. Symbols may nest in its art up to 8 levels deep, and its objects may be nested
  up to 128 levels deep; commands refuse a graphic past either limit, and a loaded file draws the object without it.
- Without perspective, the graphic maps exactly. With perspective, straight lines stay straight and each curve is
  cut into 8 cubic pieces before it is mapped. The part of a graphic at or behind the eye is left out.
- An object takes up to 64 graphics. One graphic draws up to 4,096 fills and strokes, and the graphics of one 3D
  effect draw up to 400,000 path elements together (on Revolve's side and Extrude & Bevel's, 400,000 points after the curves are cut, and up to
  2,000,000 once they are cut at the faces, with up to 64,000,000 points cut together, and the copies of their
  paints in the parts take up to 256 MiB per object, apart from those of the surfaces). Commands refuse a graphic
  past either limit. A loaded file
  draws the object without a graphic past the first limit, and without all of its graphics past the second.
- A hollow Extrude & Bevel has no caps, so the graphics on its caps are not drawn until Cap is turned on again,
  and `threeD.mapGraphic` refuses to map a new one onto a cap; its side takes graphics. With a depth of 0 the object
  is a plane, and `threeD.mapGraphic` maps graphics onto its front only. Changing between Extrude, Plane, Revolve
  and Inflate keeps the graphics, except that Plane and Revolve drop the graphics on the back cap, and Plane, Revolve
  and Inflate drop the graphics on Extrude & Bevel's side; on Revolve the front's graphics go on its side, and on
  Inflate the front's and back's go on the puffed faces.
- `effect.apply` and `effect.setParams` refuse a graphic that names a symbol the document does not have, unless
  the effect already holds a graphic of that symbol. A graphic whose symbol is deleted is no longer drawn. Renaming a symbol renames it in the graphics that use it,
  in the layers, in the art of symbols, patterns, graph designs, brushes and opacity masks, and in graphic
  styles, and Select All Unused in the Symbols panel counts a symbol a graphic uses as used, a graphic in a graphic style included.

## Map Art

The Extrude & Bevel (Classic) and Revolve (Classic) dialogs have a **Map Art…** button on their Object tab. It
shows Map Art in place of the tabs until **Done**, and the dialog's Preview, OK and Cancel apply to the maps with the
other settings ([Map artwork to a 3D object](https://web.archive.org/web/20210601162814/https://helpx.adobe.com/pdf/illustrator_reference.pdf#page=189),
[the same section in the 2010 help](https://help.adobe.com/archive/en_US/illustrator/cs5/illustrator_cs5_help.pdf#page=274)).
Rotate (Classic) and the other 3D effects take no map. The command is `threeD.mapArt` ([mcp.md](mcp.md#map-art)).

- **Surface**: the first, previous, next and last buttons and a number field pick the surface, with the count beside
  them. A light gray mark shows that the surface faces the viewer and a dark gray mark that it is hidden. With Preview
  on, the surface is outlined in red on the artboard.
- **Symbol**: None or one of the document's symbols. Each surface keeps one symbol.
- The surface is shown laid out flat with the box of the mapped art on it. A drag inside the box moves the art, a
  drag of a side or corner handle scales it about its center, and a drag just outside the box turns it. X, Y,
  Horizontal Scale, Vertical Scale and Rotation give the same values as numbers.
- **Scale to Fit** centers the art and scales it to fill the surface. **Clear** removes the surface's map, as None
  does, and **Clear All** removes every map.
- **Shade Artwork** gives the mapped art the tone and highlight of the face it lies on. **Invisible Geometry**
  draws the mapped art and leaves out the solid's faces.

Extrude & Bevel (Classic) numbers its surfaces through the object's solids in the order they are built (the top fill's
solid, then each stroke's): each solid's front cap and back cap, then each side wall from corner to corner of the
outline, then the bevel above each wall. An extruded square has six surfaces. A hollow solid has no caps to number,
and a solid whose bevel is as deep as the solid has no walls.
A side wall is laid out at its own length, which a bevel added around the shape makes longer than the outline. A bevel
is laid out at the length of the wall below it, so mapped art on a bevel is stretched or squeezed along its edge
nearer the cap. Revolve (Classic) has one surface, its swept side, unrolled around the axis as Revolve's graphics are; on a group it is
the side of the member with the longest profile, and Invisible Geometry leaves out every member's faces. The numbering
is VectorCraft's own.

- A map's X and Y place the center of the art from the center of the surface laid out flat, so a map follows the
  center when the surface changes size. A map past the object's surface count draws nothing and stays in the effect.
- Mapped art is drawn on the parts of its surface that face the viewer, clipped to the surface, and over a wall, a
  bevel or Revolve's side it bends with the facets. The Wireframe surface draws no mapped art.
- The canvas, exports, printing and **Object › Expand Appearance** draw the mapped art the same way. Renaming a symbol
  renames it in the maps, copying the object to another document brings the symbols its maps use, and Select All
  Unused counts a symbol a map uses as used.
- A map's art follows the rules and limits of a graphic's art ([Graphics](#graphics)). An object takes up to 64 maps,
  and its maps share the 400,000 path elements of one 3D effect's graphics; past that limit the object draws without
  its mapped art. The art of the maps on walls, bevels and Revolve's side, at each map's scale and with its curves cut
  into straight pieces, may also take at most 400,000 points together, and commands refuse art past it. The limits and the shading of the mapped art are
  VectorCraft's own.

## Parameters (`effect.apply`, `effect.setParams`)

Approving an existing effect without changing its values preserves its stored parameters, document revision and
Undo history, with Preview on or off. Supplying only the same validated parameters through `effect.setParams`
also changes nothing and returns an empty `ids` array. Omitted defaults and equivalent numeric JSON count as the same
values. Changed options save in one step, and invalid parameters are still refused.

`threeD.extrude`: `depth`, `twist`, `taper` (%), `cap`, `bevel` (`none`, `classic`, `rounded`, `cove`, `step`,
`ogee`, or `custom` with `bevelName` and `bevelProfile`; `{custom: name}` takes a bevel of the bevels file), `bevelWidth`, `bevelHeight`, `bevelRepeat`, `bevelSpace` (%), `bevelInside`, `bevelBothSides`.
`threeD.inflate`: `depth`, `volume` (%), `bothSides`. All three take the view and lighting parameters Revolve
uses: `rotationX`, `rotationY`, `rotationZ`, `perspectiveAngle` (degrees), `shade`, `lightColor` (`#rrggbb`, default `#ffffff`), `lightIntensity` (%), `lightRotation`,
`lightHeight` (degrees), `lightSoftness` (%), `ambient` (%, to 200), `lightBehind`. Lengths are points and scale with the object (Scale Strokes & Effects).
Commands validate the shape and the parameters before the document changes. All four also take `lights`: null (the default, use the earlier single-light fields), or an array of one to ten objects. Each object takes `color` (opaque `#rrggbb`, white), `intensity` (0..100%, 80), `rotation` (-180..180 degrees, 125.26), `height` (0..90 degrees, 30), `softness` (0..100%, 0), and `behind` (boolean, false). Omitted settings use those defaults. An explicit list replaces the earlier single light; `ambient` and `shade` remain shared. Set `lights` to null to return to the single-light fields. Invalid lists, counts, keys and values are rejected before editing; malformed loaded lists fall back to the earlier light safely. All four take `shadow` (boolean, false), `shadowPosition` (`behind` or `below`), `shadowDistance` (0..100%, 0) and `shadowBounds` (10..400%, 100) ([Shadows](#shadows)); values out of range are rejected before editing.
`threeD.extrude`, `threeD.rotate`, `threeD.revolve` and `threeD.inflate` take `graphics` ([Graphics](#graphics)): an array of up to 64 objects, each
with `symbol` (the name of a document symbol), `face` (`front`, `back` or `side`, default `front`; `threeD.rotate` and
`threeD.revolve` take `front` only, and `threeD.inflate` `front` or `back`), `x` and `y` (points, default 0), `scale` (1..10000%, default 100) and `rotation` (-360..360 degrees,
default 0). Other keys, values out of range and graphics on the other 3D effects are rejected before editing.
All four take `render` ([3D and Materials panel](#3d-and-materials-panel)): an object whose `mode` is `shaded` (the
default) or `wireframe`. A `render` that is not an object, a key other than `mode`, another mode, and `render` on
the 3D (Classic) effects are rejected before editing.

## What differs from the public documentation

Behaviour reference: the public [3D and Materials panel options](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/3d-materials-panel-options.html)
and [Create 3D objects](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/create-3d-objects.html)
pages, and the archived Extrude & Bevel option list (depth 0–2,000, cap on/off, bevel height, bevel extent in or
out, perspective). No Adobe software, assets, presets or bevel shapes were used.

- **Materials** (base and Adobe Substance), graphics on Inflate, shadows of lights after the first, and
  **ray-traced rendering** are not built: shading uses directional lights, in the paint of the fill or stroke the
  solid is built for ([Fills and strokes](#fills-and-strokes)), and only the first light casts a shadow
  ([Shadows](#shadows)). Graphics map onto planar faces, Extrude & Bevel's side as one surface unrolled flat, and
  Revolve's side only ([Graphics](#graphics)).
- The bevel shapes and their names are our own. Custom bevels come from VectorCraft's own bevels file
  ([Custom bevels](#custom-bevels)) and are cut into straight lines.
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
  Extrude & Bevel, the flat face of Rotate, and every edge of the walls and bevels along the art's curves (at a
  cap, between two bevel steps and between two wall slices) are cubic curves, with smooth anchors where the
  art's are smooth, while visibility and depth are worked out on the sampled points. Without perspective and
  bevel each curve maps exactly. With perspective, a curve is split into cubics until each lies within 0.001 pt
  of the projected outline. A face inset or outset by a bevel follows its sampled outline within the sampling
  tolerance above, one curve for each sampled edge through its two sampled points, so two faces that meet draw
  the edge they share as the same curves. A twist cuts a wall's edge into pieces at points of the art's curve,
  so each piece follows the same curve as the cap it meets. Inflate keeps the curves of its flat back, of its
  side walls and of its front and puffed back where they meet the outline, within the sampling tolerance there;
  the inside of a puffed surface stays flat facets. A piece of curve that does not fit after 8 halvings, the
  curves past 1,048,576 for one solid, and the cells' edges left once Inflate has looked at 4,194,304 edges of
  the outline to find them, stay straight. The tolerances and limits are VectorCraft's own values.
- Inflate also limits the work of filling its grid, which grows with the outline's points times the grid cells its
  outline crosses. An outline of tens of thousands of points that crosses nearly every cell (a comb of 4,000 teeth
  across the shape) is refused because the 3D surface is too complex; a closed path of 65,536 points round a
  circle builds.
- **3D (Classic)**: Extrude & Bevel (Classic), Revolve (Classic) and Rotate (Classic) are built, with their surface
  types and blend steps. Revolve (Classic) sweeps the surface Revolve sweeps, with Cap closing a partial sweep's cut
  sides ([revolve.md](revolve.md)), at 64 segments for a full turn, and draws
  every face back to front, so it has no Draw Hidden Faces setting. Rotate (Classic) turns the plane Rotate turns, seen
  from either side, with the No Shading and Diffuse surfaces only. Preserve Spot Colors, on the Lighting tab of all
  three, keeps a color linked to a spot or global swatch linked on every face: the highlight lowers its tint, and a
  face that darkens gets black over it, multiplied and overprinting, so a spot color stays on its own plate. The
  checkbox is unavailable while Shading Color is Custom
  ([Preserve Spot Color](https://help.adobe.com/archive/en_US/illustrator/cs5/illustrator_cs5_help.pdf#page=260)); how
  the faces are shaded with it is VectorCraft's own. The Black Overprint shading color keeps each face's paint and
  draws the shading over it as overprinting black, without the highlight. The shading is on the black plate in
  Separations Preview and PDF output. Their Rotation tab has the track cube
  ([Track cube](#track-cube)). Extrude & Bevel (Classic) and Revolve (Classic) take Map Art
  ([Map Art](#map-art)). Turntable and 3D export are separate, unbuilt items.

Release measurements and a repeatable CLI scene are in [3D lighting performance](3d-lighting-performance.md).

## Revision history

| Date | Change |
|---|---|
| 2026-11-27 | Unchanged 3D effect options preserve the exact effect, document revision and Undo; the same rule applies to validated `effect.setParams` calls. |
| 2026-11-27 | Classic track cube screen-plane keyboard turns preserve untouched fractional X/Y angles through preview, OK, Cancel and Undo. |
| 2026-11-13 | Extrude & Bevel cuts its walls into slices only for a twist. A taper alone keeps each wall flat, so it is one face, without the light ticks at the joints between slices or the light line down a wall (#7019). Inflate still slices its sides for a taper. |
| 2026-11-12 | The seams of Extrude & Bevel, Inflate and Rotate under a white light, and of the faces of Extrude & Bevel (Classic), Revolve (Classic) and Rotate (Classic), went from miter to round joins (the seams of Revolve and of colored lights, and 3D (Classic) Wireframe edges, were already round): no thorns on edge-on facets, and facets with no neighbor now have rounded convex corners ([Seams](#seams)). |
| 2026-11-08 | Contours that cross, touch or overlap are merged by vectorcraft-pathops before Extrude & Bevel, Inflate and Rotate build a solid, sharing its limits; others are built as drawn. |
| 2026-11-06 | Extrude & Bevel, Inflate and Rotate apply to groups and layers: every path, compound path and type in them turns about one shared axis (#4506). |
| 2026-10-21 | Four original lighting presets, shared by all four 3D effects and commands, with effective-setting recognition and existing preview/history behavior. |
| 2026-10-17 | Smooth colored Inflate lighting and surface batching; legacy white-light appearance and the 0.05 seam minimum preserved; shared light Color and multiple lights remain vector output. |
