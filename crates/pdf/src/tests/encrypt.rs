//! PDF security: password-protected files open only with a password, carry the permissions asked
//! for, and refuse what a PDF standard forbids.

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, NodeKind};
use vectorcraft_geom::{Rect, shapes};

use crate::encrypt::protect;
use crate::lab_spot::find;
use crate::*;

const VERSIONS: [Compatibility; 5] = [Compatibility::Pdf14, Compatibility::Pdf15, Compatibility::Pdf16, Compatibility::Pdf17, Compatibility::Pdf20];

fn doc() -> Document {
    let mut d = Document::new(200.0, 100.0);
    d.title = "Tïtle (x)".into();
    let mut n = Node::path(
        NodeId(0),
        shapes::rectangle(Rect::new(10.0, 10.0, 60.0, 40.0)),
        Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0),
    );
    n.id = d.alloc_id();
    let layer = d.default_layer().unwrap();
    d.insert(Some(layer), 0, n).unwrap();
    d
}

fn security(open: &str, permissions: &str) -> SecuritySettings {
    SecuritySettings { open_password: open.into(), permissions_password: permissions.into(), ..Default::default() }
}

/// `doc()` exported at `version` (with Preserve Editing when `native`), then protected by `sec`.
fn protected(version: Compatibility, sec: SecuritySettings, native: bool) -> Vec<u8> {
    let mut opts = PdfOptions::default();
    opts.settings.compatibility = version;
    opts.settings.preserve_editing = native;
    opts.native = native.then(|| b"{\"native\": true}".to_vec());
    let plain = export(&doc(), &opts).unwrap();
    opts.settings.security = sec;
    protect(plain, &opts.settings).unwrap()
}

fn shapes_of(bytes: &[u8], password: Option<&str>) -> usize {
    let d = import_with_report(bytes, &ImportOptions { password: password.map(str::to_string), ..Default::default() }).unwrap().document;
    let NodeKind::Layer { children, .. } = &d.layers[0].kind else { panic!("a layer") };
    children.len()
}

/// The encryption dictionary's text.
fn encrypt_dict(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let at = text.find("/Filter/Standard").unwrap();
    text[at..at + text[at..].find("\nendobj").unwrap()].to_string()
}

#[test]
fn every_level_opens_only_with_a_password() {
    for (v, (head, cfm)) in VERSIONS.into_iter().zip([
        ("/V 2/R 3/Length 128", ""),
        ("/V 4/R 4/Length 128", "/CFM/V2"),
        ("/V 4/R 4/Length 128", "/CFM/AESV2"),
        ("/V 5/R 6/Length 256", "/CFM/AESV3"),
        ("/V 5/R 6/Length 256", "/CFM/AESV3"),
    ]) {
        let bytes = protected(v, security("open sesame", "owner"), false);
        let dict = encrypt_dict(&bytes);
        assert!(dict.contains(head) && dict.contains(cfm), "{v:?}: {dict}");
        assert!(bytes.starts_with(format!("%PDF-{}", v.id()).as_bytes()), "{v:?} keeps its version");
        assert_eq!(info(&bytes, None).unwrap_err(), PdfError::NeedsPassword, "{v:?}");
        assert_eq!(info(&bytes, Some("guess")).unwrap_err(), PdfError::WrongPassword, "{v:?}");
        // The open password and the permissions password both open it, and the art decrypts.
        assert_eq!(shapes_of(&bytes, Some("open sesame")), 1, "{v:?}");
        assert_eq!(shapes_of(&bytes, Some("owner")), 1, "{v:?}: the permissions password opens it too");
    }
}

#[test]
fn strings_and_streams_are_encrypted() {
    let mut sec = security("pw", "");
    sec.plaintext_metadata = false;
    let bytes = protected(Compatibility::Pdf17, sec, true);
    let text = String::from_utf8_lossy(&bytes);
    for plain in ["VectorCraft", "native", "xmpmeta", "FEFF0054"] {
        assert!(!text.contains(plain), "`{plain}` is readable");
    }
    // The trailer names the dictionary; the ID stays as it was.
    let trailer = &text[text.rfind("trailer").unwrap()..];
    assert!(trailer.contains("/Encrypt ") && trailer.contains("/ID["), "{trailer}");
    // 256-bit AES in a PDF 1.7 file is declared in the catalog.
    assert!(text.contains("/Extensions<</ADBE<</BaseVersion/1.7/ExtensionLevel 8>>>>"));
    assert!(!String::from_utf8_lossy(&protected(Compatibility::Pdf20, security("pw", ""), false)).contains("/Extensions"));
}

#[test]
fn plaintext_metadata_stays_readable() {
    for v in [Compatibility::Pdf15, Compatibility::Pdf16, Compatibility::Pdf17] {
        let bytes = protected(v, security("pw", ""), false);
        assert!(find(&bytes, b"<x:xmpmeta", 0).is_some(), "{v:?}: the XMP metadata is plain");
        assert!(encrypt_dict(&bytes).contains("/EncryptMetadata false"), "{v:?}");
        assert_eq!(shapes_of(&bytes, Some("pw")), 1, "{v:?}");
    }
    // PDF 1.4 encrypts it anyway, and says so.
    let bytes = protected(Compatibility::Pdf14, security("pw", ""), false);
    assert!(find(&bytes, b"<x:xmpmeta", 0).is_none());
    let notes = |s: &PdfSettings| s.warnings(&ActiveProfiles::default()).into_iter().map(|w| w.message).collect::<Vec<_>>();
    let set = PdfSettings { compatibility: Compatibility::Pdf14, security: security("pw", ""), ..Default::default() };
    assert!(notes(&set).iter().any(|w| w.contains("plaintext metadata")), "{:?}", notes(&set));
    let set = PdfSettings { compatibility: Compatibility::Pdf15, ..set };
    assert!(notes(&set).is_empty(), "{:?}", notes(&set));
}

