//! Modules written by `crab-codegen` give the same numbers as the simulation of the schema.

use std::path::PathBuf;

use crab_codegen::Program;
use crab_core::{Library, Project, SignalType};
use crab_sim::mcu::{Manifest, McuModule, Word};
use crab_sim::schema::Simulation;

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn library() -> Library {
    let (lib, errors) = Library::load(repo_path("library")).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    lib
}

/// The carrier value `expected` of the signal `label` as the 32-bit word the module must hold.
fn word(program: &Program, label: &str, expected: f64) -> Word {
    let name = label.replace('.', "_");
    let ty = program
        .signals
        .iter()
        .find(|s| s.name == name)
        .map_or(SignalType::Float, |s| s.ty);
    if ty == SignalType::Float {
        Word::float(expected as f32)
    } else {
        Word::int(expected as i32)
    }
}

/// Runs the sheet as a module next to the simulation and compares every output after every step.
fn compare(project_dir: &str, outputs: usize, steps: u32, dt: u32) {
    let lib = library();
    let project = Project::load(repo_path(project_dir)).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    let mut module = McuModule::load(&built.wasm, &manifest, built.watched.len()).unwrap();
    let mut sim = Simulation::new(&project.logic, &lib).unwrap();
    assert!(built.watched.len() == outputs, "{:?}", built.watched);
    for step in 0..steps {
        sim.step(dt).unwrap();
        module.step(dt).unwrap();
        for (i, label) in built.watched.iter().enumerate() {
            let (block, port) = label.split_once('.').unwrap();
            let expected = word(&program, label, sim.output(block, port).unwrap());
            let got = module.outputs()[i];
            assert_eq!(got, expected, "step {step}: {label}");
        }
    }
}

#[test]
fn stateless_blocks_match_the_simulation() {
    compare("crates/crab-sim/tests/fixtures/wasm_logic", 20, 40, 100);
}

#[test]
fn timers_and_memory_blocks_match_the_simulation() {
    compare("crates/crab-sim/tests/fixtures/wasm_timers", 10, 120, 50);
}

#[test]
fn pins_match_the_simulation() {
    let lib = library();
    let project = Project::load(repo_path("crates/crab-sim/tests/fixtures/wasm_hw")).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    assert_eq!(
        manifest.allow,
        ["gpio_read", "gpio_write", "adc_read", "pwm_write"]
    );
    let mut module = McuModule::load(&built.wasm, &manifest, built.watched.len()).unwrap();
    let mut sim = Simulation::new(&project.logic, &lib).unwrap();
    for step in 0..120u32 {
        // The button is pressed (pulled low) for a while; the level sweeps over the ADC range.
        let pressed = (30..70).contains(&step);
        let counts = (step * 53) % 4400;
        sim.bench_mut()
            .set_level(4, if pressed { 0.0 } else { 1.0 });
        sim.bench_mut().set_level(34, f64::from(counts));
        module.io_mut().gpio_in.insert(4, i32::from(!pressed));
        module.io_mut().adc_in.insert(34, counts as i32);
        sim.step(50).unwrap();
        module.step(50).unwrap();
        for (i, label) in built.watched.iter().enumerate() {
            let (block, port) = label.split_once('.').unwrap();
            let expected = word(&program, label, sim.output(block, port).unwrap());
            assert_eq!(module.outputs()[i], expected, "step {step}: {label}");
        }
        let io = module.io();
        for pin in [2_i64, 5] {
            let want = sim.bench().driven(pin).unwrap_or(0.0) as i32;
            assert_eq!(
                io.gpio_out.get(&(pin as i32)).copied().unwrap_or(0),
                want,
                "step {step}: pin {pin}"
            );
        }
        // Rounded half away from zero, in 32 bits like the module.
        let duty = sim.bench().driven(18).unwrap_or(0.0) as f32;
        let want = (duty * 2.55f32).round() as i32;
        assert_eq!(
            io.pwm_out.get(&18).copied().unwrap_or(0),
            want,
            "step {step}: pwm"
        );
    }
}

#[test]
fn on_fault_goes_to_the_master_with_the_pin() {
    let lib = library();
    let mut project = Project::load(repo_path("crates/crab-sim/tests/fixtures/wasm_hw")).unwrap();
    let built = Program::build(&project, &lib)
        .unwrap()
        .wasm_module(1)
        .unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    let mut module = McuModule::load(&built.wasm, &manifest, 0).unwrap();
    module.step(50).unwrap();
    // Off by default: the inverted lamp goes high, the others low (no bits, any master runs them).
    let faults = |m: &McuModule| [2, 5, 18].map(|p| m.io().on_fault.get(&p).copied());
    assert_eq!(faults(&module), [Some(1), Some(0), Some(0)]);
    assert_eq!(built.needs, ["abi4", "on_fault"]);

    let set = |project: &mut Project, id: &str, v: i64| {
        let block = project
            .logic
            .blocks
            .iter_mut()
            .find(|b| b.id == id)
            .unwrap();
        block
            .params
            .insert("on_fault".into(), crab_core::ParamValue::Int(v));
    };
    set(&mut project, "lamp", 1);
    set(&mut project, "fan", 2);
    set(&mut project, "dim", 1);
    let built = Program::build(&project, &lib)
        .unwrap()
        .wasm_module(1)
        .unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    let mut module = McuModule::load(&built.wasm, &manifest, 0).unwrap();
    module.step(50).unwrap();
    assert_eq!(faults(&module), [Some(0), Some(2), Some(1)]);

    // Every output off by default and not inverted: the module needs no new master.
    set(&mut project, "lamp", 0);
    let lamp = project
        .logic
        .blocks
        .iter_mut()
        .find(|b| b.id == "lamp")
        .unwrap();
    lamp.params
        .insert("inverted".into(), crab_core::ParamValue::Bool(false));
    set(&mut project, "fan", 0);
    set(&mut project, "dim", 0);
    let built = Program::build(&project, &lib)
        .unwrap()
        .wasm_module(1)
        .unwrap();
    assert_eq!(built.needs, ["abi4"], "no on_fault");
}

