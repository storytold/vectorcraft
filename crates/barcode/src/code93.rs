//! Code 93 (AIM/ISO 93).
//!
//! Three bars and three spaces make nine modules a character; two of the four characters after
//! the data are check characters (a mod-47 C over the data and a mod-47 K over the data and C),
//! and the symbol closes with a stop character and a single terminating bar.
//!
//! Code 93 carries the whole of ASCII through four non-data shift characters, so — unlike
//! Code 39 — it needs no prefix convention: a shift character simply precedes the letter it
//! modifies.

use crate::linear::Linear;
use crate::{BarcodeSpec, Error, HriAlign, HriPart, Result, Symbol, finish_linear};

/// The 43 data characters in value order: a character's value is its index here, which is what
/// the two check characters are computed over.
const CHARS: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-. $/+%";

/// The nine modules of each character, one bit per module, most significant bit leftmost, as
/// width runs (bar, space, bar, space, bar, space) for the entries past the 43 data characters.
///
/// Index 43 to 46 are the four shift characters, 47 the start and stop `*`, and 48 the stop
/// again with the terminating bar appended.
const ENCS: [&str; 49] = [
    "131112", "111213", "111312", "111411", "121113", "121212", "121311", "111114", "131211", "141111", "211113", "211212", "211311", "221112",
    "221211", "231111", "112113", "112212", "112311", "122112", "132111", "111123", "111222", "111321", "121122", "131121", "212112", "212211",
    "211122", "211221", "221121", "222111", "112122", "112221", "122121", "123111", "121131", "311112", "311211", "321111", "112131", "113121",
    "211131", "121221", "312111", "311121", "122211", "111141", "1111411",
];

/// The four non-data characters the extended convention shifts with, one for each group of
/// ASCII characters. They are written as characters no caller can type, so that a symbol's own
/// data can never be mistaken for one.
const SHIFT_CONTROL: char = '\u{1}';
const SHIFT_PERCENT: char = '\u{2}';
const SHIFT_SLASH: char = '\u{3}';
const SHIFT_PLUS: char = '\u{4}';

/// The start and stop character's value in [`ENCS`].
const START_STOP: usize = 47;
const STOP_WITH_TERMINATOR: usize = 48;

/// The value of a data character, or None when Code 93's own set does not hold it (the shift
/// characters are added by [`extended`] and are not values here).
fn value(c: char) -> Option<usize> {
    CHARS.find(c)
}

/// The character at a value, for the two check characters.
fn char_at(v: usize) -> char {
    CHARS.chars().nth(v).unwrap_or('0')
}

/// The character a check value of 43 to 46 stands for: the four shift characters, which sit past
/// the data set in [`ENCS`] and so are not in [`CHARS`]. Pushing `'0'` for them — what `char_at`
/// falls back to — would make K cover a value the symbol does not carry.
fn check_char(v: usize) -> char {
    match v {
        43 => SHIFT_CONTROL,
        44 => SHIFT_PERCENT,
        45 => SHIFT_SLASH,
        46 => SHIFT_PLUS,
        _ => char_at(v),
    }
}

