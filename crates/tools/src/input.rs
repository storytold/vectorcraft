use super::{Deserialize, Point, Serialize, Value};

/// Modifier keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mods {
    #[serde(default)]
    pub shift: bool,
    /// Option on macOS.
    #[serde(default)]
    pub alt: bool,
    /// Command on macOS, Ctrl elsewhere.
    #[serde(default)]
    pub cmd: bool,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub space: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PointerKind {
    /// Button pressed.
    Down,
    /// Moved with the button held.
    Drag,
    /// Button released.
    Up,
    /// Moved with no button (hover).
    Move,
    DoubleClick,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PointerEvent {
    pub kind: PointerKind,
    /// Document coordinates.
    pub pos: Point,
    #[serde(default)]
    pub mods: Mods,
    /// Whether backquote is held for this event.
    #[serde(default)]
    pub backquote: bool,
    #[serde(default = "one")]
    pub pressure: f32,
    /// The pen's tilt and barrel rotation, when the pointer is a pen that reads either (a mouse:
    /// `None`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pen: Option<PenPose>,
}

fn one() -> f32 {
    1.0
}

/// How a pen is held, as far as it reads it. Tilt is in degrees, each −90..90 as Pointer Events
/// give it, in document axes: `[x, y]`, x positive when the pen leans to the right and y positive
/// when it leans down the page. Rotation is the barrel's, in degrees clockwise (0..360).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PenPose {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tilt: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<f32>,
}

impl PenPose {
    /// Within its ranges: each tilt is clamped to −90..90 and the rotation brought into 0..360; a
    /// tilt or rotation that is not a number is left out.
    pub fn sanitized(self) -> Self {
        let tilt = self.tilt.filter(|t| t.iter().all(|v| v.is_finite())).map(|t| t.map(|v| v.clamp(-90.0, 90.0)));
        // `rem_euclid` can round a tiny negative angle up to 360 itself.
        let rotation = self.rotation.filter(|r| r.is_finite()).map(|r| r.rem_euclid(360.0)).map(|r| if r >= 360.0 { 0.0 } else { r });
        Self { tilt, rotation }
    }
}

