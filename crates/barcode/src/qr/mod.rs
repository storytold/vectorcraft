//! QR Code, Model 2 (ISO/IEC 18004).
//!
//! The encoder picks the smallest version the data fits in, one of the four error correction
//! levels, and — for each of the eight masks — builds the whole symbol, scores it by the
//! standard's four penalty rules, and keeps the least penalised. Nothing is optimised by segment:
//! the whole payload goes in one mode, the best of numeric, alphanumeric or byte, which is what
//! a data field holding a product number, a URL or a serial number needs.

mod gf;
mod table;

use serde::{Deserialize, Serialize};

use crate::qr::table::{ALIGNMENT, ALPHANUMERIC, BLOCKS, ECC_CODEWORDS, TOTAL_CODEWORDS};
use crate::{BarcodeSpec, Error, Matrix, Result, Symbology};

/// The four error correction levels of ISO/IEC 18004: how much of the symbol is given over to
/// repair, from about 7% of the codewords at Low to about 30% at High.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Ecc {
    Low,
    Medium,
    Quartile,
    High,
}

impl Ecc {
    pub const fn all() -> [Ecc; 4] {
        [Ecc::Low, Ecc::Medium, Ecc::Quartile, Ecc::High]
    }

    /// The level's name, as the panel shows it.
    pub const fn label(self) -> &'static str {
        match self {
            Ecc::Low => "Low",
            Ecc::Medium => "Medium",
            Ecc::Quartile => "Quartile",
            Ecc::High => "High",
        }
    }

    /// The level's index into the tables of [`table`], in the order they are printed.
    const fn index(self) -> usize {
        match self {
            Ecc::Low => 0,
            Ecc::Medium => 1,
            Ecc::Quartile => 2,
            Ecc::High => 3,
        }
    }

    /// The two bits the format information carries for this level (ISO/IEC 18004 Table 12).
    const fn bits(self) -> u8 {
        match self {
            Ecc::Low => 0b01,
            Ecc::Medium => 0b00,
            Ecc::Quartile => 0b11,
            Ecc::High => 0b10,
        }
    }
}

/// How the payload is encoded: one of the three modes a single-segment symbol can use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Numeric,
    Alphanumeric,
    Byte,
}

impl Mode {
    /// The four-bit mode indicator of ISO/IEC 18004 Table 2.
    fn indicator(self) -> u8 {
        match self {
            Mode::Numeric => 0b0001,
            Mode::Alphanumeric => 0b0010,
            Mode::Byte => 0b0100,
        }
    }

    /// How many bits the character count takes at this version (Table 3).
    fn count_bits(self, version: u8) -> usize {
        let group = match version {
            1..=9 => 0,
            10..=26 => 1,
            _ => 2,
        };
        match self {
            Mode::Numeric => [10, 12, 14][group],
            Mode::Alphanumeric => [9, 11, 13][group],
            Mode::Byte => [8, 16, 16][group],
        }
    }

    /// The mode that can carry `data`, preferring the one that packs most tightly.
    fn best(data: &str) -> Mode {
        if data.chars().all(|c| c.is_ascii_digit()) {
            Mode::Numeric
        } else if data.bytes().all(|b| ALPHANUMERIC.contains(&b)) {
            Mode::Alphanumeric
        } else {
            Mode::Byte
        }
    }

    /// The number of characters the count indicator says, which is the byte count in byte mode
    /// rather than the number of characters.
    fn count(self, data: &str) -> usize {
        match self {
            Mode::Byte => data.len(),
            _ => data.chars().count(),
        }
    }

    /// The bits the payload takes, without the mode or count indicators.
    fn payload_bits(self, data: &str) -> usize {
        match self {
            // Three digits to ten bits, two to seven, one to four.
            Mode::Numeric => {
                let n = data.chars().count();
                (n / 3) * 10 + [0, 4, 7][n % 3]
            }
            // Two characters to eleven bits, one to six.
            Mode::Alphanumeric => {
                let n = data.chars().count();
                (n / 2) * 11 + (n % 2) * 6
            }
            Mode::Byte => data.len() * 8,
        }
    }

