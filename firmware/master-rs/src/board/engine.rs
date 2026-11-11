//! The WAMR engine of the board: the `crab-kernel` [`Engine`] over the C API of WAMR.
//!
//! Two ways to run a module: the classic interpreter (`.wasm`, copied to RAM) and AOT executed
//! in place from flash (`.aot` built with `wamrc --xip`). Every import of
//! [`crab_kernel::abi::IMPORTS`] is one raw native; its attachment is the import id.
//!
//! The runtime and every instance belong to the thread that created them: WAMR checks the
//! native stack against the boundary of that thread.
//!
//! `crab.yield` does not suspend a step yet: it hands out the next budget until the work limit
//! of the call is spent, then the step ends with "too long".

use core::ffi::{c_char, c_void, CStr};
use std::ffi::CString;

use crab_kernel::abi::{self, id, Ty, ABI_VERSION, IMPORTS};
use crab_kernel::engine::{Engine, Instance, Limits, Run};
use crab_kernel::host::{Host, Mem, Val};
use crab_master_rs::wamr::{crab_mem, ExecutionMode};

type Module = *mut c_void;
type Inst = *mut c_void;
type Env = *mut c_void;
type Func = *mut c_void;

/// `NativeSymbol` of `wasm_export.h`.
#[repr(C)]
struct NativeSymbol {
    symbol: *const c_char,
    func_ptr: *mut c_void,
    signature: *const c_char,
    attachment: *mut c_void,
}

extern "C" {
    fn crab_wamr_init(natives: *mut NativeSymbol, count: u32) -> i32;
    fn crab_wamr_load(
        buf: *mut u8,
        len: u32,
        memory: u32,
        err: *mut c_char,
        err_len: u32,
    ) -> Module;
    fn crab_wamr_is_aot(module: Module) -> i32;
    fn crab_wamr_unload(module: Module);
    fn crab_wamr_import(
        module: Module,
        index: i32,
        module_name: *mut *const c_char,
        name: *mut *const c_char,
    ) -> i32;
    fn crab_wamr_exec_env(inst: Inst, stack: u32) -> Env;
    fn crab_wamr_memory(inst: Inst, len: *mut u32) -> *mut u8;

    fn wasm_runtime_get_import_count(module: Module) -> i32;
    fn wasm_runtime_instantiate(
        module: Module,
        stack: u32,
        heap: u32,
        err: *mut c_char,
        err_len: u32,
    ) -> Inst;
    fn wasm_runtime_lookup_function(inst: Inst, name: *const c_char) -> Func;
    fn wasm_runtime_call_wasm(env: Env, func: Func, argc: u32, argv: *mut u32) -> bool;
    fn wasm_runtime_get_exception(inst: Inst) -> *const c_char;
    fn wasm_runtime_set_exception(inst: Inst, exception: *const c_char);
    fn wasm_runtime_clear_exception(inst: Inst);
    fn wasm_runtime_set_user_data(env: Env, data: *mut c_void);
    fn wasm_runtime_get_user_data(env: Env) -> *mut c_void;
    fn wasm_runtime_get_function_attachment(env: Env) -> *mut c_void;
    fn wasm_runtime_get_module_inst(env: Env) -> Inst;
    fn wasm_runtime_destroy_exec_env(env: Env);
    fn wasm_runtime_deinstantiate(inst: Inst);
}

/// WASM operand stack and execution environment of one module, bytes (stage 0 measurements).
const STACK: u32 = 4096;
/// The exception `crab.yield` raises when a call spent its work limit.
const TOO_LONG: &CStr = c"crab: too long";

/// A module image as the board stores it.
pub enum Image {
    /// Bytecode for the interpreter; WAMR patches it in place, so it lives in RAM.
    Wasm(Vec<u8>),
    /// An XIP AOT image mapped from flash (DROM) for as long as the firmware runs.
    AotXip(&'static [u8]),
}

/// The WAMR runtime. Create it once, on the thread that will run the modules.
pub struct Wamr {
    _one_thread: core::marker::PhantomData<*mut ()>,
}

fn signature(import: &abi::Import) -> CString {
    let letter = |t: &Ty| match t {
        Ty::I32 => 'i',
        Ty::F32 => 'f',
    };
    let mut s = String::from("(");
    s.extend(import.params.iter().map(letter));
    s.push(')');
    s.extend(import.results.iter().map(letter));
    CString::new(s).expect("signature")
}

impl Wamr {
    pub fn new() -> Result<Self, String> {
        // The table and its strings live as long as the runtime, that is, forever.
        let natives: Vec<NativeSymbol> = IMPORTS
            .iter()
            .enumerate()
            .map(|(id, import)| NativeSymbol {
                symbol: CString::new(import.name).expect("import name").into_raw(),
                func_ptr: native as *mut c_void,
                signature: signature(import).into_raw(),
                attachment: id as *mut c_void,
            })
            .collect();
        let natives = natives.leak();
        // SAFETY: the table is valid, writable and never freed.
        if unsafe { crab_wamr_init(natives.as_mut_ptr(), natives.len() as u32) } == 0 {
            return Err("WAMR не запущен".to_owned());
        }
        Ok(Self {
            _one_thread: core::marker::PhantomData,
        })
    }

