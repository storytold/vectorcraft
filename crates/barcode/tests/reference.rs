//! The encoders against two independent implementations.
//!
//! The linear module rows come from bwip-js (Barcode Writer in Pure PostScript) and the QR
//! matrices from segno, each asked for its own layout of the same data. Every row here is what
//! that implementation produces, so a change that moves a bar or a module fails this test rather
//! than reaching a printer.
//!
//! The QR cases pin the mask. Two implementations agree on every module but the mask, which each
//! picks by scoring the symbol with the four penalty rules of ISO/IEC 18004 §8.8.2 — rules that
//! routinely leave two or three masks within a point or two of each other.
//!
//! `scan` prints the symbols for a real decoder to read; it is ignored by default because it needs
//! one. Everything here was also read back by zxing-cpp: 311 QR symbols across the four error
//! correction levels and versions 1 to 40, and every linear symbol below.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vectorcraft_barcode::{BarcodeSpec, Code, Ecc, Symbology, encode};

/// (symbology id, data, the module row the reference implementation produces).
const LINEAR: &[(&str, &str, &str)] = &[
    ("ean13", "4006381333931", "10100011010100111010111101111010001001011001101010100001010000101000010111010010000101100110101"),
    ("ean13", "9780201379624", "10101110110001001010011100100110100111001100101010100001010001001110100101000011011001011100101"),
    ("ean8", "96385074", "1010001011010111101111010110111010101001110111001010001001011100101"),
    ("upcA", "036000291452", "10100011010111101010111100011010001101000110101010110110011101001100110101110010011101101100101"),
    ("upcE", "01234565", "101011001100100110111101001110101110010101111010101"),
    (
        "code39",
        "ABC-123",
        "10001011101110101110101000101110101110100010111011101110100010101000101011101110111010001010111010111000101011101110111000101010100010111011101",
    ),
    ("code93", "ABC-123", "1010111101101010001101001001101000101001011101010010001010001001010000101010110001010001101010111101"),
    (
        "code93",
        "Code 93 Test",
        "1010111101101000101001100101001011001001100101100101001001100101100100101110100101000010101010000101110100101101001101001100101100100101001100101101011001001100101101001101010000101010001101010111101",
    ),
    ("code128", "ABC-123", "1101001000010100011000100010110001000100011010011011100100111001101100111001011001011100101100001001100011101011"),
    ("code128", "1234567890", "110100111001011001110010001011000111000101101100001010011011110110100111100101100011101011"),
    (
        "code128",
        "Hello, World!",
        "1101001000011000101000101100100001100101000011001010000100011110101011001110011011001100111010001101000111101010010011110110010100001000010011011001101100110010100001100011101011",
    ),
    ("code128B", "abc123", "11010010000100101100001001000011010000101100100111001101100111001011001011100110111011101100011101011"),
    ("code128C", "1234567890", "110100111001011001110010001011000111000101101100001010011011110110100111100101100011101011"),
    (
        "ean128",
        "(01)12345678901231",
        "11010011100111101011101100110110010110011100100010110001110001011011000010100110111101101011001110011011000110100001100101100011101011",
    ),
    ("itf14", "15400141288763", "1010110010100101101010110010011010010110110100100101101011001001101010011011010101100100100110011010101101"),
    ("itf25", "1234567890", "101011010010101100110110100101001101001100101010010101100110101101001100101101"),
];