    /// Write the payload into `out`.
    fn write(self, out: &mut Bits, data: &str) {
        match self {
            Mode::Numeric => {
                let digits: Vec<u32> = data.chars().filter_map(|c| c.to_digit(10)).collect();
                for group in digits.chunks(3) {
                    match group.len() {
                        3 => out.push(group[0] * 100 + group[1] * 10 + group[2], 10),
                        2 => out.push(group[0] * 10 + group[1], 7),
                        _ => out.push(group[0], 4),
                    }
                }
            }
            Mode::Alphanumeric => {
                let values: Vec<u32> = data.bytes().map(|b| ALPHANUMERIC.iter().position(|c| *c == b).map_or(0, |i| i as u32)).collect();
                for pair in values.chunks(2) {
                    match pair {
                        [a, b] => out.push(a * 45 + b, 11),
                        [a] => out.push(*a, 6),
                        _ => {}
                    }
                }
            }
            Mode::Byte => {
                for b in data.bytes() {
                    out.push(u32::from(b), 8);
                }
            }
        }
    }
}

/// A bit stream, most significant bit first.
#[derive(Default)]
struct Bits {
    bits: Vec<bool>,
}

impl Bits {
    fn push(&mut self, value: u32, width: usize) {
        for i in (0..width).rev() {
            self.bits.push((value >> i) & 1 == 1);
        }
    }

    fn len(&self) -> usize {
        self.bits.len()
    }

    /// Pad the stream out to `capacity` bits: a terminator of up to four zeros, zeros to the next
    /// byte, then the two alternating pad codewords of ISO/IEC 18004 §8.4.9.
    fn pad_to(&mut self, capacity: usize) {
        let terminator = (capacity - self.len()).min(4);
        self.push(0, terminator);
        let to_byte = (8 - self.len() % 8) % 8;
        self.push(0, to_byte);
        let mut pad = 0usize;
        while self.len() + 8 <= capacity {
            self.push(if pad.is_multiple_of(2) { 0xEC } else { 0x11 }, 8);
            pad += 1;
        }
    }

    /// The stream as codewords, which is what it is once it has been padded to a whole number of
    /// bytes.
    fn bytes(&self) -> Vec<u8> {
        self.bits.chunks(8).map(|chunk| chunk.iter().fold(0u8, |acc, bit| (acc << 1) | u8::from(*bit))).collect()
    }
}

/// The data codewords a symbol of `version` carries at `ecc` (ISO/IEC 18004 Table 7).
pub fn data_codewords(version: u8, ecc: Ecc) -> usize {
    let v = usize::from(version).saturating_sub(1);
    let total = TOTAL_CODEWORDS.get(v).copied().unwrap_or(0);
    let ec = ECC_CODEWORDS.get(v).and_then(|row| row.get(ecc.index())).copied().unwrap_or(0);
    usize::from(total.saturating_sub(ec))
}

/// Encode a QR Code.
pub fn encode(spec: &BarcodeSpec, data: &str) -> Result<Matrix> {
    let mode = Mode::best(data);
    let count = mode.count(data);
    if count == 0 {
        return Err(Error::Data("the code is empty".into()));
    }

    // The smallest version the payload fits in, or the one the caller pinned.
    let wanted = spec.version;
    let mut chosen = None;
    for version in wanted.map_or(1, |v| v.clamp(1, 40))..=wanted.map_or(40, |v| v.clamp(1, 40)) {
        let capacity = data_codewords(version, spec.ecc) * 8;
        let needed = 4 + mode.count_bits(version) + mode.payload_bits(data);
        if needed <= capacity {
            chosen = Some((version, capacity, needed));
            break;
        }
    }
    let Some((version, capacity, _)) = chosen else {
        return Err(Error::Data(match wanted {
            Some(v) => format!("{count} characters do not fit a version {v} symbol at {} error correction", spec.ecc.label()),
            None => {
                let room = data_codewords(40, spec.ecc) * 8;
                format!("the data needs more than a version 40 QR Code at {} error correction ({room} bits)", spec.ecc.label())
            }
        }));
    };

    let mut bits = Bits::default();
    bits.push(u32::from(mode.indicator()), 4);
    bits.push(count as u32, mode.count_bits(version));
    mode.write(&mut bits, data);
    bits.pad_to(capacity);
    let data_codewords = bits.bytes();

    let codewords = interleave(&data_codewords, version, spec.ecc);
    let payload = payload_bits(&codewords, version);

    // Build the symbol once per mask and keep the one the penalty rules like best. The format
    // information is written afterwards: ISO/IEC 18004 §7.8 has the masks judged on the symbol
    // before it, so that the choice does not depend on bits that only record it.
    let size = symbol_size(version);
    let build = |mask: u8| {
        let mut matrix = Matrix2d::new(size, version);
        matrix.place(&payload);
        matrix.apply_mask(mask);
        matrix
    };
    let mask = match spec.mask.map(|m| m % 8) {
        Some(pinned) => pinned,
        None => {
            let mut best: Option<(u32, u8)> = None;
            for candidate in 0..8u8 {
                let score = penalty(&build(candidate).dark, size);
                if best.as_ref().is_none_or(|(s, _)| score < *s) {
                    best = Some((score, candidate));
                }
            }
            best.map(|(_, m)| m).unwrap_or(0)
        }
    };
    let mut winner = build(mask);
    winner.write_format(spec.ecc, mask);
    let dark = winner.dark;

    let quiet = 4 + spec.quiet_extra.clamp(0.0, 100.0).round() as usize;
    Ok(Matrix { symbology: Symbology::QrCode, size, quiet, module_mm: spec.resolved_module_mm(), dark, version, ecc: spec.ecc })
}

