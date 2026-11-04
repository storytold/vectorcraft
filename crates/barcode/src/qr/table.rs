//! The QR Code tables of ISO/IEC 18004, for Model 2 symbols of versions 1 to 40.
//!
//! Everything here is a table the standard prints: how many codewords a symbol of each version
//! carries, how those codewords are split between data and error correction at each of the four
//! levels, how many blocks they are cut into, and where the alignment patterns fall.

/// The codewords a symbol of each version carries, data and error correction together
/// (ISO/IEC 18004 Table 1). The count is the number of free modules divided by eight; a symbol
/// whose free module count is not a multiple of eight leaves the remainder as the "remainder
/// bits" the encoder writes as zeros.
pub const TOTAL_CODEWORDS: [u16; 40] = [
    26, 44, 70, 100, 134, 172, 196, 242, 292, 346, 404, 466, 532, 581, 655, 733, 815, 901, 991, 1085, 1156, 1258, 1364, 1474, 1588, 1706, 1828, 1921,
    2051, 2185, 2323, 2465, 2611, 2761, 2876, 3034, 3196, 3362, 3532, 3706,
];

/// The error correction codewords a symbol of each version carries at each level, in the order
/// L, M, Q, H (ISO/IEC 18004 Table 9).
pub const ECC_CODEWORDS: [[u16; 4]; 40] = [
    [7, 10, 13, 17],
    [10, 16, 22, 28],
    [15, 26, 36, 44],
    [20, 36, 52, 64],
    [26, 48, 72, 88],
    [36, 64, 96, 112],
    [40, 72, 108, 130],
    [48, 88, 132, 156],
    [60, 110, 160, 192],
    [72, 130, 192, 224],
    [80, 150, 224, 264],
    [96, 176, 260, 308],
    [104, 198, 288, 352],
    [120, 216, 320, 384],
    [132, 240, 360, 432],
    [144, 280, 408, 480],
    [168, 308, 448, 532],
    [180, 338, 504, 588],
    [196, 364, 546, 650],
    [224, 416, 600, 700],
    [224, 442, 644, 750],
    [252, 476, 690, 816],
    [270, 504, 750, 900],
    [300, 560, 810, 960],
    [312, 588, 870, 1050],
    [336, 644, 952, 1110],
    [360, 700, 1020, 1200],
    [390, 728, 1050, 1260],
    [420, 784, 1140, 1350],
    [450, 812, 1200, 1440],
    [480, 868, 1290, 1530],
    [510, 924, 1350, 1620],
    [540, 980, 1440, 1710],
    [570, 1036, 1530, 1800],
    [570, 1064, 1590, 1890],
    [600, 1120, 1680, 1980],
    [630, 1204, 1770, 2100],
    [660, 1260, 1860, 2220],
    [720, 1316, 1950, 2310],
    [750, 1372, 2040, 2430],
];

/// How a symbol's data codewords are cut into blocks, at each level in the order L, M, Q, H:
/// `(blocks in the first group, blocks in the second)` (ISO/IEC 18004 Table 9).
///
/// The two groups carry codewords one apart — the first group `d` each and the second `d + 1` —
/// which is how the standard keeps every block the same length once the error correction
/// codewords are added.
pub const BLOCKS: [[(u8, u8); 4]; 40] = [
    [(1, 0), (1, 0), (1, 0), (1, 0)],
    [(1, 0), (1, 0), (1, 0), (1, 0)],
    [(1, 0), (1, 0), (2, 0), (2, 0)],
    [(1, 0), (2, 0), (2, 0), (4, 0)],
    [(1, 0), (2, 0), (2, 2), (2, 2)],
    [(2, 0), (4, 0), (4, 0), (4, 0)],
    [(2, 0), (4, 0), (2, 4), (4, 1)],
    [(2, 0), (2, 2), (4, 2), (4, 2)],
    [(2, 0), (3, 2), (4, 4), (4, 4)],
    [(2, 2), (4, 1), (6, 2), (6, 2)],
    [(4, 0), (1, 4), (4, 4), (3, 8)],
    [(2, 2), (6, 2), (4, 6), (7, 4)],
    [(4, 0), (8, 1), (8, 4), (12, 4)],
    [(3, 1), (4, 5), (11, 5), (11, 5)],
    [(5, 1), (5, 5), (5, 7), (11, 7)],
    [(5, 1), (7, 3), (15, 2), (3, 13)],
    [(1, 5), (10, 1), (1, 15), (2, 17)],
    [(5, 1), (9, 4), (17, 1), (2, 19)],
    [(3, 4), (3, 11), (17, 4), (9, 16)],
    [(3, 5), (3, 13), (15, 5), (15, 10)],
    [(4, 4), (17, 0), (17, 6), (19, 6)],
    [(2, 7), (17, 0), (7, 16), (34, 0)],
    [(4, 5), (4, 14), (11, 14), (16, 14)],
    [(6, 4), (6, 14), (11, 16), (30, 2)],
    [(8, 4), (8, 13), (7, 22), (22, 13)],
    [(10, 2), (19, 4), (28, 6), (33, 4)],
    [(8, 4), (22, 3), (8, 26), (12, 28)],
    [(3, 10), (3, 23), (4, 31), (11, 31)],
    [(7, 7), (21, 7), (1, 37), (19, 26)],
    [(5, 10), (19, 10), (15, 25), (23, 25)],
    [(13, 3), (2, 29), (42, 1), (23, 28)],
    [(17, 0), (10, 23), (10, 35), (19, 35)],
    [(17, 1), (14, 21), (29, 19), (11, 46)],
    [(13, 6), (14, 23), (44, 7), (59, 1)],
    [(12, 7), (12, 26), (39, 14), (22, 41)],
    [(6, 14), (6, 34), (46, 10), (2, 64)],
    [(17, 4), (29, 14), (49, 10), (24, 46)],
    [(4, 18), (13, 32), (48, 14), (42, 32)],
    [(20, 4), (40, 7), (43, 22), (10, 67)],
    [(19, 6), (18, 31), (34, 34), (20, 61)],
];

