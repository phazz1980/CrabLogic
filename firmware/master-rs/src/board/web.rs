//! The web page of the device (`net` ops 17–24, `crab_master_rs::web` tested on the PC): an
//! ESP-IDF HTTP server on port 80 serves the page of the editor and its `/ui/...` calls; the layout
//! and the saved network form live in NVS (namespace `crab_web`), as in the C++ master.

use crab_kernel::host::Mem;
use crab_master_rs::web::{NetSaved, Request, SystemInfo, WebState, NET_FIELDS};
use esp_idf_svc::http::server::{Configuration, EspHttpServer};
use esp_idf_svc::http::Method;
use esp_idf_svc::io::Write;
use esp_idf_svc::sys;
use std::ffi::CString;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const PAGE: &str = include_str!(concat!(env!("OUT_DIR"), "/device_page.html"));
const TICK: Duration = Duration::from_millis(100);
/// The server is started again this often while it cannot be (no network yet).
const RETRY: Duration = Duration::from_secs(2);
/// Body of a request, at most.
const MAX_BODY: usize = 2048;

struct State {
    web: WebState,
    /// The server runs until reset once started (it is not `Send`, so it is not kept here).
    server: bool,
    last_try: Option<Instant>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static STARTED: Mutex<Option<Instant>> = Mutex::new(None);

fn lock() -> std::sync::MutexGuard<'static, Option<State>> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

fn now_ms() -> u32 {
    let mut started = STARTED.lock().unwrap_or_else(|e| e.into_inner());
    started
        .get_or_insert_with(Instant::now)
        .elapsed()
        .as_millis() as u32
}

/// `net` ops 17–24 of modules.
pub fn call(op: i32, a: i32, b: i32, c: i32, d: i32, mem: &mut Mem<'_>) -> i32 {
    let mut guard = lock();
    if guard.is_none() {
        let mut web = WebState::new(PAGE);
        if let Some((bytes, hash)) = load_layout() {
            web.restore_layout(&bytes, hash);
        }
        if let Some(saved) = load_net() {
            web.restore_net(saved);
        }
        *guard = Some(State {
            web,
            server: false,
            last_try: None,
        });
        drop(guard);
        start_task();
        guard = lock();
    }
    let state = guard.as_mut().expect("set above");
    state.web.net_op(op, a, b, c, d, mem, now_ms())
}

fn start_task() {
    let spawned = std::thread::Builder::new()
        .name("web".into())
        .stack_size(6144)
        .spawn(|| loop {
            tick();
            std::thread::sleep(TICK);
        });
    if spawned.is_err() {
        super::notes::push("web", "задача страницы не запущена".into());
    }
}

fn tick() {
    let mut guard = lock();
    let Some(s) = guard.as_mut() else { return };
    s.web.tick(now_ms());
    if let Some((bytes, hash)) = s.web.layout_for_save() {
        if load_layout().map(|(_, h)| h) != Some(hash) {
            save_layout(bytes, hash);
        }
    }
    if let Some(saved) = s.web.take_net_save() {
        save_net(&saved);
    }
    // Wi-Fi scan is not in the Rust master yet: the page shows an empty list.
    let _ = s.web.scan_wanted();
    if !s.server
        && s.web.complete()
        && super::net::link_ready()
        && s.last_try.is_none_or(|t| t.elapsed() >= RETRY)
    {
        s.last_try = Some(Instant::now());
        if let Some(server) = start_server() {
            core::mem::forget(server);
            s.server = true;
            super::notes::push("web", "страница устройства на порту 80".into());
        }
    }
}

fn start_server() -> Option<EspHttpServer<'static>> {
    let config = Configuration {
        uri_match_wildcard: true,
        stack_size: 8192,
        ..Default::default()
    };
    let mut server = match EspHttpServer::new(&config) {
        Ok(s) => s,
        Err(e) => {
            super::notes::push("web", format!("сервер не запущен: {e}"));
            return None;
        }
    };
    for method in [Method::Get, Method::Post] {
        let registered = server.fn_handler("/*", method, move |mut req| {
            let uri = req.uri().to_owned();
            let mut body = Vec::new();
            let mut chunk = [0u8; 256];
            while body.len() < MAX_BODY {
                match req.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => body.extend_from_slice(&chunk[..n]),
                }
            }
            let name = if method == Method::Post {
                "POST"
            } else {
                "GET"
            };
            let request = Request::from_uri(name, &uri, body);
            let info = system_info();
            let response = {
                let mut guard = lock();
                match guard.as_mut() {
                    Some(s) => s.web.handle(&request, &info),
                    None => return Ok::<(), esp_idf_svc::io::EspIOError>(()),
                }
            };
            let headers = [
                ("Content-Type", response.content_type),
                ("Cache-Control", "no-store"),
            ];
            let mut out = req.into_response(response.status, None, &headers)?;
            out.write_all(&response.body)?;
            Ok(())
        });
        if let Err(e) = registered {
            super::notes::push("web", format!("обработчик не зарегистрирован: {e}"));
            return None;
        }
    }
    Some(server)
}