/// The side of a symbol of `version` in modules.
fn symbol_size(version: u8) -> usize {
    4 * usize::from(version) + 17
}

/// Split the data codewords into blocks, add each block's error correction codewords, and
/// interleave the two runs (ISO/IEC 18004 §8.6).
fn interleave(data: &[u8], version: u8, ecc: Ecc) -> Vec<u8> {
    let v = usize::from(version).saturating_sub(1);
    let (b1, b2) = BLOCKS.get(v).and_then(|row| row.get(ecc.index())).copied().unwrap_or((1, 0));
    let (b1, b2) = (usize::from(b1), usize::from(b2));
    let blocks = b1 + b2;
    if blocks == 0 {
        return data.to_vec();
    }
    let ec_total = usize::from(ECC_CODEWORDS.get(v).and_then(|row| row.get(ecc.index())).copied().unwrap_or(0));
    let ec_per_block = ec_total / blocks;
    let total = usize::from(TOTAL_CODEWORDS.get(v).copied().unwrap_or(0));
    let data_total = total.saturating_sub(ec_total);
    // The first group is one codeword shorter than the second, which is how the standard makes
    // every block the same length once the error correction is appended.
    let short = (data_total.saturating_sub(b2)) / blocks;

    let mut data_blocks: Vec<Vec<u8>> = Vec::with_capacity(blocks);
    let mut ec_blocks: Vec<Vec<u8>> = Vec::with_capacity(blocks);
    let mut at = 0usize;
    for block in 0..blocks {
        let len = short + usize::from(block >= b1);
        let slice: Vec<u8> = data.get(at..at + len).unwrap_or_default().to_vec();
        at += len;
        ec_blocks.push(gf::codewords(&slice, ec_per_block));
        data_blocks.push(slice);
    }

    let mut out = Vec::with_capacity(total);
    let longest = data_blocks.iter().map(Vec::len).max().unwrap_or(0);
    for i in 0..longest {
        for block in &data_blocks {
            if let Some(byte) = block.get(i) {
                out.push(*byte);
            }
        }
    }
    for i in 0..ec_per_block {
        for block in &ec_blocks {
            if let Some(byte) = block.get(i) {
                out.push(*byte);
            }
        }
    }
    out
}

/// The codewords as a bit stream, followed by the remainder bits the symbol leaves free as zeros.
fn payload_bits(codewords: &[u8], version: u8) -> Vec<bool> {
    let size = symbol_size(version);
    let free = size * size - function_modules(version);
    let mut bits: Vec<bool> = Vec::with_capacity(free);
    for byte in codewords {
        for i in (0..8).rev() {
            bits.push((byte >> i) & 1 == 1);
        }
    }
    bits.resize(free, false);
    bits
}

/// How many modules a symbol spends on everything but data, which is the count the standard's
/// codeword totals are built on.
pub(crate) fn function_modules(version: u8) -> usize {
    let matrix = Matrix2d::new(symbol_size(version), version);
    matrix.reserved.iter().filter(|r| **r).count()
}

