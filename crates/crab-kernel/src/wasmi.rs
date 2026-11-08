//! The wasmi engine (PC). Every import of [`crate::abi::IMPORTS`] is linked once with its
//! signature and dispatched by id to the [`Host`].

use alloc::borrow::ToOwned;
use alloc::format;
use alloc::string::String;

use wasmi::{
    Caller, Config, Error, FuncType, Linker, Memory, Module, Store, StoreLimits,
    StoreLimitsBuilder, TrapCode, TypedFunc, ValType, F32,
};

use crate::abi::{self, Ty, ABI_VERSION, IMPORTS};
use crate::engine::{self, Limits};
use crate::host::{self, Host, Mem};

/// The wasmi engine with fuel metering on.
#[derive(Clone)]
pub struct Wasmi {
    engine: wasmi::Engine,
}

impl Default for Wasmi {
    fn default() -> Self {
        Self::new()
    }
}

impl Wasmi {
    pub fn new() -> Self {
        let mut config = Config::default();
        config.consume_fuel(true);
        Self {
            engine: wasmi::Engine::new(&config),
        }
    }
}

struct State<H> {
    host: H,
    limits: StoreLimits,
    memory: Option<Memory>,
}

/// A module running in wasmi.
pub struct WasmiInstance<H: 'static> {
    store: Store<State<H>>,
    instance: wasmi::Instance,
    step: TypedFunc<i32, ()>,
    fuel: u64,
}

fn val_type(t: Ty) -> ValType {
    match t {
        Ty::I32 => ValType::I32,
        Ty::F32 => ValType::F32,
    }
}

fn to_host(v: &wasmi::Val) -> host::Val {
    match v {
        wasmi::Val::F32(f) => host::Val::F32(f32::from(*f)),
        other => host::Val::I32(other.i32().unwrap_or(0)),
    }
}

fn link<H: Host + 'static>(linker: &mut Linker<State<H>>) -> Result<(), String> {
    for (id, import) in IMPORTS.iter().enumerate() {
        let ty = FuncType::new(
            import.params.iter().copied().map(val_type),
            import.results.iter().copied().map(val_type),
        );
        let result = import.results.first().copied();
        linker
            .func_new(
                abi::MODULE,
                import.name,
                ty,
                move |mut c: Caller<'_, State<H>>, args: &[wasmi::Val], out: &mut [wasmi::Val]| {
                    let mut buf = [host::Val::I32(0); 8];
                    let n = args.len().min(buf.len());
                    for (b, a) in buf.iter_mut().zip(args) {
                        *b = to_host(a);
                    }
                    let memory = c
                        .data()
                        .memory
                        .or_else(|| c.get_export("memory").and_then(|e| e.into_memory()));
                    let r = match memory {
                        Some(m) => {
                            let (data, state) = m.data_and_store_mut(&mut c);
                            state.host.call(id, &buf[..n], Mem::new(Some(data)))
                        }
                        None => c.data_mut().host.call(id, &buf[..n], Mem::new(None)),
                    };
                    if let (Some(slot), Some(ty)) = (out.first_mut(), result) {
                        *slot = match (ty, r) {
                            (Ty::I32, r) => wasmi::Val::I32(r.map_or(0, |v| v.i())),
                            (Ty::F32, r) => wasmi::Val::F32(F32::from(r.map_or(0.0, |v| v.f()))),
                        };
                    }
                    Ok(())
                },
            )
            .map_err(|e| format!("ошибка подключения функций: {e}"))?;
    }
    Ok(())
}

/// A trap or error as a message for the user.
pub fn describe(e: &Error) -> String {
    match e.as_trap_code() {
        Some(TrapCode::OutOfFuel) => {
            "модуль выполняется слишком долго (возможно, бесконечный цикл)".to_owned()
        }
        Some(TrapCode::MemoryOutOfBounds) => "модуль обратился за пределы памяти".to_owned(),
        Some(code) => format!("модуль остановлен: {code}"),
        None => format!("ошибка модуля: {e}"),
    }
}

impl engine::Engine for Wasmi {
    type Instance<H: Host + 'static> = WasmiInstance<H>;

