//! PDF security (Save PDF › Security): a password to open the file, a permissions password and
//! the permissions it guards, written with the standard security handler.
//!
//! The export writes a plain file; [`protect`] then encrypts every string and stream in it (the
//! XMP metadata stays readable with Plaintext Metadata) and adds the encryption dictionary. The
//! algorithm follows the compatibility ([`Encryption`]): 40-bit RC4 for PDF 1.3 (revision 2),
//! 128-bit RC4 for PDF 1.4 (revision 3) and 1.5 (revision 4), 128-bit AES for 1.6 (revision 4) and
//! 256-bit AES for 1.7 and 2.0 (revision 6). The random file key, salts and initialisation vectors
//! come from a SHA-256 stream seeded with the process's random hash keys, the time and the file
//! itself. A linearised file is encrypted between its two steps ([`protect_keeping`]): what
//! encrypted it then encrypts the hint stream written last ([`Cipher`]).
//!
//! Reading: [`user_password`] turns the permissions (owner) password of a file encrypted with RC4
//! or 128-bit AES into its open (user) password, so either password opens it.

use aes::cipher::{BlockEncrypt, KeyInit};
use md5::Md5;
use sha2::{Digest, Sha256, Sha384, Sha512};

use crate::lab_spot::{find, rfind, xref_offset};
use crate::settings::choice;
use crate::{Changes, Compatibility, PdfError, PdfSettings, Printing, SecuritySettings, Standard};

mod lexer;

pub(crate) use lexer::{Lexer, Obj, Spanned, Tok, int, is_white};

choice! {
    /// The encryption algorithm, which follows the PDF version (Compatibility).
    Encryption {
        Rc440 = "rc440", "40-bit RC4";
        Rc4 = "rc4", "128-bit RC4";
        Aes128 = "aes128", "128-bit AES";
        Aes256 = "aes256", "256-bit AES";
    } default Aes256
}

impl Encryption {
    /// The algorithm a file of PDF version `c` is encrypted with.
    pub fn for_compatibility(c: Compatibility) -> Self {
        match c {
            Compatibility::Pdf13 => Self::Rc440,
            Compatibility::Pdf14 | Compatibility::Pdf15 => Self::Rc4,
            Compatibility::Pdf16 => Self::Aes128,
            Compatibility::Pdf17 | Compatibility::Pdf20 => Self::Aes256,
        }
    }
}

/// The standard security handler revision written for PDF version `c`: 3 (RC4, metadata always
/// encrypted), 4 (crypt filters: RC4 or AES-128) or 6 (AES-256).
fn revision(c: Compatibility) -> u8 {
    match c {
        Compatibility::Pdf13 => 2,
        Compatibility::Pdf14 => 3,
        Compatibility::Pdf15 | Compatibility::Pdf16 => 4,
        Compatibility::Pdf17 | Compatibility::Pdf20 => 6,
    }
}

/// The longest password (in bytes) revisions 2–4 use: longer ones are cut to this.
const MAX_LEGACY_PASSWORD: usize = 32;
/// The longest password (UTF-8 bytes) revision 6 uses.
const MAX_PASSWORD: usize = 127;

impl SecuritySettings {
    /// A password is set: the file is encrypted.
    pub fn protected(&self) -> bool {
        !self.open_password.is_empty() || !self.permissions_password.is_empty()
    }

    /// Some permission is withheld (only a permissions password applies them).
    pub fn restricts(&self) -> bool {
        self.printing != Printing::High || self.changes != Changes::Any || !self.copy || !self.screen_reader
    }

    /// The encryption dictionary's `P` entry: the permission bits (1-based bit numbers of the PDF
    /// reference's user access permissions). Everything is allowed without a permissions password.
    pub fn permission_bits(&self) -> i32 {
        if self.permissions_password.is_empty() {
            return !3;
        }
        let bit = |n: u32| 1u32 << (n - 1);
        // Bits 1–2 are clear; the reserved bits 7–8 and 13–32 are set.
        let mut p = 0xFFFF_F0C0u32;
        p |= match self.printing {
            Printing::None => 0,
            Printing::Low => bit(3),
            Printing::High => bit(3) | bit(12),
        };
        p |= match self.changes {
            Changes::None => 0,
            Changes::Pages => bit(11),
            Changes::Forms => bit(9),
            Changes::Comments => bit(6) | bit(9),
            Changes::Any => bit(4) | bit(6) | bit(9) | bit(11),
        };
        if self.copy {
            p |= bit(5);
        }
        // Content copied may always be read out by screen readers.
        if self.copy || self.screen_reader {
            p |= bit(10);
        }
        p as i32
    }

