//! The PostScript objects the interpreter works with, its operators and its errors.

use std::cell::{Ref, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use super::data::Stream;
use super::graphics::GState;

/// A dictionary (keys are names; strings and integral reals are keyed as their name or number).
pub(crate) type Dict = HashMap<Key, Obj>;

/// A shared, mutable dictionary.
pub(crate) type DictRef = Rc<RefCell<Dict>>;

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
    /// An array; executable ones are procedures.
    Array {
        items: Shared<Obj>,
        exec: bool,
    },
    Dict(DictRef),
    Op(Op),
    Mark,
    File(Rc<RefCell<Stream>>),
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
        Self::Dict(Rc::new(RefCell::new(d)))
    }

    /// The number this is (integers and reals).
    pub fn as_num(&self) -> Option<f64> {
        match *self {
            Self::Int(i) => Some(i as f64),
            Self::Real(r) => Some(r),
            _ => None,
        }
    }

    /// The name or string text (names and strings, as dictionary keys are).
    pub fn text(&self) -> Option<Rc<str>> {
        match self {
            Self::Name(n) | Self::Exec(n) => Some(n.clone()),
            Self::Str(s) => Some(Rc::from(String::from_utf8_lossy(&s.borrow()).as_ref())),
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
            Self::Str(_) => "stringtype",
            Self::Array { .. } => "arraytype",
            Self::Dict(_) => "dicttype",
            Self::Op(_) => "operatortype",
            Self::Mark => "marktype",
            Self::File(_) => "filetype",
            Self::Save(_) => "savetype",
            Self::GState(_) => "gstatetype",
        }
    }
}

/// The values of a string or an array: a run of storage that other strings or arrays may share
/// (`getinterval` makes a new object over part of the same values, as PostScript does).
#[derive(Clone, Debug)]
pub(crate) struct Shared<T> {
    buf: Rc<RefCell<Vec<T>>>,
    at: usize,
    len: usize,
}

impl<T: Clone> Shared<T> {
    pub fn new(v: Vec<T>) -> Self {
        Self { len: v.len(), buf: Rc::new(RefCell::new(v)), at: 0 }
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
    SystemDict = "systemdict", UserDict = "userdict", GlobalDict = "globaldict", StatusDict = "statusdict",
    ErrorDict = "errordict", DollarError = "$error", Bind = "bind",
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
    SetObjectFormat = "setobjectformat", ShowPage = "showpage", CopyPage = "copypage", ErasePage = "erasepage",
    CurrentScreen = "currentscreen", SetColorScreen = "setcolorscreen", CurrentHalftone = "currenthalftone",
    CurrentTransfer = "currenttransfer", CurrentColorTransfer = "currentcolortransfer",
    CurrentBlackGeneration = "currentblackgeneration", CurrentUnderColorRemoval = "currentundercolorremoval",
    SetCacheLimit = "setcachelimit", UCache = "ucache", SetUCacheParams = "setucacheparams", FindEncoding = "findencoding",
    CacheStatus = "cachestatus", UCacheStatus = "ucachestatus", CurrentCacheParams = "currentcacheparams",
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
    KShow = "kshow", GlyphShow = "glyphshow", StringWidth = "stringwidth", CharPath = "charpath",
    SetCacheDevice = "setcachedevice", SetCharWidth = "setcharwidth",
}