    /// Loads an image. `memory` is the module's `crab.mem`: bytes of linear memory, 0 keeps
    /// what the module declares.
    pub fn load_image<H: Host + 'static>(
        &self,
        image: Image,
        memory: u32,
        allow: &[String],
        limits: Limits,
        host: H,
    ) -> Result<WamrInstance<H>, String> {
        if memory as usize > limits.max_memory {
            return Err(format!(
                "модулю нужно {memory} Б памяти, разрешено {}",
                limits.max_memory
            ));
        }
        let (mut ram, ptr, len) = match image {
            Image::Wasm(mut bytes) => {
                let (ptr, len) = (bytes.as_mut_ptr(), bytes.len());
                (Some(bytes), ptr, len)
            }
            // WAMR only reads an XIP image.
            Image::AotXip(bytes) => (None, bytes.as_ptr() as *mut u8, bytes.len()),
        };
        let mut err = [0 as c_char; 128];
        // SAFETY: the buffer outlives the module (`ram` moves into the instance; flash is static).
        let module =
            unsafe { crab_wamr_load(ptr, len as u32, memory, err.as_mut_ptr(), err.len() as u32) };
        if module.is_null() {
            return Err(format!("модуль не загружен: {}", text(err.as_ptr())));
        }
        let mode = if unsafe { crab_wamr_is_aot(module) } != 0 {
            ExecutionMode::AotXip
        } else {
            ExecutionMode::Interpreter
        };
        // From here on `m` frees what is already created if a later step fails.
        let mut m = WamrInstance {
            ctx: Box::new(Ctx {
                head: Head {
                    dispatch: dispatch::<H>,
                },
                host,
                budget: limits.yield_budget,
                yields_per_call: (limits.fuel_per_call / limits.yield_budget.max(1) as u64)
                    .min(u32::MAX as u64) as u32,
                yields_left: 0,
            }),
            module,
            inst: core::ptr::null_mut(),
            env: core::ptr::null_mut(),
            step: core::ptr::null_mut(),
            mode,
            _image: ram.take(),
        };

        let imports = m.imports();
        abi::check_imports(imports.iter().map(|(a, b)| (a.as_str(), b.as_str())), allow)?;

        // SAFETY: `module` is loaded; the error buffer is valid.
        m.inst = unsafe {
            wasm_runtime_instantiate(module, STACK, 0, err.as_mut_ptr(), err.len() as u32)
        };
        if m.inst.is_null() {
            return Err(format!("модуль не запущен: {}", text(err.as_ptr())));
        }
        m.env = unsafe { crab_wamr_exec_env(m.inst, STACK) };
        if m.env.is_null() {
            return Err("модуль не запущен: нет памяти под окружение".to_owned());
        }
        // The context is boxed, so its address stays the same when the instance moves.
        unsafe { wasm_runtime_set_user_data(m.env, (&mut *m.ctx as *mut Ctx<H>).cast()) };

        let abi_version = m.lookup(c"abi_version").ok_or_else(|| {
            format!(
                "в модуле нет функции abi_version() (нужен ABI {ABI_VERSION}): пересоберите модуль"
            )
        })?;
        let mut argv = [0u32; 4];
        m.call(abi_version, 0, &mut argv)
            .map_err(|e| format!("abi_version не выполнена: {e}"))?;
        let version = argv[0] as i32;
        if version != ABI_VERSION {
            return Err(format!(
                "модуль собран для ABI {version}, поддерживается {ABI_VERSION}: пересоберите модуль"
            ));
        }
        m.step = m
            .lookup(c"step")
            .ok_or_else(|| "в модуле нет функции step(dt_ms: i32)".to_owned())?;
        if let Some(init) = m.lookup(c"init") {
            m.call(init, 0, &mut argv)?;
        }
        Ok(m)
    }
}

