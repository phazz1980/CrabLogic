//! MQTT client of the master (`specs/wasm-kernel/plan.md`, 4.1б). Modules reach it through the `net`
//! import, ops 9..=16; the ABI is fixed in `crab-kernel` (`abi.rs`).
//!
//! The import only writes settings, queues publications, registers filters and reads the last message
//! of a filter, so a module step never waits for the broker. The thread `crab-mqtt` owns the client:
//! it creates it when the settings are complete, the network is up and a module still asks for it,
//! and drops it otherwise. Reconnects are done by the ESP-IDF client itself, except with a Sparkplug
//! will (see `start_client`).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use crab_kernel::host::Mem;
use esp_idf_svc::handle::RawHandle;
use esp_idf_svc::mqtt::client::{EspMqttClient, LwtConfiguration, MqttClientConfiguration, QoS};
use esp_idf_svc::sys::{self, EspError};

use super::devices::uptime_ms;
use super::eth;
use super::net::{self, LEASE_MS};
use super::note_queue::{dropped_text, NoteQueue};
use super::sparkplug;
use super::spin::Spin;

/// Stack of `crab-mqtt`.
const MQTT_STACK: usize = 8192;
/// How often the thread looks at the settings and the link (publish and subscribe wake it sooner).
const TICK: Duration = Duration::from_millis(200);
/// Not connected for this long without any other error: error 6.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Pause after a failed client start or subscription.
const RETRY_PAUSE: Duration = Duration::from_secs(5);
/// Longest text setting (broker, client id, user, password).
const MAX_SETTING: usize = 64;
const MAX_TOPIC: usize = 127;
const MAX_PAYLOAD: usize = 128;
/// How long the worker sleeps at most between two rounds, FreeRTOS ticks (`TICK`).
const WAIT_TICKS: u32 = TICK.as_millis() as u32 * sys::configTICK_RATE_HZ / 1000;
const QUEUE_LEN: usize = 4;
const MAX_SLOTS: i32 = 16;

mod error {
    pub const NONE: i32 = 0;
    pub const NO_BROKER: i32 = 2;
    pub const CONNECT_TIMEOUT: i32 = 6;
    pub const NO_TCP: i32 = 7;
    pub const REFUSED: i32 = 10;
    pub const SUBSCRIBE_REJECTED: i32 = 11;
    pub const BAD_TOPIC: i32 = 14;
    pub const NOT_CONNECTED: i32 = 15;
}

struct Slot {
    filter: String,
    text: Vec<u8>,
    count: u32,
    subscribed: bool,
}

/// A queued message: fixed buffers, so a publish of the step allocates nothing (the queue reaches
/// its `QUEUE_LEN` once and keeps that room).
struct Publication {
    topic: [u8; MAX_TOPIC],
    topic_len: u8,
    payload: [u8; MAX_PAYLOAD],
    payload_len: u8,
    retain: bool,
}

impl Publication {
    /// The topic was checked as UTF-8 when queued.
    fn topic(&self) -> &str {
        core::str::from_utf8(&self.topic[..usize::from(self.topic_len)]).unwrap_or("")
    }

    fn payload(&self) -> &[u8] {
        &self.payload[..usize::from(self.payload_len)]
    }
}

struct Mqtt {
    /// Broker, client id, user, password.
    text: [String; 4],
    port: i32,
    keep_alive: i32,
    /// 0 Wi-Fi, 1 Ethernet.
    network: i32,
    max_slots: i32,
    /// Op 9 code 63 came.
    complete: bool,
    /// Allocated on the first subscription.
    slots: Vec<Slot>,
    queue: VecDeque<Publication>,
}

