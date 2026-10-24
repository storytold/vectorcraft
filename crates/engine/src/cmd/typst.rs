//! Type › Typst…: Typst markup typeset by the `typst` program (desktop builds) and placed as one
//! group of paths that keeps its source ([`TypstSpec`]), to edit and typeset again in place.
//!
//! The program is `typst` on the `PATH`, or the one the `VECTORCRAFT_TYPST` environment variable
//! names. It is never taken from command params: the control channel and MCP reach these commands,
//! and must not choose what runs. It reads the markup from standard input and writes the first
//! page as SVG ([`vectorcraft_svg::import`] reads it). Its glyphs, which the SVG draws as `<use>`
//! references, come in as plain paths rather than as symbols. The project root (what `#image` and
//! `#read` can see) is the document's folder, or an empty folder for an unsaved document.
//!
//! Nothing runs until Preferences › Type › Enable Typst is on (`enableTypst`, off by default):
//! Typst downloads the `@preview` packages markup imports, a network fetch the app doesn't make
//! or check itself.
//!
//! The page is as big as its contents (no margins, no fill). An edit typesets the source again and
//! puts the new page's centre where the old one's is, with the object's scale and rotation
//! ([`TypstSpec::xf`]).

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::typst::{MAX_TYPST_SOURCE, TypstSpec};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, Point, Rect};

use super::*;
use crate::MAX_COORD;

const INSERT: &str = "type.typst.insert";
const EDIT: &str = "type.typst.edit";
const INFO: &str = "type.typst.info";

/// The text size the source starts with when none is given (pt), Typst's own.
pub const DEFAULT_FONT_SIZE: f64 = 11.0;
/// The environment variable naming the `typst` program.
pub const PROGRAM_VAR: &str = "VECTORCRAFT_TYPST";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "type.typst.insert",
            "Insert Typst",
            [],
            None,
            "{source: string (Typst markup; `$ … $` for math), preamble?: string (markup read first: #set, #import, #let), fontSize?: pt (default 11), at?: [x, y] (centre; default the first artboard's)} typeset with the `typst` program (desktop builds, once prefs enableTypst is on; `typst` on the PATH or $VECTORCRAFT_TYPST) and place it as one selected group of paths that keeps its source → {id, width, height}",
            has_doc,
            insert
        ),
        cmd!(
            "type.typst.edit",
            "Edit Typst",
            [],
            None,
            "{id?: the Typst object (default: the selected one), source?, preamble?, fontSize?} typeset a Typst object again (what is not given stays as it was), keeping its centre, scale and rotation → {id, width, height}",
            has_typst,
            edit
        ),
        cmd!(
            query "type.typst.info",
            "Typst Info",
            [],
            None,
            "{id?: the Typst object (default: the selected one)} → {id, source, preamble, fontSize}",
            has_typst,
            info
        ),
    ]
}

fn has_typst(s: &Session) -> std::result::Result<(), String> {
    target(s, &Value::Null, INFO).map(|_| ()).map_err(|e| e.to_string())
}

/// The Typst objects the selection is in: each selected object, or the nearest group around it
/// that is a Typst object (Type › Typst… edits when there is one, inserts when there is none).
pub fn selected(s: &Session) -> Result<Vec<NodeId>> {
    let st = s.doc()?;
    let d = &st.doc;
    let typst = |id: NodeId| d.node(id).is_some_and(|n| n.extras().typst.is_some());
    let mut found: Vec<NodeId> =
        st.selection.objects.iter().filter_map(|id| std::iter::successors(Some(*id), |c| d.parent_of(*c)).find(|c| typst(*c))).collect();
    found.sort_by_key(|id| id.0);
    found.dedup();
    Ok(found)
}

