//! The wasmi host: loading, instantiating and calling a plug-in under resource limits.

use wasmi::{
    Config, EnforcedLimits, Engine, Instance, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc, TypedResumableCall,
    WasmParams, WasmResults,
};

use crate::manifest::{MAX_MANIFEST_BYTES, Manifest};
use crate::{Error, Result};

/// The plug-in ABI version this host implements (`vc_abi_version` must return it).
pub const ABI_VERSION: i32 = 1;

/// Fuel handed out per slice; between slices the host checks the deadline.
const FUEL_SLICE: u64 = 20_000_000;

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
    /// Wall-clock budget for one run, in milliseconds. Native builds only: the web build has no
    /// monotonic clock in `std`, so the fuel budget alone bounds it there.
    pub wall_time_ms: u64,
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
        }
    }
}

/// A loaded, validated plug-in. Cheap to share (`Arc`) and safe to run from several threads:
/// every run gets its own store and instance.
pub struct Plugin {
    manifest: Manifest,
    engine: Engine,
    module: Module,
    limits: Limits,
    size: usize,
    source: Option<String>,
    serial: u64,
}

impl std::fmt::Debug for Plugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Plugin").field("manifest", &self.manifest).field("size", &self.size).field("source", &self.source).finish()
    }
}

/// When a run must stop (native: a wall-clock deadline).
struct Deadline {
    #[cfg(not(target_arch = "wasm32"))]
    at: std::time::Instant,
}

impl Deadline {
    fn new(limits: &Limits) -> Self {
        #[cfg(target_arch = "wasm32")]
        let _ = limits;
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            at: std::time::Instant::now() + std::time::Duration::from_millis(limits.wall_time_ms),
        }
    }
    fn check(&self) -> Result<()> {
        #[cfg(not(target_arch = "wasm32"))]
        if std::time::Instant::now() >= self.at {
            return Err(Error::Limit("time budget".into()));
        }
        Ok(())
    }
}

