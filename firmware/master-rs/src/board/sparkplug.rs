//! Sparkplug B edge node of the master (`specs/wasm-kernel/plan.md`, 4.1в). Modules reach it through
//! the `net` import, ops 30..=33; the ABI is fixed in `crab-kernel` (`abi.rs`).
//!
//! A module names the node (group, node id) and declares metrics; the master owns the session: the
//! will is NDEATH, after every connect NBIRTH goes out with all metrics, then NDATA with the changed
//! ones by alias. The thread `crab-mqtt` calls [`poll`]; a module step never waits for the broker.
//! The protobuf bytes come from `crab-sparkplug`.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU8, Ordering};

use crab_kernel::host::Mem;
use crab_sparkplug::{MessageType, Metric as SpMetric, Payload, Topic, Value, BD_SEQ, REBIRTH};
use esp_idf_svc::mqtt::client::{EspMqttClient, QoS};

use super::devices::uptime_ms;
use super::spin::Spin;
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

/// A declared metric.
#[derive(Clone)]
struct Def {
    name: String,
    /// 0 bool, 1 int, 2 float.
    kind: u8,
}

/// What a module set up: the node and its metric table. The scan writes it (`setup`, `declare`),
/// the worker keeps a copy (`Session::cfg`) and takes a new one when `version` moved.
#[derive(Clone)]
struct Cfg {
    group: String,
    node: String,
    enable: bool,
    complete: bool,
    metrics: Vec<Def>,
    /// Counts every change of the setup (names, flags, table); only compared for equality.
    version: u32,
}

impl Cfg {
    const fn new() -> Self {
        Self {
            group: String::new(),
            node: String::new(),
            enable: false,
            complete: false,
            metrics: Vec::new(),
            version: 0,
        }
    }

    /// `self = other` reusing what `self` has allocated.
    fn copy_from(&mut self, o: &Cfg) {
        self.group.clone_from(&o.group);
        self.node.clone_from(&o.node);
        self.enable = o.enable;
        self.complete = o.complete;
        self.metrics.clone_from(&o.metrics);
        self.version = o.version;
    }

    fn active(&self) -> bool {
        self.complete && self.enable
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

    /// NDEATH: only `bdSeq`, no timestamp, no `seq`.
    fn death(&self, bd_seq: u8) -> (String, Vec<u8>) {
        let payload = Payload {
            timestamp: None,
            metrics: vec![SpMetric::named(BD_SEQ, Value::Int64(i64::from(bd_seq)))],
            seq: None,
        };
        (self.topic(MessageType::NDeath), payload.encode())
    }

    /// NBIRTH: `bdSeq`, the rebirth flag, then every metric with name and alias; `bits` are the
    /// values.
    fn birth(&self, bd_seq: u8, ts: Option<u64>, seq: u8, bits: &[i32]) -> (String, Vec<u8>) {
        let mut metrics = vec![
            SpMetric::named(BD_SEQ, Value::Int64(i64::from(bd_seq))),
            SpMetric::named(REBIRTH, Value::Bool(false)),
        ];
        for (i, (m, v)) in self.metrics.iter().zip(bits).enumerate() {
            metrics.push(SpMetric::birth(&m.name, i as u64 + 1, value_of(m.kind, *v)));
        }
        let payload = Payload {
            timestamp: ts,
            metrics,
            seq: Some(u64::from(seq)),
        };
        (self.topic(MessageType::NBirth), payload.encode())
    }

    /// NDATA: the metrics whose value differs from `sent` (all of them if `all`), by alias.
    fn data(
        &self,
        ts: Option<u64>,
        seq: u8,
        all: bool,
        bits: &[i32],
        sent: &[i32],
    ) -> (String, Vec<u8>) {
        let metrics = self
            .metrics
            .iter()
            .zip(bits)
            .enumerate()
            .filter(|(i, (_, v))| all || sent.get(*i) != Some(*v))
            .map(|(i, (m, v))| SpMetric::alias(i as u64 + 1, value_of(m.kind, *v)))
            .collect();
        let payload = Payload {
            timestamp: ts,
            metrics,
            seq: Some(u64::from(seq)),
        };
        (self.topic(MessageType::NData), payload.encode())
    }
}

/// The setup as the scan and the worker share it, with the "table changed" marks.
struct Shared {
    cfg: Cfg,
    /// The table changed: announce it again once it has settled.
    dirty: bool,
    dirty_at: u32,
    /// Counts the changes of the table, so that a change during a publish is not forgotten.
    generation: u32,
}

impl Shared {
    /// The metric table changed (a new metric, a new type).
    fn table_changed(&mut self) {
        self.cfg.version = self.cfg.version.wrapping_add(1);
        self.dirty = true;
        self.dirty_at = uptime_ms() as u32;
        self.generation = self.generation.wrapping_add(1);
    }
}

/// A "ceiling" lock (`board/spin.rs`): the scan takes it for a compare or a push, the worker for a
/// copy (it allocates only when the setup grew); encoding and the broker calls are outside it.
static SP: Spin<Shared> = Spin::new(Shared {
    cfg: Cfg::new(),
    dirty: true,
    dirty_at: 0,
    generation: 0,
});

/// State of the session; read by `spdump` (the scan, a command) and by the steps (op 33), written
/// by the worker only: atomics.
static ONLINE: AtomicBool = AtomicBool::new(false);
static BIRTHS: AtomicU32 = AtomicU32::new(0);
static BD_SEQ_NOW: AtomicU8 = AtomicU8::new(0);
static TOO_BIG: AtomicU32 = AtomicU32::new(0);

/// What the worker thread (`crab-mqtt`) keeps of the node: only that thread touches it.
pub(super) struct Session {
    cfg: Cfg,
    dirty: bool,
    dirty_at: u32,
    generation: u32,
    /// The values of the last message.
    sent: [i32; MAX_METRICS],
    was_up: bool,
    seq: u8,
    last_data: u32,
    /// The journal knows about the current series of too long payloads.
    told_big: bool,
}

impl Session {
    pub(super) fn new() -> Self {
        Self {
            cfg: Cfg::new(),
            dirty: true,
            dirty_at: 0,
            generation: 0,
            sent: [0; MAX_METRICS],
            was_up: false,
            seq: 0,
            last_data: 0,
            told_big: false,
        }
    }

