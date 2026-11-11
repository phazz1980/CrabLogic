//! Wi-Fi and NTP of the master (`specs/wasm-kernel/plan.md`, 4.1а). Modules reach this through the
//! `net` import; the ABI is in `specs/wasm-io/network-plan.md`.
//!
//! The import only writes the wanted settings and reads the last snapshot, so a module step never
//! waits for the network. The thread `crab-net` owns the driver, compares the wanted settings with
//! what it applied and keeps the snapshot up to date. It starts on the first radio request.

use std::collections::VecDeque;
use std::net::Ipv4Addr;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crab_kernel::host::Mem;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::ipv4;
use esp_idf_svc::netif::{EspNetif, NetifConfiguration, NetifStack};
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sntp::{EspSntp, SntpConf};
use esp_idf_svc::sys::{self, EspError};
use esp_idf_svc::wifi::{
    AccessPointConfiguration, AuthMethod, ClientConfiguration, Configuration, EspWifi, WifiEvent,
};

use super::devices::uptime_ms;

/// Stack of `crab-net`: the Wi-Fi driver is initialised on it.
const NET_STACK: usize = 8192;
/// How often the thread looks at the wanted settings and the link.
const TICK: Duration = Duration::from_millis(200);
/// Without an address after this long the attempt counts as failed.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Pause before the next connection attempt.
const RETRY_PAUSE: Duration = Duration::from_secs(5);
/// Wi-Fi reason "no access point with this name was found".
const REASON_NO_AP_FOUND: u32 = 201;
const DEFAULT_NTP_SERVER: &str = "pool.ntp.org";
const DEFAULT_NTP_PERIOD_MS: u32 = 3_600_000;
/// SNTP does not accept a shorter period.
const MIN_NTP_PERIOD_MS: u32 = 15_000;
/// Longest text setting a module may send.
const MAX_TEXT: usize = 128;
/// Without a radio request (op 2) for this long no module wants the network any more (all were
/// deleted or stopped), so the radio goes off.
const LEASE_MS: u32 = 10_000;

mod status {
    pub const OFF: i32 = 0;
    pub const WAITING: i32 = 1;
    pub const CONNECTING: i32 = 2;
    pub const READY: i32 = 3;
    pub const RECONNECTING: i32 = 4;
    pub const ERROR: i32 = 5;
}

mod error {
    pub const NONE: i32 = 0;
    pub const TIMEOUT: i32 = 1;
    pub const NO_SSID: i32 = 2;
}

/// What modules asked for.
#[derive(Debug, Clone, PartialEq)]
struct Wanted {
    /// 0 off, 1 client, 2 access point, 3 both.
    mode: i32,
    /// `net` op 63 came: the Wi-Fi settings are complete.
    wifi_set: bool,
    ssid: String,
    password: String,
    dhcp: bool,
    ip: String,
    gateway: String,
    subnet: String,
    ap_ssid: String,
    ap_password: String,
    /// `net` op 64 came: the NTP settings are complete.
    ntp_set: bool,
    ntp_server: String,
    ntp_period_ms: u32,
}

impl Wanted {
    const fn new() -> Self {
        Self {
            mode: 0,
            wifi_set: false,
            ssid: String::new(),
            password: String::new(),
            dhcp: true,
            ip: String::new(),
            gateway: String::new(),
            subnet: String::new(),
            ap_ssid: String::new(),
            ap_password: String::new(),
            ntp_set: false,
            ntp_server: String::new(),
            ntp_period_ms: 0,
        }
    }
}

/// What the thread last saw.
#[derive(Debug, Clone, Copy, Default)]
struct Snapshot {
    client_ready: bool,
    ap_ready: bool,
    status: i32,
    error: i32,
    /// First octet in the lowest byte, as `text` op 11 prints it.
    ip: u32,
}

struct Net {
    wanted: Wanted,
    snap: Snapshot,
}

