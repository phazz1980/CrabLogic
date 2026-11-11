//! A sheet of the schema as an ABI v4 WASM module, written straight with `wasm-encoder`.
//!
//! Values are 32 bit, as in the numeric model (`specs/numeric-model`): `bool` and `int` are `i32`
//! (wrapping), `float` is `f32`, `now` is an `i32` of milliseconds that wraps; so a module gives
//! the same bits as `crab-sim`. Block outputs are mutable globals that keep their value between
//! steps; values that cross the sheet border go through `tag_get_*` / `tag_set_*`.

use std::collections::HashMap;

use crab_core::value::parse_duration_ms;
use crab_core::{ParamKind, ParamValue, SignalType};
use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, EntityType, ExportKind, ExportSection, Function,
    FunctionSection, GlobalSection, GlobalType, ImportSection, Instruction as I, MemorySection,
    MemoryType, Module, TypeSection, ValType,
};

use crate::program::{Instance, Program, Source};
use crate::sheet::{signal_label, SheetError, SheetModule, TagKind};

mod block;
mod block_emit;
mod expander;
mod hardware;
mod modbus;
mod mqtt;
mod net;
mod retain;
mod sparkplug;
mod text;
mod types;
mod uart;
mod web;
use block_emit::BlockEmit;
use text::{Buf, A, MEM_START, STR_CAP};

/// Custom section with the bytes of memory a module needs (u32, little-endian); 0 without memory.
pub const MEM_SECTION: &str = "crab.mem";
/// Bytes above the last buffer, for the master's own reads past the end of a buffer.
const MEM_SPARE: u32 = 64;
use types::*;

const TAG_GET_I: u32 = 0;
const TAG_GET_F: u32 = 1;
const TAG_SET_I: u32 = 2;
const TAG_SET_F: u32 = 3;
const OUTPUT_I: u32 = 4;
const OUTPUT_F: u32 = 5;
/// `crab.yield() -> i32` (ABI 4): the work counter ran out; the host may suspend the module and
/// returns the next budget.
const YIELD: u32 = 6;
/// Imports every module declares; the hardware ones follow.
const FIXED_IMPORTS: u32 = 7;
const GLOBAL_NOW: u32 = 0;
/// The work counter: one unit per `step` and per loop pass; at zero the module calls `crab.yield`.
const GLOBAL_FUEL: u32 = 1;
/// The counter a module starts with, before the host gives the first budget.
const FUEL_START: i32 = 10_000;
/// The ABI the emitted modules speak (`abi_version`, `abi` in the manifest).
const ABI_VERSION: i32 = 4;

/// Hardware imports are numbered after the fixed ones once the used set is known; code is
/// emitted with these placeholders (`HW_BASE + position in HW_NAMES`) and remapped in `finish`.
const HW_BASE: u32 = 100;
const HW_GPIO_READ: u32 = HW_BASE;
const HW_GPIO_WRITE: u32 = HW_BASE + 1;
const HW_ADC_READ: u32 = HW_BASE + 2;
const HW_PWM_WRITE: u32 = HW_BASE + 3;
const HW_SENSOR_READ: u32 = HW_BASE + 4;
const HW_PCF_READ: u32 = HW_BASE + 5;
const HW_PCF_WRITE: u32 = HW_BASE + 6;
const HW_MODBUS: u32 = HW_BASE + 7;
const HW_UART: u32 = HW_BASE + 8;
const HW_TEXT: u32 = HW_BASE + 9;
const HW_NET: u32 = HW_BASE + 10;
const HW_NATIVE: u32 = HW_BASE + 11;
const HW_MATH: u32 = HW_BASE + 12;
const HW_RETAIN: u32 = HW_BASE + 13;
const HW_TAG_TEXT: u32 = HW_BASE + 14;
const HW_RETAIN_TEXT: u32 = HW_BASE + 15;
/// Placeholder of the module's own text append function (see `Emitter::append`), defined last.
const APPEND_FN: u32 = HW_BASE - 1;
const HW_NAMES: [&str; 16] = [
    "gpio_read",
    "gpio_write",
    "adc_read",
    "pwm_write",
    "sensor_read",
    "pcf_read",
    "pcf_write",
    "modbus",
    "uart",
    "text",
    "net",
    "native",
    "math",
    "retain",
    "tag_text",
    "retain_text",
];
/// Bit of the pin number passed to `gpio_read` that asks for the pull-up.
const PIN_PULLUP: i32 = 0x100;
/// Bits 9-10 of the pin number passed to `gpio_write` / `pwm_write`: what the master does with the
/// pin when the module traps, hangs or is unloaded (`on_fault` of the block). 0 drives it low (PWM
/// off), 1 high (PWM full), 2 leaves it as it is. The level is the physical one (after `inverted`).
const PIN_FAULT_SHIFT: i32 = 9;
const FAULT_LOW: i32 = 0;
const FAULT_HIGH: i32 = 1;
const FAULT_HOLD: i32 = 2;
/// `f32` scratch locals are numbered from here until `finish` knows how many `i32` ones there are.
const F_LOCAL_BASE: u32 = 1000;

/// Features the master firmware reports in `modcaps=` of its `stats` answer. The first version of the
/// master had none of them; a module that uses one is listed in `WasmModule::needs`. A master that
/// does not report a needed one has to be flashed again.
pub const MASTER_CAPS: [&str; 10] = [
    "math",
    "retain",
    "tag_text",
    "retain_text",
    "uart_format",
    "pcf_expander",
    "uart_console",
    "sparkplug",
    "on_fault",
    // ABI 4 (fuel counter and `crab.yield`): every module needs it.
    "abi4",
];

/// A finished module: the `.wasm` bytes and the text of `module.toml`.
#[derive(Debug, Clone)]
pub struct WasmModule {
    pub name: String,
    pub wasm: Vec<u8>,
    /// The same module with one wrapper function per import; every call goes through the wrappers.
    /// Only for building the AOT image (the interpreter is slower with them).
    pub wasm_aot: Vec<u8>,
    pub manifest: String,
    /// What the module needs from the master firmware beyond the first version of it (see
    /// `MASTER_CAPS`): a master that does not report all of these cannot run the module.
    pub needs: Vec<String>,
    /// With `watch`: `block.port` of every signal, in the order of the `output` index.
    pub watched: Vec<String>,
}

