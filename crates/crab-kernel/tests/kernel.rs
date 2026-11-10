//! The kernel with the wasmi engine: leases on trap, unload and swap; permissions at load.

use std::cell::RefCell;
use std::rc::Rc;

use crab_kernel::abi;
use crab_kernel::engine::{Engine, Instance, Limits};
use crab_kernel::host::{Devices, Mem, ModuleHost, Val};
use crab_kernel::lease::{Fault, Hal, PinUse};
use crab_kernel::wasmi::{Wasmi, WasmiInstance};
use crab_kernel::{Kernel, Word};

/// Records every output put into its safe state.
#[derive(Clone, Default)]
struct StubHal(Rc<RefCell<Vec<(u8, PinUse, Fault)>>>);

impl StubHal {
    fn take(&self) -> Vec<(u8, PinUse, Fault)> {
        let mut v = std::mem::take(&mut *self.0.borrow_mut());
        v.sort_by_key(|x| x.0);
        v
    }
}

impl Hal for StubHal {
    fn safe(&self, pin: u8, use_: PinUse, fault: Fault) {
        self.0.borrow_mut().push((pin, use_, fault));
    }
}

struct Board {
    hal: StubHal,
    writes: u32,
}

impl Devices for Board {
    type Hal = StubHal;

    fn hal(&self) -> StubHal {
        self.hal.clone()
    }

    fn call(&mut self, import: usize, _: &[Val], _: Mem<'_>) -> Option<Val> {
        if import == abi::id::GPIO_WRITE || import == abi::id::PWM_WRITE {
            self.writes += 1;
        }
        None
    }
}

type Module = WasmiInstance<ModuleHost<Board>>;

/// A module that subscribes to `e`, sets output 0 on a message, traps when input 0 is set and
/// otherwise writes `writes` (`gpio_write` / `pwm_write` with a pin number) every step.
fn wat(writes: &str) -> Vec<u8> {
    let text = format!(
        r#"(module
  (import "crab" "gpio_write" (func $gw (param i32 i32)))
  (import "crab" "pwm_write" (func $pw (param i32 i32)))
  (import "crab" "input_i" (func $in (param i32) (result i32)))
  (import "crab" "output_i" (func $out (param i32 i32) (result i32)))
  (import "crab" "subscribe" (func $sub (param i32 i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "e")
  (global $bump (mut i32) (i32.const 1024))
  (func (export "abi_version") (result i32) i32.const 4)
  (func (export "init") (call $sub (i32.const 0) (i32.const 1)))
  (func (export "alloc") (param i32) (result i32) (local i32)
    global.get $bump
    local.set 1
    global.get $bump
    local.get 0
    i32.add
    global.set $bump
    local.get 1)
  (func (export "on_message") (param i32 i32 i32 i32)
    (drop (call $out (i32.const 0) (i32.const 1))))
  (func (export "step") (param i32)
    (if (call $in (i32.const 0)) (then unreachable))
    {writes}))"#
    );
    wat::parse_str(text).unwrap()
}

fn allow() -> Vec<String> {
    vec!["gpio_write".into(), "pwm_write".into()]
}

fn load(hal: &StubHal, writes: &str) -> Module {
    let board = Board {
        hal: hal.clone(),
        writes: 0,
    };
    Wasmi::new()
        .load(
            &wat(writes),
            &allow(),
            Limits::default(),
            ModuleHost::new(board, 1),
        )
        .unwrap()
}

/// Pin 2 low, PWM 3 off, pin 4 on (`on_fault` 1 in bits 9–10), pin 5 held (`on_fault` 2).
const A: &str = "(call $gw (i32.const 2) (i32.const 1))
    (call $pw (i32.const 3) (i32.const 100))
    (call $gw (i32.const 516) (i32.const 1))
    (call $gw (i32.const 1029) (i32.const 1))";
const B: &str = "(call $gw (i32.const 8) (i32.const 1))";

fn a_safe() -> Vec<(u8, PinUse, Fault)> {
    vec![
        (2, PinUse::Out, Fault::Low),
        (3, PinUse::Pwm, Fault::Low),
        (4, PinUse::Out, Fault::High),
    ]
}

#[test]
fn trap_and_unload_put_only_own_outputs_into_safe_state() {
    let hal = StubHal::default();
    let mut k: Kernel<Module, Board> = Kernel::new();
    let a = k.add(load(&hal, A), &[], 0).unwrap();
    let b = k.add(load(&hal, B), &[], 0).unwrap();
    k.run_quantum(10).unwrap();
    assert_eq!(k.get(a).unwrap().host().leases.len(), 4);
    assert!(hal.take().is_empty());

    k.get_mut(a).unwrap().host_mut().inputs = vec![Word::int(1)];
    let err = k.run_quantum(10).unwrap_err();
    assert!(err.contains("остановлен"), "{err}");
    assert_eq!(hal.take(), a_safe(), "hold pin 5 and module B untouched");
    assert!(k.get(a).unwrap().host().leases.is_empty());
    assert!(k.get(b).unwrap().host().leases.holds(8));

    // The module runs again, takes its outputs back; unloading it ends them again.
    k.get_mut(a).unwrap().host_mut().inputs = vec![Word::int(0)];
    k.run_quantum(10).unwrap();
    assert!(hal.take().is_empty());
    assert!(k.unload(a).is_some());
    assert_eq!(hal.take(), a_safe());
    assert!(k.get(a).is_none());

    k.unload(b);
    assert_eq!(hal.take(), vec![(8, PinUse::Out, Fault::Low)]);
}

#[test]
fn swap_hands_leases_over_and_drops_old_messages() {
    let hal = StubHal::default();
    let mut k: Kernel<Module, Board> = Kernel::new();
    let old = k.add(load(&hal, A), &[], 0).unwrap();
    k.run_quantum(10).unwrap();
    k.publish("e", b"for old");

    let new = k.swap(old, load(&hal, A), &[], 0).unwrap();
    assert!(hal.take().is_empty(), "no output changes on swap");
    assert!(k.get(old).is_none(), "old handle is stale");
    assert_eq!(k.handles(), [new]);
    let leases = &k.get(new).unwrap().host().leases;
    assert!([2, 3, 4, 5].iter().all(|&p| leases.holds(p)));

    k.run_quantum(10).unwrap();
    let out = k.get(new).unwrap().host().outputs[0];
    assert_eq!(
        out,
        Word::int(0),
        "the message for the old module did not arrive"
    );
    k.publish("e", b"for new");
    k.run_quantum(10).unwrap();
    assert_eq!(k.get(new).unwrap().host().outputs[0], Word::int(1));
    assert!(k.get(new).unwrap().host().devices.writes > 0);
}

#[test]
fn module_without_permission_is_refused_at_load() {
    let hal = StubHal::default();
    let board = Board { hal, writes: 0 };
    let err = Wasmi::new()
        .load(
            &wat(A),
            &["gpio_write".into()],
            Limits::default(),
            ModuleHost::new(board, 1),
        )
        .err()
        .unwrap();
    assert!(err.contains("crab.pwm_write"), "{err}");
}

/// Fuel as the emitter spends it: one at the start of `step` and on every back edge.
const SPEND: &str = r#"(func $spend
    (global.set $fuel (i32.sub (global.get $fuel) (i32.const 1)))
    (if (i32.lt_s (global.get $fuel) (i32.const 1))
      (then (global.set $fuel (call $yield)))))"#;

