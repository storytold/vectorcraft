//! Type › Kinsoku Shori Settings…: the document's custom Kinsoku Shori sets
//! ([`vectorcraft_doc::KinsokuSet`]). Each set lists the characters that may not start a line,
//! those that may not end one, the hanging punctuation Burasagari hangs, and the inseparable
//! sequences Bunri-Kinshi keeps together. A paragraph chooses one by name in Kinsoku Set
//! (`text.setFormat {kinsoku: name}`) and holds a copy of it, which an edit of the set replaces.
//!
//! Behavior reference: the help page
//! <https://helpx.adobe.com/illustrator/desktop/design-with-text/edit-format-text/use-kinsoku-for-japanese-line-breaking.html>.
//! The name rules, the limits and Hard as the set a deleted set's paragraphs take are VectorCraft's
//! own.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Map, Value, json};
use vectorcraft_doc::text::{KINSOKU_CATEGORY_MAX, KINSOKU_NAME_MAX, KINSOKU_SEQUENCE, KINSOKU_SETS_MAX};
use vectorcraft_doc::{Document, Kinsoku, KinsokuSet, NodeKind, ParaStyle};

use super::textstyles::each_text_mut;
use super::typecmd::refresh_bounds;
use super::{CommandSpec, Result, Session, bad, cmd, has_doc};

/// The command Type › Kinsoku Shori Settings…'s OK runs, and the menu item's.
pub const SET: &str = "type.kinsokuSets.set";

/// The keys a set may give.
const SET_KEYS: [&str; 6] = ["name", "from", "cannotStart", "cannotEnd", "hanging", "inseparable"];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "type.kinsokuSets.list",
            "Kinsoku Sets",
            [],
            None,
            "{} the active document's custom Kinsoku Shori sets, and the paragraphs and paragraph styles using each → {sets: [{name, cannotStart: characters, cannotEnd: characters, hanging: characters, inseparable: [sequence], paragraphs, styles}]}",
            has_doc,
            list
        ),
        cmd!(
            query "type.kinsokuSets.base",
            "Kinsoku Set Base",
            [],
            None,
            "{base: \"none\"|\"hard\"|\"soft\"|name of a custom set (any case), name?} the categories of a built-in or custom set, as a new set based on it starts (Type › Kinsoku Shori Settings…'s New Set): Hard and Soft list the characters they keep off a line's start and end, the East Asian comma and full stop as hanging punctuation, and the pairs Bunri-Kinshi keeps together; None lists nothing → {set: {name, cannotStart, cannotEnd, hanging, inseparable}}",
            has_doc,
            base
        ),
        cmd!(
            "type.kinsokuSets.set",
            "Kinsoku Shori Settings…",
            ["Type"],
            None,
            "{sets: [{name, from?: the name the set had (rename), cannotStart?: characters, cannotEnd?: characters, hanging?: characters, inseparable?: [sequence of 2..16 characters]}]} replace the active document's custom Kinsoku Shori sets (Type › Kinsoku Shori Settings…'s OK), as one undo step, none when nothing changes. Cannot Start Line characters never start a line and Cannot End Line characters never end one; Hanging Punctuation characters hang outside a line's end with Burasagari; while Bunri-Kinshi is on, a line never breaks inside an inseparable sequence. Each category keeps each character once, sorted, at most 4096 characters (the sequences count each of their characters); a control or space character fails the call, as do an unknown key and a `from` the document has no set of. Names are trimmed, unique in any case, at most 128 characters, never None, Hard or Soft; at most 256 sets. A set given the name of a set the document has (any case) and no `from` is that set. Every paragraph and paragraph style using a set takes its new characters and name; one using a set left out takes Hard. The built-in None, Hard and Soft can't be changed or deleted → {sets}",
            has_doc,
            set
        ),
    ]
}

