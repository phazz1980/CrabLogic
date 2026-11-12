//! Modbus (RTU and TCP, master and slave) for the Rust master, without any hardware.
//!
//! This is a port of the part of the FLProg Modbus library that the C++ master used (see
//! `h_modbus` in `specs/wasm-on-mcu/proto/master/master.ino`). The board glue only moves bytes
//! between a [`Transport`] (a UART driver or a TCP stream) and the engines here; time is passed in
//! as `now_ms`, so everything runs in desktop tests.
//!
//! Layout:
//! - [`Tables`]: holding / input registers, coils, discrete inputs with typed access;
//! - frame codec: [`crc16`], [`Request`], RTU and MBAP framing helpers;
//! - [`SlaveUnit`] (request -> reply), [`RtuSlave`], [`TcpSlave`];
//! - [`Master`] (RTU or TCP polling engine over up to four remote slaves);
//! - [`ModbusHost`]: the `modbus(op, a, b, c, d)` import of WASM modules.
//!
//! Differences from the C++ library are listed in the comments marked `Flprog:`.

use alloc::vec;
use alloc::vec::Vec;

/// Largest frame (unit id + PDU, without CRC or MBAP header) the engines accept.
pub const MAX_FRAME: usize = 250;
/// Registers or coils moved by one request (the library's limit).
pub const MAX_REGS: usize = 25;
/// Largest table the host creates, to keep the heap safe from a bad setting.
pub const MAX_TABLE: usize = 4096;
/// Idle time after which a partly received RTU frame is dropped.
const RX_IDLE_MS: u64 = 50;

/// Error code: the remote slave did not answer in time (`getLastError` of the library).
pub const ERR_TIMEOUT: u8 = 244;
/// Error code: bad CRC.
pub const ERR_CRC: u8 = 252;
/// Error code: malformed frame, wrong MBAP header, unit or function.
pub const ERR_FRAME: u8 = 253;
/// What the master reports for a slave address it does not know.
pub const ERR_NO_SLAVE: u8 = 50;

// ---------------------------------------------------------------------------------------------
// Tables
// ---------------------------------------------------------------------------------------------

/// A Modbus data table. The order of the variants is the selector of the module import
/// (`b & 3`: holding, input, coil, discrete).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Table {
    Holding,
    Input,
    Coil,
    Discrete,
}

impl Table {
    /// The master polls tables in this order (as `firstTabe` / `nextTable` do).
    const POLL_ORDER: [Table; 4] = [Table::Coil, Table::Discrete, Table::Input, Table::Holding];

    /// Table from the low two bits of the module's selector.
    pub fn from_selector(b: i32) -> Table {
        match b & 3 {
            0 => Table::Holding,
            1 => Table::Input,
            2 => Table::Coil,
            _ => Table::Discrete,
        }
    }

    fn index(self) -> usize {
        match self {
            Table::Holding => 0,
            Table::Input => 1,
            Table::Coil => 2,
            Table::Discrete => 3,
        }
    }

    fn is_bits(self) -> bool {
        matches!(self, Table::Coil | Table::Discrete)
    }

    /// The next table after `self` in the polling order (`None` after the last one).
    fn poll_next(self, has: impl Fn(Table) -> bool) -> Option<Table> {
        let at = Self::POLL_ORDER.iter().position(|t| *t == self)?;
        Self::POLL_ORDER[at + 1..].iter().copied().find(|t| has(*t))
    }
}

/// One table: addresses are `0..size`. Bits are stored as 0 or 1.
#[derive(Debug, Clone)]
struct DataTable {
    cells: Vec<i16>,
    /// Set by a change made by the program; the master sends such cells to the slave.
    dirty: Vec<bool>,
    bits: bool,
}

impl DataTable {
    fn new(table: Table, size: usize) -> Self {
        DataTable {
            cells: vec![0; size],
            dirty: vec![false; size],
            bits: table.is_bits(),
        }
    }

    fn size(&self) -> usize {
        self.cells.len()
    }

    fn get(&self, addr: i32) -> Option<usize> {
        usize::try_from(addr).ok().filter(|a| *a < self.size())
    }

    fn word(&self, addr: i32) -> i16 {
        match self.get(addr) {
            Some(i) if !self.bits => self.cells[i],
            _ => 0,
        }
    }

    fn bit(&self, addr: i32) -> bool {
        match self.get(addr) {
            Some(i) if self.bits => self.cells[i] != 0,
            _ => false,
        }
    }

    /// A write of the program: flags the cell for sending when the value changed.
    fn save(&mut self, addr: i32, value: i16) {
        if let Some(i) = self.get(addr) {
            if self.cells[i] != value {
                self.cells[i] = value;
                self.dirty[i] = true;
            }
        }
    }

    /// A write that came from the network: no flag.
    fn put(&mut self, addr: i32, value: i16) {
        if let Some(i) = self.get(addr) {
            self.cells[i] = value;
        }
    }
}

/// The four tables of a slave (own or remote) with the typed access of `ModbusMainData`.
#[derive(Debug, Clone)]
pub struct Tables {
    slots: [Option<DataTable>; 4],
    /// A remote slave in a master: the program may only save to holding registers and coils.
    master_side: bool,
}

impl Tables {
    pub fn new(master_side: bool) -> Self {
        Tables {
            slots: [None, None, None, None],
            master_side,
        }
    }

    /// Creates (or re-creates, empty) a table of `size` cells.
    pub fn configure(&mut self, table: Table, size: usize) {
        self.slots[table.index()] = Some(DataTable::new(table, size.min(MAX_TABLE)));
    }

    pub fn has(&self, table: Table) -> bool {
        self.slots[table.index()].is_some()
    }

    pub fn size(&self, table: Table) -> usize {
        self.slots[table.index()]
            .as_ref()
            .map_or(0, DataTable::size)
    }

    fn slot(&self, table: Table) -> Option<&DataTable> {
        self.slots[table.index()].as_ref()
    }

    fn slot_mut(&mut self, table: Table) -> Option<&mut DataTable> {
        self.slots[table.index()].as_mut()
    }

    fn can_save(&self, table: Table) -> bool {
        self.has(table) && (!self.master_side || matches!(table, Table::Holding | Table::Coil))
    }

    /// `readInteger`: one register as a signed 16-bit value (0 if absent).
    pub fn read_int(&self, table: Table, addr: i32) -> i16 {
        self.slot(table).map_or(0, |t| t.word(addr))
    }

    /// `readFloat`: two registers, high word first (the library's default order ABCD).
    /// Flprog: an absent second register gives garbage in C++, here 0.0.
    pub fn read_float(&self, table: Table, addr: i32) -> f32 {
        let Some(t) = self.slot(table) else {
            return 0.0;
        };
        if t.get(addr).is_none() || t.get(addr + 1).is_none() {
            return 0.0;
        }
        let hi = t.word(addr) as u16 as u32;
        let lo = t.word(addr + 1) as u16 as u32;
        f32::from_bits(hi << 16 | lo)
    }

    /// `readBool`.
    pub fn read_bool(&self, table: Table, addr: i32) -> bool {
        self.slot(table).is_some_and(|t| t.bit(addr))
    }

    /// `saveInteger`.
    pub fn save_int(&mut self, table: Table, addr: i32, value: i16) {
        if !self.can_save(table) || table.is_bits() {
            return;
        }
        if let Some(t) = self.slot_mut(table) {
            t.save(addr, value);
        }
    }

    /// `saveFloat`: high word at `addr`, low word at `addr + 1`; a missing second register is
    /// skipped silently, as in the library.
    pub fn save_float(&mut self, table: Table, addr: i32, value: f32) {
        if !self.can_save(table) || table.is_bits() {
            return;
        }
        let bits = value.to_bits();
        if let Some(t) = self.slot_mut(table) {
            t.save(addr, (bits >> 16) as u16 as i16);
            t.save(addr + 1, bits as u16 as i16);
        }
    }

    /// `saveBool`.
    pub fn save_bool(&mut self, table: Table, addr: i32, value: bool) {
        if !self.can_save(table) || !table.is_bits() {
            return;
        }
        if let Some(t) = self.slot_mut(table) {
            t.save(addr, i16::from(value));
        }
    }

    fn first_dirty(&self, table: Table) -> Option<usize> {
        self.slot(table)?.dirty.iter().position(|d| *d)
    }

    fn is_dirty(&self, table: Table, addr: usize) -> bool {
        self.slot(table)
            .is_some_and(|t| t.dirty.get(addr).copied().unwrap_or(false))
    }

    fn clear_dirty(&mut self, table: Table, addr: usize) {
        if let Some(t) = self.slot_mut(table) {
            if let Some(d) = t.dirty.get_mut(addr) {
                *d = false;
            }
        }
    }

    /// Checks `count` cells from `start` for a slave reply: at most [`MAX_REGS`], all present.
    fn range_ok(&self, table: Table, start: usize, count: usize) -> bool {
        count <= MAX_REGS && start + count <= self.size(table)
    }

