//! Check of the electrical panel (`panel.toml`, `specs/electrical-schematics`).
//!
//! It is a separate report, not a part of [`crate::check_project`]: the panel only feeds the
//! documentation, so a mistake in it must never stop the firmware build. A project with an empty
//! panel gets no diagnostics at all.

use std::collections::HashMap;

use super::{DiagKind, Report, Severity};
use crate::ident::is_valid_ident;
use crate::panel::{Device, DeviceKind};
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
        if !is_valid_ident(&device.id) {
            error(format!(
                "устройство «{}»: недопустимое обозначение (буквы, цифры, «_» и «-», без пробелов)",
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
            if want.is_some() {
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
        if !fits && is_valid_ident(&device.id) {
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