    /// Refuse passwords the file can't carry: with a PDF standard (PDF/A and PDF/X forbid
    /// encryption), two equal passwords, or passwords the algorithm of `c` can't take.
    pub(crate) fn check(&self, standard: Standard, c: Compatibility) -> Result<(), PdfError> {
        if !self.protected() {
            return Ok(());
        }
        if standard != Standard::None {
            return Err(PdfError::BadSetting(format!(
                "{} files can't be password-protected: remove the passwords or choose no standard",
                standard.label()
            )));
        }
        // Revision 2 has one printing bit: it can't say "low resolution only".
        if revision(c) == 2 && self.printing == Printing::Low && !self.permissions_password.is_empty() {
            return Err(PdfError::BadSetting(format!(
                "{} can't limit printing to low resolution: choose PDF 1.4 or later, or allow high-resolution printing",
                c.label()
            )));
        }
        if self.open_password == self.permissions_password {
            return Err(PdfError::BadSetting("the open and permissions passwords must differ".into()));
        }
        for pw in [&self.open_password, &self.permissions_password] {
            if revision(c) < 6 {
                if !pw.is_ascii() || pw.len() > MAX_LEGACY_PASSWORD {
                    return Err(PdfError::BadSetting(format!(
                        "{} passwords can have up to {MAX_LEGACY_PASSWORD} ASCII characters (PDF 1.7 and later take any)",
                        c.label()
                    )));
                }
            } else if pw.len() > MAX_PASSWORD {
                return Err(PdfError::BadSetting(format!("passwords can have up to {MAX_PASSWORD} bytes")));
            }
        }
        Ok(())
    }
}

impl PdfSettings {
    /// The algorithm the file is encrypted with; `None` without a password.
    pub fn encryption(&self) -> Option<Encryption> {
        self.security.protected().then(|| Encryption::for_compatibility(self.compatibility))
    }
}

/// The security options that are accepted but have no effect, one warning each.
pub(crate) fn warnings(set: &PdfSettings) -> Vec<String> {
    let s = &set.security;
    let mut out = vec![];
    if s.protected() && s.plaintext_metadata && revision(set.compatibility) <= 3 {
        out.push(format!("{} encryption covers the metadata too: plaintext metadata needs PDF 1.5 or later", set.compatibility.label()));
    }
    if s.protected() && set.compatibility == Compatibility::Pdf13 {
        out.push(format!(
            "{} encrypts with a 40-bit key, which can be searched in minutes: choose PDF 1.6 or later for stronger protection",
            set.compatibility.label()
        ));
    }
    out
}

// ---------- cryptography ----------

/// Padding of passwords (revisions 2–4).
const PAD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08, 0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80,
    0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

fn md5(parts: &[&[u8]]) -> [u8; 16] {
    let mut h = Md5::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

/// `pw` cut or padded to 32 bytes.
fn padded(pw: &[u8]) -> [u8; 32] {
    let mut out = PAD;
    let n = pw.len().min(32);
    out[..n].copy_from_slice(&pw[..n]);
    out[n..].copy_from_slice(&PAD[..32 - n]);
    out
}

fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    if key.is_empty() {
        return data.to_vec();
    }
    let mut s: [u8; 256] = std::array::from_fn(|i| i as u8);
    let mut j = 0u8;
    for (i, k) in (0..256).zip(key.iter().cycle()) {
        j = j.wrapping_add(s[i]).wrapping_add(*k);
        s.swap(i, usize::from(j));
    }
    let (mut i, mut j) = (0u8, 0u8);
    data.iter()
        .map(|b| {
            i = i.wrapping_add(1);
            j = j.wrapping_add(s[usize::from(i)]);
            s.swap(usize::from(i), usize::from(j));
            b ^ s[usize::from(s[usize::from(i)].wrapping_add(s[usize::from(j)]))]
        })
        .collect()
}

