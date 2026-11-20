//! LoRa in modules (`specs/lora-e220`): the `crab.config` blob of every `lora_e220` block and the
//! layout of the extra driver cells the generator and the driver (`drivers/lora_e220`) agree on.
//!
//! `lora_point` (on the centre: a slave) and `lora_centre` (on a slave: the centre) blocks name the
//! modem in their `modem` parameter; they emit no code, the modem's driver serves them.
//! Extra input cells of the driver: the values sent to the partners, partner after partner (the
//! commands on the centre, the answers on a slave). Extra output cells: for every partner the
//! values received from it, then one flag cell (`online` on the centre, `linked` on a slave).
//! Cells are 4 bytes: `bool`/`int` as `i32`, `float` as `f32`.

use std::collections::BTreeMap;

use crab_core::arrays::ElemType;
use crab_core::ParamValue;
use lora_e220::frame::{Ty as LTy, MAX_VALUES};
use lora_e220::master::MAX_PEERS;

use super::*;

/// One partner block of a modem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PointPlan {
    pub block: String,
    pub node: u8,
    /// Types of the values sent to the partner.
    pub tx: Vec<LTy>,
    /// Types of the values received from it.
    pub rx: Vec<LTy>,
    /// Structure that holds the values sent to the partner (else they come from the block ports).
    pub tx_struct: Option<String>,
    /// Structure that takes the values received from the partner.
    pub rx_struct: Option<String>,
}

/// What a `lora_e220` block serves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LoraPlan {
    pub centre: bool,
    pub points: Vec<PointPlan>,
}

impl LoraPlan {
    /// Prefix of the ports that carry the values sent to the partner.
    pub(super) fn tx_prefix(&self) -> &'static str {
        if self.centre {
            "cmd"
        } else {
            "ans"
        }
    }

    /// Prefix of the ports that carry the received values.
    pub(super) fn rx_prefix(&self) -> &'static str {
        if self.centre {
            "ans"
        } else {
            "cmd"
        }
    }

    /// The flag output of the partner block.
    pub(super) fn flag(&self) -> &'static str {
        if self.centre {
            "online"
        } else {
            "linked"
        }
    }
}

/// Port name of value `k` of type `ty` (`cmd0b`, `ans3f`).
pub(super) fn port_name(prefix: &str, k: usize, ty: LTy) -> String {
    let suffix = match ty {
        LTy::Bool => 'b',
        LTy::Int => 'i',
        LTy::Float => 'f',
    };
    format!("{prefix}{k}{suffix}")
}

fn num(i: &Instance, name: &str, default: f64) -> f64 {
    match i.params.get(name) {
        Some(ParamValue::Int(n)) => *n as f64,
        Some(ParamValue::Float(f)) => *f,
        Some(ParamValue::Bool(b)) => f64::from(u8::from(*b)),
        _ => default,
    }
}

fn text(i: &Instance, name: &str) -> String {
    match i.params.get(name) {
        Some(ParamValue::Text(t)) => t.clone(),
        _ => String::new(),
    }
}

/// Type of a structure field on the radio: `byte` goes as an integer.
fn field_type(ty: ElemType) -> LTy {
    match ty {
        ElemType::Bool => LTy::Bool,
        ElemType::Float => LTy::Float,
        _ => LTy::Int,
    }
}

/// The structure that carries the values `what` (`cmd` or `ans`) of a partner block: the value
/// types are then the field types, in order.
fn value_struct<'a>(i: &'a Instance, what: &str) -> Option<&'a str> {
    match i.params.get(format!("{what}_struct").as_str()) {
        Some(ParamValue::Text(t)) if !t.is_empty() => Some(t),
        _ => None,
    }
}

