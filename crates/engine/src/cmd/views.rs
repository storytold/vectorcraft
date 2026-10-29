//! View → New View… / Edit Views…: saved views stored in the document; per-document view
//! toggles (Show Transparency Grid).

use serde_json::{Value, json};
use vectorcraft_doc::SavedView;
use vectorcraft_geom::Point;

use super::{CommandSpec, Result, Session, bad, bool_or, cmd, f64_req, finite_numbers, has_doc, str_param};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "view.saved.new",
            "New View…",
            ["View"],
            None,
            "{name?, center: [x, y], zoom: (1 = 100%), rotation?: deg} save a view in the document (the app fills in the current one; default \"View N\"; at most 25, names up to 255 characters, each name once) → {name, index}",
            has_doc,
            new_view
        ),
        cmd!(
            "view.saved.edit",
            "Edit Views…",
            ["View"],
            None,
            "{name, newName?: rename, delete?: bool} | {edits: [{name, newName?, delete?}…]} rename or delete saved views, all in one undo step; names are those before the edit and must stay unique; no params → {views}",
            has_views,
            edit_views
        ),
        cmd!(query "view.saved.list", "Saved Views", [], None, "{} → {views: [{name, center, zoom, rotation}]}", has_doc, list_views),
        cmd!(
            query "view.transparencyGrid",
            "Show Transparency Grid",
            ["View"],
            Some("Cmd+Shift+D"),
            "{on?: bool (default: toggle)} show the transparency grid behind the active document's artboards (each document has its own setting) → {on}",
            has_doc,
            transparency_grid
        ),
    ]
}

fn transparency_grid(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.doc_mut()?;
    st.transparency_grid = p.get("on").and_then(Value::as_bool).unwrap_or(!st.transparency_grid);
    Ok(json!({ "on": st.transparency_grid }))
}

fn list_views(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(json!({ "views": s.doc()?.doc.views }))
}

fn new_view(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "view.saved.new";
    let [x, y] = p.get("center").and_then(finite_numbers::<2>).ok_or_else(|| bad(C, "center must be [x, y]"))?;
    let zoom = f64_req(p, "zoom", C)?;
    if !(zoom.is_finite() && zoom > 0.0) {
        return Err(bad(C, "zoom must be positive"));
    }
    // Given, the rotation is a number: nothing malformed falls back to 0°.
    let rotation = match p.get("rotation") {
        Some(r) => r.as_f64().filter(|r| r.is_finite()).ok_or_else(|| bad(C, "rotation must be a number of degrees"))?,
        None => 0.0,
    };
    let st = s.doc()?;
    if st.doc.views.len() >= SavedView::MAX {
        return Err(bad(C, format!("a document keeps at most {} views", SavedView::MAX)));
    }
    let name = match str_param(p, "name").and_then(SavedView::clean_name) {
        Some(n) => n,
        None => SavedView::default_name(&st.doc.views),
    };
    if st.doc.views.iter().any(|v| v.name == name) {
        return Err(bad(C, format!("a view named `{name}` exists")));
    }
    let view = SavedView { name: name.clone(), center: Point::new(x, y), zoom, rotation };
    let index = s.edit("New View", |d, _| {
        d.views.push(view);
        Ok(d.views.len() - 1)
    })?;
    Ok(json!({ "name": name, "index": index }))
}

/// Edit Views… needs a document with at least one saved view.
fn has_views(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    if st.doc.views.is_empty() { Err("no saved views".into()) } else { Ok(()) }
}

