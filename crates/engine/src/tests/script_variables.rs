//! Variables and data sets in the script object model: `Document.variables` and `dataSets`, the
//! variables page items are bound to, Lock Variables, and `importVariables` and `exportVariables`.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::tests_links::{Folder, RED, place, png, write};
use crate::*;

fn run(s: &mut Session, source: &str) -> Result<Value> {
    s.execute("script.run", &json!({"source": source, "name": "t.js"}))
}

/// What the run logged.
fn log(s: &mut Session, source: &str) -> Vec<Value> {
    let r = run(s, source).unwrap_or_else(|e| panic!("{source}: {e}"));
    r["log"].as_array().cloned().unwrap_or_default()
}

/// The message of the run's failure.
fn failure(s: &mut Session, source: &str) -> String {
    run(s, source).map(|r| panic!("{source} ran: {r}")).unwrap_err().to_string()
}

fn undo_steps(s: &Session) -> usize {
    s.active().unwrap().history.undo.len()
}

/// A document with a rectangle and, above it, a type object, so `pageItems[0]` is the type and `pageItems[1]` the rectangle.
fn doc() -> (Session, NodeId, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let r = NodeId(s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap());
    let t = NodeId(s.execute("text.create", &json!({"x": 10, "y": 20, "text": "Hello"})).unwrap()["id"].as_u64().unwrap());
    (s, t, r)
}

fn variables(s: &Session) -> Vec<(String, &'static str)> {
    s.active().unwrap().doc.variables.variables.iter().map(|v| (v.name.clone(), v.kind.label())).collect()
}

#[test]
fn variables_collection_adds_finds_renames_and_removes() {
    let (mut s, _, _) = doc();
    let before = undo_steps(&s);
    let out = log(
        &mut s,
        r#"var v = app.activeDocument.variables;
        vc.log(v.length); vc.log(v.typename); vc.log(v.parent.typename);
        var a = v.add(), b = v.add();
        vc.log([a.name, b.name, a.kind, v.length].join());
        b.name = 'title'; b.kind = VariableKind.VISIBILITY;
        vc.log([b.name, b.kind, v.getByName('title').name, v.index(0).name, v[1].name].join());
        vc.log(v[2] === undefined && v[-1] === undefined && v[0.5] === undefined);
        a.remove(); vc.log(v.length); vc.log(b.parent.typename + ',' + b.typename);"#,
    );
    assert_eq!(
        out,
        vec![
            json!(0),
            json!("Variables"),
            json!("Document"),
            json!("Variable1,Variable2,VariableKind.TEXTUAL,2"),
            json!("title,VariableKind.VISIBILITY,title,Variable1,title"),
            json!(true),
            json!(1),
            json!("Document,Variable")
        ]
    );
    assert_eq!(variables(&s), vec![("title".to_string(), "visibility")]);
    assert_eq!(undo_steps(&s), before + 1, "one undo step for the run");
    assert!(failure(&mut s, "app.activeDocument.variables.getByName('none');").contains("Not found"));
    assert!(failure(&mut s, "app.activeDocument.variables.index(4);").contains("Not found"));
    assert!(failure(&mut s, "app.activeDocument.variables.index(true);").contains("name or an index"));
}

#[test]
fn variable_names_and_kinds_are_checked_before_anything_changes() {
    let (mut s, _, _) = doc();
    s.execute("variable.define", &json!({"name": "a", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "b", "kind": "text"})).unwrap();
    let before = undo_steps(&s);
    assert!(failure(&mut s, "app.activeDocument.variables[1].name = 'a';").contains("already exists"));
    assert!(failure(&mut s, "app.activeDocument.variables[1].name = '  ';").contains("newName"));
    assert!(failure(&mut s, "app.activeDocument.variables[1].name = 5;").contains("must be a string"));
    assert!(failure(&mut s, "app.activeDocument.variables[0].kind = 'text';").contains("VariableKind"));
    assert!(failure(&mut s, "app.activeDocument.variables[0].kind = VariableKind.Unknown;").contains("Unknown"));
    let long = "x".repeat(5000);
    assert!(failure(&mut s, &format!("app.activeDocument.variables[0].name = '{long}';")).contains("too long"));
    // Setting what the variable already has runs no command.
    log(&mut s, "var v = app.activeDocument.variables[0]; v.name = 'a'; v.name = ' a '; v.kind = VariableKind.TEXTUAL;");
    assert_eq!(undo_steps(&s), before);
    assert_eq!(variables(&s), vec![("a".to_string(), "text"), ("b".to_string(), "text")]);
}

