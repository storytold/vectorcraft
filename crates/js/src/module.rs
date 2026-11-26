//! Running modules (ECMA-262 2015 section 15.2.1): loading a module and the modules it imports,
//! linking their imports to the bindings their exports name, and readying them to run, each once,
//! the modules a module imports before it.
//!
//! The host maps each specifier to a module ([`ModuleLoader`], the role of section 15.2.1.17,
//! HostResolveImportedModule): it gives a key that names the module, and its source the first time
//! it gives that key. Every module is parsed and compiled before any runs, and every import is
//! linked before any runs, so a syntax error, a missing or ambiguous export, or a module the host
//! cannot give ends the run before any of its code runs. The run's [`ModuleLimits`] bound how many
//! modules it loads and how deep its imports go; parsing takes fuel for each code unit, and each
//! binding made in linking takes a unit.
//!
//! An import reads the exporting module's binding itself, so it sees each later assignment, and a
//! read before that binding is initialized (a module that imports one that imports it back) is a
//! `ReferenceError`. A module namespace object (`import * as ns`) is not extensible, has no
//! prototype, takes no writes and has the module's export names, in code unit order, as its
//! properties. `export *` passes on every name but `default`; a name two `export *` give from
//! different bindings is left out of a namespace and is a `SyntaxError` to import.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::ast::{ExportEntry, ImportName, ModuleInfo};
use crate::bytecode::Code;
use crate::heap::{Binding, Env, EnvId, ObjId, Prop, key};
use crate::interp::{Abrupt, CompileError, ErrorType, Interp};
use crate::value::Value;

/// What the host gives for a module a run imports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedModule {
    /// What names the module: two specifiers that give one key import one module, which runs once.
    pub key: String,
    /// The module's source, given the first time the host gives its key in a run. `None` for a key
    /// the host gave before in the same run.
    pub source: Option<String>,
}

/// Why the host cannot give a module, which ends the run before any module runs: the error the
/// `import` raises, with its message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// A specifier the host does not read: a `SyntaxError`.
    Syntax(String),
    /// A limit of the host's: a `RangeError`.
    Range(String),
    /// A module that is missing, refused or unreadable: an `Error`.
    Other(String),
}

/// The host's map from a specifier to a module: called with the key of the importing module and
/// the specifier as written.
pub type ModuleLoader<'a> = dyn FnMut(&str, &str) -> Result<LoadedModule, LoadError> + 'a;

/// Bounds on the modules of one run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModuleLimits {
    /// The most modules a run loads, the first included.
    pub modules: usize,
    /// The most imports from the first module to any other, along the shortest chain.
    pub depth: usize,
}

impl Default for ModuleLimits {
    fn default() -> Self {
        Self { modules: 64, depth: 16 }
    }
}

/// The most modules a run loads, whatever its limits say: [`resolve_export`] and
/// [`exported_names`] recurse once for each module at most.
const MAX_MODULES: usize = 1024;
/// The most `(module, name)` pairs one export's resolution visits.
const MAX_RESOLVE_STEPS: usize = 1024;

#[derive(Default)]
struct ResolveSeen {
    pairs: HashSet<ResolveKey>,
    count: usize,
}

struct ResolveKey {
    module: usize,
    name: String,
}

impl PartialEq for ResolveKey {
    fn eq(&self, other: &Self) -> bool {
        #[cfg(test)]
        RESOLVE_MEMBERSHIP_CHECKS.with(|count| count.set(count.get() + 1));
        self.module == other.module && self.name == other.name
    }
}

impl Eq for ResolveKey {}

impl std::hash::Hash for ResolveKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&(self.module, &self.name), state);
    }
}

