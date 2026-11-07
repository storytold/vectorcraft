//! Values (ECMA-262 5.1 section 8) and the conversions between primitives (section 9).
//!
//! A string is a rope: concatenation makes a node that points at its two halves, and the first
//! read of its characters flattens it, with a loop and an explicit stack, into one buffer that
//! replaces the node. Strings hold no objects, so the reference counts that share them cannot form
//! a cycle; dropping a long rope also uses a loop, not recursion.

use std::cell::RefCell;
use std::rc::Rc;

use crate::heap::ObjId;

/// A JavaScript value. Objects are handles into the interpreter's heap.
#[derive(Clone, Debug)]
pub(crate) enum Value {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(Str),
    Object(ObjId),
}

impl Value {
    pub(crate) fn str(s: &str) -> Value {
        Value::String(Str::from(s))
    }

    pub(crate) fn as_object(&self) -> Option<ObjId> {
        match self {
            Value::Object(id) => Some(*id),
            _ => None,
        }
    }

    pub(crate) fn is_nullish(&self) -> bool {
        matches!(self, Value::Undefined | Value::Null)
    }

    /// ToBoolean (section 9.2).
    pub(crate) fn truthy(&self) -> bool {
        match self {
            Value::Undefined | Value::Null => false,
            Value::Bool(b) => *b,
            Value::Number(n) => !(n.is_nan() || *n == 0.0),
            Value::String(s) => s.len() > 0,
            Value::Object(_) => true,
        }
    }
}

/// The text of a string value, in UTF-16 code units: a flat buffer or a concatenation.
#[derive(Clone)]
pub(crate) struct Str(Rc<RefCell<Node>>);

enum Node {
    Flat(Rc<[u16]>),
    Concat {
        left: Str,
        right: Str,
        len: usize,
    },
    /// A node whose halves were moved out while it was dropped.
    Taken,
}

impl std::fmt::Debug for Str {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.to_rust_string())
    }
}

impl From<&str> for Str {
    fn from(s: &str) -> Self {
        Str::from_units(s.encode_utf16().collect())
    }
}

/// Strings at most this long are copied when concatenated instead of becoming a rope node.
const FLAT_CONCAT: usize = 32;

impl Str {
    pub(crate) fn from_units(units: Vec<u16>) -> Self {
        Str::from_rc(units.into())
    }

    pub(crate) fn from_rc(units: Rc<[u16]>) -> Self {
        Str(Rc::new(RefCell::new(Node::Flat(units))))
    }

    pub(crate) fn len(&self) -> usize {
        match &*self.0.borrow() {
            Node::Flat(f) => f.len(),
            Node::Concat { len, .. } => *len,
            Node::Taken => 0,
        }
    }

    /// The node is not yet flattened.
    pub(crate) fn is_rope(&self) -> bool {
        matches!(&*self.0.borrow(), Node::Concat { .. })
    }

    /// An address that identifies the node, for counting shared nodes once.
    pub(crate) fn node_addr(&self) -> usize {
        Rc::as_ptr(&self.0) as *const () as usize
    }

    /// The two halves of a concatenation node.
    pub(crate) fn halves(&self) -> Option<(Str, Str)> {
        match &*self.0.borrow() {
            Node::Concat { left, right, .. } => Some((left.clone(), right.clone())),
            _ => None,
        }
    }

    /// `a + b`. The caller checks the length limit first.
    pub(crate) fn concat(a: &Str, b: &Str) -> Str {
        let (la, lb) = (a.len(), b.len());
        if la == 0 {
            return b.clone();
        }
        if lb == 0 {
            return a.clone();
        }
        if la + lb <= FLAT_CONCAT {
            let mut units = Vec::with_capacity(la + lb);
            units.extend_from_slice(&a.flat());
            units.extend_from_slice(&b.flat());
            return Str::from_units(units);
        }
        Str(Rc::new(RefCell::new(Node::Concat { left: a.clone(), right: b.clone(), len: la + lb })))
    }

