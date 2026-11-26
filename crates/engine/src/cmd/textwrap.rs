//! Object → Text Wrap: area type flows around wrap objects stacked above it in the same layer.
//!
//! A wrap object carries [`TextWrap`] options (`Node::wrap`). After every edit [`refresh`]
//! resolves, for each area text object, the outlines of the wrap objects above it into
//! `TextObject::wrap` (text space); the layout engine subtracts them from each line's span.

use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeId, NodeKind, TextWrap};

use super::typecmd::refresh_bounds;
use super::{CommandSpec, EngineError, Result, Session, bad, checked_ids_or_selection, cmd, has_selection, number_or, switch_or};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "object.textWrap.make",
            "Make",
            ["Object", "Text Wrap"],
            None,
            "{offset?: pt (6), invert?: bool, ids?} area type below the selected objects (same layer) wraps around them; offset -1000 to 1000 (a negative one lets the text that far into them); an offset outside that range or not a number, or an invert that is not true or false, is an error → {ids}",
            has_selection,
            make
        ),
        cmd!("object.textWrap.release", "Release", ["Object", "Text Wrap"], None, "{ids?} → {ids}", has_selection, release),
        cmd!(
            "object.textWrap.options",
            "Text Wrap Options…",
            ["Object", "Text Wrap"],
            None,
            "{offset?: pt, invert?: bool, ids?} set the options of the selected wrap objects (offset -1000 to 1000; a negative one lets text into the objects); an offset outside that range or not a number, or an invert that is not true or false, is an error; no options (or null ones) → the current {offset, invert}",
            has_selection,
            options
        ),
    ]
}

/// The Offset a wrap object takes, in points.
pub const TEXT_WRAP_OFFSET_RANGE: std::ops::RangeInclusive<f64> = -1000.0..=1000.0;

/// The options `p` gives, with `base` for an omitted or null `offset` or `invert`. An `offset` that is
/// not a number or lies outside [`TEXT_WRAP_OFFSET_RANGE`], or an `invert` that is not true or false,
/// is an error. `base`'s offset is kept as it is.
fn wrap_param(p: &Value, base: TextWrap, cmd: &str) -> Result<TextWrap> {
    let key = "offset";
    let given = p.get(key).is_some_and(|v| !v.is_null());
    let offset = number_or(p, key, base.offset, cmd)?;
    if given && !TEXT_WRAP_OFFSET_RANGE.contains(&offset) {
        let (lo, hi) = (TEXT_WRAP_OFFSET_RANGE.start(), TEXT_WRAP_OFFSET_RANGE.end());
        return Err(bad(cmd, format!("`{key}` must be from {lo} to {hi}")));
    }
    Ok(TextWrap { offset, invert: switch_or(p, "invert", base.invert, cmd)? })
}