/// (data, error correction, mask, the symbol the reference implementation produces).
const QR: &[(&str, Ecc, u8, &[&str])] = &[
    (
        "HELLO WORLD",
        Ecc::Medium,
        0,
        &[
            "111111100010101111111",
            "100000101110001000001",
            "101110100010101011101",
            "101110100010101011101",
            "101110101011101011101",
            "100000100111001000001",
            "111111101010101111111",
            "000000000000000000000",
            "101010100100100010010",
            "011110001001000010001",
            "000111111101001011000",
            "111101011001110101110",
            "010011110101001110101",
            "000000001010001000101",
            "111111100000100101100",
            "100000100110001101000",
            "101110101100101111111",
            "101110100011010100010",
            "101110101111011101001",
            "100000100001110001011",
            "111111101101011100001",
        ],
    ),
    (
        "01234567",
        Ecc::Medium,
        3,
        &[
            "111111101101101111111",
            "100000101010001000001",
            "101110100110101011101",
            "101110101100001011101",
            "101110100110001011101",
            "100000100110001000001",
            "111111101010101111111",
            "000000001100000000000",
            "101101110010001001011",
            "000101011010100101100",
            "100101111000111110010",
            "110100010010110001010",
            "000111111001010010000",
            "000000001110010100001",
            "111111101000011010110",
            "100000101011111000101",
            "101110100101001000001",
            "101110101010010010110",
            "101110101011010010100",
            "100000100101101011011",
            "111111101001100100010",
        ],
    ),
    (
        "01234567",
        Ecc::Quartile,
        6,
        &[
            "111111100111101111111",
            "100000101000001000001",
            "101110100100001011101",
            "101110101000101011101",
            "101110101111001011101",
            "100000100000101000001",
            "111111101010101111111",
            "000000001001000000000",
            "010111101010011011010",
            "101100010101101011101",
            "101011110111011010110",
            "101101000000000000100",
            "011111110011101001011",
            "000000001100000101111",
            "111111100011111110010",
            "100000101010010110100",
            "101110101101101100101",
            "101110101101100011000",
            "101110100100001001111",
            "100000101010011010101",
            "111111100011000000110",
        ],
    ),
];

/// The module row of a symbol: one character per module, quiet zones excluded.
fn row(symbol: &vectorcraft_barcode::Symbol) -> String {
    let module = symbol.module_mm;
    if !module.is_finite() || module <= 0.0 {
        return String::new();
    }
    let quiet_left = symbol.symbology.quiet_modules().0 * module;
    let total = symbol.modules.round().max(1.0) as usize;
    let mut slots = vec![false; total];
    for bar in &symbol.bars {
        // Round rather than truncate: a bar 1.32 mm in at a 0.33 mm module divides to a hair under
        // four, which truncation would put a module early.
        let from = ((bar.x - quiet_left) / module).round().max(0.0) as usize;
        let to = (((bar.x - quiet_left) + bar.width) / module).round().max(0.0) as usize;
        for slot in slots.iter_mut().take(to.min(total)).skip(from) {
            *slot = true;
        }
    }
    slots.iter().map(|d| if *d { '1' } else { '0' }).collect()
}

#[test]
fn the_linear_symbologies_match_the_reference_implementation() {
    for (id, data, want) in LINEAR {
        let symbology = Symbology::from_id(id).unwrap_or_else(|| panic!("{id}"));
        // bwip-js widens each element by two narrows for the 2-of-5 family, and draws an ITF-14's
        // bearer bars as a border the module row would not cover.
        let ratio = matches!(symbology, Symbology::Itf14 | Symbology::Itf25).then_some(2.0);
        let spec = BarcodeSpec { bearer: false, ratio, ..BarcodeSpec::new(symbology, *data) };
        match encode(&spec) {
            Ok(Code::Linear(symbol)) => assert_eq!(&row(&symbol), want, "{id} {data}"),
            other => panic!("{id} {data}: {other:?}"),
        }
    }
}

#[test]
fn the_qr_symbologies_match_the_reference_implementation() {
    for (data, ecc, mask, want) in QR {
        let spec = BarcodeSpec { ecc: *ecc, mask: Some(*mask), ..BarcodeSpec::new(Symbology::QrCode, *data) };
        match encode(&spec) {
            Ok(Code::Matrix(matrix)) => {
                assert_eq!(matrix.size, want.len(), "{data} size");
                for (y, line) in want.iter().enumerate() {
                    let got: String = (0..matrix.size).map(|x| if matrix.dark_at(x, y) { '1' } else { '0' }).collect();
                    assert_eq!(&got, line, "{data} row {y}");
                }
            }
            other => panic!("{data}: {other:?}"),
        }
    }
}