/// `step` never returns: it counts loop passes into output 0.
fn endless() -> Vec<u8> {
    wat::parse_str(format!(
        r#"(module
  (import "crab" "output_i" (func $out (param i32 i32) (result i32)))
  (import "crab" "yield" (func $yield (result i32)))
  (global $fuel (mut i32) (i32.const 10000))
  (global $n (mut i32) (i32.const 0))
  {SPEND}
  (func (export "abi_version") (result i32) i32.const 4)
  (func (export "step") (param i32)
    call $spend
    (loop $l
      call $spend
      (global.set $n (i32.add (global.get $n) (i32.const 1)))
      (drop (call $out (i32.const 0) (global.get $n)))
      br $l)))"#
    ))
    .unwrap()
}

/// Counts its steps into output 0.
fn counter(abi: i32) -> Vec<u8> {
    wat::parse_str(format!(
        r#"(module
  (import "crab" "output_i" (func $out (param i32 i32) (result i32)))
  (import "crab" "yield" (func $yield (result i32)))
  (global $fuel (mut i32) (i32.const 10000))
  (global $n (mut i32) (i32.const 0))
  {SPEND}
  (func (export "abi_version") (result i32) i32.const {abi})
  (func (export "step") (param i32)
    call $spend
    (global.set $n (i32.add (global.get $n) (i32.const 1)))
    (drop (call $out (i32.const 0) (global.get $n)))))"#
    ))
    .unwrap()
}

fn load_wasm(wasm: &[u8], hal: &StubHal) -> Result<Module, String> {
    let board = Board {
        hal: hal.clone(),
        writes: 0,
    };
    Wasmi::new().load(wasm, &[], Limits::default(), ModuleHost::new(board, 1))
}

#[test]
fn endless_loop_yields_and_continues_where_it_stopped() {
    let hal = StubHal::default();
    let mut k: Kernel<Module, Board> = Kernel::new();
    let hung = k.add(load_wasm(&endless(), &hal).unwrap(), &[], 0).unwrap();
    let other = k
        .add(load_wasm(&counter(4), &hal).unwrap(), &[], 0)
        .unwrap();

    let out = |k: &Kernel<Module, Board>, h| k.get(h).unwrap().host().outputs[0].0;
    let mut passes = 0;
    for quantum in 1..=5u32 {
        k.run_quantum(10).unwrap();
        let now = out(&k, hung);
        assert!(k.get(hung).unwrap().suspended(), "still in its first step");
        assert!(now > passes, "the loop went on from where it stopped");
        passes = now;
        assert_eq!(out(&k, other), quantum, "the other module got its quantum");
    }
    // 10000 fuel per quantum; the first one also spent one at the start of `step` and one on
    // the pass that yields (it finishes after resuming).
    assert_eq!(passes, 5 * 10_000 - 2);
}

#[test]
fn old_abi_is_refused_with_rebuild_hint() {
    let err = load_wasm(&counter(3), &StubHal::default()).err().unwrap();
    assert!(
        err.contains("ABI 3") && err.contains("пересоберите модуль"),
        "{err}"
    );
}