    /// Serves a request as a slave. `Err` is a Modbus exception code.
    fn serve(&mut self, req: &Request) -> Result<Vec<u8>, u8> {
        let table = req.table();
        if !self.has(table) {
            return Err(1);
        }
        let function = req.function();
        match req {
            Request::ReadBits { start, count, .. } => {
                let (s, n) = (*start as usize, *count as usize);
                if !self.range_ok(table, s, n) {
                    return Err(2);
                }
                let mut out = vec![function, n.div_ceil(8) as u8];
                out.extend(pack_bits(
                    (0..n).map(|i| self.read_bool(table, (s + i) as i32)),
                ));
                Ok(out)
            }
            Request::ReadWords { start, count, .. } => {
                let (s, n) = (*start as usize, *count as usize);
                if !self.range_ok(table, s, n) {
                    return Err(2);
                }
                let mut out = vec![function, (n * 2) as u8];
                for i in 0..n {
                    out.extend(self.read_int(table, (s + i) as i32).to_be_bytes());
                }
                Ok(out)
            }
            Request::WriteCoil { addr, on } => {
                if !self.range_ok(table, *addr as usize, 1) {
                    return Err(2);
                }
                self.put_bit(*addr as i32, *on);
                let mut out = vec![function];
                out.extend(addr.to_be_bytes());
                out.extend(if *on { [0xFF, 0] } else { [0, 0] });
                Ok(out)
            }
            Request::WriteRegister { addr, value } => {
                if !self.range_ok(table, *addr as usize, 1) {
                    return Err(2);
                }
                self.put_word(*addr as i32, *value);
                let mut out = vec![function];
                out.extend(addr.to_be_bytes());
                out.extend(value.to_be_bytes());
                Ok(out)
            }
            Request::WriteCoils { start, values } => {
                if !self.range_ok(table, *start as usize, values.len()) {
                    return Err(2);
                }
                for (i, v) in values.iter().enumerate() {
                    self.put_bit(*start as i32 + i as i32, *v);
                }
                let mut out = vec![function];
                out.extend(start.to_be_bytes());
                out.extend((values.len() as u16).to_be_bytes());
                Ok(out)
            }
            Request::WriteRegisters { start, values } => {
                if !self.range_ok(table, *start as usize, values.len()) {
                    return Err(2);
                }
                for (i, v) in values.iter().enumerate() {
                    self.put_word(*start as i32 + i as i32, *v);
                }
                let mut out = vec![function];
                out.extend(start.to_be_bytes());
                out.extend((values.len() as u16).to_be_bytes());
                Ok(out)
            }
        }
    }

    fn put_bit(&mut self, addr: i32, on: bool) {
        if let Some(t) = self.slot_mut(Table::Coil) {
            t.put(addr, i16::from(on));
        }
    }

    fn put_word(&mut self, addr: i32, value: i16) {
        if let Some(t) = self.slot_mut(Table::Holding) {
            t.put(addr, value);
        }
    }
}

/// Bits into bytes, the first bit in the lowest bit of the first byte.
fn pack_bits(bits: impl Iterator<Item = bool>) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    for (i, b) in bits.enumerate() {
        if i % 8 == 0 {
            out.push(0);
        }
        if b {
            *out.last_mut().expect("a byte was pushed") |= 1 << (i % 8);
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Frame codec
// ---------------------------------------------------------------------------------------------

/// CRC16 of Modbus (polynomial 0xA001, start 0xFFFF).
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0xFFFFu16;
    for b in data {
        crc ^= u16::from(*b);
        for _ in 0..8 {
            let low = crc & 1 != 0;
            crc >>= 1;
            if low {
                crc ^= 0xA001;
            }
        }
    }
    crc
}

/// The body with the CRC appended (low byte first), ready for the wire.
pub fn with_crc(body: &[u8]) -> Vec<u8> {
    let mut out = body.to_vec();
    out.extend(crc16(body).to_le_bytes());
    out
}

/// Whether a whole RTU frame (with its CRC) is consistent.
pub fn crc_ok(frame: &[u8]) -> bool {
    frame.len() >= 4 && {
        let n = frame.len() - 2;
        crc16(&frame[..n]).to_le_bytes() == [frame[n], frame[n + 1]]
    }
}

/// Size of an RTU request that starts with `buf`, or 0 while it is not known (or the function is
/// not supported). The library frames by length, not by silence.
pub fn request_frame_size(buf: &[u8]) -> usize {
    if buf.len() < 2 {
        return 0;
    }
    let f = buf[1];
    if f & 0x80 != 0 {
        return 5;
    }
    if (7..15).contains(&f) || f > 16 {
        return 0;
    }
    if f < 15 {
        return 8;
    }
    if buf.len() < 7 {
        return 0;
    }
    buf[6] as usize + 9
}

/// Size of an RTU response that starts with `buf` (see [`request_frame_size`]).
pub fn response_frame_size(buf: &[u8]) -> usize {
    if buf.len() < 2 {
        return 0;
    }
    let f = buf[1];
    if f & 0x80 != 0 {
        return 5;
    }
    if (7..15).contains(&f) || f > 16 {
        return 0;
    }
    if f > 4 {
        return 8;
    }
    if buf.len() < 3 {
        return 0;
    }
    buf[2] as usize + 5
}

/// The 3.5-character silence of RTU in microseconds (fixed 1750 us above 19200 baud).
pub fn t35_us(speed: u32) -> u32 {
    if speed > 19_200 {
        1750
    } else {
        38_500_000 / speed.max(1)
    }
}

/// Time to send `len` bytes (11 bits per character, rounded up), in milliseconds.
pub fn frame_time_ms(len: usize, speed: u32) -> u64 {
    let speed = u64::from(speed.max(1));
    (len as u64 * 11 * 1000).div_ceil(speed).max(1)
}

/// A request of the supported functions 1, 2, 3, 4, 5, 6, 15 and 16.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Function 1 (coils) or 2 (discrete inputs).
    ReadBits {
        table: Table,
        start: u16,
        count: u16,
    },
    /// Function 3 (holding registers) or 4 (input registers).
    ReadWords {
        table: Table,
        start: u16,
        count: u16,
    },
    /// Function 5.
    WriteCoil { addr: u16, on: bool },
    /// Function 6.
    WriteRegister { addr: u16, value: i16 },
    /// Function 15.
    WriteCoils { start: u16, values: Vec<bool> },
    /// Function 16.
    WriteRegisters { start: u16, values: Vec<i16> },
}

impl Request {
    pub fn function(&self) -> u8 {
        match self {
            Request::ReadBits { table, .. } => {
                if *table == Table::Coil {
                    1
                } else {
                    2
                }
            }
            Request::ReadWords { table, .. } => {
                if *table == Table::Holding {
                    3
                } else {
                    4
                }
            }
            Request::WriteCoil { .. } => 5,
            Request::WriteRegister { .. } => 6,
            Request::WriteCoils { .. } => 15,
            Request::WriteRegisters { .. } => 16,
        }
    }

    /// The table the request works on.
    pub fn table(&self) -> Table {
        match self {
            Request::ReadBits { table, .. } | Request::ReadWords { table, .. } => *table,
            Request::WriteCoil { .. } | Request::WriteCoils { .. } => Table::Coil,
            Request::WriteRegister { .. } | Request::WriteRegisters { .. } => Table::Holding,
        }
    }

    /// Unit id + PDU (no CRC, no MBAP header).
    pub fn encode(&self, unit: u8) -> Vec<u8> {
        let mut out = vec![unit, self.function()];
        match self {
            Request::ReadBits { start, count, .. } | Request::ReadWords { start, count, .. } => {
                out.extend(start.to_be_bytes());
                out.extend(count.to_be_bytes());
            }
            Request::WriteCoil { addr, on } => {
                out.extend(addr.to_be_bytes());
                out.extend(if *on { [0xFF, 0] } else { [0, 0] });
            }
            Request::WriteRegister { addr, value } => {
                out.extend(addr.to_be_bytes());
                out.extend(value.to_be_bytes());
            }
            Request::WriteCoils { start, values } => {
                out.extend(start.to_be_bytes());
                out.extend((values.len() as u16).to_be_bytes());
                let bytes = pack_bits(values.iter().copied());
                out.push(bytes.len() as u8);
                out.extend(bytes);
            }
            Request::WriteRegisters { start, values } => {
                out.extend(start.to_be_bytes());
                out.extend((values.len() as u16).to_be_bytes());
                out.push((values.len() * 2) as u8);
                for v in values {
                    out.extend(v.to_be_bytes());
                }
            }
        }
        out
    }

    /// Parses unit id + PDU. `Err` is the exception code to answer with: 1 for a function that is
    /// not supported, 3 for a frame that is too short or inconsistent.
    pub fn parse(body: &[u8]) -> Result<Request, u8> {
        if body.len() < 2 {
            return Err(3);
        }
        let word = |i: usize| u16::from_be_bytes([body[i], body[i + 1]]);
        let f = body[1];
        if !matches!(f, 1..=6 | 15 | 16) {
            return Err(1);
        }
        if body.len() < 6 {
            return Err(3);
        }
        let (a, b) = (word(2), word(4));
        Ok(match f {
            1 | 2 => Request::ReadBits {
                table: if f == 1 { Table::Coil } else { Table::Discrete },
                start: a,
                count: b,
            },
            3 | 4 => Request::ReadWords {
                table: if f == 3 { Table::Holding } else { Table::Input },
                start: a,
                count: b,
            },
            5 => Request::WriteCoil {
                addr: a,
                on: body[4] == 0xFF,
            },
            6 => Request::WriteRegister {
                addr: a,
                value: b as i16,
            },
            _ => {
                if body.len() < 7 {
                    return Err(3);
                }
                let (n, bytes) = (b as usize, body[6] as usize);
                if body.len() < 7 + bytes {
                    return Err(3);
                }
                let data = &body[7..7 + bytes];
                if f == 15 {
                    if bytes < n.div_ceil(8) {
                        return Err(3);
                    }
                    let values = (0..n).map(|i| data[i / 8] >> (i % 8) & 1 != 0).collect();
                    Request::WriteCoils { start: a, values }
                } else {
                    if bytes != n * 2 {
                        return Err(3);
                    }
                    let values = (0..n)
                        .map(|i| i16::from_be_bytes([data[2 * i], data[2 * i + 1]]))
                        .collect();
                    Request::WriteRegisters { start: a, values }
                }
            }
        })
    }
}

