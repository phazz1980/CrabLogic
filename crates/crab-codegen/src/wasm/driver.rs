//! Parsing of a block author's `block.wasm` and re-encoding of its functions so they can be pasted
//! into a bigger generated module (the sheet module). See `specs/wasm-drivers/plan.md`.
//!
//! The driver is a small module: one linear memory (its "window"), numeric globals, function
//! imports and the exports `step` / `init`. Everything that would need module-wide state we cannot
//! relocate (tables, indirect calls, `memory.grow`, bulk memory, start function) is rejected.

use wasm_encoder::reencode::{self, Reencode};
use wasm_encoder::{Function, ValType};
use wasmparser::{
    CompositeInnerType, DataKind, ExternalKind, FunctionBody, GlobalType, MemArg, Operator, Parser,
    Payload, TypeRef, Validator,
};

/// Largest memory window a driver may ask for: the sheet module has one page (64 KB) for its
/// strings and all its drivers.
const MAX_WINDOW: u32 = 32768;

/// A constant initial value of a driver global (floats as bits).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum ConstVal {
    I32(i32),
    I64(i64),
    F32(u32),
    F64(u64),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct DriverImport {
    pub module: String,
    pub name: String,
    /// Index into `Driver::types`.
    pub ty: u32,
}

pub(super) type FuncSig = (Vec<ValType>, Vec<ValType>);

pub(super) struct Driver {
    /// Function types of the driver (its whole type section), appended to the sheet module's types
    /// in this order.
    pub types: Vec<FuncSig>,
    /// Function imports in the driver's own import order.
    pub imports: Vec<DriverImport>,
    /// Type index (into `types`) of each defined function, in order.
    pub funcs: Vec<u32>,
    /// Globals: value type, mutable, initial value.
    pub globals: Vec<(ValType, bool, ConstVal)>,
    /// Active data segments: (offset inside the driver's memory window, bytes).
    pub data: Vec<(u32, Vec<u8>)>,
    /// Bytes of memory window the driver needs: exported `__heap_base` (rounded up to 16) if
    /// present, else the initial memory size in bytes; 0 if the module has no memory.
    pub window: u32,
    /// Index among defined functions of export `step`, type `(i32) -> ()`.
    pub step: u32,
    /// Same for the optional export `init`, type `() -> ()`.
    pub init: Option<u32>,
    /// Raw bodies (locals + code) of the defined functions.
    raw_bodies: Vec<Vec<u8>>,
    /// Helpers for `memory.copy` / `memory.fill` (they take addresses on the stack, which the
    /// window offset of the memargs does not move): defined functions after the driver's own,
    /// listed in `funcs` too.
    helpers: Vec<Helper>,
}

/// A function added to the driver: a bulk memory operation with its addresses moved into the
/// window. Type `(i32, i32, i32) -> ()`.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Helper {
    /// `(dst, src, n)`.
    Copy,
    /// `(dst, value, n)`.
    Fill,
}

/// Where the driver lands in the sheet module.
pub(super) struct Place {
    /// Final index of driver type 0.
    pub type_base: u32,
    /// Final function index for each driver import (same order as `Driver::imports`).
    pub import_map: Vec<u32>,
    /// Final function index of driver defined function 0.
    pub func_base: u32,
    /// Final global index of driver global 0.
    pub global_base: u32,
    /// Byte address of the driver's memory window in the sheet module's memory 0.
    pub mem_base: u32,
    /// Fuel counter: (final global index of the counter, final function index of `yield`, type
    /// `() -> i32`). When set, `bodies` inserts a fuel check at the start of every function and
    /// right after every `loop`, so an endless loop in a driver still lets the host run others.
    pub fuel: Option<(u32, u32)>,
}

fn val_type(t: wasmparser::ValType) -> Result<ValType, String> {
    match t {
        wasmparser::ValType::I32 => Ok(ValType::I32),
        wasmparser::ValType::I64 => Ok(ValType::I64),
        wasmparser::ValType::F32 => Ok(ValType::F32),
        wasmparser::ValType::F64 => Ok(ValType::F64),
        other => Err(format!(
            "block.wasm: тип значения {other} не поддерживается в драйвере (только i32, i64, f32, f64)"
        )),
    }
}

