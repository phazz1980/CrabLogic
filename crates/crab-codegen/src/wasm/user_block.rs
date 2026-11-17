//! Blocks with their own `block.wasm` in a sheet module (`specs/wasm-drivers`): the module is
//! pasted into the sheet module (`driver.rs`), one copy per block instance with its own memory
//! window, globals and value cells. Its imports become small functions of the sheet module
//! ("shims"): `crab.input/param/output` read and write the cells, `crab.log` and `bus.*` call
//! the master (`crab.log`, `crab.bus`) with pointers moved into the window.
//!
//! Interface of the value imports (`specs/no-f64`; the old `f64` interface `crab.input/param/
//! output` and the module name `flp` are refused with `OLD_F64`, see `driver.rs`):
//! `crab.input_i/input_f/param_i/param_f/output_i/output_f` and the text imports
//! `crab.input_text/param_text/output_text`. A value cell is 4 bytes and sits at
//! `cells + 4 * index` (`i32` for `bool`/`int`/`duration`/`pin`, `f32` for `float`); the
//! type of a cell is the type of its port and the type of the import must match it: with a
//! constant port index in the call (`i32.const` right before it) the type of that very port is
//! checked, otherwise the block must have some port of the type (`check_v2_types`).
//! An index past the cells reads 0 / is ignored. String ports keep an unused cell, so every
//! index is the index of the port in `block.toml`; their text goes through `*_text` only:
//! the shim copies the bytes (`memory.copy`) into the block's window, at most `cap` of them,
//! and returns the copied length. `output_text(i, ptr, len)` copies into the string signal
//! of output `i` (at most its capacity). No `f64` appears in the module.

use std::rc::Rc;

use super::driver::{self, Driver};
use super::text::{Buf, Len, Str};
use super::*;
use wasm_encoder::MemArg;

/// Calls of `DRIVER_STEP + k` in the step code mean `step` of driver use `k` (see `encode`).
pub(super) const DRIVER_STEP: u32 = 1_000_000;

/// Op codes of `crab.bus`, as `crab_kernel::abi::bus_op` (the master's side).
const OP_CLAIM: i32 = 0;
const OP_LOCK: i32 = 1;
const CLAIM_PIN: i32 = 1;
const CLAIM_I2C: i32 = 2;
const CLAIM_UART: i32 = 3;
use crab_core::library::{BUS_I2C_ADDRESS, BUS_UART_PORT};

/// One argument of `crab.bus` made from the arguments of a driver import.
#[derive(Clone, Copy)]
pub(super) enum Arg {
    /// Argument `n` as it is.
    P(u32),
    /// Argument `n`, a pointer into the driver's window.
    Ptr(u32),
    /// `(arg a) | (arg b) << 16`.
    Pack(u32, u32),
}

/// What an import of a driver is: its signature and how the shim serves it.
#[derive(Clone, Copy)]
pub(super) enum Shim {
    InputI,
    InputF,
    ParamI,
    ParamF,
    OutputI,
    OutputF,
    InputText,
    ParamText,
    OutputText,
    Log,
    Config,
    Bus(i32, &'static [Arg]),
}

impl Shim {
    /// An import of the value interface (`input_i` and the like).
    pub(super) fn is_v2(self) -> bool {
        matches!(
            self,
            Shim::InputI
                | Shim::InputF
                | Shim::ParamI
                | Shim::ParamF
                | Shim::OutputI
                | Shim::OutputF
                | Shim::InputText
                | Shim::ParamText
                | Shim::OutputText
        )
    }
}

use Arg::{Pack, Ptr, P};

/// Imports a driver may use: module, name, parameter count (all `i32` unless said), shim.
const BUS_IMPORTS: &[(&str, usize, Shim)] = &[
    ("gpio_mode", 2, Shim::Bus(10, &[P(0), P(1)])),
    ("gpio_read", 1, Shim::Bus(11, &[P(0)])),
    ("gpio_write", 2, Shim::Bus(12, &[P(0), P(1)])),
    ("i2c_open", 3, Shim::Bus(20, &[P(0), P(1), P(2)])),
    ("i2c_write", 3, Shim::Bus(21, &[P(0), Ptr(1), P(2)])),
    ("i2c_read", 3, Shim::Bus(22, &[P(0), Ptr(1), P(2)])),
    ("i2c_send", 3, Shim::Bus(24, &[P(0), Ptr(1), P(2)])),
    (
        "i2c_write_read",
        5,
        Shim::Bus(23, &[P(0), Ptr(1), Pack(2, 4), Ptr(3)]),
    ),
    ("uart_open", 4, Shim::Bus(30, &[P(0), P(1), P(2), P(3)])),
    ("uart_write", 3, Shim::Bus(31, &[P(0), Ptr(1), P(2)])),
    ("uart_read", 3, Shim::Bus(32, &[P(0), Ptr(1), P(2)])),
    ("uart_available", 1, Shim::Bus(33, &[P(0)])),
    ("micros", 0, Shim::Bus(40, &[])),
    ("delay_us", 1, Shim::Bus(41, &[P(0)])),
    ("spi_open", 4, Shim::Bus(50, &[P(0), P(1), P(2), P(3)])),
    (
        "spi_transfer",
        4,
        Shim::Bus(51, &[P(0), Ptr(1), P(2), P(3)]),
    ),
    ("ow_reset", 1, Shim::Bus(60, &[P(0)])),
    ("ow_write", 3, Shim::Bus(61, &[P(0), Ptr(1), P(2)])),
    ("ow_read", 3, Shim::Bus(62, &[P(0), Ptr(1), P(2)])),
    ("ow_read_bit", 1, Shim::Bus(63, &[P(0)])),
    ("ow_write_bit", 2, Shim::Bus(64, &[P(0), P(1)])),
    ("pulse_read", 4, Shim::Bus(65, &[P(0), P(1), Ptr(2), P(3)])),
    ("count_open", 4, Shim::Bus(70, &[P(0), P(1), P(2), P(3)])),
    ("count_read", 1, Shim::Bus(71, &[P(0)])),
    ("count_clear", 1, Shim::Bus(72, &[P(0)])),
    ("count_period", 2, Shim::Bus(73, &[P(0), Ptr(1)])),
    ("count_kind", 0, Shim::Bus(74, &[])),
];

/// The shim of an import, or why the driver may not have it.
fn shim_of(
    module: &str,
    name: &str,
    params: &[ValType],
    results: &[ValType],
) -> Result<Shim, String> {
    use ValType::{F32, I32};
    let (shim, want_p, want_r): (Shim, Vec<ValType>, Vec<ValType>) = match (module, name) {
        ("flp", _) | ("crab", "input" | "param" | "output") => {
            return Err(driver::old_f64(&format!("импорт {module}.{name}")))
        }
        ("crab", "input_i") => (Shim::InputI, vec![I32], vec![I32]),
        ("crab", "input_f") => (Shim::InputF, vec![I32], vec![F32]),
        ("crab", "param_i") => (Shim::ParamI, vec![I32], vec![I32]),
        ("crab", "param_f") => (Shim::ParamF, vec![I32], vec![F32]),
        ("crab", "output_i") => (Shim::OutputI, vec![I32, I32], vec![]),
        ("crab", "output_f") => (Shim::OutputF, vec![I32, F32], vec![]),
        ("crab", "input_text") => (Shim::InputText, vec![I32; 3], vec![I32]),
        ("crab", "param_text") => (Shim::ParamText, vec![I32; 3], vec![I32]),
        ("crab", "output_text") => (Shim::OutputText, vec![I32; 3], vec![]),
        ("crab", "log") => (Shim::Log, vec![I32, I32], vec![]),
        ("crab", "config") => (Shim::Config, vec![I32, I32], vec![I32]),
        (crab_sim_bus::MODULE, _) => match BUS_IMPORTS.iter().find(|(n, ..)| *n == name) {
            Some(&(_, n, shim)) => (shim, vec![I32; n], vec![I32]),
            None => return Err(format!("нет такой функции шины: bus.{name}")),
        },
        _ => {
            return Err(format!(
                "импорт {module}.{name} недоступен драйверу (только crab.input*/param*/output*/log/config и bus.*)"
            ))
        }
    };
    if params != want_p.as_slice() || results != want_r.as_slice() {
        return Err(format!("у импорта {module}.{name} не та сигнатура"));
    }
    Ok(shim)
}

/// What a static look at a `block.wasm` found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WasmInfo {
    /// The module imports `bus.*` functions: it is a hardware driver.
    pub driver: bool,
    /// The module imports the value interface (`crab.input_i` and the like).
    pub v2: bool,
}