#[test]
fn blink_example_drives_the_led_like_the_simulation() {
    let lib = library();
    let project = Project::load(repo_path("examples/blink")).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module(1).unwrap();
    assert!(built.manifest.contains("allow = [\"gpio_write\"]"));
    let manifest = Manifest::parse(&built.manifest).unwrap();
    let mut module = McuModule::load(&built.wasm, &manifest, 0).unwrap();
    let mut sim = Simulation::new(&project.logic, &lib).unwrap();
    let mut levels = std::collections::BTreeSet::new();
    for step in 0..60 {
        sim.step(50).unwrap();
        module.step(50).unwrap();
        let want = sim.bench().driven(2).unwrap_or(0.0) as i32;
        let got = module.io().gpio_out.get(&2).copied().unwrap_or(0);
        assert_eq!(got, want, "step {step}");
        levels.insert(got);
    }
    assert_eq!(levels.len(), 2, "the led must blink");
}

#[test]
fn numeric_edges_match_the_simulation() {
    // `n` starts 3 below i32::MAX and wraps; division by zero, remainders and mixed operands.
    compare("crates/crab-sim/tests/fixtures/wasm_edges", 13, 12, 100);
}

#[test]
fn sensors_read_what_the_master_reports() {
    let lib = library();
    let project = Project::load(repo_path("crates/crab-sim/tests/fixtures/wasm_sensors")).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    assert_eq!(manifest.allow, ["sensor_read"]);
    let mut module = McuModule::load(&built.wasm, &manifest, built.watched.len()).unwrap();
    let out = |module: &McuModule, label: &str| {
        let i = built.watched.iter().position(|l| l == label).unwrap();
        module.outputs()[i]
    };
    // Keys are `kind | index << 4 | pin << 8`; fields: 0 temperature, 1 humidity, 2 read ok.
    let sensors = &mut module.io_mut().sensors;
    sensors.insert((4 << 8, 0), 21.5);
    sensors.insert((4 << 8, 2), 1.0);
    sensors.insert((4 << 8 | 1 << 4, 0), -3.25);
    sensors.insert((4 << 8 | 1 << 4, 2), 1.0);
    sensors.insert((1 | 5 << 8, 0), 24.0);
    sensors.insert((1 | 5 << 8, 1), 55.5);
    sensors.insert((1 | 5 << 8, 2), 1.0);
    module.step(50).unwrap();
    assert_eq!(out(&module, "t1.temp"), Word::float(21.5));
    assert_eq!(out(&module, "t1.ok"), Word::int(1));
    assert_eq!(out(&module, "t2.temp"), Word::float(-3.25));
    assert_eq!(out(&module, "air.temp"), Word::float(24.0));
    assert_eq!(out(&module, "air.humidity"), Word::float(55.5));
    assert_eq!(out(&module, "air.ok"), Word::int(1));
    // A failed read: the master keeps the last temperature and drops `ok`.
    module.io_mut().sensors.insert((4 << 8, 2), 0.0);
    module.step(50).unwrap();
    assert_eq!(out(&module, "t1.temp"), Word::float(21.5));
    assert_eq!(out(&module, "t1.ok"), Word::int(0));
    assert_eq!(out(&module, "t2.ok"), Word::int(1));
}

#[test]
fn pcf8574_goes_through_the_master() {
    let lib = library();
    let project = Project::load(repo_path("crates/crab-sim/tests/fixtures/wasm_pcf")).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    assert_eq!(manifest.allow, ["pcf_read", "pcf_write"]);
    let mut module = McuModule::load(&built.wasm, &manifest, built.watched.len()).unwrap();
    let out = |module: &McuModule, label: &str| {
        let i = built.watched.iter().position(|l| l == label).unwrap();
        module.outputs()[i]
    };
    // Keys are `address | sda << 8 | scl << 16`; fields: 0 the byte, 1 the chip answered.
    let inputs = 0x22 | 21 << 8 | 22 << 16;
    let relays = 0x24 | 21 << 8 | 22 << 16;
    // Optocouplers pull to ground: P1 low and the rest high means "second input pressed".
    module.io_mut().pcf_in.insert((inputs, 0), 0b1111_1101);
    module.io_mut().pcf_in.insert((inputs, 1), 1);
    module.step(50).unwrap();
    assert_eq!(out(&module, "inputs.p0"), Word::int(0));
    assert_eq!(out(&module, "inputs.p1"), Word::int(1));
    assert_eq!(out(&module, "inputs.ok"), Word::int(1));
    // Inverted relays: P3 on is a low bit, the rest stay high.
    assert_eq!(module.io().pcf_out[&relays], 0b1111_0111);
    module.io_mut().pcf_in.insert((inputs, 0), 0b1111_1110);
    module.step(50).unwrap();
    assert_eq!(module.io().pcf_out[&relays], 0b1111_1110);
}