/// The symbol being built: which modules are dark, and which are spoken for by a function
/// pattern rather than by the data.
struct Matrix2d {
    size: usize,
    dark: Vec<bool>,
    reserved: Vec<bool>,
}

impl Matrix2d {
    fn new(size: usize, version: u8) -> Self {
        let mut m = Self { size, dark: vec![false; size * size], reserved: vec![false; size * size] };
        m.draw_function_patterns(version);
        m
    }

    fn set(&mut self, row: usize, column: usize, dark: bool) {
        if row < self.size && column < self.size {
            self.dark[row * self.size + column] = dark;
            self.reserved[row * self.size + column] = true;
        }
    }

    fn draw_function_patterns(&mut self, version: u8) {
        let size = self.size;
        // The three finder patterns, each with its separator folded into the ring of light
        // modules around it.
        for &(r0, c0) in &[(0usize, 0usize), (0, size - 7), (size - 7, 0)] {
            for dr in 0..7 {
                for dc in 0..7 {
                    let edge = dr == 0 || dr == 6 || dc == 0 || dc == 6;
                    let core = (2..=4).contains(&dr) && (2..=4).contains(&dc);
                    self.set(r0 + dr, c0 + dc, edge || core);
                }
            }
        }
        for i in 0..8 {
            for (r, c) in [(7usize, i), (i, 7), (7, size - 1 - i), (i, size - 8), (size - 8, i), (size - 1 - i, 7)] {
                self.set(r, c, false);
            }
        }
        // The timing patterns.
        for i in 8..size.saturating_sub(8) {
            let dark = i % 2 == 0;
            self.set(6, i, dark);
            self.set(i, 6, dark);
        }
        // The alignment patterns, except the three that would land on a finder.
        if version > 1 {
            let coordinates = ALIGNMENT.get(usize::from(version) - 1).copied().unwrap_or(&[]);
            for &r in coordinates {
                for &c in coordinates {
                    let (r, c) = (usize::from(r), usize::from(c));
                    // The three that would land on a finder pattern are left out.
                    let (far_right, far_bottom) = (c + 10 > size, r + 10 > size);
                    let on_a_finder = (r < 9 && (c < 9 || far_right)) || (far_bottom && c < 9);
                    if on_a_finder {
                        continue;
                    }
                    for dr in 0..5 {
                        for dc in 0..5 {
                            let ring = dr == 0 || dr == 4 || dc == 0 || dc == 4;
                            self.set(r - 2 + dr, c - 2 + dc, ring || (dr == 2 && dc == 2));
                        }
                    }
                }
            }
        }
        // The format information's two copies, module by module (ISO/IEC 18004 §8.9): the first
        // runs along row 8 past the finder, up round the corner and up column 8, and the second
        // runs up column 8 from the bottom edge and along row 8 to the right edge. The module
        // where the vertical timing pattern crosses is not part of either.
        for i in 0..=5 {
            self.set(8, i, false);
        }
        self.set(8, 7, false);
        self.set(8, 8, false);
        self.set(7, 8, false);
        for i in 9..15 {
            self.set(14 - i, 8, false);
        }
        for i in 0..7 {
            self.set(size - 1 - i, 8, false);
        }
        for i in 0..8 {
            self.set(8, size - 8 + i, false);
        }
        self.set(size - 8, 8, true);
        // The version information, from version 7: eighteen bits in two blocks, the top-right
        // one six rows by three columns and the bottom-left one the other way round, both clear
        // of the finder patterns and their separators (ISO/IEC 18004 §8.10).
        if version >= 7 {
            let bits = version_info(version);
            for i in 0..18usize {
                let dark = (bits >> i) & 1 == 1;
                let (a, b) = (i / 3, i % 3);
                self.set(a, size - 11 + b, dark);
                self.set(size - 11 + b, a, dark);
            }
        }
    }

    /// Lay the data bits into the two-module-wide columns, right to left, alternating up and down
    /// and skipping the vertical timing pattern (ISO/IEC 18004 §8.7.3).
    fn place(&mut self, bits: &[bool]) {
        let size = self.size;
        let mut at = 0usize;
        let mut up = true;
        let mut right = size as isize - 1;
        while right > 0 {
            if right == 6 {
                right -= 1;
            }
            for step in 0..size {
                let row = if up { size - 1 - step } else { step };
                for column in [right, right - 1] {
                    let column = column.max(0) as usize;
                    let index = row * size + column;
                    if self.reserved.get(index).copied().unwrap_or(true) {
                        continue;
                    }
                    self.dark[index] = bits.get(at).copied().unwrap_or(false);
                    at += 1;
                }
            }
            up = !up;
            right -= 2;
        }
    }

