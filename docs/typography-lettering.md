# Typography and Custom Lettering in VectorCraft

## What you can do

- **Typesetting:** use an available font to create editable text, then adjust its size, style and spacing.
- **Custom lettering:** convert text to vector outlines and reshape the letters for a title or logo.
- **Reusable font files:** VectorCraft does not currently create or export installable TTF or OTF fonts.

This guide covers the first two workflows. The steps apply to the v0.13.1 release; the desktop app
can use installed system fonts, while the web app uses the fonts supplied by its deployment.

## Getting started with the Type Tool

1. Create a document with **File → New…**, or open an existing one.
2. Select **Type Tool** in the toolbar, or press **T** when you are not typing in a text field.
3. Click an empty part of the artboard and type a short heading, such as `MOON`. A click creates
   point text, which grows as you type. To create text that wraps inside a frame, drag a rectangle
   instead. If placeholder text appears selected, typing replaces it.
4. Press **Escape** to finish typing. Select **Selection Tool** (**V**) and click the text object.
   This lets you format the whole heading rather than just characters selected while typing.
5. Open **Window → Type → Character**. Use the font family menu at the top of the panel to search
   for a font and click its name to apply it. The menu below it chooses an available style, such as
   Regular, Bold or Italic; the choices depend on the font. **Type → Font** also lists available fonts.
6. Adjust **Font Size** (the field beside **T**) and **Tracking** (beside **VA↔**) in the Character
   panel. Hover a field's label to see its name. For example, enter `72 pt` for a large heading and
   try tracking `50` to spread its letters slightly. Positive tracking adds space; negative tracking
   tightens it. **Leading** controls the spacing between lines; **Kerning (0 = Auto)** adjusts pairs.
7. To change only part of the text, select **Type Tool**, click into the text and drag across the
   characters before using the Character panel. Press **Escape** when finished. Keep the text
   editable until its wording, font and spacing are settled.
8. Choose **File → Save As…**, select the **VectorCraft** format, and save the document as
   `lettering-editable.vectorcraft`.

The desktop app scans system fonts and loads faces as needed. If you install a font while it is
open, use **Refresh Font List** in the Character panel's menu. On the web, installing a font on your
computer does not add it to VectorCraft's font menu; the available fonts depend on the hosted build.

## Turning text into editable outlines

Create Outlines replaces text with vector paths. You can reshape those paths, but you can no longer
edit the words or choose a different font for them with the Type Tool.

1. Keep the editable file saved above. Choose **File → Save As…** again, select **VectorCraft**,
   and save as `lettering-outlines.vectorcraft`. Continue in this second file so later saves do not
   overwrite your editable original.
2. With **Selection Tool** (**V**), select the text object you want to convert.
3. Choose **Type → Create Outlines**. The default shortcut is **Ctrl+Shift+O** on Windows and Linux,
   or **Cmd+Shift+O** on macOS.
4. Select **Direct Selection Tool** (**A**) and click a letter's edge to inspect its paths and anchors.
   The text has become a group of paths, one compound path per glyph. A compound path can include
   both a letter's outer edge and its inner holes, such as the middle of an `O`.

Conversion uses the font currently drawn. Resolve missing or substituted fonts before outlining,
or the paths will capture the substitute's shapes.

## Customizing individual letters

Try making the final `N` in `MOON` taller to give the heading a distinctive ending.

1. Select **Direct Selection Tool** (**A**). Click empty canvas to clear the current selection,
   then click an anchor on the outlined `N`. Anchors are the points that define a path's shape;
   Direct Selection can reach paths inside the group without ungrouping the letters.
2. Drag the anchor a short distance to change that part of the letter. To move several anchors
   together, Shift-click them before dragging, or drag a selection rectangle around them from
   empty canvas. For the taller `N`, select its top anchors and move them up slightly.
3. On a curved letter such as `O`, select an anchor on a curve. Drag the end of a visible direction
   handle to change the curve's bend. These Bézier handles control how the curve leaves or enters
   the anchor. Straight corners may have no handles to drag.
4. Switch to **Selection Tool** (**V**) to inspect the whole heading. Use **Edit → Undo** if a change
   distorts the letter or closes an inner hole, then try a smaller adjustment.

These changes affect the artwork in this document. They do not change the installed font or other
text using it. A glyph is not always one typed character: for example, a font may join letters into
a ligature, which becomes a single glyph outline.

## Saving and exporting your artwork

- **Keep the project:** use **File → Save** to save `lettering-outlines.vectorcraft`. Retain
  `lettering-editable.vectorcraft` separately for future wording or font changes. A native file keeps
  editable text only where you have left text objects unconverted.
- **Export vector artwork:** choose **File → Export → Export As…**, select **SVG** or **PDF**, and
  continue through the export options. The outlined letters are vector shapes in the output, not
  keyboard-editable text. Check **Use Artboards** if you want the artboard bounds rather than the
  bounds of the visible artwork.
- **Create an installable font:** SVG and PDF exports are artwork, not TTF/OTF files. Renaming their
  extension does not turn them into fonts.

## Can VectorCraft create a font file?

No. VectorCraft does not currently provide a complete font-authoring workflow or TTF/OTF export.
It does not turn your letter paths into a reusable font with character encoding, glyph metrics and
font-wide kerning definitions. Character panel spacing changes format the text in your drawing;
they do not edit a font's kerning tables.

To make an installable typeface, use a dedicated font editor to assign glyphs to characters, set
metrics and spacing, and generate the font file. VectorCraft's outline tools are useful for custom
lettering artwork, but exporting that artwork is only one part of making a font.

## Troubleshooting

- **I cannot find my font.** On desktop, check that it is installed, clear the font menu's search
  and filters, then choose **Refresh Font List** from the Character panel menu. **Type → Find Font…**
  lists fonts used by the document and marks missing ones; it can replace them with available fonts.
  A missing family, style or character may use a fallback, so check the result before outlining.
  On the web, the site must supply the font; system-font scanning and the desktop refresh option
  are unavailable there.
- **I cannot change the words after Create Outlines.** The result is paths, not text. Use
  **Edit → Undo** if the conversion is still in the undo history, or reopen your editable original.
- **I cannot install my modified letters as a font.** They are drawing objects. SVG/PDF export
  does not add the structure required by an installable font; see [Can VectorCraft create a font
  file?](#can-vectorcraft-create-a-font-file).
- **How do I keep text editable?** Save the text in a native `.vectorcraft` file before outlining,
  then use a different file name for the outlined version. Saving paths in the native format does
  not turn them back into text.
