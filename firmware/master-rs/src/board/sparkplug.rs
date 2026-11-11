//! Sparkplug B edge node of the master (`specs/wasm-kernel/plan.md`, 4.1в). Modules reach it through
//! the `net` import, ops 30..=33; the ABI is the one of the C++ master (`specs/wasm-on-mcu/proto/master`).
//!
//! A module names the node (group, node id) and declares metrics; the master owns the session: the
//! will is NDEATH, after every connect NBIRTH goes out with all metrics, then NDATA with the changed
//! ones by alias. The thread `crab-mqtt` calls [`poll`]; a module step never waits for the broker.
//! The protobuf bytes come from `crab-sparkplug`.

use std::sync::{Mutex, MutexGuard};

use crab_kernel::host::Mem;
use crab_sparkplug::{MessageType, Metric as SpMetric, Payload, Topic, Value, BD_SEQ, REBIRTH};
use esp_idf_svc::mqtt::client::{EspMqttClient, QoS};

use super::devices::uptime_ms;
use super::{mqtt, net};

/// Metrics per node.
const MAX_METRICS: usize = 12;
/// Longest group, node id and metric name.
const MAX_NAME: usize = 32;
/// Longest payload; a longer one is not sent (counter `toobig`).
const MAX_PAYLOAD: usize = 600;
/// The metric table has to stay unchanged this long before the node is announced again.
const SETTLE_MS: u32 = 500;
/// Shortest pause between two NDATA.
const DATA_PERIOD_MS: u32 = 250;

struct Metric {
    name: String,
    /// 0 bool, 1 int, 2 float.
    kind: u8,
    /// The value; a float as its bits.
    bits: i32,
    /// The bits of the last message.
    sent: i32,
}

struct Sp {
    group: String,
    node: String,
    enable: bool,
    complete: bool,
    metrics: Vec<Metric>,
    /// The table changed: announce it again once it has settled.
    dirty: bool,
    dirty_at: u32,
    /// Counts the changes of the table, so that a change during a publish is not forgotten.
    generation: u32,
    online: bool,
    was_up: bool,
    bd_seq: u8,
    seq: u8,
    births: u32,
    too_big: u32,
    last_data: u32,
    /// The journal knows about the current series of too long payloads.
    told_big: bool,
}

static SP: Mutex<Sp> = Mutex::new(Sp {
    group: String::new(),
    node: String::new(),
    enable: false,
    complete: false,
    metrics: Vec::new(),
    dirty: true,
    dirty_at: 0,
    generation: 0,
    online: false,
    was_up: false,
    bd_seq: 0,
    seq: 0,
    births: 0,
    too_big: 0,
    last_data: 0,
    told_big: false,
});

fn lock() -> MutexGuard<'static, Sp> {
    SP.lock().unwrap_or_else(|e| e.into_inner())
}

impl Sp {
    fn active(&self) -> bool {
        self.complete && self.enable
    }

    fn mark_dirty(&mut self) {
        self.dirty = true;
        self.dirty_at = uptime_ms() as u32;
        self.generation = self.generation.wrapping_add(1);
    }

    fn topic(&self, kind: MessageType) -> String {
        Topic {
            group: &self.group,
            kind,
            node: &self.node,
            device: None,
        }
        .to_string()
    }

    fn bd_metric(&self) -> SpMetric {
        SpMetric::named(BD_SEQ, Value::Int64(i64::from(self.bd_seq)))
    }

    /// NDEATH: only `bdSeq`, no timestamp, no `seq`.
    fn death(&self) -> (String, Vec<u8>) {
        let payload = Payload {
            timestamp: None,
            metrics: vec![self.bd_metric()],
            seq: None,
        };
        (self.topic(MessageType::NDeath), payload.encode())
    }

    /// NBIRTH: `bdSeq`, the rebirth flag, then every metric with name and alias.
    fn birth(&self, ts: Option<u64>, seq: u8) -> (String, Vec<u8>) {
        let mut metrics = vec![
            self.bd_metric(),
            SpMetric::named(REBIRTH, Value::Bool(false)),
        ];
        for (i, m) in self.metrics.iter().enumerate() {
            metrics.push(SpMetric::birth(
                &m.name,
                i as u64 + 1,
                value_of(m.kind, m.bits),
            ));
        }
        let payload = Payload {
            timestamp: ts,
            metrics,
            seq: Some(u64::from(seq)),
        };
        (self.topic(MessageType::NBirth), payload.encode())
    }

