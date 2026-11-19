//! Register map of Modbus tags. A `modbus_tag` block belongs to an owner given by its `device`
//! parameter: a `modbus_device` block (a slave of a master) or a `modbus_rtu_slave` /
//! `modbus_tcp_slave` block (the slave role of this board). Every owner has its own four tables.
//! Pure functions over [`Logic`]; the diagnostics are in `check::modbus`.

use crate::{Block, Logic, ParamValue};

/// Cells one tag takes in its table. Coil and discrete tables are bits: one cell each. Holding
/// and input registers take one register for `int` and `bool`, two for `float`.
pub fn slot_size(table: &str, kind: &str) -> usize {
    match table {
        "coil" | "discrete" => 1,
        _ if kind == "float" => 2,
        _ => 1,
    }
}

/// Text parameter of a block, `None` when it is missing or not text.
pub(crate) fn text_param<'a>(block: &'a Block, name: &str) -> Option<&'a str> {
    match block.params.get(name) {
        Some(ParamValue::Text(t)) => Some(t.as_str()),
        _ => None,
    }
}

/// Integer parameter of a block, `None` when it is missing or not an integer.
pub(crate) fn int_param(block: &Block, name: &str) -> Option<i64> {
    match block.params.get(name) {
        Some(ParamValue::Int(n)) => Some(*n),
        _ => None,
    }
}

/// Parses a dotted IPv4 address such as `192.168.0.10`: four decimal octets, each 0..=255.
pub fn ipv4_octets(text: &str) -> Option<[u8; 4]> {
    let mut octets = [0u8; 4];
    let mut parts = text.split('.');
    for octet in &mut octets {
        let part = parts.next()?;
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *octet = part.parse().ok()?;
    }
    parts.next().is_none().then_some(octets)
}

/// Output ports of a tag: a connection leaving one of them means the tag is read.
const OUTPUT_PORTS: [&str; 3] = ["out", "out_f", "out_b"];
/// Input ports of a tag: a connection arriving at one of them means the tag is written.
const INPUT_PORTS: [&str; 3] = ["in", "in_f", "in_b"];

/// One tag of the project as it sits in its owner's table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    /// Id of the `modbus_tag` block.
    pub id: String,
    /// Id of the block the tag belongs to (its `device` parameter); empty when it is missing.
    pub owner: String,
    pub table: String,
    /// `int`, `float` or `bool`, as the block says it (`int` when it does not).
    pub kind: String,
    pub address: u16,
    /// Cells taken, see [`slot_size`].
    pub size: usize,
}

/// Direction of a tag, taken from the connections on the sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagDirection {
    /// An output is connected: the value is read from the device.
    Read,
    /// An input is connected: the value is written to the device.
    Write,
    /// Both an output and an input are connected: an error.
    Both,
    /// Nothing is connected: an error.
    None,
}

/// A problem of the register map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModbusConflict {
    /// Two tags of one owner's table share a cell. `address` is the first shared cell.
    Overlap {
        owner: String,
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

/// The farthest occupied cell of `owner`'s `table`, 0 when it is empty.
fn table_end(slots: &[Slot], owner: &str, table: &str) -> usize {
    slots
        .iter()
        .filter(|s| s.owner == owner && s.table == table)
        .map(|s| usize::from(s.address) + s.size)
        .max()
        .unwrap_or(0)
}

impl Logic {
    /// One slot per `modbus_tag` block, in block order. The address is the `reg` parameter,
    /// clamped to the 16-bit range.
    pub fn modbus_slots(&self) -> Vec<Slot> {
        self.blocks
            .iter()
            .filter(|b| b.kind == "modbus_tag")
            .map(|b| {
                let table = text_param(b, "table").unwrap_or("holding").to_owned();
                let kind = text_param(b, "kind").unwrap_or("int").to_owned();
                let address = match int_param(b, "reg") {
                    Some(n) => u16::try_from(n.clamp(0, i64::from(u16::MAX))).unwrap_or(0),
                    None => 0,
                };
                Slot {
                    id: b.id.clone(),
                    owner: text_param(b, "device").unwrap_or("").to_owned(),
                    size: slot_size(&table, &kind),
                    table,
                    kind,
                    address,
                }
            })
            .collect()
    }

    /// `modbus_device` blocks whose `master` parameter is `master_id`, in block order.
    pub fn modbus_devices(&self, master_id: &str) -> Vec<&Block> {
        self.blocks
            .iter()
            .filter(|b| b.kind == "modbus_device" && text_param(b, "master") == Some(master_id))
            .collect()
    }

    /// The tags of one owner, in block order.
    pub fn modbus_tags_of(&self, owner: &str) -> Vec<Slot> {
        self.modbus_slots()
            .into_iter()
            .filter(|s| s.owner == owner)
            .collect()
    }

    /// The address right after the farthest occupied cell of `owner`'s `table`, 0 when it is
    /// empty. Holes left by deleted tags are not reused. Saturates at `u16::MAX`.
    pub fn modbus_next_free(&self, owner: &str, table: &str) -> u16 {
        let end = table_end(&self.modbus_slots(), owner, table);
        u16::try_from(end).unwrap_or(u16::MAX)
    }

    /// Size of the holding, input, coil and discrete tables of `owner` in that order: the
    /// farthest occupied cell of each, 0 when it is empty.
    pub fn modbus_table_sizes(&self, owner: &str) -> [usize; 4] {
        let slots = self.modbus_slots();
        ["holding", "input", "coil", "discrete"].map(|table| table_end(&slots, owner, table))
    }

    /// Direction of the tag `tag_id` from the connections: a connection leaving an output
    /// (`out`, `out_f`, `out_b`) makes it a read, a connection arriving at an input (`in`,
    /// `in_f`, `in_b`) makes it a write. The `ok` output does not count.
    pub fn modbus_tag_direction(&self, tag_id: &str) -> TagDirection {
        let reads = self
            .links
            .iter()
            .any(|l| l.from.block == tag_id && OUTPUT_PORTS.contains(&l.from.port.as_str()));
        let writes = self
            .links
            .iter()
            .any(|l| l.to.block == tag_id && INPUT_PORTS.contains(&l.to.port.as_str()));
        match (reads, writes) {
            (true, true) => TagDirection::Both,
            (true, false) => TagDirection::Read,
            (false, true) => TagDirection::Write,
            (false, false) => TagDirection::None,
        }
    }

    /// Overlapping tags (each pair once, inside one owner's table) and value types that do not
    /// fit their table.
    pub fn modbus_conflicts(&self) -> Vec<ModbusConflict> {
        let slots = self.modbus_slots();
        let mut conflicts = Vec::new();
        for (i, a) in slots.iter().enumerate() {
            for b in &slots[i + 1..] {
                if a.owner != b.owner || a.table != b.table {
                    continue;
                }
                let (a_start, b_start) = (usize::from(a.address), usize::from(b.address));
                if a_start < b_start + b.size && b_start < a_start + a.size {
                    conflicts.push(ModbusConflict::Overlap {
                        owner: a.owner.clone(),
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