/// Why a sheet cannot become a WASM module.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum WasmError {
    #[error(transparent)]
    Sheet(#[from] SheetError),
    #[error("блок «{block}» ({kind}): {reason}")]
    Unsupported {
        block: String,
        kind: String,
        reason: String,
    },
}

impl Program<'_> {
    /// Builds sheet `sheet` as a module.
    pub fn wasm_module(&self, sheet: u32) -> Result<WasmModule, WasmError> {
        self.wasm_module_with(sheet, false)
    }

    /// With `watch` the module reports every block output through `output_i/f(i, v)`:
    /// for tests and the simulator, not for the board.
    pub fn wasm_module_with(&self, sheet: u32, watch: bool) -> Result<WasmModule, WasmError> {
        let module = self.sheet_module(sheet)?;
        let mut em = Emitter::new(self, &module);
        em.watch = watch;
        em.step_body()?;
        if em.mem_overflow {
            return Err(WasmError::Unsupported {
                block: String::new(),
                kind: String::new(),
                reason: "строки не помещаются в 64 КБ памяти модуля".to_owned(),
            });
        }
        Ok(em.finish())
    }
}

/// Watched signals per module the master firmware keeps (`kMaxOutputs`).
const MAX_WATCHED: usize = 128;

/// An expander chip (`pcf8574`, `pcf8575`) with blocks on it: the pins as last read and the
/// levels the blocks want, written once at the end of the step.
#[derive(Clone)]
struct XChip {
    /// `address | sda << 8 | scl << 16`, bit 24 for 16 pins (the key of `pcf_read`/`pcf_write`).
    key: i32,
    period: i32,
    in_word: u32,
    out_word: u32,
    /// Pins read by blocks: written high so the chip lets them float.
    in_mask: i32,
}

/// A text variable kept in flash by the master (`retain_text`).
#[derive(Clone)]
struct RetainedText {
    key: i32,
    buf: Buf,
    /// The last text known to be in flash.
    saved: Buf,
    /// Time of the last check.
    last: u32,
}

/// A variable kept in flash by the master (`retain`).
#[derive(Clone)]
struct Retained {
    name: String,
    /// Tag number when other sheets share the variable.
    tag: Option<u32>,
    ty: Ty,
    key: i32,
    /// The initial value as the bits of an `i32`.
    default: i32,
    /// The last value known to be in flash, as bits.
    saved: u32,
    /// Time of the last check.
    last: u32,
}

struct Global {
    ty: Ty,
    bits: u32,
}

struct Emitter<'a, 'p> {
    program: &'a Program<'p>,
    module: &'a SheetModule,
    table: Vec<crate::sheet::Tag>,
    code: Vec<I<'static>>,
    /// Globals; index 0 is `now`.
    globals: Vec<Global>,
    signals: HashMap<usize, u32>,
    vars: HashMap<String, u32>,
    /// Text variables of the sheet: a buffer each (`text.rs`).
    str_vars: HashMap<String, Buf>,
    /// Expander chips the blocks of this module stand on, by name.
    xp: std::collections::BTreeMap<String, XChip>,
    /// Mirrors of the text tags: what other sheets published, or what this sheet publishes.
    str_tags: HashMap<u32, Buf>,
    /// Retained text variables this module owns.
    retained_text: Vec<RetainedText>,
    /// `i32` and `f32` scratch locals in use by the current block (`i32` ones start at 1: local 0 is `dt`).
    scratch_i: u32,
    scratch_f: u32,
    /// Most scratch locals any block needed at once; blocks reuse the same locals.
    peak_i: u32,
    peak_f: u32,
    watch: bool,
    watched: Vec<String>,
    /// Hardware imports the code calls (placeholders, see `HW_BASE`).
    hw: std::collections::BTreeSet<u32>,
    /// The code calls the module's own append function (`APPEND_FN`).
    uses_append: bool,
    /// Newer master features used by the code that are not whole imports (see `WasmModule::needs`).
    extras: std::collections::BTreeSet<&'static str>,
    /// Offsets in `code` where `step` is cut into separate functions (block boundaries only):
    /// wasm3 refuses a function that overruns its stack height limit.
    cuts: Vec<usize>,
    /// Next free byte of the module memory (strings live there), see `text.rs`.
    mem_top: u32,
    mem_overflow: bool,
    /// Constant bytes of the data segment: (offset, bytes) and the same by content.
    data: Vec<(u32, Vec<u8>)>,
    consts: HashMap<Vec<u8>, u32>,
    /// Buffers of string signals by signal index.
    bufs: HashMap<usize, Buf>,
    /// Retained variables this module owns: loaded in `init`, saved from `step`.
    retained: Vec<Retained>,
}

/// Blocks per generated function. wasm3 without tail-call optimization (ESP32 build) recurses once
/// per operation of a function, so a long function overflows the task stack; short ones do not.
const BLOCKS_PER_CHUNK: usize = 8;
/// Pieces called straight from `step`; beyond that they go through groups of this size.
const PIECES_PER_GROUP: u32 = 16;

/// Group functions needed for `pieces` pieces after the first (0: `step` calls them itself).
fn group_count(pieces: usize) -> usize {
    if pieces as u32 <= PIECES_PER_GROUP {
        0
    } else {
        (pieces as u32).div_ceil(PIECES_PER_GROUP) as usize
    }
}