#[cfg(test)]
thread_local! {
    pub(crate) static RESOLVED_STAR_EDGES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static RESOLVE_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static RESOLVE_MEMBERSHIP_CHECKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A loaded module.
struct Record {
    key: String,
    info: ModuleInfo,
    code: Rc<Code>,
    tag: u32,
    /// The module each of `info.requests` names, by index.
    requested: Vec<usize>,
    /// The environment its code runs in and the outer one, once instantiated.
    envs: Option<(EnvId, EnvId)>,
    namespace: Option<ObjId>,
    /// Each name the module exports itself or re-exports by name, with its entry in
    /// `info.exports`.
    explicit: HashMap<String, usize>,
    /// Each `export *` request, replaced by distinct canonical targets after loading, in order.
    stars: Vec<usize>,
}

/// Why a module run could not start.
pub(crate) enum LinkFail {
    /// A module does not parse; `tag` is its source's.
    Syntax { tag: u32, error: crate::SyntaxError },
    /// An exception, raised at `line` and `column` of the module whose source has `tag`.
    At { tag: u32, line: u32, column: u32, abrupt: Abrupt },
    /// A limit or an internal error, not at any place.
    Run(Abrupt),
}

/// Where an export resolves to (ResolvedBinding).
#[derive(Clone, PartialEq, Eq)]
enum Resolution {
    /// The binding `name` of module `module`.
    Binding { module: usize, name: String },
    /// The namespace object of module `module` (`export * as ns from`).
    Namespace(usize),
}

/// An export that two `export *` declarations give from different places.
struct Ambiguous;

/// The modules of a run, loaded and linked.
struct Graph<'a, 'l> {
    interp: &'a mut Interp,
    records: Vec<Record>,
    by_key: HashMap<String, usize>,
    load: &'a mut ModuleLoader<'l>,
}

/// Loads the module `key` with `source` and every module it imports, instantiates and links them,
/// and gives them in the order they run, each with its code, the environment it runs in and the
/// outer one. The first module's source has tag `entry_tag`; each module loaded after it takes
/// the next tag. The environments stay in the interpreter's temporary roots for the caller to
/// remove once their frames are pushed.
pub(crate) fn link(
    interp: &mut Interp,
    key_name: &str,
    source: &str,
    entry_tag: u32,
    limits: ModuleLimits,
    load: &mut ModuleLoader<'_>,
) -> Result<Vec<(Rc<Code>, EnvId, EnvId)>, LinkFail> {
    let mut graph = Graph { interp, records: Vec::new(), by_key: HashMap::new(), load };
    if limits.modules == 0 {
        return Err(graph.fail_at(0, 0, ErrorType::Range, "module limit (0) reached"));
    }
    graph.add(key_name.to_string(), source, entry_tag)?;
    graph.load_imports(limits)?;
    graph.instantiate()?;
    graph.link_imports()?;
    Ok(graph.evaluation_order())
}

impl Graph<'_, '_> {
    /// Parses and compiles a module's source under `tag`.
    fn add(&mut self, key_name: String, source: &str, tag: u32) -> Result<usize, LinkFail> {
        let interp = &mut *self.interp;
        let minimum_units = source.len().div_ceil(3);
        if u64::try_from(minimum_units).unwrap_or(u64::MAX) > interp.fuel {
            interp.prepare_source(minimum_units).map_err(LinkFail::Run)?;
        }
        interp.prepare_source(0).map_err(LinkFail::Run)?;
        interp.prepare_source(source.encode_utf16().count()).map_err(LinkFail::Run)?;
        let saved = std::mem::replace(&mut interp.source_tag, tag);
        let compiled = interp.encode_source(source).and_then(|units| interp.compile_module_source(&units));
        interp.source_tag = saved;
        let (code, info) = match compiled {
            Ok(c) => c,
            Err(CompileError::Syntax(error)) => return Err(LinkFail::Syntax { tag, error }),
            Err(CompileError::Internal(m)) => return Err(LinkFail::Run(Abrupt::Internal(m))),
        };
        // The record keeps the code from here on, so the heap counts it from here on.
        self.interp.hold_module_code(&code).map_err(LinkFail::Run)?;
        let mut explicit = HashMap::new();
        let mut stars = Vec::new();
        for (i, e) in info.exports.iter().enumerate() {
            match e {
                ExportEntry::Local { export_name, .. } | ExportEntry::Indirect { export_name, .. } => {
                    explicit.entry(export_name.clone()).or_insert(i);
                }
                ExportEntry::Star { request, .. } => stars.push(*request),
            }
        }
        self.records.push(Record { key: key_name.clone(), info, code, tag, requested: Vec::new(), envs: None, namespace: None, explicit, stars });
        let index = self.records.len() - 1;
        self.by_key.insert(key_name, index);
        Ok(index)
    }

    /// An exception of type `t` with `message`, at the source offset `offset` of module `m`.
    fn fail_at(&mut self, m: usize, offset: u32, t: ErrorType, message: &str) -> LinkFail {
        let (tag, (line, column)) = match self.records.get(m) {
            Some(r) => (r.tag, r.code.source.lines.line_column(offset)),
            None => (crate::realm::UNKNOWN_SOURCE, (0, 0)),
        };
        LinkFail::At { tag, line, column, abrupt: self.interp.throw(t, message) }
    }

    /// Loads the modules each loaded module imports, breadth first, so a module's depth is the
    /// length of the shortest chain of imports to it.
    fn load_imports(&mut self, limits: ModuleLimits) -> Result<(), LinkFail> {
        let max_modules = limits.modules.min(MAX_MODULES);
        let mut depths = vec![0usize];
        let mut i = 0;
        while i < self.records.len() {
            let requests: Vec<(String, bool, u32)> = self.records[i]
                .info
                .requests
                .iter()
                .map(|r| (r.specifier.clone(), char::decode_utf16(r.units.iter().copied()).all(|c| c.is_ok()), r.span.start))
                .collect();
            let depth = depths.get(i).copied().unwrap_or(0);
            for (specifier, text, offset) in requests {
                self.interp.use_fuel().map_err(LinkFail::Run)?;
                // The host maps text to modules, and a specifier with an unpaired surrogate is not
                // text, so it is refused here, as a host refuses a specifier it does not read.
                if !text {
                    let message = format!("module specifier '{specifier}' is not valid text: it has an unpaired surrogate");
                    return Err(self.fail_at(i, offset, ErrorType::Syntax, &message));
                }
                let referrer = self.records[i].key.clone();
                let loaded = match (self.load)(&referrer, &specifier) {
                    Ok(loaded) => loaded,
                    Err(LoadError::Syntax(why)) => return Err(self.fail_at(i, offset, ErrorType::Syntax, &why)),
                    Err(LoadError::Range(why)) => return Err(self.fail_at(i, offset, ErrorType::Range, &why)),
                    Err(LoadError::Other(why)) => return Err(self.fail_at(i, offset, ErrorType::Error, &why)),
                };
                let target = match (self.by_key.get(&loaded.key), loaded.source) {
                    (Some(&j), _) => j,
                    (None, None) => {
                        let message = format!("the module '{specifier}' was given without its source");
                        return Err(self.fail_at(i, offset, ErrorType::Error, &message));
                    }
                    (None, Some(source)) => {
                        if depth >= limits.depth {
                            let message = format!("import depth limit ({}) reached at '{specifier}'", limits.depth);
                            return Err(self.fail_at(i, offset, ErrorType::Range, &message));
                        }
                        if self.records.len() >= max_modules {
                            let message = format!("module limit ({max_modules}) reached at '{specifier}'");
                            return Err(self.fail_at(i, offset, ErrorType::Range, &message));
                        }
                        let tag = crate::realm::next_tag(self.records.last().map_or(0, |r| r.tag));
                        self.interp.evaluated = tag;
                        depths.push(depth + 1);
                        self.add(loaded.key, &source, tag)?
                    }
                };
                self.records[i].requested.push(target);
            }
            let mut seen_targets = HashSet::new();
            let targets: Vec<usize> = self.records[i]
                .stars
                .iter()
                .filter_map(|&request| self.records[i].requested.get(request).copied())
                .filter(|target| seen_targets.insert(*target))
                .collect();
            self.records[i].stars = targets;
            i += 1;
        }
        Ok(())
    }

    /// Makes each module's environments, with its variables, functions and `let` and `const`
    /// names.
    fn instantiate(&mut self) -> Result<(), LinkFail> {
        for i in 0..self.records.len() {
            let code = self.records[i].code.clone();
            let envs = self.interp.instantiate_module(&code).map_err(LinkFail::Run)?;
            self.records[i].envs = Some(envs);
            for import in &self.records[i].info.imports {
                if matches!(&import.import, ImportName::Namespace) {
                    self.interp.add_import(envs.1, key(&import.local.name), Binding::lexical(true)).map_err(LinkFail::Run)?;
                }
            }
        }
        Ok(())
    }

    /// The module the request `request` of module `m` names.
    fn target(&self, m: usize, request: usize) -> Option<usize> {
        self.records.get(m).and_then(|r| r.requested.get(request)).copied()
    }

    /// The specifier of request `request` of module `m`, as written.
    fn specifier(&self, m: usize, request: usize) -> String {
        self.records.get(m).and_then(|r| r.info.requests.get(request)).map(|r| r.specifier.clone()).unwrap_or_default()
    }

    /// ResolveExport (section 15.2.1.16.3): where the export `name` of module `m` comes from,
    /// `None` when the module has no such export (or only through a cycle).
    fn resolve_export(&mut self, m: usize, name: &str, seen: &mut ResolveSeen) -> Result<Result<Option<Resolution>, Ambiguous>, LinkFail> {
        #[cfg(test)]
        RESOLVE_CALLS.with(|count| count.set(count.get() + 1));
        let visit = ResolveKey { module: m, name: name.to_string() };
        if seen.pairs.contains(&visit) {
            return Ok(Ok(None));
        }
        if seen.count >= MAX_RESOLVE_STEPS {
            let message = format!("export resolution limit ({MAX_RESOLVE_STEPS} steps) reached for '{name}'");
            return Err(LinkFail::Run(self.interp.throw(ErrorType::Range, &message)));
        }
        self.interp.use_fuel().map_err(LinkFail::Run)?;
        seen.pairs.insert(visit);
        seen.count += 1;
        let Some(record) = self.records.get(m) else { return Ok(Ok(None)) };
        // The module's own export of the name, looked up in its index, comes before any `export *`.
        let explicit = record.explicit.get(name).and_then(|&i| record.info.exports.get(i));
        match explicit {
            Some(ExportEntry::Local { local, .. }) => return Ok(Ok(Some(Resolution::Binding { module: m, name: local.clone() }))),
            Some(ExportEntry::Indirect { request, import, .. }) => {
                let (request, import) = (*request, import.clone());
                let Some(t) = self.target(m, request) else { return Ok(Ok(None)) };
                return match import {
                    ImportName::Namespace => Ok(Ok(Some(Resolution::Namespace(t)))),
                    ImportName::Name(n) => self.resolve_export(t, &n, seen),
                };
            }
            _ => {}
        }
        if name == "default" {
            return Ok(Ok(None));
        }
        let mut star = None;
        let stars = record.stars.clone();
        for t in stars {
            #[cfg(test)]
            RESOLVED_STAR_EDGES.with(|count| count.set(count.get() + 1));
            self.interp.use_fuel().map_err(LinkFail::Run)?;
            match self.resolve_export(t, name, seen)? {
                Err(Ambiguous) => return Ok(Err(Ambiguous)),
                Ok(None) => {}
                Ok(Some(r)) => match &star {
                    None => star = Some(r),
                    Some(s) if *s == r => {}
                    Some(_) => return Ok(Err(Ambiguous)),
                },
            }
        }
        Ok(Ok(star))
    }

    /// GetExportedNames (section 15.2.1.16.2): the names module `m` exports, those of its
    /// `export *` modules but `default` included, each once.
    fn exported_names(
        &mut self,
        m: usize,
        star: bool,
        visited: &mut Vec<usize>,
        names: &mut Vec<String>,
        seen: &mut HashSet<String>,
    ) -> Result<(), LinkFail> {
        if visited.contains(&m) {
            return Ok(());
        }
        visited.push(m);
        let count = self.records.get(m).map_or(0, |r| r.info.exports.len());
        for i in 0..count {
            self.interp.use_fuel().map_err(LinkFail::Run)?;
            let Some(e) = self.records.get(m).and_then(|r| r.info.exports.get(i)) else { break };
            match e {
                ExportEntry::Local { export_name, .. } | ExportEntry::Indirect { export_name, .. } => {
                    if !(star && export_name == "default") && !seen.contains(export_name) {
                        seen.insert(export_name.clone());
                        names.push(export_name.clone());
                    }
                }
                ExportEntry::Star { request, .. } => {
                    if let Some(t) = self.target(m, *request) {
                        self.exported_names(t, true, visited, names, seen)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// The environment and name of the binding `name` of module `m`.
    fn binding_of(&self, m: usize, name: &str) -> Option<(EnvId, crate::heap::Key)> {
        let (lex, env) = self.records.get(m)?.envs?;
        let k = key(name);
        self.interp.module_binding(lex, env, &k).map(|e| (e, k))
    }

    /// GetModuleNamespace (section 15.2.1.18): module `m`'s namespace object, made the first time.
    fn namespace(&mut self, m: usize) -> Result<ObjId, LinkFail> {
        if let Some(ns) = self.records.get(m).and_then(|r| r.namespace) {
            return Ok(ns);
        }
        let mut names = Vec::new();
        self.exported_names(m, false, &mut Vec::new(), &mut names, &mut HashSet::new())?;
        let mut resolved = Vec::new();
        for name in names {
            if let Ok(Some(r)) = self.resolve_export(m, &name, &mut ResolveSeen::default())? {
                resolved.push((key(&name), r));
            }
        }
        resolved.sort_by(|(a, _), (b, _)| a.cmp(b));
        let mut exports = Vec::new();
        let mut later = Vec::new();
        for (k, r) in resolved {
            match r {
                Resolution::Binding { module, name } => match self.binding_of(module, &name) {
                    Some(b) => exports.push((k, Some(b))),
                    None => return Err(LinkFail::Run(Abrupt::Internal(format!("module binding '{name}' not found")))),
                },
                Resolution::Namespace(module) => {
                    exports.push((k.clone(), None));
                    later.push((k, module));
                }
            }
        }
        let ns = self.interp.module_namespace(exports, &[]).map_err(LinkFail::Run)?;
        self.interp.hold(Value::Object(ns));
        if let Some(r) = self.records.get_mut(m) {
            r.namespace = Some(ns);
        }
        // A namespace that holds another is made first, so two that hold each other end.
        for (k, module) in later {
            let inner = self.namespace(module)?;
            let flags = crate::heap::WRITABLE | crate::heap::ENUMERABLE;
            self.interp.define(ns, k, Prop::Data { value: Value::Object(inner), flags }).map_err(LinkFail::Run)?;
        }
        Ok(ns)
    }

    /// Binds each module's imports (section 15.2.1.16.4, steps 9 and 12), after checking that
    /// each re-export resolves.
    fn link_imports(&mut self) -> Result<(), LinkFail> {
        for m in 0..self.records.len() {
            let exports = self.records[m].info.exports.clone();
            for e in &exports {
                let ExportEntry::Indirect { export_name, request, span, .. } = e else { continue };
                if self.target(m, *request).is_none() {
                    continue;
                }
                match self.resolve_export(m, export_name, &mut ResolveSeen::default())? {
                    Ok(Some(_)) => {}
                    Ok(None) => {
                        let message = format!("the module '{}' has no export to give as '{export_name}'", self.specifier(m, *request));
                        return Err(self.fail_at(m, span.start, ErrorType::Syntax, &message));
                    }
                    Err(Ambiguous) => {
                        let message = format!("the export '{export_name}' is ambiguous");
                        return Err(self.fail_at(m, span.start, ErrorType::Syntax, &message));
                    }
                }
            }
            let imports = self.records[m].info.imports.clone();
            let Some((_, env)) = self.records[m].envs else { return Err(LinkFail::Run(Abrupt::Internal("module not instantiated".into()))) };
            for imp in imports {
                let Some(t) = self.target(m, imp.request) else { continue };
                let local = key(&imp.local.name);
                let binding = match &imp.import {
                    ImportName::Namespace => self.namespace_binding(t)?,
                    ImportName::Name(n) => match self.resolve_export(t, n, &mut ResolveSeen::default())? {
                        Ok(Some(Resolution::Binding { module, name })) => match self.binding_of(module, &name) {
                            Some((target_env, target_name)) => Binding::import(target_env, target_name),
                            None => return Err(LinkFail::Run(Abrupt::Internal(format!("module binding '{name}' not found")))),
                        },
                        Ok(Some(Resolution::Namespace(module))) => self.namespace_binding(module)?,
                        Ok(None) => {
                            let message = format!("the module '{}' has no export named '{n}'", self.specifier(m, imp.request));
                            return Err(self.fail_at(m, imp.local.span.start, ErrorType::Syntax, &message));
                        }
                        Err(Ambiguous) => {
                            let message = format!("the export '{n}' of the module '{}' is ambiguous", self.specifier(m, imp.request));
                            return Err(self.fail_at(m, imp.local.span.start, ErrorType::Syntax, &message));
                        }
                    },
                };
                if matches!(&imp.import, ImportName::Namespace) {
                    self.interp.use_fuel().map_err(LinkFail::Run)?;
                    let Some(Env::Decl { bindings, .. }) = self.interp.heap.env_mut(env) else {
                        return Err(LinkFail::Run(Abrupt::Internal("namespace import environment not found".into())));
                    };
                    let Some(slot) = bindings.get_mut(&local) else {
                        return Err(LinkFail::Run(Abrupt::Internal("namespace import binding not found".into())));
                    };
                    *slot = binding;
                } else {
                    self.interp.add_import(env, local, binding).map_err(LinkFail::Run)?;
                }
            }
        }
        Ok(())
    }

    /// An immutable binding of module `m`'s namespace object.
    fn namespace_binding(&mut self, m: usize) -> Result<Binding, LinkFail> {
        let ns = self.namespace(m)?;
        Ok(Binding { constant: true, lexical: true, ..Binding::new(Value::Object(ns), false, false) })
    }

    /// The modules in the order they run (section 15.2.1.16.5, ModuleEvaluation): each module
    /// after the modules it imports, in the order it imports them, and each once; a module an
    /// import cycle reaches again while it waits is not waited for.
    fn evaluation_order(&self) -> Vec<(Rc<Code>, EnvId, EnvId)> {
        let mut state = vec![0u8; self.records.len()];
        let mut order = Vec::new();
        let mut stack = vec![(0usize, 0usize)];
        if let Some(s) = state.first_mut() {
            *s = 1;
        }
        while let Some(top) = stack.last_mut() {
            let m = top.0;
            let requested = self.records.get(m).map_or(&[][..], |r| r.requested.as_slice());
            if let Some(&t) = requested.get(top.1) {
                top.1 += 1;
                if let Some(s @ 0) = state.get_mut(t) {
                    *s = 1;
                    stack.push((t, 0));
                }
                continue;
            }
            stack.pop();
            if let Some(r) = self.records.get(m)
                && let Some((lex, env)) = r.envs
            {
                order.push((r.code.clone(), lex, env));
            }
        }
        order
    }
}
