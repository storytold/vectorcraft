//! The interpreter: the operand and dictionary stacks, running objects and procedures, and the
//! operators that don't draw (stack, arithmetic, control, composites, dictionaries).
//!
//! Every operation counts against a budget and procedures nest only so deep, so a looping or
//! recursing program ends with [`PsError::Limit`] instead of hanging.

use std::cell::RefCell;
use std::rc::Rc;

use vectorcraft_geom::Affine;

use super::graphics::{GState, Out};
use super::lex::{IMMEDIATE, Lexer};
use super::obj::{Dict, DictRef, Key, Obj, Op, PsError, Res, Shared, ps_err};

/// Most operations one file may run.
const MAX_OPS: u64 = 50_000_000;
/// Deepest procedure nesting.
const MAX_DEPTH: u32 = 400;
/// Most operands on the stack.
const MAX_STACK: usize = 100_000;
/// Largest array, string or dictionary a program may make.
pub(crate) const MAX_ALLOC: usize = 1 << 22;
/// Most dictionaries on the dictionary stack.
const MAX_DICTS: usize = 1000;
/// Most saved graphics states.
pub(crate) const MAX_GSAVE: usize = 1000;
/// Most memory (bytes) a program may ask for in all: strings, arrays, decoded data, images,
/// saved paths.
const MAX_MEMORY: usize = 1 << 30;

pub(crate) struct Interp<'a> {
    pub lex: Lexer<'a>,
    pub stack: Vec<Obj>,
    pub dicts: Vec<DictRef>,
    system: DictRef,
    user: DictRef,
    global: DictRef,
    status: DictRef,
    error: DictRef,
    /// `internaldict`'s dictionary, for font programs.
    internal: DictRef,
    /// `FontDirectory`: fonts defined with `definefont`.
    pub fonts: DictRef,
    /// Resources by category (`defineresource`).
    resources: DictRef,
    depth: u32,
    ops: u64,
    /// Bytes asked for so far (see [`MAX_MEMORY`]).
    allocated: usize,
    seed: u32,
    /// The `save`s not restored yet, oldest first.
    saves: Vec<SaveMark>,
    /// What dictionary entries were before they changed since the oldest `save` (dictionary,
    /// key, old value): `restore` puts them back, as restoring local VM does (PLRM 3rd ed.,
    /// §3.7.3 "Save and Restore": composite objects get the values they had at the `save`).
    journal: Vec<(DictRef, Key, Option<Obj>)>,
    /// Where the error that is unwinding was raised (see [`Fault`]).
    pub fault: Option<Fault>,
    /// The width a Type 3 glyph procedure gave (`setcachedevice`, `setcharwidth`).
    pub glyph_width: Option<[f64; 2]>,
    pub g: GState,
    pub saved: Vec<GState>,
    pub out: Out,
    /// The program is in the legacy Illustrator format: its group operators `u` … `U` (at the top
    /// level, whatever its prolog defines them as) are groups (see the module docs of `import`).
    pub illustrator: bool,
}

/// Where a PostScript error was raised: the operator that raised it (none for an unknown name)
/// and the named procedures it ran in, innermost first, so the user and we can see the cause.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Fault {
    pub op: Option<&'static str>,
    pub procs: Vec<Rc<str>>,
}

/// Most procedure names a [`Fault`] keeps.
const FAULT_PROCS: usize = 3;

/// Where a `save` was made: the graphics states saved and the journal's length then.
#[derive(Clone, Copy, Debug)]
struct SaveMark {
    gstates: usize,
    journal: usize,
}

fn new_dict() -> DictRef {
    Rc::new(RefCell::new(Dict::new()))
}

/// An encoding vector: 256 `/.notdef` names (type is read as Latin-1 whatever the encoding).
fn encoding() -> Obj {
    Obj::array(vec![Obj::name(".notdef"); 256])
}

/// The resource categories there are to begin with (instances of `Category`).
const CATEGORIES: &str = "Category Generic Font CIDFont CMap FontSet Encoding Form Pattern ProcSet ColorSpace Halftone \r
                          ColorRendering IdiomSet InkParams TrapParams OutputDevice ControlLanguage Localization PDL HWOptions \r
                          Filter ColorSpaceFamily Emulator IODevice ColorRenderingType FMapType FontType FormType HalftoneType \r
                          ImageType PatternType FunctionType ShadingType";

impl<'a> Interp<'a> {
    pub fn new(src: &'a [u8], g: GState, out: Out) -> Self {
        let system = new_dict();
        {
            let mut d = system.borrow_mut();
            for (op, name) in Op::ALL {
                d.insert(Key::name(name), Obj::Op(*op));
            }
            for name in ["StandardEncoding", "ISOLatin1Encoding"] {
                d.insert(Key::name(name), encoding());
            }
            // An integer, as interpreters have it (`systemdict /languagelevel get 2 ge`): run, the
            // name pushes it all the same.
            d.insert(Key::name("languagelevel"), Obj::Int(3));
        }
        // Each category's implementation dictionary (`/Generic /Category findresource`).
        let categories: Dict =
            CATEGORIES.split_whitespace().map(|c| (Key::name(c), Obj::dict(Dict::from([(Key::name("Category"), Obj::name(c))])))).collect();
        let resources = new_dict();
        resources.borrow_mut().insert(Key::name("Category"), Obj::dict(categories));
        let fonts = new_dict();
        system.borrow_mut().insert(Key::name("FontDirectory"), Obj::Dict(fonts.clone()));
        for name in ["GlobalFontDirectory", "SharedFontDirectory"] {
            system.borrow_mut().insert(Key::name(name), Obj::Dict(fonts.clone()));
        }
        let (user, global, status, error) = (new_dict(), new_dict(), new_dict(), new_dict());
        // Errors don't reach the program's handler, but prologs wrap the one they find.
        error.borrow_mut().insert(Key::name("handleerror"), Obj::proc(vec![]));
        // The standard dictionaries are values in systemdict, so `get`, `load` and `where` find
        // them as dictionaries (`/globaldict where { /globaldict get begin } if`).
        for (name, d) in [
            ("systemdict", &system),
            ("userdict", &user),
            ("globaldict", &global),
            ("statusdict", &status),
            ("errordict", &error),
            ("$error", &error),
        ] {
            system.borrow_mut().insert(Key::name(name), Obj::Dict(d.clone()));
        }
        Self {
            lex: Lexer::new(src),
            stack: vec![],
            dicts: vec![system.clone(), global.clone(), user.clone()],
            system,
            user,
            global,
            status,
            error,
            internal: new_dict(),
            fonts,
            resources,
            depth: 0,
            ops: 0,
            allocated: 0,
            seed: 1,
            saves: vec![],
            journal: vec![],
            fault: None,
            glyph_width: None,
            g,
            saved: vec![],
            out,
            illustrator: false,
        }
    }

