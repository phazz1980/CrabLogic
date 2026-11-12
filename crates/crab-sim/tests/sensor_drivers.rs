//! The sensor drivers of the library (`drivers/`, `library/<name>/block.wasm`) inside a sheet
//! module: DHT22 decodes the levels the master measured.

use std::path::{Path, PathBuf};

use crab_codegen::Program;
use crab_core::{Library, Project};
use crab_sim::mcu::{Manifest, McuModule, Word};

fn library() -> Library {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let (lib, errors) = Library::load(&dir).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    lib
}

fn project(name: &str, logic: &str) -> (PathBuf, Project) {
    let dir = std::env::temp_dir().join(format!("crab_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("project.toml"),
        format!("[project]\nname = \"{name}\"\n\n[board]\nfqbn = \"esp32:esp32:esp32\"\n"),
    )
    .unwrap();
    std::fs::write(dir.join("logic.toml"), logic).unwrap();
    let project = Project::load(&dir).unwrap();
    (dir, project)
}

/// Levels of a DHT22 answer: released line, 80 µs low and high, then 40 bits.
fn dht_levels(data: [u8; 5]) -> Vec<u16> {
    let mut levels = vec![30, 80, 80];
    for byte in data {
        for bit in (0..8).rev() {
            levels.push(50);
            levels.push(if byte >> bit & 1 != 0 { 70 } else { 27 });
        }
    }
    levels.push(50);
    levels
}

#[test]
fn dht22_driver_decodes_the_answer() {
    let lib = library();
    let (dir, project) = project(
        "dht22_driver",
        "[[block]]\nid = \"air\"\ntype = \"dht22\"\npin = 4\n",
    );
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    let mut module = McuModule::load(&built.wasm, &manifest, built.watched.len()).unwrap();
    let out = |module: &McuModule, label: &str| {
        let i = built.watched.iter().position(|l| l == label).unwrap();
        module.outputs()[i]
    };

    // 65.2 % and -10.1 °C: 0x028C, 0x8065.
    let data = [
        0x02,
        0x8C,
        0x80,
        0x65,
        0x02u8
            .wrapping_add(0x8C)
            .wrapping_add(0x80)
            .wrapping_add(0x65),
    ];
    module.io_mut().pulse_in = dht_levels(data);
    module.step(10).unwrap();
    assert_eq!(out(&module, "air.ok"), Word::int(1));
    assert_eq!(out(&module, "air.humidity"), Word::float(65.2));
    assert_eq!(out(&module, "air.temp"), Word::float(-10.1));
    let read = *module
        .io_mut()
        .bus_calls
        .iter()
        .find(|c| c[0] == 65)
        .unwrap();
    assert_eq!((read[1], read[2]), (4, 1100));

    // A bad checksum fails the reading; the next one waits for the period (2 s).
    let mut bad = data;
    bad[4] ^= 1;
    module.io_mut().pulse_in = dht_levels(bad);
    module.step(2000).unwrap();
    assert_eq!(out(&module, "air.ok"), Word::int(0));
    assert_eq!(out(&module, "air.humidity"), Word::float(65.2));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ds18b20_driver_reports_a_missing_sensor() {
    let lib = library();
    let (dir, project) = project(
        "ds18b20_driver",
        "[[block]]\nid = \"water\"\ntype = \"ds18b20\"\npin = 5\n",
    );
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    wasmparser::Validator::new()
        .validate_all(&built.wasm_aot)
        .unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    let mut module = McuModule::load(&built.wasm, &manifest, built.watched.len()).unwrap();
    module.step(10).unwrap();
    let i = built.watched.iter().position(|l| l == "water.ok").unwrap();
    assert_eq!(module.outputs()[i], Word::int(0));
    // The stub bus answers no reset: the driver asked for one on its pin.
    assert!(module
        .io_mut()
        .bus_calls
        .iter()
        .any(|c| c[1] == 5 && c[0] != 0));
    let _ = std::fs::remove_dir_all(&dir);
}