/// An MBAP-framed message of Modbus TCP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MbapFrame {
    pub txn: u16,
    pub proto: u16,
    /// Unit id + PDU.
    pub body: Vec<u8>,
}

/// `[txn][proto][length][unit + PDU]`.
pub fn mbap_encode(txn: u16, proto: u16, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(6 + body.len());
    out.extend(txn.to_be_bytes());
    out.extend(proto.to_be_bytes());
    out.extend((body.len() as u16).to_be_bytes());
    out.extend(body);
    out
}

/// Takes one whole frame off the front of a stream buffer. A header with a zero or too big length
/// resets the buffer (as the library does).
pub fn mbap_take(buf: &mut Vec<u8>) -> Option<MbapFrame> {
    if buf.len() < 6 {
        return None;
    }
    let len = u16::from_be_bytes([buf[4], buf[5]]) as usize;
    if len == 0 || len > MAX_FRAME {
        buf.clear();
        return None;
    }
    if buf.len() < 6 + len {
        return None;
    }
    let frame = MbapFrame {
        txn: u16::from_be_bytes([buf[0], buf[1]]),
        proto: u16::from_be_bytes([buf[2], buf[3]]),
        body: buf[6..6 + len].to_vec(),
    };
    buf.drain(..6 + len);
    Some(frame)
}

// ---------------------------------------------------------------------------------------------
// Transport
// ---------------------------------------------------------------------------------------------

/// Bytes in and out: a UART driver for RTU, a TCP stream for Modbus TCP. Nothing here may block.
pub trait Transport {
    /// Queues the bytes for sending.
    fn send(&mut self, bytes: &[u8]);
    /// Appends everything received since the last call to `buf`.
    fn receive(&mut self, buf: &mut Vec<u8>);
    /// TCP: whether the stream is connected (a UART is always "connected").
    fn is_connected(&mut self) -> bool {
        true
    }
    /// TCP master: start connecting to the server (called on every poll while not connected, so
    /// it must be cheap and idempotent).
    fn connect(&mut self) {}
    /// TCP master: drop the connection (after a timeout).
    fn close(&mut self) {}
}

fn drain<T: Transport>(t: &mut T) {
    let mut sink = Vec::new();
    t.receive(&mut sink);
}

// ---------------------------------------------------------------------------------------------
// Slave
// ---------------------------------------------------------------------------------------------

/// The slave logic shared by RTU and TCP: own tables, own address and the last error.
#[derive(Debug, Clone)]
pub struct SlaveUnit {
    address: u8,
    pub tables: Tables,
    last_error: u8,
}

impl SlaveUnit {
    pub fn new(address: u8) -> Self {
        SlaveUnit {
            address,
            tables: Tables::new(false),
            last_error: 0,
        }
    }

    pub fn address(&self) -> u8 {
        self.address
    }

    /// `getLastError`: 0 after a good request, the exception code (1, 2, 3) after a refused one,
    /// 252 for a CRC error and 253 for a bad frame.
    pub fn last_error(&self) -> u8 {
        self.last_error
    }

    pub fn set_last_error(&mut self, code: u8) {
        self.last_error = code;
    }

    /// Whether a frame for `unit` is ours: our address, the broadcast 0, or any if ours is 0.
    pub fn accepts(&self, unit: u8) -> bool {
        self.address == 0 || unit == self.address || unit == 0
    }

    /// Handles unit id + PDU (the address filter and the CRC / MBAP checks are done by the
    /// caller). Returns the reply (unit id + PDU), or `None` for a broadcast.
    /// Flprog: the exception reply carries our unit id (C++ always wrote 1) and the count
    /// in the reply of function 16 is the whole word.
    pub fn handle(&mut self, body: &[u8]) -> Option<Vec<u8>> {
        if body.len() < 2 {
            return None;
        }
        let unit = body[0];
        let reply = match Request::parse(body).and_then(|r| self.tables.serve(&r)) {
            Ok(pdu) => {
                self.last_error = 0;
                let mut out = vec![unit];
                out.extend(pdu);
                out
            }
            Err(code) => {
                self.last_error = code;
                vec![unit, body[1] | 0x80, code]
            }
        };
        (unit != 0).then_some(reply)
    }
}

#[derive(Debug, Clone)]
enum SlaveState {
    Idle,
    /// A whole request is waiting for the pause before it is answered.
    Pause {
        frame: Vec<u8>,
        at: u64,
    },
    /// The reply is going out.
    Sending {
        until: u64,
    },
}

/// Modbus RTU slave: poll it with the UART.
#[derive(Debug, Clone)]
pub struct RtuSlave {
    pub unit: SlaveUnit,
    speed: u32,
    rx: Vec<u8>,
    last_rx: u64,
    state: SlaveState,
    pause_ms: u64,
}

impl RtuSlave {
    pub fn new(address: u8, speed: u32) -> Self {
        let speed = if speed == 0 { 9600 } else { speed };
        RtuSlave {
            unit: SlaveUnit::new(address),
            speed,
            rx: Vec::new(),
            last_rx: 0,
            state: SlaveState::Idle,
            // The library waits 10 ms before it answers; never less than the 3.5-character silence.
            pause_ms: 10u64.max(u64::from(t35_us(speed)).div_ceil(1000)),
        }
    }

    pub fn poll<T: Transport>(&mut self, now_ms: u64, t: &mut T) {
        match core::mem::replace(&mut self.state, SlaveState::Idle) {
            SlaveState::Sending { until } if now_ms < until => {
                self.state = SlaveState::Sending { until };
                return;
            }
            SlaveState::Pause { frame, at } => {
                if now_ms < at {
                    self.state = SlaveState::Pause { frame, at };
                } else {
                    self.execute(now_ms, &frame, t);
                }
                return;
            }
            _ => {}
        }
        let mut chunk = Vec::new();
        t.receive(&mut chunk);
        for byte in chunk {
            if self.rx.len() >= MAX_FRAME + 2 {
                self.rx.clear();
            }
            self.rx.push(byte);
            self.last_rx = now_ms;
            let size = request_frame_size(&self.rx);
            if size != 0 && self.rx.len() == size {
                let frame = core::mem::take(&mut self.rx);
                self.state = SlaveState::Pause {
                    frame,
                    at: now_ms + self.pause_ms,
                };
                return;
            }
        }
        if !self.rx.is_empty() && now_ms.saturating_sub(self.last_rx) >= RX_IDLE_MS {
            self.rx.clear();
        }
    }

    fn execute<T: Transport>(&mut self, now_ms: u64, frame: &[u8], t: &mut T) {
        if !self.unit.accepts(frame[0]) {
            return;
        }
        if !crc_ok(frame) {
            self.unit.set_last_error(ERR_CRC);
            return;
        }
        if let Some(reply) = self.unit.handle(&frame[..frame.len() - 2]) {
            let bytes = with_crc(&reply);
            drain(t);
            t.send(&bytes);
            self.state = SlaveState::Sending {
                until: now_ms + frame_time_ms(bytes.len(), self.speed),
            };
        }
    }
}

/// Modbus TCP slave: poll it with the accepted connection.
#[derive(Debug, Clone)]
pub struct TcpSlave {
    pub unit: SlaveUnit,
    rx: Vec<u8>,
    rx_started: u64,
}

impl TcpSlave {
    pub fn new(address: u8) -> Self {
        TcpSlave {
            unit: SlaveUnit::new(address),
            rx: Vec::new(),
            rx_started: 0,
        }
    }