    /// The code units, flattening the rope first if it is one.
    pub(crate) fn flat(&self) -> Rc<[u16]> {
        if let Node::Flat(f) = &*self.0.borrow() {
            return f.clone();
        }
        let mut units = Vec::with_capacity(self.len());
        let mut stack = vec![self.clone()];
        while let Some(s) = stack.pop() {
            match &*s.0.borrow() {
                Node::Flat(f) => units.extend_from_slice(f),
                Node::Concat { left, right, .. } => {
                    stack.push(right.clone());
                    stack.push(left.clone());
                }
                Node::Taken => {}
            }
        }
        let flat: Rc<[u16]> = units.into();
        let old = std::mem::replace(&mut *self.0.borrow_mut(), Node::Flat(flat.clone()));
        drop_node(old);
        flat
    }

    pub(crate) fn to_rust_string(&self) -> String {
        String::from_utf16_lossy(&self.flat())
    }
}

impl PartialEq for Str {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0) || (self.len() == other.len() && self.flat() == other.flat())
    }
}

/// Drops a node's halves with a loop, so a rope of a million appends does not recurse a million
/// levels deep.
fn drop_node(node: Node) {
    let mut stack = Vec::new();
    if let Node::Concat { left, right, .. } = node {
        stack.push(left);
        stack.push(right);
    }
    while let Some(s) = stack.pop() {
        if Rc::strong_count(&s.0) == 1 {
            let taken = std::mem::replace(&mut *s.0.borrow_mut(), Node::Taken);
            if let Node::Concat { left, right, .. } = taken {
                stack.push(left);
                stack.push(right);
            }
        }
    }
}

impl Drop for Str {
    fn drop(&mut self) {
        if Rc::strong_count(&self.0) == 1 && self.is_rope() {
            let node = std::mem::replace(&mut *self.0.borrow_mut(), Node::Taken);
            drop_node(node);
        }
    }
}

/// White space and line terminators as StrWhiteSpaceChar (section 9.3.1) counts them.
pub(crate) fn is_str_space(u: u16) -> bool {
    matches!(u, 0x09 | 0x0A | 0x0B | 0x0C | 0x0D | 0x20 | 0xA0 | 0x1680 | 0x180E | 0x2028 | 0x2029 | 0x202F | 0x205F | 0x3000 | 0xFEFF)
        || (0x2000..=0x200A).contains(&u)
}

/// ToNumber applied to a string (section 9.3.1): the StringNumericLiteral grammar, NaN for
/// anything else.
pub(crate) fn string_to_number(units: &[u16]) -> f64 {
    let start = units.iter().position(|&u| !is_str_space(u)).unwrap_or(units.len());
    let end = units.iter().rposition(|&u| !is_str_space(u)).map_or(start, |i| i + 1);
    let Some(text) = units.get(start..end) else { return f64::NAN };
    if text.is_empty() {
        return 0.0;
    }
    let Ok(s) = String::from_utf16(text) else { return f64::NAN };
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        if hex.is_empty() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return f64::NAN;
        }
        return hex.bytes().fold(0.0, |acc, b| acc * 16.0 + f64::from((b as char).to_digit(16).unwrap_or(0)));
    }
    let (sign, body) = match s.as_bytes().first() {
        Some(b'+') => (1.0, &s[1..]),
        Some(b'-') => (-1.0, &s[1..]),
        _ => (1.0, s.as_str()),
    };
    if body == "Infinity" {
        return sign * f64::INFINITY;
    }
    if !is_decimal_literal(body) {
        return f64::NAN;
    }
    body.parse::<f64>().map_or(f64::NAN, |n| sign * n)
}

/// `body` is a StrUnsignedDecimalLiteral without `Infinity`: digits with an optional fraction and
/// exponent, at least one digit in the mantissa.
fn is_decimal_literal(body: &str) -> bool {
    let b = body.as_bytes();
    let mut i = 0;
    let digits = |i: &mut usize| {
        let s = *i;
        while b.get(*i).is_some_and(u8::is_ascii_digit) {
            *i += 1;
        }
        *i - s
    };
    let mut mantissa = digits(&mut i);
    if b.get(i) == Some(&b'.') {
        i += 1;
        mantissa += digits(&mut i);
    }
    if mantissa == 0 {
        return false;
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        if digits(&mut i) == 0 {
            return false;
        }
    }
    i == b.len()
}