    /// Run the program to its end, its first `showpage` or `quit`.
    pub fn run(&mut self) -> Res {
        loop {
            let Some(o) = self.lex.next()? else { return Ok(()) };
            let o = self.scanned(o, self.lex.immediate)?;
            // A group begins before its `u` runs (`Some(true)`) and ends after its `U`
            // has (`Some(false)`).
            let group = match &o {
                Obj::Exec(name) if self.illustrator => match &**name {
                    "u" => Some(true),
                    "U" => Some(false),
                    _ => None,
                },
                _ => None,
            };
            if group == Some(true) {
                self.out.begin_group();
            }
            match self.exec_token(o) {
                Err(PsError::Quit) => return Ok(()),
                r => r?,
            }
            if group == Some(false) {
                self.out.end_group();
            }
        }
    }

    /// Empty the standard dictionaries, so the memory they hold is freed with the interpreter:
    /// systemdict holds itself, and programs link dictionaries into each other.
    pub fn release(&mut self) {
        for d in [&self.system, &self.user, &self.global, &self.status, &self.error, &self.internal, &self.fonts, &self.resources] {
            if let Ok(mut d) = d.try_borrow_mut() {
                d.clear();
            }
        }
        self.dicts.clear();
        self.stack.clear();
        self.journal.clear();
    }

    // ---------- the operand stack ----------

    pub fn push(&mut self, o: Obj) -> Res {
        if self.stack.len() >= MAX_STACK {
            return Err(PsError::Limit("the operand stack overflowed"));
        }
        self.stack.push(o);
        Ok(())
    }

    pub fn push_num(&mut self, v: f64) -> Res {
        self.push(if v.is_finite() { Obj::Real(v) } else { Obj::Real(0.0) })
    }

    pub fn pop(&mut self) -> Res<Obj> {
        self.stack.pop().ok_or(PsError::Ps("stackunderflow", String::new()))
    }

    pub fn pop_num(&mut self) -> Res<f64> {
        match self.pop()?.as_num() {
            Some(v) if v.is_finite() => Ok(v),
            Some(_) => ps_err("undefinedresult", ""),
            None => ps_err("typecheck", "a number"),
        }
    }

    /// `N` numbers, in the order they were pushed.
    pub fn nums<const N: usize>(&mut self) -> Res<[f64; N]> {
        let mut out = [0.0; N];
        for slot in out.iter_mut().rev() {
            *slot = self.pop_num()?;
        }
        Ok(out)
    }

    pub fn pop_int(&mut self) -> Res<i64> {
        match self.pop()? {
            Obj::Int(i) => Ok(i),
            Obj::Real(r) if r.fract() == 0.0 && r.abs() < 1e15 => Ok(r as i64),
            _ => ps_err("typecheck", "an integer"),
        }
    }

    /// A count or index: a non-negative integer, at most [`MAX_ALLOC`].
    pub fn pop_count(&mut self) -> Res<usize> {
        let i = self.pop_int()?;
        if !(0..=MAX_ALLOC as i64).contains(&i) {
            return ps_err("rangecheck", "a count");
        }
        Ok(i as usize)
    }

    pub fn pop_bool(&mut self) -> Res<bool> {
        match self.pop()? {
            Obj::Bool(b) => Ok(b),
            _ => ps_err("typecheck", "a boolean"),
        }
    }

    pub fn pop_dict(&mut self) -> Res<DictRef> {
        match self.pop()? {
            Obj::Dict(d) => Ok(d),
            _ => ps_err("typecheck", "a dictionary"),
        }
    }

    pub fn pop_str(&mut self) -> Res<Shared<u8>> {
        match self.pop()? {
            Obj::Str(s) | Obj::ExecStr(s) => Ok(s),
            _ => ps_err("typecheck", "a string"),
        }
    }

    /// An array or procedure.
    pub fn pop_array(&mut self) -> Res<Shared<Obj>> {
        match self.pop()? {
            Obj::Array { items, .. } => Ok(items),
            _ => ps_err("typecheck", "an array"),
        }
    }

    /// A matrix operand (an array of six numbers).
    pub fn pop_matrix(&mut self) -> Res<Affine> {
        let items = self.pop_array()?;
        matrix_of(&items.borrow()).ok_or(PsError::Ps("typecheck", "a matrix".into()))
    }

    /// Store `m` into the matrix operand `items` and push it back.
    pub fn put_matrix(&mut self, items: Shared<Obj>, m: Affine) -> Res {
        if items.len() != 6 || !items.write(0, &m.as_coeffs().map(Obj::Real)) {
            return ps_err("rangecheck", "a matrix");
        }
        self.push(Obj::Array { items, exec: false })
    }

    // ---------- names and running ----------

    /// The value of name `name` on the dictionary stack.
    pub fn lookup(&self, name: &Rc<str>) -> Option<Obj> {
        self.lookup_key(&Key::Name(name.clone()))
    }

    /// The value of any key (`load` takes numbers too) on the dictionary stack.
    fn lookup_key(&self, key: &Key) -> Option<Obj> {
        self.dicts.iter().rev().find_map(|d| d.borrow().get(key).cloned())
    }

    /// Count `bytes` the program makes against [`MAX_MEMORY`].
    pub fn alloc(&mut self, bytes: usize) -> Res {
        self.allocated = self.allocated.saturating_add(bytes);
        if self.allocated > MAX_MEMORY {
            return Err(PsError::Limit("the program uses too much memory"));
        }
        Ok(())
    }

    fn tick(&mut self) -> Res {
        self.spend(1)
    }

    /// Count `ops` operations' worth of work against [`MAX_OPS`]: what an operator that costs
    /// far more than one operation (laying out type) does in a loop has to end too.
    pub fn spend(&mut self, ops: u64) -> Res {
        self.ops = self.ops.saturating_add(ops);
        if self.ops > MAX_OPS {
            return Err(PsError::Limit("the program runs too long"));
        }
        Ok(())
    }

    /// An object as read or met in a procedure body: executable names and operators run, the
    /// rest (procedures too) is pushed.
    fn exec_token(&mut self, o: Obj) -> Res {
        self.tick()?;
        match o {
            Obj::Exec(name) => {
                let Some(v) = self.lookup(&name) else {
                    self.fault.get_or_insert_default();
                    return ps_err("undefined", &name);
                };
                let proc = matches!(v, Obj::Array { exec: true, .. });
                let r = self.call(v);
                if proc
                    && matches!(r, Err(PsError::Ps(..)))
                    && let Some(f) = self.fault.as_mut()
                    && f.procs.len() < FAULT_PROCS
                {
                    f.procs.push(name);
                }
                r
            }
            Obj::Op(op) => self.op(op),
            // An executable string runs, as an operator does (procedures are pushed).
            s @ Obj::ExecStr(_) => self.call(s),
            o => self.push(o),
        }
    }