fn system_info() -> SystemInfo {
    let ip = super::net::wifi_ip().to_le_bytes();
    let mut mac = [0u8; 6];
    let mut ap: sys::wifi_ap_record_t = Default::default();
    // SAFETY: plain out-parameters; the Wi-Fi call fails harmlessly while not connected.
    let (rssi, heap_free, heap_total, flash_size, uptime_us) = unsafe {
        sys::esp_read_mac(mac.as_mut_ptr(), sys::esp_mac_type_t_ESP_MAC_WIFI_STA);
        let rssi = if sys::esp_wifi_sta_get_ap_info(&mut ap) == 0 {
            i32::from(ap.rssi)
        } else {
            0
        };
        let mut flash = 0u32;
        sys::esp_flash_get_size(core::ptr::null_mut(), &mut flash);
        (
            rssi,
            sys::esp_get_free_heap_size(),
            sys::heap_caps_get_total_size(sys::MALLOC_CAP_DEFAULT) as u32,
            flash,
            sys::esp_timer_get_time(),
        )
    };
    // SAFETY: the running partition is static data of ESP-IDF.
    let app_size = unsafe {
        let p = sys::esp_ota_get_running_partition();
        if p.is_null() {
            0
        } else {
            (*p).size
        }
    };
    SystemInfo {
        ip: format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]),
        mac: mac
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(":"),
        rssi,
        uptime_s: (uptime_us / 1_000_000) as u32,
        chip: if cfg!(target_arch = "riscv32") {
            "ESP32-C3".into()
        } else {
            "ESP32".into()
        },
        cpu_mhz: if cfg!(target_arch = "riscv32") {
            160
        } else {
            240
        },
        heap_free,
        heap_total,
        flash_size,
        app_size,
        app_free: 0,
        build: format!("crab-master-rs {}", env!("CARGO_PKG_VERSION")),
        scan: String::new(),
    }
}

// ---- NVS: namespace crab_web ----

fn nvs() -> Option<sys::nvs_handle_t> {
    static HANDLE: Mutex<Option<sys::nvs_handle_t>> = Mutex::new(None);
    let mut h = HANDLE.lock().unwrap_or_else(|e| e.into_inner());
    if h.is_none() {
        let mut handle: sys::nvs_handle_t = 0;
        // SAFETY: NVS is initialised by the master at start (`retain`/`net`); a fixed namespace.
        let ok = unsafe {
            sys::nvs_flash_init();
            sys::nvs_open(
                c"crab_web".as_ptr(),
                sys::nvs_open_mode_t_NVS_READWRITE,
                &mut handle,
            ) == 0
        };
        if ok {
            *h = Some(handle);
        }
    }
    *h
}

fn get_blob(key: &str) -> Option<Vec<u8>> {
    let h = nvs()?;
    let key = CString::new(key).ok()?;
    let mut len = 0usize;
    // SAFETY: size query, then a read into a buffer of that size.
    unsafe {
        if sys::nvs_get_blob(h, key.as_ptr(), core::ptr::null_mut(), &mut len) != 0 {
            return None;
        }
        let mut buf = vec![0u8; len];
        if sys::nvs_get_blob(h, key.as_ptr(), buf.as_mut_ptr().cast(), &mut len) != 0 {
            return None;
        }
        buf.truncate(len);
        Some(buf)
    }
}

fn set_blob(key: &str, bytes: &[u8]) {
    let (Some(h), Ok(key)) = (nvs(), CString::new(key)) else {
        return;
    };
    // SAFETY: an open handle; the bytes are copied by NVS.
    unsafe {
        sys::nvs_set_blob(h, key.as_ptr(), bytes.as_ptr().cast(), bytes.len());
        sys::nvs_commit(h);
    }
}

fn load_layout() -> Option<(Vec<u8>, u32)> {
    let hash = get_blob("lhash")?;
    let hash = u32::from_le_bytes(hash.get(..4)?.try_into().ok()?);
    Some((get_blob("layout")?, hash))
}

fn save_layout(bytes: &[u8], hash: u32) {
    set_blob("layout", bytes);
    set_blob("lhash", &hash.to_le_bytes());
}

/// The form as `sig` (4 bytes) then the fields, each a length byte and its bytes.
fn load_net() -> Option<NetSaved> {
    let bytes = get_blob("net")?;
    let sig = u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?);
    let mut fields: [String; NET_FIELDS] = Default::default();
    let mut at = 4;
    for field in &mut fields {
        let len = usize::from(*bytes.get(at)?);
        *field = String::from_utf8_lossy(bytes.get(at + 1..at + 1 + len)?).into_owned();
        at += 1 + len;
    }
    Some(NetSaved { sig, fields })
}

fn save_net(saved: &NetSaved) {
    let mut bytes = saved.sig.to_le_bytes().to_vec();
    for field in &saved.fields {
        let f = &field.as_bytes()[..field.len().min(255)];
        bytes.push(f.len() as u8);
        bytes.extend_from_slice(f);
    }
    set_blob("net", &bytes);
}
