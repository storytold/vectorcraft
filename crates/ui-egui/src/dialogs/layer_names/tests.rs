use std::io::{Read, Write};
use std::sync::atomic::Ordering;
use std::sync::{Arc, mpsc};

use super::super::missing_links::drive;
use super::*;

/// A job whose worker has finished `names` and is otherwise idle (no thread behind it).
fn idle_job(names: Vec<Value>, base: usize) -> (Job, Arc<WorkerProgress>, Arc<AtomicBool>) {
    let progress = Arc::new(WorkerProgress::default());
    progress.names.lock().unwrap().extend(names);
    let (_tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    (Job { base, progress: progress.clone(), rx, cancel: cancel.clone() }, progress, cancel)
}

/// The open dialog's fields.
fn fields(app: &VectorcraftApp) -> Fields {
    match drive::fields::<LayerNamesDialog>(app) {
        Value::Object(fields) => fields,
        other => panic!("the naming dialog is not open: {other}"),
    }
}

/// Replaces the open dialog's fields.
fn put(app: &VectorcraftApp, d: Fields) {
    app.dialogs().unwrap().with(|h| h.get_mut::<LayerNamesDialog>().unwrap().fields = d);
}

/// Groups two rectangles one by one → `layer.namingCandidates` at depth 16 (the layer and the two groups).
fn two_groups(app: &mut VectorcraftApp) -> Value {
    for x in [10, 100] {
        app.run("shape.rectangle", json!({"x":x,"y":10,"width":40,"height":20})).unwrap();
        app.run("object.group", json!({})).unwrap();
    }
    app.run("layer.namingCandidates", json!({"depth":16})).unwrap()
}

/// Fields of a run on `snapshot` with `names` ready.
fn run_fields(app: &VectorcraftApp, snapshot: &Value, names: Value) -> Fields {
    let mut d = settings(app);
    for key in ["uid", "revision"] {
        d.insert(key.into(), snapshot[key].clone());
    }
    d.insert("candidates".into(), snapshot["rows"].clone());
    d.insert("names".into(), names);
    d
}

/// Reads one HTTP request from `stream` → its JSON body.
fn read_request(stream: &mut std::net::TcpStream) -> Value {
    stream.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        header.extend(byte);
    }
    let header = String::from_utf8(header).unwrap();
    assert!(header.starts_with("POST /api/chat "));
    let len: usize = header.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|n| n.trim().parse().unwrap())).unwrap();
    let mut body = vec![0; len];
    stream.read_exact(&mut body).unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn reply(stream: &mut std::net::TcpStream, names: Value) {
    let reply = json!({"message":{"content":json!({"names":names}).to_string()}}).to_string();
    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", reply.len(), reply)
        .unwrap();
}

#[test]
/// A failed run keeps the name completed before the error.
fn a_failed_run_keeps_the_name_completed_before_the_error() {
    let (job, _, _) = idle_job(vec![json!({"id": 7, "name": "Coin Counter"})], 0);
    let mut d = Fields::new();
    fail_job(&mut d, &job, "curl failed".into());
    assert_eq!(d["names"][0]["name"], "Coin Counter");
    assert_eq!(d["error"], "curl failed");
    assert_eq!(d["busy"], false);
}