/// One live instance: a store with its own memory limit, and the module's exports.
struct Live {
    store: Store<StoreLimits>,
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

fn wasm_err(e: wasmi::Error) -> Error {
    if e.as_trap_code() == Some(wasmi::TrapCode::OutOfFuel) { Error::Limit("instruction budget".into()) } else { Error::Trap(e.to_string()) }
}

/// `vc_run(input_ptr, input_len, params_ptr, params_len) -> len << 32 | ptr` (negative: error).
type RunArgs = (i32, i32, i32, i32);

impl Plugin {
    /// Validates and loads a module: checks the ABI version, the exports and reads the manifest.
    pub fn load(bytes: &[u8], limits: Limits) -> Result<Plugin> {
        if bytes.len() > limits.max_module_bytes {
            return Err(Error::Module(format!("module is {} bytes (limit {})", bytes.len(), limits.max_module_bytes)));
        }
        if !bytes.starts_with(b"\0asm") {
            return Err(Error::Module("not a WebAssembly binary (missing \\0asm header)".into()));
        }
        let mut config = Config::default();
        config.consume_fuel(true).enforced_limits(EnforcedLimits::strict()).set_max_recursion_depth(limits.max_recursion_depth.max(16));
        let engine = Engine::new(&config);
        let module = Module::new(&engine, bytes).map_err(|e| Error::Module(e.to_string()))?;
        if let Some(imp) = module.imports().next() {
            return Err(Error::Abi(format!(
                "the module imports `{}.{}`, but plug-ins get no host imports (no WASI, files or network)",
                imp.module(),
                imp.name()
            )));
        }
        let mut p = Plugin { manifest: Manifest::placeholder(), engine, module, limits, size: bytes.len(), source: None, serial: 0 };
        let deadline = Deadline::new(&p.limits);
        let mut live = p.instantiate()?;
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
        // The other exports must exist with the right types.
        live.func::<i32, i32>("vc_alloc")?;
        live.func::<RunArgs, i64>("vc_run")?;
        Ok(p)
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
    /// Module size in bytes.
    pub fn size(&self) -> usize {
        self.size
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

    fn instantiate(&self) -> Result<Live> {
        let limits =
            StoreLimitsBuilder::new().memory_size(self.limits.max_memory_bytes).memories(1).tables(4).table_elements(100_000).instances(1).build();
        let mut store = Store::new(&self.engine, limits);
        store.limiter(|l| l);
        // The start function (if any) runs on this fuel.
        store.set_fuel(self.limits.fuel_base).map_err(wasm_err)?;
        let linker = Linker::<StoreLimits>::new(&self.engine);
        let instance = linker.instantiate_and_start(&mut store, &self.module).map_err(wasm_err)?;
        let memory =
            instance.get_memory(&store, "memory").ok_or_else(|| Error::Abi("the module must export its linear memory as `memory`".into()))?;
        Ok(Live { store, instance, memory })
    }

    /// Calls `f` with at most `fuel` instructions, in slices, checking the deadline and the
    /// caller's cancellation ([`vectorcraft_doc::cancel`]) between them.
    fn call<P: WasmParams, R: WasmResults>(&self, live: &mut Live, f: &TypedFunc<P, R>, params: P, fuel: u64, deadline: &Deadline) -> Result<R> {
        let mut left = fuel;
        let give = left.min(FUEL_SLICE);
        left -= give;
        live.store.set_fuel(give).map_err(wasm_err)?;
        let mut call = f.call_resumable(&mut live.store, params).map_err(wasm_err)?;
        loop {
            match call {
                TypedResumableCall::Finished(r) => return Ok(r),
                TypedResumableCall::HostTrap(_) => return Err(Error::Trap("unexpected host trap".into())),
                TypedResumableCall::OutOfFuel(inv) => {
                    let need = inv.required_fuel();
                    if left == 0 || need > left {
                        return Err(Error::Limit("instruction budget".into()));
                    }
                    deadline.check()?;
                    vectorcraft_doc::cancel::check().map_err(|_| Error::Cancelled)?;
                    let give = left.min(FUEL_SLICE.max(need));
                    left -= give;
                    live.store.set_fuel(give).map_err(wasm_err)?;
                    call = inv.resume(&mut live.store).map_err(wasm_err)?;
                }
            }
        }
    }

    /// Runs `vc_run` on `input` (the objects JSON) with `params` (the parameters JSON) in a fresh
    /// instance and returns the output JSON bytes.
    pub fn run(&self, input: &[u8], params: &[u8]) -> Result<Vec<u8>> {
        self.run_limited(input, params, &self.limits)
    }

    /// [`Plugin::run`] with other time and instruction budgets (live effects get a shorter one).
    pub fn run_limited(&self, input: &[u8], params: &[u8], limits: &Limits) -> Result<Vec<u8>> {
        // wasmi reports misbehaving modules as errors; a panic would be a bug in it, and must
        // still not take the document down (native builds; wasm aborts on panic anyway).
        let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.run_inner(input, params, limits)));
        run.map_err(|_| Error::Trap("the runtime panicked".into()))?
    }

    fn run_inner(&self, input: &[u8], params: &[u8], limits: &Limits) -> Result<Vec<u8>> {
        let total = input.len().checked_add(params.len()).filter(|b| *b <= i32::MAX as usize && *b <= self.limits.max_memory_bytes / 2);
        total.ok_or_else(|| Error::Limit("memory budget (the selection is too large to hand over)".into()))?;
        let deadline = Deadline::new(limits);
        let mut live = self.instantiate()?;
        let alloc: TypedFunc<i32, i32> = live.func("vc_alloc")?;
        let run: TypedFunc<RunArgs, i64> = live.func("vc_run")?;
        let place = |live: &mut Live, bytes: &[u8]| -> Result<usize> {
            let ptr = self.call(live, &alloc, bytes.len().max(1) as i32, limits.fuel_base, &deadline)? as u32 as usize;
            if ptr == 0 {
                return Err(Error::Failed("vc_alloc returned 0 (out of memory)".into()));
            }
            let outside = || Error::Abi("vc_alloc returned a block outside memory".into());
            let end = ptr.checked_add(bytes.len()).ok_or_else(outside)?;
            live.memory.data_mut(&mut live.store).get_mut(ptr..end).ok_or_else(outside)?.copy_from_slice(bytes);
            Ok(ptr)
        };
        let iptr = place(&mut live, input)?;
        let pptr = place(&mut live, params)?;
        let fuel = limits.fuel_base.saturating_add((input.len() as u64).saturating_mul(limits.fuel_per_byte));
        let packed = self.call(&mut live, &run, (iptr as i32, input.len() as i32, pptr as i32, params.len() as i32), fuel, &deadline)?;
        if packed < 0 {
            return Err(Error::Failed(format!("vc_run returned error code {packed}")));
        }
        live.read_packed(packed, "the output", limits.max_output_bytes)
    }
}
