//! The PostScript objects the interpreter works with, its operators and its errors.

use std::cell::{Ref, RefCell};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::{Rc, Weak};

use super::data::Stream;
use super::graphics::GState;

/// A dictionary (keys are names; strings and integral reals are keyed as their name or number).
pub(crate) type Dict = HashMap<Key, Obj>;

/// A shared, mutable dictionary.
#[derive(Clone, Debug)]
pub(crate) struct DictRef(Rc<RefCell<Dict>>);

impl DictRef {
    pub fn new(d: Dict) -> Self {
        let d = Rc::new(RefCell::new(d));
        note(Made::Dict(Rc::downgrade(&d)));
        Self(d)
    }
}

/// An array or a dictionary a program made.
enum Made {
    Array(Weak<RefCell<Vec<Obj>>>),
    Dict(Weak<RefCell<Dict>>),
}

/// The arrays and dictionaries made on this thread since [`track`], for [`release_tracked`].
struct Tracked {
    owner: u64,
    made: Vec<Made>,
    /// The list is cleared of dropped objects when it reaches this length.
    sweep_at: usize,
}

const SWEEP_MIN: usize = 4096;

#[cfg(test)]
thread_local! {
    /// Tests: the arrays of the list last released on this thread that were still alive after it
    /// (the ones the interpreter itself still held).
    pub(crate) static ARRAYS_LEFT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

thread_local! {
    static TRACKED: RefCell<Option<Tracked>> = const { RefCell::new(None) };
}

/// Start listing the arrays and dictionaries made on this thread (an interpreter starting) → the
/// list's owner, for [`release_tracked`]. A list already there is dropped: its objects are freed
/// as before this existed, unless they hold each other.
pub(crate) fn track() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let owner = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // (Nothing to list on when the thread is ending.)
    let _ = TRACKED.try_with(|t| t.try_borrow_mut().map(|mut t| *t = Some(Tracked { owner, made: vec![], sweep_at: SWEEP_MIN })));
    owner
}

fn note(m: Made) {
    let _ = TRACKED.try_with(|t| {
        let Ok(mut t) = t.try_borrow_mut() else { return };
        let Some(t) = t.as_mut() else { return };
        if t.made.len() >= t.sweep_at {
            // Twice what is alive at most, so the list costs a program no more than its objects.
            t.made.retain(|m| match m {
                Made::Array(a) => a.strong_count() > 0,
                Made::Dict(d) => d.strong_count() > 0,
            });
            t.sweep_at = (t.made.len() * 2).max(SWEEP_MIN);
        }
        t.made.push(m);
    });
}

/// Empty every array and dictionary still alive of the list `owner` started (the interpreter is
/// done with them), so those holding themselves or each other are freed too: reference counts
/// alone never free `/a 1 array def a 0 a put`, and a program can leave as much of that behind as
/// it may allocate, on every import (#1714).
pub(crate) fn release_tracked(owner: u64) {
    let made = TRACKED.try_with(|t| {
        let mut t = t.try_borrow_mut().ok()?;
        t.take_if(|t| t.owner == owner).map(|t| t.made)
    });
    let made = made.ok().flatten().unwrap_or_default();
    #[cfg(test)]
    let arrays: Vec<_> = made.iter().filter_map(|m| if let Made::Array(a) = m { Some(a.clone()) } else { None }).collect();
    for m in made {
        // What it holds is taken out first and dropped with nothing borrowed.
        match m {
            Made::Array(a) => {
                let held = a.upgrade().and_then(|a| a.try_borrow_mut().ok().map(|mut values| std::mem::take(&mut *values)));
                drop_held(held.unwrap_or_default().into_iter());
            }
            Made::Dict(d) => {
                let held = d.upgrade().and_then(|d| d.try_borrow_mut().ok().map(|mut d| std::mem::take(&mut *d)));
                drop_held(held.unwrap_or_default().into_values());
            }
        }
    }
    #[cfg(test)]
    ARRAYS_LEFT.with(|n| n.set(arrays.iter().filter(|a| a.strong_count() > 0).count()));
}

