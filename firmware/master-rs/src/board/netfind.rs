//! Answers the LAN board search (`specs/net-find`): one UDP datagram on `crab_netfind::PORT` gets
//! one reply to its sender. The thread listens whenever Wi-Fi or Ethernet has an address, with or
//! without a network key (a board without a key is listed and marked). The board never sends by
//! itself; replies are rate limited and carry no secrets (`crab-netfind` builds them).

use std::net::UdpSocket;
use std::sync::atomic::Ordering;
use std::time::Duration;

use crab_netfind::{encode_reply, is_request, Board, ReplyLimiter, MIN_INTERVAL_MS, PORT};
use crab_master_rs::netauth;
use esp_idf_svc::sys;

use super::spin::Spin;

const STACK: usize = 6 * 1024;
/// A request is 9 bytes; anything longer is not ours and is dropped after the read.
const RX: usize = 16;
/// How long a receive waits before the thread looks at the network state again.
const POLL: Duration = Duration::from_millis(500);

#[cfg(target_arch = "riscv32")]
const CHIP: &str = "esp32c3";
#[cfg(not(target_arch = "riscv32"))]
const CHIP: &str = "esp32";

static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// The project name the board shows in the search (`name` command, kept in NVS).
static NAME: Spin<String> = Spin::new(String::new());

/// Loads the name from NVS (key `name` of namespace `crab_net`) at start.
fn load_name() {
    let Some(h) = super::netcmd::nvs() else { return };
    let mut buf = [0u8; crab_master_rs::protocol::BOARD_NAME_MAX + 1];
    let mut len = buf.len();
    // SAFETY: the buffer and its length live through the call; the handle is closed after.
    let found = unsafe {
        let r = sys::nvs_get_str(h, c"name".as_ptr(), buf.as_mut_ptr().cast(), &mut len);
        sys::nvs_close(h);
        r == 0
    };
    if found && len > 0 {
        if let Ok(text) = core::str::from_utf8(&buf[..len - 1]) {
            *NAME.lock_wait() = text.to_owned();
        }
    }
}

/// `name` command: stores the name (or removes it with `None`); false if NVS refused.
pub fn set_name(name: Option<&str>) -> bool {
    let text = name.unwrap_or("");
    if text.len() > crab_master_rs::protocol::BOARD_NAME_MAX || text.contains('\0') {
        return false;
    }
    let Some(h) = super::netcmd::nvs() else {
        return false;
    };
    let Ok(c_text) = std::ffi::CString::new(text) else {
        return false;
    };
    // SAFETY: a valid handle, key and NUL-terminated text living through the call; closed after.
    let ok = unsafe {
        let r = if name.is_some() {
            sys::nvs_set_str(h, c"name".as_ptr(), c_text.as_ptr())
        } else {
            let r = sys::nvs_erase_key(h, c"name".as_ptr());
            if r == sys::ESP_ERR_NVS_NOT_FOUND {
                0
            } else {
                r
            }
        };
        let ok = r == 0 && sys::nvs_commit(h) == 0;
        sys::nvs_close(h);
        ok
    };
    if ok {
        *NAME.lock_wait() = text.to_owned();
    }
    ok
}

/// Starts the thread once; true if it runs.
pub fn start() -> bool {
    if STARTED.swap(true, Ordering::AcqRel) {
        return true;
    }
    load_name();
    let ok = super::spawn_background(b"crab-netfind\0", STACK, run);
    if !ok {
        STARTED.store(false, Ordering::Release);
    }
    ok
}

fn network_up() -> bool {
    super::net::link_ready() || super::eth::ready()
}

/// The station MAC as 12 lowercase hex digits (the id of the board).
fn mac_id() -> Option<String> {
    let mut mac = [0u8; 6];
    // SAFETY: the buffer has the 6 bytes the call writes.
    let err = unsafe { sys::esp_read_mac(mac.as_mut_ptr(), sys::esp_mac_type_t_ESP_MAC_WIFI_STA) };
    (err == 0).then(|| {
        let mut out = String::with_capacity(12);
        for b in mac {
            out.push(char::from(b"0123456789abcdef"[usize::from(b >> 4)]));
            out.push(char::from(b"0123456789abcdef"[usize::from(b & 15)]));
        }
        out
    })
}

fn board(id: &str) -> Board {
    Board {
        id: id.to_owned(),
        chip: CHIP.to_owned(),
        master: env!("CARGO_PKG_VERSION").to_owned(),
        hash: super::snapshot::program_crc().map(|c| format!("{c:08x}")),
        has_key: super::netcmd::has_key(),
        port: netauth::PORT,
        name: NAME.lock_wait().clone(),
    }
}

fn run() {
    let Some(id) = mac_id() else { return };
    let mut limiter = ReplyLimiter::new(MIN_INTERVAL_MS);
    let mut reply = Vec::with_capacity(crab_netfind::MAX_REPLY);
    loop {
        if !network_up() {
            std::thread::sleep(POLL);
            continue;
        }
        let Ok(socket) = UdpSocket::bind(("0.0.0.0", PORT)) else {
            std::thread::sleep(Duration::from_secs(1));
            continue;
        };
        if socket.set_read_timeout(Some(POLL)).is_err() {
            continue;
        }
        let mut buf = [0u8; RX];
        while network_up() {
            let Ok((n, from)) = socket.recv_from(&mut buf) else {
                continue;
            };
            if !is_request(&buf[..n]) || !limiter.allow(super::devices::uptime_ms() as u32) {
                continue;
            }
            reply.clear();
            if encode_reply(&board(&id), &mut reply) {
                let _ = socket.send_to(&reply, from);
            }
        }
    }
}
