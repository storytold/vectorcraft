//! Type › Composite Fonts…: the document's composite fonts ([`vectorcraft_doc::CompositeFont`]),
//! which set each class of character (Kanji, Kana, Punctuation, Symbols, Roman, Numerals) in a
//! font of its own. Type names a composite font as its family, and the font menus list them first.
//! The font database holds the active document's composite fonts ([`sync`]); a file exported here
//! carries them to another document.
//!
//! Behavior reference (public documentation):
//! <https://help.adobe.com/archive/en_US/illustrator/cs3/illustrator_cs3_help.pdf#page=347>.

use serde_json::{Map, Value, json};
use vectorcraft_doc::text::{COMPOSITE_FONTS_MAX, COMPOSITE_NAME_MAX};
use vectorcraft_doc::{CharClass, CompositeFont, CompositePart, CompositeParts};
use vectorcraft_text::FontDb;

use super::fileio::{read_file_head, write_file};
use super::textstyles::each_text_mut;
use super::typecmd::refresh_bounds;
use super::{CommandSpec, Result, Session, bad, cmd, has_doc, switch_or, text_param};

/// The extension of the files Export writes and Import reads.
pub const EXTS: &[&str] = &["vccomposite"];

/// The largest file Import reads, in bytes.
const MAX_FILE_BYTES: usize = 4 << 20;

/// The keys a part may give.
const PART_KEYS: [&str; 7] = ["font", "style", "size", "baseline", "hScale", "vScale", "scaleAtCenter"];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "type.compositeFonts.list",
            "Composite Fonts",
            [],
            None,
            "{} the active document's composite fonts → {fonts: [{name, parts: {kanji, kana, punctuation, symbols, roman, numerals: {font, style, size: % of the type size, baseline: % of the type size (positive raises), hScale: %, vScale: %, scaleAtCenter?: bool}}}]}",
            has_doc,
            list
        ),
        cmd!(
            "type.compositeFonts.set",
            "Composite Fonts…",
            ["Type"],
            None,
            "{fonts: [{name, parts: {kanji: {font, style?: \"Regular\", size?: % 10..400 = 100, baseline?: % -100..100 = 0}, kana?, punctuation?, symbols?, roman?, numerals?: {font, style?, size?, baseline?, hScale?, vScale?: % 10..400 = 100, scaleAtCenter?: bool = false}}}]} replace the active document's composite fonts (Type › Composite Fonts…'s OK), as one undo step, none when nothing changes. A composite font sets each class of character in its part's font: kanji (the base: ideographs, full-width letters and digits and anything no other class takes), kana (hiragana and katakana, both widths), punctuation (East Asian and full-width punctuation, quotes and dashes), symbols (arrows, math, shapes and other symbols), roman (half-width letters, their punctuation and spaces) and numerals (the digits 0 to 9); a part left out is the kanji part. size and baseline are percent of the type's size, hScale and vScale scale kana, roman and numerals only, and scaleAtCenter (kana only) scales them from the em box's center instead of the baseline. Names are trimmed, unique in any case, at most 255 characters, and may not name an installed font; a part's font may not be a composite font; at most 256. Type set in a composite font names it as its family (text.setStyle {font: name}), and the font menus list composite fonts first. A font taken out stays named by its type, which then shows as missing → {fonts}",
            has_doc,
            set
        ),
        cmd!(
            query "type.compositeFonts.export",
            "Export Composite Fonts",
            [],
            None,
            "{path?: .vccomposite file, names?: [name] (default: all), fonts?: [{name, parts}] (as type.compositeFonts.set takes them)} write the active document's composite fonts (all, or those named, any case), or the fonts given, to a file type.compositeFonts.import reads; without path the file's text comes back as data → {path, count} | {data, count}",
            has_doc,
            export
        ),
        cmd!(
            "type.compositeFonts.import",
            "Import Composite Fonts",
            [],
            None,
            "{path: .vccomposite file | data: its text | dataBase64: its bytes (at most 4 MB), apply?: bool = true} read the composite fonts a file holds (type.compositeFonts.export writes them); with apply they join the active document's, a font of the same name (any case) replaced, as one undo step, none when nothing changes; apply false only reads them → {fonts, imported: [name]}",
            has_doc,
            import
        ),
    ]
}