impl std::ops::Deref for DictRef {
    type Target = Rc<RefCell<Dict>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for DictRef {
    fn drop(&mut self) {
        if Rc::strong_count(&self.0) == 1
            && let Ok(mut d) = self.0.try_borrow_mut()
        {
            drop_held(d.drain().map(|(_, v)| v));
        }
    }
}

thread_local! {
    /// What the composite objects being dropped held, waiting for their turn (while any is).
    static HELD: RefCell<Option<Vec<Obj>>> = const { RefCell::new(None) };
}

/// Drop what a composite object held, now that nothing else refers to it, without recursion: a
/// program nests arrays and dictionaries as deep as it likes (`[ exch ]` in a loop), and dropping
/// them one inside the other overflows the stack. What `held` holds in its turn waits in [`HELD`]
/// for the outermost drop, which works through it.
fn drop_held(held: impl Iterator<Item = Obj>) {
    let holds = |o: &Obj| matches!(o, Obj::Array { .. } | Obj::Dict(_) | Obj::File { .. } | Obj::GState(_));
    // (Without the list, when the thread is ending: dropped as they come.)
    let outermost = HELD.try_with(|h| {
        let Ok(mut h) = h.try_borrow_mut() else { return false };
        let outermost = h.is_none();
        h.get_or_insert_with(Vec::new).extend(held.filter(holds));
        outermost
    });
    if !matches!(outermost, Ok(true)) {
        return;
    }
    // Each drop here puts what it held on the list instead of dropping it.
    while let Ok(Some(next)) = HELD.try_with(|h| h.try_borrow_mut().ok().and_then(|mut h| h.as_mut().and_then(Vec::pop))) {
        drop(next);
    }
    let _ = HELD.try_with(|h| h.try_borrow_mut().map(|mut h| *h = None)); // Nothing to do when the thread is ending.
}

/// What [`Shared`] values may be: bytes, or objects (which may hold more of themselves).
pub(crate) trait Held: Sized {
    /// Drop the values of storage nothing refers to any more.
    fn release(_values: Vec<Self>) {}
    /// Storage of these values was made.
    fn made(_storage: &Rc<RefCell<Vec<Self>>>) {}
}

impl Held for u8 {}

impl Held for Obj {
    fn release(values: Vec<Self>) {
        drop_held(values.into_iter());
    }
    fn made(storage: &Rc<RefCell<Vec<Self>>>) {
        note(Made::Array(Rc::downgrade(storage)));
    }
}

/// A dictionary key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Key {
    Name(Rc<str>),
    Int(i64),
    Bool(bool),
}

#[derive(Clone, Debug)]
pub(crate) enum Obj {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    /// A literal name (`/name`).
    Name(Rc<str>),
    /// An executable name: looked up and run.
    Exec(Rc<str>),
    Str(Shared<u8>),
    /// An executable string (`cvx`): run, it runs its text as a program.
    ExecStr(Shared<u8>),
    /// An array; executable ones are procedures.
    Array {
        items: Shared<Obj>,
        exec: bool,
    },
    Dict(DictRef),
    Op(Op),
    Mark,
    /// A file; executable ones run their data as a program (`cvx exec`).
    File {
        stream: Rc<RefCell<Stream>>,
        exec: bool,
    },
    /// A `save` level (the graphics state depth it restores to).
    Save(usize),
    /// A graphics state object (`gstate`, `currentgstate`).
    GState(Rc<GState>),
}

impl Obj {
    pub fn name(s: &str) -> Self {
        Self::Name(Rc::from(s))
    }

    pub fn string(bytes: Vec<u8>) -> Self {
        Self::Str(Shared::new(bytes))
    }

    pub fn array(items: Vec<Obj>) -> Self {
        Self::Array { items: Shared::new(items), exec: false }
    }

    pub fn proc(items: Vec<Obj>) -> Self {
        Self::Array { items: Shared::new(items), exec: true }
    }

    pub fn dict(d: Dict) -> Self {
        Self::Dict(DictRef::new(d))
    }

    /// The number this is (integers and finite reals: a NaN or infinite real is not a number).
    pub fn as_num(&self) -> Option<f64> {
        match *self {
            Self::Int(i) => Some(i as f64),
            Self::Real(r) if r.is_finite() => Some(r),
            _ => None,
        }
    }

