//! The wasmi host: loading, instantiating and calling a plug-in under resource limits.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use wasmi::{
    Config, CustomFuelCosts, EnforcedLimits, Engine, ExternType, Instance, Memory, Module, Store, StoreLimitsBuilder, TypedFunc, TypedResumableCall,
    TypedResumableCallHostTrap, Val, ValType, WasmParams, WasmResults,
};
use wasmparser::{CompositeInnerType, Parser, Payload, Validator, WasmFeatures};

use crate::host::{self, Bridge, CallFailure, Deadline, Host, Import, Report, Run};
use crate::manifest::{MANIFEST_SECTION, MAX_MANIFEST_BYTES, Manifest, Runtime};
use crate::permissions::Permission;
use crate::{Error, Kind, Result};

/// The plug-in ABI version this host implements (`vc_abi_version` must return it).
pub const ABI_VERSION: i32 = 1;

/// Fuel handed out per slice; between slices the host checks the deadline.
pub const FUEL_SLICE: u64 = 20_000_000;

/// wasmi's fuel costs, but compiling costs nothing. wasmi compiles a function when it is first
/// called, and charges the run that calls it by the function's size. When the slice a run was in
/// ran out meanwhile, wasmi ended the run with an error, not an out-of-fuel pause the host resumes:
/// a first run failed with `Limit("instruction budget")` and most of its budget left, as where the
/// slices fell decided. So a run pays for its own instructions only, as many on a first run as on
/// the next. Copying keeps wasmi's default cost, a unit for every 64 bytes.
const FUEL_COSTS: CustomFuelCosts = CustomFuelCosts { bytes_copied_per_fuel: 64, fuel_per_bytes_translated: 0, fuel_per_bytes_validated: 0 };

/// Bytes a bulk memory instruction copies or fills for each unit of fuel.
pub const BYTES_COPIED_PER_FUEL: u32 = FUEL_COSTS.bytes_copied_per_fuel;

/// The WebAssembly proposals a plug-in may use, as `docs/plugins.md` lists them. `GC_TYPES` lets
/// reference types use `funcref` and `externref`; the garbage collection proposal stays off. Every
/// proposal not listed here is off, whatever wasmi's defaults and crate features turn on.
const WASM_FEATURES: WasmFeatures = WasmFeatures::MUTABLE_GLOBAL
    .union(WasmFeatures::SIGN_EXTENSION)
    .union(WasmFeatures::SATURATING_FLOAT_TO_INT)
    .union(WasmFeatures::MULTI_VALUE)
    .union(WasmFeatures::BULK_MEMORY)
    .union(WasmFeatures::REFERENCE_TYPES)
    .union(WasmFeatures::GC_TYPES)
    .union(WasmFeatures::TAIL_CALL)
    .union(WasmFeatures::EXTENDED_CONST)
    .union(WasmFeatures::FLOATS);

/// The most parameters or results a function type may have: the limit `EnforcedLimits::strict()`
/// sets in wasmi 2.0.0.
const MAX_FUNC_TYPE_VALUES: usize = 32;

/// Validates `bytes` with only [`WASM_FEATURES`] on, before wasmi reads the module. A module that
/// uses any other proposal is refused with the validator's error, which names the proposal. A
/// function type with more than [`MAX_FUNC_TYPE_VALUES`] parameters or results is refused before
/// any function body is validated.
fn check_features(bytes: &[u8]) -> Result<()> {
    check_func_types(bytes)?;
    Validator::new_with_features(WASM_FEATURES).validate_all(bytes).map(|_| ()).map_err(|e| Error::Module(e.to_string()))
}

