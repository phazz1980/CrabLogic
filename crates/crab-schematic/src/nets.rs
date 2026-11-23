//! Circuits of the panel: the wires between devices and their numbers.
//!
//! Wires are not drawn by hand: they follow from the devices of `panel.toml` (`specs/electrical-
//! schematics`, plan, "Топология цепей"). Run [`crab_core::check_panel`] first; a device with a
//! broken binding is skipped here rather than reported.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

use crab_core::{
    ContactForm, Device, DeviceKind, Library, Panel, ParamKind, ParamValue, Project, Supply,
};

/// Designation of the controller board in the wires.
pub const BOARD: &str = "A1";
/// Designation of a source outside the panel (a feed or a 24 V supply nobody listed).
pub const EXTERNAL: &str = "EXT";

/// Device of a wire end that is a supply line (`bus:0V` in a link): the line is named by its end,
/// not by a terminal. A wire joins the line as its own node, not the one the generator draws.
pub const BUS: &str = "bus";

const CONTROL: [&str; 2] = ["+24V", "0V"];

/// What a supply line carries: a 24 V rail, or a conductor of the mains. `None` for a name that is
/// no line of the supply.
fn bus_class(supply: Supply, name: &str) -> Option<WireClass> {
    if CONTROL.contains(&name) {
        Some(WireClass::Control)
    } else if mains(supply).contains(&name) {
        Some(WireClass::Power)
    } else {
        None
    }
}

/// An end of a wire as the list writes it: `K1:A2`, and the name of the line for a bus (`0V`).
pub fn end_name(pin: &Pin) -> String {
    if pin.device == BUS {
        pin.terminal.clone()
    } else {
        pin.to_string()
    }
}

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
    /// The conductor the wire belongs to: the wires that join the same terminal share it, and so
    /// they share the number (a terminal block splits the conductor: the wires before it and after
    /// it are two).
    pub net: usize,
    pub class: WireClass,
    pub from: Pin,
    pub to: Pin,
}

impl Wire {
    /// Key of the number of the wire in the manual layout (it can be moved).
    pub fn label_key(&self) -> String {
        crab_core::panel_layout::label_key(&self.from.device, &format!("#{}", self.key()))
    }

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
    /// Keys of the wires that were drawn by hand (`panel.links`), not worked out.
    pub manual: Vec<String>,
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
        _ => supply.lines(),
    }
}

/// Devices that take part in the power graph.
fn is_powered(kind: DeviceKind) -> bool {
    matches!(
        kind,
        DeviceKind::Breaker
            | DeviceKind::Rcd
            | DeviceKind::Fuse
            | DeviceKind::Switch
            | DeviceKind::Psu
            | DeviceKind::Contactor
            | DeviceKind::Relay
            | DeviceKind::Motor
            | DeviceKind::Load
            | DeviceKind::Terminal
    )
}

/// Devices that switch their conductors with poles `1/2`, `3/4`, ...
fn has_poles(kind: DeviceKind) -> bool {
    matches!(
        kind,
        DeviceKind::Breaker
            | DeviceKind::Rcd
            | DeviceKind::Fuse
            | DeviceKind::Switch
            | DeviceKind::Contactor
            | DeviceKind::Relay
    )
}

/// Whether `d` is a contact of a relay (`of` names a relay). Such a contact with a `load` is a
/// one-pole switching device of the power chain; the contacts of contactors, breakers and
/// residual-current devices never are.
pub fn is_relay_contact(panel: &Panel, d: &Device) -> bool {
    panel.is_relay_contact(d)
}

/// Devices that take part in the power graph: the switching devices and the contacts of relays.
fn is_powered_device(panel: &Panel, d: &Device) -> bool {
    is_powered(d.kind) || is_relay_contact(panel, d)
}

