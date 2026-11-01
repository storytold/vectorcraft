//! Opening a PDF whose pages share one large content stream, with an editing file naming a page
//! hash, keeps a working set that doesn't grow with the page count. One test in its own process,
//! so the allocator's peak is this test's.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// A counting allocator forwards to the system one.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use vectorcraft_pdf::{EDITING_FILE, editing_with};

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

struct Counting;

fn grew(n: usize) {
    PEAK.fetch_max(LIVE.fetch_add(n, Relaxed) + n, Relaxed);
}

// SAFETY: every call is forwarded to the system allocator unchanged.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        grew(l.size());
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Relaxed);
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        LIVE.fetch_sub(l.size(), Relaxed);
        grew(new);
        unsafe { System.realloc(p, l, new) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

fn raw_pdf(objects: &[String]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for at in offsets {
        out.extend_from_slice(format!("{at:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

#[test]
fn pages_sharing_one_stream_are_hashed_without_keeping_a_copy_each() {
    const PAGES: usize = 12;
    const STREAM: usize = 4 << 20;
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /Names << /EmbeddedFiles 3 0 R >> >>".to_string(),
        format!("<< /Type /Pages /Kids [{}] /Count {PAGES} >>", (0..PAGES).map(|i| format!("{} 0 R ", i + 7)).collect::<String>()),
        "<< /Names [(a) 4 0 R] >>".to_string(),
        format!("<< /Type /Filespec /UF ({EDITING_FILE}) /Desc (VectorCraft editing data; page hash 0000000000000000) /EF << /F 5 0 R >> >>"),
        "<< /Length 5 >>\nstream\nhello\nendstream".to_string(),
        format!("<< /Length {STREAM} >>\nstream\n{}\nendstream", "q ".repeat(STREAM / 2)),
    ];
    objects.extend((0..PAGES).map(|_| "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Contents 6 0 R >>".to_string()));
    let bytes = raw_pdf(&objects);
    let base = LIVE.load(Relaxed);
    PEAK.store(base, Relaxed);
    let e = editing_with(&bytes, None).unwrap();
    let extra = PEAK.load(Relaxed) - base;
    assert_eq!(e.data, b"hello");
    assert!(extra < 4 * STREAM, "peak {extra} bytes above the input: each page kept its own decoded copy");
}