/// Parses `block.wasm` and checks every import the way the sheet module generator does.
pub(crate) fn inspect_block_wasm(bytes: &[u8]) -> Result<WasmInfo, String> {
    let d = driver::parse(bytes)?;
    let (mut driver, mut v2) = (false, false);
    for imp in &d.imports {
        let (params, results) = &d.types[imp.ty as usize];
        let shim = shim_of(&imp.module, &imp.name, params, results)?;
        driver |= matches!(shim, Shim::Bus(..));
        v2 |= shim.is_v2();
    }
    Ok(WasmInfo { driver, v2 })
}

/// Largest per-instance configuration a block may get through `crab.config`.
pub const MAX_BLOCK_CONFIG: usize = 4096;

/// Size of a value cell: `i32` or `f32`.
const CELL: u32 = 4;

/// Name of the import module of the bus functions (the same as `crab_sim::BUS_MODULE`).
mod crab_sim_bus {
    pub const MODULE: &str = "bus";
}

/// One instance of a block with `block.wasm` in the module.
pub(super) struct DriverUse {
    pub(super) driver: Rc<Driver>,
    pub(super) shims: Vec<Shim>,
    /// Memory window of the driver.
    pub(super) base: u32,
    /// Value cells (4-byte `i32`/`f32`) of the inputs, parameters and outputs.
    pub(super) cells_in: u32,
    pub(super) n_in: u32,
    pub(super) cells_param: u32,
    pub(super) n_param: u32,
    pub(super) cells_out: u32,
    pub(super) n_out: u32,
    /// Resources the instance claims in `init`: (kind, value).
    pub(super) claims: Vec<(i32, i32)>,
    /// Configuration bytes in the data segment: (address, length); length 0 when none.
    pub(super) config: (u32, u32),
    /// v2 text imports: (port index, string) of the string inputs and parameters, and
    /// (port index, buffer) of the string outputs.
    pub(super) text_in: Vec<(u32, Str)>,
    pub(super) text_param: Vec<(u32, Str)>,
    pub(super) text_out: Vec<(u32, Buf)>,
}

impl Emitter<'_, '_> {
    /// `n` bytes aligned to `align` (a power of two).
    fn alloc_aligned(&mut self, n: u32, align: u32) -> u32 {
        let at = (self.mem_top + align - 1) & !(align - 1);
        self.mem_top = at;
        self.alloc(n)
    }

    /// The parsed `block.wasm` of the block type (once per type).
    fn driver_of(&mut self, inst: &Instance) -> Result<Option<Rc<Driver>>, String> {
        let Some(path) = inst.lib.wasm_path() else {
            return Ok(None);
        };
        let kind = inst.block.kind.clone();
        if let Some(d) = self.parsed_drivers.get(&kind) {
            return Ok(Some(d.clone()));
        }
        let bytes = std::fs::read(&path)
            .map_err(|e| format!("не удалось прочитать {}: {e}", path.display()))?;
        let d = Rc::new(driver::parse(&bytes)?);
        self.parsed_drivers.insert(kind, d.clone());
        Ok(Some(d))
    }

