//! `examples/wasm_driver`: the BMP280 driver (Rust, `library/bmp280/block.wasm`) runs in the
//! sheet module against a stub chip with the calibration and readings of the BMP280 datasheet
//! example (25.08 °C, 100653 Pa).

use std::path::PathBuf;

use crab_codegen::Program;
use crab_core::{Library, Project};
use crab_sim::mcu::{Manifest, McuModule};

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// Register file of a BMP280 with the datasheet's example calibration and raw values.
fn bmp280() -> Vec<u8> {
    let mut regs = vec![0u8; 256];
    regs[0xD0] = 0x58;
    let calib: [i32; 12] = [
        27504, 26435, -1000, 36477, -10685, 3024, 2855, 140, -7, 15500, -14600, 6000,
    ];
    for (k, v) in calib.iter().enumerate() {
        let b = (*v as u16).to_le_bytes();
        regs[0x88 + 2 * k] = b[0];
        regs[0x88 + 2 * k + 1] = b[1];
    }
    let raw = |adc: u32| {
        [
            (adc >> 12) as u8,
            (adc >> 4) as u8,
            ((adc & 0xF) << 4) as u8,
        ]
    };
    regs[0xF7..0xFA].copy_from_slice(&raw(415_148));
    regs[0xFA..0xFD].copy_from_slice(&raw(519_888));
    regs
}

#[test]
fn bmp280_example_reads_the_datasheet_values() {
    let (mut lib, errors) = Library::load(repo_path("library")).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    let errors = lib
        .add_dir(repo_path("examples/wasm_driver/library"))
        .unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    let project = Project::load(repo_path("examples/wasm_driver")).unwrap();
    let report = crab_core::check_project(&project, &lib);
    assert!(
        report.errors().next().is_none(),
        "{:#?}",
        report.diagnostics
    );
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    let mut module = McuModule::load(&built.wasm, &manifest, built.watched.len()).unwrap();
    module.io_mut().i2c_regs = bmp280();
    // Before `init` reads anything the chip is there; the first step starts a measurement,
    // the next one (10 ms later) reads it.
    module.step(10).unwrap();
    module.step(10).unwrap();
    let out = |label: &str| {
        let i = built.watched.iter().position(|l| l == label).unwrap();
        module.outputs()[i]
    };
    let t = f32::from_bits(out("air.temperature").0);
    let p = f32::from_bits(out("air.pressure").0);
    assert!((t - 25.08).abs() < 0.001, "{t}");
    assert!((p - 1006.5327).abs() < 0.01, "{p}");
    assert_eq!(out("air.ok").0, 1);
    assert_eq!(out("hot.out").0, 0);
    // Claims: address, SDA, SCL; then the lock.
    let calls = &module.io_mut().bus_calls;
    assert_eq!(
        &calls[..4],
        &[
            [0, 2, 118, 0, 0],
            [0, 1, 21, 0, 0],
            [0, 1, 22, 0, 0],
            [1, 0, 0, 0, 0]
        ]
    );
    assert_eq!(calls[4], [20, 21, 22, 400_000, 0]);
}
