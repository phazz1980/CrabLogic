//! LoRa E220 (`specs/lora-e220`): the pins of the modem (M1 may be left out: M0 and M1 tied together), the partner blocks (`lora_point` on the
//! centre, `lora_centre` on a slave) and their link to the modem, the addresses of the slaves.
use std::collections::BTreeMap;

use super::*;
use crate::Block;

/// Slaves of one centre (`drivers/lora_e220`, `master::MAX_PEERS`).
const MAX_SLAVES: usize = 8;

fn int_param(block: &Block, name: &str, default: i64) -> i64 {
    match block.params.get(name) {
        Some(ParamValue::Int(i)) => *i,
        Some(ParamValue::Float(x)) => *x as i64,
        _ => default,
    }
}

fn text_param<'a>(block: &'a Block, name: &str) -> &'a str {
    match block.params.get(name) {
        Some(ParamValue::Text(t)) => t,
        _ => "",
    }
}

/// The modem is a centre (role 0, the default) and not a slave.
fn is_centre(modem: &Block) -> bool {
    int_param(modem, "role", 0) == 0
}

pub(super) fn check_lora(logic: &Logic, report: &mut Report) {
    let modems: Vec<&Block> = logic
        .blocks
        .iter()
        .filter(|b| b.tag.is_none() && b.kind == "lora_e220")
        .collect();
    for modem in &modems {
        for (param, caption) in [("tx", "TX"), ("rx", "RX"), ("m0", "M0")] {
            if int_param(modem, param, -1) < 0 {
                report.block_error(
                    DiagKind::Lora,
                    &modem.id,
                    format!(
                        "модем «{}»: не задан вывод {caption} (параметр «{param}»)",
                        modem.id
                    ),
                );
            }
        }
    }
    // Partner blocks by the modem they name.
    let mut by_modem: BTreeMap<&str, Vec<&Block>> = BTreeMap::new();
    for block in logic
        .blocks
        .iter()
        .filter(|b| b.tag.is_none() && matches!(b.kind.as_str(), "lora_point" | "lora_centre"))
    {
        let id = block.id.as_str();
        let name = text_param(block, "modem");
        let Some(modem) = logic.block(name).filter(|m| m.kind == "lora_e220") else {
            let message = if name.is_empty() {
                format!("блок «{id}»: не выбран модем (параметр «modem»)")
            } else if logic.block(name).is_some() {
                format!("блок «{id}»: «{name}» — не модем (нужен блок lora_e220)")
            } else {
                format!("блок «{id}»: модема «{name}» нет в проекте")
            };
            report.block_error(DiagKind::Lora, id, message);
            continue;
        };
        let (centre_block, centre_modem) = (block.kind == "lora_point", is_centre(modem));
        if centre_block != centre_modem {
            let (want, have) = if centre_block {
                ("«master»", "«slave»")
            } else {
                ("«slave»", "«master»")
            };
            report.block_error(
                DiagKind::Lora,
                id,
                format!("блок «{id}»: ему нужен модем с ролью {want}, а у «{name}» роль {have}"),
            );
            continue;
        }
        by_modem.entry(modem.id.as_str()).or_default().push(block);
    }
    for modem in modems {
        let partners = by_modem
            .get(modem.id.as_str())
            .map_or(&[][..], Vec::as_slice);
        let id = modem.id.as_str();
        if is_centre(modem) {
            if partners.is_empty() {
                report.block_error(
                    DiagKind::Lora,
                    id,
                    format!("модем «{id}» (master): нет ни одного блока «LoRa: slave»"),
                );
            }
            if partners.len() > MAX_SLAVES {
                report.block_error(
                    DiagKind::Lora,
                    id,
                    format!(
                        "модем «{id}»: slave {} при пределе {MAX_SLAVES}",
                        partners.len()
                    ),
                );
            }
            let mut nodes: BTreeMap<i64, &str> = BTreeMap::new();
            for p in partners {
                let node = int_param(p, "node", 0);
                if let Some(first) = nodes.insert(node, p.id.as_str()) {
                    report.block_error(
                        DiagKind::Lora,
                        &p.id,
                        format!(
                            "блок «{}»: адрес slave {node} уже занят блоком «{first}»",
                            p.id
                        ),
                    );
                }
            }
        } else if partners.is_empty() {
            report.block_error(
                DiagKind::Lora,
                id,
                format!("модем «{id}» (slave): нет блока «LoRa: master»"),
            );
        } else if partners.len() > 1 {
            report.block_error(
                DiagKind::Lora,
                id,
                format!("модем «{id}» (slave): блок «LoRa: master» должен быть один"),
            );
        }
    }
}
