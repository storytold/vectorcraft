# Localization parity

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-10 · **Change:** major (first per-language table, measured from the catalogs) · **Target:** Adobe Illustrator 2026 (30.x)

Per-language status of VectorCraft's interface. How the catalogs work and how to add a language:
[`development.md` › Localisation](development.md#localisation). Part of
[target-app-parity.md](target-app-parity.md).

**How it is measured:** rows with a translation in each `crates/ui-egui/src/i18n/<code>.tsv`, counted on
origin/main `01e165af`. The reference is the French catalog, which the tests hold complete: 3,416 interface
strings (menus, command labels, `tl!` literals: panels, dialogs, tooltips, Preferences) plus 604 `@msg` status and
error messages, 4,020 in all. A language's % is its rows over 4,020 (capped at 100% per part). The
`complete_menus` flag and `COMPLETE_MESSAGES` list make `i18n::tests` fail if a complete language misses a
string, so "full" here is enforced by tests. No language has help content: there is no translated (or English)
in-app help, and the docs are English.

**Native review:** every catalog was written clean-room from the English meaning, most of it by agents
(`bflatastic`), with community contributors adding or correcting (German by `dwetscher`, Japanese and Traditional
Chinese by `imachus`). None is recorded as reviewed by a native speaker; the catalog headers ask for one.

## The twelve key languages

| Language | Code | UI strings translated | Dialogs / tooltips / messages | Script support | Native review | Status | To `full` (h) |
|---|---|---|---|---|---|---|---|
| English | `en` | source, 100% | all | Latin | — | full | — |
| Simplified Chinese (Mandarin) | `zh-hans` | 3,647 rows; menus, panels, dialogs complete (tests); 85% overall | messages in English (1 of 604) | CJK UI font from craft-fonts (BIZ UDPGothic) when embedded, else a system font; IME with marked text at the caret; vertical type partial ([type-parity.md](type-parity.md)) | no | partial | 2–3 |
| Spanish | `es` | 4,009 / 4,020 (99.7%) | all (604 messages) | Latin | no | full | — (review) |
| Hindi | `hi` | 0 | none | Devanagari: the document's text shapes (HarfRust), untested for Devanagari, but the egui interface does no complex shaping | no | none | 20–30 |
| Arabic | `ar` | 0 | none | document text: bidi layout and Paragraph Direction; interface: no right-to-left layout, no Arabic shaping in egui | no | none | 25–40 |
| French | `fr` | 4,020 / 4,020 (100%) | all | Latin | no | full | — (review) |
| Portuguese | `pt-br` | 2,726 / 4,020 (68%): every menu label and `tl!` literal | messages in English | Latin | no | partial | 4–6 |
| Indonesian | `id` | 0 | none | Latin | no | none | 5–8 |
| Japanese | `ja` | 4,004 / 4,020 (99.6%) | all | CJK UI font as above; Japanese IME; vertical type partial; Japanese crop marks and composition settings | no | full | — (review) |
| German | `de` | 3,952 / 4,020 (98%; tests hold menus, literals and messages complete) | all (602 of 604 message rows; landed 2026-10-10) | Latin | contributor-written, not reviewed | full | — (review) |
| Korean | `ko` | 0 | none | Hangul needs a UI font in craft-fonts; the IME path exists | no | none | 6–10 |
| Vietnamese | `vi` | 0 | none | Latin with stacked diacritics (precomposed forms render) | no | none | 5–8 |

**12-language score: ~54%** (equal weights: 100 + 85 + 100 + 0 + 0 + 100 + 68 + 0 + 100 + 98 + 0 + 0, over 12).
**To `full` for all twelve: ~70–110 h**, most of it Arabic (right-to-left interface) and Hindi (interface
shaping). Each new Latin-script catalog is ~4,000 rows; the existing ones were written in a few agent hours each
plus review. Native-speaker review is human work and isn't in the hours.

## Other shipped languages

| Language | Code | Translated | Status |
|---|---|---|---|
| Traditional Chinese (Taiwan) | `zh-hant` | 3,425 rows, menus and literals complete; messages in English (80% overall) | partial |
| Italian | `it` | 4,011 / 4,020 | full |
| Russian | `ru` | 4,009 / 4,020 | full |
| Ukrainian | `uk` | 4,009 / 4,020 (one/few/many plurals) | full |
| Czech | `cs` | 679 / 4,020 (17%): every menu label | menus only |

Eleven non-English catalogs in all. Illustrator, as its public documentation lists, ships around two dozen
interface languages (including Korean, Polish, Dutch, Turkish, the Nordic languages, Hungarian, and Middle East
editions with Arabic and Hebrew); it doesn't ship Hindi, Indonesian or Vietnamese.

## Not done in any language

Locale-aware number and date formats, a right-to-left interface, translated help, and a Traditional Chinese UI
font for the web build.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-10 | major | First per-language table, measured from the catalog row counts against the complete French catalog |
