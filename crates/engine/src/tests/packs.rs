//! Language packs (`proof.packs`, `proof.installPack`, `proof.removePack`, #4936): an installed pack
//! spell-checks its language in the documents, an install needs downloads on, and a pack installed
//! in the background reaches the documents once it ends.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde_json::{Value, json};

use crate::packs::{self, Backend, Cancel, Pack};
use crate::*;

/// Tests that set the process-wide packs backend take turns.
fn packs_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Puts the process back to no packs and no background installs when dropped, also when the test
/// panicked: the jobs still running are cancelled and waited for, so none outlives its test. Then
/// it deletes the test's folder.
struct Reset(PathBuf);

impl Drop for Reset {
    fn drop(&mut self) {
        while packs::busy() {
            for (locale, _, _) in packs::status().installing {
                packs::cancel(&locale);
            }
            packs::cancel("");
            std::thread::yield_now();
        }
        packs::run_in_background(false);
        packs::set_backend(None);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A new folder for one test.
fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vc-packs-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Writes a small Dutch pack into `packs/nl`, laid out as the font store unpacks one.
fn write_dutch_pack(packs: &Path) {
    let dir = packs.join("nl");
    std::fs::create_dir_all(dir.join("LICENCES")).unwrap();
    std::fs::write(dir.join("pack.toml"), "locale = \"nl\"\nname = \"Nederlands (test)\"\nlicence = \"MIT\"\n").unwrap();
    std::fs::write(dir.join("LICENCES").join("COPYING"), "Test licence text.").unwrap();
    std::fs::write(dir.join("spelling.aff"), "SET UTF-8\nTRY esianrtolducgmphbykfvwjzxq\n").unwrap();
    std::fs::write(dir.join("spelling.dic"), "5\nhet\nhuis\nen\nde\nfiets\n").unwrap();
}

/// A session with a new document holding Dutch point type with one misspelt word, `fietz`.
fn dutch_session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let id = s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Het huis en de fietz"})).unwrap()["id"].clone();
    s.execute("text.setFormat", &json!({"ids": [id], "language": "Dutch"})).unwrap();
    s
}

/// The unknown words a check lists, as (word, lang).
fn unknown(s: &mut Session) -> Vec<(String, String)> {
    let r = s.execute("text.spelling.check", &json!({})).unwrap();
    r["words"].as_array().unwrap().iter().map(|w| (w["word"].as_str().unwrap().into(), w["lang"].as_str().unwrap().into())).collect()
}

fn installed_locales(s: &mut Session) -> Vec<String> {
    let r = s.execute("proof.packs", &json!({})).unwrap();
    r["installed"].as_array().unwrap().iter().map(|p| p["locale"].as_str().unwrap().to_string()).collect()
}

#[test]
fn a_pack_in_the_font_store_checks_its_language_until_it_is_removed() {
    let _turn = packs_lock();
    let root = folder("store");
    let _reset = Reset(root.clone());
    let mut s = dutch_session();
    let without = unknown(&mut s);
    // A pack another Craft app installed into the shared store.
    let store = packs::font_store_in(root.clone());
    write_dutch_pack(&store.packs_dir());
    packs::set_backend(Some(store));
    assert_eq!(unknown(&mut s), [("fietz".to_string(), "nl".to_string())], "checked in Dutch by the pack");
    assert_eq!(installed_locales(&mut s), ["nl"]);
    let listed = s.execute("proof.packs", &json!({})).unwrap();
    assert_eq!((listed["available"].clone(), listed["downloads"].clone()), (json!(true), json!(false)), "downloads stay off");
    let before = s.doc().unwrap().revision;
    assert_eq!(s.execute("proof.removePack", &json!({"locale": "NL"})).unwrap()["installed"], false);
    assert!(s.doc().unwrap().revision > before, "the document is laid out again");
    assert_eq!(unknown(&mut s), without, "checked as before the pack");
    assert!(installed_locales(&mut s).is_empty());
    assert!(s.execute("proof.removePack", &json!({"locale": "nl"})).is_err(), "no longer installed");
}

/// A stand-in for the font store: it lists a Dutch pack and installs it by writing its files.
struct Fake {
    dir: PathBuf,
    downloads: bool,
    /// While set, an install waits until it is cleared.
    hold: AtomicBool,
}

impl Backend for Fake {
    fn packs_dir(&self) -> PathBuf {
        self.dir.clone()
    }

