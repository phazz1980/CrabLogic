//! Check of the electrical panel (`panel.toml`, `specs/electrical-schematics`).
//!
//! It is a separate report, not a part of [`crate::check_project`]: the panel only feeds the
//! documentation, so a mistake in it must never stop the firmware build. A project with an empty
//! panel gets no diagnostics at all.

use std::collections::HashMap;

use super::{DiagKind, Report, Severity};
use crate::panel::{is_valid_designation, Device, DeviceKind};
use crate::{Library, ParamKind, PinMode, Project};

/// Which way a tag of the project carries the signal, seen from the board.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Way {
    Input,
    Output,
}

/// A tag of the project that the panel can be bound to.
#[derive(Clone, Copy)]
struct TagIo {
    way: Way,
    /// Digital (on/off) or analog (ADC, PWM, DAC).
    digital: bool,
}

/// Direction and kind of the signal of a tag, from the mode of its pin; `None` for bus pins.
fn tag_io(mode: PinMode) -> Option<TagIo> {
    let (way, digital) = match mode {
        PinMode::In => (Way::Input, true),
        PinMode::Adc => (Way::Input, false),
        PinMode::Out => (Way::Output, true),
        PinMode::Pwm | PinMode::Dac => (Way::Output, false),
        PinMode::Io => return None,
    };
    Some(TagIo { way, digital })
}

/// What a device kind demands of its binding.
struct Need {
    way: Way,
    /// Only a digital signal fits (coil, button).
    digital_only: bool,
    /// What the device is, for the message.
    what: &'static str,
}

fn need(kind: DeviceKind) -> Option<Need> {
    let (way, digital_only, what) = match kind {
        DeviceKind::Contactor => (Way::Output, true, "катушка контактора"),
        DeviceKind::Relay => (Way::Output, true, "катушка реле"),
        DeviceKind::Button => (Way::Input, true, "кнопка"),
        DeviceKind::Sensor => (Way::Input, false, "датчик"),
        DeviceKind::Aux => (Way::Input, true, "доп. контакт"),
        _ => return None,
    };
    Some(Need {
        way,
        digital_only,
        what,
    })
}

fn way_name(way: Way) -> &'static str {
    match way {
        Way::Input => "вход",
        Way::Output => "выход",
    }
}

fn way_dative(way: Way) -> &'static str {
    match way {
        Way::Input => "входу",
        Way::Output => "выходу",
    }
}

/// An input or output of the project that a device of the panel can be bound to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelTag {
    /// The name of the tag.
    pub name: String,
    /// A signal into the board (otherwise out of it).
    pub input: bool,
    /// On/off (otherwise analog: ADC, PWM, DAC).
    pub digital: bool,
}

/// The inputs and outputs of the project (tags with a pin that is not a bus).
pub fn panel_tags(project: &Project, lib: &Library) -> Vec<PanelTag> {
    project
        .logic
        .blocks
        .iter()
        .filter(|b| project.is_tag(lib, &b.id))
        .filter_map(|block| {
            let mode = lib
                .resolve(&project.logic, block)?
                .def
                .params
                .iter()
                .find(|p| p.kind == ParamKind::Pin)?
                .mode;
            let io = tag_io(mode)?;
            Some(PanelTag {
                name: block.id.clone(),
                input: io.way == Way::Input,
                digital: io.digital,
            })
        })
        .collect()
}

/// What a device of this kind is bound to, for a list of choices: `(input, digital only)`;
/// `None` for the devices the schema does not drive.
pub fn panel_binding(kind: DeviceKind) -> Option<(bool, bool)> {
    need(kind).map(|n| (n.way == Way::Input, n.digital_only))
}

