//! Desaturate: an example VectorCraft object filter plug-in (plug-in ABI v1, see
//! `docs/plugins.md`).
//!
//! It reads the selected paths as JSON, moves every colour (solid fills and strokes, gradient
//! stops, freeform gradient points) towards grey by `amount` percent, keeping each colour in its
//! own colour model, and hands the objects back without their `path`, so each keeps its geometry
//! exactly as it was. Only `serde_json` is used: no imports, no WASI.

use serde_json::{Value, json};

const MANIFEST: &str = r#"{
  "id": "org.vectorcraft.example.desaturate",
  "name": "Desaturate",
  "version": "1.0.0",
  "kind": "filter",
  "abi": 1,
  "author": "VectorCraft contributors",
  "description": "Moves the colours of the selected paths towards grey; an example of the plug-in ABI.",
  "params": {
    "amount": {"type": "number", "min": 0, "max": 100, "default": 100}
  }
}"#;

/// The manifest in the `vc_manifest` custom section, where the host reads it, with its `abi`
/// version, without running any of the module's code. The `vc_manifest` export below returns the
/// same text for hosts that read only the export.
#[used]
#[unsafe(link_section = "vc_manifest")]
static MANIFEST_SECTION: [u8; MANIFEST.len()] = section_bytes();

/// `MANIFEST` as a byte array, which `link_section` needs.
const fn section_bytes<const N: usize>() -> [u8; N] {
    let text = MANIFEST.as_bytes();
    let mut out = [0; N];
    let mut i = 0;
    while i < N {
        out[i] = text[i];
        i += 1;
    }
    out
}

#[unsafe(no_mangle)]
pub extern "C" fn vc_abi_version() -> u32 {
    1
}

/// `len << 32 | ptr` of the UTF-8 manifest JSON.
#[unsafe(no_mangle)]
pub extern "C" fn vc_manifest() -> u64 {
    pack(MANIFEST.as_bytes())
}

/// A block of `size` bytes the host writes the input and the parameters into. Never freed: every
/// run gets a fresh instance.
#[unsafe(no_mangle)]
pub extern "C" fn vc_alloc(size: u32) -> u32 {
    let mut block = Vec::<u8>::with_capacity(size as usize);
    let ptr = block.as_mut_ptr();
    std::mem::forget(block);
    ptr as u32
}

/// Runs the filter: `input` holds `{"objects": [...]}`, `params` the parameters. Returns the
/// output JSON as `len << 32 | ptr`, or a negative error code.
///
/// # Safety
/// The host guarantees both blocks came from `vc_alloc` and hold `*_len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vc_run(input: *const u8, input_len: u32, params: *const u8, params_len: u32) -> i64 {
    // SAFETY: see the function's contract.
    let (input, params) = unsafe { (std::slice::from_raw_parts(input, input_len as usize), std::slice::from_raw_parts(params, params_len as usize)) };
    let (Ok(mut doc), Ok(params)) = (serde_json::from_slice::<Value>(input), serde_json::from_slice::<Value>(params)) else { return -1 };
    let amount = (params["amount"].as_f64().unwrap_or(100.0) / 100.0).clamp(0.0, 1.0);
    let Some(objects) = doc["objects"].as_array_mut().filter(|o| !o.is_empty()) else {
        return output(&json!({"error": "Select some paths to desaturate."}));
    };
    // The geometry does not change, so each object goes back without its `path` and keeps the one
    // it has. A `path` parsed and written again can come back with a coordinate changed in its
    // last digit, which the host reads as a new path.
    for o in objects.iter_mut().filter_map(Value::as_object_mut) {
        o.remove("path");
    }
    desaturate(&mut doc, amount);
    output(&json!({"objects": doc["objects"]}))
}

/// Every colour (an object with a `model`) inside `v`, moved `k` of the way to grey.
fn desaturate(v: &mut Value, k: f64) {
    match v {
        Value::Object(o) => {
            let model = o.get("model").and_then(Value::as_str).map(str::to_string);
            let get = |key: &str| o.get(key).and_then(Value::as_f64).unwrap_or(0.0);
            let new: Option<Vec<(&str, f64)>> = match model.as_deref() {
                Some("rgb") => {
                    let (r, g, b) = (get("r"), get("g"), get("b"));
                    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                    Some(vec![("r", r + (y - r) * k), ("g", g + (y - g) * k), ("b", b + (y - b) * k)])
                }
                Some("cmyk") => {
                    let (c, m, y) = (get("c"), get("m"), get("y"));
                    let mean = (c + m + y) / 3.0;
                    Some(vec![("c", c + (mean - c) * k), ("m", m + (mean - m) * k), ("y", y + (mean - y) * k)])
                }
                Some("lab") => Some(vec![("a", get("a") * (1.0 - k)), ("b", get("b") * (1.0 - k))]),
                _ => None,
            };
            match new {
                Some(new) => {
                    for (key, x) in new {
                        o.insert(key.into(), json!(x));
                    }
                }
                None => o.values_mut().for_each(|c| desaturate(c, k)),
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|c| desaturate(c, k)),
        _ => {}
    }
}

/// Keeps `v` serialised in memory for the host to read, as `len << 32 | ptr`.
fn output(v: &Value) -> i64 {
    let Ok(bytes) = serde_json::to_vec(v) else { return -2 };
    let bytes = bytes.leak();
    pack(bytes) as i64
}

fn pack(bytes: &[u8]) -> u64 {
    ((bytes.len() as u64) << 32) | bytes.as_ptr() as u64
}