    fn downloads_on(&self) -> bool {
        self.downloads
    }

    fn cached_index(&self) -> Vec<Pack> {
        let nl = Pack {
            locale: "nl".into(),
            name: "Nederlands".into(),
            size: 2,
            licence: "MIT".into(),
            class: "Embeddable".into(),
            spelling: true,
            hyphenation: false,
        };
        vec![nl]
    }

    fn fetch_index(&self, _: &Cancel) -> std::result::Result<Vec<Pack>, String> {
        Ok(self.cached_index())
    }

    fn install(&self, locale: &str, cancel: &Cancel, progress: &mut dyn FnMut(u64, u64)) -> std::result::Result<(), String> {
        progress(1, 2);
        while self.hold.load(Ordering::Relaxed) {
            if cancel.is_cancelled() {
                return Err("cancelled".into());
            }
            std::thread::yield_now();
        }
        // The index lists only Dutch, which `packs::install` checks first.
        if locale != "nl" {
            return Err(format!("asked to install {locale}"));
        }
        write_dutch_pack(&self.dir);
        Ok(())
    }

    fn remove(&self, locale: &str) -> std::result::Result<(), String> {
        std::fs::remove_dir_all(self.dir.join(locale)).map_err(|e| e.to_string())
    }
}

fn error(s: &mut Session, id: &str, p: Value) -> String {
    s.execute(id, &p).unwrap_err().to_string()
}

#[test]
fn an_install_needs_downloads_on_and_a_pack_the_index_lists() {
    let _turn = packs_lock();
    let root = folder("fake");
    let _reset = Reset(root.clone());
    let mut s = dutch_session();
    assert!(error(&mut s, "proof.installPack", json!({"locale": "nl"})).contains("aren't available"), "no store, no packs");
    packs::set_backend(Some(Arc::new(Fake { dir: root.clone(), downloads: false, hold: AtomicBool::new(false) })));
    assert!(error(&mut s, "proof.installPack", json!({"locale": "nl"})).contains("downloading is turned off"));
    assert!(error(&mut s, "proof.packs", json!({"refresh": true})).contains("downloading is turned off"));
    assert!(installed_locales(&mut s).is_empty(), "nothing installed");
    packs::set_backend(Some(Arc::new(Fake { dir: root.clone(), downloads: true, hold: AtomicBool::new(false) })));
    for bad in [json!({}), json!({"locale": ""}), json!({"locale": "../nl"}), json!({"locale": 5})] {
        assert!(s.execute("proof.installPack", &bad).is_err(), "{bad} is refused");
    }
    assert!(!root.exists(), "a refused locale writes nothing");
    assert!(error(&mut s, "proof.installPack", json!({"locale": "fy"})).contains("no Language pack for fy"));
    assert!(s.execute("proof.packs", &json!({})).unwrap()["errors"]["fy"].is_string(), "the failure is reported");
    let before = s.doc().unwrap().revision;
    assert!(error(&mut s, "proof.installPack", json!({"locale": "nl_NL"})).contains("no Language pack for nl-nl"), "nl_NL reads as nl-nl");
    assert_eq!(s.execute("proof.installPack", &json!({"locale": "nl"})).unwrap()["installed"], true);
    assert!(s.doc().unwrap().revision > before, "the document is laid out again");
    assert_eq!(unknown(&mut s), [("fietz".to_string(), "nl".to_string())]);
    let listed = s.execute("proof.packs", &json!({})).unwrap();
    assert_eq!(listed["packs"][0]["installed"], true);
}

#[test]
fn an_install_in_the_background_reaches_the_documents_when_it_ends() {
    let _turn = packs_lock();
    let root = folder("background");
    let _reset = Reset(root.clone());
    let mut s = dutch_session();
    let without = unknown(&mut s);
    let fake = Arc::new(Fake { dir: root.clone(), downloads: true, hold: AtomicBool::new(true) });
    packs::set_backend(Some(fake.clone()));
    packs::run_in_background(true);
    assert_eq!(s.execute("proof.installPack", &json!({"locale": "nl"})).unwrap()["started"], true);
    assert_eq!(s.execute("proof.installPack", &json!({"locale": "nl"})).unwrap()["started"], true, "asked again, it is the same install");
    let listed = s.execute("proof.packs", &json!({})).unwrap();
    assert_eq!(listed["installing"][0]["locale"], "nl");
    assert!(error(&mut s, "proof.removePack", json!({"locale": "nl"})).contains("still installing"));
    assert_eq!(unknown(&mut s), without, "not used before it is installed");
    let before = s.doc().unwrap().revision;
    fake.hold.store(false, Ordering::Relaxed);
    while s.poll_language_packs() {
        std::thread::yield_now();
    }
    s.poll_language_packs();
    assert!(s.doc().unwrap().revision > before, "the document is laid out again");
    assert_eq!(unknown(&mut s), [("fietz".to_string(), "nl".to_string())]);
}

#[test]
fn a_cancelled_background_install_installs_nothing() {
    let _turn = packs_lock();
    let root = folder("cancel");
    let _reset = Reset(root.clone());
    let mut s = dutch_session();
    packs::set_backend(Some(Arc::new(Fake { dir: root.clone(), downloads: true, hold: AtomicBool::new(true) })));
    packs::run_in_background(true);
    assert_eq!(s.execute("proof.installPack", &json!({"locale": "nl"})).unwrap()["started"], true);
    packs::cancel("nl");
    while packs::busy() {
        std::thread::yield_now();
    }
    assert!(installed_locales(&mut s).is_empty());
    assert!(s.execute("proof.packs", &json!({})).unwrap()["errors"]["nl"].as_str().unwrap_or_default().contains("cancelled"));
}

/// A caller confined to automation roots lists the packs but can't fetch, install or remove them:
/// the store they are written into is outside its roots.
#[test]
fn a_confined_caller_lists_packs_but_cannot_change_them() {
    let _turn = packs_lock();
    let root = folder("confined");
    let _reset = Reset(root.clone());
    let roots_dir = folder("confined-roots");
    std::fs::create_dir_all(&roots_dir).unwrap();
    let mut s = dutch_session();
    write_dutch_pack(&root);
    packs::set_backend(Some(Arc::new(Fake { dir: root.clone(), downloads: true, hold: AtomicBool::new(false) })));
    let roots = crate::file_access::AutomationRoots::new(&[roots_dir.as_path()], &[roots_dir.as_path()]).unwrap();
    let mut confined = |cmd: &str, p: Value| crate::file_access::confine(roots.as_ref(), || s.execute(cmd, &p)).map_err(|e| e.to_string());
    for (cmd, p) in
        [("proof.installPack", json!({"locale": "nl"})), ("proof.removePack", json!({"locale": "nl"})), ("proof.packs", json!({"refresh": true}))]
    {
        let e = confined(cmd, p).unwrap_err();
        assert!(e.contains("automation path rejected"), "{cmd}: {e}");
    }
    assert_eq!(confined("proof.packs", json!({})).unwrap()["installed"][0]["locale"], "nl", "listing is allowed");
    assert!(root.join("nl").join("pack.toml").exists(), "the pack is still there");
    let _ = std::fs::remove_dir_all(&roots_dir);
}
