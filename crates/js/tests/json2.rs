//! Runs `json2.js`, Douglas Crockford's public-domain JSON polyfill, when `cargo xtask test262` has
//! fetched it (it sets `VECTORCRAFT_JSON2`; without it the test looks in `corpus/json2/` and skips
//! when the file is absent).
//!
//! The built-in `JSON` is removed first, so the polyfill defines its own. Each sample value is
//! written by the polyfill and by the built-in `JSON.stringify` (ECMA-262 5.1 section 15.12.3),
//! which must give the same text, and the text is read back by the polyfill's `parse` and by the
//! built-in `JSON.parse` (section 15.12.2), which must give values that write the same text again.
//!
//! The polyfill uses `Date`, `String.prototype.replace` and regular expression `test`. While the
//! interpreter lacks any of them, the test prints that it skipped and passes.

use std::path::{Path, PathBuf};

use vectorcraft_js::{JsValue, Realm};

fn json2_file() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("VECTORCRAFT_JSON2") {
        return Some(PathBuf::from(path));
    }
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/json2");
    let mut dirs: Vec<PathBuf> =
        std::fs::read_dir(corpus).ok()?.filter_map(|e| e.ok().map(|e| e.path().join("json2.js"))).filter(|p| p.is_file()).collect();
    dirs.sort();
    dirs.pop()
}

/// Built-ins the polyfill needs.
const NEEDS: &str =
    "typeof Date === 'function' && typeof ''.replace === 'function' && typeof RegExp === 'function' && typeof /a/.test === 'function'";

/// Saves the built-in `JSON` and removes it, so the polyfill defines its own.
const BEFORE: &str = "var NATIVE = JSON; delete this.JSON; this.JSON = undefined;";

/// Writes and reads each sample with both implementations; gives the first difference, or "ok".
const ROUND_TRIP: &str = r#"
var POLY = JSON;
var samples = [
    0, -1.5, 1e21, "", "plain", "quote \" backslash \\ newline \n tab \t \u0001   é",
    true, false, null, [], {}, [1, [2, [3, []]]], {a: 1, b: [true, null, "x"], c: {d: {}}},
    [undefined, function () {}], {skip: undefined, f: function () {}, keep: 2}, [NaN, Infinity, -Infinity]
];
var problem = "ok";
for (var i = 0; i < samples.length && problem === "ok"; i++) {
    var poly = POLY.stringify(samples[i]);
    var native = NATIVE.stringify(samples[i]);
    if (poly !== native) {
        problem = "sample " + i + ": json2 wrote " + poly + ", the built-in wrote " + native;
    } else if (poly !== undefined) {
        if (NATIVE.stringify(POLY.parse(native)) !== native) {
            problem = "sample " + i + ": json2 read " + native + " differently";
        } else if (POLY.stringify(NATIVE.parse(poly)) !== poly) {
            problem = "sample " + i + ": the built-in read " + poly + " differently";
        }
    }
}
var indented = {a: [1, {b: 2}], c: "d"};
if (problem === "ok" && POLY.stringify(indented, null, 2) !== NATIVE.stringify(indented, null, 2)) {
    problem = "indented output differs";
}
if (problem === "ok" && POLY.stringify(indented, ["c"]) !== NATIVE.stringify(indented, ["c"])) {
    problem = "property list output differs";
}
problem;
"#;

#[test]
fn json2_polyfill_round_trips_with_the_built_in_json() {
    let Some(path) = json2_file() else {
        println!("json2: SKIPPED (no json2.js; run cargo xtask test262)");
        return;
    };
    let source = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut realm = Realm::new();
    if realm.run(NEEDS) != Ok(JsValue::Bool(true)) {
        println!("json2: SKIPPED (the interpreter lacks Date, String.prototype.replace or RegExp, which json2.js uses)");
        return;
    }
    match realm.run(&format!("{BEFORE}\n{source}\n{ROUND_TRIP}")) {
        Ok(JsValue::String(s)) if s == "ok" => println!("json2: the polyfill and the built-in JSON agree on every sample"),
        other => panic!("json2: {other:?}"),
    }
}