#[test]
fn a_variable_object_goes_stale_and_stays_stale_when_the_name_comes_back() {
    let (mut s, _, _) = doc();
    let e = failure(
        &mut s,
        "var v = app.activeDocument.variables.add();\
         vc.run('variable.delete', {names: ['Variable1']}); vc.run('variable.define', {name: 'Variable1', kind: 'text'});\
         v.name;",
    );
    assert!(e.contains("Stale variable"), "{e}");
    // A failed run keeps nothing, so the variable is defined outside the script.
    s.execute("variable.define", &json!({"name": "Variable1", "kind": "text"})).unwrap();
    let e = failure(&mut s, "var v = app.activeDocument.variables[0]; vc.run('variable.rename', {name: 'Variable1', kind: 'image'}); v.kind;");
    assert!(e.contains("Stale variable"), "{e}");
    let e = failure(&mut s, "var v = app.activeDocument.variables[0]; v.remove(); vc.run('edit.undo'); v.name;");
    assert!(e.contains("Stale variable"), "{e}");
    // A rename through the object keeps it.
    assert_eq!(log(&mut s, "var v = app.activeDocument.variables[0]; v.name = 'renamed'; vc.log(v.name);"), vec![json!("renamed")]);
    // A named engine keeps its globals, but an object from an earlier run is stale.
    run(&mut s, "#targetengine keepvars\nvar held = app.activeDocument.variables[0];").unwrap();
    let e = failure(&mut s, "#targetengine keepvars\nheld.name;");
    assert!(e.contains("Stale variable"), "{e}");
}

#[test]
fn collections_from_an_earlier_run_throw_on_every_member() {
    let (mut s, _, _) = doc();
    run(&mut s, "#targetengine keepcols\nvar v = app.activeDocument.variables, d = app.activeDocument.dataSets;").unwrap();
    for member in ["v.typename", "v.parent", "v.length", "d.typename", "d.parent", "d.length"] {
        let e = failure(&mut s, &format!("#targetengine keepcols\n{member};"));
        assert!(e.contains("Stale document"), "{member}: {e}");
    }
}

#[test]
fn content_and_visibility_variables_bind_one_variable_per_slot() {
    let (mut s, t, r) = doc();
    for (name, kind) in [("title", "text"), ("other", "text"), ("show", "visibility"), ("hide", "visibility")] {
        s.execute("variable.define", &json!({"name": name, "kind": kind})).unwrap();
    }
    let before = undo_steps(&s);
    let out = log(
        &mut s,
        r#"var d = app.activeDocument, v = d.variables, t = d.pageItems[0], r = d.pageItems[1];
        vc.log(t.contentVariable === null && t.visibilityVariable === null);
        t.contentVariable = v.getByName('title'); t.contentVariable = v.getByName('other');
        r.visibilityVariable = v.getByName('show'); r.visibilityVariable = v.getByName('hide');
        vc.log(t.contentVariable.name + ',' + r.visibilityVariable.name);
        vc.log(v.getByName('hide').pageItems.length + ',' + v.getByName('hide').pageItems[0].typename);
        vc.log(r.contentVariable === undefined);"#,
    );
    assert_eq!(out, vec![json!(true), json!("other,hide"), json!("1,PathItem"), json!(true)]);
    let vars = &s.active().unwrap().doc.variables;
    assert_eq!(vars.bindings.get(&t).map(|b| b.iter().cloned().collect::<Vec<_>>()), Some(vec!["other".to_string()]));
    assert_eq!(vars.bindings.get(&r).map(|b| b.iter().cloned().collect::<Vec<_>>()), Some(vec!["hide".to_string()]));
    assert_eq!(undo_steps(&s), before + 1);
    // A variable of another kind is refused, and nothing changes; null unbinds that slot only.
    let e = failure(&mut s, "var d = app.activeDocument; d.pageItems[0].contentVariable = d.variables.getByName('show');");
    assert!(e.contains("visibility variable"), "{e}");
    let e = failure(&mut s, "var d = app.activeDocument; d.pageItems[1].visibilityVariable = d.variables.getByName('title');");
    assert!(e.contains("text variable"), "{e}");
    assert!(failure(&mut s, "app.activeDocument.pageItems[0].contentVariable = 'title';").contains("Variable or null"));
    log(
        &mut s,
        "var d = app.activeDocument; d.pageItems[0].visibilityVariable = d.variables.getByName('show'); d.pageItems[0].contentVariable = null;",
    );
    let vars = &s.active().unwrap().doc.variables;
    assert_eq!(vars.bindings.get(&t).map(|b| b.iter().cloned().collect::<Vec<_>>()), Some(vec!["show".to_string()]));
    // Assigning the binding the object already has runs no command.
    let steps = undo_steps(&s);
    log(
        &mut s,
        "var d = app.activeDocument; d.pageItems[0].visibilityVariable = d.variables.getByName('show'); d.pageItems[0].contentVariable = null;",
    );
    assert_eq!(undo_steps(&s), steps);
}