    /// The name or string text (names and strings, as dictionary keys are).
    pub fn text(&self) -> Option<Rc<str>> {
        match self {
            Self::Name(n) | Self::Exec(n) => Some(n.clone()),
            Self::Str(s) | Self::ExecStr(s) => Some(Rc::from(String::from_utf8_lossy(&s.borrow()).as_ref())),
            _ => None,
        }
    }

    /// The key this is in a dictionary.
    pub fn key(&self) -> Option<Key> {
        match self {
            Self::Int(i) => Some(Key::Int(*i)),
            Self::Real(r) if r.fract() == 0.0 && r.abs() < 1e15 => Some(Key::Int(*r as i64)),
            Self::Bool(b) => Some(Key::Bool(*b)),
            Self::Op(op) => Some(Key::Name(Rc::from(op.name()))),
            o => o.text().map(Key::Name),
        }
    }

    /// The items of an array or procedure.
    pub fn items(&self) -> Option<&Shared<Obj>> {
        match self {
            Self::Array { items, .. } => Some(items),
            _ => None,
        }
    }

    /// The name `type` returns.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "nulltype",
            Self::Bool(_) => "booleantype",
            Self::Int(_) => "integertype",
            Self::Real(_) => "realtype",
            Self::Name(_) | Self::Exec(_) => "nametype",
            Self::Str(_) | Self::ExecStr(_) => "stringtype",
            Self::Array { .. } => "arraytype",
            Self::Dict(_) => "dicttype",
            Self::Op(_) => "operatortype",
            Self::Mark => "marktype",
            Self::File { .. } => "filetype",
            Self::Save(_) => "savetype",
            Self::GState(_) => "gstatetype",
        }
    }
}

/// The values of a string or an array: a run of storage that other strings or arrays may share
/// (`getinterval` makes a new object over part of the same values, as PostScript does).
#[derive(Clone, Debug)]
pub(crate) struct Shared<T: Held> {
    buf: Rc<RefCell<Vec<T>>>,
    at: usize,
    len: usize,
}

impl<T: Held> Drop for Shared<T> {
    fn drop(&mut self) {
        if Rc::strong_count(&self.buf) == 1
            && let Ok(mut values) = self.buf.try_borrow_mut()
        {
            T::release(std::mem::take(&mut *values));
        }
    }
}

impl<T: Clone + Held> Shared<T> {
    pub fn new(v: Vec<T>) -> Self {
        let (len, buf) = (v.len(), Rc::new(RefCell::new(v)));
        T::made(&buf);
        Self { len, buf, at: 0 }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    /// The values. Only [`Self::write`] borrows the storage mutably, for the copy alone, so this
    /// never meets it.
    pub fn borrow(&self) -> Ref<'_, [T]> {
        let (at, end) = (self.at, self.at.saturating_add(self.len));
        Ref::map(self.buf.borrow(), |v| v.get(at..end).unwrap_or(&[]))
    }

    pub fn get(&self, i: usize) -> Option<T> {
        self.borrow().get(i).cloned()
    }

    pub fn to_vec(&self) -> Vec<T> {
        self.borrow().to_vec()
    }

    /// `n` values from `at`, sharing the storage (`getinterval`); `None` past the end.
    pub fn sub(&self, at: usize, n: usize) -> Option<Self> {
        (at.checked_add(n)? <= self.len).then(|| Self { buf: self.buf.clone(), at: self.at + at, len: n })
    }

    /// Write `vals` from `at`: false when they don't fit (or `vals` is borrowed from the storage).
    pub fn write(&self, at: usize, vals: &[T]) -> bool {
        if at.saturating_add(vals.len()) > self.len {
            return false;
        }
        let start = self.at + at;
        let Ok(mut v) = self.buf.try_borrow_mut() else { return false };
        match v.get_mut(start..start + vals.len()) {
            Some(d) => {
                d.clone_from_slice(vals);
                true
            }
            None => false,
        }
    }

    /// The same object (`eq` on arrays): the same values of the same storage.
    pub fn same(&self, o: &Self) -> bool {
        Rc::ptr_eq(&self.buf, &o.buf) && self.at == o.at && self.len == o.len
    }

    /// Do both see the same storage?
    pub fn shares(&self, o: &Self) -> bool {
        Rc::ptr_eq(&self.buf, &o.buf)
    }
}

