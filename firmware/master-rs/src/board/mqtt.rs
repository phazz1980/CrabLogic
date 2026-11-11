//! MQTT client of the master (`specs/wasm-kernel/plan.md`, 4.1б). Modules reach it through the `net`
//! import, ops 9..=16; the ABI is the one of the C++ master (`specs/wasm-on-mcu/proto/master`).
//!
//! The import only writes settings, queues publications, registers filters and reads the last message
//! of a filter, so a module step never waits for the broker. The thread `crab-mqtt` owns the client:
//! it creates it when the settings are complete, the network is up and a module still asks for it,
//! and drops it otherwise. Reconnects are done by the ESP-IDF client itself.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crab_kernel::host::Mem;
use esp_idf_svc::handle::RawHandle;
use esp_idf_svc::mqtt::client::{
    Details, EspMqttClient, EspMqttEvent, EventPayload, MqttClientConfiguration, QoS,
};
use esp_idf_svc::sys::{self, EspError};

use super::devices::uptime_ms;
use super::net::{self, LEASE_MS};

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

struct Publication {
    topic: String,
    payload: Vec<u8>,
    retain: bool,
}

struct Mqtt {
    /// Broker, client id, user, password.
    text: [String; 4],
    port: i32,
    keep_alive: i32,
    /// 0 Wi-Fi, 1 Ethernet (not supported yet, treated as Wi-Fi).
    network: i32,
    max_slots: i32,
    /// Op 9 code 63 came.
    complete: bool,
    /// Allocated on the first subscription.
    slots: Vec<Slot>,
    queue: VecDeque<Publication>,
}

static MQTT: Mutex<Mqtt> = Mutex::new(Mqtt {
    text: [String::new(), String::new(), String::new(), String::new()],
    port: 1883,
    keep_alive: 60,
    network: 0,
    max_slots: 4,
    complete: false,
    slots: Vec::new(),
    queue: VecDeque::new(),
});
/// Wakes the thread when something is queued.
static WAKE: Condvar = Condvar::new();
static STARTED: AtomicBool = AtomicBool::new(false);
/// Board uptime (ms) of the last call of ops 11..=16.
static LAST_CALL_MS: AtomicU32 = AtomicU32::new(0);
static CONNECTED: AtomicBool = AtomicBool::new(false);
static ERROR: AtomicI32 = AtomicI32::new(0);

/// Messages for the master journal (see `net::take_notes`).
static NOTES: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
const MAX_NOTES: usize = 16;

macro_rules! note {
    ($($arg:tt)*) => { push_note(format!($($arg)*)) };
}

fn push_note(text: String) {
    let mut q = NOTES.lock().unwrap_or_else(|e| e.into_inner());
    if q.len() == MAX_NOTES {
        q.pop_front();
    }
    q.push_back(text);
}

/// Takes the queued MQTT messages (for the journal).
pub fn take_notes() -> Vec<String> {
    let mut q = NOTES.lock().unwrap_or_else(|e| e.into_inner());
    q.drain(..).collect()
}

fn lock() -> MutexGuard<'static, Mqtt> {
    MQTT.lock().unwrap_or_else(|e| e.into_inner())
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
        16 => {
            let m = lock();
            m.slots.get(a as usize).map_or(0, |s| s.count as i32)
        }
        _ => 0,
    }
}