#[test]
fn an_embedded_image_cannot_take_an_image_variable() {
    let dir = Folder::new("script-vars-embedded");
    let path = dir.file("red.png");
    write(&path, &png(10, 10, RED));
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let id = place(&mut s, &path);
    s.execute("variable.define", &json!({"name": "photo", "kind": "image"})).unwrap();
    let out =
        log(&mut s, "var d = app.activeDocument; d.pageItems[0].contentVariable = d.variables[0]; vc.log(d.pageItems[0].contentVariable.name);");
    assert_eq!(out, vec![json!("photo")]);
    s.execute("links.embed", &json!({"ids": [id.0]})).unwrap();
    s.execute("variable.unbind", &json!({"ids": [id.0]})).unwrap();
    let e = failure(&mut s, "var d = app.activeDocument; vc.log(d.pageItems[0].typename); d.pageItems[0].contentVariable = d.variables[0];");
    assert!(e.contains("embedded image"), "{e}");
    assert!(s.active().unwrap().doc.variables.bindings.is_empty());
    // Its visibility still takes a visibility variable.
    s.execute("variable.define", &json!({"name": "shown", "kind": "visibility"})).unwrap();
    let out = log(
        &mut s,
        "var d = app.activeDocument; d.pageItems[0].visibilityVariable = d.variables.getByName('shown'); vc.log(d.pageItems[0].visibilityVariable.name);",
    );
    assert_eq!(out, vec![json!("shown")]);
}

#[test]
fn data_sets_capture_display_update_and_remove() {
    let (mut s, t, _) = doc();
    s.execute("variable.define", &json!({"name": "title", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "title", "ids": [t.0]})).unwrap();
    let before = undo_steps(&s);
    let out = log(
        &mut s,
        r#"var d = app.activeDocument, sets = d.dataSets;
        vc.log(sets.length + ',' + (d.activeDataset === null));
        var one = sets.add();
        vc.run('text.setText', {id: TEXT_ID, text: 'World'});
        var two = sets.add();
        vc.log([one.name, two.name, sets.length, d.activeDataset.name].join());
        one.display(); vc.log(d.activeDataset.name);
        vc.run('text.setText', {id: TEXT_ID, text: 'Again'});
        two.update(); vc.log(d.activeDataset.name);
        two.name = 'Second'; vc.log(sets.getByName('Second').name + ',' + sets.index(1).name);
        d.activeDataset = two; vc.log(d.activeDataset.name);
        one.remove(); vc.log(sets.length);"#
            .replace("TEXT_ID", &t.0.to_string())
            .as_str(),
    );
    assert_eq!(
        out,
        vec![
            json!("0,true"),
            json!("Data Set 1,Data Set 2,2,Data Set 2"),
            json!("Data Set 1"),
            json!("Data Set 1"),
            json!("Second,Second"),
            json!("Second"),
            json!(1)
        ]
    );
    let vars = &s.active().unwrap().doc.variables;
    assert_eq!(vars.datasets.len(), 1);
    assert_eq!(vars.datasets[0].values.get("title"), Some(&vectorcraft_doc::DataValue::Text("Again".into())));
    assert_eq!(vars.active_dataset.as_deref(), Some("Second"));
    assert_eq!(undo_steps(&s), before + 1, "one undo step for the run");
    let e = failure(&mut s, "var x = app.activeDocument.dataSets[0]; x.remove(); x.name;");
    assert!(e.contains("Stale data set"), "{e}");
    log(&mut s, "app.activeDocument.dataSets.removeAll(); app.activeDocument.dataSets.removeAll();");
    assert!(s.active().unwrap().doc.variables.datasets.is_empty());
    assert!(failure(&mut s, "app.activeDocument.activeDataset = 'Data Set 1';").contains("must be a Dataset"));
}