/// Call `f` with every paragraph of the type in `d`'s layers (opacity masks included), symbols,
/// graph designs, pattern tiles and brush art, as [`each_text_mut`] reaches them.
fn each_paragraph(d: &Document, f: &mut impl FnMut(&ParaStyle)) {
    let brushes = vectorcraft_brush::library(d);
    let trees = d.layers.iter().map(|l| &**l).chain(d.symbols.iter().map(|s| &*s.art)).chain(d.graph_designs.iter().map(|g| &*g.art));
    let patterns = d.patterns.iter().chain(d.pattern_edit.iter().filter_map(|pe| pe.original.as_ref()));
    let trees = trees.chain(patterns.flat_map(|p| p.art.iter().map(|a| &**a))).chain(brushes.iter().flat_map(|b| b.art()));
    for tree in trees {
        vectorcraft_doc::swatches::walk_with_masks(tree, &mut |node| {
            if let NodeKind::Text(t) = &node.kind {
                (0..t.paragraph_count()).for_each(|i| f(t.para_at(i)));
            }
        });
    }
}

/// How many paragraphs and paragraph styles use each set of the document, by lowercased name.
fn uses(s: &Session) -> Result<HashMap<String, (usize, usize)>> {
    let mut n: HashMap<String, (usize, usize)> = HashMap::new();
    let d = &s.doc()?.doc;
    each_paragraph(d, &mut |p| {
        if let Some(set) = &p.kinsoku_set {
            n.entry(set.name.to_lowercase()).or_default().0 += 1;
        }
    });
    for st in d.para_styles.iter() {
        if let Some(name) = st.attrs.get("kinsoku_set").and_then(|v| v.get("name")).and_then(Value::as_str) {
            n.entry(name.to_lowercase()).or_default().1 += 1;
        }
    }
    Ok(n)
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let uses = uses(s)?;
    let sets: Vec<Value> = s
        .doc()?
        .doc
        .kinsoku_sets
        .iter()
        .map(|set| {
            let (paragraphs, styles) = uses.get(&set.name.to_lowercase()).copied().unwrap_or_default();
            let mut v = json!(set);
            if let Some(o) = v.as_object_mut() {
                o.insert("paragraphs".into(), json!(paragraphs));
                o.insert("styles".into(), json!(styles));
            }
            v
        })
        .collect();
    Ok(json!({ "sets": sets }))
}

/// The set `name` names: a built-in one (`none`, `hard`, `soft`) or a custom set of `sets` (any
/// case).
fn named_base(sets: &[KinsokuSet], name: &str) -> Option<KinsokuSet> {
    match name {
        "none" => Some(vectorcraft_text::kinsoku_set_from(Kinsoku::None, "")),
        "hard" => Some(vectorcraft_text::kinsoku_set_from(Kinsoku::Hard, "")),
        "soft" => Some(vectorcraft_text::kinsoku_set_from(Kinsoku::Soft, "")),
        _ => KinsokuSet::named(sets, name.trim()).cloned(),
    }
}

fn base(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "type.kinsokuSets.base";
    let from = p.get("base").and_then(Value::as_str).ok_or_else(|| bad(C, "give base: \"none\", \"hard\", \"soft\" or a set's name"))?;
    let mut set = named_base(&s.doc()?.doc.kinsoku_sets, from).ok_or_else(|| bad(C, format!("no Kinsoku set named `{from}`")))?;
    set.name = match p.get("name") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(n)) => n.clone(),
        Some(_) => return Err(bad(C, "name must be text")),
    };
    Ok(json!({ "set": set }))
}

/// A category of single characters from `v` (text, one character after another): empty when left
/// out. Text far longer than a category holds is refused before it is read.
fn chars(v: Option<&Value>, key: &str, name: &str, c: &str) -> Result<Vec<char>> {
    match v {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::String(text)) => {
            if text.chars().count() > KINSOKU_CATEGORY_MAX * 4 {
                return Err(bad(c, format!("`{name}`: {key} must be at most {KINSOKU_CATEGORY_MAX} characters")));
            }
            Ok(text.chars().collect())
        }
        Some(_) => Err(bad(c, format!("`{name}`: {key} must be text, one character after another"))),
    }
}

