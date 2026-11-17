//! WASM entry points of the `lora_e220` block: `init`, `step` and the imports of the master.
//! Parameter and port numbers follow `library/lora_e220/block.toml`; the extra input and output
//! cells (the values of the partner blocks) follow the order of `crate::config`.

use crate::config::{decode, Config};
use crate::e220::{key_tag, key_word, Air, Hal, Link, Power, Settings, SubPacket, CENTRE_NODE};
use crate::frame::{Ty, MAX_VALUES};
use crate::master::{Config as MasterConfig, Master, Peer as MasterPeer};
use crate::safe::Safe;
use crate::slave::Slave;

#[link(wasm_import_module = "crab")]
extern "C" {
    fn input_i(index: i32) -> i32;
    fn input_f(index: i32) -> f32;
    fn param_i(index: i32) -> i32;
    fn param_text(index: i32, ptr: *mut u8, cap: i32) -> i32;
    fn output_i(index: i32, value: i32);
    fn output_f(index: i32, value: f32);
    fn config(ptr: *mut u8, cap: i32) -> i32;
}

#[link(wasm_import_module = "bus")]
extern "C" {
    fn gpio_mode(pin: i32, mode: i32) -> i32;
    fn gpio_read(pin: i32) -> i32;
    fn gpio_write(pin: i32, level: i32) -> i32;
    fn uart_open(port: i32, tx: i32, rx: i32, baud: i32) -> i32;
    fn uart_write(port: i32, ptr: *const u8, len: i32) -> i32;
    fn uart_read(port: i32, ptr: *mut u8, cap: i32) -> i32;
}

// Parameters (order of block.toml).
const P_ROLE: i32 = 0;
const P_NODE: i32 = 1;
const P_CHANNEL: i32 = 2;
const P_AIR: i32 = 3;
const P_POWER: i32 = 4;
const P_KEY: i32 = 6;
const P_UART: i32 = 7;
const P_TX: i32 = 8;
const P_RX: i32 = 9;
const P_M0: i32 = 10;
const P_M1: i32 = 11;
const P_AUX: i32 = 12;
const P_REPLY: i32 = 13;
const P_RETRIES: i32 = 14;
const P_OFFLINE: i32 = 15;

// Own outputs: ready, rssi, bad; the extra output cells come after them. No own inputs.
const OUT_READY: i32 = 0;
const OUT_RSSI: i32 = 1;
const OUT_BAD: i32 = 2;
const OWN_OUT: i32 = 3;

/// The module's UART speed in normal and configuration mode.
const BAUD: i32 = 9600;
const CFG_CAP: usize = 192;
const KEY_CAP: usize = 32;
const RX_CAP: usize = 64;

struct BusHal {
    port: i32,
    m0: i32,
    m1: i32,
    aux: i32,
    rx: [u8; RX_CAP],
    at: usize,
    len: usize,
}

impl Hal for BusHal {
    fn set_config_mode(&mut self, on: bool) {
        unsafe {
            gpio_write(self.m0, i32::from(on));
            // M1 tied to M0 (`m1 = -1`): one pin drives both.
            if self.m1 >= 0 {
                gpio_write(self.m1, i32::from(on));
            }
        }
    }

    fn aux(&mut self) -> Option<bool> {
        if self.aux < 0 {
            return None;
        }
        Some(unsafe { gpio_read(self.aux) } > 0)
    }

    fn write(&mut self, data: &[u8]) -> bool {
        unsafe { uart_write(self.port, data.as_ptr(), data.len() as i32) == data.len() as i32 }
    }

    fn read(&mut self) -> Option<u8> {
        if self.at >= self.len {
            let n = unsafe { uart_read(self.port, self.rx.as_mut_ptr(), RX_CAP as i32) };
            self.at = 0;
            self.len = n.max(0) as usize;
        }
        let b = *self.rx.get(self.at).filter(|_| self.at < self.len)?;
        self.at += 1;
        Some(b)
    }

    fn flush_rx(&mut self) {
        self.at = 0;
        self.len = 0;
        // Whatever the UART buffer holds is read and thrown away.
        while unsafe { uart_read(self.port, self.rx.as_mut_ptr(), RX_CAP as i32) } > 0 {}
    }
}

#[allow(clippy::large_enum_variant)] // one static instance, nothing is moved around
enum Role {
    Centre(Master),
    Slave(Slave),
}

struct Core {
    cfg: Config,
    link: Link<BusHal>,
    role: Role,
    now: u32,
}

static mut CFG: [u8; CFG_CAP] = [0; CFG_CAP];
static mut CORE: Option<Core> = None;

fn air_of(v: i32) -> Air {
    match v {
        3 => Air::R4800,
        4 => Air::R9600,
        5 => Air::R19200,
        6 => Air::R38400,
        7 => Air::R62500,
        _ => Air::R2400,
    }
}

fn power_of(v: i32) -> Power {
    match v {
        0 => Power::Dbm22,
        1 => Power::Dbm17,
        2 => Power::Dbm13,
        _ => Power::Dbm10,
    }
}