    /// A block whose code is its own `block.wasm`; false for any other block.
    pub(super) fn user_block(&mut self, inst: &Instance) -> Result<bool, WasmError> {
        let d = match self.driver_of(inst) {
            Ok(Some(d)) => d,
            Ok(None) => return Ok(false),
            Err(e) => return Err(self.unsupported(inst, &format!("block.wasm: {e}"))),
        };
        // A driver talks to its pins through the master's buses: an expander pin is not one.
        if inst.block.expander.is_some() {
            return Err(self.unsupported(
                inst,
                "драйвер (block.wasm) на выводах расширителя не работает: подключите датчик к выводу платы",
            ));
        }
        let def = &inst.lib.def;
        let mut shims = Vec::new();
        for imp in &d.imports {
            let (params, results) = &d.types[imp.ty as usize];
            let shim = shim_of(&imp.module, &imp.name, params, results)
                .map_err(|e| self.unsupported(inst, &format!("block.wasm: {e}")))?;
            match shim {
                Shim::Bus(..) => {
                    self.hw.insert(HW_BUS);
                }
                Shim::Log => {
                    self.hw.insert(HW_LOG);
                }
                _ => {}
            }
            shims.push(shim);
        }
        let v2 = shims.iter().any(|s| s.is_v2());
        self.check_v2_types(inst, &d, &shims)?;
        // The text parameters reach a block through `param_text`; a display takes its model
        // from `crab.config` as well.
        let is_display = takes_text_params(&inst.block.kind);
        let text_param = def
            .params
            .iter()
            .find(|p| p.kind == ParamKind::Text && !is_display && !v2);
        if let Some(p) = text_param {
            let why = format!(
                "строковый параметр «{}» пока не передаётся в block.wasm в модуле",
                p.name
            );
            return Err(self.unsupported(inst, &why));
        }
        let mut claims = Vec::new();
        for (i, p) in def.params.iter().enumerate() {
            let kind = if p.kind == ParamKind::Pin {
                CLAIM_PIN
            } else if p.bus == BUS_I2C_ADDRESS {
                CLAIM_I2C
            } else if p.bus == BUS_UART_PORT {
                CLAIM_UART
            } else {
                continue;
            };
            if !def
                .enabled(p.when.as_deref(), &inst.block.params)
                .unwrap_or(true)
            {
                continue;
            }
            let value = self.const_param(inst, i).map_err(|_| {
                self.unsupported(inst, "вывод, адрес или порт драйвера задаётся числом")
            })? as i32;
            if value >= 0 {
                claims.push((kind, value));
            }
        }

        // Claims go through `crab.bus` even when the driver itself never calls `bus.*`.
        if !claims.is_empty() {
            self.hw.insert(HW_BUS);
        }
        let mut config = (0, 0);
        if let Some(bytes) = self.configs.get(&inst.block.id).filter(|b| !b.is_empty()) {
            if bytes.len() > MAX_BLOCK_CONFIG {
                let why = format!(
                    "конфигурация блока {} байт больше допустимых {MAX_BLOCK_CONFIG}",
                    bytes.len()
                );
                return Err(self.unsupported(inst, &why));
            }
            let bytes = bytes.clone();
            let at = self.alloc(bytes.len() as u32);
            config = (at, bytes.len() as u32);
            self.data.push((at, bytes));
        }

        let base = self.alloc_aligned(d.window.max(16), 16);
        // Offsets past 4 GB cannot be encoded: refuse now rather than in `encode`.
        let probe = driver::Place {
            type_base: 0,
            import_map: vec![0; d.imports.len()],
            func_base: 0,
            global_base: 0,
            mem_base: base,
            fuel: None,
        };
        if let Err(e) = d.bodies(&probe) {
            return Err(self.unsupported(inst, &e));
        }
        let display = self.display_extras(inst)?;
        let lora = self.lora_extras(inst)?;
        let extra_in =
            display.as_ref().map_or(0, |d| d.n_in()) + lora.as_ref().map_or(0, |l| l.n_in());
        let extra_out =
            display.as_ref().map_or(0, |d| d.n_out()) + lora.as_ref().map_or(0, |l| l.n_out());
        let n_in = def.inputs.len() as u32 + extra_in;
        let n_param = def.params.len() as u32;
        let n_out = def.outputs.len() as u32 + extra_out;
        let cell = CELL;
        let cells_in = self.alloc_aligned(cell * n_in.max(1), cell);
        let cells_param = self.alloc_aligned(cell * n_param.max(1), cell);
        let cells_out = self.alloc_aligned(cell * n_out.max(1), cell);
        // Constant parameters are in their cells from the start (a data segment), so the
        // driver's `init` already sees its pins and address.
        for i in 0..def.params.len() {
            if def.params[i].kind == ParamKind::Text {
                continue;
            }
            if let Ok(v) = self.const_param(inst, i) {
                let at = cells_param + cell * i as u32;
                let bytes = if def.params[i].kind == ParamKind::Float {
                    (v as f32).to_le_bytes().to_vec()
                } else {
                    (v as i32).to_le_bytes().to_vec()
                };
                self.data.push((at, bytes));
            }
        }

        // Text slots of v2: the strings the shims copy from.
        let (mut text_in, mut text_param, mut text_out) = (Vec::new(), Vec::new(), Vec::new());
        {
            for (i, p) in def.inputs.iter().enumerate() {
                if p.ty == SignalType::String {
                    text_in.push((i as u32, self.str_in(inst, &p.name)?));
                }
            }
            // String widgets of a display: slots after the own inputs and the numeric widgets.
            if let Some(d) = &display {
                let first = def.inputs.len() + d.widgets.len();
                for (k, w) in d.strings.iter().enumerate() {
                    let text = match w {
                        Some(w) => self.str_in(w, "in")?,
                        None => Str {
                            ptr: 0,
                            len: Len::Const(0),
                        },
                    };
                    text_in.push(((first + k) as u32, text));
                }
            }
            for (i, p) in def.params.iter().enumerate() {
                if p.kind == ParamKind::Text {
                    text_param.push((i as u32, self.param_str(inst, &p.name)?));
                }
            }
            for (i, p) in def.outputs.iter().enumerate() {
                if p.ty == SignalType::String {
                    if let Some(b) = self.out_buf(inst, &p.name) {
                        text_out.push((i as u32, b));
                    }
                }
            }
        }

        // Inputs and parameters go to their cells every step (a parameter may come from a tag).
        for i in 0..def.inputs.len() {
            if def.inputs[i].ty == SignalType::String {
                continue;
            }
            self.c_i((cells_in + cell * i as u32) as i32);
            let ty = self.input(inst, i);
            self.store_cell(ty);
        }
        for i in 0..def.params.len() {
            if def.params[i].kind == ParamKind::Text {
                continue;
            }
            self.c_i((cells_param + cell * i as u32) as i32);
            let ty = self.param(inst, i);
            let want = Ty::of_kind(def.params[i].kind);
            self.coerce(ty, want);
            self.store_cell(want);
        }
        if let Some(d) = &display {
            self.display_inputs(d, cells_in + cell * def.inputs.len() as u32);
        }
        if let Some(l) = &lora {
            self.lora_inputs(l, cells_in + cell * def.inputs.len() as u32);
        }
        let k = self.drivers.len() as u32;
        self.op(I::LocalGet(0));
        self.op(I::Call(DRIVER_STEP + k));
        for (i, port) in def.outputs.iter().enumerate() {
            if !inst.outputs.contains_key(port.name.as_str()) || port.ty == SignalType::String {
                continue;
            }
            self.c_i((cells_out + cell * i as u32) as i32);
            let ty = if port.ty == SignalType::Float {
                self.op(I::F32Load(Self::mem_arg(0, 2)));
                Ty::F
            } else {
                self.op(I::I32Load(Self::mem_arg(0, 2)));
                Ty::I
            };
            self.set_out(inst, i, ty);
        }
        if let Some(d) = &display {
            self.display_outputs(d, cells_out + cell * def.outputs.len() as u32);
        }
        if let Some(l) = &lora {
            self.lora_outputs(l, cells_out + cell * def.outputs.len() as u32);
        }
        self.drivers.push(DriverUse {
            driver: d,
            shims,
            text_in,
            text_param,
            text_out,
            base,
            cells_in,
            n_in,
            cells_param,
            n_param,
            cells_out,
            n_out,
            claims,
            config,
        });
        Ok(true)
    }
}

