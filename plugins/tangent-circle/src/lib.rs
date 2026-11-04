//! Tangent Circle: a sandboxed VectorCraft tool plug-in.

use serde_json::{Value, json};

const MANIFEST: &str = r#"{
  "id": "org.vectorcraft.tangent-circle",
  "name": "Tangent Circle",
  "version": "1.0.0",
  "kind": "tool",
  "toolGroup": "ellipse",
  "icon": "tool-ellipse",
  "author": "VectorCraft",
  "description": "Drag a circle to snap it into external tangency with two nearby circles."
}"#;

const MAX_CIRCLES: usize = 4_096;
const MAX_NEIGHBORS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Circle {
    x: f64,
    y: f64,
    radius: f64,
}

impl Circle {
    fn distance(self, x: f64, y: f64) -> f64 {
        (x - self.x).hypot(y - self.y)
    }

    fn finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.radius.is_finite() && self.radius > 0.0
    }
}

/// Possible external tangency centers for a moved circle against two fixed circles: the
/// intersections of `(a, moved.radius + a.radius)` and `(b, moved.radius + b.radius)`.
fn tangent_centers(moved: Circle, a: Circle, b: Circle) -> Option<Vec<(f64, f64)>> {
    if !(moved.finite() && a.finite() && b.finite()) {
        return None;
    }
    let (ra, rb) = (moved.radius + a.radius, moved.radius + b.radius);
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let d = dx.hypot(dy);
    if !d.is_finite() || d <= 1e-12 || d > ra + rb + 1e-9 || d < (ra - rb).abs() - 1e-9 {
        return None;
    }
    let along = (ra * ra - rb * rb + d * d) / (2.0 * d);
    let h2 = (ra * ra - along * along).max(0.0);
    let h = h2.sqrt();
    let (ux, uy) = (dx / d, dy / d);
    let (mx, my) = (a.x + along * ux, a.y + along * uy);
    let points = if h <= 1e-9 { vec![(mx, my)] } else { vec![(mx - uy * h, my + ux * h), (mx + uy * h, my - ux * h)] };
    points.into_iter().filter(|(x, y)| x.is_finite() && y.is_finite()).collect::<Vec<_>>().into()
}

fn number_pair(v: Option<&Value>) -> Option<(f64, f64)> {
    let v = v?;
    let (x, y) =
        if let Some([x, y]) = v.as_array().map(Vec::as_slice) { (x.as_f64()?, y.as_f64()?) } else { (v.get("x")?.as_f64()?, v.get("y")?.as_f64()?) };
    (x.is_finite() && y.is_finite()).then_some((x, y))
}

fn circles(v: Option<&Value>) -> Vec<(u64, Circle)> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_CIRCLES)
        .filter_map(|c| {
            let (x, y) = number_pair(c.get("center"))?;
            let circle = Circle { x, y, radius: c.get("radius")?.as_f64()? };
            let id = c.get("id")?.as_u64()?;
            circle.finite().then_some((id, circle))
        })
        .collect()
}

fn nearest_tangent(proposed: (f64, f64), moved_radius: f64, selected: u64, all: &[(u64, Circle)], tolerance: f64) -> Option<(f64, f64, [Circle; 2])> {
    let moved = Circle { x: proposed.0, y: proposed.1, radius: moved_radius };
    let mut near: Vec<(f64, Circle)> = all
        .iter()
        .filter(|(id, _)| *id != selected)
        .map(|(_, c)| (c.distance(moved.x, moved.y) - (moved.radius + c.radius), *c))
        .filter(|(error, _)| error.is_finite() && error.abs() <= tolerance)
        .collect();
    near.sort_by(|a, b| a.0.abs().total_cmp(&b.0.abs()));
    near.truncate(MAX_NEIGHBORS);
    let mut best: Option<(f64, f64, f64, [Circle; 2])> = None;
    for i in 0..near.len() {
        for j in i + 1..near.len() {
            let refs = [near[i].1, near[j].1];
            let Some(points) = tangent_centers(moved, refs[0], refs[1]) else { continue };
            for point in points {
                let error = (point.0 - proposed.0).hypot(point.1 - proposed.1);
                if error <= tolerance && best.as_ref().is_none_or(|b| error < b.0) {
                    best = Some((error, point.0, point.1, refs));
                }
            }
        }
    }
    best.map(|(_, x, y, refs)| (x, y, refs))
}

fn empty(state: Value, guides: Vec<Value>) -> Value {
    json!({"state": state, "actions": [], "guides": guides})
}

