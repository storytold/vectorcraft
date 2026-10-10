//! Type → Find Font: the fonts a document uses (flagging missing ones), replacing one font with
//! another everywhere or in the selection, and selecting the text that uses a font. Also the fonts
//! available (the bundled and installed ones) and rescanning the installed fonts.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};

use super::typecmd::refresh_bounds;
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "text.fonts",
            "Fonts in Document",
            [],
            None,
            "{selectionOnly?} → [{family, style, runs, objects, missing, status: exact|substitute|missing, resolved: {family, style}, missingGlyphs, version?: the installed version the type names (when it names one)}] sorted by name. A font is found by any of its names (ヒラギノ角ゴシック = Hiragino Sans) among the loaded and installed fonts; substitute: the family is there but not the style (resolved names the stand-in); missing: the family is unknown (the fallback family stands in); missingGlyphs: characters of its text the resolved font lacks (drawn by a fallback font)",
            has_doc,
            fonts
        ),
        cmd!(
            "text.replaceFont",
            "Replace Font",
            [],
            None,
            "{from: {family, style?}, to: {family (any of its names; stored as the font's own family name), style?}, selectionOnly?} (style omitted: every style of the family / keep the closest style) → {runs}",
            has_doc,
            replace
        ),
        cmd!(
            "select.font",
            "Select Text by Font",
            [],
            None,
            "{family, style?} select the text objects using the font → {count}",
            has_doc,
            select_font
        ),
        cmd!(
            query "text.fontList",
            "Font List",
            [],
            None,
            "{family?} → {families: [names]} sorted: the bundled fonts, fonts added and the fonts installed on the system (none on the web), each the family's English (canonical) name for `text.setStyle` and documents (without the system's hidden families, whose names start with a dot; they still resolve by name). Preferences › Type › Show Font Names in English only changes the UI menu labels, not this list; with family: {family, styles: [names]} (upright styles by weight, then italics; an error when the family isn't available)",
            always,
            font_list
        ),
        cmd!(
            "text.rescanFonts",
            "Refresh Font List",
            [],
            None,
            "{} scan the system font folders again, for fonts installed or removed since the app started; text in a font that became available redraws in it (no fonts are installed on the web) → {families, faces} (families listed, installed faces found)",
            always,
            rescan
        ),
    ]
}

/// Text objects in scope: the selection's (and their descendants') or the whole document's.
fn scope(s: &Session, selection_only: bool) -> Result<Vec<NodeId>> {
    let st = s.doc()?;
    let mut v = vec![];
    let mut add = |n: &vectorcraft_doc::Node| {
        n.walk(&mut |c| {
            if matches!(c.kind, NodeKind::Text(_)) && !v.contains(&c.id) {
                v.push(c.id);
            }
        })
    };
    if selection_only {
        for id in &st.selection.objects {
            if let Some(n) = st.doc.node(*id) {
                add(n);
            }
        }
    } else {
        for l in &st.doc.layers {
            add(l);
        }
    }
    Ok(v)
}

/// A font as type names it: family, style and, when it names one, the installed version.
pub(super) type UsedFont = (String, String, Option<String>);

/// How a font is named in reports: family and style, then the version the type names, if any.
pub(super) fn font_label((family, style, version): &UsedFont) -> String {
    match version {
        Some(v) => format!("{family} {style} ({v})"),
        None => format!("{family} {style}"),
    }
}

/// The fonts (family, style, version) of the type in the layers and symbols of `d`.
pub(super) fn used_fonts(d: &Document) -> BTreeSet<UsedFont> {
    let mut fonts = BTreeSet::new();
    let mut add = |n: &Node| {
        if let NodeKind::Text(t) = &n.kind {
            fonts.extend(t.runs.iter().map(|r| (r.style.font_family.clone(), r.style.font_style.clone(), r.style.font_version.clone())));
        }
    };
    d.walk(&mut add);
    for s in &d.symbols {
        s.art.walk(&mut add);
    }
    fonts
}

/// What an export that draws type (as outlines, embedded fonts or pixels) says when `d` uses font
/// families that aren't available: their type is drawn in the fallback font, not the font it names.
pub(super) fn substitution_warning(d: &Document) -> Option<String> {
    let db = vectorcraft_text::FontDb::global();
    let mut missing: Vec<String> = used_fonts(d)
        .into_iter()
        .filter(|(family, style, _)| db.resolve(family, style).is_none_or(|(_, m)| m == vectorcraft_text::FontMatch::Missing))
        .map(|(family, _, _)| family)
        .collect();
    // Sorted by family: its styles and versions are neighbours.
    missing.dedup();
    (!missing.is_empty()).then(|| {
        format!(
            "fonts that aren't available were written in the fallback font, {}: {} (install them, or replace them with Find Font)",
            vectorcraft_text::FALLBACK_FAMILY,
            missing.join(", ")
        )
    })
}