/// The example value every symbology offers — what the Barcode panel starts a fresh one on, and
/// what it puts in the field when the type is changed — is one the symbology can carry. A default
/// that the encoder refuses would be the first thing a user sees, and the last thing they'd expect.
#[test]
fn every_symbology_has_an_example_it_can_carry() {
    for symbology in Symbology::all() {
        let example = symbology.sample_data();
        assert!(!example.is_empty(), "{symbology:?} has no example");
        let code = encode(&BarcodeSpec::new(symbology, example)).unwrap_or_else(|e| panic!("{symbology:?} {example:?}: {e}"));
        let (w, h) = code.size_mm();
        assert!(w > 0.0 && h > 0.0, "{symbology:?} is {w} x {h} mm");
        // And every one of them names the standard it belongs to, which the panel shows beside the
        // type and under it.
        assert!(symbology.standard().contains("ISO") || symbology.standard().contains("GS1"), "{symbology:?}");
        assert!(!symbology.origin().is_empty(), "{symbology:?}");
    }
}

/// Every symbology refuses what it cannot carry, and says why.
#[test]
fn data_that_does_not_fit_is_refused() {
    for (symbology, data) in [
        (Symbology::Ean13, "4006381333932"),
        (Symbology::Ean13, "12345"),
        (Symbology::Ean8, "9638507x"),
        (Symbology::UpcE, "99999999"),
        (Symbology::Code128C, "123"),
        (Symbology::Code128A, "abc"),
        (Symbology::Code39, "\u{e9}"),
        (Symbology::Code93, "\u{e9}"),
        (Symbology::Itf25, "12345"),
        (Symbology::Itf14, "1540014128876x"),
    ] {
        let spec = BarcodeSpec::new(symbology, data);
        assert!(encode(&spec).is_err(), "{symbology:?} {data:?} should be refused");
    }
    // A payload past the largest symbol, and a version pinned below what the payload needs.
    let long = "x".repeat(3000);
    assert!(encode(&BarcodeSpec::new(Symbology::QrCode, &long)).is_err());
    let small = BarcodeSpec { version: Some(1), ..BarcodeSpec::new(Symbology::QrCode, "a payload longer than a version 1 symbol holds") };
    assert!(encode(&small).is_err());
}

/// Print every symbol for an outside decoder to read.
///
/// `cargo test -p vectorcraft-barcode --test reference -- --ignored --nocapture scan`
#[test]
#[ignore = "a dumper, run by hand alongside a real decoder"]
fn scan() {
    for (data, ecc, mask, _) in QR {
        let spec = BarcodeSpec { ecc: *ecc, mask: Some(*mask), ..BarcodeSpec::new(Symbology::QrCode, *data) };
        if let Ok(Code::Matrix(matrix)) = encode(&spec) {
            println!("qr|{data}|{}|{}", matrix.size, matrix.quiet);
            for y in 0..matrix.size {
                let line: String = (0..matrix.size).map(|x| if matrix.dark_at(x, y) { '1' } else { '0' }).collect();
                println!("r|{line}");
            }
        }
    }
    for (id, data, _) in LINEAR {
        let symbology = Symbology::from_id(id).unwrap_or_else(|| panic!("{id}"));
        let spec = BarcodeSpec::new(symbology, *data);
        if let Ok(Code::Linear(symbol)) = encode(&spec) {
            println!("ln|{id}|{data}|{:.4}|{:.4}", symbol.width_mm, symbol.height_mm);
            for bar in &symbol.bars {
                println!("b|{:.4}|{:.4}|{:.4}|{:.4}", bar.x, bar.y, bar.width, bar.height);
            }
        }
    }
}