fn make(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = checked_ids_or_selection(s, p, "object.textWrap.make")?;
    let w = wrap_param(p, TextWrap::default(), "object.textWrap.make")?;
    s.edit("Make Text Wrap", |d, _| {
        for id in &ids {
            let n = d.node_mut(*id).ok_or(EngineError::NoNode(*id))?;
            if matches!(n.kind, NodeKind::Layer { .. }) {
                return Err(EngineError::Other("layers can't be wrap objects".into()));
            }
            n.extras_mut().wrap = Some(w);
        }
        Ok(())
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

fn release(s: &mut Session, p: &Value) -> Result<Value> {
    let ids: Vec<NodeId> = {
        let d = &s.doc()?.doc;
        checked_ids_or_selection(s, p, "object.textWrap.release")?
            .into_iter()
            .filter(|i| d.node(*i).is_some_and(|n| n.extras().wrap.is_some()))
            .collect()
    };
    if ids.is_empty() {
        return Err(EngineError::Other("Text Wrap: select a wrap object".into()));
    }
    s.edit("Release Text Wrap", |d, _| {
        for id in &ids {
            if let Some(n) = d.node_mut(*id) {
                n.extras_mut().wrap = None;
            }
        }
        Ok(())
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

fn options(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = checked_ids_or_selection(s, p, "object.textWrap.options")?;
    let current = {
        let d = &s.doc()?.doc;
        ids.iter().find_map(|i| d.node(*i).and_then(|n| n.extras().wrap)).unwrap_or_default()
    };
    if ["offset", "invert"].iter().all(|k| p.get(k).is_none_or(Value::is_null)) {
        return Ok(json!({ "offset": current.offset, "invert": current.invert }));
    }
    let w = wrap_param(p, current, "object.textWrap.options")?;
    s.edit("Text Wrap Options", |d, _| {
        for id in &ids {
            if let Some(n) = d.node_mut(*id)
                && !matches!(n.kind, NodeKind::Layer { .. })
            {
                n.extras_mut().wrap = Some(w);
            }
        }
        Ok(())
    })?;
    Ok(json!({ "offset": w.offset, "invert": w.invert }))
}

/// Re-resolve the wrap shapes of every area text object (called after each edit).
pub(crate) fn refresh(before: &Document, doc: &mut Document) {
    for id in vectorcraft_doc::text::resolve_wrap(Some(before), doc) {
        if let Some(NodeKind::Text(t)) = doc.node_mut(id).map(|n| &mut n.kind) {
            refresh_bounds(t);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_doc::NodeKind;

    use crate::{NodeId, Session};

    fn lines(s: &Session, id: u64) -> Vec<(f64, f64)> {
        let Some(NodeKind::Text(t)) = s.doc().unwrap().doc.node(NodeId(id)).map(|n| n.kind.clone()) else { panic!() };
        vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t).lines.iter().map(|l| l.avail).collect()
    }

    /// Invalid explicit ids must not turn a requested multi-object edit into a partial one.
    #[test]
    fn text_wrap_commands_reject_malformed_target_lists_without_editing() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
        let id = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 80, "height": 80})).unwrap()["id"].as_u64().unwrap();
        let other = s.execute("shape.rectangle", &json!({"x": 120, "y": 10, "width": 80, "height": 80})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.set", &json!({"ids": [id]})).unwrap();

        let before = s.doc().unwrap().history.undo.len();
        for command in ["object.textWrap.make", "object.textWrap.release", "object.textWrap.options"] {
            for ids in [json!([id, "invalid"]), json!([id, 99999]), json!({"not": "an array"})] {
                let e = s.execute(command, &json!({"ids": ids, "offset": 12})).unwrap_err().to_string();
                assert!(e.contains("id") || e.contains("array"), "{command}: {e}");
                assert_eq!(s.doc().unwrap().history.undo.len(), before, "{command} added history on error");
                assert!(s.doc().unwrap().doc.node(NodeId(id)).unwrap().extras().wrap.is_none());
                assert!(s.doc().unwrap().doc.node(NodeId(other)).unwrap().extras().wrap.is_none());
            }
        }

        s.execute("object.textWrap.make", &json!({"ids": [id]})).unwrap();
        assert!(s.doc().unwrap().doc.node(NodeId(id)).unwrap().extras().wrap.is_some());
        assert!(s.doc().unwrap().doc.node(NodeId(other)).unwrap().extras().wrap.is_none());
        s.execute("object.textWrap.options", &json!({"ids": [id], "offset": 12})).unwrap();
        assert_eq!(s.doc().unwrap().doc.node(NodeId(id)).unwrap().extras().wrap.unwrap().offset, 12.0);
        s.execute("object.textWrap.release", &json!({"ids": [id]})).unwrap();
        assert!(s.doc().unwrap().doc.node(NodeId(id)).unwrap().extras().wrap.is_none());
    }

    #[test]
    fn area_type_wraps_around_objects_above_and_follows_edits() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 600, "height": 600})).unwrap();
        let text = "word ".repeat(200);
        let t = s.execute("text.create", &json!({"x": 0, "y": 0, "text": text, "size": 12, "area": {"width": 400, "height": 400}})).unwrap()["id"]
            .as_u64()
            .unwrap();
        let plain = lines(&s, t);
        assert!(plain.iter().all(|l| l.1 > 300.0));
        // A square over the left half of the frame: lines next to it start right of it (+6 pt offset).
        let r = s.execute("shape.rectangle", &json!({"x": 0, "y": 50, "width": 200, "height": 100})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.set", &json!({"ids": [r]})).unwrap();
        s.execute("object.textWrap.make", &json!({})).unwrap();
        let wrapped = lines(&s, t);
        assert!(wrapped.iter().any(|l| l.0 >= 205.0), "{wrapped:?}");
        assert!(wrapped.iter().filter(|l| l.0 < 1.0).count() >= 2);
        // Moving the wrap object away re-flows the text.
        s.execute("object.move", &json!({"dx": 500, "dy": 0})).unwrap();
        assert!(lines(&s, t).iter().all(|l| l.0 < 1.0));
        s.execute("object.move", &json!({"dx": -500, "dy": 0})).unwrap();
        // Options round-trip; Invert Wrap keeps text inside the object only.
        assert_eq!(s.execute("object.textWrap.options", &json!({})).unwrap()["offset"], json!(6.0));
        s.execute("object.textWrap.options", &json!({"invert": true, "offset": 0})).unwrap();
        let inside = lines(&s, t);
        assert!(inside.iter().all(|l| l.1 <= 200.0 + 1e-6), "{inside:?}");
        // Release: back to the plain layout.
        s.execute("object.textWrap.release", &json!({})).unwrap();
        assert_eq!(lines(&s, t), plain);
        // Objects below the text don't wrap it.
        let below = s.execute("shape.rectangle", &json!({"x": 0, "y": 50, "width": 200, "height": 100})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.set", &json!({"ids": [below]})).unwrap();
        s.execute("object.textWrap.make", &json!({})).unwrap();
        s.execute("object.arrange.sendToBack", &json!({})).unwrap();
        assert_eq!(lines(&s, t), plain);
    }

    /// A negative offset from Text Wrap Options lets the lines beside a wrap object run that far
    /// into it, the options report it, and one undo puts back the offset before it.
    #[test]
    fn text_wrap_options_negative_offset_reflows_and_round_trips() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 600, "height": 600})).unwrap();
        let t = s
            .execute("text.create", &json!({"x": 0, "y": 0, "text": "word ".repeat(200), "size": 12, "area": {"width": 400, "height": 400}}))
            .unwrap()["id"]
            .as_u64()
            .unwrap();
        let r = s.execute("shape.rectangle", &json!({"x": -50, "y": 50, "width": 250, "height": 100})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.set", &json!({"ids": [r]})).unwrap();
        s.execute("object.textWrap.make", &json!({})).unwrap();
        let beside = |s: &Session| lines(s, t).into_iter().filter(|l| l.0 > 1.0).map(|l| l.0).collect::<Vec<_>>();
        assert!(!beside(&s).is_empty() && beside(&s).iter().all(|x| (x - 206.0).abs() < 1e-6), "{:?}", beside(&s));
        assert_eq!(s.execute("object.textWrap.options", &json!({"offset": -20})).unwrap()["offset"], json!(-20.0));
        assert!(!beside(&s).is_empty() && beside(&s).iter().all(|x| (x - 180.0).abs() < 1e-6), "{:?}", beside(&s));
        assert_eq!(s.execute("object.textWrap.options", &json!({})).unwrap()["offset"], json!(-20.0));
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.execute("object.textWrap.options", &json!({})).unwrap()["offset"], json!(6.0));
        assert!(beside(&s).iter().all(|x| (x - 206.0).abs() < 1e-6), "{:?}", beside(&s));
        // An offset of -1000 lets the text through the whole object.
        assert_eq!(s.execute("object.textWrap.options", &json!({"offset": -1000})).unwrap()["offset"], json!(-1000.0));
        assert!(lines(&s, t).iter().all(|l| l.0 < 1.0 && l.1 > 300.0));
    }

    /// An offset that is not a number or lies outside -1000 to 1000 pt, or an invert that is not
    /// true or false, is an error for Make and Options, and leaves the options and the history as
    /// they were. Omitted and null values take the default, and the range's ends are accepted.
    #[test]
    fn text_wrap_commands_refuse_bad_offsets_and_inverts_without_editing() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
        let id = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 80, "height": 80})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        let wrap = |s: &Session| s.doc().unwrap().doc.node(NodeId(id)).unwrap().extras().wrap;
        let undo = |s: &Session| s.doc().unwrap().history.undo.len();
        let refused = [
            (json!({"offset": "12"}), "`offset` must be a number"),
            (json!({"offset": true}), "`offset` must be a number"),
            (json!({"offset": [12]}), "`offset` must be a number"),
            (json!({"offset": {"pt": 12}}), "`offset` must be a number"),
            (json!({"offset": 1000.5}), "`offset` must be from -1000 to 1000"),
            (json!({"offset": -1000.5}), "`offset` must be from -1000 to 1000"),
            (json!({"offset": 5000}), "`offset` must be from -1000 to 1000"),
            (json!({"offset": 12, "invert": "true"}), "`invert` must be true or false"),
            (json!({"offset": 12, "invert": 1}), "`invert` must be true or false"),
            (json!({"invert": "false"}), "`invert` must be true or false"),
        ];
        let start = undo(&s);
        for (p, msg) in &refused {
            let e = s.execute("object.textWrap.make", p).unwrap_err().to_string();
            assert!(e.contains(msg), "make {p}: {e}");
            assert!(wrap(&s).is_none(), "make {p} made a wrap");
            assert_eq!(undo(&s), start, "make {p} added history");
        }
        // Omitted and null values: Make's 6 pt default.
        s.execute("object.textWrap.make", &json!({"offset": null, "invert": null})).unwrap();
        assert_eq!(wrap(&s), Some(vectorcraft_doc::TextWrap { offset: 6.0, invert: false }));
        s.execute("object.textWrap.options", &json!({"offset": 40, "invert": true})).unwrap();
        let before = undo(&s);
        for (p, msg) in &refused {
            let e = s.execute("object.textWrap.options", p).unwrap_err().to_string();
            assert!(e.contains(msg), "options {p}: {e}");
            assert_eq!(wrap(&s), Some(vectorcraft_doc::TextWrap { offset: 40.0, invert: true }), "options {p}");
            assert_eq!(undo(&s), before, "options {p} added history");
        }
        // Null options read the current ones without an undo step; a null one keeps its value.
        assert_eq!(s.execute("object.textWrap.options", &json!({"offset": null, "invert": null})).unwrap(), json!({"offset": 40.0, "invert": true}));
        assert_eq!(undo(&s), before);
        assert_eq!(
            s.execute("object.textWrap.options", &json!({"offset": null, "invert": false})).unwrap(),
            json!({"offset": 40.0, "invert": false})
        );
        // The range's ends are accepted by both commands.
        for (command, offset) in
            [("object.textWrap.options", 1000), ("object.textWrap.options", -1000), ("object.textWrap.make", 1000), ("object.textWrap.make", -1000)]
        {
            s.execute(command, &json!({ "offset": offset })).unwrap();
            assert_eq!(wrap(&s).unwrap().offset, f64::from(offset), "{command} {offset}");
        }
        // A stored offset outside the range (from a file) is kept when only Invert Wrap changes.
        let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
        d.node_mut(NodeId(id)).unwrap().extras_mut().wrap = Some(vectorcraft_doc::TextWrap { offset: 5000.0, invert: false });
        assert_eq!(s.execute("object.textWrap.options", &json!({"invert": true})).unwrap(), json!({"offset": 5000.0, "invert": true}));
    }

    #[test]
    fn text_flows_on_both_sides_of_a_centred_object() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 600, "height": 600})).unwrap();
        let t = s
            .execute("text.create", &json!({"x": 0, "y": 0, "text": "word ".repeat(300), "size": 12, "area": {"width": 400, "height": 400}}))
            .unwrap()["id"]
            .as_u64()
            .unwrap();
        let r = s.execute("shape.ellipse", &json!({"x": 150, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.set", &json!({"ids": [r]})).unwrap();
        s.execute("object.textWrap.make", &json!({})).unwrap();
        let Some(NodeKind::Text(tx)) = s.doc().unwrap().doc.node(NodeId(t)).map(|n| n.kind.clone()) else { panic!() };
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &tx);
        // Beside the circle each line band holds two lines: left of it and right of it.
        let beside: Vec<_> = lay.lines.iter().filter(|l| l.baseline > 120.0 && l.baseline < 180.0).collect();
        assert!(
            beside.iter().any(|l| l.avail.1 < 150.0) && beside.iter().any(|l| l.avail.0 > 250.0),
            "{:?}",
            beside.iter().map(|l| l.avail).collect::<Vec<_>>()
        );
        let left = beside.iter().find(|l| l.avail.1 < 150.0).unwrap();
        assert!(beside.iter().any(|l| l.avail.0 > 250.0 && (l.baseline - left.baseline).abs() < 1e-9), "same baseline on both sides");
        // Reading order: the right-hand line follows the left-hand one.
        let li = lay.lines.iter().position(|l| std::ptr::eq(l, *left)).unwrap();
        assert!(lay.lines[li + 1].avail.0 > 250.0);
    }
}