/// `key` with every byte XORed with `i` (the 19 extra RC4 passes of revisions 3–4).
fn xored(key: &[u8], i: u8) -> Vec<u8> {
    key.iter().map(|b| b ^ i).collect()
}

/// An AES block cipher with a 128- or 256-bit key.
enum Aes {
    A128(Box<aes::Aes128>),
    A256(Box<aes::Aes256>),
}

impl Aes {
    fn new(key: &[u8]) -> Result<Self, PdfError> {
        let bad = |_| PdfError::Write("bad AES key".into());
        match key.len() {
            16 => aes::Aes128::new_from_slice(key).map(|c| Self::A128(Box::new(c))).map_err(bad),
            _ => aes::Aes256::new_from_slice(key).map(|c| Self::A256(Box::new(c))).map_err(bad),
        }
    }

    fn block(&self, b: [u8; 16]) -> [u8; 16] {
        let mut block = aes::Block::from(b);
        match self {
            Self::A128(c) => c.encrypt_block(&mut block),
            Self::A256(c) => c.encrypt_block(&mut block),
        }
        block.into()
    }

    /// CBC from `iv`; `pad`: PKCS #7 padding (else a trailing partial block is dropped).
    fn cbc(&self, iv: [u8; 16], data: &[u8], pad: bool) -> Vec<u8> {
        let mut out = Vec::with_capacity(data.len() + 16);
        let mut prev = iv;
        let mut put = |chunk: [u8; 16], out: &mut Vec<u8>| {
            prev = self.block(std::array::from_fn(|i| chunk[i] ^ prev[i]));
            out.extend_from_slice(&prev);
        };
        let (chunks, rest) = data.as_chunks::<16>();
        for c in chunks {
            put(*c, &mut out);
        }
        if pad {
            let n = (16 - rest.len()) as u8;
            put(std::array::from_fn(|i| rest.get(i).copied().unwrap_or(n)), &mut out);
        }
        out
    }
}

/// The first `N` bytes of `b` (zero-filled when shorter).
fn first<const N: usize>(b: &[u8]) -> [u8; N] {
    std::array::from_fn(|i| b.get(i).copied().unwrap_or_default())
}

/// The revision 6 password hash of `password` with `salt` (and the 48-byte U entry when hashing
/// the owner password).
fn hash6(password: &[u8], salt: &[u8], udata: &[u8]) -> Result<[u8; 32], PdfError> {
    let mut k: Vec<u8> = Sha256::new().chain_update(password).chain_update(salt).chain_update(udata).finalize().to_vec();
    let mut round = 0u32;
    loop {
        let mut k1 = Vec::with_capacity(64 * (password.len() + k.len() + udata.len()));
        for _ in 0..64 {
            k1.extend_from_slice(password);
            k1.extend_from_slice(&k);
            k1.extend_from_slice(udata);
        }
        let key: [u8; 16] = first(&k);
        let e = Aes::new(&key)?.cbc(first(k.get(16..).unwrap_or_default()), &k1, false);
        // The first 16 bytes of E as a big-endian number, modulo 3 (256 ≡ 1 mod 3).
        k = match e.iter().take(16).map(|b| u32::from(*b)).sum::<u32>() % 3 {
            0 => Sha256::digest(&e).to_vec(),
            1 => Sha384::digest(&e).to_vec(),
            _ => Sha512::digest(&e).to_vec(),
        };
        round += 1;
        if round >= 64 && u32::from(e.last().copied().unwrap_or_default()) <= round - 32 {
            return Ok(first(&k));
        }
    }
}

/// Unpredictable bytes: SHA-256 of a seed and a counter.
struct Entropy {
    seed: [u8; 32],
    counter: u64,
}

impl Entropy {
    /// Seeded with the process's random hash keys (none on the web), the time and `material`.
    fn new(material: &[&[u8]]) -> Self {
        use std::hash::BuildHasher;
        let state = std::hash::RandomState::new();
        let mut h = Sha256::new();
        for i in 0u8..4 {
            h.update(state.hash_one(i).to_le_bytes());
        }
        h.update(vectorcraft_doc::metadata::now_unix().unwrap_or_default().to_le_bytes());
        for m in material {
            h.update((m.len() as u64).to_le_bytes());
            h.update(m);
        }
        Self { seed: h.finalize().into(), counter: 0 }
    }