static NET: Mutex<Net> = Mutex::new(Net {
    wanted: Wanted::new(),
    snap: Snapshot {
        client_ready: false,
        ap_ready: false,
        status: 0,
        error: 0,
        ip: 0,
    },
});
static STARTED: AtomicBool = AtomicBool::new(false);
/// Board uptime (ms) of the last radio request.
static MODE_AT_MS: AtomicU32 = AtomicU32::new(0);
/// Board uptime in seconds of the last NTP synchronisation plus one; 0 while there was none.
static SYNCED_AT_S: AtomicU32 = AtomicU32::new(0);
/// Reason of the last disconnect of the client; 0 after a connect.
static LAST_REASON: AtomicU32 = AtomicU32::new(0);

/// Messages for the master journal: the master prints nothing on its own (every line on USB is an
/// answer to a command), so the network thread queues them and the runtime moves them to `log`.
static NOTES: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
/// Notes kept while nobody takes them.
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

/// Takes the queued network messages (for the journal).
pub fn take_notes() -> Vec<String> {
    let mut q = NOTES.lock().unwrap_or_else(|e| e.into_inner());
    q.drain(..).collect()
}

fn lock() -> std::sync::MutexGuard<'static, Net> {
    NET.lock().unwrap_or_else(|e| e.into_inner())
}

// ---------------------------------------------------------------------------------------------
// The import
// ---------------------------------------------------------------------------------------------

/// `net(op, a, b, c, d)` for the operations that concern Wi-Fi and NTP (0..=5).
pub fn call(op: i32, a: i32, b: i32, c: i32, mem: &mut Mem<'_>) -> i32 {
    match op {
        0 => set_number(a, b),
        1 => set_text(a, b, c, mem),
        2 => set_mode(a),
        3 => read_wifi(a),
        4 => read_ntp(a, b),
        5 => write_time_text(a, b, c, mem),
        _ => 0,
    }
}

fn set_number(code: i32, value: i32) -> i32 {
    let mut net = lock();
    let w = &mut net.wanted;
    match code {
        1 => w.dhcp = value != 0,
        2 => w.ntp_period_ms = value.max(0) as u32,
        63 => w.wifi_set = true,
        64 => w.ntp_set = true,
        _ => {} // 3: NTP over Ethernet, not on this board
    }
    0
}

fn set_text(code: i32, ptr: i32, len: i32, mem: &Mem<'_>) -> i32 {
    let Some(bytes) = mem.read(ptr, len, MAX_TEXT) else {
        return 0;
    };
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let mut net = lock();
    let w = &mut net.wanted;
    let field = match code {
        1 => &mut w.ssid,
        2 => &mut w.password,
        3 => &mut w.ip,
        4 => &mut w.gateway,
        5 => &mut w.subnet,
        6 => &mut w.ap_ssid,
        7 => &mut w.ap_password,
        8 => &mut w.ntp_server,
        _ => return 0,
    };
    if *field != text {
        *field = text;
    }
    1
}

fn set_mode(mode: i32) -> i32 {
    let mode = mode.clamp(0, 3);
    MODE_AT_MS.store(uptime_ms() as u32, Ordering::Release);
    {
        let mut net = lock();
        if net.wanted.mode != mode {
            net.wanted.mode = mode;
        }
    }
    if mode != 0 && !STARTED.swap(true, Ordering::AcqRel) {
        let spawned = std::thread::Builder::new()
            .name("crab-net".into())
            .stack_size(NET_STACK)
            .spawn(worker);
        if let Err(e) = spawned {
            note!("cannot start the network thread: {e}");
            STARTED.store(false, Ordering::Release);
        }
    }
    0
}

fn read_wifi(field: i32) -> i32 {
    let s = lock().snap;
    match field {
        0 => s.client_ready as i32,
        1 => s.ap_ready as i32,
        2 => s.status,
        3 => s.error,
        4 => s.ip as i32,
        _ => 0,
    }
}

/// Unix time in seconds, if the clock was set by SNTP.
fn unix_now() -> Option<i64> {
    if SYNCED_AT_S.load(Ordering::Acquire) == 0 {
        return None;
    }
    let t = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
    Some(t.as_secs() as i64)
}