/// The Typst object `id` names, else the one selected: a selected object inside one counts.
fn target(s: &Session, p: &Value, cmd: &str) -> Result<NodeId> {
    let st = s.doc()?;
    let d = &st.doc;
    let typst = |id: NodeId| d.node(id).is_some_and(|n| n.extras().typst.is_some());
    // The object, or the nearest group around it that is a Typst object.
    let around = |id: NodeId| std::iter::successors(Some(id), |c| d.parent_of(*c)).find(|c| typst(*c));
    if let Some(v) = p.get("id").filter(|v| !v.is_null()) {
        let id = checked_id(s, v, cmd)?;
        return around(id).ok_or_else(|| bad(cmd, format!("object {} is not a Typst object", id.0)));
    }
    match selected(s)?[..] {
        [id] => Ok(id),
        [] => Err(EngineError::Other("select a Typst object".into())),
        _ => Err(EngineError::Other("select one Typst object".into())),
    }
}

/// A string param no longer than [`MAX_TYPST_SOURCE`].
fn text_param(p: &Value, key: &str, cmd: &str) -> Result<Option<String>> {
    match p.get(key).filter(|v| !v.is_null()) {
        None => Ok(None),
        Some(Value::String(t)) if t.len() <= MAX_TYPST_SOURCE => Ok(Some(t.clone())),
        Some(Value::String(_)) => Err(bad(cmd, format!("{key} is longer than {MAX_TYPST_SOURCE} bytes"))),
        Some(v) => Err(bad(cmd, format!("{key} must be a string, not {v}"))),
    }
}

/// `fontSize`, from 1 to 1296 pt.
fn size_param(p: &Value, cmd: &str) -> Result<Option<f64>> {
    match p.get("fontSize").filter(|v| !v.is_null()) {
        None => Ok(None),
        Some(v) => {
            v.as_f64().filter(|n| (1.0..=1296.0).contains(n)).map(Some).ok_or_else(|| bad(cmd, "fontSize must be a number from 1 to 1296 (pt)"))
        }
    }
}

fn insert(s: &mut Session, p: &Value) -> Result<Value> {
    let source = text_param(p, "source", INSERT)?.ok_or_else(|| bad(INSERT, "missing string `source`"))?;
    let preamble = text_param(p, "preamble", INSERT)?.unwrap_or_default();
    let font_size = size_param(p, INSERT)?.unwrap_or(DEFAULT_FONT_SIZE);
    let at = match p.get("at").filter(|v| !v.is_null()) {
        None => None,
        Some(v) => Some(finite_numbers(v).map(|[x, y]| Point::new(x, y)).ok_or_else(|| bad(INSERT, "at must be [x, y]"))?),
    };
    if at.is_some_and(|c| !(c.x.abs() <= MAX_COORD && c.y.abs() <= MAX_COORD)) {
        return Err(bad(INSERT, "at lies outside the canvas"));
    }
    enabled(s)?;
    let st = s.doc()?;
    let art = typeset(&source, &preamble, font_size, st.path.as_deref(), INSERT)?;
    let centre = at.or_else(|| st.doc.artboards.first().map(|a| a.rect.center())).unwrap_or_default();
    let m = Affine::translate(centre - art.page.center());
    let parent = st.insertion_parent();
    let spec = TypstSpec { source, preamble, font_size, page: art.page, xf: Affine::IDENTITY };
    s.edit("Insert Typst", |d, sel| {
        let node = group(d, art, spec, m);
        let size = node.geometric_bounds_with(&d.symbol_boxes()).unwrap_or_default();
        let id = d.insert(parent, usize::MAX, node)?;
        sel.set([id]);
        Ok(json!({ "id": id.0, "width": size.width(), "height": size.height() }))
    })
}