#[test]
/// The dialog draws without changing the document, and Cancel leaves it as it was.
fn the_dialog_draws_and_cancels_without_changing_the_document() {
    let mut app = drive::app();
    app.run("file.new", json!({})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    app.run("ui.nameLayers", json!({})).unwrap();
    assert!(drive::is_open::<LayerNamesDialog>(&app));
    drive::frame(&mut app);
    drive::pump(&mut app);
    drive::answer::<LayerNamesDialog>(&mut app, &[], false);
    assert!(!drive::is_open::<LayerNamesDialog>(&app));
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
}

#[test]
/// Apply Ready Names renames in one undo step; a rejected apply keeps a name finished at apply time.
fn apply_is_one_undo_step_and_a_rejected_apply_keeps_late_names() {
    let mut app = drive::app();
    app.run("file.new", json!({})).unwrap();
    let before = app.run("layer.namingCandidates", json!({})).unwrap();
    open(&mut app).unwrap();
    let d = run_fields(&app, &before, json!([{"id": before["rows"][0]["id"], "name": "Game UI Kit"}]));
    apply(&mut app, d.clone()).unwrap();
    assert_eq!(app.run("layer.namingCandidates", json!({})).unwrap()["rows"][0]["name"], "Game UI Kit");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.run("layer.namingCandidates", json!({})).unwrap()["rows"], before["rows"]);

    // The revision moved on: the apply is refused, and the name the worker finished meanwhile is kept.
    app.run("shape.rectangle", json!({"x":0,"y":0,"width":5,"height":5})).unwrap();
    let (job, _, _) = idle_job(vec![json!({"id": 999, "name": "Late Name"})], 0);
    app.naming_job = Some(job);
    let (kept, _) = apply(&mut app, d).unwrap_err();
    assert!(kept["names"].as_array().unwrap().iter().any(|n| n["name"] == "Late Name"));
    assert_eq!(kept["busy"], false, "a refused apply leaves Start Over available");
}

#[test]
/// Pausing, applying the ready names and reopening resumes with the edits; changed artwork drops the checkpoint.
fn pause_apply_partial_and_reopen_keeps_checkpoint_and_edits() {
    let mut app = drive::app();
    app.run("file.new", json!({})).unwrap();
    let snapshot = two_groups(&mut app);
    let rows = snapshot["rows"].as_array().unwrap().clone();
    assert!(rows.len() >= 3);
    open(&mut app).unwrap();
    let mut d = run_fields(&app, &snapshot, json!([{"id": rows[0]["id"], "name": "Edited collection name"}]));
    d.insert("busy".into(), json!(true));
    put(&app, d);
    let (job, progress, cancel) = idle_job(vec![json!({"id": rows[1]["id"], "name": "Play Button"})], 1);
    progress.total.store(rows.len() - 1, Ordering::Relaxed);
    progress.done.store(1, Ordering::Relaxed);
    progress.paused.store(true, Ordering::Relaxed);
    app.naming_job = Some(job);
    drive::pump(&mut app);
    let d = fields(&app);
    assert_eq!(d["names"].as_array().unwrap().len(), 2);
    assert_eq!(d["total"], rows.len());
    assert_eq!(d["names"][0]["name"], "Edited collection name");
    assert!(can_apply(&d));

    drive::answer::<LayerNamesDialog>(&mut app, &[], true);
    assert!(cancel.load(Ordering::Relaxed), "applying stops the worker");
    assert!(!drive::is_open::<LayerNamesDialog>(&app));
    let fresh = app.run("layer.namingCandidates", json!({"depth":16})).unwrap();
    assert_eq!(fresh["rows"][0]["name"], "Edited collection name");

    open(&mut app).unwrap();
    let saved = fields(&app);
    assert_eq!(saved["names"].as_array().unwrap().len(), 2);
    assert_eq!(saved["busy"], false);
    assert_eq!(saved["revision"], app.session.active().unwrap().revision);

    drive::answer::<LayerNamesDialog>(&mut app, &[], false);
    app.run("shape.rectangle", json!({"x":200,"y":10,"width":10,"height":10})).unwrap();
    open(&mut app).unwrap();
    assert!(fields(&app).get("names").is_none(), "a stale checkpoint must not target changed artwork");
}

#[test]
/// Closing the dialog while a request runs stops the worker and keeps the latest finished name.
fn closing_in_flight_job_retains_latest_completed_batch() {
    let mut app = drive::app();
    app.run("file.new", json!({})).unwrap();
    open(&mut app).unwrap();
    let snapshot = app.run("layer.namingCandidates", json!({})).unwrap();
    let mut d = run_fields(&app, &snapshot, json!([]));
    d.remove("names");
    d.insert("busy".into(), json!(true));
    put(&app, d);
    drive::pump(&mut app);
    let (job, progress, cancel) = idle_job(vec![json!({"id": snapshot["rows"][0]["id"], "name": "Ready collection"})], 0);
    progress.total.store(1, Ordering::Relaxed);
    app.naming_job = Some(job);
    drive::answer::<LayerNamesDialog>(&mut app, &[], false);
    assert!(cancel.load(Ordering::Relaxed));
    assert!(app.naming_job.is_none());
    open(&mut app).unwrap();
    assert_eq!(fields(&app)["names"][0]["name"], "Ready collection");
}

