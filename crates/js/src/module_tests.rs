use std::collections::{HashMap, HashSet};

use crate::{
    JsValue, LimitKind, Limits, LoadError, LoadedModule, ModuleLimits, Options, Progress, Realm, RunError, RunErrorKind, parse, parse_module,
};

fn es() -> Realm {
    let mut realm = Realm::with_options(Options { es2015: true, ..Options::default() }, Limits::default());
    // Modules declare no globals, so they report through this array.
    assert!(realm.run("var out = [];").is_ok());
    realm
}

/// A loader over `files`: each specifier is its own key, given with its source the first time.
fn loader(files: &[(&str, &str)]) -> impl FnMut(&str, &str) -> Result<LoadedModule, LoadError> + use<> {
    let map: HashMap<String, String> = files.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let mut given = HashSet::new();
    move |_, specifier| {
        if !specifier.starts_with('.') && specifier != "main" {
            return Err(LoadError::Syntax(format!("bare specifier: {specifier}")));
        }
        let source = map.get(specifier).ok_or_else(|| LoadError::Other(format!("module not found: {specifier}")))?;
        let first = given.insert(specifier.to_string());
        Ok(LoadedModule { key: specifier.to_string(), source: first.then(|| source.clone()) })
    }
}

/// Runs `entry` with the modules `files`. → the result and `out` joined with `|`.
fn run_with(realm: &mut Realm, entry: &str, files: &[(&str, &str)], limits: ModuleLimits) -> (Result<JsValue, RunError>, String) {
    let mut load = loader(files);
    let result = realm.run_module("main.mjs", entry, limits, &mut load);
    let out = match realm.run("out.join('|')") {
        Ok(JsValue::String(s)) => s,
        other => panic!("{other:?}"),
    };
    (result, out)
}

fn run(entry: &str, files: &[(&str, &str)]) -> String {
    match run_with(&mut es(), entry, files, ModuleLimits::default()) {
        (Ok(v), out) => {
            assert_eq!(v, JsValue::Undefined);
            out
        }
        (Err(e), _) => panic!("{entry}\n{e:?}"),
    }
}

/// The run ends with an error whose message starts with `start`, and `out` holds `ran`.
fn fails(entry: &str, files: &[(&str, &str)], start: &str, ran: &str) -> RunError {
    match run_with(&mut es(), entry, files, ModuleLimits::default()) {
        (Err(e), out) => {
            assert!(e.message.starts_with(start), "{entry}: {e:?}");
            assert_eq!(out, ran, "{entry}");
            e
        }
        (Ok(_), out) => panic!("{entry}: completed with {out}"),
    }
}

fn syntax(src: &str) {
    assert!(parse_module(src, &Options::default()).is_err(), "{src}");
}

#[test]
fn an_import_reads_the_exporting_binding_each_time() {
    let counter = "export let count = 0; export function inc() { count++; }";
    let out = run("import { count, inc } from './c.mjs'; out.push(count); inc(); inc(); out.push(count);", &[("./c.mjs", counter)]);
    assert_eq!(out, "0|2");
}

#[test]
fn imports_can_be_renamed_and_combined() {
    let m = "export default 'd'; export var a = 1; export const b = 2;";
    let out = run("import d, { a as x, b } from './m.mjs'; out.push(d, x, b);", &[("./m.mjs", m)]);
    assert_eq!(out, "d|1|2");
    let out = run("import d, * as ns from './m.mjs'; out.push(d, ns.a, ns['default']);", &[("./m.mjs", m)]);
    assert_eq!(out, "d|1|d");
}

#[test]
fn default_exports_of_expressions_and_functions() {
    let out = run("import v from './v.mjs'; out.push(v * 2);", &[("./v.mjs", "export default 20 + 1;")]);
    assert_eq!(out, "42");
    let out = run("import f from './f.mjs'; out.push(f());", &[("./f.mjs", "export default function () { return 'anon'; }")]);
    assert_eq!(out, "anon");
    // A named default function is a declaration: hoisted, and its name is bound in the module.
    let f = "out.push(typeof g); export default function g() { return 'g'; }";
    let out = run("import h from './f.mjs'; out.push(h());", &[("./f.mjs", f)]);
    assert_eq!(out, "function|g");
}

