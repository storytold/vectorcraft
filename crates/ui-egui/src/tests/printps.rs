//! Print to a PostScript file through the app: the job is saved, never sent to the printer.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::json;
use vectorcraft_engine::Session;

use crate::print::{PrintJob, PrintService, Printer};
use crate::{Services, VectorcraftApp};

/// What the app wrote: (path, bytes).
type Written = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

/// A printer that counts the jobs it is sent.
struct Counting(Rc<RefCell<usize>>);

impl PrintService for Counting {
    fn printers(&mut self) -> Vec<Printer> {
        vec![Printer { name: "Office".into(), default: true }]
    }

    fn print(&mut self, _: &PrintJob) -> Result<String, String> {
        *self.0.borrow_mut() += 1;
        Ok("Sent".into())
    }
}

#[test]
fn postscript_is_saved_not_printed() {
    let sent = Rc::new(RefCell::new(0));
    let written: Written = Rc::default();
    let w = written.clone();
    let services = Services {
        print: Some(Box::new(Counting(sent.clone()))),
        pick_save: Some(Box::new(|pick: &crate::FilePick| Some(format!("/tmp/{}", pick.name)))),
        write: Some(Box::new(move |p: &str, b: &[u8]| {
            w.borrow_mut().push((p.to_string(), b.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 100, "height": 80, "artboards": 2})).unwrap();
    // A .ps path: PostScript, at the level asked for.
    let r = app.run("file.print", json!({"path": "/tmp/job.ps", "level": 2})).unwrap();
    assert_eq!((r["format"].clone(), r["printed"].clone(), r["pages"].clone()), (json!("postscript"), json!(false), json!(2)), "{r}");
    // No path: a picked .ps file.
    let r = app.run("file.print", json!({"format": "postscript", "settings": {"copies": 2}})).unwrap();
    assert_eq!(r["pages"], 4);
    let w = written.borrow();
    assert_eq!(w.len(), 2);
    assert_eq!(w[0].0, "/tmp/job.ps");
    assert!(w[1].0.ends_with(".ps"), "{}", w[1].0);
    for (_, bytes) in w.iter() {
        assert!(bytes.starts_with(b"%!PS-Adobe-3.0\n"));
    }
    assert!(String::from_utf8_lossy(&w[0].1).contains("%%LanguageLevel: 2"));
    assert_eq!(*sent.borrow(), 0, "nothing went to the printer");
    drop(w);
    // A PDF still goes to the printer.
    app.run("file.print", json!({"settings": {"copies": 1}})).unwrap();
    assert_eq!(*sent.borrow(), 1);
    assert!(app.run("file.print", json!({"format": "ps2"})).is_err());
}

/// Submission stays off the frame and the eventual queue receipt/refusal reaches the status bar.
#[test]
fn desktop_print_pending_receipt_and_refusal_are_distinct() {
    use std::sync::{Arc, Mutex, mpsc};
    struct AsyncPrinter {
        release: Arc<Mutex<mpsc::Receiver<()>>>,
        refused: bool,
    }
    impl PrintService for AsyncPrinter {
        fn printers(&mut self) -> Vec<Printer> {
            Vec::new()
        }
        fn print(&mut self, _: &PrintJob) -> Result<String, String> {
            Err("UI thread must not submit".into())
        }
        fn shared_print(&self) -> Option<crate::print::SharedPrintFn> {
            let release = self.release.clone();
            let refused = self.refused;
            Some(Arc::new(move |job| {
                release.lock().unwrap().recv_timeout(std::time::Duration::from_secs(10)).unwrap();
                assert_eq!(job.printer, Some("Office Laser"));
                assert!(job.pdf.starts_with(b"%PDF-"));
                if refused { Err("queue refused this job".into()) } else { Ok("Sent to Office Laser".into()) }
            }))
        }
    }
    for refused in [false, true] {
        let (release, wait) = mpsc::channel();
        let services = Services {
            print: Some(Box::new(AsyncPrinter { release: Arc::new(Mutex::new(wait)), refused })),
            write_shared: Some(Arc::new(|_, _| Err("printing must not write a document".into()))),
            ..Default::default()
        };
        let mut app = VectorcraftApp::new(Session::new(), services);
        app.run("file.new", json!({ "width": 100, "height": 80 })).unwrap();
        let pending = app.run("file.print", json!({ "printer": "Office Laser" })).unwrap();
        assert_eq!(pending["background"], true);
        assert_eq!(pending["printed"], false);
        assert_eq!(pending["pages"], 1);
        assert!(pending["bytes"].as_u64().is_some_and(|bytes| bytes > 0));
        assert!(!app.ui.status.starts_with("Sent"));
        release.send(()).unwrap();
        crate::background::wait_all(&mut app);
        assert!(app.background.jobs.is_empty());
        if refused {
            assert!(app.ui.status.contains("queue refused this job"));
        } else {
            assert_eq!(app.ui.status, "Sent to Office Laser");
        }
    }
}