/// Shared by the scan (settings, publications, filters, the last message of a filter), the worker
/// `crab-mqtt` and the esp-mqtt task (`on_data`). A "ceiling" lock (`board/spin.rs`): every section
/// of the other two is a copy or a compare of a few bytes; the client calls are outside it.
static MQTT: Spin<Mqtt> = Spin::new(Mqtt {
    text: [String::new(), String::new(), String::new(), String::new()],
    port: 1883,
    keep_alive: 60,
    network: 0,
    max_slots: 4,
    complete: false,
    slots: Vec::new(),
    queue: VecDeque::new(),
});
/// The FreeRTOS task of the worker: the scan wakes it with a task notification (it never blocks).
static TASK: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(core::ptr::null_mut());
static STARTED: AtomicBool = AtomicBool::new(false);
/// Board uptime (ms) of the last call of ops 11..=16.
static LAST_CALL_MS: AtomicU32 = AtomicU32::new(0);
static CONNECTED: AtomicBool = AtomicBool::new(false);
/// Messages received per subscription slot (a copy of `Slot::count` for the steps).
static COUNTS: [AtomicU32; MAX_SLOTS as usize] = [const { AtomicU32::new(0) }; MAX_SLOTS as usize];
static ERROR: AtomicI32 = AtomicI32::new(0);

/// Messages for the master journal (see `net::take_notes`).
static NOTES: NoteQueue<String> = NoteQueue::new();

/// A note from the worker or the esp-mqtt task (waits for the queue).
macro_rules! note {
    ($($arg:tt)*) => { NOTES.push_wait(format!($($arg)*)) };
}

/// A line for the journal from the Sparkplug node (the worker).
pub(super) fn note(text: String) {
    NOTES.push_wait(text);
}

/// Wakes the worker: something is queued.
fn wake() {
    let task = TASK.load(Ordering::Acquire);
    if !task.is_null() {
        // SAFETY: the task of the worker never ends once started.
        unsafe {
            sys::xTaskGenericNotify(
                task.cast(),
                0,
                1,
                sys::eNotifyAction_eSetBits,
                core::ptr::null_mut(),
            )
        };
    }
}

/// A module is still there: keeps the client alive (ops that are not 11..=16 call it themselves).
pub(super) fn touch() {
    LAST_CALL_MS.store(uptime_ms() as u32, Ordering::Release);
}

/// Takes the queued MQTT messages (for the journal); the scan calls it between steps and never
/// waits (a busy queue gives nothing this time).
pub fn take_notes() -> Vec<String> {
    let (mut lines, dropped) = NOTES.take_try();
    if dropped > 0 {
        lines.push(dropped_text(dropped));
    }
    lines
}

// ---------------------------------------------------------------------------------------------
// The import
// ---------------------------------------------------------------------------------------------

/// `net(op, a, b, c, d)` for the MQTT operations (9..=16).
pub fn call(op: i32, a: i32, b: i32, c: i32, d: i32, mem: &mut Mem<'_>) -> i32 {
    if op >= 11 {
        LAST_CALL_MS.store(uptime_ms() as u32, Ordering::Release);
    }
    match op {
        9 => set_number(a, b),
        10 => set_text(a, b, c, mem),
        11 => match a {
            0 => i32::from(CONNECTED.load(Ordering::Acquire)),
            1 => ERROR.load(Ordering::Acquire),
            _ => 0,
        },
        12 => publish(a, b, c, d, false, mem),
        13 => publish(a, b, c, d, true, mem),
        14 => subscribe(a, b, mem),
        15 => slot_text(a, b, c, mem),
        // Read every step by every subscription: an atomic, not the lock (about 5 µs).
        16 => usize::try_from(a)
            .ok()
            .and_then(|i| COUNTS.get(i))
            .map_or(0, |c| c.load(Ordering::Acquire) as i32),
        _ => 0,
    }
}

fn set_number(code: i32, value: i32) -> i32 {
    {
        let mut m = MQTT.lock_scan();
        match code {
            1 => m.port = value,
            2 => m.keep_alive = value,
            3 => m.network = value,
            4 => m.max_slots = value.clamp(1, MAX_SLOTS),
            63 => m.complete = true,
            _ => {}
        }
    }
    if code == 63 {
        LAST_CALL_MS.store(uptime_ms() as u32, Ordering::Release);
        start_thread();
    }
    0
}

fn start_thread() {
    if STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    if !super::spawn_background(b"crab-mqtt\0", MQTT_STACK, worker) {
        // The scan side: the note is dropped (and counted) when the queue is busy.
        NOTES.push_try("cannot start the MQTT thread".to_string());
        STARTED.store(false, Ordering::Release);
    }
}

