---
title: Open a file
description: Open SVG, PDF, Illustrator (.ai), Affinity and VectorCraft files with File > Open, or add one to the current document with File > Place.
order: 10
verified: 0.10.0
---

Choose **File** › **Open…**, pick the file, and open it. To add a file to the document you already have open, use **File** › **Place…** instead.

## Open a file

1. Choose **File** › **Open…**.
2. In the file dialog, pick your file. The default filter is **All readable files**.
3. Open it.

**File** › **Open Recent Files** lists the files you opened lately. **Clear Recent Files** empties the list.

## Which files open

| Kind          | Extensions                                                              |
|---------------|-------------------------------------------------------------------------|
| VectorCraft   | `.vectorcraft`, and `.drawcraft` from the app's earlier name, DrawCraft |
| Vector        | `.svg`, `.svgz`, `.pdf`, `.ai`, `.ait`, `.eps`, `.dxf`, `.emf`, `.wmf`  |
| Images        | `.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`, `.tif`, `.tiff`, `.bmp`       |
| Photoshop     | `.psd`, `.psb`, opened as their merged image                            |
| Affinity      | `.af`, `.afdesign`, `.afpub`                                            |
| Template      | `.vctemplate`                                                           |

VectorCraft also opens `.afphoto` files from Affinity 1 and 2, recognized by their content.

## Illustrator files

VectorCraft opens `.ai` files that contain a PDF page, but exchange with Illustrator files is not reliable yet. The README lists it under what is still missing, and the project has no collection of real Illustrator files to test against. Layers can come out flattened or multiplied, and live effects, brushes and symbols can arrive as their drawn look. VectorCraft saves `.ai` files in the PDF-compatible form only, without Illustrator's private editing data. Check the result before you rely on it. See [Can VectorCraft open Illustrator and Affinity files?](../faq/illustrator-and-affinity-files.md).

## Affinity files

VectorCraft opens Affinity files and keeps layers, groups, artboards and pages, curves and shapes, fills, gradients, strokes, clipping, masks, text and images. What it could not bring in, such as effects, adjustments and brushes, is listed in the warning after the file opens. If the native data of a file cannot be read, VectorCraft opens its embedded preview instead and says why. VectorCraft does not save Affinity files.

## If a file does not open

VectorCraft shows a dialog that says "Can't open" followed by the file name, with the reason. Check that the file exists and is not damaged. If the file is a valid one that should open, [report the problem](../troubleshooting/bug-reports.md) and attach the file if you can.

## Missing fonts

When a document uses fonts that are not installed, VectorCraft lists them in a **Missing Fonts** dialog when the document opens. It can search a folder you choose for the font files.