/// `append(out, cap, src, len, cur) -> cur + n`: copies `n = min(len, cap - cur)` bytes of `src`
/// to `out + cur` in a byte loop. Text building calls it instead of the host (a host call costs
/// microseconds on the board) and instead of an inline `memory.copy` (wamrc makes that slower and
/// bigger on RISC-V, see `specs/wasm-kernel/tasks.md` 3.6в).
fn append_function() -> Function {
    use wasm_encoder::MemArg;
    let byte = MemArg {
        offset: 0,
        align: 0,
        target: wasm_encoder::MemArgTarget::Memory(0),
        ordering: None,
    };
    let (out, cap, src, len, cur, n, i) = (0, 1, 2, 3, 4, 5, 6);
    let mut f = Function::new([(2, ValType::I32)]);
    for op in [
        // n = min(len, cap - cur)
        I::LocalGet(cap),
        I::LocalGet(cur),
        I::I32Sub,
        I::LocalSet(n),
        I::LocalGet(len),
        I::LocalGet(n),
        I::LocalGet(len),
        I::LocalGet(n),
        I::I32LtS,
        I::Select,
        I::LocalSet(n),
        // out += cur
        I::LocalGet(out),
        I::LocalGet(cur),
        I::I32Add,
        I::LocalSet(out),
        I::Block(BlockType::Empty),
        I::Loop(BlockType::Empty),
        I::LocalGet(i),
        I::LocalGet(n),
        I::I32GeS,
        I::BrIf(1),
        I::LocalGet(out),
        I::LocalGet(i),
        I::I32Add,
        I::LocalGet(src),
        I::LocalGet(i),
        I::I32Add,
        I::I32Load8U(byte),
        I::I32Store8(byte),
        I::LocalGet(i),
        I::I32Const(1),
        I::I32Add,
        I::LocalSet(i),
        I::Br(0),
        I::End,
        I::End,
        I::LocalGet(cur),
        I::LocalGet(n),
        I::I32Add,
        I::End,
    ] {
        f.instruction(&op);
    }
    f
}

impl<'a, 'p> Emitter<'a, 'p> {
    fn new(program: &'a Program<'p>, module: &'a SheetModule) -> Self {
        let mut em = Self {
            program,
            module,
            table: program.tag_table(),
            code: Vec::new(),
            globals: vec![
                Global { ty: Ty::I, bits: 0 },
                Global {
                    ty: Ty::I,
                    bits: FUEL_START as u32,
                },
            ],
            signals: HashMap::new(),
            vars: HashMap::new(),
            str_vars: HashMap::new(),
            xp: Default::default(),
            str_tags: HashMap::new(),
            retained_text: Vec::new(),
            scratch_i: 0,
            scratch_f: 0,
            peak_i: 0,
            peak_f: 0,
            watch: false,
            watched: Vec::new(),
            hw: Default::default(),
            uses_append: false,
            extras: Default::default(),
            cuts: Vec::new(),
            mem_top: MEM_START,
            mem_overflow: false,
            data: Vec::new(),
            consts: HashMap::new(),
            bufs: HashMap::new(),
            retained: Vec::new(),
        };
        for &bi in &module.blocks {
            for &signal in program.blocks[bi].outputs.values() {
                if program.signals[signal].ty == SignalType::String {
                    em.string_signal(&program.blocks[bi], signal);
                    continue;
                }
                let g = em.global(Ty::of(program.signals[signal].ty), &ParamValue::Int(0));
                em.signals.insert(signal, g);
            }
        }
        for local in &module.locals {
            if local.ty == SignalType::String {
                em.string_var(&local.name, &local.init);
                continue;
            }
            let g = em.global(Ty::of(local.ty), &local.init);
            em.vars.insert(local.name.clone(), g);
        }
        em.collect_string_tags();
        em.collect_expanders();
        em.collect_retained();
        em
    }

    /// A new global of type `ty` with the initial value `init` converted as the model does.
    fn global(&mut self, ty: Ty, init: &ParamValue) -> u32 {
        let x = pv(init);
        let bits = match ty {
            Ty::I => (x as i32) as u32,
            Ty::F => (x as f32).to_bits(),
        };
        self.globals.push(Global { ty, bits });
        (self.globals.len() - 1) as u32
    }

    fn state_i(&mut self) -> u32 {
        self.global(Ty::I, &ParamValue::Int(0))
    }

    fn tmp_i(&mut self) -> u32 {
        self.scratch_i += 1;
        self.peak_i = self.peak_i.max(self.scratch_i);
        self.scratch_i
    }

    fn tmp_f(&mut self) -> u32 {
        self.scratch_f += 1;
        self.peak_f = self.peak_f.max(self.scratch_f);
        F_LOCAL_BASE + self.scratch_f - 1
    }

    fn tmp(&mut self, ty: Ty) -> u32 {
        match ty {
            Ty::I => self.tmp_i(),
            Ty::F => self.tmp_f(),
        }
    }

    fn op(&mut self, i: I<'static>) {
        self.code.push(i);
    }

    fn c_i(&mut self, x: i32) {
        self.op(I::I32Const(x));
    }

    fn c_f(&mut self, x: f32) {
        self.op(I::F32Const(x.into()));
    }

    /// A constant `x` of type `ty` (as the model casts a number to it).
    fn c(&mut self, ty: Ty, x: f64) {
        match ty {
            Ty::I => self.c_i(x as i32),
            Ty::F => self.c_f(x as f32),
        }
    }

    /// Converts the value on the stack: `int` → `float` rounds, `float` → `int` saturates (NaN is 0).
    fn coerce(&mut self, from: Ty, to: Ty) {
        match (from, to) {
            (Ty::I, Ty::F) => self.op(I::F32ConvertI32S),
            (Ty::F, Ty::I) => self.op(I::I32TruncSatF32S),
            _ => {}
        }
    }

    /// Value on the stack → `1` if it is not zero, else `0` (an `i32`).
    fn flagify(&mut self, ty: Ty) {
        match ty {
            Ty::I => {
                self.c_i(0);
                self.op(I::I32Ne);
            }
            Ty::F => {
                self.c_f(0.0);
                self.op(I::F32Ne);
            }
        }
    }

    /// Comparison `op` (0 `==`, 1 `!=`, 2 `<`, 3 `<=`, 4 `>`, else `>=`) of two values of type `ty`.
    fn compare(&mut self, ty: Ty, op: i64) {
        self.op(match (ty, op) {
            (Ty::I, 0) => I::I32Eq,
            (Ty::I, 1) => I::I32Ne,
            (Ty::I, 2) => I::I32LtS,
            (Ty::I, 3) => I::I32LeS,
            (Ty::I, 4) => I::I32GtS,
            (Ty::I, _) => I::I32GeS,
            (Ty::F, 0) => I::F32Eq,
            (Ty::F, 1) => I::F32Ne,
            (Ty::F, 2) => I::F32Lt,
            (Ty::F, 3) => I::F32Le,
            (Ty::F, 4) => I::F32Gt,
            (Ty::F, _) => I::F32Ge,
        });
    }

