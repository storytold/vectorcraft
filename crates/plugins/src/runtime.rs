//! The wasmi host: loading, instantiating and calling a plug-in under resource limits.

use wasmi::{
    Config, CustomFuelCosts, EnforcedLimits, Engine, ExternType, Instance, Memory, Module, Store, StoreLimitsBuilder, TypedFunc, TypedResumableCall,
    Val, ValType, WasmParams, WasmResults,
};
use wasmparser::{Validator, WasmFeatures};

use crate::host::{self, Bridge, Deadline, Host, Report, Run};
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

/// Validates `bytes` with only [`WASM_FEATURES`] on, before wasmi reads the module. A module that
/// uses any other proposal is refused with the validator's error, which names the proposal.
fn check_features(bytes: &[u8]) -> Result<()> {
    Validator::new_with_features(WASM_FEATURES).validate_all(bytes).map(|_| ()).map_err(|e| Error::Module(e.to_string()))
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

/// A plug-in's code: a WebAssembly module, or a JavaScript file ([`crate::js`]).
enum Code {
    Wasm { engine: Engine, module: Module },
    Js(crate::js::Script),
}

/// A loaded, validated plug-in. Cheap to share (`Arc`) and safe to run from several threads:
/// every run gets its own store and instance, or for a JavaScript plug-in its own realm.
pub struct Plugin {
    manifest: Manifest,
    code: Code,
    limits: Limits,
    size: usize,
    hash: String,
    source: Option<String>,
    serial: u64,
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
/// give the ABI version as `abi`, in place of the `vc_abi_version` export.
fn section_manifest(module: &Module) -> Result<Option<Manifest>> {
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
    Ok(Some(manifest))
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
        crate::safe_mode::check()?;
        if bytes.len() > limits.max_module_bytes {
            return Err(Error::Module(format!("module is {} bytes (limit {})", bytes.len(), limits.max_module_bytes)));
        }
        if bytes.trim_ascii_start().starts_with(b"{") {
            let (manifest, script) = crate::js::read_bundle(bytes)?;
            let hash = crate::permissions::sha256_hex(bytes);
            let mut p = Plugin { manifest, code: Code::Js(script), limits, size: bytes.len(), hash, source: None, serial: 0 };
            p.heavy_limits();
            return Ok(p);
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
        let section = section_manifest(&module)?;
        let mut p = Plugin {
            manifest: Manifest::placeholder(),
            code: Code::Wasm { engine, module: module.clone() },
            limits,
            size: bytes.len(),
            hash,
            source: None,
            serial: 0,
        };
        let live = match section {
            Some(manifest) => {
                p.manifest = manifest;
                None
            }
            None => {
                let deadline = Deadline::new(p.limits.wall_time_ms);
                // The manifest isn't read yet: every host import is linked, what it reports is
                // dropped and there is no storage to reach.
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
                p.manifest = Manifest::parse(&text)?;
                Some(live)
            }
        };
        if p.manifest.runtime != Runtime::Wasm {
            return Err(Error::Manifest("a WebAssembly module's manifest leaves `runtime` out: \"js\" is for a JavaScript plug-in".into()));
        }
        host::check_granted(&p.manifest, &imports)?;
        p.heavy_limits();
        // The other exports must exist with the right types.
        match live {
            Some(live) => {
                live.func::<i32, i32>("vc_alloc")?;
                live.func::<RunArgs, i64>("vc_run")?;
            }
            None => {
                check_exports(&module, p.limits.max_memory_bytes)?;
                check_tables(bytes)?;
            }
        }
        Ok(p)
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
        self.call_with(live, f, params, fuel, deadline, None)
    }

    /// [`Plugin::call`], making each command call a `vc_call` pauses it for with `bridge`
    /// ([`host::Run::pending`]) and resuming it with the code. Each call costs [`host::CALL_FUEL`]
    /// instructions and one for every 64 bytes of its arguments and result; the deadline and the
    /// caller's cancellation are checked before and after it.
    fn call_with<P: WasmParams, R: WasmResults>(
        &self,
        live: &mut Live,
        f: &TypedFunc<P, R>,
        params: P,
        fuel: u64,
        deadline: &Deadline,
        mut bridge: Option<&mut Bridge<'_>>,
    ) -> Result<R> {
        let mut left = fuel;
        let give = left.min(FUEL_SLICE);
        left -= give;
        live.store.set_fuel(give).map_err(wasm_err)?;
        let mut call = slice(live, &mut left, |store| f.call_resumable(store, params))?;
        loop {
            match call {
                TypedResumableCall::Finished(r) => return Ok(r),
                TypedResumableCall::HostTrap(inv) => {
                    if inv.host_error().downcast_ref::<host::StorageStop>().is_some() {
                        return Err(stop_reason(&mut live.store));
                    }
                    let pending = live.store.data_mut().run.pending.take();
                    let (Some((id, args, size)), Some(bridge)) = (pending, bridge.as_deref_mut()) else {
                        return Err(Error::Trap("unexpected host trap".into()));
                    };
                    charge(live, &mut left, host::CALL_FUEL.saturating_add(size as u64 / 64))?;
                    deadline.check()?;
                    vectorcraft_doc::cancel::check().map_err(|_| Error::Cancelled)?;
                    let answer = bridge(&id, &args);
                    let code = live.store.data_mut().run.call_answered(&id, answer);
                    let returned = live.store.data().run.call_result.as_ref().map_or(0, Vec::len);
                    charge(live, &mut left, returned as u64 / 64)?;
                    deadline.check()?;
                    vectorcraft_doc::cancel::check().map_err(|_| Error::Cancelled)?;
                    call = slice(live, &mut left, |store| inv.resume(store, &[Val::I32(code)]))?;
                }
                TypedResumableCall::OutOfFuel(inv) => {
                    let need = inv.required_fuel();
                    let held = live.store.get_fuel().map_err(wasm_err)?;
                    let total = held.saturating_add(left);
                    if need > total {
                        return Err(Error::Limit("instruction budget".into()));
                    }
                    deadline.check()?;
                    vectorcraft_doc::cancel::check().map_err(|_| Error::Cancelled)?;
                    let fill = total.min(FUEL_SLICE.max(need)).max(held);
                    left -= fill - held;
                    live.store.set_fuel(fill).map_err(wasm_err)?;
                    call = slice(live, &mut left, |store| inv.resume(store))?;
                }
            }
        }
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

    fn run_guarded(&self, input: &[u8], params: &[u8], limits: &Limits, bridge: Option<&mut Bridge<'_>>) -> (Result<Vec<u8>>, Report) {
        let mut report = Report::default();
        if let Err(e) = crate::safe_mode::check() {
            return (Err(e), report);
        }
        // wasmi reports misbehaving modules as errors; a panic would be a bug in it, and must
        // still not take the document down (native builds; wasm aborts on panic anyway).
        let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.run_inner(input, params, limits, &mut report, bridge)));
        (run.map_err(|_| Error::Trap("the runtime panicked".into())).and_then(|r| r), report)
    }

    fn run_inner(&self, input: &[u8], params: &[u8], limits: &Limits, report: &mut Report, bridge: Option<&mut Bridge<'_>>) -> Result<Vec<u8>> {
        let total = input.len().checked_add(params.len()).filter(|b| *b <= i32::MAX as usize && *b <= self.limits.max_memory_bytes / 2);
        total.ok_or_else(|| Error::Limit(SELECTION_TOO_LARGE.into()))?;
        let deadline = Deadline::new(limits.wall_time_ms);
        let m = &self.manifest;
        let run = Run::new(&m.id, m.kind != Kind::Effect, true, deadline).with_storage(crate::registry::storage(&m.id));
        if let Code::Js(script) = &self.code {
            // Its manifest lists no `commands`, so nothing calls it with a bridge.
            if bridge.is_some() {
                return Err(Error::Abi("a JavaScript plug-in makes no command calls".into()));
            }
            let call = crate::js::Call { input, params, limits, max_memory: self.limits.max_memory_bytes, deadline };
            return crate::js::run(script, m, &call, run, report);
        }
        let mut live = self.instantiate(run, &m.permissions(), report)?;
        // Calls can be made from `vc_run` on, not from the start function.
        if bridge.is_some() {
            live.store.data_mut().run.allow_calls(m.command_allowlist.clone());
        }
        let out = self.run_live(&mut live, input, params, limits, &deadline, bridge);
        let state = &mut live.store.data_mut().run;
        *report = std::mem::take(&mut state.report);
        // A failed call fails the run, whatever the plug-in did after it, unless the run was
        // canceled.
        match (state.call_failure.take(), out) {
            (_, Err(Error::Cancelled)) => Err(Error::Cancelled),
            (Some(e), _) => Err(e),
            (None, out) => out,
        }
    }

    /// Hands `input` and `params` to the instance `live` and runs `vc_run` → the output JSON bytes.
    fn run_live(
        &self,
        live: &mut Live,
        input: &[u8],
        params: &[u8],
        limits: &Limits,
        deadline: &Deadline,
        bridge: Option<&mut Bridge<'_>>,
    ) -> Result<Vec<u8>> {
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
        let fuel = limits.fuel_base.saturating_add((input.len() as u64).saturating_mul(limits.fuel_per_byte));
        let packed = self.call_with(live, &run, (iptr as i32, input.len() as i32, pptr as i32, params.len() as i32), fuel, deadline, bridge)?;
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