fn set_text(code: i32, ptr: i32, len: i32, mem: &Mem<'_>) -> i32 {
    let Some(bytes) = mem.slice(ptr, (len.max(0) as usize).min(MAX_SETTING)) else {
        return 0;
    };
    let mut m = MQTT.lock_scan();
    if let Some(field) = usize::try_from(code - 1)
        .ok()
        .and_then(|i| m.text.get_mut(i))
    {
        // Settings from variables come every step: a new string only when the text changed.
        if field.as_bytes() != bytes {
            *field = String::from_utf8_lossy(bytes).into_owned();
        }
    }
    0
}

fn set_error(code: i32) {
    ERROR.store(code, Ordering::Release);
}

/// A publish error (14, 15) does not hide a connection error (6, 7, 10...): the module retries every
/// step, so it would overwrite the reason before anyone saw it.
fn set_publish_error(code: i32) {
    let mut current = ERROR.load(Ordering::Acquire);
    while matches!(
        current,
        error::NONE | error::BAD_TOPIC | error::NOT_CONNECTED
    ) {
        match ERROR.compare_exchange(current, code, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return,
            Err(now) => current = now,
        }
    }
}

fn publish(tp: i32, tl: i32, pp: i32, pl: i32, retain: bool, mem: &Mem<'_>) -> i32 {
    let (Some(topic), Some(payload)) = (
        mem.slice(tp, (tl.max(0) as usize).min(MAX_TOPIC + 1)),
        mem.slice(pp, (pl.max(0) as usize).min(MAX_PAYLOAD)),
    ) else {
        return 0;
    };
    if topic.is_empty()
        || topic.len() > MAX_TOPIC
        || topic.iter().any(|&b| b == b'+' || b == b'#')
        || core::str::from_utf8(topic).is_err()
    {
        set_publish_error(error::BAD_TOPIC);
        return 0;
    }
    if !CONNECTED.load(Ordering::Acquire) {
        set_publish_error(error::NOT_CONNECTED);
        return 0;
    }
    {
        let mut m = MQTT.lock_scan();
        if m.queue.len() >= QUEUE_LEN {
            return 0;
        }
        let mut p = Publication {
            topic: [0; MAX_TOPIC],
            topic_len: topic.len() as u8,
            payload: [0; MAX_PAYLOAD],
            payload_len: payload.len() as u8,
            retain,
        };
        p.topic[..topic.len()].copy_from_slice(topic);
        p.payload[..payload.len()].copy_from_slice(payload);
        m.queue.push_back(p);
    }
    // A topic or link error is over once a message goes through.
    let _ = ERROR.compare_exchange(error::BAD_TOPIC, 0, Ordering::AcqRel, Ordering::Relaxed);
    let _ = ERROR.compare_exchange(error::NOT_CONNECTED, 0, Ordering::AcqRel, Ordering::Relaxed);
    wake();
    1
}

/// The longest prefix of `bytes` that fits `max` bytes and is valid UTF-8 at its end.
fn text_of(bytes: &[u8], max: usize) -> String {
    let mut s = String::from_utf8_lossy(&bytes[..bytes.len().min(max)]).into_owned();
    // A cut inside a character becomes U+FFFD; drop it.
    if bytes.len() > max && s.ends_with('\u{FFFD}') {
        s.pop();
    }
    s
}

fn subscribe(fp: i32, fl: i32, mem: &Mem<'_>) -> i32 {
    let Some(bytes) = mem.read(fp, fl, MAX_TOPIC + 1) else {
        return -1;
    };
    let filter = text_of(&bytes, MAX_TOPIC);
    let mut m = MQTT.lock_scan();
    if !m.complete || filter.is_empty() {
        return -1;
    }
    if m.slots.is_empty() {
        let n = m.max_slots.clamp(1, MAX_SLOTS) as usize;
        m.slots = (0..n)
            .map(|_| Slot {
                filter: String::new(),
                text: Vec::new(),
                count: 0,
                subscribed: false,
            })
            .collect();
    }
    if let Some(i) = m.slots.iter().position(|s| s.filter == filter) {
        return i as i32;
    }
    let Some(i) = m.slots.iter().position(|s| s.filter.is_empty()) else {
        return -1;
    };
    m.slots[i].filter = filter;
    drop(m);
    wake();
    i as i32
}

