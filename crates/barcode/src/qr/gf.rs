//! The arithmetic QR Code's error correction is built on: GF(2⁸) modulo the primitive polynomial
//! x⁸ + x⁴ + x³ + x² + 1 (0x11D), and the Reed–Solomon codes over it (ISO/IEC 18004 §8.5).

/// The field's non-zero elements as powers of the primitive element 2.
const EXP: [u8; 256] = build_exp();
/// The inverse of [`EXP`]: the power that gives each element, with 0 mapped to 0 and never read.
const LOG: [u8; 256] = build_log();

const fn build_exp() -> [u8; 256] {
    let mut table = [0u8; 256];
    let mut value: u16 = 1;
    let mut i = 0;
    while i < 255 {
        table[i] = value as u8;
        value <<= 1;
        if value & 0x100 != 0 {
            value ^= 0x11D;
        }
        i += 1;
    }
    // The 255th entry repeats the first, so that a product of two logs can be looked up without a
    // modulus when the sum is 254 or 510.
    table[255] = table[0];
    table
}

const fn build_log() -> [u8; 256] {
    let mut table = [0u8; 256];
    let exp = build_exp();
    let mut i = 0;
    while i < 255 {
        table[exp[i] as usize] = i as u8;
        i += 1;
    }
    table
}

/// The product of two field elements.
pub fn mul(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        return 0;
    }
    let sum = usize::from(LOG[a as usize]) + usize::from(LOG[b as usize]);
    EXP[sum % 255]
}

/// The generator polynomial for `n` error correction codewords: the product of (x − αⁱ) for i
/// from 0 to n − 1.
///
/// The result holds the coefficients of xⁿ⁻¹ down to x⁰; the leading coefficient is 1 and is not
/// kept, which is what the division below expects.
pub fn generator(n: usize) -> Vec<u8> {
    let mut g: Vec<u8> = vec![1];
    for i in 0..n {
        let root = EXP[i % 255];
        let mut next = vec![0u8; g.len() + 1];
        for (j, &coefficient) in g.iter().enumerate() {
            next[j] ^= coefficient;
            next[j + 1] ^= mul(coefficient, root);
        }
        g = next;
    }
    g.remove(0);
    g
}

/// The `n` error correction codewords of `data`: the remainder of the data, shifted up by n,
/// divided by the generator polynomial.
pub fn codewords(data: &[u8], n: usize) -> Vec<u8> {
    if n == 0 {
        return Vec::new();
    }
    let g = generator(n);
    let mut remainder = vec![0u8; n];
    for &byte in data {
        let factor = byte ^ remainder.first().copied().unwrap_or(0);
        remainder.copy_within(1.., 0);
        if let Some(last) = remainder.last_mut() {
            *last = 0;
        }
        if factor == 0 {
            continue;
        }
        let log = usize::from(LOG[factor as usize]);
        for (slot, &coefficient) in remainder.iter_mut().zip(g.iter()) {
            if coefficient != 0 {
                *slot ^= EXP[(usize::from(LOG[coefficient as usize]) + log) % 255];
            }
        }
    }
    remainder
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The field is a field: multiplication is commutative, 1 is the unit, and every non-zero
    /// element has an inverse.
    #[test]
    fn the_field_is_closed_and_invertible() {
        assert_eq!(mul(0, 5), 0);
        assert_eq!(mul(1, 200), 200);
        for a in [1u8, 2, 3, 87, 128, 255] {
            for b in [1u8, 2, 3, 87, 128, 255] {
                assert_eq!(mul(a, b), mul(b, a));
            }
            let inverse = (1..=255u8).find(|b| mul(a, *b) == 1);
            assert!(inverse.is_some(), "{a} has no inverse");
        }
        // α⁸ is the primitive polynomial's low byte, x⁴ + x³ + x² + 1 = 0x1D.
        assert_eq!(EXP[8], 0x1D);
        assert_eq!(EXP[0], 1);
        assert_eq!(EXP[254], 142);
    }

    /// The generator polynomials, written out, and the property that defines them: each is monic
    /// of degree n and every one of its roots is a power of alpha that the polynomial sends to
    /// zero. The two smallest are small enough to check by hand — (x − 1)(x − alpha) is
    /// x² + 3x + 2, since 1 + alpha = 3 and 1 · alpha = 2 in this field.
    #[test]
    fn the_generator_polynomials_are_as_the_field_gives_them() {
        assert_eq!(generator(2), vec![3, 2]);
        assert_eq!(generator(7), vec![127, 122, 154, 164, 11, 68, 117]);
        assert_eq!(generator(10), vec![216, 194, 159, 111, 199, 94, 95, 113, 157, 193]);
        for n in 1..=30usize {
            let g = generator(n);
            assert_eq!(g.len(), n, "degree {n}");
            for i in 0..n {
                // Evaluate at alpha^i: the coefficients run from x^(n-1) down to x^0, and the x^n
                // term is the leading 1 the generator does not keep.
                let mut value = 1u8;
                for coefficient in &g {
                    value = mul(value, EXP[i % 255]) ^ coefficient;
                }
                assert_eq!(value, 0, "n = {n}, root alpha^{i}");
            }
        }
    }

    /// The worked example of ISO/IEC 18004 Annex I: the version 1-M symbol for "01234567" has the
    /// data codewords 16 32 12 86 97 128 236 17 236 17 236 17 236 17 236 17 and the error
    /// correction codewords 165 36 212 193 237 54 199 135 44 85.
    #[test]
    fn the_annex_i_example_comes_out() {
        let data = [16u8, 32, 12, 86, 97, 128, 236, 17, 236, 17, 236, 17, 236, 17, 236, 17];
        assert_eq!(codewords(&data, 10), vec![165, 36, 212, 193, 237, 54, 199, 135, 44, 85]);
    }
}