    fn bytes<const N: usize>(&mut self) -> [u8; N] {
        let mut out = [0u8; N];
        for chunk in out.chunks_mut(32) {
            self.counter += 1;
            let block: [u8; 32] = Sha256::new().chain_update(self.seed).chain_update(self.counter.to_le_bytes()).finalize().into();
            chunk.copy_from_slice(&block[..chunk.len()]);
        }
        out
    }
}

fn hex(b: &[u8]) -> String {
    let mut s = String::with_capacity(2 * b.len() + 2);
    s.push('<');
    for x in b {
        s.push_str(&format!("{x:02X}"));
    }
    s.push('>');
    s
}

/// What encrypts the strings and streams of one file.
struct Handler {
    /// The file encryption key.
    key: Vec<u8>,
    cipher: Encryption,
    encrypt_metadata: bool,
    /// The encryption dictionary.
    dict: String,
}

impl Handler {
    /// The handler for `sec` at version `c`, for a file whose first ID is `id`.
    fn new(sec: &SecuritySettings, c: Compatibility, id: &[u8], rng: &mut Entropy) -> Result<Self, PdfError> {
        let r = revision(c);
        let cipher = Encryption::for_compatibility(c);
        let user = sec.open_password.as_bytes();
        // Without a permissions password the open password grants every permission anyway.
        let owner = if sec.permissions_password.is_empty() { user } else { sec.permissions_password.as_bytes() };
        let p = sec.permission_bits();
        let encrypt_metadata = r <= 3 || !sec.plaintext_metadata;
        let meta = if encrypt_metadata { "" } else { "/EncryptMetadata false" };
        if r == 6 {
            let key: [u8; 32] = rng.bytes();
            let (uv, uk): ([u8; 8], [u8; 8]) = (rng.bytes(), rng.bytes());
            let u = [&hash6(user, &uv, &[])?[..], &uv[..], &uk[..]].concat();
            let ue = Aes::new(&hash6(user, &uk, &[])?)?.cbc([0; 16], &key, false);
            let (ov, ok): ([u8; 8], [u8; 8]) = (rng.bytes(), rng.bytes());
            let o = [&hash6(owner, &ov, &u)?[..], &ov[..], &ok[..]].concat();
            let oe = Aes::new(&hash6(owner, &ok, &u)?)?.cbc([0; 16], &key, false);
            let mut perms = [0xFFu8; 16];
            perms[..4].copy_from_slice(&p.to_le_bytes());
            perms[8] = if encrypt_metadata { b'T' } else { b'F' };
            perms[9..12].copy_from_slice(b"adb");
            perms[12..].copy_from_slice(&rng.bytes::<4>());
            let perms = Aes::new(&key)?.block(perms);
            let dict = format!(
                "<</Filter/Standard/V 5/R 6/Length 256/CF<</StdCF<</AuthEvent/DocOpen/CFM/AESV3/Length 32>>>>/StmF/StdCF/StrF/StdCF/P {p}/O{}/U{}/OE{}/UE{}/Perms{}{meta}>>",
                hex(&o),
                hex(&u),
                hex(&oe),
                hex(&ue),
                hex(&perms)
            );
            return Ok(Self { key: key.to_vec(), cipher, encrypt_metadata, dict });
        }
        // O (algorithm 3), the file key (algorithm 2) and U (algorithms 4 and 5): a 5-byte key and
        // single RC4 passes for revision 2, a 16-byte key for revisions 3 and 4.
        let n = if r == 2 { 5 } else { 16 };
        let owner_key = owner_key(owner, r, n);
        let mut o = rc4(&owner_key, &padded(user));
        for i in (1..=19).filter(|_| r >= 3) {
            o = rc4(&xored(&owner_key, i), &o);
        }
        let no_meta: &[u8] = if encrypt_metadata { &[] } else { &[0xFF; 4] };
        let mut key = md5(&[&padded(user), &o, &p.to_le_bytes(), id, no_meta]);
        for _ in (0..50).filter(|_| r >= 3) {
            key = md5(&[&key]);
        }
        let key = key.get(..n).unwrap_or(&key).to_vec();
        let u = if r == 2 {
            rc4(&key, &PAD)
        } else {
            let mut u = rc4(&key, &md5(&[&PAD, id]));
            for i in 1..=19 {
                u = rc4(&xored(&key, i), &u);
            }
            u.extend_from_slice(&rng.bytes::<16>());
            u
        };
        let dict = match r {
            2 => format!("<</Filter/Standard/V 1/R 2/Length 40/P {p}/O{}/U{}>>", hex(&o), hex(&u)),
            3 => format!("<</Filter/Standard/V 2/R 3/Length 128/P {p}/O{}/U{}>>", hex(&o), hex(&u)),
            _ => {
                let cfm = if cipher == Encryption::Aes128 { "AESV2" } else { "V2" };
                format!(
                    "<</Filter/Standard/V 4/R 4/Length 128/CF<</StdCF<</AuthEvent/DocOpen/CFM/{cfm}/Length 16>>>>/StmF/StdCF/StrF/StdCF/P {p}/O{}/U{}{meta}>>",
                    hex(&o),
                    hex(&u)
                )
            }
        };
        Ok(Self { key, cipher, encrypt_metadata, dict })
    }