fn slot_text(slot: i32, ptr: i32, cap: i32, mem: &mut Mem<'_>) -> i32 {
    // Copied straight from the slot under the lock: nothing is allocated.
    let m = MQTT.lock_scan();
    let Some(s) = m.slots.get(slot as usize) else {
        return 0;
    };
    let n = s.text.len().min(cap.max(0) as usize);
    if n == 0 || !mem.write(ptr, &s.text[..n]) {
        return 0;
    }
    n as i32
}

// ---------------------------------------------------------------------------------------------
// The client
// ---------------------------------------------------------------------------------------------

/// Topic against a filter with `+` (one level) and `#` (the rest); as `mqtt_match` of `crab-sim`.
fn matches(filter: &str, topic: &str) -> bool {
    let (mut f, mut t) = (filter.split('/'), topic.split('/'));
    loop {
        match (f.next(), t.next()) {
            (Some("#"), _) => return true,
            (Some("+"), Some(_)) => {}
            (Some(a), Some(b)) if a == b => {}
            (None, None) => return true,
            _ => return false,
        }
    }
}

/// Says once per boot that a message with a non UTF-8 topic was dropped.
static BAD_TOPIC_NOTED: AtomicBool = AtomicBool::new(false);

fn on_connected() {
    let mut m = MQTT.lock_ceiling();
    for s in &mut m.slots {
        s.subscribed = false;
    }
    drop(m);
    ERROR.store(error::NONE, Ordering::Release);
    CONNECTED.store(true, Ordering::Release);
}

/// Copies the first chunk of a message into the slots of the matching filters. A topic that is not
/// UTF-8 cannot match a filter of a module (filters are text), so the message is dropped.
fn on_data(topic: &[u8], data: &[u8]) {
    let Ok(topic) = core::str::from_utf8(topic) else {
        if !BAD_TOPIC_NOTED.swap(true, Ordering::AcqRel) {
            note!("message dropped: topic is not UTF-8");
        }
        return;
    };
    let data = &data[..data.len().min(MAX_PAYLOAD)];
    let mut m = MQTT.lock_ceiling();
    for (i, s) in m
        .slots
        .iter_mut()
        .enumerate()
        .filter(|(_, s)| matches(&s.filter, topic))
    {
        s.text.clear();
        s.text.extend_from_slice(data);
        s.count = s.count.wrapping_add(1);
        // After the text: a step that sees the new count finds the new text.
        if let Some(c) = COUNTS.get(i) {
            c.store(s.count, Ordering::Release);
        }
    }
}

fn error_code(h: &sys::esp_mqtt_error_codes_t) -> Option<i32> {
    match h.error_type {
        sys::esp_mqtt_error_type_t_MQTT_ERROR_TYPE_TCP_TRANSPORT => Some(error::NO_TCP),
        sys::esp_mqtt_error_type_t_MQTT_ERROR_TYPE_CONNECTION_REFUSED => Some(error::REFUSED),
        sys::esp_mqtt_error_type_t_MQTT_ERROR_TYPE_SUBSCRIBE_FAILED => {
            Some(error::SUBSCRIBE_REJECTED)
        }
        _ => None,
    }
}

