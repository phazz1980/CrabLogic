//! Circuits of the panel: the wires between devices and their numbers.
//!
//! Wires are not drawn by hand: they follow from the devices of `panel.toml` (`specs/electrical-
//! schematics`, plan, "Топология цепей"). Run [`crab_core::check_panel`] first; a device with a
//! broken binding is skipped here rather than reported.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

use crab_core::{Device, DeviceKind, Library, Panel, ParamKind, ParamValue, Project, Supply};

/// Designation of the controller board in the wires.
pub const BOARD: &str = "A1";
/// Designation of a source outside the panel (a feed or a 24 V supply nobody listed).
pub const EXTERNAL: &str = "EXT";

const CONTROL: [&str; 2] = ["+24V", "0V"];

/// A terminal of a device: `K1:2`, `A1:IO26`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pin {
    pub device: String,
    pub terminal: String,
}

impl Pin {
    fn new(device: &str, terminal: impl Into<String>) -> Self {
        Self {
            device: device.to_owned(),
            terminal: terminal.into(),
        }
    }
}

impl fmt::Display for Pin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.device, self.terminal)
    }
}

/// What a wire carries; decides how it is drawn and where it is listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireClass {
    /// Mains side: L, N, L1..L3.
    Power,
    /// 24 V circuits: `+24V`, `0V`, coils.
    Control,
    /// Protective earth.
    Protective,
    /// A signal to or from a pin of the board.
    Signal,
}

/// A wire between two terminals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wire {
    pub number: u32,
    pub class: WireClass,
    pub from: Pin,
    pub to: Pin,
}

impl Wire {
    /// The key under which the number of the wire is kept in `panel.toml`.
    pub fn key(&self) -> String {
        format!("{}>{}", self.from, self.to)
    }
}

/// A terminal of the board that has a wire: the row of the list of terminals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardTerminal {
    /// `IO26`, or `relays.3` for a pin of an expander.
    pub terminal: String,
    /// The input or output of the project (tag id).
    pub tag: String,
    /// The device of the panel it is bound to.
    pub device: String,
    /// Number of its wire.
    pub wire: u32,
}

/// Wires and board terminals of a panel.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Nets {
    pub wires: Vec<Wire>,
    pub board_terminals: Vec<BoardTerminal>,
}

impl Nets {
    /// Numbers of the current wires, to be kept in the panel.
    pub fn wire_numbers(&self) -> BTreeMap<String, u32> {
        self.wires.iter().map(|w| (w.key(), w.number)).collect()
    }

    /// Keeps the numbers in the panel, dropping those of wires that are gone.
    pub fn store(&self, panel: &mut Panel) {
        panel.wire_numbers = self.wire_numbers();
    }
}

/// Conductors of the mains side for a supply, in the order of the poles.
pub fn mains(supply: Supply) -> &'static [&'static str] {
    match supply {
        Supply::Dc24 => &CONTROL,
        Supply::Ac230 => &["L", "N"],
        Supply::Ac400 => &["L1", "L2", "L3", "N"],
    }
}

/// Devices that take part in the power graph.
fn is_powered(kind: DeviceKind) -> bool {
    matches!(
        kind,
        DeviceKind::Breaker
            | DeviceKind::Rcd
            | DeviceKind::Psu
            | DeviceKind::Contactor
            | DeviceKind::Relay
            | DeviceKind::Motor
            | DeviceKind::Load
    )
}

/// Devices that switch their conductors with poles `1/2`, `3/4`, ...
fn has_poles(kind: DeviceKind) -> bool {
    matches!(
        kind,
        DeviceKind::Breaker | DeviceKind::Rcd | DeviceKind::Contactor | DeviceKind::Relay
    )
}

/// Terminal of `device` for the conductor number `i` (`name`) on its supply side.
fn in_pin(device: &Device, i: usize, name: &str) -> Pin {
    if has_poles(device.kind) {
        Pin::new(&device.id, (2 * i + 1).to_string())
    } else {
        Pin::new(&device.id, name)
    }
}