    pub fn poll<T: Transport>(&mut self, now_ms: u64, t: &mut T) {
        if !t.is_connected() {
            self.rx.clear();
            return;
        }
        // A frame that does not complete in a second is dropped.
        if !self.rx.is_empty() && now_ms.saturating_sub(self.rx_started) >= 1000 {
            self.rx.clear();
        }
        let before = self.rx.len();
        t.receive(&mut self.rx);
        if before == 0 && !self.rx.is_empty() {
            self.rx_started = now_ms;
        }
        while let Some(frame) = mbap_take(&mut self.rx) {
            if !self.unit.accepts(frame.body[0]) {
                continue;
            }
            if frame.proto != 0 {
                self.unit.set_last_error(ERR_FRAME);
                continue;
            }
            if let Some(reply) = self.unit.handle(&frame.body) {
                t.send(&mbap_encode(frame.txn, frame.proto, &reply));
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Master
// ---------------------------------------------------------------------------------------------

/// How a master talks to one remote slave.
#[derive(Debug, Clone)]
pub struct RemoteConfig {
    /// Slave address (Unit ID for TCP).
    pub address: u8,
    /// Pause between two polling cycles of this slave.
    pub period_ms: u32,
    /// How long to wait for an answer.
    pub timeout_ms: u32,
    /// Sizes of the tables in the selector order: holding, input, coils, discrete (0 = none).
    pub sizes: [usize; 4],
}

#[derive(Debug, Clone)]
struct Remote {
    address: u8,
    period_ms: u64,
    timeout_ms: u64,
    tables: Tables,
    last_request: u64,
    init: bool,
    online: bool,
    last_error: u8,
    /// No requests to this slave before this time (after a timeout).
    pause_until: u64,
}

impl Remote {
    fn new(cfg: &RemoteConfig) -> Self {
        let mut tables = Tables::new(true);
        for (i, size) in cfg.sizes.iter().enumerate() {
            if *size > 0 {
                tables.configure(Table::from_selector(i as i32), *size);
            }
        }
        Remote {
            address: cfg.address,
            period_ms: u64::from(cfg.period_ms),
            // Flprog: a zero timeout would expire at once; take the library's default instead.
            timeout_ms: if cfg.timeout_ms == 0 {
                1000
            } else {
                u64::from(cfg.timeout_ms)
            },
            tables,
            last_request: 0,
            init: false,
            // Flprog: a slave counts as active until it fails.
            online: true,
            last_error: 0,
            pause_until: 0,
        }
    }

    fn first_table(&self) -> Option<Table> {
        Table::POLL_ORDER
            .iter()
            .copied()
            .find(|t| self.tables.has(*t))
    }

    fn paused(&self, now: u64) -> bool {
        now < self.pause_until
    }

    fn is_ready(&self, now: u64) -> bool {
        !self.paused(now)
            && self.first_table().is_some()
            && (!self.init || now.saturating_sub(self.last_request) >= self.period_ms)
    }

    fn has_write(&self, now: u64) -> bool {
        !self.paused(now)
            && (self.tables.first_dirty(Table::Holding).is_some()
                || self.tables.first_dirty(Table::Coil).is_some())
    }
}

/// How a master frames its messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framing {
    Rtu,
    Tcp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MasterState {
    Ready,
    /// TCP: waiting for the connection before sending.
    Connecting {
        since: u64,
    },
    /// RTU: the request is going out.
    Sending {
        until: u64,
    },
    Waiting {
        since: u64,
    },
    /// RTU: the pause between a reply and the next request.
    Pause {
        until: u64,
    },
}

#[derive(Debug, Clone)]
struct Telegram {
    remote: usize,
    request: Request,
    start: usize,
    count: usize,
}

/// Modbus master (RTU or TCP): polls up to four remote slaves in turn and writes what the program
/// changed. Drive it with [`Master::poll`].
#[derive(Debug, Clone)]
pub struct Master {
    framing: Framing,
    speed: u32,
    remotes: Vec<Remote>,
    state: MasterState,
    /// Polling cursor: the slave, its table and the first and last address of the last chunk.
    current: Option<usize>,
    cur_table: Table,
    cur_start: usize,
    cur_last: usize,
    tele: Option<Telegram>,
    rx: Vec<u8>,
    last_rx: u64,
    txn: u16,
    pause_ms: u64,
}

impl Master {
    /// A master over `remotes` (at most four are used). `speed` is the UART speed (RTU timing).
    pub fn new(framing: Framing, speed: u32, remotes: &[RemoteConfig]) -> Self {
        let speed = if speed == 0 { 9600 } else { speed };
        Master {
            framing,
            speed,
            remotes: remotes.iter().take(4).map(Remote::new).collect(),
            state: MasterState::Ready,
            current: None,
            cur_table: Table::Coil,
            cur_start: 0,
            cur_last: 0,
            tele: None,
            rx: Vec::new(),
            last_rx: 0,
            txn: 1,
            pause_ms: 10u64.max(u64::from(t35_us(speed)).div_ceil(1000)),
        }
    }

    fn remote_at(&self, address: u8) -> Option<&Remote> {
        self.remotes.iter().find(|r| r.address == address)
    }

    /// The tables of the slave with this address (the first one if an address repeats).
    pub fn tables(&self, address: u8) -> Option<&Tables> {
        self.remote_at(address).map(|r| &r.tables)
    }

    pub fn tables_mut(&mut self, address: u8) -> Option<&mut Tables> {
        self.remotes
            .iter_mut()
            .find(|r| r.address == address)
            .map(|r| &mut r.tables)
    }

    /// `slaveStatus`: whether the slave answered the last time (true before the first request).
    pub fn slave_status(&self, address: u8) -> bool {
        self.remote_at(address).is_some_and(|r| r.online)
    }

    /// `getLastError` of a slave: 0, an exception code, 244 timeout, 252 CRC, 253 bad frame,
    /// [`ERR_NO_SLAVE`] for an unknown address.
    pub fn last_error(&self, address: u8) -> u8 {
        self.remote_at(address)
            .map_or(ERR_NO_SLAVE, |r| r.last_error)
    }

    pub fn poll<T: Transport>(&mut self, now: u64, t: &mut T) {
        match self.state {
            MasterState::Sending { until } => {
                if now < until {
                    return;
                }
                self.state = MasterState::Waiting { since: now };
            }
            MasterState::Pause { until } => {
                if now >= until {
                    self.state = MasterState::Ready;
                }
                return;
            }
            MasterState::Connecting { since } => {
                self.poll_connecting(now, since, t);
                return;
            }
            _ => {}
        }
        if let MasterState::Waiting { since } = self.state {
            self.check_answer(now, since, t);
            if self.framing == Framing::Tcp {
                return;
            }
        }
        if self.state == MasterState::Ready {
            self.next_query(now, t);
        }
    }

    fn poll_connecting<T: Transport>(&mut self, now: u64, since: u64, t: &mut T) {
        let Some(tele) = &self.tele else {
            self.state = MasterState::Ready;
            return;
        };
        if now.saturating_sub(since) >= self.remotes[tele.remote].timeout_ms {
            self.on_timeout(now, t);
        } else if t.is_connected() {
            self.send_telegram(now, t);
        } else {
            t.connect();
        }
    }

    // ----- choosing the next request -----

    fn next_query<T: Transport>(&mut self, now: u64, t: &mut T) {
        let made = if self.remotes.iter().any(|r| r.has_write(now)) {
            self.create_write(now)
        } else {
            self.next_step(now) && self.create_read()
        };
        if !made {
            return;
        }
        if self.framing == Framing::Tcp && !t.is_connected() {
            t.connect();
            self.state = MasterState::Connecting { since: now };
            return;
        }
        self.send_telegram(now, t);
    }

    fn create_write(&mut self, now: u64) -> bool {
        let Some(r) = self.remotes.iter().position(|r| r.has_write(now)) else {
            return false;
        };
        let tables = &mut self.remotes[r].tables;
        let table = if tables.first_dirty(Table::Holding).is_some() {
            Table::Holding
        } else {
            Table::Coil
        };
        let start = tables.first_dirty(table).expect("a dirty cell was found");
        let mut count = 1;
        while count < MAX_REGS && tables.is_dirty(table, start + count) {
            count += 1;
        }
        let a = start as i32;
        let request = if table == Table::Holding {
            let values: Vec<i16> = (0..count)
                .map(|i| tables.read_int(table, a + i as i32))
                .collect();
            if count == 1 {
                Request::WriteRegister {
                    addr: start as u16,
                    value: values[0],
                }
            } else {
                Request::WriteRegisters {
                    start: start as u16,
                    values,
                }
            }
        } else {
            let values: Vec<bool> = (0..count)
                .map(|i| tables.read_bool(table, a + i as i32))
                .collect();
            if count == 1 {
                Request::WriteCoil {
                    addr: start as u16,
                    on: values[0],
                }
            } else {
                Request::WriteCoils {
                    start: start as u16,
                    values,
                }
            }
        };
        for i in 0..count {
            tables.clear_dirty(table, start + i);
        }
        self.tele = Some(Telegram {
            remote: r,
            request,
            start,
            count,
        });
        true
    }

    fn create_read(&mut self) -> bool {
        let Some(r) = self.current else {
            return false;
        };
        let table = self.cur_table;
        let size = self.remotes[r].tables.size(table);
        let start = self.cur_start;
        let count = size.saturating_sub(start).min(MAX_REGS);
        if count == 0 {
            return false;
        }
        self.cur_last = start + count - 1;
        let (s, c) = (start as u16, count as u16);
        let request = if table.is_bits() {
            Request::ReadBits {
                table,
                start: s,
                count: c,
            }
        } else {
            Request::ReadWords {
                table,
                start: s,
                count: c,
            }
        };
        self.tele = Some(Telegram {
            remote: r,
            request,
            start,
            count,
        });
        true
    }

    /// Moves the polling cursor to the next chunk; false when there is nothing to poll now.
    fn next_step(&mut self, now: u64) -> bool {
        if !self.remotes.iter().any(|r| r.is_ready(now)) {
            return false;
        }
        let Some(cur) = self.current else {
            let first = self
                .remotes
                .iter()
                .position(|r| r.is_ready(now))
                .expect("a ready slave exists");
            self.begin_slave(first);
            return true;
        };
        if !self.remotes[cur].is_ready(now) {
            return self.next_slave(now);
        }
        let next = self.cur_last + 1;
        if next < self.remotes[cur].tables.size(self.cur_table) {
            self.cur_start = next;
            return true;
        }
        let tables = &self.remotes[cur].tables;
        match self.cur_table.poll_next(|t| tables.has(t)) {
            Some(table) => {
                self.cur_table = table;
                self.cur_start = 0;
                true
            }
            None => self.next_slave(now),
        }
    }

    fn begin_slave(&mut self, index: usize) {
        self.current = Some(index);
        self.cur_table = self.remotes[index]
            .first_table()
            .expect("a ready slave has a table");
        self.cur_start = 0;
    }

    /// The slave's cycle is over: its period starts, the next ready slave (after it) is taken.
    fn next_slave(&mut self, now: u64) -> bool {
        let Some(cur) = self.current else {
            return false;
        };
        self.remotes[cur].last_request = now;
        self.remotes[cur].init = true;
        let next = (cur + 1..self.remotes.len()).find(|i| self.remotes[*i].is_ready(now));
        match next {
            Some(i) => {
                self.begin_slave(i);
                true
            }
            None => {
                self.current = None;
                false
            }
        }
    }

    // ----- sending and receiving -----

    fn send_telegram<T: Transport>(&mut self, now: u64, t: &mut T) {
        let Some(tele) = &self.tele else {
            self.state = MasterState::Ready;
            return;
        };
        let body = tele.request.encode(self.remotes[tele.remote].address);
        drain(t);
        self.rx.clear();
        match self.framing {
            Framing::Rtu => {
                let bytes = with_crc(&body);
                t.send(&bytes);
                self.state = MasterState::Sending {
                    until: now + frame_time_ms(bytes.len(), self.speed),
                };
            }
            Framing::Tcp => {
                self.txn = self.txn.wrapping_add(1);
                t.send(&mbap_encode(self.txn, 0, &body));
                self.state = MasterState::Waiting { since: now };
            }
        }
    }

    fn check_answer<T: Transport>(&mut self, now: u64, since: u64, t: &mut T) {
        let Some(tele) = &self.tele else {
            self.state = MasterState::Ready;
            return;
        };
        if now.saturating_sub(since) >= self.remotes[tele.remote].timeout_ms {
            self.on_timeout(now, t);
            return;
        }
        match self.framing {
            Framing::Rtu => {
                let mut chunk = Vec::new();
                t.receive(&mut chunk);
                for byte in chunk {
                    if self.rx.len() >= MAX_FRAME + 2 {
                        self.rx.clear();
                    }
                    self.rx.push(byte);
                    self.last_rx = now;
                    let size = response_frame_size(&self.rx);
                    if size != 0 && self.rx.len() == size {
                        let frame = core::mem::take(&mut self.rx);
                        self.on_rtu_frame(now, &frame);
                        return;
                    }
                }
                if !self.rx.is_empty() && now.saturating_sub(self.last_rx) >= RX_IDLE_MS {
                    self.rx.clear();
                }
            }
            Framing::Tcp => {
                t.receive(&mut self.rx);
                while let Some(frame) = mbap_take(&mut self.rx) {
                    // An answer to an older request is skipped, we keep waiting for ours.
                    if frame.txn != self.txn {
                        continue;
                    }
                    let error = if frame.proto != 0 {
                        Some(ERR_FRAME)
                    } else {
                        self.apply(&frame.body)
                    };
                    self.finish(now, error);
                    return;
                }
            }
        }
    }

    fn on_rtu_frame(&mut self, now: u64, frame: &[u8]) {
        let error = if crc_ok(frame) {
            self.apply(&frame[..frame.len() - 2])
        } else {
            Some(ERR_CRC)
        };
        self.finish(now, error);
    }

    /// Checks a reply (unit id + PDU) against the request in flight and stores its data.
    /// `Some(code)` is the error to remember (an exception code of the slave counts too).
    fn apply(&mut self, body: &[u8]) -> Option<u8> {
        let tele = self.tele.as_ref()?;
        let remote = &mut self.remotes[tele.remote];
        let function = tele.request.function();
        if body.len() < 2 || body[0] != remote.address {
            return Some(ERR_FRAME);
        }
        if body[1] & 0x80 != 0 {
            if body[1] & 0x7F != function || body.len() < 3 {
                return Some(ERR_FRAME);
            }
            remote.online = true;
            return Some(body[2]);
        }
        if body[1] != function {
            return Some(ERR_FRAME);
        }
        let table = tele.request.table();
        match &tele.request {
            Request::ReadBits { .. } => {
                let bytes = tele.count.div_ceil(8);
                if body.len() < 3 + bytes || (body[2] as usize) < bytes {
                    return Some(ERR_FRAME);
                }
                if let Some(t) = remote.tables.slot_mut(table) {
                    for i in 0..tele.count {
                        let on = body[3 + i / 8] >> (i % 8) & 1 != 0;
                        t.put((tele.start + i) as i32, i16::from(on));
                    }
                }
            }
            Request::ReadWords { .. } => {
                let bytes = tele.count * 2;
                if body.len() < 3 + bytes || (body[2] as usize) < bytes {
                    return Some(ERR_FRAME);
                }
                if let Some(t) = remote.tables.slot_mut(table) {
                    for i in 0..tele.count {
                        let v = i16::from_be_bytes([body[3 + i * 2], body[4 + i * 2]]);
                        t.put((tele.start + i) as i32, v);
                    }
                }
            }
            _ => {}
        }
        remote.online = true;
        None
    }

    fn finish(&mut self, now: u64, error: Option<u8>) {
        if let Some(tele) = &self.tele {
            self.remotes[tele.remote].last_error = error.unwrap_or(0);
        }
        self.state = match self.framing {
            Framing::Rtu => MasterState::Pause {
                until: now + self.pause_ms,
            },
            Framing::Tcp => MasterState::Ready,
        };
    }

    fn on_timeout<T: Transport>(&mut self, now: u64, t: &mut T) {
        let pause: u64 = self
            .remotes
            .iter()
            .map(|r| r.period_ms + r.timeout_ms)
            .sum();
        if let Some(tele) = &self.tele {
            let r = &mut self.remotes[tele.remote];
            r.last_error = ERR_TIMEOUT;
            r.online = false;
            r.pause_until = now + pause;
        }
        self.rx.clear();
        if self.framing == Framing::Tcp {
            t.close();
        }
        self.state = MasterState::Ready;
    }
}

// ---------------------------------------------------------------------------------------------
// Module import
// ---------------------------------------------------------------------------------------------

/// A role a board can take, the value of setting code 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    RtuMaster,
    RtuSlave,
    TcpMaster,
    TcpSlave,
}

/// UART settings of an RTU role (setting codes 1 to 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UartSettings {
    pub uart: i32,
    pub speed: u32,
    pub parity: i32,
    /// -1 means "not set".
    pub rx: i32,
    pub tx: i32,
    pub de: i32,
}

/// TCP settings of a TCP role (setting codes 1 to 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TcpSettings {
    /// 0 Wi-Fi, 1 Ethernet.
    pub interface: i32,
    pub port: u16,
    /// Server address of a TCP master (all zeros for a slave).
    pub server_ip: [u8; 4],
}

/// What the board glue needs to open a port for a committed role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoleSettings {
    pub role: Role,
    pub uart: Option<UartSettings>,
    pub tcp: Option<TcpSettings>,
}