/// Raw handler of the client events, run on the esp-mqtt task. The safe event of esp-idf-svc 0.51 is
/// not used: its payload hides the error details and panics on a topic that is not UTF-8
/// (`from_utf8(..).unwrap()`), which would let any client of the broker reboot the board.
unsafe extern "C" fn on_raw(
    _arg: *mut core::ffi::c_void,
    _base: sys::esp_event_base_t,
    id: i32,
    data: *mut core::ffi::c_void,
) {
    // SAFETY: for client events the data is the `esp_mqtt_event_t` of the client, valid during the call.
    let Some(e) = (unsafe { (data as *const sys::esp_mqtt_event_t).as_ref() }) else {
        return;
    };
    match id {
        sys::esp_mqtt_event_id_t_MQTT_EVENT_CONNECTED => on_connected(),
        sys::esp_mqtt_event_id_t_MQTT_EVENT_DISCONNECTED => {
            CONNECTED.store(false, Ordering::Release)
        }
        sys::esp_mqtt_event_id_t_MQTT_EVENT_DATA => {
            // Later chunks of a long message carry no topic: only the first chunk is kept.
            if e.current_data_offset != 0 || e.topic.is_null() {
                return;
            }
            // SAFETY: the client keeps `topic_len` bytes at `topic` and `data_len` bytes at `data`
            // during the call; null and negative lengths are checked first.
            let (topic, data) = unsafe {
                let topic = core::slice::from_raw_parts(
                    e.topic as *const u8,
                    usize::try_from(e.topic_len).unwrap_or(0),
                );
                let data = match usize::try_from(e.data_len) {
                    Ok(len) if len > 0 && !e.data.is_null() => {
                        core::slice::from_raw_parts(e.data as *const u8, len)
                    }
                    _ => &[],
                };
                (topic, data)
            };
            on_data(topic, data);
        }
        sys::esp_mqtt_event_id_t_MQTT_EVENT_ERROR => {
            // SAFETY: `error_handle` is checked for null by `as_ref` and lives during the call.
            if let Some(code) = unsafe { e.error_handle.as_ref() }.and_then(error_code) {
                set_error(code);
            }
        }
        _ => {}
    }
}

/// The part of the settings that needs a new client.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Applied {
    broker: String,
    port: i32,
    keep_alive: i32,
    client_id: String,
    user: String,
    password: String,
    /// Sparkplug NDEATH (topic, payload); the broker takes it with CONNECT only, so a new one needs a
    /// new client.
    will: Option<(String, Vec<u8>)>,
}

/// The settings the worker works with: a copy made under the lock (`Mqtt::copy_settings`), so the
/// `Applied` strings are built outside it.
#[derive(Default)]
struct Settings {
    text: [String; 4],
    port: i32,
    keep_alive: i32,
    network: i32,
    complete: bool,
}

impl Mqtt {
    /// Copies the settings reusing what `dst` has allocated.
    fn copy_settings(&self, dst: &mut Settings) {
        for (d, s) in dst.text.iter_mut().zip(&self.text) {
            d.clone_from(s);
        }
        dst.port = self.port;
        dst.keep_alive = self.keep_alive;
        dst.network = self.network;
        dst.complete = self.complete;
    }
}

fn applied_of(m: &Settings) -> Applied {
    Applied {
        broker: m.text[0].trim().to_string(),
        port: m.port,
        keep_alive: m.keep_alive,
        client_id: m.text[1].clone(),
        user: m.text[2].clone(),
        password: m.text[3].clone(),
        will: None,
    }
}

fn default_client_id() -> String {
    let mut mac = [0u8; 6];
    // SAFETY: the buffer has the 6 bytes the call writes.
    let ok = unsafe { sys::esp_read_mac(mac.as_mut_ptr(), sys::esp_mac_type_t_ESP_MAC_WIFI_STA) };
    if ok != 0 {
        return "crab-master".to_string();
    }
    format!("crab-{:02x}{:02x}{:02x}", mac[3], mac[4], mac[5])
}

/// The settings now; the will comes from the Sparkplug node (its lock is taken after the MQTT lock
/// is released, so the two never nest).
fn snapshot(cfg: &mut Settings, sp: &mut sparkplug::Session) -> (Applied, bool, i32) {
    MQTT.lock_ceiling().copy_settings(cfg);
    let mut applied = applied_of(cfg);
    applied.will = sp.will();
    (applied, cfg.complete, cfg.network)
}