/// Terminal of `device` for the conductor number `i` (`name`) on its load side.
fn out_pin(device: &Device, i: usize, name: &str) -> Pin {
    if has_poles(device.kind) {
        Pin::new(&device.id, (2 * i + 2).to_string())
    } else {
        // A feed or a power supply gives the conductor under its own name.
        Pin::new(&device.id, name)
    }
}

/// Terminal of the board a device is bound to, with the tag behind it.
fn board_terminal(project: &Project, lib: &Library, bind: &str) -> Option<(String, String)> {
    let name = bind.split('.').next().unwrap_or_default();
    let tag = project.logic.tag_of(name);
    if !project.is_tag(lib, tag) {
        return None;
    }
    let block = project.logic.block(tag)?;
    let def = lib.resolve(&project.logic, block)?;
    let pin_param = def.def.params.iter().find(|p| p.kind == ParamKind::Pin)?;
    let terminal = match block.params.get(&pin_param.name) {
        Some(ParamValue::Int(pin)) => match &block.expander {
            Some(expander) => format!("{expander}.{pin}"),
            None => format!("IO{pin}"),
        },
        _ => tag.to_owned(),
    };
    Some((terminal, tag.to_owned()))
}

struct Draft {
    class: WireClass,
    from: Pin,
    to: Pin,
}

