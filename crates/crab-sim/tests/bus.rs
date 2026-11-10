//! The internal bus (`specs/pubsub-bus`): state and event topics, bounded queues, replacing a module.

use crab_sim::bus::Bus;
use crab_sim::mcu::{Manifest, McuModule, Word};

fn manifest(toml: &str) -> Manifest {
    Manifest::parse(toml).unwrap()
}

fn load(src: &str, manifest: &Manifest) -> McuModule {
    McuModule::load(&wat::parse_str(src).unwrap(), manifest, 1).unwrap()
}

/// A module that publishes `payloads` (one byte each) on `topic` at every step.
fn publisher(topic: &str, payloads: &[u8]) -> String {
    let calls: String = (0..payloads.len())
        .map(|i| {
            format!(
                "(call $publish (i32.const 0) (i32.const {}) (i32.const {}) (i32.const 1))",
                topic.len(),
                16 + i
            )
        })
        .collect();
    let bytes: String = payloads.iter().map(|b| format!("\\{b:02x}")).collect();
    format!(
        r#"(module
            (import "crab" "publish" (func $publish (param i32 i32 i32 i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "{topic}")
            (data (i32.const 16) "{bytes}")
            (func (export "abi_version") (result i32) i32.const 4)
            (func (export "step") (param i32) {calls}))"#
    )
}

/// A module that subscribes to `topic`; tag 0 becomes `tag0 * 10 + first byte`, tag 1 counts messages.
fn subscriber(topic: &str) -> String {
    format!(
        r#"(module
            (import "crab" "subscribe" (func $subscribe (param i32 i32)))
            (import "crab" "tag_get_i" (func $get (param i32) (result i32)))
            (import "crab" "tag_set_i" (func $set (param i32 i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "{topic}")
            (global $heap (mut i32) (i32.const 1024))
            (func (export "abi_version") (result i32) i32.const 4)
            (func (export "init") (call $subscribe (i32.const 0) (i32.const {})))
            (func (export "alloc") (param $n i32) (result i32)
                (global.get $heap)
                (global.set $heap (i32.add (global.get $heap) (local.get $n))))
            (func (export "on_message") (param i32 i32 i32 i32)
                (call $set (i32.const 0)
                    (i32.add (i32.mul (call $get (i32.const 0)) (i32.const 10))
                             (i32.load8_u (local.get 2))))
                (call $set (i32.const 1) (i32.add (call $get (i32.const 1)) (i32.const 1))))
            (func (export "step") (param i32)))"#,
        topic.len()
    )
}

fn tag(bus: &mut Bus, module: usize, id: i32) -> i32 {
    bus.module(module)
        .unwrap()
        .tag(id)
        .unwrap_or(Word(0))
        .as_int()
}

const STATE_TEMP: &str = r#"topics = [{ name = "temp", kind = "state" }]"#;

#[test]
fn state_topic_gives_the_last_value_to_a_late_subscriber() {
    let mut bus = Bus::new();
    let m = manifest(STATE_TEMP);
    let sensor = bus.add(load(&publisher("temp", &[7]), &m), &m).unwrap();
    bus.step(10).unwrap();
    assert_eq!(bus.last("temp"), Some(&[7][..]));
    // the sensor is replaced by a silent one, a new module subscribes afterwards
    bus.replace(sensor, load(&publisher("other", &[1]), &m), &m)
        .unwrap();
    let late = bus.add(load(&subscriber("temp"), &m), &m).unwrap();
    bus.step(10).unwrap();
    bus.step(10).unwrap();
    assert_eq!(tag(&mut bus, late, 0), 7, "last value delivered once");
    assert_eq!(tag(&mut bus, late, 1), 1);
}

#[test]
fn replacing_the_sensor_does_not_disturb_the_reader() {
    let mut bus = Bus::new();
    let m = manifest(STATE_TEMP);
    let sensor = bus.add(load(&publisher("temp", &[1]), &m), &m).unwrap();
    let bot = bus.add(load(&subscriber("temp"), &m), &m).unwrap();
    bus.step(10).unwrap();
    bus.step(10).unwrap();
    assert_eq!(tag(&mut bus, bot, 0), 11); // 1 then 1 again
    bus.replace(sensor, load(&publisher("temp", &[2]), &m), &m)
        .unwrap();
    bus.step(10).unwrap();
    bus.step(10).unwrap();
    assert_eq!(tag(&mut bus, bot, 0), 1122, "the reader saw the new sensor");
}

#[test]
fn full_event_queue_drops_the_oldest_and_counts_it() {
    let mut bus = Bus::new();
    let none = manifest("");
    let slow = manifest("queue = 2");
    bus.add(load(&publisher("ev", &[1, 2, 3]), &none), &none)
        .unwrap();
    let reader = bus.add(load(&subscriber("ev"), &slow), &slow).unwrap();
    // the publisher runs first, so the reader finds 1, 2, 3 waiting and its queue holds two
    bus.step(10).unwrap();
    // the queue of two kept the newest two: 2 and 3
    assert_eq!(tag(&mut bus, reader, 0), 23);
    assert_eq!(tag(&mut bus, reader, 1), 2);
    assert_eq!(bus.lost(reader), 1);
}

#[test]
fn state_topic_keeps_only_the_newest_pending_value() {
    let mut bus = Bus::new();
    let m = manifest(STATE_TEMP);
    bus.add(load(&publisher("temp", &[1, 2, 3]), &m), &m)
        .unwrap();
    let reader = bus.add(load(&subscriber("temp"), &m), &m).unwrap();
    bus.step(10).unwrap();
    assert_eq!(tag(&mut bus, reader, 0), 3);
    assert_eq!(tag(&mut bus, reader, 1), 1);
    assert_eq!(bus.lost(reader), 0, "replacing a state value is not a loss");
}

#[test]
fn message_longer_than_the_topic_allows_is_refused() {
    let mut bus = Bus::new();
    let m = manifest(r#"topics = [{ name = "tiny", kind = "event", max_len = 0 }]"#);
    let long = manifest(r#"topics = [{ name = "ev", kind = "event", max_len = 1 }]"#);
    // a 2-byte message on a topic limited to 1 byte
    let src = publisher("ev", &[1]).replace("(i32.const 1))", "(i32.const 2))");
    bus.add(load(&src, &long), &long).unwrap();
    let reader = bus.add(load(&subscriber("ev"), &m), &m).unwrap();
    bus.step(10).unwrap();
    bus.step(10).unwrap();
    assert_eq!(bus.refused("ev"), 2);
    assert_eq!(tag(&mut bus, reader, 1), 0);
}

#[test]
fn a_topic_cannot_be_both_state_and_event() {
    let mut bus = Bus::new();
    let state = manifest(STATE_TEMP);
    let event = manifest(r#"topics = [{ name = "temp", kind = "event" }]"#);
    bus.add(load(&publisher("temp", &[1]), &state), &state)
        .unwrap();
    assert!(bus.add(load(&subscriber("temp"), &event), &event).is_err());
}
