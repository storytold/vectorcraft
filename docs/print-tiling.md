# Print a large poster across multiple pages

## Overview

Use tiled printing to spread a large poster, sign or sewing pattern across several sheets of A4,
Letter or other paper. VectorCraft divides the drawing into numbered tiles at the scale you choose.
Overlap repeats part of the drawing on adjacent sheets to help you align them after printing.

## Print a poster on multiple sheets

1. Open your drawing in VectorCraft. For a full-size pattern, check that the drawing has the intended
   dimensions before printing.
2. Choose **File → Print…** to open VectorCraft's **Print** dialog. In **Printer**, select your printer
   or **PDF File** to save the tiled job as a PDF.
3. In **General**, under **Artboards**, choose **All** or **Range:** to select the artboards to print.
   For example, use **Range:** `1` for the first artboard. Choose **Ignore Artboards (all the art as
   one page)** only if you want to tile the bounds of all the artwork instead of individual artboards.
4. Under **Media**, set **Size** to the paper loaded in the printer, such as **A4**, **Letter** or a Japanese paper (**B5 (JIS)**, **Japanese Postcard**, **Japanese Envelope #3**).
   Choose **Custom** to enter **Width** and **Height**. **Auto-Rotate (turn the paper to each artboard)**
   is on by default; turn it off to choose a fixed **Orientation**, such as **Portrait** or **Landscape**.
5. In **Advanced**, set **Unprintable margin** to allow for the edges your printer cannot print.
   VectorCraft uses one value for all four edges; use a value that covers the printer's largest
   required margin. The default is zero, and VectorCraft does not read this value from the printer.
6. Return to **General**. Under **Options**, set **Scaling** to **Tile Imageable Areas** or
   **Tile Full Pages** (see [Choose a tiling mode](#choose-a-tiling-mode)). Set **Scale**, both **W**
   and **H**, to `100%` to keep the drawing's size. Use the same percentage in both fields to keep
   its proportions; larger percentages enlarge the drawing and can require more sheets.
7. Set **Overlap** to repeat a strip along adjoining tiles. For example, `10 mm` gives room for
   alignment. Length fields accept units such as `mm` and `in`. Overlap must be less than half
   the smaller tile dimension; increasing it can increase the sheet count.
8. Leave **Tile Range** empty initially. Use the **Previous page** and **Next page** arrows beneath
   the preview to inspect each sheet. The label identifies its artboard and tile; the preview also
   shows the paper size and scale. Check **Summary** for warnings or invalid settings.
9. Keep **Tile Range** empty to print every tile, or enter a range such as `1-3, 5` to print a subset.
   Check the preview again after changing the range.
10. Click **Print**. With a printer selected, VectorCraft sends the laid-out job to the system's print
    service. With **PDF File** selected, it asks where to save the PDF, with one page per printed
    tile for each copy in composite output. **Done** keeps the print settings with the document
    without printing; save the document to retain them for next time.

The tiling controls belong to VectorCraft's **Print** dialog on Windows, macOS and Linux.
**Setup…**, beside **Printer**, opens the system's printer settings, whose interface varies by
platform. Set tiling in VectorCraft's **Scaling** list. Keep the printer's paper settings consistent
with **Media**. In the web app, printing goes through the browser's print dialog; **PDF File**
downloads the PDF.

For a paper proof, select **PDF File** and click **Print**, then open the saved PDF to inspect its
pages before sending it to a printer. When printing that PDF, keep its page size and scale rather
than applying another fit-to-page reduction. For a sewing pattern, measure a known length on a
printed sheet before printing the full job.

## Choose a tiling mode

| Scaling | How it divides the drawing | When to use it |
|---|---|---|
| **Tile Full Pages** | Each tile uses the full paper width and height. Adjacent tiles advance by that size minus **Overlap**. | Use when you need a grid based on whole sheets, such as with a printer that supports the required borderless paper setting. On a printer with unprintable edges, artwork near tile edges can be lost unless sufficient overlap covers those gaps. |
| **Tile Imageable Areas** | Each tile uses the paper area inside **Unprintable margin**. Adjacent tiles advance by that smaller size minus **Overlap**. | Use for printers with unprintable edges. Set the margin in **Advanced** so the tiles account for those edges; this can require more sheets than full-page tiling. |

With **Unprintable margin** set to zero, the two modes use the same tile dimensions. Neither mode
makes a printer capable of borderless printing. **Fit to Page** scales an artboard onto a single
sheet; choose a tile mode to keep a large drawing spread across sheets.

## Check the layout on the canvas

1. Set up tiling in **File → Print…**, then click **Done** to keep the settings.
2. Choose **View → Show Print Tiling**. The canvas shows each sheet's paper edge and a dashed,
   numbered imageable area. Tiles excluded by **Tile Range** appear dimmer. The overlay uses the
   saved print settings; it does not print anything.
3. Select the **Print Tiling Tool**. In the default toolbar, press and hold the **Hand Tool** button
   in the **Navigate** group to open its tool list, then choose **Print Tiling Tool**. The tool also
   shows the overlay while it is active.
4. Drag on the canvas to position the grid. The pointer places the top-left corner of a page's
   imageable area and snaps near artboard edges. Tiles extend left and up as needed to cover the
   drawing, so that page may not be tile 1 after the move. Reopen **File → Print…** to check the
   resulting pages and tile numbers. Dragging the dialog's preview does not reposition tiled pages.

To restore the default grid position, double-click on the canvas with **Print Tiling Tool** active,
or double-click the tool's toolbar button. Alternatively, reopen **File → Print…** and click
**Reset** beneath **Placement** when the dialog says **The Print Tiling tool placed the pages.**
Click **Done** to keep that change. **View → Hide Print Tiling** hides the overlay after switching
to another tool.

## Print only selected tiles

**Tile Range** uses numbers starting at 1, across each row from left to right, then down.
For a grid with three columns and two rows, the first row is `1, 2, 3` and the second is `4, 5, 6`.
Enter `2, 5` to print the middle column, or `1-3, 5` to print the first row and tile 5.
Leave the field empty to print all tiles.

**Range:** under **Artboards** selects drawings; **Tile Range** selects sheets within each chosen
drawing. Tile numbering restarts for each artboard, and the same tile range applies to each one.
To reprint one damaged sheet of one artboard, select that artboard with **Range:** and its tile
with **Tile Range**. A tile number must exist on every selected artboard. Changing paper, scale,
margin, overlap or grid position can change the grid and its numbering; check again before reprinting.

## Assemble the printed sheets

Lay out the sheets in tile order. Align repeated artwork in the overlap, trim unprinted edges as
needed, and tape or glue the sheets together. Print a pair of adjacent tiles first to check the
scale, margins and overlap before committing to a large job.

## Troubleshooting

- **The preview shows only one page.** It displays one sheet at a time. Use the arrows and check
  the count beneath it. If the count is one, check **Tile Range**, the chosen artboards, paper size
  and scale: the drawing may fit on one tile.
- **The image shrinks to one sheet.** Change **Scaling** from **Fit to Page** to a tile mode and
  check both scale percentages. Also check for another fit-to-page setting when printing a saved PDF.
- **Artwork is missing along sheet edges.** Check the printer's unprintable margins. Use
  **Tile Imageable Areas** with a suitable **Advanced → Unprintable margin**, or allow enough
  overlap in full-page tiling to cover the missing edges. Check that the physical paper matches
  **Media**; a zero margin does not remove a printer's hardware limits.
- **No tiling boundaries appear on the canvas.** Keep the settings with **Done**, then enable
  **View → Show Print Tiling** or select **Print Tiling Tool**. If the settings are invalid, the
  overlay has no pages to draw; reopen **Print** and check **Summary**.
- **The job has the wrong number of pages.** Check **Artboards**, **Tile Range**, **Copies** and
  **Skip Blank Artboards**. Paper size, orientation, scale, margins and overlap affect the grid.
  Under **Output**, **Mode → Separations** produces a page per enabled ink; use **Composite** for
  an ordinary poster. The dialog preview shows one copy; the saved PDF includes all copies.

## Related documentation

- [Printing and PDF file output](mcp.md#printing) — command reference.
- [Print tiling](mcp.md#print-tiling) — overlay and positioning command reference.
- [Print settings](mcp.md#print) — paper, scaling and output settings reference.
