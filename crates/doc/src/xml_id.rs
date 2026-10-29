//! Object names ↔ XML IDs (Preferences › Units › Identify Objects By).
//!
//! Illustrator's public documentation: with XML ID selected, the Layers panel shows and edits
//! names that conform to XML naming (letter, underscore or colon first; no spaces). Underscores
//! stand for spaces and `_xHH_` / `_xHHHH_` escape other characters, as in `.ai` `AI10_ArtUID`
//! values ([edit dynamic objects](https://helpx.adobe.com/illustrator/desktop/automate-visualize-data/automate-actions/edit-dynamic-objects.html)).

/// An object name from its XML id: `_` is a space and `_xHH_` / `_xHHHH_` a character; ids made
/// unique end with `_` and 20 or more digits (stripped).
pub fn to_object_name(id: &str) -> String {
    let id = strip_unique(id);
    let mut out = String::with_capacity(id.len());
    let mut rest = id;
    while let Some(c) = rest.chars().next() {
        if c == '_' {
            if let Some((ch, n)) = escaped(rest) {
                out.push(ch);
                rest = rest.get(n..).unwrap_or_default();
                continue;
            }
            out.push(' ');
        } else {
            out.push(c);
        }
        rest = rest.get(c.len_utf8()..).unwrap_or_default();
    }
    out
}

/// The XML id for an object name (the inverse of [`to_object_name`]). The empty name stays empty,
/// since a bare `_` would read back as a space.
pub fn from_object_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len().saturating_mul(2));
    let last = name.chars().count().saturating_sub(1);
    let chars: Vec<char> = name.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c == ' ' && (i == last || starts_fake_escape(chars.get(i + 1..).unwrap_or_default())) {
            // A trailing bare `_` could read back as the end of a uniqueness suffix, and a bare
            // `_` before `x<hex>_` would read back as an `_xHH_` escape.
            out.push_str("_x20_");
        } else if c == ' ' {
            out.push('_');
        } else if c == '_' {
            out.push_str("_x5F_");
        } else if (i == 0 && !(c.is_alphabetic() || c == ':')) || !(c.is_alphanumeric() || c == '.' || c == '-' || c == ':') {
            // A first character that cannot start an XML name is escaped, never given a bare `_`
            // prefix (that would read back as a space).
            out.push_str(&format!("_x{:X}_", u32::from(c)));
        } else {
            out.push(c);
        }
    }
    out
}

/// Whether `id` is a usable XML ID for Identify Objects By › XML ID: non-empty, no whitespace,
/// starts with a letter, `_` or `:`, and otherwise letters, digits, `.`, `-`, `_` or `:`.
pub fn is_valid(id: &str) -> bool {
    let id = id.trim();
    if id.is_empty() || id.chars().any(char::is_whitespace) {
        return false;
    }
    let mut chars = id.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_alphabetic() || first == '_' || first == ':') {
        return false;
    }
    chars.all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '-' | ':'))
}

/// Whether the encoded `rest` (after a space) starts with `x`, 1 to 6 hex digits and a character
/// whose encoding begins with `_`, so that a bare `_` before it would form an escape.
fn starts_fake_escape(rest: &[char]) -> bool {
    let Some(('x', digits)) = rest.split_first() else {
        return false;
    };
    let n = digits.iter().take_while(|c| c.is_ascii_hexdigit()).count();
    if n == 0 || n > 6 {
        return false;
    }
    digits.get(n).is_some_and(|&c| c == ' ' || c == '_' || !(c.is_alphanumeric() || matches!(c, '.' | '-' | ':')))
}

/// `_xHHHH_` at the start of `s`: the character and the escape's length.
fn escaped(s: &str) -> Option<(char, usize)> {
    let body = s.strip_prefix("_x")?;
    let end = body.find('_')?;
    let hex = body.get(..end)?;
    if hex.is_empty() || hex.len() > 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let c = char::from_u32(u32::from_str_radix(hex, 16).ok()?)?;
    Some((c, end + 3))
}

/// `id` without the `_000…_` suffix that makes it unique.
fn strip_unique(id: &str) -> &str {
    let Some(body) = id.strip_suffix('_') else {
        return id;
    };
    let Some(at) = body.rfind('_') else {
        return id;
    };
    let digits = body.get(at + 1..).unwrap_or_default();
    if digits.len() >= 20 && digits.bytes().all(|b| b.is_ascii_digit()) { body.get(..at).unwrap_or(id) } else { id }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_ai_style_ids() {
        for (name, id) in [
            ("Layer 1", "Layer_1"),
            ("My Rect #2", "My_Rect__x23_2"),
            ("1abc (paren)", "_x31_abc__x28_paren_x29_"),
            ("Ünï 日本", "Ünï_日本"),
            ("a_b", "a_x5F_b"),
        ] {
            assert_eq!(from_object_name(name), id, "encode {name}");
            assert_eq!(to_object_name(id), name, "decode {id}");
        }
        assert_eq!(to_object_name("S_00000092429200007574234970000005974767392385616558_"), "S");
        assert!(is_valid("Layer_1") && is_valid("_x31_abc") && is_valid(":root"));
        assert!(!is_valid("My Rect") && !is_valid("1abc") && !is_valid(""));
    }

    #[test]
    fn sign_is_not_a_hex_digit_in_escapes() {
        assert_eq!(to_object_name("a_x+41_b"), "a x+41 b");
        assert_eq!(to_object_name("a_x-41_b"), "a x-41 b");
    }

    #[test]
    fn space_before_x_hex_and_underscore_is_not_an_escape() {
        for name in ["layer x1 copy", "a xa b", "a xe b", "a xb b", "a x1(b", "a x1_b", "a x123456 c"] {
            let id = from_object_name(name);
            assert_eq!(to_object_name(&id), name, "name {name:?} -> id {id:?}");
        }
        assert_eq!(from_object_name("layer x1 copy"), "layer_x20_x1_copy");
    }

    #[test]
    fn round_trip_names_that_cannot_start_an_xml_name() {
        for name in ["-abc", ".5 mm", "²x", "Ⅷ", "", "1abc", "Layer 1", " lead", "_a", ":a", "#"] {
            let id = from_object_name(name);
            assert_eq!(to_object_name(&id), name, "name {name:?} -> id {id:?}");
        }
        let name = "Scan 12345678901234567890 ";
        let id = from_object_name(name);
        assert!(id.ends_with("_x20_"), "{id:?}");
        assert_eq!(to_object_name(&id), name);
        assert_eq!(from_object_name("-abc"), "_x2D_abc");
        assert_eq!(from_object_name(".5 mm"), "_x2E_5_mm");
        assert_eq!(from_object_name("²x"), "_xB2_x");
    }
}
