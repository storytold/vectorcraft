# Hardware parity

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-10 · **Change:** major (first checklist) · **Target:** Adobe Illustrator 2026 (30.x)

What hardware Illustrator uses, per platform, against VectorCraft. Illustrator's side is from its public
documentation (GPU Performance, touch workspace, pen pressure pages); ours from the code
(`crates/ui-egui/src/canvas.rs`, `touch.rs`, `graphics.rs`, `crates/render`). Part of
[target-app-parity.md](target-app-parity.md).

## Summary

**~35% (estimated), 25–45 h.** VectorCraft rasterizes the canvas on the CPU (multithreaded `vello_cpu`) and uses
the GPU only to composite, so any GPU that can show a window works, at the cost of GPU-accelerated preview
on huge documents. Pen input is pressure-only and Windows-first.

| Feature | Illustrator | VectorCraft macOS | Windows | Linux | Web | Status / hours |
|---|---|---|---|---|---|---|
| GPU-accelerated canvas (GPU Performance, GPU Preview, GPU anti-aliasing) | Metal, DirectX | CPU raster, GPU composite (wgpu/Metal) | CPU raster, GPU composite (DX12, OpenGL fallback) | CPU raster, GPU composite (Vulkan/GL) | WebGPU, WebGL2 fallback | partial; a GPU renderer spike is pending (M5): 15–25 h |
| Multithreaded rendering off the UI thread | partial | yes | yes | yes | single-threaded | beyond Illustrator |
| Hybrid-graphics GPU choice | automatic | power-saving GPU by default, Preferences › Graphics Processor | same; DX12 then OpenGL; no Vulkan unless asked (#545, #806) | the desktop's GPU | browser | done |
| Restart on another GPU when the first fails to present | — | yes (#502, #964) | yes | yes | restart on lost device (#369) | done |
| Pen pressure | yes (Wacom, Windows Ink, macOS tablet events) | **no** (winit reports none, #852) | yes (pointer force via `WM_POINTER`) | X11 only via XWayland; none on Wayland (#491, #764) | browser pointer events | partial: 4–8 h for macOS and Wayland (winit 0.31) |
| Pen tilt, bearing, barrel rotation (6D Art Pen) | yes, drives brushes | no | no | no | no | missing: 4–8 h after the input layer (#372) |
| Pen eraser end | yes | no | no | no | no | missing |
| Touch gestures (pinch zoom, two-finger pan, rotate view, Touch workspace) | yes (Windows touch, Surface) | trackpad pinch and scroll | pinch, pan; two-finger tap undo, three-finger redo (#585) | same where winit reports touch | same | partial: touch workspace and rotate gesture missing, 3–5 h |
| Trackpad (smooth scroll, pinch) | yes | yes | precision touchpads | yes | yes | done |
| HiDPI / Retina, per-monitor scale | yes | yes | yes | yes | yes | done |
| Multiple monitors: panels and document windows on another screen | yes | no (one OS window) | no | no | — | missing: 6–10 h (with multiple windows, gap G7) |
| Display colour profile (proof and display through the monitor's ICC) | yes | no | no | no | — | missing: 3–5 h |
| 3D GPU rendering (Materials, ray tracing) | yes | — | — | — | — | blocked on 3D (gap G3) |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-10 | major | First checklist from the canvas, touch and graphics code and the open hardware issues |