/// Set `v` from command parameters, its name trimmed, with the name it had (`from`, trimmed) when
/// it gives one.
fn parse_set(v: &Value, c: &str) -> Result<(KinsokuSet, Option<String>)> {
    let o = v.as_object().ok_or_else(|| bad(c, "each set must be {name, cannotStart?, cannotEnd?, hanging?, inseparable?}"))?;
    if let Some(k) = o.keys().find(|k| !SET_KEYS.contains(&k.as_str())) {
        return Err(bad(c, format!("unknown key `{k}`; a set takes {}", SET_KEYS.join(", "))));
    }
    let raw = o.get("name").and_then(Value::as_str).ok_or_else(|| bad(c, "each set needs a name"))?;
    let name = KinsokuSet::clean_name(raw).ok_or_else(|| {
        bad(c, format!("`{raw}` can't name a Kinsoku set (1 to {KINSOKU_NAME_MAX} characters, no control characters, not None, Hard or Soft)"))
    })?;
    let from = match o.get("from") {
        None | Some(Value::Null) => None,
        Some(Value::String(f)) => Some(f.trim().to_string()),
        Some(_) => return Err(bad(c, format!("`{name}`: from must be text"))),
    };
    let inseparable = match o.get("inseparable") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(list)) => {
            if list.len() > KINSOKU_CATEGORY_MAX {
                return Err(bad(c, format!("`{name}`: inseparable: at most {KINSOKU_CATEGORY_MAX} characters in all")));
            }
            list.iter()
                .map(|s| {
                    s.as_str().map(str::to_string).ok_or_else(|| {
                        bad(
                            c,
                            format!(
                                "`{name}`: inseparable must be a list of {} to {} characters each",
                                KINSOKU_SEQUENCE.start(),
                                KINSOKU_SEQUENCE.end()
                            ),
                        )
                    })
                })
                .collect::<Result<Vec<_>>>()?
        }
        Some(_) => return Err(bad(c, format!("`{name}`: inseparable must be a list of sequences"))),
    };
    let mut set = KinsokuSet {
        cannot_start: chars(o.get("cannotStart"), "cannotStart", &name, c)?,
        cannot_end: chars(o.get("cannotEnd"), "cannotEnd", &name, c)?,
        hanging: chars(o.get("hanging"), "hanging", &name, c)?,
        inseparable,
        name,
    };
    set.normalize();
    set.check().map_err(|e| bad(c, format!("`{}`: {e}", set.name)))?;
    Ok((set, from))
}