    /// Two values of type `ty` on the stack → the smaller (`a < b ? a : b`) or the greater
    /// (`a > b ? a : b`): the same select as in the simulation, NaN included.
    fn pick(&mut self, ty: Ty, greater: bool) {
        let (a, b) = (self.tmp(ty), self.tmp(ty));
        self.op(I::LocalSet(b));
        self.op(I::LocalSet(a));
        self.op(I::LocalGet(a));
        self.op(I::LocalGet(b));
        self.op(I::LocalGet(a));
        self.op(I::LocalGet(b));
        self.compare(ty, if greater { 4 } else { 2 });
        self.op(I::Select);
    }

    fn unsupported(&self, inst: &Instance, reason: &str) -> WasmError {
        WasmError::Unsupported {
            block: inst.block.id.clone(),
            kind: inst.block.kind.clone(),
            reason: reason.to_owned(),
        }
    }

    fn tag_of(&self, kind: &TagKind) -> Option<(u32, Ty)> {
        self.table
            .iter()
            .find(|t| &t.kind == kind)
            .map(|t| (t.number, Ty::of(t.ty)))
    }

    fn var_ty(&self, name: &str) -> Ty {
        if let Some((_, ty)) = self.tag_of(&TagKind::Var(name.to_owned())) {
            ty
        } else if let Some(&g) = self.vars.get(name) {
            self.globals[g as usize].ty
        } else {
            Ty::I
        }
    }

    fn tag_get(&mut self, n: u32, ty: Ty) {
        self.c_i(n as i32);
        self.op(I::Call(if ty == Ty::F { TAG_GET_F } else { TAG_GET_I }));
    }

    /// The tag number `n` and the value of type `ty` are on the stack.
    fn tag_set_call(&mut self, ty: Ty) {
        self.op(I::Call(if ty == Ty::F { TAG_SET_F } else { TAG_SET_I }));
    }

    fn var_read(&mut self, name: &str) -> Ty {
        if let Some((n, ty)) = self.tag_of(&TagKind::Var(name.to_owned())) {
            self.tag_get(n, ty);
            ty
        } else if let Some(&g) = self.vars.get(name) {
            self.op(I::GlobalGet(g));
            self.globals[g as usize].ty
        } else {
            self.c_i(0);
            Ty::I
        }
    }

    /// Writes the value of type `from` on the stack into a variable.
    fn var_write(&mut self, name: &str, from: Ty) {
        let ty = self.var_ty(name);
        self.coerce(from, ty);
        if let Some((n, _)) = self.tag_of(&TagKind::Var(name.to_owned())) {
            let t = self.tmp(ty);
            self.op(I::LocalSet(t));
            self.c_i(n as i32);
            self.op(I::LocalGet(t));
            self.tag_set_call(ty);
        } else if let Some(&g) = self.vars.get(name) {
            self.op(I::GlobalSet(g));
        } else {
            self.op(I::Drop);
        }
    }

    /// Pushes the value on port `port` converted to the real type of the port.
    fn operand(&mut self, inst: &Instance, port: &str) -> Ty {
        let input = &inst.inputs[port];
        let want = Ty::of(input.ty);
        let from = match &input.source {
            Source::Default(v) => {
                self.c(want, pv(v));
                want
            }
            Source::Var(name) => self.var_read(name),
            Source::Signal { index, invert } => {
                let ty = Ty::of(self.program.signals[*index].ty);
                if let Some(&g) = self.signals.get(index) {
                    self.op(I::GlobalGet(g));
                } else {
                    let label = signal_label(self.program, *index);
                    let (n, tag_ty) = self
                        .tag_of(&TagKind::Signal(label))
                        .expect("cross-sheet link has a tag");
                    self.tag_get(n, tag_ty);
                    self.coerce(tag_ty, ty);
                }
                if *invert {
                    self.op(I::I32Eqz);
                }
                ty
            }
        };
        self.coerce(from, want);
        if inst.block.inverted_inputs.contains(port) {
            self.op(I::I32Eqz);
        }
        want
    }

    /// Type of input `i` in `block.toml` order (what `input` pushes); a port that is off counts as 0.
    fn input_ty(&self, inst: &Instance, i: usize) -> Ty {
        match inst.lib.def.inputs.get(i) {
            Some(p) => match inst.inputs.get(p.name.as_str()) {
                Some(input) => Ty::of(input.ty),
                None => Ty::of(p.ty).widen(Ty::I),
            },
            None => Ty::I,
        }
    }

    /// Input number `i` in `block.toml` order; zero if the port is off.
    fn input(&mut self, inst: &Instance, i: usize) -> Ty {
        match inst.lib.def.inputs.get(i) {
            Some(p) if inst.inputs.contains_key(p.name.as_str()) => self.operand(inst, &p.name),
            _ => {
                let ty = self.input_ty(inst, i);
                self.c(ty, 0.0);
                ty
            }
        }
    }

    /// Input `i` converted to `want`.
    fn input_as(&mut self, inst: &Instance, i: usize, want: Ty) {
        let ty = self.input(inst, i);
        self.coerce(ty, want);
    }

    /// Input `i` as an `i32` flag (not zero).
    fn flag(&mut self, inst: &Instance, i: usize) {
        let ty = self.input(inst, i);
        self.flagify(ty);
    }

    /// Type of parameter `i`: by its kind, or of the variable that feeds it.
    fn param_ty(&self, inst: &Instance, i: usize) -> Ty {
        let Some(p) = inst.lib.def.params.get(i) else {
            return Ty::F;
        };
        if Self::param_is_input(inst, &p.name) {
            return Ty::of(inst.inputs[p.name.as_str()].ty);
        }
        match inst.param_vars.get(p.name.as_str()) {
            Some(var) => self.var_ty(var),
            None => Ty::of_kind(p.kind),
        }
    }