impl PointerEvent {
    pub fn new(kind: PointerKind, x: f64, y: f64) -> Self {
        Self { kind, pos: Point::new(x, y), mods: Mods::default(), backquote: false, pressure: 1.0, pen: None }
    }
    pub fn with_mods(mut self, m: Mods) -> Self {
        self.mods = m;
        self
    }
    /// The pen `pressure` of a pointer event given as JSON (control channel, MCP): 0..1, default 1.
    pub fn json_pressure(e: &Value) -> f32 {
        e.get("pressure").and_then(Value::as_f64).filter(|f| f.is_finite()).map_or(1.0, |f| f.clamp(0.0, 1.0) as f32)
    }
    /// The pen's tilt and rotation of a pointer event given as JSON (control channel, MCP):
    /// `tiltX` and `tiltY` (degrees, −90..90, in document axes; one without the other is 0) and
    /// `rotation` (degrees clockwise, brought into 0..360). Values that are not numbers are left
    /// out; `None` without any.
    pub fn json_pen(e: &Value) -> Option<PenPose> {
        let num = |k: &str| e.get(k).and_then(Value::as_f64).filter(|f| f.is_finite());
        let tilt = |v: Option<f64>| v.map_or(0.0, |v| v.clamp(-90.0, 90.0) as f32);
        let (tx, ty) = (num("tiltX"), num("tiltY"));
        let tilt = (tx.is_some() || ty.is_some()).then(|| [tilt(tx), tilt(ty)]);
        let pose = PenPose { tilt, rotation: num("rotation").map(|r| r.rem_euclid(360.0) as f32) }.sanitized();
        (pose != PenPose::default()).then_some(pose)
    }
    /// How long the pointer then holds still, in seconds, from a JSON pointer event's `holdMs`
    /// (0..60000; default 0): the time [`Tool::tick`] gets.
    pub fn json_hold(e: &Value) -> f64 {
        e.get("holdMs").and_then(Value::as_f64).filter(|f| f.is_finite()).map_or(0.0, |ms| ms.clamp(0.0, 60_000.0) / 1000.0)
    }
    /// Whether backquote is held, from the `backquote` of a JSON pointer event or request (control
    /// channel, MCP): `default` when the key is absent, an error when it is not a boolean.
    pub fn json_backquote(v: &Value, default: bool) -> Result<bool, String> {
        v.get("backquote").map_or(Ok(default), |b| b.as_bool().ok_or_else(|| "`backquote` must be a boolean".to_owned()))
    }
    /// Checks the optional keys of a JSON pointer event (control channel, MCP): `pressure`,
    /// `holdMs`, `tiltX`, `tiltY` and `rotation` must be numbers, and `space` must be `doc` or
    /// `screen`. A key that is absent or null keeps its default.
    pub fn json_check(e: &Value) -> Result<(), String> {
        let given = |k: &str| e.get(k).filter(|v| !v.is_null());
        if let Some(k) = ["pressure", "holdMs", "tiltX", "tiltY", "rotation"].into_iter().find(|k| given(k).is_some_and(|v| v.as_f64().is_none())) {
            return Err(format!("`{k}` must be a number"));
        }
        if given("space").is_some_and(|v| !matches!(v.as_str(), Some("doc" | "screen"))) {
            return Err("`space` must be doc or screen".to_owned());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backquote_json_defaults_false_and_decodes_true() {
        let event = PointerEvent::new(PointerKind::Down, 10.0, 20.0);
        let mut value = serde_json::to_value(event).unwrap();
        value.as_object_mut().unwrap().remove("backquote");
        assert!(!serde_json::from_value::<PointerEvent>(value.clone()).unwrap().backquote);
        value["backquote"] = serde_json::json!(true);
        assert!(serde_json::from_value::<PointerEvent>(value).unwrap().backquote);
    }

    #[test]
    fn json_backquote_takes_the_default_only_when_absent() {
        use serde_json::json;
        assert_eq!(PointerEvent::json_backquote(&json!({}), true), Ok(true));
        assert_eq!(PointerEvent::json_backquote(&json!({"backquote": false}), true), Ok(false));
        assert_eq!(PointerEvent::json_backquote(&json!({"backquote": true}), false), Ok(true));
        for bad in [json!("true"), json!(1), json!(null)] {
            assert!(PointerEvent::json_backquote(&json!({"backquote": bad}), false).is_err(), "{bad}");
        }
    }

    #[test]
    fn json_check_refuses_pen_values_and_spaces_of_the_wrong_type() {
        use serde_json::json;
        for ok in [
            json!({}),
            json!({"pressure": 0.5, "holdMs": 10, "tiltX": -5, "tiltY": 5, "rotation": 400, "space": "screen"}),
            json!({"pressure": null, "space": null}),
            json!({"space": "doc"}),
        ] {
            assert_eq!(PointerEvent::json_check(&ok), Ok(()), "{ok}");
        }
        for k in ["pressure", "holdMs", "tiltX", "tiltY", "rotation"] {
            for bad in [json!("1"), json!(true), json!([1])] {
                assert_eq!(PointerEvent::json_check(&json!({k: bad})), Err(format!("`{k}` must be a number")), "{k}: {bad}");
            }
        }
        for bad in [json!("screeen"), json!(true), json!(1)] {
            assert_eq!(PointerEvent::json_check(&json!({"space": bad})), Err("`space` must be doc or screen".to_owned()), "{bad}");
        }
    }
}

/// Keys tools care about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolKey {
    Enter,
    Escape,
    Backspace,
    Delete,
    Up,
    Down,
    Left,
    Right,
    /// Illustrator: ↑/↓ while drawing a polygon/star change sides/points; `[`/`]` change sizes.
    BracketLeft,
    BracketRight,
    Tab,
    Home,
    End,
    /// A digit key 0–9 (5 while dragging with the Perspective Selection tool moves perpendicular
    /// to the plane).
    Digit(u8),
    /// A letter key a–z, lowercase (E while dragging a Selection or Group Selection marquee makes it
    /// select only the objects entirely inside it, and E again switches it back).
    Letter(char),
}

/// What a tool asks the engine to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Start an interaction (snapshot the document). The label becomes the undo name.
    Begin(String),
    /// Replace the current preview with this command, applied to the snapshot.
    Preview(String, Value),
    /// Finish the interaction: keep the last preview as one undo step.
    Commit,
    /// Abort the interaction: restore the snapshot.
    Cancel,
    /// Execute a command immediately (its own undo step, if it edits).
    Exec(String, Value),
    /// Ask the UI to open a dialog (e.g. click with the Rectangle tool → size dialog).
    Dialog(String, Value),
    /// Switch to another tool (e.g. after placing a text frame).
    SwitchTool(String),
    /// Call the tool's `notify` after the preceding actions ran (e.g. to pick up a created object).
    Notify(String),
}
