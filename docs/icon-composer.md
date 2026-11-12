# Export for Icon Composer

> **Last updated:** 2026-11-12 · **Change:** major (first version)

File › Export › Export for Icon Composer… writes a document's layers as separate files for
[Icon Composer](https://developer.apple.com/icon-composer/), Apple's tool that builds an app icon from layers given as
SVG or PNG files. It is beyond Illustrator: Illustrator has no such command of its own. The behaviour below is
VectorCraft's own design, worked out from what Icon Composer takes in.

## What it writes

- **One file per layer.** Each visible top-level layer is its own file, with its sublayers included. Hidden layers,
  template layers and layers with no visible art on the artboard are left out. Objects of a layer that lie wholly off
  the artboard are left out of its file.
- **The artboard's size.** Every file has the bounds of its artboard, so the layers line up when Icon Composer stacks
  them. PNG files are drawn at 1 pixel per point on a transparent background (a 1024 × 1024 pt artboard gives
  1024 × 1024 px), and the artboard's background color is left out.
- **Names in stacking order.** Files are named `<n>.<layer name>.svg` (or `.png`) in lower case. `n` counts from 1 for
  the back layer to the front one, so the files sort in stacking order. Characters that can't be in a file name on
  every platform become `-`.
- **A folder per export.** The files go in a new folder named after the document, inside the folder you choose. When
  a folder of that name is already there, the new one has the date and time (UTC) after the name, so an export
  never overwrites an earlier one.
- **A sub-folder per artboard.** A document with several artboards gets a sub-folder per exported artboard, named
  after it, so each icon's layers stay together. You can export all artboards or a range (`1-3, 5`).

On the web, the folder downloads as one ZIP file.

## Automation

`document.exportForIconComposer` takes `{folder?, format?: svg|png, artboard? | artboards? | range?, zip?}`. Its params
description is in the command list that MCP and the control channel give, and in
[control-protocol.md](control-protocol.md). Without `folder`, the files
come back in the reply, or as one ZIP with `zip: true`.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-11-12 | major | First version. |