/// Terminals of the auxiliary contact `contact` used in a circuit: the contact is the `k`-th of its
/// device, so a normally open one is `k3`-`k4` and a normally closed one `k1`-`k2` (`13-14`,
/// `21-22`). A changeover uses its common and normally open terminals (`k1`-`k4`).
pub fn aux_terminals(panel: &Panel, contact: &Device) -> (String, String) {
    let all = aux_all_terminals(panel, contact);
    match contact.form {
        ContactForm::Changeover => (all[0].clone(), all[2].clone()),
        _ => (all[0].clone(), all[1].clone()),
    }
}

/// Every terminal of the auxiliary contact `contact`: two for a normally open or normally closed
/// one, three for a changeover (common, normally closed, normally open). Manual `pins` (`21-22`,
/// or `11-12-14` for a changeover) override the numbers by the contact, when their count fits the
/// form.
pub fn aux_all_terminals(panel: &Panel, contact: &Device) -> Vec<String> {
    let k = panel
        .aux_of(&contact.of)
        .position(|d| d.id == contact.id)
        .map_or(1, |i| i + 1);
    let manual: Vec<&str> = contact
        .pins
        .split('-')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if manual.len() == contact_all_terminals(k, contact.form).len() {
        return manual.into_iter().map(str::to_owned).collect();
    }
    contact_all_terminals(k, contact.form)
}

/// The normally closed terminal of a changeover contact (`k2`, the middle one of its terminals), or
/// `None` for a normally open or normally closed contact.
pub fn changeover_nc(panel: &Panel, contact: &Device) -> Option<String> {
    (contact.form == ContactForm::Changeover).then(|| aux_all_terminals(panel, contact)[1].clone())
}

/// Terminals of the `k`-th (from 1) auxiliary contact of a device used in a circuit: the two of a
/// normally open or normally closed contact, the common and normally open ones of a changeover.
pub fn contact_terminals(k: usize, form: ContactForm) -> (String, String) {
    match form {
        ContactForm::No => (format!("{k}3"), format!("{k}4")),
        ContactForm::Nc => (format!("{k}1"), format!("{k}2")),
        ContactForm::Changeover => (format!("{k}1"), format!("{k}4")),
    }
}

/// Every terminal of the `k`-th (from 1) auxiliary contact of a device: `k3`, `k4` for a normally
/// open contact, `k1`, `k2` for a normally closed one, `k1` (common), `k2` (normally closed) and
/// `k4` (normally open) for a changeover.
pub fn contact_all_terminals(k: usize, form: ContactForm) -> Vec<String> {
    let terminals: &[usize] = match form {
        ContactForm::No => &[3, 4],
        ContactForm::Nc => &[1, 2],
        ContactForm::Changeover => &[1, 2, 4],
    };
    terminals.iter().map(|t| format!("{k}{t}")).collect()
}

/// Terminal of `device` for the conductor number `i` (`name`) on its supply side. A relay contact
/// has one pole: its supply terminal.
pub(crate) fn in_pin(panel: &Panel, device: &Device, i: usize, name: &str) -> Pin {
    if is_relay_contact(panel, device) {
        return Pin::new(&device.id, aux_terminals(panel, device).0);
    }
    if has_poles(device.kind) {
        Pin::new(&device.id, (2 * i + 1).to_string())
    } else {
        Pin::new(&device.id, name)
    }
}

