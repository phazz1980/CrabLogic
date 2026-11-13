//! Blocks with their own `block.wasm` in a sheet module (`specs/wasm-drivers`): the module is
//! pasted into the sheet module (`driver.rs`), one copy per block instance with its own memory
//! window, globals and value cells. Its imports become small functions of the sheet module
//! ("shims"): `crab.input/param/output` read and write the cells, `crab.log` and `bus.*` call
//! the master (`crab.log`, `crab.bus`) with pointers moved into the window.

use std::rc::Rc;

use super::driver::{self, Driver};
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
    Input,
    Param,
    Output,
    Log,
    Config,
    Bus(i32, &'static [Arg]),
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
];

/// The shim of an import, or why the driver may not have it.
fn shim_of(
    module: &str,
    name: &str,
    params: &[ValType],
    results: &[ValType],
) -> Result<Shim, String> {
    use ValType::{F64, I32};
    let (shim, want_p, want_r): (Shim, Vec<ValType>, Vec<ValType>) = match (module, name) {
        ("crab" | "flp", "input") => (Shim::Input, vec![I32], vec![F64]),
        ("crab" | "flp", "param") => (Shim::Param, vec![I32], vec![F64]),
        ("crab" | "flp", "output") => (Shim::Output, vec![I32, F64], vec![]),
        ("crab" | "flp", "log") => (Shim::Log, vec![I32, I32], vec![]),
        ("crab" | "flp", "config") => (Shim::Config, vec![I32, I32], vec![I32]),
        (crab_sim_bus::MODULE, _) => match BUS_IMPORTS.iter().find(|(n, ..)| *n == name) {
            Some(&(_, n, shim)) => (shim, vec![I32; n], vec![I32]),
            None => return Err(format!("нет такой функции шины: bus.{name}")),
        },
        _ => {
            return Err(format!(
                "импорт {module}.{name} недоступен драйверу (только crab.input/param/output/log/config и bus.*)"
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
}

/// Parses `block.wasm` and checks every import the way the sheet module generator does.
pub(crate) fn inspect_block_wasm(bytes: &[u8]) -> Result<WasmInfo, String> {
    let d = driver::parse(bytes)?;
    let mut driver = false;
    for imp in &d.imports {
        let (params, results) = &d.types[imp.ty as usize];
        if let Shim::Bus(..) = shim_of(&imp.module, &imp.name, params, results)? {
            driver = true;
        }
    }
    Ok(WasmInfo { driver })
}

/// Largest per-instance configuration a block may get through `crab.config`.
pub const MAX_BLOCK_CONFIG: usize = 4096;

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
    /// `f64` cells of the inputs, parameters and outputs.
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
        // A display takes its kind and size from `crab.config`; its text parameter (`model`)
        // keeps a zero cell (the indices of the other parameters do not move).
        let is_display = takes_text_params(&inst.block.kind);
        let text_param = def
            .params
            .iter()
            .find(|p| p.kind == ParamKind::Text && !is_display);
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
        let extra_in = display.as_ref().map_or(0, |d| d.n_in());
        let extra_out = display.as_ref().map_or(0, |d| d.n_out());
        let n_in = def.inputs.len() as u32 + extra_in;
        let n_param = def.params.len() as u32;
        let n_out = def.outputs.len() as u32 + extra_out;
        let cells_in = self.alloc_aligned(8 * n_in.max(1), 8);
        let cells_param = self.alloc_aligned(8 * n_param.max(1), 8);
        let cells_out = self.alloc_aligned(8 * n_out.max(1), 8);
        // Constant parameters are in their cells from the start (a data segment), so the
        // driver's `init` already sees its pins and address.
        for i in 0..def.params.len() {
            if let Ok(v) = self.const_param(inst, i) {
                let at = cells_param + 8 * i as u32;
                self.data.push((at, v.to_le_bytes().to_vec()));
            }
        }

        // Inputs and parameters go to their cells every step (a parameter may come from a tag).
        let f64_store = I::F64Store(Self::mem_arg(0, 3));
        for i in 0..def.inputs.len() {
            self.c_i((cells_in + 8 * i as u32) as i32);
            let ty = self.input(inst, i);
            self.op(to_f64(ty));
            self.op(f64_store.clone());
        }
        for i in 0..def.params.len() {
            if def.params[i].kind == ParamKind::Text {
                continue;
            }
            self.c_i((cells_param + 8 * i as u32) as i32);
            let ty = self.param(inst, i);
            self.op(to_f64(ty));
            self.op(f64_store.clone());
        }
        if let Some(d) = &display {
            self.display_inputs(d, cells_in + 8 * def.inputs.len() as u32);
        }
        let k = self.drivers.len() as u32;
        self.op(I::LocalGet(0));
        self.op(I::Call(DRIVER_STEP + k));
        for (i, port) in def.outputs.iter().enumerate() {
            if !inst.outputs.contains_key(port.name.as_str()) {
                continue;
            }
            self.c_i((cells_out + 8 * i as u32) as i32);
            self.op(I::F64Load(Self::mem_arg(0, 3)));
            let ty = if port.ty == SignalType::Float {
                self.op(I::F32DemoteF64);
                Ty::F
            } else {
                self.op(I::I32TruncSatF64S);
                Ty::I
            };
            self.set_out(inst, i, ty);
        }
        if let Some(d) = &display {
            self.display_outputs(d, cells_out + 8 * def.outputs.len() as u32);
        }
        self.drivers.push(DriverUse {
            driver: d,
            shims,
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

/// What a display block gets besides its own ports (see `display.rs`).
struct DisplayExtras<'a, 'p> {
    /// The widget input slots: the instance that feeds the widget on this sheet, if any.
    widgets: Vec<Option<&'a Instance<'p>>>,
    menu: Option<MenuExtras<'a, 'p>>,
}

struct MenuExtras<'a, 'p> {
    inst: &'a Instance<'p>,
    vars: Vec<super::display::MenuVar>,
}

impl DisplayExtras<'_, '_> {
    fn n_in(&self) -> u32 {
        self.widgets.len() as u32 + self.menu.as_ref().map_or(0, |m| 4 + m.vars.len() as u32)
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
        let widgets = plan
            .widgets
            .iter()
            .map(|w| {
                let all: Vec<&Instance> = on_sheet(w.clone())
                    .filter(|i| i.inputs.contains_key("in"))
                    .collect();
                all.iter()
                    .copied()
                    .find(|i| in_linked(i))
                    .or(all.first().copied())
            })
            .collect();
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
        Ok(Some(DisplayExtras { widgets, menu }))
    }

    /// Stores the widget values, the menu buttons and the menu variables in the input cells
    /// after the block's own ports (`first` is the address of the first extra cell).
    fn display_inputs(&mut self, d: &DisplayExtras, first: u32) {
        let f64_store = I::F64Store(Self::mem_arg(0, 3));
        let mut cell = first;
        for w in d.widgets.iter().copied() {
            if let Some(w) = w {
                self.c_i(cell as i32);
                let ty = self.operand(w, "in");
                self.op(to_f64(ty));
                self.op(f64_store.clone());
            }
            cell += 8;
        }
        let Some(menu) = &d.menu else {
            return;
        };
        for button in ["up", "down", "ok", "back"] {
            if menu.inst.inputs.contains_key(button) {
                self.c_i(cell as i32);
                let ty = self.operand(menu.inst, button);
                self.op(to_f64(ty));
                self.op(f64_store.clone());
            }
            cell += 8;
        }
        for v in &menu.vars {
            self.c_i(cell as i32);
            let ty = self.var_read(&v.name);
            self.op(to_f64(ty));
            self.op(f64_store.clone());
            cell += 8;
        }
    }

    /// After the driver's step: the menu outputs go to the `menu` block, a confirmed variable
    /// is written (and saved at once when it is retained).
    fn display_outputs(&mut self, d: &DisplayExtras, first: u32) {
        let Some(menu) = &d.menu else {
            return;
        };
        let load = |e: &mut Self, cell: u32| {
            e.c_i(cell as i32);
            e.op(I::F64Load(Self::mem_arg(0, 3)));
        };
        let def = &menu.inst.lib.def;
        for (k, port) in ["open", "item", "changed"].into_iter().enumerate() {
            let Some(i) = def.outputs.iter().position(|p| p.name == port) else {
                continue;
            };
            if !menu.inst.outputs.contains_key(port) {
                continue;
            }
            load(self, first + 8 * k as u32);
            self.op(I::I32TruncSatF64S);
            self.set_out(menu.inst, i, Ty::I);
        }
        let slot = first + 8 * 3;
        for (v, var) in menu.vars.iter().enumerate() {
            load(self, slot);
            self.op(I::F64Const((v as f64).into()));
            self.op(I::F64Eq);
            if var.action {
                // A pulse of one step.
                self.var_write(&var.name, Ty::I);
                continue;
            }
            self.op(I::If(BlockType::Empty));
            load(self, slot + 8 * (1 + v as u32));
            let ty = self.var_ty(&var.name);
            self.op(if ty == Ty::F {
                I::F32DemoteF64
            } else {
                I::I32TruncSatF64S
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

fn to_f64(ty: Ty) -> I<'static> {
    match ty {
        Ty::I => I::F64ConvertI32S,
        Ty::F => I::F64PromoteF32,
    }
}

/// Body of the shim of one driver import. `bus` and `log` are the final indices of the
/// master's imports (or their AOT wrappers).
pub(super) fn shim_body(shim: Shim, u: &DriverUse, bus: u32, log: u32) -> Function {
    let mut f = Function::new([]);
    let arg = Emitter::mem_arg(0, 3);
    // `index < n ? cell(index) : 0`, the same for inputs and parameters.
    let read = |f: &mut Function, cells: u32, n: u32| {
        f.instruction(&I::LocalGet(0));
        f.instruction(&I::I32Const(n as i32));
        f.instruction(&I::I32LtU);
        f.instruction(&I::If(BlockType::Result(ValType::F64)));
        f.instruction(&I::LocalGet(0));
        f.instruction(&I::I32Const(8));
        f.instruction(&I::I32Mul);
        f.instruction(&I::F64Load(MemArg {
            offset: u64::from(cells),
            ..arg
        }));
        f.instruction(&I::Else);
        f.instruction(&I::F64Const(0.0.into()));
        f.instruction(&I::End);
    };
    match shim {
        Shim::Input => read(&mut f, u.cells_in, u.n_in),
        Shim::Param => read(&mut f, u.cells_param, u.n_param),
        Shim::Output => {
            f.instruction(&I::LocalGet(0));
            f.instruction(&I::I32Const(u.n_out as i32));
            f.instruction(&I::I32LtU);
            f.instruction(&I::If(BlockType::Empty));
            f.instruction(&I::LocalGet(0));
            f.instruction(&I::I32Const(8));
            f.instruction(&I::I32Mul);
            f.instruction(&I::LocalGet(1));
            f.instruction(&I::F64Store(MemArg {
                offset: u64::from(u.cells_out),
                ..arg
            }));
            f.instruction(&I::End);
        }
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