fn edit(s: &mut Session, p: &Value) -> Result<Value> {
    let id = target(s, p, EDIT)?;
    enabled(s)?;
    let st = s.doc()?;
    let old = st.doc.node(id).ok_or(EngineError::NoNode(id))?;
    let Some(was) = old.extras().typst.as_deref().cloned() else { return Err(bad(EDIT, "not a Typst object")) };
    let source = text_param(p, "source", EDIT)?.unwrap_or(was.source);
    let preamble = text_param(p, "preamble", EDIT)?.unwrap_or(was.preamble);
    // What the object keeps came from a file, unchecked: too long is refused, and a size Typst
    // can't use is its default.
    if source.len() > MAX_TYPST_SOURCE || preamble.len() > MAX_TYPST_SOURCE {
        return Err(bad(EDIT, format!("the Typst source is longer than {MAX_TYPST_SOURCE} bytes")));
    }
    let font_size = size_param(p, EDIT)?.unwrap_or(was.font_size);
    let font_size = if (1.0..=1296.0).contains(&font_size) { font_size } else { DEFAULT_FONT_SIZE };
    // A file's spec is untrusted: one that can't place the page puts it where the art was.
    let xf = Some(was.xf).filter(|a| a.as_coeffs().iter().all(|c| c.is_finite()) && a.determinant().abs() > 1e-12);
    let art = typeset(&source, &preamble, font_size, st.path.as_deref(), EDIT)?;
    let m = match xf {
        // The new page's centre where the old one's was, as the object was moved, scaled and rotated.
        Some(xf) => xf * Affine::translate(was.page.center() - art.page.center()),
        None => {
            let c = old.geometric_bounds_with(&st.doc.symbol_boxes()).map_or(art.page.center(), |b| b.center());
            Affine::translate(c - art.page.center())
        }
    };
    // The group keeps everything of its own (name, appearance, transparency, opacity mask,
    // attributes, graphic style…): only its contents and its spec are new.
    let kept = old.clone();
    let spec = TypstSpec { source, preamble, font_size, page: art.page, xf: Affine::IDENTITY };
    s.edit("Edit Typst", |d, sel| {
        let new = group(d, art, spec, m);
        let mut node = kept;
        node.id = new.id;
        node.bbox_angle = new.bbox_angle;
        node.extras_mut().typst = new.extras().typst.clone();
        node.kind = new.kind;
        let size = node.geometric_bounds_with(&d.symbol_boxes()).unwrap_or_default();
        let (par, index, _) = d.position(id).ok_or(EngineError::NoNode(id))?;
        d.remove(id)?;
        let new = d.insert(par, index, node)?;
        sel.set([new]);
        Ok(json!({ "id": new.0, "width": size.width(), "height": size.height() }))
    })
}

fn info(s: &mut Session, p: &Value) -> Result<Value> {
    let id = target(s, p, INFO)?;
    let n = s.doc()?.doc.node(id).ok_or(EngineError::NoNode(id))?;
    let t = n.extras().typst.as_deref().ok_or_else(|| bad(INFO, "not a Typst object"))?;
    Ok(json!({ "id": id.0, "source": t.source, "preamble": t.preamble, "fontSize": t.font_size }))
}

/// Typeset art: the SVG import it came from (for its resources), its objects and its page.
pub(crate) struct Typeset {
    doc: Document,
    nodes: Vec<Node>,
    page: Rect,
}

/// The art of a Typst object as a group of `d` that keeps `spec`, moved by `m`.
fn group(d: &mut Document, mut art: Typeset, spec: TypstSpec, m: Affine) -> Node {
    super::place::adopt::adopt(d, &art.doc, &mut art.nodes);
    let children = art.nodes.iter().map(|n| Arc::new(d.reid(n))).collect();
    let mut g = Node::group(d.alloc_id(), children);
    g.extras_mut().typst = Some(Box::new(spec));
    // The spec's `xf` becomes `m` with the art.
    super::place::transformed(g, m)
}

/// Refused while Preferences › Type › Enable Typst is off.
fn enabled(s: &Session) -> Result<()> {
    if s.prefs.enable_typst {
        return Ok(());
    }
    Err(EngineError::Other("Typst is off: turn on Enable Typst in Preferences › Type".into()))
}

/// `source` typeset, with `preamble` before it, from the folder of the document at `doc_path`.
pub(crate) fn typeset(source: &str, preamble: &str, font_size: f64, doc_path: Option<&str>, cmd: &str) -> Result<Typeset> {
    if source.trim().is_empty() {
        return Err(bad(cmd, "the Typst source is empty"));
    }
    let (input, header) = wrapped(source, preamble, font_size);
    let svg = run::compile(&input, doc_path).map_err(|e| EngineError::Other(diagnostics(&e, header, line_count(preamble))))?;
    art(&svg).map_err(|e| EngineError::Other(format!("Typst's SVG: {e}")))
}

