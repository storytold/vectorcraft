# Localization parity

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-16 · **Change:** minor (Bulgarian (`bg`) and Greek (`el`) catalogs complete under the tests, #1909, #2076; Finnish (`fi`) catalog added, complete under the tests; Swedish (`sv`) catalog added, complete under the tests; scores and estimates unchanged; Brazilian Portuguese (`pt-br`) catalog completed under the tests: menus, `tl!` literals and messages; 12-language score 71% → 74%; #1914) · **Target:** Adobe Illustrator 2026 (30.x)

Per-language status of VectorCraft's interface. How the catalogs work and how to add a language:
[`development.md` › Localisation](development.md#localisation). Part of
[target-app-parity.md](target-app-parity.md).

**How it is measured:** rows with a translation in each `crates/ui-egui/src/i18n/<code>.tsv`, counted on
origin/main `01e165af`, except the Czech row updated against the current local catalog.
The reference is the French catalog, which the tests hold complete: 3,416 interface
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
| Portuguese | `pt-br` | 4,707 catalog entries: menus, `tl!` literals and messages complete (tests); one|other plurals | all | Latin | contributor is a Brazilian Portuguese speaker; further native review welcome | full | — (review) |
| Indonesian | `id` | 0 | none | Latin | no | none | 5–8 |
| Japanese | `ja` | 4,004 / 4,020 (99.6%) | all | CJK UI font as above; Japanese IME; vertical type partial; Japanese crop marks and composition settings | no | full | — (review) |
| German | `de` | 3,952 / 4,020 (98%; tests hold menus, literals and messages complete) | all (602 of 604 message rows; landed 2026-10-10) | Latin | contributor-written, not reviewed | full | — (review) |
| Korean | `ko` | 4,151 rows: menus, `tl!` literals and messages complete (tests); one plural form | all | Hangul from an installed system font on the desktop (Apple SD Gothic Neo, Malgun Gothic, Noto Sans CJK KR…); the web build has no Korean UI font yet (craft-fonts); IME path exists | no (AI-drafted) | full | — (review; web font) |
| Vietnamese | `vi` | 4,225 rows: menus, `tl!` literals and messages complete (tests); one plural form; 3,548 plain + 13 `@plural` + 12 contextual + 652 `@msg` | all | Latin with Vietnamese diacritics (precomposed forms; glyph tests pass) | contributor is a Vietnamese speaker; AI-assisted draft, further native review welcome | full | — (review) |

**12-language score: ~74%** (equal weights: 100 + 85 + 100 + 0 + 0 + 100 + 100 + 0 + 100 + 98 + 100 + 100, over 12).
**To `full` for all twelve: ~50–85 h**, most of it Arabic (right-to-left interface) and Hindi (interface
shaping). Each new Latin-script catalog is ~4,000 rows; the existing ones were written in a few agent hours each
plus review. Native-speaker review is human work and isn't in the hours.

## Other shipped languages

| Language | Code | Translated | Status |
|---|---|---|---|
| Traditional Chinese (Taiwan) | `zh-hant` | 3,425 rows, menus and literals complete; messages in English (80% overall) | partial |
| Italian | `it` | 4,011 / 4,020 | full |
| Russian | `ru` | 4,009 / 4,020 | full |
| Ukrainian | `uk` | 4,009 / 4,020 (one/few/many plurals) | full |
| Hungarian | `hu` | 4,524 rows: menus, `tl!` literals and messages complete (tests); one plural form | full |
| Polish | `pl` | 4,510 rows (counted 2026-10-13): menus, `tl!` literals and messages complete (tests); one/few/many plurals | full; native review pending |
| Finnish | `fi` | 4,770 rows: menus, `tl!` literals and messages complete (tests); one/other plurals, 668 `@msg` rows | full; AI-assisted, native proofreading pending |
| Czech | `cs` | 4,411 catalog entries, including 636 `@msg` rows (measured locally 2026-10-11); menu, literal and message coverage enforced by tests | catalog complete; render-site gaps reduced; native review pending |
| Swedish | `sv` | 4,770 rows, 668 of them `@msg` (counted 2026-10-15): menus, `tl!` literals and messages complete (tests); one/other plurals | full; native review welcome |
| Bulgarian | `bg` | 4,910 rows, 703 of them `@msg` (counted 2026-10-16): menus, `tl!` literals and messages complete (tests) | full; AI-drafted, native review pending |
| Greek | `el` | 4,880 rows, 709 of them `@msg` (counted 2026-10-16): menus, `tl!` literals and messages complete (tests) | full; native review welcome |

Eighteen non-English catalogs in all. Illustrator, as its public documentation lists, ships around two dozen
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
template. Other catalogs fall back to English for new variable/template lookups. Native review is
still needed, and catalog coverage does not prove that every variable in the UI is translated.

## Not done in any language

Locale-aware number and date formats, a right-to-left interface, translated help, and a Traditional Chinese UI
font for the web build.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-16 | minor | Bulgarian (`bg`, #1909) and Greek (`el`, #2076) catalogs added, complete under the tests; Swedish (#2139) and Finnish (#1994) rows for strings added since; eighteen non-English catalogs |
| 2026-10-16 | minor | Finnish (`fi`) catalog added, complete under the tests (menus, literals, messages; one/other plurals); AI-assisted, native proofreading pending |
| 2026-10-15 | minor | Swedish (`sv`) catalog added, complete under the tests (menus, literals, messages; one/other plurals); every `sv-*` locale resolves to it; AI-drafted, native review welcome; scores and estimates unchanged |
| 2026-10-14 | minor | Brazilian Portuguese (`pt-br`, #1914): catalog completed under the tests (menus, literals, messages); 12-language score 71% → 74%, hours 55–90 → 50–85 |
| 2026-10-13 | minor | The Polish translator confirmed the Polish rows the maintainers had added after the catalog was written (#1614); Tool flyouts are as wide as their longest translated name and shortcut need, so names no longer run into shortcuts in Spanish and other languages (#1650, fixes #1616); scores and estimates unchanged |
| 2026-10-13 | minor | Polish (`pl`, #1498): the row count is corrected to the 4,510 rows in `pl.tsv` (it said 4,492), 654 of them `@msg` rows; AI-drafted, native review pending; scores and estimates unchanged |
| 2026-10-13 | minor | French: the 423 strings added since the catalog was last complete are translated, and 188 entries are reworded to the feature names French users know (#1511); Hungarian: one message uses the informal address like the rest of the interface (#1474); the entry counts in the tables are the earlier measurements; native review still pending; scores and estimates unchanged |
| 2026-10-12 | minor | Vietnamese (`vi`) catalog added, complete under the tests (menus, literals, messages; one plural form); 12-language score 63% → 71%, hours 60–100 → 55–90 |
| 2026-10-13 | minor | Czech: 128 more strings translated (tool names, PDF and Image Trace presets, pie graph options and others added since the catalog was completed; #1461); the entry count in the table is the 2026-10-11 measurement; native review still pending; scores and estimates unchanged |
)
| 2026-10-12 | minor | Korean (`ko`) catalog added, complete under the tests (menus, literals, messages); 12-language score 54% → 63%, hours 70–110 → 60–100 |
| 2026-10-12 | minor | Polish (`pl`) catalog added, complete under the tests (menus, literals, messages; one/few/many plurals) |
| 2026-10-12 | minor | Hungarian (`hu`) catalog added, complete under the tests |
| 2026-10-11 | minor | Czech render-site follow-up: file-dialog filters, diagnostics, preset numbering, contributor tooltips/plurals, gradient grammar and contextual printer mark styles; estimates unchanged |
| 2026-10-10 | major | First per-language table, measured from the catalog row counts against the complete French catalog |
