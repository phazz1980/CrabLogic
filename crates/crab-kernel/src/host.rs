//! Host functions: what a module's `crab.*` imports do.
//!
//! [`ModuleHost`] implements the board-independent part itself (values, tags, the bus, log, text,
//! math, string tags) and takes the output leases; the hardware imports go to [`Devices`], which
//! the board (or the PC stub) implements.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::abi::id;
use crate::lease::{Fault, Hal, Leases, PinUse};

pub const MAX_LOG_LINES: usize = 200;
pub const MAX_MESSAGE: usize = 4096;
pub const MAX_TOPICS: usize = 32;
/// Output events kept between takes: the C++ master's ring of 32 holds 31 (one cell tells full
/// from empty).
pub const MAX_EVENTS: usize = 31;

/// A value crossing the ABI.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Val {
    I32(i32),
    F32(f32),
}

impl Val {
    /// The value as `i32` (the bits of an `f32`).
    pub fn i(self) -> i32 {
        match self {
            Val::I32(v) => v,
            Val::F32(v) => v.to_bits() as i32,
        }
    }

    /// The value as `f32` (the bits of an `i32`).
    pub fn f(self) -> f32 {
        match self {
            Val::I32(v) => f32::from_bits(v as u32),
            Val::F32(v) => v,
        }
    }
}

/// One 4-byte value slot: the bits of an `i32` (`bool`, `int`) or an `f32` (`float`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Word(pub u32);

impl Word {
    pub fn int(v: i32) -> Self {
        Self(v as u32)
    }
    pub fn float(v: f32) -> Self {
        Self(v.to_bits())
    }
    pub fn as_int(self) -> i32 {
        self.0 as i32
    }
    pub fn as_float(self) -> f32 {
        f32::from_bits(self.0)
    }
}

/// The linear memory of the calling module (none if it exports no `memory`).
pub struct Mem<'a> {
    data: Option<&'a mut [u8]>,
}

impl<'a> Mem<'a> {
    pub fn new(data: Option<&'a mut [u8]>) -> Self {
        Self { data }
    }

    /// The whole memory.
    pub fn bytes(&mut self) -> Option<&mut [u8]> {
        self.data.as_deref_mut()
    }

    fn range(&self, ptr: i32, len: usize) -> Option<core::ops::Range<usize>> {
        let start = usize::try_from(ptr).ok()?;
        let end = start.checked_add(len)?;
        (end <= self.data.as_ref()?.len()).then_some(start..end)
    }

    /// Reads at most `max` bytes; `None` if the range is outside the memory.
    pub fn read(&self, ptr: i32, len: i32, max: usize) -> Option<Vec<u8>> {
        let r = self.range(ptr, (len.max(0) as usize).min(max))?;
        Some(self.data.as_ref()?[r].to_vec())
    }

    /// Writes `bytes`; `false` if the range is outside the memory.
    pub fn write(&mut self, ptr: i32, bytes: &[u8]) -> bool {
        let Some(r) = self.range(ptr, bytes.len()) else {
            return false;
        };
        match self.data.as_deref_mut() {
            Some(d) => {
                d[r].copy_from_slice(bytes);
                true
            }
            None => false,
        }
    }
}

/// What an engine calls when a module calls an import.
pub trait Host {
    /// `import` is the position in [`crate::abi::IMPORTS`]; `args` match its parameters. Returns
    /// the result for imports that have one.
    fn call(&mut self, import: usize, args: &[Val], mem: Mem<'_>) -> Option<Val>;
}

/// The hardware of a board as modules see it.
pub trait Devices {
    type Hal: Hal;

    /// The handle leases use to put outputs into their safe state.
    fn hal(&self) -> Self::Hal;