/// Checks the panel of `project` against its inputs and outputs.
pub fn check_panel(project: &Project, lib: &Library) -> Report {
    let mut report = Report::default();
    let panel = &project.panel;
    if panel.is_empty() {
        return report;
    }
    let mut error = |message: String| {
        report.push(DiagKind::Panel, Severity::Error, None, None, message);
    };

    // The inputs and outputs of the project: tags with a pin that is not a bus.
    let mut tags: HashMap<&str, TagIo> = HashMap::new();
    for block in &project.logic.blocks {
        if !project.is_tag(lib, &block.id) {
            continue;
        }
        let mode = lib.resolve(&project.logic, block).and_then(|l| {
            l.def
                .params
                .iter()
                .find(|p| p.kind == ParamKind::Pin)
                .map(|p| p.mode)
        });
        if let Some(io) = mode.and_then(tag_io) {
            tags.insert(block.id.as_str(), io);
        }
    }

    // Designations: valid and unique.
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for device in &panel.devices {
        if !is_valid_designation(&device.id) {
            error(format!(
                "устройство «{}»: недопустимое обозначение (буквы, цифры, «_», «-» и «.», без пробелов)",
                device.id
            ));
        }
        *seen.entry(device.id.as_str()).or_default() += 1;
    }
    for (i, device) in panel.devices.iter().enumerate() {
        // Say it once, at the first of the equal ones.
        let first = panel.devices.iter().position(|d| d.id == device.id) == Some(i);
        if first && seen[device.id.as_str()] > 1 {
            error(format!(
                "обозначение {} занято несколькими устройствами",
                device.designation()
            ));
        }
    }

    // Bindings.
    let mut owner: HashMap<&str, &Device> = HashMap::new();
    for device in &panel.devices {
        let want = need(device.kind);
        if device.bind.is_empty() {
            // An auxiliary contact may stay unbound: it is then only drawn on the power sheet.
            if want.is_some() && device.kind != DeviceKind::Aux {
                error(format!(
                    "{} не привязано к входу или выходу проекта",
                    device.designation()
                ));
            }
            continue;
        }
        let name = device.bind.split('.').next().unwrap_or_default();
        let tag = project.logic.tag_of(name);
        let Some(io) = tags.get(tag) else {
            error(format!(
                "{}: в проекте нет входа или выхода «{name}»",
                device.designation()
            ));
            continue;
        };
        if let Some(want) = want {
            if io.way != want.way {
                error(format!(
                    "{}: {} привязывается к {} платы, а «{tag}» — {}",
                    device.designation(),
                    want.what,
                    way_dative(want.way),
                    way_name(io.way)
                ));
                continue;
            }
            if want.digital_only && !io.digital {
                error(format!(
                    "{}: {} привязывается к дискретному {}, а «{tag}» — аналоговый (АЦП, ШИМ или ЦАП)",
                    device.designation(),
                    want.what,
                    way_dative(want.way)
                ));
                continue;
            }
        }
        if let Some(first) = owner.insert(tag, device) {
            error(format!(
                "«{tag}» уже привязан к {}, второе устройство — {}",
                first.designation(),
                device.designation()
            ));
        }
    }

    // A sensor of a motor (a thermistor) belongs to a motor.
    for device in panel
        .devices
        .iter()
        .filter(|d| d.kind == DeviceKind::Sensor && !d.of.is_empty())
    {
        match panel.device(&device.of) {
            None => error(format!(
                "датчик {}: нет двигателя «{}», у которого он стоит",
                device.id, device.of
            )),
            Some(parent) if parent.kind != DeviceKind::Motor => error(format!(
                "датчик {}: {} не двигатель (датчик температуры ставится у двигателя)",
                device.id,
                parent.designation()
            )),
            Some(_) => {}
        }
    }

    // Devices on a branch of another device (`from`): the source must exist and feed others.
    for device in panel.devices.iter().filter(|d| !d.from.is_empty()) {
        let fine = panel.device(&device.from).is_some_and(|s| {
            let relay_contact = panel.is_relay_contact(s);
            s.id != device.id
                && (relay_contact
                    || matches!(
                        s.kind,
                        DeviceKind::Feed
                            | DeviceKind::Breaker
                            | DeviceKind::Rcd
                            | DeviceKind::Fuse
                            | DeviceKind::Switch
                            | DeviceKind::Psu
                            | DeviceKind::Contactor
                            | DeviceKind::Relay
                            | DeviceKind::Terminal
                    ))
        });
        if !fine {
            error(format!(
                "{}: питается от «{}», а такого источника нет",
                device.designation(),
                device.from
            ));
        }
    }

    // Auxiliary contacts: they belong to a switching device and go to the board or to a coil.
    for device in panel.devices.iter().filter(|d| d.kind == DeviceKind::Aux) {
        let name = format!("доп. контакт {}", device.id);
        match panel.device(&device.of) {
            None => error(format!(
                "{name}: нет аппарата «{}», которому он принадлежит",
                device.of
            )),
            Some(parent)
                if !matches!(
                    parent.kind,
                    DeviceKind::Breaker
                        | DeviceKind::Rcd
                        | DeviceKind::Contactor
                        | DeviceKind::Relay
                ) =>
            {
                error(format!(
                    "{name}: у {} не бывает доп. контактов (автомат, УЗО, контактор или реле)",
                    parent.designation()
                ));
            }
            Some(_) => {}
        }
        // Only a contact of a relay switches a load of its own: the main contacts of a contactor
        // and the contacts of a breaker stand in the chain of their device.
        let parent = panel.device(&device.of);
        let of_relay = parent.is_some_and(|p| p.kind == DeviceKind::Relay);
        let powered = !device.load.is_empty() || !device.from.is_empty();
        if powered && parent.is_some() && !of_relay {
            error(format!(
                "{name}: свою нагрузку коммутирует только контакт реле, у {} нагрузку задают у самого аппарата",
                parent.map(Device::designation).unwrap_or_default()
            ));
        }
        if powered && of_relay {
            if !device.series.is_empty() || !device.bind.is_empty() {
                error(format!(
                    "{name}: контакт коммутирует нагрузку и стоит в цепи управления, нужно что-то одно"
                ));
            }
            // An old relay that switches the same load by its poles: two feeders of one device.
            if let Some(relay) = parent.filter(|r| !device.load.is_empty() && r.load == device.load)
            {
                error(format!(
                    "{name}: нагрузку «{}» уже коммутирует само реле {}",
                    device.load,
                    relay.designation()
                ));
            }
        }
        if !device.series.is_empty() {
            if !device.bind.is_empty() {
                error(format!(
                    "{name}: задан и вход платы, и катушка «{}», нужно что-то одно",
                    device.series
                ));
            }
            match panel.device(&device.series) {
                None => error(format!("{name}: нет катушки «{}»", device.series)),
                Some(coil) if !matches!(coil.kind, DeviceKind::Contactor | DeviceKind::Relay) => {
                    error(format!(
                        "{name}: {} не катушка (контактор или реле)",
                        coil.designation()
                    ));
                }
                Some(_) => {}
            }
        }
    }

    // What a device switches or feeds.
    for device in &panel.devices {
        if device.load.is_empty() {
            continue;
        }
        if device.load == device.id {
            error(format!(
                "{}: устройство не может коммутировать само себя",
                device.designation()
            ));
        } else if panel.device(&device.load).is_none() {
            error(format!(
                "{}: нет устройства «{}», которое оно коммутирует",
                device.designation(),
                device.load
            ));
        }
    }

    // Every input and output of the project needs a device.
    let unused = project.unused_tags(lib);
    let mut free: Vec<(&str, TagIo)> = tags
        .iter()
        .filter(|(id, _)| !unused.iter().any(|u| u == *id) && !owner.contains_key(*id))
        .map(|(&id, &io)| (id, io))
        .collect();
    free.sort_by_key(|&(id, _)| id);
    for (id, io) in free {
        error(format!("{} «{id}» без устройства в щите", way_name(io.way)));
    }

    // The letter of the designation (IEC 81346).
    for device in &panel.devices {
        let letter = device.kind.letter();
        let fits = device.id.starts_with(letter)
            || (device.kind == DeviceKind::Load && device.id.starts_with(['E', 'Y', 'H', 'R']));
        if !fits && device.kind != DeviceKind::Aux && is_valid_designation(&device.id) {
            report.push(
                DiagKind::Panel,
                Severity::Warning,
                None,
                None,
                format!(
                    "{}: по IEC 81346 обозначение этого устройства начинается с «{letter}»",
                    device.designation()
                ),
            );
        }
    }
    report
}