    fn apply_mask(&mut self, mask: u8) {
        for row in 0..self.size {
            for column in 0..self.size {
                if table::mask(mask, row, column) && !self.reserved[row * self.size + column] {
                    self.dark[row * self.size + column] = !self.dark[row * self.size + column];
                }
            }
        }
    }

    /// Write the format information for `ecc` and `mask`, twice: once around the top-left finder
    /// and once split between the other two (ISO/IEC 18004 §8.9).
    fn write_format(&mut self, ecc: Ecc, mask: u8) {
        let size = self.size;
        let format = format_info(ecc.bits(), mask);
        // `bit(i)` is the i-th most significant of the fifteen, which is the order the modules
        // take: ISO/IEC 18004 §8.9 puts the most significant bit at (8, 0).
        let bit = |i: usize| (format >> (14 - i)) & 1 == 1;
        // The first copy: along row 8 past the finder, up round the corner, and up column 8 —
        // skipping the module where the vertical timing pattern crosses.
        for i in 0..6 {
            self.set(8, i, bit(i));
        }
        self.set(8, 7, bit(6));
        self.set(8, 8, bit(7));
        self.set(7, 8, bit(8));
        for i in 9..15 {
            self.set(14 - i, 8, bit(i));
        }
        // The second copy runs the other way round: bit 0 along row 8 from the right edge, then
        // bit 14 up column 8 from below, so that the two copies are mirror images.
        let lowest = |i: usize| (format >> i) & 1 == 1;
        for i in 0..8 {
            self.set(8, size - 1 - i, lowest(i));
        }
        for i in 8..15 {
            self.set(size - 15 + i, 8, lowest(i));
        }
    }
}

/// The fifteen-bit format information: the two error correction bits and the three mask bits,
/// extended with a BCH(15, 5) check and masked with 0x5412 (ISO/IEC 18004 §8.9).
fn format_info(ecc: u8, mask: u8) -> u16 {
    let data = (u32::from(ecc & 0b11) << 3) | u32::from(mask & 0b111);
    let mut value = data << 10;
    let generator = 0x537u32;
    for i in (10..15).rev() {
        if value & (1 << i) != 0 {
            value ^= generator << (i - 10);
        }
    }
    (((data << 10) | value) as u16) ^ 0x5412
}

/// The eighteen-bit version information of a symbol from version 7: the six version bits and a
/// BCH(18, 6) check (ISO/IEC 18004 §8.10).
fn version_info(version: u8) -> u32 {
    let data = u32::from(version) & 0x3F;
    let mut value = data << 12;
    let generator = 0x1F25u32;
    for i in (12..18).rev() {
        if value & (1 << i) != 0 {
            value ^= generator << (i - 12);
        }
    }
    (data << 12) | value
}

