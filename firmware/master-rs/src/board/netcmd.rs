//! The command server over the network (`specs/ota`, phase 1): the same commands as over USB
//! (`info`, `list`, module upload, `del`, `log`) from one client on TCP port 4040, after a login.
//!
//! - The thread `crab-netcmd` listens only while a key is stored (`netkey` over USB) and Wi-Fi or
//!   Ethernet has an address; it never touches the scan: the bytes go into the parser of `crab-io`
//!   and the answers come back as a copy of the answer queue.
//! - Login: a fresh nonce, HMAC-SHA256 with the key (mbedtls of ESP-IDF); wrong answers are
//!   slowed down and locked out (`crab_master_rs::netauth`).
//! - While a client is logged in, `crab-io` takes no bytes from USB (two command streams would
//!   mix in the middle of an upload); the answers go to both.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crab_master_rs::netauth::{self, Limiter, FAIL_PAUSE_MS, PORT};
use esp_idf_svc::sys;

use super::spin::Spin;

const STACK: usize = 6 * 1024;
/// Answers waiting for the client, bytes; a client that does not read is dropped.
const OUT_MAX: usize = 16 * 1024;
/// How long a client has to answer the login, and how long a write to it may take.
const LOGIN_WAIT: Duration = Duration::from_secs(5);
const WRITE_WAIT: Duration = Duration::from_secs(2);
/// A client silent this long is dropped: a Wi-Fi link that went away sends no FIN, and the
/// server would stay busy (USB input stays off while a client is logged in).
const IDLE: Duration = Duration::from_secs(120);

/// The key (from NVS at start, `netkey` over USB). `crab-io` and `crab-netcmd` only.
static KEY: Spin<Option<[u8; 32]>> = Spin::new(None);
static HAS_KEY: AtomicBool = AtomicBool::new(false);
/// A client is logged in.
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// Copy of the answers for the client (`crab-io` puts, `crab-netcmd` takes).
static OUT: Spin<Vec<u8>> = Spin::new(Vec::new());
/// The copy did not fit: the client is dropped.
static OVERFLOW: AtomicBool = AtomicBool::new(false);
/// The server thread runs (started only once a key exists, to save its stack otherwise).
static STARTED: AtomicBool = AtomicBool::new(false);

/// Whether a network client is logged in (then `crab-io` ignores USB input).
pub fn active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}

/// For `info`.
pub fn status_line() -> String {
    format!(
        "netcmd port={PORT} key={} thread={} client={}",
        if HAS_KEY.load(Ordering::Acquire) {
            "yes"
        } else {
            "no"
        },
        if STARTED.load(Ordering::Acquire) {
            "yes"
        } else {
            "no"
        },
        u8::from(active())
    )
}

/// A copy of the answers that went to USB, for the client (called by `crab-io`).
pub fn mirror(bytes: &[u8]) {
    if !active() || bytes.is_empty() {
        return;
    }
    let mut out = OUT.lock_wait();
    if out.len() + bytes.len() > OUT_MAX {
        OVERFLOW.store(true, Ordering::Release);
        return;
    }
    out.extend_from_slice(bytes);
}

// ---- the key in NVS (namespace `crab_net`) ----

fn nvs() -> Option<sys::nvs_handle_t> {
    let mut h: sys::nvs_handle_t = 0;
    // SAFETY: NVS is set up at start (a second init is harmless); a fixed namespace.
    let ok = unsafe {
        sys::nvs_flash_init();
        sys::nvs_open(
            c"crab_net".as_ptr(),
            sys::nvs_open_mode_t_NVS_READWRITE,
            &mut h,
        ) == 0
    };
    ok.then_some(h)
}

fn load_key() {
    let Some(h) = nvs() else { return };
    let mut key = [0u8; 32];
    let mut len = key.len();
    // SAFETY: the buffer and its length live through the call; the handle is closed after.
    let found = unsafe {
        let found = sys::nvs_get_blob(h, c"key".as_ptr(), key.as_mut_ptr().cast(), &mut len) == 0;
        sys::nvs_close(h);
        found
    } && len == key.len();
    if found {
        *KEY.lock_wait() = Some(key);
        HAS_KEY.store(true, Ordering::Release);
    }
}

/// `netkey` over USB: stores or removes the key; false if NVS refused.
pub fn set_key(key: Option<[u8; 32]>) -> bool {
    let Some(h) = nvs() else { return false };
    // SAFETY: a valid handle and key name; the data lives through the call; closed after.
    let ok = unsafe {
        let r = match &key {
            Some(k) => sys::nvs_set_blob(h, c"key".as_ptr(), k.as_ptr().cast(), k.len()),
            None => {
                let r = sys::nvs_erase_key(h, c"key".as_ptr());
                if r == sys::ESP_ERR_NVS_NOT_FOUND {
                    0
                } else {
                    r
                }
            }
        };
        let ok = r == 0 && sys::nvs_commit(h) == 0;
        sys::nvs_close(h);
        ok
    };
    if ok {
        *KEY.lock_wait() = key;
        HAS_KEY.store(key.is_some(), Ordering::Release);
        if key.is_some() && !spawn() {
            println!("err netcmd thread");
        }
    }
    ok
}

// ---- the server ----