/// Register the active document's composite fonts in the font database, and remove the ones the
/// other open documents (or an earlier active document) defined. Run after each command and when
/// another document takes the front, so an undo, a revert, an opened or closed file or a document
/// switch draws type in the active document's own composite fonts, and type naming one it does
/// not define shows as missing. Whether a composite font was added, changed or removed.
pub(crate) fn sync(s: &mut Session) -> bool {
    let db = FontDb::global();
    let mut names = Vec::new();
    let mut changed = false;
    for f in s.active().map(|d| d.doc.composite_fonts.to_vec()).unwrap_or_default() {
        changed |= db.set_composite_font(&f);
        names.push(f.name.to_lowercase());
    }
    let others = s.docs.iter().flat_map(|d| d.doc.composite_fonts.iter().map(|f| f.name.to_lowercase()));
    let stale: Vec<String> = s.composite_names.iter().cloned().chain(others).filter(|n| !names.contains(n)).collect();
    for gone in stale {
        changed |= db.remove_composite_font(&gone);
    }
    s.composite_names = names;
    changed
}

/// Leave out a file's unusable composite fonts before registering and measuring its type.
pub(crate) fn filter_loaded(doc: &mut vectorcraft_doc::Document) {
    let db = FontDb::global();
    if let Some(kept) = CompositeFont::usable(&doc.composite_fonts) {
        *doc.composite_fonts = kept;
    }
    doc.composite_fonts.retain(|f| !db.has_installed_family(&f.name));
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(json!({ "fonts": *s.doc()?.doc.composite_fonts }))
}

/// Composite font `v` from command parameters or a file: its name trimmed, each part left out a
/// copy of the Kanji part, and keys or values a part can't take refused.
fn parse_font(v: &Value, c: &str) -> Result<CompositeFont> {
    let o = v.as_object().ok_or_else(|| bad(c, "each font must be {name, parts}"))?;
    if let Some(k) = o.keys().find(|k| !["name", "parts"].contains(&k.as_str())) {
        return Err(bad(c, format!("unknown key `{k}`; a font takes name and parts")));
    }
    let raw = o.get("name").and_then(Value::as_str).ok_or_else(|| bad(c, "each font needs a name"))?;
    let name = CompositeFont::clean_name(raw)
        .ok_or_else(|| bad(c, format!("`{raw}` can't name a composite font (1 to {COMPOSITE_NAME_MAX} characters, no control characters)")))?;
    let parts = o.get("parts").and_then(Value::as_object).ok_or_else(|| bad(c, format!("`{name}`: parts must be an object")))?;
    if let Some(k) = parts.keys().find(|k| !CharClass::ALL.iter().any(|c| c.key() == k.as_str())) {
        return Err(bad(c, format!("`{name}`: unknown class `{k}`; the classes are kanji, kana, punctuation, symbols, roman and numerals")));
    }
    let part = |class: CharClass| -> Result<Option<CompositePart>> {
        let Some(p) = parts.get(class.key()) else { return Ok(None) };
        let po = p.as_object().ok_or_else(|| bad(c, format!("`{name}`: {} must be an object", class.key())))?;
        if let Some(k) = po.keys().find(|k| !PART_KEYS.contains(&k.as_str())) {
            return Err(bad(c, format!("`{name}`: unknown key `{k}` in {}", class.key())));
        }
        serde_json::from_value(p.clone()).map(Some).map_err(|e| bad(c, format!("`{name}`: {}: {e}", class.key())))
    };
    let kanji = part(CharClass::Kanji)?.ok_or_else(|| bad(c, format!("`{name}`: give the kanji part")))?;
    let mut f = CompositeFont { name: name.clone(), parts: CompositeParts::all(kanji) };
    for class in CharClass::ALL.into_iter().skip(1) {
        if let Some(p) = part(class)? {
            *f.parts.get_mut(class) = p;
        }
    }
    f.check().map_err(|e| bad(c, format!("`{}`: {e}", f.name)))?;
    Ok(f)
}

