//! Window › Info (`info.readout`): the selection's size and the active tool's readout, as the Info
//! panel shows them.

use serde_json::{Value, json};
use vectorcraft_render::effects;

use super::{CommandSpec, Result, Session, always, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "info.readout",
        "Info",
        [],
        None,
        "{} → {tool: the active tool's id, width?, height?: the sides of the selection's bounding box in pt (the box around the anchors selected with Direct Selection, when there are some), measured as prefs usePreviewBounds says; left out with nothing selected, readout?: what the Info panel shows for the tool in use: {kind: \"drag\", dx, dy, distance, angle} while the Selection, Direct Selection or Group Selection tool moves the selection or the Gradient tool draws a gradient vector on the art; {kind: \"scale\", width, height} (percent) during a Scale tool drag; {kind: \"rotate\", center: [x, y], angle: deg counter-clockwise, as object.rotate takes it} and {kind: \"reflect\", center, angle: the axis in deg counter-clockwise from horizontal, as object.reflect takes it} during a Rotate or Reflect tool drag; {kind: \"shear\", center, axis: 0 for a horizontal shear and 90 for a vertical one, angle: deg, as object.shear takes it} during a Shear tool drag; {kind: \"brush\", brush: name|null} while the Paintbrush is the tool; left out for any other tool and between drags. Lengths are in pt, a drag's angle is in deg counter-clockwise from the x axis, and center is the center of the selection's bounding box as the drag has carried it, in document coordinates}",
        always,
        readout
    )]
}

impl Session {
    /// The sides of the selection's bounding box as the Info panel shows them: of the box around
    /// the anchors selected with Direct Selection when there are some, else of the bounding box
    /// rotated with rotated objects ([`Session::transform_box`]), measured as Use Preview Bounds
    /// says. None with nothing selected.
    pub fn info_size(&self) -> Option<(f64, f64)> {
        let st = self.active()?;
        let anchors = effects::preview::anchor_bounds_with(&st.selection, &st.doc, self.bounds_measure());
        let r = anchors.or_else(|| self.transform_box(&st.selection.objects).map(|b| b.rect))?;
        Some((r.width(), r.height()))
    }
}

fn readout(s: &mut Session, _: &Value) -> Result<Value> {
    let mut out = json!({ "tool": s.tool_id() });
    if let Some((w, h)) = s.info_size() {
        out["width"] = json!(w);
        out["height"] = json!(h);
    }
    if let Some(r) = s.tool_readout() {
        out["readout"] = r.to_json();
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_tools::{PointerEvent, PointerKind};

    use crate::{Session, ViewInfo};

    fn view() -> ViewInfo {
        ViewInfo { smart_guides: false, snap_to_point: false, ..Default::default() }
    }

    /// A 100 by 60 rectangle at (50, 50), selected, with a 10 pt stroke.
    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
        let id = s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 60})).unwrap()["id"].clone();
        s.execute("select.set", &json!({ "ids": [id] })).unwrap();
        s.execute("stroke.set", &json!({"weight": 10})).unwrap();
        s
    }

    fn tool(s: &mut Session, id: &str) {
        s.select_tool(id, view()).unwrap();
    }

    fn pointer(s: &mut Session, kind: PointerKind, x: f64, y: f64) {
        s.pointer(&PointerEvent::new(kind, x, y), view()).unwrap();
    }

    fn info(s: &mut Session) -> serde_json::Value {
        s.execute("info.readout", &json!({})).unwrap()
    }

    fn near(v: &serde_json::Value, want: f64) -> bool {
        v.as_f64().is_some_and(|v| (v - want).abs() < 1e-6)
    }

    #[test]
    fn the_size_follows_use_preview_bounds() {
        let mut s = session();
        let i = info(&mut s);
        assert!(near(&i["width"], 100.0) && near(&i["height"], 60.0), "{i}");
        assert!(i.get("readout").is_none(), "{i}");
        s.execute("prefs.set", &json!({"key": "usePreviewBounds", "value": true})).unwrap();
        let i = info(&mut s);
        assert!(near(&i["width"], 110.0) && near(&i["height"], 70.0), "{i}");
        s.execute("select.none", &json!({})).unwrap();
        let i = info(&mut s);
        assert!(i.get("width").is_none() && i.get("height").is_none(), "{i}");
    }

    #[test]
    fn a_move_reads_how_far_it_has_gone_until_the_release() {
        let mut s = session();
        tool(&mut s, "selection");
        pointer(&mut s, PointerKind::Down, 100.0, 80.0);
        assert!(info(&mut s).get("readout").is_none());
        pointer(&mut s, PointerKind::Drag, 130.0, 40.0);
        let i = info(&mut s);
        let r = &i["readout"];
        assert_eq!(r["kind"], "drag", "{i}");
        assert!(near(&r["dx"], 30.0) && near(&r["dy"], -40.0) && near(&r["distance"], 50.0), "{i}");
        assert!(near(&r["angle"], 53.13010235415598), "{i}");
        pointer(&mut s, PointerKind::Up, 130.0, 40.0);
        let i = info(&mut s);
        assert!(i.get("readout").is_none(), "{i}");
        assert!(near(&i["width"], 100.0), "{i}");
    }

    #[test]
    fn each_transform_tool_reads_its_drag() {
        for (id, check) in [
            ("scale", (|r| r["kind"] == "scale" && near(&r["width"], 200.0) && near(&r["height"], 200.0)) as fn(&serde_json::Value) -> bool),
            ("rotate", |r| r["kind"] == "rotate" && near(&r["center"][0], 100.0) && near(&r["center"][1], 80.0) && near(&r["angle"], -90.0)),
            ("reflect", |r| r["kind"] == "reflect" && near(&r["center"][0], 100.0) && near(&r["angle"], 0.0)),
            ("shear", |r| {
                r["kind"] == "shear"
                    && near(&r["center"][0], 100.0)
                    && near(&r["center"][1], 80.0)
                    && near(&r["axis"], 0.0)
                    && near(&r["angle"], 45.0)
            }),
        ] {
            let mut s = session();
            tool(&mut s, id);
            // The reference point is the box's center (100, 80).
            let from = if id == "shear" { (100.0, 60.0) } else { (120.0, 100.0) };
            pointer(&mut s, PointerKind::Down, from.0, from.1);
            let to = match id {
                "scale" => (140.0, 120.0),
                "rotate" => (80.0, 100.0),
                "reflect" => (160.0, 80.0),
                _ => (120.0, 60.0),
            };
            pointer(&mut s, PointerKind::Drag, to.0, to.1);
            let i = info(&mut s);
            assert_eq!(i["tool"], id);
            assert!(check(&i["readout"]), "{id}: {i}");
            pointer(&mut s, PointerKind::Up, to.0, to.1);
            assert!(info(&mut s).get("readout").is_none(), "{id}");
        }
    }

    #[test]
    fn the_paintbrush_reads_its_brush() {
        let mut s = session();
        tool(&mut s, "paintbrush");
        let i = info(&mut s);
        assert_eq!(i["readout"], json!({"kind": "brush", "brush": null}), "{i}");
    }

    #[test]
    fn a_parameter_is_refused() {
        let mut s = session();
        let e = s.execute("info.readout", &json!({"tool": "scale"})).unwrap_err().to_string();
        assert!(e.contains("tool"), "{e}");
    }
}