impl Emitter<'_, '_> {
    /// Stores the value of type `ty` on the stack (below it the address) into a v2 cell.
    pub(super) fn store_cell(&mut self, ty: Ty) {
        self.op(match ty {
            Ty::I => I::I32Store(Self::mem_arg(0, 2)),
            Ty::F => I::F32Store(Self::mem_arg(0, 2)),
        });
    }

    /// The string a text parameter has now: an input, a text variable or a constant.
    fn param_str(&mut self, inst: &Instance, name: &str) -> Result<Str, WasmError> {
        if Self::param_is_input(inst, name) {
            return self.str_in(inst, name);
        }
        if let Some(var) = inst.param_vars.get(name) {
            return match self.string_var_buf(var) {
                Some((b, _)) => Ok(Str {
                    ptr: b.ptr,
                    len: Len::Global(b.len),
                }),
                None => Err(self.unsupported(inst, "текстовая переменная не найдена")),
            };
        }
        let text = self.text_param(inst, name);
        Ok(self.const_str(text.as_bytes()))
    }

    /// An import reads a cell of its own type. With a constant port index in the call the type
    /// of that very port is checked; otherwise (an index computed by the block) the block must
    /// have some port of the type of the import.
    fn check_v2_types(&self, inst: &Instance, d: &Driver, shims: &[Shim]) -> Result<(), WasmError> {
        /// What a port is for the import: a number (`Some(Ty)`) or a string (`None`).
        type PortTy = Option<Ty>;
        let def = &inst.lib.def;
        // Ports of a display's extra slots have no type in `block.toml`: the display checks them.
        let display = self.program.display_plan(&inst.block.id).is_some()
            || self.program.lora_plan(&inst.block.id).is_some();
        let in_tys: Vec<PortTy> = (0..def.inputs.len())
            .map(|i| (def.inputs[i].ty != SignalType::String).then(|| self.input_ty(inst, i)))
            .collect();
        let param_tys: Vec<PortTy> = def
            .params
            .iter()
            .map(|p| (p.kind != ParamKind::Text).then(|| Ty::of_kind(p.kind)))
            .collect();
        let out_tys: Vec<PortTy> = def
            .outputs
            .iter()
            .map(|p| (p.ty != SignalType::String).then(|| Ty::of(p.ty)))
            .collect();
        // Import name, port types of its side, the type it works with (`None`: text), the
        // port kind for the message.
        let rule = |s: Shim| -> Option<(&'static str, &Vec<PortTy>, PortTy, &'static str)> {
            Some(match s {
                Shim::InputI => ("input_i", &in_tys, Some(Ty::I), "bool или int вход"),
                Shim::InputF => ("input_f", &in_tys, Some(Ty::F), "float вход"),
                Shim::ParamI => (
                    "param_i",
                    &param_tys,
                    Some(Ty::I),
                    "параметр bool, int, duration или pin",
                ),
                Shim::ParamF => ("param_f", &param_tys, Some(Ty::F), "float параметр"),
                Shim::OutputI => ("output_i", &out_tys, Some(Ty::I), "bool или int выход"),
                Shim::OutputF => ("output_f", &out_tys, Some(Ty::F), "float выход"),
                Shim::InputText => ("input_text", &in_tys, None, "вход string"),
                Shim::ParamText => ("param_text", &param_tys, None, "параметр text"),
                Shim::OutputText => ("output_text", &out_tys, None, "выход string"),
                _ => return None,
            })
        };
        let mut seen = Vec::new();
        for &s in shims {
            let Some((name, tys, want, what)) = rule(s) else {
                continue;
            };
            // A text import has no port of the type to look for besides the string ones.
            if want.is_some() && !seen.contains(&name) {
                seen.push(name);
                let extras =
                    display && !name.ends_with("text") && name != "param_i" && name != "param_f";
                if !extras && !tys.contains(&want) {
                    return Err(self.unsupported(
                        inst,
                        &format!(
                            "block.wasm использует crab.{name}, а в блоке нет портов этого типа (нужен {what}): тип импорта должен совпадать с типом порта в block.toml"
                        ),
                    ));
                }
            }
        }
        for (imp, index) in d.const_first_args() {
            let Some((name, tys, want, what)) = rule(shims[imp]) else {
                continue;
            };
            // An index past the ports: a display's extra slot, or a cell that reads 0.
            let Some(have) = usize::try_from(index).ok().and_then(|i| tys.get(i)) else {
                continue;
            };
            if *have != want {
                let is = match have {
                    Some(Ty::I) => "число int (i32)",
                    Some(Ty::F) => "число float (f32)",
                    None => "строка",
                };
                return Err(self.unsupported(
                    inst,
                    &format!(
                        "block.wasm вызывает crab.{name}({index}, …), а порт {index} в block.toml — {is}: нужен {what}"
                    ),
                ));
            }
        }
        Ok(())
    }
}