fn read_ntp(field: i32, offset: i32) -> i32 {
    let synced = SYNCED_AT_S.load(Ordering::Acquire);
    match field {
        0 => i32::from(synced != 0),
        1 => {
            if synced == 0 {
                0
            } else {
                ((uptime_ms() / 1000) as u32).saturating_sub(synced - 1) as i32
            }
        }
        2..=8 => match unix_now() {
            Some(u) => {
                let (h, m, s, day, month, year, wd) = civil(u + i64::from(offset));
                [h, m, s, day, month, year, wd][field as usize - 2]
            }
            None => 0,
        },
        9 => unix_now().map_or(0, |u| u as i32),
        _ => 0,
    }
}

fn write_time_text(ptr: i32, cap: i32, offset: i32, mem: &mut Mem<'_>) -> i32 {
    let Some(unix) = unix_now() else {
        return 0;
    };
    let (h, m, s, ..) = civil(unix + i64::from(offset));
    let line = format!("{h:02}:{m:02}:{s:02}");
    let n = line.len().min(cap.max(0) as usize);
    if n == 0 || !mem.write(ptr, &line.as_bytes()[..n]) {
        return 0;
    }
    n as i32
}

/// Hour, minute, second, day, month, year and weekday (1 Monday .. 7 Sunday) of a Unix time.
/// The same algorithm as `civil` of `crab-sim`.
fn civil(t: i64) -> (i32, i32, i32, i32, i32, i32, i32) {
    let days = t.div_euclid(86400);
    let rest = t.rem_euclid(86400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let weekday = (days + 3).rem_euclid(7) + 1;
    (
        (rest / 3600) as i32,
        (rest % 3600 / 60) as i32,
        (rest % 60) as i32,
        day as i32,
        month as i32,
        year as i32,
        weekday as i32,
    )
}

// ---------------------------------------------------------------------------------------------
// The thread
// ---------------------------------------------------------------------------------------------

fn set_snapshot(f: impl FnOnce(&mut Snapshot)) {
    f(&mut lock().snap);
}

fn ip_to_u32(ip: Ipv4Addr) -> u32 {
    u32::from_le_bytes(ip.octets())
}

/// The part of the settings that needs the driver to be reconfigured.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Radio {
    /// Client network name; empty while the client is not wanted or has no name yet.
    ssid: String,
    password: String,
    /// The access point, if wanted.
    ap: Option<(String, String)>,
    /// Fixed address of the client; `None` for DHCP.
    fixed: Option<(Ipv4Addr, Ipv4Addr, u8)>,
}

fn parse_fixed(w: &Wanted) -> Option<(Ipv4Addr, Ipv4Addr, u8)> {
    if w.dhcp {
        return None;
    }
    let parse = |s: &str| Ipv4Addr::from_str(s.trim()).ok();
    let (Some(ip), Some(gw), Some(mask)) = (parse(&w.ip), parse(&w.gateway), parse(&w.subnet))
    else {
        note!("fixed address is incomplete or wrong, using DHCP");
        return None;
    };
    let bits = u32::from(mask);
    // A mask is ones followed by zeros.
    if bits.leading_ones() + bits.trailing_zeros() != 32 {
        note!("wrong subnet mask, using DHCP");
        return None;
    }
    Some((ip, gw, bits.leading_ones() as u8))
}

fn radio_of(w: &Wanted) -> Radio {
    let client = w.mode & 1 != 0;
    let ap = w.mode & 2 != 0;
    Radio {
        ssid: if client {
            w.ssid.clone()
        } else {
            String::new()
        },
        password: if client {
            w.password.clone()
        } else {
            String::new()
        },
        ap: ap.then(|| (w.ap_ssid.clone(), w.ap_password.clone())),
        fixed: if client { parse_fixed(w) } else { None },
    }
}

