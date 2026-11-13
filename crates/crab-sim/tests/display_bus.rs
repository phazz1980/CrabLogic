//! Display chips on the stub I²C bus: bytes a module writes with `bus` op 21 reach the model
//! at that address, a module sees no acknowledge from an address nobody answers.

use crab_sim::mcu::{Manifest, McuModule, StubIo};
use crab_sim::{DisplayDevice, DisplayKind};

/// Writes `data` to address 0x3C and 0x77 (nobody there); out0 is not used, the results are
/// kept in memory words 64 and 68 ... via tag_set_i.
const MODULE: &str = r#"(module
  (import "crab" "bus" (func $bus (param i32 i32 i32 i32 i32) (result i32)))
  (import "crab" "tag_set_i" (func $set (param i32 i32)))
  (memory (export "memory") 1)
  (data (i32.const 32) "\00\b0\02\10")
  (data (i32.const 48) "\40\01\80")
  (func (export "abi_version") (result i32) i32.const 4)
  (func (export "init")
    (drop (call $bus (i32.const 20) (i32.const 21) (i32.const 22) (i32.const 400000) (i32.const 0))))
  (func (export "step") (param i32)
    (drop (call $bus (i32.const 21) (i32.const 60) (i32.const 32) (i32.const 4) (i32.const 0)))
    (call $set (i32.const 0)
      (call $bus (i32.const 21) (i32.const 60) (i32.const 48) (i32.const 3) (i32.const 0)))
    (call $set (i32.const 1)
      (call $bus (i32.const 21) (i32.const 119) (i32.const 48) (i32.const 3) (i32.const 0)))))"#;

#[test]
fn module_writes_reach_the_display_at_its_address() {
    let manifest = Manifest {
        allow: vec!["bus".to_owned()],
        ..Manifest::parse("").unwrap()
    };
    let mut io = StubIo::default();
    io.displays.push(DisplayDevice::ssd1306("oled", 0x3C, 64));
    let mut m = McuModule::load_with(&wat::parse_str(MODULE).unwrap(), &manifest, 1, io).unwrap();
    m.step(10).unwrap();
    assert_eq!(m.tag(0).map(|w| w.as_int()), Some(3));
    assert_eq!(m.tag(1).map(|w| w.as_int()), Some(-1));
    let views = m.displays();
    assert_eq!(views.len(), 1);
    let v = &views[0];
    assert_eq!(v.kind, DisplayKind::Ssd1306);
    // Page 0 (0xB0), column 2 (0x02, 0x10), then data 0x01 and 0x80: the second write continues.
    assert!(v.pixels[2]);
    assert!(v.pixels[3 + 7 * 128]);
}