/// The `modbus(op, a, b, c, d) -> i32` import of WASM modules (`h_modbus` of the C++ master).
///
/// - op 0: setting, `a` = code, `b` = value; code 0 clears the list, 63 commits it (once per role);
/// - op 1: read, `a` = slave address, `b` = table | kind << 4 | master << 8 | tcp << 9, `c` = register;
/// - op 2: write, as read, `d` = value (floats as their bits);
/// - op 3: status, `a >= 0` the slave answers (1/0), `a < 0` last error of our slave.
///
/// Setting codes: 0 role (1 RTU master, 2 RTU slave, 3 TCP master, 4 TCP slave), 1 uart (TCP:
/// interface), 2 speed (TCP: port), 3 parity (TCP master: server IP bits, first octet lowest),
/// 4 rx, 5 tx, 6 de, 7 count (slave: address), 8..11 slave tables (holding, input, coils,
/// discrete), 16 + 8 n + k master slave n: 0 address, 1 period, 2 timeout, 3..6 table sizes.
#[derive(Debug, Clone)]
pub struct ModbusHost {
    settings: [i32; 64],
    rtu_master: Option<Master>,
    rtu_slave: Option<RtuSlave>,
    tcp_master: Option<Master>,
    tcp_slave: Option<TcpSlave>,
    roles: Vec<RoleSettings>,
    fresh: Vec<RoleSettings>,
}

impl Default for ModbusHost {
    fn default() -> Self {
        Self::new()
    }
}

impl ModbusHost {
    pub fn new() -> Self {
        ModbusHost {
            settings: [0; 64],
            rtu_master: None,
            rtu_slave: None,
            tcp_master: None,
            tcp_slave: None,
            roles: Vec::new(),
            fresh: Vec::new(),
        }
    }

    /// Every committed role.
    pub fn roles(&self) -> &[RoleSettings] {
        &self.roles
    }

    /// Roles committed since the last call; the glue opens their UART or socket.
    pub fn take_new_roles(&mut self) -> Vec<RoleSettings> {
        core::mem::take(&mut self.fresh)
    }

    pub fn has_rtu_master(&self) -> bool {
        self.rtu_master.is_some()
    }

    pub fn has_rtu_slave(&self) -> bool {
        self.rtu_slave.is_some()
    }

    pub fn has_tcp_master(&self) -> bool {
        self.tcp_master.is_some()
    }

    pub fn has_tcp_slave(&self) -> bool {
        self.tcp_slave.is_some()
    }

    pub fn poll_rtu_master<T: Transport>(&mut self, now_ms: u64, t: &mut T) {
        if let Some(m) = &mut self.rtu_master {
            m.poll(now_ms, t);
        }
    }

    pub fn poll_rtu_slave<T: Transport>(&mut self, now_ms: u64, t: &mut T) {
        if let Some(s) = &mut self.rtu_slave {
            s.poll(now_ms, t);
        }
    }

    pub fn poll_tcp_master<T: Transport>(&mut self, now_ms: u64, t: &mut T) {
        if let Some(m) = &mut self.tcp_master {
            m.poll(now_ms, t);
        }
    }

    pub fn poll_tcp_slave<T: Transport>(&mut self, now_ms: u64, t: &mut T) {
        if let Some(s) = &mut self.tcp_slave {
            s.poll(now_ms, t);
        }
    }

    /// One call of the import.
    pub fn call(&mut self, op: i32, a: i32, b: i32, c: i32, d: i32) -> i32 {
        let table = Table::from_selector(b);
        let kind = b >> 4 & 15;
        let master = b >> 8 & 1 != 0;
        let tcp = b >> 9 & 1 != 0;
        match op {
            0 => {
                if a == 0 {
                    self.settings = [0; 64];
                }
                if a == 63 {
                    self.commit();
                } else if (0..63).contains(&a) {
                    self.settings[a as usize] = b;
                }
                0
            }
            1 => self.tables(tcp, master, a).map_or(0, |t| match kind {
                1 => t.read_float(table, c).to_bits() as i32,
                2 => i32::from(t.read_bool(table, c)),
                _ => i32::from(t.read_int(table, c)),
            }),
            2 => {
                if let Some(t) = self.tables(tcp, master, a) {
                    match kind {
                        1 => t.save_float(table, c, f32::from_bits(d as u32)),
                        2 => t.save_bool(table, c, d != 0),
                        _ => t.save_int(table, c, d as i16),
                    }
                }
                0
            }
            3 => self.status(tcp, a),
            _ => 0,
        }
    }

