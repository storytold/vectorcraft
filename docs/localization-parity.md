# Localization parity

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-11-24 · **Change:** minor (Arabic (`ar`): catalog added with Modern Standard Arabic menus, preferences, tools, dialogs, commands and 6-form CLDR plurals; native name العربية; Hindi catalog draft complete under the translation tests; contributor loanword terminology reviewed; wider proofreading welcome; Brazilian Portuguese: 154 rows for the strings the sibling catalogs translate — smart-filter and print presets, dimension lines, swatches, canvas messages; previously minor: Bosnian (`bs`): complete catalog; previously minor: Vietnamese text: stacked diacritics tested and attach in every form, #4491; previously: Catalan (`ca`) catalog added, complete under the tests)

Per-language status of VectorCraft's interface. How the catalogs work and how to add a language:
[`development.md` › Localisation](development.md#localisation). Part of
[target-app-parity.md](target-app-parity.md).

**How it is measured:** rows with a translation in each `crates/ui-egui/src/i18n/<code>.tsv`, counted on
origin/main `01e165af`, except the rows that give a count date: those were counted on the `main` commit named in the row (the rows dated 2026-10-21 on `main` at `cb0a7d9`).
The percentages (Spanish's, and the historical score below) use the French catalog on `01e165af` as the reference, which was complete when counted: 3,416 interface
strings (menus, command labels, `tl!` literals: panels, dialogs, tooltips, Preferences) plus 604 `@msg` status and
error messages, 4,020 in all. A language's % is its rows over 4,020 (capped at 100% per part); today's catalogs are
larger, so the row counts dated 2026-11-17 give no percentage, and the untranslated report's count for a language
includes the strings it lists under "every language". A missing
translation never fails a test or a build: the interface shows the English text until a translator adds the row.
`cargo xtask ci` prints the untranslated report (`cargo xtask i18n-report` alone), which lists per language the
strings missing from each catalog with the `complete_menus` flag and the messages missing from each language in
the `COMPLETE_MESSAGES` list. Every non-English language marked "full" has the flag, and all of them except
Bosnian and Bulgarian are in the list. No language has help content: there is no translated (or
English) in-app help, and the docs are English.

**Native review:** every catalog was written clean-room from the English meaning, most of it by agents
(`bflatastic`), with community contributors adding or correcting (German by `dwetscher`, Japanese and Traditional
Chinese by `imachus`). Georgian and the original Gujarati catalog are recorded as reviewed by native speakers; the contributor approved all 387 later Gujarati additions on 2026-11-14.

> **Gujarati status (2026-11-14, #6613):** 4,673 catalog rows: the original 4,286 remain unchanged; all 387 additions are contributor-reviewed and approved, with 11 wording corrections. Status/error messages remain partial, and the pinned CI/release fonts and web startup list contain no Gujarati face; web selection stays unavailable until a Gujarati interface face is bundled. Scores and estimates unchanged.

## The thirteen key languages

| Language | Code | UI strings translated | Dialogs / tooltips / messages | Script support | Native review | Status | To `full` (h) |
|---|---|---|---|---|---|---|---|
| English | `en` | source, 100% | all | Latin | — | full | — |
| Simplified Chinese (Mandarin) | `zh-hans` | 9,625 rows, 3,735 of them `@msg` and 51 `@msgplural` (counted 2026-11-17 on `main` at `ee60c51fd9`) | all (the untranslated report lists 237 messages without a row for it) | CJK UI font from craft-fonts (BIZ UDPGothic) when embedded, else a system font; IME with marked text at the caret; vertical type partial ([type-parity.md](type-parity.md)) | no | full | — (review) |
| Spanish | `es` | 4,009 / 4,020 (99.7%) | all (3,770 `@msg` and 55 `@msgplural` rows, counted 2026-11-18: the untranslated report lists 269 messages without a Spanish row, 236 in its own section and 33 that all 24 checked languages lack) | Latin | no | full | — (review) |
| Hindi | `hi` | 8,782 rows (2026-11-13): 3,014 `@msg`, 47 `@msgplural`, 41 `@plural`; the untranslated report on main `f793044f83` (2026-11-14) lists 676 missing rows for it: 608 in its own section and 68 that every checked language lacks | menus, dialogs, tooltips and scanned messages; contributor-approved loanword terminology; agent wording pass | UI and document text shape with HarfRust; installed desktop Devanagari fallback, or a `Deva` craft-fonts face; the web fetches Noto Sans Devanagari when the interface needs it | contributor terminology review; 2026-11-13 wording pass on messages and longer labels, and a word-sense pass on short labels, with a glossary-term test; wider proofread welcome | complete catalog, machine-assisted draft | reported missing rows, native review and layout |
| Gujarati | `gu` | 4,673 rows (2026-11-14): original 4,286 unchanged; all 387 additions contributor-reviewed and approved | menus and dialogs; status/error messages partial | installed Gujarati fonts on desktop; optional `Gujr` craft-fonts fallback; no Gujarati face in the pinned CI/release fonts or web startup list | original catalog reviewed by a native speaker; all 387 later additions approved by the contributor, with 11 wording corrections | contributor-reviewed catalog | remaining messages and a bundled Gujarati web font |
| Arabic | `ar` | registered with 6-form CLDR plurals and native name العربية; translations maintained upstream in `storytold/craft-i18n` (`locales/ar/vectorcraft.tsv`) under AGENTS.md rule 16; preparatory registration kept unavailable in picker until upstream catalog is consumed | menus, dialogs, preferences | Arabic: document text has bidi layout and paragraph direction; interface strings render in Modern Standard Arabic via system fallback fonts; full shell mirroring pending STD-I18N-001 | no (AI-drafted, clean-room) | preparatory | 15–20 |
| French | `fr` | 9,166 rows, 3,436 of them `@msg` and 48 `@msgplural` (counted 2026-11-13): the untranslated report lists 178 missing messages for it | all | Latin | no | full | — (review) |
| Portuguese | `pt-br` | 9,378 rows, 3,563 of them `@msg` and 48 `@msgplural` (counted 2026-11-13): the untranslated report lists 55 missing messages for it, which open #7116, #7211, #7330, #7378 and #7405 translate; one/other plurals | all | Latin | contributor is a Brazilian Portuguese speaker; further native review welcome | full | — (review) |
| Indonesian | `id` | 7,553 rows, 1,682 of them `@msg` and 49 `@msgplural` (counted 2026-11-15 on `main` at `df984b2a02`): in `COMPLETE_MESSAGES`, and the untranslated report lists 50 missing menu strings, 35 `tl!` literals, 11 undo labels and 1,929 messages for it, the same sets as for Danish; one plural form | most dialogs and tooltips; status and error messages partial | Latin | no | full | — (review) |
| Japanese | `ja` | 4,004 / 4,020 (99.6%) | partial (3,562 `@msg` and 49 `@msgplural` rows, counted 2026-11-14: the untranslated report lists 98 messages without a Japanese row, 77 of which every complete-message language lacks; open #7116 and #7211 translate 35 of them) | CJK UI font as above; Japanese IME; vertical type partial; Japanese crop marks and composition settings | no | full | — (review) |
| German | `de` | 3,952 / 4,020 (98%; menus, literals and messages complete when counted) | all (3,563 `@msg` and 48 `@msgplural` rows, counted 2026-11-13: the untranslated report lists 52 missing messages for it, which open #7116, #7211 and #7378 translate) | Latin | contributor-written, not reviewed | full | — (review) |
| Korean | `ko` | 4,870 rows (counted 2026-10-21): menus, `tl!` literals and messages complete when counted; one plural form | all (3,438 `@msg` and 48 `@msgplural` rows, counted 2026-11-13: the untranslated report lists 39 missing messages for it) | Hangul from an installed system font on the desktop (Apple SD Gothic Neo, Malgun Gothic, Noto Sans CJK KR…); the web build fetches NanumGothic from craft-fonts when the interface needs it; IME path exists | no (AI-drafted) | full | — (review) |
| Vietnamese | `vi` | 7,654 rows, 1,968 of them `@msg` and 51 `@msgplural` (counted 2026-11-17 on `main` at `ee60c51fd9`); one plural form | dialogs and tooltips all; messages partial (the untranslated report lists 1,974 messages without a Vietnamese row) | Latin with Vietnamese diacritics (stacked marks attach in precomposed, decomposed and keyboard forms, #4491; glyph tests pass) | contributor is a Vietnamese speaker; AI-assisted draft, further native review welcome | full | — (review) |

**Historical 12-language score: ~82%** (equal weights: 100 + 85 + 100 + 0 + 0 + 100 + 100 + 100 + 100 + 98 + 100 + 100, over 12).
**To `full` for the twelve in that score: ~50–85 h** (historical estimate, not re-scored here). Hindi now has
a catalog that the tests count as complete, and UI shaping; its remaining work is the rows the
untranslated report lists for it, native review and layout checking. Arabic still needs its catalog and right-to-left interface. Each new Latin-script catalog is ~4,000 rows; the existing ones were written in a few agent hours each
plus review. Native-speaker review is human work and isn't in the hours.

## Other shipped languages

| Language | Code | Translated | Status |
|---|---|---|---|
| Traditional Chinese (Taiwan) | `zh-hant` | 9,428 rows, 3,735 of them `@msg` and 51 `@msgplural` (counted 2026-11-17 on `main` at `ee60c51fd9`): the untranslated report lists 237 missing messages for it; menus, `tl!` literals and messages were complete when counted on 2026-10-21 (80% overall) | partial |
| Italian | `it` | 9,415 rows, 3,740 of them `@msg` and 51 `@msgplural` (counted 2026-11-17 on `main` at `ee60c51fd9`): the untranslated report lists 237 missing messages for it | full |
| Dutch | `nl` | 9,848 rows, 3,747 of them `@msg` and 51 `@msgplural` (counted 2026-11-17 on `main` at `ee60c51fd9`): in `COMPLETE_MESSAGES`, and the untranslated report lists 235 missing messages for it; one/other plurals | full |
| Danish | `da` | 9,928 rows, 3,838 of them `@msg` and 51 `@msgplural` (counted 2026-11-17 on `main` at `ee60c51fd9`): in `COMPLETE_MESSAGES`, and the untranslated report lists 118 missing messages for it, 15 of them missing in every language; one/other plurals | full |
| Russian | `ru` | 9,866 rows, 3,742 of them `@msg` and 51 `@msgplural` (counted 2026-11-17 on `main` at `ee60c51fd9`): in `COMPLETE_MESSAGES`; the untranslated report lists 242 messages, 72 menu strings, 34 `tl!` literals and 16 undo labels as missing | full |
| Ukrainian | `uk` | 9,412 rows, 3,738 of them `@msg` and 51 `@msgplural` (counted 2026-11-17 on `main` at `ee60c51fd9`); one/few/many plurals | full |
| Hungarian | `hu` | 9,774 rows, 3,743 of them `@msg` and 51 `@msgplural` (counted 2026-11-17 on `main` at `ee60c51fd9`): the untranslated report lists 236 missing messages for it, and lists 73 menu strings, 38 `tl!` strings, 16 undo labels and 45 swatch names without a row; one plural form | full |
| Polish | `pl` | 9,650 rows, 3,736 of them `@msg` and 51 `@msgplural` (counted 2026-11-17 on `main` at `ee60c51fd9`): the untranslated report lists 242 missing messages for it, and 73 menu strings, 39 `tl!` literals, 16 undo labels and 45 swatch names; one/few/many plurals | full; native review pending |
| Finnish | `fi` | 8,074 rows, 1,975 of them `@msg` and 51 `@msgplural` (counted 2026-11-17 on `main` at `ee60c51fd9`): the untranslated report lists 1,974 missing messages for it; one/other plurals | full; AI-assisted, native proofreading pending |
| Czech | `cs` | 9,761 rows, 3,742 of them `@msg` and 51 `@msgplural` (counted 2026-11-17 on `main` at `ee60c51fd9`): the untranslated report lists 222 missing Czech messages, and 15 messages that no checked language has a row for | catalog complete; render-site gaps reduced; native review pending |
| Bosnian | `bs` | 7,227 rows, 1,205 of them `@msg` and 47 `@msgplural` (counted 2026-11-17 on `main` at `ee60c51fd9`): the untranslated report lists 113 missing menu strings, 66 `tl!` literals and 30 undo labels for it; not in `COMPLETE_MESSAGES`, so the report does not check its messages; one/few/other plurals | full; AI-assisted, native review pending |
| Swedish | `sv` | 9,498 rows, 3,311 of them `@msg` and 51 `@msgplural` (counted 2026-11-17 on `main` at `ee60c51fd9`): the untranslated report lists 647 missing messages for it, and 31 menu strings, 14 `tl!` literals and 6 undo labels; one/other plurals | full; native review welcome |
| Bulgarian | `bg` | 7,133 rows, 1,111 of them `@msg` and 47 `@msgplural` (counted 2026-11-17 on `main` at `ee60c51fd9`): the untranslated report lists 76 menu strings, 48 `tl!` literals, 16 undo labels and 45 swatch names for it; its messages are partial | full; AI-drafted, native review pending |
| Greek | `el` | 8,003 rows, 1,972 of them `@msg` and 51 `@msgplural` (counted 2026-11-17 on `main` at `ee60c51fd9`): the untranslated report lists 1,974 missing messages for it | full; native review welcome |
| Georgian | `ka` | 9,758 rows, 3,740 of them `@msg` and 49 `@msgplural` (counted 2026-11-15): menus, panels, dialogs and plurals (one count-neutral form); the untranslated report lists 1 missing Georgian message; reviewed by a native speaker | full |
| Turkish | `tr` | 9,784 rows, 3,746 of them `@msg` and 51 `@msgplural` (counted 2026-11-17 on `main` at `ee60c51fd9`); 235 messages, 79 menu strings, 39 `tl!` literals, 16 undo labels and the default swatch names still fall back to English | full |
| Catalan | `ca` | 8,036 rows, 1,956 of them `@msg` and 51 `@msgplural` (counted 2026-11-17 on `main` at `ee60c51fd9`): the untranslated report lists 1,990 missing messages for it; one/other plurals; every `ca-*` locale resolves here | full; AI-drafted, native review welcome |

Twenty-seven non-English catalogs in all (registry count, 2026-11-06). Illustrator, as its public documentation lists, ships around two dozen
interface languages (including Polish, Dutch, Turkish, the Nordic languages, Hungarian, and Middle East
editions with Arabic and Hebrew); it doesn't ship Hindi, Indonesian or Vietnamese.

## Czech render-site follow-up

The desktop and web file pickers look up filter labels at assignment time; format names and
extensions stay unchanged. Diagnostics retain numeric values and units. New Document's suggested
preset name uses `Preset {number}`. Contributor tooltips use named placeholders and Czech plurals;
handles and chosen names remain unchanged. Gradient descriptions reuse `{kind} Gradient` with a
contextual radial adjective. PDF/Print mark styles use `mark-style`, so `Japanese` displays as
`Japonské` while the language name stays `Japonština`; their Summary uses the same context.

The search placeholder, Home screen, toolbar/panel tooltips and Preferences already use lookups.
No new `tl!("literal")` keys were needed: only `cs.tsv` adds 19 AI-draft rows and revises the gradient
template. Every other complete catalog (`complete_menus`) has the same rows and a `Roman` row for the
other mark style; the untranslated report lists a catalog that lacks one of them.
Native review is still needed, and catalog coverage does not prove that every variable in the UI is
translated.

## Not done in any language

Locale-aware number and date formats, a right-to-left interface, translated help, and a Traditional Chinese UI
font for the web build.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-11-24 | minor | Arabic (`ar`): catalog added with Modern Standard Arabic menus, preferences, tools, dialogs, commands and 6-form CLDR plurals; registered as twenty-sixth non-English language; native name العربية |
| 2026-11-14 | minor | Gujarati (#6613): contributor proofreading completed for all 387 additions, with 11 wording corrections; 4,673 reviewed catalog rows. Status/error messages remain partial; web selection requires a bundled Gujarati face. Scores and estimates unchanged. |
| 2026-11-12 | minor | Gujarati (`gu`): reviewed by a native speaker; zero/one singular plurals (CLDR); Gujarati craft-fonts faces join the UI fonts; strings added on main since branching translated (4,286 rows) |
| 2026-11-11 | minor | Gujarati (`gu`): 4,235-row machine-assisted interface catalog, one/other plurals, desktop Gujarati font fallback and a glyph-coverage filter; status and error messages not yet translated; native review pending |
| 2026-11-06 | minor | Hindi (`hi`): 7,877-row machine-assisted catalog, zero/one singular plurals, desktop font fallback and optional Devanagari craft-fonts fallback; all 55 translation tests pass; native review and a web font remain; scores and estimates unchanged |
| 2026-10-30 | minor | Bosnian (`bs`, Latin, Ijekavian): complete catalog of 5,841 rows, every `bs-*` locale, one/few/other plurals; AI-assisted, not yet native-reviewed |
| 2026-10-29 | minor | Vietnamese text: letters with two marks (Ể, Ứ, Ự) attach in precomposed, decomposed and keyboard forms, and a font lacking the marks falls back for the whole letter (#4491); scores unchanged |
| 2026-10-28 | minor | Catalan (`ca`): complete catalog under the tests (menus, literals, messages; one/other plurals); every `ca-*` locale resolves here; AI-drafted, native review welcome. Added the Turkish and Indonesian rows the table was missing. Scores and estimates unchanged |
| 2026-10-23 | patch | A missing translation never fails a test or a build; one untranslated report, printed by `cargo xtask ci` and `cargo xtask i18n-report`, lists the missing strings per language; scores unchanged |
| 2026-10-16 | minor | Bulgarian (`bg`, #1909) and Greek (`el`, #2076) catalogs added, complete under the tests; Swedish (#2139) and Finnish (#1994) rows for strings added since; eighteen non-English catalogs |
| 2026-10-16 | minor | Finnish (`fi`) catalog added, complete under the tests (menus, literals, messages; one/other plurals); AI-assisted, native proofreading pending |
| 2026-10-15 | minor | Swedish (`sv`) catalog added, complete under the tests (menus, literals, messages; one/other plurals); every `sv-*` locale resolves to it; AI-drafted, native review welcome; scores and estimates unchanged |
| 2026-10-14 | minor | Brazilian Portuguese (`pt-br`, #1914): catalog completed under the tests (menus, literals, messages); 12-language score 71% → 74%, hours 55–90 → 50–85 |
| 2026-10-13 | minor | The Polish translator confirmed the Polish rows the maintainers had added after the catalog was written (#1614); Tool flyouts are as wide as their longest translated name and shortcut need, so names no longer run into shortcuts in Spanish and other languages (#1650, fixes #1616); scores and estimates unchanged |
| 2026-10-13 | minor | Polish (`pl`, #1498): the row count is corrected to the 4,510 rows in `pl.tsv` (it said 4,492), 654 of them `@msg` rows; AI-drafted, contributor loanword terminology reviewed; wider proofreading welcome; scores and estimates unchanged |
| 2026-10-13 | minor | French: the 423 strings added since the catalog was last complete are translated, and 188 entries are reworded to the feature names French users know (#1511); Hungarian: one message uses the informal address like the rest of the interface (#1474); the entry counts in the tables are the earlier measurements; native review still pending; scores and estimates unchanged |
| 2026-10-12 | minor | Vietnamese (`vi`) catalog added, complete under the tests (menus, literals, messages; one plural form); 12-language score 63% → 71%, hours 60–100 → 55–90 |
| 2026-10-13 | minor | Czech: 128 more strings translated (tool names, PDF and Image Trace presets, pie graph options and others added since the catalog was completed; #1461); the entry count in the table is the 2026-10-11 measurement; native review still pending; scores and estimates unchanged |
)
| 2026-10-12 | minor | Korean (`ko`) catalog added, complete under the tests (menus, literals, messages); 12-language score 54% → 63%, hours 70–110 → 60–100 |
| 2026-10-12 | minor | Polish (`pl`) catalog added, complete under the tests (menus, literals, messages; one/few/many plurals) |
| 2026-10-12 | minor | Hungarian (`hu`) catalog added, complete under the tests |
| 2026-10-12 | minor | Georgian (`ka`) catalog, complete |
| 2026-10-11 | minor | Czech render-site follow-up: file-dialog filters, diagnostics, preset numbering, contributor tooltips/plurals, gradient grammar and contextual printer mark styles; estimates unchanged |
| 2026-10-10 | major | First per-language table, measured from the catalog row counts against the complete French catalog |