    /// `data` of object `num generation` encrypted.
    fn encrypt(&self, num: u32, generation: u16, data: &[u8], rng: &mut Entropy) -> Result<Vec<u8>, PdfError> {
        // The object's key: the file key's length plus 5 bytes, at most 16 (algorithm 1).
        let object_key = || {
            let salt: &[u8] = if self.cipher == Encryption::Aes128 { b"sAlT" } else { &[] };
            let h = md5(&[&self.key, &num.to_le_bytes()[..3], &generation.to_le_bytes(), salt]);
            h.get(..(self.key.len() + 5).min(16)).unwrap_or(&h).to_vec()
        };
        let aes = |key: &[u8], rng: &mut Entropy| -> Result<Vec<u8>, PdfError> {
            let iv: [u8; 16] = rng.bytes();
            Ok([&iv[..], &Aes::new(key)?.cbc(iv, data, true)].concat())
        };
        match self.cipher {
            Encryption::Rc440 | Encryption::Rc4 => Ok(rc4(&object_key(), data)),
            Encryption::Aes128 => aes(&object_key(), rng),
            Encryption::Aes256 => aes(&self.key, rng),
        }
    }
}

/// The RC4 key the owner password encrypts O with (algorithm 3, steps a–d).
fn owner_key(owner: &[u8], r: u8, n: usize) -> Vec<u8> {
    let mut h = md5(&[&padded(owner)]);
    if r >= 3 {
        for _ in 0..50 {
            h = md5(&[h.get(..n).unwrap_or(&h)]);
        }
    }
    h.get(..n).unwrap_or(&h).to_vec()
}

// ---------- encrypting a written file ----------

fn failed(why: &str) -> PdfError {
    PdfError::Write(format!("can't encrypt the PDF: {why}"))
}

/// A replacement of bytes `start..end` of the file.
type Edit = (usize, usize, Vec<u8>);

/// The catalog entry declaring 256-bit AES in a PDF 1.7 file (extension level 8).
const AES256_EXTENSION: &[u8] = b"/Extensions<</ADBE<</BaseVersion/1.7/ExtensionLevel 8>>>>";

/// An in-use cross-reference entry: object number, generation, offset.
type Entry = (u32, u16, usize);

