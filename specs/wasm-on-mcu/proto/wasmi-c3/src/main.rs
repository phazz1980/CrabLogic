//! wasmi on ESP32-C3: heap per module, step time, hot swap, fuel. Prints a report to the USB console.
//! Same modules as the wasm3 measurements in `specs/wasm-on-mcu/plan.md`.
#![no_std]
#![no_main]

extern crate alloc;

mod atomics;

use alloc::vec::Vec;
use esp_hal::{clock::CpuClock, main, time::Instant};
use esp_println::println;
use wasmi::{
    Caller, CompilationMode, Config, Engine, Error, ExternType, Instance, Linker, Module, Store,
    TypedFunc, TypedResumableCall, Val,
};

esp_bootloader_esp_idf::esp_app_desc!();

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("PANIC: {info}");
    loop {}
}

const HEAP_SIZE: usize = 200 * 1024;

static MODULES: &[(&str, &[u8])] = &[
    ("blink", include_bytes!(concat!(env!("OUT_DIR"), "/blink.wasm"))),
    ("counter", include_bytes!(concat!(env!("OUT_DIR"), "/counter.wasm"))),
    ("counter_p1", include_bytes!(concat!(env!("OUT_DIR"), "/counter_p1.wasm"))),
    ("thermo17", include_bytes!("../modules/thermostat17.wasm")),
    ("self42", include_bytes!("../modules/selftest42.wasm")),
    ("self107", include_bytes!("../modules/selftest107.wasm")),
];
static HANG: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/hang.wasm"));

/// Host state of one module: what the master keeps per slot (inputs, outputs, tags).
struct Host {
    inputs: [i32; 16],
    params: [i32; 16],
    outputs: [i32; 64],
    tags: [i32; 32],
    pins: u32,
    published: u32,
}

impl Host {
    fn new() -> Self {
        Self { inputs: [0; 16], params: [0; 16], outputs: [0; 64], tags: [0; 32], pins: 0, published: 0 }
    }
}

fn idx(i: i32, n: usize) -> usize {
    (i as usize) % n
}

/// Typed host functions (the hot path); every other import of the module gets a stub returning zeros.
const TYPED: &[&str] = &[
    "input_i", "input_f", "param_i", "param_f", "output_i", "output_f", "tag_get_i", "tag_get_f",
    "tag_set_i", "tag_set_f", "gpio_write", "gpio_read", "millis", "publish", "subscribe", "log",
];