/// Evaluates a constant expression made of a single `*.const`.
fn const_expr(expr: &wasmparser::ConstExpr<'_>, what: &str) -> Result<ConstVal, String> {
    let mut reader = expr.get_operators_reader();
    let first = reader
        .read()
        .map_err(|e| format!("block.wasm: {what}: {e}"))?;
    let val = match first {
        Operator::I32Const { value } => ConstVal::I32(value),
        Operator::I64Const { value } => ConstVal::I64(value),
        Operator::F32Const { value } => ConstVal::F32(value.bits()),
        Operator::F64Const { value } => ConstVal::F64(value.bits()),
        other => {
            return Err(format!(
                "block.wasm: {what}: допустима только константа (i32/i64/f32/f64.const), найдено {other:?}"
            ))
        }
    };
    match reader.read() {
        Ok(Operator::End) => Ok(val),
        _ => Err(format!(
            "block.wasm: {what}: допустима только одна константа без вычислений"
        )),
    }
}

fn sig_is(sig: &FuncSig, params: &[ValType], results: &[ValType]) -> bool {
    sig.0 == params && sig.1 == results
}

fn perr(e: wasmparser::BinaryReaderError) -> String {
    format!("block.wasm: {e}")
}

pub(super) fn parse(wasm: &[u8]) -> Result<Driver, String> {
    Validator::new()
        .validate_all(wasm)
        .map_err(|e| format!("block.wasm: некорректный модуль WASM: {e}"))?;

    let mut types: Vec<FuncSig> = Vec::new();
    let mut imports: Vec<DriverImport> = Vec::new();
    let mut funcs: Vec<u32> = Vec::new();
    let mut globals: Vec<(ValType, bool, ConstVal)> = Vec::new();
    let mut data: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut raw_bodies: Vec<Vec<u8>> = Vec::new();
    let mut memory_pages: Option<u64> = None;
    let mut exports: Vec<(String, ExternalKind, u32)> = Vec::new();

    for payload in Parser::new(0).parse_all(wasm) {
        match payload.map_err(perr)? {
            Payload::TypeSection(r) => {
                for group in r {
                    for sub in group.map_err(perr)?.into_types() {
                        let CompositeInnerType::Func(f) = &sub.composite_type.inner else {
                            return Err(
                                "block.wasm: типы struct/array не поддерживаются в драйвере".into(),
                            );
                        };
                        let params = f
                            .params()
                            .iter()
                            .map(|t| val_type(*t))
                            .collect::<Result<Vec<_>, _>>()?;
                        let results = f
                            .results()
                            .iter()
                            .map(|t| val_type(*t))
                            .collect::<Result<Vec<_>, _>>()?;
                        types.push((params, results));
                    }
                }
            }
            Payload::ImportSection(r) => {
                for imp in r.into_imports() {
                    let imp = imp.map_err(perr)?;
                    match imp.ty {
                        TypeRef::Func(t) => imports.push(DriverImport {
                            module: imp.module.to_string(),
                            name: imp.name.to_string(),
                            ty: t,
                        }),
                        TypeRef::Memory(_) => {
                            return Err("block.wasm: импорт памяти не поддерживается: драйвер владеет своей памятью".into())
                        }
                        TypeRef::Table(_) => {
                            return Err(
                                "block.wasm: импорт таблицы не поддерживается в драйвере".into()
                            )
                        }
                        TypeRef::Global(_) => {
                            return Err("block.wasm: импорт глобальной переменной не поддерживается в драйвере".into())
                        }
                        _ => {
                            return Err(
                                "block.wasm: импорт этого вида не поддерживается в драйвере".into()
                            )
                        }
                    }
                }
            }
            Payload::FunctionSection(r) => {
                for t in r {
                    funcs.push(t.map_err(perr)?);
                }
            }
            Payload::TableSection(_) => {
                return Err("block.wasm: таблицы и косвенные вызовы (call_indirect) не поддерживаются в драйвере".into())
            }
            Payload::ElementSection(_) => {
                return Err(
                    "block.wasm: таблицы и секции element не поддерживаются в драйвере".into(),
                )
            }
            Payload::TagSection(_) => {
                return Err("block.wasm: исключения (tag) не поддерживаются в драйвере".into())
            }
            Payload::StartSection { .. } => {
                return Err("block.wasm: стартовая функция (start) не поддерживается: используйте экспорт init".into())
            }
            Payload::MemorySection(r) => {
                for m in r {
                    let m = m.map_err(perr)?;
                    if memory_pages.is_some() {
                        return Err(
                            "block.wasm: больше одной памяти не поддерживается в драйвере".into(),
                        );
                    }
                    if m.memory64 {
                        return Err("block.wasm: memory64 не поддерживается в драйвере".into());
                    }
                    if m.shared {
                        return Err(
                            "block.wasm: общая (shared) память не поддерживается в драйвере".into(),
                        );
                    }
                    memory_pages = Some(m.initial);
                }
            }
            Payload::GlobalSection(r) => {
                for g in r {
                    let g = g.map_err(perr)?;
                    let GlobalType {
                        content_type,
                        mutable,
                        shared,
                    } = g.ty;
                    if shared {
                        return Err(
                            "block.wasm: общие (shared) глобальные переменные не поддерживаются"
                                .into(),
                        );
                    }
                    let init = const_expr(&g.init_expr, "инициализатор глобальной переменной")?;
                    globals.push((val_type(content_type)?, mutable, init));
                }
            }
            Payload::ExportSection(r) => {
                for e in r {
                    let e = e.map_err(perr)?;
                    exports.push((e.name.to_string(), e.kind, e.index));
                }
            }
            Payload::DataSection(r) => {
                for d in r {
                    let d = d.map_err(perr)?;
                    match d.kind {
                        DataKind::Passive => {
                            return Err("block.wasm: пассивные сегменты данных не поддерживаются в драйвере".into())
                        }
                        DataKind::Active {
                            memory_index,
                            offset_expr,
                        } => {
                            if memory_index != 0 {
                                return Err("block.wasm: сегмент данных не в памяти 0".into());
                            }
                            let off = match const_expr(&offset_expr, "смещение сегмента данных")? {
                                ConstVal::I32(v) => v as u32,
                                _ => {
                                    return Err("block.wasm: смещение сегмента данных должно быть i32.const".into())
                                }
                            };
                            data.push((off, d.data.to_vec()));
                        }
                    }
                }
            }
            Payload::CodeSectionEntry(body) => raw_bodies.push(body.as_bytes().to_vec()),
            _ => {}
        }
    }

    if funcs.len() != raw_bodies.len() {
        return Err("block.wasm: число функций не совпадает с числом тел".into());
    }
    if imports.iter().any(|i| i.ty as usize >= types.len()) {
        return Err("block.wasm: импорт ссылается на несуществующий тип".into());
    }

    // Operator whitelist, checked once here.
    let (mut copies, mut fills) = (false, false);
    for raw in &raw_bodies {
        let body = FunctionBody::new(wasmparser::BinaryReader::new(raw, 0));
        let mut reader = body.get_operators_reader().map_err(perr)?;
        while !reader.eof() {
            match reader.read().map_err(perr)? {
                Operator::MemoryCopy {
                    dst_mem: 0,
                    src_mem: 0,
                } => copies = true,
                Operator::MemoryFill { mem: 0 } => fills = true,
                op => check_op(&op)?,
            }
        }
    }
    let mut helpers = Vec::new();
    if copies || fills {
        let ty = types.len() as u32;
        types.push((vec![ValType::I32; 3], vec![]));
        for (used, h) in [(copies, Helper::Copy), (fills, Helper::Fill)] {
            if used {
                funcs.push(ty);
                helpers.push(h);
            }
        }
    }

    // Memory window.
    let mem_bytes = memory_pages.map(|p| p.saturating_mul(65536));
    let mut heap_base = None;
    for (name, kind, idx) in &exports {
        if name == "__heap_base" && *kind == ExternalKind::Global {
            match globals.get(*idx as usize) {
                Some((ValType::I32, false, ConstVal::I32(v))) => heap_base = Some(*v as u32 as u64),
                _ => return Err(
                    "block.wasm: __heap_base должен быть неизменяемой глобальной i32 с константой"
                        .into(),
                ),
            }
        }
    }
    let window = match (mem_bytes, heap_base) {
        (None, _) => 0,
        (Some(_), Some(h)) => h.div_ceil(16) * 16,
        (Some(m), None) => m,
    };
    if window > MAX_WINDOW as u64 {
        return Err(format!(
            "block.wasm: окно памяти драйвера {window} байт больше допустимых {MAX_WINDOW}{}",
            if heap_base.is_none() {
                ": экспортируйте __heap_base (Rust: -C link-arg=--export=__heap_base, clang: -Wl,--export=__heap_base) и задайте небольшой стек (-zstack-size=4096)"
            } else {
                "; задайте меньший стек (-zstack-size=4096)"
            }
        ));
    }
    let window = window as u32;
    for (off, bytes) in &data {
        if *off as u64 + bytes.len() as u64 > window as u64 {
            return Err(format!(
                "block.wasm: сегмент данных по адресу {off} выходит за окно памяти ({window} байт)"
            ));
        }
    }

    // step / init.
    let n_imp = imports.len() as u32;
    let find = |want: &str| -> Result<Option<u32>, String> {
        for (name, kind, idx) in &exports {
            if name == want {
                if *kind != ExternalKind::Func {
                    return Err(format!("block.wasm: экспорт {want} должен быть функцией"));
                }
                if *idx < n_imp {
                    return Err(format!("block.wasm: экспорт {want} не может быть импортом"));
                }
                return Ok(Some(*idx - n_imp));
            }
        }
        Ok(None)
    };
    let step = find("step")?
        .ok_or_else(|| "block.wasm: нет экспорта step (функция (i32) -> ())".to_string())?;
    if !sig_is(&types[funcs[step as usize] as usize], &[ValType::I32], &[]) {
        return Err("block.wasm: step должна иметь сигнатуру (i32) -> ()".into());
    }
    let init = find("init")?;
    if let Some(i) = init {
        if !sig_is(&types[funcs[i as usize] as usize], &[], &[]) {
            return Err("block.wasm: init должна иметь сигнатуру () -> ()".into());
        }
    }

    Ok(Driver {
        types,
        imports,
        funcs,
        globals,
        data,
        window,
        step,
        init,
        raw_bodies,
        helpers,
    })
}