    /// NDATA: the metrics that differ from the last message (all of them if `all`), by alias.
    fn data(&self, ts: Option<u64>, seq: u8, all: bool) -> (String, Vec<u8>) {
        let metrics = self
            .metrics
            .iter()
            .enumerate()
            .filter(|(_, m)| all || m.bits != m.sent)
            .map(|(i, m)| SpMetric::alias(i as u64 + 1, value_of(m.kind, m.bits)))
            .collect();
        let payload = Payload {
            timestamp: ts,
            metrics,
            seq: Some(u64::from(seq)),
        };
        (self.topic(MessageType::NData), payload.encode())
    }

    /// Remembers what a message carried.
    fn mark_sent(&mut self, snapshot: &[i32]) {
        for (m, bits) in self.metrics.iter_mut().zip(snapshot) {
            m.sent = *bits;
        }
    }

    /// One journal line per series of too long payloads, not one per tick.
    fn tell_big(&mut self) {
        self.too_big += 1;
        if !self.told_big {
            self.told_big = true;
            mqtt::note("sparkplug: payload too big".to_string());
        }
    }
}

fn value_of(kind: u8, bits: i32) -> Value {
    match kind {
        0 => Value::Bool(bits != 0),
        1 => Value::Int32(bits),
        _ => Value::Float(f32::from_bits(bits as u32)),
    }
}

// ---------------------------------------------------------------------------------------------
// The import
// ---------------------------------------------------------------------------------------------

/// `net(op, a, b, c)` for the Sparkplug operations (30..=33).
pub fn call(op: i32, a: i32, b: i32, c: i32, mem: &mut Mem<'_>) -> i32 {
    // The node keeps the MQTT client alive as the MQTT ops do.
    mqtt::touch();
    match op {
        30 => setup(a, b, c, mem),
        31 => declare(a, b, c, mem),
        32 => {
            let mut sp = lock();
            if let Some(m) = usize::try_from(a).ok().and_then(|i| sp.metrics.get_mut(i)) {
                m.bits = b;
            }
            0
        }
        33 => {
            let sp = lock();
            match a {
                0 => i32::from(sp.online),
                1 => sp.births as i32,
                _ => 0,
            }
        }
        _ => 0,
    }
}

/// Names are cut at 32 bytes, as in the C++ master (a cut character is dropped).
fn name_of(bytes: &[u8]) -> String {
    let mut s = String::from_utf8_lossy(bytes).into_owned();
    if bytes.len() >= MAX_NAME && s.ends_with('\u{FFFD}') {
        s.pop();
    }
    s
}

fn setup(code: i32, b: i32, c: i32, mem: &Mem<'_>) -> i32 {
    match code {
        1 | 2 => {
            let Some(bytes) = mem.read(b, c, MAX_NAME) else {
                return 0;
            };
            let text = name_of(&bytes);
            let mut sp = lock();
            let field = if code == 1 {
                &mut sp.group
            } else {
                &mut sp.node
            };
            if *field != text {
                *field = text;
            }
        }
        4 => lock().enable = b != 0,
        63 => lock().complete = true,
        _ => {}
    }
    0
}

fn declare(ptr: i32, len: i32, kind: i32, mem: &Mem<'_>) -> i32 {
    if len <= 0 || len > MAX_NAME as i32 || !(0..=2).contains(&kind) {
        return -1;
    }
    let Some(bytes) = mem.read(ptr, len, MAX_NAME) else {
        return -1;
    };
    let name = name_of(&bytes);
    let kind = kind as u8;
    let mut sp = lock();
    if let Some(i) = sp.metrics.iter().position(|m| m.name == name) {
        if sp.metrics[i].kind != kind {
            sp.metrics[i].kind = kind;
            sp.mark_dirty();
        }
        return i as i32;
    }
    if sp.metrics.len() >= MAX_METRICS {
        return -1;
    }
    sp.metrics.push(Metric {
        name,
        kind,
        bits: 0,
        sent: 0,
    });
    sp.mark_dirty();
    (sp.metrics.len() - 1) as i32
}

// ---------------------------------------------------------------------------------------------
// The session
// ---------------------------------------------------------------------------------------------