#[test]
fn modbus_rtu_goes_through_the_master() {
    let lib = library();
    let project = Project::load(repo_path("examples/modbus_rtu")).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module(1).unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    assert_eq!(manifest.allow, ["modbus"]);
    let mut module = McuModule::load(&built.wasm, &manifest, 0).unwrap();
    // Master reads: slave 10, input table (1), float (1), master (1); the coil of the panel is a slave read.
    let temp = (10, 1 | 1 << 4 | 1 << 8, 0);
    let coil = (0, 2 | 2 << 4, 0);
    module
        .io_mut()
        .modbus_in
        .insert(temp, 21.5f32.to_bits() as i32);
    module.io_mut().modbus_in.insert(coil, 1);
    module.io_mut().modbus_status.insert(10, 1);
    module.step(50).unwrap();
    let io = module.io();
    // Both roles are set up once, each list starting with its role (code 0) and ending with code 63.
    let roles: Vec<_> = io.modbus_cfg.iter().filter(|(c, _)| *c == 0).collect();
    assert_eq!(roles, [&(0, 1), &(0, 2)]);
    assert_eq!(io.modbus_cfg.iter().filter(|(c, _)| *c == 63).count(), 2);
    assert!(
        io.modbus_cfg.contains(&(2, 9600)),
        "master speed from UART2"
    );
    assert!(
        io.modbus_cfg.contains(&(2, 19200)),
        "slave speed from UART1"
    );
    assert!(io.modbus_cfg.contains(&(16, 10)) && io.modbus_cfg.contains(&(24, 11)));
    // The temperature goes to the panel's input registers, the relay command to slave 11.
    assert_eq!(io.modbus_out[&(0, 1 | 1 << 4, 0)], 21.5f32.to_bits() as i32);
    assert_eq!(io.modbus_out[&(0, 3 | 2 << 4, 0)], 1);
    assert_eq!(io.modbus_out[&(11, 2 | 2 << 4 | 1 << 8, 0)], 1);
    let n = module.io().modbus_cfg.len();
    module.step(50).unwrap();
    assert_eq!(module.io().modbus_cfg.len(), n, "settings are sent once");
}

#[test]
fn modbus_tcp_master_goes_through_the_master() {
    let lib = library();
    let project = Project::load(repo_path("examples/modbus_tcp_master")).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module(1).unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    assert!(manifest.allow.contains(&"modbus".to_string()));
    let mut module = McuModule::load(&built.wasm, &manifest, 0).unwrap();
    // Unit 1, input table (1), float (1), master (1), TCP (bit 9).
    let tcp = 1 << 8 | 1 << 9;
    let temp = (1, 1 | 1 << 4 | tcp, 0);
    module
        .io_mut()
        .modbus_in
        .insert(temp, 21.5f32.to_bits() as i32);
    module.io_mut().modbus_status.insert(1, 1);
    module.step(50).unwrap();
    let io = module.io();
    assert!(io.modbus_cfg.contains(&(0, 3)), "TCP master role");
    assert!(io.modbus_cfg.contains(&(1, 0)), "Wi-Fi interface");
    assert!(io.modbus_cfg.contains(&(2, 502)), "port");
    // 192.168.1.50 with the first octet in the lowest byte.
    assert!(io
        .modbus_cfg
        .contains(&(3, i32::from_le_bytes([192, 168, 1, 50]))));
    assert!(io.modbus_cfg.contains(&(16, 1)), "Unit ID");
    assert_eq!(io.modbus_cfg.iter().filter(|(c, _)| *c == 63).count(), 1);
    // The button is inverted with a pull-up: released reads as 1 on the coil.
    assert_eq!(io.modbus_out[&(1, 2 | 2 << 4 | tcp, 0)], 1);
}

#[test]
fn sensors_example_switches_pins_from_sensor_values() {
    let lib = library();
    let project = Project::load(repo_path("examples/sensors")).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module(1).unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    assert_eq!(manifest.allow, ["gpio_write", "sensor_read"]);
    let mut module = McuModule::load(&built.wasm, &manifest, 0).unwrap();
    let (water, air) = (4 << 8, 1 | 15 << 8);
    let sensors = &mut module.io_mut().sensors;
    sensors.extend([((water, 0), 30.0), ((water, 2), 1.0)]);
    sensors.extend([((air, 0), 20.0), ((air, 1), 80.0), ((air, 2), 1.0)]);
    module.step(50).unwrap();
    let pin = |module: &McuModule, p: i32| module.io().gpio_out.get(&p).copied();
    assert_eq!(pin(&module, 26), Some(1), "humid: fan on");
    assert_eq!(pin(&module, 27), Some(1), "cold water: heater on");
    assert_eq!(pin(&module, 2), Some(0), "both sensors ok: no fault");
    module.io_mut().sensors.insert((air, 2), 0.0);
    module.step(50).unwrap();
    assert_eq!(pin(&module, 2), Some(1), "air sensor lost: fault lamp");
}