fn start_client(a: &Applied) -> Result<EspMqttClient<'static>, EspError> {
    let url = format!("mqtt://{}:{}", a.broker, a.port);
    let client_id = if a.client_id.is_empty() {
        default_client_id()
    } else {
        a.client_id.clone()
    };
    let lwt = a.will.as_ref().map(|(topic, payload)| LwtConfiguration {
        topic,
        payload,
        qos: QoS::AtMostOnce,
        retain: false,
    });
    let conf = MqttClientConfiguration {
        client_id: Some(&client_id),
        lwt,
        // With a will the ESP-IDF reconnect is off: it would bring the node back with the old
        // `bdSeq`. The worker drops the client when the link is lost (`sparkplug::track`) and makes a
        // new one with the next `bdSeq`; a client that never got connected is renewed after
        // `RETRY_PAUSE`.
        reconnect_timeout: a.will.is_none().then(|| Duration::from_secs(0)),
        username: (!a.user.is_empty()).then_some(a.user.as_str()),
        password: (!a.user.is_empty()).then_some(a.password.as_str()),
        keep_alive_interval: (a.keep_alive > 0).then(|| Duration::from_secs(a.keep_alive as u64)),
        ..Default::default()
    };
    // Events are read by `on_raw`; the safe callback must not touch the payload (see `on_raw`).
    let client = EspMqttClient::new_cb(&url, &conf, |_event| {})?;
    // SAFETY: the handle is valid while `client` lives, and the handler is a plain function without
    // captured state (the argument is not used).
    let registered = unsafe {
        sys::esp_mqtt_client_register_event(
            client.handle(),
            sys::esp_mqtt_event_id_t_MQTT_EVENT_ANY,
            Some(on_raw),
            core::ptr::null_mut(),
        )
    };
    if registered != 0 {
        note!("cannot watch client events (code {registered})");
    }
    Ok(client)
}

fn reason(code: i32) -> &'static str {
    match code {
        error::NO_BROKER => "no broker set",
        error::CONNECT_TIMEOUT => "connect timeout",
        error::NO_TCP => "no TCP connection to the broker",
        error::REFUSED => "broker refused the connection",
        error::SUBSCRIBE_REJECTED => "broker rejected a subscription",
        error::BAD_TOPIC => "invalid topic",
        error::NOT_CONNECTED => "publish while not connected",
        _ => "error",
    }
}

struct Live {
    client: EspMqttClient<'static>,
    applied: Applied,
    /// Since when the client is not connected (creation or the last disconnect).
    down_since: Instant,
}

/// What the journal has been told, so that it gets changes only.
struct Told {
    connected: bool,
    error: i32,
    failure: String,
}

impl Told {
    fn failure(&mut self, text: String) {
        if self.failure != text {
            note!("{text}");
            self.failure = text;
        }
    }
}

fn watch(told: &mut Told, live: &mut Option<Live>, broker: &str) {
    let connected = CONNECTED.load(Ordering::Acquire);
    if connected != told.connected {
        told.connected = connected;
        if connected {
            note!("connected to {broker}");
            told.failure.clear();
        } else {
            note!("disconnected");
            if let Some(l) = live.as_mut() {
                l.down_since = Instant::now();
            }
        }
    }
    if !connected
        && matches!(
            ERROR.load(Ordering::Acquire),
            error::NONE | error::NOT_CONNECTED
        )
    {
        if let Some(l) = live.as_ref() {
            if l.down_since.elapsed() > CONNECT_TIMEOUT {
                set_error(error::CONNECT_TIMEOUT);
            }
        }
    }
    let error = ERROR.load(Ordering::Acquire);
    if error != told.error {
        told.error = error;
        if error != error::NONE {
            note!("error {error}: {}", reason(error));
        }
    }
}

/// Subscribes new filters and sends the queued publications.
fn serve(live: &mut Live, told: &mut Told, retry_at: &mut Instant) {
    if Instant::now() >= *retry_at {
        // One filter at a time: copied under the lock, subscribed without it.
        let mut filter = String::new();
        loop {
            let pending = {
                let m = MQTT.lock_ceiling();
                m.slots
                    .iter()
                    .position(|s| !s.subscribed && !s.filter.is_empty())
                    .inspect(|&i| {
                        filter.clear();
                        filter.push_str(&m.slots[i].filter);
                    })
            };
            let Some(i) = pending else { break };
            match live.client.subscribe(&filter, QoS::AtMostOnce) {
                Ok(_) => {
                    if let Some(s) = MQTT.lock_ceiling().slots.get_mut(i) {
                        s.subscribed = true;
                    }
                }
                Err(e) => {
                    told.failure(format!("subscribe {filter:?} failed: {e}"));
                    *retry_at = Instant::now() + RETRY_PAUSE;
                    break;
                }
            }
        }
    }
    // The lock is not held while enqueueing: the client task may be waiting for it in `on_event`.
    loop {
        let Some(p) = MQTT.lock_ceiling().queue.pop_front() else {
            break;
        };
        if let Err(e) = live
            .client
            .enqueue(p.topic(), QoS::AtMostOnce, p.retain, p.payload())
        {
            MQTT.lock_ceiling().queue.push_front(p);
            told.failure(format!("publish failed: {e}"));
            break;
        }
    }
}

