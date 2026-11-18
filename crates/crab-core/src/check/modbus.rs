//! Modbus register map of a slave project (`modbus_map`): overlapping tags and value types that
//! do not fit their table.
use super::*;
use crate::modbus_map::ModbusConflict;

pub(super) fn check_modbus(logic: &Logic, report: &mut Report) {
    for conflict in logic.modbus_conflicts() {
        match conflict {
            ModbusConflict::Overlap {
                table,
                first,
                second,
                address,
            } => report.push(
                DiagKind::ParamValue,
                Severity::Error,
                Some(&second),
                None,
                format!(
                    "Modbus: теги «{first}» и «{second}» пересекаются в таблице {table} (адрес {address})"
                ),
            ),
            ModbusConflict::BadKind { id, table, kind } => report.push(
                DiagKind::ParamValue,
                Severity::Error,
                Some(&id),
                None,
                format!("Modbus: тег «{id}»: тип {kind} не подходит таблице {table}"),
            ),
        }
    }
}
