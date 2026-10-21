//! Placing a plain-text file (Text Import Options): its bytes decoded by character set and
//! platform, extra returns and spaces cleaned up, the text set as area type.

use serde_json::Value;

use super::*;

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

/// Text Import Options (`file.place`'s `text` param).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct TextOptions {
    /// Decode as the platform's 8-bit character set (Windows-1252 or Mac Roman) instead of
    /// Unicode.
    ansi: bool,
    /// The file comes from a Mac (its 8-bit character set is Mac Roman).
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
        let choice = |key: &str, values: [&str; 2]| match str_param(t, key) {
            None => Ok(false),
            Some(v) if v == values[0] => Ok(false),
            Some(v) if v == values[1] => Ok(true),
            Some(v) => Err(bad(cmd, format!("text.{key} `{v}`: use \"{}\" or \"{}\"", values[0], values[1]))),
        };
        let spaces_to_tab = match t.get("replaceSpaces") {
            None | Some(Value::Null) => None,
            Some(v) => Some(
                v.as_u64().filter(|n| (2..=100).contains(n)).ok_or_else(|| bad(cmd, "text.replaceSpaces must be a whole number from 2 to 100"))?
                    as usize,
            ),
        };
        Ok(Self {
            ansi: choice("characterSet", ["unicode", "ansi"])?,
            mac: choice("platform", ["windows", "mac"])?,
            remove_line_returns: bool_or(t, "removeLineReturns", false),
            remove_paragraph_returns: bool_or(t, "removeParagraphReturns", false),
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

/// `bytes` as text: Unicode (UTF-8, or UTF-16 with a byte-order mark; bytes that aren't UTF-8 are
/// read as the platform's 8-bit set), or the platform's 8-bit character set.
fn decode(bytes: &[u8], o: TextOptions) -> String {
    let eight_bit = |b: &[u8]| -> String { b.iter().map(|&c| if o.mac { mac_roman(c) } else { windows_1252(c) }).collect() };
    if o.ansi {
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
        _ => std::str::from_utf8(bytes).map_or_else(|_| eight_bit(bytes), str::to_string),
    }
}

/// Byte `b` of Windows-1252.
pub(super) fn windows_1252(b: u8) -> char {
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
pub(super) fn mac_roman(b: u8) -> char {
    const HIGH: [char; 128] = [
        'Ä', 'Å', 'Ç', 'É', 'Ñ', 'Ö', 'Ü', 'á', 'à', 'â', 'ä', 'ã', 'å', 'ç', 'é', 'è', //
        'ê', 'ë', 'í', 'ì', 'î', 'ï', 'ñ', 'ó', 'ò', 'ô', 'ö', 'õ', 'ú', 'ù', 'û', 'ü', //
        '†', '°', '¢', '£', '§', '•', '¶', 'ß', '®', '©', '™', '´', '¨', '≠', 'Æ', 'Ø', //
        '∞', '±', '≤', '≥', '¥', 'µ', '∂', '∑', '∏', 'π', '∫', 'ª', 'º', 'Ω', 'æ', 'ø', //
        '¿', '¡', '¬', '√', 'ƒ', '≈', '∆', '«', '»', '…', '\u{A0}', 'À', 'Ã', 'Õ', 'Œ', 'œ', //
        '–', '—', '“', '”', '‘', '’', '÷', '◊', 'ÿ', 'Ÿ', '⁄', '€', '‹', '›', 'ﬁ', 'ﬂ', //
        '‡', '·', '‚', '„', '‰', 'Â', 'Ê', 'Á', 'Ë', 'È', 'Í', 'Î', 'Ï', 'Ì', 'Ó', 'Ô', //
        '\u{F8FF}', 'Ò', 'Ú', 'Û', 'Ù', 'ı', 'ˆ', '˜', '¯', '˘', '˙', '˚', '¸', '˝', '˛', 'ˇ',
    ];
    match b {
        0x80.. => HIGH.get(usize::from(b - 0x80)).copied().unwrap_or(char::REPLACEMENT_CHARACTER),
        _ => char::from(b),
    }
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
        let ansi = TextOptions { ansi: true, ..TextOptions::default() };
        assert_eq!(decode(&[0x93, b'q', 0x94, 0xE9], ansi), "“q”é");
        let mac = TextOptions { ansi: true, mac: true, ..TextOptions::default() };
        assert_eq!(decode(&[0xD2, b'q', 0xD3, 0x8E, 0xA5, 0xDB], mac), "“q”é•€");
    }

    #[test]
    fn options_parse_and_reject_bad_values() {
        let p = serde_json::json!({"text": {"characterSet": "ansi", "platform": "mac", "removeLineReturns": true, "replaceSpaces": 4}});
        let o = TextOptions::parse(&p, "file.place").unwrap();
        assert_eq!(o, TextOptions { ansi: true, mac: true, remove_line_returns: true, remove_paragraph_returns: false, spaces_to_tab: Some(4) });
        assert_eq!(TextOptions::parse(&serde_json::json!({}), "file.place").unwrap(), TextOptions::default());
        for bad in
            [serde_json::json!({"text": 3}), serde_json::json!({"text": {"platform": "amiga"}}), serde_json::json!({"text": {"replaceSpaces": 1}})]
        {
            assert!(TextOptions::parse(&bad, "file.place").is_err(), "{bad}");
        }
    }
}