fn run_value(input: &Value) -> Value {
    let Some(event) = input.get("event") else { return json!({"error": "missing event"}) };
    let Some(kind) = event.get("kind").and_then(Value::as_str) else { return json!({"error": "missing event kind"}) };
    let Some((px, py)) = number_pair(event.get("pos")) else { return json!({"error": "event position must be finite"}) };
    let all = circles(input.get("circles"));
    let state = input.get("state").cloned().unwrap_or(Value::Null);
    let zoom = input.get("zoom").and_then(Value::as_f64).filter(|z| z.is_finite() && *z > 1e-9).unwrap_or(1.0);
    let tolerance = input.get("tolerance").and_then(Value::as_f64).filter(|t| t.is_finite() && *t >= 0.0).unwrap_or(4.0);

    if kind == "down" {
        let hit = all.iter().rev().find(|(_, c)| c.distance(px, py) <= c.radius + 2.0 / zoom);
        let Some((id, c)) = hit else {
            return json!({"state": null, "actions": [{"type":"exec","command":"select.set","params":{"ids":[]}}], "guides": []});
        };
        let state = json!({"selected": id, "start": [px, py], "grab": [c.x, c.y], "radius": c.radius, "began": false});
        return json!({"state": state, "actions": [{"type":"exec","command":"select.set","params":{"ids":[id]}}], "guides": []});
    }

    let Some(selected) = state.get("selected").and_then(Value::as_u64) else { return empty(Value::Null, vec![]) };
    let Some((start_x, start_y)) = number_pair(state.get("start")) else { return empty(Value::Null, vec![]) };
    let Some((grab_x, grab_y)) = number_pair(state.get("grab")) else { return empty(Value::Null, vec![]) };
    let Some(radius) = state.get("radius").and_then(Value::as_f64).filter(|r| r.is_finite() && *r > 0.0) else {
        return empty(Value::Null, vec![]);
    };
    let began = state.get("began").and_then(Value::as_bool).unwrap_or(false);

    if kind == "up" {
        let actions = if began { vec![json!({"type":"commit"})] } else { vec![] };
        return json!({"state": null, "actions": actions, "guides": []});
    }
    if kind != "drag" {
        return empty(state, vec![]);
    }
    if (px - start_x).hypot(py - start_y) < 2.0 / zoom {
        return empty(state, vec![]);
    }

    let proposed = (grab_x + px - start_x, grab_y + py - start_y);
    let snapped = nearest_tangent(proposed, radius, selected, &all, tolerance);
    let (center, guides) = if let Some((x, y, refs)) = snapped {
        let mut guides = Vec::with_capacity(4);
        for r in refs {
            let d = r.distance(x, y);
            if d <= 1e-12 {
                continue;
            }
            let (cx, cy) = (r.x + (x - r.x) * r.radius / d, r.y + (y - r.y) * r.radius / d);
            guides.push(json!({"type":"line","a":[x,y],"b":[cx,cy]}));
            guides.push(json!({"type":"label","p":[cx,cy],"text":"tangent"}));
        }
        ((x, y), guides)
    } else {
        (proposed, vec![])
    };
    let (dx, dy) = (center.0 - grab_x, center.1 - grab_y);
    let mut actions = Vec::with_capacity(2);
    if !began {
        actions.push(json!({"type":"begin","label":"Move"}));
    }
    actions.push(json!({"type":"preview","command":"object.transform","params":{"matrix":[1.0,0.0,0.0,1.0,dx,dy]}}));
    let next = json!({"selected": selected, "start": [start_x, start_y], "grab": [grab_x, grab_y], "radius": radius, "began": true});
    json!({"state": next, "actions": actions, "guides": guides})
}

#[unsafe(no_mangle)]
pub extern "C" fn vc_abi_version() -> u32 {
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn vc_manifest() -> u64 {
    pack(MANIFEST.as_bytes())
}

#[unsafe(no_mangle)]
pub extern "C" fn vc_alloc(size: u32) -> u32 {
    let mut block = Vec::<u8>::with_capacity(size as usize);
    let ptr = block.as_mut_ptr();
    std::mem::forget(block);
    ptr as u32
}

/// # Safety
/// The host passes two blocks returned by `vc_alloc`, each holding its stated number of bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vc_run(input: *const u8, input_len: u32, _params: *const u8, _params_len: u32) -> i64 {
    // SAFETY: the host's contract guarantees both pointers and lengths.
    let bytes = unsafe { std::slice::from_raw_parts(input, input_len as usize) };
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else { return -1 };
    let output = run_value(&value);
    let Ok(bytes) = serde_json::to_vec(&output) else { return -2 };
    let bytes = bytes.leak();
    pack(bytes) as i64
}

fn pack(bytes: &[u8]) -> u64 {
    ((bytes.len() as u64) << 32) | bytes.as_ptr() as u64
}

#[cfg(test)]
mod tests {
    use super::{Circle, nearest_tangent, run_value, tangent_centers};
    use serde_json::{Value, json};

    fn circle(id: u64, x: f64, y: f64, radius: f64) -> Value {
        json!({"id": id, "center": [x, y], "radius": radius})
    }