    /// A hardware import ([`crate::abi::Perm::Allow`]) the kernel does not handle itself.
    fn call(&mut self, import: usize, args: &[Val], mem: Mem<'_>) -> Option<Val>;

    /// Time passes before a step of `dt_ms` milliseconds.
    fn tick(&mut self, _dt_ms: u32) {}
}

/// A 0/1 output that changed, for the debugger's timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputEvent {
    pub slot: u8,
    pub value: u8,
    /// Board time of the step that reported it.
    pub ms: u32,
}

/// The state the host keeps for one module.
pub struct ModuleHost<D: Devices> {
    pub inputs: Vec<Word>,
    pub params: Vec<Word>,
    pub outputs: Vec<Word>,
    pub tags: BTreeMap<i32, Word>,
    pub text_tags: BTreeMap<i32, Vec<u8>>,
    /// Topics the module subscribed to, in order.
    pub subscriptions: Vec<String>,
    /// Messages published since the last take.
    pub published: Vec<(String, Vec<u8>)>,
    /// Lines written with `log` since the last take.
    pub log: Vec<String>,
    /// Outputs the module drives.
    pub leases: Leases<D::Hal>,
    /// Time that passed while a `step` was suspended; the next `step` gets it on top of its own.
    pub missed_ms: u32,
    /// Board time at the start of the current step; whoever runs the module sets it before the step.
    pub now_ms: u32,
    /// Forced value per output slot (the debugger overrides what the module reports).
    forced: Vec<Option<Word>>,
    events: VecDeque<OutputEvent>,
    lost: u32,
    pub devices: D,
}

impl<D: Devices> ModuleHost<D> {
    pub fn new(devices: D, n_outputs: usize) -> Self {
        Self {
            inputs: Vec::new(),
            params: Vec::new(),
            outputs: vec![Word::default(); n_outputs],
            tags: BTreeMap::new(),
            text_tags: BTreeMap::new(),
            subscriptions: Vec::new(),
            published: Vec::new(),
            log: Vec::new(),
            leases: Leases::new(devices.hal()),
            missed_ms: 0,
            now_ms: 0,
            forced: vec![None; n_outputs],
            events: VecDeque::new(),
            lost: 0,
            devices,
        }
    }

    /// Makes the output `slot` report `value` whatever the module computes; false if out of range.
    pub fn force(&mut self, slot: usize, value: Word) -> bool {
        match self.forced.get_mut(slot) {
            Some(f) => {
                *f = Some(value);
                true
            }
            None => false,
        }
    }

    /// Ends forcing of `slot`; false if out of range.
    pub fn unforce(&mut self, slot: usize) -> bool {
        match self.forced.get_mut(slot) {
            Some(f) => {
                *f = None;
                true
            }
            None => false,
        }
    }

    pub fn unforce_all(&mut self) {
        self.forced.fill(None);
    }

    pub fn forced(&self, slot: usize) -> Option<Word> {
        self.forced.get(slot).copied().flatten()
    }

    /// The 0/1 output events since the last take (oldest first) and how many were dropped.
    pub fn take_events(&mut self) -> (Vec<OutputEvent>, u32) {
        (
            self.events.drain(..).collect(),
            core::mem::take(&mut self.lost),
        )
    }

    /// `output_i`: the value the module must use (forced or its own), recording the change.
    fn output_i(&mut self, slot: i32, v: i32) -> i32 {
        let Some(i) = usize::try_from(slot)
            .ok()
            .filter(|&i| i < self.outputs.len())
        else {
            return v;
        };
        let v = self.forced[i].map_or(v, Word::as_int);
        let (old, new) = (self.outputs[i].0, v as u32);
        // Only 0/1 signals make events, like the C++ master.
        if old != new && old <= 1 && new <= 1 && i < 256 {
            if self.events.len() >= MAX_EVENTS {
                self.lost = self.lost.saturating_add(1);
            } else {
                self.events.push_back(OutputEvent {
                    slot: i as u8,
                    value: new as u8,
                    ms: self.now_ms,
                });
            }
        }
        self.outputs[i] = Word::int(v);
        v
    }