/// Whitelist of operators the driver may use.
fn check_op(op: &Operator<'_>) -> Result<(), String> {
    use Operator::*;
    let mem0 = |m: &MemArg| -> Result<(), String> {
        if m.target != wasmparser::MemArgTarget::Memory(0) {
            return Err("block.wasm: доступ к памяти не 0".into());
        }
        Ok(())
    };
    match op {
        CallIndirect { .. } | ReturnCallIndirect { .. } => Err(
            "block.wasm: таблицы и косвенные вызовы (call_indirect) не поддерживаются в драйвере"
                .into(),
        ),
        MemoryGrow { .. } | MemorySize { .. } => Err(
            "block.wasm: memory.grow и memory.size не поддерживаются: память драйвера фиксирована"
                .into(),
        ),
        MemoryCopy { .. } | MemoryFill { .. } | MemoryInit { .. } | DataDrop { .. } => Err(
            "block.wasm: memory.copy/fill/init и data.drop (bulk memory) не поддерживаются в драйвере; соберите без bulk-memory".into(),
        ),
        I32Load { memarg }
        | I64Load { memarg }
        | F32Load { memarg }
        | F64Load { memarg }
        | I32Load8S { memarg }
        | I32Load8U { memarg }
        | I32Load16S { memarg }
        | I32Load16U { memarg }
        | I64Load8S { memarg }
        | I64Load8U { memarg }
        | I64Load16S { memarg }
        | I64Load16U { memarg }
        | I64Load32S { memarg }
        | I64Load32U { memarg }
        | I32Store { memarg }
        | I64Store { memarg }
        | F32Store { memarg }
        | F64Store { memarg }
        | I32Store8 { memarg }
        | I32Store16 { memarg }
        | I64Store8 { memarg }
        | I64Store16 { memarg }
        | I64Store32 { memarg } => mem0(memarg),
        Unreachable | Nop | Block { .. } | Loop { .. } | If { .. } | Else | End | Br { .. }
        | BrIf { .. } | BrTable { .. } | Return | Call { .. } | Drop | Select
        | LocalGet { .. } | LocalSet { .. } | LocalTee { .. } | GlobalGet { .. }
        | GlobalSet { .. } | I32Const { .. } | I64Const { .. } | F32Const { .. }
        | F64Const { .. }
        // i32
        | I32Eqz | I32Eq | I32Ne | I32LtS | I32LtU | I32GtS | I32GtU | I32LeS | I32LeU | I32GeS
        | I32GeU | I32Clz | I32Ctz | I32Popcnt | I32Add | I32Sub | I32Mul | I32DivS | I32DivU
        | I32RemS | I32RemU | I32And | I32Or | I32Xor | I32Shl | I32ShrS | I32ShrU | I32Rotl
        | I32Rotr
        // i64
        | I64Eqz | I64Eq | I64Ne | I64LtS | I64LtU | I64GtS | I64GtU | I64LeS | I64LeU | I64GeS
        | I64GeU | I64Clz | I64Ctz | I64Popcnt | I64Add | I64Sub | I64Mul | I64DivS | I64DivU
        | I64RemS | I64RemU | I64And | I64Or | I64Xor | I64Shl | I64ShrS | I64ShrU | I64Rotl
        | I64Rotr
        // f32
        | F32Eq | F32Ne | F32Lt | F32Gt | F32Le | F32Ge | F32Abs | F32Neg | F32Ceil | F32Floor
        | F32Trunc | F32Nearest | F32Sqrt | F32Add | F32Sub | F32Mul | F32Div | F32Min | F32Max
        | F32Copysign
        // f64
        | F64Eq | F64Ne | F64Lt | F64Gt | F64Le | F64Ge | F64Abs | F64Neg | F64Ceil | F64Floor
        | F64Trunc | F64Nearest | F64Sqrt | F64Add | F64Sub | F64Mul | F64Div | F64Min | F64Max
        | F64Copysign
        // conversions
        | I32WrapI64 | I32TruncF32S | I32TruncF32U | I32TruncF64S | I32TruncF64U
        | I64ExtendI32S | I64ExtendI32U | I64TruncF32S | I64TruncF32U | I64TruncF64S
        | I64TruncF64U | F32ConvertI32S | F32ConvertI32U | F32ConvertI64S | F32ConvertI64U
        | F32DemoteF64 | F64ConvertI32S | F64ConvertI32U | F64ConvertI64S | F64ConvertI64U
        | F64PromoteF32 | I32ReinterpretF32 | I64ReinterpretF64 | F32ReinterpretI32
        | F64ReinterpretI64
        // sign extension and saturating truncation
        | I32Extend8S | I32Extend16S | I64Extend8S | I64Extend16S | I64Extend32S
        | I32TruncSatF32S | I32TruncSatF32U | I32TruncSatF64S | I32TruncSatF64U
        | I64TruncSatF32S | I64TruncSatF32U | I64TruncSatF64S | I64TruncSatF64U => Ok(()),
        other => Err(format!(
            "block.wasm: операция {other:?} не поддерживается в драйвере"
        )),
    }
}

