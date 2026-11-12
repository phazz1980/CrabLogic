//! Modbus of the project (`modbus(op, a, b, c, d)`): the protocol is `crate::modbus` (tested on the
//! PC); here are its ports. A role committed by a module opens its UART (RTU) or socket (TCP); a task
//! of the master polls every role every few milliseconds, modules only read and write the tables.

use crab_master_rs::modbus::{ModbusHost, Role, RoleSettings, Transport};
use esp_idf_svc::sys;
use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(2);
/// A TCP master that cannot reach its server tries again this often.
const RECONNECT: Duration = Duration::from_secs(2);
const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);

/// Default pins of a Modbus UART when the block leaves them at -1 (those of `uart.rs`).
#[cfg(target_arch = "riscv32")]
const DEFAULT_PINS: [(i32, i32); 2] = [(-1, -1), (18, 19)];
#[cfg(not(target_arch = "riscv32"))]
const DEFAULT_PINS: [(i32, i32); 3] = [(-1, -1), (26, 27), (16, 17)];

/// The protocol state and the open ports of every role.
struct State {
    host: ModbusHost,
    rtu_master: Option<Uart>,
    rtu_slave: Option<Uart>,
    tcp_master: Option<TcpClient>,
    tcp_slave: Option<TcpServer>,
    task: bool,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// `modbus(op, a, b, c, d)`: the import of modules.
pub fn call(op: i32, a: i32, b: i32, c: i32, d: i32) -> i32 {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let state = guard.get_or_insert_with(|| State {
        host: ModbusHost::new(),
        rtu_master: None,
        rtu_slave: None,
        tcp_master: None,
        tcp_slave: None,
        task: false,
    });
    let result = state.host.call(op, a, b, c, d);
    for role in state.host.take_new_roles() {
        open(state, role);
    }
    if !state.task && !state.host.roles().is_empty() {
        state.task = start_task();
    }
    result
}

fn note(text: String) {
    super::notes::push("modbus", text);
}

fn open(state: &mut State, role: RoleSettings) {
    match (role.role, role.uart, role.tcp) {
        (Role::RtuMaster | Role::RtuSlave, Some(u), _) => {
            let Some(port) = Uart::open(u.uart, u.speed, u.parity, u.rx, u.tx, u.de) else {
                note(format!(
                    "UART{} не открыт (занят или неверные выводы)",
                    u.uart
                ));
                return;
            };
            if role.role == Role::RtuMaster {
                state.rtu_master = Some(port);
            } else {
                state.rtu_slave = Some(port);
            }
        }
        (Role::TcpMaster, _, Some(t)) => {
            let ip = Ipv4Addr::from(t.server_ip);
            state.tcp_master = Some(TcpClient::new(SocketAddr::from((ip, t.port))));
        }
        (Role::TcpSlave, _, Some(t)) => state.tcp_slave = Some(TcpServer::new(t.port)),
        _ => note("роль без настроек порта".into()),
    }
}

fn start_task() -> bool {
    let spawned = std::thread::Builder::new()
        .name("modbus".into())
        .stack_size(6144)
        .spawn(|| {
            let started = Instant::now();
            loop {
                poll_all(started.elapsed().as_millis() as u64);
                std::thread::sleep(POLL);
            }
        });
    if spawned.is_err() {
        note("задача опроса не запущена".into());
    }
    spawned.is_ok()
}

fn poll_all(now_ms: u64) {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = guard.as_mut() else { return };
    if let Some(t) = &mut s.rtu_master {
        s.host.poll_rtu_master(now_ms, t);
    }
    if let Some(t) = &mut s.rtu_slave {
        s.host.poll_rtu_slave(now_ms, t);
    }
    if let Some(t) = &mut s.tcp_master {
        s.host.poll_tcp_master(now_ms, t);
    }
    if let Some(t) = &mut s.tcp_slave {
        t.accept();
        s.host.poll_tcp_slave(now_ms, t);
    }
}

/// A UART of the ESP-IDF driver (RS-485 half duplex when the DE pin is set).
struct Uart {
    port: sys::uart_port_t,
}

impl Uart {
    fn open(uart: i32, speed: u32, parity: i32, rx: i32, tx: i32, de: i32) -> Option<Self> {
        let n = usize::try_from(uart)
            .ok()
            .filter(|&n| n >= 1 && n < DEFAULT_PINS.len())?;
        let port = n as sys::uart_port_t;
        let pin = |v: i32, default: i32| if v >= 0 { v } else { default };
        let (rx, tx) = (pin(rx, DEFAULT_PINS[n].0), pin(tx, DEFAULT_PINS[n].1));
        let cfg = sys::uart_config_t {
            baud_rate: if speed > 0 { speed as i32 } else { 9600 },
            data_bits: sys::uart_word_length_t_UART_DATA_8_BITS,
            parity: match parity {
                1 => sys::uart_parity_t_UART_PARITY_EVEN,
                2 => sys::uart_parity_t_UART_PARITY_ODD,
                _ => sys::uart_parity_t_UART_PARITY_DISABLE,
            },
            stop_bits: sys::uart_stop_bits_t_UART_STOP_BITS_1,
            flow_ctrl: sys::uart_hw_flowcontrol_t_UART_HW_FLOWCTRL_DISABLE,
            ..Default::default()
        };
        // SAFETY: a valid port number; the driver is installed first, so a port in use is left alone.
        unsafe {
            if sys::uart_driver_install(port, 512, 512, 0, core::ptr::null_mut(), 0) != 0 {
                return None;
            }
            let rts = if de >= 0 { de } else { -1 };
            if sys::uart_param_config(port, &cfg) != 0
                || sys::uart_set_pin(port, tx, rx, rts, -1) != 0
            {
                sys::uart_driver_delete(port);
                return None;
            }
            if de >= 0 {
                sys::uart_set_mode(port, sys::uart_mode_t_UART_MODE_RS485_HALF_DUPLEX);
            }
        }
        Some(Self { port })
    }
}

impl Transport for Uart {
    fn send(&mut self, bytes: &[u8]) {
        // SAFETY: an installed port; the driver copies the bytes into its ring buffer.
        unsafe { sys::uart_write_bytes(self.port, bytes.as_ptr().cast(), bytes.len()) };
    }