    /// Run a value (what a name stands for, or `exec`'s operand): procedures and operators run,
    /// executable names are looked up, the rest is pushed.
    pub fn call(&mut self, v: Obj) -> Res {
        match v {
            Obj::Array { items, exec: true } => self.run_proc(&items),
            Obj::File { stream, exec: true } => match self.program_of(&stream)? {
                Some(program) => self.run_program(&program),
                None => Ok(()),
            },
            Obj::ExecStr(s) => self.run_program(&s.to_vec()),
            Obj::Op(op) => self.op(op),
            Obj::Exec(name) => {
                self.enter()?;
                let r = match self.lookup(&name) {
                    Some(v) => self.call(v),
                    None => ps_err("undefined", &name),
                };
                self.depth -= 1;
                r
            }
            o => self.push(o),
        }
    }

    fn enter(&mut self) -> Res {
        if self.depth >= MAX_DEPTH {
            return Err(PsError::Limit("procedures nest too deeply"));
        }
        self.depth += 1;
        Ok(())
    }

    fn run_proc(&mut self, items: &Shared<Obj>) -> Res {
        self.enter()?;
        let r = self.run_items(items);
        self.depth -= 1;
        r
    }

    /// Run `program` (what is left of an executable file, an executable string), token by token.
    fn run_program(&mut self, program: &[u8]) -> Res {
        self.enter()?;
        let mut lex = Lexer::new(program);
        let r = loop {
            match lex.next() {
                Ok(Some(o)) => {
                    if let Err(e) = self.scanned(o, lex.immediate).and_then(|o| self.exec_token(o)) {
                        break Err(e);
                    }
                }
                Ok(None) => break Ok(()),
                Err(e) => break Err(e),
            }
        };
        self.depth -= 1;
        r
    }

    /// An object as the scanner gives it: with `immediate`, its `//name`s replaced by their
    /// values (down through procedures), as they are when read.
    pub fn scanned(&self, o: Obj, immediate: bool) -> Res<Obj> {
        if immediate { self.resolve(o, 0) } else { Ok(o) }
    }

    fn resolve(&self, o: Obj, depth: usize) -> Res<Obj> {
        match o {
            Obj::Exec(n) if n.starts_with(IMMEDIATE) => {
                let name = n.get(IMMEDIATE.len()..).unwrap_or_default();
                self.lookup(&Rc::from(name)).ok_or_else(|| PsError::Ps("undefined", name.to_string()))
            }
            Obj::Array { items, exec: true } if depth < MAX_DEPTH as usize => {
                for i in 0..items.len() {
                    let Some(item) = items.get(i) else { break };
                    if matches!(&item, Obj::Exec(n) if n.starts_with(IMMEDIATE)) || matches!(&item, Obj::Array { exec: true, .. }) {
                        // A procedure the scanner just made: nothing else reads it.
                        items.write(i, &[self.resolve(item, depth + 1)?]);
                    }
                }
                Ok(Obj::Array { items, exec: true })
            }
            o => Ok(o),
        }
    }

    fn run_items(&mut self, items: &Shared<Obj>) -> Res {
        let mut i = 0;
        loop {
            let Some(o) = items.get(i) else { return Ok(()) };
            i += 1;
            self.exec_token(o)?;
        }
    }

