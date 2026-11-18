//! Register map of a slave project: the cells of the four Modbus tables that its "Modbus write"
//! and "Modbus read" blocks take. Pure functions over [`Logic`]; the check that reports conflicts
//! is in `check::modbus`.

use crate::{Logic, ParamValue};

/// Cells one tag takes in its table. Coil and discrete tables are bits: one cell each. Holding
/// and input registers take one register for `int` and `bool`, two for `float`.
pub fn slot_size(table: &str, kind: &str) -> usize {
    match table {
        "coil" | "discrete" => 1,
        _ if kind == "float" => 2,
        _ => 1,
    }
}

/// One register of the project as it sits in its table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    /// Id of the block.
    pub id: String,
    pub table: String,
    /// `int`, `float` or `bool`, as the block says it (`int` when it does not).
    pub kind: String,
    pub address: u16,
    /// Cells taken, see [`slot_size`].
    pub size: usize,
    /// The block is a "Modbus write" block: the schema writes the value, the master reads it.
    pub from_schema: bool,
}

/// A problem of the register map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModbusConflict {
    /// Two tags of one table share a cell. `address` is the first shared cell.
    Overlap {
        table: String,
        first: String,
        second: String,
        address: u16,
    },
    /// The value type does not fit the table: a non-bool tag in a coil or discrete table, or a
    /// bool tag in a holding or input table.
    BadKind {
        id: String,
        table: String,
        kind: String,
    },
}

impl Logic {
    /// One slot per "Modbus read" / "Modbus write" block, in block order. The address is the
    /// `reg` parameter, clamped to the 16-bit range.
    pub fn modbus_slots(&self) -> Vec<Slot> {
        self.blocks
            .iter()
            .filter_map(|b| {
                let from_schema = match b.kind.as_str() {
                    "modbus_write" => true,
                    "modbus_read" => false,
                    _ => return None,
                };
                let text = |name: &str, default: &str| match b.params.get(name) {
                    Some(ParamValue::Text(t)) => t.clone(),
                    _ => default.to_owned(),
                };
                let table = text("table", if from_schema { "input" } else { "holding" });
                let kind = text("kind", "int");
                let address = match b.params.get("reg") {
                    Some(ParamValue::Int(n)) => {
                        u16::try_from((*n).clamp(0, i64::from(u16::MAX))).unwrap_or(0)
                    }
                    _ => 0,
                };
                Some(Slot {
                    id: b.id.clone(),
                    size: slot_size(&table, &kind),
                    table,
                    kind,
                    address,
                    from_schema,
                })
            })
            .collect()
    }

    /// The address right after the farthest occupied cell of `table`, 0 when the table is empty.
    /// Holes left by deleted tags are not reused. Saturates at `u16::MAX`. `kind` is accepted for
    /// the call site: the next address does not depend on the type of the new tag.
    pub fn modbus_next_free(&self, table: &str, _kind: &str) -> u16 {
        let end = self
            .modbus_slots()
            .iter()
            .filter(|s| s.table == table)
            .map(|s| usize::from(s.address) + s.size)
            .max()
            .unwrap_or(0);
        u16::try_from(end).unwrap_or(u16::MAX)
    }

    /// Size of the holding, input, coil and discrete tables in that order: the farthest occupied
    /// cell of each, 0 when it is empty.
    pub fn modbus_table_sizes(&self) -> [usize; 4] {
        let slots = self.modbus_slots();
        ["holding", "input", "coil", "discrete"].map(|table| {
            slots
                .iter()
                .filter(|s| s.table == table)
                .map(|s| usize::from(s.address) + s.size)
                .max()
                .unwrap_or(0)
        })
    }

    /// Overlapping tags (each pair once) and value types that do not fit their table.
    pub fn modbus_conflicts(&self) -> Vec<ModbusConflict> {
        let slots = self.modbus_slots();
        let mut conflicts = Vec::new();
        for (i, a) in slots.iter().enumerate() {
            for b in &slots[i + 1..] {
                if a.table != b.table {
                    continue;
                }
                let (a_start, b_start) = (usize::from(a.address), usize::from(b.address));
                if a_start < b_start + b.size && b_start < a_start + a.size {
                    conflicts.push(ModbusConflict::Overlap {
                        table: a.table.clone(),
                        first: a.id.clone(),
                        second: b.id.clone(),
                        address: a.address.max(b.address),
                    });
                }
            }
        }
        for s in &slots {
            let bad = match s.table.as_str() {
                "coil" | "discrete" => s.kind != "bool",
                "holding" | "input" => s.kind == "bool",
                _ => false,
            };
            if bad {
                conflicts.push(ModbusConflict::BadKind {
                    id: s.id.clone(),
                    table: s.table.clone(),
                    kind: s.kind.clone(),
                });
            }
        }
        conflicts
    }
}
