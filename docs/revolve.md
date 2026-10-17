# Revolve

This initial implementation follows the scope agreed in [issue #605](https://github.com/storytold/vectorcraft/issues/605#issuecomment-6070543567).

Draw an open profile with the Pen tool, select it, then choose **Effect > 3D and Materials > Revolve**.
The profile's horizontal distance from a vertical axis is its radius. **Axis side** chooses the left or right
edge of the profile's bounds; **Offset** moves the axis outward. For a cylinder made from a vertical
line, give it a nonzero offset. A vertical line at zero offset lies on the axis and has no surface.

**Revolve angle** sweeps 0 to 360 degrees. **Rotation X/Y/Z** rotates the viewing direction, with all three axes defaulting to 0 degrees. **Perspective**
runs 0° to 160°, as documented: 0° is an orthographic view, and wider angles bring the eye nearer (see
[three-d.md](three-d.md#perspective)). **Segments** controls radial resolution (8 to 128 for a full turn).
Lighting supports one to ten directional lights with the documented Light Settings (see [three-d.md](three-d.md#lighting)):
**Color**, **Intensity**, **Rotation**, **Height**, **Softness** and **Ambient Light**, and **Light behind the object**.
Color is white by default, with a picker and a reset dot. Colored light preserves neutral ambient illumination.
Turn **Shading** off to keep the source paint without lighting.

Shaded Revolve surfaces automatically use smooth lighting from the profile's tangents,
represented by linear gradients on vector triangles. There is no separate smoothing control.
Segments still controls the body silhouette; smoothing does not increase radial resolution.
Capped endpoint rings use curved vector boundaries. Original profile corners remain sharp,
and circular end caps keep uniform flat lighting. Turning Shading off disables lighting.
Smooth lighting follows the same light gizmo, rotation, perspective, preview, Cancel and undo behavior.
SVG and PDF export retain vector paths and gradients; Expand Appearance keeps
those gradients at their original positions when trimming visible surfaces. Non-solid
source paints retain their paint with a gradient shading overlay.

The dialog has **Rotation** and **Lighting** tabs. Rotation groups the profile and view controls. Drag a value or its label
to adjust it, use its slider, or double-click the value to type. Angles show degrees and lighting
values show percentages. **Segments** is in the **Advanced** section. Lighting controls are disabled
when **Shading** is off. A filled dot at the end of a row means the value differs from its default;
click the dot to reset that value. **Reset all** restores every Revolve parameter to its default.

The compact settings dialog opens beside the artboard. With **Preview** enabled, the selected
object has X (red), Y (green) and Z (blue) rotation rings directly on the artboard. Drag a ring to
adjust that axis; drag inside the rings to rotate freely. Hold Shift to snap to 15-degree increments.
The rings and the rotation fields stay in sync. Escape during a drag restores that gesture and
keeps the settings open. Escape again or Cancel restores the document before the dialog opened.
The gizmo appears while editing Revolve; normal source-path selection returns when the dialog closes.

Select the **Lighting** tab to use the light gizmo on the artboard. Drag the sun handle to move
the light on its side of the object: round the centre for its Rotation, towards the centre for a
greater Height. A hollow sun marks a light behind the object. Drag the **Rotation** ring or its label
horizontally to turn the light round the object; drag the **Height** line or its label vertically to
raise or lower it. The labels remain usable when the ring shrinks to a point (Height 90°).
Hold Shift to snap to 15-degree increments. Light gestures update their matching fields without
rotating the object, and keep the same temporary preview, Escape, Cancel and undo behaviour.
Intensity, Softness and Ambient Light are draggable fields and sliders. The Lights list selects which light the gizmo and settings edit; Add Light and Delete Light manage up to ten lights, keeping at least one. Ambient is shared and applied once. Preview, Cancel, Reset all and undo include the whole list.

**Shading** off hides the light gizmo and disables lighting controls. Switch to **Rotation** to return
to the object rings. **Show axis** in **Advanced** displays the source's vertical axis on the object.
Turning **Preview** off restores the original artwork and hides artboard controls until Preview is enabled.
In small windows, the controls scroll while OK and Cancel stay visible. Reset dots restore individual
settings; **Reset all** restores parameters on both tabs.

Preview changes are temporary until OK. Cancel restores the document; OK creates one undo step.
The source anchors remain editable. Appearance lists the effect for reopening its options.
SVG/PDF export evaluates filled vector faces; native saves retain the profile and effect parameters.
**Object > Expand Appearance** replaces the live effect with ordinary editable vector faces.

**Cap** in the shape controls closes the circular rings at the endpoints of an open profile.
It is on by default. Its saved parameter is `cap` (boolean, default true); missing parameters
also use caps. Set it to false to keep the ends open. The reset dot and Reset all restore Cap to on. Each cap is one closed curved vector path from the endpoint ring to the rotation axis,
with uniform flat shading. A full turn produces a projected circular disk (an ellipse in an
orthographic view); a partial turn produces a sector. Body faces share the same curved rim,
so no triangle patches interrupt the cap edge. Caps share the surface's rotation, perspective and paint, and appear in preview,
SVG/PDF export and Expand Appearance. The checkbox has a reset dot and is included in Reset all.

An endpoint on the axis needs no cap. Closed profiles and open profiles whose endpoints coincide
keep their existing geometry. Each open subpath of a compound profile is capped independently.
A partial revolve adds only the swept sector at each endpoint; the angular cut sides remain open.
This option does not make intersecting or compound profiles into a watertight solid.

In **Advanced**, **Keep visible surfaces only when expanding** is on by default. Expansion removes
completely covered faces and trims partially covered ones using the current rotation and perspective.
Visible interior surfaces through open ends remain. Uncheck it to retain every projected face.
The checkbox has its own reset dot and is included in **Reset all**. Its saved command parameter is
`expandVisibleOnly`; it does not change the live preview or SVG/PDF export baking.

Trimming applies to opaque solid paint. Transparent objects, gradients, patterns, non-normal blending,
later geometry effects and surfaces exceeding bounded visibility limits keep their complete geometry
to preserve their appearance. Visibility follows the same painter order as the live effect, including
its approximation for intersecting surfaces. Uncut faces keep their curved outlines. Visibility calculations use bounded polygon approximations
of curved rims; clipped portions become vector polygons. Expansion changes are undoable
with the standard command.

## Initial limits

- Paths and compound paths only. Type, images and groups must be converted to paths first.
- Open profiles have circular end caps by default; turning Cap off keeps them open. Partial-sweep angular cut sides remain open. Closed profile boundaries are swept as provided.
- One Revolve per source object. Edit its options rather than applying another.
- Object-level Revolve uses the top visible fill, falling back to a stroke. Per-item Revolve uses that item's paint.
- Smooth diffuse shading, one to ten directional lights, no textures, cast shadows or physical materials. Smooth shading interpolates lighting in projected triangles; it does not smooth the silhouette or add specular highlights.
- Faces use back-to-front depth sorting. Caps facing the camera cover their rim seams; caps facing away sit behind the body. Intersecting profiles can have incorrect visibility.
- Curves use sixteen samples per segment; profiles are limited to 1,024 samples and surfaces to 32,768 faces. Smooth shaded side quads become two triangles and share that same limit. A continuous cap retains its original radial mesh cost within this limit. Projected ring curves have a 0.001-point fitting tolerance and a shared 32,768-curve limit.
- Axis, bounds and ordinary selection handles continue to refer to the editable 2D source profile.
- Extrude & Bevel, Inflate and Rotate are described in [three-d.md](three-d.md). Materials remain unimplemented.

## Architecture

Revolve geometry in `vectorcraft-three-d` uses `vectorcraft-geom`; the crate also uses `vectorcraft-pathops`
for the other 3D types. It creates a surface of revolution,
applies Euler view rotation, projects it, shades faces and sorts their depth. It has no UI or GPU dependency.
`vectorcraft-effects` turns those faces into document art. The renderer, export baking and
Expand Appearance use the same evaluator. The standard `effect.apply`, `effect.setParams`,
`effect.remove` and `effect.expandAppearance` commands expose it to the CLI, control channel and MCP.

Generated faces include narrow matching strokes to cover antialiasing seams. Their round
joins bound the overlap to half the stroke width, preventing acute triangle corners from
creating spikes along the silhouette in the canvas, SVG/PDF and expanded artwork.

Behaviour references: [public 3D and Materials options](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/3d-materials-panel-options.html) and [public 3D effects documentation](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/create-3d-objects.html).
This implementation uses original mathematics and source code, with no Adobe software, assets or presets.