/// Refuses a function type in the type section with more than [`MAX_FUNC_TYPE_VALUES`] parameters
/// or results, with wasmi's message. Reading stops at the code section. A section that doesn't
/// parse is left to the validator, which reports it.
fn check_func_types(bytes: &[u8]) -> Result<()> {
    for payload in Parser::new(0).parse_all(bytes) {
        let types = match payload {
            Ok(Payload::TypeSection(types)) => types,
            Ok(Payload::CodeSectionStart { .. }) | Err(_) => return Ok(()),
            Ok(_) => continue,
        };
        for group in types {
            let Ok(group) = group else { return Ok(()) };
            for ty in group.types() {
                if let CompositeInnerType::Func(f) = &ty.composite_type.inner {
                    if f.params().len() > MAX_FUNC_TYPE_VALUES {
                        return Err(Error::Module(format!("a function type exceeds the limit of {MAX_FUNC_TYPE_VALUES} parameters")));
                    }
                    if f.results().len() > MAX_FUNC_TYPE_VALUES {
                        return Err(Error::Module(format!("a function type exceeds the limit of {MAX_FUNC_TYPE_VALUES} results")));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Sets each WebAssembly proposal that wasmi's `Config` has a setter for in this build on or off,
/// to match [`WASM_FEATURES`].
fn set_features(config: &mut Config) -> &mut Config {
    config
        .wasm_mutable_global(true)
        .wasm_sign_extension(true)
        .wasm_saturating_float_to_int(true)
        .wasm_multi_value(true)
        .wasm_bulk_memory(true)
        .wasm_reference_types(true)
        .wasm_tail_call(true)
        .wasm_extended_const(true)
        .floats(true)
        .wasm_multi_memory(false)
        .wasm_custom_page_sizes(false)
        .wasm_wide_arithmetic(false)
}

/// Instructions a heavy filter's run may use besides [`Limits::fuel_per_byte`] for each byte of
/// input ([`crate::Budget::Heavy`]): 20 times a light one's. Its output's size can't add to it:
/// the host learns that only once the run has ended.
pub const HEAVY_FUEL: u64 = 1_000_000_000;

/// Wall-clock budget of a heavy filter's run, in milliseconds.
pub const HEAVY_WALL_TIME_MS: u64 = 300_000;

/// Instructions a filter's run may use besides [`Limits::fuel_per_byte`] for each byte of input
/// when Preferences › Plug-ins & Scratch Disks › Plug-in Instruction Budget is Large
/// ([`Plugin::large_limits`]): 5 times the default.
pub const LARGE_FUEL: u64 = 250_000_000;

/// The limit a run reports when its input and parameters together are past half its memory.
pub(crate) const SELECTION_TOO_LARGE: &str = "memory budget (the selection is too large to hand over)";

/// A heavy run's budget as people read it: (minutes it may take, times the instructions a light
/// run gets).
pub fn heavy_terms() -> (u64, u64) {
    (HEAVY_WALL_TIME_MS / 60_000, HEAVY_FUEL / Limits::default().fuel_base.max(1))
}

/// Resource limits for running plug-ins.
#[derive(Clone, Debug, PartialEq)]
pub struct Limits {
    /// Largest module accepted.
    pub max_module_bytes: usize,
    /// Cap on a plug-in's linear memory. The input and the parameters may use half of it.
    pub max_memory_bytes: usize,
    /// Largest output JSON read back.
    pub max_output_bytes: usize,
    /// Instructions (fuel) any call may use, plus [`Limits::fuel_per_byte`] per byte of input for
    /// `vc_run`.
    pub fuel_base: u64,
    pub fuel_per_byte: u64,
    /// Deepest call nesting.
    pub max_recursion_depth: usize,
    /// Wall-clock budget for one run, in milliseconds. The web build measures it with the page's
    /// `performance.now()`.
    pub wall_time_ms: u64,
    /// The most a heavy filter ([`crate::Budget::Heavy`]) is given instead of `fuel_base` and
    /// `wall_time_ms`: what the host allows, which the module's manifest can ask for but never
    /// raise. Set them to `fuel_base` and `wall_time_ms` to give heavy filters nothing more.
    pub heavy_fuel: u64,
    pub heavy_wall_time_ms: u64,
    /// What one export or import is given instead of `fuel_base` and `wall_time_ms`
    /// ([`crate::export::limits`]).
    pub export_fuel: u64,
    pub export_wall_time_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_module_bytes: 32 << 20,
            max_memory_bytes: 512 << 20,
            max_output_bytes: 64 << 20,
            fuel_base: 50_000_000,
            fuel_per_byte: 4_000,
            max_recursion_depth: 1024,
            wall_time_ms: 60_000,
            heavy_fuel: HEAVY_FUEL,
            heavy_wall_time_ms: HEAVY_WALL_TIME_MS,
            export_fuel: crate::export::FUEL,
            export_wall_time_ms: crate::export::WALL_TIME_MS,
        }
    }
}

/// Do the plug-ins loaded from now on keep their module's bytes ([`Plugin::module_bytes`])?
static KEEP_MODULES: AtomicBool = AtomicBool::new(false);

/// From now on, each plug-in loaded keeps its module's bytes ([`Plugin::module_bytes`]) when `on`.
/// The web build turns it on: it runs filters in a worker, which it hands the module to.
pub fn keep_modules(on: bool) {
    KEEP_MODULES.store(on, Ordering::Relaxed);
}

/// A plug-in's code: a WebAssembly module, or a JavaScript file ([`crate::js`]).
enum Code {
    Wasm { engine: Engine, module: Module },
    Js(crate::js::Script),
}

/// A loaded, validated plug-in. Cheap to share (`Arc`) and safe to run from several threads:
/// every run gets its own store and instance, or for a JavaScript plug-in its own realm.
pub struct Plugin {
    manifest: Manifest,
    /// The manifest JSON as `vc_manifest` returned it, or as the module's [`MANIFEST_SECTION`]
    /// custom section holds it. Empty for a JavaScript plug-in.
    manifest_text: Vec<u8>,
    code: Code,
    /// The module's bytes, kept while [`keep_modules`] is on.
    bytes: Option<Arc<[u8]>>,
    limits: Limits,
    size: usize,
    hash: String,
    source: Option<String>,
    serial: u64,
}

/// Where a run that [`Plugin::begin_run`] started is: waiting for the answer to a command call it
/// made with `vc_call`, or ended.
#[derive(Debug)]
pub enum Step {
    /// It called command `command` with `args`, and waits for [`Plugin::resume_run`]'s answer.
    Call { command: String, args: serde_json::Value },
    /// It ended: its output JSON, or why it has none, and what it reported through the host imports.
    Done(Result<Vec<u8>>, Report),
}

/// A run that [`Plugin::begin_run`] started: what it needs to go on after a command call.
#[derive(Default)]
pub struct RunJob {
    paused: Option<Paused>,
}

impl RunJob {
    /// Does it wait for a command call's answer?
    pub fn waiting(&self) -> bool {
        self.paused.is_some()
    }
}

/// A run paused at a `vc_call` of command `command`.
struct Paused {
    live: Live,
    left: u64,
    deadline: Deadline,
    limits: Limits,
    command: String,
    call: Box<TypedResumableCallHostTrap<i64>>,
}

/// What a call does next once the store's fuel pauses are dealt with: it finished, or it waits for
/// the answer to a command call (the command, its arguments and the paused call).
enum Driven<R> {
    Finished(R),
    Call(String, serde_json::Value, Box<TypedResumableCallHostTrap<R>>),
}

/// Runs `f`, catching a panic as [`Error::Trap`]: wasmi reports misbehaving modules as errors, so a
/// panic is a bug in it, and must still not take the document down (native builds; wasm aborts on
/// panic anyway).
fn guarded(f: impl FnOnce() -> (Option<Paused>, Step)) -> (Option<Paused>, Step) {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .unwrap_or_else(|_| (None, Step::Done(Err(Error::Trap("the runtime panicked".into())), Report::default())))
}

/// Is export `name` of `module` a function with these `params` and `results`?
fn exports_func(module: &Module, name: &str, params: &[ValType], results: &[ValType]) -> Result<()> {
    let e = match module.get_export(name) {
        Some(ExternType::Func(f)) if f.params() == params && f.results() == results => return Ok(()),
        Some(_) => "it has another type",
        None => "it is missing",
    };
    Err(Error::Abi(format!("export `{name}`: {e}")))
}

impl std::fmt::Debug for Plugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Plugin").field("manifest", &self.manifest).field("size", &self.size).field("source", &self.source).finish()
    }
}

/// One live instance: a store with its own memory limit and the run's state, and the module's
/// exports.
struct Live {
    store: Store<Host>,
    instance: Instance,
    memory: Memory,
}

impl Live {
    fn func<P: WasmParams, R: WasmResults>(&self, name: &str) -> Result<TypedFunc<P, R>> {
        self.instance.get_typed_func::<P, R>(&self.store, name).map_err(|e| Error::Abi(format!("export `{name}`: {e}")))
    }

    /// The `len << 32 | ptr` block a plug-in returned, copied out (at most `max` bytes).
    fn read_packed(&self, packed: i64, what: &str, max: usize) -> Result<Vec<u8>> {
        let packed = packed as u64;
        let (ptr, len) = ((packed & 0xffff_ffff) as usize, (packed >> 32) as usize);
        if len > max {
            return Err(Error::Limit(format!("output size ({what} is {len} bytes, limit {max})")));
        }
        let bytes = self.memory.data(&self.store).get(ptr..ptr.saturating_add(len));
        bytes.map(<[u8]>::to_vec).ok_or_else(|| Error::Abi(format!("{what} points outside memory")))
    }
}

/// Takes `cost` instructions from a run: from the budget still to hand out (`left`), then from what
/// the store holds. A run that can't pay ends with its instruction budget.
fn charge(live: &mut Live, left: &mut u64, cost: u64) -> Result<()> {
    let held = live.store.get_fuel().map_err(wasm_err)?;
    if held.saturating_add(*left) < cost {
        return Err(Error::Limit("instruction budget".into()));
    }
    let from_left = cost.min(*left);
    *left -= from_left;
    live.store.set_fuel(held - (cost - from_left)).map_err(wasm_err)
}

/// Resumes a run's call with `step`: the store's storage calls may also pay from `left`
/// ([`host::Run::fuel_left`]), and what they took from it comes off `left` when the call pauses →
/// the pause, or the error the call ended with ([`run_err`]).
fn slice<T>(live: &mut Live, left: &mut u64, step: impl FnOnce(&mut Store<Host>) -> std::result::Result<T, wasmi::Error>) -> Result<T> {
    live.store.data_mut().run.fuel_left = *left;
    let paused = step(&mut live.store);
    *left = std::mem::take(&mut live.store.data_mut().run.fuel_left);
    paused.map_err(|e| run_err(&mut live.store, e))
}

/// Why a storage call ended a run of `store` ([`host::Run::storage_stop`]).
fn stop_reason(store: &mut Store<Host>) -> Error {
    store.data_mut().run.storage_stop.take().unwrap_or_else(|| Error::Limit("instruction budget".into()))
}

/// The error `e` that a run of `store` ended with: a storage call's reason ([`host::StorageStop`])
/// or [`wasm_err`]'s.
fn run_err(store: &mut Store<Host>, e: wasmi::Error) -> Error {
    if e.downcast_ref::<host::StorageStop>().is_some() { stop_reason(store) } else { wasm_err(e) }
}

fn wasm_err(e: wasmi::Error) -> Error {
    if e.as_trap_code() == Some(wasmi::TrapCode::OutOfFuel) { Error::Limit("instruction budget".into()) } else { Error::Trap(e.to_string()) }
}

/// `vc_run(input_ptr, input_len, params_ptr, params_len) -> len << 32 | ptr` (negative: error).
type RunArgs = (i32, i32, i32, i32);

/// The manifest in the module's [`MANIFEST_SECTION`] custom section, or `None` when it has none.
/// A module with more than one is refused, and so is a section longer than [`MAX_MANIFEST_BYTES`],
/// which [`Manifest::parse`] checks before it decodes the bytes. The manifest in the section must
/// give the ABI version as `abi`, in place of the `vc_abi_version` export. Also gives the section's
/// bytes, the manifest JSON.
fn section_manifest(module: &Module) -> Result<Option<(Manifest, Vec<u8>)>> {
    let mut sections = module.custom_sections().filter(|s| s.name() == MANIFEST_SECTION);
    let Some(section) = sections.next() else {
        return Ok(None);
    };
    if sections.next().is_some() {
        return Err(Error::Manifest(format!("the module has more than one `{MANIFEST_SECTION}` custom section")));
    }
    let manifest = Manifest::parse(section.data())?;
    // `Manifest::parse` has checked that the section is UTF-8 JSON of at most `MAX_MANIFEST_BYTES`.
    let abi = serde_json::from_slice::<serde_json::Value>(section.data()).ok().and_then(|v| v.get("abi").and_then(serde_json::Value::as_i64));
    if abi != Some(i64::from(ABI_VERSION)) {
        return Err(Error::Abi(format!(
            "the manifest in the `{MANIFEST_SECTION}` custom section must give `abi` as {ABI_VERSION}, the ABI version this host implements"
        )));
    }
    Ok(Some((manifest, section.data().to_vec())))
}

/// Most tables a module may define: the store an instance runs in holds no more.
const MAX_TABLES: u64 = 4;
/// Most elements a table may hold: the store an instance runs in holds no more.
const MAX_TABLE_ELEMENTS: u64 = 100_000;

/// The number of tables the module in `bytes` defines and the largest initial size among them,
/// from its table section, or `None` when its sections can't be read. `bytes` is a module
/// [`check_features`] has validated.
fn table_sizes(bytes: &[u8]) -> Option<(u64, u64)> {
    for payload in wasmparser::Parser::new(0).parse_all(bytes) {
        if let wasmparser::Payload::TableSection(reader) = payload.ok()? {
            let count = u64::from(reader.count());
            let mut largest = 0;
            for table in reader {
                largest = largest.max(table.ok()?.ty.initial);
            }
            return Some((count, largest));
        }
    }
    Some((0, 0))
}

/// Checks the tables the module in `bytes` defines against the store's limits ([`MAX_TABLES`]
/// tables of at most [`MAX_TABLE_ELEMENTS`] elements), which instantiating it checks. The host
/// offers no tables to import, so the module defines every table it has.
fn check_tables(bytes: &[u8]) -> Result<()> {
    let Some((count, largest)) = table_sizes(bytes) else {
        return Err(Error::Module("the module's table section could not be read".into()));
    };
    if count > MAX_TABLES || largest > MAX_TABLE_ELEMENTS {
        return Err(Error::Limit(format!(
            "table budget (the module defines {count} tables, the largest with {largest} elements; limit {MAX_TABLES} tables of {MAX_TABLE_ELEMENTS} elements)"
        )));
    }
    Ok(())
}

/// Checks the exports a run needs without instantiating `module`: its memory as `memory`, whose
/// initial size fits in `max_memory_bytes`, and `vc_alloc` and `vc_run` with the ABI's types.
fn check_exports(module: &Module, max_memory_bytes: usize) -> Result<()> {
    use ValType::{I32, I64};
    let export = |name: &str| module.exports().find(|e| e.name() == name).map(|e| e.ty().clone());
    let Some(ExternType::Memory(memory)) = export("memory") else {
        return Err(Error::Abi("the module must export its linear memory as `memory`".into()));
    };
    // Pages are 64 KiB: the engine's configuration leaves custom page sizes off.
    let initial = memory.minimum().saturating_mul(1 << 16);
    if initial > max_memory_bytes as u64 {
        return Err(Error::Limit(format!("memory budget (the module's initial memory is {initial} bytes, limit {max_memory_bytes})")));
    }
    let wanted: [(&str, &[ValType], &[ValType], &str); 2] =
        [("vc_alloc", &[I32], &[I32], "(param i32) (result i32)"), ("vc_run", &[I32, I32, I32, I32], &[I64], "(param i32 i32 i32 i32) (result i64)")];
    for (name, params, results, signature) in wanted {
        if !matches!(export(name), Some(ExternType::Func(f)) if f.params() == params && f.results() == results) {
            return Err(Error::Abi(format!("the module must export `{name}` with the type {signature}")));
        }
    }
    Ok(())
}

impl Plugin {
    /// Validates and loads a module: reads the manifest, checks that each host import it asks for
    /// is one the host offers ([`crate::host::IMPORTS`]) and its manifest allows, and checks the
    /// exports. A module that carries its manifest in a [`MANIFEST_SECTION`] custom section is
    /// checked without being instantiated, so none of its code runs; one without that section is
    /// instantiated to call `vc_abi_version` and `vc_manifest`. Bytes that hold a JSON object with
    /// `"runtime": "js"` are a JavaScript plug-in ([`crate::js`]): its manifest is checked and its
    /// script parsed, and none of it runs. In Safe Mode nothing is loaded ([`Error::SafeMode`]).
    pub fn load(bytes: &[u8], limits: Limits) -> Result<Plugin> {
        let (mut p, imports) = Self::compile(bytes, limits)?;
        if p.read_section(bytes, &imports)? {
            return Ok(p);
        }
        let deadline = Deadline::new(p.limits.wall_time_ms);
        // The manifest isn't read yet: every host import is linked, what it reports is dropped and
        // there is no storage to reach.
        let mut live = p.instantiate(Run::new("", false, false, deadline), &crate::permissions::ALL, &mut Report::default())?;
        let version: TypedFunc<(), i32> = live.func("vc_abi_version")?;
        let v = p.call(&mut live, &version, (), p.limits.fuel_base, &deadline)?;
        if v != ABI_VERSION {
            return Err(Error::Abi(format!("vc_abi_version returned {v}; this host implements version {ABI_VERSION}")));
        }
        let manifest: TypedFunc<(), i64> = live.func("vc_manifest")?;
        let packed = p.call(&mut live, &manifest, (), p.limits.fuel_base, &deadline)?;
        let text = live.read_packed(packed, "vc_manifest", MAX_MANIFEST_BYTES).map_err(|e| match e {
            Error::Limit(_) => Error::Manifest(format!("manifest is too long (limit {MAX_MANIFEST_BYTES} bytes)")),
            e => e,
        })?;
        p.read(text, &imports)?;
        // The other exports must exist with the right types.
        live.func::<i32, i32>("vc_alloc")?;
        live.func::<RunArgs, i64>("vc_run")?;
        Ok(p)
    }

    /// [`Plugin::load`] of a module that a load of the same bytes in another place (the web build's
    /// plug-in worker) has checked, with the manifest JSON that load read ([`Plugin::manifest_text`]).
    /// Every check that needs none of the module's code to run is made again; the module's start
    /// function, `vc_abi_version` and `vc_manifest` don't run. A module with a [`MANIFEST_SECTION`]
    /// custom section loads as [`Plugin::load`] loads it, from that section, and `manifest` is unused.
    pub fn load_checked(bytes: &[u8], limits: Limits, manifest: &[u8]) -> Result<Plugin> {
        let (mut p, imports) = Self::compile(bytes, limits)?;
        if p.read_section(bytes, &imports)? {
            return Ok(p);
        }
        if manifest.len() > MAX_MANIFEST_BYTES {
            return Err(Error::Manifest(format!("manifest is too long (limit {MAX_MANIFEST_BYTES} bytes)")));
        }
        // A JavaScript plug-in has no section to read, and `read_section` has loaded it.
        let Code::Wasm { module, .. } = &p.code else { return Ok(p) };
        exports_func(module, "vc_abi_version", &[], &[ValType::I32])?;
        exports_func(module, "vc_manifest", &[], &[ValType::I64])?;
        exports_func(module, "vc_alloc", &[ValType::I32], &[ValType::I32])?;
        exports_func(module, "vc_run", &[ValType::I32; 4], &[ValType::I64])?;
        if !matches!(module.get_export("memory"), Some(ExternType::Memory(_))) {
            return Err(Error::Abi("the module must export its linear memory as `memory`".into()));
        }
        p.read(manifest.to_vec(), &imports)?;
        Ok(p)
    }

    /// Takes manifest JSON `text` as the plug-in's: parses it, checks that it allows each host
    /// import of `imports`, and gives a heavy filter its larger budget.
    fn read(&mut self, text: Vec<u8>, imports: &[&Import]) -> Result<()> {
        let manifest = Manifest::parse(&text)?;
        self.take(manifest, text, imports)
    }

    /// Reads the manifest from the module's [`MANIFEST_SECTION`] custom section
    /// ([`section_manifest`]) and checks the exports and tables a run needs, running none of the
    /// module's code; `bytes` is the module → false, with nothing read, when it has no such section.
    /// A JavaScript plug-in, whose manifest [`Plugin::compile`] has read, gives true.
    fn read_section(&mut self, bytes: &[u8], imports: &[&Import]) -> Result<bool> {
        let Code::Wasm { module, .. } = &self.code else {
            return Ok(true);
        };
        let module = module.clone();
        let Some((manifest, text)) = section_manifest(&module)? else {
            return Ok(false);
        };
        self.take(manifest, text, imports)?;
        check_exports(&module, self.limits.max_memory_bytes)?;
        check_tables(bytes)?;
        Ok(true)
    }

    /// Takes `manifest`, parsed from JSON `text`, as the plug-in's: checks that it allows each host
    /// import of `imports`, and gives a heavy filter its larger budget. A WebAssembly module's
    /// manifest must leave `runtime` out.
    fn take(&mut self, manifest: Manifest, text: Vec<u8>, imports: &[&Import]) -> Result<()> {
        if manifest.runtime != Runtime::Wasm {
            return Err(Error::Manifest("a WebAssembly module's manifest leaves `runtime` out: \"js\" is for a JavaScript plug-in".into()));
        }
        self.manifest = manifest;
        host::check_granted(&self.manifest, imports)?;
        self.heavy_limits();
        self.manifest_text = text;
        Ok(())
    }

    /// The checks of a module that run none of its code: its size, header, WebAssembly proposals and
    /// host imports, and wasmi's own validation → the plug-in, with its manifest still to be read,
    /// and the host imports it asks for. In Safe Mode nothing is loaded ([`Error::SafeMode`]).
    fn compile(bytes: &[u8], limits: Limits) -> Result<(Plugin, Vec<&'static Import>)> {
        crate::safe_mode::check()?;
        if bytes.len() > limits.max_module_bytes {
            return Err(Error::Module(format!("module is {} bytes (limit {})", bytes.len(), limits.max_module_bytes)));
        }
        let kept = || KEEP_MODULES.load(Ordering::Relaxed).then(|| Arc::from(bytes));
        // A JavaScript plug-in's manifest is read here; `read_section` then has nothing to read.
        if bytes.trim_ascii_start().starts_with(b"{") {
            let (manifest, script) = crate::js::read_bundle(bytes)?;
            let hash = crate::permissions::sha256_hex(bytes);
            let mut p = Plugin {
                manifest,
                manifest_text: Vec::new(),
                code: Code::Js(script),
                bytes: kept(),
                limits,
                size: bytes.len(),
                hash,
                source: None,
                serial: 0,
            };
            p.heavy_limits();
            return Ok((p, Vec::new()));
        }
        if !bytes.starts_with(b"\0asm") {
            return Err(Error::Module("not a WebAssembly binary (missing \\0asm header)".into()));
        }
        check_features(bytes)?;
        let mut config = Config::default();
        set_features(&mut config)
            .consume_fuel(true)
            .fuel_cost(FUEL_COSTS)
            .enforced_limits(EnforcedLimits::strict())
            .set_max_recursion_depth(limits.max_recursion_depth.max(16));
        let engine = Engine::new(&config);
        let module = Module::new(&engine, bytes).map_err(|e| Error::Module(e.to_string()))?;
        let imports = host::check_imports(&module)?;
        let hash = crate::permissions::sha256_hex(bytes);
        let p = Plugin {
            manifest: Manifest::placeholder(),
            manifest_text: Vec::new(),
            code: Code::Wasm { engine, module },
            bytes: kept(),
            limits,
            size: bytes.len(),
            hash,
            source: None,
            serial: 0,
        };
        Ok((p, imports))
    }

    /// A heavy filter's runs get the larger budget the host allows for one, which the host lets the
    /// user allow or refuse before installing it.
    fn heavy_limits(&mut self) {
        if self.manifest.budget == crate::Budget::Heavy {
            self.limits.fuel_base = self.limits.fuel_base.max(self.limits.heavy_fuel);
            self.limits.wall_time_ms = self.limits.wall_time_ms.max(self.limits.heavy_wall_time_ms);
        }
    }

    /// What runs its code ([`Manifest::runtime`]).
    pub fn runtime(&self) -> Runtime {
        self.manifest.runtime
    }

    /// A JavaScript plug-in's script file name; `None` for a WebAssembly module.
    pub fn script(&self) -> Option<&str> {
        match &self.code {
            Code::Js(script) => Some(&script.name),
            Code::Wasm { .. } => None,
        }
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    /// The manifest JSON as the module's `vc_manifest` returned it or its manifest section holds
    /// it, for [`Plugin::load_checked`].
    pub fn manifest_text(&self) -> &[u8] {
        &self.manifest_text
    }
    /// The module's bytes, when it was loaded while [`keep_modules`] was on.
    pub fn module_bytes(&self) -> Option<&Arc<[u8]>> {
        self.bytes.as_ref()
    }
    pub fn id(&self) -> &str {
        &self.manifest.id
    }
    pub fn limits(&self) -> &Limits {
        &self.limits
    }
    /// Its limits with the Large instruction budget: at least [`LARGE_FUEL`] instructions besides
    /// those for each byte of input, and every other limit, the time budget included, as it is. A
    /// heavy filter keeps its larger budget.
    pub fn large_limits(&self) -> Limits {
        Limits { fuel_base: self.limits.fuel_base.max(LARGE_FUEL), ..self.limits.clone() }
    }
    /// Module size in bytes.
    pub fn size(&self) -> usize {
        self.size
    }
    /// The module's SHA-256, in lower-case hexadecimal.
    pub fn hash(&self) -> &str {
        &self.hash
    }
    /// Where the module was loaded from, if it came from a file.
    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }
    /// A number unique to this installation (re-installing a plug-in gives it a new one), so
    /// caches of its results can tell versions apart. 0 until installed.
    pub fn serial(&self) -> u64 {
        self.serial
    }
    pub(crate) fn set_serial(&mut self, serial: u64) {
        self.serial = serial;
    }

    /// A new instance for `run`, with the host imports of the permissions `granted` linked. When the
    /// module's start function fails, what it reported through the host imports goes to `report`.
    fn instantiate(&self, run: Run, granted: &[Permission], report: &mut Report) -> Result<Live> {
        let Code::Wasm { engine, module } = &self.code else { return Err(Error::Module("not a WebAssembly binary (missing \\0asm header)".into())) };
        let limits = StoreLimitsBuilder::new()
            .memory_size(self.limits.max_memory_bytes)
            .memories(1)
            .tables(MAX_TABLES as usize)
            .table_elements(MAX_TABLE_ELEMENTS as usize)
            .instances(1)
            .build();
        let mut store = Store::new(engine, Host { limits, run });
        store.limiter(|h| &mut h.limits);
        // The start function (if any) runs on one slice of fuel: it runs before `call` can check
        // the deadline or a cancel between slices, so a heavy budget would let it run for minutes.
        store.set_fuel(self.limits.fuel_base.min(FUEL_SLICE)).map_err(wasm_err)?;
        let linker = host::linker(engine, granted);
        let instance = match linker.instantiate_and_start(&mut store, module) {
            Ok(instance) => instance,
            Err(e) => {
                *report = std::mem::take(&mut store.data_mut().run.report);
                return Err(run_err(&mut store, e));
            }
        };
        let memory =
            instance.get_memory(&store, "memory").ok_or_else(|| Error::Abi("the module must export its linear memory as `memory`".into()))?;
        Ok(Live { store, instance, memory })
    }

    /// Calls `f` with at most `fuel` instructions, in slices, checking the deadline and the
    /// caller's cancellation ([`vectorcraft_doc::cancel`]) between them.
    ///
    /// wasmi pauses before an instruction that the store's fuel can't pay for, and charges nothing
    /// for it. It charges a `memory.fill` or `memory.copy` for its whole length before writing, so
    /// up to that cost can be left in the store at a pause. Each slice tops the store up to the
    /// slice size, keeping that fuel, and a run ends with the instruction budget only when the
    /// store's fuel and the rest of the budget together can't pay for the next instruction. A
    /// storage call pays for its bytes ([`host::STORAGE_BYTES_PER_FUEL`]) from the store's fuel and
    /// the rest of the budget as it is made, and one they can't pay for ends the run.
    fn call<P: WasmParams, R: WasmResults>(&self, live: &mut Live, f: &TypedFunc<P, R>, params: P, fuel: u64, deadline: &Deadline) -> Result<R> {
        let mut left = fuel;
        let call = Self::first_slice(live, &mut left, |store| f.call_resumable(store, params))?;
        match Self::drive(live, &mut left, deadline, call)? {
            Driven::Finished(r) => Ok(r),
            Driven::Call(..) => Err(Error::Trap("unexpected host trap".into())),
        }
    }

    /// Starts a call with `start`, on the first slice of the `left` instructions it may use.
    fn first_slice<R>(
        live: &mut Live,
        left: &mut u64,
        start: impl FnOnce(&mut Store<Host>) -> std::result::Result<TypedResumableCall<R>, wasmi::Error>,
    ) -> Result<TypedResumableCall<R>> {
        let give = (*left).min(FUEL_SLICE);
        *left -= give;
        live.store.set_fuel(give).map_err(wasm_err)?;
        slice(live, left, start)
    }

    /// Goes on with `call` slice by slice ([`Plugin::call`]) until it finishes or pauses for a
    /// command call that `vc_call` asked for ([`host::Run::pending`]). Each command call costs
    /// [`host::CALL_FUEL`] instructions and one for every 64 bytes of its arguments, and the deadline
    /// and the caller's cancellation are checked before it is made ([`Plugin::answer`]).
    fn drive<R: WasmResults>(live: &mut Live, left: &mut u64, deadline: &Deadline, mut call: TypedResumableCall<R>) -> Result<Driven<R>> {
        loop {
            match call {
                TypedResumableCall::Finished(r) => return Ok(Driven::Finished(r)),
                TypedResumableCall::HostTrap(inv) => {
                    if inv.host_error().downcast_ref::<host::StorageStop>().is_some() {
                        return Err(stop_reason(&mut live.store));
                    }
                    let Some((id, args, size)) = live.store.data_mut().run.pending.take() else {
                        return Err(Error::Trap("unexpected host trap".into()));
                    };
                    charge(live, left, host::CALL_FUEL.saturating_add(size as u64 / 64))?;
                    deadline.check()?;
                    vectorcraft_doc::cancel::check().map_err(|_| Error::Cancelled)?;
                    return Ok(Driven::Call(id, args, Box::new(inv)));
                }
                TypedResumableCall::OutOfFuel(inv) => {
                    let need = inv.required_fuel();
                    let held = live.store.get_fuel().map_err(wasm_err)?;
                    let total = held.saturating_add(*left);
                    if need > total {
                        return Err(Error::Limit("instruction budget".into()));
                    }
                    deadline.check()?;
                    vectorcraft_doc::cancel::check().map_err(|_| Error::Cancelled)?;
                    let fill = total.min(FUEL_SLICE.max(need)).max(held);
                    *left -= fill - held;
                    live.store.set_fuel(fill).map_err(wasm_err)?;
                    call = slice(live, left, |store| inv.resume(store))?;
                }
            }
        }
    }

    /// Resumes `call`, which paused for command call `id`, with the host's `answer`: the code
    /// `vc_call` returns comes from [`host::Run::call_answered`]. The result costs one instruction for
    /// every 64 bytes, and the deadline and the caller's cancellation are checked again first.
    fn answer<R: WasmResults>(
        live: &mut Live,
        left: &mut u64,
        deadline: &Deadline,
        id: &str,
        answer: std::result::Result<serde_json::Value, CallFailure>,
        call: Box<TypedResumableCallHostTrap<R>>,
    ) -> Result<TypedResumableCall<R>> {
        let code = live.store.data_mut().run.call_answered(id, answer);
        let returned = live.store.data().run.call_result.as_ref().map_or(0, Vec::len);
        charge(live, left, returned as u64 / 64)?;
        deadline.check()?;
        vectorcraft_doc::cancel::check().map_err(|_| Error::Cancelled)?;
        slice(live, left, |store| call.resume(store, &[Val::I32(code)]))
    }

    /// Runs `vc_run` on `input` (the objects JSON) with `params` (the parameters JSON) in a fresh
    /// instance and returns the output JSON bytes.
    pub fn run(&self, input: &[u8], params: &[u8]) -> Result<Vec<u8>> {
        self.run_limited(input, params, &self.limits)
    }

    /// [`Plugin::run`] with other time and instruction budgets (live effects get a shorter one). In
    /// Safe Mode nothing runs ([`Error::SafeMode`]), even a plug-in installed before it was on.
    pub fn run_limited(&self, input: &[u8], params: &[u8], limits: &Limits) -> Result<Vec<u8>> {
        self.run_reporting(input, params, limits).0
    }

    /// [`Plugin::run_limited`], and what the run reported through the host imports: its last
    /// progress report and its log lines, which also go to the app's log under its id. Its storage
    /// imports reach the storage of its id ([`crate::registry::storage`]). Inside a
    /// [`vectorcraft_doc::cancel::scope`] its progress goes to the scope's sink (a live effect's
    /// doesn't) and `vc_cancelled` sees the scope's Cancel.
    pub fn run_reporting(&self, input: &[u8], params: &[u8], limits: &Limits) -> (Result<Vec<u8>>, Report) {
        self.run_guarded(input, params, limits, None)
    }

    /// [`Plugin::run_reporting`] for a filter whose manifest's `commandAllowlist` names commands:
    /// each `vc_call` of one of them is made with `bridge`, on the caller's thread, while the run
    /// waits. A failed call fails the run, also when the plug-in goes on and returns an output; the
    /// error names the command.
    pub fn run_calling(&self, input: &[u8], params: &[u8], limits: &Limits, bridge: &mut Bridge<'_>) -> (Result<Vec<u8>>, Report) {
        self.run_guarded(input, params, limits, Some(bridge))
    }

    fn run_guarded(&self, input: &[u8], params: &[u8], limits: &Limits, mut bridge: Option<&mut Bridge<'_>>) -> (Result<Vec<u8>>, Report) {
        let (mut job, mut step) = self.begin_run(input, params, limits, bridge.is_some());
        loop {
            match step {
                Step::Done(out, report) => return (out, report),
                Step::Call { command, args } => {
                    let Some(bridge) = bridge.as_deref_mut() else {
                        return (Err(Error::Trap("unexpected host trap".into())), Report::default());
                    };
                    // A panic making the call fails the run, as one in the runtime does.
                    let Ok(answer) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| bridge(&command, &args))) else {
                        return (Err(Error::Trap("the runtime panicked".into())), Report::default());
                    };
                    step = self.resume_run(&mut job, answer);
                }
            }
        }
    }

    /// Starts [`Plugin::run_reporting`]'s run, which can wait for the answers to its command calls
    /// in between: with `calls`, each `vc_call` of a command its manifest's `commandAllowlist` names
    /// ends the step with [`Step::Call`], and [`Plugin::resume_run`] goes on with the answer. The
    /// run's time budget runs on while it waits. In Safe Mode nothing runs ([`Error::SafeMode`]).
    pub fn begin_run(&self, input: &[u8], params: &[u8], limits: &Limits, calls: bool) -> (RunJob, Step) {
        if let Err(e) = crate::safe_mode::check() {
            return (RunJob::default(), Step::Done(Err(e), Report::default()));
        }
        let (paused, step) = guarded(|| self.begin_inner(input, params, limits, calls));
        (RunJob { paused }, step)
    }

    /// Goes on with `job`, which waits for the answer to a command call ([`Step::Call`]), with that
    /// call's `answer` → where the run is next. A failed call fails the run, also when the plug-in
    /// goes on and returns an output; the error names the command.
    pub fn resume_run(&self, job: &mut RunJob, answer: std::result::Result<serde_json::Value, CallFailure>) -> Step {
        let Some(Paused { mut live, mut left, deadline, limits, command, call }) = job.paused.take() else {
            return Step::Done(Err(Error::Trap("unexpected host trap".into())), Report::default());
        };
        let (paused, step) = guarded(|| {
            let next = Self::answer(&mut live, &mut left, &deadline, &command, answer, call);
            self.advance(live, left, deadline, limits, next)
        });
        job.paused = paused;
        step
    }

    fn begin_inner(&self, input: &[u8], params: &[u8], limits: &Limits, calls: bool) -> (Option<Paused>, Step) {
        let total = input.len().checked_add(params.len()).filter(|b| *b <= i32::MAX as usize && *b <= self.limits.max_memory_bytes / 2);
        if total.is_none() {
            return (None, Step::Done(Err(Error::Limit(SELECTION_TOO_LARGE.into())), Report::default()));
        }
        let deadline = Deadline::new(limits.wall_time_ms);
        let m = &self.manifest;
        let run = Run::new(&m.id, m.kind != Kind::Effect, true, deadline).with_storage(crate::registry::storage(&m.id));
        let mut report = Report::default();
        if let Code::Js(script) = &self.code {
            // Its manifest lists no `commands`, so nothing calls it with a bridge.
            if calls {
                return (None, Step::Done(Err(Error::Abi("a JavaScript plug-in makes no command calls".into())), report));
            }
            let call = crate::js::Call { input, params, limits, max_memory: self.limits.max_memory_bytes, deadline };
            let out = crate::js::run(script, m, &call, run, &mut report);
            return (None, Step::Done(out, report));
        }
        let mut live = match self.instantiate(run, &m.permissions(), &mut report) {
            Ok(live) => live,
            Err(e) => return (None, Step::Done(Err(e), report)),
        };
        // Calls can be made from `vc_run` on, not from the start function.
        if calls {
            live.store.data_mut().run.allow_calls(m.command_allowlist.clone());
        }
        let mut left = 0;
        let call = self.start_live(&mut live, &mut left, input, params, limits, &deadline);
        self.advance(live, left, deadline, limits.clone(), call)
    }

    /// Hands `input` and `params` to the instance `live` and starts `vc_run`, on the first slice of
    /// its instructions; the rest is left in `left`.
    fn start_live(
        &self,
        live: &mut Live,
        left: &mut u64,
        input: &[u8],
        params: &[u8],
        limits: &Limits,
        deadline: &Deadline,
    ) -> Result<TypedResumableCall<i64>> {
        let alloc: TypedFunc<i32, i32> = live.func("vc_alloc")?;
        let run: TypedFunc<RunArgs, i64> = live.func("vc_run")?;
        let place = |live: &mut Live, bytes: &[u8]| -> Result<usize> {
            let ptr = self.call(live, &alloc, bytes.len().max(1) as i32, limits.fuel_base, deadline)? as u32 as usize;
            if ptr == 0 {
                return Err(Error::Failed("vc_alloc returned 0 (out of memory)".into()));
            }
            let outside = || Error::Abi("vc_alloc returned a block outside memory".into());
            let end = ptr.checked_add(bytes.len()).ok_or_else(outside)?;
            live.memory.data_mut(&mut live.store).get_mut(ptr..end).ok_or_else(outside)?.copy_from_slice(bytes);
            Ok(ptr)
        };
        let iptr = place(live, input)?;
        let pptr = place(live, params)?;
        *left = limits.fuel_base.saturating_add((input.len() as u64).saturating_mul(limits.fuel_per_byte));
        let args = (iptr as i32, input.len() as i32, pptr as i32, params.len() as i32);
        Self::first_slice(live, left, |store| run.call_resumable(store, args))
    }

    /// Takes `vc_run`'s `call` on until it pauses for a command call (kept, with the instance, to
    /// go on from) or ends, then → its output JSON bytes or why it has none, with its report.
    fn advance(
        &self,
        mut live: Live,
        mut left: u64,
        deadline: Deadline,
        limits: Limits,
        call: Result<TypedResumableCall<i64>>,
    ) -> (Option<Paused>, Step) {
        let out = match call.and_then(|call| Self::drive(&mut live, &mut left, &deadline, call)) {
            Ok(Driven::Call(command, args, call)) => {
                let step = Step::Call { command: command.clone(), args };
                return (Some(Paused { live, left, deadline, limits, command, call }), step);
            }
            Ok(Driven::Finished(packed)) => Self::output(&live, packed, &limits, &deadline),
            Err(e) => Err(e),
        };
        let state = &mut live.store.data_mut().run;
        let report = std::mem::take(&mut state.report);
        // A failed call fails the run, whatever the plug-in did after it, unless the run was
        // canceled.
        let out = match (state.call_failure.take(), out) {
            (_, Err(Error::Cancelled)) => Err(Error::Cancelled),
            (Some(e), _) => Err(e),
            (None, out) => out,
        };
        (None, Step::Done(out, report))
    }

    /// The output block `vc_run` returned (`packed`) → the output JSON bytes.
    fn output(live: &Live, packed: i64, limits: &Limits, deadline: &Deadline) -> Result<Vec<u8>> {
        // A cancelled run, or one past its time budget, changes nothing, also when the plug-in saw
        // `vc_cancelled` and returned.
        vectorcraft_doc::cancel::check().map_err(|_| Error::Cancelled)?;
        deadline.check()?;
        if packed < 0 {
            return Err(Error::Failed(format!("vc_run returned error code {packed}")));
        }
        live.read_packed(packed, "the output", limits.max_output_bytes)
    }
}