/// The in-use entries of the cross-reference table at `xref` and the trailer dictionary after it.
fn xref_table(pdf: &[u8], xref: usize) -> Result<(Vec<Entry>, Obj<'_>), PdfError> {
    let mut lx = Lexer::at(pdf, xref + 4);
    let mut entries = vec![];
    loop {
        let word = |t: Option<Spanned<'_>>| match t {
            Some((Tok::Word(w), ..)) => int(w),
            _ => None,
        };
        let mut look = lx.clone();
        if let Some((Tok::Word(b"trailer"), ..)) = look.next() {
            lx = look;
            break;
        }
        let (first, count) = (word(lx.next()), word(lx.next()));
        let (Some(first), Some(count)) = (first, count) else { return Err(failed("bad cross-reference table")) };
        for i in 0..count {
            let (off, generation, kind) = (word(lx.next()), word(lx.next()), lx.next());
            let (Some(off), Some(generation), Some((Tok::Word(kind), ..))) = (off, generation, kind) else {
                return Err(failed("bad cross-reference entry"));
            };
            if kind == b"n" {
                let num = u32::try_from(first + i).map_err(|_| failed("bad object number"))?;
                entries.push((num, u16::try_from(generation).unwrap_or(0), usize::try_from(off).map_err(|_| failed("bad offset"))?));
            }
        }
    }
    let trailer = lx.object(0).ok_or_else(|| failed("bad trailer"))?;
    Ok((entries, trailer))
}

/// The edits encrypting the object at `off` (its strings and stream data); `catalog`: add the
/// 256-bit AES extension to it.
fn object_edits(pdf: &[u8], off: usize, h: &Handler, catalog: bool, rng: &mut Entropy, edits: &mut Vec<Edit>) -> Result<(), PdfError> {
    let mut lx = Lexer::at(pdf, off);
    let (num, generation) = lx.header().ok_or_else(|| failed("an object's header is missing"))?;
    let obj = lx.object(0).ok_or_else(|| failed("unreadable object"))?;
    let mut strings = vec![];
    obj.strings(&mut strings);
    for s in strings {
        if let Obj::Str { bytes, start, end } = s {
            edits.push((*start, *end, hex(&h.encrypt(num, generation, bytes, rng)?).into_bytes()));
        }
    }
    if let (true, Obj::Dict { start, .. }) = (catalog, &obj)
        && obj.get(b"Extensions").is_none()
    {
        edits.push((start + 2, start + 2, AES256_EXTENSION.to_vec()));
    }
    let Some(((len_start, len_end), (data, data_end))) = stream_spans(pdf, &mut lx, &obj).map_err(failed)? else { return Ok(()) };
    let ty = obj.name(b"Type");
    if matches!(ty, Some(b"XRef" | b"ObjStm")) {
        return Err(failed("cross-reference and object streams aren't supported"));
    }
    if ty == Some(&b"Metadata"[..]) && !h.encrypt_metadata {
        return Ok(());
    }
    let bytes = pdf.get(data..data_end).ok_or_else(|| failed("a stream runs past the end"))?;
    let enc = h.encrypt(num, generation, bytes, rng)?;
    edits.push((len_start, len_end, enc.len().to_string().into_bytes()));
    edits.push((data, data_end, enc));
    Ok(())
}

/// The spans of a stream's length value and of its data.
pub(crate) type StreamSpans = ((usize, usize), (usize, usize));

/// Where the object whose dictionary `dict` the lexer `lx` has just read keeps its stream data:
/// its [`StreamSpans`]; `None` when it isn't a stream.
pub(crate) fn stream_spans(pdf: &[u8], lx: &mut Lexer<'_>, dict: &Obj<'_>) -> Result<Option<StreamSpans>, &'static str> {
    let Some((Tok::Word(b"stream"), _, keyword_end)) = lx.next() else { return Ok(None) };
    let Some(Obj::Int { value, start, end }) = dict.get(b"Length") else { return Err("a stream's length isn't a number") };
    // The data starts after the end of line that follows `stream`.
    let mut data = keyword_end;
    if pdf.get(data) == Some(&b'\r') {
        data += 1;
    }
    if pdf.get(data) == Some(&b'\n') {
        data += 1;
    }
    let data_end = usize::try_from(*value).ok().and_then(|n| data.checked_add(n)).ok_or("bad stream length")?;
    if data_end > pdf.len() {
        return Err("a stream runs past the end");
    }
    if !matches!(Lexer::at(pdf, data_end).next(), Some((Tok::Word(b"endstream"), ..))) {
        return Err("a stream's length is wrong");
    }
    Ok(Some(((*start, *end), (data, data_end))))
}

/// What encrypts the streams of a file [`protect_keeping`] encrypted, for those written after it
/// (the hint stream of a linearised file).
pub(crate) struct Cipher {
    h: Handler,
    rng: Entropy,
}