    /// A parameter turned into an input (`param_in`) that has a source: its value comes
    /// from the link on every scan, not from the constant.
    fn param_is_input(inst: &Instance, name: &str) -> bool {
        inst.block.param_in.contains(name) && inst.inputs.contains_key(name)
    }

    fn param(&mut self, inst: &Instance, i: usize) -> Ty {
        let Some(p) = inst.lib.def.params.get(i) else {
            self.c_f(0.0);
            return Ty::F;
        };
        if Self::param_is_input(inst, &p.name) {
            return self.operand(inst, &p.name);
        }
        if let Some(var) = inst.param_vars.get(p.name.as_str()) {
            self.var_read(var)
        } else {
            let ty = Ty::of_kind(p.kind);
            let x = inst
                .params
                .get(p.name.as_str())
                .map_or(0.0, |v| num(v, p.kind));
            self.c(ty, x);
            ty
        }
    }

    fn param_as(&mut self, inst: &Instance, i: usize, want: Ty) {
        let ty = self.param(inst, i);
        self.coerce(ty, want);
    }

    /// Parameter `i` as a flag (not zero).
    fn param_flag(&mut self, inst: &Instance, i: usize) {
        let ty = self.param(inst, i);
        self.flagify(ty);
    }

    /// A time parameter in milliseconds as an `i32` that stands for a `u32` (negative counts as 0).
    fn param_ms(&mut self, inst: &Instance, i: usize) {
        let Some(p) = inst.lib.def.params.get(i) else {
            return self.c_i(0);
        };
        let fed = if Self::param_is_input(inst, &p.name) {
            Some(self.operand(inst, &p.name))
        } else {
            inst.param_vars
                .get(p.name.as_str())
                .map(|var| self.var_read(var))
        };
        if let Some(ty) = fed {
            match ty {
                Ty::F => self.op(I::I32TruncSatF32U),
                Ty::I => {
                    let x = self.tmp_i();
                    self.op(I::LocalSet(x));
                    self.op(I::LocalGet(x));
                    self.c_i(0);
                    self.op(I::LocalGet(x));
                    self.c_i(0);
                    self.op(I::I32GtS);
                    self.op(I::Select);
                }
            }
        } else {
            let ms = inst
                .params
                .get(p.name.as_str())
                .map_or(0.0, |v| num(v, p.kind));
            self.c_i((ms as u32) as i32);
        }
    }

    /// Constant value of parameter `i`, for blocks that cannot take it from a variable.
    fn const_param(&self, inst: &Instance, i: usize) -> Result<f64, WasmError> {
        let p = &inst.lib.def.params[i];
        if inst.param_vars.contains_key(p.name.as_str()) || Self::param_is_input(inst, &p.name) {
            return Err(self.unsupported(
                inst,
                &format!(
                    "параметр «{}» из переменной или входа пока не поддержан",
                    p.name
                ),
            ));
        }
        Ok(inst
            .params
            .get(p.name.as_str())
            .map_or(0.0, |v| num(v, p.kind)))
    }

    /// Current value of output `i` (the previous one while the block has not written it).
    fn out_get(&mut self, inst: &Instance, i: usize) -> Ty {
        let name = inst.lib.def.outputs[i].name.as_str();
        let g = self.signals[&inst.outputs[name]];
        self.op(I::GlobalGet(g));
        self.globals[g as usize].ty
    }

    /// Stores the value of type `from` on the stack as output `i`: global, tag for other sheets,
    /// bound variable.
    fn set_out(&mut self, inst: &Instance, i: usize, from: Ty) {
        let name = inst.lib.def.outputs[i].name.as_str();
        let signal = inst.outputs[name];
        let g = self.signals[&signal];
        let ty = self.globals[g as usize].ty;
        self.coerce(from, ty);
        self.op(I::GlobalSet(g));
        let label = signal_label(self.program, signal);
        // The master keeps `MAX_WATCHED` signals per module; reporting more only grows the code.
        let slot = self.watched.iter().position(|l| *l == label).or_else(|| {
            (self.watch && self.watched.len() < MAX_WATCHED).then(|| {
                self.watched.push(label.clone());
                self.watched.len() - 1
            })
        });
        if let (true, Some(k)) = (self.watch, slot) {
            self.c_i(k as i32);
            self.op(I::GlobalGet(g));
            // The host answers with the value to use (a forced one while forcing).
            self.op(I::Call(if ty == Ty::F { OUTPUT_F } else { OUTPUT_I }));
            self.op(I::GlobalSet(g));
        }
        if let Some((n, tag_ty)) = self.tag_of(&TagKind::Signal(label)) {
            self.c_i(n as i32);
            self.op(I::GlobalGet(g));
            self.coerce(ty, tag_ty);
            self.tag_set_call(tag_ty);
        }
        if let Some(var) = inst.output_vars.get(name) {
            self.op(I::GlobalGet(g));
            self.var_write(var, ty);
        }
    }

    /// Live (active) inputs combined with `join`; `empty` when there are none.
    fn fold_flags(&mut self, inst: &Instance, join: I<'static>, empty: i32) {
        let live: Vec<usize> = inst
            .lib
            .def
            .inputs
            .iter()
            .enumerate()
            .filter(|(_, p)| inst.inputs.contains_key(p.name.as_str()))
            .map(|(i, _)| i)
            .collect();
        if live.is_empty() {
            self.c_i(empty);
        }
        for (k, &i) in live.iter().enumerate() {
            self.flag(inst, i);
            if k > 0 {
                self.op(join.clone());
            }
        }
    }

    /// `if cond { then } else { otherwise }` giving a value of type `ty`; the condition (`i32`)
    /// is on the stack.
    fn choose(&mut self, ty: Ty, then: impl FnOnce(&mut Self), otherwise: impl FnOnce(&mut Self)) {
        self.op(I::If(BlockType::Result(ty.val())));
        then(self);
        self.op(I::Else);
        otherwise(self);
        self.op(I::End);
    }

    /// Milliseconds since the global `start` (an `i32` that stands for a `u32`).
    fn since(&mut self, start: u32) {
        self.op(I::GlobalGet(GLOBAL_NOW));
        self.op(I::GlobalGet(start));
        self.op(I::I32Sub);
    }