/// Loads the key and starts the thread if there is one (after `crab-io`); false if it could not.
pub fn start() -> bool {
    load_key();
    !HAS_KEY.load(Ordering::Acquire) || spawn()
}

/// Starts the server thread once; true if it runs.
fn spawn() -> bool {
    if STARTED.swap(true, Ordering::AcqRel) {
        return true;
    }
    let ok = super::spawn_background(b"crab-netcmd\0", STACK, run);
    if !ok {
        STARTED.store(false, Ordering::Release);
    }
    ok
}

fn uptime_ms() -> u64 {
    super::devices::uptime_ms()
}

fn network_up() -> bool {
    super::net::link_ready() || super::eth::ready()
}

fn run() {
    let mut limiter = Limiter::default();
    loop {
        if !(HAS_KEY.load(Ordering::Acquire) && network_up()) {
            std::thread::sleep(Duration::from_millis(500));
            continue;
        }
        let listener = match TcpListener::bind(("0.0.0.0", PORT)) {
            Ok(l) => l,
            Err(_) => {
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        if listener.set_nonblocking(true).is_err() {
            continue;
        }
        while HAS_KEY.load(Ordering::Acquire) && network_up() {
            match listener.accept() {
                Ok((stream, _)) => {
                    if limiter.allows(uptime_ms()) {
                        serve(stream, &listener, &mut limiter);
                    }
                    // Locked: the connection just closes.
                }
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    }
}

/// A second client while one is served: told and closed.
fn refuse_others(listener: &TcpListener) {
    while let Ok((mut other, _)) = listener.accept() {
        let _ = other.set_write_timeout(Some(Duration::from_millis(200)));
        let _ = other.write_all(b"err busy\n");
    }
}

fn hmac(key: &[u8; 32], data: &[u8]) -> Option<[u8; 32]> {
    let mut out = [0u8; 32];
    // SAFETY: the digest table is static; the buffers live through the call.
    let ok = unsafe {
        let md = sys::mbedtls_md_info_from_type(sys::mbedtls_md_type_t_MBEDTLS_MD_SHA256);
        !md.is_null()
            && sys::mbedtls_md_hmac(
                md,
                key.as_ptr(),
                key.len(),
                data.as_ptr(),
                data.len(),
                out.as_mut_ptr(),
            ) == 0
    };
    ok.then_some(out)
}

/// One line from the client within `wait`; `None` on a timeout or a closed connection.
fn read_line(stream: &mut TcpStream, wait: Duration) -> Option<String> {
    let started = Instant::now();
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    while started.elapsed() < wait && line.len() < 160 {
        match stream.read(&mut byte) {
            Ok(0) => return None,
            Ok(_) if byte[0] == b'\n' => return String::from_utf8(line).ok(),
            Ok(_) => line.push(byte[0]),
            Err(_) => {}
        }
    }
    None
}

fn login(stream: &mut TcpStream) -> bool {
    let Some(key) = *KEY.lock_wait() else {
        return false;
    };
    let mut nonce = [0u8; 16];
    // SAFETY: the buffer is writable for its length.
    unsafe { sys::esp_fill_random(nonce.as_mut_ptr().cast(), nonce.len()) };
    let mut hex = [0u8; 32];
    let line = format!("auth {}\n", netauth::to_hex(&nonce, &mut hex));
    if stream.write_all(line.as_bytes()).is_err() {
        return false;
    }
    let answer = read_line(stream, LOGIN_WAIT).and_then(|l| netauth::parse_answer(&l));
    match (answer, hmac(&key, &nonce)) {
        (Some(got), Some(want)) => netauth::same(&got, &want),
        _ => false,
    }
}

fn serve(mut stream: TcpStream, listener: &TcpListener, limiter: &mut Limiter) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(20)));
    let _ = stream.set_write_timeout(Some(WRITE_WAIT));
    if !login(&mut stream) {
        limiter.failed(uptime_ms());
        let _ = stream.write_all(b"err auth\n");
        std::thread::sleep(Duration::from_millis(FAIL_PAUSE_MS));
        return;
    }
    limiter.succeeded();
    OUT.lock_wait().clear();
    OVERFLOW.store(false, Ordering::Release);
    if stream.write_all(b"ok\n").is_err() {
        return;
    }
    super::io::with_input(|i| i.clear());
    ACTIVE.store(true, Ordering::Release);
    super::notes::push("netcmd", "клиент по сети подключён".to_owned());
    let mut buf = [0u8; 512];
    let mut out = Vec::new();
    let mut heard = Instant::now();
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                heard = Instant::now();
                super::io::feed(&buf[..n], true);
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
        }
        core::mem::swap(&mut *OUT.lock_wait(), &mut out);
        if !out.is_empty() {
            if stream.write_all(&out).is_err() {
                break;
            }
            out.clear();
        }
        if OVERFLOW.load(Ordering::Acquire)
            || !network_up()
            || !HAS_KEY.load(Ordering::Acquire)
            || heard.elapsed() > IDLE
        {
            break;
        }
        refuse_others(listener);
    }
    super::io::with_input(|i| i.clear());
    ACTIVE.store(false, Ordering::Release);
    OUT.lock_wait().clear();
    super::notes::push("netcmd", "клиент по сети отключён".to_owned());
}
