# Revolve

This initial implementation follows the scope agreed in [issue #605](https://github.com/storytold/vectorcraft/issues/605#issuecomment-6070543567).

Draw an open profile with the Pen tool, select it, then choose **Effect > 3D and Materials > Revolve**.
The profile's horizontal distance from a vertical axis is its radius. **Axis side** chooses the left or right
edge of the profile's bounds; **Offset** moves the axis outward. For a cylinder made from a vertical
line, give it a nonzero offset. A vertical line at zero offset lies on the axis and has no surface.

**Revolve angle** sweeps 0 to 360 degrees. **Rotation X/Y/Z** rotates the viewing direction, with all three axes defaulting to 0 degrees. **Perspective**
is zero for an orthographic view. **Segments** controls radial resolution (8 to 128 for a full turn).
Lighting is a directional light with azimuth, elevation, intensity and ambient contribution.
Turn **Shading** off to keep the source paint without lighting.

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
its direction on the current hemisphere. A hollow sun marks a back light. Drag the **Direction**
ring or its label horizontally to adjust azimuth; drag the **Elevation** arc or its label vertically
to adjust elevation. The labels remain usable when a ring is viewed edge-on or the light is at a pole.
Hold Shift to snap to 15-degree increments. Light gestures update their matching fields without
rotating the object, and keep the same temporary preview, Escape, Cancel and undo behaviour.
Light intensity and ambient contribution remain available as draggable fields and sliders.

**Shading** off hides the light gizmo and disables lighting controls. Switch to **Rotation** to return
to the object rings. **Show axis** in **Advanced** displays the source's vertical axis on the object.
Turning **Preview** off restores the original artwork and hides artboard controls until Preview is enabled.
In small windows, the controls scroll while OK and Cancel stay visible. Reset dots restore individual
settings; **Reset all** restores parameters on both tabs.

Preview changes are temporary until OK. Cancel restores the document; OK creates one undo step.
The source anchors remain editable. Appearance lists the effect for reopening its options.
SVG/PDF export evaluates filled vector faces; native saves retain the profile and effect parameters.
**Object > Expand Appearance** replaces the live effect with ordinary editable vector faces.

In **Advanced**, **Keep visible surfaces only when expanding** is on by default. Expansion removes
completely covered faces and trims partially covered ones using the current rotation and perspective.
Visible interior surfaces through open ends remain. Uncheck it to retain every projected face.
The checkbox has its own reset dot and is included in **Reset all**. Its saved command parameter is
`expandVisibleOnly`; it does not change the live preview or SVG/PDF export baking.

Trimming applies to opaque solid paint. Transparent objects, gradients, patterns, non-normal blending,
later geometry effects and surfaces exceeding bounded visibility limits keep their complete geometry
to preserve their appearance. Visibility follows the same painter order as the live effect, including
its approximation for intersecting surfaces. Expansion changes are undoable with the standard command.

## Initial limits

- Paths and compound paths only. Type, images and groups must be converted to paths first.
- Open profiles produce uncapped surfaces. Closed profile boundaries are swept as provided.
- One Revolve per source object. Edit its options rather than applying another.
- Object-level Revolve uses the top visible fill, falling back to a stroke. Per-item Revolve uses that item's paint.
- Flat shading, one directional light, no textures, cast shadows, end caps or physical materials.
- Faces use back-to-front depth sorting. Intersecting profiles can have incorrect visibility.
- Curves use sixteen samples per segment; profiles are limited to 1,024 samples and surfaces to 32,768 faces.
- Axis, bounds and ordinary selection handles continue to refer to the editable 2D source profile.
- Extrude & Bevel, Inflate, Rotate and Materials remain unimplemented.

## Architecture

`vectorcraft-three-d` depends only on `vectorcraft-geom`. It creates a surface of revolution,
applies Euler view rotation, projects it, shades faces and sorts their depth. It has no UI or GPU dependency.
`vectorcraft-effects` turns those faces into document art. The renderer, export baking and
Expand Appearance use the same evaluator. The standard `effect.apply`, `effect.setParams`,
`effect.remove` and `effect.expandAppearance` commands expose it to the CLI, control channel and MCP.

Behaviour reference: [public 3D effects documentation](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/create-3d-objects.html).
This implementation uses original mathematics and source code, with no Adobe software, assets or presets.