    fn step_body(&mut self) -> Result<(), WasmError> {
        self.fuel_check();
        self.op(I::GlobalGet(GLOBAL_NOW));
        self.op(I::LocalGet(0));
        self.op(I::I32Add);
        self.op(I::GlobalSet(GLOBAL_NOW));
        let module = self.module;
        let program = self.program;
        if let Some(ui) = module
            .blocks
            .iter()
            .map(|&bi| &program.blocks[bi])
            .find(|i| i.block.kind == "web_ui")
            .filter(|ui| self.web_enabled(ui))
        {
            // The page is set up before any widget reads or writes a value.
            self.scratch_i = 0;
            self.scratch_f = 0;
            self.web_setup(ui)?;
        }
        self.pull_string_tags();
        self.expander_poll();
        let blocks = self.module.blocks.clone();
        for (k, bi) in blocks.into_iter().enumerate() {
            if k > 0 && k % BLOCKS_PER_CHUNK == 0 {
                self.cuts.push(self.code.len());
            }
            // A block's temporaries never outlive it: state lives in globals.
            self.scratch_i = 0;
            self.scratch_f = 0;
            let program = self.program;
            self.block(&program.blocks[bi])?;
            self.publish_outputs(&program.blocks[bi]);
        }
        self.expander_flush();
        self.retain_save();
        Ok(())
    }

    /// One unit of work: the counter goes down and, when it runs out, `crab.yield` gives the next
    /// budget (the host may suspend the module there). Emitted at the start of `step` and at the
    /// top of every loop, so a loop that never ends still lets the other modules run.
    fn fuel_check(&mut self) {
        self.op(I::GlobalGet(GLOBAL_FUEL));
        self.c_i(1);
        self.op(I::I32Sub);
        self.op(I::GlobalSet(GLOBAL_FUEL));
        self.op(I::GlobalGet(GLOBAL_FUEL));
        self.c_i(1);
        self.op(I::I32LtS);
        self.op(I::If(BlockType::Empty));
        self.op(I::Call(YIELD));
        self.op(I::GlobalSet(GLOBAL_FUEL));
        self.op(I::End);
    }

    /// Writes an output block: `store` runs only when `en` allows.
    fn hold_unless_enabled(&mut self, inst: &Instance, store: impl FnOnce(&mut Self)) {
        let gated = inst.inputs.contains_key("en");
        if gated {
            self.flag(inst, 1);
            self.op(I::If(BlockType::Empty));
        }
        store(self);
        if gated {
            self.op(I::End);
        }
    }

    /// `i32` on the stack → clamped into `[lo, hi]`.
    fn clamp_int(&mut self, lo: i32, hi: i32) {
        self.c_i(lo);
        self.pick(Ty::I, true);
        self.c_i(hi);
        self.pick(Ty::I, false);
    }

    /// Text of a parameter (`table`, `kind`), empty when it is not set.
    fn text_param(&self, inst: &Instance, name: &str) -> String {
        match inst.params.get(name) {
            Some(ParamValue::Text(s)) => s.clone(),
            _ => String::new(),
        }
    }

    /// Number of a parameter by name, see `const_param`.
    fn named_param(&self, inst: &Instance, name: &str) -> Result<i32, WasmError> {
        let i = inst
            .lib
            .def
            .params
            .iter()
            .position(|p| p.name == name)
            .unwrap_or(0);
        Ok(self.const_param(inst, i)? as i32)
    }

    /// Number of a parameter by name as it is (a variable feeding it counts as 0).
    fn named_param_f(&self, inst: &Instance, name: &str) -> f64 {
        let i = inst
            .lib
            .def
            .params
            .iter()
            .position(|p| p.name == name)
            .unwrap_or(0);
        self.const_param(inst, i).unwrap_or(0.0)
    }

    fn finish(self) -> WasmModule {
        let (wasm, manifest, needs) = self.encode(false);
        let (wasm_aot, _, _) = self.encode(true);
        WasmModule {
            name: self.module.name.clone(),
            wasm,
            wasm_aot,
            manifest,
            needs,
            watched: self.watched,
        }
    }