    /// Takes the setup and the marks from the scan's copy: a short section under the lock.
    fn sync(&mut self) {
        let g = SP.lock_ceiling();
        if g.cfg.version != self.cfg.version {
            self.cfg.copy_from(&g.cfg);
        }
        self.dirty = g.dirty;
        self.dirty_at = g.dirty_at;
        self.generation = g.generation;
    }

    /// The will of the client: NDEATH with the current `bdSeq`. `None` while the node is not set up
    /// or not enabled. The broker gets it with CONNECT only, so it is part of the client settings.
    pub(super) fn will(&mut self) -> Option<(String, Vec<u8>)> {
        self.sync();
        self.cfg
            .active()
            .then(|| self.cfg.death(BD_SEQ_NOW.load(Ordering::Acquire)))
    }

    /// Called by the worker every tick before it reads the settings. A session that was up and is
    /// not any more starts a new `bdSeq`; `true` tells the worker to drop the client (its will is
    /// old, and a client that reconnects by itself would bring the node back with it).
    pub(super) fn track(&mut self, connected: bool) -> bool {
        self.sync();
        if !self.cfg.active() {
            self.was_up = false;
            ONLINE.store(false, Ordering::Release);
            return false;
        }
        if connected {
            self.was_up = true;
            return false;
        }
        ONLINE.store(false, Ordering::Release);
        self.end_session()
    }

    /// The master dropped the client itself (new settings, no module wants it): the session is over.
    pub(super) fn ended(&mut self) {
        ONLINE.store(false, Ordering::Release);
        self.end_session();
    }

    /// `bdSeq` grows once per ended session, however many ways lead to the end: `was_up` is the
    /// guard.
    fn end_session(&mut self) -> bool {
        if !self.was_up {
            return false;
        }
        self.was_up = false;
        BD_SEQ_NOW.fetch_add(1, Ordering::AcqRel);
        true
    }

    /// One journal line per series of too long payloads, not one per tick.
    fn tell_big(&mut self) {
        TOO_BIG.fetch_add(1, Ordering::Relaxed);
        if !self.told_big {
            self.told_big = true;
            mqtt::note("sparkplug: payload too big".to_string());
        }
    }

