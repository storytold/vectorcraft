//! Operand values, and the dictionaries the editing data writes postfix: `/Type :` opens one,
//! `values key ,` adds an entry (a type name such as `/Real` may stand before the key; in an
//! `/Array` there is no key), `;` closes it (after a last `values /key` entry, or a bare value: its
//! content), leaving it as a value.

use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum V {
    Num(f64),
    Str(Vec<u8>),
    Name(String),
    Arr(Vec<V>),
    Obj(Rc<Obj>),
    /// `[`, until its `]`.
    Mark,
}

impl V {
    pub fn num(&self) -> Option<f64> {
        match self {
            V::Num(v) => Some(*v),
            _ => None,
        }
    }
    pub fn obj(&self) -> Option<&Obj> {
        match self {
            V::Obj(o) => Some(o),
            _ => None,
        }
    }
    /// A string's or name's text.
    pub fn text(&self) -> Option<String> {
        match self {
            V::Str(s) => Some(text(s)),
            V::Name(n) => Some(n.clone()),
            _ => None,
        }
    }
}

/// Text from the data's bytes: UTF-8, else Windows-1252 (Latin-1 for its gaps).
pub(super) fn text(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|b| cp1252(*b)).collect(),
    }
}

fn cp1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—',
        '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
    ];
    match b {
        0x80..=0x9f => HIGH.get(usize::from(b - 0x80)).copied().unwrap_or('?'),
        _ => char::from(b),
    }
}

/// A dictionary.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Obj {
    pub ty: String,
    /// Entries in order: the key (`None` in an array) and the values before it.
    pub entries: Vec<(Option<String>, Vec<V>)>,
    /// Values closed without a key.
    pub content: Vec<V>,
    /// Vector art enclosed by `X=` … `X+` in an opacity-mask dictionary.
    pub art: Option<std::sync::Arc<vectorcraft_doc::Node>>,
}

impl Obj {
    /// The values of entry `key` (the first one).
    pub fn get(&self, key: &str) -> Option<&[V]> {
        self.entries.iter().find(|(k, _)| k.as_deref() == Some(key)).map(|(_, v)| v.as_slice())
    }
    /// The numbers of entry `key`.
    pub fn nums(&self, key: &str) -> Vec<f64> {
        self.get(key).unwrap_or_default().iter().filter_map(V::num).collect()
    }
    /// The first text value of entry `key`.
    pub fn text(&self, key: &str) -> Option<String> {
        self.get(key)?.iter().find(|v| matches!(v, V::Str(_))).and_then(V::text)
    }
    /// The dictionary value of entry `key`.
    pub fn obj(&self, key: &str) -> Option<&Obj> {
        self.get(key)?.iter().find_map(V::obj)
    }
    /// The dictionaries an array holds, in order.
    pub fn items(&self) -> impl Iterator<Item = &Obj> {
        self.entries.iter().flat_map(|(_, v)| v.iter()).chain(self.content.iter()).filter_map(V::obj)
    }
    /// The first text of the content.
    pub fn content_text(&self) -> Option<String> {
        self.content.iter().find_map(V::text)
    }
}

/// Add the values on `stack` above `base` to `o` as one entry (`,`): the last is the key, except
/// in an array.
pub(super) fn add_entry(o: &mut Obj, stack: &mut Vec<V>, base: usize) {
    let mut vals: Vec<V> = stack.drain(base.min(stack.len())..).collect();
    if o.ty == "Array" {
        o.entries.push((None, vals));
        return;
    }
    let key = match vals.last() {
        Some(V::Str(_) | V::Name(_)) => vals.pop().and_then(|k| k.text()),
        _ => None,
    };
    o.entries.push((key, vals));
}

/// Close `o` (`;`): a last entry ending with a name is an entry, other values are its content.
pub(super) fn close(o: &mut Obj, stack: &mut Vec<V>, base: usize) {
    if stack.len() <= base {
        return;
    }
    if o.ty != "Array" && matches!(stack.last(), Some(V::Name(_))) {
        add_entry(o, stack, base);
    } else {
        let vals: Vec<V> = stack.drain(base.min(stack.len())..).collect();
        o.content.extend(vals);
    }
}

/// An object name from its XML id (`AI10_ArtUID`).
pub(super) fn xml_name(id: &str) -> String {
    vectorcraft_doc::xml_id::to_object_name(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_from_xml_ids() {
        assert_eq!(xml_name("Layer_1"), "Layer 1");
        assert_eq!(xml_name("My_Rect__x23_2"), "My Rect #2");
        assert_eq!(xml_name("_x31_abc__x28_paren_x29_"), "1abc (paren)");
        assert_eq!(xml_name("Ünï_日本"), "Ünï 日本");
        assert_eq!(xml_name("S_00000092429200007574234970000005974767392385616558_"), "S");
        assert_eq!(xml_name("a_x_"), "a x ");
        assert_eq!(xml_name("_xD800_"), " xD800 ");
    }

    #[test]
    fn entries_and_content() {
        let mut o = Obj { ty: "XMLUID".into(), ..Obj::default() };
        let mut st = vec![V::Num(9.0), V::Str(b"Main".to_vec())];
        close(&mut o, &mut st, 1);
        assert_eq!(o.content_text().as_deref(), Some("Main"));
        assert_eq!(st, vec![V::Num(9.0)]);
        let mut d = Obj { ty: "Dictionary".into(), ..Obj::default() };
        let mut st = vec![V::Num(1.0), V::Num(2.0), V::Name("RealPoint".into()), V::Str(b"P".to_vec())];
        add_entry(&mut d, &mut st, 0);
        let mut st = vec![V::Name("Def".into())];
        close(&mut d, &mut st, 0);
        assert_eq!(d.nums("P"), vec![1.0, 2.0]);
        assert_eq!(d.get("Def"), Some(&[][..]));
        let mut a = Obj { ty: "Array".into(), ..Obj::default() };
        let mut st = vec![V::Obj(Rc::new(d.clone()))];
        add_entry(&mut a, &mut st, 0);
        assert_eq!(a.items().count(), 1);
    }

    #[test]
    fn latin_text() {
        assert_eq!(text(b"caf\xe9 \x80"), "café €");
        assert_eq!(text("ü".as_bytes()), "ü");
    }
}