fn user_err(msg: &str) -> reencode::Error<String> {
    reencode::Error::UserError(format!("block.wasm: {msg}"))
}

fn map_err(e: reencode::Error<String>) -> String {
    match e {
        reencode::Error::UserError(s) => s,
        other => format!("block.wasm: {other}"),
    }
}

/// Rewrites indices and memory offsets while re-encoding a body.
struct Relocator<'a> {
    place: &'a Place,
    n_imports: u32,
}

impl Reencode for Relocator<'_> {
    type Error = String;

    fn function_index(&mut self, func: u32) -> Result<u32, reencode::Error<String>> {
        if func < self.n_imports {
            self.place
                .import_map
                .get(func as usize)
                .copied()
                .ok_or_else(|| user_err("в Place нет номера для импорта драйвера"))
        } else {
            self.place
                .func_base
                .checked_add(func - self.n_imports)
                .ok_or_else(|| user_err("переполнение номера функции"))
        }
    }

    fn global_index(&mut self, global: u32) -> Result<u32, reencode::Error<String>> {
        self.place
            .global_base
            .checked_add(global)
            .ok_or_else(|| user_err("переполнение номера глобальной переменной"))
    }

    fn type_index(&mut self, ty: u32) -> Result<u32, reencode::Error<String>> {
        self.place
            .type_base
            .checked_add(ty)
            .ok_or_else(|| user_err("переполнение номера типа"))
    }

    fn mem_arg(
        &mut self,
        arg: wasmparser::MemArg,
    ) -> Result<wasm_encoder::MemArg, reencode::Error<String>> {
        if arg.target != wasmparser::MemArgTarget::Memory(0) {
            return Err(user_err("доступ к памяти не 0"));
        }
        let offset = arg.offset + self.place.mem_base as u64;
        if offset > u32::MAX as u64 {
            return Err(user_err("смещение доступа к памяти выходит за 4 ГБ"));
        }
        Ok(wasm_encoder::MemArg {
            offset,
            align: arg.align.into(),
            target: wasm_encoder::MemArgTarget::Memory(0),
            ordering: None,
        })
    }
}