    /// The tables a read or write goes to. Flprog: C++ sent a TCP call to the TCP master whenever
    /// one existed; here the master bit decides, as for RTU.
    fn tables(&mut self, tcp: bool, master: bool, unit: i32) -> Option<&mut Tables> {
        let address = unit as u8;
        match (tcp, master) {
            (false, true) => self.rtu_master.as_mut()?.tables_mut(address),
            (false, false) => self.rtu_slave.as_mut().map(|s| &mut s.unit.tables),
            (true, true) => self.tcp_master.as_mut()?.tables_mut(address),
            (true, false) => self.tcp_slave.as_mut().map(|s| &mut s.unit.tables),
        }
    }

    fn status(&self, tcp: bool, a: i32) -> i32 {
        if a >= 0 {
            let master = if tcp {
                &self.tcp_master
            } else {
                &self.rtu_master
            };
            i32::from(master.as_ref().is_some_and(|m| m.slave_status(a as u8)))
        } else if tcp {
            self.tcp_slave
                .as_ref()
                .map_or(0, |s| i32::from(s.unit.last_error()))
        } else {
            self.rtu_slave
                .as_ref()
                .map_or(0, |s| i32::from(s.unit.last_error()))
        }
    }

    fn remotes(&self) -> Vec<RemoteConfig> {
        let n = self.settings[7].clamp(1, 4) as usize;
        (0..n)
            .map(|i| {
                let b = 16 + 8 * i;
                let size = |k: usize| self.settings[b + 3 + k].max(0) as usize;
                RemoteConfig {
                    address: self.settings[b] as u8,
                    period_ms: self.settings[b + 1] as u32,
                    timeout_ms: self.settings[b + 2] as u32,
                    sizes: [size(0), size(1), size(2), size(3)],
                }
            })
            .collect()
    }

    fn configure(&self, tables: &mut Tables) {
        for k in 0..4 {
            let size = self.settings[8 + k];
            if size > 0 {
                tables.configure(Table::from_selector(k as i32), size as usize);
            }
        }
    }

