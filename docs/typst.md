# Typst

Type › **Typst…** typesets [Typst](https://typst.app) markup (math, text, tables, anything Typst draws) and places
it as one group of paths that keeps its source. Select the group and choose Type › Typst… again to change the
source: it is typeset again where it was, at the size and angle you gave it. Like TexText in Inkscape, but for
Typst. It isn't an Illustrator feature.

## Using it

Typst is **off by default**: turn on **Enable Typst** in Preferences › Type, or in the dialog. Until then
nothing is typeset, so a document, the control channel or MCP can't make the app run `typst`. Automation
confined with `--automation-read-root` can't turn it on (`prefs.set` refuses), only off.

- **Text**: the Typst markup. Write math between dollar signs (`$ sum_(k=1)^n k = (n(n+1))/2 $`) or ordinary
  Typst markup (`#emph[hello]`, `#table(…)`).
- **Preamble**: markup read before the text, for `#set`, `#show`, `#import` and `#let` rules, such as
  `#set text(font: "New Computer Modern", fill: blue)`.
- **Size**: the text size the markup starts with (pt; 11 by default, as in Typst).

OK typesets. When Typst reports an error, the dialog stays open and shows it, with its line counted in the text
or in the preamble (`line 2:4: error: expected expression`).

The page is as big as what it draws, with no margins and no fill; only the first page is placed. Glyphs come in
as plain filled paths (no fonts are needed to open, print or export the document), in the colours the markup
gives them. The group can be moved, scaled, rotated and recoloured like any other. Editing it typesets it
again: its centre stays where it is, with its scale and rotation, and the group keeps its own name,
appearance, opacity, opacity mask, attributes and graphic style, but colours and other changes to the paths
inside are replaced by what Typst draws. Ungroup it to keep the paths as ordinary art.

The source is saved in the VectorCraft document (`.vectorcraft`). Formats that only carry art (SVG, PDF, EPS…)
export the paths.

## Requirements

- The **desktop app** with Typst installed: it runs the `typst` program on the `PATH` (tested with Typst 0.15), or the
  one the `VECTORCRAFT_TYPST` environment variable names. The web app can't run programs, so there the command
  explains that.
- Typst finds fonts as it always does: the fonts it embeds, the system's, and `TYPST_FONT_PATHS`.
- **Network:** `#import "@preview/…"` packages are downloaded by Typst from its package registry the first
  time they are used, as on the command line, and cached by Typst. The app doesn't make or check that
  download, and Typst has no setting to turn it off; the dialog says so next to Enable Typst.
- `#image` and `#read` paths are relative to the document's folder (for a document not yet saved, a private
  empty folder made for the run, under a random name, and removed after it). Typst can't read outside it,
  except through links in it. When automation is confined (`--automation-read-root`), Typst always gets the
  empty folder, since a link in the document's folder could lead outside the read root.
- Typst gets 30 seconds per typesetting.

## Commands

| Command | Params | Result |
|---|---|---|
| `type.typst.insert` | `{source, preamble?, fontSize?, at?: [x, y]}` | `{id, width, height}`: the group, selected, centred on `at` (default: the first artboard's centre) |
| `type.typst.edit` | `{id?, source?, preamble?, fontSize?}` (what isn't given stays) | `{id, width, height}` (the edited group has a new id) |
| `type.typst.info` | `{id?}` | `{id, source, preamble, fontSize}` |
| `ui.typst` | `{}` | opens the dialog (`typst`: fields `source`, `preamble`, `fontSize`) |

`id` defaults to the selected Typst object (an object selected inside one counts); with two or more selected,
`ui.typst` and the commands ask to select one. Insert and edit need `enableTypst` on (`prefs.set`); info doesn't. Each insert or edit is one
undo step. The program is never chosen by command params, so the control channel and MCP can't make the app run
another one.

## How it works

The markup is wrapped with `#set page(width: auto, height: auto, margin: 0pt, fill: none)` and the text size,
piped to `typst compile --format svg --pages 1 - -`, and the SVG read back with the SVG importer
(`crates/engine/src/cmd/typst.rs`). Typst draws each glyph as a `<use>` of an outline; those instances are
broken into paths, and the one-object groups around them dropped. The group stores a `TypstSpec`
(`crates/doc/src/typst.rs`): the source, preamble and size, the typeset page, and the transform from the page
to the document, which every transform of the group updates. An edit places the new page through that transform,
centred where the old page was.