#[test]
fn uart_bytes_go_through_the_master() {
    let lib = library();
    let project = Project::load(repo_path("examples/uart_bytes")).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module(1).unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    assert_eq!(manifest.allow, ["uart"]);
    let mut module = McuModule::load(&built.wasm, &manifest, 0).unwrap();
    module
        .io_mut()
        .uart_rx
        .entry(2)
        .or_default()
        .extend([65, 200]);
    module.step(10).unwrap();
    module.step(10).unwrap();
    let io = module.io();
    // The port is opened once by each of its four blocks, with the speed and frame of the board.
    assert!(io.uart_cfg.contains(&(2, 1, 115200)));
    assert!(io.uart_cfg.contains(&(2, 2, 8)) && io.uart_cfg.contains(&(2, 5, 16)));
    assert_eq!(io.uart_cfg.iter().filter(|c| c.1 == 63).count(), 4);
    assert_eq!(
        io.uart_speed.get(&2),
        Some(&57600),
        "apply switches the speed"
    );
    // Echo: the bytes come back as one byte each; the scaled value goes out as a float.
    let tx = &io.uart_tx[&2];
    let bytes: Vec<_> = tx.iter().filter(|t| t.1 == 0).map(|t| t.0).collect();
    assert_eq!(bytes, [65, 200]);
    let floats: Vec<_> = tx
        .iter()
        .filter(|t| t.1 == 2)
        .map(|t| f32::from_bits(t.0 as u32))
        .collect();
    assert!(
        floats.len() >= 2 && (floats[0] - 25.49).abs() < 0.1,
        "{floats:?}"
    );
    let n = module.io().uart_cfg.len();
    module.step(10).unwrap();
    assert_eq!(module.io().uart_cfg.len(), n, "settings are sent once");
}

#[test]
fn uart_lines_and_strings_work_in_a_module() {
    let lib = library();
    let project = Project::load(repo_path("examples/text_wasm")).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    assert!(manifest.allow.iter().any(|a| a == "text"), "{manifest:?}");
    let mut module = McuModule::load(&built.wasm, &manifest, built.watched.len()).unwrap();
    let out = |module: &McuModule, label: &str| {
        let i = built.watched.iter().position(|l| l == label).unwrap();
        module.outputs()[i]
    };
    let rx = |module: &mut McuModule, text: &str| {
        module
            .io_mut()
            .uart_rx
            .entry(2)
            .or_default()
            .extend(text.bytes());
    };
    // One line per step, as in the C++ block; CR before LF is skipped.
    rx(&mut module, "25.55\r\nOn\n");
    module.step(10).unwrap();
    assert_eq!(module.io().uart_text[&2], b"T=25.5C\r\n");
    assert_eq!(out(&module, "len.out"), Word::int(5));
    module.step(10).unwrap();
    assert_eq!(module.io().uart_text[&2], b"T=25.5C\r\nT=25.5C\r\n");
    assert_eq!(out(&module, "is_on.out"), Word::int(1));
    assert_eq!(out(&module, "len.out"), Word::int(2));
    // A half line waits for its end.
    rx(&mut module, "7");
    module.step(10).unwrap();
    assert_eq!(out(&module, "len.out"), Word::int(2));
    rx(&mut module, "\n");
    module.step(10).unwrap();
    assert_eq!(out(&module, "len.out"), Word::int(1));
}

#[test]
fn text_blocks_match_the_simulation() {
    compare("crates/crab-sim/tests/fixtures/wasm_text", 19, 3, 100);
}

#[test]
fn wifi_and_ntp_work_in_a_module() {
    let lib = library();
    let project = Project::load(repo_path("examples/net_wasm")).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    assert!(manifest.allow.iter().any(|a| a == "net"), "{manifest:?}");
    let mut module = McuModule::load(&built.wasm, &manifest, built.watched.len()).unwrap();
    let out = |module: &McuModule, label: &str| {
        let i = built
            .watched
            .iter()
            .position(|l| l == label)
            .unwrap_or_else(|| panic!("{label}: {:?}", built.watched));
        module.outputs()[i]
    };
    module.step(10).unwrap();
    // Settings go out once, the radio request is the client every step.
    let io = module.io();
    assert_eq!(io.net_text[&1], "MyNetwork");
    assert_eq!(io.net_text[&2], "secret123");
    assert_eq!(io.net_text[&8], "pool.ntp.org");
    assert_eq!(io.net_num[&1], 1);
    assert_eq!(io.net_num[&2], 3_600_000);
    assert!(io.net_num.contains_key(&63) && io.net_num.contains_key(&64));
    assert_eq!(io.net_radio, 1);
    assert_eq!(out(&module, "clock.valid"), Word::int(0));
    assert_eq!(out(&module, "addr_len.out"), Word::int(0));
    assert_eq!(out(&module, "work.out"), Word::int(0));
    // Connected, time known: 2026-11-04 (Wednesday) 10:30:15 UTC, zone +3 -> 13:30:15.
    let unix = 1_793_788_215u32;
    let io = module.io_mut();
    io.net_wifi.insert(0, 1);
    io.net_wifi.insert(4, i32::from_le_bytes([192, 168, 1, 77]));
    io.net_unix = Some(unix);
    module.step(10).unwrap();
    assert_eq!(out(&module, "clock.valid"), Word::int(1));
    assert_eq!(out(&module, "clock.hour"), Word::int(13));
    assert_eq!(out(&module, "clock.minute"), Word::int(30));
    assert_eq!(out(&module, "clock.unix"), Word::int(unix as i32));
    assert_eq!(out(&module, "addr_len.out"), Word::int(12));
    assert_eq!(out(&module, "time_len.out"), Word::int(8));
    assert_eq!(out(&module, "work.out"), Word::int(1));
}