    /// Setting code 63: creates the role once (a second commit of the same role is ignored).
    fn commit(&mut self) {
        let s = self.settings;
        let uart = UartSettings {
            uart: s[1],
            speed: if s[2] > 0 { s[2] as u32 } else { 9600 },
            parity: s[3],
            rx: s[4],
            tx: s[5],
            de: s[6],
        };
        let tcp = TcpSettings {
            interface: s[1],
            port: s[2] as u16,
            server_ip: s[3].to_le_bytes(),
        };
        let role = match s[0] {
            1 if self.rtu_master.is_none() => {
                self.rtu_master = Some(Master::new(Framing::Rtu, uart.speed, &self.remotes()));
                RoleSettings {
                    role: Role::RtuMaster,
                    uart: Some(uart),
                    tcp: None,
                }
            }
            2 if self.rtu_slave.is_none() => {
                let mut slave = RtuSlave::new(s[7] as u8, uart.speed);
                self.configure(&mut slave.unit.tables);
                self.rtu_slave = Some(slave);
                RoleSettings {
                    role: Role::RtuSlave,
                    uart: Some(uart),
                    tcp: None,
                }
            }
            3 if self.tcp_master.is_none() => {
                self.tcp_master = Some(Master::new(Framing::Tcp, 0, &self.remotes()));
                RoleSettings {
                    role: Role::TcpMaster,
                    uart: None,
                    tcp: Some(tcp),
                }
            }
            4 if self.tcp_slave.is_none() => {
                let mut slave = TcpSlave::new(s[7] as u8);
                self.configure(&mut slave.unit.tables);
                self.tcp_slave = Some(slave);
                RoleSettings {
                    role: Role::TcpSlave,
                    uart: None,
                    tcp: Some(TcpSettings {
                        server_ip: [0; 4],
                        ..tcp
                    }),
                }
            }
            _ => return,
        };
        self.roles.push(role);
        self.fresh.push(role);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    type Pipe = Rc<RefCell<Vec<u8>>>;

    /// One end of an in-memory wire.
    struct End {
        tx: Pipe,
        rx: Pipe,
        connected: bool,
        connects: u32,
        closes: u32,
    }

    impl Transport for End {
        fn send(&mut self, bytes: &[u8]) {
            self.tx.borrow_mut().extend_from_slice(bytes);
        }
        fn receive(&mut self, buf: &mut Vec<u8>) {
            buf.append(&mut self.rx.borrow_mut());
        }
        fn is_connected(&mut self) -> bool {
            self.connected
        }
        fn connect(&mut self) {
            self.connects += 1;
        }
        fn close(&mut self) {
            self.closes += 1;
        }
    }

    fn wire() -> (End, End) {
        let (a, b): (Pipe, Pipe) = Default::default();
        let end = |tx: &Pipe, rx: &Pipe| End {
            tx: tx.clone(),
            rx: rx.clone(),
            connected: true,
            connects: 0,
            closes: 0,
        };
        (end(&a, &b), end(&b, &a))
    }

    fn cfg(address: u8, sizes: [usize; 4]) -> RemoteConfig {
        RemoteConfig {
            address,
            period_ms: 100,
            timeout_ms: 200,
            sizes,
        }
    }

    fn slave_tables(slave: &mut Tables) {
        slave.configure(Table::Holding, 10);
        slave.configure(Table::Input, 10);
        slave.configure(Table::Coil, 10);
        slave.configure(Table::Discrete, 10);
    }

    #[test]
    fn crc_of_known_frames() {
        assert_eq!(
            with_crc(&[0x01, 0x03, 0x00, 0x00, 0x00, 0x0A]),
            [0x01, 0x03, 0x00, 0x00, 0x00, 0x0A, 0xC5, 0xCD]
        );
        assert!(crc_ok(&[0x01, 0x03, 0x00, 0x00, 0x00, 0x0A, 0xC5, 0xCD]));
        assert!(!crc_ok(&[0x01, 0x03, 0x00, 0x00, 0x00, 0x0A, 0xC5, 0xCE]));
        assert_eq!(t35_us(9600), 4010);
        assert_eq!(t35_us(115_200), 1750);
    }

    #[test]
    fn frame_sizes_follow_the_library() {
        assert_eq!(request_frame_size(&[1, 3]), 8);
        assert_eq!(request_frame_size(&[1, 0x83]), 5);
        assert_eq!(request_frame_size(&[1, 7]), 0);
        assert_eq!(request_frame_size(&[1, 16, 0, 0, 0, 2]), 0);
        assert_eq!(request_frame_size(&[1, 16, 0, 0, 0, 2, 4]), 13);
        assert_eq!(response_frame_size(&[1, 3, 4]), 9);
        assert_eq!(response_frame_size(&[1, 16]), 8);
    }

    #[test]
    fn float_word_order_is_flprog_abcd() {
        let mut t = Tables::new(false);
        t.configure(Table::Holding, 4);
        t.save_float(Table::Holding, 0, 1.5);
        t.save_float(Table::Holding, 2, -273.15);
        // 1.5f = 0x3FC00000, -273.15f = 0xC3889333: the high word goes to the lower register.
        assert_eq!(t.read_int(Table::Holding, 0) as u16, 0x3FC0);
        assert_eq!(t.read_int(Table::Holding, 1) as u16, 0x0000);
        assert_eq!(t.read_int(Table::Holding, 2) as u16, 0xC388);
        assert_eq!(t.read_int(Table::Holding, 3) as u16, 0x9333);
        assert_eq!(t.read_float(Table::Holding, 0), 1.5);
        assert_eq!(t.read_float(Table::Holding, 2), -273.15);
        // The second register is missing: nothing sensible to read.
        assert_eq!(t.read_float(Table::Holding, 3), 0.0);
    }

    #[test]
    fn typed_access_rules() {
        let mut t = Tables::new(true);
        t.configure(Table::Input, 2);
        t.configure(Table::Coil, 2);
        // A master may not save into an input table; a bool does not go into registers.
        t.save_int(Table::Input, 0, 5);
        assert_eq!(t.read_int(Table::Input, 0), 0);
        t.save_bool(Table::Coil, 1, true);
        assert!(t.read_bool(Table::Coil, 1));
        assert_eq!(t.read_int(Table::Coil, 1), 0);
        assert!(!t.read_bool(Table::Input, 0));
        assert_eq!(t.first_dirty(Table::Coil), Some(1));
    }

    #[test]
    fn slave_replies_and_exceptions() {
        let mut slave = RtuSlave::new(5, 9600);
        slave_tables(&mut slave.unit.tables);
        slave.unit.tables.save_int(Table::Holding, 2, 0x1234);
        let (mut host, mut line) = wire();
        let ask = |body: &[u8], slave: &mut RtuSlave, host: &mut End, line: &mut End, t0: u64| {
            host.tx.borrow_mut().extend(with_crc(body));
            for t in t0..t0 + 40 {
                slave.poll(t, line);
            }
            let mut out = Vec::new();
            host.receive(&mut out);
            out
        };
        // Read holding register 2.
        let r = ask(&[5, 3, 0, 2, 0, 1], &mut slave, &mut line, &mut host, 0);
        assert_eq!(r, with_crc(&[5, 3, 2, 0x12, 0x34]));
        assert_eq!(slave.unit.last_error(), 0);
        // Out of range address: exception 2.
        let r = ask(&[5, 3, 0, 9, 0, 2], &mut slave, &mut line, &mut host, 100);
        assert_eq!(r, with_crc(&[5, 0x83, 2]));
        assert_eq!(slave.unit.last_error(), 2);
        // A table that is not configured is an illegal function: exception 1.
        let mut bare = RtuSlave::new(5, 9600);
        bare.unit.tables.configure(Table::Holding, 4);
        let r = ask(&[5, 4, 0, 0, 0, 1], &mut bare, &mut line, &mut host, 200);
        assert_eq!(r, with_crc(&[5, 0x84, 1]));
        assert_eq!(bare.unit.last_error(), 1);
        // Another address and the broadcast get no answer; a write to broadcast still happens.
        let r = ask(&[6, 3, 0, 2, 0, 1], &mut slave, &mut line, &mut host, 300);
        assert!(r.is_empty());
        let r = ask(&[0, 6, 0, 1, 0, 7], &mut slave, &mut line, &mut host, 400);
        assert!(r.is_empty());
        assert_eq!(slave.unit.tables.read_int(Table::Holding, 1), 7);
        // A broken CRC is remembered as 252 and not answered.
        let mut bad = with_crc(&[5, 3, 0, 2, 0, 1]);
        bad[7] ^= 1;
        host.tx.borrow_mut().extend(bad);
        for t in 500..540 {
            slave.poll(t, &mut line);
        }
        assert!(host.rx.borrow().is_empty());
        assert_eq!(slave.unit.last_error(), ERR_CRC);
    }

    /// Runs master and slave for `ms` milliseconds from `from`.
    fn run(
        master: &mut Master,
        slave: &mut RtuSlave,
        m: &mut End,
        s: &mut End,
        from: u64,
        ms: u64,
    ) {
        for t in from..from + ms {
            master.poll(t, m);
            slave.poll(t, s);
        }
    }

    #[test]
    fn rtu_master_reads_all_tables_and_writes_changes() {
        let mut master = Master::new(Framing::Rtu, 9600, &[cfg(5, [30, 3, 12, 2])]);
        let mut slave = RtuSlave::new(5, 9600);
        slave_tables(&mut slave.unit.tables);
        slave.unit.tables.configure(Table::Holding, 30);
        slave.unit.tables.configure(Table::Coil, 12);
        slave.unit.tables.configure(Table::Discrete, 2);
        slave.unit.tables.configure(Table::Input, 3);
        slave.unit.tables.save_int(Table::Holding, 0, -5);
        slave.unit.tables.save_int(Table::Holding, 29, 300);
        slave.unit.tables.save_float(Table::Holding, 10, 21.5);
        slave.unit.tables.save_int(Table::Input, 2, 1234);
        slave.unit.tables.save_bool(Table::Coil, 11, true);
        slave.unit.tables.save_bool(Table::Discrete, 1, true);
        let (mut m, mut s) = wire();
        run(&mut master, &mut slave, &mut m, &mut s, 0, 1500);
        let t = master.tables(5).unwrap();
        assert_eq!(t.read_int(Table::Holding, 0), -5);
        assert_eq!(
            t.read_int(Table::Holding, 29),
            300,
            "chunked read of 30 registers"
        );
        assert_eq!(t.read_float(Table::Holding, 10), 21.5);
        assert_eq!(t.read_int(Table::Input, 2), 1234);
        assert!(t.read_bool(Table::Coil, 11));
        assert!(t.read_bool(Table::Discrete, 1));
        assert!(!t.read_bool(Table::Discrete, 0));
        assert!(master.slave_status(5));
        assert_eq!(master.last_error(5), 0);
        assert_eq!(master.last_error(9), ERR_NO_SLAVE);

        // The program writes: two registers (function 16), one register (6), a coil (5), coils (15).
        let t = master.tables_mut(5).unwrap();
        t.save_float(Table::Holding, 4, -2.25);
        t.save_int(Table::Holding, 7, 77);
        t.save_bool(Table::Coil, 3, true);
        t.save_bool(Table::Coil, 8, true);
        t.save_bool(Table::Coil, 9, true);
        run(&mut master, &mut slave, &mut m, &mut s, 1500, 1500);
        let st = &slave.unit.tables;
        assert_eq!(st.read_float(Table::Holding, 4), -2.25);
        assert_eq!(st.read_int(Table::Holding, 7), 77);
        assert!(st.read_bool(Table::Coil, 3));
        assert!(st.read_bool(Table::Coil, 8));
        assert!(st.read_bool(Table::Coil, 9));
        assert!(!st.read_bool(Table::Coil, 4));
        // A saved value that does not change is not sent again.
        assert_eq!(master.tables(5).unwrap().first_dirty(Table::Holding), None);
    }

    #[test]
    fn rtu_timeout_and_recovery() {
        let mut master = Master::new(Framing::Rtu, 9600, &[cfg(5, [4, 0, 0, 0])]);
        let mut slave = RtuSlave::new(5, 9600);
        slave.unit.tables.configure(Table::Holding, 4);
        slave.unit.tables.save_int(Table::Holding, 1, 42);
        let (mut m, mut s) = wire();
        // The slave is silent: nobody polls it.
        for t in 0..400 {
            master.poll(t, &mut m);
        }
        assert!(!master.slave_status(5));
        assert_eq!(master.last_error(5), ERR_TIMEOUT);
        // After the pause (period + timeout) the master tries again and the slave answers.
        run(&mut master, &mut slave, &mut m, &mut s, 400, 1500);
        assert!(master.slave_status(5));
        assert_eq!(master.last_error(5), 0);
        assert_eq!(master.tables(5).unwrap().read_int(Table::Holding, 1), 42);
    }

    #[test]
    fn rtu_master_with_two_slaves_and_exception() {
        let mut master = Master::new(
            Framing::Rtu,
            19_200,
            &[cfg(1, [2, 0, 0, 0]), cfg(2, [0, 2, 0, 0])],
        );
        let mut one = RtuSlave::new(1, 19_200);
        one.unit.tables.configure(Table::Holding, 2);
        one.unit.tables.save_int(Table::Holding, 0, 11);
        let mut two = RtuSlave::new(2, 19_200);
        // Slave 2 has fewer input registers than the master asks for: exception 2.
        two.unit.tables.configure(Table::Input, 1);
        // One shared line: what the master sends reaches both slaves, both answer into the master.
        let (mut m, mut s1) = wire();
        let mut s2 = End {
            tx: s1.tx.clone(),
            rx: Default::default(),
            connected: true,
            connects: 0,
            closes: 0,
        };
        for t in 0..2000 {
            master.poll(t, &mut m);
            let sent = std::mem::take(&mut *m.tx.borrow_mut());
            s1.rx.borrow_mut().extend_from_slice(&sent);
            s2.rx.borrow_mut().extend_from_slice(&sent);
            one.poll(t, &mut s1);
            two.poll(t, &mut s2);
        }
        assert_eq!(master.tables(1).unwrap().read_int(Table::Holding, 0), 11);
        assert_eq!(master.last_error(2), 2);
        assert!(master.slave_status(2), "an exception is still an answer");
        assert_eq!(master.last_error(1), 0);
    }

    #[test]
    fn mbap_framing_round_trip() {
        let body = Request::ReadWords {
            table: Table::Holding,
            start: 2,
            count: 3,
        }
        .encode(7);
        let frame = mbap_encode(0x0102, 0, &body);
        assert_eq!(&frame[..7], &[1, 2, 0, 0, 0, 6, 7]);
        let mut stream = frame.clone();
        stream.extend(&frame[..4]);
        let got = mbap_take(&mut stream).unwrap();
        assert_eq!(
            got,
            MbapFrame {
                txn: 0x0102,
                proto: 0,
                body
            }
        );
        assert_eq!(stream.len(), 4, "the next frame stays");
        assert!(mbap_take(&mut stream).is_none());
        let mut junk = vec![0, 1, 0, 0, 0, 0, 9];
        assert!(mbap_take(&mut junk).is_none());
        assert!(junk.is_empty(), "a zero length resets the stream");
    }

    #[test]
    fn request_encode_parse_round_trip() {
        let all = [
            Request::ReadBits {
                table: Table::Coil,
                start: 1,
                count: 9,
            },
            Request::ReadBits {
                table: Table::Discrete,
                start: 0,
                count: 1,
            },
            Request::ReadWords {
                table: Table::Holding,
                start: 3,
                count: 2,
            },
            Request::ReadWords {
                table: Table::Input,
                start: 0,
                count: 5,
            },
            Request::WriteCoil { addr: 4, on: true },
            Request::WriteRegister { addr: 9, value: -3 },
            Request::WriteCoils {
                start: 2,
                values: vec![true, false, true, true, false, false, false, false, true],
            },
            Request::WriteRegisters {
                start: 1,
                values: vec![1, -2, 300],
            },
        ];
        for r in all {
            assert_eq!(Request::parse(&r.encode(3)), Ok(r));
        }
        assert_eq!(Request::parse(&[1, 9, 0, 0, 0, 0]), Err(1));
        assert_eq!(Request::parse(&[1, 3, 0]), Err(3));
    }

    #[test]
    fn tcp_master_and_slave_over_the_wire() {
        let mut master = Master::new(Framing::Tcp, 0, &[cfg(9, [4, 4, 2, 2])]);
        let mut slave = TcpSlave::new(9);
        slave_tables(&mut slave.unit.tables);
        slave.unit.tables.save_float(Table::Input, 0, 3.5);
        slave.unit.tables.save_bool(Table::Discrete, 1, true);
        let (mut m, mut s) = wire();
        m.connected = false;
        for t in 0..50 {
            master.poll(t, &mut m);
            slave.poll(t, &mut s);
        }
        assert!(m.connects > 0, "the master asks the glue to connect");
        assert_eq!(master.tables(9).unwrap().read_float(Table::Input, 0), 0.0);
        m.connected = true;
        let t = master.tables_mut(9).unwrap();
        t.save_int(Table::Holding, 1, -9);
        for t in 50..600 {
            master.poll(t, &mut m);
            slave.poll(t, &mut s);
        }
        let t = master.tables(9).unwrap();
        assert_eq!(t.read_float(Table::Input, 0), 3.5);
        assert!(t.read_bool(Table::Discrete, 1));
        assert_eq!(slave.unit.tables.read_int(Table::Holding, 1), -9);
        assert!(master.slave_status(9));
        assert_eq!(slave.unit.last_error(), 0);
    }

    #[test]
    fn tcp_timeout_closes_the_connection() {
        let mut master = Master::new(Framing::Tcp, 0, &[cfg(9, [2, 0, 0, 0])]);
        let (mut m, _s) = wire();
        for t in 0..500 {
            master.poll(t, &mut m);
        }
        assert!(!master.slave_status(9));
        assert_eq!(master.last_error(9), ERR_TIMEOUT);
        assert!(m.closes >= 1);
    }

    #[test]
    fn tcp_slave_checks_the_header_and_echoes_the_transaction() {
        let mut slave = TcpSlave::new(9);
        slave.unit.tables.configure(Table::Holding, 2);
        slave.unit.tables.save_int(Table::Holding, 0, 5);
        let (host, mut line) = wire();
        let body = [9, 3, 0, 0, 0, 1];
        host.tx.borrow_mut().extend(mbap_encode(0xABCD, 0, &body));
        slave.poll(0, &mut line);
        assert_eq!(
            host.rx.borrow().clone(),
            mbap_encode(0xABCD, 0, &[9, 3, 2, 0, 5])
        );
        host.rx.borrow_mut().clear();
        host.tx.borrow_mut().extend(mbap_encode(1, 7, &body));
        slave.poll(1, &mut line);
        assert!(host.rx.borrow().is_empty());
        assert_eq!(slave.unit.last_error(), ERR_FRAME);
        host.tx
            .borrow_mut()
            .extend(mbap_encode(2, 0, &[9, 3, 0, 5, 0, 1]));
        slave.poll(2, &mut line);
        assert_eq!(host.rx.borrow().clone(), mbap_encode(2, 0, &[9, 0x83, 2]));
        assert_eq!(slave.unit.last_error(), 2);
    }

    fn set(host: &mut ModbusHost, code: i32, value: i32) {
        host.call(0, code, value, 0, 0);
    }

    #[test]
    fn host_settings_commit_read_write_status() {
        let mut host = ModbusHost::new();
        // Before the commit every read is 0.
        assert_eq!(host.call(1, 0, 0, 0, 0), 0);
        // An RTU slave: address 3, 8 holding, 4 input, 8 coils.
        set(&mut host, 0, 2);
        set(&mut host, 1, 1);
        set(&mut host, 2, 19_200);
        set(&mut host, 3, 0);
        set(&mut host, 4, 16);
        set(&mut host, 5, 17);
        set(&mut host, 6, -1);
        set(&mut host, 7, 3);
        set(&mut host, 8, 8);
        set(&mut host, 9, 4);
        set(&mut host, 10, 8);
        set(&mut host, 63, 0);
        assert!(host.has_rtu_slave() && !host.has_rtu_master());
        let new = host.take_new_roles();
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].role, Role::RtuSlave);
        let u = new[0].uart.unwrap();
        assert_eq!((u.uart, u.speed, u.rx, u.tx, u.de), (1, 19_200, 16, 17, -1));
        assert!(host.take_new_roles().is_empty());