fn worker() {
    let mut live: Option<Live> = None;
    let mut told = Told {
        connected: false,
        error: 0,
        failure: String::new(),
    };
    let mut retry_at = Instant::now();
    let mut cfg = Settings::default();
    let mut sp = sparkplug::Session::new();
    // SAFETY: the handle of the calling task.
    TASK.store(
        unsafe { sys::xTaskGetCurrentTaskHandle() }.cast(),
        Ordering::Release,
    );
    loop {
        // Sleeps until the scan queues something, `TICK` at most (a lost notification only delays).
        // SAFETY: waits on the notification of the calling task.
        unsafe { sys::ulTaskGenericNotifyTake(0, 1, WAIT_TICKS) };
        // A Sparkplug session that ended starts a new `bdSeq` before the settings (and the will)
        // are read for the next client.
        if sp.track(CONNECTED.load(Ordering::Acquire)) && live.take().is_some() {
            CONNECTED.store(false, Ordering::Release);
            note!("sparkplug: session ended, restarting the client");
        }
        let (mut applied, complete, network) = snapshot(&mut cfg, &mut sp);
        let idle =
            (uptime_ms() as u32).wrapping_sub(LAST_CALL_MS.load(Ordering::Acquire)) > LEASE_MS;
        let wanted = complete
            && !idle
            && if network == 1 {
                eth::ready()
            } else {
                net::link_ready()
            };
        if !wanted {
            if live.take().is_some() {
                sp.ended();
                note!("client stopped");
            }
            CONNECTED.store(false, Ordering::Release);
        } else {
            if live.as_ref().is_some_and(|l| l.applied != applied) {
                live = None;
                CONNECTED.store(false, Ordering::Release);
                note!("settings changed, restarting the client");
                // A session that was up ends here: the new client has the next `bdSeq`.
                sp.ended();
                applied = snapshot(&mut cfg, &mut sp).0;
            }
            // A client without auto reconnect (see `start_client`) that is not connected for a while
            // is renewed; its will is still valid, so `bdSeq` stays.
            if applied.will.is_some()
                && !CONNECTED.load(Ordering::Acquire)
                && live
                    .as_ref()
                    .is_some_and(|l| l.down_since.elapsed() > RETRY_PAUSE)
            {
                live = None;
                note!("sparkplug: no connection, restarting the client");
            }
            if live.is_none() && Instant::now() >= retry_at {
                if applied.broker.is_empty() {
                    set_error(error::NO_BROKER);
                } else {
                    let _ = ERROR.compare_exchange(
                        error::NO_BROKER,
                        0,
                        Ordering::AcqRel,
                        Ordering::Relaxed,
                    );
                    match start_client(&applied) {
                        Ok(client) => {
                            note!("client started, broker {}:{}", applied.broker, applied.port);
                            live = Some(Live {
                                client,
                                applied: applied.clone(),
                                down_since: Instant::now(),
                            });
                        }
                        Err(e) => {
                            told.failure(format!("cannot start the client: {e}"));
                            retry_at = Instant::now() + RETRY_PAUSE;
                        }
                    }
                }
            }
        }
        let broker = format!("{}:{}", applied.broker, applied.port);
        watch(&mut told, &mut live, &broker);
        let connected = CONNECTED.load(Ordering::Acquire);
        if let Some(l) = live.as_mut() {
            if connected {
                serve(l, &mut told, &mut retry_at);
            }
            sp.poll(&mut l.client, connected);
        }
    }
}
