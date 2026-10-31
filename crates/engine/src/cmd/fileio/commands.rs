use super::access::file_list;
use super::formats::formats;
use super::{CommandSpec, action_batch, always, batch, cmd, export, has_doc, has_selection, load, pdfimport, save, screens};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "document.open",
            "Open Document",
            [],
            None,
            "{path | name | dataBase64 (a file's path, or its name with its bytes), pages?: \"2-3, 5\" (PDF/.ai: 1-based, default all; one artboard each) | page?: n, cropTo?: bounding|art|crop (default)|trim|bleed|media (the box each artboard gets; bounding: the art's bounds; trim: the bleed box round it becomes the document's bleed; a PDF-compatible .ai or a PDF saved with its editing data opens on its trim boxes for crop), password? (encrypted PDFs; see document.pdfInfo), textAs?: text (default: point type per line, lines wrapped in a frame as area type where they lay out the same, in the file's fonts by name; missing ones are listed in warnings) | outlines (glyph paths), layers?: true (default: optional content groups become layers with their visibility, print state and lock — art that is off comes in as a hidden layer; other art goes to a layer per page) | false (one layer per page, without the art that is off; Save then asks for a name), editingData?: true (default: an EPS, .ai or PDF that carries its editor's own copy of the art opens from it, with its layers, hidden objects, guides, artboards and the art outside them; a .ai or PDF whose type shows opens from its PDF part with textAs: outlines) | \"always\" (as true, and a .ai's editing copy is used even where it draws differently from its PDF part, which can be older; the warnings say so) | false (only what its page or PDF part prints, for print pipelines; a .ai saved without PDF compatibility then can't be opened), colorMode?: rgb|cmyk (the mode the document opens in, its colours converted as file.documentColorMode does, grays? as there; default: the file's — a PDF keeps CMYK, Gray and spot inks (spot swatches at a tint) and opens in CMYK when painted mostly in CMYK), grays?: as file.documentColorMode, dxf?: {layout?: \"Model\" (default) | a paper layout's name (document.dxfInfo lists them), unit?, scale? (the ratio: 1 unit of the art = scale drawing units; default: the drawing at 1:1 in its own unit), fit?: false (scale to fit an artboard of fitTo?: [612, 792], turned to the art's orientation), scaleLineweights?: false (lineweights scale with the art), center?: true (false: the drawing's origin on the artboard's bottom-left corner; fitted art's bottom-left corner), mergeLayers?: false (all art on one layer)} (DXF), crop?: as cropTo (its older name)} → {index, title, format, warnings, restored, missingLinks, modifiedLinks, updatedLinks: [{name, path, ids}]}; any readable format (see document.formats): .vectorcraft/.drawcraft, .svg/.svgz, .pdf/.ai, .ait, ASCII .dxf (each DXF layer with art a layer; blocks symbols; what can't come in is listed in warnings), PNG/JPEG/GIF/WebP/TIFF/BMP (an image opens as a document of its size at the resolution it declares, as file.place sizes it; 72 ppi, 1 px = 1 pt, when it declares none), .emf/.wmf (one artboard, the picture's frame; records VectorCraft doesn't read are skipped with one warning), .eps and PostScript .ai (one artboard, the bounding box: an EPS file VectorCraft wrote restores the document it carries, restored: true; other files are read by a PostScript interpreter — paths, colours and spot inks, clips, gradients, images, type in the fonts named — and come in as their TIFF preview, with a warning, when their PostScript can't be read). A PDF/.ai/.ait or SVG saved with Preserve Editing restores the native document it carries (restored: true; a PDF only when no pages are picked), unless the file was changed elsewhere since or the data can't be read: then its artwork is imported and the first warning says why. Templates (native templates, .ait) open as a new untitled document; a restored .ai keeps its path (Save writes .ai again). Linked images are read from their files (looked for at their path, then relative to the document): missing ones show their saved preview (links.relink), modified ones are read again only with Preferences › Update Links: Automatically (else links.update). An SVG's <image> files (relative links from the SVG's folder) stay linked, SVG files become art, missing ones a placeholder in their box (a warning and missingLinks)",
            always,
            load::open
        ),
        cmd!(
            query "file.list",
            "List Folder",
            [],
            None,
            "{path} the files and folders in folder `path`, by name: what an agent may open there (held to the automation read root when one is in force) → {path, entries: [{name, kind: file|folder|link, size? (files, bytes)}], truncated (more than 5000 entries)} (#1163)",
            always,
            file_list
        ),
        cmd!(
            query "document.serialize",
            "Serialize Document",
            [],
            None,
            "{format?: vectorcraft (default)|template|svg|svgz|pdf|ai|png|jpg|webp|gif|png8|txt|dxf|eps|emf|wmf|tiff|bmp|tga|psd (ait, a .ait template, is written by file.saveAsTemplate), …the format's options (see document.formats; SVG ones also as svg: {…}), selectedOnly?: false (the selected objects alone, in their layers)} → {text, warnings} for svg (plus dataBase64, the file, when its encoding isn't UTF-8), else {dataBase64, warnings}; an SVG of several artboards also gives files: [{name, text}], linked images linked: [{name, dataBase64}]",
            has_doc,
            export::serialize
        ),
        cmd!(
            "document.export",
            "Export Document",
            [],
            None,
            "{path?, format?: svg|svgz|pdf|ai|png|jpg|webp|gif|png8|txt|dxf|eps|emf|wmf|tiff|bmp|tga|psd|vectorcraft|template (default: from the path's extension, else png; png8 writes an indexed .png; ait, a .ait template, is written by file.saveAsTemplate; an exporter plug-in's format is plugin.<plug-in id>, which an extension only it lists picks too), params?: {…} (an exporter plug-in's parameters; missing ones take their defaults, see plugin.info), selectedOnly?: false (the selected objects alone, in their layers), artboard?: 0, artboards?: [i…], rect?: [x, y, width, height] (raster formats: that area of the page in points, y down, instead of an artboard; not with artboard, artboards, range or useArtboards), range?: \"1-3, 5\" | \"all\" (1-based; PDF and .ai write one page per artboard, default all; SVG writes one file per artboard, {stem}-{artboard}.svg; raster formats write one artboard), useArtboards?: true (raster: one file per chosen artboard, default all, {stem}-{artboard}.{ext}; pdf/ai: every page) | false (pdf/ai/raster: the bounds of the visible art; SVG has it as an SVG option), ppi?: 72 (raster formats: pixels per inch, stored in the file; wins over scale), scale?: 1 (pixels per point), background?: transparent|white|black|\"#rrggbb\" (jpg: white when transparent), antiAlias?: none|art (default)|type (text snapped to pixels), interlaced?: false (png, Adam7), embedIcc?: true (png: an sRGB chunk marks the colours as sRGB), quality?: 90 (jpg: 0–100), colorModel?: rgb|cmyk|gray (jpg), method?: baseline|optimized|progressive, scans?: 3 (3–5, progressive), embedIcc?: true, imageMap?: none|client|server (an HTML or NCSA map of the objects with a URL, written as <stem>.html / <stem>.map), colors?: 256 (gif/png8: 2–256), reduction?: perceptual|selective (default)|adaptive|web|blackWhite|gray, dither?: none|diffusion (default)|pattern|noise, ditherAmount?: 100, transparency?: true, matte?: white|\"#rrggbb\"|none, interlaced?, lossless?: true (webp: lossy WebP isn't available yet: written lossless, with a warning), encoding?: utf8|utf16 (txt: the stories in stacking order, back to front, a thread once; utf16 with a byte order mark), lineEndings?: lf|crlf, selectionOnly?: false, svg?: {styling, outlineText, images, objectIds, decimals, minify, responsive, useArtboards, preserveEditing, metadata, fewerTspans, hiddenLayers, encoding, profile, embedFonts} (see document.formats), …the SVG options given flat instead, …the PDF options of document.exportPdf, …the DXF options of document.exportDxf (useArtboards: one drawing per artboard), …the EPS options of document.exportEps (useArtboards: one file per artboard, {stem}_{artboard}.eps, after a central {stem}.eps of the whole document when there are two or more and art shows; else the visible art), …emf and wmf write one picture of the artboard (useArtboards: true one file per chosen artboard, false the bounds of the visible art; EMF keeps curves, clipping, transparent images and gradients as images clipped to their shape; WMF flattens curves into polygons behind a placeable header; what a format leaves out comes back in warnings), colorModel?: rgb|cmyk|gray (tiff: rgb keeps transparency as an alpha channel), lzw?: true, byteOrder?: little|big, embedIcc?: true, colorModel?: rgb|gray (bmp), depth?: 24 (bmp: 1 black and white, 4|8 a palette by reduction and dither, 16, 24, 32 with alpha), fileFormat?: windows|os2 (1, 4, 8 or 24 bits), rle?: false (4 and 8 bits, windows), flipRows?: false (top-down rows), depth?: 24 (tga: 16 one-bit alpha, 24, 32 with alpha), colorModel?: rgb|cmyk|gray (psd), layers?: true (each top-level layer a pixel layer with its opacity and blend mode; a background colour is a Background layer; false: one flat image, on white), maxEditability?: false (layers and sublayers become groups, each object a layer named as in the Layers panel, text by its text; clipping, masked or knockout layers stay one layer), hiddenLayers?: false (hidden layers and objects written hidden instead of left out), embedIcc?: true (at most 30000 pixels a side; formats without alpha are flattened on white)} → {path, format, bytes, warnings, files?: [path…] (several), linked?: [path…] (linked images, image maps)}; no path → {dataBase64, format, bytes, warnings, files?: [{name, dataBase64}], linked?: [{name, dataBase64}]}. Never changes the document's path",
            has_doc,
            export::export
        ),
        cmd!(
            "document.exportSelection",
            "Export Selection…",
            ["File"],
            None,
            "{path?, format?: png|jpg|webp|svg|svgz|pdf|ai|gif|png8|tiff|bmp|tga|psd|txt|dxf|eps|emf|wmf|vectorcraft|template (default: from the extension, else png), scale?: 1, …the format's options} the selected objects cropped to their bounds (template layers left out) → {path, format, bytes, warnings, bounds} (no path → {dataBase64, format, bytes, warnings, bounds}); warnings say what the format approximated or left out, as for document.export",
            has_selection,
            export::export_selection
        ),
        cmd!(
            "document.exportForScreens",
            "Export for Screens",
            ["File", "Export"],
            None,
            "{folder?, zip?: false (one .zip of every file, each deflated or stored, whichever is smaller: no folder → {name, dataBase64, bytes, files: [name…]}; with a folder it is written there → {path, bytes, files}), artboards?: [index…] | range?: \"1-3\" (default all), assets?: [asset id…] (instead of artboards: each asset's art alone, cropped to its bounds and named after the asset; see assets.list), layers?: [layer id…] (instead of artboards: each layer's art alone, sublayers included, named after the layer), selection?: one|each (instead of artboards: the selected art as one file, or a file per selected object, named after it), fullDocument?: false (one file per format instead: a PDF of every artboard, other formats the bounds of all visible art, named after the document), includeBleed?: false (artboards grown by the document's bleed), exportPreset?: (a preset of document.exportPresets.list: its settings, under these params), subfolders?: false (each row's files in a sub-folder: its folder, else its size for raster (1x, 2x, 100w…) or its format (SVG, PDF)), folders?: none|rows (as subfolders)|format (a folder per extension: png, svg, pdf…)|item (a folder per artboard, asset, layer or object)|itemFormat (item/extension) (default: rows with subfolders or a preset, else none), name?: (a file name pattern instead of the item's name, with <filename> for the document, <name> for the artboard, asset, layer or object, <index> from 1, #### for the counter zero-padded to the run of #, <format>, <scale>, <suffix> (else the row's suffix ends the name), YYYYMMDD, YYYY-MM-DD, YYYY, <date>, <time>, <YYYY>, <MM>, <DD>, <hh>, <mm> and <ss>; a trailing .ext is dropped, as in `COMPANY-Logo-<filename>-####-YYYYMMDD`; different files it names alike get -2, -3…), counterStart?: 1, counterPer?: item (every file of an item has its number)|file, date?: YYYY-MM-DD (the date the pattern uses; default today, UTC), preset?: mobile (PNG 1x, 2x, 3x) | density (PNG 0.75x–4x in ldpi…xxxhdpi sub-folders) instead of formats (turns subfolders on), formats?: [{format: png|png8|jpg|webp|gif|tiff|bmp|tga|psd|svg|svgz|pdf|eps|ai|vectorcraft, scale?: 1 | \"2x\" | \"100w\" | \"100h\" | \"72ppi\", width?: px | height?: px | ppi? (raster only; width, then height, then ppi win over scale), suffix?: (raster default: @2x, @100w, @100h, none at 1x; vector formats drop size suffixes), folder?: (its sub-folder), quality?: (jpg 0–100), …the format's options (antiAlias, background, preset (pdf)…)}], settings?: {png|png8|jpg|webp|gif|tiff|bmp|tga|psd|svg|svgz|pdf|eps|ai|vectorcraft: {…options for every row of that format}} (rows' own options win), prefix?, antiAlias?: none|art|type (raster rows without their own), openLocation?: (remembered; the app shows the files after exporting)} one file per artboard and format (a PDF holds its artboard alone; artboards with the same name, in any case, get -2, -3…; an unnamed one is Artboard-N) → {files: [path…]}; no folder → {files: [{name, dataBase64}]} (names include sub-folders: 2x/Icon@2x.png); every reply has warnings: [the encoders' warnings for the files, each once: an image that couldn't be decoded, type written in the fallback font…]. eps, ai and vectorcraft rows write a document of the item alone (its one artboard). The params (but zip and assets) are remembered in the document (document.exportSettings; not an undo step)",
            has_doc,
            screens::export_for_screens
        ),
        cmd!(
            query "document.exportPresets.list",
            "Export Presets",
            [],
            None,
            "{} → {presets: [{name, builtIn, settings}]} the built-in batch export presets (Logo Package, Print-Ready PDF), then the saved ones; any name works as document.exportForScreens' exportPreset",
            always,
            screens::presets_list
        ),
        cmd!(
            query "document.exportPresets.save",
            "Save Export Preset",
            [],
            None,
            "{name, settings: {formats, settings, preset, name, prefix, counterStart, counterPer, folders, subfolders, includeBleed, antiAlias, folder, fullDocument} (as document.exportForScreens; other keys are left out)} save (or replace) a batch export preset, kept with the preferences; built-in names are refused → {name, created, settings}",
            always,
            screens::presets_save
        ),
        cmd!(
            query "document.exportPresets.delete",
            "Delete Export Preset",
            [],
            None,
            "{name} delete a saved batch export preset (built-in ones stay) → {deleted}",
            always,
            screens::presets_delete
        ),
        cmd!(
            query "command.batch",
            "Batch",
            [],
            None,
            "{label?, commands: [{command, params}], artboard?: index (the active artboard; the app passes it: each edit.pasteInPlace, edit.pasteInFront, edit.pasteInBack, object.align, object.distribute, object.distributeSpacing or select.allOnArtboard step that names none gets it, and each artboard.fitToArt or artboard.fitToSelection step that names no index gets it as index, or the last artboard of a document with fewer)} run several commands as ONE undo step (one in each document they edit: steps may open, switch, close or revert documents); stops at the first error and rolls everything back, documents included",
            has_doc,
            batch::batch
        ),
        cmd!(
            "action.batch",
            "Batch…",
            [],
            None,
            "{commands: [{command, params}] (the action's steps), source?: folder (default) | datasets, folder? (required for folder: the source folder), includeSubdirs?: false, destination?: none (default: leave each file open) | saveAndClose (save over the source and close) | folder (save or export into destinationFolder and close), destinationFolder? (required when destination is folder; also the folder Override Save/Export write into), overrideOpen?: true (skip document.open / file.open steps: the batch opens each source file), overrideSave?: true (point document.save / file.save / file.saveAs at destinationFolder/{stem}.{ext}), overrideExport?: true (point document.export / document.exportForScreens / document.exportSelection the same way), stopOnError?: true, label?} play the commands on every readable file in folder (and its subfolders when includeSubdirs), or on every data set of the open document (source: datasets; File + Data Set Name for the folder destination and Override Save/Export, with `/`, `\\`, `:`, `<`, `>`, `\"`, `|`, `?`, `*` and control characters in the data set's name as `-`, and `-2`, `-3`… added to a name already used). Caps: 500 files or data sets, 8 folder levels, 256 steps. Batches cannot nest. → {processed: [{path|dataset, result|error}…], count, failed, stopped}",
            always,
            action_batch::action_batch
        ),
        cmd!(
            query "document.formats",
            "File Formats",
            [],
            None,
            "{} → {formats: [{id, label, extensions, mime, read, write, raster, options: {name: {type, default, description}}, plugin?: the id of the exporter plug-in that adds the format (its id is plugin.<plug-in id>; see plugin.list)}], readable: [id…], writable: [id…], openExtensions: [ext…], unsupported: [{id, label, extensions, hint}] (formats asked for that can be neither opened nor written, such as DWG and PICT, with what to use instead)}",
            always,
            formats
        ),
        cmd!(
            query "document.pdfInfo",
            "PDF Info",
            [],
            None,
            "{path | name? | dataBase64 (a file's path, or its bytes), password?, thumbnail?: page (1-based), thumbnailSize?: 160 (px, longest side), cropTo?: crop (the thumbnail's box)} → {pages, needsPassword, wrongPassword?, pageInfo: [{width, height (pt, as shown), rotation, boxes: {media, crop, bleed, trim, art: [x0, y0, x1, y1] (PDF space, pt)}}], thumbnail?: PNG dataBase64}; a page whose UserUnit (PDF 1.6) is above 1 gives its size and boxes in pt, its numbers times the UserUnit (one above 75,000 counts as 75,000); an encrypted PDF without its password → {pages: 0, needsPassword: true}; a PostScript file (an EPS, a .ps file, a .ai in PostScript form) → {pages: 1, needsPassword: false, pageInfo: [{width, height (its bounding box, a US Letter page without one), rotation: 0, boxes: {}}]} (password and cropTo don't apply; thumbnail: 1 shows what the page prints)",
            always,
            pdfimport::pdf_info
        ),
        cmd!(
            "document.exportForOffice",
            "Save for Office Documents…",
            ["File"],
            None,
            "{path?, ppi?: 150 (72, 150, 300… pixels per inch), transparent?: false (else on white), artboard?: 0, acknowledgeLoss?: bool (write over the file the document was read from although reading it left things out)} the artboard as a PNG for office documents and slides → {path, bytes, width, height, warnings} (pixels); no path → {dataBase64, bytes, width, height, warnings}; warnings as for document.export (an image that couldn't be decoded, type in the fallback font or that doesn't fit its frame)",
            has_doc,
            export::export_for_office
        ),
        cmd!(
            query "document.exportSettings",
            "Export for Screens Settings",
            [],
            None,
            "{} → {settings}: the document.exportForScreens params the document last exported with ({} when never; saved with the document, the dialog reopens on them)",
            has_doc,
            screens::export_settings
        ),
    ]
    .into_iter()
    .chain(save::specs())
    .collect()
}