#[test]
fn ethernet_and_wifi_work_together_in_a_module() {
    let lib = library();
    let project = Project::load(repo_path("examples/net_eth_wasm")).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    assert!(manifest.allow.iter().any(|a| a == "net"), "{manifest:?}");
    let mut module = McuModule::load(&built.wasm, &manifest, built.watched.len()).unwrap();
    let out = |module: &McuModule, label: &str| {
        let i = built
            .watched
            .iter()
            .position(|l| l == label)
            .unwrap_or_else(|| panic!("{label}: {:?}", built.watched));
        module.outputs()[i]
    };
    module.step(10).unwrap();
    // The pins, the chip select and the MAC byte go to the master once; DHCP is on.
    let io = module.io();
    assert_eq!(io.eth_num[&2], 14);
    assert_eq!(io.eth_num[&3], 12);
    assert_eq!(io.eth_num[&4], 13);
    assert_eq!(io.eth_num[&5], 15);
    assert_eq!(io.eth_num[&6], 41);
    assert_eq!(io.eth_num[&1], 1);
    assert!(io.eth_num.contains_key(&63));
    assert!(!io.eth_text.contains_key(&1));
    // NTP asks for the Ethernet interface; Wi-Fi settings are independent.
    assert_eq!(io.net_num[&3], 1);
    assert_eq!(io.net_text[&1], "MyNetwork");
    assert_eq!(out(&module, "work.out"), Word::int(0));
    assert_eq!(out(&module, "eth_len.out"), Word::int(0));
    // Both up, time known.
    let io = module.io_mut();
    io.eth_state.insert(0, 1);
    io.eth_state.insert(1, i32::from_le_bytes([10, 0, 0, 5]));
    io.net_wifi.insert(0, 1);
    io.net_wifi.insert(4, i32::from_le_bytes([192, 168, 1, 77]));
    io.net_unix = Some(1_793_788_215);
    module.step(10).unwrap();
    assert_eq!(out(&module, "eth_len.out"), Word::int(8));
    assert_eq!(out(&module, "wifi_len.out"), Word::int(12));
    assert_eq!(out(&module, "work.out"), Word::int(1));
}

#[test]
fn mqtt_goes_through_the_master() {
    let lib = library();
    let project = Project::load(repo_path("examples/mqtt_wasm")).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    assert!(manifest.allow.iter().any(|a| a == "net"), "{manifest:?}");
    let mut module = McuModule::load(&built.wasm, &manifest, built.watched.len()).unwrap();
    module.step(10).unwrap();
    // Settings go out once, the filter is registered in the first slot.
    let io = module.io();
    assert_eq!(io.mqtt_text[&1], "broker.hivemq.com");
    assert_eq!(io.mqtt_num[&1], 1883);
    assert_eq!(io.mqtt_num[&4], 2);
    assert!(io.mqtt_num.contains_key(&63));
    assert_eq!(io.mqtt_filters, ["crab/led"]);
    // No link, nothing published.
    assert!(io.mqtt_published.is_empty());
    // A message arrives and the module echoes it back once the client is connected.
    let io = module.io_mut();
    io.mqtt_state.insert(0, 1);
    io.mqtt_receive("crab/led", "1");
    module.step(10).unwrap();
    module.step(10).unwrap();
    let io = module.io();
    assert_eq!(
        io.mqtt_published,
        [("crab/echo".to_string(), "1".to_string(), false)]
    );
}

#[test]
fn web_page_goes_through_the_master() {
    let lib = library();
    let project = Project::load(repo_path("examples/web_wasm")).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    assert!(manifest.allow.iter().any(|a| a == "net"), "{manifest:?}");
    let mut module = McuModule::load(&built.wasm, &manifest, built.watched.len()).unwrap();
    module.step(10).unwrap();
    let io = module.io();
    // The setup comes once, before any widget works: layout, table, interface, password.
    assert!(io.web_complete);
    // Wi-Fi (bit 1) and the «Система» page (bit 4).
    assert_eq!(io.web_mask, 5);
    assert_eq!(io.web_pass, "1234");
    assert!(
        io.web_layout.starts_with("{\"t\":\"Модуль\""),
        "{}",
        io.web_layout
    );
    assert!(io.web_layout.contains("\"k\":\"slider\""));
    assert!(io.web_layout.contains("\"k\":\"textin\""));
    assert!(io.web_layout.contains("\"y\":1"));
    assert!(io.web_layout.contains("\"k\":\"chart\""));
    // An image of a widget (`tank_2`) is not a widget of its own.
    let kinds: Vec<u8> = io.web_items.iter().map(|i| i.kind).collect();
    assert_eq!(
        kinds,
        [0, 0, 2, 0, 0, 4, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1]
    );
    // The network form: defaults from the block, one widget of the table; the saved ssid is shown.
    assert_eq!(io.web_net.len(), 10);
    assert_eq!(io.web_net[0], "home");
    assert_eq!(io.web_net[2], "1");
    // The slider keeps its initial value; the text input its initial text.
    assert_eq!(io.web_items[1].value, 25.0);
    assert_eq!(io.web_items[2].text, "CRAB");
    // The chart: 30 points and a period of 2 s go to the master, which samples the value itself.
    assert_eq!(io.web_items[5].points, 30);
    assert_eq!(io.web_items[5].value, 25.0);
    module.step(10).unwrap();
    let io = module.io();
    assert_eq!(io.web_items[3].value, 25.0, "value shown from the slider");
    assert_eq!(io.web_items[6].text, "Привет, CRAB");
    assert_eq!(io.web_items[17].text, "home", "ssid from the form");
    // The page changes the switch, the slider and the name.
    let io = module.io_mut();
    io.web_items[0].value = 1.0;
    io.web_items[1].value = 70.0;
    io.web_items[2].text = "Мир".to_string();
    module.step(10).unwrap();
    module.step(10).unwrap();
    let io = module.io();
    assert_eq!(io.web_items[3].value, 70.0);
    assert_eq!(io.web_items[4].value, 1.0, "led follows the switch");
    assert_eq!(io.web_items[6].text, "Привет, Мир");
}

