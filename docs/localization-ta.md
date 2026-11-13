# Tamil interface terminology and verification

The `ta` catalog (`crates/ui-egui/src/i18n/ta.tsv`, interface name **தமிழ்**) has 8,842 rows:
3,022 `@msg`, 47 `@msgplural` and 41 `@plural`. It covers every row of the Hindi catalog, the
newest complete one, plus the strings the untranslated report listed beyond it, so the report lists no
Tamil gaps.

## Provenance

- 5,194 rows reuse this project's Tamil in [craft-i18n](https://github.com/storytold/craft-i18n):
  `locales/ta/vectorcraft.tsv` (4,125 rows) and `locales/ta/common.tsv` (924), written by
  @newsbaba24x7 with AI assistance and partly reviewed by a native speaker, plus 145 rows whose
  English the other Tamil Craft catalogs already translate.
- The other 3,296 rows (mostly status and error messages) are AI-assisted translations from
  the plain meaning of the English, clean room, using VectorCraft's source to tell what each
  string means on screen. No other product's localisation or string table was consulted.
- Native-speaker review of those rows is pending; coverage tests alone do not establish
  translation quality.

## Style

- Commands and buttons use the plain imperative (சேமி, திற, நீக்கு); option labels and headings
  are noun phrases; messages addressed to the user are polite Tamil.
- Product and format names, file extensions, keyboard keys, units, digits (0–9), placeholders,
  command ids, parameter names and enum values stay exactly as in English.
- Hint-bar fragments keep the English leading and trailing spaces, since the hint bar assembles
  sentences from separately translated pieces.
- The built-in workspace and perspective preset names stay in English, as in the other catalogs.
- Tamil counts use `one` for 1 and `other` for everything else (`plural_one_other`); `@plural`
  and `@msgplural` rows have two forms.

## Terminology

| English | Tamil |
|---|---|
| Artboard / Artboards | ஆர்ட்போர்டு / ஆர்ட்போர்டுகள் |
| Path / Anchor point | பாதை / நங்கூரப் புள்ளி |
| Fill / Stroke | நிரப்பு / கோடு |
| Gradient | சாய்வு |
| Symbol (reusable art) / symbol (character) | சின்னம் / குறியீடு |
| Layer / Layers | அடுக்கு / அடுக்குகள் |
| Selection | தேர்வு |
| Preview | முன்காட்சி |
| Toolbar | கருவிப் பட்டை |
| Spot colour | புள்ளி நிறம் |
| Mask | முகமூடி |
| Pattern / Patterns | வடிவமாதிரி / வடிவமாதிரிகள் (வடிவம் is Shape); Dash Pattern கீற்று வடிவமாதிரி |

## Fonts

- Desktop: Tamil falls back to an installed Tamil face, trying Nirmala UI, Tamil Sangam MN,
  Tamil MN, Noto Sans Tamil, Noto Sans Tamil UI, Lohit Tamil and Latha first
  (`crates/text/src/fontdb/names.rs`).
- Builds whose craft-fonts checkout has Noto Sans Tamil (craft-fonts#21; the revision the
  workflows pin predates it) add its `Taml` face at the end of every UI family.
- Web: Noto Sans Tamil is a `startup` face in `crates/text/web-fonts.txt`, fetched before the app
  starts; a craft-fonts checkout without it leaves it out with a warning. The revision the workflows
  pin predates it, so until the pin moves (a separate pull request) the web build has no Tamil font
  and Tamil text shows missing-glyph boxes there.

## Verification

- `tamil_resolves_locales_and_uses_one_other_plurals`: `ta`, `ta-IN`, `ta_IN.UTF-8`, `ta-LK`,
  `ta-SG` and `ta-Taml-IN` resolve to Tamil; 1 is singular, 0 and 2+ plural; every `@plural` and
  `@msgplural` row has two forms.
- `bundled_fonts_cover_the_catalogs` checks Tamil glyphs when craft-fonts has a `Taml` face.
- `tamil_prefers_fonts_made_for_it` checks the desktop Tamil font order.