#[test]
fn update_of_a_data_set_that_holds_the_art_runs_no_command() {
    let (mut s, t, _) = doc();
    s.execute("variable.define", &json!({"name": "title", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "title", "ids": [t.0]})).unwrap();
    s.execute("dataset.capture", &json!({})).unwrap();
    let before = undo_steps(&s);
    let r = run(&mut s, "var x = app.activeDocument.dataSets[0]; x.update(); x.display(); app.activeDocument.activeDataset = x;").unwrap();
    assert_eq!(undo_steps(&s), before);
    assert_eq!(r["slowestCommands"], json!([]), "{r}");
}

#[test]
fn a_data_set_object_goes_stale_when_an_import_replaces_the_rows() {
    let (mut s, t, _) = doc();
    s.execute("variable.define", &json!({"name": "title", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "title", "ids": [t.0]})).unwrap();
    s.execute("dataset.capture", &json!({})).unwrap();
    let e = failure(&mut s, "var x = app.activeDocument.dataSets[0]; vc.run('dataset.import', {csv: 'title\\nNew'}); x.name;");
    assert!(e.contains("Stale data set"), "{e}");
}

#[test]
fn an_import_that_leaves_the_data_sets_as_they_were_keeps_the_data_set_objects() {
    let (mut s, t, _) = doc();
    s.execute("variable.define", &json!({"name": "title", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "title", "ids": [t.0]})).unwrap();
    s.execute("dataset.import", &json!({"csv": "title\nNew"})).unwrap();
    let out = log(&mut s, "var x = app.activeDocument.dataSets[0]; vc.run('dataset.import', {csv: 'title\\nNew'}); vc.log(x.name);");
    assert_eq!(out.len(), 1, "{out:?}");
    let e = failure(&mut s, "var x = app.activeDocument.dataSets[0]; vc.run('dataset.import', {csv: 'title\\nOther'}); x.name;");
    assert!(e.contains("Stale data set"), "{e}");
}

#[test]
fn variables_locked_refuses_new_variables_but_not_bindings() {
    let (mut s, _, _) = doc();
    s.execute("variable.define", &json!({"name": "show", "kind": "visibility"})).unwrap();
    let out = log(
        &mut s,
        "var d = app.activeDocument; vc.log(d.variablesLocked); d.variablesLocked = true; d.variablesLocked = true;\
         d.pageItems[1].visibilityVariable = d.variables[0]; vc.log(d.variablesLocked);",
    );
    assert_eq!(out, vec![json!(false), json!(true)]);
    assert!(s.active().unwrap().doc.variables.locked);
    let e = failure(&mut s, "app.activeDocument.variables.add();");
    assert!(e.contains("locked"), "{e}");
    assert!(failure(&mut s, "app.activeDocument.variablesLocked = 1;").contains("true or false"));
    assert_eq!(variables(&s).len(), 1);
}

/// A variable library of two variables and two data sets, written for these tests from the elements
/// and attributes `variable.loadLibrary` reads: `variableSets`, `variableSet`, `variables` of
/// `variable` elements with `varName` and `trait`, and `sampleDataSets` of `sampleDataSet` elements
/// with `dataSetName`.
const LIBRARY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<variableSets>
  <variableSet varSetName="binding1" locked="none">
    <variables>
      <variable varName="title" trait="textcontent"/>
      <variable varName="badge" trait="visibility"/>
    </variables>
    <sampleDataSets>
      <sampleDataSet dataSetName="One"><title><p>Hello</p></title><badge>true</badge></sampleDataSet>
      <sampleDataSet dataSetName="Two"><title><p>World</p></title><badge>false</badge></sampleDataSet>
    </sampleDataSets>
  </variableSet>