#[test]
fn type_conversions_match_the_simulation() {
    // 40 steps of -0.5 pass the ties (-0.5, -1.5, ...) and whole values in every mode.
    let lib = library();
    let project = Project::load(repo_path("crates/crab-sim/tests/fixtures/wasm_convert")).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    assert!(built.watched.len() >= 5, "{:?}", built.watched);
    compare(
        "crates/crab-sim/tests/fixtures/wasm_convert",
        built.watched.len(),
        40,
        100,
    );
}

#[test]
fn parameters_read_from_inputs_match_the_simulation() {
    compare("crates/crab-sim/tests/fixtures/wasm_param_in", 10, 80, 50);
}

#[test]
fn generators_match_the_simulation_on_every_scan_step() {
    for dt in [10, 50, 100, 250, 1500] {
        compare("crates/crab-sim/tests/fixtures/wasm_generator", 7, 300, dt);
    }
}

/// A generator that stays enabled follows `(t % period) < on` whatever the scan step is,
/// also when a scan skips several phases (`specs/generator-deadline`).
#[test]
fn generator_keeps_the_phase_of_the_time_when_scans_skip_phases() {
    let lib = library();
    let project =
        Project::load(repo_path("crates/crab-sim/tests/fixtures/wasm_generator")).unwrap();
    let pairs = [
        ("g1", 100u32, 100u32),
        ("g2", 300, 500),
        ("g4", 13, 7),
        ("g7", 2, 1),
        ("g5", 1, 1),
    ];
    for dt in [10u32, 50, 250, 1500, 777] {
        let mut sim = Simulation::new(&project.logic, &lib).unwrap();
        let mut t = 0u32;
        for _ in 0..400 {
            sim.step(dt).unwrap();
            t += dt;
            for (id, on, off) in pairs {
                let want = (t % (on + off)) < on;
                let got = sim.output(id, "out").unwrap() != 0.0;
                assert_eq!(got, want, "{id} dt={dt} t={t}");
            }
        }
    }
}

#[test]
fn abs_sq_sqrt_match_the_simulation() {
    // Outputs: xr, nx, d, di and the six blocks under test.
    compare("crates/crab-sim/tests/fixtures/wasm_math", 12, 40, 100);
}

#[test]
fn bit_and_word_blocks_match_the_simulation() {
    compare("crates/crab-sim/tests/fixtures/wasm_bits", 32, 60, 100);
}

#[test]
fn number_change_scale_table_and_random_match_the_simulation() {
    compare("crates/crab-sim/tests/fixtures/wasm_misc", 7, 80, 100);
}

#[test]
fn string_change_matches_the_simulation() {
    compare("crates/crab-sim/tests/fixtures/wasm_strchg", 4, 40, 100);
}

#[test]
fn pid_and_selectors_match_the_simulation() {
    compare("crates/crab-sim/tests/fixtures/wasm_ctrl", 36, 100, 100);
}

#[test]
fn trigonometry_and_pow_go_through_the_master() {
    let lib = library();
    let project = Project::load(repo_path("crates/crab-sim/tests/fixtures/wasm_trig")).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    assert!(built.manifest.contains("\"math\""), "{}", built.manifest);
    compare("crates/crab-sim/tests/fixtures/wasm_trig", 11, 50, 100);
}

#[test]
fn flash_store_matches_the_simulation() {
    // Outputs: xr, step, nx, gen, keep, cr, nc.
    compare("crates/crab-sim/tests/fixtures/wasm_retain", 7, 60, 100);
}

#[test]
fn retained_variables_are_loaded_and_saved_through_the_master() {
    use crab_sim::mcu::StubIo;
    let lib = library();
    let project = Project::load(repo_path("crates/crab-sim/tests/fixtures/wasm_retain")).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    assert!(built.manifest.contains("\"retain\""), "{}", built.manifest);
    let manifest = Manifest::parse(&built.manifest).unwrap();
    let count_at = |module: &McuModule| {
        let i = built.watched.iter().position(|l| l == "nc.out").unwrap();
        module.outputs()[i]
    };
    let mut first = McuModule::load(&built.wasm, &manifest, built.watched.len()).unwrap();
    for _ in 0..30 {
        first.step(100).unwrap();
    }
    assert_eq!(count_at(&first), Word::int(30));
    // Written to flash after the first second and again when the value moved; the flash_store
    // key holds its float and the counter holds an int.
    let kept = first.io().retain.clone();
    assert_eq!(kept.len(), 2, "{kept:?}");
    let saved = kept
        .values()
        .copied()
        .filter(|v| (1..=30).contains(v))
        .count();
    assert_eq!(saved, 1, "{kept:?}");
    // After a restart the counter goes on from what flash holds.
    let io = StubIo {
        retain: kept.clone(),
        ..Default::default()
    };
    let mut second = McuModule::load_with(&built.wasm, &manifest, built.watched.len(), io).unwrap();
    second.step(100).unwrap();
    let last = kept
        .values()
        .copied()
        .filter(|v| (1..=30).contains(v))
        .max()
        .unwrap();
    assert_eq!(count_at(&second), Word::int(last + 1));
}

