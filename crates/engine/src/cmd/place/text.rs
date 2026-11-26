//! Placing a plain-text file (Text Import Options): its bytes decoded by character set and
//! platform, extra returns and spaces cleaned up, the text set as area type.

use serde_json::Value;

use super::super::{text_param, utf16_order_without_mark};
use super::{Point, Rect, Result, bad, fileio, shapes, switch_or};

/// Text files as `file.place` reports them (not a format `document.open` reads).
pub(super) const FORMAT: fileio::Format = fileio::Format {
    id: "txt",
    label: "Text",
    extensions: fileio::TEXT_EXTS,
    mime: "text/plain",
    read: true,
    write: false,
    raster: false,
    options: &[],
};

/// The most bytes a text or RTF file placed as type may have, and the most bytes of text a Word
/// document may give.
pub(super) const MAX_TEXT: usize = 4 << 20;

/// A character set of Text Import Options (`text.characterSet`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum CharacterSet {
    /// UTF-8, or UTF-16 with or without a byte-order mark ([`decode`]).
    #[default]
    Unicode,
    /// The platform's 8-bit character set: Windows-1252, or Mac Roman on a Mac.
    Ansi,
    ShiftJis,
    Gb2312,
    ChineseBig5,
    Cyrillic,
    Gb18030,
    Greek,
    Turkish,
    Baltic,
    CentralEuropean,
}

/// Each character set by its `text.characterSet` value.
const CHARACTER_SETS: [(&str, CharacterSet); 11] = [
    ("unicode", CharacterSet::Unicode),
    ("ansi", CharacterSet::Ansi),
    ("shiftJis", CharacterSet::ShiftJis),
    ("gb2312", CharacterSet::Gb2312),
    ("chineseBig5", CharacterSet::ChineseBig5),
    ("cyrillic", CharacterSet::Cyrillic),
    ("gb18030", CharacterSet::Gb18030),
    ("greek", CharacterSet::Greek),
    ("turkish", CharacterSet::Turkish),
    ("baltic", CharacterSet::Baltic),
    ("centralEuropean", CharacterSet::CentralEuropean),
];

impl CharacterSet {
    /// The Encoding Standard decoder of a set other than Unicode and ANSI, for a file from a Mac
    /// when `mac`: GB2312 reads as GBK, Cyrillic as windows-1251 (x-mac-cyrillic on a Mac), and
    /// Greek, Turkish, Baltic and Central European as windows-1253, 1254, 1257 and 1250 on both
    /// platforms.
    fn encoding(self, mac: bool) -> Option<&'static encoding_rs::Encoding> {
        Some(match self {
            Self::Unicode | Self::Ansi => return None,
            Self::ShiftJis => encoding_rs::SHIFT_JIS,
            Self::Gb2312 => encoding_rs::GBK,
            Self::ChineseBig5 => encoding_rs::BIG5,
            Self::Cyrillic if mac => encoding_rs::X_MAC_CYRILLIC,
            Self::Cyrillic => encoding_rs::WINDOWS_1251,
            Self::Gb18030 => encoding_rs::GB18030,
            Self::Greek => encoding_rs::WINDOWS_1253,
            Self::Turkish => encoding_rs::WINDOWS_1254,
            Self::Baltic => encoding_rs::WINDOWS_1257,
            Self::CentralEuropean => encoding_rs::WINDOWS_1250,
        })
    }
}

/// Text Import Options (`file.place`'s `text` param).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct TextOptions {
    /// The character set the file's bytes are decoded as.
    character_set: CharacterSet,
    /// The file comes from a Mac (its ANSI set is Mac Roman, its Cyrillic set x-mac-cyrillic).
    mac: bool,
    /// Join the lines of each block of text (blank lines end paragraphs).
    remove_line_returns: bool,
    /// Drop the blank lines between paragraphs.
    remove_paragraph_returns: bool,
    /// Replace runs of this many spaces or more with a tab (2 or more).
    spaces_to_tab: Option<usize>,
}