fn sta_netif(fixed: Option<(Ipv4Addr, Ipv4Addr, u8)>) -> Result<EspNetif, EspError> {
    let Some((ip, gateway, prefix)) = fixed else {
        return EspNetif::new(NetifStack::Sta);
    };
    let mut conf = NetifConfiguration::wifi_default_client();
    conf.ip_configuration = Some(ipv4::Configuration::Client(
        ipv4::ClientConfiguration::Fixed(ipv4::ClientSettings {
            ip,
            subnet: ipv4::Subnet {
                gateway,
                mask: ipv4::Mask(prefix),
            },
            // The router usually answers DNS too, which SNTP needs for the server name.
            dns: Some(gateway),
            secondary_dns: None,
        }),
    ));
    EspNetif::new_with_conf(&conf)
}

/// The longest prefix of `s` that fits `max` bytes.
fn cut(s: &str, max: usize) -> &str {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn configuration(r: &Radio) -> Configuration {
    let client = (!r.ssid.is_empty()).then(|| ClientConfiguration {
        ssid: cut(&r.ssid, 32).try_into().unwrap_or_default(),
        password: cut(&r.password, 64).try_into().unwrap_or_default(),
        auth_method: if r.password.is_empty() {
            AuthMethod::None
        } else {
            AuthMethod::WPA2Personal
        },
        ..Default::default()
    });
    let ap = r.ap.as_ref().map(|(ssid, password)| {
        // A WPA2 password has at least 8 characters; a shorter one means an open point.
        let secured = password.len() >= 8;
        AccessPointConfiguration {
            ssid: cut(if ssid.is_empty() { "CrabLogic" } else { ssid }, 32)
                .try_into()
                .unwrap_or_default(),
            password: cut(if secured { password } else { "" }, 64)
                .try_into()
                .unwrap_or_default(),
            auth_method: if secured {
                AuthMethod::WPA2Personal
            } else {
                AuthMethod::None
            },
            max_connections: 4,
            ..Default::default()
        }
    });
    match (client, ap) {
        (Some(c), Some(a)) => Configuration::Mixed(c, a),
        (Some(c), None) => Configuration::Client(c),
        (None, Some(a)) => Configuration::AccessPoint(a),
        (None, None) => Configuration::None,
    }
}

/// The client side of the link: attempts, timeouts and what to report.
struct Link {
    /// Start of the current attempt to get an address.
    attempt_since: Instant,
    next_connect: Instant,
    /// The client has an address now.
    was_ready: bool,
    /// The link was lost after it had been up and is not back yet (status 4 instead of 2).
    lost: bool,
}

impl Link {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            attempt_since: now,
            next_connect: now,
            was_ready: false,
            lost: false,
        }
    }
}

struct Ntp {
    // Held to keep the service running.
    _sntp: EspSntp<'static>,
    server: String,
    period_ms: u32,
}

struct Worker {
    wifi: EspWifi<'static>,
    /// What the driver is configured with; `None` while it is stopped.
    applied: Option<Radio>,
    /// Fixed address the client interface was made with.
    netif_fixed: Option<(Ipv4Addr, Ipv4Addr, u8)>,
    link: Link,
    ntp: Option<Ntp>,
}

fn log_status(last: &mut (i32, i32), status: i32, error: i32) {
    if *last != (status, error) {
        *last = (status, error);
        note!("wifi status {status} error {error}");
    }
}

impl Worker {
    fn stop(&mut self) {
        if self.applied.take().is_some() {
            let _ = self.wifi.disconnect();
            if let Err(e) = self.wifi.stop() {
                note!("wifi stop failed: {e}");
            }
            note!("wifi stopped");
        }
    }

    /// Reconfigures the driver for new settings (stopping it first).
    fn apply(&mut self, r: Radio) {
        let _ = self.wifi.disconnect();
        let _ = self.wifi.stop();
        self.applied = None;
        if r.ssid.is_empty() && r.ap.is_none() {
            return;
        }
        if r.fixed != self.netif_fixed {
            match sta_netif(r.fixed).and_then(|n| self.wifi.swap_netif_sta(n).map(drop)) {
                Ok(()) => self.netif_fixed = r.fixed,
                Err(e) => note!("cannot change the client interface: {e}"),
            }
        }
        let result = self
            .wifi
            .set_configuration(&configuration(&r))
            .and_then(|()| self.wifi.start());
        match result {
            Ok(()) => {
                note!(
                    "wifi started (client {:?}, access point {})",
                    r.ssid,
                    r.ap.is_some()
                );
                self.link = Link::new();
                self.applied = Some(r);
            }
            Err(e) => note!("wifi start failed: {e}"),
        }
    }