/// `fuel -= 1; if fuel < 1 { fuel = yield() }`.
fn emit_fuel_check(f: &mut Function, global: u32, yield_fn: u32) {
    use wasm_encoder::Instruction as I;
    f.instruction(&I::GlobalGet(global));
    f.instruction(&I::I32Const(1));
    f.instruction(&I::I32Sub);
    f.instruction(&I::GlobalSet(global));
    f.instruction(&I::GlobalGet(global));
    f.instruction(&I::I32Const(1));
    f.instruction(&I::I32LtS);
    f.instruction(&I::If(wasm_encoder::BlockType::Empty));
    f.instruction(&I::Call(yield_fn));
    f.instruction(&I::GlobalSet(global));
    f.instruction(&I::End);
}

impl Driver {
    /// Re-encoded bodies of the defined functions, in order.
    pub(super) fn bodies(&self, place: &Place) -> Result<Vec<Function>, String> {
        if place.import_map.len() != self.imports.len() {
            return Err(
                "block.wasm: размер import_map не совпадает с числом импортов драйвера".into(),
            );
        }
        let mut rel = Relocator {
            place,
            n_imports: self.imports.len() as u32,
        };
        let mut out = Vec::with_capacity(self.funcs.len());
        let own = self.raw_bodies.len() as u32;
        let helper = |h: Helper| {
            let k = self.helpers.iter().position(|x| *x == h).unwrap_or(0) as u32;
            place.func_base + own + k
        };
        for raw in &self.raw_bodies {
            let body = FunctionBody::new(wasmparser::BinaryReader::new(raw, 0));
            let mut f = rel
                .new_function_with_parsed_locals(&body)
                .map_err(map_err)?;
            let mut reader = body.get_operators_reader().map_err(perr)?;
            if let Some((g, y)) = place.fuel {
                emit_fuel_check(&mut f, g, y);
            }
            while !reader.eof() {
                let ins = rel.parse_instruction(&mut reader).map_err(map_err)?;
                let is_loop = matches!(ins, wasm_encoder::Instruction::Loop(_));
                match ins {
                    wasm_encoder::Instruction::MemoryCopy { .. } => {
                        f.instruction(&wasm_encoder::Instruction::Call(helper(Helper::Copy)))
                    }
                    wasm_encoder::Instruction::MemoryFill(_) => {
                        f.instruction(&wasm_encoder::Instruction::Call(helper(Helper::Fill)))
                    }
                    ins => f.instruction(&ins),
                };
                if let (Some((g, y)), true) = (place.fuel, is_loop) {
                    emit_fuel_check(&mut f, g, y);
                }
            }
            out.push(f);
        }
        for &h in &self.helpers {
            out.push(helper_body(h, place.mem_base));
        }
        Ok(out)
    }
}