fn set_number(code: i32, value: i32) -> i32 {
    {
        let mut m = lock();
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
    let spawned = std::thread::Builder::new()
        .name("crab-mqtt".into())
        .stack_size(MQTT_STACK)
        .spawn(worker);
    if let Err(e) = spawned {
        note!("cannot start the MQTT thread: {e}");
        STARTED.store(false, Ordering::Release);
    }
}

fn set_text(code: i32, ptr: i32, len: i32, mem: &Mem<'_>) -> i32 {
    let Some(bytes) = mem.read(ptr, len, MAX_SETTING) else {
        return 0;
    };
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let mut m = lock();
    if let Some(field) = usize::try_from(code - 1)
        .ok()
        .and_then(|i| m.text.get_mut(i))
    {
        if *field != text {
            *field = text;
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
        mem.read(tp, tl, MAX_TOPIC + 1),
        mem.read(pp, pl, MAX_PAYLOAD),
    ) else {
        return 0;
    };
    if topic.is_empty() || topic.len() > MAX_TOPIC || topic.iter().any(|&b| b == b'+' || b == b'#')
    {
        set_publish_error(error::BAD_TOPIC);
        return 0;
    }
    if !CONNECTED.load(Ordering::Acquire) {
        set_publish_error(error::NOT_CONNECTED);
        return 0;
    }
    {
        let mut m = lock();
        if m.queue.len() >= QUEUE_LEN {
            return 0;
        }
        m.queue.push_back(Publication {
            topic: String::from_utf8_lossy(&topic).into_owned(),
            payload,
            retain,
        });
    }
    // A topic or link error is over once a message goes through.
    let _ = ERROR.compare_exchange(error::BAD_TOPIC, 0, Ordering::AcqRel, Ordering::Relaxed);
    let _ = ERROR.compare_exchange(error::NOT_CONNECTED, 0, Ordering::AcqRel, Ordering::Relaxed);
    WAKE.notify_one();
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
    let mut m = lock();
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
    WAKE.notify_one();
    i as i32
}

fn slot_text(slot: i32, ptr: i32, cap: i32, mem: &mut Mem<'_>) -> i32 {
    let text = {
        let m = lock();
        let Some(s) = m.slots.get(slot as usize) else {
            return 0;
        };
        let n = s.text.len().min(cap.max(0) as usize);
        s.text[..n].to_vec()
    };
    if text.is_empty() || !mem.write(ptr, &text) {
        return 0;
    }
    text.len() as i32
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

/// Runs on the esp-mqtt task: copies the message into the slots of the matching filters.
fn on_event(payload: EventPayload<'_, EspError>) {
    match payload {
        EventPayload::Connected(_) => {
            let mut m = lock();
            for s in &mut m.slots {
                s.subscribed = false;
            }
            drop(m);
            ERROR.store(error::NONE, Ordering::Release);
            CONNECTED.store(true, Ordering::Release);
        }
        EventPayload::Disconnected => CONNECTED.store(false, Ordering::Release),
        EventPayload::Received {
            topic: Some(topic),
            data,
            details,
            ..
        } => {
            if matches!(details, Details::SubsequentChunk(_)) {
                return;
            }
            let data = &data[..data.len().min(MAX_PAYLOAD)];
            let mut m = lock();
            for s in m.slots.iter_mut().filter(|s| matches(&s.filter, topic)) {
                s.text.clear();
                s.text.extend_from_slice(data);
                s.count = s.count.wrapping_add(1);
            }
        }
        _ => {}
    }
}

/// Handler of MQTT_EVENT_ERROR: the safe wrapper hides the error details, so the reason is read here.
unsafe extern "C" fn on_error(
    _arg: *mut core::ffi::c_void,
    _base: sys::esp_event_base_t,
    _id: i32,
    data: *mut core::ffi::c_void,
) {
    // SAFETY: for this event the data is the `esp_mqtt_event_t` of the client, valid during the call;
    // `error_handle` is checked for null before it is read.
    let code = unsafe {
        let event = data as *const sys::esp_mqtt_event_t;
        match event.as_ref().and_then(|e| e.error_handle.as_ref()) {
            Some(h) => match h.error_type {
                sys::esp_mqtt_error_type_t_MQTT_ERROR_TYPE_TCP_TRANSPORT => error::NO_TCP,
                sys::esp_mqtt_error_type_t_MQTT_ERROR_TYPE_CONNECTION_REFUSED => error::REFUSED,
                sys::esp_mqtt_error_type_t_MQTT_ERROR_TYPE_SUBSCRIBE_FAILED => {
                    error::SUBSCRIBE_REJECTED
                }
                _ => return,
            },
            None => return,
        }
    };
    set_error(code);
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
}

fn applied_of(m: &Mqtt) -> Applied {
    Applied {
        broker: m.text[0].trim().to_string(),
        port: m.port,
        keep_alive: m.keep_alive,
        client_id: m.text[1].clone(),
        user: m.text[2].clone(),
        password: m.text[3].clone(),
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

fn start_client(a: &Applied) -> Result<EspMqttClient<'static>, EspError> {
    let url = format!("mqtt://{}:{}", a.broker, a.port);
    let client_id = if a.client_id.is_empty() {
        default_client_id()
    } else {
        a.client_id.clone()
    };
    let conf = MqttClientConfiguration {
        client_id: Some(&client_id),
        username: (!a.user.is_empty()).then_some(a.user.as_str()),
        password: (!a.user.is_empty()).then_some(a.password.as_str()),
        keep_alive_interval: (a.keep_alive > 0).then(|| Duration::from_secs(a.keep_alive as u64)),
        ..Default::default()
    };
    let client = EspMqttClient::new_cb(&url, &conf, |event: EspMqttEvent<'_>| {
        on_event(event.payload())
    })?;
    // SAFETY: the handle is valid while `client` lives, and the handler is a plain function without
    // captured state (the argument is not used).
    let registered = unsafe {
        sys::esp_mqtt_client_register_event(
            client.handle(),
            sys::esp_mqtt_event_id_t_MQTT_EVENT_ERROR,
            Some(on_error),
            core::ptr::null_mut(),
        )
    };
    if registered != 0 {
        note!("cannot watch client errors (code {registered})");
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
    ethernet: bool,
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
        let todo: Vec<(usize, String)> = lock()
            .slots
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.subscribed && !s.filter.is_empty())
            .map(|(i, s)| (i, s.filter.clone()))
            .collect();
        for (i, filter) in todo {
            match live.client.subscribe(&filter, QoS::AtMostOnce) {
                Ok(_) => {
                    if let Some(s) = lock().slots.get_mut(i) {
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
        let Some(p) = lock().queue.pop_front() else {
            break;
        };
        if let Err(e) = live
            .client
            .enqueue(&p.topic, QoS::AtMostOnce, p.retain, &p.payload)
        {
            lock().queue.push_front(p);
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
        ethernet: false,
        failure: String::new(),
    };
    let mut retry_at = Instant::now();
    loop {
        let (applied, complete, network) = {
            let guard = lock();
            let (guard, _) = WAKE
                .wait_timeout(guard, TICK)
                .unwrap_or_else(|e| e.into_inner());
            (applied_of(&guard), guard.complete, guard.network)
        };
        if network == 1 && !told.ethernet {
            told.ethernet = true;
            note!("Ethernet is not supported yet, using Wi-Fi");
        }
        let idle =
            (uptime_ms() as u32).wrapping_sub(LAST_CALL_MS.load(Ordering::Acquire)) > LEASE_MS;
        let wanted = complete && !idle && net::link_ready();
        if !wanted {
            if live.take().is_some() {
                note!("client stopped");
            }
            CONNECTED.store(false, Ordering::Release);
        } else {
            if live.as_ref().is_some_and(|l| l.applied != applied) {
                live = None;
                CONNECTED.store(false, Ordering::Release);
                note!("settings changed, restarting the client");
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
        if CONNECTED.load(Ordering::Acquire) {
            if let Some(l) = live.as_mut() {
                serve(l, &mut told, &mut retry_at);
            }
        }
    }
}