/// What a display block gets besides its own ports (see `display.rs`).
struct DisplayExtras<'a, 'p> {
    /// The widget input slots: the instance that feeds the widget on this sheet, if any.
    widgets: Vec<Option<&'a Instance<'p>>>,
    /// The string widget slots (after the numeric ones): the instance feeding the widget, if any.
    strings: Vec<Option<&'a Instance<'p>>>,
    menu: Option<MenuExtras<'a, 'p>>,
}

struct MenuExtras<'a, 'p> {
    inst: &'a Instance<'p>,
    vars: Vec<super::display::MenuVar>,
}

impl DisplayExtras<'_, '_> {
    fn n_in(&self) -> u32 {
        self.widgets.len() as u32
            + self.strings.len() as u32
            + self.menu.as_ref().map_or(0, |m| 4 + m.vars.len() as u32)
    }

    fn n_out(&self) -> u32 {
        self.menu.as_ref().map_or(0, |m| 4 + m.vars.len() as u32)
    }
}

/// The `in` of an instance is linked (not the default of an unconnected port).
fn in_linked(i: &Instance) -> bool {
    !matches!(i.inputs["in"].source, Source::Default(_))
}

impl<'a, 'p> Emitter<'a, 'p> {
    /// The extra inputs and outputs of a display block; `None` for any other block.
    fn display_extras(
        &mut self,
        inst: &Instance,
    ) -> Result<Option<DisplayExtras<'a, 'p>>, WasmError> {
        let program = self.program;
        let Some(plan) = program.display_plan(&inst.block.id) else {
            return Ok(None);
        };
        let module = self.module;
        let on_sheet = |id: String| {
            module
                .blocks
                .iter()
                .map(|&bi| &program.blocks[bi])
                .filter(move |i| i.block.id == id || i.block.tag.as_deref() == Some(id.as_str()))
        };
        // The widget itself or an image of it on this sheet feeds it; a linked one wins.
        let feeder = |w: &String| {
            let all: Vec<&Instance> = on_sheet(w.clone())
                .filter(|i| i.inputs.contains_key("in"))
                .collect();
            all.iter()
                .copied()
                .find(|i| in_linked(i))
                .or(all.first().copied())
        };
        let widgets = plan.widgets.iter().map(feeder).collect();
        let strings = plan.string_widgets.iter().map(feeder).collect();
        let menu = match plan.menu {
            Some(m) => {
                let Some(menu) = on_sheet(m.block.clone()).find(|i| i.block.id == m.block) else {
                    return Err(self.unsupported(
                        inst,
                        &format!(
                            "меню «{}» должно стоять на том же листе, что и дисплей",
                            m.block
                        ),
                    ));
                };
                Some(MenuExtras {
                    inst: menu,
                    vars: m.vars,
                })
            }
            None => None,
        };
        Ok(Some(DisplayExtras {
            widgets,
            strings,
            menu,
        }))
    }

    /// Stores the widget values, the menu buttons and the menu variables in the input cells
    /// after the block's own ports (`first` is the address of the first extra cell).
    ///
    /// Layout of the extras (cell index counted after the block's own inputs; cells are
    /// 4 bytes, typed by the source signal: `bool`/`int` -> `i32`, `float` -> `f32`): numeric widgets (one per widget of the plan), then string widgets (task 6:
    /// `disp_string`; read with `input_text` at their slot, the numeric cell stays unused; an unlinked one reads as an empty string),
    /// then the four menu buttons `up`, `down`, `ok`, `back` (`i32`), then one cell per menu
    /// variable (type of the variable). Extra output cells follow the block's own outputs:
    /// `open`, `item`, `changed` (`i32`), the index of the variable just confirmed (`i32`; a value that is no variable index
    /// means none), then one cell per menu variable (type of the variable).
    fn display_inputs(&mut self, d: &DisplayExtras, first: u32) {
        let cell = CELL;
        let mut at = first;
        for w in d.widgets.iter().copied() {
            if let Some(w) = w {
                self.c_i(at as i32);
                let ty = self.operand(w, "in");
                self.store_cell(ty);
            }
            at += cell;
        }
        // String widgets: their text is read with `input_text`, the numeric cell stays unused.
        at += cell * d.strings.len() as u32;
        let Some(menu) = &d.menu else {
            return;
        };
        for button in ["up", "down", "ok", "back"] {
            if menu.inst.inputs.contains_key(button) {
                self.c_i(at as i32);
                let ty = self.operand(menu.inst, button);
                self.store_cell(ty);
            }
            at += cell;
        }
        for v in &menu.vars {
            self.c_i(at as i32);
            let ty = self.var_read(&v.name);
            self.store_cell(ty);
            at += cell;
        }
    }