        let holding = 0;
        let float = 1 << 4;
        let boolean = 2 << 4;
        let coil = 2;
        // int16 is sign-extended.
        host.call(2, 0, holding, 1, -2);
        assert_eq!(host.call(1, 0, holding, 1, 0), -2);
        host.call(2, 0, holding, 2, 0xFFFF);
        assert_eq!(host.call(1, 0, holding, 2, 0), -1);
        // A float travels as its bits through two registers.
        host.call(2, 0, holding | float, 4, 1.5f32.to_bits() as i32);
        assert_eq!(
            host.call(1, 0, holding | float, 4, 0),
            1.5f32.to_bits() as i32
        );
        assert_eq!(host.call(1, 0, holding, 4, 0), 0x3FC0);
        host.call(2, 0, holding | float, 6, (-273.15f32).to_bits() as i32);
        assert_eq!(
            f32::from_bits(host.call(1, 0, holding | float, 6, 0) as u32),
            -273.15
        );
        // A bool.
        host.call(2, 0, coil | boolean, 5, 7);
        assert_eq!(host.call(1, 0, coil | boolean, 5, 0), 1);
        host.call(2, 0, coil | boolean, 5, 0);
        assert_eq!(host.call(1, 0, coil | boolean, 5, 0), 0);
        // Out of range reads are 0.
        assert_eq!(host.call(1, 0, holding, 100, 0), 0);
        // Status of the own slave: no error yet.
        assert_eq!(host.call(3, -1, 0, 0, 0), 0);
        // A second commit of the same role changes nothing.
        set(&mut host, 0, 2);
        set(&mut host, 7, 4);
        set(&mut host, 63, 0);
        assert_eq!(host.roles().len(), 1);
    }

    #[test]
    fn host_master_to_host_slave() {
        let mut master = ModbusHost::new();
        let mut slave = ModbusHost::new();
        // Master of two slaves (the second does not exist), poll 50 ms, timeout 100 ms.
        set(&mut master, 0, 1);
        set(&mut master, 2, 9600);
        set(&mut master, 7, 2);
        for (n, addr) in [(0, 5), (1, 6)] {
            let b = 16 + 8 * n;
            set(&mut master, b, addr);
            set(&mut master, b + 1, 50);
            set(&mut master, b + 2, 100);
            set(&mut master, b + 3, 4);
            set(&mut master, b + 4, 4);
            set(&mut master, b + 5, 4);
            set(&mut master, b + 6, 4);
        }
        set(&mut master, 63, 0);
        set(&mut slave, 0, 2);
        set(&mut slave, 2, 9600);
        set(&mut slave, 7, 5);
        for k in 8..12 {
            set(&mut slave, k, 4);
        }
        set(&mut slave, 63, 0);

        let master_bit = 1 << 8;
        slave.call(2, 0, 1 | 1 << 4, 0, 2.5f32.to_bits() as i32); // input register 0..1 as float
        slave.call(2, 0, 0, 3, 99); // holding 3
        slave.call(2, 0, 3 | 2 << 4, 2, 1); // discrete 2
        let (mut m, mut s) = wire();
        for t in 0..1500 {
            master.poll_rtu_master(t, &mut m);
            slave.poll_rtu_slave(t, &mut s);
        }
        assert_eq!(
            f32::from_bits(master.call(1, 5, 1 | 1 << 4 | master_bit, 0, 0) as u32),
            2.5
        );
        assert_eq!(master.call(1, 5, master_bit, 3, 0), 99);
        assert_eq!(master.call(1, 5, 3 | 2 << 4 | master_bit, 2, 0), 1);
        assert_eq!(master.call(3, 5, 0, 0, 0), 1);
        assert_eq!(
            master.call(3, 6, 0, 0, 0),
            0,
            "the missing slave is offline"
        );
        assert_eq!(
            master.call(3, 7, 0, 0, 0),
            0,
            "an unknown slave is not online"
        );
        // The master writes a holding register and a float; the slave sees them.
        master.call(2, 5, master_bit, 1, -400);
        master.call(2, 5, master_bit | 1 << 4, 2, 0.25f32.to_bits() as i32);
        for t in 1500..3000 {
            master.poll_rtu_master(t, &mut m);
            slave.poll_rtu_slave(t, &mut s);
        }
        assert_eq!(slave.call(1, 0, 0, 1, 0), -400);
        assert_eq!(f32::from_bits(slave.call(1, 0, 1 << 4, 2, 0) as u32), 0.25);
        assert_eq!(slave.call(3, -1, 0, 0, 0), 0);
    }

    #[test]
    fn host_tcp_roles_report_their_settings() {
        let mut host = ModbusHost::new();
        set(&mut host, 0, 3);
        set(&mut host, 1, 1);
        set(&mut host, 2, 502);
        set(&mut host, 3, i32::from_le_bytes([192, 168, 1, 10]));
        set(&mut host, 7, 1);
        set(&mut host, 16, 1);
        set(&mut host, 17, 100);
        set(&mut host, 18, 500);
        set(&mut host, 19, 2);
        set(&mut host, 63, 0);
        let roles = host.take_new_roles();
        assert_eq!(roles.len(), 1);
        assert_eq!(roles[0].role, Role::TcpMaster);
        assert_eq!(
            roles[0].tcp,
            Some(TcpSettings {
                interface: 1,
                port: 502,
                server_ip: [192, 168, 1, 10]
            })
        );
        assert!(host.has_tcp_master());
        // TCP calls go to the TCP master (bit 9).
        host.call(2, 1, 1 << 9 | 1 << 8, 0, 7);
        assert_eq!(host.call(1, 1, 1 << 9 | 1 << 8, 0, 0), 7);
        assert_eq!(
            host.call(1, 1, 1 << 8, 0, 0),
            0,
            "the RTU master does not exist"
        );
        assert_eq!(host.call(3, 1, 1 << 9, 0, 0), 1);
    }
}