#[test]
fn a_namespace_object_has_the_exports_in_order_and_takes_no_writes() {
    let m = "export var zeta = 1; export let alpha = 2; export function mid() {} export default 3;";
    let out = run(
        "import * as ns from './m.mjs';
         out.push(Object.keys(ns).join(','), Object.prototype.toString.call(ns), Object.isExtensible(ns), Object.getPrototypeOf(ns) === null);
         try { ns.alpha = 5; } catch (e) { out.push(e.name); }
         try { ns.added = 5; } catch (e) { out.push(e.name); }
         try { delete ns.alpha; } catch (e) { out.push(e.name); }
         out.push(ns.alpha, Object.getOwnPropertyDescriptor(ns, 'zeta').writable);",
        &[("./m.mjs", m)],
    );
    assert_eq!(out, "alpha,default,mid,zeta|[object Module]|false|true|TypeError|TypeError|TypeError|2|true");
}

#[test]
fn a_namespace_takes_only_definitions_that_change_nothing() {
    let m = "export var a = 1;";
    let out = run(
        "import * as ns from './m.mjs';
         out.push(Object.seal(ns) === ns, Object.isSealed(ns), Object.isFrozen(ns));
         out.push(Object.defineProperty(ns, 'a', { value: 1, writable: true, enumerable: true, configurable: false }) === ns);
         var ds = [{ value: 2 }, { writable: false }, { enumerable: false }, { configurable: true }, { get: function () {} }];
         for (var i = 0; i < ds.length; i++) {
           try { Object.defineProperty(ns, 'a', ds[i]); out.push('defined'); } catch (e) { out.push(e.name); }
         }
         try { Object.defineProperty(ns, 'b', { value: 1 }); } catch (e) { out.push(e.name); }
         try { Object.freeze(ns); } catch (e) { out.push(e.name); }
         out.push(ns.a);",
        &[("./m.mjs", m)],
    );
    assert_eq!(out, "true|true|false|true|TypeError|TypeError|TypeError|TypeError|TypeError|TypeError|TypeError|1");
}

#[test]
fn a_namespace_reads_live_values() {
    let m = "export var n = 1; export function bump() { n = n + 1; }";
    let out = run("import * as ns from './m.mjs'; ns.bump(); out.push(ns.n);", &[("./m.mjs", m)]);
    assert_eq!(out, "2");
}

#[test]
fn assigning_to_an_import_is_a_type_error() {
    let m = "export var a = 1;";
    let out = run("import { a } from './m.mjs'; try { a = 2; } catch (e) { out.push(e.name); } out.push(a);", &[("./m.mjs", m)]);
    assert_eq!(out, "TypeError|1");
    let out = run("import * as ns from './m.mjs'; try { ns = 2; } catch (e) { out.push(e.name); }", &[("./m.mjs", m)]);
    assert_eq!(out, "TypeError");
}

#[test]
fn a_cycle_of_two_modules_runs_each_once_dependency_first() {
    // main imports b, b imports main back: b runs first and sees main's hoisted function.
    let b = "import { fa } from 'main'; out.push('b ' + fa()); export const b = 'B';";
    let mut realm = es();
    let mut load = loader(&[("./b.mjs", b), ("main", "")]);
    let mut load = |referrer: &str, specifier: &str| {
        if specifier == "main" { Ok(LoadedModule { key: "main.mjs".into(), source: None }) } else { load(referrer, specifier) }
    };
    let entry = "import { b } from './b.mjs'; export function fa() { return 'fa'; } out.push('a ' + b);";
    let r = realm.run_module("main.mjs", entry, ModuleLimits::default(), &mut load);
    assert_eq!(r, Ok(JsValue::Undefined));
    assert_eq!(realm.run("out.join('|')"), Ok(JsValue::String("b fa|a B".into())));
}