    /// Sends NBIRTH after a connect and NDATA for changed values; as `spPoll` of the C++ master.
    pub(super) fn poll(&mut self, client: &mut EspMqttClient<'static>, connected: bool) {
        let now = uptime_ms() as u32;
        self.sync();
        if !self.cfg.active() || !connected {
            ONLINE.store(false, Ordering::Release);
            return;
        }
        if self.dirty {
            // The metric table changed: announce everything again once it has settled.
            ONLINE.store(false, Ordering::Release);
            if now.wrapping_sub(self.dirty_at) < SETTLE_MS {
                return;
            }
        }
        let ts = net::unix_ms();
        // Values read before publishing: a change during the publish goes into the next message.
        let n = self.cfg.metrics.len().min(MAX_METRICS);
        let mut bits = [0i32; MAX_METRICS];
        for (b, v) in bits.iter_mut().zip(&BITS).take(n) {
            *b = v.load(Ordering::Acquire);
        }
        let bits = &bits[..n];
        let generation = self.generation;
        let bd_seq = BD_SEQ_NOW.load(Ordering::Acquire);
        if !ONLINE.load(Ordering::Acquire) {
            let (topic, bytes) = self.cfg.birth(bd_seq, ts, 0, bits);
            if bytes.len() > MAX_PAYLOAD {
                SP.lock_ceiling().dirty = false;
                self.tell_big();
                return;
            }
            if client
                .enqueue(&topic, QoS::AtMostOnce, false, &bytes)
                .is_err()
            {
                return;
            }
            self.seq = 1;
            ONLINE.store(true, Ordering::Release);
            {
                // A change of the table during the publish needs a new birth.
                let mut g = SP.lock_ceiling();
                g.dirty = g.generation != generation;
            }
            BIRTHS.fetch_add(1, Ordering::AcqRel);
            self.last_data = now;
            self.told_big = false;
            self.sent[..n].copy_from_slice(bits);
            mqtt::note(format!(
                "sparkplug: birth sent ({n} metrics, bdSeq {bd_seq})"
            ));
            return;
        }
        if now.wrapping_sub(self.last_data) < DATA_PERIOD_MS || bits == &self.sent[..n] {
            return;
        }
        let (topic, bytes) = self.cfg.data(ts, self.seq, false, bits, &self.sent[..n]);
        if bytes.len() > MAX_PAYLOAD {
            self.tell_big();
            return;
        }
        if client
            .enqueue(&topic, QoS::AtMostOnce, false, &bytes)
            .is_err()
        {
            return;
        }
        self.seq = self.seq.wrapping_add(1);
        self.last_data = now;
        self.told_big = false;
        self.sent[..n].copy_from_slice(bits);
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
        // Every step, per metric: atomics, not the lock (about 5 µs on the ESP32). The worker
        // takes the values in `poll`; the session state is atomics too.
        32 => {
            if let Some(v) = usize::try_from(a).ok().and_then(|i| BITS.get(i)) {
                v.store(b, Ordering::Release);
            }
            0
        }
        33 => match a {
            0 => i32::from(ONLINE.load(Ordering::Acquire)),
            1 => BIRTHS.load(Ordering::Acquire) as i32,
            _ => 0,
        },
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
            let Some(bytes) = mem.slice(b, (c.max(0) as usize).min(MAX_NAME)) else {
                return 0;
            };
            let mut sp = SP.lock_scan();
            let field = if code == 1 {
                &mut sp.cfg.group
            } else {
                &mut sp.cfg.node
            };
            // Names from variables come every step: a new string only when the text changed.
            if field.as_bytes() != bytes {
                let text = name_of(bytes);
                if *field != text {
                    *field = text;
                    sp.cfg.version = sp.cfg.version.wrapping_add(1);
                }
            }
        }
        4 => {
            let mut sp = SP.lock_scan();
            let on = b != 0;
            if sp.cfg.enable != on {
                sp.cfg.enable = on;
                sp.cfg.version = sp.cfg.version.wrapping_add(1);
            }
        }
        63 => {
            let mut sp = SP.lock_scan();
            if !sp.cfg.complete {
                sp.cfg.complete = true;
                sp.cfg.version = sp.cfg.version.wrapping_add(1);
            }
        }
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
    let mut sp = SP.lock_scan();
    if let Some(i) = sp.cfg.metrics.iter().position(|m| m.name == name) {
        if sp.cfg.metrics[i].kind != kind {
            sp.cfg.metrics[i].kind = kind;
            sp.table_changed();
        }
        return i as i32;
    }
    if sp.cfg.metrics.len() >= MAX_METRICS {
        return -1;
    }
    sp.cfg.metrics.push(Def { name, kind });
    sp.table_changed();
    (sp.cfg.metrics.len() - 1) as i32
}

// ---------------------------------------------------------------------------------------------
// The session
// ---------------------------------------------------------------------------------------------

/// Values of the metrics as the steps write them (op 32).
static BITS: [AtomicI32; MAX_METRICS] = [const { AtomicI32::new(0) }; MAX_METRICS];

/// `spdump <ms>`: NDEATH, NBIRTH and NDATA (all metrics) as hex, nothing is sent. Runs on the scan
/// (a command): the setup is copied under the lock, the encoding and the printing come after it.
pub(super) fn dump(ts_ms: u64) {
    let cfg = {
        let sp = SP.lock_scan();
        sp.cfg.clone()
    };
    let n = cfg.metrics.len().min(MAX_METRICS);
    let mut bits = [0i32; MAX_METRICS];
    for (b, v) in bits.iter_mut().zip(&BITS).take(n) {
        *b = v.load(Ordering::Acquire);
    }
    let bits = &bits[..n];
    let bd_seq = BD_SEQ_NOW.load(Ordering::Acquire);
    let messages = [
        cfg.death(bd_seq),
        cfg.birth(bd_seq, Some(ts_ms), 0, bits),
        cfg.data(Some(ts_ms), 1, true, bits, &[]),
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
        u8::from(ONLINE.load(Ordering::Acquire)),
        BIRTHS.load(Ordering::Acquire),
        bd_seq,
        cfg.metrics.len(),
        TOO_BIG.load(Ordering::Relaxed)
    );
    println!("ok");
}