/// The types of the values `what` of a partner block: the fields of its structure, or `n_<what>`
/// values `t_<what><k>`.
fn types(logic: &crab_core::Logic, i: &Instance, what: &str) -> Vec<LTy> {
    if let Some(name) = value_struct(i, what) {
        return logic
            .struct_var(name)
            .map(|st| {
                st.fields
                    .iter()
                    .take(MAX_VALUES)
                    .map(|f| field_type(f.ty))
                    .collect()
            })
            .unwrap_or_default();
    }
    let n = num(i, &format!("n_{what}"), 0.0).clamp(0.0, MAX_VALUES as f64) as usize;
    (0..n)
        .map(|k| match num(i, &format!("t_{what}{k}"), 1.0) as i64 {
            0 => LTy::Bool,
            2 => LTy::Float,
            _ => LTy::Int,
        })
        .collect()
}

impl Program<'_> {
    /// Plan of modem block `id`; `None` when it is not a `lora_e220` block.
    pub(super) fn lora_plan(&self, id: &str) -> Option<LoraPlan> {
        let modem = self
            .blocks
            .iter()
            .find(|i| i.block.id == id && i.block.tag.is_none())?;
        if modem.block.kind != "lora_e220" {
            return None;
        }
        let centre = num(modem, "role", 0.0) < 0.5;
        let kind = if centre { "lora_point" } else { "lora_centre" };
        let mut points = Vec::new();
        for block in &self.project.logic.blocks {
            if block.kind != kind || block.tag.is_some() {
                continue;
            }
            let Some(p) = self.blocks.iter().find(|i| i.block.id == block.id) else {
                continue;
            };
            if text(p, "modem") != id {
                continue;
            }
            let logic = &self.project.logic;
            let (cmd, ans) = (types(logic, p, "cmd"), types(logic, p, "ans"));
            let (cmd_struct, ans_struct) = (
                value_struct(p, "cmd").map(str::to_owned),
                value_struct(p, "ans").map(str::to_owned),
            );
            let ((tx, rx), (tx_struct, rx_struct)) = if centre {
                ((cmd, ans), (cmd_struct, ans_struct))
            } else {
                ((ans, cmd), (ans_struct, cmd_struct))
            };
            points.push(PointPlan {
                block: block.id.clone(),
                node: if centre { num(p, "node", 0.0) as u8 } else { 0 },
                tx,
                rx,
                tx_struct,
                rx_struct,
            });
        }
        // A slave serves one centre; the centre at most MAX_PEERS slaves.
        points.truncate(if centre { MAX_PEERS } else { 1 });
        Some(LoraPlan { centre, points })
    }

    /// The `crab.config` blob of every `lora_e220` block of the project, by block id. A modem
    /// without partner blocks gets none (its driver stays idle, the schema check says why).
    pub fn lora_configs(&self) -> BTreeMap<String, Vec<u8>> {
        self.blocks
            .iter()
            .filter(|i| i.block.tag.is_none())
            .filter_map(|i| {
                let plan = self.lora_plan(&i.block.id)?;
                if plan.points.is_empty() {
                    return None;
                }
                let peers: Vec<_> = plan
                    .points
                    .iter()
                    .map(|p| (p.node, p.tx.clone(), p.rx.clone()))
                    .collect();
                Some((
                    i.block.id.clone(),
                    lora_e220::config::encode(plan.centre, &peers),
                ))
            })
            .collect()
    }
}

/// Blocks that emit no code: the modem's driver serves them.
pub(super) fn is_inert(kind: &str) -> bool {
    kind == "lora_point" || kind == "lora_centre"
}

/// The partner blocks of a modem as the emitter feeds and reads them.
pub(super) struct LoraExtras<'a, 'p> {
    pub plan: LoraPlan,
    /// For every partner of the plan: the block instance on this sheet.
    pub points: Vec<&'a Instance<'p>>,
}

impl LoraExtras<'_, '_> {
    pub(super) fn n_in(&self) -> u32 {
        self.plan.points.iter().map(|p| p.tx.len() as u32).sum()
    }

    pub(super) fn n_out(&self) -> u32 {
        self.plan.points.iter().map(|p| p.rx.len() as u32 + 1).sum()
    }
}

