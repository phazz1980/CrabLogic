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

/// Reads an output of the module by its label.
type Reader = Box<dyn Fn(&McuModule, &str) -> Word>;

/// Builds the one-sheet project, loads it, and gives the module with the output reader.
fn load_module(name: &str, logic: &str) -> (PathBuf, McuModule, Reader) {
    let lib = library();
    let (dir, project) = project(name, logic);
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    let module = McuModule::load(&built.wasm, &manifest, built.watched.len()).unwrap();
    let watched = built.watched.clone();
    let out = move |m: &McuModule, label: &str| {
        let i = watched.iter().position(|l| l == label).unwrap();
        m.outputs()[i]
    };
    (dir, module, Box::new(out))
}

const RESET_PIN: &str = "[[block]]\nid = \"rst\"\ntype = \"digital_in\"\npin = 12\n\n\
                         [[link]]\nfrom = \"rst.out\"\nto = \"cnt.reset\"\n";

#[test]
fn fast_counter_driver_counts_and_resets() {
    let logic = format!(
        "[[block]]\nid = \"cnt\"\ntype = \"fast_counter\"\npin = 4\nedge = 2\nfilter = 7\n\n{RESET_PIN}"
    );
    let (dir, mut module, out) = load_module("fast_counter_driver", &logic);
    module.io_mut().count = 1234;
    module.step(10).unwrap();
    assert_eq!(out(&module, "cnt.count"), Word::int(1234));
    assert_eq!(out(&module, "cnt.ok"), Word::int(1));
    let open = *module
        .io_mut()
        .bus_calls
        .iter()
        .find(|c| c[0] == 70)
        .unwrap();
    // Counter on pin 4, no direction pin (-1), both edges, 7 µs filter.
    assert_eq!(open[1..], [4, -1, 2, 7]);
    // The hidden direction pin is not claimed; the counted pin is.
    let claims: Vec<_> = module
        .io_mut()
        .bus_calls
        .iter()
        .filter(|c| c[0] == 0)
        .map(|c| c[2])
        .collect();
    assert!(claims.contains(&4) && !claims.contains(&-1), "{claims:?}");

    module.io_mut().gpio_in.insert(12, 1);
    module.step(10).unwrap();
    assert_eq!(out(&module, "cnt.count"), Word::int(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fast_counter_direction_pin_is_claimed_only_when_enabled() {
    let logic = "[[block]]\nid = \"cnt\"\ntype = \"fast_counter\"\npin = 4\ndirection = true\ndir_pin = 5\n";
    let (dir, mut module, _) = load_module("fast_counter_dir", logic);
    module.step(10).unwrap();
    let calls = module.io_mut().bus_calls.clone();
    let open = calls.iter().find(|c| c[0] == 70).unwrap();
    assert_eq!(open[1..3], [4, 5]);
    let claims: Vec<_> = calls.iter().filter(|c| c[0] == 0).map(|c| c[2]).collect();
    assert!(claims.contains(&4) && claims.contains(&5), "{claims:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn frequency_driver_uses_the_period_and_the_count() {
    let logic = "[[block]]\nid = \"cnt\"\ntype = \"frequency\"\npin = 4\nwindow = \"100ms\"\n";
    let (dir, mut module, out) = load_module("frequency_driver", logic);
    // Mid-window nothing is updated yet.
    module.step(50).unwrap();
    assert_eq!(out(&module, "cnt.hz"), Word::float(0.0));
    // 3 timed edges of 10 ms: 100 Hz.
    module.io_mut().count = 3;
    module.io_mut().count_period = (3, 30_000);
    module.step(50).unwrap();
    assert_eq!(out(&module, "cnt.hz"), Word::float(100.0));
    assert_eq!(out(&module, "cnt.count"), Word::int(3));
    assert_eq!(out(&module, "cnt.ok"), Word::int(1));
    let open = *module
        .io_mut()
        .bus_calls
        .iter()
        .find(|c| c[0] == 70)
        .unwrap();
    // The edge times are kept (0x100).
    assert_eq!(open[1..], [4, -1, 0x100, 0]);
    // Fast edges: the master could not time them, the count over the window decides.
    module.io_mut().count = 3 + 2000;
    module.step(100).unwrap();
    assert_eq!(out(&module, "cnt.hz"), Word::float(20_000.0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn quad_encoder_driver_gives_position_direction_and_resets() {
    let logic =
        "[[block]]\nid = \"cnt\"\ntype = \"quad_encoder\"\npin_a = 4\npin_b = 5\nmode = 2\n\n"
            .to_owned()
            + RESET_PIN;
    let (dir, mut module, out) = load_module("quad_driver", &logic);
    module.io_mut().count = 4;
    module.step(10).unwrap();
    assert_eq!(out(&module, "cnt.pos"), Word::int(4));
    assert_eq!(out(&module, "cnt.dir"), Word::int(1));
    let open = *module
        .io_mut()
        .bus_calls
        .iter()
        .find(|c| c[0] == 70)
        .unwrap();
    // x4 is mode 5, filter 1 µs by default.
    assert_eq!(open[1..], [4, 5, 5, 1]);
    module.io_mut().count = 2;
    module.step(10).unwrap();
    assert_eq!(out(&module, "cnt.dir"), Word::int(-1));
    // Rest for 200 ms: stands.
    module.step(200).unwrap();
    assert_eq!(out(&module, "cnt.pos"), Word::int(2));
    assert_eq!(out(&module, "cnt.dir"), Word::int(0));
    module.io_mut().gpio_in.insert(12, 1);
    module.step(10).unwrap();
    assert_eq!(out(&module, "cnt.pos"), Word::int(0));
    assert_eq!(out(&module, "cnt.dir"), Word::int(0));
    let _ = std::fs::remove_dir_all(&dir);
}