fn text(d: &Document, id: NodeId) -> Option<&vectorcraft_doc::TextObject> {
    match d.node(id).map(|n| &n.kind) {
        Some(NodeKind::Text(t)) => Some(t),
        _ => None,
    }
}

fn fonts(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = scope(s, bool_or(p, "selectionOnly", false))?;
    let d = &s.doc()?.doc;
    let db = vectorcraft_text::FontDb::global();
    let mut found: BTreeMap<UsedFont, (usize, Vec<NodeId>, String)> = BTreeMap::new();
    for id in ids {
        for r in text(d, id).map(|t| t.runs.as_slice()).unwrap_or_default() {
            let e = found.entry((r.style.font_family.clone(), r.style.font_style.clone(), r.style.font_version.clone())).or_default();
            e.0 += 1;
            e.2.push_str(&r.text);
            if !e.1.contains(&id) {
                e.1.push(id);
            }
        }
    }
    let list: Vec<Value> = found
        .into_iter()
        .map(|((family, style, version), (runs, objects, chars))| {
            // The face the type is set in: the version it names when that one is installed.
            let resolved = db.resolve(&family, &style).map(|(f, m)| (db.face_version(&family, &style, version.as_deref()).unwrap_or(f), m));
            let status = resolved.as_ref().map(|(_, m)| *m).unwrap_or(vectorcraft_text::FontMatch::Missing);
            let mut lacking: Vec<char> = chars.chars().filter(|c| !c.is_whitespace() && !c.is_control()).collect();
            lacking.sort_unstable();
            lacking.dedup();
            lacking.retain(|c| !resolved.as_ref().is_some_and(|(f, _)| f.covers(*c)));
            let mut row = json!({
                "family": family, "style": style, "runs": runs, "objects": objects.len(),
                "missing": status == vectorcraft_text::FontMatch::Missing,
                "status": status.as_str(),
                "resolved": resolved.as_ref().map(|(f, _)| json!({ "family": f.family, "style": f.style })),
                "missingGlyphs": lacking.len(),
            });
            if let Some(v) = version {
                row["version"] = json!(v);
            }
            row
        })
        .collect();
    Ok(json!(list))
}

fn font_param(p: &Value, k: &str, c: &str) -> Result<(String, Option<String>)> {
    let v = p.get(k).ok_or_else(|| bad(c, format!("missing `{k}`")))?;
    let family =
        v.get("family").and_then(Value::as_str).filter(|f| !f.trim().is_empty()).ok_or_else(|| bad(c, format!("`{k}.family` is required")))?;
    Ok((family.to_string(), v.get("style").and_then(Value::as_str).map(str::to_string)))
}

fn replace(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.replaceFont";
    let (ff, fs) = font_param(p, "from", C)?;
    let (tf, ts) = font_param(p, "to", C)?;
    let db = vectorcraft_text::FontDb::global();
    // Any name of the family (ヒラギノ角ゴシック) is stored as its own name (Hiragino Sans).
    let tf = match db.resolve(&tf, ts.as_deref().unwrap_or("Regular")) {
        Some((f, m)) if m != vectorcraft_text::FontMatch::Missing => f.family.clone(),
        _ => return Err(bad(C, format!("font family `{tf}` is not available"))),
    };
    let ids = scope(s, bool_or(p, "selectionOnly", false))?;
    let matches = |st: &vectorcraft_doc::CharStyle| {
        st.font_family.eq_ignore_ascii_case(&ff) && fs.as_ref().is_none_or(|x| st.font_style.eq_ignore_ascii_case(x))
    };
    let n = s.edit("Replace Font", |d, _| {
        let mut n = 0;
        for id in &ids {
            let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) else { continue };
            let mut changed = false;
            for r in &mut t.runs {
                if matches(&r.style) {
                    // An explicit style, else the closest the new family has to the old one.
                    let style = ts
                        .clone()
                        .or_else(|| db.face(&tf, &r.style.font_style).map(|f| f.style.clone()))
                        .unwrap_or_else(|| r.style.font_style.clone());
                    r.style.font_family = tf.clone();
                    r.style.font_style = style;
                    // Another font: none of the old one's versions.
                    r.style.font_version = None;
                    changed = true;
                    n += 1;
                }
            }
            if changed {
                vectorcraft_text::edit::normalize(&mut t.runs);
                refresh_bounds(t);
            }
        }
        Ok(n)
    })?;
    Ok(json!({ "runs": n }))
}