    fn output_f(&mut self, slot: i32, v: f32) -> f32 {
        let Some(i) = usize::try_from(slot)
            .ok()
            .filter(|&i| i < self.outputs.len())
        else {
            return v;
        };
        let v = self.forced[i].map_or(v, Word::as_float);
        self.outputs[i] = Word::float(v);
        v
    }
}

fn slot(slots: &[Word], i: i32) -> Word {
    slots.get(i as usize).copied().unwrap_or_default()
}

impl<D: Devices> Host for ModuleHost<D> {
    fn call(&mut self, import: usize, args: &[Val], mut mem: Mem<'_>) -> Option<Val> {
        let a = |i: usize| args.get(i).map_or(0, |v| v.i());
        let f = |i: usize| args.get(i).map_or(0.0, |v| v.f());
        let int = |v: i32| Some(Val::I32(v));
        let float = |v: f32| Some(Val::F32(v));
        match import {
            id::INPUT_I => int(slot(&self.inputs, a(0)).as_int()),
            id::INPUT_F => float(slot(&self.inputs, a(0)).as_float()),
            id::PARAM_I => int(slot(&self.params, a(0)).as_int()),
            id::PARAM_F => float(slot(&self.params, a(0)).as_float()),
            id::OUTPUT_I => int(self.output_i(a(0), a(1))),
            id::OUTPUT_F => float(self.output_f(a(0), f(1))),
            id::LOG => {
                if let Some(bytes) = mem.read(a(0), a(1), 1024) {
                    if self.log.len() < MAX_LOG_LINES {
                        self.log.push(String::from_utf8_lossy(&bytes).into_owned());
                    }
                }
                None
            }
            id::TAG_GET_I => int(self.tags.get(&a(0)).copied().unwrap_or_default().as_int()),
            id::TAG_GET_F => float(self.tags.get(&a(0)).copied().unwrap_or_default().as_float()),
            id::TAG_SET_I => {
                self.tags.insert(a(0), Word::int(a(1)));
                None
            }
            id::TAG_SET_F => {
                self.tags.insert(a(0), Word::float(f(1)));
                None
            }
            id::PUBLISH => {
                if let (Some(topic), Some(data)) =
                    (mem.read(a(0), a(1), 128), mem.read(a(2), a(3), MAX_MESSAGE))
                {
                    let topic = String::from_utf8_lossy(&topic).into_owned();
                    self.published.push((topic, data));
                }
                None
            }
            id::SUBSCRIBE => {
                if let Some(topic) = mem.read(a(0), a(1), 128) {
                    let topic = String::from_utf8_lossy(&topic).into_owned();
                    let subs = &mut self.subscriptions;
                    if subs.len() < MAX_TOPICS && !subs.contains(&topic) {
                        subs.push(topic);
                    }
                }
                None
            }
            id::GPIO_WRITE | id::PWM_WRITE => {
                let pin = a(0);
                let use_ = if import == id::GPIO_WRITE {
                    PinUse::Out
                } else {
                    PinUse::Pwm
                };
                self.leases
                    .take((pin & 0xFF) as u8, use_, Fault::from_pin_bits(pin));
                self.devices.call(import, args, mem)
            }
            id::TEXT => int(match mem.bytes() {
                Some(m) => text_op(m, core::array::from_fn(a)),
                None => 0,
            }),
            // `math(op, a, b)`: 0 sin, 1 cos, 2 tan, 3 pow (32 bit, like `sinf` and friends).
            id::MATH => float(math(a(0), f(1), f(2))),
            // `tag_text(op, id, ptr, n)`: 0 reads the text of a tag into the memory, 1 stores one.
            id::TAG_TEXT => int(if a(0) == 0 {
                let text = self.text_tags.get(&a(1)).cloned().unwrap_or_default();
                let len = text.len().min(a(3).max(0) as usize);
                if mem.write(a(2), &text[..len]) {
                    len as i32
                } else {
                    0
                }
            } else {
                match mem.read(a(2), a(3), 128) {
                    Some(bytes) => {
                        self.text_tags.insert(a(1), bytes);
                        1
                    }
                    None => 0,
                }
            }),
            _ => self.devices.call(import, args, mem),
        }
    }
}

/// `math(op, a, b)` of the ABI. With `std` it is the math of the host (on the PC the same as the
/// simulator); without it, `libm`.
fn math(op: i32, a: f32, b: f32) -> f32 {
    #[cfg(feature = "std")]
    return match op {
        0 => a.sin(),
        1 => a.cos(),
        2 => a.tan(),
        _ => a.powf(b),
    };
    #[cfg(not(feature = "std"))]
    match op {
        0 => libm::sinf(a),
        1 => libm::cosf(a),
        2 => libm::tanf(a),
        _ => libm::powf(a, b),
    }
}

/// `text(op, out, cap, a, alen, b, blen)`: the string operations of the master (`textOp`).
pub fn text_op(m: &mut [u8], args: [i32; 7]) -> i32 {
    let [op, out, cap, a, alen, b, blen] = args;
    let size = m.len();
    let inm = |p: i32, l: i32| p >= 0 && l >= 0 && p as usize + l as usize <= size;
    let reads_a = op == 1 || (5..=10).contains(&op);
    let reads_b = op == 5 || op == 8;
    let mut ok = (1..=11).contains(&op) && (!reads_a || inm(a, alen)) && (!reads_b || inm(b, blen));
    if (1..=4).contains(&op) || op == 7 || op == 11 {
        ok = ok && inm(out, cap);
    }
    if op == 6 || op == 10 {
        ok = ok && inm(out, 4);
    }
    if !ok {
        return if op == 8 || op == 9 { -1 } else { 0 };
    }
    let slice = |m: &[u8], p: i32, l: i32| m[p as usize..(p + l) as usize].to_vec();
    let put = |m: &mut [u8], cur: i32, s: &[u8]| -> i32 {
        let cur = cur.clamp(0, cap);
        let n = s.len().min((cap - cur) as usize);
        let at = (out + cur) as usize;
        m[at..at + n].copy_from_slice(&s[..n]);
        cur + n as i32
    };
    match op {
        1 => {
            let cur = b.clamp(0, cap);
            let n = alen.min(cap - cur) as usize;
            m.copy_within(a as usize..a as usize + n, (out + cur) as usize);
            cur + n as i32
        }
        2 => {
            let mut t = [0u8; crate::numfmt::FIXED_MAX];
            let n =
                crate::numfmt::fixed(f32::from_bits(a as u32), alen.clamp(0, 9) as usize, &mut t);
            put(m, b, &t[..n.min(47)])
        }
        3 => {
            let mut t = [0u8; 11];
            let n = crate::numfmt::int(a, &mut t);
            put(m, b, &t[..n])
        }
        4 => put(m, b, &[(a & 0xFF) as u8]),
        5 => {
            let (x, y) = (slice(m, a, alen), slice(m, b, blen));
            i32::from(text_match(&x, &y, out, cap != 0))
        }
        6 => {
            // Straight to f32 in a stack copy (decimal comma as point); too long text is not a number.
            if alen >= 48 {
                return 0;
            }
            let n = alen as usize;
            let mut t = [0u8; 47];
            t[..n].copy_from_slice(&m[a as usize..a as usize + n]);
            for c in &mut t[..n] {
                if *c == b',' {
                    *c = b'.';
                }
            }
            let Ok(s) = core::str::from_utf8(&t[..n]) else {
                return 0;
            };
            match s.trim_matches([' ', '\t', '\r', '\n']).parse::<f32>() {
                Ok(v) if v.is_finite() => {
                    m[out as usize..out as usize + 4].copy_from_slice(&v.to_le_bytes());
                    1
                }
                _ => 0,
            }
        }
        7 => {
            let start = b.clamp(0, alen);
            let end = if blen < 0 || start + blen > alen {
                alen
            } else {
                start + blen
            };
            let n = (end - start).min(cap) as usize;
            let from = (a + start) as usize;
            m.copy_within(from..from + n, out as usize);
            n as i32
        }
        8 => {
            let from = out.max(0);
            if from > alen {
                return -1;
            }
            if blen == 0 {
                return from;
            }
            let (x, y) = (slice(m, a, alen), slice(m, b, blen));
            (from as usize..)
                .take_while(|&at| at + y.len() <= x.len())
                .find(|&at| x[at..at + y.len()] == y[..])
                .map_or(-1, |at| at as i32)
        }
        9 => {
            if b >= 0 && b < alen {
                i32::from(m[(a + b) as usize])
            } else {
                -1
            }
        }
        10 => {
            let (mut v, mut part, mut digits, mut good) = ([0i32; 4], 0usize, false, true);
            for &ch in &m[a as usize..(a + alen) as usize] {
                if ch.is_ascii_digit() {
                    v[part] = v[part] * 10 + i32::from(ch - b'0');
                    digits = true;
                    good &= v[part] <= 255;
                } else if ch == b'.' && digits && part < 3 {
                    part += 1;
                    digits = false;
                } else {
                    good = false;
                }
                if !good {
                    break;
                }
            }
            if good && digits && part == 3 {
                for (i, x) in v.iter().enumerate() {
                    m[out as usize + i] = *x as u8;
                }
                1
            } else {
                0
            }
        }
        _ => {
            let t = format!(
                "{}.{}.{}.{}",
                a & 255,
                (a >> 8) & 255,
                (a >> 16) & 255,
                (a >> 24) & 255
            );
            put(m, 0, t.as_bytes())
        }
    }
}

/// `textMatch` of the master: mode 0 equal, 1 contains, 2 starts with, 3 ends with.
fn text_match(a: &[u8], b: &[u8], mode: i32, nocase: bool) -> bool {
    if (mode == 0 && a.len() != b.len()) || b.len() > a.len() {
        return false;
    }
    let first = if mode == 3 { a.len() - b.len() } else { 0 };
    let last = match mode {
        1 => a.len() - b.len(),
        0 => 0,
        _ => first,
    };
    (first..=last).any(|at| {
        a[at..at + b.len()].iter().zip(b).all(|(x, y)| {
            if nocase {
                x.eq_ignore_ascii_case(y)
            } else {
                x == y
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lease::NoHal;

    struct Dev;

    impl Devices for Dev {
        type Hal = NoHal;
        fn hal(&self) -> NoHal {
            NoHal
        }
        fn call(&mut self, _: usize, _: &[Val], _: Mem<'_>) -> Option<Val> {
            None
        }
    }

    fn host(n: usize) -> ModuleHost<Dev> {
        ModuleHost::new(Dev, n)
    }

    fn out_i(h: &mut ModuleHost<Dev>, slot: i32, v: i32) -> i32 {
        let r = h.call(id::OUTPUT_I, &[Val::I32(slot), Val::I32(v)], Mem::new(None));
        r.unwrap().i()
    }

    fn out_f(h: &mut ModuleHost<Dev>, slot: i32, v: f32) -> f32 {
        let r = h.call(id::OUTPUT_F, &[Val::I32(slot), Val::F32(v)], Mem::new(None));
        r.unwrap().f()
    }

    fn ev(slot: u8, value: u8, ms: u32) -> OutputEvent {
        OutputEvent { slot, value, ms }
    }

    #[test]
    fn unforced_returns_and_stores() {
        let mut h = host(2);
        assert_eq!(out_i(&mut h, 0, 7), 7);
        assert_eq!(h.outputs[0], Word::int(7));
        assert_eq!(out_f(&mut h, 1, 1.5), 1.5);
        assert_eq!(h.outputs[1], Word::float(1.5));
    }

    #[test]
    fn forced_int_and_unforce() {
        let mut h = host(3);
        assert!(h.force(0, Word::int(9)));
        assert!(h.force(1, Word::int(4)));
        assert_eq!(h.forced(0), Some(Word::int(9)));
        assert_eq!(out_i(&mut h, 0, 1), 9);
        assert_eq!(h.outputs[0], Word::int(9));
        assert!(h.unforce(0));
        assert_eq!(h.forced(0), None);
        assert_eq!(out_i(&mut h, 0, 1), 1);
        assert_eq!(h.outputs[0], Word::int(1));
        h.unforce_all();
        assert_eq!(h.forced(1), None);
        assert_eq!(out_i(&mut h, 1, 2), 2);
    }

    #[test]
    fn forced_float() {
        let mut h = host(1);
        assert!(h.force(0, Word::float(2.5)));
        assert_eq!(out_f(&mut h, 0, 1.0), 2.5);
        assert_eq!(h.outputs[0], Word::float(2.5));
        h.unforce(0);
        assert_eq!(out_f(&mut h, 0, 1.0), 1.0);
    }

    #[test]
    fn out_of_range() {
        let mut h = host(2);
        assert!(!h.force(2, Word::int(1)));
        assert!(!h.unforce(5));
        assert_eq!(h.forced(2), None);
        assert_eq!(out_i(&mut h, 2, 5), 5);
        assert_eq!(out_i(&mut h, -1, 6), 6);
        assert_eq!(out_f(&mut h, 9, 0.5), 0.5);
        assert_eq!(h.outputs, vec![Word::default(); 2]);
        assert_eq!(h.take_events(), (Vec::new(), 0));
    }

    #[test]
    fn events_of_bool_changes() {
        let mut h = host(2);
        for (ms, v) in [(10, 0), (20, 1), (30, 1), (40, 0)] {
            h.now_ms = ms;
            out_i(&mut h, 1, v);
        }
        assert_eq!(h.take_events(), (vec![ev(1, 1, 20), ev(1, 0, 40)], 0));
        assert_eq!(h.take_events(), (Vec::new(), 0));
    }

    #[test]
    fn no_events_for_non_bool_or_float() {
        let mut h = host(2);
        out_i(&mut h, 0, 1);
        h.take_events();
        out_i(&mut h, 0, 5);
        out_i(&mut h, 0, 1);
        out_f(&mut h, 1, 1.0);
        out_f(&mut h, 1, 0.0);
        assert_eq!(h.take_events(), (Vec::new(), 0));
    }

    #[test]
    fn forced_slot_records_once() {
        let mut h = host(1);
        h.force(0, Word::int(1));
        for ms in 0..3 {
            h.now_ms = ms;
            out_i(&mut h, 0, 0);
        }
        assert_eq!(h.take_events().0, vec![ev(0, 1, 0)]);
    }

    #[test]
    fn ring_is_bounded() {
        let mut h = host(1);
        for k in 0..(MAX_EVENTS as i32 + 5) {
            out_i(&mut h, 0, (k + 1) & 1);
        }
        let (events, lost) = h.take_events();
        assert_eq!(events.len(), MAX_EVENTS);
        assert_eq!(lost, 5);
        assert_eq!(h.take_events().1, 0);
    }

    /// Runs text op 6 on `text`, returns (ok, number).
    fn to_num(text: &[u8]) -> (i32, f32) {
        let mut m = [0u8; 128];
        m[8..8 + text.len()].copy_from_slice(text);
        let ok = text_op(&mut m, [6, 0, 0, 8, text.len() as i32, 0, 0]);
        (ok, f32::from_le_bytes([m[0], m[1], m[2], m[3]]))
    }

    #[test]
    fn text_to_num_parses_like_docs() {
        assert_eq!(to_num(b"21.5"), (1, 21.5));
        assert_eq!(to_num(b" -3 "), (1, -3.0));
        assert_eq!(to_num(b"1e3"), (1, 1000.0));
        assert_eq!(to_num(b"21,5"), (1, 21.5));
        assert_eq!(to_num(b"\t+0.1\r\n"), (1, 0.1));
        assert_eq!(to_num(b".5"), (1, 0.5));
        for bad in [
            &b"abc"[..],
            b"",
            b"  ",
            b"12kg",
            b"1e39",
            b"inf",
            b"NaN",
            b"\xff1",
        ] {
            assert_eq!(to_num(bad).0, 0, "{bad:?}");
        }
        let long = [b'0'; 48];
        assert_eq!(to_num(&long).0, 0);
        assert_eq!(to_num(&long[..47]).0, 1);
    }
}