/// Error unless `fonts` can stand together in a document: names unique in any case, none naming
/// an installed font, no part set in a composite font, and at most [`COMPOSITE_FONTS_MAX`].
fn check_set(fonts: &[CompositeFont], c: &str) -> Result<()> {
    if fonts.len() > COMPOSITE_FONTS_MAX {
        return Err(bad(c, format!("a document keeps at most {COMPOSITE_FONTS_MAX} composite fonts")));
    }
    let db = FontDb::global();
    let mut names = std::collections::HashSet::new();
    for f in fonts {
        if !names.insert(f.name.to_lowercase()) {
            return Err(bad(c, format!("two composite fonts are named `{}`", f.name)));
        }
        if db.has_installed_family(&f.name) {
            return Err(bad(c, format!("`{}` is the name of an installed font", f.name)));
        }
    }
    for f in fonts {
        for class in CharClass::ALL {
            let font = &f.parts.get(class).font;
            if names.contains(&font.to_lowercase()) || db.composite_font(font).is_some() {
                return Err(bad(c, format!("`{}`: {} is set in `{font}`, a composite font", f.name, class.key())));
            }
        }
    }
    Ok(())
}

/// Make `after` the active document's composite fonts, as one undo step named `label` (none when
/// they are what it has). The font database forgets the ones taken out, and type set in a font
/// that changed is measured again.
fn replace(s: &mut Session, label: &str, after: Vec<CompositeFont>) -> Result<()> {
    let before = s.doc()?.doc.composite_fonts.to_vec();
    if before == after {
        return Ok(());
    }
    let changed: Vec<String> =
        before.iter().filter(|b| !after.contains(b)).chain(after.iter().filter(|a| !before.contains(a))).map(|f| f.name.to_lowercase()).collect();
    let db = FontDb::global();
    for f in &before {
        if !after.iter().any(|a| CompositeFont::same_name(&a.name, &f.name)) {
            db.remove_composite_font(&f.name);
        }
    }
    for f in &after {
        db.set_composite_font(f);
    }
    let r = s.edit(label, |d, _| {
        *d.composite_fonts = after.clone();
        each_text_mut(d, &mut |t| {
            if t.runs.iter().any(|r| changed.contains(&r.style.font_family.to_lowercase())) {
                refresh_bounds(t);
            }
            Ok(())
        })
    });
    sync(s);
    r
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "type.compositeFonts.set";
    let list = p.get("fonts").and_then(Value::as_array).ok_or_else(|| bad(C, "give fonts: [{name, parts}…]"))?;
    let fonts = list.iter().map(|v| parse_font(v, C)).collect::<Result<Vec<_>>>()?;
    check_set(&fonts, C)?;
    replace(s, "Composite Fonts", fonts.clone())?;
    Ok(json!({ "fonts": fonts }))
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "type.compositeFonts.export";
    let path = text_param(p, "path", C)?;
    if path.is_some_and(|p| p.trim().is_empty()) {
        return Err(bad(C, "path is empty"));
    }
    let all = match p.get("fonts") {
        None | Some(Value::Null) => s.doc()?.doc.composite_fonts.to_vec(),
        Some(Value::Array(list)) => {
            let fonts = list.iter().map(|v| parse_font(v, C)).collect::<Result<Vec<_>>>()?;
            check_set(&fonts, C)?;
            fonts
        }
        Some(_) => return Err(bad(C, "fonts must be a list of composite fonts")),
    };
    let fonts = match p.get("names") {
        None | Some(Value::Null) => all,
        Some(Value::Array(names)) => {
            let mut out = Vec::with_capacity(names.len());
            for n in names {
                let n = n.as_str().ok_or_else(|| bad(C, "names must be strings"))?;
                let f = all
                    .iter()
                    .find(|f| CompositeFont::same_name(&f.name, n.trim()))
                    .ok_or_else(|| bad(C, format!("no composite font named `{n}`")))?;
                if !out.contains(f) {
                    out.push(f.clone());
                }
            }
            out
        }
        Some(_) => return Err(bad(C, "names must be a list of names")),
    };
    if fonts.is_empty() {
        return Err(bad(C, "no composite fonts to export"));
    }
    let mut file = Map::new();
    file.insert("compositeFonts".into(), json!(fonts));
    let text = serde_json::to_string_pretty(&file).map_err(|e| bad(C, e.to_string()))?;
    match path {
        Some(path) => {
            write_file(path, text.as_bytes())?;
            Ok(json!({ "path": path, "count": fonts.len() }))
        }
        None => Ok(json!({ "data": text, "count": fonts.len() })),
    }
}