fn select_font(s: &mut Session, p: &Value) -> Result<Value> {
    let family = str_param(p, "family").ok_or_else(|| bad("select.font", "missing `family`"))?.to_string();
    let style = str_param(p, "style").map(str::to_string);
    let ids = scope(s, false)?;
    let d = &s.doc()?.doc;
    let hits: Vec<NodeId> = ids
        .into_iter()
        .filter(|id| {
            text(d, *id).is_some_and(|t| {
                t.runs.iter().any(|r| {
                    r.style.font_family.eq_ignore_ascii_case(&family) && style.as_ref().is_none_or(|x| r.style.font_style.eq_ignore_ascii_case(x))
                })
            })
        })
        .collect();
    let n = hits.len();
    s.select(|_, sel| sel.set(hits))?;
    Ok(json!({ "count": n }))
}

fn font_list(_: &mut Session, p: &Value) -> Result<Value> {
    let db = vectorcraft_text::FontDb::global();
    match str_param(p, "family") {
        Some(family) => {
            if !db.has_family(family) {
                return Err(bad("text.fontList", format!("font family `{family}` is not available")));
            }
            // The name as the font spells it.
            let name = db.family_list().iter().find(|f| f.eq_ignore_ascii_case(family)).cloned().unwrap_or_else(|| family.to_string());
            Ok(json!({ "family": name, "styles": db.styles(family) }))
        }
        None => Ok(json!({ "families": *db.menu_family_list() })),
    }
}