#[test]
fn text_selectors_and_variables_match_the_simulation() {
    compare("crates/crab-sim/tests/fixtures/wasm_strsel", 10, 40, 100);
}

/// FNV-1a as the key of a retained variable (`crab-codegen` hashes `name:type`).
fn retain_key(name: &str, ty: &str) -> i32 {
    let mut hash: u32 = 0x811c_9dc5;
    for b in format!("{name}:{ty}").bytes() {
        hash = (hash ^ u32::from(b)).wrapping_mul(0x0100_0193);
    }
    hash as i32
}

#[test]
fn texts_cross_sheets_and_are_kept_in_flash() {
    use crab_sim::mcu::StubIo;
    let lib = library();
    let project = Project::load(repo_path("crates/crab-sim/tests/fixtures/wasm_xsheet")).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let first = program.wasm_module_with(1, true).unwrap();
    let second = program.wasm_module_with(2, true).unwrap();
    for built in [&first, &second] {
        assert!(
            built.manifest.contains("\"tag_text\""),
            "{}",
            built.manifest
        );
    }
    assert!(
        second.manifest.contains("\"retain_text\""),
        "{}",
        second.manifest
    );
    let load = |built: &crab_codegen::WasmModule, io: StubIo| {
        let manifest = Manifest::parse(&built.manifest).unwrap();
        McuModule::load_with(&built.wasm, &manifest, built.watched.len(), io).unwrap()
    };
    let frozen = retain_key("frozen", "String");
    let mut io = StubIo::default();
    io.retain_text.insert(frozen, b"preset".to_vec());
    let mut m1 = load(&first, StubIo::default());
    let mut m2 = load(&second, io);
    // Numbers and texts that cross the border go from one module to the other after every scan.
    let relay = |from: &McuModule, to: &mut McuModule| {
        for id in 0..16 {
            if let Some(w) = from.tag(id) {
                to.set_tag(id, w);
            }
        }
        for (id, text) in from.text_tags() {
            to.set_text_tag(*id, text.clone());
        }
    };
    let out = |built: &crab_codegen::WasmModule, m: &McuModule, label: &str| {
        let i = built.watched.iter().position(|l| l == label).unwrap();
        m.outputs()[i]
    };
    let mut sim = Simulation::new(&project.logic, &lib).unwrap();
    for step in 0..40u32 {
        sim.step(100).unwrap();
        m1.step(100).unwrap();
        relay(&m1, &mut m2);
        m2.step(100).unwrap();
        for (built, m) in [(&first, &m1), (&second, &m2)] {
            for (i, label) in built.watched.iter().enumerate() {
                // The simulation has no flash: the kept text only shows in the module.
                if label == "frl.out" {
                    continue;
                }
                let (block, port) = label.split_once('.').unwrap();
                let expected = word(&program, label, sim.output(block, port).unwrap());
                assert_eq!(m.outputs()[i], expected, "step {step}: {label}");
            }
        }
    }
    // The text that crossed was the number printed on the first sheet.
    assert_ne!(out(&second, &m2, "mgl.out"), Word::int(0));
    assert_eq!(out(&second, &m2, "hl.out"), Word::int(5));
    // The kept text is read at start (`frozen`), the written one goes to flash.
    assert_eq!(out(&second, &m2, "frl.out"), Word::int(6));
    let saved = m2
        .io()
        .retain_text
        .get(&retain_key("name", "String"))
        .cloned();
    assert!(
        saved.is_some_and(|t| t.starts_with(b"n=")),
        "{:?}",
        m2.io().retain_text
    );
}

#[test]
fn blocks_on_expander_pins_match_the_simulation() {
    let lib = library();
    let project = Project::load(repo_path("crates/crab-sim/tests/fixtures/wasm_expander")).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    assert_eq!(manifest.allow, ["pcf_read", "pcf_write"]);
    let mut module = McuModule::load(&built.wasm, &manifest, built.watched.len()).unwrap();
    let mut sim = Simulation::new(&project.logic, &lib).unwrap();
    let key_in = 34 | 0xFF << 8 | 0xFF << 16;
    let key_out = 32 | 0xFF << 8 | 0xFF << 16 | 1 << 24;
    for step in 0..60u32 {
        // Buttons to the ground (pressed = low), the limit switch on the output chip too.
        let level = |period: u32| i32::from((step / period).is_multiple_of(2));
        let (p2, p3, p15) = (level(3), level(5), level(7));
        for (pin, v) in [(2, p2), (3, p3), (15, p15)] {
            sim.bench_mut().set_level(pin, f64::from(v));
        }
        let io = module.io_mut();
        io.pcf_in.insert(
            (key_in, 0),
            0xFF & !(1 << 2) & !(1 << 3) | p2 << 2 | p3 << 3,
        );
        io.pcf_in.insert((key_in, 1), 1);
        io.pcf_in
            .insert((key_out, 0), 0xFFFF & !(1 << 15) | p15 << 15);
        io.pcf_in.insert((key_out, 1), 1);
        sim.step(50).unwrap();
        module.step(50).unwrap();
        for (i, label) in built.watched.iter().enumerate() {
            let (block, port) = label.split_once('.').unwrap();
            let expected = word(&program, label, sim.output(block, port).unwrap());
            assert_eq!(module.outputs()[i], expected, "step {step}: {label}");
        }
        // The chip gets the physical levels of the outputs, inputs high.
        let mut want: i32 = 0xFFFF;
        for pin in [4, 5, 9] {
            let v = sim.bench().driven(pin).unwrap_or(0.0) as i32;
            want = want & !(1 << pin) | v << pin;
        }
        assert_eq!(module.io().pcf_out[&key_out], want, "step {step}: relays");
    }
}