#[test]
/// Automation edits only names that are ready, and refuses settings while naming runs.
fn automation_edits_ready_names_and_refuses_settings_while_naming() {
    let ctx = egui::Context::default();
    let mut dialog = LayerNamesDialog { fields: Fields::new(), request: None };
    assert!(dialog.set_json_field(&ctx, "names", json!([{"id": 1, "name": "X"}])).is_err());
    dialog.fields.insert("names".into(), json!([{"id": 1, "name": "Coin"}]));
    dialog.set_json_field(&ctx, "names", json!([{"id": 1, "name": "Coin Counter"}])).unwrap();
    assert_eq!(dialog.fields["names"][0]["name"], "Coin Counter");
    assert!(dialog.set_json_field(&ctx, "names", json!([{"id": 2, "name": "Other"}])).is_err());
    dialog.fields.insert("busy".into(), json!(true));
    assert!(dialog.set_json_field(&ctx, "model", json!("other:7b")).is_err());
    dialog.set_json_field(&ctx, "action", json!("pause")).unwrap();
    assert_eq!(dialog.request, Some(Request::Pause));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
/// Renders a preview per group with artwork and speaks Ollama's chat protocol with a strict schema.
fn renders_previews_and_uses_ollama_protocol() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/api/chat", listener.local_addr().unwrap());
    let mut app = drive::app();
    app.run("file.new", json!({})).unwrap();
    app.run("shape.rectangle", json!({"x":0,"y":0,"width":20,"height":20})).unwrap();
    app.run("layer.new", json!({"name":"Empty"})).unwrap();
    let snapshot = app.run("layer.namingCandidates", json!({})).unwrap();
    let rows = snapshot["rows"].as_array().unwrap().clone();
    assert_eq!(rows.len(), 2);
    let id = rows.iter().find(|r| r["name"] != "Empty").unwrap()["id"].clone();
    let expected = json!([{"id": id, "name":"Play Button"}]);
    let answer = expected.clone();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_request(&mut stream);
        assert_eq!(request["model"], "test-vision");
        assert_eq!(request["stream"], false);
        assert_eq!(request["format"]["required"], json!(["names"]));
        assert_eq!(request["format"]["properties"]["names"]["maxItems"], 1);
        assert_eq!(request["format"]["properties"]["names"]["items"]["properties"]["id"]["enum"], json!([id]));
        assert_eq!(request["options"]["temperature"], 0);
        assert!(!request["messages"][1]["content"].as_str().unwrap().contains("Layer 1"), "the prompt carries no layer names");
        assert_eq!(request["messages"][1]["images"].as_array().unwrap().len(), 1);
        reply(&mut stream, answer);
    });
    let flag = AtomicBool::new(false);
    let progress = WorkerProgress::default();
    let result = generate(&app.session.active().unwrap().doc, &rows, &endpoint, "test-vision", "English", &flag, &progress).unwrap();
    assert_eq!(progress.total.load(Ordering::Relaxed), 1, "the empty layer is not sent");
    assert_eq!(progress.done.load(Ordering::Relaxed), 1);
    server.join().unwrap();
    assert_eq!(result, expected);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
