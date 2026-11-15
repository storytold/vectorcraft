//! A `<feConvolveMatrix>` whose `order` overflows usvg's `order_x * order_y` u32 multiply must be
//! refused before usvg sees it: usvg 0.48 parses the attribute through f64 and casts it with
//! `as i32`, so a large value like `1066570220609` saturates to i32::MAX and the multiply
//! overflows (a debug or overflow-checks build panics). A release build wraps instead, and with a
//! one-entry `kernelMatrix` the wrapped product passes usvg's length check, leaving a filter that
//! claims a 2147483647×2147483647 kernel. Everything else is usvg's pass-through and must keep
//! importing, however large the order is on its own.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vectorcraft_svg::import;

/// An overflowing `order` with a `kernelMatrix` that parses (usvg `parser/filter.rs`).
#[test]
fn convolve_matrix_overflowing_order_with_kernel_is_refused() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><filter id="f"><feConvolveMatrix kernelMatrix="1 2 3 4" order="1066570220609 17"/></filter><rect width="4" height="4" filter="url(#f)"/></svg>"#;
    let r = import(svg);
    assert!(r.is_err(), "an order that overflows the multiply must be refused, not imported");
}

/// The release-wrap case: a one-entry `kernelMatrix`, whose wrapped `order_x * order_y` product
/// (2147483647² mod 2³² = 1) passes usvg's length check.
#[test]
fn convolve_matrix_overflowing_order_with_wrapping_kernel_is_refused() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><filter id="f"><feConvolveMatrix order="2147483647" kernelMatrix="1"/></filter><rect width="4" height="4" filter="url(#f)"/></svg>"#;
    let r = import(svg);
    assert!(r.is_err(), "an overflowing order must be refused even when a kernel would wrap past the length check");
}

/// An `order` whose second side fails to parse defaults it to the first (svgtypes rejects
/// out-of-range numbers like `-1e999`), so this is huge on both sides and refused too.
#[test]
fn convolve_matrix_overflowing_order_with_defaulted_second_side_is_refused() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><filter id="f"><feConvolveMatrix kernelMatrix="-60173 1e-999 1e999 0" order="946183596582 -1e999" divisor="-1e999"/></filter><rect width="4" height="4" filter="url(#f)"/></svg>"#;
    let r = import(svg);
    assert!(r.is_err(), "an order that overflows the multiply must be refused, not imported");
}

/// An order usvg can multiply keeps importing, however large on its own: only the overflowing
/// product is refused.
#[test]
fn convolve_matrix_large_but_fitting_order_still_imports() {
    for order in ["100", "80 80", "65535 65535", "2147483647 1"] {
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><filter id="f"><feConvolveMatrix order="{order}"/></filter><rect width="4" height="4" filter="url(#f)"/></svg>"#
        );
        assert!(import(&svg).is_ok(), "order={order} fits the u32 multiply and must keep importing");
    }
}

/// usvg falls back to 3 when an order doesn't parse or isn't positive: those keep importing.
#[test]
fn convolve_matrix_nonpositive_or_unparseable_order_still_imports() {
    let kernel = r#"kernelMatrix="0 0 0 0 1 0 0 0 0""#;
    for order in ["0", "-3", "-1e999", "nan"] {
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><filter id="f"><feConvolveMatrix order="{order}" {kernel}/></filter><rect width="4" height="4" filter="url(#f)"/></svg>"#
        );
        assert!(import(&svg).is_ok(), "order={order} is usvg's pass-through and must keep importing");
    }
}

/// A `kernelMatrix` that doesn't parse or doesn't match the order leaves the filter empty, which
/// usvg drops: the file keeps importing (the filter is drawn without, as a warning).
#[test]
fn convolve_matrix_unparseable_or_mismatched_kernel_still_imports() {
    let mismatched = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><filter id="f"><feConvolveMatrix order="3" kernelMatrix="0 0 0 0 1 0 0 0"/></filter><rect width="4" height="4" filter="url(#f)"/></svg>"#;
    assert!(import(mismatched).is_ok(), "a kernel of the wrong length is usvg's pass-through");
    let unparseable = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><filter id="f"><feConvolveMatrix order="3" kernelMatrix="1e999 0 0 0 0 0 0 0 0"/></filter><rect width="4" height="4" filter="url(#f)"/></svg>"#;
    assert!(import(unparseable).is_ok(), "an unparseable kernel is usvg's pass-through");
}

/// Sane kernels keep importing: 3×3, 2×2, and a defaulted order.
#[test]
fn sane_convolve_matrices_still_import() {
    let three = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><filter id="f"><feConvolveMatrix order="3" kernelMatrix="0 0 0 0 1 0 0 0 0"/></filter><rect width="4" height="4" filter="url(#f)"/></svg>"#;
    assert!(import(three).is_ok(), "an ordinary filter must still import");
    let two = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><filter id="f"><feConvolveMatrix order="2" kernelMatrix="0 1 1 0"/></filter><rect width="4" height="4" filter="url(#f)"/></svg>"#;
    assert!(import(two).is_ok(), "a small kernel must still import");
    let defaulted = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><filter id="f"><feConvolveMatrix kernelMatrix="0 0 0 0 1 0 0 0 0"/></filter><rect width="4" height="4" filter="url(#f)"/></svg>"#;
    assert!(import(defaulted).is_ok(), "a filter without order must still import");
}
