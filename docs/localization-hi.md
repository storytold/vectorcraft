# Hindi interface terminology and verification

The `hi` catalog is a machine-assisted Hindi draft written from VectorCraft's English strings.
It starts from matching entries in PhotoCraft's clean-room Hindi catalog. VectorCraft's current
loanword preferences below take precedence over that starting vocabulary.
The remaining drafts used the local Argos English–Hindi 1.1 model, followed by an agent wording
pass on commands, vector vocabulary, counted messages, warnings and diagnostic identifiers.
The contributor is a native Hindi speaker and approved the loanword preferences below. This
records terminology feedback, not an independent native proofread of every row. No proprietary
localization resources were used; coverage tests alone do not establish translation quality.

## Style

Prefer familiar design-app loanwords, as requested by the contributor: Save is सेव, Save As is
सेव ऐज़…, a vector path is पाथ (never पाठ), text is टेक्स्ट, a document is डॉक्यूमेंट,
preview is प्रीव्यू and workspace is वर्कस्पेस. The contributor also approved सिलेक्शन,
प्रॉपर्टीज़, ऑप्शन्स and ओपन. Keep Hindi sentence structure and auxiliaries;
use सेव करें within a sentence. Keep nukta in फ़ाइल/फ़ॉन्ट/ज़ूम and -एँ in हटाएँ/दिखाएँ.
Retain ASCII digits, unit symbols, keyboard shortcuts and product/format names. Command IDs,
parameter names, enum values and numbers in diagnostics must stay exactly as supplied.
Keep fragment edge spaces: the hint bar assembles sentences from separately translated pieces.
The built-in workspace and perspective preset names stay in English, as in the other catalogs.

Hindi integer counts use the first plural form for 0 and 1 and the second for 2+ (`plural_french`).
Both `@plural` and `@msgplural` rows have two forms. The latter translate status messages when
displayed; the engine, control channel and MCP continue to return English messages.

In diagnostics, "a number of points" is a numeric value in the point unit: write संख्या and
पॉइंट, never कई अंक ("many digits"). Keep every bound and alternative the English lists, and
every clause such as "all" or "what follows it". Type as a text object is टाइप (एरिया टाइप,
पॉइंट टाइप), not प्रकार, which stays for a kind. `hindi_rows_use_the_glossary_term_for_what_their_english_names`
in `crates/ui-egui/src/i18n/tests.rs` checks the save, swatch, crop, resolution and path terms
and the point unit in every row.

## Terminology

| English | Hindi |
|---|---|
| Layer / Layers | लेयर / लेयर्स |
| Object | ऑब्जेक्ट |
| Selection / Select | सिलेक्शन / सिलेक्ट |
| Properties / Options / Open | प्रॉपर्टीज़ / ऑप्शन्स / ओपन |
| Shape (vector) | शेप |
| Shape (generic form) | आकार |
| Artboard | आर्टबोर्ड |
| Artwork | आर्टवर्क |
| Path / Anchor point / Handle | पाथ / एंकर पॉइंट / हैंडल |
| File path on disk | फ़ाइल पाथ |
| Fill / Stroke | फ़िल / स्ट्रोक |
| Gradient / Mesh / Blend | ग्रेडिएंट / मेश / ब्लेंड |
| Pattern / Swatch | पैटर्न / स्वॉच |
| Type (text object) / Crop / Resolution | टाइप / क्रॉप / रिज़ॉल्यूशन |
| Appearance / Graphic style | अपीयरेंस / ग्राफ़िक स्टाइल |
| Symbol / Envelope | सिंबल / एन्वेलप |
| Clipping mask / Opacity mask | क्लिपिंग मास्क / ओपेसिटी मास्क |
| Image Trace / Live Paint | इमेज ट्रेस / लाइव पेंट |
| Shape Builder / Pathfinder | शेप बिल्डर / पाथफ़ाइंडर |
| Kerning / Leading / Tracking | कर्निंग / लीडिंग / ट्रैकिंग |
| Character / Paragraph | कैरेक्टर / पैराग्राफ़ |
| Save / Save As | सेव / सेव ऐज़… |
| Document / Text | डॉक्यूमेंट / टेक्स्ट |
| Preferences / Workspace | प्रेफ़रेंसेज़ / वर्कस्पेस |
| Preview / Opacity | प्रीव्यू / ओपेसिटी |
| W / H / X / Y | W / H / X / Y |

## Scope and checks (2026-11-06)

The draft has 7,877 unique rows, including 2,537 `@msg`, 45 `@msgplural` and 38 `@plural` rows.
Its source set combines the sibling catalogs with the current test scanner's UI/undo and error
message dumps, so it includes runtime labels outside the literal scanner. Row count is not a
percentage of the application's entire interface.
On 2026-11-13 the catalog has 8,732 rows, including 3,003 `@msg`, 47 `@msgplural` and 41
`@plural` rows. A wording pass on that date corrected 193 rows outside the plug-in
messages for meaning, dropped clauses, English left in place and the save, swatch, crop,
resolution, path and type terms. It is not a native proofread.

The comparison covers release `v0.21.1` (`f6aa4309`) and current `main` (`3c7d9e1f`). Their
non-English catalogs have no missing Hindi keys except the names deliberately kept in English.
A separate scan of the release's `tl!` literals checks strings beyond its catalogs. Current
main added paragraph labels, spine-index validation and two error messages: six rows in total.
Main's workspace version is still 0.21.0; this PR does not change release versioning.

`cargo test -p vectorcraft-ui-egui --lib i18n::tests` passes all 55 tests in Ubuntu 24.04 (WSL).
The untranslated report has no Hindi section, meaning it found no missing Hindi rows among the
categories it checks. This does not check dynamically assembled strings that the scanners miss.
Additional checks compare placeholders, backtick-delimited identifiers and numeric values.
The native Hindi UI fallback test also passes with the installed Nirmala UI font. A running
desktop preview was checked in Hindi, including the New Document dialog and File menu.
The final `cargo xtask ci` passed all 12 stages in Ubuntu 24.04 (WSL): formatting, workspace
clippy, workspace tests, doctests, no-bundled-fonts, PDF dependencies, assets, brands, cursors,
the translation report, layers and WebAssembly. The language picker keeps each language's own
name, including English; its native-menu regression passed with the final catalog.

## Fonts and remaining review

The pinned egui fork shapes Devanagari with HarfRust. The desktop's existing `UiFonts` loader
finds an installed covering face, such as Nirmala UI, Kohinoor Devanagari or Noto Sans Devanagari.
If a `CRAFT_FONTS_DIR` build includes a face tagged `Deva`, it is also installed at the end of
each UI font family. No font binaries are added to this repository. Without either an embedded
or installed covering face, Hindi cannot display correctly. The web has no system fonts and
needs a Devanagari face supplied through craft-fonts; this catalog alone does not supply one.

Before release: native-speaker review of the draft (especially less common labels and error
messages), visual checks for clipped labels and matras at different UI scales, and a web-font
build. Reviewers should use this glossary to make terminology corrections consistently.