    /// After the driver's step: the menu outputs go to the `menu` block, a confirmed variable
    /// is written (and saved at once when it is retained).
    fn display_outputs(&mut self, d: &DisplayExtras, first: u32) {
        let Some(menu) = &d.menu else {
            return;
        };
        let cell = CELL;
        // An `i32` out of a cell.
        let load_i = |e: &mut Self, at: u32| {
            e.c_i(at as i32);
            e.op(I::I32Load(Self::mem_arg(0, 2)));
        };
        let def = &menu.inst.lib.def;
        for (k, port) in ["open", "item", "changed"].into_iter().enumerate() {
            let Some(i) = def.outputs.iter().position(|p| p.name == port) else {
                continue;
            };
            if !menu.inst.outputs.contains_key(port) {
                continue;
            }
            load_i(self, first + cell * k as u32);
            self.set_out(menu.inst, i, Ty::I);
        }
        let slot = first + cell * 3;
        for (v, var) in menu.vars.iter().enumerate() {
            load_i(self, slot);
            self.c_i(v as i32);
            self.op(I::I32Eq);
            if var.action {
                // A pulse of one step.
                self.var_write(&var.name, Ty::I);
                continue;
            }
            self.op(I::If(BlockType::Empty));
            let at = slot + cell * (1 + v as u32);
            let ty = self.var_ty(&var.name);
            self.c_i(at as i32);
            self.op(if ty == Ty::F {
                I::F32Load(Self::mem_arg(0, 2))
            } else {
                I::I32Load(Self::mem_arg(0, 2))
            });
            self.var_write(&var.name, ty);
            // Retained: saved in this scan, not after `settle` and `period`.
            if let Some(at) = self.retained.iter().position(|r| r.name == var.name) {
                let force = match self.retained[at].force {
                    Some(g) => g,
                    None => {
                        let g = self.state_i();
                        self.retained[at].force = Some(g);
                        g
                    }
                };
                self.c_i(1);
                self.op(I::GlobalSet(force));
            }
            self.op(I::End);
        }
    }
}

/// Body of the shim of one driver import. `bus` and `log` are the final indices of the
/// master's imports (or their AOT wrappers).
pub(super) fn shim_body(shim: Shim, u: &DriverUse, bus: u32, log: u32) -> Function {
    let mut f = Function::new([]);
    match shim {
        Shim::InputI => read_cell(&mut f, u.cells_in, u.n_in, ValType::I32),
        Shim::InputF => read_cell(&mut f, u.cells_in, u.n_in, ValType::F32),
        Shim::ParamI => read_cell(&mut f, u.cells_param, u.n_param, ValType::I32),
        Shim::ParamF => read_cell(&mut f, u.cells_param, u.n_param, ValType::F32),
        Shim::OutputI | Shim::OutputF => {
            f.instruction(&I::LocalGet(0));
            f.instruction(&I::I32Const(u.n_out as i32));
            f.instruction(&I::I32LtU);
            f.instruction(&I::If(BlockType::Empty));
            f.instruction(&I::LocalGet(0));
            f.instruction(&I::I32Const(4));
            f.instruction(&I::I32Mul);
            f.instruction(&I::LocalGet(1));
            let m = MemArg {
                offset: u64::from(u.cells_out),
                ..Emitter::mem_arg(0, 2)
            };
            f.instruction(&if matches!(shim, Shim::OutputF) {
                I::F32Store(m)
            } else {
                I::I32Store(m)
            });
            f.instruction(&I::End);
        }
        Shim::InputText => return text_in_shim(u, &u.text_in),
        Shim::ParamText => return text_in_shim(u, &u.text_param),
        Shim::OutputText => return text_out_shim(u),
        Shim::Log => {
            f.instruction(&I::LocalGet(0));
            f.instruction(&I::I32Const(u.base as i32));
            f.instruction(&I::I32Add);
            f.instruction(&I::LocalGet(1));
            f.instruction(&I::Call(log));
        }
        Shim::Config => return config_shim(u),
        Shim::Bus(op, args) => {
            f.instruction(&I::I32Const(op));
            for a in args {
                match *a {
                    P(n) => {
                        f.instruction(&I::LocalGet(n));
                    }
                    Ptr(n) => {
                        f.instruction(&I::LocalGet(n));
                        f.instruction(&I::I32Const(u.base as i32));
                        f.instruction(&I::I32Add);
                    }
                    Pack(a, b) => {
                        f.instruction(&I::LocalGet(a));
                        f.instruction(&I::LocalGet(b));
                        f.instruction(&I::I32Const(16));
                        f.instruction(&I::I32Shl);
                        f.instruction(&I::I32Or);
                    }
                }
            }
            for _ in args.len()..4 {
                f.instruction(&I::I32Const(0));
            }
            f.instruction(&I::Call(bus));
        }
    }
    f.instruction(&I::End);
    f
}

/// `index < n ? cell(index) : 0` for a cell of 4 bytes of type `ty`.
fn read_cell(f: &mut Function, cells: u32, n: u32, ty: ValType) {
    f.instruction(&I::LocalGet(0));
    f.instruction(&I::I32Const(n as i32));
    f.instruction(&I::I32LtU);
    f.instruction(&I::If(BlockType::Result(ty)));
    f.instruction(&I::LocalGet(0));
    f.instruction(&I::I32Const(4));
    f.instruction(&I::I32Mul);
    let m = MemArg {
        offset: u64::from(cells),
        ..Emitter::mem_arg(0, 2)
    };
    f.instruction(&if ty == ValType::F32 {
        I::F32Load(m)
    } else {
        I::I32Load(m)
    });
    f.instruction(&I::Else);
    f.instruction(&if ty == ValType::F32 {
        I::F32Const(0.0.into())
    } else {
        I::I32Const(0)
    });
    f.instruction(&I::End);
}