    /// Looks at the client link and fills the snapshot.
    fn watch(&mut self, last: &mut (i32, i32), client_wanted: bool, has_ssid: bool) {
        let ap_wanted = self.applied.as_ref().is_some_and(|r| r.ap.is_some());
        let started = self.applied.is_some() && self.wifi.is_started().unwrap_or(false);
        let ap_ip = self
            .wifi
            .ap_netif()
            .get_ip_info()
            .map(|i| i.ip)
            .unwrap_or(Ipv4Addr::UNSPECIFIED);
        let ap_ready = ap_wanted && started && !ap_ip.is_unspecified();

        let (mut client_ready, mut st, mut err, mut ip) =
            (false, status::OFF, error::NONE, Ipv4Addr::UNSPECIFIED);
        if client_wanted && !has_ssid {
            st = status::WAITING;
        } else if client_wanted && started {
            let connected = self.wifi.is_connected().unwrap_or(false);
            let info_ip = self
                .wifi
                .sta_netif()
                .get_ip_info()
                .map(|i| i.ip)
                .unwrap_or(Ipv4Addr::UNSPECIFIED);
            let up = self.wifi.sta_netif().is_up().unwrap_or(false);
            let now = Instant::now();
            if connected && up && !info_ip.is_unspecified() {
                client_ready = true;
                ip = info_ip;
                st = status::READY;
                if !self.link.was_ready {
                    note!("wifi ready, address {info_ip}");
                }
                self.link.was_ready = true;
                self.link.lost = false;
                self.link.attempt_since = now;
            } else {
                if self.link.was_ready {
                    note!("wifi link lost");
                    self.link.was_ready = false;
                    self.link.lost = true;
                    self.link.attempt_since = now;
                    self.link.next_connect = now;
                    LAST_REASON.store(0, Ordering::Release);
                }
                let reason = LAST_REASON.load(Ordering::Acquire);
                let failed = if !connected && reason == REASON_NO_AP_FOUND {
                    Some(error::NO_SSID)
                } else if now.duration_since(self.link.attempt_since) > CONNECT_TIMEOUT {
                    Some(if reason == REASON_NO_AP_FOUND {
                        error::NO_SSID
                    } else {
                        error::TIMEOUT
                    })
                } else {
                    None
                };
                match failed {
                    Some(e) => {
                        st = status::ERROR;
                        err = e;
                    }
                    None => {
                        st = if self.link.lost {
                            status::RECONNECTING
                        } else {
                            status::CONNECTING
                        };
                    }
                }
                if !connected && now >= self.link.next_connect {
                    self.link.next_connect = now + RETRY_PAUSE;
                    if let Err(e) = self.wifi.connect() {
                        note!("wifi connect: {e}");
                    }
                }
            }
        } else if client_wanted {
            st = status::CONNECTING;
        }

        if !client_wanted && ap_wanted {
            st = if ap_ready {
                status::READY
            } else {
                status::CONNECTING
            };
        }
        log_status(last, st, err);
        set_snapshot(|s| {
            s.client_ready = client_ready;
            s.ap_ready = ap_ready;
            s.status = st;
            s.error = err;
            s.ip = if client_ready {
                ip_to_u32(ip)
            } else if ap_ready {
                ip_to_u32(ap_ip)
            } else {
                0
            };
        });
    }