/// Hashes what [`Shared::same`] compares: the storage and the run of it.
impl<T: Held> Hash for Shared<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Rc::as_ptr(&self.buf).hash(state);
        (self.at, self.len).hash(state);
    }
}

impl Key {
    pub fn name(s: &str) -> Self {
        Self::Name(Rc::from(s))
    }

    /// The key as an object (`forall` on a dictionary).
    pub fn obj(&self) -> Obj {
        match self {
            Self::Name(n) => Obj::Name(n.clone()),
            Self::Int(i) => Obj::Int(*i),
            Self::Bool(b) => Obj::Bool(*b),
        }
    }
}

/// Why interpretation stopped.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PsError {
    /// A PostScript error (`stopped` catches it): its name and the operator or name it was raised
    /// at.
    Ps(&'static str, String),
    /// `exit` outside a loop body is propagated up to the loop.
    Exit,
    /// `stop`.
    Stop,
    /// `quit`, or the end of the page: interpretation is over, without an error.
    Quit,
    /// A limit was reached (operations, nesting, memory): the file is abandoned.
    Limit(&'static str),
}

pub(crate) type Res<T = ()> = Result<T, PsError>;

/// `v` limited to the range between `a` and `b` (in either order). Unlike `f64::clamp` this never
/// panics: when one bound is NaN the range collapses to the other bound (both NaN gives `v`), and a
/// NaN `v` gives the lower bound.
pub(crate) fn clamp_between(v: f64, a: f64, b: f64) -> f64 {
    v.max(a.min(b)).min(a.max(b))
}

/// A PostScript error raised at `at`.
pub(crate) fn ps_err<T>(name: &'static str, at: &str) -> Res<T> {
    Err(PsError::Ps(name, at.to_string()))
}

macro_rules! ops {
    ($($v:ident = $n:literal),* $(,)?) => {
        /// The operators the interpreter knows.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub(crate) enum Op { $($v),* }

        impl Op {
            /// Every operator with its name (`systemdict`).
            pub const ALL: &'static [(Op, &'static str)] = &[$((Op::$v, $n)),*];

            pub fn name(self) -> &'static str {
                match self { $(Op::$v => $n),* }
            }
        }
    };
}