    fn event(kind: &str, x: f64, y: f64) -> Value {
        json!({"kind": kind, "pos": [x, y], "mods": {}})
    }

    fn input(event: Value, state: Value, selected: Vec<u64>, circles: Vec<Value>) -> Value {
        json!({"event": event, "state": state, "selection": selected, "circles": circles, "zoom": 1.0, "tolerance": 4.0})
    }

    #[test]
    fn pressing_a_circle_selects_it_and_remembers_the_grab() {
        let out =
            run_value(&input(event("down", 110.0, 100.0), Value::Null, vec![], vec![circle(1, 100.0, 100.0, 10.0), circle(2, 200.0, 100.0, 20.0)]));
        assert_eq!(out["state"]["selected"], 1);
        assert_eq!(out["state"]["grab"], json!([100.0, 100.0]));
        assert_eq!(out["actions"], json!([{"type":"exec","command":"select.set","params":{"ids":[1]}}]));
    }

    #[test]
    fn a_neighbor_ring_does_not_take_a_press_at_the_moved_circles_center() {
        let out = run_value(&input(
            event("down", 100.0, 100.0),
            Value::Null,
            vec![],
            vec![circle(1, 100.0, 100.0, 10.0), circle(2, 80.0, 80.0, 20.0), circle(3, 120.0, 120.0, 20.0)],
        ));
        assert_eq!(out["state"]["selected"], 1);
    }

    #[test]
    fn dragging_to_the_two_circle_solution_keeps_the_radius_and_previews_the_exact_translation() {
        let down = run_value(&input(
            event("down", 100.0, 110.0),
            Value::Null,
            vec![],
            vec![circle(1, 100.0, 100.0, 10.0), circle(2, 80.0, 100.0, 20.0), circle(3, 120.0, 100.0, 20.0)],
        ));
        let out = run_value(&input(
            event("drag", 101.0, 134.0),
            down["state"].clone(),
            vec![1],
            vec![circle(1, 100.0, 100.0, 10.0), circle(2, 80.0, 100.0, 20.0), circle(3, 120.0, 100.0, 20.0)],
        ));
        let all = vec![
            (1, Circle { x: 100.0, y: 100.0, radius: 10.0 }),
            (2, Circle { x: 80.0, y: 100.0, radius: 20.0 }),
            (3, Circle { x: 120.0, y: 100.0, radius: 20.0 }),
        ];
        assert!(nearest_tangent((101.0, 124.0), 10.0, 1, &all, 4.0).is_some(), "the solver finds the expected pair");
        assert_eq!(out["actions"][0]["type"], "begin");
        assert_eq!(out["actions"][1]["command"], "object.transform");
        let matrix = out["actions"][1]["params"]["matrix"].as_array().expect("matrix");
        assert!(matrix[4].as_f64().expect("dx").abs() < 1e-9, "{matrix:?}");
        assert!((matrix[5].as_f64().expect("dy") - 22.360_679_774_998).abs() < 1e-9);
        assert_eq!(out["guides"].as_array().map(Vec::len), Some(4), "two lines and two labels");
    }

    #[test]
    fn unreachable_references_leave_the_pointer_drag_unsnapped() {
        let down = run_value(&input(
            event("down", 0.0, 0.0),
            Value::Null,
            vec![],
            vec![circle(1, 0.0, 0.0, 10.0), circle(2, 500.0, 0.0, 20.0), circle(3, 1000.0, 0.0, 20.0)],
        ));
        let out = run_value(&input(
            event("drag", 10.0, 20.0),
            down["state"].clone(),
            vec![1],
            vec![circle(1, 0.0, 0.0, 10.0), circle(2, 500.0, 0.0, 20.0), circle(3, 1000.0, 0.0, 20.0)],
        ));
        assert_eq!(out["actions"][1]["params"]["matrix"], json!([1.0, 0.0, 0.0, 1.0, 10.0, 20.0]));
        assert_eq!(out["guides"], json!([]));
    }

    #[test]
    fn tangent_centers_are_exact_and_degenerate_pairs_give_none() {
        let moved = Circle { x: 0.0, y: 0.0, radius: 10.0 };
        let a = Circle { x: 0.0, y: 100.0, radius: 20.0 };
        let b = Circle { x: 40.0, y: 100.0, radius: 20.0 };
        let points = tangent_centers(moved, a, b).unwrap();
        assert_eq!(points.len(), 2);
        for p in points {
            assert!((((p.0 - a.x).powi(2) + (p.1 - a.y).powi(2)).sqrt() - 30.0).abs() < 1e-9);
            assert!((((p.0 - b.x).powi(2) + (p.1 - b.y).powi(2)).sqrt() - 30.0).abs() < 1e-9);
        }
        assert!(tangent_centers(moved, Circle { x: 500.0, y: 0.0, radius: 20.0 }, Circle { x: 1000.0, y: 0.0, radius: 20.0 }).is_none());
    }
}