    /// Runs SNTP while the client has an address and NTP was set up.
    fn watch_ntp(&mut self, w: &Wanted, client_ready: bool) {
        if !w.ntp_set || !client_ready {
            if !w.ntp_set {
                self.ntp = None;
            }
            return;
        }
        let server = if w.ntp_server.trim().is_empty() {
            DEFAULT_NTP_SERVER.to_string()
        } else {
            w.ntp_server.trim().to_string()
        };
        let period_ms = match w.ntp_period_ms {
            0 => DEFAULT_NTP_PERIOD_MS,
            p => p.max(MIN_NTP_PERIOD_MS),
        };
        if self
            .ntp
            .as_ref()
            .is_some_and(|n| n.server == server && n.period_ms == period_ms)
        {
            return;
        }
        // Drop the old service first: only one may exist.
        self.ntp = None;
        // SAFETY: only stores the interval used by the SNTP service.
        unsafe { sys::sntp_set_sync_interval(period_ms) };
        let mut conf = SntpConf::default();
        conf.servers[0] = &server;
        let callback = |_: Duration| {
            // Runs on the tcpip thread: no printing here.
            SYNCED_AT_S.store((uptime_ms() / 1000) as u32 + 1, Ordering::Release);
        };
        match EspSntp::new_with_callback(&conf, callback) {
            Ok(sntp) => {
                note!("ntp started, server {server}, period {period_ms} ms");
                self.ntp = Some(Ntp {
                    _sntp: sntp,
                    server,
                    period_ms,
                });
            }
            Err(e) => note!("ntp start failed: {e}"),
        }
    }
}

fn init() -> Result<Worker, String> {
    let peripherals = Peripherals::take().map_err(|e| format!("peripherals: {e}"))?;
    // The driver keeps the modem for the life of the program.
    let modem: &'static mut Modem = Box::leak(Box::new(peripherals.modem));
    let sysloop = EspSystemEventLoop::take().map_err(|e| format!("event loop: {e}"))?;
    let nvs = EspDefaultNvsPartition::take().ok();
    let wifi =
        EspWifi::new(modem, sysloop.clone(), nvs).map_err(|e| format!("wifi driver: {e}"))?;
    let subscription = sysloop
        .subscribe::<WifiEvent, _>(|event| match event {
            WifiEvent::StaDisconnected(d) => {
                LAST_REASON.store(u32::from(d.reason()), Ordering::Release)
            }
            WifiEvent::StaConnected(_) => LAST_REASON.store(0, Ordering::Release),
            _ => {}
        })
        .map_err(|e| format!("wifi events: {e}"))?;
    // The subscription must live as long as the program.
    std::mem::forget(subscription);
    Ok(Worker {
        wifi,
        applied: None,
        netif_fixed: None,
        link: Link::new(),
        ntp: None,
    })
}

fn worker() {
    let mut w = match init() {
        Ok(w) => w,
        Err(e) => {
            note!("cannot start Wi-Fi: {e}");
            set_snapshot(|s| {
                s.status = status::ERROR;
                s.error = error::TIMEOUT;
            });
            // The peripherals are gone after a failed start: no second try before a reboot.
            return;
        }
    };
    note!("wifi driver ready");
    let mut last_status = (-1, -1);
    let mut last_sync = 0;
    loop {
        let wanted = lock().wanted.clone();
        let idle = (uptime_ms() as u32).wrapping_sub(MODE_AT_MS.load(Ordering::Acquire)) > LEASE_MS;
        if wanted.mode == 0 || !wanted.wifi_set || idle {
            w.stop();
            set_snapshot(|s| *s = Snapshot::default());
            log_status(&mut last_status, status::OFF, error::NONE);
        } else {
            let radio = radio_of(&wanted);
            if w.applied.as_ref() != Some(&radio) {
                w.apply(radio);
            }
            let has_ssid = w.applied.as_ref().is_some_and(|r| !r.ssid.is_empty());
            w.watch(&mut last_status, wanted.mode & 1 != 0, has_ssid);
        }
        let ready = lock().snap.client_ready;
        w.watch_ntp(&wanted, ready);
        let sync = SYNCED_AT_S.load(Ordering::Acquire);
        if sync != last_sync {
            last_sync = sync;
            note!("time synchronised");
        }
        std::thread::sleep(TICK);
    }
}
