//! Designations with the number of the sheet (`K17.1`: the first device of its kind on sheet 17).

use std::collections::BTreeMap;

use crab_core::{DeviceKind, Panel};

use crate::Document;

/// A device of a group: its column, its place in the panel and its designation.
type Entry<'a> = (usize, usize, &'a str);

/// New designations `(old, new)` of the devices of `panel`: the letter of the kind, the number of
/// the sheet the device is drawn on and its number among the devices of that letter on the sheet
/// (by column, then in the order of the panel). A device with a coil is counted on the sheet of
/// the coil; others on the power sheet. Terminal blocks and auxiliary contacts (they are drawn
/// with their device) are left alone, as are devices that are not drawn.
pub fn sheet_designations(doc: &Document, panel: &Panel) -> Vec<(String, String)> {
    // (sheet, letter) -> devices with the column and the place in the panel.
    let mut groups: BTreeMap<(usize, char), Vec<Entry>> = BTreeMap::new();
    for (index, device) in panel.devices.iter().enumerate() {
        if matches!(device.kind, DeviceKind::Terminal | DeviceKind::Aux) {
            continue;
        }
        let Some(at) = doc
            .coil_at
            .get(&device.id)
            .or_else(|| doc.power_at.get(&device.id))
        else {
            continue;
        };
        let column = at.zone.parse().unwrap_or(0);
        groups
            .entry((at.sheet, device.kind.letter()))
            .or_default()
            .push((column, index, &device.id));
    }
    let mut names = Vec::new();
    for ((sheet, letter), mut devices) in groups {
        devices.sort_unstable();
        for (n, (_, _, id)) in devices.into_iter().enumerate() {
            names.push((id.to_owned(), format!("{letter}{sheet}.{}", n + 1)));
        }
    }
    names
}