    fn load<H: Host + 'static>(
        &self,
        wasm: &[u8],
        allow: &[String],
        limits: Limits,
        host: H,
    ) -> Result<WasmiInstance<H>, String> {
        let module =
            Module::new(&self.engine, wasm).map_err(|e| format!("модуль WASM не загружен: {e}"))?;
        abi::check_imports(module.imports().map(|i| (i.module(), i.name())), allow)?;

        let mut store = Store::new(
            &self.engine,
            State {
                host,
                limits: StoreLimitsBuilder::new()
                    .memory_size(limits.max_memory)
                    .build(),
                memory: None,
            },
        );
        store.limiter(|s| &mut s.limits);

        let mut linker = <Linker<State<H>>>::new(&self.engine);
        link(&mut linker)?;
        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|e| format!("модуль не запущен: {e}"))?;
        store.data_mut().memory = instance.get_memory(&store, "memory");

        let abi_version = instance
            .get_typed_func::<(), i32>(&store, "abi_version")
            .map_err(|_| "в модуле нет функции abi_version() (нужен ABI 3)".to_owned())?;
        store
            .set_fuel(limits.fuel_per_call)
            .map_err(|e| format!("ошибка лимита выполнения: {e}"))?;
        let version = abi_version
            .call(&mut store, ())
            .map_err(|e| format!("abi_version не выполнена: {e}"))?;
        if version != ABI_VERSION {
            return Err(format!(
                "модуль собран для ABI {version}, поддерживается {ABI_VERSION}"
            ));
        }
        let step = instance
            .get_typed_func::<i32, ()>(&store, "step")
            .map_err(|_| "в модуле нет функции step(dt_ms: i32)".to_owned())?;
        let mut m = WasmiInstance {
            store,
            instance,
            step,
            fuel: limits.fuel_per_call,
        };
        if let Ok(init) = m.instance.get_typed_func::<(), ()>(&m.store, "init") {
            m.refuel()?;
            init.call(&mut m.store, ()).map_err(|e| describe(&e))?;
        }
        Ok(m)
    }
}

impl<H: 'static> WasmiInstance<H> {
    fn refuel(&mut self) -> Result<(), String> {
        self.store
            .set_fuel(self.fuel)
            .map_err(|e| format!("ошибка лимита выполнения: {e}"))
    }
}

impl<H: 'static> engine::Instance<H> for WasmiInstance<H> {
    fn host(&self) -> &H {
        &self.store.data().host
    }

    fn host_mut(&mut self) -> &mut H {
        &mut self.store.data_mut().host
    }

    fn call_step(&mut self, dt_ms: u32) -> Result<(), String> {
        self.refuel()?;
        self.step
            .call(&mut self.store, dt_ms as i32)
            .map_err(|e| describe(&e))
    }

    fn call_on_message(&mut self, topic: &str, data: &[u8]) -> Result<(), String> {
        let alloc = self
            .instance
            .get_typed_func::<i32, i32>(&self.store, "alloc")
            .map_err(|_| "модулю нужна функция alloc(len) для приёма сообщений".to_owned())?;
        let on_message = self
            .instance
            .get_typed_func::<(i32, i32, i32, i32), ()>(&self.store, "on_message")
            .map_err(|_| {
                "в модуле нет функции on_message(topic_ptr, topic_len, ptr, len)".to_owned()
            })?;
        let memory = self
            .store
            .data()
            .memory
            .ok_or_else(|| "модуль не экспортирует память «memory»".to_owned())?;
        self.refuel()?;
        let put = |store: &mut Store<State<H>>, bytes: &[u8]| -> Result<i32, String> {
            let ptr = alloc
                .call(&mut *store, bytes.len() as i32)
                .map_err(|e| describe(&e))?;
            memory
                .write(&mut *store, ptr as usize, bytes)
                .map_err(|_| "alloc вернула адрес за пределами памяти".to_owned())?;
            Ok(ptr)
        };
        let topic_ptr = put(&mut self.store, topic.as_bytes())?;
        let data_ptr = put(&mut self.store, data)?;
        on_message
            .call(
                &mut self.store,
                (topic_ptr, topic.len() as i32, data_ptr, data.len() as i32),
            )
            .map_err(|e| describe(&e))
    }
}