/// ToString applied to a number (section 9.8.1).
pub(crate) fn number_to_str(n: f64) -> Str {
    Str::from(crate::number::to_string(n).as_str())
}

/// ToInt32 (section 9.5).
pub(crate) fn to_int32(n: f64) -> i32 {
    to_uint32(n) as i32
}

/// ToUint32 (section 9.6).
pub(crate) fn to_uint32(n: f64) -> u32 {
    if !n.is_finite() || n == 0.0 {
        return 0;
    }
    let int = n.trunc();
    let m = int.rem_euclid(4_294_967_296.0);
    m as u32
}

/// The array index a property name stands for (section 15.4: a canonical numeric string below
/// 2^32 − 1).
pub(crate) fn array_index(key: &[u16]) -> Option<u32> {
    if key.is_empty() || key.len() > 10 || (key.len() > 1 && key.first() == Some(&u16::from(b'0'))) {
        return None;
    }
    let mut n: u64 = 0;
    for &u in key {
        let d = u.checked_sub(u16::from(b'0')).filter(|d| *d <= 9)?;
        n = n * 10 + u64::from(d);
    }
    u32::try_from(n).ok().filter(|&n| n != u32::MAX)
}

/// The decimal digits of an array index.
pub(crate) fn index_key(i: u32) -> Rc<[u16]> {
    i.to_string().encode_utf16().collect::<Vec<u16>>().into()
}

#[cfg(test)]
mod tests {
    use super::{Str, array_index, string_to_number, to_int32, to_uint32};

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn strings_convert_to_numbers_by_the_string_numeric_literal_grammar() {
        let cases: &[(&str, f64)] = &[
            ("", 0.0),
            ("  \n ", 0.0),
            ("12", 12.0),
            (" -1.5e2 ", -150.0),
            ("0x1F", 31.0),
            (".5", 0.5),
            ("5.", 5.0),
            ("+Infinity", f64::INFINITY),
            ("-Infinity", f64::NEG_INFINITY),
        ];
        for (s, want) in cases {
            assert_eq!(string_to_number(&u(s)), *want, "{s:?}");
        }
        for s in ["inf", "nan", "1_0", "0x", "1e", ".", "--1", "0x-1", "infinity", "1 2"] {
            assert!(string_to_number(&u(s)).is_nan(), "{s:?}");
        }
    }

    #[test]
    fn int32_and_uint32_wrap() {
        assert_eq!(to_int32(4_294_967_296.0 + 5.0), 5);
        assert_eq!(to_int32(2_147_483_648.0), i32::MIN);
        assert_eq!(to_uint32(-1.0), u32::MAX);
        assert_eq!(to_int32(f64::NAN), 0);
        assert_eq!(to_int32(-1.9), -1);
    }

    #[test]
    fn array_indices_are_canonical_and_below_the_largest_u32() {
        assert_eq!(array_index(&u("0")), Some(0));
        assert_eq!(array_index(&u("4294967294")), Some(4_294_967_294));
        assert_eq!(array_index(&u("4294967295")), None);
        assert_eq!(array_index(&u("01")), None);
        assert_eq!(array_index(&u("1.0")), None);
        assert_eq!(array_index(&u("")), None);
    }

    #[test]
    fn a_million_appends_flatten_and_drop_without_recursion() {
        let handle = std::thread::Builder::new()
            .stack_size(256 << 10)
            .spawn(|| {
                let x = Str::from("abcdefghijklmnopqrstuvwxyz0123456789");
                let mut s = Str::from("");
                for _ in 0..1_000_000 {
                    s = Str::concat(&s, &x);
                }
                let len = s.len();
                let flat = s.flat();
                (len, flat.len())
            })
            .unwrap();
        assert_eq!(handle.join().unwrap(), (36_000_000, 36_000_000));
        let handle = std::thread::Builder::new()
            .stack_size(256 << 10)
            .spawn(|| {
                let x = Str::from("abcdefghijklmnopqrstuvwxyz0123456789");
                let mut s = Str::from("");
                for _ in 0..1_000_000 {
                    s = Str::concat(&x, &s);
                }
                s.len()
            })
            .unwrap();
        assert_eq!(handle.join().unwrap(), 36_000_000);
    }
}