ops! {
    // Stack.
    Pop = "pop", Exch = "exch", Dup = "dup", Copy = "copy", Index = "index", Roll = "roll", Clear = "clear", Count = "count",
    Mark = "mark", MarkBracket = "[", CloseArray = "]", DictBegin = "<<", DictEnd = ">>", ClearToMark = "cleartomark",
    CountToMark = "counttomark",
    // Arithmetic.
    Add = "add", Sub = "sub", Mul = "mul", Div = "div", Idiv = "idiv", Mod = "mod", Neg = "neg", Abs = "abs", Sqrt = "sqrt",
    Sin = "sin", Cos = "cos", Atan = "atan", Exp = "exp", Ln = "ln", Log = "log", Round = "round", Floor = "floor",
    Ceiling = "ceiling", Truncate = "truncate", Cvi = "cvi", Cvr = "cvr", Rand = "rand", Srand = "srand", Rrand = "rrand",
    // Relations and logic.
    Eq = "eq", Ne = "ne", Gt = "gt", Ge = "ge", Lt = "lt", Le = "le", And = "and", Or = "or", Xor = "xor", Not = "not",
    Bitshift = "bitshift", True = "true", False = "false", Null = "null",
    // Control.
    Exec = "exec", If = "if", IfElse = "ifelse", For = "for", Repeat = "repeat", Loop = "loop", Exit = "exit",
    Forall = "forall", Stopped = "stopped", Stop = "stop", Quit = "quit", CountExecStack = "countexecstack",
    // Types.
    Type = "type", Cvx = "cvx", Cvlit = "cvlit", Xcheck = "xcheck", Cvn = "cvn", Cvs = "cvs", Cvrs = "cvrs",
    ReadOnly = "readonly", ExecuteOnly = "executeonly", NoAccess = "noaccess", Rcheck = "rcheck", Wcheck = "wcheck",
    // Composites.
    NewArray = "array", PackedArray = "packedarray", SetPacking = "setpacking", CurrentPacking = "currentpacking",
    Length = "length", Get = "get", Put = "put", GetInterval = "getinterval", PutInterval = "putinterval",
    Aload = "aload", Astore = "astore", NewString = "string", Search = "search", AnchorSearch = "anchorsearch",
    // Dictionaries.
    NewDict = "dict", MaxLength = "maxlength", Begin = "begin", End = "end", Def = "def", Load = "load", Store = "store",
    Known = "known", Where = "where", Undef = "undef", CurrentDict = "currentdict", CountDictStack = "countdictstack",
    DictStack = "dictstack", SystemDict = "systemdict", UserDict = "userdict", GlobalDict = "globaldict", StatusDict = "statusdict",
    ErrorDict = "errordict", DollarError = "$error", InternalDict = "internaldict", Bind = "bind",
    // Virtual memory and the environment.
    Save = "save", Restore = "restore", SetGlobal = "setglobal", CurrentGlobal = "currentglobal", VmStatus = "vmstatus",
    Version = "version", Product = "product", RealTime = "realtime", UserTime = "usertime",
    Print = "print", EqPrint = "=", EqEqPrint = "==", Pstack = "pstack", Stack = "stack", Flush = "flush",
    // Graphics state.
    Gsave = "gsave", Grestore = "grestore", GrestoreAll = "grestoreall", InitGraphics = "initgraphics",
    GStateNew = "gstate", CurrentGState = "currentgstate", SetGState = "setgstate",
    SetLineWidth = "setlinewidth", CurrentLineWidth = "currentlinewidth", SetLineCap = "setlinecap",
    CurrentLineCap = "currentlinecap", SetLineJoin = "setlinejoin", CurrentLineJoin = "currentlinejoin",
    SetMiterLimit = "setmiterlimit", CurrentMiterLimit = "currentmiterlimit", SetDash = "setdash", CurrentDash = "currentdash",
    SetGray = "setgray", CurrentGray = "currentgray", SetRgbColor = "setrgbcolor", CurrentRgbColor = "currentrgbcolor",
    SetHsbColor = "sethsbcolor", SetCmykColor = "setcmykcolor", CurrentCmykColor = "currentcmykcolor",
    SetColorSpace = "setcolorspace", CurrentColorSpace = "currentcolorspace", SetColor = "setcolor", CurrentColor = "currentcolor",
    SetPattern = "setpattern", MakePattern = "makepattern", SetOverprint = "setoverprint", CurrentOverprint = "currentoverprint",
    // Device settings without an effect on the art.
    SetFlat = "setflat", CurrentFlat = "currentflat", SetScreen = "setscreen", SetHalftone = "sethalftone",
    SetTransfer = "settransfer", SetColorTransfer = "setcolortransfer", SetBlackGeneration = "setblackgeneration",
    SetUnderColorRemoval = "setundercolorremoval", SetColorRendering = "setcolorrendering", SetSmoothness = "setsmoothness",
    SetStrokeAdjust = "setstrokeadjust", CurrentStrokeAdjust = "currentstrokeadjust", SetPageDevice = "setpagedevice",
    CurrentPageDevice = "currentpagedevice", SetUserParams = "setuserparams", SetSystemParams = "setsystemparams",
    CurrentUserParams = "currentuserparams", CurrentSystemParams = "currentsystemparams",
    SetObjectFormat = "setobjectformat", ShowPage = "showpage", CopyPage = "copypage", ErasePage = "erasepage",
    CurrentScreen = "currentscreen", SetColorScreen = "setcolorscreen", CurrentHalftone = "currenthalftone",
    CurrentTransfer = "currenttransfer", CurrentColorTransfer = "currentcolortransfer",
    CurrentBlackGeneration = "currentblackgeneration", CurrentUnderColorRemoval = "currentundercolorremoval",
    SetCacheLimit = "setcachelimit", UCache = "ucache", SetUCacheParams = "setucacheparams", FindEncoding = "findencoding",
    // Coordinates.
    Matrix = "matrix", IdentMatrix = "identmatrix", DefaultMatrix = "defaultmatrix", CurrentMatrix = "currentmatrix",
    SetMatrix = "setmatrix", InitMatrix = "initmatrix", Concat = "concat", ConcatMatrix = "concatmatrix",
    Translate = "translate", Scale = "scale", Rotate = "rotate", Transform = "transform", ITransform = "itransform",
    DTransform = "dtransform", IDTransform = "idtransform", InvertMatrix = "invertmatrix",
    // Paths and painting.
    NewPath = "newpath", CurrentPoint = "currentpoint", MoveTo = "moveto", RMoveTo = "rmoveto", LineTo = "lineto",
    RLineTo = "rlineto", CurveTo = "curveto", RCurveTo = "rcurveto", Arc = "arc", Arcn = "arcn", Arct = "arct",
    Arcto = "arcto", ClosePath = "closepath", FlattenPath = "flattenpath", ReversePath = "reversepath",
    PathBBox = "pathbbox", ClipPath = "clippath", InitClip = "initclip", Clip = "clip", EoClip = "eoclip",
    RectClip = "rectclip", ClipSave = "clipsave", ClipRestore = "cliprestore", Fill = "fill", EoFill = "eofill",
    Stroke = "stroke", RectFill = "rectfill", RectStroke = "rectstroke", ShFill = "shfill",
    // Images.
    Image = "image", ImageMask = "imagemask", ColorImage = "colorimage",
    // Files.
    CurrentFile = "currentfile", Filter = "filter", ReadHexString = "readhexstring", ReadString = "readstring",
    ReadLine = "readline", FlushFile = "flushfile", CloseFile = "closefile", File = "file", Eexec = "eexec",
    // Fonts and type.
    FindFont = "findfont", ScaleFont = "scalefont", MakeFont = "makefont", SetFont = "setfont", SelectFont = "selectfont",
    CurrentFont = "currentfont", DefineFont = "definefont", UndefineFont = "undefinefont", FindResource = "findresource",
    DefineResource = "defineresource", UndefineResource = "undefineresource", ResourceStatus = "resourcestatus",
    ResourceForAll = "resourceforall", Show = "show", AShow = "ashow",
    WidthShow = "widthshow", AWidthShow = "awidthshow", XShow = "xshow", YShow = "yshow", XYShow = "xyshow",
    KShow = "kshow", CShow = "cshow", GlyphShow = "glyphshow", StringWidth = "stringwidth", CharPath = "charpath",
    SetCacheDevice = "setcachedevice", SetCharWidth = "setcharwidth", SetCacheDevice2 = "setcachedevice2", RootFont = "rootfont",
    // Forms, devices and files programs probe (no output device or file system here).
    ExecForm = "execform", NullDevice = "nulldevice", PdfMark = "pdfmark", Status = "status", Token = "token",
    BytesAvailable = "bytesavailable", ResetFile = "resetfile", Write = "write", WriteString = "writestring",
    WriteHexString = "writehexstring", Echo = "echo", StrokePath = "strokepath", PathForAll = "pathforall",
    // Queries and settings without an effect on the art.
    CurrentHsbColor = "currenthsbcolor", CurrentColorRendering = "currentcolorrendering",
    FindColorRendering = "findcolorrendering", CurrentColorScreen = "currentcolorscreen", CurrentSmoothness = "currentsmoothness",
    SetHalftonePhase = "sethalftonephase", CurrentHalftonePhase = "currenthalftonephase", SetDevParams = "setdevparams",
    CurrentDevParams = "currentdevparams", Gcheck = "gcheck", Scheck = "scheck", CurrentShared = "currentshared",
    SetShared = "setshared", ClearDictStack = "cleardictstack", ExecStack = "execstack", SetCacheParams = "setcacheparams",
    CurrentCacheParams = "currentcacheparams", CacheStatus = "cachestatus", UCacheStatus = "ucachestatus",
    SetVmThreshold = "setvmthreshold", VmReclaim = "vmreclaim", StartJob = "startjob", CurrentObjectFormat = "currentobjectformat",
    Revision = "revision", SerialNumber = "serialnumber", Read = "read", FilePosition = "fileposition",
    SetFilePosition = "setfileposition", DeleteFile = "deletefile", RenameFile = "renamefile", Run = "run",
    // User paths.
    UFill = "ufill", UEoFill = "ueofill", UStroke = "ustroke", UAppend = "uappend", UPath = "upath", SetBBox = "setbbox",
}