/// The Code 93 characters that carry `text`: the data set directly when the text stays inside it,
/// else the shift sequences of the Code 93 Extended convention.
///
/// The four shift characters are non-data, so the four ASCII characters that look like them
/// (`$`, `%`, `+` and `/`) are encoded as themselves — which is where Code 93 differs from
/// Code 39's Full ASCII convention, and why this table cannot be shared with it.
fn extended(text: &str, what: &str) -> Result<Vec<char>> {
    const SHIFTS: [char; 4] = [SHIFT_CONTROL, SHIFT_PERCENT, SHIFT_SLASH, SHIFT_PLUS];
    let mut out = Vec::new();
    for c in text.chars() {
        let n = c as u32;
        let s = |i: usize, letter: char| vec![SHIFTS[i], letter];
        let pair = match n {
            0 => vec![SHIFTS[1], 'U'],
            1..=26 => s(0, char::from_u32(n + 64).unwrap_or('A')),
            27..=31 => s(1, char::from_u32(n + 38).unwrap_or('A')),
            32..=35 => vec![c],
            36 => vec![c],
            37 => vec![c],
            38 => vec![c],
            39..=41 => vec![c],
            42 => vec![c],
            43 => vec![c],
            44 => vec![c],
            45..=47 => vec![c],
            48..=57 => vec![c],
            58 => s(2, 'Z'),
            59..=63 => s(1, char::from_u32(n - 16).unwrap_or('A')),
            64 => s(1, 'V'),
            65..=90 => vec![c],
            91..=95 => s(1, char::from_u32(n - 27).unwrap_or('A')),
            96 => s(1, 'W'),
            97..=122 => s(3, char::from_u32(n - 32).unwrap_or('A')),
            123..=127 => s(1, char::from_u32(n - 43).unwrap_or('A')),
            _ => return Err(Error::Data(format!("{what} cannot encode {}: it is outside ASCII", c.escape_debug()))),
        };
        out.extend(pair);
    }
    Ok(out)
}

/// Encode a Code 93 symbol.
pub fn encode(spec: &BarcodeSpec, data: &str) -> Result<Symbol> {
    let mut text = extended(data, "Code 93")?;
    let height = spec.resolved_height_mm();
    let mut l = Linear::new();
    write_character(&mut l, START_STOP, height);
    for &c in &text {
        write_character(&mut l, index_of(c)?, height);
    }
    // C over the data, then K over the data and C, both mod 47 with the weights running 1, 2, 3 …
    // from the right and cycling at 20 and 15 (AIM/ISO 93).
    let c = checksum(&text, 20, 0);
    text.push(check_char(c));
    let k = checksum(&text, 15, 0) % 47;
    write_character(&mut l, c, height);
    write_character(&mut l, k, height);
    write_character(&mut l, STOP_WITH_TERMINATOR, height);
    let modules = l.width_modules();
    Ok(finish_linear(spec, l, centred(data, modules)))
}

/// A character's index into [`ENCS`]: a shift character's own entry, else its value in the set.
fn index_of(c: char) -> Result<usize> {
    match c {
        SHIFT_CONTROL | SHIFT_PERCENT | SHIFT_SLASH | SHIFT_PLUS => Ok(code_value(c)),
        _ => value(c).ok_or_else(|| Error::Data(format!("Code 93 cannot encode {c:?}"))),
    }
}

/// A character's value for the check characters: its place in the data set, or 43 to 46 for the
/// four shift characters, which carry no data of their own. The start and stop `*` is 47 and takes
/// no part in either sum.
fn code_value(c: char) -> usize {
    match c {
        SHIFT_CONTROL => 43,
        SHIFT_PERCENT => 44,
        SHIFT_SLASH => 45,
        SHIFT_PLUS => 46,
        _ => value(c).unwrap_or(0),
    }
}

/// The mod-47 sum of `chars`' values with the weights running 1, 2, 3 … from the right and
/// cycling at `cycle`. `extra` is added before the modulus, which is how K folds in C.
fn checksum(chars: &[char], cycle: usize, extra: usize) -> usize {
    let n = chars.len();
    let sum: usize = chars.iter().enumerate().map(|(i, c)| code_value(*c) * ((n - i - 1) % cycle + 1)).sum();
    (sum + extra) % 47
}

/// Write one character's six elements.
fn write_character(l: &mut Linear, index: usize, height: f64) {
    let Some(runs) = ENCS.get(index) else { return };
    for (i, ch) in runs.chars().enumerate() {
        let width = ch.to_digit(10).map_or(1.0, f64::from);
        match i % 2 {
            0 => l.bar(width, 0.0, height),
            _ => l.gap(width),
        }
    }
}