#[no_mangle]
pub extern "C" fn init() {
    unsafe {
        let ptr = core::ptr::addr_of_mut!(CFG).cast::<u8>();
        let n = config(ptr, CFG_CAP as i32);
        if n <= 0 || n as usize > CFG_CAP {
            return;
        }
        let Some(cfg) = decode(core::slice::from_raw_parts(ptr, n as usize)) else {
            return;
        };
        let centre = param_i(P_ROLE) == 0;
        if centre != cfg.centre {
            return;
        }
        let mut key = [0u8; KEY_CAP];
        let kn = param_text(P_KEY, key.as_mut_ptr(), KEY_CAP as i32).clamp(0, KEY_CAP as i32);
        let key = key.part(kn as usize);
        let node = if centre {
            CENTRE_NODE
        } else {
            param_i(P_NODE).clamp(0, 14) as u8
        };
        let settings = Settings {
            addr: u16::from_be_bytes([key_tag(key), node]),
            channel: param_i(P_CHANNEL).clamp(0, 83) as u8,
            air: air_of(param_i(P_AIR)),
            sub: SubPacket::B200,
            power: power_of(param_i(P_POWER)),
            key: key_word(key),
        };
        let hal = BusHal {
            port: param_i(P_UART),
            m0: param_i(P_M0),
            m1: param_i(P_M1),
            aux: param_i(P_AUX),
            rx: [0; RX_CAP],
            at: 0,
            len: 0,
        };
        uart_open(hal.port, param_i(P_TX), param_i(P_RX), BAUD);
        gpio_mode(hal.m0, 1);
        if hal.m1 >= 0 {
            gpio_mode(hal.m1, 1);
        }
        if hal.aux >= 0 {
            gpio_mode(hal.aux, 2);
        }
        let offline = param_i(P_OFFLINE).max(1000) as u32;
        let role = if centre {
            let mut m = Master::new(MasterConfig {
                reply_timeout_ms: param_i(P_REPLY).max(100) as u32,
                retries: param_i(P_RETRIES).clamp(0, 5) as u8,
                offline_ms: offline,
                ..MasterConfig::default()
            });
            for p in cfg.peers() {
                m.add(MasterPeer::new(p.node, p.tx, p.rx));
            }
            Role::Centre(m)
        } else {
            let Some(p) = cfg.peers().next() else { return };
            // The slave's `tx` are its answers, `rx` the commands of the centre.
            Role::Slave(Slave::new(node, p.rx, p.tx, offline))
        };
        CORE = Some(Core {
            link: Link::new(hal, &settings, false, centre),
            cfg,
            role,
            now: 0,
        });
    }
}

/// Raw 32-bit cells of the values of one partner: `bool`/`int` as `i32`, `float` as `f32` bits.
fn read_cells(first: i32, types: &[Ty], out: &mut [u32; MAX_VALUES]) {
    for (k, t) in types.iter().enumerate() {
        let at = first + k as i32;
        let v = unsafe {
            match t {
                Ty::Float => input_f(at).to_bits(),
                _ => input_i(at) as u32,
            }
        };
        out.wr(k, v);
    }
}

fn write_cells(first: i32, types: &[Ty], cells: &[u32; MAX_VALUES]) {
    for (k, t) in types.iter().enumerate() {
        let at = first + k as i32;
        let v = cells.rd(k);
        unsafe {
            match t {
                Ty::Float => output_f(at, f32::from_bits(v)),
                _ => output_i(at, v as i32),
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn step(dt_ms: i32) {
    let Some(core) = (unsafe { &mut *core::ptr::addr_of_mut!(CORE) }) else {
        return;
    };
    core.now = core.now.wrapping_add(dt_ms.max(0) as u32);
    let now = core.now;
    core.link.step(now);
    let ready = core.link.ready();
    unsafe {
        output_i(OUT_READY, i32::from(ready));
        output_i(OUT_RSSI, i32::from(core.link.rssi()));
    }
    let (mut at_in, mut at_out) = (0i32, OWN_OUT);
    let mut bad = core.link.dropped();
    match &mut core.role {
        Role::Centre(master) => {
            for (i, p) in core.cfg.peers().enumerate() {
                if let Some(peer) = master.peer_mut(i) {
                    read_cells(at_in, p.tx.types(), &mut peer.tx);
                }
                at_in += p.tx.len() as i32;
            }
            if ready {
                master.step(now, &mut core.link);
            }
            let zero = [0u32; MAX_VALUES];
            for (i, p) in core.cfg.peers().enumerate() {
                let Some(peer) = master.peer(i) else { continue };
                bad = bad.wrapping_add(peer.stats.bad_crc);
                // An offline slave shows zeros and `online = false`, never stale answers.
                let live = ready && peer.online;
                write_cells(at_out, p.rx.types(), if live { &peer.rx } else { &zero });
                at_out += p.rx.len() as i32;
                unsafe { output_i(at_out, i32::from(live)) };
                at_out += 1;
            }
        }
        Role::Slave(slave) => {
            if let Some(p) = core.cfg.peers().next() {
                read_cells(at_in, p.tx.types(), &mut slave.tx);
                if ready {
                    slave.step(now, &mut core.link);
                }
                bad = bad
                    .wrapping_add(slave.stats.bad_crc)
                    .wrapping_add(slave.stats.bad_frame);
                // The last commands stay while the centre is silent; `linked` says so.
                write_cells(at_out, p.rx.types(), &slave.rx);
                at_out += p.rx.len() as i32;
                unsafe { output_i(at_out, i32::from(ready && slave.linked(now))) };
            }
        }
    }
    unsafe { output_i(OUT_BAD, bad as i32) };
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