impl Engine for Wamr {
    type Instance<H: Host + 'static> = WamrInstance<H>;

    /// Runs bytecode in the interpreter. An AOT image cannot come as a borrowed slice: it runs
    /// in place from flash, see [`Wamr::load_image`].
    fn load<H: Host + 'static>(
        &self,
        wasm: &[u8],
        allow: &[String],
        limits: Limits,
        host: H,
    ) -> Result<WamrInstance<H>, String> {
        if !wasm.starts_with(b"\0asm") {
            return Err("ожидался модуль .wasm; образ .aot загружается из флеша".to_owned());
        }
        let memory = crab_mem(wasm).unwrap_or(0);
        self.load_image(Image::Wasm(wasm.to_vec()), memory, allow, limits, host)
    }
}

/// What the natives reach through the user data of the execution environment; the first field
/// of [`Ctx`], so the pointer is the same for every host type.
#[repr(C)]
struct Head {
    dispatch: unsafe fn(*mut Head, usize, Inst, *mut u64),
}

#[repr(C)]
struct Ctx<H> {
    head: Head,
    host: H,
    /// What `crab.yield` gives back.
    budget: i32,
    /// How many times `crab.yield` may be called in one call of the module.
    yields_per_call: u32,
    yields_left: u32,
}

/// Every import of the ABI enters here (a raw native: arguments and the result are 64-bit slots).
unsafe extern "C" fn native(env: Env, args: *mut u64) {
    let id = wasm_runtime_get_function_attachment(env) as usize;
    let head = wasm_runtime_get_user_data(env) as *mut Head;
    if !head.is_null() {
        ((*head).dispatch)(head, id, wasm_runtime_get_module_inst(env), args);
    }
}

unsafe fn dispatch<H: Host>(head: *mut Head, import_id: usize, inst: Inst, args: *mut u64) {
    let ctx = &mut *(head as *mut Ctx<H>);
    let Some(import) = IMPORTS.get(import_id) else {
        return;
    };
    // The low half of slot 0 takes the result.
    let result = args as *mut u32;
    if import_id == id::YIELD {
        if ctx.yields_left == 0 {
            wasm_runtime_set_exception(inst, TOO_LONG.as_ptr());
        } else {
            ctx.yields_left -= 1;
            *result = ctx.budget as u32;
        }
        return;
    }
    let mut buf = [Val::I32(0); 8];
    let n = import.params.len().min(buf.len());
    for (i, ty) in import.params.iter().take(n).enumerate() {
        let bits = *args.add(i) as u32;
        buf[i] = match ty {
            Ty::I32 => Val::I32(bits as i32),
            Ty::F32 => Val::F32(f32::from_bits(bits)),
        };
    }
    let mut len = 0u32;
    let base = crab_wamr_memory(inst, &mut len);
    let mem = if base.is_null() || len == 0 {
        Mem::new(None)
    } else {
        // SAFETY: the module is stopped inside this native, nobody else touches its memory.
        Mem::new(Some(core::slice::from_raw_parts_mut(base, len as usize)))
    };
    let r = ctx.host.call(import_id, &buf[..n], mem);
    match import.results.first() {
        Some(Ty::I32) => *result = r.map_or(0, |v| v.i()) as u32,
        Some(Ty::F32) => *result = r.map_or(0.0, |v| v.f()).to_bits(),
        None => {}
    }
}