/// The will of the client: NDEATH with the current `bdSeq`. `None` while the node is not set up or
/// not enabled. The broker gets it with CONNECT only, so it is part of the client settings.
pub(super) fn will() -> Option<(String, Vec<u8>)> {
    let sp = lock();
    sp.active().then(|| sp.death())
}

/// Called by the worker every tick before it reads the settings. A session that was up and is not
/// any more starts a new `bdSeq`; `true` tells the worker to drop the client (its will is old, and a
/// client that reconnects by itself would bring the node back with it).
pub(super) fn track(connected: bool) -> bool {
    let mut sp = lock();
    if !sp.active() {
        sp.was_up = false;
        sp.online = false;
        return false;
    }
    if connected {
        sp.was_up = true;
        return false;
    }
    sp.online = false;
    end_session(&mut sp)
}

/// The master dropped the client itself (new settings, no module wants it): the session is over.
pub(super) fn ended() {
    let mut sp = lock();
    sp.online = false;
    end_session(&mut sp);
}

/// `bdSeq` grows once per ended session, however many ways lead to the end: `was_up` is the guard.
fn end_session(sp: &mut Sp) -> bool {
    if !sp.was_up {
        return false;
    }
    sp.was_up = false;
    sp.bd_seq = sp.bd_seq.wrapping_add(1);
    true
}

/// Sends NBIRTH after a connect and NDATA for changed values; as `spPoll` of the C++ master.
pub(super) fn poll(client: &mut EspMqttClient<'static>, connected: bool) {
    let now = uptime_ms() as u32;
    // Built under the lock, sent without it: the client task may be waiting for locks of this module.
    let mut sp = lock();
    if !sp.active() || !connected {
        sp.online = false;
        return;
    }
    if sp.dirty {
        // The metric table changed: announce everything again once it has settled.
        sp.online = false;
        if now.wrapping_sub(sp.dirty_at) < SETTLE_MS {
            return;
        }
    }
    let ts = net::unix_ms();
    // Values read before publishing: a change during the publish goes into the next message.
    let snapshot: Vec<i32> = sp.metrics.iter().map(|m| m.bits).collect();
    let generation = sp.generation;
    if !sp.online {
        let (topic, bytes) = sp.birth(ts, 0);
        if bytes.len() > MAX_PAYLOAD {
            sp.dirty = false;
            sp.tell_big();
            return;
        }
        drop(sp);
        if client
            .enqueue(&topic, QoS::AtMostOnce, false, &bytes)
            .is_err()
        {
            return;
        }
        let mut sp = lock();
        sp.seq = 1;
        sp.online = true;
        // A change of the table during the publish needs a new birth.
        sp.dirty = sp.generation != generation;
        sp.births += 1;
        sp.last_data = now;
        sp.told_big = false;
        sp.mark_sent(&snapshot);
        mqtt::note(format!(
            "sparkplug: birth sent ({} metrics, bdSeq {})",
            snapshot.len(),
            sp.bd_seq
        ));
        return;
    }
    if now.wrapping_sub(sp.last_data) < DATA_PERIOD_MS
        || sp.metrics.iter().all(|m| m.bits == m.sent)
    {
        return;
    }
    let seq = sp.seq;
    let (topic, bytes) = sp.data(ts, seq, false);
    if bytes.len() > MAX_PAYLOAD {
        sp.tell_big();
        return;
    }
    drop(sp);
    if client
        .enqueue(&topic, QoS::AtMostOnce, false, &bytes)
        .is_err()
    {
        return;
    }
    let mut sp = lock();
    sp.seq = sp.seq.wrapping_add(1);
    sp.last_data = now;
    sp.told_big = false;
    sp.mark_sent(&snapshot);
}

/// `spdump <ms>`: NDEATH, NBIRTH and NDATA (all metrics) as hex, nothing is sent.
pub(super) fn dump(ts_ms: u64) {
    let sp = lock();
    let messages = [
        sp.death(),
        sp.birth(Some(ts_ms), 0),
        sp.data(Some(ts_ms), 1, true),
    ];
    for (topic, bytes) in &messages {
        if bytes.len() > MAX_PAYLOAD {
            println!("sp {topic} too-big");
        } else {
            let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            println!("sp {topic} {hex}");
        }
    }
    println!(
        "sp state online={} births={} bdseq={} metrics={} toobig={}",
        u8::from(sp.online),
        sp.births,
        sp.bd_seq,
        sp.metrics.len(),
        sp.too_big
    );
    println!("ok");
}