impl Cipher {
    /// `data`, the stream of object `num` (generation 0), encrypted.
    pub(crate) fn stream(&mut self, num: u32, data: &[u8]) -> Result<Vec<u8>, PdfError> {
        self.h.encrypt(num, 0, data, &mut self.rng)
    }
}

/// Encrypt `pdf` (as the export writes it: one cross-reference table) as `set`'s Security
/// section says; without a password it comes back as it is.
pub(crate) fn protect(pdf: Vec<u8>, set: &PdfSettings) -> Result<Vec<u8>, PdfError> {
    protect_keeping(pdf, set).map(|(pdf, _)| pdf)
}

/// [`protect`], keeping what encrypted the file (`None` when it isn't encrypted). The objects keep
/// their order and numbers; the encryption dictionary comes after them, numbered last.
pub(crate) fn protect_keeping(pdf: Vec<u8>, set: &PdfSettings) -> Result<(Vec<u8>, Option<Cipher>), PdfError> {
    let sec = &set.security;
    if !sec.protected() {
        return Ok((pdf, None));
    }
    let xref = xref_offset(&pdf).ok_or_else(|| failed("no cross-reference table"))?;
    let (mut objects, trailer) = xref_table(&pdf, xref)?;
    let Obj::Dict { start: t_start, end: t_end, .. } = &trailer else { return Err(failed("bad trailer")) };
    if trailer.get(b"Prev").is_some() || trailer.get(b"Encrypt").is_some() {
        return Err(failed("the file was updated or encrypted already"));
    }
    let Some(Obj::Int { value: size, start: size_start, end: size_end }) = trailer.get(b"Size") else {
        return Err(failed("the trailer has no size"));
    };
    let size = u32::try_from(*size).ok().filter(|s| *s as usize <= 2 * objects.len() + 16).ok_or_else(|| failed("bad trailer size"))?;
    let root = match trailer.get(b"Root") {
        Some(Obj::Ref(n, _)) => Some(*n),
        _ => None,
    };
    let mut rng = Entropy::new(&[sec.open_password.as_bytes(), sec.permissions_password.as_bytes(), &pdf]);
    let (id, new_id) = match trailer.first_string(b"ID") {
        Some(id) => (id.to_vec(), None),
        None => {
            let id: [u8; 16] = rng.bytes();
            (id.to_vec(), Some(id))
        }
    };
    let h = Handler::new(sec, set.compatibility, &id, &mut rng)?;
    let extension = h.cipher == Encryption::Aes256 && set.compatibility == Compatibility::Pdf17;

    objects.sort_by_key(|o| o.2);
    let mut edits = vec![];
    for (num, _, off) in &objects {
        object_edits(&pdf, *off, &h, extension && root == Some(*num), &mut rng, &mut edits)?;
    }
    edits.sort_by_key(|e| (e.0, e.1));

    // The objects with their edits, recording where each one now starts.
    let mut out = Vec::with_capacity(pdf.len() + pdf.len() / 8 + 1024);
    let mut offsets = vec![None; size as usize + 1];
    let (mut at, mut pending) = (0, edits.iter().peekable());
    let mut copy_to = |to: usize, out: &mut Vec<u8>, at: &mut usize| -> Result<(), PdfError> {
        while let Some((s, e, new)) = pending.next_if(|e| e.0 < to) {
            out.extend_from_slice(pdf.get(*at..*s).ok_or_else(|| failed("overlapping edits"))?);
            out.extend_from_slice(new);
            *at = *e;
        }
        out.extend_from_slice(pdf.get(*at..to).ok_or_else(|| failed("overlapping edits"))?);
        *at = to;
        Ok(())
    };
    for (num, generation, off) in &objects {
        copy_to(*off, &mut out, &mut at)?;
        let slot = offsets.get_mut(*num as usize).filter(|_| *num < size).ok_or_else(|| failed("bad object number"))?;
        *slot = Some((out.len(), *generation));
    }
    copy_to(xref, &mut out, &mut at)?;
    if let Some(slot) = offsets.get_mut(size as usize) {
        *slot = Some((out.len(), 0));
    }
    out.extend_from_slice(format!("{size} 0 obj\n{}\nendobj\n", h.dict).as_bytes());

    // A new cross-reference table and the trailer naming the encryption dictionary.
    let table = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", size + 1).as_bytes());
    for o in &offsets {
        let entry = match o {
            Some((off, generation)) => format!("{off:010} {generation:05} n\r\n"),
            None => "0000000000 65535 f\r\n".to_string(),
        };
        out.extend_from_slice(entry.as_bytes());
    }
    out.extend_from_slice(b"trailer\n");
    let dict_body = t_end.checked_sub(2).filter(|b| *b >= *size_end).ok_or_else(|| failed("bad trailer"))?;
    out.extend_from_slice(pdf.get(*t_start..*size_start).ok_or_else(|| failed("bad trailer"))?);
    out.extend_from_slice((size + 1).to_string().as_bytes());
    out.extend_from_slice(pdf.get(*size_end..dict_body).ok_or_else(|| failed("bad trailer"))?);
    if let Some(id) = new_id {
        out.extend_from_slice(format!("/ID[{}{}]", hex(&id), hex(&id)).as_bytes());
    }
    out.extend_from_slice(format!("/Encrypt {size} 0 R>>\nstartxref\n{table}\n%%EOF").as_bytes());
    Ok((out, Some(Cipher { h, rng })))
}