/// `input_text(i, ptr, cap) -> n` (also `param_text`): copies `n = min(cap, len)` bytes of the
/// string of slot `i` to `ptr` of the block's window and returns `n`; 0 for a slot that is no
/// string or when the bytes do not fit the window.
fn text_in_shim(u: &DriverUse, slots: &[(u32, Str)]) -> Function {
    let window = u.driver.window;
    // Locals: 0 i, 1 ptr, 2 cap, 3 n.
    let mut f = Function::new([(1, ValType::I32)]);
    let mut i = |x: I<'static>| {
        f.instruction(&x);
    };
    for &(slot, s) in slots {
        let len = |i: &mut dyn FnMut(I<'static>)| match s.len {
            Len::Const(n) => i(I::I32Const(n as i32)),
            Len::Global(g) => i(I::GlobalGet(g)),
        };
        i(I::LocalGet(0));
        i(I::I32Const(slot as i32));
        i(I::I32Eq);
        i(I::If(BlockType::Empty));
        // n = cap > len ? len : cap
        len(&mut i);
        i(I::LocalGet(2));
        i(I::LocalGet(2));
        len(&mut i);
        i(I::I32GtU);
        i(I::Select);
        i(I::LocalSet(3));
        // Does not fit the window: nothing is copied.
        i(I::LocalGet(1));
        i(I::I32Const(window as i32));
        i(I::I32GtU);
        i(I::If(BlockType::Empty));
        i(I::I32Const(0));
        i(I::Return);
        i(I::End);
        i(I::LocalGet(3));
        i(I::I32Const(window as i32));
        i(I::LocalGet(1));
        i(I::I32Sub);
        i(I::I32GtU);
        i(I::If(BlockType::Empty));
        i(I::I32Const(0));
        i(I::Return);
        i(I::End);
        i(I::LocalGet(1));
        i(I::I32Const(u.base as i32));
        i(I::I32Add);
        i(I::I32Const(s.ptr as i32));
        i(I::LocalGet(3));
        i(I::MemoryCopy {
            src_mem: 0,
            dst_mem: 0,
        });
        i(I::LocalGet(3));
        i(I::Return);
        i(I::End);
    }
    i(I::I32Const(0));
    i(I::End);
    f
}

/// `output_text(i, ptr, len)`: copies `min(len, capacity)` bytes from `ptr` of the window into
/// the buffer of the string output `i` and sets its length; nothing for a bad range.
fn text_out_shim(u: &DriverUse) -> Function {
    let window = u.driver.window;
    // Locals: 0 i, 1 ptr, 2 len, 3 n.
    let mut f = Function::new([(1, ValType::I32)]);
    let mut i = |x: I<'static>| {
        f.instruction(&x);
    };
    for &(slot, b) in &u.text_out {
        i(I::LocalGet(0));
        i(I::I32Const(slot as i32));
        i(I::I32Eq);
        i(I::If(BlockType::Empty));
        // n = len > cap ? cap : len
        i(I::I32Const(b.cap as i32));
        i(I::LocalGet(2));
        i(I::LocalGet(2));
        i(I::I32Const(b.cap as i32));
        i(I::I32GtU);
        i(I::Select);
        i(I::LocalSet(3));
        i(I::LocalGet(1));
        i(I::I32Const(window as i32));
        i(I::I32GtU);
        i(I::If(BlockType::Empty));
        i(I::Return);
        i(I::End);
        i(I::LocalGet(3));
        i(I::I32Const(window as i32));
        i(I::LocalGet(1));
        i(I::I32Sub);
        i(I::I32GtU);
        i(I::If(BlockType::Empty));
        i(I::Return);
        i(I::End);
        i(I::I32Const(b.ptr as i32));
        i(I::LocalGet(1));
        i(I::I32Const(u.base as i32));
        i(I::I32Add);
        i(I::LocalGet(3));
        i(I::MemoryCopy {
            src_mem: 0,
            dst_mem: 0,
        });
        i(I::LocalGet(3));
        i(I::GlobalSet(b.len));
        i(I::Return);
        i(I::End);
    }
    i(I::End);
    f
}

/// `config(ptr, cap) -> len`: copies `min(cap, len)` bytes of the instance's configuration to
/// `ptr` of the driver's window (nothing when that does not fit the window) and returns `len`.
fn config_shim(u: &DriverUse) -> Function {
    let (at, len) = u.config;
    let window = u.driver.window;
    // Locals: 0 ptr, 1 cap, 2 n, 3 i.
    let mut f = Function::new([(2, ValType::I32)]);
    let mut i = |x: I<'static>| {
        f.instruction(&x);
    };
    // n = cap > len ? len : cap
    i(I::I32Const(len as i32));
    i(I::LocalGet(1));
    i(I::LocalGet(1));
    i(I::I32Const(len as i32));
    i(I::I32GtU);
    i(I::Select);
    i(I::LocalSet(2));
    // if ptr > window || n > window - ptr { n = 0 }
    i(I::LocalGet(0));
    i(I::I32Const(window as i32));
    i(I::I32GtU);
    i(I::If(BlockType::Empty));
    i(I::I32Const(0));
    i(I::LocalSet(2));
    i(I::Else);
    i(I::LocalGet(2));
    i(I::I32Const(window as i32));
    i(I::LocalGet(0));
    i(I::I32Sub);
    i(I::I32GtU);
    i(I::If(BlockType::Empty));
    i(I::I32Const(0));
    i(I::LocalSet(2));
    i(I::End);
    i(I::End);
    // for i in 0..n { window[ptr + i] = data[i] }
    i(I::Block(BlockType::Empty));
    i(I::Loop(BlockType::Empty));
    i(I::LocalGet(3));
    i(I::LocalGet(2));
    i(I::I32GeU);
    i(I::BrIf(1));
    i(I::LocalGet(0));
    i(I::LocalGet(3));
    i(I::I32Add);
    i(I::LocalGet(3));
    i(I::I32Load8U(Emitter::mem_arg(u64::from(at), 0)));
    i(I::I32Store8(Emitter::mem_arg(u64::from(u.base), 0)));
    i(I::LocalGet(3));
    i(I::I32Const(1));
    i(I::I32Add);
    i(I::LocalSet(3));
    i(I::Br(0));
    i(I::End);
    i(I::End);
    i(I::I32Const(len as i32));
    i(I::End);
    f
}