#[test]
fn reading_an_import_before_its_module_ran_is_a_reference_error() {
    let b = "import { a } from 'main'; out.push('b'); out.push(a);";
    let mut realm = es();
    let mut inner = loader(&[("./b.mjs", b)]);
    let mut load = |referrer: &str, specifier: &str| {
        if specifier == "main" { Ok(LoadedModule { key: "main.mjs".into(), source: None }) } else { inner(referrer, specifier) }
    };
    let r = realm.run_module("main.mjs", "import './b.mjs'; export let a = 1;", ModuleLimits::default(), &mut load);
    match r {
        Err(e) => assert!(e.message.starts_with("ReferenceError"), "{e:?}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(realm.run("out.join('|')"), Ok(JsValue::String("b".into())));
}

#[test]
fn a_namespace_export_its_module_has_not_initialized_is_a_reference_error() {
    let b = "import * as ns from 'main';
        var tries = [() => ns.a, () => Object.keys(ns), () => Object.prototype.hasOwnProperty.call(ns, 'a'), () => Object.prototype.propertyIsEnumerable.call(ns, 'a'),
            () => Object.getOwnPropertyDescriptor(ns, 'a'), () => { for (var k in ns) {} }, () => Object.isSealed(ns), () => Object.isFrozen(ns),
            () => Object.seal(ns), () => Object.freeze(ns), () => Object.defineProperties({}, ns), () => JSON.stringify(ns)];
        tries.forEach(function (f) { try { f(); out.push('no error'); } catch (e) { out.push(e.name); } });
        out.push('a' in ns, Object.getOwnPropertyNames(ns).join());";
    let mut realm = es();
    let mut inner = loader(&[("./b.mjs", b)]);
    let mut load = |referrer: &str, specifier: &str| {
        if specifier == "main" { Ok(LoadedModule { key: "main.mjs".into(), source: None }) } else { inner(referrer, specifier) }
    };
    let r = realm.run_module("main.mjs", "import './b.mjs'; export let a = 1;", ModuleLimits::default(), &mut load);
    assert_eq!(r, Ok(JsValue::Undefined));
    let errors = "ReferenceError|".repeat(12);
    assert_eq!(realm.run("out.join('|')"), Ok(JsValue::String(format!("{errors}true|a"))));
}

#[test]
fn a_module_two_modules_import_runs_once() {
    let shared = "out.push('shared'); export var s = 1;";
    let a = "import { s } from './shared.mjs'; out.push('a' + s);";
    let b = "import { s } from './shared.mjs'; out.push('b' + s);";
    let out = run("import './a.mjs'; import './b.mjs'; out.push('main');", &[("./shared.mjs", shared), ("./a.mjs", a), ("./b.mjs", b)]);
    assert_eq!(out, "shared|a1|b1|main");
}

#[test]
fn re_exports_pass_names_on() {
    let base = "export var x = 1, y = 2; export default 'base';";
    let mid = "export { x as renamed } from './base.mjs'; export * from './base.mjs'; export * as all from './base.mjs';";
    let out = run(
        "import { renamed, x, y, all } from './mid.mjs'; import * as m from './mid.mjs'; out.push(renamed, x, y, all['default'], Object.keys(m).join(','));",
        &[("./base.mjs", base), ("./mid.mjs", mid)],
    );
    // `export *` does not pass `default` on.
    assert_eq!(out, "1|1|2|base|all,renamed,x,y");
}

#[test]
fn an_exported_import_is_a_re_export() {
    let base = "export var v = 'v';";
    let mid = "import { v } from './base.mjs'; import * as ns from './base.mjs'; export { v as w, ns };";
    let out = run("import { w, ns } from './mid.mjs'; out.push(w, ns.v);", &[("./base.mjs", base), ("./mid.mjs", mid)]);
    assert_eq!(out, "v|v");
}

#[test]
fn a_name_two_stars_give_is_ambiguous() {
    let files =
        [("./a.mjs", "export var n = 1;"), ("./b.mjs", "export var n = 2;"), ("./both.mjs", "export * from './a.mjs'; export * from './b.mjs';")];
    fails("import { n } from './both.mjs'; out.push('ran');", &files, "SyntaxError: the export 'n'", "");
    // The namespace leaves the name out.
    let out = run("import * as ns from './both.mjs'; out.push(Object.keys(ns).length);", &files);
    assert_eq!(out, "0");
}

#[test]
fn a_missing_export_ends_the_run_before_any_module_runs() {
    let files = [("./m.mjs", "out.push('m'); export var a = 1;")];
    let e = fails("import { nope } from './m.mjs';\nout.push('main');", &files, "SyntaxError: the module './m.mjs' has no export named 'nope'", "");
    assert_eq!((e.line, e.column), (1, 10));
    let files = [("./m.mjs", "export { nothing as gone } from './n.mjs';"), ("./n.mjs", "")];
    fails("import './m.mjs';", &files, "SyntaxError: the module './n.mjs' has no export to give as 'gone'", "");
}

#[test]
fn a_module_the_loader_cannot_give_is_named_in_the_error() {
    let e = fails("out.push(1);\nimport { a } from './missing.mjs';", &[], "Error: module not found: ./missing.mjs", "");
    assert_eq!(e.line, 2);
    fails("import x from 'lib';", &[], "SyntaxError: bare specifier: lib", "");
}

#[test]
fn a_syntax_error_in_an_imported_module_names_its_source() {
    let mut realm = es();
    let mut load = loader(&[("./a.mjs", "out.push('a');"), ("./bad.mjs", "\nvar = ;")]);
    let r = realm.run_module("main.mjs", "import './a.mjs'; import './bad.mjs';", ModuleLimits::default(), &mut load);
    let e = r.err().unwrap_or_else(|| panic!("ran"));
    assert_eq!((e.kind, e.line), (RunErrorKind::Syntax, 2));
    // The entry is source 0, then each module in the order it was loaded.
    assert_eq!(realm.error_source(), 2);
    assert_eq!(realm.run("out.length"), Ok(JsValue::Number(0.0)));
}

#[test]
fn module_code_is_strict_and_its_this_is_undefined() {
    let out = run("out.push(this === undefined, (() => this)() === undefined); try { undeclared = 1; } catch (e) { out.push(e.name); }", &[]);
    assert_eq!(out, "true|true|ReferenceError");
    // Its declarations stay in the module.
    let mut realm = es();
    let (r, _) = run_with(&mut realm, "var hidden = 1; function also() {}", &[], ModuleLimits::default());
    assert_eq!(r, Ok(JsValue::Undefined));
    assert_eq!(realm.run("typeof hidden + typeof also"), Ok(JsValue::String("undefinedundefined".into())));
}

#[test]
fn an_exception_in_a_module_ends_the_run_and_the_modules_after_it_do_not_run() {
    let files = [("./a.mjs", "out.push('a'); throw new TypeError('stop');"), ("./b.mjs", "try { out.push('b'); } catch (e) { out.push('caught'); }")];
    fails("import './a.mjs'; import './b.mjs'; try { out.push('main'); } catch (e) {}", &files, "TypeError: stop", "a");
}

#[test]
fn the_module_count_and_import_depth_are_limited() {
    let files = [("./a.mjs", "import './b.mjs';"), ("./b.mjs", "import './c.mjs';"), ("./c.mjs", "out.push('c');")];
    let mut realm = es();
    let (r, out) = run_with(&mut realm, "import './a.mjs';", &files, ModuleLimits { modules: 3, depth: 16 });
    assert!(r.as_ref().is_err_and(|e| e.message.starts_with("RangeError: module limit (3) reached at './c.mjs'")), "{r:?}");
    assert_eq!(out, "");
    let (r, _) = run_with(&mut es(), "import './a.mjs';", &files, ModuleLimits { modules: 64, depth: 2 });
    assert!(r.as_ref().is_err_and(|e| e.message.starts_with("RangeError: import depth limit (2) reached at './c.mjs'")), "{r:?}");
    let (r, out) = run_with(&mut es(), "import './a.mjs';", &files, ModuleLimits { modules: 4, depth: 3 });
    assert_eq!((r, out.as_str()), (Ok(JsValue::Undefined), "c"));
}

#[test]
fn a_module_at_the_depth_limit_may_import_a_module_already_loaded() {
    let files = [("./a.mjs", "import './b.mjs'; out.push('a');"), ("./b.mjs", "import './a.mjs'; out.push('b');")];
    let (r, out) = run_with(&mut es(), "import './a.mjs';", &files, ModuleLimits { modules: 64, depth: 2 });
    assert_eq!((r, out.as_str()), (Ok(JsValue::Undefined), "b|a"));
}

#[test]
fn a_module_limit_of_zero_runs_no_module() {
    let (r, out) = run_with(&mut es(), "out.push('main');", &[], ModuleLimits { modules: 0, depth: 16 });
    assert!(r.as_ref().is_err_and(|e| e.message.starts_with("RangeError: module limit (0) reached")), "{r:?}");
    assert_eq!(out, "");
}

#[test]
fn a_module_run_is_held_to_the_fuel_limit() {
    let mut realm = Realm::with_options(Options { es2015: true, ..Options::default() }, Limits { fuel: 50, ..Limits::default() });
    let mut load = loader(&[("./m.mjs", &"var a = 1;\n".repeat(20))]);
    let r = realm.run_module("main.mjs", "import './m.mjs';", ModuleLimits::default(), &mut load);
    assert_eq!(r.map_err(|e| e.kind), Err(RunErrorKind::Limit(LimitKind::Fuel)));
}

#[test]
fn a_module_run_advances_in_slices_and_runs_each_module_in_turn() {
    let mut realm = es();
    let mut load = loader(&[("./a.mjs", "for (var i = 0; i < 200; i++) {} out.push('a');")]);
    let entry = "import './a.mjs'; for (var j = 0; j < 200; j++) {} out.push('main');";
    assert_eq!(realm.start_module("main.mjs", entry, ModuleLimits::default(), &mut load), Ok(()));
    let mut slices = 0;
    loop {
        match realm.advance(10) {
            Ok(Progress::Paused) => slices += 1,
            Ok(Progress::Done(v)) => {
                assert_eq!(v, JsValue::Undefined);
                break;
            }
            Err(e) => panic!("{e:?}"),
        }
    }
    assert!(slices > 20, "{slices}");
    assert_eq!(realm.run("out.join('|')"), Ok(JsValue::String("a|main".into())));
}

#[test]
fn an_es5_realm_runs_no_module() {
    let mut realm = Realm::new();
    let mut load = loader(&[]);
    let r = realm.run_module("main.mjs", "export var a = 1;", ModuleLimits::default(), &mut load);
    assert_eq!(r.map_err(|e| e.kind), Err(RunErrorKind::Syntax));
}

#[test]
fn a_script_does_not_read_import_or_export() {
    for src in ["import a from './a.mjs';", "export var a = 1;", "import './a.mjs';"] {
        assert!(parse(src, &Options { es2015: true, ..Options::default() }).is_err(), "{src}");
        assert!(es().run(src).is_err_and(|e| e.kind == RunErrorKind::Syntax), "{src}");
    }
}

#[test]
fn module_early_errors() {
    for src in [
        "export var a; export { a };",
        "export { undeclared };",
        "export default 1; export default 2;",
        "import { a } from './x.mjs'; var a;",
        "var a; import a from './x.mjs';",
        "import { a, a } from './x.mjs';",
        "function f() { import a from './x.mjs'; }",
        "{ export var a; }",
        "import { default } from './x.mjs';",
        "export { default };",
        "import a from x;",
        "import * from './x.mjs';",
        "with (a) {}",
        "export default class {}",
        "import('./x.mjs');",
        "#include 'x.js'\n",
        // Top-level functions are lexical declarations in a module.
        "function f() {} function f() {}",
        "var f; function f() {}",
        "function f() {} var f;",
        "let f; function f() {}",
    ] {
        syntax(src);
    }
    for src in [
        "import { default as d, if as i } from './x.mjs'; export { d as default, i as if };",
        "import {} from './x.mjs'; export {};",
        "export function f() {} export { f as g };",
        "var as = 1, from = 2; export { as, from };",
        "export * as default from './x.mjs';",
        "function f() {} { function f() {} } function g() { var g; function h() {} var h; }",
    ] {
        assert!(parse_module(src, &Options::default()).is_ok(), "{src}");
    }
}

#[test]
fn a_module_lists_what_it_imports_and_exports() {
    let (_, info) =
        match parse_module("import a, { b as c } from './x.mjs'; export { c as d }; export * from './y.mjs'; import './x.mjs';", &Options::default())
        {
            Ok(p) => p,
            Err(e) => panic!("{e:?}"),
        };
    let specifiers: Vec<&str> = info.requests.iter().map(|r| r.specifier.as_str()).collect();
    assert_eq!(specifiers, ["./x.mjs", "./y.mjs"]);
    assert_eq!(info.imports.len(), 2);
    assert!(
        matches!(&info.exports[..], [crate::ExportEntry::Star { request: 1, .. }, crate::ExportEntry::Indirect { export_name, request: 0, .. }] if export_name == "d")
    );
}

#[test]
fn modules_survive_a_collection_while_they_link() {
    let mut realm = Realm::with_options(Options { es2015: true, ..Options::default() }, Limits { heap_bytes: 64 << 20, ..Limits::default() });
    assert!(realm.run("var out = []; var junk = []; for (var i = 0; i < 2000; i++) junk.push({ i: i }); junk = null;").is_ok());
    let m = "export var v = 'kept'; export function f() { return v; }";
    let mut load = loader(&[("./m.mjs", m)]);
    let r = realm.run_module(
        "main.mjs",
        "import * as ns from './m.mjs'; import { f } from './m.mjs'; out.push(ns.f(), f());",
        ModuleLimits::default(),
        &mut load,
    );
    assert_eq!(r, Ok(JsValue::Undefined));
    realm.collect_garbage();
    assert_eq!(realm.run("out.join('|')"), Ok(JsValue::String("kept|kept".into())));
}