/// The free modules a symbol of each version has left over once its codewords are laid out:
/// zero for version 1, seven for versions 2 to 6, and from version 7 the small remainders the
/// version information and the alignment patterns leave (ISO/IEC 18004 Table 1).
#[cfg(test)]
pub const REMAINDER_BITS: [u8; 40] =
    [0, 7, 7, 7, 7, 7, 0, 0, 0, 0, 0, 0, 0, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 3, 3, 3, 3, 3, 3, 3, 0, 0, 0, 0, 0, 0];

/// Where a symbol's alignment patterns are centred, along both axes (ISO/IEC 18004 Annex E).
///
/// The first is always module 6 and the last is seven in from the far edge; versions 2 to 6 have
/// two, and each later version adds coordinates so that the patterns sit on a square grid.
pub const ALIGNMENT: [&[u8]; 40] = [
    &[],
    &[6, 18],
    &[6, 22],
    &[6, 26],
    &[6, 30],
    &[6, 34],
    &[6, 22, 38],
    &[6, 24, 42],
    &[6, 26, 46],
    &[6, 28, 50],
    &[6, 30, 54],
    &[6, 32, 58],
    &[6, 34, 62],
    &[6, 26, 46, 66],
    &[6, 26, 48, 70],
    &[6, 26, 50, 74],
    &[6, 30, 54, 78],
    &[6, 30, 56, 82],
    &[6, 30, 58, 86],
    &[6, 34, 62, 90],
    &[6, 28, 50, 72, 94],
    &[6, 26, 50, 74, 98],
    &[6, 30, 54, 78, 102],
    &[6, 28, 54, 80, 106],
    &[6, 32, 58, 84, 110],
    &[6, 30, 58, 86, 114],
    &[6, 34, 62, 90, 118],
    &[6, 26, 50, 74, 98, 122],
    &[6, 30, 54, 78, 102, 126],
    &[6, 26, 52, 78, 104, 130],
    &[6, 30, 56, 82, 108, 134],
    &[6, 34, 60, 86, 112, 138],
    &[6, 30, 58, 86, 114, 142],
    &[6, 34, 62, 90, 118, 146],
    &[6, 30, 54, 78, 102, 126, 150],
    &[6, 24, 50, 76, 102, 128, 154],
    &[6, 28, 54, 80, 106, 132, 158],
    &[6, 32, 58, 84, 110, 136, 162],
    &[6, 26, 54, 82, 110, 138, 166],
    &[6, 30, 58, 86, 114, 142, 170],
];

/// The characters the alphanumeric mode carries, in value order (ISO/IEC 18004 Table 5).
pub const ALPHANUMERIC: &[u8; 45] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:";

/// The eight masks of ISO/IEC 18004 §8.8.1, as the module condition that decides which modules
/// are inverted.
pub fn mask(mask: u8, row: usize, column: usize) -> bool {
    let (i, j) = (row as u32, column as u32);
    match mask {
        0 => (i + j) % 2 == 0,
        1 => i % 2 == 0,
        2 => j % 3 == 0,
        3 => (i + j) % 3 == 0,
        4 => (i / 2 + j / 3) % 2 == 0,
        5 => (i * j) % 2 + (i * j) % 3 == 0,
        6 => ((i * j) % 2 + (i * j) % 3) % 2 == 0,
        7 => ((i + j) % 2 + (i * j) % 3) % 2 == 0,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every version's data codewords come out whole, which is the check that the error
    /// correction total and the block counts belong together.
    #[test]
    fn the_tables_agree_with_each_other() {
        for v in 0..40usize {
            let total = TOTAL_CODEWORDS[v];
            for level in 0..4usize {
                let (b1, b2) = BLOCKS[v][level];
                let blocks = u16::from(b1) + u16::from(b2);
                assert!(blocks > 0, "version {} level {level}", v + 1);
                let ecc = ECC_CODEWORDS[v][level];
                assert_eq!(ecc % blocks, 0, "version {} level {level}: {ecc} ec over {blocks} blocks", v + 1);
                let data = total - ecc;
                assert!(data >= u16::from(b2), "version {} level {level}", v + 1);
                let per_block = (data - u16::from(b2)) / blocks;
                assert!(per_block > 0, "version {} level {level}", v + 1);
                assert_eq!(per_block * blocks + u16::from(b2), data, "version {} level {level}", v + 1);
            }
        }
    }

    /// A symbol has exactly the codewords the standard gives it, and exactly the remainder bits
    /// it leaves free, once its function patterns have taken their modules: this is the check
    /// that the finder, timing, alignment, format and version patterns are laid out where
    /// ISO/IEC 18004 puts them.
    #[test]
    fn the_symbol_carries_the_codewords_the_standard_gives_it() {
        for (v, total) in TOTAL_CODEWORDS.iter().enumerate() {
            let version = (v + 1) as u8;
            let size = 4 * usize::from(version) + 17;
            let free = size * size - super::super::function_modules(version);
            assert_eq!(free / 8, usize::from(*total), "version {version} carries {free} free modules");
            assert_eq!(free % 8, usize::from(REMAINDER_BITS[v]), "version {version} remainder bits");
        }
    }
}