impl TextOptions {
    /// The options in `p.text` for command `cmd`.
    pub(super) fn parse(p: &Value, cmd: &str) -> Result<Self> {
        let Some(t) = p.get("text") else { return Ok(Self::default()) };
        if !t.is_object() {
            return Err(bad(cmd, "text must be {characterSet?, platform?, removeLineReturns?, removeParagraphReturns?, replaceSpaces?}"));
        }
        // The index of `text[key]` in `values`; 0 when it is omitted or null.
        let choice = |key: &str, values: &[&str]| -> Result<usize> {
            let Some(v) = text_param(t, key, cmd)? else { return Ok(0) };
            values.iter().position(|x| *x == v).ok_or_else(|| {
                let (last, rest) = values.split_last().map_or(("", &[][..]), |(last, rest)| (*last, rest));
                bad(cmd, format!("text.{key} `{v}`: use \"{}\" or \"{}\"", rest.join("\", \""), last))
            })
        };
        let spaces_to_tab = match t.get("replaceSpaces") {
            None | Some(Value::Null) => None,
            Some(v) => Some(
                v.as_u64().filter(|n| (2..=100).contains(n)).ok_or_else(|| bad(cmd, "text.replaceSpaces must be a whole number from 2 to 100"))?
                    as usize,
            ),
        };
        let set = choice("characterSet", &CHARACTER_SETS.map(|(name, _)| name))?;
        Ok(Self {
            character_set: CHARACTER_SETS.get(set).map_or(CharacterSet::Unicode, |(_, s)| *s),
            mac: choice("platform", &["windows", "mac"])? == 1,
            remove_line_returns: switch_or(t, "removeLineReturns", false, cmd)?,
            remove_paragraph_returns: switch_or(t, "removeParagraphReturns", false, cmd)?,
            spaces_to_tab,
        })
    }
}

/// The text of a text file's `bytes`, decoded and cleaned up as `o` says: paragraphs separated by
/// `\n`.
pub(super) fn import(bytes: &[u8], o: TextOptions, cmd: &str) -> Result<String> {
    if bytes.len() > MAX_TEXT {
        return Err(bad(cmd, format!("the text file is too large to place as type (at most {} MB)", MAX_TEXT >> 20)));
    }
    Ok(clean(&decode(bytes, o), o))
}

/// `bytes` as text: Unicode (UTF-8, or UTF-16 with a byte-order mark, or without one when the
/// file has an even length, reads as valid UTF-16 with no NUL character in the byte order
/// [`utf16_order_without_mark`] gives; bytes that aren't UTF-8 are read as the platform's 8-bit
/// set), the platform's 8-bit character set, or another set's decoder
/// ([`CharacterSet::encoding`]), which reads a byte-order mark as ordinary bytes and a malformed
/// sequence as U+FFFD.
fn decode(bytes: &[u8], o: TextOptions) -> String {
    if let Some(encoding) = o.character_set.encoding(o.mac) {
        return encoding.decode_without_bom_handling(bytes).0.into_owned();
    }
    let eight_bit = |b: &[u8]| -> String { b.iter().map(|&c| if o.mac { mac_roman(c) } else { windows_1252(c) }).collect() };
    if o.character_set == CharacterSet::Ansi {
        return eight_bit(bytes);
    }
    let utf16 = |rest: &[u8], le: bool| {
        let units = rest.as_chunks::<2>().0.iter().map(|&c| if le { u16::from_le_bytes(c) } else { u16::from_be_bytes(c) });
        char::decode_utf16(units).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)).collect()
    };
    match bytes {
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        [0xFF, 0xFE, rest @ ..] => utf16(rest, true),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, false),
        _ => {
            let without_mark = (bytes.len().is_multiple_of(2) && bytes.contains(&0))
                .then(|| utf16_order_without_mark(bytes))
                .flatten()
                .and_then(|le| {
                    let units = bytes.as_chunks::<2>().0.iter().map(|&c| if le { u16::from_le_bytes(c) } else { u16::from_be_bytes(c) });
                    char::decode_utf16(units).collect::<std::result::Result<String, _>>().ok()
                })
                .filter(|text| !text.contains('\0'));
            without_mark.unwrap_or_else(|| std::str::from_utf8(bytes).map_or_else(|_| eight_bit(bytes), str::to_string))
        }
    }
}