/// Claims of all driver uses, the lock, then `init` of each driver (the code of `init`).
pub(super) fn init_drivers(
    f: &mut Function,
    uses: &[DriverUse],
    bus: u32,
    init_of: &[Option<u32>],
) {
    if uses.is_empty() {
        return;
    }
    for u in uses {
        for &(kind, value) in &u.claims {
            for v in [OP_CLAIM, kind, value, 0, 0] {
                f.instruction(&I::I32Const(v));
            }
            f.instruction(&I::Call(bus));
            f.instruction(&I::Drop);
        }
    }
    if uses
        .iter()
        .any(|u| u.shims.iter().any(|s| matches!(s, Shim::Bus(..))))
    {
        for v in [OP_LOCK, 0, 0, 0, 0] {
            f.instruction(&I::I32Const(v));
        }
        f.instruction(&I::Call(bus));
        f.instruction(&I::Drop);
    }
    for init in init_of.iter().flatten() {
        f.instruction(&I::Call(*init));
    }
}

/// Blocks with `block.wasm` whose text parameters are allowed: the displays take their model
/// from `crab.config`, the cell of the text parameter stays 0.
pub fn takes_text_params(kind: &str) -> bool {
    matches!(kind, "oled" | "lcd")
}

#[cfg(test)]
mod tests {
    use super::inspect_block_wasm;

    #[test]
    fn config_import_is_known() {
        let ok = wat::parse_str(
            r#"(module (import "crab" "config" (func (param i32 i32) (result i32)))
                 (func (export "step") (param i32)))"#,
        )
        .unwrap();
        assert!(!inspect_block_wasm(&ok).unwrap().driver);
        let bad = wat::parse_str(
            r#"(module (import "crab" "config" (func (param i32) (result i32)))
                 (func (export "step") (param i32)))"#,
        )
        .unwrap();
        assert!(inspect_block_wasm(&bad).unwrap_err().contains("сигнатур"));
    }
}

#[cfg(test)]
mod v2_tests {
    use super::inspect_block_wasm;
    use crate::wasm::driver::OLD_F64;

    fn wasm(imports: &str) -> Vec<u8> {
        wat::parse_str(format!(
            "(module {imports} (func (export \"step\") (param i32)))"
        ))
        .unwrap()
    }

    #[test]
    fn v2_imports_are_known_and_typed() {
        let ok = wasm(
            r#"(import "crab" "input_i" (func (param i32) (result i32)))
               (import "crab" "input_f" (func (param i32) (result f32)))
               (import "crab" "param_i" (func (param i32) (result i32)))
               (import "crab" "param_f" (func (param i32) (result f32)))
               (import "crab" "output_i" (func (param i32 i32)))
               (import "crab" "output_f" (func (param i32 f32)))
               (import "crab" "input_text" (func (param i32 i32 i32) (result i32)))
               (import "crab" "param_text" (func (param i32 i32 i32) (result i32)))
               (import "crab" "output_text" (func (param i32 i32 i32)))"#,
        );
        let info = inspect_block_wasm(&ok).unwrap();
        assert!(info.v2 && !info.driver);
        // `input_f` that returns f64 is the old interface: refused.
        let bad = wasm(r#"(import "crab" "input_f" (func (param i32) (result f64)))"#);
        assert!(inspect_block_wasm(&bad).unwrap_err().contains(OLD_F64));
        // The legacy module name has no imports any more.
        let flp = wasm(r#"(import "flp" "input_i" (func (param i32) (result i32)))"#);
        assert!(inspect_block_wasm(&flp).unwrap_err().contains(OLD_F64));
    }

    #[test]
    fn the_old_interface_is_refused() {
        for imports in [
            r#"(import "crab" "input" (func (param i32) (result f64)))"#,
            r#"(import "crab" "param" (func (param i32) (result f64)))"#,
            r#"(import "crab" "output" (func (param i32 f64)))"#,
            r#"(import "flp" "log" (func (param i32 i32)))"#,
            r#"(import "crab" "input" (func (param i32) (result f64)))
               (import "crab" "output_i" (func (param i32 i32)))"#,
        ] {
            let e = inspect_block_wasm(&wasm(imports)).unwrap_err();
            assert!(e.contains(OLD_F64), "{imports}: {e}");
        }
    }

    #[test]
    fn any_f64_is_refused() {
        // A body with an f64 instruction, no old imports.
        let op = wat::parse_str(
            r#"(module (func (export "step") (param i32)
                 (drop (f64.add (f64.const 1) (f64.const 2)))))"#,
        )
        .unwrap();
        assert!(inspect_block_wasm(&op).unwrap_err().contains(OLD_F64));
        // An f64 local that is never used.
        let local =
            wat::parse_str(r#"(module (func (export "step") (param i32) (local f64)))"#).unwrap();
        assert!(inspect_block_wasm(&local).unwrap_err().contains(OLD_F64));
        // An f64 global.
        let global = wat::parse_str(
            r#"(module (global (mut f64) (f64.const 0)) (func (export "step") (param i32)))"#,
        )
        .unwrap();
        assert!(inspect_block_wasm(&global).unwrap_err().contains(OLD_F64));
        // Without f64: fine.
        let ok =
            wat::parse_str(r#"(module (func (export "step") (param i32) (local f32)))"#).unwrap();
        assert!(inspect_block_wasm(&ok).is_ok());
    }
}