#[test]
fn sparkplug_node_and_metrics_reach_the_master_ops() {
    let lib = library();
    let project =
        Project::load(repo_path("crates/crab-sim/tests/fixtures/wasm_sparkplug")).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    assert!(manifest.allow.iter().any(|a| a == "net"), "{manifest:?}");
    let mut module = McuModule::load(&built.wasm, &manifest, built.watched.len()).unwrap();
    module.step(10).unwrap();
    module.step(10).unwrap();
    let io = module.io();
    assert_eq!(io.sp_text[&1], "Plant");
    assert_eq!(io.sp_text[&2], "Esp1");
    assert!(io.sp_complete, "setup is announced once");
    assert_eq!(io.sp_enable, 1, "an unconnected en means enabled");
    let mut metrics = io.sp_metrics.clone();
    metrics.sort();
    assert_eq!(
        metrics,
        [
            ("Hot".to_string(), 0, 1),
            ("Temperature".to_string(), 2, 21.5f32.to_bits()),
            ("Whole".to_string(), 1, 22), // float_to_int rounds 21.5
        ]
    );
}

#[test]
fn network_settings_follow_text_variables() {
    use crab_sim::mcu::StubIo;
    let lib = library();
    let project = Project::load(repo_path("examples/wifi_bound")).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    let key = retain_key("Сеть", "String");
    let run = |kept: Option<&[u8]>| {
        let mut io = StubIo::default();
        if let Some(text) = kept {
            io.retain_text.insert(key, text.to_vec());
        }
        let mut module =
            McuModule::load_with(&built.wasm, &manifest, built.watched.len(), io).unwrap();
        module.step(10).unwrap();
        module.step(10).unwrap();
        (module.io().net_text.get(&1).cloned(), module.io().net_radio)
    };
    // The name from the variable (the initial one, or the one kept in flash) goes to the master.
    assert_eq!(run(None), (Some("MyNetwork".to_owned()), 1));
    assert_eq!(run(Some(b"HomeNet")), (Some("HomeNet".to_owned()), 1));
    // Without a name the client stays off.
    assert_eq!(run(Some(b"")), (Some(String::new()), 0));
}

#[test]
fn uart_frame_format_comes_from_an_input() {
    let lib = library();
    let project = Project::load(repo_path("crates/crab-sim/tests/fixtures/wasm_uartfmt")).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    let mut module = McuModule::load(&built.wasm, &manifest, built.watched.len()).unwrap();
    module.step(10).unwrap();
    let io = module.io();
    assert_eq!(io.uart_format.get(&2).map(String::as_str), Some("8E1"));
    assert_eq!(io.uart_speed.get(&2), Some(&19200));
}

#[test]
fn block_parameters_taken_from_variables_are_read_every_scan() {
    // The simulation does not know bound parameters, so the module is checked on its own.
    let lib = library();
    let project =
        Project::load(repo_path("crates/crab-sim/tests/fixtures/wasm_param_var")).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    let mut module = McuModule::load(&built.wasm, &manifest, built.watched.len()).unwrap();
    let at = |label: &str| built.watched.iter().position(|l| l == label).unwrap();
    let (chg, pid) = (at("chg.out"), at("pid.out"));
    let mut reports = Vec::new();
    for step in 0..14 {
        module.step(100).unwrap();
        if step == 0 {
            // kp = 2 from the variable, error 20 - 0: the output is 40 (the block's own kp is 1).
            assert_eq!(module.outputs()[pid], Word::float(40.0));
        }
        if module.outputs()[chg] == Word::int(1) {
            reports.push(step);
        }
    }
    // x grows by 1.5; the dead band 4 comes from the variable (the block's own is 1): a report
    // every third scan once the input has moved by more than 4.
    assert_eq!(reports, [3, 6, 9, 12]);
}

/// Modules that use what only a newer master can do say so in `needs`; the others need nothing.
#[test]
fn modules_list_what_they_need_from_the_master() {
    let lib = library();
    let needs = |dir: &str| {
        let project = Project::load(repo_path(dir)).unwrap();
        let program = Program::build(&project, &lib).unwrap();
        let mut needs = program.wasm_module(1).unwrap().needs;
        // Every module needs a master of ABI 4.
        let abi = needs.iter().position(|n| n == "abi4").expect("abi4");
        needs.remove(abi);
        needs
    };
    let fixtures = "crates/crab-sim/tests/fixtures";
    assert_eq!(needs(&format!("{fixtures}/wasm_trig")), ["math"]);
    assert_eq!(needs(&format!("{fixtures}/wasm_retain")), ["retain"]);
    assert_eq!(
        needs(&format!("{fixtures}/wasm_expander")),
        ["pcf_expander"]
    );
    assert_eq!(
        needs(&format!("{fixtures}/wasm_logic")),
        Vec::<String>::new()
    );
    // Text to USB goes to the console of the master.
    assert!(needs("examples/selftest_logic3").contains(&"uart_console".to_owned()));
}