#[test]
fn a_permissions_password_alone_opens_without_a_password_and_restricts() {
    for v in VERSIONS {
        let sec = SecuritySettings { printing: Printing::Low, changes: Changes::Comments, copy: false, ..security("", "owner only") };
        let bytes = protected(v, sec.clone(), false);
        assert_eq!(shapes_of(&bytes, None), 1, "{v:?} opens without a password");
        assert!(encrypt_dict(&bytes).contains(&format!("/P {}", sec.permission_bits())), "{v:?}");
    }
}

#[test]
fn unicode_passwords_open_aes256_files() {
    let bytes = protected(Compatibility::Pdf20, security("pässwörd", "ünïcode"), false);
    assert_eq!(info(&bytes, Some("pässwörd")).unwrap().pages.len(), 1);
    assert_eq!(info(&bytes, Some("ünïcode")).unwrap().pages.len(), 1);
    assert_eq!(info(&bytes, Some("passwort")).unwrap_err(), PdfError::WrongPassword);
}

#[test]
fn editing_data_comes_back_with_the_password() {
    // The editing file is found by its name (a string) and read from its stream: both decrypt.
    for v in VERSIONS {
        let bytes = protected(v, security("pw", "own"), true);
        assert_eq!(editing(&bytes), None, "{v:?}: locked");
        for pw in ["pw", "own"] {
            let e = editing_with(&bytes, Some(pw)).unwrap();
            assert_eq!((e.data.as_slice(), e.intact), (&b"{\"native\": true}"[..], true), "{v:?} {pw}");
        }
    }
}

#[test]
fn without_a_password_nothing_changes() {
    let mut opts = PdfOptions::default();
    let plain = export(&doc(), &opts).unwrap();
    opts.settings.security = SecuritySettings { printing: Printing::None, ..Default::default() };
    assert_eq!(protect(plain.clone(), &opts.settings).unwrap(), plain);
    assert_eq!(opts.settings.encryption(), None);
    opts.settings.security.open_password = "x".into();
    assert_eq!(opts.settings.encryption(), Some(Encryption::Aes256));
    for (c, e) in VERSIONS.into_iter().zip([Encryption::Rc4, Encryption::Rc4, Encryption::Aes128, Encryption::Aes256, Encryption::Aes256]) {
        assert_eq!(Encryption::for_compatibility(c), e);
    }
}

#[test]
fn the_permissions_password_opens_files_of_other_writers() {
    use vectorcraft_testkit::pdf::{PdfPage, pdf};
    // Revision 2 (40-bit RC4): the owner password is the user password followed by `-owner`.
    let bytes = pdf(&[PdfPage::new(50.0, 50.0, "0 g 0 0 10 10 re f")], Some("pw"));
    assert_eq!(info(&bytes, Some("pw-owner")).unwrap().pages.len(), 1);
    assert_eq!(info(&bytes, Some("pw-other")).unwrap_err(), PdfError::WrongPassword);
}

#[test]
fn damaged_files_never_panic_when_reading_the_owner_password() {
    let bytes = protected(Compatibility::Pdf15, security("pw", "own"), false);
    for cut in (0..bytes.len()).step_by(97) {
        let _ = crate::encrypt::user_password(&bytes[..cut], "own");
        let _ = info(&bytes[cut..], Some("own"));
    }
    for junk in [&b""[..], b"startxref", b"startxref\n99999999999999999999", b"trailer<</Encrypt<</R 3/O()>>>>startxref 0"] {
        assert_eq!(crate::encrypt::user_password(junk, "own"), None);
    }
}

#[test]
fn the_same_file_and_passwords_get_fresh_keys_each_time() {
    // Same document, same passwords, same second: the salts and the file key still differ.
    let mut opts = PdfOptions::default();
    opts.settings.compatibility = Compatibility::Pdf20;
    let plain = export(&doc(), &opts).unwrap();
    opts.settings.security = security("pw", "own");
    let a = protect(plain.clone(), &opts.settings).unwrap();
    let b = protect(plain, &opts.settings).unwrap();
    assert_ne!(encrypt_dict(&a), encrypt_dict(&b));
    for bytes in [&a, &b] {
        assert_eq!(shapes_of(bytes, Some("pw")), 1);
        assert_eq!(shapes_of(bytes, Some("own")), 1);
    }
}

#[test]
fn the_file_key_is_seeded_from_the_os_random_source() {
    // `RandomState` is no CSPRNG and its keys are fixed on the web: the seed must come from the OS.
    let src = include_str!("../encrypt.rs");
    assert!(!src.contains("RandomState"), "encrypt.rs seeds its keys from RandomState");
    assert!(src.contains("getrandom::fill"), "encrypt.rs doesn't seed its keys from the OS random source");
}