impl<'a, 'p> Emitter<'a, 'p> {
    /// The partner blocks of modem `inst`; `None` for any other block.
    pub(super) fn lora_extras(
        &mut self,
        inst: &Instance,
    ) -> Result<Option<LoraExtras<'a, 'p>>, WasmError> {
        let program = self.program;
        let Some(plan) = program.lora_plan(&inst.block.id) else {
            return Ok(None);
        };
        let module = self.module;
        let mut points = Vec::new();
        for p in &plan.points {
            let found = module
                .blocks
                .iter()
                .map(|&bi| &program.blocks[bi])
                .find(|i| i.block.id == p.block);
            match found {
                Some(i) => points.push(i),
                None => {
                    return Err(self.unsupported(
                        inst,
                        &format!(
                            "блок обмена «{}» должен стоять на том же листе, что и модем",
                            p.block
                        ),
                    ))
                }
            }
        }
        Ok(Some(LoraExtras { plan, points }))
    }

    /// Stores the values for the partners into the extra input cells (`first` is the address of
    /// the first one): a port that is not wired reads its default.
    pub(super) fn lora_inputs(&mut self, l: &LoraExtras, first: u32) -> Result<(), WasmError> {
        let mut at = first;
        for (p, inst) in l.plan.points.iter().zip(&l.points) {
            let slots = match &p.tx_struct {
                Some(name) => self.struct_mem(inst, name)?,
                None => Vec::new(),
            };
            for (k, ty) in p.tx.iter().enumerate() {
                if let Some(mem) = slots.get(k) {
                    // The value is the field of the structure.
                    self.c_i(at as i32);
                    self.c_i(0);
                    self.array_load(*mem);
                    self.store_cell(mem.ty_of());
                    at += CELL;
                    continue;
                }
                let port = port_name(l.plan.tx_prefix(), k, *ty);
                if inst.inputs.contains_key(port.as_str()) {
                    self.c_i(at as i32);
                    let t = self.operand(inst, &port);
                    self.store_cell(t);
                }
                at += CELL;
            }
        }
        Ok(())
    }

    /// After the driver's step: the received values and the flags go to the partner blocks.
    pub(super) fn lora_outputs(&mut self, l: &LoraExtras, first: u32) -> Result<(), WasmError> {
        let mut at = first;
        for (p, inst) in l.plan.points.iter().zip(&l.points) {
            let def = &inst.lib.def;
            let slots = match &p.rx_struct {
                Some(name) => self.struct_mem(inst, name)?,
                None => Vec::new(),
            };
            for (k, ty) in p.rx.iter().enumerate() {
                if let Some(mem) = slots.get(k) {
                    // The received value goes into the field of the structure.
                    self.c_i(0);
                    self.c_i(at as i32);
                    if *ty == LTy::Float {
                        self.op(I::F32Load(Self::mem_arg(0, 2)));
                    } else {
                        self.op(I::I32Load(Self::mem_arg(0, 2)));
                    }
                    self.array_store(*mem);
                    at += CELL;
                    continue;
                }
                let port = port_name(l.plan.rx_prefix(), k, *ty);
                if let Some(i) = def.outputs.iter().position(|o| o.name == port) {
                    if inst.outputs.contains_key(port.as_str()) {
                        self.c_i(at as i32);
                        let t = if *ty == LTy::Float {
                            self.op(I::F32Load(Self::mem_arg(0, 2)));
                            Ty::F
                        } else {
                            self.op(I::I32Load(Self::mem_arg(0, 2)));
                            Ty::I
                        };
                        self.set_out(inst, i, t);
                    }
                }
                at += CELL;
            }
            let flag = l.plan.flag();
            if let Some(i) = def.outputs.iter().position(|o| o.name == flag) {
                if inst.outputs.contains_key(flag) {
                    self.c_i(at as i32);
                    self.op(I::I32Load(Self::mem_arg(0, 2)));
                    self.set_out(inst, i, Ty::I);
                }
            }
            at += CELL;
        }
        Ok(())
    }
}

/// Cell size (as in `user_block`).
const CELL: u32 = 4;