/// Make `after` the active document's custom sets, as one undo step (none when nothing changes).
/// `renamed` maps the lowercased name of each set of the document that is kept to the index in
/// `after` of what it becomes; a set it leaves out is deleted, and its paragraphs and paragraph
/// styles take Hard.
fn replace(s: &mut Session, after: Vec<KinsokuSet>, renamed: HashMap<String, usize>) -> Result<()> {
    let before = s.doc()?.doc.kinsoku_sets.to_vec();
    let after_arcs: Vec<Arc<KinsokuSet>> = after.iter().cloned().map(Arc::new).collect();
    // The paragraph value of each set of the document as it was: the new set, or none (Hard).
    let next = |old: &KinsokuSet| -> Option<Option<Arc<KinsokuSet>>> {
        let key = old.name.to_lowercase();
        before.iter().any(|b| b.name.to_lowercase() == key).then(|| renamed.get(&key).and_then(|&i| after_arcs.get(i).cloned()))
    };
    let fix = |p: &mut ParaStyle| -> bool {
        let Some(old) = p.kinsoku_set.as_deref() else { return false };
        let Some(new) = next(old) else { return false };
        if new.as_deref() == Some(old) {
            return false;
        }
        if new.is_none() {
            p.kinsoku = Kinsoku::Hard;
        }
        p.kinsoku_set = new;
        true
    };
    let mut texts_change = false;
    each_paragraph(&s.doc()?.doc, &mut |p| texts_change |= !texts_change && fix(&mut p.clone()));
    let style_attrs = |attrs: &Map<String, Value>| -> Option<Map<String, Value>> {
        let old: KinsokuSet = serde_json::from_value(attrs.get("kinsoku_set")?.clone()).ok()?;
        let new = next(&old)?;
        if new.as_deref() == Some(&old) {
            return None;
        }
        let mut m = attrs.clone();
        match new {
            Some(set) => m.insert("kinsoku_set".into(), json!(set)),
            None => {
                m.insert("kinsoku".into(), json!(Kinsoku::Hard));
                m.insert("kinsoku_set".into(), Value::Null)
            }
        };
        Some(m)
    };
    let styles_change = s.doc()?.doc.para_styles.iter().any(|st| style_attrs(&st.attrs).is_some());
    if before == after && !texts_change && !styles_change {
        return Ok(());
    }
    s.edit("Kinsoku Shori Settings", |d, _| {
        *d.kinsoku_sets = after.clone();
        for st in d.para_styles.iter_mut() {
            if let Some(m) = style_attrs(&st.attrs) {
                st.attrs = m;
            }
        }
        each_text_mut(d, &mut |t| {
            let mut changed = false;
            for p in t.para_styles_mut() {
                changed |= fix(p);
            }
            if changed {
                refresh_bounds(t);
            }
            Ok(())
        })
    })
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let list =
        p.get("sets").and_then(Value::as_array).ok_or_else(|| bad(SET, "give sets: [{name, cannotStart?, cannotEnd?, hanging?, inseparable?}…]"))?;
    if list.len() > KINSOKU_SETS_MAX {
        return Err(bad(SET, format!("a document keeps at most {KINSOKU_SETS_MAX} Kinsoku sets")));
    }
    let parsed = list.iter().map(|v| parse_set(v, SET)).collect::<Result<Vec<_>>>()?;
    let mut names = std::collections::HashSet::new();
    for (set, _) in &parsed {
        if !names.insert(set.name.to_lowercase()) {
            return Err(bad(SET, format!("two Kinsoku sets are named `{}`", set.name)));
        }
    }
    let before = s.doc()?.doc.kinsoku_sets.to_vec();
    let has = |n: &str| before.iter().any(|b| KinsokuSet::same_name(&b.name, n));
    // Which set of the document each given set continues: the one `from` names, else the one of
    // its own name that no other set's `from` claims.
    let mut renamed: HashMap<String, usize> = HashMap::new();
    for (i, (set, from)) in parsed.iter().enumerate() {
        if let Some(f) = from {
            if !has(f) {
                return Err(bad(SET, format!("`{}`: the document has no Kinsoku set named `{f}`", set.name)));
            }
            if renamed.insert(f.to_lowercase(), i).is_some() {
                return Err(bad(SET, format!("two Kinsoku sets come from `{f}`")));
            }
        }
    }
    for (i, (set, from)) in parsed.iter().enumerate() {
        let key = set.name.to_lowercase();
        if from.is_none() && has(&set.name) && !renamed.contains_key(&key) {
            renamed.insert(key, i);
        }
    }
    let after: Vec<KinsokuSet> = parsed.into_iter().map(|(set, _)| set).collect();
    replace(s, after.clone(), renamed)?;
    Ok(json!({ "sets": after }))
}

/// The custom set of the active document that `text.setFormat`'s `kinsoku` names (any case), as a
/// paragraph holds it.
pub(crate) fn chosen(s: &Session, name: &str) -> Result<Option<Arc<KinsokuSet>>> {
    Ok(KinsokuSet::named(&s.doc()?.doc.kinsoku_sets, name.trim()).cloned().map(Arc::new))
}

#[cfg(test)]
mod tests;