/// The markup the program reads, and how many lines come before the preamble.
fn wrapped(source: &str, preamble: &str, font_size: f64) -> (String, usize) {
    let header = format!("#set page(width: auto, height: auto, margin: 0pt, fill: none)\n#set text(size: {font_size}pt)\n");
    let lines = header.lines().count();
    (format!("{header}{preamble}\n{source}\n"), lines)
}

/// Lines `text` takes in [`wrapped`] (it is followed by a line break).
fn line_count(text: &str) -> usize {
    text.split('\n').count()
}

/// The program's messages with their `<stdin>:line:column` places counted in the source (or the
/// preamble) as the user wrote it.
fn diagnostics(stderr: &str, header: usize, preamble: usize) -> String {
    let place = |line: usize| match line.checked_sub(header) {
        Some(l) if l > preamble => format!("line {}", l - preamble),
        Some(l) if l > 0 => format!("preamble line {l}"),
        _ => format!("line {line} of the page setup"),
    };
    let mut out: Vec<String> = stderr
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(20)
        .map(|l| match l.strip_prefix("<stdin>:").and_then(|r| r.split_once(':')) {
            Some((n, rest)) if let Ok(n) = n.parse::<usize>() => format!("{}:{rest}", place(n)),
            _ => l.to_string(),
        })
        .collect();
    if out.is_empty() {
        out.push("Typst failed without a message".into());
    }
    out.join("\n")
}

/// The objects of a Typst SVG page, its glyphs as plain paths, and the page.
fn art(svg: &str) -> std::result::Result<Typeset, String> {
    let mut doc = vectorcraft_svg::import(svg).map_err(|e| e.to_string())?;
    let page = doc.artboards.first().map(|a| a.rect).ok_or("the page is empty")?;
    // Each glyph is a `<use>` of a glyph outline: an instance, broken here (art inside a symbol
    // can be an instance too, so a few rounds).
    for _ in 0..4 {
        let mut ids = vec![];
        for l in &doc.layers {
            l.walk(&mut |n| {
                if matches!(n.kind, NodeKind::SymbolInstance { .. }) {
                    ids.push(n.id)
                }
            });
        }
        if ids.is_empty() {
            break;
        }
        super::brushsym::break_links(&mut doc, &ids).map_err(|e| e.to_string())?;
    }
    doc.symbols.clear();
    let layers = std::mem::take(&mut doc.layers);
    let nodes: Vec<Node> = layers.iter().flat_map(|l| l.children().map_or(&[][..], Vec::as_slice)).map(|n| collapsed((**n).clone())).collect();
    if nodes.is_empty() {
        return Err("the page is empty".into());
    }
    Ok(Typeset { doc, nodes, page })
}

/// `n` with each plain group of one object (as each glyph's `<g>` and its outline come in) replaced
/// by that object.
fn collapsed(mut n: Node) -> Node {
    if let Some(ch) = n.children_mut() {
        for c in ch.iter_mut() {
            *c = Arc::new(collapsed((**c).clone()));
        }
    }
    let plain = n.has_default_transparency() && *n.appearance == Default::default() && n.extras().typst.is_none();
    match &n.kind {
        NodeKind::Group { children, clip: false } if plain && children.len() == 1 => children.first().map_or(n.clone(), |c| (**c).clone()),
        _ => n,
    }
}

/// Running the `typst` program.
#[cfg(not(target_arch = "wasm32"))]
mod run {
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    /// How long the program may take.
    const TIMEOUT: Duration = Duration::from_secs(30);
    /// The most SVG read (bytes).
    const MAX_SVG: u64 = 64 << 20;
    /// The most messages read (bytes).
    const MAX_MESSAGES: u64 = 64 << 10;