/// The four penalty rules of ISO/IEC 18004 §8.8.2, summed: the score a mask is judged by.
fn penalty(dark: &[bool], size: usize) -> u32 {
    let at = |row: usize, column: usize| dark.get(row * size + column).copied().unwrap_or(false);
    let mut score = 0u32;

    // Rule 1: runs of five or more modules of one colour, in a row or a column.
    for line in 0..size {
        for horizontal in [true, false] {
            let mut run = 1usize;
            let mut previous = if horizontal { at(line, 0) } else { at(0, line) };
            for i in 1..size {
                let current = if horizontal { at(line, i) } else { at(i, line) };
                if current == previous {
                    run += 1;
                } else {
                    if run >= 5 {
                        score += 3 + (run as u32 - 5);
                    }
                    run = 1;
                    previous = current;
                }
            }
            if run >= 5 {
                score += 3 + (run as u32 - 5);
            }
        }
    }

    // Rule 2: every two-by-two block of one colour.
    for row in 0..size.saturating_sub(1) {
        for column in 0..size.saturating_sub(1) {
            let first = at(row, column);
            if first == at(row, column + 1) && first == at(row + 1, column) && first == at(row + 1, column + 1) {
                score += 3;
            }
        }
    }

    // Rule 3: the finder-like pattern 1011101 with four light modules before or after it. The
    // area beyond the symbol's edge is its quiet zone, so a pattern running up to the edge counts
    // as one that has its four light modules.
    const PATTERN: [bool; 7] = [true, false, true, true, true, false, true];
    let light = |line: &[bool], from: usize, to: usize| line.get(from..to).is_none_or(|s| s.iter().all(|b| !*b));
    let mut line: Vec<bool> = Vec::with_capacity(size);
    for index in 0..size {
        for horizontal in [true, false] {
            line.clear();
            for i in 0..size {
                line.push(if horizontal { at(index, i) } else { at(i, index) });
            }
            for start in 0..size.saturating_sub(6) {
                if line.get(start..start + 7) != Some(&PATTERN[..]) {
                    continue;
                }
                let after = start + 7;
                if light(&line, start.saturating_sub(4), start) || light(&line, after, after + 4) {
                    score += 40;
                }
            }
        }
    }

    // Rule 4: how far the symbol is from half dark, in steps of five per cent.
    let total = (size * size).max(1);
    let dark_count = dark.iter().filter(|d| **d).count();
    let percent = dark_count as f64 * 100.0 / total as f64;
    let steps = ((percent - 50.0).abs() / 5.0) as u32;
    score + steps * 10
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mode_picks_the_tightest_that_fits() {
        assert_eq!(Mode::best("12345678"), Mode::Numeric);
        assert_eq!(Mode::best("HELLO WORLD $%*+-./:"), Mode::Alphanumeric);
        assert_eq!(Mode::best("hello"), Mode::Byte);
        assert_eq!(Mode::best("https://example.com/a?b=1"), Mode::Byte);
    }

    /// The mode indicators and character counts of ISO/IEC 18004 Tables 2 and 3.
    #[test]
    fn the_indicators_and_counts_are_the_standard_ones() {
        assert_eq!(Mode::Numeric.indicator(), 0b0001);
        assert_eq!(Mode::Alphanumeric.indicator(), 0b0010);
        assert_eq!(Mode::Byte.indicator(), 0b0100);
        assert_eq!(Mode::Numeric.count_bits(1), 10);
        assert_eq!(Mode::Numeric.count_bits(10), 12);
        assert_eq!(Mode::Numeric.count_bits(27), 14);
        assert_eq!(Mode::Alphanumeric.count_bits(9), 9);
        assert_eq!(Mode::Byte.count_bits(1), 8);
        assert_eq!(Mode::Byte.count_bits(10), 16);
    }

    /// The two worked format strings ISO/IEC 18004 §8.9 prints: level M with mask 0 is
    /// 101010000010010, and level M with mask 1 is 101000100100101.
    #[test]
    fn the_format_information_is_the_standard_ones() {
        let bits = |v: u16| (0..15).map(|i| if (v >> i) & 1 == 1 { '1' } else { '0' }).rev().collect::<String>();
        assert_eq!(bits(format_info(Ecc::Medium.bits(), 0)), "101010000010010");
        assert_eq!(bits(format_info(Ecc::Medium.bits(), 1)), "101000100100101");
        assert_eq!(bits(format_info(Ecc::Low.bits(), 7)), "110100101110110");
        assert_eq!(bits(format_info(Ecc::High.bits(), 0)), "001011010001001");
    }

    /// The version information of ISO/IEC 18004 Annex D: version 7 is 000111110010010100.
    #[test]
    fn the_version_information_is_the_standard_one() {
        let bits = |v: u32| (0..18).map(|i| if (v >> i) & 1 == 1 { '1' } else { '0' }).rev().collect::<String>();
        assert_eq!(bits(version_info(7)), "000111110010010100");
        assert_eq!(bits(version_info(8)), "001000010110111100");
        assert_eq!(bits(version_info(40)), "101000110001101001");
    }

    #[test]
    fn a_symbol_has_the_size_its_version_says() {
        for version in 1..=40u8 {
            let spec = BarcodeSpec { version: Some(version), data: "X".into(), ..BarcodeSpec::default() };
            let Ok(Matrix { size, version: v, .. }) = encode(&spec, "X") else { panic!("version {version}") };
            assert_eq!(size, 4 * usize::from(version) + 17);
            assert_eq!(v, version);
        }
    }
}
