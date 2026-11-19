use std::path::PathBuf;

use crab_core::{Library, Project};
use crab_sim::{BenchInput, Computed, Simulation};

fn repo(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

fn sim(example: &str) -> Simulation {
    let (lib, _) = Library::load(repo("library")).unwrap();
    let project = Project::load(repo(&format!("examples/{example}"))).unwrap();
    Simulation::new(&project.logic, &lib).unwrap()
}

/// Feeds a board input of the examples (buttons on the ground with a pull-up and inversion,
/// analog pots) so that the block reads `value`.
fn put(s: &mut Simulation, block: &str, value: f64) {
    match s.bench_input(block, "out").unwrap() {
        BenchInput::Digital { pin } => s.set_pin_level(pin, 1.0 - value),
        BenchInput::Analog { pin } => s.set_pin_level(pin, value),
        other => panic!("{other:?}"),
    }
}

fn on(s: &Simulation, block: &str, port: &str) -> bool {
    s.output(block, port).unwrap() != 0.0
}

#[test]
fn start_stop_runs_the_motor_fan_bell_and_counter() {
    let mut s = sim("start_stop");
    assert_eq!(s.computed("start"), Some(Computed::Bench));
    assert_eq!(s.computed("run"), Some(Computed::Builtin));

    assert!(
        !s.set_output("run", "q", 1.0),
        "computed outputs are not editable"
    );
    s.step(100).unwrap();
    assert!(!on(&s, "run", "q"));

    // Start pressed: the motor runs, the counter sees one start.
    put(&mut s, "start", 1.0);
    s.step(100).unwrap();
    put(&mut s, "start", 0.0);
    s.step(100).unwrap();
    assert!(on(&s, "run", "q"));
    assert_eq!(s.input("motor", "in"), Some(1.0));
    assert_eq!(s.output("starts", "count"), Some(1.0));

    // Stop pressed: the bell rings 2 s, the fan keeps running 30 s.
    put(&mut s, "stop", 1.0);
    s.step(100).unwrap();
    put(&mut s, "stop", 0.0);
    assert!(!on(&s, "run", "q"));
    assert!(on(&s, "bell", "out"));
    for _ in 0..19 {
        s.step(100).unwrap();
    }
    assert!(on(&s, "bell", "out"));
    s.step(100).unwrap();
    assert!(!on(&s, "bell", "out"));
    assert!(on(&s, "fan_delay", "out"));
    for _ in 0..290 {
        s.step(100).unwrap();
    }
    assert!(!on(&s, "fan_delay", "out"));
    assert_eq!(s.time_ms(), 100 * (3 + 1 + 20 + 290));
}

#[test]
fn reset_clears_state_and_time() {
    let mut s = sim("start_stop");
    put(&mut s, "start", 1.0);
    s.step(100).unwrap();
    assert!(on(&s, "run", "q"));
    s.reset().unwrap();
    assert_eq!(s.time_ms(), 0);
    assert!(!on(&s, "run", "q"));
}

#[test]
fn schema_with_errors_is_refused() {
    let (lib, _) = Library::load(repo("library")).unwrap();
    let mut project = Project::load(repo("examples/start_stop")).unwrap();
    project.logic.links.clear();
    assert!(Simulation::new(&project.logic, &lib).is_err());
}

#[test]
fn blink_alternates_every_half_second() {
    let mut s = sim("blink");
    let mut seen = Vec::new();
    for _ in 0..20 {
        s.step(100).unwrap();
        seen.push(on(&s, "blink", "out"));
    }
    // 100 ms steps: on for 400 ms (0..500 exclusive of the 500th), then off for 500 ms.
    assert_eq!(&seen[..4], &[true; 4]);
    assert_eq!(&seen[4..9], &[false; 5]);
    assert_eq!(&seen[9..14], &[true; 5]);
}

#[test]
fn toggle_flips_on_each_clean_press() {
    let mut s = sim("toggle");
    for _ in 0..2 {
        let before = on(&s, "toggle", "out");
        put(&mut s, "button", 1.0);
        for _ in 0..5 {
            s.step(20).unwrap();
        }
        put(&mut s, "button", 0.0);
        for _ in 0..5 {
            s.step(20).unwrap();
        }
        assert_eq!(on(&s, "toggle", "out"), !before);
    }
}

#[test]
fn variables_are_written_on_enable_and_read_back() {
    let mut s = sim("retain");
    assert_eq!(s.var("Порог"), Some(10.0));
    assert_eq!(s.block_var("total_r"), Some("Нажатий всего"));
    // The button is inverted (pull-up): 0 means pressed.
    put(&mut s, "button", 1.0);
    s.step(100).unwrap();
    put(&mut s, "button", 0.0);
    s.step(100).unwrap();
    s.step(100).unwrap();
    assert_eq!(s.var("Нажатий всего"), Some(1.0));
    assert_eq!(s.output("total_r", "out"), Some(1.0));
    assert!(s.set_var("Порог", 3.0));
    s.reset().unwrap();
    assert_eq!(s.var("Порог"), Some(10.0));
}

#[test]
fn parameter_read_from_an_input_follows_it_every_scan() {
    let mut s = sim("param_inputs");
    // The potentiometer sets the delay of the timer, in milliseconds.
    put(&mut s, "pot", 500.0);
    put(&mut s, "button", 1.0);
    s.step(100).unwrap();
    s.step(300).unwrap();
    assert!(!on(&s, "delay", "out"), "400 ms < 500 ms");
    s.step(200).unwrap();
    assert!(on(&s, "delay", "out"), "600 ms >= 500 ms");

    // Raising the delay on the fly switches the output off again.
    put(&mut s, "pot", 3000.0);
    s.step(100).unwrap();
    assert!(!on(&s, "delay", "out"));

    // The limit takes its upper bound from the constant block (2000).
    put(&mut s, "pot", 1500.0);
    s.step(100).unwrap();
    assert_eq!(s.output("cap", "out"), Some(1500.0));
}

/// Strings travel along the links beside the numbers.
#[test]
fn strings_flow_through_text_blocks() {
    let dir = std::env::temp_dir().join(format!("crab-sim-text-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("project.toml"),
        "[project]\nname = \"t\"\n\n[board]\nfqbn = \"esp32:esp32:esp32\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("logic.toml"),
        r#"
[[block]]
id = "hello"
type = "text_const"
value = "Hi"

[[block]]
id = "temp"
type = "constant"
value = 21.46

[[block]]
id = "num"
type = "num_to_text"
decimals = 1
prefix = "T="

[[block]]
id = "join"
type = "text_concat"
count = 2
separator = " "

[[block]]
id = "len"
type = "text_length"

[[block]]
id = "is_hi"
type = "text_compare"
b = "hi"
ignore_case = true

[[link]]
from = "temp.out"
to = "num.in"

[[link]]
from = "hello.out"
to = "join.a"

[[link]]
from = "num.out"
to = "join.b"

[[link]]
from = "join.out"
to = "len.in"

[[link]]
from = "hello.out"
to = "is_hi.a"
"#,
    )
    .unwrap();
    let (lib, _) = Library::load(repo("library")).unwrap();
    let project = Project::load(&dir).unwrap();
    let mut s = Simulation::new(&project.logic, &lib).unwrap();
    s.step(10).unwrap();
    assert_eq!(s.output_text("join", "out"), Some("Hi T=21.5"));
    assert_eq!(s.output("len", "out"), Some(9.0));
    assert!(on(&s, "is_hi", "out"));
    assert_eq!(s.output_text("len", "out"), None, "numbers have no text");
}

/// Web controls start at `initial` and can be changed by hand; widgets show; MQTT
/// publication pulses on change.
#[test]
fn web_and_mqtt_blocks_in_the_schema() {
    let dir = std::env::temp_dir().join(format!("crab-sim-web-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("project.toml"),
        "[project]\nname = \"t\"\n\n[board]\nfqbn = \"esp32:esp32:esp32\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("logic.toml"),
        r#"
[[block]]
id = "sw"
type = "web_switch"
initial = true

[[block]]
id = "sl"
type = "web_slider"
initial = 40

[[block]]
id = "led"
type = "web_led"

[[block]]
id = "pub"
type = "mqtt_publish"

[[block]]
id = "sub"
type = "mqtt_subscribe"

[[link]]
from = "sw.out"
to = "led.in"

[[link]]
from = "sl.out"
to = "pub.value"
"#,
    )
    .unwrap();
    let (lib, _) = Library::load(repo("library")).unwrap();
    let project = Project::load(&dir).unwrap();
    let mut s = Simulation::new(&project.logic, &lib).unwrap();
    assert!(on(&s, "sw", "out"));
    assert_eq!(s.output("sl", "out"), Some(40.0));
    s.step(10).unwrap();
    assert_eq!(s.input("led", "in"), Some(1.0));
    assert!(on(&s, "pub", "sent"), "first value is published");
    s.step(10).unwrap();
    assert!(on(&s, "pub", "sent"), "the pulse is long enough to be seen");
    s.step(400).unwrap();
    assert!(!on(&s, "pub", "sent"));
    assert!(s.set_output("sl", "out", 55.0));
    s.step(10).unwrap();
    assert!(on(&s, "pub", "sent"), "a changed value is published");
    // A message fed by hand raises `received` for a while.
    assert!(s.set_output_text("sub", "out", "on"));
    assert!(on(&s, "sub", "received"));
    s.step(100).unwrap();
    assert!(on(&s, "sub", "received"));
    s.step(300).unwrap();
    assert!(!on(&s, "sub", "received"));
    s.reset().unwrap();
    assert_eq!(s.output("sl", "out"), Some(40.0));
}

/// String project variables: written by `var_set`, read by `var_get`, restored on reset.
#[test]
fn string_variables_are_stored() {
    let dir = std::env::temp_dir().join(format!("crab-sim-svar-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("project.toml"),
        "[project]\nname = \"t\"\n\n[board]\nfqbn = \"esp32:esp32:esp32\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("logic.toml"),
        r#"
[[var]]
name = "msg"
type = "String"
init = "start"

[[block]]
id = "src"
type = "text_const"
value = "changed"

[[block]]
id = "w"
type = "var_set"
var = "msg"
write_enable = true

[[block]]
id = "r"
type = "var_get"
var = "msg"

[[link]]
from = "src.out"
to = "w.in"
"#,
    )
    .unwrap();
    let (lib, _) = Library::load(repo("library")).unwrap();
    let project = Project::load(&dir).unwrap();
    let mut s = Simulation::new(&project.logic, &lib).unwrap();
    assert_eq!(s.var_text("msg"), Some("start"));
    s.step(10).unwrap();
    s.step(10).unwrap();
    assert_eq!(s.var_text("msg"), Some("changed"));
    assert_eq!(s.output_text("r", "out"), Some("changed"));
    s.reset().unwrap();
    assert_eq!(s.var_text("msg"), Some("start"));
}

#[test]
fn bench_button_lights_the_led_through_pullup_and_inversion() {
    let mut s = sim("button");
    assert_eq!(s.computed("button"), Some(Computed::Bench));
    assert_eq!(
        s.bench_input("button", "out"),
        Some(BenchInput::Digital { pin: 13 })
    );
    // Released: the pull-up holds the pin high, the inverted input reads false.
    s.step(10).unwrap();
    assert_eq!(s.bench().driven(2), Some(0.0));
    // Pressed: the pin goes to ground.
    s.set_pin_level(13, 0.0);
    s.step(10).unwrap();
    assert_eq!(s.bench().driven(2), Some(1.0));
    assert_eq!(s.output_pin("led"), Some((2, 1.0)));
    s.set_pin_level(13, 1.0);
    s.step(10).unwrap();
    assert_eq!(s.bench().driven(2), Some(0.0));
}

#[test]
fn bench_network_outage_reaches_the_outputs() {
    let mut s = sim("mqtt");
    assert_eq!(s.bench_input("wifi", "ready"), Some(BenchInput::Link));
    assert_eq!(
        s.bench_input("broker", "connected"),
        Some(BenchInput::Broker)
    );
    s.step(10).unwrap();
    assert_eq!(s.output("wifi", "ready"), Some(1.0));
    assert_eq!(s.output("broker", "connected"), Some(1.0));
    s.set_broker_down(true);
    s.step(10).unwrap();
    assert_eq!(s.output("wifi", "ready"), Some(1.0));
    assert_eq!(s.output("broker", "connected"), Some(0.0));
    s.set_offline(true);
    s.step(10).unwrap();
    assert_eq!(s.output("wifi", "ready"), Some(0.0));
    assert_eq!(s.output_text("wifi", "address"), Some(""));
}

/// A simulation of the schema `logic` on a board with two UARTs (as the block checks set them).
fn sim_logic(logic: &str) -> Simulation {
    const BOARD: &str = "[project]\nname = \"modbus\"\n\n[board]\nfqbn = \"esp32:esp32:esp32\"\n\n\
        [[uart]]\nnumber = 1\nspeed = 19200\nformat = \"8N1\"\nrx = 25\ntx = 27\n\n\
        [[uart]]\nnumber = 2\nspeed = 9600\nformat = \"8N1\"\nrx = 16\ntx = 17\n";
    let (lib, _) = Library::load(repo("library")).unwrap();
    let project = Project::from_sources("modbus", BOARD, logic, "").unwrap();
    Simulation::new(&project.logic, &lib).unwrap()
}

#[test]
fn bench_modbus_master_reads_a_register_and_loses_the_slave() {
    let mut s = sim_logic(
        r#"
[[block]]
id = "master"
type = "modbus_rtu_master"
uart = 2

[[block]]
id = "sensor"
type = "modbus_device"
master = "master"
unit = 10

[[block]]
id = "alarm"
type = "modbus_tag"
device = "sensor"
table = "discrete"
kind = "bool"
reg = 0

[[block]]
id = "led"
type = "digital_out"
pin = 2

[[link]]
from = "alarm.out_b"
to = "led.in"
"#,
    );
    let Some(BenchInput::Modbus { slave, table, reg }) = s.bench_input("alarm", "out_b") else {
        panic!("a read modbus_tag reads the bench");
    };
    assert_eq!((slave, table.as_str(), reg), (10, "discrete", 0));
    s.set_register(slave, &table, reg, 1.0);
    s.step(10).unwrap();
    assert_eq!(s.output("alarm", "out_b"), Some(1.0));
    assert_eq!(s.output("alarm", "ok"), Some(1.0));
    assert_eq!(s.output_pin("led"), Some((2, 1.0)));
    s.set_silent(slave, true);
    s.step(10).unwrap();
    assert_eq!(s.output("alarm", "ok"), Some(0.0));
    // The last value stays while the device is silent.
    assert_eq!(s.output("alarm", "out_b"), Some(1.0));
}

#[test]
fn bench_written_modbus_tag_puts_its_value_into_the_device() {
    let mut s = sim_logic(
        r#"
[[block]]
id = "master"
type = "modbus_rtu_master"
uart = 2

[[block]]
id = "sensor"
type = "modbus_device"
master = "master"
unit = 10

[[block]]
id = "button"
type = "digital_in"
pin = 13
pullup = true
inverted = true

[[block]]
id = "relay"
type = "modbus_tag"
device = "sensor"
table = "coil"
kind = "bool"
reg = 3

[[link]]
from = "button.out"
to = "relay.in_b"
"#,
    );
    put(&mut s, "button", 1.0);
    s.step(10).unwrap();
    assert_eq!(s.bench().register(10, "coil", 3), 1.0);
    s.set_silent(10, true);
    put(&mut s, "button", 0.0);
    s.step(10).unwrap();
    assert_eq!(
        s.bench().register(10, "coil", 3),
        1.0,
        "a silent device gets nothing"
    );
}

#[test]
fn mqtt_publish_waits_for_the_broker() {
    let mut s = sim("mqtt");
    s.step(10).unwrap();
    s.step(1000).unwrap();
    assert_eq!(s.output("button_pub", "sent"), Some(0.0));
    s.set_broker_down(true);
    s.set_pin_level(13, 0.0);
    s.step(10).unwrap();
    assert_eq!(s.output("button_pub", "sent"), Some(0.0));
    s.set_broker_down(false);
    s.step(10).unwrap();
    assert_eq!(s.output("button_pub", "sent"), Some(1.0));
}

#[test]
fn bench_own_modbus_slave_meets_the_master_and_the_program() {
    let mut s = sim_logic(
        r#"
[[block]]
id = "wifi"
type = "wifi"
ssid = "MyNetwork"

[[block]]
id = "slave"
type = "modbus_tcp_slave"

[[block]]
id = "led_reg"
type = "modbus_tag"
device = "slave"
table = "coil"
kind = "bool"
reg = 0

[[block]]
id = "led"
type = "digital_out"
pin = 2

[[block]]
id = "button"
type = "digital_in"
pin = 13
pullup = true
inverted = true

[[block]]
id = "button_reg"
type = "modbus_tag"
device = "slave"
table = "discrete"
kind = "bool"
reg = 0

[[link]]
from = "led_reg.out_b"
to = "led.in"

[[link]]
from = "button.out"
to = "button_reg.in_b"
"#,
    );
    let Some(BenchInput::Modbus { slave, table, reg }) = s.bench_input("led_reg", "out_b") else {
        panic!("a read modbus_tag reads the bench");
    };
    assert_eq!((slave, table.as_str(), reg), (0, "coil", 0));
    assert_eq!(
        s.bench_input("button_reg", "in_b"),
        None,
        "a written tag does not read"
    );
    s.step(10).unwrap();
    assert_eq!(s.output("led_reg", "out_b"), Some(0.0));
    // The master writes a coil, the program reads it.
    s.set_register(slave, &table, reg, 1.0);
    s.step(10).unwrap();
    assert_eq!(s.output("led_reg", "out_b"), Some(1.0));
    // The program writes a discrete input, the master would read it.
    put(&mut s, "button", 1.0);
    s.step(10).unwrap();
    let on = s.bench().register(slave, "discrete", 0);
    put(&mut s, "button", 0.0);
    s.step(10).unwrap();
    assert_ne!(on, s.bench().register(slave, "discrete", 0));
}

#[test]
fn bench_uart_read_gets_the_typed_line() {
    let mut s = sim("text_commands");
    let Some(BenchInput::Uart { port }) = s.bench_input("rx", "line") else {
        panic!("uart_read reads the bench");
    };
    s.step(10).unwrap();
    assert_eq!(s.output("rx", "ready"), Some(0.0));
    s.send_uart(port, "hello");
    s.step(10).unwrap();
    assert_eq!(s.output("rx", "ready"), Some(1.0));
    assert_eq!(s.output_text("rx", "line"), Some("hello"));
    s.step(10).unwrap();
    assert_eq!(s.output("rx", "ready"), Some(0.0));
    assert_eq!(s.output_text("rx", "line"), Some("hello"));
}

fn counters_sim() -> Simulation {
    let (lib, _) = Library::load(repo("library")).unwrap();
    let logic = toml::from_str(
        r#"
[[block]]
id = "rst"
type = "digital_in"
pin = 13

[[block]]
id = "hsc"
type = "fast_counter"
pin = 25

[[block]]
id = "f"
type = "frequency"
pin = 26

[[block]]
id = "enc"
type = "quad_encoder"
pin_a = 32
pin_b = 33

[[link]]
from = "rst.out"
to = "hsc.reset"

[[link]]
from = "rst.out"
to = "enc.reset"
"#,
    )
    .unwrap();
    Simulation::new(&logic, &lib).unwrap()
}

#[test]
fn bench_counters_count_the_set_rate() {
    let mut s = counters_sim();
    for (block, port) in [("hsc", "count"), ("f", "hz"), ("enc", "pos")] {
        assert!(matches!(
            s.bench_input(block, port),
            Some(BenchInput::Rate { .. })
        ));
    }
    assert_eq!(s.bench_input("hsc", "ok"), None);
    s.set_pin_level(25, 1000.0);
    s.set_pin_level(26, 250.0);
    s.set_pin_level(32, 100.0);
    for _ in 0..100 {
        s.step(10).unwrap();
    }
    // One second of simulated time.
    assert_eq!(s.output("hsc", "count"), Some(1000.0));
    assert_eq!(s.output("hsc", "ok"), Some(1.0));
    assert_eq!(s.output("f", "hz"), Some(250.0));
    assert_eq!(s.output("f", "count"), Some(250.0));
    assert_eq!(s.output("enc", "pos"), Some(100.0));
    assert_eq!(s.output("enc", "dir"), Some(1.0));
    // Reset holds the counters at zero.
    s.set_pin_level(13, 1.0);
    s.step(10).unwrap();
    assert_eq!(s.output("hsc", "count"), Some(0.0));
    assert_eq!(s.output("enc", "pos"), Some(0.0));
    s.set_pin_level(13, 0.0);
    s.step(10).unwrap();
    assert_eq!(s.output("hsc", "count"), Some(10.0));
}

#[test]
fn bench_encoder_counts_backwards_on_negative_rate() {
    let mut s = counters_sim();
    s.set_pin_level(32, -50.0);
    for _ in 0..100 {
        s.step(10).unwrap();
    }
    assert_eq!(s.output("enc", "pos"), Some(-50.0));
    assert_eq!(s.output("enc", "dir"), Some(-1.0));
    s.set_pin_level(32, 0.0);
    s.step(10).unwrap();
    assert_eq!(s.output("enc", "dir"), Some(0.0));
    assert_eq!(s.output("enc", "pos"), Some(-50.0));
}

#[test]
fn analog_out_drives_the_dac_code_in_the_bench() {
    let (lib, _) = Library::load(repo("library")).unwrap();
    let logic = toml::from_str(
        r#"
[[block]]
id = "pot"
type = "analog_in"
pin = 34

[[block]]
id = "dac"
type = "analog_out"
pin = 25

[[link]]
from = "pot.out"
to = "dac.in"
"#,
    )
    .unwrap();
    let mut s = Simulation::new(&logic, &lib).unwrap();
    for (level, code) in [(128.0, 128.0), (300.0, 255.0), (0.0, 0.0)] {
        s.set_pin_level(34, level);
        s.step(10).unwrap();
        assert_eq!(s.output_pin("dac"), Some((25, code)), "level {level}");
    }
}

#[test]
fn an_unused_tag_with_an_empty_input_does_not_stop_the_simulation() {
    let (lib, _) = Library::load(repo("library")).unwrap();
    let mut p = Project::load(repo("examples/tag_copies")).unwrap();
    p.take_off_board("lamp_2").unwrap();
    p.take_off_board("lamp").unwrap();
    assert_eq!(p.unused_tags(&lib), ["lamp"]);
    // The editor check passes, so the simulation must start too.
    assert!(Simulation::new(&p.logic, &lib).is_err());
    let mut s = Simulation::for_project(&p, &lib).unwrap();
    s.step(10).unwrap();
}