    fn receive(&mut self, buf: &mut Vec<u8>) {
        let mut chunk = [0u8; 128];
        loop {
            // SAFETY: an installed port; at most `chunk.len()` bytes, no waiting.
            let n = unsafe {
                sys::uart_read_bytes(self.port, chunk.as_mut_ptr().cast(), chunk.len() as u32, 0)
            };
            if n <= 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n as usize]);
        }
    }
}

/// Reads what a non-blocking stream has; `false` when the peer closed it or it failed.
fn read_stream(stream: &mut TcpStream, buf: &mut Vec<u8>) -> bool {
    let mut chunk = [0u8; 260];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => return false,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == ErrorKind::WouldBlock => return true,
            Err(_) => return false,
        }
    }
}

/// The connection of a TCP master to its server.
struct TcpClient {
    server: SocketAddr,
    stream: Option<TcpStream>,
    last_try: Option<Instant>,
}

impl TcpClient {
    fn new(server: SocketAddr) -> Self {
        Self {
            server,
            stream: None,
            last_try: None,
        }
    }
}

impl Transport for TcpClient {
    fn send(&mut self, bytes: &[u8]) {
        if let Some(s) = &mut self.stream {
            if s.write_all(bytes).is_err() {
                self.stream = None;
            }
        }
    }

    fn receive(&mut self, buf: &mut Vec<u8>) {
        if let Some(s) = &mut self.stream {
            if !read_stream(s, buf) {
                self.stream = None;
            }
        }
    }

    fn is_connected(&mut self) -> bool {
        self.stream.is_some()
    }

    fn connect(&mut self) {
        if self.stream.is_some() || self.last_try.is_some_and(|t| t.elapsed() < RECONNECT) {
            return;
        }
        self.last_try = Some(Instant::now());
        if let Ok(s) = TcpStream::connect_timeout(&self.server, CONNECT_TIMEOUT) {
            if s.set_nonblocking(true).is_ok() {
                let _ = s.set_nodelay(true);
                self.stream = Some(s);
            }
        }
    }

    fn close(&mut self) {
        self.stream = None;
    }
}

/// A TCP slave: one client at a time, a new connection replaces the old one.
struct TcpServer {
    port: u16,
    listener: Option<TcpListener>,
    client: Option<TcpStream>,
    last_try: Option<Instant>,
}

impl TcpServer {
    fn new(port: u16) -> Self {
        Self {
            port,
            listener: None,
            client: None,
            last_try: None,
        }
    }

    /// Opens the listener once the network is up, then takes a waiting client.
    fn accept(&mut self) {
        if self.listener.is_none() {
            if self.last_try.is_some_and(|t| t.elapsed() < RECONNECT) {
                return;
            }
            self.last_try = Some(Instant::now());
            let port = if self.port == 0 { 502 } else { self.port };
            match TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)) {
                Ok(l) if l.set_nonblocking(true).is_ok() => self.listener = Some(l),
                _ => return,
            }
        }
        if let Some(l) = &self.listener {
            if let Ok((s, _)) = l.accept() {
                if s.set_nonblocking(true).is_ok() {
                    let _ = s.set_nodelay(true);
                    self.client = Some(s);
                }
            }
        }
    }
}

impl Transport for TcpServer {
    fn send(&mut self, bytes: &[u8]) {
        if let Some(s) = &mut self.client {
            if s.write_all(bytes).is_err() {
                self.client = None;
            }
        }
    }

    fn receive(&mut self, buf: &mut Vec<u8>) {
        if let Some(s) = &mut self.client {
            if !read_stream(s, buf) {
                self.client = None;
            }
        }
    }

    fn is_connected(&mut self) -> bool {
        self.client.is_some()
    }
}