</variableSets>
"#;

/// A fresh folder with a `main` folder for the script, by its real path.
fn folder(name: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("vc-script-vars-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("main")).unwrap();
    std::fs::canonicalize(&base).unwrap()
}

/// Runs `main/main.jsx` in `base` with `text`, on `s`.
fn run_file(s: &mut Session, base: &Path, text: &str) -> Result<Value> {
    let main = base.join("main/main.jsx");
    std::fs::write(&main, text).unwrap();
    s.execute("script.run", &json!({"path": main.to_string_lossy()}))
}

#[test]
fn import_variables_replaces_the_variables_and_keeps_compatible_bindings() {
    let base = folder("import");
    std::fs::write(base.join("main/lib.xml"), LIBRARY).unwrap();
    let (mut s, t, _) = doc();
    s.execute("variable.define", &json!({"name": "title", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "gone", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "title", "ids": [t.0]})).unwrap();
    let before = undo_steps(&s);
    let r = run_file(
        &mut s,
        &base,
        "var d = app.activeDocument; d.importVariables(new File('lib.xml'));\
         vc.log(d.variables.length + ',' + d.dataSets.length + ',' + d.dataSets[1].name);\
         d.dataSets.getByName('Two').display(); vc.log(d.pageItems[0].contentVariable.name);",
    )
    .unwrap();
    assert_eq!(r["log"], json!(["2,2,Two", "title"]));
    assert_eq!(variables(&s), vec![("title".to_string(), "text"), ("badge".to_string(), "visibility")]);
    assert_eq!(s.active().unwrap().doc.story_text(t), "World");
    assert_eq!(undo_steps(&s), before + 1);
}

#[test]
fn import_variables_takes_only_a_file_the_script_may_read() {
    let base = folder("import-refused");
    std::fs::write(base.join("main/lib.xml"), LIBRARY).unwrap();
    std::fs::write(base.join("outside.xml"), LIBRARY).unwrap();
    let (mut s, _, _) = doc();
    s.execute("variable.define", &json!({"name": "keep", "kind": "text"})).unwrap();
    let fail = |s: &mut Session, text: &str| run_file(s, &base, text).map(|r| panic!("{text} ran: {r}")).unwrap_err().to_string();
    assert!(fail(&mut s, "app.activeDocument.importVariables('lib.xml');").contains("needs a File"));
    assert!(fail(&mut s, "app.activeDocument.importVariables(new Folder('.'));").contains("needs a File"));
    let outside = base.join("outside.xml").to_string_lossy().replace('\\', "/");
    let e = fail(&mut s, &format!("app.activeDocument.importVariables(new File('{outside}'));"));
    assert!(e.contains("access denied"), "{e}");
    // The open document's own file is refused, though the script's commands may open it.
    let saved = base.join("doc.vectorcraft").to_string_lossy().into_owned();
    s.execute("file.saveAs", &json!({"path": saved})).unwrap();
    let e = fail(&mut s, &format!("app.activeDocument.importVariables(new File('{}'));", saved.replace('\\', "/")));
    assert!(e.contains("access denied"), "{e}");
    // A plain object cannot pass for a File.
    let e = fail(&mut s, "var o = {}; o['\\u0000fileRef'] = new File('lib.xml').fsName; app.activeDocument.importVariables(o);");
    assert!(e.contains("not plain data"), "{e}");
    assert_eq!(variables(&s), vec![("keep".to_string(), "text")]);
}

#[test]
fn import_variables_refuses_hostile_libraries_before_any_edit() {
    let base = folder("import-hostile");
    let external = r#"<?xml version="1.0"?><!DOCTYPE v [<!ENTITY x SYSTEM "file:///etc/hosts">]><variableSets><variableSet><variables><variable varName="a" trait="textcontent"/></variables></variableSet></variableSets>"#;
    std::fs::write(base.join("main/external.xml"), external).unwrap();
    let deep = format!("<variableSets>{}{}</variableSets>", "<a>".repeat(300), "</a>".repeat(300));
    std::fs::write(base.join("main/deep.xml"), deep).unwrap();
    let mut big = LIBRARY.as_bytes().to_vec();
    big.resize((8 << 20) + 1, b' ');
    std::fs::write(base.join("main/big.xml"), big).unwrap();
    let twice = LIBRARY.replace("varName=\"badge\"", "varName=\"title\"");
    std::fs::write(base.join("main/twice.xml"), twice).unwrap();
    let (mut s, _, _) = doc();
    s.execute("variable.define", &json!({"name": "keep", "kind": "text"})).unwrap();
    let before = undo_steps(&s);
    for (file, says) in [("external.xml", "document type declaration"), ("deep.xml", "deep"), ("big.xml", "larger than"), ("twice.xml", "twice")] {
        let e = run_file(&mut s, &base, &format!("app.activeDocument.importVariables(new File('{file}'));")).unwrap_err().to_string();
        assert!(e.contains(says), "{file}: {e}");
    }
    assert_eq!(variables(&s), vec![("keep".to_string(), "text")]);
    assert_eq!(undo_steps(&s), before);
    // Locked variables refuse a library that would delete or define one.
    s.execute("variable.lock", &json!({"locked": true})).unwrap();
    std::fs::write(base.join("main/lib.xml"), LIBRARY).unwrap();
    let e = run_file(&mut s, &base, "app.activeDocument.importVariables(new File('lib.xml'));").unwrap_err().to_string();
    assert!(e.contains("locked"), "{e}");
    assert_eq!(variables(&s), vec![("keep".to_string(), "text")]);
}

#[test]
fn export_variables_writes_a_library_that_import_reads_back() {
    let (mut s, t, _) = doc();
    s.execute("variable.define", &json!({"name": "title", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "title", "ids": [t.0]})).unwrap();
    s.execute("dataset.capture", &json!({"name": "Row"})).unwrap();
    let out = log(
        &mut s,
        "var d = app.activeDocument, f = new File(Folder.temp + '/out.xml');\
         d.exportVariables(f); f.open('r'); var x = f.read(); f.close(); vc.log(x);\
         d.variables.removeAll(); d.importVariables(f); vc.log(d.variables[0].name + ',' + d.dataSets[0].name);",
    );
    let xml = out[0].as_str().unwrap();
    for part in ["varName=\"title\"", "trait=\"textcontent\"", "dataSetName=\"Row\"", "<p>Hello</p>"] {
        assert!(xml.contains(part), "{part} in {xml}");
    }
    assert_eq!(out[1], json!("title,Row"));
}

#[test]
fn export_variables_writes_only_where_the_script_may_write() {
    let base = folder("export-refused");
    let (mut s, _, _) = doc();
    s.execute("variable.define", &json!({"name": "title", "kind": "text"})).unwrap();
    // The open document's own file is refused, though the script's commands may save it.
    let saved = base.join("main/doc.vectorcraft").to_string_lossy().into_owned();
    s.execute("file.saveAs", &json!({"path": saved})).unwrap();
    let before = std::fs::read(&saved).unwrap();
    let path = saved.replace('\\', "/");
    let e = run_file(&mut s, &base, &format!("app.activeDocument.exportVariables(new File('{path}'));")).unwrap_err().to_string();
    assert!(e.contains("access denied"), "{e}");
    assert_eq!(std::fs::read(&saved).unwrap(), before);
    // The script's own folder is only read.
    let e = run_file(&mut s, &base, "app.activeDocument.exportVariables(new File('out.xml'));").unwrap_err().to_string();
    assert!(e.contains("access denied"), "{e}");
    assert!(!base.join("main/out.xml").exists());
    assert!(run_file(&mut s, &base, "app.activeDocument.exportVariables('out.xml');").unwrap_err().to_string().contains("needs a File"));
    // A library that cannot be written leaves no file.
    s.execute("variable.rename", &json!({"name": "title", "newName": "has space"})).unwrap();
    let e = failure(&mut s, "app.activeDocument.exportVariables(new File(Folder.temp + '/bad.xml'));");
    assert!(e.contains("can't be saved"), "{e}");
    let out =
        log(&mut s, "var f = new File(Folder.temp + '/bad.xml'); try { app.activeDocument.exportVariables(f); } catch (e) {} vc.log(f.exists);");
    assert_eq!(out, vec![json!(false)]);
}
