//! Modbus tags and devices (`modbus_map`): owners of tags, tag directions, overlaps and value
//! types in the register tables, the slave count and unit ids of each master.
use super::*;
use crate::modbus_map::{int_param, ipv4_octets, text_param, ModbusConflict, TagDirection};
use crate::Block;

/// Master blocks: the Modbus master on this board.
const MASTER_KINDS: [&str; 2] = ["modbus_rtu_master", "modbus_tcp_master"];
/// Slave-role blocks: this board is the slave.
const SLAVE_KINDS: [&str; 2] = ["modbus_rtu_slave", "modbus_tcp_slave"];
/// Default of the master's `slaves` parameter (the number of slave devices it may have).
const DEFAULT_SLAVES: i64 = 4;
/// Default TCP port of a Modbus TCP master (its library default).
const DEFAULT_TCP_PORT: i64 = 502;

pub(super) fn check_modbus(logic: &Logic, report: &mut Report) {
    check_devices(logic, report);
    check_tags(logic, report);
}

/// Devices: the master exists, the address and port are valid for the master's kind, the number
/// of devices fits the master's limit, and two devices clash only at the same unit and endpoint.
fn check_devices(logic: &Logic, report: &mut Report) {
    for device in logic.blocks.iter().filter(|b| b.kind == "modbus_device") {
        let master = text_param(device, "master").unwrap_or("");
        let found = logic
            .blocks
            .iter()
            .any(|b| b.id == master && MASTER_KINDS.contains(&b.kind.as_str()));
        if !found {
            report.push(
                DiagKind::ParamValue,
                Severity::Error,
                Some(&device.id),
                None,
                format!(
                    "Modbus: slave «{}»: master «{master}» не найден (нужен блок modbus_rtu_master или modbus_tcp_master)",
                    device.id
                ),
            );
        }
        check_device_address(logic, device, master, report);
    }

    for master in logic
        .blocks
        .iter()
        .filter(|b| MASTER_KINDS.contains(&b.kind.as_str()))
    {
        let devices = logic.modbus_devices(&master.id);
        let limit = int_param(master, "slaves").unwrap_or(DEFAULT_SLAVES);
        let count = i64::try_from(devices.len()).unwrap_or(i64::MAX);
        if count > limit {
            report.push(
                DiagKind::ParamValue,
                Severity::Error,
                Some(&master.id),
                None,
                format!(
                    "Modbus: у master «{}» {count} slave, а предел «slaves» — {limit}",
                    master.id
                ),
            );
        }
        for (i, a) in devices.iter().enumerate() {
            for b in &devices[i + 1..] {
                let (Some(unit_a), Some(unit_b)) = (int_param(a, "unit"), int_param(b, "unit"))
                else {
                    continue;
                };
                if unit_a == unit_b && endpoint(master, a) == endpoint(master, b) {
                    report.push(
                        DiagKind::ParamValue,
                        Severity::Error,
                        Some(&b.id),
                        None,
                        format!(
                            "Modbus: у master «{}» два slave с Unit ID {unit_a}: «{}» и «{}»",
                            master.id, a.id, b.id
                        ),
                    );
                }
            }
        }
    }
}

/// The address of a device: `ip` is a dotted quad when it is set, and `ip` or `port` are set only
/// under a Modbus TCP master.
fn check_device_address(logic: &Logic, device: &Block, master_id: &str, report: &mut Report) {
    let ip = text_param(device, "ip").unwrap_or("");
    let port = int_param(device, "port").unwrap_or(0);
    if !ip.is_empty() && ipv4_octets(ip).is_none() {
        report.push(
            DiagKind::ParamValue,
            Severity::Error,
            Some(&device.id),
            None,
            format!(
                "Modbus: slave «{}»: адрес «{ip}» — не IP-адрес вида 192.168.0.10",
                device.id
            ),
        );
    }
    let master_kind = logic
        .blocks
        .iter()
        .find(|b| b.id == master_id)
        .map(|b| b.kind.as_str());
    if master_kind == Some("modbus_rtu_master") && (!ip.is_empty() || port != 0) {
        report.push(
            DiagKind::ParamValue,
            Severity::Error,
            Some(&device.id),
            None,
            format!(
                "Modbus: slave «{}»: адрес и порт slave нужны только для Modbus TCP master",
                device.id
            ),
        );
    }
}

/// The address a device is polled at: its own `ip` and `port`, or the master's when they are
/// empty or 0 (the library default port 502). A Modbus RTU master has one line for all devices.
fn endpoint(master: &Block, device: &Block) -> (String, i64) {
    if master.kind != "modbus_tcp_master" {
        return (String::new(), 0);
    }
    let ip = match text_param(device, "ip") {
        Some(ip) if !ip.is_empty() => ip,
        _ => text_param(master, "ip").unwrap_or(""),
    };
    let port = match int_param(device, "port") {
        Some(p) if p != 0 => p,
        _ => int_param(master, "port").unwrap_or(DEFAULT_TCP_PORT),
    };
    (ip.to_owned(), port)
}

/// Tags: the owner exists, the direction is one of read or write, and the table cells fit.
fn check_tags(logic: &Logic, report: &mut Report) {
    for tag in logic.blocks.iter().filter(|b| b.kind == "modbus_tag") {
        let owner = text_param(tag, "device").unwrap_or("");
        let owner_found = logic.blocks.iter().any(|b| {
            b.id == owner && (b.kind == "modbus_device" || SLAVE_KINDS.contains(&b.kind.as_str()))
        });
        if !owner_found {
            let message = if owner.is_empty() {
                format!(
                    "Modbus: тег «{}» не привязан к slave или устройству (параметр device)",
                    tag.id
                )
            } else {
                format!(
                    "Modbus: тег «{}»: устройство «{owner}» не найдено (нужен modbus_device или slave)",
                    tag.id
                )
            };
            report.push(
                DiagKind::ParamValue,
                Severity::Error,
                Some(&tag.id),
                None,
                message,
            );
        }

        match logic.modbus_tag_direction(&tag.id) {
            TagDirection::Both => report.push(
                DiagKind::ParamValue,
                Severity::Error,
                Some(&tag.id),
                None,
                format!(
                    "Modbus: тег «{}» читается и записывается: подключите только вход или только выход",
                    tag.id
                ),
            ),
            TagDirection::None => report.push(
                DiagKind::ParamValue,
                Severity::Error,
                Some(&tag.id),
                None,
                format!("Modbus: тег «{}» не подключён к схеме", tag.id),
            ),
            TagDirection::Read | TagDirection::Write => {}
        }
    }

    for conflict in logic.modbus_conflicts() {
        match conflict {
            ModbusConflict::Overlap {
                owner,
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
                    "Modbus: теги «{first}» и «{second}» пересекаются в таблице {table} устройства «{owner}» (адрес {address})"
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