/// The human readable line of a symbol whose digits run the width of the bar pattern.
fn centred(data: &str, modules: f64) -> Vec<HriPart> {
    vec![HriPart { text: data.to_string(), at_mm: modules / 2.0, align: HriAlign::Center, width_mm: modules, full_size: true }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modules(data: &str) -> String {
        crate::testutil::module_row(&crate::testutil::linear(crate::Symbology::Code93, data))
    }

    /// The start and stop characters, and the terminating bar.
    #[test]
    fn the_start_and_stop_are_the_standard_pattern() {
        assert_eq!(ENCS[START_STOP], "111141");
        assert_eq!(ENCS[STOP_WITH_TERMINATOR], "1111411");
        let row = modules("0");
        assert!(row.starts_with("101011110"), "{row}");
        assert!(row.ends_with("1010111101"), "{row}");
    }

    /// The value order the check characters are computed over, and the pair the worked example
    /// of the symbology specification gives.
    #[test]
    fn the_check_characters_are_mod_47() {
        assert_eq!(value('0'), Some(0));
        assert_eq!(value('Z'), Some(35));
        assert_eq!(value('-'), Some(36));
        assert_eq!(value('.'), Some(37));
        assert_eq!(value(' '), Some(38));
        assert_eq!(value('$'), Some(39));
        assert_eq!(value('/'), Some(40));
        assert_eq!(value('+'), Some(41));
        assert_eq!(value('%'), Some(42));
        // "CODE 93" checks to the characters at values 14 and 0, which the reference symbol
        // carries as the letters E and 0.
        let text: Vec<char> = "CODE 93".chars().collect();
        let c = checksum(&text, 20, 0);
        let mut with_c = text.clone();
        with_c.push(char_at(c));
        let k = checksum(&with_c, 15, 0) % 47;
        assert_eq!((c, k), (14, 0));
        assert_eq!(char_at(c), 'E');
        assert_eq!(char_at(k), '0');
    }

    #[test]
    fn the_symbol_is_the_width_the_standard_gives() {
        assert_eq!(modules("A").len(), 9 * 5 + 1);
        assert_eq!(modules("ABC123").len(), 9 * 10 + 1);
    }

    /// A lower-case letter, a colon and a control character each take one shift character.
    #[test]
    fn the_extended_set_reaches_the_whole_of_ascii() {
        assert_eq!(extended("a", "Code 93").map(|v| v.len()), Ok(2));
        assert_eq!(extended("A", "Code 93").map(|v| v.len()), Ok(1));
        assert_eq!(extended(":", "Code 93").map(|v| v.len()), Ok(2));
        assert_eq!(extended("\u{0}", "Code 93").map(|v| v.len()), Ok(2));
        // The four characters that look like shift characters are encoded as themselves.
        assert_eq!(extended("$%+/", "Code 93").map(|v| v.len()), Ok(4));
        assert!(extended("é", "Code 93").is_err());
    }

    /// The check characters run over the data and, for K, over C. A C of 43 to 46 is one of the
    /// four shift characters — which are not in the data set — and pushing `'0'` for it, what
    /// `char_at` falls back to, makes K cover a value the symbol does not carry.
    #[test]
    fn a_check_character_past_the_data_set_keeps_its_value() {
        assert_eq!(check_char(0), '0');
        assert_eq!(check_char(42), '%');
        assert_eq!((check_char(43), check_char(44), check_char(45), check_char(46)), (SHIFT_CONTROL, SHIFT_PERCENT, SHIFT_SLASH, SHIFT_PLUS));
        // A data value whose C is 43: its K is computed with the shift character's value.
        let text: Vec<char> = (0..5000)
            .map(|n| n.to_string())
            .find(|d| {
                let t: Vec<char> = d.chars().collect();
                checksum(&t, 20, 0) == 43
            })
            .expect("some short number gives C = 43")
            .chars()
            .collect();
        let mut with_c = text.clone();
        with_c.push(check_char(43));
        let mut wrongly = text.clone();
        wrongly.push(char_at(43));
        assert_ne!(checksum(&with_c, 15, 0) % 47, checksum(&wrongly, 15, 0) % 47, "K must see the shift's value (43), not '0'");
        let data: String = text.iter().collect();
        assert!(crate::encode(&BarcodeSpec::new(crate::Symbology::Code93, &data)).is_ok());
    }
}