/// Byte `b` of Windows-1252.
pub(in crate::cmd) fn windows_1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}', //
        '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
    ];
    match b {
        0x80..=0x9F => HIGH.get(usize::from(b - 0x80)).copied().unwrap_or(char::REPLACEMENT_CHARACTER),
        _ => char::from(b),
    }
}

/// Byte `b` of Mac Roman.
pub(in crate::cmd) fn mac_roman(b: u8) -> char {
    craft_codepage::windows_byte(craft_codepage::MAC_ROMAN_CODE_PAGE, b)
}

/// `text` with its line breaks (CR LF, CR or LF) as paragraph breaks, cleaned up as `o` says:
/// with `remove_line_returns` each block of lines is one paragraph (a blank line ends it, and stays
/// as an empty paragraph unless `remove_paragraph_returns`); with only `remove_paragraph_returns`
/// each line is a paragraph and blank lines go. Other control characters than tabs are dropped,
/// and so are blank lines at the end.
fn clean(text: &str, o: TextOptions) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let text: String = text.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).collect();
    let blank = |l: &str| l.trim().is_empty();
    let mut paras: Vec<String> = vec![];
    if o.remove_line_returns {
        let mut block: Option<String> = None;
        for line in text.split('\n') {
            if blank(line) {
                paras.extend(block.take());
                if !o.remove_paragraph_returns {
                    paras.push(String::new());
                }
                continue;
            }
            match &mut block {
                Some(b) => {
                    b.push(' ');
                    b.push_str(line.trim());
                }
                None => block = Some(line.trim_end().to_string()),
            }
        }
        paras.extend(block);
    } else {
        paras = text.split('\n').filter(|l| !(o.remove_paragraph_returns && blank(l))).map(str::to_string).collect();
    }
    while paras.last().is_some_and(|p| blank(p)) {
        paras.pop();
    }
    let out = paras.join("\n");
    match o.spaces_to_tab {
        Some(n) => tabs_for_spaces(&out, n),
        None => out,
    }
}

/// `s` with each run of `n` or more spaces replaced by a tab.
fn tabs_for_spaces(s: &str, n: usize) -> String {
    let mut out = String::with_capacity(s.len());
    let mut run = 0;
    let flush = |out: &mut String, run: usize| {
        if run >= n {
            out.push('\t');
        } else {
            out.extend(std::iter::repeat_n(' ', run));
        }
    };
    for c in s.chars() {
        if c == ' ' {
            run += 1;
            continue;
        }
        flush(&mut out, run);
        run = 0;
        out.push(c);
    }
    flush(&mut out, run);
    out
}

/// The text placed type gets from a text file, an RTF file or a Word document.
pub(super) struct Story {
    /// The text in runs of one character formatting; paragraphs are separated by `\n`.
    pub(super) runs: Vec<vectorcraft_doc::TextRun>,
    /// Each paragraph's attributes; empty: the defaults.
    pub(super) paras: Vec<vectorcraft_doc::ParaStyle>,
}

impl Story {
    /// `text` in the default character and paragraph attributes.
    pub(super) fn plain(text: String) -> Self {
        Self { runs: vec![vectorcraft_doc::TextRun::new(text, vectorcraft_doc::CharStyle::default())], paras: vec![] }
    }

    /// The text without its formatting.
    pub(super) fn plain_text(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }
}