/// Terminal of `device` for the conductor number `i` (`name`) on its load side. A relay contact
/// has one pole: its load terminal.
pub(crate) fn out_pin(panel: &Panel, device: &Device, i: usize, name: &str) -> Pin {
    if is_relay_contact(panel, device) {
        return Pin::new(&device.id, aux_terminals(panel, device).1);
    }
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

/// The pin written `device:terminal` in a link.
fn pin_of(end: &str) -> Option<Pin> {
    let (device, terminal) = end.split_once(':')?;
    (!device.is_empty() && !terminal.is_empty()).then(|| Pin::new(device, terminal))
}

/// Terminals of a device on which a wire can be drawn by hand.
pub fn terminals_of(panel: &Panel, device: &Device) -> Vec<String> {
    let lines = || mains(panel.supply).iter().map(|s| (*s).to_owned());
    let (lo, hi) = panel.supply.pole_range();
    let poles = || {
        let n = device.poles.map_or(hi, |p| p.clamp(lo, hi));
        (1..=2 * usize::from(n)).map(|i| i.to_string())
    };
    match device.kind {
        DeviceKind::Contactor | DeviceKind::Relay => ["A1".to_owned(), "A2".to_owned()]
            .into_iter()
            .chain(poles())
            .collect(),
        DeviceKind::Breaker | DeviceKind::Rcd | DeviceKind::Fuse | DeviceKind::Switch => {
            poles().collect()
        }
        DeviceKind::Aux => aux_all_terminals(panel, device),
        DeviceKind::Sensor => vec!["+".to_owned(), "-".to_owned(), "S".to_owned()],
        DeviceKind::Button => vec!["1".to_owned(), "2".to_owned()],
        DeviceKind::Motor | DeviceKind::Load | DeviceKind::Terminal | DeviceKind::Feed => {
            lines().chain(["PE".to_owned()]).collect()
        }
        DeviceKind::Psu => lines().chain(CONTROL.map(str::to_owned)).collect(),
    }
}

/// The wires that make one conductor: those that share a terminal (not through a terminal block). Returns the groups of the
/// indices of `wires`, in the order of their first wire.
fn join_wires(
    wires: &[Wire],
    terminals: &HashSet<&str>,
    lines: &HashMap<String, String>,
) -> Vec<Vec<usize>> {
    let mut ids: HashMap<String, usize> = HashMap::new();
    let mut parent: Vec<usize> = Vec::new();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let mut ends: Vec<(usize, usize)> = Vec::with_capacity(wires.len());
    for w in wires {
        // A terminal block splits the conductor: its terminal is two nodes, the side the wire
        // comes in and the side it goes on.
        let mut id = |pin: &Pin, side: &str| {
            let next = parent.len();
            let name = if pin.device == BUS {
                // A wire to a supply line is the conductor the generator draws under its name.
                lines
                    .get(&pin.terminal)
                    .cloned()
                    .unwrap_or_else(|| pin.to_string())
            } else if terminals.contains(pin.device.as_str()) {
                format!("{pin}/{side}")
            } else {
                pin.to_string()
            };
            let id = *ids.entry(name).or_insert(next);
            if id == next {
                parent.push(next);
            }
            id
        };
        let (a, b) = (id(&w.from, "out"), id(&w.to, "in"));
        ends.push((a, b));
    }
    for &(a, b) in &ends {
        let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
        parent[ra] = rb;
    }
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut at: HashMap<usize, usize> = HashMap::new();
    for (i, &(a, _)) in ends.iter().enumerate() {
        let r = root(&mut parent, a);
        let g = *at.entry(r).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[g].push(i);
    }
    groups
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
        let can_feed = is_powered_device(panel, d) || d.kind == DeviceKind::Feed;
        if can_feed && d.load != d.id && ids.contains(d.load.as_str()) {
            feeder.entry(d.load.as_str()).or_insert(d);
        }
    }
    // A device on a branch (`from`) hangs on that device, besides the one it feeds by `load`.
    for d in devices.iter().filter(|d| !d.from.is_empty()) {
        if let Some(source) = devices.iter().find(|s| s.id == d.from && s.id != d.id) {
            feeder.insert(d.id.as_str(), source);
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
    // A terminal block that stands in no chain draws no wires.
    let in_chain = |d: &&Device| {
        // A relay contact stands in a chain only when it is fed or has a load that exists.
        if is_relay_contact(panel, d) {
            return feeder.contains_key(d.id.as_str()) || ids.contains(d.load.as_str());
        }
        // A device just added and not connected to anything draws no wires.
        if d.free && !feeder.contains_key(d.id.as_str()) {
            return false;
        }
        d.kind != DeviceKind::Terminal
            || feeder.contains_key(d.id.as_str())
            || ids.contains(d.load.as_str())
    };
    for t in devices
        .iter()
        .filter(|d| is_powered_device(panel, d))
        .filter(in_chain)
    {
        let up = feeder.get(t.id.as_str()).copied();
        let control = control_domain(t);
        let (names, class): (&[&str], _) = if control {
            (&CONTROL, WireClass::Control)
        } else {
            (mains(panel.supply), WireClass::Power)
        };
        // A switching device with fewer poles passes on only the first conductors; what stands
        // under it (up to the next power supply) gets just those.
        let mut lines = names.len();
        let mut walk = Some(t);
        let (lo, hi) = if control {
            Supply::Dc24.pole_range()
        } else {
            panel.supply.pole_range()
        };
        for _ in 0..=devices.len() {
            let Some(d) = walk else { break };
            // A relay contact has one pole and is not clamped to the poles of the supply.
            let poles = if is_relay_contact(panel, d) {
                Some(1)
            } else if has_poles(d.kind) {
                d.poles.map(|p| usize::from(p.clamp(lo, hi)))
            } else {
                None
            };
            if let Some(p) = poles.filter(|_| control_domain(d) == control) {
                lines = lines.min(p);
            }
            walk = feeder.get(d.id.as_str()).copied();
        }
        for (i, name) in names.iter().enumerate().take(lines) {
            let from = match (up, feed) {
                (Some(up), _) => out_pin(panel, up, i, name),
                (None, Some(feed)) => out_pin(panel, feed, i, name),
                (None, None) => Pin::new(EXTERNAL, *name),
            };
            // A device behind a power supply sees its 24 V as the conductors `+24V`, `0V`.
            drafts.push(Draft {
                class,
                from,
                to: in_pin(panel, t, i, name),
            });
        }
    }
    // Protective earth to everything that has a body to earth and to the terminal blocks; it
    // comes from the nearest terminal block above the device, else from the feed.
    for t in devices.iter().filter(|d| {
        matches!(
            d.kind,
            DeviceKind::Motor | DeviceKind::Load | DeviceKind::Psu | DeviceKind::Terminal
        ) && in_chain(d)
    }) {
        let mut above = feeder.get(t.id.as_str()).copied();
        let mut terminal = None;
        for _ in 0..=devices.len() {
            let Some(d) = above else { break };
            if d.kind == DeviceKind::Terminal {
                terminal = Some(d);
                break;
            }
            above = feeder.get(d.id.as_str()).copied();
        }
        drafts.push(Draft {
            class: WireClass::Protective,
            from: match (terminal, feed) {
                (Some(x), _) => Pin::new(&x.id, "PE"),
                (None, Some(f)) => Pin::new(&f.id, "PE"),
                (None, None) => Pin::new(EXTERNAL, "PE"),
            },
            to: Pin::new(&t.id, "PE"),
        });
    }

    // Control circuits: the 24 V source, then the coils, sensors and buttons.
    let psu = devices.iter().find(|d| d.kind == DeviceKind::Psu);
    let source = |name: &str, i: usize| -> Pin {
        match (psu, feed) {
            (Some(psu), _) => out_pin(panel, psu, i, name),
            (None, Some(feed)) if panel.supply == Supply::Dc24 => out_pin(panel, feed, i, name),
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
                // The output of the board goes through the contacts in series with the coil, in
                // the order of the panel, to `A1`.
                let mut from = board;
                let mut class = WireClass::Signal;
                for contact in devices.iter().filter(|c| {
                    c.kind == DeviceKind::Aux && c.series == d.id && panel.device(&c.of).is_some()
                }) {
                    let (inn, out) = aux_terminals(panel, contact);
                    drafts.push(Draft {
                        class,
                        from,
                        to: Pin::new(&contact.id, inn),
                    });
                    from = Pin::new(&contact.id, out);
                    class = WireClass::Control;
                }
                drafts.push(Draft {
                    class,
                    from,
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
            DeviceKind::Aux if d.series.is_empty() && panel.device(&d.of).is_some() => {
                // An auxiliary contact on an input of the board, like a button.
                let (inn, out) = aux_terminals(panel, d);
                drafts.push(Draft {
                    class: WireClass::Control,
                    from: source("+24V", 0),
                    to: Pin::new(&d.id, inn),
                });
                drafts.push(Draft {
                    class: WireClass::Signal,
                    from: Pin::new(&d.id, out),
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

    // Wires drawn by hand replace the automatic ones at the terminals they join.
    let manual: Vec<Draft> = panel
        .links
        .iter()
        .filter_map(|link| {
            let (from, to) = (pin_of(&link.from)?, pin_of(&link.to)?);
            let known = |p: &Pin| match p.device.as_str() {
                BUS => bus_class(panel.supply, &p.terminal).is_some(),
                device => device == BOARD || device == EXTERNAL || ids.contains(device),
            };
            let buses = from.device == BUS && to.device == BUS;
            if !(known(&from) && known(&to) && from != to && !buses) {
                return None;
            }
            let bus = [&from, &to]
                .into_iter()
                .find(|p| p.device == BUS)
                .and_then(|p| bus_class(panel.supply, &p.terminal));
            Some(Draft {
                class: if from.device == BOARD || to.device == BOARD {
                    WireClass::Signal
                } else {
                    bus.unwrap_or(WireClass::Control)
                },
                from,
                to,
            })
        })
        .collect();
    // A wire to a supply line adds to the potential of the terminal, it does not replace the
    // automatic wire there: only the wires between two terminals of the panel do.
    drafts.retain(|d| {
        !manual
            .iter()
            .filter(|m| m.from.device != BUS && m.to.device != BUS)
            .any(|m| {
                [&m.from, &m.to]
                    .iter()
                    .any(|p| **p == d.from || **p == d.to)
            })
    });
    let manual_keys: Vec<String> = manual
        .iter()
        .map(|d| format!("{}>{}", d.from, d.to))
        .collect();
    drafts.extend(manual);

    // Numbers: the wires that join the same terminal are one conductor (a bus drop, the earth, a
    // wire through a terminal block) and have one number. The numbers kept in the panel stay, the
    // new ones follow the largest.
    let mut seen: HashSet<String> = HashSet::new();
    let mut wires: Vec<Wire> = Vec::new();
    for draft in drafts {
        let wire = Wire {
            number: 0,
            net: 0,
            class: draft.class,
            from: draft.from,
            to: draft.to,
        };
        if seen.insert(wire.key()) {
            wires.push(wire);
        }
    }
    let terminals: HashSet<&str> = devices
        .iter()
        .filter(|d| d.kind == DeviceKind::Terminal)
        .map(|d| d.id.as_str())
        .collect();
    // The conductor of each supply line, by its name: the 24 V source, or the feed (or the
    // outside) for the mains. A wire to `bus:<name>` joins it.
    let mut lines: HashMap<String, String> = HashMap::new();
    for (i, name) in CONTROL.into_iter().enumerate() {
        lines.insert(name.to_owned(), source(name, i).to_string());
    }
    for (i, name) in mains(panel.supply).iter().enumerate() {
        let pin = match feed {
            Some(feed) => out_pin(panel, feed, i, name),
            None => Pin::new(EXTERNAL, *name),
        };
        lines.entry((*name).to_owned()).or_insert(pin.to_string());
    }
    let nets = join_wires(&wires, &terminals, &lines);
    for (net, members) in nets.iter().enumerate() {
        members.iter().for_each(|&i| wires[i].net = net);
    }
    let mut used: HashSet<u32> = HashSet::new();
    let mut fresh: Vec<usize> = Vec::new();
    for (net, members) in nets.iter().enumerate() {
        let kept = members
            .iter()
            .filter_map(|&i| panel.wire_numbers.get(&wires[i].key()).copied())
            .find(|n| *n > 0 && used.insert(*n));
        match kept {
            Some(number) => members.iter().for_each(|&i| wires[i].number = number),
            None => fresh.push(net),
        }
    }
    let start = used.iter().copied().max().unwrap_or(0) + 1;
    for (net, number) in fresh.into_iter().zip(start..) {
        nets[net].iter().for_each(|&i| wires[i].number = number);
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
        manual: manual_keys,
        wires,
        board_terminals,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel(text: &str) -> Panel {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn terminals_of_each_form() {
        for (form, first, second) in [
            (ContactForm::No, ["13", "14"], ["23", "24"]),
            (ContactForm::Nc, ["11", "12"], ["21", "22"]),
            (ContactForm::Changeover, ["11", "14"], ["21", "24"]),
        ] {
            let form_text = match form {
                ContactForm::No => "no",
                ContactForm::Nc => "nc",
                ContactForm::Changeover => "changeover",
            };
            let p = panel(&format!(
                r#"
[[device]]
id = "Q1"
kind = "breaker"
[[device]]
id = "Q1_1"
kind = "aux"
of = "Q1"
form = "{form_text}"
[[device]]
id = "Q1_2"
kind = "aux"
of = "Q1"
form = "{form_text}"
"#
            ));
            let first_dev = p.device("Q1_1").unwrap();
            let second_dev = p.device("Q1_2").unwrap();
            // The terminals in a circuit: the first contact is `k=1`, the second `k=2`.
            let pair = |s: [&str; 2]| (s[0].to_owned(), s[1].to_owned());
            assert_eq!(aux_terminals(&p, first_dev), pair(first), "{form:?}");
            assert_eq!(aux_terminals(&p, second_dev), pair(second), "{form:?}");
            assert_eq!(
                terminals_of(&p, first_dev).len(),
                if form == ContactForm::Changeover {
                    3
                } else {
                    2
                }
            );
        }
    }

    #[test]
    fn changeover_terminals_by_contact_number() {
        assert_eq!(
            contact_all_terminals(2, ContactForm::Changeover),
            ["21", "22", "24"]
        );
        assert_eq!(contact_all_terminals(1, ContactForm::No), ["13", "14"]);
        assert_eq!(contact_all_terminals(1, ContactForm::Nc), ["11", "12"]);
        assert_eq!(
            contact_terminals(1, ContactForm::Changeover),
            ("11".to_owned(), "14".to_owned())
        );
    }

    #[test]
    fn pins_override_numbers() {
        let p = panel(
            r#"
[[device]]
id = "Q1"
kind = "breaker"
[[device]]
id = "Q1_1"
kind = "aux"
of = "Q1"
form = "changeover"
pins = "11-12-14"
[[device]]
id = "Q1_2"
kind = "aux"
of = "Q1"
form = "nc"
pins = "21-22"
[[device]]
id = "Q1_3"
kind = "aux"
of = "Q1"
form = "changeover"
pins = "31-32"
"#,
        );
        let co = p.device("Q1_1").unwrap();
        assert_eq!(aux_all_terminals(&p, co), ["11", "12", "14"]);
        assert_eq!(aux_terminals(&p, co), ("11".to_owned(), "14".to_owned()));
        assert_eq!(
            aux_all_terminals(&p, p.device("Q1_2").unwrap()),
            ["21", "22"]
        );
        // Two pins do not fit a changeover: the numbers by the contact are used.
        assert_eq!(
            aux_all_terminals(&p, p.device("Q1_3").unwrap()),
            ["31", "32", "34"]
        );
    }
}