    /// Encodes the module. With `aot`, every import gets a defined wrapper function (appended after
    /// all other defined functions) and every call goes through it: wamrc turns a call of an import
    /// into a bulky argument-packing sequence at each call site, a call of a wrapper into a plain one.
    fn encode(&self, aot: bool) -> (Vec<u8>, String, Vec<String>) {
        let mut types = TypeSection::new();
        types.ty().function([ValType::I32], [ValType::I32]); // 0: tag_get_i, gpio_read, adc_read
        types.ty().function([ValType::I32], [ValType::F32]); // 1: tag_get_f
        types.ty().function([ValType::I32, ValType::I32], []); // 2: tag_set_i, output_i, gpio_write, pwm_write
        types.ty().function([ValType::I32, ValType::F32], []); // 3: tag_set_f, output_f
        types.ty().function([], [ValType::I32]); // 4: abi_version
        types.ty().function([ValType::I32], []); // 5: step
        types.ty().function([], []); // 6: init
        types
            .ty()
            .function([ValType::I32, ValType::I32], [ValType::I32]); // 7: output_i
        types
            .ty()
            .function([ValType::I32, ValType::F32], [ValType::F32]); // 8: output_f
        types
            .ty()
            .function([ValType::I32, ValType::I32, ValType::I32], [ValType::F32]); // 9: sensor_read
        types
            .ty()
            .function([ValType::I32, ValType::I32, ValType::I32], [ValType::I32]); // 10: pcf_read
        types.ty().function([ValType::I32; 5], [ValType::I32]); // 11: modbus
        types.ty().function([ValType::I32; 4], [ValType::I32]); // 12: uart
        types.ty().function([ValType::I32; 7], [ValType::I32]); // 13: text
        types
            .ty()
            .function([ValType::I32, ValType::F32, ValType::F32], [ValType::F32]); // 14: math

        let mut imports = ImportSection::new();
        imports.import("crab", "tag_get_i", EntityType::Function(0));
        imports.import("crab", "tag_get_f", EntityType::Function(1));
        imports.import("crab", "tag_set_i", EntityType::Function(2));
        imports.import("crab", "tag_set_f", EntityType::Function(3));
        imports.import("crab", "output_i", EntityType::Function(7));
        imports.import("crab", "output_f", EntityType::Function(8));
        imports.import("crab", "yield", EntityType::Function(4));
        // Type index of every import, in import order (for the AOT wrappers).
        let mut import_types: Vec<u32> = vec![0, 1, 2, 3, 7, 8, 4];
        // Hardware imports follow in placeholder order; defined functions come after them.
        let used: Vec<u32> = self.hw.iter().copied().collect();
        let mut allow = Vec::new();
        for &h in &used {
            let name = HW_NAMES[(h - HW_BASE) as usize];
            let ty = match h {
                HW_GPIO_READ | HW_ADC_READ => 0,
                HW_SENSOR_READ => 9,
                HW_PCF_READ => 10,
                HW_PCF_WRITE => 7,
                HW_MODBUS | HW_NET => 11,
                HW_UART | HW_NATIVE | HW_TAG_TEXT | HW_RETAIN_TEXT => 12,
                HW_TEXT => 13,
                HW_MATH => 14,
                HW_RETAIN => 10,
                _ => 2,
            };
            imports.import("crab", name, EntityType::Function(ty));
            import_types.push(ty);
            allow.push(format!("\"{name}\""));
        }
        let first = FIXED_IMPORTS + used.len() as u32;
        let remap = |n: u32| match used.iter().position(|&h| h == n) {
            Some(k) => FIXED_IMPORTS + k as u32,
            None => n,
        };
        // The append function follows `abi`, `step`, `init`, pieces and groups; the wrapper of
        // import `k` comes after it.
        let groups = group_count(self.cuts.len());
        let append_fn = first + 3 + self.cuts.len() as u32 + groups as u32;
        let wrap_base = append_fn + u32::from(self.uses_append);
        // Index to call for a call of import `n` (a fixed index or a hardware placeholder).
        let target = |n: u32| {
            if n == APPEND_FN {
                return append_fn;
            }
            let n = if n >= HW_BASE { remap(n) } else { n };
            if aot && n < first {
                wrap_base + n
            } else {
                n
            }
        };
        let peak_i = self.peak_i;
        let local = |n: u32| {
            if n >= F_LOCAL_BASE {
                1 + peak_i + (n - F_LOCAL_BASE)
            } else {
                n
            }
        };

        let mut funcs = FunctionSection::new();
        funcs.function(4);
        funcs.function(5);
        funcs.function(6);
        for _ in &self.cuts {
            funcs.function(5);
        }
        for _ in 0..groups {
            funcs.function(5);
        }
        if self.uses_append {
            funcs.function(11);
        }
        if aot {
            for &ty in &import_types {
                funcs.function(ty);
            }
        }

        // A page (64 KB) is declared only by modules with strings; the master gives such a module only
        // the bytes it uses (`crab.mem`, `specs/mcu-memory`), not the whole page.
        let has_memory = self.mem_top > MEM_START || self.hw.contains(&HW_TEXT);
        let mem_need = if has_memory {
            ((self.mem_top + 15) & !15) + MEM_SPARE
        } else {
            0
        };
        let mut memory = MemorySection::new();
        memory.memory(MemoryType {
            minimum: u64::from(has_memory),
            maximum: None,
            memory64: false,
            shared: false,
            page_size_log2: None,
        });

        let mut globals = GlobalSection::new();
        for g in &self.globals {
            let init = match g.ty {
                Ty::I => ConstExpr::i32_const(g.bits as i32),
                Ty::F => ConstExpr::f32_const(f32::from_bits(g.bits).into()),
            };
            globals.global(
                GlobalType {
                    val_type: g.ty.val(),
                    mutable: true,
                    shared: false,
                },
                &init,
            );
        }

        let mut exports = ExportSection::new();
        exports.export("memory", ExportKind::Memory, 0);
        exports.export("abi_version", ExportKind::Func, first);
        exports.export("step", ExportKind::Func, first + 1);
        exports.export("init", ExportKind::Func, first + 2);

        let mut abi = Function::new([]);
        abi.instruction(&I::I32Const(ABI_VERSION))
            .instruction(&I::End);

        // `step` runs the first piece and calls the rest, each a function of its own with the same locals.
        let mut locals = vec![ValType::I32; self.peak_i as usize];
        locals.extend(vec![ValType::F32; self.peak_f as usize]);
        let mut bounds = vec![0];
        bounds.extend(self.cuts.iter().copied());
        bounds.push(self.code.len());
        let mut pieces: Vec<Function> = Vec::new();
        for w in bounds.windows(2) {
            let mut f = Function::new_with_locals_types(locals.clone());
            for i in &self.code[w[0]..w[1]] {
                match i {
                    I::Call(n) => f.instruction(&I::Call(target(*n))),
                    I::LocalGet(n) => f.instruction(&I::LocalGet(local(*n))),
                    I::LocalSet(n) => f.instruction(&I::LocalSet(local(*n))),
                    I::LocalTee(n) => f.instruction(&I::LocalTee(local(*n))),
                    other => f.instruction(other),
                };
            }
            pieces.push(f);
        }
        let mut step = pieces.remove(0);
        // The interpreter does not tail-call between wasm functions: every call in a row takes
        // C stack on the board. Many pieces are therefore called through groups of `PIECES_PER_GROUP`.
        let rest = pieces.len() as u32;
        let mut group_fns: Vec<Function> = Vec::new();
        if groups == 0 {
            for k in 0..rest {
                step.instruction(&I::LocalGet(0));
                step.instruction(&I::Call(first + 3 + k));
            }
        } else {
            for g in 0..groups as u32 {
                let mut f = Function::new([]);
                let from = g * PIECES_PER_GROUP;
                for k in from..rest.min(from + PIECES_PER_GROUP) {
                    f.instruction(&I::LocalGet(0));
                    f.instruction(&I::Call(first + 3 + k));
                }
                f.instruction(&I::End);
                group_fns.push(f);
                step.instruction(&I::LocalGet(0));
                step.instruction(&I::Call(first + 3 + rest + g));
            }
        }
        step.instruction(&I::End);
        for p in &mut pieces {
            p.instruction(&I::End);
        }

        // The owner of a tag writes its initial value once.
        let mut init = Function::new([(1, ValType::I32)]);
        for r in &self.retained_text {
            // What flash holds replaces the initial text; the copy for the change check follows.
            init.instruction(&I::I32Const(0));
            init.instruction(&I::I32Const(r.key));
            init.instruction(&I::I32Const(r.buf.ptr as i32));
            init.instruction(&I::I32Const(r.buf.cap as i32));
            init.instruction(&I::Call(target(HW_RETAIN_TEXT)));
            init.instruction(&I::LocalTee(0));
            init.instruction(&I::I32Const(0));
            init.instruction(&I::I32GeS);
            init.instruction(&I::If(BlockType::Empty));
            init.instruction(&I::LocalGet(0));
            init.instruction(&I::GlobalSet(r.buf.len));
            init.instruction(&I::End);
            for v in [1, r.saved.ptr as i32, r.saved.cap as i32, r.buf.ptr as i32] {
                init.instruction(&I::I32Const(v));
            }
            init.instruction(&I::GlobalGet(r.buf.len));
            init.instruction(&I::I32Const(0));
            init.instruction(&I::I32Const(0));
            init.instruction(&I::Call(target(HW_TEXT)));
            init.instruction(&I::GlobalSet(r.saved.len));
        }
        for r in &self.retained {
            // The value kept in flash (or the initial one) goes where the variable lives.
            init.instruction(&I::I32Const(0));
            init.instruction(&I::I32Const(r.key));
            init.instruction(&I::I32Const(r.default));
            init.instruction(&I::Call(target(HW_RETAIN)));
            init.instruction(&I::LocalTee(0));
            init.instruction(&I::GlobalSet(r.saved));
            match r.tag {
                Some(n) => {
                    init.instruction(&I::I32Const(n as i32));
                    init.instruction(&I::LocalGet(0));
                    if r.ty == Ty::F {
                        init.instruction(&I::F32ReinterpretI32);
                    }
                    init.instruction(&I::Call(target(if r.ty == Ty::F {
                        TAG_SET_F
                    } else {
                        TAG_SET_I
                    })));
                }
                None => {
                    init.instruction(&I::LocalGet(0));
                    if r.ty == Ty::F {
                        init.instruction(&I::F32ReinterpretI32);
                    }
                    init.instruction(&I::GlobalSet(self.vars[&r.name]));
                }
            }
        }
        for t in &self.module.tags {
            if t.tag.ty == SignalType::String {
                // The owner of a text variable publishes its initial (or kept) text.
                if let (Some(_), Some(buf)) = (&t.init, self.str_tags.get(&t.tag.number)) {
                    for v in [1, t.tag.number as i32, buf.ptr as i32] {
                        init.instruction(&I::I32Const(v));
                    }
                    init.instruction(&I::GlobalGet(buf.len));
                    init.instruction(&I::Call(target(HW_TAG_TEXT)));
                    init.instruction(&I::Drop);
                }
                continue;
            }
            if self.retained.iter().any(|r| r.tag == Some(t.tag.number)) {
                continue;
            }
            if let Some(value) = &t.init {
                init.instruction(&I::I32Const(t.tag.number as i32));
                let ty = Ty::of(t.tag.ty);
                match ty {
                    Ty::I => init.instruction(&I::I32Const(pv(value) as i32)),
                    Ty::F => init.instruction(&I::F32Const((pv(value) as f32).into())),
                };
                init.instruction(&I::Call(target(if ty == Ty::F {
                    TAG_SET_F
                } else {
                    TAG_SET_I
                })));
            }
        }
        init.instruction(&I::End);

        let mut code = CodeSection::new();
        code.function(&abi);
        code.function(&step);
        code.function(&init);
        for p in &pieces {
            code.function(p);
        }
        for g in &group_fns {
            code.function(g);
        }
        if self.uses_append {
            code.function(&append_function());
        }
        if aot {
            for (k, &ty) in import_types.iter().enumerate() {
                let nparams = match ty {
                    0 | 1 | 5 => 1,
                    2 | 3 | 7 | 8 => 2,
                    4 | 6 => 0,
                    9 | 10 | 14 => 3,
                    11 => 5,
                    12 => 4,
                    _ => 7,
                };
                let mut w = Function::new([]);
                for p in 0..nparams {
                    w.instruction(&I::LocalGet(p));
                }
                w.instruction(&I::Call(k as u32));
                w.instruction(&I::End);
                code.function(&w);
            }
        }

        let mut module = Module::new();
        module
            .section(&types)
            .section(&imports)
            .section(&funcs)
            .section(&memory)
            .section(&globals)
            .section(&exports)
            .section(&code);
        if !self.data.is_empty() {
            let mut data = wasm_encoder::DataSection::new();
            for (at, bytes) in &self.data {
                data.active(0, &ConstExpr::i32_const(*at as i32), bytes.iter().copied());
            }
            module.section(&data);
        }
        module.section(&wasm_encoder::CustomSection {
            name: MEM_SECTION.into(),
            data: mem_need.to_le_bytes().as_slice().into(),
        });

        let mut manifest = format!(
            "name = \"{}\"\nabi = {ABI_VERSION}\nallow = [{}]\n",
            self.module.name,
            allow.join(", ")
        );
        for t in &self.module.tags {
            let name = match &t.tag.kind {
                TagKind::Var(n) | TagKind::Signal(n) => n.replace('"', "'"),
            };
            manifest.push_str(&format!(
                "\n[[tag]]\nnumber = {}\nname = \"{}\"\ntype = \"{}\"\n",
                t.tag.number,
                name,
                format!("{:?}", t.tag.ty).to_lowercase()
            ));
        }
        let mut needs: Vec<String> = used
            .iter()
            .map(|&h| HW_NAMES[(h - HW_BASE) as usize])
            .filter(|n| MASTER_CAPS.contains(n))
            .map(str::to_owned)
            .collect();
        needs.extend(self.extras.iter().map(|n| (*n).to_owned()));
        needs.push("abi4".to_owned());
        needs.sort();
        (module.finish(), manifest, needs)
    }
}
