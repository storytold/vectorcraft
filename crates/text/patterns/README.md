# Indic hyphenation patterns

The eleven `hyph-*.tex` files are unmodified upstream files from
[hyphenation/tex-hyphen](https://github.com/hyphenation/tex-hyphen/tree/5684c0f51c0b81133db2efbe60a408b4155a3ff5/hyph-utf8/tex/generic/hyph-utf8/patterns/tex),
pinned at commit `5684c0f51c0b81133db2efbe60a408b4155a3ff5`.

Hindi, Marathi, Gujarati, Bengali, Punjabi (Gurmukhi), Tamil, Telugu, Kannada,
Malayalam, Odia and Assamese: copyright (C) 2016 Santhosh Thottingal.
VectorCraft uses these under their **MIT** license option; the full copyright
and permission notice is preserved at the top of each file. The alternative
LGPL/GPL licenses are not selected.

Sanskrit remains selectable for shaping, but automatic hyphenation is pending
an allowlisted source (#4947). The hyph-utf8 Sanskrit file's custom grant is
outside the repository's license allowlist, as is the LPPL license of CTAN's
older `sanhyph` package. Neither file is bundled. No language silently falls
back to Hindi or English rules; explicit user exceptions remain available.

The matcher reads only the contents of the TeX `\patterns{…}` block after
removing comments. Odd maximum weights permit breaks and even weights forbid
them (Liang patterns). Paragraph length limits count Unicode scalar values,
as for the existing English implementation. Automatic points and preferences
exceptions are restricted to whole Unicode graphemes and actual shaping
cluster starts. Both composers consume the same points; their existing
Hyphen Limit and Hyphenation Zone behavior applies.

Breaks after viramas are conservatively suppressed, including Tamil pulli
breaks permitted by the upstream patterns and Unicode grapheme segmentation.
This preserves consonant clusters even when the font subdivides them.