fn text(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    // SAFETY: WAMR returns NUL-terminated strings; our buffers are zero-filled.
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

/// A WAMR exception as a message for the user.
fn describe(exception: &str) -> String {
    let what = exception.strip_prefix("Exception: ").unwrap_or(exception);
    if what == TOO_LONG.to_str().unwrap_or_default() {
        "модуль выполняется слишком долго (возможно, бесконечный цикл)".to_owned()
    } else if what.contains("out of bounds memory access") {
        "модуль обратился за пределы памяти".to_owned()
    } else {
        format!("модуль остановлен: {what}")
    }
}

/// A module running in WAMR.
pub struct WamrInstance<H> {
    ctx: Box<Ctx<H>>,
    module: Module,
    inst: Inst,
    env: Env,
    step: Func,
    mode: ExecutionMode,
    /// The bytecode the interpreter runs.
    _image: Option<Vec<u8>>,
}

impl<H> WamrInstance<H> {
    pub fn mode(&self) -> ExecutionMode {
        self.mode
    }

    /// Bytes of linear memory the module has now.
    pub fn memory_len(&self) -> u32 {
        let mut len = 0u32;
        // SAFETY: `inst` is a live instance.
        unsafe { crab_wamr_memory(self.inst, &mut len) };
        len
    }

    fn imports(&self) -> Vec<(String, String)> {
        // SAFETY: `module` is loaded; the names belong to it and are copied at once.
        unsafe {
            (0..wasm_runtime_get_import_count(self.module))
                .filter_map(|i| {
                    let (mut module, mut name) = (core::ptr::null(), core::ptr::null());
                    (crab_wamr_import(self.module, i, &mut module, &mut name) != 0)
                        .then(|| (text(module), text(name)))
                })
                .collect()
        }
    }

    fn lookup(&self, name: &CStr) -> Option<Func> {
        // SAFETY: `inst` is a live instance.
        let f = unsafe { wasm_runtime_lookup_function(self.inst, name.as_ptr()) };
        (!f.is_null()).then_some(f)
    }

    /// Calls a function; `argv` holds the arguments and then the result.
    fn call(&mut self, func: Func, argc: u32, argv: &mut [u32; 4]) -> Result<(), String> {
        self.ctx.yields_left = self.ctx.yields_per_call;
        // SAFETY: called on the thread that created the instance; `argv` fits the signature.
        if unsafe { wasm_runtime_call_wasm(self.env, func, argc, argv.as_mut_ptr()) } {
            return Ok(());
        }
        let exception = text(unsafe { wasm_runtime_get_exception(self.inst) });
        // The module stays loaded and may be called again.
        unsafe { wasm_runtime_clear_exception(self.inst) };
        Err(describe(&exception))
    }

    /// Copies `bytes` into memory the module allocates with `alloc`.
    fn put(&mut self, alloc: Func, bytes: &[u8]) -> Result<u32, String> {
        let mut argv = [bytes.len() as u32, 0, 0, 0];
        self.call(alloc, 1, &mut argv)?;
        let ptr = argv[0];
        let mut len = 0u32;
        // SAFETY: the memory is read after `alloc` returned (it may have grown).
        let base = unsafe { crab_wamr_memory(self.inst, &mut len) };
        let end = (ptr as usize).checked_add(bytes.len());
        if base.is_null() || end.is_none_or(|end| end > len as usize) {
            return Err("alloc вернула адрес за пределами памяти".to_owned());
        }
        // SAFETY: the range was checked against the memory size.
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), base.add(ptr as usize), bytes.len())
        };
        Ok(ptr)
    }
}

impl<H> Drop for WamrInstance<H> {
    fn drop(&mut self) {
        // SAFETY: each pointer is either null or owned by this instance.
        unsafe {
            if !self.env.is_null() {
                wasm_runtime_destroy_exec_env(self.env);
            }
            if !self.inst.is_null() {
                wasm_runtime_deinstantiate(self.inst);
            }
            crab_wamr_unload(self.module);
        }
    }
}

impl<H> Instance<H> for WamrInstance<H> {
    fn host(&self) -> &H {
        &self.ctx.host
    }

    fn host_mut(&mut self) -> &mut H {
        &mut self.ctx.host
    }

    fn call_step(&mut self, dt_ms: u32) -> Result<Run, String> {
        let mut argv = [dt_ms, 0, 0, 0];
        self.call(self.step, 1, &mut argv)?;
        Ok(Run::Done)
    }

    fn resume(&mut self) -> Result<Run, String> {
        Ok(Run::Done)
    }

    fn suspended(&self) -> bool {
        false
    }

    fn call_on_message(&mut self, topic: &str, data: &[u8]) -> Result<(), String> {
        let alloc = self
            .lookup(c"alloc")
            .ok_or_else(|| "модулю нужна функция alloc(len) для приёма сообщений".to_owned())?;
        let on_message = self.lookup(c"on_message").ok_or_else(|| {
            "в модуле нет функции on_message(topic_ptr, topic_len, ptr, len)".to_owned()
        })?;
        let topic_ptr = self.put(alloc, topic.as_bytes())?;
        let data_ptr = self.put(alloc, data)?;
        let mut argv = [topic_ptr, topic.len() as u32, data_ptr, data.len() as u32];
        self.call(on_message, 4, &mut argv)
    }
}