// ---------- reading ----------

/// The trailer dictionary of `pdf`: after its last cross-reference table, or the dictionary of
/// its last cross-reference stream.
fn trailer(pdf: &[u8]) -> Option<Obj<'_>> {
    let sx = rfind(pdf, b"startxref")?;
    let mut lx = Lexer::at(pdf, sx + 9);
    let off = match lx.next()? {
        (Tok::Word(w), ..) => usize::try_from(int(w)?).ok()?,
        _ => return None,
    };
    if pdf.get(off..)?.starts_with(b"xref") {
        let at = find(pdf, b"trailer", off)?;
        return Lexer::at(pdf, at + 7).object(0);
    }
    let mut lx = Lexer::at(pdf, off);
    lx.header()?;
    lx.object(0)
}

/// The object `num generation` of `pdf`, found by its header (the last one in the file).
fn find_object(pdf: &[u8], num: u32, generation: u16) -> Option<Obj<'_>> {
    let head = format!("{num} {generation} obj");
    let mut end = pdf.len();
    while let Some(at) = rfind(pdf.get(..end)?, head.as_bytes()) {
        if at == 0 || pdf.get(at - 1).is_some_and(|b| is_white(*b)) {
            let mut lx = Lexer::at(pdf, at);
            lx.header()?;
            return lx.object(0);
        }
        end = at + head.len() - 1;
    }
    None
}

/// The open (user) password of `pdf`, from its permissions (owner) password `owner`: for files
/// encrypted with RC4 or 128-bit AES (revisions 2–4), whose user password is in their O entry.
/// `None` for other files, a wrong password or a user password that isn't UTF-8.
pub(crate) fn user_password(pdf: &[u8], owner: &str) -> Option<String> {
    let trailer = trailer(pdf)?;
    let found;
    let enc = match trailer.get(b"Encrypt")? {
        Obj::Ref(n, g) => {
            found = find_object(pdf, *n, *g)?;
            &found
        }
        d @ Obj::Dict { .. } => d,
        _ => return None,
    };
    let r = u8::try_from(enc.int(b"R")?).ok().filter(|r| (2..=4).contains(r))?;
    let n = if r == 2 { 5 } else { usize::try_from(enc.int(b"Length").unwrap_or(40) / 8).ok()?.clamp(5, 16) };
    let key = owner_key(owner.as_bytes(), r, n);
    let mut user = enc.string(b"O")?.get(..32)?.to_vec();
    if r == 2 {
        user = rc4(&key, &user);
    } else {
        for i in (0..=19).rev() {
            user = rc4(&xored(&key, i), &user);
        }
    }
    // The password is what comes before the padding.
    let len = (0..=32).find(|l| user.get(*l..) == PAD.get(..32 - l))?;
    String::from_utf8(user.get(..len)?.to_vec()).ok()
}