/// The composite fonts in the bytes of an exported file.
fn read_fonts(bytes: &[u8], c: &str) -> Result<Vec<CompositeFont>> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| bad(c, format!("not a composite fonts file: {e}")))?;
    let list = v.get("compositeFonts").and_then(Value::as_array).ok_or_else(|| bad(c, "not a composite fonts file: no compositeFonts list"))?;
    if list.len() > COMPOSITE_FONTS_MAX {
        return Err(bad(c, format!("the file holds more than {COMPOSITE_FONTS_MAX} composite fonts")));
    }
    list.iter().map(|f| parse_font(f, c)).collect()
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "type.compositeFonts.import";
    let apply = switch_or(p, "apply", true, C)?;
    let given = (text_param(p, "path", C)?, text_param(p, "data", C)?, text_param(p, "dataBase64", C)?);
    let too_large = || bad(C, "the file is larger than 4 MB");
    let bytes = match given {
        (Some(path), None, None) if !path.trim().is_empty() => read_file_head(path, MAX_FILE_BYTES + 1)?,
        (None, Some(text), None) => text.as_bytes().to_vec(),
        (None, None, Some(b64)) => {
            if b64.len() > MAX_FILE_BYTES / 3 * 4 + 4 {
                return Err(too_large());
            }
            vectorcraft_format::base64_decode(b64).ok_or_else(|| bad(C, "dataBase64 is not base64"))?
        }
        _ => return Err(bad(C, "give one of path, data or dataBase64")),
    };
    if bytes.len() > MAX_FILE_BYTES {
        return Err(too_large());
    }
    let fonts = read_fonts(&bytes, C)?;
    if fonts.is_empty() {
        return Err(bad(C, "the file holds no composite fonts"));
    }
    let mut names = std::collections::HashSet::new();
    if let Some(f) = fonts.iter().find(|f| !names.insert(f.name.to_lowercase())) {
        return Err(bad(C, format!("the file holds two composite fonts named `{}`", f.name)));
    }
    if apply {
        let mut after = s.doc()?.doc.composite_fonts.to_vec();
        for f in &fonts {
            match after.iter_mut().find(|a| CompositeFont::same_name(&a.name, &f.name)) {
                Some(a) => *a = f.clone(),
                None => after.push(f.clone()),
            }
        }
        check_set(&after, C)?;
        replace(s, "Import Composite Fonts", after)?;
    } else {
        check_set(&fonts, C)?;
    }
    let imported: Vec<&str> = fonts.iter().map(|f| f.name.as_str()).collect();
    Ok(json!({ "fonts": fonts, "imported": imported }))
}

#[cfg(test)]
mod tests;