fn edit_views(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "view.saved.edit";
    let (edits, one) = match p.get("edits").and_then(Value::as_array) {
        Some(list) => (list.as_slice(), false),
        None if str_param(p, "name").is_some() => (std::slice::from_ref(p), true),
        None => return list_views(s, p),
    };
    let before = s.doc()?.doc.views.clone();
    // Edits name the views as they are now, so they don't depend on each other's order.
    let mut after: Vec<Option<SavedView>> = before.iter().cloned().map(Some).collect();
    for e in edits {
        let name = str_param(e, "name").ok_or_else(|| bad(C, "missing name"))?;
        let i = before.iter().position(|v| v.name == name).ok_or_else(|| bad(C, format!("no view named `{name}`")))?;
        let Some(slot) = after.get_mut(i) else { continue };
        if bool_or(e, "delete", false) {
            *slot = None;
        } else if let Some(v) = slot {
            v.name = str_param(e, "newName").and_then(SavedView::clean_name).ok_or_else(|| bad(C, "give newName or delete"))?;
        }
    }
    let after: Vec<SavedView> = after.into_iter().flatten().collect();
    let mut names = std::collections::HashSet::with_capacity(after.len());
    for v in &after {
        if !names.insert(v.name.as_str()) {
            return Err(bad(C, format!("a view named `{}` exists", v.name)));
        }
    }
    // One rename or delete keeps the name of its step; several are one Edit Views step.
    let deleted = after.len() < before.len();
    let label = if !one {
        "Edit Views"
    } else if deleted {
        "Delete View"
    } else {
        "Rename View"
    };
    let result = match (one, deleted) {
        (true, true) => json!({ "deleted": str_param(p, "name") }),
        (true, false) => json!({ "name": str_param(p, "newName").and_then(SavedView::clean_name) }),
        (false, _) => json!({ "views": after }),
    };
    if before != after {
        s.edit(label, |d, _| {
            d.views = after;
            Ok(())
        })?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::Session;

    /// Malformed dimensions are rejected rather than saving an arbitrary or partial viewport.
    #[test]
    fn saved_views_require_exact_finite_coordinates_and_rotation() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let before = s.doc().unwrap().history.undo.len();
        for params in [
            json!({"center": [10], "zoom": 2}),
            json!({"center": [10, 20, 30], "zoom": 2}),
            json!({"center": [10, "bad"], "zoom": 2}),
            json!({"center": [10, 20], "zoom": 2, "rotation": "bad"}),
            json!({"center": [10, 20], "zoom": 2, "rotation": null}),
        ] {
            assert!(s.execute("view.saved.new", &params).is_err(), "accepted {params}");
            assert!(s.doc().unwrap().doc.views.is_empty());
            assert_eq!(s.doc().unwrap().history.undo.len(), before);
        }
        s.execute("view.saved.new", &json!({"center": [10, 20], "zoom": 2, "rotation": -15})).unwrap();
        let views = &s.doc().unwrap().doc.views;
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].rotation, -15.0);
    }

    #[test]
    fn save_rename_delete_views() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let v = s.execute("view.saved.new", &json!({"center": [100, 200], "zoom": 2.0})).unwrap();
        assert_eq!(v["name"], json!("View 1"));
        s.execute("view.saved.new", &json!({"name": "Logo", "center": [10, 20], "zoom": 4.0, "rotation": 15})).unwrap();
        assert!(s.execute("view.saved.new", &json!({"name": "Logo", "center": [0, 0], "zoom": 1.0})).is_err());
        assert!(s.execute("view.saved.new", &json!({"center": [0, 0], "zoom": 0})).is_err());
        s.execute("view.saved.edit", &json!({"name": "View 1", "newName": "Overview"})).unwrap();
        let l = s.execute("view.saved.list", &json!({})).unwrap();
        assert_eq!(l["views"][0]["name"], json!("Overview"));
        assert_eq!(l["views"][1]["rotation"], json!(15.0));
        s.execute("view.saved.edit", &json!({"name": "Logo", "delete": true})).unwrap();
        assert_eq!(s.doc().unwrap().doc.views.len(), 1);
        // Views travel with the file.
        let bytes = vectorcraft_format::save_file(&s.doc().unwrap().doc);
        assert_eq!(vectorcraft_format::load(&bytes).unwrap().views[0].name, "Overview");
    }

    fn names(s: &mut Session) -> Vec<String> {
        s.doc().unwrap().doc.views.iter().map(|v| v.name.clone()).collect()
    }

    fn session_with_views(names: &[&str]) -> Session {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        for (i, n) in names.iter().enumerate() {
            s.execute("view.saved.new", &json!({"name": n, "center": [i, i], "zoom": 1 + i})).unwrap();
        }
        s
    }

    /// Two named views go out with the file and come back with their names, centres, zooms and
    /// rotations, in order.
    #[test]
    fn saved_views_reopen_with_their_names_and_viewports() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        s.execute("view.saved.new", &json!({"name": "Overview", "center": [306, 396], "zoom": 0.5})).unwrap();
        s.execute("view.saved.new", &json!({"name": "Logo detail", "center": [40.25, -12.5], "zoom": 8, "rotation": -30})).unwrap();
        let saved = s.doc().unwrap().doc.views.clone();
        let back = vectorcraft_format::load(&vectorcraft_format::save_file(&s.doc().unwrap().doc)).unwrap();
        assert_eq!(back.views, saved);
        assert_eq!((back.views[1].center.x, back.views[1].center.y, back.views[1].zoom, back.views[1].rotation), (40.25, -12.5, 8.0, -30.0));
    }

    /// Edit Views… applies its renames and deletes together, as one undo step, by the names the
    /// views had before (so two can swap names); a shared or unknown name changes nothing.
    #[test]
    fn edit_views_applies_several_edits_as_one_step() {
        let mut s = session_with_views(&["A", "B", "C"]);
        let steps = s.doc().unwrap().history.undo.len();
        let edits = json!({"edits": [
            {"name": "A", "newName": "B"},
            {"name": "B", "newName": " A "},
            {"name": "C", "delete": true},
        ]});
        let r = s.execute("view.saved.edit", &edits).unwrap();
        assert_eq!(r["views"].as_array().map(Vec::len), Some(2));
        assert_eq!(names(&mut s), ["B", "A"]);
        // The views keep their viewports under their new names.
        assert_eq!(s.doc().unwrap().doc.views[0].zoom, 1.0);
        assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1);
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(names(&mut s), ["A", "B", "C"]);
        for bad in [
            json!({"edits": [{"name": "A", "newName": "C"}]}),
            json!({"edits": [{"name": "A", "newName": "X"}, {"name": "B", "newName": "X"}]}),
            json!({"edits": [{"name": "A", "delete": true}, {"name": "Nope", "delete": true}]}),
            json!({"edits": [{"name": "A", "newName": "  "}]}),
        ] {
            assert!(s.execute("view.saved.edit", &bad).is_err(), "accepted {bad}");
            assert_eq!(names(&mut s), ["A", "B", "C"]);
        }
    }

    /// A document keeps at most 25 views; unnamed ones take the first free "View N", and names are
    /// capped like other names the user gives.
    #[test]
    fn a_document_keeps_at_most_25_views() {
        let all: Vec<String> = (0..25).map(|i| format!("V{i}")).collect();
        let mut s = session_with_views(&all.iter().map(String::as_str).collect::<Vec<_>>());
        assert!(s.execute("view.saved.new", &json!({"name": "One too many", "center": [0, 0], "zoom": 1})).is_err());
        assert_eq!(s.doc().unwrap().doc.views.len(), 25);
        s.execute("view.saved.edit", &json!({"name": "V0", "delete": true})).unwrap();
        assert_eq!(s.execute("view.saved.new", &json!({"center": [0, 0], "zoom": 1})).unwrap()["name"], "View 1");
        s.execute("view.saved.edit", &json!({"name": "V1", "delete": true})).unwrap();
        let long = "x".repeat(1000);
        assert_eq!(s.execute("view.saved.new", &json!({"name": long, "center": [0, 0], "zoom": 1})).unwrap()["name"], "x".repeat(255));
    }

    /// Edit Views… is off until the document has a view.
    #[test]
    fn edit_views_needs_a_saved_view() {
        let mut s = session_with_views(&[]);
        assert!(s.execute("view.saved.edit", &json!({})).is_err());
        s.execute("view.saved.new", &json!({"name": "Only", "center": [0, 0], "zoom": 1})).unwrap();
        assert_eq!(s.execute("view.saved.edit", &json!({})).unwrap()["views"][0]["name"], "Only");
    }
}