    /// The first page of `input` as SVG, else the program's messages.
    pub(super) fn compile(input: &str, doc_path: Option<&str>) -> Result<String, String> {
        let program = std::env::var(super::PROGRAM_VAR).ok().filter(|p| !p.is_empty()).unwrap_or_else(|| "typst".into());
        // Lives to the end of the run: an empty root is removed when it drops.
        let root = root(doc_path)?;
        let mut child = Command::new(&program)
            .args(["compile", "--format", "svg", "--pages", "1", "--diagnostic-format", "short", "--root"])
            .arg(root.path())
            .args(["-", "-"])
            .current_dir(root.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => {
                    format!("Typst isn't installed: `{program}` wasn't found (install it from typst.app, or set VECTORCRAFT_TYPST to its path)")
                }
                _ => format!("couldn't run `{program}`: {e}"),
            })?;
        // Both outputs are read while it runs, so neither pipe fills and stalls it.
        let read = |pipe: Option<Box<dyn Read + Send>>, max: u64| {
            std::thread::spawn(move || {
                let mut buf = vec![];
                if let Some(mut p) = pipe {
                    // A read error leaves what came before it, and the exit status says what went wrong.
                    let _ = (&mut p).take(max).read_to_end(&mut buf);
                    // Past the cap: drained, so the program can finish.
                    let _ = std::io::copy(&mut p, &mut std::io::sink());
                }
                buf
            })
        };
        let out = read(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>), MAX_SVG + 1);
        let err = read(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>), MAX_MESSAGES);
        let deadline = Instant::now() + TIMEOUT;
        // Written while it runs too: a program that doesn't read it can't stall this past the
        // deadline (killed, it closes the pipe and the writer ends).
        if let Some(mut stdin) = child.stdin.take() {
            let input = input.as_bytes().to_vec();
            std::thread::spawn(move || {
                // A program that exits before reading it all says why on stderr.
                let _ = stdin.write_all(&input);
            });
        }
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
                Ok(None) => {
                    // It is ended either way; a failure to kill it leaves nothing else to do.
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("Typst took longer than {} seconds", TIMEOUT.as_secs()));
                }
                Err(e) => return Err(format!("couldn't wait for `{program}`: {e}")),
            }
        };
        let out = out.join().unwrap_or_default();
        let err = err.join().unwrap_or_default();
        if !status.success() {
            return Err(String::from_utf8_lossy(&err).into_owned());
        }
        if out.len() as u64 > MAX_SVG {
            return Err(format!("Typst's SVG is larger than {} MB", MAX_SVG >> 20));
        }
        String::from_utf8(out).map_err(|_| "Typst's SVG isn't UTF-8".into())
    }

    /// The project root: the document's folder, else an empty folder of our own. While automation
    /// is confined ([`crate::file_access`]), always the empty folder: Typst reads its root itself,
    /// past the checks the app's own reads go through, and follows links in it to anywhere.
    fn root(doc_path: Option<&str>) -> Result<Root, String> {
        let confined = crate::file_access::current().is_some();
        if let Some(dir) = doc_path.and_then(|p| std::path::Path::new(p).parent()).filter(|d| !confined && d.is_dir()) {
            return Ok(Root::Doc(dir.to_path_buf()));
        }
        EmptyRoot::new().map(Root::Empty)
    }

    /// Where Typst's `--root` points for one run.
    pub(super) enum Root {
        /// The document's folder.
        Doc(std::path::PathBuf),
        /// A private empty folder, removed after the run.
        Empty(EmptyRoot),
    }

    impl Root {
        pub(super) fn path(&self) -> &std::path::Path {
            match self {
                Root::Doc(p) => p,
                Root::Empty(e) => &e.0,
            }
        }
    }

    /// A private empty folder made for one Typst run and removed when dropped. Its name is random
    /// and it is created, not reused, so nobody else can have put files in it first (a guessable
    /// name made with `create_dir_all` would accept a folder another user prepared); on Unix only
    /// this user can open it.
    pub(super) struct EmptyRoot(std::path::PathBuf);

    impl EmptyRoot {
        pub(super) fn new() -> Result<Self, String> {
            use std::hash::{BuildHasher, Hasher};
            let base = std::env::temp_dir();
            let mut last = None;
            for _ in 0..16 {
                // A fresh `RandomState` is keyed from the OS, so the name can't be guessed ahead.
                let mut h = std::collections::hash_map::RandomState::new().build_hasher();
                h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
                h.write_u32(std::process::id());
                let dir = base.join(format!("vectorcraft-typst-{:016x}", h.finish()));
                let mut builder = std::fs::DirBuilder::new();
                #[cfg(unix)]
                std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
                match builder.create(&dir) {
                    Ok(()) => return Ok(Self(dir)),
                    // Taken (by an earlier run, or by someone guessing): another name.
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last = Some(e),
                    Err(e) => return Err(format!("couldn't make a folder for Typst: {e}")),
                }
            }
            Err(format!("couldn't make a folder for Typst: {}", last.map(|e| e.to_string()).unwrap_or_default()))
        }
    }

    impl Drop for EmptyRoot {
        fn drop(&mut self) {
            // Best effort: a folder left behind is empty and harmless.
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

/// The web build can't run programs.
#[cfg(target_arch = "wasm32")]
mod run {
    pub(super) fn compile(_: &str, _: Option<&str>) -> Result<String, String> {
        Err("Typst needs the desktop app: the web app can't run the typst program".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The empty root is a fresh private folder each time, and it goes away with its guard.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_empty_root_is_private_fresh_and_removed() {
        let a = run::EmptyRoot::new().unwrap();
        let b = run::EmptyRoot::new().unwrap();
        let (pa, pb) = (run::Root::Empty(a).path().to_path_buf(), run::Root::Empty(b).path().to_path_buf());
        assert_ne!(pa, pb, "two runs share a root");
        assert!(!pa.exists() && !pb.exists(), "a dropped root stays");
        let c = run::EmptyRoot::new().unwrap();
        let pc = run::Root::Empty(c);
        let dir = pc.path().to_path_buf();
        assert!(dir.is_dir());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "the root isn't empty");
        assert!(dir.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("vectorcraft-typst-") && n.len() == 34));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        }
        drop(pc);
        assert!(!dir.exists(), "the root stays after the run");
    }

    #[test]
    fn diagnostics_count_lines_in_the_source_and_preamble() {
        let (_, header) = wrapped("x", "a\nb", 11.0);
        assert_eq!(header, 2);
        let pre = line_count("a\nb");
        assert_eq!(diagnostics("<stdin>:6:4: error: expected expression", header, pre), "line 2:4: error: expected expression");
        assert_eq!(diagnostics("<stdin>:3:1: error: unknown variable: foo", header, pre), "preamble line 1:1: error: unknown variable: foo");
        assert_eq!(diagnostics("error: file not found", header, pre), "error: file not found");
        assert_eq!(diagnostics("", header, pre), "Typst failed without a message");
    }

    #[test]
    fn glyph_uses_come_in_as_paths() {
        // A page as Typst writes it: each glyph a `<use>` of an outline in `<defs>`, flipped.
        let svg = r##"<svg viewBox="0 0 20 10" width="20pt" height="10pt" xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><g transform="matrix(1 0 0 -1 0 8)"><use xlink:href="#g1" x="0" y="0" fill="#0000ff" fill-rule="nonzero"/></g><g transform="matrix(1 0 0 -1 10 8)"><use xlink:href="#g1" x="0" y="0" fill="#0000ff" fill-rule="nonzero"/></g><defs><symbol id="g1" overflow="visible"><path d="M 0 0 L 8 0 L 8 6 Z"/></symbol></defs></svg>"##;
        let t = art(svg).unwrap();
        assert!((t.page.width() - 20.0).abs() < 1e-4 && (t.page.height() - 10.0).abs() < 1e-4, "{:?}", t.page);
        assert!(t.doc.symbols.is_empty());
        assert_eq!(t.nodes.len(), 2);
        for n in &t.nodes {
            n.walk(&mut |c| assert!(!matches!(c.kind, NodeKind::SymbolInstance { .. } | NodeKind::Group { .. }), "{:?}", c.kind));
        }
    }
}