fn linker_for(engine: &Engine, module: &Module) -> Linker<Host> {
    let mut l = <Linker<Host>>::new(engine);
    l.func_wrap("crab", "input_i", |c: Caller<'_, Host>, i: i32| c.data().inputs[idx(i, 16)]).unwrap();
    l.func_wrap("crab", "input_f", |c: Caller<'_, Host>, i: i32| f32::from_bits(c.data().inputs[idx(i, 16)] as u32))
        .unwrap();
    l.func_wrap("crab", "param_i", |c: Caller<'_, Host>, i: i32| c.data().params[idx(i, 16)]).unwrap();
    l.func_wrap("crab", "param_f", |c: Caller<'_, Host>, i: i32| f32::from_bits(c.data().params[idx(i, 16)] as u32))
        .unwrap();
    l.func_wrap("crab", "output_i", |mut c: Caller<'_, Host>, i: i32, v: i32| {
        c.data_mut().outputs[idx(i, 64)] = v;
        v
    })
    .unwrap();
    l.func_wrap("crab", "output_f", |mut c: Caller<'_, Host>, i: i32, v: f32| {
        c.data_mut().outputs[idx(i, 64)] = v.to_bits() as i32;
        v
    })
    .unwrap();
    l.func_wrap("crab", "tag_get_i", |c: Caller<'_, Host>, i: i32| c.data().tags[idx(i, 32)]).unwrap();
    l.func_wrap("crab", "tag_get_f", |c: Caller<'_, Host>, i: i32| f32::from_bits(c.data().tags[idx(i, 32)] as u32))
        .unwrap();
    l.func_wrap("crab", "tag_set_i", |mut c: Caller<'_, Host>, i: i32, v: i32| c.data_mut().tags[idx(i, 32)] = v)
        .unwrap();
    l.func_wrap("crab", "tag_set_f", |mut c: Caller<'_, Host>, i: i32, v: f32| {
        c.data_mut().tags[idx(i, 32)] = v.to_bits() as i32
    })
    .unwrap();
    l.func_wrap("crab", "gpio_write", |mut c: Caller<'_, Host>, pin: i32, v: i32| {
        let bit = 1u32 << (pin as u32 % 32);
        let p = &mut c.data_mut().pins;
        *p = if v != 0 { *p | bit } else { *p & !bit };
    })
    .unwrap();
    l.func_wrap("crab", "gpio_read", |c: Caller<'_, Host>, pin: i32| (c.data().pins >> (pin as u32 % 32) & 1) as i32)
        .unwrap();
    l.func_wrap("crab", "millis", || Instant::now().duration_since_epoch().as_millis() as i32).unwrap();
    l.func_wrap("crab", "publish", |mut c: Caller<'_, Host>, _a: i32, _b: i32, _c: i32, _d: i32| {
        c.data_mut().published += 1
    })
    .unwrap();
    l.func_wrap("crab", "subscribe", |_c: Caller<'_, Host>, _a: i32, _b: i32| {}).unwrap();
    l.func_wrap("crab", "log", |_c: Caller<'_, Host>, _a: i32, _b: i32| {}).unwrap();
    for imp in module.imports() {
        if let ExternType::Func(ty) = imp.ty() {
            if imp.module() == "crab" && TYPED.contains(&imp.name()) {
                continue;
            }
            l.func_new(imp.module(), imp.name(), ty.clone(), |_c, _args, results| {
                for r in results.iter_mut() {
                    *r = Val::default_for_ty(r.ty());
                }
                Ok(())
            })
            .unwrap();
        }
    }
    l
}

/// A loaded module, like a slot of the master.
struct Loaded {
    store: Store<Host>,
    _instance: Instance,
    step: TypedFunc<i32, ()>,
    mem: usize,
}

fn used() -> usize {
    esp_alloc::HEAP.used()
}

fn now_us() -> u64 {
    Instant::now().duration_since_epoch().as_micros()
}

const FUEL: u64 = 1 << 40;

fn load(engine: &Engine, bytes: &[u8], fuel: bool) -> Result<Loaded, Error> {
    let module = Module::new(engine, bytes)?;
    let mut store = Store::new(engine, Host::new());
    if fuel {
        store.set_fuel(FUEL)?;
    }
    let instance = {
        let linker = linker_for(engine, &module);
        linker.instantiate_and_start(&mut store, &module)?
    };
    if let Ok(f) = instance.get_typed_func::<(), i32>(&store, "abi_version") {
        f.call(&mut store, ())?;
    }
    if let Ok(f) = instance.get_typed_func::<(), ()>(&store, "init") {
        f.call(&mut store, ())?;
    }
    let step = instance.get_typed_func::<i32, ()>(&store, "step")?;
    let mem = instance.get_memory(&store, "memory").map_or(0, |m| m.data_size(&store));
    Ok(Loaded { store, _instance: instance, step, mem })
}

fn engine(mode: CompilationMode, fuel: bool) -> Engine {
    let mut c = Config::default();
    c.compilation_mode(mode);
    c.consume_fuel(fuel);
    c.wasm_custom_page_sizes(true);
    Engine::new(&c)
}

fn mode_name(m: CompilationMode) -> &'static str {
    match m {
        CompilationMode::Eager => "eager",
        CompilationMode::LazyTranslation => "lazytr",
        CompilationMode::Lazy => "lazy",
    }
}

/// Heap of one module in a fresh engine: after load, after the first step (lazy translation), and
/// what stays after dropping it.
fn measure_heap(mode: CompilationMode) {
    println!("\n== heap per module, mode={} (fuel on), bytes", mode_name(mode));
    println!("name        wasm   engine   load  +step1  memory  kept-in-engine  after-engine-drop");
    for (name, bytes) in MODULES {
        let h0 = used();
        let e = engine(mode, true);
        let h1 = used();
        let mut m = match load(&e, bytes, true) {
            Ok(m) => m,
            Err(err) => {
                println!("{name:<10} load failed: {err}");
                continue;
            }
        };
        let h2 = used();
        let ok = m.step.call(&mut m.store, 10).is_ok();
        let h3 = used();
        let mem = m.mem;
        drop(m);
        let h4 = used();
        drop(e);
        let h5 = used();
        println!(
            "{name:<10} {:>5} {:>8} {:>6} {:>7} {:>7} {:>15} {:>18}{}",
            bytes.len(),
            h1 as isize - h0 as isize,
            h2 as isize - h1 as isize,
            h3 as isize - h2 as isize,
            mem,
            h4 as isize - h1 as isize,
            h5 as isize - h0 as isize,
            if ok { "" } else { "  (step trapped)" }
        );
    }
}

/// All modules at once in one engine, as on a master with several slots.
fn measure_all_together(mode: CompilationMode) {
    let h0 = used();
    let e = engine(mode, true);
    let mut v = Vec::new();
    // Modules with a full 64 KiB page do not fit together into the 200 KiB heap; leave them out.
    for (name, bytes) in MODULES.iter().filter(|(n, _)| !matches!(*n, "counter" | "self42" | "self107")) {
        match load(&e, bytes, true) {
            Ok(mut m) => {
                let _ = m.step.call(&mut m.store, 10);
                v.push(m);
            }
            Err(err) => println!("{name}: {err}"),
        }
    }
    let mem: usize = v.iter().map(|m| m.mem).sum();
    println!(
        "\n== all {} modules in one engine, mode={}: {} bytes heap ({} of them linear memory), free {}",
        v.len(),
        mode_name(mode),
        used() - h0,
        mem,
        esp_alloc::HEAP.free()
    );
}

fn measure_step_time() {
    println!("\n== step time, us (avg of 1000 steps after 10 warm-up), mode=lazytr");
    println!("name        fuel-off  fuel-on  fuel/step");
    for (name, bytes) in MODULES {
        let mut t = [0u64; 2];
        let mut fuel_per_step = 0;
        for k in 0..2 {
            // Fresh engine per run: an engine keeps the code of dropped modules until it is dropped.
            let e = engine(CompilationMode::LazyTranslation, k == 1);
            let mut m = match load(&e, bytes, k == 1) {
                Ok(m) => m,
                Err(err) => {
                    println!("{name}: load failed: {err}");
                    continue;
                }
            };
            for _ in 0..10 {
                let _ = m.step.call(&mut m.store, 10);
            }
            let f0 = m.store.get_fuel().unwrap_or(0);
            let s = now_us();
            for _ in 0..1000 {
                let _ = m.step.call(&mut m.store, 10);
            }
            t[k] = now_us() - s;
            if k == 1 {
                fuel_per_step = (f0 - m.store.get_fuel().unwrap_or(0)) / 1000;
            }
        }
        // t is the total of 1000 steps in us: per step = t / 1000, printed with one decimal.
        println!(
            "{name:<10} {:>6}.{} {:>6}.{} {:>9}",
            t[0] / 1000,
            t[0] % 1000 / 100,
            t[1] / 1000,
            t[1] % 1000 / 100,
            fuel_per_step
        );
    }
}

/// Unload + load the same module many times: time per swap and heap drift. With `own_engine` every
/// load gets a fresh engine (dropped with the module), otherwise all loads share one engine.
fn measure_hot_swap(mode: CompilationMode, idx: usize, n: u32, own_engine: bool) {
    let (name, bytes) = MODULES[idx];
    let mut e = engine(mode, true);
    let mut m = load(&e, bytes, true).unwrap();
    let _ = m.step.call(&mut m.store, 10);
    let base = used();
    let (mut total, mut max, mut done) = (0u64, 0u64, 0u32);
    let kind = if own_engine { "own engine per load" } else { "shared engine" };
    println!("
== hot swap of {name}, mode={}, {kind}, {n} times", mode_name(mode));
    for i in 1..=n {
        let drift = used() as isize - base as isize;
        if drift > 20_000 {
            println!("  swap {i:>5}: heap drift {drift:+} before load, stopping (would run out of heap)");
            break;
        }
        let s = now_us();
        drop(m);
        if own_engine {
            e = engine(mode, true);
        }
        m = load(&e, bytes, true).unwrap();
        let _ = m.step.call(&mut m.store, 10);
        let d = now_us() - s;
        total += d;
        max = max.max(d);
        done = i;
        let drift = used() as isize - base as isize;
        if i <= 3 || i % 250 == 0 {
            println!("  swap {i:>5}: heap used {} (drift {drift:+})", used());
        }
    }
    println!("  swap time incl. first step: avg {} us, max {} us", total / done.max(1) as u64, max);
}

/// A module that never returns: fuel stops it, and the call can be resumed with more fuel.
fn measure_fuel_stop() {
    println!("\n== fuel: endless loop module");
    let e = engine(CompilationMode::LazyTranslation, true);
    let mut m = load(&e, HANG, true).unwrap();
    {
        // Probe 1: resumable call that finishes normally (blink).
        let mut b = load(&e, MODULES[0].1, true).unwrap();
        let r = b.step.call_resumable(&mut b.store, 10);
        println!("  probe resumable finished: {}", matches!(r, Ok(TypedResumableCall::Finished(()))));
        // Probe 2: plain call running out of fuel.
        m.store.set_fuel(10_000).unwrap();
        let r = m.step.call(&mut m.store, 10);
        println!("  probe plain out-of-fuel: {}", if r.is_err() { "trapped" } else { "returned?!" });
    }
    for quantum in [10_000u64, 100_000, 1_000_000] {
        m.store.set_fuel(quantum).unwrap();
        let s = now_us();
        let r = m.step.call_resumable(&mut m.store, 10);
        let d = now_us() - s;
        match r {
            Ok(TypedResumableCall::OutOfFuel(call)) => {
                // Resume once with the same quantum: the module continues where it stopped.
                m.store.set_fuel(quantum).unwrap();
                let s2 = now_us();
                let again = call.resume(&mut m.store);
                let d2 = now_us() - s2;
                let state = match again {
                    Ok(TypedResumableCall::OutOfFuel(_)) => "out of fuel again",
                    Ok(TypedResumableCall::Finished(())) => "finished?!",
                    Ok(_) => "host trap",
                    Err(_) => "error",
                };
                println!("  quantum {quantum:>8}: stopped after {d} us; resumed: {state} after {d2} us");
            }
            Ok(_) => println!("  quantum {quantum}: not out of fuel?!"),
            Err(err) => println!("  quantum {quantum}: error {err}"),
        }
    }
    // A plain call (the usual step) traps with "out of fuel" and the module stays usable.
    m.store.set_fuel(50_000).unwrap();
    let r = m.step.call(&mut m.store, 10);
    println!("  plain call: {}", if r.is_err() { "trapped (out of fuel)" } else { "returned?!" });
}

#[main]
fn main() -> ! {
    let _p = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    esp_alloc::heap_allocator!(size: HEAP_SIZE);
    // Give the USB console time to come up after the reset.
    let s = now_us();
    while now_us() - s < 2_000_000 {}

    println!("\n######## wasmi {} on ESP32-C3, heap {} bytes, free {}", "2.0.0", HEAP_SIZE, esp_alloc::HEAP.free());
    measure_fuel_stop();
    measure_heap(CompilationMode::Eager);
    measure_heap(CompilationMode::LazyTranslation);
    measure_heap(CompilationMode::Lazy);
    measure_all_together(CompilationMode::LazyTranslation);
    measure_all_together(CompilationMode::Eager);
    measure_step_time();
    measure_hot_swap(CompilationMode::LazyTranslation, 5, 1000, false);
    measure_hot_swap(CompilationMode::LazyTranslation, 5, 1000, true);
    measure_hot_swap(CompilationMode::Eager, 5, 1000, true);
    measure_hot_swap(CompilationMode::LazyTranslation, 3, 1000, true);
    measure_hot_swap(CompilationMode::LazyTranslation, 0, 1000, true);
    measure_fuel_stop();
    println!("\n######## done, heap used {} free {}", used(), esp_alloc::HEAP.free());
    loop {}
}