/// Pause lets the current request finish and asks for nothing more; Continue skips the groups already named.
fn pause_finishes_current_request_and_resume_skips_ready_groups() {
    let mut app = drive::app();
    app.run("file.new", json!({})).unwrap();
    let snapshot = two_groups(&mut app);
    let rows = snapshot["rows"].as_array().unwrap().clone();
    assert_eq!(rows.len(), 3);
    let expected: Vec<_> = rows.iter().skip(1).map(|r| r["id"].clone()).collect();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/api/chat", listener.local_addr().unwrap());
    let (requests, seen) = mpsc::channel();
    let (release, gate) = mpsc::channel::<()>();
    let server = std::thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            let id = request["format"]["properties"]["names"]["items"]["properties"]["id"]["enum"][0].clone();
            requests.send(id.clone()).unwrap();
            gate.recv_timeout(Duration::from_secs(30)).unwrap();
            reply(&mut stream, json!([{"id": id, "name": "Ready Button"}]));
        }
    });
    let mut d = run_fields(&app, &snapshot, json!([{"id": rows[0]["id"], "name": "Already completed"}]));
    d.insert("endpoint".into(), json!(endpoint));
    d.insert("model".into(), json!("test-vision"));
    d.insert("depth".into(), json!(16));
    start(&mut app, &mut d).unwrap();
    let job = app.naming_job.as_ref().unwrap();
    assert_eq!(seen.recv_timeout(Duration::from_secs(30)).unwrap(), expected[0]);
    job.progress.paused.store(true, Ordering::Relaxed);
    release.send(()).unwrap();
    let until = std::time::Instant::now() + Duration::from_secs(30);
    while job.progress.done.load(Ordering::Relaxed) < 1 {
        assert!(std::time::Instant::now() < until);
        std::thread::sleep(Duration::from_millis(5));
    }
    sync(&mut d, job).unwrap();
    assert_eq!(d["names"].as_array().unwrap().len(), 2);
    assert!(seen.recv_timeout(Duration::from_millis(100)).is_err(), "pause must not request the next group");
    job.progress.paused.store(false, Ordering::Relaxed);
    assert_eq!(seen.recv_timeout(Duration::from_secs(30)).unwrap(), expected[1]);
    release.send(()).unwrap();
    let result = job.rx.recv_timeout(Duration::from_secs(30)).unwrap().unwrap();
    assert_eq!(result.as_array().unwrap().len(), 2);
    sync(&mut d, job).unwrap();
    assert_eq!(d["names"].as_array().unwrap().len(), 3);
    assert_eq!(d["names"][0]["name"], "Already completed");
    server.join().unwrap();
}

/// Opt-in real-model check; normal CI uses the deterministic local HTTP fixture above.
#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "requires a running local Ollama vision model"]
fn real_ollama_returns_the_required_names_array() {
    let model = std::env::var("VECTORCRAFT_NAMING_OLLAMA_MODEL").unwrap_or_else(|_| "qwen2.5vl:3b".into());
    let mut app = drive::app();
    app.run("file.new", json!({})).unwrap();
    for i in 0..4 {
        if i != 0 {
            app.run("layer.new", json!({})).unwrap();
        }
        app.run("shape.rectangle", json!({"x":i*30,"y":0,"width":20,"height":20})).unwrap();
    }
    let snapshot = app.run("layer.namingCandidates", json!({"depth":16})).unwrap();
    let rows = snapshot["rows"].as_array().unwrap().clone();
    let flag = AtomicBool::new(false);
    let result =
        generate(&app.session.active().unwrap().doc, &rows, "http://localhost:11434/api/chat", &model, "English", &flag, &WorkerProgress::default())
            .unwrap();
    assert_eq!(result.as_array().unwrap().len(), rows.len());
    for name in result.as_array().unwrap() {
        assert!(!matches!(name["name"].as_str().unwrap(), "<Group>" | "<Clip Group>" | "objects"));
    }
}

#[test]
/// Model output cannot add targets or invalid names.
fn model_cannot_inject_targets_or_invalid_names() {
    let rows = vec![json!({"id": 4}), json!({"id": 7})];
    for content in [
        r#"{"names":[{"id":4,"name":"X"},{"id":4,"name":"Y"}]}"#,
        r#"{"names":[{"id":4,"name":"X"},{"id":99,"name":"Y"}]}"#,
        r#"{"names":[{"id":4,"name":"X"},{"id":7,"name":"  "}]}"#,
        "not json",
    ] {
        assert!(validate(content, &rows).is_err());
    }
    let names = validate(r#"{"names":[{"id":7,"name":" Settings "},{"id":4,"name":"Play"}]}"#, &rows).unwrap();
    assert_eq!(names, vec![json!({"id":4,"name":"Play"}), json!({"id":7,"name":"Settings"})]);
}
