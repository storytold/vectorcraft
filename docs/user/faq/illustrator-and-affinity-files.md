---
title: Can VectorCraft open Illustrator and Affinity files?
description: VectorCraft opens Illustrator .ai files, but exchange with them is not reliable yet; Affinity files open natively.
order: 20
verified: 0.10.0
---

VectorCraft opens `.ai` files, but exchange with Illustrator files is not reliable yet. Affinity files open natively. VectorCraft writes `.ai` only as a PDF-compatible file and does not write Affinity files.

## Illustrator (.ai)

The README lists reliable exchange of Illustrator's own `.ai` files under what is still missing. The project has no collection of real Illustrator files to test against, so the list below is the project's own assessment, not a tested guarantee.

- VectorCraft aims to read the PDF page inside an `.ai` file, including vectors, text, images, color, transparency and layers, and the structure of Illustrator's editing data.
- A PDF saved from Illustrator with its editing data opens from that data, as an `.ai` file does. An `.ai` file with a bleed opens on its artboard, with the bleed around it.
- Saving as **PDF-compatible .ai** writes a file that Illustrator opens as plain artwork, without Illustrator's private editing data.
- Known problems, all open: the layer structure can come out flattened or multiplied ([#951](https://github.com/storytold/vectorcraft/issues/951), [#868](https://github.com/storytold/vectorcraft/issues/868)), guides and non-printing lines can be lost ([#779](https://github.com/storytold/vectorcraft/issues/779)), and live effects, brushes and symbols can come in as their drawn look ([#637](https://github.com/storytold/vectorcraft/issues/637)).

## Affinity

VectorCraft opens `.af` (Affinity 3), `.afdesign` and `.afpub` files, and `.afphoto` files from Affinity 1 and 2 by their content. What could not come in is listed in the warning when the file opens.

## More

See [Open a file](../guides/open-a-file.md). If a file of yours does not open right, [report the problem](../troubleshooting/bug-reports.md).