/// Body of a bulk memory helper: the addresses move into the window at `base`.
fn helper_body(h: Helper, base: u32) -> Function {
    use wasm_encoder::Instruction as I;
    let mut f = Function::new([]);
    let moved = |f: &mut Function, n: u32| {
        f.instruction(&I::LocalGet(n));
        f.instruction(&I::I32Const(base as i32));
        f.instruction(&I::I32Add);
    };
    moved(&mut f, 0);
    match h {
        Helper::Copy => {
            moved(&mut f, 1);
            f.instruction(&I::LocalGet(2));
            f.instruction(&I::MemoryCopy {
                src_mem: 0,
                dst_mem: 0,
            });
        }
        Helper::Fill => {
            f.instruction(&I::LocalGet(1));
            f.instruction(&I::LocalGet(2));
            f.instruction(&I::MemoryFill(0));
        }
    }
    f.instruction(&I::End);
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
    (module
      (import "crab" "input" (func $in (param i32) (result f64)))
      (import "crab" "output" (func $out (param i32 f64)))
      (import "bus" "i2c_read" (func $i2c (param i32 i32 i32) (result i32)))
      (memory 1)
      (global $sp (mut i32) (i32.const 1024))
      (global $hb i32 (i32.const HB))
      (export "__heap_base" (global $hb))
      (export "step" (func $step))
      (export "init" (func $init))
      (data (i32.const 1024) "abc")
      (func $helper (param i32) (result i32)
        global.get $sp
        local.get 0
        i32.add
        i32.load offset=8)
      (func $step (param i32)
        local.get 0
        local.get 0
        call $in
        call $out
        i32.const 1
        i32.const 2
        i32.const 3
        call $i2c
        call $helper
        drop)
      (func $init
        global.get $sp
        i32.const 5
        i32.store offset=4))
    "#;

    fn good(hb: u32) -> Vec<u8> {
        wat::parse_str(GOOD.replace("HB", &hb.to_string())).unwrap()
    }

    fn place() -> Place {
        Place {
            type_base: 20,
            import_map: vec![7, 8, 9],
            func_base: 50,
            global_base: 30,
            mem_base: 4096,
            fuel: None,
        }
    }

    fn ops(f: &Function) -> Vec<String> {
        // Wrap the body in a minimal module to decode it again.
        let mut m = wasm_encoder::Module::new();
        let mut t = wasm_encoder::TypeSection::new();
        t.ty().function([], []);
        m.section(&t);
        let mut fs = wasm_encoder::FunctionSection::new();
        fs.function(0);
        m.section(&fs);
        let mut c = wasm_encoder::CodeSection::new();
        c.function(f);
        m.section(&c);
        let bytes = m.finish();
        for p in Parser::new(0).parse_all(&bytes) {
            if let Payload::CodeSectionEntry(b) = p.unwrap() {
                let mut r = b.get_operators_reader().unwrap();
                let mut v = vec![];
                while !r.eof() {
                    v.push(format!("{:?}", r.read().unwrap()));
                }
                return v;
            }
        }
        unreachable!()
    }

    #[test]
    fn parses_fields() {
        let d = parse(&good(2000)).unwrap();
        assert_eq!(d.window, 2000);
        assert_eq!(parse(&good(2001)).unwrap().window, 2016);
        assert_eq!(d.imports.len(), 3);
        assert_eq!(d.imports[0].module, "crab");
        assert_eq!(d.imports[2].name, "i2c_read");
        assert_eq!(d.funcs.len(), 3);
        assert_eq!(d.step, 1);
        assert_eq!(d.init, Some(2));
        assert_eq!(d.data, vec![(1024, b"abc".to_vec())]);
        assert_eq!(d.globals[0], (ValType::I32, true, ConstVal::I32(1024)));
        assert_eq!(d.globals[1], (ValType::I32, false, ConstVal::I32(2000)));
        let sig = &d.types[d.imports[0].ty as usize];
        assert_eq!(sig, &(vec![ValType::I32], vec![ValType::F64]));
    }

    #[test]
    fn relocates_bodies() {
        let d = parse(&good(2000)).unwrap();
        let b = d.bodies(&place()).unwrap();
        assert_eq!(b.len(), 3);
        let helper = ops(&b[0]);
        assert!(helper
            .iter()
            .any(|s| s.starts_with("GlobalGet { global_index: 30 }")));
        assert!(helper
            .iter()
            .any(|s| s.contains("I32Load") && s.contains("offset: 4104")));
        let step = ops(&b[1]);
        assert!(step.contains(&"Call { function_index: 7 }".to_string()));
        assert!(step.contains(&"Call { function_index: 8 }".to_string()));
        assert!(step.contains(&"Call { function_index: 9 }".to_string()));
        assert!(step.contains(&"Call { function_index: 50 }".to_string()));
        let init = ops(&b[2]);
        assert!(init
            .iter()
            .any(|s| s.contains("I32Store") && s.contains("offset: 4100")));
    }

    #[test]
    fn fuel_check_after_loop_and_at_start() {
        let wasm = wat::parse_str(
            r#"(module (func $f (param i32) (loop $l br $l)) (export "step" (func $f)))"#,
        )
        .unwrap();
        let d = parse(&wasm).unwrap();
        let mut p = Place {
            type_base: 0,
            import_map: vec![],
            func_base: 5,
            global_base: 0,
            mem_base: 0,
            fuel: Some((77, 88)),
        };
        let v = ops(&d.bodies(&p).unwrap()[0]);
        let check = [
            "GlobalGet { global_index: 77 }",
            "I32Const { value: 1 }",
            "I32Sub",
            "GlobalSet { global_index: 77 }",
            "GlobalGet { global_index: 77 }",
            "I32Const { value: 1 }",
            "I32LtS",
            "If { blockty: Empty }",
            "Call { function_index: 88 }",
            "GlobalSet { global_index: 77 }",
            "End",
        ];
        assert_eq!(v[..11], check);
        assert!(v[11].starts_with("Loop"), "{v:?}");
        assert_eq!(v[12..23], check);
        assert!(v[23].starts_with("Br"), "{v:?}");
        p.fuel = None;
        assert_eq!(ops(&d.bodies(&p).unwrap()[0]).len(), 4);
    }

    #[test]
    fn offset_overflow_is_error() {
        let d = parse(&good(2000)).unwrap();
        let mut p = place();
        p.mem_base = u32::MAX;
        assert!(d.bodies(&p).is_err());
    }

    fn err_of(wat_src: &str) -> String {
        match parse(&wat::parse_str(wat_src).unwrap()) {
            Ok(_) => panic!("expected error"),
            Err(e) => e,
        }
    }

    #[test]
    fn rejects_call_indirect() {
        let e = err_of(
            r#"(module (type $t (func)) (table 1 funcref) (memory 1)
               (func $step (param i32) i32.const 0 call_indirect (type $t))
               (export "step" (func $step)))"#,
        );
        assert!(e.contains("call_indirect"), "{e}");
    }

    #[test]
    fn rejects_memory_grow() {
        let e = err_of(
            r#"(module (memory 1)
               (func $step (param i32) i32.const 1 memory.grow drop)
               (export "step" (func $step)))"#,
        );
        assert!(e.contains("memory.grow"), "{e}");
    }

    #[test]
    fn rejects_missing_step() {
        let e = err_of(r#"(module (func $f) (export "init" (func $f)))"#);
        assert!(e.contains("step"), "{e}");
    }

    #[test]
    fn rejects_wrong_step_signature() {
        let e = err_of(r#"(module (func $f) (export "step" (func $f)))"#);
        assert!(e.contains("step"), "{e}");
    }

    #[test]
    fn rejects_big_window() {
        let e = err_of(r#"(module (memory 2) (func $f (param i32)) (export "step" (func $f)))"#);
        assert!(e.contains("окно памяти"), "{e}");
    }

    #[test]
    fn rejects_data_with_global_offset() {
        let e = err_of(
            r#"(module (global $g i32 (i32.const 8)) (memory 1)
               (data (global.get $g) "x")
               (func $f (param i32)) (export "step" (func $f)))"#,
        );
        assert!(e.contains("смещение сегмента данных"), "{e}");
    }

    #[test]
    fn rejects_start_and_bulk() {
        let e = err_of(
            r#"(module (func $s) (start $s) (func $f (param i32)) (export "step" (func $f)))"#,
        );
        assert!(e.contains("start"), "{e}");
        let e = err_of(
            r#"(module (memory 1) (data $d "ab")
               (func $f (param i32) i32.const 0 i32.const 0 i32.const 2 memory.init $d)
               (export "step" (func $f)))"#,
        );
        assert!(e.contains("bulk") || e.contains("пассив"), "{e}");
    }

    #[test]
    fn copy_and_fill_go_through_helpers_in_the_window() {
        let wasm = wat::parse_str(
            r#"(module (memory 1) (global i32 (i32.const 64)) (export "__heap_base" (global 0))
               (func $f (param i32)
                 i32.const 0 i32.const 8 i32.const 4 memory.copy
                 i32.const 16 i32.const 7 i32.const 4 memory.fill)
               (export "step" (func $f)))"#,
        )
        .unwrap();
        let d = parse(&wasm).unwrap();
        // The step and two helpers of type (i32, i32, i32) -> ().
        assert_eq!(d.funcs.len(), 3);
        assert_eq!(
            d.types[d.funcs[1] as usize],
            (vec![ValType::I32; 3], vec![])
        );
        let mut p = place();
        p.import_map = vec![];
        let bodies = d.bodies(&p).unwrap();
        assert_eq!(bodies.len(), 3);
        let step = ops(&bodies[0]).join(" ");
        assert!(
            step.contains(&format!("Call {{ function_index: {} }}", p.func_base + 1)),
            "{step}"
        );
        assert!(
            step.contains(&format!("Call {{ function_index: {} }}", p.func_base + 2)),
            "{step}"
        );
        assert!(!step.contains("MemoryCopy"), "{step}");
        let copy = ops(&bodies[1]).join(" ");
        assert!(
            copy.contains("MemoryCopy")
                && copy.contains(&format!("I32Const {{ value: {} }}", p.mem_base)),
            "{copy}"
        );
    }

    #[test]
    fn no_memory_means_zero_window() {
        let d = parse(
            &wat::parse_str(r#"(module (func $f (param i32)) (export "step" (func $f)))"#).unwrap(),
        )
        .unwrap();
        assert_eq!(d.window, 0);
        assert!(d.init.is_none());
    }
}