    /// Run `proc` as a loop body: `Ok(false)` when it ran `exit`.
    pub fn body(&mut self, proc: &Obj) -> Res<bool> {
        self.tick()?;
        match self.call(proc.clone()) {
            Ok(()) => Ok(true),
            Err(PsError::Exit) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Run a procedure operand, executable or not (`exec`, `if`, loops).
    pub fn pop_proc(&mut self) -> Res<Obj> {
        match self.pop()? {
            Obj::Array { items, .. } => Ok(Obj::Array { items, exec: true }),
            o @ (Obj::Op(_) | Obj::Exec(_)) => Ok(o),
            _ => ps_err("typecheck", "a procedure"),
        }
    }

    // ---------- operators ----------

    pub fn op(&mut self, op: Op) -> Res {
        // Recursive operators use small helpers. Data dispatch returns before graphics run,
        // so nested execution does not retain the large frame of unrelated data operators.
        let result = match op {
            Op::Exec => self.exec_op(),
            Op::If => self.if_op(),
            Op::IfElse => self.if_else_op(),
            Op::For => self.for_op(),
            Op::Repeat => self.repeat_op(),
            Op::Loop => self.loop_op(),
            Op::Forall => self.forall(),
            Op::Stopped => self.stopped_op(),
            Op::Exit => Err(PsError::Exit),
            Op::Stop => Err(PsError::Stop),
            Op::Quit => Err(PsError::Quit),
            _ => self.op_inner(op).and_then(|graphics| if graphics { self.graphics_op(op) } else { Ok(()) }),
        };
        result.map_err(|e| match e {
            PsError::Ps(name, at) => {
                // The innermost operator raised it (`exec` running a procedure doesn't).
                self.fault.get_or_insert(Fault { op: Some(op.name()), procs: vec![] });
                PsError::Ps(name, if at.is_empty() { op.name().to_string() } else { at })
            }
            e => e,
        })
    }

    fn exec_op(&mut self) -> Res {
        let o = self.pop()?;
        self.call(o)?;
        Ok(())
    }

    fn if_op(&mut self) -> Res {
        let p = self.pop_proc()?;
        if self.pop_bool()? {
            self.call(p)?;
        }
        Ok(())
    }

    fn if_else_op(&mut self) -> Res {
        let (no, yes) = (self.pop_proc()?, self.pop_proc()?);
        let pick = if self.pop_bool()? { yes } else { no };
        self.call(pick)?;
        Ok(())
    }

    fn for_op(&mut self) -> Res {
        let p = self.pop_proc()?;
        let (limit, inc, init) = (self.pop()?, self.pop()?, self.pop()?);
        match (&init, &inc, &limit) {
            (Obj::Int(i0), Obj::Int(d), Obj::Int(l)) => {
                let (mut i, d, l) = (*i0, *d, *l);
                while (d >= 0 && i <= l) || (d < 0 && i >= l) {
                    self.push(Obj::Int(i))?;
                    if !self.body(&p)? {
                        break;
                    }
                    i = i.checked_add(d).ok_or(PsError::Ps("rangecheck", String::new()))?;
                    if d == 0 {
                        self.tick()?;
                    }
                }
            }
            _ => {
                let n = |o: &Obj| o.as_num().filter(|v| v.is_finite()).ok_or(PsError::Ps("typecheck", String::new()));
                let (i0, d, l) = (n(&init)?, n(&inc)?, n(&limit)?);
                let mut k = 0.0;
                loop {
                    let i = i0 + d * k;
                    if !((d >= 0.0 && i <= l) || (d < 0.0 && i >= l)) {
                        break;
                    }
                    self.push(Obj::Real(i))?;
                    if !self.body(&p)? {
                        break;
                    }
                    k += 1.0;
                }
            }
        }
        Ok(())
    }

    fn repeat_op(&mut self) -> Res {
        let p = self.pop_proc()?;
        let n = self.pop_int()?;
        if n < 0 {
            return ps_err("rangecheck", "");
        }
        for _ in 0..n {
            if !self.body(&p)? {
                break;
            }
        }
        Ok(())
    }

    fn loop_op(&mut self) -> Res {
        let p = self.pop_proc()?;
        while self.body(&p)? {}
        Ok(())
    }

    fn stopped_op(&mut self) -> Res {
        let p = self.pop()?;
        let depth = self.depth;
        match self.call(p) {
            Ok(()) => self.push(Obj::Bool(false))?,
            Err(PsError::Ps(..) | PsError::Stop | PsError::Exit) => {
                self.depth = depth;
                self.fault = None;
                self.push(Obj::Bool(true))?;
            }
            Err(e) => return Err(e),
        }
        Ok(())
    }

    /// Data operators never call the graphics handler from this large frame.
    fn op_inner(&mut self, op: Op) -> Res<bool> {
        use Op::*;
        match op {
            Pop => {
                self.pop()?;
            }
            Exch => {
                let (b, a) = (self.pop()?, self.pop()?);
                self.push(b)?;
                self.push(a)?;
            }
            Dup => {
                let a = self.stack.last().cloned().ok_or(PsError::Ps("stackunderflow", String::new()))?;
                self.push(a)?;
            }
            Copy => self.copy()?,
            Index => {
                let n = self.pop_count()?;
                let at = self.stack.len().checked_sub(n + 1).ok_or(PsError::Ps("rangecheck", String::new()))?;
                let o = self.stack.get(at).cloned().ok_or(PsError::Ps("rangecheck", String::new()))?;
                self.push(o)?;
            }
            Roll => {
                let j = self.pop_int()?;
                let n = self.pop_count()?;
                let at = self.stack.len().checked_sub(n).ok_or(PsError::Ps("rangecheck", String::new()))?;
                if n > 0 {
                    let k = j.rem_euclid(n as i64) as usize;
                    if let Some(s) = self.stack.get_mut(at..) {
                        s.rotate_right(k);
                    }
                }
            }
            Clear => self.stack.clear(),
            Count => self.push(Obj::Int(self.stack.len() as i64))?,
            Mark | MarkBracket | DictBegin => self.push(Obj::Mark)?,
            CloseArray => {
                let items = self.pop_to_mark()?;
                self.push(Obj::array(items))?;
            }
            DictEnd => {
                let items = self.pop_to_mark()?;
                let mut d = Dict::new();
                for pair in items.chunks(2) {
                    let [k, v] = pair else { return ps_err("rangecheck", ">>") };
                    d.insert(k.key().ok_or(PsError::Ps("typecheck", ">>".into()))?, v.clone());
                }
                self.push(Obj::dict(d))?;
            }
            ClearToMark => {
                self.pop_to_mark()?;
            }
            CountToMark => {
                let n = self.stack.iter().rev().position(|o| matches!(o, Obj::Mark)).ok_or(PsError::Ps("unmatchedmark", String::new()))?;
                self.push(Obj::Int(n as i64))?;
            }
            Add | Sub | Mul => {
                let (b, a) = (self.pop()?, self.pop()?);
                let r = match (&a, &b) {
                    (Obj::Int(x), Obj::Int(y)) => match op {
                        Add => x.checked_add(*y).map(Obj::Int),
                        Sub => x.checked_sub(*y).map(Obj::Int),
                        _ => x.checked_mul(*y).map(Obj::Int),
                    },
                    _ => None,
                };
                let r = match r {
                    Some(r) => r,
                    None => {
                        let (x, y) =
                            (a.as_num().ok_or(PsError::Ps("typecheck", String::new()))?, b.as_num().ok_or(PsError::Ps("typecheck", String::new()))?);
                        Obj::Real(match op {
                            Add => x + y,
                            Sub => x - y,
                            _ => x * y,
                        })
                    }
                };
                self.push(r)?;
            }
            Div => {
                let [a, b] = self.nums()?;
                if b == 0.0 {
                    return ps_err("undefinedresult", "");
                }
                self.push_num(a / b)?;
            }
            Idiv | Mod => {
                let (b, a) = (self.pop_int()?, self.pop_int()?);
                let r = if op == Idiv { a.checked_div(b) } else { a.checked_rem(b) };
                self.push(Obj::Int(r.ok_or(PsError::Ps("undefinedresult", String::new()))?))?;
            }
            Neg | Abs => match self.pop()? {
                Obj::Int(i) => self.push(Obj::Int(if op == Neg { i.saturating_neg() } else { i.saturating_abs() }))?,
                Obj::Real(r) => self.push(Obj::Real(if op == Neg { -r } else { r.abs() }))?,
                _ => return ps_err("typecheck", ""),
            },
            Sqrt | Sin | Cos | Exp | Ln | Log | Atan => {
                let r = match op {
                    Sqrt => self.pop_num()?.sqrt(),
                    Sin => self.pop_num()?.to_radians().sin(),
                    Cos => self.pop_num()?.to_radians().cos(),
                    Ln => self.pop_num()?.ln(),
                    Log => self.pop_num()?.log10(),
                    Exp => {
                        let [b, e] = self.nums()?;
                        b.powf(e)
                    }
                    _ => {
                        let [num, den] = self.nums()?;
                        num.atan2(den).to_degrees().rem_euclid(360.0)
                    }
                };
                if !r.is_finite() {
                    return ps_err("undefinedresult", "");
                }
                self.push(Obj::Real(r))?;
            }
            Round | Floor | Ceiling | Truncate => match self.pop()? {
                Obj::Int(i) => self.push(Obj::Int(i))?,
                Obj::Real(r) => self.push(Obj::Real(match op {
                    Round => (r + 0.5).floor(),
                    Floor => r.floor(),
                    Ceiling => r.ceil(),
                    _ => r.trunc(),
                }))?,
                _ => return ps_err("typecheck", ""),
            },
            Cvi => {
                let v = match self.pop()? {
                    Obj::Str(s) => String::from_utf8_lossy(&s.borrow()).trim().parse::<f64>().map_err(|_| PsError::Ps("typecheck", String::new()))?,
                    o => o.as_num().ok_or(PsError::Ps("typecheck", String::new()))?,
                };
                if !(v.is_finite() && v.abs() < 9e15) {
                    return ps_err("rangecheck", "");
                }
                self.push(Obj::Int(v.trunc() as i64))?;
            }
            Cvr => {
                let v = match self.pop()? {
                    Obj::Str(s) => String::from_utf8_lossy(&s.borrow()).trim().parse::<f64>().map_err(|_| PsError::Ps("typecheck", String::new()))?,
                    o => o.as_num().ok_or(PsError::Ps("typecheck", String::new()))?,
                };
                self.push_num(v)?;
            }
            Rand => {
                // A small linear congruential generator: deterministic, like a fresh printer.
                self.seed = self.seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                self.push(Obj::Int(i64::from(self.seed >> 1)))?;
            }
            Srand => self.seed = self.pop_int()? as u32,
            Rrand => self.push(Obj::Int(i64::from(self.seed)))?,
            Eq | Ne => {
                let (b, a) = (self.pop()?, self.pop()?);
                let same = equal(&a, &b);
                self.push(Obj::Bool(same == (op == Eq)))?;
            }
            Gt | Ge | Lt | Le => {
                let (b, a) = (self.pop()?, self.pop()?);
                let ord = match (&a, &b) {
                    (Obj::Str(x), Obj::Str(y)) => x.borrow().partial_cmp(&*y.borrow()),
                    _ => a.as_num().zip(b.as_num()).and_then(|(x, y)| x.partial_cmp(&y)),
                }
                .ok_or(PsError::Ps("typecheck", String::new()))?;
                let r = match op {
                    Gt => ord.is_gt(),
                    Ge => ord.is_ge(),
                    Lt => ord.is_lt(),
                    _ => ord.is_le(),
                };
                self.push(Obj::Bool(r))?;
            }
            And | Or | Xor => {
                let (b, a) = (self.pop()?, self.pop()?);
                let r = match (a, b) {
                    (Obj::Bool(x), Obj::Bool(y)) => Obj::Bool(match op {
                        And => x && y,
                        Or => x || y,
                        _ => x ^ y,
                    }),
                    (Obj::Int(x), Obj::Int(y)) => Obj::Int(match op {
                        And => x & y,
                        Or => x | y,
                        _ => x ^ y,
                    }),
                    _ => return ps_err("typecheck", ""),
                };
                self.push(r)?;
            }
            Not => match self.pop()? {
                Obj::Bool(b) => self.push(Obj::Bool(!b))?,
                Obj::Int(i) => self.push(Obj::Int(!i))?,
                _ => return ps_err("typecheck", ""),
            },
            Bitshift => {
                let (s, v) = (self.pop_int()?, self.pop_int()?);
                let r = if s >= 0 { v.checked_shl(s.min(63) as u32) } else { v.checked_shr((-s).min(63) as u32) };
                self.push(Obj::Int(r.unwrap_or(0)))?;
            }
            True => self.push(Obj::Bool(true))?,
            False => self.push(Obj::Bool(false))?,
            Null => self.push(Obj::Null)?,
            CountExecStack => self.push(Obj::Int(i64::from(self.depth) + 1))?,
            Type => {
                let o = self.pop()?;
                self.push(Obj::Exec(Rc::from(o.type_name())))?;
            }
            Cvx => {
                let o = match self.pop()? {
                    Obj::Array { items, .. } => Obj::Array { items, exec: true },
                    Obj::File { stream, .. } => Obj::File { stream, exec: true },
                    Obj::Str(s) => Obj::ExecStr(s),
                    Obj::Name(n) => Obj::Exec(n),
                    o => o,
                };
                self.push(o)?;
            }
            Cvlit => {
                let o = match self.pop()? {
                    Obj::Array { items, .. } => Obj::Array { items, exec: false },
                    Obj::File { stream, .. } => Obj::File { stream, exec: false },
                    Obj::ExecStr(s) => Obj::Str(s),
                    Obj::Exec(n) => Obj::Name(n),
                    o => o,
                };
                self.push(o)?;
            }
            Xcheck => {
                let o = self.pop()?;
                self.push(Obj::Bool(matches!(
                    o,
                    Obj::Array { exec: true, .. } | Obj::File { exec: true, .. } | Obj::ExecStr(_) | Obj::Exec(_) | Obj::Op(_)
                )))?;
            }
            Cvn => {
                let s = self.pop_str()?;
                let n: Rc<str> = Rc::from(String::from_utf8_lossy(&s.borrow()).as_ref());
                self.push(Obj::Name(n))?;
            }
            Cvs | Cvrs => {
                let s = self.pop_str()?;
                let text = if op == Cvrs {
                    let radix = self.pop_int()?;
                    let v = self.pop_num()?;
                    if !(2..=36).contains(&radix) {
                        return ps_err("rangecheck", "");
                    }
                    radix_text(v as i64, radix as u32)
                } else {
                    show_text(&self.pop()?)
                };
                if !s.write(0, text.as_bytes()) {
                    return ps_err("rangecheck", "");
                }
                self.push_interval(Obj::Str(s), 0, text.len())?;
            }
            ReadOnly | ExecuteOnly | NoAccess | SetPacking | SetGlobal | SetObjectFormat => {
                if matches!(op, SetPacking | SetGlobal | SetObjectFormat) {
                    self.pop()?;
                }
            }
            Rcheck | Wcheck => {
                self.pop()?;
                self.push(Obj::Bool(true))?;
            }
            CurrentPacking | CurrentGlobal => self.push(Obj::Bool(false))?,
            NewArray => {
                let n = self.pop_count()?;
                self.alloc(n * std::mem::size_of::<Obj>())?;
                self.push(Obj::array(vec![Obj::Null; n]))?;
            }
            PackedArray => {
                let n = self.pop_count()?;
                let at = self.stack.len().checked_sub(n).ok_or(PsError::Ps("stackunderflow", String::new()))?;
                let items = self.stack.split_off(at);
                self.push(Obj::array(items))?;
            }
            NewString => {
                let n = self.pop_count()?;
                self.alloc(n)?;
                self.push(Obj::string(vec![0; n]))?;
            }
            Length => {
                let n = match self.pop()? {
                    Obj::Array { items, .. } => items.len(),
                    Obj::Str(s) | Obj::ExecStr(s) => s.len(),
                    Obj::Dict(d) => d.borrow().len(),
                    Obj::Name(n) | Obj::Exec(n) => n.len(),
                    _ => return ps_err("typecheck", ""),
                };
                self.push(Obj::Int(n as i64))?;
            }
            MaxLength => {
                let d = self.pop_dict()?;
                let n = d.borrow().len().max(d.borrow().capacity());
                self.push(Obj::Int(n as i64))?;
            }
            Get => {
                let k = self.pop()?;
                let o = match self.pop()? {
                    Obj::Array { items, .. } => items.get(index(&k)?).ok_or(PsError::Ps("rangecheck", String::new()))?,
                    Obj::Str(s) | Obj::ExecStr(s) => Obj::Int(i64::from(s.get(index(&k)?).ok_or(PsError::Ps("rangecheck", String::new()))?)),
                    Obj::Dict(d) => {
                        let key = k.key().ok_or(PsError::Ps("typecheck", String::new()))?;
                        let v = d.borrow().get(&key).cloned();
                        v.ok_or_else(|| PsError::Ps("undefined", show_text(&k)))?
                    }
                    _ => return ps_err("typecheck", ""),
                };
                self.push(o)?;
            }
            Put => {
                let (v, k, c) = (self.pop()?, self.pop()?, self.pop()?);
                let written = match c {
                    Obj::Array { items, .. } => items.write(index(&k)?, std::slice::from_ref(&v)),
                    Obj::Str(s) => {
                        let b = v.as_num().ok_or(PsError::Ps("typecheck", String::new()))?;
                        s.write(index(&k)?, &[b as i64 as u8])
                    }
                    Obj::Dict(d) => {
                        let key = k.key().ok_or(PsError::Ps("typecheck", String::new()))?;
                        self.insert(&d, key, Some(v))?;
                        true
                    }
                    _ => return ps_err("typecheck", ""),
                };
                if !written {
                    return ps_err("rangecheck", "");
                }
            }
            GetInterval => {
                let n = self.pop_count()?;
                let at = self.pop_count()?;
                let o = self.pop()?;
                self.push_interval(o, at, n)?;
            }
            PutInterval => {
                let src = self.pop()?;
                let at = self.pop_count()?;
                let written = match (self.pop()?, src) {
                    (Obj::Array { items, .. }, Obj::Array { items: from, .. }) => items.write(at, &from.to_vec()),
                    (Obj::Str(s), Obj::Str(from)) => s.write(at, &from.to_vec()),
                    _ => return ps_err("typecheck", ""),
                };
                if !written {
                    return ps_err("rangecheck", "");
                }
            }
            Aload => {
                let items = self.pop_array()?;
                for o in items.to_vec() {
                    self.push(o)?;
                }
                self.push(Obj::Array { items, exec: false })?;
            }
            Astore => {
                let items = self.pop_array()?;
                let at = self.stack.len().checked_sub(items.len()).ok_or(PsError::Ps("stackunderflow", String::new()))?;
                if !items.write(0, self.stack.get(at..).unwrap_or_default()) {
                    return ps_err("rangecheck", "");
                }
                self.stack.truncate(at);
                self.push(Obj::Array { items, exec: false })?;
            }
            Search | AnchorSearch => {
                let seek = self.pop_str()?.to_vec();
                let s = self.pop_str()?;
                let text = s.to_vec();
                let at = if op == Search { super::lex::find(&text, &seek) } else { text.starts_with(&seek).then_some(0) };
                match at {
                    Some(i) => {
                        let o = Obj::Str(s);
                        if op == Search {
                            self.push_interval(o.clone(), i + seek.len(), text.len() - i - seek.len())?;
                            self.push_interval(o.clone(), i, seek.len())?;
                            self.push_interval(o, 0, i)?;
                        } else {
                            self.push_interval(o.clone(), seek.len(), text.len() - seek.len())?;
                            self.push_interval(o, 0, seek.len())?;
                        }
                        self.push(Obj::Bool(true))?;
                    }
                    None => {
                        self.push(Obj::Str(s))?;
                        self.push(Obj::Bool(false))?;
                    }
                }
            }
            NewDict => {
                self.pop_count()?;
                self.push(Obj::dict(Default::default()))?;
            }
            Begin => {
                let d = self.pop_dict()?;
                if self.dicts.len() >= MAX_DICTS {
                    return Err(PsError::Limit("the dictionary stack overflowed"));
                }
                self.dicts.push(d);
            }
            End => {
                // systemdict, globaldict and userdict stay.
                if self.dicts.len() <= 3 {
                    return ps_err("dictstackunderflow", "end");
                }
                self.dicts.pop();
            }
            Def => {
                let v = self.pop()?;
                let k = self.pop()?.key().ok_or(PsError::Ps("typecheck", String::new()))?;
                let d = self.dicts.last().cloned().ok_or(PsError::Ps("dictstackunderflow", String::new()))?;
                self.insert(&d, k, Some(v))?;
            }
            Load => {
                let k = self.pop()?;
                let key = k.key().ok_or(PsError::Ps("typecheck", String::new()))?;
                let v = self.lookup_key(&key).ok_or_else(|| PsError::Ps("undefined", show_text(&k)))?;
                self.push(v)?;
            }
            Store => {
                let v = self.pop()?;
                let k = self.pop()?.key().ok_or(PsError::Ps("typecheck", String::new()))?;
                let d = self.dicts.iter().rev().find(|d| d.borrow().contains_key(&k)).or(self.dicts.last()).cloned();
                let d = d.ok_or(PsError::Ps("dictstackunderflow", String::new()))?;
                self.insert(&d, k, Some(v))?;
            }
            Known => {
                let k = self.pop()?.key().ok_or(PsError::Ps("typecheck", String::new()))?;
                let d = self.pop_dict()?;
                let known = d.borrow().contains_key(&k);
                self.push(Obj::Bool(known))?;
            }
            Where => {
                let k = self.pop()?.key().ok_or(PsError::Ps("typecheck", String::new()))?;
                match self.dicts.iter().rev().find(|d| d.borrow().contains_key(&k)).cloned() {
                    Some(d) => {
                        self.push(Obj::Dict(d))?;
                        self.push(Obj::Bool(true))?;
                    }
                    None => self.push(Obj::Bool(false))?,
                }
            }
            Undef => {
                let k = self.pop()?.key().ok_or(PsError::Ps("typecheck", String::new()))?;
                let d = self.pop_dict()?;
                self.insert(&d, k, None)?;
            }
            CurrentDict => {
                let d = self.dicts.last().cloned().ok_or(PsError::Ps("dictstackunderflow", String::new()))?;
                self.push(Obj::Dict(d))?;
            }
            CountDictStack => self.push(Obj::Int(self.dicts.len() as i64))?,
            DictStack => {
                // The dictionary stack, bottom first, stored into the array's start.
                let items = self.pop_array()?;
                let dicts: Vec<Obj> = self.dicts.iter().map(|d| Obj::Dict(d.clone())).collect();
                let part = items.sub(0, dicts.len()).filter(|p| p.write(0, &dicts)).ok_or(PsError::Ps("rangecheck", String::new()))?;
                self.push(Obj::Array { items: part, exec: false })?;
            }
            SystemDict => self.push(Obj::Dict(self.system.clone()))?,
            UserDict => self.push(Obj::Dict(self.user.clone()))?,
            GlobalDict => self.push(Obj::Dict(self.global.clone()))?,
            StatusDict => self.push(Obj::Dict(self.status.clone()))?,
            ErrorDict | DollarError => self.push(Obj::Dict(self.error.clone()))?,
            InternalDict => {
                // The operand is a password that every interpreter accepts.
                self.pop_int()?;
                self.push(Obj::Dict(self.internal.clone()))?;
            }
            Bind => {
                let o = self.pop()?;
                if let Obj::Array { items, .. } = &o {
                    self.bind(items, 0);
                }
                self.push(o)?;
            }
            Save => {
                self.gsave()?;
                self.push(Obj::Save(self.saves.len()))?;
                self.saves.push(SaveMark { gstates: self.saved.len(), journal: self.journal.len() });
            }
            Restore => {
                let Obj::Save(i) = self.pop()? else { return ps_err("typecheck", "") };
                // A save restored already (or through an outer one) is no longer valid: §3.7.3,
                // `invalidrestore`.
                let mark = self.saves.get(i).copied().ok_or(PsError::Ps("invalidrestore", String::new()))?;
                self.saves.truncate(i);
                // The dictionaries as they were at the save, newest change undone first.
                for (d, k, old) in self.journal.drain(mark.journal..).rev() {
                    let mut d = d.borrow_mut();
                    match old {
                        Some(v) => d.insert(k, v),
                        None => d.remove(&k),
                    };
                }
                while self.saved.len() >= mark.gstates && !self.saved.is_empty() {
                    self.grestore();
                }
            }
            VmStatus => {
                for v in [0, 1 << 20, 1 << 30] {
                    self.push(Obj::Int(v))?;
                }
            }
            Version => self.push(Obj::string(b"3010".to_vec()))?,
            Revision => self.push(Obj::Int(1))?,
            SerialNumber => self.push(Obj::Int(0))?,
            Product => self.push(Obj::string(b"VectorCraft".to_vec()))?,
            RealTime | UserTime => self.push(Obj::Int(0))?,
            Print | EqPrint | EqEqPrint => {
                self.pop()?;
            }
            Pstack | Stack | Flush => {}
            // What every interpreter (not only a distiller) answers, as Ghostscript does: the marks
            // are dropped.
            PdfMark => {
                self.pop_to_mark()?;
            }
            Gcheck | Scheck => {
                self.pop()?;
                self.push(Obj::Bool(false))?;
            }
            CurrentShared => self.push(Obj::Bool(false))?,
            SetShared | SetVmThreshold | VmReclaim | Echo => {
                self.pop()?;
            }
            ClearDictStack => self.dicts.truncate(3),
            ExecStack => {
                let items = self.pop_array()?;
                self.push_interval(Obj::Array { items, exec: false }, 0, 0)?;
            }
            SetCacheParams => {
                self.pop_to_mark()?;
            }
            CurrentCacheParams | UCacheStatus => {
                self.push(Obj::Mark)?;
                for _ in 0..if op == UCacheStatus { 5 } else { 2 } {
                    self.push(Obj::Int(1 << 20))?;
                }
            }
            // An empty font cache of a device's sizes, in `cachestatus`'s order (PLRM 3rd ed.,
            // chapter 8): bytes used and most, fonts used and most, glyphs used and most, and the
            // most bytes one cached glyph may take (positive on a real device: programs divide by it).
            CacheStatus => {
                for v in [0, 1 << 20, 0, 1000, 0, 10_000, 25_000] {
                    self.push(Obj::Int(v))?;
                }
            }
            StartJob => {
                self.pop()?;
                self.pop()?;
                self.push(Obj::Bool(false))?;
            }
            CurrentObjectFormat => self.push(Obj::Int(0))?,
            SetDevParams => {
                self.pop_dict()?;
                self.pop()?;
            }
            CurrentDevParams => {
                self.pop()?;
                self.push(Obj::dict(Dict::new()))?;
            }
            _ => return Ok(true),
        }
        Ok(false)
    }

    /// The objects above the topmost mark, which is removed.
    fn pop_to_mark(&mut self) -> Res<Vec<Obj>> {
        let at = self.stack.iter().rposition(|o| matches!(o, Obj::Mark)).ok_or(PsError::Ps("unmatchedmark", String::new()))?;
        let items = self.stack.split_off(at + 1);
        self.stack.pop();
        Ok(items)
    }

    /// `copy`: the top `n` operands again, or a composite into another.
    fn copy(&mut self) -> Res {
        match self.pop()? {
            Obj::Int(n) => {
                let n = usize::try_from(n).map_err(|_| PsError::Ps("rangecheck", String::new()))?;
                let at = self.stack.len().checked_sub(n).ok_or(PsError::Ps("stackunderflow", String::new()))?;
                self.alloc(n * std::mem::size_of::<Obj>())?;
                let top: Vec<Obj> = self.stack.get(at..).unwrap_or_default().to_vec();
                for o in top {
                    self.push(o)?;
                }
            }
            Obj::Array { items, exec } => {
                let from = self.pop_array()?.to_vec();
                if !items.write(0, &from) {
                    return ps_err("rangecheck", "copy");
                }
                self.push_interval(Obj::Array { items, exec }, 0, from.len())?;
            }
            Obj::Str(s) => {
                let from = self.pop_str()?.to_vec();
                if !s.write(0, &from) {
                    return ps_err("rangecheck", "copy");
                }
                self.push_interval(Obj::Str(s), 0, from.len())?;
            }
            Obj::Dict(d) => {
                let from = self.pop_dict()?.borrow().clone();
                for (k, v) in from {
                    self.insert(&d, k, Some(v))?;
                }
                self.push(Obj::Dict(d))?;
            }
            _ => return ps_err("typecheck", "copy"),
        }
        Ok(())
    }

    /// `n` items of an array or string from `at`, sharing its storage.
    pub fn push_interval(&mut self, o: Obj, at: usize, n: usize) -> Res {
        let range = || PsError::Ps("rangecheck", String::new());
        let r = match o {
            Obj::Array { items, exec } => Obj::Array { items: items.sub(at, n).ok_or_else(range)?, exec },
            Obj::Str(s) => Obj::Str(s.sub(at, n).ok_or_else(range)?),
            Obj::ExecStr(s) => Obj::ExecStr(s.sub(at, n).ok_or_else(range)?),
            _ => return ps_err("typecheck", "getinterval"),
        };
        self.push(r)
    }

    fn forall(&mut self) -> Res {
        let p = self.pop_proc()?;
        match self.pop()? {
            Obj::Array { items, .. } => {
                let mut i = 0;
                while let Some(o) = items.get(i) {
                    i += 1;
                    self.push(o)?;
                    if !self.body(&p)? {
                        break;
                    }
                }
            }
            Obj::Str(s) | Obj::ExecStr(s) => {
                for b in s.to_vec() {
                    self.push(Obj::Int(i64::from(b)))?;
                    if !self.body(&p)? {
                        break;
                    }
                }
            }
            Obj::Dict(d) => {
                let pairs: Vec<(Key, Obj)> = d.borrow().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                for (k, v) in pairs {
                    self.push(k.obj())?;
                    self.push(v)?;
                    if !self.body(&p)? {
                        break;
                    }
                }
            }
            _ => return ps_err("typecheck", "forall"),
        }
        Ok(())
    }

    /// Replace the names in a procedure that stand for operators by the operators, down through
    /// nested procedures.
    fn bind(&mut self, items: &Shared<Obj>, depth: u32) {
        if depth > 64 {
            return;
        }
        for i in 0..items.len() {
            match items.get(i) {
                Some(Obj::Exec(name)) => {
                    if let Some(o @ Obj::Op(_)) = self.lookup(&name) {
                        // `i` is in range and nothing is reading the procedure: the write can't fail.
                        items.write(i, &[o]);
                    }
                }
                Some(Obj::Array { items: inner, exec: true }) if !inner.shares(items) => self.bind(&inner, depth + 1),
                _ => {}
            }
        }
    }

    /// Put `v` under `k` in `d` (`None`: remove it), within [`MAX_ALLOC`] entries; while a `save`
    /// is open the old value is kept for `restore`.
    fn insert(&mut self, d: &DictRef, k: Key, v: Option<Obj>) -> Res {
        let old = {
            let mut dm = d.borrow_mut();
            if v.is_some() && dm.len() >= MAX_ALLOC && !dm.contains_key(&k) {
                return Err(PsError::Limit("a dictionary grew too large"));
            }
            match v {
                Some(v) => dm.insert(k.clone(), v),
                None => dm.remove(&k),
            }
        };
        if !self.saves.is_empty() {
            self.alloc(2 * std::mem::size_of::<Obj>())?;
            self.journal.push((d.clone(), k, old));
        }
        Ok(())
    }

    // ---------- resources ----------

    /// The resources of `category`.
    pub fn category(&mut self, category: &Obj) -> Res<DictRef> {
        let k = category.key().ok_or(PsError::Ps("typecheck", String::new()))?;
        let mut all = self.resources.borrow_mut();
        Ok(all.entry(k).or_insert_with(|| Obj::Dict(new_dict())).clone()).and_then(|o| match o {
            Obj::Dict(d) => Ok(d),
            _ => ps_err("typecheck", ""),
        })
    }

    /// The instances of `category`: `FontDirectory` for fonts.
    pub fn instances(&mut self, category: &Obj) -> Res<DictRef> {
        if category.text().as_deref() == Some("Font") { Ok(self.fonts.clone()) } else { self.category(category) }
    }

    /// `template proc scratch category resourceforall`: `proc` run on each instance name that
    /// matches `template`, copied into `scratch` (in name order).
    pub fn resource_for_all(&mut self) -> Res {
        let category = self.pop()?;
        let scratch = self.pop_str()?;
        let p = self.pop_proc()?;
        let template = self.pop_str()?.to_vec();
        let mut names: Vec<Vec<u8>> = self.instances(&category)?.borrow().keys().map(|k| show_text(&k.obj()).into_bytes()).collect();
        names.sort();
        for name in names {
            if !self.matches(&template, &name)? {
                continue;
            }
            if !scratch.write(0, &name) {
                return ps_err("rangecheck", "resourceforall");
            }
            self.push_interval(Obj::Str(scratch.clone()), 0, name.len())?;
            if !self.body(&p)? {
                break;
            }
        }
        Ok(())
    }

    /// Does `name` match a `resourceforall` template: `*` any run of bytes, `?` any one, `\` the
    /// next one as it is? Each step counts as an operation.
    fn matches(&mut self, template: &[u8], name: &[u8]) -> Res<bool> {
        let (mut t, mut n) = (0, 0);
        // Where the last `*` was followed (template, name): a mismatch after it retries from there.
        let mut star: Option<(usize, usize)> = None;
        loop {
            self.tick()?;
            if n == name.len() {
                return Ok(template.get(t..).unwrap_or_default().iter().all(|b| *b == b'*'));
            }
            let (want, width) = match template.get(t) {
                Some(b'*') => {
                    t += 1;
                    star = Some((t, n));
                    continue;
                }
                Some(b'?') => (name.get(n).copied(), 1),
                Some(b'\\') => (template.get(t + 1).copied(), 2),
                c => (c.copied(), 1),
            };
            if want.is_some() && want == name.get(n).copied() {
                (t, n) = (t + width, n + 1);
                continue;
            }
            // The last `*` takes one more byte, else there is no match.
            let Some((st, sn)) = star else { return Ok(false) };
            star = Some((st, sn + 1));
            (t, n) = (st, sn + 1);
        }
    }
}

/// An array or string index.
fn index(k: &Obj) -> Res<usize> {
    match *k {
        Obj::Int(i) if i >= 0 => Ok(i as usize),
        _ => ps_err("rangecheck", "an index"),
    }
}

/// `eq`: numbers by value, strings and names by text, composites by identity.
fn equal(a: &Obj, b: &Obj) -> bool {
    match (a, b) {
        (Obj::Null, Obj::Null) | (Obj::Mark, Obj::Mark) => true,
        (Obj::Bool(x), Obj::Bool(y)) => x == y,
        (Obj::Array { items: x, .. }, Obj::Array { items: y, .. }) => x.same(y),
        (Obj::Dict(x), Obj::Dict(y)) => Rc::ptr_eq(x, y),
        (Obj::Op(x), Obj::Op(y)) => x == y,
        _ => match (a.as_num(), b.as_num()) {
            (Some(x), Some(y)) => x == y,
            _ => matches!((a.text(), b.text()), (Some(x), Some(y)) if x == y),
        },
    }
}

/// The text `cvs` writes for `o`.
fn show_text(o: &Obj) -> String {
    match o {
        Obj::Int(i) => i.to_string(),
        Obj::Real(r) => {
            let s = format!("{r}");
            if s.contains('.') || s.contains('e') { s } else { format!("{s}.0") }
        }
        Obj::Bool(b) => b.to_string(),
        Obj::Name(n) | Obj::Exec(n) => n.to_string(),
        Obj::Str(s) | Obj::ExecStr(s) => String::from_utf8_lossy(&s.borrow()).into_owned(),
        Obj::Op(op) => op.name().to_string(),
        _ => "--nostringval--".into(),
    }
}

/// `v` in `radix`, as `cvrs` writes it (negative numbers as their 32-bit two's complement).
fn radix_text(v: i64, radix: u32) -> String {
    let mut n = if v < 0 { u64::from(v as u32) } else { v as u64 };
    if n == 0 {
        return "0".into();
    }
    let mut digits = vec![];
    while n > 0 {
        digits.push(std::char::from_digit((n % u64::from(radix)) as u32, radix).unwrap_or('0').to_ascii_uppercase());
        n /= u64::from(radix);
    }
    digits.iter().rev().collect()
}

/// A matrix from six numbers.
pub(crate) fn matrix_of(items: &[Obj]) -> Option<Affine> {
    let [a, b, c, d, e, f] = items else { return None };
    let v = [a, b, c, d, e, f].map(|o| o.as_num().filter(|v| v.is_finite()));
    let [Some(a), Some(b), Some(c), Some(d), Some(e), Some(f)] = v else { return None };
    Some(Affine::new([a, b, c, d, e, f]))
}

/// A matrix as an array.
pub(crate) fn matrix_obj(m: Affine) -> Obj {
    Obj::array(m.as_coeffs().iter().map(|v| Obj::Real(*v)).collect())
}