/// The area type of `story` filling `frame` (at the origin; placing moves it), its superscripts,
/// subscripts and small caps in the proportions of `setup` (Document Setup).
pub(super) fn area_text(story: Story, frame: Rect, setup: &vectorcraft_doc::DocSetup) -> vectorcraft_doc::TextObject {
    let mut t = vectorcraft_doc::TextObject::point(Point::ORIGIN, "", vectorcraft_doc::CharStyle::default());
    if !story.runs.is_empty() {
        t.runs = story.runs;
    }
    for run in &mut t.runs {
        run.style.position = crate::cmd::docsetup::position_in(setup, run.style.position);
        if run.style.small_caps.is_some() {
            run.style.small_caps = Some(setup.small_caps_size);
        }
    }
    if !story.paras.is_empty() {
        t.set_paragraph_styles(story.paras);
    }
    t.kind = vectorcraft_doc::TextKind::Area { frame: shapes::rectangle(Rect::from_origin_size(Point::ORIGIN, frame.size())) };
    t.system_layout = setup.system_layout;
    crate::cmd::typecmd::refresh_bounds(&mut t);
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(lines: bool, paras: bool) -> TextOptions {
        TextOptions { remove_line_returns: lines, remove_paragraph_returns: paras, ..TextOptions::default() }
    }

    #[test]
    fn crlf_with_returns_removed_gives_one_paragraph_per_block() {
        let text = "First line\r\nof one block\r\n\r\nSecond block\r\nends here\r\n\r\n\r\nThird\r\n";
        assert_eq!(clean(text, opts(true, true)), "First line of one block\nSecond block ends here\nThird");
        assert_eq!(clean(text, opts(true, false)), "First line of one block\n\nSecond block ends here\n\n\nThird");
        assert_eq!(clean(text, opts(false, true)), "First line\nof one block\nSecond block\nends here\nThird");
        assert_eq!(clean(text, opts(false, false)), "First line\nof one block\n\nSecond block\nends here\n\n\nThird");
        assert_eq!(clean("Old\rMac\r\rText", opts(true, true)), "Old Mac\nText", "CR alone breaks lines too");
    }

    #[test]
    fn runs_of_spaces_become_tabs_and_controls_go() {
        let o = TextOptions { spaces_to_tab: Some(3), ..TextOptions::default() };
        assert_eq!(clean("a  b   c     d\u{0}e\tf", o), "a  b\tc\tde\tf");
    }

    #[test]
    fn decodes_by_character_set_and_platform() {
        let unicode = TextOptions::default();
        assert_eq!(decode("café".as_bytes(), unicode), "café");
        assert_eq!(decode(&[0xEF, 0xBB, 0xBF, b'h', b'i'], unicode), "hi", "UTF-8 byte-order mark");
        assert_eq!(decode(&[0xFF, 0xFE, b'h', 0, 0xE9, 0], unicode), "hé", "UTF-16 LE");
        assert_eq!(decode(&[0xFE, 0xFF, 0, b'h', 0x20, 0xAC], unicode), "h€", "UTF-16 BE");
        assert_eq!(decode(&[b'c', b'a', b'f', 0xE9, 0x80], unicode), "café€", "not UTF-8: Windows-1252");
        let ansi = TextOptions { character_set: CharacterSet::Ansi, ..TextOptions::default() };
        assert_eq!(decode(&[0x93, b'q', 0x94, 0xE9], ansi), "“q”é");
        let mac = TextOptions { character_set: CharacterSet::Ansi, mac: true, ..TextOptions::default() };
        assert_eq!(decode(&[0xD2, b'q', 0xD3, 0x8E, 0xCA, 0xA5, 0xDB, 0xDE, 0xF0], mac), "“q”é\u{a0}•€ﬁ\u{f8ff}");
    }

    /// The other character sets decode by their Encoding Standard decoders: the platform matters
    /// for Cyrillic alone, a byte-order mark is read as ordinary bytes, and a malformed sequence
    /// reads as U+FFFD.
    #[test]
    fn decodes_the_other_character_sets() {
        let cases: [(CharacterSet, &[u8], &str); 9] = [
            (CharacterSet::ShiftJis, &[0x93, 0xFA, 0x96, 0x7B], "日本"),
            (CharacterSet::Gb2312, &[0xD6, 0xD0, 0xCE, 0xC4], "中文"),
            (CharacterSet::ChineseBig5, &[0xA4, 0xA4, 0xA4, 0xE5], "中文"),
            (CharacterSet::Cyrillic, &[0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2], "Привет"),
            (CharacterSet::Gb18030, &[0x95, 0x32, 0x82, 0x36], "\u{20000}"),
            (CharacterSet::Greek, &[0xC5, 0xEB, 0xEB, 0xDC, 0xE4, 0xE1], "Ελλάδα"),
            (CharacterSet::Turkish, &[0xDD, 0x73, 0x74], "İst"),
            (CharacterSet::Baltic, &[0x52, 0xEE, 0x67, 0x61], "Rīga"),
            (CharacterSet::CentralEuropean, &[0xA3, 0xF3, 0x64, 0x9F], "Łódź"),
        ];
        for (set, bytes, text) in cases {
            let windows = TextOptions { character_set: set, ..TextOptions::default() };
            assert_eq!(decode(bytes, windows), text, "{set:?}");
            if set != CharacterSet::Cyrillic {
                assert_eq!(decode(bytes, TextOptions { mac: true, ..windows }), text, "{set:?} from a Mac");
            }
        }
        let mac_cyrillic = TextOptions { character_set: CharacterSet::Cyrillic, mac: true, ..TextOptions::default() };
        assert_eq!(decode(&[0x8F, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2], mac_cyrillic), "Привет");
        let shift_jis = TextOptions { character_set: CharacterSet::ShiftJis, ..TextOptions::default() };
        assert_eq!(decode(&[0x82, 0x22], shift_jis), "\u{FFFD}\"", "a lead byte before ASCII");
        assert_eq!(decode(&[b'a', 0x82], shift_jis), "a\u{FFFD}", "a lead byte at the end");
        let central = TextOptions { character_set: CharacterSet::CentralEuropean, ..TextOptions::default() };
        assert_eq!(decode(&[0xEF, 0xBB, 0xBF, b'A'], central), "ď»żA", "a UTF-8 byte-order mark is read as windows-1250");
        assert_eq!(decode(&[0xFF, 0xFE, b'A', 0], shift_jis), "\u{FFFD}\u{FFFD}A\0", "a UTF-16 byte-order mark is read as Shift JIS");
    }

    #[test]
    fn options_parse_and_reject_bad_values() {
        let p = serde_json::json!({"text": {"characterSet": "ansi", "platform": "mac", "removeLineReturns": true, "replaceSpaces": 4}});
        let o = TextOptions::parse(&p, "file.place").unwrap();
        assert_eq!(
            o,
            TextOptions {
                character_set: CharacterSet::Ansi,
                mac: true,
                remove_line_returns: true,
                remove_paragraph_returns: false,
                spaces_to_tab: Some(4)
            }
        );
        for (name, set) in CHARACTER_SETS {
            let o = TextOptions::parse(&serde_json::json!({"text": {"characterSet": name}}), "file.place").unwrap();
            assert_eq!(o.character_set, set, "{name}");
        }
        assert_eq!(TextOptions::parse(&serde_json::json!({}), "file.place").unwrap(), TextOptions::default());
        for bad in [
            serde_json::json!({"text": 3}),
            serde_json::json!({"text": {"platform": "amiga"}}),
            serde_json::json!({"text": {"replaceSpaces": 1}}),
            serde_json::json!({"text": {"characterSet": 3}}),
            serde_json::json!({"text": {"platform": true}}),
            serde_json::json!({"text": {"removeLineReturns": "yes"}}),
            serde_json::json!({"text": {"removeParagraphReturns": 1}}),
        ] {
            assert!(TextOptions::parse(&bad, "file.place").is_err(), "{bad}");
        }
        let null = serde_json::json!({"text": {"characterSet": null, "platform": null, "removeLineReturns": null}});
        assert_eq!(TextOptions::parse(&null, "file.place").unwrap(), TextOptions::default(), "null leaves the default");
    }
}