/// Works out the wires of the panel and numbers them. The numbers already kept in
/// `project.panel.wire_numbers` stay with their wires; new wires get the numbers after the
/// largest one. The result does not change the project: call [`Nets::store`] to keep the numbers.
pub fn build_nets(project: &Project, lib: &Library) -> Nets {
    let panel = &project.panel;
    let devices = &panel.devices;
    let ids: HashSet<&str> = devices.iter().map(|d| d.id.as_str()).collect();
    let feed = devices.iter().find(|d| d.kind == DeviceKind::Feed);
    let mut drafts: Vec<Draft> = Vec::new();

    // Who feeds whom: `load` of the feeding device names the one it feeds.
    let mut feeder: HashMap<&str, &Device> = HashMap::new();
    for d in devices {
        let can_feed = is_powered(d.kind) || d.kind == DeviceKind::Feed;
        if can_feed && d.load != d.id && ids.contains(d.load.as_str()) {
            feeder.entry(d.load.as_str()).or_insert(d);
        }
    }
    // Whether the circuit of a device is the 24 V one (behind a power supply).
    let control_domain = |device: &Device| -> bool {
        let mut current = device;
        for _ in 0..=devices.len() {
            match feeder.get(current.id.as_str()) {
                Some(up) if up.kind == DeviceKind::Psu => return true,
                Some(up) => current = up,
                None => return false,
            }
        }
        false
    };

    // Power circuits.
    for t in devices.iter().filter(|d| is_powered(d.kind)) {
        let up = feeder.get(t.id.as_str()).copied();
        let control = control_domain(t);
        let (names, class): (&[&str], _) = if control {
            (&CONTROL, WireClass::Control)
        } else {
            (mains(panel.supply), WireClass::Power)
        };
        for (i, name) in names.iter().enumerate() {
            let from = match (up, feed) {
                (Some(up), _) => out_pin(up, i, name),
                (None, Some(feed)) => out_pin(feed, i, name),
                (None, None) => Pin::new(EXTERNAL, *name),
            };
            // A device behind a power supply sees its 24 V as the conductors `+24V`, `0V`.
            drafts.push(Draft {
                class,
                from,
                to: in_pin(t, i, name),
            });
        }
    }
    // Protective earth to everything that has a body to earth.
    for t in devices.iter().filter(|d| {
        matches!(
            d.kind,
            DeviceKind::Motor | DeviceKind::Load | DeviceKind::Psu
        )
    }) {
        drafts.push(Draft {
            class: WireClass::Protective,
            from: feed.map_or_else(|| Pin::new(EXTERNAL, "PE"), |f| Pin::new(&f.id, "PE")),
            to: Pin::new(&t.id, "PE"),
        });
    }

    // Control circuits: the 24 V source, then the coils, sensors and buttons.
    let psu = devices.iter().find(|d| d.kind == DeviceKind::Psu);
    let source = |name: &str, i: usize| -> Pin {
        match (psu, feed) {
            (Some(psu), _) => out_pin(psu, i, name),
            (None, Some(feed)) if panel.supply == Supply::Dc24 => out_pin(feed, i, name),
            _ => Pin::new(EXTERNAL, name),
        }
    };
    let mut bound: Vec<(String, String, &Device)> = Vec::new();
    for d in devices {
        let Some(bind) = (!d.bind.is_empty()).then_some(d.bind.as_str()) else {
            continue;
        };
        let Some((terminal, tag)) = board_terminal(project, lib, bind) else {
            continue;
        };
        let board = Pin::new(BOARD, &terminal);
        match d.kind {
            DeviceKind::Contactor | DeviceKind::Relay => {
                drafts.push(Draft {
                    class: WireClass::Signal,
                    from: board,
                    to: Pin::new(&d.id, "A1"),
                });
                drafts.push(Draft {
                    class: WireClass::Control,
                    from: Pin::new(&d.id, "A2"),
                    to: source("0V", 1),
                });
            }
            DeviceKind::Sensor => {
                drafts.push(Draft {
                    class: WireClass::Control,
                    from: source("+24V", 0),
                    to: Pin::new(&d.id, "+"),
                });
                drafts.push(Draft {
                    class: WireClass::Control,
                    from: source("0V", 1),
                    to: Pin::new(&d.id, "-"),
                });
                drafts.push(Draft {
                    class: WireClass::Signal,
                    from: Pin::new(&d.id, "S"),
                    to: board,
                });
            }
            DeviceKind::Button => {
                drafts.push(Draft {
                    class: WireClass::Control,
                    from: source("+24V", 0),
                    to: Pin::new(&d.id, "1"),
                });
                drafts.push(Draft {
                    class: WireClass::Signal,
                    from: Pin::new(&d.id, "2"),
                    to: board,
                });
            }
            _ => continue,
        }
        bound.push((terminal, tag, d));
    }

    // Numbers: kept ones stay, the rest follow the largest.
    let mut used: HashSet<u32> = HashSet::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut wires: Vec<Wire> = Vec::new();
    let mut fresh: Vec<usize> = Vec::new();
    for draft in drafts {
        let wire = Wire {
            number: 0,
            class: draft.class,
            from: draft.from,
            to: draft.to,
        };
        let key = wire.key();
        if !seen.insert(key.clone()) {
            continue;
        }
        let kept = panel.wire_numbers.get(&key).copied();
        match kept.filter(|n| *n > 0 && used.insert(*n)) {
            Some(number) => wires.push(Wire { number, ..wire }),
            None => {
                fresh.push(wires.len());
                wires.push(wire);
            }
        }
    }
    let mut next = used.iter().copied().max().unwrap_or(0);
    for i in fresh {
        next += 1;
        wires[i].number = next;
    }

    // The list of board terminals, with the number of the wire on each.
    let mut board_terminals = Vec::new();
    let mut listed: HashSet<&str> = HashSet::new();
    for (terminal, tag, device) in &bound {
        if !listed.insert(terminal.as_str()) {
            continue;
        }
        let wire = wires
            .iter()
            .find(|w| {
                w.from.device == BOARD && w.from.terminal == *terminal
                    || w.to.device == BOARD && w.to.terminal == *terminal
            })
            .map_or(0, |w| w.number);
        board_terminals.push(BoardTerminal {
            terminal: terminal.clone(),
            tag: tag.clone(),
            device: device.id.clone(),
            wire,
        });
    }
    Nets {
        wires,
        board_terminals,
    }
}