pub(crate) fn rescan(s: &mut Session, _: &Value) -> Result<Value> {
    let db = vectorcraft_text::FontDb::global();
    #[cfg(not(target_arch = "wasm32"))]
    let faces = db.load_system_fonts();
    #[cfg(target_arch = "wasm32")]
    let faces = 0;
    // The canvas redraws type in the fonts available now (renderers lay it out again when the
    // fonts change), and the bounds follow; that isn't an edit to save.
    for d in &mut s.docs {
        let mut texts = vec![];
        d.doc.walk(|n| {
            if matches!(n.kind, NodeKind::Text(_)) {
                texts.push(n.id);
            }
        });
        if !texts.is_empty() {
            let saved = !d.is_dirty();
            let doc = std::sync::Arc::make_mut(&mut d.doc);
            for id in texts {
                if let Some(NodeKind::Text(t)) = doc.node_mut(id).map(|n| &mut n.kind) {
                    refresh_bounds(t);
                }
            }
            if saved {
                d.mark_saved();
            }
        }
        d.revision += 1;
    }
    Ok(json!({ "families": db.menu_family_list().len(), "faces": faces }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Type › Options › Additional Fonts Folder (#683) adds its folder to the system font scan,
    /// trimmed; clearing it takes the folder out again.
    #[test]
    fn the_additional_fonts_folder_joins_the_font_scan() {
        let mut s = Session::new();
        let dir = std::env::temp_dir().join("vectorcraft-fonts-folder-pref");
        s.execute("prefs.set", &json!({"key": "fontsFolder", "value": format!("  {}  ", dir.display())})).unwrap();
        assert_eq!(vectorcraft_text::user_font_dirs(), std::slice::from_ref(&dir));
        if !cfg!(target_arch = "wasm32") {
            assert!(vectorcraft_text::system_font_dirs().contains(&dir));
        }
        s.execute("prefs.set", &json!({"key": "fontsFolder", "value": ""})).unwrap();
        assert!(vectorcraft_text::user_font_dirs().is_empty() && !vectorcraft_text::system_font_dirs().contains(&dir));
    }

    #[test]
    fn list_flag_missing_replace_and_select() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        let a = s.execute("text.create", &json!({"x": 10, "y": 20, "text": "Serif", "font": "Source Serif 4"})).unwrap()["id"].as_u64().unwrap();
        let b = s.execute("text.create", &json!({"x": 10, "y": 60, "text": "Gone", "font": "No Such Font Family"})).unwrap()["id"].as_u64().unwrap();
        let list = s.execute("text.fonts", &json!({})).unwrap();
        let missing: Vec<&str> = list.as_array().unwrap().iter().filter(|f| f["missing"] == true).map(|f| f["family"].as_str().unwrap()).collect();
        assert_eq!(missing, ["No Such Font Family"]);
        assert_eq!(s.execute("select.font", &json!({"family": "No Such Font Family"})).unwrap()["count"], 1);
        assert_eq!(s.doc().unwrap().selection.objects, [NodeId(b)]);
        let r = s.execute("text.replaceFont", &json!({"from": {"family": "No Such Font Family"}, "to": {"family": "Source Sans 3"}})).unwrap();
        assert_eq!(r["runs"], 1);
        let fams: Vec<String> =
            s.execute("text.fonts", &json!({})).unwrap().as_array().unwrap().iter().map(|f| f["family"].as_str().unwrap().to_string()).collect();
        assert_eq!(fams, ["Source Sans 3", "Source Serif 4"]);
        assert!(s.execute("text.replaceFont", &json!({"from": {"family": "Source Serif 4"}, "to": {"family": "Nope"}})).is_err());
        let _ = a;
    }
}

#[cfg(test)]
mod open_tests {
    use super::*;

    #[test]
    fn list_says_how_each_font_resolved() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        // "Black Wide" is a style no Source Sans 3 has, bundled or installed, so it is always a
        // substitute (the installed family can hold a real Black).
        for (text, font, style) in
            [("Exact", "Source Sans 3", "Semibold"), ("Closest", "Source Sans 3", "Black Wide"), ("Gone", "No Such Font Family", "Regular")]
        {
            s.execute("text.create", &json!({"x": 10, "y": 20, "text": text, "font": font, "style": style})).unwrap();
        }
        let list = s.execute("text.fonts", &json!({})).unwrap();
        let rows: Vec<(String, String, String, String)> = list
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                (
                    f["style"].as_str().unwrap().into(),
                    f["status"].as_str().unwrap().into(),
                    f["resolved"]["family"].as_str().unwrap().into(),
                    f["resolved"]["style"].as_str().unwrap().into(),
                )
            })
            .collect();
        let row = |style: &str| rows.iter().find(|r| r.0 == style).cloned().unwrap();
        assert_eq!(row("Semibold").1, "exact");
        assert_eq!(row("Black Wide").1, "substitute", "{rows:?}");
        assert_eq!(row("Black Wide").2, "Source Sans 3");
        assert_eq!((row("Regular").1.as_str(), row("Regular").2.as_str()), ("missing", vectorcraft_text::FALLBACK_FAMILY));
        s.execute("text.create", &json!({"x": 10, "y": 90, "text": "Ab 雅楽", "font": "Inter"})).unwrap();
        let inter = s.execute("text.fonts", &json!({})).unwrap().as_array().unwrap().iter().find(|f| f["family"] == "Inter").cloned().unwrap();
        assert_eq!(inter["missingGlyphs"], 2, "雅 and 楽 come from a fallback font: {inter}");
    }

    #[test]
    fn opened_documents_have_exact_text_bounds() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 200})).unwrap();
        let id = s.execute("text.create", &json!({"x": 10, "y": 60, "text": "WIDE TRACKED", "size": 40})).unwrap()["id"].as_u64().unwrap();
        s.execute("text.setStyle", &json!({"tracking": 400})).unwrap();
        let want = s.doc().unwrap().doc.node(NodeId(id)).unwrap().geometric_bounds().unwrap();
        // Save and reopen: the cache isn't in the file, yet the bounds must match.
        let bytes = vectorcraft_format::save(&s.doc().unwrap().doc, false);
        let reopened = vectorcraft_format::load(&bytes).unwrap();
        let mut t = Session::new();
        t.add_document(reopened, None);
        let got = t.doc().unwrap().doc.node(NodeId(id)).unwrap().geometric_bounds().unwrap();
        assert!((got.width() - want.width()).abs() < 1e-6, "{got:?} vs {want:?}");
    }
}

#[cfg(test)]
mod dirty_tests {
    use super::*;

    #[test]
    fn selection_never_marks_modified_and_undo_to_saved_is_clean() {
        let mut s = Session::new();
        s.add_document(Document::new(100.0, 100.0), None);
        let id = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
        s.doc_mut().unwrap().mark_saved();
        s.execute("select.none", &json!({})).unwrap();
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        assert!(!s.doc().unwrap().is_dirty(), "selecting is not an edit");
        s.execute("object.move", &json!({"dx": 5, "dy": 0})).unwrap();
        assert!(s.doc().unwrap().is_dirty());
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(!s.doc().unwrap().is_dirty(), "back to the saved state");
    }
}
